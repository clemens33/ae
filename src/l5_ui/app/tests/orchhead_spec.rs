//! Frozen acceptance for orchhead: the orchestrator column of `ae app` shows
//! the work of the whole fleet in one short segment, drops its attention
//! segment, and draws no board column. Every test drives the app through its
//! input and frame, and none names an API the slice adds, so each compiles
//! before and after it.
//!
//! The counts come from the fleet listing, [`World`], whose sessions carry each
//! seat's declared state across every server `ae list` may read. They do not
//! come from the picker facts, which the app reads from its own tmux server
//! only. The fixture tells the sources and the counts apart on purpose. The
//! orchestrator is a running session that [`fleet::rows`] never lists. One
//! running session lives on another server and no picker row names it. A
//! stopped session keeps old declared states. A seat that declared `working`
//! has a dead pane. The counts of `waiting-user` seats, generic needs-you
//! seats, `waiting-agent` seats and needy sessions all differ from one another.
//! The fleet read is the production fold ([`Reader::fold`]) with a valid
//! picker row for `api`; only the needs, which the fold reads from journals, are
//! set by hand, as the other app specs set them.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "frozen acceptance owns scratch state roots and terminal buffers"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::*;
use crate::console::input::Keys;
use crate::console::needs::{Row as Need, SeatRef, Section, Source, Verdict};
use crate::digest::AgentEntry;
use crate::theme::FleetOrder;
use crate::tmux::PickerSession;

pub(super) const ORCH: &str = crate::orchestrator::ORCHESTRATOR_SESSION;
pub(super) const OID: &str = "0199c0de-2222-4890-abcd-ef0123456789";
pub(super) const MAIN_TURN: &str = "ORCHHEAD_MAIN_SENTINEL";
const API_ASK: &str = "API_ASK_SENTINEL";
const ORCH_ASK: &str = "ORCH_ASK_SENTINEL";
pub(super) const WIDE: u16 = 200;
/// A frame whose side panel (about 102 cells here, 77 at [`WIDE`]) keeps the whole
/// work segment beside the names, the activity and the zoom label. A side panel
/// is half the columns right of the sidebar (44 wide from 140 columns, rule
/// included).
const FIT: u16 = 250;
pub(super) const HEIGHT: u16 = 35;

/// What the exact fleet below adds up to: five working seats (the
/// orchestrator's three, one of `api`, one of `farm` on another server), two
/// `waiting-user` seats and five running sessions.
const WORK: &str = "fleet ● 5 working · ⚠ 2 wait you · ≡ 5 sessions";
/// The shorter steps of the segment, longest first after [`WORK`].
const STEP2: &str = "● 5 working · ⚠ 2 wait you · ≡ 5 sessions";
const STEP3: &str = "● 5 · ⚠ 2 · ≡ 5";
const STEP4: &str = "● 5 · ⚠ 2";
/// What [`Mix::Flipped`] adds up to, a later read of the same fleet.
const FLIPPED: &str = "fleet ● 0 working · ⚠ 4 wait you · ≡ 3 sessions";

/// Which fleet the fixture lists.
#[derive(Clone, Copy)]
pub(super) enum Mix {
    /// Every running session lists its seats.
    Exact,
    /// [`Mix::Exact`] plus a session whose liveness is unknown.
    Partial,
    /// [`Mix::Exact`] read while one source failed to enumerate: no session is
    /// marked unknown, and the world still counts the loss.
    Lossy,
    /// Two running sessions and no seat that works or waits.
    Quiet,
    /// The orchestrator alone runs; the home session is stopped.
    Alone,
    /// A later read of the fleet with the counts turned around.
    Flipped,
}

/// One seat as the listing declares it: name, declared state, pane alive.
type Declared = (&'static str, &'static str, bool);

const ALIVE: bool = true;

