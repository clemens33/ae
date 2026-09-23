#![no_main]

use libfuzzer_sys::fuzz_target;

// events.jsonl summaries: hand-editable hostile persisted state.
// `challenge_named` is the ONE grammar the currency walk and both episode
// folds read a watchdog challenge record's summary by.
fuzz_target!(|data: &[u8]| {
    let summary = String::from_utf8_lossy(data);
    let _ = std::hint::black_box(ae::watchdog::challenge_named(&summary));
});
