//! Parser for bounded tails of Codex rollout JSONL files.

use crate::json::{self, Value};

use super::{
    Account, CREDIT_BALANCE_MAX, Credits, ParseError, Row, Status, epoch, freshness, minutes,
    percent, vendor_timestamp,
};

/// One parsed Codex rollout observation: its windows and its account facts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Every vendor limit bucket and window the tail reported.
    pub rows: Vec<Row>,
    /// Account-wide credit and spend-control state, when reported.
    pub account: Account,
}

/// Parse complete records from a bounded Codex rollout tail.
///
/// Set `starts_at_record_boundary` to false when the byte cap cut the first
/// record; bytes through its newline are then ignored. Bytes after the final
/// newline are always an in-progress record and are ignored.
///
/// # Errors
///
/// Returns [`ParseError`] when any complete record is invalid UTF-8 or JSON.
pub fn parse(
    bytes: &[u8],
    starts_at_record_boundary: bool,
    now: i64,
) -> Result<Snapshot, ParseError> {
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

    let mut rows = Vec::new();
    let mut account = Account::default();
    for raw in bytes
        .get(from..complete)
        .unwrap_or_default()
        .split(|byte| *byte == b'\n')
    {
        if raw.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let text = std::str::from_utf8(raw).map_err(|_| ParseError::Utf8)?;
        let record = json::parse(text).map_err(|_| ParseError::Json)?;
        let Some(limits) = rate_limits(&record) else {
            continue;
        };
        account = account_state(limits);
        let Some(next) = quota_rows(limits, &record, now) else {
            continue;
        };
        let bucket = &next[0].bucket;
        rows.retain(|row: &Row| row.bucket != *bucket);
        rows.extend(next);
    }
    rows.sort_by(|left, right| {
        left.bucket
            .cmp(&right.bucket)
            .then_with(|| left.window_minutes.cmp(&right.window_minutes))
    });
    Ok(Snapshot { rows, account })
}

fn rate_limits(record: &Value) -> Option<&Value> {
    if record.get_str("type") != Some("event_msg") {
        return None;
    }
    let payload = record.get("payload")?;
    if payload.get_str("type") != Some("token_count") {
        return None;
    }
    payload.get("rate_limits")
}

/// Read the account facts the client reports beside its windows.
///
/// The balance is a vendor literal: it is bounded here, at the hostile-input
/// boundary, rather than at every place that displays it.
fn account_state(limits: &Value) -> Account {
    let credits = limits
        .get("credits")
        .map_or(Credits::Unreported, |credits| {
            if credits.get("unlimited") == Some(&Value::Bool(true)) {
                Credits::Unlimited
            } else if credits.get("has_credits") == Some(&Value::Bool(true)) {
                match credits.get_str("balance") {
                    Some(balance) => {
                        Credits::Available(balance.chars().take(CREDIT_BALANCE_MAX).collect())
                    }
                    None => Credits::Unreported,
                }
            } else if credits.get("has_credits") == Some(&Value::Bool(false)) {
                Credits::Exhausted
            } else {
                Credits::Unreported
            }
        });
    let spend_control_reached = match limits.get("spend_control_reached") {
        Some(Value::Bool(reached)) => Some(*reached),
        _ => None,
    };
    Account {
        credits,
        spend_control_reached,
    }
}

fn quota_rows(limits: &Value, record: &Value, now: i64) -> Option<Vec<Row>> {
    let bucket = limits.get_str("limit_id")?.to_owned();
    let qualifier = limits.get_str("plan_type").map(str::to_owned);
    let observed_at = record.get_str("timestamp").and_then(vendor_timestamp);
    let mut rows: Vec<Row> = [limits.get("primary"), limits.get("secondary")]
        .into_iter()
        .flatten()
        .filter(|value| !matches!(value, Value::Null))
        .map(|window| window_row(&bucket, qualifier.as_deref(), observed_at, window, now))
        .collect();
    if rows.is_empty() {
        rows.push(Row {
            bucket,
            qualifier,
            window_minutes: None,
            used_percent: None,
            resets_at: None,
            observed_at,
            status: Status::Unknown,
        });
    }
    Some(rows)
}

