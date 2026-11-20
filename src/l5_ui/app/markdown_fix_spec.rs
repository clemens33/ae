//! Navigator oracle P1–P5, nav-review-p1-b1.md (`CommonMark` order and fidelity).
//! #272 spacing amendment: mdspace-nav, `OpenAI` / GPT-6.1-Sol.
//! Lead Q4 ruling ae-20261028T142653Z-d2f90c98: tight nested lists stay adjacent;
//! loose items/quotes separate children. Code/table/quote/rule/heading children
//! of ANY item get one separator before/after. Literal code/HTML blanks remain.
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
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[0].trim(), "p1");
    assert_eq!(rows[1], "");
    assert!(rows[2].contains("| a | b |"));
    assert!(rows[3].contains("| 1 | 2 |"));
    assert_eq!(rows[4], "");
    assert_eq!(rows[5].trim(), "p2");
    assert_eq!((r.line_of(2), r.line_of(3)), (2, 4));
    assert!((1..r.rows()).all(|row| r.line_of(row - 1) <= r.line_of(row)));
}

#[test]
fn two_tables_in_one_quote_keep_their_own_columns() {
    let r = render(
        "> | a | b | c |\n> |---|---|---|\n> | 1 | 2 | 3 |\n>\n> | x |\n> |---|\n> | y |\n",
        40,
    );
    let rows = texts(&r);
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0].matches('|').count(), 4);
    assert_eq!(rows[2], "");
    assert_eq!(rows[3].matches('|').count(), 2);
    assert!(rows[0].contains('a') && rows[1].contains('1'));
    assert!(rows[3].contains('x') && rows[4].contains('y'));
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

