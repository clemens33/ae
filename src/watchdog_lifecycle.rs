//! `watchdog start|stop|status` — the per-session daemon's LIFECYCLE.
//!
//! Slice A.3 gave the core the watchdog's whole body (`_watchdog-run`, see
//! [`crate::watchdog_daemon`]) and left bash managing it: `cmd_watchdog`
//! (ae:7498) resolved a session and shelled into the generated `watchdog`
//! helper, whose `_watchdog_start` / `_watchdog_stop` / `_watchdog_status`
//! (ae:11464-11588) owned the pane, the pidfile, the start lock and the meta
//! flag. This module is that management, ported.
//!
//! # What the pane runs, and where the knobs are
//!
//! `start` splits a pane running `<meta>/watchdog _run`, exactly as bash did —
//! and the helper is not a fallback: it is where the `AE_WATCHDOG_*` / legacy
//! `AE_LOOP_*` knob NAMES are spelled. The core may not read the environment,
//! so every tunable has to cross as a flag, and the process that can see an
//! operator's exports is the one tmux starts in the pane. Pointing that pane
//! straight at `ae-core _watchdog-run` would silently run a cadence nobody
//! chose.
//!
//! When a session has NO `watchdog` helper the pane runs the core directly, and
//! the words after `--` are appended to its argv: that is the channel a caller
//! who knows the knobs (the glue, which can read the environment) uses to carry
//! them through. No `--` tail means the daemon's own defaults, which is the
//! honest answer when nobody said otherwise.
//!
//! # Three states, not two
//!
//! Bash asked `kill -0` and `_monitor_find_pane`, and a tmux probe that FAILED
//! was indistinguishable from a pane that was absent — so an unanswerable
//! server read as "not running", which on `start` means spawning a second
//! daemon beside a live one. [`Presence`] keeps that third answer, and both
//! edges refuse to act on it (#28: only a VERIFIED absence may start a
//! companion, and only a verified presence may be killed).
//!
//! # The pane is the kill
//!
//! `stop` does not signal the recorded pid. Since A.3 the daemon IS the pane's
//! process, so the ownership-checked `kill-pane`
//! ([`watchdog_glue::kill_owned_pane`]) takes it with the pane — and a bare kill
//! of a recorded pid is the stranger-kill that guard exists to refuse. The
//! pidfile is then retracted only while it still names the pid we observed, and
//! the bar options are cleared through the daemon's own
//! [`watchdog_daemon::clear_published`], because a pane killed by SIGHUP does
//! not get to run its own cleanup.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use crate::inventory::ServerId;
use crate::session_tmux::{Op, Split, argv, interpret_pane_id};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::{lifecycle, meta, session_launch, tmux, transport, watchdog_daemon, watchdog_glue};

/// The frozen usage, both lines (ae:7501-7502), plus the knob passthrough.
pub const USAGE: &str = "Usage: ae watchdog <start|stop|status> [session-name] [--pane <id>] [-- <knob flags>]\n  (If run inside an ae session, session-name is optional.)";

/// The `@ae_agent` stamp the watchdog's pane carries.
const AGENT_STAMP: &str = "_watchdog";

/// The generated helper the pane runs, and its internal subcommand.
const HELPER: &str = "watchdog";

/// The pane title, so the monitor window's border names it (ae:11552).
const PANE_TITLE: &str = "ae watchdog";

/// The start lock's name under the session's meta dir — the frozen
/// `.watchdog.start.lock` (ae:11536).
const START_LOCK: &str = ".watchdog.start.lock";

/// How long a starter blocks on the start lock before DEFERRING — the frozen
/// `AE_WATCHDOG_START_LOCK_WAIT_SEC` default.
const START_LOCK_WAIT: Duration = Duration::from_secs(15);

/// How many polls the registration wait takes, and the pause between them —
/// the frozen `AE_WATCHDOG_START_REGISTER_TRIES` default of 200 × `sleep 0.05`.
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
    /// The server could not be asked. NOT absence — see the module docs.
    Unknown,
}

