//! The exec: the one place this crate starts a child process.
//!
//! [`crate::tmux`] owns both halves that can be WRONG — which server an
//! argument list addresses, and what a completed run means — and both are
//! proven against real isolated servers. What is here is the part in between,
//! which is a detail rather than a decision: it hands a derived argument list to
//! `tmux`, waits, and hands the completed run back to be interpreted. It derives
//! no argv of its own and interprets no bytes of its own.

use std::os::unix::fs::OpenOptionsExt as _;
use std::path::PathBuf;

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

/// Whether `server` has the exact session `name` — `tmux has-session -t =name`
/// before a cross-session lookup.
#[must_use]
pub fn session_exists(server: &ServerId, name: &str) -> bool {
    addressable(server) && run(PROGRAM, &tmux::has_session_args(server, name)).0
}

/// Whether `name` is verifiably STOPPED on its recorded `server` — the
/// tri-state the destructive compact gate crosses.
#[must_use]
pub fn verify_session_absent(server: &ServerId, name: &str) -> tmux::StopProbe {
    if !addressable(server) {
        return tmux::StopProbe::Unknown;
    }
    let (succeeded, stdout, stderr) = run_captured(PROGRAM, &tmux::list_sessions_args(server));
    tmux::interpret_stopped(succeeded, &stdout, &stderr, name)
}

/// What a stop-verification run SAW about `name` on `server`, before anything
/// is concluded from it — the RESUME's and the LISTING's question.
///
/// [`verify_session_absent`] is the strict verdict and stays the one the
/// destructive gates cross. This one keeps the two failure classes apart so
/// [`tmux::classify_absence`] can weigh a missing socket against the host's
/// boot time; the decision, and the boot time it needs, belong to the caller.
#[must_use]
pub fn probe_absence(server: &ServerId, name: &str) -> tmux::Absence {
    if !addressable(server) {
        return tmux::Absence::Unreachable;
    }
    let (succeeded, stdout, stderr) = run_captured(PROGRAM, &tmux::list_sessions_args(server));
    tmux::read_absence(succeeded, &stdout, &stderr, name)
}

/// Whether `server`'s SOCKET is not there at all — the server-level question,
/// which names no session.
#[must_use]
pub fn server_socket_missing(server: &ServerId) -> bool {
    if !addressable(server) {
        return false;
    }
    let (succeeded, _, stderr) = run_captured(PROGRAM, &tmux::list_sessions_args(server));
    !succeeded && tmux::read_failure(&stderr) == tmux::Absence::SocketMissing
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

/// The slot roster of `session` on the AMBIENT server, or `None` when the
/// enumeration failed.
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

/// One session option's value on `server`, or `None` when it is unset or the
/// server did not answer.
#[must_use]
pub fn observe_session_option(server: &ServerId, session: &str, name: &str) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::session_option_args(server, session, name));
    tmux::interpret_session_option(succeeded, &stdout)
}

/// One session option's three-state reading — the observation a WRITER may
/// act on, because it never collapses a failed read into "unset".
#[must_use]
pub fn observe_option_reading(server: &ServerId, session: &str, name: &str) -> tmux::OptionReading {
    if !addressable(server) {
        return tmux::OptionReading::Unknown;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::session_option_args(server, session, name));
    tmux::interpret_option_reading(succeeded, &stdout)
}

/// The two exact tmux-environment values that prove an old session belongs to
/// one ae state root. Either failed or malformed read makes the proof absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionOwnership {
    /// The nonempty `AE_SESSION` ownership marker. Its value is deliberately
    /// opaque: ae has always stamped `1`, and the exact tmux target owns the
    /// session name independently.
    pub marker: String,
    /// `AE_HOME`, expected to name the caller's state root.
    pub home: String,
}

