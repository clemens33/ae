//! Batch-1 regressions from F1-F10 and lead amendments 11-15.
//! Tests use the real decision pipeline; damaged wire bytes are fixture inputs.

#![allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "persisted payload temp names have the exact case-sensitive .tmp suffix"
)]

use super::{
    Decision, Dir, Duration, Feed, Files, Limits, Maintainer, Memory, MemoryJournals, Next, Op,
    Ordering, Outcome, Payload, Prepared, Read, Refused, Root, S, Shown, SourceSpec, T, Why, birth,
    check, complete, disk, epoch, full, limits, pending_foreign, primary_index, published, recover,
    run_to_idle, seed, view,
};
use std::collections::BTreeSet;
use std::io;
use std::os::unix::fs::MetadataExt as _;

#[test]
fn s2_f1_files_sync_every_staged_write_before_intent_publication() {
    use disk::FileEvent;
    let root = Root::new();
    let mut files = Files::at(&root.0);
    let (mut journals, mut feeds) = seed(&mut files);
    let work = pending_foreign(&mut files, &mut journals, &mut feeds);
    let plan =
        published(disk::publish(&mut files, &mut journals, &work).expect("real publish plan"));
    let _ = disk::take_file_events();
    complete(&plan, &mut files);
    let events = disk::take_file_events();
    let intent = events.iter().position(|event| matches!(event, FileEvent::Renamed { from, to } if from == "stats/intent.tmp" && to == "stats/intent"))
        .expect("real intent commit rename");
    let mut writes = 0;
    for (index, event) in events.iter().enumerate() {
        if let FileEvent::Created(name) = event {
            writes += 1;
            assert!(
                matches!(events.get(index + 1), Some(FileEvent::Synced { name: synced, .. }) if synced == name),
                "actual file sync follows create for {name}"
            );
        }
        if let FileEvent::Synced { .. } = event {
            assert!(
                index < intent,
                "every staged sync finishes before intent commit"
            );
        }
    }
    assert_eq!(
        writes,
        plan.ops
            .iter()
            .filter(|op| matches!(op, Op::Write { .. }))
            .count()
    );
    assert!(writes > 1, "multi-group staging exercised");
}

#[test]
fn s2_f1_files_sync_failure_stops_before_intent_and_preserves_answer() {
    let root = Root::new();
    let mut files = Files::at(&root.0);
    let (mut journals, mut feeds) = seed(&mut files);
    let before = view(&mut files, None);
    let work = pending_foreign(&mut files, &mut journals, &mut feeds);
    let plan = published(disk::publish(&mut files, &mut journals, &work).expect("publish plan"));
    let _ = disk::take_file_events();
    disk::fail_next_sync("stats/view.tmp");
    let result = plan.apply(&mut files, plan.ops.len());
    let events = disk::take_file_events();
    assert!(
        result.is_err(),
        "a real Files sync failure must fail the Write"
    );
    assert!(
        !events.iter().any(
            |event| matches!(event, disk::FileEvent::Renamed { to, .. } if to == "stats/intent")
        )
    );
    assert_eq!(files.read("stats/intent", disk::INTENT_CAP), Read::Absent);
    assert_eq!(view(&mut files, None), before);
    let Shown::Ready { view: sum, .. } = disk::resum(&mut files).expect("unchanged session sum")
    else {
        panic!("existing session sum");
    };
    assert_eq!(sum.all, before.all);
    recover(&mut files);
    assert_eq!(
        view(&mut files, None),
        before,
        "uncommitted temps never count"
    );
}

fn bytes(dir: &mut dyn Dir, name: &str) -> Vec<u8> {
    let Read::Bytes(value) = dir.read(name, disk::EVIDENCE_CAP) else {
        panic!("fixture must have readable {name}");
    };
    value
}

fn replace(dir: &mut dyn Dir, name: &str, value: &[u8]) {
    dir.apply(&Op::Remove(name.to_owned()))
        .expect("remove existing fixture file");
    dir.apply(&Op::Write {
        name: name.to_owned(),
        bytes: value.to_vec(),
    })
    .expect("replace fixture file");
}

fn at_epoch(target: u64) -> (Memory, MemoryJournals, Vec<Feed>) {
    let mut dir = Memory::new();
    let (mut journals, mut feeds) = seed(&mut dir);
    for expected in 1..=target {
        let plan = disk::repair(&mut dir).expect("advance fixture epoch");
        complete(&plan, &mut dir);
        assert_eq!(epoch(&mut dir), expected);
        for feed in &mut feeds {
            full(&mut dir, &mut journals, feed);
        }
    }
    (dir, journals, feeds)
}

