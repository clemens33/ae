//! The file reader's pure half: the bounds a file is kept to, the body its
//! bytes or a refusal become, the wrap of its lines into display rows, the
//! viewport math and the status words.
//!
//! PURE — the bytes arrive as an argument; nothing here reads or paints. File
//! text is hostile: it is kept as read (lossily decoded, a trailing `\r`
//! aside) and neutralised where it is wrapped for paint, by the one
//! `board::terminal_text`.

use super::filedoor;
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
/// The column `wrap` starts a line's first row at, which fixes its tab stops:
/// one cell behind the bar the painter does not draw.
pub const WRAP_COLUMN: usize = 1;

/// The files a browser opens natively and the reader cannot show: each
/// extension (matched without regard to case) and the kind the note names. Raw
/// markup (HTML, SVG) or a binary refusal (images, PDF) would be the reader's
/// only answer for them.
const VISUAL: [(&str, &str); 13] = [
    ("html", "HTML page"),
    ("htm", "HTML page"),
    ("xhtml", "HTML page"),
    ("svg", "SVG image"),
    ("png", "image"),
    ("jpg", "image"),
    ("jpeg", "image"),
    ("gif", "image"),
    ("webp", "image"),
    ("avif", "image"),
    ("bmp", "image"),
    ("ico", "image"),
    ("pdf", "PDF document"),
];

/// The kind of visual file `path` names by its extension, if it is one. The
/// extension alone decides: such a file is never read, so no content is
/// looked at, and a leading dot alone (`.html`) is a name, not an extension.
#[must_use]
pub fn visual(path: &str) -> Option<&'static str> {
    let ext = std::path::Path::new(path).extension()?.to_str()?;
    VISUAL
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(ext))
        .map(|(_, kind)| *kind)
}

/// What the panel says instead of a visual file's content: the kind, that ae
/// does not render it, the way back to the link in the lane and the hint row's
/// own words for opening a link — the terminal's link, not ae, opens the file.
#[must_use]
pub fn visual_note(kind: &str) -> String {
    format!(
        "{kind}: ae does not render this file. Press Esc, then on the link: {}. \
         Your system opens it in your browser or default app.",
        crate::console::link::open_hint()
    )
}

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
    Tree { lines: Vec<String> },
    Gap(String),
}

