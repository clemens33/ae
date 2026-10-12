//! Where a long turn folds. PURE, over rows a surface already laid out, so the
//! tmux chat and `ae app` fold by one rule and each keeps its own row layout.

/// The rows a folded turn shows.
pub(crate) const SHOWN: usize = 8;

/// The fewest rows a fold must hide: a fold always saves rows, so it costs
/// none. A turn of up to `SHOWN + MIN_HIDDEN - 1` rows shows whole.
pub(crate) const MIN_HIDDEN: usize = 3;

/// Whether a turn of `count` rows has a fold, when the fold line takes `marker`
/// rows to draw: it hides [`MIN_HIDDEN`] rows or more and, the line drawn,
/// still leaves the turn shorter. A pane too narrow for the line shows whole.
pub(crate) fn foldable(count: usize, marker: usize) -> bool {
    count >= SHOWN + MIN_HIDDEN && count > SHOWN + marker
}

/// `rows` as a turn shows them, and how many rows a fold hid: the first
/// [`SHOWN`] when the turn is [`foldable`] with a `marker` row fold line and not
/// `open`, else all of them.
pub(crate) fn folded<T>(mut rows: Vec<T>, open: bool, marker: usize) -> (Vec<T>, usize) {
    if open || !foldable(rows.len(), marker) {
        return (rows, 0);
    }
    let hidden = rows.len() - SHOWN;
    rows.truncate(SHOWN);
    (rows, hidden)
}

/// The words of the fold row of a turn that hid `hidden` rows.
pub(crate) fn line(hidden: usize) -> String {
    format!("... {hidden} more lines")
}

#[cfg(test)]
mod tests {
    use super::{MIN_HIDDEN, SHOWN, foldable, folded, line};

    #[test]
    fn a_turn_folds_from_eleven_rows_and_hides_what_it_names() {
        let rows = |n: usize| (1..=n).collect::<Vec<_>>();
        for n in [0, 1, SHOWN, 9, 10] {
            assert!(!foldable(n, 1), "{n} rows show whole");
            assert_eq!(folded(rows(n), false, 1), (rows(n), 0));
        }
        assert_eq!(SHOWN + MIN_HIDDEN, 11);
        for n in [11, 12, 400] {
            assert!(foldable(n, 1));
            assert_eq!(folded(rows(n), false, 1), (rows(SHOWN), n - SHOWN));
            assert_eq!(folded(rows(n), true, 1), (rows(n), 0), "open shows all");
        }
        assert_eq!(line(3), "... 3 more lines");
    }

    #[test]
    fn a_fold_whose_line_takes_more_rows_than_it_saves_leaves_the_turn_whole() {
        // Folds exactly when the shown rows and the line leave it shorter.
        for (count, marker) in [(11, 2), (11, 3), (12, 3), (12, 4), (13, 5), (14, 5)] {
            let folds = count > SHOWN + marker;
            assert_eq!(foldable(count, marker), folds, "{count}, line {marker}");
            let rows = (0..count).collect::<Vec<_>>();
            assert_eq!(folded(rows, false, marker).1 > 0, folds);
        }
    }
}
