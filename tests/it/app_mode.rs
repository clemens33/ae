//! Which mode an `ae app` capture shows: its last physical row begins with the
//! mode word, and nothing else on the screen can say it.

/// Whether `screen` shows the app writing: the final physical row of the
/// capture begins, past its margin, with the mode word `write`. A browse,
/// held or settings row begins with its own word, a blank row says nothing,
/// and a `write` further up the screen is chat text, not the mode.
pub fn writing(screen: &str) -> bool {
    screen
        .lines()
        .next_back()
        .is_some_and(|row| row.trim_start().starts_with("write"))
}

/// A frame whose final row is `keys`.
fn frame(keys: &str) -> String {
    format!("to api › lead\n\nEnter writes\n\n{keys}")
}

#[test]
fn the_write_word_on_the_final_row_is_writing_with_or_without_a_note() {
    for keys in [
        "  write      ae 2026.10.45 ⚙",
        "  write      ae 2026.10.45 *",
    ] {
        assert!(writing(&frame(keys)), "{keys}");
        let noted = format!("to api › lead\nx\nq again to quit\n\n{keys}");
        assert!(writing(&noted), "a note on the note row: {keys}");
    }
}

#[test]
fn the_other_modes_and_a_blank_final_row_are_not_writing() {
    for keys in [
        "  browse   ? keys      ae 2026.10.45 ⚙",
        "  held      ae 2026.10.45 ⚙",
        "  settings   Esc close      ae 2026.10.45 ⚙",
        "",
    ] {
        assert!(!writing(&frame(keys)), "{keys:?}");
    }
}

#[test]
fn a_write_word_above_the_final_row_is_chat_text_not_the_mode() {
    let screen = "  write notes\nfine\n  browse   ? keys\n";
    assert!(!writing(screen));
    assert!(!writing(""));
}
