//! Issue #334: a failed respawn must carry tmux's own reason.

use super::{quietwd_spec::Rig, run_tmux_op, tmux_op_failure};
use crate::session_tmux::{Op, argv};

#[test]
fn missing_respawn_pane_reports_tmux_reason() {
    let rig = Rig::new("paneerr");
    let command = vec!["sleep 600".to_owned()];
    let (ok, reason) = run_tmux_op(&argv(
        &rig.server,
        &Op::RespawnPane {
            pane: "%2147483647",
            work_dir: "",
            command: &command,
        },
    ));
    assert!(!ok, "a missing pane cannot be respawned");
    assert!(
        reason.contains("can't find pane"),
        "tmux must name why respawn failed: {reason:?}"
    );
}

#[test]
fn successful_operation_keeps_its_multiline_stdout() {
    let rig = Rig::new("paneerr-success");
    let expected = rig.must(&[
        "capture-pane",
        "-p",
        "-J",
        "-S",
        "-100",
        "-E",
        "-",
        "-t",
        &rig.pane,
    ]);
    assert!(
        expected.lines().count() > 1,
        "the success control has multiple lines"
    );
    let (ok, output) = run_tmux_op(&argv(
        &rig.server,
        &Op::CapturePane {
            pane: &rig.pane,
            lines: 100,
        },
    ));
    assert!(ok);
    assert_eq!(output, expected, "successful stdout must remain intact");
}

#[test]
fn diagnostic_is_one_bounded_unicode_line_without_control_characters() {
    assert_eq!(
        tmux_op_failure(b"can't find pane: %7\nsecond line\n"),
        "can't find pane: %7"
    );
    assert_eq!(tmux_op_failure(b"bad\tvalue\r\nsecond line"), "bad value");
    let long = format!("can't find pane: {}", "界".repeat(400));
    let reason = tmux_op_failure(long.as_bytes());
    assert!(reason.starts_with("can't find pane:"));
    assert!(reason.chars().count() <= 256);
    assert!(!reason.chars().any(char::is_control));
    assert!(reason.ends_with('界'), "a Unicode character is never split");
    assert_eq!(tmux_op_failure(b""), "tmux gave no reason");
}
