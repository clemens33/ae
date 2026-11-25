//! `desk check`: the token boundary of a board match, the binding of a wait to
//! its launch, the way a relay target is read, and the sections with their cut.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "a fixture builds its own records and states what it expects"
)]

use super::{
    CLOSED_SHOWN, EXIT_UNBOARDED, Fate, Gap, Hint, Join, Journal, Kind, Report, Row, SessionInput,
    Wait, Witness, absent_label, board_error_line, degradation, evaluate, fate, fleet_gaps, hint,
    holders, is_owner_opening, join, keyed_declaration, mentions, render, waits_of, witness_of,
};
use crate::desk::{Desk, parse_input};
use crate::digest::Status;
use crate::events::{Cursor, Event, SessionKey};
use crate::inventory::{FailedSource, ServerId};
use crate::meta::Meta;
use crate::relay::Claim;
use crate::session::{MetaRead, RecordSnapshot, SessionRead};
use crate::time::Timestamp;
use crate::tracked::{self, EventFields};
use std::fmt::Write as _;

const ORCH: &str = "11111111-2222-4333-8444-555555555555";
const TARGET: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

fn at(epoch: i64) -> Timestamp {
    Timestamp::from_epoch(epoch)
}

fn board(text: &str) -> Desk {
    Desk {
        updated: at(1_700_000_000),
        items: parse_input(text.as_bytes()).expect("a board in the input grammar"),
    }
}

fn journal(name: &str, uuid: &str, lines: &[String]) -> Journal {
    let bytes: Vec<u8> = lines
        .iter()
        .flat_map(|line| format!("{}\n", line.trim_end()).into_bytes())
        .collect();
    let states = crate::requests::states_in(&bytes, SessionKey::new(name, uuid));
    let events = lines
        .iter()
        .map(|line| Event::parse_line(line.trim_end()).expect("a record"))
        .collect();
    Journal::new(events, states)
}

fn session(name: &str, uuid: &str, status: Status, lines: &[String]) -> SessionInput {
    SessionInput {
        name: name.to_owned(),
        uuid: Some(uuid.to_owned()),
        status,
        record_gap: None,
        journal: Ok(journal(name, uuid, lines)),
        waits: Ok(Vec::new()),
    }
}

/// The owner's ask in `ORCH`'s chat.
fn owner_ask(id: &str, ts: i64) -> String {
    let fields = EventFields {
        target_session_id: ORCH,
        ..EventFields::new(
            at(ts),
            tracked::CONSOLE_SINK,
            "ask",
            "boss",
            id,
            "",
            "",
            "main",
            "orch",
            "",
            "",
        )
    };
    tracked::event_line(&fields)
}

/// `ORCH`'s claim of relay `id` to `TARGET`, citing `owner` when it is bound.
fn claim_line(id: &str, owner: Option<&str>, ts: i64) -> String {
    let fields = EventFields {
        actor_session_id: ORCH,
        target_session_id: TARGET,
        target_session_uuid: TARGET,
        ..EventFields::new(
            at(ts),
            "boss",
            "relay-attempt",
            "target:lead",
            id,
            "main",
            "orch",
            "main",
            "target",
            "task",
            "",
        )
    };
    let keys: Vec<(&'static str, &str)> = owner.map_or_else(Vec::new, |owner| {
        vec![("relay_mode", "bound"), ("owner_input", owner)]
    });
    tracked::event_line_with(&fields, "launch-1", &keys)
}

fn claim(outcome: Option<&'static str>) -> Claim {
    Claim {
        target_session: "target".to_owned(),
        target_uuid: TARGET.to_owned(),
        target_slot: "main".to_owned(),
        text: String::new(),
        outcome,
    }
}

fn wait(since: Option<i64>, epoch: Option<i64>) -> Wait {
    Wait {
        agent: "lead".to_owned(),
        since: since.map(at),
        slot: Some("main".to_owned()),
        launch_id: Some("tok-1".to_owned()),
        launch_epoch: epoch,
        witness: Witness::Absent,
    }
}

fn with_waits(waits: Vec<Wait>) -> SessionInput {
    let mut input = session("alpha", TARGET, Status::Running, &[]);
    input.waits = Ok(waits);
    input
}

fn rows(report: &Report) -> Vec<String> {
    report
        .unboarded
        .iter()
        .map(|row| row.key.clone())
        .collect::<Vec<_>>()
}

#[test]
fn a_typed_id_matches_only_as_a_whole_token() {
    let id = "ae-20261124T092511Z-985e4bdc";
    for (text, expected) in [
        (format!("Waiting on ({id})"), true),
        (format!("Waiting on {id}."), true),
        (format!("see https://example.test/ask/{id}"), true),
        (id.to_string(), true),
        (format!("{id}-backup"), false),
        (format!("prefix-{id}"), false),
        (format!("{id}x"), false),
        (format!("{id}.log"), false),
        (format!("x{id}"), false),
        (format!("{id}:lead"), false),
        ("nothing here".to_owned(), false),
    ] {
        assert_eq!(mentions(&text, id), expected, "{text}");
    }
    assert!(!mentions("anything", ""), "an empty id names nothing");
    assert!(mentions("@alpha:lead waits", "alpha:lead"));
    assert!(mentions("alpha:lead", "alpha:lead"));
    assert!(!mentions("alpha:lead2", "alpha:lead"));
}

#[test]
fn a_hint_names_the_item_and_the_field() {
    let id = "req-1";
    let desk = board(&format!(
        "RUNNING\nR1 | unrelated\nR2 | Waiting on {id}\nR3 | See link | https://example.test/{id}\nR4 | Both {id} | https://example.test/{id}\n"
    ));
    let items = &desk.items;
    let named = hint(items, id).expect("a hint");
    assert_eq!((named.item.as_str(), named.via), ("R2", "text"));
    assert!(hint(&items[..1], id).is_none());
    let link = hint(&items[2..3], id).expect("a hint");
    assert_eq!((link.item.as_str(), link.via), ("R3", "link"));
    let both = hint(&items[3..], id).expect("a hint");
    assert_eq!((both.item.as_str(), both.via), ("R4", "text and link"));
}

#[test]
fn a_wait_binds_only_to_a_launch_that_began_strictly_before_its_declaration() {
    for (since, epoch, bound, phrase) in [
        (100, 90, true, ""),
        (100, 99, true, ""),
        (100, 100, false, "in the same second"),
        (100, 101, false, "before the seat's current launch"),
    ] {
        let input = with_waits(vec![wait(Some(since), Some(epoch))]);
        let report = evaluate(None, &[input], Vec::new(), at(200));
        assert_eq!(
            report.unboarded.len(),
            usize::from(bound),
            "{since} {epoch}"
        );
        assert_eq!(
            report.unresolved.len(),
            usize::from(!bound),
            "{since} {epoch}"
        );
        if bound {
            let row = &report.unboarded[0];
            assert_eq!(row.kind, Kind::Seat);
            assert_eq!(row.key, "alpha:lead");
            assert!(
                row.place.contains(TARGET) && row.place.contains("tok-1"),
                "{}",
                row.place
            );
        } else {
            let gap = &report.unresolved[0];
            assert_eq!(gap.subject, "seat alpha:lead");
            assert!(gap.reason.contains(phrase), "{}", gap.reason);
        }
    }
}

