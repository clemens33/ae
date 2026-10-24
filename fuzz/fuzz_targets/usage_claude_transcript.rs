#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    const NOW: i64 = 1_788_870_600;
    let whole = data.len().is_multiple_of(2);

    let boundary = data.first().is_none_or(|byte| byte & 1 == 1);
    let hourly = ae::usage::hourly::parse(data, boundary, whole, NOW, false);
    let _ = std::hint::black_box(hourly.finish(NOW));
    let parent = ae::usage::claude::parse(data);
    let sidechain = ae::usage::claude::parse(data.get(data.len() / 2..).unwrap_or_default());
    let _ = std::hint::black_box(ae::usage::claude::reduce(&parent, &[sidechain]));
});
