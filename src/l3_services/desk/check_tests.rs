//! `desk check`: the token boundary of a board match, the binding of a wait to
//! its launch, the way a relay target is read, and the sections with their cut.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "a fixture builds its own records and states what it expects"
)]

use super::{
    CLOSED_SHOWN, EXIT_UNBOARDED, Fate, Gap, Hint, Join, Journal, Kind, Report, Row, SessionInput,
    Wait, absent_label, board_error_line, degradation, evaluate, fate, fleet_gaps, hint, join,
    mentions, render,
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
        (format!("{id}"), true),
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
    let Fate::Unresolved(why) = fate(&claim(None), "r1", origin, &none) else {
        panic!("a target no session holds is unresolved");
    };
    assert!(why.contains(TARGET), "{why}");
    let twice = sessions(vec![
        session("one", TARGET, Status::Running, &[]),
        session("two", TARGET, Status::Running, &[]),
    ]);
    let Fate::Unresolved(why) = fate(&claim(None), "r1", origin, &twice) else {
        panic!("a target two sessions hold is unresolved");
    };
    assert!(why.starts_with("2 sessions hold"), "{why}");
    for (status, word) in [(Status::Stopped, "stopped"), (Status::Unknown, "unknown")] {
        let held = sessions(vec![session("t", TARGET, status, &[])]);
        let Fate::Unresolved(why) = fate(&claim(None), "r1", origin, &held) else {
            panic!("a {word} target is unresolved");
        };
        assert!(why.contains(word), "{why}");
    }
    let mut unreadable = session("t", TARGET, Status::Running, &[]);
    unreadable.journal = Err("the journal is a directory".to_owned());
    let Fate::Unresolved(why) = fate(&claim(None), "r1", origin, &[unreadable]) else {
        panic!("an unreadable target journal is unresolved");
    };
    assert!(why.starts_with("the target journal: "), "{why}");
    let silent = sessions(vec![session("t", TARGET, Status::Running, &[])]);
    let Fate::Open(word, label) = fate(&claim(Some("not-delivered")), "r1", origin, &silent) else {
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
    assert_eq!(rows(&report), ["relay-1"], "the owner ask is no obligation");
    assert_eq!(report.unboarded[0].kind, Kind::Relay);
    assert!(
        report.unboarded[0].detail.contains("no opening"),
        "{}",
        report.unboarded[0].detail
    );
    assert_eq!((report.read, report.stopped, report.unreadable), (1, 1, 0));
    let ask = report
        .unresolved
        .iter()
        .find(|gap| gap.subject == "session orch")
        .expect("the idle owner ask is coverage");
    assert!(
        ask.reason
            .starts_with("1 open owner ask(s) in a session stopped, not chased: ask-1"),
        "{}",
        ask.reason
    );
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

#[test]
fn idle_owner_asks_name_three_ids_and_count_the_rest() {
    let lines: Vec<String> = ["ask-a", "ask-b", "ask-c", "ask-d", "ask-e"]
        .iter()
        .zip(100..)
        .map(|(id, ts)| owner_ask(id, ts))
        .collect();
    let orch = session("orch", ORCH, Status::Unknown, &lines);
    let report = evaluate(None, &[orch], Vec::new(), at(200));
    assert!(report.unboarded.is_empty());
    let ask = report
        .unresolved
        .iter()
        .find(|gap| gap.reason.contains("open owner ask(s)"))
        .expect("the idle asks are one coverage line");
    assert!(
        ask.reason
            .contains("5 open owner ask(s) in a session of unknown liveness"),
        "{}",
        ask.reason
    );
    assert!(
        ask.reason.ends_with("ask-a, ask-b, ask-c, ... and 2 more"),
        "{}",
        ask.reason
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
    target.record_gap = Some("record degraded: journal records were lost".to_owned());
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
            .contains("the target's record: record degraded: journal records were lost"),
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

fn complete_events() -> Option<SessionRead> {
    Some(SessionRead {
        events: Vec::new(),
        last_active: None,
        pending: Vec::new(),
        cursor: Cursor::default(),
        skipped: Vec::new(),
    })
}

#[test]
fn a_degradation_costs_the_relay_proof_only_when_it_touches_the_proof() {
    let id = "session_id=aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee\n";
    let lost_proof = [
        (format!("{id}no equals sign here\n"), complete_events()),
        (
            format!("{id}session_id=aaaaaaaa-bbbb-4ccc-8ddd-000000000000\n"),
            complete_events(),
        ),
        (id.to_owned(), None),
    ];
    for (meta, events) in lost_proof {
        let (reason, proof) = degradation(Some(&meta_snapshot(&meta, events)));
        assert!(proof, "{meta:?}: {reason}");
        assert!(!reason.is_empty());
    }
    let (reason, proof) = degradation(Some(&meta_snapshot(
        &format!("{id}goal=a\ngoal=b\n"),
        complete_events(),
    )));
    assert!(!proof, "{reason}");
    assert!(reason.contains("duplicate meta key goal"), "{reason}");
    let (reason, proof) = degradation(Some(&meta_snapshot(id, complete_events())));
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
