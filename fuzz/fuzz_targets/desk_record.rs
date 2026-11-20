#![no_main]

use ae::desk::{Desk, diff, encode, parse, parse_input};
use ae::time::Timestamp;
use libfuzzer_sys::fuzz_target;

// The board record and the orchestrator's input for it: hand-editable
// persisted state the chat, the app and the Telegram bridge read on every
// pass. The stored grammar has one spelling per board, so a record that parses
// must encode back to the very bytes read. An input that parses must encode
// into a record that parses back to the same items, and its items differ from
// themselves by nothing.
fuzz_target!(|data: &[u8]| {
    if let Ok(desk) = std::hint::black_box(parse(data)) {
        assert_eq!(encode(&desk), data, "a parsed record encodes back to itself");
    }
    if let Ok(items) = std::hint::black_box(parse_input(data)) {
        let desk = Desk {
            updated: Timestamp::from_epoch(0),
            items,
        };
        let stored = encode(&desk);
        let again = parse(&stored).expect("an encoded board parses");
        assert_eq!(again, desk, "the encoded board parses back to itself");
        assert!(diff(&desk.items, &again.items).is_empty());
    }
});
