//! Frozen credits acceptance: new-API pins. Compile-red on base by design.
//!
//! INSTALL (driver, byte-identical): copy to `src/quota/credits_api_spec.rs`
//! and add beside the `pace_history_spec` block in `src/quota.rs` this triple,
//! as text: `#[cfg(test)]`, `#[path = "quota/credits_api_spec.rs"]`,
//! `mod credits_api_spec;`
//! Oracle: `.local/brief-credits.md` R1-R2 and `.local/credits/nav-critique.md`
//! rulings V3/V4. This file references `normalize_balance`, `Derived::shown`,
//! `Effective::Credits` and `view::Why::Credits`, none of which exists on base,
//! so it fails to compile there; `credits_spec.rs` carries the runtime RED.

use super::view;
use super::{
    Account, Credits, Derived, Effective, Group, Observation, Policy, Row, Status, derived,
    normalize_balance,
};
use crate::tool::ToolKind;

const NOW: i64 = 1_800_000_000;
const ROW_AT: i64 = NOW - 60;
const RESETS_AT: i64 = NOW + 3600;

const fn policy(
    credits: Credits,
    at: Option<i64>,
    spend: Option<bool>,
    resets: Option<u8>,
) -> Policy {
    Policy::for_tests(
        resets,
        Account {
            credits,
            credits_observed_at: at,
            spend_control_reached: spend,
            spend_observed_at: match spend {
                Some(_) => at,
                None => None,
            },
        },
    )
}

fn row(used: &str) -> Row {
    Row {
        bucket: "codex".to_owned(),
        qualifier: Some("pro".to_owned()),
        window_minutes: Some(10_080),
        used_percent: Some(used.to_owned()),
        resets_at: Some(RESETS_AT),
        observed_at: Some(ROW_AT),
        status: Status::Fresh,
    }
}

fn group(rows: Vec<Row>, policy: Policy) -> Group {
    Group {
        profiles: vec!["solx".to_owned()],
        tool: ToolKind::Codex,
        home: Some(std::path::PathBuf::from("/tmp/cx")),
        source: Some(std::path::PathBuf::from("/tmp/cx/sessions")),
        clients: vec!["cx".to_owned()],
        rollout: None,
        owner: None,
        rows,
        hint: None,
        summary: None,
        policy,
        notes: Vec::new(),
        plan: None,
    }
}

#[test]
fn normalize_balance_accepts_plain_decimals_exactly() {
    for (literal, normalized) in [
        ("62500.0000000000", "62500"),
        ("12.50", "12.5"),
        ("5.00", "5"),
        ("0.000", "0"),
        ("0", "0"),
        ("0.001", "0.001"),
        ("100", "100"),
        ("123456789", "123456789"),
    ] {
        assert_eq!(
            normalize_balance(literal).as_deref(),
            Some(normalized),
            "{literal}"
        );
    }
}

#[test]
fn normalize_balance_rejects_non_plain() {
    // The twin of `machine::decimal`'s table: one plain-decimal rule in the
    // tree, so a stored Available literal always JSON-renders as a number.
    for literal in [
        "", "-", ".5", "5.", "007", "+1", "1e3", "0x10", "NaN", "inf", "1,5", "1 ", "--1", "1.2.3",
        "$5", " 7",
    ] {
        assert_eq!(normalize_balance(literal), None, "{literal:?}");
    }
}

#[test]
fn shown_returns_visible_figure() {
    let shown = |policy: Policy, used: &str| {
        derived(&policy, &row(used))
            .expect("usable percentage")
            .shown()
    };
    let relieved = policy(
        Credits::Available("62500".to_owned()),
        Some(ROW_AT),
        Some(false),
        None,
    );
    assert_eq!(shown(relieved, "96").to_bits(), 96.0_f64.to_bits());
    let plain = Policy::for_tests(None, Account::default());
    assert_eq!(shown(plain, "96").to_bits(), 96.0_f64.to_bits());
    let unlimited = policy(Credits::Unlimited, Some(ROW_AT), Some(false), None);
    assert_eq!(shown(unlimited, "96").to_bits(), 0.0_f64.to_bits());
    let resets = Policy::for_tests(Some(1), Account::default());
    assert_eq!(shown(resets, "96").to_bits(), 48.0_f64.to_bits());
}

#[test]
fn why_credits_identity() {
    let policy = policy(
        Credits::Available("62500".to_owned()),
        Some(ROW_AT),
        Some(false),
        None,
    );
    let group = group(vec![row("96")], policy);
    let scopes = Observation {
        groups: vec![group.clone()],
        rendered: vec![group],
        home: None,
        now: NOW,
    }
    .quota_view();
    assert_eq!(scopes[0].windows[0].why, Some(view::Why::Credits));
}

#[test]
fn effective_credits_carries_raw_window() {
    let policy = policy(
        Credits::Available("62500".to_owned()),
        Some(ROW_AT),
        Some(false),
        None,
    );
    match super::effective(&policy, 96.0, Some(ROW_AT)) {
        Effective::Credits { used } => assert_eq!(used.to_bits(), 96.0_f64.to_bits()),
        other => panic!("a fresh balance derives Credits, not {other:?}"),
    }
}

#[test]
fn derivation_types_stay_copy() {
    const fn assert_copy<T: Copy>() {}
    // `window_view` maps one `derived` several times; a `String` in the
    // variant would break that `Copy` chain, so the balance stays out.
    assert_copy::<Effective>();
    assert_copy::<Derived>();
}
