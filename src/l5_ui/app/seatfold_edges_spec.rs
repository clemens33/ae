//! Edges of the spawned group that the frozen acceptance in `seatfold_spec`
//! leaves open: how the folded tally gives way in a narrow sidebar, where a
//! click on the group line lands, a stopped session's stale seat facts and the
//! typed wait word that the seat rows carry over from the old per-seat loop.
//! Oracles are the brief's grammar (the session tally's marks, letters and
//! order; a tally that does not fit gives way like a session's) and arithmetic
//! on the cells written out in each test, never the implementation's output.

use std::collections::BTreeMap;
use std::time::Instant;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::fleet::{Counts, Facts, Fleet, Line2, Row};
use super::loader::{Answer, FleetRead, ViewRead};
use super::model::{Drag, Edge};
use super::{App, take_keys};
use crate::console::input::{Key, Mouse, MouseKind};
use crate::console::lane::Lane;
use crate::console::needs::SeatRef;
use crate::digest::{AgentEntry, Declaration, SessionEntry, Status};
use crate::listing::World;
use crate::theme::{Look, Mark};
use crate::time::Timestamp;
use crate::tmux::PickerAgent;

const ID: &str = "0199c0de-1234-4890-abcd-ef0123456789";

/// A session `api` with these seats, `(slot, name, state)`, in a sidebar `width`
/// cells wide.
fn fixture(seats: &[(&str, &str, &str)], width: u16) -> App {
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
    app.looks.insert(
        "api".into(),
        (Some(Look::read("on", "darcula", "on", "on")), None),
    );
    app.facts.insert(
        "api".into(),
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
        name: "api".into(),
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
    app.model.grab(Drag {
        edge: Edge::Sidebar,
        from: 44,
        at: 44,
    });
    app.model.drag_to(width);
    app.model.let_go();
    app
}

fn frame(app: &mut App) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, 200, 45));
    app.frame(&mut buf);
    buf
}

