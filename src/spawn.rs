//! `_spawn` and `_retire`: adding an agent to a live workspace, and removing
//! one — decisions, state AND tmux effects, in one operation.
//!
//! Ported from `ae`'s `_cmd_spawn`, `_spawn_rollback`, `_cmd_retire` and the
//! `send_agent_cmd` launch path they share. The composition half (what the pane
//! is told to run) is [`crate::launch`]; this module is the ORDER, and the
//! order is the contract:
//!
//! 1. the SEAT is reserved in meta BEFORE the pane exists, so the roster is
//!    never racy and a concurrent spawn cannot take the same index;
//! 2. the window is created, its pane stamped and its window renamed;
//! 3. `workspace.md` is regenerated from the live panes;
//! 4. the slot's start marker is claimed and the pane command is pasted into
//!    the pane's shell, where the core composes the agent and becomes it;
//! 5. the BRIEF rides the launch turn on the UserTurn channel, else is
//!    delivered only after the TUI proves it will accept input.
//!
//! A failure before the pane can launch ROLLS BACK: the seat goes, the launch
//! artifacts go, and the pane is killed through the ownership guard. A brief
//! delivery failure is different: its pane is live, so it keeps the seat's
//! `spawn` record and appends a distinct `spawn-failed` diagnosis.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::deliver::{self, Shape};
use crate::inventory::ServerId;
use crate::launch;
use crate::meta::{self, Meta, ServerSelector};
use crate::session_launch::capture;
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::store;
use crate::time::Timestamp;
use crate::tool::ToolKind;
use crate::tracked::{self, EventFields};
use crate::transport;
use crate::watchdog_glue;

/// The usage line.
pub const SPAWN_USAGE: &str = "Usage: spawn <name> --using <profile> [--] [prompt]";

/// The `retire` usage.
pub const RETIRE_USAGE: &str = "Usage: retire <agent-name|pane-id>";

/// How long to wait for a new pane's shell before pasting.
const SHELL_SETTLE: Duration = Duration::from_millis(300);

/// How many polls the brief's readiness wait takes, at half a second each.
const BRIEF_READY_POLLS: u32 = 30;

/// How many polls the launch's process wait takes.
const START_POLLS: u32 = 10;

/// The pause between those polls.
const START_POLL: Duration = Duration::from_millis(100);

/// How long to let a booting TUI swallow the Enter before looking for staged
/// text.
const LINGER_SETTLE: Duration = Duration::from_millis(1500);

/// How much of the brief the lingering check looks for on screen.
const LINGER_PREFIX: usize = 40;

/// The event action a completed spawn records.
const SPAWN_ACTION: &str = "spawn";

/// The event action a spawn whose brief never landed records.
const SPAWN_FAILED_ACTION: &str = "spawn-failed";

/// The event action a retire records.
const RETIRE_ACTION: &str = "retire";

// ---- argv -----------------------------------------------------------------

/// What a `_spawn` argv said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// The new agent's name — REQUIRED, and its identity.
    pub name: String,
    /// The `[profiles]` recipe to launch it with.
    pub profile: String,
    /// The task, as typed.
    pub prompt: String,
}

/// Parse `<name> --using <profile> [--] [prompt…]`.
///
/// # Errors
///
/// The usage line to print, when the name is missing or option-shaped, when
/// `--using` has no value, or when no profile was named at all.
pub fn parse(tail: &[String]) -> Result<Parsed, String> {
    let [name, rest @ ..] = tail else {
        return Err(SPAWN_USAGE.to_owned());
    };
    if name.starts_with('-') {
        return Err(SPAWN_USAGE.to_owned());
    }
    let mut profile = String::new();
    let mut rest = rest;
    loop {
        match rest {
            [flag, value, tail @ ..] if flag == "--using" => {
                profile.clone_from(value);
                rest = tail;
            }
            [flag] if flag == "--using" => {
                return Err("Error: --using requires a profile name.".to_owned());
            }
            [flag, tail @ ..] if flag.starts_with("--using=") => {
                flag["--using=".len()..].clone_into(&mut profile);
                rest = tail;
            }
            [flag, tail @ ..] if flag == "--" => {
                rest = tail;
                break;
            }
            _ => break,
        }
    }
    if profile.is_empty() {
        return Err(
            "Error: spawn needs --using <profile> (the profiles are listed in workspace.md)."
                .to_owned(),
        );
    }
    if profile.contains('@') {
        return Err(format!(
            "Error: --using '{profile}' names a client override with '@' — profile@client is launch-only \
             (ae <name> --lead/--colead/--seat). Spawn takes a bare profile."
        ));
    }
    Ok(Parsed {
        name: name.clone(),
        profile,
        prompt: rest.join(" "),
    })
}

// ---- the session's facts --------------------------------------------------

/// The session facts both operations read out of `meta`.
struct Facts {
    session: String,
    work_dir: String,
    origin: String,
    mode: String,
    main_pane: String,
    server: ServerId,
    /// The session's own config, when it still exists.
    global: Option<PathBuf>,
    /// The origin's `.ae/config`, which layers over it.
    local: Option<PathBuf>,
}

/// One `key=value` row's value, or empty.
fn row(bytes: &[u8], key: &str) -> String {
    meta::first_value(bytes, key)
        .map(|value| String::from_utf8_lossy(value).into_owned())
        .unwrap_or_default()
}

