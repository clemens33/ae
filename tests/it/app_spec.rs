//! sideapp stage-B spec, part 2: pure-interface pins against plan §8.
//!
//! Oracles: the brief rulings, the o3-side-q frames, consult-sideapp.md §§1-7,
//! the Q1-Q5 answers, and the existing owners (`theme`, `brief::age`,
//! `topic_lines`, `needs::Section`, `PickerAgent::mark`). Oracles never come
//! from the new code.
//!
//! Compiles against the driver's interface-stub commit (post chat-needs-you
//! rebase): `ae::app::{fleet,overview,model,draw}`, the new `input::Key`
//! variants, `Keys::app`/`Keys::idle`, `ESC_IDLE`. RED lands at assertion
//! sites against neutral stub returns.
//!
//! Deliberate exclusions (driver unit-pins the decided behavior): needs rows
//! with `Verdict::Unknown` (trust gaps, never needy); the exact `as_str` word
//! a blank-detail Question falls back to (pinned here as non-blank only).
//! New types are never assumed `PartialEq`: `matches!` and field asserts only.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures build pure values only"
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use ae::app::draw::{Composer, Screen, draw};
use ae::app::fleet::{Counts, Facts, Fleet, Line2, Row, rows};
use ae::app::model::{Act, Key as AppKey, Model, browse_keys};
use ae::app::overview::{self, Open};
use ae::attention::Reason;
use ae::console::input::{ESC_IDLE, Key, Keys};
use ae::console::lane::{Item, Kind, Lane};
use ae::console::needs::{Row as NeedRow, SeatRef, Section, Source, Verdict};
use ae::digest::{AgentEntry, SessionEntry, Status};
use ae::listing::World;
use ae::theme::{FleetOrder, Look, Mark, Palette};
use ae::time::Timestamp;
use ae::tmux::PickerAgent;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Modifier};

const NOW: i64 = 1_760_000_000; // 2025-10-09T08:53:20Z; every age is relative.

fn now() -> Timestamp {
    Timestamp::from_epoch(NOW)
}

fn hex(hex: &str) -> Color {
    let bytes = hex.strip_prefix('#').expect("a #rrggbb token");
    let channel = |at: usize| u8::from_str_radix(&bytes[at..at + 2], 16).expect("hex pair");
    Color::Rgb(channel(0), channel(2), channel(4))
}

fn darcula() -> Look {
    Look::read("on", "darcula", "on", "on")
}

fn theme_off() -> Look {
    Look::read("on", "darcula", "off", "on")
}

// ---------------------------------------------------------------------------
// fleet::rows
// ---------------------------------------------------------------------------

fn picker(name: &str, state: &str) -> PickerAgent {
    PickerAgent {
        name: name.to_owned(),
        profile: "sonnet55x".to_owned(),
        state: state.to_owned(),
        pane: "%7".to_owned(),
        client: "claude".to_owned(),
        model: "Sonnet 5.5".to_owned(),
        effort: "xhigh".to_owned(),
        drift: false,
    }
}

fn need(slot: &str, seat: &str, reason: Reason, detail: &str, age_secs: i64) -> NeedRow {
    NeedRow {
        seat: SeatRef {
            slot: slot.to_owned(),
            name: seat.to_owned(),
        },
        lead_pair: true,
        verdict: Verdict::Reason(reason),
        source: Source::Declaration,
        stale: None,
        since_micros: Some((NOW - age_secs) * 1_000_000),
        detail: detail.to_owned(),
        record: None,
    }
}

/// The four maps `world` hands back, one alias so the signature stays read.
type WorldParts = (
    World,
    BTreeMap<String, Facts>,
    BTreeMap<String, i64>,
    BTreeMap<String, Section>,
);

/// The shared world: orchestrator + docs + api (home) + infra (needy) +
/// deadseat + old (stopped). `offsite` and `bare` ride worlds of their own.
#[allow(clippy::too_many_lines, reason = "one shared six-session fixture")]
fn world() -> WorldParts {
    let mut orchestrator = SessionEntry::new("orchestrator", Status::Running);
    orchestrator.last_active_epoch = Some(NOW - 30);
    let mut docs = SessionEntry::new("docs", Status::Running);
    docs.goal = Some("Rewrite the quickstart guide".to_owned());
    docs.goal_set_epoch = Some(NOW - 240);
    docs.branch = Some("main".to_owned());
    docs.last_active_epoch = Some(NOW - 240);
    let mut api = SessionEntry::new("api", Status::Running);
    api.goal = Some("Ship the v2 auth endpoints".to_owned());
    api.goal_set_epoch = Some(NOW - 7200);
    api.branch = Some("feat/auth-v2".to_owned());
    api.last_active_epoch = Some(NOW - 60);
    let mut infra = SessionEntry::new("infra", Status::Running);
    infra.goal = Some("Move CI to arm runners".to_owned());
    infra.goal_set_epoch = Some(NOW - 10_800);
    infra.branch = Some("ci/arm".to_owned());
    infra.last_active_epoch = Some(NOW - 60);
    infra.attention = Some(Reason::WaitingUser);
    let mut deadseat = SessionEntry::new("deadseat", Status::Running);
    deadseat.goal = Some("Keep the lights on".to_owned());
    deadseat.goal_set_epoch = Some(NOW - 60);
    deadseat.last_active_epoch = Some(NOW - 60);
    deadseat.attention = Some(Reason::Dead);
    let mut old = SessionEntry::new("old", Status::Stopped);
    old.agents = vec![
        AgentEntry {
            name: "lead".to_owned(),
            ..Default::default()
        },
        AgentEntry {
            name: "colead".to_owned(),
            ..Default::default()
        },
    ];
    let world = World::new(now(), vec![orchestrator, docs, api, infra, deadseat, old]);
    let facts: BTreeMap<String, Facts> = BTreeMap::from([
        (
            "orchestrator".to_owned(),
            Facts::Seats {
                id: "$0".to_owned(),
                agents: vec![picker("lead", "working")],
            },
        ),
        (
            "docs".to_owned(),
            Facts::Seats {
                id: "$2".to_owned(),
                agents: vec![
                    picker("lead", "working"),
                    picker("colead", "blocked"),
                    picker("scribe", "done"),
                    picker("tester", "idle"),
                ],
            },
        ),
        (
            "api".to_owned(),
            Facts::Seats {
                id: "$1".to_owned(),
                agents: vec![
                    picker("lead", "working"),
                    picker("builder", "working"),
                    picker("reviewer", "working"),
                    picker("colead", "waiting-agent"),
                    picker("tester", "done"),
                    picker("scribe", "done"),
                ],
            },
        ),
        (
            "infra".to_owned(),
            Facts::Seats {
                id: "$3".to_owned(),
                agents: vec![
                    picker("lead", "waiting-user"),
                    picker("builder", "working"),
                    picker("tester", "working"),
                    picker("colead", "waiting-agent"),
                ],
            },
        ),
        (
            "deadseat".to_owned(),
            Facts::Seats {
                id: "$4".to_owned(),
                agents: vec![picker("builder", "dead")],
            },
        ),
    ]);
    let last_live: BTreeMap<String, i64> = BTreeMap::from([("old".to_owned(), NOW - 172_800)]);
    let needs: BTreeMap<String, Section> = BTreeMap::from([
        (
            "infra".to_owned(),
            Section {
                rows: vec![need(
                    "main",
                    "lead",
                    Reason::WaitingUser,
                    "Approve runner cost: +$40/month for arm CI?",
                    240,
                )],
            },
        ),
        (
            "deadseat".to_owned(),
            Section {
                rows: vec![need("spawned.1", "builder", Reason::Dead, "", 60)],
            },
        ),
    ]);
    (world, facts, last_live, needs)
}

