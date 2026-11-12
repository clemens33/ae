//! The exec: the one place this crate starts a child process.
//!
//! [`crate::tmux`] owns both halves that can be WRONG — which server an
//! argument list addresses, and what a completed run means — and both are
//! proven against real isolated servers. What is here is the part in between,
//! which is a detail rather than a decision: it hands a derived argument list to
//! `tmux`, waits, and hands the completed run back to be interpreted. It derives
//! no argv of its own and interprets no bytes of its own.

use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    settled_read(
        || {
            let (succeeded, stdout, stderr) =
                run_captured(PROGRAM, &tmux::list_sessions_args(server));
            tmux::read_absence(succeeded, &stdout, &stderr, name)
        },
        || std::thread::sleep(KILL_VERIFY_SETTLE),
    )
}

/// One absence read, asked again ONCE past `settle` when it was `Unreachable`:
/// the last session's death leaves the server mid-exit, and a client that
/// connected before the exit finished reads `server exited unexpectedly`,
/// which proves nothing. The second read crosses the same proof, so a server
/// still unreachable stays `Unreachable`; no other first answer is re-asked.
fn settled_read(mut read: impl FnMut() -> tmux::Absence, settle: impl FnOnce()) -> tmux::Absence {
    let first = read();
    if first != tmux::Absence::Unreachable {
        return first;
    }
    settle();
    read()
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

/// Every pane of every session on `server`, with each session's published
/// branch, from ONE `list-panes -a` — or `None` when the enumeration failed.
#[must_use]
pub fn observe_all_panes(server: &ServerId) -> Option<tmux::ServerPanes> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::list_all_panes_args(server));
    tmux::interpret_all_panes(succeeded, &stdout).ok()
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
    viewer_observation(server, pane, &mut false)
}

/// The caller or guarded target's viewer observation, with one failed tmux
/// command re-asked. Positive answers, including vacant stamps, are final.
#[must_use]
pub(crate) fn reask_viewer(server: &ServerId, pane: &str) -> Option<tmux::ObservedViewer> {
    viewer_observation(server, pane, &mut true)
}

/// One budget follows the observation through every native-view leg.
fn viewer_observation(
    server: &ServerId,
    pane: &str,
    reask: &mut bool,
) -> Option<tmux::ObservedViewer> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = observation_read(&tmux::viewer_args(server, pane), reask);
    let viewer = tmux::interpret_viewer(succeeded, &stdout)?;
    let Some(view) = viewer
        .session
        .as_deref()
        .filter(|name| tmux::is_view_session(name))
    else {
        return Some(viewer);
    };
    let home = home_of(server, view, pane, reask)?;
    let (succeeded, stdout) = observation_read(&tmux::viewer_args(server, &home.target), reask);
    tmux::interpret_viewer(succeeded, &stdout)
        .filter(|viewer| viewer.session.as_deref() == Some(home.name.as_str()))
}

/// Re-ask only a failed COMMAND, once across the whole observation. Parsing
/// and identity comparisons stay outside this rule: a positive refusal must
/// never be re-read into a pass. No settle is added under the send-lock.
fn observation_read(args: &[String], reask: &mut bool) -> (bool, String) {
    let first = run(PROGRAM, args);
    if !first.0 && std::mem::take(reask) {
        return run(PROGRAM, args);
    }
    first
}

/// The real session a native view stands in for, proven for one pane.
struct Home {
    /// The `$<n>` id the view recorded.
    id: String,
    /// That session's name, from the same read that listed the pane in it.
    name: String,
    /// The pane pinned inside it, `$<n>:.%<m>`.
    target: String,
}

/// The real session the native view `view` recorded, when `pane` is one of
/// its panes; `None` on any gap — never the view. tmux answers a pinned read
/// for a pane outside the session too, so membership is read, not assumed.
fn home_of(server: &ServerId, view: &str, pane: &str, reask: &mut bool) -> Option<Home> {
    let (succeeded, stdout) = observation_read(&tmux::view_of_args(server, view), reask);
    let id = succeeded.then(|| tmux::view_of_value(&stdout)).flatten()?;
    let target = tmux::pinned_pane_target(&id, pane)?;
    let (succeeded, stdout) = observation_read(&tmux::view_membership_args(server, &id), reask);
    let name = tmux::view_member_name(succeeded, &stdout, &id, pane)?;
    Some(Home { id, name, target })
}

/// Whether `server` is something this transport may put on the wire.
fn addressable(server: &ServerId) -> bool {
    match server {
        ServerId::Selected(Selector::Socket(path)) => tmux::is_addressable_socket(path),
        ServerId::Ambient | ServerId::Selected(Selector::Name(_)) => true,
    }
}

/// The one child every leg starts, `-u` first for a tmux client ae reads and
/// the inherited variables removed; [`spawn`] and [`spawn_until`] wire it.
#[allow(
    clippy::disallowed_types,
    reason = "the product's door: ae cannot answer a liveness question without running tmux, nor deliver a tracked request without the session's send helper, nor derive an archive preview's git facts without running git, nor read agy's or muse's quota windows without running them"
)]
fn command_for<A: AsRef<std::ffi::OsStr>>(
    program: &str,
    args: &[A],
    envs: &[(&str, &str)],
    utf8_client: bool,
) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    if utf8_client {
        // tmux prints `_` for every non-ASCII character and control byte to a
        // client it does not judge UTF-8, and it judges that from `$TMUX` or a
        // UTF-8 locale alone. A plain shell may have neither, yet ae reads what
        // the client prints as UTF-8 (#187).
        command.arg("-u");
    }
    command.args(args);
    command.envs(envs.iter().copied());
    // `AE_VERSION` is the TARGET PIN of `ae upgrade` and nothing else's input.
    command.env_remove("AE_VERSION");
    if program == "git" {
        // A hook's caller environment would otherwise re-aim every query.
        for var in GIT_LOCAL_ENV_VARS {
            command.env_remove(var);
        }
    }
    command
}

/// The children [`spawn`] starts are observed by tests alone.
#[cfg(not(test))]
fn note_spawn<'a>(_: &std::ffi::OsStr, _: impl Iterator<Item = &'a std::ffi::OsStr>) {}

#[cfg(test)]
pub(crate) use tests::{
    WaitChildEvent, hold_release_grace, note_child_reaped, note_child_spawned, note_spawn,
    note_waiter_exit, release_grace, spawns_during, wait_children_during, waiter_exits_during,
};

/// How each waiter [`run_tmux_wait`] started ended is observed by tests alone.
#[cfg(not(test))]
const fn note_waiter_exit(_: u32, _: std::process::ExitStatus) {}

/// Whether a wait-leg child started, and its pid, is observed by tests alone.
#[cfg(not(test))]
const fn note_child_spawned(_: Option<u32>) {}

/// Each completed reap of a wait-leg child is observed by tests alone.
#[cfg(not(test))]
const fn note_child_reaped(_: u32, _: std::process::ExitStatus) {}

/// Run `program` with `args`; report whether it succeeded, and what it printed.
fn spawn<A: AsRef<std::ffi::OsStr>>(
    program: &str,
    args: &[A],
    envs: &[(&str, &str)],
    streams: Streams<'_>,
    feed: Option<&[u8]>,
) -> Option<std::process::Output> {
    let mut command = command_for(program, args, envs, declares_utf8(program, streams));
    note_spawn(command.get_program(), command.get_args());
    if matches!(streams, Streams::InheritStderr) {
        command.stderr(std::process::Stdio::inherit());
    }
    if matches!(streams, Streams::Detached | Streams::DetachedIn(_)) {
        if let Streams::DetachedIn(dir) = streams {
            command.current_dir(dir);
            std::os::unix::process::CommandExt::process_group(&mut command, 0);
        }
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
        // whole output ever being allocated. This read is on a handle WE minted
        // (create_new, 0600, single writer, guard-removed), not a world path —
        // which is why it is not in the phase-3 world-read inventory.
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

/// How a deadline-bounded run ended.
enum Ran {
    /// It exited by itself: its status and its stdout, at most `cap` bytes.
    Exited(std::process::ExitStatus, Vec<u8>),
    /// The deadline passed first; the child was killed and reaped.
    TimedOut,
    /// No such program on the child's `PATH`.
    Absent,
    Over,
    /// Not started, watched or read back.
    Failed,
}

/// [`spawn`]'s sibling for the legs that talk to the network (agy's and muse's): it stops
/// waiting at `deadline`. A supplied capture is written by the macOS script;
/// its stdout is discarded. No stream is a pipe: stdin and
/// stderr are closed, and stdout is a scratch file whose length each `POLL`
/// reads from ae's own handle. A stopped child is killed, then reaped; a
/// grandchild outlives it and can write only into the unlinked scratch inode.
fn spawn_until<A: AsRef<std::ffi::OsStr>>(
    program: &str,
    args: &[A],
    envs: &[(&str, &str)],
    cap: u64,
    deadline: Duration,
    capture: Option<&CaptureScratch>,
    cwd: Option<&Path>,
) -> Ran {
    const POLL: Duration = Duration::from_millis(25);
    fn halt(child: &mut std::process::Child, ended: Ran) -> Ran {
        let _ = child.kill();
        let _ = child.wait();
        ended
    }
    let owned = capture
        .is_none()
        .then(|| {
            CaptureScratch::tagged(match program {
                "agy" => "agy",
                "muse" => "muse",
                _ => "clipboard",
            })
        })
        .flatten();
    let Some(scratch) = capture.or(owned.as_ref()) else {
        return Ran::Failed;
    };
    let Ok(handle) = scratch.file().try_clone() else {
        return Ran::Failed;
    };
    // Never a tmux client, so never `-u`.
    let mut command = command_for(program, args, envs, false);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    command.stdin(std::process::Stdio::null());
    command.stdout(capture_output(handle, capture.is_some()));
    command.stderr(std::process::Stdio::null());
    let started = std::time::Instant::now();
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Ran::Absent,
        Err(_) => return Ran::Failed,
    };
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(_) => return halt(&mut child, Ran::Failed),
        }
        match scratch.file().metadata() {
            Ok(written) if written.len() <= cap => {}
            // Past its cap, or ae's own handle cannot be read.
            Ok(_) => return halt(&mut child, Ran::Over),
            Err(_) => return halt(&mut child, Ran::Failed),
        }
        if started.elapsed() >= deadline {
            return halt(&mut child, Ran::TimedOut);
        }
        std::thread::sleep(POLL);
    };
    // Rewound, as the file capture above is, and read to at most `cap + 1`.
    let mut cursor = scratch.file();
    let (limit, mut stdout) = (cap.saturating_add(1), Vec::new());
    let read = std::io::Seek::seek(&mut cursor, std::io::SeekFrom::Start(0)).and_then(|_| {
        std::io::Read::read_to_end(&mut std::io::Read::take(cursor, limit), &mut stdout)
    });
    match read {
        Ok(len) if u64::try_from(len).is_ok_and(|len| len <= cap) => Ran::Exited(status, stdout),
        Ok(_) => Ran::Over,
        Err(_) => Ran::Failed,
    }
}

fn capture_output(handle: std::fs::File, separate: bool) -> std::process::Stdio {
    if separate {
        std::process::Stdio::null()
    } else {
        handle.into()
    }
}

/// Fixed clipboard programs; only checkout builds honour the fake-tool door.
pub(crate) fn run_clipboard_image() -> Result<Vec<u8>, String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "checkout-only clipboard tool seam"
    )]
    let fake = crate::shape::current()
        .honours_environment()
        .then(|| std::env::var("AE_TEST_CLIPBOARD_TOOL").ok())
        .flatten();
    #[allow(
        clippy::disallowed_methods,
        reason = "named Linux clipboard display door"
    )]
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let scratch = if fake.is_none() && cfg!(target_os = "macos") {
        Some(CaptureScratch::tagged("clipboard").ok_or("clipboard tool failed")?)
    } else {
        None
    };
    let path = scratch
        .as_ref()
        .map(|s| s.path.to_str().ok_or("clipboard tool failed"))
        .transpose()?
        .unwrap_or_default();
    let (program, args): (&str, &[&str]) = if let Some(fake) = fake.as_deref() {
        (fake, &[])
    } else if scratch.is_some() {
        ("osascript", &["-e", CLIPBOARD_SCRIPT, path])
    } else if wayland {
        ("wl-paste", &["--no-newline", "--type", "image/png"])
    } else {
        (
            "xclip",
            &["-selection", "clipboard", "-t", "image/png", "-o"],
        )
    };
    match spawn_until(
        program,
        args,
        &[],
        20 * 1024 * 1024,
        Duration::from_secs(8),
        scratch.as_ref(),
        None,
    ) {
        Ran::Exited(exit, bytes) if exit.success() => Ok(bytes),
        Ran::Absent => Err(format!(
            "clipboard tool missing: {}",
            Path::new(program)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        )),
        Ran::Over => Err("clipboard image over 20 MiB".to_owned()),
        Ran::TimedOut => Err("clipboard read timed out".to_owned()),
        _ => Err("clipboard tool failed".to_owned()),
    }
}

const CLIPBOARD_SCRIPT: &str = "on run argv\ntry\nset imageData to the clipboard as «class PNGf»\non error\nreturn\nend try\nset imageFile to open for access POSIX file (item 1 of argv) with write permission\ntry\nset eof imageFile to 0\nwrite imageData to imageFile\nclose access imageFile\non error\nclose access imageFile\nerror number -1\nend try\nend run";