/// Read the six session facts, the target server and the config layering.
fn facts(dir: &Path) -> Result<Facts, String> {
    let bytes = meta::read_bytes(dir).map_err(|why| format!("cannot read the meta: {why}"))?;
    let session = row(&bytes, "session");
    if session.is_empty() {
        return Err("session not found in metadata".to_owned());
    }
    let origin = row(&bytes, "origin");
    let recorded = row(&bytes, "config");
    let global = (!recorded.is_empty()).then(|| PathBuf::from(&recorded));
    let local = (!origin.is_empty()).then(|| Path::new(&origin).join(".ae").join("config"));
    let server = match Meta::parse(&String::from_utf8_lossy(&bytes)).server_selector() {
        ServerSelector::Positive(selector) => ServerId::Selected(selector),
        _ => ServerId::Ambient,
    };
    Ok(Facts {
        session,
        work_dir: row(&bytes, "work_dir"),
        origin,
        mode: row(&bytes, "mode"),
        main_pane: row(&bytes, "main_pane"),
        server,
        global,
        local,
    })
}

impl Facts {
    /// The config files the renders read, in layering order.
    fn config_files(&self) -> Vec<PathBuf> {
        self.global
            .iter()
            .chain(self.local.iter())
            .cloned()
            .collect()
    }

    /// The layered `[profiles]`, with an ABSENT file read as absent.
    fn identity(&self) -> Result<crate::config::IdentityConfig, crate::config::ConfigError> {
        let mut global = self.global.as_deref();
        let mut local = self.local.as_deref();
        for _ in 0..2 {
            match crate::config::read_identity(global, local) {
                Err(crate::config::ConfigError::Unreadable(path)) => {
                    if local == Some(path.as_path()) {
                        local = None;
                    } else if global == Some(path.as_path()) {
                        global = None;
                    } else {
                        return Err(crate::config::ConfigError::Unreadable(path));
                    }
                }
                other => return other,
            }
        }
        crate::config::read_identity(global, local)
    }

    /// Rewrite `workspace.md` from the live panes.
    fn regenerate_manifest(&self, dir: &Path) {
        let document = crate::render::manifest_document(
            dir,
            &self.session,
            &self.work_dir,
            &self.origin,
            &self.mode,
            &self.main_pane,
            &self.config_files(),
        );
        let _ = std::fs::write(dir.join("workspace.md"), document);
    }
}

// ---- spawn ----------------------------------------------------------------