/// Read the ownership pair from `session` on `server`.
#[must_use]
pub fn observe_session_ownership(server: &ServerId, session: &str) -> Option<SessionOwnership> {
    if !addressable(server) {
        return None;
    }
    let (marker_ok, marker_output) = run(PROGRAM, &tmux::marker_args(server, session));
    let marker =
        tmux::interpret_marker(marker_ok, &marker_output).filter(|value| !value.is_empty())?;
    let (homed, home) = run(
        PROGRAM,
        &tmux::environment_value_args(server, session, tmux::HOME_VARIABLE),
    );
    let home = tmux::interpret_environment_value(homed, &home, tmux::HOME_VARIABLE)?;
    Some(SessionOwnership { marker, home })
}

/// What running the `send` helper produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// The helper's exit code; `None` when it could not be spawned at all, or
    /// died to a signal.
    pub code: Option<i32>,
    /// Its stdout — from `_send-deliver`, the stored body's path plus `\n`.
    pub stdout: String,
}

/// Run the session's send helper at `helper` with an optional leading
/// cross-session capability, then `target` and `message`, plus `envs` (the
/// event fields the body store names the recovery file after). stderr is
/// INHERITED — the helper's refusals and
/// unconfirmed-submit lines are the caller's diagnostics, verbatim; stdin is
/// null.
#[must_use]
pub fn deliver(
    helper: &std::path::Path,
    target: &str,
    message: &str,
    cross_session: bool,
    envs: &[(&str, &str)],
) -> Delivery {
    let mut args = Vec::with_capacity(if cross_session { 3 } else { 2 });
    if cross_session {
        args.push(crate::tracked::CROSS_SESSION_FLAG.to_owned());
    }
    args.push(target.to_owned());
    args.push(message.to_owned());
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
    streams: Streams<'_>,
    feed: Option<&[u8]>,
) -> Option<std::process::Output> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    command.envs(envs.iter().copied());
    // `AE_VERSION` is the TARGET PIN of `ae upgrade` and nothing else's input.
    command.env_remove("AE_VERSION");
    if matches!(streams, Streams::InheritStderr) {
        command.stderr(std::process::Stdio::inherit());
    }
    if matches!(streams, Streams::Detached) {
        command.stdin(std::process::Stdio::null());
        command.stdout(std::process::Stdio::null());
        command.stderr(std::process::Stdio::null());
        let mut child = command.spawn().ok()?;
        // A long-lived watchdog schedules many short children, and lifecycle
        // callers share this detached leg for their supervisors. Dropping the
        // handles leaves zombies while their parent lives, so one tiny waiter
        // owns each handle while the detached child still outlives its caller.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        return Some(std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: Vec::new(),
            stderr: Vec::new(),
        });
    }
    if matches!(streams, Streams::Terminal) {
        // Nothing is captured, so there is nothing to return but the status —
        // and an `Output` carrying it keeps every caller of this door reading
        // one shape.
        return command.status().ok().map(|status| std::process::Output {
            status,
            stdout: Vec::new(),
            stderr: Vec::new(),
        });
    }
    if let Streams::CapturedToFile { file, cap } = streams {
        // A body and a stdout file cannot both own the child's streams.
        if feed.is_some() {
            return None;
        }
        let handle = file.try_clone().ok()?;
        command.stdout(std::process::Stdio::from(handle));
        // stderr stays captured by `output()`; stdout is the scratch file's.
        let output = command.output().ok()?;
        // The child wrote through a dup of this handle, so the offset is shared
        // and sits at the end: rewind, then read AT MOST `cap + 1` bytes. A
        // child past its ceiling is refused by the caller's reader without its
        // whole output ever being allocated.
        let mut cursor = file;
        if std::io::Seek::seek(&mut cursor, std::io::SeekFrom::Start(0)).is_err() {
            return None;
        }
        let mut limited = std::io::Read::take(cursor, cap.saturating_add(1));
        let mut bytes = Vec::new();
        if std::io::Read::read_to_end(&mut limited, &mut bytes).is_err() {
            return None;
        }
        return Some(std::process::Output {
            status: output.status,
            stdout: bytes,
            stderr: output.stderr,
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
#[derive(Debug, Clone, Copy)]
enum Streams<'a> {
    /// Both captured.
    Captured,
    /// Stdout captured, stderr ae's own — the send helper, whose diagnostics
    /// belong in the pane that invoked it.
    InheritStderr,
    /// Every stream is ae's own.
    Terminal,
    /// Started and NOT waited for.
    Detached,
    /// Stdout redirected into this scratch handle — its owner created it
    /// exclusively at 0600 — and read back after the child exits, at most
    /// `cap + 1` bytes. The `opencode` legs need it: `opencode export` exits
    /// before its stdout pipe drains (measured on 1.18.31, 2026-09-18: 131072
    /// of 3949726 bytes through a pipe, the whole document to a regular file),
    /// so a pipe capture truncates the JSON document while a file receives it
    /// whole. Passing a `feed` body together with this variant is a caller
    /// error, and the door refuses it.
    CapturedToFile { file: &'a std::fs::File, cap: u64 },
}

/// One captured child's scratch file: created exclusively at 0600 and removed
/// when this guard drops, so no early return — nor a future caller — can leave
/// a conversation transcript in the temp directory.
struct CaptureScratch {
    file: std::fs::File,
    path: PathBuf,
}

impl CaptureScratch {
    /// Mint one under the OS temp directory. The pid and an attempt counter
    /// name it; `create_new` refuses a pre-planted node — a symlink included —
    /// and a collision retries the next name, the house loop
    /// `upgrade::Scratch` runs.
    fn new() -> Option<Self> {
        let base = std::env::temp_dir();
        for attempt in 0..64_u32 {
            let path = base.join(format!("ae-opencode.{}.{attempt}.json", std::process::id()));
            let opened = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path);
            match opened {
                Ok(file) => return Some(Self { file, path }),
                Err(why) if why.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return None,
            }
        }
        None
    }

    /// The handle the door clones for the child's stdout and reads back.
    fn file(&self) -> &std::fs::File {
        &self.file
    }
}

impl Drop for CaptureScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
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

/// The boot-time leg of the one process door — the ONLY way product code runs
/// `sysctl`, and it takes no arguments at all, because there is exactly one
/// question ae asks it: when did this host boot. Linux answers that from
/// `/proc/stat` and never reaches here; macOS has no such file and ae has no
/// libc to call, so the answer comes from a child process. The reading is
/// [`crate::doors::boot_time`]'s.
pub(crate) fn run_sysctl() -> (bool, String) {
    match spawn(
        "sysctl",
        &["-n", "kern.boottime"],
        &[],
        Streams::Captured,
        None,
    ) {
        Some(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ),
        None => (false, String::new()),
    }
}

/// The `opencode` leg of the one process door — the ONLY way product code runs
/// `opencode`, the program FIXED here so a caller chooses nothing at all.
///
/// The capture goes through a SCRATCH FILE, never a pipe: `opencode export`
/// exits before its stdout pipe drains (measured on 1.18.31, 2026-09-18), so a
/// piped capture truncates the document while a regular file receives it
/// whole. The file lives in the OS temp directory, is created 0600, and its
/// guard removes it on every path. `cap` is the caller's byte ceiling: the door
/// returns at most `cap + 1` bytes, and the caller's own reader refuses
/// anything past its cap with its own reason.
pub(crate) fn run_opencode(
    argv: &crate::session_launch::capture::OpenCodeArgv,
    cap: u64,
) -> (bool, String) {
    let Some(scratch) = CaptureScratch::new() else {
        return (false, String::new());
    };
    let captured = spawn(
        "opencode",
        argv.as_args(),
        &[],
        Streams::CapturedToFile {
            file: scratch.file(),
            cap,
        },
        None,
    );
    match captured {
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

/// The automatic-upgrade leg of the existing detached process door. The argv
/// is sealed by `autoupgrade`; this layer only adds fixed `nohup` and streams.
pub(crate) fn run_autoupgrade_detached(argv: &crate::autoupgrade::DetachedArgv) -> bool {
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

/// The motion ticker's single pane observation for `session` on `server`.
#[must_use]
pub(crate) fn observe_motion_panes(
    server: &ServerId,
    session: &str,
) -> Option<Vec<tmux::MotionPane>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::motion_panes_args(server, session));
    tmux::interpret_motion_panes(succeeded, &stdout)
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

/// The IMMUTABLE identity — the `$<n>` id AND the creation instant — of the
/// session named EXACTLY `name`, or `None`.
///
/// `#{session_id}` alone is not immutable: tmux starts again at `$0` once
/// every session on a server is gone, so a name whose session was killed and
/// recreated can answer the same id while being another incarnation. This is
/// the identity a WRITE carries.
#[must_use]
pub fn observe_session_identity(server: &ServerId, name: &str) -> Option<tmux::SessionIdentity> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::session_identities_args(server));
    tmux::interpret_session_identity(succeeded, &stdout, name)
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

/// Set one session option through the SERVER-SIDE guard: one tmux invocation
/// whose `if-shell` checks the proven server and session incarnations and the
/// option's vacancy, then sets in the same queued command. There is no window
/// between a check and the write for a replacement to use.
#[must_use]
pub fn publish_guarded_session_option(
    server: &ServerId,
    expected_server: &tmux::ServerIdentity,
    expected_session: &tmux::SessionIdentity,
    name: &str,
    value: &str,
) -> bool {
    if !addressable(server) {
        return false;
    }
    let (succeeded, _) = run(
        PROGRAM,
        &tmux::guarded_session_option_args(server, expected_server, expected_session, name, value),
    );
    succeeded
}

/// The session identity one exact PANE belongs to. Used by the MIGRATION
/// capture for a recorded main pane; a pane id is reusable after a server
/// restart, so the caller must pair this with the session's ownership pair and
/// the guarded write still reproves the full identity.
#[must_use]
pub fn observe_pane_session_identity(
    server: &ServerId,
    pane: &str,
) -> Option<tmux::SessionIdentity> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::pane_session_identity_args(server, pane));
    tmux::interpret_pane_session_identity(succeeded, &stdout)
}

/// Publish several option values in one tmux process.
#[must_use]
pub(crate) fn publish_options(server: &ServerId, writes: &[tmux::OptionWrite]) -> bool {
    if !addressable(server) || writes.is_empty() {
        return false;
    }
    run(PROGRAM, &tmux::set_options_args(server, writes)).0
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

/// Remove one transient session option and publish its mutually exclusive
/// replacement in one tmux command queue.
#[must_use]
pub fn replace_session_option(
    server: &ServerId,
    target: &str,
    remove: &str,
    set: &str,
    value: &str,
) -> bool {
    if !addressable(server) {
        return false;
    }
    run(
        PROGRAM,
        &tmux::replace_session_option_args(server, target, remove, set, value),
    )
    .0
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

/// Show a transient message on one attached client.
#[must_use]
pub fn display_client_message(server: &ServerId, client: &str, text: &str) -> bool {
    if !addressable(server) {
        return false;
    }
    let (succeeded, _) = run(
        PROGRAM,
        &tmux::display_client_message_args(server, client, text),
    );
    succeeded
}

/// Ask one attached client to authorize a deferred tmux command.
#[must_use]
pub fn confirm_before(server: &ServerId, client: &str, prompt: &str, continuation: &str) -> bool {
    if !addressable(server) {
        return false;
    }
    let (succeeded, _) = run(
        PROGRAM,
        &tmux::confirm_before_args(server, client, prompt, continuation),
    );
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

/// Every stamped pane of every session on `server`, or `None` when the
/// listing failed — the fleet picker's one tmux read.
#[must_use]
pub fn observe_fleet_panes(server: &ServerId) -> Option<Vec<tmux::FleetPane>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::fleet_panes_args(server));
    tmux::interpret_fleet_panes(succeeded, &stdout)
}

/// Every live ae session as the fleet picker reads it in one server call.
#[must_use]
pub fn observe_picker_sessions(server: &ServerId) -> Option<Vec<tmux::PickerSession>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::picker_sessions_args(server));
    tmux::interpret_picker_sessions(succeeded, &stdout)
}

