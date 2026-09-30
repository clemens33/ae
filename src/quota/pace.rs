//! Pace: how fast a quota window is filling, and whether it empties first.
//!
//! PURE, like the parsers beside it: rows and points in, one verdict out. It is
//! a consumer of [`Derived::judge`], the one derivation of a judged percentage,
//! never a second one; it creates no level and reads no `Classified`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::{
    Derived, Effective, FUTURE_SKEW_SECS, Group, Policy, Reading, Row, Status, freshness,
    row_percent, span_label,
};

/// Points kept per key: with the spacing below they reach one lookback back.
const RETAINED: u8 = 32;
const MIN_SPACING_SECS: i64 = 5 * 60;
/// Same window instance: `resets_at` within this of the newest row's.
const INSTANCE_SECS: u64 = 120;
/// A drop this small is vendor rounding, not a window restart.
const ROUNDING_PP: f64 = 0.5;
/// Less judged movement than this is quantization, so it prints a bound.
const MIN_DELTA_PP: f64 = 2.0;
/// The newest row must be no older than this to be trusted (the watchdog's
/// staleness bound, `quota_samples_at`).
const TRUST_SECS: i64 = 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub observed_at: i64,
    pub used: f64,
    pub resets_at: i64,
}

/// The thinned history of one bucket window, oldest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub bucket: String,
    pub qualifier: Option<String>,
    pub window_minutes: u32,
    pub points: Vec<Point>,
}

/// One quota window of one account: the four dimensions the watchdog keys on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Key {
    source: PathBuf,
    bucket: String,
    qualifier: Option<String>,
    window_minutes: Option<u32>,
}