/// `_spawn <meta-dir> <name> --using <profile> [--] [prompt]`.
///
/// # Errors
///
/// Only a failure to write `out` or `err`. Every refusal is an exit code.
#[allow(clippy::too_many_lines, reason = "the frozen order, kept in one place")]
pub fn run_spawn(
    dir: &Path,
    tail: &[String],
    caller: &str,
    now: Timestamp,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let parsed = match parse(tail) {
        Ok(parsed) => parsed,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(EXIT_FAILED);
        }
    };
    // THE PEER BOUNDARY.
    if !crate::config::is_agent_name(&parsed.name) {
        writeln!(
            err,
            "Error: invalid agent name '{}'. Names must match {}.",
            parsed.name,
            crate::config::AGENT_NAME_GRAMMAR
        )?;
        return Ok(EXIT_FAILED);
    }
    let facts = match facts(dir) {
        Ok(facts) => facts,
        Err(why) => {
            writeln!(err, "Error: {why}")?;
            return Ok(EXIT_FAILED);
        }
    };
    if !transport::session_exists(&facts.server, &facts.session) {
        writeln!(err, "Error: session '{}' not running", facts.session)?;
        return Ok(EXIT_FAILED);
    }
    let cfg = match facts.identity() {
        Ok(cfg) => cfg,
        Err(why) => {
            writeln!(err, "{why}")?;
            return Ok(EXIT_FAILED);
        }
    };
    let home = crate::doors::home();
    let command = match cfg.command(&parsed.profile, home.as_deref()) {
        Ok(command) => command,
        Err(why) => {
            writeln!(err, "{why}")?;
            return Ok(EXIT_FAILED);
        }
    };
    let Some(command) = command else {
        writeln!(
            err,
            "Error: profile '{}' not defined in [profiles] of {}",
            parsed.profile,
            facts
                .global
                .as_ref()
                .map_or_else(String::new, |path| path.display().to_string())
        )?;
        return Ok(EXIT_FAILED);
    };
    // The brief the new agent is handed.
    let brief = if parsed.prompt.is_empty() {
        format!(
            "You were spawned into an ae workspace. Read {}/workspace.md for details.",
            dir.display()
        )
    } else if caller.is_empty() {
        parsed.prompt.clone()
    } else {
        format!(
            "{} — When done, reply back via: {}/send \"{caller}\" \"<your reply>\"",
            parsed.prompt,
            dir.display()
        )
    };

    // THE SAME GRAMMAR AS A LAUNCH SEAT, before any effect. config.rs enforces the
    // one-simple-command lexer for the initial roster, and a profile selected at
    // spawn is held to the same one: a value like `bad = "touch m; tail -f
    // /dev/null"` is REFUSED rather than run. Tool and binary come from the one
    // validated parse.
    let lexed = match crate::launch_cmd::lex_simple_command(command.as_str()) {
        Ok(lexed) => lexed,
        Err(why) => {
            writeln!(
                err,
                "Error: profile '{}' refused — {why}. Nothing was spawned.",
                parsed.profile
            )?;
            return Ok(EXIT_FAILED);
        }
    };
    let tool = lexed.tool();
    let binary = lexed.binary.clone();
    let session_id = if launch::takes_launch_session_id(tool) {
        launch::generate_uuid()
    } else {
        launch::PENDING.to_owned()
    };
    // Identity v2: the SEAT is the core's to allocate and write — the name
    // grammar, uniqueness and the lowest free index are decided under one hold
    // of the meta lock, BEFORE the pane exists, so the roster is never racy.
    let sid = (session_id != launch::PENDING).then_some(session_id.as_str());
    let slot =
        match crate::identity::add_seat_slot(dir, &parsed.name, &parsed.profile, &binary, sid) {
            Ok(slot) => slot,
            Err(why) => {
                writeln!(err, "Error: {why}")?;
                return Ok(EXIT_FAILED);
            }
        };
    // The launch id guards observed-model writes for every seat. Capture tools
    // also use it to distinguish their own stores, but marker injection stays
    // gated by the adapter capability. Record it before the pane exists so
    // `_run` can compose either use from the same durable identity.
    if meta::rewrite(
        dir,
        &format!("launch_id.{slot}"),
        Some(&crate::session_launch::launch_token(tool, None)),
    )
    .is_err()
    {
        writeln!(
            err,
            "ae: could not record the launch token of '{}'.",
            parsed.name
        )?;
    }
    // The capture lower bound is a BIRTH fact: publish it before the pane can
    // exec the tool. `launch_time` remains the post-exec lifecycle stamp.
    if tool.adapter().capture.is_needed()
        && meta::rewrite(
            dir,
            &format!("capture_floor.{slot}"),
            Some(&now.epoch().to_string()),
        )
        .is_err()
    {
        let _ = crate::identity::remove_seat_slot(dir, &parsed.name);
        writeln!(
            err,
            "Error: '{}' capture floor could not be recorded — nothing was spawned.",
            parsed.name
        )?;
        return Ok(EXIT_FAILED);
    }

    // THE LAUNCH-ATTEMPT STAMP, before the window this spawn is about to
    // create. A spawn is an ae launch into a tmux server exactly as a resume
    // is, so it owes the same evidence — see [`crate::store::LAUNCH_ATTEMPT`].
    // CHECKED: an unrecorded attempt would make a later reboot proof read this
    // session as untouched since the boot, so nothing is spawned instead.
    if let Err(why) = crate::store::open(dir).stamp_launch_attempt(now.epoch()) {
        let _ = crate::identity::remove_seat_slot(dir, &parsed.name);
        writeln!(
            err,
            "Error: '{}' launch attempt could not be recorded ({why}) — nothing was spawned.",
            parsed.name
        )?;
        return Ok(EXIT_FAILED);
    }

    // New window per spawned agent: the main window keeps the lead layout
    // untouched and N parallel workers stay usable.
    let Some(pane) = transport::new_window(&facts.server, &facts.session, &facts.work_dir) else {
        let _ = crate::identity::remove_seat_slot(dir, &parsed.name);
        writeln!(
            err,
            "Error: could not create a pane for '{}' — seat released.",
            parsed.name
        )?;
        return Ok(EXIT_FAILED);
    };
    stamp_pane(&facts.server, &pane, &parsed.name, &slot, &parsed.profile);
    crate::session_launch::name_agent_window(&facts.server, &pane, &parsed.name);
    // A spawn makes a NEW window, and the pane border, menu and popup styles
    // live in the window table — so the window is stamped here rather than
    // waiting a watchdog cycle to be dressed.
    if let Some(look) = crate::session_launch::look_of(&facts.server, &facts.session) {
        crate::session_launch::stamp_window(&facts.server, &pane, &look);
    }
    facts.regenerate_manifest(dir);
    // Let the new pane's shell finish drawing its prompt before anything is
    // pasted into it.
    std::thread::sleep(SHELL_SETTLE);

    // Everything a launch command is made of — the context injection, the
    // session id, the create-vs-resume decision — is composed by `_run` IN the
    // pane, from this session's own state.
    if let Err(why) = crate::run::clear_slot(dir, &slot) {
        rollback(dir, &facts, &slot, &pane, &parsed.name, err)?;
        writeln!(
            err,
            "Error: '{}' could not claim slot {slot} ({why}) — spawn rolled back.",
            parsed.name
        )?;
        return Ok(EXIT_FAILED);
    }
    // A slot being claimed again takes nothing from the seat that had it: a
    // retry record left by a PREVIOUS occupant would otherwise outlive it and
    // be weighed against this new seat's incarnation.
    //
    // AFTER the claim succeeded, never before: the arm above ROLLS BACK, and a
    // clear that failed leaves the previous occupant still holding the slot —
    // so removing its record first would destroy a brief that is still owed to
    // a seat that still exists.
    crate::brief_retry::remove(dir, &slot);
    // Codex's workspace context rides `developer_instructions`; what is left for
    // the launch command is the seat's registration handshake, which travels
    // with the brief as the inline first message `_run` composes. That combined
    // turn is the BRIEF — the handshake rides under the brief marker, so the
    // task contract keeps the first line's authority (rule 8b).
    // The actor every brief marker names: the verified caller, or `unverified`
    // when no pane identity could be bound — never bare, because bare is the
    // human's signature.
    let actor = if caller.is_empty() {
        deliver::UNVERIFIED
    } else {
        caller
    };
    let initial = launch::initial_turn_with_brief(tool, dir, &slot, actor, &brief);
    // Publish the recoverable text BEFORE anything can paste it.
    if !initial.is_empty() {
        let stored = deliver::store_body(dir, &format!("spawn-{slot}"), SPAWN_ACTION, &initial)
            .and_then(|_| crate::run::publish_prompt(dir, &slot, &initial));
        if let Err(why) = stored {
            rollback(dir, &facts, &slot, &pane, &parsed.name, err)?;
            writeln!(
                err,
                "Error: '{}' task body could not be stored ({why}) — spawn rolled back.",
                parsed.name
            )?;
            return Ok(EXIT_FAILED);
        }
    }
    // A spawn brief rides the LAUNCH TURN on the UserTurn channel — the same
    // single positional that already carries the context — so no paste can
    // land mid-turn. Gate on the RAW prompt: `brief` is never empty, it
    // carries a default text when no prompt was given, and a no-prompt spawn
    // keeps today's paste path byte-identical.
    let brief_rides_argv = if !parsed.prompt.is_empty()
        && matches!(
            tool.adapter().launch.context,
            crate::tool::ContextChannel::UserTurn { .. }
        ) {
        // Render the SAME context `_run` will compose with: the seat rows are
        // claimed, so both renders are byte-identical short of a concurrent
        // config edit, whose drift the bound's floor absorbs.
        let local = crate::config::local_overlay(dir, &facts.origin);
        let config_files =
            crate::config::ctx_config_files(facts.global.as_deref(), local.as_deref());
        let ctx = crate::render::context_document(
            dir,
            &facts.session,
            &facts.work_dir,
            &slot,
            &config_files,
        );
        // Read back what was recorded, exactly as `_run` reads it — a failed
        // record means an empty marker in both processes, not a skew.
        let launch_id = crate::meta::read_bytes(dir)
            .ok()
            .and_then(|bytes| {
                crate::meta::sole_value(&bytes, &format!("launch_id.{slot}"))
                    .map(|value| String::from_utf8_lossy(value).into_owned())
            })
            .unwrap_or_default();
        let framed = crate::provenance::first_line(&crate::provenance::brief(actor), &brief);
        let full = launch::user_turn_text(
            &ctx,
            tool.adapter().launch_marker,
            &launch_id,
            &slot,
            Some(&framed),
        );
        if launch::folded_turn_fits(&full) {
            let stored =
                deliver::store_body(dir, &format!("spawn-{slot}"), SPAWN_ACTION, &framed)
                    .and_then(|_| crate::run::publish_prompt(dir, &slot, &framed));
            if let Err(why) = stored {
                rollback(dir, &facts, &slot, &pane, &parsed.name, err)?;
                writeln!(
                    err,
                    "Error: '{}' task body could not be stored ({why}) — spawn rolled back.",
                    parsed.name
                )?;
                return Ok(EXIT_FAILED);
            }
            true
        } else {
            writeln!(
                err,
                "ae: spawn '{}': folded launch turn {} bytes exceeds {} — briefing by paste instead.",
                parsed.name,
                launch::quoted_turn_len(&full),
                launch::MAX_FOLDED_TURN_BYTES
            )?;
            false
        }
    } else {
        false
    };
    // RESOLVED, never raw.
    let Some(core) = crate::shape::resolved_exe() else {
        rollback(dir, &facts, &slot, &pane, &parsed.name, err)?;
        writeln!(
            err,
            "Error: the core could not name its own binary — spawn rolled back."
        )?;
        return Ok(EXIT_FAILED);
    };
    // The pane command is pasted into a SHELL, which is the one delivery in ae
    // whose reader is meant to be a shell.
    let _ = deliver::submit_shell_text(
        &facts.server,
        &pane,
        &crate::run::pane_command_with_snapshot(&core, dir, &slot, command.as_str()),
    );
    wait_for_agent_start(&facts.server, &pane, tool);
    // Preserve the post-exec lifecycle stamp separately from the pre-exec
    // capture floor, then start the detached capture.
    if tool.adapter().capture.is_needed() {
        let _ = meta::rewrite(
            dir,
            &format!("launch_time.{slot}"),
            Some(&crate::time::Timestamp::now().epoch().to_string()),
        );
        // The post-launch id capture, detached: it polls and scans for minutes,
        // so it cannot run inside the process the spawning agent waits on.
        capture::start(
            dir,
            &[capture::Target {
                slot: slot.clone(),
                tool,
                pane: pane.clone(),
            }],
        );
    }

    // BRIEF-DELIVERED, tracked apart from pane-created. The codex turn and
    // the folded turn are mutually exclusive: `initial` is non-empty only for
    // `RegisterSessionId`, which only codex declares, and codex is not a
    // UserTurn tool.
    let failure = if initial.is_empty() && !brief_rides_argv {
        deliver_brief(
            dir,
            &facts,
            &pane,
            &slot,
            &parsed.name,
            &brief,
            actor,
            tool,
            err,
        )?
    } else {
        None
    };
    if let Some(refusal) = failure {
        report_undelivered(
            &Undelivered {
                dir,
                name: &parsed.name,
                slot: &slot,
                pane: &pane,
                brief: &brief,
                actor,
                now,
            },
            &refusal,
            err,
        )?;
        record_spawn(dir, now, caller, &parsed.name, &parsed.prompt);
        let _ = store::open(dir).append_event(&tracked::event_line(&EventFields {
            ts: now,
            actor: actor_of(caller),
            action: SPAWN_FAILED_ACTION,
            target: &parsed.name,
            reference: "",
            actor_slot: "",
            actor_session: "",
            target_slot: "",
            target_session: "",
            target_server: "",
            target_pane: "",
            target_session_uuid: "",
            caller_server: "",
            caller_pane: "",
            caller_session_uuid: "",
            identity_gap: "",
            summary: &format!("brief not delivered: {}", refusal.reason),
            body_file: "",
        }));
        return Ok(EXIT_FAILED);
    }
    writeln!(out, "Spawned {} in pane {pane}", parsed.name)?;
    record_spawn(dir, now, caller, &parsed.name, &parsed.prompt);
    Ok(0)
}

