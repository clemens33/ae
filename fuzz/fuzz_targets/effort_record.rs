#![no_main]

use ae::effort::{Record, Stored, stored};
use libfuzzer_sys::fuzz_target;

// One seat's `effort.<slot>` meta record: hand-editable persisted state the
// watchdog will read every cycle. The grammar has one spelling per record, so
// a value that parses must encode back to the very text read, and the meta
// reader must find a present record again under its own slot from its own
// encoding.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Ok(record) = std::hint::black_box(Record::parse(&text)) {
        assert_eq!(
            record.encode(),
            text,
            "a parsed record encodes back to its text"
        );
    }
    for slot in ["main", "spawned.0"] {
        if let Stored::Present(record) = std::hint::black_box(stored(data, slot)) {
            assert_eq!(record.slot, slot);
            let line = format!("effort.{slot}={}", record.encode());
            assert_eq!(stored(line.as_bytes(), slot), Stored::Present(record));
        }
    }
});
