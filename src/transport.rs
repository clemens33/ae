//! The exec: the one place this crate starts a child process.
//!
//! [`crate::tmux`] owns both halves that can be WRONG — which server an
//! argument list addresses, and what a completed run means — and both are
//! proven against real isolated servers. What is here is the part in between,
//! which is a detail rather than a decision: it hands a derived argument list to
//! `tmux`, waits, and hands the completed run back to be interpreted. It derives
//! no argv of its own and interprets no bytes of its own.

use crate::inventory::{DiscoveredSession, Discovery, QueryFailed, ServerId};
use crate::meta::Selector;
use crate::tmux;

/// The program ae runs to talk to a tmux server.
const PROGRAM: &str = "tmux";

/// The real tmux transport: [`Discovery`], answered by running `tmux`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Tmux;

impl Discovery for Tmux {
    /// Every session `server` reports, each with the ownership marker that
    /// server holds for it.
    fn enumerate(&self, server: &ServerId) -> Result<Vec<DiscoveredSession>, QueryFailed> {
        if !addressable(server) {
            return Err(QueryFailed);
        }
        let (succeeded, stdout) = run(PROGRAM, &tmux::list_sessions_args(server));
        let names = tmux::interpret_sessions(succeeded, &stdout)?;
        Ok(names
            .into_iter()
            .map(|name| {
                let (succeeded, stdout) = run(PROGRAM, &tmux::marker_args(server, &name));
                let marker = tmux::interpret_marker(succeeded, &stdout);
                DiscoveredSession { name, marker }
            })
            .collect())
    }
}

/// Whether `server` has a session `name` — the frozen resolver's `tmux
/// has-session -t` before a cross-session lookup (prefix-matched, as tmux
/// does).
#[must_use]
pub fn session_exists(server: &ServerId, name: &str) -> bool {
    addressable(server) && run(PROGRAM, &tmux::has_session_args(server, name)).0
}

/// Whether `name` is verifiably STOPPED on its recorded `server` — the frozen
/// `_end_verify_gone` tri-state the destructive compact gate crosses.
#[must_use]
pub fn verify_session_absent(server: &ServerId, name: &str) -> tmux::StopProbe {
    if !addressable(server) {
        return tmux::StopProbe::Unknown;
    }
    let (succeeded, stdout, stderr) = run_captured(PROGRAM, &tmux::list_sessions_args(server));
    tmux::interpret_stopped(succeeded, &stdout, &stderr, name)
}

/// The pane roster of `session` on `server`, or `None` when the enumeration
/// failed — see [`tmux::interpret_agents`].
#[must_use]
pub fn observe_agents(server: &ServerId, session: &str) -> Option<Vec<tmux::ObservedAgent>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::agents_args(server, session));
    tmux::interpret_agents(succeeded, &stdout)
}

/// The slot roster of `session` on the AMBIENT server — the frozen
/// `ae_slot_resolver`'s query — or `None` when the enumeration failed.
#[must_use]
pub fn observe_slots(server: &ServerId, session: &str) -> Option<Vec<tmux::ObservedSlot>> {
    let (succeeded, stdout) = run(PROGRAM, &tmux::slots_args(server, session));
    tmux::interpret_slots(succeeded, &stdout)
}

/// Every pane of `session` on `server`, or `None` when the enumeration failed.
#[must_use]
pub fn observe_panes(server: &ServerId, session: &str) -> Option<Vec<tmux::ObservedPane>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::list_panes_args(server, session));
    tmux::interpret_panes(succeeded, &stdout).ok()
}

/// The branch the watchdog last published for `session`, or `None`.
#[must_use]
pub fn observe_branch(server: &ServerId, session: &str) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let args = tmux::session_option_args(server, session, tmux::BRANCH_OPTION);
    let (succeeded, stdout) = run(PROGRAM, &args);
    tmux::interpret_session_option(succeeded, &stdout)
}

/// What running the frozen `send` helper produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// The helper's exit code; `None` when it could not be spawned at all, or
    /// died to a signal.
    pub code: Option<i32>,
    /// Its stdout — from `_send-deliver`, the stored body's path plus `\n`.
    pub stdout: String,
}

/// Run the session's send helper at `helper` with `target` and `message`,
/// plus `envs` (the event fields the frozen body store names the recovery
/// file after). stderr is INHERITED — the helper's refusals and
/// unconfirmed-submit lines are the caller's diagnostics, verbatim; stdin is
/// null.
#[must_use]
pub fn deliver(
    helper: &std::path::Path,
    target: &str,
    message: &str,
    envs: &[(&str, &str)],
) -> Delivery {
    let args = [target.to_owned(), message.to_owned()];
    match spawn(
        &helper.display().to_string(),
        &args,
        envs,
        Streams::InheritStderr,
        None,
    ) {
        Some(output) => Delivery {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        },
        None => Delivery {
            code: None,
            stdout: String::new(),
        },
    }
}

