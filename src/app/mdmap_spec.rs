//! Frozen characterisation spec: mdmap phase 1 (#300), navigator mdmap-nav
//! (Meta / muse-wonder-full-1.0-contributor), base 86674177. Oracle per case:
//! `CommonMark`/GFM structure or an ae ruling where one states it, else the
//! current base output, labelled a behaviour pin in the test name or comment.
//! Bug-shaped current behaviour is pinned, never fixed here; the two cases
//! are BUG? below. Driver installs this file byte-identically; the wiring is
//! the `mdmap_spec` child mod at the end of `src/app/markdown.rs`, beside
//! `fix_spec`. Expected dumps were taken from base probes of the unrefactored
//! `Mapper::map` (`.local/mdmap/probe1-base.log`, `probe2-base.log`).
//!
//! BUG? (pinned, reported for #300, not fixed by the refactor):
//! 1. An HTML block interrupting a tight list item's paragraph glues the
//!    words across the block boundary (`before<div>`), pinned by
//!    `mapped_html_block_in_tight_item_glues_to_the_open_row_pin`.
//! 2. HTML block rows keep `line 0` in `Mapped` and anchor `line_of` to the
//!    previous block instead of their own source line, pinned by
//!    `html_block_rows_anchor_to_the_previous_block_pin`.
//!
//! Amendment (mutants round, same navigator): group 4 pins the four
//! `cargo-mutants --in-diff` survivors on the refactored tree (78 tested,
//! 73 caught, 1 unviable let-chain `||` in `close_link`); each pin was shown
//! RED against its own mutant before this freeze.

use std::fmt::Write as _;

use super::*;
use pulldown_cmark::{CodeBlockKind, HeadingLevel};

fn texts(r: &Rendered) -> Vec<&str> {
    (0..r.rows())
        .map(|row| r.text(row).unwrap_or("?"))
        .collect()
}

fn marks_of(r: &Rendered, needle: &str) -> Marks {
    for row in 0..r.rows() {
        if let Some(at) = r.text(row).unwrap_or_default().find(needle) {
            return r.marks(row, at);
        }
    }
    panic!("{needle:?} not drawn in {:?}", texts(r));
}

/// One flag string per `Marks` value: `H<level>` or `-`, then one letter per
/// set field in field order (strong, emphasis, strike, code, `code_block`,
/// quote, link, target, html, note).
fn mstr(m: Marks) -> String {
    let mut s = String::new();
    match m.heading {
        Some(level) => {
            let _ = write!(s, "H{level}");
        }
        None => s.push('-'),
    }
    for (on, ch) in [
        (m.strong, 's'),
        (m.emphasis, 'e'),
        (m.strike, '~'),
        (m.code, 'c'),
        (m.code_block, 'B'),
        (m.quote, 'q'),
        (m.link, 'l'),
        (m.target, 't'),
        (m.html, 'h'),
        (m.note, 'n'),
    ] {
        if on {
            s.push(ch);
        }
    }
    s
}

fn rowstr(row: &Row) -> String {
    let marks: Vec<String> = row
        .marks
        .iter()
        .map(|(at, m)| format!("{at}:{}", mstr(*m)))
        .collect();
    format!("line={} |{}| [{}]", row.line, row.text, marks.join(","))
}

fn mapped_dump(mapped: &Mapped) -> Vec<String> {
    let mut out = Vec::new();
    for item in &mapped.items {
        match item {
            Item::Line(row, hang, code) => {
                out.push(format!("L hang={hang} code={code} {}", rowstr(row)));
            }
            Item::Table(table) => {
                out.push(format!("T hang={} align={:?}", table.hang, table.align));
                for trow in &table.rows {
                    let cells: Vec<String> = trow.iter().map(rowstr).collect();
                    out.push(format!("  TR [{}]", cells.join("] [")));
                }
            }
            Item::Rule(line) => out.push(format!("RULE line={line}")),
            Item::Gap => out.push("G".to_owned()),
        }
    }
    out
}

/// The `Mapped` dump of every parsed block, in `Document` order.
fn dump_source(source: &str, opts: Opts) -> Vec<String> {
    let doc = Document::parse(source);
    doc.blocks
        .iter()
        .flat_map(|block| mapped_dump(&Mapper::new(&doc, block, opts).map()))
        .collect()
}

