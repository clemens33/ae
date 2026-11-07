//! Existing framing/admission assertions rehomed without changing their bodies.
#![allow(clippy::expect_used, reason = "retained acceptance assertions")]
use crate::stats::{Reason, SourceGap, View};
use crate::stats_fixture as f;
use crate::usage::{Line, LineReader, Pump};
use std::cell::Cell;
use std::io::{self, BufRead, BufReader, Cursor, Read};
use std::rc::Rc;
fn cert(n: u32) -> Vec<u8> {
    f::birth(
        f::S,
        n,
        &f::fields("codex", None, Some("sol"), Some("high")),
    )
    .into_bytes()
}
fn gap(view: &View, reason: SourceGap) {
    assert!(
        view.all
            .gaps
            .iter()
            .any(|gap| gap.reason == Reason::Source(reason)),
        "missing {reason:?}: {:?}",
        view.all.gaps
    );
}
fn reference(bytes: &[u8], cap: usize) -> Vec<Option<Vec<u8>>> {
    let mut lines: Vec<_> = bytes.split(|byte| *byte == b'\n').collect();
    if lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines
        .into_iter()
        .map(|line| (line.len() <= cap).then(|| line.to_vec()))
        .collect()
}

struct Metered<R> {
    reader: R,
    consumed: Rc<Cell<usize>>,
}
impl<R: Read> Read for Metered<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.reader.read(buffer)?;
        self.consumed.set(self.consumed.get() + count);
        Ok(count)
    }
}
impl<R: BufRead> BufRead for Metered<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.reader.fill_buf()
    }
    fn consume(&mut self, count: usize) {
        self.reader.consume(count);
        self.consumed.set(self.consumed.get() + count);
    }
}

#[test]
fn framing_and_resumed_pumps_match_independent_split_with_exact_work_bound() {
    let mut giant = vec![b'x'; 8193];
    giant.extend(b"\nOK\n");
    let cases = [
        b"".to_vec(),
        b"\n".to_vec(),
        b"a\nb\n".to_vec(),
        b"last".to_vec(),
        b"\r\n\n\xff".to_vec(),
        b"abc\nabcd\nabcdefgh\nZ".to_vec(),
        giant,
    ];
    for bytes in &cases {
        for cap in [0, 1, 3, 8] {
            let expected = reference(bytes, cap);
            for capacity in [1, 2, 7, 64 * 1024] {
                let mut reader = BufReader::with_capacity(capacity, Cursor::new(bytes));
                let mut once = Vec::new();
                let count = crate::usage::each_line(&mut reader, cap, &mut |line| {
                    once.push(match line {
                        Line::Text(text) => Some(text.to_vec()),
                        Line::Over => None,
                    });
                })
                .expect("in-memory read");
                assert_eq!(count, bytes.len() as u64);
                assert_eq!(once, expected, "one-shot cap={cap}, capacity={capacity}");
                for work in [1, 2, 7, 64 * 1024] {
                    let consumed = Rc::new(Cell::new(0));
                    let reader = Metered {
                        reader: BufReader::with_capacity(capacity, Cursor::new(bytes)),
                        consumed: Rc::clone(&consumed),
                    };
                    let mut reader = LineReader::new(reader, cap);
                    let mut actual = Vec::new();
                    let mut done = false;
                    for _ in 0..bytes.len() + 2 {
                        let before = consumed.get();
                        let result = reader
                            .pump(work, &mut |line| {
                                actual.push(match line {
                                    Line::Text(text) => Some(text.to_vec()),
                                    Line::Over => None,
                                });
                            })
                            .expect("in-memory pump");
                        assert!(
                            consumed.get() - before
                                <= usize::try_from(work).expect("small fixture work fits usize"),
                            "pump exceeded exact work bound, including discarded overlong bytes"
                        );
                        if let Pump::Done(count) = result {
                            assert_eq!(count, bytes.len() as u64);
                            done = true;
                            break;
                        }
                        assert!(
                            consumed.get() > before,
                            "positive-work More pump must progress"
                        );
                    }
                    assert!(done, "finite stream must finish");
                    assert_eq!(
                        actual, expected,
                        "resumed cap={cap}, capacity={capacity}, work={work}"
                    );
                }
            }
        }
    }
}

