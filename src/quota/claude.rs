//! Parser for Claude Code's local `cachedUsageUtilization` object.

use crate::json::{self, Value};

use super::{ParseError, Row, Status, epoch, freshness, percent, vendor_timestamp};

/// One parsed Claude cache observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Account identity used only to detect cache switches; never display it.
    pub account_uuid: Option<String>,
    /// The cache's own fetch time.
    pub observed_at: Option<i64>,
    /// Every vendor limit bucket, without collapsing scoped limits.
    pub rows: Vec<Row>,
}

/// Parse a complete Claude settings file.
///
/// `Ok(None)` means that the file is valid JSON but carries no quota cache.
///
/// # Errors
///
/// Returns [`ParseError`] for invalid UTF-8, malformed JSON, or an incompatible
/// `cachedUsageUtilization` container.
pub fn parse(bytes: &[u8], now: i64) -> Result<Option<Snapshot>, ParseError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ParseError::Utf8)?;
    let root = json::parse(text).map_err(|_| ParseError::Json)?;
    let Some(cache) = root.get("cachedUsageUtilization") else {
        return Ok(None);
    };
    if !matches!(cache, Value::Obj(_)) {
        return Err(ParseError::Shape);
    }

    let observed_at = epoch(cache.get("fetchedAtMs")).map(|millis| millis.div_euclid(1_000));
    let account_uuid = cache.get_str("accountUuid").map(str::to_owned);
    let utilization = cache.get("utilization").unwrap_or(cache);
    if !matches!(utilization, Value::Obj(_)) {
        return Err(ParseError::Shape);
    }

    let mut rows = match utilization.get("limits") {
        Some(Value::Arr(limits)) => limits
            .iter()
            .filter_map(|limit| limit_row(limit, observed_at, now))
            .collect(),
        Some(Value::Null) | None => legacy_rows(utilization, observed_at, now),
        Some(_) => return Err(ParseError::Shape),
    };
    rows.sort_by(|left, right| {
        left.bucket
            .cmp(&right.bucket)
            .then_with(|| left.qualifier.cmp(&right.qualifier))
    });

    Ok(Some(Snapshot {
        account_uuid,
        observed_at,
        rows,
    }))
}

fn limit_row(limit: &Value, observed_at: Option<i64>, now: i64) -> Option<Row> {
    let kind = limit.get_str("kind")?.to_owned();
    let qualifier = limit
        .get("scope")
        .and_then(|scope| scope.get("model"))
        .and_then(|model| model.get_str("display_name"))
        .map(str::to_owned);
    let window_minutes = match kind.as_str() {
        "session" => Some(300),
        "weekly_all" | "weekly_scoped" => Some(10_080),
        _ => None,
    };
    let used_percent = percent(limit.get("percent"));
    let resets_at = limit.get_str("resets_at").and_then(vendor_timestamp);
    let applicable = window_minutes.is_some()
        && used_percent.is_some()
        && (kind != "weekly_scoped" || qualifier.is_some());
    let status = if applicable {
        freshness(observed_at, resets_at, now)
    } else {
        Status::Unknown
    };
    Some(Row {
        bucket: kind,
        qualifier,
        window_minutes,
        used_percent,
        resets_at,
        observed_at,
        status,
    })
}

fn legacy_rows(utilization: &Value, observed_at: Option<i64>, now: i64) -> Vec<Row> {
    [
        ("five_hour", "session", 300),
        ("seven_day", "weekly_all", 10_080),
    ]
    .into_iter()
    .filter_map(|(field, bucket, window_minutes)| {
        let value = utilization.get(field)?;
        if matches!(value, Value::Null) {
            return None;
        }
        let used_percent = percent(value.get("utilization"));
        let resets_at = value.get_str("resets_at").and_then(vendor_timestamp);
        let status = if used_percent.is_some() {
            freshness(observed_at, resets_at, now)
        } else {
            Status::Unknown
        };
        Some(Row {
            bucket: bucket.to_owned(),
            qualifier: None,
            window_minutes: Some(window_minutes),
            used_percent,
            resets_at,
            observed_at,
            status,
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::quota::Status;
    use crate::time::Timestamp;

    const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/quota/claude-cache.json");

    fn epoch(text: &str) -> i64 {
        Timestamp::parse(text).expect("fixture timestamp").epoch()
    }

    #[test]
    fn the_real_redacted_cache_preserves_every_bucket() {
        let snapshot = parse(FIXTURE, epoch("2026-09-08T09:10:00Z"))
            .expect("fixture parses")
            .expect("fixture carries cache");
        assert_eq!(snapshot.account_uuid.as_deref(), Some("[REDACTED]"));
        assert_eq!(snapshot.rows.len(), 3);
        assert_eq!(snapshot.rows[0].bucket, "session");
        assert_eq!(snapshot.rows[0].window_minutes, Some(300));
        assert_eq!(snapshot.rows[1].bucket, "weekly_all");
        assert_eq!(snapshot.rows[1].used_percent.as_deref(), Some("9"));
        assert_eq!(snapshot.rows[2].bucket, "weekly_scoped");
        assert_eq!(snapshot.rows[2].qualifier.as_deref(), Some("Fable"));
        assert_eq!(snapshot.rows[2].used_percent.as_deref(), Some("100"));
        assert!(snapshot.rows.iter().all(|row| row.status == Status::Fresh));
    }

    #[test]
    fn missing_or_expired_clocks_make_numbers_unknown() {
        let missing = br#"{"cachedUsageUtilization":{"fetchedAtMs":"bad","utilization":{"limits":[{"kind":"session","percent":66,"resets_at":null}]}}}"#;
        let parsed = parse(missing, 10).expect("valid document").expect("cache");
        assert_eq!(parsed.rows[0].status, Status::Unknown);

        let at_reset = parse(FIXTURE, epoch("2026-09-08T11:49:59Z"))
            .expect("fixture parses")
            .expect("cache");
        assert_eq!(at_reset.rows[0].status, Status::Unknown);
    }

    #[test]
    fn a_missing_cache_is_distinct_from_a_broken_cache() {
        assert_eq!(parse(br#"{"theme":"dark"}"#, 0), Ok(None));
        assert!(parse(br#"{"cachedUsageUtilization":[]}"#, 0).is_err());
        assert!(parse(b"{", 0).is_err());
    }

    #[test]
    fn ten_thousand_hostile_inputs_never_panic() {
        let mut state = 0x6a09_e667_f3bc_c909_u64;
        for case in 0..10_000_usize {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = usize::try_from(state & 0xff).expect("bounded length");
            let mut bytes = Vec::with_capacity(len);
            for offset in 0..len {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                bytes.push(state.to_le_bytes()[offset & 7]);
            }
            let _ = parse(&bytes, i64::try_from(case).expect("bounded case"));
        }
    }
}
