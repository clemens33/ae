//! Frozen characterisation of [`super::console_items`] at df6842eb.
//!
//! Oracle: the doc comment on `console_items` and the `Kind` docs. `docs/chat.md`
//! Chat asks (the bullet at lines 34–49) states the stored-body, older-core,
//! admission and reseat-speaker rules. It is silent on acceptance, follow-up
//! index, lateness and `via`; those follow the doc comment. Anything else is a
//! behaviour pin of this base.
//!
//! Bug-shaped, pinned, not fixed (#300):
//! - B1: an ask with an empty or missing ref does not read `body_file`.
//! - B2: `thread_items` and `may_close` pass session `""`, so a retire that
//!   bars acceptance for the real session does not bar inside `thread_items`.

use super::{
    ASK_OVER_GAP, Body, Item, Kind, OLD_CORE_GAP, Seat, Via, answers, console_items, may_close,
    thread_items,
};
use crate::console::view::tag;
use crate::events::Event;
use std::cell::RefCell;

const S: &str = "aedev";
const T0: &str = "2026-09-30T06:00:00Z";
const T1: &str = "2026-09-30T06:01:00Z";
const T2: &str = "2026-09-30T06:02:00Z";
const M0: i64 = 1_790_748_000_000_000;
const M1: i64 = 1_790_748_060_000_000;
const M2: i64 = 1_790_748_120_000_000;
const ID: &str = "ae-20260930T060000Z-0000abcd";
const ID_B: &str = "ae-20260930T060000Z-0000bbcd";
const UUID: &str = "1b4e28ba-2fa1-11d2-883f-0016d3cc4321";

fn ev(ts: &str, actor: &str, action: &str, reference: &str, extra: &str) -> Event {
    let line = format!(
        r#"{{"ts":"{ts}","actor":"{actor}","action":"{action}","ref":"{reference}"{extra}}}"#
    );
    Event::parse_line(&line).unwrap()
}

fn caller(slot: &str, pane: &str) -> String {
    format!(
        r#","actor_slot":"{slot}","actor_session":"{S}","caller_server":"/tmp/ae","caller_pane":"{pane}","caller_session_uuid":"{UUID}""#
    )
}

fn me() -> String {
    caller("main", "%1")
}

fn ask_to(id: &str, name: &str, slot: &str, summary: &str) -> Event {
    let extra = format!(
        r#","target":"{name}","target_slot":"{slot}","target_session":"{S}","target_server":"/tmp/ae","target_pane":"%1","target_session_uuid":"{UUID}","summary":"{summary}""#
    );
    ev(T0, "console:local", "ask", id, &extra)
}

fn asked(id: &str, summary: &str) -> Event {
    ask_to(id, "lead", "main", summary)
}

