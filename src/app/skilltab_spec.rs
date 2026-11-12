//! Frozen acceptance for the Statistics tab's three inner tabs, authored
//! by navigator `skilltab-nav` for the accepted inner-tabs design: no
//! Settings tab 8, Statistics (tab 7) carries one row of inner tabs
//! `Agents`, `Skills`, `Tools`, with `Left`/`Right` stepping the row and
//! `n`/`p` paging all four `StatsPages` as on main. Install as
//! `src/app/skilltab_spec.rs` and wire from `src/app.rs` under
//! `cfg(test)`. RED on `612a485e` is by assertion only: nothing named
//! here is new on that commit (no model key variant, no inner type, no
//! row hit type), so a missing symbol never decides the outcome.
//!
//! `r5` supersedes `r4`, which froze the retired tab-8 split. All
//! helpers are carried over from `r4` unchanged.
#![allow(clippy::expect_used, reason = "frozen acceptance assertions")]

use std::sync::mpsc;
use std::time::Instant;

use ratatui_core::{buffer::Buffer, layout::Rect};

use super::fleet::Fleet;
use super::loader::{Answer, Request};
use super::model::{Act, Key, Model, SettingsTab};
use super::settings::StatsScope;
use super::{App, take_keys};
use crate::console::input;
use crate::stats::{self, Cell, Source};
use crate::stats_fixture as f;
use crate::tool_stats::*;

fn key(app: &mut App, text: &str) {
    let _ = take_keys(
        app,
        vec![(input::Key::Text(text.as_bytes().to_vec()), Instant::now())],
    );
}

fn arrow(app: &mut App, key: input::Key) {
    let _ = take_keys(app, vec![(key, Instant::now())]);
}

fn frame(app: &mut App, width: u16) -> String {
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, 90));
    app.frame(&mut buffer);
    (0..90)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
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

#[test]
fn top_cycle_wraps_from_statistics_to_quota_and_digit_8_is_swallowed() {
    let fleet = Fleet::default();
    let mut model = Model::new(&fleet);
    model.open_settings(7);
    assert_eq!(model.key(Key::Digit(8), &fleet, false), Act::None);
    assert!(matches!(
        model.settings().expect("open").tab,
        SettingsTab::Quota
    ));
    model.key(Key::Digit(6), &fleet, false);
    assert!(matches!(
        model.settings().expect("open").tab,
        SettingsTab::Accounts
    ));
    assert_eq!(model.key(Key::Tab, &fleet, false), Act::Redraw);
    assert!(matches!(
        model.settings().expect("open").tab,
        SettingsTab::Statistics
    ));
    assert_eq!(model.key(Key::Tab, &fleet, false), Act::Redraw);
    assert!(matches!(
        model.settings().expect("open").tab,
        SettingsTab::Quota
    ));
}

#[test]
fn tab7_opens_on_agents_and_draws_the_spawn_view() {
    let (mut app, _rx, generation) = open_app();
    key(&mut app, "7");
    land_stats(&mut app, generation);
    let shown = frame(&mut app, 180);
    assert!(
        shown.contains("Recorded worker spawns: 12"),
        "Agents shows the recorded spawns: {shown}"
    );
    assert!(
        !shown.contains("fixture-skill"),
        "Agents shows no tool table: {shown}"
    );
}

#[test]
fn left_and_right_step_the_inner_row_and_wrap() {
    let (mut app, _rx, generation) = open_app();
    key(&mut app, "7");
    land_stats(&mut app, generation);
    arrow(&mut app, input::Key::Right);
    land(&mut app, generation, answer_view(Page::Skills));
    assert!(
        frame(&mut app, 180).contains("fixture-skill"),
        "one step right lands on Skills"
    );
    arrow(&mut app, input::Key::Right);
    land(&mut app, generation, answer_view(Page::Mcp));
    assert!(
        frame(&mut app, 180).contains("fixture-server"),
        "entering Tools lands on MCP servers"
    );
    arrow(&mut app, input::Key::Right);
    land_stats(&mut app, generation);
    assert!(
        frame(&mut app, 180).contains("Recorded worker spawns: 12"),
        "the row wraps back to Agents"
    );
    arrow(&mut app, input::Key::Left);
    land(&mut app, generation, answer_view(Page::Mcp));
    assert!(
        frame(&mut app, 180).contains("fixture-server"),
        "left from Agents wraps to Tools, on MCP servers"
    );
    arrow(&mut app, input::Key::Left);
    land(&mut app, generation, answer_view(Page::Skills));
    assert!(
        frame(&mut app, 180).contains("fixture-skill"),
        "left from Tools lands on Skills"
    );
    arrow(&mut app, input::Key::Left);
    land_stats(&mut app, generation);
    assert!(
        frame(&mut app, 180).contains("Recorded worker spawns: 12"),
        "left from Skills reaches Agents"
    );
}

#[test]
fn arrows_off_tab7_do_nothing() {
    let (mut app, rx, _generation) = open_app();
    let _ = frame(&mut app, 180);
    let _ = rx.try_iter().collect::<Vec<_>>();
    arrow(&mut app, input::Key::Left);
    arrow(&mut app, input::Key::Right);
    let _ = frame(&mut app, 180);
    assert!(
        rx.try_iter().all(|request| !matches!(
            request,
            Request::Tools { shown: true, .. } | Request::Stats { shown: true, .. }
        )),
        "arrows off tab 7 arm no watch"
    );
}

