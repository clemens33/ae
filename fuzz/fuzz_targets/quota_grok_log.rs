#![no_main]

use libfuzzer_sys::fuzz_target;

// A bounded tail of the grok CLI's shared debug log: vendor-written, mostly
// lines ae must not read. The first byte carries the one flag the byte cap
// flips — whether the tail starts on a line boundary — and the rest is the log.
// Parsed at a fixed clock and at one the input's first eight bytes choose, so
// the window arithmetic sees hostile times as well as hostile bytes. The fixed
// clock is 2026-09-30T12:00:00Z, so the seeds read as fresh, stale and future.
const NOW: i64 = 1_790_769_600;

fuzz_target!(|data: &[u8]| {
    let boundary = data.first().is_none_or(|byte| byte & 1 == 1);
    let bytes = data.get(1..).unwrap_or_default();
    let _ = std::hint::black_box(ae::quota::grok::parse(bytes, boundary, NOW));
    let mut clock = [0u8; 8];
    for (slot, byte) in clock.iter_mut().zip(bytes.iter()) {
        *slot = *byte;
    }
    let _ = std::hint::black_box(ae::quota::grok::parse(
        bytes,
        boundary,
        i64::from_le_bytes(clock),
    ));
});
