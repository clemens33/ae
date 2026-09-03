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
//! 4. the launch script is published and pasted into the pane's shell;
//! 5. the BRIEF is delivered only after the TUI proves it will accept input.
//!
//! Any failure after step 1 ROLLS BACK: the seat goes, the launch artifacts go,
//! and the pane is killed through the ownership guard. A spawn that returned
//! without rolling back would leave a phantom — a roster entry with an empty
//! pane, visible to `ae list`, addressable by every helper, retirable only by
//! hand.
//!
//! # Pane-created and brief-delivered are different truths
//!
//! Only the second means the worker was assigned its task. Conflating them is
//! how a delivery failure became a success at the caller: the failure printed
//! to stderr and control ran on to a task-bearing `spawn` event and rc=0, so
//! supervision saw an assigned worker whose brief was absent. stderr is for
//! humans; the exit code and the event log are the machine-visible contract,
//! and they must agree with reality.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::deliver::region::Tool;
use crate::deliver::{self, Shape};
use crate::inventory::ServerId;
use crate::launch;
use crate::launch_cmd::ToolKind;
use crate::meta::{self, Meta, ServerSelector};
use crate::session_launch::capture;
use crate::state::{self, EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;
use crate::tracked::{self, EventFields};
use crate::transport;
use crate::watchdog_glue;

/// The frozen usage line.
pub const SPAWN_USAGE: &str = "Usage: spawn <name> --using <profile> [--] [prompt]";

/// The frozen `retire` usage.
pub const RETIRE_USAGE: &str = "Usage: retire <agent-name|pane-id>";

/// How long the frozen path waits for a new pane's shell before pasting.
const SHELL_SETTLE: Duration = Duration::from_millis(300);

/// How many polls the brief's readiness wait takes — the frozen `30`, at half
/// a second each.
const BRIEF_READY_POLLS: u32 = 30;

/// How many polls the launch's process wait takes — the frozen `10`.
const START_POLLS: u32 = 10;

/// The pause between those polls — the frozen `sleep 0.1`.
const START_POLL: Duration = Duration::from_millis(100);

/// How long to let a booting TUI swallow the Enter before looking for staged
/// text — the frozen `sleep 1.5`.
const LINGER_SETTLE: Duration = Duration::from_millis(1500);

/// How much of the brief the lingering check looks for on screen — the frozen
/// `${prompt:0:40}`.
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
    /// The task, as typed. Empty when none was given.
    pub prompt: String,
}