fn listed(mix: Mix) -> Vec<(&'static str, Status, Vec<Declared>)> {
    let orch = |states: &[Declared]| (ORCH, Status::Running, states.to_vec());
    let exact = vec![
        orch(&[
            ("olead", "working", ALIVE),
            ("oworker", "working", ALIVE),
            ("oextra", "working", ALIVE),
        ]),
        (
            "api",
            Status::Running,
            vec![
                ("lead", "working", ALIVE),
                ("colead", "waiting-user", ALIVE),
                ("w1", "blocked", ALIVE),
            ],
        ),
        (
            "beta",
            Status::Running,
            vec![
                ("lead", "waiting-user", ALIVE),
                ("w1", "blocked", ALIVE),
                ("w2", "waiting-agent", ALIVE),
                ("w3", "waiting-agent", ALIVE),
            ],
        ),
        // The one seat that declared `working` lost its pane: it works at nothing.
        (
            "gamma",
            Status::Running,
            vec![
                ("lead", "idle", ALIVE),
                ("w1", "done", ALIVE),
                ("w2", "waiting-agent", ALIVE),
                ("w3", "working", !ALIVE),
            ],
        ),
        // On another server: no picker row, so no seat fact, and the listing
        // is the only evidence there is.
        ("farm", Status::Running, vec![("lead", "working", ALIVE)]),
        // The roster a stopped session left behind: none of it counts.
        (
            "old",
            Status::Stopped,
            vec![
                ("lead", "working", ALIVE),
                ("colead", "working", ALIVE),
                ("w1", "waiting-user", ALIVE),
            ],
        ),
    ];
    match mix {
        Mix::Exact | Mix::Lossy => exact,
        Mix::Partial => exact
            .into_iter()
            .chain([(
                "ghost",
                Status::Unknown,
                vec![
                    ("lead", "working", ALIVE),
                    ("colead", "waiting-user", ALIVE),
                ],
            )])
            .collect(),
        Mix::Quiet => vec![
            orch(&[("olead", "idle", ALIVE)]),
            (
                "api",
                Status::Running,
                vec![("lead", "idle", ALIVE), ("colead", "done", ALIVE)],
            ),
        ],
        Mix::Alone => vec![
            orch(&[("olead", "idle", ALIVE)]),
            (
                "api",
                Status::Stopped,
                vec![("lead", "working", ALIVE), ("colead", "working", ALIVE)],
            ),
        ],
        Mix::Flipped => vec![
            orch(&[("olead", "waiting-user", ALIVE)]),
            (
                "api",
                Status::Running,
                vec![
                    ("lead", "waiting-user", ALIVE),
                    ("colead", "waiting-user", ALIVE),
                ],
            ),
            (
                "beta",
                Status::Running,
                vec![("lead", "waiting-user", ALIVE)],
            ),
        ],
    }
}

fn world(mix: Mix, now: Timestamp) -> World {
    let sessions = (listed(mix).into_iter())
        .map(|(name, status, seats)| {
            let mut session = entry(name, status, None);
            session.agents = (seats.into_iter())
                .map(|(seat, state, alive)| AgentEntry {
                    reference: seat.to_owned(),
                    name: seat.to_owned(),
                    state: Some(state.to_owned()),
                    // As `session.rs` emits it: a stopped session's seats are
                    // dead, an unknown one's pane liveness is not known.
                    alive: match status {
                        Status::Running => Some(alive),
                        Status::Stopped => Some(false),
                        Status::Unknown => None,
                    },
                    ..AgentEntry::default()
                })
                .collect();
            session
        })
        .collect();
    let mut world = World::new(now, sessions);
    if matches!(mix, Mix::Lossy) {
        world.losses = 1;
    }
    world
}

/// The watchdog's roster fact for `api` alone, as its own server publishes it
/// (`v1;<epoch>;<interval>;<name:profile:state:pane>…`): the one session the
/// app's tmux server knows.
fn picker(world: &World, now: Timestamp) -> Vec<PickerSession> {
    (world.sessions.iter())
        .filter(|session| session.name == "api" && session.status == Status::Running)
        .map(|session| {
            let mut seats = String::new();
            for (at, seat) in session.agents.iter().enumerate() {
                let state = seat.state.as_deref().unwrap_or("idle");
                let _ = write!(seats, ";{}:p:{state}:%{}", seat.name, at + 1);
            }
            PickerSession {
                name: session.name.clone(),
                id: "$1".to_owned(),
                rank: 0,
                glyph: String::new(),
                main_pane: String::new(),
                branch: String::new(),
                agents: format!("v1;{};30{seats}", now.epoch()),
                activity: None,
                created_at: None,
                goal: String::new(),
            }
        })
        .collect()
}

fn ask(seat: &str, words: &str) -> Section {
    Section {
        rows: vec![Need {
            seat: SeatRef {
                slot: "main".to_owned(),
                name: seat.to_owned(),
            },
            lead_pair: true,
            verdict: Verdict::Reason(Reason::WaitingUser),
            source: Source::Declaration,
            stale: None,
            since_micros: None,
            detail: words.to_owned(),
            record: None,
        }],
    }
}

