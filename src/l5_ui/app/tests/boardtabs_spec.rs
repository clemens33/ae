//! Frozen acceptance for boardtabs: per-section board tabs with counts,
//! keyboard and mouse switching, narrow-frame label shortening, restart
//! persistence, unchanged hidden marker and `/board` text, and per-tab scroll.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "frozen acceptance owns scratch state roots and terminal buffers"
)]

use std::fmt::Write as _;

use super::boardfull_spec::{
    MAIN_TURN, OID, ORCH, PANEL_TURN, bytes, drag_border, paint, row_text, rules_at_bottom,
    sample_desk, send_key, setup_board,
};
use super::f12panel_spec::{Carrier, receiver, restart};
use super::*;
use crate::app::handover;
use crate::console::deskview::{self, DeskView};
use crate::console::link;

const WIDE: u16 = 200;
const HEIGHT: u16 = 35;

const ALL_SECTIONS: &[u8] = b"\
DECIDE
D1 | First decision
D2 | Second decision
COMING
C1 | Upcoming work
RUNNING
R1 | Active slice
LINKS
L1 | Reference link | https://example.test/ref
";

const FIVE_SECTIONS: &[u8] = b"\
DECIDE
D1 | Decide item
COMING
C1 | Coming item
RUNNING
R1 | Running item
WAITING ON OTHERS
W1 | Waiting item
LINKS
L1 | Links item
";

const HANDOVER_WIRE: &str = "v1|%7|2026.10.99|1760259000|web|6f1c2b7e-0d3a-4c58-9a2e-5b1d7c9e0a11|agents|colead|31|9|2|0|1|4";
const HANDOVER_ORCH_ID: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";

fn click(app: &mut App, (column, row): (u16, u16)) {
    for kind in [MouseKind::Click, MouseKind::Release] {
        let press = Key::Mouse(Mouse { kind, column, row });
        assert!(take_keys(app, vec![(press, Instant::now())]).is_some());
    }
}

fn wheel(app: &mut App, up: bool, (column, row): (u16, u16)) {
    let kind = if up {
        MouseKind::WheelUp
    } else {
        MouseKind::WheelDown
    };
    let ev = Key::Mouse(Mouse { kind, column, row });
    assert!(take_keys(app, vec![(ev, Instant::now())]).is_some());
}

fn update_board(app: &mut App, seq: u64, board: &[u8]) {
    app.answer(Answer::View(ViewRead {
        name: ORCH.to_owned(),
        id: OID.to_owned(),
        seq,
        lane: Lane {
            items: vec![said(PANEL_TURN, 0)],
            coverage: Vec::new(),
        },
        needs: None,
        desk: Some(sample_desk(board)),
        roster: None,
    }));
}

fn locate_in_rect(cells: &Buffer, rect: Rect, needle: &str) -> Option<(u16, u16)> {
    let width = u16::try_from(needle.len()).expect("short ASCII needle");
    if rect.width < width {
        return None;
    }
    (rect.y..rect.bottom()).find_map(|y| {
        (rect.x..=rect.right().saturating_sub(width)).find_map(|x| {
            let candidate: String = (x..x + width).map(|cx| cells[(cx, y)].symbol()).collect();
            (candidate == needle).then_some((x, y))
        })
    })
}

fn board_rect_for(app: &App, cells: &Buffer) -> Rect {
    if let Some(panel) = app.layout.panel
        && panel.right() < cells.area.width
    {
        return Rect::new(
            panel.right(),
            0,
            cells.area.width - panel.right(),
            cells.area.height,
        );
    }
    app.layout.panel.expect("board or panel rect")
}

