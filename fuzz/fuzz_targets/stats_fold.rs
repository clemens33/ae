#![no_main]

use std::cell::Cell;
use std::io::{BufRead, BufReader, Cursor, Read};
use std::rc::Rc;

use ae::stats::{Fold, Reading, Reason, Scoped, Source, SourceGap, ThisSession, fold};
use ae::usage::{Line, LineReader, Pump};
use libfuzzer_sys::fuzz_target;

const S: &str = "11111111-1111-4111-8111-111111111111";
const T: &str = "22222222-2222-4222-8222-222222222222";

/// Up to four sources, split at NUL. The third part is a meta, not a journal,
/// and that source replays the first journal under whatever identity it proves.
fn sources(data: &[u8]) -> Vec<Source> {
    let parts: Vec<&[u8]> = data.split(|byte| *byte == 0).take(4).collect();
    let first = parts.first().copied().unwrap_or_default();
    parts
        .iter()
        .enumerate()
        .map(|(index, part)| match index {
            0 => Source::live(
                "a",
                format!("session_id={S}\n").as_bytes(),
                Ok(part.to_vec()),
            ),
            1 => Source::archive(T, Ok(part.to_vec())),
            2 => Source::live("c", part, Ok(first.to_vec())),
            _ => Source::archive(S, Ok(part.to_vec())),
        })
        .collect()
}

fn balanced(scoped: &Scoped) {
    assert!(scoped.balanced(), "{scoped:?}");
}

/// A buffered reader that counts the bytes consumed from it, and only those.
struct Metered<R> {
    reader: R,
    consumed: Rc<Cell<usize>>,
}

impl<R: Read> Read for Metered<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buffer)
    }
}

impl<R: BufRead> BufRead for Metered<R> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.reader.fill_buf()
    }

    fn consume(&mut self, count: usize) {
        self.reader.consume(count);
        self.consumed.set(self.consumed.get() + count);
    }
}

