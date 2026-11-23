//! Acceptance for board visibility through the app's input, frame and handover.

use super::boardfull_spec::{
    MAIN_TURN, OID, ORCH, PANEL_TURN, bytes, drag_border, locate, paint, row_text, rules_at_bottom,
    sample_desk, send_key, setup_board,
};
use super::f12panel_spec::{Carrier, receiver, restart};
use super::*;
use crate::console::deskview::DeskView;

const WIDE: u16 = 200;
const HEIGHT: u16 = 35;
const DECISION: &str = "Check release choice";
const SECOND_DECISION: &str = "Confirm scope";
const UPCOMING: &str = "Review tomorrow";
const BOARD: &[u8] = b"DECIDE\nD1 | Check release choice\nD2 | Confirm scope\n";
const WITHOUT_DECISIONS: &[u8] = b"COMING\nC1 | Review tomorrow\n";

fn draw(app: &mut App, width: u16) -> Buffer {
    paint(app, width, HEIGHT)
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

fn assert_visibility_key(cells: &Buffer, label: &str) {
    let header = row_text(cells, 0);
    let key_bar = row_text(cells, cells.area.height - 1);
    assert!(header.contains(label), "header {header:?} lacks {label}");
    assert!(key_bar.contains(label), "key bar {key_bar:?} lacks {label}");
}

#[test]
fn lowercase_b_gives_the_board_columns_back_to_the_chat_and_restores_the_dragged_width() {
    for full in [false, true] {
        let (_root, mut app) = setup_board("boardhide-wide", Some(sample_desk(BOARD)));
        if full {
            bytes(&mut app, b"F");
        }
        let shown = draw(&mut app, WIDE);
        assert!(text(&shown).contains(DECISION));
        let original = app.layout.panel.expect("panel is on screen");
        assert!(original.right() < WIDE);

        drag_border(&mut app, original.right(), 145);
        let dragged = draw(&mut app, WIDE);
        assert!(rules_at_bottom(&dragged).contains(&145));
        assert_eq!(app.layout.panel.expect("dragged panel").right(), 145);

        // Hide while the board owns the keys. The chat must take over again.
        bytes(&mut app, b"B");
        let focused = draw(&mut app, WIDE);
        assert!(row_text(&focused, 0).contains("B chat"));
        bytes(&mut app, b"b");
        let hidden = draw(&mut app, WIDE);
        let output = text(&hidden);
        assert!(
            !output.contains(DECISION) && !output.contains(SECOND_DECISION),
            "both board items are hidden, full={full}"
        );
        assert!(output.contains(PANEL_TURN));
        assert_eq!(output.contains(MAIN_TURN), !full);
        let panel = app.layout.panel.expect("chat is still on screen");
        assert_eq!(panel.right(), WIDE);
        assert!(panel.width > original.width);
        assert!(!rules_at_bottom(&hidden).contains(&145));
        assert_visibility_key(&dragged, "b hide");
        assert_visibility_key(&hidden, "b show");

        if !full {
            drag_border(&mut app, panel.x, 130);
            let resized = draw(&mut app, WIDE);
            assert!(rules_at_bottom(&resized).contains(&130));
        }

        bytes(&mut app, b"b");
        let restored = draw(&mut app, WIDE);
        assert!(text(&restored).contains(DECISION));
        assert!(row_text(&restored, 0).contains("B board"));
        assert_eq!(app.layout.panel.expect("restored panel").right(), 145);
        drag_border(&mut app, 145, 155);
        let dragged_again = draw(&mut app, WIDE);
        assert!(rules_at_bottom(&dragged_again).contains(&155));
    }
}

#[test]
fn a_hidden_board_marks_only_current_decisions() {
    for full in [false, true] {
        let (_root, mut app) = setup_board("boardhide-cue", Some(sample_desk(WITHOUT_DECISIONS)));
        if full {
            bytes(&mut app, b"F");
        }
        // The COMING item stands on its own tab: focus the board and pick it.
        let _ = draw(&mut app, WIDE);
        bytes(&mut app, b"B");
        bytes(&mut app, b"2");
        let shown = draw(&mut app, WIDE);
        assert!(text(&shown).contains(UPCOMING));
        assert!(!row_text(&shown, 0).contains("desk 0"));
        bytes(&mut app, b"b");

        let empty = draw(&mut app, WIDE);
        assert!(!text(&empty).contains(UPCOMING));
        assert!(!row_text(&empty, 0).contains("desk 0"));
        update_board(&mut app, 2, BOARD);
        let waiting = draw(&mut app, WIDE);
        assert!(!text(&waiting).contains(DECISION));
        assert!(row_text(&waiting, 0).contains("desk 2"));

        update_board(&mut app, 3, WITHOUT_DECISIONS);
        let resolved = draw(&mut app, WIDE);
        assert!(!text(&resolved).contains(UPCOMING));
        assert!(!row_text(&resolved, 0).contains("desk "));
    }
}

#[test]
fn a_restart_keeps_visibility_independently_of_panel_mode_and_carrier() {
    for (carrier, full, hidden) in [
        (Carrier::Exec, false, true),
        (Carrier::Exec, true, false),
        (Carrier::Option, false, false),
        (Carrier::Option, true, true),
    ] {
        let (root, mut before) = setup_board("boardhide-restart", Some(sample_desk(BOARD)));
        if full {
            bytes(&mut before, b"F");
        }
        let _ = draw(&mut before, WIDE);
        if hidden {
            bytes(&mut before, b"b");
        }
        let (_, keep) = restart(&mut before, &root);
        let mut after = receiver(&root, keep, carrier, loader::Proof::Proven(OID.to_owned()));
        update_board(&mut after, 1, BOARD);
        let restored = draw(&mut after, WIDE);
        let where_from = format!("{carrier:?}, full={full}, hidden={hidden}");
        assert_eq!(text(&restored).contains(DECISION), !hidden, "{where_from}");
        let header = row_text(&restored, 0);
        assert!(
            header.contains(if full { "F side" } else { "F full" }),
            "{where_from}"
        );
        assert!(
            header.contains(if hidden { "b show" } else { "b hide" }),
            "{where_from}"
        );
        assert_eq!(header.contains("desk 2"), hidden, "{where_from}");

        bytes(&mut after, b"b");
        let toggled = draw(&mut after, WIDE);
        assert_eq!(text(&toggled).contains(DECISION), hidden, "{where_from}");
    }
}

#[test]
fn a_narrow_shown_board_still_uses_uppercase_b_and_escape_without_changing_the_panel_geometry() {
    for (width, full) in [(120, false), (60, true)] {
        let (_root, mut app) = setup_board("boardhide-narrow", Some(sample_desk(BOARD)));
        if full {
            bytes(&mut app, b"F");
        }
        let chat = draw(&mut app, width);
        let panel = app.layout.panel.expect("narrow panel");
        assert!(text(&chat).contains(PANEL_TURN) && !text(&chat).contains(DECISION));
        assert!(row_text(&chat, 0).contains("desk 2"));

        bytes(&mut app, b"B");
        let overlay = draw(&mut app, width);
        assert_eq!(app.layout.panel, Some(panel));
        assert!(!text(&overlay).contains(PANEL_TURN) && text(&overlay).contains(DECISION));
        send_key(&mut app, Key::Escape);
        let chat = draw(&mut app, width);
        assert_eq!(app.layout.panel, Some(panel));
        assert!(text(&chat).contains(PANEL_TURN) && !text(&chat).contains(DECISION));

        bytes(&mut app, b"B");
        let _ = draw(&mut app, width);
        bytes(&mut app, b"b");
        let hidden = draw(&mut app, width);
        assert_eq!(app.layout.panel, Some(panel));
        assert!(text(&hidden).contains(PANEL_TURN) && !text(&hidden).contains(DECISION));
        assert!(row_text(&hidden, 0).contains("desk 2"));
        bytes(&mut app, b"B");
        assert!(
            !text(&draw(&mut app, width)).contains(DECISION),
            "B respects b"
        );

        bytes(&mut app, b"b");
        let shown = draw(&mut app, width);
        assert!(text(&shown).contains(PANEL_TURN) && !text(&shown).contains(DECISION));
        bytes(&mut app, b"B");
        assert!(text(&draw(&mut app, width)).contains(DECISION));
    }
}

#[test]
fn b_and_shift_b_remain_text_in_the_composer_and_b_is_inert_without_a_board() {
    let (_root, mut app) = setup_board("boardhide-compose", Some(sample_desk(BOARD)));
    let _ = draw(&mut app, WIDE);
    bytes(&mut app, b"i");
    bytes(&mut app, b"bB");
    assert_eq!(app.writer_input().map(Input::draft).as_deref(), Some("bB"));
    assert!(text(&draw(&mut app, WIDE)).contains(DECISION));
    send_key(&mut app, Key::Escape);
    bytes(&mut app, b"?");
    let help = text(&draw(&mut app, WIDE));
    assert!(
        help.lines().any(|line| {
            let words = line.split_whitespace().collect::<Vec<_>>();
            words.contains(&"b") && line.contains("board") && line.contains("hide")
        }),
        "Keys tab explains b\n{help}"
    );

    let (_root, mut app) = setup_board("boardhide-absent", Some(DeskView::Absent));
    let absent = draw(&mut app, WIDE);
    bytes(&mut app, b"b");
    let after = draw(&mut app, WIDE);
    assert_eq!(row_text(&after, 0), row_text(&absent, 0));
    update_board(&mut app, 2, BOARD);
    assert!(text(&draw(&mut app, WIDE)).contains(DECISION));
}

#[test]
fn a_press_made_on_the_board_before_hiding_cannot_reach_the_replacement_frame() {
    let (_root, mut app) = setup_board("boardhide-press", Some(sample_desk(BOARD)));
    bytes(&mut app, b"F");
    let shown = draw(&mut app, WIDE);
    let (column, row) = locate(&shown, DECISION).expect("decision on the old board");
    let origin = Instant::now();
    bytes(&mut app, b"b");
    let _ = draw(&mut app, WIDE);
    app.flash = None;
    let press = Key::Mouse(Mouse {
        kind: MouseKind::Click,
        column,
        row,
    });
    assert!(take_keys(&mut app, vec![(press, origin)]).is_some());
    assert_eq!(
        app.flash.as_ref().map(|(line, _)| line.as_str()),
        Some("click not taken: its frame is gone")
    );
}