fn fleet() -> Fleet {
    let (world, facts, last_live, needs) = world();
    let order = FleetOrder::from_validated(vec!["docs".to_owned()]);
    rows(
        &world,
        &facts,
        &last_live,
        &order,
        &needs,
        Some("api"),
        now(),
    )
}

/// Orchestrator pinned first, then the human's order, then creation, then
/// stopped; indexes are the sidebar's own 1-based run; home flagged once.
#[test]
fn app_fleet_rows_order_pin_orchestrator_order_creation_stopped() {
    let fleet = fleet();
    let names: Vec<&str> = fleet.rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(
        names,
        ["orchestrator", "docs", "api", "infra", "deadseat", "old"],
        "pinned, ordered, created, stopped"
    );
    let indexes: Vec<usize> = fleet.rows.iter().map(|row| row.index).collect();
    assert_eq!(indexes, [1, 2, 3, 4, 5, 6], "sidebar's own 1-based run");
    let home: Vec<&str> = fleet
        .rows
        .iter()
        .filter(|row| row.home)
        .map(|row| row.name.as_str())
        .collect();
    assert_eq!(home, ["api"], "exactly the home row");
    assert_eq!(fleet.home.as_deref(), Some("api"));
}

/// Marks come from the world rollup, else from live seats; needy comes only
/// from the needs Section (Q4), and a Dead row counts as needy.
#[test]
fn app_fleet_rows_mark_and_needy() {
    let fleet = fleet();
    let by_name = |name: &str| {
        fleet
            .rows
            .iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("row {name}"))
    };
    assert_eq!(by_name("infra").mark, Mark::NeedsYou);
    assert!(by_name("infra").needy, "a Reason row is needy");
    assert_eq!(by_name("deadseat").mark, Mark::Dead);
    assert!(by_name("deadseat").needy, "a Dead row is needy too");
    assert_eq!(by_name("api").mark, Mark::Working);
    assert!(!by_name("api").needy, "no Section, no need");
    assert_eq!(by_name("docs").mark, Mark::Working);
    assert!(!by_name("docs").needy, "counts never make needy");
    assert_eq!(by_name("orchestrator").mark, Mark::Working);
}

/// Counts tally picker marks in `BY_URGENCY` order, non-zero only; blocked
/// keeps ⚠ with the letter; unknown facts draw `?`; stopped draws Stopped.
#[test]
fn app_fleet_rows_counts() {
    use Mark::{Done, Idle, NeedsYou, WaitingAgent, Working};
    let fleet = fleet();
    let by_name = |name: &str| {
        fleet
            .rows
            .iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("row {name}"))
    };
    assert!(matches!(
        by_name("api").counts,
        Counts::Marks(ref tallied)
            if *tallied == [(Working, false, 3), (WaitingAgent, false, 1), (Done, false, 2)]
    ));
    assert!(matches!(
        by_name("infra").counts,
        Counts::Marks(ref tallied)
            if *tallied == [(NeedsYou, false, 1), (Working, false, 2), (WaitingAgent, false, 1)]
    ));
    assert!(matches!(
        by_name("docs").counts,
        Counts::Marks(ref tallied)
            if *tallied
                == [
                    (NeedsYou, true, 1),
                    (Working, false, 1),
                    (Done, false, 1),
                    (Idle, false, 1)
                ]
    ));
    assert!(matches!(by_name("old").counts, Counts::Stopped));
    // Another server, and a silent watchdog, both read as unknown.
    for facts in [Facts::OtherServer, Facts::NotPublished] {
        let mut entry = SessionEntry::new("offsite", Status::Running);
        entry.last_active_epoch = Some(NOW - 60);
        let world = World::new(now(), vec![entry]);
        let fleet = rows(
            &world,
            &BTreeMap::from([("offsite".to_owned(), facts)]),
            &BTreeMap::new(),
            &FleetOrder::EMPTY,
            &BTreeMap::new(),
            None,
            now(),
        );
        assert_eq!(fleet.rows.len(), 1);
        assert!(matches!(fleet.rows[0].counts, Counts::Unknown));
        assert_eq!(
            fleet.rows[0].mark,
            Mark::Stale,
            "a fact ae could not establish reads stale"
        );
        assert!(!fleet.rows[0].needy);
    }
}

/// Line 2 precedence: stopped, then Question, `SeatDown`, Goal, `NoGoal` (Q4: a
/// Dead row never feeds the Question; blank detail falls back to a word).
#[test]
fn app_fleet_rows_line_two() {
    let fleet = fleet();
    let by_name = |name: &str| {
        fleet
            .rows
            .iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("row {name}"))
    };
    assert!(matches!(
        by_name("infra").line2,
        Line2::Question { ref text, age_secs: Some(240) }
            if text == "Approve runner cost: +$40/month for arm CI?"
    ));
    assert!(
        matches!(by_name("deadseat").line2, Line2::SeatDown),
        "Dead feeds SeatDown, never the Question"
    );
    assert!(matches!(
        by_name("api").line2,
        Line2::Goal { ref text, age_secs: Some(7200) }
            if text == "Ship the v2 auth endpoints"
    ));
    assert!(matches!(
        by_name("old").line2,
        Line2::NotRunning {
            seats: 2,
            stopped_secs: Some(172_800)
        }
    ));
    let mut entry = SessionEntry::new("bare", Status::Running);
    entry.last_active_epoch = Some(NOW - 60);
    let world = World::new(now(), vec![entry]);
    let fleet = rows(
        &world,
        &BTreeMap::from([(
            "bare".to_owned(),
            Facts::Seats {
                id: "$5".to_owned(),
                agents: vec![picker("lead", "idle")],
            },
        )]),
        &BTreeMap::new(),
        &FleetOrder::EMPTY,
        &BTreeMap::new(),
        None,
        now(),
    );
    assert!(matches!(fleet.rows[0].line2, Line2::NoGoal));
    assert_eq!(fleet.rows[0].mark, Mark::Idle);
    // A blank detail still asks in words.
    let world = World::new(now(), vec![SessionEntry::new("vague", Status::Running)]);
    let fleet = rows(
        &world,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &FleetOrder::EMPTY,
        &BTreeMap::from([(
            "vague".to_owned(),
            Section {
                rows: vec![need("main", "lead", Reason::WaitingUser, "  ", 60)],
            },
        )]),
        None,
        now(),
    );
    assert!(
        matches!(
            fleet.rows[0].line2,
            Line2::Question { ref text, age_secs: Some(60) } if !text.trim().is_empty()
        ),
        "blank detail falls back to the reason word"
    );
}

