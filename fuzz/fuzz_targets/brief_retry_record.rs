#![no_main]

use libfuzzer_sys::fuzz_target;

// The brief-retry record is hand-editable persisted state on the WATCHDOG's
// path: a human, a crashed write or another tool can leave any bytes at that
// name, and the daemon reads it every cycle to decide whether to paste a brief
// into a live agent with the original spawner's authority. The parse is bounded
// and clock-free, so oversize input must be refused rather than parsed, and no
// input may panic.
//
// The second leg is the WRITER's contract: anything the parser accepts must
// render back to bytes the parser accepts identically. A record that parsed
// once and then reads differently would be a brief that changes between
// restarts.
fuzz_target!(|data: &[u8]| {
    let parsed = std::hint::black_box(ae::brief_retry::parse(data));
    if let Ok(record) = parsed {
        let rendered = ae::brief_retry::render(&record);
        let again = ae::brief_retry::parse(rendered.as_bytes());
        assert_eq!(
            Ok(record),
            again,
            "a parsed record must render back to itself"
        );
    }
});
