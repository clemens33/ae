//! Seat overrides: the command-line profile and client selections, their resolution against the roster and the recorded session.

use super::{
    Env, IdentityConfig, Meta, Path, PathBuf, Plan, Seat, ToolKind, config, meta, meta_value,
    node_exists, roster,
};
use crate::run::pair_judge::{
    RecordedSelection, SeatOverrideRefusal, cross_tool_refusal, invalid_client_refusal, known_list,
    recorded_selection, refuse_recorded_client_conflict, resolve_client_override,
    unknown_store_refusal,
};

/// One override resolved through the exact config snapshot preflight read.
pub(super) struct ResolvedSeatOverride {
    pub(super) agent: String,
    /// The bare profile label — `profile.<slot>` never carries `@`.
    pub(super) profile: String,
    /// The honored `profile@client` label, if the selection named one.
    pub(super) client: Option<String>,
    pub(super) command: config::ResolvedCommand,
    pub(super) parsed: crate::launch_cmd::SimpleCommand,
    /// Whether the CURRENT resolution proved the label pairing: false when it
    /// resolved `Unknown`, in which case the build honors the seat but records
    /// no new pairing.
    pub(super) store_proven: bool,
    /// The recorded profile this override replaces as its `[profile_successors]`
    /// successor, `None` for an explicit flag or a carried label kept as recorded.
    pub(super) successor_of: Option<String>,
    /// For a successor move: every recorded fact it was decided from, which
    /// the meta read under the lifecycle lock must still carry.
    pub(super) decided_on: Option<Box<DecidedOn>>,
}

/// The recorded facts an automatic `[profile_successors]` move was decided
/// from before the lifecycle lock: the seat's whole roster row and the config
/// files its identity was read from. A locked meta that differs in any of
/// them is another seat, and the move is stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DecidedOn {
    entry: crate::meta::RosterEntry,
    config: Option<String>,
    origin: Option<String>,
    local_config: Option<String>,
}

impl DecidedOn {
    fn of(ctx: &OverrideCtx, entry: &crate::meta::RosterEntry, recorded: &Meta) -> Box<Self> {
        Box::new(Self {
            entry: entry.clone(),
            config: ctx.recorded_config.map(ToOwned::to_owned),
            origin: recorded.origin().map(ToOwned::to_owned),
            local_config: ctx.recorded_local_config.map(ToOwned::to_owned),
        })
    }

    /// Whether the meta read under the lock (`bytes`) still records exactly
    /// these facts.
    fn holds_in(&self, bytes: &[u8]) -> bool {
        let locked = Meta::parse(&String::from_utf8_lossy(bytes));
        let row = |key: &str| meta::first_value(bytes, key).map(String::from_utf8_lossy);
        row("config").as_deref() == self.config.as_deref()
            && row(config::LOCAL_CONFIG_KEY).as_deref() == self.local_config.as_deref()
            && locked.origin() == self.origin.as_deref()
            && locked.roster().contains(&self.entry)
    }
}

/// Split a `--lead`/`--colead`/`--seat` value into its profile and optional
/// `profile@client` override label.
///
/// Neither the profile grammar (`is_config_key`) nor the client grammar
/// (`is_agent_name`) admits `@`, so one `@` splits unambiguously; an empty
/// half or a second `@` is a usage error naming the value.
pub(super) fn split_seat_override(value: &str) -> Result<(String, Option<String>), String> {
    split_profile_client(value).ok_or_else(|| {
        format!(
            "Error: --seat selection '{value}' is not <profile> or <profile>@<client> \
             (e.g. --seat lead=solx, --lead fablex@cc-work)."
        )
    })
}

/// The grammar every `profile@client` spelling shares (launch `--seat`, spawn
/// `--using`): one `@` splits a profile from its client, no `@` is a bare
/// profile, and an empty half or a second `@` is `None` for the caller to
/// word.
pub(crate) fn split_profile_client(value: &str) -> Option<(String, Option<String>)> {
    let Some((profile, client)) = value.split_once('@') else {
        return Some((value.to_owned(), None));
    };
    if profile.is_empty() || client.is_empty() || client.contains('@') {
        return None;
    }
    Some((profile.to_owned(), Some(client.to_owned())))
}

/// Identity plus parsed override commands carried unchanged through the lock.
pub(super) struct SeatOverrideSnapshot {
    pub(super) cfg: IdentityConfig,
    pub(super) overrides: Vec<ResolvedSeatOverride>,
}

/// The launch-agent names an override may target, in seat order.
fn override_agents(cfg: &IdentityConfig, plan: &Plan, dir: &Path, resuming: bool) -> Vec<String> {
    if resuming {
        let Ok(bytes) = meta::read_bytes(dir) else {
            return Vec::new();
        };
        return Meta::parse(&String::from_utf8_lossy(&bytes))
            .roster()
            .iter()
            .filter(|entry| entry.slot == "main" || entry.slot.starts_with("worker."))
            .map(|entry| entry.name.clone())
            .collect();
    }

    let mut agents = Vec::new();
    if let Some(main) = plan.main.as_deref().or(cfg.main.as_deref()) {
        let main = main.trim();
        if !main.is_empty() && cfg.roster_profile(main).is_some() {
            agents.push(main.to_owned());
        }
    }
    let workers = plan.workers.as_deref().or(cfg.workers.as_deref());
    if let Some(workers) = workers {
        agents.extend(
            workers
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty() && cfg.roster_profile(name).is_some())
                .map(str::to_owned),
        );
    }
    agents
}

/// Read the config this launch will use without changing session state.
fn override_identity(
    env: &Env,
    dir: &Path,
    resuming: bool,
) -> Result<IdentityConfig, SeatOverrideRefusal> {
    let mut global = env.global.clone();
    let mut local = env.local.clone();
    if resuming {
        if let Some(stored) = meta_value(dir, "config").filter(|value| !value.is_empty()) {
            global = Some(PathBuf::from(stored));
        }
        let origin = meta_value(dir, "origin").unwrap_or_default();
        local = config::local_overlay(dir, &origin);
    }
    let read = if resuming {
        config::read_identity(global.as_deref(), local.as_deref())
    } else {
        config::read_identity_with_global_default(
            global.as_deref(),
            local.as_deref(),
            crate::entry::DEFAULT_CONFIG,
        )
    };
    read.map_err(|why| SeatOverrideRefusal::Failed(why.to_string()))
}

