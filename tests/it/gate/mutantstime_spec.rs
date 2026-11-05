//! Independent #234 acceptance. Frozen separately before implementation.
//! Requirements: plan-r2 + owner-r2-addendum, base 3b1b8f71.

use super::{compiled_it_test_names, decode_mutants_inventory, mutants_timeout_policy};
use crate::parity::capture::ExitOutcome;

const APPROVED: [&str; 3] = [
    "autotrust_spec::measured_yes_and_no_use_only_the_arrow_then_enter_once_across_watchdog_restart",
    "reseat_headroom_spec::a_readable_unknown_frame_holds_headroom_instead_of_stopping_the_tool",
    "reseat_headroom_spec::busy_and_draft_holds_do_not_spend_attempts_and_move_after_idle",
];

const CAP: &str = "slow-timeout = { period = \"90s\", terminate-after = 2, grace-period = \"0s\" }";
const GLOBAL: &str =
    "slow-timeout = { period = \"30s\", terminate-after = 2, grace-period = \"0s\" }";
const COMPACT: &str = "slow-timeout = { period = \"5s\", terminate-after = 2 }";
const LINUX: &str = "slow-timeout = { period = \"60s\", terminate-after = 5 }";
const MENU: &str = "slow-timeout = { period = \"30s\", terminate-after = 2 }";

// Independent requirement inputs. This never asks the implementation to emit
// its own fixture, selectors, approved names, or expected classifications.
fn baseline() -> String {
    format!(
        "[profile.default]\ntest-threads = 8\nfail-fast = false\n\n\
         [test-groups.tmux-clients]\nmax-threads = 3\n\n\
         [[profile.default.overrides]]\nfilter = 'test(/^menu::/)'\ntest-group = 'tmux-clients'\n\n\
         [[profile.default.overrides]]\nfilter = 'test(=compact::a_writerless_fifo_swapped_in_after_lstat_is_refused_without_blocking)'\n{COMPACT}\n\n\
         [profile.linux]\nfail-fast = false\n{LINUX}\n\n\
         [[profile.linux.overrides]]\nfilter = 'test(/^menu::/)'\n{MENU}\n\n\
         [profile.mutants]\ntest-threads = 2\nfail-fast = true\n{GLOBAL}\n"
    )
}

fn exception(name: &str, timeout: &str) -> String {
    format!("\n[[profile.mutants.overrides]]\nfilter = 'test(={name})'\n{timeout}\n")
}

fn approved_config() -> String {
    let mut text = baseline();
    for name in APPROVED {
        text.push_str(&exception(name, CAP));
    }
    text
}