#[test]
fn s2_f2_pending_tails_bound_each_step_and_skip_unchanged_bytes() {
    let root = Root::new();
    let first = birth(S, 1);
    let second = birth(T, 2);
    let lim = Limits {
        chunk: first.len() as u64,
        line: 4096,
    };
    root.live("a", S, &first);
    root.live("b", T, &second);
    let mut m = Maintainer::new(&root.0, lim, Duration::ZERO);
    run_to_idle(&mut m, 20);
    let mut a = first;
    let mut b = second;
    let next_a = birth(S, 3);
    let next_b = birth(T, 4);
    a.extend_from_slice(&next_a[..next_a.len() - 1]);
    b.extend_from_slice(&next_b[..next_b.len() - 1]);
    root.live("a", S, &a);
    root.live("b", T, &b);
    let mut observed = BTreeSet::new();
    for _ in 0..8 {
        let step = m.step();
        assert!(
            step.read <= lim.chunk,
            "one source chunk per step: {step:?}"
        );
        if step.read > 0 {
            let source = step.source.expect("positive read names its one source");
            assert!(observed.insert(source), "unchanged suffix read twice");
        }
    }
    assert_eq!(
        observed,
        BTreeSet::from(["live/a".to_owned(), "live/b".to_owned()])
    );
    let _ = disk::take_journal_reads();
    for _ in 0..8 {
        assert_eq!(m.step().read, 0, "unchanged live tails are stat-only");
    }
    assert_eq!(
        disk::take_journal_reads().total,
        0,
        "stat-only also skips witness content reads"
    );
    let before = view(&mut Files::at(&root.0), None);
    assert_eq!(before.all.total, 2, "partial lines do not count");
    drop(m);
    let mut restarted = Maintainer::new(&root.0, lim, Duration::ZERO);
    let mut reread = BTreeSet::new();
    for _ in 0..8 {
        let step = restarted.step();
        assert!(step.read <= lim.chunk);
        if step.read > 0 {
            assert!(reread.insert(step.source.expect("one source after restart")));
        }
    }
    let _ = disk::take_journal_reads();
    for _ in 0..8 {
        assert_eq!(
            restarted.step().read,
            0,
            "restart eventually remembers seen lengths"
        );
    }
    assert_eq!(disk::take_journal_reads().total, 0);
    a.push(b'\n');
    b.push(b'\n');
    root.live("a", S, &a);
    root.live("b", T, &b);
    run_to_idle(&mut restarted, 30);
    let feeds = [
        Feed {
            cursor: a.len() as u64,
            active: true,
            ..Feed::live("a", Some(S), a)
        },
        Feed {
            cursor: b.len() as u64,
            active: true,
            ..Feed::live("b", Some(T), b)
        },
    ];
    check(&mut Files::at(&root.0), &feeds, lim.line);
}

#[test]
fn s2_f4_damaged_epoch_uses_surviving_root_or_session_authority() {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the fixed 32-byte epoch cap fits usize on every supported target"
    )]
    for damage in [
        None,
        Some(b"bad\n".to_vec()),
        Some(vec![b'x'; disk::EPOCH_CAP as usize + 1]),
    ] {
        for surviving in ["root", "session"] {
            let (mut dir, mut journals, mut feeds) = at_epoch(3);
            let old_work = pending_foreign(&mut dir, &mut journals, &mut feeds);
            if surviving == "root" {
                dir.apply(&Op::RemoveTree("stats/session".to_owned()))
                    .expect("remove session evidence");
                dir.apply(&Op::MakeDir("stats/session".to_owned()))
                    .expect("empty session directory");
            } else {
                dir.apply(&Op::Remove("stats/view".to_owned()))
                    .expect("remove root authority");
            }
            dir.apply(&Op::Remove("stats/epoch".to_owned()))
                .expect("remove epoch authority");
            if let Some(value) = &damage {
                dir.plant("stats/epoch", value);
            }
            let plan = disk::repair(&mut dir).expect("readable surviving authority permits repair");
            complete(&plan, &mut dir);
            assert_eq!(epoch(&mut dir), 4, "must advance the proven epoch 3");
            assert!(matches!(
                disk::publish(&mut dir, &mut journals, &old_work).expect("old prepare decision"),
                Decision::Discard(Why::Epoch)
            ));
        }
    }
}

