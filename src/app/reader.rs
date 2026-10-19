//! The file reader's pure half: the bounds a file is kept to, the body its
//! bytes or a refusal become, the wrap of its lines into display rows, the
//! viewport math and the status words.
//!
//! PURE — the bytes arrive as an argument; nothing here reads or paints. File
//! text is hostile: it is kept as read (lossily decoded, a trailing `\r`
//! aside) and neutralised where it is wrapped for paint, by the one
//! `board::terminal_text`.

use crate::board::terminal_text;
use crate::console::wrap::wrap;

/// The most bytes a file may hold to be read: a larger one is refused whole.
pub const MAX_BYTES: usize = 1_048_576;
/// The most lines a file may keep after its trailing blank lines are dropped:
/// more is refused whole.
pub const MAX_LINES: usize = 50_000;
/// The most scalars one kept line holds, its ellipsis included.
pub const LINE_CHARS: usize = 2000;
/// The leading bytes searched for a NUL: one there makes the file binary.
pub const BINARY_PROBE: usize = 8192;
/// The rows of context kept above a line the view jumps to.
const CONTEXT: usize = 3;

/// The cells a gutter takes for a file of `lines` lines: the line numbers'
/// digits, right-aligned, and one blank.
#[must_use]
pub fn gutter(lines: usize) -> usize {
    lines.to_string().len() + 1
}

/// What the panel shows of its file: its kept lines and its size on disk, or
/// the one line that says why there are none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Text { lines: Vec<String>, size: u64 },
    Gap(String),
}

/// The body `bytes` become, `size` being the file's length on disk. Binary
/// (a NUL inside the first [`BINARY_PROBE`] bytes) and a file past
/// [`MAX_LINES`] are named gaps. Otherwise the text is lossy UTF-8, split on
/// `\n` with a trailing `\r` run trimmed from each line, trailing blank lines
/// dropped, every other line kept and each capped at [`LINE_CHARS`]. No control
/// is stripped: the painter neutralises.
#[must_use]
pub fn decode(bytes: &[u8], size: u64) -> Body {
    if bytes.iter().take(BINARY_PROBE).any(|byte| *byte == 0) {
        return Body::Gap("binary file (NUL byte)".to_owned());
    }
    let text = String::from_utf8_lossy(bytes);
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    if lines.len() > MAX_LINES {
        return Body::Gap(format!(
            "too many lines: {} (limit {MAX_LINES})",
            lines.len()
        ));
    }
    Body::Text {
        lines: lines.iter().map(|line| capped(line)).collect(),
        size,
    }
}

/// `line`, or its first scalars and an ellipsis within [`LINE_CHARS`].
fn capped(line: &str) -> String {
    if line.chars().count() <= LINE_CHARS {
        return line.to_owned();
    }
    let mut kept: String = line.chars().take(LINE_CHARS - 1).collect();
    kept.push('…');
    kept
}

/// One display row of a wrapped file: which line it belongs to (from 0), the
/// blank cells it hangs by and its text, already neutralised.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    line: usize,
    gap: usize,
    text: String,
}

/// A file's lines wrapped to a text width: the display rows, and where each
/// line's first row stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flow {
    rows: Vec<Row>,
    starts: Vec<usize>,
}

/// `lines` wrapped to `width` text columns, the gutter not counted: a tab
/// expands, an indent is kept and a continuation hangs two cells, as the peek
/// panel wraps. A width of 0 wraps as 1.
#[must_use]
pub fn flow(lines: &[String], width: usize) -> Flow {
    // `wrap` measures from a one-cell bar the painter does not draw.
    let room = width.max(1) + 1;
    let mut flow = Flow::default();
    for (line, text) in lines.iter().enumerate() {
        flow.starts.push(flow.rows.len());
        for (gap, piece) in wrap(&terminal_text(text), room, 0) {
            flow.rows.push(Row {
                line,
                gap,
                text: piece,
            });
        }
    }
    flow
}

impl Flow {
    /// How many display rows the file has.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    /// The display row `line` (from 1) starts at: line 0 is line 1, a line
    /// past the end is the last line, and a file with no lines starts at 0.
    #[must_use]
    pub fn row_of(&self, line: usize) -> usize {
        let at = line.max(1) - 1;
        self.starts
            .get(at)
            .or(self.starts.last())
            .copied()
            .unwrap_or(0)
    }

    /// Display row `at`: its line (from 0), whether it is that line's first
    /// row, its hang and its text.
    pub(crate) fn display(&self, at: usize) -> Option<(usize, bool, usize, &str)> {
        let row = self.rows.get(at)?;
        let first = self.starts.get(row.line) == Some(&at);
        Some((row.line, first, row.gap, row.text.as_str()))
    }
}

/// Where a viewport of `visible` rows over `rows` starts when its top is
/// `top`, and the furthest the top can go.
#[must_use]
pub fn window(rows: usize, visible: usize, top: usize) -> (usize, usize) {
    let max_top = rows.saturating_sub(visible);
    (top.min(max_top), max_top)
}

/// The top row that shows display row `row`: three rows of context above it
/// where there are, never so low that the row leaves a window of `visible`
/// rows (at least one), and never past `max_top`.
#[must_use]
pub fn top_for(row: usize, visible: usize, max_top: usize) -> usize {
    let low = row.saturating_add(1).saturating_sub(visible.max(1));
    row.saturating_sub(CONTEXT).max(low).min(row).min(max_top)
}

/// What the panel draws: the file as the link named it, its newest body, its
/// wrap and how far down the viewport is. `links` counts the file links of the
/// turn the first of which was opened.
#[derive(Debug, Clone, Copy)]
pub struct View<'a> {
    pub path: &'a str,
    pub line: Option<usize>,
    /// `None` until the answer lands.
    pub body: Option<&'a Body>,
    /// The body's wrap at the width the frame draws it in, once made.
    pub flow: Option<&'a Flow>,
    /// The first display row shown.
    pub top: usize,
    pub links: usize,
}

/// The panel's status words for a viewport of `visible` rows: loading, why
/// nothing is read, or the display rows drawn of all of them, and a note that
/// more file links stand in the turn.
#[must_use]
pub fn status(view: &View<'_>, visible: usize) -> String {
    let total = match view.body {
        None => return "loading".to_owned(),
        Some(Body::Gap(_)) => return "not read".to_owned(),
        Some(Body::Text { lines, .. }) => view.flow.map_or(lines.len(), Flow::rows),
    };
    let (start, _) = window(total, visible, view.top);
    let first = if total == 0 { 0 } else { start + 1 };
    let last = start.saturating_add(visible).min(total);
    let more = if view.links > 1 {
        format!(" · first of {} links", view.links)
    } else {
        String::new()
    };
    format!("rows {first}-{last}/{total}{more}")
}
