//! Acceptance for #237 items 5-6, authored before implementation.
//! Oracle: brief + plan. One in-memory stopped group, visible-only selection,
//! live process totals, one-line width ladder. Fixtures never launch tmux.

use std::collections::BTreeMap;
use std::time::Instant;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::fleet::{self, Counts, Facts, Fleet, Line2, Row};
use super::loader::{Answer, FleetRead};
use super::model::{Drag, Edge};
use super::{App, browse, draw, menu};
use crate::console::input::{Key, Mouse, MouseKind};
use crate::digest::{SessionEntry, Status};
use crate::listing::World;
use crate::theme::{FleetOrder, Look, Mark};
use crate::time::Timestamp;
use crate::tmux::PickerAgent;

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

fn read(rows: Vec<Row>, home: Option<&str>) -> FleetRead {
    let now = Timestamp::from_epoch(1_792_579_200);
    let entries = rows
        .iter()
        .map(|row| {
            SessionEntry::new(
                &row.name,
                if row.counts == Counts::Stopped {
                    Status::Stopped
                } else {
                    Status::Running
                },
            )
        })
        .collect();
    let ids: BTreeMap<String, String> = rows
        .iter()
        .map(|row| (row.name.clone(), ID.to_owned()))
        .collect();
    FleetRead {
        dirs: BTreeMap::new(),
        sole: ids.clone(),
        ids,
        world: World::new(now, entries),
        facts: BTreeMap::new(),
        needs: BTreeMap::new(),
        fleet: Fleet {
            rows,
            home: home.map(str::to_owned),
        },
        pairs: BTreeMap::new(),
        scanned: true,
    }
}

fn app(rows: Vec<Row>, home: Option<&str>) -> App {
    let mut app = App::new(home.map(str::to_owned), None, None, None);
    app.answer(Answer::Fleet(read(rows, home)));
    for row in &app.fleet.rows {
        app.looks.insert(
            row.name.clone(),
            (Some(Look::read("on", "darcula", "on", "on")), None),
        );
    }
    app
}

fn mixed(home: Option<&str>) -> App {
    app(
        vec![
            row("run-a", 1, false),
            row("run-b", 2, false),
            row("old-a", 3, true),
            row("old-b", 4, true),
        ],
        home,
    )
}

fn frame(app: &mut App, sidebar: u16, icons: bool) -> Buffer {
    app.model.grab(Drag {
        edge: Edge::Sidebar,
        from: 44,
        at: 44,
    });
    app.model.drag_to(sidebar);
    app.model.let_go();
    for (look, _) in app.looks.values_mut() {
        *look = Some(Look::read(
            if icons { "on" } else { "off" },
            "darcula",
            "on",
            "on",
        ));
    }
    let mut buf = Buffer::empty(Rect::new(0, 0, 180, 45));
    app.frame(&mut buf);
    buf
}

fn left(buf: &Buffer, sidebar: u16, y: u16) -> String {
    (0..sidebar)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
        .trim()
        .to_owned()
}

fn keys(app: &mut App, bytes: &[u8]) {
    assert!(
        browse(app, &Key::Text(bytes.to_vec()), Instant::now()).is_some(),
        "navigation must not quit"
    );
}

fn session_click(app: &App, name: &str) -> Mouse {
    (0..45).flat_map(|row| (0..180).map(move |column| Mouse { kind: MouseKind::Click, column, row })).find(|mouse| matches!(app.layout.hit(*mouse), Some(draw::Hit::Session(target)) if target == name)).expect("drawn session target")
}

fn toggle_click(buf: &Buffer, sidebar: u16) -> Mouse {
    let row = (0..45)
        .find(|y| left(buf, sidebar, *y).contains("Stopped ("))
        .expect("stopped group label");
    Mouse {
        kind: MouseKind::Click,
        column: 5,
        row,
    }
}

