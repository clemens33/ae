//! `_end`: the whole end operation.
//!
//! CAPTURE THEN DELETE, and the order is the contract. The archive is published
//! AFTER the session is verifiably stopped and after git has had its say, and
//! BEFORE any live state is removed — so a failed archive returns non-zero with
//! the whole session still on disk. ae never deletes a session it could not
//! capture.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::archive;
use crate::config::{Workspace, read_workspace};
use crate::inventory::ServerId;
use crate::meta::{self, Selector, ServerSelector};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;
use crate::tmux::StopProbe;
use crate::transport;

use super::{
    ClientPrompt, DetachedArgv, all_sessions, announce_to_clients, confirm_on_client, dir_exists,
    emit_lifecycle_event, kill_verified, live_id, lock, meta_value, name_is_usable, path_exists,
    recorded_server, server_of, sessions_dir, worktrees_dir,
};

/// The usage line.
const USAGE: &str =
    "Usage: _end [-f] [--purge-history|--keep-history] [--assume-stopped] <session-name|all>";

const END_REQUEST_ACTION: &str = "end-request";
const END_RESULT_ACTION: &str = "end-result";

/// What `ae end` will do with a session's memory, in five answers.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    /// The archive is published, keyed by the session's recorded id.
    Keep,
    /// No archive is created, and one already there is deleted.
    Purge,
    /// Nothing to archive — a leftover directory with no session memory.
    Nothing,
    /// There IS state, but it cannot be archived: end must refuse.
    Unavailable,
}

impl Action {
    fn word(&self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Purge => "purge",
            Self::Nothing => "nothing",
            Self::Unavailable => "unavailable",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        match word {
            "keep" => Some(Self::Keep),
            "purge" => Some(Self::Purge),
            "nothing" => Some(Self::Nothing),
            "unavailable" => Some(Self::Unavailable),
            _ => None,
        }
    }
}

/// One target's resolved plan.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Plan {
    action: Action,
    detail: String,
    purge: bool,
}

/// One prompt-time plan carried across the tmux and detached-process handoffs.
#[derive(Clone)]
struct ConfirmedPlan {
    name: String,
    plan: Plan,
    /// The history choice came from config, so config remains part of the
    /// changed-plan check under the lifecycle lock.
    from_default: bool,
}

#[derive(Default)]
struct ConfirmedParts {
    name: String,
    action: Option<Action>,
    detail: Option<String>,
    purge: Option<bool>,
    from_default: Option<bool>,
}

impl Plan {
    /// The target's line in the confirmation list — BOTH actions, per target,
    /// because "unless a session's own config says" is a sentence about no
    /// particular session and this prompt is the last thing between a human and
    /// a delete.
    fn line(&self, name: &str) -> String {
        let history = if self.purge {
            "conversation files DELETED"
        } else {
            "conversation files KEPT"
        };
        let detail = &self.detail;
        match self.action {
            Action::Keep => format!("  - {name}: archive -> {detail}/ · {history}"),
            Action::Purge => {
                format!("  - {name}: NO archive, and {detail}/ is DELETED if it exists · {history}")
            }
            Action::Nothing => format!("  - {name}: nothing to archive ({detail}) · {history}"),
            Action::Unavailable => {
                format!("  - {name}: CANNOT be archived ({detail}) — end will refuse it")
            }
        }
    }
}

/// What the argv said.
#[derive(Clone)]
struct Args {
    target: String,
    force: bool,
    assume_stopped: bool,
    mode: RunMode,
    pane: Option<String>,
    confirmed: Vec<ConfirmedPlan>,
    /// `Some(true)` for `--purge-history`, `Some(false)` for `--keep-history`,
    /// `None` when the caller passed neither and each session's OWN config
    /// decides.
    purge_cli: Option<bool>,
}

/// Which process owns the destructive sequence.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RunMode {
    Direct,
    Handoff,
    Supervise,
}

/// `_end [-f] [--purge-history|--keep-history] [--assume-stopped] <name|all>`.
#[allow(
    clippy::too_many_lines,
    reason = "the confirmation, handoff and direct paths share one ordered preflight"
)]
pub(crate) fn run(
    root: &Path,
    tail: &[String],
    ambient_pane: Option<&str>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let mut args = match parse(tail) {
        Ok(args) => args,
        Err(message) => {
            writeln!(err, "{message}")?;
            return Ok(EXIT_USAGE);
        }
    };
    if args.pane.is_none() {
        args.pane = ambient_pane.map(ToOwned::to_owned);
    }
    let caller_server = crate::doors::caller_server();
    let caller_session = args
        .pane
        .as_deref()
        .and_then(|pane| super::recorded_caller_session(root, caller_server.as_ref(), pane));
    if args.target.is_empty() {
        let Some(own) = &caller_session else {
            writeln!(err, "{USAGE}")?;
            return Ok(EXIT_USAGE);
        };
        args.target.clone_from(own);
    }
    if args.mode == RunMode::Supervise {
        return run_supervised(root, &args, out, err);
    }
    // The stopped acknowledgement is PER-TARGET destructive intent — never
    // valid for 'all'.
    if args.target == "all" && args.assume_stopped {
        writeln!(
            err,
            "Error: --assume-stopped is per-target only — not valid with 'all'."
        )?;
        return Ok(EXIT_USAGE);
    }

    // RESOLVE BEFORE PROMPTING.
    let targets: Vec<String> = if args.mode == RunMode::Handoff && !args.confirmed.is_empty() {
        let carried: Vec<String> = args
            .confirmed
            .iter()
            .map(|confirmed| confirmed.name.clone())
            .collect();
        let matches_operand =
            args.target == "all" || (carried.len() == 1 && carried.first() == Some(&args.target));
        let unique = carried
            .iter()
            .enumerate()
            .all(|(index, name)| !carried[..index].contains(name));
        if !matches_operand || !unique {
            writeln!(
                err,
                "Error: carried end plans do not match '{}'.",
                args.target
            )?;
            return Ok(EXIT_USAGE);
        }
        carried
    } else if args.target == "all" {
        all_sessions(root)
    } else {
        if !name_is_usable(root, &args.target) {
            writeln!(err, "Session '{}' not found.", args.target)?;
            return Ok(EXIT_FAILED);
        }
        let dir = sessions_dir(root).join(&args.target);
        if !dir_exists(&dir) && !dir_exists(&worktrees_dir(root).join(&args.target)) {
            writeln!(err, "Session '{}' not found.", args.target)?;
            return Ok(EXIT_FAILED);
        }
        vec![args.target.clone()]
    };

    // ONE resolution: the prompt renders from these fields and the frozen
    // contract is built from the same ones.
    let frozen: Vec<(String, Plan)> = targets
        .iter()
        .map(|name| {
            let plan = confirmed_for(&args, name).map_or_else(
                || resolve_plan(root, name, args.purge_cli),
                |confirmed| confirmed.plan.clone(),
            );
            (name.clone(), plan)
        })
        .collect();

    if args.mode == RunMode::Handoff {
        if frozen.is_empty() {
            writeln!(out, "No ae sessions.")?;
            return Ok(0);
        }
        return handoff(
            root,
            &args,
            &frozen,
            caller_session.as_deref(),
            false,
            out,
            err,
        );
    }

    if !args.force
        && !std::io::IsTerminal::is_terminal(&std::io::stdin())
        && let Some(caller) = &caller_session
    {
        let Some(continuation) = handoff_argv(&args, &frozen) else {
            writeln!(
                err,
                "Error: ae cannot name its own executable, so it cannot hand '{}' to a supervisor — nothing was ended.",
                args.target
            )?;
            return Ok(EXIT_FAILED);
        };
        let Some(caller_server) = caller_server.as_ref() else {
            writeln!(err, "Error: the calling tmux server is unknown; pass -f.")?;
            return Ok(EXIT_FAILED);
        };
        return match confirm_on_client(
            caller_server,
            caller,
            &end_prompt(&args.target, &frozen),
            &continuation,
        ) {
            ClientPrompt::Shown => Ok(0),
            ClientPrompt::Nobody => {
                writeln!(err, "Error: nobody attached to confirm; pass -f.")?;
                Ok(EXIT_FAILED)
            }
            ClientPrompt::Failed => {
                writeln!(err, "Error: tmux refused the confirmation prompt; pass -f.")?;
                Ok(EXIT_FAILED)
            }
        };
    }

    if !args.force {
        // Set REGARDLESS of how many targets there were: the human was asked,
        // and what they were asked about is now the whole of what may be ended.
        confirm_body(root, &args, &frozen, err)?;
        write!(err, "Continue? [y/N] ")?;
        err.flush()?;
        match read_reply() {
            Some(reply) if reply.starts_with('y') || reply.starts_with('Y') => {}
            Some(_) => {
                writeln!(err, "Aborted.")?;
                return Ok(0);
            }
            None => {
                writeln!(
                    err,
                    "Error: could not obtain confirmation — no input on stdin."
                )?;
                writeln!(
                    err,
                    "  Nothing was stopped and nothing was deleted. Run it from a terminal, or"
                )?;
                writeln!(err, "  pass -f if you mean to proceed without being asked.")?;
                return Ok(EXIT_FAILED);
            }
        }
    }

    if frozen.is_empty() {
        writeln!(out, "No ae sessions.")?;
        return Ok(0);
    }

    if caller_session
        .as_ref()
        .is_some_and(|caller| targets.iter().any(|target| target == caller))
    {
        return handoff(
            root,
            &args,
            &frozen,
            caller_session.as_deref(),
            true,
            out,
            err,
        );
    }

    // EXACTLY the list the human was shown.
    let mut failures = 0_u32;
    for (name, plan) in &frozen {
        // `-f` freezes nothing, because nothing was promised.
        let contract = confirmed_for(&args, name)
            .map(|confirmed| &confirmed.plan)
            .or_else(|| (!args.force).then_some(plan));
        if !end_one(root, name, &args, contract, out, err)? {
            failures += 1;
        }
    }
    if failures > 0 {
        writeln!(
            err,
            "{failures} session(s) failed to end. See errors above."
        )?;
        return Ok(EXIT_FAILED);
    }
    Ok(0)
}

