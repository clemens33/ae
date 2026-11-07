//! Frozen S3 API/crash proofs. Installed after baseline runtime RED.
#![allow(clippy::expect_used, reason = "acceptance assertions and fixture IO")]

#[path = "s3_cases.rs"]
mod s3_cases;
use s3_cases as cases;
#[path = "s3_keys.rs"]
mod keys;
use super::s3_behavior::Root;
use crate::stats_store::disk::{
    self, Decision, Dir, Files, Journal, Life, Memory, MemoryJournals, Next, Op, Prepared, Read,
    SourceSpec, Standing, Why, Window,
};
use crate::stats_store::doc;
use cases::{Kind, LIMITS, S, T, birth, check, complete, plan, recover, scenario, seed, work};

fn snapshot(dir: &mut dyn Dir, name: &str) -> Vec<u8> {
    match dir.read(name, disk::EVIDENCE_CAP) {
        Read::Bytes(bytes) => bytes,
        other => panic!("expected {name}: {other:?}"),
    }
}

fn crash_matrix(real: bool, kind: Kind) {
    let mut probe = Memory::new();
    let (publication, _, _) = scenario(&mut probe, &mut MemoryJournals::new(), kind);
    for cut in 0..=publication.ops.len() {
        let root = Root::new();
        let mut journals = MemoryJournals::new();
        if real {
            cases::crash(&mut Files::at(&root.0), &mut journals, kind, cut, None);
        } else {
            cases::crash(&mut Memory::new(), &mut journals, kind, cut, None);
        }
    }
}

#[test]
fn s3_remove_crash_every_op_files_and_memory() {
    crash_matrix(false, Kind::Remove);
    crash_matrix(true, Kind::Remove);
}
#[test]
fn s3_replace_crash_every_op_files_and_memory() {
    crash_matrix(false, Kind::Replace);
    crash_matrix(true, Kind::Replace);
}
#[test]
fn s3_reorigin_crash_every_op_files_and_memory() {
    crash_matrix(false, Kind::Reorigin);
    crash_matrix(true, Kind::Reorigin);
}
#[test]
fn s3_rebind_crash_every_op_files_and_memory() {
    crash_matrix(false, Kind::Rebind);
    crash_matrix(true, Kind::Rebind);
}

#[test]
fn s3_remove_recovery_can_crash_at_every_op() {
    let mut probe = Memory::new();
    let (publication, _, _) = scenario(&mut probe, &mut MemoryJournals::new(), Kind::Remove);
    let commit = publication.ops.iter().position(|op| matches!(op, Op::Rename { to, .. } if to == &format!("stats/session/{S}.evidence"))).expect("primary commit") + 1;
    publication
        .apply(&mut probe, commit)
        .expect("committed removal prefix");
    let redo = disk::recovery(&mut probe).expect("removal recovery");
    for recut in 0..=redo.ops.len() {
        cases::crash(
            &mut Memory::new(),
            &mut MemoryJournals::new(),
            Kind::Remove,
            commit,
            Some(recut),
        );
    }
}

fn prune_fixture(dir: &mut dyn Dir) -> (disk::Plan, Vec<cases::Feed>) {
    let (removal, _, after) = scenario(dir, &mut MemoryJournals::new(), Kind::Remove);
    let intent_removed = removal
        .ops
        .iter()
        .position(|op| matches!(op, Op::Remove(name) if name == "stats/intent"))
        .expect("intent cleanup")
        + 1;
    removal
        .apply(dir, intent_removed)
        .expect("leave empty group files");
    recover(dir);
    check(dir, &after);
    (disk::prune(dir, Some(S)).expect("empty group prune"), after)
}

#[test]
fn s3_prune_leftovers_crash_every_op_files_and_memory() {
    let (probe, _) = prune_fixture(&mut Memory::new());
    assert!(
        !probe.ops.is_empty(),
        "removal interrupted before leftover cleanup"
    );
    for real in [false, true] {
        for cut in 0..=probe.ops.len() {
            let root = Root::new();
            let mut memory = Memory::new();
            let mut files = Files::at(&root.0);
            let dir: &mut dyn Dir = if real { &mut files } else { &mut memory };
            let (pruning, after) = prune_fixture(dir);
            pruning.apply(dir, cut).expect("prune prefix");
            recover(dir);
            let rest = disk::prune(dir, Some(S)).expect("resume prune");
            complete(dir, &rest);
            check(dir, &after);
            assert!(matches!(
                dir.read(&format!("stats/session/{S}.view"), 1),
                Read::Absent
            ));
            assert!(matches!(
                dir.read(&format!("stats/session/{S}.evidence"), 1),
                Read::Absent
            ));
        }
    }
}

