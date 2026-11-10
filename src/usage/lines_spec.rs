//! #288 A4. Independent LF framing oracle from plan-r3 section 6.
//! This file installs at `src/usage/lines_spec.rs` and is wired by `usage::lines`.
#![allow(clippy::expect_used, reason = "frozen fixture cursor assertions")]

use super::{Framed, Lines};

#[derive(Debug, PartialEq, Eq)]
enum Seen {
    Line(Vec<u8>),
    OverCap,
}

fn accept(events: &mut Vec<Seen>, frame: Framed<'_>) {
    events.push(match frame {
        Framed::Line(bytes) => Seen::Line(bytes.to_vec()),
        Framed::OverCap => Seen::OverCap,
    });
}

// This oracle sees final bytes, not Lines or a product parser. Unterminated tails
// above cap are diagnosed once even before their LF arrives.
fn oracle(bytes: &[u8], cap: usize) -> Vec<Seen> {
    let mut want = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            want.push(if index - start > cap {
                Seen::OverCap
            } else {
                Seen::Line(bytes[start..index].to_vec())
            });
            start = index + 1;
        }
    }
    if bytes.len() - start > cap {
        want.push(Seen::OverCap);
    }
    want
}

fn drive(bytes: &[u8], cap: usize, ends: &[usize]) -> (Lines, Vec<Seen>) {
    let mut lines = Lines::new(cap);
    let mut seen = Vec::new();
    for end in ends {
        let before = lines.cursor();
        let start = usize::try_from(before).expect("fixture cursor");
        assert!(start <= *end && *end <= bytes.len());
        let consumed = lines.step(&bytes[start..*end], &mut |event| accept(&mut seen, event));
        assert_eq!(lines.cursor(), before + consumed);
        assert!(lines.cursor() <= u64::try_from(*end).expect("fixture length"));
    }
    (lines, seen)
}

#[test]
fn a4_lines_match_lf_oracle_at_every_split_and_repeated_offer() {
    for cap in [1, 3, 8] {
        for input in [
            b"".as_slice(),
            b"\n",
            b"a\n",
            b"a\nbb\nccc",
            b"abc\r\n\nlast\n",
            b"0123456789abcdef\nvalid\npartial",
            b"0123456789abcdef",
        ] {
            for split in 0..=input.len() {
                let ends = [split, split, input.len(), input.len()];
                let (_, actual) = drive(input, cap, &ends);
                assert_eq!(actual, oracle(input, cap), "cap={cap} split={split}");
            }
            let ends: Vec<_> = (0..=input.len()).collect();
            let (_, actual) = drive(input, cap, &ends);
            assert_eq!(actual, oracle(input, cap), "bytewise cap={cap}");
        }
    }
}

#[test]
fn a4_tail_is_not_committed_until_lf_even_when_valid_json() {
    let input = b"{}\n{\"a\":1}\n";
    let (mut lines, mut seen) = drive(input, 64, &[10]);
    assert_eq!(lines.cursor(), 3);
    assert_eq!(seen, vec![Seen::Line(b"{}".to_vec())]);
    assert!(!lines.skipping());
    let start = usize::try_from(lines.cursor()).expect("fixture cursor");
    lines.step(&input[start..], &mut |event| accept(&mut seen, event));
    assert_eq!(lines.cursor(), 11);
    assert_eq!(
        seen,
        vec![
            Seen::Line(b"{}".to_vec()),
            Seen::Line(b"{\"a\":1}".to_vec())
        ]
    );
}

#[test]
fn a4_cap_is_inclusive_and_one_overlong_line_has_one_diagnostic() {
    let input = b"1234\n12345\nZ\n";
    let (_, seen) = drive(input, 4, &[5, 10, 11, 11, 13]);
    assert_eq!(
        seen,
        vec![
            Seen::Line(b"1234".to_vec()),
            Seen::OverCap,
            Seen::Line(b"Z".to_vec())
        ]
    );
}

#[test]
fn a4_large_overlong_partial_advances_each_step_then_recovers() {
    const STEP: usize = 4 << 20;
    const CAP: usize = 1 << 20;
    let mut input = vec![b'x'; STEP * 2 + CAP + 1];
    input.extend_from_slice(b"\n{}\n");
    let mut lines = Lines::new(CAP);
    let mut seen = Vec::new();
    for end in [STEP, STEP * 2, STEP * 2 + CAP + 1, input.len()] {
        let before = lines.cursor();
        let start = usize::try_from(before).expect("fixture cursor");
        lines.step(&input[start..end], &mut |event| accept(&mut seen, event));
        assert!(lines.cursor() > before, "over-cap input must make progress");
    }
    assert_eq!(seen, vec![Seen::OverCap, Seen::Line(b"{}".to_vec())]);
    assert!(!lines.skipping());
    assert_eq!(
        lines.cursor(),
        u64::try_from(input.len()).expect("fixture length")
    );
}
