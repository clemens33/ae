//! `watchdog start|stop|status` — the per-session daemon's LIFECYCLE.
//!
//! The daemon's body is [`crate::watchdog_daemon`] (`_watchdog-run`); this
//! module resolves a session and owns the pane, the pidfile, the start lock and
//! the meta flag around it.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use crate::inventory::ServerId;
use crate::session_tmux::{Op, Split, argv, interpret_pane_id};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::{lifecycle, meta, session_launch, tmux, transport, watchdog_daemon, watchdog_glue};

/// The usage, both lines, plus the knob passthrough.
pub const USAGE: &str = "Usage: ae watchdog <start|stop|status> [session-name] [--pane <id>] [-- <knob flags>]\n  (If run inside an ae session, session-name is optional.)";

/// The `@ae_agent` stamp the watchdog's pane carries.
const AGENT_STAMP: &str = "_watchdog";

/// The generated helper the pane runs, and its internal subcommand.
const HELPER: &str = "watchdog";

/// The pane title, so the monitor window's border names it.
const PANE_TITLE: &str = "ae watchdog";

/// The start lock's name under the session's meta dir.
const START_LOCK: &str = ".watchdog.start.lock";

/// The event actions a start/stop records, whatever its outcome.
const START_ACTION: &str = "watchdog-start";
const STOP_ACTION: &str = "watchdog-stop";

/// How long a starter blocks on the start lock before DEFERRING.
const START_LOCK_WAIT: Duration = Duration::from_secs(15);

/// How many polls the registration wait takes, and the pause between them.
const REGISTER_TRIES: u32 = 200;
const REGISTER_POLL: Duration = Duration::from_millis(50);

/// What the argv asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Start,
    Stop,
    Status,
}

impl Action {
    /// The action a word names, or `None` when it names none.
    fn parse(word: &str) -> Option<Self> {
        match word {
            "start" => Some(Self::Start),
            "stop" => Some(Self::Stop),
            "status" => Some(Self::Status),
            _ => None,
        }
    }
}

/// Whether a session's watchdog is running — and the third answer, which is the
/// point of the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// A published pidfile, a live process, and this session's own tagged pane.
    Running(u32),
    /// Verified absent: no pidfile, or one whose daemon is provably gone.
    Stopped,
    /// The server could not be asked.
    Unknown,
}

/// `watchdog <start|stop|status> [session] [--pane <id>]`.
///
/// # Errors
///
/// Only writing to `out`/`err`; every refusal is an exit code, not an `Err`.
pub fn run(
    root: &Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let mut action = None;
    let mut target = String::new();
    let mut pane = String::new();
    let mut knobs: Vec<String> = Vec::new();
    let mut rest = tail;
    while let [word, after @ ..] = rest {
        rest = after;
        if word == "--" {
            // Everything after `--` is the daemon's, not ours: a knob flag this
            // module does not model and must not validate.
            knobs = rest.to_vec();
            break;
        }
        if word == "--pane" {
            let Some((value, tail)) = rest.split_first() else {
                writeln!(err, "{USAGE}")?;
                return Ok(EXIT_USAGE);
            };
            value.clone_into(&mut pane);
            rest = tail;
            continue;
        }
        if action.is_none() {
            let Some(parsed) = Action::parse(word) else {
                writeln!(err, "{USAGE}")?;
                return Ok(EXIT_USAGE);
            };
            action = Some(parsed);
            continue;
        }
        if target.is_empty() {
            word.clone_into(&mut target);
            continue;
        }
        writeln!(
            err,
            "Error: unexpected extra argument '{word}' — ae watchdog takes one session name."
        )?;
        return Ok(EXIT_USAGE);
    }
    let Some(action) = action else {
        writeln!(err, "{USAGE}")?;
        return Ok(EXIT_USAGE);
    };
    let caller_server = crate::doors::caller_server();
    if target.is_empty()
        && let Some(own) = lifecycle::recorded_caller_session(root, caller_server.as_ref(), &pane)
    {
        target = own;
    }
    if target.is_empty() {
        writeln!(
            err,
            "Error: no session name given and not inside an ae tmux session"
        )?;
        return Ok(EXIT_FAILED);
    }
    // The name is about to become a directory under the sessions root, so it is
    // checked against the grammar (or the already-a-directory arm) before
    // it is joined to anything.
    if !lifecycle::name_is_usable(root, &target) {
        writeln!(err, "Error: session '{target}' not found")?;
        return Ok(EXIT_FAILED);
    }
    let meta_dir = lifecycle::sessions_dir(root).join(&target);
    if !lifecycle::dir_exists(&meta_dir) {
        writeln!(err, "Error: session '{target}' not found")?;
        return Ok(EXIT_FAILED);
    }
    // FAIL CLOSED on a record that does not name one server: start, stop and
    // status all address the session BY NAME on this server, and the ambient
    // fallback would aim every one of them at whatever else answers to it.
    let Some(server) = session_launch::recorded_server_resolved(&meta_dir) else {
        writeln!(
            err,
            "Error: session '{target}' {}. The watchdog was not touched.",
            session_launch::AMBIGUOUS_SERVER
        )?;
        return Ok(EXIT_FAILED);
    };
    match action {
        Action::Status => status(&server, &target, &meta_dir, out),
        Action::Start => {
            let caller = Caller::of(START_ACTION, caller_server.as_ref(), &pane);
            start(&server, &target, &meta_dir, &knobs, &caller, out, err)
        }
        Action::Stop => {
            let caller = Caller::of(STOP_ACTION, caller_server.as_ref(), &pane);
            stop(root, &server, &target, &meta_dir, &caller, out, err)
        }
    }
}

