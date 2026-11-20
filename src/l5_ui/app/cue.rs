//! Unread cues: which sessions answered since this app last showed them.

use std::collections::{BTreeMap, BTreeSet};

/// When each session was last shown selected (or first met). PURE and memory
/// only: the answer moment is the one the sidebar orders by (`answer_epoch`).
#[derive(Debug, Default)]
pub(super) struct Cue {
    seen: BTreeMap<String, i64>,
}

impl Cue {
    /// One frame: `(name, newest answer epoch, skip)` per row (`skip` = the
    /// selected or home row). A skipped row is seen `now` and never marks;
    /// another marks when its answer is STRICTLY newer than its last sighting
    /// (whole seconds). A first sight baselines at `now`, so a restart shows
    /// no marks; a row absent from this frame is forgotten and baselines again.
    pub(super) fn frame<'a>(
        &mut self,
        rows: impl Iterator<Item = (&'a str, Option<i64>, bool)>,
        now: i64,
    ) -> BTreeSet<String> {
        let mut seen = BTreeMap::new();
        let mut marked = BTreeSet::new();
        for (name, answered, skip) in rows {
            let last = if skip {
                now
            } else {
                self.seen.get(name).copied().unwrap_or(now)
            };
            if !skip && answered.is_some_and(|at| at > last) {
                marked.insert(name.to_owned());
            }
            seen.insert(name.to_owned(), last);
        }
        self.seen = seen;
        marked
    }
}

#[cfg(test)]
mod tests {
    use super::Cue;
    use std::collections::BTreeSet;
    const T: i64 = 1_791_456_000;
    /// First sight baselines (a restart shows no marks); only a strictly newer
    /// answer marks; skipped (selected/home) rows never mark, and seeing one
    /// advances its baseline.
    #[test]
    fn baseline_then_strictly_newer_marks_and_skip_never_marks() {
        let mut cue = Cue::default();
        let mut rows = |skip: bool, beta: Option<i64>, now: i64| {
            let rows = [("api", Some(T + 10), true), ("beta", beta, skip)];
            cue.frame(rows.into_iter(), now)
        };
        let beta = BTreeSet::from(["beta".to_owned()]);
        let m = rows(false, Some(T - 100), T);
        assert!(m.is_empty(), "first sight baselines");
        let m = rows(false, Some(T + 10), T + 5);
        assert_eq!(m, beta, "newer marks, home never");
        let m = rows(true, Some(T + 6), T + 6);
        assert!(m.is_empty(), "selected never marks");
        let m = rows(false, Some(T + 6), T + 30);
        assert!(m.is_empty(), "not newer than last selected");
        let m = rows(false, Some(T + 31), T + 30);
        assert_eq!(m, beta, "strictly newer marks");
    }
    /// A row absent for a frame is pruned: when it returns, its old answer
    /// rebaselines instead of marking.
    #[test]
    fn absence_prunes_so_a_returning_answer_rebaselines() {
        let mut cue = Cue::default();
        let beta = BTreeSet::from(["beta".to_owned()]);
        let m = cue.frame([("beta", Some(T), false)].into_iter(), T);
        assert!(m.is_empty(), "baselines");
        let m = cue.frame([("beta", Some(T + 10), false)].into_iter(), T + 5);
        assert_eq!(m, beta, "newer marks");
        let m = cue.frame([("api", None, true)].into_iter(), T + 6);
        assert!(m.is_empty(), "skipped");
        let m = cue.frame([("beta", Some(T + 10), false)].into_iter(), T + 30);
        assert!(m.is_empty(), "pruned: rebaselined");
    }
}