/// The identity a STOPPED session's flagless resume launches with: the
/// recorded global config and project overlay, as the resume's own setup
/// points `env` at them, through [`launch_identity`], which never lets the
/// dedicated orchestrator overlay's legacy identity rows take authority. Every
/// automatic `[profile_successors]` decision is taken on it.
///
/// It reads the config files the recorded meta `bytes` name, the same read
/// every other fact of the decision comes from.
fn automatic_identity(env: &Env, bytes: &[u8]) -> Result<IdentityConfig, SeatOverrideRefusal> {
    let mut resumed = env.clone();
    let row = |key: &str| crate::lifecycle::meta_value(bytes, key);
    let stored = row("config");
    if !stored.is_empty() {
        resumed.global = Some(PathBuf::from(stored));
    }
    resumed.local = config::local_overlay_in(Some(bytes), &row("origin"));
    launch_identity(&resumed, None, false)
        .map_err(|why| SeatOverrideRefusal::Failed(why.to_string()))
}

pub(super) fn running_override_refusal(session: &str) -> String {
    format!("Error: session '{session}' is running; stop it before changing a seat profile.")
}

/// The one read-only launch-config resolver. Preflight supplies the defaults
/// that the launch will seed later; a resume reads its recorded files as-is.
pub(super) fn launch_identity(
    env: &Env,
    snapshot: Option<&SeatOverrideSnapshot>,
    before_seed: bool,
) -> Result<IdentityConfig, config::ConfigError> {
    if let Some(snapshot) = snapshot {
        return Ok(snapshot.cfg.clone());
    }
    let orchestrator_seat = env
        .local
        .as_deref()
        .is_some_and(|local| crate::meta::is_seat_overlay(local, &env.home));
    let local = if orchestrator_seat {
        None
    } else {
        env.local.as_deref()
    };
    let mut cfg = if before_seed {
        config::read_identity_with_global_default(
            env.global.as_deref(),
            local,
            crate::entry::DEFAULT_CONFIG,
        )?
    } else {
        config::read_identity(env.global.as_deref(), local)?
    };
    if orchestrator_seat {
        // The dedicated overlay contributes only workspace choices. Its legacy
        // profile/roster sections are deliberately ignored at both call sites.
        let defaults = if before_seed && !env.local.as_deref().is_some_and(node_exists) {
            config::parse_identity(crate::orchestrator::DEFAULT_CONFIG)?
        } else {
            IdentityConfig::default()
        };
        let keys = config::read_workspace_keys(None, env.local.as_deref(), &["main", "workers"]);
        cfg.main = keys[0].clone().or(defaults.main).or(cfg.main);
        cfg.workers = keys[1].clone().or(defaults.workers).or(cfg.workers);
    }
    Ok(cfg)
}

/// Existing metadata deliberately bypasses this creation-only reservation.
pub(super) fn reserved_launch_actor(
    env: &Env,
    plan: &Plan,
    snapshot: Option<&SeatOverrideSnapshot>,
) -> Option<String> {
    let mut cfg = launch_identity(env, snapshot, true).ok()?;
    if let Some(workers) = &plan.workers {
        cfg.workers = Some(workers.clone());
    }
    config::launch_plan(&cfg, plan.main.as_deref(), crate::doors::home().as_deref())
        .ok()?
        .seats
        .into_iter()
        .find(|seat| config::is_reserved_actor(&seat.name))
        .map(|seat| seat.name)
}

