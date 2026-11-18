//! Frozen acceptance for seatfold. Oracle: brief-seatfold.md and lead's
//! 2026-11-18 ruling: no new `!` or header jump. Slots define the group.
//! Real app frames and input paths; no tmux commands or live panes.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::fleet::{Counts, Facts, Fleet, Line2, Row};
use super::loader::{Answer, FleetRead, ViewRead};
use super::model::{Drag, Edge};
use super::overview::Open;
use super::{App, take_keys};
use crate::console::input::{Key, Mouse, MouseKind};
use crate::console::lane::Lane;
use crate::console::needs::SeatRef;
use crate::digest::{SessionEntry, Status};
use crate::listing::World;
use crate::theme::{Look, Mark};
use crate::time::Timestamp;
use crate::tmux::PickerAgent;

const ID: &str = "0199c0de-1234-4890-abcd-ef0123456789";
const SIDEBAR: u16 = 68;

fn fixture(seats: &[(&str, &str, &str)]) -> App {
    assert!(
        seats
            .iter()
            .all(|(_, name, _)| crate::config::is_agent_name(name))
    );
    let mut app = App::new(Some("api".into()), None, None, None);
    let row = Row {
        name: "api".into(),
        index: 1,
        mark: Mark::Working,
        needy: false,
        unanswered: Some(0),
        counts: Counts::Marks(vec![(Mark::Working, false, seats.len())]),
        line2: Line2::NoGoal,
        home: true,
    };
    app.answer(Answer::Fleet(FleetRead {
        dirs: BTreeMap::new(),
        ids: BTreeMap::from([("api".into(), ID.into())]),
        sole: BTreeMap::from([("api".into(), ID.into())]),
        world: World::new(
            Timestamp::now(),
            vec![SessionEntry::new("api", Status::Running)],
        ),
        facts: BTreeMap::new(),
        needs: BTreeMap::new(),
        fleet: Fleet {
            rows: vec![row],
            home: Some("api".into()),
        },
        pairs: BTreeMap::new(),
        scanned: true,
    }));
    install(&mut app, "api", seats);
    app.model.grab(Drag {
        edge: Edge::Sidebar,
        from: 44,
        at: 44,
    });
    app.model.drag_to(SIDEBAR);
    app.model.let_go();
    app
}

fn install(app: &mut App, session: &str, seats: &[(&str, &str, &str)]) {
    app.ids.insert(session.into(), ID.into());
    app.looks.insert(
        session.into(),
        (Some(Look::read("on", "darcula", "on", "on")), None),
    );
    app.facts.insert(
        session.into(),
        Facts::Seats {
            id: "$1".into(),
            agents: seats
                .iter()
                .map(|(_, name, state)| PickerAgent {
                    name: (*name).into(),
                    state: (*state).into(),
                    pane: "%1".into(),
                    profile: String::new(),
                    client: String::new(),
                    model: String::new(),
                    effort: String::new(),
                    drift: false,
                    task: false,
                })
                .collect(),
        },
    );
    app.answer(Answer::View(ViewRead {
        name: session.into(),
        id: ID.into(),
        seq: 1,
        lane: Lane::default(),
        needs: None,
        roster: Some(
            seats
                .iter()
                .map(|(slot, name, _)| SeatRef {
                    slot: (*slot).into(),
                    name: (*name).into(),
                })
                .collect(),
        ),
    }));
}

fn mixed() -> App {
    fixture(&[
        ("main", "lead", "working"),
        ("worker.0", "colead", "done"),
        ("spawned.0", "scout", "working"),
        ("spawned.1", "builder", "working"),
    ])
}

fn frame(app: &mut App, height: u16) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, 200, height));
    app.frame(&mut buf);
    buf
}

fn line(buf: &Buffer, y: u16) -> String {
    (0..SIDEBAR)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
        .trim()
        .to_owned()
}

fn lines(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height).map(|y| line(buf, y)).collect()
}

fn group(buf: &Buffer) -> (u16, String) {
    let found: Vec<_> = (0..buf.area.height)
        .filter_map(|y| {
            let text = line(buf, y);
            text.contains("Spawned (").then_some((y, text))
        })
        .collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one spawned group: {:?}",
        lines(buf)
    );
    found[0].clone()
}

fn click_group(app: &mut App, buf: &Buffer) {
    assert!(app.mouse(Mouse {
        kind: MouseKind::Click,
        column: 5,
        row: group(buf).0
    }));
}

fn key(app: &mut App, bytes: &[u8]) {
    assert!(take_keys(app, vec![(Key::Text(bytes.to_vec()), Instant::now())]).is_some());
}

