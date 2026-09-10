#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let parent = ae::usage::claude::parse(data);
    let sidechain = ae::usage::claude::parse(data.get(data.len() / 2..).unwrap_or_default());
    let _ = std::hint::black_box(ae::usage::claude::reduce(&parent, &[sidechain]));
});