fn solo_resume_refusal(plan: &Plan, dir: &Path, resuming: bool) -> Option<String> {
    if plan.workers.as_deref() != Some("") || !resuming {
        return None;
    }
    let workers = meta::read_bytes(dir)
        .ok()
        .map(|bytes| Meta::parse(&String::from_utf8_lossy(&bytes)))
        .map(|parsed| {
            parsed
                .roster()
                .iter()
                .filter(|entry| entry.slot.starts_with("worker."))
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    (!workers.is_empty()).then(|| {
        format!(
            "Error: '{}' already has workers ({workers}); '--solo' applies to a first launch.",
            plan.name.as_deref().unwrap_or_default()
        )
    })
}

fn solo_worker_override_refusal(plan: &Plan, cfg: &IdentityConfig) -> Option<String> {
    if plan.workers.as_deref() != Some("") {
        return None;
    }
    let main = plan.main.as_deref().or(cfg.main.as_deref()).map(str::trim);
    let configured_workers = cfg
        .workers
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty() && Some(*name) != main);
    plan.seat_profiles
        .iter()
        .any(|(agent, _)| configured_workers.clone().any(|worker| worker == agent))
        .then(|| "Error: '--solo' starts the lead alone; drop --colead/--seat <worker>.".to_owned())
}

/// The recorded main identity when the standing roster has no worker seats.
///
/// Such a roster is the only durable fact a later launch has that this session
/// was created solo. Carrying it into the existing config snapshot prevents a
/// configured worker list from growing the roster on resume, while keeping the
/// command paired with the profile that the main seat recorded.
pub(super) fn recorded_solo_identity(dir: &Path) -> Result<Option<(String, String)>, String> {
    let bytes = meta::read_bytes(dir).map_err(|why| format!("cannot read meta ({why})"))?;
    let parsed = Meta::parse(&String::from_utf8_lossy(&bytes));
    if parsed.schema() != Some("2") {
        return Err("missing schema=2".to_owned());
    }
    if let Some(anomaly) = parsed
        .anomalies()
        .iter()
        .find(|item| roster::roster_doubting(item))
    {
        return Err(anomaly.to_string());
    }
    let Some(main) = parsed.roster().iter().find(|entry| entry.slot == "main") else {
        return Err("missing seat.main".to_owned());
    };
    let Some(profile) = main.profile.as_ref().filter(|profile| !profile.is_empty()) else {
        return Err("missing profile.main".to_owned());
    };
    if parsed
        .roster()
        .iter()
        .any(|entry| entry.slot.starts_with("worker."))
    {
        return Ok(None);
    }
    Ok(Some((main.name.clone(), profile.clone())))
}

fn doubtful_solo_meta_refusal(plan: &Plan, anomaly: &str) -> String {
    format!(
        "Error: session '{}' has doubtful roster metadata ({anomaly}); run 'ae doctor' before resuming.",
        plan.name.as_deref().unwrap_or_default()
    )
}

fn frozen_solo_identity(
    plan: &Plan,
    dir: &Path,
    resuming: bool,
    running: bool,
) -> Result<Option<(String, String)>, SeatOverrideRefusal> {
    // The dedicated orchestrator already has a separate global-identity rule;
    // forcing its single-seat roster through this snapshot would re-enable
    // legacy identity rows from its local workspace overlay.
    if !resuming
        || running
        || plan.name.as_deref() == Some(crate::orchestrator::ORCHESTRATOR_SESSION)
    {
        return Ok(None);
    }
    recorded_solo_identity(dir)
        .map_err(|anomaly| SeatOverrideRefusal::Failed(doubtful_solo_meta_refusal(plan, &anomaly)))
}

fn freeze_solo_config(
    cfg: &mut IdentityConfig,
    plan: &Plan,
    recorded_main: &str,
    recorded_profile: &str,
) -> Result<(), SeatOverrideRefusal> {
    cfg.workers = Some(String::new());
    let selected_main = plan.main.as_deref().or(cfg.main.as_deref()).map(str::trim);
    let Some((_, bound_profile)) = cfg
        .roster
        .iter_mut()
        .find(|(name, _)| Some(name.as_str()) == selected_main)
    else {
        return Err(SeatOverrideRefusal::Failed(format!(
            "Error: recorded solo main '{recorded_main}' is not in [roster] of the current config."
        )));
    };
    recorded_profile.clone_into(bound_profile);
    Ok(())
}

/// The same gate for a spawn: `profile@client` for the agent `agent`, a seat
/// with no recorded store — a FIRST START, so a pairing whose store the
/// launcher cannot verify refuses (F6) instead of recording an unproven fact.
///
/// This is [`resolve_client_override`] itself plus the first-start store proof
/// the launch takes in `resolve_one_override`; a spawn never has its own copy
/// of the adapter gate. The caller has already judged approval and existence,
/// so the unknown-name texts (which speak of `--seat`) are not reached here.
///
/// # Errors
///
/// The refusal line, whichever of the adapter gate, the expansion or the store
/// proof refused.
pub(crate) fn resolve_spawn_pair(
    cfg: &IdentityConfig,
    agent: &str,
    profile: &str,
    label: &str,
    home: Option<&Path>,
) -> Result<(config::ResolvedCommand, crate::launch_cmd::SimpleCommand), String> {
    let line = |refusal: SeatOverrideRefusal| match refusal {
        SeatOverrideRefusal::Usage(line) | SeatOverrideRefusal::Failed(line) => line,
    };
    let profiles: Vec<&str> = cfg.profiles.iter().map(|(name, _)| name.as_str()).collect();
    let clients: Vec<&str> = cfg.clients.iter().map(|(name, _)| name.as_str()).collect();
    let (command, parsed) = resolve_client_override(
        cfg,
        agent,
        profile,
        label,
        home,
        &known_list(&profiles),
        &known_list(&clients),
    )
    .map_err(line)?;
    if let Some(reason) = current_pairing_proof(&command, parsed.tool(), home) {
        return Err(line(unknown_store_refusal(
            agent,
            &format!("{profile}@{label}"),
            &reason,
        )));
    }
    Ok((command, parsed))
}

/// A `profile@client` selection for a NEW choice — `spawn --using` or
/// `reseat --using` — resolved to its command, or refused. The order is the
/// contract, every step before any effect:
///
/// 1. the session's recorded GLOBAL config is the only identity read — a
///    project overlay can neither approve a pair nor redefine an approved
///    name, so a pair means exactly what the human's own config says;
/// 2. the pair must be on the global `[client_pairs]` list — POLICY before
///    existence, so an unapproved pair learns nothing about which names exist;
/// 3. the profile and the client must exist;
/// 4. the launch's own adapter gate and first-start store proof
///    ([`resolve_spawn_pair`]).
///
/// `nothing` closes the refusals that name the selection, in the caller's own
/// verb (`Nothing was spawned.`).
///
/// # Errors
///
/// The refusal line, whichever step refused.
pub(crate) fn approved_pair(
    global: Option<&Path>,
    agent: &str,
    profile: &str,
    label: &str,
    home: Option<&Path>,
    nothing: &str,
) -> Result<(config::ResolvedCommand, crate::launch_cmd::SimpleCommand), String> {
    let spelling = format!("{profile}@{label}");
    let Some(global) = global else {
        return Err(format!(
            "Error: --using '{spelling}': this session records no global config, so no pair can be approved. {nothing}"
        ));
    };
    // ONE read of the global file feeds both the identity and the approved
    // list, so an approval and the command it approves cannot straddle an edit.
    let text = match config::read_selected(global) {
        Ok(text) => text,
        Err(config::ConfigError::Unreadable(path)) => {
            return Err(format!(
                "Error: --using '{spelling}': the global config {} cannot be read, so no pair can be approved. {nothing}",
                path.display()
            ));
        }
        Err(why) => return Err(why.to_string()),
    };
    let cfg = config::parse_identity_in(global, &text).map_err(|why| why.to_string())?;
    let verdict = crate::pairs::parse_in(global, &text).judge(profile, label);
    if let Some(why) = verdict.refusal(profile, global) {
        return Err(format!("Error: --using '{spelling}' {why} {nothing}"));
    }
    if cfg.profile(profile).is_none() {
        return Err(format!(
            "Error: profile '{profile}' not defined in [profiles] of {}",
            global.display()
        ));
    }
    if cfg.client(label).is_none() {
        return Err(format!(
            "Error: client '{label}' is not defined in [clients] of {}.",
            global.display()
        ));
    }
    resolve_spawn_pair(&cfg, agent, profile, label, home)
}

/// The CURRENT resolution's proof of a label pairing: `None` when proved,
/// else the `Unknown` reason. Decided on the RAW resolution, so a first start
/// learns it without canonicalizing anything it will not compare.
fn current_pairing_proof(
    command: &config::ResolvedCommand,
    tool: ToolKind,
    home: Option<&Path>,
) -> Option<String> {
    let home_value = home.map(|path| path.display().to_string());
    let resolution = crate::launch_cmd::config_home_resolution(command, tool, &|name| {
        if name == "HOME" {
            home_value.clone()
        } else {
            None
        }
    });
    match resolution.home {
        crate::launch_cmd::Resolved::Unknown(reason) => Some(reason),
        _ => None,
    }
}

/// The preflight snapshot one `--seat` value resolves against.
struct OverrideCtx<'a> {
    cfg: &'a IdentityConfig,
    agents: &'a [String],
    known_agents: &'a str,
    known_profiles: &'a str,
    known_clients: &'a str,
    recorded: Option<&'a Meta>,
    session: &'a str,
    home: Option<&'a Path>,
    /// The state root the vendor block records live under (`AE_HOME`).
    state_root: &'a Path,
    /// The judged `[profile_successors]` table of the recorded global config.
    successors: &'a crate::successors::Successors,
    /// The `config` row of the `recorded` meta, from the same read.
    recorded_config: Option<&'a str>,
    /// The `local_config` row of the `recorded` meta, from the same read.
    recorded_local_config: Option<&'a str>,
}