/// Stopped sessions sort dated-first by epoch desc, undated after, ties by
/// name — the picker's own rule.
#[test]
fn app_fleet_rows_stopped_order() {
    let world = World::new(
        now(),
        vec![
            SessionEntry::new("undated", Status::Stopped),
            SessionEntry::new("older", Status::Stopped),
            SessionEntry::new("newer", Status::Stopped),
            SessionEntry::new("tied-b", Status::Stopped),
            SessionEntry::new("tied-a", Status::Stopped),
        ],
    );
    let last_live: BTreeMap<String, i64> = BTreeMap::from([
        ("older".to_owned(), NOW - 200_000),
        ("newer".to_owned(), NOW - 1_000),
        ("tied-a".to_owned(), NOW - 50_000),
        ("tied-b".to_owned(), NOW - 50_000),
    ]);
    let fleet = rows(
        &world,
        &BTreeMap::new(),
        &last_live,
        &FleetOrder::EMPTY,
        &BTreeMap::new(),
        None,
        now(),
    );
    let names: Vec<&str> = fleet.rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(
        names,
        ["newer", "tied-a", "tied-b", "older", "undated"],
        "dated desc, ties by name, undated last"
    );
    assert!(
        matches!(
            fleet.rows[4].line2,
            Line2::NotRunning {
                stopped_secs: None,
                ..
            }
        ),
        "no readable moment stays unreadable"
    );
}

// ---------------------------------------------------------------------------
// fleet::attention
// ---------------------------------------------------------------------------

fn calm(name: &str, index: usize) -> Row {
    Row {
        name: name.to_owned(),
        index,
        mark: Mark::Working,
        needy: false,
        counts: Counts::Marks(vec![(Mark::Working, false, 1)]),
        line2: Line2::NoGoal,
        home: false,
    }
}

fn needy(name: &str, index: usize, mark: Mark) -> Row {
    Row {
        name: name.to_owned(),
        index,
        mark,
        needy: true,
        counts: Counts::Marks(vec![(mark, false, 1)]),
        line2: Line2::NoGoal,
        home: false,
    }
}

// ---------------------------------------------------------------------------
// model
// ---------------------------------------------------------------------------

fn model_fleet() -> Fleet {
    Fleet {
        rows: vec![
            Row {
                name: "api".to_owned(),
                index: 1,
                mark: Mark::Working,
                needy: false,
                counts: Counts::Marks(vec![(Mark::Working, false, 1)]),
                line2: Line2::NoGoal,
                home: true,
            },
            calm("docs", 2),
            needy("infra", 3, Mark::NeedsYou),
            needy("ops", 4, Mark::Dead),
        ],
        home: Some("api".to_owned()),
    }
}

/// Selection starts at home, else the first row, else none.
#[test]
fn app_model_selects_home_first_row_or_none() {
    let fleet = model_fleet();
    assert_eq!(Model::new(&fleet).selected(), Some("api"));
    let homeless = Fleet {
        rows: vec![calm("docs", 1)],
        home: None,
    };
    assert_eq!(Model::new(&homeless).selected(), Some("docs"));
    let empty = Fleet {
        rows: vec![],
        home: None,
    };
    assert_eq!(Model::new(&empty).selected(), None);
}

/// Digits select by sidebar index; arrows move clamped with no wrap; a
/// selection that changes nothing answers None.
#[test]
fn app_model_digit_and_arrows() {
    let fleet = model_fleet();
    let mut model = Model::new(&fleet);
    assert!(matches!(
        model.key(AppKey::Digit(3), &fleet, true),
        Act::Select(ref name) if name == "infra"
    ));
    assert_eq!(model.selected(), Some("infra"));
    assert!(
        matches!(model.key(AppKey::Digit(3), &fleet, true), Act::None),
        "re-selecting answers nothing"
    );
    assert!(
        matches!(model.key(AppKey::Digit(9), &fleet, true), Act::None),
        "no row answers nothing"
    );
    assert!(matches!(
        model.key(AppKey::Down, &fleet, true),
        Act::Select(ref name) if name == "ops"
    ));
    assert!(
        matches!(model.key(AppKey::Down, &fleet, true), Act::None),
        "the bottom clamps"
    );
    assert!(matches!(
        model.key(AppKey::Up, &fleet, true),
        Act::Select(ref name) if name == "infra"
    ));
    let _ = model.key(AppKey::Digit(1), &fleet, true);
    assert!(
        matches!(model.key(AppKey::Up, &fleet, true), Act::None),
        "the top clamps"
    );
}

/// `!` walks the needy rows in fleet order and wraps; with no needy row it
/// answers nothing.
#[test]
fn app_model_next_need_walks_and_wraps() {
    let fleet = model_fleet();
    let mut model = Model::new(&fleet);
    assert!(matches!(
        model.key(AppKey::NextNeed, &fleet, true),
        Act::Select(ref name) if name == "infra"
    ));
    assert!(matches!(
        model.key(AppKey::NextNeed, &fleet, true),
        Act::Select(ref name) if name == "ops"
    ));
    assert!(
        matches!(
        model.key(AppKey::NextNeed, &fleet, true), Act::Select(ref name) if name == "infra"),
        "wraps around"
    );
    let quiet = Fleet {
        rows: vec![calm("api", 1)],
        home: Some("api".to_owned()),
    };
    let mut model = Model::new(&quiet);
    assert!(matches!(
        model.key(AppKey::NextNeed, &quiet, true),
        Act::None
    ));
}

/// Tab answers nothing: the Overview tab it toggled is gone. Compose
/// opens on an eligible selected row, including without home. Esc returns home.
#[test]
fn app_model_tab_compose_esc() {
    let fleet = model_fleet();
    let mut model = Model::new(&fleet);
    assert!(
        matches!(model.key(AppKey::Tab, &fleet, true), Act::None),
        "Tab answers nothing"
    );
    assert!(
        matches!(model.key(AppKey::Compose, &fleet, true), Act::Compose),
        "eligible selected home composes"
    );
    let _ = model.key(AppKey::Digit(3), &fleet, true);
    assert!(
        matches!(model.key(AppKey::Compose, &fleet, true), Act::Compose),
        "eligible selected non-home composes"
    );
    assert!(
        matches!(
        model.key(AppKey::Esc, &fleet, true), Act::Select(ref name) if name == "api"),
        "Esc returns home"
    );
    assert!(
        matches!(model.key(AppKey::Esc, &fleet, true), Act::None),
        "Esc on home answers nothing"
    );
    assert!(
        matches!(model.key(AppKey::Compose, &fleet, false), Act::None),
        "ineligible selection composes nothing"
    );
    let homeless = Fleet {
        rows: vec![calm("docs", 1)],
        home: None,
    };
    let mut model = Model::new(&homeless);
    assert!(
        matches!(model.key(AppKey::Compose, &homeless, true), Act::Compose),
        "eligible selected session needs no home"
    );
    assert!(
        matches!(model.key(AppKey::Esc, &homeless, true), Act::None),
        "no home answers nothing"
    );
}

