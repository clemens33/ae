//! Frozen acceptance for the hint a hand-typed `ae app` prints (#230 O7 S4).
//! Oracle: the brief. A human-typed `ae app` that opens the app prints ONE
//! stderr line, exactly `ae app: plain ae opens the app`; ae's own starts (the
//! landings' marker, an exec restart's carrier, the chat window) never do; and
//! every refusal keeps today's bytes. Reuses the Settings rig; each pane runs
//! `ae app` with stderr in a file, so the stream is judged apart from the
//! screen.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::{Rig, WAIT, quote};

const SAYS: &str = "ae app: plain ae opens the app";
const CLOSED_IN_TMUX: &str = "ae app closed - run: ae app (in a chat window, prefix h reopens it)";
const CLOSED_OUTSIDE: &str = "ae app closed - run: ae app";
const NO_TERMINAL: &str = "ae app draws a terminal UI; use ae chat to print the lane\n";
const USAGE: &str = "Usage: ae app [session]\n\n  session   the home session, selected first (default: the session this pane belongs to)\n";

static SERIAL: AtomicUsize = AtomicUsize::new(0);

/// One `ae app …` typed by hand in a window of the rig's session: stderr in a
/// file, the exit code in another.
struct Typed {
    pane: String,
    err: PathBuf,
    rc: PathBuf,
}

/// `outside` drops the tmux environment the window inherits, so the app sees
/// a terminal and no tmux; otherwise it runs in an unstamped pane of the
/// rig's own server.
fn typed(rig: &Rig, outside: bool, operands: &[&str]) -> Typed {
    launched(rig, outside, operands, true)
}

/// [`typed`], or with stderr left on the terminal when `to_file` is false.
fn launched(rig: &Rig, outside: bool, operands: &[&str], to_file: bool) -> Typed {
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    let err = rig.root.join(format!("typed-{serial}.err"));
    let rc = rig.root.join(format!("typed-{serial}.rc"));
    let root = quote(&rig.root.to_string_lossy());
    let mut line = String::from("env ");
    if outside {
        line.push_str("-u TMUX -u TMUX_PANE ");
    }
    let _ = write!(
        line,
        "-u CLAUDE_CONFIG_DIR -u CODEX_HOME HOME={root} AE_HOME={root} CONFIG_FILE={} AE_TMUX_SERVER_KIND=socket AE_TMUX_SERVER={} AE_NO_AUTOSTART=1 {} app",
        quote(&rig.root.join("config").to_string_lossy()),
        quote(&rig.socket.to_string_lossy()),
        quote(env!("CARGO_BIN_EXE_ae")),
    );
    for operand in operands {
        let _ = write!(line, " {}", quote(operand));
    }
    if to_file {
        let _ = write!(line, " 2>{}", quote(&err.to_string_lossy()));
    }
    let _ = write!(
        line,
        "; printf '%s' \"$?\" > {}; sleep 30",
        quote(&rc.to_string_lossy()),
    );
    let cwd = rig.root.to_string_lossy();
    let pane = rig
        .tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &rig.name,
            "-c",
            &cwd,
            &line,
        ])
        .trim()
        .to_owned();
    rig.tmux(&["resize-window", "-t", &pane, "-x", "220", "-y", "60"]);
    Typed { pane, err, rc }
}