/// Resolve one `--seat <agent>=<profile[@client]>` value: the agent must be a
/// launch seat, the selection must resolve (with the R1 gate for `@`), and a
/// recorded seat must honor it (write-once client, re-taken store, same tool).
fn resolve_one_override(
    ctx: &OverrideCtx,
    agent: &str,
    value: &str,
) -> Result<ResolvedSeatOverride, SeatOverrideRefusal> {
    if !ctx.agents.iter().any(|known| known == agent) {
        return Err(SeatOverrideRefusal::Usage(format!(
            "Error: unknown launch agent '{agent}' in --seat. Known agents: {}.",
            ctx.known_agents
        )));
    }
    let (profile, client) = split_seat_override(value).map_err(SeatOverrideRefusal::Usage)?;
    // A scope its vendor refused (`ae quota clear <scope>` ends it)
    // takes no seat: the one refusal `spawn --using` owes too, before any
    // server effect. An `OpenCode` block of one upstream provider refuses only
    // the profiles whose model id names that provider.
    if let Some(why) = crate::vendorblock::scope_refusal_in(
        ctx.cfg,
        ctx.state_root,
        &profile,
        client.as_deref(),
        crate::time::Timestamp::now().epoch(),
    ) {
        return Err(SeatOverrideRefusal::Failed(format!("Error: {why}")));
    }
    let (command, parsed) = if let Some(label) = client.as_deref() {
        resolve_client_override(
            ctx.cfg,
            agent,
            &profile,
            label,
            ctx.home,
            ctx.known_profiles,
            ctx.known_clients,
        )?
    } else {
        let command = ctx
            .cfg
            .command(&profile, ctx.home)
            .map_err(|why| SeatOverrideRefusal::Failed(why.to_string()))?;
        let Some(command) = command else {
            return Err(SeatOverrideRefusal::Usage(format!(
                "Error: unknown profile '{profile}' in --seat. Known profiles: {}.",
                ctx.known_profiles
            )));
        };
        let parsed = crate::launch_cmd::lex_simple_command(command.as_str()).map_err(|why| {
            SeatOverrideRefusal::Failed(format!(
                "Error: [profiles] {profile} (seat '{agent}'): the launch command must be one simple command — it has {why}."
            ))
        })?;
        (command, parsed)
    };
    let store_proven = if let Some(parsed_meta) = ctx.recorded
        && let Some(entry) = parsed_meta
            .roster()
            .iter()
            .find(|entry| entry.name == agent)
    {
        let proven = refuse_recorded_client_conflict(
            ctx.session,
            agent,
            &profile,
            client.as_deref(),
            entry,
            ctx.cfg,
            &command,
            parsed.tool(),
            ctx.home,
        )?;
        let old = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or_default());
        let new = parsed.tool();
        if old != new {
            return Err(SeatOverrideRefusal::Usage(cross_tool_refusal(
                agent, old, new,
            )));
        }
        proven
    } else {
        // No recorded seat: nothing to compare. But a first start that
        // cannot resolve its override refuses — F6: nothing is stranded, and
        // proceeding would ignore an explicit flag. A bare profile keeps the
        // legacy path (nothing to record either way).
        match current_pairing_proof(&command, parsed.tool(), ctx.home) {
            None => true,
            Some(reason) => {
                if let Some(label) = client.as_deref() {
                    return Err(unknown_store_refusal(
                        agent,
                        &format!("{profile}@{label}"),
                        &reason,
                    ));
                }
                false
            }
        }
    };
    Ok(ResolvedSeatOverride {
        agent: agent.to_owned(),
        profile,
        client,
        command,
        parsed,
        store_proven,
        successor_of: None,
        decided_on: None,
    })
}

/// Honor every recorded `client.<slot>` label a flagless resume resumes.
///
/// `ae s` with no seat flags is "resume as recorded": each carried label is
/// resolved through the SAME path an explicit `@` selection takes — the R1
/// adapter gate, then the R4 re-take against the recorded store — so a
/// hostile or moved label refuses here, before any write, exactly as if it
/// had been spelled. Seats an explicit flag names are skipped: the explicit
/// selection already judged them (a bare profile refuses, an exact label
/// re-takes), and judging twice would double-report.
///
/// A recorded profile with a usable `[profile_successors]` successor is
/// replaced by it, decided and resolved on `auto`, the identity a flagless
/// stopped resume launches with.
fn synthesize_recorded_honors(
    ctx: &OverrideCtx,
    auto: &OverrideCtx,
    seat_profiles: &[(String, String)],
) -> Result<Vec<ResolvedSeatOverride>, SeatOverrideRefusal> {
    let Some(recorded) = ctx.recorded else {
        return Ok(Vec::new());
    };
    let mut honored = Vec::new();
    for entry in recorded.roster() {
        if entry.slot != "main" && !entry.slot.starts_with("worker.") {
            continue;
        }
        if !ctx.agents.iter().any(|known| known == &entry.name) {
            continue;
        }
        if seat_profiles.iter().any(|(agent, _)| agent == &entry.name) {
            continue;
        }
        // The usability gate above already refused every unusable row, so a
        // `Missing` here is a legacy seat to skip — anything else that is not
        // a provable label refuses with its own remedy, never silently.
        let RecordedSelection::Label(label) =
            recorded_selection(ctx.session, ctx.cfg, entry).map_err(SeatOverrideRefusal::Failed)?
        else {
            continue;
        };
        let Some(recorded_profile) = entry.profile.as_deref().filter(|name| !name.is_empty())
        else {
            return Err(SeatOverrideRefusal::Failed(format!(
                "Error: session '{}' seat '{}' recorded client override '{label}' but no profile — \
                 fix the meta row, or end the session (ae end {}).",
                ctx.session, entry.name, ctx.session
            )));
        };
        // A usable successor takes the recorded profile's place under the SAME
        // label; the judgement below is the one the recorded profile would get.
        let successor = crate::run::pair_judge::seat_successor(
            auto.session,
            auto.cfg,
            auto.successors,
            entry,
            auto.home,
            Some(auto.state_root),
        );
        let ctx = if successor.is_some() { auto } else { ctx };
        let profile = successor.as_deref().unwrap_or(recorded_profile);
        let (command, parsed) = resolve_client_override(
            ctx.cfg,
            &entry.name,
            profile,
            &label,
            ctx.home,
            ctx.known_profiles,
            ctx.known_clients,
        )?;
        let proven = refuse_recorded_client_conflict(
            ctx.session,
            &entry.name,
            profile,
            Some(&label),
            entry,
            ctx.cfg,
            &command,
            parsed.tool(),
            ctx.home,
        )?;
        honored.push(ResolvedSeatOverride {
            agent: entry.name.clone(),
            profile: profile.to_owned(),
            client: Some(label),
            command,
            parsed,
            store_proven: proven,
            successor_of: successor.is_some().then(|| recorded_profile.to_owned()),
            decided_on: successor
                .is_some()
                .then(|| DecidedOn::of(ctx, entry, recorded)),
        });
    }
    Ok(honored)
}

