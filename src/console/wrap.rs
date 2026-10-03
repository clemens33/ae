//! A row of the chat laid out in the cells of a pane.
//!
//! The measure is `event_text::cell`, a safe upper bound: a row it fits never
//! overflows, accented Latin may wrap early. Text keeps its Unicode.

use crate::event_text::cell;

/// The width in cells of the bar every row starts with.
const BAR: usize = 1;

/// The spaces after the bar and the piece of text, for each visual row of
/// `text` in a pane `width` cells wide. Every `\n` starts a new logical line;
/// the first line's first row sits `first` spaces behind the bar, every other
/// row two. A line keeps its own leading spaces on every row, the whole indent
/// capped so two cells of text remain; it shrinks first, then to nothing.
/// Tabs become spaces to the next stop of eight, counted as the unwrapped row
/// would draw them. A row breaks after the last space that fits, else between
/// two scalars, and always takes one scalar, so no text is dropped even in a
/// pane too narrow for it.
pub(super) fn wrap(text: &str, width: usize, first: usize) -> Vec<(usize, String)> {
    let cap = width.saturating_sub(BAR + 2);
    let mut rows = Vec::new();
    for (at, line) in text.split('\n').enumerate() {
        let lead = if at == 0 { first } else { 2 };
        let flat = expand_tabs(line, BAR + lead);
        let content = flat.trim_start_matches(' ');
        let hang = flat.len() - content.len();
        let mut gap = (lead + hang).min(cap);
        let mut rest = content;
        loop {
            let (piece, tail) = split_row(rest, width.saturating_sub(BAR + gap));
            rows.push((gap, piece.to_owned()));
            if tail.is_empty() {
                break;
            }
            (rest, gap) = (tail, (2 + hang).min(cap));
        }
    }
    rows
}

/// `line` with each tab spent as spaces, its first scalar `column` cells in.
fn expand_tabs(line: &str, column: usize) -> String {
    let mut out = String::with_capacity(line.len());
    let mut column = column;
    for ch in line.chars() {
        if ch == '\t' {
            let spaces = 8 - column % 8;
            out.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            out.push(ch);
            column += cell(ch);
        }
    }
    out
}

/// The first row of `text` in `room` cells and what is left, which never
/// starts with a space. The row is at least one scalar.
fn split_row(text: &str, room: usize) -> (&str, &str) {
    let (mut used, mut space) = (0, None);
    for (at, ch) in text.char_indices() {
        if used + cell(ch) > room && at > 0 {
            let cut = if ch == ' ' { Some(at) } else { space };
            return match cut {
                Some(cut) => (
                    text[..cut].trim_end_matches(' '),
                    text[cut..].trim_start_matches(' '),
                ),
                None => text.split_at(at),
            };
        }
        if ch == ' ' && !text[..at].ends_with(' ') {
            space = Some(at);
        }
        used += cell(ch);
    }
    (text, "")
}

#[cfg(test)]
mod tests {
    use super::wrap;
    use crate::event_text::cell;

    fn row(gap: usize, text: &str) -> (usize, String) {
        (gap, text.to_owned())
    }

    /// The cells a row takes: the bar, its gap, its piece.
    fn cells((gap, piece): &(usize, String)) -> usize {
        1 + gap + piece.chars().map(cell).sum::<usize>()
    }

    fn letters(text: &str) -> String {
        text.chars().filter(|ch| !ch.is_whitespace()).collect()
    }

    #[test]
    fn from_three_cells_up_no_row_overflows_and_no_text_is_lost() {
        let texts = [
            "alpha beta gamma delta epsilon",
            "https://example.invalid/abcdefghijklmnopqrstuvwxyz0123456789",
            "中中中中中 a中bé中c",
            "\t中\tend\tof tabs",
            "    indented code under a hanging indent that is far too wide",
            "one\ntwo words\n\nafter a blank",
            "",
        ];
        for text in texts {
            for width in 3..=30 {
                let rows = wrap(text, width, 2);
                for row in &rows {
                    assert!(cells(row) <= width, "{width}: {text:?}: {row:?}");
                    assert!(!row.1.contains(['\t', '\n']), "{row:?}");
                }
                let kept: String = rows.iter().map(|(_, piece)| letters(piece)).collect();
                assert_eq!(kept, letters(text), "{width}: {text:?}");
            }
        }
    }

    #[test]
    fn a_row_breaks_after_the_last_space_that_fits_else_between_scalars() {
        let alpha = [row(2, "alpha"), row(2, "beta"), row(2, "gamma")];
        assert_eq!(wrap("alpha beta gamma", 12, 2), alpha);
        assert_eq!(wrap("alpha beta", 13, 2), [row(2, "alpha beta")]);
        assert_eq!(wrap("a   b", 5, 2), [row(2, "a"), row(2, "b")]);
        assert_eq!(wrap("abcdefghij", 8, 2), [row(2, "abcde"), row(2, "fghij")]);
        assert_eq!(
            wrap("abc defghi", 8, 2),
            [row(2, "abc"), row(2, "defgh"), row(2, "i")]
        );
    }

    #[test]
    fn a_tab_is_spent_as_spaces_to_the_next_stop_counted_from_the_first_row() {
        assert_eq!(wrap("a\tb", 40, 2), [row(2, "a    b")]);
        assert_eq!(wrap("\tb", 40, 2), [row(7, "b")]);
        assert_eq!(wrap("a\tb", 40, 1), [row(1, "a     b")]);
    }

    #[test]
    fn a_hanging_indent_keeps_its_width_until_two_cells_of_text_would_go() {
        let code = "    code words here";
        assert_eq!(
            wrap(code, 14, 2),
            [row(6, "code"), row(6, "words"), row(6, "here")]
        );
        let deep = "          x y";
        assert_eq!(wrap(deep, 6, 2), [row(3, "x"), row(3, "y")]);
        assert_eq!(wrap(deep, 3, 2), [row(0, "x"), row(0, "y")]);
    }

    #[test]
    fn every_newline_starts_a_row_of_its_own_and_a_blank_line_stays_one() {
        assert_eq!(wrap("a\nb", 20, 1), [row(1, "a"), row(2, "b")]);
        assert_eq!(wrap("", 10, 2), [row(2, "")]);
        assert_eq!(
            wrap("a\n\nb", 10, 2),
            [row(2, "a"), row(2, ""), row(2, "b")]
        );
    }

    #[test]
    fn the_first_row_margin_and_every_gap_give_way_in_a_narrow_pane() {
        assert_eq!(wrap("ab cd", 3, 1), [row(0, "ab"), row(0, "cd")]);
        assert_eq!(wrap("ab cd", 4, 1), [row(1, "ab"), row(1, "cd")]);
        assert_eq!(wrap("ab cd", 4, 2), [row(1, "ab"), row(1, "cd")]);
        assert_eq!(wrap("ab cd", 5, 1), [row(1, "ab"), row(2, "cd")]);
    }

    #[test]
    fn below_three_cells_a_row_takes_one_scalar_and_may_overflow() {
        assert_eq!(wrap("中ab", 3, 2), [row(0, "中"), row(0, "ab")]);
        assert_eq!(wrap("中ab", 2, 2), [row(0, "中"), row(0, "a"), row(0, "b")]);
        assert_eq!(wrap("中a", 1, 2), [row(0, "中"), row(0, "a")]);
        assert_eq!(wrap("", 0, 2), [row(0, "")]);
    }
}
