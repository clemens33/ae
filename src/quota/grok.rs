//! Parser for bounded tails of the grok CLI's shared debug log.
//!
//! The log is a diagnostic stream, not a contract: most lines carry prompt and
//! path text, and any line may be malformed. Only a line that contains the one
//! billing message is ever parsed, and nothing from any other line is kept.

use crate::json::{self, Value};

use super::{Row, Status, freshness, percent, vendor_timestamp};

/// The exact `msg` of the one record this parser reads.
pub const MSG: &str = "billing: fetched credits config";

/// Bytes `ae quota` reads from the end of the log.
pub const TAIL_CAP: u64 = 512 * 1024;

/// Longest line that is considered at all.
const MAX_CANDIDATE: usize = 64 * 1024;

/// Shortest and longest billing period that is a window, in minutes.
const PERIOD_MINUTES: std::ops::RangeInclusive<i64> = 60..=31 * 24 * 60;

/// Longest `subscriptionTier` that reaches a cell.
const TIER_MAX: usize = 24;

/// What the newest billing record says, or nothing ae may show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// The credit window, or `None` when the newest record (or every record)
    /// is unusable: the scope then renders as unsupported.
    pub row: Option<Row>,
    /// The account may spend past the window, so 100 % is not a hard stop.
    pub on_demand_enabled: bool,
}

/// Find the newest billing record in a bounded log tail.
///
/// Set `starts_at_record_boundary` to false when the byte cap cut the first
/// line; bytes through its newline are then ignored. Bytes after the final
/// newline are an in-progress line and are ignored. A malformed line is
/// skipped, never an error: one stray line must not blind the scope.
#[must_use]
pub fn parse(bytes: &[u8], starts_at_record_boundary: bool, now: i64) -> Snapshot {
    let from = if starts_at_record_boundary {
        0
    } else {
        bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |newline| newline + 1)
    };
    let complete = bytes
        .get(from..)
        .and_then(|tail| tail.iter().rposition(|byte| *byte == b'\n'))
        .map_or(from, |newline| from + newline + 1);

    let mut newest: Option<(i64, Value)> = None;
    for line in bytes
        .get(from..complete)
        .unwrap_or_default()
        .split(|byte| *byte == b'\n')
    {
        if let Some((stamp, record)) = candidate(line)
            && newest.as_ref().is_none_or(|(held, _)| stamp >= *held)
        {
            newest = Some((stamp, record));
        }
    }
    newest.map_or_else(Snapshot::default, |(stamp, record)| {
        snapshot(&record, stamp, now)
    })
}

/// One billing record and its `ts`, or `None` for any other line.
///
/// The prefilter is on the message's own bytes, so it assumes nothing about
/// key order, spacing or quoting; the parsed `msg` must then equal it.
fn candidate(line: &[u8]) -> Option<(i64, Value)> {
    let needle = MSG.as_bytes();
    if line.len() > MAX_CANDIDATE || !line.windows(needle.len()).any(|window| window == needle) {
        return None;
    }
    let record = json::parse(std::str::from_utf8(line).ok()?).ok()?;
    if record.get_str("msg") != Some(MSG) {
        return None;
    }
    let stamp = record.get_str("ts").and_then(vendor_timestamp)?;
    Some((stamp, record))
}

fn snapshot(record: &Value, stamp: i64, now: i64) -> Snapshot {
    let ctx = record.get("ctx");
    let Some(config) = ctx
        .and_then(|ctx| ctx.get("config"))
        .filter(|config| matches!(config, Value::Obj(_)))
    else {
        return Snapshot::default();
    };
    let (Some(used), Some(end)) = (
        percent(config.get("creditUsagePercent")),
        config
            .get_str("billingPeriodEnd")
            .and_then(vendor_timestamp),
    ) else {
        return Snapshot::default();
    };
    let window_minutes = config
        .get_str("billingPeriodStart")
        .and_then(vendor_timestamp)
        .and_then(|start| end.checked_sub(start))
        .map(|seconds| seconds / 60)
        .filter(|minutes| PERIOD_MINUTES.contains(minutes))
        .and_then(|minutes| u32::try_from(minutes).ok());
    let qualifier = ctx
        .and_then(|ctx| ctx.get_str("subscriptionTier"))
        .filter(|tier| tier_is_safe(tier))
        .map(str::to_owned);
    // A period ae cannot place is not a window: it claims no number.
    let placed = window_minutes.is_some();
    Snapshot {
        row: Some(Row {
            bucket: "credits".to_owned(),
            qualifier,
            window_minutes,
            used_percent: placed.then_some(used),
            resets_at: placed.then_some(end),
            observed_at: Some(stamp),
            status: if placed {
                freshness(Some(stamp), Some(end), now)
            } else {
                Status::Unknown
            },
        }),
        on_demand_enabled: ctx.and_then(|ctx| ctx.get("onDemandEnabled"))
            == Some(&Value::Bool(true)),
    }
}