/// Record the seat a live pane opened, whether its first brief landed or not.
fn record_spawn(dir: &Path, now: Timestamp, caller: &str, name: &str, prompt: &str) {
    let _ = store::open(dir).append_event(&tracked::event_line(&EventFields {
        ts: now,
        actor: actor_of(caller),
        action: SPAWN_ACTION,
        target: name,
        reference: "",
        actor_slot: "",
        actor_session: "",
        target_slot: "",
        target_session: "",
        target_server: "",
        target_pane: "",
        target_session_uuid: "",
        caller_server: "",
        caller_pane: "",
        caller_session_uuid: "",
        identity_gap: "",
        summary: prompt,
        body_file: "",
    }));
}

/// The event's actor: the caller's stamp, or the human.
fn actor_of(caller: &str) -> &str {
    if caller.is_empty() { "human" } else { caller }
}

/// Label the pane. Its new window is named separately, so this function stays
/// correct if pane stamping is ever reused for a split.
fn stamp_pane(server: &ServerId, pane: &str, name: &str, slot: &str, profile: &str) {
    let _ = transport::set_pane_title(server, pane, &format!("ae:{name}"));
    // The IDENTITY, verbatim; the label beside it is the same name as DRAWN,
    // with everything a drawer would read as a style taken out.
    let _ = transport::publish_option(
        server,
        crate::tmux::OptionScope::Pane,
        pane,
        "@ae_agent",
        name,
    );
    let _ = transport::publish_option(
        server,
        crate::tmux::OptionScope::Pane,
        pane,
        crate::theme::AGENT_LABEL_OPTION,
        &crate::theme::agent_label(name),
    );
    let _ = transport::publish_option(
        server,
        crate::tmux::OptionScope::Pane,
        pane,
        "@ae_slot",
        slot,
    );
    let _ = transport::publish_option(
        server,
        crate::tmux::OptionScope::Pane,
        pane,
        crate::theme::PROFILE_OPTION,
        // SANITISED at the sink: a profile name comes back off a hand-editable
        // meta, and the drawer reads `#[…]` out of an option value, so a
        // profile carrying one would restyle the pane border it names.
        &crate::theme::bar_text(profile, crate::theme::PROFILE_WIDTH),
    );
}

