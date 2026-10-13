//! Frozen appside acceptance: final lead rulings R1/R2/R3, 2026-10-08.
//! Facts belong to the selected World entry; no second meta or tmux UUID read.
//! Terminal predicates tolerate partial paints and require two equal frames.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance creates private stores, terminals and read-gate fixtures"
)]

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ae::app::draw::{Composer, Screen, draw};
use ae::app::fleet::{Counts, Fleet, Line2, Row};
use ae::app::model::Model;
use ae::app::overview;
use ae::console::lane::Lane;
use ae::digest::{SessionEntry, Status};
use ae::theme::{Look, Mark};
use ae::time::Timestamp;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const TID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);

fn fleet(count: usize) -> Fleet {
    Fleet {
        rows: (1..=count)
            .map(|index| Row {
                name: format!("side{index:02}"),
                index,
                mark: Mark::Working,
                needy: false,
                counts: Counts::Unknown,
                line2: Line2::NoGoal,
                home: index == 1,
            })
            .collect(),
        home: Some("side01".to_owned()),
    }
}

fn frame(width: u16, height: u16, count: usize, entry: &SessionEntry) -> String {
    let fleet = fleet(count);
    let model = Model::new(&fleet);
    let now = Timestamp::from_epoch(1_791_446_400);
    let overview = overview::of(entry, None, None, now);
    let lane = Lane::default();
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(entry),
        pair: &[],
        agents: None,
        lane: &lane,
        composer: Composer::Home {
            home: "side01",
            speaker: "lead",
            view: None,
            draft: "",
        },
        look: Some(Look::read("off", "", "off", "off")),
        zone: None,
        now,
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    draw(&screen, &mut buf);
    (0..height)
        .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn sidebar(screen: &str) -> Vec<String> {
    let Some(width) = screen.lines().next().and_then(|line| line.find('│')) else {
        return Vec::new();
    };
    // The prefix before the first rule is ASCII, so bytes and cells agree.
    screen
        .lines()
        .map(|line| line.chars().take(width).collect())
        .collect()
}

/// The chat header row: the session, its pair and its recorded dir.
fn header(screen: &str) -> &str {
    screen.lines().next().unwrap_or_default()
}

fn border(screen: &str) -> Option<usize> {
    sidebar(screen)
        .iter()
        .position(|line| line.starts_with("──"))
}

fn body(screen: &str) -> Vec<String> {
    let Some(top) = border(screen) else {
        return Vec::new();
    };
    let side = sidebar(screen);
    side.iter()
        .skip(top + 1)
        .take(side.len().saturating_sub(top + 1))
        .map(|row| row.trim().to_owned())
        .collect()
}

#[test]
fn undragged_tall_frames_show_more_of_twenty_four_sessions() {
    let entry = SessionEntry::new("side01", Status::Running);
    // Exact observable pins from R1; no copy of the implementation formula.
    for (height, shown, rule) in [
        (30, 6, 19),
        (40, 9, 25),
        (43, 10, 27),
        (45, 11, 29),
        (62, 16, 39),
    ] {
        let screen = frame(160, height, 24, &entry);
        let cards = sidebar(&screen)
            .iter()
            .filter(|row| row.split_whitespace().any(|word| word.starts_with("side")))
            .count();
        assert_eq!(cards, shown, "160x{height}:\n{screen}");
        assert_eq!(border(&screen), Some(rule), "160x{height}");
        assert!(body(&screen).len() >= 3, "readable tab body remains");
        assert!(
            screen
                .lines()
                .last()
                .is_some_and(|row| row.contains("? keys"))
        );
    }
}

#[test]
fn minimum_frame_keeps_the_capped_list_and_chat_usable() {
    let entry = SessionEntry::new("side01", Status::Running);
    let screen = frame(90, 20, 24, &entry);
    assert_eq!(border(&screen), Some(15), "R1 capped-small-frame rule");
    let side = sidebar(&screen);
    assert!(side[4].contains("side01"));
    assert!(side[10].contains("side04"));
    assert!(side[12].contains("20 more"));
    assert!(side[14].contains("Agents") && !side[14].contains("Overview"));
    assert!(
        screen.contains("Enter writes"),
        "composer survives: {screen}"
    );
    assert!(
        screen
            .lines()
            .last()
            .is_some_and(|row| row.contains("? keys"))
    );
}

#[test]
fn one_session_keeps_its_content_sized_list_at_both_required_frames() {
    let entry = SessionEntry::new("side01", Status::Running);
    for (width, height) in [(90, 20), (160, 45)] {
        let screen = frame(width, height, 1, &entry);
        assert_eq!(border(&screen), Some(8), "{width}x{height}");
        assert!(sidebar(&screen)[4].contains("side01"));
        assert!(screen.contains("Enter writes"));
    }
}

struct Rig {
    tool: super::deliver::Rig,
    root: PathBuf,
    socket: PathBuf,
    store: PathBuf,
    gates: PathBuf,
    name: String,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let tool = super::deliver::Rig::new(tag, "codex", 0);
        let root = tool
            .dir
            .parent()
            .expect("sessions")
            .parent()
            .expect("root")
            .to_owned();
        let name = tool
            .dir
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        let store = root.join("claude-home");
        let gates = root.join("read-gates");
        fs::create_dir(&gates).expect("private gates");
        super::board::plant_transcript(
            &store,
            "work",
            TID,
            &[super::board::user("2026-10-08T08:00:00Z", "side-live-lane")],
        );
        let rig = Self {
            socket: root.join("sock"),
            tool,
            root,
            store,
            gates,
            name,
        };
        rig.meta("git", "/launch/work", Some(UUID));
        fs::write(rig.root.join("config"), "[workspace]\nchat = app\n").expect("config");
        for (key, value) in [
            ("@ae_look", "off"),
            ("@ae_session_uuid", UUID),
            ("@ae_main_pane", rig.tool.pane.as_str()),
        ] {
            rig.tmux(&["set-option", "-t", &rig.name, key, value]);
        }
        rig
    }

    fn meta(&self, mode: &str, dir: &str, id: Option<&str>) {
        let identity = id.map_or_else(String::new, |id| format!("session_id={id}\n"));
        fs::write(self.tool.dir.join("meta"), format!(
            "schema=2\nsession={}\n{identity}mode={mode}\nwork_dir={dir}\norigin=/launch/source\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=claude\nlaunch_id.main=tok-side\nharness_session.main={TID}\nconfig_home.main={}\nseat.worker.0=colead\nagent_bin.worker.0=claude\nlaunch_id.worker.0=tok-colead\nconfig_home.worker.0={}\n",
            self.name, self.socket.display(), self.store.display(), self.store.display(),
        )).expect("fixture meta");
        fs::write(self.tool.dir.join("events.jsonl"), "").expect("empty journal");
    }

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|word| (*word).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &self.root);
        assert!(ok, "private tmux {tail:?}: {out}");
        out
    }

    fn open(&self) -> String {
        let command = format!(
            "exec env -u CLAUDE_CONFIG_DIR -u CODEX_HOME {} app {}",
            quote(env!("CARGO_BIN_EXE_ae")),
            quote(&self.name)
        );
        let env = [
            format!("AE_HOME={}", self.root.display()),
            format!("HOME={}", self.root.display()),
            format!("CONFIG_FILE={}", self.root.join("config").display()),
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
            &self.name,
        ];
        for pair in &env {
            args.extend(["-e", pair]);
        }
        args.push(&command);
        let pane = self.tmux(&args).trim().to_owned();
        self.tmux(&["resize-window", "-t", &pane, "-x", "160", "-y", "45"]);
        pane
    }

    fn wait(&self, pane: &str, limit: Duration, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + limit;
        let mut previous = None;
        loop {
            let screen = self.tmux(&["capture-pane", "-p", "-t", pane]);
            if met(&screen) && previous.as_deref() == Some(screen.as_str()) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}; screen:\n{screen}");
            previous = Some(screen);
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn key(&self, pane: &str, bytes: &str) {
        self.tmux(&["send-keys", "-t", pane, "-l", "--", bytes]);
    }
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[test]
fn live_facts_refresh_from_the_selected_world_even_without_a_session_id() {
    let rig = Rig::new("sidelive");
    let pane = rig.open();
    rig.wait(
        &pane,
        WAIT,
        "GUARD actual app reads fixture lane",
        |screen| screen.contains("side-live-lane"),
    );
    rig.wait(&pane, WAIT, "the recorded dir is in the header", |screen| {
        header(screen).contains("/launch/work")
    });
    for id in [None, Some("not-a-canonical-id")] {
        rig.meta("full", "/replacement/work", id);
        rig.wait(
            &pane,
            WAIT,
            "the same world's dir refreshes; id is not a facts gate",
            |screen| header(screen).contains("/replacement/work"),
        );
    }
}

#[test]
fn keys_repaint_while_the_background_fleet_read_is_held() {
    let rig = Rig::new("sideasync");
    let pane = rig.open();
    rig.wait(
        &pane,
        WAIT,
        "GUARD actual app reads fixture lane",
        |screen| screen.contains("side-live-lane"),
    );
    let gate = rig.gates.join("@world");
    fs::write(&gate, "hold").expect("hold only fixture reader");
    let until = Instant::now() + WAIT;
    while !fs::read_to_string(rig.gates.join("trace"))
        .unwrap_or_default()
        .contains("held @world")
    {
        assert!(Instant::now() < until, "fleet read never attested held");
        std::thread::sleep(Duration::from_millis(25));
    }
    rig.key(&pane, "s");
    rig.wait(
        &pane,
        WAIT,
        "settings repaints while world read is held",
        |screen| screen.contains("Settings"),
    );
    assert!(gate.exists(), "key did not release the background read");
    rig.key(&pane, "\x1b");
    rig.wait(
        &pane,
        WAIT,
        "Esc repaints the held snapshot's dir",
        |screen| header(screen).contains("/launch/work"),
    );
    fs::remove_file(gate).expect("release fixture reader");
}