fn dump_hand(source: &str, events: Vec<Event<'static>>, opts: Opts) -> Vec<String> {
    let mut starts = vec![0usize];
    starts.extend(source.match_indices('\n').map(|(at, _)| at + 1));
    let doc = Document {
        source: source.to_owned(),
        starts,
        blocks: vec![Block {
            range: 0..source.len(),
            events: events.into_iter().map(|event| (event, 0..0)).collect(),
        }],
    };
    mapped_dump(&Mapper::new(&doc, &doc.blocks[0], opts).map())
}

// ---------------------------------------------------------------------------
// Group 1: `Mapped` dumps over parsed sources (the order traps render masks)
// ---------------------------------------------------------------------------

#[test]
fn mapped_code_rows_keep_the_code_flag_and_blank_lines_are_space_rows_pin() {
    assert_eq!(
        dump_source("```\na\n\n\n```\n\nafter\n", Opts::default()),
        [
            "G",
            "L hang=0 code=true line=0 |a| [0:-B]",
            "L hang=0 code=true line=0 | | [0:-B]",
            "L hang=0 code=true line=0 | | [0:-B]",
            "G",
            "L hang=0 code=false line=6 |after| [0:-]",
        ]
    );
}

#[test]
fn mapped_heading_in_quoted_item_separates_and_clears_pin() {
    assert_eq!(
        dump_source("> - # head\n> - b\n", Opts::default()),
        [
            "G",
            "G",
            "L hang=4 code=false line=0 |  - head| [0:-q,4:H1q]",
            "G",
            "L hang=4 code=false line=1 |  - b| [0:-q]",
            "G",
        ]
    );
}

#[test]
fn mapped_table_in_loose_item_captures_item_hang_and_restores_it_pin() {
    assert_eq!(
        dump_source(
            "- pre\n\n  | a |\n  |---|\n  | 1 |\n\n  post\n- next\n",
            Opts::default()
        ),
        [
            "L hang=2 code=false line=0 |- pre| [0:-]",
            "G",
            "T hang=2 align=[None]",
            "  TR [line=2 |a| [0:-]]",
            "  TR [line=4 |1| [0:-]]",
            "G",
            "G",
            "L hang=2 code=false line=6 |  post| [0:-]",
            "G",
            "L hang=2 code=false line=7 |- next| [0:-]",
        ]
    );
}

#[test]
fn mapped_html_block_in_tight_item_glues_to_the_open_row_pin() {
    // BUG? 1: the HTML block is a separate block in `CommonMark`, yet its
    // text is appended to the still-open item row with no space or break.
    assert_eq!(
        dump_source("- before\n  <div>x</div>\n- after\n", Opts::default()),
        [
            "L hang=2 code=false line=0 |- before<div>x</div>| [0:-,8:-h]",
            "L hang=2 code=false line=2 |- after| [0:-]",
        ]
    );
}

#[test]
fn mapped_empty_heading_is_only_gaps_and_image_in_link_marks_stack_pin() {
    assert_eq!(dump_source("#\n\npara\n", Opts::default())[..2], ["G", "G"]);
    assert_eq!(texts(&render("#\n\npara\n", 40)), ["para"]);
    assert_eq!(
        dump_source("[![alt](pic.png)](https://x.io)\n", Opts::default()),
        ["L hang=0 code=false line=0 |[image: alt](pic.png) (https://x.io)| [0:-,0:-l,11:-lt]"]
    );
}

// ---------------------------------------------------------------------------
// Group 2: rendered rows over parsed sources
// ---------------------------------------------------------------------------

#[test]
fn ordered_ordinals_use_their_own_width_and_resume_after_a_nested_list() {
    assert_eq!(
        texts(&render("9. nine\n10. ten\n11. eleven\n", 40)),
        ["9. nine", "10. ten", "11. eleven"]
    );
    assert_eq!(
        texts(&render("10. word word word word word word\n", 16)),
        ["10. word word", "    word word", "    word word"],
        "continuations hang under the 4-cell bullet"
    );
    assert_eq!(
        texts(&render("0) zero\n1) one\n", 40)),
        ["0. zero", "1. one"],
        "paren delimiters draw as `n. ` from the list start"
    );
    assert_eq!(
        texts(&render("1. a\n   1. i1\n   2. i2\n2. b\n", 40)),
        ["1. a", "  1. i1", "  2. i2", "2. b"],
        "the outer ordinal resumes after the nested list ends"
    );
}

