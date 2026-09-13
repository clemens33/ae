#![no_main]

use libfuzzer_sys::fuzz_target;

// Session meta: hand-editable persisted state, parsed infallibly into a
// document plus a list of anomalies. Every byte reaches the key/value split.
// The stopped-rename intent is the same hostile class (a crashed write or a
// hand edit plants it): the same target drives its typed validator, so the
// transaction's recovery carrier has parser coverage before cutover.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = std::hint::black_box(ae::meta::Meta::parse(&text));
    let _ = std::hint::black_box(ae::meta::meta_agent_role(data));
    let _ = std::hint::black_box(ae::rename::parse_intent(data));
});
