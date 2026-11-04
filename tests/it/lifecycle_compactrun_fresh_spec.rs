//! Frozen fresh-request addendum on base 550c4cd7.
//! Oracle: architecture.md's stored memo byte offset; commands.md's timeout
//! leaves the request open and stops/archives nothing. Exact stderr is a base pin.

use super::*;

#[test]
fn compactrun_fresh_request_preserves_nonzero_memo_baseline_and_stays_open_on_timeout() {
    let rig = Rig::new("crfresh");
    // A harmless real foreground process receives the real tracked delivery.
    // It never sends a reply or writes a memo; no delivery behavior is mocked.
    assert!(
        rig.tmux(&["respawn-pane", "-k", "-t", &rig.pane, "cat"]).0,
        "start fixture receiver"
    );
    assert!(
        rig.tmux(&["set-option", "-p", "-t", &rig.pane, "@ae_slot", "main"])
            .0,
        "stamp fixture receiver"
    );
    let deadline = Instant::now() + SETUP_WAIT;
    loop {
        let (ok, command) = rig.tmux(&[
            "display-message",
            "-p",
            "-t",
            &rig.pane,
            "#{pane_current_command}",
        ]);
        if ok && command.trim() == "cat" {
            break;
        }
        assert!(Instant::now() < deadline, "receiver never ready: {command}");
        std::thread::sleep(Duration::from_millis(10));
    }
    let memo = "2026-08-29T00:00:00Z\tfake:lead\thandover\told handover\n\
                2026-08-29T00:01:00Z\tfake:lead\tnote\told note\n";
    std::fs::write(rig.dir.join("memo.tsv"), memo).expect("nonempty old memo");
    let meta = std::fs::read(rig.dir.join("meta")).expect("original meta");
    assert!(
        !rig.dir.join("events.jsonl").exists(),
        "fresh request fixture starts without a ledger"
    );
    let output = bounded(
        ae().env("AE_HOME", &rig.home)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("AE_COMPACT_HANDOVER_SECS", "1")
            .args(["_compact", "-f", &rig.name])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("start fresh semantic reboot"),
        Duration::from_secs(15),
    )
    .expect("bounded handover timeout returns");
    let err = String::from_utf8_lossy(&output.stderr);
    let ledger = std::fs::read(rig.dir.join("events.jsonl")).expect("delivered request ledger");
    let requests = ae::requests::states(&ledger);
    assert_eq!(requests.len(), 1, "one fresh request: {err}");
    let request = &requests[0];
    let reference = std::str::from_utf8(&request.id).expect("request reference");
    assert!(
        err.contains(&format!(
            "reboot: handover requested as {reference}; waiting for the reply AND a new handover memo.\n"
        )),
        "fresh delivery reaches the handover wait: {err}"
    );
    let body_path = std::str::from_utf8(&request.body_file).expect("stored body path");
    let body = std::fs::read_to_string(body_path).expect("durable request body");
    assert!(
        body.lines()
            .any(|line| line == format!("AE-COMPACT-MEMO-BASELINE={}", memo.len())),
        "fresh request carries the pre-delivery memo byte offset: {body}"
    );
    assert_eq!(output.status.code(), Some(1), "handover timeout: {err}");
    assert!(output.stdout.is_empty(), "timeout never crosses boundary");
    assert!(
        err.contains("reboot: no complete handover within 1s — a reply AND a new handover memo are both required.\n"),
        "timeout follows successful fresh delivery: {err}"
    );
    assert_eq!(request.status, ae::requests::Status::Pending);
    assert_eq!(
        std::fs::read(rig.dir.join("meta")).expect("meta survives"),
        meta
    );
    assert_eq!(
        std::fs::read_to_string(rig.dir.join("memo.tsv")).expect("memo survives"),
        memo
    );
    assert!(rig.session_is_live(), "source remains live after timeout");
    assert!(!rig.archive().exists(), "timeout never archives source");
}
