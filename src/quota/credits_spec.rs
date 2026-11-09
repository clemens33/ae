//! Frozen credits acceptance: R1 normalization, R2 second pool, R3 agreement.
//!
//! INSTALL (driver, byte-identical): copy to `src/quota/credits_spec.rs` and add
//! beside the `pace_history_spec` block in `src/quota.rs` this triple, as text:
//! `#[cfg(test)]`, `#[path = "quota/credits_spec.rs"]`, `mod credits_spec;`
//! Oracle: `.local/brief-credits.md` R1-R4, `.local/credits/nav-critique.md`
//! rulings, and hand-written fixture lines. Never the code under test.
//!
//! Existing API only: on the base commit this file COMPILES. The relief
//! assertions then fail at runtime while the parked and unchanged guards pass,
//! which is the RED that proves the tests discriminate. New-API pins
//! (`normalize_balance`, `Effective::Credits`, `Why::Credits`, `Derived::shown`)
//! live in `credits_api_spec.rs`, which is compile-red on base by design.

use std::collections::BTreeSet;
use std::path::Path;

use super::codex;
use super::history::{self, Row as HistoryRow};
use super::machine;
use super::pace::{Pace, PaceTable, Point};
use super::view;
use super::{
    Account, Classified, Credits, Group, Observation, Policy, QuotaLevel, Reading, Row, Status,
    credits_label, derived, effective, history_sample, render_at, table_cells,
    untrusted_credit_note,
};
use crate::tool::ToolKind;

const NOW: i64 = 1_800_000_000;
/// The window observation every rig shares.
const ROW_AT: i64 = NOW - 60;
const RESETS_AT: i64 = NOW + 3600;
/// `now` for Codex record fixtures, mirroring the codex suite's own clock.
const CODEX_NOW: i64 = 1_788_858_600;

const fn account(credits: Credits, at: Option<i64>, spend: Option<bool>) -> Account {
    Account {
        credits,
        credits_observed_at: at,
        spend_control_reached: spend,
        spend_observed_at: match spend {
            Some(_) => at,
            None => None,
        },
    }
}