impl Typed {
    /// The exit code the command left, once it wrote one.
    fn exit(&self) -> String {
        let until = Instant::now() + WAIT;
        loop {
            if let Ok(code) = fs::read_to_string(&self.rc)
                && !code.trim().is_empty()
            {
                return code.trim().to_owned();
            }
            assert!(
                Instant::now() < until,
                "the command ended: {}",
                self.rc.display()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Wait until the app has drawn the fleet, quit it, and hand back the
    /// exit code and everything it wrote on stderr.
    fn quit(&self, rig: &Rig) -> (String, String) {
        rig.wait(&self.pane, WAIT, "the app draws the fleet", |screen| {
            screen.contains(&rig.name)
        });
        rig.keys(&self.pane, "C-c");
        let code = self.exit();
        (code, self.stderr())
    }

    fn stderr(&self) -> String {
        fs::read_to_string(&self.err).expect("the stderr file")
    }
}

/// RED on base: nothing prints the hint there.
#[test]
fn a_hand_typed_ae_app_that_opens_says_the_hint_once() {
    let rig = Rig::new("hintsays");
    let name = rig.name.clone();
    for (outside, operands, closed) in [
        (false, vec![name.as_str()], CLOSED_IN_TMUX),
        (false, vec![], CLOSED_IN_TMUX),
        (true, vec![name.as_str()], CLOSED_OUTSIDE),
        (true, vec![], CLOSED_OUTSIDE),
    ] {
        let run = typed(&rig, outside, &operands);
        let (code, err) = run.quit(&rig);
        let case = format!("outside {outside}, operands {operands:?}");
        assert_eq!(code, "0", "{case}: the app quits cleanly");
        assert_eq!(
            err,
            format!("{SAYS}\n{closed}\n"),
            "{case}: the hint is the first stderr line, once, then today's exit hint"
        );
    }
}

/// ae's own starts say nothing: the landings' marker (with and without a
/// session), and an exec restart's carrier — whose text is hostile and is
/// ignored as a view. RED on base for the marker (unknown there); the
/// carrier case fails any implementation that always prints.
#[test]
fn an_app_start_ae_makes_itself_says_nothing() {
    let rig = Rig::new("hintown");
    let name = rig.name.clone();
    for operands in [
        vec![name.as_str(), "--app-own"],
        vec!["--app-own"],
        vec![name.as_str(), "--app-view", "not-a-view"],
        vec!["--app-view", "not-a-view"],
    ] {
        let run = typed(&rig, true, &operands);
        let (code, err) = run.quit(&rig);
        assert_eq!(code, "0", "{operands:?}: the app quits cleanly");
        assert_eq!(
            err,
            format!("{CLOSED_OUTSIDE}\n"),
            "{operands:?}: only today's exit hint"
        );
    }
}

/// The normal screen the running app left behind: what was on the terminal
/// before it took the alternate screen, which is where a start's stderr lives.
fn behind_the_app(rig: &Rig, pane: &str) -> String {
    rig.tmux(&["capture-pane", "-p", "-a", "-t", pane])
}

/// The app in the session's chat window (`chat = app`) is ae's own start: its
/// pane carries the `@ae_console` stamp, so nothing is said on the screen it
/// left behind. The control is a hand-typed start whose stderr stays on the
/// terminal: the same capture shows its hint, so the empty one means something.
/// The control is RED on base; the chat window is a guard.
#[test]
fn the_chat_window_app_says_nothing() {
    let rig = Rig::new("hintchat");
    let name = rig.name.clone();
    let control = launched(&rig, true, &[name.as_str()], false);
    rig.wait(
        &control.pane,
        WAIT,
        "the typed app draws the fleet",
        |screen| screen.contains(&name),
    );
    let behind = behind_the_app(&rig, &control.pane);
    assert!(
        behind.contains(SAYS),
        "a typed start leaves its hint behind:\n{behind}"
    );
    let pane = rig.owner();
    rig.ready(&pane);
    let behind = behind_the_app(&rig, &pane);
    assert!(
        !behind.contains(SAYS),
        "the chat window left no hint behind:\n{behind}"
    );
}

/// Refusals keep today's bytes and say no hint. GREEN on base.
#[test]
fn refusals_keep_today_s_bytes_and_say_no_hint() {
    let rig = Rig::new("hintrefuse");
    let missing = typed(&rig, true, &["nosuch"]);
    assert_eq!(missing.exit(), "1");
    assert_eq!(missing.stderr(), "ae app: no session named nosuch\n");
    let usage = typed(&rig, true, &["--bogus"]);
    assert_eq!(usage.exit(), "2");
    assert_eq!(usage.stderr(), USAGE);
    for tail in [&["app"][..], &["app", "lane"][..]] {
        let out = crate::cli::ae()
            .args(tail)
            .output()
            .expect("the ae binary should run");
        assert_eq!(out.status.code(), Some(1), "{tail:?}");
        assert!(out.stdout.is_empty(), "{tail:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            NO_TERMINAL,
            "{tail:?}"
        );
    }
}

/// The internal marker is accepted once, in place — before a carrier — and
/// refused as usage (exit 2, today's text) anywhere else. Without a terminal
/// an accepted marker reaches today's no-terminal refusal. RED on base for the
/// accepted forms.
#[test]
fn the_internal_marker_is_accepted_once_and_in_place() {
    let run = |tail: &[&str]| {
        let out = crate::cli::ae()
            .arg("app")
            .args(tail)
            .output()
            .expect("the ae binary should run");
        assert!(out.stdout.is_empty(), "{tail:?}");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    for tail in [
        &["--app-own"][..],
        &["lane", "--app-own"],
        &["--app-own", "--app-view", "x"],
        &["lane", "--app-own", "--app-view", "x"],
    ] {
        assert_eq!(run(tail), (Some(1), NO_TERMINAL.to_owned()), "{tail:?}");
    }
    for tail in [
        &["--app-own", "--app-own"][..],
        &["lane", "--app-own", "--app-own"],
        &["--app-own", "lane"],
        &["--app-view", "x", "--app-own"],
        &["lane", "--app-view", "x", "--app-own"],
        &["lane", "lane", "--app-own"],
    ] {
        assert_eq!(run(tail), (Some(2), USAGE.to_owned()), "{tail:?}");
    }
}
