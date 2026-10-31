//! #220: a reader that closes stdout early ends ae normally, not as a failure.
//!
//! Every closed pipe here has its read end dropped BEFORE the child exists, so
//! the child's first stdout write meets `EPIPE` deterministically: no race with
//! a `head -1` that may or may not have exited yet. The oracle is the same
//! command with a readable stdout: a closed stdout may change neither its exit
//! status nor what it says on stderr. A streaming command must END once its
//! reader is gone. Errors that are not a closed stdout (a closed stderr, a full
//! device) keep failing.

use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use super::cli::{OwnedScratch, Runner, ae, bounded};
use super::support::{SETUP_WAIT, first_pane};

/// Generous: every case ends on its first write, while a follow that never
/// notices its reader left polls on forever.
const LIMIT: Duration = Duration::from_secs(30);

const CONVERSATION: &str = "0199c0de-1234-4890-abcd-ef0123456789";

/// A pipe nobody can read: its read end is gone before any child starts.
#[allow(clippy::expect_used)]
fn closed_pipe() -> std::io::PipeWriter {
    let (reader, writer) = std::io::pipe().expect("a pipe");
    drop(reader);
    writer
}

#[allow(clippy::expect_used)]
fn finish(runner: &mut Runner, what: &str) -> Output {
    let child = runner.spawn().expect("the ae binary should start");
    bounded(child, LIMIT).unwrap_or_else(|| panic!("{what}: ae did not exit within {LIMIT:?}"))
}

fn run(words: &[&str], stdout: Stdio) -> Output {
    finish(
        ae().args(words)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(Stdio::piped()),
        &format!("{words:?}"),
    )
}

/// The issue's own reproduction, `ae version | head -1`, and its siblings:
/// a command that succeeds with a readable stdout still succeeds, and says
/// nothing more on stderr, when its reader has already gone.
#[test]
fn a_successful_command_whose_reader_left_ends_quietly_and_successfully() {
    for words in [&["version"][..], &["--version"][..], &["help"][..]] {
        let control = run(words, Stdio::piped());
        assert_eq!(
            control.status.code(),
            Some(0),
            "{words:?}: control {control:?}"
        );
        assert!(
            !control.stdout.is_empty(),
            "{words:?}: the command writes stdout, so the closed run meets EPIPE"
        );
        let closed = run(words, closed_pipe().into());
        let said = String::from_utf8_lossy(&closed.stderr);
        assert!(!said.contains("Broken pipe"), "{words:?}: stderr {said:?}");
        assert_eq!(
            closed.status.code(),
            Some(0),
            "{words:?}: a reader leaving is not a failure; stderr {said:?}"
        );
        assert_eq!(
            said,
            String::from_utf8_lossy(&control.stderr),
            "{words:?}: a closed stdout adds nothing to stderr"
        );
    }
}

/// A follow writes until its reader leaves; then it must END, quietly, rather
/// than keep polling into a pipe nobody reads.
#[test]
fn a_following_board_ends_quietly_once_its_reader_has_left() {
    let scratch = OwnedScratch::root("pipequiet", "follow");
    let root = scratch.path();
    let store = root.join("claude");
    super::board::plant_transcript(
        &store,
        "work",
        CONVERSATION,
        &[super::board::user("2026-09-16T09:00:00Z", "human words")],
    );
    super::board::plant_session(
        root,
        "one",
        &super::board::claude_roster("main", "lead", CONVERSATION, &store),
    );
    let board = |words: &[&str], stdout: Stdio| {
        finish(
            ae().env("HOME", root)
                .env("AE_HOME", root)
                .env_remove("TMUX_PANE")
                .args(words)
                .stdin(Stdio::null())
                .stdout(stdout)
                .stderr(Stdio::piped()),
            &format!("{words:?}"),
        )
    };
    let control = board(&["board", "one"], Stdio::piped());
    assert_eq!(control.status.code(), Some(0), "fixture board {control:?}");
    assert!(
        !control.stdout.is_empty(),
        "the board writes its first pass"
    );
    let closed = board(&["board", "one", "--follow"], closed_pipe().into());
    let said = String::from_utf8_lossy(&closed.stderr);
    assert!(!said.contains("Broken pipe"), "stderr {said:?}");
    assert_eq!(
        closed.status.code(),
        Some(0),
        "a follow whose reader left ends normally; stderr {said:?}"
    );
    assert_eq!(
        said,
        String::from_utf8_lossy(&control.stderr),
        "the follow says nothing the one-shot board did not"
    );
}

