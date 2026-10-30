//! Acceptance tests for the app-first FIRST USE (slice entry3, #230 O7 S3):
//! bare `ae` with NO recorded session creates one from the current directory
//! and lands in the app on it.
//!
//! The contract under test:
//! * outside tmux, with a terminal at BOTH ends, a state root that records no
//!   session creates one named after the canonical cwd (every char outside
//!   `[A-Za-z0-9_-]` becomes `-`, a routed verb takes `-1`), records that
//!   directory as its origin, says so in ONE stderr line, and opens the app on
//!   it; an unrecorded live tmux session elsewhere on the server blocks
//!   nothing;
//! * a live tmux session of the derived name is the launch's own refusal: exit
//!   1, nothing recorded, no app;
//! * everywhere Rule T fails (a pipe, `--no-attach`, `--tmux`, a pane inside
//!   tmux) the entry is today's, refusal line and exit code included.
//!
//! Black-box: real private tmux servers, real launches, real terminals.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures own private filesystem, environment and tmux state"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::cli::OwnedScratch;
use super::parity::{
    Invocation,
    capture::{ExitOutcome, raw},
};
use super::phase2::run_tmux;

const WAIT: Duration = Duration::from_secs(30);

/// The mark that follows the session the app was opened on.
const HOME: &str = "◂";

/// Today's answer to a bare `ae` that has nothing to open or attach.
const REFUSAL: &str = "ae: no running ae session. Start one with: ae <name>";

const CONFIG: &str = "[profiles]\nidle = \"sleep 600\"\n[roster]\nlead = idle\n\
    [workspace]\nmain = lead\nlayout = vertical\n";

struct Ran {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// One private tmux server family, one EMPTY state root and one project
/// directory whose name the test picks. `host` is a tmux session ae never
/// records: it only gives the OUTSIDE panes a tty.
struct Rig {
    scratch: OwnedScratch,
    home: PathBuf,
    project: PathBuf,
    socket: PathBuf,
    panes: PathBuf,
}

impl Rig {
    /// The panes live on the entry's own server, so `host` is a live,
    /// unrecorded session there.
    fn new(tag: &str, project: &str) -> Self {
        Self::build(tag, project, false)
    }

    /// The entry's server stays EMPTY; the panes live on a second server.
    fn split(tag: &str, project: &str) -> Self {
        Self::build(tag, project, true)
    }

    fn build(tag: &str, project: &str, separate_panes: bool) -> Self {
        let mut scratch = OwnedScratch::root("entryfirst", tag);
        let home = scratch.join("state");
        let project = scratch.join(project);
        let socket = scratch.join("sock");
        let panes = if separate_panes {
            scratch.join("host-sock")
        } else {
            socket.clone()
        };
        scratch.add_tmux_server(socket.clone());
        if separate_panes {
            scratch.add_tmux_server(panes.clone());
        }
        std::fs::create_dir_all(&home).expect("an isolated state root");
        std::fs::create_dir_all(&project).expect("an isolated project");
        std::fs::write(home.join("config"), CONFIG).expect("a harmless configuration");
        let rig = Self {
            scratch,
            home,
            project,
            socket,
            panes,
        };
        assert!(
            rig.pane_tmux(&["-f", "/dev/null", "new-session", "-d", "-s", "host"])
                .0,
            "a private tmux server for the panes"
        );
        assert!(
            rig.pane_tmux(&["set-option", "-g", "default-size", "160x45"])
                .0,
            "room for the frame"
        );
        rig
    }

    fn tmux(&self, tail: &[&str]) -> (bool, String) {
        self.tmux_on(&self.socket, tail)
    }

    fn pane_tmux(&self, tail: &[&str]) -> (bool, String) {
        self.tmux_on(&self.panes, tail)
    }

    fn tmux_on(&self, socket: &Path, tail: &[&str]) -> (bool, String) {
        let mut args = vec!["-S".to_owned(), socket.display().to_string()];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &self.scratch)
    }