/// PgUp/PgDn scroll the chat; Quit quits.
#[test]
fn app_model_page_and_quit() {
    let fleet = model_fleet();
    let mut model = Model::new(&fleet);
    assert!(matches!(
        model.key(AppKey::PageUp, &fleet, true),
        Act::Redraw
    ));
    assert!(matches!(
        model.key(AppKey::PageDown, &fleet, true),
        Act::Redraw
    ));
    assert!(matches!(model.key(AppKey::Quit, &fleet, true), Act::Quit));
}

// ---------------------------------------------------------------------------
// model::browse_keys + Keys::app + Keys::idle
// ---------------------------------------------------------------------------

/// The BROWSE decode table, every row of it: digits, `!`, `j`/`k`, `i` and
/// Enter, `q` and ^C, the editing keys; a paste, editing keys and every other
/// byte are swallowed.
fn app_key_name(key: AppKey) -> String {
    match key {
        AppKey::Digit(n) => format!("Digit({n})"),
        AppKey::Up => "Up".to_owned(),
        AppKey::Down => "Down".to_owned(),
        AppKey::NextNeed => "NextNeed".to_owned(),
        AppKey::Tab => "Tab".to_owned(),
        AppKey::Compose => "Compose".to_owned(),
        AppKey::Esc => "Esc".to_owned(),
        AppKey::PageUp => "PageUp".to_owned(),
        AppKey::PageDown => "PageDown".to_owned(),
        AppKey::Quit => "Quit".to_owned(),
        AppKey::Open => "Open".to_owned(),
        AppKey::SeatNext => "SeatNext".to_owned(),
        AppKey::SeatPrev => "SeatPrev".to_owned(),
        AppKey::TurnOlder => "TurnOlder".to_owned(),
        AppKey::TurnNewer => "TurnNewer".to_owned(),
        AppKey::Copy => "Copy".to_owned(),
        AppKey::Peek => "Peek".to_owned(),
    }
}

#[test]
fn app_browse_keys_decode_table() {
    let text = |byte: u8| Key::Text(vec![byte]);
    for (byte, want) in [
        (b'1', "Digit(1)"),
        (b'5', "Digit(5)"),
        (b'9', "Digit(9)"),
        (b'!', "NextNeed"),
        (b'j', "Down"),
        (b'k', "Up"),
        (b'i', "Compose"),
        (b'q', "Quit"),
        (b'[', "TurnOlder"),
        (b']', "TurnNewer"),
        (b'y', "Copy"),
    ] {
        let got: Vec<String> = browse_keys(&text(byte))
            .iter()
            .copied()
            .map(app_key_name)
            .collect();
        assert_eq!(got, [want], "byte {byte} decodes");
    }
    for (key, want) in [
        (Key::Enter, "Compose"),
        (Key::Up, "Up"),
        (Key::Down, "Down"),
        (Key::PageUp, "PageUp"),
        (Key::PageDown, "PageDown"),
        (Key::Tab, "Tab"),
        (Key::Escape, "Esc"),
        (Key::Interrupt, "Quit"),
    ] {
        let got: Vec<String> = browse_keys(&key)
            .iter()
            .copied()
            .map(app_key_name)
            .collect();
        assert_eq!(got, [want], "key {key:?} decodes");
    }
    for key in [
        Key::Pasted(vec![b'q', b'!', b'1']),
        text(b'x'),
        text(b' '),
        Key::Backspace,
        Key::Delete,
        Key::Left,
        Key::Right,
        Key::Home,
        Key::End,
        Key::ClearLine,
    ] {
        assert!(
            browse_keys(&key).is_empty(),
            "swallowed whole, never a nav key: {key:?}"
        );
    }
}

/// `Keys::app()` decodes the nav keys the chat consumes or never sees: arrows
/// and pages, Tab, ^C as Interrupt, and a bracketed paste as one Pasted.
#[test]
fn app_keys_decode_nav_and_paste() {
    let stamp = Instant::now();
    let feed = |keys: &mut Keys, bytes: &[u8]| {
        keys.feed(bytes, stamp)
            .into_iter()
            .map(|(key, _)| key)
            .collect::<Vec<_>>()
    };
    let mut app = Keys::app();
    assert_eq!(feed(&mut app, b"\x1b[A"), [Key::Up]);
    assert_eq!(feed(&mut app, b"\x1b[B"), [Key::Down]);
    assert_eq!(feed(&mut app, b"\x1b[5~"), [Key::PageUp]);
    assert_eq!(feed(&mut app, b"\x1b[6~"), [Key::PageDown]);
    assert_eq!(feed(&mut app, b"\x09"), [Key::Tab]);
    assert_eq!(feed(&mut app, b"\x03"), [Key::Interrupt]);
    assert_eq!(feed(&mut app, b"\r"), [Key::Enter]);
    assert_eq!(
        feed(&mut app, b"\x1b[200~q!1\x1b[201~"),
        [Key::Pasted(b"q!1".to_vec())],
        "a paste is one key, never nav keys"
    );
    // The chat's decoder stays byte-identical: no nav key escapes it.
    let mut chat = Keys::default();
    for bytes in [b"\x1b[A".as_slice(), b"\x09".as_slice(), b"\x03".as_slice()] {
        let keys = feed(&mut chat, bytes);
        assert!(
            !keys.iter().any(|key| matches!(
                key,
                Key::Up
                    | Key::Down
                    | Key::PageUp
                    | Key::PageDown
                    | Key::Tab
                    | Key::Escape
                    | Key::Interrupt
                    | Key::Pasted(_)
            )),
            "default() decodes no app key: {bytes:?} -> {keys:?}"
        );
    }
    assert_eq!(feed(&mut Keys::default(), b"\x09"), [Key::Text(vec![9])]);
}