/// Every live pane membership as the picker verifies it in one server call.
#[must_use]
pub fn observe_picker_panes(server: &ServerId) -> Option<Vec<tmux::PickerPane>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::picker_panes_args(server));
    tmux::interpret_picker_panes(succeeded, &stdout)
}

/// The validated live snapshot for one explicit picker client.
#[must_use]
pub fn observe_picker_client_session(
    server: &ServerId,
    client: &str,
) -> Option<tmux::PickerClient> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::picker_client_sessions_args(server));
    tmux::interpret_picker_client_session(succeeded, &stdout, client)
}

/// The picker's whole pre-draw read in ONE tmux invocation — see
/// [`tmux::picker_read_args`] for what travels and why one connection.
///
/// The run's exit status is deliberately not consulted: tmux stops a command
/// list at the first failure, and each command's own completion marker is the
/// finer proof — a section whose marker never arrived is unknown, and output
/// that is not this read's at all is refused. That is the same verdict a failed
/// run would produce, without letting one late failure discard the sections
/// that did answer.
#[must_use]
pub fn observe_picker_read(server: &ServerId, client: &str) -> Option<tmux::PickerRead> {
    if !addressable(server) {
        return None;
    }
    let (_, stdout) = run(PROGRAM, &tmux::picker_read_args(server, client));
    tmux::interpret_picker_read(&stdout, client)
}

