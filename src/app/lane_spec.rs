//! Frozen phase-2 acceptance spec. `rows` is the unchanged full renderer,
//! retained only under cfg(test); expectations never come from `tail`.
//! The visit observer is filled at the actual body-wrap site, without clocks,
//! environment doors or global state. The old poison is deliberately small:
//! visiting it is the failure, regardless of CPU speed.

#![allow(
    clippy::expect_used,
    reason = "pure fixtures use bounded indices and pane dimensions"
)]

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::Style;
use ratatui_core::text::Line;

use super::draw::Paint;
use super::fleet::{Counts, Fleet, Line2, Row};
use super::lane::{rows, tail};
use super::loader::{Answer, ViewRead, Wake};
use super::model::Model;
use super::{App, drain};
use crate::console::input::Keys;
use crate::console::lane::{Item, Kind, Lane};
use crate::console::view;
use crate::digest::{SessionEntry, Status};
use crate::listing::World;
use crate::theme::{Look, Mark};
use crate::time::Timestamp;

const ID: &str = "0199c0de-1234-4890-abcd-ef0123456789";

fn kind(index: usize) -> Kind {
    let seat = || "lead".to_owned();
    let id = || "ask-id".to_owned();
    match index % 12 {
        0 => Kind::Pane {
            seat: seat(),
            generation: 2,
        },
        1 => Kind::Assistant {
            seat: seat(),
            generation: 1,
        },
        2 => Kind::Inbound {
            from: "human".to_owned(),
            to: seat(),
        },
        3 => Kind::Reply {
            from: seat(),
            to: "human".to_owned(),
        },
        4 => Kind::Said {
            who: "ae".to_owned(),
        },
        5 => Kind::Card { seat: seat() },
        6 => Kind::NeedsYou { seat: seat() },
        7 => Kind::Asked {
            to: seat(),
            id: id(),
            uncertain: true,
        },
        8 => Kind::NotDelivered {
            to: seat(),
            id: id(),
        },
        9 => Kind::Closed { id: id() },
        10 => Kind::Answer {
            seat: seat(),
            id: id(),
            follow_up: 2,
            late: true,
            gap: Some("reply unread".to_owned()),
            speaker: Some("new-profile".to_owned()),
        },
        _ => Kind::Unadmitted {
            from: seat(),
            id: id(),
            why: "old owner".to_owned(),
        },
    }
}

fn mixed() -> Lane {
    let bodies = [
        "",
        "short",
        "a word boundary then more words",
        "one\n\nthree\n",
        "界 e\u{301} 🦀\tend",
        "\u{1b}[31m inert \r body",
    ];
    Lane {
        items: (0..12)
            .map(|index| Item {
                micros: (i64::try_from(index).expect("fixture index") - 1) * 60_000_000,
                kind: kind(index),
                body: bodies[index % bodies.len()].to_owned(),
                record: None,
            })
            .collect(),
        coverage: vec!["older gap".to_owned(), "second \u{1b} gap".to_owned()],
    }
}

