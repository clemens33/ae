//! `ae quota --json`: the table's rows as one JSON document.
//!
//! PURE, like the table beside it: the same ordered groups, pace verdicts,
//! home and instant go in, one [`Value`] comes out. It judges nothing itself —
//! every cell is read from the owner the table reads it from, and a window the
//! table shows as `-` is `null` here — so the two surfaces cannot disagree
//! about a fact, only about its spelling.

use std::collections::BTreeSet;
use std::path::Path;

use super::pace::PaceTable;
use super::{
    Credits, Derived, Effective, Group, Policy, RolloutSummary, Row, Status, collect_notes,
    derived, manual_refresh_hint, placeholder, row_percent, scope_label, stated_observed_at,
};
use crate::json::Value;

/// The document's schema version. Fields are only ever added.
const VERSION: i64 = 1;

/// A finite float as a JSON number; anything else is `null`.
pub(super) fn number(value: f64) -> Value {
    if value.is_finite() {
        Value::Raw(format!("{value}"))
    } else {
        Value::Null
    }
}

/// A pace figure as its table cell spells it: one decimal.
pub(super) fn tenths(value: f64) -> f64 {
    format!("{value:.1}").parse().unwrap_or(value)
}

/// An exact plain decimal literal as a number token, with no float round trip;
/// any other spelling states no amount, so it is `null`.
fn decimal(literal: &str) -> Value {
    let unsigned = literal.strip_prefix('-').unwrap_or(literal);
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let plain =
        digits(whole) && (whole == "0" || !whole.starts_with('0')) && fraction.is_none_or(digits);
    if plain {
        Value::Raw(literal.to_owned())
    } else {
        Value::Null
    }
}

/// The CREDITS cell as a state word and an exact balance, when one is stated.
fn credits(policy: &Policy) -> Value {
    let (state, balance) = if policy.spend_capped() {
        ("spend-cap", Value::Null)
    } else {
        match &policy.account.credits {
            Credits::Unreported => ("unreported", Value::Null),
            Credits::Unlimited => ("unlimited", Value::Null),
            Credits::Available(balance) => ("available", decimal(balance)),
            Credits::AvailableUnknown => ("available", Value::Null),
            Credits::Exhausted => ("none", Value::Null),
        }
    };
    Value::obj([("state", Value::str(state)), ("balance", balance)])
}

/// How the judged percentage got there; `null` when nothing was derived.
fn derivation(derived: Derived) -> Value {
    match derived.effective {
        Effective::Resets { resets, .. } if resets > 0 => Value::str(format!("x{resets}")),
        Effective::Unlimited => Value::str("unlimited"),
        Effective::SpendCapped => Value::str("spend-cap"),
        Effective::Raw | Effective::Resets { .. } => Value::Null,
    }
}

fn text(value: Option<&str>) -> Value {
    value.map_or(Value::Null, Value::str)
}

fn epoch(value: Option<i64>) -> Value {
    value.map_or(Value::Null, Value::Num)
}

fn count(value: usize) -> Value {
    Value::Num(i64::try_from(value).unwrap_or(i64::MAX))
}

fn summary(rollouts: &RolloutSummary) -> Value {
    Value::obj([
        ("hidden_rollouts", count(rollouts.hidden)),
        ("unreadable_rollouts", count(rollouts.unreadable)),
        ("oldest_observed_at", epoch(rollouts.oldest_observed)),
        ("not_read", Value::Bool(rollouts.not_read)),
        ("status", text(rollouts.status.map(Status::as_str))),
    ])
}

