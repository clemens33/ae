//! `desk check`: the token boundary of a board match, the binding of a wait to
//! its launch, the way a relay target is read, and the sections with their cut.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "a fixture builds its own records and states what it expects"
)]

use super::{
    CLOSED_SHOWN, EXIT_UNBOARDED, Fate, Gap, Hint, Journal, Kind, Report, Row, SessionInput, Wait,
    absent_label, evaluate, fate, hint, mentions, render,
};
use crate::desk::{Desk, parse_input};
use crate::digest::Status;
use crate::events::Event;
use crate::relay::Claim;
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

fn journal(lines: &[String]) -> Journal {
    let bytes: Vec<u8> = lines
        .iter()
        .flat_map(|line| format!("{}\n", line.trim_end()).into_bytes())
        .collect();
    let events = lines
        .iter()
        .map(|line| Event::parse_line(line.trim_end()).expect("a record"))
        .collect();
    Journal { bytes, events }
}

fn session(name: &str, uuid: &str, status: Status, lines: &[String]) -> SessionInput {
    SessionInput {
        name: name.to_owned(),
        uuid: Some(uuid.to_owned()),
        status,
        record_gap: None,
        journal: Ok(journal(lines)),
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
fn a_wait_binds_to_a_launch_that_began_before_its_declaration() {
    for (since, epoch, bound) in [
        (Some(100), Some(90), true),
        (Some(100), Some(100), true),
        (Some(100), Some(101), false),
    ] {
        let input = with_waits(vec![wait(since, epoch)]);
        let report = evaluate(None, &[input], None, at(200));
        assert_eq!(
            report.unboarded.len(),
            usize::from(bound),
            "{since:?} {epoch:?}"
        );
        assert_eq!(
            report.unresolved.len(),
            usize::from(!bound),
            "{since:?} {epoch:?}"
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
            assert!(
                gap.reason.contains("before the seat's current launch"),
                "{}",
                gap.reason
            );
        }
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
        let report = evaluate(None, &[with_waits(vec![wait])], None, at(200));
        assert!(report.unboarded.is_empty(), "{name}");
        let reason = report.unresolved.first().map(|gap| gap.reason.clone());
        missing.push((name, reason.is_some_and(|reason| reason.contains(expected))));
    }
    assert!(missing.iter().all(|(_, found)| *found), "{missing:?}");
    let mut anonymous = with_waits(vec![wait(Some(100), Some(90))]);
    anonymous.uuid = None;
    let report = evaluate(None, &[anonymous], None, at(200));
    assert!(report.unboarded.is_empty());
    assert!(report.unresolved[0].reason.contains("no session id"));
}

#[test]
fn a_seat_state_that_is_not_exact_is_a_gap_never_clean() {
    let mut input = session("alpha", TARGET, Status::Running, &[]);
    input.waits = Err("seat states are not exact".to_owned());
    let report = evaluate(None, &[input], None, at(200));
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
        None,
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
fn a_fleet_that_did_not_pair_is_one_gap() {
    let report = evaluate(
        None,
        &[],
        Some("the fleet read did not pair its records"),
        at(5),
    );
    assert_eq!(report.unresolved.len(), 1);
    assert_eq!(report.unresolved[0].subject, "fleet");
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
    let report = evaluate(None, &[orch, stopped], None, at(200));
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
    let report = evaluate(None, &[orch], None, at(200));
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
        None,
        at(200),
    );
    assert_eq!(bare.code(), EXIT_UNBOARDED);
    let named = evaluate(
        Some(&board("RUNNING\nR1 | Waiting on ask-1\n")),
        &[orch()],
        None,
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
    let report = evaluate(None, &[orch, target], None, at(400));
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