fn window<'a>(rows: &'a [Line<'static>], scroll: usize, page: usize) -> &'a [Line<'static>] {
    let last = rows.len().saturating_sub(scroll).max(page.min(rows.len()));
    &rows[last.saturating_sub(page)..last]
}

/// All row offsets, including splits through headers, bodies and separators,
/// preserve both text and style. Width/look/zone changes are fresh inputs.
#[test]
fn every_window_is_equivalent_to_the_unchanged_full_renderer() {
    let lane = mixed();
    let looks = [
        (None, None),
        (Some(Look::read("on", "darcula", "on", "on")), Some("+0530")),
        (
            Some(Look::read("on", "darcula", "off", "on")),
            Some("-0700"),
        ),
        (
            Some(Look::read("on", "darcula", "on", "on")),
            Some("bad-zone"),
        ),
    ];
    for width in [1, 2, 11, 51, 111] {
        for (look, zone) in looks {
            let paint = Paint::of(look.as_ref());
            let clock = view::Style::resolve(true, look, zone, "lead");
            let full = rows(&lane, width, paint, &clock, "lead");
            for page in [1, 7, 23] {
                for scroll in 0..=full.len() + page {
                    let need = scroll + 2 * page;
                    let part = tail(&lane, width, paint, &clock, "lead", need, None);
                    assert!(
                        part.rows.len() >= need.min(full.len()),
                        "missing requested rows"
                    );
                    assert!(part.rows.len() <= full.len(), "invented rows");
                    assert_eq!(
                        part.rows,
                        full[full.len() - part.rows.len()..],
                        "not a suffix"
                    );
                    assert_eq!(part.complete, part.rows.len() == full.len(), "false top");
                    assert_eq!(
                        window(&part.rows, scroll, page),
                        window(&full, scroll, page),
                        "width {width}, zone {zone:?}, scroll {scroll}, page {page}"
                    );
                }
            }
        }
    }
}

fn turns(count: usize) -> Lane {
    Lane {
        items: (0..count)
            .map(|index| Item {
                micros: i64::try_from(index).expect("fixture index") * 60_000_000,
                kind: Kind::Said {
                    who: "lead".to_owned(),
                },
                body: format!("turn-{index:04}"),
                record: None,
            })
            .collect(),
        coverage: vec!["oldest gap".to_owned(), "another gap".to_owned()],
    }
}

/// The newest page must not visit a hidden old body; reaching it later still
/// renders exactly the full reference. No payload timing or RAM stress.
#[test]
fn old_poison_is_unvisited_until_scroll_reaches_it() {
    let mut lane = turns(80);
    lane.items[0].body = "OLD POISON\n  preserved indentation\n界 old body".to_owned();
    let paint = Paint::of(None);
    let clock = view::Style::PLAIN;
    let full = rows(&lane, 51, paint, &clock, "lead");
    let mut visits = Vec::new();
    let newest = tail(&lane, 51, paint, &clock, "lead", 32, Some(&mut visits));
    assert!(
        !visits.contains(&0),
        "newest page wrapped hidden old poison: {visits:?}"
    );
    assert!(!visits.is_empty(), "observer must attest real wrapping");
    let mut unique = visits.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(visits.len(), unique.len(), "one body wrapped twice");
    assert_eq!(newest.rows, full[full.len() - newest.rows.len()..]);
    assert!(!newest.complete, "oldest rows have not been produced");
    visits.clear();
    let oldest = tail(
        &lane,
        51,
        paint,
        &clock,
        "lead",
        full.len() + 32,
        Some(&mut visits),
    );
    assert!(visits.contains(&0), "scroll reaching poison must wrap it");
    assert!(oldest.complete);
    assert_eq!(oldest.rows, full, "scroll loses old content");
}

/// No displayed rows means no body work, including a long lane.
#[test]
fn a_zero_row_request_wraps_no_items() {
    let lane = turns(200);
    let mut visits = Vec::new();
    let part = tail(
        &lane,
        80,
        Paint::of(None),
        &view::Style::PLAIN,
        "lead",
        0,
        Some(&mut visits),
    );
    assert!(
        visits.is_empty(),
        "zero-row request wrapped bodies: {visits:?}"
    );
    assert!(part.rows.is_empty());
    assert!(!part.complete);
    let empty = Lane::default();
    let part = tail(
        &empty,
        80,
        Paint::of(None),
        &view::Style::PLAIN,
        "lead",
        0,
        None,
    );
    assert!(part.complete && part.rows.is_empty());
}

/// Coverage is also lane content: thousands of hidden one-row gaps must not
/// be materialized for a small window, and the true oldest gap remains reachable.
#[test]
fn coverage_only_work_is_bounded_by_requested_rows() {
    let lane = Lane {
        items: Vec::new(),
        coverage: (0..4096).map(|index| format!("gap-{index:04}")).collect(),
    };
    let paint = Paint::of(None);
    let clock = view::Style::PLAIN;
    let full = rows(&lane, 80, paint, &clock, "lead");
    for need in [0, 1, 7, 31, 4096, usize::MAX] {
        let mut visits = Vec::new();
        let part = tail(&lane, 80, paint, &clock, "lead", need, Some(&mut visits));
        let count = need.min(full.len());
        assert_eq!(
            part.rows.len(),
            count,
            "coverage request {need} materialized hidden gaps"
        );
        assert_eq!(part.rows, full[full.len() - count..]);
        assert_eq!(part.complete, count == full.len());
        assert!(visits.is_empty(), "coverage has no item body");
    }
}

fn app(lane: Lane) -> App {
    let mut app = App::new(None, None, None, None);
    app.fleet = Fleet {
        rows: vec![Row {
            name: "api".to_owned(),
            index: 1,
            mark: Mark::Working,
            needy: false,
            counts: Counts::Unknown,
            line2: Line2::NoGoal,
            home: false,
        }],
        home: None,
    };
    app.model = Model::new(&app.fleet);
    app.fleeted = true;
    let mut entry = SessionEntry::new("api", Status::Stopped);
    entry.main_agent = Some("lead".to_owned());
    app.world = World::new(Timestamp::from_epoch(0), vec![entry]);
    app.dirs
        .insert("api".to_owned(), PathBuf::from("pure-fixture-record"));
    app.ids.insert("api".to_owned(), ID.to_owned());
    app.answer(Answer::View(ViewRead {
        name: "api".to_owned(),
        id: ID.to_owned(),
        seq: 1,
        lane,
        needs: None,
        roster: None,
    }));
    app
}

const AREA: Rect = Rect::new(0, 0, 160, 37);
const PAGE: usize = 27;

fn frame(app: &mut App) -> Buffer {
    let mut buf = Buffer::empty(AREA);
    app.frame(&mut buf);
    buf
}

fn assert_window(buf: &Buffer, full: &[Line<'static>], scroll: usize) {
    let mut expected = Buffer::empty(AREA);
    let visible = window(full, scroll, PAGE);
    let count = u16::try_from(visible.len()).expect("one page");
    for (offset, line) in visible.iter().enumerate() {
        expected.set_line(
            47,
            30 - count + u16::try_from(offset).expect("one page"),
            line,
            111,
        );
    }
    let last = full.len().saturating_sub(scroll).max(PAGE.min(full.len()));
    if scroll > 0 && last < full.len() {
        expected.set_stringn(47, 30, "↓ newer turns below · PgDn", 111, Style::new());
    }
    for y in 3..=30 {
        for x in 47..158 {
            assert_eq!(
                buf[(x, y)],
                expected[(x, y)],
                "scroll {scroll}, cell {x},{y}"
            );
        }
    }
}

fn wheel(up: bool, count: usize) -> Vec<u8> {
    format!("\u{1b}[<{};48;6M", if up { 64 } else { 65 })
        .repeat(count)
        .into_bytes()
}

/// Replay through the same drain/frame boundary as `run()`. A lazy renderer may
/// defer a suffix of keys while it extends the known bound; all must survive.
fn burst(app: &mut App, bytes: Vec<u8>) -> Buffer {
    let (_sender, wakes) = mpsc::channel();
    let mut keys = Keys::app();
    let mut first = Some(Wake::Keys(Instant::now(), bytes));
    let mut buf = frame(app);
    for _ in 0..32 {
        assert!(
            drain(app, &mut keys, &wakes, &mut first).is_some(),
            "unexpected quit"
        );
        app.focus();
        buf = frame(app);
    }
    buf
}

#[test]
fn wheel_burst_before_a_frame_loses_no_scroll_steps() {
    let lane = turns(160);
    let full = rows(&lane, 111, Paint::of(None), &view::Style::PLAIN, "lead");
    let mut app = app(lane);
    let buf = burst(&mut app, wheel(true, 40));
    assert_eq!(
        app.model.scroll_rows(PAGE),
        120,
        "clamped at a temporary false top"
    );
    assert_window(&buf, &full, 120);
}

#[test]
fn wheel_crossing_unknown_top_then_down_keeps_sequential_clamping() {
    let lane = turns(80);
    let full = rows(&lane, 111, Paint::of(None), &view::Style::PLAIN, "lead");
    let top = full.len() - PAGE;
    let mut app = app(lane);
    let mut bytes = wheel(true, 200);
    bytes.extend(wheel(false, 7));
    let buf = burst(&mut app, bytes);
    assert_eq!(
        app.model.scroll_rows(PAGE),
        top - 21,
        "lost Down after crossing true top"
    );
    assert_window(&buf, &full, top - 21);
}

#[test]
fn the_true_top_shows_oldest_content_stops_and_accepts_down() {
    let lane = turns(80);
    let full = rows(&lane, 111, Paint::of(None), &view::Style::PLAIN, "lead");
    let top = full.len() - PAGE;
    let mut app = app(lane);
    let buf = burst(&mut app, wheel(true, 200));
    assert_eq!(app.model.scroll_rows(PAGE), top);
    assert_window(&buf, &full, top);
    let buf = burst(&mut app, wheel(true, 200));
    assert_eq!(
        app.model.scroll_rows(PAGE),
        top,
        "top moved under another Up burst"
    );
    assert_window(&buf, &full, top);
    let mut bytes = wheel(true, 40);
    bytes.extend(wheel(false, 1));
    let buf = burst(&mut app, bytes);
    assert_eq!(app.model.scroll_rows(PAGE), top - 3);
    assert_window(&buf, &full, top - 3);
}

#[test]
fn interrupt_survives_a_deferred_wheel_burst() {
    let mut app = app(turns(80));
    let _ = frame(&mut app);
    let mut bytes = wheel(true, 200);
    bytes.push(3);
    let (_sender, wakes) = mpsc::channel();
    let mut keys = Keys::app();
    let mut first = Some(Wake::Keys(Instant::now(), bytes));
    for _ in 0..32 {
        if drain(&mut app, &mut keys, &wakes, &mut first).is_none() {
            return;
        }
        let _ = frame(&mut app);
        app.focus();
    }
    panic!("Interrupt stranded behind deferred wheel events");
}