/// Who asked — the facts every audit record carries.
struct Caller<'a> {
    action: &'static str,
    actor: String,
    pane: &'a str,
}

impl<'a> Caller<'a> {
    fn of(action: &'static str, caller_server: Option<&ServerId>, pane: &'a str) -> Self {
        Self {
            action,
            actor: actor_of(caller_server, pane),
            pane,
        }
    }

    /// One audit record for a start/stop that REACHED its decision, through the
    /// one events writer. NO `target` (currency rule: this must not un-quiet
    /// anyone); an append failure warns and never fails the command.
    fn audit(&self, meta_dir: &Path, outcome: &str, err: &mut impl Write) {
        let summary = if self.pane.is_empty() {
            outcome.to_owned()
        } else {
            format!("{outcome} (pane {})", self.pane)
        };
        let now = crate::time::Timestamp::now();
        let (actor, action) = (self.actor.as_str(), self.action);
        let line = crate::tracked::event_line(&crate::tracked::EventFields::new(
            now, actor, action, "", "", "", "", "", "", &summary, "",
        ));
        if let Err(why) = crate::store::open(meta_dir).append_event(&line) {
            let _ = writeln!(err, "ae: watchdog audit failed: {why}");
        }
    }
}

/// The event's actor: the calling pane's ae stamp, or the human spelling
/// every other audit uses when no agent called.
fn actor_of(caller_server: Option<&ServerId>, pane: &str) -> String {
    if pane.is_empty() {
        return "human".to_owned();
    }
    let agent = caller_server
        .and_then(|server| transport::observe_pane_owner(server, pane))
        .map(|owner| owner.agent)
        .unwrap_or_default();
    if agent.is_empty() {
        "human".to_owned()
    } else {
        agent
    }
}

/// `watchdog status` — running, stopped, or the honest unknown.
fn status(
    server: &ServerId,
    session: &str,
    meta_dir: &Path,
    out: &mut impl Write,
) -> crate::Result<u8> {
    match presence(server, session, meta_dir) {
        Presence::Running(pid) => writeln!(out, "Watchdog is running (pid {pid}).")?,
        Presence::Stopped => writeln!(out, "Watchdog is not running.")?,
        // An unanswerable server is NOT absence: reporting "not running" about
        // a session tmux never spoke for would be a lie.
        Presence::Unknown => writeln!(
            out,
            "Watchdog state unknown — tmux did not answer, so a running watchdog cannot be ruled out."
        )?,
    }
    Ok(0)
}

