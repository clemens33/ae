//! Frozen selection acceptance: brief appselect R1-R6 and lead rulings 2026-10-11.
//! Text oracles are literal drawn-cell fixtures, independent of the extractor.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::too_many_lines,
    reason = "acceptance owns a private terminal fixture and reads its effects"
)]

use ae::app::{draw, fleet, model, overview, peek, select};
use ae::console::lane::{Item, Kind, Lane};
use ae::digest::{SessionEntry, Status};
use ae::theme::Mark;
use ratatui_core::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Modifier, Style},
};
use std::time::{Duration, Instant};

fn selection(area: Rect, anchor: (u16, u16), head: (u16, u16)) -> select::Selection {
    let region = select::Region {
        panel: select::Panel::Lane,
        area,
    };
    let mut selected = select::Selection::start(&region, anchor.into(), 0);
    let _ = selected.extend(head.into());
    selected
}

#[test]
fn stream_rows_trim_blanks_keep_empty_rows_and_reverse_direction() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 12, 5));
    for (row, line) in [(1, "abcdef"), (2, ""), (3, "last")] {
        buf.set_string(1, row, line, Style::default());
    }
    let area = Rect::new(1, 1, 8, 3);
    for (anchor, head) in [((4, 1), (3, 3)), ((3, 3), (4, 1))] {
        let held = selection(area, anchor, head);
        assert_eq!(select::text(&buf, &held, &[]), "def\n\nlas", "R3 rows");
    }
    let held = selection(area, (4, 1), (11, 4));
    assert_eq!(select::text(&buf, &held, &[]), "def\n\nlast", "R3 clamp");
    assert_eq!(held.head(), Position::new(8, 3));
}

#[test]
fn wide_continuations_and_combining_clusters_copy_whole_drawn_symbols() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 3));
    buf.set_string(1, 1, "A你e\u{301}Z", Style::default());
    let held = selection(Rect::new(1, 1, 8, 1), (3, 1), (4, 1));
    assert_eq!(
        select::text(&buf, &held, &[]),
        "你e\u{301}",
        "R3 whole glyph"
    );
    let original = buf.clone();
    select::highlight(&mut buf, &held, &[]);
    for column in 0..10 {
        assert_eq!(
            buf[(column, 1)].modifier != original[(column, 1)].modifier,
            (2..=4).contains(&column),
            "R1 glyph cell {column}"
        );
    }
}

#[test]
fn controls_copy_as_drawn_and_overlay_inverts_already_reversed_cells() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 8, 3));
    buf.set_string(1, 1, "A?\u{FFFD}", Style::default());
    buf[(2, 1)].set_symbol("\x1b[31m");
    buf[(1, 1)].modifier = Modifier::BOLD | Modifier::REVERSED;
    let held = selection(Rect::new(1, 1, 6, 1), (1, 1), (6, 1));
    assert_eq!(
        select::text(&buf, &held, &[]),
        "A \u{FFFD}",
        "R3 neutralisation"
    );
    let original = buf.clone();
    select::highlight(&mut buf, &held, &[]);
    assert_eq!(buf[(1, 1)].modifier, Modifier::BOLD, "R1 toggle reverse");
    for column in 4..8 {
        assert_eq!(buf[(column, 1)], original[(column, 1)], "R1 blank cell");
    }
}

#[test]
fn marked_turn_chrome_never_receives_overlay_or_copied_glyphs() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 8, 3));
    buf.set_string(1, 1, "a│bc", Style::default());
    let blanks = [Rect::new(2, 1, 1, 1)];
    let held = selection(Rect::new(1, 1, 6, 1), (1, 1), (4, 1));
    let original = buf.clone();
    assert_eq!(
        select::text(&buf, &held, &blanks),
        "a bc",
        "R3 masked bytes"
    );
    select::highlight(&mut buf, &held, &blanks);
    assert_eq!(buf[(2, 1)], original[(2, 1)], "R3 no bar overlay");
    assert_ne!(buf[(3, 1)].modifier, original[(3, 1)].modifier);
}

