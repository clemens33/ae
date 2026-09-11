#![no_main]

use libfuzzer_sys::fuzz_target;

// Session meta: hand-editable persisted state, parsed infallibly into a
// document plus a list of anomalies. Every byte reaches the key/value split.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = std::hint::black_box(ae::meta::Meta::parse(&text));
    let _ = std::hint::black_box(ae::meta::meta_agent_role(data));
});