#[test]
fn the_mutants_timeout_exceptions_are_named_and_finite() {
    // Acquisition is unconditional. An error cannot be replaced by an empty
    // or synthetic registered-name list. Runtime proof of the door is separate.
    let registered = compiled_it_test_names().expect("bounded compiled IT inventory");
    let registered: Vec<&str> = registered.iter().map(String::as_str).collect();
    for required in APPROVED {
        assert_eq!(
            registered.iter().filter(|name| **name == required).count(),
            1,
            "approved identifier must exist exactly once in the compiled IT inventory: {required}"
        );
    }
    let actual = super::read(&super::root().join(".config/nextest.toml"));
    let actual_violations = mutants_timeout_policy(&actual, &registered);
    assert!(
        actual_violations.is_empty(),
        "actual config: {actual_violations:?}"
    );
    for name in APPROVED {
        assert!(
            actual.contains(&exception(name, CAP)),
            "actual config must carry the canonical approved 180-second block: {name}"
        );
    }

    let good = approved_config();
    let short = "slow-timeout = { period = \"60s\", terminate-after = 2, grace-period = \"0s\" }";
    let mut cases: Vec<(String, String, Vec<&str>, bool)> = Vec::new();
    {
        let mut add = |label: &str, input: String, accepted: bool| {
            cases.push((label.to_owned(), input, APPROVED.to_vec(), accepted));
        };
        add(
            "approved 180-second caps; compact20; linux baselines; group-only",
            good.clone(),
            true,
        );
        add(
            "approved shorter finite120 cap",
            good.replacen(CAP, short, 1),
            true,
        );
        let mut reverse = baseline();
        for name in APPROVED.into_iter().rev() {
            reverse.push_str(&exception(name, CAP));
        }
        add("nonconflicting override order", reverse, true);
        add(
            "unrelated mutants group-only override",
            format!(
                "{good}\n[[profile.mutants.overrides]]\nfilter = 'test(/^menu::/)'\ntest-group = 'tmux-clients'\n"
            ),
            true,
        );
        add(
            "timeout-looking comment is inert",
            format!(
                "{good}\n# slow-timeout = {{ period = \"999s\" }}\n# [[profile.mutants.overrides]]\n"
            ),
            true,
        );
        add("missing all required exceptions", baseline(), false);
        for name in APPROVED {
            add(
                &format!("missing required exception {name}"),
                good.replace(&exception(name, CAP), ""),
                false,
            );
        }
        add(
            "unknown exact name",
            format!("{good}{}", exception("unapproved", CAP)),
            false,
        );
        add(
            "fourth app-selected exception",
            format!(
                "{good}{}",
                exception(
                    "app_selected_spec::home_and_non_home_uuid_replacement_drop_old_memory_once_after_positive_fleet_read",
                    CAP
                )
            ),
            false,
        );
        for selector in [
            "test(/^reseat_headroom_spec::/)",
            "all()",
            "test(autotrust_spec)",
            "test(=unapproved) | all()",
        ] {
            add(
                &format!("broad selector {selector}"),
                format!("{good}\n[[profile.mutants.overrides]]\nfilter = '{selector}'\n{CAP}\n"),
                false,
            );
        }
        for timeout in [
            "slow-timeout = { period = \"91s\", terminate-after = 2, grace-period = \"0s\" }",
            "slow-timeout = { period = \"90s\", terminate-after = 2 }",
            "slow-timeout = { period = \"90s\", terminate-after = 2, grace-period = \"1s\" }",
            "slow-timeout = { period = \"90s\" }",
            "slow-timeout = \"90s\"",
            "slow-timeout = { period = \"90s\", terminate-after = 0, grace-period = \"0s\" }",
            "slow-timeout = { period = \"90s\", terminate-after = \"infinite\", grace-period = \"0s\" }",
            "slow-timeout = { period = \"4294967295s\", terminate-after = 4294967295, grace-period = \"0s\" }",
            "slow-timeout = { period = \"1m\", terminate-after = 2, grace-period = \"0s\" }",
            "slow-timeout = { period = \"0s\", terminate-after = 2, grace-period = \"0s\" }",
            "slow-timeout = { period = \"90s\", terminate-after = 2, grace-period = \"0s\", unknown = 1 }",
            "slow-timeout = { period = \"90s\", terminate-after = 2, terminate-after = 99, grace-period = \"0s\" }",
            "slow-timeout = {\nperiod = \"90s\",\nterminate-after = 2,\ngrace-period = \"0s\"\n}",
        ] {
            add(
                &format!("unsupported/unbounded timeout {timeout}"),
                good.replacen(CAP, timeout, 1),
                false,
            );
        }
        add(
            "required block missing timeout",
            good.replacen(CAP, "", 1),
            false,
        );
        add(
            "duplicate required timeout key",
            good.replacen(CAP, &format!("{CAP}\n{CAP}"), 1),
            false,
        );
        add(
            "duplicate required exception after",
            format!("{good}{}", exception(APPROVED[0], CAP)),
            false,
        );
        add(
            "duplicate required exception before",
            format!("{}\n{good}", exception(APPROVED[0], short)),
            false,
        );
        add(
            "unknown override before approved overrides",
            format!("{}\n{good}", exception("unapproved", CAP)),
            false,
        );
        add(
            "missing selector in timeout block",
            good.replacen(&format!("filter = 'test(={})'\n", APPROVED[0]), "", 1),
            false,
        );
        add(
            "duplicate selector in timeout block",
            good.replacen(
                &format!("filter = 'test(={})'", APPROVED[0]),
                &format!("filter = 'test(={})'\nfilter = 'all()'", APPROVED[0]),
                1,
            ),
            false,
        );
        for header in [
            "[[ profile.mutants.overrides ]]",
            "[[profile.\"mutants\".overrides]]",
            "[[profile.'mutants'.overrides]]",
            "[[\"profile\".mutants.overrides]]",
        ] {
            add(
                &format!("unsupported header {header}"),
                good.replacen("[[profile.mutants.overrides]]", header, 1),
                false,
            );
            // Keep every required canonical block: an ignored alternate
            // header must not escape detection behind a missing-name error.
            add(
                &format!("appended unsupported header {header}"),
                format!("{good}\n{header}\nfilter = 'all()'\n{CAP}\n"),
                false,
            );
        }
        for key in [
            "\"slow-timeout\"",
            "\"slow\\u002dtimeout\"",
            "'slow-timeout'",
            "slow-timeout.period",
            "\"slow-timeout\".period",
        ] {
            add(
                &format!("unsupported key {key}"),
                good.replacen("slow-timeout =", &format!("{key} ="), 1),
                false,
            );
            add(
                &format!("appended unsupported timeout key {key}"),
                format!(
                    "{good}\n[[profile.mutants.overrides]]\nfilter = 'all()'\n{key} = {{ period = \"90s\", terminate-after = 2, grace-period = \"0s\" }}\n"
                ),
                false,
            );
        }
        add(
        "timeout subtable",
        good.replacen(CAP, "[profile.mutants.overrides.slow-timeout]\nperiod = \"90s\"\nterminate-after = 2\ngrace-period = \"0s\"", 1),
        false,
    );
        add(
            "appended timeout subtable after complete approved set",
            format!(
                "{good}\n[[profile.mutants.overrides]]\nfilter = 'all()'\n[profile.mutants.overrides.slow-timeout]\nperiod = \"90s\"\nterminate-after = 2\ngrace-period = \"0s\"\n"
            ),
            false,
        );
        add(
            "appended dotted timeout after complete approved set",
            format!(
                "{good}\n[[profile.mutants.overrides]]\nfilter = 'all()'\nslow-timeout.period = \"90s\"\nslow-timeout.terminate-after = 2\nslow-timeout.grace-period = \"0s\"\n"
            ),
            false,
        );
        add(
            "dotted timeout before canonical headers",
            format!(
                "profile.mutants.slow-timeout = {{ period = \"999s\", terminate-after = 2 }}\n{good}"
            ),
            false,
        );
        add(
            "unclassified profile timeout",
            format!("{good}\n[profile.unapproved]\n{CAP}\n"),
            false,
        );
        add(
            "global base widened120",
            good.replacen(GLOBAL, short, 1),
            false,
        );
        add("global base missing", good.replacen(GLOBAL, "", 1), false);
        add(
            "global duplicate base timeout",
            good.replacen(GLOBAL, &format!("{GLOBAL}\n{GLOBAL}"), 1),
            false,
        );
        for header in [
            "[ profile.mutants ]",
            "[profile.\"mutants\"]",
            "['profile'.mutants]",
        ] {
            add(
                &format!("unsupported base header {header}"),
                good.replacen("[profile.mutants]", header, 1),
                false,
            );
        }
        add(
            "unattributable timeout before first header",
            format!("{CAP}\n{good}"),
            false,
        );
        add(
            "compact inherited20 widened",
            good.replacen(COMPACT, CAP, 1),
            false,
        );
        add(
            "compact inherited20 removed",
            good.replacen(COMPACT, "", 1),
            false,
        );
        add(
            "linux baseline changed",
            good.replacen(LINUX, CAP, 1),
            false,
        );
        add("linux baseline removed", good.replacen(LINUX, "", 1), false);
        add(
            "linux menu override changed",
            good.replacen(MENU, CAP, 1),
            false,
        );
        add(
            "linux menu override removed",
            good.replacen(MENU, "", 1),
            false,
        );
        for field in [
            "retries = 1",
            "on-timeout = 'pass'",
            "slow-timeout.period = '90s'",
        ] {
            add(
                &format!("unsupported approved block field {field}"),
                good.replacen(CAP, &format!("{CAP}\n{field}"), 1),
                false,
            );
        }
        for timeout in [
            "slow-timeout = { \"period\" = \"90s\", terminate-after = 2, grace-period = \"0s\" }",
            "slow-timeout = { period = \"90s\", 'terminate-after' = 2, grace-period = \"0s\" }",
            "slow-timeout = { period = \"90s\", terminate-after = 2, grace-period = \"0s\", on-timeout = 'pass' }",
        ] {
            add(
                &format!("unsupported inline timeout field {timeout}"),
                good.replacen(CAP, timeout, 1),
                false,
            );
        }
        for header in [
            "[[profile.default.overrides]]",
            "[[profile.linux.overrides]]",
        ] {
            add(
                &format!("new forbidden timeout {header}"),
                format!("{good}\n{header}\nfilter = 'test(=unapproved)'\n{CAP}\n"),
                false,
            );
        }
    }
    cases.push((
        "empty compiled inventory".into(),
        good.clone(),
        vec![],
        false,
    ));
    for (index, required) in APPROVED.into_iter().enumerate() {
        let mut missing = APPROVED.to_vec();
        missing.remove(index);
        cases.push((
            format!("compiled name missing {required}"),
            good.clone(),
            missing,
            false,
        ));
        let mut duplicate = APPROVED.to_vec();
        duplicate.push(required);
        cases.push((
            format!("compiled name duplicate {required}"),
            good.clone(),
            duplicate,
            false,
        ));
    }
    let mut extra = APPROVED.to_vec();
    extra.push("unrelated::registered_test");
    cases.push(("unrelated compiled name accepted".into(), good, extra, true));

    // Aggregate all cases: an always-accept weakening must report every
    // negative as a behavioural mismatch, rather than stop at its first arm.
    let mut failures = Vec::new();
    for (label, input, inventory, accepted) in cases {
        let violations = mutants_timeout_policy(&input, &inventory);
        if violations.is_empty() != accepted {
            failures.push(format!(
                "{label}: expected accepted={accepted}; violations={violations:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "policy matrix mismatches:\n{}",
        failures.join("\n")
    );
}

#[test]
fn compiled_inventory_decoding_never_accepts_unknown_or_partial_acquisition() {
    let listing = APPROVED.map(|name| format!("{name}: test\n")).concat();
    let mut decoded = decode_mutants_inventory(ExitOutcome::Code(0), &listing, "")
        .expect("exact successful terse inventory");
    let mut expected = APPROVED.map(str::to_owned).to_vec();
    decoded.sort();
    expected.sort();
    assert_eq!(decoded, expected);
    // Decoding preserves multiplicity; the policy rejects duplicate approved
    // identifiers. Deduplication here would destroy the evidence it needs.
    let duplicate_listing = format!("{listing}{}: test\n", APPROVED[0]);
    let mut duplicate_expected = APPROVED.map(str::to_owned).to_vec();
    duplicate_expected.push(APPROVED[0].to_owned());
    let mut duplicate_decoded =
        decode_mutants_inventory(ExitOutcome::Code(0), &duplicate_listing, "")
            .expect("duplicate names must be preserved for policy validation");
    duplicate_expected.sort();
    duplicate_decoded.sort();
    assert_eq!(duplicate_decoded, duplicate_expected);
    for (label, outcome, stdout, stderr) in [
        (
            "nonzero with plausible partial inventory",
            ExitOutcome::Code(1),
            listing.as_str(),
            "failed",
        ),
        (
            "signal with plausible inventory",
            ExitOutcome::Signalled,
            listing.as_str(),
            "",
        ),
        ("empty successful inventory", ExitOutcome::Code(0), "", ""),
        (
            "blank successful inventory",
            ExitOutcome::Code(0),
            "\n\n",
            "",
        ),
        (
            "malformed record",
            ExitOutcome::Code(0),
            "unrelated: test\ncorruption\n",
            "",
        ),
        ("empty name", ExitOutcome::Code(0), ": test\n", ""),
        (
            "benchmark record",
            ExitOutcome::Code(0),
            "unrelated: benchmark\n",
            "",
        ),
        (
            "unsupported summary trailer",
            ExitOutcome::Code(0),
            "unrelated: test\n1 test, 0 benchmarks\n",
            "",
        ),
        (
            "nonempty acquisition stderr",
            ExitOutcome::Code(0),
            listing.as_str(),
            "warning: incomplete inventory",
        ),
    ] {
        assert!(
            decode_mutants_inventory(outcome, stdout, stderr).is_err(),
            "{label}"
        );
    }
}
