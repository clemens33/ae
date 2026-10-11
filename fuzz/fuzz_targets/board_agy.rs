#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The first byte sizes the chunks, 1..=256, so chunk-boundary paths in
    // the ONE splitter are exercised; the second byte is the flag word
    // (bit0 `--assistant`, bit1 entry 0 history / 1 transcript, bit2 the
    // transcript leg opened the truncated sibling, bit3 rows found, bit4
    // read once); the rest is the transcript. A missing byte is off, and a
    // pre-extension flag keeps its entry: bit1 clear still reads history.
    let (chunk, rest) = match data.split_first() {
        Some((size, rest)) => (usize::from(*size) % 256 + 1, rest),
        None => (1, data),
    };
    let (flag, bytes) = match rest.split_first() {
        Some((flag, rest)) => (*flag, rest),
        None => (0, rest),
    };
    let mut splitter = ae::board::Splitter::new();
    for piece in bytes.chunks(chunk) {
        splitter.feed(piece);
    }
    let binding = ae::board::Binding::default()
        .for_seat("0199c0de-ffff-4890-abcd-ef0123456789")
        .with_assistant(flag & 1 == 1)
        .with_assistant_rows_found(flag & 8 == 8)
        .with_assistant_read_once(flag & 16 == 16);
    let streamed = splitter
        .finish()
        .for_seat("0199c0de-ffff-4890-abcd-ef0123456789")
        .with_assistant(flag & 1 == 1)
        .with_assistant_rows_found(flag & 8 == 8)
        .with_assistant_read_once(flag & 16 == 16);
    let (rows, coverage) = if flag & 2 == 0 {
        ae::board::agy::read_stream(&streamed, "s:seat", "fuzz.jsonl", ae::tool::ToolKind::Agy)
    } else {
        ae::board::agy_transcript::read_stream(
            &streamed,
            "s:seat",
            "agy:0199c0de-ffff-4890-abcd-ef0123456789",
            ae::tool::ToolKind::Agy,
            flag & 4 == 4,
        )
    };
    // The door's path: `feed_each` lends each line to `read_fed`, none kept.
    let mut streaming = ae::board::Splitter::new();
    let mut feed = |each: &mut dyn FnMut(&ae::board::Line)| {
        for piece in bytes.chunks(chunk) {
            streaming.feed_each(piece, &mut |line| each(&line));
        }
        streaming.ending()
    };
    let (seat, source) = ("s:seat", ae::tool::ToolKind::Agy);
    let fed = if flag & 2 == 0 {
        ae::board::agy::read_fed(&mut feed, &binding, seat, "fuzz.jsonl", source)
    } else {
        let file = "agy:0199c0de-ffff-4890-abcd-ef0123456789";
        ae::board::agy_transcript::read_fed(&mut feed, &binding, seat, file, source, flag & 4 == 4)
    };
    assert_eq!((&fed.0, &fed.1), (&rows, &coverage));
    let _ = std::hint::black_box(ae::board::collect(rows));
    let _ = std::hint::black_box(coverage);
});
