//! Acceptance of the app board column, compiled as a child of `draw`.

use super::{Columns, Rect, Split, chat_start, desk_area, panel_area};

#[test]
fn a_board_column_fits_only_beside_two_full_width_chat_columns() {
    let side = Columns {
        panel: Some((false, false)),
        desk: true,
        ..Columns::default()
    };
    let focused_side = Columns {
        panel: Some((false, true)),
        desk: true,
        ..Columns::default()
    };
    let full = Columns {
        panel: Some((true, true)),
        ..Columns::default()
    };
    let narrow = Rect::new(0, 0, 160, 35);
    let boundary = Rect::new(0, 0, 161, 35);
    let wide = Rect::new(0, 0, 200, 35);
    let split = Split::default();
    assert!(desk_area(narrow, split, side).is_none());
    assert!(desk_area(narrow, split, focused_side).is_none());
    assert!(desk_area(wide, split, Columns::default()).is_none());
    assert!(desk_area(wide, split, full).is_none());
    assert_eq!(
        desk_area(boundary, split, side),
        Some(Rect::new(125, 0, 36, 35))
    );

    for columns in [side, focused_side] {
        for split in [
            split,
            Split {
                panel: Some(200),
                ..split
            },
            Split {
                sidebar: Some(52),
                panel: Some(13),
                ..split
            },
        ] {
            let rect = desk_area(wide, split, columns).expect("side mode has room at 200");
            let panel = panel_area(wide, split, columns).expect("side panel");
            assert_eq!(rect.width, 36);
            assert_eq!(rect.x + rect.width, wide.width);
            assert_eq!(panel.x + panel.width, rect.x);
            assert!(panel.width >= 40);
            assert!(panel.x - chat_start(wide, split) >= 40);
        }
    }
}