fn window_row(
    bucket: &str,
    qualifier: Option<&str>,
    observed_at: Option<i64>,
    window: &Value,
    now: i64,
) -> Row {
    let window_minutes = minutes(window.get("window_minutes"));
    let used_percent = percent(window.get("used_percent"));
    let resets_at = epoch(window.get("resets_at"));
    let status = if window_minutes.is_some() && used_percent.is_some() {
        freshness(observed_at, resets_at, now)
    } else {
        Status::Unknown
    };
    Row {
        bucket: bucket.to_owned(),
        qualifier: qualifier.map(str::to_owned),
        window_minutes,
        used_percent,
        resets_at,
        observed_at,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::quota::{Account, Credits, ParseError, Status};
    use crate::time::Timestamp;

    const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/quota/codex-rollout.jsonl");

    fn epoch(text: &str) -> i64 {
        Timestamp::parse(text).expect("fixture timestamp").epoch()
    }

    #[test]
    fn interleaved_buckets_keep_the_newest_observation_per_bucket() {
        let rows = parse(FIXTURE, true, epoch("2026-09-08T09:10:00Z"))
            .expect("fixture parses")
            .rows;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].bucket, "codex");
        assert_eq!(rows[0].window_minutes, Some(10_080));
        assert_eq!(rows[1].bucket, "codex_bengalfox");
        assert_eq!(rows[1].window_minutes, Some(300));
        assert_eq!(rows[1].used_percent.as_deref(), Some("3.5"));
        assert_eq!(rows[2].window_minutes, Some(10_080));
        assert_eq!(rows[2].used_percent.as_deref(), Some("12.0"));
        assert!(rows.iter().all(|row| row.status == Status::Fresh));
    }

    #[test]
    fn an_incomplete_final_record_is_skipped_without_refreshing_age() {
        let mut bytes = FIXTURE.to_vec();
        bytes.extend_from_slice(br#"{"timestamp":"2099-01-01T00:00:00Z""#);
        let rows = parse(&bytes, true, epoch("2026-09-08T09:10:00Z"))
            .expect("partial tail is not a record")
            .rows;
        let bengal = rows
            .iter()
            .find(|row| row.bucket == "codex_bengalfox")
            .expect("bucket remains");
        assert_eq!(bengal.observed_at, Some(epoch("2026-09-08T09:02:00Z")));
    }

    #[test]
    fn a_malformed_complete_record_is_a_read_error() {
        let mut bytes = FIXTURE.to_vec();
        bytes.extend_from_slice(b"{broken}\n");
        assert_eq!(
            parse(&bytes, true, epoch("2026-09-08T09:10:00Z")),
            Err(ParseError::Json)
        );
    }

    #[test]
    fn a_cap_cut_first_record_is_not_treated_as_malformed() {
        let mut bytes = b"middle of json}\n".to_vec();
        bytes.extend_from_slice(FIXTURE);
        assert_eq!(
            parse(&bytes, false, epoch("2026-09-08T09:10:00Z")),
            parse(FIXTURE, true, epoch("2026-09-08T09:10:00Z"))
        );
    }

    #[test]
    fn credits_and_spend_control_come_from_the_newest_record_that_carries_them() {
        let record = |credits: &str, spend: &str| {
            format!(
                r#"{{"timestamp":"2026-09-08T09:00:00Z","type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"limit_id":"codex","plan_type":"pro","primary":{{"used_percent":95.0,"window_minutes":10080,"resets_at":1789445400}},"secondary":null,"credits":{credits},"spend_control_reached":{spend}}}}}}}
"#
            )
        };
        let parsed =
            |text: String| parse(text.as_bytes(), true, 1_788_858_600).expect("record parses");

        let live = parsed(record(
            r#"{"has_credits":false,"unlimited":false,"balance":"0"}"#,
            "null",
        ));
        assert_eq!(live.account.credits, Credits::Exhausted);
        assert_eq!(live.account.spend_control_reached, None);
        assert_eq!(live.rows.len(), 1, "windows are unchanged by account facts");

        let unlimited = parsed(record(
            r#"{"has_credits":true,"unlimited":true,"balance":"0"}"#,
            "false",
        ));
        assert_eq!(unlimited.account.credits, Credits::Unlimited);
        assert_eq!(unlimited.account.spend_control_reached, Some(false));

        let balance = parsed(record(
            r#"{"has_credits":true,"unlimited":false,"balance":"12.50"}"#,
            "true",
        ));
        assert_eq!(
            balance.account.credits,
            Credits::Available("12.50".to_owned())
        );
        assert_eq!(balance.account.spend_control_reached, Some(true));

        let hostile = parsed(record(
            &format!(
                r#"{{"has_credits":true,"unlimited":false,"balance":"{}"}}"#,
                "9".repeat(300)
            ),
            "true",
        ));
        match &hostile.account.credits {
            Credits::Available(shown) => assert!(
                shown.chars().count() <= 9,
                "a hostile balance is bounded at the parser: {shown}"
            ),
            other => panic!("a reported balance stays a balance: {other:?}"),
        }

        let absent = parse(FIXTURE, true, epoch("2026-09-08T09:10:00Z")).expect("fixture parses");
        assert_eq!(
            absent.account,
            Account::default(),
            "a client that reports no credit state gets none invented"
        );
    }

    #[test]
    fn missing_secondary_and_expired_windows_are_explicit() {
        let line = concat!(
            r#"{"timestamp":"2026-09-08T09:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","plan_type":"pro","primary":{"used_percent":7.0,"window_minutes":300,"resets_at":1788858000},"secondary":null}}}"#,
            "\n"
        );
        let rows = parse(line.as_bytes(), true, 1_788_858_000)
            .expect("record parses")
            .rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, Status::Unknown);
    }

    #[test]
    fn ten_thousand_hostile_inputs_never_panic() {
        let mut state = 0xbb67_ae85_84ca_a73b_u64;
        for case in 0..10_000_usize {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = usize::try_from(state & 0xff).expect("bounded length");
            let mut bytes = Vec::with_capacity(len + 1);
            for offset in 0..len {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                bytes.push(state.to_le_bytes()[offset & 7]);
            }
            bytes.push(b'\n');
            let _ = parse(
                &bytes,
                state & 1 == 0,
                i64::try_from(case).expect("bounded case"),
            );
        }
    }
}