/// The calling pane's identity readings, from the AMBIENT server.
#[must_use]
pub fn observe_viewer(server: &ServerId, pane: &str) -> Option<tmux::ObservedViewer> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::viewer_args(server, pane));
    tmux::interpret_viewer(succeeded, &stdout)
}

/// Whether `server` is something this transport may put on the wire.
fn addressable(server: &ServerId) -> bool {
    match server {
        ServerId::Selected(Selector::Socket(path)) => tmux::is_addressable_socket(path),
        ServerId::Ambient | ServerId::Selected(Selector::Name(_)) => true,
    }
}

/// Run `program` with `args`; report whether it succeeded, and what it printed.
#[allow(
    clippy::disallowed_types,
    reason = "the product's door: ae cannot answer a liveness question without running tmux, nor deliver a tracked request without the session's send helper, nor derive an archive preview's git facts without running git"
)]
fn spawn<A: AsRef<std::ffi::OsStr>>(
    program: &str,
    args: &[A],
    envs: &[(&str, &str)],
    streams: Streams,
    feed: Option<&[u8]>,
) -> Option<std::process::Output> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    command.envs(envs.iter().copied());
    // B42, ported from the wrapper's pre-exec `unset`: `AE_VERSION` is the
    // TARGET PIN of `ae upgrade` and nothing else's input.
    command.env_remove("AE_VERSION");
    if streams == Streams::InheritStderr {
        command.stderr(std::process::Stdio::inherit());
    }
    if streams == Streams::Detached {
        command.stdin(std::process::Stdio::null());
        command.stdout(std::process::Stdio::null());
        command.stderr(std::process::Stdio::null());
        // The child is deliberately not waited for, so it is reaped by init
        // when it finishes.
        return command.spawn().ok().map(|_child| std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: Vec::new(),
            stderr: Vec::new(),
        });
    }
    if streams == Streams::Terminal {
        // Nothing is captured, so there is nothing to return but the status —
        // and an `Output` carrying it keeps every caller of this door reading
        // one shape.
        return command.status().ok().map(|status| std::process::Output {
            status,
            stdout: Vec::new(),
            stderr: Vec::new(),
        });
    }
    let Some(bytes) = feed else {
        return command.output().ok();
    };
    // THE BODY GOES IN ON STDIN, NOT IN ARGV.
    command.stdin(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());
    let mut child = command.spawn().ok()?;
    if let Some(mut sink) = child.stdin.take() {
        let _wrote = std::io::Write::write_all(&mut sink, bytes);
    }
    child.wait_with_output().ok()
}

/// How the door wires a child's streams — and therefore what it can report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Streams {
    /// Both captured.
    Captured,
    /// Stdout captured, stderr ae's own — the send helper, whose diagnostics
    /// belong in the pane that invoked it.
    InheritStderr,
    /// Every stream is ae's own.
    Terminal,
    /// Started and NOT waited for.
    Detached,
}

/// Run `program`, and report whether it succeeded and what it printed.
fn run(program: &str, args: &[String]) -> (bool, String) {
    match spawn(program, args, &[], Streams::Captured, None) {
        Some(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ),
        // Nothing ran.
        None => (false, String::new()),
    }
}

/// Like [`run`], but ALSO returns the child's stderr — the one caller that
/// needs it is the stop verification, which reads tmux's `no server running on
/// …` diagnostic to tell a clean server exit (proof the session is gone) from
/// any other failure (unproven).
fn run_captured(program: &str, args: &[String]) -> (bool, String, String) {
    match spawn(program, args, &[], Streams::Captured, None) {
        Some(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ),
        None => (false, String::new(), String::new()),
    }
}

/// The git leg of the one process door — the ONLY way product code runs `git`,
/// and the program is FIXED here so a caller chooses the arguments, never the
/// binary.
pub(crate) fn run_git(argv: &crate::git::GitArgv) -> (bool, String) {
    match spawn("git", argv.as_os_args(), &[], Streams::Captured, None) {
        Some(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ),
        None => (false, String::new()),
    }
}

/// The process-table snapshot leg of the one process door — the ONLY way
/// product code runs `ps`, the program FIXED here.
pub(crate) fn run_ps(argv: &crate::procs::PsArgv) -> (bool, String) {
    match spawn("ps", argv.as_args(), &[], Streams::Captured, None) {
        Some(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ),
        None => (false, String::new()),
    }
}

