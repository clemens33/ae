//! Independent acceptance tests for the app-first bare `ae` entry (slice
//! entry1). Black-box: real private tmux servers, real launches and real
//! terminals. Nothing here names a route, a decision function or a fact
//! struct, so an implementation may shape them freely.
//!
//! The contract under test (brief §rulings):
//! * bare `ae`, outside tmux, with a terminal at BOTH ends and at least one
//!   RECORDED session (running or stopped), opens the app fleet view and
//!   creates and resumes nothing;
//! * inside tmux on its own launch server, the same landing happens unless the
//!   calling pane is a SEAT pane (`@ae_agent`), which keeps `ae list`; a chat
//!   pane (`@ae_console`) is not a seat pane; an unreadable pane fails closed
//!   to `ae list`;
//! * everything else — no terminal, `--no-attach`, no recorded session, a
//!   caller session ae does not record — keeps the pre-change entry;
//! * `ae --tmux` is bare `ae` with the view fixed to today's (attach outside,
//!   list inside) and `--tmux --no-attach` is a usage error, in either order;
//! * the restore gate runs BEFORE the landing;
//! * every word `ae` answers itself is refused as a session name, `upgrade`
//!   included.

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

const WAIT: Duration = Duration::from_secs(20);

const CONFIG: &str = "[profiles]\nidle = \"sleep 600\"\n[roster]\nlead = idle\n\
    [workspace]\nmain = lead\nlayout = vertical\n";

/// What one real entry run left behind: its exit code (none when a signal
/// ended it) and both streams, kept as bytes because the refusal text is not
/// guaranteed to be UTF-8.
struct Ran {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// One private tmux server, one state root, one project. `host` is a tmux
/// session ae never records: it only gives the OUTSIDE panes a tty.
struct Rig {
    scratch: OwnedScratch,
    home: PathBuf,
    project: PathBuf,
    /// The server the entry is configured with (`AE_TMUX_SERVER`).
    socket: PathBuf,
    /// The server the test's own panes live on. The same one unless a case
    /// must keep the entry's server emptier than the fixtures' (`split`).
    panes: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        Self::build(tag, false)
    }

    /// A rig whose entry server stays EMPTY: the panes live on a second,
    /// private server, so a fresh recording state root is what the entry sees.
    fn split(tag: &str) -> Self {
        Self::build(tag, true)
    }

