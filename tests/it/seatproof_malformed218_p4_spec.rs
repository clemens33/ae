// P4 named malformed-expectation cases. Include as a sibling of the frozen
// residual218_spec module; the original six-case loop remains unchanged.
use super::*;

fn malformed(tag: &str, vars: &[(&str, &str)]) {
    let w = World::new(tag);
    let before = w.screen();
    let mut command = w.command("send", true, false);
    for &(key, value) in vars {
        command.env(key, value);
    }
    let output = command.output().expect("SETUP: real malformed-env helper");
    // The independent requirement is no input plus unprovable refusal. B0
    // reaches the forbidden-input assertion, not an unavailable future API.
    w.untouched(&before);
    w.refuse(&output, "could not be proven");
}

#[test]
fn r7_slot_only_expectation_is_unprovable() {
    malformed("r218-p4-slot-only", &[("_AE_EXPECT_SLOT", SLOT)]);
}

#[test]
fn r7_launch_only_expectation_is_unprovable() {
    malformed("r218-p4-launch-only", &[("_AE_EXPECT_LAUNCH", "tok-kid")]);
}

#[test]
fn r7_legacy_only_expectation_is_unprovable() {
    malformed("r218-p4-legacy-only", &[("_AE_EXPECT_LEGACY", "1")]);
}

#[test]
fn r7_launch_and_legacy_expectations_conflict() {
    malformed(
        "r218-p4-conflicting",
        &[
            ("_AE_EXPECT_SLOT", SLOT),
            ("_AE_EXPECT_LAUNCH", "tok-kid"),
            ("_AE_EXPECT_LEGACY", "1"),
        ],
    );
}

#[test]
fn r7_noncanonical_legacy_value_is_unprovable() {
    malformed(
        "r218-p4-legacy-value",
        &[("_AE_EXPECT_SLOT", SLOT), ("_AE_EXPECT_LEGACY", "2")],
    );
}

#[test]
fn r7_empty_launch_value_becomes_unprovable_slot_only() {
    // send_env omits the empty value. Preserve that accepted door behavior:
    // this names the construction separately, without inventing a wire case.
    malformed(
        "r218-p4-empty-token",
        &[("_AE_EXPECT_SLOT", SLOT), ("_AE_EXPECT_LAUNCH", "")],
    );
}

#[test]
fn r9_malformed_slot_keeps_unprovable_refusal_before_identity_mismatch() {
    for (tag, slot) in [("r218-mut-slot-empty", ""), ("r218-mut-slot-path", "x/y")] {
        let w = World::new(tag);
        let before = w.screen();
        let mut command = w.command("send", true, false);
        for key in [
            "_AE_EXPECT_SLOT",
            "_AE_EXPECT_LAUNCH",
            "_AE_EXPECT_LEGACY",
            "AE_SENDER_OVERRIDE",
        ] {
            command.env_remove(key);
        }
        command
            .env("_AE_EXPECT_SLOT", slot)
            .env("_AE_EXPECT_LAUNCH", "tok-kid")
            .env("_AE_EVENT_ACTION", "done-challenge")
            .env("_AE_EVENT_SUMMARY", BODY);
        let output = command.output().expect("SETUP: real malformed-slot helper");
        w.untouched(&before);
        // B5-M pins the refusal class; only the path arm reaches the slot guard.
        w.refuse(&output, "could not be proven");
        // Observation only: ordinary-refusal challenge non-vacuity is unproved.
        assert_eq!(
            w.count("done-challenge"),
            0,
            "malformed expectation booked a carried attempt: {}",
            w.events()
        );
    }
}