/// Whether a leg starts `program` as a UTF-8 tmux client: every tmux leg ae
/// reads, never `Terminal`, whose client draws on the human's own terminal.
fn declares_utf8(program: &str, streams: Streams<'_>) -> bool {
    program == PROGRAM && !matches!(streams, Streams::Terminal)
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
    /// [`Streams::Detached`], started in this directory and in a process group
    /// of its own, so a signal to ae's terminal never reaches it.
    DetachedIn(&'a Path),
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
    fn tagged(tag: &str) -> Option<Self> {
        let base = std::env::temp_dir();
        for attempt in 0..64_u32 {
            let path = base.join(format!("ae-{tag}.{}.{attempt}.json", std::process::id()));
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

    /// The opencode capture's: `ae-opencode.<pid>.<n>.json`.
    fn new() -> Option<Self> {
        Self::tagged("opencode")
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

/// The repository-selecting variables git reads before `-C`, as
/// `git rev-parse --local-env-vars` lists them (git 2.55.0): the git leg
/// scrubs every one so a query names its repository with `-C` alone.
const GIT_LOCAL_ENV_VARS: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

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

/// What the on-demand agy quota leg came back with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgyRun {
    /// `/quota`'s stdout, whole and UTF-8.
    Done { text: String },
    /// No `agy` on `PATH`.
    Absent,
    /// `--version` did not PROVE a release at or above the floor: `/quota` never ran.
    TooOld,
    /// `/quota` ran past its deadline and was killed.
    TimedOut,
    /// `/quota` failed, or wrote past 64 KiB or not UTF-8.
    Failed,
}

/// The deadlines of `--version` (0.27 s measured) and `/quota` (3-5 s
/// measured), agy 1.2.14 on 2026-10-01.
const AGY_DEADLINES: [Duration; 2] = [Duration::from_secs(3), Duration::from_secs(8)];

/// The agy leg of the one process door, program and argvs FIXED here, and with
/// Muse's one of the two legs that reach the network (each through its own
/// tool's login). An agy below
/// [`crate::quota::agy::FLOOR`] reads `/quota` as a prompt and starts an agent
/// turn, so `/quota` runs only once `--version` PROVED a release at or above it.
/// The child runs with `HOME=home`: the account asked is the one ae judged.
pub(crate) fn run_agy_quota(home: &Path) -> AgyRun {
    agy_quota_with(home, &[], AGY_DEADLINES)
}

/// [`run_agy_quota`] with the child's environment and deadlines passed in, so
/// the leg's own tests can put a fake first on the child's `PATH`. A HOME that
/// is not UTF-8 cannot be handed over, so agy is not started.
fn agy_quota_with(home: &Path, envs: &[(&str, &str)], deadlines: [Duration; 2]) -> AgyRun {
    let Some(home) = home.to_str() else {
        return AgyRun::Failed;
    };
    let envs = &[envs, &[("HOME", home)]].concat();
    let [version_deadline, quota_deadline] = deadlines;
    match spawn_until(
        "agy",
        &["--version"],
        envs,
        4096,
        version_deadline,
        None,
        None,
    ) {
        Ran::Absent => return AgyRun::Absent,
        Ran::Exited(exit, out) if exit.success() && crate::quota::agy::supports_quota(&out) => {}
        Ran::Exited(..) | Ran::TimedOut | Ran::Failed | Ran::Over => return AgyRun::TooOld,
    }
    let quota = ["-p", "/quota", "--output-format", "json"];
    match spawn_until("agy", &quota, envs, 65_536, quota_deadline, None, None) {
        Ran::Exited(exit, out) if exit.success() => {
            String::from_utf8(out).map_or(AgyRun::Failed, |text| AgyRun::Done { text })
        }
        Ran::TimedOut => AgyRun::TimedOut,
        Ran::Exited(..) | Ran::Absent | Ran::Failed | Ran::Over => AgyRun::Failed,
    }
}

/// The directory a Muse quota call runs in: new, empty, private (0700) and
/// UNDER the OS temp directory, removed when this guard drops.
///
/// Muse takes its working directory as its workspace and puts its own tool
/// output under `$TMPDIR`; a workspace that holds `$TMPDIR` is refused at start
/// (measured on muse 1.4.11, 2026-10-20), and an untrusted one keeps project
/// rules and skills out of a paid model call. A crash between mint and drop
/// leaves one `ae-muse-cwd.<pid>.<n>` directory behind; nothing sweeps it.
struct Workspace {
    path: PathBuf,
}

impl Workspace {
    /// Mint one. `DirBuilder` refuses a taken name, a planted link included,
    /// and a collision retries the next name.
    fn new() -> Option<Self> {
        use std::os::unix::fs::DirBuilderExt as _;
        let base = std::env::temp_dir();
        for attempt in 0..64_u32 {
            let path = base.join(format!("ae-muse-cwd.{}.{attempt}", std::process::id()));
            match std::fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Some(Self { path }),
                Err(why) if why.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return None,
            }
        }
        None
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// What the on-demand Muse quota leg came back with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MuseRun {
    /// `muse exec`'s stdout, whole and UTF-8.
    Done { text: String },
    /// No `muse` on `PATH`: nothing was asked, so nothing was spent.
    Absent,
    /// `muse exec` ran past its deadline and was killed.
    TimedOut,
    /// `muse exec` wrote past the output ceiling and was killed.
    TooLarge,
    /// `muse exec` exited by itself with this non-zero status. `text` is its
    /// stdout when that is UTF-8, else empty: the exit status always wins, and
    /// a failed run states its reason (a usage limit, with its reset) there.
    Exited { code: u8, text: String },
    /// `muse exec` exited cleanly and wrote output that is not UTF-8.
    NotUtf8,
    /// `muse exec` was not started, not read back or killed by a signal.
    Failed,
}

/// `muse exec` runs one ~32K-token model call on a shared bucket: 12.8 s, over 30 s and
/// 57 s measured (muse 1.4.11, 2026-10-20), so the deadline leaves room above the slowest.
const MUSE_DEADLINE: Duration = Duration::from_secs(90);

/// The most stdout the call is read from; 23 KB measured.
const MUSE_OUTPUT_CAP: u64 = 256 * 1024;

/// The cheapest call that makes muse report its account allowance: no session
/// log, no foreign personal context, the smallest reasoning effort. Fixed here,
/// so a caller chooses nothing.
const MUSE_ARGV: [&str; 9] = [
    "exec",
    "--json",
    "--preset",
    "empty",
    "--reasoning-effort",
    "minimal",
    "--no-session-log",
    "--no-foreign-personal-context",
    "Reply with exactly: ok",
];

/// The Muse leg of the one process door, program and argv FIXED here. It spends
/// one model call of the account, so its only caller sits behind the opt-in and
/// the held cache. The child runs with `HOME=home`: the account asked is the one
/// ae judged, in a [`Workspace`] of its own.
pub(crate) fn run_muse_quota(home: &Path) -> MuseRun {
    muse_quota_with(home, &[], MUSE_DEADLINE)
}

/// [`run_muse_quota`] with the child's environment and deadline passed in. A
/// HOME that is not UTF-8 cannot be handed over, so muse is not started.
fn muse_quota_with(home: &Path, envs: &[(&str, &str)], deadline: Duration) -> MuseRun {
    let Some(home) = home.to_str() else {
        return MuseRun::Failed;
    };
    let envs = &[envs, &[("HOME", home)]].concat();
    let Some(workspace) = Workspace::new() else {
        return MuseRun::Failed;
    };
    match spawn_until(
        "muse",
        &MUSE_ARGV,
        envs,
        MUSE_OUTPUT_CAP,
        deadline,
        None,
        Some(workspace.path()),
    ) {
        Ran::Exited(exit, out) if exit.success() => {
            String::from_utf8(out).map_or(MuseRun::NotUtf8, |text| MuseRun::Done { text })
        }
        Ran::Exited(exit, out) => {
            exit.code()
                .and_then(|code| u8::try_from(code).ok())
                .map_or(MuseRun::Failed, |code| MuseRun::Exited {
                    code,
                    text: String::from_utf8(out).unwrap_or_default(),
                })
        }
        Ran::TimedOut => MuseRun::TimedOut,
        Ran::Over => MuseRun::TooLarge,
        Ran::Absent => MuseRun::Absent,
        Ran::Failed => MuseRun::Failed,
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
/// returns at most `cap + 1` bytes, and what that one extra byte means belongs
/// to the caller — the board's reader refuses it with its budget reason, while
/// the id-capture leg has no cap check and simply finds no id (the seat stays
/// `pending`, never a wrong id).
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

/// The motion ticker's pane observation for `session` on `server`, as one
/// process of its own. Production reads it inside [`exchange_motion`]; tests
/// read it alone.
#[cfg(test)]
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

/// Watchdog start/stop's existing caller-owner read, with one failed command
/// re-asked. Kill proofs and other plain owner observations retain one read.
pub(crate) fn reask_pane_owner(
    server: &ServerId,
    pane: &str,
) -> Option<crate::watchdog_glue::PaneOwner> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = observation_read(
        &crate::watchdog_glue::pane_owner_args(server, pane),
        &mut true,
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

/// A live pane's cwd — TEST ONLY for the spawn-seam pins. `None` when the
/// query failed or answered empty; the CALLER hard-fails on `None`, because
/// an unobserved pane proves nothing about the seam.
#[cfg(test)]
#[must_use]
pub fn observe_pane_current_path(server: &ServerId, pane: &str) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::pane_current_path_args(server, pane));
    tmux::interpret_display_value(succeeded, &stdout)
}

/// `pane`'s `@ae_console` stamp, empty when it carries none, or `None` when
/// tmux did not answer.
#[must_use]
pub fn observe_pane_console(server: &ServerId, pane: &str) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::pane_console_args(server, pane));
    tmux::interpret_pane_console(succeeded, &stdout)
}

/// The session name and window index of `pane`, raw; `None` if tmux is silent.
#[must_use]
pub fn observe_pane_place(server: &ServerId, pane: &str) -> Option<(String, String)> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::pane_place_args(server, pane));
    let place = tmux::interpret_pane_place(succeeded, &stdout)?;
    if !tmux::is_view_session(&place.0) {
        return Some(place);
    }
    let home = home_of(server, &place.0, pane, &mut false)?;
    let (succeeded, stdout) = run(PROGRAM, &tmux::pane_place_args(server, &home.target));
    tmux::interpret_pane_place(succeeded, &stdout).filter(|(session, _)| *session == home.name)
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
    pane_session_observation(server, pane, &mut false)
}

/// The guarded target's physical-session observation, re-asking at most one
/// failed command, including the name and native-view legs.
#[must_use]
pub(crate) fn reask_pane_session_identity(
    server: &ServerId,
    pane: &str,
) -> Option<tmux::SessionIdentity> {
    pane_session_observation(server, pane, &mut true)
}

fn pane_session_observation(
    server: &ServerId,
    pane: &str,
    reask: &mut bool,
) -> Option<tmux::SessionIdentity> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) =
        observation_read(&tmux::pane_session_identity_args(server, pane), reask);
    let identity = tmux::interpret_pane_session_identity(succeeded, &stdout)?;
    // The format carries no name, so the session it answered for is named
    // by its own id before it is believed.
    let (succeeded, stdout) =
        observation_read(&tmux::session_name_of_args(server, &identity.id), reask);
    let session = tmux::interpret_session_option(succeeded, &stdout)?;
    if !tmux::is_view_session(&session) {
        return Some(identity);
    }
    let home = home_of(server, &session, pane, reask)?;
    let (succeeded, stdout) = observation_read(
        &tmux::pane_session_identity_args(server, &home.target),
        reask,
    );
    tmux::interpret_pane_session_identity(succeeded, &stdout).filter(|seen| seen.id == home.id)
}

/// Publish several option values in one tmux process.
#[must_use]
pub(crate) fn publish_options(server: &ServerId, writes: &[tmux::OptionWrite]) -> bool {
    if !addressable(server) || writes.is_empty() {
        return false;
    }
    run(PROGRAM, &tmux::set_options_args(server, writes)).0
}

/// What one verdict cycle's publication proved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CycleOutcome {
    /// The wake hook is installed on the batch's session.
    pub(crate) hook_proven: bool,
    /// Whether each operation landed, index for index with the batch's ops.
    pub(crate) landed: Vec<bool>,
}

/// Publish one verdict cycle's operations as ONE tmux process — or a few, only
/// when the batch passes tmux's command size ([`tmux::cycle_batch_chunks`]).
///
/// The wake hook comes first when the batch names a channel, followed by a
/// marker line: tmux drops the rest of a `;` group after an error, so the
/// marker on stdout is the proof that the installation ran, whatever happened
/// to the operations after it. An invocation that did not exit 0 is followed
/// by today's individual calls, one per operation, so one refused operation
/// (a pane that vanished since the cycle read it) cannot cost the others
/// their write. Every value is asserted again each cycle, so running an
/// already-landed operation again changes nothing.
#[must_use]
pub(crate) fn publish_cycle(server: &ServerId, batch: &tmux::CycleBatch) -> CycleOutcome {
    let channel = batch
        .channel
        .as_deref()
        .filter(|channel| tmux::is_wake_channel(channel));
    if !addressable(server) || (batch.ops.is_empty() && channel.is_none()) {
        return CycleOutcome {
            hook_proven: false,
            landed: vec![false; batch.ops.len()],
        };
    }
    let mut hook_proven = false;
    let mut landed = vec![false; batch.ops.len()];
    let chunks = tmux::cycle_batch_chunks(server, &batch.session, channel, &batch.ops);
    for (index, chunk) in chunks.into_iter().enumerate() {
        let (succeeded, stdout) = run(PROGRAM, &chunk.args);
        // The hook block only ever rides the FIRST invocation.
        if index == 0 && channel.is_some() {
            hook_proven = stdout.lines().any(|line| line == tmux::WAKE_HOOK_MARKER);
        }
        for op in chunk.ops {
            if let (Some(slot), Some(operation)) = (landed.get_mut(op), batch.ops.get(op)) {
                *slot = succeeded || publish_cycle_op(server, operation);
            }
        }
    }
    CycleOutcome {
        hook_proven,
        landed,
    }
}

