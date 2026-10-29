#![no_main]

use ae::events::Event;
use ae::wait_on::{Moved, Target, moved};
use libfuzzer_sys::fuzz_target;

const WAITER: (&str, &str, &str) = ("main", "lead", "lead");
const SEATS: [(&str, &str, &str); 3] = [
    ("main", "lead", "lead"),
    ("worker.0", "colead", "colead"),
    ("spawned.0", "hand", "hand"),
];

fn booking(found: &Moved) -> Event {
    Event::parse_line(&format!(
        r#"{{"ts":"2023-11-14T22:13:20Z","actor":"watchdog","action":"target-moved","target":"lead","target_slot":"main","target_session":"s","ref":"{}","summary":"{}"}}"#,
        found.ordinal,
        found.class.summary()
    ))
    .expect("a synthetic booking parses")
}

// What the seat a `waiting-agent --on` declaration names did since that
// wait began: the fold behind the watchdog's `target-moved` nudge. Each
// line is one `events.jsonl` record; a line the journal parser refuses is
// skipped, because the strict ledger is the caller's. No input may panic,
// the fold is a function of its records, a move it reports is a real
// transition inside the current wait (ordinal at least one), and a
// watchdog booking of that ordinal addressed to the waiter, appended,
// books it without minting another transition.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let events: Vec<Event> = text
        .lines()
        .filter_map(|line| Event::parse_line(line).ok())
        .collect();
    let fold = |events: &[Event], raw: &str| {
        let target = Target::parse(raw, "s").expect("a fixed target parses");
        moved(events, "s", WAITER, &target, &SEATS)
    };
    for raw in ["hand", "lead", "ghost", "@owner", "owner:colead"] {
        std::hint::black_box(fold(&events, raw));
    }
    let read = std::hint::black_box(fold(&events, "colead"));
    assert_eq!(fold(&events, "colead"), read);
    let Some(found) = read else {
        return;
    };
    assert!(
        found.ordinal >= 1,
        "a reported move transitioned: {found:?}"
    );
    let mut grown = events.clone();
    grown.push(booking(&found));
    let booked = fold(&grown, "colead").expect("a booking keeps the move");
    assert_eq!(
        booked.ordinal, found.ordinal,
        "a booking mints no transition"
    );
    assert!(
        booked.booked >= found.ordinal,
        "a delivered booking books its ordinal: {booked:?} after {found:?}"
    );
});