fn fleet() -> fleet::Fleet {
    fleet::Fleet {
        home: Some("one".to_owned()),
        rows: ["one", "two"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| fleet::Row {
                name: name.to_owned(),
                index: index + 1,
                mark: Mark::Working,
                needy: false,
                counts: fleet::Counts::Unknown,
                line2: fleet::Line2::NoGoal,
                home: index == 0,
            })
            .collect(),
    }
}

#[test]
fn drawn_map_names_panel_interiors_and_excludes_header_composer_and_rules() {
    let fleet = fleet();
    let model = model::Model::new(&fleet);
    let overview = overview::Overview::default();
    let entry = SessionEntry::new("one", Status::Running);
    let lane = Lane {
        items: vec![Item {
            micros: 1,
            kind: Kind::Said {
                who: "lead".to_owned(),
            },
            body: "lane text".to_owned(),
            record: None,
        }],
        coverage: vec![],
    };
    let screen = draw::Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(&entry),
        pair: &[],
        agents: None,
        lane: &lane,
        composer: draw::Composer::ReadOnly { why: "test" },
        look: None,
        zone: None,
        now: ae::time::Timestamp::from_epoch(1),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    let map = draw::draw_map(&screen, &mut buf);
    for panel in [
        select::Panel::Lane,
        select::Panel::List,
        select::Panel::Body,
    ] {
        assert!(map.regions.iter().any(|r| r.panel == panel), "R3 {panel:?}");
    }
    for at in [
        (44, 10),
        (47, 1),
        (47, 2),
        (47, 40),
        (47, 42),
        (47, 44),
        (0, 5),
    ] {
        assert!(map.region_at(at.into()).is_none(), "R3 chrome {at:?}");
    }
    for (width, height) in [(39, 8), (40, 7), (0, 0)] {
        let mut tiny = Buffer::empty(Rect::new(0, 0, width, height));
        assert!(
            draw::draw_map(&screen, &mut tiny).regions.is_empty(),
            "R3 draw floor"
        );
    }
    let body = peek::Body::Lines(vec!["peek text".to_owned()]);
    let view = peek::View {
        session: "one",
        seat: "lead",
        body: Some(&body),
        age_secs: 0,
        back: 0,
        newer_waiting: false,
    };
    let (_, map) = draw::draw_peek_map(&view, None, &mut buf);
    assert_eq!(map.regions.len(), 1, "peek replaces all underlying panels");
    assert_eq!(map.regions[0].panel, select::Panel::Peek);
    assert_eq!(map.regions[0].area, Rect::new(2, 3, 158, 41));
    for at in [(2, 0), (2, 1), (2, 2), (0, 3), (2, 44)] {
        assert!(map.region_at(at.into()).is_none());
    }
}

#[test]
fn transition_epoch_remembers_a_switch_away_and_back() {
    let fleet = fleet();
    let mut model = model::Model::new(&fleet);
    let old = model.epoch();
    let _ = model.key(model::Key::Digit(1), &fleet, false, true);
    assert_eq!(model.epoch(), old, "same session is no transition");
    let _ = model.key(model::Key::Digit(2), &fleet, false, true);
    let _ = model.key(model::Key::Digit(1), &fleet, false, true);
    assert_eq!(model.selected(), Some("one"));
    assert_ne!(model.epoch(), old, "R6 session away/back");
    let old = model.epoch();
    let _ = model.key(model::Key::Tab, &fleet, false, true);
    let _ = model.key(model::Key::Tab, &fleet, false, true);
    assert_ne!(model.epoch(), old, "R6 tab away/back");
    let old = model.epoch();
    model.open_peek(17);
    model.close_peek();
    assert_ne!(model.epoch(), old, "R6 peek open/close");
}

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const TID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const WORD: &str = "DRAG_ALPHA_91827";
const PEEK: &str = "PEEK_ALPHA_91827";
const CANCEL: &str = "CANCEL_SENTINEL_91829";

