//! Pins for the run loop's own decisions, which a unit test cannot reach: when
//! it paints, what one pass paints, and what it prints on the way out. The
//! oracles are docs/app.md (a refused paste says why "for a few seconds"; a
//! clean quit leaves one plain restart line), the terminal bytes the app
//! wrote and the captured cells. Private sockets and existing process doors
//! only; the CHECKOUT-only read gate holds the loop at a real read site.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(15);
const FRAME: Duration = Duration::from_secs(3);
const HOME_UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
/// Longer than the few seconds a refused paste is named for.
const HINT_OVER: Duration = Duration::from_secs(6);
const IDLE: Duration = Duration::from_millis(1500);

/// Releases on assertion unwind and, independently, at a 45 s deadline.
struct Hold {
    path: PathBuf,
    before: usize,
    release: Option<mpsc::Sender<()>>,
    watcher: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Hold {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
    }
}

struct Rig {
    tool: super::deliver::Rig,
    root: PathBuf,
    socket: PathBuf,
    home: String,
    gates: PathBuf,
}

impl Rig {
    fn new(tag: &str, siblings: &[&str]) -> Self {
        let tool = super::deliver::Rig::new(tag, "codex", 0);
        let root = tool
            .dir
            .parent()
            .expect("sessions")
            .parent()
            .expect("root")
            .to_owned();
        let home = tool
            .dir
            .file_name()
            .expect("session")
            .to_string_lossy()
            .into_owned();
        let socket = root.join("sock");
        let gates = root.join("read-gates");
        fs::create_dir(&gates).expect("private gate directory");
        let rig = Self {
            tool,
            root,
            socket,
            home,
            gates,
        };
        rig.seed(&rig.home, HOME_UUID, 0);
        for (at, name) in siblings.iter().enumerate() {
            let uuid = format!("0199c0de-bbbb-4890-abcd-ef012345{:04x}", at + 1);
            rig.seed(name, &uuid, at + 1);
        }
        fs::write(rig.root.join("config"), "[workspace]\nchat = app\n").expect("app config");
        for (option, value) in [
            ("@ae_look", "off"),
            ("@ae_session_uuid", HOME_UUID),
            ("@ae_motion", "off"),
            ("@ae_main_pane", rig.tool.pane.as_str()),
            ("default-size", "160x45"),
        ] {
            rig.tmux(&["set-option", "-t", &rig.home, option, value]);
        }
        rig.tmux(&[
            "set-option",
            "-p",
            "-t",
            &rig.tool.pane,
            "@ae_agent",
            "lead",
        ]);
        rig
    }