impl Body {
    /// The lines a body shows and the gutter cells they take: a file's lines
    /// are numbered, a tree's rows are not; a gap has none.
    #[must_use]
    pub fn lines(&self) -> Option<(&[String], usize)> {
        match self {
            Self::Text { lines, .. } => Some((lines, gutter(lines.len()))),
            Self::Tree { lines } => Some((lines, 0)),
            Self::Gap(_) => None,
        }
    }
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

/// The bidi format controls: a name holding one could reorder what the row
/// shows around it.
const BIDI: [char; 12] = [
    '\u{061c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}',
    '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
];

/// An entry name as one tree row shows it: lossy UTF-8 through
/// `board::terminal_text`, and the `\n`, `\t` and bidi controls that owner
/// keeps for text become U+FFFD too, so one name is one row and cannot forge
/// another entry.
fn tree_name(name: &std::ffi::OsStr) -> String {
    terminal_text(&name.to_string_lossy())
        .chars()
        .map(|ch| {
            if ch == '\n' || ch == '\t' || BIDI.contains(&ch) {
                '\u{FFFD}'
            } else {
                ch
            }
        })
        .collect()
}

/// The mark after an entry's name: `ls -F`'s for its kind, and for a folder
/// not listed, why.
fn tree_mark(node: &filedoor::Node) -> String {
    use filedoor::{Kind, Listed};
    let kind = match node.kind {
        Kind::Folder => "/",
        Kind::Link => "@",
        Kind::Fifo => "|",
        Kind::Socket => "=",
        Kind::File | Kind::Other => "",
    };
    let state = match &node.listed {
        Listed::Walked => String::new(),
        Listed::Collapsed | Listed::Deep | Listed::Spent => " …".to_owned(),
        Listed::Unreadable(why) => format!(" (unreadable: {why})"),
        Listed::Moved => " (changed while listed)".to_owned(),
    };
    format!("{kind}{state}")
}

/// The rows a folder's tree shows, pre-order under `tree(1)`'s branch glyphs:
/// each listed entry once, its name made one row by `tree_name`, and under
/// a folder whose entries were cut one `+N more` row (`+N or more, not all
/// read` when the read budget ran out inside it). A tree with no entries is
/// one `(empty folder)` row.
#[must_use]
pub fn tree_lines(tree: &filedoor::Tree) -> Vec<String> {
    let mut rows = Vec::new();
    tree_rows(tree, 0, "", &mut rows);
    if rows.is_empty() {
        rows.push("(empty folder)".to_owned());
    }
    rows
}

/// [`tree_lines`] for node `at`'s entries, each row led by `prefix`.
fn tree_rows(tree: &filedoor::Tree, at: usize, prefix: &str, rows: &mut Vec<String>) {
    let Some(node) = tree.nodes.get(at) else {
        return;
    };
    let cut = node.more > 0 || !node.exact;
    let count = node.children.len() + usize::from(cut);
    for (step, child) in node.children.iter().enumerate() {
        let Some(entry) = tree.nodes.get(*child) else {
            continue;
        };
        let last = step + 1 == count;
        let branch = if last { "└── " } else { "├── " };
        rows.push(format!(
            "{prefix}{branch}{}{}",
            tree_name(&entry.name),
            tree_mark(entry)
        ));
        let hang = if last { "    " } else { "│   " };
        tree_rows(tree, *child, &format!("{prefix}{hang}"), rows);
    }
    if cut {
        let more = if node.exact {
            format!("+{} more", node.more)
        } else {
            format!("+{} or more, not all read", node.more)
        };
        rows.push(format!("{prefix}└── {more}"));
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

    /// Where display row `at`'s text stands in its line's `flat` text (the
    /// line neutralised and its tabs spent from [`WRAP_COLUMN`]): the bytes the
    /// wrap kept, found by walking the line's rows from its first with a
    /// cursor that skips only the blanks the wrap dropped. `None` when a row
    /// is not where the cursor says, so a caller never colours text it cannot
    /// place.
    pub(crate) fn span_of(&self, at: usize, flat: &str) -> Option<std::ops::Range<usize>> {
        let line = self.rows.get(at)?.line;
        let first = *self.starts.get(line)?;
        let mut cursor = 0;
        for row in self.rows.get(first..=at)? {
            let rest = flat.get(cursor..)?;
            cursor += rest.len() - rest.trim_start_matches(' ').len();
            if !flat.get(cursor..)?.starts_with(row.text.as_str()) {
                return None;
            }
            cursor += row.text.len();
        }
        Some(cursor - self.rows.get(at)?.text.len()..cursor)
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
        Some(Body::Text { lines, .. } | Body::Tree { lines }) => {
            view.flow.map_or(lines.len(), Flow::rows)
        }
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

#[cfg(test)]
#[path = "synhl_spec.rs"]
mod synhl_spec;

#[cfg(test)]
mod tree_tests {
    use super::filedoor::{Kind, Listed, Node, Tree};
    use super::{tree_lines, tree_name};
    use std::ffi::OsString;

    fn node(name: &str, kind: Kind, listed: Listed, children: Vec<usize>) -> Node {
        Node {
            name: OsString::from(name),
            kind,
            listed,
            children,
            more: 0,
            exact: true,
        }
    }

    #[test]
    fn a_tree_is_drawn_pre_order_with_branches_marks_and_states() {
        let mut root = node(
            "",
            Kind::Folder,
            Listed::Walked,
            vec![1, 3, 4, 5, 6, 7, 8, 9],
        );
        root.more = 2;
        let mut src = node("src", Kind::Folder, Listed::Walked, vec![2]);
        src.more = 4;
        src.exact = false;
        let tree = Tree {
            nodes: vec![
                root,
                src,
                node("main.rs", Kind::File, Listed::Walked, vec![]),
                node(".git", Kind::Folder, Listed::Collapsed, vec![]),
                node("deep", Kind::Folder, Listed::Deep, vec![]),
                node("late", Kind::Folder, Listed::Spent, vec![]),
                node(
                    "locked",
                    Kind::Folder,
                    Listed::Unreadable("permission denied".to_owned()),
                    vec![],
                ),
                node("alias", Kind::Link, Listed::Walked, vec![]),
                node("pipe", Kind::Fifo, Listed::Walked, vec![]),
                node("sock", Kind::Socket, Listed::Walked, vec![]),
            ],
        };
        assert_eq!(
            tree_lines(&tree),
            [
                "├── src/",
                "│   ├── main.rs",
                "│   └── +4 or more, not all read",
                "├── .git/ …",
                "├── deep/ …",
                "├── late/ …",
                "├── locked/ (unreadable: permission denied)",
                "├── alias@",
                "├── pipe|",
                "├── sock=",
                "└── +2 more",
            ]
        );
    }

    #[test]
    fn the_last_entry_closes_its_branch_and_an_empty_tree_says_so() {
        let tree = Tree {
            nodes: vec![
                node("", Kind::Folder, Listed::Walked, vec![1]),
                node("only", Kind::Folder, Listed::Walked, vec![2]),
                node("leaf", Kind::Other, Listed::Walked, vec![]),
            ],
        };
        assert_eq!(tree_lines(&tree), ["└── only/", "    └── leaf"]);
        let moved = Tree {
            nodes: vec![
                node("", Kind::Folder, Listed::Walked, vec![1]),
                node("swapped", Kind::Folder, Listed::Moved, vec![]),
            ],
        };
        assert_eq!(tree_lines(&moved), ["└── swapped/ (changed while listed)"]);
        let empty = Tree {
            nodes: vec![node("", Kind::Folder, Listed::Walked, vec![])],
        };
        assert_eq!(tree_lines(&empty), ["(empty folder)"]);
    }

    #[test]
    fn a_name_is_one_row_of_terminal_text() {
        assert_eq!(tree_name("plain name.rs".as_ref()), "plain name.rs");
        for hostile in [
            "a\nb",
            "a\tb",
            "a\u{1b}b",
            "a\u{202e}b",
            "a\u{2066}b",
            "a\u{061c}b",
        ] {
            assert_eq!(tree_name(hostile.as_ref()), "a\u{FFFD}b", "{hostile:?}");
        }
    }
}
