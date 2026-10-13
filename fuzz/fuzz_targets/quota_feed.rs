#![no_main]

use libfuzzer_sys::fuzz_target;

// The statusline feed's two hostile inputs: the payload Claude Code pipes to a
// statusline, and ae's own record of it, which anything on the machine can
// edit. A record that parses must render back to exactly its own bytes.
fuzz_target!(|data: &[u8]| {
    let _ = std::hint::black_box(ae::quota::feed::extract(data));
    if let Ok(record) = ae::quota::feed::parse(data) {
        assert_eq!(
            ae::quota::feed::render(&record)
                .as_deref()
                .map(str::as_bytes),
            Some(data)
        );
    }
});
