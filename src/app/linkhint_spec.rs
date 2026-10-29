//! Acceptance cases: linkhint (#271). An un-dragged press and release on a link
//! cell ae does not open itself shows `link::open_hint()` on the hint row and
//! does nothing else; the mark toggle and drag selection stay as today.
//! Oracle: the issue, the lead's ruling and the literals below.

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::fleet::{Counts, Fleet, Line2, Row};
use super::loader::Request;
use super::model::Model;
use super::{App, take_keys};
use crate::console::input::{self, Key, MouseKind};
use crate::console::lane::{Item, Kind, Lane};
use crate::console::link;
use crate::digest::{SessionEntry, Status};
use crate::theme::Mark;
use crate::time::Timestamp;

const BODY: &str = "see https://x.io/a then src/a.rs and plain words";

#[test]
fn the_hint_names_the_platform_modifier() {
    #[cfg(target_os = "macos")]
    assert_eq!(link::open_hint(), "Shift+Cmd+click to open link");
    #[cfg(not(target_os = "macos"))]
    assert_eq!(link::open_hint(), "Ctrl+click or Shift+click to open link");
}

/// An app on session `api` whose lane holds one card or lead turn, marked or not.
fn app_of(card: bool, body: &str, marked: bool) -> (App, Receiver<Request>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::new(Some("api".to_owned()), None, None, Some(tx));
    (app.fleeted, app.loading) = (true, false);
    let row = Row {
        name: "api".to_owned(),
        index: 1,
        mark: Mark::Working,
        needy: false,
        unanswered: None,
        counts: Counts::Unknown,
        line2: Line2::NoGoal,
        home: true,
    };
    let fleet = Fleet {
        rows: vec![row],
        home: Some("api".to_owned()),
    };
    app.model = Model::new(&fleet);
    app.fleet = fleet;
    let mut entry = SessionEntry::new("api", Status::Running);
    entry.work_dir = Some("/w".to_owned());
    app.world.sessions.push(entry.clone());
    let home = Some(std::path::Path::new("/h"));
    app.overview = super::overview::of(&entry, None, home, Timestamp::from_epoch(1_759_500_600));
    let seat = "lead".to_owned();
    let kind = if card {
        Kind::Card { seat }
    } else {
        Kind::Said { who: seat }
    };
    let (micros, body, record) = (1_759_500_000_000_000, body.to_owned(), None);
    let items = vec![Item {
        micros,
        kind,
        body,
        record,
    }];
    app.lane = std::rc::Rc::new(Lane {
        items,
        coverage: vec![],
    });
    if marked {
        let key = app.keys()[0];
        app.model.set_turn(Some(key));
    }
    (app, rx)
}

fn mouse_at(app: &mut App, kind: MouseKind, (column, row): (u16, u16)) {
    let mouse = input::Mouse { kind, column, row };
    let taken = take_keys(app, vec![(Key::Mouse(mouse), Instant::now())]);
    assert!(taken.is_some(), "a mouse report never quits");
}

fn rows(app: &mut App) -> Vec<String> {
    let mut cells = Buffer::empty(Rect::new(0, 0, 160, 32));
    app.frame(&mut cells);
    let row = |y| (0..160).map(|x| cells[(x, y)].symbol()).collect::<String>();
    (0..32).map(row).collect()
}

fn cell_of(app: &mut App, text: &str) -> (u16, u16) {
    let shown = rows(app);
    let lane = app.layout.lane_area().expect("the lane is drawn");
    for y in lane.y..lane.y + lane.height {
        let drawn = &shown[usize::from(y)];
        if let Some(at) = drawn.find(text) {
            let column = u16::try_from(drawn[..at].chars().count()).expect("a column");
            return (column, y);
        }
    }
    panic!("{text} not drawn:\n{}", shown.join("\n"));
}

fn reads(rx: &Receiver<Request>) -> usize {
    rx.try_iter()
        .filter(|r| matches!(r, Request::Read(Some(_))))
        .count()
}

fn hinted(app: &mut App) -> bool {
    let drawn = rows(app).iter().any(|row| row.contains(link::open_hint()));
    app.note().as_deref() == Some(link::open_hint()) && drawn
}

fn unhinted(app: &mut App) -> bool {
    app.flash.is_none() && !rows(app).iter().any(|row| row.contains("to open link"))
}