#[test]
fn a_changed_bullet_char_starts_a_new_separated_list() {
    assert_eq!(
        texts(&render("- a\n* b\n- c\n", 40)),
        ["- a", "", "- b", "", "- c"]
    );
}

#[test]
fn container_exits_restore_the_enclosing_indent_exactly() {
    assert_eq!(
        texts(&render(
            "> | a |\n> |---|\n> | 1 |\n>\n> after\n\nplain\n",
            40
        )),
        ["  | a |", "  | 1 |", "", "  after", "", "plain"]
    );
    assert_eq!(
        texts(&render("> > deep\n>\n> shallow\n\nplain\n", 40)),
        ["    deep", "", "  shallow", "", "plain"]
    );
    assert_eq!(
        texts(&render("> - a\n> - b\n>\n> para\n", 40)),
        ["  - a", "  - b", "", "  para"]
    );
    assert_eq!(
        texts(&render("- before\n  ***\n  after\n- last\n", 20)),
        [
            "- before",
            "",
            "--------------------",
            "",
            "  after",
            "- last"
        ],
        "a rule inside an item spans the full width, undented"
    );
}

#[test]
fn code_block_edges_pin() {
    let trailing = render("```\na\n\n\n```\n\nafter\n", 40);
    assert_eq!(texts(&trailing), ["a", "", "", "", "after"]);
    assert!(
        trailing.cut(1).is_some() && trailing.pad(1) == 40,
        "a literal code blank is a padded code row, not a separator"
    );
    assert!(trailing.cut(3).is_none(), "row 3 is the block separator");
    assert_eq!(
        texts(&render("before\n\n```\ncode one\ncode two\n", 40)),
        ["before", "", "code one", "code two"],
        "an unclosed fence still maps its code rows"
    );
    assert_eq!(
        texts(&render("before\n\n```\n```\n\nafter\n", 40)),
        ["before", "", "after"],
        "an empty fence contributes no row"
    );
}

#[test]
fn table_alignment_cell_marks_and_ragged_rows_pin() {
    assert_eq!(
        texts(&render(
            "| h1 | h222 |\n|:--:|-----|\n| x | y |\n| longcell | z |\n",
            60
        )),
        [
            "|    h1    | h222 |",
            "|    x     | y    |",
            "| longcell | z    |"
        ],
        "a centered column splits its padding, left half first"
    );
    let marked = render(
        "| **b** | `c` |\n|---|---|\n| [lab](https://x.io) | *e* |\n",
        80,
    );
    assert_eq!(
        marks_of(&marked, "b"),
        Marks {
            strong: true,
            ..Marks::default()
        }
    );
    assert_eq!(
        marks_of(&marked, "c"),
        Marks {
            code: true,
            ..Marks::default()
        }
    );
    assert_eq!(
        marks_of(&marked, "lab"),
        Marks {
            link: true,
            ..Marks::default()
        }
    );
    assert_eq!(
        marks_of(&marked, "https://x.io"),
        Marks {
            link: true,
            target: true,
            ..Marks::default()
        }
    );
    assert_eq!(
        texts(&render(
            "| a | b | c |\n|---|---|---|\n| 1 |\n| | | |\n",
            60
        )),
        ["| a | b | c |", "| 1 |   |   |", "|   |   |   |"]
    );
    assert_eq!(
        texts(&render("| a \\| b | c |\n|---|---|\n| 1 | 2 |\n", 60)),
        ["| a | b | c |", "| 1     | 2 |"],
        "an escaped pipe stays inside its cell"
    );
}

#[test]
fn link_forms_render_label_and_target_exactly_pin() {
    assert_eq!(
        texts(&render(
            "mail <a@b.co> and [t](https://x.io \"Title\") and [e]() end\n",
            120
        )),
        ["mail a@b.co and t (https://x.io) and e () end"],
        "email autolink once, title dropped, empty destination kept"
    );
    assert_eq!(
        texts(&render(
            "[col][] and [short] and ![rimg][i]\n\n[col]: https://c.io\n[short]: https://s.io\n[i]: img.png\n",
            120
        )),
        ["col (https://c.io) and short (https://s.io) and [image: rimg](img.png)"]
    );
    let empty_alt = render("x ![](pic.png) y\n", 80);
    assert_eq!(texts(&empty_alt), ["x [image: ](pic.png) y"]);
    assert_eq!(
        marks_of(&empty_alt, "pic.png"),
        Marks {
            target: true,
            ..Marks::default()
        },
        "an image target wears no link mark, only the target mark"
    );
}

