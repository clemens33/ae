//! Unit cases for the o8 panel binding beside the frozen acceptance: what an
//! opening asks the reader for, which proof binds, what a seat answer may
//! change, and the status column an unbound panel draws. A child of
//! `app::tests`, driven by hand without tmux or a reader thread.

use super::*;
use std::sync::mpsc::Receiver;

const ORCH: &str = crate::orchestrator::ORCHESTRATOR_SESSION;
const OID: &str = "0199c0de-2222-4890-abcd-ef0123456789";
const NEW: &str = "0199c0de-3333-4890-abcd-ef0123456789";
const WAITING: &str = "waiting for the orchestrator seat";
const LOST: &str = "orchestrator stopped; O starts it";

/// A hand-driven app beside two sessions, its reader asks recorded.
fn ready(proof: loader::Proof) -> (App, Receiver<loader::Request>) {
    let mut app = app(None);
    two(&mut app);
    let (ask, asks) = std::sync::mpsc::channel();
    app.ask = Some(ask);
    app.orch.proof = proof;
    (app, asks)
}

/// The seat asks the app sent since the last look, by generation.
fn seats(asks: &Receiver<loader::Request>) -> Vec<u64> {
    asks.try_iter()
        .filter_map(|request| match request {
            loader::Request::Seat(generation) => Some(generation),
            _ => None,
        })
        .collect()
}

fn seat(app: &mut App, generation: u64, said: loader::Said) {
    app.answer(Answer::Seat { generation, said });
}

fn proof(app: &mut App, proof: loader::Proof) {
    app.answer(Answer::Orchestrator(proof));
}

fn pin(app: &App) -> Option<String> {
    (app.orch.panel.as_ref()).and_then(|panel| panel.pin().map(str::to_owned))
}

#[test]
fn an_opening_with_no_seat_asks_once_and_binds_the_first_proof_whatever_its_line() {
    let (mut app, asks) = ready(loader::Proof::Absent);
    app.open_panel(false);
    assert_eq!(seats(&asks), [1], "one seat ask per opening");
    assert_eq!(app.panel_line().as_deref(), Some(WAITING));
    let why = "ae orchestrator: no profile for the seat";
    seat(&mut app, 1, loader::Said::Waiting(Some(why.to_owned())));
    assert_eq!(
        app.panel_line().as_deref(),
        Some(why),
        "a failed start says why"
    );
    assert_eq!(pin(&app), None, "a seat answer never binds");
    proof(&mut app, loader::Proof::Proven(OID.to_owned()));
    assert_eq!(pin(&app).as_deref(), Some(OID), "the first proof binds");
    assert_eq!(app.panel_line(), None);
    assert!(seats(&asks).is_empty(), "a bound panel asks nothing more");
}

#[test]
fn a_refusal_ends_the_wait_and_no_later_proof_binds_until_a_reopen() {
    let (mut app, asks) = ready(loader::Proof::Absent);
    app.open_panel(false);
    let why = "the orchestrator role is held by 'router', which is stopped; ae router resumes it";
    seat(&mut app, 1, loader::Said::Refused(why.to_owned()));
    proof(&mut app, loader::Proof::Proven(OID.to_owned()));
    assert_eq!(
        pin(&app),
        None,
        "a refused opening never binds a later proof"
    );
    assert_eq!(app.panel_line().as_deref(), Some(why));
    app.close_panel();
    app.open_panel(false);
    assert_eq!(seats(&asks), [1], "a proven reopen asks for nothing");
    assert_eq!(pin(&app).as_deref(), Some(OID), "an explicit reopen binds");
}

#[test]
fn an_older_openings_answer_changes_nothing() {
    let (mut app, asks) = ready(loader::Proof::Absent);
    app.open_panel(false);
    app.close_panel();
    app.open_panel(false);
    assert_eq!(seats(&asks), [1, 2]);
    seat(&mut app, 1, loader::Said::Refused("old refusal".to_owned()));
    assert_eq!(app.panel_line().as_deref(), Some(WAITING));
    seat(
        &mut app,
        2,
        loader::Said::Waiting(Some("new line".to_owned())),
    );
    assert_eq!(app.panel_line().as_deref(), Some("new line"));
}

