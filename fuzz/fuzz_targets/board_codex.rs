#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The first byte sizes the chunks, 1..=256, so chunk-boundary paths in
    // the ONE splitter are exercised; the second byte is the reply mode
    // (bit 2 the console default, else `& 1` the `--assistant` flag); the rest is
    // the transcript. A missing byte is off.
    let (chunk, rest) = match data.split_first() {
        Some((size, rest)) => (usize::from(*size) % 256 + 1, rest),
        None => (1, data),
    };
    let (replies, bytes) = match rest.split_first() {
        Some((flag, rest)) if flag & 4 == 4 => (ae::board::Replies::ToHuman, rest),
        Some((flag, rest)) if flag & 1 == 1 => (ae::board::Replies::All, rest),
        Some((_, rest)) => (ae::board::Replies::Off, rest),
        None => (ae::board::Replies::Off, rest),
    };
    let mut splitter = ae::board::Splitter::new();
    for piece in bytes.chunks(chunk) {
        splitter.feed(piece);
    }
    let streamed = splitter.finish().with_replies(replies);
    let (rows, coverage) =
        ae::board::codex::read_stream(&streamed, "s:seat", "fuzz.jsonl", ae::tool::ToolKind::Codex);
    // The door's path: `feed_each` lends each line to `read_fed`, none kept.
    let mut streaming = ae::board::Splitter::new();
    let fed = ae::board::codex::read_fed(
        &mut |each| {
            for piece in bytes.chunks(chunk) {
                streaming.feed_each(piece, &mut |line| each(&line));
            }
            streaming.ending()
        },
        &ae::board::Binding::default().with_replies(replies),
        "s:seat",
        "fuzz.jsonl",
        ae::tool::ToolKind::Codex,
    );
    assert_eq!((&fed.0, &fed.1), (&rows, &coverage));
    assert_eq!(streaming.ending(), streamed.ending());
    let _ = std::hint::black_box(ae::board::collect(rows));
    let _ = std::hint::black_box(coverage);
});