#[test]
fn s3_remove_stamp_and_recovery_preconditions_are_enforced() {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let before = seed(&mut dir, &mut journals);
    assert!(matches!(
        disk::remove(
            &mut dir,
            Some(S),
            "live/a",
            Some((u64::MAX, before[0].cursor))
        )
        .expect("remove stamp check"),
        Decision::Discard(Why::Stamp)
    ));
    check(&mut dir, &before);
    let removal = plan(disk::remove(&mut dir, Some(S), "live/a", None).expect("remove plan"));
    let intent = removal
        .ops
        .iter()
        .position(|op| matches!(op, Op::Rename { to, .. } if to == "stats/intent"))
        .expect("intent")
        + 1;
    removal.apply(&mut dir, intent).expect("unrecovered intent");
    assert_eq!(
        disk::remove(&mut dir, Some(S), "live/a", None),
        Err(disk::Refused::RecoveryOwed)
    );
    assert_eq!(
        disk::prune(&mut dir, Some(S)),
        Err(disk::Refused::RecoveryOwed)
    );
    recover(&mut dir);
    check(&mut dir, &before);
}

struct Counted {
    inner: MemoryJournals,
    bytes: u64,
}
impl Journal for Counted {
    fn window(
        &mut self,
        source: &str,
        from: u64,
        max: u64,
    ) -> Result<Window, crate::stats::SourceGap> {
        let window = self.inner.window(source, from, max)?;
        self.bytes += window.bytes.len() as u64;
        Ok(window)
    }
    fn standing(&mut self, source: &str) -> Standing {
        self.inner.standing(source)
    }
}

#[test]
fn s3_rebind_preserves_cursor_and_rebinds_identity_without_full_replay() {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let mut feeds = seed(&mut dir, &mut journals);
    // A donor larger than the witness makes full replay distinguishable from copying.
    feeds[0].bytes.extend([b' '; 8192]);
    feeds[0].bytes.push(b'\n');
    journals.set(feeds[0].id(), 1, 20, &feeds[0].bytes);
    loop {
        let next = Prepared::read(
            &mut dir,
            &mut journals,
            &feeds[0].spec,
            disk::Limits {
                chunk: 16384,
                line: 16384,
            },
            None,
        )
        .expect("grow donor");
        let Next::Work(prepared) = next else {
            panic!("donor growth");
        };
        let update = plan(disk::publish(&mut dir, &mut journals, &prepared).expect("donor update"));
        complete(&mut dir, &update);
        if prepared.span().1 == feeds[0].bytes.len() as u64 {
            break;
        }
    }
    feeds[0].cursor = feeds[0].bytes.len() as u64;
    let mut copy = feeds[0].clone();
    copy.spec = SourceSpec::Archive {
        id: format!("archive/{S}"),
        uuid: S.to_owned(),
    };
    copy.ino = 123;
    journals.set(copy.id(), 9, copy.ino, &copy.bytes);
    let mut counted = Counted {
        inner: journals,
        bytes: 0,
    };
    let observed = Prepared::read_life(
        &mut dir,
        &mut counted,
        &copy.spec,
        LIMITS,
        None,
        None,
        &Life {
            fingerprint: Some("b".repeat(64)),
            donors: vec!["live/a".to_owned()],
        },
    )
    .expect("donor preparation");
    let Next::Work(prepared) = observed.next else {
        panic!("rebind work");
    };
    assert_eq!(prepared.donor(), Some("live/a"));
    assert_eq!(prepared.span().1, feeds[0].cursor);
    assert!(
        counted.bytes <= 4096,
        "rebind reads the witness, not the history: {}",
        counted.bytes
    );
    counted.bytes = 0;
    let publication =
        plan(disk::publish(&mut dir, &mut counted, &prepared).expect("rebind commit"));
    complete(&mut dir, &publication);
    assert!(counted.bytes <= 4096, "commit rechecks only the witness");
    let view = snapshot(&mut dir, &format!("stats/session/{S}.view"));
    let view =
        doc::parse_session(std::str::from_utf8(&view).expect("view text")).expect("session view");
    let row = view.rows.get(copy.id()).expect("rebound source row");
    assert_eq!(row.cursor, feeds[0].cursor);
    assert_eq!((row.provenance.dev, row.provenance.ino), (9, 123));
    feeds.push(copy);
    check(&mut dir, &feeds);
}