/// A lone ESC becomes Escape only after the idle bound, in app mode alone: a
/// fast `[A` still completes Up, a stalled `ESC [` is dropped, a paste is
/// untouched, and a second ESC flushes the first.
#[test]
fn app_keys_idle_expires_lone_esc() {
    assert_eq!(ESC_IDLE, Duration::from_millis(50));
    let t0 = Instant::now();
    let mut app = Keys::app();
    assert!(app.feed(b"\x1b", t0).is_empty(), "ESC waits");
    assert!(app.idle(t0).is_empty(), "fresh, kept");
    let under = (t0 + ESC_IDLE)
        .checked_sub(Duration::from_millis(1))
        .expect("one millisecond under the bound");
    assert!(app.idle(under).is_empty(), "under the bound, kept");
    assert_eq!(
        app.idle(t0 + ESC_IDLE),
        [(Key::Escape, t0)],
        "past the bound, Escape"
    );
    assert_eq!(
        app.feed(b"\x1b[A", t0 + ESC_IDLE),
        [(Key::Up, t0 + ESC_IDLE)],
        "a later sequence starts fresh"
    );
    // A fast continuation still wins over the bound.
    let mut app = Keys::app();
    assert!(app.feed(b"\x1b", t0).is_empty());
    assert_eq!(app.feed(b"[A", t0), [(Key::Up, t0)]);
    // A stalled incomplete sequence is dropped, never a key.
    let mut app = Keys::app();
    assert!(app.feed(b"\x1b[", t0).is_empty());
    assert!(app.idle(t0 + ESC_IDLE).is_empty(), "dropped, no key");
    // A second ESC flushes the first and starts over.
    let mut app = Keys::app();
    assert!(app.feed(b"\x1b", t0).is_empty());
    assert_eq!(app.feed(b"\x1b", t0), [(Key::Escape, t0)]);
    assert_eq!(app.idle(t0 + ESC_IDLE), [(Key::Escape, t0)]);
    // Inside a paste nothing expires.
    let mut app = Keys::app();
    assert!(app.feed(b"\x1b[200~", t0).is_empty());
    assert!(app.idle(t0 + ESC_IDLE).is_empty(), "paste stays literal");
    assert_eq!(
        app.feed(b"hi\x1b[201~", t0),
        [(Key::Pasted(b"hi".to_vec()), t0)]
    );
    // The chat never idles: byte-identical.
    let mut chat = Keys::default();
    assert!(chat.feed(b"\x1b", t0).is_empty());
    assert!(chat.idle(t0 + ESC_IDLE).is_empty());
}

// ---------------------------------------------------------------------------
// overview::of
// ---------------------------------------------------------------------------

/// Open from every Section row in order, the dir from the record under the
/// viewer's home.
#[test]
fn app_overview_folds_open_and_dir() {
    let mut entry = SessionEntry::new("api", Status::Running);
    entry.work_dir = Some("/home/me/projects/ae".to_owned());
    let needs = Section {
        rows: vec![
            need("main", "lead", Reason::WaitingUser, "Approve X?", 240),
            need("worker.0", "colead", Reason::Blocked, "registry down", 3600),
        ],
    };
    let overview = overview::of(&entry, Some(&needs), Some(Path::new("/home/me")), now());
    assert_eq!(overview.open.len(), 2, "every Section row opens");
    assert_eq!(overview.open[0].seat.as_str(), "lead", "the seat NAME asks");
    assert_eq!(overview.open[0].text.as_str(), "Approve X?");
    assert_eq!(overview.open[0].age_secs, Some(240));
    assert_eq!(overview.open[1].seat.as_str(), "colead");
    assert_eq!(overview.open[1].text.as_str(), "registry down");
    assert_eq!(overview.open[1].age_secs, Some(3600));
    assert_eq!(overview.dir.as_deref(), Some("~/projects/ae"));
}

/// No needs and no recorded dir is the empty Overview; a dir outside the
/// home stays verbatim and an empty record names none.
#[test]
fn app_overview_is_empty_without_needs_or_dir() {
    let mut entry = SessionEntry::new("api", Status::Running);
    let overview = overview::of(&entry, None, None, now());
    assert!(overview.open.is_empty());
    assert_eq!(overview.dir, None);
    entry.work_dir = Some("/srv/ae".to_owned());
    let home = Some(Path::new("/home/me"));
    assert_eq!(
        overview::of(&entry, None, home, now()).dir.as_deref(),
        Some("/srv/ae")
    );
    entry.work_dir = Some(String::new());
    assert_eq!(overview::of(&entry, None, home, now()).dir, None);
}

// ---------------------------------------------------------------------------
// draw
// ---------------------------------------------------------------------------

fn cell_at(buf: &Buffer, x: u16, y: u16) -> &ratatui_core::buffer::Cell {
    let width = usize::from(buf.area.width);
    &buf.content[usize::from(y) * width + usize::from(x)]
}

fn row_text(buf: &Buffer, y: u16) -> String {
    let text: String = (0..buf.area.width)
        .map(|x| cell_at(buf, x, y).symbol())
        .collect();
    text.trim_end().to_owned()
}