/// The successor of every UNLABELED launch seat a flagless part of this resume
/// restores: a recorded main or worker seat no flag names, whose recorded
/// profile has a successor usable for it, is resolved as if `--seat
/// <agent>=<successor>` had been given. A successor that does not resolve for
/// the seat keeps the recorded profile: a table row never refuses a resume.
fn synthesize_successors(
    ctx: &OverrideCtx,
    seat_profiles: &[(String, String)],
) -> Vec<ResolvedSeatOverride> {
    let Some(recorded) = ctx.recorded else {
        return Vec::new();
    };
    let mut moved = Vec::new();
    for entry in recorded.roster() {
        if entry.slot != "main" && !entry.slot.starts_with("worker.") {
            continue;
        }
        if !matches!(entry.client, crate::meta::RecordedClient::Missing)
            || !ctx.agents.iter().any(|known| known == &entry.name)
            || seat_profiles.iter().any(|(agent, _)| agent == &entry.name)
        {
            continue;
        }
        let Some(target) = crate::run::pair_judge::seat_successor(
            ctx.session,
            ctx.cfg,
            ctx.successors,
            entry,
            ctx.home,
            Some(ctx.state_root),
        ) else {
            continue;
        };
        if let Ok(mut replacement) = resolve_one_override(ctx, &entry.name, &target) {
            replacement.successor_of.clone_from(&entry.profile);
            replacement.decided_on = Some(DecidedOn::of(ctx, entry, recorded));
            moved.push(replacement);
        }
    }
    moved
}

/// Whether the session's recorded GLOBAL config declares any
/// `[profile_successors]` row at all. Shape alone decides, before any identity
/// is read, so a config without the section keeps every resume path as it was.
fn successors_declared(dir: &Path) -> bool {
    let Some(global) = meta_value(dir, "config").filter(|value| !value.is_empty()) else {
        return false;
    };
    let global = PathBuf::from(global);
    config::read_selected(&global)
        .is_ok_and(|text| !crate::successors::parse_in(&global, &text).is_empty())
}

/// The notes a resume of a stopped session prints for its ignored
/// `[profile_successors]` rows, judged against the identity the resume uses.
pub(super) fn resume_successor_notes(env: &Env, dir: &Path) -> Vec<String> {
    if !successors_declared(dir) {
        return Vec::new();
    }
    let Ok(bytes) = meta::read_bytes(dir) else {
        return Vec::new();
    };
    let Ok(cfg) = automatic_identity(env, &bytes) else {
        return Vec::new();
    };
    let home = crate::doors::home();
    crate::run::pair_judge::recorded_successors(&bytes, &cfg, home.as_deref()).notes()
}

/// Whether a flagless resume carries labels it must honor: a `Label` row on a
/// launch seat. Shape alone decides, so a legacy resume still returns before
/// any config is read.
fn flagless_honor_needed(recorded: Option<&Meta>, resuming: bool, running: bool) -> bool {
    resuming
        && !running
        && recorded.is_some_and(|parsed| {
            parsed.roster().iter().any(|entry| {
                (entry.slot == "main" || entry.slot.starts_with("worker."))
                    && matches!(entry.client, crate::meta::RecordedClient::Label(_))
            })
        })
}

/// Every refusal a launch owes BEFORE it writes anything: its seat overrides,
/// then — for a resume of a stopped session — its restored profiles.
pub(super) fn validate_seat_overrides(
    env: &Env,
    plan: &Plan,
    dir: &Path,
    resuming: bool,
    running: bool,
) -> Result<Option<SeatOverrideSnapshot>, SeatOverrideRefusal> {
    let snapshot = seat_override_snapshot(env, plan, dir, resuming, running)?;
    if resuming && !running {
        let refusal = match &snapshot {
            Some(held) => restored_profiles_refusal(plan, dir, &held.cfg, &held.overrides),
            // NAMED RESIDUAL: ANY session named orchestrator skips this (keyed on
            // the name, not the seat overlay); the dedicated seat reads its
            // identity from the global config alone, so only its own launch
            // refuses, after the migration writes (version step, v2 `tag_priors`).
            None if plan.name.as_deref() == Some(crate::orchestrator::ORCHESTRATOR_SESSION) => None,
            None => {
                restored_profiles_refusal(plan, dir, &override_identity(env, dir, resuming)?, &[])
            }
        };
        if let Some(line) = refusal {
            return Err(SeatOverrideRefusal::Failed(line));
        }
    }
    Ok(snapshot)
}

/// ONE read of the recorded meta: the roster, its `config` and `local_config`
/// rows, the automatic identity and the successor table all come from it.
struct RecordedRead {
    bytes: Option<Vec<u8>>,
    meta: Option<Meta>,
    config: Option<String>,
    local_config: Option<String>,
}

impl RecordedRead {
    fn of(dir: &Path, resuming: bool) -> Self {
        let bytes = resuming.then(|| meta::read_bytes(dir).ok()).flatten();
        let row = |key: &str| {
            bytes
                .as_deref()
                .and_then(|bytes| meta::first_value(bytes, key))
                .map(|value| String::from_utf8_lossy(value).into_owned())
        };
        Self {
            meta: bytes
                .as_deref()
                .map(|bytes| Meta::parse(&String::from_utf8_lossy(bytes))),
            config: row("config"),
            local_config: row(config::LOCAL_CONFIG_KEY),
            bytes,
        }
    }
}