#[test]
fn s2_f4_repair_uses_maximum_authority_and_refuses_unproved_epoch() {
    let (mut dir, _, _) = at_epoch(2);
    let (mut newer, _, _) = at_epoch(5);
    let root_at_five = bytes(&mut newer, "stats/view");
    dir.plant("stats/view", &root_at_five);
    let plan = disk::repair(&mut dir).expect("root proves higher epoch");
    complete(&plan, &mut dir);
    assert_eq!(epoch(&mut dir), 6, "maximum, not only stats/epoch");

    let (mut dir, _, _) = at_epoch(2);
    let name = format!("stats/session/{S}.view");
    let session_at_five = bytes(&mut newer, &name);
    dir.plant(&name, &session_at_five);
    let plan = disk::repair(&mut dir).expect("session proves higher epoch");
    complete(&plan, &mut dir);
    assert_eq!(epoch(&mut dir), 6);

    let (mut dir, _, _) = at_epoch(3);
    dir.apply(&Op::RemoveTree("stats/session".to_owned()))
        .expect("remove session authority");
    dir.apply(&Op::Remove("stats/view".to_owned()))
        .expect("remove root authority");
    dir.plant("stats/epoch", b"unprovable\n");
    let before: Vec<_> = dir
        .names()
        .into_iter()
        .map(|name| {
            let value = dir.file(&name).expect("file snapshot").to_vec();
            (name, value)
        })
        .collect();
    let refused = disk::repair(&mut dir).expect_err("existing payload needs proven epoch");
    assert!(
        refused.describe().contains("epoch"),
        "named epoch refusal: {refused:?}"
    );
    assert!(
        !matches!(refused, Refused::RecoveryOwed),
        "no recoverable debris in fixture"
    );
    assert_eq!(
        dir.names(),
        before
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>()
    );
    for (name, value) in before {
        assert_eq!(dir.file(&name), Some(value.as_slice()));
    }

    let mut never_built = Memory::new();
    let plan = disk::repair(&mut never_built).expect("never-built fallback is permitted");
    complete(&plan, &mut never_built);
    if matches!(
        never_built.read("stats/epoch", disk::EPOCH_CAP),
        Read::Bytes(_)
    ) {
        assert_eq!(epoch(&mut never_built), 1);
    } else {
        assert!(matches!(
            disk::shown(&mut never_built, None),
            Shown::Building
        ));
    }
}

#[test]
fn s2_f4_retired_payload_content_must_match_its_name_before_swap() {
    let (mut dir, _, _) = at_epoch(3);
    let repair = disk::repair(&mut dir).expect("staged repair");
    let cut = repair
        .ops
        .iter()
        .position(|op| matches!(op, Op::Rename { from, .. } if from == "stats"))
        .expect("retirement boundary")
        + 1;
    repair.apply(&mut dir, cut).expect("interrupt swap");
    dir.apply(&Op::Rename {
        from: "stats.old.3".to_owned(),
        to: "stats.old.9".to_owned(),
    })
    .expect("hostile mismatched retired name");
    dir.plant("stats.new/epoch", b"10\n");
    match disk::recovery(&mut dir) {
        Err(refused) => assert!(refused.damage().is_some(), "named mismatched-epoch refusal"),
        Ok(plan) => {
            complete(&plan, &mut dir);
            assert_eq!(
                epoch(&mut dir),
                3,
                "restore readable old payload; do not promote bogus epoch 10"
            );
        }
    }
}

fn missing_root_new_group(dir: &mut dyn Dir) {
    recover(dir);
    let mut journals = MemoryJournals::new();
    let mut feeds = vec![Feed::live("a", Some(S), birth(S, 1))];
    journals.set("a", 1, feeds[0].ino, &feeds[0].bytes);
    full(dir, &mut journals, &mut feeds[0]);
    dir.apply(&Op::Remove("stats/view".to_owned()))
        .expect("remove only root view");
    let mut t = Feed::live("b", Some(T), birth(T, 2));
    journals.set("b", 1, t.ino, &t.bytes);
    let work = match Prepared::read(dir, &mut journals, &t.spec, limits(), None) {
        Ok(Next::Work(work)) => work,
        Err(refused) => {
            missing_root_refusal(dir, &refused);
            return;
        }
        other => panic!("new group work or named root damage: {other:?}"),
    };
    match disk::publish(dir, &mut journals, &work) {
        Ok(Decision::Publish(plan)) => {
            complete(&plan, dir);
            t.active = true;
            t.cursor = work.span().1;
            feeds.push(t);
            check(dir, &feeds, limits().line);
        }
        Err(refused) => {
            missing_root_refusal(dir, &refused);
        }
        Ok(other @ Decision::Discard(_)) => panic!("fresh source has no stale stamp: {other:?}"),
    }
}

fn missing_root_refusal(dir: &mut dyn Dir, refused: &Refused) {
    assert!(
        matches!(refused, Refused::RecoveryOwed) || refused.damage().is_some(),
        "named root damage: {refused:?}"
    );
    assert!(
        !matches!(
            disk::shown(dir, None),
            Shown::Ready {
                updating: false,
                ..
            }
        ),
        "missing root is not complete"
    );
}

