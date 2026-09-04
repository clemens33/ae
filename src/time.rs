//! The one timestamp shape ae's formats use.
//!
//! Every event's `ts` is ISO 8601 UTC at second precision, and
//! `generated_at` uses the same form. Two uses, one shape, so one
//! type: a [`Timestamp`] is epoch seconds that knows how to read and write that
//! spelling and nothing else.
//!
//! Hand-written for the reason a JSON crate is: the shape is a contract row, and
//! a contract this crate must not get wrong is a contract it should own. The
//! civil-date arithmetic is Howard Hinnant's `days_from_civil` /
//! `civil_from_days` (public domain), which is exact for the whole proleptic
//! Gregorian range and needs no table.

use std::fmt;

/// A point in time, as epoch seconds (UTC).
///
/// ```
/// use ae::time::Timestamp;
/// let t = Timestamp::parse("2026-05-29T14:00:00Z").expect("the example parses");
/// assert_eq!(t.to_string(), "2026-05-29T14:00:00Z");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    /// Wrap epoch seconds.
    #[must_use]
    pub const fn from_epoch(seconds: i64) -> Self {
        Self(seconds)
    }

    /// The epoch seconds behind this timestamp.
    #[must_use]
    pub const fn epoch(self) -> i64 {
        self.0
    }

    /// Read the documented spelling: `YYYY-MM-DDTHH:MM:SSZ`.
    ///
    /// Strict on purpose. Accepting fractional
    /// seconds or a numeric offset would be a *tolerance* nothing grants, and
    /// tolerance in a reader is how an undocumented format quietly becomes the
    /// format. A caller that meets something else gets `None` and degrades —
    /// it never guesses.
    ///
    /// ```
    /// use ae::time::Timestamp;
    /// assert!(Timestamp::parse("2026-05-19T07:29:45Z").is_some());
    /// assert!(Timestamp::parse("2026-05-19T07:29:45.500Z").is_none());
    /// assert!(Timestamp::parse("2026-05-19T07:29:45+02:00").is_none());
    /// ```
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() != 20 {
            return None;
        }
        for (index, expected) in [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')] {
            if bytes.get(index) != Some(&expected) {
                return None;
            }
        }
        if bytes.get(19) != Some(&b'Z') {
            return None;
        }
        let year = number(text.get(0..4)?)?;
        let month = number(text.get(5..7)?)?;
        let day = number(text.get(8..10)?)?;
        let hour = number(text.get(11..13)?)?;
        let minute = number(text.get(14..16)?)?;
        let second = number(text.get(17..19)?)?;
        if hour > 23 || minute > 59 || second > 59 {
            return None;
        }
        let days = days_from_civil(year, month, day);
        // The ONE date check, and it is exact: any month or day the calendar
        // does not have — 13, 00, 2026-02-30, 1900-02-29 — normalises to some
        // other date on the way back, so it fails to round-trip.
        if civil_from_days(days) != (year, month, day) {
            return None;
        }
        Some(Self(days * 86_400 + hour * 3600 + minute * 60 + second))
    }

    /// This instant, from the system clock.
    #[must_use]
    pub fn now() -> Self {
        let now = std::time::SystemTime::now();
        match now.duration_since(std::time::UNIX_EPOCH) {
            Ok(since) => Self(i64::try_from(since.as_secs()).unwrap_or(i64::MAX)),
            Err(before) => Self(
                i64::try_from(before.duration().as_secs()).map_or(i64::MIN, i64::saturating_neg),
            ),
        }
    }

    /// Whole seconds from `self` to `later`, saturating rather than wrapping.
    #[must_use]
    pub const fn seconds_until(self, later: Self) -> i64 {
        later.0.saturating_sub(self.0)
    }
}

impl fmt::Display for Timestamp {
    /// Write the documented spelling: `YYYY-MM-DDTHH:MM:SSZ`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let days = self.0.div_euclid(86_400);
        let rest = self.0.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        write!(
            f,
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
            rest / 3600,
            (rest % 3600) / 60,
            rest % 60
        )
    }
}

/// Parse a run of ASCII digits.
fn number(text: &str) -> Option<i64> {
    if text.bytes().all(|b| b.is_ascii_digit()) {
        text.parse().ok()
    } else {
        None
    }
}