/// The known agent, profile and client lists a refusal names.
fn known_lists(cfg: &IdentityConfig, agents: &[String]) -> (String, String, String) {
    let agent_names: Vec<&str> = agents.iter().map(String::as_str).collect();
    let profile_names: Vec<&str> = cfg.profiles.iter().map(|(name, _)| name.as_str()).collect();
    let client_names: Vec<&str> = cfg.clients.iter().map(|(name, _)| name.as_str()).collect();
    (
        known_list(&agent_names),
        known_list(&profile_names),
        known_list(&client_names),
    )
}

/// The refusal for the first recorded `client.<slot>` row in an unusable
/// shape, `None` when every row is usable or the meta does not read.
fn invalid_client_row_refusal(plan: &Plan, dir: &Path) -> Option<String> {
    let session = plan.name.as_deref().unwrap_or_default();
    let bytes = meta::read_bytes(dir).ok()?;
    let parsed = Meta::parse(&String::from_utf8_lossy(&bytes));
    parsed
        .roster()
        .iter()
        .find_map(|entry| match &entry.client {
            crate::meta::RecordedClient::Invalid => {
                Some(invalid_client_refusal(session, &entry.name, &entry.slot))
            }
            _ => None,
        })
}

/// The automatic identity and the judged successor table of a STOPPED
/// resume's recorded meta `bytes` (`None` for any other launch: no table).
/// A resume that reads its identity only for the table already holds the
/// automatic identity as `cfg`; any other takes it from [`automatic_identity`]
/// and moves no seat when that identity does not read.
fn automatic_successors(
    env: &Env,
    bytes: Option<&[u8]>,
    successors_only: bool,
    cfg: &IdentityConfig,
    home: Option<&Path>,
) -> (Option<IdentityConfig>, crate::successors::Successors) {
    let Some(bytes) = bytes else {
        return (None, crate::successors::Successors::default());
    };
    if successors_only {
        return (
            None,
            crate::run::pair_judge::recorded_successors(bytes, cfg, home),
        );
    }
    match automatic_identity(env, bytes) {
        Ok(automatic) => {
            let table = crate::run::pair_judge::recorded_successors(bytes, &automatic, home);
            (Some(automatic), table)
        }
        Err(_) => (None, crate::successors::Successors::default()),
    }
}

fn seat_override_snapshot(
    env: &Env,
    plan: &Plan,
    dir: &Path,
    resuming: bool,
    running: bool,
) -> Result<Option<SeatOverrideSnapshot>, SeatOverrideRefusal> {
    if let Some(line) = solo_resume_refusal(plan, dir, resuming) {
        return Err(SeatOverrideRefusal::Usage(line));
    }
    // A recorded `client.<slot>` row in an unusable SHAPE refuses here with
    // its own remedy — before the solo-freeze check below, whose generic
    // doubtful-roster refusal would otherwise claim every damaged row first.
    // Shape alone decides, so no config is needed; the config-absent leg
    // lives with the override checks further down. A running session takes
    // the running refusal instead, never this one.
    if resuming
        && !running
        && let Some(line) = invalid_client_row_refusal(plan, dir)
    {
        return Err(SeatOverrideRefusal::Failed(line));
    }
    let frozen_solo_identity = frozen_solo_identity(plan, dir, resuming, running)?;
    let read = RecordedRead::of(dir, resuming);
    let (recorded_bytes, recorded) = (&read.bytes, &read.meta);
    // A resume whose only reason to read the identity is a declared
    // `[profile_successors]` section answers `None` again below when no seat
    // moves, so a table that moves nothing changes no launch path.
    let successors_only = plan.seat_profiles.is_empty()
        && frozen_solo_identity.is_none()
        && !flagless_honor_needed(recorded.as_ref(), resuming, running);
    if successors_only && !(resuming && !running && successors_declared(dir)) {
        return Ok(None);
    }
    if running && !plan.seat_profiles.is_empty() {
        let session = plan.name.as_deref().unwrap_or_default();
        return Err(SeatOverrideRefusal::Usage(running_override_refusal(
            session,
        )));
    }

    // A resume that reads its identity only for the table reads the identity a
    // flagless stopped resume launches with; any other launch keeps the
    // identity its flags and carried labels were always judged on.
    let mut cfg = if successors_only {
        automatic_identity(env, recorded_bytes.as_deref().unwrap_or_default())?
    } else {
        override_identity(env, dir, resuming)?
    };
    if let Some((recorded_main, recorded_profile)) = frozen_solo_identity {
        freeze_solo_config(&mut cfg, plan, &recorded_main, &recorded_profile)?;
    }
    if let Some(line) = solo_worker_override_refusal(plan, &cfg) {
        return Err(SeatOverrideRefusal::Usage(line));
    }
    let agents = override_agents(&cfg, plan, dir, resuming);
    let (known_agents, known_profiles, known_clients) = known_lists(&cfg, &agents);
    let session = plan.name.as_deref().unwrap_or_default();
    // Every recorded seat's client row must be usable before any rewrite,
    // whether or not this launch names that seat: the meta rewrite below
    // would otherwise launder an Invalid row into a carried Label or drop it
    // toward Missing. A launch without overrides takes the same gate past the
    // lock, beside the doubtful-roster refusal.
    if let Some(parsed_meta) = recorded.as_ref() {
        for entry in parsed_meta.roster() {
            recorded_selection(session, &cfg, entry).map_err(SeatOverrideRefusal::Failed)?;
        }
    }

    let mut overrides = Vec::with_capacity(plan.seat_profiles.len());
    let home = crate::doors::home();
    // Every automatic decision is taken on the identity a flagless stopped
    // resume launches with ([`automatic_identity`]); an identity that does
    // not read leaves every seat on its recorded profile.
    let stopped_resume = resuming && !running;
    let (automatic, successors) = automatic_successors(
        env,
        recorded_bytes.as_deref().filter(|_| stopped_resume),
        successors_only,
        &cfg,
        home.as_deref(),
    );
    let auto_cfg = automatic.as_ref().unwrap_or(&cfg);
    let ctx = OverrideCtx {
        cfg: &cfg,
        agents: &agents,
        known_agents: &known_agents,
        known_profiles: &known_profiles,
        known_clients: &known_clients,
        recorded: recorded.as_ref(),
        session,
        home: home.as_deref(),
        state_root: &env.home,
        successors: &successors,
        recorded_config: read.config.as_deref(),
        recorded_local_config: read.local_config.as_deref(),
    };
    let auto = OverrideCtx {
        cfg: auto_cfg,
        ..ctx
    };
    for (agent, value) in &plan.seat_profiles {
        overrides.push(resolve_one_override(&ctx, agent, value)?);
    }
    // Flagless seats last: the explicit loop above already judged every seat
    // a flag names, so these are exactly the carried labels the flags left
    // alone — honored through the same resolve + re-take.
    overrides.extend(synthesize_recorded_honors(
        &ctx,
        &auto,
        &plan.seat_profiles,
    )?);
    // Then the unlabeled seats a usable successor moves, each as if its
    // `--seat` had been given; an explicit flag above always wins.
    overrides.extend(synthesize_successors(&auto, &plan.seat_profiles));
    if successors_only && overrides.is_empty() {
        return Ok(None);
    }
    for replacement in &overrides {
        if let Some((_, bound)) = cfg
            .roster
            .iter_mut()
            .find(|(name, _)| name == &replacement.agent)
        {
            bound.clone_from(&replacement.profile);
        }
    }
    Ok(Some(SeatOverrideSnapshot { cfg, overrides }))
}

