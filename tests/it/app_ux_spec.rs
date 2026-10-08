//! Frozen appux acceptance. Oracles: brief-appux R1/R2, fixture bytes and
//! captured terminal cells. Compiles on 567ac981; no implementation stubs,
//! private Settings API, real harness stores or new process doors.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ae::app::draw::{Composer, Screen, draw};
use ae::app::fleet::{Counts, Fleet, Line2, Row, attention};
use ae::app::model::Model;
use ae::app::overview::Overview;
use ae::console::lane::Lane;
use ae::theme::{Look, Mark};
use ae::time::Timestamp;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

fn fleet(needy: &[usize]) -> Fleet {
    Fleet {
        rows: (0..10)
            .map(|at| Row {
                name: format!("session-{at}"),
                index: at + 1,
                mark: if at % 2 == 0 {
                    Mark::Dead
                } else {
                    Mark::NeedsYou
                },
                needy: needy.contains(&at),
                counts: Counts::Marks(Vec::new()),
                line2: Line2::NoGoal,
                home: false,
            })
            .collect(),
        home: None,
    }
}

#[test]
fn attention_quiet_keeps_the_sentence() {
    assert_eq!(attention(&fleet(&[]), 2..7, 1), "Nothing needs you.");
}

#[test]
fn attention_one_visible_names_it_without_a_mark() {
    assert_eq!(attention(&fleet(&[2]), 2..3, 44), "session-2 needs you");
}

#[test]
fn attention_one_above_uses_a_plain_location() {
    assert_eq!(
        attention(&fleet(&[2]), 3..7, 44),
        "session-2 needs you · above"
    );
}

#[test]
fn attention_one_at_the_exclusive_end_is_below() {
    assert_eq!(
        attention(&fleet(&[2]), 0..2, 44),
        "session-2 needs you · below"
    );
}

#[test]
fn attention_many_counts_only_needy_rows_above_and_below() {
    let fleet = fleet(&[0, 2, 3, 5, 7, 8, 9]);
    for (visible, expected) in [
        (0..10, "7 need you"),
        (0..7, "7 need you · 3 below"),
        (2..10, "7 need you · 1 above"),
        (2..7, "7 need you · 1 above · 3 below"),
        (0..0, "7 need you · 7 below"),
    ] {
        for width in [1, 20, 44, 160] {
            assert_eq!(attention(&fleet, visible.clone(), width), expected);
        }
    }
    assert_eq!(attention(&self::fleet(&[2, 3]), 2..4, 44), "2 need you");
}

fn painted(fleet: &Fleet, look: &Look) -> Buffer {
    let model = Model::new(fleet);
    let overview = Overview::default();
    let lane = Lane::default();
    let screen = Screen {
        fleet,
        model: &model,
        overview: &overview,
        selected: None,
        pair: &[],
        agents: None,
        lane: &lane,
        composer: Composer::NoHome,
        look: Some(*look),
        zone: None,
        now: Timestamp::from_epoch(1_791_200_000),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 43));
    let _ = draw(&screen, &mut buf);
    buf
}

fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

#[test]
fn attention_draw_keeps_session_count_and_next_need_cell_in_both_looks() {
    for mode in ["on", "off"] {
        let buf = painted(
            &fleet(&[0, 1, 2, 3, 4, 5, 6]),
            &Look::read(mode, "", mode, "off"),
        );
        assert!(row(&buf, 1).starts_with("  Sessions 10"));
        assert!(row(&buf, 3).starts_with("  7 need you  "));
        assert_eq!(buf[(40, 3)].symbol(), " ", "blank before the key");
        assert_eq!(buf[(41, 3)].symbol(), "!");
    }
}

#[test]
fn attention_draw_still_clips_a_single_long_name_before_next_need() {
    let mut fleet = fleet(&[0]);
    fleet.rows[0].name = "n".repeat(80);
    let buf = painted(&fleet, &Look::read("on", "", "off", "off"));
    assert_eq!(&row(&buf, 3)[2..40], "n".repeat(38));
    assert_eq!(buf[(40, 3)].symbol(), " ");
    assert_eq!(buf[(41, 3)].symbol(), "!");
}

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const TID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);

struct Rig {
    tool: super::deliver::Rig,
    root: PathBuf,
    socket: PathBuf,
    home: String,
}