fn parse(tail: &[String]) -> Result<Args, String> {
    let mut args = Args {
        target: String::new(),
        force: false,
        assume_stopped: false,
        mode: RunMode::Direct,
        pane: None,
        confirmed: Vec::new(),
        purge_cli: None,
    };
    let mut confirmed = Vec::<ConfirmedParts>::new();
    for arg in tail {
        match arg.as_str() {
            "-f" | "--force" => args.force = true,
            "--supervise" => args.mode = RunMode::Supervise,
            "--handoff" => args.mode = RunMode::Handoff,
            "--assume-stopped" => args.assume_stopped = true,
            "--purge-history" => args.purge_cli = Some(true),
            "--keep-history" => args.purge_cli = Some(false),
            "--pane" => {
                return Err("Error: --pane needs a pane id as its next argument.".to_owned());
            }
            flag if flag.starts_with("--pane=") => {
                args.pane = flag.strip_prefix("--pane=").map(ToOwned::to_owned);
            }
            flag if flag.starts_with("--confirmed-target=") => {
                let name = flag.strip_prefix("--confirmed-target=").unwrap_or_default();
                if name.is_empty() {
                    return Err("Error: a carried end plan has no target.".to_owned());
                }
                confirmed.push(ConfirmedParts {
                    name: name.to_owned(),
                    ..ConfirmedParts::default()
                });
            }
            flag if flag.starts_with("--confirmed-action=") => {
                let word = flag.strip_prefix("--confirmed-action=").unwrap_or_default();
                let Some(action) = Action::from_word(word) else {
                    return Err(format!("Error: invalid carried end action '{word}'."));
                };
                last_confirmed(&mut confirmed, flag)?.action = Some(action);
            }
            flag if flag.starts_with("--confirmed-detail=") => {
                let detail = flag.strip_prefix("--confirmed-detail=").unwrap_or_default();
                last_confirmed(&mut confirmed, flag)?.detail = Some(detail.to_owned());
            }
            flag if flag.starts_with("--confirmed-purge=") => {
                let word = flag.strip_prefix("--confirmed-purge=").unwrap_or_default();
                let purge = match word {
                    "on" => true,
                    "off" => false,
                    _ => return Err(format!("Error: invalid carried purge decision '{word}'.")),
                };
                last_confirmed(&mut confirmed, flag)?.purge = Some(purge);
            }
            flag if flag.starts_with("--confirmed-source=") => {
                let word = flag.strip_prefix("--confirmed-source=").unwrap_or_default();
                let from_default = match word {
                    "default" => true,
                    "explicit" => false,
                    _ => return Err(format!("Error: invalid carried end source '{word}'.")),
                };
                last_confirmed(&mut confirmed, flag)?.from_default = Some(from_default);
            }
            flag if flag.starts_with('-') => {
                return Err(format!(
                    "Error: unknown flag '{flag}'. Use -f, --purge-history, --keep-history, --assume-stopped."
                ));
            }
            name if args.target.is_empty() => name.clone_into(&mut args.target),
            // Destructive command — never silently drop a stray arg.
            extra => {
                return Err(format!(
                    "Error: unexpected extra argument '{extra}' — _end takes one session name (or 'all')."
                ));
            }
        }
    }
    args.confirmed = confirmed
        .into_iter()
        .map(|parts| {
            let (Some(action), Some(detail), Some(purge), Some(from_default)) =
                (parts.action, parts.detail, parts.purge, parts.from_default)
            else {
                return Err(format!(
                    "Error: carried end plan for '{}' is incomplete.",
                    parts.name
                ));
            };
            Ok(ConfirmedPlan {
                name: parts.name,
                plan: Plan {
                    action,
                    detail,
                    purge,
                },
                from_default,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(args)
}

fn last_confirmed<'a>(
    confirmed: &'a mut [ConfirmedParts],
    flag: &str,
) -> Result<&'a mut ConfirmedParts, String> {
    confirmed
        .last_mut()
        .ok_or_else(|| format!("Error: '{flag}' has no carried end target."))
}

fn confirmed_for<'a>(args: &'a Args, name: &str) -> Option<&'a ConfirmedPlan> {
    args.confirmed
        .iter()
        .find(|confirmed| confirmed.name == name)
}

fn confirmed_plans(args: &Args, frozen: &[(String, Plan)]) -> Vec<ConfirmedPlan> {
    if !args.confirmed.is_empty() {
        return args.confirmed.clone();
    }
    frozen
        .iter()
        .map(|(name, plan)| ConfirmedPlan {
            name: name.clone(),
            plan: plan.clone(),
            from_default: args.purge_cli.is_none(),
        })
        .collect()
}

fn end_prompt(name: &str, frozen: &[(String, Plan)]) -> String {
    let has_purge = frozen.iter().any(|(_, plan)| plan.purge);
    let has_keep = frozen.iter().any(|(_, plan)| !plan.purge);
    let consequence = match (has_keep, has_purge) {
        (true, false) => "Archives, then deletes its state.",
        (false, true) => "Deletes its state and purges the agent history.",
        (true, true) => "Archives kept histories, purges configured histories, then deletes state.",
        (false, false) => "Deletes its state.",
    };
    format!("End '{name}'? {consequence} (y/n)")
}

