//! Frozen acceptance additions for the inner-tab mutants, authored by
//! navigator `skilltab-nav` from the requirement after the inner-tabs
//! implementation landed. The frozen spec `skilltab_spec` (r5, r6)
//! already pins what each inner shows; these six tests pin the
//! wirings no behavioural test of that round could see: clicking an
//! inner title is what shows it, the whole title extent clicks, the
//! gaps do not, the shown inner carries the selected style, the
//! overflow stop agrees, and arrows move the inner only on
//! Statistics. Each is shown to fail on its mutant.
//! Install as `src/app/skilltab_inner_spec.rs` and wire from `src/app.rs`
//! under `cfg(test)`. All drives go through `take_keys`, `App::frame`
//! and `App::settings_mouse`; nothing new is named but what the app
//! crate already has. Every assertion here is RED wherever the inner
//! wiring is absent (including `612a485e`, which has no inner row).
#![allow(clippy::expect_used, reason = "frozen acceptance assertions")]

use std::sync::mpsc;
use std::time::Instant;

use ratatui_core::{buffer::Buffer, layout::Rect};

use super::loader::{Answer, Request};
use super::{App, take_keys};
use crate::console::input::{Key, Mouse, MouseKind};
use crate::stats::{self, Cell, Source};
use crate::stats_fixture as f;
use crate::tool_stats::*;

fn key(app: &mut App, text: &str) {
    let _ = take_keys(
        app,
        vec![(Key::Text(text.as_bytes().to_vec()), Instant::now())],
    );
}

fn arrow(app: &mut App, key: Key) {
    let _ = take_keys(app, vec![(key, Instant::now())]);
}

