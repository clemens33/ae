//! #270 acceptance oracle: issue + lead-ruling-r1 + approved plan-r3.
//! Real App frames/input; no renderer stub, no internal collaborator mocks.
use super::*;
use ratatui_core::{buffer::Buffer, layout::Rect, style::Modifier};

fn item(kind: Kind, body: &str) -> Item {
    Item {
        micros: 1_759_500_000_000_000,
        kind,
        body: body.into(),
        record: None,
    }
}
fn said(body: &str) -> Item {
    item(Kind::Said { who: "lead".into() }, body)
}
fn app(items: Vec<Item>) -> App {
    let mut a = App::new(Some("api".into()), None, None, None);
    (a.fleeted, a.loading) = (true, false);
    a.fleet = fleet::Fleet {
        home: Some("api".into()),
        rows: vec![fleet::Row {
            name: "api".into(),
            index: 1,
            mark: theme::Mark::Working,
            needy: false,
            unanswered: None,
            counts: fleet::Counts::Unknown,
            line2: fleet::Line2::NoGoal,
            home: true,
        }],
    };
    a.model = model::Model::new(&a.fleet);
    a.overview.base = link::Base {
        dir: Some("/w".into()),
        home: Some("/h".into()),
    };
    for i in &items {
        a.expanded.insert(a.fold_key(i));
    }
    a.lane = Rc::new(Lane {
        items,
        coverage: Vec::new(),
    });
    a
}
fn frame(a: &mut App, width: u16, height: u16) -> Buffer {
    let mut b = Buffer::empty(Rect::new(0, 0, width, height));
    a.frame(&mut b);
    b
}
fn row(b: &Buffer, y: u16) -> String {
    (0..b.area.width).map(|x| b[(x, y)].symbol()).collect()
}
fn text(b: &Buffer) -> String {
    (0..b.area.height)
        .map(|y| row(b, y))
        .collect::<Vec<_>>()
        .join("\n")
}
fn position(b: &Buffer, needle: &str) -> Position {
    for y in 0..b.area.height {
        if let Some(at) = row(b, y).find(needle) {
            let x = ratatui_core::text::Line::from(&row(b, y)[..at]).width();
            return Position::new(u16::try_from(x).unwrap(), y);
        }
    }
    panic!("missing {needle:?}:\n{}", text(b));
}
fn click(a: &mut App, p: Position) {
    for kind in [MouseKind::Click, MouseKind::Release] {
        a.mouse(Mouse {
            kind,
            column: p.x,
            row: p.y,
        });
    }
}
fn paths(a: &mut App) -> Vec<String> {
    mark(a);
    a.open_reader();
    a.reader.files.iter().map(|f| f.path.clone()).collect()
}
fn mark(a: &mut App) {
    let key = a.keys()[0];
    a.model.set_turn(Some(key));
}

#[test]
fn production_frame_renders_agent_markdown_with_semantic_styles() {
    let body = "# Heading\n\n**strong** *emphasis* ~~strike~~ `inline`\n\n> quote\n\n- first\n  - nested\n\n| key | val |\n|---|---|\n| a | b |\n\n---\n\n[label](https://example.test/a)\n\n![alt](pic.png)\n\n```rust\ncode_line\n\ncode_tail\n```";
    let mut a = app(vec![said(body)]);
    let b = frame(&mut a, 80, 70);
    let t = text(&b);
    for absent in [
        "# Heading",
        "**strong**",
        "*emphasis*",
        "~~strike~~",
        "`inline`",
        "```",
        "![alt]",
    ] {
        assert!(!t.contains(absent), "syntax survives {absent}:\n{t}");
    }
    for present in [
        "Heading",
        "quote",
        "- first",
        "nested",
        "| key | val |",
        "| a",
        "label (https://example.test/a)",
        "[image: alt](pic.png)",
        "code_line",
        "code_tail",
    ] {
        assert!(t.contains(present), "missing {present}:\n{t}");
    }
    for (word, style) in [
        ("Heading", Modifier::BOLD),
        ("strong", Modifier::BOLD),
        ("emphasis", Modifier::ITALIC),
        ("strike", Modifier::CROSSED_OUT),
    ] {
        assert!(
            b[position(&b, word)].modifier.contains(style),
            "{word} semantic style"
        );
    }
    let quote = position(&b, "quote");
    let heading = position(&b, "Heading");
    assert!(quote.x > heading.x, "quote indent remains visible");
    assert!(
        t.lines()
            .any(|r| r.trim().len() > 20 && r.trim().chars().all(|c| c == '-')),
        "rule row"
    );
}

