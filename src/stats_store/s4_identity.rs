//! Frozen S4 identity failure: a durable journal append is a counted refusal.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and contract assertions"
)]

use super::disk::{self, Counted, Refused};
use super::s4_behavior::{Fixture, S, birth, drain, source};
use crate::json::Value;
use std::fs;

#[test]
fn s4_post_append_identity_failure_preserves_the_birth_as_a_counting_gap() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, b"");
    let line = String::from_utf8(birth(S, 1)).expect("valid fixture event with LF");
    let _ = disk::take_file_events();
    let _ = disk::take_payload_reads();
    let _ = disk::take_journal_reads();
    crate::store::identity_fault::arm();
    let counted = crate::store::open(&dir)
        .append_event_counted(&line)
        .expect("a successful durable append must not report a journal failure");
    let Counted::Refused(Refused::Io(ref reason)) = counted else {
        panic!("unproved post-append identity is a counted refusal: {counted:?}");
    };
    assert!(
        reason.contains("journal identity"),
        "named reason: {reason}"
    );
    assert!(!counted.counted());
    assert!(
        counted.reason().is_some(),
        "site 1 has a counting-gap reason"
    );
    assert!(disk::take_file_events().is_empty(), "nothing counted");
    assert!(!root.0.join("stats").exists(), "no payload was published");
    assert_eq!(disk::take_payload_reads().opened, 0);
    assert_eq!(disk::take_journal_reads().total, 0);

    let wire = fs::read(dir.join("events.jsonl")).expect("durable birth remains readable");
    assert!(wire.ends_with(b"\n"), "the stamped record retains its LF");
    assert_eq!(wire.iter().filter(|&&byte| byte == b'\n').count(), 1);
    let text = std::str::from_utf8(&wire).expect("one UTF-8 journal record");
    let actual = crate::json::parse(text.trim_end()).expect("actual stamped object");
    let Value::Obj(expected) = crate::json::parse(line.trim_end()).expect("fixture object") else {
        panic!("fixture is an object");
    };
    for (name, value) in expected {
        assert_eq!(actual.get(&name), Some(&value), "journal field {name} kept");
    }
    assert_eq!(actual.get("schema_version"), Some(&Value::Num(1)));

    // The hook is one-shot. A later birth counts normally, and the maintainer
    // recovers the earlier durable birth from the journal without a duplicate.
    let next = String::from_utf8(birth(S, 2)).expect("second fixture event");
    let counted = crate::store::open(&dir)
        .append_event_counted(&next)
        .expect("the next append also succeeds");
    assert!(matches!(counted, Counted::Published { .. }), "{counted:?}");
    assert!(
        fs::read(dir.join("events.jsonl"))
            .expect("journal kept")
            .starts_with(&wire)
    );
    drain(&mut root.maintainer());
    root.check(|| vec![source(&dir, "alpha")]);
}