/// One cycle operation as its own process, with the argv its call site used
/// before the cycle was batched.
fn publish_cycle_op(server: &ServerId, op: &tmux::CycleOp) -> bool {
    match op {
        tmux::CycleOp::Set(write) => publish_options(server, std::slice::from_ref(write)),
        tmux::CycleOp::Unset {
            scope,
            target,
            name,
        } => clear_option(server, *scope, target, name),
    }
}

/// How a wait for this session's attach ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Waited {
    /// The read before the wait saw a client on the session.
    Attached,
    /// The channel was signalled.
    Woke,
    /// The deadline passed; the waiter was released and reaped.
    TimedOut,
    /// The waiter did not start, or its read did not answer.
    Failed,
}

/// How often a waiter's output and exit are looked at.
const WAIT_POLL: Duration = Duration::from_millis(50);

/// How often a released waiter is looked at while it exits.
const RELEASE_POLL: Duration = Duration::from_millis(10);

/// How long a released waiter gets to exit before the backstop kills it.
const RELEASE_GRACE: Duration = Duration::from_secs(1);

/// The grace [`release_waiter`] gives: [`RELEASE_GRACE`], which only a test
/// can lengthen, for its own thread, so a loaded host's late exit is not the
/// backstop's kill.
#[cfg(not(test))]
const fn release_grace() -> Duration {
    RELEASE_GRACE
}

/// Wait until `session` on `server` has a client, `channel` is signalled, or
/// `deadline` passes: ONE tmux client, [`tmux::waiter_args`], whose read and
/// registration are one queued pass.
///
/// No waiter outlives the call. One that ends by itself has said everything:
/// `Attached` when its read counted a client, `Woke` when the channel was
/// signalled, `Failed` otherwise. One ae stops waiting for — a client counted
/// while it still waits, a malformed read, the deadline — is RELEASED with a
/// signal on its own channel and reaped: a killed `wait-for` client leaves its
/// queued item and a server descriptor behind until the channel's next signal.
pub(crate) fn run_tmux_wait(
    server: &ServerId,
    session: &str,
    channel: &str,
    deadline: std::time::Instant,
) -> Waited {
    if !addressable(server) || !tmux::is_wake_channel(channel) {
        return Waited::Failed;
    }
    let Some(scratch) = CaptureScratch::tagged("wait") else {
        return Waited::Failed;
    };
    let Ok(handle) = scratch.file().try_clone() else {
        return Waited::Failed;
    };
    // No pipe: stdout is a scratch file ae reads back by position.
    let Some(mut child) =
        start_wait_client(&tmux::waiter_args(server, session, channel), handle.into())
    else {
        return Waited::Failed;
    };
    loop {
        let Ok(exited) = child.try_wait() else {
            return release_waiter(server, channel, &mut child, Waited::Failed);
        };
        // Read after the exit check: a child seen exited has written all it
        // will write.
        let read = tmux::interpret_wait_read(&waiter_output(&scratch));
        if let Some(status) = exited {
            note_waiter_exit(child.id(), status);
            note_child_reaped(child.id(), status);
            return match read {
                tmux::WaitRead::Attached(count) if count > 0 => Waited::Attached,
                tmux::WaitRead::Attached(_) if status.success() => Waited::Woke,
                tmux::WaitRead::Attached(_)
                | tmux::WaitRead::Pending
                | tmux::WaitRead::Malformed => Waited::Failed,
            };
        }
        match read {
            tmux::WaitRead::Attached(count) if count > 0 => {
                return release_waiter(server, channel, &mut child, Waited::Attached);
            }
            tmux::WaitRead::Malformed => {
                return release_waiter(server, channel, &mut child, Waited::Failed);
            }
            tmux::WaitRead::Attached(_) | tmux::WaitRead::Pending => {}
        }
        let now = std::time::Instant::now();
        if now >= deadline {
            return release_waiter(server, channel, &mut child, Waited::TimedOut);
        }
        std::thread::sleep(WAIT_POLL.min(deadline.saturating_duration_since(now)));
    }
}

/// One tmux client of the wait leg, built by [`command_for`] like every child
/// and counted like every child: stdin and stderr closed, stdout where the
/// caller says.
fn start_wait_client(args: &[String], stdout: std::process::Stdio) -> Option<std::process::Child> {
    let mut command = command_for(PROGRAM, args, &[], true);
    note_spawn(command.get_program(), command.get_args());
    command.stdin(std::process::Stdio::null());
    command.stdout(stdout);
    command.stderr(std::process::Stdio::null());
    let child = command.spawn().ok();
    note_child_spawned(child.as_ref().map(std::process::Child::id));
    child
}

/// What a waiter printed so far, at most a few lines' worth.
///
/// Read by POSITION: the waiter writes through a clone of this handle, and a
/// clone shares the file offset, so a seek here would move where the waiter's
/// next write lands.
fn waiter_output(scratch: &CaptureScratch) -> String {
    use std::os::unix::fs::FileExt as _;
    const CAP: usize = 256;
    let mut stdout = vec![0_u8; CAP];
    let mut filled = 0;
    while let (Some(rest), Ok(offset)) = (stdout.get_mut(filled..), u64::try_from(filled)) {
        if rest.is_empty() {
            break;
        }
        match scratch.file().read_at(rest, offset) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(why) if why.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                filled = 0;
                break;
            }
        }
    }
    stdout.truncate(filled);
    String::from_utf8_lossy(&stdout).into_owned()
}

/// Release a waiter ae is done with, then reap it, and report `outcome`.
///
/// The signal ends the waiter's `wait-for`, so tmux frees its queued item. If
/// the waiter had already been woken, the signal is one stale wake for the
/// next waiter: an early return the interval loop re-arms past, never an early
/// cycle.
///
/// The release is a child of its own inside the same [`RELEASE_GRACE`], so a
/// server that stopped answering holds neither it nor the waiter past the
/// grace. Past it, the backstop kills and reaps whatever still runs, and only
/// THEN says so on stderr, best effort: a killed waiter may leave its item
/// behind. Every backstop reports `Failed`, whichever child it had to kill,
/// so the interval polls from there instead of trusting a stalled server.
fn release_waiter(
    server: &ServerId,
    channel: &str,
    waiter: &mut std::process::Child,
    outcome: Waited,
) -> Waited {
    use std::io::Write as _;
    let mut release = start_wait_client(
        &tmux::wake_release_args(server, channel),
        std::process::Stdio::null(),
    );
    let grace = std::time::Instant::now() + release_grace();
    let mut reaped = false;
    loop {
        if !reaped {
            match waiter.try_wait() {
                Ok(Some(status)) => {
                    note_waiter_exit(waiter.id(), status);
                    note_child_reaped(waiter.id(), status);
                    reaped = true;
                }
                Ok(None) => {}
                Err(_) => break,
            }
        }
        if let Some(client) = release.as_mut() {
            match client.try_wait() {
                Ok(Some(status)) => {
                    note_child_reaped(client.id(), status);
                    release = None;
                }
                Ok(None) => {}
                Err(_) => break,
            }
        }
        if reaped && release.is_none() {
            return outcome;
        }
        if std::time::Instant::now() >= grace {
            break;
        }
        std::thread::sleep(RELEASE_POLL);
    }
    // The mandatory cleanup first, each step's result kept for the line below.
    let mut cleaned = true;
    if !reaped {
        cleaned &= waiter.kill().is_ok();
        match waiter.wait() {
            Ok(status) => {
                note_waiter_exit(waiter.id(), status);
                note_child_reaped(waiter.id(), status);
            }
            Err(_) => cleaned = false,
        }
    }
    if let Some(mut client) = release {
        cleaned &= client.kill().is_ok();
        let reap = client.wait();
        if let Ok(status) = reap {
            note_child_reaped(client.id(), status);
        }
        cleaned &= reap.is_ok();
    }
    let what = if reaped { "release" } else { "waiter" };
    let done = if cleaned {
        "killed and reaped"
    } else {
        "the kill or the reap failed"
    };
    let _ = writeln!(
        std::io::stderr(),
        "ae: watchdog: the {what} on {channel} did not exit within {}s of the release; {done}",
        release_grace().as_secs()
    );
    Waited::Failed
}

/// One plain `wait-for <channel>` client, held across several waits: a wait
/// that times out leaves it waiting for the next one, so no release can race
/// a signal that is already on its way.
#[derive(Debug)]
pub(crate) struct ChannelWaiter {
    channel: String,
    child: std::process::Child,
}

impl ChannelWaiter {
    /// Start waiting for `channel` on `server`; `None` when the waiter could
    /// not start.
    pub(crate) fn start(server: &ServerId, channel: &str) -> Option<Self> {
        if !addressable(server) || !tmux::is_wake_channel(channel) {
            return None;
        }
        let child = start_wait_client(
            &tmux::channel_waiter_args(server, channel),
            std::process::Stdio::null(),
        )?;
        Some(Self {
            channel: channel.to_owned(),
            child,
        })
    }

    /// Wait until the channel is signalled (`Woke`), `deadline` passes
    /// (`TimedOut`, still waiting), or the client ended without a signal
    /// (`Failed`). After `Woke` or `Failed` the client is reaped.
    pub(crate) fn wait_until(&mut self, deadline: std::time::Instant) -> Waited {
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    note_waiter_exit(self.child.id(), status);
                    note_child_reaped(self.child.id(), status);
                    return if status.success() {
                        Waited::Woke
                    } else {
                        Waited::Failed
                    };
                }
                Ok(None) => {}
                Err(_) => return Waited::Failed,
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return Waited::TimedOut;
            }
            std::thread::sleep(WAIT_POLL.min(deadline.saturating_duration_since(now)));
        }
    }

    /// End a waiter that is still waiting: a signal on its own channel, then
    /// the reap ([`release_waiter`], whose backstop kill is only for a server
    /// that stopped answering).
    pub(crate) fn release(mut self, server: &ServerId) {
        let _ = release_waiter(server, &self.channel, &mut self.child, Waited::TimedOut);
    }
}

/// Signal `channel` on `server` once: `wait-for -S`, which wakes every waiter
/// on it, or is kept for the next waiter when none waits.
#[must_use]
pub(crate) fn signal_channel(server: &ServerId, channel: &str) -> bool {
    addressable(server)
        && tmux::is_wake_channel(channel)
        && run(PROGRAM, &tmux::wake_release_args(server, channel)).0
}

/// The ownership proofs one verdict cycle reads, in the order asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PeerOwnershipRead {
    /// The socket path this server answered with, when the read asked it.
    pub(crate) socket: Option<String>,
    /// Every requested peer, with its proof or `None`.
    pub(crate) peers: Vec<(String, Option<SessionOwnership>)>,
}

/// Read the ownership pair of every peer in `peers` on `server`, fresh, with
/// this server's socket path riding along: ONE process for a fleet whose peers
/// all answer.
///
/// Each answer is today's per-peer read ([`observe_session_ownership`]). A peer
/// whose read fails stops tmux's queued list there and is `None`, exactly as
/// before; one more invocation resumes with the next peer, so a vanished peer
/// costs one process and never another peer's proof. No peers asks nothing,
/// and the socket is then asked only where it is needed, as before.
#[must_use]
pub(crate) fn observe_peer_ownership(server: &ServerId, peers: &[String]) -> PeerOwnershipRead {
    let mut read = PeerOwnershipRead {
        socket: None,
        peers: Vec::with_capacity(peers.len()),
    };
    if !addressable(server) {
        read.peers = peers.iter().map(|peer| (peer.clone(), None)).collect();
        return read;
    }
    let nonce = read_nonce();
    let mut socket = !peers.is_empty();
    while read.peers.len() < peers.len() {
        let rest = peers.get(read.peers.len()..).unwrap_or_default();
        let query = tmux::ownership_read_args(server, &nonce, socket, rest);
        let (_, stdout) = run(PROGRAM, &query.args);
        let answer = tmux::interpret_ownership_read(&stdout, &nonce, socket, query.peers);
        if socket {
            read.socket = answer.socket;
            socket = false;
        } else if answer.peers.is_empty() {
            // Unreachable by construction — a read without the socket section
            // always decides its first peer — but a loop must not hang on it.
            break;
        }
        for (peer, proof) in rest.iter().zip(answer.peers) {
            read.peers.push((
                peer.clone(),
                proof.map(|(marker, home)| SessionOwnership { marker, home }),
            ));
        }
    }
    for peer in peers.get(read.peers.len()..).unwrap_or_default() {
        read.peers.push((peer.clone(), None));
    }
    read
}

/// The token the completion markers of one queued read carry: 64 random bits
/// from [`crate::entropy`], fresh per read. A value set before the read could
/// print a line that passes for a marker only by guessing them, a 1 in 2^64
/// chance per read; any other line is only an extra line in its own section,
/// which the one-line rule refuses.
fn read_nonce() -> String {
    format!("{:016x}", crate::entropy())
}

/// What one motion exchange observed and published, kept apart: a reading
/// that completed stands even when the writes after it were refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MotionExchange {
    /// The pane and fleet listings, or `None` when either did not answer.
    pub(crate) reading: Option<(Vec<tmux::MotionPane>, Vec<tmux::FleetListingRow>)>,
    /// `None` when there was nothing to write; else whether the writes landed.
    pub(crate) published: Option<bool>,
}

