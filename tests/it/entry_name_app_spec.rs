//! Independent acceptance tests for the app-first NAMED entry (slice entry2):
//! `ae <name>` on a terminal lands in the app with `<name>` selected.
//!
//! The contract under test:
//! * a RUNNING session opens the app on it and creates or resumes nothing;
//! * a STOPPED session resumes headless in its STORED working copy — the
//!   caller's cwd alone never repoints it — and the app opens on it;
//! * an UNKNOWN name starts in the caller's cwd (launch flags apply) and the
//!   app opens on it;
//! * an explicit `--dir` differing from the recorded origin is exit 2 naming
//!   the recorded directory when the line LANDS (running or stopped alike),
//!   while the same `--dir` is accepted; `--dir --tmux` still reads `--tmux`
//!   as the path;
//! * `--tmux` is today's launch/resume/attach exactly, and `--tmux
//!   --no-attach` with a name is exit 2 in either order, never a parse miss;
//! * Rule T: the app is reached only when stdin AND stdout are a terminal, the
//!   line carries no `--no-attach`, and the caller's pane is not a seat. A
//!   pipe, a seat pane or an unreadable pane keeps today's entry, another
//!   server keeps its attach hint, and an unrecorded caller session still
//!   lands because the app's home is the TARGET session;
//! * the orchestrator seat launch never opens an app, and `ae app` keeps its
//!   own arguments and refusal.
//!
//! Black-box: real private tmux servers, real launches, real terminals.
//! Nothing here names a route, a decision function or a fact struct.

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

/// `idle` gives an agent pane with no work; `orchestrator` is the seat the
/// dedicated entry launches.
const CONFIG: &str = "[profiles]\nidle = \"sleep 600\"\n[roster]\nlead = idle\n\
    orchestrator = idle\n[workspace]\nmain = lead\nlayout = vertical\n";

struct Ran {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Ran {
    fn out(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    fn err(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    fn all(&self) -> String {
        format!("{}{}", self.out(), self.err())
    }
}

/// One private tmux server family, one state root, one recorded project and
/// two other directories. `host` is a tmux session ae never records: it only
/// gives the OUTSIDE panes a tty.
struct Rig {
    scratch: OwnedScratch,
    home: PathBuf,
    project: PathBuf,
    elsewhere: PathBuf,
    other: PathBuf,
    socket: PathBuf,
    panes: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        Self::build(tag, false)
    }

    /// A rig whose entry server stays EMPTY: the panes live on a second
    /// private server, so an entry there sees a different tmux server.
    fn split(tag: &str) -> Self {
        Self::build(tag, true)
    }

    fn build(tag: &str, separate_panes: bool) -> Self {
        let mut scratch = OwnedScratch::root("entrynameapp", tag);
        let home = scratch.join("state");
        let project = scratch.join("project");
        let elsewhere = scratch.join("elsewhere");
        let other = scratch.join("other");
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
        for dir in [&project, &elsewhere, &other] {
            std::fs::create_dir_all(dir).expect("an isolated directory");
        }
        std::fs::write(home.join("config"), CONFIG).expect("a harmless configuration");
        let rig = Self {
            scratch,
            home,
            project,
            elsewhere,
            other,
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

    fn env(&self) -> [(String, String); 8] {
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
        ]
    }

    /// The real entry from the test process: no terminal at either end, a
    /// cleared environment.
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
            stdout: std::fs::read(&out).unwrap_or_default(),
            stderr: std::fs::read(&err).unwrap_or_default(),
        }
    }

    fn launched(&self, args: &[&str], tag: &str) -> Option<i32> {
        self.run(args, tag).code
    }

    fn launch(&self, name: &str) {
        assert_eq!(
            self.launched(&["--local", name, "--no-attach"], &format!("launch-{name}")),
            Some(0),
            "launch {name}"
        );
    }

    fn stop(&self, name: &str) {
        let (ok, why) = self.tmux(&["kill-session", "-t", &format!("={name}")]);
        assert!(ok, "stop {name}: {why}");
    }