/// `watchdog <start|stop|status> [session] [--pane <id>]`.
///
/// `--pane` is how "inside an ae session" arrives: the caller's own pane id,
/// which the core resolves to a session itself rather than trusting a name it
/// was told. Without it, and without a name, there is nothing to act on.
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
    if target.is_empty() {
        // The caller's own pane names its session. A probe that fails leaves the
        // target empty, which is the same refusal as never having been inside
        // one — better than guessing at a session to act on.
        if !pane.is_empty()
            && let Some(owner) = transport::observe_pane_owner(&ServerId::Ambient, &pane)
        {
            target = owner.session;
        }
    }
    if target.is_empty() {
        writeln!(
            err,
            "Error: no session name given and not inside an ae tmux session"
        )?;
        return Ok(EXIT_FAILED);
    }
    // The name is about to become a directory under the sessions root, so it is
    // checked against the grammar (or the legacy already-a-directory arm) before
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
        Action::Start => start(&server, &target, &meta_dir, &knobs, out, err),
        Action::Stop => stop(&server, &target, &meta_dir, out, err),
    }
}

/// `watchdog status` — the frozen two lines, plus the honest third.
fn status(
    server: &ServerId,
    session: &str,
    meta_dir: &Path,
    out: &mut impl Write,
) -> crate::Result<u8> {
    match presence(server, session, meta_dir) {
        Presence::Running(pid) => writeln!(out, "Watchdog is running (pid {pid}).")?,
        Presence::Stopped => writeln!(out, "Watchdog is not running.")?,
        // Bash could not say this: an unanswerable server read as absence and
        // `status` reported "not running" about a session it had not seen.
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
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    // A pre-rename watchdog can outlive a `doctor --refresh` (helpers are
    // rewritten, a running daemon is not restarted), so it is reaped before the
    // check-and-spawn — never two watchdogs side by side.
    watchdog_glue::reap_legacy(server, session, meta_dir, err)?;
    // SINGLE-STARTER MUTUAL EXCLUSION. The launch's autostart and an explicit
    // `ae watchdog start` reach here, and without the lock both read "not
    // running" in the window before either publishes a pidfile and BOTH spawn a
    // daemon. Failing to take it is not licence to proceed: the holder is
    // establishing the one daemon, so we DEFER. The handle is held for the rest
    // of the function — the spawn and the registration wait included — because
    // the pidfile appears asynchronously and releasing at pane-creation would
    // hand the next starter the same empty window.
    let held = crate::state::acquire(&meta_dir.join(START_LOCK), START_LOCK_WAIT);
    let Ok(_held) = held else {
        writeln!(
            err,
            "Watchdog start deferred: could not acquire the start lock (another start in progress)."
        )?;
        return Ok(0);
    };
    match presence(server, session, meta_dir) {
        Presence::Running(pid) => {
            // Idempotent: confirm the meta flag and leave the live daemon's
            // status-right indicator alone.
            writeln!(out, "Watchdog is already running (pid {pid}).")?;
            let _ = meta::rewrite(meta_dir, "watchdog", Some("true"));
            return Ok(0);
        }
        Presence::Unknown => {
            writeln!(
                err,
                "Watchdog start skipped — tmux did not answer, so a running watchdog cannot be ruled out."
            )?;
            return Ok(0);
        }
        Presence::Stopped => {}
    }
    // The ae-monitor window with the `_events` pane must exist first: the
    // watchdog pane is split ABOVE it, so the visual order stays
    // watchdog-on-top / events-below.
    let Some(anchor) = session_launch::ensure_events_pane(server, session, meta_dir) else {
        writeln!(err, "Error: could not create watchdog pane")?;
        return Ok(EXIT_FAILED);
    };
    let Some(command) = daemon_command(meta_dir, knobs) else {
        writeln!(
            err,
            "Error: no watchdog helper in the session and this process cannot name itself; nothing to run."
        )?;
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
        return Ok(EXIT_FAILED);
    };
    // The stamp goes on BEFORE the daemon publishes its pidfile, so whenever a
    // pidfile exists its pane is already findable — which is what makes
    // [`Presence`]'s pane requirement safe against a starting watchdog.
    let _ = transport::publish_option(
        server,
        tmux::OptionScope::Pane,
        &pane,
        "@ae_agent",
        AGENT_STAMP,
    );
    let _ = transport::set_pane_title(server, &pane, PANE_TITLE);
    let _ = transport::run_tmux_op(&argv(server, &Op::DisablePane { pane: &pane }));
    // REGISTRATION, by the same criteria the NEXT starter will apply. The read
    // is non-destructive — a partial file is never deleted here, because the
    // daemon that is mid-publish is the one we are waiting for.
    let mut registered = false;
    for _ in 0..REGISTER_TRIES {
        if matches!(observe(server, session, meta_dir), Presence::Running(_)) {
            registered = true;
            break;
        }
        std::thread::sleep(REGISTER_POLL);
    }
    if !registered {
        // Nothing live-but-unregistered may be left for the next starter to
        // duplicate, and the lock is still held while we tear our own pane down.
        watchdog_glue::kill_owned_pane(server, &pane, session, Some(AGENT_STAMP), err)?;
        writeln!(
            err,
            "Error: watchdog did not publish a pidfile within the start bound; start aborted."
        )?;
        return Ok(EXIT_FAILED);
    }
    let _ = meta::rewrite(meta_dir, "watchdog", Some("true"));
    writeln!(
        out,
        "Watchdog started in hidden ae-monitor window. Use 'peek _watchdog' or 'peek _events' to inspect."
    )?;
    Ok(0)
}

/// `watchdog stop` — reap, kill the pane, retract the registration and the bar.
fn stop(
    server: &ServerId,
    session: &str,
    meta_dir: &Path,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let legacy = watchdog_glue::reap_legacy(server, session, meta_dir, err)?;
    let mut stopped = false;
    match presence(server, session, meta_dir) {
        Presence::Running(pid) => {
            // The daemon is the pane's process: killing the pane kills it.
            // Presence proved the pane was there a moment ago; if it is gone now
            // the daemon went with it and there is nothing left to kill.
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
            return Ok(EXIT_FAILED);
        }
        Presence::Stopped => {}
    }
    if stopped || !legacy.is_empty() {
        writeln!(out, "Watchdog stopped.")?;
    } else {
        writeln!(out, "Watchdog is not running.")?;
    }
    let _ = meta::rewrite(meta_dir, "watchdog", Some("false"));
    Ok(0)
}

/// The argv the watchdog pane runs.
///
/// The session's own `watchdog` helper when it has one — the frozen shape, and
/// the only process in this path that can read an operator's `AE_WATCHDOG_*`
/// exports. Otherwise the core directly, with whatever knob flags the caller
/// passed after `--`. `None` when there is no helper AND this process cannot
/// name its own executable: there is then nothing to start, and a `PATH` lookup
/// could resolve to a different core than the one holding the contract.
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
    let core = std::env::current_exe().ok()?;
    let mut command = vec![
        core.display().to_string(),
        crate::cli::WATCHDOG_RUN.to_owned(),
        meta_dir.display().to_string(),
    ];
    command.extend(knobs.iter().cloned());
    Some(command)
}