#[test]
fn s2_f5_missing_root_cannot_drop_other_groups_from_delta() {
    missing_root_new_group(&mut Memory::new());
    let root = Root::new();
    missing_root_new_group(&mut Files::at(&root.0));
}

struct Switched {
    inner: Memory,
    after_root: Option<Memory>,
}
impl Dir for Switched {
    fn read(&mut self, name: &str, cap: u64) -> Read {
        let value = self.inner.read(name, cap);
        if name == "stats/view"
            && let Some(next) = self.after_root.take()
        {
            self.inner = next;
        }
        value
    }
    fn apply(&mut self, op: &Op) -> io::Result<()> {
        self.inner.apply(op)
    }
    fn list(&mut self, name: &str) -> io::Result<Vec<String>> {
        self.inner.list(name)
    }
}

fn coherent(answer: &Shown, old: &crate::stats::View, new: &crate::stats::View) {
    if let Shown::Ready {
        view: actual,
        updating: false,
        ..
    } = answer
    {
        assert!(
            actual == old || actual == new,
            "complete answer mixes committed generations: {actual:?}"
        );
    }
}

#[test]
fn s2_f6_reader_detects_committed_generation_interleaving_and_mismatch() {
    let (mut old, mut journals, mut feeds) = at_epoch(0);
    let old_view = view(&mut old, Some(S));
    let old_root = bytes(&mut old, "stats/view");
    let mut new = old.clone();
    let work = pending_foreign(&mut new, &mut journals, &mut feeds);
    complete(
        &published(disk::publish(&mut new, &mut journals, &work).expect("concurrent publish")),
        &mut new,
    );
    let new_view = view(&mut new, Some(S));
    assert_ne!(old_view, new_view, "interleaving changes the queried scope");
    let mut switched = Switched {
        inner: old,
        after_root: Some(new.clone()),
    };
    let answer = disk::shown(&mut switched, Some(S));
    assert!(
        switched.after_root.is_none(),
        "reader crossed real root-read boundary"
    );
    coherent(&answer, &old_view, &new_view);
    new.plant("stats/view", &old_root);
    coherent(&disk::shown(&mut new, Some(S)), &old_view, &new_view);
}

#[test]
fn s2_f7_startup_finishes_intent_when_every_source_is_at_eof() {
    let root = Root::new();
    let initial = birth(S, 1);
    root.live("a", S, &initial);
    let mut m = Maintainer::new(&root.0, limits(), Duration::ZERO);
    run_to_idle(&mut m, 10);
    drop(m);
    let appended = [initial, birth(S, 2)].concat();
    root.live("a", S, &appended);
    let spec = SourceSpec::Live {
        id: "live/a".to_owned(),
        label: "a".to_owned(),
        meta: format!("session_id={S}\n").into_bytes(),
    };
    let mut journals = MemoryJournals::new();
    let journal = root.0.join("sessions/a/events.jsonl");
    #[allow(
        clippy::disallowed_methods,
        reason = "test obtains the identity of its private journal to match the real maintainer witness"
    )]
    let metadata = std::fs::metadata(&journal).expect("fixture journal identity");
    journals.set("live/a", metadata.dev(), metadata.ino(), &appended);
    let mut files = Files::at(&root.0);
    let Next::Work(work) = Prepared::read(&mut files, &mut journals, &spec, limits(), None)
        .expect("append preparation")
    else {
        panic!("append work");
    };
    let plan = published(disk::publish(&mut files, &mut journals, &work).expect("append plan"));
    let remove = plan
        .ops
        .iter()
        .position(|op| matches!(op, Op::Remove(name) if name == "stats/intent"))
        .expect("intent removal after full root publication");
    plan.apply(&mut files, remove)
        .expect("crash before intent removal");
    assert!(matches!(
        files.read("stats/intent", disk::INTENT_CAP),
        Read::Bytes(_)
    ));
    let mut restarted = Maintainer::new(&root.0, limits(), Duration::ZERO);
    let first = restarted.step();
    assert!(
        first.held > Duration::ZERO,
        "startup recovery reports its root-lock interval"
    );
    assert!(first.active >= first.held);
    assert_eq!(
        files.read("stats/intent", disk::INTENT_CAP),
        Read::Absent,
        "first startup step finishes owed recovery"
    );
    run_to_idle(&mut restarted, 20);
    assert_eq!(
        files.read("stats/intent", disk::INTENT_CAP),
        Read::Absent,
        "recovery needs no journal append"
    );
    assert!(matches!(
        disk::shown(&mut files, None),
        Shown::Ready {
            updating: false,
            ..
        }
    ));
    assert_eq!(view(&mut files, None).all.total, 2);
}

