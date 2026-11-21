//! Frozen acceptance for boarddrag: the board column's left border grabs,
//! drags within the frame's floors, keeps its width while hidden, and
//! survives a restart handover without changing any no-desk wire.

use super::*;
use crate::app::handover;

const ORCH: &str = crate::orchestrator::ORCHESTRATOR_SESSION;
const OID: &str = "0199c0de-2222-4890-abcd-ef0123456789";

pub(super) fn board_rules(app: &mut App, width: u16) -> Vec<u16> {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, 45));
    app.frame(&mut buf);
    app.frame_shown(Instant::now());
    (0..width)
        .filter(|&x| buf[(x, 44)].symbol() == "│")
        .collect()
}

fn deliver_board(app: &mut App, seq: u64) {
    app.answer(Answer::View(ViewRead {
        name: ORCH.to_owned(),
        id: OID.to_owned(),
        seq,
        lane: Lane {
            items: vec![said("A chat turn beside the board", 0)],
            coverage: Vec::new(),
        },
        needs: None,
        desk: Some(crate::console::deskview::DeskView::Shown(
            crate::desk::Desk {
                updated: Timestamp::now(),
                items: crate::desk::parse_input(b"DECIDE\nD1 | Confirm next task\n")
                    .expect("a valid board"),
            },
        )),
        roster: None,
    }));
}

pub(super) fn shown_board() -> App {
    let mut app = app(None);
    two(&mut app);
    app.orch.proof = loader::Proof::Proven(OID.to_owned());
    app.dirs.insert(ORCH.to_owned(), PathBuf::from(ORCH));
    app.ids.insert(ORCH.to_owned(), OID.to_owned());
    app.open_panel(false);
    deliver_board(&mut app, 1);
    app
}

pub(super) fn drag_rule(app: &mut App, from: u16, to: u16) {
    for (kind, column) in [
        (MouseKind::Click, from),
        (MouseKind::Drag, to),
        (MouseKind::Release, to),
    ] {
        app.mouse_made(
            Mouse {
                kind,
                column,
                row: 5,
            },
            Instant::now(),
        );
    }
}

#[test]
fn a_dragged_board_clamps_to_its_floor_and_keeps_both_chat_columns_at_40() {
    let mut app = shown_board();
    assert_eq!(board_rules(&mut app, 200), [44, 105, 164]);

    // Near-cell grab from border - 1 (inside layout.panel) routes to the border, not selection.
    // Pointer-to-rule offset is preserved: 163 -> 159 moves the rule from 164 to 160.
    drag_rule(&mut app, 163, 159);
    assert!(
        app.select.is_none(),
        "border - 1 grabs the rule, not panel text"
    );
    assert_eq!(board_rules(&mut app, 200), [44, 103, 160]);

    // Drag right past the 24-column floor: clamps at 200 - 24 = 176.
    drag_rule(&mut app, 160, 185);
    assert_eq!(board_rules(&mut app, 200), [44, 111, 176]);

    // Drag left past the two 40-column chat floors (45 + 40 + 40 = 125): clamps at 125.
    drag_rule(&mut app, 176, 100);
    assert_eq!(board_rules(&mut app, 200), [44, 85, 125]);
}

#[test]
fn a_dragged_board_width_survives_narrowing_full_panel_and_close() {
    let mut app = shown_board();
    assert_eq!(board_rules(&mut app, 200), [44, 105, 164]);

    // Drag board to 24 columns (rule at 176).
    drag_rule(&mut app, 164, 176);
    assert_eq!(board_rules(&mut app, 200), [44, 111, 176]);

    // At 149 wide (45 + 40 + 40 + 24), the 24-column dragged board still fits.
    assert_eq!(board_rules(&mut app, 149), [44, 85, 125]);
    // At 148 wide, even 24 does not fit: board hides, two chat columns remain.
    assert_eq!(board_rules(&mut app, 148), [44, 97]);
    // Widening back to 200 restores the 24-column board.
    assert_eq!(board_rules(&mut app, 200), [44, 111, 176]);

    // Drag board to 50 columns (rule at 150), flip full and back, close and reopen.
    drag_rule(&mut app, 176, 150);
    assert_eq!(board_rules(&mut app, 200), [44, 98, 150]);
    app.flip_panel();
    assert!(board_rules(&mut app, 200).is_empty());
    app.flip_panel();
    assert_eq!(board_rules(&mut app, 200), [44, 98, 150]);
    app.close_panel();
    assert_eq!(board_rules(&mut app, 200), [44]);
    app.open_panel(false);
    deliver_board(&mut app, 2);
    assert_eq!(board_rules(&mut app, 200), [44, 98, 150]);
}

#[test]
fn a_dragged_board_width_crosses_restart_and_preserves_undragged_wires() {
    let mut undragged = shown_board();
    let _ = board_rules(&mut undragged, 200);
    let plain = undragged
        .handover("%1", crate::VERSION, 100)
        .expect("handover")
        .render();
    assert!(
        !plain.contains("desk="),
        "undragged board adds no desk field: {plain}"
    );
    assert_eq!(
        handover::parse(&plain).map(|h| h.render()).as_deref(),
        Some(plain.as_str())
    );

    // Dragged board (50 wide) survives v2 (open panel) and v1 (closed panel) restarts.
    drag_rule(&mut undragged, 164, 150);
    assert_eq!(board_rules(&mut undragged, 200), [44, 98, 150]);
    let v2_text = undragged
        .handover("%1", crate::VERSION, 100)
        .expect("v2 handover")
        .render();
    assert!(
        v2_text.ends_with("|desk=50"),
        "v2 carries desk=50: {v2_text}"
    );
    let v2_parsed = handover::parse(&v2_text).expect("v2 with desk parses");
    assert_eq!(v2_parsed.render(), v2_text);

    let mut next = shown_board();
    next.model.adopt(&next.fleet, &v2_parsed.view);
    assert_eq!(board_rules(&mut next, 200), [44, 98, 150]);
    // Temporary clamp at 160 (available 35) never overwrites stored 50.
    assert_eq!(board_rules(&mut next, 160), [44, 85, 125]);
    assert_eq!(
        next.handover("%1", crate::VERSION, 100)
            .expect("kept 50")
            .render(),
        v2_text
    );

    // Closing the panel keeps the board width across a v1 restart.
    next.close_panel();
    let v1_text = next
        .handover("%1", crate::VERSION, 100)
        .expect("v1 handover")
        .render();
    assert!(
        v1_text.ends_with("|desk=50"),
        "v1 carries desk=50: {v1_text}"
    );
    let v1_parsed = handover::parse(&v1_text).expect("v1 with desk parses");
    assert_eq!(v1_parsed.render(), v1_text);

    let mut reopened = shown_board();
    reopened.model.adopt(&reopened.fleet, &v1_parsed.view);
    assert_eq!(board_rules(&mut reopened, 200), [44, 98, 150]);

    for bad in [
        "desk=",
        "desk=-1",
        "desk=01",
        "desk=65536",
        "desk=x",
        "desk=50|extra",
    ] {
        let hostile = v2_text.replacen("desk=50", bad, 1);
        assert!(
            handover::parse(&hostile).is_none(),
            "hostile desk field must be refused: {hostile}"
        );
    }
}