/// The sidebar's row `y`, `width` cells, trimmed.
fn line(buf: &Buffer, y: u16, width: u16) -> String {
    (0..width)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The one group line and its row, or a failure naming every row.
fn group(buf: &Buffer, width: u16) -> (u16, String) {
    let rows: Vec<(u16, String)> = (0..buf.area.height)
        .map(|y| (y, line(buf, y, width)))
        .collect();
    let found: Vec<&(u16, String)> = rows
        .iter()
        .filter(|(_, text)| text.contains("Spawned ("))
        .collect();
    assert_eq!(found.len(), 1, "exactly one spawned group: {rows:?}");
    found[0].clone()
}

fn key(app: &mut App, bytes: &[u8]) {
    assert!(take_keys(app, vec![(Key::Text(bytes.to_vec()), Instant::now())]).is_some());
}

/// Six spawned seats that wear five marks, in the session tally's order:
/// dead, needs you, working twice, waiting on an agent, done.
fn six() -> [(&'static str, &'static str, &'static str); 7] {
    [
        ("main", "lead", "working"),
        ("spawned.0", "a", "done"),
        ("spawned.1", "b", "working"),
        ("spawned.2", "c", "waiting-user"),
        ("spawned.3", "d", "dead"),
        ("spawned.4", "e", "working"),
        ("spawned.5", "f", "waiting-agent"),
    ]
}

/// The Agents body keeps `width - 4` cells per row. `▸ Spawned (6)` takes 13 of
/// them, each mark and count (`✖1`) takes its two cells and a blank before
/// it, and the cut tally closes on a blank and `…`: five parts need 13 + 15 = 28
/// cells, four and the cut need 27, three and the cut need 24. The line gives
/// way like a session's tally does: the richest that fits, else fewer parts
/// closed on `…`.
#[test]
fn the_folded_tally_gives_way_cell_by_cell_like_a_session_tally() {
    for (width, want) in [
        (30, "▸ Spawned (6) ✖1 ⚠1 ●2 …"),
        (31, "▸ Spawned (6) ✖1 ⚠1 ●2 ◔1 …"),
        (32, "▸ Spawned (6) ✖1 ⚠1 ●2 ◔1 ✓1"),
        (44, "▸ Spawned (6) ✖1 ⚠1 ●2 ◔1 ✓1"),
    ] {
        let mut app = fixture(&six(), width);
        let _ = frame(&mut app);
        key(&mut app, b"Z");
        let shown = frame(&mut app);
        assert_eq!(group(&shown, width).1, want, "sidebar {width} cells wide");
    }
}

/// A press at the last cell of the line's area folds the group, and a press
/// just right of it, in the gutter before the sidebar's border, does not.
#[test]
fn the_group_line_takes_a_press_up_to_its_last_cell_and_no_further() {
    let width = 44;
    let mut app = fixture(&six(), width);
    let shown = frame(&mut app);
    let (row, text) = group(&shown, width);
    assert_eq!(text, "▾ Spawned (6)");
    // The area ends at `width - 2`: `width - 3` is its last cell.
    let beside = Mouse {
        kind: MouseKind::Click,
        column: width - 2,
        row,
    };
    assert!(!app.mouse(beside), "the gutter is no target of the line");
    let after = frame(&mut app);
    assert_eq!(group(&after, width).1, "▾ Spawned (6)", "nothing folded");
    let last = Mouse {
        kind: MouseKind::Click,
        column: width - 3,
        row,
    };
    assert!(app.mouse(last), "the last cell of the line folds");
    let after = frame(&mut app);
    assert!(group(&after, width).1.starts_with("▸ Spawned (6)"));
}

/// A stopped session draws `Not running: no seat facts.` and no group, so `Z`
/// has nothing to fold there: the flag the next running session reads stays
/// open, though stale seat facts and a roster of the stopped one still stand.
#[test]
fn a_stopped_session_with_stale_seat_facts_leaves_the_fold_flag_alone() {
    let mut app = fixture(&six(), 44);
    app.world.sessions[0].status = Status::Stopped;
    let stopped = frame(&mut app);
    assert!(
        (0..stopped.area.height).all(|y| !line(&stopped, y, 44).contains("Spawned (")),
        "a stopped session draws no group"
    );
    key(&mut app, b"Z");
    app.world.sessions[0].status = Status::Running;
    let running = frame(&mut app);
    assert_eq!(
        group(&running, 44).1,
        "▾ Spawned (6)",
        "the flag stayed open"
    );
}

/// A roster read from an incarnation that is not the one the fleet names now
/// (the home lane is kept until its next read) must not group the seats of
/// the new one: a seat the old roster held as `spawned.0` is a fixed seat in
/// the new one, and until the new roster is read nothing is grouped.
#[test]
fn a_roster_read_from_a_replaced_incarnation_groups_nothing() {
    const NEW: &str = "0199c0de-5678-4890-abcd-ef0123456789";
    let seats = [
        ("main", "lead", "working"),
        ("worker.0", "colead", "working"),
        ("spawned.0", "scout", "working"),
        ("spawned.1", "builder", "working"),
    ];
    let mut app = fixture(&seats, 44);
    let _ = frame(&mut app);
    app.ids.insert("api".into(), NEW.into());
    let stale = frame(&mut app);
    assert!(
        (0..stale.area.height).all(|y| !line(&stale, y, 44).contains("Spawned (")),
        "the old incarnation's roster draws no group"
    );
    key(&mut app, b"Z");
    let renamed = [
        ("main", "lead"),
        ("worker.0", "colead"),
        ("worker.1", "scout"),
        ("spawned.0", "builder"),
    ];
    app.answer(Answer::View(ViewRead {
        name: "api".into(),
        id: NEW.into(),
        seq: 2,
        lane: Lane::default(),
        needs: None,
        roster: Some(
            renamed
                .iter()
                .map(|(slot, name)| SeatRef {
                    slot: (*slot).into(),
                    name: (*name).into(),
                })
                .collect(),
        ),
    }));
    let fresh = frame(&mut app);
    let (row, text) = group(&fresh, 44);
    assert_eq!(text, "▾ Spawned (1)", "the Z above had nothing to fold");
    let scout = (0..fresh.area.height)
        .find(|y| line(&fresh, *y, 44).contains("scout"))
        .expect("scout is drawn");
    assert!(scout < row, "scout is a fixed seat of the new incarnation");
}

/// A seat that waits on a typed target reads `waiting on <target>` in place of
/// the generic wait word, in the fixed seats and in the spawned group alike,
/// and only its own seat does: a second `waiting-agent` seat that declared no
/// target keeps the generic word, whichever seat the digest lists first.
#[test]
fn a_typed_wait_names_its_target_on_its_own_seat_only() {
    let seats = [
        ("main", "alpha", "waiting-agent"),
        ("worker.0", "bravo", "waiting-agent"),
        ("spawned.0", "scout", "waiting-agent"),
    ];
    let mut app = fixture(&seats, 80);
    app.world.sessions[0].agents = vec![AgentEntry {
        name: "alpha".into(),
        declaration: Some(Declaration::Made {
            reason: String::new(),
            at: None,
            on: crate::wait_on::Target::parse("ghost", "api"),
            overdue: false,
            missing: false,
            moved: None,
        }),
        ..AgentEntry::default()
    }];
    let shown = frame(&mut app);
    let row = |name: &str| {
        (0..shown.area.height)
            .map(|y| line(&shown, y, 80))
            .find(|text| text.contains(name))
            .unwrap_or_else(|| panic!("{name} is drawn"))
    };
    assert!(
        row("alpha").contains("waiting on ghost"),
        "{}",
        row("alpha")
    );
    for other in ["bravo", "scout"] {
        assert!(!row(other).contains("ghost"), "{}", row(other));
        assert!(row(other).contains("waiting on an agent"), "{}", row(other));
    }
}

/// A seat that declared its work keeps the declaration row under its name, in
/// the fixed seats and in the spawned group, and a seat without one has none.
/// The row says the seat's own declared reason, which each test seat spells
/// out, and its age.
#[test]
fn a_declared_seat_keeps_its_declaration_row_under_its_name() {
    let seats = [
        ("main", "alpha", "working"),
        ("worker.0", "bravo", "working"),
        ("spawned.0", "scout", "working"),
        ("spawned.1", "tango", "working"),
    ];
    let mut app = fixture(&seats, 80);
    let declared = |name: &str| AgentEntry {
        name: name.into(),
        declaration: Some(Declaration::Made {
            reason: "reading the plan".into(),
            at: None,
            on: None,
            overdue: false,
            missing: false,
            moved: None,
        }),
        ..AgentEntry::default()
    };
    app.world.sessions[0].agents = vec![declared("alpha"), declared("scout")];
    let shown = frame(&mut app);
    let rows: Vec<String> = (0..shown.area.height)
        .map(|y| line(&shown, y, 80))
        .collect();
    let below = |name: &str| {
        let at = rows
            .iter()
            .position(|text| text.contains(name))
            .unwrap_or_else(|| panic!("{name} is drawn: {rows:?}"));
        rows[at + 1].clone()
    };
    for name in ["alpha", "scout"] {
        assert!(
            below(name).starts_with("reading the plan"),
            "under {name}: {}",
            below(name)
        );
    }
    for name in ["bravo", "tango"] {
        assert!(
            !below(name).contains("reading the plan"),
            "{name} declared nothing: {}",
            below(name)
        );
    }
}