#[test]
fn stopped_default_is_one_collapsed_group_after_running_with_no_hidden_targets() {
    let mut app = mixed(None);
    let buf = frame(&mut app, 60, true);
    let labels: Vec<_> = (0..45)
        .filter_map(|y| {
            let text = left(&buf, 60, y);
            text.contains("Stopped (").then_some((y, text))
        })
        .collect();
    assert_eq!(
        labels.len(),
        1,
        "one group, not one toggle per stopped session"
    );
    assert_eq!(labels[0].1, "▸ Stopped (2)");
    let last_running = session_click(&app, "run-b").row;
    assert_eq!(labels[0].0, last_running + 1);
    assert_eq!(
        app.layout.sessions().collect::<Vec<_>>(),
        ["run-a", "run-b"]
    );
    assert!(
        !(0..45)
            .any(|y| left(&buf, 60, y).contains("old-a") || left(&buf, 60, y).contains("old-b"))
    );
    let selected = app.model.selected().map(str::to_owned);
    assert!(app.click(toggle_click(&buf, 60)), "click expands group");
    let expanded = frame(&mut app, 60, true);
    assert_eq!(left(&expanded, 60, labels[0].0), "▾ Stopped (2)");
    assert_eq!(
        app.layout.sessions().collect::<Vec<_>>(),
        ["run-a", "run-b", "old-a", "old-b"]
    );
    assert_eq!(
        app.model.selected(),
        selected.as_deref(),
        "expanding never moves selection"
    );
    assert!(
        app.click(toggle_click(&expanded, 60)),
        "click collapses group"
    );
    let _ = frame(&mut app, 60, true);
    assert_eq!(
        app.layout.sessions().collect::<Vec<_>>(),
        ["run-a", "run-b"]
    );
    assert_eq!(app.world.sessions.len(), 4);
}

#[test]
fn no_stopped_sessions_means_no_group_and_toggle_key_is_a_noop() {
    let mut app = app(vec![row("run-a", 1, false)], None);
    keys(&mut app, b"z");
    let buf = frame(&mut app, 60, true);
    assert!(!(0..45).any(|y| left(&buf, 60, y).contains("Stopped (")));
    assert_eq!(app.model.selected(), Some("run-a"));
}

#[test]
fn z_expands_and_collapses_and_hidden_navigation_never_selects_stopped() {
    let mut app = mixed(None);
    let _ = frame(&mut app, 60, true);
    for bytes in [&b"jjjj"[..], &b"3"[..], &b"4"[..], &b"!"[..]] {
        keys(&mut app, bytes);
        assert!(matches!(app.model.selected(), Some("run-a" | "run-b")));
    }
    keys(&mut app, b"z");
    let _ = frame(&mut app, 60, true);
    let click = session_click(&app, "old-a");
    assert!(app.click(click));
    assert_eq!(app.model.selected(), Some("old-a"));
    assert_eq!(
        app.layout.cards["old-a"].items(),
        &[menu::Item::Start],
        "same existing Start item"
    );
    assert!(app.layout.cards["old-a"].refusal().is_none());
    keys(&mut app, b"z");
    let _ = frame(&mut app, 60, true);
    assert_eq!(
        app.model.selected(),
        Some("run-a"),
        "hidden selection falls back to first running row without visible home"
    );
}

#[test]
fn collapse_prefers_visible_home_and_never_replays_a_hidden_session_click() {
    let mut app = mixed(Some("run-b"));
    keys(&mut app, b"z");
    let _ = frame(&mut app, 60, true);
    let old_click = session_click(&app, "old-b");
    assert!(app.click(old_click));
    assert_eq!(app.model.selected(), Some("old-b"));
    keys(&mut app, b"z");
    assert_eq!(
        app.model.selected(),
        Some("run-b"),
        "visible home wins over first running row"
    );
    // No new frame: the old layout still contains old-b's target.
    let _ = app.click(old_click);
    assert_eq!(
        app.model.selected(),
        Some("run-b"),
        "stale target cannot select hidden row"
    );
}