/// One window. What the table withholds from an untrustworthy row is `null`.
fn window(group: &Group, row: &Row, reason: Option<&str>, pace: Value, now: i64) -> Value {
    let trusted = matches!(row.status, Status::Fresh | Status::Stale);
    let figure = derived(&group.policy, row).filter(|_| trusted);
    let when = |value: Option<i64>| if trusted { epoch(value) } else { Value::Null };
    Value::obj([
        (
            "bucket",
            text((*row != placeholder(row.status)).then_some(row.bucket.as_str())),
        ),
        ("qualifier", text(row.qualifier.as_deref())),
        ("window_minutes", when(row.window_minutes.map(i64::from))),
        (
            "used_percent",
            row_percent(row)
                .filter(|_| trusted)
                .map_or(Value::Null, number),
        ),
        // The figure a threshold classifies, unrounded: the cell's one decimal is for reading.
        (
            "effective_percent",
            figure.map_or(Value::Null, |figure| number(figure.judged())),
        ),
        ("derivation", figure.map_or(Value::Null, derivation)),
        ("pace", pace),
        ("resets_at", when(row.resets_at)),
        (
            "observed_at",
            epoch(stated_observed_at(row.observed_at, now)),
        ),
        ("status", Value::str(row.status.as_str())),
        ("reason", text(reason)),
    ])
}