#[test]
fn the_two_same_second_histories_are_both_unresolved_and_neither_a_row() {
    // History A: an old wait declared in second 100, then a relaunch that
    // stamped second 100. History B: a relaunch that stamped second 100, then
    // a new wait declared in second 100. The record is the same in both.
    for history in ["old wait, then relaunch", "relaunch, then new wait"] {
        let report = evaluate(
            None,
            &[with_waits(vec![wait(Some(100), Some(100))])],
            Vec::new(),
            at(200),
        );
        assert!(report.unboarded.is_empty(), "{history}");
        assert_eq!(report.unresolved.len(), 1, "{history}");
        assert!(
            report.unresolved[0].reason.contains("not proved"),
            "{history}: {}",
            report.unresolved[0].reason
        );
    }
}

#[test]
fn a_wait_without_a_proven_launch_is_unresolved_with_its_reason() {
    let mut missing = Vec::new();
    for (name, wait, expected) in [
        (
            "no slot",
            Wait {
                slot: None,
                ..wait(Some(100), Some(90))
            },
            "no slot",
        ),
        (
            "no launch id",
            Wait {
                launch_id: None,
                ..wait(Some(100), Some(90))
            },
            "launch_id.main",
        ),
        ("no baseline", wait(Some(100), None), "launch_time.main"),
        ("no time", wait(None, Some(90)), "declaration time"),
    ] {
        let report = evaluate(None, &[with_waits(vec![wait])], Vec::new(), at(200));
        assert!(report.unboarded.is_empty(), "{name}");
        let reason = report.unresolved.first().map(|gap| gap.reason.clone());
        missing.push((name, reason.is_some_and(|reason| reason.contains(expected))));
    }
    assert!(missing.iter().all(|(_, found)| *found), "{missing:?}");
    let mut anonymous = with_waits(vec![wait(Some(100), Some(90))]);
    anonymous.uuid = None;
    let report = evaluate(None, &[anonymous], Vec::new(), at(200));
    assert!(report.unboarded.is_empty());
    assert!(report.unresolved[0].reason.contains("no session id"));
}

#[test]
fn a_seat_state_that_is_not_exact_is_a_gap_never_clean() {
    let mut input = session("alpha", TARGET, Status::Running, &[]);
    input.waits = Err("seat states are not exact".to_owned());
    let report = evaluate(None, &[input], Vec::new(), at(200));
    assert!(report.unboarded.is_empty());
    assert_eq!(report.unresolved[0].subject, "seats of alpha");
}

#[test]
fn sessions_are_read_stopped_or_unreadable_and_only_the_last_is_a_gap() {
    let mut broken = session("broken", ORCH, Status::Running, &[]);
    broken.journal = Err("journal line 3 is not a record".to_owned());
    let unknown = session("unknown", TARGET, Status::Unknown, &[]);
    let stopped = session(
        "stopped",
        "cccccccc-dddd-4eee-8fff-000000000000",
        Status::Stopped,
        &[],
    );
    let mut degraded = session(
        "degraded",
        "dddddddd-dddd-4eee-8fff-000000000000",
        Status::Running,
        &[],
    );
    degraded.record_gap = Some("record partly unreadable".to_owned());
    let fine = session(
        "fine",
        "eeeeeeee-dddd-4eee-8fff-000000000000",
        Status::Running,
        &[],
    );
    let report = evaluate(
        None,
        &[broken, unknown, stopped, degraded, fine],
        Vec::new(),
        at(5),
    );
    assert_eq!((report.read, report.stopped, report.unreadable), (1, 1, 3));
    let subjects: Vec<&str> = report
        .unresolved
        .iter()
        .map(|gap| gap.subject.as_str())
        .collect();
    assert_eq!(
        subjects,
        ["session broken", "session unknown", "session degraded"]
    );
    assert!(
        report.unresolved[0]
            .reason
            .starts_with("journal unreadable: ")
    );
    assert_eq!(report.unresolved[1].reason, "liveness unknown");
    assert_eq!(report.code(), 0, "coverage gaps never decide the exit");
}

#[test]
fn a_fleet_that_lost_a_source_names_each_one_and_never_reads_clean() {
    use std::path::PathBuf;
    let lost = [
        FailedSource::CanonicalRoot(PathBuf::from("/s/sessions")),
        FailedSource::WorktreeRoot(PathBuf::from("/s/worktrees")),
        FailedSource::WorktreeState(PathBuf::from("/w/.ae")),
        FailedSource::Server(ServerId::Ambient),
    ];
    let gaps = fleet_gaps(&lost, false, true);
    assert_eq!(gaps.len(), 4);
    for (gap, phrase) in gaps.iter().zip([
        "/s/sessions would not enumerate",
        "/s/worktrees would not enumerate",
        "/w/.ae would not enumerate",
        "a tmux server did not answer",
    ]) {
        assert_eq!(gap.subject, "fleet source");
        assert!(gap.reason.contains(phrase), "{}", gap.reason);
    }
    let unnamed = fleet_gaps(&[], false, true);
    assert_eq!(unnamed.len(), 1);
    assert!(unnamed[0].reason.contains("names no source"));
    let unpaired = fleet_gaps(&[], true, false);
    assert_eq!(unpaired.len(), 1);
    assert_eq!(unpaired[0].subject, "fleet");
    assert!(fleet_gaps(&[], true, true).is_empty());
    let report = evaluate(None, &[], gaps, at(5));
    assert_eq!(report.unresolved.len(), 4);
    assert_eq!(report.code(), 0, "a coverage gap never decides the exit");
}

