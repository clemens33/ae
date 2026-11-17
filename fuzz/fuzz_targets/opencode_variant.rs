#![no_main]

use ae::opencode_variant::{request_of, saved_for};
use libfuzzer_sys::fuzz_target;

// The saved-variant reader over OpenCode's `model.json`: the first line is the
// model word a profile asks for, the rest is the file's body. Neither may
// panic, and a word that splits must key the lookup it names.
fuzz_target!(|data: &[u8]| {
    let split = data.iter().position(|byte| *byte == b'\n');
    let (word, body) = match split {
        Some(at) => (&data[..at], &data[at + 1..]),
        None => (data, &data[..0]),
    };
    let word = String::from_utf8_lossy(word);
    if let Some(request) = std::hint::black_box(request_of(&word)) {
        std::hint::black_box(saved_for(body, &request.key));
    }
    std::hint::black_box(saved_for(body, &word));
});
