//! appmouse frozen acceptance spec: mouse support in `ae app`.
//!
//! Oracles: brief-appmouse.md rulings 1-8, the DRAWN frame (a captured pane's
//! own cells pick every click target — never driver arithmetic), docs/app.md
//! keys/ownership words, and the public `Keys::app().feed` decoder. Nothing
//! here calls driver-invented API, so the suite compiles on the S0 stub and
//! each mouse test is RED there at its mouse assertion; guards (marked GUARD)
//! are green on S0 and prove the rig.
//!
//! RED-on-S0 map: every `sgr_*`, `x10_*`, `mouse_*` decoder test fails on S0
//! (the stub emits no `Key::Mouse`); `chat_decoder_never_spells_mouse`,
//! `paste_keeps_mouse_bytes_literal` and `idle_drops_partial_mouse_then_fresh`
//! pass on S0 as guards. Live: `guard_literal_arrow_moves_selection` passes on
//! S0 (arrow injection works); `tiny_clicks_noop`,
//! `click_composer_on_stopped_row_noops` and `chat_window_never_enables_mouse`
//! pass vacuously on S0 and pin their rule on GREEN; every other live test is
//! RED on S0.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ae::console::input::{ESC_IDLE, Key, Keys, Mouse, MouseKind};

// ---------------------------------------------------------------------------
// decoder: SGR through the public app decoder
// ---------------------------------------------------------------------------

/// The keys `bytes` decode to, stamps dropped.
fn feed(keys: &mut Keys, bytes: &[u8]) -> Vec<Key> {
    keys.feed(bytes, Instant::now())
        .into_iter()
        .map(|(key, _)| key)
        .collect()
}

fn mouse(kind: MouseKind, column: u16, row: u16) -> Key {
    Key::Mouse(Mouse { kind, column, row })
}

/// Ruling 2/3: a plain left press reports its 1-based cell as 0-based.
#[test]
fn sgr_click_reports_zero_based_cell() {
    let mut app = Keys::app();
    assert_eq!(
        feed(&mut app, b"\x1b[<0;7;6M"),
        [mouse(MouseKind::Click, 6, 5)]
    );
    assert_eq!(
        feed(&mut app, b"\x1b[<0;1;1M"),
        [mouse(MouseKind::Click, 0, 0)]
    );
    assert_eq!(
        feed(&mut app, b"\x1b[<0;123;45M"),
        [mouse(MouseKind::Click, 122, 44)]
    );
}

/// Ruling 2/3: wheel codes report direction with the converted cell.
#[test]
fn sgr_wheel_reports_direction() {
    let mut app = Keys::app();
    assert_eq!(
        feed(&mut app, b"\x1b[<64;10;20M"),
        [mouse(MouseKind::WheelUp, 9, 19)]
    );
    assert_eq!(
        feed(&mut app, b"\x1b[<65;10;20M"),
        [mouse(MouseKind::WheelDown, 9, 19)]
    );
}

/// appresize ruling 3 supersedes the old release drop: unmodified left
/// release is admitted; other buttons still vanish, then a click decodes.
#[test]
fn sgr_release_admitted_and_other_buttons_dropped_then_click() {
    assert_eq!(
        feed(&mut Keys::app(), b"\x1b[<0;5;5m\x1b[<0;9;9M"),
        [
            mouse(MouseKind::Release, 4, 4),
            mouse(MouseKind::Click, 8, 8)
        ]
    );
    for dropped in [
        b"\x1b[<3;5;5m".as_slice(),
        b"\x1b[<1;5;5M".as_slice(),
        b"\x1b[<2;5;5M".as_slice(),
        b"\x1b[<64;5;5m".as_slice(),
    ] {
        let mut app = Keys::app();
        let mut bytes = dropped.to_vec();
        bytes.extend_from_slice(b"\x1b[<0;9;9M");
        assert_eq!(
            feed(&mut app, &bytes),
            [mouse(MouseKind::Click, 8, 8)],
            "dropped then click: {bytes:?}"
        );
    }
}

