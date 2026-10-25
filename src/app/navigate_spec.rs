//! Frozen acceptance cases: mdreader P2 (#251 NAVIGATE): a click on a file
//! link in a chat turn opens the reader on THAT link, `n`/`p` step through the
//! turn's file links, and closing returns to the lane as `r` does today.
//! Independent navigator mdreader-nav (Anthropic / Claude Opus 5.5), frozen
//! against driver mdreader's plan r3 (.local/mdreader/plan-p2.md sha aa803416,
//! lead ACK bbb467aa). Oracle: issue #251, the brief's P2 words and that ACK
//! — never the code. Driver installs byte-identically, wired from src/app.rs
//! via `#[cfg(test)] #[path = "app/navigate_spec.rs"] mod navigate_spec;`.
//!
//! Surface: none new. Every case drives the App the way the terminal does
//! (`take_keys` with decoded keys and mouse reports, `frame`, `answer`) and
//! reads what it sends (`Request::Read`) and draws (the reader header).
//!
//! Behaviour pinned (plan r3 + ACK):
//! - ONE list of file links per turn, over every part the lane lays out
//!   (prose, options, evidence, a closed ask's excerpt), in source order,
//!   duplicates kept as distinct occurrences, web links left out. `r` opens
//!   its first entry, a click its clicked occurrence; fold, clip and scroll
//!   never shorten it.
//! - The header names the position: no suffix for a one-link turn,
//!   ` · first of N links` at the first entry (a `p` back to it too),
//!   ` · K of N links` at entry K > 1.
//! - `n`/`p` in the reader step that list: one new read each, a new
//!   generation, mode reset to the link's default (rendered for `.md` without
//!   `:line`, raw with one), top reset; at either end nothing is sent, nothing
//!   flashes, nothing wraps. A late answer for a stepped-away generation never
//!   paints.
//! - A press on a FILE-link cell (any cell of a wrapped link, a wide
//!   character's covered cell included) changes no mark and sends nothing;
//!   the un-dragged release on the SAME occurrence opens it. A release
//!   elsewhere, a drag, a second press, a key or a wheel notch between them
//!   cancels. A press on a web link or plain text keeps today's mark toggle.
//! - A click aimed at an older frame opens only if that turn still displays
//!   what the frame drew; otherwise it refuses `the view changed`. Press and
//!   release both aimed at that frame are judged against it.
//! - A closed ask's excerpt is cut at 80 characters with `…`: an occurrence
//!   touching that cut, one ending exactly at character 80 included, is
//!   CONSERVATIVELY left out of the list and the click cells; an excerpt that
//!   was not cut keeps a path ending at character 80.
//! - The decoder still drops every modified press (Cmd/Ctrl/Shift/Alt-click
//!   stay the terminal's); a release with no press opens nothing.
//! - The keys overlay's Reader section names `n/p`.
//! - A file-link press, open, step and close ask the loader for reads and
//!   closes ONLY: no writing change, focus, settings, peek or reread request
//!   (r3, colead I1). A press on a web link or plain text keeps today's
//!   effects, so its cases read the reads alone.
//! - A change of the displayed turn, or of the base its path resolves
//!   against, between a valid press and its release opens nothing and never
//!   retargets (r3, colead I2).
//! - Closing returns to a SCROLLED lane exactly: position, mark and focus;
//!   a link the scroll hid stays in the list (r3, colead I3). A link whose
//!   letter carries a combining mark opens from every cell (r3, colead N1).
//!
//! Named residuals (not pinned): a terminal that reports Cmd-click as a plain
//! press; the legacy OSC 8 run of a clipped excerpt path; hover preview (out of
//! scope); the render proof in ae-dev (its own gate).

use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::fleet::{Counts, Fleet, Line2, Row};
use super::loader::{Answer, ReadAsk, Request};
use super::model::Model;
use super::reader::Body;
use super::{App, take_keys};
use crate::console::input;
use crate::console::lane::{Item, Kind, Lane};
use crate::digest::{SessionEntry, Status};
use crate::theme::Mark;
use crate::time::Timestamp;

const MICROS: i64 = 1_759_500_000_000_000;
/// Wide enough that the longest header any case pins (65 cells, measured) is drawn
/// whole: the reader header clips at the right edge of its column.
const WIDTH: u16 = 160;
const HEIGHT: u16 = 32;

// ---------------------------------------------------------------------------
// Rig
// ---------------------------------------------------------------------------

fn fleet(names: &[&str]) -> Fleet {
    let row = |(at, name): (usize, &&str)| Row {
        name: (*name).to_owned(),
        index: at + 1,
        mark: Mark::Working,
        needy: false,
        unanswered: None,
        counts: Counts::Unknown,
        line2: Line2::NoGoal,
        home: *name == "api",
    };
    Fleet {
        rows: names.iter().enumerate().map(row).collect(),
        home: Some("api".to_owned()),
    }
}

fn item(micros: i64, kind: Kind, body: &str) -> Item {
    Item {
        micros,
        kind,
        body: body.to_owned(),
        record: None,
    }
}

fn said(micros: i64, body: &str) -> Item {
    item(
        micros,
        Kind::Said {
            who: "lead".to_owned(),
        },
        body,
    )
}

/// An app on session `api` (work dir `/w`, home `/h`) whose lane holds
/// `items`; the turn at `marked` is marked when given.
fn app_with(items: Vec<Item>, marked: Option<usize>) -> (App, Receiver<Request>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::new(Some("api".to_owned()), None, None, Some(tx));
    (app.fleeted, app.loading) = (true, false);
    let fleet = fleet(&["api"]);
    app.model = Model::new(&fleet);
    app.fleet = fleet;
    let mut entry = SessionEntry::new("api", Status::Running);
    entry.work_dir = Some("/w".to_owned());
    app.world.sessions.push(entry.clone());
    app.overview = super::overview::of(
        &entry,
        None,
        Some(Path::new("/h")),
        Timestamp::from_epoch(1_759_500_600),
    );
    app.lane = Rc::new(Lane {
        items,
        coverage: vec![],
    });
    if let Some(at) = marked {
        let key = app.keys()[at];
        app.model.set_turn(Some(key));
    }
    (app, rx)
}