/// A status the command decided BEFORE it wrote is the command's answer, not
/// the reader's: `doctor` builds its whole report, then writes it, then exits
/// with the report's verdict. A reader that left keeps that verdict and adds
/// nothing to stderr.
#[test]
fn a_status_decided_before_the_output_survives_a_reader_that_left() {
    let scratch = OwnedScratch::root("pipequiet", "doctor");
    let home = scratch.path().join("aehome");
    std::fs::create_dir_all(home.join("sessions")).expect("a sessions root");
    let absent = scratch.path().join("absent").display().to_string();
    let doctor = |stdout: Stdio| {
        finish(
            ae().env("AE_HOME", &home)
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .args([ae::cli::DOCTOR, "--global", absent.as_str()])
                .stdin(Stdio::null())
                .stdout(stdout)
                .stderr(Stdio::piped()),
            "doctor --global <absent>",
        )
    };
    // The first report creates the roots it names; the oracle is a report
    // over the settled home, which is what the closed run sees too.
    let warm = doctor(Stdio::piped());
    assert_eq!(warm.status.code(), Some(1), "fixture doctor {warm:?}");
    let control = doctor(Stdio::piped());
    assert_eq!(control.status.code(), Some(1), "control {control:?}");
    assert!(
        !control.stdout.is_empty(),
        "the report is written to stdout"
    );
    let closed = doctor(closed_pipe().into());
    let said = String::from_utf8_lossy(&closed.stderr);
    assert!(!said.contains("Broken pipe"), "stderr {said:?}");
    assert_eq!(
        closed.status.code(),
        control.status.code(),
        "the report's own verdict; stderr {said:?}"
    );
    assert_eq!(
        said,
        String::from_utf8_lossy(&control.stderr),
        "a closed stdout adds nothing to stderr"
    );
}

/// A stop prompt nobody can read is a stop nobody confirmed: the session is
/// "Not stopped", ae's failure (`EXIT_FAILED`), decided the moment the prompt
/// is put, whatever happens to the prompt itself. A reader that left keeps
/// that failure and adds nothing to stderr. The prompt needs a terminal on
/// stdin, so stdin is a pane's terminal on a private tmux server.
#[test]
fn a_stop_prompt_whose_reader_left_is_still_a_stop_that_did_not_happen() {
    let mut scratch = OwnedScratch::root("pipequiet", "stop");
    let socket = scratch.path().join("s");
    scratch.add_tmux_server(socket.clone());
    let home = scratch.path().join("aehome");
    std::fs::create_dir_all(home.join("sessions").join("pqstop")).expect("a session dir");
    let tmux = |tail: &[&str]| {
        let mut args = vec![
            "-S".to_owned(),
            socket.display().to_string(),
            "-f".to_owned(),
            "/dev/null".to_owned(),
        ];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        super::phase2::run_tmux(&args, scratch.path())
    };
    let (started, _) = tmux(&[
        "new-session",
        "-d",
        "-s",
        "pq",
        "-x",
        "80",
        "-y",
        "24",
        "sleep 600",
    ]);
    assert!(started, "a private tmux server lends a terminal");
    let (_, tty) = tmux(&["display-message", "-p", "-t", "pq", "#{pane_tty}"]);
    let terminal = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(tty.trim())
        .expect("the pane's terminal");
    let out = finish(
        ae().env("AE_HOME", &home)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .args([ae::cli::STOP, "--self", "pqstop"])
            .stdin(terminal)
            .stdout(closed_pipe())
            .stderr(Stdio::piped()),
        "_stop --self, stdout closed",
    );
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(!said.contains("Broken pipe"), "stderr {said:?}");
    assert_eq!(
        out.status.code(),
        Some(i32::from(ae::entry::EXIT_FAILED)),
        "an unconfirmed stop is not stopped; stderr {said:?}"
    );
}