fn all_text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One column of one row: `row_text` spans the full width, so a pin about a
/// column's edge or whole line reads through these, never the full row.
fn span_text(buf: &Buffer, y: u16, xs: std::ops::Range<u16>) -> String {
    xs.map(|x| cell_at(buf, x, y).symbol())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

fn side_text(buf: &Buffer, y: u16, rule: u16) -> String {
    span_text(buf, y, 0..rule)
}

fn chat_text(buf: &Buffer, y: u16, rule: u16) -> String {
    span_text(buf, y, rule + 1..buf.area.width)
}

fn draw_fleet() -> Fleet {
    Fleet {
        rows: vec![
            Row {
                name: "api".to_owned(),
                index: 1,
                mark: Mark::Working,
                needy: false,
                counts: Counts::Marks(vec![
                    (Mark::Working, false, 3),
                    (Mark::WaitingAgent, false, 1),
                    (Mark::Done, false, 2),
                ]),
                line2: Line2::Goal {
                    text: "Ship the v2 auth endpoints".to_owned(),
                    age_secs: Some(7200),
                },
                home: true,
            },
            Row {
                name: "docs".to_owned(),
                index: 2,
                mark: Mark::Working,
                needy: false,
                counts: Counts::Marks(vec![(Mark::Working, false, 1), (Mark::Done, false, 1)]),
                line2: Line2::Goal {
                    text: "Rewrite the quickstart guide".to_owned(),
                    age_secs: Some(240),
                },
                home: false,
            },
            Row {
                name: "infra".to_owned(),
                index: 3,
                mark: Mark::NeedsYou,
                needy: true,
                counts: Counts::Marks(vec![(Mark::NeedsYou, false, 1), (Mark::Working, false, 2)]),
                line2: Line2::Question {
                    text: "Approve runner cost: +$40/month?".to_owned(),
                    age_secs: Some(240),
                },
                home: false,
            },
        ],
        home: Some("api".to_owned()),
    }
}

fn draw_overview() -> ae::app::overview::Overview {
    ae::app::overview::Overview {
        open: vec![Open {
            seat: "lead".to_owned(),
            text: "Approve runner cost: +$40/month?".to_owned(),
            age_secs: Some(240),
        }],
        ..Default::default()
    }
}

fn draw_entry() -> SessionEntry {
    let mut entry = SessionEntry::new("api", Status::Running);
    entry.goal = Some("Ship the v2 auth endpoints".to_owned());
    entry.goal_set_epoch = Some(NOW - 7200);
    entry.branch = Some("feat/auth-v2".to_owned());
    entry.last_active_epoch = Some(NOW - 60);
    entry
}

fn draw_lane() -> Lane {
    Lane {
        items: vec![
            Item {
                micros: (NOW - 600) * 1_000_000,
                kind: Kind::Asked {
                    to: "lead".to_owned(),
                    id: "r-1".to_owned(),
                    uncertain: false,
                    gap: None,
                },
                body: "ship it?".to_owned(),
                record: None,
            },
            Item {
                micros: (NOW - 300) * 1_000_000,
                kind: Kind::Said {
                    who: "lead".to_owned(),
                },
                body: "hello lane".to_owned(),
                record: None,
            },
        ],
        coverage: vec![],
    }
}

fn agents6() -> Facts {
    Facts::Seats {
        id: "$1".to_owned(),
        agents: vec![
            picker("lead", "working"),
            picker("builder", "working"),
            picker("reviewer", "working"),
            picker("colead", "waiting-agent"),
            picker("tester", "done"),
            picker("scribe", "done"),
        ],
    }
}

/// The 160x43 home view: sidebar geometry, needy lighting, the Agents
/// heading and body, chat column, composer and keys row, every colour from the darcula owner.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one pinned 160x43 frame proves the whole home geometry"
)]
fn app_draw_home_160x43() {
    let palette = Palette::DARCULA;
    let fleet = draw_fleet();
    let model = Model::new(&fleet);
    let overview = draw_overview();
    let entry = draw_entry();
    let pair = ["lead".to_owned(), "colead".to_owned()];
    let agents = agents6();
    let lane = draw_lane();
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(&entry),
        pair: &pair,
        agents: Some(&agents),
        lane: &lane,
        composer: Composer::Home {
            home: "api",
            speaker: "lead",
            view: None,
            draft: "check the scopes",
        },
        look: Some(darcula()),
        zone: None,
        now: now(),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 43));
    draw(&screen, &mut buf);
    // The rule stands at x = 44 through the final hint row.
    for y in 0..43 {
        assert_eq!(cell_at(&buf, 44, y).symbol(), "│", "rule row {y}");
        assert_eq!(cell_at(&buf, 44, y).fg, hex(palette.border));
    }
    assert_eq!(cell_at(&buf, 44, 42).symbol(), "│", "hint stays in chat");
    // Fleet summary; the lone `!` key marker is gone.
    assert!(row_text(&buf, 0).contains("▣ 3 "), "counts sessions");
    assert_eq!(cell_at(&buf, 2, 0).fg, hex(palette.text));
    assert!(side_text(&buf, 2, 40).is_empty(), "attention prose removed");
    assert_eq!(cell_at(&buf, 41, 2).symbol(), " ");
    // The selected home row: title name, dim window tag, counts below.
    assert_eq!(cell_at(&buf, 2, 4).symbol(), Mark::Working.glyph(true));
    assert_eq!(cell_at(&buf, 2, 4).fg, hex(palette.working));
    assert_eq!(cell_at(&buf, 6, 4).symbol(), "a");
    assert_eq!(cell_at(&buf, 7, 4).symbol(), "p");
    assert_eq!(cell_at(&buf, 8, 4).symbol(), "i");
    assert_eq!(cell_at(&buf, 6, 4).fg, hex(palette.title));
    assert!(row_text(&buf, 4).contains("this window"));
    assert!(side_text(&buf, 5, 44).ends_with("●3 ◔1 ✓2"), "counts below");
    assert!(!row_text(&buf, 5).contains("Ship the v2 auth endpoints"));
    assert_eq!(cell_at(&buf, 2, 7).symbol(), Mark::Working.glyph(true));
    assert!(
        cell_at(&buf, 6, 7).modifier.contains(Modifier::BOLD),
        "a calm name stays bold text"
    );
    // The needy row lights its edge and its mark, nothing else.
    for y in [10, 11] {
        assert_eq!(cell_at(&buf, 0, y).symbol(), "│", "edge row {y}");
        assert_eq!(cell_at(&buf, 0, y).fg, hex(palette.needs_you));
    }
    assert_eq!(cell_at(&buf, 2, 10).symbol(), "⚠");
    assert_eq!(cell_at(&buf, 2, 10).fg, hex(palette.needs_you));
    assert!(row_text(&buf, 11).contains("Approve runner cost"));
    // The heading, rule, and the Agents body in order.
    assert!(row_text(&buf, 13).contains("Agents 6"));
    assert!(!row_text(&buf, 13).contains("Overview"));
    assert!(cell_at(&buf, 2, 13).modifier.contains(Modifier::BOLD));
    assert!(!side_text(&buf, 13, 44).ends_with("Tab"));
    assert_eq!(
        side_text(&buf, 14, 44),
        "─".repeat(44),
        "the heading rule spans the sidebar"
    );
    let body: Vec<String> = (15..29).map(|y| row_text(&buf, y)).collect();
    let at = |needle: &str| {
        body.iter()
            .position(|row| row.contains(needle))
            .unwrap_or_else(|| {
                let body_text = body.join("\n");
                panic!("body holds {needle:?}:\n{body_text}")
            })
    };
    assert!(at("lead") < at("builder"), "seats in roster order");
    assert!(
        body[at("lead") + 2].contains("Approve runner cost"),
        "the lead's reason sits under its seat rows"
    );
    assert!(!body.join("\n").contains("asked by"), "no asked-by row");
    // The chat column: header, rule, bottom-anchored turns, composer, keys.
    let head = row_text(&buf, 0);
    assert_eq!(cell_at(&buf, 47, 0).symbol(), "a", "chat starts at x = 47");
    assert_eq!(cell_at(&buf, 48, 0).symbol(), "p");
    assert_eq!(cell_at(&buf, 49, 0).symbol(), "i");
    assert_eq!(cell_at(&buf, 47, 0).fg, hex(palette.title));
    assert!(head.contains("lead + colead"));
    let age = ae::brief::age(Some(60));
    let right = format!("feat/auth-v2  ·  active {age}");
    assert!(head.ends_with(&right), "branch and activity: {head:?}");
    for x in 47..157 {
        assert_eq!(cell_at(&buf, x, 1).symbol(), "─", "head rule {x}");
    }
    let text = all_text(&buf);
    let ship = text.find("ship it?").expect("the ask shows");
    let hello = text.find("hello lane").expect("the newest shows");
    assert!(ship < hello, "turns read oldest first");
    assert!(text.contains("08:48"), "viewer clock on the turn");
    assert!(text.contains("to lead"), "the ask names its seat");
    assert!(
        chat_text(&buf, 37, 44).contains("hello lane"),
        "the newest sits just above the blank"
    );
    assert!(
        chat_text(&buf, 38, 44).trim_start().is_empty(),
        "a blank parts the turn from the composer"
    );
    for x in 47..157 {
        assert_eq!(cell_at(&buf, x, 39).symbol(), "─", "composer rule {x}");
    }
    assert_eq!(cell_at(&buf, 47, 40).symbol(), "t");
    assert_eq!(chat_text(&buf, 40, 44).trim_start(), "to api › lead");
    assert_eq!(cell_at(&buf, 47, 41).symbol(), "c");
    assert_eq!(chat_text(&buf, 41, 44).trim_start(), "check the scopes");
    assert!(
        chat_text(&buf, 42, 44)
            .trim_start()
            .starts_with("draft kept · Enter writes · ? keys")
    );
    assert!(side_text(&buf, 42, 44).trim_start().is_empty());
    assert_eq!(cell_at(&buf, 44, 42).symbol(), "│");
    let keys = chat_text(&buf, 42, 44);
    assert!(
        keys.trim_start()
            .starts_with("draft kept · Enter writes · ? keys"),
        "browse hint without a mode word: {keys:?}"
    );
    assert!(keys.contains("? keys"), "the one hint: {keys:?}");
    for gone in [
        "1-9 session",
        "! next need",
        "Tab overview / agents",
        "qq quit",
    ] {
        assert!(!keys.contains(gone), "the row names no {gone}: {keys:?}");
    }
}