fn push_confirmed(command: &mut Vec<String>, confirmed: &ConfirmedPlan) {
    command.push(format!("--confirmed-target={}", confirmed.name));
    command.push(format!(
        "--confirmed-action={}",
        confirmed.plan.action.word()
    ));
    command.push(format!("--confirmed-detail={}", confirmed.plan.detail));
    command.push(format!(
        "--confirmed-purge={}",
        if confirmed.plan.purge { "on" } else { "off" }
    ));
    command.push(format!(
        "--confirmed-source={}",
        if confirmed.from_default {
            "default"
        } else {
            "explicit"
        }
    ));
    command.push(
        if confirmed.plan.purge {
            "--purge-history"
        } else {
            "--keep-history"
        }
        .to_owned(),
    );
}

/// The detached worker's exact argv.
fn supervisor_argv(args: &Args, confirmed: &ConfirmedPlan) -> Option<DetachedArgv> {
    let own = crate::shape::resolved_exe()?;
    Some(supervisor_args(
        own.to_string_lossy().into_owned(),
        args,
        confirmed,
    ))
}

fn supervisor_args(core: String, args: &Args, confirmed: &ConfirmedPlan) -> DetachedArgv {
    let mut command = vec![
        core,
        crate::cli::END.to_owned(),
        "--supervise".to_owned(),
        confirmed.name.clone(),
    ];
    push_confirmed(&mut command, confirmed);
    if args.assume_stopped {
        command.push("--assume-stopped".to_owned());
    }
    if let Some(pane) = args.pane.as_deref().filter(|pane| !pane.is_empty()) {
        command.push(format!("--pane={pane}"));
    }
    DetachedArgv(command)
}

/// The short-lived tmux job's exact argv. It records intent, starts the
/// detached worker and exits before the target session is killed.
fn handoff_argv(args: &Args, frozen: &[(String, Plan)]) -> Option<DetachedArgv> {
    let own = crate::shape::resolved_exe()?;
    let mut command = vec![
        own.to_string_lossy().into_owned(),
        crate::cli::END.to_owned(),
        "--handoff".to_owned(),
        args.target.clone(),
    ];
    for confirmed in confirmed_plans(args, frozen) {
        push_confirmed(&mut command, &confirmed);
    }
    if args.assume_stopped {
        command.push("--assume-stopped".to_owned());
    }
    if let Some(pane) = args.pane.as_deref().filter(|pane| !pane.is_empty()) {
        command.push(format!("--pane={pane}"));
    }
    Some(DetachedArgv(command))
}

/// Facts which survive the successful removal of a session's live state.
struct Notice {
    name: String,
    dir: PathBuf,
    server: Option<ServerId>,
    archive_id: String,
    purge: bool,
}

fn notices(root: &Path, args: &Args) -> Vec<Notice> {
    args.confirmed
        .iter()
        .map(|confirmed| {
            let name = confirmed.name.clone();
            let dir = sessions_dir(root).join(&name);
            let bytes = meta::read_bytes(&dir).unwrap_or_default();
            Notice {
                server: recorded_server(root, &name),
                archive_id: archive::canonical_uuid(&meta_value(&bytes, "session_id")),
                purge: confirmed.plan.purge,
                name,
                dir,
            }
        })
        .collect()
}

fn request_summary(pane: Option<&str>) -> String {
    let Some(pane) = pane.filter(|pane| !pane.is_empty()) else {
        return "end requested by supervisor".to_owned();
    };
    let agent = crate::doors::caller_server()
        .as_ref()
        .and_then(|server| transport::observe_viewer(server, pane))
        .and_then(|viewer| viewer.agent)
        .unwrap_or_else(|| "unknown".to_owned());
    format!("end requested from inside by {agent}/{pane}")
}

/// `_end --supervise`: run the whole end where killing a target pane cannot
/// kill this process. The handoff's request is captured by the archive; only
/// failures get a result event because only failures retain a live directory.
fn run_supervised(
    root: &Path,
    args: &Args,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    if args.confirmed.len() != 1
        || args.confirmed.first().map(|plan| &plan.name) != Some(&args.target)
    {
        writeln!(
            err,
            "Error: end supervisor received no matching confirmed plan."
        )?;
        return Ok(EXIT_USAGE);
    }
    let notices = notices(root, args);
    let mut tail = vec!["-f".to_owned(), args.target.clone()];
    for confirmed in &args.confirmed {
        push_confirmed(&mut tail, confirmed);
    }
    if args.assume_stopped {
        tail.push("--assume-stopped".to_owned());
    }
    let mut captured_out = Vec::new();
    let mut captured_err = Vec::new();
    let code = match run(root, &tail, None, &mut captured_out, &mut captured_err) {
        Ok(code) => code,
        Err(why) => {
            let summary = format!("FAILED: end supervisor I/O: {why}");
            for notice in &notices {
                if dir_exists(&notice.dir) {
                    emit_lifecycle_event(&notice.dir, &notice.name, END_RESULT_ACTION, &summary);
                }
            }
            writeln!(err, "Error: end supervisor I/O failed: {why}")?;
            return Ok(EXIT_FAILED);
        }
    };

    if code == 0 {
        for notice in &notices {
            let line = if !notice.purge && !notice.archive_id.is_empty() {
                format!("Ended {} — archived {}", notice.name, notice.archive_id)
            } else {
                format!("Ended {}", notice.name)
            };
            if let Some(server) = &notice.server {
                announce_to_clients(server, &line);
            }
        }
    } else {
        let reason = String::from_utf8_lossy(&captured_err);
        let reason = reason
            .lines()
            .find(|line| line.starts_with("Error:"))
            .or_else(|| reason.lines().find(|line| !line.is_empty()))
            .unwrap_or("end failed");
        let summary = format!("FAILED: {reason}");
        for notice in &notices {
            if dir_exists(&notice.dir) {
                emit_lifecycle_event(&notice.dir, &notice.name, END_RESULT_ACTION, &summary);
            }
        }
    }
    out.write_all(&captured_out)?;
    err.write_all(&captured_err)?;
    Ok(code)
}

/// Hand an already-authorized end to a detached process.
fn handoff(
    root: &Path,
    args: &Args,
    frozen: &[(String, Plan)],
    caller_session: Option<&str>,
    report_start: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let carried = confirmed_plans(args, frozen);
    let mut ordered: Vec<&String> = frozen.iter().map(|(name, _)| name).collect();
    ordered.sort_by_key(|name| u8::from(caller_session == Some(name.as_str())));
    let Some(supervisors) = ordered
        .iter()
        .map(|name| {
            carried
                .iter()
                .find(|confirmed| confirmed.name == name.as_str())
                .and_then(|confirmed| supervisor_argv(args, confirmed))
                .map(|argv| (*name, argv))
        })
        .collect::<Option<Vec<_>>>()
    else {
        writeln!(
            err,
            "Error: ae cannot name its own executable, so it cannot hand '{}' to a supervisor — nothing was ended.",
            args.target
        )?;
        return Ok(EXIT_FAILED);
    };
    let requested = request_summary(args.pane.as_deref());
    for (name, _) in frozen {
        emit_lifecycle_event(
            &sessions_dir(root).join(name),
            name,
            END_REQUEST_ACTION,
            &requested,
        );
    }
    let mut failures = 0_u32;
    for (name, argv) in supervisors {
        if !transport::run_detached(&argv) {
            failures += 1;
            let dir = sessions_dir(root).join(name);
            emit_lifecycle_event(
                &dir,
                name,
                END_RESULT_ACTION,
                "FAILED: supervisor did not start",
            );
        }
    }
    if failures > 0 {
        writeln!(
            err,
            "Error: could not start {failures} end supervisor(s) for '{}' — affected state was preserved.",
            args.target,
        )?;
        return Ok(EXIT_FAILED);
    }
    if !report_start {
        return Ok(0);
    }
    if args.target == "all" {
        writeln!(out, "Ending all ae sessions out of pane.")?;
    } else {
        writeln!(
            out,
            "Ending '{}' out of pane; this pane will close.",
            args.target
        )?;
    }
    Ok(0)
}

