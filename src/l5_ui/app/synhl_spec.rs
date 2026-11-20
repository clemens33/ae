//! Independent acceptance cases for #274. Oracle: issue #274, the slice brief,
//! lead seam rulings, the 4.5:1 text ruling, and the agreed palette mapping.
//! Existing public render and shared live column APIs compile at the base.
//! Only cell styles change. The reader's decoded text and layout stay intact.

use crate::app::draw::{self, Layout};
use crate::app::markdown::{self, Shown};
use crate::app::reader::{self, Body};
use crate::app::select::{Panel, Region};
use crate::theme::{Look, Palette};
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Modifier};

const PALETTES: [Palette; 3] = [Palette::DARCULA, Palette::NEUTRAL, Palette::WARM];

struct Sample {
    ext: &'static str,
    tag: &'static str,
    source: &'static str,
    keyword: &'static str,
    string: &'static str,
    number: &'static str,
    comment: Option<&'static str>,
}

const SAMPLES: [Sample; 11] = [
    Sample {
        ext: "rs",
        tag: "rust",
        source: "let value = \"sample\"; let count = 42; // remark",
        keyword: "let",
        string: "\"sample\"",
        number: "42",
        comment: Some("// remark"),
    },
    Sample {
        ext: "py",
        tag: "python",
        source: "if value == \"sample\": count = 42 # remark",
        keyword: "if",
        string: "\"sample\"",
        number: "42",
        comment: Some("# remark"),
    },
    Sample {
        ext: "sh",
        tag: "bash",
        source: "if [ \"sample\" ]; then count=42; fi # remark",
        keyword: "if",
        string: "\"sample\"",
        number: "42",
        comment: Some("# remark"),
    },
    Sample {
        ext: "fish",
        tag: "fish",
        source: "if test \"sample\"; set count 42; end # remark",
        keyword: "if",
        string: "\"sample\"",
        number: "42",
        comment: Some("# remark"),
    },
    Sample {
        ext: "toml",
        tag: "toml",
        source: "enabled = true\nmessage = \"sample\"\ncount = 42 # remark",
        keyword: "true",
        string: "\"sample\"",
        number: "42",
        comment: Some("# remark"),
    },
    Sample {
        ext: "json",
        tag: "json",
        source: "{\"enabled\": true, \"message\": \"sample\", \"count\": 42}",
        keyword: "true",
        string: "\"sample\"",
        number: "42",
        comment: None,
    },
    Sample {
        ext: "yaml",
        tag: "yaml",
        source: "enabled: true\nmessage: \"sample\"\ncount: 42 # remark",
        keyword: "true",
        string: "\"sample\"",
        number: "42",
        comment: Some("# remark"),
    },
    Sample {
        ext: "yml",
        tag: "yml",
        source: "enabled: true\nmessage: \"sample\"\ncount: 42 # remark",
        keyword: "true",
        string: "\"sample\"",
        number: "42",
        comment: Some("# remark"),
    },
    Sample {
        ext: "js",
        tag: "javascript",
        source: "const value = \"sample\"; const count = 42; // remark",
        keyword: "const",
        string: "\"sample\"",
        number: "42",
        comment: Some("// remark"),
    },
    Sample {
        ext: "ts",
        tag: "typescript",
        source: "const value: string = \"sample\"; const count = 42; // remark",
        keyword: "const",
        string: "\"sample\"",
        number: "42",
        comment: Some("// remark"),
    },
    Sample {
        ext: "RS",
        tag: "Rs extra-info",
        source: "let value = \"sample\"; let count = 42; // remark",
        keyword: "let",
        string: "\"sample\"",
        number: "42",
        comment: Some("// remark"),
    },
];

fn look(palette: Palette) -> Look {
    Look {
        palette,
        ..Look::DEFAULT
    }
}

fn lines(body: &Body) -> &[String] {
    match body {
        Body::Text { lines, .. } => lines,
        Body::Gap(why) => panic!("expected retained text, got {why}"),
        Body::Tree { .. } => panic!("expected retained text, got a tree"),
    }
}

fn layout(area: Rect) -> Layout {
    let mut result = Layout::default();
    result.map.regions.push(Region {
        panel: Panel::Lane,
        area,
    });
    result
}