    /// A session with a journal and a transcript. The home one is the live
    /// lead pair of the private server; any other has no tmux session, so
    /// the app shows it stopped and read-only.
    fn seed(&self, name: &str, uuid: &str, at: usize) {
        let dir = self.root.join("sessions").join(name);
        fs::create_dir_all(&dir).expect("fixture session");
        let store = self.root.join(format!("claude-{at}"));
        let tid = format!("0199c0de-aaaa-4890-abcd-ef012345{at:04x}");
        let turns = (0..40)
            .map(|turn| {
                let marker = if turn == 39 { "newest" } else { "older" };
                super::board::user(
                    &format!("2026-10-04T10:00:{turn:02}Z"),
                    &format!(
                        "{name}-{marker}-{turn:02} {}",
                        "fixture words for scroll ".repeat(20)
                    ),
                )
            })
            .collect::<Vec<_>>();
        super::board::plant_transcript(&store, "work", &tid, &turns);
        fs::write(dir.join("meta"), format!(
            "session={name}\nmode=local\nsession_id={uuid}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\ngoal=goal-{name}\nseat.main=lead\nagent_bin.main=claude\nlaunch_id.main=tok-{at}\nharness_session.main={tid}\nconfig_home.main={}\nseat.worker.0=colead\nagent_bin.worker.0=claude\nlaunch_id.worker.0=tok-colead-{at}\nconfig_home.worker.0={}\n",
            self.socket.display(), store.display(), store.display(),
        )).expect("fixture meta");
        fs::write(dir.join("events.jsonl"), "").expect("regular fixture journal");
    }

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &self.root);
        assert!(ok, "private tmux {tail:?}: {out}");
        out
    }

    /// A direct app window carrying the gate env explicitly.
    fn open(&self) -> String {
        let command = format!(
            "env -u CLAUDE_CONFIG_DIR -u CODEX_HOME {} app {}; sleep 60",
            quote(env!("CARGO_BIN_EXE_ae")),
            quote(&self.home),
        );
        let env = [
            format!("AE_HOME={}", self.root.display()),
            format!("CONFIG_FILE={}", self.root.join("config").display()),
            format!("HOME={}", self.root.display()),
            "AE_TMUX_SERVER_KIND=socket".to_owned(),
            format!("AE_TMUX_SERVER={}", self.socket.display()),
            format!("AE_TEST_APP_READ_GATE={}", self.gates.display()),
        ];
        let mut args = vec![
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &self.home,
        ];
        for pair in &env {
            args.extend(["-e", pair.as_str()]);
        }
        args.push(&command);
        let pane = self.tmux(&args).trim().to_owned();
        self.tmux(&["resize-window", "-t", &pane, "-x", "160", "-y", "45"]);
        pane
    }

    fn screen(&self, pane: &str) -> String {
        self.tmux(&["capture-pane", "-p", "-t", pane])
    }

    fn wait(&self, pane: &str, within: Duration, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + within;
        loop {
            let screen = self.screen(pane);
            if met(&screen) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}; terminal screen:\n{screen}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn ready(&self, pane: &str, count: usize) -> String {
        self.wait(pane, WAIT, "fixture fleet and home lane drawn", |screen| {
            screen.contains(&format!("Sessions {count}"))
                && header_is(screen, &self.home)
                && screen.contains(&format!("{}-newest-39", self.home))
        })
    }

    fn keys(&self, pane: &str, keys: &str) {
        self.tmux(&["send-keys", "-t", pane, keys]);
    }

    fn literal(&self, pane: &str, keys: &str) {
        self.tmux(&["send-keys", "-t", pane, "-l", "--", keys]);
    }

    fn trace_count(&self, key: &str) -> usize {
        let line = format!("held {key}");
        fs::read_to_string(self.gates.join("trace"))
            .unwrap_or_default()
            .lines()
            .filter(|entry| *entry == line)
            .count()
    }

    fn hold(&self, key: &str) -> Hold {
        let before = self.trace_count(key);
        let path = self.gates.join(key);
        fs::write(&path, "hold").expect("arm a read-site gate");
        let (release, released) = mpsc::channel();
        let deadline_path = path.clone();
        let watcher = std::thread::spawn(move || {
            let _ = released.recv_timeout(Duration::from_secs(45));
            let _ = fs::remove_file(deadline_path);
        });
        Hold {
            path,
            before,
            release: Some(release),
            watcher: Some(watcher),
        }
    }

    fn held(&self, hold: &Hold, key: &str, why: &str) {
        let until = Instant::now() + WAIT;
        while self.trace_count(key) <= hold.before {
            assert!(
                Instant::now() < until,
                "{why}: real read site never attested held {key}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(hold.path.exists(), "gate remains held for the oracle");
    }
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// A plain sidebar capture always lists every name. Only the right-hand
/// header on row 1 attests selection (160 columns => sidebar rule at 44).
fn header_is(screen: &str, name: &str) -> bool {
    screen.lines().nth(1).is_some_and(|line| {
        line.chars()
            .skip(47)
            .collect::<String>()
            .trim_start()
            .starts_with(name)
    })
}

/// A key's repaint and a hint's end land in one pass of the loop and both
/// paint, with no further event to carry either. The pass is held at the
/// decoder read until the refused-paste hint is older than its few seconds,
/// so that one pass both redraws for the key and ends the hint.
#[test]
fn a_key_and_a_hint_that_ends_in_one_pass_both_paint_without_another_event() {
    let rig = Rig::new("lploop", &["lp-next"]);
    let pane = rig.open();
    rig.ready(&pane, 2);
    rig.keys(&pane, "j");
    rig.wait(&pane, WAIT, "the stopped neighbour is selected", |screen| {
        header_is(screen, "lp-next") && screen.contains("lp-next-newest-39")
    });
    // Park the loader, so that no refresh answer repaints in this test's
    // place: only the key and the hint can.
    let world = rig.hold("@world");
    rig.held(&world, "@world", "periodic world refresh");
    rig.literal(&pane, "\x1b[200~PASTE-REFUSED\x1b[201~");
    rig.wait(&pane, FRAME, "the refused paste names itself", |screen| {
        screen.contains("paste not taken")
    });
    let named = Instant::now();
    let keys = rig.hold("@app-keys");
    rig.keys(&pane, "k");
    rig.held(&keys, "@app-keys", "the key's read, before it is decoded");
    while named.elapsed() < HINT_OVER {
        std::thread::sleep(Duration::from_millis(50));
    }
    let held = rig.screen(&pane);
    assert!(
        held.contains("paste not taken") && header_is(&held, "lp-next"),
        "nothing paints while the pass is held:\n{held}"
    );
    drop(keys);
    rig.wait(
        &pane,
        FRAME,
        "the key and the end of the hint paint in the pass that carried both",
        |screen| header_is(screen, &rig.home) && !screen.contains("paste not taken"),
    );
    assert!(world.path.exists(), "no refresh answer helped");
}

/// A frame nothing changed is not painted again: between two events the app
/// writes nothing to its terminal, however many ticks pass. A painted frame
/// always writes (at least the cursor escape), so the bytes the pane shows
/// are the frames the app drew. The loader is parked, so no answer can be
/// the cause of a paint; the capture is proved live by a repaint reaching it
/// before the window, and the app proved listening by one reaching it after.
#[test]
fn an_idle_app_paints_no_frame_until_something_changes() {
    let rig = Rig::new("lpidle", &[]);
    let pane = rig.open();
    rig.ready(&pane, 1);
    let world = rig.hold("@world");
    rig.held(&world, "@world", "periodic world refresh");
    let written = rig.root.join("pane-output");
    fs::write(&written, "").expect("capture file");
    rig.tmux(&[
        "pipe-pane",
        "-O",
        "-t",
        &pane,
        &format!("cat >> {}", quote(&written.to_string_lossy())),
    ]);
    let size = || fs::metadata(&written).expect("capture").len();
    rig.keys(&pane, "Tab");
    rig.wait(&pane, FRAME, "the Agents tab paints", |screen| {
        screen.contains("No seat facts")
    });
    // The repaint reached the capture; wait until it has all arrived.
    let until = Instant::now() + FRAME;
    let mut seen = 0;
    while size() == 0 || size() != seen {
        seen = size();
        assert!(Instant::now() < until, "the capture never settled");
        std::thread::sleep(Duration::from_millis(300));
    }
    let until = Instant::now() + IDLE;
    while Instant::now() < until {
        assert_eq!(
            size(),
            seen,
            "the idle app painted again: {:?}",
            fs::read(&written).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    rig.keys(&pane, "Tab");
    rig.wait(
        &pane,
        FRAME,
        "the app still listens and repaints",
        |screen| screen.contains(&format!("goal-{}", rig.home)),
    );
    assert!(size() > seen, "the second repaint reached the capture");
    assert!(world.path.exists(), "the loader stayed parked");
}

/// A clean quit leaves one plain restart line and no word about a background
/// reader: that line is for a reader that died, and this one did not.
#[test]
fn a_clean_quit_says_nothing_of_the_background_reader() {
    let rig = Rig::new("lpquit", &[]);
    let pane = rig.open();
    rig.ready(&pane, 1);
    rig.keys(&pane, "C-c");
    let shown = rig.wait(&pane, FRAME, "the restart line", |screen| {
        screen.contains("ae app closed")
    });
    assert_eq!(shown.matches("ae app closed").count(), 1);
    assert!(
        !shown.contains("background reader"),
        "a clean quit names no dead reader:\n{shown}"
    );
}
