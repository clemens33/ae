#![no_main]

use libfuzzer_sys::fuzz_target;

/// The one-line cap of the door: a longer complete line is covered, never parsed.
const LINE_CAP: usize = 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    // The first byte sizes the chunks, 1..=256, so chunk-boundary paths in
    // the ONE splitter are exercised; the second byte is the flag word (bit 2 the
    // console default, else `& 1` the `--assistant` flag, bit 7 the raw text
    // mode); the rest is the transcript. A missing byte is off.
    let (chunk, rest) = match data.split_first() {
        Some((size, rest)) => (usize::from(*size) % 256 + 1, rest),
        None => (1, data),
    };
    let (flag, bytes) = match rest.split_first() {
        Some((flag, rest)) => (*flag, rest),
        None => (0, rest),
    };
    let replies = if flag & 4 == 4 {
        ae::board::Replies::ToHuman
    } else if flag & 1 == 1 {
        ae::board::Replies::All
    } else {
        ae::board::Replies::Off
    };
    let text = if flag & 0x80 == 0x80 {
        ae::board::TextMode::Raw
    } else {
        ae::board::TextMode::Trimmed
    };
    let mut splitter = ae::board::Splitter::new();
    for piece in bytes.chunks(chunk) {
        splitter.feed(piece);
    }
    let streamed = splitter.finish().with_replies(replies).with_text(text);
    let (rows, coverage) = ae::board::claude::read_stream(
        &streamed,
        "s:seat",
        "fuzz.jsonl",
        ae::tool::ToolKind::Claude,
    );
    // The door's path: `feed_each` lends each line to `read_fed`, none kept.
    let mut streaming = ae::board::Splitter::new();
    let fed = ae::board::claude::read_fed(
        &mut |each| {
            for piece in bytes.chunks(chunk) {
                streaming.feed_each(piece, &mut |line| each(&line));
            }
            streaming.ending()
        },
        &ae::board::Binding::default()
            .with_replies(replies)
            .with_text(text),
        "s:seat",
        "fuzz.jsonl",
        ae::tool::ToolKind::Claude,
    );
    assert_eq!((&fed.0, &fed.1), (&rows, &coverage));
    assert_eq!(streaming.ending(), streamed.ending());

    // The same stream read in the other text mode: the same rows in the same
    // roles, and the trimmed text is the raw text trimmed.
    let other_text = match text {
        ae::board::TextMode::Raw => ae::board::TextMode::Trimmed,
        ae::board::TextMode::Trimmed => ae::board::TextMode::Raw,
    };
    let (other_rows, other_coverage) = ae::board::claude::read_stream(
        &streamed.with_text(other_text),
        "s:seat",
        "fuzz.jsonl",
        ae::tool::ToolKind::Claude,
    );
    assert_eq!(&other_coverage, &coverage);
    assert_eq!(other_rows.len(), rows.len());
    for (mine, theirs) in rows.iter().zip(&other_rows) {
        assert_eq!(
            (mine.ts, mine.role, mine.offset, mine.generation),
            (theirs.ts, theirs.role, theirs.offset, theirs.generation)
        );
        let (trimmed, raw) = match text {
            ae::board::TextMode::Trimmed => (&mine.body, &theirs.body),
            ae::board::TextMode::Raw => (&theirs.body, &mine.body),
        };
        assert_eq!(trimmed.as_str(), raw.trim());
    }

    // The count of lines that are not JSON equals an independent count of the
    // complete in-cap lines that are not UTF-8 or do not parse, in every mode.
    let pieces: Vec<&[u8]> = bytes.split(|byte| *byte == b'\n').collect();
    let complete = pieces.split_last().map_or(&[][..], |(_, head)| head);
    let not_json = complete
        .iter()
        .filter(|line| line.len() <= LINE_CAP)
        .filter(|line| !std::str::from_utf8(line).is_ok_and(|text| ae::json::parse(text).is_ok()))
        .count();
    let reported = coverage
        .iter()
        .find_map(|item| {
            item.reason
                .strip_suffix(" lines not JSON")
                .or_else(|| item.reason.strip_suffix(" line not JSON"))
                .and_then(|count| count.parse::<usize>().ok())
        })
        .unwrap_or(0);
    assert_eq!(reported, not_json);

    // The dispatch role proves the whole content, so its normalised body is the
    // rendering whatever the text mode. Every body also drives the recognizer, so
    // it stays reachable although the reader owns its one production call.
    for row in &rows {
        let proves = ae::tool::ToolKind::Claude.is_guided_dispatch(&row.body);
        if row.role == ae::board::Role::Dispatch {
            assert!(proves);
        }
        // #171: drive the wrapper recognizer over every produced row body, so the
        // helper stays reachable although `hidden` owns its one production call.
        let _ = std::hint::black_box(ae::board::claude::is_wrapped_ae_turn(&row.body));
    }
    let _ = std::hint::black_box(ae::board::collect(rows));
    let _ = std::hint::black_box(coverage);
});
