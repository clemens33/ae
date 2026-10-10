//! Frozen acceptance for version cells and the read-only settings screen.
//! Oracles: brief-appsettings objectives, lead's four-source ruling, fixture
//! bytes and the terminal's drawn cells. No settings implementation API, no
//! fabricated loader answers, no real harness stores. RED on the unchanged
//! base is a missing version/opened settings frame, never a missing symbol.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal/config/cache fixtures and reads their effects"
)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ae::app::draw::{Composer, Screen, draw};
use ae::app::fleet::Fleet;
use ae::app::model::Model;
use ae::app::overview::Overview;
use ae::console::lane::Lane;
use ae::theme::Look;
use ae::time::Timestamp;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::Color;

#[path = "app_instructions_spec.rs"]
mod instructions;
#[path = "app_typing_spec.rs"]
mod typing;

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const TID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);
const FRAME: Duration = Duration::from_secs(3);
const REFRESH_SLACK: Duration = Duration::from_secs(12);
const HINT: &str = "  browse   ? keys";
const WORKSPACE_KEYS: [&str; 20] = [
    "main",
    "workers",
    "layout",
    "auto_upgrade",
    "fleet_order",
    "restore",
    "palette",
    "icons",
    "theme",
    "motion",
    "chat",
    "quota",
    "quota_every_secs",
    "idle_nudge_secs",
    "done_confirmations",
    "auto_reseat",
    "auto_reseat_sessions",
    "auto_reseat_grace_secs",
    "auto_reseat_at",
    "purge_agent_history",
];

fn closed_frame(width: u16, height: u16, look: &Look, home: bool) -> Buffer {
    let fleet = Fleet {
        rows: Vec::new(),
        home: Some("fixture".to_owned()),
    };
    let model = Model::new(&fleet);
    let overview = Overview::default();
    let lane = Lane::default();
    let composer = if home {
        Composer::Home {
            home: "fixture",
            speaker: "lead",
            view: None,
            draft: "",
        }
    } else {
        Composer::NoHome
    };
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: None,
        pair: &[],
        agents: None,
        lane: &lane,
        composer,
        look: Some(*look),
        zone: None,
        now: Timestamp::from_epoch(1_791_200_000),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    let _ = draw(&screen, &mut buf);
    buf
}

fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

#[test]
fn version_and_gear_end_the_keys_row_after_its_one_hint() {
    for width in [160, 220, 300] {
        for icons in [true, false] {
            let look = Look::read(if icons { "on" } else { "off" }, "", "off", "off");
            let buf = closed_frame(width, 28, &look, false);
            let line = row(&buf, 27);
            assert!(line.starts_with(HINT), "mode word and hint: {line:?}");
            let tail = format!("{} {}", ae::version_line(), if icons { "⚙" } else { "*" });
            assert!(line.trim_end().ends_with(&tail), "right end: {line:?}");
            let start = line.find(&ae::version_line()).expect("drawn version");
            assert!(start >= HINT.len() + 2, "version overlaps the hint");
        }
    }
}

#[test]
fn theme_off_adds_no_colour_to_version_and_gear() {
    let buf = closed_frame(160, 43, &Look::read("off", "", "off", "off"), false);
    assert!(
        row(&buf, 42).contains(&ae::version_line()),
        "version is drawn"
    );
    for cell in &buf.content {
        assert_eq!(cell.fg, Color::Reset);
        assert_eq!(cell.bg, Color::Reset);
    }
}