#[test]
fn n_and_p_page_across_skills_into_tools() {
    let (mut app, rx, generation) = open_app();
    key(&mut app, "7");
    arrow(&mut app, input::Key::Right);
    let _ = frame(&mut app, 180);
    land(&mut app, generation, answer_view(Page::Skills));
    assert!(
        frame(&mut app, 180).contains("fixture-skill"),
        "the Skills inner shows loaded skills"
    );
    key(&mut app, "n");
    let _ = frame(&mut app, 180);
    land(&mut app, generation, answer_view(Page::Mcp));
    assert!(
        frame(&mut app, 180).contains("fixture-server"),
        "n crosses Skills to MCP servers within Tools"
    );
    key(&mut app, "n");
    let _ = frame(&mut app, 180);
    let asked = rx.try_iter().collect::<Vec<_>>();
    land(&mut app, generation, answer_view(Page::Tools));
    let tools = frame(&mut app, 180);
    assert!(
        tools.contains("Read") && !tools.contains("fixture-server"),
        "n inside Tools reaches the Tools page: {tools}"
    );
    assert!(
        tools_at(&asked, generation, Page::Tools),
        "the Tools page watch was armed while crossing"
    );
    key(&mut app, "n");
    land_stats(&mut app, generation);
    assert!(
        frame(&mut app, 180).contains("Recorded worker spawns: 12"),
        "n from the Tools page wraps to Agents"
    );
}

#[test]
fn scope_flip_is_shared_across_inners_and_drops_held_numbers() {
    let (mut app, _rx, generation) = open_app();
    key(&mut app, "7");
    arrow(&mut app, input::Key::Right);
    let _ = frame(&mut app, 180);
    land(&mut app, generation, answer_view(Page::Skills));
    assert!(frame(&mut app, 180).contains("fixture-skill"));
    key(&mut app, "m");
    assert_eq!(
        app.settings_bodies.stats.scope,
        StatsScope::ThisSession,
        "m flips the shared scope on Skills"
    );
    assert!(
        app.settings_bodies.stats.tools.is_none(),
        "the flip drops the held tool numbers"
    );
    arrow(&mut app, input::Key::Left);
    assert_eq!(
        app.settings_bodies.stats.scope,
        StatsScope::ThisSession,
        "Agents shows the same scope"
    );
    land_stats(&mut app, generation);
    assert!(
        frame(&mut app, 180).contains("Configured model: 5 / 10"),
        "Agents draws this session under the shared scope"
    );
    key(&mut app, "r");
    assert!(
        app.settings_bodies.stats.store.reset.is_some(),
        "r on Agents requests the rebuild"
    );
}

#[test]
fn r_on_skills_and_tools_inners_requests_nothing() {
    let (mut app, _rx, _generation) = open_app();
    key(&mut app, "7");
    arrow(&mut app, input::Key::Right);
    let _ = frame(&mut app, 180);
    key(&mut app, "r");
    assert!(
        app.settings_bodies.stats.store.reset.is_none(),
        "r on Skills requests nothing"
    );
    arrow(&mut app, input::Key::Right);
    let _ = frame(&mut app, 180);
    key(&mut app, "r");
    assert!(
        app.settings_bodies.stats.store.reset.is_none(),
        "r on Tools requests nothing"
    );
}

#[test]
fn tool_watch_arms_only_off_the_agents_inner() {
    let (mut app, rx, generation) = open_app();
    key(&mut app, "7");
    let _ = frame(&mut app, 180);
    let asked = rx.try_iter().collect::<Vec<_>>();
    assert!(
        asked.iter().any(|request| matches!(
            request,
            Request::Stats {
                generation: seen,
                shown: true,
            } if *seen == generation
        )),
        "Agents arms the spawn watch"
    );
    assert!(
        asked
            .iter()
            .all(|request| !matches!(request, Request::Tools { shown: true, .. })),
        "Agents arms no tool watch"
    );
    arrow(&mut app, input::Key::Right);
    let _ = frame(&mut app, 180);
    assert!(
        tools_at(&rx.try_iter().collect::<Vec<_>>(), generation, Page::Skills),
        "Skills arms the skills watch"
    );
    arrow(&mut app, input::Key::Right);
    let _ = frame(&mut app, 180);
    assert!(
        tools_at(&rx.try_iter().collect::<Vec<_>>(), generation, Page::Mcp),
        "Tools arms the MCP servers watch"
    );
}

#[test]
fn a_new_open_resets_the_inner_to_agents() {
    let (mut app, rx, generation) = open_app();
    key(&mut app, "7");
    arrow(&mut app, input::Key::Right);
    let _ = frame(&mut app, 180);
    land(&mut app, generation, answer_view(Page::Skills));
    assert!(frame(&mut app, 180).contains("fixture-skill"));
    arrow(&mut app, input::Key::Right);
    let _ = frame(&mut app, 180);
    land(&mut app, generation, answer_view(Page::Mcp));
    assert!(frame(&mut app, 180).contains("fixture-server"));
    app.close_settings();
    app.open_settings();
    let generation = app.model.settings().expect("reopened").generation;
    key(&mut app, "7");
    land_stats(&mut app, generation);
    let shown = frame(&mut app, 180);
    assert!(
        shown.contains("Recorded worker spawns: 12") && !shown.contains("fixture-server"),
        "a new open starts on Agents: {shown}"
    );
    let _ = rx;
}