#[test]
fn a_target_is_followed_by_uuid_and_only_a_running_readable_one_is_read() {
    let origin = ORCH;
    let sessions = |targets: Vec<SessionInput>| targets;
    let none = sessions(vec![session("other", ORCH, Status::Running, &[])]);
    let Fate::Unresolved(why) = fate(&claim(None), "r1", origin, &holders(&none)) else {
        panic!("a target no session holds is unresolved");
    };
    assert!(why.contains(TARGET), "{why}");
    let twice = sessions(vec![
        session("one", TARGET, Status::Running, &[]),
        session("two", TARGET, Status::Running, &[]),
    ]);
    let Fate::Unresolved(why) = fate(&claim(None), "r1", origin, &holders(&twice)) else {
        panic!("a target two sessions hold is unresolved");
    };
    assert!(why.starts_with("2 sessions hold"), "{why}");
    for (status, word) in [(Status::Stopped, "stopped"), (Status::Unknown, "unknown")] {
        let held = sessions(vec![session("t", TARGET, status, &[])]);
        let Fate::Unresolved(why) = fate(&claim(None), "r1", origin, &holders(&held)) else {
            panic!("a {word} target is unresolved");
        };
        assert!(why.contains(word), "{why}");
    }
    let mut unreadable = session("t", TARGET, Status::Running, &[]);
    unreadable.journal = Err("the journal is a directory".to_owned());
    let Fate::Unresolved(why) = fate(&claim(None), "r1", origin, &holders(&[unreadable])) else {
        panic!("an unreadable target journal is unresolved");
    };
    assert!(why.starts_with("the target journal: "), "{why}");
    let silent = sessions(vec![session("t", TARGET, Status::Running, &[])]);
    let Fate::Open(word, label) = fate(
        &claim(Some("not-delivered")),
        "r1",
        origin,
        &holders(&silent),
    ) else {
        panic!("a running target with no opening leaves the relay open");
    };
    assert_eq!(
        (word, label.as_str()),
        ("absent", "undelivered, no opening")
    );
}

#[test]
fn an_absent_opening_is_labelled_by_what_the_attempt_recorded() {
    assert_eq!(
        absent_label(&claim(Some("not-delivered"))),
        "undelivered, no opening"
    );
    assert_eq!(
        absent_label(&claim(Some("sent"))),
        "no opening, outcome sent"
    );
    assert_eq!(
        absent_label(&claim(Some("uncertain"))),
        "no opening, outcome uncertain"
    );
    assert_eq!(absent_label(&claim(None)), "no opening, outcome unrecorded");
}

#[test]
fn an_owner_ask_lists_its_relays_and_a_stopped_target_stays_unresolved() {
    let lines = [
        owner_ask("ask-1", 100),
        claim_line("relay-1", Some("ask-1"), 110),
    ];
    let orch = session("orch", ORCH, Status::Running, &lines);
    let stopped = session("target", TARGET, Status::Stopped, &[]);
    let report = evaluate(None, &[orch, stopped], Vec::new(), at(200));
    assert_eq!(rows(&report), ["ask-1"]);
    let ask = &report.unboarded[0];
    assert_eq!(ask.kind, Kind::OwnerAsk);
    assert!(ask.detail.contains("relay-1 unresolved"), "{}", ask.detail);
    assert!(report.closed.is_empty());
    let gap = report
        .unresolved
        .iter()
        .find(|gap| gap.subject.starts_with("relay relay-1"))
        .expect("the relay is unresolved coverage");
    assert!(gap.reason.contains("stopped"), "{}", gap.reason);
    assert_eq!(report.stopped, 1);
}

#[test]
fn a_replied_owner_ask_is_no_obligation_while_its_relay_stays_a_gap() {
    let reply = EventFields::new(
        at(150),
        "boss",
        "reply",
        tracked::CONSOLE_SINK,
        "ask-1",
        "main",
        "orch",
        "",
        "",
        "answered",
        "",
    );
    let lines = [
        owner_ask("ask-1", 100),
        claim_line("relay-1", Some("ask-1"), 110),
        tracked::event_line(&reply),
    ];
    let orch = session("orch", ORCH, Status::Running, &lines);
    let report = evaluate(None, &[orch], Vec::new(), at(200));
    assert!(report.unboarded.is_empty(), "{:?}", report.unboarded);
    assert!(
        report
            .unresolved
            .iter()
            .any(|gap| gap.subject.starts_with("relay relay-1"))
    );
}

#[test]
fn a_board_item_moves_an_obligation_to_the_hint_section_and_off_the_exit_code() {
    let lines = [owner_ask("ask-1", 100)];
    let orch = || session("orch", ORCH, Status::Running, &lines);
    let bare = evaluate(
        Some(&board("RUNNING\nR1 | nothing\n")),
        &[orch()],
        Vec::new(),
        at(200),
    );
    assert_eq!(bare.code(), EXIT_UNBOARDED);
    let named = evaluate(
        Some(&board("RUNNING\nR1 | Waiting on ask-1\n")),
        &[orch()],
        Vec::new(),
        at(200),
    );
    assert_eq!(named.code(), 0);
    assert!(named.unboarded.is_empty());
    assert_eq!(named.hinted.len(), 1);
    assert_eq!(
        named.hinted[0].1,
        Hint {
            item: "R1".to_owned(),
            via: "text"
        }
    );
}

fn closed_row(index: i64) -> Row {
    Row {
        kind: Kind::Relay,
        key: format!("relay-{index:02}"),
        place: "orch -> target:main".to_owned(),
        at: Some(at(1_000 + index)),
        detail: "replied".to_owned(),
    }
}

fn report_with(closed: Vec<Row>) -> Report {
    Report {
        as_of: at(5_000),
        board: None,
        read: 2,
        stopped: 0,
        unreadable: 0,
        unboarded: Vec::new(),
        hinted: Vec::new(),
        closed,
        unresolved: Vec::new(),
    }
}

#[test]
fn the_known_closed_list_is_cut_at_twenty_unless_all_is_asked() {
    let closed: Vec<Row> = (0..21).rev().map(closed_row).collect();
    let report = report_with(closed);
    let short = render(&report, false);
    assert!(short.contains("known closed (21)\n"), "{short}");
    assert!(
        short.contains("  1 older omitted (--all lists them)\n"),
        "{short}"
    );
    assert!(
        short.contains("relay-20") && short.contains("relay-01"),
        "{short}"
    );
    assert!(!short.contains("relay-00"), "{short}");
    assert_eq!(CLOSED_SHOWN, 20);
    let all = render(&report, true);
    assert!(all.contains("relay-00"), "{all}");
    assert!(!all.contains("omitted"), "{all}");
    let exact = render(&report_with((0..20).map(closed_row).collect()), false);
    assert!(
        !exact.contains("omitted"),
        "twenty fit without a cut: {exact}"
    );
}

#[test]
fn evaluate_orders_open_obligations_oldest_first() {
    let lines = [
        claim_line("relay-old", None, 100),
        claim_line("relay-new", None, 300),
    ];
    let orch = session("orch", ORCH, Status::Running, &lines);
    let target = session("target", TARGET, Status::Running, &[]);
    let report = evaluate(None, &[orch, target], Vec::new(), at(400));
    assert_eq!(rows(&report), ["relay-old", "relay-new"]);
    assert!(
        report.unboarded[0]
            .detail
            .contains("no opening, outcome unrecorded")
    );
}

