//! Independent acceptance tests for #225 informational traffic.
//! Install as a child module of watchdog, byte-identically.

use super::{
    DoneProgress, QuietKind, WaitProgress, WaitState, declaration_current, done_progress,
    ends_quiet, latest_relevant_event, quiet_reason, wait_progress,
};
use crate::events::Event;
use crate::time::Timestamp;

fn row(at: i64, actor: &str, action: &str, reference: Option<&str>, target: Option<&str>) -> Event {
    let mut event =
        Event::parse_line(r#"{"ts":"2026-11-07T00:00:00Z","actor":"seat","action":"state"}"#)
            .expect("the independent journal specimen parses");
    event.ts = Timestamp::from_epoch(at);
    event.actor = actor.to_owned();
    event.action = action.to_owned();
    event.reference = reference.map(str::to_owned);
    event.target = target.map(str::to_owned);
    event
}

fn declared(at: i64, state: &str) -> Event {
    row(at, "seat", "state", Some(state), None)
}

fn challenge(at: i64, state: &str) -> Event {
    let mut event = row(
        at,
        "watchdog",
        if state == "done" {
            "done-challenge"
        } else {
            "wait-challenge"
        },
        Some("launch-1"),
        Some("seat"),
    );
    event.summary = Some(if state == "done" {
        "confirmation 1/2".to_owned()
    } else {
        format!("{state} confirmation 1/2")
    });
    event
}

fn done(events: &[Event], now: i64) -> DoneProgress {
    done_progress(
        events,
        "demo",
        "main",
        "seat",
        Some("launch-1"),
        Timestamp::from_epoch(now),
        300,
        2,
    )
}

fn wait(events: &[Event], now: i64, state: WaitState) -> WaitProgress {
    wait_progress(
        events,
        "demo",
        "main",
        "seat",
        Some("launch-1"),
        Timestamp::from_epoch(now),
        300,
        2,
        state,
    )
}

#[test]
fn informational_records_preserve_done_on_the_shared_currency_surface() {
    for (actor, action, target) in [
        ("peer", "send", Some("seat")),
        ("seat", "send", Some("peer")),
        ("seat", "memo", None),
        ("seat", "reply", Some("peer")),
        ("seat", "retire", Some("child")),
        ("seat", "cancel", Some("peer")),
    ] {
        let informational = row(1500, actor, action, None, target);
        assert!(
            !ends_quiet("done", &informational, actor == "seat", &[]),
            "{actor} {action}"
        );
        let events = [declared(1000, "done"), informational];
        let selected = latest_relevant_event(&events, "demo", "main", "seat")
            .expect("the declaration stays selectable");
        assert_eq!(
            quiet_reason(&selected),
            Some(QuietKind::Done),
            "{actor} {action}"
        );
        assert!(declaration_current(&selected), "{actor} {action}");
    }
}

#[test]
fn actionable_work_still_ends_done_and_unknown_actions_fail_toward_a_challenge() {
    for (actor, action, target) in [
        ("peer", "ask", Some("seat")),
        ("peer", "review", Some("seat")),
        ("peer", "reply", Some("seat")),
        ("seat", "ask", Some("peer")),
        ("seat", "review", Some("peer")),
        ("seat", "spawn", Some("child")),
        ("seat", "accept", Some("peer")),
        ("peer", "future-action", Some("seat")),
    ] {
        let actionable = row(1500, actor, action, Some("request-1"), target);
        assert!(
            ends_quiet("done", &actionable, actor == "seat", &[]),
            "{actor} {action}"
        );
        let events = [declared(1000, "done"), actionable];
        assert_eq!(done(&events, 2000), DoneProgress::None, "{actor} {action}");
        assert!(
            latest_relevant_event(&events, "demo", "main", "seat")
                .and_then(|selected| quiet_reason(&selected))
                .is_none(),
            "{actor} {action}"
        );
    }
}

#[test]
fn human_messages_and_own_new_declarations_keep_their_existing_authority() {
    for state in ["done", "waiting-agent", "waiting-user", "blocked"] {
        for actor in ["telegram:42", "discord:42"] {
            assert!(ends_quiet(
                state,
                &row(1500, actor, "send", None, Some("seat")),
                false,
                &[]
            ));
        }
        assert!(ends_quiet(
            state,
            &row(1500, "console:local", "ask", Some("r1"), Some("seat")),
            false,
            &[]
        ));
        assert!(ends_quiet(state, &declared(1500, "working"), true, &[]));
    }
}

#[test]
fn a_confirmed_done_survives_the_sanitized_f576_information_sequence() {
    let events = [
        declared(1000, "done"),
        challenge(1300, "done"),
        declared(1310, "done"),
        challenge(1610, "done"),
        declared(1620, "done"),
        row(1800, "lead", "send", None, Some("seat")),
        row(1900, "seat", "send", None, Some("lead")),
        row(2000, "seat", "memo", None, None),
    ];
    assert_eq!(done(&events, 20_000), DoneProgress::Confirmed);
    let selected = latest_relevant_event(&events, "demo", "main", "seat").expect("a current proof");
    assert_eq!(selected.event.ts, Timestamp::from_epoch(1620));
    assert_eq!(quiet_reason(&selected), Some(QuietKind::Done));
    assert!(declaration_current(&selected));
}

#[test]
fn informational_records_do_not_remove_the_spacing_earned_by_a_real_wait_proof() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        for (actor, action, target) in [
            ("peer", "send", Some("seat")),
            ("seat", "send", Some("peer")),
            ("seat", "memo", None),
            ("seat", "reply", Some("peer")),
            ("seat", "retire", Some("child")),
            ("seat", "cancel", Some("peer")),
        ] {
            let events = [
                declared(1000, state.as_str()),
                challenge(1300, state.as_str()),
                declared(1310, state.as_str()),
                row(1320, actor, action, None, target),
            ];
            assert_eq!(
                wait(&events, 1610, state),
                WaitProgress::Provisional {
                    confirmations: 1,
                    required: 2
                },
                "{state:?}: {actor} {action}"
            );
            assert!(
                matches!(
                    wait(&events, 1910, state),
                    WaitProgress::ChallengeDue { .. }
                ),
                "the bounded existing fold still has a due opportunity"
            );
        }
    }
}

