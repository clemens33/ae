//! Seat overrides: the command-line profile and client selections, their resolution against the roster and the recorded session.

use super::{
    Env, IdentityConfig, Meta, Path, PathBuf, Plan, Seat, ToolKind, config, meta, meta_value,
    node_exists, roster,
};

/// One command-line seat override that could not be honored.
#[derive(Debug)]
pub(super) enum SeatOverrideRefusal {
    /// The user named a profile or launch seat that does not exist, or asked a
    /// stopped conversation to cross harnesses.
    Usage(String),
    /// The selected config or profile command itself is not launchable.
    Failed(String),
}

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
             (e.g. --seat lead=solx, --lead fablex@cc-mic)."
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
        .is_some_and(|local| crate::orchestrator::is_seat_overlay(local, &env.home));
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

/// Resolve one `profile@client` selection: both sides must exist, the pair
/// must substitute — and then the R1 gate: BOTH binaries must be the SAME
/// KNOWN harness. `Unknown` on either side refuses, because
/// `ToolKind::from_binary_name` is `from_known_binary_name(...).unwrap_or(Unknown)`
/// and any two unknowns would compare equal through it.
pub(super) fn resolve_client_override(
    cfg: &IdentityConfig,
    agent: &str,
    profile: &str,
    label: &str,
    home: Option<&Path>,
    known_profiles: &str,
    known_clients: &str,
) -> Result<(config::ResolvedCommand, crate::launch_cmd::SimpleCommand), SeatOverrideRefusal> {
    let resolved = cfg
        .command_with_client(profile, label, home)
        .map_err(|why| match why {
            config::OverrideError::UnknownProfile => SeatOverrideRefusal::Usage(format!(
                "Error: unknown profile '{profile}' in --seat. Known profiles: {known_profiles}."
            )),
            config::OverrideError::UnknownClient => SeatOverrideRefusal::Usage(format!(
                "Error: unknown client '{label}' in --seat. Known clients: {known_clients}."
            )),
            config::OverrideError::ProfileNotSimple(why) => SeatOverrideRefusal::Failed(format!(
                "Error: [profiles] {profile} (seat '{agent}'): the launch command must be one simple command — it has {why}."
            )),
            config::OverrideError::Refused(why) => SeatOverrideRefusal::Failed(why.to_string()),
        })?;
    let parsed = crate::launch_cmd::lex_simple_command(resolved.command.as_str()).map_err(|why| {
        SeatOverrideRefusal::Failed(format!(
            "Error: [profiles] {profile} (seat '{agent}'): the launch command must be one simple command — it has {why}."
        ))
    })?;
    let original = ToolKind::from_known_binary_name(&resolved.original_binary);
    let switched = ToolKind::from_known_binary_name(&parsed.binary);
    if original.is_none() || switched.is_none() || original != switched {
        let from = match resolved.original_client.as_deref() {
            Some(client) => format!("profile '{profile}' (client '{client}')"),
            None => format!("profile '{profile}'"),
        };
        return Err(SeatOverrideRefusal::Usage(client_adapter_refusal(
            agent,
            profile,
            label,
            &from,
            (&resolved.original_binary, original),
            (&parsed.binary, switched),
        )));
    }
    Ok((resolved.command, parsed))
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

/// What one side of an `@` pair runs, for the refusal's two values.
fn adapter_word(binary: &str, known: Option<ToolKind>) -> String {
    match known {
        Some(kind) => kind.as_str().to_owned(),
        None => format!("an unrecognized binary '{binary}'"),
    }
}

fn client_adapter_refusal(
    agent: &str,
    profile: &str,
    label: &str,
    from: &str,
    original: (&str, Option<ToolKind>),
    switched: (&str, Option<ToolKind>),
) -> String {
    format!(
        "Error: cannot launch seat '{agent}' as '{profile}@{label}': {from} runs {} but client '{label}' runs {} — \
         a client override may change the account, never the harness adapter ('@' needs the same known harness on both sides). \
         Use a duplicate profile to cross harnesses.",
        adapter_word(original.0, original.1),
        adapter_word(switched.0, switched.1)
    )
}

/// A recorded `client.<slot>` row resolved against the CURRENT config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RecordedSelection {
    /// No row: no override was recorded for this seat.
    Missing,
    /// Exactly this label was recorded, and it still names a client.
    Label(String),
}