#[test]
fn s2_f7_empty_fleet_finishes_each_owed_repair_prefix() {
    let mut count = None;
    for cut in 0..16 {
        let root = Root::new();
        let mut files = Files::at(&root.0);
        let _ = seed(&mut files);
        let plan = disk::repair(&mut files).expect("repair plan");
        let n = *count.get_or_insert(plan.ops.len());
        if cut > n {
            break;
        }
        plan.apply(&mut files, cut).expect("repair prefix");
        let mut restarted = Maintainer::new(&root.0, limits(), Duration::ZERO);
        let first = restarted.step();
        if cut > 0 && cut < n {
            assert!(
                first.held > Duration::ZERO,
                "owed repair prefix reports its lock interval"
            );
        }
        assert!(
            disk::recovery(&mut files)
                .expect("first startup step recovers repair prefix")
                .ops
                .is_empty()
        );
        run_to_idle(&mut restarted, 20);
        let owed = disk::recovery(&mut files).expect("coherent restart payload");
        assert!(
            owed.ops.is_empty(),
            "empty-fleet startup left repair work at cut {cut}: {owed:?}"
        );
        let names = files.list("").expect("top-level fixture names");
        assert!(
            !names
                .iter()
                .any(|name| name == "stats.new" || name.starts_with("stats.old."))
        );
    }
    assert!(
        count.is_some_and(|n| n < 16),
        "all repair boundaries enumerated"
    );
}

#[test]
fn s2_f9_failed_discovery_never_becomes_complete_absence() {
    for failed in ["archive", "sessions", "worktrees"] {
        let root = Root::new();
        if failed != "sessions" {
            root.live("a", S, &birth(S, 1));
        }
        std::fs::write(root.0.join(failed), b"not a directory").expect("invalid discovery root");
        let mut m = Maintainer::new(&root.0, limits(), Duration::ZERO);
        let mut explicit = false;
        for _ in 0..20 {
            let step = m.step();
            assert!(step.read <= limits().chunk);
            if let Outcome::Refused(refused) = &step.outcome {
                assert!(
                    refused.describe().contains(failed),
                    "refusal names failed root: {refused:?}"
                );
                explicit = true;
                break;
            }
            if matches!(step.outcome, Outcome::Idle) {
                break;
            }
        }
        if !explicit {
            let label = if failed == "archive" {
                "archive"
            } else {
                "sessions"
            };
            let answer = view(&mut Files::at(&root.0), None);
            assert!(
                answer.all.sources_total > answer.all.sources_read,
                "{failed} must contribute failed coverage"
            );
            assert!(
                answer.all.gaps.iter().any(|gap| {
                    matches!(
                        gap.reason,
                        crate::stats::Reason::Source(crate::stats::SourceGap::Unreadable)
                    ) && gap
                        .source
                        .as_deref()
                        .is_some_and(|name| name.contains(label))
                }),
                "{failed} needs a named unreadable discovery gap"
            );
        }
    }
}

#[test]
fn s2_f10_repair_refuses_resolvable_temps_and_cleans_damaged_intent() {
    for temp in ["stats/view.tmp", "stats/session/orphan.evidence.tmp"] {
        let (mut dir, _, _) = at_epoch(2);
        dir.plant(temp, b"unpublished stage");
        assert!(
            matches!(disk::repair(&mut dir), Err(Refused::RecoveryOwed)),
            "resolvable temp blocks repair: {temp}"
        );
        recover(&mut dir);
        assert!(
            disk::repair(&mut dir).is_ok(),
            "recovery resolves temp precondition"
        );
    }
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the fixed 64 KiB intent cap fits usize on every supported target"
    )]
    for intent in [
        b"unparseable intent".to_vec(),
        vec![b'x'; disk::INTENT_CAP as usize + 1],
    ] {
        let (mut dir, _, _) = at_epoch(2);
        dir.plant("stats/intent", &intent);
        dir.plant("stats/view.tmp", b"root stage");
        dir.plant("stats/session/orphan.view.tmp", b"session stage");
        assert!(
            disk::recovery(&mut dir).is_err(),
            "damaged intent cannot recover"
        );
        let plan = disk::repair(&mut dir).expect("amendment 11 permits damaged-intent takeover");
        complete(&plan, &mut dir);
        assert_eq!(epoch(&mut dir), 3);
        assert_eq!(dir.read("stats/intent", disk::INTENT_CAP), Read::Absent);
        assert!(
            dir.names().iter().all(|name| !name.ends_with(".tmp")),
            "repair removes every temp"
        );
        recover(&mut dir);
    }
}