impl Rig {
    fn new(tag: &str, mode: &str) -> Self {
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
        let store = root.join("claude-home");
        super::board::plant_transcript(
            &store,
            "work",
            TID,
            &[super::board::user(
                "2026-10-07T10:00:00Z",
                "appux-fixture-lane",
            )],
        );
        fs::write(tool.dir.join("meta"), format!(
            "schema=2\nsession={home}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=claude\nlaunch_id.main=tok-appux\nharness_session.main={TID}\nconfig_home.main={}\nseat.worker.0=colead\nagent_bin.worker.0=claude\nlaunch_id.worker.0=tok-colead\nconfig_home.worker.0={}\n",
            socket.display(), store.display(), store.display(),
        )).expect("fixture meta");
        fs::write(root.join("config"), "[workspace]\nchat = app\n").expect("fixture config");
        let rig = Self {
            tool,
            root,
            socket,
            home,
        };
        for (key, value) in [
            ("@ae_look", mode),
            ("@ae_icons", mode),
            ("@ae_motion", "off"),
            ("@ae_session_uuid", UUID),
            ("@ae_main_pane", rig.tool.pane.as_str()),
        ] {
            rig.tmux(&["set-option", "-t", &rig.home, key, value]);
        }
        rig
    }

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &self.root);
        assert!(ok, "private tmux {tail:?}: {out}");
        out
    }

    fn open(&self, width: u16, height: u16) -> String {
        // The private server already exists; every app child additionally sees
        // a private HOME/TMUX_TMPDIR and no inherited harness-home override.
        let command = format!(
            "env -u CLAUDE_CONFIG_DIR -u CODEX_HOME {} app {}",
            quote(env!("CARGO_BIN_EXE_ae")),
            quote(&self.home),
        );
        let env = [
            format!("HOME={}", self.root.display()),
            format!("AE_HOME={}", self.root.display()),
            format!("TMUX_TMPDIR={}", self.root.display()),
            format!("CONFIG_FILE={}", self.root.join("config").display()),
            "AE_TMUX_SERVER_KIND=socket".to_owned(),
            format!("AE_TMUX_SERVER={}", self.socket.display()),
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
        self.resize(&pane, width, height);
        self.wait(&pane, WAIT, "fixture app drawn", |s| {
            s.contains("appux-fixture-lane")
        });
        pane
    }

    fn resize(&self, pane: &str, width: u16, height: u16) {
        self.tmux(&[
            "resize-window",
            "-t",
            pane,
            "-x",
            &width.to_string(),
            "-y",
            &height.to_string(),
        ]);
    }

    fn screen(&self, pane: &str) -> String {
        self.tmux(&["capture-pane", "-p", "-t", pane])
    }

    fn wait(&self, pane: &str, within: Duration, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + within;
        let mut previous = None;
        loop {
            let screen = self.screen(pane);
            if met(&screen) && previous.as_deref() == Some(screen.as_str()) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}; terminal:\n{screen}");
            previous = Some(screen);
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn keys(&self, pane: &str, key: &str) {
        self.tmux(&["send-keys", "-t", pane, key]);
    }

    fn literal(&self, pane: &str, text: &str) {
        self.tmux(&["send-keys", "-t", pane, "-l", "--", text]);
    }

    fn click(&self, pane: &str, x: usize, y: usize) {
        self.literal(pane, &format!("\x1b[<0;{};{}M", x + 1, y + 1));
    }

    fn gear(&self, pane: &str) {
        let screen = self.screen(pane);
        let (y, line) = screen.lines().enumerate().last().expect("drawn keys row");
        let x = line
            .chars()
            .position(|ch| ch == '*' || ch == '⚙')
            .expect("drawn gear");
        self.click(pane, x, y);
    }

    fn settings(&self, pane: &str) -> String {
        self.keys(pane, "s");
        self.wait(pane, WAIT, "settings opened", is_settings)
    }

    fn tab(&self, pane: &str, name: &str, marker: &str) -> String {
        let screen = self.screen(pane);
        let (x, y) = target_in(&screen, 1, name);
        // A title's last cell must work too, not only its first column.
        self.click(pane, x + name.chars().count() - 1, y);
        self.wait(pane, WAIT, "clicked tab body drawn", |s| {
            is_settings(s) && s.lines().skip(3).any(|line| line.contains(marker))
        })
    }

    fn writer_held(&self) {
        let file = fs::OpenOptions::new()
            .append(true)
            .open(self.tool.dir.join(".console-writer.lock"))
            .expect("writer lease exists");
        assert!(
            matches!(file.try_lock(), Err(fs::TryLockError::WouldBlock)),
            "Settings keeps the actual writing lease (R-B5)"
        );
    }

    /// A later visible wheel effect proves earlier presses were processed.
    /// docs/app.md: one Settings wheel notch moves three body rows. The
    /// fourth row of the already drawn Config body is the independent oracle.
    fn swallowed_then_wheel(&self, pane: &str, presses: &[(usize, usize)]) -> String {
        let before = self.screen(pane);
        let top = before.lines().nth(3).expect("drawn Config top");
        assert!(
            top.contains("session:"),
            "Config starts at its top: {before}"
        );
        let expected = before.lines().nth(6).expect("fourth Config row");
        for (x, y) in presses {
            self.click(pane, *x, *y);
        }
        self.literal(pane, "\x1b[<65;2;5M");
        let after = self.wait(
            pane,
            WAIT,
            "later wheel effect orders earlier clicks",
            |s| s.lines().nth(3) != Some(top),
        );
        assert!(
            is_settings(&after),
            "earlier presses closed overlay: {after}"
        );
        assert_eq!(
            after.lines().nth(3),
            Some(expected),
            "only the wheel moves scroll"
        );
        assert!(
            after
                .lines()
                .nth(3)
                .is_some_and(|line| line.trim_start().starts_with("workers = ")),
            "earlier presses changed the Config tab: {after}"
        );
        after
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
                .all(|name| line.contains(name))
        })
}