/// The first journal read the streamed way: lines framed by a resumable reader
/// at a fuzzed buffer size, work per pump and line cap, then folded.
fn streamed(data: &[u8]) {
    let first = data.split(|byte| *byte == 0).next().unwrap_or_default();
    let size = data.len();
    let capacity = 1 + size % 17;
    let work = 1 + u64::try_from(size / 3).unwrap_or(0) % 23;
    let cap = [3 + size % 40, 300, 4096][(size / 7) % 3];
    let meta = format!("session_id={S}\n");

    let consumed = Rc::new(Cell::new(0));
    let reader = Metered {
        reader: BufReader::with_capacity(capacity, Cursor::new(first)),
        consumed: Rc::clone(&consumed),
    };
    let mut lines = LineReader::new(reader, cap);
    let Ok(mut reading) = Reading::live("a", meta.as_bytes(), u64::MAX) else {
        panic!("a plain meta opens");
    };
    let mut seen: Vec<Option<Vec<u8>>> = Vec::new();
    loop {
        let before = consumed.get();
        let step = lines
            .pump(work, &mut |line| match line {
                Line::Text(text) => {
                    seen.push(Some(text.to_vec()));
                    assert!(reading.line(text).is_ok(), "no ceiling was set");
                }
                Line::Over => {
                    seen.push(None);
                    reading.over_long();
                }
            })
            .expect("a memory reader does not fail");
        let spent = consumed.get() - before;
        assert!(
            spent <= usize::try_from(work).unwrap_or(usize::MAX),
            "a pump stays inside its work"
        );
        if let Pump::Done(count) = step {
            assert_eq!(usize::try_from(count).unwrap_or(usize::MAX), first.len());
            break;
        }
        assert!(spent > 0, "a pump that is not done made progress");
    }

    let mut reference: Vec<&[u8]> = first.split(|byte| *byte == b'\n').collect();
    if reference.last().is_some_and(|line| line.is_empty()) {
        reference.pop();
    }
    let expected: Vec<Option<Vec<u8>>> = reference
        .iter()
        .map(|line| (line.len() <= cap).then(|| line.to_vec()))
        .collect();
    assert_eq!(seen, expected, "framing equals an independent split");

    // The fold of the lines that fit is the whole-bytes fold of those lines; the
    // refused ones are one named gap, counted once each.
    let overs = u64::try_from(seen.iter().filter(|line| line.is_none()).count()).unwrap_or(0);
    let kept: Vec<u8> = seen
        .iter()
        .flatten()
        .flat_map(|line| line.iter().copied().chain([b'\n']))
        .collect();
    let mut streamed = Fold::default();
    streamed.add_reading(reading);
    let streamed = streamed.finish(Some(S));
    let whole = fold(vec![Source::live("a", meta.as_bytes(), Ok(kept))], Some(S));
    let limit_gaps = |gaps: &[ae::stats::Gap]| -> u64 {
        gaps.iter()
            .filter(|gap| gap.reason == Reason::LineLimit)
            .map(|gap| gap.count)
            .sum()
    };
    assert_eq!(limit_gaps(&streamed.all.gaps), overs);
    assert_eq!(limit_gaps(&whole.all.gaps), 0);
    assert_eq!(streamed.all.total, whole.all.total);
    assert_eq!(streamed.all.harness, whole.all.harness);
    assert_eq!(streamed.all.joint, whole.all.joint);
    assert_eq!(streamed.all.range, whole.all.range);
    let rest = |gaps: &[ae::stats::Gap]| -> Vec<ae::stats::Gap> {
        gaps.iter()
            .filter(|gap| gap.reason != Reason::LineLimit)
            .cloned()
            .collect()
    };
    assert_eq!(rest(&streamed.all.gaps), rest(&whole.all.gaps));
    balanced(&streamed.all);

    // A ceiling on what the fold keeps: the charge never falls, a line that
    // would pass the room is refused before it is kept and the source whole,
    // and what is admitted stays inside the limit. The same lines read with no
    // ceiling say what the refused line would have charged.
    let limit = 4096 + u64::try_from(size).unwrap_or(0) * 7 % 30_000;
    let mut capped = Fold::with_retained(limit);
    let Ok(mut reading) = Reading::live("a", meta.as_bytes(), capped.room()) else {
        panic!("the source charge fits every limit tried");
    };
    let Ok(mut free) = Reading::live("a", meta.as_bytes(), u64::MAX) else {
        panic!("a plain meta opens");
    };
    let mut last = reading.charge();
    let mut refused = false;
    for line in expected.iter().flatten() {
        let result = reading.line(line);
        assert!(free.line(line).is_ok(), "no ceiling was set");
        if result.is_err() {
            assert_eq!(reading.charge(), last, "a refusal keeps nothing new");
            assert!(free.charge() > limit, "a refusal means over the room");
            refused = true;
            break;
        }
        assert!(reading.charge() >= last, "the charge never falls");
        assert_eq!(reading.charge(), free.charge(), "admission prices as free");
        assert!(reading.charge() <= limit, "an admitted line fits the room");
        last = reading.charge();
    }
    if refused {
        capped.add(reading.fail(SourceGap::Retained));
    } else {
        assert!(reading.charge() <= limit);
        capped.add_reading(reading);
        assert!(capped.room() <= limit);
    }
    let view = capped.finish(Some(S));
    balanced(&view.all);
    if refused {
        assert_eq!(view.all.total, 0, "a refused source leaves nothing");
    }
}

// Journal bytes are hostile: any can sit in a live session or a kept archive.
// Every table sums to the total, in both scopes, and the order sources were
// read in is no evidence. The first journal also goes the streamed way.
fuzz_target!(|data: &[u8]| {
    let view = fold(sources(data), Some(S));
    balanced(&view.all);
    let ThisSession::Ready(current) = &view.this else {
        panic!("a session id was given");
    };
    balanced(current);
    assert!(current.total <= view.all.total);
    let reversed = fold(sources(data).into_iter().rev().collect(), Some(S));
    assert_eq!(view, reversed);
    let _ = std::hint::black_box(view.all.gaps.iter().map(|gap| gap.line()).count());
    streamed(data);
});
