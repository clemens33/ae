//! Pins of the board column's drawing, compiled as a child of `draw`: the rows
//! a board too short for them keeps, and the cells and targets of the tab bar.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::Modifier;

use super::{Hit, Layout, Model, Paint, desk_row, desk_tabs, draw_desk};
use crate::console::deskview::{self, DeskRow, DeskView, RowKind, RowLink};
use crate::console::input::{Mouse, MouseKind};
use crate::desk::{Desk, Section, parse_input};
use crate::time::Timestamp;

const FIVE: &[u8] = b"\
DECIDE
D1 | Decide item
D2 | Second decide
COMING
C1 | Coming item
RUNNING
R1 | Running item
LINKS
L1 | Links item
";

fn desk() -> Desk {
    Desk {
        updated: Timestamp::from_epoch(0),
        items: parse_input(FIVE).expect("a valid board"),
    }
}

fn row(cells: &Buffer, y: u16) -> String {
    (0..cells.area.width)
        .map(|x| cells[(x, y)].symbol())
        .collect()
}

fn draw(view: &DeskView, size: (u16, u16)) -> (Buffer, Layout) {
    let area = Rect::new(0, 0, size.0, size.1);
    let mut cells = Buffer::empty(area);
    let mut layout = Layout::default();
    let model = Model::default();
    draw_desk(
        &mut cells,
        (area, false),
        Some(view),
        (None, None),
        &mut layout,
        &model,
        0,
    );
    (cells, layout)
}

fn click(layout: &Layout, column: u16, row: u16) -> Option<Hit> {
    layout.hit(Mouse {
        kind: MouseKind::Click,
        column,
        row,
    })
}

#[test]
fn a_board_that_is_absent_or_unreadable_starts_on_its_first_row() {
    for (view, text, first) in [
        (
            DeskView::Unreadable("cut".to_owned()),
            "desk unreadable: cut",
            0,
        ),
        (DeskView::Absent, "no board set", 1),
    ] {
        let (cells, layout) = draw(&view, (36, 10));
        let found = (0..10).find(|y| row(&cells, *y).contains(text));
        assert_eq!(found, Some(first), "{text} stands on row {first}");
        assert!(cells.content().iter().all(|cell| cell.symbol() != "·"));
        assert!(click(&layout, 3, 0).is_none(), "no tab stands on it");
    }
}

#[test]
fn a_board_one_row_high_keeps_its_head_and_draws_no_bar() {
    let (cells, layout) = draw(&DeskView::Shown(desk()), (36, 1));
    assert!(row(&cells, 0).contains("BOARD"), "{:?}", row(&cells, 0));
    assert!(click(&layout, 3, 0).is_none());
}

#[test]
fn a_board_with_no_row_left_for_its_items_keeps_the_bar_whole() {
    // The head and the bar fill both rows: the overflow cue has no row of its own.
    let (cells, layout) = draw(&DeskView::Shown(desk()), (36, 2));
    assert!(row(&cells, 0).contains("BOARD"));
    let bar = row(&cells, 1);
    assert!(bar.contains("DEC 2") && bar.contains("LINK 1"), "{bar:?}");
    assert!(!bar.contains("more"), "{bar:?}");
    assert!(matches!(
        click(&layout, 3, 1),
        Some(Hit::DeskTab(Section::Decide))
    ));
}

#[test]
fn a_link_the_room_cuts_away_leaves_no_run() {
    let mut cells = Buffer::empty(Rect::new(0, 0, 10, 1));
    let mut layout = Layout::default();
    let row = DeskRow {
        kind: RowKind::Link,
        text: "  abcdef".to_owned(),
        links: vec![RowLink {
            range: 2..8,
            url: "https://example.test/cut".to_owned(),
        }],
    };
    desk_row(&mut cells, &mut layout, &row, (0, 0, 2), Paint::of(None));
    assert!(layout.links.is_empty(), "{:?}", layout.links);
    desk_row(&mut cells, &mut layout, &row, (0, 0, 8), Paint::of(None));
    assert_eq!(layout.links.len(), 1);
    assert_eq!(layout.links[0].width, 6);
}

#[test]
fn the_bar_draws_its_labels_joined_by_gaps_with_none_after_the_last() {
    let desk = desk();
    let bar = deskview::tab_bar(&desk, 34);
    let mut cells = Buffer::empty(Rect::new(0, 0, 40, 1));
    let mut layout = Layout::default();
    desk_tabs(
        &mut cells,
        &mut layout,
        &bar,
        (2, 0, 34),
        (Paint::of(None), Section::Coming),
    );
    let drawn = row(&cells, 0);
    assert_eq!(drawn.trim_end(), "  DEC 2·COM 1·RUN 1·WAIT 0·LINK 1");
    assert!(
        drawn.chars().skip(33).all(|cell| cell == ' '),
        "no gap after the last label: {drawn:?}"
    );

    // The chosen label is in the selected ink, the others are not.
    let selected = |x: u16| cells[(x, 0)].modifier.contains(Modifier::REVERSED);
    assert!(!(2..7).any(selected), "DEC is not chosen");
    assert!((8..13).all(selected), "COM is");
    assert!(!(14..19).any(selected), "RUN is not chosen");

    // Each label is a target for its own section; the gap between is none.
    assert!(matches!(
        click(&layout, 2, 0),
        Some(Hit::DeskTab(Section::Decide))
    ));
    assert!(matches!(
        click(&layout, 6, 0),
        Some(Hit::DeskTab(Section::Decide))
    ));
    assert!(click(&layout, 7, 0).is_none());
    assert!(matches!(
        click(&layout, 8, 0),
        Some(Hit::DeskTab(Section::Coming))
    ));
    assert!(matches!(
        click(&layout, 32, 0),
        Some(Hit::DeskTab(Section::Links))
    ));
    assert!(click(&layout, 33, 0).is_none());
}

#[test]
fn a_label_the_room_cuts_keeps_the_cells_it_shows() {
    let bar = deskview::tab_bar(&desk(), 34);
    let mut cells = Buffer::empty(Rect::new(0, 0, 40, 1));
    let mut layout = Layout::default();
    // The room ends after four cells: `DEC 2` shows as `DEC `, nothing follows.
    desk_tabs(
        &mut cells,
        &mut layout,
        &bar,
        (2, 0, 4),
        (Paint::of(None), Section::Decide),
    );
    assert_eq!(row(&cells, 0).trim_end(), "  DEC");
    assert!(matches!(
        click(&layout, 5, 0),
        Some(Hit::DeskTab(Section::Decide))
    ));
    assert!(click(&layout, 6, 0).is_none());
}