/// One marked turn saying `body`.
fn marked_app(body: &str) -> (App, Receiver<Request>) {
    app_with(vec![said(MICROS, body)], Some(0))
}

/// Move session `api`'s work dir, and so the base its links resolve against,
/// to `dir`; the drawn paths read the same.
fn rebased(app: &mut App, dir: &str) {
    let mut entry = SessionEntry::new("api", Status::Running);
    entry.work_dir = Some(dir.to_owned());
    app.world.sessions = vec![entry.clone()];
    app.overview = super::overview::of(
        &entry,
        None,
        Some(Path::new("/h")),
        Timestamp::from_epoch(1_759_500_600),
    );
}

fn pressed(app: &mut App, bytes: &[u8]) -> Option<bool> {
    take_keys(
        app,
        vec![(input::Key::Text(bytes.to_vec()), Instant::now())],
    )
}

fn mouse_at(app: &mut App, kind: input::MouseKind, (x, y): (u16, u16), origin: Instant) {
    let mouse = input::Mouse {
        kind,
        column: x,
        row: y,
    };
    let taken = take_keys(app, vec![(input::Key::Mouse(mouse), origin)]);
    assert!(taken.is_some(), "a mouse report never quits");
}

fn press(app: &mut App, at: (u16, u16)) {
    mouse_at(app, input::MouseKind::Click, at, Instant::now());
}

fn release(app: &mut App, at: (u16, u16)) {
    mouse_at(app, input::MouseKind::Release, at, Instant::now());
}

fn click(app: &mut App, at: (u16, u16)) {
    press(app, at);
    release(app, at);
}

fn frame_of(app: &mut App) -> Buffer {
    let mut cells = Buffer::empty(Rect::new(0, 0, WIDTH, HEIGHT));
    app.frame(&mut cells);
    cells
}