/// Read the confirmation.
fn read_reply() -> Option<String> {
    let mut buffer = String::new();
    // A LINE, not to EOF: a human at a terminal answers and presses Enter, and
    // reading to EOF there would block forever waiting for a ^D they were never
    // asked for.
    match std::io::stdin().read_line(&mut buffer) {
        Ok(read) if read > 0 => Some(buffer.trim_start().to_owned()),
        _ => None,
    }
}

/// The confirmation body — on STDERR, so `_end`'s stdout carries only what
/// actually happened.
fn confirm_body(
    root: &Path,
    args: &Args,
    frozen: &[(String, Plan)],
    err: &mut impl Write,
) -> io::Result<()> {
    if args.target == "all" {
        writeln!(err, "This will END every ae session:")?;
    } else {
        writeln!(err, "This will END the session:")?;
    }
    for (name, plan) in frozen {
        writeln!(err, "{}", plan.line(name))?;
    }
    if frozen.is_empty() {
        writeln!(err, "  (none)")?;
    }
    writeln!(
        err,
        "Work in a managed workspace is committed and pushed to ae/<session> first; the session is then removed from {}.",
        sessions_dir(root).display()
    )?;
    Ok(())
}

/// What `ae end` will do with this session's memory, resolved BEFORE the prompt
/// so the confirmation can name the exact path.
fn resolve_plan(root: &Path, name: &str, purge_cli: Option<bool>) -> Plan {
    let dir = sessions_dir(root).join(name);
    let bytes = meta::read_bytes(&dir).unwrap_or_default();
    let purge = effective_purge(&bytes, purge_cli);
    let archive_root = root.join("archive");

    if meta::read_bytes(&dir).is_err() {
        // A missing meta is not the same as nothing to lose.
        let store = crate::store::open(&dir);
        let has_memory = has_nonempty(&store.memo_path())
            || has_nonempty(&store.events_path())
            || has_message_payload(&dir.join("messages"));
        return if has_memory {
            Plan {
                action: Action::Unavailable,
                detail: "its meta is missing, so ae cannot identify the session — but its memo, events or request payloads are still there".to_owned(),
                purge,
            }
        } else {
            Plan {
                action: Action::Nothing,
                detail: "no session memory to archive".to_owned(),
                purge,
            }
        };
    }

    let raw = meta_value(&bytes, "session_id");
    let aid = archive::canonical_uuid(&raw);
    // ABSENT vs CORRUPT, decided BEFORE purge gets a say.
    if aid.is_empty() && !raw.is_empty() {
        return Plan {
            action: Action::Unavailable,
            detail: format!(
                "its session_id ({raw}) is not a UUID, and ae will not overwrite a value it cannot parse"
            ),
            purge,
        };
    }
    if aid.is_empty() {
        // CLEAN CUT, as `_compact-freeze` already rules for the same state: a
        // session with no valid id is unsupported old state, refused with a
        // refresh/migrate instruction rather than minted an id on the way to
        // an immutable archive.
        return Plan {
            action: Action::Unavailable,
            detail: "it records no valid session id — refresh or migrate the session, then retry"
                .to_owned(),
            purge,
        };
    }
    if purge {
        return Plan {
            action: Action::Purge,
            detail: archive_root.join(&aid).display().to_string(),
            purge,
        };
    }
    Plan {
        action: Action::Keep,
        detail: archive_root.join(&aid).display().to_string(),
        purge,
    }
}

/// THE purge sensor — one definition, consulted by the confirmation prompt
/// (before the human answers) and by the cleanup (when it acts).
fn effective_purge(bytes: &[u8], purge_cli: Option<bool>) -> bool {
    if let Some(explicit) = purge_cli {
        return explicit;
    }
    workspace_of(bytes).is_some_and(|w| w.purge_agent_history)
}

/// The `[workspace]` block a session's own config resolves to — its recorded
/// global config layered under its origin's local `.ae/config`.
fn workspace_of(bytes: &[u8]) -> Option<Workspace> {
    let config = meta_value(bytes, "config");
    let origin = meta_value(bytes, "origin");
    let global = (!config.is_empty() && config != "/dev/null")
        .then(|| PathBuf::from(&config))
        .filter(|path| archive::regular_file(path));
    let local = (!origin.is_empty())
        .then(|| Path::new(&origin).join(".ae").join("config"))
        .filter(|path| archive::regular_file(path));
    read_workspace(global.as_deref(), local.as_deref()).ok()
}

fn has_nonempty(path: &Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the archive plan must know whether a meta-less session directory still holds memory before calling it unarchivable"
    )]
    let meta = std::fs::metadata(path);
    meta.is_ok_and(|m| m.is_file() && m.len() > 0)
}

fn has_message_payload(dir: &Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the same question as `has_nonempty`, over the request-payload directory"
    )]
    let entries = std::fs::read_dir(dir);
    entries.is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".txt"))
    })
}

// ---- one session -----------------------------------------------------------