#[test]
fn all_stopped_has_no_selection_until_expanded_and_none_again_on_collapse() {
    let mut app = app(vec![row("old-a", 1, true)], Some("old-a"));
    let _ = frame(&mut app, 60, true);
    assert_eq!(app.model.selected(), None, "hidden home is not selectable");
    keys(&mut app, b"z1");
    let _ = frame(&mut app, 60, true);
    assert_eq!(app.model.selected(), Some("old-a"));
    keys(&mut app, b"z");
    let _ = frame(&mut app, 60, true);
    assert_eq!(app.model.selected(), None);
}

#[test]
fn running_selection_that_stops_remains_selected_and_opens_group_on_refresh() {
    let mut app = mixed(Some("run-b"));
    let _ = frame(&mut app, 60, true);
    keys(&mut app, b"1");
    assert_eq!(app.model.selected(), Some("run-a"));
    app.answer(Answer::Fleet(read(
        vec![
            row("run-b", 1, false),
            row("run-a", 2, true),
            row("old-a", 3, true),
        ],
        Some("run-b"),
    )));
    let _ = frame(&mut app, 60, true);
    assert_eq!(app.model.selected(), Some("run-a"));
    assert!(app.layout.sessions().any(|name| name == "run-a"));
}

#[test]
fn expanded_preference_survives_fleet_refresh() {
    let mut app = mixed(None);
    keys(&mut app, b"z");
    let _ = frame(&mut app, 60, true);
    app.answer(Answer::Fleet(read(
        vec![row("run-a", 1, false), row("old-c", 2, true)],
        None,
    )));
    let buf = frame(&mut app, 60, true);
    assert!((0..45).any(|y| left(&buf, 60, y) == "▾ Stopped (1)"));
    assert!(app.layout.sessions().any(|name| name == "old-c"));
}

fn agent(name: &str, state: &str, pane: &str) -> PickerAgent {
    PickerAgent {
        name: name.into(),
        profile: "sol61x".into(),
        state: state.into(),
        pane: pane.into(),
        client: String::new(),
        model: String::new(),
        effort: String::new(),
        drift: false,
    }
}

#[test]
fn agent_total_uses_current_live_roster_across_running_sessions_and_ignores_dead_and_stopped() {
    let now = Timestamp::from_epoch(1_792_579_200);
    let world = World::new(
        now,
        vec![
            SessionEntry::new("run-a", Status::Running),
            SessionEntry::new("run-b", Status::Running),
            SessionEntry::new("old-a", Status::Stopped),
        ],
    );
    let facts = BTreeMap::from([
        (
            "run-a".into(),
            Facts::Seats {
                id: "$1".into(),
                agents: vec![
                    agent("lead", "working", "%1"),
                    agent("colead", "waiting-agent", "%2"),
                    agent("build", "done", "%3"),
                    agent("dead-seat", "dead", ""),
                ],
            },
        ),
        (
            "run-b".into(),
            Facts::Seats {
                id: "$2".into(),
                agents: vec![
                    agent("lead", "working", "%4"),
                    agent("colead", "waiting-user", "%5"),
                ],
            },
        ),
        (
            "old-a".into(),
            Facts::Seats {
                id: "$3".into(),
                agents: vec![agent("lead", "working", "%6")],
            },
        ),
    ]);
    let fleet = fleet::rows(
        &world,
        &facts,
        &BTreeMap::new(),
        &FleetOrder::EMPTY,
        &BTreeMap::new(),
        None,
        now,
    );
    let sum = fleet::summary(&fleet);
    assert_eq!(
        sum.agents, 5,
        "lead/co-lead/current workers count, dead and stopped do not; retired roster entries are absent"
    );
    assert_eq!(
        sum.working, 2,
        "existing Working marks, never a new classifier"
    );
    assert!(!sum.unknown);
    let mut fleet = fleet;
    fleet.rows[0].needy = true;
    assert_eq!(
        fleet::summary(&fleet).needy,
        1,
        "Needs you remains count of needy sessions"
    );
    fleet.rows.push(row("stored-old", 4, true));
    assert_eq!(
        fleet::summary(&fleet).agents,
        5,
        "99 saved stopped seats do not count"
    );
}

