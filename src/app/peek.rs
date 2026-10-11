//! The peek panel's pure half: the bounds a capture is kept to, the body a
//! capture or a refusal becomes, the viewport math and the status line.
//!
//! PURE — the capture arrives as an argument; nothing here reads or paints.
//! Pane text is hostile: it is kept as captured (a trailing `\r` aside) and
//! neutralised where it is painted, by the one `board::terminal_text`.

use std::time::Duration;

use crate::console::open::Refusal;

/// How often the open panel's pane is captured again.
pub const PEEK_EVERY: Duration = Duration::from_secs(2);
/// The lines of history captured above the visible screen.
pub const HISTORY: u32 = 200;
/// The most captured lines kept: the newest.
pub const KEPT_LINES: usize = 400;
/// The most scalars one kept line holds, its ellipsis included.
pub const LINE_CHARS: usize = 2000;
/// The age, in whole seconds, from which a capture is marked stale.
pub const STALE_SECS: u64 = 5;

/// What the panel shows of its pane: the captured lines, or the one line that
/// says why there are none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Lines(Vec<String>),
    Gap(String),
}

/// What the panel draws: the seat it peeks, its newest body, how old that
/// capture is and how far back the viewport is scrolled.
#[derive(Debug, Clone, Copy)]
pub struct View<'a> {
    pub session: &'a str,
    pub seat: &'a str,
    /// `None` until the first answer lands.
    pub body: Option<&'a Body>,
    pub age_secs: u64,
    /// Rows scrolled back from the newest.
    pub back: usize,
    /// A newer capture is held while the viewport is scrolled back.
    pub newer_waiting: bool,
}

/// The lines a capture is kept as: split on `\n`, one trailing `\r` run
/// trimmed from each (a `\r` inside a line stays for the painter), blank
/// lines at the end dropped, the newest [`KEPT_LINES`] kept, each capped.
#[must_use]
pub fn bound(text: &str) -> Vec<String> {
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    let skip = lines.len().saturating_sub(KEPT_LINES);
    lines[skip..].iter().map(|line| capped(line)).collect()
}

/// `line`, or its first scalars and an ellipsis within [`LINE_CHARS`].
fn capped(line: &str) -> String {
    if line.chars().count() <= LINE_CHARS {
        return line.to_owned();
    }
    let mut kept: String = line.chars().take(LINE_CHARS - 1).collect();
    kept.push('…');
    kept
}

/// The body a refused proof shows, by name.
#[must_use]
pub fn refused(seat: &str, why: &Refusal) -> Body {
    Body::Gap(format!("refused: peek {seat}: {}", why.why()))
}

/// The body one capture became: `None` or a failed run is a named gap, a
/// screen of nothing is another.
#[must_use]
pub fn captured(read: Option<(bool, &str)>) -> Body {
    let Some((true, text)) = read else {
        return Body::Gap("capture failed: tmux did not return the pane's screen".to_owned());
    };
    let lines = bound(text);
    if lines.is_empty() {
        return Body::Gap("(the pane shows nothing)".to_owned());
    }
    Body::Lines(lines)
}

/// Where a viewport of `visible` rows over `rows` starts when scrolled `back`
/// rows from the newest, and how far back it can go at most.
#[must_use]
pub fn window(rows: usize, visible: usize, back: usize) -> (usize, usize) {
    let max_back = rows.saturating_sub(visible);
    (max_back - back.min(max_back), max_back)
}

