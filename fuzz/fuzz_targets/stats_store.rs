#![no_main]

// One frozen oracle is shared with the integration tests; see fuzz/README.md.
#[path = "../../tests/it/stats_store_s1_spec.rs"]
mod spec;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    spec::fuzz_case(data);
});