/// The refusal a resume owes for a restored profile that no longer resolves,
/// decided READ-ONLY, before the migration chain writes the meta: the same
/// [`restore_saved_roster`] the launch runs, over a scratch copy of the seats
/// `cfg` plans. It reads only the `seat.<slot>` and `profile.<slot>` rows,
/// which the chain never moves. A config whose roster is not launchable is
/// refused here with the launch's own text, since that refusal would otherwise
/// come after the write.
fn restored_profiles_refusal(
    plan: &Plan,
    dir: &Path,
    cfg: &IdentityConfig,
    replaced: &[ResolvedSeatOverride],
) -> Option<String> {
    let mut planned = cfg.clone();
    if let Some(workers) = &plan.workers {
        planned.workers = Some(workers.clone());
    }
    let home = crate::doors::home();
    let mut seats = match config::launch_plan(&planned, plan.main.as_deref(), home.as_deref()) {
        Ok(resolved) => resolved.seats,
        Err(violations) => {
            return Some(config::render_violations(&violations).trim_end().to_owned());
        }
    };
    let session = plan.name.as_deref().unwrap_or_default();
    restore_saved_roster(dir, session, cfg, home.as_deref(), replaced, &mut seats).err()
}

/// Replace every command-derived field of `seat` with a resolved profile's.
fn rebind_command(
    seat: &mut Seat,
    command: &config::ResolvedCommand,
    parsed: &crate::launch_cmd::SimpleCommand,
) {
    command.clone_into(&mut seat.command);
    seat.assign_span.clone_from(&parsed.assign_span);
    seat.argv_span.clone_from(&parsed.argv_span);
    seat.binary.clone_from(&parsed.binary);
    seat.tool = parsed.tool();
}

/// Replace one resolved seat's profile and every command-derived field.
fn reprofile_seat(seat: &mut Seat, replacement: &ResolvedSeatOverride) {
    replacement.profile.clone_into(&mut seat.profile);
    seat.client_override.clone_from(&replacement.client);
    seat.store_proven = replacement.store_proven;
    rebind_command(seat, &replacement.command, &replacement.parsed);
}

/// A RESUME RESTORES THE ROSTER IT SAVED: each seat's recorded name and profile
/// replace what the current config binds, and the seat's command-derived facts
/// (command, spans, binary, tool) are re-resolved from that RESTORED profile —
/// they are what `agent_bin.<slot>` and every launch decision read, so they must
/// name the tool the recorded profile actually launches, never the tool the
/// config's own binding for the slot would. A seat an explicit or recorded-label
/// override will replace is left to that override.
///
/// A `[profile_successors]` move was decided before the lifecycle lock, from
/// the meta as it was then: the seat must still record that SOURCE here, read
/// under the lock, or the move is stale (another launch moved or changed the
/// seat meanwhile) and the resume refuses, so a move and its journal row are
/// only ever made from the meta they rewrite.
///
/// # Errors
///
/// The refusal line for a restored profile that no longer resolves, or for a
/// stale successor move, before any write.
pub(super) fn restore_saved_roster(
    dir: &Path,
    session: &str,
    cfg: &IdentityConfig,
    home: Option<&Path>,
    overrides: &[ResolvedSeatOverride],
    seats: &mut [Seat],
) -> Result<(), String> {
    let locked = overrides
        .iter()
        .any(|replacement| replacement.decided_on.is_some())
        .then(|| meta::read_bytes(dir).ok())
        .flatten();
    for seat in seats {
        if let Some(saved) =
            meta_value(dir, &format!("seat.{}", seat.slot)).filter(|v| !v.is_empty())
        {
            seat.name = saved;
        }
        let saved = meta_value(dir, &format!("profile.{}", seat.slot)).filter(|v| !v.is_empty());
        let replacement = overrides
            .iter()
            .find(|replacement| replacement.agent == seat.name);
        if let Some(decided) = replacement.and_then(|moved| moved.decided_on.as_deref())
            && (decided.entry.slot != seat.slot
                || !locked
                    .as_deref()
                    .is_some_and(|bytes| decided.holds_in(bytes)))
        {
            return Err(format!(
                "Error: session '{session}' seat '{}' ({}) changed its recorded seat while this \
                 launch was starting — nothing was resumed; retry the launch.",
                seat.name, seat.slot
            ));
        }
        let Some(saved) = saved else {
            continue;
        };
        if saved != seat.profile && replacement.is_none() {
            let refuse = |cause: String| {
                format!(
                    "Error: session '{session}' cannot resume: seat '{}' ({}) records profile \
                     '{saved}', but {cause} — nothing was resumed. Restore the profile in the \
                     config, or resume with an explicit override (ae {session} --seat {}=<profile>).",
                    seat.name, seat.slot, seat.name
                )
            };
            let command = cfg
                .command(&saved, home)
                .map_err(|why| refuse(why.to_string()))?;
            let command =
                command.ok_or_else(|| refuse("[profiles] no longer defines it".into()))?;
            let parsed = crate::launch_cmd::lex_simple_command(command.as_str())
                .map_err(|why| refuse(format!("it is not one simple command — it has {why}")))?;
            rebind_command(seat, &command, &parsed);
        }
        saved.clone_into(&mut seat.profile);
    }
    Ok(())
}

