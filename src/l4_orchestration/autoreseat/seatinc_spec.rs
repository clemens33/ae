//! Independent #218 2c auto-reseat caller acceptance. Wire under leg.rs.
//! These tests observe the environment at the real `transport::deliver` door.
//! Helper-side refusal, byte preservation and audit behavior are established
//! by seatinc's real Cycle tests and the shipped seatproof residual tests.
#![allow(
    clippy::disallowed_methods,
    reason = "owned fixture directories and external helper"
)]

use super::*;
use std::os::unix::fs::PermissionsExt as _;
use std::sync::atomic::{AtomicU64, Ordering};

struct Rig(PathBuf);

impl Rig {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-seatinc-auto-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("owned auto fixture");
        // An external send stub records what the caller handed it. It never
        // implements the incarnation proof and cannot manufacture its result.
        let script = format!(
            r#"#!/bin/sh
printf '%s\t%s\t%s\t%s\t%s\n' "$1" "$_AE_EVENT_ACTION" "$_AE_EXPECT_SLOT" "$_AE_EXPECT_LAUNCH" "$_AE_EXPECT_LEGACY" >> '{}/captured'
exit 0
"#,
            path.display()
        );
        std::fs::write(path.join("send"), script).expect("owned send fixture");
        std::fs::set_permissions(path.join("send"), std::fs::Permissions::from_mode(0o755))
            .expect("owned mode");
        Self(path)
    }

    fn meta(&self, launch: &str) -> Vec<u8> {
        let bytes = format!("session=current\nsession_id=11111111-1111-4111-8111-111111111111\nseat.main=lead\nlaunch_id.main=lead-decided\nseat.worker.0=colead\nlaunch_id.worker.0=colead-decided\nseat.spawned.0=moved\nlaunch_id.spawned.0={launch}\nseat.spawned.1=parent\nlaunch_id.spawned.1=parent-decided\n").into_bytes();
        std::fs::write(self.0.join("meta"), &bytes).expect("owned meta");
        bytes
    }

    fn argv(&self) -> Argv {
        Argv {
            session: "current".into(),
            dir: self.0.clone(),
            slot: "spawned.0".into(),
            key: Timestamp::now(),
        }
    }

    fn captured(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(self.0.join("captured"))
            .unwrap_or_default()
            .lines()
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn seatinc_continuation_binds_the_minted_incarnation_for_carried_and_kept() {
    for fate in [super::super::Fate::Carried, super::super::Fate::Kept] {
        let rig = Rig::new();
        rig.meta("successor");
        let _ = nudge(&rig.argv(), "moved", fate, Some("minted-by-move"));
        let rows = rig.captured();
        assert_eq!(rows.len(), 1, "one continuation turn: {rows:?}");
        assert_eq!(
            rows[0],
            [
                "moved",
                super::super::NOTICE_ACTION,
                "spawned.0",
                "minted-by-move",
                ""
            ]
        );
    }
}

#[test]
fn seatinc_tell_binds_each_recipient_from_the_original_snapshot_even_after_a_move() {
    for replaced in [false, true] {
        let rig = Rig::new();
        let bytes = rig.meta("moved-decided");
        let meta = crate::meta::Meta::parse(std::str::from_utf8(&bytes).expect("valid fixture"));
        if replaced {
            // The detached leg already chose the recipients from bytes. Its
            // own move takes time, during which other seats can be replaced.
            let current = String::from_utf8(bytes.clone())
                .expect("valid fixture")
                .replace("-decided", "-successor");
            std::fs::write(rig.0.join("meta"), current).expect("current successors");
        }
        tell(
            &rig.argv(),
            Timestamp::now(),
            (meta.roster(), true, Some("parent"), &bytes),
            "ending notice",
        );
        let rows = rig.captured();
        assert_eq!(
            rows.len(),
            3,
            "lead pair and spawner, moved seat excluded: {rows:?}"
        );
        for (name, slot, launch) in [
            ("lead", "main", "lead-decided"),
            ("colead", "worker.0", "colead-decided"),
            ("parent", "spawned.1", "parent-decided"),
        ] {
            let row = rows
                .iter()
                .find(|row| row[0] == name)
                .expect("decided recipient");
            assert_eq!(
                (&*row[1], &*row[2], &*row[3], &*row[4]),
                (super::super::NOTICE_ACTION, slot, launch, ""),
                "old decision must never adopt current launch: {row:?}"
            );
        }
    }
}

#[test]
fn seatinc_continuation_missing_minted_token_refuses_instead_of_claiming_legacy() {
    for (label, minted) in [("absent", None), ("empty", Some(""))] {
        let rig = Rig::new();
        rig.meta("successor");
        let result = nudge(&rig.argv(), "moved", super::super::Fate::Kept, minted);
        assert_eq!(result, super::super::Nudge::Refused, "{label}");
        let rows = rig.captured();
        assert!(
            rows.is_empty(),
            "{label}: damaged snapshot must refuse before spawning: {rows:?}"
        );
        let events = crate::watchdog_daemon::read_events(&rig.0);
        assert_eq!(
            events
                .iter()
                .filter(|event| event.action == "refused")
                .count(),
            1,
            "{label}: one refusal: {events:?}"
        );
        let summary = events
            .iter()
            .find(|event| event.action == "refused")
            .and_then(|event| event.summary.as_deref())
            .unwrap_or_default();
        assert!(
            summary.contains("seat moved (spawned.0) could not be proven")
                && summary
                    .contains("the move recorded no launch token; NOTHING further was typed."),
            "{label}: correct refused seat: {summary}"
        );
        assert!(
            events
                .iter()
                .filter(|event| event.action == "refused")
                .all(|event| event.actor == WATCHDOG_ACTOR)
        );
    }
}