/// `ae _stop all -y` over `names`, each a live session on one private tmux
/// server, with stdout pre-closed: the result, and how many of `names` are
/// still running after it.
#[allow(clippy::expect_used)]
fn fleet_stop(tag: &str, names: &[&str]) -> (Output, usize) {
    let mut scratch = OwnedScratch::root("pipequiet", tag);
    let home = scratch.path().to_path_buf();
    let socket = home.join("s");
    scratch.add_tmux_server(socket.clone());
    let config = home.join("config");
    std::fs::write(
        &config,
        "[profiles]\nfake = \"sh\"\n\n[roster]\nfake = fake\n\n[workspace]\nmain = fake\nlayout = vertical\n",
    )
    .expect("a config");
    let tmux = |tail: &[&str]| {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(socket.clone()),
        ));
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        super::phase2::run_tmux(&args, &home)
    };
    let live = || {
        let (_, listed) = tmux(&["list-sessions", "-F", "#{session_name}"]);
        listed.lines().filter(|name| names.contains(name)).count()
    };
    for (index, name) in names.iter().copied().enumerate() {
        assert!(
            tmux(&["-f", "/dev/null", "new-session", "-d", "-s", name, "sh"]).0,
            "{name} starts"
        );
        let pane = first_pane(tmux, &["-s", "-t", name], Instant::now() + SETUP_WAIT);
        assert!(tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", "lead"]).0);
        let dir = home.join("sessions").join(name);
        std::fs::create_dir_all(&dir).expect("a session dir");
        std::fs::write(
            dir.join("meta"),
            format!(
                "session={name}\nsession_id=44444444-4444-4444-4444-44444444444{index}\n\
                 session_id_origin=session\nwork_dir={home}\norigin={home}\nmode=local\n\
                 layout=vertical\nconfig={config}\nmain_pane={pane}\ntmux_server_kind=socket\n\
                 tmux_server={socket}\nschema=2\nseat.main=lead\nprofile.main=fake\n\
                 agent_bin.main=sh\n",
                home = home.display(),
                config = config.display(),
                socket = socket.display(),
            ),
        )
        .expect("a v2 meta");
    }
    assert!(
        tmux(&[
            "set-environment",
            "-g",
            "AE_HOME",
            &home.display().to_string()
        ])
        .0,
        "the server-owned continuation inherits the private state root"
    );
    assert_eq!(live(), names.len(), "every session runs before the stop");
    let out = finish(
        ae().env("AE_HOME", &home)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .args([ae::cli::STOP, "all", "-y"])
            .stdin(Stdio::null())
            .stdout(closed_pipe())
            .stderr(Stdio::piped()),
        &format!("_stop all -y over {names:?}, stdout closed"),
    );
    let left = live();
    (out, left)
}

/// `stop all` stops one session at a time and reports each on stdout. A
/// reader that leaves after a report is not a fleet stopped: while a session
/// is still running the stop FAILED (`EXIT_FAILED`) and goes no further, so
/// nothing is destroyed unwatched; once the last session is down it is the
/// success it is, said quietly.
#[test]
fn a_fleet_stop_whose_reader_left_succeeds_only_once_the_last_session_is_down() {
    for (tag, names, code, left) in [
        ("fleet1", &["pqa"][..], 0, 0),
        (
            "fleet2",
            &["pqa", "pqb"][..],
            i32::from(ae::entry::EXIT_FAILED),
            1,
        ),
    ] {
        let (out, live) = fleet_stop(tag, names);
        let said = String::from_utf8_lossy(&out.stderr);
        assert!(!said.contains("Broken pipe"), "{names:?}: stderr {said:?}");
        assert_eq!(out.status.code(), Some(code), "{names:?}: stderr {said:?}");
        assert_eq!(
            live, left,
            "{names:?}: the first session is stopped, and nothing after the reader left"
        );
    }
}

/// A command that fails without a readable stdout keeps its exit and its
/// diagnosis: a closed stdout is never a reason to succeed.
#[test]
fn a_closed_stdout_does_not_turn_a_failure_into_success() {
    let control = run(&["--frobnicate"], Stdio::piped());
    assert_eq!(control.status.code(), Some(2), "control {control:?}");
    let closed = run(&["--frobnicate"], closed_pipe().into());
    assert_eq!(
        closed.status.code(),
        control.status.code(),
        "stderr {:?}",
        String::from_utf8_lossy(&closed.stderr)
    );
    assert_eq!(closed.stderr, control.stderr, "the same diagnosis");
}

/// Only STDOUT's reader leaving is normal: a closed stderr under a failing
/// command is still a failure.
#[test]
fn a_closed_stderr_is_still_a_failure() {
    let out = finish(
        ae().arg("--frobnicate")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(closed_pipe()),
        "--frobnicate, stderr closed",
    );
    assert!(!out.status.success(), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
}

/// A stdout that fails for another reason than a gone reader is still an
/// error ae reports and fails on.
#[cfg(target_os = "linux")]
#[test]
fn a_stdout_that_fails_for_another_reason_is_still_reported() {
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("/dev/full");
    let out = finish(
        ae().arg("version")
            .stdin(Stdio::null())
            .stdout(full)
            .stderr(Stdio::piped()),
        "version > /dev/full",
    );
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "stderr {said:?}");
    assert!(
        said.contains("os error 28"),
        "the device's error is named: {said:?}"
    );
}

