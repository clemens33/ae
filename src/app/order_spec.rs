//! Frozen apporder acceptance: R1-R5 + lead Q1-Q3/R4 rulings, 2026-10-08.
//! The oracle is the brief and measured tmux 3.4/3.7b table, never the sorter.
//! Raw listing fixtures exercise the existing parser/fold boundary; journal
//! fixtures exercise the real session-entry fold. No implementation stubs.

#![allow(clippy::expect_used, reason = "bounded acceptance fixtures")]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::loader::{Answer, FleetRead, Reader, Wake};
use super::tests::{Root, session};
use super::{App, drain, draw, fleet};
use crate::console::input::{Keys, Mouse, MouseKind};
use crate::digest::Status;
use crate::listing::World;
use crate::session::{SessionRuntime, entry_for};
use crate::theme::{FleetOrder, FleetRow, Look, Mark};
use crate::time::Timestamp;
use crate::{store, theme, tmux};

const NOW: i64 = 1_791_456_000;

fn now() -> Timestamp {
    Timestamp::from_epoch(NOW)
}

// name, monotonically assigned tmux id, activity, creation, free-text goal.
type Live<'a> = (&'a str, u64, &'a str, &'a str, &'a str);

fn listing(live: &[Live<'_>]) -> Vec<tmux::PickerSession> {
    let mut raw = String::new();
    for (name, id, activity, created, goal) in live {
        writeln!(raw,
            "{name} | ${id} | 0 |  | %7 |  | v1;{NOW};30;lead:sonnet55x:working:%7 | {activity} | {created} | {goal}"
        ).expect("raw listing row");
    }
    tmux::interpret_picker_sessions(true, &raw).expect("successful listing")
}

fn fold(
    root: &Root,
    sessions: &[(&str, Status)],
    live: &[Live<'_>],
    order: &FleetOrder,
) -> FleetRead {
    let dirs: BTreeMap<String, PathBuf> = sessions
        .iter()
        .map(|(name, _)| {
            let dir = root.0.join("sessions").join(name);
            (name.to_string(), dir)
        })
        .collect();
    let entries = sessions
        .iter()
        .map(|(name, status)| {
            entry_for(
                &dirs[*name],
                name,
                &SessionRuntime::new(*status),
                now(),
                300,
            )
        })
        .collect();
    let picker = listing(live);
    Reader::new(root.0.clone(), None, None).fold(
        dirs,
        World::new(now(), entries),
        Some(&picker),
        order,
        now(),
    )
}

fn names(fleet: &fleet::Fleet) -> Vec<&str> {
    fleet.rows.iter().map(|row| row.name.as_str()).collect()
}

fn recorded(root: &Root, name: &str, actor: &str, action: &str, epoch: i64) {
    let dir = session(root, name, "");
    let line = format!(
        "{{\"ts\":\"{}\",\"actor\":\"{actor}\",\"action\":\"{action}\",\"target\":\"lead\",\"target_session\":\"{name}\",\"summary\":\"fixture\"}}\n",
        Timestamp::from_epoch(epoch)
    );
    store::open(&dir)
        .append_event(&line)
        .expect("fixture event");
}

fn answered(root: &Root, name: &str, sink: &str, epoch: i64) {
    let dir = session(root, name, "");
    let line = format!(
        "{{\"ts\":\"{}\",\"actor\":\"lead\",\"action\":\"reply\",\"target\":\"{sink}\",\"target_session\":\"{name}\",\"summary\":\"fixture\"}}\n",
        Timestamp::from_epoch(epoch)
    );
    store::open(&dir)
        .append_event(&line)
        .expect("fixture event");
}

#[test]
fn orchestrator_then_config_pins_then_answer_recency_then_unknown_then_stopped() {
    let root = Root::new("order-precedence");
    recorded(&root, "stopold", "console:local", "ask", NOW - 1);
    answered(&root, "recent", "console:local", NOW - 10);
    answered(&root, "older", "telegram:42", NOW - 20);
    for (name, tail) in [
        ("orchestrator", ""),
        ("pin1", ""),
        ("pin2", ""),
        ("older", ""),
        ("recent", ""),
        ("unknown", ""),
        ("stopnew", "launch_time.main=1791455900\n"),
        ("stopold", "launch_time.main=1791455800\n"),
        ("undated", ""),
    ] {
        session(&root, name, tail);
    }
    let read = fold(
        &root,
        &[
            ("unknown", Status::Running),
            ("stopold", Status::Stopped),
            ("recent", Status::Running),
            ("pin2", Status::Running),
            ("undated", Status::Stopped),
            ("older", Status::Running),
            ("orchestrator", Status::Running),
            ("stopnew", Status::Stopped),
            ("pin1", Status::Running),
        ],
        &[
            ("unknown", 0, "1791455999", "1791455999", ""),
            ("older", 1, "1791455000", "1791454000", ""),
            ("recent", 2, "1791455900", "1791454000", ""),
            ("pin2", 3, "1791455990", "1791454000", ""),
            ("pin1", 4, "", "1791454000", ""),
            ("orchestrator", 9, "", "1791454000", ""),
        ],
        &FleetOrder::from_validated(vec!["pin1".into(), "pin2".into(), "stopold".into()]),
    );
    assert_eq!(
        names(&read.fleet),
        [
            "orchestrator",
            "pin1",
            "pin2",
            "recent",
            "older",
            "unknown",
            "stopnew",
            "stopold",
            "undated"
        ]
    );
    let indices: Vec<usize> = read.fleet.rows.iter().map(|row| row.index).collect();
    assert_eq!(indices, (1..=9).collect::<Vec<_>>());
}

#[test]
fn equal_activity_and_unknowns_keep_creation_order_instead_of_input_or_name_order() {
    let root = Root::new("order-ties");
    let read = fold(
        &root,
        &[
            ("aa", Status::Running),
            ("ab", Status::Running),
            ("za", Status::Running),
            ("zb", Status::Running),
        ],
        &[
            ("aa", 4, "0", "1791454000", ""),
            ("ab", 3, "", "1791454000", ""),
            ("za", 2, "1791455000", "1791454000", ""),
            ("zb", 1, "1791455000", "1791454000", ""),
        ],
        &FleetOrder::EMPTY,
    );
    assert_eq!(names(&read.fleet), ["zb", "za", "ab", "aa"]);
}

#[test]
fn untouched_and_invalid_tmux_times_cannot_outrank_a_known_ai_answer() {
    let root = Root::new("order-untouched");
    answered(&root, "visited", "console:local", NOW - 10);
    let read = fold(
        &root,
        &[
            ("fresh", Status::Running),
            ("invalid", Status::Running),
            ("backward", Status::Running),
            ("nocreated", Status::Running),
            ("visited", Status::Running),
        ],
        &[
            ("fresh", 0, "1791455999", "1791455999", ""),
            ("invalid", 1, "garbage", "1791454000", ""),
            ("backward", 2, "1791453000", "1791454000", ""),
            ("nocreated", 3, "1791455990", "", ""),
            ("visited", 4, "1791455000", "1791454000", ""),
        ],
        &FleetOrder::EMPTY,
    );
    assert_eq!(
        names(&read.fleet),
        ["visited", "fresh", "invalid", "backward", "nocreated"]
    );
}

#[test]
fn newest_reply_to_a_human_sink_counts_and_other_records_do_not() {
    let root = Root::new("order-human-journal");
    recorded(&root, "remote", "console:local", "ask", NOW - 10);
    recorded(&root, "remote", "console:local", "ask", NOW - 100);
    answered(&root, "remote", "console:local", NOW - 50);
    answered(&root, "remote", "telegram:42", NOW - 5);
    recorded(&root, "agent", "lead", "ask", NOW);
    answered(&root, "near", "console:localx", NOW);
    recorded(&root, "review", "console:local", "review", NOW);
    recorded(&root, "cancel", "console:local", "cancel", NOW);
    answered(&root, "ledger", "ae:seats:0199c0de", NOW);
    let read = fold(
        &root,
        &[
            ("agent", Status::Running),
            ("near", Status::Running),
            ("review", Status::Running),
            ("cancel", Status::Running),
            ("ledger", Status::Running),
            ("remote", Status::Running),
        ],
        &[],
        &FleetOrder::EMPTY,
    );
    assert_eq!(
        names(&read.fleet),
        ["remote", "agent", "cancel", "ledger", "near", "review"]
    );
}

#[test]
fn newest_ai_answer_wins_regardless_of_tmux_touch_even_for_untouched_sessions() {
    let root = Root::new("order-source-max");
    answered(&root, "older", "console:local", NOW - 100);
    answered(&root, "older", "telegram:42", NOW - 90);
    answered(&root, "newer", "discord:7", NOW - 10);
    let read = fold(
        &root,
        &[
            ("newer", Status::Running),
            ("untouched", Status::Running),
            ("older", Status::Running),
        ],
        &[
            ("newer", 0, "1791455900", "1791454000", ""),
            ("untouched", 1, "1791455999", "1791455999", ""),
            ("older", 2, "1791455995", "1791454000", ""),
        ],
        &FleetOrder::EMPTY,
    );
    assert_eq!(names(&read.fleet), ["newer", "older", "untouched"]);
}

#[test]
fn sidebar_orders_running_rows_by_newest_ai_answer_to_human() {
    let root = Root::new("order-answer-key");
    answered(&root, "console", "console:local", NOW - 30);
    answered(&root, "tg", "telegram:42", NOW - 10);
    answered(&root, "dc", "discord:7", NOW - 20);
    let read = fold(
        &root,
        &[
            ("console", Status::Running),
            ("tg", Status::Running),
            ("dc", Status::Running),
            ("none", Status::Running),
        ],
        &[
            ("none", 0, "1791455999", "1791454000", ""),
            ("console", 1, "1791455990", "1791454000", ""),
            ("dc", 2, "1791455980", "1791454000", ""),
            ("tg", 3, "1791455970", "1791454000", ""),
        ],
        &FleetOrder::EMPTY,
    );
    assert_eq!(names(&read.fleet), ["tg", "dc", "console", "none"]);
}

#[test]
fn human_input_and_tmux_touch_move_no_row() {
    let root = Root::new("order-human-still");
    recorded(&root, "touched", "console:local", "ask", NOW - 10);
    answered(&root, "answered", "console:local", NOW - 100);
    let read = fold(
        &root,
        &[("touched", Status::Running), ("answered", Status::Running)],
        &[
            ("touched", 0, "1791455999", "1791454000", ""),
            ("answered", 1, "1791455000", "1791454000", ""),
        ],
        &FleetOrder::EMPTY,
    );
    assert_eq!(names(&read.fleet), ["answered", "touched"]);
}

#[test]
fn parser_preserves_new_and_legacy_goals_with_pipes_and_a_numeric_legacy_goal() {
    let modern = listing(&[("new", 1, "1791455990", "1791454000", "ship | v1 | keep")]);
    assert_eq!(modern[0].goal, "ship | v1 | keep");
    for goal in ["12345", "ship | v1 | keep", "ship | v1"] {
        let raw = format!("old | $0 | 0 |  | %7 |  |  | {goal}\n");
        let old = tmux::interpret_picker_sessions(true, &raw).expect("legacy listing");
        assert_eq!(old[0].goal, goal);
    }
}

#[test]
fn sidebar_recency_leaves_strip_and_picker_creation_order_unchanged() {
    let root = Root::new("order-sidebar-only");
    answered(&root, "new", "console:local", NOW - 10);
    let sessions = [("old", Status::Running), ("new", Status::Running)];
    let live = [
        ("old", 0, "1791455000", "1791454000", ""),
        ("new", 1, "1791455900", "1791454000", ""),
    ];
    let read = fold(&root, &sessions, &live, &FleetOrder::EMPTY);
    assert_eq!(names(&read.fleet), ["new", "old"]);
    let strip_rows: Vec<FleetRow> = listing(&live)
        .iter()
        .map(|row| FleetRow {
            name: row.name.clone(),
            id: row.id.clone(),
            mark: Mark::Idle,
            current: false,
        })
        .collect();
    let strip = theme::fleet_strip(&Look::DEFAULT, &strip_rows, None, &FleetOrder::EMPTY);
    assert!(strip.find("old").expect("old on strip") < strip.find("new").expect("new on strip"));
    assert_eq!(
        theme::next_fleet_session(&strip_rows, "old", &FleetOrder::EMPTY).as_deref(),
        Some("new")
    );
    // The picker's public shared tail remains creation-based for equal rank.
    assert!(theme::fleet_tail_cmp(&FleetOrder::EMPTY, ("old", 0), ("new", 1)).is_lt());
}

fn read_order(root: &Root, live: &[Live<'_>]) -> FleetRead {
    fold(
        root,
        &[
            ("a", Status::Running),
            ("b", Status::Running),
            ("c", Status::Running),
        ],
        live,
        &FleetOrder::EMPTY,
    )
}

fn initial_app(root: &Root) -> App {
    let mut app = App::new(None, None, None, None);
    answered(root, "a", "console:local", NOW - 10);
    answered(root, "b", "console:local", NOW - 20);
    answered(root, "c", "console:local", NOW - 30);
    app.answer(Answer::Fleet(read_order(
        root,
        &[
            ("a", 0, "1791455300", "1791454000", ""),
            ("b", 1, "1791455200", "1791454000", ""),
            ("c", 2, "1791455100", "1791454000", ""),
        ],
    )));
    frame(&mut app);
    app
}

fn reordered(root: &Root) -> FleetRead {
    answered(root, "b", "console:local", NOW - 5);
    read_order(
        root,
        &[
            ("a", 0, "1791455300", "1791454000", ""),
            ("b", 1, "1791455900", "1791454000", ""),
            ("c", 2, "1791455100", "1791454000", ""),
        ],
    )
}

fn frame(app: &mut App) {
    app.frame(&mut Buffer::empty(Rect::new(0, 0, 160, 45)));
}

#[test]
fn a_fleet_refresh_moves_rows_at_the_frame_and_keeps_the_selected_name() {
    let root = Root::new("order-frame-selection");
    let mut app = initial_app(&root);
    assert_eq!(app.model.selected(), Some("a"));
    app.answer(Answer::Fleet(reordered(&root)));
    assert_eq!(names(&app.fleet), ["a", "b", "c"], "still the drawn order");
    frame(&mut app);
    assert_eq!(names(&app.fleet), ["b", "a", "c"]);
    assert_eq!(app.model.selected(), Some("a"));
    assert_eq!(app.model.position(&app.fleet), Some(1));
}

#[test]
fn same_drain_digits_and_arrows_act_on_the_last_frame_before_reordering() {
    for (tag, bytes, selected) in [("digit", &b"1"[..], "a"), ("arrow", &b"j"[..], "b")] {
        let root = Root::new(&format!("order-drain-{tag}"));
        let mut app = initial_app(&root);
        let (tx, rx) = mpsc::channel();
        tx.send(Wake::Keys(Instant::now(), bytes.to_vec()))
            .expect("queued key");
        assert_eq!(
            drain(
                &mut app,
                &mut Keys::app(),
                &rx,
                &mut Some(Wake::Answer(Box::new(Answer::Fleet(reordered(&root)))))
            ),
            Some(true)
        );
        assert_eq!(app.model.selected(), Some(selected), "the human saw a,b,c");
        frame(&mut app);
        assert_eq!(names(&app.fleet), ["b", "a", "c"]);
        assert_eq!(app.model.selected(), Some(selected));
    }
}

#[test]
fn two_unpainted_reads_keep_the_drawn_click_name_and_use_the_latest_order_on_frame() {
    let root = Root::new("order-two-reads-click");
    let mut app = initial_app(&root);
    let click = (0..45)
        .flat_map(|row| {
            (0..160).map(move |column| Mouse {
                kind: MouseKind::Click,
                column,
                row,
            })
        })
        .find(
            |mouse| matches!(app.layout.hit(*mouse), Some(draw::Hit::Session(name)) if name == "b"),
        )
        .expect("b target in the drawn frame");
    app.answer(Answer::Fleet(reordered(&root)));
    answered(&root, "c", "console:local", NOW - 1);
    app.answer(Answer::Fleet(read_order(
        &root,
        &[
            ("a", 0, "1791455300", "1791454000", ""),
            ("b", 1, "1791455900", "1791454000", ""),
            ("c", 2, "1791455990", "1791454000", ""),
        ],
    )));
    assert!(app.click(click));
    assert_eq!(app.model.selected(), Some("b"));
    frame(&mut app);
    assert_eq!(names(&app.fleet), ["c", "b", "a"]);
    assert_eq!(app.model.selected(), Some("b"));
}

#[test]
fn a_wheel_between_refresh_and_frame_keeps_the_selected_session() {
    let root = Root::new("order-wheel");
    let mut app = initial_app(&root);
    app.answer(Answer::Fleet(reordered(&root)));
    let _ = app.mouse(Mouse {
        kind: MouseKind::WheelDown,
        column: 5,
        row: 5,
    });
    assert_eq!(app.model.selected(), Some("a"));
    frame(&mut app);
    assert_eq!(names(&app.fleet), ["b", "a", "c"]);
    assert_eq!(app.model.selected(), Some("a"));
}