/// Parse `<name> --using <profile> [--] [prompt…]`.
///
/// `--` ends the options so a prompt may start with a dash; anything else that
/// is not a recognised option ends them too, which is what lets an unquoted
/// prompt follow the profile.
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
///
/// The config files are derived the way the frozen helper wires them: the
/// session's own `config=` row, plus the origin's `.ae/config` when that file
/// exists. The core does not read the environment, so this is the only source.
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
    ///
    /// Both are named unconditionally: the render's own reader SKIPS a file it
    /// cannot open, which is the frozen `[[ -f … ]]` gate's effect without a
    /// second existence probe of the world.
    fn config_files(&self) -> Vec<PathBuf> {
        self.global
            .iter()
            .chain(self.local.iter())
            .cloned()
            .collect()
    }

    /// The layered `[profiles]`, with an ABSENT file read as absent.
    ///
    /// The frozen helper gates each file on `-f` before naming it; this reads
    /// it and drops the one that could not be opened, which reaches the same
    /// answer without a second door onto the filesystem. A file that exists but
    /// is malformed still refuses — that is a config error, not an absence.
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
    // THE PEER BOUNDARY. A spawn name arrives from another agent, and since #59
    // it is interpolated into the new agent's own system prompt — so it is
    // allowlisted HERE, before any effect. The seat write re-checks it.
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
    let Some(command) = cfg.profile(&parsed.profile).map(ToOwned::to_owned) else {
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
    // The brief the new agent is handed. A caller we could identify gets a
    // reply-back instruction; an unidentifiable one does not, because a `send`
    // to nobody is worse than no instruction at all.
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
    // one-simple-command lexer for the initial roster only; a profile selected at
    // spawn used to reach `bash -lc` unvalidated, so `bad = "touch m; tail -f
    // /dev/null"` executed the semicolon command and then reported the spawn
    // incomplete with the seat left in meta (colead gate b5d60fec, repro). Tool
    // and binary now come from the one validated parse.
    let lexed = match crate::launch_cmd::lex_simple_command(&command) {
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
    let binary = crate::launch_cmd::split_binary(&command)
        .map(|split| split.binary_name().to_owned())
        .unwrap_or_default();
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
    // The launch token is a CAPTURE fact of the tools with no launch-time id
    // flag, not a roster row — but it is written before the pane exists, so the
    // injected context can name it.
    let launch_id = if launch::supports_launch_id(tool) {
        let token = launch::generate_uuid();
        if meta::rewrite(dir, &format!("launch_id.{slot}"), Some(&token)).is_err() {
            writeln!(
                err,
                "ae: could not record the launch token of '{}'.",
                parsed.name
            )?;
        }
        token
    } else {
        String::new()
    };

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
    stamp_pane(&facts.server, &pane, &parsed.name, &slot);
    facts.regenerate_manifest(dir);
    // Let the new pane's shell finish drawing its prompt before anything is
    // pasted into it.
    std::thread::sleep(SHELL_SETTLE);

    let ctx = crate::render::context_document(
        dir,
        &facts.session,
        &facts.work_dir,
        &slot,
        &facts.config_files(),
    );
    let pre = launch::inject_session_id(&command, &session_id);
    let injected = launch::inject_ae_context(&pre, dir, &slot, &ctx, &launch_id);
    if let Some(warning) = &injected.warning {
        writeln!(err, "{warning}")?;
    }
    // For a tool with no system-prompt channel the context AND the brief travel
    // as the launch command's inline first message.
    let inline = launch::initial_prompt_for(tool);
    let initial = if inline.is_empty() {
        String::new()
    } else {
        format!("{inline} --- {brief}")
    };
    // Publish the recoverable text BEFORE anything can paste it. A storage
    // failure is terminal for the spawn.
    if !initial.is_empty()
        && let Err(why) = deliver::store_body(dir, &format!("spawn-{slot}"), SPAWN_ACTION, &initial)
    {
        rollback(dir, &facts, &slot, &pane, &parsed.name, err)?;
        writeln!(
            err,
            "Error: '{}' task body could not be stored ({why}) — spawn rolled back.",
            parsed.name
        )?;
        return Ok(EXIT_FAILED);
    }
    let launch_cmd = launch::build_launch_command(&injected.cmd, &initial, &session_id, &pre);
    let script = match launch::write_launch_script(dir, &slot, &launch_cmd, &session_id, &pre) {
        Ok(script) => script,
        Err(why) => {
            rollback(dir, &facts, &slot, &pane, &parsed.name, err)?;
            writeln!(
                err,
                "ae: could not write the launch script for '{slot}' ({why}) — agent not started"
            )?;
            writeln!(
                err,
                "Error: '{}' could not be launched — spawn rolled back.",
                parsed.name
            )?;
            return Ok(EXIT_FAILED);
        }
    };
    // The launch command is pasted into a SHELL, which is the one delivery in
    // ae whose reader is meant to be a shell. Fire and forget: an unconfirmed
    // submit must not abort a launch that may well have taken.
    let _ = deliver::submit_shell_text(
        &facts.server,
        &pane,
        &launch::shell_quote(&script.display().to_string()),
    );
    wait_for_agent_start(&facts.server, &pane, tool);
    // The capture tools need the launch instant to filter stale sessions, so it
    // is recorded BEFORE the capture child is started — the child reads it back
    // out of meta, and a capture with no floor would accept a conversation this
    // spawn did not create.
    if launch::supports_launch_id(tool) {
        let _ = meta::rewrite(
            dir,
            &format!("launch_time.{slot}"),
            Some(&crate::time::Timestamp::now().epoch().to_string()),
        );
        // The post-launch id capture, detached: it polls and scans for minutes,
        // so it cannot run inside the process the spawning agent waits on. The
        // frozen path forked it from bash after the core returned; the core owns
        // it now, which is also what makes it start from the facts it has just
        // written rather than from a second reader's guess at the slot.
        //
        // HERE, not after the brief. Two reasons, and both are deliberate
        // departures from the frozen "only on rc==0": the capture window is
        // finite (the tool writes its session record within seconds of
        // starting, and the poll gives up), so time spent waiting for a
        // readiness-gated paste is time taken off it; and a brief that fails to
        // deliver does NOT roll the seat back — the agent is alive and its id is
        // still worth having. A capture that outlives a seat which later goes
        // away is harmless: `set-harness-session` refuses a slot that is not in
        // the roster.
        capture::start(
            dir,
            &[capture::Target {
                slot: slot.clone(),
                tool,
                pane: pane.clone(),
            }],
        );
    }

    // BRIEF-DELIVERED, tracked apart from pane-created. A tool that carried its
    // brief inline has nothing left to paste, so delivery is settled already.
    let failure = if initial.is_empty() {
        deliver_brief(
            dir,
            &facts,
            &pane,
            &slot,
            &parsed.name,
            &brief,
            caller,
            tool,
            err,
        )?
    } else {
        None
    };
    if let Some(reason) = failure {
        report_undelivered(dir, &parsed.name, &pane, &brief, &reason, err)?;
        let _ = state::emit(
            dir,
            &tracked::event_line(&EventFields {
                ts: now,
                actor: actor_of(caller),
                action: SPAWN_FAILED_ACTION,
                target: &parsed.name,
                reference: "",
                actor_slot: "",
                actor_session: "",
                target_slot: "",
                target_session: "",
                summary: &format!("brief not delivered: {reason}"),
                body_file: "",
            }),
        );
        return Ok(EXIT_FAILED);
    }
    writeln!(out, "Spawned {} in pane {pane}", parsed.name)?;
    let _ = state::emit(
        dir,
        &tracked::event_line(&EventFields {
            ts: now,
            actor: actor_of(caller),
            action: SPAWN_ACTION,
            target: &parsed.name,
            reference: "",
            actor_slot: "",
            actor_session: "",
            target_slot: "",
            target_session: "",
            summary: &parsed.prompt,
            body_file: "",
        }),
    );
    Ok(0)
}

/// The event's actor: the caller's stamp, or the human.
fn actor_of(caller: &str) -> &str {
    if caller.is_empty() { "human" } else { caller }
}

/// Label the pane and name the worker's window.
///
/// `@ae_agent` IS the bare name under identity v2. The explicit
/// `rename-window` also disables tmux's automatic-rename, so the window keeps
/// the role name instead of following the foreground process — and the name is
/// format-escaped, because a window name is a tmux FORMAT and `#(cmd)` in one
/// runs a shell.
fn stamp_pane(server: &ServerId, pane: &str, name: &str, slot: &str) {
    let _ = transport::set_pane_title(server, pane, &format!("ae:{name}"));
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
        "@ae_slot",
        slot,
    );
    let _ = transport::rename_window(server, pane, &crate::tmux::format_literal(name));
}

