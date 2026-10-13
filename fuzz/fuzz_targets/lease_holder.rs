#![no_main]

use libfuzzer_sys::fuzz_target;

// The writer lease's holder record is hand-editable persisted state: an `ae app`
// writes it into a lock file, but a human, a crashed write or another tool can
// leave any bytes there, and every refused writer reads it to say where the
// holder sits. The reader is bounded (HOLDER_CAP), so oversize input must be
// refused rather than parsed, and no input may panic.
//
// The second leg is the writer's contract: the grammar has exactly one spelling
// per (session, window), so anything the parser accepts must encode back to the
// very bytes it was given.
fuzz_target!(|data: &[u8]| {
    let parsed = std::hint::black_box(ae::console::holder::parse(data));
    if let Some((session, window)) = parsed {
        let encoded = ae::console::holder::encode(&session, &window);
        assert_eq!(
            encoded.as_deref(),
            Some(data),
            "a parsed record must encode back to its bytes"
        );
    }
});
