//! Frozen acceptance: input repaints while a real read site is held.
//! The CHECKOUT-only S0 gate attests before blocking; the drawn frame, not
//! loader internals or disk speed, is the oracle. Regular fixture journals
//! and transcripts judge content. No real harness stores or ae state.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use crate::app_mode::writing;
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(15);
const FRAME: Duration = Duration::from_secs(3);
const QUIT: Duration = Duration::from_secs(1);
const REFRESH_SLACK: Duration = Duration::from_secs(8);
const HOME_UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";

/// Releases on assertion unwind and, independently, at a 45 s deadline.
/// The app's own S0 bound is 60 s; a failing test never leaves a held read.
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

    fn seed(&self, name: &str, uuid: &str, at: usize) {
        let dir = self.root.join("sessions").join(name);
        fs::create_dir_all(&dir).expect("fixture session");
        let store = self.root.join(format!("claude-{at}"));
        let tid = format!("0199c0de-aaaa-4890-abcd-ef012345{at:04x}");
        // A genuinely scrollable lane, distinct for each session. The large
        // fixture is content, never a proxy for a particular disk speed.
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

    /// A direct app window carries the gate env explicitly. Its shell writes
    /// terminal modes after the app exits, including on a failing test.
    fn open(&self) -> (String, PathBuf) {
        let modes = self.root.join("modes-after-app");
        let command = format!(
            "env -u CLAUDE_CONFIG_DIR -u CODEX_HOME {} app {}; stty -a > {}; sleep 60",
            quote(env!("CARGO_BIN_EXE_ae")),
            quote(&self.home),
            quote(&modes.to_string_lossy()),
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
        (pane, modes)
    }

    /// The existing `_console` route stamps ownership before launching. Its
    /// tmux window inherits the gate from the private session environment.
    fn open_owner(&self) -> String {
        for (key, value) in [
            ("AE_TEST_APP_READ_GATE", self.gates.display().to_string()),
            ("HOME", self.root.display().to_string()),
        ] {
            self.tmux(&["set-environment", "-t", &self.home, key, &value]);
        }
        self.tmux(&[
            "set-environment",
            "-r",
            "-t",
            &self.home,
            "CLAUDE_CONFIG_DIR",
        ]);
        self.tmux(&["set-environment", "-r", "-t", &self.home, "CODEX_HOME"]);
        let out = super::cli::ae()
            .env("AE_HOME", &self.root)
            .env("HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .env("TMUX", format!("{},1,0", self.socket.display()))
            .env("TMUX_PANE", &self.tool.pane)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .arg("_console")
            .output()
            .expect("owning app window opens");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        self.tmux(&[
            "list-panes",
            "-s",
            "-t",
            &self.home,
            "-F",
            "#{pane_id}|#{@ae_console}",
        ])
        .lines()
        .find_map(|row| {
            let (pane, stamp) = row.split_once('|')?;
            (stamp == HOME_UUID).then(|| pane.to_owned())
        })
        .expect("the stamped owning app pane")
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

    fn click_session(&self, pane: &str, name: &str) {
        let screen = self.screen(pane);
        let (row, col) = screen
            .lines()
            .enumerate()
            .find_map(|(row, line)| {
                let prefix = line.chars().take(44).collect::<String>();
                prefix.find(name).map(|col| (row, col))
            })
            .expect("the drawn sidebar row");
        self.literal(pane, &format!("\x1b[<0;{};{}M", col + 1, row + 1));
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
        assert!(hold.path.exists(), "gate remains held for the frame oracle");
    }

    fn choose(&self, pane: &str, name: &str) {
        self.click_session(pane, name);
        self.wait(pane, WAIT, "fixture selection lane loaded", |screen| {
            header_is(screen, name) && screen.contains(&format!("{name}-newest-39"))
        });
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

#[test]
fn the_first_frame_precedes_the_first_home_journal_read() {
    let rig = Rig::new("snfirst", &[]);
    let hold = rig.hold(&rig.home);
    let (pane, _) = rig.open();
    rig.held(&hold, &rig.home, "first home journal read");
    let screen = rig.wait(
        &pane,
        FRAME,
        "first home header and loading paint before the first read finishes",
        |screen| header_is(screen, &rig.home) && screen.contains("loading"),
    );
    assert!(
        !screen.contains(&format!("{}-newest-39", rig.home)),
        "the first unread lane cannot already be drawn"
    );
    assert!(hold.path.exists());
    drop(hold);
    rig.ready(&pane, 1);
}

#[test]
fn keys_repaint_selection_before_a_held_journal_read_finishes() {
    let rig = Rig::new("snkey", &["sn-next"]);
    let (pane, _) = rig.open();
    rig.ready(&pane, 2);
    let hold = rig.hold(&rig.home);
    rig.held(&hold, &rig.home, "selected periodic journal read");
    rig.keys(&pane, "j");
    rig.wait(
        &pane,
        FRAME,
        "j selection paints before the held read releases",
        |screen| header_is(screen, "sn-next"),
    );
    assert!(
        hold.path.exists(),
        "selection was judged during the held read"
    );
}

#[test]
fn clicks_repaint_selection_before_a_held_world_refresh_finishes() {
    let rig = Rig::new("snclick", &["sn-next"]);
    let (pane, _) = rig.open();
    rig.ready(&pane, 2);
    let hold = rig.hold("@world");
    rig.held(&hold, "@world", "periodic world refresh");
    rig.click_session(&pane, "sn-next");
    rig.wait(
        &pane,
        FRAME,
        "click selection paints while world refresh is held",
        |screen| header_is(screen, "sn-next"),
    );
    assert!(hold.path.exists());
}

#[test]
fn tab_repaints_during_a_held_refresh() {
    let rig = Rig::new("sntab", &[]);
    let (pane, _) = rig.open();
    rig.ready(&pane, 1);
    // Establish the existing tab/scroll oracles before holding any read.
    rig.keys(&pane, "Tab");
    rig.wait(&pane, WAIT, "fixture Agents panel", |screen| {
        screen.contains("No seat facts")
    });
    rig.keys(&pane, "Tab");
    rig.wait(&pane, WAIT, "fixture Overview returns", |screen| {
        screen.contains(&format!("goal-{}", rig.home))
    });
    let hold = rig.hold("@world");
    rig.held(&hold, "@world", "periodic world refresh");
    rig.keys(&pane, "Tab");
    rig.wait(
        &pane,
        FRAME,
        "Tab paints Agents while refresh is held",
        |screen| screen.contains("No seat facts"),
    );
    assert!(hold.path.exists());
}

#[test]
fn wheel_repaints_during_a_held_refresh() {
    let rig = Rig::new("snwheel", &[]);
    let (pane, _) = rig.open();
    rig.ready(&pane, 1);
    // Judge scrollability independently of the held-read oracle.
    rig.literal(&pane, "\x1b[<64;70;20M");
    rig.wait(&pane, WAIT, "fixture lane scrolls", |screen| {
        screen.contains("newer turns below")
    });
    rig.literal(&pane, "\x1b[<65;70;20M");
    rig.wait(&pane, WAIT, "fixture lane returns to newest", |screen| {
        !screen.contains("newer turns below") && screen.contains(&format!("{}-newest-39", rig.home))
    });
    let hold = rig.hold("@world");
    rig.held(&hold, "@world", "periodic world refresh");
    rig.literal(&pane, "\x1b[<64;70;20M");
    rig.wait(
        &pane,
        FRAME,
        "wheel scroll paints while refresh is held",
        |screen| screen.contains("newer turns below"),
    );
    rig.literal(&pane, "\x1b[<65;70;20M");
    rig.wait(
        &pane,
        FRAME,
        "wheel returns to newest while refresh is held",
        |screen| {
            !screen.contains("newer turns below")
                && screen.contains(&format!("{}-newest-39", rig.home))
        },
    );
    assert!(hold.path.exists());
}

#[test]
fn an_unvisited_preloaded_neighbour_draws_while_the_loader_is_busy() {
    let rig = Rig::new("snpre", &["sn-neighbour1", "sn-neighbour2"]);
    let hold = rig.hold("sn-neighbour2");
    let (pane, _) = rig.open();
    rig.ready(&pane, 3);
    rig.held(
        &hold,
        "sn-neighbour2",
        "loader preloads the second sidebar neighbour without input",
    );
    let first = rig.hold("sn-neighbour1");
    rig.click_session(&pane, "sn-neighbour1");
    rig.wait(
        &pane,
        FRAME,
        "first neighbour was preloaded and paints before the second read finishes",
        |screen| header_is(screen, "sn-neighbour1") && screen.contains("sn-neighbour1-newest-39"),
    );
    assert!(hold.path.exists());
    assert!(
        first.path.exists(),
        "an on-demand read cannot supply the cached lane"
    );
}

#[test]
fn a_read_lane_survives_more_than_three_session_visits() {
    let rig = Rig::new(
        "sncache",
        &["sn-cache1", "sn-cache2", "sn-cache3", "sn-cache4"],
    );
    let (pane, _) = rig.open();
    rig.ready(&pane, 5);
    for name in ["sn-cache1", "sn-cache2", "sn-cache3", "sn-cache4"] {
        rig.choose(&pane, name);
    }
    rig.choose(&pane, &rig.home);
    let hold = rig.hold("sn-cache1");
    rig.click_session(&pane, "sn-cache1");
    rig.wait(
        &pane,
        FRAME,
        "already read lane appears before its held reread",
        |screen| {
            header_is(screen, "sn-cache1")
                && screen.contains("sn-cache1-newest-39")
                && !screen.contains("loading")
        },
    );
    rig.held(
        &hold,
        "sn-cache1",
        "selection schedules a background reread",
    );
    assert!(hold.path.exists());
}

#[test]
fn cold_loading_and_late_answers_never_show_another_sessions_lane() {
    let rig = Rig::new("sncold", &["sn-cold1", "sn-cold2"]);
    let first = rig.hold("sn-cold1");
    let second = rig.hold("sn-cold2");
    let (pane, _) = rig.open();
    rig.ready(&pane, 3);
    rig.click_session(&pane, "sn-cold1");
    rig.held(&first, "sn-cold1", "cold first read");
    let screen = rig.wait(
        &pane,
        FRAME,
        "cold selection paints header and loading before read finishes",
        |screen| header_is(screen, "sn-cold1") && screen.contains("loading"),
    );
    assert!(
        !screen.contains(&format!("{}-newest-39", rig.home)),
        "home lane hidden under cold header"
    );
    assert!(
        !screen.contains("sn-cold1-newest-39"),
        "no completed cold lane while held"
    );
    rig.click_session(&pane, "sn-cold2");
    rig.wait(
        &pane,
        FRAME,
        "newer cold selection paints while older read is held",
        |screen| header_is(screen, "sn-cold2") && screen.contains("loading"),
    );
    drop(first);
    rig.held(
        &second,
        "sn-cold2",
        "newer selected read after older answer",
    );
    // The older answer has been sent, but the UI may not have consumed it at
    // the second read's attestation. Check every captured frame for 1.5s so
    // a stale installation cannot pass by winning one capture race.
    let until = Instant::now() + Duration::from_millis(1500);
    loop {
        let screen = rig.screen(&pane);
        assert!(
            header_is(&screen, "sn-cold2") && screen.contains("loading"),
            "late first answer leaves newer selection loading: {screen}"
        );
        assert!(
            !screen.contains("sn-cold1-newest-39"),
            "late lane never installed under another header: {screen}"
        );
        assert!(
            second.path.exists(),
            "newer read stays held during late-answer checks"
        );
        if Instant::now() >= until {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    drop(second);
    rig.wait(
        &pane,
        WAIT,
        "released cold lane replaces loading with its own content",
        |screen| {
            header_is(screen, "sn-cold2")
                && screen.contains("sn-cold2-newest-39")
                && !screen.contains("loading")
        },
    );
}

#[test]
fn a_selected_journal_append_appears_within_refresh_plus_slack() {
    let rig = Rig::new("snappend", &[]);
    let (pane, _) = rig.open();
    rig.ready(&pane, 1);
    let mut journal = fs::OpenOptions::new()
        .append(true)
        .open(rig.tool.dir.join("events.jsonl"))
        .expect("regular journal");
    writeln!(journal, "{{\"ts\":\"2026-10-05T10:01:00Z\",\"actor\":\"telegram:42\",\"action\":\"send\",\"target\":\"lead\",\"summary\":\"sn-journal-appended\"}}").expect("append event");
    drop(journal);
    rig.wait(
        &pane,
        REFRESH_SLACK,
        "selected journal append appears within REFRESH (5s) + 3s",
        |screen| screen.contains("sn-journal-appended"),
    );
}

#[test]
fn a_close_outcome_paints_without_waiting_for_a_held_journal_read() {
    let rig = Rig::new("snclose", &[]);
    let pane = rig.open_owner();
    rig.ready(&pane, 1);
    rig.wait(&pane, WAIT, "the fixture app owns its composer", |screen| {
        screen.contains(&format!("to {} › lead", rig.home))
    });
    rig.keys(&pane, "i");
    rig.wait(&pane, WAIT, "writing starts", writing);
    rig.literal(&pane, "/close");
    rig.wait(
        &pane,
        WAIT,
        "close command draft prepared before holding the read",
        |screen| screen.contains(&format!("to {} › lead   /close", rig.home)),
    );
    let hold = rig.hold(&rig.home);
    rig.held(
        &hold,
        &rig.home,
        "home background journal reread before close submit",
    );
    rig.keys(&pane, "Enter");
    rig.wait(
        &pane,
        FRAME,
        "close outcome paints while journal reread is held",
        |screen| screen.contains("refused: no open ask to close"),
    );
    assert!(
        hold.path.exists(),
        "close outcome never needed inline view/release"
    );
}

fn quit_during_read(tag: &str, key: &str) {
    let rig = Rig::new(tag, &[]);
    let (pane, modes) = rig.open();
    rig.ready(&pane, 1);
    let hold = rig.hold(&rig.home);
    rig.held(
        &hold,
        &rig.home,
        "quit fixture loader is inside a real journal read",
    );
    let until = Instant::now() + QUIT;
    rig.keys(&pane, key);
    loop {
        if fs::read_to_string(&modes).is_ok_and(|text| !text.is_empty()) {
            break;
        }
        assert!(
            Instant::now() < until,
            "{key} exits and restores terminal within 1s while read remains held"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(
        rig.tmux(&["display-message", "-p", "-t", &pane, "#{alternate_on}"])
            .trim(),
        "0",
        "quit leaves alternate screen"
    );
    let modes = fs::read_to_string(modes).expect("restored terminal modes");
    let flags = modes.split_whitespace().collect::<Vec<_>>();
    for flag in ["icanon", "echo"] {
        assert!(flags.contains(&flag), "quit restores {flag}: {modes}");
        assert!(
            !flags.contains(&format!("-{flag}").as_str()),
            "quit leaves {flag} enabled: {modes}"
        );
    }
    assert!(
        hold.path.exists(),
        "quit never needed release/join of the busy loader"
    );
}

#[test]
fn q_restores_the_terminal_within_one_second_of_a_held_read() {
    quit_during_read("snquit", "qq");
}

#[test]
fn control_c_restores_the_terminal_within_one_second_of_a_held_read() {
    quit_during_read("snctrl", "C-c");
}
