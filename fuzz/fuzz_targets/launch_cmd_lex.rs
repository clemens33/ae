#![no_main]

use libfuzzer_sys::fuzz_target;

// The one-simple-command lexer, over a profile command string. It scans a
// `Vec<char>` and slices it by recorded index, so an input that moves a span is
// the interesting one.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = std::hint::black_box(ae::launch_cmd::lex_simple_command(&text));
});
