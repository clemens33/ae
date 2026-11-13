//! #288 mutants slot 2 survivors of the Statistics keys (driver tests, not
//! frozen spec): `m` on a tool page waits for the other scope's own answer,
//! and `[` steps to the previous filter, wrapping.
#![allow(clippy::expect_used, reason = "test assertions")]

use super::loader::Answer;
use super::settings::StatsPage;
use super::{App, take_keys};
use crate::console::input;
use crate::stats::Cell;
use crate::tool_stats::{Ask, Cols, Filter, Ident, Item, Page, Row, Scope, State, ToolCols, View};
use std::time::Instant;

fn key(app: &mut App, text: &str) {
    let _ = take_keys(
        app,
        vec![(input::Key::Text(text.as_bytes().to_vec()), Instant::now())],
    );
}

fn filters() -> Vec<Filter> {
    vec![
        Filter::All,
        Filter::Client(Cell::Value("work".into())),
        Filter::Client(Cell::Value("play".into())),
    ]
}

fn tools_view() -> View {
    let cols = Cols::Tool(ToolCols {
        attempts: 1,
        ..ToolCols::default()
    });
    View {
        ask: Ask {
            page: Page::Tools,
            scope: Scope::All,
            filter: Filter::All,
        },
        state: State::Numbers,
        rows: vec![Row {
            item: Item::Tool(Ident::Name("Read".into())),
            cols,
        }],
        more: None,
        items: 1,
        totals: cols,
        harnesses: Vec::new(),
        orphans: 0,
        detail: Vec::new(),
        detail_more: None,
        range: None,
        read: 1,
        listed: 1,
        recounting: 0,
        gaps: Vec::new(),
        filters: filters(),
    }
}

/// The Statistics tab open on its Tools page with an answer landed.
fn on_tools() -> App {
    let (tx, _rx) = std::sync::mpsc::channel();
    let mut app = App::new(None, None, None, Some(tx));
    app.fleeted = true;
    app.open_settings();
    let generation = app.model.settings().expect("open overlay").generation;
    key(&mut app, "7");
    key(&mut app, "p");
    assert_eq!(
        app.settings_bodies.stats.page,
        StatsPage::Tools,
        "control: the Tools page"
    );
    app.answer(Answer::Tools {
        generation,
        view: Box::new(tools_view()),
    });
    assert!(
        app.settings_bodies.stats.tools.is_some(),
        "control: the answer landed"
    );
    app
}

/// app.rs:1377: `m` on a tool page drops the numbers of the other scope
/// until its own answer lands.
#[test]
fn m_on_a_tool_page_waits_for_the_other_scope_answer() {
    let mut app = on_tools();
    let before = app.settings_bodies.stats.scope;
    key(&mut app, "m");
    assert_ne!(
        app.settings_bodies.stats.scope, before,
        "control: the scope flipped"
    );
    assert!(
        app.settings_bodies.stats.tools.is_none(),
        "the other scope's numbers are not drawn as this one's"
    );
}

/// app.rs:1420: `[` steps to the previous filter and `]` to the next, each
/// wrapping, over three filters (two would not tell them apart).
#[test]
fn bracket_keys_step_back_and_forth_over_the_offered_filters() {
    let mut app = on_tools();
    let offered = filters();
    for (letter, expected) in [
        ("[", &offered[2]),
        ("[", &offered[1]),
        ("]", &offered[2]),
        ("]", &offered[0]),
    ] {
        key(&mut app, letter);
        assert_eq!(
            &app.settings_bodies.stats.filter, expected,
            "after {letter}"
        );
    }
}