    fn env(&self) -> [(String, String); 9] {
        [
            ("HOME".to_owned(), self.scratch.display().to_string()),
            ("AE_HOME".to_owned(), self.home.display().to_string()),
            (
                "CONFIG_FILE".to_owned(),
                self.home.join("config").display().to_string(),
            ),
            (
                "AE_TMUX_SERVER".to_owned(),
                self.socket.display().to_string(),
            ),
            ("AE_TMUX_SERVER_KIND".to_owned(), "socket".to_owned()),
            ("AE_NO_AUTOSTART".to_owned(), "1".to_owned()),
            ("TMUX_TMPDIR".to_owned(), self.scratch.display().to_string()),
            (
                "AE_TEST_GO_USAGE_URL".to_owned(),
                "http://127.0.0.1:9/".to_owned(),
            ),
            (
                "AE_TEST_COPILOT_USAGE_URL".to_owned(),
                "http://127.0.0.1:9/".to_owned(),
            ),
        ]
    }

    /// The real entry from the test process, in the project: no terminal at
    /// either end, a cleared environment.
    fn run(&self, args: &[&str], tag: &str) -> Ran {
        let inherited_path = std::env::var("PATH").expect("setup: PATH");
        let mut call = Invocation::new(env!("CARGO_BIN_EXE_ae"))
            .env_cleared()
            .env("PATH", inherited_path)
            .env("SHELL", "/bin/sh");
        for (key, value) in self.env() {
            call = call.env(key, value);
        }
        for arg in args {
            call = call.arg(arg);
        }
        let out = self.scratch.join(format!("{tag}.stdout"));
        let err = self.scratch.join(format!("{tag}.stderr"));
        let status = raw::run(&call, &self.project, &out, &err)
            .unwrap_or_else(|why| panic!("the real ae entry: {why}"));
        let code = match status.outcome() {
            ExitOutcome::Code(code) => Some(code),
            ExitOutcome::Signalled => None,
        };
        Ran {
            code,
            stdout: String::from_utf8_lossy(&std::fs::read(&out).unwrap_or_default()).into_owned(),
            stderr: String::from_utf8_lossy(&std::fs::read(&err).unwrap_or_default()).into_owned(),
        }
    }

    fn recorded(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.home.join("sessions"))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .filter(|name| !name.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// One value of a session's record.
    fn meta(&self, name: &str, key: &str) -> String {
        let meta = std::fs::read_to_string(self.home.join("sessions").join(name).join("meta"))
            .expect("a session record");
        meta.lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("a recorded {key}"))
            .to_owned()
    }

    fn canonical_project(&self) -> PathBuf {
        std::fs::canonicalize(&self.project).expect("a canonical project directory")
    }

    /// A window in `host` running `command` with this rig's ae environment.
    /// Its pane id.
    fn pane(&self, command: &str) -> String {
        let mut args: Vec<String> = vec![
            "new-window".to_owned(),
            "-d".to_owned(),
            "-P".to_owned(),
            "-F".to_owned(),
            "#{pane_id}".to_owned(),
            "-t".to_owned(),
            "=host".to_owned(),
        ];
        for (key, value) in self.env() {
            args.push("-e".to_owned());
            args.push(format!("{key}={value}"));
        }
        args.push(command.to_owned());
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        let (ok, pane) = self.pane_tmux(&words);
        assert!(ok, "a pane in host: {pane}");
        pane.trim().to_owned()
    }

    fn screen(&self, pane: &str) -> String {
        let (ok, screen) = self.pane_tmux(&["capture-pane", "-p", "-t", pane]);
        assert!(ok, "capture {pane}: {screen}");
        screen
    }

