//! Pins of the board tabs' redraw answers: what a press of a tab key or a click
//! on the board tells the loop, which no picture of the frame shows.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the acceptance fixture owns the scratch records and the in-memory terminal"
)]

use super::boardfull_spec::{bytes, paint, setup_board};
use super::*;
use crate::console::input::Keys;
use crate::desk::{Section, parse_input};

const WIDE: u16 = 200;
const HEIGHT: u16 = 35;

const BOARD: &[u8] = b"\
DECIDE
D1 | Decide item
COMING
C1 | Coming item
RUNNING
R1 | Running item
";

/// The redraw answer of the keys, as `take_keys` gives it.
fn typed(app: &mut App, raw: &[u8]) -> Option<bool> {
    take_keys(app, Keys::app().feed(raw, Instant::now()))
}

/// The redraw answer of one mouse event.
fn mouse(app: &mut App, kind: MouseKind, (column, row): (u16, u16)) -> Option<bool> {
    let event = Key::Mouse(Mouse { kind, column, row });
    take_keys(app, vec![(event, Instant::now())])
}

/// The first cell of `needle` on the frame.
fn find(cells: &Buffer, needle: &str) -> (u16, u16) {
    let width = u16::try_from(needle.len()).expect("a short needle");
    (0..cells.area.height)
        .find_map(|y| {
            (0..=cells.area.width.saturating_sub(width)).find_map(|x| {
                let found: String = (x..x + width).map(|at| cells[(at, y)].symbol()).collect();
                (found == needle).then_some((x, y))
            })
        })
        .unwrap_or_else(|| panic!("{needle:?} is not on the frame"))
}

fn board_app(tag: &str) -> App {
    let desk = crate::console::deskview::DeskView::Shown(crate::desk::Desk {
        updated: Timestamp::from_epoch(0),
        items: parse_input(BOARD).expect("a valid board"),
    });
    let (_root, mut app) = setup_board(tag, Some(desk));
    let _ = paint(&mut app, WIDE, HEIGHT);
    app
}

#[test]
fn a_picked_tab_asks_for_a_redraw_and_a_key_that_picks_none_still_ends_an_armed_press() {
    let mut app = board_app("btp-keys");
    bytes(&mut app, b"B");
    assert!(app.desk_focused());

    // A tab that is picked changes what the frame shows.
    assert_eq!(typed(&mut app, b"2"), Some(true));
    assert_eq!(app.model.desk_tab(), Section::Coming);

    // A press that waits for its second key ends on a byte the board ignores,
    // and the redraw says the arming is gone.
    assert_eq!(typed(&mut app, b"q"), Some(true));
    assert!(app.armed.is_some(), "q arms the quit");
    assert_eq!(typed(&mut app, b"7"), Some(true));
    assert!(app.armed.is_none(), "the inert key ended it");
    assert_eq!(app.model.desk_tab(), Section::Coming);

    // The same byte with nothing armed changes nothing.
    assert_eq!(typed(&mut app, b"7"), Some(false));
}

#[test]
fn a_click_on_the_board_asks_for_a_redraw_whether_it_took_the_keys_or_picked_a_tab() {
    let mut app = board_app("btp-click");
    assert!(!app.desk_focused());

    // The chosen label, with the keys elsewhere: no tab changes, the board
    // takes the keys, and the frame must be drawn again for it.
    let cells = paint(&mut app, WIDE, HEIGHT);
    let decide = find(&cells, "DEC 1");
    assert_eq!(mouse(&mut app, MouseKind::Click, decide), Some(true));
    assert!(app.desk_focused());
    assert_eq!(app.model.desk_tab(), Section::Decide);
    let _ = mouse(&mut app, MouseKind::Release, decide);

    // Another label, with the keys already on the board: the tab changes and
    // nothing else does.
    let cells = paint(&mut app, WIDE, HEIGHT);
    let coming = find(&cells, "COM 1");
    assert_eq!(mouse(&mut app, MouseKind::Click, coming), Some(true));
    assert!(app.desk_focused());
    assert_eq!(app.model.desk_tab(), Section::Coming);
    let _ = mouse(&mut app, MouseKind::Release, coming);

    // The chosen label again, with the keys already there: nothing changes.
    let cells = paint(&mut app, WIDE, HEIGHT);
    let coming = find(&cells, "COM 1");
    assert_eq!(mouse(&mut app, MouseKind::Click, coming), Some(false));
}
