//! Vendor quota snapshots parsed from local client-owned caches.
//!
//! Parsing stays separate from discovery and rendering: the client files are
//! hostile persisted state, while this module is a pure bytes-to-rows boundary.

pub mod claude;
pub mod codex;

use crate::json::Value;
use crate::time::Timestamp;

/// Maximum age of an observation that may be called fresh.
pub const FRESH_SECS: i64 = 15 * 60;

/// Future clock skew at which an observation becomes unknown.
pub const FUTURE_SKEW_SECS: i64 = 5 * 60;

/// Whether a parsed quota row is safe to use as current evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Observed no more than 15 minutes ago.
    Fresh,
    /// Observed more than 15 minutes ago, before its reset.
    Stale,
    /// Missing, expired, skewed, or otherwise not applicable.
    Unknown,
    /// The client exposes no usable local quota state.
    Unsupported,
    /// A present source could not be read or parsed.
    ReadError,
}

/// One vendor bucket and one of its windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Vendor bucket identifier (`weekly_all`, `codex_bengalfox`, and so on).
    pub bucket: String,
    /// Optional vendor qualifier: Claude model scope or Codex plan.
    pub qualifier: Option<String>,
    /// Window duration reported or defined by the vendor, in minutes.
    pub window_minutes: Option<u32>,
    /// Vendor utilization numeric literal, without a percent sign.
    pub used_percent: Option<String>,
    /// Window reset as Unix epoch seconds.
    pub resets_at: Option<i64>,
    /// Cache or rollout observation as Unix epoch seconds.
    pub observed_at: Option<i64>,
    /// Freshness and applicability verdict for this row.
    pub status: Status,
}

/// Parsing failed before a trustworthy snapshot could be produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Input was not valid UTF-8.
    Utf8,
    /// A complete record was not valid JSON.
    Json,
    /// A named quota container had an incompatible shape.
    Shape,
}

/// Derive a freshness verdict from the record's own clocks.
#[must_use]
pub const fn freshness(observed_at: Option<i64>, resets_at: Option<i64>, now: i64) -> Status {
    let (Some(observed), Some(reset)) = (observed_at, resets_at) else {
        return Status::Unknown;
    };
    if reset <= now || observed.saturating_sub(now) >= FUTURE_SKEW_SECS {
        return Status::Unknown;
    }
    if now.saturating_sub(observed) <= FRESH_SECS {
        Status::Fresh
    } else {
        Status::Stale
    }
}

/// Read one non-negative numeric JSON literal for later display.
pub(crate) fn percent(value: Option<&Value>) -> Option<String> {
    let literal = match value? {
        Value::Num(number) => number.to_string(),
        Value::Raw(raw) => raw.clone(),
        _ => return None,
    };
    literal
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map(|_| literal)
}

/// Read an integral JSON number as an epoch.
pub(crate) fn epoch(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Num(number) => Some(*number),
        Value::Raw(raw) => raw.parse().ok(),
        _ => None,
    }
}

/// Read a positive integral JSON number as minutes.
pub(crate) fn minutes(value: Option<&Value>) -> Option<u32> {
    let number = epoch(value)?;
    u32::try_from(number).ok().filter(|minutes| *minutes > 0)
}

/// Read the UTC RFC 3339 spellings present in vendor caches.
pub(crate) fn vendor_timestamp(text: &str) -> Option<i64> {
    let date = text.get(..19)?;
    let suffix = text.get(19..)?;
    let utc = suffix == "Z"
        || suffix == "+00:00"
        || suffix.strip_prefix('.').is_some_and(|tail| {
            tail.strip_suffix('Z')
                .or_else(|| tail.strip_suffix("+00:00"))
                .is_some_and(|fraction| {
                    !fraction.is_empty() && fraction.bytes().all(|byte| byte.is_ascii_digit())
                })
        });
    if !utc {
        return None;
    }
    let mut canonical = String::with_capacity(20);
    canonical.push_str(date);
    canonical.push('Z');
    Timestamp::parse(&canonical).map(Timestamp::epoch)
}

#[cfg(test)]
mod tests {
    use super::{FRESH_SECS, FUTURE_SKEW_SECS, Status, freshness, vendor_timestamp};

    #[test]
    fn freshness_boundaries_are_exact() {
        let now = 10_000;
        assert_eq!(
            freshness(Some(now - FRESH_SECS), Some(now + 1), now),
            Status::Fresh
        );
        assert_eq!(
            freshness(Some(now - FRESH_SECS - 1), Some(now + 1), now),
            Status::Stale
        );
        assert_eq!(freshness(Some(now), Some(now), now), Status::Unknown);
        assert_eq!(
            freshness(Some(now + FUTURE_SKEW_SECS), Some(now + 1_000), now),
            Status::Unknown
        );
        assert_eq!(
            freshness(Some(now + FUTURE_SKEW_SECS - 1), Some(now + 1_000), now),
            Status::Fresh
        );
        assert_eq!(freshness(None, Some(now + 1), now), Status::Unknown);
        assert_eq!(freshness(Some(now), None, now), Status::Unknown);
    }

    #[test]
    fn vendor_utc_timestamps_accept_the_two_observed_clients() {
        let expected = vendor_timestamp("2026-09-08T09:10:25Z");
        assert!(expected.is_some());
        assert_eq!(vendor_timestamp("2026-09-08T09:10:25.781Z"), expected);
        assert_eq!(
            vendor_timestamp("2026-09-08T09:10:25.759599+00:00"),
            expected
        );
        assert_eq!(vendor_timestamp("2026-09-08T09:10:25+02:00"), None);
    }
}