/// Attested read hold, released both on assertion unwind and at a deadline.
struct Hold {
    path: PathBuf,
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
    name: String,
    store: PathBuf,
    gates: PathBuf,
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
            .expect("session")
            .to_string_lossy()
            .into_owned();
        let socket = root.join("sock");
        let store = root.join("claude-home");
        let gates = root.join("read-gates");
        fs::create_dir(&gates).expect("private read gates");
        super::board::plant_transcript(
            &store,
            "work",
            TID,
            &[super::board::user(
                "2026-10-04T10:00:00Z",
                "settings-fixture-lane",
            )],
        );
        let rig = Self {
            tool,
            root,
            socket,
            name,
            store,
            gates,
        };
        rig.meta("");
        fs::write(rig.root.join("config"), "[workspace]\nchat = app\n").expect("config");
        for (key, value) in [
            ("@ae_look", "off"),
            ("@ae_icons", "off"),
            ("@ae_session_uuid", UUID),
            ("@ae_motion", "off"),
            ("@ae_main_pane", rig.tool.pane.as_str()),
            ("default-size", "220x60"),
        ] {
            rig.tmux(&["set-option", "-t", &rig.name, key, value]);
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

    fn meta(&self, extra: &str) {
        fs::write(self.tool.dir.join("meta"), format!(
            "schema=2\nsession={}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=claude\nlaunch_id.main=tok-settings\nharness_session.main={TID}\nconfig_home.main={}\nseat.worker.0=colead\nagent_bin.worker.0=claude\nlaunch_id.worker.0=tok-colead\nconfig_home.worker.0={}\n{extra}",
            self.name, self.socket.display(), self.store.display(), self.store.display(),
        )).expect("fixture meta");
        fs::write(self.tool.dir.join("events.jsonl"), "").expect("fixture journal");
    }

    fn stopped(&self, name: &str) {
        let dir = self.root.join("sessions").join(name);
        let serial = fs::read_dir(self.root.join("sessions"))
            .expect("fixture sessions")
            .count();
        let id = format!("0199c0de-bbbb-4890-abcd-{serial:012x}");
        fs::create_dir_all(&dir).expect("stopped fixture session");
        let meta = fs::read_to_string(self.tool.dir.join("meta"))
            .expect("fixture meta")
            .replace(
                &format!("session={}\n", self.name),
                &format!("session={name}\n"),
            )
            .replace(UUID, &id);
        fs::write(dir.join("meta"), meta).expect("stopped fixture meta");
    }

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &self.root);
        assert!(ok, "private tmux {tail:?}: {out}");
        out
    }

    fn direct(&self, width: u16, height: u16, config: &Path) -> (String, PathBuf) {
        let exited = self.root.join("app-exited");
        let command = format!(
            "env -u CLAUDE_CONFIG_DIR -u CODEX_HOME {} app {}; stty -a > {}; sleep 30",
            quote(env!("CARGO_BIN_EXE_ae")),
            quote(&self.name),
            quote(&exited.to_string_lossy()),
        );
        let env = [
            format!("AE_HOME={}", self.root.display()),
            format!("HOME={}", self.root.display()),
            format!("CONFIG_FILE={}", config.display()),
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
            "-c",
        ];
        let cwd = self.root.to_string_lossy();
        args.push(&cwd);
        for pair in &env {
            args.extend(["-e", pair.as_str()]);
        }
        args.push(&command);
        let pane = self.tmux(&args).trim().to_owned();
        self.tmux(&[
            "resize-window",
            "-t",
            &pane,
            "-x",
            &width.to_string(),
            "-y",
            &height.to_string(),
        ]);
        (pane, exited)
    }

    fn owner(&self) -> String {
        for (key, value) in [
            (
                "AE_TEST_APP_READ_GATE",
                self.gates.to_string_lossy().into_owned(),
            ),
            ("HOME", self.root.to_string_lossy().into_owned()),
        ] {
            self.tmux(&["set-environment", "-t", &self.name, key, &value]);
        }
        for key in ["CLAUDE_CONFIG_DIR", "CODEX_HOME"] {
            self.tmux(&["set-environment", "-r", "-t", &self.name, key]);
        }
        let out = super::cli::ae()
            .current_dir(&self.root)
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
            .expect("owning app starts");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let pane = self
            .tmux(&[
                "list-panes",
                "-s",
                "-t",
                &self.name,
                "-F",
                "#{pane_id}|#{@ae_console}",
            ])
            .lines()
            .find_map(|line| {
                let (pane, stamp) = line.split_once('|')?;
                (stamp == UUID).then(|| pane.to_owned())
            })
            .expect("stamped app pane");
        self.tmux(&["resize-window", "-t", &pane, "-x", "220", "-y", "60"]);
        pane
    }

    fn screen(&self, pane: &str) -> String {
        self.tmux(&["capture-pane", "-p", "-t", pane])
    }

    fn styled(&self, pane: &str) -> String {
        self.tmux(&["capture-pane", "-p", "-e", "-t", pane])
    }

    fn wait(&self, pane: &str, within: Duration, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + within;
        let mut previous = None;
        loop {
            let screen = self.screen(pane);
            if met(&screen) && previous.as_deref() == Some(screen.as_str()) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}; terminal screen:\n{screen}");
            previous = Some(screen);
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn ready(&self, pane: &str) -> String {
        self.wait(pane, WAIT, "home fixture drawn", |s| {
            s.contains("settings-fixture-lane")
        })
    }

    fn settings(&self, pane: &str) -> String {
        self.keys(pane, "s");
        self.wait(pane, FRAME, "s opens the settings frame", is_settings)
    }

    fn keys(&self, pane: &str, key: &str) {
        self.tmux(&["send-keys", "-t", pane, key]);
    }

    fn literal(&self, pane: &str, text: &str) {
        self.tmux(&["send-keys", "-t", pane, "-l", "--", text]);
    }

    fn click(&self, pane: &str, column: usize, row: usize) {
        self.literal(pane, &format!("\x1b[<0;{};{}M", column + 1, row + 1));
    }

    fn gear(&self, pane: &str) {
        let screen = self.screen(pane);
        let (y, line) = screen.lines().enumerate().last().expect("keys row");
        let x = line
            .chars()
            .position(|ch| ch == '*' || ch == '⚙')
            .expect("drawn gear");
        self.click(pane, x, y);
    }

    fn tab(&self, pane: &str, key: &str, wanted: &str) {
        self.keys(pane, key);
        let until = Instant::now() + FRAME;
        loop {
            let screen = self.styled(pane);
            let tabs = screen.lines().nth(1).unwrap_or_default();
            if reversed_text(tabs).contains(wanted) {
                return;
            }
            assert!(Instant::now() < until, "{wanted} tab selected: {screen:?}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn hold(&self) -> Hold {
        self.hold_read("settings")
    }

    fn hold_read(&self, key: &str) -> Hold {
        let path = self.gates.join(key);
        fs::write(&path, "hold").expect("arm real read hold");
        let deadline = path.clone();
        let (release, released) = mpsc::channel();
        let watcher = std::thread::spawn(move || {
            let _ = released.recv_timeout(Duration::from_secs(45));
            let _ = fs::remove_file(deadline);
        });
        Hold {
            path,
            release: Some(release),
            watcher: Some(watcher),
        }
    }

    fn held(&self) {
        self.held_after("settings", 0);
    }

    fn held_count(&self, key: &str) -> usize {
        let marker = format!("held {key}");
        fs::read_to_string(self.gates.join("trace"))
            .unwrap_or_default()
            .lines()
            .filter(|line| *line == marker)
            .count()
    }

    fn held_after(&self, key: &str, previous: usize) {
        let until = Instant::now() + WAIT;
        loop {
            if self.held_count(key) > previous {
                return;
            }
            assert!(
                Instant::now() < until,
                "real {key} read never attested its next hold"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn quota(&self, count: usize, percent: u8) {
        let mut text = "[workspace]\nchat = app\n[clients]\n".to_owned();
        for index in 0..count {
            let name = format!("c{index:02}");
            let home = self.root.join(&name);
            fs::create_dir_all(&home).expect("fixture quota home");
            // One session window, raw 66% and one reset => effective 33% x1.
            // A fresh timestamp keeps the live refresh test independent of date.
            fs::write(home.join(".claude.json"), cache(percent)).expect("fixture quota cache");
            writeln!(
                text,
                "{name} = claude config_home=$HOME/{name} manual_resets=1"
            )
            .expect("fixture config string");
        }
        fs::write(self.root.join("config"), text).expect("fixture quota clients");
    }
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn is_settings(screen: &str) -> bool {
    screen
        .lines()
        .next()
        .is_some_and(|line| line.contains("Settings"))
        && screen.lines().nth(1).is_some_and(|line| {
            ["Quota", "Config", "About"]
                .iter()
                .all(|tab| line.contains(tab))
        })
}

fn cache(percent: u8) -> String {
    let now = Timestamp::now();
    let reset = Timestamp::from_epoch(now.epoch() + 3600);
    format!(
        "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"session\",\"group\":\"session\",\"percent\":{percent},\"resets_at\":\"{reset}\"}}]}}}}}}",
        now.epoch() * 1000,
    )
}

/// Terminal SGR observation, not app state: selected tab is reversed with
/// theme off. Only SGR 0/7/27 affect this independent terminal oracle.
fn reversed_text(text: &str) -> String {
    let mut chars = text.chars();
    let mut reverse = false;
    let mut selected = String::new();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.next() == Some('[') {
            let mut args = String::new();
            for byte in chars.by_ref() {
                if byte == 'm' {
                    break;
                }
                args.push(byte);
            }
            for arg in args.split(';') {
                match arg {
                    "" | "0" | "27" => reverse = false,
                    "7" => reverse = true,
                    _ => {}
                }
            }
        } else if reverse {
            selected.push(ch);
        }
    }
    selected
}

fn has_colour_sgr(text: &str) -> bool {
    text.split("\x1b[").skip(1).any(|tail| {
        let Some((args, _)) = tail.split_once('m') else {
            return false;
        };
        args.split(';')
            .filter_map(|arg| arg.parse::<u16>().ok())
            .any(|code| {
                (30..=49).contains(&code) && code != 39 && code != 49 || (90..=107).contains(&code)
            })
    })
}

fn config_row(screen: &str, key: &str, value: &str, source: &str) {
    let prefix = format!("{key} = ");
    let found = screen
        .lines()
        .find(|line| line.trim_start().starts_with(&prefix))
        .expect("config key is drawn");
    assert!(
        found.contains(&format!("{key} = {value}")),
        "{key} value: {found:?}"
    );
    let marker = format!("({source})");
    assert!(
        found.split_whitespace().any(|column| column == marker),
        "{key} provenance {source}: {found:?}"
    );
}

#[test]
fn settings_keys_tabs_and_all_three_close_keys_act_on_the_drawn_screen() {
    let rig = Rig::new("settingskeys");
    let (pane, _) = rig.direct(220, 60, &rig.root.join("config"));
    rig.ready(&pane);
    for close in ["Escape", "q", "s"] {
        let opened = rig.settings(&pane);
        let hints = opened.lines().last().expect("settings keys row");
        assert!(
            hints.contains("Esc close") && !hints.contains("^C"),
            "{opened}"
        );
        assert!(
            reversed_text(rig.styled(&pane).lines().nth(1).unwrap_or_default()).contains("Quota")
        );
        rig.tab(&pane, "Tab", "Config");
        rig.tab(&pane, "Tab", "About");
        rig.tab(&pane, "Tab", "Instructions");
        rig.tab(&pane, "Tab", "Keys");
        rig.tab(&pane, "Tab", "Quota");
        for (key, tab) in [("3", "About"), ("1", "Quota"), ("2", "Config")] {
            rig.tab(&pane, key, tab);
        }
        rig.keys(&pane, close);
        rig.wait(&pane, FRAME, "close restores the home frame", |s| {
            !is_settings(s) && s.contains("settings-fixture-lane")
        });
    }
}

#[test]
fn gear_opens_and_modal_keys_preserve_the_composer_draft_and_mode() {
    let rig = Rig::new("settingsdraft");
    let pane = rig.owner();
    rig.ready(&pane);
    rig.keys(&pane, "Enter");
    rig.literal(&pane, "draft-s");
    rig.wait(&pane, WAIT, "s is typed into the composer", |s| {
        s.contains("draft-s") && s.lines().last().is_some_and(|line| line.contains("write"))
    });
    rig.gear(&pane);
    rig.wait(&pane, FRAME, "gear opens while writing", is_settings);
    rig.tab(&pane, "3", "About");
    rig.literal(&pane, "hidden-text");
    rig.keys(&pane, "q");
    rig.wait(
        &pane,
        FRAME,
        "modal q closes rather than entering the draft",
        |s| {
            !is_settings(s)
                && s.contains("draft-s")
                && s.lines().last().is_some_and(|line| line.contains("write"))
        },
    );
    rig.literal(&pane, "-resumed");
    let screen = rig.wait(
        &pane,
        FRAME,
        "composer mode resumes with the same draft",
        |s| s.contains("draft-s-resumed"),
    );
    assert!(
        !screen.contains("hidden-text"),
        "overlay text reached composer: {screen}"
    );
}

#[test]
fn held_settings_read_shows_loading_on_every_tab_and_keys_keep_working() {
    let rig = Rig::new("settingscold");
    rig.quota(1, 66);
    let (pane, _) = rig.direct(220, 60, &rig.root.join("config"));
    rig.ready(&pane);
    let hold = rig.hold();
    rig.settings(&pane);
    rig.held();
    for (key, tab) in [
        ("1", "Quota"),
        ("2", "Config"),
        ("3", "About"),
        ("4", "Instructions"),
    ] {
        rig.tab(&pane, key, tab);
        rig.wait(
            &pane,
            FRAME,
            "cold tab names loading while read stays held",
            |s| s.contains("loading"),
        );
    }
    rig.tab(&pane, "1", "Quota");
    drop(hold);
    rig.wait(
        &pane,
        WAIT,
        "released real read replaces loading with fixture quota",
        |s| s.contains("claude/") && s.contains("33% x1") && !s.contains("loading"),
    );
}

#[test]
fn late_settings_answer_cannot_reopen_a_closed_panel_and_control_c_quits_inside_it() {
    let rig = Rig::new("settingslate");
    rig.quota(1, 66);
    let (pane, exited) = rig.direct(220, 60, &rig.root.join("config"));
    rig.ready(&pane);
    let hold = rig.hold();
    rig.settings(&pane);
    rig.held();
    rig.keys(&pane, "Escape");
    rig.wait(&pane, FRAME, "Esc closes during held read", |s| {
        !is_settings(s)
    });
    let previous = rig.held_count("@world");
    let world = rig.hold_read("@world");
    drop(hold);
    rig.held_after("@world", previous);
    // Reader::run sends answers sequentially through one Sender<Wake>.
    // Its later fleet answer follows the settings answer on that FIFO;
    // drawing this new sidebar row proves UI application, not just send.
    rig.stopped("settings-late-sentinel");
    drop(world);
    let screen = rig.wait(&pane, WAIT, "later fleet answer applied", |s| {
        s.contains("settings-late-sentinel")
    });
    assert!(
        !is_settings(&screen),
        "late answer reopened overlay: {screen}"
    );
    rig.settings(&pane);
    rig.keys(&pane, "C-c");
    let until = Instant::now() + FRAME;
    while !exited.exists() {
        assert!(Instant::now() < until, "control-C did not quit settings");
        std::thread::sleep(Duration::from_millis(25));
    }
    let modes = fs::read_to_string(exited).expect("terminal restored after exit");
    assert!(!modes.contains("-icanon"), "raw mode survived: {modes}");
}

#[test]
fn closed_settings_does_not_read_before_open_or_keep_refreshing_after_close() {
    let rig = Rig::new("settingsclosed");
    rig.quota(1, 66);
    let (pane, _) = rig.direct(220, 60, &rig.root.join("config"));
    rig.ready(&pane);
    let held = rig.hold();
    let cycle = |sentinel: &str| {
        let previous = rig.held_count("@world");
        let world = rig.hold_read("@world");
        rig.held_after("@world", previous);
        rig.stopped(sentinel);
        drop(world);
        rig.wait(&pane, WAIT, "closed fleet refresh applied", |s| {
            s.contains(sentinel)
        });
    };
    cycle("settings-cycle-one");
    cycle("settings-cycle-two");
    assert!(held.path.exists(), "settings hold stayed armed");
    assert_eq!(rig.held_count("settings"), 0, "closed app read settings");
    rig.settings(&pane);
    rig.held();
    rig.keys(&pane, "Escape");
    rig.wait(&pane, FRAME, "held panel closes", |s| !is_settings(s));
    let settings_reads = rig.held_count("settings");
    let previous = rig.held_count("@world");
    let world = rig.hold_read("@world");
    drop(held);
    rig.held_after("@world", previous);
    rig.stopped("settings-cycle-three");
    drop(world);
    rig.wait(&pane, WAIT, "post-close answer applied", |s| {
        s.contains("settings-cycle-three")
    });
    let held = rig.hold();
    cycle("settings-cycle-four");
    cycle("settings-cycle-five");
    assert!(held.path.exists(), "post-close settings hold stayed armed");
    assert_eq!(
        rig.held_count("settings"),
        settings_reads,
        "closed panel kept refreshing settings"
    );
}

#[test]
fn smallest_supported_settings_panel_keeps_title_tabs_and_a_close_hint() {
    let rig = Rig::new("settingstiny");
    let (pane, _) = rig.direct(220, 60, &rig.root.join("config"));
    rig.ready(&pane);
    rig.tmux(&["resize-window", "-t", &pane, "-x", "40", "-y", "8"]);
    rig.wait(&pane, FRAME, "smallest supported frame repainted", |s| {
        s.lines()
            .next()
            .is_some_and(|line| line.contains("now 40x8"))
    });
    let screen = rig.settings(&pane);
    assert!(
        screen
            .lines()
            .next()
            .is_some_and(|line| line.contains("Settings"))
    );
    assert!(screen.lines().nth(1).is_some_and(|line| {
        ["Quota", "Config", "About", "Instructions", "Keys"]
            .iter()
            .all(|tab| line.contains(tab))
    }));
    let hint = screen.lines().last().expect("tiny keys row");
    assert!(
        hint.contains("close") && (hint.contains("Esc") || hint.contains('q')),
        "close hint remains visible at 40x8: {screen}"
    );
    rig.keys(&pane, "Escape");
    rig.wait(&pane, FRAME, "tiny overlay closes", |s| !is_settings(s));
}

#[test]
fn quota_fixture_rows_keep_the_shared_effective_derivation_and_refresh_while_open() {
    let rig = Rig::new("settingsquota");
    rig.quota(1, 66);
    let config_before = fs::read(rig.root.join("config")).expect("config before read-only screen");
    let cache_path = rig.root.join("c00/.claude.json");
    let cache_before = fs::read(&cache_path).expect("cache before screen");
    let (pane, _) = rig.direct(220, 60, &rig.root.join("config"));
    rig.ready(&pane);
    rig.settings(&pane);
    let screen = rig.wait(&pane, WAIT, "fixture quota window drawn", |s| {
        s.contains("33% x1")
    });
    let quota_rows = screen
        .lines()
        .filter(|line| line.contains("window resets"))
        .collect::<Vec<_>>();
    assert_eq!(
        quota_rows.len(),
        1,
        "one fixture window, no invented rows: {screen}"
    );
    let label = quota_rows[0].trim_start();
    assert!(
        label.starts_with("session 5h | 33% x1 | window resets "),
        "shared row: {label}"
    );
    assert!(
        label.contains(" | seen ") && label.ends_with(" | fresh"),
        "shared provenance: {label}"
    );
    assert_eq!(
        fs::read(&cache_path).expect("cache after open"),
        cache_before,
        "settings wrote quota cache"
    );
    fs::write(&cache_path, cache(80)).expect("new external fixture quota sample");
    rig.wait(
        &pane,
        REFRESH_SLACK,
        "open quota refreshes: raw 80 / (1+1) = 40",
        |s| s.contains("40% x1") && !s.contains("33% x1"),
    );
    assert_eq!(
        fs::read(rig.root.join("config")).expect("config after settings"),
        config_before
    );
}

#[test]
fn settings_scroll_keys_and_wheel_use_the_panel_body_and_reset_on_tab_switch() {
    let rig = Rig::new("settingsscroll");
    rig.quota(16, 66);
    let (pane, _) = rig.direct(160, 12, &rig.root.join("config"));
    rig.ready(&pane);
    rig.settings(&pane);
    let first = rig.wait(&pane, WAIT, "scrollable fixture quota body drawn", |s| {
        s.contains("33% x1")
    });
    let top = first.lines().nth(3).expect("first body row").to_owned();
    for (down, up) in [("j", "k"), ("Down", "Up"), ("PageDown", "PageUp")] {
        rig.keys(&pane, down);
        rig.wait(&pane, FRAME, "panel scrolls down", |s| {
            s.lines().nth(3) != Some(top.as_str())
        });
        rig.keys(&pane, up);
        rig.wait(&pane, FRAME, "panel returns to its first row", |s| {
            s.lines().nth(3) == Some(top.as_str())
        });
    }
    // Scroll works over the left side too; hidden chat geometry cannot vote.
    rig.literal(&pane, "\x1b[<65;2;5M");
    rig.wait(&pane, FRAME, "wheel over panel moves body", |s| {
        s.lines().nth(3) != Some(top.as_str())
    });
    rig.literal(&pane, "\x1b[<64;2;5M");
    rig.wait(&pane, FRAME, "wheel returns panel", |s| {
        s.lines().nth(3) == Some(top.as_str())
    });
    rig.keys(&pane, "PageDown");
    rig.wait(&pane, FRAME, "page moves before tab switch", |s| {
        s.lines().nth(3) != Some(top.as_str())
    });
    rig.tab(&pane, "2", "Config");
    rig.tab(&pane, "1", "Quota");
    rig.wait(&pane, FRAME, "switching tabs resets scroll", |s| {
        s.lines().nth(3) == Some(top.as_str())
    });
}

#[test]
fn config_draws_all_documented_keys_and_launch_session_global_default_provenance() {
    let rig = Rig::new("settingsconfig");
    let recorded = rig.root.join("recorded-config");
    let origin = rig.root.join("origin");
    fs::create_dir_all(origin.join(".ae")).expect("fixture origin overlay");
    let overlay = origin.join(".ae/config");
    fs::write(&recorded, "[workspace]\nmain = alpha-lead\nworkers = alpha-worker\nicons = on\nquota = on\nquota_every_secs = 90\nfleet_order = alpha-fleet\nrestore = on\n").expect("recorded global");
    fs::write(&overlay, "[workspace]\nworkers = beta-worker\nicons = off\nquota = on\nquota_every_secs = 80\nfleet_order = beta-fleet\nrestore = on\n").expect("session overlay");
    fs::write(rig.root.join("config"), "[workspace]\nchat = app\nmain = gamma-lead\npalette = warm\nfleet_order = gamma-fleet\nrestore = off\n").expect("different current global");
    rig.meta(&format!("config={}\norigin={}\nquota=off\nquota_every_secs=17\nidle_nudge_secs=19\ndone_confirmations=4\n", recorded.display(), origin.display()));
    let originals =
        [&recorded, &overlay, &rig.root.join("config")].map(|path| fs::read(path).expect("before"));
    let (pane, _) = rig.direct(220, 60, &rig.root.join("config"));
    rig.ready(&pane);
    rig.settings(&pane);
    rig.tab(&pane, "2", "Config");
    let screen = rig.wait(&pane, WAIT, "resolved fixture config drawn", |s| {
        s.contains("done_confirmations = 4")
    });
    for (key, value, source) in [
        ("main", "alpha-lead", "global"),
        ("workers", "beta-worker", "session"),
        ("layout", "lead-pair", "launch"),
        ("auto_upgrade", "on", "default"),
        ("palette", "darcula", "default"),
        ("icons", "off", "session"),
        ("theme", "on", "default"),
        ("motion", "on", "default"),
        ("chat", "on", "default"),
        ("quota", "off", "launch"),
        ("quota_every_secs", "17", "launch"),
        ("idle_nudge_secs", "19", "launch"),
        ("done_confirmations", "4", "launch"),
        ("fleet_order", "gamma-fleet", "global"),
        ("restore", "off", "global"),
        ("auto_reseat", "off", "default"),
        ("auto_reseat_sessions", "(every session)", "default"),
        ("auto_reseat_grace_secs", "600", "default"),
        ("auto_reseat_at", "95", "default"),
        ("purge_agent_history", "off", "default"),
    ] {
        config_row(&screen, key, value, source);
    }
    let keys = screen
        .lines()
        .filter_map(|line| {
            line.trim_start()
                .split_once(" = ")
                .map(|(key, _)| key.to_owned())
        })
        .collect::<Vec<_>>();
    assert_eq!(keys, WORKSPACE_KEYS);
    for key in [
        "fleet_order",
        "restore",
        "auto_upgrade",
        "auto_reseat",
        "auto_reseat_sessions",
        "auto_reseat_grace_secs",
        "auto_reseat_at",
    ] {
        let line = screen
            .lines()
            .find(|line| line.trim_start().starts_with(&format!("{key} = ")))
            .expect("global-only row");
        assert!(
            line.contains("global only"),
            "global-only scope is visible: {line}"
        );
    }
    for path in [&recorded, &overlay, &rig.root.join("config")] {
        assert_eq!(
            screen.matches(&path.to_string_lossy().to_string()).count(),
            1,
            "source file named once: {screen}"
        );
    }
    let after =
        [&recorded, &overlay, &rig.root.join("config")].map(|path| fs::read(path).expect("after"));
    assert_eq!(after, originals, "settings must not write configs");
}

#[test]
fn covered_session_targets_and_drag_borders_cannot_act_through_settings() {
    let rig = Rig::new("settingshidden");
    rig.stopped("settings-other");
    let (pane, _) = rig.direct(220, 60, &rig.root.join("config"));
    rig.ready(&pane);
    let before = rig.wait(&pane, WAIT, "covered targets drawn before open", |s| {
        s.contains("settings-other") && s.contains("Overview")
    });
    let target = |needle: &str| {
        before
            .lines()
            .enumerate()
            .find_map(|(y, line)| line.find(needle).map(|x| (line[..x].chars().count(), y)))
            .expect("drawn old target")
    };
    let session = target("settings-other");
    let tab = target("Overview");
    let border = before
        .lines()
        .next()
        .expect("header")
        .chars()
        .position(|ch| ch == '│')
        .expect("drawn sidebar border");
    // Preserve both header rows and border positions. The far-right age
    // can tick independently while the modal is open, so exclude it.
    let signature = |screen: &str| {
        screen
            .lines()
            .take(2)
            .map(|line| line.chars().take(120).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let header = signature(&before);
    rig.settings(&pane);
    rig.tab(&pane, "3", "About");
    rig.click(&pane, session.0, session.1);
    rig.click(&pane, tab.0, tab.1);
    rig.click(&pane, border, 1);
    rig.literal(
        &pane,
        &format!("\x1b[<32;{};2M\x1b[<0;{};2m", border + 22, border + 22),
    );
    rig.keys(&pane, "Enter");
    rig.keys(&pane, "2");
    rig.keys(&pane, "4");
    rig.keys(&pane, "q");
    rig.wait(
        &pane,
        FRAME,
        "close restores original selection and split",
        |s| !is_settings(s) && signature(s) == header,
    );
}

#[test]
fn malformed_config_is_one_honest_row_instead_of_an_empty_or_default_table() {
    let rig = Rig::new("settingsbad");
    let broken = rig.root.join("broken-config");
    fs::write(
        &broken,
        "[workspace]\nworkers = ignored\n[prompt]\ninstructions = \"\"\"\nunclosed\n",
    )
    .expect("torn fixture config");
    rig.meta(&format!("config={}\n", broken.display()));
    let (pane, _) = rig.direct(220, 60, &rig.root.join("config"));
    rig.ready(&pane);
    rig.settings(&pane);
    rig.tab(&pane, "2", "Config");
    let screen = rig.wait(&pane, WAIT, "broken config named", |s| {
        s.contains(&broken.to_string_lossy().to_string()) && !s.contains("loading")
    });
    let errors = screen
        .lines()
        .filter(|line| {
            line.contains(&broken.to_string_lossy().to_string())
                && (line.contains("unavailable")
                    || line.contains("unreadable")
                    || line.contains("unclosed")
                    || line.contains("unterminated"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        errors.len(),
        1,
        "one row names broken file and reason: {screen}"
    );
    assert!(
        !screen.lines().any(|line| WORKSPACE_KEYS
            .iter()
            .any(|key| { line.trim_start().starts_with(&format!("{key} = ")) })),
        "error cannot masquerade as defaults: {screen}"
    );
}

#[test]
fn hostile_config_values_and_about_paths_are_visible_as_neutralised_text() {
    let rig = Rig::new("settingshostile");
    let hostile = rig.root.join("config-\x1b[31m-\x07");
    fs::write(
        &hostile,
        "[workspace]\nchat = app\nworkers = value\x1b[31m\x07\0\n",
    )
    .expect("hostile text fixture");
    // This fixture is usable under the existing config value grammar:
    // the UI must neutralise its controls, not hide it as a parse failure.
    let text = fs::read_to_string(&hostile).expect("hostile fixture readable");
    assert_eq!(
        ae::config::workspace_key_in(&hostile, &text, "workers").expect("existing value grammar"),
        Some("value\x1b[31m\x07\0".to_owned())
    );
    let (pane, _) = rig.direct(220, 60, &hostile);
    rig.ready(&pane);
    rig.settings(&pane);
    rig.tab(&pane, "2", "Config");
    let screen = rig.wait(&pane, WAIT, "controls remain visible but inert", |s| {
        s.contains("value�[31m��")
    });
    assert!(!screen.contains('\x1b') && !screen.contains('\x07') && !screen.contains('\0'));
    rig.tab(&pane, "3", "About");
    rig.wait(&pane, WAIT, "hostile config path neutralised", |s| {
        s.contains("config-�[31m-�")
    });
    assert!(
        !has_colour_sgr(&rig.styled(&pane)),
        "hostile text injected colour"
    );
}

#[test]
fn about_draws_versions_floor_paths_recorded_server_and_plain_links_without_colour() {
    let rig = Rig::new("settingsabout");
    let (pane, _) = rig.direct(220, 60, &rig.root.join("config"));
    rig.ready(&pane);
    rig.settings(&pane);
    rig.tab(&pane, "3", "About");
    let screen = rig.wait(&pane, WAIT, "about facts loaded", |s| {
        s.contains("https://github.com/clemens33/ae/releases")
    });
    let tmux = rig.tmux(&["display-message", "-p", "#{version}"]);
    assert!(
        screen
            .lines()
            .any(|line| line.trim().ends_with(&ae::version_line())),
        "ae version row: {screen}"
    );
    let tmux_row = screen
        .lines()
        .find(|line| {
            line.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.')
                .any(|word| word == tmux.trim())
                && line.to_ascii_lowercase().contains("tmux")
        })
        .expect("actual addressed tmux version has its own row");
    assert!(
        tmux_row
            .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.')
            .any(|word| word == "ok")
            || tmux_row.contains("clears the 3.4 floor"),
        "floor verdict beside version: {tmux_row}"
    );
    assert!(
        screen.lines().any(|line| line
            .trim()
            .ends_with(&rig.root.to_string_lossy().to_string())),
        "state root has its own row: {screen}"
    );
    assert!(
        screen.lines().any(|line| line
            .trim()
            .ends_with(&rig.root.join("config").to_string_lossy().to_string())),
        "config path has its own row: {screen}"
    );
    assert!(
        screen.contains(&format!("-S {}", rig.socket.display())),
        "recorded server: {screen}"
    );
    for link in [
        "https://github.com/clemens33/ae",
        "https://github.com/clemens33/ae/releases",
        "https://github.com/clemens33/ae/blob/main/docs/app.md",
    ] {
        assert!(
            screen.lines().any(|line| line.trim().ends_with(link)),
            "plain link has its own row {link}: {screen}"
        );
    }
    assert!(
        !has_colour_sgr(&rig.styled(&pane)),
        "theme off gained colour"
    );
    assert!(
        reversed_text(rig.styled(&pane).lines().nth(1).unwrap_or_default()).contains("About"),
        "selected tab remains visible without colour"
    );
}