/// [`observe`], plus the frozen stale-pidfile cleanup.
///
/// A pidfile naming a dead process, or one whose pane is verifiably gone, is an
/// artifact rather than a registration: it is removed so `status` cannot lie,
/// `start` does not wrongly defer, and `stop` does not report a daemon that is
/// not there. Removal is ownership-checked, so a replacement that has already
/// published its own pid is never touched.
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
///
/// The registration wait uses this rather than [`presence`]: the daemon it is
/// waiting for may be mid-publish, and a reader that deletes what it finds
/// half-written is the duplicate-spawn hazard the atomic publish exists to
/// close.
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
    // A pid `ps` did not list is gone. A `ps` that did not answer is NOT: the
    // pane and the pidfile are then the evidence we have, and they both say
    // running — failure is UNKNOWN, never dead, which is `procs`' own rule.
    if pid_alive(pid) == Some(false) {
        Presence::Stopped
    } else {
        Presence::Running(pid)
    }
}

/// Whether `pid` is in the process table — `None` when there is no table.
///
/// The `kill -0` of the frozen predicate, without a signal: this crate forbids
/// `unsafe`, so liveness is read from the snapshot [`crate::procs`] already
/// takes for the watchdog's own dead-checks. Failure is UNKNOWN, never dead —
/// the same direction that module documents.
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
mod tests {
    use super::{Action, Presence, USAGE};

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
}
