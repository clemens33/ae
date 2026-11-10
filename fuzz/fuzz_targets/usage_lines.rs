#![no_main]

use ae::usage::lines::{Framed, LINE_CAP, Lines};
use libfuzzer_sys::fuzz_target;

#[derive(Debug, PartialEq, Eq)]
enum Seen {
    Line(Vec<u8>),
    OverCap,
}

/// An independent oracle over the final bytes: `split(LF)` with the
/// inclusive length rule, and one over-cap mark for an unterminated tail
/// longer than the cap.
fn oracle(bytes: &[u8], cap: usize) -> Vec<Seen> {
    let mut want = Vec::new();
    let mut parts = bytes.split(|byte| *byte == b'\n').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_some() {
            want.push(if part.len() > cap {
                Seen::OverCap
            } else {
                Seen::Line(part.to_vec())
            });
        } else if part.len() > cap {
            want.push(Seen::OverCap);
        }
    }
    want
}

fuzz_target!(|data: &[u8]| {
    // First byte the cap (0..=63, or the product cap), second the split
    // seed, the rest the file's bytes.
    let Some((&cap, rest)) = data.split_first() else {
        return;
    };
    let Some((&seed, bytes)) = rest.split_first() else {
        return;
    };
    let cap = if cap == 255 {
        LINE_CAP
    } else {
        usize::from(cap % 64)
    };
    let mut lines = Lines::new(cap);
    let mut seen = Vec::new();
    let mut state = u32::from(seed) | 1;
    let mut end = 0_usize;
    loop {
        // A pseudo-random end at or after the cursor, sometimes repeated.
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let start = usize::try_from(lines.cursor()).unwrap_or(usize::MAX);
        assert!(start <= bytes.len());
        end = (end.max(start) + (state as usize % 9)).min(bytes.len());
        let before = lines.cursor();
        let consumed = lines.step(&bytes[start..end], &mut |frame| {
            seen.push(match frame {
                Framed::Line(line) => Seen::Line(line.to_vec()),
                Framed::OverCap => Seen::OverCap,
            });
        });
        assert_eq!(lines.cursor(), before + consumed);
        assert!(lines.cursor() <= end as u64);
        if end - start > cap {
            assert!(consumed > 0, "more than the cap of new bytes advances");
        }
        if end == bytes.len() {
            let start = usize::try_from(lines.cursor()).unwrap_or(usize::MAX);
            let again = lines.step(&bytes[start..], &mut |_| {
                panic!("a repeated offer frames nothing new");
            });
            assert_eq!(again, 0);
            break;
        }
    }
    assert_eq!(seen, oracle(bytes, cap));
});
