//! Toggling the stopped group must not move the Agents section.
//! Oracle: the owner ask (aedev-stopped-sidebar-stable-split-20261022), not the
//! frame the code draws today: the sessions region keeps its rows and the Agents
//! heading its row while the group opens and closes; the overflow scrolls inside
//! the sessions region. Fixtures never launch tmux.

use std::collections::BTreeMap;
use std::time::Instant;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::fleet::{Counts, Fleet, Line2, Row};
use super::loader::{Answer, FleetRead};
use super::model::{Drag, Edge};
use super::{App, browse};
use crate::console::input::Key;
use crate::digest::{SessionEntry, Status};
use crate::listing::World;
use crate::theme::{Look, Mark};
use crate::time::Timestamp;

const ID: &str = "0199c0de-1234-4890-abcd-ef0123456789";

fn row(name: &str, index: usize, stopped: bool) -> Row {
    Row {
        name: name.into(),
        index,
        mark: if stopped { Mark::Dead } else { Mark::Working },
        needy: false,
        unanswered: Some(0),
        counts: if stopped {
            Counts::Stopped
        } else {
            Counts::Marks(vec![(Mark::Working, false, 2)])
        },
        line2: if stopped {
            Line2::NotRunning {
                seats: 99,
                stopped_secs: Some(60),
            }
        } else {
            Line2::NoGoal
        },
        home: false,
    }
}

/// `running` running sessions `run-N`, then `stopped` stopped ones `old-N`.
fn fleet(running: usize, stopped: usize) -> App {
    let rows: Vec<Row> = (0..running)
        .map(|at| row(&format!("run-{at}"), at + 1, false))
        .chain((0..stopped).map(|at| row(&format!("old-{at}"), running + at + 1, true)))
        .collect();
    let entries = rows
        .iter()
        .map(|row| {
            let status = if row.counts == Counts::Stopped {
                Status::Stopped
            } else {
                Status::Running
            };
            SessionEntry::new(&row.name, status)
        })
        .collect();
    let ids: BTreeMap<String, String> = (rows.iter())
        .map(|row| (row.name.clone(), ID.to_owned()))
        .collect();
    let now = Timestamp::from_epoch(1_792_579_200);
    let read = FleetRead {
        dirs: BTreeMap::new(),
        sole: ids.clone(),
        ids,
        world: World::new(now, entries),
        facts: BTreeMap::new(),
        needs: BTreeMap::new(),
        fleet: Fleet { rows, home: None },
        pairs: BTreeMap::new(),
        scanned: true,
    };
    let mut app = App::new(None, None, None, None);
    app.answer(Answer::Fleet(read));
    for row in &app.fleet.rows {
        app.looks.insert(
            row.name.clone(),
            (Some(Look::read("on", "darcula", "on", "on")), None),
        );
    }
    app
}

fn frame(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    app.frame(&mut buf);
    buf
}

/// The sidebar's text on row `y`: the cells left of the chat column.
fn side(buf: &Buffer, y: u16) -> String {
    (0..30)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
        .trim()
        .to_owned()
}

fn rows_of(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height).map(|y| side(buf, y)).collect()
}

/// The row the Agents heading stands on.
fn heading(buf: &Buffer) -> u16 {
    // Row 0 is the summary line, which also starts with the word.
    let at = (rows_of(buf).iter().skip(2)).position(|text| text.starts_with("Agents"));
    u16::try_from(at.expect("the Agents heading is drawn") + 2).unwrap_or(u16::MAX)
}

fn keys(app: &mut App, bytes: &[u8]) {
    assert!(
        browse(app, &Key::Text(bytes.to_vec()), Instant::now()).is_some(),
        "navigation must not quit"
    );
}

const SIZES: [(u16, u16); 3] = [(180, 45), (100, 30), (92, 22)];

/// The sessions region: the rows between the summary and the blank row above
/// the Agents heading.
fn region(buf: &Buffer) -> Vec<String> {
    let at = usize::from(heading(buf));
    rows_of(buf)[2..at - 1].to_vec()
}