/// appresize ruling 3 admits plain left motion 32; modifier bits, other
/// motion and modified wheel still vanish, then a click decodes.
#[test]
fn sgr_modifier_and_motion_reports_dropped_then_click() {
    assert_eq!(
        feed(&mut Keys::app(), b"\x1b[<32;5;5M\x1b[<0;9;9M"),
        [mouse(MouseKind::Drag, 4, 4), mouse(MouseKind::Click, 8, 8)]
    );
    for code in [4, 8, 16, 20, 24, 33, 34, 35, 36, 68, 69] {
        let mut app = Keys::app();
        let bytes = format!("\x1b[<{code};5;5M\x1b[<0;9;9M");
        assert_eq!(
            feed(&mut app, bytes.as_bytes()),
            [mouse(MouseKind::Click, 8, 8)],
            "code {code} dropped then click"
        );
    }
}

/// Ruling 2: malformed and oversized reports vanish — never Text, never a
/// panic — and a following click still decodes.
#[test]
fn sgr_malformed_dropped_never_text() {
    for dropped in [
        "<M",
        "<;1;1M",
        "<0;;1M",
        "<0;1M",
        "<0;1;2;3M",
        "<a;1;1M",
        "<0;b;1M",
        "<0;1;cM",
        "<0;0;1M",
        "<0;1;0M",
        "<0;99999;1M",
        "<0;1;99999M",
        "<-1;1;1M",
        "<0;1;1",
    ] {
        let mut app = Keys::app();
        let bytes = format!("\x1b[{dropped}\x1b[<0;9;9M");
        assert_eq!(
            feed(&mut app, bytes.as_bytes()),
            [mouse(MouseKind::Click, 8, 8)],
            "malformed {dropped:?} dropped then click"
        );
    }
}

/// Ruling 2 + plan res 1: an unterminated report past the storage cap is
/// discarded through its CSI final with no tail in Text; a fresh ESC starts
/// over without an Escape for the discarded bytes.
#[test]
fn sgr_overlong_discards_to_final_then_resyncs() {
    let mut overlong = b"\x1b[<0;".to_vec();
    overlong.extend(std::iter::repeat_n(b'9', 40));
    let mut app = Keys::app();
    assert!(feed(&mut app, &overlong).is_empty(), "no tail in Text");
    assert!(feed(&mut app, b"M").is_empty(), "the final is consumed");
    assert_eq!(feed(&mut app, b"q"), [Key::Text(b"q".to_vec())]);
    let mut app = Keys::app();
    assert!(feed(&mut app, &overlong).is_empty());
    assert_eq!(feed(&mut app, b"\x1b[A"), [Key::Up], "ESC resyncs");
}

/// Ruling 2 + plan res 2: legacy X10 consumes exactly its 3 payload bytes —
/// even ESC/control bytes, even split across reads — and emits nothing.
#[test]
fn x10_consumes_three_bytes_then_click() {
    let payload = b"\x1b[MABC\x1b[<0;9;9M";
    for at in 1..payload.len() {
        let mut app = Keys::app();
        let stamp = Instant::now();
        let mut keys = app.feed(&payload[..at], stamp);
        keys.extend(app.feed(&payload[at..], stamp));
        let keys: Vec<Key> = keys.into_iter().map(|(key, _)| key).collect();
        assert_eq!(
            keys,
            [mouse(MouseKind::Click, 8, 8)],
            "X10 split at {at} then click"
        );
    }
    let mut app = Keys::app();
    let bytes = b"\x1b[M\x1bBC\x1b[<0;9;9M";
    assert_eq!(
        feed(&mut app, bytes),
        [mouse(MouseKind::Click, 8, 8)],
        "an ESC payload byte is consumed, not a resync"
    );
}

/// Objective: the chat's decoder never spells a mouse key, on any mouse
/// bytes. GUARD: green on S0, pins the chat unchanged through S2.
#[test]
fn chat_decoder_never_spells_mouse() {
    let battery = [
        b"\x1b[<0;7;6M".as_slice(),
        b"\x1b[<64;10;20M".as_slice(),
        b"\x1b[<65;10;20M".as_slice(),
        b"\x1b[<0;5;5m".as_slice(),
        b"\x1b[<3;5;5m".as_slice(),
        b"\x1b[<32;5;5M".as_slice(),
        b"\x1b[<0;99999;1M".as_slice(),
        b"\x1b[MABC".as_slice(),
    ];
    for bytes in battery {
        let mut chat = Keys::default();
        assert!(
            !feed(&mut chat, bytes)
                .iter()
                .any(|key| matches!(key, Key::Mouse(_))),
            "chat stays mouse-free: {bytes:?}"
        );
    }
}