#[test]
fn nested_marks_restore_by_stack_pin() {
    let heading = render("# Head **bold** tail *em* end\n\npara\n", 60);
    assert_eq!(
        marks_of(&heading, "bold"),
        Marks {
            heading: Some(1),
            strong: true,
            ..Marks::default()
        }
    );
    assert_eq!(
        marks_of(&heading, "tail"),
        Marks {
            heading: Some(1),
            ..Marks::default()
        },
        "closing strong inside a heading restores heading-only marks"
    );
    assert_eq!(marks_of(&heading, "para"), Marks::default());
    let strong_link = render("**bold [lab](https://x.io) still** after\n", 80);
    assert_eq!(
        marks_of(&strong_link, "lab"),
        Marks {
            strong: true,
            link: true,
            ..Marks::default()
        }
    );
    assert_eq!(
        marks_of(&strong_link, "https://x.io"),
        Marks {
            strong: true,
            link: true,
            target: true,
            ..Marks::default()
        },
        "the target is written before the saved marks restore"
    );
    assert_eq!(
        marks_of(&strong_link, "still"),
        Marks {
            strong: true,
            ..Marks::default()
        }
    );
    assert_eq!(marks_of(&strong_link, "after"), Marks::default());
    let strikes = render("**bold ~~gone~~ tail** plain ~~**both**~~ end\n", 80);
    assert_eq!(
        marks_of(&strikes, "gone"),
        Marks {
            strong: true,
            strike: true,
            ..Marks::default()
        }
    );
    assert_eq!(
        marks_of(&strikes, "both"),
        Marks {
            strong: true,
            strike: true,
            ..Marks::default()
        }
    );
    assert_eq!(marks_of(&strikes, "end"), Marks::default());
}

#[test]
fn lane_soft_breaks_flush_with_container_hang_and_backslash_breaks_hard() {
    let lane = Opts {
        soft_break_row: true,
    };
    assert_eq!(
        texts(&render_with(
            "- alpha\n  beta\n- next\n",
            40,
            &Budget::LANE,
            lane
        )),
        ["- alpha", "  beta", "- next"]
    );
    assert_eq!(
        texts(&render_with("> alpha\n> beta\n", 40, &Budget::LANE, lane)),
        ["  alpha", "  beta"]
    );
    assert_eq!(texts(&render("one\\\ntwo\n", 40)), ["one", "two"]);
    assert_eq!(
        texts(&render("a `x\ny` b and ``co`de`` end\n", 60)),
        ["a x y b and co`de end"],
        "a code span joins its soft break as one space"
    );
}

#[test]
fn html_block_rows_anchor_to_the_previous_block_pin() {
    // BUG? 2: the HTML block starts at source line 2, but no rendered row
    // anchors there; every HTML row reports the previous block's line.
    let r = render("para one\n\n<div>\nblock\n</div>\n\npara two\n", 40);
    assert_eq!(
        texts(&r),
        ["para one", "", "<div>", "block", "</div>", "", "para two"]
    );
    assert_eq!(
        (0..r.rows()).map(|row| r.line_of(row)).collect::<Vec<_>>(),
        [0, 0, 0, 0, 0, 0, 6]
    );
    let code = render("top\n\n```\nc1\nc2\nc3\n```\n\nbot\n", 40);
    assert_eq!(
        (0..code.rows())
            .map(|row| code.line_of(row))
            .collect::<Vec<_>>(),
        [0, 0, 2, 2, 2, 2, 8],
        "by contrast every code row anchors its own block start"
    );
}

// ---------------------------------------------------------------------------
// Group 3: hand-built event streams the parser never emits (behaviour pins)
// ---------------------------------------------------------------------------