#[test]
fn s2_f8_healed_damage_recheck_never_prepares_journal_under_lock() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    let root = Root::new();
    let mut journal = birth(S, 1);
    root.live("a", S, &journal);
    let mut m = Maintainer::new(&root.0, limits(), Duration::ZERO);
    run_to_idle(&mut m, 10);
    let view_name = format!("stats/session/{S}.view");
    let old = bytes(&mut Files::at(&root.0), &view_name);
    let old_epoch = epoch(&mut Files::at(&root.0));
    std::fs::write(root.0.join(&view_name), b"damaged session view")
        .expect("damage before prepare");
    journal.extend(birth(S, 2));
    root.live("a", S, &journal);
    let called = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&called);
    m.on_recheck(move |path| {
        let lock = Payload::at(path)
            .lock(Duration::ZERO)
            .expect("hook runs before our lock request");
        std::fs::write(path.join(&view_name), &old).expect("another writer heals the payload");
        drop(lock);
        observed.store(true, Ordering::SeqCst);
    });
    let _ = disk::take_journal_reads();
    let step = m.step();
    assert!(
        called.load(Ordering::SeqCst),
        "real damage path reached the boundary hook"
    );
    assert!(
        matches!(step.outcome, Outcome::Deferred),
        "healed state needs unlocked preparation"
    );
    assert!(
        step.held > Duration::ZERO,
        "actual recheck lock interval is reported"
    );
    assert!(step.active >= step.held);
    let read = disk::take_journal_reads();
    assert_eq!(
        read.under_lock, 0,
        "no digest or witness read during damage recheck"
    );
    assert_eq!(
        epoch(&mut Files::at(&root.0)),
        old_epoch,
        "healed state was not repaired"
    );
    let next = m.step();
    assert!(
        matches!(next.outcome, Outcome::Published { .. }),
        "next turn prepares and publishes"
    );
    assert!(
        disk::take_journal_reads().total > 0,
        "observer counts the real subsequent journal read"
    );
    assert_eq!(view(&mut Files::at(&root.0), None).all.total, 2);
}

fn first_build_crash(dir: &mut dyn Dir, cut: usize) -> usize {
    let mut feed = Feed::live("a", Some(S), birth(S, 1));
    let mut journals = MemoryJournals::new();
    journals.set("a", 1, feed.ino, &feed.bytes);
    let Next::Work(work) = Prepared::read(dir, &mut journals, &feed.spec, limits(), None)
        .expect("never-built preparation")
    else {
        panic!("initial work");
    };
    let plan = published(disk::publish(dir, &mut journals, &work).expect("first-build plan"));
    let n = plan.ops.len();
    plan.apply(dir, cut)
        .expect("first-publication crash prefix");
    recover(dir);
    let top = dir.list("").expect("recovered initial root");
    assert!(
        !top.iter()
            .any(|name| name == "stats.new" || name.starts_with("stats.old."))
    );
    if top.iter().any(|name| name == "stats") {
        assert_eq!(
            epoch(dir),
            0,
            "first payload cannot exist without proven epoch 0"
        );
    }
    match Prepared::read(dir, &mut journals, &feed.spec, limits(), None)
        .expect("resume first build")
    {
        Next::Work(work) => {
            complete(
                &published(disk::publish(dir, &mut journals, &work).expect("resume publish")),
                dir,
            );
        }
        Next::Nothing => {}
        Next::Holding(_) => panic!("small LF line needs no hold"),
    }
    feed.active = true;
    feed.cursor = feed.bytes.len() as u64;
    check(dir, &[feed], limits().line);
    n
}

#[test]
fn s2_f4_first_build_recovers_every_op_before_and_after_epoch_write() {
    let n = first_build_crash(&mut Memory::new(), 0);
    for cut in 0..=n {
        first_build_crash(&mut Memory::new(), cut);
    }
    let root = Root::new();
    let n = first_build_crash(&mut Files::at(&root.0), 0);
    for cut in 0..=n {
        let root = Root::new();
        first_build_crash(&mut Files::at(&root.0), cut);
    }
    for invalid in [None, Some(b"invalid epoch\n".as_slice())] {
        let mut dir = Memory::new();
        dir.apply(&Op::MakeDir("stats.new".to_owned()))
            .expect("initial staging");
        if let Some(value) = invalid {
            dir.plant("stats.new/epoch", value);
        }
        recover(&mut dir);
        assert!(
            dir.list("").expect("never-built root").is_empty(),
            "invalid initial staging is removed"
        );
    }
}