#[test]
fn a_real_answer_ends_waiting_agent_but_an_unrelated_answer_does_not() {
    let start = [
        row(900, "seat", "ask", Some("request-1"), Some("peer")),
        declared(1000, "waiting-agent"),
    ];
    for (reference, should_end) in [("request-1", true), ("request-2", false)] {
        let mut events = start.to_vec();
        events.push(row(1400, "peer", "reply", Some(reference), Some("seat")));
        assert_eq!(
            wait(&events, 1500, WaitState::WaitingAgent) == WaitProgress::None,
            should_end,
            "{reference}"
        );
        let selected =
            latest_relevant_event(&events, "demo", "main", "seat").expect("relevant event");
        assert_eq!(quiet_reason(&selected).is_none(), should_end, "{reference}");
    }
}

#[test]
fn informational_currency_uses_routing_keys_even_when_display_names_disagree() {
    let own = Event::parse_line(r#"{"ts":"2026-11-07T00:00:00Z","actor":"former-name","action":"state","ref":"done","actor_slot":"main","actor_session":"demo"}"#).expect("routed declaration");
    let own_memo = Event::parse_line(r#"{"ts":"2026-11-07T00:01:00Z","actor":"former-name","action":"memo","actor_slot":"main","actor_session":"demo"}"#).expect("routed memo");
    let unrelated_ask = Event::parse_line(r#"{"ts":"2026-11-07T00:02:00Z","actor":"peer","action":"ask","target":"seat","target_slot":"main","target_session":"another"}"#).expect("other session request");
    let events = [own, own_memo, unrelated_ask];
    let selected =
        latest_relevant_event(&events, "demo", "main", "seat").expect("our routed proof");
    assert!(selected.is_own);
    assert_eq!(quiet_reason(&selected), Some(QuietKind::Done));
    assert!(declaration_current(&selected));
}