/// A tier is shown whole or not at all: a value that needs cleaning is dropped.
fn tier_is_safe(tier: &str) -> bool {
    tier.len() <= TIER_MAX
        && tier
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && tier.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_' | b'.' | b'+' | b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::{MSG, Snapshot, parse};
    use crate::quota::{Row, Status};
    use crate::time::Timestamp;

    const NOW: &str = "2026-09-30T12:00:00Z";
    const FRESH: &str = "2026-09-30T11:55:00Z";
    const STALE: &str = "2026-09-30T07:08:44.712Z";

    fn epoch(text: &str) -> i64 {
        Timestamp::parse(text).expect("test timestamp").epoch()
    }

    fn line(stamp: &str, pct: &str) -> String {
        format!(
            "{{\"ts\":\"{stamp}\",\"src\":\"shell\",\"ver\":\"1.0.44\",\"msg\":\"{MSG}\",\"ctx\":{{\"config\":{{\"creditUsagePercent\":{pct},\"billingPeriodStart\":\"2026-09-28T00:00:00Z\",\"billingPeriodEnd\":\"2026-10-05T00:00:00Z\"}},\"onDemandEnabled\":false,\"subscriptionTier\":\"SuperGrok\"}}}}\n"
        )
    }

    fn read(log: &str) -> Snapshot {
        parse(log.as_bytes(), true, epoch(NOW))
    }

    fn row(log: &str) -> Row {
        read(log).row.expect("row")
    }

    fn used_tail(bytes: &[u8], boundary: bool) -> Option<String> {
        parse(bytes, boundary, epoch(NOW)).row?.used_percent
    }

    fn used(log: &str) -> Option<String> {
        used_tail(log.as_bytes(), true)
    }

    #[test]
    fn a_fresh_record_is_the_weekly_credit_window() {
        assert_eq!(MSG, "billing: fetched credits config");
        let fresh = row(&line(FRESH, "3.0"));
        assert_eq!(fresh.bucket, "credits");
        assert_eq!(fresh.qualifier.as_deref(), Some("SuperGrok"));
        assert_eq!(fresh.used_percent.as_deref(), Some("3.0"));
        assert_eq!(fresh.window_minutes, Some(10_080));
        assert_eq!(fresh.resets_at, Some(epoch("2026-10-05T00:00:00Z")));
        assert_eq!(fresh.observed_at, Some(epoch(FRESH)));
        assert_eq!(fresh.status, Status::Fresh);
    }

    #[test]
    fn an_old_record_is_stale_with_its_own_age_and_the_limit_literal_survives() {
        let stale = row(&line(STALE, "100.0"));
        assert_eq!(stale.status, Status::Stale);
        assert_eq!(stale.observed_at, Some(epoch("2026-09-30T07:08:44Z")));
        assert_eq!(stale.used_percent.as_deref(), Some("100.0"));
    }

    #[test]
    fn the_newest_ts_wins_and_a_future_one_is_unknown_with_no_fallback() {
        let older = line(STALE, "4.0");
        assert_eq!(
            used(&format!("{}{older}", line(FRESH, "9.0"))).as_deref(),
            Some("9.0")
        );
        assert_eq!(
            used(&format!("{}{}", line(FRESH, "1.0"), line(FRESH, "2.0"))).as_deref(),
            Some("2.0")
        );
        let future = row(&format!("{older}{}", line("2026-09-30T12:05:00Z", "8.0")));
        assert_eq!(future.status, Status::Unknown);
        assert_eq!(future.observed_at, Some(epoch("2026-09-30T12:05:00Z")));
    }

    #[test]
    fn an_unusable_newest_record_never_falls_back_to_an_older_number() {
        let older = line(STALE, "4.0");
        let mut newest: Vec<String> = ["null", "-1", "1e999", "\"7\"", "[]"]
            .iter()
            .map(|pct| line(FRESH, pct))
            .collect();
        newest.push(line(FRESH, "0").replace("\"creditUsagePercent\":0,", ""));
        newest.push(
            line(FRESH, "3").replace("\"billingPeriodEnd\":\"2026-10-05T00:00:00Z\"", "\"x\":1"),
        );
        // A `config` that is not an object, with the old object kept well-formed.
        newest.push(line(FRESH, "3").replace("\"config\":{", "\"config\":7,\"x\":{"));
        for record in newest {
            assert_eq!(
                read(&format!("{older}{record}")),
                Snapshot::default(),
                "{record}"
            );
        }
        // A record ae cannot order is no candidate at all.
        let unordered = [
            line("yesterday", "3"),
            line(FRESH, "3").replace("\"ts\":\"2026-09-30T11:55:00Z\",", ""),
        ];
        for record in unordered {
            assert_eq!(used(&format!("{older}{record}")).as_deref(), Some("4.0"));
        }
        assert_eq!(read(""), Snapshot::default());
        assert_eq!(read("{\"msg\":\"other\"}\nnot json\n"), Snapshot::default());
    }

    #[test]
    fn a_cap_cut_first_line_is_dropped_and_an_open_last_line_is_not_a_record() {
        let whole = line(FRESH, "5.0");
        let cut = format!("{}{}", &whole[20..], line(STALE, "4.0"));
        assert_eq!(used_tail(cut.as_bytes(), false).as_deref(), Some("4.0"));
        assert_eq!(used_tail(&whole.as_bytes()[20..], false), None);
        let open = format!("{}{}", line(STALE, "4.0"), &line(FRESH, "9.0")[..60]);
        assert_eq!(used(&open).as_deref(), Some("4.0"));
    }

    #[test]
    fn hostile_lines_never_blind_the_scope_and_spacing_never_hides_the_record() {
        let mut log = line(FRESH, "6.0").into_bytes();
        log.extend_from_slice(format!("{{\"msg\":\"{MSG}\",\"ts\":\n").as_bytes());
        log.extend_from_slice(
            b"\xff\xfe binary billing: fetched credits config \xc0\n\xff\xfe other line\n",
        );
        log.extend_from_slice(
            format!("{{\"msg\":\"{MSG}\",\"pad\":\"{}\"}}\n", "x".repeat(70_000)).as_bytes(),
        );
        let quoted = format!(
            "{{\"ts\":\"2026-09-30T11:59:00Z\",\"msg\":\"user pasted {MSG}\",\"ctx\":{{\"config\":{{\"creditUsagePercent\":99}}}}}}\n"
        );
        log.extend_from_slice(quoted.as_bytes());
        assert_eq!(used_tail(&log, true).as_deref(), Some("6.0"));

        let spaced = format!(
            "{{ \"ctx\" : {{ \"config\" : {{ \"billingPeriodEnd\" : \"2026-10-05T00:00:00Z\", \"billingPeriodStart\" : \"2026-09-28T00:00:00Z\", \"creditUsagePercent\" : 12 }} }}, \"msg\" : \"{MSG}\", \"ts\" : \"{FRESH}\" }}\n"
        );
        assert_eq!(used(&spaced).as_deref(), Some("12"));
    }

    #[test]
    fn a_period_that_is_no_window_claims_no_number() {
        for start in [
            "2026-10-04T23:30:00Z",
            "2026-07-01T00:00:00Z",
            "2026-10-06T00:00:00Z",
            "soon",
        ] {
            let odd = row(&line(FRESH, "3").replace("2026-09-28T00:00:00Z", start));
            assert_eq!(odd.status, Status::Unknown, "{start}");
            assert_eq!(
                (odd.used_percent, odd.resets_at, odd.window_minutes),
                (None, None, None)
            );
        }
    }

    #[test]
    fn the_tier_is_shown_whole_or_not_at_all_and_on_demand_only_when_true() {
        for tier in [
            "Super\\u001b[31mGrok",
            "\\u00e9",
            "a-very-long-subscription-tier-name",
            "-lead",
        ] {
            assert_eq!(
                row(&line(FRESH, "3").replace("SuperGrok", tier)).qualifier,
                None,
                "{tier}"
            );
        }
        let plain = row(&line(FRESH, "3").replace("SuperGrok", "Super Grok-Heavy_1.5+"));
        assert_eq!(plain.qualifier.as_deref(), Some("Super Grok-Heavy_1.5+"));
        for (value, expected) in [("true", true), ("false", false), ("\"true\"", false)] {
            let record = line(FRESH, "3").replace(
                "\"onDemandEnabled\":false",
                &format!("\"onDemandEnabled\":{value}"),
            );
            assert_eq!(read(&record).on_demand_enabled, expected, "{value}");
        }
    }

    #[test]
    fn ten_thousand_hostile_inputs_never_panic() {
        let mut state = 0x6a09_e667_f3bc_c909_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let seed = line(FRESH, "3.0").into_bytes();
        for case in 0..10_000_usize {
            let len = usize::try_from(next() & 0xff).expect("bounded length");
            let mut bytes: Vec<u8> = (0..len).map(|_| next().to_le_bytes()[0]).collect();
            if case % 2 == 0 {
                let at = usize::try_from(next()).unwrap_or(0) % (seed.len() + 1);
                let mut shaped = seed[..at].to_vec();
                shaped.extend_from_slice(MSG.as_bytes());
                shaped.append(&mut bytes);
                bytes = shaped;
            }
            let clock = i64::try_from(next() >> 2).expect("bounded clock");
            let _ = parse(&bytes, case % 3 != 0, clock);
        }
    }
}