/// `watchdog start` — reap, serialize, spawn the pane, wait for registration.
fn start(
    server: &ServerId,
    session: &str,
    meta_dir: &Path,
    knobs: &[String],
    caller: &Caller,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    // A pre-rename watchdog can outlive a `doctor --refresh` (helpers are
    // rewritten, a running daemon is not restarted), so it is reaped before the
    // check-and-spawn — never two watchdogs side by side.
    watchdog_glue::reap_legacy(server, session, meta_dir, err)?;
    // SINGLE-STARTER MUTUAL EXCLUSION.
    let held = crate::store::lock(&meta_dir.join(START_LOCK), START_LOCK_WAIT);
    let Ok(_held) = held else {
        writeln!(
            err,
            "Watchdog start deferred: could not acquire the start lock (another start in progress)."
        )?;
        let outcome = "refused: could not acquire the start lock (another start in progress)";
        caller.audit(meta_dir, outcome, err);
        return Ok(0);
    };
    match presence(server, session, meta_dir) {
        Presence::Running(pid) => {
            // Idempotent: confirm the meta flag and leave the live daemon's
            // status-right indicator alone.
            writeln!(out, "Watchdog is already running (pid {pid}).")?;
            let outcome = format!("already running (pid {pid})");
            caller.audit(meta_dir, &outcome, err);
            let _ = meta::rewrite(meta_dir, "watchdog", Some("true"));
            return Ok(0);
        }
        Presence::Unknown => {
            writeln!(
                err,
                "Watchdog start skipped — tmux did not answer, so a running watchdog cannot be ruled out."
            )?;
            let outcome = "refused: tmux did not answer";
            caller.audit(meta_dir, outcome, err);
            return Ok(0);
        }
        Presence::Stopped => {}
    }
    // The ae-monitor window with the `_events` pane must exist first: the
    // watchdog pane is split ABOVE it, so the visual order stays
    // watchdog-on-top / events-below.
    let Some(anchor) = session_launch::ensure_events_pane(server, session, meta_dir) else {
        writeln!(err, "Error: could not create watchdog pane")?;
        let outcome = "refused: could not create watchdog pane";
        caller.audit(meta_dir, outcome, err);
        return Ok(EXIT_FAILED);
    };
    let Some(command) = daemon_command(meta_dir, knobs) else {
        writeln!(
            err,
            "Error: no watchdog helper in the session and this process cannot name itself; nothing to run."
        )?;
        let outcome = "refused: no watchdog helper and the core cannot name itself";
        caller.audit(meta_dir, outcome, err);
        return Ok(EXIT_FAILED);
    };
    let (succeeded, stdout) = transport::run_tmux_op(&argv(
        server,
        &Op::SplitWindow {
            target: &anchor,
            work_dir: "",
            split: Split::VerticalBefore,
            command: &command,
        },
    ));
    let Some(pane) = interpret_pane_id(succeeded, &stdout) else {
        writeln!(err, "Error: could not create watchdog pane")?;
        let outcome = "refused: could not create watchdog pane";
        caller.audit(meta_dir, outcome, err);
        return Ok(EXIT_FAILED);
    };
    // The stamp goes on BEFORE the daemon publishes its pidfile, so whenever a
    // pidfile exists its pane is already findable — which is what makes
    // [`Presence`]'s pane requirement safe against a starting watchdog.
    for (option, value) in [
        ("@ae_agent", AGENT_STAMP.to_owned()),
        (
            crate::theme::AGENT_LABEL_OPTION,
            crate::theme::agent_label(AGENT_STAMP),
        ),
    ] {
        let _ = transport::publish_option(server, tmux::OptionScope::Pane, &pane, option, &value);
    }
    let _ = transport::set_pane_title(server, &pane, PANE_TITLE);
    let _ = transport::run_tmux_op(&argv(server, &Op::DisablePane { pane: &pane }));
    // REGISTRATION, by the same criteria the NEXT starter will apply.
    if await_running(server, session, meta_dir).is_none() {
        // Nothing live-but-unregistered may be left for the next starter to
        // duplicate, and the lock is still held while we tear our own pane down.
        watchdog_glue::kill_owned_pane(server, &pane, session, Some(AGENT_STAMP), err)?;
        writeln!(
            err,
            "Error: watchdog did not publish a pidfile within the start bound; start aborted."
        )?;
        let outcome = "refused: watchdog did not publish a pidfile within the start bound";
        caller.audit(meta_dir, outcome, err);
        return Ok(EXIT_FAILED);
    }
    let _ = meta::rewrite(meta_dir, "watchdog", Some("true"));
    writeln!(
        out,
        "Watchdog started in hidden ae-monitor window. Use 'peek _watchdog' or 'peek _events' to inspect."
    )?;
    caller.audit(meta_dir, "started", err);
    Ok(0)
}

