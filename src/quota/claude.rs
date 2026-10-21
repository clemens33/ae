//! Parser for Claude Code's local `cachedUsageUtilization` object and the
//! plan label its `oauthAccount` block reports.

use crate::json::{self, Value};

use super::{ParseError, Row, Status, epoch, freshness, percent, vendor_timestamp};

/// One parsed Claude cache observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The cache's own fetch time.
    pub observed_at: Option<i64>,
    /// Every vendor limit bucket, without collapsing scoped limits.
    pub rows: Vec<Row>,
}

/// What a settings file yields: the quota cache and the plan label, each
/// present on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// The quota cache, or `None` for a valid file that carries none.
    pub snapshot: Option<Snapshot>,
    /// The plan the account block reports, or `None` when it names none.
    pub plan: Option<Plan>,
}

/// The account's own plan words: four enum tokens and the stamp of the report.
///
/// Exactly these five keys of `oauthAccount` are retained and projected for the
/// plan display; no other field of that block is kept or shown. The account-switch
/// check in `parse` also reads `oauthAccount.accountUuid`, only to compare it with
/// the cache's, and retains or renders no identity. The values are names the vendor
/// reports, never a capacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// `organizationType`, such as `claude_max`.
    pub organization_type: Option<String>,
    /// `organizationRateLimitTier`, such as `default_claude_max_20x`.
    pub organization_rate_limit_tier: Option<String>,
    /// `userRateLimitTier`, which a team seat may set apart from the organization's.
    pub user_rate_limit_tier: Option<String>,
    /// `seatTier`, such as `team_tier_1`.
    pub seat_tier: Option<String>,
    /// `profileFetchedAt` in epoch seconds: when the client last fetched the profile.
    pub fetched_at: Option<i64>,
}

/// The longest token a plan field may be.
const TOKEN_MAX: usize = 40;

/// Whether `text` is a plain token: `[A-Za-z0-9][A-Za-z0-9._-]{0,39}`.
fn is_token(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() <= TOKEN_MAX
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// One plan field: the string under `key` when it is a plain token.
fn token(account: &Value, key: &str) -> Option<String> {
    account
        .get_str(key)
        .filter(|text| is_token(text))
        .map(str::to_owned)
}

/// The plan an `oauthAccount` block reports. A field that fails its gate is
/// dropped alone; no surviving identity field means no plan, and a stamp alone
/// names nothing.
fn plan_of(root: &Value) -> Option<Plan> {
    let account = root.get("oauthAccount")?;
    let plan = Plan {
        organization_type: token(account, "organizationType"),
        organization_rate_limit_tier: token(account, "organizationRateLimitTier"),
        user_rate_limit_tier: token(account, "userRateLimitTier"),
        seat_tier: token(account, "seatTier"),
        fetched_at: epoch(account.get("profileFetchedAt"))
            .filter(|millis| *millis > 0)
            .map(|millis| millis.div_euclid(1_000)),
    };
    (plan.organization_type.is_some()
        || plan.organization_rate_limit_tier.is_some()
        || plan.user_rate_limit_tier.is_some()
        || plan.seat_tier.is_some())
    .then_some(plan)
}

/// A rate-limit tier in words: `default_claude_max_20x` is `max 20x`; any other
/// spelling stays as reported.
fn tier_words(tier: &str) -> String {
    match tier.strip_prefix("default_claude_") {
        Some(rest) if !rest.is_empty() => rest.replace('_', " "),
        _ => tier.to_owned(),
    }
}

impl Plan {
    /// The label, such as `claude max 20x`, or `None` when no field names one.
    ///
    /// The plan type is the name, unless the organization tier (then the user
    /// tier) repeats it and says more, which then names the plan. Whatever
    /// else the account reports is drawn beside the name, in `org`, `seat`,
    /// `user` order, so a disagreement is shown and never settled here.
    #[must_use]
    pub fn label(&self) -> Option<String> {
        let kind = self.organization_type.as_deref().map(|kind| {
            match kind.strip_prefix("claude_") {
                Some(rest) if !rest.is_empty() => rest,
                _ => kind,
            }
            .to_owned()
        });
        let org = self.organization_rate_limit_tier.as_deref().map(tier_words);
        let user = self.user_rate_limit_tier.as_deref().map(tier_words);
        let refines = |tier: &String, name: &String| {
            tier == name
                || tier
                    .strip_prefix(name.as_str())
                    .is_some_and(|rest| rest.starts_with(' '))
        };
        let (name, supplier) = match &kind {
            Some(name) => match (&org, &user) {
                (Some(tier), _) if refines(tier, name) => (Some(tier.clone()), Some(Field::Org)),
                (_, Some(tier)) if refines(tier, name) => (Some(tier.clone()), Some(Field::User)),
                _ => (kind.clone(), None),
            },
            None => match (&org, &user) {
                (Some(tier), _) => (Some(tier.clone()), Some(Field::Org)),
                (_, Some(tier)) => (Some(tier.clone()), Some(Field::User)),
                _ => (None, None),
            },
        };
        let extras: Vec<String> = [
            (Field::Org, "org", org.as_deref()),
            (Field::Seat, "seat", self.seat_tier.as_deref()),
            (Field::User, "user", user.as_deref()),
        ]
        .into_iter()
        .filter_map(|(field, word, value)| {
            let value = value?;
            (supplier != Some(field) && name.as_deref() != Some(value))
                .then(|| format!("{word} {value}"))
        })
        .collect();
        if name.is_none() && extras.is_empty() {
            return None;
        }
        let mut label = "claude".to_owned();
        if let Some(name) = &name {
            label.push(' ');
            label.push_str(name);
        }
        if !extras.is_empty() {
            label.push_str(" (");
            label.push_str(&extras.join(", "));
            label.push(')');
        }
        Some(label)
    }
}

/// The field a plan label drew its name from, so it is not drawn twice.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Org,
    Seat,
    User,
}