/// The motion ticker's I/O: read `session`'s panes and the fleet when
/// `observe`, and publish `writes` — ONE process for both.
///
/// The reads are today's two listings, the motion `list-panes` and
/// [`observe_fleet_listing`]'s `list-sessions`, queued with their completion
/// markers ahead of
/// the writes ([`tmux::motion_exchange_args`]). A read that fails ends tmux's
/// list there, so no write lands past a missing read, and a write that is
/// refused leaves the completed reading standing. Writes too large to ride
/// along go as their own process, and only once the reads ran. Without
/// `observe`, the writes alone are one process, as before.
#[must_use]
pub(crate) fn exchange_motion(
    server: &ServerId,
    session: &str,
    observe: bool,
    writes: &[tmux::OptionWrite],
) -> MotionExchange {
    let write = !writes.is_empty();
    if !observe {
        let published = write.then(|| publish_options(server, writes));
        return MotionExchange {
            reading: None,
            published,
        };
    }
    if !addressable(server) {
        return MotionExchange {
            reading: None,
            published: write.then_some(false),
        };
    }
    let nonce = read_nonce();
    let query = tmux::motion_exchange_args(server, session, &nonce, writes);
    let (succeeded, stdout) = run(PROGRAM, &query.args);
    let reading = tmux::interpret_motion_exchange(&stdout, &nonce);
    let published = write.then(|| {
        if query.writes {
            succeeded
        } else {
            succeeded && publish_options(server, writes)
        }
    });
    MotionExchange { reading, published }
}

/// `session`'s look and then each session option of `options`, fresh, in ONE
/// process ([`tmux::session_look_read_args`]).
///
/// Each answer is today's separate read ([`observe_look`],
/// [`observe_session_option`]). A read that fails ends tmux's queued list
/// there, so it and every later answer are `None`, as a failed separate read
/// was.
#[must_use]
pub(crate) fn observe_session_look(
    server: &ServerId,
    session: &str,
    options: &[&str],
) -> tmux::SessionLookRead {
    if !addressable(server) {
        return tmux::SessionLookRead {
            look: None,
            options: vec![None; options.len()],
        };
    }
    let nonce = read_nonce();
    let (_, stdout) = run(
        PROGRAM,
        &tmux::session_look_read_args(server, session, &nonce, options),
    );
    tmux::interpret_session_look_read(&stdout, &nonce, options.len())
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

/// Select a proven `/open` target inside its own session, guarded again by
/// tmux at the select: whether tmux ran it and what it printed, or `None` when
/// a fact of the target fails its grammar and nothing ran.
#[must_use]
pub fn open_seat(
    server: &ServerId,
    target: &crate::console::open::Target,
) -> Option<(bool, String)> {
    open_seat_via(server, target, None)
}

/// [`open_seat`], handing `client` to the target's session in the same guarded
/// command when one is named.
#[must_use]
pub fn open_seat_via(
    server: &ServerId,
    target: &crate::console::open::Target,
    client: Option<&str>,
) -> Option<(bool, String)> {
    let args = crate::console::open::select_args_via(server, target, client)?;
    if !addressable(server) {
        return Some((false, String::new()));
    }
    Some(run(PROGRAM, &args))
}

/// One tmux leg of the orchestrator jump (`crate::jump`, which builds every
/// argv it passes): whether tmux ran it and what it printed.
#[must_use]
pub(crate) fn run_jump(server: &ServerId, args: &[String]) -> (bool, String) {
    if !addressable(server) {
        return (false, String::new());
    }
    run(PROGRAM, args)
}

/// The screen of a proven target as plain text: whether tmux ran the capture and
/// what it printed, or `None` when the pane fails its grammar and nothing ran.
#[must_use]
pub fn capture_seat(
    server: &ServerId,
    target: &crate::console::open::Target,
    lines: u32,
) -> Option<(bool, String)> {
    let args = crate::console::open::capture_argv(server, target, lines)?;
    if !addressable(server) {
        return Some((false, String::new()));
    }
    Some(run(PROGRAM, &args))
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
    let name = tmux::interpret_session_option(succeeded, &stdout)?;
    if !tmux::is_view_session(&name) {
        return Some(name);
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::view_of_args(server, &name));
    let id = succeeded.then(|| tmux::view_of_value(&stdout)).flatten()?;
    let (succeeded, stdout) = run(PROGRAM, &tmux::session_name_of_args(server, &id));
    tmux::interpret_session_option(succeeded, &stdout).filter(|name| !tmux::is_view_session(name))
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

/// The zone offset `session`'s server formats its clock in, as it answered, or
/// `None` when it did not answer.
#[must_use]
pub fn observe_zone(server: &ServerId, session: &str) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::zone_args(server, session));
    tmux::interpret_zone(succeeded, &stdout)
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

/// Hand this terminal to a tmux client attached to the view session `view`
/// on `pane` of the proven `session_id`, marked as the view `token` owns, and
/// report what tmux exited with once the human returns: `None` when it did
/// not run, or a signal ended it.
#[must_use]
pub fn view_pane(
    server: &ServerId,
    session_id: &str,
    view: &str,
    pane: &str,
    token: &str,
) -> Option<i32> {
    if !addressable(server) {
        return None;
    }
    let args = tmux::view_attach_args(server, session_id, view, pane, token);
    spawn(PROGRAM, &args, &[], Streams::Terminal, None)?
        .status
        .code()
}

/// Assert the pane view's return keys on `server`; whether tmux took both.
/// An ambient server is the user's own tmux, whose root table ae never
/// writes: refused, nothing run.
#[must_use]
pub fn bind_view_return(server: &ServerId) -> bool {
    !matches!(server, ServerId::Ambient)
        && tmux::VIEW_RETURN_KEYS
            .iter()
            .all(|key| write_run(server, &tmux::view_return_bind_args(server, key)))
}

/// Create the view session `view` in `session_id`'s group; whether tmux took it.
#[must_use]
pub fn make_view(
    server: &ServerId,
    session_id: &str,
    view: &str,
    session: &str,
    label: &str,
    uuid: &str,
) -> bool {
    let line = crate::theme::view_status_format(tmux::VIEW_HINT);
    let group = tmux::view_group_args(server, session_id, view, session, label, &line);
    tmux::with_view_uuid(group, view, uuid).is_some_and(|args| write_run(server, &args))
}

/// Move the proven viewer `client` into the view session — the inside-tmux
/// view; whether tmux took the whole chain.
#[must_use]
pub fn switch_view(
    server: &ServerId,
    session_id: &str,
    view: &str,
    pane: &str,
    client: &str,
    token: &str,
    ret: &str,
) -> bool {
    write_run(
        server,
        &tmux::view_switch_args(server, session_id, view, pane, client, token, ret),
    )
}

/// The session, window and pane one `client` shows, or `None`.
#[must_use]
pub fn observe_client_place(server: &ServerId, client: &str) -> Option<(String, String, String)> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::view_client_place_args(server, client));
    tmux::interpret_view_client_place(succeeded, &stdout)
}

/// Destroy exactly the view session `view`; whether tmux took it.
#[must_use]
pub fn destroy_view(server: &ServerId, view: &str) -> bool {
    write_run(server, &tmux::view_destroy_args(server, view))
}

/// What a stop found about the pane views of the session it stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopViews {
    /// No view holds the session's windows.
    Clear,
    /// These view sessions still do, after every kill ae tried.
    Held(Vec<String>),
    /// tmux did not answer in a shape ae can judge.
    Unreadable,
}

/// How long a kill waits before re-asking a verify that landed mid-exit —
/// one scheduling breath, long enough for an exiting server to settle into
/// an answer the strict proof can read.
pub const KILL_VERIFY_SETTLE: Duration = Duration::from_millis(100);

/// What one listing a stop reads said.
enum ViewRead<T> {
    Rows(T),
    /// The listing failed with the clean-exit diagnostic — the strict proof's
    /// own evidence that the server, and with it every session, is gone.
    Gone,
    Unreadable,
}

/// Run one listing a stop reads and judge it. A failure is judged by its own
/// diagnostic, exactly as [`verify_session_absent`] judges one: a missing
/// socket proves nothing. A failure that proves nothing is asked again ONCE
/// past [`KILL_VERIFY_SETTLE`], because a read landing while the last kill's
/// server is mid-exit answers `server exited unexpectedly`.
fn read_views<T>(args: &[String], interpret: impl Fn(bool, &str) -> Option<T>) -> ViewRead<T> {
    for attempt in 0..2 {
        if attempt > 0 {
            std::thread::sleep(KILL_VERIFY_SETTLE);
        }
        let (succeeded, stdout, stderr) = run_captured(PROGRAM, args);
        if let Some(rows) = interpret(succeeded, &stdout) {
            return ViewRead::Rows(rows);
        }
        if succeeded {
            return ViewRead::Unreadable;
        }
        if tmux::read_failure(&stderr) == tmux::Absence::Proven {
            return ViewRead::Gone;
        }
    }
    ViewRead::Unreadable
}

/// A view listing's rows: `None` when tmux did not answer in a shape ae can
/// judge, and no rows when the server is proven gone — the last session's
/// kill ends the server, and with it every view.
fn view_rows<T>(
    args: &[String],
    interpret: impl Fn(bool, &str) -> Option<Vec<T>>,
) -> Option<Vec<T>> {
    match read_views(args, interpret) {
        ViewRead::Rows(rows) => Some(rows),
        ViewRead::Gone => Some(Vec::new()),
        ViewRead::Unreadable => None,
    }
}

/// Kill the views among `holders`, then read again with `holding`: the views
/// still holding, or `None` when a read failed.
fn sweep_views(
    server: &ServerId,
    holders: Vec<String>,
    holding: impl Fn() -> Option<Vec<String>>,
) -> Option<Vec<String>> {
    if holders.is_empty() {
        return Some(holders);
    }
    for view in &holders {
        let _ = run(PROGRAM, &tmux::kill_session_args(server, view));
    }
    holding()
}

/// Stop the real session `session_id` so that no pane view outlives it
/// holding its windows: views holding a window of it, or recorded as views of
/// it, die BEFORE it, the real kill is today's exact kill, and views that
/// appeared meanwhile die after it. A snapshot or listing ae cannot read
/// kills nothing; a view that survives the first sweep keeps the real session
/// alive too.
#[must_use]
pub fn stop_views(server: &ServerId, session_id: &str) -> StopViews {
    /// How many times a view still holding the dead session's windows is
    /// killed again before the stop reports it.
    const ROUNDS: usize = 3;
    if !addressable(server) {
        return StopViews::Unreadable;
    }
    let windows = match read_views(
        &tmux::view_window_snapshot_args(server, session_id),
        tmux::interpret_window_snapshot,
    ) {
        ViewRead::Rows(windows) => windows,
        // No server, no session and no view: nothing is left to stop.
        ViewRead::Gone => return StopViews::Clear,
        ViewRead::Unreadable => return StopViews::Unreadable,
    };
    let holding = || {
        let rows = view_rows(
            &tmux::view_holder_listing_args(server),
            tmux::interpret_view_holders,
        )?;
        Some(tmux::views_holding(&windows, session_id, &rows))
    };
    let Some(before) = holding() else {
        return StopViews::Unreadable;
    };
    match sweep_views(server, before, holding) {
        Some(left) if left.is_empty() => {}
        Some(left) => return StopViews::Held(left),
        None => return StopViews::Unreadable,
    }
    let _ = run(PROGRAM, &tmux::kill_session_args(server, session_id));
    let Some(mut left) = holding() else {
        return StopViews::Unreadable;
    };
    for _ in 0..ROUNDS {
        if left.is_empty() {
            return StopViews::Clear;
        }
        left = match sweep_views(server, left, holding) {
            Some(left) => left,
            None => return StopViews::Unreadable,
        };
    }
    if left.is_empty() {
        StopViews::Clear
    } else {
        StopViews::Held(left)
    }
}

/// Kill the pane views recorded as views of the ae session `uuid` and say
/// whether any is left — for a stop that found the real session already gone,
/// whose windows only such a view can still hold.
#[must_use]
pub fn orphan_views(server: &ServerId, uuid: &str) -> StopViews {
    if !addressable(server) {
        return StopViews::Unreadable;
    }
    let holding = || {
        let rows = view_rows(
            &tmux::view_uuid_listing_args(server),
            tmux::interpret_view_uuids,
        )?;
        Some(tmux::orphan_view_ids(uuid, &rows))
    };
    let Some(before) = holding() else {
        return StopViews::Unreadable;
    };
    match sweep_views(server, before, holding) {
        Some(left) if left.is_empty() => StopViews::Clear,
        Some(left) => StopViews::Held(left),
        None => StopViews::Unreadable,
    }
}

/// Every session's id, attachment and name, or `None` when tmux did not
/// answer in that shape.
#[must_use]
pub fn observe_view_census(server: &ServerId) -> Option<Vec<tmux::ViewCensusRow>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::view_census_args(server));
    tmux::interpret_view_census(succeeded, &stdout)
}

/// Run one view `command` against `session_id`; whether tmux took it.
#[must_use]
pub fn run_view_command(server: &ServerId, session_id: &str, command: &str) -> bool {
    write_run(
        server,
        &tmux::view_command_args(server, session_id, command),
    )
}

/// `session_id`'s view marker and current window, or `None`.
#[must_use]
pub fn observe_view_marker(
    server: &ServerId,
    session_id: &str,
) -> Option<(String, String, String)> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::view_marker_args(server, session_id));
    tmux::interpret_view_marker(succeeded, &stdout)
}

/// The window and active pane `target` resolves to, or `None`.
#[must_use]
pub fn observe_view_place(server: &ServerId, target: &str) -> Option<(String, String)> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::view_place_args(server, target));
    tmux::interpret_view_place(succeeded, &stdout)
}

/// Move one exact client to one exact session, or report tmux's refusal.
#[must_use]
pub fn switch_client(server: &ServerId, client: &str, session: &str) -> bool {
    addressable(server) && run(PROGRAM, &tmux::switch_client_args(server, client, session)).0
}

