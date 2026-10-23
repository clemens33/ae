#![no_main]

use libfuzzer_sys::fuzz_target;

// OpenCode Go usage JSON, a rejection's error body and the shared cache
// record: vendor bytes ae does not control, plus a hand-editable file under
// the state root. First byte selects the parser; the rest is hostile input.
// Parsed twice — once at a fixed clock, once at a clock the next eight bytes
// choose — so window arithmetic sees hostile times as well as hostile bytes.
// The same vendor bytes are also read as an error body: a code that parses
// must be one the cache records back, and no other slug is an outcome.
const NOW: i64 = 1_792_416_000; // 2026-10-19T12:00:00Z

fuzz_target!(|data: &[u8]| {
    let Some((selector, rest)) = data.split_first() else {
        return;
    };
    let mut clock = [0u8; 8];
    for (slot, byte) in clock.iter_mut().zip(rest.iter()) {
        *slot = *byte;
    }
    let chosen = i64::from_le_bytes(clock);
    if *selector == 0 {
        let _ = std::hint::black_box(ae::quota::go::parse(rest, NOW, NOW));
        let _ = std::hint::black_box(ae::quota::go::parse(rest, chosen, NOW));
        let _ = std::hint::black_box(ae::quota::go::parse(rest, NOW, chosen));
        if let Some(code) = ae::quota::go::rejection(rest) {
            assert!(ae::quota::go::is_outcome(&format!(
                "unauthorized:{}",
                code.name()
            )));
        }
    } else {
        let _ = std::hint::black_box(ae::quota::go::parse_cache(rest));
    }
});
