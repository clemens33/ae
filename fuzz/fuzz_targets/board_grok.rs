#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The first byte sizes the chunks, 1..=256, so chunk-boundary paths in
    // the ONE splitter are exercised; the rest is the transcript.
    let (chunk, bytes) = match data.split_first() {
        Some((size, rest)) => (usize::from(*size) % 256 + 1, rest),
        None => (1, data),
    };
    let mut splitter = ae::board::Splitter::new();
    for piece in bytes.chunks(chunk) {
        splitter.feed(piece);
    }
    let (rows, coverage) = ae::board::grok::read_stream(
        &splitter.finish(),
        "s:seat",
        "fuzz.jsonl",
        ae::tool::ToolKind::Grok,
    );
    let _ = std::hint::black_box(ae::board::collect(rows));
    let _ = std::hint::black_box(coverage);
});