#[test]
fn row_source_table_renders_whole_agent_bodies_and_keeps_flattened_and_human_rows_plain() {
    let whole = [
        Kind::Assistant {
            seat: "lead".into(),
            generation: 0,
        },
        Kind::Said { who: "lead".into() },
        Kind::Answer {
            seat: "lead".into(),
            id: "q".into(),
            follow_up: 0,
            late: false,
            gap: None,
            speaker: None,
        },
    ];
    for kind in whole {
        let mut a = app(vec![item(kind.clone(), "**WHOLE**")]);
        let b = frame(&mut a, 80, 20);
        assert!(!text(&b).contains("**WHOLE**"), "whole {kind:?}");
        assert!(b[position(&b, "WHOLE")].modifier.contains(Modifier::BOLD));
    }
    let plain = [
        Kind::Pane {
            seat: "lead".into(),
            generation: 0,
        },
        Kind::Inbound {
            from: "human:chat".into(),
            to: "lead".into(),
        },
        Kind::Asked {
            to: "lead".into(),
            id: "q".into(),
            uncertain: false,
            gap: None,
            via: None,
        },
        Kind::NotDelivered {
            to: "lead".into(),
            id: "q".into(),
        },
        Kind::Said { who: "ae".into() },
        Kind::Reply {
            from: "lead".into(),
            to: "human:chat".into(),
        },
        Kind::Unadmitted {
            from: "lead".into(),
            id: "q".into(),
            why: "late".into(),
        },
        Kind::NeedsYou {
            seat: "lead".into(),
        },
        Kind::Answer {
            seat: "lead".into(),
            id: "q".into(),
            follow_up: 0,
            late: false,
            gap: Some("body missing".into()),
            speaker: None,
        },
    ];
    for kind in plain {
        let mut a = app(vec![item(kind.clone(), "**PLAIN**")]);
        assert!(
            text(&frame(&mut a, 80, 20)).contains("**PLAIN**"),
            "plain {kind:?}"
        );
    }
    let mut a = app(vec![said(&format!("**CUT**\n{}", "x".repeat(3492)))]);
    assert!(
        !text(&frame(&mut a, 80, 70)).contains("**CUT**"),
        "capped say keeps newlines, still renders"
    );
    assert!(
        lane::preview_why(&a.lane.items[0]).is_some(),
        "copy preview qualifier retained"
    );
}

#[test]
fn card_structure_and_hanging_labels_survive_inline_markdown() {
    let mut a = app(vec![item(
        Kind::Card {
            seat: "lead".into(),
        },
        "**Choose**: 1. **Alpha** | 2. *Beta*. Evidence: `src/a.rs:7`",
    )]);
    let b = frame(&mut a, 80, 30);
    let t = text(&b);
    assert!(
        t.contains("1. Alpha") && t.contains("2. Beta"),
        "labels preserved:\n{t}"
    );
    assert!(
        !t.contains("**") && !t.contains("*Beta*") && !t.contains('`'),
        "part text rendered:\n{t}"
    );
    assert!(b[position(&b, "Alpha")].modifier.contains(Modifier::BOLD));
    assert!(b[position(&b, "Beta")].modifier.contains(Modifier::ITALIC));
    assert_eq!(
        position(&b, "Alpha").x,
        position(&b, "Beta").x,
        "label margins match"
    );
}

#[test]
fn a_failed_card_part_falls_back_for_the_entire_turn() {
    let body = "**Choose**: 1. **Alpha** | 2. [r]: src/a.rs";
    let mut a = app(vec![item(
        Kind::Card {
            seat: "lead".into(),
        },
        body,
    )]);
    let b = frame(&mut a, 80, 30);
    assert!(
        text(&b).contains("**Choose**") && text(&b).contains("**Alpha**"),
        "zero-row definition rejects every part"
    );
}

