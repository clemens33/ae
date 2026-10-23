//! Pure parser for GitHub's undocumented `copilot_internal/user` answer.
//! Unknown fields carry no verdict; missing or invalid measurements yield notes.

use crate::json::{self, Value};

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
        row.resets_at = root.get_str("quota_reset_date").and_then(go::timestamp);
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
