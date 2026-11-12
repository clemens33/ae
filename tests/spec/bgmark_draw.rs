// Include inside app/draw.rs's existing cfg(test) mod tests.
#[test]
fn bgmark_keys_legend_teaches_all_eight_marks_in_both_vocabularies() {
    for icons in [true, false] {
        let rows = super::marks_legend(Paint::of(None), icons);
        for (unicode, ascii, word) in [
            ("✖", "x", "dead"),
            ("⚠", "!", "needs you"),
            ("◌", "?", "unknown"),
            ("●", "*", "working"),
            ("◉", "&", "background"),
            ("◔", "~", "waiting on an agent"),
            ("✓", "+", "finished"),
            ("·", "-", "no current status"),
        ] {
            let glyph = if icons { unicode } else { ascii };
            assert!(
                rows.iter()
                    .any(|(text, _)| text.trim_start().starts_with(glyph)
                        && text.to_ascii_lowercase().contains(word)),
                "{glyph} {word}: {rows:?}"
            );
        }
        assert_eq!(rows.len(), 9, "one heading and eight marks");
    }
}

#[test]
fn bgmark_narrow_reason_still_fits_fourteen_cells_with_eight_marks() {
    let mut shot = Shot::new();
    shot.overview.open = vec![Open {
        seat: "lead".to_owned(),
        text: "approve the plan".repeat(8),
        age_secs: Some(60),
    }];
    for icons in [true, false] {
        let screen = shot.screen(READ_ONLY);
        let ctx = super::Ctx {
            screen: &screen,
            paint: Paint::of(screen.look.as_ref()),
            icons,
            wait: super::Wait::default(),
        };
        assert_eq!(super::reason_words(&ctx, 13), None);
        let words = super::reason_words(&ctx, 14).expect("twelve cells beside the mark");
        assert!(super::drawn_cols(&words) <= 14, "{words}");
    }
}