fn raw(
    path: &str,
    source: &str,
    width: u16,
    top: usize,
    drawn: Option<&Look>,
    live: bool,
) -> Buffer {
    let body = reader::decode(source.as_bytes(), 0);
    let flow = reader::flow(
        lines(&body),
        usize::from(width).saturating_sub(reader::gutter(lines(&body).len())),
    );
    let view = reader::View {
        path,
        line: None,
        body: Some(&body),
        flow: Some(&flow),
        top,
        links: 1,
    };
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, 96));
    if live {
        let mut layout = layout(buffer.area);
        let bounds = draw::paint_reader_column(0, &view, drawn, &mut buffer, &mut layout);
        assert_eq!(bounds.visible, 95);
        assert_eq!(bounds.max_back, flow.rows().saturating_sub(95));
    } else {
        let bounds = draw::draw_reader(&view, drawn.copied(), &mut buffer);
        assert_eq!(bounds.visible, 95);
        assert_eq!(bounds.max_back, flow.rows().saturating_sub(95));
    }
    buffer
}

fn fenced(source: &str, tag: &str, width: u16, drawn: Option<&Look>, live: bool) -> Buffer {
    markdown_buffer(&format!("~~~{tag}\n{source}\n~~~\n"), width, drawn, live)
}

fn markdown_buffer(source: &str, width: u16, drawn: Option<&Look>, live: bool) -> Buffer {
    let source_lines = source.lines().map(str::to_owned).collect::<Vec<_>>();
    let rendered = markdown::render(source, usize::from(width));
    assert!(rendered.refusal().is_none(), "bounded fixture must render");
    let view = markdown::View {
        path: "/fixture/sample.md",
        line: None,
        lines: &source_lines,
        shown: Shown::Rendered(&rendered),
        refusal: None,
        top: 0,
        links: 1,
    };
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, 96));
    if live {
        let mut layout = layout(buffer.area);
        let _ = draw::paint_markdown_column(0, &view, drawn, &mut buffer, &mut layout);
    } else {
        let _ = draw::draw_markdown(&view, drawn.copied(), &mut buffer);
    }
    buffer
}