#[test]
fn starts_open_with_one_group_after_workspace_and_counts_all_seats() {
    let mut app = mixed();
    let buf = frame(&mut app, 45);
    let (y, text) = group(&buf);
    assert_eq!(text, "▾ Spawned (2)");
    assert_eq!(app.layout.seats, ["lead", "colead", "scout", "builder"]);
    let last_fixed = (0..y)
        .find(|row| line(&buf, *row).contains("colead"))
        .expect("fixed seat");
    let lead = (0..45)
        .find(|row| line(&buf, *row).contains("● lead"))
        .expect("lead row");
    assert!(lead < last_fixed && last_fixed < y);
    assert!((y + 1..45).any(|row| line(&buf, row).contains("scout")));
    assert!(lines(&buf).iter().any(|row| row == "Agents 4"));
}

#[test]
fn key_and_click_toggle_only_the_spawned_rows_and_keep_totals() {
    let mut app = mixed();
    let open = frame(&mut app, 45);
    let summary = line(&open, 0);
    key(&mut app, b"Z");
    let closed = frame(&mut app, 45);
    assert_eq!(group(&closed).1, "▸ Spawned (2) ●2");
    assert_eq!(app.layout.seats, ["lead", "colead"]);
    assert_eq!(line(&closed, 0), summary);
    assert!(lines(&closed).iter().any(|row| row == "Agents 4"));
    assert!(
        !lines(&closed)
            .iter()
            .any(|row| row.contains("scout") || row.contains("builder"))
    );
    click_group(&mut app, &closed);
    assert_eq!(group(&frame(&mut app, 45)).1, "▾ Spawned (2)");
    let opened = frame(&mut app, 45);
    click_group(&mut app, &opened);
    assert_eq!(app.model.selected(), Some("api"));
    assert_eq!(group(&frame(&mut app, 45)).1, "▸ Spawned (2) ●2");
}

#[test]
fn roster_slots_win_over_names_and_unknown_reason_rows_stay_outside_fold() {
    let mut app = fixture(&[
        ("main", "spawned", "working"),
        ("worker.0", "worker", "working"),
        ("spawned.0", "lead", "working"),
    ]);
    app.overview.open.push(Open {
        seat: "orphan".into(),
        text: "unknown-slot request".into(),
        age_secs: None,
    });
    let _ = frame(&mut app, 45);
    key(&mut app, b"Z");
    let closed = frame(&mut app, 45);
    assert_eq!(group(&closed).1, "▸ Spawned (1) ●1");
    assert_eq!(app.layout.seats, ["spawned", "worker", "orphan"]);
    assert!(!lines(&closed).iter().any(|row| row.contains("lead")));
    assert!((group(&closed).0 + 1..45).any(|y| line(&closed, y).contains("orphan")));
}

#[test]
fn no_spawned_seat_has_no_group_and_toggle_changes_nothing() {
    for seats in [vec![], vec![("main", "lead", "working")]] {
        let mut app = fixture(&seats);
        let before = frame(&mut app, 45);
        key(&mut app, b"Z");
        let after = frame(&mut app, 45);
        assert_eq!(before, after);
        assert!(!lines(&after).iter().any(|row| row.contains("Spawned (")));
    }
}

#[test]
fn fold_is_one_flag_across_sessions_and_new_apps_start_open() {
    let mut app = mixed();
    let _ = frame(&mut app, 45);
    key(&mut app, b"Z");
    let mut row = app.fleet.rows[0].clone();
    row.name = "other".into();
    row.index = 2;
    row.home = false;
    app.fleet.rows.push(row);
    app.world
        .sessions
        .push(SessionEntry::new("other", Status::Running));
    install(
        &mut app,
        "other",
        &[
            ("main", "other-lead", "working"),
            ("spawned.2", "other-job", "done"),
        ],
    );
    key(&mut app, b"j");
    assert_eq!(app.model.selected(), Some("other"));
    assert_eq!(group(&frame(&mut app, 45)).1, "▸ Spawned (1) ✓1");
    key(&mut app, b"Z");
    let _ = frame(&mut app, 45);
    key(&mut app, b"k");
    assert_eq!(group(&frame(&mut app, 45)).1, "▾ Spawned (2)");
    assert_eq!(group(&frame(&mut mixed(), 45)).1, "▾ Spawned (2)");
}

#[test]
fn ascii_arrows_and_hidden_mark_tally_keep_the_session_urgency_order() {
    let mut app = fixture(&[
        ("main", "lead", "working"),
        ("spawned.0", "a", "done"),
        ("spawned.1", "b", "working"),
        ("spawned.2", "c", "waiting-user"),
        ("spawned.3", "d", "dead"),
        ("spawned.4", "e", "working"),
        ("spawned.5", "f", "waiting-agent"),
    ]);
    let _ = frame(&mut app, 45);
    key(&mut app, b"Z");
    assert_eq!(
        group(&frame(&mut app, 45)).1,
        "▸ Spawned (6) ✖1 ⚠1 ●2 ◔1 ✓1"
    );
    app.looks.insert(
        "api".into(),
        (Some(Look::read("off", "darcula", "on", "on")), None),
    );
    assert_eq!(
        group(&frame(&mut app, 45)).1,
        "> Spawned (6) x1 !1 *2 ~1 +1"
    );
    key(&mut app, b"Z");
    assert_eq!(group(&frame(&mut app, 45)).1, "v Spawned (6)");
}