fn damaged_primary(dir: &mut dyn Dir, remove: bool) -> String {
    let (mut journals, mut feeds) = seed(dir);
    let work = pending_foreign(dir, &mut journals, &mut feeds);
    let plan = published(disk::publish(dir, &mut journals, &work).expect("valid intent plan"));
    plan.apply(dir, primary_index(&plan) + 1)
        .expect("intent and primary committed before damage");
    let name = format!("stats/session/{S}.evidence");
    if remove {
        dir.apply(&Op::Remove(name.clone()))
            .expect("remove primary named by valid intent");
    } else {
        replace(dir, &name, b"damaged primary evidence");
    }
    name
}

#[test]
fn s2_f10_valid_intent_with_damaged_or_missing_primary_allows_takeover() {
    for remove in [false, true] {
        let mut dir = Memory::new();
        let name = damaged_primary(&mut dir, remove);
        let refused =
            disk::recovery(&mut dir).expect_err("intent replay must detect primary damage");
        assert!(
            refused.damage().is_some(),
            "proven damage, not IO: {refused:?}"
        );
        assert!(refused.describe().contains(&name));
        let old_epoch = epoch(&mut dir);
        let repair = disk::repair(&mut dir).expect("amendment 15 permits damage takeover");
        complete(&repair, &mut dir);
        assert_eq!(epoch(&mut dir), old_epoch + 1);
        assert_eq!(dir.read("stats/intent", disk::INTENT_CAP), Read::Absent);
        assert!(dir.names().iter().all(|name| !name.ends_with(".tmp")));
        recover(&mut dir);
    }
}

struct Unavailable {
    inner: Memory,
    read: Option<String>,
    listing: Option<String>,
    writes: usize,
}
impl Dir for Unavailable {
    fn read(&mut self, name: &str, cap: u64) -> Read {
        if self.read.as_deref() == Some(name) {
            Read::Unreadable("EACCES injected at the external read door".to_owned())
        } else {
            self.inner.read(name, cap)
        }
    }
    fn list(&mut self, name: &str) -> io::Result<Vec<String>> {
        if self.listing.as_deref() == Some(name) {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected directory IO error",
            ))
        } else {
            self.inner.list(name)
        }
    }
    fn apply(&mut self, op: &Op) -> io::Result<()> {
        self.writes += 1;
        self.inner.apply(op)
    }
}

#[test]
fn s2_f10_io_error_refuses_repair_and_keeps_every_file() {
    for fail in ["stats/intent", "primary", "listing"] {
        let mut dir = Memory::new();
        let (mut journals, mut feeds) = seed(&mut dir);
        let work = pending_foreign(&mut dir, &mut journals, &mut feeds);
        let plan = published(disk::publish(&mut dir, &mut journals, &work).expect("intent plan"));
        plan.apply(&mut dir, primary_index(&plan) + 1)
            .expect("valid interrupted publish");
        let before: Vec<_> = dir
            .names()
            .into_iter()
            .map(|name| {
                let value = dir.file(&name).expect("snapshot").to_vec();
                (name, value)
            })
            .collect();
        let read = match fail {
            "stats/intent" => Some("stats/intent".to_owned()),
            "primary" => Some(format!("stats/session/{S}.evidence")),
            _ => None,
        };
        let mut blocked = Unavailable {
            inner: dir,
            read,
            listing: (fail == "listing").then(|| "stats".to_owned()),
            writes: 0,
        };
        assert!(
            matches!(disk::recovery(&mut blocked), Err(Refused::Io(_))),
            "external IO is not damage: {fail}"
        );
        assert!(
            matches!(disk::repair(&mut blocked), Err(Refused::Io(_))),
            "IO cannot authorize a repair plan: {fail}"
        );
        assert_eq!(blocked.writes, 0);
        assert_eq!(
            blocked.inner.names(),
            before
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>()
        );
        for (name, value) in before {
            assert_eq!(blocked.inner.file(&name), Some(value.as_slice()));
        }
    }
    for name in ["stats/view".to_owned(), format!("stats/session/{S}.view")] {
        let (dir, _, _) = at_epoch(2);
        let before = dir.clone();
        let mut blocked = Unavailable {
            inner: dir,
            read: Some(name),
            listing: None,
            writes: 0,
        };
        assert!(
            disk::recovery(&mut blocked)
                .expect("no intent or leftovers")
                .ops
                .is_empty()
        );
        assert!(
            matches!(disk::repair(&mut blocked), Err(Refused::Io(_))),
            "epoch-authority scan cannot skip an IO failure and guess a lower authority"
        );
        assert_eq!(blocked.writes, 0);
        assert_eq!(blocked.inner.names(), before.names());
        for name in before.names() {
            assert_eq!(blocked.inner.file(&name), before.file(&name));
        }
    }
}