/// Run `argv` as one background tmux `run-shell` job on `server`, as a picker
/// row does; whether tmux took it, not whether the job later succeeds.
#[must_use]
pub fn run_shell_background(server: &ServerId, argv: &[String]) -> bool {
    addressable(server) && run(PROGRAM, &tmux::run_shell_background_args(server, argv)).0
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

/// `pane`'s size and cursor row, or `None` when tmux did not answer.
#[must_use]
pub fn observe_pane_size(server: &ServerId, pane: &str) -> Option<tmux::PaneSize> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::pane_size_args(server, pane));
    tmux::interpret_pane_size(succeeded, &stdout)
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

/// Load `bytes` into `server`'s default paste buffer on STDIN, and into
/// `client`'s terminal clipboard when one is named. The error is tmux's first
/// stderr line, or that it did not run.
///
/// # Errors
///
/// Why tmux refused the load.
pub fn copy_buffer(server: &ServerId, client: Option<&str>, bytes: &[u8]) -> Result<(), String> {
    const DID_NOT_RUN: &str = "tmux did not run";
    if !addressable(server) {
        return Err(DID_NOT_RUN.to_owned());
    }
    let args = crate::console::open::copy_args(server, client);
    let output = spawn(PROGRAM, &args, &[], Streams::Captured, Some(bytes))
        .ok_or_else(|| DID_NOT_RUN.to_owned())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(stderr
        .lines()
        .next()
        .unwrap_or("tmux refused it")
        .to_owned())
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
    if let Some((query, required)) = argv.if_missing_line() {
        let (ok, output) = run_tmux_op(&query);
        if !ok {
            return (false, output);
        }
        if output.lines().any(|line| line.trim() == required) {
            return (true, String::new());
        }
        // Accepted race: two simultaneous first assertions may append one
        // duplicate row. It is harmless; later reads see it and stop growth.
    }
    match spawn(PROGRAM, argv.as_args(), &[], Streams::Captured, None) {
        Some(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ),
        None => (false, String::new()),
    }
}

/// Make `server` refuse `kill-server` (every spelling), when it is the server
/// the lock guards ([`crate::session_tmux::kill_lock_guards`]): read the alias
/// listing through the one process door, then write what
/// [`crate::session_tmux::kill_lock_writes`] plans. A server the lock does not
/// guard, or one that does not answer, is left alone.
pub(crate) fn assert_kill_lock(server: &ServerId) {
    let mut sockets = crate::fleet_read::SocketPaths::live();
    if !crate::session_tmux::kill_lock_guards(server, &mut sockets) {
        return;
    }
    let Some(query) = crate::session_tmux::kill_lock_query(server) else {
        return;
    };
    let (answered, listing) = run_tmux_op(&query);
    if !answered {
        return;
    }
    for write in crate::session_tmux::kill_lock_writes(server, &listing) {
        let _ = run_tmux_op(&write);
    }
}

/// The `!` leg's popup: `args` from `console::shell::popup_args`, started
/// DETACHED because `display-popup` holds its caller until the popup closes.
/// Whether tmux started; what the popup runs is the human's own line.
#[must_use]
pub(crate) fn open_popup(server: &ServerId, args: &[String]) -> bool {
    addressable(server) && spawn(PROGRAM, args, &[], Streams::Detached, None).is_some()
}