/// The panel's status line: how old the capture is and how much of it there
/// is, or why nothing is shown. A scrolled-back view says it is held.
#[must_use]
pub fn status(view: &View<'_>) -> String {
    let rows = match view.body {
        None => return "loading".to_owned(),
        Some(Body::Gap(_)) => return "not captured".to_owned(),
        Some(Body::Lines(lines)) => lines.len(),
    };
    let stale = if view.age_secs >= STALE_SECS {
        "STALE · "
    } else {
        ""
    };
    if view.back > 0 {
        let newer = if view.newer_waiting {
            " · newer waiting"
        } else {
            ""
        };
        return format!("{stale}paused · {} back{newer}", view.back);
    }
    let unit = if rows == 1 { "row" } else { "rows" };
    format!("{stale}captured {}s ago · {rows} {unit}", view.age_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(body: Option<&Body>, age_secs: u64, back: usize, newer_waiting: bool) -> View<'_> {
        View {
            session: "s",
            seat: "lead",
            body,
            age_secs,
            back,
            newer_waiting,
        }
    }

    fn lines(count: usize) -> Body {
        Body::Lines((0..count).map(|n| n.to_string()).collect())
    }

    #[test]
    fn a_line_is_capped_at_its_scalar_budget_with_the_ellipsis_inside_it() {
        let at = "é".repeat(LINE_CHARS);
        assert_eq!(bound(&at), vec![at.clone()]);
        let over = "é".repeat(LINE_CHARS + 1);
        let kept = &bound(&over)[0];
        assert_eq!(kept.chars().count(), LINE_CHARS);
        assert!(kept.ends_with('…'));
        assert!(kept.starts_with(&"é".repeat(LINE_CHARS - 1)));
    }

    #[test]
    fn only_the_newest_lines_are_kept_and_the_edge_keeps_every_one() {
        let text = |n: usize| {
            (0..n)
                .map(|i| format!("l{i}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(bound(&text(KEPT_LINES)).len(), KEPT_LINES);
        let over = bound(&text(KEPT_LINES + 1));
        assert_eq!(over.len(), KEPT_LINES);
        assert_eq!(over[0], "l1");
        assert_eq!(over[KEPT_LINES - 1], format!("l{KEPT_LINES}"));
    }

    #[test]
    fn trailing_blank_lines_go_and_a_middle_blank_stays() {
        assert_eq!(bound("a\n\nb\n   \n\t\n\n"), vec!["a", "", "b"]);
        assert!(bound("  \n\n").is_empty());
        assert!(bound("").is_empty());
    }

    #[test]
    fn a_trailing_carriage_return_is_trimmed_and_an_inner_one_is_left_for_the_painter() {
        assert_eq!(bound("a\r\nb\r\r\nc\rd\r\n"), vec!["a", "b", "c\rd"]);
    }

    #[test]
    fn a_capture_is_lines_or_a_named_gap() {
        assert_eq!(captured(Some((true, "x\ny\n"))), lines_of(&["x", "y"]));
        let failed = "capture failed: tmux did not return the pane's screen";
        assert_eq!(captured(None), Body::Gap(failed.to_owned()));
        assert_eq!(captured(Some((false, "x"))), Body::Gap(failed.to_owned()));
        let nothing = Body::Gap("(the pane shows nothing)".to_owned());
        assert_eq!(captured(Some((true, "\n \n"))), nothing);
        assert_eq!(captured(Some((true, ""))), nothing);
    }

    fn lines_of(rows: &[&str]) -> Body {
        Body::Lines(rows.iter().map(|row| (*row).to_owned()).collect())
    }

    #[test]
    fn a_refusal_is_named_by_seat_and_reason() {
        assert_eq!(
            refused("lead", &Refusal::Dead),
            Body::Gap("refused: peek lead: its pane is dead".to_owned())
        );
    }

    #[test]
    fn the_window_pins_the_newest_rows_and_clamps_the_scroll() {
        assert_eq!(window(10, 4, 0), (6, 6));
        assert_eq!(window(10, 4, 2), (4, 6));
        assert_eq!(window(10, 4, 6), (0, 6));
        assert_eq!(window(10, 4, 99), (0, 6));
        assert_eq!(window(3, 4, 0), (0, 0));
        assert_eq!(window(4, 4, 1), (0, 0));
        assert_eq!(window(0, 0, 0), (0, 0));
    }

    #[test]
    fn the_status_says_loading_then_the_age_and_the_rows() {
        assert_eq!(status(&view(None, 0, 0, false)), "loading");
        let gap = Body::Gap("x".to_owned());
        assert_eq!(status(&view(Some(&gap), 9, 3, true)), "not captured");
        let some = lines(3);
        assert_eq!(
            status(&view(Some(&some), 1, 0, false)),
            "captured 1s ago · 3 rows"
        );
        let one = lines(1);
        assert_eq!(
            status(&view(Some(&one), 0, 0, false)),
            "captured 0s ago · 1 row"
        );
    }

    #[test]
    fn the_stale_mark_begins_at_the_fifth_second_and_not_the_fourth() {
        let some = lines(2);
        assert_eq!(
            status(&view(Some(&some), STALE_SECS - 1, 0, false)),
            "captured 4s ago · 2 rows"
        );
        assert_eq!(
            status(&view(Some(&some), STALE_SECS, 0, false)),
            "STALE · captured 5s ago · 2 rows"
        );
    }

    #[test]
    fn a_scrolled_back_view_says_paused_and_whether_newer_is_waiting() {
        let some = lines(2);
        assert_eq!(status(&view(Some(&some), 1, 7, false)), "paused · 7 back");
        assert_eq!(
            status(&view(Some(&some), 1, 7, true)),
            "paused · 7 back · newer waiting"
        );
        assert_eq!(
            status(&view(Some(&some), 5, 1, true)),
            "STALE · paused · 1 back · newer waiting"
        );
    }
}