#[test]
fn lane_keeps_soft_break_rows_while_reader_commonmark_default_joins_them() {
    for body in ["alpha\nbeta", "> alpha\n> beta", "- alpha\n  beta"] {
        let mut a = app(vec![said(body)]);
        let b = frame(&mut a, 80, 25);
        assert!(
            position(&b, "alpha").y < position(&b, "beta").y,
            "lane keeps report lines: {body}"
        );
        let r = markdown::render(body, 78);
        assert!(
            (0..r.rows()).any(|n| r.text(n).is_some_and(|t| t.contains("alpha beta"))),
            "reader default unchanged"
        );
    }
}

#[test]
fn rendered_file_occurrences_drive_r_click_ordinals_and_resize() {
    let body = "[one][r] [two][r]\n\n[r]: src/a.rs\n\n`src/b.rs:7` and [decoded](src/c&#46;rs)";
    let mut a = app(vec![said(body)]);
    let _ = frame(&mut a, 80, 30);
    assert_eq!(
        paths(&mut a),
        ["/w/src/a.rs", "/w/src/a.rs", "/w/src/b.rs", "/w/src/c.rs"]
    );
    a.close_reader();
    let b = frame(&mut a, 80, 30);
    let second = text(&b).lines().position(|r| r.contains("two")).unwrap();
    let mut hits = Vec::new();
    for x in 0..80 {
        if let Some(h) = a.layout.file(Mouse {
            kind: MouseKind::Click,
            column: x,
            row: u16::try_from(second).unwrap(),
        }) && hits
            .last()
            .is_none_or(|(_, old): &(u16, usize)| *old != h.link.index)
        {
            hits.push((x, h.link.index));
        }
    }
    let x = hits
        .iter()
        .find(|(_, index)| *index == 1)
        .expect("second rendered destination clickable")
        .0;
    click(&mut a, Position::new(x, u16::try_from(second).unwrap()));
    assert_eq!(a.reader.index, 1);
    assert_eq!(a.reader.path, "/w/src/a.rs");
    a.close_reader();
    let _ = frame(&mut a, 44, 40);
    assert_eq!(
        paths(&mut a),
        ["/w/src/a.rs", "/w/src/a.rs", "/w/src/b.rs", "/w/src/c.rs"],
        "resize keeps logical occurrences"
    );
    assert!(text(&b).contains("one (src/a.rs)"));
}

#[test]
fn one_wrapped_path_has_one_ordinal_and_every_segment_opens_the_same_file() {
    let path = format!("src/{}.rs:17", "long".repeat(20));
    let mut a = app(vec![said(&format!("`{path}`"))]);
    let b = frame(&mut a, 44, 30);
    let mut segments = Vec::new();
    for y in 2..b.area.height.saturating_sub(5) {
        for x in 0..b.area.width {
            if let Some(hit) = a.layout.file(Mouse {
                kind: MouseKind::Click,
                column: x,
                row: y,
            }) {
                assert_eq!(hit.link.index, 0);
                assert_eq!(hit.link.file.line, Some(17));
                assert!(
                    a.file_list(&hit.clone()).is_some(),
                    "each segment re-proves"
                );
                if segments.last().is_none_or(|p: &Position| p.y != y) {
                    segments.push(Position::new(x, y));
                }
            }
        }
    }
    assert!(segments.len() > 1, "path really wraps");
    assert_eq!(paths(&mut a).len(), 1, "one occurrence, not one per row");
}

#[test]
fn table_paths_keep_order_through_raw_width_fallback_and_markdown_resume() {
    let body = "| left | right |\n|---|---|\n| src/a.rs | src/b_with_a_very_long_filename.rs |\n";
    let mut a = app(vec![said(body)]);
    for width in [80, 40, 80] {
        let _ = frame(&mut a, width, 30);
        assert_eq!(
            paths(&mut a),
            ["/w/src/a.rs", "/w/src/b_with_a_very_long_filename.rs"]
        );
        a.close_reader();
    }
}