/// How a stdout handed to [`ae::run_program`] fails: at a write, at a flush
/// after accepting every write, or after accepting part of one write.
#[derive(Clone, Copy)]
enum Fault {
    Write(std::io::ErrorKind),
    Flush(std::io::ErrorKind),
    Partial(std::io::ErrorKind),
}

/// A stdout that fails once as its [`Fault`] says and keeps failing after,
/// counting every write that still reaches it once it has failed.
struct Stdout {
    fault: Fault,
    failed: bool,
    accepted: usize,
    writes_after_failure: usize,
}

impl Stdout {
    fn new(fault: Fault) -> Self {
        Self {
            fault,
            failed: false,
            accepted: 0,
            writes_after_failure: 0,
        }
    }

    fn kind(&self) -> std::io::ErrorKind {
        match self.fault {
            Fault::Write(kind) | Fault::Flush(kind) | Fault::Partial(kind) => kind,
        }
    }
}

impl std::io::Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.failed {
            self.writes_after_failure += 1;
            return Err(self.kind().into());
        }
        match self.fault {
            Fault::Flush(_) => Ok(buf.len()),
            Fault::Partial(_) if self.accepted == 0 && buf.len() > 1 => {
                self.accepted = 1;
                Ok(1)
            }
            Fault::Write(kind) | Fault::Partial(kind) => {
                self.failed = true;
                Err(kind.into())
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.failed {
            return Err(self.kind().into());
        }
        if let Fault::Flush(kind) = self.fault {
            self.failed = true;
            return Err(kind.into());
        }
        Ok(())
    }
}

fn whole(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

/// The whole-argv boundary the binary exits through: a gone reader, met at a
/// write, at a flush or partway through a write, ends `--version` with its
/// success, says nothing, and sends no further write into the closed stream.
#[test]
fn the_program_boundary_ends_normally_wherever_stdout_meets_its_gone_reader() {
    use std::io::ErrorKind::BrokenPipe;
    for (fault, what) in [
        (Fault::Write(BrokenPipe), "write"),
        (Fault::Flush(BrokenPipe), "flush"),
        (Fault::Partial(BrokenPipe), "partial write"),
    ] {
        let mut out = Stdout::new(fault);
        let mut err = Vec::new();
        let code = ae::run_program(Some("ae"), &whole(&["--version"]), &mut out, &mut err);
        assert!(
            matches!(&code, Ok(0)),
            "{what}: a gone reader is a normal end: {code:?}"
        );
        assert!(out.failed, "{what}: the fault was met");
        assert!(
            err.is_empty(),
            "{what}: {:?}",
            String::from_utf8_lossy(&err)
        );
        assert_eq!(
            out.writes_after_failure, 0,
            "{what}: nothing more is written into a closed stdout"
        );
    }
}

/// Any other stdout failure, at a write or at a flush, is still an error at
/// the boundary, with its own kind.
#[test]
fn the_program_boundary_still_fails_on_any_other_stdout_error() {
    use std::io::ErrorKind::Other;
    for (fault, what) in [
        (Fault::Write(Other), "write"),
        (Fault::Flush(Other), "flush"),
    ] {
        let mut out = Stdout::new(fault);
        let mut err = Vec::new();
        let code = ae::run_program(Some("ae"), &whole(&["--version"]), &mut out, &mut err);
        assert!(
            matches!(&code, Err(ae::Error::Io(cause)) if cause.kind() == Other),
            "{what}: {code:?}"
        );
    }
}

/// A gone STDERR reader is not a gone stdout reader: the boundary still fails.
#[test]
fn the_program_boundary_still_fails_when_stderr_has_no_reader() {
    use std::io::ErrorKind::BrokenPipe;
    let mut out = Vec::new();
    let mut err = Stdout::new(Fault::Write(BrokenPipe));
    let code = ae::run_program(Some("ae"), &whole(&["--frobnicate"]), &mut out, &mut err);
    assert!(
        matches!(&code, Err(ae::Error::Io(cause)) if cause.kind() == BrokenPipe),
        "{code:?}"
    );
    assert!(out.is_empty(), "{:?}", String::from_utf8_lossy(&out));
}