/// Ruling 2: inside a bracketed paste mouse bytes stay literal (the existing
/// paste rule). GUARD: green on S0.
#[test]
fn paste_keeps_mouse_bytes_literal() {
    let mut app = Keys::app();
    let report = b"\x1b[<0;5;5M";
    let mut bytes = b"\x1b[200~".to_vec();
    bytes.extend_from_slice(report);
    bytes.extend_from_slice(b"\x1b[201~");
    assert_eq!(feed(&mut app, &bytes), [Key::Pasted(report.to_vec())]);
}

/// A read boundary never changes a report: every split of one wheel report
/// decodes to the same non-empty keys.
#[test]
fn mouse_survives_every_read_split() {
    let bytes = b"\x1b[<64;10;20M";
    let stamp = Instant::now();
    let whole: Vec<(Key, Instant)> = Keys::app().feed(bytes, stamp);
    assert_eq!(whole, [(mouse(MouseKind::WheelUp, 9, 19), stamp)]);
    for at in 1..bytes.len() {
        let mut app = Keys::app();
        let mut keys = app.feed(&bytes[..at], stamp);
        keys.extend(app.feed(&bytes[at..], stamp));
        assert_eq!(keys, whole, "wheel split at {at}");
    }
}

/// Plan res 1: a partial report past the idle bound is dropped, never a key;
/// later bytes start fresh. GUARD: green on S0.
#[test]
fn idle_drops_partial_mouse_then_fresh() {
    let t0 = Instant::now();
    let mut app = Keys::app();
    assert!(app.feed(b"\x1b[<0;1", t0).is_empty());
    assert!(app.idle(t0 + ESC_IDLE).is_empty(), "dropped, no key");
    assert_eq!(
        app.feed(b"q", t0 + ESC_IDLE)
            .into_iter()
            .map(|(key, _)| key)
            .collect::<Vec<_>>(),
        [Key::Text(b"q".to_vec())]
    );
}

/// A report carries the stamp of the read that held its first byte.
#[test]
fn mouse_carries_first_byte_stamp() {
    let t0 = Instant::now();
    let t1 = t0 + Duration::from_millis(1);
    let mut app = Keys::app();
    let mut keys = app.feed(b"\x1b[<0;", t0);
    keys.extend(app.feed(b"7;6M", t1));
    assert_eq!(keys, [(mouse(MouseKind::Click, 6, 5), t0)]);
}

// ---------------------------------------------------------------------------
// live: the app in a private tmux, literal SGR bytes into its pane
// ---------------------------------------------------------------------------

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);
/// A no-op settles this long before the frame is judged unchanged.
const SETTLE: Duration = Duration::from_millis(1500);

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

    /// The screen once `met` holds, within `within`.
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

/// The 0-based (column, row) of `needle`'s first cell in `screen`, columns in
/// characters.
fn cell_of(screen: &str, needle: &str) -> Option<(usize, usize)> {
    screen.lines().enumerate().find_map(|(row, line)| {
        line.chars()
            .collect::<Vec<_>>()
            .windows(needle.chars().count())
            .position(|w| w.iter().collect::<String>() == needle)
            .map(|col| (col, row))
    })
}

/// The sidebar occurrence of `needle`: session names also show in the chat
/// header and composer, which stand far right of the sidebar's cells.
fn sidebar_cell_of(screen: &str, needle: &str) -> Option<(usize, usize)> {
    screen.lines().enumerate().find_map(|(row, line)| {
        line.chars()
            .collect::<Vec<_>>()
            .windows(needle.chars().count())
            .position(|w| w.iter().collect::<String>() == needle)
            .filter(|col| *col <= 10)
            .map(|col| (col, row))
    })
}