/// End one session, under its lifecycle lock.
#[allow(
    clippy::too_many_lines,
    reason = "the frozen order IS the contract; splitting it would put the sequence in two places"
)]
fn end_one(
    root: &Path,
    name: &str,
    args: &Args,
    contract: Option<&Plan>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<bool> {
    let Ok(_guard) = lock(root, name) else {
        writeln!(
            err,
            "Error: another lifecycle operation (start/resume/end) is in progress for '{name}' — retry shortly. State preserved."
        )?;
        return Ok(false);
    };
    let dir = sessions_dir(root).join(name);
    // The chain, under the lifecycle lock this end already holds. A refusal is
    // REPORTED, never fatal — the whole point of an end is that a session can
    // always be torn down, whatever shape its meta is in.
    if let Some(note) = crate::migrate::session_noted(&dir, name) {
        writeln!(err, "{note}")?;
    }
    let bytes = meta::read_bytes(&dir).unwrap_or_default();

    // ══ THE INVARIANT: ae NEVER deletes session state unless
    //    (a) the target was positively identified on ITS OWN recorded server
    //        and its kill was verified, or
    //    (b) the human passed --assume-stopped for THIS single target, and the
    //        full enumerable sweep found nothing AND left no unverifiable
    //        socket standing, or
    //    (c) the target's POSITIVE record names a server that verifiably lacks
    //        the session.
    let selector = server_of(&bytes);
    let mut session_id: Option<String> = None;
    let mut server = None;
    match selector {
        ServerSelector::Positive(sel) => {
            let id = ServerId::Selected(sel);
            session_id = live_id(&id, name);
            if session_id.is_none()
                && transport::verify_session_absent(&id, name) == StopProbe::Unknown
            {
                writeln!(
                    err,
                    "Error: cannot verify session '{name}' state (its recorded tmux server is unreachable) — state preserved."
                )?;
                return Ok(false);
            }
            server = Some(id);
        }
        ServerSelector::Missing | ServerSelector::Ambiguous => {
            // No positive ownership record.
            match sweep(root, name) {
                Sweep::Found(where_) => {
                    writeln!(
                        err,
                        "Error: session '{name}' has no positive server record AND is LIVE on enumerable server '{where_}'."
                    )?;
                    writeln!(
                        err,
                        "  Establish ownership first: 'ae doctor --refresh {name}'."
                    )?;
                    writeln!(err, "  State preserved.")?;
                    return Ok(false);
                }
                Sweep::Unsure(where_) => {
                    writeln!(
                        err,
                        "Error: cannot verify enumerable tmux server '{where_}' (socket exists but did not answer) — session '{name}' could be hiding behind it."
                    )?;
                    writeln!(
                        err,
                        "  Fix that server (permissions/staleness), then retry. State preserved."
                    )?;
                    return Ok(false);
                }
                Sweep::Clear => {}
            }
            if !args.assume_stopped {
                writeln!(
                    err,
                    "Error: session '{name}' has no positive server record (pre-fix or ambiguous meta) — it may be live somewhere ae cannot enumerate."
                )?;
                writeln!(
                    err,
                    "  Resolve: 'ae doctor --refresh {name}', or acknowledge it is stopped: ae end --assume-stopped {name}"
                )?;
                writeln!(err, "  State preserved.")?;
                return Ok(false);
            }
            writeln!(
                out,
                "Proceeding on explicit --assume-stopped acknowledgement (not live on any enumerable server, default included)."
            )?;
        }
    }

    let mode = meta_value(&bytes, "mode");
    let origin = meta_value(&bytes, "origin");
    let work_dir = resolve_workdir(root, &bytes, name);

    // A session ae cannot IDENTIFY is refused HERE, before anything is stopped:
    // the archive step would refuse it at the end anyway, and there is no
    // reason to stop a session for a failure already visible.
    let current_purge = confirmed_for(args, name).map_or(args.purge_cli, |confirmed| {
        if confirmed.from_default {
            None
        } else {
            Some(confirmed.plan.purge)
        }
    });
    let plan = resolve_plan(root, name, current_purge);
    if plan.action == Action::Unavailable {
        writeln!(
            err,
            "Error: session '{name}' cannot be archived — {}.",
            plan.detail
        )?;
        writeln!(
            err,
            "  Nothing was stopped and NOTHING was deleted; ae does not delete a session it could not capture."
        )?;
        return Ok(false);
    }
    // A contract that no longer matches the one confirmed is caught here too,
    // not only at the archive step: the step is the backstop under the lock,
    // but by then the session has been stopped.
    if let Some(frozen) = contract
        && *frozen != plan
    {
        writeln!(
            err,
            "Error: what '{name}' would do changed between the confirmation and now — refusing to act on an answer given to a different question."
        )?;
        writeln!(err, "  Confirmed: {}", frozen.line(name))?;
        writeln!(err, "  Now:       {}", plan.line(name))?;
        writeln!(
            err,
            "  Nothing was stopped and nothing was deleted. Re-run 'ae end {name}' to see the current plan."
        )?;
        return Ok(false);
    }

    // Local mode manages no branch of its own, so the archive records the git
    // outcome honestly as not-managed rather than inventing a range from the
    // human's own checkout.
    if mode == "local" || mode.is_empty() {
        if let (Some(server), Some(id)) = (server.as_ref(), session_id.as_ref())
            && !kill_verified(server, name, "end", id, err)?
        {
            return Ok(false);
        }
        // CAPTURE BEFORE DELETE.
        if !archive_step(root, name, &plan, &Git::none(), out, err)? {
            return Ok(false);
        }
        if !cleanup(root, name, &bytes, &plan, false, out, err)? {
            return Ok(false);
        }
        writeln!(out, "Ended local session {name}")?;
        return Ok(true);
    }

    // git/full mode.
    let wdir_bytes = work_dir.as_os_str().as_encoded_bytes().to_vec();
    if !dir_exists(&work_dir) || !crate::git::is_work_tree(&wdir_bytes) {
        writeln!(
            err,
            "Error: working directory is not a git repo — cannot preserve work."
        )?;
        writeln!(
            err,
            "  Nothing was stopped and nothing was deleted; the session is still running."
        )?;
        writeln!(err, "  Working directory: {}", work_dir.display())?;
        return Ok(false);
    }
    // STOP FIRST — and verify it.
    if let (Some(server), Some(id)) = (server.as_ref(), session_id.as_ref())
        && !kill_verified(server, name, "end", id, err)?
    {
        return Ok(false);
    }

    if crate::git::has_pending_work(&wdir_bytes) {
        writeln!(out, "Committing changes in {name}...")?;
        let stamp = Timestamp::now().to_string();
        let human = stamp.replace('T', " ").replace('Z', " UTC");
        if !crate::git::commit_all(
            &wdir_bytes,
            &format!("ae: end session {name}"),
            &format!("Ended: {human}"),
        ) {
            writeln!(
                err,
                "Error: commit failed. The session is STOPPED; nothing was deleted."
            )?;
            writeln!(
                err,
                "  Your work, ae state and agent conversation files are all preserved."
            )?;
            writeln!(err, "  Working directory: {}", work_dir.display())?;
            writeln!(err, "  Fix the commit there, then re-run: ae end {name}")?;
            writeln!(err, "  Or pick the session back up: ae {name}")?;
            return Ok(false);
        }
    }

    // The push outcome is recorded by the branch that ACTUALLY runs and handed
    // to the archive — never rediscovered afterwards, when the remote may have
    // moved and the worktree may already be gone.
    let has_origin = crate::git::has_origin(&wdir_bytes);
    let mut git = Git::none();
    let mut preserve_workdir = false;
    if has_origin {
        crate::git::fetch_origin(&wdir_bytes);
    }
    if has_origin && !crate::git::head_is_on_a_remote(&wdir_bytes) {
        let branch = sanitize_branch_name(name);
        writeln!(out, "Pushing to origin/{branch}...")?;
        writeln!(
            out,
            "  {} file(s) changed",
            crate::git::pushed_file_count(&wdir_bytes)
        )?;
        if !crate::git::push_head(&wdir_bytes, &branch) {
            writeln!(
                err,
                "Error: push failed. The session is STOPPED; nothing was deleted."
            )?;
            writeln!(
                err,
                "  The commit is safe locally — ae state and the working tree are preserved."
            )?;
            writeln!(err, "  Working directory: {}", work_dir.display())?;
            writeln!(
                err,
                "  To retry the push: cd {} && git push -u origin HEAD:refs/heads/{branch}",
                work_dir.display()
            )?;
            writeln!(err, "  Then finish up:    ae end {name}")?;
            writeln!(err, "  Or pick the session back up: ae {name}")?;
            return Ok(false);
        }
        writeln!(out, "Pushed to origin/{branch}")?;
        "pushed".clone_into(&mut git.outcome);
        git.push_ref = format!("origin/{branch}");
        git.workdir = work_dir.display().to_string();
    } else if has_origin {
        writeln!(out, "No new commits to push.")?;
        "already-reachable".clone_into(&mut git.outcome);
        git.workdir = work_dir.display().to_string();
    } else {
        // B3 durability: a no-remote git target just COMMITTED work that exists
        // ONLY in this directory.
        writeln!(out, "No remote 'origin' — skipping push.")?;
        preserve_workdir = true;
        "no-origin".clone_into(&mut git.outcome);
        git.preserved = work_dir.display().to_string();
        git.workdir = work_dir.display().to_string();
    }

    // The last thing before the session stops existing: capture it, or fail and
    // keep it.
    if !archive_step(root, name, &plan, &git, out, err)? {
        return Ok(false);
    }
    if !cleanup(root, name, &bytes, &plan, preserve_workdir, out, err)? {
        return Ok(false);
    }
    if preserve_workdir {
        writeln!(
            out,
            "Ended {name} — work committed locally (no origin remote to push to)."
        )?;
        writeln!(out, "  Directory preserved: {}", work_dir.display())?;
    } else {
        writeln!(out, "Ended {name}")?;
    }
    let _ = (origin, mode);
    Ok(true)
}

/// The operation facts the archive publisher needs and the core does not derive
/// itself.
struct Git {
    outcome: String,
    push_ref: String,
    preserved: String,
    workdir: String,
}

impl Git {
    /// Local mode manages no branch of its own.
    fn none() -> Self {
        Self {
            outcome: "not-managed".to_owned(),
            push_ref: "-".to_owned(),
            preserved: "-".to_owned(),
            workdir: String::new(),
        }
    }
}

/// Publish the archive (or perform the purge) for a session that is verifiably
/// stopped, BEFORE any live state is removed.
fn archive_step(
    root: &Path,
    name: &str,
    plan: &Plan,
    git: &Git,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<bool> {
    let dir = sessions_dir(root).join(name);
    match plan.action {
        Action::Nothing => Ok(true),
        Action::Unavailable => {
            writeln!(
                err,
                "Error: session '{name}' cannot be archived — {}.",
                plan.detail
            )?;
            writeln!(
                err,
                "  The session is STOPPED and NOTHING was deleted; ae does not delete a session it could not capture."
            )?;
            Ok(false)
        }
        Action::Purge => {
            let bytes = meta::read_bytes(&dir).unwrap_or_default();
            let aid = archive::canonical_uuid(&meta_value(&bytes, "session_id"));
            if aid.is_empty() {
                writeln!(
                    out,
                    "No archive written (--purge-history); this session never had an id to key one."
                )?;
                return Ok(true);
            }
            let parent = meta_value(&bytes, "parent_archive_id");
            let mut captured = Vec::new();
            let code = archive::purge::run(&dir, &aid, name, &parent, &mut captured, err)?;
            if code != 0 {
                // The purge already emitted the precise state — pre-commit:
                // nothing deleted; post-commit: PURGE INCOMPLETE.
                writeln!(
                    err,
                    "Error: purging the archive for '{name}' failed (see above) — the session is STOPPED."
                )?;
                return Ok(false);
            }
            let removed = String::from_utf8_lossy(&captured).trim().to_owned();
            if removed.is_empty() {
                writeln!(
                    out,
                    "No archive written (--purge-history); none existed for {aid}."
                )?;
            } else {
                writeln!(out, "Purged archive {aid}")?;
                writeln!(out, "  removed {removed}")?;
            }
            Ok(true)
        }
        Action::Keep => {
            let bytes = meta::read_bytes(&dir).unwrap_or_default();
            let aid = archive::canonical_uuid(&meta_value(&bytes, "session_id"));
            let archived_at = Timestamp::now().to_string();
            let ops = archive::publish::Ops {
                push_outcome: &git.outcome,
                push_ref: if git.push_ref.is_empty() {
                    "-"
                } else {
                    &git.push_ref
                },
                preserved: if git.preserved.is_empty() {
                    "-"
                } else {
                    &git.preserved
                },
                workdir: &git.workdir,
                archived_at: &archived_at,
            };
            let mut captured = Vec::new();
            let code = archive::publish::run(&dir, &ops, &mut captured, err)?;
            if code != 0 {
                writeln!(
                    err,
                    "Error: archiving session '{name}' failed — the session is STOPPED and NOTHING was deleted."
                )?;
                writeln!(
                    err,
                    "  Fix the cause reported above, then re-run: ae end {name}"
                )?;
                return Ok(false);
            }
            let line = String::from_utf8_lossy(&captured);
            let mut fields = line.trim_end_matches('\n').split('\t');
            let path = fields.next().unwrap_or_default();
            let files = fields.next().unwrap_or_default();
            let count = fields.next().unwrap_or_default();
            writeln!(out, "Archived {aid}")?;
            writeln!(out, "  {path}")?;
            writeln!(out, "  {files} file(s), {count} byte(s)")?;
            Ok(true)
        }
    }
}

/// Remove the live session.
fn cleanup(
    root: &Path,
    name: &str,
    bytes: &[u8],
    plan: &Plan,
    preserve: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<bool> {
    let dir = sessions_dir(root).join(name);
    let mode = meta_value(bytes, "mode");
    let origin = meta_value(bytes, "origin");

    // Read meta BEFORE the removal below.
    if plan.purge {
        purge_conversation_files(root, bytes, out, err)?;
    } else {
        writeln!(
            out,
            "  kept agent conversation files (claude/codex/agy token history; purge: ae end --purge-history)"
        )?;
    }

    // The core owns the removal — a rename-to-tombstone commit boundary that
    // clears the canonical name atomically and reports success only after the
    // removal is durable.
    let valid = super::name_is_valid(name);
    if (mode == "local" || mode.is_empty()) && valid {
        if crate::teardown::run(&dir, out, err)? != 0 {
            writeln!(
                err,
                "Error: the session state for '{name}' was not fully removed — see the teardown diagnostic above; the session is NOT ended."
            )?;
            if plan.purge {
                writeln!(
                    err,
                    "  The agent conversation history WAS purged as requested; only the session-state removal did not complete."
                )?;
            }
            return Ok(false);
        }
        // Older layouts: the worktree-nested and origin-nested paths.
        remove_legacy(root, name, &origin);
        if preserve {
            writeln!(
                out,
                "Removed ae session state for {name} (working directory preserved)"
            )?;
        } else {
            writeln!(out, "Cleaned up local session {name}")?;
        }
        return Ok(true);
    }
    if (mode == "full" || mode == "git") && valid {
        // The core owns BOTH the managed workdir AND the canonical state
        // (workdir first, canonical last).
        if crate::teardown::run_nonlocal(&dir, preserve, out, err)? != 0 {
            writeln!(
                err,
                "Error: the teardown of '{name}' did not complete — see the diagnostic above; the session is NOT ended."
            )?;
            if plan.purge {
                writeln!(
                    err,
                    "  The agent conversation history WAS purged as requested; only the teardown did not complete."
                )?;
            }
            return Ok(false);
        }
        // The core already removed the managed workdir together with the
        // canonical state; it owns BOTH resources.
        if mode == "git" && !origin.is_empty() {
            crate::git::worktree_prune(origin.as_bytes());
        }
        if !preserve {
            writeln!(out, "Removed worktree for {name}")?;
        }
        return Ok(true);
    }
    // A grammar-invalid name: removed here, so it is never made un-endable.
    let _ = std::fs::remove_dir_all(&dir);
    remove_legacy(root, name, &origin);
    if preserve {
        writeln!(
            out,
            "Removed ae session state for {name} (working directory preserved)"
        )?;
    } else {
        writeln!(out, "Cleaned up local session {name}")?;
    }
    Ok(true)
}

/// The pre-canonical state locations, removed best-effort.
fn remove_legacy(root: &Path, name: &str, origin: &str) {
    let _ = std::fs::remove_dir_all(worktrees_dir(root).join(name).join(".ae").join(name));
    if !origin.is_empty() {
        let nested = Path::new(origin).join(".ae").join(name);
        if dir_exists(&nested) {
            let _ = std::fs::remove_dir_all(nested);
        }
    }
}

/// Delete the agent CLI conversation files this session's captured harness ids
/// name.
fn purge_conversation_files(
    root: &Path,
    bytes: &[u8],
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<()> {
    let text = String::from_utf8_lossy(bytes);
    let parsed = meta::Meta::parse(&text);
    let home = root.parent().map(Path::to_path_buf);
    for entry in parsed.roster() {
        let recorded = entry.harness_session.as_deref().unwrap_or_default();
        if recorded.is_empty() || recorded == "pending" {
            continue;
        }
        // A NAME, NEVER A PATH. `harness_session.<slot>` is metadata — a
        // hand-editable file, and `set-harness-session` screens only for control
        // bytes — and every arm below interpolates it into a filename that is
        // then joined onto a tool's store and REMOVED. `Path::join` with an
        // absolute operand DISCARDS the store entirely, so a row reading
        // `/etc/passwd` is not a conversation id, it is a deletion target; and
        // `../../x` walks out of the store just as well. So the value is proven
        // to be a UUID first, with the grammar the archive already uses, and an
        // id that is not one names NOTHING — reported as a loss, because a
        // conversation ae cannot safely name is a conversation the purge did not
        // remove, and the operator asked for it to be gone.
        let uuid = crate::archive::canonical_uuid(recorded);
        if uuid.is_empty() {
            writeln!(
                err,
                "  note: conversation file for slot {} left in place (recorded id is not a UUID)",
                entry.slot
            )?;
            continue;
        }
        let uuid = uuid.as_str();
        let tool = entry.binary.as_deref().unwrap_or_default();
        let Some(home) = home.as_ref() else { continue };
        match tool {
            "claude" => {
                if let Some(file) = find_claude_file(home, uuid)
                    && std::fs::remove_file(&file).is_ok()
                {
                    writeln!(out, "  removed claude conversation: {}", file.display())?;
                }
            }
            "codex" => {
                if let Some(file) = find_codex_file(home, uuid)
                    && std::fs::remove_file(&file).is_ok()
                {
                    writeln!(out, "  removed codex rollout: {}", file.display())?;
                }
            }
            // agy names its conversation EXACTLY: one file per id, in one flat
            // directory, so there is no lookup to implement and no ambiguity to
            // refuse. The `-wal`/`-shm` siblings go with it — left behind they
            // are an orphaned write-ahead log for a database that no longer
            // exists, which is the same retention leak as the database itself.
            "agy" => {
                for file in agy_conversation_files(home, uuid) {
                    if std::fs::remove_file(&file).is_ok() {
                        writeln!(out, "  removed agy conversation: {}", file.display())?;
                    }
                }
            }
            "gemini" | "opencode" => {
                writeln!(
                    err,
                    "  note: {tool} conversation file for slot {} left in place (lookup not yet implemented)",
                    entry.slot
                )?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// agy's conversation database for `uuid`, and the `SQLite` sidecars beside it.
///
/// No search and no ambiguity check, unlike the claude and codex finders: agy
/// keeps one file per conversation NAMED for the id in one flat directory, so
/// the path is computed rather than looked for, and only paths that exist are
/// returned. `-wal` and `-shm` are appended to the FULL file name (`<id>.db-wal`
/// is what `SQLite` writes), which is why they are built from the database's own
/// name rather than from a second extension.
///
/// `uuid` is the caller's proven one. Computing a path from an unvalidated id is
/// how a metadata row becomes a deletion target, so the check lives at the
/// boundary where the row is READ rather than here — one guard covering every
/// arm, instead of one per tool that a fourth tool would be added without.
fn agy_conversation_files(home: &Path, uuid: &str) -> Vec<PathBuf> {
    let store = home.join(crate::session_launch::capture::AGY_CONVERSATIONS);
    [
        format!("{uuid}.db"),
        format!("{uuid}.db-wal"),
        format!("{uuid}.db-shm"),
    ]
    .into_iter()
    .map(|name| store.join(name))
    .filter(|path| path_exists(path))
    .collect()
}

/// `~/.claude/projects/*/<uuid>.jsonl`, and only when exactly one matches.
fn find_claude_file(home: &Path, uuid: &str) -> Option<PathBuf> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the frozen conversation-file purge enumerates ~/.claude/projects"
    )]
    let entries = std::fs::read_dir(home.join(".claude").join("projects")).ok()?;
    let mut hits: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path().join(format!("{uuid}.jsonl")))
        .filter(|path| path_exists(path))
        .collect();
    if hits.len() == 1 { hits.pop() } else { None }
}

/// `~/.codex/sessions/Y/M/D/*<uuid>*.jsonl`, and only when exactly one matches.
fn find_codex_file(home: &Path, uuid: &str) -> Option<PathBuf> {
    fn children(path: &Path) -> Vec<PathBuf> {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the frozen conversation-file purge walks ~/.codex/sessions/Y/M/D"
        )]
        let entries = std::fs::read_dir(path);
        entries.map_or_else(
            |_| Vec::new(),
            |entries| entries.flatten().map(|entry| entry.path()).collect(),
        )
    }
    let mut level = vec![home.join(".codex").join("sessions")];
    for _ in 0..3 {
        level = level.iter().flat_map(|path| children(path)).collect();
    }
    let mut hits: Vec<PathBuf> = level
        .into_iter()
        .filter(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            name.contains(uuid) && name.ends_with(".jsonl")
        })
        .collect();
    if hits.len() == 1 { hits.pop() } else { None }
}

/// Where a session's work is — its recorded `work_dir`, else the standard
/// worktree path.
fn resolve_workdir(root: &Path, bytes: &[u8], name: &str) -> PathBuf {
    let recorded = meta_value(bytes, "work_dir");
    if !recorded.is_empty() {
        let path = PathBuf::from(&recorded);
        if dir_exists(&path) {
            return path;
        }
    }
    worktrees_dir(root).join(name)
}

/// `sanitize_branch_name`: strip an `ae-` prefix, replace everything outside
/// `[A-Za-z0-9._-]` with `-`, collapse repeats, trim edges, prefix `ae/`.
fn sanitize_branch_name(name: &str) -> String {
    let stripped = name.strip_prefix("ae-").unwrap_or(name);
    let mut out = String::with_capacity(stripped.len());
    for ch in stripped.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    format!("ae/{}", out.trim_matches('-'))
}

// ---- the enumerable sweep --------------------------------------------------

/// What the enumerable sweep found — a TRI-state, because "no answer" and "not
/// there" are different facts and only one of them may authorise a deletion.
enum Sweep {
    /// The name is LIVE on this enumerable server.
    Found(String),
    /// This socket exists but did not answer, so the target could be hiding
    /// behind it.
    Unsure(String),
    /// Every enumerable socket answered, and none of them has the name.
    Clear,
}

/// Ask every enumerable tmux socket whether `name` is live there.
fn sweep(root: &Path, name: &str) -> Sweep {
    let Some(dir) = socket_dir(root) else {
        // Nowhere to enumerate is not proof of absence, but it is also not a
        // sighting; the acknowledgement stands on its own, as it does for a
        // server ae cannot enumerate at all.
        return Sweep::Clear;
    };
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the frozen _end_sweep_servers enumerates the tmux socket directory before an --assume-stopped deletion"
    )]
    let entries = std::fs::read_dir(&dir);
    let Ok(entries) = entries else {
        return Sweep::Clear;
    };
    let mut unsure = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !crate::tmux::is_addressable_socket(&path) {
            continue;
        }
        let label = entry.file_name().to_string_lossy().into_owned();
        let server = ServerId::Selected(Selector::Socket(path));
        match transport::verify_session_absent(&server, name) {
            StopProbe::Present => return Sweep::Found(label),
            StopProbe::Absent => {}
            StopProbe::Unknown => {
                if unsure.is_none() {
                    unsure = Some(label);
                }
            }
        }
    }
    unsure.map_or(Sweep::Clear, Sweep::Unsure)
}

