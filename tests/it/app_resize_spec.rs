//! appresize frozen acceptance: requirements + the DRAWN frame are the oracle.
//! S0 9a4142eb only names Drag/Release; it never emits them. Live tests drive
//! actual App dispatch, composer and hit-testing, not a parallel test reducer.
//! Raw pane reports prove the terminal path; a nested client separately proves
//! stock tmux forwarding with session mouse on. All stores/sockets are fixtures.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns isolated terminal fixtures and reads their effects"
)]

use crate::app_mode::writing;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ae::console::input::{Key, Keys, Mouse, MouseKind};

fn feed(keys: &mut Keys, bytes: &[u8]) -> Vec<Key> {
    keys.feed(bytes, Instant::now())
        .into_iter()
        .map(|(key, _)| key)
        .collect()
}

fn mouse(kind: MouseKind, column: u16, row: u16) -> Key {
    Key::Mouse(Mouse { kind, column, row })
}

#[test]
fn left_motion_is_admitted_at_zero_based_cell() {
    assert_eq!(
        feed(&mut Keys::app(), b"\x1b[<32;67;8M"),
        [mouse(MouseKind::Drag, 66, 7)]
    );
}

#[test]
fn left_release_is_admitted_at_zero_based_cell() {
    assert_eq!(
        feed(&mut Keys::app(), b"\x1b[<0;67;8m"),
        [mouse(MouseKind::Release, 66, 7)]
    );
}

#[test]
fn drag_and_release_survive_every_read_boundary_and_keep_origin() {
    let bytes = b"\x1b[<32;67;8M\x1b[<0;67;8m";
    let first = Instant::now();
    let second = first + Duration::from_millis(1);
    for split in 0..=bytes.len() {
        let mut keys = Keys::app();
        let mut got = keys.feed(&bytes[..split], first);
        got.extend(keys.feed(&bytes[split..], second));
        assert_eq!(
            got.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>(),
            [
                mouse(MouseKind::Drag, 66, 7),
                mouse(MouseKind::Release, 66, 7)
            ],
            "split {split}"
        );
        assert_eq!(got[0].1, if split == 0 { second } else { first });
        assert_eq!(got[1].1, if split <= 11 { second } else { first });
    }
}

/// Exhaustive SGR button-byte table: modifiers, non-left motion, any-motion,
/// other buttons and non-left releases remain dropped; next click survives.
#[test]
fn only_the_five_unmodified_sgr_reports_are_admitted() {
    for code in 0..=127 {
        for suffix in ['M', 'm'] {
            let allowed = matches!((code, suffix), (0 | 32 | 64 | 65, 'M') | (0, 'm'));
            if allowed {
                continue;
            }
            let bytes = format!("\x1b[<{code};5;5{suffix}\x1b[<0;9;9M");
            assert_eq!(
                feed(&mut Keys::app(), bytes.as_bytes()),
                [mouse(MouseKind::Click, 8, 8)],
                "code {code}, suffix {suffix}"
            );
        }
    }
}

#[test]
fn chat_still_emits_no_mouse_key_and_x10_still_drops_whole() {
    let bytes = b"\x1b[<0;45;8M\x1b[<32;67;8M\x1b[<0;67;8m";
    for split in 0..=bytes.len() {
        let mut chat = Keys::default();
        let mut got = feed(&mut chat, &bytes[..split]);
        got.extend(feed(&mut chat, &bytes[split..]));
        assert!(got.iter().all(|key| !matches!(key, Key::Mouse(_))));
    }
    assert_eq!(
        feed(&mut Keys::app(), b"\x1b[M \x1b!q"),
        [Key::Text(b"q".to_vec())]
    );
}

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);
const MOVE: Duration = Duration::from_secs(3);

/// One lead-pair session on a private server whose chat window is `ae app`,
/// shaped like `app_live::Rig`: its own root, socket and config, a fixture
/// Claude store for a scrollable lane, and room for stopped sibling rows.
struct Rig {
    tool: super::deliver::Rig,
    root: PathBuf,
    socket: PathBuf,
    name: String,
}