/// A running non-home selection names its own target and shows its own lane.
#[test]
fn app_draw_non_home_address_names_the_selected_target() {
    let fleet = draw_fleet();
    let mut model = Model::new(&fleet);
    let _ = model.key(AppKey::Digit(3), &fleet, true);
    let overview = draw_overview();
    let mut infra = SessionEntry::new("infra", Status::Running);
    infra.branch = Some("ci/arm".to_owned());
    infra.last_active_epoch = Some(NOW - 60);
    let pair = ["lead".to_owned(), "colead".to_owned()];
    let agents = agents6();
    let lane = Lane {
        items: vec![Item {
            micros: (NOW - 60) * 1_000_000,
            kind: Kind::Said {
                who: "lead".to_owned(),
            },
            body: "infra-only line".to_owned(),
            record: None,
        }],
        coverage: vec![],
    };
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(&infra),
        pair: &pair,
        agents: Some(&agents),
        lane: &lane,
        composer: Composer::Home {
            home: "infra",
            speaker: "lead",
            view: None,
            draft: "",
        },
        look: Some(darcula()),
        zone: None,
        now: now(),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 43));
    draw(&screen, &mut buf);
    let head = row_text(&buf, 0);
    assert!(head.contains("infra"), "the target is named");
    assert_eq!(
        chat_text(&buf, 40, 44).trim_start(),
        "to infra (not home) › lead",
        "the drawn address names the non-home target"
    );
    let text = all_text(&buf);
    assert!(text.contains("Enter writes"), "selected target is eligible");
    assert!(text.contains("infra-only line"), "the foreign lane shows");
    assert!(!text.contains("hello lane"), "never the home lane");
}

/// A selected running session outside home carries the non-home address;
/// Esc still stays a no-op (pinned in the reducer test).
#[test]
fn app_draw_no_home_names_the_selected_target() {
    let fleet = Fleet {
        rows: vec![calm("docs", 1)],
        home: None,
    };
    let model = Model::new(&fleet);
    let overview = ae::app::overview::Overview::default();
    let entry = SessionEntry::new("docs", Status::Running);
    let pair = ["lead".to_owned(), "colead".to_owned()];
    let lane = Lane {
        items: vec![],
        coverage: vec![],
    };
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(&entry),
        pair: &pair,
        agents: None,
        lane: &lane,
        composer: Composer::Home {
            home: "docs",
            speaker: "lead",
            view: None,
            draft: "",
        },
        look: Some(darcula()),
        zone: None,
        now: now(),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 43));
    draw(&screen, &mut buf);
    assert!(row_text(&buf, 0).contains("docs"));
    assert!(!row_text(&buf, 0).contains("viewed from"));
    assert_eq!(
        chat_text(&buf, 40, 44).trim_start(),
        "to docs (not home) › lead"
    );
    assert!(all_text(&buf).contains("Enter writes"));
}

/// `theme = off` draws no colour anywhere: every cell is Reset, the selected
/// name turns bold + reverse, the header name bold, the edge a plain rule.
#[test]
fn app_draw_theme_off_is_plain() {
    let fleet = draw_fleet();
    let model = Model::new(&fleet);
    let overview = draw_overview();
    let entry = draw_entry();
    let pair = ["lead".to_owned(), "colead".to_owned()];
    let agents = agents6();
    let lane = draw_lane();
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(&entry),
        pair: &pair,
        agents: Some(&agents),
        lane: &lane,
        composer: Composer::Home {
            home: "api",
            speaker: "lead",
            view: None,
            draft: "",
        },
        look: Some(theme_off()),
        zone: None,
        now: now(),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 43));
    draw(&screen, &mut buf);
    for cell in &buf.content {
        assert_eq!(cell.fg, Color::Reset, "no foreground anywhere");
        assert_eq!(cell.bg, Color::Reset, "no ground anywhere");
    }
    let name = cell_at(&buf, 6, 4);
    assert!(name.modifier.contains(Modifier::BOLD));
    assert!(name.modifier.contains(Modifier::REVERSED));
    assert!(cell_at(&buf, 47, 0).modifier.contains(Modifier::BOLD));
    assert_eq!(cell_at(&buf, 0, 10).symbol(), "│");
    assert_eq!(cell_at(&buf, 2, 10).symbol(), "⚠");
}

/// The 100x28 shape: a 34-wide sidebar, one line per item,
/// and a header that drops the branch before the activity.
#[test]
fn app_draw_narrow_100x28() {
    let fleet = draw_fleet();
    let model = Model::new(&fleet);
    let overview = draw_overview();
    let entry = draw_entry();
    let pair = ["lead".to_owned(), "colead".to_owned()];
    let agents = agents6();
    let lane = draw_lane();
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(&entry),
        pair: &pair,
        agents: Some(&agents),
        lane: &lane,
        composer: Composer::Home {
            home: "api",
            speaker: "lead",
            view: None,
            draft: "",
        },
        look: Some(darcula()),
        zone: None,
        now: now(),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 100, 28));
    draw(&screen, &mut buf);
    for y in 0..28 {
        assert_eq!(cell_at(&buf, 34, y).symbol(), "│", "rule row {y}");
    }
    assert_eq!(cell_at(&buf, 34, 27).symbol(), "│", "hint stays in chat");
    assert!(
        chat_text(&buf, 27, 34)
            .trim_start()
            .starts_with("Enter writes · ? keys"),
        "hint stays in the chat on the last row"
    );
    let head = row_text(&buf, 0);
    assert!(head.contains("api"), "the session is named");
    assert!(head.contains("active"), "activity survives");
    assert!(!head.contains("feat/auth-v2"), "the branch goes first");
    let text = all_text(&buf);
    assert!(!text.contains("asked by"), "no second lines");
}

/// Below 90x20 the sidebar goes away: one dim header line over the full-width
/// chat. Below 40x8 a single line says so.
#[test]
fn app_draw_small_falls_back() {
    let palette = Palette::DARCULA;
    let fleet = draw_fleet();
    let model = Model::new(&fleet);
    let overview = draw_overview();
    let entry = draw_entry();
    let pair = ["lead".to_owned(), "colead".to_owned()];
    let agents = agents6();
    let lane = draw_lane();
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(&entry),
        pair: &pair,
        agents: Some(&agents),
        lane: &lane,
        composer: Composer::Home {
            home: "api",
            speaker: "lead",
            view: None,
            draft: "",
        },
        look: Some(darcula()),
        zone: None,
        now: now(),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 89, 30));
    draw(&screen, &mut buf);
    let head = row_text(&buf, 1);
    assert!(
        head.contains("sidebar needs 90x20"),
        "the dim line: {head:?}"
    );
    assert_eq!(cell_at(&buf, 0, 1).fg, hex(palette.dim));
    assert_ne!(cell_at(&buf, 44, 4).symbol(), "│", "no sidebar rule");
    assert!(all_text(&buf).contains("hello lane"), "the chat stays");
    let mut buf = Buffer::empty(Rect::new(0, 0, 39, 7));
    draw(&screen, &mut buf);
    assert!(
        row_text(&buf, 0).contains("ae app needs at least 40x8 (now 39x7)"),
        "one line"
    );
    for y in 1..7 {
        assert!(row_text(&buf, y).is_empty(), "row {y} stays blank");
    }
}