fn render(app: &mut App, width: u16, height: u16) -> (String, Buffer) {
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
    app.frame(&mut buffer);
    let text = (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    (text, buffer)
}

fn frame(app: &mut App, width: u16) -> String {
    render(app, width, 90).0
}

fn answer_view(page: Page) -> View {
    let item = match page {
        Page::Skills => Item::Skill {
            name: Ident::Name("fixture-skill".into()),
            scope: None,
        },
        Page::Mcp => Item::Mcp {
            server: Ident::Name("fixture-server".into()),
            tool: Ident::Name("search".into()),
        },
        Page::Tools => Item::Tool(Ident::Name("Read".into())),
    };
    let totals = match page {
        Page::Skills => Cols::Skill(SkillCols {
            attempts: 1,
            unresolved: 1,
            ..SkillCols::default()
        }),
        Page::Mcp => Cols::Mcp(McpCols {
            attempts: 1,
            returned: 1,
            success: 1,
            ..McpCols::default()
        }),
        Page::Tools => Cols::Tool(ToolCols {
            attempts: 1,
            unresolved: 1,
            ..ToolCols::default()
        }),
    };
    View {
        ask: Ask {
            page,
            scope: Scope::All,
            filter: Filter::All,
        },
        state: State::Numbers,
        rows: vec![Row {
            item: item.clone(),
            cols: totals,
        }],
        more: None,
        items: 1,
        totals,
        harnesses: vec![HarnessGroup {
            harness: Adapter::Claude,
            items: 1,
            totals,
            rows: vec![Row {
                item: item.clone(),
                cols: totals,
            }],
            more: None,
        }],
        orphans: 0,
        detail: vec![DetailRow {
            item,
            harness: Adapter::Claude,
            client: Cell::Value("work".into()),
            model: Cell::Value("model-a".into()),
            effort: Cell::Unknown,
            attempts: 1,
        }],
        detail_more: None,
        range: Some((1_793_491_200_000_000, 1_793_577_600_000_000)),
        read: 1,
        listed: 1,
        recounting: 0,
        gaps: Vec::new(),
        filters: vec![Filter::All, Filter::Client(Cell::Value("work".into()))],
    }
}

fn land(app: &mut App, generation: u64, view: View) {
    app.answer(Answer::Tools {
        generation,
        view: Box::new(view),
    });
}

fn land_stats(app: &mut App, generation: u64) {
    let (s, t) = f::matrix();
    let view = stats::fold(
        vec![
            Source::live("S", format!("session_id={}\n", f::S).as_bytes(), Ok(s)),
            Source::live("T", format!("session_id={}\n", f::T).as_bytes(), Ok(t)),
        ],
        Some(f::S),
    );
    app.answer(Answer::Stats {
        generation,
        glance: Box::new(crate::stats_store::disk::Glance::Ready {
            view,
            epoch: 1,
            r#gen: 1,
            updating: false,
            all: crate::stats_store::disk::Progress {
                listed: 2,
                pending: 0,
            },
            this: crate::stats_store::disk::ThisProgress::Ready(
                crate::stats_store::disk::Progress {
                    listed: 1,
                    pending: 0,
                },
            ),
        }),
    });
}

fn open_app() -> (App, mpsc::Receiver<Request>, u64) {
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(None, None, None, Some(tx));
    app.fleeted = true;
    app.open_settings();
    let generation = app.model.settings().expect("open overlay").generation;
    (app, rx, generation)
}

/// A click at the drawn column, row 2.
fn click(app: &mut App, column: u16) -> bool {
    app.settings_mouse(Mouse {
        kind: MouseKind::Click,
        column,
        row: 2,
    })
}

fn tools_at(requests: &[Request], generation: u64, page: Page) -> bool {
    requests.iter().any(|request| {
        matches!(
            request,
            Request::Tools {
                generation: seen,
                shown: true,
                ask,
            } if *seen == generation && ask.page == page && ask.scope == Scope::All && ask.filter == Filter::All
        )
    })
}

/// The full style of one buffer cell, as one comparable key.
fn style_at(buffer: &Buffer, column: u16, row: u16) -> String {
    let cell = &buffer[(column, row)];
    format!("{:?}|{:?}|{:?}", cell.fg, cell.bg, cell.modifier)
}

/// The drawn start column of `title` on `line`.
fn start_of(line: &str, title: &str) -> u16 {
    let at = line
        .find(title)
        .unwrap_or_else(|| panic!("{title} missing from {line:?}"));
    u16::try_from(at).expect("columns fit in u16")
}

#[test]
fn clicking_an_inner_title_shows_that_inner() {
    for width in [120_u16, 40_u16] {
        let (mut app, rx, generation) = open_app();
        key(&mut app, "7");
        land_stats(&mut app, generation);
        assert!(frame(&mut app, width).contains("Recorded worker spawns: 12"));
        assert!(click(&mut app, 13), "the Skills click acts");
        let _ = frame(&mut app, width);
        land(&mut app, generation, answer_view(Page::Skills));
        assert!(
            frame(&mut app, width).contains("fixture-skill"),
            "the Skills click shows loaded skills at width {width}"
        );
        assert!(click(&mut app, 20), "the Tools click acts");
        let _ = frame(&mut app, width);
        land(&mut app, generation, answer_view(Page::Mcp));
        let tools = frame(&mut app, width);
        assert!(
            tools.contains("fixture-server"),
            "clicking Tools lands on the MCP servers page at width {width}: {tools}"
        );
        assert!(
            tools_at(&rx.try_iter().collect::<Vec<_>>(), generation, Page::Mcp),
            "the MCP servers watch is armed"
        );
        let shown = frame(&mut app, width);
        assert!(
            !click(&mut app, 20),
            "a click on the shown title changes nothing"
        );
        assert_eq!(
            frame(&mut app, width),
            shown,
            "the frame is identical after the no-op click"
        );
    }
}

#[test]
fn the_edges_of_each_inner_title_click_and_the_gaps_do_not() {
    let (mut app, _rx, generation) = open_app();
    key(&mut app, "7");
    land_stats(&mut app, generation);
    let _ = frame(&mut app, 180);
    // title extents on the row: Agents [2, 8), Skills [10, 16), Tools [18, 23)
    assert!(click(&mut app, 10), "the first cell of Skills clicks");
    land(&mut app, generation, answer_view(Page::Skills));
    assert!(frame(&mut app, 180).contains("fixture-skill"));
    assert!(click(&mut app, 22), "the last cell of Tools clicks");
    let _ = frame(&mut app, 180);
    land(&mut app, generation, answer_view(Page::Mcp));
    assert!(frame(&mut app, 180).contains("fixture-server"));
    assert!(click(&mut app, 7), "the last cell of Agents clicks");
    let _ = frame(&mut app, 180);
    assert!(frame(&mut app, 180).contains("Recorded worker spawns: 12"));
    assert!(click(&mut app, 15), "the last cell of Skills clicks");
    let _ = frame(&mut app, 180);
    land(&mut app, generation, answer_view(Page::Skills));
    assert!(frame(&mut app, 180).contains("fixture-skill"));
    assert!(click(&mut app, 2), "the first cell of Agents clicks");
    let _ = frame(&mut app, 180);
    assert!(frame(&mut app, 180).contains("Recorded worker spawns: 12"));
    assert!(click(&mut app, 18), "the first cell of Tools clicks");
    let _ = frame(&mut app, 180);
    land(&mut app, generation, answer_view(Page::Mcp));
    assert!(frame(&mut app, 180).contains("fixture-server"));
    // the gaps and right of Tools select nothing, placed off the
    // neighbour title so any overlap is visible
    let before = frame(&mut app, 180);
    for column in [8_u16, 9_u16, 16_u16, 17_u16] {
        assert!(
            !click(&mut app, column),
            "the gap cell {column} selects nothing off Tools"
        );
    }
    assert_eq!(frame(&mut app, 180), before, "first gap ring unchanged");
    assert!(click(&mut app, 5), "Agents still clicks from Tools");
    let _ = frame(&mut app, 180);
    let before = frame(&mut app, 180);
    for column in [23_u16, 24_u16] {
        assert!(
            !click(&mut app, column),
            "the cell right of Tools selects nothing off Agents"
        );
    }
    assert_eq!(
        frame(&mut app, 180),
        before,
        "gap clicks change nothing on the shown inner"
    );
}

#[test]
fn the_shown_inner_is_drawn_selected_like_the_selected_top_tab() {
    let (mut app, _rx, _generation) = open_app();
    key(&mut app, "7");
    let render_now = |app: &mut App| render(app, 180, 90);
    let check = |text: &str, buffer: &Buffer, expected: &str| {
        let top = text.lines().nth(1).expect("the tab row").to_owned();
        let anchor = style_at(buffer, start_of(&top, "Statistics"), 1);
        let inner = text.lines().nth(2).expect("the inner row").to_owned();
        for title in ["Agents", "Skills", "Tools"] {
            let start = start_of(&inner, title);
            assert_eq!(
                style_at(buffer, start, 2) == anchor,
                title == expected,
                "{title} carries the selected style where {expected} is shown"
            );
        }
    };
    let (text, buffer) = render_now(&mut app);
    check(&text, &buffer, "Agents");
    arrow(&mut app, Key::Left);
    let (text, buffer) = render_now(&mut app);
    check(&text, &buffer, "Tools");
    arrow(&mut app, Key::Left);
    let (text, buffer) = render_now(&mut app);
    check(&text, &buffer, "Skills");
    key(&mut app, "n");
    let (text, buffer) = render_now(&mut app);
    check(&text, &buffer, "Tools");
    key(&mut app, "n");
    let (text, buffer) = render_now(&mut app);
    check(&text, &buffer, "Tools");
    key(&mut app, "n");
    let (text, buffer) = render_now(&mut app);
    check(&text, &buffer, "Agents");
}

#[test]
fn the_overflow_body_stops_on_the_last_line_that_fits() {
    let (mut app, _rx, generation) = open_app();
    key(&mut app, "7");
    land_stats(&mut app, generation);
    let tall = frame(&mut app, 180);
    // body rows sit at 4 to height - 2 (the footer row past them is not
    // one of the body lines)
    let lines: Vec<&str> = tall.lines().skip(4).take(85).collect();
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(0, |at| at + 1);
    let body: Vec<String> = lines[..end].iter().map(ToString::to_string).collect();
    assert!(body.len() > 5, "the spawn body overflows at height 10");
    assert!(!body[5].trim().is_empty(), "the sixth body line is drawn");
    // five body lines fit with the inner row, the bottom one just above
    // the bottom border at row 8
    let (short, _) = render(&mut app, 180, 10);
    assert_eq!(
        short.lines().nth(8),
        Some(body[4].as_str()),
        "the last line that fits is drawn"
    );
    assert!(!short.contains(body[5].as_str()), "the one after it is not");
    // the scroll stop agrees: far enough down and the last body line
    // sits on the row above the border, and stays
    let last = body.len() - 1;
    for _ in 0..=last {
        arrow(&mut app, Key::Down);
    }
    let (short, _) = render(&mut app, 180, 10);
    assert_eq!(
        short.lines().nth(8),
        Some(body[last].as_str()),
        "the scroll reaches the last body line"
    );
    for _ in 0..5 {
        arrow(&mut app, Key::Down);
    }
    let (short, _) = render(&mut app, 180, 10);
    assert_eq!(
        short.lines().nth(8),
        Some(body[last].as_str()),
        "the scroll stop holds the last line"
    );
}

#[test]
fn arrows_off_statistics_leave_the_inner_alone() {
    for tab in ["1", "2", "3", "4", "5", "6"] {
        let (mut app, _rx, generation) = open_app();
        key(&mut app, tab);
        arrow(&mut app, Key::Left);
        key(&mut app, "7");
        land_stats(&mut app, generation);
        assert!(
            frame(&mut app, 180).contains("Recorded worker spawns: 12"),
            "Left on tab {tab} leaves the Agents inner alone"
        );
    }
    for tab in ["1", "2", "3", "4", "5", "6"] {
        let (mut app, _rx, generation) = open_app();
        key(&mut app, tab);
        arrow(&mut app, Key::Right);
        key(&mut app, "7");
        land_stats(&mut app, generation);
        assert!(
            frame(&mut app, 180).contains("Recorded worker spawns: 12"),
            "Right on tab {tab} leaves the Agents inner alone"
        );
    }
    let (mut app, _rx, generation) = open_app();
    key(&mut app, "7");
    arrow(&mut app, Key::Right);
    let _ = frame(&mut app, 180);
    land(&mut app, generation, answer_view(Page::Skills));
    assert!(frame(&mut app, 180).contains("fixture-skill"));
    key(&mut app, "2");
    arrow(&mut app, Key::Left);
    arrow(&mut app, Key::Right);
    key(&mut app, "7");
    let _ = frame(&mut app, 180);
    land(&mut app, generation, answer_view(Page::Skills));
    assert!(
        frame(&mut app, 180).contains("fixture-skill"),
        "the Skills choice survives arrows on Config"
    );
}

#[test]
fn the_shown_inner_highlights_the_inner_row_after_off_tab_arrows() {
    let (mut app, _rx, _generation) = open_app();
    key(&mut app, "4");
    arrow(&mut app, Key::Left);
    arrow(&mut app, Key::Right);
    key(&mut app, "7");
    let (text, buffer) = render(&mut app, 180, 90);
    let top = text.lines().nth(1).expect("the tab row").to_owned();
    let anchor = style_at(&buffer, start_of(&top, "Statistics"), 1);
    let inner = text.lines().nth(2).expect("the inner row").to_owned();
    assert!(
        style_at(&buffer, start_of(&inner, "Agents"), 2) == anchor,
        "Agents shows the selected style after arrows on Instructions"
    );
}
