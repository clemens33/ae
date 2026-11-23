//! The desk-stale tick's pure judgements: which journal records are a
//! relevant change, and what a send outcome does to the claim.

use std::collections::BTreeMap;

use super::{Journal, Settle, Stale, changed, old, settle};
use crate::desk::Desk;
use crate::events::{Cursor, Event, SessionKey};
use crate::time::Timestamp;

const SESSION: &str = "desk-peer";
const ID: &str = "0199c0de-5555-4890-abcd-ef0123456789";
const UPDATED: i64 = 1_795_164_000;

fn at(offset: i64) -> String {
    Timestamp::from_epoch(UPDATED + offset).to_string()
}

fn by(actor: &str, slot: &str, offset: i64, action: &str, tail: &str) -> Event {
    Event::parse_line(&format!(
        r#"{{"ts":"{}","actor":"{actor}","action":"{action}","actor_slot":"{slot}","actor_session":"{SESSION}"{tail}}}"#,
        at(offset)
    ))
    .expect("fixture event")
}

fn worker(offset: i64, action: &str, tail: &str) -> Event {
    by("gemini:worker", "w1", offset, action, tail)
}

fn state(offset: i64, state: &str) -> Event {
    worker(offset, "state", &format!(r#","ref":"{state}""#))
}

fn ends(events: &[Event]) -> Vec<Cursor> {
    (1..=events.len() as u64)
        .map(|n| Cursor {
            generation: 0,
            offset: n * 100,
        })
        .collect()
}

/// The newest change index, with the first `fresh` records already consumed.
fn newest(events: &[Event], fresh: usize, ack: Option<&str>) -> Option<usize> {
    let ends = ends(events);
    let journal = Journal {
        key: SessionKey::new(SESSION, ID),
        events,
        ends: &ends,
        mark: fresh
            .checked_sub(1)
            .and_then(|last| ends.get(last).copied()),
    };
    changed(&journal, Timestamp::from_epoch(UPDATED), ack)
}

fn judge(events: &[Event], fresh: usize, ack: Option<&str>) -> bool {
    newest(events, fresh, ack).is_some()
}

#[test]
fn each_relevant_state_arriving_after_the_update_is_a_change() {
    for target in ["waiting-user", "blocked", "done"] {
        let events = [state(-100, "working"), state(10, target)];
        assert!(judge(&events, 0, None), "{target}");
    }
    let events = [state(-100, "blocked"), state(10, "working")];
    assert!(!judge(&events, 0, None), "working is no relevant state");
    let events = [state(-100, "working"), state(10, "waiting-agent")];
    assert!(
        !judge(&events, 0, None),
        "waiting-agent is no relevant state"
    );
}

#[test]
fn a_change_at_or_before_the_update_or_before_the_mark_is_not_counted() {
    let events = [state(-100, "working"), state(0, "blocked")];
    assert!(!judge(&events, 0, None), "at the update");
    let events = [state(-100, "working"), state(10, "blocked")];
    assert!(!judge(&events, 2, None), "already consumed");
    assert!(judge(&events, 1, None), "past the mark");
}

#[test]
fn a_same_state_repeat_is_no_change_even_across_the_mark() {
    let events = [
        state(-100, "blocked"),
        worker(10, "state", r#","ref":"blocked","summary":"a new reason""#),
    ];
    assert!(!judge(&events, 1, None));
}

#[test]
fn only_the_orchestrator_mains_done_acknowledgement_is_excluded() {
    let main = |offset, state: &str| {
        by(
            "claude:boss",
            "main",
            offset,
            "state",
            &format!(r#","ref":"{state}""#),
        )
    };
    let ack = [main(-100, "working"), main(10, "done")];
    assert!(
        !judge(&ack, 0, Some("boss")),
        "the overview acknowledgement"
    );
    assert!(judge(&ack, 0, None), "another session's main seat");
    let blocked = [main(-100, "working"), main(10, "blocked")];
    assert!(judge(&blocked, 0, Some("boss")), "the main seat blocked");
}

#[test]
fn a_request_to_a_human_sink_counts_when_it_opens_after_the_update() {
    let ask = |target: &str| worker(10, "ask", &format!(r#","ref":"r-1","target":"{target}""#));
    assert!(judge(&[ask("telegram:42")], 0, None));
    assert!(judge(&[ask("console:local")], 0, None));
    assert!(!judge(&[ask("lead")], 0, None), "an agent target");
    assert!(!judge(&[ask("ae:compact:x")], 0, None), "an ae ledger sink");
    let review = worker(10, "review", r#","ref":"r-2","target":"discord:7""#);
    assert!(judge(&[review], 0, None));
}

#[test]
fn closing_an_established_human_request_counts_by_its_original_target() {
    let open = |target: &str| {
        worker(
            -50,
            "review",
            &format!(r#","ref":"r-1","target":"{target}""#),
        )
    };
    let cancel = worker(10, "cancel", r#","ref":"r-1""#);
    assert!(judge(&[open("telegram:42"), cancel.clone()], 1, None));
    assert!(
        !judge(&[open("lead"), cancel.clone()], 1, None),
        "an agent request"
    );
    assert!(
        !judge(std::slice::from_ref(&cancel), 0, None),
        "a cancel that closes nothing"
    );
    let early = worker(-10, "cancel", r#","ref":"r-1""#);
    assert!(
        !judge(&[open("telegram:42"), early], 0, None),
        "a closure at or before the update"
    );
}

#[test]
fn the_desk_must_be_strictly_older_than_five_minutes_and_not_in_the_future() {
    let updated = Timestamp::from_epoch(UPDATED);
    assert!(!old(updated, UPDATED + 300));
    assert!(old(updated, UPDATED + 301));
    assert!(!old(updated, UPDATED - 1));
}

#[test]
fn a_booked_tick_requires_the_same_still_stale_desk_before_paste() {
    let desk = Desk {
        updated: Timestamp::from_epoch(UPDATED),
        items: Vec::new(),
    };
    let booked = Stale {
        claim: BTreeMap::new(),
        updated: desk.updated,
    };
    assert!(booked.desk_unchanged(Some(&desk), UPDATED + 301));
    assert!(!booked.desk_unchanged(None, UPDATED + 301), "desk removed");
    assert!(
        !booked.desk_unchanged(Some(&desk), UPDATED + 300),
        "same record after the clock moved back to the threshold"
    );

    let replaced = Desk {
        updated: Timestamp::from_epoch(UPDATED + 1),
        ..desk
    };
    assert!(
        !booked.desk_unchanged(Some(&replaced), UPDATED + 302),
        "a newer record is ineligible for this booking even if it is also old"
    );
}

#[test]
fn only_a_typed_pre_paste_refusal_releases_the_claim() {
    let marker = crate::send::RETRYABLE_MARKER;
    assert_eq!(settle(Some(0), ""), Settle::Keep);
    assert_eq!(settle(Some(0), marker), Settle::Keep);
    assert_eq!(settle(Some(1), "submit unverified"), Settle::Keep);
    assert_eq!(settle(None, marker), Settle::Keep);
    assert_eq!(
        settle(Some(75), &format!("refused\n{marker}\n")),
        Settle::Release
    );
    assert_eq!(
        settle(Some(1), &format!("{marker} and more")),
        Settle::Keep,
        "the marker must stand on its own line"
    );
}

#[test]
fn the_tick_represents_the_newest_change_of_the_journal() {
    let events = [
        state(-100, "working"),
        state(10, "blocked"),
        worker(20, "send", r#","summary":"unrelated""#),
        worker(30, "ask", r#","ref":"r-1","target":"telegram:42""#),
        worker(40, "send", r#","summary":"unrelated""#),
    ];
    assert_eq!(newest(&events, 0, None), Some(3));
    assert_eq!(newest(&events, 4, None), None, "consumed through the ask");
}

#[test]
fn a_declaration_counts_only_for_its_own_session_and_launch() {
    let routed = |session: &str, launch: &str, offset, state: &str| {
        Event::parse_line(&format!(
            r#"{{"ts":"{}","actor":"gemini:worker","action":"state","ref":"{state}","actor_slot":"w1","actor_session":"{session}","actor_launch_id":"{launch}"}}"#,
            at(offset)
        ))
        .expect("routed declaration")
    };
    let elsewhere = [
        routed(SESSION, "tok-a", -100, "working"),
        routed("other-session", "tok-a", 10, "blocked"),
    ];
    assert!(!judge(&elsewhere, 0, None), "routed to another session");
    let relaunched = [
        routed(SESSION, "tok-a", -100, "blocked"),
        routed(SESSION, "tok-b", 10, "blocked"),
    ];
    assert!(judge(&relaunched, 1, None), "a new launch in a reused slot");
    let other_id = by(
        "gemini:worker",
        "w1",
        10,
        "state",
        r#","ref":"blocked","actor_session_id":"0199c0de-7777-4890-abcd-ef0123456789""#,
    );
    assert!(!judge(&[other_id], 0, None), "another session's id");
}

#[test]
fn a_legacy_declaration_counts_for_a_seat_and_ignores_reserved_actors() {
    let declared = |actor: &str| {
        Event::parse_line(&format!(
            r#"{{"ts":"{}","actor":"{actor}","action":"state","ref":"blocked"}}"#,
            at(10)
        ))
        .expect("legacy declaration")
    };
    assert!(judge(&[declared("worker")], 0, None));
    for actor in ["ae", "human", "watchdog"] {
        assert!(!judge(&[declared(actor)], 0, None), "{actor} is not a seat");
    }
}
