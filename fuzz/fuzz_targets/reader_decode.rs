#![no_main]

use ae::app::reader::{self, Body, Flow, View};
use libfuzzer_sys::fuzz_target;

// A file the in-app reader opens is hostile bytes: any tool, a crashed write
// or an agent's output can leave anything in a path a lane links. `decode`
// must take every byte string without a panic, refuse a NUL inside the
// first 8192 bytes whole, keep the size it was given, hand back at most 50,000
// lines of at most 2000 scalars with no line break and no trailing carriage
// return inside any of them and a last line that is not blank, and refuse
// nothing else but a file with too many lines. `flow` (the wrap) must be
// total at any width: a row for every line at least, each line's first row
// in order and inside the rows, past-the-end asking the last line. The
// viewport math (`window`, `top_for`) and the status words must be total over
// any numbers, and keep the top inside what they bound.
fuzz_target!(|data: &[u8]| {
    let size = u64::try_from(data.len()).unwrap_or(u64::MAX);
    let width = usize::from(data.first().copied().unwrap_or(0)) % 130;
    let body = std::hint::black_box(reader::decode(data, size));
    let nul = data
        .iter()
        .take(reader::BINARY_PROBE)
        .any(|byte| *byte == 0);
    match &body {
        Body::Gap(why) => {
            assert!(!why.is_empty(), "a refusal says why");
            assert!(
                nul || why.starts_with("too many lines"),
                "refused for neither a NUL nor too many lines: {why:?}"
            );
        }
        Body::Text { lines, size: kept } => {
            assert!(!nul, "a NUL inside the probe is never text");
            assert_eq!(*kept, size, "the size is the caller's");
            assert!(lines.len() <= reader::MAX_LINES);
            for line in lines {
                assert!(line.chars().count() <= reader::LINE_CHARS);
                assert!(!line.contains('\n') && !line.ends_with('\r'));
            }
            assert!(
                lines.last().is_none_or(|line| !line.trim().is_empty()),
                "trailing blank lines are dropped"
            );
            check_flow(lines, width);
        }
    }
    let [a, b, c, d] = [data.get(1), data.get(2), data.get(3), data.get(4)]
        .map(|byte| usize::from(byte.copied().unwrap_or(0)));
    let (rows, visible, top) = (a * 7, b % 40, c * 5);
    let (start, max_top) = reader::window(rows, visible, top);
    assert!(start <= top.min(rows) && start <= max_top);
    let row = d % (rows + 1);
    assert!(reader::top_for(row, visible, max_top) <= max_top);
    let view = View {
        path: "/w/a",
        line: Some(a),
        body: Some(&body),
        flow: None,
        top,
        links: b,
    };
    let _ = std::hint::black_box(reader::status(&view, visible));
});

fn check_flow(lines: &[String], width: usize) {
    let flow: Flow = std::hint::black_box(reader::flow(lines, width));
    if lines.is_empty() {
        assert_eq!(flow.rows(), 0);
        return;
    }
    assert!(flow.rows() >= lines.len(), "every line has a row");
    let mut last = 0;
    for line in 1..=lines.len() {
        let row = flow.row_of(line);
        assert!(
            last <= row && row < flow.rows(),
            "line {line} starts at {row}"
        );
        last = row;
    }
    assert_eq!(flow.row_of(lines.len() + 9), flow.row_of(lines.len()));
    assert_eq!(flow.row_of(0), flow.row_of(1));
}