#[test]
fn s3_mismatching_donor_is_not_copied_and_reads_actual_new_journal() {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let mut feeds = seed(&mut dir, &mut journals);
    let mut copy = feeds[0].clone();
    copy.spec = SourceSpec::Archive {
        id: format!("archive/{S}"),
        uuid: S.to_owned(),
    };
    copy.bytes = [birth(S, 7), birth(T, 8)].concat();
    copy.cursor = copy.bytes.len() as u64;
    journals.set(copy.id(), 1, 123, &copy.bytes);
    let prepared = work(
        &mut dir,
        &mut journals,
        &copy.spec,
        &Life {
            fingerprint: None,
            donors: vec!["live/a".to_owned()],
        },
    );
    assert_eq!(
        prepared.donor(),
        None,
        "readable opposed witness must reject the donor"
    );
    let publication = plan(
        disk::publish(&mut dir, &mut journals, &prepared).expect("ordinary fresh registration"),
    );
    complete(&mut dir, &publication);
    feeds.push(copy);
    check(&mut dir, &feeds);
}

#[test]
fn s3_donor_change_between_prepare_and_commit_discards_rebind() {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let mut feeds = seed(&mut dir, &mut journals);
    let copy = SourceSpec::Archive {
        id: format!("archive/{S}"),
        uuid: S.to_owned(),
    };
    journals.set(copy.id(), 1, 123, &feeds[0].bytes);
    let stale = work(
        &mut dir,
        &mut journals,
        &copy,
        &Life {
            fingerprint: None,
            donors: vec!["live/a".to_owned()],
        },
    );
    assert_eq!(stale.donor(), Some("live/a"));
    feeds[0].bytes.extend(birth(S, 5));
    feeds[0].cursor = feeds[0].bytes.len() as u64;
    journals.set("live/a", 1, 20, &feeds[0].bytes);
    let newer = work(&mut dir, &mut journals, &feeds[0].spec, &Life::default());
    let publication = plan(disk::publish(&mut dir, &mut journals, &newer).expect("donor advances"));
    complete(&mut dir, &publication);
    assert!(matches!(
        disk::publish(&mut dir, &mut journals, &stale).expect("donor stamp"),
        Decision::Discard(Why::Stamp)
    ));
    check(&mut dir, &feeds);
}

#[test]
fn s3_reorigin_invalidates_stale_chunk_and_stale_reorigin() {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let mut feeds = seed(&mut dir, &mut journals);
    feeds[0].bytes.extend(birth(S, 6));
    journals.set("live/a", 1, 20, &feeds[0].bytes);
    let chunk = work(&mut dir, &mut journals, &feeds[0].spec, &Life::default());
    let spec = SourceSpec::Live {
        id: "live/a".to_owned(),
        label: "renamed".to_owned(),
        meta: format!("session_id={S}\nseat.spawned.9=unborn\n").into_bytes(),
    };
    let a = work(&mut dir, &mut journals, &spec, &Life::default());
    let b = work(&mut dir, &mut journals, &spec, &Life::default());
    let publication = plan(disk::publish(&mut dir, &mut journals, &a).expect("reorigin"));
    complete(&mut dir, &publication);
    feeds[0].spec = spec;
    assert!(matches!(
        disk::publish(&mut dir, &mut journals, &chunk).expect("stale chunk"),
        Decision::Discard(Why::Stamp)
    ));
    assert!(matches!(
        disk::publish(&mut dir, &mut journals, &b).expect("stale reorigin"),
        Decision::Discard(Why::Stamp)
    ));
    check(&mut dir, &feeds); // Only the old consumed prefix, never the unread new birth.
}

#[test]
fn s3_standing_rejects_live_and_failed_registration_without_a_witness() {
    for failed in [false, true] {
        let mut dir = Memory::new();
        let mut journals = MemoryJournals::new();
        let feeds = seed(&mut dir, &mut journals);
        let spec = if failed {
            SourceSpec::Failed {
                id: "live/new".to_owned(),
                label: "new".to_owned(),
                session: Some(S.to_owned()),
                gap: crate::stats::SourceGap::Missing,
            }
        } else {
            journals.set("live/new", 1, 55, &birth(S, 4));
            SourceSpec::Live {
                id: "live/new".to_owned(),
                label: "new".to_owned(),
                meta: format!("session_id={S}\n").into_bytes(),
            }
        };
        let stale = work(&mut dir, &mut journals, &spec, &Life::default());
        journals.stand("live/new", Standing::Gone);
        assert!(matches!(
            disk::publish(&mut dir, &mut journals, &stale).expect("standing under lock"),
            Decision::Discard(Why::Gone)
        ));
        check(&mut dir, &feeds);
    }
}