#[test]
fn hand_ends_without_starts_pop_nothing_and_stay_total_pin() {
    assert_eq!(
        dump_hand(
            "x",
            vec![
                Event::End(TagEnd::Emphasis),
                Event::End(TagEnd::Link),
                Event::End(TagEnd::BlockQuote(None)),
                Event::End(TagEnd::List(true)),
                Event::Start(Tag::Paragraph),
                Event::Text("x".into()),
                Event::End(TagEnd::Paragraph),
            ],
            Opts::default()
        ),
        ["G", "L hang=0 code=false line=0 |x| [0:-]"]
    );
}

#[test]
fn hand_stacks_are_kind_blind_pin() {
    assert_eq!(
        dump_hand(
            "ab",
            vec![
                Event::Start(Tag::Paragraph),
                Event::Start(Tag::Emphasis),
                Event::Text("a".into()),
                Event::End(TagEnd::Link),
                Event::Text("b".into()),
                Event::End(TagEnd::Emphasis),
                Event::End(TagEnd::Paragraph),
            ],
            Opts::default()
        ),
        ["L hang=0 code=false line=0 |ab| [0:-,0:-e,1:-]"],
        "End Link pops the emphasis save even with no link open"
    );
    assert_eq!(
        dump_hand(
            "alt",
            vec![
                Event::Start(Tag::Paragraph),
                Event::Start(Tag::Image {
                    link_type: LinkType::Inline,
                    dest_url: "pic.png".into(),
                    title: "".into(),
                    id: "".into(),
                }),
                Event::Text("alt".into()),
                Event::End(TagEnd::Link),
                Event::End(TagEnd::Paragraph),
            ],
            Opts::default()
        ),
        ["L hang=0 code=false line=0 |[image: alt](pic.png)| [0:-,11:-t]"],
        "the `](` separator comes from the popped tuple, not the end tag"
    );
}

#[test]
fn hand_item_without_list_and_table_bits_without_table_pin() {
    assert_eq!(
        dump_hand(
            "x",
            vec![
                Event::Start(Tag::Item),
                Event::Text("x".into()),
                Event::End(TagEnd::Item),
            ],
            Opts::default()
        ),
        ["L hang=2 code=false line=0 |- x| [0:-]"]
    );
    assert_eq!(
        dump_hand(
            "x",
            vec![
                Event::Start(Tag::TableRow),
                Event::Start(Tag::Paragraph),
                Event::Text("x".into()),
                Event::End(TagEnd::TableCell),
                Event::End(TagEnd::Paragraph),
            ],
            Opts::default()
        ),
        ["L hang=0 code=false line=0 |x| [0:-]"]
    );
    assert_eq!(
        dump_hand(
            "c p",
            vec![
                Event::Start(Tag::Table(vec![Alignment::None])),
                Event::Start(Tag::TableCell),
                Event::Text("c".into()),
                Event::End(TagEnd::TableCell),
                Event::End(TagEnd::Table),
                Event::Start(Tag::Paragraph),
                Event::Text("p".into()),
                Event::End(TagEnd::Paragraph),
            ],
            Opts::default()
        ),
        [
            "G",
            "T hang=0 align=[None]",
            "L hang=0 code=false line=0 |c| [0:-]",
            "G",
            "L hang=0 code=false line=0 |p| [0:-]",
        ],
        "a cell with no row survives as the open row until the table end"
    );
}

#[test]
fn hand_text_code_and_break_events_outside_their_blocks_pin() {
    assert_eq!(
        dump_hand(
            "a b",
            vec![
                Event::Text("a".into()),
                Event::HardBreak,
                Event::Text("b".into()),
            ],
            Opts::default()
        ),
        [
            "L hang=0 code=false line=0 |a| [0:-]",
            "L hang=0 code=false line=0 |b| [0:-]",
        ]
    );
    assert_eq!(
        dump_hand(
            "a\nb",
            vec![
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced("".into()))),
                Event::Code("a\nb".into()),
                Event::End(TagEnd::CodeBlock),
            ],
            Opts::default()
        ),
        ["G", "L hang=0 code=true line=0 |a\nb| [0:-B,0:-cB]", "G"],
        "an inline Code event is never split on its newlines, even in code"
    );
    assert_eq!(
        dump_hand(
            "a b",
            vec![
                Event::Start(Tag::Paragraph),
                Event::Text("a".into()),
                Event::SoftBreak,
                Event::Text("b".into()),
                Event::End(TagEnd::Paragraph),
            ],
            Opts {
                soft_break_row: true,
            }
        ),
        [
            "L hang=0 code=false line=0 |a| [0:-]",
            "L hang=0 code=false line=0 |b| [0:-]",
        ]
    );
}

