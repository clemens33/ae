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
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::viewer_args(server, pane));
    let viewer = tmux::interpret_viewer(succeeded, &stdout)?;
    let Some(view) = viewer
        .session
        .as_deref()
        .filter(|name| tmux::is_view_session(name))
    else {
        return Some(viewer);
    };
    let home = home_of(server, view, pane)?;
    let (succeeded, stdout) = run(PROGRAM, &tmux::viewer_args(server, &home.target));
    tmux::interpret_viewer(succeeded, &stdout)
        .filter(|viewer| viewer.session.as_deref() == Some(home.name.as_str()))
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
fn home_of(server: &ServerId, view: &str, pane: &str) -> Option<Home> {
    let (succeeded, stdout) = run(PROGRAM, &tmux::view_of_args(server, view));
    let id = succeeded.then(|| tmux::view_of_value(&stdout)).flatten()?;
    let target = tmux::pinned_pane_target(&id, pane)?;
    let (succeeded, stdout) = run(PROGRAM, &tmux::view_membership_args(server, &id));
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
pub(crate) use tests::{note_spawn, spawns_during};

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
    let home = home_of(server, &place.0, pane)?;
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
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::pane_session_identity_args(server, pane));
    let identity = tmux::interpret_pane_session_identity(succeeded, &stdout)?;
    // The format carries no name, so the session it answered for is named
    // by its own id before it is believed.
    let (succeeded, stdout) = run(PROGRAM, &tmux::session_name_of_args(server, &identity.id));
    let session = tmux::interpret_session_option(succeeded, &stdout)?;
    if !tmux::is_view_session(&session) {
        return Some(identity);
    }
    let home = home_of(server, &session, pane)?;
    let (succeeded, stdout) = run(
        PROGRAM,
        &tmux::pane_session_identity_args(server, &home.target),
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
    match spawn(PROGRAM, argv.as_args(), &[], Streams::Captured, None) {
        Some(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ),
        None => (false, String::new()),
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

#[cfg(test)]
mod tests {
    thread_local! {
        static SPAWNS: std::cell::RefCell<Option<Vec<Vec<String>>>> =
            const { std::cell::RefCell::new(None) };
    }

    /// Record one child [`super::spawn`] is about to start, its built argv
    /// (`-u` included), when this thread is inside [`spawns_during`].
    pub(crate) fn note_spawn<'a>(
        program: &std::ffi::OsStr,
        args: impl Iterator<Item = &'a std::ffi::OsStr>,
    ) {
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
}