impl Key {
    fn of(source: &std::path::Path, row: &Row) -> Self {
        Self {
            source: source.to_path_buf(),
            bucket: row.bucket.clone(),
            qualifier: row.qualifier.clone(),
            window_minutes: row.window_minutes,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct SeriesSet(BTreeMap<Key, Vec<Point>>);

impl SeriesSet {
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.values().all(Vec::is_empty)
    }

    /// Fold one rollout's series of `source` in, thinning the union again.
    pub(crate) fn add(&mut self, source: &std::path::Path, series: Series) {
        let key = Key {
            source: source.to_path_buf(),
            bucket: series.bucket,
            qualifier: series.qualifier,
            window_minutes: Some(series.window_minutes),
        };
        let held = self.0.entry(key).or_default();
        held.extend(series.points);
        *held = thin(std::mem::take(held), series.window_minutes);
    }
}

/// How far back a pace looks: a fifth of the window, between 30 min and 12 h.
fn lookback_secs(window_minutes: u32) -> i64 {
    (i64::from(window_minutes) * 60 / 5).clamp(30 * 60, 12 * 3600)
}

/// Keep at most [`RETAINED`] points, newest always, each at least `spacing`
/// older than the one kept before it; equal stamps keep the HIGHER used.
pub fn thin(mut points: Vec<Point>, window_minutes: u32) -> Vec<Point> {
    points.sort_by(|left, right| {
        left.observed_at
            .cmp(&right.observed_at)
            .then_with(|| right.used.total_cmp(&left.used))
    });
    points.dedup_by_key(|point| point.observed_at);
    let spacing = (lookback_secs(window_minutes) / i64::from(RETAINED)).max(MIN_SPACING_SECS);
    let mut kept: Vec<Point> = Vec::new();
    for point in points.into_iter().rev() {
        if kept
            .last()
            .is_none_or(|last| last.observed_at.saturating_sub(point.observed_at) >= spacing)
        {
            kept.push(point);
        }
        if kept.len() == usize::from(RETAINED) {
            break;
        }
    }
    kept.reverse();
    kept
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Pace {
    /// No verdict applies: not trusted, blind, unlimited or capped.
    Absent,
    /// A trusted row whose history cannot support a rate yet.
    Unknown,
    /// Movement under the quantization floor: at most this many %/h.
    Bound { per_h: f64 },
    Rate {
        per_h: f64,
        eta_secs: f64,
        before_reset: bool,
    },
}

/// Seconds as an exact f64 (i32-bounded, so the conversion is total).
fn secs(value: i64) -> f64 {
    f64::from(i32::try_from(value.clamp(0, i64::from(i32::MAX))).unwrap_or(i32::MAX))
}

impl Pace {
    /// The pace of `newest`, the row the table shows, which is always the exact endpoint.
    /// Newer, other-instance and pre-restart history is ignored; both ends use `policy`.
    pub(crate) fn of(
        policy: &Policy,
        window_minutes: u32,
        newest: Point,
        history: &[Point],
        now: i64,
    ) -> Self {
        let derived = Derived::judge(policy, newest.used, Some(newest.observed_at));
        if matches!(
            derived.effective,
            Effective::Unlimited | Effective::SpendCapped
        ) {
            return Self::Absent;
        }
        let run = run_to(newest, history);
        let lookback = lookback_secs(window_minutes);
        let Some(oldest) = run
            .iter()
            .find(|point| newest.observed_at.saturating_sub(point.observed_at) <= lookback)
        else {
            return Self::Unknown;
        };
        let span = newest.observed_at.saturating_sub(oldest.observed_at);
        if span < lookback / 3 {
            return Self::Unknown;
        }
        // Rounding may leave an older point a hair above the newest: never a
        // negative movement.
        let earlier = Derived::judge(
            policy,
            oldest.used.min(newest.used),
            Some(newest.observed_at),
        )
        .judged();
        let latest = derived.judged();
        let span_h = secs(span) / 3600.0;
        let delta = latest - earlier;
        if !(delta.is_finite() && span_h > 0.0 && delta >= 0.0) {
            return Self::Unknown;
        }
        if delta < MIN_DELTA_PP {
            return Self::Bound {
                per_h: MIN_DELTA_PP / span_h,
            };
        }
        let per_h = delta / span_h;
        let eta_secs = ((100.0 - latest) / per_h * 3600.0).max(0.0);
        if !(per_h.is_finite() && per_h > 0.0 && eta_secs.is_finite()) {
            return Self::Unknown;
        }
        Self::Rate {
            per_h,
            eta_secs,
            before_reset: eta_secs < secs(newest.resets_at.saturating_sub(now)),
        }
    }
}

impl Pace {
    /// The PACE cell: the ETA is rounded once, here.
    pub(crate) fn cell(self) -> String {
        match self {
            Self::Absent => "-".to_owned(),
            Self::Unknown => "unknown".to_owned(),
            // An upper bound rounds UP, or `<0.2` would claim less than the rule allows.
            Self::Bound { per_h } => format!("<{:.1}%/h", (per_h * 10.0 - 1e-9).ceil() / 10.0),
            Self::Rate {
                per_h,
                eta_secs,
                before_reset,
            } => {
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "a finite, non-negative ETA of at most a few million seconds"
                )]
                let eta = eta_secs.round() as i64;
                let flag = if before_reset { "!" } else { "" };
                format!("{per_h:.1}%/h ~{}{flag}", span_label(eta))
            }
        }
    }
}

/// The same-instance run of points ending at `newest`, restarting on a drop past rounding.
fn run_to(newest: Point, history: &[Point]) -> Vec<Point> {
    let mut older: Vec<Point> = history
        .iter()
        .copied()
        .filter(|point| {
            point.observed_at < newest.observed_at
                && point.resets_at.abs_diff(newest.resets_at) <= INSTANCE_SECS
        })
        .collect();
    older.sort_by(|left, right| {
        left.observed_at
            .cmp(&right.observed_at)
            .then_with(|| right.used.total_cmp(&left.used))
    });
    older.dedup_by_key(|point| point.observed_at);
    let mut run: Vec<Point> = Vec::new();
    let mut peak = f64::MIN;
    for point in older.into_iter().chain([newest]) {
        if point.used < peak - ROUNDING_PP {
            run.clear();
            peak = f64::MIN;
        }
        peak = peak.max(point.used);
        run.push(point);
    }
    run
}

/// One key's verdict, and the row it was computed from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Verdict {
    pub(crate) observed_at: i64,
    pub(crate) used: f64,
    pub(crate) pace: Pace,
}

#[derive(Debug, Default)]
pub(crate) struct PaceTable(BTreeMap<Key, Verdict>);

struct Gathered<'a> {
    /// The newest row of the key, trusted or not.
    winner: &'a Row,
    /// The policy every trusted row of the key merges to.
    merged: Option<Reading>,
    points: Vec<Point>,
}