#[test]
fn the_report_prints_every_section_in_order_with_its_count() {
    let mut report = report_with(vec![closed_row(1)]);
    report.unboarded.push(Row {
        kind: Kind::Seat,
        key: "alpha:lead".to_owned(),
        place: format!("uuid {TARGET}  launch tok-1"),
        at: Some(at(4_000)),
        detail: "waiting-user since 1970-01-01T01:06:40Z".to_owned(),
    });
    report.hinted.push((
        closed_row(2),
        Hint {
            item: "R1".to_owned(),
            via: "link",
        },
    ));
    report.unresolved.push(Gap {
        subject: "session x".to_owned(),
        reason: "liveness unknown".to_owned(),
    });
    let text = render(&report, false);
    let headings: Vec<&str> = text
        .lines()
        .filter(|line| {
            !line.starts_with(' ') && !line.starts_with("board:") && !line.starts_with("desk check")
        })
        .collect();
    assert_eq!(
        headings,
        [
            "unboarded (1)",
            "boarded (hint) (1)",
            "known closed (1)",
            "unresolved coverage (1)"
        ]
    );
    assert!(text.starts_with("board: absent\n"), "{text}");
    assert!(text.contains("hint: R1 via link"), "{text}");
    assert!(text.contains("  seat alpha:lead  uuid "), "{text}");
    assert!(
        text.contains("sessions 2 read, 0 stopped, 0 unreadable"),
        "{text}"
    );
    let mut present = report_with(Vec::new());
    present.board = Some((3, at(1_000)));
    assert!(render(&present, false).starts_with("board: 3 items, updated "));
}

#[test]
fn shown_fields_cannot_carry_control_text() {
    let mut report = report_with(Vec::new());
    report.unresolved.push(Gap {
        subject: "session \u{1b}[31mred".to_owned(),
        reason: "line one\nline two".to_owned(),
    });
    let text = render(&report, false);
    assert!(!text.contains('\u{1b}'), "{text:?}");
    assert!(text.contains("line one line two"), "{text:?}");
}

#[test]
fn a_stopped_dispatcher_still_owes_its_relay_to_a_running_target() {
    let lines = [
        owner_ask("ask-1", 100),
        claim_line("relay-1", Some("ask-1"), 110),
    ];
    let orch = session("orch", ORCH, Status::Stopped, &lines);
    let target = session("target", TARGET, Status::Running, &[]);
    let report = evaluate(None, &[orch, target], Vec::new(), at(200));
    assert_eq!(rows(&report), ["ask-1", "relay-1"]);
    let ask = &report.unboarded[0];
    assert_eq!(ask.kind, Kind::OwnerAsk);
    assert_eq!(
        ask.detail, "reply owed, relays relay-1 absent, origin stopped",
        "the owner ask stays a candidate and says where it stands"
    );
    let relay = &report.unboarded[1];
    assert_eq!(relay.kind, Kind::Relay);
    assert!(relay.detail.contains("no opening"), "{}", relay.detail);
    assert!(
        relay.detail.ends_with(", origin stopped"),
        "{}",
        relay.detail
    );
    assert_eq!((report.read, report.stopped, report.unreadable), (1, 1, 0));
    assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
    assert_eq!(report.code(), EXIT_UNBOARDED);
    let named = evaluate(
        Some(&board("RUNNING\nR1 | Waiting on ask-1\n")),
        &[
            session("orch", ORCH, Status::Stopped, &lines),
            session("target", TARGET, Status::Running, &[]),
        ],
        Vec::new(),
        at(200),
    );
    assert_eq!(rows(&named), ["relay-1"], "an exact match moves the ask");
    assert_eq!(named.hinted.len(), 1);
    assert_eq!(named.hinted[0].0.key, "ask-1");
}

#[test]
fn an_unreadable_stopped_journal_is_coverage_and_not_a_stopped_count() {
    let mut orch = session("orch", ORCH, Status::Stopped, &[]);
    orch.journal = Err("journal line 9 is not a record".to_owned());
    let report = evaluate(None, &[orch], Vec::new(), at(200));
    assert_eq!((report.read, report.stopped, report.unreadable), (0, 0, 1));
    assert_eq!(report.unresolved.len(), 1);
    assert_eq!(
        report.unresolved[0].reason,
        "stopped, and journal unreadable: journal line 9 is not a record"
    );
}

fn partial(name: &str, uuid: &str, status: Status, lines: &[String], why: &str) -> SessionInput {
    let mut input = session(name, uuid, status, &[]);
    let events = lines
        .iter()
        .map(|line| Event::parse_line(line.trim_end()).expect("a record"))
        .collect();
    input.journal = Ok(Journal::partial(events, why.to_owned()));
    input
}

#[test]
fn an_unknown_liveness_origin_keeps_its_asks_as_labelled_candidates_and_its_read_reason() {
    let lines: Vec<String> = ["ask-a", "ask-b"]
        .iter()
        .zip(100..)
        .map(|(id, ts)| owner_ask(id, ts))
        .collect();
    let mut orch = session("orch", ORCH, Status::Unknown, &lines);
    let report = evaluate(None, &[orch.clone()], Vec::new(), at(200));
    assert_eq!(rows(&report), ["ask-a", "ask-b"]);
    assert!(
        report
            .unboarded
            .iter()
            .all(|row| row.detail == "reply owed, no relay, origin liveness unknown")
    );
    assert_eq!(report.unresolved.len(), 1);
    assert_eq!(report.unresolved[0].reason, "liveness unknown");
    orch.record_gap = Some("record degraded: the meta is unreadable".to_owned());
    let damaged = evaluate(None, &[orch], Vec::new(), at(200));
    assert!(damaged.unboarded.is_empty());
    assert_eq!(
        damaged.unresolved[0].reason,
        "liveness unknown, and record degraded: the meta is unreadable"
    );
}

#[test]
fn a_partly_read_origin_keeps_its_claims_and_loses_its_asks_to_coverage() {
    let lines: Vec<String> = [
        owner_ask("ask-a", 100),
        owner_ask("ask-b", 101),
        owner_ask("ask-c", 102),
        owner_ask("ask-d", 103),
        owner_ask("ask-e", 104),
        claim_line("relay-1", None, 110),
    ]
    .into();
    let orch = partial(
        "orch",
        ORCH,
        Status::Running,
        &lines,
        "line 7 is not a record",
    );
    let target = session("target", TARGET, Status::Running, &[]);
    let report = evaluate(None, &[orch, target], Vec::new(), at(200));
    assert_eq!(rows(&report), ["relay-1"], "the readable claim stands");
    assert!(
        report.unboarded[0]
            .detail
            .ends_with(", origin journal partly unreadable"),
        "{}",
        report.unboarded[0].detail
    );
    assert_eq!((report.read, report.unreadable), (1, 1));
    assert_eq!(
        report.unresolved.len(),
        1,
        "one read loss is one coverage line: {:?}",
        report.unresolved
    );
    let gap = &report.unresolved[0];
    assert_eq!(gap.subject, "session orch");
    assert_eq!(
        gap.reason,
        "journal partly unreadable, its readable claims are kept and its owner asks and seat states are not proved: line 7 is not a record; 5 owner ask(s) not proved open: ask-a, ask-b, ask-c, ... and 2 more"
    );
}