/// The `opencode` leg of the one process door — the ONLY way product code runs
/// `opencode`, the program FIXED here so a caller chooses nothing at all.
pub(crate) fn run_opencode(argv: &crate::session_launch::capture::OpenCodeArgv) -> (bool, String) {
    match spawn("opencode", argv.as_args(), &[], Streams::Captured, None) {
        Some(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ),
        None => (false, String::new()),
    }
}

/// The orchestrator-report leg of the one process door — the ONLY way product
/// code runs a session's `say` helper, and the program is not chosen here at
/// all: it arrives inside a [`crate::monitor::Notice`], whose fields are
/// private to `src/monitor.rs` and whose one constructor joins the literal
/// `say` onto a session directory.
pub(crate) fn run_say(notice: &crate::monitor::Notice) -> bool {
    spawn(
        &notice.helper().display().to_string(),
        &notice.args(),
        &[],
        Streams::InheritStderr,
        None,
    )
    .is_some_and(|output| output.status.success())
}

/// The detached-supervisor leg of the one process door — the ONLY way product
/// code starts a process that must OUTLIVE it, and the program is FIXED here to
/// `nohup`.
pub(crate) fn run_detached(argv: &crate::lifecycle::DetachedArgv) -> bool {
    spawn("nohup", argv.as_args(), &[], Streams::Detached, None).is_some()
}

/// The watchdog's pane roster of `session` on `server` — richer than
/// [`observe_agents`], carrying the pid and foreground command the cycle's
/// dead/stale checks read.
#[must_use]
pub fn observe_watch_panes(server: &ServerId, session: &str) -> Option<Vec<tmux::WatchPane>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::watch_panes_args(server, session));
    tmux::interpret_watch_panes(succeeded, &stdout)
}

/// Who a pane says it belongs to — `#{session_name}` and `@ae_agent`, read from
/// the PANE ITSELF at the moment a kill is being authorised.
#[must_use]
pub fn observe_pane_owner(
    server: &ServerId,
    pane: &str,
) -> Option<crate::watchdog_glue::PaneOwner> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(
        PROGRAM,
        &crate::watchdog_glue::pane_owner_args(server, pane),
    );
    crate::watchdog_glue::interpret_pane_owner(succeeded, &stdout)
}

/// Kill one pane by exact id.
#[must_use]
pub fn kill_pane(server: &ServerId, pane: &str) -> bool {
    if !addressable(server) {
        return false;
    }
    run(PROGRAM, &crate::watchdog_glue::kill_pane_args(server, pane)).0
}

/// Kill one whole session by EXACT id on `server`, the lifecycle kill behind
/// `ae stop`, `ae end` and `ae compact`.
#[must_use]
pub fn kill_session(server: &ServerId, session_id: &str) -> bool {
    if !addressable(server) {
        return false;
    }
    run(PROGRAM, &tmux::kill_session_args(server, session_id)).0
}

/// Create a spawned agent's own window and return its pane id.
#[must_use]
pub fn new_window(server: &ServerId, session: &str, work_dir: &str) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::new_window_args(server, session, work_dir));
    tmux::interpret_new_window(succeeded, &stdout)
}

/// Set a pane's title.
#[must_use]
pub fn set_pane_title(server: &ServerId, pane: &str, title: &str) -> bool {
    if !addressable(server) {
        return false;
    }
    run(PROGRAM, &tmux::pane_title_args(server, pane, title)).0
}

/// Rename the window `pane` lives in.
#[must_use]
pub fn rename_window(server: &ServerId, pane: &str, name: &str) -> bool {
    if !addressable(server) {
        return false;
    }
    run(PROGRAM, &tmux::rename_window_args(server, pane, name)).0
}

/// The last ~40 joined lines of `pane` on `server`, or `None` when the capture
/// failed or the server is non-addressable.
#[must_use]
pub fn capture_pane(server: &ServerId, pane: &str) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::capture_pane_args(server, pane));
    succeeded.then_some(stdout)
}

/// The id tmux holds for the session named exactly `name` on `server`.
#[must_use]
pub fn observe_session_id(server: &ServerId, name: &str) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::session_ids_args(server));
    tmux::interpret_session_id(succeeded, &stdout, name)
}

/// `session`'s panes with the window each belongs to, for the per-window glyphs.
#[must_use]
pub fn observe_window_panes(server: &ServerId, session: &str) -> Option<Vec<tmux::WindowPane>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::window_panes_args(server, session));
    tmux::interpret_window_panes(succeeded, &stdout)
}

