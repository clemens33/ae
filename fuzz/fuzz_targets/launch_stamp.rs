#![no_main]

use libfuzzer_sys::fuzz_target;

// The launch-attempt stamp is hand-editable persisted state on the RESUME path:
// a human, a crashed write or another tool can leave any bytes at that name, and
// a resume reads it before deciding whether a session can be proven gone. The
// reducer is bounded, so oversize input must be refused rather than parsed.
fuzz_target!(|data: &[u8]| {
    let _ = std::hint::black_box(ae::store::parse_launch_attempt(data));
});