fn cell_of(screen: &str, needle: &str) -> (u16, u16) {
    screen
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.find(needle).map(|byte| {
                (
                    u16::try_from(ratatui_core::text::Span::raw(&line[..byte]).width())
                        .expect("column"),
                    u16::try_from(row).expect("row"),
                )
            })
        })
        .expect("drawn fixture marker")
}

/// Independent ECMA-48 SGR decoder: only reverse state and cell coordinates.
fn reversed(screen: &str, at: (u16, u16)) -> bool {
    let (mut byte, mut x, mut y, mut reverse) = (0, 0_u16, 0_u16, false);
    while byte < screen.len() {
        if screen[byte..].starts_with("\x1b[") {
            let end = screen[byte + 2..].find('m').expect("SGR terminator") + byte + 2;
            let mut codes = screen[byte + 2..end].split(';');
            while let Some(code) = codes.next() {
                match code {
                    "38" | "48" | "58" => {
                        let count = match codes.next() {
                            Some("2") => 3,
                            Some("5") => 1,
                            _ => 0,
                        };
                        for _ in 0..count {
                            let _ = codes.next();
                        }
                    }
                    "0" | "" | "27" => reverse = false,
                    "7" => reverse = true,
                    _ => {}
                }
            }
            byte = end + 1;
        } else {
            let ch = screen[byte..].chars().next().expect("character");
            if ch == '\n' {
                x = 0;
                y += 1;
            } else {
                if (x, y) == at {
                    return reverse;
                }
                x += u16::try_from(ratatui_core::text::Span::raw(ch.to_string()).width())
                    .expect("cell width");
            }
            byte += ch.len_utf8();
        }
    }
    false
}

#[test]
fn sgr_colour_operands_never_change_reverse_state() {
    assert!(!reversed("\x1b[38;2;0;7;27mX", (0, 0)));
    assert!(reversed("\x1b[7;48;5;27;58;2;;0;27mX", (0, 0)));
}