const fn policy(
    credits: Credits,
    at: Option<i64>,
    spend: Option<bool>,
    resets: Option<u8>,
) -> Policy {
    Policy::for_tests(resets, account(credits, at, spend))
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

fn observation(group: Group) -> Observation {
    Observation {
        groups: vec![group.clone()],
        rendered: vec![group],
        home: None,
        now: NOW,
    }
}

/// One `token_count` record carrying `credits` JSON beside a 96% weekly window.
fn record(credits: &str) -> Vec<u8> {
    format!(
        "{{\"timestamp\":\"2026-09-08T09:00:00Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"rate_limits\":{{\"limit_id\":\"codex\",\"plan_type\":\"pro\",\"primary\":{{\"used_percent\":96,\"window_minutes\":10080,\"resets_at\":1789445400}},\"secondary\":null,\"credits\":{credits},\"spend_control_reached\":false}}}}}}\n",
    )
    .into_bytes()
}

fn parsed(credits: &str) -> codex::Snapshot {
    let bytes = record(credits);
    codex::parse(&bytes, true, CODEX_NOW).expect("record parses")
}

fn balance_case(balance: &str) -> Credits {
    parsed(&format!(
        "{{\"has_credits\":true,\"unlimited\":false,\"balance\":\"{balance}\"}}"
    ))
    .account
    .credits
}

fn classified(policy: Policy, row: Row) -> Classified {
    let reading = Reading::of(policy, row).expect("usable percentage and stamp");
    Classified::first(reading)
}

#[test]
fn real_codex_literal_normalizes_to_62500() {
    let snapshot = parsed(r#"{"has_credits":true,"unlimited":false,"balance":"62500.0000000000"}"#);
    assert_eq!(
        snapshot.account.credits,
        Credits::Available("62500".to_owned())
    );
    assert_eq!(
        snapshot.rows[0].used_percent.as_deref(),
        Some("96"),
        "windows are unchanged by account facts"
    );
}

#[test]
fn trailing_fraction_zeros_drop_and_bare_dot_drops() {
    for (literal, normalized) in [
        ("12.50", "12.5"),
        ("5.00", "5"),
        ("0.000", "0"),
        ("0.001", "0.001"),
        ("100", "100"),
    ] {
        assert_eq!(
            balance_case(literal),
            Credits::Available(normalized.to_owned()),
            "{literal}"
        );
    }
}

#[test]
fn exact_nine_char_literal_is_kept() {
    assert_eq!(
        balance_case("123456789"),
        Credits::Available("123456789".to_owned())
    );
}

#[test]
fn non_plain_decimals_are_available_without_amount() {
    for literal in [
        "-5", "+1", "1e3", "007", ".5", "5.", "", " 7", "1,5", "NaN", "1.2.3",
    ] {
        assert_eq!(
            balance_case(literal),
            Credits::AvailableUnknown,
            "{literal:?} states no exact amount"
        );
    }
}

#[test]
fn overlong_normalized_balance_is_available_without_amount() {
    assert_eq!(
        balance_case(&"9".repeat(300)),
        Credits::AvailableUnknown,
        "a balance ae cannot state exactly is never shown clipped"
    );
    assert_eq!(
        balance_case("123456789.5"),
        Credits::AvailableUnknown,
        "eleven chars after normalization exceeds the nine-char cell"
    );
}

#[test]
fn missing_balance_is_available_without_amount() {
    let snapshot = parsed(r#"{"has_credits":true,"unlimited":false}"#);
    assert_eq!(snapshot.account.credits, Credits::AvailableUnknown);
}

#[test]
fn credit_state_neighbors_unchanged() {
    let unlimited = parsed(r#"{"has_credits":true,"unlimited":true,"balance":"0"}"#);
    assert_eq!(unlimited.account.credits, Credits::Unlimited);
    let exhausted = parsed(r#"{"has_credits":false,"unlimited":false,"balance":"0"}"#);
    assert_eq!(exhausted.account.credits, Credits::Exhausted);
    let bytes = record("null");
    let absent = codex::parse(&bytes, true, CODEX_NOW).expect("record parses");
    // Every fixture record carries `"spend_control_reached":false`, so a
    // `credits:null` record reports no credit state but one real spend fact,
    // stamped with the record time 2026-09-08T09:00:00Z (1_788_858_000).
    assert_eq!(
        absent.account,
        Account {
            credits: Credits::Unreported,
            credits_observed_at: None,
            spend_control_reached: Some(false),
            spend_observed_at: Some(1_788_858_000),
        }
    );
}

#[test]
fn usable_credits_relieve_exactly_like_unlimited_does() {
    let relieve = |credits: Credits, at: Option<i64>, window: Option<i64>| {
        account(credits, at, Some(false)).credits_relieve(window)
    };
    for (credits, at, window, expected, why) in [
        (
            Credits::Unlimited,
            Some(ROW_AT),
            Some(ROW_AT),
            true,
            "unlimited",
        ),
        (
            Credits::Available("62500".to_owned()),
            Some(ROW_AT),
            Some(ROW_AT),
            true,
            "positive balance, same stamp",
        ),
        (
            Credits::Available("0.001".to_owned()),
            Some(ROW_AT),
            Some(ROW_AT),
            true,
            "smallest positive balance",
        ),
        (
            Credits::AvailableUnknown,
            Some(ROW_AT),
            Some(ROW_AT),
            true,
            "unstated amount",
        ),
        (
            Credits::Available("0".to_owned()),
            Some(ROW_AT),
            Some(ROW_AT),
            false,
            "zero relieves nothing",
        ),
        (
            Credits::Available("0.0".to_owned()),
            Some(ROW_AT),
            Some(ROW_AT),
            false,
            "unnormalized zero relieves nothing",
        ),
        (
            Credits::Exhausted,
            Some(ROW_AT),
            Some(ROW_AT),
            false,
            "exhausted",
        ),
        (
            Credits::Unreported,
            Some(ROW_AT),
            Some(ROW_AT),
            false,
            "unreported",
        ),
        (
            Credits::Available("62500".to_owned()),
            Some(ROW_AT - 1),
            Some(ROW_AT),
            false,
            "older than the window",
        ),
        (
            Credits::Available("62500".to_owned()),
            None,
            Some(ROW_AT),
            false,
            "unstamped",
        ),
        (
            Credits::AvailableUnknown,
            Some(ROW_AT - 1),
            Some(ROW_AT),
            false,
            "older unstated amount",
        ),
    ] {
        if expected {
            assert!(relieve(credits, at, window), "{why}");
        } else {
            assert!(!relieve(credits, at, window), "{why}");
        }
    }
}

#[test]
fn effective_order_with_second_pool() {
    let fresh = |credits: Credits| account(credits, Some(ROW_AT), Some(false));
    let stale_balance = account(
        Credits::Available("62500".to_owned()),
        Some(ROW_AT - 3600),
        Some(false),
    );
    let capped = account(
        Credits::Available("62500".to_owned()),
        Some(ROW_AT),
        Some(true),
    );
    let plain = Account::default();
    let case = |declared: Option<u8>, account: &Account, used: f64| {
        let policy = Policy::for_tests(declared, account.clone());
        let judged = effective(&policy, used, Some(ROW_AT));
        (judged.cell(), judged.judged(used))
    };
    for (declared, account, used, cell, judged, why) in [
        (None, &plain, 96.0, "-", 96.0, "raw control"),
        (
            None,
            &fresh(Credits::Available("62500".to_owned())),
            96.0,
            "96% cr",
            0.0,
            "balance carries the window",
        ),
        (
            None,
            &fresh(Credits::AvailableUnknown),
            96.0,
            "96% cr",
            0.0,
            "unstated amount carries the window",
        ),
        (
            None,
            &fresh(Credits::Available("0".to_owned())),
            96.0,
            "-",
            96.0,
            "zero balance",
        ),
        (
            None,
            &fresh(Credits::Exhausted),
            96.0,
            "-",
            96.0,
            "exhausted",
        ),
        (None, &stale_balance, 96.0, "-", 96.0, "older report"),
        (
            None,
            &fresh(Credits::Unlimited),
            96.0,
            "0%",
            0.0,
            "unlimited control",
        ),
        (
            None,
            &capped,
            96.0,
            "100%",
            100.0,
            "spend cap outranks the balance",
        ),
    ] {
        let (actual_cell, actual_judged) = case(declared, account, used);
        assert_eq!(actual_cell, cell, "{why}");
        assert_eq!(actual_judged.to_bits(), f64::to_bits(judged), "{why}");
        assert!(
            actual_cell.chars().count() <= 9,
            "{why}: the cell fits its 9-char column"
        );
    }
}

#[test]
fn effective_credits_outrank_resets_and_keep_decimals() {
    let fresh = |credits: Credits| account(credits, Some(ROW_AT), Some(false));
    let stale_balance = account(
        Credits::Available("62500".to_owned()),
        Some(ROW_AT - 3600),
        Some(false),
    );
    let plain = Account::default();
    let case = |declared: Option<u8>, account: &Account, used: f64| {
        let policy = Policy::for_tests(declared, account.clone());
        let judged = effective(&policy, used, Some(ROW_AT));
        (judged.cell(), judged.judged(used))
    };
    for (declared, account, used, cell, judged, why) in [
        (
            Some(1),
            &fresh(Credits::Available("62500".to_owned())),
            96.0,
            "96% cr",
            0.0,
            "credits outrank declared resets",
        ),
        (
            Some(1),
            &stale_balance,
            96.0,
            "48% x1",
            48.0,
            "a stale report leaves the resets",
        ),
        (Some(1), &plain, 96.0, "48% x1", 48.0, "resets control"),
        (
            None,
            &fresh(Credits::Available("62500".to_owned())),
            96.5,
            "96.5% cr",
            0.0,
            "the decimal survives",
        ),
        (
            None,
            &fresh(Credits::Available("62500".to_owned())),
            100.0,
            "100% cr",
            0.0,
            "a full window on credits",
        ),
    ] {
        let (actual_cell, actual_judged) = case(declared, account, used);
        assert_eq!(actual_cell, cell, "{why}");
        assert_eq!(actual_judged.to_bits(), f64::to_bits(judged), "{why}");
        assert!(
            actual_cell.chars().count() <= 9,
            "{why}: the cell fits its 9-char column"
        );
    }
}

#[test]
fn eighty_percent_stepdown_dropped_while_credits_relieve() {
    let relieved = policy(
        Credits::Available("62500".to_owned()),
        Some(ROW_AT),
        Some(false),
        None,
    );
    let decided = classified(relieved, row("85"));
    assert_eq!(decided.level(), QuotaLevel::Headroom);
    assert_eq!(decided.judged().to_bits(), 0.0_f64.to_bits());
}

/// The one rig every agreement test shares: a 96% weekly window with a fresh
/// 62500 balance. R3: one `derived` value feeds the table, the watchdog, the
/// advisory, the machine output, the app view, PACE and history.
fn relieved() -> (Policy, Row) {
    (
        policy(
            Credits::Available("62500".to_owned()),
            Some(ROW_AT),
            Some(false),
            None,
        ),
        row("96"),
    )
}

#[test]
fn relieved_window_derives_one_visible_value() {
    let (policy, row) = relieved();
    let judged = derived(&policy, &row).expect("usable percentage");
    assert_eq!(judged.judged().to_bits(), 0.0_f64.to_bits());
    assert_eq!(judged.cell(), "96% cr");
    assert_eq!(judged.derivation(), Some("runs on credits".to_owned()));
    assert_eq!(judged.settings_cell(), "96% credits");
}

#[test]
fn relieved_window_table_cells_keep_used_and_show_credits() {
    let (policy, row) = relieved();
    let group = group(vec![row.clone()], policy);
    let mut claimed = BTreeSet::new();
    let cells = table_cells(
        &group,
        &row,
        0,
        &PaceTable::default(),
        &mut claimed,
        None,
        NOW,
    );
    assert_eq!(cells[4], "96%", "USED stays the vendor figure");
    assert_eq!(
        cells[5], "96% cr",
        "EFFECTIVE never pretends the window empty"
    );
    assert_eq!(cells[6], "62500", "CREDITS shows the normalized literal");
    assert_eq!(
        cells[7], "-",
        "no PACE verdict while credits carry the window"
    );
}

#[test]
fn relieved_window_watchdog_and_advisory_agree_with_table() {
    let (policy, row) = relieved();
    let group = group(vec![row.clone()], policy);
    // The watchdog threshold and auto reseat read exactly this level and this
    // figure (`autoreseat::leg::window`); Headroom here is "not parked".
    let decided = classified(group.policy.clone(), row);
    assert_eq!(decided.level(), QuotaLevel::Headroom);
    assert_eq!(decided.judged().to_bits(), 0.0_f64.to_bits());
    let line = observation(group.clone()).state_line(&group, &decided, Path::new("/tmp/meta"));
    assert!(line.contains("96%"), "{line}");
    assert!(line.contains("(headroom;"), "{line}");
    assert!(line.contains("runs on credits 62500"), "{line}");
}

#[test]
fn relieved_window_view_machine_pace_history_agree() {
    let (policy, row) = relieved();
    let group = group(vec![row.clone()], policy.clone());
    let scopes = observation(group.clone()).quota_view();
    let window = &scopes[0].windows[0];
    assert_eq!(window.raw.as_deref(), Some("96%"));
    assert_eq!(window.counts, None, "the label did not move");
    assert_eq!(window.fill, Some(8), "the bar follows the shown figure");
    assert!(window.why.is_some(), "a reason is named");
    assert_eq!(view::note(window, true), "runs on credits");
    assert_eq!(view::note(window, false), "credits");
    assert_eq!(window.level, view::Level::Headroom);
    assert_eq!(window.burn, None);
    assert_eq!(scopes[0].account.as_deref(), Some("credits 62500"));

    let document = machine::document([&group], &PaceTable::default(), None, NOW).render();
    assert!(
        document.contains(r#""used_percent":96,"effective_percent":0,"derivation":"credits""#),
        "{document}"
    );
    assert!(
        document.contains(r#""state":"available","balance":62500"#),
        "{document}"
    );

    let newest = Point {
        observed_at: ROW_AT,
        used: 96.0,
        resets_at: RESETS_AT,
    };
    let earlier = Point {
        observed_at: ROW_AT - 6 * 3600,
        used: 90.0,
        resets_at: RESETS_AT,
    };
    assert_eq!(
        Pace::measured(&policy, 10_080, newest, &[earlier], NOW),
        (Pace::Absent, None),
        "a rate over a sufficient span is still no verdict on credits"
    );

    let reading = Reading::of(policy, row).expect("usable percentage and stamp");
    let sample = history_sample(&group, &reading, "producer", "incarnation", NOW).expect("sample");
    assert_eq!(sample.derivation, "credits");
    assert_eq!(sample.effective, "0.000000");
}

#[test]
fn available_unknown_relieve_without_amount() {
    let policy = policy(Credits::AvailableUnknown, Some(ROW_AT), Some(false), None);
    let decided = classified(policy.clone(), row("96"));
    assert_eq!(decided.level(), QuotaLevel::Headroom);
    assert_eq!(decided.judged().to_bits(), 0.0_f64.to_bits());
    assert_eq!(credits_label(&policy), "available");
    let group = group(vec![row("96")], policy);
    let line = observation(group.clone()).state_line(&group, &decided, Path::new("/tmp/meta"));
    assert!(line.contains("runs on credits,"), "{line}");
    let document = machine::document([&group], &PaceTable::default(), None, NOW).render();
    assert!(
        document.contains(r#""state":"available","balance":null"#),
        "{document}"
    );
}

#[test]
fn stale_credit_report_parks() {
    let policy = policy(
        Credits::Available("62500".to_owned()),
        Some(ROW_AT - 3600),
        Some(false),
        None,
    );
    let decided = classified(policy.clone(), row("96"));
    assert_eq!(decided.level(), QuotaLevel::Critical);
    assert_eq!(decided.judged().to_bits(), 96.0_f64.to_bits());
    let judged = derived(&policy, &row("96")).expect("usable percentage");
    assert_eq!(judged.cell(), "-");
    let group = group(vec![row("96")], policy);
    let mut claimed = BTreeSet::new();
    let cells = table_cells(
        &group,
        &group.rows[0],
        0,
        &PaceTable::default(),
        &mut claimed,
        None,
        NOW,
    );
    assert_eq!(cells[4], "96%");
    assert_eq!(cells[5], "-");
    let note = untrusted_credit_note(&group).expect("the ignored claim is named");
    assert_eq!(
        note,
        "credits 62500 was reported older than the newest window here, so the claim is ignored; EFFECTIVE follows the remaining rules"
    );
    let table = render_at(std::slice::from_ref(&group), None, NOW);
    assert!(table.contains(&note), "{table}");
    let document = machine::document([&group], &PaceTable::default(), None, NOW).render();
    assert!(
        document.contains(r#""used_percent":96,"effective_percent":96,"derivation":null"#),
        "{document}"
    );
}

#[test]
fn exhausted_credits_park_without_a_claim_note() {
    let policy = policy(Credits::Exhausted, Some(ROW_AT), Some(false), None);
    let decided = classified(policy.clone(), row("96"));
    assert_eq!(decided.level(), QuotaLevel::Critical);
    let judged = derived(&policy, &row("96")).expect("usable percentage");
    assert_eq!(judged.cell(), "-");
    let group = group(vec![row("96")], policy);
    assert_eq!(untrusted_credit_note(&group), None);
}

#[test]
fn zero_balance_parks_without_a_claim_note() {
    let policy = policy(
        Credits::Available("0".to_owned()),
        Some(ROW_AT),
        Some(false),
        None,
    );
    let decided = classified(policy.clone(), row("96"));
    assert_eq!(decided.level(), QuotaLevel::Critical);
    assert_eq!(decided.judged().to_bits(), 96.0_f64.to_bits());
    let group = group(vec![row("96")], policy);
    assert_eq!(untrusted_credit_note(&group), None);
}

#[test]
fn spend_cap_outranks_balance_on_every_surface() {
    let policy = policy(
        Credits::Available("62500".to_owned()),
        Some(ROW_AT),
        Some(true),
        None,
    );
    let decided = classified(policy.clone(), row("96"));
    assert_eq!(decided.level(), QuotaLevel::Critical);
    assert_eq!(decided.judged().to_bits(), 100.0_f64.to_bits());
    assert_eq!(credits_label(&policy), "spend-cap");
    let group = group(vec![row("96")], policy);
    assert_eq!(untrusted_credit_note(&group), None);
    let document = machine::document([&group], &PaceTable::default(), None, NOW).render();
    assert!(
        document.contains(r#""effective_percent":100,"derivation":"spend-cap""#),
        "{document}"
    );
    assert!(
        document.contains(r#""state":"spend-cap","balance":null"#),
        "{document}"
    );
}

#[test]
fn stale_unlimited_note_keeps_existing_words() {
    let policy = policy(Credits::Unlimited, Some(ROW_AT - 3600), Some(false), None);
    let group = group(vec![row("96")], policy);
    assert_eq!(
        untrusted_credit_note(&group).as_deref(),
        Some(
            "credits unlimited was reported older than the newest window here, so the claim is ignored; EFFECTIVE follows the remaining rules"
        )
    );
}

#[test]
fn stale_unstated_amount_note_names_availability() {
    let policy = policy(
        Credits::AvailableUnknown,
        Some(ROW_AT - 3600),
        Some(false),
        None,
    );
    let group = group(vec![row("96")], policy);
    assert_eq!(
        untrusted_credit_note(&group).as_deref(),
        Some(
            "credits available was reported older than the newest window here, so the claim is ignored; EFFECTIVE follows the remaining rules"
        )
    );
}

#[test]
fn fresh_claims_raise_no_note() {
    for credits in [
        Credits::Unlimited,
        Credits::Available("62500".to_owned()),
        Credits::AvailableUnknown,
    ] {
        let policy = policy(credits, Some(ROW_AT), Some(false), None);
        let group = group(vec![row("96")], policy);
        assert_eq!(untrusted_credit_note(&group), None);
    }
}

#[test]
fn relief_is_direction_symmetric() {
    let mut decided = classified(Policy::for_tests(None, Account::default()), row("96"));
    assert_eq!(decided.level(), QuotaLevel::Critical);
    let relieved = policy(
        Credits::Available("62500".to_owned()),
        Some(ROW_AT),
        Some(false),
        None,
    );
    let incoming = Reading::of(relieved, row("96")).expect("usable percentage and stamp");
    decided.adopt(&incoming);
    assert_eq!(decided.level(), QuotaLevel::Headroom);
    let spent = policy(Credits::Exhausted, Some(ROW_AT + 60), Some(false), None);
    let mut newer = row("96");
    newer.observed_at = Some(ROW_AT + 60);
    let outgoing = Reading::of(spent, newer).expect("usable percentage and stamp");
    decided.adopt(&outgoing);
    assert_eq!(decided.level(), QuotaLevel::Critical);
}

#[test]
fn history_parse_accepts_credits_derivation() {
    let HistoryRow::Sample(sample) = history::parse(
        r#"{"v":1,"kind":"sample","producer":"s","incarnation":"11111111-1111-4111-8111-111111111111","collected_at":2000029950,"scope":{"source":"/h/.claude.json","tool":"claude","account":"/h"},"bucket":"weekly_all","window_minutes":10080,"resets_at":2000289200,"observed_at":2000029940,"raw":"96","effective":"0.000000","derivation":"credits","policy":{"credits":"available","version":1,"manual_resets":null}}"#,
    )
    .expect("the daemon reads its own lines") else {
        panic!("sample seed");
    };
    assert_eq!(sample.derivation, "credits");
    assert_eq!(sample.effective, "0.000000");
}

#[test]
fn credits_cell_spellings() {
    let label = |credits: Credits, spend: Option<bool>| {
        credits_label(&policy(credits, Some(ROW_AT), spend, None))
    };
    assert_eq!(label(Credits::Unreported, None), "-");
    assert_eq!(label(Credits::Unlimited, Some(false)), "unlimited");
    assert_eq!(
        label(Credits::Available("62500".to_owned()), Some(false)),
        "62500"
    );
    assert_eq!(label(Credits::AvailableUnknown, None), "available");
    assert_eq!(label(Credits::Exhausted, None), "none");
    assert_eq!(
        label(Credits::Available("62500".to_owned()), Some(true)),
        "spend-cap"
    );
}

#[test]
fn real_codex_record_parks_nothing() {
    let snapshot = parsed(r#"{"has_credits":true,"unlimited":false,"balance":"62500.0000000000"}"#);
    let policy = Policy::for_tests(None, snapshot.account.clone());
    let row = snapshot.rows[0].clone();
    assert_eq!(credits_label(&policy), "62500");
    assert_eq!(
        derived(&policy, &row).expect("usable percentage").cell(),
        "96% cr"
    );
    let decided = classified(policy, row);
    assert_eq!(decided.level(), QuotaLevel::Headroom);
    assert_eq!(decided.judged().to_bits(), 0.0_f64.to_bits());
}