#[test]
fn s3_seeded_lifecycle_fuzz_arms_are_live() {
    for operation in 0..6 {
        for cut in 0..48 {
            cases::fuzz_case(&[operation, cut, cut]);
        }
    }
}

#[test]
fn s3_additive_keys_accept_s2_and_refuse_hostile_new_shapes() {
    for mode in 0..12 {
        keys::exercise(&[mode]);
    }
}

#[test]
fn s3_unindexed_s2_root_is_resummed_before_ownership_proof() {
    for failed in [false, true] {
        let mut dir = Memory::new();
        let mut journals = MemoryJournals::new();
        let old = if failed {
            SourceSpec::Failed {
                id: "live/moved".to_owned(),
                label: "moved".to_owned(),
                session: Some(S.to_owned()),
                gap: crate::stats::SourceGap::Missing,
            }
        } else {
            SourceSpec::Live {
                id: "live/moved".to_owned(),
                label: "moved".to_owned(),
                meta: format!("session_id={S}\n").into_bytes(),
            }
        };
        journals.set("live/moved", 1, 44, &birth(S, 7));
        let stale = work(&mut dir, &mut journals, &old, &Life::default());
        let spec = if failed {
            SourceSpec::Failed {
                id: "live/moved".to_owned(),
                label: "moved".to_owned(),
                session: Some(T.to_owned()),
                gap: crate::stats::SourceGap::Missing,
            }
        } else {
            SourceSpec::Live {
                id: "live/moved".to_owned(),
                label: "moved".to_owned(),
                meta: format!("session_id={T}\n").into_bytes(),
            }
        };
        let newer = work(&mut dir, &mut journals, &spec, &Life::default());
        let publication = plan(disk::publish(&mut dir, &mut journals, &newer).expect("new owner"));
        complete(&mut dir, &publication);
        let root = snapshot(&mut dir, "stats/view");
        dir.plant(
            "stats/view",
            &keys::field(&root, "{\"group\":", "ids", None),
        );
        assert!(matches!(
            disk::publish(&mut dir, &mut journals, &stale).expect("S2 index reconstruction"),
            Decision::Discard(Why::Owned)
        ));
        let mut feed = cases::Feed::live("live/moved", Some(T), birth(S, 7));
        feed.spec = spec;
        feed.active = true;
        feed.cursor = if failed { 0 } else { feed.bytes.len() as u64 };
        check(&mut dir, &[feed]);
    }
}

#[test]
fn s3_valid_archive_tree_edits_revalidate_but_quiet_rounds_do_not() {
    use super::s3_behavior::{assert_valid, drain};
    let root = Root::new();
    let path = root.archive(S, &birth(S, 1));
    let mut maintainer = root.maintainer();
    let _ = crate::stats_store::maint::take_validations();
    drain(&mut maintainer);
    assert!(
        crate::stats_store::maint::take_validations()
            .iter()
            .any(|id| id == S)
    );
    drain(&mut maintainer);
    assert!(
        crate::stats_store::maint::take_validations().is_empty(),
        "same full fingerprint reuses validation"
    );
    let events = std::fs::read(path.join("events.jsonl")).expect("unchanged journal");
    for (node, added) in [
        ("digest.md", "\nvalidated extra prose\n"),
        ("retained/nested/state", "more retained bytes"),
    ] {
        let before = crate::archive::tree_identity(&path).expect("old tree identity");
        let target = path.join(node);
        let mut bytes = std::fs::read(&target).expect("existing judged node");
        bytes.extend_from_slice(added.as_bytes());
        std::fs::write(target, bytes).expect("valid tree edit");
        assert_valid(&path, S);
        assert_ne!(
            crate::archive::tree_identity(&path).expect("new identity"),
            before
        );
        drain(&mut maintainer);
        assert!(
            crate::stats_store::maint::take_validations()
                .iter()
                .any(|id| id == S),
            "valid changed node {node} triggers validation"
        );
        assert_eq!(
            std::fs::read(path.join("events.jsonl")).expect("journal"),
            events
        );
        let expected = crate::stats::fold(
            vec![crate::stats::Source::archive(S, Ok(events.clone()))],
            Some(S),
        );
        let disk::Shown::Ready { view, .. } = disk::shown(&mut Files::at(&root.0), Some(S)) else {
            panic!("valid archive stays counted");
        };
        assert_eq!(view, expected);
    }
}