/// `server`'s identity pair, or `None` when it did not answer with one.
#[must_use]
pub fn observe_server_identity(server: &ServerId) -> Option<tmux::ServerIdentity> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::server_identity_args(server));
    tmux::interpret_server_identity(succeeded, &stdout)
}

/// The one attached client called `client`, with the process behind it.
#[must_use]
pub fn observe_menu_client(server: &ServerId, client: &str) -> Option<tmux::MenuClient> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::menu_clients_args(server));
    tmux::interpret_menu_client(succeeded, &stdout, client)
}

/// Draw `menu` centred on one explicit client, with one explicit target pane.
#[must_use]
pub fn display_menu_centred(
    server: &ServerId,
    client: &str,
    target: &str,
    menu: &tmux::Menu,
    menu_mouse: bool,
) -> bool {
    addressable(server)
        && run(
            PROGRAM,
            &tmux::display_menu_centred_args(server, client, target, menu, menu_mouse),
        )
        .0
}

/// Draw settings at one explicit client's numeric bottom-right coordinate,
/// with one explicit target pane supplying the command context.
#[must_use]
pub fn display_settings_menu(
    server: &ServerId,
    client: &str,
    target: &str,
    x: usize,
    menu: &tmux::Menu,
    menu_mouse: bool,
) -> bool {
    addressable(server)
        && run(
            PROGRAM,
            &tmux::display_settings_menu_args(server, client, target, x, menu, menu_mouse),
        )
        .0
}