/// The Agents body lists seat, state and client · model · effort; a session
/// with no seat facts names its gap, and a stopped session its rest.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one pinned Agents tab plus both gap lines"
)]
fn app_draw_agents_tab_and_gaps() {
    fn show<'a>(
        fleet: &'a Fleet,
        model: &'a Model,
        overview: &'a ae::app::overview::Overview,
        selected: Option<&'a SessionEntry>,
        pair: &'a [String],
        agents: Option<&'a Facts>,
        lane: &'a Lane,
    ) -> Screen<'a> {
        Screen {
            fleet,
            model,
            overview,
            selected,
            pair,
            agents,
            lane,
            composer: Composer::Home {
                home: "api",
                speaker: "lead",
                view: None,
                draft: "",
            },
            look: Some(darcula()),
            zone: None,
            now: now(),
        }
    }
    let fleet = draw_fleet();
    let model = Model::new(&fleet);
    let overview = draw_overview();
    let entry = draw_entry();
    let pair = ["lead".to_owned(), "colead".to_owned()];
    let agents = agents6();
    let lane = draw_lane();
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 43));
    draw(
        &show(
            &fleet,
            &model,
            &overview,
            Some(&entry),
            &pair,
            Some(&agents),
            &lane,
        ),
        &mut buf,
    );
    let text = all_text(&buf);
    assert!(text.contains("builder"), "seats list");
    assert!(text.contains("claude · Sonnet 5.5 · xhigh"), "facts list");
    let state_row = (0..buf.area.height)
        .map(|y| side_text(&buf, y, 44))
        .find(|row| row.contains("builder"))
        .expect("the builder row");
    assert!(state_row.ends_with("working"), "state right: {state_row:?}");
    assert!(
        !text.contains("Enter opens that pane"),
        "phase 1 sends nothing"
    );
    for (facts, want) in [
        (Facts::OtherServer, "No seat facts: on another tmux server."),
        (
            Facts::NotPublished,
            "No seat facts: watchdog publishes none.",
        ),
    ] {
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 43));
        draw(
            &show(
                &fleet,
                &model,
                &overview,
                Some(&entry),
                &pair,
                Some(&facts),
                &lane,
            ),
            &mut buf,
        );
        assert!(all_text(&buf).contains(want), "the gap is named");
    }
    let stopped = SessionEntry::new("old", Status::Stopped);
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 43));
    draw(
        &show(
            &fleet,
            &model,
            &overview,
            Some(&stopped),
            &pair,
            Some(&Facts::NotPublished),
            &lane,
        ),
        &mut buf,
    );
    assert!(
        all_text(&buf).contains("Not running: no seat facts."),
        "rest wins over the facts variant"
    );
}

/// `!` scrolls its row into view: seven sessions at 160x30 show five, the
/// needy seventh appears only after the key.
#[test]
fn app_draw_next_need_scrolls_into_view() {
    let mut rows: Vec<Row> = (1..=6)
        .map(|index| calm(&format!("s{index}"), index))
        .collect();
    rows[0].home = true;
    rows.push(needy("s7", 7, Mark::NeedsYou));
    let fleet = Fleet {
        rows,
        home: Some("s1".to_owned()),
    };
    let mut model = Model::new(&fleet);
    let overview = ae::app::overview::Overview {
        open: vec![Open {
            seat: "lead".to_owned(),
            text: "Wake up?".to_owned(),
            age_secs: Some(60),
        }],
        ..Default::default()
    };
    let entry = SessionEntry::new("s1", Status::Running);
    let pair: Vec<String> = vec![];
    let lane = Lane {
        items: vec![],
        coverage: vec![],
    };
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(&entry),
        pair: &pair,
        agents: None,
        lane: &lane,
        composer: Composer::Home {
            home: "s1",
            speaker: "lead",
            view: None,
            draft: "",
        },
        look: Some(darcula()),
        zone: None,
        now: now(),
    };
    let list_holds =
        |buf: &Buffer, name: &str| (4..15).any(|y| side_text(buf, y, 44).contains(name));
    {
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 30));
        draw(&screen, &mut buf);
        assert!(!list_holds(&buf, "s7"), "below the fold at first");
        assert_eq!(side_text(&buf, 2, 44).trim(), "↓", "off-screen cue");
    }
    let _ = model.key(AppKey::NextNeed, &fleet, true);
    assert_eq!(model.selected(), Some("s7"));
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(&entry),
        pair: &pair,
        agents: None,
        lane: &lane,
        composer: Composer::Home {
            home: "s1",
            speaker: "lead",
            view: None,
            draft: "",
        },
        look: Some(darcula()),
        zone: None,
        now: now(),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 30));
    draw(&screen, &mut buf);
    assert!(list_holds(&buf, "s7"), "scrolled into view");
}

/// A drawn look with a viewer zone shifts the lane clock: the 08:48:20Z turn
/// reads 10:48 at +0200, and no UTC head leaks through.
#[test]
fn app_draw_zone_shifts_lane_clock() {
    let fleet = draw_fleet();
    let model = Model::new(&fleet);
    let overview = draw_overview();
    let entry = draw_entry();
    let pair = ["lead".to_owned(), "colead".to_owned()];
    let agents = agents6();
    let lane = draw_lane();
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(&entry),
        pair: &pair,
        agents: Some(&agents),
        lane: &lane,
        composer: Composer::Home {
            home: "api",
            speaker: "lead",
            view: None,
            draft: "",
        },
        look: Some(darcula()),
        zone: Some("+0200"),
        now: now(),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 43));
    draw(&screen, &mut buf);
    let text = all_text(&buf);
    assert!(text.contains("10:48"), "the viewer zone shifts the turn");
    assert!(!text.contains("08:48"), "no UTC leak");
}

/// Home selected but not the owner: the composer names the existing reason
/// and the way back, at the address row 40; the header stays the home form.
#[test]
fn app_draw_home_not_owner_is_read_only() {
    let fleet = draw_fleet();
    let model = Model::new(&fleet);
    let overview = draw_overview();
    let entry = draw_entry();
    let pair = ["lead".to_owned(), "colead".to_owned()];
    let agents = agents6();
    let lane = draw_lane();
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(&entry),
        pair: &pair,
        agents: Some(&agents),
        lane: &lane,
        composer: Composer::ReadOnly {
            why: "owned elsewhere",
        },
        look: Some(darcula()),
        zone: None,
        now: now(),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 43));
    draw(&screen, &mut buf);
    assert!(!row_text(&buf, 0).contains("viewed from"));
    assert_eq!(
        chat_text(&buf, 40, 44).trim_start(),
        "read-only · owned elsewhere"
    );
}