#[test]
fn an_undragged_release_on_a_web_link_shows_the_hint_and_changes_nothing_else() {
    let (mut app, rx) = app_of(false, BODY, true);
    let at = cell_of(&mut app, "https://x.io/a");
    mouse_at(&mut app, MouseKind::Click, at);
    assert!(unhinted(&mut app), "the hint waits for the release");
    assert_eq!(app.model.turn(), None, "the press unmarks, as today");
    mouse_at(&mut app, MouseKind::Release, at);
    assert!(hinted(&mut app), "the release shows the hint");
    assert_eq!(app.model.turn(), None, "the hint moves no mark");
    assert_eq!(reads(&rx), 0, "no read is asked for");
    assert!(!app.model.reader_open() && app.select.is_none());
    let (_, shown) = app.flash.clone().expect("the hint is the flash");
    assert!(!app.lapse(shown + Duration::from_secs(1)), "still shown");
    assert!(app.lapse(shown + super::FLASH_WINDOW + Duration::from_secs(1)));
    assert!(unhinted(&mut app), "it lapses with the flash window");
}

#[test]
fn every_cell_of_a_web_link_in_any_turn_hints() {
    let on_card = "Pick: a | b.\n\ndetails: https://x.io/card";
    for (card, body, marked, url) in [
        (false, BODY, true, "https://x.io/a"),
        (false, BODY, false, "https://x.io/a"),
        (true, on_card, false, "https://x.io/card"),
    ] {
        for nth in [0, 5, url.len() - 1] {
            let (mut app, _rx) = app_of(card, body, marked);
            let (x, y) = cell_of(&mut app, url);
            let at = (x + u16::try_from(nth).expect("a column"), y);
            mouse_at(&mut app, MouseKind::Click, at);
            mouse_at(&mut app, MouseKind::Release, at);
            assert!(hinted(&mut app), "{url} cell {nth} marked {marked}");
        }
    }
}

#[test]
fn a_drag_from_a_web_link_selects_and_shows_no_hint() {
    let (mut app, _rx) = app_of(false, BODY, true);
    let mark = app.model.turn();
    let (x, y) = cell_of(&mut app, "https://x.io/a");
    mouse_at(&mut app, MouseKind::Click, (x, y));
    mouse_at(&mut app, MouseKind::Drag, (x + 4, y));
    mouse_at(&mut app, MouseKind::Release, (x + 4, y));
    assert!(unhinted(&mut app), "a drag is a selection, not a click");
    assert!(app.select.is_some(), "the dragged selection stands");
    assert_eq!(app.model.turn(), mark, "the drag puts the mark back");
}

#[test]
fn no_hint_without_a_press_and_release_on_the_same_web_link_cell() {
    let (mut app, _rx) = app_of(false, BODY, true);
    let link = cell_of(&mut app, "https://x.io/a");
    let plain = cell_of(&mut app, "plain");
    let file = cell_of(&mut app, "src/a.rs");
    mouse_at(&mut app, MouseKind::Release, link);
    assert!(unhinted(&mut app), "a release with no press");
    for off in [plain, file] {
        mouse_at(&mut app, MouseKind::Click, link);
        mouse_at(&mut app, MouseKind::Release, off);
        assert!(unhinted(&mut app), "pressed on the link, released {off:?}");
    }
    mouse_at(&mut app, MouseKind::Click, plain);
    mouse_at(&mut app, MouseKind::Release, plain);
    assert!(unhinted(&mut app), "a click on plain text");
}

#[test]
fn a_second_press_a_key_or_a_wheel_notch_between_press_and_release_cancels() {
    for between in ["press", "key", "wheel"] {
        let (mut app, _rx) = app_of(false, BODY, true);
        let link = cell_of(&mut app, "https://x.io/a");
        let plain = cell_of(&mut app, "plain");
        mouse_at(&mut app, MouseKind::Click, link);
        match between {
            "press" => mouse_at(&mut app, MouseKind::Click, plain),
            "key" => {
                let _ = take_keys(&mut app, vec![(Key::Text(b"j".to_vec()), Instant::now())]);
            }
            _ => mouse_at(&mut app, MouseKind::WheelDown, link),
        }
        mouse_at(&mut app, MouseKind::Release, link);
        assert!(unhinted(&mut app), "{between}");
    }
}

#[test]
fn a_file_link_click_reads_it_and_shows_no_hint() {
    let (mut app, rx) = app_of(false, BODY, true);
    let at = cell_of(&mut app, "src/a.rs");
    mouse_at(&mut app, MouseKind::Click, at);
    mouse_at(&mut app, MouseKind::Release, at);
    assert_eq!(reads(&rx), 1, "a file link opens the reader");
    assert!(app.model.reader_open());
    assert!(app.flash.is_none(), "a link ae opens needs no hint");
}