/// Publish one user option on `target`, which must be an exact id.
#[must_use]
pub fn publish_option(
    server: &ServerId,
    scope: tmux::OptionScope,
    target: &str,
    name: &str,
    value: &str,
) -> bool {
    if !addressable(server) {
        return false;
    }
    let (succeeded, _) = run(
        PROGRAM,
        &tmux::set_option_args(server, scope, target, name, value),
    );
    succeeded
}

/// Remove one user option from `target`.
#[must_use]
pub fn clear_option(server: &ServerId, scope: tmux::OptionScope, target: &str, name: &str) -> bool {
    if !addressable(server) {
        return false;
    }
    let (succeeded, _) = run(
        PROGRAM,
        &tmux::unset_option_args(server, scope, target, name),
    );
    succeeded
}

/// Show a transient message on `target`'s clients.
#[must_use]
pub fn display_message(server: &ServerId, target: &str, text: &str) -> bool {
    if !addressable(server) {
        return false;
    }
    let (succeeded, _) = run(PROGRAM, &tmux::display_message_args(server, target, text));
    succeeded
}

/// Every session name `server` reports, or `None` when it did not answer.
#[must_use]
pub fn session_names(server: &ServerId) -> Option<Vec<String>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::list_sessions_args(server));
    tmux::interpret_sessions(succeeded, &stdout).ok()
}

/// The session the CALLING client is in, or `None` when there is no answer.
#[must_use]
pub fn observe_current_session(server: &ServerId) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::current_session_args(server));
    tmux::interpret_session_option(succeeded, &stdout)
}

/// The socket path `server` names for itself, or `None` when it did not answer.
#[must_use]
pub fn observe_socket_path(server: &ServerId) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::socket_path_args(server));
    tmux::interpret_display_value(succeeded, &stdout)
}

/// The process id `server` reports, or `None` when it did not answer.
#[must_use]
pub fn observe_server_pid(server: &ServerId) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::server_pid_args(server));
    tmux::interpret_display_value(succeeded, &stdout)
}

/// The ttys of every pane on `server`, or `None` when it did not answer.
#[must_use]
pub fn observe_pane_ttys(server: &ServerId) -> Option<Vec<String>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::pane_ttys_args(server));
    tmux::interpret_pane_ttys(succeeded, &stdout)
}

/// Hand this terminal to tmux and report what tmux exited with.
#[must_use]
pub fn focus(server: &ServerId, verb: tmux::FocusVerb, session: &str) -> u8 {
    if !addressable(server) {
        return FOCUS_FAILED;
    }
    let args = tmux::focus_args(server, verb, session);
    match spawn(PROGRAM, &args, &[], Streams::Terminal, None) {
        Some(output) => output
            .status
            .code()
            .and_then(|code| u8::try_from(code).ok())
            .unwrap_or(FOCUS_FAILED),
        None => FOCUS_FAILED,
    }
}

/// What a focus that never ran reports — `127`, the shell's command-not-found.
pub const FOCUS_FAILED: u8 = 127;

// ---------------------------------------------------------------------------
// Pane DELIVERY — the paste path's runs (B move 1).

/// What `pane` is running and under which pid, or `None` when the read failed.
#[must_use]
pub fn observe_pane_probe(server: &ServerId, pane: &str) -> Option<tmux::ObservedPaneProbe> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::pane_probe_args(server, pane));
    tmux::interpret_pane_probe(succeeded, &stdout)
}

/// `pane`'s visible screen, or `None` when the capture failed.
#[must_use]
pub fn capture_screen(server: &ServerId, pane: &str, styling: tmux::Styling) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::capture_screen_args(server, pane, styling));
    succeeded.then_some(stdout)
}

/// Stage `bytes` in `server`'s buffer `buffer`, on STDIN.
#[must_use]
pub fn load_buffer(server: &ServerId, buffer: &str, bytes: &[u8]) -> bool {
    if !addressable(server) {
        return false;
    }
    let args = tmux::load_buffer_args(server, buffer);
    spawn(PROGRAM, &args, &[], Streams::Captured, Some(bytes))
        .is_some_and(|output| output.status.success())
}

/// Paste `buffer` into `pane`, bracketed, deleting the buffer.
#[must_use]
pub fn paste_buffer(server: &ServerId, buffer: &str, pane: &str) -> bool {
    write_run(server, &tmux::paste_buffer_args(server, buffer, pane))
}

/// Drop a staged buffer that was never pasted.
#[must_use]
pub fn delete_buffer(server: &ServerId, buffer: &str) -> bool {
    write_run(server, &tmux::delete_buffer_args(server, buffer))
}