    fn has_session(&self, name: &str) -> bool {
        self.tmux(&["has-session", "-t", &format!("={name}")]).0
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.home.join("sessions").join(name)
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

    /// The pane the session was launched on, as its own record names it.
    fn main_pane(&self, name: &str) -> String {
        let meta = std::fs::read_to_string(self.dir(name).join("meta")).expect("a session record");
        meta.lines()
            .find_map(|line| line.strip_prefix("main_pane="))
            .expect("a recorded main pane")
            .to_owned()
    }

    /// Where that pane's own process runs.
    fn main_pane_cwd(&self, name: &str) -> PathBuf {
        let pane = self.main_pane(name);
        let (ok, where_) = self.tmux(&["display", "-p", "-t", &pane, "#{pane_current_path}"]);
        assert!(ok, "the main pane answers: {where_}");
        PathBuf::from(where_.trim())
    }

    fn canonical(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).expect("a canonical scratch directory")
    }

    /// A window in `session` running `command`, with this rig's ae
    /// environment. Its pane id. tmux owns `TMUX_PANE`, so a case that needs a
    /// bogus one sets it INSIDE the command, where the shell keeps it.
    fn pane(&self, session: &str, command: &str) -> String {
        self.pane_on(&self.panes, session, command)
    }

    fn pane_on(&self, socket: &Path, session: &str, command: &str) -> String {
        let mut args: Vec<String> = vec![
            "new-window".to_owned(),
            "-d".to_owned(),
            "-P".to_owned(),
            "-F".to_owned(),
            "#{pane_id}".to_owned(),
            "-t".to_owned(),
            format!("={session}"),
        ];
        for (key, value) in self.env() {
            args.push("-e".to_owned());
            args.push(format!("{key}={value}"));
        }
        args.push(command.to_owned());
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        let (ok, pane) = self.tmux_on(socket, &words);
        assert!(ok, "a pane in {session}: {pane}");
        pane.trim().to_owned()
    }

    /// The terminal the pane's own process draws on.
    fn pane_tty(&self, pane: &str) -> String {
        let (ok, tty) = self.tmux(&["display", "-p", "-t", pane, "#{pane_tty}"]);
        assert!(ok, "the pane {pane} names its tty: {tty}");
        tty.trim().to_owned()
    }

