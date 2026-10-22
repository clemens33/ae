#![no_main]

use ae::attention::Reason;
use ae::events::{Event, REACHABLE_AGAIN, UNREACHABLE};
use ae::session::alert_reason_in;
use libfuzzer_sys::fuzz_target;

const SEAT: &str = r#""target":"lead","target_slot":"main","target_session":"s""#;

fn record(action: &str, summary: &str) -> Event {
    Event::parse_line(&format!(
        r#"{{"ts":"2023-11-14T22:13:20Z","actor":"watchdog","action":"{action}",{SEAT},"summary":"{summary}"}}"#
    ))
    .expect("a synthetic record parses")
}

// A seat's journal folded for the alert its entry shows, the ONE unreachable
// failure included. Each line is one `events.jsonl` record; a line the journal
// parser refuses is skipped, because the strict ledger is the caller's. No
// input may panic, the fold is a function of its records, a failure booked
// last outranks a prompt's naming and its clear and yields only to a dead pane,
// and only the seat's own record, a landed delivery or the watchdog's
// reachable-again clear end it.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let events: Vec<Event> = text
        .lines()
        .filter_map(|line| Event::parse_line(line).ok())
        .collect();
    let fold = |events: &[Event]| alert_reason_in(events, "s", "main", "lead");
    let read = std::hint::black_box(fold(&events));
    assert_eq!(fold(&events), read);
    let failure = record(
        "alert",
        &format!("{UNREACHABLE} — 3 undelivered attempts (idle 5m)"),
    );
    let after = |tail: &[Event]| {
        let mut grown = events.clone();
        grown.push(failure.clone());
        grown.extend_from_slice(tail);
        fold(&grown)
    };
    let own = Event::parse_line(
        r#"{"ts":"2023-11-14T22:13:20Z","actor":"lead","action":"state","actor_slot":"main","actor_session":"s","summary":"working"}"#,
    )
    .expect("an own record parses");
    let named = record("human-prompt", "Warnings · 1 of 1 · Startup — press: esc");
    assert_eq!(
        after(&[]),
        Some(Reason::Stale),
        "a failure booked last stands"
    );
    assert_eq!(
        after(&[named.clone()]),
        Some(Reason::Stale),
        "over a naming"
    );
    assert_eq!(
        after(&[named, record("human-prompt-cleared", "gone")]),
        Some(Reason::Stale),
        "and over its clear"
    );
    assert_eq!(
        after(&[record("send", "hi")]),
        None,
        "a landed delivery ends it"
    );
    assert_eq!(
        after(&[record("send", "[unconfirmed] hi")]),
        Some(Reason::Stale),
        "an unconfirmed one does not"
    );
    assert_eq!(after(&[own]), None, "the seat's own record ends it");
    assert_eq!(
        after(&[record(
            "alert-cleared",
            &format!("{REACHABLE_AGAIN} — nudge landed")
        )]),
        None,
        "the reachable-again clear ends it"
    );
    assert_eq!(after(&[record("alert", "dead pane")]), Some(Reason::Dead));
});