#[test]
fn toggling_stopped_never_moves_the_agents_heading() {
    // The open list fits its share (2 + toggle + 6), and overflows it (2 + toggle + 40).
    for stopped in [6, 40] {
        for (width, height) in SIZES {
            let mut app = fleet(2, stopped);
            let closed = frame(&mut app, width, height);
            let before = heading(&closed);
            keys(&mut app, b"z");
            assert!(app.stopped().open, "z opens the group");
            let open = frame(&mut app, width, height);
            assert_eq!(
                heading(&open),
                before,
                "open, {stopped} at {width}x{height}"
            );
            keys(&mut app, b"z");
            let again = frame(&mut app, width, height);
            assert_eq!(
                heading(&again),
                before,
                "closed again, {stopped} at {width}x{height}"
            );
            assert_eq!(
                rows_of(&again),
                rows_of(&closed),
                "{stopped} at {width}x{height}"
            );
        }
    }
}

#[test]
fn the_toggle_row_stays_drawn_open_and_closed() {
    for stopped in [6, 40] {
        let mut app = fleet(2, stopped);
        let closed = region(&frame(&mut app, 180, 45));
        assert!(
            closed.contains(&format!("▸ Stopped ({stopped})")),
            "{closed:?}"
        );
        keys(&mut app, b"z");
        let open = region(&frame(&mut app, 180, 45));
        assert!(open.contains(&format!("▾ Stopped ({stopped})")), "{open:?}");
    }
}

#[test]
fn without_a_stopped_group_the_list_is_as_long_as_its_sessions() {
    for running in [1, 3, 5] {
        let mut app = fleet(running, 0);
        let buf = frame(&mut app, 180, 45);
        // The list, a blank row, then the heading.
        assert_eq!(
            usize::from(heading(&buf)),
            2 + running + 1,
            "{running} running"
        );
    }
}

#[test]
fn the_overflow_of_an_open_group_scrolls_inside_the_sessions_region() {
    let mut app = fleet(2, 40);
    let at = heading(&frame(&mut app, 180, 45));
    keys(&mut app, b"z");
    let open = frame(&mut app, 180, 45);
    let rows = rows_of(&open);
    assert!(
        region(&open).iter().any(|text| text.contains("more")),
        "the hidden stopped rows are counted in the region: {:?}",
        region(&open)
    );
    assert!(
        rows[usize::from(at)..]
            .iter()
            .all(|text| !text.contains("old-")),
        "no stopped session is drawn under the heading"
    );
}

#[test]
fn walking_to_the_last_stopped_row_scrolls_it_into_view_without_moving_the_heading() {
    let mut app = fleet(2, 40);
    let at = heading(&frame(&mut app, 180, 45));
    keys(&mut app, b"z");
    let _ = frame(&mut app, 180, 45);
    for _ in 0..42 {
        keys(&mut app, b"j");
        let _ = frame(&mut app, 180, 45);
    }
    assert_eq!(app.model.selected(), Some("old-39"), "the last stopped row");
    let end = frame(&mut app, 180, 45);
    assert_eq!(heading(&end), at, "the heading stays");
    assert!(
        region(&end).iter().any(|text| text.contains("old-39")),
        "the selected row is in the sessions region"
    );
}

#[test]
fn a_dragged_list_keeps_its_boundary_across_a_toggle() {
    let mut app = fleet(2, 40);
    app.model.grab(Drag {
        edge: Edge::List,
        from: 6,
        at: 6,
    });
    app.model.drag_to(9);
    app.model.let_go();
    let closed = heading(&frame(&mut app, 180, 45));
    keys(&mut app, b"z");
    assert_eq!(heading(&frame(&mut app, 180, 45)), closed);
}

#[test]
fn a_taller_pane_may_move_the_heading_but_a_toggle_at_one_size_may_not() {
    let mut app = fleet(30, 6);
    let tall = heading(&frame(&mut app, 180, 50));
    let short = heading(&frame(&mut app, 180, 30));
    assert_ne!(tall, short, "a resize moves the boundary");
    keys(&mut app, b"z");
    assert_eq!(heading(&frame(&mut app, 180, 30)), short);
    assert_eq!(heading(&frame(&mut app, 180, 50)), tall);
}