/// Wait, briefly, for the tool's process to replace the pane's shell — the
/// frozen `wait_for_agent_start`.
fn wait_for_agent_start(server: &ServerId, pane: &str, tool: ToolKind) {
    if !tool.adapter().input.wait_for_process {
        return;
    }
    for _ in 0..START_POLLS {
        let current = transport::observe_pane_probe(server, pane)
            .map(|probe| probe.command)
            .unwrap_or_default();
        // opencode's process reports as `opencode.exe` (its bun-built
        // launcher), so an exact comparison never matched and this wait
        // silently degraded to the is-it-still-a-shell check.
        if current.strip_suffix(".exe").unwrap_or(&current) == tool.as_str() {
            return;
        }
        if !crate::watchdog::command_is_shell(&current) {
            return;
        }
        std::thread::sleep(START_POLL);
    }
}

/// A brief that did not land, and the recovery its failure kind has.
struct BriefRefusal {
    reason: String,
    recovery: BriefRecovery,
}

/// What a failed brief's recovery is, per failure kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BriefRecovery {
    /// The seat looks like a live agent that did not take the brief: re-send.
    Resend,
    /// The agent is PROVEN gone (its pane is a shell): a `send` would be
    /// refused by the dead-pane guard; retire and spawn again.
    Retire,
    /// The pane was NOT proven live or dead. Ordinary delivery fails OPEN on
    /// an unproven pane, so a send could execute the brief in a shell: do NOT
    /// send, inspect the seat first.
    Inspect,
}

/// The brief refusal a dead seat gets, quoted by the reason and the recovery.
const BRIEF_REFUSED_DEAD: &str = "brief REFUSED — the pane is a shell, not a running agent (it died before or during delivery); NOTHING was pasted";

/// The recovery for a failure that PROVES NOTHING about the pane: re-observe
/// it through the one liveness owner and choose from that, so a send is never
/// advised on a guess. Ordinary delivery fails OPEN on an unproven pane, so a
/// `send` following a wrong guess can execute the brief in a shell.
fn unproved_recovery(facts: &Facts, dir: &Path, pane: &str, slot: &str) -> BriefRecovery {
    match deliver::observe_pane_liveness(&facts.server, dir, pane, slot) {
        deliver::PaneLiveness::Alive => BriefRecovery::Resend,
        deliver::PaneLiveness::Dead => BriefRecovery::Retire,
        deliver::PaneLiveness::Unproven => BriefRecovery::Inspect,
    }
}

