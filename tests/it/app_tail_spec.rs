//! Phase-2 frame oracle: the public draw path must keep the full renderer's
//! chat cells. Rows below are reconstructed from short, explicitly broken
//! fixture lines; they call neither the new tail renderer nor a render helper.

#![allow(
    clippy::expect_used,
    reason = "pure fixtures use bounded indices and pane dimensions"
)]

use ae::app::draw::{Composer, Screen, draw};
use ae::app::fleet::{Counts, Fleet, Line2, Row};
use ae::app::model::{Key, Model};
use ae::app::overview::Overview;
use ae::console::lane::{Item, Kind, Lane};
use ae::digest::{SessionEntry, Status};
use ae::theme::Mark;
use ae::time::Timestamp;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::Style;

fn fixture() -> (Lane, Vec<String>) {
    let mut lane = Lane {
        items: Vec::new(),
        coverage: vec!["fixture gap zero".to_owned(), "fixture gap one".to_owned()],
    };
    let mut rows: Vec<String> = lane
        .coverage
        .iter()
        .map(|gap| format!("coverage incomplete: {gap}"))
        .collect();
    for index in 0..37 {
        // Even at the narrowest tested pane, these ASCII lines fit intact.
        let lines: Vec<String> = (0..=index % 5)
            .map(|line| format!("turn-{index:02}-line-{line}"))
            .collect();
        lane.items.push(Item {
            micros: index * 60_000_000,
            kind: Kind::Said {
                who: "lead".to_owned(),
            },
            body: lines.join("\n"),
            record: None,
        });
        rows.push(String::new());
        rows.push(format!("lead  {:02}:{:02}  said", index / 60, index % 60));
        rows.extend(lines.into_iter().map(|line| format!("  {line}")));
    }
    (lane, rows)
}

fn fleet() -> Fleet {
    Fleet {
        rows: vec![Row {
            name: "api".to_owned(),
            index: 1,
            mark: Mark::Working,
            needy: false,
            counts: Counts::Unknown,
            line2: Line2::NoGoal,
            home: true,
        }],
        home: Some("api".to_owned()),
    }
}

fn chat_left(width: u16, height: u16) -> u16 {
    if width >= 140 && height >= 20 {
        47
    } else if width >= 90 && height >= 20 {
        37
    } else {
        2
    }
}

/// Project the independently reconstructed FULL sequence onto one window.
fn reference(rows: &[String], area: Rect, scroll: usize) -> Buffer {
    let (left, bottom) = (chat_left(area.width, area.height), area.height - 6);
    let room = area.width - 2 - left;
    let page = usize::from(area.height - 9);
    let last = rows.len().saturating_sub(scroll).max(page.min(rows.len()));
    let first = last.saturating_sub(page);
    let count = u16::try_from(last - first).expect("a page fits the pane");
    let mut buf = Buffer::empty(area);
    for (offset, row) in rows[first..last].iter().enumerate() {
        buf.set_stringn(
            left,
            bottom - count + u16::try_from(offset).expect("a row fits the pane"),
            row,
            usize::from(room),
            Style::new(),
        );
    }
    if scroll > 0 && last < rows.len() {
        buf.set_stringn(
            left,
            bottom,
            "↓ newer turns below · PgDn",
            usize::from(room),
            Style::new(),
        );
    }
    buf
}

/// Public `draw`, across sidebar thresholds and keyboard page offsets,
/// paints precisely the full reference's rows, styles, blank separators,
/// coverage, bottom anchoring and newer-turns marker.
#[test]
fn drawn_chat_equals_the_full_reference_at_bottom_middle_and_top() {
    let (lane, rows) = fixture();
    let fleet = fleet();
    let overview = Overview::default();
    let mut entry = SessionEntry::new("api", Status::Stopped);
    entry.main_agent = Some("lead".to_owned());
    let pair = ["lead".to_owned(), "colead".to_owned()];
    for (width, height) in [
        (40, 13),
        (89, 19),
        (90, 20),
        (139, 36),
        (140, 36),
        (160, 43),
    ] {
        let area = Rect::new(0, 0, width, height);
        let page = usize::from(height - 9);
        let mut model = Model::new(&fleet);
        for pages in 0..=rows.len().div_ceil(page) + 2 {
            let screen = Screen {
                fleet: &fleet,
                model: &model,
                overview: &overview,
                selected: Some(&entry),
                pair: &pair,
                agents: None,
                lane: &lane,
                composer: Composer::Home {
                    home: "api",
                    speaker: "lead",
                    view: None,
                    draft: "",
                },
                look: None,
                zone: None,
                now: Timestamp::from_epoch(0),
            };
            let mut actual = Buffer::empty(area);
            draw(&screen, &mut actual);
            let expected = reference(&rows, area, pages * page);
            for y in 3..=height - 6 {
                for x in chat_left(width, height)..width - 2 {
                    assert_eq!(
                        actual[(x, y)],
                        expected[(x, y)],
                        "chat cell ({x},{y}), pane {width}x{height}, page {pages}"
                    );
                }
            }
            let _ = model.key(Key::PageUp, &fleet, false, false);
        }
    }
}