/// Days since 1970-01-01 for a proleptic-Gregorian civil date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The inverse of [`days_from_civil`].
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_position = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_position + 2) / 5 + 1;
    let month = month_position + if month_position < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::{Timestamp, civil_from_days, days_from_civil};

    #[test]
    fn the_sc_509_example_round_trips() {
        // commands.md's --json digest block, verbatim.
        let t = Timestamp::parse("2026-05-29T14:00:00Z").expect("the documented form parses");
        assert_eq!(t.to_string(), "2026-05-29T14:00:00Z");
    }

    #[test]
    fn the_events_md_example_round_trips() {
        // ISO 8601 UTC, second precision.
        let t = Timestamp::parse("2026-05-19T07:29:45Z").expect("the documented form parses");
        assert_eq!(t.to_string(), "2026-05-19T07:29:45Z");
    }

    #[test]
    fn the_epoch_itself_is_the_zero_point() {
        let t = Timestamp::parse("1970-01-01T00:00:00Z").expect("the epoch parses");
        assert_eq!(t.epoch(), 0);
        assert_eq!(Timestamp::from_epoch(0).to_string(), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn leap_days_and_century_rules_are_exact() {
        // 2000 is a leap year, 1900 and 2100 are not — the three cases a
        // hand-rolled calendar gets wrong.
        assert!(Timestamp::parse("2000-02-29T00:00:00Z").is_some());
        assert!(Timestamp::parse("1900-02-29T00:00:00Z").is_none());
        assert!(Timestamp::parse("2100-02-29T00:00:00Z").is_none());
        assert!(Timestamp::parse("2024-02-29T00:00:00Z").is_some());
    }

    #[test]
    fn a_day_that_does_not_exist_is_refused() {
        for absent in [
            "2026-02-30T00:00:00Z",
            "2026-04-31T00:00:00Z",
            "2026-00-10T00:00:00Z",
            "2026-13-10T00:00:00Z",
            "2026-01-00T00:00:00Z",
            "2026-01-32T00:00:00Z",
        ] {
            assert!(Timestamp::parse(absent).is_none(), "{absent} is not a date");
        }
    }

    #[test]
    fn an_impossible_clock_reading_is_refused() {
        for absurd in [
            "2026-05-19T24:00:00Z",
            "2026-05-19T07:60:00Z",
            "2026-05-19T07:29:60Z",
        ] {
            assert!(Timestamp::parse(absurd).is_none(), "{absurd} is not a time");
        }
    }

    #[test]
    fn a_shape_the_contract_does_not_document_is_refused() {
        // ONE spelling is documented.
        for other in [
            "2026-05-19T07:29:45.500Z",
            "2026-05-19T07:29:45+02:00",
            "2026-05-19 07:29:45Z",
            "2026-05-19T07:29:45",
            "26-05-19T07:29:45Z",
            "2026-05-19T07:29:45z",
            "",
            "not a timestamp at all",
        ] {
            assert!(
                Timestamp::parse(other).is_none(),
                "{other:?} must not parse"
            );
        }
    }

    #[test]
    fn a_non_digit_in_a_numeric_field_is_refused() {
        // `"+026-05-19T07:29:45Z"` has the right length and the right
        // separators, and `i64::from_str` would happily accept the sign.
        for sneaky in ["+026-05-19T07:29:45Z", "20x6-05-19T07:29:45Z"] {
            assert!(
                Timestamp::parse(sneaky).is_none(),
                "{sneaky} must not parse"
            );
        }
    }

    #[test]
    fn civil_arithmetic_round_trips_across_a_long_span() {
        // Every day from 1969 to 2100: the pair must be each other's inverse.
        let from = days_from_civil(1969, 1, 1);
        let to = days_from_civil(2100, 1, 1);
        for day in from..to {
            let (y, m, d) = civil_from_days(day);
            assert_eq!(days_from_civil(y, m, d), day, "{y:04}-{m:02}-{d:02}");
        }
    }

    #[test]
    fn the_civil_arithmetic_holds_on_the_far_side_of_year_zero() {
        // This module's docs claim the whole proleptic Gregorian range, and the
        // negative-year branches of both functions exist to deliver it.
        for day in days_from_civil(-5, 1, 1)..days_from_civil(5, 1, 1) {
            let (y, m, d) = civil_from_days(day);
            assert_eq!(days_from_civil(y, m, d), day, "{y}-{m:02}-{d:02}");
        }
        // Year 0 is a leap year in the proleptic calendar (divisible by 400).
        assert_eq!(days_from_civil(0, 2, 29) + 1, days_from_civil(0, 3, 1));
        assert_eq!(civil_from_days(days_from_civil(-1, 12, 31)), (-1, 12, 31));
    }

    #[test]
    fn parsing_and_formatting_are_each_other_s_inverse() {
        for day in 0..(366 * 60) {
            let t = Timestamp::from_epoch(day * 86_400 + 43_199);
            let text = t.to_string();
            assert_eq!(Timestamp::parse(&text), Some(t), "{text}");
        }
    }

    #[test]
    fn a_pre_epoch_instant_still_formats() {
        let t = Timestamp::from_epoch(-1);
        assert_eq!(t.to_string(), "1969-12-31T23:59:59Z");
        assert_eq!(Timestamp::parse("1969-12-31T23:59:59Z"), Some(t));
    }

    #[test]
    fn seconds_until_measures_in_both_directions() {
        let early = Timestamp::from_epoch(1_000);
        let late = Timestamp::from_epoch(1_300);
        assert_eq!(early.seconds_until(late), 300);
        assert_eq!(late.seconds_until(early), -300);
        // Saturating, so an absurd pair cannot wrap into a small answer.
        assert_eq!(
            Timestamp::from_epoch(i64::MIN).seconds_until(Timestamp::from_epoch(i64::MAX)),
            i64::MAX
        );
    }

    #[test]
    fn now_reads_the_clock_as_a_plausible_instant() {
        // Not a value test — a smoke test that the clock path is wired and
        // lands this side of 2020 rather than at the epoch.
        assert!(Timestamp::now().epoch() > 1_577_836_800);
    }
}
