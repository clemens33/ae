//! The board column keeps its place beside a side panel only while a board is
//! shown or cannot be read: with none, every width is the one without a board.

use super::{
    Columns, DESK_W, Drag, Edge, Rect, Split, desk_area, desk_slot, dragged_to, panel_area,
    panel_full,
};
use crate::console::input::{Mouse, MouseKind};

const WIDE: Rect = Rect::new(0, 0, 200, 35);

fn side(desk: bool) -> Columns {
    Columns {
        panel: Some((false, false)),
        desk,
        ..Columns::default()
    }
}

#[test]
fn the_board_slot_is_the_pure_area_only_while_a_board_is_present() {
    let split = Split::default();
    assert!(desk_area(WIDE, split, side(false)).is_some());
    assert_eq!(desk_slot(WIDE, split, side(false)), None);
    assert_eq!(
        desk_slot(WIDE, split, side(true)),
        desk_area(WIDE, split, side(true))
    );
    let narrow = Rect::new(0, 0, 160, 35);
    assert_eq!(desk_slot(narrow, split, side(true)), None);
}

#[test]
fn a_side_panel_ends_at_the_frame_without_a_board_and_beside_the_column_with_one() {
    let split = Split::default();
    let bare = panel_area(WIDE, split, side(false)).expect("side panel with no board");
    assert_eq!(bare.x + bare.width, WIDE.width);
    let beside = panel_area(WIDE, split, side(true)).expect("side panel with a board");
    assert_eq!(beside.x + beside.width, WIDE.width - DESK_W);
    assert!(beside.width < bare.width);
}

#[test]
fn a_frame_without_room_for_the_column_stands_as_it_does_without_a_board() {
    let split = Split::default();
    let held = |desk| Columns {
        panel: Some((false, true)),
        desk,
        ..Columns::default()
    };
    let frame = Rect::new(0, 0, 150, 35);
    for desk in [false, true] {
        assert!(!panel_full(frame, split, held(desk)), "desk {desk}");
        let panel = panel_area(frame, split, held(desk)).expect("side panel");
        assert_eq!(panel.x + panel.width, frame.width, "desk {desk}");
    }
    let tight = Rect::new(0, 0, 100, 35);
    assert!(panel_full(tight, split, held(false)));
    assert!(panel_full(tight, split, held(true)));
}

#[test]
fn a_dragged_panel_edge_clamps_beside_the_column_only_while_a_board_is_present() {
    let split = Split::default();
    let drag = Drag {
        edge: Edge::Panel,
        from: 100,
        at: 100,
    };
    let far = Mouse {
        kind: MouseKind::Drag,
        column: 0,
        row: 1,
    };
    let bare = dragged_to(drag, far, WIDE, split, 3, false);
    let beside = dragged_to(drag, far, WIDE, split, 3, true);
    assert_eq!(bare - beside, DESK_W);
}
