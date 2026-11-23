//! The snapshot's own derivations: seat order, the worker count and when a
//! snapshot is partial.

use super::{Cache, Input, Live, LiveSession, REASON_CHARS, SessionBody, clip, from_reads};
use crate::digest::{AgentEntry, SessionEntry, Status};
use crate::time::Timestamp;

const AS_OF: i64 = 1_800_000_000;

fn agent(name: &str, state: &str) -> AgentEntry {
    AgentEntry {
        reference: name.to_owned(),
        name: name.to_owned(),
        alive: Some(true),
        state: Some(state.to_owned()),
        ..AgentEntry::default()
    }
}

fn body(roster: &[(&str, &str)]) -> SessionBody {
    let mut entry = SessionEntry::new("s", Status::Running);
    entry.main_agent = Some("lead".to_owned());
    entry.agents = roster
        .iter()
        .map(|(name, state)| agent(name, state))
        .collect();
    let input = Input {
        entry: &entry,
        dir: None,
        pending: Ok(&[]),
    };
    let live = from_reads(&[input], "s", Timestamp::from_epoch(AS_OF), 0);
    let session = live.sessions.into_iter().next();
    session
        .and_then(|session| session.body.ok())
        .expect("one readable session")
}

#[test]
fn seats_stand_waits_first_then_working_then_the_rest_in_roster_order() {
    let body = body(&[
        ("other", "paused"),
        ("closed", "done"),
        ("first", "working"),
        ("asker", "waiting-agent"),
        ("stuck", "blocked"),
        ("human", "waiting-user"),
        ("second", "working"),
    ]);
    let order: Vec<&str> = body.seats.iter().map(|seat| seat.name.as_str()).collect();
    assert_eq!(
        order,
        [
            "human", "stuck", "asker", "first", "second", "closed", "other"
        ]
    );
}

#[test]
fn running_workers_count_working_seats_other_than_the_main_one() {
    let body = body(&[
        ("lead", "working"),
        ("first", "working"),
        ("asker", "waiting-agent"),
        ("second", "working"),
    ]);
    assert_eq!(body.running_workers(), 2);
}

#[test]
fn a_snapshot_is_partial_when_a_source_or_a_session_body_is_missing() {
    let readable = LiveSession {
        name: "s".to_owned(),
        dir: None,
        uuid: None,
        body: Ok(SessionBody {
            seats: Vec::new(),
            asks: Vec::new(),
        }),
    };
    let unreadable = LiveSession {
        body: Err("journal unreadable".to_owned()),
        ..readable.clone()
    };
    let live = |sessions: Vec<LiveSession>, losses: usize| Live {
        as_of: Timestamp::from_epoch(AS_OF),
        sessions,
        losses,
    };
    assert!(!live(vec![readable.clone()], 0).partial(), "all read");
    assert!(live(vec![readable.clone()], 1).partial(), "a lost source");
    assert!(
        live(vec![readable.clone()], 2).partial(),
        "two lost sources"
    );
    assert!(live(vec![readable, unreadable], 0).partial(), "a lost body");
}

#[test]
fn the_own_session_comes_first_and_the_rest_by_name() {
    let entries: Vec<SessionEntry> = ["beta", "own", "alpha"]
        .into_iter()
        .map(|name| SessionEntry::new(name, Status::Running))
        .collect();
    let inputs: Vec<Input<'_>> = entries
        .iter()
        .map(|entry| Input {
            entry,
            dir: None,
            pending: Ok(&[]),
        })
        .collect();
    let live = from_reads(&inputs, "own", Timestamp::from_epoch(AS_OF), 0);
    let names: Vec<&str> = live
        .sessions
        .iter()
        .map(|session| session.name.as_str())
        .collect();
    assert_eq!(names, ["own", "alpha", "beta"]);
}

#[test]
fn a_long_reason_is_cut_to_its_budget_with_an_ellipsis() {
    let fits = "x".repeat(REASON_CHARS);
    assert_eq!(clip(&fits), fits);
    let cut = clip(&"y".repeat(REASON_CHARS * 2));
    assert_eq!(cut.chars().count(), REASON_CHARS);
    assert_eq!(cut, format!("{}…", "y".repeat(REASON_CHARS - 1)));
}

#[test]
fn the_cache_serves_its_snapshot_only_while_young_and_never_from_the_future() {
    let sample = |at: i64| Live {
        as_of: Timestamp::from_epoch(at),
        sessions: Vec::new(),
        losses: 0,
    };
    let mut cache = Cache::holding(AS_OF, sample(AS_OF));
    assert_eq!(cache.held(), Some(&sample(AS_OF)), "held as given");
    let at = |cache: &mut Cache, now: i64| cache.get(now, || sample(now)).as_of.epoch();
    assert_eq!(at(&mut cache, AS_OF + 59), AS_OF, "young: kept");
    assert_eq!(at(&mut cache, AS_OF + 60), AS_OF + 60, "old: read again");
    assert_eq!(
        at(&mut cache, AS_OF + 10),
        AS_OF + 10,
        "a sample from the future is read again"
    );
    assert_eq!(
        cache.held(),
        Some(&sample(AS_OF + 10)),
        "the new read is held"
    );
}

#[test]
fn a_fleet_read_whose_records_do_not_pair_shows_every_session_unavailable_with_that_reason() {
    let read = crate::fleet_read::WorldRead {
        snapshot: crate::liveness::Snapshot {
            sessions: Vec::new(),
            incomplete: Vec::new(),
        },
        world: crate::listing::World::new(
            Timestamp::from_epoch(AS_OF),
            vec![SessionEntry::new("own", Status::Running)],
        ),
        panes: std::collections::BTreeMap::new(),
        pids: std::collections::BTreeMap::new(),
        sockets: crate::fleet_read::SocketPaths::live(),
    };
    let live = super::from_world(&read, "own");
    let bodies: Vec<&Result<SessionBody, String>> =
        live.sessions.iter().map(|session| &session.body).collect();
    assert_eq!(
        bodies,
        [&Err(
            "journal unreadable: the fleet read did not pair its records".to_owned()
        )]
    );
}