/// GUARD: literal escape bytes reach the app and move the selection — the
/// injection path every mouse test below rides. Green on S0.
#[test]
fn guard_literal_arrow_moves_selection() {
    let rig = Rig::new("amguard", 0, &["amstop"]);
    let pane = rig.open_app();
    rig.wait(&pane, WAIT, "both rows listed", |screen| {
        screen.contains("Sessions 2") && screen.contains("amstop")
    });
    rig.tmux(&["send-keys", "-t", &pane, "-l", "--", "\x1b[B"]);
    rig.wait(&pane, WAIT, "Down selects the stopped row", |screen| {
        screen.contains("amstop is stopped")
    });
    rig.tmux(&["send-keys", "-t", &pane, "-l", "--", "\x1b[A"]);
    rig.wait(&pane, WAIT, "Up returns home", |screen| {
        screen.contains("Enter writes")
    });
}

/// Ruling 5: clicking a session row selects it as its number key would; the
/// stopped target names its own refusal, the home row returns.
#[test]
fn click_selects_session_row() {
    let rig = Rig::new("amsel", 0, &["amstop"]);
    let pane = rig.open_app();
    let home = rig.name.clone();
    let screen = rig.wait(&pane, WAIT, "both rows listed", |screen| {
        screen.contains("Sessions 2") && screen.contains("amstop")
    });
    let (col, row) = sidebar_cell_of(&screen, "amstop").expect("the stopped row");
    rig.click(&pane, col, row);
    let screen = rig.wait(&pane, WAIT, "the click selects the row", |screen| {
        screen.contains("amstop is stopped")
    });
    let (col, row) = sidebar_cell_of(&screen, &home).expect("the home row");
    rig.click(&pane, col, row);
    rig.wait(&pane, WAIT, "the home row returns", |screen| {
        screen.contains("Enter writes")
    });
}

/// Ruling 5: clicking a tab label shows that tab, like Tab would.
#[test]
fn click_tab_label_switches_tabs() {
    let rig = Rig::new("amtab", 0, &[]);
    let pane = rig.open_app();
    let screen = rig.wait(&pane, WAIT, "first paint lists the session", |screen| {
        screen.contains("Sessions 1") && screen.contains("Overview")
    });
    let (col, row) = cell_of(&screen, "Agents").expect("the Agents label");
    rig.click(&pane, col, row);
    let screen = rig.wait(&pane, WAIT, "Agents shows", |screen| {
        screen.contains("No seat facts")
    });
    let (col, row) = cell_of(&screen, "Overview").expect("the Overview label");
    rig.click(&pane, col, row);
    rig.wait(&pane, WAIT, "Overview returns", |screen| {
        screen.contains("Goal")
    });
}

/// Ruling 5: clicking the composer starts writing exactly where Enter would;
/// while writing, a row click returns to browsing with the draft kept.
#[test]
fn click_composer_writes_then_row_returns_with_draft() {
    let rig = Rig::new("amcomp", 0, &[]);
    let pane = rig.open_app();
    let home = rig.name.clone();
    let address = format!("to {home}");
    let screen = rig.wait(&pane, WAIT, "the owner composer", |screen| {
        screen.contains(&address)
    });
    let (col, row) = cell_of(&screen, &address).expect("the composer row");
    rig.click(&pane, col, row);
    rig.wait(&pane, WAIT, "writing starts", |screen| {
        screen.contains("Enter sends")
    });
    rig.tmux(&["send-keys", "-t", &pane, "-l", "--", "mousedraft"]);
    let screen = rig.wait(&pane, WAIT, "the draft shows", |screen| {
        screen.contains("mousedraft")
    });
    let (col, row) = sidebar_cell_of(&screen, &home).expect("the home row");
    rig.click(&pane, col, row);
    rig.wait(
        &pane,
        WAIT,
        "browsing returns with the draft kept",
        |screen| screen.contains("draft kept"),
    );
}

