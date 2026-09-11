#![no_main]

use libfuzzer_sys::fuzz_target;

// The launch-epoch aggregate over a raw session meta. `Meta::parse` does NOT
// reach it: this reducer reads the same hostile document for a different
// question — the newest moment a launch recorded — and folds every row, so both
// the per-row claim reading and the aggregate are driven from here.
fuzz_target!(|data: &[u8]| {
    let _ = std::hint::black_box(ae::inventory::launch_epochs(data));
});
