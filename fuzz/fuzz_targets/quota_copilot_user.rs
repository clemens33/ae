#![no_main]

use libfuzzer_sys::fuzz_target;

const NOW: i64 = 1_792_416_000;

fuzz_target!(|data: &[u8]| {
    let mut clock = [0_u8; 8];
    for (slot, byte) in clock.iter_mut().zip(data.iter()) {
        *slot = *byte;
    }
    let chosen = i64::from_le_bytes(clock);
    let _ = std::hint::black_box(ae::quota::copilot::parse(data, NOW, NOW));
    let _ = std::hint::black_box(ae::quota::copilot::parse(data, chosen, NOW));
    let _ = std::hint::black_box(ae::quota::copilot::parse(data, NOW, chosen));
});
