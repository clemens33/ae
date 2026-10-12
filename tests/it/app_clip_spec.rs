//! appclip acceptance. Oracle: a private OUTER tmux server (`set-clipboard on`)
//! plays the human's terminal; an OSC 52 written to its pane lands as a paste
//! buffer there, which is what a Mac terminal does to the clipboard. The
//! expected text is the literal fixture, never read from the unit under test.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const TID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);
const TEXT: &str = "CLIPPROBE alpha";
const NEW: [&str; 8] = [
    "-f",
    "/dev/null",
    "new-session",
    "-d",
    "-x",
    "120",
    "-y",
    "40",
];

struct Rig {
    scratch: super::cli::OwnedScratch,
    outer: PathBuf,
    host: PathBuf,
    /// The app's pane on `host`, and the pane of `outer` that shows it.
    app: String,
    term: String,
}

impl Rig {
    /// `nested`: the app runs in a pane of a second server that `outer` shows
    /// through an attached client; else it runs in `outer`'s own pane, a plain
    /// terminal tab with no `$TMUX`.
    fn new(tag: &str, nested: bool) -> Self {
        let mut scratch = super::cli::OwnedScratch::root("appclip", tag);
        let (outer, inner) = (scratch.join("outer"), scratch.join("inner"));
        scratch.add_tmux_server(outer.clone());
        scratch.add_tmux_server(inner.clone());
        let host = if nested { inner.clone() } else { outer.clone() };
        let root = scratch.path().to_owned();
        let said = super::board::user("2026-10-12T10:00:00Z", "CLIPPROBE alpha bravo charlie");
        super::board::plant_transcript(&root.join("store"), "work", TID, &[said]);
        fs::create_dir_all(root.join("sessions/aeprobe")).expect("session dir");
        fs::write(root.join("sessions/aeprobe/meta"), format!(
            "schema=2\nsession=aeprobe\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=claude\nlaunch_id.main=tok-clip\nharness_session.main={TID}\nconfig_home.main={}\n",
            host.display(), root.join("store").display(),
        )).expect("meta");
        fs::write(root.join("config"), "[workspace]\nchat = app\n").expect("config");
        let mut rig = Self {
            scratch,
            outer,
            host,
            app: String::new(),
            term: String::new(),
        };
        rig.tmux(
            &rig.outer,
            &[&NEW[..], &["-s", "term", "sleep 300"]].concat(),
        );
        rig.tmux(&rig.outer, &["set-option", "-g", "set-clipboard", "on"]);
        let ae = env!("CARGO_BIN_EXE_ae");
        let app = format!("env -u CLAUDE_CONFIG_DIR -u CODEX_HOME {ae} app aeprobe");
        let env = [
            format!("HOME={}", root.display()),
            format!("AE_HOME={}", root.display()),
            format!("CONFIG_FILE={}", root.join("config").display()),
            "AE_TMUX_SERVER_KIND=socket".to_owned(),
            format!("AE_TMUX_SERVER={}", rig.host.display()),
        ];
        let mut args = vec!["new-window", "-d", "-P", "-F", "#{pane_id}"];
        for pair in &env {
            args.extend(["-e", pair.as_str()]);
        }
        let plain = format!("env -u TMUX -u TMUX_PANE TERM=xterm-ghostty {app}");
        if nested {
            rig.tmux(
                &inner,
                &[&NEW[..], &["-s", "aeprobe", "sleep 300"]].concat(),
            );
            rig.tmux(&inner, &["set-option", "-t", "aeprobe", "mouse", "on"]);
            rig.app = rig.tmux(
                &inner,
                &[&args[..], &["-t", "aeprobe", app.as_str()]].concat(),
            );
            rig.tmux(&inner, &["select-window", "-t", rig.app.trim()]);
            // tmux attach needs its TERM's terminfo, which the Linux gate image lacks for ghostty.
            let attach = format!("env TERM=xterm-256color tmux -S {} attach", inner.display());
            rig.term = rig.tmux(
                &rig.outer,
                &["new-window", "-d", "-P", "-F", "#{pane_id}", &attach],
            );
        } else {
            rig.term = rig.tmux(
                &rig.outer,
                &[&args[..], &["-t", "term", plain.as_str()]].concat(),
            );
        }
        (rig.app, rig.term) = (rig.app.trim().to_owned(), rig.term.trim().to_owned());
        rig.wait("app drawn", |screen| screen.contains("CLIPPROBE"));
        rig
    }