#[test]
fn hand_heading_level_and_email_autolink_pin() {
    assert_eq!(
        dump_hand(
            "h",
            vec![
                Event::Start(Tag::Heading {
                    level: HeadingLevel::H3,
                    id: None,
                    classes: Vec::new(),
                    attrs: Vec::new(),
                }),
                Event::Text("h".into()),
                Event::End(TagEnd::Heading(HeadingLevel::H3)),
            ],
            Opts::default()
        ),
        ["G", "L hang=0 code=false line=0 |h| [0:-,0:H3]", "G"]
    );
    assert_eq!(
        dump_hand(
            "a@b.co",
            vec![
                Event::Start(Tag::Paragraph),
                Event::Start(Tag::Link {
                    link_type: LinkType::Email,
                    dest_url: "a@b.co".into(),
                    title: "".into(),
                    id: "".into(),
                }),
                Event::Text("a@b.co".into()),
                Event::End(TagEnd::Link),
                Event::End(TagEnd::Paragraph),
            ],
            Opts::default()
        ),
        ["L hang=0 code=false line=0 |a@b.co| [0:-,0:-l]"],
        "an email autolink appends no target"
    );
}

// ---------------------------------------------------------------------------
// Group 4: mutants-round amendment (one pin per surviving mutant)
// ---------------------------------------------------------------------------

#[test]
fn quote_marks_survive_the_inner_close_and_end_at_the_outer_one() {
    let r = render("- a\n\n  > > deep\n  >\n  > shallow\n\n  after\n", 40);
    assert_eq!(
        texts(&r),
        ["- a", "", "      deep", "", "    shallow", "", "  after"]
    );
    let quoted = Marks {
        quote: true,
        ..Marks::default()
    };
    assert_eq!(marks_of(&r, "deep"), quoted);
    assert_eq!(
        marks_of(&r, "shallow"),
        quoted,
        "still quoted after only the inner quote closes"
    );
    assert_eq!(
        marks_of(&r, "after"),
        Marks::default(),
        "the item text after the outer close is unquoted"
    );
}

#[test]
fn an_html_block_in_a_quote_keeps_its_separators() {
    assert_eq!(
        texts(&render("> para\n>\n> <div>x</div>\n>\n> after\n", 40)),
        ["  para", "", "  <div>x</div>", "", "  after"],
        "the HTML block start still asks the loose quote scope for a gap"
    );
}

#[test]
fn hand_text_with_a_newline_outside_code_is_never_split_pin() {
    assert_eq!(
        dump_hand(
            "a\nb",
            vec![
                Event::Start(Tag::Paragraph),
                Event::Text("a\nb".into()),
                Event::End(TagEnd::Paragraph),
            ],
            Opts::default()
        ),
        ["L hang=0 code=false line=0 |a\nb| [0:-]"],
        "only code-mode text splits on newlines"
    );
}

// Parser-unreachable (a paragraph end closing a heading): a group-3 style
// behaviour pin; the retire-via-lead rule of the first gate applies.
#[test]
fn hand_paragraph_end_flushes_and_clears_the_heading_pin() {
    assert_eq!(
        dump_hand(
            "a b",
            vec![
                Event::Start(Tag::Paragraph),
                Event::Text("a".into()),
                Event::End(TagEnd::Paragraph),
                Event::Text("b".into()),
            ],
            Opts::default()
        ),
        [
            "L hang=0 code=false line=0 |a| [0:-]",
            "L hang=0 code=false line=0 |b| [0:-]",
        ]
    );
    assert_eq!(
        dump_hand(
            "h x",
            vec![
                Event::Start(Tag::Heading {
                    level: HeadingLevel::H2,
                    id: None,
                    classes: Vec::new(),
                    attrs: Vec::new(),
                }),
                Event::Text("h".into()),
                Event::End(TagEnd::Paragraph),
                Event::Text("x".into()),
            ],
            Opts::default()
        ),
        [
            "G",
            "L hang=0 code=false line=0 |h| [0:-,0:H2]",
            "L hang=0 code=false line=0 |x| [0:-]",
        ]
    );
}