impl Rig {
    fn styled(&self, pane: &str, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + MOVE;
        loop {
            let screen = self.tmux(&["capture-pane", "-e", "-p", "-t", pane]);
            if met(&screen) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}; styled screen:\n{screen}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// The session; `turns` user turns in its fixture transcript, `stopped`
    /// extra stopped rows beside it.
    fn new(tag: &str, turns: usize, stopped: &[&str]) -> Self {
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
        let tid = "0199c0de-aaaa-4890-abcd-ef0123456789";
        let lines: Vec<String> = (0..turns)
            .map(|at| {
                let marker = if at == 0 {
                    "scrollprobe-oldest"
                } else if at + 1 == turns {
                    "scrollprobe-newest"
                } else {
                    "scrollprobe-filler"
                };
                let body = format!(
                    "{marker}-{at:02} {}",
                    "lorem ipsum dolor sit amet ".repeat(10)
                );
                super::board::user(&format!("2026-10-04T10:00:{at:02}Z"), &body)
            })
            .collect();
        super::board::plant_transcript(&store, "work", tid, &lines);
        std::fs::write(tool.dir.join("meta"), format!(
            "session={}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=claude\nlaunch_id.main=tok-rig\nharness_session.main={tid}\nconfig_home.main={}\nseat.worker.0=colead\nagent_bin.worker.0=claude\nlaunch_id.worker.0=tok-colead\nconfig_home.worker.0={}\n",
            name, socket.display(), store.display(), store.display(),
        )).expect("lead pair meta");
        for sibling in stopped {
            let dir = root.join("sessions").join(sibling);
            std::fs::create_dir_all(&dir).expect("stopped dir");
            std::fs::write(dir.join("meta"), format!(
                "session={sibling}\nmode=local\nsession_id=0199c0de-bbbb-4890-abcd-ef0123456789\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=claude\nlaunch_id.main=tok-stop\n",
                socket.display(),
            )).expect("stopped meta");
        }
        std::fs::write(root.join("config"), "[workspace]\nchat = app\n").expect("chat = app");
        let rig = Self {
            tool,
            root,
            socket,
            name,
        };
        rig.tmux(&["set-option", "-t", &rig.name, "@ae_look", "off"]);
        for (option, value) in [
            ("@ae_session_uuid", UUID),
            ("@ae_motion", "off"),
            ("@ae_main_pane", rig.tool.pane.as_str()),
            ("default-size", "160x45"),
        ] {
            rig.tmux(&["set-option", "-t", &rig.name, option, value]);
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

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &self.root);
        assert!(ok, "private tmux {tail:?}: {out}");
        out
    }

    /// Open the chat window, which `chat = app` makes the app: its pane. The
    /// app sees the fixture root as HOME and no harness config overrides, so
    /// no real transcript store is ever read.
    fn open_app(&self) -> String {
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
            .expect("the chat window opens");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        self.tmux(&[
            "list-panes",
            "-s",
            "-t",
            &self.name,
            "-F",
            "#{pane_id}|#{@ae_console}",
        ])
        .lines()
        .find_map(|row| {
            let (pane, stamp) = row.split_once('|')?;
            (stamp == UUID).then(|| pane.to_owned())
        })
        .expect("the stamped app window")
    }

    fn screen(&self, pane: &str) -> String {
        self.tmux(&["capture-pane", "-p", "-t", pane])
    }

    /// Two identical captures once `met` holds, within `within`. The backend
    /// streams cells, so one capture can contain only part of a repaint.
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

    /// A plain left press at the 0-based cell, as literal SGR bytes.
    fn click(&self, pane: &str, col0: usize, row0: usize) {
        let seq = format!("\x1b[<0;{};{}M", col0 + 1, row0 + 1);
        self.tmux(&["send-keys", "-t", pane, "-l", "--", &seq]);
    }

    /// One wheel notch at the 0-based cell, as literal SGR bytes.
    fn wheel(&self, pane: &str, up: bool, col0: usize, row0: usize) {
        let code = if up { 64 } else { 65 };
        let seq = format!("\x1b[<{};{};{}M", code, col0 + 1, row0 + 1);
        self.tmux(&["send-keys", "-t", pane, "-l", "--", &seq]);
    }
}

impl Rig {
    fn send(&self, pane: &str, bytes: &str) {
        self.tmux(&["send-keys", "-t", pane, "-l", "--", bytes]);
    }