/// Draw `menu` on `client`, or the server's current client when absent.
#[must_use]
pub fn display_menu(
    server: &ServerId,
    client: Option<&str>,
    menu: &tmux::Menu,
    menu_mouse: bool,
) -> bool {
    addressable(server)
        && run(
            PROGRAM,
            &tmux::display_menu_for_client_args(server, client, menu, menu_mouse),
        )
        .0
}

/// The version `server` is RUNNING, or `None` when it did not answer.
#[must_use]
pub fn observe_tmux_version(server: &ServerId) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::version_args(server));
    tmux::interpret_display_value(succeeded, &stdout)
}

/// The version of the tmux BINARY on `PATH`, or `None` when it did not run.
#[must_use]
pub fn observe_tmux_program_version() -> Option<String> {
    let (succeeded, stdout) = run(PROGRAM, &tmux::program_version_args());
    tmux::interpret_program_version(succeeded, &stdout)
}

/// What `server` answered when asked its own version, told apart from the two
/// ways the asking can fail.
#[must_use]
pub fn probe_tmux_version(server: &ServerId) -> tmux::VersionProbe {
    // A socket path ae cannot address is a socket with no server behind it,
    // which is the same fact tmux states as "no server running on …".
    if !addressable(server) {
        return tmux::VersionProbe::NoServer;
    }
    let (succeeded, stdout, stderr) = run_captured(PROGRAM, &tmux::version_args(server));
    tmux::interpret_version(succeeded, &stdout, &stderr)
}

