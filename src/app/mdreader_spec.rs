//! Frozen acceptance cases: mdreader P1 (#251 RENDER) — the Markdown mapper,
//! its draw entry and the reader's rendered/raw mode in the App. Independent
//! navigator mdreader-nav (Anthropic / Claude Opus 5.5), frozen against driver
//! mdreader's plan r2 (lead ACK e6ff0e3a) and nav pins (1)-(5). Oracle: issue
//! #251 body, its review-amendments and owner-decision comments, and brief
//! rulings R1-R6 — never the code. Driver installs byte-identically, wired from
//! src/app.rs via `#[cfg(test)] #[path = "app/mdreader_spec.rs"] mod
//! mdreader_spec;`. The browse decode table row for `m` is the companion
//! amendment to `tests/it/app_spec.rs`.
//!
//! Surface compiled against (plan r2 :11-31 plus the draw seam named here):
//! `markdown::{is_markdown(&str)->bool, render(&str, usize)->Rendered,
//! MAX_ROWS, MAX_CELLS, MAX_COLUMNS, MAX_TABLE_WIDTH, MAX_DEPTH}`;
//! `Rendered::{rows()->usize, text(row)->Option<&str>, marks(row, byte)->Marks,
//! line_of(row)->usize (0-based block start), row_of(line 1-based)->usize
//! (total), notes()->usize, refusal()->Option<&'static str>}`; `Marks` is
//! `Debug + Clone + Copy + PartialEq + Eq + Default` with pub fields
//! `heading: Option<u8>` and `strong, emphasis, strike, code, code_block,
//! quote, link, target, html, note: bool`. The draw seam:
//! `markdown::View<'a> { path: &'a str, line: Option<usize>, lines: &'a
//! [String], shown: Shown<'a>, refusal: Option<&'a str>, top: usize, links:
//! usize }`, `markdown::Shown<'a>::{Rendered(&'a Rendered), Raw(&'a
//! reader::Flow)}` and `draw::draw_markdown(&markdown::View, Option<Look>, &mut
//! Buffer) -> Bounds` — header row then body, like `draw_reader`; Rendered has
//! no gutter, Raw draws today's gutter and bold target number. The new browse
//! key is `model::Key::Mode` (`m`).
//!
//! Behaviour pinned beyond the plan's words (agreed in the pair, sent to lead):
//! a rule is ONE row of one repeated non-alphanumeric character spanning the
//! width; theme-off draw wears `BOLD` on heading and strong cells, `ITALIC`
//! on emphasis, `CROSSED_OUT` on strike; no rendered row holds any control
//! character (tab included: it is expanded or replaced before measuring).
//!
//! With a drawn look a link target wears the palette's `dim` foreground and
//! inline and block code a ground of their own, padded across a code row
//! (r2, colead review 298da6ff).
//!
//! Named residuals (not pinned here): other colours (heading hues, quote
//! bar), drag-select copying rendered text, the fuzz target itself (its own
//! gate: `just rust-fuzz-check` + one 60 s run).

use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::Modifier;
use ratatui_core::text::Line;

use super::draw::draw_markdown;
use super::fleet::{Counts, Fleet, Line2, Row};
use super::loader::{Answer, Request};
use super::markdown::{self, Marks, Rendered, Shown};
use super::model::Model;
use super::reader::{self, Body};
use super::{App, take_keys};
use crate::console::input;
use crate::console::lane::{Item, Kind, Lane};
use crate::digest::{SessionEntry, Status};
use crate::theme::Mark;
use crate::time::Timestamp;

// ---------------------------------------------------------------------------
// helpers over the pure mapper
// ---------------------------------------------------------------------------

/// The narrowest width a row is held to. Below it a marker, an indent or one
/// wide scalar may overflow and the draw clips (the named residual
/// `console::wrap` already has: it emits one scalar even when too narrow).
/// At and above it every row fits, deep nesting included: an indent is
/// clamped so the text keeps room.
const FLOOR: usize = 8;

/// Cells `text` takes, as the terminal measures it.
fn cells(text: &str) -> usize {
    Line::from(text).width()
}

fn texts(r: &Rendered) -> Vec<String> {
    (0..r.rows())
        .map(|row| {
            r.text(row)
                .unwrap_or_else(|| panic!("row {row} of {}", r.rows()))
                .to_owned()
        })
        .collect()
}