/// The `!` leg outside tmux: the human's own line, one argument to a FIXED
/// `/bin/sh -c`, started in `dir` with null streams and never waited for. ae
/// writes no script text and reads no `$SHELL`. Whether it started.
#[must_use]
pub(crate) fn run_shell_line(argv: &crate::console::shell::ShellArgv, dir: &Path) -> bool {
    let streams = Streams::DetachedIn(dir);
    spawn("/bin/sh", argv.as_args(), &[], streams, None).is_some()
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

// Insert before the tests module in src/transport.rs. Acceptance support only.
#[cfg(test)]
pub(crate) mod fixture_tmux {
    use crate::inventory::ServerId;
    use crate::meta::Selector;

    #[derive(Debug)]
    pub(crate) struct Read {
        pub(crate) ok: bool,
        pub(crate) stdout: String,
        pub(crate) stderr: String,
    }

    impl Read {
        pub(crate) fn proven(&self) -> bool {
            !self.ok && crate::tmux::read_failure(&self.stderr) == crate::tmux::Absence::Proven
        }
    }

    fn run(server: &ServerId, words: &[&str]) -> Read {
        if !matches!(server, ServerId::Selected(Selector::Socket(_))) {
            return Read {
                ok: false,
                stdout: String::new(),
                stderr: "fixture operation requires an owned socket".to_owned(),
            };
        }
        let mut args = crate::tmux::server_args(server);
        args.extend(words.iter().map(|word| (*word).to_owned()));
        let (ok, stdout, stderr) = super::run_captured(super::PROGRAM, &args);
        Read { ok, stdout, stderr }
    }

    pub(crate) fn end(server: &ServerId) -> Read {
        run(server, &["kill-server"])
    }

    // Rescue only. Every server in these acceptance arms starts exactly api.
    pub(crate) fn end_api_session(server: &ServerId) -> Read {
        run(server, &["kill-session", "-t", "=api"])
    }

    pub(crate) fn exit_empty(server: &ServerId, value: &str) -> Read {
        run(server, &["set-option", "-s", "exit-empty", value])
    }

    pub(crate) fn read_exit_empty(server: &ServerId) -> Read {
        run(server, &["show-options", "-sv", "exit-empty"])
    }

    pub(crate) fn probe(server: &ServerId) -> Read {
        run(server, &["list-sessions"])
    }

    pub(crate) fn unrefuse(server: &ServerId, index: u32) -> Read {
        run(
            server,
            &["set-option", "-su", &format!("command-alias[{index}]")],
        )
    }
}

// The quiet idle acceptance. Its reap checks read the starts and reaps of the
// measured call's own thread ([`tests::wait_children_during`]).
#[cfg(test)]
pub(crate) mod quietwd_spec;

#[cfg(test)]
mod paneerr_spec;

#[cfg(test)]
mod tests {
    thread_local! {
        static SPAWNS: std::cell::RefCell<Option<Vec<Vec<String>>>> =
            const { std::cell::RefCell::new(None) };
        static WAITER_EXITS: std::cell::RefCell<Option<Vec<(u32, std::process::ExitStatus)>>> =
            const { std::cell::RefCell::new(None) };
        static WAIT_CHILDREN: std::cell::RefCell<Option<Vec<WaitChildEvent>>> =
            const { std::cell::RefCell::new(None) };
        static RELEASE_GRACE_HELD: std::cell::Cell<Option<std::time::Duration>> =
            const { std::cell::Cell::new(None) };
    }

    /// The grace [`super::release_waiter`] gives on this thread: the one a
    /// test holds with [`hold_release_grace`], else [`super::RELEASE_GRACE`].
    pub(crate) fn release_grace() -> std::time::Duration {
        RELEASE_GRACE_HELD
            .with(std::cell::Cell::get)
            .unwrap_or(super::RELEASE_GRACE)
    }

    /// Gives back the grace this thread had before [`hold_release_grace`].
    pub(crate) struct HeldReleaseGrace {
        prior: Option<std::time::Duration>,
    }

    impl Drop for HeldReleaseGrace {
        fn drop(&mut self) {
            RELEASE_GRACE_HELD.with(|held| held.set(self.prior));
        }
    }

    /// Let a released waiter on this thread take `grace` to exit before the
    /// backstop kills it, until the guard drops. The backstop itself is not
    /// replaced: a waiter that never exits is still killed, after `grace`.
    pub(crate) fn hold_release_grace(grace: std::time::Duration) -> HeldReleaseGrace {
        HeldReleaseGrace {
            prior: RELEASE_GRACE_HELD.with(|held| held.replace(Some(grace))),
        }
    }

    /// One step in the life of a child this thread starts inside
    /// [`wait_children_during`], in the order it happened.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum WaitChildEvent {
        /// [`note_spawn`] saw a start of any program, before the outcome.
        Attempted,
        /// The start succeeded: the actual child's pid.
        Spawned(u32),
        /// The start failed: no child exists.
        SpawnFailed,
        /// A `wait` or `try_wait` returned this status for the child.
        Reaped(u32, std::process::ExitStatus),
    }

    fn note_wait_child(event: WaitChildEvent) {
        WAIT_CHILDREN.with(|seen| {
            if let Some(seen) = seen.borrow_mut().as_mut() {
                seen.push(event);
            }
        });
    }

    /// Record whether a wait-leg start succeeded, with the child's pid.
    pub(crate) fn note_child_spawned(pid: Option<u32>) {
        note_wait_child(pid.map_or(WaitChildEvent::SpawnFailed, WaitChildEvent::Spawned));
    }

    /// Record one completed reap of a wait-leg child.
    pub(crate) fn note_child_reaped(pid: u32, status: std::process::ExitStatus) {
        note_wait_child(WaitChildEvent::Reaped(pid, status));
    }

    /// Run `body`, returning what it returned and every start and reap this
    /// thread made meanwhile. The trace is complete when `body` returns: a
    /// later start or reap is never added to it. Observes only.
    pub(crate) fn wait_children_during<T>(body: impl FnOnce() -> T) -> (T, Vec<WaitChildEvent>) {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                WAIT_CHILDREN.with(|seen| seen.borrow_mut().take());
            }
        }
        WAIT_CHILDREN.with(|seen| *seen.borrow_mut() = Some(Vec::new()));
        let reset = Reset;
        let out = body();
        let seen = WAIT_CHILDREN.with(|seen| seen.borrow_mut().take().unwrap_or_default());
        drop(reset);
        (out, seen)
    }

    /// Record how one waiter of [`super::run_tmux_wait`] ended: its pid and the
    /// status its reap returned, when this thread is inside
    /// [`waiter_exits_during`]. The release client is not a waiter.
    pub(crate) fn note_waiter_exit(pid: u32, status: std::process::ExitStatus) {
        WAITER_EXITS.with(|seen| {
            if let Some(seen) = seen.borrow_mut().as_mut() {
                seen.push((pid, status));
            }
        });
    }

    /// Run `body`, returning what it returned and every waiter this thread's
    /// [`super::run_tmux_wait`] reaped meanwhile, with the status the reap
    /// returned. Observes only: nothing is replaced.
    pub(crate) fn waiter_exits_during<T>(
        body: impl FnOnce() -> T,
    ) -> (T, Vec<(u32, std::process::ExitStatus)>) {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                WAITER_EXITS.with(|seen| seen.borrow_mut().take());
            }
        }
        WAITER_EXITS.with(|seen| *seen.borrow_mut() = Some(Vec::new()));
        let reset = Reset;
        let out = body();
        let seen = WAITER_EXITS.with(|seen| seen.borrow_mut().take().unwrap_or_default());
        drop(reset);
        (out, seen)
    }

    /// Record one child [`super::spawn`] is about to start, its built argv
    /// (`-u` included), when this thread is inside [`spawns_during`].
    pub(crate) fn note_spawn<'a>(
        program: &std::ffi::OsStr,
        args: impl Iterator<Item = &'a std::ffi::OsStr>,
    ) {
        note_wait_child(WaitChildEvent::Attempted);
        SPAWNS.with(|seen| {
            if let Some(seen) = seen.borrow_mut().as_mut() {
                let mut words = vec![program.to_string_lossy().into_owned()];
                words.extend(args.map(|arg| arg.to_string_lossy().into_owned()));
                seen.push(words);
            }
        });
    }

    /// Run `body`, returning what it returned and every child this thread's
    /// [`super::spawn`] started meanwhile. Observes only: nothing is replaced.
    pub(crate) fn spawns_during<T>(body: impl FnOnce() -> T) -> (T, Vec<Vec<String>>) {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                SPAWNS.with(|seen| seen.borrow_mut().take());
            }
        }
        SPAWNS.with(|seen| *seen.borrow_mut() = Some(Vec::new()));
        let reset = Reset;
        let out = body();
        let seen = SPAWNS.with(|seen| seen.borrow_mut().take().unwrap_or_default());
        drop(reset);
        (out, seen)
    }

    /// What a finished wait left behind, judged by the children it started:
    /// a start no reap closed, and a started pid still in `table`. A row the
    /// wait never started is not its own, such as a child that was already
    /// exiting when the process-table read itself was started.
    fn wait_left_behind(events: &[WaitChildEvent], table: &[crate::procs::Proc]) -> Vec<String> {
        let mut left = Vec::new();
        for event in events {
            let WaitChildEvent::Spawned(pid) = event else {
                continue;
            };
            if !events
                .iter()
                .any(|seen| matches!(seen, WaitChildEvent::Reaped(reaped, _) if reaped == pid))
            {
                left.push(format!("pid {pid} was started and never reaped"));
            }
            if let Some(row) = table.iter().find(|row| row.pid == *pid) {
                left.push(format!("pid {pid} is still in the process table: {row:?}"));
            }
        }
        left
    }

    /// Fail unless the wait left nothing behind: the waiter's start is in
    /// the trace, and [`wait_left_behind`] finds nothing in the live table.
    fn assert_wait_left_nothing(waiter: u32, events: &[WaitChildEvent]) {
        assert!(
            events.contains(&WaitChildEvent::Spawned(waiter)),
            "the trace lacks the waiter's start: {events:?}"
        );
        let table = crate::procs::snapshot().expect("process table");
        let left = wait_left_behind(events, &table);
        assert!(left.is_empty(), "a child outlived the wait: {left:?}");
    }

    fn exited(code: i32) -> std::process::ExitStatus {
        std::os::unix::process::ExitStatusExt::from_raw(code)
    }

    fn row(pid: u32, parent: u32, comm: &str) -> crate::procs::Proc {
        crate::procs::Proc {
            pid,
            ppid: parent,
            comm: comm.to_owned(),
        }
    }

    #[test]
    fn a_row_the_wait_never_started_is_not_a_leftover_of_the_wait() {
        // The shape the stalled-server tests met on macOS under load: the wait
        // started and reaped two children (the waiter and its release), and the
        // table read caught the test's own `ps` and three exiting children
        // that carry the test process's name in parentheses.
        let own = std::process::id();
        let events = [
            WaitChildEvent::Attempted,
            WaitChildEvent::Spawned(41_001),
            WaitChildEvent::Attempted,
            WaitChildEvent::Spawned(41_002),
            WaitChildEvent::Reaped(41_001, exited(9)),
            WaitChildEvent::Reaped(41_002, exited(9)),
        ];
        let table = [
            row(41_003, own, "(ae-bc7af8f05eb6f)"),
            row(41_004, own, "(ae-bc7af8f05eb6f)"),
            row(41_005, own, "ps"),
            row(1, 0, "launchd"),
        ];
        assert_eq!(wait_left_behind(&events, &table), Vec::<String>::new());
    }

    #[test]
    fn a_started_pid_still_in_the_table_is_a_leftover_of_the_wait() {
        let own = std::process::id();
        let events = [
            WaitChildEvent::Attempted,
            WaitChildEvent::Spawned(41_001),
            WaitChildEvent::Reaped(41_001, exited(9)),
        ];
        let table = [
            row(41_003, own, "(ae-bc7af8f05eb6f)"),
            row(41_001, own, "tmux"),
        ];
        let left = wait_left_behind(&events, &table);
        assert_eq!(
            left.len(),
            1,
            "only the started pid is the wait's: {left:?}"
        );
        assert!(left[0].contains("41001"), "{left:?}");
    }

    #[test]
    fn a_started_pid_with_no_reap_is_a_leftover_of_the_wait() {
        let events = [
            WaitChildEvent::Attempted,
            WaitChildEvent::Spawned(41_001),
            WaitChildEvent::Attempted,
            WaitChildEvent::SpawnFailed,
        ];
        let left = wait_left_behind(&events, &[]);
        assert_eq!(left.len(), 1, "one start has no reap: {left:?}");
        assert!(left[0].contains("41001"), "{left:?}");
    }

    #[test]
    fn an_unreachable_first_read_is_asked_again_once_past_the_settle_and_no_other_is() {
        use crate::tmux::Absence::{Listed, Proven, SocketMissing, Unreachable};
        let run = |first, second| {
            let log = std::cell::RefCell::new(Vec::new());
            let mut answers = [first, second].into_iter();
            let read = || {
                log.borrow_mut().push("read");
                answers.next().expect("at most two reads")
            };
            let seen = super::settled_read(read, || log.borrow_mut().push("settle"));
            (seen, log.into_inner())
        };
        for s in [Proven, Listed, SocketMissing, Unreachable] {
            assert_eq!(run(Unreachable, s), (s, ["read", "settle", "read"].into()));
        }
        for first in [Listed, Proven, SocketMissing] {
            assert_eq!(run(first, Unreachable), (first, vec!["read"]));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clipboard_mac_no_image_discards_script_output_and_returns_empty_success() {
        for outcome in ["error number -1700", "return \"junk stdout\""] {
            let scratch = super::CaptureScratch::tagged("clip-test").unwrap();
            let path = scratch.path.to_str().unwrap();
            let script = super::CLIPBOARD_SCRIPT
                .replace("set imageData to the clipboard as «class PNGf»", outcome);
            let out = super::spawn_until(
                "osascript",
                &["-e", &script, path],
                &[],
                1024,
                std::time::Duration::from_secs(8),
                Some(&scratch),
                None,
            );
            assert!(
                matches!(out, super::Ran::Exited(exit, bytes) if exit.success() && bytes.is_empty())
            );
        }
    }
    use super::{
        AGY_DEADLINES, AgyRun, CaptureScratch, GIT_LOCAL_ENV_VARS, PROGRAM, Streams, Tmux,
        agy_quota_with, declares_utf8, run, spawn,
    };
    use crate::inventory::{Discovery, QueryFailed, ServerId};
    use crate::meta::Selector;
    use std::path::PathBuf;
    use std::time::Duration;

    /// A fake `agy`, written at test time and ALONE on the child's `PATH`: it
    /// appends its argv to `argv` beside it, then runs the version or the quota
    /// script. Only absolute programs reach it, so nothing real can.
    struct FakeAgy {
        dir: PathBuf,
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "a test: plants its own fake agy, and reads what it wrote, in its own scratch dir"
    )]
    impl FakeAgy {
        fn new(tag: &str, version: &str, quota: &str) -> Self {
            use std::os::unix::fs::PermissionsExt as _;
            let dir = std::env::temp_dir().join(format!("ae-agy-leg-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("the scratch dir");
            let script = format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"${{0%/*}}/argv\"\n\
                 if [ \"$1\" = --version ]; then {version}; exit 0; fi\n{quota}\n"
            );
            let fake = dir.join("agy");
            std::fs::write(&fake, script).expect("the fake agy");
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
                .expect("the fake agy is executable");
            Self { dir }
        }

        fn run(&self, deadlines: [Duration; 2]) -> AgyRun {
            let path = self.dir.display().to_string();
            agy_quota_with(&self.dir, &[("PATH", path.as_str())], deadlines)
        }

        /// What the fake wrote into `name` beside itself; empty when nothing.
        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.dir.join(name)).unwrap_or_default()
        }
    }

    impl Drop for FakeAgy {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    const SHORT: [Duration; 2] = [Duration::from_millis(400), Duration::from_millis(600)];
    // A hang must leave its argv/pid witness even when process startup is slow.
    // Still well below the fake's 30 s sleep, so a missed kill fails the bound.
    const HANG_DEADLINE: Duration = Duration::from_secs(3);
    // These calls assert answers, not deadlines: leave room for a loaded host.
    const GENEROUS: [Duration; 2] = [Duration::from_secs(20), Duration::from_secs(20)];
    const VERSION_OK: &str = "printf '%s\\n' 1.2.14";
    const BOTH_CALLS: &str = "--version\n-p /quota --output-format json\n";

    #[test]
    fn the_agy_leg_runs_the_version_gate_then_the_fixed_quota_call() {
        let fake = FakeAgy::new("ok", VERSION_OK, "printf '%s' '{\"status\":\"SUCCESS\"}'");
        match fake.run(GENEROUS) {
            AgyRun::Done { text, .. } => assert_eq!(text, "{\"status\":\"SUCCESS\"}"),
            other => panic!("the call answered: {other:?}"),
        }
        assert_eq!(fake.read("argv"), BOTH_CALLS);
        // Exactly 64 KiB is still an answer; one byte more is refused below.
        let full = FakeAgy::new("full", VERSION_OK, "exec /usr/bin/head -c 65536 /dev/zero");
        assert!(matches!(full.run(GENEROUS), AgyRun::Done { text, .. } if text.len() == 65_536));
        assert_eq!(AGY_DEADLINES.map(|deadline| deadline.as_secs()), [3, 8]);
    }

    #[test]
    fn the_agy_leg_asks_the_account_of_the_home_it_was_handed() {
        // A fresh scratch dir, so never this process's own HOME.
        let fake = FakeAgy::new("home", VERSION_OK, "printf '%s' \"$HOME\"");
        let home = fake.dir.display().to_string();
        assert!(matches!(fake.run(GENEROUS), AgyRun::Done { text } if text == home));
        let raw = <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(b"/tmp/\xff");
        let path = [("PATH", home.as_str())];
        // A HOME agy cannot be handed starts nothing: one run in the log.
        assert_eq!(
            agy_quota_with(raw.as_ref(), &path, GENEROUS),
            AgyRun::Failed
        );
        assert_eq!(fake.read("argv"), BOTH_CALLS);
    }

    #[test]
    fn a_version_gate_that_proves_nothing_never_sends_the_quota_prompt() {
        // Only the hanging version judges a deadline; the others must answer
        // before we can prove they refused the quota prompt on their merits.
        for (tag, version, version_deadline) in [
            ("old", "printf '%s\\n' 1.1.9", GENEROUS[0]),
            ("fails", "printf '%s\\n' 1.2.14; exit 3", GENEROUS[0]),
            ("hangs", "exec /bin/sleep 30", HANG_DEADLINE),
        ] {
            let fake = FakeAgy::new(tag, version, "exit 96");
            let started = std::time::Instant::now();
            assert_eq!(
                fake.run([version_deadline, GENEROUS[1]]),
                AgyRun::TooOld,
                "{tag}"
            );
            let took = started.elapsed();
            if tag == "hangs" {
                assert!(
                    took >= version_deadline && took < Duration::from_secs(10),
                    "{tag}: {took:?}"
                );
            }
            assert_eq!(fake.read("argv"), "--version\n", "{tag}: the prompt ran");
        }
    }

    #[test]
    fn a_quota_call_past_its_deadline_is_killed_and_reaped() {
        let fake = FakeAgy::new(
            "hang",
            VERSION_OK,
            "printf '%s' \"$$\" > \"${0%/*}/pid\"; exec /bin/sleep 30",
        );
        // The version must answer before the quota deadline can be judged.
        let deadlines = [GENEROUS[0], HANG_DEADLINE];
        let started = std::time::Instant::now();
        assert_eq!(fake.run(deadlines), AgyRun::TimedOut);
        let took = started.elapsed();
        // Also bound end-to-end progress: VERSION_OK prints immediately, and
        // its generous deadline only allows for a loaded host's startup.
        assert!(
            took >= deadlines[1] && took < Duration::from_secs(10),
            "{took:?}"
        );
        let pid: u32 = fake.read("pid").parse().expect("its pid");
        let table = crate::procs::snapshot().expect("the process table");
        assert!(
            table.iter().all(|row| row.pid != pid),
            "{pid} survived or was not reaped"
        );
        assert_eq!(fake.read("argv"), BOTH_CALLS);
    }

    /// The deadline is generous, so each row is refused on its own merits; the
    /// `growth` row proves the cap bounds the FILE while the child still runs.
    #[test]
    fn a_quota_answer_ae_cannot_use_is_a_failure() {
        for (tag, quota) in [
            ("nonzero", "printf '%s' '{}'; exit 7"),
            ("not-utf8", "printf '\\377'"),
            ("oversize", "exec /usr/bin/head -c 65537 /dev/zero"),
            (
                "growth",
                "/usr/bin/head -c 70000 /dev/zero; exec /bin/sleep 30",
            ),
        ] {
            let fake = FakeAgy::new(tag, VERSION_OK, quota);
            let started = std::time::Instant::now();
            assert_eq!(fake.run(GENEROUS), AgyRun::Failed, "{tag}");
            assert!(started.elapsed() < Duration::from_secs(10), "{tag}");
            assert_eq!(fake.read("argv"), BOTH_CALLS, "{tag}");
        }
    }

    #[test]
    fn no_agy_on_the_path_is_absent() {
        let fake = FakeAgy::new("absent", VERSION_OK, "exit 96");
        let _ = std::fs::remove_file(fake.dir.join("agy"));
        assert_eq!(fake.run(SHORT), AgyRun::Absent);
        assert_eq!(fake.read("argv"), "");
    }

    /// Only a missing program is absent: an `agy` the child may not execute is
    /// an install ae cannot ask, so the version gate stays shut.
    #[test]
    fn an_agy_that_cannot_be_executed_is_too_old_not_absent() {
        use std::os::unix::fs::PermissionsExt as _;
        let fake = FakeAgy::new("noexec", VERSION_OK, "exit 96");
        let mode = std::fs::Permissions::from_mode(0o644);
        std::fs::set_permissions(fake.dir.join("agy"), mode)
            .expect("the fake agy is not executable");
        assert_eq!(fake.run(SHORT), AgyRun::TooOld);
        assert_eq!(fake.read("argv"), "");
    }

    #[test]
    fn a_program_that_ran_reports_success_and_its_output() {
        // THE CONTROL, AND IT COMES FIRST.
        assert_eq!(
            run("/bin/echo", &["ok".to_owned()]),
            (true, "ok\n".to_owned()),
            "this suite needs /bin/echo to prove the exec can succeed at all"
        );
    }

    /// `-u` goes on every tmux leg ae reads (#187), and on nothing else: not
    /// the human's own `Terminal` client, and no other program on any leg.
    #[test]
    fn only_a_tmux_leg_that_ae_reads_is_started_as_a_utf8_client() {
        let scratch = CaptureScratch::new().expect("a scratch file");
        let legs = [
            Streams::Captured,
            Streams::InheritStderr,
            Streams::Detached,
            Streams::CapturedToFile {
                file: scratch.file(),
                cap: 0,
            },
            Streams::Terminal,
        ];
        for leg in legs {
            let read_by_ae = !matches!(leg, Streams::Terminal);
            assert_eq!(declares_utf8(PROGRAM, leg), read_by_ae, "tmux on {leg:?}");
            for other in ["git", "ps", "sysctl", "nohup", "opencode", "/s/demo/send"] {
                assert!(!declares_utf8(other, leg), "{other} on {leg:?}");
            }
        }
    }

    /// The rule above is WIRED: `spawn` itself puts `-u` in front of a tmux
    /// leg's words. A fake `tmux` first on the child's PATH echoes its argv.
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test: plants and removes its own fake tmux"
    )]
    fn spawn_hands_a_captured_tmux_leg_u_before_its_words() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("ae-transport-u-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the scratch dir");
        let fake = dir.join(PROGRAM);
        std::fs::write(&fake, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").expect("the fake tmux");
        let mode = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&fake, mode).expect("the fake tmux is executable");
        let path = dir.display().to_string();
        let envs = [("PATH", path.as_str())];
        let output = spawn(PROGRAM, &["list-sessions"], &envs, Streams::Captured, None);
        let _ = std::fs::remove_dir_all(&dir);
        let stdout = output.expect("the fake tmux ran").stdout;
        assert_eq!(String::from_utf8_lossy(&stdout), "-u\nlist-sessions\n");
    }

    /// The scrub list is the literal `git rev-parse --local-env-vars` set —
    /// a dropped name silently re-aims every query at the caller's repository.
    #[test]
    fn the_git_scrub_list_is_the_literal_local_env_vars() {
        assert_eq!(GIT_LOCAL_ENV_VARS.len(), 15);
        for var in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_PREFIX",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
        ] {
            assert!(GIT_LOCAL_ENV_VARS.contains(&var), "the scrub lost {var}");
        }
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
        use std::os::unix::fs::OpenOptionsExt as _;
        // The plant wears the scratch file's own mode and is removed by a
        // guard, so even a failing run leaves no world-readable look-alike.
        struct Plant(PathBuf);
        impl Drop for Plant {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let first = std::env::temp_dir().join(format!("ae-opencode.{}.0.json", std::process::id()));
        let _ = std::fs::remove_file(&first);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&first)
            .expect("the plant");
        let _plant = Plant(first.clone());
        let scratch = CaptureScratch::new().expect("a capture despite the plant");
        assert_ne!(
            scratch.path, first,
            "the collision was skipped, not refused"
        );
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

    /// One fake `muse` ALONE on the child's `PATH`, run through the Muse leg.
    #[allow(
        clippy::disallowed_methods,
        reason = "a test: plants its own fake muse in its own scratch dir"
    )]
    fn through_muse_leg(tag: &str, body: &str, deadline: Duration) -> (super::MuseRun, String) {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("ae-muse-leg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the scratch dir");
        let fake = dir.join("muse");
        std::fs::write(&fake, format!("#!/bin/sh\n{body}\n")).expect("the fake muse");
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
            .expect("the fake muse is executable");
        let path = dir.display().to_string();
        let ran = super::muse_quota_with(&dir, &[("PATH", path.as_str())], deadline);
        let _ = std::fs::remove_dir_all(&dir);
        (ran, path)
    }

    #[test]
    fn the_muse_leg_names_how_a_call_ended() {
        use super::MuseRun;
        const ROOMY: Duration = Duration::from_secs(20);
        let table = [
            (
                "ok",
                "printf '%s' hello",
                MuseRun::Done {
                    text: "hello".to_owned(),
                },
            ),
            (
                "exit3",
                "echo noise >&2; exit 3",
                MuseRun::Exited {
                    code: 3,
                    text: String::new(),
                },
            ),
            (
                "exit255",
                "exit 255",
                MuseRun::Exited {
                    code: 255,
                    text: String::new(),
                },
            ),
            ("binary", "printf '\\377\\376'", MuseRun::NotUtf8),
            (
                "big",
                "exec /usr/bin/head -c 262145 /dev/zero",
                MuseRun::TooLarge,
            ),
            ("signal", "kill -9 $$", MuseRun::Failed),
        ];
        for (tag, body, expected) in table {
            assert_eq!(through_muse_leg(tag, body, ROOMY).0, expected, "{tag}");
        }
        // Exactly the output ceiling is still an answer.
        let full = through_muse_leg("full", "exec /usr/bin/head -c 262144 /dev/zero", ROOMY).0;
        assert!(matches!(full, MuseRun::Done { text } if text.len() == 262_144));
        let started = std::time::Instant::now();
        let hung = through_muse_leg("hang", "exec /bin/sleep 30", Duration::from_secs(1)).0;
        assert_eq!(hung, MuseRun::TimedOut);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "killed and reaped"
        );
    }

    #[test]
    fn the_muse_leg_asks_the_home_it_was_handed_and_finds_no_muse_without_one() {
        use super::MuseRun;
        let (ran, home) =
            through_muse_leg("home", "printf '%s' \"$HOME\"", Duration::from_secs(20));
        assert_eq!(ran, MuseRun::Done { text: home });
        // A HOME muse cannot be handed starts nothing.
        let raw = <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(b"/tmp/\xff");
        assert_eq!(
            super::muse_quota_with(raw.as_ref(), &[], Duration::from_secs(20)),
            MuseRun::Failed
        );
        // An empty `PATH` has no muse: nothing asked, nothing spent.
        let empty = std::env::temp_dir().join(format!("ae-muse-leg-empty-{}", std::process::id()));
        std::fs::create_dir_all(&empty).expect("the empty dir");
        let path = empty.display().to_string();
        let absent =
            super::muse_quota_with(&empty, &[("PATH", path.as_str())], Duration::from_secs(20));
        let _ = std::fs::remove_dir_all(&empty);
        assert_eq!(absent, MuseRun::Absent);
    }

    #[test]
    fn the_muse_leg_runs_in_a_fresh_workspace_that_is_gone_afterwards() {
        use super::MuseRun;
        use crate::store::{SourcePresence, source_presence};
        // The fake's `PATH` holds only itself: absolute programs, or builtins.
        let body = "printf '%s\\n' \"$(pwd -P)\"; /bin/ls -A . | /usr/bin/wc -l";
        let (ran, _) = through_muse_leg("cwd", body, Duration::from_secs(20));
        let MuseRun::Done { text } = ran else {
            panic!("the call answered: {ran:?}");
        };
        let mut lines = text.lines();
        // The physical directory the child stood in, recorded while it ran.
        let cwd = PathBuf::from(lines.next().expect("the cwd line"));
        assert_eq!(lines.next().map(str::trim), Some("0"), "empty at entry");
        let temp = std::env::temp_dir()
            .canonicalize()
            .expect("canonical temp dir");
        assert_eq!(
            cwd.parent(),
            Some(temp.as_path()),
            "a direct child of the temp dir, never the temp dir itself: {}",
            cwd.display()
        );
        let name = cwd
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default();
        assert!(name.starts_with("ae-muse-cwd."), "{}", cwd.display());
        assert_eq!(
            source_presence(&cwd),
            SourcePresence::Absent,
            "{}",
            cwd.display()
        );
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test: reads what its own shell line wrote in its own scratch dir"
    )]
    fn bang_run_shell_line_runs_in_dir_detached_with_null_stdin_and_open_popup_refuses_relative() {
        use crate::console::shell::{ShellArgv, popup_args};
        let root = std::env::temp_dir().join(format!("ae-bang-transport-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch dir");
        let canonical = root.canonicalize().expect("canonical scratch dir");

        // T7: `run_shell_line` runs `/bin/sh -c` in `dir`, stdin is `/dev/null`
        // (`cat` exits immediately with 0 bytes), and a background child does not
        // block `run_shell_line` from returning. `mv` publishes `out_file` only
        // after both writes complete so `exists()` never races a partial write.
        let out_file = root.join("shell-out.txt");
        let cmd = format!(
            "cat > {0}.stdin && pwd -P > {0}.tmp && mv {0}.tmp {0}; (sleep 3) &",
            out_file.display()
        );
        let argv = ShellArgv::of(&cmd).expect("valid shell line");
        let started = std::time::Instant::now();
        assert!(super::run_shell_line(&argv, &root), "shell starts");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "detached leg must not wait for background sleep"
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline && !out_file.exists() {
            std::thread::sleep(Duration::from_millis(20));
        }
        let pwd = std::fs::read_to_string(&out_file).expect("pwd output");
        assert_eq!(PathBuf::from(pwd.trim()), canonical);
        let stdin_bytes =
            std::fs::read(format!("{}.stdin", out_file.display())).expect("stdin file");
        assert!(stdin_bytes.is_empty(), "stdin must be /dev/null");

        let popup = popup_args(
            &ServerId::Ambient,
            "/dev/ttys001",
            canonical.to_str().expect("utf8 path"),
            "git status",
        )
        .expect("popup args");
        let relative = ServerId::Selected(Selector::Socket(PathBuf::from("relative/bang.sock")));
        assert!(
            !super::open_popup(&relative, &popup),
            "relative socket refused"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_waiter_ae_stops_for_is_released_exits_normally_and_leaves_no_stale_signal() {
        use super::{Waited, run_tmux_wait, spawns_during, waiter_exits_during};
        use std::time::Instant;
        let rig = super::quietwd_spec::Rig::new("release");
        let wait = |within: Duration| {
            waiter_exits_during(|| {
                spawns_during(|| {
                    run_tmux_wait(
                        &rig.server,
                        "current",
                        "qw-release",
                        Instant::now() + within,
                    )
                })
            })
        };
        let ((result, calls), exits) = wait(Duration::from_millis(150));
        assert_eq!(result, Waited::TimedOut);
        assert_eq!(calls.len(), 2, "the waiter and its release: {calls:?}");
        assert!(
            calls[1].ends_with(&[
                "wait-for".to_owned(),
                "-S".to_owned(),
                "--".to_owned(),
                "qw-release".to_owned()
            ]),
            "the second child releases the waiter's own channel: {calls:?}"
        );
        assert_eq!(exits.len(), 1, "the waiter was reaped before the return");
        assert_eq!(
            exits[0].1.code(),
            Some(0),
            "released, never killed: {:?}",
            exits[0].1
        );
        // The release met a registered waiter, so it left no toggled signal: the
        // next waiter waits its whole deadline instead of returning at once.
        let began = Instant::now();
        let ((result, _), _) = wait(Duration::from_millis(300));
        assert_eq!(result, Waited::TimedOut);
        assert!(began.elapsed() >= Duration::from_millis(300));
        rig.attach("current", 0);
        let ((result, calls), exits) = wait(Duration::from_secs(5));
        assert_eq!(result, Waited::Attached);
        assert_eq!(calls.len(), 2, "a counted client releases the waiter too");
        assert_eq!(exits.len(), 1);
        assert_eq!(exits[0].1.code(), Some(0), "released, never killed");
    }

    #[test]
    fn a_channel_waiter_waits_out_its_deadline_wakes_on_a_signal_and_is_released_never_killed() {
        use super::{ChannelWaiter, Waited, signal_channel, spawns_during, waiter_exits_during};
        use std::time::Instant;
        let rig = super::quietwd_spec::Rig::new("channel");
        let channel = "qw-channel";
        // No signal: the wait ends at its deadline, not before, and the
        // client keeps waiting for the next wait.
        let mut waiter = ChannelWaiter::start(&rig.server, channel).expect("the waiter starts");
        let began = Instant::now();
        assert_eq!(
            waiter.wait_until(began + Duration::from_millis(300)),
            Waited::TimedOut
        );
        assert!(
            began.elapsed() >= Duration::from_millis(300),
            "a wait with no signal lasts until its deadline"
        );
        // A signal on its channel wakes that same client, which is reaped.
        assert!(signal_channel(&rig.server, channel), "the signal is sent");
        let (woke, exits) =
            waiter_exits_during(|| waiter.wait_until(Instant::now() + Duration::from_secs(5)));
        assert_eq!(woke, Waited::Woke);
        assert_eq!(exits.len(), 1, "the woken client was reaped: {exits:?}");
        // A client still waiting is released by a signal on its own channel,
        // then reaped with a normal exit.
        let waiter = ChannelWaiter::start(&rig.server, channel).expect("the waiter starts");
        let pid = waiter.child.id();
        let (((), calls), exits) =
            waiter_exits_during(|| spawns_during(|| waiter.release(&rig.server)));
        assert_eq!(calls.len(), 1, "one release child: {calls:?}");
        assert!(
            calls[0].ends_with(&[
                "wait-for".to_owned(),
                "-S".to_owned(),
                "--".to_owned(),
                channel.to_owned()
            ]),
            "the release signals the waiter's own channel: {calls:?}"
        );
        assert_eq!(exits.len(), 1, "the waiter was reaped before the return");
        assert_eq!(exits[0].0, pid, "the reaped client is the waiter itself");
        assert_eq!(
            exits[0].1.code(),
            Some(0),
            "released, never killed: {:?}",
            exits[0].1
        );
    }

    #[test]
    fn polling_a_waiters_output_leaves_the_writers_cursor_where_it_was() {
        use std::io::{Seek as _, Write as _};
        // The child writes through a clone of this handle, and a clone shares
        // the file offset: a poll that moves it moves where the child writes.
        let scratch = CaptureScratch::tagged("wait-cursor").expect("scratch");
        let mut writer = scratch.file().try_clone().expect("the child's clone");
        let long = "x".repeat(300);
        writer.write_all(long.as_bytes()).expect("write");
        let seen = super::waiter_output(&scratch);
        assert_eq!(seen.len(), 256, "the poll reads at most its cap");
        assert_eq!(
            writer.stream_position().expect("cursor"),
            300,
            "the poll moved the writer's cursor"
        );
        writer
            .write_all(b"\nae-wd:a|0\n")
            .expect("the child's next write");
        let mut whole = String::new();
        let mut file = scratch.file();
        file.seek(std::io::SeekFrom::Start(0)).expect("rewind");
        std::io::Read::read_to_string(&mut file, &mut whole).expect("read");
        assert_eq!(
            whole,
            format!("{long}\nae-wd:a|0\n"),
            "nothing was overwritten"
        );
    }

    #[test]
    fn a_stalled_server_cannot_hold_the_wait_or_its_release_past_their_bounds() {
        use super::{Waited, run_tmux_wait, wait_children_during, waiter_exits_during};
        use std::time::Instant;
        // A socket that accepts and never answers: every tmux client on it,
        // the waiter and its release alike, hangs until it is killed.
        let dir = std::env::temp_dir().join(format!("qwst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).expect("fixture dir");
        let socket = dir.join("s");
        let listener = std::os::unix::net::UnixListener::bind(&socket).expect("mute server");
        let server = ServerId::Selected(Selector::Socket(socket));
        let (tell, told) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let began = Instant::now();
            let ((result, exits), events) = wait_children_during(|| {
                waiter_exits_during(|| {
                    run_tmux_wait(
                        &server,
                        "current",
                        "qw-stall",
                        Instant::now() + Duration::from_millis(200),
                    )
                })
            });
            let _ = tell.send((result, exits, events, began.elapsed()));
        });
        let (result, exits, events, took) = told
            .recv_timeout(Duration::from_secs(5))
            .expect("a stalled release blocked the wait past every bound");
        drop(listener);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(result, Waited::Failed, "a forced cleanup is no outcome");
        assert!(took < Duration::from_secs(3), "bounded: {took:?}");
        assert_eq!(exits.len(), 1, "the waiter was reaped");
        assert_wait_left_nothing(exits[0].0, &events);
    }

    #[test]
    fn a_stalled_release_alone_still_ends_the_wait_failed_after_its_cleanup() {
        use super::{Waited, run_tmux_wait, wait_children_during, waiter_exits_during};
        use std::time::Instant;
        // The waiter connects first and the mute server holds it; once its
        // release connects too, the server drops the waiter, which exits by
        // itself, and holds the release unanswered.
        let dir = std::env::temp_dir().join(format!("qwsr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).expect("fixture dir");
        let socket = dir.join("s");
        let listener = std::os::unix::net::UnixListener::bind(&socket).expect("mute server");
        let server = ServerId::Selected(Selector::Socket(socket));
        let (hand_over, handed) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (waiter, _) = listener.accept().expect("the waiter connects");
            let (release, _) = listener.accept().expect("the release connects");
            drop(waiter);
            let _ = hand_over.send(release);
        });
        let (tell, told) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let began = Instant::now();
            let ((result, exits), events) = wait_children_during(|| {
                waiter_exits_during(|| {
                    run_tmux_wait(
                        &server,
                        "current",
                        "qw-stall-release",
                        Instant::now() + Duration::from_millis(200),
                    )
                })
            });
            let _ = tell.send((result, exits, events, began.elapsed()));
        });
        let held = handed
            .recv_timeout(Duration::from_secs(5))
            .expect("the waiter and then its release connect");
        let (result, exits, events, took) = told
            .recv_timeout(Duration::from_secs(5))
            .expect("a stalled release blocked the wait past every bound");
        drop(held);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            result,
            Waited::Failed,
            "every forced cleanup reports Failed, even when the waiter left by itself"
        );
        assert!(took < Duration::from_secs(3), "bounded: {took:?}");
        assert_eq!(exits.len(), 1, "the waiter was reaped once");
        assert_eq!(
            std::os::unix::process::ExitStatusExt::signal(&exits[0].1),
            None,
            "the waiter left by itself, unkilled"
        );
        assert_wait_left_nothing(exits[0].0, &events);
    }

    #[test]
    fn a_held_release_grace_lengthens_the_backstop_and_does_not_remove_it() {
        use super::{Waited, hold_release_grace, run_tmux_wait};
        use std::time::Instant;
        // The same mute server as the stalled-server test: the waiter and its
        // release hang until the backstop kills them. The grace held on the
        // waiting thread is 2 s, past the 1 s default plus the 200 ms
        // deadline, so only a backstop that waits the held grace takes this
        // long, and only a backstop that still fires ends the wait Failed.
        // The bound is a floor: load can only lengthen the wait.
        let held_grace = Duration::from_secs(2);
        let dir = std::env::temp_dir().join(format!("qwhg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).expect("fixture dir");
        let socket = dir.join("s");
        let listener = std::os::unix::net::UnixListener::bind(&socket).expect("mute server");
        let server = ServerId::Selected(Selector::Socket(socket));
        let (tell, told) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _held = hold_release_grace(held_grace);
            let began = Instant::now();
            let result = run_tmux_wait(
                &server,
                "current",
                "qw-held-grace",
                Instant::now() + Duration::from_millis(200),
            );
            let _ = tell.send((result, began.elapsed()));
        });
        let (result, took) = told
            .recv_timeout(Duration::from_secs(10))
            .expect("the backstop never fired");
        drop(listener);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(result, Waited::Failed, "the backstop still kills and fails");
        assert!(
            took >= held_grace,
            "the backstop waited the held grace, not the default: {took:?}"
        );
    }

    #[test]
    fn a_waiter_is_no_client_of_the_session_it_waits_on() {
        use super::{Waited, run_tmux_wait};
        use std::time::Instant;
        let rig = super::quietwd_spec::Rig::new("invisible");
        let result = std::thread::scope(|scope| {
            let waiting = scope.spawn(|| {
                run_tmux_wait(
                    &rig.server,
                    "current",
                    "qw-ghost",
                    Instant::now() + Duration::from_millis(800),
                )
            });
            std::thread::sleep(Duration::from_millis(300));
            assert_eq!(rig.must(&["list-clients", "-F", "#{client_name}"]), "");
            assert_eq!(
                rig.must(&[
                    "display-message",
                    "-p",
                    "-t",
                    "=current:",
                    "#{session_attached}"
                ])
                .trim(),
                "0",
                "a waiter must never count as an attachment"
            );
            waiting.join().expect("waiter thread")
        });
        assert_eq!(
            result,
            Waited::TimedOut,
            "the waiter's own read did not count itself"
        );
    }

    /// A channel no hook can name is a wait nothing could ever end early: the
    /// wait fails at once and starts no process, even on a live server.
    #[test]
    fn a_wait_on_a_channel_no_hook_can_name_starts_nothing_and_fails() {
        use super::{Waited, run_tmux_wait, spawns_during};
        use std::time::Instant;
        let rig = super::quietwd_spec::Rig::new("badchan");
        for channel in ["", "qw bad", "qw;bad"] {
            assert!(!crate::tmux::is_wake_channel(channel), "{channel:?}");
            let began = Instant::now();
            let (result, calls) = spawns_during(|| {
                run_tmux_wait(
                    &rig.server,
                    "current",
                    channel,
                    Instant::now() + Duration::from_millis(400),
                )
            });
            assert_eq!(result, Waited::Failed, "{channel:?}");
            assert!(calls.is_empty(), "{channel:?} started a child: {calls:?}");
            assert!(
                began.elapsed() < Duration::from_millis(400),
                "{channel:?} waited instead of failing at once"
            );
        }
    }

    /// A client that attached before the waiter registered has already
    /// signalled the channel through its hook, so the waiter's read counts it
    /// and its wait returns at once. The read decides: Attached, not a wake.
    #[test]
    fn a_client_counted_beside_a_stored_signal_is_attached_not_a_wake() {
        use super::{Waited, run_tmux_wait, waiter_exits_during};
        use std::time::Instant;
        let rig = super::quietwd_spec::Rig::new("stored");
        rig.attach("current", 0);
        // One signal with no waiter is stored for the next waiter; two toggle
        // it away, which the spec pins separately.
        rig.must(&["wait-for", "-S", "qw-stored"]);
        let (result, exits) = waiter_exits_during(|| {
            run_tmux_wait(
                &rig.server,
                "current",
                "qw-stored",
                Instant::now() + Duration::from_secs(5),
            )
        });
        assert_eq!(result, Waited::Attached);
        assert_eq!(exits.len(), 1, "the waiter was reaped once: {exits:?}");
        assert_eq!(exits[0].1.code(), Some(0), "never killed: {:?}", exits[0].1);
    }

    /// A server that dies under a waiter after its read is not a signal: the
    /// waiter exits unsuccessfully ("server exited unexpectedly"), and the wait
    /// fails, so the interval polls instead of trusting a wake.
    #[test]
    fn a_waiter_whose_server_dies_after_its_read_fails_instead_of_waking() {
        use super::{Waited, run_tmux_wait, waiter_exits_during};
        use std::time::Instant;
        let rig = super::quietwd_spec::Rig::new("lost");
        let pid = rig
            .must(&["display-message", "-p", "#{pid}"])
            .trim()
            .to_owned();
        assert!(
            !pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit()),
            "server pid: {pid:?}"
        );
        let (result, exits) = std::thread::scope(|scope| {
            let waiting = scope.spawn(|| {
                waiter_exits_during(|| {
                    run_tmux_wait(
                        &rig.server,
                        "current",
                        "qw-lost",
                        Instant::now() + Duration::from_secs(10),
                    )
                })
            });
            // Long past the waiter's read and registration.
            std::thread::sleep(Duration::from_millis(400));
            // The server's own child kills it outright, so no orderly exit
            // reaches the waiter; the fixture command may fail with it.
            let _ = rig.raw(&["run-shell", "-b", &format!("kill -9 {pid}")]);
            waiting.join().expect("waiter thread")
        });
        assert_eq!(result, Waited::Failed, "a lost server is no wake");
        assert_eq!(exits.len(), 1, "the waiter was reaped once: {exits:?}");
        assert!(
            !exits[0].1.success(),
            "the waiter must have seen its server die: {:?}",
            exits[0].1
        );
    }

    /// A read error that does not go away (here EBADF from a handle opened
    /// write-only; a physical EIO is the same shape) ends the read at once with
    /// nothing. Retrying it would spin before the wait ever checks its deadline.
    #[test]
    fn a_waiter_read_that_keeps_failing_ends_at_once_with_nothing() {
        use super::{CaptureScratch, waiter_output};
        let readable = CaptureScratch::tagged("unreadable").expect("scratch");
        std::fs::write(&readable.path, b"ae-wd:a|1\n").expect("bytes a read would return");
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&readable.path)
            .expect("write-only handle");
        let unreadable = CaptureScratch {
            file,
            path: readable.path.clone(),
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(waiter_output(&unreadable));
        });
        let read = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("a permanent read error must end the read, not retry it");
        assert_eq!(
            read, "",
            "a failed read yields nothing, never partial bytes"
        );
    }

    /// The joined motion read answers exactly what the two separate reads
    /// answer, for values with a CR and for a value with a newline that prints
    /// a fleet-like row: every section reaches today's interpreter unchanged.
    #[test]
    fn the_joined_motion_read_answers_exactly_as_the_two_separate_reads_do() {
        let rig = super::quietwd_spec::Rig::new("same");
        rig.must(&["set-option", "-p", "-t", &rig.pane, "@ae_agent", "lead\r"]);
        rig.must(&["set-option", "-t", "=current:", "@ae_palette", "b\r"]);
        let _ = rig.add_session("other");
        rig.must(&[
            "set-option",
            "-t",
            "=other:",
            "@ae_look",
            "on\nforged | $77 | 2 |  |  |  | ",
        ]);
        let separate = super::observe_motion_panes(&rig.server, "current")
            .zip(super::observe_fleet_listing(&rig.server));
        assert!(separate.is_some(), "both separate reads answer");
        let (joined, calls) =
            spawns_during(|| super::exchange_motion(&rig.server, "current", true, &[]));
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(joined.reading, separate);
        assert_eq!(joined.published, None, "nothing to write");
    }

    /// Writes too large to ride with the reads go as their own process, after
    /// the reads, and still never past a missing read.
    #[test]
    fn writes_past_the_command_bound_go_alone_and_only_after_a_completed_read() {
        use crate::tmux::{COMMAND_BYTES, OptionScope, OptionWrite};
        let rig = super::quietwd_spec::Rig::new("oversized");
        let big = "x".repeat(COMMAND_BYTES);
        let writes = [OptionWrite::new(
            OptionScope::Pane,
            &rig.pane,
            "@qw_big",
            &big,
        )];
        let (result, calls) =
            spawns_during(|| super::exchange_motion(&rig.server, "current", true, &writes));
        assert_eq!(
            calls.len(),
            2,
            "the reads, then the writes: {}",
            calls.len()
        );
        assert!(
            !calls[0].iter().any(|word| word == "set-option"),
            "the reads travel alone"
        );
        assert!(result.reading.is_some());
        // tmux 3.4 refuses a command this long and later releases accept it;
        // either way the result is the write's own.
        let landed = rig.option("-p", &rig.pane, "@qw_big") == big;
        assert_eq!(result.published, Some(landed));
        let held = [OptionWrite::new(
            OptionScope::Pane,
            &rig.pane,
            "@qw_big_after_missing",
            &big,
        )];
        let (result, calls) =
            spawns_during(|| super::exchange_motion(&rig.server, "absent", true, &held));
        assert_eq!(calls.len(), 1, "a missing read sends no writes");
        assert!(result.reading.is_none());
        assert_eq!(result.published, Some(false));
        assert_eq!(rig.option("-p", &rig.pane, "@qw_big_after_missing"), "");
    }

    /// The cycle's look read is one process, and each answer is what its
    /// separate read answers.
    #[test]
    fn a_session_look_read_is_one_process_and_answers_as_the_separate_reads_do() {
        let rig = super::quietwd_spec::Rig::new("look");
        rig.must(&["set-option", "-t", "=current:", "@ae_icons", "off"]);
        rig.must(&["set-option", "-t", "=current:", "@ae_palette", "b\r"]);
        rig.must(&["set-option", "-t", "=current:", "@qw_stamp", "s1"]);
        rig.must(&["set-option", "-t", "=current:", "@qw_set", "123"]);
        let options = ["@qw_stamp", "@qw_unset", "@qw_set"];
        let look = super::observe_look(&rig.server, "current");
        assert!(look.is_some());
        let each: Vec<_> = options
            .iter()
            .map(|name| super::observe_session_option(&rig.server, "current", name))
            .collect();
        let (read, calls) =
            spawns_during(|| super::observe_session_look(&rig.server, "current", &options));
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(read.look, look);
        assert_eq!(read.options, each);
        assert_eq!(
            read.options,
            [Some("s1".to_owned()), None, Some("123".to_owned())]
        );
        // `display-message` resolves its target with CMD_FIND_CANFAIL, so the
        // look of an absent session answers an empty format, separate or
        // joined; the first `show-options` then fails and ends the read.
        let (read, calls) =
            spawns_during(|| super::observe_session_look(&rig.server, "absent", &options));
        assert_eq!(calls.len(), 1);
        assert_eq!(
            read,
            crate::tmux::SessionLookRead {
                look: super::observe_look(&rig.server, "absent"),
                options: vec![None; 3],
            }
        );
    }
}
