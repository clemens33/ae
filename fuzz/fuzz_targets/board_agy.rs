#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The first byte sizes the chunks, 1..=256, so chunk-boundary paths in
    // the ONE splitter are exercised; the second byte is the `--assistant`
    // flag (`& 1`); the rest is the transcript. A missing byte is off.
    let (chunk, rest) = match data.split_first() {
        Some((size, rest)) => (usize::from(*size) % 256 + 1, rest),
        None => (1, data),
    };
    let (assistant, bytes) = match rest.split_first() {
        Some((flag, rest)) => (flag & 1 == 1, rest),
        None => (false, rest),
    };
    let mut splitter = ae::board::Splitter::new();
    for piece in bytes.chunks(chunk) {
        splitter.feed(piece);
    }
    let (rows, coverage) = ae::board::agy::read_stream(
        &splitter
            .finish()
            .for_seat("0199c0de-ffff-4890-abcd-ef0123456789")
            .with_assistant(assistant),
        "s:seat",
        "fuzz.jsonl",
        ae::tool::ToolKind::Agy,
    );
    let _ = std::hint::black_box(ae::board::collect(rows));
    let _ = std::hint::black_box(coverage);
});