#[test]
fn a_lost_seat_says_o_starts_it_asks_nothing_and_never_takes_another() {
    let (mut app, asks) = ready(loader::Proof::Proven(OID.to_owned()));
    app.open_panel(false);
    assert_eq!(pin(&app).as_deref(), Some(OID));
    proof(&mut app, loader::Proof::Absent);
    assert_eq!(app.panel_line().as_deref(), Some(LOST));
    assert!(seats(&asks).is_empty(), "a lost seat is never restarted");
    proof(&mut app, loader::Proof::Proven(NEW.to_owned()));
    assert_eq!(
        app.panel_line().as_deref(),
        Some("orchestrator replaced; O reopens")
    );
    assert_eq!(pin(&app).as_deref(), Some(OID));
}

#[test]
fn an_unproven_opening_asks_nothing_and_says_why() {
    let (mut app, asks) = ready(loader::Proof::Gap("meta: x".to_owned()));
    app.open_panel(false);
    assert!(seats(&asks).is_empty());
    assert_eq!(
        app.panel_line().as_deref(),
        Some("orchestrator unproven: meta: x")
    );
    proof(&mut app, loader::Proof::Proven(OID.to_owned()));
    assert_eq!(pin(&app), None, "an unproven opening never pins a recovery");
}

#[test]
fn a_home_seat_opens_on_its_proof_and_a_home_with_none_running_opens_nothing() {
    let mut app = app(Some(ORCH));
    let (ask, asks) = std::sync::mpsc::channel();
    app.ask = Some(ask);
    proof(&mut app, loader::Proof::Absent);
    assert!(app.orch.panel.is_none(), "no seat runs: O asks for it");
    assert!(seats(&asks).is_empty());
    let mut app = super::app(Some(ORCH));
    proof(&mut app, loader::Proof::Proven(OID.to_owned()));
    assert!(app.orch.panel.as_ref().is_some_and(|panel| panel.full));
    assert_eq!(pin(&app).as_deref(), Some(OID));
}

#[test]
fn an_unbound_panel_is_a_status_column_with_its_line_whole_once_and_no_stopped_seat() {
    let (mut app, _asks) = ready(loader::Proof::Absent);
    app.world.sessions.push(entry(ORCH, Status::Stopped, None));
    app.open_panel(false);
    let why = "orchestrator not started: name collision: the canonical 'orchestrator' \
               state exists without the orchestrator role";
    seat(&mut app, 1, loader::Said::Refused(why.to_owned()));
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    app.frame(&mut buf);
    let cells = app.layout.panel.expect("the side panel drew");
    let rows: Vec<String> = (cells.y..cells.y + cells.height)
        .map(|y| {
            (cells.x..cells.x + cells.width)
                .map(|x| buf[(x, y)].symbol())
                .collect()
        })
        .collect();
    let header = rows.first().cloned().unwrap_or_default();
    assert!(header.contains(ORCH), "the header names the seat: {header}");
    assert!(
        !header.to_ascii_lowercase().contains("stopped") && !header.contains("active"),
        "no seat state in an unbound header: {header}"
    );
    let words = rows
        .iter()
        .map(|row| row.trim_matches(|c: char| c == ' ' || c == '│'))
        .filter(|row| !row.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        words.matches(why).count(),
        1,
        "the line, whole, once: {words}"
    );
    assert!(rows.iter().any(|row| row.trim().ends_with("read-only")));
    assert!(!words.contains("read-only ·"), "the reason is not repeated");
    assert!(!words.contains("Not running: no seat facts."));
}

/// The frame a 160-column terminal shows now, its layout kept.
fn drawn(app: &mut App) {
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    app.frame(&mut buf);
}

/// Where the side panel's border stands: its left column.
fn border(app: &App) -> Option<u16> {
    app.layout.panel.map(|cells| cells.x)
}

/// Press the border at `from`, move to `to`, let go, and draw again.
fn dragged(app: &mut App, from: u16, to: u16) {
    for (kind, column) in [
        (MouseKind::Click, from),
        (MouseKind::Drag, to),
        (MouseKind::Release, to),
    ] {
        let _ = app.mouse(Mouse {
            kind,
            column,
            row: 1,
        });
    }
    drawn(app);
}