#[test]
fn retained_charge_exact_boundary_and_roster_ownership_are_admitted() {
    use crate::stats::{Fold, Reading};
    let meta = format!("session_id={}\n", f::S);
    let bytes = cert(1);
    let line = bytes.strip_suffix(b"\n").expect("fixture has LF");
    let charge = 4096 + 6144 + 8 * line.len() as u64;
    for shortage in [0, 1] {
        let limit = charge - shortage;
        let mut fold = Fold::with_retained(limit);
        assert_eq!(fold.room(), limit);
        let mut reading = Reading::live("charge", meta.as_bytes(), fold.room())
            .unwrap_or_else(|_| panic!("source base charge fits"));
        assert_eq!(reading.charge(), 4096);
        let result = reading.line(line);
        if shortage == 0 {
            assert!(result.is_ok(), "exact admission bound fits");
            assert_eq!(reading.charge(), charge);
            fold.add_reading(reading);
            assert_eq!(fold.room(), 0);
            assert_eq!(fold.finish(Some(f::S)).all.total, 1);
        } else {
            assert_eq!(
                result,
                Err(SourceGap::Retained),
                "one byte short must refuse"
            );
            assert_eq!(reading.charge(), 4096, "refusal retains no new state");
            fold.add(reading.fail(SourceGap::Retained));
            let view = fold.finish(Some(f::S));
            assert_eq!(view.all.total, 0);
            gap(&view, SourceGap::Retained);
        }
    }
    let mut roster = meta;
    for n in 0..100 {
        use std::fmt::Write as _;
        writeln!(roster, "seat.spawned.{n}=worker-{n}").expect("String write");
    }
    let Err(rejected) = Reading::live("roster", roster.as_bytes(), 4096) else {
        panic!("variable owned roster cannot hide behind flat SOURCE charge");
    };
    let mut refused = Fold::with_retained(4096);
    refused.add(rejected);
    gap(&refused.finish(Some(f::S)), SourceGap::Retained);
    let mut fold = Fold::with_retained(32 * 1024);
    let reading = Reading::live("roster", roster.as_bytes(), fold.room())
        .unwrap_or_else(|_| panic!("small roster fits"));
    assert!(
        reading.charge() > 4096,
        "cursor owns roster while source is open"
    );
    fold.add_reading(reading);
    assert_eq!(
        fold.room(),
        32 * 1024 - 4096,
        "finished source releases temporary roster charge"
    );
}

#[test]
fn retained_refusal_never_admits_a_second_record_before_rollback() {
    use crate::stats::{Fold, Reading};
    let meta = format!("session_id={}\n", f::S);
    let first = cert(1);
    let first = first.strip_suffix(b"\n").expect("fixture LF");
    let second = cert(2);
    let second = second.strip_suffix(b"\n").expect("fixture LF");
    let limit = 4096 + 2 * 6144 + 8 * (first.len() + second.len()) as u64 - 1;
    let mut fold = Fold::with_retained(limit);
    let mut reading = Reading::live("preflight", meta.as_bytes(), fold.room())
        .unwrap_or_else(|_| panic!("source fits"));
    reading.line(first).expect("first birth fits");
    let before = reading.charge();
    assert_eq!(reading.line(second), Err(SourceGap::Retained));
    assert_eq!(
        reading.charge(),
        before,
        "rejected record is never admitted"
    );
    fold.add(reading.fail(SourceGap::Retained));
    assert_eq!(
        fold.room(),
        limit,
        "rejected source commits no retained charge"
    );
    let view = fold.finish(Some(f::S));
    assert_eq!(view.all.total, 0);
    gap(&view, SourceGap::Retained);
}