/// A recorded row no launch may interpret: empty, duplicated or malformed.
pub(super) fn invalid_client_refusal(session: &str, agent: &str, slot: &str) -> String {
    format!(
        "Error: session '{session}' seat '{agent}' records an unusable client (client.{slot} is empty, duplicated or malformed) — \
         fix the meta row, or end the session (ae end {session})."
    )
}

/// A recorded label that selects a different client than this launch.
pub(super) fn different_label_refusal(
    session: &str,
    agent: &str,
    profile: &str,
    known: &str,
    label: &str,
) -> String {
    format!(
        "Error: session '{session}' seat '{agent}' recorded client override '{known}' but this launch selects '{label}' — \
         a recorded client is write-once. Resume with '{profile}@{known}', restore the '{known}' client, or end the session (ae end {session})."
    )
}

/// Resolve one recorded seat's client row, or refuse with its remedy.
///
/// Shape-Invalid and config-absent are BOTH unusable, but only the absent
/// label has a cheap remedy: restoring the removed or renamed `[clients]` row
/// is safe, because the row then reads `Label` again with its identity
/// unchanged. A malformed row must be fixed by hand. Either way `ae end` is
/// the last resort, never a silent fallback.
pub(super) fn recorded_selection(
    session: &str,
    cfg: &IdentityConfig,
    entry: &meta::RosterEntry,
) -> Result<RecordedSelection, String> {
    match &entry.client {
        crate::meta::RecordedClient::Missing => Ok(RecordedSelection::Missing),
        crate::meta::RecordedClient::Label(label) if cfg.client(label).is_some() => {
            Ok(RecordedSelection::Label(label.clone()))
        }
        crate::meta::RecordedClient::Label(label) => Err(format!(
            "Error: session '{session}' seat '{}' recorded client override '{label}' but no [clients] row names it now — \
             restore the '{label}' client, or end the session (ae end {session}).",
            entry.name
        )),
        crate::meta::RecordedClient::Invalid => {
            Err(invalid_client_refusal(session, &entry.name, &entry.slot))
        }
    }
}

/// Refuse an override the recorded seat cannot honor.
///
/// A recorded client is write-once: only its exact label is accepted, any
/// different label refuses whatever its facts (config is mutable, so an
/// equality proof cannot cross a config edit), and a bare profile cannot drop
/// it. Whatever the labels say, the recorded STORE triple is re-taken against
/// the current resolution, which catches a label whose definition moved. The
/// `bool` answers whether that re-take PROVED the pairing — `false` under an
/// `Unknown` current resolution, which honors the seat but records nothing.
#[allow(
    clippy::too_many_arguments,
    reason = "one refusal site for one override against one recorded seat"
)]
pub(super) fn refuse_recorded_client_conflict(
    session: &str,
    agent: &str,
    profile: &str,
    client: Option<&str>,
    entry: &meta::RosterEntry,
    cfg: &IdentityConfig,
    command: &config::ResolvedCommand,
    tool: ToolKind,
    home: Option<&Path>,
) -> Result<bool, SeatOverrideRefusal> {
    let recorded = recorded_selection(session, cfg, entry).map_err(SeatOverrideRefusal::Failed)?;
    match (recorded, client) {
        // No recorded override and none selected: the legacy path, untouched.
        (RecordedSelection::Missing, None) => Ok(true),
        // First override for this seat: the recorded store still rules.
        (RecordedSelection::Missing, Some(label)) => refuse_store_conflict(
            session,
            agent,
            &format!("{profile}@{label}"),
            entry,
            command,
            tool,
            home,
        ),
        // A bare profile names no label, so it cannot satisfy write-once.
        (RecordedSelection::Label(known), None) => Err(SeatOverrideRefusal::Usage(format!(
            "Error: session '{session}' seat '{agent}' recorded client override '{known}' — \
             re-pair with '{profile}@{known}' to keep it, or end the session (ae end {session}). \
             A bare profile cannot drop a recorded client."
        ))),
        // The exact label, re-proved against the current store below.
        (RecordedSelection::Label(known), Some(label)) if known == label => refuse_store_conflict(
            session,
            agent,
            &format!("{profile}@{label}"),
            entry,
            command,
            tool,
            home,
        ),
        // Any different label refuses, whatever its facts today.
        (RecordedSelection::Label(known), Some(label)) => Err(SeatOverrideRefusal::Usage(
            different_label_refusal(session, agent, profile, &known, label),
        )),
    }
}

