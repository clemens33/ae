#![no_main]

use libfuzzer_sys::fuzz_target;

// The identity v2 INI parser, over one whole config text. `read_identity` reads
// the file and hands the parser a `str`, so lossy conversion puts every input
// byte in play as the one thing the parser can see.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = std::hint::black_box(ae::config::parse_identity(&text));
});
