#![no_main]

use libfuzzer_sys::fuzz_target;

// R15's strip over hostile record bytes: strict UTF-8, line-ending
// normalization, control strip by decoded codepoint. Both fields are driven —
// the field only selects the error payload, but both const payloads ride.
fuzz_target!(|data: &[u8]| {
    let _ = std::hint::black_box(ae::sanitize::sanitize(data, ae::sanitize::Field::Goal));
    let _ = std::hint::black_box(ae::sanitize::sanitize(data, ae::sanitize::Field::Decision));
});