#[test]
fn the_side_panel_border_follows_a_drag_both_ways_within_both_floors_and_moves_no_keys() {
    let (mut app, _asks) = ready(loader::Proof::Proven(OID.to_owned()));
    app.open_panel(false);
    drawn(&mut app);
    // 160 columns: a 44-column sidebar and its rule leave the chat 115, so a
    // panel stands 57 wide by default, 40 at least and 75 at most.
    let start = border(&app).expect("a side panel");
    assert_eq!(start, 160 - 57);
    let focused = app.panel_focused();
    dragged(&mut app, start, start + 8);
    assert_eq!(
        border(&app),
        Some(start + 8),
        "the border follows the pointer"
    );
    assert_eq!(app.model.split().panel, Some(57 - 8), "the width is kept");
    assert_eq!(app.panel_focused(), focused, "a border press moves no keys");
    dragged(&mut app, start + 8, 0);
    assert_eq!(border(&app), Some(160 - 75), "the main column keeps 40");
    dragged(&mut app, 160 - 75, 159);
    assert_eq!(border(&app), Some(160 - 40), "the panel keeps 40");
    assert!(app.model.drag().is_none(), "every release ends its drag");
}

#[test]
fn a_held_panel_border_is_marked_until_let_go() {
    let (mut app, _asks) = ready(loader::Proof::Proven(OID.to_owned()));
    app.open_panel(false);
    let area = Rect::new(0, 0, 160, 45);
    let style = |app: &mut App, at: u16| {
        let mut buf = Buffer::empty(area);
        app.frame(&mut buf);
        buf[(at, 5)].style()
    };
    drawn(&mut app);
    let at = border(&app).expect("a side panel");
    let resting = style(&mut app, at);
    let press = |kind| Mouse {
        kind,
        column: at,
        row: 1,
    };
    let _ = app.mouse(press(MouseKind::Click));
    assert_ne!(style(&mut app, at), resting, "a held border is marked");
    let _ = app.mouse(press(MouseKind::Release));
    assert_eq!(style(&mut app, at), resting, "letting go unmarks it");
}

/// The panel column's lane, read through the panel without a frame: whether
/// it still waits for a read, and how many turns it holds.
fn panel_lane(app: &mut App) -> (bool, usize) {
    app.in_panel(|app| (app.loading, app.lane.items.len()))
        .expect("an open panel")
}

/// A panel open on the proven incarnation `OID`, with the seat's directory and
/// id known to the app, as a fleet read leaves them.
fn bound_panel() -> App {
    let (mut app, _asks) = ready(loader::Proof::Proven(OID.to_owned()));
    app.dirs.insert(ORCH.to_owned(), PathBuf::from(ORCH));
    app.ids.insert(ORCH.to_owned(), OID.to_owned());
    app.open_panel(false);
    assert_eq!(pin(&app).as_deref(), Some(OID), "the panel binds the proof");
    assert_eq!(
        panel_lane(&mut app),
        (true, 0),
        "before its first read the panel waits"
    );
    app
}

#[test]
fn a_read_of_the_pinned_orchestrator_lane_reaches_the_open_panel_at_once() {
    let mut app = bound_panel();
    app.answer(view(ORCH, OID, 1, "PINNED_TURN"));
    assert_eq!(
        panel_lane(&mut app),
        (false, 1),
        "the pinned incarnation's read shows in the panel without another event"
    );
}

#[test]
fn a_listed_namesake_read_leaves_the_open_panel_on_its_pinned_lane() {
    let mut app = bound_panel();
    app.answer(view(ORCH, OID, 1, "PINNED_TURN"));
    assert_eq!(panel_lane(&mut app), (false, 1), "the pinned read shows");
    // Another incarnation holds the name and is listed: an ordinary row.
    app.fleet.rows.push(row(ORCH, 3, false));
    app.ids.insert(ORCH.to_owned(), NEW.to_owned());
    app.answer(view(ORCH, NEW, 2, "NAMESAKE_TURN"));
    assert_eq!(
        app.shown.get(ORCH).map(|shown| shown.id.as_str()),
        Some(NEW),
        "the namesake's read is kept as an ordinary row's"
    );
    assert_eq!(
        panel_lane(&mut app),
        (false, 1),
        "an ordinary row's read never refreshes the panel, which keeps its pinned lane"
    );
}
