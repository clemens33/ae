//! #288 UI acceptance through model keys, `App::answer` and actual cell rendering.
#![allow(clippy::expect_used, reason = "frozen acceptance assertions")]
use super::fleet::Fleet;
use super::loader::{Answer, Request};
use super::model::{Act, Key, Model};
use super::settings::StatsPage;
use super::{App, take_keys};
use crate::console::input;
use crate::stats::Cell;
use crate::tool_stats::*;
use ratatui_core::{buffer::Buffer, layout::Rect};
use std::sync::mpsc;
use std::time::Instant;

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

fn key(app: &mut App, text: &str) {
    let _ = take_keys(
        app,
        vec![(input::Key::Text(text.as_bytes().to_vec()), Instant::now())],
    );
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

fn opened() -> (App, mpsc::Receiver<Request>, u64) {
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(None, None, None, Some(tx));
    app.fleeted = true;
    app.open_settings();
    let generation = app.model.settings().expect("open overlay").generation;
    key(&mut app, "7");
    (app, rx, generation)
}

fn land(app: &mut App, generation: u64, view: View) {
    app.answer(Answer::Tools {
        generation,
        view: Box::new(view),
    });
}

fn coverage_notes(text: &str) {
    for note in [
        "Claude effort: not observed",
        "unknown unless proven for the whole conversation",
        "No observed use in covered sources != unused.",
        "completed only",
        "no children",
        "newest 4 earlier conversations",
        "client labels follow the recorded session meta",
        "a hand edit of that meta relabels its conversations",
        "a client label added on resume applies to the whole conversation",
        "the account store is proven unchanged",
    ] {
        assert!(text.contains(note), "missing coverage note {note}: {text}");
    }
}

fn tool_requests(rx: &mpsc::Receiver<Request>) -> Vec<(u64, bool, Ask)> {
    rx.try_iter()
        .filter_map(|request| match request {
            Request::Tools {
                generation,
                shown,
                ask,
            } => Some((generation, shown, ask)),
            _ => None,
        })
        .collect()
}

#[test]
fn a8_page_and_filter_keys_are_statistics_local_and_keep_spawns_default() {
    let fleet = Fleet::default();
    let mut model = Model::new(&fleet);
    model.open_settings(7);
    model.key(Key::Digit(7), &fleet, false);
    for (key, act) in [
        (Key::SeatNext, Act::StatsPage(true)),
        (Key::SeatPrev, Act::StatsPage(false)),
        (Key::TurnOlder, Act::StatsFilter(false)),
        (Key::TurnNewer, Act::StatsFilter(true)),
    ] {
        assert_eq!(model.key(key, &fleet, false), act);
    }
    model.key(Key::Digit(1), &fleet, false);
    for key in [Key::SeatNext, Key::SeatPrev, Key::TurnOlder, Key::TurnNewer] {
        assert!(!matches!(
            model.key(key, &fleet, false),
            Act::StatsPage(_) | Act::StatsFilter(_)
        ));
    }
    let (mut app, rx, generation) = opened();
    assert_eq!(app.settings_bodies.stats.page, StatsPage::Spawns);
    let _ = frame(&mut app, 180);
    assert!(
        rx.try_iter()
            .all(|request| !matches!(request, Request::Tools { shown: true, .. }))
    );
    for (letter, expected) in [
        ("n", StatsPage::Skills),
        ("n", StatsPage::Mcp),
        ("n", StatsPage::Tools),
        ("n", StatsPage::Spawns),
        ("p", StatsPage::Tools),
    ] {
        key(&mut app, letter);
        assert_eq!(app.settings_bodies.stats.page, expected);
    }
    let tools = answer_view(Page::Tools);
    land(&mut app, generation, tools);
    let _ = frame(&mut app, 180);
    let _ = rx.try_iter().collect::<Vec<_>>();
    key(&mut app, "]");
    let _ = frame(&mut app, 180);
    assert_eq!(
        app.settings_bodies.stats.filter,
        Filter::Client(Cell::Value("work".into()))
    );
    assert!(rx.try_iter().any(|request| matches!(request, Request::Tools { generation: g, shown: true, ask: Ask { page: Page::Tools, filter: Filter::Client(Cell::Value(ref label)), .. } } if g == generation && label == "work")));
    key(&mut app, "[");
    assert_eq!(app.settings_bodies.stats.filter, Filter::All);
    key(&mut app, "r");
    assert!(
        app.settings_bodies.stats.store.reset.is_none(),
        "r cannot reset the store from a Tools page"
    );
}

#[test]
fn a8_stale_answers_cannot_enter_closed_or_reopened_overlay() {
    let (mut app, _rx, first) = opened();
    key(&mut app, "n");
    land(&mut app, first, answer_view(Page::Skills));
    assert!(app.settings_bodies.stats.tools.is_some());
    app.close_settings();
    let held = app.settings_bodies.stats.clone();
    land(&mut app, first, answer_view(Page::Tools));
    assert_eq!(
        app.settings_bodies.stats, held,
        "closed generation rejects incoming answer"
    );
    app.open_settings();
    let second = app.model.settings().expect("reopened overlay").generation;
    assert_ne!(first, second);
    assert!(app.settings_bodies.stats.tools.is_none());
    land(&mut app, first, answer_view(Page::Tools));
    assert!(
        app.settings_bodies.stats.tools.is_none(),
        "old answer cannot replace a new open"
    );
    key(&mut app, "7");
    key(&mut app, "n");
    land(&mut app, second, answer_view(Page::Skills));
    assert!(frame(&mut app, 180).contains("fixture-skill"));
}

#[test]
fn a8_page_scope_filter_and_hide_requests_carry_the_actual_selection() {
    let (mut app, rx, generation) = opened();
    let _ = frame(&mut app, 220);
    let _ = rx.try_iter().collect::<Vec<_>>();
    for (letter, page) in [
        ("n", Page::Skills),
        ("n", Page::Mcp),
        ("n", Page::Tools),
        ("p", Page::Mcp),
    ] {
        key(&mut app, letter);
        let _ = frame(&mut app, 220);
        let expected = Ask {
            page,
            scope: Scope::All,
            filter: Filter::All,
        };
        assert!(
            tool_requests(&rx).contains(&(generation, true, expected)),
            "the selected page must arm its actual request"
        );
        land(&mut app, generation, answer_view(page));
    }
    key(&mut app, "m");
    let _ = frame(&mut app, 220);
    assert!(tool_requests(&rx).contains(&(
        generation,
        true,
        Ask {
            page: Page::Mcp,
            scope: Scope::This(None),
            filter: Filter::All
        }
    )));
    key(&mut app, "m");
    let _ = frame(&mut app, 220);
    assert!(tool_requests(&rx).contains(&(
        generation,
        true,
        Ask {
            page: Page::Mcp,
            scope: Scope::All,
            filter: Filter::All
        }
    )));
    land(&mut app, generation, answer_view(Page::Mcp));
    key(&mut app, "]");
    let text = frame(&mut app, 220);
    let filter = Filter::Client(Cell::Value("work".into()));
    assert!(
        text.contains("client=work"),
        "active filter is named in the title: {text}"
    );
    assert!(tool_requests(&rx).contains(&(
        generation,
        true,
        Ask {
            page: Page::Mcp,
            scope: Scope::All,
            filter
        }
    )));
    key(&mut app, "p");
    let _ = frame(&mut app, 220);
    let _ = rx.try_iter().collect::<Vec<_>>();
    key(&mut app, "p");
    let _ = frame(&mut app, 220);
    assert!(
        tool_requests(&rx)
            .iter()
            .any(|(g, shown, _)| *g == generation && !shown)
    );
    key(&mut app, "n");
    let _ = frame(&mut app, 220);
    let _ = rx.try_iter().collect::<Vec<_>>();
    app.close_settings();
    // Settings close is itself the loader cancellation door. Either a
    // separate Tools hide or that existing close message disarms the watch.
    assert!(rx.try_iter().any(|request| matches!(request,
        Request::Tools { generation: g, shown: false, .. } |
        Request::Settings { generation: g, open: false } if g == generation)));
}

#[test]
fn a5_every_tool_page_renders_coverage_notes_and_distinct_columns() {
    let (mut app, _rx, generation) = opened();
    for (page, heading, columns) in [
        (
            Page::Skills,
            "Skills",
            vec!["Loaded", "Not loaded", "Unknown", "Unresolved"],
        ),
        (
            Page::Mcp,
            "MCP",
            vec!["Returned", "Success", "Error", "Unresolved"],
        ),
        (
            Page::Tools,
            "Tools",
            vec!["Returned", "Error", "Unresolved"],
        ),
    ] {
        key(&mut app, "n");
        assert!(frame(&mut app, 220).contains("loading"));
        land(&mut app, generation, answer_view(page));
        let text = frame(&mut app, 220);
        for word in std::iter::once(heading).chain(columns) {
            assert!(text.contains(word), "missing {word}: {text}");
        }
        for note in [
            "Covers 1 conversations",
            "Claude effort: not observed",
            "unknown unless proven for the whole conversation",
            "No observed use in covered sources != unused.",
            "completed only",
            "no children",
            "newest 4 earlier conversations",
            "client labels follow the recorded session meta",
            "a hand edit of that meta relabels its conversations",
            "a client label added on resume applies to the whole conversation",
            "the account store is proven unchanged",
        ] {
            assert!(text.contains(note), "missing coverage note {note}: {text}");
        }
        coverage_notes(&text);
        assert!(
            text.contains("2026-11-01") && text.contains("2026-11-02") && text.contains(" .. "),
            "both covered range bounds are visible: {text}"
        );
    }
}

#[test]
fn a8_m_scope_resolves_selected_id_before_home_and_uses_home_without_selection() {
    use super::fleet::{Counts, Line2, Row};
    use crate::theme::Mark;
    use crate::tool_stats_fixture as f;
    for selected in [false, true] {
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(Some("home".into()), None, None, Some(tx));
        app.fleeted = true;
        if selected {
            app.fleet.rows = ["home", "selected"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| Row {
                    name: name.into(),
                    index: index + 1,
                    mark: Mark::Working,
                    needy: false,
                    unanswered: None,
                    counts: Counts::Unknown,
                    line2: Line2::NoGoal,
                    home: name == "home",
                })
                .collect();
            app.model = Model::new(&app.fleet);
            assert!(app.model.select_name(&app.fleet, "selected", 0).is_some());
            assert_eq!(app.model.selected(), Some("selected"));
            app.ids.insert("selected".into(), f::S.into());
            app.ids.insert("home".into(), f::T.into());
        } else {
            app.model = Model::new(&Fleet::default());
            assert_eq!(app.model.selected(), None);
            app.ids.insert("home".into(), f::S.into());
        }
        app.open_settings();
        let generation = app.model.settings().expect("open overlay").generation;
        key(&mut app, "7");
        key(&mut app, "n");
        let _ = frame(&mut app, 220);
        let _ = rx.try_iter().collect::<Vec<_>>();
        key(&mut app, "m");
        let _ = frame(&mut app, 220);
        assert!(
            tool_requests(&rx).contains(&(
                generation,
                true,
                Ask {
                    page: Page::Skills,
                    scope: Scope::This(Some(f::S.into())),
                    filter: Filter::All
                }
            )),
            "selected={selected}: m must carry the recorded session id"
        );
    }
}

#[test]
fn a5_rendered_states_distinguish_progress_gaps_unsupported_and_no_session_id() {
    let (mut app, _rx, generation) = opened();
    key(&mut app, "p");
    let mut progress = answer_view(Page::Tools);
    progress.read = 1;
    progress.listed = 3;
    progress.recounting = 1;
    progress.gaps.push(GapLine {
        kind: GapKind::Malformed,
        conversation: Some(crate::tool_stats_fixture::C.into()),
        count: 2,
    });
    land(&mut app, generation, progress);
    let text = frame(&mut app, 200);
    for fragment in [
        "Partial",
        "1 of 3 conversations read",
        "recounting 1 changed",
        "Read",
    ] {
        assert!(text.contains(fragment), "{fragment}: {text}");
    }
    assert!(
        text.lines().any(|line| line
            .split_whitespace()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|pair| pair == ["malformed", "2"])),
        "the malformed count is the exact token 2: {text}"
    );
    let mut none = answer_view(Page::Tools);
    none.state = State::NoSupported;
    none.rows.clear();
    none.detail.clear();
    none.gaps = vec![GapLine {
        kind: GapKind::Unsupported("grok".into()),
        conversation: None,
        count: 1,
    }];
    land(&mut app, generation, none);
    let text = frame(&mut app, 200);
    assert!(
        text.contains("No ae-associated Claude or Codex conversations"),
        "{text}"
    );
    assert!(
        text.contains("grok"),
        "unsupported coverage remains visible: {text}"
    );
    coverage_notes(&text);
    let mut no_id = answer_view(Page::Tools);
    no_id.state = State::NoSessionId;
    no_id.ask.scope = Scope::This(None);
    key(&mut app, "m");
    land(&mut app, generation, no_id);
    let text = frame(&mut app, 200);
    assert!(text.contains("This session has no session id: nothing to show"));
    coverage_notes(&text);
}