/// The open asks: the orchestrator's and `api`'s own, distinct, and one more
/// each for `beta` and `gamma`, so three sessions are needy besides the
/// orchestrator and no needs count equals the `waiting-user` count.
fn needs() -> BTreeMap<String, Section> {
    BTreeMap::from([
        (ORCH.to_owned(), ask("olead", ORCH_ASK)),
        ("api".to_owned(), ask("lead", API_ASK)),
        ("beta".to_owned(), ask("lead", "BETA_ASK")),
        ("gamma".to_owned(), ask("lead", "GAMMA_ASK")),
    ])
}

fn orchestrator_record(root: &Root) {
    let dir = root.0.join("sessions").join(ORCH);
    std::fs::create_dir_all(&dir).expect("private orchestrator dir");
    let overlay = root.0.join("orchestrator.config");
    std::fs::write(&overlay, "[workspace]\norchestrator = true\n").expect("orchestrator overlay");
    let meta = format!(
        "schema=2\nsession_id={OID}\nlayout=lead-pair\nseat.main=lead\nmeta_agent=true\nlocal_config={}\n",
        overlay.display()
    );
    std::fs::write(dir.join("meta"), meta).expect("orchestrator meta");
}

/// One fleet read of `mix`: the production fold over the listing and the
/// picker, with the asks set by hand and the sidebar rows folded again.
fn read_fleet(root: &Root, mix: Mix) -> loader::FleetRead {
    let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
    let now = Timestamp::now();
    let world = world(mix, now);
    for session in &world.sessions {
        std::fs::create_dir_all(root.0.join("sessions").join(&session.name)).expect("session dir");
    }
    let dirs = (world.sessions.iter())
        .map(|session| {
            (
                session.name.clone(),
                root.0.join("sessions").join(&session.name),
            )
        })
        .collect();
    let rows = picker(&world, now);
    let mut read = reader.fold(dirs, world, Some(&rows), &FleetOrder::EMPTY, now);
    read.needs = needs();
    read.fleet = fleet::rows(
        &read.world,
        Some(ORCH),
        &read.facts,
        &BTreeMap::new(),
        &FleetOrder::EMPTY,
        &read.needs,
        Some("api"),
        now,
    );
    read
}

/// An app on `api` whose fleet read lists `mix`, with the orchestrator panel
/// open on the proved incarnation and no board anywhere.
pub(super) fn rig(tag: &str, mix: Mix) -> (Root, App) {
    let root = Root::new(tag);
    orchestrator_record(&root);
    let mut app = app(Some("api"));
    app.answer(Answer::Fleet(read_fleet(&root, mix)));
    app.lane = Rc::new(Lane {
        items: vec![said(MAIN_TURN, 0)],
        coverage: Vec::new(),
    });
    app.loading = false;
    app.answer(Answer::Orchestrator(loader::Proof::Proven(OID.to_owned())));
    bytes(&mut app, b"O");
    (root, app)
}

pub(super) fn paint(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut cells = Buffer::empty(Rect::new(0, 0, width, height));
    app.frame(&mut cells);
    app.frame_shown(Instant::now());
    cells
}

pub(super) fn bytes(app: &mut App, value: &[u8]) {
    let at = Instant::now();
    assert!(take_keys(app, Keys::app().feed(value, at)).is_some());
}

pub(super) fn row_text(cells: &Buffer, y: u16) -> String {
    (0..cells.area.width)
        .map(|x| cells[(x, y)].symbol())
        .collect()
}

fn click(app: &mut App, (column, row): (u16, u16)) {
    for kind in [MouseKind::Click, MouseKind::Release] {
        let at = Instant::now();
        let mouse = Key::Mouse(Mouse { kind, column, row });
        assert!(take_keys(app, vec![(mouse, at)]).is_some());
    }
}

fn locate(cells: &Buffer, y: u16, needle: &str) -> Option<(u16, u16)> {
    let width = u16::try_from(needle.chars().count()).expect("short needle");
    (0..=cells.area.width.saturating_sub(width)).find_map(|x| {
        let candidate: String = (x..x + width).map(|cx| cells[(cx, y)].symbol()).collect();
        (candidate == needle).then_some((x, y))
    })
}