/// Parse a complete Claude settings file.
///
/// `snapshot` is `None` when the file is valid JSON but carries no quota
/// cache; the plan is read from the account block whether or not a cache is
/// there.
///
/// # Errors
///
/// Returns [`ParseError`] for invalid UTF-8, malformed JSON, or an incompatible
/// `cachedUsageUtilization` container.
pub fn parse(bytes: &[u8], now: i64) -> Result<Parsed, ParseError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ParseError::Utf8)?;
    let root = json::parse(text).map_err(|_| ParseError::Json)?;
    let plan = plan_of(&root);
    let Some(cache) = root.get("cachedUsageUtilization") else {
        return Ok(Parsed {
            snapshot: None,
            plan,
        });
    };
    if !matches!(cache, Value::Obj(_)) {
        return Err(ParseError::Shape);
    }

    let observed_at = epoch(cache.get("fetchedAtMs")).map(|millis| millis.div_euclid(1_000));
    let account_switched = matches!(
        (
            root.get("oauthAccount")
                .and_then(|account| account.get_str("accountUuid")),
            cache.get_str("accountUuid"),
        ),
        (Some(current), Some(cached)) if current != cached
    );
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
    if account_switched {
        for row in &mut rows {
            row.status = Status::Unknown;
        }
    }

    Ok(Parsed {
        snapshot: Some(Snapshot { observed_at, rows }),
        plan,
    })
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
    use super::{Plan, parse};
    use crate::quota::Status;
    use crate::time::Timestamp;

    const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/quota/claude-cache.json");
    const PLAN_FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/quota/claude-plan.json");

    /// The five named keys, and nothing else, as a synthetic account block.
    const NAMED: &str = "\
        \"organizationType\":\"claude_team\",\
        \"organizationRateLimitTier\":\"default_claude_max_20x\",\
        \"userRateLimitTier\":\"default_claude_max_5x\",\
        \"seatTier\":\"team_tier_1\",\
        \"profileFetchedAt\":1788855960000";

    fn epoch(text: &str) -> i64 {
        Timestamp::parse(text).expect("fixture timestamp").epoch()
    }

    fn plan(
        organization_type: Option<&str>,
        organization_rate_limit_tier: Option<&str>,
        user_rate_limit_tier: Option<&str>,
        seat_tier: Option<&str>,
    ) -> Plan {
        Plan {
            organization_type: organization_type.map(str::to_owned),
            organization_rate_limit_tier: organization_rate_limit_tier.map(str::to_owned),
            user_rate_limit_tier: user_rate_limit_tier.map(str::to_owned),
            seat_tier: seat_tier.map(str::to_owned),
            fetched_at: None,
        }
    }

    #[test]
    fn the_real_redacted_cache_preserves_every_bucket() {
        let parsed = parse(FIXTURE, epoch("2026-09-08T09:10:00Z")).expect("fixture parses");
        let snapshot = parsed.snapshot.expect("fixture carries cache");
        assert!(
            parsed.plan.is_none(),
            "a cache with no account block names no plan"
        );
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
        let parsed = parse(missing, 10)
            .expect("valid document")
            .snapshot
            .expect("cache");
        assert_eq!(parsed.rows[0].status, Status::Unknown);

        let at_reset = parse(FIXTURE, epoch("2026-09-08T11:49:59Z"))
            .expect("fixture parses")
            .snapshot
            .expect("cache");
        assert_eq!(at_reset.rows[0].status, Status::Unknown);
    }

    #[test]
    fn a_cached_account_switch_invalidates_numbers_without_retaining_ids() {
        let mismatch = br#"{
            "oauthAccount":{"accountUuid":"current"},
            "cachedUsageUtilization":{
                "accountUuid":"cached",
                "fetchedAtMs":10000,
                "utilization":{"limits":[{
                    "kind":"session",
                    "percent":66,
                    "resets_at":"1970-01-01T01:00:00Z"
                }]}
            }
        }"#;
        let parsed = parse(mismatch, 11)
            .expect("valid document")
            .snapshot
            .expect("cache");
        assert_eq!(parsed.rows[0].status, Status::Unknown);

        for missing in [
            br#"{"cachedUsageUtilization":{"accountUuid":"cached","fetchedAtMs":10000,"utilization":{"limits":[{"kind":"session","percent":66,"resets_at":"1970-01-01T01:00:00Z"}]}}}"#.as_slice(),
            br#"{"oauthAccount":{"accountUuid":"current"},"cachedUsageUtilization":{"fetchedAtMs":10000,"utilization":{"limits":[{"kind":"session","percent":66,"resets_at":"1970-01-01T01:00:00Z"}]}}}"#.as_slice(),
        ] {
            let parsed = parse(missing, 11).expect("valid document").snapshot.expect("cache");
            assert_eq!(parsed.rows[0].status, Status::Fresh);
        }
    }

    #[test]
    fn a_missing_cache_is_distinct_from_a_broken_cache() {
        let account_only = parse(br#"{"theme":"dark"}"#, 0).expect("valid document");
        assert!(account_only.snapshot.is_none());
        assert!(account_only.plan.is_none());
        assert!(parse(br#"{"cachedUsageUtilization":[]}"#, 0).is_err());
        assert!(parse(b"{", 0).is_err());
    }

    #[test]
    fn the_five_named_keys_become_one_plan_beside_the_ordinary_rows() {
        let document = format!(
            "{{\"oauthAccount\":{{{NAMED}}},\"cachedUsageUtilization\":{{\"fetchedAtMs\":1788858413691,\"utilization\":{{\"limits\":[{{\"kind\":\"session\",\"percent\":66,\"resets_at\":\"2026-09-08T11:49:59Z\"}}]}}}}}}"
        );
        let parsed = parse(document.as_bytes(), epoch("2026-09-08T09:10:00Z")).expect("valid");
        let plan = parsed.plan.expect("a reported plan");
        assert_eq!(plan.organization_type.as_deref(), Some("claude_team"));
        assert_eq!(
            plan.organization_rate_limit_tier.as_deref(),
            Some("default_claude_max_20x")
        );
        assert_eq!(
            plan.user_rate_limit_tier.as_deref(),
            Some("default_claude_max_5x")
        );
        assert_eq!(plan.seat_tier.as_deref(), Some("team_tier_1"));
        assert_eq!(plan.fetched_at, Some(1_788_855_960));
        assert_eq!(parsed.snapshot.expect("cache").rows.len(), 1);
        assert_eq!(
            plan.label().as_deref(),
            Some("claude team (org max 20x, seat team_tier_1, user max 5x)")
        );
    }

    #[test]
    fn the_fixture_plan_reads_the_same_way_the_label_draws_it() {
        let parsed = parse(PLAN_FIXTURE, epoch("2026-09-08T09:10:00Z")).expect("fixture parses");
        let plan = parsed.plan.expect("the fixture reports a plan");
        assert_eq!(plan.fetched_at, Some(1_788_855_960));
        assert_eq!(plan.label().as_deref(), Some("claude max 20x"));
        assert_eq!(parsed.snapshot.expect("cache").rows.len(), 3);
    }

    #[test]
    fn a_plan_survives_a_missing_cache_and_never_a_broken_file() {
        let account_only = br#"{"oauthAccount":{"organizationType":"claude_max"},"theme":"dark"}"#;
        let parsed = parse(account_only, 0).expect("valid document");
        assert!(parsed.snapshot.is_none(), "no cache, so no rows to judge");
        assert_eq!(
            parsed
                .plan
                .expect("the plan does not need a cache")
                .label()
                .as_deref(),
            Some("claude max")
        );

        let broken = format!("{{\"oauthAccount\":{{{NAMED}}},\"cachedUsageUtilization\":[]}}");
        assert!(
            parse(broken.as_bytes(), 0).is_err(),
            "a damaged cache is not a quiet absence"
        );
    }

    #[test]
    fn a_field_that_is_not_a_plain_bounded_token_is_dropped_alone() {
        let long = "a".repeat(41);
        let cases = [
            "\"team tier\"".to_owned(),
            "\"team\\ntier\"".to_owned(),
            "\"$(reboot)\"".to_owned(),
            "\"team:tier\"".to_owned(),
            "\"\"".to_owned(),
            format!("\"{long}\""),
            "5".to_owned(),
            "null".to_owned(),
            "[\"team\"]".to_owned(),
        ];
        for value in cases {
            let document = format!(
                "{{\"oauthAccount\":{{\"organizationType\":{value},\"seatTier\":\"team_tier_1\"}}}}"
            );
            let parsed = parse(document.as_bytes(), 0).expect("valid document");
            let plan = parsed
                .plan
                .unwrap_or_else(|| panic!("seatTier alone still names a plan: {value}"));
            assert_eq!(plan.organization_type, None, "{value}");
            assert_eq!(plan.seat_tier.as_deref(), Some("team_tier_1"), "{value}");
        }

        let nothing = br#"{"oauthAccount":{"organizationType":5,"seatTier":null}}"#;
        assert!(
            parse(nothing, 0).expect("valid document").plan.is_none(),
            "no field survives, so nothing is reported"
        );
        let stamp_only = br#"{"oauthAccount":{"profileFetchedAt":1788855960000}}"#;
        assert!(
            parse(stamp_only, 0).expect("valid document").plan.is_none(),
            "a stamp alone names no plan"
        );
    }

    #[test]
    fn the_reported_stamp_is_read_only_as_a_positive_json_number() {
        for (value, expected) in [
            (r#""1788855960000""#, None),
            ("1788855960000.5", None),
            ("true", None),
            ("null", None),
            ("0", None),
            ("-1000", None),
            ("1788855960999", Some(1_788_855_960)),
            ("1788855960000", Some(1_788_855_960)),
        ] {
            let document = format!(
                "{{\"oauthAccount\":{{\"organizationType\":\"claude_max\",\"profileFetchedAt\":{value}}}}}"
            );
            let parsed = parse(document.as_bytes(), 0).expect("valid document");
            let plan = parsed.plan.expect("organizationType names a plan");
            assert_eq!(plan.fetched_at, expected, "{value}");
        }
    }

    #[test]
    fn no_sibling_of_the_five_named_keys_ever_reaches_the_plan() {
        const UUID: &str = "SENTINEL-ACCOUNT-UUID-3f9a";
        const EMAIL: &str = "sentinel@example.invalid";
        const ORG: &str = "SENTINEL ORGANIZATION NAME";
        let document = format!(
            "{{\"oauthAccount\":{{{NAMED},\
             \"accountUuid\":\"{UUID}\",\
             \"emailAddress\":\"{EMAIL}\",\
             \"organizationName\":\"{ORG}\",\
             \"billingType\":\"stripe_subscription\",\
             \"hasExtraUsageEnabled\":true}}}}"
        );
        let plan = parse(document.as_bytes(), 0)
            .expect("valid document")
            .plan
            .expect("the five named keys");
        let seen = format!("{plan:?}");
        for sentinel in [
            UUID,
            EMAIL,
            ORG,
            "stripe_subscription",
            "hasExtraUsageEnabled",
            "accountUuid",
            "emailAddress",
            "organizationName",
        ] {
            assert!(!seen.contains(sentinel), "{sentinel} reached {seen}");
        }
        assert_eq!(
            plan.label().as_deref(),
            Some("claude team (org max 20x, seat team_tier_1, user max 5x)")
        );
    }

    #[test]
    fn the_label_names_the_reported_plan_and_every_disagreement() {
        for (fields, expected) in [
            (
                plan(
                    Some("claude_max"),
                    Some("default_claude_max_20x"),
                    None,
                    None,
                ),
                Some("claude max 20x"),
            ),
            (
                plan(Some("claude_team"), None, None, Some("team_tier_1")),
                Some("claude team (seat team_tier_1)"),
            ),
            (
                plan(None, Some("default_claude_max_5x"), None, None),
                Some("claude max 5x"),
            ),
            (
                plan(None, None, Some("default_claude_max_5x"), None),
                Some("claude max 5x"),
            ),
            (
                plan(Some("claude_max"), None, None, None),
                Some("claude max"),
            ),
            (
                plan(
                    Some("claude_max"),
                    Some("default_claude_max_20x"),
                    Some("default_claude_max_20x"),
                    None,
                ),
                Some("claude max 20x"),
            ),
            (
                plan(
                    Some("claude_team"),
                    Some("default_raven"),
                    Some("default_claude_max_5x"),
                    Some("team_tier_1"),
                ),
                Some("claude team (org default_raven, seat team_tier_1, user max 5x)"),
            ),
            (
                plan(None, None, None, Some("team_tier_1")),
                Some("claude (seat team_tier_1)"),
            ),
            (plan(None, None, None, None), None),
        ] {
            assert_eq!(fields.label().as_deref(), expected, "{fields:?}");
        }
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