/// Ruling 5: a spacing row between sessions is a blank row and no-ops.
#[test]
fn click_blank_row_noops() {
    let rig = Rig::new("amblank", 0, &["amstop"]);
    let pane = rig.open_app();
    let home = rig.name.clone();
    let screen = rig.wait(&pane, WAIT, "both rows listed", |screen| {
        screen.contains("Sessions 2") && screen.contains("Enter writes")
    });
    let (col, row) = sidebar_cell_of(&screen, &home).expect("the home row");
    let rows: Vec<&str> = screen.lines().collect();
    assert!(
        !rows[row + 2].contains(&home) && !rows[row + 2].contains("amstop"),
        "the target is a spacing row:\n{screen}"
    );
    rig.click(&pane, col, row + 2);
    std::thread::sleep(SETTLE);
    let screen = rig.screen(&pane);
    assert!(
        screen.contains("Enter writes"),
        "still browsing home:\n{screen}"
    );
    assert!(
        screen.lines().nth(1).is_some_and(|row| row.contains(&home)),
        "no selection moved:\n{screen}"
    );
}

/// Ruling 5: the "more" row no-ops.
#[test]
fn click_more_row_noops() {
    let stopped = [
        "mstop00", "mstop01", "mstop02", "mstop03", "mstop04", "mstop05", "mstop06", "mstop07",
        "mstop08", "mstop09", "mstop10", "mstop11",
    ];
    let rig = Rig::new("ammore", 0, &stopped);
    let pane = rig.open_app();
    let screen = rig.wait(&pane, WAIT, "the windowed list", |screen| {
        screen.contains("Sessions 13") && screen.contains("more")
    });
    let (col, row) = sidebar_cell_of(&screen, "more").expect("the more row");
    rig.click(&pane, col, row);
    std::thread::sleep(SETTLE);
    let screen = rig.screen(&pane);
    assert!(
        screen.contains("Enter writes"),
        "still browsing home:\n{screen}"
    );
    assert!(
        screen
            .lines()
            .nth(1)
            .is_some_and(|row| row.contains(&rig.name)),
        "no selection moved:\n{screen}"
    );
}

/// Ruling 5 + plan res 4: the wheel over the chat column scrolls the chat a
/// few rows per notch (the scrolled frame says newer turns wait below); back
/// down it settles at the newest turn.
#[test]
fn wheel_over_chat_scrolls_and_returns() {
    let rig = Rig::new("amwheel", 12, &[]);
    let pane = rig.open_app();
    let home = rig.name.clone();
    let address = format!("to {home}");
    let screen = rig.wait(&pane, WAIT, "newest turn at the bottom", |screen| {
        screen.contains("scrollprobe-newest") && !screen.contains("scrollprobe-oldest")
    });
    let (col, row) = cell_of(&screen, &address).expect("the composer row");
    for _ in 0..8 {
        rig.wheel(&pane, true, col, row - 4);
    }
    rig.wait(&pane, WAIT, "scrolled back to the oldest turn", |screen| {
        screen.contains("scrollprobe-oldest") && screen.contains("newer turns below")
    });
    for _ in 0..8 {
        rig.wheel(&pane, false, col, row - 4);
    }
    rig.wait(&pane, WAIT, "settled back at the newest turn", |screen| {
        !screen.contains("newer turns below") && screen.contains("scrollprobe-newest")
    });
}

/// Ruling 5: the wheel over the sidebar no-ops — no scroll, no selection.
#[test]
fn wheel_over_sidebar_noops() {
    let rig = Rig::new("amwside", 12, &["amstop"]);
    let pane = rig.open_app();
    let home = rig.name.clone();
    let screen = rig.wait(&pane, WAIT, "both rows listed", |screen| {
        screen.contains("Sessions 2") && screen.contains("scrollprobe-newest")
    });
    let (col, row) = sidebar_cell_of(&screen, &home).expect("the home row");
    for _ in 0..3 {
        rig.wheel(&pane, true, col, row);
    }
    std::thread::sleep(SETTLE);
    let screen = rig.screen(&pane);
    assert!(
        !screen.contains("newer turns below"),
        "nothing scrolled:\n{screen}"
    );
    assert!(
        screen.contains("Enter writes"),
        "still browsing home:\n{screen}"
    );
}

