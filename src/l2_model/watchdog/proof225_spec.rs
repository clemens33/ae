//! Independent journal/fold acceptance for bounded unchanged-proof challenges.

use super::{
    CHILD_ACTIVITY_CAP, DONE_HELD_ACTION, DoneProgress, ProofQuiet, WaitProgress, WaitState,
    done_progress, done_progress_proof, last_watchdog_delivery, quiet_ceiling, wait_progress_proof,
};
use crate::events::Event;
use crate::time::Timestamp;

fn row(at: i64, actor: &str, action: &str, reference: Option<&str>, target: Option<&str>) -> Event {
    let mut event =
        Event::parse_line(r#"{"ts":"2026-11-07T00:00:00Z","actor":"seat","action":"state"}"#)
            .expect("independent event fixture");
    event.ts = Timestamp::from_epoch(at);
    event.actor = actor.to_owned();
    event.action = action.to_owned();
    event.reference = reference.map(str::to_owned);
    event.target = target.map(str::to_owned);
    event
}

fn proof(at: i64, state: &str) -> Event {
    row(at, "seat", "state", Some(state), None)
}
fn challenge(at: i64) -> Event {
    let mut event = row(
        at,
        "watchdog",
        "done-challenge",
        Some("launch-1"),
        Some("seat"),
    );
    event.summary = Some("confirmation 1/2".to_owned());
    event
}
fn held(at: i64, launch: &str) -> Event {
    let mut event = row(at, "watchdog", DONE_HELD_ACTION, Some(launch), Some("seat"));
    event.summary = Some("confirmation 2/2".to_owned());
    event
}
fn fold(events: &[Event], required: u8) -> DoneProgress {
    done_progress(
        events,
        "demo",
        "main",
        "seat",
        Some("launch-1"),
        Timestamp::from_epoch(3000),
        300,
        required,
    )
}

#[test]
fn proof_news_tracks_information_and_actionable_news_in_append_order() {
    let mut events = vec![proof(1000, "blocked")];
    // The older timestamp still arrives AFTER the proof. Append order is the oracle.
    events.push(row(900, "peer", "send", None, Some("seat")));
    let (progress, reading) = wait_progress_proof(
        &events,
        "demo",
        "main",
        "seat",
        Some("launch-1"),
        Timestamp::from_epoch(1400),
        300,
        2,
        WaitState::Blocked,
    );
    assert!(matches!(progress, WaitProgress::ChallengeDue { .. }));
    assert_eq!(reading.proof, Some(Timestamp::from_epoch(1000)));
    assert!(
        reading.unchanged,
        "an informational record is not changed work"
    );
    assert_eq!(
        reading.settle,
        Some(Timestamp::from_epoch(1000)),
        "settle uses the newest epoch, without moving behind proof"
    );
    events.push(row(1400, "peer", "ask", Some("r1"), Some("seat")));
    let (_, changed) = wait_progress_proof(
        &events,
        "demo",
        "main",
        "seat",
        Some("launch-1"),
        Timestamp::from_epoch(1500),
        300,
        2,
        WaitState::Blocked,
    );
    assert!(
        !changed.unchanged,
        "actionable news that a blocked declaration outlives still ends unchanged status"
    );
    events.push(proof(1600, "blocked"));
    let (_, renewed) = wait_progress_proof(
        &events,
        "demo",
        "main",
        "seat",
        Some("launch-1"),
        Timestamp::from_epoch(1700),
        300,
        2,
        WaitState::Blocked,
    );
    assert!(renewed.unchanged);
    assert_eq!(renewed.proof, Some(Timestamp::from_epoch(1600)));
}

#[test]
fn informative_paint_moves_settle_but_not_the_proof_anchor() {
    let events = [
        proof(1000, "done"),
        row(1200, "peer", "send", None, Some("seat")),
        row(1300, "seat", "memo", None, None),
    ];
    let (_, reading) = done_progress_proof(
        &events,
        "demo",
        "main",
        "seat",
        Some("launch-1"),
        Timestamp::from_epoch(1500),
        300,
        2,
    );
    assert_eq!(
        reading,
        ProofQuiet {
            proof: Some(Timestamp::from_epoch(1000)),
            unchanged: true,
            settle: Some(Timestamp::from_epoch(1300))
        }
    );
}

#[test]
fn done_held_fills_only_the_last_slot_after_a_real_answered_challenge() {
    let events = [
        proof(1000, "done"),
        challenge(1300),
        proof(1310, "done"),
        held(1610, "launch-1"),
    ];
    assert_eq!(fold(&events, 2), DoneProgress::Confirmed);
    assert!(
        !matches!(fold(&events, 3), DoneProgress::Confirmed),
        "a held event must not fill multiple remaining slots"
    );
    assert_eq!(
        last_watchdog_delivery(&events, "demo", "main", "seat"),
        Some(Timestamp::from_epoch(1300)),
        "a journal-only held record paints no pane"
    );
}

#[test]
fn invalid_held_records_cannot_create_or_cross_a_done_episode() {
    let mut undelivered = challenge(1300);
    undelivered.summary = Some("[unconfirmed] confirmation 1/2".to_owned());
    let cases = [
        vec![
            proof(1000, "done"),
            undelivered,
            proof(1310, "done"),
            held(1610, "launch-1"),
        ],
        vec![held(1610, "launch-1")],
        vec![proof(1000, "done"), held(1610, "launch-1")],
        vec![proof(1000, "done"), challenge(1300), held(1610, "launch-1")],
        vec![
            proof(1000, "done"),
            challenge(1300),
            proof(1310, "done"),
            held(1610, "different-launch"),
        ],
        vec![
            proof(1000, "done"),
            challenge(1300),
            proof(1310, "done"),
            row(1400, "peer", "ask", Some("new-work"), Some("seat")),
            held(1610, "launch-1"),
        ],
        vec![
            proof(1000, "done"),
            challenge(1300),
            proof(1310, "done"),
            challenge(1610),
            held(1620, "launch-1"),
        ],
    ];
    for (index, events) in cases.iter().enumerate() {
        assert_ne!(
            fold(events, 2),
            DoneProgress::Confirmed,
            "invalid held sequence {index}"
        );
    }
    let events = [
        proof(1000, "done"),
        challenge(1300),
        proof(1310, "done"),
        held(1610, "launch-1"),
        held(1620, "launch-1"),
    ];
    assert_ne!(
        fold(&events, 3),
        DoneProgress::Confirmed,
        "repeated held records cannot build up real confirmations"
    );
}

#[test]
fn quiet_and_child_bounds_use_the_approved_values_without_overflow() {
    assert_eq!(quiet_ceiling(300), 900);
    assert_eq!(quiet_ceiling(0), 0);
    assert_eq!(
        quiet_ceiling(u64::MAX),
        u64::MAX,
        "overflow falls back to one cadence"
    );
    assert_eq!(
        CHILD_ACTIVITY_CAP, 24,
        "lead D3: two-hour child cap at the default cadence"
    );
}

#[test]
fn disabled_knobs_preserve_the_existing_progress_contract() {
    let events = [proof(1000, "done")];
    assert_eq!(
        done_progress(
            &events,
            "demo",
            "main",
            "seat",
            Some("launch-1"),
            Timestamp::from_epoch(2000),
            0,
            2
        ),
        DoneProgress::Confirmed
    );
    let waits = [proof(1000, "waiting-agent")];
    for (cadence, required) in [(0, 2), (300, 0)] {
        let (progress, _) = wait_progress_proof(
            &waits,
            "demo",
            "main",
            "seat",
            Some("launch-1"),
            Timestamp::from_epoch(2000),
            cadence,
            required,
            WaitState::WaitingAgent,
        );
        assert_eq!(progress, WaitProgress::None);
    }
}