#[test]
fn large_legacy_target_charges_raw_line_and_owned_target_separately() {
    use crate::stats::Reading;
    let meta = format!("session_id={}\n", f::S);
    let target = "w".repeat(1024 * 1024);
    let bytes =
        f::legacy("brief").replace("\"target\":\"scout\"", &format!("\"target\":\"{target}\""));
    let line = bytes.strip_suffix('\n').expect("fixture LF").as_bytes();
    let limit = 4096 + 512 + line.len() as u64 + line.len() as u64 / 2;
    let mut reading = Reading::live("legacy-target", meta.as_bytes(), limit)
        .unwrap_or_else(|_| panic!("source fits"));
    assert_eq!(
        reading.line(line),
        Err(SourceGap::Retained),
        "budget between one and two raw-line lengths cannot retain both copies"
    );
    assert_eq!(
        reading.charge(),
        4096,
        "large refused record retains no state"
    );
}

#[test]
fn statsread_v5_source_and_roster_exact_room_use_additive_byte_prices() {
    use crate::stats::{Fold, Reading};
    let plain = format!("session_id={}\n", f::S);
    let roster = format!("{plain}seat.spawned.0=ab\n");
    // Independent contract arithmetic: SOURCE + slot bytes + name bytes + seat.
    for (meta, price) in [
        (plain.as_bytes(), 4096),
        (roster.as_bytes(), 4096 + 9 + 2 + 112),
    ] {
        let reading = Reading::live("exact-open", meta, price)
            .unwrap_or_else(|_| panic!("exact source and roster admission must fit"));
        assert_eq!(
            reading.charge(),
            price,
            "roster strings are priced by their sum"
        );
        let mut fold = Fold::with_retained(price);
        fold.add_reading(reading);
        assert_eq!(
            fold.room(),
            price - 4096,
            "committing releases the entire roster charge"
        );
        assert_eq!(fold.finish(Some(f::S)).all.sources_read, 1);
        let Err(source) = Reading::live("short-open", meta, price - 1) else {
            panic!("one byte below the source and roster price must refuse");
        };
        let mut refused = Fold::with_retained(price - 1);
        refused.add(source);
        assert_eq!(
            refused.room(),
            price - 1,
            "refused source commits no charge"
        );
        let view = refused.finish(Some(f::S));
        assert_eq!((view.all.sources_read, view.all.sources_total), (0, 1));
        gap(&view, SourceGap::Retained);
    }
}

#[test]
fn s5_failed_retained_source_releases_room_for_a_later_source() {
    use crate::stats::{Fold, Reading};
    let meta = format!("session_id={}\n", f::S);
    let mut fold = Fold::with_retained(16 * 1024);
    let mut reading =
        Reading::live("a", meta.as_bytes(), fold.room()).unwrap_or_else(|_| panic!("source fits"));
    let first = cert(1);
    reading
        .line(first.strip_suffix(b"\n").expect("LF"))
        .expect("first birth fits");
    let second = cert(2);
    assert_eq!(
        reading.line(second.strip_suffix(b"\n").expect("LF")),
        Err(SourceGap::Retained)
    );
    fold.add(reading.fail(SourceGap::Retained));
    assert_eq!(fold.room(), 16 * 1024);
    let later = Reading::live("b", meta.as_bytes(), fold.room())
        .unwrap_or_else(|_| panic!("later source fits after rollback"));
    fold.add_reading(later);
    let actual = fold.finish(Some(f::S));
    assert_eq!(
        actual.all.total, 0,
        "prefix cannot leak from rejected source"
    );
    assert_eq!((actual.all.sources_read, actual.all.sources_total), (1, 2));
    assert!(
        actual
            .all
            .gaps
            .iter()
            .any(|gap| gap.source.as_deref() == Some("a")
                && gap.reason == Reason::Source(SourceGap::Retained)
                && gap.line().contains("over the memory budget"))
    );
}