/// Row 0 of the main column's cells and of the panel's cells: the two headers
/// of one frame.
fn headers(app: &App, cells: &Buffer) -> (String, String) {
    let panel = app.layout.panel.expect("the panel is on screen");
    let part =
        |from: u16, to: u16| -> String { (from..to).map(|x| cells[(x, 0)].symbol()).collect() };
    (part(0, panel.x), part(panel.x, panel.right()))
}

/// What the orchestrator view must never show again: the board column, its
/// header words, its hint and its cue.
pub(super) fn assert_no_board(app: &App, cells: &Buffer, what: &str) {
    let shown = text(cells);
    for word in [
        "B chat",
        "B board",
        "B/Esc chat",
        "b hide",
        "b show",
        "BOARD",
        "DECIDE",
        "COMING",
        "WAITING",
        "desk 2",
        "Check release choice",
        "Confirm scope",
    ] {
        assert!(!shown.contains(word), "{what}: {word:?} is still drawn");
    }
    let panel = app.layout.panel.expect("the panel is on screen");
    assert_eq!(
        panel.right(),
        cells.area.width,
        "{what}: the panel takes every cell to the right edge, no board beside it"
    );
}

#[test]
fn the_orchestrator_header_names_the_work_of_the_whole_fleet() {
    for full in [false, true] {
        let (_root, mut app) = rig("orchhead-work", Mix::Exact);
        if full {
            bytes(&mut app, b"F");
        }
        let cells = paint(&mut app, FIT, HEIGHT);
        let (main, panel) = headers(&app, &cells);
        assert!(
            panel.contains(WORK),
            "full={full}: the orchestrator header lacks {WORK:?}: {panel:?}"
        );
        assert!(
            !main.contains("wait you") && !main.contains("sessions"),
            "full={full}: only the orchestrator header carries it: {main:?}"
        );
    }
}

#[test]
fn zero_counts_stay_visible_and_one_session_is_singular() {
    let (_root, mut app) = rig("orchhead-quiet", Mix::Quiet);
    let cells = paint(&mut app, FIT, HEIGHT);
    let (_, panel) = headers(&app, &cells);
    assert!(
        panel.contains("fleet ● 0 working · ⚠ 0 wait you · ≡ 2 sessions"),
        "{panel:?}"
    );
    let (_root, mut alone) = rig("orchhead-alone", Mix::Alone);
    let cells = paint(&mut alone, FIT, HEIGHT);
    let (_, panel) = headers(&alone, &cells);
    assert!(
        panel.contains("fleet ● 0 working · ⚠ 0 wait you · ≡ 1 session")
            && !panel.contains("1 sessions"),
        "a stopped session's old working seats count nothing: {panel:?}"
    );
}

#[test]
fn an_unknown_session_or_a_lost_source_makes_the_seat_counts_lower_bounds() {
    for (tag, mix, why) in [
        (
            "orchhead-partial",
            Mix::Partial,
            "an unknown session counts as no session and as a gap, and its declared seats count nothing",
        ),
        (
            "orchhead-lossy",
            Mix::Lossy,
            "a source that failed to enumerate is a gap with no unknown row to show it",
        ),
    ] {
        let (_root, mut app) = rig(tag, mix);
        let cells = paint(&mut app, FIT, HEIGHT);
        let (_, panel) = headers(&app, &cells);
        assert!(
            panel.contains("fleet ● 5+ working · ⚠ 2+ wait you · ≡ 5 sessions"),
            "{why}: {panel:?}"
        );
    }
}

#[test]
fn a_later_fleet_read_replaces_the_counts_without_reopening_the_panel() {
    let (root, mut app) = rig("orchhead-refresh", Mix::Exact);
    for (mix, shown, gone) in [(Mix::Flipped, FLIPPED, WORK), (Mix::Exact, WORK, FLIPPED)] {
        app.answer(Answer::Fleet(read_fleet(&root, mix)));
        let cells = paint(&mut app, FIT, HEIGHT);
        let (_, panel) = headers(&app, &cells);
        assert!(
            panel.contains(shown) && !panel.contains(gone),
            "the read replaces the last counts: {panel:?}"
        );
        assert!(
            panel.contains("F full") && app.layout.panel.is_some(),
            "the panel stays open: {panel:?}"
        );
    }
}

