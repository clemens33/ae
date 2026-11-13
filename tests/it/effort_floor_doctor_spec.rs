//! #318: floor rows the policy cannot use must be visible in doctor output.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures inspect only process-owned scratch homes"
)]

fn doctor(tag: &str, row: &str) -> String {
    let scratch = super::cli::OwnedScratch::root("efd", tag);
    for path in ["home", "state/sessions", "work"] {
        std::fs::create_dir_all(scratch.join(path)).expect("owned doctor fixture");
    }
    let config = scratch.join("config");
    std::fs::write(&config, format!("[profiles]\nbase = /bin/sh -c :\nknown = codex --yolo -m gpt-6.1-sol -c model_reasoning_effort=xhigh\n[roster]\nlead = base\n[workspace]\nmain = lead\nworkers = \n[effort_floor]\n{row}\n")).expect("literal floor row and a profile that pins the known model");
    let output = super::cli::ae()
        .args(["doctor", "--global"])
        .arg(config)
        .env("HOME", scratch.join("home"))
        .env("AE_HOME", scratch.join("state"))
        .env_remove("CONFIG_FILE")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .current_dir(scratch.join("work"))
        .output()
        .expect("real doctor");
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn floor_findings(report: &str) -> String {
    report
        .lines()
        .filter(|line| {
            line.contains("effort_floor") && (line.starts_with("WARN") || line.starts_with("FAIL"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn p2_doctor_accepts_a_valid_floor_row_for_a_model_pinned_by_a_profile() {
    let report = doctor("valid", "\"gpt-6.1-sol\" = \"medium\"");
    assert!(
        report.starts_with("ae doctor\n"),
        "doctor actually rendered the report: {report}"
    );
    assert!(
        floor_findings(&report).is_empty(),
        "a valid known-model row has no floor finding: {report}"
    );
}

#[test]
fn p2_doctor_names_an_unknown_model_in_a_floor_row() {
    let report = doctor("model", "\"p2-unknown-model\" = \"medium\"");
    let findings = floor_findings(&report);
    assert!(
        findings.contains("p2-unknown-model"),
        "doctor must identify the unusable model floor row: {report}"
    );
}

#[test]
fn p2_doctor_names_an_unknown_effort_in_a_known_model_floor_row() {
    let report = doctor("level", "\"gpt-6.1-sol\" = \"p2-unknown-effort\"");
    let findings = floor_findings(&report);
    assert!(
        findings.contains("gpt-6.1-sol") && findings.contains("p2-unknown-effort"),
        "doctor must identify the unusable effort value: {report}"
    );
}