fn dump(r: &Rendered) -> String {
    texts(r)
        .iter()
        .enumerate()
        .map(|(row, text)| format!("{row:>4} |{text}|"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The invariants EVERY render keeps, whatever the source: each row is text,
/// no row holds a control character, no row is wider than the width (0 is
/// 1, past 4096 is 4096) or, below [`FLOOR`] cells, than the floor, anchors
/// run in source order and stay inside the source, `row_of` is total, and
/// `text` past the end is `None`.
fn sane(r: &Rendered, source: &str, width: usize) {
    let width = width.clamp(1, 4096).max(FLOOR);
    let lines = source.split('\n').count().max(1);
    let mut last = 0;
    for (row, text) in texts(r).iter().enumerate() {
        assert!(
            !text.chars().any(char::is_control),
            "row {row} carries a control: {text:?}\n{}",
            dump(r)
        );
        assert!(
            cells(text) <= width,
            "row {row} is {} cells at width {width}: {text:?}",
            cells(text)
        );
        let at = r.line_of(row);
        assert!(at >= last, "anchors run in source order at row {row}");
        assert!(at < lines, "row {row} anchors line {at} of {lines}");
        last = at;
    }
    assert!(r.text(r.rows()).is_none(), "no row past the end");
    for line in 0..=lines + 2 {
        let row = r.row_of(line);
        assert!(
            row < r.rows().max(1),
            "row_of({line}) = {row} of {}",
            r.rows()
        );
    }
}

fn render(source: &str, width: usize) -> Rendered {
    let r = markdown::render(source, width);
    sane(&r, source, width);
    r
}

/// The row and byte where `needle` is first drawn.
fn find(r: &Rendered, needle: &str) -> (usize, usize) {
    texts(r)
        .iter()
        .enumerate()
        .find_map(|(row, text)| text.find(needle).map(|at| (row, at)))
        .unwrap_or_else(|| panic!("{needle:?} is not drawn:\n{}", dump(r)))
}

fn has(r: &Rendered, needle: &str) -> bool {
    texts(r).iter().any(|text| text.contains(needle))
}

fn marks_at(r: &Rendered, needle: &str) -> Marks {
    let (row, at) = find(r, needle);
    r.marks(row, at)
}

/// The cell column `needle` starts at in its row, and the one it ends before.
fn span(r: &Rendered, needle: &str) -> (usize, usize) {
    let (row, at) = find(r, needle);
    let text = r.text(row).unwrap_or_default();
    let start = cells(&text[..at]);
    (start, start + cells(needle))
}

fn row_text(r: &Rendered, needle: &str) -> String {
    let (row, _) = find(r, needle);
    r.text(row).unwrap_or_default().to_owned()
}

// ---------------------------------------------------------------------------
// markdown::is_markdown — which files open rendered
// ---------------------------------------------------------------------------

#[test]
fn only_md_and_markdown_extensions_are_markdown_ascii_case_blind() {
    for yes in [
        "/w/docs/r.md",
        "/w/README.MD",
        "/w/a/Notes.Md",
        "/w/x.markdown",
        "/w/x.MARKDOWN",
        "/w/x.Markdown",
        "rel/plain.md",
    ] {
        assert!(markdown::is_markdown(yes), "{yes} is Markdown");
    }
    for no in [
        "/w/src/a.rs",
        "/w/a.mdx",
        "/w/a.md.txt",
        "/w/a.mkd",
        "/w/a.txt",
        "/w/md",
        "/w/dir.md/file",
        "/w/a.md/",
        "/w/amd",
        "",
    ] {
        assert!(!markdown::is_markdown(no), "{no} is not Markdown");
    }
}

#[test]
fn the_budgets_are_the_ruled_numbers() {
    assert_eq!(markdown::MAX_ROWS, 50_000);
    assert_eq!(markdown::MAX_CELLS, 2_000_000);
    assert_eq!(markdown::MAX_COLUMNS, 32);
    assert_eq!(markdown::MAX_TABLE_WIDTH, 240);
    assert_eq!(markdown::MAX_DEPTH, 32);
}

// ---------------------------------------------------------------------------
// blocks: headings, paragraphs, lists, code, quotes, rules
// ---------------------------------------------------------------------------

#[test]
fn headings_drop_their_markers_and_carry_their_level() {
    let src = "# One\n## Two\n### Three\n#### Four\n##### Five\n###### Six\n\nAlpha\n=====\n\nBeta\n----\n";
    let r = render(src, 60);
    for (word, level) in [
        ("One", 1),
        ("Two", 2),
        ("Three", 3),
        ("Four", 4),
        ("Five", 5),
        ("Six", 6),
        ("Alpha", 1),
        ("Beta", 2),
    ] {
        assert_eq!(
            marks_at(&r, word).heading,
            Some(level),
            "{word}\n{}",
            dump(&r)
        );
    }
    assert!(!has(&r, "#"), "no ATX marker is drawn\n{}", dump(&r));
    assert!(!has(&r, "====="), "no setext underline is drawn");
    assert_eq!(r.refusal(), None);
    assert_eq!(r.notes(), 0);
}

#[test]
fn inline_emphasis_strong_strike_and_code_lose_their_markers() {
    let src =
        "plain *emword* **strongword** ~~goneword~~ `codeword` _underem_ __understrong__ end\n";
    let r = render(src, 120);
    assert_eq!(
        marks_at(&r, "plain"),
        Marks::default(),
        "plain text wears nothing"
    );
    let em = marks_at(&r, "emword");
    assert!(
        em.emphasis && !em.strong && !em.strike && !em.code,
        "{em:?}"
    );
    let st = marks_at(&r, "strongword");
    assert!(st.strong && !st.emphasis, "{st:?}");
    let gone = marks_at(&r, "goneword");
    assert!(gone.strike && !gone.strong, "{gone:?}");
    let code = marks_at(&r, "codeword");
    assert!(code.code && !code.code_block, "{code:?}");
    assert!(marks_at(&r, "underem").emphasis);
    assert!(marks_at(&r, "understrong").strong);
    let row = row_text(&r, "plain");
    for marker in ['*', '~', '`', '_'] {
        assert!(!row.contains(marker), "{marker} is not drawn: {row:?}");
    }
    let nested = render("***both*** and **bold *inner* tail**\n", 80);
    let both = marks_at(&nested, "both");
    assert!(both.strong && both.emphasis, "{both:?}");
    let inner = marks_at(&nested, "inner");
    assert!(inner.strong && inner.emphasis, "{inner:?}");
    let tail = marks_at(&nested, "tail");
    assert!(tail.strong && !tail.emphasis, "{tail:?}");
}

#[test]
fn a_paragraph_joins_soft_breaks_and_wraps_without_losing_a_word() {
    let r = render("one\ntwo\n", 80);
    assert!(has(&r, "one two"), "a soft break is a space\n{}", dump(&r));
    let hard = render("one  \ntwo\n", 80);
    assert_ne!(
        find(&hard, "one").0,
        find(&hard, "two").0,
        "a hard break is a new row"
    );
    let words: Vec<String> = (0..40).map(|n| format!("w{n:02}")).collect();
    let src = words.join(" ");
    let r = render(&src, 12);
    assert!(r.rows() > 1);
    let drawn: Vec<String> = texts(&r)
        .iter()
        .flat_map(|row| {
            row.split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(drawn, words, "every word, once, in order");
    let wide = "漢字かな交じり文".repeat(6);
    let r = render(&wide, 7);
    let joined: String = texts(&r).iter().map(|row| row.trim()).collect();
    assert_eq!(joined, wide, "no wide scalar is lost or split");
}

#[test]
fn lists_nest_two_cells_a_level_and_ordered_keep_their_start() {
    let src = "- alpha\n  - beta\n    - gamma\n- delta\n\n3. three\n4. four\n";
    let r = render(src, 60);
    assert_eq!(row_text(&r, "alpha"), "- alpha", "{}", dump(&r));
    assert_eq!(row_text(&r, "beta"), "  - beta");
    assert_eq!(row_text(&r, "gamma"), "    - gamma");
    assert_eq!(row_text(&r, "delta"), "- delta");
    assert_eq!(row_text(&r, "three"), "3. three");
    assert_eq!(row_text(&r, "four"), "4. four");
    let long = format!("- {}\n", ["word"; 10].join(" "));
    let r = render(&long, 20);
    let rows = texts(&r);
    let item: Vec<&String> = rows.iter().filter(|row| row.contains("word")).collect();
    assert!(item.len() > 1, "the item wraps\n{}", dump(&r));
    assert!(item[0].starts_with("- word"));
    for row in &item[1..] {
        assert!(
            row.starts_with("  ") && !row[2..].starts_with(' '),
            "a continuation hangs under the item text: {row:?}"
        );
    }
}

#[test]
fn fenced_and_indented_code_is_verbatim_monospace_without_fences() {
    let src = "before\n\n```rust\nlet x = 1;\n    indented();\n\ttabbed();\n```\n\nafter\n\n    four space code\n";
    let r = render(src, 60);
    assert!(!has(&r, "```"), "fences are not drawn\n{}", dump(&r));
    assert!(!has(&r, "rust"), "the info string is not drawn");
    let let_row = row_text(&r, "let x = 1;");
    assert_eq!(let_row.trim(), "let x = 1;");
    assert!(marks_at(&r, "let x").code_block);
    assert!(has(&r, "    indented();"), "code indentation is kept");
    assert!(marks_at(&r, "tabbed();").code_block);
    assert!(
        marks_at(&r, "four space code").code_block,
        "an indented block is code too"
    );
    assert!(!marks_at(&r, "before").code_block);
    assert!(!marks_at(&r, "after").code_block);
    assert_eq!(
        span(&r, "indented();").0,
        span(&r, "let x").0 + 4,
        "four spaces deeper than its neighbour"
    );
}

#[test]
fn block_quotes_indent_and_mark_their_text() {
    let r = render("> quoted words here\n\nafter\n", 60);
    let quoted = marks_at(&r, "quoted");
    assert!(quoted.quote, "{quoted:?}");
    assert!(
        find(&r, "quoted").1 > 0,
        "a quote is set in from the edge\n{}",
        dump(&r)
    );
    assert!(!marks_at(&r, "after").quote);
    assert_eq!(row_text(&r, "after"), "after");
}

#[test]
fn a_rule_is_one_row_of_one_character_across_the_width() {
    for width in [10, 30, 77] {
        let r = render("above\n\n---\n\nbelow\n", width);
        let (top, _) = find(&r, "above");
        let (bottom, _) = find(&r, "below");
        let rules: Vec<String> = texts(&r)[top + 1..bottom]
            .iter()
            .filter(|row| !row.trim().is_empty())
            .cloned()
            .collect();
        assert_eq!(rules.len(), 1, "one rule row at {width}\n{}", dump(&r));
        let rule = &rules[0];
        let first = rule.chars().next().unwrap_or(' ');
        assert!(
            !first.is_alphanumeric() && rule.chars().all(|ch| ch == first),
            "{rule:?}"
        );
        assert_eq!(cells(rule), width, "the rule spans the width");
    }
}

// ---------------------------------------------------------------------------
// tables
// ---------------------------------------------------------------------------

const TABLE: &str = "| left | right |\n|:-----|------:|\n| x1 | yy22 |\n| zzz | w |\n";

#[test]
fn a_table_that_fits_is_an_aligned_grid() {
    let r = render(TABLE, 80);
    assert!(
        !has(&r, ":--") && !has(&r, "--:"),
        "the delimiter row is not drawn as source\n{}",
        dump(&r)
    );
    let rows = [find(&r, "left").0, find(&r, "x1").0, find(&r, "zzz").0];
    assert!(
        rows[0] < rows[1] && rows[1] < rows[2],
        "header first, rows in order"
    );
    let left = span(&r, "left").0;
    assert_eq!(span(&r, "x1").0, left, "a left column starts together");
    assert_eq!(span(&r, "zzz").0, left);
    let right = span(&r, "right").1;
    assert_eq!(span(&r, "yy22").1, right, "a right column ends together");
    assert_eq!(span(&r, "w").1, right);
    assert_eq!(r.notes(), 0);
}

#[test]
fn a_table_wider_than_the_viewport_is_raw_rows_with_no_note() {
    let src = "| aaaaaaaaaaaaaaaa | bbbbbbbbbbbbbbbbbbbb |\n|:---|---:|\n| c | d |\n\nafter\n";
    let r = render(src, 30);
    assert!(
        has(&r, "|:---|---:|"),
        "the source rows are shown\n{}",
        dump(&r)
    );
    assert_eq!(r.notes(), 0, "an ordinary fallback says nothing");
    assert!(!has(&r, "raw:"));
    assert_eq!(row_text(&r, "after"), "after", "the next block renders");
    let wide = render(src, 80);
    assert!(
        !has(&wide, ":---"),
        "the same table fits at 80\n{}",
        dump(&wide)
    );
}

fn columns(count: usize, cell: &str) -> String {
    let head: Vec<String> = (0..count).map(|n| format!("{cell}{n}")).collect();
    let rule = vec![":--"; count];
    let body: Vec<String> = (0..count).map(|n| format!("v{n}")).collect();
    format!(
        "| {} |\n|{}|\n| {} |\n",
        head.join(" | "),
        rule.join("|"),
        body.join(" | ")
    )
}

#[test]
fn past_the_column_budget_a_table_is_raw_after_one_note() {
    let fits = render(&columns(32, "c"), 4096);
    assert_eq!(fits.notes(), 0, "{}", dump(&fits));
    assert!(!has(&fits, ":--"));
    let src = format!("intro\n\n{}\noutro\n", columns(33, "c"));
    let over = render(&src, 4096);
    assert_eq!(over.notes(), 1, "{}", dump(&over));
    assert_eq!(
        row_text(&over, "raw: table"),
        "raw: table has 33 columns (limit 32)"
    );
    assert!(marks_at(&over, "raw: table").note);
    let (note, _) = find(&over, "raw: table");
    let (source, _) = find(&over, ":--");
    assert!(source > note, "the raw source follows its note");
    assert_eq!(
        row_text(&over, "intro"),
        "intro",
        "the blocks around render"
    );
    assert_eq!(row_text(&over, "outro"), "outro");
    assert!(!marks_at(&over, "outro").note);
    // Columns are judged before width: 33 wide columns name the columns.
    let both = render(&columns(33, "cccccccccc"), 4096);
    assert_eq!(
        row_text(&both, "raw:"),
        "raw: table has 33 columns (limit 32)"
    );
}

#[test]
fn past_the_padded_width_budget_a_table_is_raw_after_one_note() {
    let cell = |n: usize, ch: char| std::iter::repeat_n(ch, n).collect::<String>();
    let table = |n: usize| {
        format!(
            "| {} | {} | {} |\n|---|---|---|\n| a | b | c |\n",
            cell(n, 'p'),
            cell(n, 'q'),
            cell(n, 'r')
        )
    };
    let fits = render(&table(60), 4096);
    assert_eq!(fits.notes(), 0, "{}", dump(&fits));
    assert!(!has(&fits, "|---|"));
    let over = render(&table(100), 4096);
    assert_eq!(over.notes(), 1, "{}", dump(&over));
    assert_eq!(row_text(&over, "raw:"), "raw: table wider than 240 cells");
    assert!(has(&over, "|---|---|---|"));
    // The budget is judged before the viewport: a narrow column still names it.
    let narrow = render(&table(100), 100);
    assert_eq!(narrow.notes(), 1, "{}", dump(&narrow));
    assert_eq!(row_text(&narrow, "raw:"), "raw: table wider than 240 cells");
}

// ---------------------------------------------------------------------------
// depth and the global budgets
// ---------------------------------------------------------------------------

#[test]
fn nesting_past_the_depth_budget_is_raw_after_one_note() {
    let quotes = |n: usize| format!("{} deep{n}\n", ">".repeat(n));
    let fits = render(&quotes(32), 200);
    assert_eq!(fits.notes(), 0, "{}", dump(&fits));
    assert!(marks_at(&fits, "deep32").quote);
    let src = format!("intro\n\n{}\noutro\n", quotes(33));
    let over = render(&src, 200);
    assert_eq!(over.notes(), 1, "{}", dump(&over));
    assert_eq!(row_text(&over, "raw:"), "raw: nested deeper than 32");
    assert!(marks_at(&over, "raw:").note);
    assert!(has(&over, ">>>>>>>>"), "the raw source follows");
    assert!(find(&over, ">>>>").0 > find(&over, "raw:").0);
    assert_eq!(row_text(&over, "outro"), "outro");
}

/// `items` list items nested one in the next, each its emphasised name,
/// every line behind `quotes` block-quote markers: `quotes + items` open
/// containers around the innermost item's text.
fn nested(quotes: usize, items: usize) -> String {
    let prefix = "> ".repeat(quotes);
    (0..items)
        .map(|at| format!("{prefix}{}- *e{at}*", "  ".repeat(at)))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn list_items_and_quotes_count_together_toward_the_depth_budget() {
    for (quotes, items) in [(0, 32), (16, 16), (31, 1)] {
        let src = format!("intro\n\n{}\n\noutro\n", nested(quotes, items));
        let fits = render(&src, 200);
        assert_eq!(fits.notes(), 0, "{quotes}+{items} renders\n{}", dump(&fits));
        let last = format!("e{}", items - 1);
        assert!(marks_at(&fits, &last).emphasis, "{quotes}+{items}");
        assert!(!has(&fits, "*e"), "no emphasis marker survives");
    }
    for (quotes, items) in [(0, 33), (16, 17), (32, 1)] {
        let src = format!("intro\n\n{}\n\noutro\n", nested(quotes, items));
        let over = render(&src, 200);
        assert_eq!(over.notes(), 1, "{quotes}+{items} is raw\n{}", dump(&over));
        assert_eq!(row_text(&over, "raw:"), "raw: nested deeper than 32");
        assert!(has(&over, "- *e0*"), "the raw source follows its note");
        assert!(find(&over, "*e0*").0 > find(&over, "raw:").0);
        assert_eq!(row_text(&over, "intro"), "intro");
        assert_eq!(row_text(&over, "outro"), "outro", "the next block renders");
        assert!(!marks_at(&over, "outro").note);
    }
}

#[test]
fn past_the_row_budget_the_whole_document_refuses_at_that_width() {
    let para = "x".repeat(2000);
    let src = vec![para.as_str(); 30].join("\n\n");
    let narrow = markdown::render(&src, 1);
    assert_eq!(narrow.refusal(), Some("too large to render (rows limit)"));
    let wide = render(&src, 4096);
    assert_eq!(wide.refusal(), None, "the same file renders wide");
}

#[test]
fn past_the_cell_budget_the_whole_document_refuses_padding_counted() {
    let src = format!("```\n{}```\n", "c\n".repeat(1000));
    let wide = markdown::render(&src, 4096);
    assert_eq!(
        wide.refusal(),
        Some("too large to render (cells limit)"),
        "1000 code rows padded to 4096 cells"
    );
    let fits = render(&src, 80);
    assert_eq!(fits.refusal(), None);
    assert!(fits.rows() >= 1000);
}

// ---------------------------------------------------------------------------
// links, images, HTML: inert text
// ---------------------------------------------------------------------------

#[test]
fn links_are_label_and_dim_target_text_and_images_are_named() {
    let r = render(
        "see [the label](https://x.io/p) now\n\nauto <https://x.io/a> end\n\n![alt text](pic.png)\n\n[ref][1] and [r](docs/x.md)\n\n[1]: https://r.io\n",
        120,
    );
    assert!(has(&r, "the label (https://x.io/p)"), "{}", dump(&r));
    assert!(marks_at(&r, "the label").link);
    assert!(marks_at(&r, "https://x.io/p").target);
    assert!(!marks_at(&r, "the label").target);
    assert_eq!(marks_at(&r, "see"), Marks::default());
    let auto = row_text(&r, "https://x.io/a");
    assert_eq!(
        auto.matches("https://x.io/a").count(),
        1,
        "an autolink shows once: {auto:?}"
    );
    assert!(has(&r, "[image: alt text](pic.png)"));
    assert!(
        has(&r, "ref (https://r.io)"),
        "a reference resolves to text"
    );
    assert!(has(&r, "r (docs/x.md)"));
    assert!(!has(&r, "[1]:"), "a definition is not drawn");
    assert!(!has(&r, "!["), "no image syntax survives");
}

#[test]
fn raw_html_is_literal_text_never_interpreted_or_dropped() {
    let r = render(
        "<div class=\"x\" onclick=\"y()\">hi</div>\n\ntext <b>inline</b> and <!-- c --> end\n\n<script>alert(1)</script>\n",
        120,
    );
    assert!(
        has(&r, "<div class=\"x\" onclick=\"y()\">hi</div>"),
        "{}",
        dump(&r)
    );
    assert!(marks_at(&r, "<div").html);
    assert!(has(&r, "<b>inline</b>"));
    assert!(marks_at(&r, "<b>").html);
    assert!(!marks_at(&r, "inline").strong, "a tag styles nothing");
    assert!(has(&r, "<!-- c -->"));
    assert!(has(&r, "<script>alert(1)</script>"));
}

#[test]
fn only_tables_and_strikethrough_extensions_interpret() {
    for (src, literal) in [
        ("- [ ] task one\n- [x] done\n", "[ ] task one"),
        ("- [ ] task one\n- [x] done\n", "[x] done"),
        ("word[^1]\n\n[^1]: the note\n", "word[^1]"),
        ("word[^1]\n\n[^1]: the note\n", "[^1]: the note"),
        ("\"smart\" -- and ... 'q'\n", "\"smart\" -- and ... 'q'"),
        ("# Head {#anchor}\n", "Head {#anchor}"),
        ("math $x^2$ here\n", "$x^2$"),
        ("> [!NOTE]\n> body\n", "[!NOTE]"),
        ("a [[wiki]] b\n", "[[wiki]]"),
        ("x^2^ up\n", "x^2^"),
        ("Term\n: definition\n", ": definition"),
    ] {
        let r = render(src, 80);
        assert!(has(&r, literal), "{src:?} keeps {literal:?}\n{}", dump(&r));
    }
}

// ---------------------------------------------------------------------------
// hostile bytes and totality
// ---------------------------------------------------------------------------

const HOSTILE: &str = "# H\u{1b}[2Jx\n\npara \u{1b}]52;c;QUFB\u{7} tail \u{9b}31m c1 \u{7f} del\n\n`co\u{1b}de`\n\n```\nfe\u{1b}[31mnce\n```\n\n| t\u{1b}h | b |\n|---|---|\n| c\u{1b}ell | d |\n\n[li\u{1b}nk](https://x/y) ![al\u{1b}t](pq.png)\n\n- it\u{1b}em\n\n> qu\u{1b}ote\n\n<p>ht\u{1b}ml</p>\n";

#[test]
fn escape_sequences_anywhere_are_neutralised_never_drawn() {
    for width in [0, 1, 3, 20, 80, 4096] {
        let r = render(HOSTILE, width);
        let all = texts(&r).join("\n");
        assert!(!all.contains('\u{1b}') && !all.contains('\u{7}'), "{width}");
        assert!(
            !all.contains('\u{9b}') && !all.contains('\u{7f}'),
            "{width}"
        );
        if width >= 20 {
            assert!(all.contains('\u{fffd}'), "an introducer shows as U+FFFD");
            for kept in ["para", "tail", "fe", "nce", "ell", "nk", "em", "ote", "ml"] {
                assert!(
                    all.contains(kept),
                    "{kept} survives at {width}\n{}",
                    dump(&r)
                );
            }
        }
    }
}

#[test]
fn render_is_total_at_every_width_and_clamps_both_ends() {
    let fixtures = [
        "",
        "\n\n\n",
        "# only",
        TABLE,
        HOSTILE,
        "- a\n  - b\n    - c\n",
        "```\nopen fence never closed\n",
        "> > > > > > > > > > > > > > > > > > > > deep twenty\n",
        "- a\n  - b\n    - c\n      - d\n        - e\n          - f\n            - g\n",
        "[a](b \"t\") ![c](d) <e> **f *g* h**",
        "\r\n\r\n# crlf\r\npara\r\n",
    ];
    for source in fixtures {
        for width in [0, 1, 2, 5, 80, 4096, 10_000] {
            let _ = render(source, width);
        }
        assert_eq!(
            texts(&markdown::render(source, 0)),
            texts(&markdown::render(source, 1)),
            "width 0 is width 1: {source:?}"
        );
        assert_eq!(
            texts(&markdown::render(source, 10_000)),
            texts(&markdown::render(source, 4096)),
            "a width past 4096 is 4096: {source:?}"
        );
    }
    let empty = render("", 40);
    assert_eq!(empty.rows(), 0);
    assert_eq!(empty.row_of(1), 0);
    assert_eq!(empty.refusal(), None);
}

#[test]
fn rows_anchor_to_their_block_start_line_and_row_of_finds_them() {
    let src = "# H\n\npara one\n\n- a\n- b\n";
    let r = render(src, 40);
    let head = find(&r, "H").0;
    assert_eq!(
        r.text(head),
        Some("H"),
        "the heading row is rendered\n{}",
        dump(&r)
    );
    let para = find(&r, "para one").0;
    let list = find(&r, "- a").0;
    assert_eq!(r.line_of(head), 0);
    assert_eq!(r.line_of(para), 2);
    assert_eq!(r.line_of(list), 4);
    assert!(r.line_of(find(&r, "- b").0) >= 4);
    assert_eq!(r.row_of(1), head, "line 1 is the heading's row");
    assert_eq!(r.row_of(0), head, "line 0 asks line 1");
    assert_eq!(r.row_of(3), para, "line 3 is the paragraph's first row");
    assert_eq!(r.row_of(5), list);
}

// ---------------------------------------------------------------------------
// draw::draw_markdown — the column over a bare buffer
// ---------------------------------------------------------------------------

fn buf(width: u16, height: u16) -> Buffer {
    Buffer::empty(Rect::new(0, 0, width, height))
}

fn buf_row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// The first cell of row `y` at which `needle` is drawn.
fn cell_of(buf: &Buffer, y: u16, needle: &str) -> u16 {
    let symbols: Vec<&str> = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
    (0..symbols.len())
        .find(|&x| symbols[x..].concat().starts_with(needle))
        .and_then(|x| u16::try_from(x).ok())
        .unwrap_or_else(|| panic!("{needle:?} not on row {y}: {}", buf_row(buf, y)))
}

fn source_lines(src: &str) -> Vec<String> {
    src.split('\n').map(str::to_owned).collect()
}

#[test]
fn draw_rendered_has_no_gutter_and_wears_the_inline_modifiers() {
    let src = "# Big\n\n**bold** *it* ~~st~~ plain\n";
    let lines = source_lines(src);
    let rendered = markdown::render(src, 40);
    let view = markdown::View {
        path: "/w/r.md",
        line: None,
        lines: &lines,
        shown: Shown::Rendered(&rendered),
        refusal: None,
        top: 0,
        links: 1,
    };
    let mut cells = buf(40, 10);
    let bounds = draw_markdown(&view, None, &mut cells);
    assert_eq!(bounds.visible, 9);
    let header = buf_row(&cells, 0);
    assert!(
        header.starts_with(&format!(
            "Read · /w/r.md · rendered · rows 1-{0}/{0}",
            rendered.rows()
        )),
        "{header}"
    );
    assert!(
        buf_row(&cells, 1).starts_with("Big"),
        "no gutter: {}",
        buf_row(&cells, 1)
    );
    let y = (1..10)
        .find(|&y| buf_row(&cells, y).contains("bold"))
        .unwrap_or_else(|| panic!("bold row"));
    let at = |needle: &str| cells[(cell_of(&cells, y, needle), y)].modifier;
    assert!(
        cells[(0, 1)].modifier.contains(Modifier::BOLD),
        "a heading is bold"
    );
    assert!(at("bold").contains(Modifier::BOLD));
    assert!(at("it ").contains(Modifier::ITALIC));
    assert!(at("st").contains(Modifier::CROSSED_OUT));
    assert!(!at("plain").contains(Modifier::BOLD));
    assert!(!at("plain").contains(Modifier::ITALIC));
}

#[test]
fn draw_with_a_look_dims_link_targets_and_grounds_code_apart() {
    let src = "see [label](https://x.io/t) and `let n = 4;` plain\n\n```rust\nlet s = \"str\"; // note 42\nfn main() { s.len() }\n```\n";
    let lines = source_lines(src);
    let rendered = markdown::render(src, 50);
    let view = markdown::View {
        path: "/w/r.md",
        line: None,
        lines: &lines,
        shown: Shown::Rendered(&rendered),
        refusal: None,
        top: 0,
        links: 1,
    };
    let look = crate::theme::Look::read("on", "darcula", "on", "on");
    let mut cells = buf(50, 10);
    let _ = draw_markdown(&view, Some(look), &mut cells);
    let row_with = |needle: &str| {
        (1..10)
            .find(|&y| buf_row(&cells, y).contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} drawn:\n{}", whole(&cells)))
    };
    let y = row_with("label");
    let cell = |needle: &str, y: u16| &cells[(cell_of(&cells, y, needle), y)];
    let [red, green, blue] = crate::theme::rgb(look.palette.dim);
    let dim = ratatui_core::style::Color::Rgb(red, green, blue);
    let target = cell_of(&cells, y, "https://x.io/t");
    for x in target..target + 14 {
        assert_eq!(cells[(x, y)].fg, dim, "every target cell is dim: x {x}");
    }
    assert_ne!(cell("label", y).fg, dim, "its label is not");
    let plain = cell("plain", y).bg;
    // No syntax colour: every visible character of a code span or row wears
    // ONE foreground, whatever its token (keyword, string, comment, number).
    let one_ink = |row: u16, from: u16, to: u16, what: &str| {
        let inks: Vec<_> = (from..to)
            .filter(|&x| !cells[(x, row)].symbol().trim().is_empty())
            .map(|x| cells[(x, row)].fg)
            .collect();
        assert!(!inks.is_empty(), "{what} draws characters");
        assert!(
            inks.iter().all(|ink| *ink == inks[0]),
            "{what} wears one foreground: {inks:?}"
        );
    };
    let inline = cell_of(&cells, y, "let n = 4;");
    let ground = cells[(inline, y)].bg;
    assert_ne!(ground, plain, "inline code wears its own ground");
    for x in inline..inline + 10 {
        assert_eq!(cells[(x, y)].bg, ground, "inline code is one ground: x {x}");
    }
    one_ink(y, inline, inline + 10, "inline code");
    let first = row_with("let s = ");
    let second = row_with("fn main()");
    assert_eq!(second, first + 1, "fence lines are not drawn between");
    assert!(
        !whole(&cells).contains("rust"),
        "the info string is not drawn"
    );
    for row in [first, second] {
        one_ink(row, 0, 50, "a code row");
    }
    assert_eq!(
        cells[(cell_of(&cells, first, "let"), first)].fg,
        cells[(cell_of(&cells, second, "fn"), second)].fg,
        "the whole block wears one foreground"
    );
    let code = cell("let s = ", first).bg;
    assert_ne!(code, plain, "a code block wears its own ground");
    for row in [first, second] {
        for x in 0..50 {
            assert_eq!(
                cells[(x, row)].bg,
                code,
                "a code row is one ground, padded across: x {x} row {row}"
            );
        }
    }
    assert!(
        !buf_row(&cells, first - 1).contains("```") && !buf_row(&cells, second + 1).contains("```"),
        "no fence cell is drawn"
    );
}

#[test]
fn draw_raw_markdown_names_raw_keeps_the_gutter_and_the_bold_target() {
    let lines = vec!["one".to_owned(), "# two".to_owned(), "three".to_owned()];
    let flow = reader::flow(&lines, 40 - reader::gutter(3));
    let view = markdown::View {
        path: "/w/r.md",
        line: Some(2),
        lines: &lines,
        shown: Shown::Raw(&flow),
        refusal: None,
        top: 0,
        links: 1,
    };
    let mut cells = buf(40, 10);
    let bounds = draw_markdown(&view, None, &mut cells);
    assert_eq!((bounds.visible, bounds.max_back), (9, 0));
    assert!(
        buf_row(&cells, 0).starts_with("Read · /w/r.md:2 · raw · rows 1-3/3"),
        "{}",
        buf_row(&cells, 0)
    );
    assert_eq!(buf_row(&cells, 1), "1 one");
    assert_eq!(buf_row(&cells, 2), "2 # two");
    assert!(cells[(0, 2)].modifier.contains(Modifier::BOLD));
    assert!(!cells[(0, 1)].modifier.contains(Modifier::BOLD));
    let refused = markdown::View {
        refusal: Some("too large to render (rows limit)"),
        line: None,
        links: 3,
        ..view
    };
    let mut cells = buf(96, 10);
    let _ = draw_markdown(&refused, None, &mut cells);
    assert!(
        buf_row(&cells, 0).starts_with(
            "Read · /w/r.md · raw: too large to render (rows limit) · rows 1-3/3 · first of 3 links"
        ),
        "{}",
        buf_row(&cells, 0)
    );
}

#[test]
fn draw_markdown_neutralises_the_path_and_is_total_on_one_row() {
    let lines = source_lines("# a");
    let rendered = markdown::render("# a", 20);
    let view = markdown::View {
        path: "/w/a\u{1b}b.md",
        line: None,
        lines: &lines,
        shown: Shown::Rendered(&rendered),
        refusal: None,
        top: 0,
        links: 1,
    };
    let mut cells = buf(30, 1);
    let bounds = draw_markdown(&view, None, &mut cells);
    assert_eq!(bounds.visible, 0);
    assert!(buf_row(&cells, 0).starts_with("Read · /w/a\u{fffd}b.md"));
    let mut cells = buf(30, 4);
    let _ = draw_markdown(&markdown::View { top: 999, ..view }, None, &mut cells);
    assert!(
        buf_row(&cells, 1).starts_with('a'),
        "a top past the end clamps"
    );
}

// ---------------------------------------------------------------------------
// App — a Markdown link opens rendered, m toggles, :line opens raw
// ---------------------------------------------------------------------------

fn fleet(names: &[&str]) -> Fleet {
    let row = |(at, name): (usize, &&str)| Row {
        name: (*name).to_owned(),
        index: at + 1,
        mark: Mark::Working,
        needy: false,
        unanswered: None,
        counts: Counts::Unknown,
        line2: Line2::NoGoal,
        home: *name == "api",
    };
    Fleet {
        rows: names.iter().enumerate().map(row).collect(),
        home: Some("api".to_owned()),
    }
}

fn said(body: &str) -> Item {
    Item {
        micros: 1_759_500_000_000_000,
        kind: Kind::Said {
            who: "lead".to_owned(),
        },
        body: body.to_owned(),
        record: None,
    }
}

/// An app on session `api` (work dir `/w`, home `/h`) whose lane holds one
/// MARKED turn saying `body`.
fn marked_app(body: &str) -> (App, Receiver<Request>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::new(Some("api".to_owned()), None, None, Some(tx));
    (app.fleeted, app.loading) = (true, false);
    let fleet = fleet(&["api"]);
    app.model = Model::new(&fleet);
    app.fleet = fleet;
    let mut entry = SessionEntry::new("api", Status::Running);
    entry.work_dir = Some("/w".to_owned());
    app.world.sessions.push(entry.clone());
    app.overview = super::overview::of(
        &entry,
        None,
        Some(Path::new("/h")),
        Timestamp::from_epoch(1_759_500_600),
    );
    app.lane = Rc::new(Lane {
        items: vec![said(body)],
        coverage: vec![],
    });
    let key = app.keys()[0];
    app.model.set_turn(Some(key));
    (app, rx)
}

fn pressed(app: &mut App, bytes: &[u8]) -> Option<bool> {
    take_keys(
        app,
        vec![(input::Key::Text(bytes.to_vec()), Instant::now())],
    )
}

fn frame_of(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut cells = buf(width, height);
    app.frame(&mut cells);
    cells
}

/// The reader panel as text: its header row and the body rows under it, each
/// read from the header's first cell to the right edge.
fn panel(cells: &Buffer) -> Vec<String> {
    let (x, y) = (0..cells.area.height)
        .find_map(|y| {
            let symbols: Vec<&str> = (0..cells.area.width)
                .map(|x| cells[(x, y)].symbol())
                .collect();
            (0..symbols.len())
                .find(|&x| symbols[x..].concat().starts_with("Read · "))
                .map(|x| (x, y))
        })
        .unwrap_or_else(|| panic!("no reader header:\n{}", whole(cells)));
    (y..cells.area.height)
        .map(|row| {
            (u16::try_from(x).unwrap_or(0)..cells.area.width)
                .map(|col| cells[(col, row)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

fn whole(cells: &Buffer) -> String {
    (0..cells.area.height)
        .map(|y| buf_row(cells, y))
        .collect::<Vec<_>>()
        .join("\n")
}

fn opened(app: &mut App, rx: &Receiver<Request>) -> super::loader::ReadAsk {
    assert_eq!(pressed(app, b"r"), Some(true));
    let Ok(Request::Read(Some(ask))) = rx.try_recv() else {
        panic!("the open asks for one read");
    };
    *ask
}

fn answer(app: &mut App, generation: u64, src: &str) {
    app.answer(Answer::Read {
        generation,
        body: Body::Text {
            lines: src.lines().map(str::to_owned).collect(),
            size: 0,
        },
    });
}

const REPORT: &str = "# Title\n\nSome *body* text.";

#[test]
fn a_markdown_link_opens_rendered_and_m_toggles_raw_without_a_reread() {
    let (mut app, rx) = marked_app("report at docs/r.md done");
    let mark = app.model.turn();
    let ask = opened(&mut app, &rx);
    assert_eq!((ask.path.as_str(), ask.line), ("/w/docs/r.md", None));
    answer(&mut app, ask.generation, REPORT);
    let shown = panel(&frame_of(&mut app, 160, 45));
    assert!(
        shown[0].starts_with("Read · /w/docs/r.md · rendered · rows"),
        "{}",
        shown[0]
    );
    assert_eq!(shown[1], "Title", "rendered, no gutter, no marker");
    assert!(
        shown.iter().any(|row| row.contains("Some body text.")),
        "{shown:?}"
    );
    assert!(!shown.iter().any(|row| row.contains("# Title")));
    let _ = pressed(&mut app, b"m");
    assert!(rx.try_recv().is_err(), "a toggle reads nothing again");
    let raw = panel(&frame_of(&mut app, 160, 45));
    assert!(
        raw[0].starts_with("Read · /w/docs/r.md · raw · rows 1-3/3"),
        "{}",
        raw[0]
    );
    assert_eq!(raw[1], "1 # Title");
    assert_eq!(raw[3], "3 Some *body* text.");
    let _ = pressed(&mut app, b"m");
    assert!(rx.try_recv().is_err());
    let back = panel(&frame_of(&mut app, 160, 45));
    assert!(back[0].contains(" · rendered · "), "{}", back[0]);
    assert_eq!(back[1], "Title");
    assert!(app.model.reader_open());
    assert_eq!(
        take_keys(&mut app, vec![(input::Key::Escape, Instant::now())]),
        Some(true)
    );
    assert!(!app.model.reader_open());
    assert!(matches!(rx.try_recv(), Ok(Request::Read(None))));
    assert_eq!(app.model.turn(), mark, "the mark never moved");
    assert!(
        whole(&frame_of(&mut app, 160, 45)).contains("docs/r.md"),
        "the lane is back"
    );
}

#[test]
fn a_line_link_into_markdown_opens_raw_at_that_line_zero_included() {
    for (turn, line) in [("see docs/r.md:3 now", 3), ("see docs/r.md:0 now", 0)] {
        let (mut app, rx) = marked_app(turn);
        let ask = opened(&mut app, &rx);
        assert_eq!(ask.line, Some(line));
        answer(&mut app, ask.generation, REPORT);
        let cells = frame_of(&mut app, 160, 45);
        let shown = panel(&cells);
        assert!(
            shown[0].starts_with(&format!("Read · /w/docs/r.md:{line} · raw · rows")),
            "{}",
            shown[0]
        );
        assert_eq!(shown[1], "1 # Title");
        let _ = pressed(&mut app, b"m");
        let rendered = panel(&frame_of(&mut app, 160, 45));
        assert!(
            rendered[0].contains(&format!("r.md:{line} · rendered · ")),
            "{}",
            rendered[0]
        );
    }
}

#[test]
fn any_other_file_opens_exactly_as_today_and_m_is_inert_there() {
    let (mut app, rx) = marked_app("see src/b.rs now");
    let ask = opened(&mut app, &rx);
    answer(&mut app, ask.generation, "fn a() {}\n# not a heading");
    let shown = panel(&frame_of(&mut app, 160, 45));
    assert_eq!(
        shown[0], "Read · /w/src/b.rs · rows 1-2/2",
        "today's header bytes"
    );
    assert_eq!(shown[1], "1 fn a() {}");
    assert_eq!(shown[2], "2 # not a heading");
    let _ = pressed(&mut app, b"m");
    assert!(rx.try_recv().is_err());
    assert_eq!(
        panel(&frame_of(&mut app, 160, 45)),
        shown,
        "m changes nothing"
    );
    assert!(app.model.reader_open());
}

#[test]
fn m_in_browse_with_no_reader_is_inert() {
    let (mut app, rx) = marked_app("report at docs/r.md done");
    let mark = app.model.turn();
    let _ = pressed(&mut app, b"m");
    assert!(rx.try_recv().is_err(), "m asks nothing");
    assert!(!app.model.reader_open(), "m opens nothing");
    assert_eq!(app.model.turn(), mark, "the mark stays");
    let shown = whole(&frame_of(&mut app, 160, 45));
    assert!(!shown.contains("Read · "), "no reader is drawn");
    assert!(
        !shown.contains("too large to render"),
        "and nothing is flashed"
    );
}

#[test]
fn upper_case_and_long_extensions_open_rendered_with_the_links_suffix() {
    let (mut app, rx) = marked_app("see docs/R.MARKDOWN and docs/other.md");
    let ask = opened(&mut app, &rx);
    assert_eq!(ask.links, 2);
    answer(&mut app, ask.generation, REPORT);
    let shown = panel(&frame_of(&mut app, 160, 45));
    assert!(
        shown[0].starts_with("Read · /w/docs/R.MARKDOWN · rendered · rows"),
        "{}",
        shown[0]
    );
    assert!(shown[0].ends_with(" · first of 2 links"), "{}", shown[0]);
    assert_eq!(shown[1], "Title");
}

fn headings(count: usize) -> String {
    (1..=count)
        .map(|n| format!("# H{n:03}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn toggle_and_resize_keep_the_top_source_line() {
    let (mut app, rx) = marked_app("jump docs/h.md:150 here");
    let ask = opened(&mut app, &rx);
    answer(&mut app, ask.generation, &headings(300));
    let raw = panel(&frame_of(&mut app, 160, 45));
    assert_eq!(raw[1], "147 # H147", "today's jump: three lines of context");
    let _ = pressed(&mut app, b"m");
    let rendered = panel(&frame_of(&mut app, 160, 45));
    assert_eq!(
        rendered[1], "H147",
        "raw top line -> its block's first rendered row"
    );
    let resized = panel(&frame_of(&mut app, 120, 40));
    assert_eq!(resized[1], "H147", "a resize keeps the anchor");
    let _ = pressed(&mut app, b"m");
    let raw = panel(&frame_of(&mut app, 120, 40));
    assert_eq!(raw[1], "147 # H147", "rendered top block -> its raw line");
}

#[test]
fn rendered_scrolls_with_the_reader_keys() {
    let (mut app, rx) = marked_app("see docs/h.md now");
    let ask = opened(&mut app, &rx);
    answer(&mut app, ask.generation, &headings(300));
    assert_eq!(panel(&frame_of(&mut app, 160, 45))[1], "H001");
    let _ = pressed(&mut app, b"G");
    let bottom = panel(&frame_of(&mut app, 160, 45));
    assert!(bottom.iter().any(|row| row == "H300"), "{bottom:?}");
    assert!(!bottom.iter().any(|row| row == "H001"));
    let _ = pressed(&mut app, b"g");
    assert_eq!(panel(&frame_of(&mut app, 160, 45))[1], "H001");
    let _ = pressed(&mut app, b"j");
    assert_ne!(
        panel(&frame_of(&mut app, 160, 45))[1],
        "H001",
        "j scrolls a row"
    );
}

#[test]
fn a_document_past_a_global_budget_opens_raw_and_m_says_why() {
    let src = format!("```\n{}```", "c\n".repeat(30_000));
    let (mut app, rx) = marked_app("see docs/big.md now");
    let ask = opened(&mut app, &rx);
    answer(&mut app, ask.generation, &src);
    let cells = frame_of(&mut app, 250, 45);
    let shown = panel(&cells);
    assert!(
        shown[0]
            .starts_with("Read · /w/docs/big.md · raw: too large to render (cells limit) · rows"),
        "{}",
        shown[0]
    );
    assert!(
        shown[1].ends_with("1 ```"),
        "raw, gutter kept: {}",
        shown[1]
    );
    let _ = pressed(&mut app, b"m");
    let cells = frame_of(&mut app, 250, 45);
    assert!(
        whole(&cells).matches("too large to render").count() >= 2,
        "the hint row says why, beside the header"
    );
    assert!(panel(&cells)[0].contains(" · raw: too large to render (cells limit) · "));
    assert!(rx.try_recv().is_err(), "nothing is read again");
}
