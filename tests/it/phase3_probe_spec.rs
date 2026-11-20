//! Frozen R2b acceptance; independently authored from the invariant and source.
//! Install unchanged as tests/it/phase3_probe_spec.rs, registered by phase3.rs.

use super::{decode_world_reading_sites, world_read_probe_args};
use crate::parity::capture::ExitOutcome;
use std::path::Path;

/// Independent file-level inventory. Source witnesses live in inventory.md.
pub(super) fn expected_production_files() -> Vec<String> {
    [
        "src/app/filedoor.rs",
        "src/app/tty.rs",
        "src/l2_model/archive.rs",
        "src/l2_model/archive/store.rs",
        "src/l3_services/autoupgrade.rs",
        "src/l3_services/board.rs",
        "src/l3_services/brief_retry.rs",
        "src/l3_services/carry.rs",
        "src/l3_services/compact.rs",
        "src/l2_model/config.rs",
        "src/doctor.rs",
        "src/l1_platform/doors.rs",
        "src/l2_model/events.rs",
        "src/l3_services/go_usage.rs",
        "src/l3_services/init.rs",
        "src/l3_services/install.rs",
        "src/l2_model/inventory.rs",
        "src/lib.rs",
        "src/l4_orchestration/lifecycle.rs",
        "src/l4_orchestration/lifecycle/compaction.rs",
        "src/l4_orchestration/lifecycle/end.rs",
        "src/l2_model/meta.rs",
        "src/l4_orchestration/migrate.rs",
        "src/monitor.rs",
        "src/l3_services/quota.rs",
        "src/l3_services/quota/history/store.rs",
        "src/l4_orchestration/rename.rs",
        "src/l3_services/render.rs",
        "src/l3_services/run.rs",
        "src/l3_services/session.rs",
        "src/l4_orchestration/session_launch.rs",
        "src/l4_orchestration/session_launch/capture.rs",
        "src/l1_platform/shape.rs",
        "src/l2_model/store.rs",
        "src/l4_orchestration/teardown.rs",
        "src/l3_services/telegram.rs",
        "src/l4_orchestration/telegram_lifecycle.rs",
        "src/l1_platform/transport.rs",
        "src/l3_services/usage.rs",
        "src/l4_orchestration/watchdog_daemon.rs",
        "src/l4_orchestration/watchdog_glue.rs",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

const SITE: &str = r#"{"reason":"compiler-message","message":{"code":{"code":"clippy::disallowed_methods"},"spans":[{"file_name":"src/l1_platform/doors.rs","line_start":38,"is_primary":true}]}}"#;
const OTHER: &str =
    r#"{"reason":"compiler-message","message":{"code":{"code":"unused_variables"},"spans":[]}}"#;
const ARTIFACT: &str = r#"{"reason":"compiler-artifact","fresh":true}"#;

fn rejected(outcome: ExitOutcome, stdout: &str, stderr: &str, words: &[&str]) {
    let error = decode_world_reading_sites(outcome, stdout, stderr)
        .expect_err("incomplete compiler evidence must never yield an inventory");
    let lower = error.to_lowercase();
    for word in words {
        assert!(
            lower.contains(word),
            "named diagnostic must contain {word:?}: {error}"
        );
    }
}

#[test]
fn nonzero_exit_rejects_plausible_partial_sites() {
    // Catches Result::is_ok(), ignored child status, or returning partial sites.
    rejected(
        ExitOutcome::Code(101),
        SITE,
        "error: could not compile ae: injected-failure-marker",
        &["clippy", "101", "injected-failure-marker"],
    );
}

#[test]
fn signalled_exit_rejects_plausible_partial_sites() {
    // Catches treating a missing exit code as success.
    rejected(
        ExitOutcome::Signalled,
        SITE,
        "killed",
        &["clippy", "signal"],
    );
}

#[test]
fn undecodable_record_among_valid_sites_is_not_dropped() {
    // Catches the old `let Ok(..) else { continue }` decode path.
    let stdout = format!("{SITE}\n{{undecodable\n{OTHER}\n");
    rejected(ExitOutcome::Code(0), &stdout, "", &["clippy", "decode"]);
}

#[test]
fn non_json_record_among_valid_sites_is_not_dropped() {
    // Catches skipping non-{ records before the JSON decoder sees them.
    let stdout = format!("{SITE}\nunexpected compiler output\n");
    rejected(ExitOutcome::Code(0), &stdout, "", &["clippy", "decode"]);
}

#[test]
fn malformed_boundary_span_cannot_shrink_inventory() {
    // Catches skipped missing/empty spans, missing file names, and bad lines.
    for malformed in [
        r#"{"reason":"compiler-message","message":{"code":{"code":"clippy::disallowed_methods"},"spans":[{"file_name":"src/new_door.rs","line_start":"broken","is_primary":true}]}}"#,
        r#"{"reason":"compiler-message","message":{"code":{"code":"clippy::disallowed_methods"},"spans":[]}}"#,
        r#"{"reason":"compiler-message","message":{"code":{"code":"clippy::disallowed_methods"}}}"#,
        r#"{"reason":"compiler-message","message":{"code":{"code":"clippy::disallowed_methods"},"spans":[{"line_start":12,"is_primary":true}]}}"#,
    ] {
        let stdout = format!("{SITE}\n{malformed}\n");
        rejected(ExitOutcome::Code(0), &stdout, "", &["clippy", "span"]);
    }
}

#[test]
fn successful_compiler_with_no_sites_is_rejected() {
    // Catches dropping --force-warn or the nonempty-site check.
    rejected(ExitOutcome::Code(0), OTHER, "", &["clippy", "site"]);
}

#[test]
fn empty_and_artifact_only_streams_are_rejected() {
    // Catches a cached/no-message stream being accepted as a complete probe.
    for stdout in ["", ARTIFACT] {
        rejected(ExitOutcome::Code(0), stdout, "", &["clippy", "message"]);
    }
}

#[test]
fn success_returns_sites_and_ignores_valid_unrelated_records() {
    // Positive control catches an implementation that unconditionally fails.
    let stdout = format!("{ARTIFACT}\n{OTHER}\n{SITE}\n");
    assert_eq!(
        decode_world_reading_sites(ExitOutcome::Code(0), &stdout, "")
            .expect("complete successful probe"),
        vec![("src/l1_platform/doors.rs".to_owned(), 38)],
    );
}

#[test]
fn probe_arguments_select_both_production_targets_and_force_the_named_lint() {
    // Catches later removal of --bins when main.rs has no existing named call,
    // removal of lib/features/locked/JSON coverage, or accidentally selecting tests.
    let target = Path::new("r2b-owned-target");
    let args = world_read_probe_args(target);
    let args: Vec<_> = args
        .iter()
        .map(|arg| arg.to_str().expect("fixed probe arguments are UTF-8"))
        .collect();
    assert_eq!(args.first(), Some(&"clippy"));
    let separator = args
        .iter()
        .position(|arg| *arg == "--")
        .expect("rustc separator");
    let cargo = &args[..separator];
    for required in [
        "--locked",
        "--lib",
        "--bins",
        "--all-features",
        "--message-format=json",
    ] {
        assert!(
            cargo.contains(&required),
            "missing cargo argument {required}"
        );
    }
    for forbidden in ["--all-targets", "--tests"] {
        assert!(
            !cargo.contains(&forbidden),
            "test target selected: {forbidden}"
        );
    }
    assert!(
        cargo
            .windows(2)
            .any(|pair| pair == ["--target-dir", "r2b-owned-target"])
    );
    assert!(
        args[separator + 1..]
            .windows(2)
            .any(|pair| pair == ["--force-warn", "clippy::disallowed_methods"])
    );
}
