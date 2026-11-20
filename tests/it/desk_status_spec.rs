//! Read-only Telegram board status through the normal CLI.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "this acceptance fixture owns the scratch state and starts the real binary"
)]

use std::fs;

#[test]
fn status_surfaces_a_pin_hold_and_observes_operator_repair_without_writing() {
    let home = super::cli::OwnedScratch::root("desk-status", "hold");
    let telegram = home.path().join("telegram");
    fs::create_dir_all(&telegram).expect("scratch bridge directory");
    let path = telegram.join("desk-pin");
    let report = || {
        let out = super::cli::ae()
            .env("AE_HOME", home.path())
            .args(["telegram", "status"])
            .output()
            .expect("telegram status runs");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("status text")
    };
    assert!(
        report()
            .lines()
            .any(|line| line.contains("board:") && line.contains("none yet"))
    );

    let damaged = "ae-desk-pin 1\nchat -1001234567890\nmessage broken\n";
    fs::write(&path, damaged).expect("damaged pin checkpoint");
    let status = report();
    let row = status
        .lines()
        .find(|line| line.contains("board:"))
        .expect("board status row");
    assert!(row.contains("HOLD") && row.contains("repair or remove"));
    assert!(row.contains(&path.display().to_string()));
    assert_eq!(
        fs::read_to_string(&path).expect("preserved checkpoint"),
        damaged
    );

    let repaired = "ae-desk-pin 1\nchat -1001234567890\nmessage 42\npinned 1\n";
    fs::write(&path, repaired).expect("operator repair");
    let status = report();
    assert!(
        status
            .lines()
            .any(|line| line.contains("board:") && line.contains("message 42 pinned"))
    );
    assert!(!status.contains("HOLD"));
    assert_eq!(
        fs::read_to_string(&path).expect("repaired checkpoint"),
        repaired
    );
}