/// Ruling 6: without a sidebar clicks still hit what is drawn — the composer
/// starts writing.
#[test]
fn narrow_clicks_hit_drawn_only() {
    let rig = Rig::new("amnarrow", 0, &[]);
    let pane = rig.open_app();
    let home = rig.name.clone();
    let address = format!("to {home}");
    rig.wait(&pane, WAIT, "first paint lists the session", |screen| {
        screen.contains("Sessions 1")
    });
    rig.tmux(&["resize-window", "-t", &pane, "-x", "80", "-y", "24"]);
    let screen = rig.wait(&pane, WAIT, "the sidebar goes", |screen| {
        screen.contains("sidebar needs")
    });
    let (col, row) = cell_of(&screen, &address).expect("the composer row");
    rig.click(&pane, col, row);
    rig.wait(&pane, WAIT, "writing starts", |screen| {
        screen.contains("Enter sends")
    });
}

/// Ruling 6: below the minimum size clicks no-op; back at full size the
/// selection never moved. GUARD on S0: green vacuously, pins the rule on GREEN.
#[test]
fn tiny_clicks_noop() {
    let rig = Rig::new("amtiny", 0, &["amstop"]);
    let pane = rig.open_app();
    rig.wait(&pane, WAIT, "both rows listed", |screen| {
        screen.contains("Sessions 2")
    });
    rig.tmux(&["resize-window", "-t", &pane, "-x", "30", "-y", "10"]);
    rig.wait(&pane, WAIT, "the needs-size note", |screen| {
        screen.contains("needs at least")
    });
    rig.click(&pane, 2, 2);
    rig.wheel(&pane, true, 2, 4);
    std::thread::sleep(SETTLE);
    rig.tmux(&["resize-window", "-t", &pane, "-x", "160", "-y", "45"]);
    rig.wait(&pane, WAIT, "home still selected", |screen| {
        screen.contains("Sessions 2") && screen.contains("Enter writes")
    });
}

/// Ruling 1: while the app runs, its pane reports mouse mode on.
#[test]
fn live_app_enables_mouse_mode() {
    let rig = Rig::new("amflag", 0, &[]);
    let pane = rig.open_app();
    rig.wait(&pane, WAIT, "first paint lists the session", |screen| {
        screen.contains("Sessions 1")
    });
    let flag = rig.tmux(&["display-message", "-p", "-t", &pane, "#{mouse_any_flag}"]);
    assert_eq!(flag.trim(), "1", "mouse mode is on in the app pane");
}

// ---------------------------------------------------------------------------
// round 2 (lead review fbd8cddc): exact modes, wheel while writing, foreign
// composer, one notch in rows, tab while writing, horizontal wheel, chat modes
// ---------------------------------------------------------------------------