fn target_in(screen: &str, row: usize, label: &str) -> (usize, usize) {
    let line = screen.lines().nth(row).expect("drawn target row");
    let byte = line.find(label).expect("drawn target label");
    (line[..byte].chars().count(), row)
}

fn close_target(screen: &str) -> (usize, usize) {
    let last = screen.lines().count().saturating_sub(1);
    let y = [0, last]
        .into_iter()
        .find(|y| {
            screen
                .lines()
                .nth(*y)
                .is_some_and(|line| line.contains("close"))
        })
        .expect("drawn close affordance in title or hint row");
    target_in(screen, y, "close")
}

#[test]
fn settings_guard_keys_wheel_gear_and_writer_work_on_the_base() {
    let rig = Rig::new("uxguard", "off");
    let pane = rig.open(160, 12);
    rig.settings(&pane);
    rig.keys(&pane, "2");
    let first = rig.wait(&pane, WAIT, "keyboard config ready", |s| {
        s.contains("session:")
    });
    let top = first.lines().nth(3).expect("first body row").to_owned();
    rig.swallowed_then_wheel(&pane, &[(2, 2)]);
    rig.literal(&pane, "\x1b[<64;2;5M");
    rig.wait(&pane, WAIT, "wheel returns body", |s| {
        s.lines().nth(3) == Some(top.as_str())
    });
    rig.gear(&pane);
    rig.wait(&pane, WAIT, "gear closes on base", |s| !is_settings(s));
    rig.keys(&pane, "Enter");
    rig.literal(&pane, "guard-draft");
    rig.wait(&pane, WAIT, "writing ready on base", |s| {
        s.contains("guard-draft")
    });
    rig.gear(&pane);
    rig.wait(&pane, WAIT, "gear opens on base", is_settings);
    rig.writer_held();
    rig.keys(&pane, "Escape");
    rig.wait(&pane, WAIT, "Esc resumes draft on base", |s| {
        !is_settings(s) && s.contains("guard-draft")
    });
    rig.writer_held();
}

#[test]
fn settings_mouse_tabs_show_their_bodies_with_icons_and_theme_on_or_off() {
    for mode in ["off", "on"] {
        let rig = Rig::new(&format!("uxtabs{mode}"), mode);
        let pane = rig.open(160, 30);
        rig.settings(&pane);
        for (name, marker) in [
            ("About", "tmux"),
            ("Config", "session:"),
            ("Quota", "no quota scopes read"),
        ] {
            rig.tab(&pane, name, marker);
        }
        rig.resize(&pane, 40, 8);
        rig.wait(&pane, WAIT, "minimum tab frame drawn", |s| {
            is_settings(s) && s.lines().count() == 8
        });
        rig.tab(&pane, "Config", "session:");
        rig.tab(&pane, "About", "tmux");
    }
}

