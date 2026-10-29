#![no_main]

use ae::app::markdown::{self, MAX_CELLS, MAX_ROWS};
use libfuzzer_sys::fuzz_target;

// Two little-endian width bytes, then hostile lossy UTF-8 Markdown. Exercise
// the real parser, semantic mapper and layout, including global refusals.
fuzz_target!(|data: &[u8]| {
    let width = usize::from(u16::from_le_bytes([
        data.first().copied().unwrap_or(0),
        data.get(1).copied().unwrap_or(0),
    ]));
    let source = String::from_utf8_lossy(data.get(2..).unwrap_or_default());
    let r = std::hint::black_box(markdown::render(&source, width));
    assert!(r.rows() <= MAX_ROWS);
    let mut last = 0;
    let mut cells = 0;
    for row in 0..r.rows() {
        let text = r.text(row).unwrap_or_default();
        assert!(!text.chars().any(char::is_control));
        let count = text.chars().map(ae::event_text::cell).sum::<usize>();
        assert!(count <= width.clamp(1, 4096).max(8));
        cells += count;
        assert!(cells <= MAX_CELLS);
        let line = r.line_of(row);
        assert!(line >= last && line < source.split('\n').count());
        last = line;
        assert!(r.row_of(line + 1) < r.rows());
    }
    assert!(r.text(r.rows()).is_none());
    assert!(r.row_of(usize::MAX) < r.rows().max(1));
    if r.refusal().is_some() {
        assert_eq!(r.rows(), 0);
    }
    // The chat lane's mode: its own budget, and every row is its cut's blank gap
    // and the exact slice of a displayed line it names, less trailing blanks.
    let opts = markdown::Opts {
        soft_break_row: true,
    };
    let lane = markdown::render_with(&source, width, &markdown::Budget::LANE, opts);
    assert!(lane.rows() <= markdown::Budget::LANE.rows);
    assert!(lane.cells() <= markdown::Budget::LANE.cells);
    for row in 0..lane.rows() {
        let text = lane.text(row).unwrap_or_default();
        match lane.cut(row) {
            Some(cut) => {
                let piece =
                    (lane.lines().get(cut.line)).and_then(|l| l.get(cut.from..cut.from + cut.len));
                assert!(piece.is_some());
                let drawn = format!("{}{}", " ".repeat(cut.gap), piece.unwrap_or_default());
                assert_eq!(text, drawn.trim_end_matches(' '));
            }
            None => assert!(text.trim().is_empty()),
        }
    }
    assert!(lane.retained_bytes() >= lane.lines().iter().map(String::len).sum::<usize>());
});
