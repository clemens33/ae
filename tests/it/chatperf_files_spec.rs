//! Frozen chatperf acceptance for the app's kept journals (lead rulings
//! 2026-10-13): an append is parsed alone; an in-place rewrite, a shrink or a
//! replacement is read whole; every answer equals today's whole read, the
//! pending requests re-derived with the session key the meta records NOW.
#![allow(clippy::disallowed_methods, clippy::expect_used, reason = "fixture")]
use super::{Files, ID, Root, append, event, length, same_files, seed};
use crate::session::RecordSnapshot;
use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;

fn ino(path: &Path) -> u64 {
    fs::metadata(path).expect("journal").ino()
}

/// About 640 KB of complete records none of the pins below quote.
fn pad() -> Vec<u8> {
    let words = "z".repeat(520);
    (0..1000)
        .flat_map(|n| event(&format!("pad{n:04}{words}")))
        .collect()
}

pub(in crate::app) fn file_proof<F: Files>(make: impl Fn() -> F) {
    let root = Root::new("chatperf-proof");
    let dir = seed(&root, "api");
    let journal = dir.join("events.jsonl");
    let pad = pad();
    let middle = [pad.as_slice(), &event("alpha"), &pad, &event("bravo")].concat();
    fs::write(&journal, &middle).expect("a long journal");
    let mut subject = make();
    same_files(&mut subject, &dir);
    let cold = subject.costs();
    assert_eq!(cold, length(&journal), "the first read is whole");
    let next = event("charlie");
    append(&journal, &next);
    same_files(&mut subject, &dir);
    assert_eq!(
        subject.costs(),
        cold + next.len() as u64,
        "an append parses its own bytes, no others"
    );
    let at = ino(&journal);
    let edited = [
        pad.as_slice(),
        &event("ALPHA"),
        &pad,
        &event("bravo"),
        &next,
        &event("delta"),
    ]
    .concat();
    assert_eq!(event("ALPHA").len(), event("alpha").len());
    let before = subject.costs();
    fs::write(&journal, &edited).expect("rewrite in place");
    assert_eq!(ino(&journal), at, "the same file");
    same_files(&mut subject, &dir);
    assert_eq!(
        subject.costs(),
        before + length(&journal),
        "a rewrite in place that grows is read whole"
    );
    let staged = dir.join("staged");
    fs::write(&staged, [edited.as_slice(), &event("echo")].concat()).expect("staged");
    fs::rename(&staged, &journal).expect("replace");
    assert_ne!(ino(&journal), at, "a new file");
    let before = subject.costs();
    same_files(&mut subject, &dir);
    assert_eq!(
        subject.costs(),
        before + length(&journal),
        "a replaced journal is read whole, even one that only grew"
    );
    fs::write(&journal, event("tiny")).expect("shrink in place");
    let before = subject.costs();
    same_files(&mut subject, &dir);
    assert_eq!(
        subject.costs(),
        before + length(&journal),
        "a shrunk journal is read whole"
    );
}

/// A kept journal's pending requests are folded with the key the meta holds
/// at each read: a new `session_id` reopens what a retire closed for the old.
pub(in crate::app) fn file_session_key<F: Files>(make: impl Fn() -> F) {
    let root = Root::new("chatperf-key");
    let dir = seed(&root, "api");
    let ask = format!(
        "{{\"ts\":\"2026-01-01T00:00:00Z\",\"actor\":\"lead\",\"action\":\"ask\",\"target\":\"colead\",\"ref\":\"ae-20260101T000000Z-0000abcd\",\"actor_slot\":\"main\",\"actor_session\":\"api\",\"actor_session_id\":\"{ID}\",\"target_slot\":\"worker.0\",\"target_session\":\"api\",\"target_session_id\":\"{ID}\",\"summary\":\"q\"}}\n"
    );
    let retire = "{\"ts\":\"2026-01-01T00:01:00Z\",\"actor\":\"lead\",\"action\":\"retire\",\"target\":\"colead\",\"target_slot\":\"worker.0\"}\n";
    fs::write(dir.join("events.jsonl"), format!("{ask}{retire}")).expect("journal");
    let pending = |dir: &Path| {
        RecordSnapshot::read(dir)
            .events
            .expect("readable")
            .pending
            .len()
    };
    let mut subject = make();
    same_files(&mut subject, &dir);
    assert_eq!(pending(&dir), 0, "the retire closed the ask");
    let other = ID.replace("1234", "9876");
    fs::write(
        dir.join("meta"),
        format!(
            "schema=2\nsession_id={other}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\n"
        ),
    )
    .expect("a new session_id, the journal untouched");
    assert_eq!(pending(&dir), 1, "today's whole read reopens it");
    same_files(&mut subject, &dir);
}