    fn tmux(&self, sock: &Path, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), sock.display().to_string()];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, self.scratch.path());
        assert!(ok, "private tmux {tail:?}: {out}");
        out
    }

    /// The terminal's screen once `met` holds; the wait fails with that screen.
    fn wait(&self, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + WAIT;
        loop {
            let screen = self.tmux(&self.outer, &["capture-pane", "-p", "-t", &self.term]);
            if met(&screen) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}; terminal:\n{screen}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Bytes typed at the terminal in one write.
    fn type_at(&self, bytes: &str) {
        let tail = ["send-keys", "-t", &self.term, "-l", "--", bytes];
        self.tmux(&self.outer, &tail);
        std::thread::sleep(Duration::from_millis(150));
    }

    /// Mouse bytes typed at the terminal, at a 0-based cell.
    fn mouse(&self, kind: u8, release: bool, (x, y): (usize, usize)) {
        let end = if release { 'm' } else { 'M' };
        self.type_at(&format!("\x1b[<{kind};{};{}{end}", x + 1, y + 1));
    }

    /// Drag over the fixture text, first cell to last.
    fn drag(&self) {
        self.drag_around(|| ());
    }

    /// The same drag, with `between` done while the button is down.
    fn drag_around(&self, between: impl FnOnce()) {
        let screen = self.wait("fixture text drawn", |s| s.contains(TEXT));
        let (row, line) = screen
            .lines()
            .enumerate()
            .find(|(_, l)| l.contains(TEXT))
            .expect("a row");
        let x = line[..line.find(TEXT).expect("text")].chars().count();
        let last = x + TEXT.chars().count() - 1;
        self.mouse(0, false, (x, row));
        self.mouse(32, false, (x + 5, row));
        between();
        self.mouse(32, false, (last, row));
        self.mouse(0, true, (last, row));
    }

    /// The exact bytes of every paste buffer of `sock`, newest first.
    fn buffers(&self, sock: &Path) -> Vec<String> {
        let names = self.tmux(sock, &["list-buffers", "-F", "#{buffer_name}"]);
        let show = |name| self.tmux(sock, &["show-buffer", "-b", name]);
        names.lines().map(show).collect()
    }
}

/// A drag in a plain terminal tab reaches the terminal's clipboard, and the
/// hint row does not claim it arrived.
#[test]
fn a_drag_in_a_plain_terminal_reaches_the_terminal_clipboard() {
    let rig = Rig::new("plain", false);
    rig.drag();
    rig.wait("no OSC 52 reached the terminal", |_| {
        rig.buffers(&rig.outer) == [TEXT]
    });
    let said = "sent 1 line to the terminal clipboard - unconfirmed";
    rig.wait("the hint row says what was sent", |s| s.contains(said));
}

/// A turn that lands while the button is down does not cost the copy: the
/// release copies what was under the drag at the press, and the turn shows
/// after. The wait outlasts the 5 s poll, so the read happens mid-drag.
#[test]
fn a_turn_landing_mid_drag_does_not_cost_the_copy() {
    let rig = Rig::new("lane", false);
    let late = super::board::user("2026-10-12T10:00:30Z", "CLIPLATE a turn lands");
    let path = rig.scratch.join(format!("store/projects/work/{TID}.jsonl"));
    rig.drag_around(|| {
        let old = fs::read_to_string(&path).expect("transcript");
        fs::write(&path, format!("{old}{late}\n")).expect("a turn lands");
        std::thread::sleep(Duration::from_secs(7));
    });
    let hint = |s: &str| s.contains("selection cleared") || s.contains("terminal clipboard");
    rig.wait("the release answers on the hint row", hint);
    rig.wait("the copy never reached the terminal", |_| {
        rig.buffers(&rig.outer) == [TEXT]
    });
    rig.wait("the turn shows after the copy", |s| s.contains("CLIPLATE"));
}

/// A copy accepted in the same batch as the quit still reaches the terminal.
#[test]
fn a_copy_typed_just_before_quitting_is_not_lost() {
    let rig = Rig::new("quit", false);
    rig.type_at("[y\x03");
    let until = Instant::now() + WAIT;
    while rig.buffers(&rig.outer) != ["CLIPPROBE alpha bravo charlie"] {
        assert!(Instant::now() < until, "the copy was lost with the quit");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Inside tmux nothing changes: tmux alone copies, once, and the app writes no
/// OSC 52 of its own into its pane.
#[test]
fn a_drag_inside_tmux_copies_once_and_the_app_writes_no_osc_52() {
    let rig = Rig::new("nested", true);
    let log = rig.scratch.join("pane-output");
    let pipe = format!("cat >> {}", log.display());
    rig.tmux(&rig.host, &["pipe-pane", "-O", "-t", &rig.app, &pipe]);
    rig.drag();
    rig.wait("tmux did not copy", |_| rig.buffers(&rig.outer) == [TEXT]);
    rig.wait("the tmux hint", |s| s.contains("to the tmux buffer;"));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(rig.buffers(&rig.outer), [TEXT], "the terminal got one copy");
    assert_eq!(rig.buffers(&rig.host), [TEXT], "tmux holds one copy");
    let output = fs::read(&log).expect("pane output logged");
    assert!(
        !output.windows(5).any(|w| w == b"\x1b]52;"),
        "the app wrote its own OSC 52"
    );
}