#[test]
fn hidden_blocked_seat_keeps_the_blocked_letter_in_both_vocabularies() {
    let mut app = fixture(&[
        ("main", "lead", "working"),
        ("spawned.0", "blocked-job", "blocked"),
    ]);
    let _ = frame(&mut app, 45);
    key(&mut app, b"Z");
    assert_eq!(group(&frame(&mut app, 45)).1, "▸ Spawned (1) ⚠B1");
    app.looks.insert(
        "api".into(),
        (Some(Look::read("off", "darcula", "on", "on")), None),
    );
    assert_eq!(group(&frame(&mut app, 45)).1, "> Spawned (1) !B1");
}

#[test]
fn folding_clears_spawned_cursor_and_keys_in_the_same_read_cannot_reach_it() {
    let mut app = mixed();
    let _ = frame(&mut app, 45);
    key(&mut app, b"p");
    assert_eq!(app.model.focus(), Some("builder"));
    key(&mut app, b"Z");
    assert_eq!(app.model.focus(), None);
    key(&mut app, b"o");
    assert!(!app.armed.as_ref().is_some_and(|arm| matches!(&arm.what,
        super::Arm::Open(drawn) if drawn.focused.as_deref() == Some("builder") || drawn.focused.as_deref() == Some("scout"))));
    key(&mut app, b"o");
    assert!(app.pane_view.is_none());
    // No new frame: pruning must already protect the next key in this read.
    key(&mut app, b"p");
    assert_eq!(app.model.focus(), Some("colead"));
    key(&mut app, b"nn");
    assert_eq!(app.model.focus(), Some("colead"));
    let _ = frame(&mut app, 45);
    key(&mut app, b"n");
    assert_eq!(app.model.focus(), Some("lead"));
}

#[test]
fn folding_keeps_a_workspace_cursor_and_unfolding_moves_no_cursor() {
    let mut app = mixed();
    let _ = frame(&mut app, 45);
    key(&mut app, b"n");
    key(&mut app, b"Z");
    assert_eq!(app.model.focus(), Some("lead"));
    let _ = frame(&mut app, 45);
    key(&mut app, b"Z");
    assert_eq!(app.model.focus(), Some("lead"));
}

#[test]
fn clicking_fold_closes_a_spawned_peek_but_keeps_a_workspace_peek() {
    for (seat, remains) in [("scout", false), ("lead", true)] {
        let mut app = mixed();
        app.server = Some(crate::inventory::ServerId::Ambient);
        app.dirs
            .insert("api".into(), PathBuf::from("/seatfold-fixture"));
        let _ = frame(&mut app, 45);
        app.focus_seat(Some(seat.into()));
        key(&mut app, b"v");
        assert!(app.model.peek_open(), "the real peek input path opens");
        key(&mut app, b"Z");
        assert!(app.model.peek_open(), "peek owns the Z key");
        let shown = frame(&mut app, 45);
        assert_eq!(group(&shown).1, "▾ Spawned (2)", "Z is swallowed in peek");
        click_group(&mut app, &shown);
        assert_eq!(app.model.peek_open(), remains, "peek on {seat}");
        if !remains {
            assert_eq!(app.peek.target, None);
            assert_eq!(app.model.focus(), None);
        }
    }
}

#[test]
fn folded_spawned_only_session_has_no_seat_for_navigation_or_peek() {
    let mut app = fixture(&[("spawned.0", "only-job", "working")]);
    let _ = frame(&mut app, 45);
    key(&mut app, b"Z");
    let _ = frame(&mut app, 45);
    assert!(app.layout.seats.is_empty());
    key(&mut app, b"npv");
    assert_eq!(app.model.focus(), None);
    assert!(!app.model.peek_open());
    assert!(app.drawn.as_ref().expect("drawn session").rows.is_empty());
}

#[test]
fn header_keeps_hidden_need_and_need_jump_does_not_unfold() {
    let mut app = mixed();
    app.overview.open.push(Open {
        seat: "scout".into(),
        text: "NEED-TEXT".into(),
        age_secs: None,
    });
    app.fleet.rows[0].needy = true;
    let _ = frame(&mut app, 45);
    key(&mut app, b"Z");
    let buf = frame(&mut app, 45);
    let full = (0..200).map(|x| buf[(x, 0)].symbol()).collect::<String>();
    assert!(full.contains("scout: NEED-TEXT"), "{full}");
    key(&mut app, b"!");
    assert!(group(&frame(&mut app, 45)).1.starts_with("▸ Spawned ("));
}