    fn wait(&self, pane: &str, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + WAIT;
        loop {
            let screen = self.screen(pane);
            if met(&screen) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}; terminal screen:\n{screen}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Run `ae args` in the project on a terminal, outside tmux unless
    /// `inside`, with stderr in `err`; the exit code lands in `rc`.
    fn terminal(&self, args: &[&str], tag: &str, inside: bool) -> Pane {
        let rc = self.scratch.join(format!("{tag}.rc"));
        let err = self.scratch.join(format!("{tag}.err"));
        let gate = self.scratch.join(format!("{tag}.go"));
        let mut line = format!(
            "while [ ! -e {} ]; do sleep 0.05; done; cd {} || exit 3; ",
            quote(&gate.display().to_string()),
            quote(&self.project.display().to_string())
        );
        if !inside {
            line.push_str("env -u TMUX -u TMUX_PANE ");
        }
        line.push_str(&quote(env!("CARGO_BIN_EXE_ae")));
        for arg in args {
            line.push(' ');
            line.push_str(&quote(arg));
        }
        let _ = write!(
            line,
            " 2>{}; printf '%s' \"$?\" > {}; exec cat >/dev/null",
            quote(&err.display().to_string()),
            quote(&rc.display().to_string())
        );
        let pane = self.pane(&line);
        std::fs::write(&gate, "").expect("release the pane");
        Pane { id: pane, rc, err }
    }

    /// Quit the app drawn in `pane` and hand back the exit code it left.
    fn quit(&self, pane: &Pane) -> String {
        self.pane_tmux(&["send-keys", "-t", &pane.id, "q", "q"]);
        let code = pane.code();
        // ae's own landing never says the hand-typed `ae app` hint (#230 O7 S4).
        let said = std::fs::read_to_string(&pane.err).unwrap_or_default();
        assert!(
            !said.contains("plain ae opens the app"),
            "a landing says no hint: {said:?}"
        );
        code
    }
}

/// One entry run on a terminal: where its exit code and its stderr land.
struct Pane {
    id: String,
    rc: PathBuf,
    err: PathBuf,
}

impl Pane {
    /// The exit code the run left behind, once it wrote one.
    fn code(&self) -> String {
        let until = Instant::now() + WAIT;
        loop {
            if let Ok(code) = std::fs::read_to_string(&self.rc)
                && !code.trim().is_empty()
            {
                return code.trim().to_owned();
            }
            assert!(
                Instant::now() < until,
                "the entry ended: {}",
                self.rc.display()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn stderr(&self) -> String {
        std::fs::read_to_string(&self.err).unwrap_or_default()
    }
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The app row carries its home-session mark and the sidebar rule.
fn lands(screen: &str, name: &str) -> bool {
    screen
        .lines()
        .any(|row| row.contains(&format!("{name} {HOME}")) && row.contains('│'))
}

fn has_frame(screen: &str) -> bool {
    screen
        .lines()
        .any(|row| row.contains(&format!(" {HOME}")) && row.contains('│'))
}

/// Whether `stderr` leads with the one line first use says for the session it
/// made in `dir`, and says it once (the app adds its own farewell after).
fn says_first_use_once(stderr: &str, name: &str, dir: &Path) -> bool {
    stderr.starts_with(&first_use_line(name, dir)) && stderr.matches("first use:").count() == 1
}

/// The one line first use says, for the session it made in `dir`.
fn first_use_line(name: &str, dir: &Path) -> String {
    format!(
        "first use: created session {name} in {}; old view: ae {name} --tmux\n",
        dir.display()
    )
}

/// No recorded session, a terminal at both ends, outside tmux: the entry
/// creates ONE session named after the canonical cwd, records that directory,
/// says so once on stderr and opens the app on it. `host` is a live unrecorded
/// tmux session on the same server and blocks nothing.
#[test]
fn no_recorded_session_creates_one_from_the_cwd_and_lands_in_the_app() {
    let rig = Rig::new("create", "My Project.v2");
    assert!(rig.recorded().is_empty(), "a fresh state root records none");

    let pane = rig.terminal(&[], "first", false);
    rig.wait(&pane.id, "the app on the new session", |screen| {
        lands(screen, "My-Project-v2")
    });
    assert_eq!(rig.quit(&pane), "0", "the app quits cleanly");
    assert_eq!(rig.recorded(), ["My-Project-v2"], "exactly one session");
    assert_eq!(
        rig.meta("My-Project-v2", "origin"),
        rig.canonical_project().display().to_string(),
        "the origin is the canonical cwd"
    );
    assert!(
        says_first_use_once(&pane.stderr(), "My-Project-v2", &rig.canonical_project()),
        "ONE first-use line, first:\n{}",
        pane.stderr()
    );
}

/// A directory named like an ae command would make a session nothing can
/// address: the name takes the `-1` suffix.
#[test]
fn a_directory_named_like_an_ae_command_gets_the_suffixed_name() {
    let rig = Rig::new("verb", "list");
    let pane = rig.terminal(&[], "verb", false);
    rig.wait(&pane.id, "the app on the suffixed session", |screen| {
        lands(screen, "list-1")
    });
    assert_eq!(rig.quit(&pane), "0");
    assert_eq!(rig.recorded(), ["list-1"]);
    assert!(
        says_first_use_once(&pane.stderr(), "list-1", &rig.canonical_project()),
        "{}",
        pane.stderr()
    );
}

/// A live tmux session of the derived name is not an ae session: the launch
/// refuses as it always has (exit 1), nothing is recorded and no app opens —
/// no silent second name.
#[test]
fn a_live_unrecorded_session_of_that_name_is_the_launchs_own_refusal() {
    let rig = Rig::new("taken", "taken");
    assert!(
        rig.tmux(&["new-session", "-d", "-s", "taken"]).0,
        "an unrecorded tmux session named like the project"
    );

    let pane = rig.terminal(&[], "taken", false);
    assert_eq!(pane.code(), "1", "the launch's refusal exits 1");
    assert!(
        pane.stderr()
            .contains("tmux session 'taken' exists but is not an ae session."),
        "the launch's own refusal:\n{}",
        pane.stderr()
    );
    assert!(!pane.stderr().contains("first use:"), "nothing was created");
    let screen = rig.screen(&pane.id);
    assert!(!has_frame(&screen), "no app opens:\n{screen}");
    assert!(rig.recorded().is_empty(), "nothing was recorded");
}

/// Everywhere Rule T fails the entry is today's, byte for byte: a pipe,
/// `--no-attach` and `--tmux` on a terminal each print the refusal, exit 1 and
/// create nothing; so does a pane INSIDE tmux with no recorded session (the
/// residual: its `ae list` is S1's, not first use's).
#[test]
fn rule_t_failing_keeps_todays_refusal_and_creates_nothing() {
    let rig = Rig::split("rulet", "proj");

    let piped = rig.run(&[], "piped");
    assert_eq!(piped.code, Some(1), "{}{}", piped.stdout, piped.stderr);
    assert!(piped.stdout.is_empty(), "{}", piped.stdout);
    assert_eq!(piped.stderr, format!("{REFUSAL}\n"));

    for (tag, args) in [("detached", ["--no-attach"]), ("tmux", ["--tmux"])] {
        let pane = rig.terminal(&args, tag, false);
        let screen = rig.wait(&pane.id, "today's refusal", |_| {
            std::fs::metadata(&pane.rc).is_ok()
        });
        assert_eq!(pane.code(), "1", "{tag} exits 1:\n{screen}");
        assert_eq!(pane.stderr(), format!("{REFUSAL}\n"), "{tag}");
        assert!(!has_frame(&screen), "{tag} draws no app:\n{screen}");
    }

    assert!(
        rig.recorded().is_empty(),
        "no outside run created a session"
    );

    let inside = Rig::new("inside", "proj");
    let pane = inside.terminal(&[], "inside", true);
    let screen = inside.wait(&pane.id, "the inside-tmux run ends", |_| {
        std::fs::metadata(&pane.rc).is_ok()
    });
    assert!(!has_frame(&screen), "inside tmux draws no app:\n{screen}");
    assert!(
        !pane.stderr().contains("first use:"),
        "inside tmux creates nothing:\n{}",
        pane.stderr()
    );
    assert!(inside.recorded().is_empty(), "inside tmux created nothing");
}