/// The whole report: every group the table prints, in the table's order.
pub(super) fn document<'a>(
    groups: impl IntoIterator<Item = &'a Group>,
    pace: &PaceTable,
    home: Option<&Path>,
    now: i64,
) -> Value {
    let mut notes: Vec<String> = Vec::new();
    let mut claimed = BTreeSet::new();
    let mut scopes = Vec::new();
    for group in groups {
        collect_notes(&mut notes, group);
        let refresh_hint = manual_refresh_hint(group.tool, group.rows.iter().map(|row| row.status));
        let reason = group.hint.as_deref().or(refresh_hint);
        let windows = group
            .rows
            .iter()
            .map(|row| {
                let verdict = pace.pace_for(group, row, &mut claimed);
                window(
                    group,
                    row,
                    reason,
                    verdict
                        .and_then(super::pace::Pace::json)
                        .unwrap_or(Value::Null),
                    now,
                )
            })
            .collect();
        scopes.push(Value::obj([
            (
                "profiles",
                Value::Arr(group.profiles.iter().map(Value::str).collect()),
            ),
            ("scope", Value::str(scope_label(group, home))),
            ("tool", Value::str(group.tool.as_str())),
            ("rollout_owner", text(group.owner.as_deref())),
            (
                "credits",
                if group.summary.is_some() {
                    Value::Null
                } else {
                    credits(&group.policy)
                },
            ),
            (
                "summary",
                group.summary.as_ref().map_or(Value::Null, summary),
            ),
            ("windows", Value::Arr(windows)),
        ]));
    }
    Value::obj([
        ("version", Value::Num(VERSION)),
        ("now", Value::Num(now)),
        ("scopes", Value::Arr(scopes)),
        (
            "notes",
            Value::Arr(notes.into_iter().map(Value::Str).collect()),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{decimal, document, number, tenths, window};
    use crate::json::Value;
    use crate::quota::pace::PaceTable;
    use crate::quota::{
        Account, Credits, Group, Policy, RolloutSummary, Row, Status, placeholder, render_at,
    };
    use crate::tool::ToolKind;

    const NOW: i64 = 1_800_000_000;

    fn row(bucket: &str, used: Option<&str>, status: Status) -> Row {
        Row {
            bucket: bucket.to_owned(),
            qualifier: Some("pro".to_owned()),
            window_minutes: Some(300),
            used_percent: used.map(str::to_owned),
            resets_at: Some(NOW + 3600),
            observed_at: Some(NOW - 60),
            status,
        }
    }

    fn group(rows: Vec<Row>, policy: Policy) -> Group {
        Group {
            profiles: vec!["solx".to_owned()],
            tool: ToolKind::Codex,
            home: Some(PathBuf::from("/tmp/cx")),
            source: Some(PathBuf::from("/tmp/cx/sessions")),
            clients: vec!["cx".to_owned()],
            rollout: None,
            owner: None,
            rows,
            hint: None,
            summary: None,
            policy,
            notes: Vec::new(),
        }
    }

    fn one(group: &Group) -> String {
        window(group, &group.rows[0], None, Value::Null, NOW).render()
    }

    #[test]
    fn a_trusted_window_states_each_fact_as_a_number_or_null() {
        let declared = group(
            vec![row("codex", Some("95"), Status::Fresh)],
            Policy::for_tests(Some(1), Account::default()),
        );
        assert_eq!(
            one(&declared),
            r#"{"bucket":"codex","qualifier":"pro","window_minutes":300,"used_percent":95,"effective_percent":47.5,"derivation":"x1","pace":null,"resets_at":1800003600,"observed_at":1799999940,"status":"fresh","reason":null}"#
        );
        // Nothing declared or reported: the judged number is the window's own, no derivation.
        let raw = group(
            vec![row("codex", Some("3.50"), Status::Stale)],
            Policy::default(),
        );
        assert_eq!(
            one(&raw),
            r#"{"bucket":"codex","qualifier":"pro","window_minutes":300,"used_percent":3.5,"effective_percent":3.5,"derivation":null,"pace":null,"resets_at":1800003600,"observed_at":1799999940,"status":"stale","reason":null}"#
        );
        // A declaration of zero resets derives nothing: the word is `null`, not `x0`.
        let zero = group(
            vec![row("codex", Some("12.0"), Status::Fresh)],
            Policy::for_tests(Some(0), Account::default()),
        );
        assert!(
            one(&zero).contains(r#""used_percent":12,"effective_percent":12,"derivation":null"#),
            "{}",
            one(&zero)
        );
    }

    #[test]
    fn what_the_table_withholds_from_an_untrustworthy_row_is_null() {
        let unknown = group(
            vec![row("codex", Some("95"), Status::Unknown)],
            Policy::for_tests(Some(1), Account::default()),
        );
        assert_eq!(
            one(&unknown),
            r#"{"bucket":"codex","qualifier":"pro","window_minutes":null,"used_percent":null,"effective_percent":null,"derivation":null,"pace":null,"resets_at":null,"observed_at":1799999940,"status":"unknown","reason":null}"#
        );
        // A stamp in the future states no age.
        let mut skewed = row("codex", Some("1"), Status::Unknown);
        skewed.observed_at = Some(NOW + 600);
        let skewed = group(vec![skewed], Policy::default());
        assert!(
            one(&skewed).contains(r#""observed_at":null"#),
            "{}",
            one(&skewed)
        );
        // A row with no usable percentage has no used and no effective figure.
        let none = group(vec![row("codex", None, Status::Fresh)], Policy::default());
        assert!(
            one(&none).contains(r#""used_percent":null,"effective_percent":null"#),
            "{}",
            one(&none)
        );
    }

    #[test]
    fn a_placeholder_row_has_no_bucket_and_the_scope_hint_is_its_reason() {
        let mut unsupported = group(vec![placeholder(Status::Unsupported)], Policy::default());
        unsupported.hint = Some("run /usage in grok".to_owned());
        let rendered = window(
            &unsupported,
            &unsupported.rows[0],
            unsupported.hint.as_deref(),
            Value::Null,
            NOW,
        )
        .render();
        assert_eq!(
            rendered,
            r#"{"bucket":null,"qualifier":null,"window_minutes":null,"used_percent":null,"effective_percent":null,"derivation":null,"pace":null,"resets_at":null,"observed_at":null,"status":"unsupported","reason":"run /usage in grok"}"#
        );
        // A vendor bucket that merely looks like the sentinel is still a bucket.
        let mut named = row("-", Some("1"), Status::Fresh);
        named.qualifier = None;
        let named = group(vec![named], Policy::default());
        assert!(one(&named).contains(r#""bucket":"-""#), "{}", one(&named));
    }

    #[test]
    fn unlimited_credits_and_a_spend_cap_derive_by_the_tables_rule() {
        let unlimited = Account {
            credits: Credits::Unlimited,
            credits_observed_at: Some(NOW - 30),
            ..Account::default()
        };
        let free = group(
            vec![row("codex", Some("80"), Status::Fresh)],
            Policy::for_tests(None, unlimited),
        );
        assert!(
            one(&free)
                .contains(r#""used_percent":80,"effective_percent":0,"derivation":"unlimited""#),
            "{}",
            one(&free)
        );
        let capped = Account {
            credits: Credits::Available("5.00".to_owned()),
            spend_control_reached: Some(true),
            ..Account::default()
        };
        let stopped = group(
            vec![row("codex", Some("10"), Status::Fresh)],
            Policy::for_tests(Some(3), capped),
        );
        assert!(
            one(&stopped)
                .contains(r#""used_percent":10,"effective_percent":100,"derivation":"spend-cap""#),
            "{}",
            one(&stopped)
        );
    }

    #[test]
    fn credits_state_the_cell_and_an_exact_balance_never_a_rounded_one() {
        let of = |credits: Credits, spend: Option<bool>| {
            let account = Account {
                credits,
                spend_control_reached: spend,
                ..Account::default()
            };
            super::credits(&Policy::for_tests(None, account)).render()
        };
        assert_eq!(
            of(Credits::Unreported, None),
            r#"{"state":"unreported","balance":null}"#
        );
        assert_eq!(
            of(Credits::Unlimited, Some(false)),
            r#"{"state":"unlimited","balance":null}"#
        );
        assert_eq!(
            of(Credits::Exhausted, None),
            r#"{"state":"none","balance":null}"#
        );
        assert_eq!(
            of(Credits::AvailableUnknown, None),
            r#"{"state":"available","balance":null}"#
        );
        // The literal's own digits, trailing zero included: no float in between.
        assert_eq!(
            of(Credits::Available("12.50".to_owned()), None),
            r#"{"state":"available","balance":12.50}"#
        );
        assert_eq!(
            of(Credits::Available("123456789".to_owned()), None),
            r#"{"state":"available","balance":123456789}"#
        );
        // The cap outranks every credit state, as the CREDITS cell does.
        assert_eq!(
            of(Credits::Available("5.00".to_owned()), Some(true)),
            r#"{"state":"spend-cap","balance":null}"#
        );
    }

    #[test]
    fn only_a_plain_decimal_becomes_a_number_token() {
        for literal in ["0", "-0.5", "12.50", "100", "0.001", "123456789"] {
            assert_eq!(
                decimal(literal),
                Value::Raw(literal.to_owned()),
                "{literal}"
            );
        }
        for literal in [
            "", "-", ".5", "5.", "007", "+1", "1e3", "0x10", "NaN", "inf", "1,5", "1 ", "--1",
            "1.2.3", "$5",
        ] {
            assert_eq!(decimal(literal), Value::Null, "{literal:?}");
        }
        assert_eq!(number(f64::NAN), Value::Null);
        assert_eq!(number(f64::INFINITY), Value::Null);
        assert_eq!(number(47.5), Value::Raw("47.5".to_owned()));
        assert_eq!(number(1e21).render(), "1000000000000000000000");
    }

    #[test]
    fn a_number_is_rounded_exactly_as_the_cell_rounds_it() {
        let cell = |value: f64| number(tenths(value)).render();
        assert_eq!(cell(47.5), "47.5");
        assert_eq!(cell(1.75), "1.8");
        assert_eq!(cell(0.25), "0.2");
        assert_eq!(cell(33.333_333), "33.3");
        assert_eq!(cell(12.0), "12");
    }

    #[test]
    fn the_effective_figure_is_the_unrounded_judged_percentage() {
        // (declared resets, window usage, judged figure): usage / (1 + resets), worked by hand. Each sits just
        // under a 90 or 95 threshold that the table's one-decimal cell rounds ONTO (89.96 reads 90.0%), so a
        // rounded export would classify differently from the figure the watchdog judges.
        for (resets, used, judged, derivation) in [
            (0_u8, "89.96", "89.96", "null"),
            (0, "94.96", "94.96", "null"),
            (1, "189.9", "94.95", r#""x1""#),
            (1, "89.96", "44.98", r#""x1""#),
            (3, "359.8", "89.95", r#""x3""#),
        ] {
            let declared = group(
                vec![row("codex", Some(used), Status::Fresh)],
                Policy::for_tests(Some(resets), Account::default()),
            );
            let expected = format!(r#""effective_percent":{judged},"derivation":{derivation},"#);
            assert!(
                one(&declared).contains(&expected),
                "{used}/{resets}: {}",
                one(&declared)
            );
            // The same value the threshold reads, not a second derivation of it.
            let read = crate::quota::derived(&declared.policy, &declared.rows[0])
                .map(|derived| number(derived.judged()).render());
            assert_eq!(read.as_deref(), Some(judged), "{used}/{resets}");
        }
        let boundary = group(
            vec![row("codex", Some("89.96"), Status::Fresh)],
            Policy::for_tests(Some(0), Account::default()),
        );
        let table = render_at(std::slice::from_ref(&boundary), None, NOW);
        assert!(
            table.contains("90%"),
            "the cell rounds 89.96 to one decimal: {table}"
        );
        assert!(
            !one(&boundary).contains(r#""effective_percent":90,"#),
            "{}",
            one(&boundary)
        );
    }

    #[test]
    fn json_and_table_state_the_same_effective_figure() {
        let declared = group(
            vec![row("codex", Some("95"), Status::Fresh)],
            Policy::for_tests(Some(1), Account::default()),
        );
        let table = render_at(std::slice::from_ref(&declared), None, NOW);
        assert!(table.contains("95%   47.5% x1"), "{table}");
        let doc = document([&declared], &PaceTable::default(), None, NOW).render();
        assert!(
            doc.contains(r#""used_percent":95,"effective_percent":47.5,"derivation":"x1""#),
            "{doc}"
        );
    }

    #[test]
    fn the_document_lists_scopes_summary_entries_and_notes_once() {
        let mut shown = group(
            vec![row("codex", Some("7"), Status::Fresh)],
            Policy::default(),
        );
        shown.owner = Some("session:lead".to_owned());
        shown.notes = vec!["declared note".to_owned(), "bell\u{7}note".to_owned()];
        let mut hidden = group(Vec::new(), Policy::default());
        hidden.summary = Some(RolloutSummary {
            hidden: 3,
            unreadable: 1,
            oldest_observed: Some(NOW - 7200),
            not_read: false,
            status: Some(Status::ReadError),
        });
        hidden.notes = vec!["declared note".to_owned()];
        let doc = document([&shown, &hidden], &PaceTable::default(), None, NOW).render();
        assert!(
            doc.starts_with(r#"{"version":1,"now":1800000000,"scopes":[{"profiles":["solx"],"scope":"codex · cx · unidentified (session:lead)","tool":"codex","rollout_owner":"session:lead","credits":{"state":"unreported","balance":null},"summary":null,"windows":[{"bucket":"codex","#),
            "{doc}"
        );
        assert!(
            doc.contains(
                r#"{"profiles":["solx"],"scope":"codex · cx · unidentified","tool":"codex","rollout_owner":null,"credits":null,"summary":{"hidden_rollouts":3,"unreadable_rollouts":1,"oldest_observed_at":1799992800,"not_read":false,"status":"read-error"},"windows":[]}"#
            ),
            "{doc}"
        );
        // Each note once, in order, and the control byte escaped by the crate's own writer.
        assert!(
            doc.ends_with(r#""notes":["declared note","bell\u0007note"]}"#),
            "{doc}"
        );
        assert!(!doc.chars().any(char::is_control), "{doc:?}");
    }
}