#[test]
fn a5_detail_joint_row_and_nonzero_orphans_are_actually_drawn() {
    let (mut app, _rx, generation) = opened();
    key(&mut app, "p");
    let mut view = answer_view(Page::Tools);
    view.orphans = 3;
    land(&mut app, generation, view);
    let text = frame(&mut app, 220);
    assert!(text.contains("Orphan results: 3"), "{text}");
    let detail = text
        .split_once("Detail")
        .expect("positive Detail heading")
        .1
        .to_ascii_lowercase();
    assert!(
        detail
            .lines()
            .any(|line| ["read", "claude", "work", "model-a", "unknown", "1"]
                .iter()
                .all(|value| line.contains(value))),
        "one joint row must carry the item and all four dimensions: {detail}"
    );
}

#[test]
fn a7_real_gap_lines_and_draw_drop_the_private_path_and_malformed_body() {
    use crate::tool_stats_fixture::{self as f, Fixture};
    use crate::usage::calls_read;
    let root = Fixture::new();
    root.live(
        "alpha",
        &root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a"),
        "",
    );
    let bytes = f::call("safe", "safe", "Read", "model-a", "{}")
        + &format!("{{\"type\":\"{}\",BAD}}\n", f::PRIVATE);
    root.claude("store-a", f::C, bytes.as_bytes());
    let reader = calls_read::scan(root.0.clone(), Some(root.home()), Caps::SLICE);
    let view = reader.view(&Ask {
        page: Page::Tools,
        scope: Scope::All,
        filter: Filter::All,
    });
    assert!(
        !view.gaps.is_empty(),
        "privacy checks must exercise actual gap lines"
    );
    assert!(
        view.gaps
            .iter()
            .any(|gap| gap.kind == GapKind::Malformed && gap.count == 1)
    );
    for gap in &view.gaps {
        let line = gap.line();
        assert!(
            !line.contains(f::PRIVATE) && !line.contains(&root.0.display().to_string()),
            "{line}"
        );
    }
    let (mut app, _rx, generation) = opened();
    key(&mut app, "p");
    let _ = calls_read::take_units();
    land(&mut app, generation, view);
    let text = frame(&mut app, 220);
    assert!(
        text.contains("malformed 1"),
        "the real diagnostic is visible: {text}"
    );
    assert!(!text.contains(f::PRIVATE) && !text.contains(&root.0.display().to_string()));
    assert!(
        calls_read::take_units().is_empty(),
        "draw and answer admission perform no usage IO"
    );
}

#[test]
fn a5_more_rows_and_narrow_identity_are_visible_without_detail_below_forty() {
    let (mut app, _rx, generation) = opened();
    key(&mut app, "p");
    let mut view = answer_view(Page::Tools);
    view.items = 257;
    view.more = Some(More {
        items: 1,
        attempts: 2,
    });
    view.totals = Cols::Tool(ToolCols {
        attempts: 258,
        unresolved: 258,
        ..ToolCols::default()
    });
    view.rows[0].item = Item::Tool(Ident::Name(
        "a-very-long-but-valid-tool-identity-that-needs-truncation".into(),
    ));
    land(&mut app, generation, view);
    let wide = frame(&mut app, 200);
    assert!(
        wide.contains("+1 more items, 2 attempts not shown"),
        "{wide}"
    );
    // The app floor is 40 columns. Its body has fewer than 40 after gutters.
    let narrow = frame(&mut app, 40);
    assert!(!narrow.contains("Detail"), "{narrow}");
    assert!(
        narrow.contains('…'),
        "long identity is visibly truncated: {narrow}"
    );
}