fn rect_text(cells: &Buffer, rect: Rect) -> String {
    (rect.y..rect.bottom())
        .map(|y| {
            (rect.x..rect.right())
                .map(|x| cells[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Find the single row inside `rect` carrying all five tab labels (in any of
/// the three approved forms: full, short, or letter) for the given counts.
fn find_tab_row(cells: &Buffer, rect: Rect, counts: [usize; 5]) -> (u16, [String; 5]) {
    let forms = [
        [
            format!("DECIDE {}", counts[0]),
            format!("COMING {}", counts[1]),
            format!("RUNNING {}", counts[2]),
            format!("WAITING {}", counts[3]),
            format!("LINKS {}", counts[4]),
        ],
        [
            format!("DEC {}", counts[0]),
            format!("COM {}", counts[1]),
            format!("RUN {}", counts[2]),
            format!("WAIT {}", counts[3]),
            format!("LINK {}", counts[4]),
        ],
        [
            format!("D{}", counts[0]),
            format!("C{}", counts[1]),
            format!("R{}", counts[2]),
            format!("W{}", counts[3]),
            format!("L{}", counts[4]),
        ],
    ];
    for labels in forms {
        for y in rect.y..rect.bottom() {
            let row_rect = Rect::new(rect.x, y, rect.width, 1);
            if labels
                .iter()
                .all(|label| locate_in_rect(cells, row_rect, label).is_some())
            {
                return (y, labels);
            }
        }
    }
    panic!(
        "no single-row tab bar for counts {counts:?} inside {rect:?}:\n{}",
        rect_text(cells, rect)
    );
}

fn setup_two_sessions_board(tag: &str, desk: DeskView) -> (Root, App) {
    let root = Root::new(tag);
    let dir = root.0.join("sessions").join(ORCH);
    std::fs::create_dir_all(&dir).expect("private orchestrator dir");
    let overlay = root.0.join("orchestrator.config");
    std::fs::write(&overlay, "[workspace]\norchestrator = true\n").expect("orchestrator overlay");
    let meta = format!(
        "schema=2\nsession_id={OID}\nlayout=lead-pair\nseat.main=lead\nmeta_agent=true\nlocal_config={}\n",
        overlay.display()
    );
    std::fs::write(dir.join("meta"), meta).expect("orchestrator meta");
    let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
    let mut app = app(Some("api"));
    refold(
        &mut app,
        &reader,
        &root,
        &[
            ("api", Status::Running),
            ("web", Status::Running),
            (ORCH, Status::Running),
        ],
    );
    app.lane = Rc::new(Lane {
        items: vec![said(MAIN_TURN, 0)],
        coverage: Vec::new(),
    });
    app.loading = false;
    app.answer(Answer::Orchestrator(loader::Proof::Proven(OID.to_owned())));
    bytes(&mut app, b"O");
    app.answer(Answer::View(ViewRead {
        name: ORCH.to_owned(),
        id: OID.to_owned(),
        seq: 1,
        lane: Lane {
            items: vec![said(PANEL_TURN, 0)],
            coverage: Vec::new(),
        },
        needs: None,
        desk: Some(desk),
        roster: None,
    }));
    (root, app)
}

#[test]
fn default_tab_is_decide_with_counts_across_120_161_and_200_side_and_zoomed() {
    for (width, full) in [
        (120, false),
        (120, true),
        (161, false),
        (161, true),
        (200, false),
        (200, true),
    ] {
        let (_root, mut app) = setup_board("bt-render", Some(sample_desk(ALL_SECTIONS)));
        if full {
            bytes(&mut app, b"F");
        }
        let _ = paint(&mut app, width, HEIGHT);
        if width == 120 && !full {
            bytes(&mut app, b"B");
        }
        let cells = paint(&mut app, width, HEIGHT);
        let board = board_rect_for(&app, &cells);
        let body = rect_text(&cells, board);
        assert!(
            body.contains("First decision") && body.contains("Second decision"),
            "DECIDE items shown by default at {width} full={full}:\n{body}"
        );
        assert!(
            !body.contains("Upcoming work")
                && !body.contains("Active slice")
                && !body.contains("Reference link"),
            "other sections hidden on default DECIDE tab at {width} full={full}:\n{body}"
        );
        let (tab_y, _) = find_tab_row(&cells, board, [2, 1, 1, 0, 1]);
        let (_, header_y) = locate_in_rect(&cells, board, "BOARD").expect("BOARD header");
        assert!(
            tab_y > header_y,
            "tab bar sits below header at {width} full={full}"
        );
    }

    // Empty DECIDE still opens on DECIDE with count 0 and `no items`.
    let (_root, mut empty_decide) = setup_board(
        "bt-empty-decide",
        Some(sample_desk(b"RUNNING\nR1 | Only running\n")),
    );
    let cells = paint(&mut empty_decide, WIDE, HEIGHT);
    let board = board_rect_for(&empty_decide, &cells);
    let body = rect_text(&cells, board);
    let _ = find_tab_row(&cells, board, [0, 0, 1, 0, 0]);
    assert!(
        body.contains("no items"),
        "empty DECIDE says no items:\n{body}"
    );
    assert!(
        !body.contains("Only running"),
        "RUNNING item stays hidden while DECIDE is selected:\n{body}"
    );
}

#[test]
fn keys_switch_tabs_only_while_board_has_focus_and_wrap() {
    let (_root, mut app) = setup_two_sessions_board("bt-keys", sample_desk(FIVE_SECTIONS));
    let _ = paint(&mut app, WIDE, HEIGHT);
    assert_eq!(app.model.selected(), Some("api"));

    // Unfocused: digit 2 selects the second session, not COMING.
    bytes(&mut app, b"2");
    let unfocused = paint(&mut app, WIDE, HEIGHT);
    assert_eq!(app.model.selected(), Some("web"));
    assert!(text(&unfocused).contains("Decide item"));
    assert!(!text(&unfocused).contains("Coming item"));

    // Focus board: 1-5 select directly, 6-9 are inert.
    bytes(&mut app, b"B");
    for (digit, expected, absent) in [
        (b"2".as_slice(), "Coming item", "Decide item"),
        (b"3", "Running item", "Coming item"),
        (b"4", "Waiting item", "Running item"),
        (b"5", "Links item", "Waiting item"),
        (b"1", "Decide item", "Links item"),
    ] {
        bytes(&mut app, digit);
        let cells = paint(&mut app, WIDE, HEIGHT);
        let shown = text(&cells);
        assert!(shown.contains(expected), "digit {digit:?} shows {expected}");
        assert!(!shown.contains(absent), "digit {digit:?} hides {absent}");
    }
    assert_eq!(
        app.model.selected(),
        Some("web"),
        "board digits do not change session selection"
    );

    for inert in [b"6", b"7", b"8", b"9"] {
        let redraw = take_keys(&mut app, vec![(Key::Text(inert.to_vec()), Instant::now())]);
        assert_eq!(redraw, Some(false), "digit {inert:?} is inert on board");
    }
    assert!(text(&paint(&mut app, WIDE, HEIGHT)).contains("Decide item"));

    // Tab wraps forward through all 5 sections back to DECIDE.
    for expected in [
        "Coming item",
        "Running item",
        "Waiting item",
        "Links item",
        "Decide item",
    ] {
        send_key(&mut app, Key::Tab);
        assert!(
            text(&paint(&mut app, WIDE, HEIGHT)).contains(expected),
            "Tab reaches {expected}"
        );
    }

    // Right wraps forward; Left wraps backward.
    send_key(&mut app, Key::Right);
    assert!(text(&paint(&mut app, WIDE, HEIGHT)).contains("Coming item"));
    send_key(&mut app, Key::Left);
    assert!(text(&paint(&mut app, WIDE, HEIGHT)).contains("Decide item"));
    send_key(&mut app, Key::Left);
    assert!(text(&paint(&mut app, WIDE, HEIGHT)).contains("Links item"));
    send_key(&mut app, Key::Left);
    assert!(text(&paint(&mut app, WIDE, HEIGHT)).contains("Waiting item"));

    // Return focus to chat: 1 selects first session again, board stays on WAITING.
    bytes(&mut app, b"B");
    bytes(&mut app, b"1");
    let back = paint(&mut app, WIDE, HEIGHT);
    assert_eq!(app.model.selected(), Some("api"));
    assert!(text(&back).contains("Waiting item"));

    // Unreadable board draws no tab bar and ignores tab keys while focused.
    let unreadable = DeskView::Unreadable("cut".to_owned());
    let (_r2, mut bad) = setup_board("bt-unreadable", Some(unreadable));
    let _ = paint(&mut bad, WIDE, HEIGHT);
    bytes(&mut bad, b"B");
    for key in [Key::Text(b"2".to_vec()), Key::Tab, Key::Left, Key::Right] {
        assert_eq!(
            take_keys(&mut bad, vec![(key, Instant::now())]),
            Some(false),
            "unreadable board ignores tab keys"
        );
    }
}

#[test]
fn clicking_tab_labels_switches_tabs_unfocused_composing_and_narrow() {
    let (_root, mut app) = setup_board("bt-click", Some(sample_desk(FIVE_SECTIONS)));
    let cells = paint(&mut app, WIDE, HEIGHT);
    let board = board_rect_for(&app, &cells);
    let (tab_y, labels) = find_tab_row(&cells, board, [1, 1, 1, 1, 1]);
    let row_rect = Rect::new(board.x, tab_y, board.width, 1);

    // Unfocused click on RUNNING switches tab and gives board focus.
    let run_pos = locate_in_rect(&cells, row_rect, &labels[2]).expect("RUNNING tab label");
    click(&mut app, run_pos);
    let after_run = paint(&mut app, WIDE, HEIGHT);
    assert!(text(&after_run).contains("Running item"));
    assert!(
        row_text(&after_run, 0).contains("B chat"),
        "clicking tab while unfocused gives board focus"
    );

    // Unfocus board, start composing, click LINKS: tab switches, composer stays active.
    bytes(&mut app, b"B");
    bytes(&mut app, b"i");
    bytes(&mut app, b"draft");
    assert!(app.composing());
    let while_writing = paint(&mut app, WIDE, HEIGHT);
    let links_pos = locate_in_rect(&while_writing, row_rect, &labels[4]).expect("LINKS tab label");
    click(&mut app, links_pos);
    let after_links = paint(&mut app, WIDE, HEIGHT);
    assert!(text(&after_links).contains("Links item"));
    assert!(
        app.composing(),
        "tab click while composing keeps composer active"
    );
    assert_eq!(
        app.writer_input().map(Input::draft).as_deref(),
        Some("draft")
    );
    send_key(&mut app, Key::Escape);

    // Narrow side-by-side frame (120 cols): open board with B, click COMING tab.
    let _ = paint(&mut app, 120, HEIGHT);
    bytes(&mut app, b"B");
    let narrow = paint(&mut app, 120, HEIGHT);
    let narrow_board = board_rect_for(&app, &narrow);
    let (narrow_y, narrow_labels) = find_tab_row(&narrow, narrow_board, [1, 1, 1, 1, 1]);
    let narrow_row = Rect::new(narrow_board.x, narrow_y, narrow_board.width, 1);
    let coming_pos =
        locate_in_rect(&narrow, narrow_row, &narrow_labels[1]).expect("COMING tab label");
    click(&mut app, coming_pos);
    let after_coming = paint(&mut app, 120, HEIGHT);
    assert!(text(&after_coming).contains("Coming item"));
    assert!(!text(&after_coming).contains("Links item"));
}

#[test]
fn narrow_frames_shorten_tab_labels_without_wrapping_down_to_floor() {
    let (_root, mut app) = setup_board("bt-narrow-labels", Some(sample_desk(ALL_SECTIONS)));
    // Default 36-col board (room = 34): short form on one row.
    let default_cells = paint(&mut app, WIDE, HEIGHT);
    let default_board = board_rect_for(&app, &default_cells);
    let (y, labels) = find_tab_row(&default_cells, default_board, [2, 1, 1, 0, 1]);
    assert_eq!(labels, ["DEC 2", "COM 1", "RUN 1", "WAIT 0", "LINK 1"]);
    assert!(row_text(&default_cells, y).contains('·'));

    // Drag board wide enough for full labels (width 52 -> room 50 >= 45).
    let border = *rules_at_bottom(&default_cells)
        .last()
        .expect("board border");
    drag_border(&mut app, border, WIDE - 52);
    let wide_cells = paint(&mut app, WIDE, HEIGHT);
    let wide_board = board_rect_for(&app, &wide_cells);
    let (_, wide_labels) = find_tab_row(&wide_cells, wide_board, [2, 1, 1, 0, 1]);
    assert_eq!(
        wide_labels,
        ["DECIDE 2", "COMING 1", "RUNNING 1", "WAITING 0", "LINKS 1"]
    );

    // Drag board down to 24-col floor (room = 22): letter form on one row.
    let border = *rules_at_bottom(&wide_cells).last().expect("board border");
    drag_border(&mut app, border, WIDE - 24);
    let floor_cells = paint(&mut app, WIDE, HEIGHT);
    let floor_board = board_rect_for(&app, &floor_cells);
    assert_eq!(floor_board.width, 24);
    let (_, floor_labels) = find_tab_row(&floor_cells, floor_board, [2, 1, 1, 0, 1]);
    assert_eq!(floor_labels, ["D2", "C1", "R1", "W0", "L1"]);

    // 2-digit count at 24-col floor (MAX_ITEMS = 16): tight letter form fits without wrapping.
    let mut ten_decide = String::from("DECIDE\n");
    for n in 1..=10 {
        let _ = writeln!(ten_decide, "D{n} | Decision {n}");
    }
    ten_decide.push_str("COMING\nC1 | One coming\n");
    update_board(&mut app, 2, ten_decide.as_bytes());
    let two_digit = paint(&mut app, WIDE, HEIGHT);
    let floor_board = board_rect_for(&app, &two_digit);
    assert_eq!(floor_board.width, 24);
    let (_, tight_labels) = find_tab_row(&two_digit, floor_board, [10, 1, 0, 0, 0]);
    assert_eq!(tight_labels, ["D10", "C1", "R0", "W0", "L0"]);
}

#[test]
fn chosen_tab_survives_restart_across_carriers_modes_and_wire_validation() {
    for (carrier, full, hidden, digit, expected) in [
        (Carrier::Exec, false, false, b"3".as_slice(), "Running item"),
        (Carrier::Exec, true, false, b"5", "Links item"),
        (Carrier::Option, false, true, b"2", "Coming item"),
        (Carrier::Option, true, false, b"4", "Waiting item"),
    ] {
        let (root, mut before) = setup_board("bt-restart", Some(sample_desk(FIVE_SECTIONS)));
        if full {
            bytes(&mut before, b"F");
        }
        let _ = paint(&mut before, WIDE, HEIGHT);
        bytes(&mut before, b"B");
        bytes(&mut before, digit);
        if hidden {
            bytes(&mut before, b"b");
        }
        let (_, keep) = restart(&mut before, &root);
        let mut after = receiver(&root, keep, carrier, loader::Proof::Proven(OID.to_owned()));
        update_board(&mut after, 1, FIVE_SECTIONS);
        let restored = paint(&mut after, WIDE, HEIGHT);
        let ctx = format!("{carrier:?} full={full} hidden={hidden} digit={digit:?}");
        if hidden {
            assert!(!text(&restored).contains(expected), "{ctx}");
            bytes(&mut after, b"b");
            let shown = paint(&mut after, WIDE, HEIGHT);
            assert!(text(&shown).contains(expected), "unhidden keeps tab: {ctx}");
            assert!(!text(&shown).contains("Decide item"), "{ctx}");
        } else {
            assert!(
                text(&restored).contains(expected),
                "restart keeps tab: {ctx}"
            );
            assert!(!text(&restored).contains("Decide item"), "{ctx}");
        }
    }

    // Handover wire canonicality: round-trips valid forms and refuses non-canonical forms.
    let open_panel = format!("side,{HANDOVER_ORCH_ID},0,0");
    let v2 = HANDOVER_WIRE.replacen("v1|", "v2|", 1);
    let v3 = HANDOVER_WIRE.replacen("v1|", "v3|", 1);
    for base in [
        HANDOVER_WIRE.to_owned(),
        format!("{v2}||{open_panel}"),
        format!("{v3}|56|{open_panel}"),
    ] {
        for suffix in [
            "tab=coming",
            "44,tab=links",
            "hidden,tab=running",
            "44,hidden,tab=waiting",
        ] {
            let line = format!("{base}|desk={suffix}");
            let parsed = handover::parse(&line)
                .unwrap_or_else(|| panic!("canonical tab wire must parse: {line}"));
            assert_eq!(parsed.render(), line);
        }
    }

    let overview = HANDOVER_WIRE.replace("|agents|", "|overview|");
    for bad in [
        format!("{HANDOVER_WIRE}|desk=tab="),
        format!("{HANDOVER_WIRE}|desk=tab=decide"),
        format!("{HANDOVER_WIRE}|desk=tab=Coming"),
        format!("{HANDOVER_WIRE}|desk=tab=coming,hidden"),
        format!("{HANDOVER_WIRE}|desk=44,tab=coming,hidden"),
        format!("{HANDOVER_WIRE}|desk=tab=coming,tab=links"),
        format!("{HANDOVER_WIRE}|desk=hidden,44,tab=links"),
        format!("{overview}|desk=tab=coming"),
    ] {
        assert!(handover::parse(&bad).is_none(), "must refuse {bad:?}");
    }
}

#[test]
fn hidden_marker_always_counts_decide_regardless_of_selected_tab() {
    let raw = b"DECIDE\nD1 | First\nD2 | Second\nRUNNING\nR1 | One\nR2 | Two\nR3 | Three\n";
    let (_root, mut app) = setup_board("bt-hidden-cue", Some(sample_desk(raw)));
    let _ = paint(&mut app, WIDE, HEIGHT);
    bytes(&mut app, b"B");
    bytes(&mut app, b"3");
    assert!(text(&paint(&mut app, WIDE, HEIGHT)).contains("Three"));

    bytes(&mut app, b"b");
    let hidden = paint(&mut app, WIDE, HEIGHT);
    let header = row_text(&hidden, 0);
    assert!(
        header.contains("desk 2") && !header.contains("desk 3"),
        "hidden marker counts DECIDE (2), not selected RUNNING (3): {header:?}"
    );

    // Unhiding returns to the previously selected RUNNING tab.
    bytes(&mut app, b"b");
    assert!(text(&paint(&mut app, WIDE, HEIGHT)).contains("Three"));

    // Zero DECIDE items with RUNNING selected: hidden header shows no `desk ` marker.
    let no_decide = b"RUNNING\nR1 | One\nR2 | Two\nR3 | Three\n";
    update_board(&mut app, 2, no_decide);
    bytes(&mut app, b"b");
    let hidden_empty = paint(&mut app, WIDE, HEIGHT);
    assert!(
        !row_text(&hidden_empty, 0).contains("desk "),
        "no DECIDE items means no hidden desk marker"
    );
}

#[test]
fn scrolling_works_inside_a_tab_with_fixed_header_and_tab_bar_and_resets_on_switch() {
    let web_url = "https://example.test/tab-scroll-link";
    let mut raw = String::from("DECIDE\nD1 | Single decision\nRUNNING\nR1 | Top running item\n");
    for n in 2..=13 {
        let _ = writeln!(
            raw,
            "R{n} | Middle running step {n:02} with extra wrapped detail text"
        );
    }
    let _ = writeln!(raw, "R14 | Bottom running item | {web_url}");

    let (_root, mut app) = setup_board("bt-scroll", Some(sample_desk(raw.as_bytes())));
    let _ = paint(&mut app, WIDE, 20);
    bytes(&mut app, b"B");
    bytes(&mut app, b"3");
    let top = paint(&mut app, WIDE, 20);
    let board = board_rect_for(&app, &top);
    let (tab_y, labels) = find_tab_row(&top, board, [1, 0, 14, 0, 0]);
    let (_, header_y) = locate_in_rect(&top, board, "BOARD").expect("BOARD header");
    assert!(rect_text(&top, board).contains("Top running item"));
    assert!(!rect_text(&top, board).contains("Bottom running item"));

    // Scroll to bottom with PgDn and wheel: header and tab bar stay fixed.
    for _ in 0..8 {
        send_key(&mut app, Key::PageDown);
    }
    let board_x = board.x + 2;
    for _ in 0..10 {
        wheel(&mut app, false, (board_x, 5));
    }
    let bottom = paint(&mut app, WIDE, 20);
    let bottom_body = rect_text(&bottom, board);
    assert!(bottom_body.contains("Bottom running item"));
    assert!(!bottom_body.contains("Top running item"));
    assert_eq!(
        locate_in_rect(&bottom, board, "BOARD").map(|(_, y)| y),
        Some(header_y),
        "header stays fixed while scrolling"
    );
    let (after_tab_y, _) = find_tab_row(&bottom, board, [1, 0, 14, 0, 0]);
    assert_eq!(after_tab_y, tab_y, "tab bar stays fixed while scrolling");

    // Link on scrolled tab body is clickable.
    let run = app
        .layout
        .links
        .iter()
        .find(|run| run.url == web_url)
        .expect("scrolled tab link run");
    let link_pos = (run.x, run.y);
    app.flash = None;
    click(&mut app, link_pos);
    assert_eq!(
        app.flash.as_ref().map(|(line, _)| line.as_str()),
        Some(link::open_hint())
    );

    // Re-selecting RUNNING preserves scroll position and owes no redraw.
    let same_key = take_keys(&mut app, vec![(Key::Text(b"3".to_vec()), Instant::now())]);
    assert_eq!(same_key, Some(false), "re-selecting active tab is a no-op");
    let row_rect = Rect::new(board.x, tab_y, board.width, 1);
    let run_pos = locate_in_rect(&bottom, row_rect, &labels[2]).expect("RUNNING tab");
    click(&mut app, run_pos);
    assert!(
        rect_text(&paint(&mut app, WIDE, 20), board).contains("Bottom running item"),
        "re-clicking active tab keeps scroll position"
    );

    // Switching away and back resets scroll to top.
    bytes(&mut app, b"1");
    assert!(rect_text(&paint(&mut app, WIDE, 20), board).contains("Single decision"));
    bytes(&mut app, b"3");
    let reset = paint(&mut app, WIDE, 20);
    assert!(rect_text(&reset, board).contains("Top running item"));
    assert!(!rect_text(&reset, board).contains("Bottom running item"));
}

#[test]
fn desk_record_and_chat_board_text_remain_unchanged() {
    let view = sample_desk(FIVE_SECTIONS);
    let out = deskview::board_text(&view, None);
    for heading in ["DECIDE", "COMING", "RUNNING", "WAITING ON OTHERS", "LINKS"] {
        assert!(out.contains(heading), "/board keeps {heading}:\n{out}");
    }
    for item in [
        "Decide item",
        "Coming item",
        "Running item",
        "Waiting item",
        "Links item",
    ] {
        assert!(out.contains(item), "/board keeps {item}:\n{out}");
    }
}