#[test]
fn settings_mouse_switch_and_same_tab_click_reset_scroll_like_keys() {
    let rig = Rig::new("uxscroll", "off");
    let pane = rig.open(160, 12);
    rig.settings(&pane);
    rig.keys(&pane, "2");
    let first = rig.wait(&pane, WAIT, "config body ready", |s| s.contains("session:"));
    let top = first.lines().nth(3).expect("first body row").to_owned();
    for switch_away in [true, false] {
        rig.literal(&pane, "\x1b[<65;2;5M");
        rig.wait(&pane, WAIT, "wheel scrolls overlay", |s| {
            s.lines().nth(3) != Some(top.as_str())
        });
        if switch_away {
            rig.tab(&pane, "About", "tmux");
        }
        rig.tab(&pane, "Config", "session:");
        rig.wait(&pane, WAIT, "click reset config scroll", |s| {
            s.lines().nth(3) == Some(top.as_str())
        });
    }
}

#[test]
fn settings_close_label_uses_the_resized_drawn_cells_even_at_the_minimum() {
    let rig = Rig::new("uxclose", "off");
    let pane = rig.open(160, 30);
    rig.settings(&pane);
    rig.keys(&pane, "2");
    rig.wait(&pane, WAIT, "Config ready before resizing", |s| {
        s.contains("session:")
    });
    rig.resize(&pane, 40, 8);
    let screen = rig.wait(&pane, WAIT, "minimum overlay redraw", |s| {
        is_settings(s)
            && s.lines().count() == 8
            && s.lines()
                .next()
                .is_some_and(|line| line.chars().count() <= 40)
    });
    assert!(is_settings(&screen));
    let (x, y) = close_target(&screen);
    rig.swallowed_then_wheel(&pane, &[(x, 2)]);
    rig.click(&pane, x + "close".len() - 1, y);
    rig.wait(&pane, WAIT, "drawn close label closes", |s| !is_settings(s));
    rig.resize(&pane, 160, 30);
    rig.wait(&pane, WAIT, "home frame resized back", |s| {
        s.contains("appux-fixture-lane")
    });
    rig.settings(&pane);
    rig.gear(&pane);
    rig.wait(&pane, WAIT, "gear still closes", |s| !is_settings(s));
}

#[test]
fn settings_mouse_tabs_and_close_preserve_the_writing_draft() {
    let rig = Rig::new("uxdraft", "off");
    let pane = rig.open(160, 12);
    rig.keys(&pane, "Enter");
    rig.literal(&pane, "draft-before");
    rig.wait(&pane, WAIT, "draft is being written", |s| {
        s.contains("draft-before") && s.lines().last().is_some_and(|line| line.contains("write"))
    });
    rig.gear(&pane);
    rig.wait(&pane, WAIT, "gear opens during writing", is_settings);
    rig.writer_held();
    rig.tab(&pane, "About", "tmux");
    rig.tab(&pane, "Config", "session:");
    rig.writer_held();
    rig.literal(&pane, "hidden-text");
    // Click the rule and a tab's following blank: neither is an affordance.
    let screen = rig.screen(&pane);
    let (x, y) = target_in(&screen, 1, "About");
    let screen = rig.swallowed_then_wheel(&pane, &[(x + "About".len(), y), (2, 2)]);
    let (x, y) = close_target(&screen);
    rig.click(&pane, x, y);
    rig.wait(&pane, WAIT, "close resumes writing", |s| {
        !is_settings(s)
            && s.contains("draft-before")
            && s.lines().last().is_some_and(|line| line.contains("write"))
    });
    rig.writer_held();
    rig.literal(&pane, "-after");
    let screen = rig.wait(&pane, WAIT, "same composer continues", |s| {
        s.contains("draft-before-after")
    });
    assert!(
        !screen.contains("hidden-text"),
        "overlay input changed draft: {screen}"
    );
}