/// Deliver the brief by paste: every claude/opencode spawn, and the UserTurn
/// spawns the fold does not take (no prompt, or past the bound).
#[allow(
    clippy::too_many_arguments,
    reason = "one call site; every argument is a fact about it"
)]
fn deliver_brief(
    dir: &Path,
    facts: &Facts,
    pane: &str,
    slot: &str,
    name: &str,
    brief: &str,
    actor: &str,
    kind: ToolKind,
    err: &mut impl Write,
) -> io::Result<Option<BriefRefusal>> {
    // The tool is the CONFIGURED one, not the pane's live command: a wrapper, an
    // interpreter or a `.exe` launcher makes the live command say something
    // else while the box on screen is still the tool's.
    let model = kind.adapter().input.model;
    let composed = kind.adapter().input.composed;
    // DO NOT paste into a state we could not confirm idle.
    if !deliver::wait_input_ready(&facts.server, pane, model, composed, BRIEF_READY_POLLS) {
        return Ok(Some(BriefRefusal {
            reason: "input never reached a confirmed-idle state (busy, modal, or unreadable)"
                .to_owned(),
            // Readiness never ran, or ran and never settled: it proves nothing
            // about liveness, so the recovery comes from a fresh observation.
            recovery: unproved_recovery(facts, dir, pane, slot),
        }));
    }
    let request = deliver::Request {
        dir,
        server: &facts.server,
        pane,
        logged_target: name,
        target_session: &facts.session,
        pane_slot: slot,
        own_session: &facts.session,
        action: SPAWN_ACTION,
        reference: &format!("spawn-{slot}"),
        actor,
        body: brief,
        shape: Shape::Launch,
        defer: deliver::DEFAULT_DEFER,
        composed,
    };
    let outcome = deliver::deliver(&request, err)?;
    let body_file = match &outcome {
        Ok(delivered) => delivered.body_file.clone(),
        Err(failure) => failure.body_file().to_owned(),
    };
    if let Err(failure) = outcome {
        let (reason, recovery) = match &failure {
            // A pre-paste refusal did not stage anything, so it must not
            // borrow the submit uncertainty wording.
            deliver::Failure::NotComposed { .. } => (
                format!(
                    "brief REFUSED — the pane was not composed when delivery reached it; NOTHING was pasted. Body preserved at {body_file}"
                ),
                BriefRecovery::Resend,
            ),
            deliver::Failure::Unproven { .. } => (
                format!(
                    "brief REFUSED — the pane could not be proven a live agent at delivery time; NOTHING was pasted. Body preserved at {body_file}"
                ),
                BriefRecovery::Inspect,
            ),
            deliver::Failure::DeadPane => (BRIEF_REFUSED_DEAD.to_owned(), BriefRecovery::Retire),
            // The paste was ACCEPTED by the box; only its Enter went
            // unconfirmed. That is positive evidence the pane was live.
            deliver::Failure::Unconfirmed { notice: false, .. } => (
                format!(
                    "brief submit UNCONFIRMED ({failure:?}) — body preserved at {body_file}; it may be staged unsent"
                ),
                BriefRecovery::Resend,
            ),
            // A held lock, a failed paste, a failed store, a failed notice
            // proof: the failure alone does not establish the pane's CURRENT
            // liveness, so observe it fresh rather than guessing. Some of
            // these carry a published body and some do not, so the reason
            // claims none.
            _ => (
                format!(
                    "brief delivery FAILED ({failure:?}) — the failure does not establish the pane's current liveness"
                ),
                unproved_recovery(facts, dir, pane, slot),
            ),
        };
        return Ok(Some(BriefRefusal { reason, recovery }));
    }
    // A booting TUI can swallow the post-paste Enter, leaving the brief staged
    // in the input box.
    std::thread::sleep(LINGER_SETTLE);
    let head: String = brief.chars().take(LINGER_PREFIX).collect();
    if let Some(screen) = transport::capture_pane(&facts.server, pane) {
        let tail: Vec<&str> = screen.lines().rev().take(6).collect();
        if tail.iter().any(|line| line.contains(&head)) {
            let _ = transport::send_key(&facts.server, pane, crate::tmux::Key::Enter);
        }
    }
    Ok(None)
}

/// One undelivered brief, and everything its record would need.
struct Undelivered<'a> {
    dir: &'a Path,
    name: &'a str,
    slot: &'a str,
    pane: &'a str,
    brief: &'a str,
    actor: &'a str,
    now: Timestamp,
}

/// Record the brief for a later retry, when a retry could ever work.
///
/// A PROVEN-DEAD seat gets none: its pane is a shell, so the retry's liveness
/// gate could never pass, and a record there would buy nothing but a give-up
/// half an hour later. The two live-ish recoveries get one, because the gate
/// decides fail-closed at delivery time and an unproven pane may well be fine.
///
/// Returns the refusal when a record was NOT written, so the advice can say the
/// true thing in every case rather than promising a retry nobody will make.
fn record_for_retry(undelivered: &Undelivered<'_>, recovery: BriefRecovery) -> Option<String> {
    if recovery == BriefRecovery::Retire {
        return Some("the seat is gone".to_owned());
    }
    let launch_id = crate::meta::read_bytes(undelivered.dir)
        .ok()
        .and_then(|bytes| {
            crate::meta::sole_value(&bytes, &format!("launch_id.{}", undelivered.slot))
                .map(|value| String::from_utf8_lossy(value).into_owned())
        })
        .filter(|value| !value.is_empty());
    let Some(launch_id) = launch_id else {
        return Some("the seat has no recorded launch token".to_owned());
    };
    let record = crate::brief_retry::Record {
        slot: undelivered.slot.to_owned(),
        reference: format!("spawn-{}", undelivered.slot),
        pane: undelivered.pane.to_owned(),
        launch_id,
        actor: undelivered.actor.to_owned(),
        attempts: 0,
        created: undelivered.now.epoch(),
        phase: crate::brief_retry::Phase::Armed,
        body: undelivered.brief.to_owned(),
    };
    crate::brief_retry::publish(undelivered.dir, &record).err()
}