    fn build(tag: &str, separate_panes: bool) -> Self {
        let mut scratch = OwnedScratch::root("entryapp", tag);
        let home = scratch.join("state");
        let project = scratch.join("project");
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
        std::fs::create_dir_all(&home).expect("isolated state root");
        std::fs::create_dir_all(&project).expect("isolated project");
        std::fs::write(home.join("config"), CONFIG).expect("harmless configuration");
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

    /// The real entry from the test process: no terminal on either end, no
    /// inherited tmux (a cleared environment, so a run started inside a tmux
    /// pane still sees none), this rig's state and server.
    fn run(&self, args: &[&str], tag: &str) -> Ran {
        let inherited_path =
            std::env::var("PATH").unwrap_or_else(|why| panic!("setup: PATH: {why}"));
        let mut call = Invocation::new(env!("CARGO_BIN_EXE_ae"))
            .env_cleared()
            .env("PATH", inherited_path)
            .env("HOME", &self.scratch)
            .env("AE_HOME", &self.home)
            .env("CONFIG_FILE", self.home.join("config"))
            .env("TMUX_TMPDIR", &self.scratch)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .env("AE_NO_AUTOSTART", "1")
            .env("SHELL", "/bin/sh")
            .env("AE_TEST_GO_USAGE_URL", "http://127.0.0.1:9/");
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

    /// One recorded, running session on this rig's server.
    fn launch(&self, name: &str) {
        let out = self.run(&["--local", name, "--no-attach"], &format!("launch-{name}"));
        assert_eq!(
            out.code,
            Some(0),
            "launch {name}: {}",
            String::from_utf8_lossy(&out.stderr)
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

    /// The session names the state root records; dotfiles are lock and stamp
    /// dirt, not sessions.
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

    /// A window in `session` running `command`, with this rig's ae
    /// environment. Its pane id. tmux owns `TMUX_PANE` (a `-e` is overwritten
    /// by the pane's own id), so a case that needs a bogus one sets it inside
    /// the command, where the shell keeps it.
    fn pane(&self, session: &str, command: &str) -> String {
        let env = [
            format!("AE_HOME={}", self.home.display()),
            format!("CONFIG_FILE={}", self.home.join("config").display()),
            format!("HOME={}", self.scratch.display()),
            "AE_TMUX_SERVER_KIND=socket".to_owned(),
            format!("AE_TMUX_SERVER={}", self.socket.display()),
            "AE_NO_AUTOSTART=1".to_owned(),
            format!("TMUX_TMPDIR={}", self.scratch.display()),
        ];
        let target = format!("={session}");
        let mut args: Vec<String> = vec![
            "new-window".to_owned(),
            "-d".to_owned(),
            "-P".to_owned(),
            "-F".to_owned(),
            "#{pane_id}".to_owned(),
            "-t".to_owned(),
            target,
        ];
        for pair in &env {
            args.push("-e".to_owned());
            args.push(pair.clone());
        }
        args.push(command.to_owned());
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        let (ok, pane) = self.pane_tmux(&words);
        assert!(ok, "a pane in {session}: {pane}");
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
/// (or its environment) before the entry reads it. The final `exec cat` holds
/// the pane open after the entry ends, so the screen and the exit code stay
/// readable while nothing outlives the pane.
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

/// Every pane is 160 columns: its default 44-cell sidebar fits the word rung.
/// Positive and negative app checks share this predicate, so both discriminate
/// the current frame rather than relying on a removed summary glyph.
fn app_frame(screen: &str) -> bool {
    screen.lines().next().is_some_and(|row| {
        row.starts_with("  Agents ")
            && row.contains(" · Working ")
            && row.contains(" · Needs you ")
            && row.contains('│')
    })
}

/// The named visible sidebar row proves the initial fleet read has completed.
fn lands(screen: &str, name: &str) -> bool {
    app_frame(screen)
        && screen.lines().skip(2).any(|row| {
            row.chars().skip(4).collect::<String>().starts_with(name) && row.contains('│')
        })
}

/// The `ae list` table row of a running session, in the same screen.
fn listed(screen: &str, name: &str) -> bool {
    screen
        .lines()
        .any(|row| row.trim_start().starts_with(name) && row.contains("running"))
}

/// One recorded session, running or stopped, lands outside tmux in the app —
/// and the landing creates and resumes nothing.
#[test]
fn a_recorded_session_lands_outside_tmux_in_the_app_running_or_stopped() {
    let rig = Rig::new("out");
    rig.launch("one");

    let gate = rig.scratch.join("go-running");
    let rc = rig.scratch.join("rc-running");
    let pane = rig.pane("host", &gated(&command(&[], &rc, true), &gate));
    std::fs::write(&gate, "").expect("release the pane");
    let screen = rig.wait(&pane, "a recorded running session lands", |screen| {
        lands(screen, "one")
    });
    assert!(
        !screen.contains("Stopped ("),
        "the row is running, not stopped:\n{screen}"
    );
    assert_eq!(rig.quit(&pane, &rc), "0", "the app quits cleanly");
    assert_eq!(rig.recorded(), ["one"], "the landing created no session");

    rig.stop("one");
    let gate = rig.scratch.join("go-stopped");
    let rc = rig.scratch.join("rc-stopped");
    let pane = rig.pane("host", &gated(&command(&[], &rc, true), &gate));
    std::fs::write(&gate, "").expect("release the pane");
    let screen = rig.wait(&pane, "a recorded stopped session lands too", |screen| {
        app_frame(screen) && screen.contains("▸ Stopped (1)")
    });
    assert!(
        !lands(&screen, "one"),
        "stopped rows start collapsed: {screen}"
    );
    assert_eq!(rig.quit(&pane, &rc), "0", "the app quits cleanly");
    assert_eq!(rig.recorded(), ["one"], "the landing resumed nothing");
    assert!(
        !rig.has_session("one"),
        "the landing resumed nothing: no tmux session came back"
    );
}

/// Zero recorded sessions is today's refusal, not a first-use app: the entry
/// creates nothing, resumes nothing and draws no frame.
#[test]
fn no_recorded_session_keeps_todays_refusal() {
    let rig = Rig::split("empty");
    assert!(rig.recorded().is_empty(), "a fresh state root records none");
    assert!(
        !rig.tmux(&["has-session", "-t", "=host"]).0,
        "the entry's server holds no session at all"
    );

    let gate = rig.scratch.join("go");
    let rc = rig.scratch.join("rc");
    let pane = rig.pane("host", &gated(&command(&[], &rc, true), &gate));
    std::fs::write(&gate, "").expect("release the pane");
    let screen = rig.wait(&pane, "an empty state root refuses", |screen| {
        screen.contains("no running ae session")
    });
    assert!(
        screen.contains("ae: no running ae session. Start one with: ae <name>"),
        "today's refusal, verbatim:\n{screen}"
    );
    assert!(!app_frame(&screen), "no recording draws no app:\n{screen}");
    assert_eq!(wait_for_rc(&rc), "1", "the refusal exits 1");
    assert!(rig.recorded().is_empty(), "the refusal created nothing");
}

/// An unstamped pane in a recorded ae session is not a seat pane: it lands.
#[test]
fn an_unstamped_pane_in_an_ae_session_lands_in_the_app() {
    let rig = Rig::new("in");
    rig.launch("one");
    let gate = rig.scratch.join("go");
    let rc = rig.scratch.join("rc");
    let pane = rig.pane("one", &gated(&command(&[], &rc, false), &gate));
    std::fs::write(&gate, "").expect("release the pane");
    let screen = rig.wait(&pane, "an unstamped ae pane lands", |screen| {
        lands(screen, "one")
    });
    assert!(
        !screen.contains("Stopped ("),
        "the row is running, not stopped:\n{screen}"
    );
    assert_eq!(rig.quit(&pane, &rc), "0", "the app quits cleanly");
}

/// The seat-pane test is `@ae_agent` alone: a stamped pane keeps `ae list`, an
/// unreadable pane fails closed to it, and a chat pane (`@ae_console`) lands.
#[test]
fn a_seat_pane_and_an_unreadable_pane_keep_the_list_while_a_chat_pane_lands() {
    let rig = Rig::new("seat");
    rig.launch("one");

    let gate = rig.scratch.join("go-seat");
    let rc = rig.scratch.join("rc-seat");
    let seat = rig.pane("one", &gated(&command(&[], &rc, false), &gate));
    assert!(
        rig.tmux(&["set-option", "-p", "-t", &seat, "@ae_agent", "lead"])
            .0,
        "a seat pane carries @ae_agent"
    );
    std::fs::write(&gate, "").expect("release the pane");
    let screen = rig.wait(&seat, "a seat pane keeps the list", |screen| {
        listed(screen, "one")
    });
    assert!(!app_frame(&screen), "a seat pane draws no app");
    assert_eq!(wait_for_rc(&rc), "0", "the list exits 0");

    let gate = rig.scratch.join("go-unreadable");
    let rc = rig.scratch.join("rc-unreadable");
    let lost = rig.pane(
        "one",
        &gated(
            &format!("TMUX_PANE=%99999 {}", command(&[], &rc, false)),
            &gate,
        ),
    );
    std::fs::write(&gate, "").expect("release the pane");
    let screen = rig.wait(&lost, "an unreadable pane keeps the list", |screen| {
        listed(screen, "one")
    });
    assert!(!app_frame(&screen), "an unreadable pane draws no app");
    assert_eq!(wait_for_rc(&rc), "0", "the list exits 0");

    let gate = rig.scratch.join("go-chat");
    let rc = rig.scratch.join("rc-chat");
    let chat = rig.pane("one", &gated(&command(&[], &rc, false), &gate));
    assert!(
        rig.tmux(&[
            "set-option",
            "-p",
            "-t",
            &chat,
            "@ae_console",
            "0199c0de-cccc-4890-abcd-ef0123456789"
        ])
        .0,
        "a chat pane carries @ae_console"
    );
    std::fs::write(&gate, "").expect("release the pane");
    rig.wait(&chat, "a chat pane lands", |screen| lands(screen, "one"));
    assert_eq!(rig.quit(&chat, &rc), "0", "the app quits cleanly");

    // A pane in a tmux session ae does not record: the app could not have a
    // home there (`ae app` refuses an unknown name), so it fails closed.
    let gate = rig.scratch.join("go-unrecorded");
    let rc = rig.scratch.join("rc-unrecorded");
    let stray = rig.pane("host", &gated(&command(&[], &rc, false), &gate));
    std::fs::write(&gate, "").expect("release the pane");
    let screen = rig.wait(
        &stray,
        "an unrecorded caller session keeps the list",
        |screen| listed(screen, "one"),
    );
    assert!(!app_frame(&screen), "an unknown home draws no app");
    assert_eq!(wait_for_rc(&rc), "0", "the list exits 0");
}

/// The door is chosen by the terminal, never by the recording: without a
/// terminal the entry still takes today's attach path — and with one it lands.
#[test]
fn the_door_is_chosen_by_the_terminal_not_by_the_recording() {
    let rig = Rig::new("tty");
    rig.launch("one");
    let out = rig.run(&[], "outside");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_ne!(out.code, Some(2), "{stderr}");
    assert_ne!(out.code, Some(0), "no terminal, no client: {stderr}");
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        !stderr.contains(ae::app::NO_TERMINAL),
        "without a terminal the entry must not reach the app: {stderr}"
    );

    let gate = rig.scratch.join("go");
    let rc = rig.scratch.join("rc");
    let pane = rig.pane("host", &gated(&command(&[], &rc, true), &gate));
    std::fs::write(&gate, "").expect("release the pane");
    rig.wait(&pane, "with a terminal the same rig lands", |screen| {
        lands(screen, "one")
    });
    assert_eq!(rig.quit(&pane, &rc), "0", "the app quits cleanly");
}

/// `--no-attach` and `--tmux` keep today's meanings, and the two flags cannot
/// be combined.
#[test]
fn the_bare_forms_keep_their_meaning() {
    let rig = Rig::new("forms");
    rig.launch("one");

    // `--no-attach` outside: today's hint, exit 0, no app.
    let gate = rig.scratch.join("go-hint");
    let rc = rig.scratch.join("rc-hint");
    let pane = rig.pane("host", &gated(&command(&["--no-attach"], &rc, true), &gate));
    std::fs::write(&gate, "").expect("release the pane");
    let screen = rig.wait(&pane, "the attach hint is printed", |screen| {
        screen.contains("ae: attach with: tmux -S ")
    });
    assert!(
        screen.contains(&format!(
            "ae: attach with: tmux -S {} attach",
            rig.socket.display()
        )),
        "the hint names the launch server:\n{screen}"
    );
    assert!(!app_frame(&screen), "`--no-attach` draws no app");
    assert_eq!(wait_for_rc(&rc), "0", "`--no-attach` exits 0");

    // `--tmux` inside its own server: today's list.
    let gate = rig.scratch.join("go-tmux-in");
    let rc = rig.scratch.join("rc-tmux-in");
    let inside = rig.pane("one", &gated(&command(&["--tmux"], &rc, false), &gate));
    std::fs::write(&gate, "").expect("release the pane");
    let screen = rig.wait(&inside, "`--tmux` lists inside tmux", |screen| {
        listed(screen, "one")
    });
    assert!(!app_frame(&screen), "`--tmux` draws no app: {screen}");
    assert_eq!(wait_for_rc(&rc), "0", "`--tmux` exits 0");

    // `--tmux` outside, no terminal: today's attach, not the app.
    let out = rig.run(&["--tmux"], "tmux-outside");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_ne!(out.code, Some(0), "{stderr}");
    assert_ne!(out.code, Some(2), "{stderr}");
    assert!(
        !stderr.contains(ae::app::NO_TERMINAL),
        "`--tmux` without a terminal must not reach the app: {stderr}"
    );

    // Both orders of the conflicting pair are usage errors, and `--tmux` is a
    // known flag (a bug report about a flag that does not exist is not the
    // contract here).
    for (i, args) in [["--tmux", "--no-attach"], ["--no-attach", "--tmux"]]
        .iter()
        .enumerate()
    {
        let out = rig.run(args, &format!("conflict-{i}"));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.code, Some(2), "{args:?}: {stderr}");
        assert!(out.stdout.is_empty(), "{args:?}: {stderr}");
        assert!(!stderr.contains("unknown flag"), "{args:?}: {stderr}");
        assert!(
            stderr.contains("--tmux") && stderr.contains("--no-attach"),
            "{args:?} names both flags: {stderr}"
        );
    }
}

/// The restore gate runs BEFORE the landing: a recorded session its server no
/// longer holds is restored, and the app then lists it as running.
#[test]
fn a_crashed_cohort_is_restored_before_the_app_lands() {
    let rig = Rig::new("restore");
    rig.launch("one");
    rig.stop("one");
    assert!(!rig.has_session("one"), "the recorded server lost it");
    ae::watchdog_glue::touch_beat(&rig.dir("one")).expect("a fresh beat");

    let gate = rig.scratch.join("go");
    let rc = rig.scratch.join("rc");
    let pane = rig.pane("host", &gated(&command(&[], &rc, true), &gate));
    std::fs::write(&gate, "").expect("release the pane");
    let screen = rig.wait(&pane, "the restore lands in the app", |screen| {
        lands(screen, "one")
    });
    assert!(
        !screen.contains("Stopped ("),
        "the restored session is running in the frame:\n{screen}"
    );
    assert_eq!(rig.quit(&pane, &rc), "0", "the app quits cleanly");
    assert!(
        rig.has_session("one"),
        "the landing happened after a real restore"
    );
}

/// The routing audit: every word `ae` answers itself is refused as a session
/// name. `upgrade` is answered above the router (`lib.rs::run`) and was the one
/// word missing from the set.
#[test]
fn a_word_the_entry_answers_is_a_reserved_session_name() {
    for word in ["compact", "init", "quota", "usage", "upgrade"] {
        assert!(
            ae::entry::ROUTED_VERBS.contains(&word),
            "{word} is answered by `ae` itself, so no session may take it"
        );
        assert!(
            ae::entry::name_is_routed_verb(word),
            "{word} must be refused by the one predicate"
        );
    }
    let rig = Rig::new("word");
    let out = rig.run(&["rename", "hold", "upgrade"], "rename");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.code, Some(2), "{stderr}");
    assert!(out.stdout.is_empty(), "{stderr}");
    assert!(
        stderr.contains("'upgrade' is an ae command"),
        "the routed refusal, not a bare name check: {stderr}"
    );
    assert!(!rig.dir("hold").exists(), "a refused name built state");
    assert!(!rig.dir("upgrade").exists(), "a refused name built state");
}
