//! Frozen O8 repair oracle: persisted originals copy byte for byte.
#![allow(
    clippy::disallowed_methods,
    reason = "test owns and reads isolated ledger fixtures"
)]

use super::{Verb, find, repair, write_new};
use crate::events::SessionKey;
use crate::store;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const U: &str = "11111111-1111-4111-8111-111111111111";
const V: &str = "22222222-2222-4222-8222-222222222222";
const ID: &str = "o8-repair";

struct Rig(PathBuf);
impl Rig {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "ae-o8-repair-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        for (name, uuid) in [("one", U), ("two", V)] {
            let dir = root.join(name);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(
                dir.join("meta"),
                format!("session={name}\nsession_id={uuid}\n"),
            )
            .unwrap();
        }
        Self(root)
    }
    fn dir(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn opening(suffix: &str) -> String {
    format!(
        r#"{{"ts":"2026-10-28T12:00:00Z","actor":"lead","action":"ask","target":"helper","ref":"{ID}","actor_slot":"main","actor_session":"one","actor_session_id":"{U}","actor_launch_id":"a","target_slot":"spawned.0","target_session":"two","target_session_id":"{V}","target_launch_id":"b"{suffix}}}"#
    )
}
fn closing(verb: Verb, suffix: &str) -> String {
    let (actor, slot, session, uuid, action) = match verb {
        Verb::Accept => ("helper", "spawned.0", "two", V, "accept"),
        Verb::Withdraw => ("lead", "main", "one", U, "cancel"),
    };
    let (target, target_slot, target_session, target_uuid) = match verb {
        Verb::Accept => ("lead", "main", "one", U),
        Verb::Withdraw => ("helper", "spawned.0", "two", V),
    };
    format!(
        r#"{{"ts":"2026-10-28T12:00:01Z","actor":"{actor}","action":"{action}","ref":"{ID}","actor_slot":"{slot}","actor_session":"{session}","actor_session_id":"{uuid}","target":"{target}","target_slot":"{target_slot}","target_session":"{target_session}","target_session_id":"{target_uuid}","summary":"original α"{suffix}}}"#
    )
}
fn plant(dir: &Path, text: &str) {
    std::fs::write(store::open(dir).events_path(), text).unwrap();
}
fn bytes(dir: &Path) -> Vec<u8> {
    std::fs::read(store::open(dir).events_path()).unwrap()
}

#[test]
fn strict_watchdog_ledger_preserves_version_metadata_as_inert_data() {
    for suffix in [
        "",
        ",\"schema_version\":1",
        ",\"schema_version\":null",
        ",\"schema_version\":1,\"schema_version\":2",
    ] {
        let journal = format!("{}\n", opening(suffix));
        let events =
            crate::autotrust::ledger(store::SourceRead::Ready(journal.into_bytes())).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "ask");
        assert_eq!(events[0].reference.as_deref(), Some(ID));
    }
}

#[test]
fn both_verbs_repair_original_version_bytes_without_new_facts_or_repeat_writes() {
    for verb in [Verb::Accept, Verb::Withdraw] {
        for suffix in [
            "",
            ",\"schema_version\":0",
            ",\"schema_version\":1",
            ",\"schema_version\":null",
            ",\"schema_version\":1,\"schema_version\":2",
        ] {
            for (source_version, target_version) in
                [("", ",\"schema_version\":1"), (",\"schema_version\":1", "")]
            {
                let rig = Rig::new();
                let source = rig.dir("one");
                let target = rig.dir("two");
                let original = closing(verb, suffix);
                let source_log = format!("{}\n{original}\n", opening(source_version));
                let target_log = format!("{}\n", opening(target_version));
                plant(&source, &source_log);
                plant(&target, &target_log);
                let request = find(source_log.as_bytes(), SessionKey::new("one", U), ID).unwrap();
                assert!(
                    super::took(verb, &request),
                    "fixture names the already-recorded outcome"
                );
                let mut out = Vec::new();
                let mut err = Vec::new();
                assert_eq!(
                    repair(
                        verb,
                        &request,
                        original.as_bytes(),
                        Some(("two".into(), target.clone())),
                        &mut out,
                        &mut err
                    )
                    .unwrap(),
                    0,
                    "{err:?}"
                );
                assert_eq!(bytes(&source), source_log.as_bytes());
                let expected = format!("{target_log}{original}\n");
                assert_eq!(
                    bytes(&target),
                    expected.as_bytes(),
                    "repair preserves every original byte"
                );
                let target_read = crate::requests::states(&bytes(&target));
                assert!(super::took(verb, &target_read[0]));
                repair(
                    verb,
                    &request,
                    original.as_bytes(),
                    Some(("two".into(), target.clone())),
                    &mut out,
                    &mut err,
                )
                .unwrap();
                assert_eq!(
                    bytes(&target),
                    expected.as_bytes(),
                    "retry never adds a second record"
                );
                assert_eq!(bytes(&source), source_log.as_bytes());
                // The admission exemption remains byte-exact after the copy.
                assert!(
                    crate::requests::bound_opening(
                        [&bytes(&source), &bytes(&target)],
                        ID,
                        Some(original.as_bytes())
                    )
                    .is_ok()
                );
            }
        }
    }
}

#[test]
fn new_mirrored_records_are_versioned_identically_and_existing_openings_stay_verbatim() {
    for verb in [Verb::Accept, Verb::Withdraw] {
        let rig = Rig::new();
        let source = rig.dir("one");
        let target = rig.dir("two");
        let source_log = format!("{}\n", opening(""));
        let target_log = format!("{}\n", opening(",\"schema_version\":1"));
        plant(&source, &source_log);
        plant(&target, &target_log);
        let request = find(source_log.as_bytes(), SessionKey::new("one", U), ID).unwrap();
        let composed = closing(verb, "") + "\n";
        let mut out = Vec::new();
        let mut err = Vec::new();
        assert_eq!(
            write_new(
                verb,
                &store::open(&source),
                SessionKey::new("one", U),
                &request,
                &composed,
                Some(("two".into(), target.clone())),
                &mut out,
                &mut err
            )
            .unwrap(),
            0,
            "{err:?}"
        );
        let expected = closing(verb, ",\"schema_version\":1") + "\n";
        assert_eq!(bytes(&source), format!("{source_log}{expected}").as_bytes());
        assert_eq!(bytes(&target), format!("{target_log}{expected}").as_bytes());
    }
}