/// Say what happened, where the brief is, and who will hand it over.
fn report_undelivered(
    undelivered: &Undelivered<'_>,
    refusal: &BriefRefusal,
    err: &mut impl Write,
) -> io::Result<()> {
    let name = undelivered.name;
    let dir = undelivered.dir;
    let file = dir.join(format!("undelivered.{name}.txt"));
    let preserved = write_private(&file, undelivered.brief).is_ok();
    writeln!(
        err,
        "ae: SPAWN INCOMPLETE — {name} exists in pane {}, brief NOT delivered",
        undelivered.pane
    )?;
    writeln!(err, "ae: reason: {}", refusal.reason)?;
    // Every recovery must be able to find the brief: name the fallback file
    // this report just published, or say plainly that even that failed.
    if preserved {
        writeln!(err, "ae: the brief is preserved at {}", file.display())?;
    } else {
        writeln!(
            err,
            "ae: WARNING: the brief could not be preserved to disk at {}",
            file.display()
        )?;
    }
    let refused_record = record_for_retry(undelivered, refusal.recovery);
    match refusal.recovery {
        // The pane is a SHELL: a `send` would be refused by the dead-pane
        // guard, so the only recovery is retiring the dead seat and spawning
        // again. The live-seat wording must NOT appear here.
        BriefRecovery::Retire => {
            writeln!(
                err,
                "ae: the agent is GONE (its pane is a shell) — a send would be refused; retire the seat and spawn again:"
            )?;
            writeln!(err, "ae:   {}/retire {name}", dir.display())?;
            return Ok(());
        }
        // The pane was not proven either way. Ordinary delivery FAILS OPEN on
        // an unproven pane, so a send could execute the brief in a shell: the
        // recovery must advise inspection, never a send.
        BriefRecovery::Inspect => {
            writeln!(
                err,
                "ae: the pane was NOT proven live or dead (a shell may hold a stale frame) — do NOT send; inspect the seat, then retire or re-spawn:"
            )?;
            writeln!(err, "ae:   {}/peek {name}", dir.display())?;
        }
        BriefRecovery::Resend => {}
    }
    // THE RETRY NOTICE, and it must not promise what is not on record. When a
    // record was written, a hand re-send would deliver the brief TWICE, so the
    // advice is the opposite of what it used to be. When the record was
    // refused, nothing will retry it and the old advice is exactly right.
    if let Some(why) = refused_record {
        writeln!(
            err,
            "ae: this brief will NOT be retried automatically ({why})."
        )?;
        if refusal.recovery == BriefRecovery::Resend {
            writeln!(
                err,
                "ae: do NOT respawn (the pane is live) — send to the existing agent:"
            )?;
            if preserved {
                writeln!(
                    err,
                    "ae:   {}/send {name} \"$(cat {})\"",
                    dir.display(),
                    file.display()
                )?;
            } else {
                writeln!(
                    err,
                    "ae:   {}/send {name} '<re-send your brief>'",
                    dir.display()
                )?;
            }
        }
        return Ok(());
    }
    writeln!(
        err,
        "ae: ae has RECORDED this brief and will retry it itself — at most twice, within 30 minutes, and only into this same seat."
    )?;
    writeln!(
        err,
        "ae: do NOT re-send it by hand: the retry and your send would both land, and the agent would get the brief twice."
    )?;
    writeln!(
        err,
        "ae: the retry is the session's watchdog, so if none is running for this session nothing will retry it."
    )?;
    writeln!(
        err,
        "ae: a brief-gave-up event is the signal to hand it over yourself; retiring the seat cancels the retry."
    )
}

/// Write `text` at 0600 — the same material as the pane content.
fn write_private(path: &Path, text: &str) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut file = std::fs::File::create(path)?;
    file.write_all(text.as_bytes())?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

/// Undo a spawn whose agent never launched.
fn rollback(
    dir: &Path,
    facts: &Facts,
    slot: &str,
    pane: &str,
    name: &str,
    err: &mut impl Write,
) -> io::Result<()> {
    if crate::identity::remove_seat_slot(dir, name).is_err() {
        writeln!(
            err,
            "ae: spawn rollback could not remove the seat of '{name}' ({slot}) — remove it with 'retire'."
        )?;
    }
    drop_launch_artifacts(dir, slot);
    let _ = watchdog_glue::kill_owned_pane(&facts.server, pane, &facts.session, Some(name), err);
    facts.regenerate_manifest(dir);
    Ok(())
}

/// The slot's start marker and recorded first message — dead weight once the
/// pane is gone, and a hazard once the slot number is handed to someone else.
fn drop_launch_artifacts(dir: &Path, slot: &str) {
    // Retiring a seat CANCELS its undelivered brief: there is no longer anyone
    // for it to be delivered to, and a record that outlived its seat would be
    // weighed against whoever takes the slot next.
    crate::brief_retry::remove(dir, slot);
    let _ = crate::run::clear_slot(dir, slot);
}

// ---- retire ---------------------------------------------------------------

