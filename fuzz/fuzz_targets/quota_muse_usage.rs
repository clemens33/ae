#![no_main]

use libfuzzer_sys::fuzz_target;

// Muse's `exec --json` stream and the held cache record: vendor bytes ae does
// not control, plus a hand-editable file under the state root. First byte
// selects the parser; the rest is hostile input. The stream is parsed at a
// fixed clock and at a clock the next eight bytes choose, so window arithmetic
// sees hostile times as well as hostile bytes, and a cache record that parses
// must render back to a record that parses to the same value.
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
        let _ = std::hint::black_box(ae::quota::muse::parse(rest, NOW));
        let _ = std::hint::black_box(ae::quota::muse::parse(rest, chosen));
        if let Ok(allowance) = ae::quota::muse::extract(rest) {
            let _ = std::hint::black_box(allowance.rows(chosen));
        }
    } else if let Some(cached) = ae::quota::muse::parse_cache(rest) {
        let again = ae::quota::muse::parse_cache(cached.render().as_bytes());
        assert_eq!(again.as_ref(), Some(&cached));
        let _ = std::hint::black_box(cached.due("/h/.config/muse", chosen));
        let _ = std::hint::black_box(cached.rows(chosen));
    }
});