/// WHICH tmux would actually run, for the floor gate and the surfaces that
/// report it.
///
/// The live server first, because a running one keeps executing the binary that
/// started it whatever `PATH` now holds. The `PATH` binary is consulted ONLY
/// when tmux positively said there is no server: a server that merely could not
/// be reached might be any version, and answering for it with the version of a
/// binary that will never run it is how an old server clears the floor.
#[must_use]
pub fn observe_tmux_floor(server: &ServerId) -> crate::tmux_floor::Probe {
    match probe_tmux_version(server) {
        tmux::VersionProbe::Answered(found) => crate::tmux_floor::Probe::Server(found),
        tmux::VersionProbe::NoServer => match observe_tmux_program_version() {
            Some(found) => crate::tmux_floor::Probe::Executable(found),
            None => crate::tmux_floor::Probe::Silent,
        },
        // A server that could not be reached is a REFUSAL — except when there
        // is no runnable tmux at all, which is a machine without tmux and not a
        // server with a problem. The `PATH` binary is asked only to tell those
        // two apart; its version is never stood in for the server's.
        tmux::VersionProbe::Unreachable => match observe_tmux_program_version() {
            Some(_) => crate::tmux_floor::Probe::Unreachable,
            None => crate::tmux_floor::Probe::Silent,
        },
    }
}

/// The key-table entries `server` reports for `table` (`root`, `prefix`), or
/// `None` when it did not answer — `ae doctor`'s read-only input-map check.
#[must_use]
pub fn observe_key_bindings(server: &ServerId, table: &str) -> Option<Vec<tmux::KeyBinding>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::list_keys_args(server, table));
    tmux::interpret_list_keys(succeeded, &stdout)
}

/// Every session on `server` with the attention its own watchdog published, or
/// `None` when the server did not answer.
#[must_use]
pub fn observe_fleet_sessions(server: &ServerId) -> Option<Vec<tmux::FleetSession>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::fleet_sessions_args(server));
    tmux::interpret_fleet_sessions(succeeded, &stdout)
}

/// Every session on `server` with its attention AND the look it is drawn in, or
/// `None` when the server did not answer.
///
/// The watchdog's read: the same one process answers who is on the server and
/// what each of them is drawn in, so a daemon filling a watchdog-less peer's
/// fleet strip draws it in THAT session's colours without a query per peer.
#[must_use]
pub fn observe_fleet_listing(server: &ServerId) -> Option<Vec<tmux::FleetListingRow>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::fleet_listing_args(server));
    tmux::interpret_fleet_listing(succeeded, &stdout)
}

/// The `(icons, palette)` pair `session` is drawn with, each empty when unset.
#[must_use]
pub fn observe_look(server: &ServerId, session: &str) -> Option<tmux::LookOptions> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::look_args(server, session));
    tmux::interpret_look(succeeded, &stdout)
}

/// The `(icons, palette)` pair of the session the CALLING client is in.
#[must_use]
pub fn observe_look_here(server: &ServerId) -> Option<tmux::LookOptions> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::look_here_args(server));
    tmux::interpret_look(succeeded, &stdout)
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