/// One side of the R4 identity comparison, as the refusal names it.
fn recorded_store_shown(
    home: &crate::meta::RecordedConfigHome,
    base: &crate::meta::RecordedConfigHomeBase,
) -> String {
    match home {
        crate::meta::RecordedConfigHome::Path(path) => {
            format!("{} (explicit)", path.display())
        }
        crate::meta::RecordedConfigHome::Implicit(path) => match base {
            crate::meta::RecordedConfigHomeBase::Path(home) => {
                format!("{} (implicit, base {})", path.display(), home.display())
            }
            _ => format!("{} (implicit)", path.display()),
        },
        crate::meta::RecordedConfigHome::Absent => "absent".to_owned(),
        crate::meta::RecordedConfigHome::Unknown
        | crate::meta::RecordedConfigHome::Invalid
        | crate::meta::RecordedConfigHome::Missing => "unusable".to_owned(),
    }
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

/// F6: an override whose current resolution is `Unknown` on a seat with no
/// retained store refuses — proceeding would ignore an explicit flag while
/// recording nothing recoverable, and a first start strands nothing.
fn unknown_store_refusal(agent: &str, spelling: &str, reason: &str) -> SeatOverrideRefusal {
    SeatOverrideRefusal::Usage(format!(
        "Error: cannot launch seat '{agent}' as '{spelling}': the override resolves to an unknown conversation store ({reason}) — \
         the launcher cannot verify where this conversation would live. Make the account path absolute, then retry."
    ))
}

/// Re-take the R4 identity check for one override: the recorded store triple
/// (MODE + PATH + BASE — never path alone, an implicit and an explicit row
/// over one path are different accounts) against the CURRENT resolution of
/// the command this launch would exec.
///
/// The lookup is controlled, not ambient: `HOME` is the launch home, every
/// other variable unset. An explicit client assigns its own variable, so its
/// resolution never consults the environment at all; an implicit one needs
/// exactly `HOME`. A CURRENT resolution of `Unknown` (a pane-only `$VAR` the
/// launcher cannot see) proves no conflict — `_run` retains the recorded
/// store with a notice, so the seat stays safe without this gate refusing —
/// but it proves no pairing either, and the `false` carries exactly that: the
/// caller honors the seat and records no new label fact. That mercy belongs
/// to a RESUME — a seat with a retained store to proceed on. A first start
/// (no recorded store) that resolves `Unknown` refuses outright instead: F6,
/// because proceeding would ignore an explicit flag and strand nothing.
pub(super) fn refuse_store_conflict(
    session: &str,
    agent: &str,
    spelling: &str,
    entry: &meta::RosterEntry,
    command: &config::ResolvedCommand,
    tool: ToolKind,
    home: Option<&Path>,
) -> Result<bool, SeatOverrideRefusal> {
    if entry.config_home == crate::meta::RecordedConfigHome::Invalid {
        return Err(SeatOverrideRefusal::Failed(format!(
            "Error: cannot resume seat '{agent}' as '{spelling}': seat '{}' has malformed or duplicate config_home metadata — \
             fix the meta row, or end the session (ae end {session}).",
            entry.slot
        )));
    }
    if entry.config_home == crate::meta::RecordedConfigHome::Unknown {
        return Err(SeatOverrideRefusal::Failed(format!(
            "Error: cannot resume seat '{agent}' as '{spelling}': the recorded conversation store (config_home.{}) is unknown — \
             end the session and start over (ae end {session}).",
            entry.slot
        )));
    }
    let home_value = home.map(|path| path.display().to_string());
    let resolution = crate::launch_cmd::config_home_resolution(command, tool, &|name| {
        if name == "HOME" {
            home_value.clone()
        } else {
            None
        }
    });
    if entry.config_home == crate::meta::RecordedConfigHome::Missing {
        // First start for this seat: the override participates in the one
        // resolution and is recorded normally. But an `Unknown` resolution
        // refuses — F6: nothing retained means nothing stranded, and
        // proceeding would silently ignore the explicit flag.
        if let crate::launch_cmd::Resolved::Unknown(reason) = &resolution.home {
            return Err(unknown_store_refusal(agent, spelling, reason));
        }
        return Ok(true);
    }
    let current =
        crate::run::canonical_config_home(&resolution.home).map_err(SeatOverrideRefusal::Failed)?;
    let current_base =
        crate::run::canonical_config_home(&resolution.base).map_err(SeatOverrideRefusal::Failed)?;
    if matches!(current, crate::launch_cmd::Resolved::Unknown(_)) {
        return Ok(false);
    }
    let matches = match (&entry.config_home, &current) {
        (crate::meta::RecordedConfigHome::Absent, crate::launch_cmd::Resolved::Absent) => true,
        (
            crate::meta::RecordedConfigHome::Path(recorded),
            crate::launch_cmd::Resolved::Path(now),
        ) => resolution.explicit && recorded == now,
        (
            crate::meta::RecordedConfigHome::Implicit(recorded),
            crate::launch_cmd::Resolved::Path(now),
        ) => {
            !resolution.explicit
                && recorded == now
                && matches!(
                    (&entry.config_home_base, &current_base),
                    (
                        crate::meta::RecordedConfigHomeBase::Path(recorded),
                        crate::launch_cmd::Resolved::Path(now)
                    ) if recorded == now
                )
        }
        _ => false,
    };
    if matches {
        return Ok(true);
    }
    let now = match (&current, resolution.explicit) {
        (crate::launch_cmd::Resolved::Path(path), true) => {
            format!("{} (explicit)", path.display())
        }
        (crate::launch_cmd::Resolved::Path(path), false) => match &current_base {
            crate::launch_cmd::Resolved::Path(base) => {
                format!("{} (implicit, base {})", path.display(), base.display())
            }
            _ => format!("{} (implicit)", path.display()),
        },
        (crate::launch_cmd::Resolved::Absent, _) => "absent".to_owned(),
        (crate::launch_cmd::Resolved::Unknown(_), _) => "unresolvable".to_owned(),
    };
    // A store the pane selects through an exported account variable reads
    // exactly like a mode flip here: the launcher cannot see pane-only
    // environment, so the refusal names the escape — pinning the store in the
    // row makes the resolution observable and the refusal exact again.
    let exported_hint = if !resolution.explicit
        && matches!(entry.config_home, crate::meta::RecordedConfigHome::Path(_))
    {
        " If this seat's store is selected by an exported account variable the launcher cannot see, \
         pin it in the [clients] row as config_home=<path> instead."
    } else {
        ""
    };
    Err(SeatOverrideRefusal::Usage(format!(
        "Error: cannot resume seat '{agent}' as '{spelling}': the recorded conversation lives in {}, \
         but this launch resolves to {now} — a client override cannot move a retained conversation. \
         End the session to adopt the new account (ae end {session}).{exported_hint}",
        recorded_store_shown(&entry.config_home, &entry.config_home_base)
    )))
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
    })
}

