//! #307 framing and admission contracts; requires the planned new interface.
use super::*;
use crate::usage::{Line, LineReader, Pump};
use std::cell::Cell;
use std::io::{self, BufRead, BufReader, Cursor, Read};
use std::rc::Rc;

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
fn overlong_lines_have_own_labeled_gap_and_do_not_discard_other_spawns() {
    let root = Scratch::new("307-line-limit");
    let cap = cert(1).len() - 1;
    let mut bytes = cert(1);
    bytes.extend(vec![b'x'; cap + 1]);
    bytes.push(b'\n');
    bytes.extend(cert(2));
    bytes.extend(vec![b'y'; cap + 100]);
    let dir = root.live("lines", f::S, &bytes);
    let mut scan = Scan::limited(
        root.0.clone(),
        BTreeMap::from([("lines".into(), dir)]),
        true,
        1,
        Some(f::S.into()),
        Limits {
            line: cap,
            chunk: 3,
            ..LIMITS
        },
    );
    let view = drain(&mut scan);
    assert_eq!(
        view.all.total, 2,
        "valid births around refused lines survive"
    );
    assert_eq!((view.all.sources_read, view.all.sources_total), (1, 1));
    assert_eq!(view.all.gaps.len(), 1);
    let gap = &view.all.gaps[0];
    assert_eq!(gap.reason, Reason::LineLimit);
    assert_eq!(gap.source.as_deref(), Some("lines"));
    assert_eq!(
        gap.count, 2,
        "each overlong line counted once across pumps, including EOF"
    );
    assert!(gap.line().contains("line"));
    assert!(gap.line().contains("limit"));
    assert!(view.all.partial());
    assert!(view.all.balanced());
}

#[test]
fn retained_limit_fails_source_atomically_with_its_own_reason() {
    let root = Scratch::new("307-retained");
    let mut bytes = cert(1);
    bytes.extend(cert(2));
    let a = root.live("a", f::S, &bytes);
    let b = root.live("b", f::T, b"");
    let mut scan = Scan::limited(
        root.0.clone(),
        BTreeMap::from([("a".into(), a), ("b".into(), b)]),
        true,
        1,
        Some(f::S.into()),
        Limits {
            retained: 16 * 1024,
            ..LIMITS
        },
    );
    let view = drain(&mut scan);
    assert_eq!(
        view.all.total, 0,
        "a prefix birth must not leak from rejected source"
    );
    assert_eq!(
        (view.all.sources_read, view.all.sources_total),
        (1, 2),
        "small later source still fits after failed source releases admission"
    );
    assert!(
        view.all
            .gaps
            .iter()
            .any(|gap| gap.source.as_deref() == Some("a")
                && gap.reason == Reason::Source(SourceGap::Retained)
                && gap.line().contains("over the memory budget"))
    );
    assert!(view.all.partial());
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
fn shrink_between_chunks_discards_source_prefix_and_closing_cancels_scan() {
    let root = Scratch::new("307-shrink-cancel");
    let mut bytes = cert(1);
    bytes.extend(vec![b' '; 128 * 1024]);
    let dir = root.live("active", f::S, &bytes);
    let mut scan = Scan::limited(
        root.0.clone(),
        BTreeMap::from([("active".into(), dir.clone())]),
        true,
        1,
        None,
        Limits {
            chunk: cert(1).len() as u64,
            ..LIMITS
        },
    );
    assert!(scan.step().is_none(), "list");
    assert!(
        scan.step().is_none(),
        "source remains open after first chunk"
    );
    assert_eq!(scan.remaining(), Some(1));
    std::fs::write(dir.join("events.jsonl"), b"").expect("owned snapshot deliberately shrinks");
    let view = drain(&mut scan);
    assert_eq!(view.all.total, 0, "failure rolls back prefix birth");
    gap(&view, SourceGap::Unreadable);
    std::fs::write(dir.join("events.jsonl"), bytes).expect("restore owned fixture");
    let mut reader = Reader::new(root.0.clone(), None, None);
    reader.scanned = true;
    reader.dirs.insert("active".into(), dir.clone());
    reader.take(Request::Settings {
        open: true,
        generation: 1,
    });
    reader.take(Request::Stats { generation: 1 });
    reader.stats = Some(Scan::limited(
        root.0.clone(),
        BTreeMap::from([("active".into(), dir)]),
        true,
        1,
        None,
        Limits {
            chunk: cert(1).len() as u64,
            ..LIMITS
        },
    ));
    assert!(reader.stats_step().is_none(), "list");
    assert!(
        reader.stats_step().is_none(),
        "source remains open after one chunk"
    );
    let before = snapshot(&root.0);
    reader.take(Request::Settings {
        open: false,
        generation: 1,
    });
    assert!(
        reader.stats.is_none(),
        "close drops the cursor and unpublished source"
    );
    assert_eq!(
        snapshot(&root.0),
        before,
        "cancel writes no store, journal, temp or cache"
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
fn statsread_v5_exact_member_cap_with_short_scan_budget_reports_budget() {
    let root = Scratch::new("307-exact-member-budget");
    root.archived(f::S, b"");
    let (meta, _) = archive_content(f::S);
    let cap = u64::try_from(meta.len()).expect("small fixture fits u64");
    let mut scan = Scan::limited(
        root.0.clone(),
        BTreeMap::new(),
        true,
        1,
        None,
        Limits {
            member: cap,
            bytes: cap - 1,
            ..LIMITS
        },
    );
    let view = drain(&mut scan);
    assert_eq!((view.all.sources_read, view.all.sources_total), (0, 1));
    assert_eq!(view.all.gaps.len(), 1);
    assert_eq!(
        view.all.gaps[0].reason,
        Reason::Source(SourceGap::Budget),
        "an exact-cap member is admissible; only the shared read budget ran out"
    );
    assert_eq!(view.all.gaps[0].source.as_deref(), Some(f::S));
    assert!(view.all.partial());
}