/// `${TMUX_TMPDIR:-/tmp}/tmux-<uid>` — where tmux keeps its default sockets.
fn socket_dir(root: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt as _;
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: TMUX_TMPDIR is where tmux itself looks, and the state root's owner is the uid tmux names its socket directory after"
    )]
    let base = std::env::var_os("TMUX_TMPDIR")
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: see above — the uid is read from the state root rather than from libc"
    )]
    let owner = std::fs::metadata(root).ok()?;
    Some(base.join(format!("tmux-{}", owner.uid())))
}

#[cfg(test)]
mod tests {
    use super::{
        Action, ConfirmedPlan, Plan, end_prompt, parse, path_exists, purge_conversation_files,
        sanitize_branch_name, supervisor_args,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn the_in_pane_end_prompt_names_the_session_and_the_consequence() {
        let keep = vec![(
            "inside".to_owned(),
            Plan {
                action: Action::Keep,
                detail: "/archive/id".to_owned(),
                purge: false,
            },
        )];
        assert_eq!(
            end_prompt("inside", &keep),
            "End 'inside'? Archives, then deletes its state. (y/n)"
        );
        let purge = vec![(
            "inside".to_owned(),
            Plan {
                action: Action::Purge,
                detail: "/archive/id".to_owned(),
                purge: true,
            },
        )];
        assert_eq!(
            end_prompt("inside", &purge),
            "End 'inside'? Deletes its state and purges the agent history. (y/n)"
        );
    }

    #[test]
    fn the_supervisor_argv_carries_the_confirmed_plan_and_history_flag() {
        let args =
            parse(&["inside".to_owned(), "--assume-stopped".to_owned()]).expect("valid end args");
        let confirmed = ConfirmedPlan {
            name: "inside".to_owned(),
            plan: Plan {
                action: Action::Keep,
                detail: "/archive/id".to_owned(),
                purge: false,
            },
            from_default: true,
        };
        assert_eq!(
            supervisor_args("/core".to_owned(), &args, &confirmed).as_args(),
            [
                "/core",
                "_end",
                "--supervise",
                "inside",
                "--confirmed-target=inside",
                "--confirmed-action=keep",
                "--confirmed-detail=/archive/id",
                "--confirmed-purge=off",
                "--confirmed-source=default",
                "--keep-history",
                "--assume-stopped",
            ]
        );
    }

    /// `--purge-history` deletes the conversations ae can name by EXACT id, and
    /// nothing else. agy can be named exactly — one file per id — so it belongs
    /// in that set; it was silently skipped, which is a retention leak rather
    /// than a conservative default, because the operator asked for a purge.
    #[test]
    fn a_purge_removes_the_agy_conversation_and_its_sidecars_and_no_one_else_s() {
        let scratch = std::env::temp_dir().join(format!("ae-purge-agy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let home = scratch.join("home");
        // `purge_conversation_files` derives the tool homes from the state
        // root's PARENT, which is what makes a fake home possible at all.
        let root = home.join(".ae");
        let store = home.join(".gemini/antigravity-cli/conversations");
        std::fs::create_dir_all(&store).expect("a conversation store");
        std::fs::create_dir_all(&root).expect("a state root");

        let mine = "643393ad-eb92-4b9e-ab7a-0fe7b1221fa1";
        let sibling = "aaaaaaaa-1111-4111-8111-111111111111";
        let write = |path: &Path| std::fs::write(path, "x").expect("a fixture conversation");
        for suffix in ["db", "db-wal", "db-shm"] {
            write(&store.join(format!("{mine}.{suffix}")));
            write(&store.join(format!("{sibling}.{suffix}")));
        }
        // A seat whose id was never captured names nothing, so it must delete
        // nothing — the guard that stops a pending seat purging a stranger.
        let meta = format!(
            "session=x\nschema=2\nseat.main=lead\nagent_bin.main=agy\n\
             harness_session.main={mine}\nseat.worker.1=hand\nagent_bin.worker.1=agy\n\
             harness_session.worker.1=pending\n"
        );
        let mut out = Vec::new();
        let mut err = Vec::new();
        purge_conversation_files(&root, meta.as_bytes(), &mut out, &mut err)
            .expect("the purge writes its report");
        let said = String::from_utf8_lossy(&out).into_owned();

        for suffix in ["db", "db-wal", "db-shm"] {
            let gone = store.join(format!("{mine}.{suffix}"));
            assert!(
                !path_exists(&gone),
                "the seat's own {suffix} must be removed: {said}"
            );
            assert!(
                said.contains(&format!("removed agy conversation: {}", gone.display())),
                "and the removal must be reported: {said}"
            );
            assert!(
                path_exists(&store.join(format!("{sibling}.{suffix}"))),
                "another conversation's {suffix} must be untouched"
            );
        }
        assert!(
            !said.contains("pending"),
            "a seat with no captured id names nothing: {said}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// A recorded id is a NAME, not a path — and the purge is `remove_file`.
    ///
    /// `harness_session.<slot>` is hand-editable metadata that only ever gets
    /// screened for control bytes, so an absolute value makes `Path::join`
    /// discard the store and a dot-dot value walks out of it. Either one turns a
    /// roster row into a deletion target anywhere the user can write. The guard
    /// is at the row, so this covers claude and codex too.
    #[test]
    fn a_recorded_id_that_is_not_a_uuid_names_nothing_and_deletes_nothing() {
        let scratch = std::env::temp_dir().join(format!("ae-purge-esc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let home = scratch.join("home");
        let root = home.join(".ae");
        let store = home.join(".gemini/antigravity-cli/conversations");
        std::fs::create_dir_all(&store).expect("a conversation store");
        std::fs::create_dir_all(&root).expect("a state root");

        // Two bystanders OUTSIDE the store, each named by one of the escapes.
        let outside = scratch.join("outside");
        std::fs::create_dir_all(&outside).expect("a bystander dir");
        let absolute = outside.join("absolute");
        let dotdot = home.join(".gemini/antigravity-cli/escaped.db");
        std::fs::write(&absolute, "keep me").expect("a bystander");
        std::fs::write(&dotdot, "keep me").expect("a bystander");

        for (tool, id) in [
            ("agy", absolute.display().to_string()),
            ("agy", "../escaped".to_owned()),
            ("claude", absolute.display().to_string()),
            ("codex", "../escaped".to_owned()),
            // Shapes that are close to a UUID but are not one.
            ("agy", "643393ad-eb92-4b9e-ab7a".to_owned()),
            (
                "agy",
                "643393ad-eb92-4b9e-ab7a-0fe7b1221fa1/../../x".to_owned(),
            ),
        ] {
            let meta = format!(
                "session=x\nschema=2\nseat.main=lead\nagent_bin.main={tool}\n\
                 harness_session.main={id}\n"
            );
            let mut out = Vec::new();
            let mut err = Vec::new();
            purge_conversation_files(&root, meta.as_bytes(), &mut out, &mut err)
                .expect("the purge reports rather than fails");
            let said = String::from_utf8_lossy(&out).into_owned();
            let noted = String::from_utf8_lossy(&err).into_owned();
            assert!(
                said.is_empty(),
                "nothing may be reported removed for {tool} id {id}: {said}"
            );
            assert!(
                noted.contains("is not a UUID"),
                "and the loss must be reported for {tool} id {id}: {noted}"
            );
            assert!(
                path_exists(&absolute) && path_exists(&dotdot),
                "a file outside the store was removed via {tool} id {id}"
            );
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// The paths are computed, never searched, and only existing ones are named.
    #[test]
    fn the_agy_purge_names_only_files_that_are_there() {
        let scratch = std::env::temp_dir().join(format!("ae-purge-agy2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let store = scratch.join(".gemini/antigravity-cli/conversations");
        std::fs::create_dir_all(&store).expect("a conversation store");
        let id = "643393ad-eb92-4b9e-ab7a-0fe7b1221fa1";
        assert_eq!(
            super::agy_conversation_files(&scratch, id),
            Vec::<PathBuf>::new(),
            "an id with nothing on disk names nothing"
        );
        std::fs::write(store.join(format!("{id}.db")), "x").expect("a conversation");
        assert_eq!(
            super::agy_conversation_files(&scratch, id),
            vec![store.join(format!("{id}.db"))],
            "a database with no sidecars is one file, not three"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn the_branch_name_is_the_frozen_sanitisation() {
        assert_eq!(sanitize_branch_name("proj"), "ae/proj");
        assert_eq!(sanitize_branch_name("ae-proj"), "ae/proj");
        assert_eq!(sanitize_branch_name("a/b c"), "ae/a-b-c");
        assert_eq!(sanitize_branch_name("--x--"), "ae/x");
        assert_eq!(
            sanitize_branch_name("keep.dots_and-dashes"),
            "ae/keep.dots_and-dashes"
        );
    }

    #[test]
    fn a_plan_line_states_both_actions() {
        let keep = Plan {
            action: Action::Keep,
            detail: "/a/archive/u".to_owned(),
            purge: false,
        };
        assert_eq!(
            keep.line("s"),
            "  - s: archive -> /a/archive/u/ · conversation files KEPT"
        );
        let purge = Plan {
            action: Action::Purge,
            detail: "/a/archive/u".to_owned(),
            purge: true,
        };
        assert_eq!(
            purge.line("s"),
            "  - s: NO archive, and /a/archive/u/ is DELETED if it exists · conversation files DELETED"
        );
    }
}