/// Hand this terminal to `server`; tmux chooses its most recently used session.
#[must_use]
pub fn attach(server: &ServerId) -> u8 {
    if !addressable(server) {
        return FOCUS_FAILED;
    }
    let args = tmux::attach_args(server);
    match spawn(PROGRAM, &args, &[], Streams::Terminal, None) {
        Some(output) => output
            .status
            .code()
            .and_then(|code| u8::try_from(code).ok())
            .unwrap_or(FOCUS_FAILED),
        None => FOCUS_FAILED,
    }
}

/// Move one exact client to one exact session, or report tmux's refusal.
#[must_use]
pub fn switch_client(server: &ServerId, client: &str, session: &str) -> bool {
    addressable(server) && run(PROGRAM, &tmux::switch_client_args(server, client, session)).0
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

/// Whether `pane` is a dead pane tmux is holding on screen, or `None` when
/// that could not be read — see [`tmux::interpret_pane_dead`].
#[must_use]
pub fn observe_pane_dead(server: &ServerId, pane: &str) -> Option<bool> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::pane_dead_args(server, pane));
    tmux::interpret_pane_dead(succeeded, &stdout)
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
    use super::{CaptureScratch, Streams, Tmux, run, spawn};
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
    #[allow(
        clippy::disallowed_methods,
        reason = "a test: creates, stats and removes its own scratch capture files"
    )]
    fn a_captured_to_file_spawn_returns_the_whole_stdout_and_its_guard_removes_it() {
        // The opencode capture's contract: the child's stdout reaches the
        // scratch file whole and comes back through the same `Output` shape
        // every other leg returns. 128 KiB is exactly the scale a pipe lost.
        let scratch = CaptureScratch::new().expect("a scratch file");
        let path = scratch.path.clone();
        let output = spawn(
            "/bin/sh",
            &["-c".to_owned(), "yes x | head -c 131072".to_owned()],
            &[],
            Streams::CapturedToFile {
                file: scratch.file(),
                cap: u64::MAX,
            },
            None,
        )
        .expect("the shell runs");
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 131_072, "the whole stdout came back");
        assert!(
            path.exists(),
            "the guard still owns the file while it lives"
        );
        drop(scratch);
        assert!(!path.exists(), "the guard removed the scratch file");
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test: stats its own scratch capture file"
    )]
    fn the_capture_scratch_is_created_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let scratch = CaptureScratch::new().expect("a scratch file");
        let mode = std::fs::metadata(&scratch.path)
            .expect("the scratch file")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "the whole conversation transcript is owner-only"
        );
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test: pre-plants and removes its own scratch capture name"
    )]
    fn a_pre_planted_first_name_still_yields_a_capture() {
        let first = std::env::temp_dir().join(format!("ae-opencode.{}.0.json", std::process::id()));
        let _ = std::fs::remove_file(&first);
        std::fs::write(&first, b"planted").expect("the plant");
        let scratch = CaptureScratch::new().expect("a capture despite the plant");
        assert_ne!(
            scratch.path, first,
            "the collision was skipped, not refused"
        );
        drop(scratch);
        let _ = std::fs::remove_file(&first);
    }

    #[test]
    fn a_captured_to_file_spawn_returns_at_most_cap_plus_one_bytes() {
        let scratch = CaptureScratch::new().expect("a scratch file");
        let output = spawn(
            "/bin/sh",
            &["-c".to_owned(), "yes x | head -c 200000".to_owned()],
            &[],
            Streams::CapturedToFile {
                file: scratch.file(),
                cap: 1000,
            },
            None,
        )
        .expect("the shell runs");
        assert_eq!(
            output.stdout.len(),
            1001,
            "the door reads at most cap + 1 bytes"
        );
    }

    #[test]
    fn a_capture_with_a_feed_body_is_refused() {
        let scratch = CaptureScratch::new().expect("a scratch file");
        assert!(
            spawn::<&str>(
                "/bin/cat",
                &[],
                &[],
                Streams::CapturedToFile {
                    file: scratch.file(),
                    cap: 1000,
                },
                Some(b"body"),
            )
            .is_none(),
            "a body and a stdout file cannot both own the child's streams"
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