#[test]
fn s2_f9_failed_enumeration_does_not_complete_a_repair_episode() {
    let root = Root::new();
    let mut journal = birth(S, 1);
    root.live("a", S, &journal);
    std::fs::write(root.0.join("archive"), b"not a directory").expect("failed enumeration");
    let mut m = Maintainer::new(&root.0, limits(), Duration::ZERO);
    run_to_idle(&mut m, 20);
    std::fs::write(root.0.join("stats/view"), b"first damage").expect("first damage");
    journal.extend(birth(S, 2));
    root.live("a", S, &journal);
    let repaired = (0..20)
        .map(|_| m.step())
        .find(|step| matches!(step.outcome, Outcome::Repaired { .. }));
    assert!(repaired.is_some(), "one automatic repair is allowed");
    run_to_idle(&mut m, 20);
    std::fs::write(root.0.join("stats/view"), b"second damage").expect("second damage");
    journal.extend(birth(S, 3));
    root.live("a", S, &journal);
    let mut stopped = false;
    for _ in 0..20 {
        let step = m.step();
        assert!(
            !matches!(step.outcome, Outcome::Repaired { .. }),
            "failed enumeration cannot reset repair budget"
        );
        if matches!(step.outcome, Outcome::Refused(_)) {
            stopped = m.stopped();
            break;
        }
    }
    assert!(
        stopped,
        "repeated damage stops while the rebuild still lacks discovery coverage"
    );
}

fn payload_snapshot(dir: &mut dyn Dir) -> Vec<(String, Vec<u8>)> {
    let mut snapshot = Vec::new();
    for parent in ["stats", "stats/session"] {
        for name in dir.list(parent).expect("snapshot payload names") {
            if parent == "stats" && name == "session" {
                continue;
            }
            let name = format!("{parent}/{name}");
            snapshot.push((name.clone(), bytes(dir, &name)));
        }
    }
    snapshot.sort();
    snapshot
}

#[test]
fn s2_f10_files_permission_error_is_io_and_never_authorizes_repair() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = Root::new();
    let mut files = Files::at(&root.0);
    let (mut journals, mut feeds) = seed(&mut files);
    let work = pending_foreign(&mut files, &mut journals, &mut feeds);
    let plan = published(disk::publish(&mut files, &mut journals, &work).expect("valid intent"));
    plan.apply(&mut files, primary_index(&plan) + 1)
        .expect("interrupt publication");
    let before = payload_snapshot(&mut files);
    let path = root.0.join("stats/epoch");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o0))
        .expect("deny fixture reads");
    #[allow(
        clippy::disallowed_methods,
        reason = "test probes whether the OS enforces its private chmod fixture, including privileged test runners"
    )]
    let enforced = std::fs::File::open(&path);
    if enforced.is_ok() {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("restore private mode");
        eprintln!(
            "permission fixture is not enforced by this OS identity; deterministic IO-door test still runs"
        );
        return;
    }
    let observed = files.read("stats/epoch", disk::EPOCH_CAP);
    let recovery = disk::recovery(&mut files);
    let repair = disk::repair(&mut files);
    let shown = disk::shown(&mut files, None);
    let mut m = Maintainer::new(&root.0, limits(), Duration::ZERO);
    let step = m.step();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("restore fixture mode");
    assert!(
        matches!(observed, Read::Unreadable(_)),
        "real Files door retains IO classification"
    );
    assert!(matches!(recovery, Err(Refused::Io(_))));
    assert!(matches!(repair, Err(Refused::Io(_))));
    assert!(matches!(shown, Shown::Refused(Refused::Io(_))));
    assert!(matches!(step.outcome, Outcome::Refused(Refused::Io(_))));
    assert!(
        !m.stopped(),
        "transient IO does not consume the repeated-damage stop policy"
    );
    assert_eq!(
        payload_snapshot(&mut files),
        before,
        "no file removed or rewritten on IO failure"
    );
}

#[test]
fn s2_f10_nonregular_payload_node_is_proven_damage_on_both_backends() {
    fn check_node(dir: &mut dyn Dir) {
        let _ = seed(dir);
        dir.apply(&Op::Remove("stats/view".to_owned()))
            .expect("remove regular root view");
        dir.apply(&Op::MakeDir("stats/view".to_owned()))
            .expect("nonregular replacement");
        assert!(matches!(
            dir.read("stats/view", disk::ROOT_VIEW_CAP),
            Read::Invalid(_)
        ));
        assert!(matches!(
            disk::shown(dir, None),
            Shown::Refused(Refused::Damaged { .. })
        ));
        let before = epoch(dir);
        let repair = disk::repair(dir).expect("regularity violation is damage, not transient IO");
        complete(&repair, dir);
        assert_eq!(epoch(dir), before + 1);
    }
    check_node(&mut Memory::new());
    let root = Root::new();
    check_node(&mut Files::at(&root.0));
}