fn reply(ts: &str, id: &str, fields: &str, summary: &str) -> Event {
    let extra = format!(r#","target":"console:local"{fields},"summary":"{summary}""#);
    ev(ts, "lead", "reply", id, &extra)
}

fn accept(ts: &str, id: &str, fields: &str, summary: &str) -> Event {
    let extra = format!(r#","target":"console:local"{fields},"summary":"{summary}""#);
    ev(ts, "lead", "accept", id, &extra)
}

fn retire(slot: &str) -> Event {
    ev(
        T1,
        "lead",
        "retire",
        ID,
        &format!(r#","target_slot":"{slot}""#),
    )
}

fn seat(profile: Option<&str>) -> Seat {
    Seat {
        slot: "main".to_owned(),
        name: "lead".to_owned(),
        profile: profile.map(str::to_owned),
    }
}

fn old(_: &Event) -> Body {
    Body::OldCore
}

fn grammar(origin: &str) -> Via {
    Via::of_grammar(origin)
}

struct Out {
    items: Vec<Item>,
    gaps: Vec<String>,
}

fn draw(
    session: &str,
    seats: &[Seat],
    events: &[Event],
    body: &dyn Fn(&Event) -> Body,
    via: &dyn Fn(&str) -> Via,
) -> Out {
    let mut gaps = Vec::new();
    let items = console_items(session, seats, events, body, via, &mut gaps);
    Out { items, gaps }
}

fn draw_old(events: &[Event]) -> Out {
    draw(S, &[seat(Some("m1"))], events, &old, &grammar)
}

fn check(item: &Item, record: usize, micros: i64, kind: &Kind, body: &str, msg: &str) {
    assert_eq!(item.record, Some(record), "{msg}");
    assert_eq!(item.micros, micros, "{msg}");
    assert_eq!(&item.kind, kind, "{msg}");
    assert_eq!(item.body, body, "{msg}");
}

fn accepted(id: &str) -> Kind {
    Kind::Accepted {
        seat: "lead".to_owned(),
        id: id.to_owned(),
    }
}

#[test]
fn g1_acceptance_draws_one_row_only_while_it_means_something() {
    let me = me();
    let stale = caller("main", "%2");
    let spawned = caller("spawned.0", "%1");
    let gap = format!(r#"{me},"identity_gap":"vacant""#);
    let msg = "g1 acceptance";

    let out = draw_old(&[asked(ID, "q"), accept(T1, ID, &me, "on it")]);
    assert_eq!(out.items.len(), 2, "{msg}");
    check(&out.items[1], 1, M1, &accepted(ID), "on it", msg);
    assert!(out.gaps.is_empty(), "{msg}");

    let out = draw_old(&[
        asked(ID, "q"),
        accept(T1, ID, &me, "on it"),
        accept(T2, ID, &me, "again"),
    ]);
    assert_eq!(out.items.len(), 2, "g1 second accept draws nothing");

    for fields in [&stale, &spawned, &gap] {
        let out = draw_old(&[
            asked(ID, "q"),
            accept(T1, ID, fields, "no"),
            accept(T2, ID, &me, "yes"),
        ]);
        assert_eq!(out.items.len(), 2, "g1 bad accept does not consume");
        check(
            &out.items[1],
            2,
            M2,
            &accepted(ID),
            "yes",
            "g1 bad accept does not consume",
        );
    }

    let out = draw_old(&[
        asked(ID, "q"),
        reply(T1, ID, &me, "a"),
        accept(T2, ID, &me, "late"),
    ]);
    assert_eq!(
        out.items.len(),
        2,
        "g1 accept after admitted reply draws nothing"
    );

    let out = draw_old(&[
        asked(ID, "q"),
        ev(T1, "console:local", "cancel", ID, r#","summary":"by you""#),
        accept(T2, ID, &me, "late"),
    ]);
    assert_eq!(out.items.len(), 2, "g1 accept after close draws nothing");
    assert!(
        matches!(out.items[1].kind, Kind::Closed { .. }),
        "g1 accept after close draws nothing"
    );

    let out = draw_old(&[asked(ID, "q"), retire("main"), accept(T2, ID, &me, "no")]);
    assert_eq!(
        out.items.len(),
        1,
        "g1 accept after target retire draws nothing"
    );
}

#[test]
fn g1_retire_scope_and_missing_threads() {
    let me = me();
    let stale = caller("main", "%2");

    let out = draw_old(&[
        asked(ID, "q"),
        retire("worker.0"),
        accept(T2, ID, &me, "yes"),
    ]);
    assert_eq!(out.items.len(), 2, "g1 other-slot retire does not bar");
    check(
        &out.items[1],
        2,
        M2,
        &accepted(ID),
        "yes",
        "g1 other-slot retire does not bar",
    );

    let events = [asked(ID, "q"), retire("main"), accept(T2, ID, &me, "yes")];
    let out = draw("elsewhere", &[seat(Some("m1"))], &events, &old, &grammar);
    assert_eq!(
        out.items.len(),
        2,
        "g1 retire in another session does not bar"
    );

    // The console asker is the sink, so asker-gone fires only when the ask
    // record itself carries that actor routing key. Synthetic.
    let asker = format!(
        r#","target":"lead","target_slot":"main","target_session":"{S}","target_server":"/tmp/ae","target_pane":"%1","target_session_uuid":"{UUID}","actor_slot":"worker.0","actor_session":"{S}","summary":"q""#
    );
    let out = draw_old(&[
        ev(T0, "console:local", "ask", ID, &asker),
        retire("worker.0"),
        accept(T2, ID, &me, "no"),
    ]);
    assert_eq!(out.items.len(), 1, "g1 synthetic asker retire bars");

    let out = draw_old(&[
        asked(ID, "q"),
        reply(T1, ID, &stale, "x"),
        accept(T2, ID, &me, "yes"),
    ]);
    assert_eq!(
        out.items.len(),
        3,
        "g1 accept after unadmitted reply still draws"
    );
    check(
        &out.items[2],
        2,
        M2,
        &accepted(ID),
        "yes",
        "g1 accept after unadmitted reply still draws",
    );

    assert!(
        draw_old(&[accept(T1, ID, &me, "no")]).items.is_empty(),
        "g1 accept with no thread draws nothing"
    );
    assert_eq!(
        draw_old(&[asked(ID, "q"), accept(T1, ID_B, &me, "no")])
            .items
            .len(),
        1,
        "g1 accept unknown ref draws nothing"
    );
    let mut bare = accept(T1, ID, &me, "no");
    bare.reference = None;
    assert_eq!(
        draw_old(&[asked(ID, "q"), bare]).items.len(),
        1,
        "g1 accept empty ref draws nothing"
    );
    let wrong = format!(r#","target":"lead"{me},"summary":"no""#);
    assert_eq!(
        draw_old(&[asked(ID, "q"), ev(T1, "lead", "accept", ID, &wrong)])
            .items
            .len(),
        1,
        "g1 accept not to console draws nothing"
    );
}

#[test]
fn g1_reask_keeps_the_answer_and_the_drawn_row() {
    let me = me();

    let out = draw_old(&[
        asked(ID, "q"),
        ev(T1, "lead", "cancel", ID, r#","summary":"nope""#),
        accept(T2, ID, &me, "yes"),
    ]);
    assert_eq!(
        out.items.len(),
        2,
        "g1 accept after non-console cancel still draws"
    );
    check(
        &out.items[1],
        2,
        M2,
        &accepted(ID),
        "yes",
        "g1 accept after non-console cancel still draws",
    );

    let out = draw_old(&[
        asked(ID, "q"),
        accept(T1, ID, &me, "first"),
        asked(ID, "again"),
        accept(T2, ID, &me, "second"),
    ]);
    assert_eq!(out.items.len(), 4, "g1 re-ask resets acceptance");
    check(
        &out.items[1],
        1,
        M1,
        &accepted(ID),
        "first",
        "g1 re-ask resets acceptance",
    );
    check(
        &out.items[3],
        3,
        M2,
        &accepted(ID),
        "second",
        "g1 re-ask resets acceptance",
    );
    assert_eq!(out.items[2].micros, M0, "g1 re-ask resets acceptance");

    let out = draw_old(&[
        asked(ID, "q"),
        accept(T1, ID, &me, "on it"),
        reply(T2, ID, &me, "a"),
    ]);
    assert!(
        matches!(
            out.items[2].kind,
            Kind::Answer {
                follow_up: 0,
                late: false,
                speaker: None,
                ..
            }
        ),
        "g1 accept then reply is follow-up 0"
    );

    let out = draw_old(&[asked(ID, "q"), accept(T1, ID, &me, "on it"), retire("main")]);
    // retire() is stamped T1; it sits after the accept in the journal.
    assert_eq!(
        out.items.len(),
        2,
        "g1 later retire keeps the drawn accept row"
    );
    check(
        &out.items[1],
        1,
        M1,
        &accepted(ID),
        "on it",
        "g1 later retire keeps the drawn accept row",
    );

    let extra = format!(r#","target":"console:local"{me}"#);
    let out = draw_old(&[asked(ID, "q"), ev(T1, "lead", "accept", ID, &extra)]);
    assert_eq!(out.items[1].body, "", "g1 acceptance");
}

#[test]
fn g2_ask_records_gap_via_uncertainty_and_collapsed_text() {
    let msg = "g2 ask";
    assert_eq!(OLD_CORE_GAP, "body missing (old core)", "{msg}");
    assert_eq!(
        ASK_OVER_GAP, "full text not shown: stored message over 64 KiB",
        "{msg}"
    );

    let out = draw_old(&[asked(ID, "q")]);
    check(
        &out.items[0],
        0,
        M0,
        &Kind::Asked {
            to: "lead".to_owned(),
            id: ID.to_owned(),
            uncertain: false,
            gap: Some(OLD_CORE_GAP.to_owned()),
            via: None,
        },
        "q",
        msg,
    );
    assert!(out.gaps.is_empty(), "{msg}");
    assert!(!tag(&out.items[0].kind).contains("old core"), "{msg}");

    let reads = RefCell::new(0u32);
    let body = |_: &Event| {
        *reads.borrow_mut() += 1;
        Body::Missing
    };
    let mut bare = asked(ID, "q");
    bare.reference = None;
    bare.body_file = Some("/m/messages/x.txt".to_owned());
    bare.relay_origin = Some(UUID.to_owned());
    let out = draw(S, &[seat(Some("m1"))], &[bare], &body, &grammar);
    assert_eq!(*reads.borrow(), 0, "g2 ask B1 empty ref skips body_file");
    assert_eq!(
        out.items[0].kind,
        Kind::Asked {
            to: "lead".to_owned(),
            id: String::new(),
            uncertain: false,
            gap: Some(OLD_CORE_GAP.to_owned()),
            via: Some(Via::Unavailable),
        },
        "g2 ask B1 empty ref skips body_file"
    );

    let mut stored = asked(ID, "q");
    stored.body_file = Some("/m/messages/x.txt".to_owned());
    let cases = [
        (Body::Missing, "body missing"),
        (Body::Refused("odd".to_owned()), "body refused: odd"),
        (Body::NotUtf8, "body not UTF-8"),
        (Body::Oversized, ASK_OVER_GAP),
        (Body::OldCore, OLD_CORE_GAP),
    ];
    for (read, gap) in cases {
        let reads = RefCell::new(0u32);
        let body = |_: &Event| {
            *reads.borrow_mut() += 1;
            read.clone()
        };
        let out = draw(S, &[seat(None)], &[stored.clone()], &body, &grammar);
        assert_eq!(*reads.borrow(), 1, "{msg}");
        assert_eq!(
            out.gaps,
            [format!(
                "ask to lead — {gap}; its 600-character summary is shown"
            )],
            "{msg}"
        );
        assert!(
            matches!(&out.items[0].kind, Kind::Asked { gap: Some(got), .. } if got == gap),
            "{msg} {gap}"
        );
        assert_eq!(out.items[0].body, "q", "{msg}");
    }

    let reads = RefCell::new(0u32);
    let body = |_: &Event| {
        *reads.borrow_mut() += 1;
        Body::Whole(r#"see [File: "/tmp/notes/a.pdf"]"#.to_owned())
    };
    let out = draw(S, &[seat(None)], &[stored.clone()], &body, &grammar);
    assert_eq!(*reads.borrow(), 1, "{msg}");
    assert!(out.gaps.is_empty(), "{msg}");
    assert_eq!(out.items[0].body, "see [File: a.pdf]", "{msg}");
    assert!(
        matches!(out.items[0].kind, Kind::Asked { gap: None, .. }),
        "{msg}"
    );
}

#[test]
fn g2_collapse_uncertainty_and_via() {
    let msg = "g2 ask";
    let mut legacy = asked(ID, "q");
    // Set after parse so the token's quotes are not JSON.
    legacy.summary = Some(r#"see [Image #1: "/tmp/shot.png"]"#.to_owned());
    let out = draw_old(&[legacy]);
    assert_eq!(out.items[0].body, "see [Image #1]", "{msg}");

    let mut doubt = asked(ID, "plain");
    doubt.unverifiable = Some(String::new());
    let out = draw_old(&[doubt]);
    assert!(
        matches!(
            out.items[0].kind,
            Kind::Asked {
                uncertain: true,
                ..
            }
        ),
        "{msg}"
    );
    let out = draw_old(&[asked(ID, "[unconfirmed] q")]);
    assert!(
        matches!(
            out.items[0].kind,
            Kind::Asked {
                uncertain: true,
                ..
            }
        ),
        "{msg}"
    );

    let mut first = asked(ID, "q");
    first.relay_origin = Some(UUID.to_owned());
    let mut third = asked(ID_B, "r");
    third.relay_origin = Some("nope".to_owned());
    let mut answered = reply(T1, ID, &me(), "a");
    answered.relay_origin = Some(UUID.to_owned());
    let seen = RefCell::new(Vec::new());
    let via = |origin: &str| {
        seen.borrow_mut().push(origin.to_owned());
        Via::of_grammar(origin)
    };
    let events = [first, asked("ae-middle", "m"), third, answered];
    let out = draw(S, &[seat(Some("m1"))], &events, &old, &via);
    assert_eq!(seen.borrow().as_slice(), [UUID, "nope"], "{msg}");
    assert!(
        matches!(
            out.items[0].kind,
            Kind::Asked {
                via: Some(Via::Unavailable),
                ..
            }
        ),
        "{msg}"
    );
    assert!(
        matches!(out.items[1].kind, Kind::Asked { via: None, .. }),
        "{msg}"
    );
    assert!(
        matches!(
            out.items[2].kind,
            Kind::Asked {
                via: Some(Via::Unreadable),
                ..
            }
        ),
        "{msg}"
    );
    let seen = RefCell::new(Vec::new());
    let via = |origin: &str| {
        seen.borrow_mut().push(origin.to_owned());
        Via::Named("orch".to_owned())
    };
    let mut named = asked(ID, "q");
    named.relay_origin = Some(UUID.to_owned());
    let out = draw(S, &[seat(None)], &[named], &old, &via);
    assert_eq!(seen.borrow().as_slice(), [UUID], "{msg}");
    assert!(
        matches!(out.items[0].kind, Kind::Asked { via: Some(Via::Named(ref name)), .. } if name == "orch"),
        "{msg}"
    );
}

#[test]
fn g3_abandoned_cancel_and_other_sink_actions() {
    let msg = "g3 sink arms";
    let me = me();
    let out = draw_old(&[
        asked(ID, "q"),
        ev(
            T1,
            "console:local",
            "delivery-abandoned",
            ID,
            r#","target":"lead","summary":"refused: busy""#,
        ),
        reply(T2, ID, &me, "a"),
    ]);
    assert_eq!(out.items.len(), 3, "{msg}");
    check(
        &out.items[1],
        1,
        M1,
        &Kind::NotDelivered {
            to: "lead".to_owned(),
            id: ID.to_owned(),
        },
        "refused: busy",
        msg,
    );
    assert!(
        matches!(
            out.items[2].kind,
            Kind::Answer {
                follow_up: 0,
                late: false,
                ..
            }
        ),
        "{msg}"
    );

    let out = draw_old(&[ev(
        T0,
        "console:local",
        "cancel",
        ID,
        r#","summary":"by you""#,
    )]);
    check(
        &out.items[0],
        0,
        M0,
        &Kind::Closed { id: ID.to_owned() },
        "by you",
        msg,
    );
    let mut bare = ev(T0, "console:local", "cancel", ID, r#","summary":"x""#);
    bare.reference = None;
    let out = draw_old(&[bare]);
    assert!(
        matches!(out.items[0].kind, Kind::Closed { ref id } if id.is_empty()),
        "{msg}"
    );

    for action in ["send", "accept", "reply"] {
        let extra = r#","target":"console:local","summary":"x""#;
        assert!(
            draw_old(&[ev(T0, "console:local", action, ID, extra)])
                .items
                .is_empty(),
            "{msg} {action}"
        );
    }

    let out = draw_old(&[
        asked(ID, "q"),
        ev(T1, "lead", "cancel", ID, r#","summary":"nope""#),
        reply(T2, ID, &me, "a"),
    ]);
    assert_eq!(out.items.len(), 2, "{msg}");
    assert!(
        matches!(
            out.items[1].kind,
            Kind::Answer {
                follow_up: 0,
                late: false,
                ..
            }
        ),
        "{msg}"
    );
}

#[test]
fn g4_reseat_labels_only_later_replies_of_the_moved_slot() {
    let msg = "g4 reseat";
    let me = me();
    let seats = [
        Seat {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: Some("m1".to_owned()),
        },
        Seat {
            slot: "worker.0".to_owned(),
            name: "colead".to_owned(),
            profile: Some("w1".to_owned()),
        },
    ];
    let labelled = |summary: &str, slot: &str| {
        let moved = ev(
            T1,
            "human",
            "reseat",
            "r",
            &format!(r#","target":"lead","target_slot":"{slot}","summary":"{summary}""#),
        );
        let events = [
            asked(ID, "q"),
            reply(T0, ID, &me, "before"),
            moved,
            reply(T2, ID, &me, "after"),
        ];
        draw(S, &seats, &events, &old, &grammar)
    };
    for summary in [
        "reseated lead to m1",
        "pasted, tool not seen",
        "tool stopped after the reseat",
    ] {
        let out = labelled(summary, "main");
        assert_eq!(out.items.len(), 3, "{msg} {summary}");
        assert!(
            matches!(
                out.items[1].kind,
                Kind::Answer {
                    speaker: None,
                    follow_up: 0,
                    ..
                }
            ),
            "{msg} {summary}"
        );
        assert!(
            matches!(
                out.items[2].kind,
                Kind::Answer { speaker: Some(ref profile), follow_up: 1, late: false, .. }
                    if profile == "m1"
            ),
            "{msg} {summary}"
        );
    }

    let out = labelled("lead reseated to m1", "main");
    assert!(
        matches!(out.items[2].kind, Kind::Answer { speaker: None, .. }),
        "{msg}"
    );
    let out = labelled("reseated lead to m1", "worker.0");
    assert!(
        matches!(out.items[2].kind, Kind::Answer { speaker: None, .. }),
        "{msg}"
    );
    let events = [
        asked(ID, "q"),
        ev(
            T1,
            "human",
            "reseat",
            "r",
            r#","summary":"reseated lead to m1""#,
        ),
        reply(T2, ID, &me, "after"),
    ];
    let out = draw(S, &seats, &events, &old, &grammar);
    assert!(
        matches!(out.items[1].kind, Kind::Answer { speaker: None, .. }),
        "{msg}"
    );
}

#[test]
fn g4_unrecorded_profile_and_which_threads_move() {
    let msg = "g4 reseat";
    let me = me();
    let seats = [seat(Some("m1"))];
    let events = [
        asked(ID, "q"),
        ev(
            T1,
            "human",
            "reseat",
            "r",
            r#","target_slot":"main","summary":"reseated lead to m1""#,
        ),
        reply(T2, ID, &me, "after"),
    ];
    let out = draw(S, &[seat(None)], &events, &old, &grammar);
    assert!(
        matches!(out.items[1].kind, Kind::Answer { speaker: Some(ref profile), .. } if profile == "unrecorded"),
        "{msg}"
    );
    let out = draw(S, &[], &events, &old, &grammar);
    assert!(
        matches!(out.items[1].kind, Kind::Answer { speaker: Some(ref profile), .. } if profile == "unrecorded"),
        "{msg}"
    );

    let events = [
        ev(
            T1,
            "human",
            "reseat",
            "r",
            r#","target_slot":"main","summary":"reseated lead to m1""#,
        ),
        asked(ID, "q"),
        reply(T2, ID, &me, "after"),
    ];
    let out = draw(S, &seats, &events, &old, &grammar);
    assert!(
        matches!(out.items[1].kind, Kind::Answer { speaker: None, .. }),
        "{msg}"
    );
}

#[test]
fn g4_only_the_moved_slots_later_replies_are_labelled() {
    let msg = "g4 reseat";
    let me = me();
    let colead = caller("worker.0", "%1");
    let seats = [
        Seat {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: Some("m1".to_owned()),
        },
        Seat {
            slot: "worker.0".to_owned(),
            name: "colead".to_owned(),
            profile: Some("w1".to_owned()),
        },
    ];
    let events = [
        asked(ID, "q"),
        ask_to(ID_B, "colead", "worker.0", "other"),
        ev(
            T1,
            "human",
            "reseat",
            "r",
            r#","target_slot":"main","summary":"reseated lead to m1""#,
        ),
        reply(T2, ID, &me, "lead"),
        reply(T2, ID_B, &colead, "colead"),
    ];
    let out = draw(S, &seats, &events, &old, &grammar);
    assert!(
        matches!(out.items[2].kind, Kind::Answer { speaker: Some(ref profile), .. } if profile == "m1"),
        "{msg}"
    );
    assert!(
        matches!(out.items[3].kind, Kind::Answer { speaker: None, .. }),
        "{msg}"
    );

    let events = [
        asked(ID, "q"),
        ask_to(ID_B, "lead", "main", "also"),
        ev(
            T1,
            "human",
            "reseat",
            "r",
            r#","target_slot":"main","summary":"reseated lead to m1""#,
        ),
        reply(T2, ID, &me, "one"),
        reply(T2, ID_B, &me, "two"),
    ];
    let out = draw(S, &seats, &events, &old, &grammar);
    assert!(
        matches!(
            out.items[2].kind,
            Kind::Answer {
                speaker: Some(_),
                ..
            }
        ),
        "{msg}"
    );
    assert!(
        matches!(
            out.items[3].kind,
            Kind::Answer {
                speaker: Some(_),
                ..
            }
        ),
        "{msg}"
    );
}

#[test]
fn g4_a_late_reply_after_the_move_is_labelled() {
    let msg = "g4 reseat";
    let me = me();
    let seats = [
        Seat {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: Some("m1".to_owned()),
        },
        Seat {
            slot: "worker.0".to_owned(),
            name: "colead".to_owned(),
            profile: Some("w1".to_owned()),
        },
    ];
    let events = [
        asked(ID, "q"),
        ev(T1, "console:local", "cancel", ID, r#","summary":"by you""#),
        ev(
            T1,
            "human",
            "reseat",
            "r",
            r#","target_slot":"main","summary":"reseated lead to m1""#,
        ),
        reply(T2, ID, &me, "after"),
    ];
    let out = draw(S, &seats, &events, &old, &grammar);
    assert!(
        matches!(
            out.items[2].kind,
            Kind::Answer { follow_up: 0, late: true, speaker: Some(ref profile), .. }
                if profile == "m1"
        ),
        "{msg}"
    );
}

#[test]
fn g5_body_is_read_only_for_stored_asks_and_admitted_replies() {
    let msg = "g5 reads";
    let me = me();
    let log = RefCell::new(Vec::new());
    let body = |event: &Event| {
        log.borrow_mut().push(format!(
            "{}:{}",
            event.action,
            event.reference.as_deref().unwrap_or("-")
        ));
        match event.action.as_str() {
            "ask" => Body::Missing,
            _ if event.summary.as_deref() == Some("big") => Body::Oversized,
            _ => Body::Refused("odd".to_owned()),
        }
    };
    let mut stored = asked(ID, "q");
    stored.body_file = Some("/m/messages/x.txt".to_owned());
    let mut bare = asked(ID_B, "skip");
    bare.reference = None;
    bare.body_file = Some("/m/messages/y.txt".to_owned());
    let events = [
        stored,
        reply(T1, ID, &caller("main", "%2"), "nope"),
        reply(T1, ID, &me, "yes"),
        accept(T2, ID, &me, "on it"),
        bare,
        reply(T2, ID, &me, "big"),
    ];
    let out = draw(S, &[seat(Some("m1"))], &events, &body, &grammar);
    assert_eq!(
        log.borrow().as_slice(),
        [
            "ask:".to_owned() + ID,
            format!("reply:{ID}"),
            format!("reply:{ID}")
        ],
        "{msg}"
    );
    assert!(
        matches!(
            out.items[1].kind,
            Kind::Unadmitted { ref from, ref why, .. } if from == "lead" && why == "stale pane"
        ),
        "{msg}"
    );
    assert_eq!(out.items[1].body, "nope", "{msg}");
    assert_eq!(
        out.gaps,
        [
            "ask to lead — body missing; its 600-character summary is shown".to_owned(),
            "reply from lead — body refused: odd; its 600-character summary is shown".to_owned(),
            "reply from lead — body over 65536 bytes; its 600-character summary is shown"
                .to_owned(),
        ],
        "{msg}"
    );
    let whole = |_: &Event| Body::Whole(r#"see [Image #1: "/tmp/shot.png"]"#.to_owned());
    let out = draw(
        S,
        &[seat(None)],
        &[asked(ID, "q"), reply(T1, ID, &me, "a")],
        &whole,
        &grammar,
    );
    assert_eq!(
        out.items[1].body, r#"see [Image #1: "/tmp/shot.png"]"#,
        "{msg}"
    );
}

#[test]
fn g6_reask_restarts_follow_up_lateness_and_speaker() {
    let msg = "g6 lifecycle";
    let me = me();
    let seats = [
        seat(Some("m1")),
        Seat {
            slot: "worker.0".to_owned(),
            name: "colead".to_owned(),
            profile: Some("w1".to_owned()),
        },
    ];
    let events = [
        asked(ID, "q"),
        reply(T1, ID, &me, "a"),
        ev(T1, "console:local", "cancel", ID, r#","summary":"by you""#),
        asked(ID, "again"),
        reply(T2, ID, &me, "b"),
    ];
    let out = draw(S, &seats, &events, &old, &grammar);
    assert!(
        matches!(
            out.items[4].kind,
            Kind::Answer {
                follow_up: 0,
                late: false,
                speaker: None,
                ..
            }
        ),
        "{msg}"
    );

    let events = [
        asked(ID, "q"),
        ev(
            T1,
            "human",
            "reseat",
            "r",
            r#","target_slot":"main","summary":"reseated lead to m1""#,
        ),
        asked(ID, "again"),
        reply(T2, ID, &me, "b"),
    ];
    let out = draw(S, &seats, &events, &old, &grammar);
    assert!(
        matches!(
            out.items[2].kind,
            Kind::Answer {
                speaker: None,
                follow_up: 0,
                ..
            }
        ),
        "{msg}"
    );
}

#[test]
fn g6_interleaved_threads_stay_apart() {
    let msg = "g6 lifecycle";
    let me = me();
    let colead = caller("worker.0", "%1");
    let seats = [
        seat(Some("m1")),
        Seat {
            slot: "worker.0".to_owned(),
            name: "colead".to_owned(),
            profile: Some("w1".to_owned()),
        },
    ];
    let events = [
        asked(ID, "q"),
        ask_to(ID_B, "colead", "worker.0", "other"),
        reply(T1, ID, &me, "a"),
        reply(T1, ID_B, &colead, "b"),
        ev(T1, "console:local", "cancel", ID, r#","summary":"by you""#),
        ev(
            T2,
            "human",
            "reseat",
            "r",
            r#","target_slot":"main","summary":"reseated lead to m1""#,
        ),
        reply(T2, ID, &me, "late"),
        reply(T2, ID_B, &colead, "more"),
    ];
    let out = draw(S, &seats, &events, &old, &grammar);
    assert!(
        matches!(
            out.items[2].kind,
            Kind::Answer {
                follow_up: 0,
                late: false,
                ..
            }
        ),
        "{msg}"
    );
    assert!(
        matches!(
            out.items[3].kind,
            Kind::Answer {
                follow_up: 0,
                late: false,
                speaker: None,
                ..
            }
        ),
        "{msg}"
    );
    assert!(
        matches!(
            out.items[5].kind,
            Kind::Answer { follow_up: 1, late: true, speaker: Some(ref profile), .. }
                if profile == "m1"
        ),
        "{msg}"
    );
    assert!(
        matches!(
            out.items[6].kind,
            Kind::Answer {
                follow_up: 1,
                late: false,
                speaker: None,
                ..
            }
        ),
        "{msg}"
    );

    let events = [
        asked(ID, "q"),
        reply(T1, ID, &me, "a"),
        reply(T1, ID, &me, "a"),
    ];
    let out = draw_old(&events);
    assert_eq!(out.items[1].record, Some(1), "{msg}");
    assert_eq!(out.items[2].record, Some(2), "{msg}");
    assert_eq!(out.items[1].body, out.items[2].body, "{msg}");
}

#[test]
fn g7_record_index_micros_and_callers_follow_the_same_fold() {
    let msg = "g7 callers";
    let me = me();
    let events = [
        asked(ID, "q"),
        ev(T1, "lead", "state", "waiting-user", r#","summary":"x""#),
        reply(T2, ID, &me, "a"),
    ];
    let out = draw_old(&events);
    assert_eq!(out.items.len(), 2, "{msg}");
    assert_eq!(out.items[0].record, Some(0), "{msg}");
    assert_eq!(out.items[0].micros, M0, "{msg}");
    assert_eq!(out.items[1].record, Some(2), "{msg}");
    assert_eq!(out.items[1].micros, M2, "{msg}");
    assert_eq!(
        answers(S, &events, ID),
        vec![(2, "lead".to_owned())],
        "{msg}"
    );

    let mut gaps = Vec::new();
    let direct = console_items("", &[], &events, &old, &grammar, &mut gaps);
    assert_eq!(thread_items(&events), direct, "{msg}");
    assert_eq!(
        gaps,
        [
            "reply from lead — body missing (old core); its 600-character summary is shown"
                .to_owned()
        ],
        "{msg}"
    );

    let events = [asked(ID, "q"), retire("main"), accept(T2, ID, &me, "on it")];
    let real = draw_old(&events);
    assert_eq!(real.items.len(), 1, "g7 callers B2");
    let plain = thread_items(&events);
    assert!(
        plain
            .iter()
            .any(|item| matches!(item.kind, Kind::Accepted { .. })),
        "g7 callers B2"
    );
    assert_eq!(may_close(&events, ID), Ok(()), "g7 callers B2");
}

#[test]
fn g8_retire_and_moved_reseat_consume_the_record() {
    let msg = "g8 consume";
    let me = me();
    let seats = [seat(Some("m1"))];
    let events = [
        asked(ID, "q"),
        ev(
            T1,
            "console:local",
            "retire",
            ID,
            r#","target_slot":"main""#,
        ),
        accept(T2, ID, &me, "no"),
    ];
    let out = draw(S, &seats, &events, &old, &grammar);
    assert_eq!(out.items.len(), 1, "{msg}");
    assert!(matches!(out.items[0].kind, Kind::Asked { .. }), "{msg}");

    let events = [
        asked(ID, "q"),
        ev(
            T1,
            "console:local",
            "reseat",
            "r",
            r#","target_slot":"main","summary":"reseated lead to m1""#,
        ),
        reply(T2, ID, &me, "after"),
    ];
    let out = draw(S, &seats, &events, &old, &grammar);
    assert_eq!(out.items.len(), 2, "{msg}");
    assert!(
        matches!(out.items[1].kind, Kind::Answer { speaker: Some(ref profile), .. } if profile == "m1"),
        "{msg}"
    );
}