#[test]
fn the_orchestrator_header_drops_its_ask_and_other_seats_keep_theirs() {
    for full in [false, true] {
        let (_root, mut app) = rig("orchhead-ask", Mix::Exact);
        if full {
            bytes(&mut app, b"F");
        }
        let cells = paint(&mut app, FIT, HEIGHT);
        let (main, panel) = headers(&app, &cells);
        if !full {
            assert!(
                main.contains(API_ASK) && main.contains('⚠'),
                "the other seat's header keeps its ask: {main:?}"
            );
        }
        assert!(
            !panel.contains(ORCH_ASK) && !panel.contains("olead:"),
            "full={full}: the orchestrator header carries no attention segment: {panel:?}"
        );
        assert!(
            panel.contains(WORK),
            "full={full}: the work segment stands in its place: {panel:?}"
        );
    }
}

#[test]
fn the_activity_word_keeps_its_place_and_the_segment_takes_the_room_it_leaves() {
    let (_root, mut app) = rig("orchhead-priority", Mix::Exact);
    // At 232 columns the side panel leaves 41 cells beside the activity: the second step
    // and no more. The first step needs 47 cells, a frame of about 241 columns.
    for (width, shown) in [(232, STEP2), (FIT, WORK)] {
        let cells = paint(&mut app, width, HEIGHT);
        let (_, panel) = headers(&app, &cells);
        assert_eq!(
            segment(&panel).as_deref(),
            Some(shown),
            "{width}: {panel:?}"
        );
        assert!(
            panel.contains("active") && panel.contains("pane") && panel.contains("F full"),
            "{width}: the activity and the controls stay: {panel:?}"
        );
    }
}

/// The work segment of an orchestrator header, whole: the first field after
/// the names, unless that field is the activity or a control. Fields stand two
/// or more spaces apart, the segment itself keeps single spaces, so a cut
/// segment is a different string, never a shorter step.
fn segment(header: &str) -> Option<String> {
    let after = &header[header.find("lead")? + "lead".len()..];
    let first = after
        .split("  ")
        .map(str::trim)
        .find(|field| !field.is_empty())?;
    let other = first.starts_with("active") || ["pane", "F full", "F side"].contains(&first);
    (!other).then(|| first.to_owned())
}

#[test]
fn at_the_owners_200_columns_the_side_panel_shows_the_counts_beside_the_activity() {
    // The side panel is 77 cells at 200 columns and leaves 26 beside the
    // names, the activity and the controls: the third step, without the fleet
    // word or the session count (15 cells). The lower-bound form (two `+` more, 17 cells)
    // still fits there, and the lower-bound second step (45 cells) needs a frame of
    // about 233 columns, its first step one of about 245.
    for (tag, mix, width, counts) in [
        ("orchhead-200", Mix::Exact, WIDE, STEP3),
        (
            "orchhead-200-partial",
            Mix::Partial,
            WIDE,
            "● 5+ · ⚠ 2+ · ≡ 5",
        ),
        (
            "orchhead-210-partial",
            Mix::Partial,
            210,
            "● 5+ · ⚠ 2+ · ≡ 5",
        ),
        (
            "orchhead-240-partial",
            Mix::Partial,
            240,
            "● 5+ working · ⚠ 2+ wait you · ≡ 5 sessions",
        ),
    ] {
        let (_root, mut app) = rig(tag, mix);
        let cells = paint(&mut app, width, HEIGHT);
        let (_, panel) = headers(&app, &cells);
        assert_eq!(
            segment(&panel).as_deref(),
            Some(counts),
            "{width}: the longest step that fits stands whole: {panel:?}"
        );
        assert!(
            !panel.contains("fleet"),
            "{width}: no fleet word at these steps: {panel:?}"
        );
        assert!(
            panel.contains(ORCH)
                && panel.contains("lead")
                && panel.contains("active")
                && panel.contains("F full"),
            "{width}: the names, the activity and the controls stay: {panel:?}"
        );
    }
}

#[test]
fn a_narrower_side_panel_shows_a_shorter_step_never_a_cut_one() {
    let (_root, mut app) = rig("orchhead-steps", Mix::Exact);
    // The activity word keeps its place until even it no longer fits: it is
    // gone at 130 columns, where the side panel is 47 cells wide.
    for (width, step, activity) in [
        (195, Some(STEP3), true),
        (175, Some(STEP4), true),
        (160, None, true),
        (130, None, false),
    ] {
        let cells = paint(&mut app, width, HEIGHT);
        let (_, panel) = headers(&app, &cells);
        assert_eq!(
            segment(&panel).as_deref(),
            step,
            "{width}: one whole step, or none: {panel:?}"
        );
        assert!(
            panel.contains(ORCH) && panel.contains("lead") && panel.contains("F full"),
            "{width}: the names and the controls stay: {panel:?}"
        );
        assert!(
            !activity || panel.contains("active"),
            "{width}: the activity word stays while it fits: {panel:?}"
        );
    }
}

