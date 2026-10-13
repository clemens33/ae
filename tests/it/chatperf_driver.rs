//! Driver regressions supplement the independent frozen acceptance.
#![allow(clippy::disallowed_methods, clippy::expect_used, reason = "fixture")]
use super::{
    chatperf_spec::{append, asked, filler, parsed, write},
    tests::Rig,
};
use crate::events::Held;
use std::fs;

#[test]
fn a_follow_primed_proof_is_reused_by_the_runtime_strict_path() {
    let rig = Rig::new("chatperf-follow-strict");
    write(
        &rig,
        format!("{}{}", filler(40), asked(1, "alpha")).as_bytes(),
    );
    let dir = rig.0.join("s");
    let mut held = Held::default();
    held.session(&dir).expect("follow");
    let tail = asked(2, "bravo");
    append(&rig, tail.as_bytes());
    let (strict, cost) = parsed(|| super::snapshot_in(&mut held, &dir));
    assert_eq!(
        strict,
        super::snapshot(crate::store::read_source(&dir.join("events.jsonl")))
    );
    assert_eq!(
        cost,
        tail.len() as u64,
        "the fd and strict path use one file key"
    );
}

#[test]
fn a_body_memo_refuses_a_new_symlink_node_and_a_symlink_directory() {
    let rig = Rig::new("chatperf-body-links");
    let dir = rig.0.join("s");
    let messages = dir.join("messages");
    fs::create_dir(&messages).expect("messages");
    let id = "ae-20260930T060000Z-0000abcd";
    let name = format!("{id}.reply.00000a.txt");
    let file = messages.join(&name);
    fs::write(&file, "body words").expect("body");
    let event = crate::events::Event::parse_line(&format!(
        "{{\"ts\":\"2026-09-30T06:00:00Z\",\"actor\":\"lead\",\"action\":\"reply\",\"ref\":\"{id}\",\"body_file\":\"/x/messages/{name}\"}}"
    )).expect("event");
    let console = rig.console();
    assert_eq!(
        console.body(&event),
        super::lane::Body::Whole("body words".into())
    );
    let target = dir.join("target");
    fs::rename(&file, &target).expect("move the unchanged node");
    std::os::unix::fs::symlink(&target, &file).expect("file link");
    assert_eq!(console.body(&event), super::body_for(&dir, &event));
    fs::remove_file(&file).expect("link");
    fs::rename(&target, &file).expect("restore node");
    assert_eq!(
        console.body(&event),
        super::lane::Body::Whole("body words".into())
    );
    let moved = dir.join("moved");
    fs::rename(&messages, &moved).expect("move directory");
    std::os::unix::fs::symlink(&moved, &messages).expect("directory link");
    assert_eq!(console.body(&event), super::body_for(&dir, &event));
}