/// Wait, briefly, for the tool's process to replace the pane's shell — the
/// frozen `wait_for_agent_start`.
///
/// Best effort: it proves the process is RUNNING, never that it will accept
/// input, which is why the brief has its own readiness gate. Unmodelled tools
/// are not waited for at all, exactly as the frozen `case` skips them.
fn wait_for_agent_start(server: &ServerId, pane: &str, tool: ToolKind) {
    if !matches!(
        tool,
        ToolKind::Claude | ToolKind::Codex | ToolKind::OpenCode
    ) {
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

/// Deliver the brief to a tool whose context rode a system-prompt channel.
///
/// `None` is delivered; `Some(reason)` is not, and the caller must then report
/// it rather than claim the task was assigned.
#[allow(
    clippy::too_many_arguments,
    reason = "one call site, all six are facts"
)]
fn deliver_brief(
    dir: &Path,
    facts: &Facts,
    pane: &str,
    slot: &str,
    name: &str,
    brief: &str,
    caller: &str,
    kind: ToolKind,
    err: &mut impl Write,
) -> io::Result<Option<String>> {
    // The tool is the CONFIGURED one, not the pane's live command: that is the
    // fact the frozen `_wait_input_ready` is handed, and it is right for the
    // same reason `ae_target_tool` prefers `agent_bin.<slot>` — a wrapper, an
    // interpreter or a `.exe` launcher makes the live command say something
    // else while the box on screen is still the tool's.
    let tool = match kind {
        ToolKind::Claude => Tool::Claude,
        ToolKind::Codex => Tool::Codex,
        _ => Tool::Other,
    };
    // DO NOT paste into a state we could not confirm idle. The old path polled
    // for any of `❯|bypass permissions|for shortcuts` and pasted on timeout
    // regardless — so claude's trust dialog read READY and the brief went INTO
    // THE MODAL, where Enter answers the dialog instead. A brief is re-sendable;
    // a clobbered modal is not.
    if !deliver::wait_input_ready(&facts.server, pane, tool, BRIEF_READY_POLLS) {
        return Ok(Some(
            "input never reached a confirmed-idle state (busy, modal, or unreadable)".to_owned(),
        ));
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
        actor: caller,
        body: brief,
        shape: Shape::Launch,
        defer: deliver::DEFAULT_DEFER,
    };
    let outcome = deliver::deliver(&request, err)?;
    let body_file = match &outcome {
        Ok(delivered) => delivered.body_file.clone(),
        Err(failure) => failure.body_file().to_owned(),
    };
    if let Err(failure) = outcome {
        return Ok(Some(format!(
            "brief submit UNCONFIRMED ({failure:?}) — body preserved at {body_file}; it may be staged unsent"
        )));
    }
    // A booting TUI can swallow the post-paste Enter, leaving the brief staged
    // in the input box. Nudge Enter ONCE more if it is still on screen — and
    // only here, never on a pane we deliberately did not touch.
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

/// Say what happened, where the brief is, and how to hand it over by hand.
///
/// The pane EXISTS and is in meta — say so, so nobody respawns a duplicate —
/// but never claim the task was assigned. The brief is preserved on disk rather
/// than quoted into the message: it is arbitrary multi-line text, and a
/// `<the brief>` placeholder is not a resend path anyone can run.
fn report_undelivered(
    dir: &Path,
    name: &str,
    pane: &str,
    brief: &str,
    reason: &str,
    err: &mut impl Write,
) -> io::Result<()> {
    let file = dir.join(format!("undelivered.{name}.txt"));
    let preserved = write_private(&file, brief).is_ok();
    writeln!(
        err,
        "ae: SPAWN INCOMPLETE — {name} exists in pane {pane}, brief NOT delivered"
    )?;
    writeln!(err, "ae: reason: {reason}")?;
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
        )
    } else {
        writeln!(
            err,
            "ae:   {}/send {name} '<re-send your brief>'",
            dir.display()
        )
    }
}