#[test]
fn a_narrowing_header_shortens_the_segment_by_whole_steps_and_loses_it_before_any_name() {
    let (_root, mut app) = rig("orchhead-narrow", Mix::Exact);
    bytes(&mut app, b"F");
    let steps = [WORK, STEP2, STEP3, STEP4];
    let (mut last, mut seen) = (0, [false; 4]);
    let (mut first_at, mut names_only_at, mut gone_for_good) = (None, None, false);
    for width in (44..=WIDE).rev() {
        let header = row_text(&paint(&mut app, width, HEIGHT), 0);
        let segment = segment(&header);
        let step = segment
            .as_deref()
            .map(|text| steps.iter().position(|step| *step == text));
        assert!(
            step.is_none_or(|step| step.is_some()),
            "{width}: a step is shown whole, never cut: {header:?}"
        );
        let names = header.contains(ORCH) && header.contains("lead");
        // `segment()` finds nothing once `lead` is gone, so the marker words are
        // checked on their own: with a name missing none of them may stand.
        assert!(
            names
                || !["working", "wait you", "session", "●", "⚠", "≡"]
                    .iter()
                    .any(|word| header.contains(word)),
            "{width}: the segment never outlives the session and agent names: {header:?}"
        );
        if let Some(Some(step)) = step {
            assert!(!gone_for_good, "{width}: the segment came back: {header:?}");
            assert!(step >= last, "{width}: a longer step came back: {header:?}");
            (last, seen[step]) = (step, true);
            first_at.get_or_insert(width);
        } else if names {
            gone_for_good = true;
            names_only_at = Some(width);
        }
    }
    let (first, names_only) = (first_at.expect("wide"), names_only_at.expect("narrow"));
    assert!(
        names_only < first,
        "a width keeps the names and no step: {names_only} < {first}"
    );
    assert_eq!(seen, [true; 4], "every step stands at some width");
}

#[test]
fn pane_and_the_zoom_label_stay_and_still_click() {
    let (_root, mut app) = rig("orchhead-keep", Mix::Exact);
    let cells = paint(&mut app, WIDE, HEIGHT);
    let (_, panel) = headers(&app, &cells);
    assert!(
        panel.contains("pane") && panel.contains("F full"),
        "{panel:?}"
    );
    let at = locate(&cells, 0, "F full").expect("the zoom label");
    click(&mut app, at);
    let full = paint(&mut app, WIDE, HEIGHT);
    assert!(row_text(&full, 0).contains("F side"));
    assert_eq!(app.layout.panel.expect("full panel").x, 0);
    let at = locate(&full, 0, "F side").expect("the zoom label");
    click(&mut app, at);
    let side = paint(&mut app, WIDE, HEIGHT);
    assert!(row_text(&side, 0).contains("F full"));
    assert!(app.layout.panel.expect("side panel").x > 0);
}

#[test]
fn no_orchestrator_view_names_the_board_or_its_keys() {
    for (width, full) in [(WIDE, false), (WIDE, true), (120, false), (60, true)] {
        let (_root, mut app) = rig("orchhead-nowords", Mix::Exact);
        if full {
            bytes(&mut app, b"F");
        }
        let cells = paint(&mut app, width, HEIGHT);
        assert_no_board(&app, &cells, &format!("{width} full={full}"));
        let hint = row_text(&cells, HEIGHT - 1);
        assert!(
            !hint.contains("b hide") && !hint.contains("b show"),
            "{width} full={full}: {hint:?}"
        );
    }
}

#[test]
fn b_and_capital_b_change_nothing_in_the_orchestrator_view() {
    for (width, full) in [(WIDE, false), (WIDE, true), (120, false), (60, true)] {
        let (_root, mut app) = rig("orchhead-keys", Mix::Exact);
        if full {
            bytes(&mut app, b"F");
        }
        let before = paint(&mut app, width, HEIGHT);
        let panel = app.layout.panel;
        for key in [b"B", b"b", b"B", b"b"] {
            bytes(&mut app, key);
            let after = paint(&mut app, width, HEIGHT);
            assert_eq!(
                before, after,
                "{width} full={full}: `{}` moved a cell",
                key[0] as char
            );
            assert_eq!(app.layout.panel, panel, "{width} full={full}");
            assert!(drafted(&app).is_empty(), "the key was taken as text");
        }
    }
}

