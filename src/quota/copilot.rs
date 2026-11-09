//! Pure parser for GitHub's undocumented `copilot_internal/user` answer.
//! Unknown fields carry no verdict; missing or invalid measurements yield notes.

use crate::json::{self, Value};
use crate::time::Timestamp;

use super::{Row, Status, freshness, go, percent};

/// The measured premium allowance and the account's display-only facts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    /// One monthly row, with no claimed fixed window length.
    pub rows: Vec<Row>,
    /// Fixed text only: no vendor messages or unknown plan names.
    pub notes: Vec<String>,
    /// A closed plan name, or none when unknown.
    pub plan: Option<String>,
    /// Whether premium interactions are explicitly unlimited.
    pub unlimited: bool,
    pub(crate) valid: bool,
}

/// Parse an answer observed at `observed_at`, judged at `now`.
#[must_use]
pub fn parse(bytes: &[u8], observed_at: i64, now: i64) -> Parsed {
    let root = std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| json::parse(text).ok());
    let mut parsed = Parsed::default();
    let mut row = Row {
        bucket: "premium".to_owned(),
        qualifier: None,
        window_minutes: None,
        used_percent: None,
        resets_at: None,
        observed_at: Some(observed_at),
        status: Status::ReadError,
    };
    if let Some(root @ Value::Obj(_)) = root {
        parsed.valid = true;
        parsed.plan = root
            .get_str("copilot_plan")
            .filter(|plan| matches!(*plan, "individual" | "business" | "enterprise"))
            .map(str::to_owned);
        row.resets_at = reset(&root);
        if let Some(premium @ Value::Obj(_)) = root
            .get("quota_snapshots")
            .and_then(|snapshots| snapshots.get("premium_interactions"))
        {
            parsed.unlimited = premium.get("unlimited") == Some(&Value::Bool(true));
            row.used_percent = used(premium);
            if row.used_percent.is_none() && !parsed.unlimited {
                parsed
                    .notes
                    .push("github copilot: premium allowance unreadable".to_owned());
            }
            if row.resets_at.is_none() {
                parsed.notes.push(
                    "github copilot: reset date unreported or unreadable; usage withheld"
                        .to_owned(),
                );
            }
            row.status = if row.used_percent.is_some() || parsed.unlimited {
                freshness(Some(observed_at), row.resets_at, now)
            } else {
                Status::ReadError
            };
        } else {
            parsed.notes.push(
                "github copilot: no Copilot premium quota reported for this account".to_owned(),
            );
        }
    } else {
        parsed
            .notes
            .push("github copilot: answer unparseable".to_owned());
    }
    parsed.rows.push(row);
    parsed
}

/// The reset instant. `quota_reset_date_utc` is an RFC 3339 instant and wins;
/// `quota_reset_date` is the fallback and is an instant or a bare `YYYY-MM-DD`.
fn reset(root: &Value) -> Option<i64> {
    root.get_str("quota_reset_date_utc")
        .and_then(go::timestamp)
        .or_else(|| root.get_str("quota_reset_date").and_then(instant_or_day))
}

// A bare day is UTC midnight, as GitHub states in its `_utc` twin of the field.
// `Timestamp::parse` is exact, so an impossible day never normalises into another.
fn instant_or_day(text: &str) -> Option<i64> {
    go::timestamp(text).or_else(|| {
        if text.len() != 10 {
            return None;
        }
        Timestamp::parse(&format!("{text}T00:00:00Z")).map(Timestamp::epoch)
    })
}

