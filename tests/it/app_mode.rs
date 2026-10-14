//! Writing carries its keys hint on the footer; a held composer is
//! distinguished by its fixed address row, never by arbitrary chat text.

/// The footer carries the writing hint while writing; a held address is two
/// rows above it, under the composer rule. Blank, browse and settings reject.
pub fn writing(screen: &str) -> bool {
    let mut rows = screen.lines().rev();
    let footer = rows.next().unwrap_or_default();
    let address = rows.nth(1).unwrap_or_default();
    let rule = rows.next().unwrap_or_default();
    let held = address.contains("not writing:") && rule.trim_end().ends_with("────");
    !held && footer.contains("Enter sends") && footer.trim_end().ends_with(['⚙', '*'])
}

/// A frame whose final row is `keys`.
fn frame(keys: &str) -> String {
    format!("────\nto api\n\n{keys}")
}

#[test]
fn the_hint_footer_is_writing_with_or_without_a_note() {
    for keys in [
        "   Enter sends · Alt+Enter ^J newline · Esc browses      ae 2026.10.45 ⚙",
        "   Enter sends · Alt+Enter ^J newline · Esc browses      ae 2026.10.45 *",
    ] {
        assert!(writing(&frame(keys)), "{keys}");
        let noted = format!("not writing: chat text\n────\nto api\nx\nq again to quit · {keys}");
        assert!(writing(&noted), "a note on the note row: {keys}");
        let draft = format!("────\nto api\nnot writing: draft text\nlast draft row\n{keys}");
        assert!(writing(&draft), "draft prose is not the held address");
    }
}

#[test]
fn the_other_modes_and_a_blank_final_row_are_not_writing() {
    for keys in [
        "     ? keys      ae 2026.10.45 ⚙",
        "  settings   Esc close      ae 2026.10.45 ⚙",
        "",
    ] {
        assert!(!writing(&frame(keys)), "{keys:?}");
    }
    for gear in ["⚙", "*"] {
        let held = format!("────\nnot writing: refused · Esc browses\n\n  ae 2026.10.45 {gear}");
        assert!(!writing(&held), "a blank-left held footer is not writable");
    }
}

#[test]
fn a_version_above_the_final_row_is_chat_text_not_the_mode() {
    let screen = "  ae chat text\nfine\n     ? keys\n";
    assert!(!writing(screen));
    assert!(!writing(""));
    assert!(!writing("ae chat text"));
}