/// A resumed seat whose command now launches another tool than it recorded.
pub(super) fn cross_tool_refusal(agent: &str, old: ToolKind, new: ToolKind) -> String {
    format!(
        "Error: cannot change agent '{agent}' from {} to {} on resume — its recorded conversation cannot cross tool kinds.",
        old.as_str(),
        new.as_str()
    )
}

/// Validate every explicit seat profile before the launch's first write.
/// Comma list of known names for a refusal, or `<none>` when empty.
pub(super) fn known_list(names: &[&str]) -> String {
    if names.is_empty() {
        "<none>".to_owned()
    } else {
        names.join(", ")
    }
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
fn synthesize_recorded_honors(
    ctx: &OverrideCtx,
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
        let Some(profile) = entry.profile.as_deref().filter(|name| !name.is_empty()) else {
            return Err(SeatOverrideRefusal::Failed(format!(
                "Error: session '{}' seat '{}' recorded client override '{label}' but no profile — \
                 fix the meta row, or end the session (ae end {}).",
                ctx.session, entry.name, ctx.session
            )));
        };
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
        });
    }
    Ok(honored)
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
    if resuming && !running {
        let session = plan.name.as_deref().unwrap_or_default();
        let refusal = meta::read_bytes(dir)
            .ok()
            .map(|bytes| Meta::parse(&String::from_utf8_lossy(&bytes)))
            .as_ref()
            .and_then(|parsed| {
                parsed
                    .roster()
                    .iter()
                    .find_map(|entry| match &entry.client {
                        crate::meta::RecordedClient::Invalid => {
                            Some(invalid_client_refusal(session, &entry.name, &entry.slot))
                        }
                        _ => None,
                    })
            });
        if let Some(line) = refusal {
            return Err(SeatOverrideRefusal::Failed(line));
        }
    }
    let frozen_solo_identity = frozen_solo_identity(plan, dir, resuming, running)?;
    let recorded = if resuming {
        meta::read_bytes(dir)
            .ok()
            .map(|bytes| Meta::parse(&String::from_utf8_lossy(&bytes)))
    } else {
        None
    };
    if plan.seat_profiles.is_empty()
        && frozen_solo_identity.is_none()
        && !flagless_honor_needed(recorded.as_ref(), resuming, running)
    {
        return Ok(None);
    }
    if running && !plan.seat_profiles.is_empty() {
        let session = plan.name.as_deref().unwrap_or_default();
        return Err(SeatOverrideRefusal::Usage(running_override_refusal(
            session,
        )));
    }

    let mut cfg = override_identity(env, dir, resuming)?;
    if let Some((recorded_main, recorded_profile)) = frozen_solo_identity {
        freeze_solo_config(&mut cfg, plan, &recorded_main, &recorded_profile)?;
    }
    if let Some(line) = solo_worker_override_refusal(plan, &cfg) {
        return Err(SeatOverrideRefusal::Usage(line));
    }
    let agents = override_agents(&cfg, plan, dir, resuming);
    let agent_names: Vec<&str> = agents.iter().map(String::as_str).collect();
    let known_agents = known_list(&agent_names);
    let profile_names: Vec<&str> = cfg
        .profiles
        .iter()
        .map(|(profile, _)| profile.as_str())
        .collect();
    let known_profiles = known_list(&profile_names);
    let client_names: Vec<&str> = cfg
        .clients
        .iter()
        .map(|(client, _)| client.as_str())
        .collect();
    let known_clients = known_list(&client_names);
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
    let ctx = OverrideCtx {
        cfg: &cfg,
        agents: &agents,
        known_agents: &known_agents,
        known_profiles: &known_profiles,
        known_clients: &known_clients,
        recorded: recorded.as_ref(),
        session,
        home: home.as_deref(),
    };
    for (agent, value) in &plan.seat_profiles {
        overrides.push(resolve_one_override(&ctx, agent, value)?);
    }
    // Flagless seats last: the explicit loop above already judged every seat
    // a flag names, so these are exactly the carried labels the flags left
    // alone — honored through the same resolve + re-take.
    overrides.extend(synthesize_recorded_honors(&ctx, &plan.seat_profiles)?);
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
    let replaced: Vec<&str> = replaced.iter().map(|o| o.agent.as_str()).collect();
    restore_saved_roster(dir, session, cfg, home.as_deref(), &replaced, &mut seats).err()
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
/// # Errors
///
/// The refusal line for a restored profile that no longer resolves, before any
/// write.
pub(super) fn restore_saved_roster(
    dir: &Path,
    session: &str,
    cfg: &IdentityConfig,
    home: Option<&Path>,
    overridden: &[&str],
    seats: &mut [Seat],
) -> Result<(), String> {
    for seat in seats {
        if let Some(saved) =
            meta_value(dir, &format!("seat.{}", seat.slot)).filter(|v| !v.is_empty())
        {
            seat.name = saved;
        }
        let Some(saved) =
            meta_value(dir, &format!("profile.{}", seat.slot)).filter(|v| !v.is_empty())
        else {
            continue;
        };
        if saved != seat.profile && !overridden.contains(&seat.name.as_str()) {
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
