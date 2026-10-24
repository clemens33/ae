#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    const NOW: i64 = 1_788_870_600;
    let whole = data.len().is_multiple_of(2);

    let boundary = data.first().is_none_or(|byte| byte & 1 == 1);
    let hourly = ae::usage::hourly::parse(data, boundary, whole, NOW, true);
    let _ = std::hint::black_box(hourly.finish(NOW));
    let body = data.get(1..).unwrap_or_default();
    let (head, tail) = body.split_at(body.len() / 2);
    let _ = std::hint::black_box(ae::usage::codex::parse_with_head(head, tail, boundary));
});
