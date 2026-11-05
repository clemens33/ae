//! Independent storage-bound regression for the resumable line reader.

use super::{Line, LineReader, Pump};
use std::io::{BufReader, Cursor};

#[test]
fn pending_capacity_never_exceeds_the_content_cap() {
    // Small caps catch Vec's minimum allocation. The non-power-of-two cap
    // catches geometric growth beyond the admitted content length.
    for cap in [0_usize, 1, 3, 1000] {
        for buffer in [7, 100] {
            for work in [1_u64, 17, 257] {
                let short = cap.min(700);
                let mut input = vec![b'a'; short];
                input.push(b'\n');
                input.extend(std::iter::repeat_n(b'b', cap));
                input.push(b'\n');
                input.extend(std::iter::repeat_n(b'c', cap + 1));
                input.push(b'\n');
                input.extend(std::iter::repeat_n(b'd', cap));
                let input_len = input.len();
                let reader = BufReader::with_capacity(buffer, Cursor::new(input));
                let mut lines = LineReader::new(reader, cap);
                let mut actual = Vec::new();
                let mut done = false;
                for _ in 0..=input_len {
                    let result = lines
                        .pump(work, &mut |line| {
                            actual.push(match line {
                                Line::Text(body) => Some(body.len()),
                                Line::Over => None,
                            });
                        })
                        .expect("fixture reader succeeds");
                    assert!(
                        lines.pending.capacity() <= cap,
                        "capacity {} > cap {cap}, buffer {buffer}, work {work}",
                        lines.pending.capacity(),
                    );
                    if let Pump::Done(read) = result {
                        assert_eq!(read, u64::try_from(input_len).expect("fixture fits u64"));
                        done = true;
                        break;
                    }
                }
                assert!(
                    done,
                    "reader must finish within one pump per input byte + EOF"
                );
                let mut expected = vec![Some(short), Some(cap), None];
                if cap != 0 {
                    expected.push(Some(cap));
                }
                assert_eq!(actual, expected, "cap {cap}, buffer {buffer}, work {work}");
            }
        }
    }
}

#[test]
fn statsread_v5_completed_long_line_releases_storage_before_next_line() {
    // The retention policy permits at most 64 KiB between completed lines.
    // Stop just before LF to establish a live allocation above that threshold.
    let body_len = 64 * 1024 + 1;
    for buffer in [7, 100] {
        let mut input = vec![b'x'; body_len];
        input.extend(b"\nY\n");
        let input_len = input.len();
        let reader = BufReader::with_capacity(buffer, Cursor::new(input));
        let mut lines = LineReader::new(reader, 128 * 1024);
        let mut actual = Vec::new();
        let mut sink = |line: Line<'_>| {
            actual.push(match line {
                Line::Text(body) => Some(body.len()),
                Line::Over => None,
            });
        };
        assert_eq!(
            lines
                .pump(
                    u64::try_from(body_len).expect("fixture fits u64"),
                    &mut sink
                )
                .expect("fixture reader"),
            Pump::More
        );
        assert_eq!(lines.pending.len(), body_len, "first pump stops before LF");
        assert!(
            lines.pending.capacity() > 64 * 1024,
            "fixture establishes oversized pending storage"
        );
        assert_eq!(lines.pump(1, &mut sink).expect("fixture LF"), Pump::More);
        assert!(
            lines.pending.capacity() <= 64 * 1024,
            "completed long line must release storage before the next line"
        );
        assert_eq!(
            lines.pump(u64::MAX, &mut sink).expect("fixture tail"),
            Pump::Done(u64::try_from(input_len).expect("fixture fits u64"))
        );
        assert_eq!(
            actual,
            vec![Some(body_len), Some(1)],
            "release preserves framing and next line"
        );
    }
}
