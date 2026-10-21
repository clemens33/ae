#![no_main]

use libfuzzer_sys::fuzz_target;

// Claude Code's own settings file: vendor-written state ae does not control and
// re-reads on every `ae quota`. Parsed twice — once at a fixed clock, once at a
// clock the input's first eight bytes choose — so the window arithmetic sees
// hostile times as well as hostile bytes.
const NOW: i64 = 1_788_858_600;

fuzz_target!(|data: &[u8]| {
    // The same file also reports the account's plan; its label is drawn from
    // whatever survived the token gate.
    let parsed = ae::quota::claude::parse(data, NOW);
    if let Some(plan) = parsed.as_ref().ok().and_then(|parsed| parsed.plan.as_ref()) {
        let _ = std::hint::black_box(plan.label());
    }
    let _ = std::hint::black_box(parsed);
    let mut clock = [0u8; 8];
    for (slot, byte) in clock.iter_mut().zip(data.iter()) {
        *slot = *byte;
    }
    let _ = std::hint::black_box(ae::quota::claude::parse(data, i64::from_le_bytes(clock)));
});