/// Wait for the watchdog pane to publish a pidfile that names its live
/// process. Rename uses the same registration proof as a normal start.
pub(crate) fn await_running(server: &ServerId, session: &str, meta_dir: &Path) -> Option<u32> {
    for _ in 0..REGISTER_TRIES {
        if let Presence::Running(pid) = observe(server, session, meta_dir) {
            return Some(pid);
        }
        std::thread::sleep(REGISTER_POLL);
    }
    None
}

/// `watchdog stop` — reap, kill the pane, retract the registration and the bar.
fn stop(
    root: &Path,
    server: &ServerId,
    session: &str,
    meta_dir: &Path,
    caller: &Caller,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let legacy = watchdog_glue::reap_legacy(server, session, meta_dir, err)?;
    let mut stopped = false;
    match presence(server, session, meta_dir) {
        Presence::Running(pid) => {
            // The daemon is the pane's process: killing the pane kills it.
            if let PaneLook::Present(pane) = stamped_pane(server, session, AGENT_STAMP) {
                watchdog_glue::kill_owned_pane(server, &pane, session, Some(AGENT_STAMP), err)?;
            }
            let _ = watchdog_glue::clear_pid(meta_dir, pid);
            // A pane killed by SIGHUP does not run the daemon's own cleanup, so
            // the bars are retracted here — otherwise a stopped watchdog keeps
            // asserting a health it is no longer measuring.
            let _ = watchdog_daemon::clear_published(server, session);
            stopped = true;
        }
        Presence::Unknown => {
            writeln!(
                err,
                "Error: tmux did not answer — the watchdog could not be stopped and may still be running."
            )?;
            let outcome = "refused: tmux did not answer";
            caller.audit(meta_dir, outcome, err);
            return Ok(EXIT_FAILED);
        }
        Presence::Stopped => {}
    }
    // Past the match, so it covers BOTH arms that get here: the daemon this
    // stop killed, and one that had already died and left its last verdict
    // standing. Either way nothing measures this session now, and the session
    // itself is still running — it keeps the launch seed's Stale rank so every
    // fleet strip and the picker still list it, and its own bar says why. The
    // unanswerable server returns above rather than reaching this: a watchdog
    // that may still be running must not be described as absent.
    let _ = watchdog_daemon::seed_unwatched(server, session, root);
    let outcome = if stopped || !legacy.is_empty() {
        writeln!(out, "Watchdog stopped.")?;
        "stopped"
    } else {
        writeln!(out, "Watchdog is not running.")?;
        "not running"
    };
    caller.audit(meta_dir, outcome, err);
    let _ = meta::rewrite(meta_dir, "watchdog", Some("false"));
    Ok(0)
}

/// The argv the watchdog pane runs.
fn daemon_command(meta_dir: &Path, knobs: &[String]) -> Option<Vec<String>> {
    let helper = meta_dir.join(HELPER);
    if lifecycle::path_exists(&helper) {
        // The core-written shim is `exec <core> _watchdog-run <meta> "$@"`: the
        // entry and the meta dir are already inside it, so ONLY the knobs
        // follow — an extra word arrives as an unknown argument and the daemon
        // dies before it publishes a pidfile (measured, glue cut 3).
        let mut command = vec![helper.display().to_string()];
        command.extend(knobs.iter().cloned());
        return Some(command);
    }
    // RESOLVED, never raw: this is the daemon's own command word, and an
    // unresolved macOS answer would name whichever link started it.
    let core = crate::shape::resolved_exe()?;
    let mut command = vec![
        core.display().to_string(),
        crate::cli::WATCHDOG_RUN.to_owned(),
        meta_dir.display().to_string(),
    ];
    command.extend(knobs.iter().cloned());
    Some(command)
}

/// [`observe`], plus the stale-pidfile cleanup.
#[must_use]
pub fn presence(server: &ServerId, session: &str, meta_dir: &Path) -> Presence {
    let seen = observe(server, session, meta_dir);
    if seen == Presence::Stopped
        && let Some(pid) = watchdog_glue::read_pid(meta_dir)
    {
        let _ = watchdog_glue::clear_pid(meta_dir, pid);
    }
    seen
}