/// A rig window running `command` (sh) with the app's scratch environment and
/// no harness config overrides; its pane.
fn window(rig: &Rig, command: &str) -> String {
    let home = rig.root.display().to_string();
    let env = [
        format!("AE_HOME={}", rig.root.display()),
        format!("CONFIG_FILE={}", rig.root.join("config").display()),
        format!("HOME={home}"),
        "AE_TMUX_SERVER_KIND=socket".to_owned(),
        format!("AE_TMUX_SERVER={}", rig.socket.display()),
    ];
    let mut args = vec![
        "new-window",
        "-d",
        "-P",
        "-F",
        "#{pane_id}",
        "-t",
        rig.name.as_str(),
    ];
    for pair in &env {
        args.extend(["-e", pair.as_str()]);
    }
    args.push(command);
    rig.tmux(&args).trim().to_owned()
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// appresize ruling 2: button-motion + SGR while running, no any-motion;
/// after `q` both are off again. The five formats exist on
/// the tmux 3.4 floor (probed in the lane container), so no fallback.
#[test]
fn live_app_mouse_mode_set_exact() {
    let rig = Rig::new("ammodes", 0, &[]);
    let pane = window(
        &rig,
        &format!(
            "env -u CLAUDE_CONFIG_DIR -u CODEX_HOME {} app {}; sleep 60",
            quote(env!("CARGO_BIN_EXE_ae")),
            quote(&rig.name),
        ),
    );
    rig.wait(&pane, WAIT, "first paint lists the session", |screen| {
        screen.contains("Sessions 1")
    });
    for (format, want) in [
        ("#{mouse_any_flag}", "1"),
        ("#{mouse_standard_flag}", "0"),
        ("#{mouse_sgr_flag}", "1"),
        ("#{mouse_button_flag}", "1"),
        ("#{mouse_all_flag}", "0"),
    ] {
        let got = rig.tmux(&["display-message", "-p", "-t", &pane, format]);
        assert_eq!(got.trim(), want, "{format} while running");
    }
    rig.tmux(&["send-keys", "-t", &pane, "q", "q"]);
    rig.wait(&pane, WAIT, "the app quit", |screen| {
        !screen.contains("Sessions 1")
    });
    for format in ["#{mouse_any_flag}", "#{mouse_sgr_flag}"] {
        let got = rig.tmux(&["display-message", "-p", "-t", &pane, format]);
        assert_eq!(got.trim(), "0", "{format} after quitting");
    }
}

/// Ruling 5: the wheel scrolls in both modes — while writing the frame moves
/// and the draft stays on the composer.
#[test]
fn wheel_while_writing_scrolls_and_keeps_draft() {
    let rig = Rig::new("amwwrite", 12, &[]);
    let pane = rig.open_app();
    let home = rig.name.clone();
    let address = format!("to {home}");
    rig.wait(&pane, WAIT, "newest turn at the bottom", |screen| {
        screen.contains("scrollprobe-newest") && !screen.contains("scrollprobe-oldest")
    });
    rig.tmux(&["send-keys", "-t", &pane, "i"]);
    rig.wait(&pane, WAIT, "writing starts", |screen| {
        screen.contains("Enter sends")
    });
    rig.tmux(&["send-keys", "-t", &pane, "-l", "--", "wheeldraft"]);
    let screen = rig.wait(&pane, WAIT, "the draft shows", |screen| {
        screen.contains("wheeldraft")
    });
    let (col, row) = cell_of(&screen, &address).expect("the composer row");
    for _ in 0..8 {
        rig.wheel(&pane, true, col, row - 4);
    }
    rig.wait(&pane, WAIT, "scrolled and still writing", |screen| {
        screen.contains("newer turns below")
            && screen.contains("Enter sends")
            && screen.contains("wheeldraft")
    });
}

/// Ruling 5: the composer click only starts writing where Enter would — on a
/// stopped row it no-ops: the named refusal and lack of write mode are pinned.
#[test]
fn click_composer_on_stopped_row_noops() {
    let rig = Rig::new("amforeign", 0, &["amstop"]);
    let pane = rig.open_app();
    rig.wait(&pane, WAIT, "both rows listed", |screen| {
        screen.contains("Sessions 2")
    });
    rig.tmux(&["send-keys", "-t", &pane, "2"]);
    let screen = rig.wait(
        &pane,
        WAIT,
        "B2 selected stopped row names its refusal",
        |screen| screen.contains("amstop is stopped"),
    );
    let (col, row) = cell_of(&screen, "amstop is stopped").expect("the stopped composer");
    rig.click(&pane, col, row);
    std::thread::sleep(SETTLE);
    let screen = rig.screen(&pane);
    assert!(
        screen.contains("amstop is stopped"),
        "still the browse composer:\n{screen}"
    );
    assert!(
        !screen.contains("Enter sends"),
        "no writing started:\n{screen}"
    );
}

/// Ruling 5: one wheel notch moves the chat exactly 3 rows, read off the
/// frame — wheel up shows older turns, so between two scrolled frames one
/// more notch up sits the same mid-screen marker exactly 3 rows lower, and one
/// notch down returns it.
#[test]
fn one_wheel_notch_moves_exactly_three_rows() {
    const MARK: &str = "scrollprobe-filler-05";
    let rig = Rig::new("amnotch", 12, &[]);
    let pane = rig.open_app();
    let home = rig.name.clone();
    let address = format!("to {home}");
    let screen = rig.wait(&pane, WAIT, "newest turn at the bottom", |screen| {
        screen.contains("scrollprobe-newest") && !screen.contains("scrollprobe-oldest")
    });
    let (col, row) = cell_of(&screen, &address).expect("the composer row");
    rig.wheel(&pane, true, col, row - 4);
    let screen = rig.wait(&pane, WAIT, "scrolled back one notch", |screen| {
        screen.contains("newer turns below")
    });
    let row1 = screen
        .lines()
        .position(|line| line.contains(MARK))
        .expect("the mid-screen marker on screen");
    rig.wheel(&pane, true, col, row - 4);
    rig.wait(&pane, WAIT, "exactly 3 rows lower", |screen| {
        screen
            .lines()
            .position(|line| line.contains(MARK))
            .is_some_and(|at| at == row1 + 3)
    });
    rig.wheel(&pane, false, col, row - 4);
    rig.wait(&pane, WAIT, "one notch down returns it", |screen| {
        screen
            .lines()
            .position(|line| line.contains(MARK))
            .is_some_and(|at| at == row1)
    });
}

/// While writing, a tab-label click returns to browsing with the draft kept,
/// then shows that tab.
#[test]
fn click_tab_while_writing_returns_with_draft() {
    let rig = Rig::new("amtabw", 0, &[]);
    let pane = rig.open_app();
    rig.wait(&pane, WAIT, "first paint lists the session", |screen| {
        screen.contains("Sessions 1")
    });
    rig.tmux(&["send-keys", "-t", &pane, "i"]);
    rig.wait(&pane, WAIT, "writing starts", |screen| {
        screen.contains("Enter sends")
    });
    rig.tmux(&["send-keys", "-t", &pane, "-l", "--", "tabdraft"]);
    let screen = rig.wait(&pane, WAIT, "the draft shows", |screen| {
        screen.contains("tabdraft")
    });
    let (col, row) = cell_of(&screen, "Agents").expect("the Agents label");
    rig.click(&pane, col, row);
    rig.wait(&pane, WAIT, "Agents with the draft kept", |screen| {
        screen.contains("No seat facts") && screen.contains("draft kept")
    });
}

/// Ruling 2: horizontal wheel reports vanish like any other non-kept report.
#[test]
fn sgr_horizontal_wheel_dropped_then_click() {
    for code in [66, 67] {
        let mut app = Keys::app();
        let bytes = format!("\x1b[<{code};5;5M\x1b[<0;9;9M");
        assert_eq!(
            feed(&mut app, bytes.as_bytes()),
            [mouse(MouseKind::Click, 8, 8)],
            "code {code} dropped then click"
        );
    }
}

/// A mouse report ends at M/m alone: a CSI-final byte inside a malformed
/// report never ends it early with a Text tail (driver-agreed mechanism).
#[test]
fn sgr_malformed_stops_only_at_m() {
    for dropped in ["<0;1q;1M", "<4;2;2Q;3;3M", "<0;9z9;9M"] {
        let mut app = Keys::app();
        let bytes = format!("\x1b[{dropped}\x1b[<0;9;9M");
        assert_eq!(
            feed(&mut app, bytes.as_bytes()),
            [mouse(MouseKind::Click, 8, 8)],
            "malformed {dropped:?} dropped then click"
        );
    }
}

/// `ae chat` never enables mouse mode: its window reports no mouse flags.
/// GUARD: green on S0 and GREEN, pins the chat unchanged.
#[test]
fn chat_window_never_enables_mouse() {
    let rig = Rig::new("amchat", 0, &[]);
    std::fs::write(rig.root.join("config"), "[workspace]\nchat = on\n").expect("chat = on");
    let pane = rig.open_app();
    rig.wait(&pane, WAIT, "the chat composer", |screen| {
        screen.contains("to lead")
    });
    let dead = rig.tmux(&["display-message", "-p", "-t", &pane, "#{pane_dead}"]);
    assert_eq!(dead.trim(), "0", "the chat is running");
    let flag = rig.tmux(&["display-message", "-p", "-t", &pane, "#{mouse_any_flag}"]);
    assert_eq!(flag.trim(), "0", "the chat enables no mouse mode");
}