#[test]
fn a_partly_read_target_proves_no_request_state() {
    let orch = session(
        "orch",
        ORCH,
        Status::Running,
        &[claim_line("relay-1", None, 110)],
    );
    let target = partial(
        "target",
        TARGET,
        Status::Running,
        &[],
        "line 3 is not a record",
    );
    let report = evaluate(None, &[orch, target], Vec::new(), at(200));
    assert!(report.unboarded.is_empty(), "{:?}", report.unboarded);
    assert!(report.closed.is_empty());
    let relay = report
        .unresolved
        .iter()
        .find(|gap| gap.subject.starts_with("relay relay-1"))
        .expect("the relay is coverage");
    assert!(
        relay
            .reason
            .starts_with("the target journal is only partly readable: line 3"),
        "{}",
        relay.reason
    );
}

#[test]
fn a_target_whose_record_lost_its_proof_keeps_the_relay_unresolved() {
    let orch = session(
        "orch",
        ORCH,
        Status::Running,
        &[claim_line("relay-1", None, 110)],
    );
    let mut target = session("target", TARGET, Status::Running, &[]);
    target.record_gap = Some("record degraded: the meta is unreadable".to_owned());
    let report = evaluate(None, &[orch, target], Vec::new(), at(200));
    assert!(report.unboarded.is_empty(), "{:?}", report.unboarded);
    assert!(report.closed.is_empty());
    let relay = report
        .unresolved
        .iter()
        .find(|gap| gap.subject.starts_with("relay relay-1"))
        .expect("the relay is coverage");
    assert!(
        relay
            .reason
            .contains("the target's record: record degraded: the meta is unreadable"),
        "{}",
        relay.reason
    );
}

#[test]
fn an_unrelated_degradation_leaves_the_relay_read_and_names_its_reason_for_the_seats() {
    let orch = session(
        "orch",
        ORCH,
        Status::Running,
        &[claim_line("relay-1", None, 110)],
    );
    let mut target = session("target", TARGET, Status::Running, &[]);
    target.waits = Err("record degraded: duplicate meta key goal at line 4".to_owned());
    let report = evaluate(None, &[orch, target], Vec::new(), at(200));
    assert_eq!(rows(&report), ["relay-1"], "the target's proof stands");
    assert_eq!(report.read, 2);
    let seats = report
        .unresolved
        .iter()
        .find(|gap| gap.subject == "seats of target")
        .expect("the seats are coverage");
    assert!(
        seats.reason.contains("duplicate meta key goal"),
        "{}",
        seats.reason
    );
}

fn meta_snapshot(meta: &str, events: Option<SessionRead>) -> RecordSnapshot {
    RecordSnapshot {
        meta: Some(Meta::parse(meta)),
        meta_read: MetaRead::Parsed,
        events,
        legacy_created_epoch: None,
    }
}

fn complete_events() -> SessionRead {
    SessionRead {
        events: Vec::new(),
        last_active: None,
        pending: Vec::new(),
        cursor: Cursor::default(),
        skipped: Vec::new(),
    }
}

#[test]
fn a_degradation_costs_the_relay_proof_only_when_it_touches_the_proof() {
    let id = "session_id=aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee\n";
    let lost_proof = [
        (
            format!("{id}no equals sign here\n"),
            Some(complete_events()),
        ),
        (
            format!("{id}session_id=aaaaaaaa-bbbb-4ccc-8ddd-000000000000\n"),
            Some(complete_events()),
        ),
    ];
    for (meta, events) in lost_proof {
        let (reason, proof) = degradation(Some(&meta_snapshot(&meta, events)));
        assert!(proof, "{meta:?}: {reason}");
        assert!(!reason.is_empty());
    }
    // Lost journal records leave the record's identity: the strict journal
    // read decides what the journal still proves.
    let (reason, proof) = degradation(Some(&meta_snapshot(id, None)));
    assert!(!proof, "{reason}");
    assert_eq!(reason, "journal records were lost");
    let (reason, proof) = degradation(Some(&meta_snapshot(
        &format!("{id}goal=a\ngoal=b\n"),
        Some(complete_events()),
    )));
    assert!(!proof, "{reason}");
    assert!(reason.contains("duplicate meta key goal"), "{reason}");
    let (reason, proof) = degradation(Some(&meta_snapshot(id, Some(complete_events()))));
    assert!(!proof);
    assert_eq!(reason, "an unusable idle-nudge pin");
    let (reason, proof) = degradation(None);
    assert!(proof);
    assert_eq!(reason, "no durable record");
    let unread = RecordSnapshot::default();
    let (reason, proof) = degradation(Some(&unread));
    assert!(proof);
    assert!(reason.contains("the meta is unreadable"), "{reason}");
}