/// Apply already-preflighted overrides to final, possibly restored seats.
pub(super) fn apply_seat_overrides(
    snapshot: &SeatOverrideSnapshot,
    seats: &mut [Seat],
) -> Result<(), String> {
    for replacement in &snapshot.overrides {
        let Some(seat) = seats.iter_mut().find(|seat| seat.name == replacement.agent) else {
            return Err(format!(
                "Error: launch agent '{}' disappeared after preflight.",
                replacement.agent
            ));
        };
        reprofile_seat(seat, replacement);
    }
    Ok(())
}

#[cfg(test)]
#[path = "vendorblock_launch_spec.rs"]
mod vendorblock_launch_spec;

#[cfg(test)]
mod decided_on_tests {
    use super::DecidedOn;
    use crate::meta::Meta;

    const RECORDED: &str = "schema=2\nconfig=/g/config\nlocal_config=/l/config\norigin=/o\nseat.main=lead\n\
        profile.main=legacy\nagent_bin.main=codex\nharness_session.main=sid-1\n";

    fn decided() -> DecidedOn {
        let recorded = Meta::parse(RECORDED);
        let entry = recorded
            .roster()
            .iter()
            .find(|entry| entry.slot == "main")
            .cloned()
            .expect("recorded main seat");
        DecidedOn {
            entry,
            config: Some("/g/config".to_owned()),
            origin: recorded.origin().map(ToOwned::to_owned),
            local_config: Some("/l/config".to_owned()),
        }
    }

    #[test]
    fn the_unchanged_record_holds() {
        assert!(decided().holds_in(RECORDED.as_bytes()));
    }

    #[test]
    fn any_changed_seat_fact_or_config_file_makes_the_move_stale() {
        for (from, to) in [
            ("profile.main=legacy", "profile.main=next"),
            ("agent_bin.main=codex", "agent_bin.main=claude"),
            ("harness_session.main=sid-1", "harness_session.main=sid-2"),
            ("seat.main=lead", "seat.main=other"),
            ("config=/g/config", "config=/g/other"),
            ("origin=/o", "origin=/p"),
            ("local_config=/l/config", "local_config=/l/other"),
            ("local_config=/l/config\n", ""),
        ] {
            let locked = RECORDED.replace(from, to);
            assert!(!decided().holds_in(locked.as_bytes()), "{to}");
        }
        let labeled = format!("{RECORDED}client.main=primary\n");
        assert!(!decided().holds_in(labeled.as_bytes()));
    }
}

#[cfg(test)]
mod automatic_identity_tests {
    use super::{Env, Plan, seat_override_snapshot};
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let root = std::env::temp_dir().join(format!(
            "ae-autoidentity-unit-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch root");
        root
    }

    /// A stopped resume with an explicit `--seat` flag on one seat still takes
    /// the automatic successor move of the other seat on the recorded GLOBAL
    /// identity: the dedicated orchestrator overlay redefines the successor
    /// on another harness, and that legacy row must not refuse the move. The
    /// explicit flag still resolves on its normal override identity, overlay
    /// included.
    #[test]
    fn a_flagged_resume_decides_the_automatic_move_without_the_orchestrator_overlay() {
        let root = scratch("overlay");
        let home = root.join("home");
        let dir = home.join("sessions").join("s");
        std::fs::create_dir_all(&dir).expect("session dir");
        let global = root.join("config");
        std::fs::write(
            &global,
            "[clients]\nprimary = codex\n[profiles]\nlegacy = \"primary -m old\"\n\
             next = \"primary -m new\"\nmate-old = \"primary -m mate-old\"\n\
             mate-new = \"primary -m mate-new\"\n[roster]\nlead = legacy\nmate = mate-old\n\
             [workspace]\nmain = lead\nworkers = mate\n[profile_successors]\nlegacy = next\n",
        )
        .expect("global config");
        let overlay = home.join(crate::meta::SEAT_CONFIG_FILE);
        std::fs::write(
            &overlay,
            "[profiles]\nnext = \"claude --model new\"\nmate-new = \"primary -m mate-overlay\"\n",
        )
        .expect("orchestrator overlay");
        std::fs::write(
            dir.join("meta"),
            format!(
                "schema=2\nmeta_version=2\nmode=local\nsession=s\nsession_id=SID\ncreated=1\n\
                 started=1\nwork_dir=/w\norigin=/o\nconfig={}\nlocal_config={}\n\
                 seat.main=lead\nprofile.main=legacy\nagent_bin.main=codex\n\
                 harness_session.main=sid-main\nseat.worker.0=mate\nprofile.worker.0=mate-old\n\
                 agent_bin.worker.0=codex\nharness_session.worker.0=sid-mate\n",
                global.display(),
                overlay.display()
            ),
        )
        .expect("meta");
        let env = Env {
            home: home.clone(),
            cwd: PathBuf::from("/w"),
            global: Some(global),
            local: None,
            server_kind: String::new(),
            server_value: String::new(),
            caller_server: None,
            inside_tmux: false,
            attach: true,
            core: None,
            core_version: None,
            no_autostart: true,
            starter: super::super::Starter::Caller,
            test_pre_lock_marker: None,
        };
        let plan = Plan {
            name: Some("s".to_owned()),
            seat_profiles: vec![("mate".to_owned(), "mate-new".to_owned())],
            ..Plan::default()
        };
        let snapshot = seat_override_snapshot(&env, &plan, &dir, true, false);
        let _ = std::fs::remove_dir_all(&root);
        let moved: Vec<(String, String, Option<String>, String)> = snapshot
            .expect("the flagged resume is accepted")
            .expect("the flag yields a snapshot")
            .overrides
            .iter()
            .map(|seat| {
                (
                    seat.agent.clone(),
                    seat.profile.clone(),
                    seat.successor_of.clone(),
                    seat.command
                        .as_str()
                        .split_whitespace()
                        .last()
                        .unwrap_or_default()
                        .to_owned(),
                )
            })
            .collect();
        assert_eq!(
            moved,
            vec![
                (
                    "mate".to_owned(),
                    "mate-new".to_owned(),
                    None,
                    "mate-overlay".to_owned()
                ),
                (
                    "lead".to_owned(),
                    "next".to_owned(),
                    Some("legacy".to_owned()),
                    "new".to_owned()
                ),
            ]
        );
    }
}