#[test]
fn the_keys_help_lists_no_board_key() {
    let (_root, mut app) = rig("orchhead-help", Mix::Exact);
    bytes(&mut app, b"?");
    let shown = text(&paint(&mut app, 120, 100));
    assert!(
        shown.contains("PgUp/PgDn"),
        "the scroll keys stay:\n{shown}"
    );
    assert!(shown.contains("narrow / widen"), "`<` `>` stay:\n{shown}");
    let word = |word: &str| {
        shown
            .split(|c: char| !c.is_alphanumeric())
            .any(|token| token.eq_ignore_ascii_case(word))
    };
    assert!(
        !word("board"),
        "the Keys tab still names the board:\n{shown}"
    );
}

/// The foreground of `needle`'s first symbol in row 0 of the orchestrator
/// panel's header, never of the sidebar's band to its left.
fn ink_at(app: &App, cells: &Buffer, needle: &str) -> ratatui_core::style::Color {
    let panel = app.layout.panel.expect("the panel is on screen");
    let width = u16::try_from(needle.chars().count()).expect("short needle");
    let at = (panel.x..=panel.right().saturating_sub(width)).find(|x| {
        let seen: String = (*x..*x + width).map(|cx| cells[(cx, 0)].symbol()).collect();
        seen == needle
    });
    cells[(
        at.unwrap_or_else(|| panic!("{needle:?} on the panel header")),
        0,
    )]
        .fg
}

#[test]
fn the_segment_glyphs_and_counts_wear_the_inks_of_the_sidebar_band() {
    use crate::theme::{Look, Mark, Palette, rgb};
    let look = Look::DEFAULT;
    let palette: Palette = look.palette;
    let colour = |token: &str| {
        let [r, g, b] = rgb(token);
        ratatui_core::style::Color::Rgb(r, g, b)
    };
    for (mix, waiting) in [(Mix::Exact, true), (Mix::Quiet, false)] {
        let (_root, mut app) = rig("orchhead-ink", mix);
        for name in ["api", ORCH] {
            app.looks.insert(name.to_owned(), (Some(look), None));
        }
        let cells = paint(&mut app, FIT, HEIGHT);
        let tone = if waiting {
            palette.needs_you
        } else {
            palette.text
        };
        assert_eq!(
            ink_at(&app, &cells, "●"),
            colour(palette.accent(Mark::Working))
        );
        assert_eq!(
            ink_at(&app, &cells, "⚠ "),
            colour(palette.accent(Mark::NeedsYou))
        );
        assert_eq!(
            ink_at(&app, &cells, "≡"),
            colour(palette.text),
            "≡ wears the text ink"
        );
        let n = if waiting { "2 wait" } else { "0 wait" };
        assert_eq!(
            ink_at(&app, &cells, n),
            colour(tone),
            "the wait-you count: {waiting}"
        );
        assert_eq!(ink_at(&app, &cells, "working"), colour(palette.dim));
        assert_eq!(
            ink_at(
                &app,
                &cells,
                if waiting { "5 working" } else { "0 working" }
            ),
            colour(palette.text),
            "the working count"
        );
    }
}

#[test]
fn with_icons_off_the_segment_wears_the_ascii_marks() {
    let (_root, mut app) = rig("orchhead-ascii", Mix::Exact);
    for name in ["api", ORCH] {
        let look = crate::theme::Look {
            icons: false,
            ..crate::theme::Look::DEFAULT
        };
        app.looks.insert(name.to_owned(), (Some(look), None));
    }
    for (width, want) in [
        (FIT, "fleet * 5 working · ! 2 wait you · = 5 sessions"),
        (WIDE, "* 5 · ! 2 · = 5"),
    ] {
        let cells = paint(&mut app, width, HEIGHT);
        let (_, panel) = headers(&app, &cells);
        assert_eq!(segment(&panel).as_deref(), Some(want), "{panel:?}");
        assert!(!panel.contains('●') && !panel.contains('≡'), "{panel:?}");
    }
}