// Counts win. Present-but-invalid or partial counts never fall back to percent.
fn used(premium: &Value) -> Option<String> {
    let count = |value: Option<&Value>| match value {
        Some(Value::Num(number)) => Some(*number),
        _ => None,
    };
    let used = match (premium.get("entitlement"), premium.get("remaining")) {
        (None, None) => {
            100.0
                - percent(premium.get("percent_remaining"))?
                    .parse::<f64>()
                    .ok()
                    .filter(|number| *number <= 100.0)?
        }
        (entitlement, remaining) => {
            let total = count(entitlement)?;
            let left = count(remaining)?;
            if total <= 0 || !(0..=total).contains(&left) {
                return None;
            }
            // Decimal conversion avoids a lossy integer-cast exemption.
            (total - left).to_string().parse::<f64>().ok()?
                / total.to_string().parse::<f64>().ok()?
                * 100.0
        }
    };
    Some(format!("{used:.1}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-12-01T00:00:00Z, computed with GNU date, never with the code under test.
    const DEC_FIRST: i64 = 1_796_083_200;
    // 2026-11-17T01:46:40Z: after the answer was observed, before the reset.
    const NOW: i64 = 1_795_000_000;
    const WITHHELD: &str = "github copilot: reset date unreported or unreadable; usage withheld";

    /// The shape GitHub answered on 2026-11-09, minus login, id, organisation,
    /// endpoint and tracking values.
    fn answer(reset_fields: &str) -> String {
        format!(
            r#"{{"copilot_plan":"business","chat_enabled":true,{reset_fields}"quota_snapshots":{{"premium_interactions":{{"entitlement":30000,"overage_permitted":false,"percent_remaining":99.3,"quota_id":"premium_interactions","quota_remaining":29794.0,"remaining":29794,"unlimited":false}},"chat":{{"entitlement":0,"remaining":0,"percent_remaining":100.0,"unlimited":true}}}}}}"#
        )
    }

    fn read(reset_fields: &str) -> Parsed {
        parse(answer(reset_fields).as_bytes(), NOW, NOW)
    }

    fn reset_of(reset_fields: &str) -> Option<i64> {
        read(reset_fields)
            .rows
            .first()
            .and_then(|row| row.resets_at)
    }

    #[test]
    fn the_observed_answer_reads_point_seven_percent_and_the_first_of_december() {
        let parsed = read(
            r#""quota_reset_date":"2026-12-01","quota_reset_date_utc":"2026-12-01T00:00:00.000Z","#,
        );
        let row = parsed.rows.first();
        assert_eq!(row.and_then(|r| r.used_percent.as_deref()), Some("0.7"));
        assert_eq!(row.and_then(|r| r.resets_at), Some(DEC_FIRST));
        assert_eq!(row.map(|r| r.status), Some(Status::Fresh));
        assert!(parsed.notes.is_empty(), "no note: {:?}", parsed.notes);
        assert_eq!(parsed.plan.as_deref(), Some("business"));
    }

    #[test]
    fn the_utc_field_alone_gives_the_reset() {
        assert_eq!(
            reset_of(r#""quota_reset_date_utc":"2026-12-01T00:00:00.000Z","#),
            Some(DEC_FIRST)
        );
        assert_eq!(
            reset_of(r#""quota_reset_date_utc":"2026-12-01T00:00:00Z","#),
            Some(DEC_FIRST)
        );
    }

    #[test]
    fn a_bare_date_alone_is_midnight_utc_of_that_day() {
        assert_eq!(
            reset_of(r#""quota_reset_date":"2026-12-01","#),
            Some(DEC_FIRST)
        );
        // The day before: the bare date is not rounded up or down.
        assert_eq!(
            reset_of(r#""quota_reset_date":"2026-11-30","#),
            Some(DEC_FIRST - 86_400)
        );
    }

    #[test]
    fn an_instant_in_the_plain_field_still_reads() {
        assert_eq!(
            reset_of(r#""quota_reset_date":"2026-12-01T00:00:00Z","#),
            Some(DEC_FIRST)
        );
        assert_eq!(
            reset_of(r#""quota_reset_date":"2026-12-01T01:00:00+01:00","#),
            Some(DEC_FIRST)
        );
    }

    #[test]
    fn agreeing_fields_give_the_same_reset() {
        assert_eq!(
            reset_of(
                r#""quota_reset_date":"2026-12-01","quota_reset_date_utc":"2026-12-01T00:00:00.000Z","#
            ),
            Some(DEC_FIRST)
        );
    }

    #[test]
    fn the_utc_field_wins_when_the_fields_disagree() {
        assert_eq!(
            reset_of(
                r#""quota_reset_date":"2026-11-01","quota_reset_date_utc":"2026-12-01T00:00:00.000Z","#
            ),
            Some(DEC_FIRST)
        );
        assert_eq!(
            reset_of(
                r#""quota_reset_date":"2026-12-01T00:00:00Z","quota_reset_date_utc":"2026-12-02T00:00:00.000Z","#
            ),
            Some(DEC_FIRST + 86_400)
        );
    }

    #[test]
    fn an_unreadable_utc_field_falls_back_to_the_plain_one() {
        assert_eq!(
            reset_of(r#""quota_reset_date":"2026-12-01","quota_reset_date_utc":"soon","#),
            Some(DEC_FIRST)
        );
        assert_eq!(
            reset_of(r#""quota_reset_date":"2026-12-01","quota_reset_date_utc":null,"#),
            Some(DEC_FIRST)
        );
    }

    #[test]
    fn neither_field_withholds_usage_with_the_fixed_note() {
        for fields in [
            "",
            r#""quota_reset_date":null,"#,
            r#""quota_reset_date":"","quota_reset_date_utc":"","#,
            r#""quota_reset_date":"next month","quota_reset_date_utc":"later","#,
        ] {
            let parsed = read(fields);
            let row = parsed.rows.first();
            assert_eq!(row.and_then(|r| r.resets_at), None, "{fields}");
            assert_eq!(row.map(|r| r.status), Some(Status::Unknown), "{fields}");
            assert_eq!(parsed.notes, vec![WITHHELD.to_owned()], "{fields}");
        }
    }

    #[test]
    fn an_impossible_date_is_never_a_reset() {
        for date in [
            "2026-02-30",
            "2026-13-01",
            "2026-00-10",
            "2026-12-00",
            "2026-12-32",
            "2026-12-1",
            "2026-1-01",
            "20261201",
            "2026-12-01 ",
            "+026-12-01",
        ] {
            let fields = format!(r#""quota_reset_date":"{date}","#);
            assert_eq!(reset_of(&fields), None, "{date}");
            assert_eq!(read(&fields).notes, vec![WITHHELD.to_owned()], "{date}");
        }
        assert_eq!(
            reset_of(
                r#""quota_reset_date":"2026-02-30","quota_reset_date_utc":"2026-02-30T00:00:00.000Z","#
            ),
            None
        );
    }
}
