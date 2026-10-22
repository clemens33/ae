#![no_main]

use ae::autotrust::{DISMISS_RESERVED, Due, dismiss_due_at};
use ae::events::Event;
use libfuzzer_sys::fuzz_target;

const START: &str = "main:start-one:7";
const CLEARED: &str = r#"{"ts":"2023-11-14T22:13:20Z","actor":"watchdog","action":"human-prompt-cleared","target":"lead","target_slot":"main","target_session":"s","summary":"gone"}"#;

// A seat's journal read for its codex panel episode: is the run named for one
// fixed tool start owed its ONE Escape? Each line is one `events.jsonl` record;
// a line the journal parser refuses is skipped, because the strict ledger is
// the caller's. No input may panic, the answer is a function of its records,
// input inside the quiet span or a closed run is never owed, and a reservation
// for every moment the journal names spends the run whatever it holds.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let events: Vec<Event> = text
        .lines()
        .filter_map(|line| Event::parse_line(line).ok())
        .collect();
    let due = |events: &[Event], now: i64, input: Option<i64>| {
        dismiss_due_at(events, "s", "main", "lead", START, now, input)
    };
    // Whichever record supplies the first sight, its moment is reserved here.
    let mut spent = events.clone();
    spent.extend(events.iter().map(|event| {
        let mut reserved = event.clone();
        reserved.action = DISMISS_RESERVED.to_owned();
        reserved.reference = Some(format!("{START}@{}", event.ts.epoch()));
        reserved
    }));
    let mut closed = events.clone();
    closed.push(Event::parse_line(CLEARED).expect("a clear record parses"));
    let newest = events
        .iter()
        .map(|event| event.ts.epoch())
        .max()
        .unwrap_or(0);
    for now in [
        newest,
        newest.saturating_add(59),
        newest.saturating_add(60),
        newest.saturating_add(86_400),
        i64::MIN,
        i64::MAX,
    ] {
        let recent = [now, now.saturating_sub(59), now.saturating_add(1)];
        for input in [None, Some(now.saturating_sub(60)), Some(i64::MIN)]
            .into_iter()
            .chain(recent.map(Some))
        {
            let answer = std::hint::black_box(due(&events, now, input));
            assert_eq!(due(&events, now, input), answer);
            assert!(
                matches!(answer, Due::Yes | Due::No),
                "no doubt without a ledger"
            );
            if input.is_some_and(|at| recent.contains(&at)) {
                assert_eq!(answer, Due::No, "input inside the quiet span holds");
            }
            assert_eq!(due(&spent, now, input), Due::No, "a reserved run is spent");
            assert_eq!(
                due(&closed, now, input),
                Due::No,
                "a closed run is not owed"
            );
        }
    }
});