impl PaceTable {
    /// Judge every key of `groups`: identity before trust. The newest stamped row wins
    /// among ALL rows (malformed or expired included) and is gated after; a newer invalid
    /// row leaves the key with no pace, never an older neighbour's.
    pub(crate) fn build(groups: &[Group], series: &SeriesSet, now: i64) -> Self {
        let mut gathered: BTreeMap<Key, Gathered<'_>> = BTreeMap::new();
        for group in groups {
            let Some(source) = group.source.as_deref() else {
                continue;
            };
            for row in &group.rows {
                if row.observed_at.is_none() {
                    continue;
                }
                let entry = gathered
                    .entry(Key::of(source, row))
                    .or_insert_with(|| Gathered {
                        winner: row,
                        merged: None,
                        points: Vec::new(),
                    });
                if beats(row, entry.winner) {
                    entry.winner = row;
                }
                if !trusted(row, now) {
                    continue;
                }
                let Some(reading) = Reading::of(group.policy.clone(), row.clone()) else {
                    continue;
                };
                if let Some(resets_at) = row.resets_at {
                    entry.points.push(Point {
                        observed_at: reading.observed_at,
                        used: reading.used,
                        resets_at,
                    });
                }
                match entry.merged.as_mut() {
                    Some(merged) => {
                        merged.adopt(&reading);
                    }
                    None => entry.merged = Some(reading),
                }
            }
        }
        let mut table = BTreeMap::new();
        for (key, gathered) in gathered {
            let verdict = key.verdict(&gathered, series, now);
            table.insert(key, verdict);
        }
        Self(table)
    }

    #[cfg(test)]
    pub(crate) fn entries(&self) -> impl Iterator<Item = (&Key, &Verdict)> {
        self.0.iter()
    }
}

impl PaceTable {
    /// The PACE cell of one rendered row.
    pub(crate) fn cell_for(&self, group: &Group, row: &Row, claimed: &mut BTreeSet<Key>) -> String {
        let Some(source) = group.source.as_deref() else {
            return "-".to_owned();
        };
        let key = Key::of(source, row);
        let shown_is_winner = |verdict: &Verdict| {
            row.observed_at == Some(verdict.observed_at)
                && row_percent(row).is_some_and(|used| used.total_cmp(&verdict.used).is_eq())
        };
        match self.0.get(&key) {
            Some(verdict) if shown_is_winner(verdict) && claimed.insert(key) => verdict.pace.cell(),
            _ => "-".to_owned(),
        }
    }
}

impl Key {
    fn verdict(&self, gathered: &Gathered<'_>, series: &SeriesSet, now: i64) -> Verdict {
        let winner = gathered.winner;
        let used = row_percent(winner);
        let shown = |pace| Verdict {
            observed_at: winner.observed_at.unwrap_or_default(),
            used: used.unwrap_or_default(),
            pace,
        };
        let (Some(used), Some(resets_at), Some(window), Some(merged)) = (
            used,
            winner.resets_at,
            self.window_minutes,
            gathered.merged.as_ref(),
        ) else {
            return shown(Pace::Absent);
        };
        if !trusted(winner, now) {
            return shown(Pace::Absent);
        }
        let newest = Point {
            observed_at: winner.observed_at.unwrap_or_default(),
            used,
            resets_at,
        };
        let mut history = gathered.points.clone();
        if let Some(held) = series.0.get(self) {
            history.extend(held.iter().copied());
        }
        shown(Pace::of(&merged.policy, window, newest, &history, now))
    }
}

/// Whether `row` is the key's newest: later stamp; on a tie a usable row, then higher used,
/// then first seen.
fn beats(row: &Row, held: &Row) -> bool {
    row.observed_at > held.observed_at
        || (row.observed_at == held.observed_at
            && match (row_percent(row), row_percent(held)) {
                (Some(new), Some(old)) => new > old,
                (Some(_), None) => true,
                _ => false,
            })
}