fn whole(cells: &Buffer) -> String {
    (0..cells.area.height)
        .map(|y| {
            (0..cells.area.width)
                .map(|x| cells[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the app asked of its loader since the last call: the reads and the
/// number of closes. Any other request is an effect a file link may not have
/// (a writing change, a focus, a settings or peek ask, a reread) and fails the
/// case by name.
#[track_caller]
fn effects(rx: &Receiver<Request>) -> (Vec<ReadAsk>, usize) {
    let (mut reads, mut closes) = (Vec::new(), 0);
    while let Ok(request) = rx.try_recv() {
        match request {
            Request::Read(Some(ask)) => reads.push(*ask),
            Request::Read(None) => closes += 1,
            Request::Writing(writing) => panic!(
                "a file link changed the writing (Writing({}))",
                writing.is_some()
            ),
            Request::Focus(focus) => panic!("a file link moved the focus to {focus:?}"),
            _ => panic!("a file link asked the loader for something other than a read"),
        }
    }
    (reads, closes)
}

/// The reads asked for since the last call: no close, no other effect.
#[track_caller]
fn asks(rx: &Receiver<Request>) -> Vec<ReadAsk> {
    let (reads, closes) = effects(rx);
    assert_eq!(closes, 0, "nothing was closed");
    reads
}

/// The reads asked for since the last call, every other request dropped: for
/// presses that keep today's turn behaviour, which may end the writing.
fn reads_only(rx: &Receiver<Request>) -> Vec<ReadAsk> {
    let mut out = Vec::new();
    while let Ok(request) = rx.try_recv() {
        if let Request::Read(Some(ask)) = request {
            out.push(*ask);
        }
    }
    out
}

/// The one read asked for since the last call.
#[track_caller]
fn one_ask(rx: &Receiver<Request>) -> ReadAsk {
    let mut all = asks(rx);
    assert_eq!(all.len(), 1, "exactly one read is asked for");
    all.remove(0)
}

fn answer(app: &mut App, generation: u64, src: &str) {
    app.answer(Answer::Read {
        generation,
        body: Body::Text {
            lines: src.lines().map(str::to_owned).collect(),
            size: 0,
        },
    });
}

/// The reader header the next frame draws.
#[track_caller]
fn header(app: &mut App) -> String {
    let cells = frame_of(app);
    let shown = whole(&cells);
    shown
        .lines()
        .find_map(|line| {
            line.find("Read · ")
                .map(|at| line[at..].trim_end().to_owned())
        })
        .unwrap_or_else(|| panic!("no reader header:\n{shown}"))
}

/// Open, answer and read the header: the ask and the header it drew.
#[track_caller]
fn landed(app: &mut App, rx: &Receiver<Request>, src: &str) -> (ReadAsk, String) {
    let ask = one_ask(rx);
    answer(app, ask.generation, src);
    let header = header(app);
    (ask, header)
}

fn wide(ch: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&ch)
}

/// The cells `text` takes, one string each: a letter with the combining mark
/// these cases use (U+0345, alphabetic, zero width) shares its letter's cell.
fn units(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for ch in text.chars() {
        match out.last_mut() {
            Some(last) if ch == '\u{345}' => last.push(ch),
            _ => out.push(ch.to_string()),
        }
    }
    out
}

/// Every drawn occurrence of `link` in the lane, in reading order: the cells
/// that spell it, a wide character's covered cell included, a combining mark
/// in its letter's cell, read across a row
/// end where the lane wraps it (the rest of that row blank, the next row's
/// lead-in neither a letter nor a digit).
fn occurrences(app: &App, cells: &Buffer, link: &str) -> Vec<Vec<(u16, u16)>> {
    let lane = app.layout.lane_area().expect("the lane is drawn");
    let want = units(link);
    let (right, bottom) = (lane.x + lane.width, lane.y + lane.height);
    let sym = |x: u16, y: u16| cells[(x, y)].symbol().to_owned();
    let spell = |x0: u16, y0: u16| -> Option<Vec<(u16, u16)>> {
        let (mut x, mut y, mut at, mut run) = (x0, y0, 0, Vec::new());
        while at < want.len() {
            if x < right && sym(x, y) == want[at] {
                run.push((x, y));
                if want[at].chars().next().is_some_and(wide) {
                    run.push((x + 1, y));
                    x += 2;
                } else {
                    x += 1;
                }
                at += 1;
                continue;
            }
            let blank = (x..right).all(|x| sym(x, y).trim().is_empty());
            if run.is_empty() || !blank || y + 1 >= bottom {
                return None;
            }
            y += 1;
            let start = (lane.x..right).find(|&x| sym(x, y) == want[at])?;
            let lead_in = (lane.x..start).all(|x| !sym(x, y).chars().any(char::is_alphanumeric));
            if !lead_in {
                return None;
            }
            x = start;
        }
        Some(run)
    };
    let mut found = Vec::new();
    for y in lane.y..bottom {
        for x in lane.x..right {
            if let Some(run) = spell(x, y) {
                found.push(run);
            }
        }
    }
    found
}

/// The first cell of the `nth` drawn occurrence of `link`.
#[track_caller]
fn cell_of(app: &mut App, link: &str, nth: usize) -> (u16, u16) {
    let cells = frame_of(app);
    let found = occurrences(app, &cells, link);
    found
        .get(nth)
        .and_then(|run| run.first().copied())
        .unwrap_or_else(|| panic!("occurrence {nth} of {link} drawn:\n{}", whole(&cells)))
}

const MD: &str = "# T\n\nbody one\n\nbody two";

// ---------------------------------------------------------------------------
// Click opens THAT link
// ---------------------------------------------------------------------------

#[test]
fn a_click_on_the_second_link_opens_it_and_names_its_place() {
    let (mut app, rx) = marked_app("see src/a.rs then docs/b.md:7 end");
    let mark = app.model.turn();
    let at = cell_of(&mut app, "docs/b.md:7", 0);
    press(&mut app, at);
    assert!(asks(&rx).is_empty(), "a press alone reads nothing");
    assert!(!app.model.reader_open(), "a press alone opens nothing");
    assert_eq!(app.model.turn(), mark, "a file-link press changes no mark");
    release(&mut app, at);
    let (ask, head) = landed(
        &mut app,
        &rx,
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight",
    );
    assert_eq!((ask.path.as_str(), ask.line), ("/w/docs/b.md", Some(7)));
    assert!(app.model.reader_open());
    assert_eq!(app.model.turn(), mark, "the open changes no mark");
    assert!(head.starts_with("Read · /w/docs/b.md:7 · "), "{head}");
    assert!(head.ends_with(" · 2 of 2 links"), "{head}");
}

#[test]
fn a_click_on_an_unmarked_turn_opens_without_marking_it() {
    let (mut app, rx) = app_with(vec![said(MICROS, "only src/one.rs here")], None);
    let at = cell_of(&mut app, "src/one.rs", 0);
    click(&mut app, at);
    let (ask, head) = landed(&mut app, &rx, "x");
    assert_eq!((ask.path.as_str(), ask.line), ("/w/src/one.rs", None));
    assert_eq!(app.model.turn(), None, "no turn is marked by the click");
    assert!(
        !head.contains(" links"),
        "a one-link turn names no place: {head}"
    );
}

#[test]
fn every_cell_of_a_wrapped_link_opens_it_and_no_other_lane_cell_reads() {
    let link = format!(
        "docs/{}漢字漢字/{}/tail.md:12",
        "segment-".repeat(9),
        "deeper-part-".repeat(5)
    );
    let body = format!("see {link} end");
    let (mut app, _rx) = marked_app(&body);
    let cells = frame_of(&mut app);
    let found = occurrences(&app, &cells, &link);
    assert_eq!(found.len(), 1, "one drawn occurrence:\n{}", whole(&cells));
    let run = found[0].clone();
    let rows: std::collections::BTreeSet<u16> = run.iter().map(|(_, y)| *y).collect();
    assert!(rows.len() >= 2, "the link wraps:\n{}", whole(&cells));
    assert!(
        run.iter()
            .any(|(x, y)| cells[(*x, *y)].symbol().trim().is_empty()),
        "a wide character's covered cell is in the run",
    );
    for &at in &run {
        let (mut app, rx) = marked_app(&body);
        let _ = frame_of(&mut app);
        click(&mut app, at);
        let ask = one_ask(&rx);
        assert_eq!(
            (ask.path.as_str(), ask.line),
            (
                format!("/w/{}", link.trim_end_matches(":12")).as_str(),
                Some(12)
            ),
            "cell {at:?}",
        );
    }
    let lane = app.layout.lane_area().expect("lane");
    let (mut app, rx) = marked_app(&body);
    let _ = frame_of(&mut app);
    for y in lane.y..lane.y + lane.height {
        for x in lane.x..lane.x + lane.width {
            if run.contains(&(x, y)) {
                continue;
            }
            click(&mut app, (x, y));
            assert!(reads_only(&rx).is_empty(), "cell ({x}, {y}) reads nothing");
            assert!(!app.model.reader_open(), "cell ({x}, {y}) opens nothing");
        }
    }
}

#[test]
fn every_cell_of_a_link_with_a_combining_mark_opens_it() {
    let link = "docs/te\u{345}st/x.md";
    let body = format!("see {link} end");
    let (mut app, _rx) = marked_app(&body);
    let cells = frame_of(&mut app);
    let found = occurrences(&app, &cells, link);
    assert_eq!(found.len(), 1, "one drawn occurrence:\n{}", whole(&cells));
    let run = found[0].clone();
    assert_eq!(
        run.len(),
        units(link).len(),
        "the mark shares its letter's cell"
    );
    assert!(
        run.iter()
            .any(|(x, y)| cells[(*x, *y)].symbol() == "e\u{345}"),
        "the marked letter is one drawn cell",
    );
    for &at in &run {
        let (mut app, rx) = marked_app(&body);
        let _ = frame_of(&mut app);
        click(&mut app, at);
        let ask = one_ask(&rx);
        assert_eq!(ask.path, "/w/docs/te\u{345}st/x.md", "cell {at:?}");
    }
}

#[test]
fn a_press_on_a_web_link_or_plain_text_keeps_the_mark_toggle_and_reads_nothing() {
    for (target, nth) in [("https://x.io/a", 0), ("see", 0)] {
        let (mut app, rx) = marked_app("see https://x.io/a then src/a.rs");
        let mark = app.model.turn();
        assert!(mark.is_some());
        let at = cell_of(&mut app, target, nth);
        press(&mut app, at);
        assert_eq!(
            app.model.turn(),
            None,
            "{target}: the press unmarks, as today"
        );
        release(&mut app, at);
        assert!(reads_only(&rx).is_empty(), "{target}: nothing read");
        assert!(!app.model.reader_open());
    }
}

#[test]
fn a_drag_from_a_link_selects_and_never_opens() {
    let (mut app, rx) = marked_app("see docs/long-name.md end");
    let mark = app.model.turn();
    let (x, y) = cell_of(&mut app, "docs/long-name.md", 0);
    press(&mut app, (x, y));
    assert_eq!(app.model.turn(), mark, "a file-link press changes no mark");
    mouse_at(&mut app, input::MouseKind::Drag, (x + 3, y), Instant::now());
    release(&mut app, (x + 3, y));
    assert!(asks(&rx).is_empty(), "a drag reads nothing");
    assert!(!app.model.reader_open());
    assert!(
        app.select.is_some(),
        "the dragged selection stands for its copy"
    );
    assert_eq!(app.model.turn(), mark, "a file-link press changes no mark");
}

// ---------------------------------------------------------------------------
// Cancels
// ---------------------------------------------------------------------------

const TWO: &str = "first src/a.rs then src/b.rs and some plain words";

#[test]
fn a_release_off_the_pressed_link_opens_nothing() {
    let (mut app, rx) = marked_app(TWO);
    let a = cell_of(&mut app, "src/a.rs", 0);
    let b = cell_of(&mut app, "src/b.rs", 0);
    let plain = cell_of(&mut app, "plain", 0);
    press(&mut app, a);
    release(&mut app, plain);
    assert!(asks(&rx).is_empty(), "released on plain text");
    press(&mut app, a);
    release(&mut app, b);
    assert!(asks(&rx).is_empty(), "released on another link");
    assert!(!app.model.reader_open());
}

#[test]
fn a_second_press_cancels_the_first_and_opens_only_its_own_link() {
    let (mut app, rx) = marked_app(TWO);
    let a = cell_of(&mut app, "src/a.rs", 0);
    let b = cell_of(&mut app, "src/b.rs", 0);
    press(&mut app, a);
    press(&mut app, b);
    release(&mut app, b);
    let ask = one_ask(&rx);
    assert_eq!(ask.path, "/w/src/b.rs");
}

#[test]
fn a_key_or_a_wheel_notch_between_press_and_release_cancels() {
    for between in ["key", "wheel"] {
        let (mut app, rx) = marked_app(TWO);
        let a = cell_of(&mut app, "src/a.rs", 0);
        press(&mut app, a);
        if between == "key" {
            let _ = pressed(&mut app, b"j");
        } else {
            mouse_at(&mut app, input::MouseKind::WheelDown, a, Instant::now());
        }
        release(&mut app, a);
        assert!(reads_only(&rx).is_empty(), "{between}: nothing read");
        assert!(!app.model.reader_open(), "{between}: nothing open");
    }
}

#[test]
fn a_release_with_no_press_opens_nothing() {
    let (mut app, rx) = marked_app(TWO);
    let a = cell_of(&mut app, "src/a.rs", 0);
    release(&mut app, a);
    assert!(asks(&rx).is_empty());
    assert!(!app.model.reader_open());
}

#[test]
fn the_decoder_drops_every_modified_press() {
    let now = Instant::now();
    for code in [4u32, 8, 16, 20, 24, 28] {
        let mut keys = input::Keys::app();
        let seq = format!("\x1b[<{code};10;5M");
        let got = keys.feed(seq.as_bytes(), now);
        assert!(
            !got.iter()
                .any(|(key, _)| matches!(key, input::Key::Mouse(_))),
            "modifier code {code} reaches the app",
        );
    }
    let mut keys = input::Keys::app();
    let got = keys.feed(b"\x1b[<0;10;5M", now);
    assert!(
        got.iter().any(|(key, _)| matches!(
            key,
            input::Key::Mouse(input::Mouse {
                kind: input::MouseKind::Click,
                column: 9,
                row: 4,
            })
        )),
        "a plain press still decodes",
    );
}

// ---------------------------------------------------------------------------
// Older frames
// ---------------------------------------------------------------------------

#[test]
fn a_click_on_an_older_frame_of_a_changed_turn_refuses_by_name() {
    let (mut app, rx) = marked_app("see src/a.rs then docs/b.md:7 end");
    let at = cell_of(&mut app, "docs/b.md:7", 0);
    let drawn = Instant::now();
    app.lane = Rc::new(Lane {
        items: vec![said(MICROS, "see src/a.rs then docs/zz.md:7 end")],
        coverage: vec![],
    });
    let _ = frame_of(&mut app);
    mouse_at(&mut app, input::MouseKind::Click, at, drawn);
    mouse_at(&mut app, input::MouseKind::Release, at, drawn);
    assert!(asks(&rx).is_empty(), "a changed turn opens nothing");
    assert!(!app.model.reader_open());
    let flash = app
        .flash
        .as_ref()
        .map(|(words, _)| words.clone())
        .unwrap_or_default();
    assert!(flash.contains("the view changed"), "{flash:?}");
}

#[test]
fn a_click_on_an_older_frame_of_an_unchanged_turn_opens_its_link() {
    let (mut app, rx) = marked_app("see src/a.rs then docs/b.md:7 end");
    let at = cell_of(&mut app, "docs/b.md:7", 0);
    let drawn = Instant::now();
    app.lane = Rc::new(Lane {
        items: vec![
            said(MICROS, "see src/a.rs then docs/b.md:7 end"),
            said(MICROS + 1, "a later turn\nwith\nseveral\nrows\nof\nwords"),
        ],
        coverage: vec![],
    });
    let _ = frame_of(&mut app);
    mouse_at(&mut app, input::MouseKind::Click, at, drawn);
    mouse_at(&mut app, input::MouseKind::Release, at, drawn);
    let ask = one_ask(&rx);
    assert_eq!((ask.path.as_str(), ask.line), ("/w/docs/b.md", Some(7)));
}

#[test]
fn a_change_between_a_valid_press_and_its_release_opens_nothing() {
    for redraw in [false, true] {
        let (mut app, rx) = marked_app("see src/a.rs then docs/b.md:7 end");
        let at = cell_of(&mut app, "docs/b.md:7", 0);
        press(&mut app, at);
        app.lane = Rc::new(Lane {
            items: vec![said(MICROS, "see src/a.rs then docs/zz.md:7 end")],
            coverage: vec![],
        });
        if redraw {
            let _ = frame_of(&mut app);
        }
        release(&mut app, at);
        assert!(
            asks(&rx).is_empty(),
            "redraw {redraw}: the changed turn opens nothing"
        );
        assert!(!app.model.reader_open(), "redraw {redraw}");
    }
    let (mut app, rx) = marked_app(TWO);
    let at = cell_of(&mut app, "src/a.rs", 0);
    press(&mut app, at);
    rebased(&mut app, "/v");
    let _ = frame_of(&mut app);
    release(&mut app, at);
    assert!(asks(&rx).is_empty(), "a changed base opens nothing");
    assert!(!app.model.reader_open());
    let key = app.keys()[0];
    app.model.set_turn(Some(key));
    let _ = pressed(&mut app, b"r");
    assert_eq!(
        one_ask(&rx).path,
        "/v/src/a.rs",
        "the base did change: r now reads under /v",
    );
}

// ---------------------------------------------------------------------------
// n / p
// ---------------------------------------------------------------------------

const FOUR: &str = "a src/x.rs:1 b docs/y.md c https://x.io/z d src/x.rs:9 e src/x.rs:1 f";

#[test]
fn n_and_p_step_every_occurrence_in_order_and_stop_at_the_ends() {
    let (mut app, rx) = marked_app(FOUR);
    let _ = pressed(&mut app, b"r");
    let (first, head) = landed(&mut app, &rx, "x\ny");
    assert_eq!((first.path.as_str(), first.line), ("/w/src/x.rs", Some(1)));
    assert!(head.ends_with(" · first of 4 links"), "{head}");
    let mut last = first.generation;
    for (want, line, place) in [
        ("/w/docs/y.md", None, "2 of 4"),
        ("/w/src/x.rs", Some(9), "3 of 4"),
        ("/w/src/x.rs", Some(1), "4 of 4"),
    ] {
        let _ = pressed(&mut app, b"n");
        let (ask, head) = landed(&mut app, &rx, "x\ny");
        assert_eq!((ask.path.as_str(), ask.line), (want, line));
        assert!(ask.generation > last, "each step is a new generation");
        last = ask.generation;
        assert!(head.ends_with(&format!(" · {place} links")), "{head}");
    }
    app.flash = None;
    let _ = pressed(&mut app, b"n");
    assert!(asks(&rx).is_empty(), "n at the last link sends nothing");
    assert!(app.flash.is_none(), "n at the last link flashes nothing");
    assert!(header(&mut app).ends_with(" · 4 of 4 links"), "no wrap");
    let _ = pressed(&mut app, b"p");
    let (ask, head) = landed(&mut app, &rx, "x\ny");
    assert_eq!((ask.path.as_str(), ask.line), ("/w/src/x.rs", Some(9)));
    assert_eq!(
        ask.generation,
        last + 1,
        "the stop at the end moved no generation"
    );
    assert!(head.ends_with(" · 3 of 4 links"), "{head}");
    let _ = pressed(&mut app, b"p");
    let _ = landed(&mut app, &rx, "x");
    let _ = pressed(&mut app, b"p");
    let (ask, head) = landed(&mut app, &rx, "x");
    assert_eq!((ask.path.as_str(), ask.line), ("/w/src/x.rs", Some(1)));
    assert!(
        head.ends_with(" · first of 4 links"),
        "p back to the first: {head}"
    );
    let _ = pressed(&mut app, b"p");
    assert!(asks(&rx).is_empty(), "p at the first link sends nothing");
    assert!(app.model.reader_open());
}

#[test]
fn a_late_answer_for_the_link_stepped_away_from_never_paints() {
    let (mut app, rx) = marked_app("see src/a.rs and src/b.rs");
    let _ = pressed(&mut app, b"r");
    let old = one_ask(&rx);
    let _ = pressed(&mut app, b"n");
    let new = one_ask(&rx);
    assert_eq!(new.path, "/w/src/b.rs");
    answer(&mut app, old.generation, "STALEBODY");
    let shown = whole(&frame_of(&mut app));
    assert!(!shown.contains("STALEBODY"), "{shown}");
    answer(&mut app, new.generation, "FRESHBODY");
    let shown = whole(&frame_of(&mut app));
    assert!(shown.contains("FRESHBODY"), "{shown}");
}

#[test]
fn each_step_opens_in_its_own_default_mode_from_the_top() {
    let (mut app, rx) = marked_app("docs/a.md docs/b.md:3 docs/c.md");
    let long = (1..=60)
        .map(|n| format!("para {n}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let _ = pressed(&mut app, b"r");
    let (_, head) = landed(&mut app, &rx, MD);
    assert!(head.contains(" · rendered · "), "{head}");
    let _ = pressed(&mut app, b"m");
    assert!(header(&mut app).contains(" · raw · "), "m shows raw");
    let _ = pressed(&mut app, b"n");
    let (ask, head) = landed(&mut app, &rx, &long);
    assert_eq!((ask.path.as_str(), ask.line), ("/w/docs/b.md", Some(3)));
    assert!(head.starts_with("Read · /w/docs/b.md:3 · raw · "), "{head}");
    let _ = pressed(&mut app, b"G");
    assert!(!header(&mut app).contains(" · rows 1-"), "G scrolled down");
    let _ = pressed(&mut app, b"n");
    let (ask, head) = landed(&mut app, &rx, &long);
    assert_eq!((ask.path.as_str(), ask.line), ("/w/docs/c.md", None));
    assert!(
        head.starts_with("Read · /w/docs/c.md · rendered · rows 1-"),
        "rendered again, from the top; m was not carried: {head}",
    );
}

#[test]
fn a_one_link_turn_names_no_place_and_n_p_send_nothing() {
    let (mut app, rx) = marked_app("only src/one.rs here");
    let _ = pressed(&mut app, b"r");
    let (_, head) = landed(&mut app, &rx, "x");
    assert!(!head.contains(" links"), "{head}");
    for key in [b"n", b"p"] {
        let _ = pressed(&mut app, key);
        assert!(asks(&rx).is_empty(), "nothing to step to");
    }
    assert!(app.model.reader_open());
}

#[test]
fn a_click_opened_reader_steps_from_the_clicked_link() {
    let (mut app, rx) = marked_app("src/a.rs docs/b.md src/c.rs");
    let at = cell_of(&mut app, "docs/b.md", 0);
    click(&mut app, at);
    let (ask, head) = landed(&mut app, &rx, MD);
    assert_eq!(ask.path, "/w/docs/b.md");
    assert!(head.ends_with(" · 2 of 3 links"), "{head}");
    let _ = pressed(&mut app, b"n");
    let (ask, head) = landed(&mut app, &rx, "x");
    assert_eq!(ask.path, "/w/src/c.rs");
    assert!(head.ends_with(" · 3 of 3 links"), "{head}");
    let _ = pressed(&mut app, b"p");
    let _ = landed(&mut app, &rx, MD);
    let _ = pressed(&mut app, b"p");
    let (ask, head) = landed(&mut app, &rx, "x");
    assert_eq!(ask.path, "/w/src/a.rs");
    assert!(head.ends_with(" · first of 3 links"), "{head}");
}

#[test]
fn a_click_on_the_duplicate_opens_its_own_occurrence() {
    let (mut app, rx) = marked_app(FOUR);
    let at = cell_of(&mut app, "src/x.rs:1", 1);
    click(&mut app, at);
    let (ask, head) = landed(&mut app, &rx, "x");
    assert_eq!((ask.path.as_str(), ask.line), ("/w/src/x.rs", Some(1)));
    assert!(
        head.ends_with(" · 4 of 4 links"),
        "the second x.rs:1, not the first: {head}"
    );
}

#[test]
fn the_place_is_named_while_loading_and_on_a_refused_read() {
    let (mut app, rx) = marked_app("see src/a.rs then docs/b.md:7 end");
    let at = cell_of(&mut app, "docs/b.md:7", 0);
    click(&mut app, at);
    let ask = one_ask(&rx);
    let head = header(&mut app);
    assert!(head.contains("loading"), "{head}");
    assert!(
        head.ends_with(" · 2 of 2 links"),
        "loading names the place: {head}"
    );
    app.answer(Answer::Read {
        generation: ask.generation,
        body: Body::Gap("refused: read /w/docs/b.md: not a regular file".to_owned()),
    });
    let head = header(&mut app);
    assert!(
        head.ends_with(" · 2 of 2 links"),
        "a refused read names the place: {head}"
    );
    let _ = pressed(&mut app, b"p");
    let ask = one_ask(&rx);
    assert_eq!(ask.path, "/w/src/a.rs");
    assert!(header(&mut app).ends_with(" · first of 2 links"));
}

#[test]
fn batched_r_n_p_q_read_in_order_and_close() {
    let (mut app, rx) = marked_app("x src/a.rs y src/b.rs z src/c.rs");
    let now = Instant::now();
    let keys = [b"r", b"n", b"n", b"p", b"q"]
        .iter()
        .map(|bytes| (input::Key::Text(bytes.to_vec()), now))
        .collect();
    let _ = take_keys(&mut app, keys);
    let mut reads = Vec::new();
    let mut closed = false;
    while let Ok(request) = rx.try_recv() {
        match request {
            Request::Read(Some(ask)) => {
                assert!(!closed, "no read after the close");
                reads.push(ask.path.clone());
            }
            Request::Read(None) => closed = true,
            _ => panic!("the batch asked the loader for something other than a read"),
        }
    }
    assert_eq!(
        reads,
        ["/w/src/a.rs", "/w/src/b.rs", "/w/src/c.rs", "/w/src/b.rs"]
    );
    assert!(closed, "q closed the reader");
    assert!(!app.model.reader_open());
}

// ---------------------------------------------------------------------------
// One list over every part
// ---------------------------------------------------------------------------

#[test]
fn a_card_lists_question_options_and_evidence_alike_for_r_and_click() {
    let body = "Pick per src/q.rs now: 1 vertical src/v.rs | 2 horizontal src/h.rs now.\n\ndetails: .local/x.md";
    let card = item(
        MICROS,
        Kind::Card {
            seat: "lead".to_owned(),
        },
        body,
    );
    let (mut app, rx) = app_with(vec![card.clone()], Some(0));
    let _ = pressed(&mut app, b"r");
    let (ask, head) = landed(&mut app, &rx, "x");
    assert_eq!(ask.path, "/w/src/q.rs");
    assert!(head.ends_with(" · first of 4 links"), "{head}");
    for (want, place) in [
        ("/w/src/v.rs", "2 of 4"),
        ("/w/src/h.rs", "3 of 4"),
        ("/w/.local/x.md", "4 of 4"),
    ] {
        let _ = pressed(&mut app, b"n");
        let (ask, head) = landed(&mut app, &rx, "x");
        assert_eq!(ask.path, want);
        assert!(head.ends_with(&format!(" · {place} links")), "{head}");
    }
    for (link, place) in [("src/h.rs", "3 of 4"), (".local/x.md", "4 of 4")] {
        let (mut app, rx) = app_with(vec![card.clone()], Some(0));
        let at = cell_of(&mut app, link, 0);
        click(&mut app, at);
        let (ask, head) = landed(&mut app, &rx, "x");
        assert_eq!(ask.path, format!("/w/{link}"));
        assert!(
            head.ends_with(&format!(" · {place} links")),
            "{link}: {head}"
        );
    }
}

#[test]
fn the_fold_never_shortens_the_list() {
    let mut lines = vec!["top src/top.rs".to_owned()];
    lines.extend((2..40).map(|n| format!("row {n}")));
    lines.push("deep docs/deep.md".to_owned());
    let body = lines.join("\n");
    let (mut app, rx) = marked_app(&body);
    let shown = whole(&frame_of(&mut app));
    assert!(
        !(shown.contains("src/top.rs") && shown.contains("docs/deep.md")),
        "the fold hides one of the two links:\n{shown}",
    );
    let _ = pressed(&mut app, b"r");
    let (ask, head) = landed(&mut app, &rx, "x");
    assert_eq!(ask.path, "/w/src/top.rs");
    assert!(head.ends_with(" · first of 2 links"), "{head}");
    let _ = pressed(&mut app, b"n");
    let (ask, head) = landed(&mut app, &rx, MD);
    assert_eq!(ask.path, "/w/docs/deep.md");
    assert!(head.ends_with(" · 2 of 2 links"), "{head}");
}

/// A lane of one console ask `ae-1` saying `asked`, then its close.
fn closed_app(asked: &str) -> (App, Receiver<Request>) {
    let ask = item(
        MICROS,
        Kind::Asked {
            to: "lead".to_owned(),
            id: "ae-1".to_owned(),
            uncertain: false,
            gap: None,
            via: None,
        },
        asked,
    );
    let close = item(
        MICROS + 1,
        Kind::Closed {
            id: "ae-1".to_owned(),
        },
        "",
    );
    app_with(vec![ask, close], Some(1))
}

/// `head`, then single-letter words up to character `upto`, then `path`.
fn padded(head: &str, upto: usize, path: &str) -> String {
    let mut text = head.to_owned();
    while text.chars().count() + 2 <= upto {
        text.push_str("w ");
    }
    while text.chars().count() < upto {
        text.push(' ');
    }
    text.push_str(path);
    text
}

#[test]
fn a_closed_ask_reads_its_excerpt_links_but_never_a_path_cut_at_80() {
    // The path starts at character 69: the excerpt keeps `src/app/rea`.
    let cut = padded("see src/early.rs and ", 69, "src/app/reader.rs tail");
    assert!(cut.chars().count() > 80);
    let (mut app, rx) = closed_app(&cut);
    let cells = frame_of(&mut app);
    let clipped = occurrences(&app, &cells, "src/app/rea");
    let last = clipped.last().expect("the cut path is drawn").clone();
    let (x, y) = *last.last().expect("cells");
    assert_eq!(
        cells[(x + 1, y)].symbol(),
        "…",
        "the closed excerpt draws the cut path:\n{}",
        whole(&cells),
    );
    let _ = pressed(&mut app, b"r");
    let (ask, head) = landed(&mut app, &rx, "x");
    assert_eq!(ask.path, "/w/src/early.rs");
    assert!(
        !head.contains(" links"),
        "the cut path is not listed: {head}"
    );
    let (mut app, rx) = closed_app(&cut);
    let _ = frame_of(&mut app);
    for at in last {
        click(&mut app, at);
        assert!(
            reads_only(&rx).is_empty(),
            "the cut path's cell {at:?} reads nothing"
        );
        assert!(!app.model.reader_open());
    }
}

#[test]
fn a_closed_ask_leaves_out_a_path_ending_at_the_cut_and_keeps_one_not_cut() {
    let at80 = padded("see src/early.rs and ", 69, "docs/end.md");
    assert_eq!(at80.chars().count(), 80);
    let cut = format!("{at80} and more words after");
    let (mut app, rx) = closed_app(&cut);
    let _ = pressed(&mut app, b"r");
    let (ask, head) = landed(&mut app, &rx, "x");
    assert_eq!(ask.path, "/w/src/early.rs");
    assert!(
        !head.contains(" links"),
        "a path ending at the cut is left out: {head}"
    );
    let (mut app, rx) = closed_app(&at80);
    let _ = pressed(&mut app, b"r");
    let (ask, head) = landed(&mut app, &rx, "x");
    assert_eq!(ask.path, "/w/src/early.rs");
    assert!(
        head.ends_with(" · first of 2 links"),
        "nothing was cut: {head}"
    );
    let _ = pressed(&mut app, b"n");
    let (ask, _) = landed(&mut app, &rx, MD);
    assert_eq!(ask.path, "/w/docs/end.md");
}

#[test]
fn an_accepted_turn_has_no_file_link() {
    let ask = item(
        MICROS,
        Kind::Asked {
            to: "lead".to_owned(),
            id: "ae-1".to_owned(),
            uncertain: false,
            gap: None,
            via: None,
        },
        "see src/a.rs",
    );
    let accepted = item(
        MICROS + 1,
        Kind::Accepted {
            seat: "lead".to_owned(),
            id: "ae-1".to_owned(),
        },
        "on it, see src/z.rs",
    );
    let (mut app, rx) = app_with(vec![ask, accepted], None);
    let keys = app.keys();
    app.model.set_turn(Some(keys[1]));
    let _ = pressed(&mut app, b"r");
    assert!(asks(&rx).is_empty(), "an accepted turn reads nothing");
    let flash = app
        .flash
        .as_ref()
        .map(|(words, _)| words.clone())
        .unwrap_or_default();
    assert!(flash.contains("no file link in that turn"), "{flash:?}");
}

// ---------------------------------------------------------------------------
// Close returns as today; writing survives
// ---------------------------------------------------------------------------

#[test]
fn a_click_opened_reader_closes_back_to_the_same_lane_and_mark() {
    let (mut app, rx) = marked_app(TWO);
    let mark = app.model.turn();
    let before = whole(&frame_of(&mut app));
    let at = cell_of(&mut app, "src/b.rs", 0);
    click(&mut app, at);
    let _ = landed(&mut app, &rx, "x");
    let _ = pressed(&mut app, b"q");
    assert!(!app.model.reader_open());
    let (reads, closes) = effects(&rx);
    assert!(reads.is_empty(), "the close reads nothing");
    assert_eq!(closes, 1, "the close tells the reader thread once");
    assert_eq!(app.model.turn(), mark);
    assert_eq!(
        whole(&frame_of(&mut app)),
        before,
        "the lane is drawn as it was"
    );
}

#[test]
fn a_click_from_a_scrolled_lane_closes_back_to_the_same_place() {
    let filler = (1..=10)
        .map(|n| format!("filler {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut items = vec![said(MICROS, &format!("src/a1.rs\n{filler}\ndocs/a2.md"))];
    for k in 1..=12 {
        items.push(said(MICROS + k, &format!("later {k}\nmore {k}\nend {k}")));
    }
    let (mut app, rx) = app_with(items, None);
    let bottom = whole(&frame_of(&mut app));
    let lane = app.layout.lane_area().expect("the lane is drawn");
    let mut shown = bottom.clone();
    for _ in 0..40 {
        if shown.contains("docs/a2.md") {
            break;
        }
        mouse_at(
            &mut app,
            input::MouseKind::WheelUp,
            (lane.x + 2, lane.y + 2),
            Instant::now(),
        );
        shown = whole(&frame_of(&mut app));
    }
    let mark = app.keys()[1];
    app.model.set_turn(Some(mark));
    app.model.set_focus(Some("lead".to_owned()));
    let before = whole(&frame_of(&mut app));
    assert!(
        before.contains("docs/a2.md") && !before.contains("src/a1.rs") && before != bottom,
        "the lane is scrolled up, the turn's first link above the view:\n{before}",
    );
    assert!(asks(&rx).is_empty());
    let at = cell_of(&mut app, "docs/a2.md", 0);
    click(&mut app, at);
    let (ask, head) = landed(&mut app, &rx, MD);
    assert_eq!(ask.path, "/w/docs/a2.md");
    assert!(head.ends_with(" · 2 of 2 links"), "{head}");
    let _ = pressed(&mut app, b"p");
    let (ask, head) = landed(&mut app, &rx, "x");
    assert_eq!(
        ask.path, "/w/src/a1.rs",
        "the link the scroll hid stays listed"
    );
    assert!(head.ends_with(" · first of 2 links"), "{head}");
    let _ = pressed(&mut app, b"q");
    let (reads, closes) = effects(&rx);
    assert!(reads.is_empty(), "the close reads nothing");
    assert_eq!(closes, 1, "the close tells the reader thread once");
    assert_eq!(app.model.turn(), Some(mark), "the mark is kept");
    assert_eq!(app.model.focus(), Some("lead"), "the focus is kept");
    assert_eq!(
        whole(&frame_of(&mut app)),
        before,
        "the lane is back where it was scrolled",
    );
}

#[test]
fn a_click_while_writing_opens_the_link_and_the_close_keeps_the_draft() {
    let root = super::tests::Root::new("navigate-compose");
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::new(Some("api".to_owned()), None, None, Some(tx));
    (app.fleeted, app.loading) = (true, false);
    let reader = super::loader::Reader::new(root.0.clone(), Some("api".to_owned()), None);
    let dirs = std::iter::once(("api".to_owned(), root.0.join("sessions").join("api"))).collect();
    let world = crate::listing::World::new(
        Timestamp::now(),
        vec![super::tests::entry("api", Status::Running, None)],
    );
    let order = crate::theme::FleetOrder::EMPTY;
    let read = reader.fold(dirs, world, None, &order, Timestamp::now());
    app.answer(Answer::Fleet(read));
    app.answer(Answer::View(super::loader::ViewRead {
        name: "api".to_owned(),
        id: super::tests::ID.to_owned(),
        seq: 1,
        lane: Lane {
            items: vec![said(MICROS, "see /abs/f.rs now")],
            coverage: Vec::new(),
        },
        needs: None,
        roster: None,
    }));
    app.take(input::Reading::Owner, Instant::now());
    app.write(true);
    assert!(app.composing(), "writing started");
    assert_eq!(
        app.compose(input::Key::Text(b"draft words".to_vec()), Instant::now()),
        Some(())
    );
    let _ = reads_only(&rx);
    let at = cell_of(&mut app, "/abs/f.rs", 0);
    click(&mut app, at);
    assert!(app.composing(), "the click kept the writing");
    let (ask, _) = landed(&mut app, &rx, "x");
    assert_eq!(ask.path, "/abs/f.rs");
    let _ = take_keys(&mut app, vec![(input::Key::Escape, Instant::now())]);
    assert!(!app.model.reader_open(), "Esc closes the reader");
    let (reads, closes) = effects(&rx);
    assert!(reads.is_empty(), "the close reads nothing");
    assert_eq!(closes, 1, "the close tells the reader thread once");
    assert!(app.composing(), "still writing after the close");
    let draft = app
        .shown_input()
        .map(input::Input::draft)
        .unwrap_or_default();
    assert_eq!(draft, "draft words");
}

// ---------------------------------------------------------------------------
// Keys overlay
// ---------------------------------------------------------------------------

#[test]
fn the_keys_overlay_names_n_p_in_the_reader_section() {
    let (mut app, _rx) = marked_app(TWO);
    let _ = pressed(&mut app, b",");
    let _ = pressed(&mut app, b"5");
    let mut cells = Buffer::empty(Rect::new(0, 0, 160, 120));
    app.frame(&mut cells);
    let shown = whole(&cells);
    let rows: Vec<&str> = shown.lines().map(str::trim).collect();
    let start = rows
        .iter()
        .position(|row| row.contains("Reader (r on a marked turn)"))
        .unwrap_or_else(|| panic!("reader section drawn:\n{shown}"));
    let end = (start + 1..rows.len())
        .find(|&at| rows[at].contains("Settings (the gear"))
        .unwrap_or(rows.len());
    assert!(
        rows[start + 1..end].iter().any(|row| row.contains("n/p")),
        "the reader keys name n/p:\n{}",
        rows[start..end].join("\n"),
    );
}
