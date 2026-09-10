#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let boundary = data.first().is_none_or(|byte| byte & 1 == 1);
    let _ = std::hint::black_box(ae::usage::codex::parse(
        data.get(1..).unwrap_or_default(),
        boundary,
    ));
});
