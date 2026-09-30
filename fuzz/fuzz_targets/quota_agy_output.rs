#![no_main]

use libfuzzer_sys::fuzz_target;

// agy's stdout, read back from the on-demand call's scratch capture: vendor
// bytes ae does not control. The whole input is one `--version` answer and one
// `/quota` report, the report parsed at a fixed clock and at one the input's
// first eight bytes choose, so the window arithmetic sees hostile times as well
// as hostile bytes. The fixed clock is 2026-10-01T00:00:00Z, so the measured
// seed reads as fresh.
const NOW: i64 = 1_790_812_800;

fuzz_target!(|data: &[u8]| {
    let _ = std::hint::black_box(ae::quota::agy::version(data));
    let _ = std::hint::black_box(ae::quota::agy::supports_quota(data));
    let _ = std::hint::black_box(ae::quota::agy::parse(data, NOW));
    let mut clock = [0u8; 8];
    for (slot, byte) in clock.iter_mut().zip(data.iter()) {
        *slot = *byte;
    }
    let _ = std::hint::black_box(ae::quota::agy::parse(data, i64::from_le_bytes(clock)));
});