#[test]
fn scrolling_counts_visible_body_rows_and_fold_clamps_the_offset() {
    let owned: Vec<_> = (0..20)
        .map(|n| (format!("spawned.{n}"), format!("job-{n}")))
        .collect();
    let mut seats = vec![("main", "lead", "working")];
    seats.extend(
        owned
            .iter()
            .map(|(slot, name)| (slot.as_str(), name.as_str(), "working")),
    );
    let mut app = fixture(&seats);
    let buf = frame(&mut app, 20);
    assert!(lines(&buf).iter().any(|row| row.contains("more rows")));
    let first = app.layout.body_top.expect("body offset");
    let body_y = (0..20)
        .find(|y| line(&buf, *y) == "Agents 21")
        .expect("Agents heading")
        + 2;
    assert!(app.mouse(Mouse {
        kind: MouseKind::WheelDown,
        column: 5,
        row: body_y
    }));
    let _ = frame(&mut app, 20);
    assert_eq!(app.layout.body_top, Some(first + 3));
    key(&mut app, b"Z");
    let closed = frame(&mut app, 20);
    assert_eq!(
        app.layout.body_max, 0,
        "only fixed rows and one group remain"
    );
    assert_eq!(app.layout.body_top, Some(0));
    assert!(!lines(&closed).iter().any(|row| row.contains("more rows")));
    assert_eq!(app.layout.seats, ["lead"]);
}

#[test]
fn panel_focus_routes_spawned_toggle_to_the_sidebar() {
    let mut app = mixed();
    let _ = frame(&mut app, 45);
    key(&mut app, b"O");
    let _ = frame(&mut app, 45);
    assert!(app.panel_focused());
    key(&mut app, b"Z");
    let closed = frame(&mut app, 45);
    assert!(group(&closed).1.starts_with("▸ Spawned ("));
    click_group(&mut app, &closed);
    assert_eq!(group(&frame(&mut app, 45)).1, "▾ Spawned (2)");
}

#[test]
fn toggle_in_fixed_only_session_does_not_change_the_flag() {
    let mut app = fixture(&[("main", "lead", "working")]);
    let _ = frame(&mut app, 45);
    key(&mut app, b"Z");
    let mut row = app.fleet.rows[0].clone();
    row.name = "other".into();
    row.index = 2;
    row.home = false;
    app.fleet.rows.push(row);
    app.world
        .sessions
        .push(SessionEntry::new("other", Status::Running));
    install(
        &mut app,
        "other",
        &[
            ("main", "other-lead", "working"),
            ("spawned.0", "job", "working"),
        ],
    );
    key(&mut app, b"j");
    assert_eq!(group(&frame(&mut app, 45)).1, "▾ Spawned (1)");
}

#[test]
fn toggle_works_when_all_spawned_seats_are_below_the_viewport() {
    let owned: Vec<_> = (0..25)
        .map(|n| (format!("worker.{n}"), format!("fixed-{n}")))
        .collect();
    let mut seats: Vec<_> = owned
        .iter()
        .map(|(slot, name)| (slot.as_str(), name.as_str(), "working"))
        .collect();
    seats.push(("spawned.0", "hidden-job", "working"));
    let mut app = fixture(&seats);
    let _ = frame(&mut app, 20);
    assert!(!app.layout.seats.iter().any(|name| name == "hidden-job"));
    key(&mut app, b"Z");
    // Widening the viewport makes the flag observable without changing it.
    let tall = frame(&mut app, 60);
    assert_eq!(group(&tall).1, "▸ Spawned (1) ●1");
    assert!(!app.layout.seats.iter().any(|name| name == "hidden-job"));
}

#[test]
fn composing_keeps_uppercase_z_as_text_and_click_can_toggle() {
    let root = super::tests::Root::new("seatfold-write");
    let mut app = mixed();
    let dir = root.0.join("sessions/api");
    app.dirs.insert("api".into(), dir.clone());
    let console = crate::console::Console::bound("api".into(), dir, ID.into());
    let pair =
        crate::console::term::pair_of(console.seats().expect("fixture seats")).expect("lead pair");
    app.pairs.insert("api".into(), Ok(pair));
    app.show();
    let _ = frame(&mut app, 45);
    key(&mut app, b"i");
    assert!(app.composing());
    key(&mut app, b"Z");
    assert_eq!(app.shown_input().expect("composer").draft(), "Z");
    let open = frame(&mut app, 45);
    assert_eq!(group(&open).1, "▾ Spawned (2)");
    click_group(&mut app, &open);
    assert!(app.composing());
    assert_eq!(app.shown_input().expect("composer").draft(), "Z");
    assert!(group(&frame(&mut app, 45)).1.starts_with("▸ Spawned ("));
}