fn summary_app(unknown: bool) -> App {
    let mut live = row("run-a", 1, false);
    live.counts = Counts::Marks(vec![
        (Mark::Working, false, 10),
        (Mark::Idle, false, 22),
        (Mark::Dead, false, 7),
    ]);
    live.needy = true;
    let mut rows = vec![live, row("old-a", 2, true)];
    if unknown {
        let mut missing = row("unread", 3, false);
        missing.counts = Counts::Unknown;
        rows.push(missing);
    }
    app(rows, None)
}

#[test]
fn summary_is_one_line_in_requested_order_and_keeps_partial_count_marker() {
    for (unknown, expected) in [
        (false, "Agents 32 · Working 10 · Needs you 1"),
        (true, "Agents 32+? · Working 10+? · Needs you 1"),
    ] {
        let mut app = summary_app(unknown);
        let buf = frame(&mut app, 80, true);
        assert_eq!(left(&buf, 80, 0), expected);
        assert!(left(&buf, 80, 1).is_empty(), "no second summary line");
        keys(&mut app, b"z");
        let expanded = frame(&mut app, 80, true);
        assert_eq!(
            left(&expanded, 80, 0),
            expected,
            "group visibility cannot change totals"
        );
    }
}

#[test]
fn last_running_selection_that_stops_stays_visible_until_explicit_collapse() {
    let mut app = app(vec![row("run-a", 1, false)], Some("run-a"));
    let _ = frame(&mut app, 60, true);
    app.answer(Answer::Fleet(read(
        vec![row("run-a", 1, true)],
        Some("run-a"),
    )));
    let _ = frame(&mut app, 60, true);
    assert_eq!(app.model.selected(), Some("run-a"));
    assert!(app.layout.sessions().any(|name| name == "run-a"));
    keys(&mut app, b"z");
    let _ = frame(&mut app, 60, true);
    assert_eq!(app.model.selected(), None);
}

#[test]
fn summary_width_ladder_is_full_words_short_labels_glyph_counts_then_need_only() {
    // Sidebar content width is its rule minus four cells (two on each side).
    // 34 -> 30 cells fits the 28-cell glyph rung; 30 -> 26 needs only.
    // The ladder is pinned through rendered geometry, not a helper's output.
    for icons in [false, true] {
        for (sidebar, huge, expected) in [
            (44, false, "Agents 32 · Working 10 · Needs you 1"),
            (30, false, "Agt 32 · Wrk 10 · Need 1"),
            (
                34,
                true,
                if icons {
                    "◈ 1234567890 ● 123456789 ⚠ 1"
                } else {
                    "@ 1234567890 * 123456789 ! 1"
                },
            ),
            (30, true, if icons { "⚠ 1" } else { "! 1" }),
        ] {
            let mut app = if huge {
                let mut live = row("run-a", 1, false);
                live.needy = true;
                live.counts = Counts::Marks(vec![
                    (Mark::Working, false, 123_456_789),
                    (Mark::Idle, false, 1_111_111_101),
                ]);
                app(vec![live], None)
            } else {
                summary_app(false)
            };
            let buf = frame(&mut app, sidebar, icons);
            assert_eq!(
                left(&buf, sidebar, 0),
                expected,
                "sidebar {sidebar}, huge={huge}, icons={icons}"
            );
            assert!(left(&buf, sidebar, 1).is_empty());
            // Every rung ends in the needy count 1; its ink comes from the
            // palette, including the glyph and needs-only rungs.
            let digit = u16::try_from(expected.chars().count() + 1).expect("summary fits u16");
            let [red, green, blue] =
                crate::theme::rgb(Look::read("on", "darcula", "on", "on").palette.needs_you);
            assert_eq!(
                buf[(digit, 0)].fg,
                ratatui_core::style::Color::Rgb(red, green, blue),
                "needy number ink: sidebar {sidebar}, huge={huge}, icons={icons}"
            );
        }
    }
}