struct Rig {
    scratch: super::cli::OwnedScratch,
    socket: std::path::PathBuf,
    home: String,
    app: String,
    peer: String,
}
impl Rig {
    fn tmux(&self, args: &[&str]) -> String {
        let mut all = vec!["-S".to_owned(), self.socket.display().to_string()];
        all.extend(args.iter().map(|s| (*s).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&all, self.scratch.path());
        assert!(ok, "private tmux {args:?}: {out}");
        out
    }
    fn spawn(&self, command: &str, args: &[&str]) -> String {
        let mut all = vec!["-f", "/dev/null", command, "-d", "-P", "-F", "#{pane_id}"];
        all.extend_from_slice(args);
        self.tmux(&all).trim().to_owned()
    }
    fn new() -> Self {
        let mut scratch = super::cli::OwnedScratch::root("appselect", "live");
        let socket = scratch.join("sock");
        scratch.add_tmux_server(socket.clone());
        let mut rig = Self {
            scratch,
            socket,
            home: "dlappselect".to_owned(),
            app: String::new(),
            peer: String::new(),
        };
        let lead = rig.spawn(
            "new-session",
            &["-s", &rig.home, "-x", "160", "-y", "45", "sleep", "600"],
        );
        let dir = rig.scratch.join("sessions").join(&rig.home);
        std::fs::create_dir_all(&dir).expect("session fixture");
        let store = rig.scratch.join("claude-home");
        let transcript = format!(
            r#"{{"type":"user","timestamp":"2026-10-07T20:00:00Z","message":{{"role":"user","content":"{WORD}"}}}}"#
        );
        super::board::plant_transcript(&store, "work", TID, &[transcript]);
        std::fs::write(dir.join("meta"),format!("session={}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=claude\nlaunch_id.main=tok-lead\nharness_session.main={TID}\nconfig_home.main={}\nseat.worker.0=colead\nagent_bin.worker.0=codex\nlaunch_id.worker.0=tok-peer\n",rig.home,rig.socket.display(),store.display())).expect("meta");
        std::fs::write(dir.join("events.jsonl"), "").expect("journal");
        std::fs::write(
            dir.join(".launch-attempt"),
            ae::time::Timestamp::now().epoch().to_string(),
        )
        .expect("launch stamp");
        let _ = ae::watchdog_glue::touch_beat(&dir);
        for (opt, val) in [
            ("@ae_look", "off"),
            ("@ae_session_uuid", UUID),
            ("@ae_attn_rank", "0"),
            ("@ae_attn_glyph", "idle"),
            ("@ae_motion", "off"),
            ("@ae_main_pane", lead.as_str()),
            ("default-size", "160x45"),
        ] {
            rig.tmux(&["set-option", "-t", &rig.home, opt, val]);
        }
        for (opt, val) in [("@ae_agent", "lead"), ("@ae_slot", "main")] {
            rig.tmux(&["set-option", "-p", "-t", &lead, opt, val]);
        }
        let fixture = format!(
            "printf '{PEEK}\\n'; while test ! -e '{}'; do sleep 0.1; done; printf 'PEEK_NEW_92837\\n'; exec sleep 600",
            rig.scratch.join("new-peek").display()
        );
        rig.peer = rig.spawn("new-window", &["-t", &rig.home, "sh", "-c", &fixture]);
        for (opt, val) in [("@ae_agent", "colead"), ("@ae_slot", "worker.0")] {
            rig.tmux(&["set-option", "-p", "-t", &rig.peer, opt, val]);
        }
        let facts = format!(
            "v1;{};3600;lead:idle:working:{lead};colead:idle:working:{}",
            ae::time::Timestamp::now().epoch(),
            rig.peer
        );
        rig.tmux(&["set-option", "-t", &rig.home, "@ae_agents", &facts]);
        let config = rig.scratch.join("config");
        std::fs::write(&config, "[workspace]\nchat = app\n").expect("config");
        let vars = [
            format!("AE_HOME={}", rig.scratch.path().display()),
            format!("HOME={}", rig.scratch.path().display()),
            format!("CONFIG_FILE={}", config.display()),
            "AE_TMUX_SERVER_KIND=socket".to_owned(),
            format!("AE_TMUX_SERVER={}", rig.socket.display()),
        ];
        let mut args = vec![
            "-t",
            &rig.home,
            "env",
            "-u",
            "CLAUDE_CONFIG_DIR",
            "-u",
            "CODEX_HOME",
        ];
        args.extend(vars.iter().map(String::as_str));
        args.extend([env!("CARGO_BIN_EXE_ae"), "app", &rig.home]);
        rig.app = rig.spawn("new-window", &args);
        rig.tmux(&["resize-window", "-t", &rig.app, "-x", "160", "-y", "45"]);
        rig.wait("lane ready", |screen| screen.contains(WORD));
        rig
    }
    fn screen(&self) -> String {
        self.tmux(&["capture-pane", "-p", "-t", &self.app])
    }
    fn styled(&self) -> String {
        self.tmux(&["capture-pane", "-e", "-p", "-t", &self.app])
    }
    fn raw(&self, text: &str) {
        self.tmux(&["send-keys", "-t", &self.app, "-l", "--", text]);
    }
    fn mouse(&self, kind: u8, at: (u16, u16), release: bool) {
        self.raw(&format!(
            "\x1b[<{kind};{};{}{}",
            at.0 + 1,
            at.1 + 1,
            if release { 'm' } else { 'M' }
        ));
    }
    fn wait(&self, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            let screen = self.screen();
            if met(&screen) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}\n{screen}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    fn buffer(&self) -> String {
        self.tmux(&["show-buffer"])
    }
    fn drag(&self, word: &str) -> (u16, u16) {
        let start = cell_of(&self.screen(), word);
        let end = (
            start.0 + u16::try_from(word.len() - 1).expect("ASCII width"),
            start.1,
        );
        self.mouse(0, start, false);
        self.mouse(32, end, false);
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            let styled = self.styled();
            if (start.0..=end.0).all(|x| reversed(&styled, (x, start.1))) {
                assert!(!reversed(&styled, (start.0 - 1, start.1)));
                assert!(
                    !reversed(&styled, (end.0 + 1, end.1)),
                    "R1 no over-highlight"
                );
                break;
            }
            assert!(
                Instant::now() < until,
                "R1 drawn reverse highlight missing: {styled:?}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        end
    }
}

#[test]
fn live_drag_copies_exact_cells_cancel_leaves_buffer_and_peek_resumes() {
    let rig = Rig::new();
    rig.tmux(&["select-window", "-t", &rig.app]);
    let _viewer = super::cli::tmux_attached_client(&rig.socket, &rig.home).expect("viewing client");
    rig.wait("client viewing app", |_| {
        rig.tmux(&["list-clients", "-F", "#{pane_id}"])
            .lines()
            .any(|pane| pane == rig.app)
    });
    let end = rig.drag(WORD);
    rig.mouse(0, end, true);
    rig.wait("R2 copy outcome", |s| {
        s.contains("copied 1 line to the tmux buffer; terminal clipboard")
    });
    assert_eq!(
        rig.buffer(),
        WORD,
        "R2 real default tmux buffer exact bytes"
    );
    rig.tmux(&["set-buffer", CANCEL]);
    let end = rig.drag(WORD);
    rig.raw("\x1b");
    rig.wait("R6 Esc clears overlay", |s| {
        let at = cell_of(s, WORD);
        !reversed(&rig.styled(), at)
    });
    rig.mouse(0, end, true);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(rig.buffer(), CANCEL, "cancel preserves unrelated buffer");
    let at = cell_of(&rig.screen(), WORD);
    rig.mouse(0, at, false);
    rig.mouse(0, at, true);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(rig.buffer(), CANCEL, "R4 plain click never ships selection");
    rig.raw("\t");
    rig.wait("Agents tab", |s| {
        s.lines()
            .any(|row| row.starts_with('▸') || row.starts_with('>'))
    });
    rig.raw("n");
    rig.wait("colead focused", |s| {
        s.lines().any(|row| {
            let left = row.split('│').next().unwrap_or_default();
            (left.starts_with('▸') || left.starts_with('>')) && left.contains("colead")
        })
    });
    rig.raw("v");
    rig.wait("peek opened", |s| s.contains(PEEK));
    let end = rig.drag(PEEK);
    std::fs::write(rig.scratch.join("new-peek"), "ready").expect("new capture fixture");
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        assert!(
            !rig.screen().contains("PEEK_NEW_92837"),
            "new Lines held during live selection"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    rig.mouse(0, end, true);
    rig.wait("peek copy hint visible", |s| {
        s.contains("copied 1 line to the tmux buffer; terminal clipboard")
    });
    assert_eq!(
        rig.buffer(),
        PEEK,
        "held capture copies originally highlighted cells"
    );
    rig.wait("next frame adopts newest capture", |s| {
        s.contains("PEEK_NEW_92837")
    });
    let end = rig.drag("PEEK_NEW_92837");
    rig.tmux(&["set-option", "-p", "-t", &rig.peer, "@ae_agent", "intruder"]);
    rig.wait("Gap invalidates selection", |s| {
        s.contains("refused: peek colead: its pane carries another agent")
    });
    rig.mouse(0, end, true);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(rig.buffer(), PEEK, "Gap/refusal frame change ships nothing");
}