fn row(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

fn text(buffer: &Buffer) -> Vec<String> {
    (1..buffer.area.height).map(|y| row(buffer, y)).collect()
}

fn equal_body(left: &Buffer, right: &Buffer) {
    assert_eq!(left.area, right.area);
    for y in 1..left.area.height {
        for x in 0..left.area.width {
            assert_eq!(left[(x, y)], right[(x, y)], "body cell ({x},{y})");
        }
    }
}

fn color(hex: &str) -> Color {
    let [r, g, b] = crate::theme::rgb(hex);
    Color::Rgb(r, g, b)
}

// Independent WCAG sRGB reference, applied to the actual rendered cell pair.
fn luminance(color: Color) -> f64 {
    let Color::Rgb(r, g, b) = color else {
        panic!("drawn syntax must use palette RGB, got {color:?}");
    };
    let linear = |byte: u8| {
        let channel = f64::from(byte) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

fn token(buffer: &Buffer, needle: &str, fg: &str, bg: &str, modifier: Modifier) {
    let (x, y) = (1..buffer.area.height)
        .find_map(|y| {
            let line = row(buffer, y);
            line.find(needle).map(|byte| {
                let x = u16::try_from(line[..byte].chars().count()).expect("fixture width");
                (x, y)
            })
        })
        .unwrap_or_else(|| panic!("{needle:?} missing from {:?}", text(buffer)));
    for step in 0..needle.chars().count() {
        let x = x + u16::try_from(step).expect("fixture token width");
        let cell = &buffer[(x, y)];
        assert_eq!(cell.fg, color(fg), "{needle:?}, ({x},{y}) foreground");
        assert_eq!(cell.bg, color(bg), "{needle:?}, ({x},{y}) ground");
        assert_eq!(cell.modifier, modifier, "{needle:?}, ({x},{y}) modifier");
        let (a, b) = (luminance(cell.fg), luminance(cell.bg));
        let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
        assert!(ratio >= 4.5, "{needle:?} has contrast {ratio}");
    }
}

fn sample_styles(buffer: &Buffer, sample: &Sample, palette: Palette, fence: bool) {
    let ground = if fence { palette.panel } else { palette.ink };
    token(
        buffer,
        sample.keyword,
        palette.waiting_agent,
        ground,
        Modifier::BOLD,
    );
    token(
        buffer,
        sample.string,
        if fence {
            palette.title
        } else {
            palette.working
        },
        ground,
        Modifier::empty(),
    );
    token(
        buffer,
        sample.number,
        palette.title,
        ground,
        if fence {
            Modifier::BOLD
        } else {
            Modifier::empty()
        },
    );
    if let Some(comment) = sample.comment {
        token(buffer, comment, palette.text, ground, Modifier::ITALIC);
    }
}

#[test]
fn synhl_each_language_styles_actual_file_cells_on_public_and_live_paths() {
    for sample in &SAMPLES {
        for palette in PALETTES {
            for live in [false, true] {
                let path = format!("/fixture/sample.{}", sample.ext);
                let buffer = raw(&path, sample.source, 100, 0, Some(&look(palette)), live);
                sample_styles(&buffer, sample, palette, false);
            }
        }
    }
}

#[test]
fn synhl_tagged_fences_use_the_same_token_classes_on_public_and_live_paths() {
    for sample in &SAMPLES {
        for palette in PALETTES {
            for live in [false, true] {
                let buffer = fenced(sample.source, sample.tag, 100, Some(&look(palette)), live);
                sample_styles(&buffer, sample, palette, true);
            }
        }
    }
}

#[test]
fn synhl_highlighting_preserves_every_drawn_symbol_and_viewport() {
    let sources = [
        "let value = \"é界 sample\"; let count = 42; // remark\nEND",
        "\tlet repeated = \"same same same same same same same same same\"; // remark\nEND",
        "let s = \"unterminated\n/* unfinished\nEND",
        "let s = \"\u{1b}]52;c;PAYLOAD\u{7}\u{9b}31m\";\nEND",
    ];
    for source in sources {
        for width in [12, 24, 80] {
            for live in [false, true] {
                let plain = raw(
                    "/fixture/sample.unknown",
                    source,
                    width,
                    0,
                    Some(&Look::DEFAULT),
                    live,
                );
                let styled = raw(
                    "/fixture/sample.rs",
                    source,
                    width,
                    0,
                    Some(&Look::DEFAULT),
                    live,
                );
                assert_eq!(text(&styled), text(&plain), "{source:?}, width {width}");
                for tag in [
                    "rust",
                    "python",
                    "fish",
                    "toml",
                    "json",
                    "yaml",
                    "typescript",
                ] {
                    let plain = fenced(source, "", width, Some(&Look::DEFAULT), live);
                    let styled = fenced(source, tag, width, Some(&Look::DEFAULT), live);
                    assert_eq!(text(&styled), text(&plain), "tag {tag}, width {width}");
                }
            }
        }
    }
}

#[test]
fn synhl_disabled_look_keeps_exact_legacy_body_cells() {
    for drawn in [
        None,
        Some(Look {
            drawn: false,
            ..Look::DEFAULT
        }),
    ] {
        for sample in &SAMPLES {
            for live in [false, true] {
                let plain = raw(
                    "/fixture/sample.unknown",
                    sample.source,
                    80,
                    0,
                    drawn.as_ref(),
                    live,
                );
                let styled = raw(
                    &format!("/fixture/sample.{}", sample.ext),
                    sample.source,
                    80,
                    0,
                    drawn.as_ref(),
                    live,
                );
                equal_body(&plain, &styled);
                let plain = fenced(sample.source, "", 80, drawn.as_ref(), live);
                let styled = fenced(sample.source, sample.tag, 80, drawn.as_ref(), live);
                equal_body(&plain, &styled);
            }
        }
    }
}

#[test]
fn synhl_unknown_paths_and_unknown_untagged_or_indented_fences_stay_plain() {
    for live in [false, true] {
        let source = "let value = \"sample\"; let count = 42; // remark";
        for palette in PALETTES {
            let drawn = Some(look(palette));
            let plain = raw(
                "/fixture/sample.unknown",
                source,
                80,
                0,
                drawn.as_ref(),
                live,
            );
            for path in [
                "/fixture/sample",
                "/fixture/sample.txt",
                "/fixture/sample.rs.unknown",
            ] {
                equal_body(&plain, &raw(path, source, 80, 0, drawn.as_ref(), live));
            }
            token(&plain, source, palette.text, palette.ink, Modifier::empty());
            let untagged = fenced(source, "", 80, drawn.as_ref(), live);
            token(
                &untagged,
                source,
                palette.text,
                palette.panel,
                Modifier::empty(),
            );
            equal_body(
                &untagged,
                &fenced(source, "unknown", 80, drawn.as_ref(), live),
            );
            equal_body(
                &untagged,
                &markdown_buffer(&format!("    {source}\n"), 80, drawn.as_ref(), live),
            );
        }
    }
}

#[test]
fn synhl_wrap_continuations_keep_string_style_at_nonzero_viewport() {
    let source = format!(
        "let s = \"{}\";\n{}",
        "abcdefgh".repeat(300),
        "END\n".repeat(40)
    );
    for live in [false, true] {
        let plain = raw(
            "/fixture/sample.txt",
            &source,
            32,
            8,
            Some(&Look::DEFAULT),
            live,
        );
        let styled = raw(
            "/fixture/sample.rs",
            &source,
            32,
            8,
            Some(&Look::DEFAULT),
            live,
        );
        assert_eq!(text(&styled), text(&plain));
        assert!(!row(&styled, 1).contains("let s"));
        let palette = Palette::DARCULA;
        token(
            &styled,
            "abcdefgh",
            palette.working,
            palette.ink,
            Modifier::empty(),
        );
        let decoded = reader::decode(source.as_bytes(), 0);
        assert_eq!(lines(&decoded)[0].chars().count(), reader::LINE_CHARS);
        assert!(lines(&decoded)[0].ends_with('…'));
        assert!(text(&styled).iter().any(|line| line.contains('…')));
        assert!(text(&styled).iter().any(|line| line.contains("END")));
    }
}

#[test]
fn synhl_hostile_retained_text_is_inert_and_fully_visible() {
    let source = "let s = \"\u{1b}]52;c;PAYLOAD\u{7}\u{9b}31m\"; /* unfinished\nEND";
    for live in [false, true] {
        let raw = raw(
            "/fixture/sample.rs",
            source,
            80,
            0,
            Some(&Look::DEFAULT),
            live,
        );
        let md = fenced(source, "rust", 80, Some(&Look::DEFAULT), live);
        for buffer in [&raw, &md] {
            let drawn = text(buffer).join("\n");
            assert!(drawn.contains("�]52;c;PAYLOAD��31m"), "{drawn:?}");
            assert!(drawn.contains("/* unfinished"));
            assert!(drawn.contains("END"));
            assert!(
                !text(buffer)
                    .iter()
                    .any(|line| line.chars().any(char::is_control)),
                "{drawn:?}"
            );
        }
    }
}

#[test]
fn synhl_fence_state_does_not_colour_following_unknown_fences_or_prose() {
    let source = "~~~rust\n/* unfinished\n~~~\n\nplain prose\n\n~~~unknown\nlet untouched = \"literal\";\n~~~\n\n~~~\nlet untagged = \"literal\";\n~~~\n\n~~~python\nif value == \"sample\": count = 42 # remark\n~~~\n";
    for live in [false, true] {
        for palette in PALETTES {
            let buffer = markdown_buffer(source, 100, Some(&look(palette)), live);
            token(
                &buffer,
                "plain prose",
                palette.text,
                palette.ink,
                Modifier::empty(),
            );
            token(
                &buffer,
                "let untouched",
                palette.text,
                palette.panel,
                Modifier::empty(),
            );
            token(
                &buffer,
                "let untagged",
                palette.text,
                palette.panel,
                Modifier::empty(),
            );
            sample_styles(&buffer, &SAMPLES[1], palette, true);
        }
    }
}

#[test]
fn synhl_oversized_fence_line_preserves_the_tail_beyond_the_token_budget() {
    let source = format!("let s = \"{}\"; TAIL_END", "a".repeat(2400));
    for live in [false, true] {
        let plain = fenced(&source, "", 80, Some(&Look::DEFAULT), live);
        let styled = fenced(&source, "rust", 80, Some(&Look::DEFAULT), live);
        assert_eq!(text(&styled), text(&plain));
        assert!(text(&styled).iter().any(|line| line.contains("TAIL_END")));
        assert!(text(&styled).iter().any(|line| line.contains("\";")));
    }
}