#[test]
fn sibling_block_kinds_have_exactly_one_separator() {
    let rule = "-".repeat(40);
    let mut failures = Vec::new();
    let kinds = [
        ("# head\n", "head"),
        ("para\n", "para"),
        ("```\ncode\n```\n", "code"),
        ("- item\n", "- item"),
        ("| a |\n|---|\n", "| a |"),
        ("> quote\n", "  quote"),
        ("---\n", rule.as_str()),
    ];
    for (left, a) in kinds {
        for (right, b) in kinds {
            // Different list markers force two sibling lists, rather than one
            // loose list. Other adjacent kinds stay independent Markdown blocks.
            let (right, b) = if left == "- item\n" && right == left {
                ("1. item\n", "1. item")
            } else {
                (right, b)
            };
            let source = format!("\n\n{left}\n\n{right}\n\n");
            let r = render(&source, 40);
            assert_eq!(r.refusal(), None, "{source:?}");
            if texts(&r) != [a, "", b] {
                failures.push(format!(
                    "{source:?}: {:?}, want {:?}",
                    texts(&r),
                    [a, "", b]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn raw_table_fallback_is_separated_from_its_rendered_neighbors() {
    let source = "before\n\n| abcdefghi | jklmnopqr |\n|---|---|\n| x | y |\n\nafter\n";
    let r = render(source, 20);
    let rows = texts(&r);
    assert_eq!(r.refusal(), None);
    assert_eq!(r.notes(), 0, "viewport fallback has no refusal note");
    assert_eq!(rows[0], "before");
    assert_eq!(rows[1], "");
    assert_eq!(rows[rows.len() - 2], "");
    assert_eq!(rows[rows.len() - 1], "after");
    assert!(
        rows.iter().any(|row| row.contains("---")),
        "raw delimiter remains"
    );
}

#[test]
fn tight_nested_lists_keep_adjacent_items() {
    let r = render("- a\n  - b\n  - c\n- d\n", 40);
    assert_eq!(texts(&r), ["- a", "  - b", "  - c", "- d"]);
    let ordered = render("3. a\n4. b\n", 40);
    assert_eq!(texts(&ordered), ["3. a", "4. b"]);
}

#[test]
fn loose_items_separate_paragraphs_nested_lists_and_siblings() {
    let r = render("- a\n\n  continued\n\n  - b\n  - c\n\n- d\n", 40);
    assert_eq!(
        texts(&r),
        ["- a", "", "  continued", "", "  - b", "  - c", "", "- d"]
    );
}

#[test]
fn loose_list_separates_items_even_when_first_items_have_only_nested_lists() {
    for (source, expected) in [
        (
            "-\n  - nested\n\n- following\n",
            ["  - nested", "", "- following"],
        ),
        (
            "-\n  - first\n\n-\n  - second\n",
            ["  - first", "", "  - second"],
        ),
    ] {
        assert_eq!(texts(&render(source, 40)), expected, "{source:?}");
    }
}

#[test]
fn loose_nested_only_lists_space_all_items_even_before_the_first_blank_source_line() {
    let r = render("-\n  - a\n-\n  - b\n\n-\n  - c\n", 40);
    assert_eq!(texts(&r), ["  - a", "", "  - b", "", "  - c"]);
}

#[test]
fn quoted_loose_nested_only_items_keep_one_separator() {
    let r = render("> -\n>   - a\n>\n> -\n>   - b\n", 40);
    assert_eq!(texts(&r), ["    - a", "", "    - b"]);
}

#[test]
fn blank_between_child_lists_makes_the_enclosing_list_loose() {
    let r = render("-\n  - a\n\n  1. b\n-\n  - c\n", 40);
    assert_eq!(texts(&r), ["  - a", "", "  1. b", "", "  - c"]);
}

#[test]
fn inner_looseness_does_not_add_separators_to_the_tight_outer_list() {
    let r = render("- a\n  - b\n\n  - c\n- d\n", 40);
    assert_eq!(texts(&r), ["- a", "  - b", "", "  - c", "- d"]);
}

#[test]
fn tight_item_code_child_has_a_separator_on_both_sides() {
    let r = render("- before\n  ```\n  code\n  ```\n  after\n- last\n", 40);
    assert_eq!(
        texts(&r),
        ["- before", "", "  code", "", "  after", "- last"]
    );
    assert!(r.marks(2, 2).code_block);
    assert!(!r.marks(4, 2).code_block);
}

#[test]
fn quote_children_space_blocks_but_keep_their_tight_list_tight() {
    let r = render(
        "> # head\n>\n> para\n>\n> - a\n> - b\n>\n> ```\n> code\n> ```\n",
        40,
    );
    assert_eq!(
        texts(&r),
        ["  head", "", "  para", "", "  - a", "  - b", "", "  code"]
    );
}

#[test]
fn tight_item_special_children_space_without_loosening_other_items() {
    for child in ["  # head\n", "  > quote\n", "  ***\n", "  | a |\n  |---|\n"] {
        let source = format!("- before\n{child}- after\n- last\n");
        let r = render(&source, 40);
        let rows = texts(&r);
        assert_eq!(rows.len(), 6, "{source:?}: {rows:?}");
        assert_eq!(rows[0], "- before");
        assert_eq!(rows[1], "");
        assert!(!rows[2].is_empty());
        assert_eq!(&rows[3..], ["", "- after", "- last"]);
    }
}

#[test]
fn soft_hard_and_wrapped_lines_are_not_block_boundaries() {
    assert_eq!(texts(&render("one\ntwo\n", 40)), ["one two"]);
    assert_eq!(texts(&render("one  \ntwo\n", 40)), ["one", "two"]);
    assert_eq!(
        texts(&render("one two three\n", 5)),
        ["one", "two", "three"]
    );
}

#[test]
fn separators_have_no_marks_and_preserve_source_navigation_at_every_width() {
    let source = "# head\n\n[link](https://x.io)\n\n```\ncode\n```\n\n> quote\n";
    for width in [1, 3, 8, 40] {
        let r = render(source, width);
        assert_eq!(r.refusal(), None);
        let rows = texts(&r);
        assert!(!rows.first().is_none_or(|s| s.is_empty()));
        assert!(!rows.last().is_none_or(|s| s.is_empty()));
        assert_eq!(rows.iter().filter(|s| s.is_empty()).count(), 3);
        assert!(rows.windows(2).all(|pair| pair != ["", ""]));
        for (row, text) in rows.iter().enumerate() {
            assert!(cells(text) <= width, "{width}: {text:?}");
            if text.is_empty() {
                for byte in [0, 1, width] {
                    assert_eq!(r.marks(row, byte), Marks::default());
                }
            }
            if row > 0 {
                assert!(r.line_of(row - 1) <= r.line_of(row));
            }
        }
        for line in [1, 3, 6, 9] {
            assert!(!r.text(r.row_of(line)).unwrap_or_default().is_empty());
        }
    }
}

#[test]
fn separator_rows_are_charged_to_the_row_budget() {
    let fits = render(&"x\n\n".repeat(25_000), 1);
    assert_eq!(fits.refusal(), None);
    assert_eq!(fits.rows(), 49_999);
    let over = render(&"x\n\n".repeat(25_001), 1);
    assert_eq!(over.refusal(), Some("too large to render (rows limit)"));
    assert_eq!(over.rows(), 0, "global refusal clears partial rows");
}

#[test]
fn drawn_separators_remain_blank_selectable_and_part_of_viewport_bounds() {
    use crate::app::{draw::draw_markdown, select};
    use ratatui_core::{
        buffer::Buffer,
        layout::{Position, Rect},
    };
    let source = "# head\n\npara\n\nend\n";
    let lines: Vec<String> = source.lines().map(str::to_owned).collect();
    let r = render(source, 40);
    let mut view = View {
        path: "/w/r.md",
        line: None,
        lines: &lines,
        shown: Shown::Rendered(&r),
        refusal: None,
        top: 0,
        links: 1,
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 4));
    for cell in &mut buf.content {
        cell.set_symbol("X");
    }
    let bounds = draw_markdown(&view, None, &mut buf);
    assert_eq!((bounds.visible, bounds.max_back), (3, 2));
    let region = select::Region {
        panel: select::Panel::Peek,
        area: Rect::new(0, 1, 40, 3),
    };
    let mut selection = select::Selection::start(&region, Position::new(0, 1), 0);
    selection.extend(Position::new(3, 3));
    assert_eq!(select::text(&buf, &selection, &[]), "head\n\npara");
    for x in 0..40 {
        assert_eq!(buf[(x, 2)].symbol(), " ");
        assert_eq!(buf[(x, 2)].modifier, ratatui_core::style::Modifier::empty());
    }
    view.top = usize::MAX;
    let clamped = draw_markdown(&view, None, &mut buf);
    assert_eq!(clamped, bounds);
    let body: Vec<String> = (1..4)
        .map(|y| {
            (0..40)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect();
    assert_eq!(body, ["para", "", "end"]);
}