/// Write `text` at 0600 — the same material as the pane content.
fn write_private(path: &Path, text: &str) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut file = std::fs::File::create(path)?;
    file.write_all(text.as_bytes())?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

/// Undo a spawn whose agent never launched.
///
/// The same teardown `_retire` performs, run on the failure path. Every step is
/// tolerated — the caller's non-zero status is what matters — but the seat's
/// removal is REPORTED when it fails, because a seat nobody removed is a
/// phantom the operator has to clear with `retire`.
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

/// The slot's launch script and its re-run marker — dead weight once the pane
/// is gone.
fn drop_launch_artifacts(dir: &Path, slot: &str) {
    let safe = launch::safe_slot(slot);
    let _ = std::fs::remove_file(dir.join(format!("launch.{safe}.sh")));
    let _ = std::fs::remove_file(dir.join(format!("launch.{safe}.started")));
}

// ---- retire ---------------------------------------------------------------

/// `_retire <meta-dir> <name|%pane>`.
///
/// Resolution is EXACT: `@ae_agent` is the bare name under identity v2, and a
/// `%pane` target must be a pane of THIS session. A seat whose pane is already
/// gone still resolves by name at the seat removal below, so a spawn that died
/// between its seat and its pane can still be cleaned up.
///
/// CORE FIRST: the seat removal — which refuses a `main` or `worker.*` launch
/// seat and an unknown name — runs before anything is killed, so a refused
/// retire kills nothing.
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
    let _ = state::emit(
        dir,
        &tracked::event_line(&EventFields {
            ts: now,
            actor: actor_of(caller),
            action: RETIRE_ACTION,
            target: &agent,
            reference: "",
            actor_slot: "",
            actor_session: "",
            target_slot: "",
            target_session: "",
            summary: "",
            body_file: "",
        }),
    );
    Ok(0)
}
