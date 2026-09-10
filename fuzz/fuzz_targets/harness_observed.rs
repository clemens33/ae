#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = std::hint::black_box(ae::harness_state::decode_idle(&text));
    let _ = std::hint::black_box(ae::harness_state::observed_from_option(&text));
});