/// Send one key to `pane` WITHOUT selecting it.
#[must_use]
pub fn send_key(server: &ServerId, pane: &str, key: tmux::Key) -> bool {
    write_run(server, &tmux::send_keys_args(server, pane, key))
}

/// Every attached client's viewed pane and last-input epoch, or `None`.
#[must_use]
pub fn observe_clients(server: &ServerId) -> Option<Vec<tmux::ObservedClient>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::list_clients_args(server));
    tmux::interpret_clients(succeeded, &stdout)
}

/// A write whose only answer is its exit status.
fn write_run(server: &ServerId, args: &[String]) -> bool {
    addressable(server) && run(PROGRAM, args).0
}

/// The launch operation's tmux leg of the one process door — the ONLY way
/// product code runs a tmux command that is not already a typed builder above.
#[must_use]
pub(crate) fn run_tmux_op(argv: &crate::session_tmux::TmuxArgv) -> (bool, String) {
    match spawn(PROGRAM, argv.as_args(), &[], Streams::Captured, None) {
        Some(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ),
        None => (false, String::new()),
    }
}

/// The DETACHED leg of the one process door — the ONLY way product code starts
/// a child it does not wait for.
#[must_use]
pub(crate) fn spawn_detached(
    program: &std::path::Path,
    argv: &crate::session_launch::capture::CaptureArgv,
) -> bool {
    let Some(program) = program.to_str() else {
        return false;
    };
    spawn(program, argv.as_args(), &[], Streams::Detached, None).is_some()
}

#[cfg(test)]
mod tests {
    use super::{Tmux, run};
    use crate::inventory::{Discovery, QueryFailed, ServerId};
    use crate::meta::Selector;
    use std::path::PathBuf;

    #[test]
    fn a_program_that_ran_reports_success_and_its_output() {
        // THE CONTROL, AND IT COMES FIRST.
        assert_eq!(
            run("/bin/echo", &["ok".to_owned()]),
            (true, "ok\n".to_owned()),
            "this suite needs /bin/echo to prove the exec can succeed at all"
        );
    }

    #[test]
    fn a_program_that_cannot_be_spawned_is_a_failed_run_and_not_an_empty_one() {
        // The bytes are the same as a successful empty query's.
        let (succeeded, stdout) = run("ae-no-such-program-exists-anywhere", &[]);
        assert!(!succeeded, "a program that is not there did not run");
        assert!(stdout.is_empty());
        assert_eq!(
            crate::tmux::interpret_sessions(succeeded, &stdout),
            Err(QueryFailed),
            "and the pair is a FAILED query rather than a server with no sessions"
        );
    }

    #[test]
    fn a_child_that_ran_and_failed_is_a_failed_run_though_its_output_reads_like_an_answer() {
        // THE SHAPE THIS SLICE EXISTS TO KILL, and the one the two arms beside
        // it cannot reach.
        let (succeeded, stdout) = run(
            "/bin/sh",
            &["-c".to_owned(), "echo plausible; exit 1".to_owned()],
        );
        assert!(
            !succeeded,
            "a child that completed with a non-zero status did not succeed"
        );
        assert_eq!(
            stdout, "plausible\n",
            "and its bytes ARE here, which is exactly what makes dropping the status tempting"
        );
        assert_eq!(
            crate::tmux::interpret_sessions(succeeded, &stdout),
            Err(QueryFailed),
            "the status decides; output that reads like an answer does not"
        );
    }

    #[test]
    fn a_child_killed_by_a_signal_is_a_failed_run_although_it_has_no_exit_code_at_all() {
        // THE ARM NEITHER OF THE OTHERS REACHES, and it was found by the
        // reviewer attacking cold rather than by my own mutation list — which
        // is the argument for cold attacks in one sentence.
        let (succeeded, stdout) = run("/bin/sh", &["-c".to_owned(), "kill -TERM $$".to_owned()]);
        assert!(
            !succeeded,
            "a child that died on a signal did not succeed, though it reported no code"
        );
        assert!(stdout.is_empty(), "and it printed nothing before dying");
        assert_eq!(
            crate::tmux::interpret_sessions(succeeded, &stdout),
            Err(QueryFailed),
            "empty output from a killed child is not a server with no sessions"
        );
    }

    #[test]
    fn a_relative_socket_is_refused_rather_than_put_on_the_wire() {
        // WHAT THIS PINS IS THE OUTCOME, NOT THE REFUSAL.
        let relative = ServerId::Selected(Selector::Socket(PathBuf::from("relative/ae.sock")));
        assert_eq!(Tmux.enumerate(&relative), Err(QueryFailed));
    }
}
