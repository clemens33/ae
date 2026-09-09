#![no_main]

use libfuzzer_sys::fuzz_target;

// A bounded tail of a Codex rollout JSONL, the other vendor-written state. The
// first byte carries the one flag the byte cap actually flips — whether the
// tail starts on a record boundary — and the rest is the JSONL. An empty input
// is a boundary-aligned empty tail.
const NOW: i64 = 1_788_858_600;

fuzz_target!(|data: &[u8]| {
    let boundary = data.first().is_none_or(|byte| byte & 1 == 1);
    let bytes = data.get(1..).unwrap_or_default();
    let _ = std::hint::black_box(ae::quota::codex::parse(bytes, boundary, NOW));
});