/// `_retire <meta-dir> <name|%pane>`.
///
/// # Errors
///
/// Only a failure to write `out` or `err`.
pub fn run_retire(
    dir: &Path,
    tail: &[String],
    caller: &str,
    now: Timestamp,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let [target] = tail else {
        writeln!(err, "{RETIRE_USAGE}")?;
        writeln!(err, "  Examples: retire researcher")?;
        writeln!(err, "           retire %5")?;
        return Ok(EXIT_USAGE);
    };
    let facts = match facts(dir) {
        Ok(facts) => facts,
        Err(why) => {
            writeln!(err, "Error: {why}")?;
            return Ok(EXIT_FAILED);
        }
    };
    let panes = transport::observe_agents(&facts.server, &facts.session).unwrap_or_default();
    let (resolved, agent) = if let Some(pane) = target.strip_prefix('%') {
        let id = format!("%{pane}");
        let Some(found) = panes.iter().find(|row| row.pane == id) else {
            writeln!(
                err,
                "Error: pane '{target}' not found in session '{}'",
                facts.session
            )?;
            return Ok(EXIT_FAILED);
        };
        (found.pane.clone(), found.agent.clone())
    } else {
        let found = panes.iter().find(|row| row.agent == *target);
        (
            found.map(|row| row.pane.clone()).unwrap_or_default(),
            target.clone(),
        )
    };
    if !resolved.is_empty() && resolved == facts.main_pane {
        writeln!(
            err,
            "Error: cannot retire the main agent — use 'ae end' instead"
        )?;
        return Ok(EXIT_FAILED);
    }
    let retired_identity = crate::session::read_meta(dir).ok().and_then(|meta| {
        meta.roster()
            .iter()
            .find(|entry| entry.name == agent)
            .cloned()
    });
    let slot = match crate::identity::remove_seat_slot(dir, &agent) {
        Ok(slot) => slot,
        Err(why) => {
            writeln!(err, "Error: {why}")?;
            return Ok(EXIT_FAILED);
        }
    };
    if !resolved.is_empty() {
        let _ = watchdog_glue::kill_owned_pane(
            &facts.server,
            &resolved,
            &facts.session,
            Some(&agent),
            err,
        );
    }
    drop_launch_artifacts(dir, &slot);
    // No layout rebalance: the worker lived in its own window, so killing the
    // pane closed that window and the main window's layout was never touched.
    facts.regenerate_manifest(dir);
    writeln!(out, "Retired {agent} (pane {resolved})")?;
    let reference = retired_identity
        .as_ref()
        .and_then(|entry| entry.harness_session.as_deref())
        .filter(|id| crate::archive::canonical_uuid(id) == *id)
        .unwrap_or("");
    let summary = retired_identity.as_ref().map_or_else(String::new, |entry| {
        let config_home = match &entry.config_home {
            crate::meta::RecordedConfigHome::Invalid => "invalid".to_owned(),
            value => value.record_value().unwrap_or_default(),
        };
        let config_home_base = match &entry.config_home_base {
            crate::meta::RecordedConfigHomeBase::Invalid => "invalid".to_owned(),
            value => value.record_value().unwrap_or_default(),
        };
        format!(
            "tool={} profile={} config_home={} config_home_base={}",
            entry.binary.as_deref().unwrap_or(""),
            entry.profile.as_deref().unwrap_or(""),
            config_home,
            config_home_base,
        )
    });
    let _ = store::open(dir).append_event(&tracked::event_line(&EventFields::new(
        now,
        actor_of(caller),
        RETIRE_ACTION,
        &agent,
        reference,
        "",
        "",
        &slot,
        "",
        &summary,
        "",
    )));
    Ok(0)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        clippy::unwrap_used,
        reason = "fixtures build and inspect real directories; the capability boundary is about \
                  what PRODUCT code may reach, which is why the inventory counts product lines"
    )]

    use super::{BriefRecovery, Undelivered, drop_launch_artifacts, record_for_retry};
    use crate::time::Timestamp;

    /// A scratch session dir whose meta carries `token` as the slot's launch
    /// id, or no token row at all when it is empty.
    fn seat(tag: &str, token: &str) -> std::path::PathBuf {
        let dir =
            std::path::PathBuf::from(format!("/tmp/ae-spawnrec.{}.{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a scratch dir");
        let meta = match token {
            "" => "mode=local\n".to_owned(),
            token => format!("launch_id.spawned.1={token}\n"),
        };
        assert!(std::fs::write(dir.join("meta"), meta).is_ok(), "a meta");
        dir
    }

    fn undelivered<'a>(dir: &'a std::path::Path, actor: &'a str) -> Undelivered<'a> {
        Undelivered {
            dir,
            name: "scribe",
            slot: "spawned.1",
            pane: "%105",
            brief: "build the thing",
            actor,
            now: Timestamp::from_epoch(1_789_100_000),
        }
    }

    /// A record is written only where a retry could ever work, and the reason
    /// is RETURNED whenever one was not — so the advice printed beside it says
    /// the true thing instead of promising a retry nobody will make.
    #[test]
    fn a_record_is_written_only_when_a_retry_could_work_and_says_so_when_it_is_not() {
        let refuses = |tag: &str, token: &str, recovery, reason: &str| {
            let dir = seat(tag, token);
            assert_eq!(
                record_for_retry(&undelivered(&dir, "lead"), recovery),
                Some(reason.to_owned())
            );
            assert!(crate::brief_retry::read(&dir, "spawned.1").is_none());
            let _ = std::fs::remove_dir_all(&dir);
        };
        // A PROVEN-DEAD seat: its pane is a shell, so no gate could ever pass.
        refuses("gone", "tok-1", BriefRecovery::Retire, "the seat is gone");
        // NO LAUNCH TOKEN is no incarnation key, and a record without one could
        // be weighed against whoever holds the slot later.
        let tokenless = "the seat has no recorded launch token";
        refuses("tokenless", "", BriefRecovery::Resend, tokenless);

        // A LIVE-ISH recovery gets one, and it reads back — for an agent actor
        // and for the unverified spelling alike, since both reach a marker.
        for actor in ["lead", crate::deliver::UNVERIFIED] {
            let dir = seat(actor, "tok-1");
            let refusal = record_for_retry(&undelivered(&dir, actor), BriefRecovery::Resend);
            assert_eq!(refusal, None, "a record should have been published");
            let record = crate::brief_retry::read(&dir, "spawned.1")
                .expect("a record")
                .expect("a readable record");
            assert_eq!(record.actor, actor);
            assert_eq!(record.launch_id, "tok-1");
            assert_eq!(record.attempts, 0);
            assert_eq!(record.reference, "spawn-spawned.1");
            // THE MARKER IS NOT STORED: the body is the brief alone, and
            // `deliver` stamps `brief(<actor>)` from the actor above.
            assert_eq!(record.body, "build the thing");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// Retiring a seat CANCELS its undelivered brief: there is nobody left for
    /// it to be delivered to, and a record that outlived its seat would be
    /// weighed against whoever takes the slot next.
    #[test]
    fn retiring_a_seat_cancels_the_brief_it_never_received() {
        let dir = seat("retire", "tok-1");
        assert_eq!(
            record_for_retry(&undelivered(&dir, "lead"), BriefRecovery::Resend),
            None
        );
        assert!(crate::brief_retry::read(&dir, "spawned.1").is_some());

        drop_launch_artifacts(&dir, "spawned.1");
        assert!(
            crate::brief_retry::read(&dir, "spawned.1").is_none(),
            "a retired seat's record must be gone"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
