//! Frozen #287 slice 3: an unjournaled brief-failed fact cannot remove a
//! committed worker or suppress the existing undelivered-brief report.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures inspect only their own private tmux server and files"
)]

use super::Rig;
use std::fmt::Write as _;

#[test]
fn stats3_a_failed_brief_fact_append_keeps_the_launched_worker_and_reports_the_gap() {
    let rig = Rig::new("s3brieflost");
    let mut meta = rig.meta();
    writeln!(meta, "session_id=11111111-1111-4111-8111-111111111111").expect("session UUID");
    std::fs::write(rig.dir.join("meta"), meta).expect("fixture meta");
    assert!(
        rig.tmux(&[
            "set-option",
            "-t",
            &format!("={}:", rig.session),
            "@ae_session_uuid",
            "11111111-1111-4111-8111-111111111111",
        ])
        .0
    );
    rig.make_claude_input_busy();
    std::fs::create_dir(rig.dir.join("events.jsonl")).expect("journal append failure");
    let (code, stdout, stderr) = rig.run(
        ae::cli::SPAWN,
        &["busy", "--using", "fake", "--", "stats3 canary"],
    );
    assert_eq!(
        code,
        Some(1),
        "brief remains undelivered: {stdout}\n{stderr}"
    );
    assert!(stderr.contains("SPAWN INCOMPLETE"), "{stderr}");
    assert!(
        stderr.contains("brief-failed"),
        "name the fact that could not be journaled: {stderr}"
    );
    assert!(stderr.contains("journal"), "{stderr}");
    assert!(
        rig.launched.is_file(),
        "the real fixture harness launched despite the journal failure"
    );
    assert!(
        rig.panes()
            .iter()
            .any(|(_, slot, name)| slot == "spawned.0" && name == "busy")
    );
    assert!(
        rig.meta().contains("seat.spawned.0=busy"),
        "the committed seat stays retained"
    );
    assert!(
        rig.dir.join("undelivered.busy.txt").is_file(),
        "the existing recovery artifact remains"
    );
}