#[test]
fn a_relay_binds_to_an_owner_ask_only_through_a_bound_attempt_that_cites_it() {
    let attempt = |keys: &[(&'static str, &str)]| -> Event {
        let fields = EventFields {
            actor_session_id: ORCH,
            target_session_id: TARGET,
            target_session_uuid: TARGET,
            ..EventFields::new(
                at(110),
                "boss",
                "relay-attempt",
                "target:lead",
                "relay-1",
                "main",
                "orch",
                "main",
                "target",
                "task",
                "",
            )
        };
        Event::parse_line(tracked::event_line_with(&fields, "launch-1", keys).trim_end())
            .expect("a record")
    };
    assert_eq!(join(&attempt(&[])), Join::Absent);
    assert_eq!(
        join(&attempt(&[
            ("relay_mode", "bound"),
            ("owner_input", "ask-1")
        ])),
        Join::Bound("ask-1".to_owned())
    );
    assert!(matches!(
        join(&attempt(&[("relay_mode", "bound")])),
        Join::Malformed(why) if why.contains("cites no ask")
    ));
    assert!(matches!(
        join(&attempt(&[("relay_mode", "quote"), ("owner_input", "ask-1")])),
        Join::Malformed(why) if why.contains("mode is quote")
    ));
    assert!(matches!(
        join(&attempt(&[("owner_input", "ask-1")])),
        Join::Malformed(why) if why.contains("mode is unrecorded")
    ));
    assert_eq!(join(&attempt(&[("relay_mode", "quote")])), Join::Absent);
}

#[test]
fn an_unbound_or_unheld_owner_join_is_coverage_and_never_an_owner_fact() {
    let lines = [
        owner_ask("ask-1", 100),
        claim_line("relay-1", Some("ask-gone"), 110),
    ];
    let orch = session("orch", ORCH, Status::Running, &lines);
    let target = session("target", TARGET, Status::Running, &[]);
    let report = evaluate(None, &[orch, target], Vec::new(), at(200));
    let ask = report
        .unboarded
        .iter()
        .find(|row| row.kind == Kind::OwnerAsk)
        .expect("the owner ask is still owed");
    assert_eq!(
        ask.detail, "reply owed, no relay",
        "the join names ask-gone"
    );
    let gap = report
        .unresolved
        .iter()
        .find(|gap| gap.reason.starts_with("owner join not read"))
        .expect("the join is coverage");
    assert!(gap.reason.contains("ask-gone"), "{}", gap.reason);
}

#[test]
fn a_journal_index_hands_back_each_claim_and_opening_once_in_order() {
    let lines = [
        claim_line("relay-1", None, 100),
        owner_ask("ask-1", 105),
        claim_line("relay-2", None, 110),
        claim_line("relay-1", None, 120),
    ];
    let journal = journal("orch", ORCH, &lines);
    assert_eq!(journal.attempts, ["relay-1", "relay-2"]);
    let one = journal.relay_events("relay-1");
    assert_eq!(one.len(), 2);
    assert!(
        one.iter()
            .all(|event| event.reference.as_deref() == Some("relay-1"))
    );
    assert_eq!(journal.relay_events("relay-2").len(), 1);
    assert!(journal.relay_events("nothing").is_empty());
    assert_eq!(journal.ask_events("ask-1").len(), 1);
    assert!(journal.ask_events("relay-1").is_empty());
}

#[test]
fn the_board_error_line_cannot_be_altered_by_control_text() {
    let line = board_error_line(
        std::path::Path::new("/s/\u{1b}[31mred\n/desk-record"),
        &"line one\nline two",
    );
    assert!(!line.contains('\u{1b}') && !line.contains('\n'), "{line:?}");
    assert!(
        line.starts_with("desk check: the board record /s/"),
        "{line}"
    );
    assert!(
        line.ends_with("cannot be read: line one line two"),
        "{line}"
    );
}

/// A declaration record with `tail` appended to its fields.
fn declaration_with(ts: i64, actor: &str, value: &str, tail: &str) -> Event {
    let body = crate::state::event_body(
        at(ts),
        actor,
        &crate::state::Declaration {
            value: value.to_owned(),
            reason: "needs the owner for the next decision".to_owned(),
            on: None,
        },
    );
    let line = body.lines().next().expect("a record");
    let line = format!("{}{tail}}}", line.trim_end_matches('}'));
    Event::parse_line(&line).expect("a record")
}

fn declaration(ts: i64, actor: &str, value: &str, launch: Option<&str>) -> Event {
    let tail = launch.map_or_else(String::new, |id| format!(",\"actor_launch_id\":\"{id}\""));
    declaration_with(ts, actor, value, &tail)
}

/// A declaration that carries the routing key a stamping writer leaves: the
/// seat's slot, the session it spoke in by name and, when it has one, by id.
fn routed_declaration(
    ts: i64,
    session: &str,
    id: Option<&str>,
    value: &str,
    launch: Option<&str>,
) -> Event {
    let mut tail = format!(",\"actor_slot\":\"main\",\"actor_session\":\"{session}\"");
    if let Some(id) = id {
        let _ = write!(tail, ",\"actor_session_id\":\"{id}\"");
    }
    if let Some(launch) = launch {
        let _ = write!(tail, ",\"actor_launch_id\":\"{launch}\"");
    }
    declaration_with(ts, "lead", value, &tail)
}

#[test]
fn the_declaration_row_names_its_launch_or_says_it_does_not() {
    let stamped = declaration(100, "lead", "waiting-user", Some("tok-1"));
    assert_eq!(witness_of(&stamped), Witness::Launch("tok-1".to_owned()));
    let unstamped = declaration(100, "lead", "waiting-user", None);
    assert_eq!(witness_of(&unstamped), Witness::Absent);
    let empty = declaration(100, "lead", "waiting-user", Some(""));
    let Witness::Unproved(why) = witness_of(&empty) else {
        panic!("an empty launch name proves nothing");
    };
    assert!(why.contains("present and empty"), "{why}");
}

#[test]
fn a_declaration_is_the_seats_by_its_session_id_and_never_by_its_name_alone() {
    let key = SessionKey::new("alpha", TARGET);
    let newest = |events: &[Event]| {
        keyed_declaration(events, key, "main", "lead").map(|event| event.ts.epoch())
    };
    // The seat's own stamped row.
    let own = [routed_declaration(
        100,
        "alpha",
        Some(TARGET),
        "waiting-user",
        Some("tok-1"),
    )];
    assert_eq!(newest(&own), Some(100));
    // The same name and slot under another session id is another
    // incarnation's row: it supplies nothing.
    let foreign = [routed_declaration(
        100,
        "alpha",
        Some(ORCH),
        "waiting-user",
        Some("tok-1"),
    )];
    assert_eq!(newest(&foreign), None);
    // Beside an older row of the seat itself, the seat's own row is the one read.
    let beside = [
        declaration(90, "lead", "waiting-user", None),
        routed_declaration(100, "alpha", Some(ORCH), "waiting-user", Some("tok-1")),
    ];
    assert_eq!(newest(&beside), Some(90));
    // A foreign row newer than the seat's own cannot end it.
    let newer_foreign = [
        routed_declaration(100, "alpha", Some(TARGET), "waiting-user", Some("tok-1")),
        routed_declaration(150, "alpha", Some(ORCH), "working", Some("tok-0")),
    ];
    assert_eq!(newest(&newer_foreign), Some(100));
    // A row pinned to this id before a rename is still the seat's.
    let renamed = [routed_declaration(
        100,
        "alpha-old",
        Some(TARGET),
        "waiting-user",
        Some("tok-1"),
    )];
    assert_eq!(newest(&renamed), Some(100));
    // A row with no session id is judged by the name alone.
    let legacy = [routed_declaration(
        100,
        "alpha",
        None,
        "waiting-user",
        Some("tok-1"),
    )];
    assert_eq!(newest(&legacy), Some(100));
    let other = [routed_declaration(
        100,
        "other",
        None,
        "waiting-user",
        Some("tok-1"),
    )];
    assert_eq!(newest(&other), None);
}

const ALPHA_META: &str = "session=alpha\nsession_id=aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee\nseat.main=lead\nagent_bin.main=gemini\nlaunch_id.main=tok-1\nlaunch_time.main=90\n";

/// The waits of session `alpha`, found from the entry the fleet read builds
/// out of `events`: the whole route from the journal to the seats.
fn waits_in(events: Vec<Event>) -> Vec<Wait> {
    let snapshot = meta_snapshot(
        ALPHA_META,
        Some(SessionRead {
            events,
            last_active: None,
            pending: Vec::new(),
            cursor: Cursor::default(),
            skipped: Vec::new(),
        }),
    );
    let entry = crate::session::entry_from(
        &snapshot,
        "alpha",
        &crate::session::SessionRuntime::new(Status::Running),
        at(200),
        3600,
    );
    assert!(entry.agent_state_is_exact(), "the fixture reads whole");
    waits_of(&entry, Some(&snapshot), at(200))
}

#[test]
fn the_waits_come_from_the_roster_by_the_stable_key_and_not_from_the_name_only_digest() {
    let launch = Witness::Launch("tok-1".to_owned());
    let own_wait = || routed_declaration(100, "alpha", Some(TARGET), "waiting-user", Some("tok-1"));
    let unproved = |wait: &Wait, phrase: &str| {
        let Witness::Unproved(why) = &wait.witness else {
            panic!("expected an unproved wait, got {:?}", wait.witness);
        };
        assert!(why.contains(phrase), "{why}");
    };

    // The seat's own row waits, and its launch name travels with it.
    let waits = waits_in(vec![own_wait()]);
    assert_eq!(waits.len(), 1, "{waits:?}");
    assert_eq!(
        (waits[0].since, &waits[0].witness),
        (Some(at(100)), &launch)
    );
    assert_eq!(waits[0].launch_id.as_deref(), Some("tok-1"));
    assert_eq!(waits[0].launch_epoch, Some(90));

    // A foreign working row of the same name is newer, and the name-only
    // digest reads it: the seat's own row still waits.
    let waits = waits_in(vec![
        own_wait(),
        routed_declaration(150, "alpha", Some(ORCH), "working", Some("tok-0")),
    ]);
    assert_eq!(waits.len(), 1, "{waits:?}");
    assert_eq!(
        (waits[0].since, &waits[0].witness),
        (Some(at(100)), &launch)
    );

    // A row the seat made under its name before a rename waits, though the
    // digest reads the new name only and shows nothing.
    let waits = waits_in(vec![routed_declaration(
        100,
        "alpha-old",
        Some(TARGET),
        "waiting-user",
        Some("tok-1"),
    )]);
    assert_eq!(waits.len(), 1, "{waits:?}");
    assert_eq!(
        (waits[0].since, &waits[0].witness),
        (Some(at(100)), &launch)
    );

    // A foreign waiting row of the same name looks like a wait to the digest
    // and is no declaration of this seat: coverage, never a row.
    let waits = waits_in(vec![routed_declaration(
        100,
        "alpha",
        Some(ORCH),
        "waiting-user",
        Some("tok-1"),
    )]);
    assert_eq!(waits.len(), 1, "{waits:?}");
    unproved(&waits[0], "no declaration of the seat under its session id");
    let report = evaluate(None, &[with_waits(waits)], Vec::new(), at(200));
    assert!(report.unboarded.is_empty());
    assert_eq!(report.unresolved.len(), 1);
    assert!(
        report.unresolved[0].reason.starts_with(
            "waiting-user, but the journal holds no declaration of the seat under its session id"
        ),
        "{}",
        report.unresolved[0].reason
    );

    // The seat's own newer working row ends its own wait, whatever a foreign
    // waiting row says after it.
    let waits = waits_in(vec![
        own_wait(),
        routed_declaration(120, "alpha", Some(TARGET), "working", Some("tok-1")),
        routed_declaration(150, "alpha", Some(ORCH), "waiting-user", Some("tok-1")),
    ]);
    assert_eq!(waits.len(), 1, "{waits:?}");
    unproved(&waits[0], "is working, not waiting-user");

    // An unstamped row keeps the baseline rule: a keyless declaration of the
    // seat waits and names no launch.
    let waits = waits_in(vec![declaration(100, "lead", "waiting-user", None)]);
    assert_eq!(waits.len(), 1, "{waits:?}");
    assert_eq!(
        (waits[0].since, &waits[0].witness),
        (Some(at(100)), &Witness::Absent)
    );

    // A seat that only works waits for nobody.
    assert!(waits_in(vec![declaration(100, "lead", "working", None)]).is_empty());
}

#[test]
fn a_direct_launch_name_wins_and_a_mismatch_is_unresolved_even_when_the_baseline_would_pass() {
    let bound = |witness: Witness, since: i64, epoch: i64| {
        let wait = Wait {
            witness,
            ..wait(Some(since), Some(epoch))
        };
        evaluate(None, &[with_waits(vec![wait])], Vec::new(), at(200))
    };
    // The baseline alone would leave the same second unresolved: the name binds.
    let same = bound(Witness::Launch("tok-1".to_owned()), 100, 100);
    assert_eq!(rows(&same), ["alpha:lead"]);
    assert!(same.unresolved.is_empty());
    // The baseline alone would bind: the other launch's name does not.
    let other = bound(Witness::Launch("tok-0".to_owned()), 100, 90);
    assert!(other.unboarded.is_empty());
    assert_eq!(other.unresolved.len(), 1);
    assert_eq!(
        other.unresolved[0].reason,
        "waiting-user declared by launch tok-0, not the seat's current launch tok-1"
    );
    // The name needs no baseline: none is read when the row names its launch.
    let wait = Wait {
        witness: Witness::Launch("tok-1".to_owned()),
        ..wait(Some(100), None)
    };
    let no_baseline = evaluate(None, &[with_waits(vec![wait])], Vec::new(), at(200));
    assert_eq!(rows(&no_baseline), ["alpha:lead"]);
    let unproved = bound(
        Witness::Unproved("the journal holds no declaration of the seat".to_owned()),
        100,
        90,
    );
    assert!(unproved.unboarded.is_empty());
    assert!(
        unproved.unresolved[0]
            .reason
            .starts_with("waiting-user, but the journal holds no declaration")
    );
}

fn relayed_ask(id: &str, ts: i64) -> String {
    let fields = EventFields {
        target_session_id: ORCH,
        ..EventFields::new(
            at(ts),
            tracked::CONSOLE_SINK,
            "ask",
            "boss",
            id,
            "",
            "",
            "main",
            "orch",
            "",
            "",
        )
    };
    tracked::event_line_with(&fields, "launch-1", &[("relay_origin", TARGET)])
}

/// An ask an agent opened in `ORCH`'s journal under the id of an owner ask.
fn agent_ask(id: &str, ts: i64) -> String {
    let fields = EventFields {
        actor_session_id: ORCH,
        ..EventFields::new(
            at(ts),
            "boss",
            "ask",
            "target:lead",
            id,
            "main",
            "orch",
            "main",
            "target",
            "",
            "",
        )
    };
    tracked::event_line(&fields)
}

#[test]
fn a_join_cites_one_owner_opening_even_when_it_was_answered_since() {
    let reply = EventFields::new(
        at(150),
        "boss",
        "reply",
        tracked::CONSOLE_SINK,
        "ask-1",
        "main",
        "orch",
        "",
        "",
        "answered",
        "",
    );
    let joined = |ask: Vec<String>| {
        let mut lines = ask;
        lines.push(claim_line("relay-1", Some("ask-1"), 110));
        let orch = session("orch", ORCH, Status::Running, &lines);
        let target = session("target", TARGET, Status::Running, &[]);
        evaluate(None, &[orch, target], Vec::new(), at(200))
    };
    let unread = |report: &Report| {
        report
            .unresolved
            .iter()
            .filter(|gap| gap.reason.starts_with("owner join not read"))
            .count()
    };
    // The owner's own line links the relay exactly when the join is read.
    let linked = |report: &Report| {
        report
            .unboarded
            .iter()
            .filter(|row| row.key == "ask-1" && row.detail.contains("relay-1"))
            .count()
    };
    let open = joined(vec![owner_ask("ask-1", 100)]);
    assert_eq!((unread(&open), linked(&open)), (0, 1));
    let answered = joined(vec![owner_ask("ask-1", 100), tracked::event_line(&reply)]);
    assert_eq!(
        unread(&answered),
        0,
        "a closed owner ask is still an owner ask"
    );
    let foreign = joined(vec![relayed_ask("ask-1", 100)]);
    assert_eq!(unread(&foreign), 1, "a relayed opening is no owner ask");
    assert_eq!(linked(&foreign), 0);
    let repeated = joined(vec![owner_ask("ask-1", 100), owner_ask("ask-1", 101)]);
    assert_eq!(
        (unread(&repeated), linked(&repeated)),
        (1, 0),
        "a repeated opening is no one owner ask, and the owner's line does not claim the relay"
    );
    let beside_an_agent = joined(vec![owner_ask("ask-1", 100), agent_ask("ask-1", 101)]);
    assert_eq!(
        (unread(&beside_an_agent), linked(&beside_an_agent)),
        (1, 0),
        "a second opening of any actor is no one owner ask"
    );
    let absent = joined(Vec::new());
    assert_eq!(unread(&absent), 1);
}

#[test]
fn the_request_index_reads_each_state_once_by_id() {
    let reply = EventFields::new(
        at(150),
        "boss",
        "reply",
        tracked::CONSOLE_SINK,
        "ask-1",
        "main",
        "orch",
        "",
        "",
        "answered",
        "",
    );
    let open = journal(
        "orch",
        ORCH,
        &[owner_ask("ask-1", 100), owner_ask("ask-2", 101)],
    );
    assert_eq!(open.request_word("ask-1"), "pending");
    assert_eq!(open.request_word("ask-2"), "pending");
    assert_eq!(open.request_word("ask-3"), "absent");
    let answered = journal(
        "orch",
        ORCH,
        &[
            owner_ask("ask-1", 100),
            owner_ask("ask-2", 101),
            tracked::event_line(&reply),
        ],
    );
    assert_eq!(answered.request_word("ask-1"), "replied");
    assert_eq!(answered.request_word("ask-2"), "pending");
}

/// The owner's ask in `ORCH`'s chat, with `pane` as the session uuid the
/// console recorded for the pane it was typed in.
fn owner_ask_in(id: &str, pane: &str) -> Event {
    let fields = EventFields {
        target_session_id: ORCH,
        target_session_uuid: pane,
        ..EventFields::new(
            at(100),
            tracked::CONSOLE_SINK,
            "ask",
            "boss",
            id,
            "",
            "",
            "main",
            "orch",
            "",
            "",
        )
    };
    Event::parse_line(tracked::event_line(&fields).trim_end()).expect("a record")
}

#[test]
fn an_owner_opening_names_its_own_chat_or_no_pane_and_never_another_chat() {
    for (pane, expected) in [("", true), (ORCH, true), (TARGET, false)] {
        let event = owner_ask_in("ask-1", pane);
        assert_eq!(is_owner_opening(&event, ORCH), expected, "pane {pane:?}");
    }
    let event = owner_ask_in("ask-1", ORCH);
    assert!(
        !is_owner_opening(&event, TARGET),
        "an opening is the chat of its target session id only"
    );
}

#[test]
fn a_session_without_a_recorded_id_is_coverage_only_when_it_holds_claims_or_runs() {
    let claims = vec![claim_line("relay-1", None, 100)];
    for (index, (lines, status, gap)) in [
        (claims.clone(), Status::Stopped, true),
        (Vec::new(), Status::Running, true),
        (claims, Status::Running, true),
        (Vec::new(), Status::Stopped, false),
    ]
    .into_iter()
    .enumerate()
    {
        let mut input = session("noid", ORCH, status, &lines);
        input.uuid = None;
        let report = evaluate(None, &[input], Vec::new(), at(200));
        let gaps: Vec<&Gap> = report
            .unresolved
            .iter()
            .filter(|gap| gap.subject == "session noid")
            .collect();
        assert_eq!(gaps.len(), usize::from(gap), "case {index}");
        if gap {
            assert!(
                gaps[0].reason.contains("no session id recorded"),
                "case {index}: {}",
                gaps[0].reason
            );
        }
        assert!(report.unboarded.is_empty(), "case {index}");
    }
}

#[test]
fn only_a_running_seat_of_a_whole_record_lists_its_waits() {
    let listed = |input: SessionInput| {
        let report = evaluate(None, &[input], Vec::new(), at(200));
        (report.unboarded.len(), report.unresolved.len())
    };
    let waiting = || with_waits(vec![wait(Some(100), Some(90))]);
    assert_eq!(listed(waiting()), (1, 0), "the positive control");
    let mut stopped = waiting();
    stopped.status = Status::Stopped;
    assert_eq!(listed(stopped), (0, 0), "a stopped session answers no wait");
    let mut damaged = waiting();
    damaged.record_gap = Some("record partly unreadable".to_owned());
    let (rows, gaps) = listed(damaged);
    assert_eq!(rows, 0, "a damaged record proves no wait");
    assert_eq!(
        gaps, 1,
        "the damage is the session's own line, not a seat's"
    );
}
