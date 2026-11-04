//! Frozen pure-decision contract tests, authored before the helpers exist.
//! Oracle: locked's phase-0 L4/M3j/M3l checklist, unchanged base behavior.
use super::*;

#[test]
fn locked_armed_live_choice_preserves_each_boundary_and_the_unarmed_case() {
    assert!(armed_live(None, "sold").is_ok());
    for boundary in [
        "after-intent",
        "after-work-move",
        "after-state-move",
        "after-meta",
        "after-assets",
        "after-result",
    ] {
        assert_eq!(
            armed_live(Some(boundary), "sold").err().map(|line| line.0),
            Some(format!(
                "Error: AE_TEST_RENAME_CRASH_AT='{boundary}' is armed for a stopped rename, but session 'sold' is live — live crash cuts need a separate ruling. Nothing was renamed."
            ))
        );
    }
}

#[test]
fn locked_told_choice_uses_both_facts_and_prefers_the_missing_journal_reason() {
    for (journaled, watchdog, expected) in [
        (false, false, Told::Untold("rename not journaled")),
        (false, true, Told::Untold("rename not journaled")),
        (true, false, Told::Untold("watchdog off")),
        (true, true, Told::Told),
    ] {
        assert_eq!(
            told(journaled, watchdog),
            expected,
            "journaled={journaled}, watchdog={watchdog}"
        );
    }
}

#[test]
fn locked_monitor_refusal_names_the_expected_monitor_and_keeps_the_remedy() {
    for (watchdog, expected) in [
        (
            false,
            "Error: the session was renamed to 'snew', but its events monitor was not rebound (failed observation).",
        ),
        (
            true,
            "Error: the session was renamed to 'snew', but its monitor processes were not rebound (failed observation). The renamed session has NO verified watchdog; run 'ae watchdog start snew' after fixing it.",
        ),
    ] {
        assert_eq!(
            monitor_refusal("snew", "failed observation", watchdog).0,
            expected
        );
    }
}