/// The `quota_samples_at` gates: fresh or stale by its own clock, within the trust bound.
fn trusted(row: &Row, now: i64) -> bool {
    matches!(row.status, Status::Fresh | Status::Stale)
        && matches!(
            freshness(row.observed_at, row.resets_at, now),
            Status::Fresh | Status::Stale
        )
        && row.observed_at.is_some_and(|at| {
            now.saturating_sub(at) <= TRUST_SECS && at.saturating_sub(now) < FUTURE_SKEW_SECS
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Key, Pace, PaceTable, Point, Series, SeriesSet, lookback_secs, thin};
    use crate::quota::{Account, Credits, Group, Policy, Row, Status};
    use crate::tool::ToolKind;

    const NOW: i64 = 1_800_000_000;
    const RESET: i64 = NOW + 2 * 3600;

    fn at(ago: i64, used: f64) -> Point {
        Point {
            observed_at: NOW - ago,
            used,
            resets_at: RESET,
        }
    }

    fn raw() -> Policy {
        Policy::for_tests(None, Account::default())
    }

    fn of(newest: Point, history: &[Point]) -> Pace {
        Pace::of(&raw(), 300, newest, history, NOW)
    }

    fn rate(pace: Pace) -> (f64, f64, bool) {
        match pace {
            Pace::Rate {
                per_h,
                eta_secs,
                before_reset,
            } => (per_h, eta_secs, before_reset),
            other => panic!("expected a rate, got {other:?}"),
        }
    }

    #[test]
    fn a_rate_is_judged_per_hour_and_flagged_only_when_it_empties_before_the_reset() {
        let (per_h, eta, flagged) = rate(of(at(0, 16.0), &[at(3600, 10.0), at(1800, 13.0)]));
        assert!((per_h - 6.0).abs() < 1e-9 && (eta - 14.0 * 3600.0).abs() < 1.0);
        assert!(!flagged, "14 h to empty, reset in 2 h is not before it");
        let (_, eta, flagged) = rate(of(at(0, 90.0), &[at(3600, 60.0)]));
        assert!((eta - 1200.0).abs() < 1.0 && flagged);
        let (_, eta, flagged) = rate(of(at(0, 105.0), &[at(3600, 60.0)]));
        assert!(eta == 0.0 && flagged, "USED over 100 is empty now");
    }

    #[test]
    fn too_little_span_or_movement_never_becomes_a_number() {
        assert_eq!(of(at(0, 16.0), &[at(600, 10.0)]), Pace::Unknown);
        assert_eq!(of(at(0, 16.0), &[]), Pace::Unknown);
        assert_eq!(of(at(0, 16.0), &[at(7200, 1.0)]), Pace::Unknown);
        let Pace::Bound { per_h } = of(at(0, 11.0), &[at(3600, 10.0)]) else {
            panic!("a 1 pp tick over an hour is a bound");
        };
        assert!((per_h - 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_bound_is_printed_rounded_up_so_it_never_claims_less_than_the_rule_allows() {
        // Weekly: 1.99 pp over 9 h bounds the rate at 2/9 = 0.222 %/h, never "<0.2".
        let pace = Pace::of(&raw(), 10_080, at(0, 11.99), &[at(9 * 3600, 10.0)], NOW);
        let Pace::Bound { per_h } = pace else {
            panic!("1.99 pp over 9 h is a bound, got {pace:?}");
        };
        assert!(per_h > 0.2, "{per_h}");
        assert_eq!(pace.cell(), "<0.3%/h");
        assert_eq!(Pace::Bound { per_h: 0.2 }.cell(), "<0.2%/h");
    }

    #[test]
    fn the_lookback_is_a_fifth_of_the_window_between_half_an_hour_and_twelve() {
        assert_eq!(
            lookback_secs(60),
            30 * 60,
            "a short window floors at 30 min"
        );
        assert_eq!(lookback_secs(300), 3600);
        assert_eq!(
            lookback_secs(10_080),
            12 * 3600,
            "a long window caps at 12 h"
        );
    }

    #[test]
    fn a_span_of_exactly_a_third_of_the_lookback_counts_and_one_second_less_does_not() {
        assert_eq!(of(at(0, 16.0), &[at(1199, 10.0)]), Pace::Unknown);
        let (per_h, ..) = rate(of(at(0, 16.0), &[at(1200, 10.0)]));
        assert!((per_h - 18.0).abs() < 1e-9, "6 pp in a third of an hour");
    }

    #[test]
    fn an_eta_equal_to_the_time_left_is_not_before_the_reset() {
        let at_reset = |left: i64, ago: i64, used: f64| Point {
            resets_at: NOW + left,
            ..at(ago, used)
        };
        // 40 pp per hour from 90 % is exactly 900 s from empty.
        let (_, eta, flagged) = rate(of(at_reset(900, 0, 90.0), &[at_reset(900, 3600, 50.0)]));
        assert!((eta - 900.0).abs() < 1e-9 && !flagged, "{eta} {flagged}");
        let (_, _, flagged) = rate(of(at_reset(901, 0, 90.0), &[at_reset(901, 3600, 50.0)]));
        assert!(flagged, "empty 1 s before the reset is before it");
    }

    #[test]
    fn a_hostile_reading_is_unknown_never_an_infinite_rate() {
        // 1e308 is a finite, parseable percentage; over the 5 h floor span of a
        // third of an hour its rate overflows to +inf while its ETA reads 0.
        for bad in [f64::NAN, f64::INFINITY, 1e308] {
            assert_eq!(of(at(0, bad), &[at(1200, 0.0)]), Pace::Unknown, "{bad}");
        }
    }

    #[test]
    fn a_drop_is_rounding_up_to_half_a_point_and_a_restart_beyond() {
        // Exactly half a point is still rounding.
        let Pace::Bound { .. } = of(at(0, 10.0), &[at(3600, 10.5)]) else {
            panic!("a 0.5 pp drop is one run, not a restart");
        };
        // 10.4 -> 10.0 is vendor rounding: the series stays one run, never negative.
        let Pace::Bound { .. } = of(at(0, 10.1), &[at(3600, 10.4), at(1800, 10.0)]) else {
            panic!("rounding must not restart or go negative");
        };
        // 30 -> 5 is a window restart: only the points after it count.
        assert_eq!(
            of(at(0, 9.0), &[at(3600, 30.0), at(600, 5.0)]),
            Pace::Unknown
        );
        let (per_h, ..) = rate(of(at(0, 9.0), &[at(7000, 30.0), at(3600, 5.0)]));
        assert!((per_h - 4.0).abs() < 1e-9);
    }

    #[test]
    fn history_is_read_as_a_set_not_as_an_order() {
        let sorted = [at(3600, 10.0), at(1800, 13.0)];
        let shuffled = [
            at(1800, 13.0),
            at(3600, 10.0),
            at(3600, 4.0),
            at(1800, 13.0),
        ];
        assert_eq!(of(at(0, 16.0), &sorted), of(at(0, 16.0), &shuffled));
    }

    #[test]
    fn the_endpoint_is_always_the_newest_row_the_table_shows() {
        let base = of(at(60, 16.0), &[at(3660, 10.0)]);
        let elsewhere = Point {
            resets_at: RESET + 3600,
            ..at(2000, 99.0)
        };
        // Newer than the shown row, the same stamp, and another window instance.
        let noisy = [at(3660, 10.0), at(0, 99.0), at(60, 50.0), elsewhere];
        assert_eq!(of(at(60, 16.0), &noisy), base);
        // A newer stamp FOLLOWED by an older record: the older one is the shown
        // row, so its own USED ends the series.
        let (per_h, ..) = rate(of(at(600, 12.0), &[at(4200, 10.0), at(0, 40.0)]));
        assert!((per_h - 2.0).abs() < 1e-9, "the 40.0 reading is never used");
    }

    #[test]
    fn pace_is_judged_under_the_current_policy() {
        let history = [at(3600, 10.0)];
        let reset_once = Policy::for_tests(Some(1), Account::default());
        let (per_h, ..) = rate(Pace::of(&reset_once, 300, at(0, 16.0), &history, NOW));
        assert!((per_h - 3.0).abs() < 1e-9, "declared resets halve the pace");
        let unlimited = Account {
            credits: Credits::Unlimited,
            credits_observed_at: Some(NOW),
            ..Account::default()
        };
        let capped = Account {
            spend_control_reached: Some(true),
            spend_observed_at: Some(NOW),
            ..Account::default()
        };
        for account in [unlimited, capped] {
            let policy = Policy::for_tests(None, account);
            assert_eq!(
                Pace::of(&policy, 300, at(0, 16.0), &history, NOW),
                Pace::Absent
            );
        }
    }

    #[test]
    fn retention_keeps_the_newest_spaced_and_bounded_per_window_class() {
        let five_hour: Vec<Point> = (0..200_i32)
            .map(|n| at(i64::from(n) * 60, 200.0 - f64::from(n)))
            .collect();
        let kept = thin(five_hour, 300);
        assert_eq!(kept.len(), 32);
        assert_eq!(kept.last().map(|p| p.observed_at), Some(NOW));
        assert!(
            kept.windows(2)
                .all(|w| w[1].observed_at - w[0].observed_at >= 300)
        );
        let weekly: Vec<Point> = (0..80_i32)
            .map(|n| at(i64::from(n) * 1350, 80.0 - f64::from(n)))
            .collect();
        let kept = thin(weekly, 10_080);
        assert_eq!(kept.len(), 32);
        assert!(NOW - kept[0].observed_at >= 11 * 3600, "reaches back ~12 h");
        let dense: Vec<Point> = (0..800_i32)
            .map(|n| at(i64::from(n) * 60, 800.0 - f64::from(n)))
            .collect();
        let kept_dense = thin(dense, 10_080);
        assert_eq!(kept_dense.len(), 32);
        assert!(
            kept_dense
                .windows(2)
                .all(|w| w[1].observed_at - w[0].observed_at >= 1350),
            "weekly spacing is a 32nd of the 12 h lookback, not the 5 min floor"
        );
        let tie = thin(vec![at(0, 5.0), at(0, 7.0), at(0, 6.0)], 300);
        assert_eq!(tie, [at(0, 7.0)], "an equal stamp keeps the higher used");
        let (per_h, ..) = rate(Pace::of(&raw(), 10_080, at(0, 80.0), &kept[1..], NOW));
        assert!(per_h > 2.0, "a thinned weekly series carries a rate");
    }

    #[test]
    fn an_extreme_reset_epoch_is_another_window_instance_not_a_panic() {
        let base = of(at(0, 16.0), &[at(3600, 10.0)]);
        for extreme in [i64::MIN, i64::MAX, -1, 0] {
            let far = Point {
                resets_at: extreme,
                ..at(1800, 13.0)
            };
            assert_eq!(of(at(0, 16.0), &[at(3600, 10.0), far]), base, "{extreme}");
            // The shown row itself can carry the extreme reset.
            let shown = Point {
                resets_at: extreme,
                ..at(0, 16.0)
            };
            let _ = Pace::of(&raw(), 300, shown, &[far, at(3600, 10.0)], NOW);
        }
        // The same epochs arrive through the parser as plain JSON integers.
        let record = |resets: i64, time: &str| {
            format!(
                r#"{{"timestamp":"{time}","type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"limit_id":"codex","plan_type":"plus","primary":{{"used_percent":9.0,"window_minutes":300,"resets_at":{resets}}},"secondary":null}}}}}}
"#
            )
        };
        let text = [
            record(i64::MIN, "2026-09-08T08:00:00Z"),
            record(i64::MAX, "2026-09-08T08:30:00Z"),
            record(1_788_861_600, "2026-09-08T09:00:00Z"),
        ]
        .concat();
        let now = crate::time::Timestamp::parse("2026-09-08T09:05:00Z")
            .expect("fixture")
            .epoch();
        let snapshot = crate::quota::codex::parse(text.as_bytes(), true, now).expect("parses");
        let mut series = SeriesSet::default();
        for held in snapshot.points {
            series.add(&PathBuf::from("/vendor/codex"), held);
        }
        let mut shown = group(snapshot.rows, raw());
        shown.source = Some(PathBuf::from("/vendor/codex"));
        let _ = PaceTable::build(&[shown], &series, now);
    }

    /// Lead ruling (P1.0): no live codex 5 h window exists on the measured
    /// fleet, so the 5 h class is proven here, from synthetic rollout records
    /// through the parser, the retained series and the table derivation.
    #[test]
    fn a_five_hour_window_from_rollout_records_yields_a_rate_and_an_eta() {
        let stamp = |text: &str| {
            crate::time::Timestamp::parse(text)
                .expect("fixture")
                .epoch()
        };
        let resets = stamp("2026-09-08T11:00:00Z");
        let record = |time: &str, used: u32| {
            format!(
                r#"{{"timestamp":"{time}","type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"limit_id":"codex","plan_type":"plus","primary":{{"used_percent":{used}.0,"window_minutes":300,"resets_at":{resets}}},"secondary":null}}}}}}
"#
            )
        };
        let text: String = [
            ("2026-09-08T08:00:00Z", 10),
            ("2026-09-08T08:15:00Z", 13),
            ("2026-09-08T08:30:00Z", 16),
            ("2026-09-08T08:45:00Z", 19),
            ("2026-09-08T09:00:00Z", 22),
        ]
        .iter()
        .map(|(time, used)| record(time, *used))
        .collect();
        let now = stamp("2026-09-08T09:05:00Z");
        let snapshot = crate::quota::codex::parse(text.as_bytes(), true, now).expect("parses");
        let source = PathBuf::from("/vendor/codex");
        let mut series = SeriesSet::default();
        for held in snapshot.points {
            series.add(&source, held);
        }
        let mut five_hour = group(snapshot.rows, raw());
        five_hour.source = Some(source);
        let table = PaceTable::build(&[five_hour], &series, now);
        let [(key, verdict)] = table.entries().collect::<Vec<_>>()[..] else {
            panic!("one 5 h window, one key");
        };
        assert_eq!(key.window_minutes, Some(300));
        let (per_h, eta, flagged) = rate(verdict.pace);
        assert!(
            (per_h - 12.0).abs() < 1e-9,
            "12 pp over the 60 min lookback"
        );
        assert!((eta - 6.5 * 3600.0).abs() < 1.0, "78 pp left at 12 %/h");
        assert!(!flagged, "6.5 h to empty, the window resets in under 2 h");
    }

    fn row(used: Option<&str>, ago: i64, resets: i64, status: Status) -> Row {
        Row {
            bucket: "codex".to_owned(),
            qualifier: Some("plus".to_owned()),
            window_minutes: Some(300),
            used_percent: used.map(str::to_owned),
            resets_at: Some(resets),
            observed_at: Some(NOW - ago),
            status,
        }
    }

    fn group(rows: Vec<Row>, policy: Policy) -> Group {
        Group {
            profiles: Vec::new(),
            tool: ToolKind::Codex,
            home: None,
            source: Some(PathBuf::from("/vendor/codex")),
            clients: Vec::new(),
            rollout: None,
            owner: None,
            rows,
            hint: None,
            summary: None,
            policy,
            notes: Vec::new(),
        }
    }

    fn history() -> SeriesSet {
        let mut set = SeriesSet::default();
        set.add(
            &PathBuf::from("/vendor/codex"),
            Series {
                bucket: "codex".to_owned(),
                qualifier: Some("plus".to_owned()),
                window_minutes: 300,
                points: vec![at(3660, 10.0), at(1800, 13.0)],
            },
        );
        set
    }

    fn verdicts(groups: &[Group], series: &SeriesSet) -> Vec<(f64, Pace)> {
        PaceTable::build(groups, series, NOW)
            .entries()
            .map(|(_, verdict)| (verdict.used, verdict.pace))
            .collect()
    }

    #[test]
    fn a_trusted_newest_row_and_the_series_make_a_verdict() {
        let fresh = group(vec![row(Some("16.0"), 60, RESET, Status::Fresh)], raw());
        let got = verdicts(std::slice::from_ref(&fresh), &history());
        assert!(
            matches!(got[..], [(u, Pace::Rate { .. })] if (u - 16.0).abs() < 1e-9),
            "{got:?}"
        );
        assert_eq!(
            verdicts(&[fresh], &SeriesSet::default()),
            [(16.0, Pace::Unknown)],
            "a trusted row with no history is unknown, not absent"
        );
    }

    #[test]
    fn a_newer_row_that_cannot_be_trusted_leaves_no_older_pace_behind() {
        let old = row(Some("16.0"), 300, RESET, Status::Fresh);
        let newer = |used: Option<&str>, resets: i64, status| row(used, 60, resets, status);
        for bad in [
            newer(Some("17.0"), NOW - 10, Status::Unknown),
            newer(None, RESET, Status::Unknown),
            newer(Some("NaN"), RESET, Status::Unknown),
            newer(Some("17.0"), RESET, Status::Unknown),
        ] {
            let got = verdicts(&[group(vec![old.clone(), bad], raw())], &history());
            assert!(matches!(got[..], [(_, Pace::Absent)]), "{got:?}");
        }
        let skewed = Row {
            observed_at: Some(NOW + 600),
            ..old.clone()
        };
        let got = verdicts(&[group(vec![old, skewed], raw())], &history());
        assert!(matches!(got[..], [(_, Pace::Absent)]), "{got:?}");
    }

    #[test]
    fn stale_rows_are_trusted_up_to_an_hour() {
        let stale = |ago| group(vec![row(Some("16.0"), ago, RESET, Status::Stale)], raw());
        let mut set = SeriesSet::default();
        set.add(
            &PathBuf::from("/vendor/codex"),
            Series {
                bucket: "codex".to_owned(),
                qualifier: Some("plus".to_owned()),
                window_minutes: 300,
                points: vec![at(4500, 10.0), at(3000, 12.0)],
            },
        );
        assert!(matches!(
            verdicts(&[stale(1200)], &set)[..],
            [(_, Pace::Rate { .. })]
        ));
        assert!(matches!(
            verdicts(&[stale(3601)], &set)[..],
            [(_, Pace::Absent)]
        ));
    }

    #[test]
    fn on_an_equal_stamp_the_higher_used_row_is_shown_and_every_policy_merges() {
        let low = group(vec![row(Some("40.0"), 60, RESET, Status::Fresh)], raw());
        let high = group(vec![row(Some("41.0"), 60, RESET, Status::Fresh)], raw());
        for groups in [[low.clone(), high.clone()], [high, low.clone()]] {
            let got = verdicts(&groups, &SeriesSet::default());
            assert_eq!(got, [(41.0, Pace::Unknown)]);
        }
        let capped = Policy::for_tests(
            None,
            Account {
                spend_control_reached: Some(true),
                spend_observed_at: Some(NOW),
                ..Account::default()
            },
        );
        let other = group(vec![row(Some("40.0"), 60, RESET, Status::Fresh)], capped);
        let got = verdicts(&[low, other], &history());
        assert_eq!(
            got,
            [(40.0, Pace::Absent)],
            "a cap on any rollout binds the key"
        );
    }

    #[test]
    fn on_an_equal_stamp_and_used_the_first_seen_row_keeps_the_key() {
        // Only the first row shares the history's window instance.
        let first = group(vec![row(Some("16.0"), 60, RESET, Status::Fresh)], raw());
        let second = group(
            vec![row(Some("16.0"), 60, RESET + 10_000, Status::Fresh)],
            raw(),
        );
        let got = verdicts(&[first, second], &history());
        assert!(matches!(got[..], [(_, Pace::Rate { .. })]), "{got:?}");
    }

    #[test]
    fn a_usable_row_beats_an_unusable_one_of_the_same_stamp() {
        let blank = group(vec![row(None, 60, RESET, Status::Fresh)], raw());
        let usable = group(vec![row(Some("16.0"), 60, RESET, Status::Fresh)], raw());
        let got = verdicts(&[blank, usable], &history());
        assert!(matches!(got[..], [(_, Pace::Rate { .. })]), "{got:?}");
    }

    #[test]
    fn pace_cells_are_spelled_once_and_never_guess() {
        assert_eq!(Pace::Absent.cell(), "-");
        assert_eq!(Pace::Unknown.cell(), "unknown");
        assert_eq!(Pace::Bound { per_h: 2.0 }.cell(), "<2.0%/h");
        let rate = |per_h, eta_secs, before_reset| {
            Pace::Rate {
                per_h,
                eta_secs,
                before_reset,
            }
            .cell()
        };
        assert_eq!(rate(2.4, 7799.6, true), "2.4%/h ~2h10m!");
        assert_eq!(rate(6.0, 50_400.0, false), "6.0%/h ~14h00m");
        assert_eq!(rate(30.0, 0.0, true), "30.0%/h ~0m!");
        assert_eq!(rate(1.0, 100_000.0, false), "1.0%/h ~1d 3h");
    }

    /// Per shown row, in render order, the cell the table would draw.
    fn drawn(all: &[Group], shown: &[Group], series: &SeriesSet) -> Vec<String> {
        let table = PaceTable::build(all, series, NOW);
        let mut claimed = std::collections::BTreeSet::new();
        shown
            .iter()
            .flat_map(|group| group.rows.iter().map(move |row| (group, row)))
            .map(|(group, row)| table.cell_for(group, row, &mut claimed))
            .collect()
    }

    #[test]
    fn identical_duplicates_draw_one_cell_on_the_render_order_winner() {
        let twin = || group(vec![row(Some("16.0"), 60, RESET, Status::Fresh)], raw());
        let both = [twin(), twin()];
        let got = drawn(&both, &both, &history());
        assert!(got[0].contains("%/h") && got[1] == "-", "{got:?}");
    }

    #[test]
    fn a_hidden_newest_winner_dashes_every_shown_older_row() {
        let hidden = group(vec![row(Some("18.0"), 30, RESET, Status::Fresh)], raw());
        let shown = group(vec![row(Some("16.0"), 60, RESET, Status::Fresh)], raw());
        let got = drawn(&[hidden, shown.clone()], &[shown], &history());
        assert_eq!(got, ["-"], "never a stale neighbour's pace");
    }

    #[test]
    fn only_the_row_the_pace_was_computed_from_carries_it() {
        let low = group(vec![row(Some("40.0"), 60, RESET, Status::Fresh)], raw());
        let high = group(vec![row(Some("41.0"), 60, RESET, Status::Fresh)], raw());
        let both = [low, high];
        let got = drawn(&both, &both, &SeriesSet::default());
        assert_eq!(got, ["-", "unknown"], "the higher used is the winner");
        let expired = group(
            vec![row(Some("16.0"), 60, NOW - 10, Status::Unknown)],
            raw(),
        );
        assert_eq!(
            drawn(
                std::slice::from_ref(&expired),
                std::slice::from_ref(&expired),
                &history()
            ),
            ["-"]
        );
    }

    #[test]
    fn the_table_draws_the_pace_column_between_credits_and_resets() {
        let shown = group(vec![row(Some("16.0"), 60, RESET, Status::Fresh)], raw());
        let table = PaceTable::build(std::slice::from_ref(&shown), &history(), NOW);
        let text = super::super::render_with_pace(std::slice::from_ref(&shown), &table, None, NOW);
        let header: Vec<&str> = text
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .collect();
        let at = |name| header.iter().position(|word| *word == name);
        assert_eq!(at("PACE"), at("CREDITS").map(|column| column + 1), "{text}");
        assert_eq!(at("RESETS"), at("PACE").map(|column| column + 1), "{text}");
        assert!(text.contains("%/h ~"), "{text}");
        let blind = super::super::render_at(&[shown], None, NOW);
        assert!(!blind.contains("%/h"), "no table, no pace: {blind}");
    }

    #[test]
    fn a_key_names_its_four_dimensions() {
        let source = PathBuf::from("/vendor/codex");
        let base = Key::of(&source, &row(Some("1"), 0, RESET, Status::Fresh));
        let mut other = row(Some("1"), 0, RESET, Status::Fresh);
        other.qualifier = None;
        assert_ne!(base, Key::of(&source, &other));
        assert_eq!(base.window_minutes, Some(300));
    }
}