#[test]
fn drag_copies_drawn_text_and_y_copies_source_with_mark_bars_excluded() {
    let source = "**alpha** *beta*";
    let mut a = app(vec![said(source)]);
    mark(&mut a);
    let b = frame(&mut a, 80, 20);
    let p = position(&b, "alpha beta");
    for (kind, x) in [
        (MouseKind::Click, p.x),
        (MouseKind::Drag, p.x + 9),
        (MouseKind::Release, p.x + 9),
    ] {
        a.mouse(Mouse {
            kind,
            column: x,
            row: p.y,
        });
    }
    let _ = frame(&mut a, 80, 20);
    let _ = a.shipped();
    assert_eq!(
        a.clip.as_deref(),
        Some("alpha beta"),
        "drawn copy omits Markdown and chrome"
    );
    a.unselect();
    mark(&mut a);
    a.copy();
    assert_eq!(a.clip.as_deref(), Some(source), "turn copy keeps source");
}

#[test]
fn fenced_code_ground_covers_padding_and_blank_rows() {
    let mut a = app(vec![said("```\nleft\n\nright\n```")]);
    let look = theme::Look::DEFAULT;
    a.looks.insert("api".into(), (Some(look), None));
    let b = frame(&mut a, 80, 25);
    let first = position(&b, "left");
    let last = position(&b, "right");
    let ground = draw::Paint::of(Some(&look)).ground(|p| p.panel).bg.unwrap();
    assert!(last.y > first.y + 1, "code keeps its blank row");
    for y in first.y..=last.y {
        for x in first.x..b.area.width - 2 {
            assert_eq!(b[(x, y)].bg, ground, "code panel at ({x},{y})");
        }
    }
}

#[test]
fn fold_counts_rendered_rows_and_expansion_preserves_mark_and_scroll_bounds() {
    let body = (0..32)
        .map(|n| format!("**row{n:02}**"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut a = app(vec![said(&body)]);
    a.expanded.clear();
    let b = frame(&mut a, 80, 45);
    assert!(
        text(&b).contains("8 more lines"),
        "32 rendered report rows fold at 24"
    );
    assert!(!text(&b).contains("row31"));
    mark(&mut a);
    let fold = a.fold_key(&a.lane.items[0]);
    a.expanded.insert(fold);
    let b = frame(&mut a, 80, 45);
    assert!(text(&b).contains("row31") && !text(&b).contains("**row31**"));
    assert_eq!(a.model.turn(), Some(a.keys()[0]));
    assert!(a.layout.complete && a.layout.max_scroll <= 33);
}

#[test]
fn hostile_markdown_never_puts_controls_or_unsafe_urls_into_drawn_cells() {
    for wrapper in [
        "{}",
        "`{}`",
        "```\n{}\n```",
        "[x]({})",
        "![{}](pic.png)",
        "| a |\n|---|\n| {} |",
    ] {
        let hostile = "x\x1b]8;;https://evil.test\x07\x1b[31m\u{009b}\u{202e}y";
        let body = wrapper.replace("{}", hostile);
        let mut a = app(vec![said(&body)]);
        let b = frame(&mut a, 80, 40);
        assert!(
            b.content
                .iter()
                .all(|c| !c.symbol().chars().any(char::is_control))
        );
        assert!(!text(&b).contains('\u{202e}'), "bidi neutralized");
        assert!(a.layout.links.iter().all(|r| link::is_safe_url(&r.url)));
    }
    let mut a = app(vec![said(
        "[exec](javascript:alert%281%29) [data](data:text/plain,x)",
    )]);
    let _ = frame(&mut a, 80, 25);
    assert!(
        a.layout.links.is_empty(),
        "unsupported schemes are text only"
    );
}

#[test]
fn refusal_preserves_the_exact_old_whole_turn_text_after_partial_rendering() {
    let body = format!("**intro**\n\n```\n{}```\n**tail**", "c\n".repeat(2001));
    let mut a = app(vec![said(&body)]);
    let b = frame(&mut a, 80, 20);
    assert!(
        text(&b).contains("**tail**"),
        "overflow reverts the WHOLE turn, tail not lost"
    );
    mark(&mut a);
    a.copy();
    assert_eq!(a.clip.as_deref(), Some(body.as_str()));
}
