//! Status exposes the two checkpoints between no message and a pinned message.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the acceptance test starts the CLI and owns its scratch state"
)]

use std::fs;

#[test]
fn status_distinguishes_a_chat_without_a_message_from_a_message_pending_pin() {
    let home = super::cli::OwnedScratch::root("desk-status", "pending");
    let dir = home.path().join("telegram");
    fs::create_dir_all(&dir).expect("scratch bridge state");
    let checkpoint = dir.join("desk-pin");
    for (bytes, expected) in [
        (
            "ae-desk-pin 1\nchat -1001234567890\n",
            "no message yet (chat -1001234567890)",
        ),
        (
            "ae-desk-pin 1\nchat -1001234567890\nmessage 42\npinned 0\n",
            "message 42 sent, pin pending",
        ),
    ] {
        fs::write(&checkpoint, bytes).expect("prepared checkpoint");
        let out = super::cli::ae()
            .env("AE_HOME", home.path())
            .args(["telegram", "status"])
            .output()
            .expect("read-only status runs");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let output = String::from_utf8(out.stdout).expect("status text");
        let row = output
            .lines()
            .find(|line| line.contains("board:"))
            .expect("board row");
        assert!(row.ends_with(expected), "{row:?}");
        assert_eq!(fs::read_to_string(&checkpoint).expect("state bytes"), bytes);
    }
}