    fn report(&self, pane: &str, code: u8, col: usize, row: usize, suffix: char) {
        self.send(
            pane,
            &format!("\x1b[<{code};{};{}{suffix}", col + 1, row + 1),
        );
    }

    fn motion(&self, pane: &str, col: usize, row: usize) {
        self.report(pane, 32, col, row, 'M');
    }

    fn release(&self, pane: &str, col: usize, row: usize) {
        self.report(pane, 0, col, row, 'm');
    }

    fn resize(&self, pane: &str, width: usize, height: usize) {
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

    fn ready(&self, pane: &str) -> String {
        self.wait(
            pane,
            WAIT,
            "fixture loaded and home owns composer",
            |screen| screen.contains("Overview") && screen.contains("Enter writes"),
        )
    }

    fn vertical(&self, pane: &str, column: usize) -> String {
        self.wait(
            pane,
            MOVE,
            &format!("vertical rule moved to {column}"),
            |screen| vertical_rule(screen) == Some(column),
        )
    }

    fn horizontal(&self, pane: &str, row: usize) -> String {
        self.wait(
            pane,
            MOVE,
            &format!("horizontal rule moved to {row}"),
            |screen| horizontal_rule(screen) == Some(row),
        )
    }

    fn grab_vertical(&self, pane: &str, column: usize) {
        let screen = self.screen(pane);
        self.click(
            pane,
            vertical_rule(&screen).expect("drawn vertical rule"),
            1,
        );
        self.motion(pane, column, 1);
    }

    fn grab_horizontal(&self, pane: &str, row: usize) {
        let screen = self.screen(pane);
        self.click(
            pane,
            1,
            horizontal_rule(&screen).expect("drawn horizontal rule"),
        );
        self.motion(pane, 1, row);
    }
}

fn vertical_rule(screen: &str) -> Option<usize> {
    screen.lines().next()?.chars().position(|cell| cell == '│')
}

fn horizontal_rule(screen: &str) -> Option<usize> {
    screen.lines().position(|row| row.starts_with("───"))
}

fn cell_of(screen: &str, needle: &str) -> Option<(usize, usize)> {
    screen.lines().enumerate().find_map(|(row, line)| {
        line.chars()
            .collect::<Vec<_>>()
            .windows(needle.chars().count())
            .position(|cells| cells.iter().collect::<String>() == needle)
            .map(|col| (col, row))
    })
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

const STOPPED: &[&str] = &[
    "zstop00", "zstop01", "zstop02", "zstop03", "zstop04", "zstop05", "zstop06", "zstop07",
    "zstop08", "zstop09", "zstop10", "zstop11",
];

#[test]
fn live_vertical_drag_moves_chat_and_release_ends_it() {
    let rig = Rig::new("arvert", 0, &[]);
    let pane = rig.open_app();
    let screen = rig.ready(&pane);
    assert_eq!(vertical_rule(&screen), Some(44), "unchanged wide default");
    rig.grab_vertical(&pane, 66);
    let screen = rig.vertical(&pane, 66);
    let (col, _) = cell_of(&screen, &format!("to {}", rig.name)).expect("home composer");
    assert_eq!(col, 69, "chat begins at rule+1, then its two-cell inset");
    assert!(
        screen.contains("Enter writes"),
        "grab selects/writes nothing"
    );
    rig.release(&pane, 66, 1);
    rig.motion(&pane, 76, 1);
    rig.send(&pane, "\t");
    let screen = rig.wait(
        &pane,
        MOVE,
        "ordered Tab sentinel processed after orphan motion",
        |screen| screen.contains("No seat facts"),
    );
    assert_eq!(vertical_rule(&screen), Some(66), "release ends drag");
}

#[test]
fn live_vertical_drag_clamps_both_ends_and_keeps_rows_legible() {
    let rig = Rig::new("arclamp", 0, STOPPED);
    let pane = rig.open_app();
    rig.ready(&pane);
    rig.grab_vertical(&pane, 159);
    rig.vertical(&pane, 119); // 160 - rule - one rule cell = 40 chat columns.
    rig.motion(&pane, 0, 1);
    let screen = rig.vertical(&pane, 30); // plan's chosen minimum, >= brief's 28.
    assert!(screen.contains("Overview") && screen.contains("Agents"));
    assert!(screen.contains("more"), "list elision still fits");
    assert!(
        screen.contains("Enter writes"),
        "grab leaves selection unchanged"
    );
    rig.release(&pane, 0, 1);
}

#[test]
fn live_horizontal_drag_reserves_blank_rows_then_clamps() {
    let rig = Rig::new("arblank", 0, &[]);
    let pane = rig.open_app();
    assert_eq!(
        horizontal_rule(&rig.ready(&pane)),
        Some(9),
        "unchanged one-row default"
    );
    rig.grab_horizontal(&pane, 20);
    let screen = rig.horizontal(&pane, 20);
    let (_, tab) = cell_of(&screen, "Overview").expect("tabs stay above rule");
    assert_eq!(tab, 19);
    assert!(cell_of(&screen, "Goal").is_some_and(|(_, row)| row > 20));
    rig.motion(&pane, 1, 44);
    rig.horizontal(&pane, 39); // floor43; body40,41,42 = three rows.
    rig.motion(&pane, 1, 0);
    rig.horizontal(&pane, 9); // one session needs two list rows, no more row.
    rig.release(&pane, 1, 0);
}

#[test]
fn live_horizontal_minimum_keeps_one_session_and_more_row() {
    let rig = Rig::new("arlist", 0, STOPPED);
    let pane = rig.open_app();
    let screen = rig.ready(&pane);
    assert_eq!(
        horizontal_rule(&screen),
        Some(30),
        "larger undragged windowed default"
    );
    rig.grab_horizontal(&pane, 0);
    let screen = rig.horizontal(&pane, 10);
    assert!(
        screen
            .lines()
            .nth(5)
            .is_some_and(|row| row.contains(&rig.name))
    );
    assert!(
        screen
            .lines()
            .nth(7)
            .is_some_and(|row| row.contains("more"))
    );
    assert!(screen.contains("Goal"), "tab body remains usable");
    rig.release(&pane, 1, 0);
}

/// A zero-distance drag cannot make a default edge jump. Both existing
/// content-sized defaults remain usable minima (lead ruling 5a).
#[test]
fn live_zero_distance_grab_keeps_both_default_rules_for_one_and_many_sessions() {
    for stopped in [&[][..], STOPPED] {
        let rig = Rig::new(
            if stopped.is_empty() {
                "arstill1"
            } else {
                "arstill13"
            },
            0,
            stopped,
        );
        let pane = rig.open_app();
        let before = rig.ready(&pane);
        let vertical = vertical_rule(&before).expect("drawn vertical rule");
        let horizontal = horizontal_rule(&before).expect("drawn horizontal rule");
        rig.click(&pane, vertical, 1);
        rig.motion(&pane, vertical, 1);
        rig.send(&pane, "\t");
        let after = rig.wait(
            &pane,
            MOVE,
            "ordered Tab after zero-distance vertical motion",
            |screen| screen.contains("No seat facts"),
        );
        assert_eq!(vertical_rule(&after), Some(vertical));
        assert_eq!(horizontal_rule(&after), Some(horizontal));
        rig.click(&pane, 1, horizontal);
        rig.motion(&pane, 1, horizontal);
        rig.send(&pane, "\t");
        let after = rig.wait(
            &pane,
            MOVE,
            "ordered Tab after zero-distance horizontal motion",
            |screen| screen.contains("Goal"),
        );
        assert_eq!(vertical_rule(&after), Some(vertical));
        assert_eq!(horizontal_rule(&after), Some(horizontal));
    }
}

#[test]
fn live_chosen_sizes_survive_shrink_hidden_sidebar_and_growth() {
    let rig = Rig::new("arkeep", 0, &[]);
    let pane = rig.open_app();
    rig.ready(&pane);
    rig.grab_horizontal(&pane, 30);
    rig.horizontal(&pane, 30);
    rig.release(&pane, 1, 30);
    rig.grab_vertical(&pane, 66);
    rig.vertical(&pane, 66);
    // Resize is not input: active drag continues, display clamps only.
    rig.resize(&pane, 90, 20);
    let screen = rig.vertical(&pane, 49);
    assert_eq!(horizontal_rule(&screen), Some(14));
    rig.resize(&pane, 160, 45);
    let screen = rig.vertical(&pane, 66);
    assert_eq!(horizontal_rule(&screen), Some(30));
    rig.motion(&pane, 70, 1);
    rig.vertical(&pane, 70); // active drag survived resize.
    rig.resize(&pane, 80, 20);
    rig.wait(&pane, WAIT, "sidebar hidden below boundary", |screen| {
        screen.contains("sidebar needs")
    });
    rig.motion(&pane, 1, 1); // invisible edge must not overwrite chosen width.
    rig.send(&pane, "i");
    rig.wait(
        &pane,
        MOVE,
        "ordered writing sentinel after invisible-edge motion",
        writing,
    );
    rig.resize(&pane, 160, 45);
    let screen = rig.vertical(&pane, 70);
    assert_eq!(horizontal_rule(&screen), Some(30));
}

#[test]
fn live_lost_release_then_click_selects_row_and_old_drag_stays_dead() {
    let rig = Rig::new("arlost", 0, &["zstop"]);
    let pane = rig.open_app();
    rig.ready(&pane);
    rig.grab_vertical(&pane, 66);
    let screen = rig.vertical(&pane, 66);
    let (_, row) = cell_of(&screen, "zstop").expect("drawn session row");
    // Within one cell of rule, but row target wins over grab tolerance.
    rig.click(&pane, 65, row);
    rig.wait(
        &pane,
        MOVE,
        "ordinary click selects stopped row",
        |screen| screen.contains("zstop is stopped"),
    );
    rig.motion(&pane, 80, row);
    // Tab-label click is both unchanged click behavior and ordered sentinel.
    let (col, row) = cell_of(&screen, "Agents").expect("tab label");
    rig.click(&pane, col, row);
    let screen = rig.wait(
        &pane,
        MOVE,
        "tab click processed after orphan motion",
        |screen| screen.contains("Not running: no seat facts."),
    );
    assert_eq!(vertical_rule(&screen), Some(66));
    assert!(screen.contains("zstop is stopped"));
}

#[test]
fn live_key_and_paste_cancel_drag_keep_size_and_act_as_before() {
    let rig = Rig::new("arkey", 0, &[]);
    let pane = rig.open_app();
    rig.ready(&pane);
    rig.grab_vertical(&pane, 66);
    rig.vertical(&pane, 66);
    rig.send(&pane, "i");
    rig.wait(&pane, MOVE, "key enters writing", writing);
    rig.motion(&pane, 80, 1);
    rig.send(&pane, "key-sentinel");
    let screen = rig.wait(
        &pane,
        MOVE,
        "typed sentinel processed after orphan motion",
        |screen| screen.contains("key-sentinel"),
    );
    assert_eq!(vertical_rule(&screen), Some(66), "key ended lost drag");
    rig.grab_vertical(&pane, 60);
    rig.vertical(&pane, 60);
    rig.send(&pane, "\x1b[200~paste-kept\x1b[201~");
    rig.wait(&pane, MOVE, "paste still composes", |screen| {
        screen.contains("paste-kept")
    });
    rig.motion(&pane, 75, 1);
    rig.send(&pane, "!");
    let screen = rig.wait(
        &pane,
        MOVE,
        "typed sentinel processed after paste and orphan motion",
        |screen| screen.contains("paste-kept!"),
    );
    assert_eq!(vertical_rule(&screen), Some(60), "paste ended lost drag");
    assert!(writing(&screen) && screen.contains("paste-kept"));
}

#[test]
fn live_wheel_cancels_drag_and_chat_wheel_still_scrolls() {
    let rig = Rig::new("arwheel", 12, &[]);
    let pane = rig.open_app();
    rig.ready(&pane);
    rig.wait(&pane, WAIT, "transcript loaded", |screen| {
        screen.contains("scrollprobe-newest")
    });
    rig.grab_vertical(&pane, 66);
    rig.vertical(&pane, 66);
    rig.wheel(&pane, true, 2, 1); // sidebar wheel ends drag, otherwise no-op.
    rig.motion(&pane, 80, 1);
    rig.send(&pane, "\t");
    let screen = rig.wait(
        &pane,
        MOVE,
        "Tab sentinel processed after wheel and orphan motion",
        |screen| screen.contains("No seat facts"),
    );
    assert_eq!(vertical_rule(&screen), Some(66));
    assert!(!screen.contains("newer turns below"));
    for _ in 0..8 {
        rig.wheel(&pane, true, 75, 20);
    }
    rig.wait(&pane, MOVE, "chat wheel unchanged", |screen| {
        screen.contains("newer turns below")
    });
    for _ in 0..8 {
        rig.wheel(&pane, false, 75, 20);
    }
    rig.wait(&pane, MOVE, "wheel returns to newest", |screen| {
        screen.contains("scrollprobe-newest") && !screen.contains("newer turns below")
    });
}

#[test]
fn live_drag_while_composing_keeps_draft_and_wrap_tracks_width() {
    let rig = Rig::new("arwrap", 0, &[]);
    let pane = rig.open_app();
    rig.ready(&pane);
    rig.resize(&pane, 100, 30);
    rig.vertical(&pane, 34);
    rig.send(&pane, "i");
    rig.wait(&pane, MOVE, "writing started", writing);
    rig.send(&pane, &format!("{}wrap-end", "a".repeat(80)));
    let before = rig.wait(&pane, MOVE, "long draft drawn", |screen| {
        screen.contains("wrap-end")
    });
    let address = format!("to {} › lead   ", rig.name);
    let composer = |screen: &str| {
        let (top, column) = screen
            .lines()
            .enumerate()
            .find_map(|(row, line)| {
                line.find(&address)
                    .map(|at| (row, line[..at].chars().count() + address.chars().count()))
            })
            .expect("drawn composer address");
        let rows = screen
            .lines()
            .skip(top)
            .take(27 - top)
            .map(|row| {
                row.chars()
                    .skip(column)
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>();
        (top, column, rows)
    };
    assert_eq!(
        composer(&before),
        (
            24,
            58,
            vec!["a".repeat(40), "a".repeat(40), "wrap-end".to_owned()]
        ),
        "whole draft: 80 cells wrap exactly twice at 40 cells"
    );
    assert_eq!(
        rig.tmux(&[
            "display-message",
            "-p",
            "-t",
            &pane,
            "#{cursor_flag},#{cursor_x},#{cursor_y}"
        ])
        .trim(),
        "1,66,26"
    );
    rig.grab_vertical(&pane, 56);
    let after = rig.vertical(&pane, 56);
    assert!(writing(&after), "drag keeps composing active");
    assert!(after.contains("wrap-end"), "draft kept");
    // Whole draft after the drag: 80 = 4*18 + 8, then wrap-end.
    assert_eq!(
        composer(&after),
        (
            22,
            80,
            vec![
                "a".repeat(18),
                "a".repeat(18),
                "a".repeat(18),
                "a".repeat(18),
                "aaaaaaaawrap-end".to_owned()
            ]
        ),
        "all rows indent under the address at the resized 18-cell draft width"
    );
    assert_eq!(
        rig.tmux(&[
            "display-message",
            "-p",
            "-t",
            &pane,
            "#{cursor_flag},#{cursor_x},#{cursor_y}"
        ])
        .trim(),
        "1,96,26"
    );
    rig.release(&pane, 56, 1);
}

#[test]
fn live_grab_feedback_changes_rule_style_then_release_restores_it() {
    for look in [false, true] {
        let rig = Rig::new(if look { "arcolor" } else { "arplain" }, 0, &[]);
        if look {
            rig.tmux(&["set-option", "-u", "-t", &rig.name, "@ae_look"]);
        }
        let pane = rig.open_app();
        rig.ready(&pane);
        let before = rig.tmux(&["capture-pane", "-e", "-p", "-t", &pane]);
        let before = before.lines().next().expect("top rule row").to_owned();
        rig.click(&pane, 44, 1);
        let grabbed = rig.styled(&pane, "grab changed rule style", |screen| {
            screen.lines().next().is_some_and(|row| row != before)
        });
        assert!(grabbed.contains("Enter writes"));
        let grabbed = grabbed.lines().next().expect("top rule row");
        assert_ne!(
            grabbed, before,
            "grab changes visible style with theme={look}"
        );
        if look {
            assert!(grabbed.contains("38;2;"));
        } else {
            assert!(!grabbed.contains("38;2;"), "theme off draws no color");
        }
        rig.release(&pane, 44, 1);
        let after = rig.styled(&pane, "release restored original style", |screen| {
            screen.lines().next() == Some(before.as_str())
        });
        assert_eq!(
            after.lines().next(),
            Some(before.as_str()),
            "release restores original rule style"
        );
    }
}

/// A real tmux client's terminal receives the reports; stock bindings must
/// forward them to the app. Direct app-pane injection cannot satisfy this test.
#[test]
fn live_drag_through_stock_tmux_client_reaches_app_and_release() {
    let rig = Rig::new("arclient", 0, &[]);
    let pane = rig.open_app();
    rig.ready(&pane);
    rig.tmux(&["set-option", "-t", &rig.name, "status", "off"]);
    rig.tmux(&["set-option", "-t", &rig.name, "mouse", "on"]);
    // A second private server provides a real terminal to the first client.
    let outer = rig.root.join("outer.sock");
    let command = format!(
        "exec env -u TMUX -u TMUX_PANE -u CLAUDE_CONFIG_DIR -u CODEX_HOME tmux -u -S {} attach-session -t {}",
        quote(&rig.socket.display().to_string()),
        quote(&rig.name),
    );
    let outer_tmux = |tail: &[&str]| {
        let mut args = vec!["-S".to_owned(), outer.display().to_string()];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &rig.root);
        assert!(ok, "outer private tmux {tail:?}: {out}");
        out
    };
    let terminal = outer_tmux(&[
        "-f",
        "/dev/null",
        "new-session",
        "-d",
        "-P",
        "-F",
        "#{pane_id}",
        "-x",
        "160",
        "-y",
        "46",
        "-s",
        "terminal",
        &command,
    ])
    .trim()
    .to_owned();
    let until = Instant::now() + WAIT;
    loop {
        let clients = rig.tmux(&["list-clients", "-F", "#{session_name}"]);
        if clients.lines().any(|name| name == rig.name) {
            break;
        }
        assert!(Instant::now() < until, "real inner client never attached");
        std::thread::sleep(Duration::from_millis(25));
    }
    rig.vertical(&pane, 44);
    let before = rig.tmux(&["capture-pane", "-e", "-p", "-t", &pane]);
    let before_style = style_before_rule(before.lines().next().expect("rule row"));
    let bytes = "\x1b[<0;45;2M\x1b[<32;67;2M";
    outer_tmux(&["send-keys", "-t", &terminal, "-l", "--", bytes]);
    rig.vertical(&pane, 66);
    let grabbed = rig.styled(&pane, "forwarded press gave visible feedback", |screen| {
        style_before_rule(screen.lines().next().expect("rule row")) != before_style
    });
    assert_ne!(
        style_before_rule(grabbed.lines().next().expect("rule row")),
        before_style,
        "forwarded press gave visible feedback"
    );
    outer_tmux(&["send-keys", "-t", &terminal, "-l", "--", "\x1b[<0;67;2m"]);
    let after = rig.styled(&pane, "forwarded release restored style", |screen| {
        style_before_rule(screen.lines().next().expect("rule row")) == before_style
    });
    assert_eq!(
        style_before_rule(after.lines().next().expect("rule row")),
        before_style,
        "forwarded release restored the original rule style"
    );
    outer_tmux(&["kill-server"]);
}

/// Row zero is blank before its vertical rule. Removing spaces leaves its
/// exact terminal style sequences without depending on their SGR spelling.
fn style_before_rule(row: &str) -> String {
    row.split_once('│').expect("drawn rule").0.replace(' ', "")
}
