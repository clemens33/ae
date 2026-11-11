#![no_main]

use std::collections::HashSet;

use ae::memo::{OVERVIEW_TEXT_CHARS, OVERVIEW_TOPICS, View, records, view};
use libfuzzer_sys::fuzz_target;

fn has(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

// `memo.tsv` is hand-editable, so every read of it is a read of hostile bytes.
// The other views only have to be total. The bare `memo read` overview is
// checked against what the requirement says, by a different route than the
// code takes: the shown records come from a REVERSE scan (the latest record of
// each topic, until the newest topics are used up), the shape is a header line,
// a text line and a blank line per record, and a text is whole or cut to
// exactly the cap with its last character the ellipsis, even when the cut
// reads bytes that are not UTF-8. The hint line is there exactly when the
// output hides something, and then it names the counts; without it the output
// is the full read, byte for byte.
fuzz_target!(|data: &[u8]| {
    let all = view(data, &View::All(None));
    let first = records(data)
        .next()
        .map(|record| String::from_utf8_lossy(record.topic).into_owned());
    let _ = std::hint::black_box(view(data, &View::All(first)));
    let tail = usize::from(data.first().copied().unwrap_or(0) % 8);
    let _ = std::hint::black_box(view(data, &View::Tail(tail)));

    let out = view(data, &View::Overview);
    let found: Vec<_> = records(data).collect();
    let mut seen: HashSet<&[u8]> = HashSet::new();
    let mut shown = Vec::new();
    for record in found.iter().rev() {
        if seen.insert(record.topic) && shown.len() < OVERVIEW_TOPICS {
            shown.push(record);
        }
    }
    shown.reverse();
    let topics = seen.len();

    let lines: Vec<&[u8]> = out.split(|byte| *byte == b'\n').collect();
    let hinted = match lines.len() % 3 {
        1 => false,
        2 => true,
        _ => panic!("an overview is whole records and at most one hint line"),
    };
    let body = if hinted {
        lines.len() - 2
    } else {
        lines.len() - 1
    };
    assert_eq!(body, shown.len() * 3, "one record per newest topic");

    let mut cut = false;
    for (at, record) in shown.iter().enumerate() {
        let header = lines[3 * at];
        let text = lines[3 * at + 1];
        assert!(lines[3 * at + 2].is_empty(), "a blank line ends a record");
        let lead = [record.ts, " — ".as_bytes(), record.author].concat();
        assert!(header.starts_with(&lead), "the header names ts and author");
        let source = String::from_utf8_lossy(record.text);
        if text == record.text {
            assert!(
                source.chars().count() <= OVERVIEW_TEXT_CHARS,
                "an uncut text fits the cap"
            );
        } else {
            cut = true;
            let kept = std::str::from_utf8(text).expect("a cut text is characters");
            assert_eq!(kept.chars().count(), OVERVIEW_TEXT_CHARS);
            assert!(kept.ends_with('…'));
            assert!(source.chars().count() > OVERVIEW_TEXT_CHARS);
            assert!(source.starts_with(&kept[..kept.len() - '…'.len_utf8()]));
        }
    }

    assert_eq!(hinted, shown.len() < found.len() || cut);
    if hinted {
        let hint = lines[lines.len() - 2];
        let counts = format!(
            "newest record of {} of {topics} topics, {} records in all",
            shown.len(),
            found.len()
        );
        assert!(has(hint, counts.as_bytes()), "the hint names the counts");
        let cap = format!("long texts are cut at {OVERVIEW_TEXT_CHARS} characters");
        assert_eq!(has(hint, cap.as_bytes()), cut, "the hint names a cut");
    } else {
        assert_eq!(out, all, "an overview that hides nothing is the full read");
    }
});