#[test]
fn icons_off_toggle_has_ascii_direction_and_stays_clickable() {
    let mut app = mixed(None);
    let closed = frame(&mut app, 60, false);
    let press = toggle_click(&closed, 60);
    assert_eq!(left(&closed, 60, press.row), "> Stopped (2)");
    assert!(app.click(press));
    let open = frame(&mut app, 60, false);
    assert_eq!(left(&open, 60, press.row), "v Stopped (2)");
}

#[test]
fn expanded_navigation_skips_group_header_and_digits_count_sessions_only() {
    let mut app = mixed(None);
    keys(&mut app, b"z2");
    assert_eq!(app.model.selected(), Some("run-b"));
    keys(&mut app, b"j");
    assert_eq!(
        app.model.selected(),
        Some("old-a"),
        "Down crosses group header without selecting it"
    );
    keys(&mut app, b"k");
    assert_eq!(
        app.model.selected(),
        Some("run-b"),
        "Up crosses group header without selecting it"
    );
    keys(&mut app, b"3");
    assert_eq!(app.model.selected(), Some("old-a"));
    keys(&mut app, b"4");
    assert_eq!(app.model.selected(), Some("old-b"));
}

#[test]
fn restored_view_of_stopped_session_opens_group_and_preserves_explicit_selection() {
    let mut app = mixed(None);
    let _ = frame(&mut app, 60, true);
    app.answer(Answer::Handover {
        view: super::handover::View {
            session: "old-b".into(),
            id: ID.into(),
            focus: None,
            sidebar: None,
            list: None,
            pages: 0,
            rows: 0,
            list_top: None,
            body_top: 0,
        },
        at: Timestamp::now().epoch(),
    });
    app.answer(Answer::Fleet(read(
        vec![row("run-a", 1, false), row("old-b", 2, true)],
        None,
    )));
    let buf = frame(&mut app, 60, true);
    assert_eq!(app.model.selected(), Some("old-b"));
    assert!(app.layout.sessions().any(|name| name == "old-b"));
    assert!((0..45).any(|y| left(&buf, 60, y) == "▾ Stopped (1)"));
}

#[test]
fn stopped_toggle_clicks_keep_working_while_peek_or_reader_owns_the_chat() {
    for reading in [false, true] {
        let mut app = mixed(None);
        if reading {
            app.model.open_reader(1);
        } else {
            app.model.open_peek(1);
        }
        let closed = frame(&mut app, 60, true);
        let selected = app.model.selected().map(str::to_owned);
        assert!(app.mouse(toggle_click(&closed, 60)), "open from panel");
        let open = frame(&mut app, 60, true);
        assert_eq!(
            app.layout.sessions().collect::<Vec<_>>(),
            ["run-a", "run-b", "old-a", "old-b"]
        );
        assert_eq!(app.model.selected(), selected.as_deref());
        assert_eq!(app.model.reader_open(), reading);
        assert_eq!(app.model.peek_open(), !reading);
        assert!(app.mouse(toggle_click(&open, 60)), "close from panel");
        let _ = frame(&mut app, 60, true);
        assert_eq!(
            app.layout.sessions().collect::<Vec<_>>(),
            ["run-a", "run-b"]
        );
        assert_eq!(app.model.selected(), selected.as_deref());
        assert_eq!(app.model.reader_open(), reading);
        assert_eq!(app.model.peek_open(), !reading);
    }
}