/// Read the three facts a registration rests on, and change NOTHING.
#[must_use]
pub fn observe(server: &ServerId, session: &str, meta_dir: &Path) -> Presence {
    let Some(pid) = watchdog_glue::read_pid(meta_dir) else {
        return Presence::Stopped;
    };
    // A stale pidfile can name a RECYCLED, unrelated process, so the pid alone
    // is never the answer: this session's own tagged pane must be live too.
    match stamped_pane(server, session, AGENT_STAMP) {
        PaneLook::Present(_) => {}
        PaneLook::Absent => return Presence::Stopped,
        PaneLook::Unanswered => return Presence::Unknown,
    }
    // A pid `ps` did not list is gone.
    if pid_alive(pid) == Some(false) {
        Presence::Stopped
    } else {
        Presence::Running(pid)
    }
}

/// Whether `pid` is in the process table — `None` when there is no table.
fn pid_alive(pid: u32) -> Option<bool> {
    let table = crate::procs::snapshot()?;
    Some(table.iter().any(|proc| proc.pid == pid))
}

/// What one look for a stamped pane found.
enum PaneLook {
    /// The pane is there, and this is its id.
    Present(String),
    /// The server answered, and no pane carries the stamp.
    Absent,
    /// The server did not answer at all — not the same thing as `Absent`.
    Unanswered,
}

/// Look for the pane of `session` stamped `agent`.
fn stamped_pane(server: &ServerId, session: &str, agent: &str) -> PaneLook {
    let Some(observed) = transport::observe_agents(server, session) else {
        return PaneLook::Unanswered;
    };
    observed
        .into_iter()
        .find(|pane| pane.agent == agent)
        .map_or(PaneLook::Absent, |pane| PaneLook::Present(pane.pane))
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests read back what the door wrote; the boundary is on product code — see clippy.toml"
)]
mod tests {
    use super::{Action, Caller, Presence, START_ACTION, USAGE, actor_of};

    #[test]
    fn the_three_subcommands_are_the_whole_grammar() {
        assert_eq!(Action::parse("start"), Some(Action::Start));
        assert_eq!(Action::parse("stop"), Some(Action::Stop));
        assert_eq!(Action::parse("status"), Some(Action::Status));
        // No abbreviation, no `restart`, and no `_run`: the pane's internal
        // subcommand is the helper's, never a word a human may type here.
        for word in ["", "sta", "restart", "_run", "_ensure-monitor", "START"] {
            assert_eq!(Action::parse(word), None, "'{word}' is not an action");
        }
    }

    #[test]
    fn the_usage_names_both_the_actions_and_the_optional_name() {
        assert!(USAGE.contains("start|stop|status"));
        assert!(USAGE.contains("[session-name]"));
        assert!(USAGE.contains("session-name is optional"));
    }

    #[test]
    fn unknown_is_neither_running_nor_stopped() {
        // The type's whole reason for existing: an unanswerable probe must not
        // compare equal to a verified absence, because `start` acts on absence.
        assert_ne!(Presence::Unknown, Presence::Stopped);
        assert_ne!(Presence::Unknown, Presence::Running(1));
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(format!("/tmp/ae-wdlife-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch meta dir");
        dir
    }

    #[test]
    fn an_unstamped_caller_is_the_human() {
        assert_eq!(actor_of(None, ""), "human");
        // A pane with no server to ask is unprovable, never an agent.
        assert_eq!(actor_of(None, "%9"), "human");
    }

    #[test]
    fn hostile_actor_bytes_cannot_escape_the_writers_json() {
        let dir = scratch("hostile");
        let caller = Caller {
            action: START_ACTION,
            actor: "a\"b\nc\x07".to_owned(),
            pane: "%1",
        };
        caller.audit(&dir, "refused: x\"y", &mut Vec::new());
        let body = std::fs::read_to_string(dir.join("events.jsonl")).expect("the audit line");
        assert_eq!(body.lines().count(), 1, "still one line: {body:?}");
        assert!(body.contains("(pane %1)"), "{body}");
        crate::events::Event::parse_line(body.trim_end()).expect("the hostile line parses");
    }
}