    /// Every client attached to this rig's entry server.
    fn clients(&self) -> Vec<String> {
        self.tmux(&["list-clients", "-F", "#{client_tty}"])
            .1
            .lines()
            .map(|line| line.trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect()
    }

    fn screen(&self, pane: &str) -> String {
        self.screen_on(&self.panes, pane)
    }

    fn screen_on(&self, socket: &Path, pane: &str) -> String {
        let (ok, screen) = self.tmux_on(socket, &["capture-pane", "-p", "-t", pane]);
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

    /// True when the refusal names the directory the session records.
    fn names(screen: &str, dir: &Path) -> bool {
        let canonical = Self::canonical(dir);
        screen.contains(&dir.display().to_string())
            || screen.contains(&canonical.display().to_string())
    }

    /// Quit the app drawn in `pane` and hand back the exit code it left.
    fn quit(&self, pane: &str, rc: &Path) -> String {
        self.pane_tmux(&["send-keys", "-t", pane, "q", "q"]);
        wait_for_rc(rc)
    }
}

/// The exit code a pane's command left behind, once it wrote one.
fn wait_for_rc(rc: &Path) -> String {
    let until = Instant::now() + WAIT;
    loop {
        if let Ok(code) = std::fs::read_to_string(rc)
            && !code.trim().is_empty()
        {
            return code.trim().to_owned();
        }
        assert!(Instant::now() < until, "the entry ended: {}", rc.display());
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// A pane body that waits for `gate`, so the fixture can stamp a pane option
/// before the entry reads it. The final `exec cat` holds the pane open after
/// the entry ends, so the screen and the exit code stay readable.
fn gated(line: &str, gate: &Path) -> String {
    format!(
        "while [ ! -e {} ]; do sleep 0.05; done; {line}",
        quote(&gate.display().to_string())
    )
}

/// The entry's own command line, run in a pane. `outside` drops the tmux
/// environment the pane inherits: the entry then sees a terminal, but no tmux.
fn command(args: &[&str], rc: &Path, outside: bool) -> String {
    let mut line = if outside {
        "env -u TMUX -u TMUX_PANE ".to_owned()
    } else {
        String::new()
    };
    line.push_str(&quote(env!("CARGO_BIN_EXE_ae")));
    for arg in args {
        line.push(' ');
        line.push_str(&quote(arg));
    }
    let _ = write!(
        line,
        "; printf '%s' \"$?\" > {}; exec cat >/dev/null",
        quote(&rc.display().to_string())
    );
    line
}

/// The same line, started in `cwd` (the pane's own cwd is not the caller's).
fn command_in(cwd: &Path, args: &[&str], rc: &Path, outside: bool) -> String {
    format!(
        "cd {} || exit 3; {}",
        quote(&cwd.display().to_string()),
        command(args, rc, outside)
    )
}

/// The app row carries its home-session mark and the sidebar rule.
fn lands(screen: &str, name: &str) -> bool {
    screen
        .lines()
        .any(|row| row.contains(&format!("{name} {HOME}")) && row.contains("│"))
}

fn has_frame(screen: &str) -> bool {
    screen
        .lines()
        .any(|row| row.contains(&format!(" {HOME}")) && row.contains("│"))
}

fn refused(screen: &str, name: &str) -> bool {
    !lands(screen, name)
}

#[test]
fn a_running_session_opens_the_app_on_it_and_touches_nothing() {
    let rig = Rig::new("running");
    rig.launch("one");
    let rc = rig.scratch.join("rc");
    let gate = rig.scratch.join("go");
    let pane = rig.pane("host", &gated(&command(&["one"], &rc, true), &gate));
    std::fs::write(&gate, "").expect("release the pane");
    rig.wait(&pane, "the app on the running session", |screen| {
        lands(screen, "one")
    });
    assert_eq!(rig.quit(&pane, &rc), "0", "the app exits cleanly");
    assert_eq!(
        rig.recorded(),
        vec!["one".to_owned()],
        "nothing was created"
    );
    assert!(
        rig.has_session("one"),
        "the running session was not restarted"
    );
    assert_eq!(
        rig.main_pane_cwd("one"),
        Rig::canonical(&rig.project),
        "a running session keeps its own working copy"
    );
}

#[test]
fn a_stopped_session_resumes_in_its_stored_working_copy_then_opens_the_app() {
    let rig = Rig::new("stopped");
    rig.launch("one");
    rig.stop("one");
    let rc = rig.scratch.join("rc");
    let gate = rig.scratch.join("go");
    let pane = rig.pane(
        "host",
        &gated(&command_in(&rig.elsewhere, &["one"], &rc, true), &gate),
    );
    std::fs::write(&gate, "").expect("release the pane");
    let screen = rig.wait(&pane, "the app on the resumed session", |screen| {
        lands(screen, "one")
    });
    let resumed_row = screen
        .lines()
        .find(|row| row.contains(&format!("one {HOME}")))
        .expect("the selected session row");
    assert!(
        !resumed_row.to_lowercase().contains("stopped"),
        "the app offers the resumed session as running; row:\n{resumed_row}"
    );
    assert_eq!(rig.quit(&pane, &rc), "0");
    assert!(rig.has_session("one"), "the session was resumed");
    assert_eq!(
        rig.main_pane_cwd("one"),
        Rig::canonical(&rig.project),
        "the stored working copy decides, never the caller's cwd"
    );
}

#[test]
fn an_unknown_name_starts_in_the_callers_cwd_then_opens_the_app() {
    let rig = Rig::new("unknown");
    let rc = rig.scratch.join("rc");
    let gate = rig.scratch.join("go");
    let pane = rig.pane(
        "host",
        &gated(&command_in(&rig.project, &["fresh"], &rc, true), &gate),
    );
    std::fs::write(&gate, "").expect("release the pane");
    rig.wait(&pane, "the app on the new session", |screen| {
        lands(screen, "fresh")
    });
    assert_eq!(rig.quit(&pane, &rc), "0");
    assert_eq!(rig.recorded(), vec!["fresh".to_owned()]);
    assert_eq!(
        rig.main_pane_cwd("fresh"),
        Rig::canonical(&rig.project),
        "an unknown name starts in the caller's cwd"
    );

    let rc = rig.scratch.join("rc-dir");
    let gate = rig.scratch.join("go-dir");
    let line = command_in(
        &rig.project,
        &["brandnew", "--dir", &rig.elsewhere.display().to_string()],
        &rc,
        true,
    );
    let pane = rig.pane("host", &gated(&line, &gate));
    std::fs::write(&gate, "").expect("release the pane");
    rig.wait(&pane, "the app on the second new session", |screen| {
        lands(screen, "brandnew")
    });
    assert_eq!(rig.quit(&pane, &rc), "0");
    assert_eq!(
        rig.main_pane_cwd("brandnew"),
        Rig::canonical(&rig.elsewhere),
        "an explicit --dir is the origin of a new session"
    );
}

#[test]
fn a_differing_dir_on_a_landing_line_is_usage_and_names_the_recorded_origin() {
    let rig = Rig::new("dir");
    rig.launch("one");
    let rc = rig.scratch.join("rc-running");
    let gate = rig.scratch.join("go-running");
    let line = command_in(
        &rig.elsewhere,
        &["one", "--dir", &rig.other.display().to_string()],
        &rc,
        true,
    );
    let pane = rig.pane("host", &gated(&line, &gate));
    std::fs::write(&gate, "").expect("release the pane");
    let code = wait_for_rc(&rc);
    let screen = rig.screen(&pane);
    assert_eq!(code, "2", "a differing --dir is usage; screen:\n{screen}");
    assert!(
        Rig::names(&screen, &rig.project),
        "the refusal names the recorded dir:\n{screen}"
    );
    assert!(refused(&screen, "one"), "no app on a refused origin");
    assert!(rig.has_session("one"), "a refusal changes nothing");

    rig.stop("one");
    let rc = rig.scratch.join("rc-stopped");
    let gate = rig.scratch.join("go-stopped");
    let line = command(
        &["one", "--dir", &rig.other.display().to_string()],
        &rc,
        true,
    );
    let pane = rig.pane("host", &gated(&line, &gate));
    std::fs::write(&gate, "").expect("release the pane");
    let code = wait_for_rc(&rc);
    let screen = rig.screen(&pane);
    assert_eq!(
        code, "2",
        "a stopped session refuses the same way; screen:\n{screen}"
    );
    assert!(
        Rig::names(&screen, &rig.project),
        "the refusal names the stored dir:\n{screen}"
    );
    assert!(!rig.has_session("one"), "a refused origin resumes nothing");

    let rc = rig.scratch.join("rc-same");
    let gate = rig.scratch.join("go-same");
    let line = command_in(
        &rig.elsewhere,
        &["one", "--dir", &rig.project.display().to_string()],
        &rc,
        true,
    );
    let pane = rig.pane("host", &gated(&line, &gate));
    std::fs::write(&gate, "").expect("release the pane");
    rig.wait(
        &pane,
        "the app on the session with the same --dir",
        |screen| lands(screen, "one"),
    );
    assert_eq!(rig.quit(&pane, &rc), "0");
    assert_eq!(
        rig.main_pane_cwd("one"),
        Rig::canonical(&rig.project),
        "the same --dir is accepted and the stored origin still decides"
    );
}

#[test]
fn tmux_is_todays_entry_exactly() {
    let rig = Rig::new("tmux");
    rig.launch("one");

    let rc = rig.scratch.join("rc-inplace");
    let gate = rig.scratch.join("go-inplace");
    let pane = rig.pane(
        "one",
        &gated(&command(&["one", "--tmux"], &rc, false), &gate),
    );
    std::fs::write(&gate, "").expect("release the pane");
    let screen = rig.wait(&pane, "today's in-place line", |screen| {
        screen.contains("you are in 'one'")
    });
    assert!(
        !has_frame(&screen),
        "no app on the --tmux path; screen:\n{screen}"
    );
    assert_eq!(wait_for_rc(&rc), "0");

    let attached = rig.run(&["one", "--tmux"], "tmux-no-terminal");
    assert!(
        !matches!(attached.code, Some(0 | 2)),
        "a terminal is still required: {:?}",
        attached.code
    );
    assert!(
        !attached.err().contains("ae app draws"),
        "not the app's refusal:\n{}",
        attached.err()
    );

    for order in [["--tmux", "--no-attach"], ["--no-attach", "--tmux"]] {
        let conflict = rig.run(&["one", order[0], order[1]], "tmux-conflict");
        assert_eq!(
            conflict.code,
            Some(2),
            "a contradiction is usage: {}",
            conflict.err()
        );
        assert!(conflict.stdout.is_empty(), "no output on a usage error");
        let text = conflict.err();
        assert!(
            text.contains("--tmux") && text.contains("--no-attach"),
            "it names both flags:\n{text}"
        );
        assert!(
            !text.contains("unknown flag"),
            "a contradiction, not a parse miss:\n{text}"
        );
    }

    let path = rig.run(&["brandnew", "--dir", "--tmux"], "dir-tmux");
    assert_eq!(
        path.code,
        Some(1),
        "--tmux is still the path: {}",
        path.err()
    );
    assert!(
        path.err().contains("does not exist or is not a directory"),
        "{}",
        path.err()
    );
    assert!(
        path.err().contains("--tmux"),
        "the path is named: {}",
        path.err()
    );
    assert!(!rig.dir("brandnew").exists(), "nothing was created");
}

#[test]
fn rule_t_keeps_todays_entry_everywhere_else() {
    let rig = Rig::new("rulet");
    rig.launch("one");

    let piped = rig.run(&["one"], "piped");
    assert!(
        !matches!(piped.code, Some(0 | 2)),
        "a pipe is no terminal: {:?}",
        piped.code
    );
    assert!(
        !piped.all().contains("ae app draws"),
        "the app is never entered from a pipe:\n{}",
        piped.all()
    );
    assert!(
        !has_frame(&piped.out()),
        "no frame on a pipe:\n{}",
        piped.out()
    );

    let rc = rig.scratch.join("rc-headless");
    let gate = rig.scratch.join("go-headless");
    let pane = rig.pane(
        "host",
        &gated(&command(&["one", "--no-attach"], &rc, true), &gate),
    );
    std::fs::write(&gate, "").expect("release the pane");
    let screen = rig.wait(&pane, "today's headless line", |screen| {
        screen.contains("Attach with:")
    });
    assert!(
        !has_frame(&screen),
        "--no-attach never lands; screen:\n{screen}"
    );
    assert_eq!(wait_for_rc(&rc), "0");

    let rc = rig.scratch.join("rc-seat");
    let gate = rig.scratch.join("go-seat");
    let seat = rig.pane("one", &gated(&command(&["one"], &rc, false), &gate));
    assert!(
        rig.tmux(&["set-option", "-p", "-t", &seat, "@ae_agent", "lead"])
            .0,
        "a seat pane carries @ae_agent"
    );
    std::fs::write(&gate, "").expect("release the pane");
    let screen = rig.wait(&seat, "a seat pane keeps today's entry", |screen| {
        screen.contains("you are in 'one'")
    });
    assert!(
        !has_frame(&screen),
        "a seat pane never lands; screen:\n{screen}"
    );
    assert_eq!(wait_for_rc(&rc), "0");

    let rc = rig.scratch.join("rc-lost");
    let gate = rig.scratch.join("go-lost");
    let lost = rig.pane(
        "one",
        &gated(
            &format!("TMUX_PANE=%99999 {}", command(&["one"], &rc, false)),
            &gate,
        ),
    );
    std::fs::write(&gate, "").expect("release the pane");
    assert_ne!(
        wait_for_rc(&rc),
        "0",
        "today's entry fails on a pane it cannot read"
    );
    let screen = rig.screen(&lost);
    assert!(
        !has_frame(&screen),
        "an unreadable pane never lands; screen:\n{screen}"
    );
    assert!(rig.has_session("one"), "the session is untouched");

    let split = Rig::split("rulet-other");
    split.launch("one");
    let rc = split.scratch.join("rc-other");
    let gate = split.scratch.join("go-other");
    let pane = split.pane_on(
        &split.panes,
        "host",
        &gated(&command(&["one"], &rc, false), &gate),
    );
    std::fs::write(&gate, "").expect("release the pane");
    let screen = split.wait(&pane, "another server keeps its hint", |screen| {
        screen.contains("is on another tmux server") && screen.contains("Attach with:")
    });
    assert!(
        !has_frame(&screen),
        "another server never lands; screen:\n{screen}"
    );
    assert_eq!(wait_for_rc(&rc), "0");
}

#[test]
fn an_unrecorded_caller_session_still_lands_on_the_named_session() {
    let rig = Rig::new("unrecorded");
    rig.launch("one");
    let rc = rig.scratch.join("rc");
    let gate = rig.scratch.join("go");
    let pane = rig.pane("host", &gated(&command(&["one"], &rc, false), &gate));
    std::fs::write(&gate, "").expect("release the pane");
    rig.wait(
        &pane,
        "the app from an unrecorded caller session",
        |screen| lands(screen, "one"),
    );
    assert_eq!(rig.quit(&pane, &rc), "0");
    assert_eq!(
        rig.recorded(),
        vec!["one".to_owned()],
        "the app's home is the target session"
    );
}

#[test]
fn a_headless_dir_conflict_keeps_todays_refusal_and_ae_app_its_own() {
    let rig = Rig::new("headless");
    rig.launch("one");
    let refused_dir = rig.run(
        &[
            "one",
            "--dir",
            &rig.other.display().to_string(),
            "--no-attach",
        ],
        "headless-dir",
    );
    assert_eq!(
        refused_dir.code,
        Some(1),
        "without a landing the refusal is unchanged: {}",
        refused_dir.err()
    );
    assert!(
        refused_dir.err().contains("different origin"),
        "{}",
        refused_dir.err()
    );
    assert!(
        Rig::names(&refused_dir.all(), &rig.project),
        "the refusal names the recorded dir:\n{}",
        refused_dir.all()
    );
    assert!(rig.has_session("one"), "the running session is untouched");

    let app = rig.run(&["app"], "app-no-terminal");
    assert_eq!(app.code, Some(1), "ae app keeps its refusal: {}", app.err());
    assert!(
        app.err().contains("ae app draws a terminal UI"),
        "{}",
        app.err()
    );
    assert_eq!(
        rig.recorded(),
        vec!["one".to_owned()],
        "ae app creates nothing"
    );
}

#[test]
fn the_orchestrator_seat_launch_never_opens_the_app() {
    let rig = Rig::new("orchestrator");

    // Without --no-attach: today's seat launch ATTACHES a client on the pane's
    // own terminal. A landing here would draw the app instead and attach
    // nothing, so the client is the evidence that the seat branch did not land.
    let rc = rig.scratch.join("rc-attach");
    let gate = rig.scratch.join("go-attach");
    let pane = rig.pane(
        "host",
        &gated(
            &command_in(&rig.project, &["orchestrator"], &rc, true),
            &gate,
        ),
    );
    std::fs::write(&gate, "").expect("release the pane");
    let tty = rig.pane_tty(&pane);
    let until = Instant::now() + WAIT;
    while !rig.clients().contains(&tty) {
        assert!(
            Instant::now() < until,
            "today's seat launch attaches a client on {tty}; screen:\n{}",
            rig.screen(&pane)
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    let screen = rig.screen(&pane);
    assert!(
        !has_frame(&screen),
        "the seat opens no app; screen:\n{screen}"
    );
    assert_eq!(rig.recorded(), vec!["orchestrator".to_owned()]);
    let (ok, why) = rig.tmux(&["detach-client", "-t", &tty]);
    assert!(ok, "detach the nested client: {why}");
    assert_eq!(wait_for_rc(&rc), "0", "today's attach returns 0 on detach");

    // With --no-attach: this already-running seat prints its attach hint, but no app.
    let rc = rig.scratch.join("rc-headless");
    let gate = rig.scratch.join("go-headless");
    let pane = rig.pane(
        "host",
        &gated(
            &command_in(&rig.project, &["orchestrator", "--no-attach"], &rc, true),
            &gate,
        ),
    );
    std::fs::write(&gate, "").expect("release the pane");
    let _ = rig.wait(
        &pane,
        "the running orchestrator seat to print its attach hint",
        |screen| screen.contains("Session 'orchestrator' is running. Attach with:"),
    );
    assert_eq!(wait_for_rc(&rc), "0");
    let screen = rig.screen(&pane);
    assert!(
        !has_frame(&screen),
        "the seat opens no app; screen:\n{screen}"
    );
    assert!(
        rig.clients().is_empty(),
        "a headless seat attaches no client"
    );
}
