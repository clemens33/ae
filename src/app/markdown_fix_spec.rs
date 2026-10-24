//! Navigator oracle P1–P5, nav-review-p1-b1.md (`CommonMark` order and fidelity).
use super::*;

fn texts(r: &Rendered) -> Vec<&str> {
    (0..r.rows()).filter_map(|row| r.text(row)).collect()
}

#[test]
fn code_and_literal_html_keep_blank_rows() {
    let code = render("```\na\n\nb\n```\n", 40);
    assert_eq!(texts(&code), ["a", "", "b"]);
    assert!(code.marks(1, 0).code_block);
    let html = render("<pre>\nx\n\ny\n</pre>\n", 40);
    assert_eq!(texts(&html), ["<pre>", "x", "", "y", "</pre>"]);
}

#[test]
fn quoted_table_keeps_document_order_and_source_anchors() {
    let r = render(
        "> p1\n>\n> | a | b |\n> |---|---|\n> | 1 | 2 |\n>\n> p2\n",
        40,
    );
    let rows = texts(&r);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].trim(), "p1");
    assert!(rows[1].contains("| a | b |"));
    assert!(rows[2].contains("| 1 | 2 |"));
    assert_eq!(rows[3].trim(), "p2");
    assert_eq!((r.line_of(1), r.line_of(2)), (2, 4));
    assert!((1..r.rows()).all(|row| r.line_of(row - 1) <= r.line_of(row)));
}

#[test]
fn two_tables_in_one_quote_keep_their_own_columns() {
    let r = render(
        "> | a | b | c |\n> |---|---|---|\n> | 1 | 2 | 3 |\n>\n> | x |\n> |---|\n> | y |\n",
        40,
    );
    let rows = texts(&r);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].matches('|').count(), 4);
    assert_eq!(rows[2].matches('|').count(), 2);
    assert!(rows[0].contains('a') && rows[1].contains('1'));
    assert!(rows[2].contains('x') && rows[3].contains('y'));
}

#[test]
fn quote_exit_keeps_the_enclosing_list_indent() {
    let r = render("- a\n\n  > q\n\n  b\n", 40);
    let rows = texts(&r);
    let b = rows.iter().find(|row| row.trim() == "b").copied();
    assert_eq!(b, Some("  b"));
    let q = rows.iter().find(|row| row.trim() == "q").copied();
    assert!(q.is_some_and(|q| q.len() - q.trim_start().len() > 2));
}

#[test]
fn raw_markdown_resets_the_populated_panel() {
    use crate::app::draw::draw_markdown;
    use ratatui_core::{buffer::Buffer, layout::Rect};
    let lines = vec!["one".to_owned()];
    let flow = reader::flow(&lines, 38);
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 5));
    for cell in &mut buf.content {
        cell.set_symbol("X");
    }
    let _ = draw_markdown(
        &View {
            path: "/w/r.md",
            line: None,
            lines: &lines,
            shown: Shown::Raw(&flow),
            refusal: None,
            top: 0,
            links: 1,
        },
        None,
        &mut buf,
    );
    assert!(buf.content.iter().all(|cell| cell.symbol() != "X"));
}
