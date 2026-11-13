//! Frozen S2 acceptance. Requirements: effective S2 plan and lead rulings 1-10.
#![allow(clippy::expect_used, reason = "acceptance assertions and fixture IO")]
#![allow(
    clippy::too_many_lines,
    reason = "explicit crash matrix and independent oracle"
)]

#[path = "s2_cases.rs"]
mod cases;

#[path = "s2_regression_spec.rs"]
mod regression;

#[path = "s2_batch2_spec.rs"]
mod batch2;

#[path = "s2_mutants_spec.rs"]
mod mutants;

use super::disk::{
    self, Decision, Dir, Files, Journal, Limits, Memory, MemoryJournals, Next, Op, Payload, Plan,
    Prepared, Read, Refused, Shown, SourceSpec, Why,
};
use super::maint::{self, Maintainer, Outcome, Step};
use super::{Doc, parse_count};
use cases::{Feed, PRIVATE, Rig, S, T, birth, check, complete, fixtures, published, recover, view};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "ae-s2-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).expect("exclusive private root");
        Self(p)
    }
    fn live(&self, name: &str, session: &str, bytes: &[u8]) {
        let p = self.0.join("sessions").join(name);
        std::fs::create_dir_all(&p).expect("fixture session");
        std::fs::write(p.join("meta"), format!("session_id={session}\n")).expect("fixture meta");
        std::fs::write(p.join("events.jsonl"), bytes).expect("fixture journal");
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn limits() -> Limits {
    Limits {
        chunk: 4 << 20,
        line: 4 << 20,
    }
}
fn full(dir: &mut dyn Dir, journals: &mut dyn Journal, feed: &mut Feed) {
    let Next::Work(work) =
        Prepared::read(dir, journals, &feed.spec, limits(), None).expect("prepare full fixture")
    else {
        panic!("full LF fixture work");
    };
    assert_eq!(
        work.span().1,
        feed.bytes.len() as u64,
        "whole fixture fits the independent work bound"
    );
    let plan = published(disk::publish(dir, journals, &work).expect("publish full"));
    complete(&plan, dir);
    feed.cursor = work.span().1;
    feed.active = true;
}
fn seed(dir: &mut dyn Dir) -> (MemoryJournals, Vec<Feed>) {
    recover(dir);
    let mut feeds = vec![
        Feed::live("a", Some(S), birth(S, 1)),
        Feed::live("b", Some(T), birth(T, 3)),
    ];
    let mut journals = MemoryJournals::new();
    for f in &mut feeds {
        journals.set(f.id(), 1, f.ino, &f.bytes);
        full(dir, &mut journals, f);
    }
    check(dir, &feeds, limits().line);
    (journals, feeds)
}
fn pending_foreign(
    dir: &mut dyn Dir,
    journals: &mut MemoryJournals,
    feeds: &mut [Feed],
) -> Prepared {
    feeds[0].bytes.extend(birth(T, 2));
    feeds[0].bytes.extend(birth(S, 4));
    journals.set(feeds[0].id(), 1, feeds[0].ino, &feeds[0].bytes);
    let Next::Work(work) =
        Prepared::read(dir, journals, &feeds[0].spec, limits(), None).expect("prepare foreign")
    else {
        panic!("foreign work");
    };
    work
}
fn primary_index(plan: &Plan) -> usize {
    plan.ops.iter().position(|op| matches!(op, Op::Rename { to, .. } if to == &format!("stats/session/{S}.evidence"))).expect("primary evidence rename")
}
fn epoch(dir: &mut dyn Dir) -> u64 {
    let Read::Bytes(bytes) = dir.read("stats/epoch", disk::EPOCH_CAP) else {
        panic!("readable epoch");
    };
    std::str::from_utf8(&bytes)
        .expect("epoch text")
        .trim()
        .parse()
        .expect("epoch integer")
}

#[test]
fn s2_fold_oracle_foreign_idless_and_privacy() {
    let mut rig = Rig::new(fixtures(), 4 << 20, 4 << 20);
    for i in 0..rig.feeds.len() {
        rig.catchup(i);
    }
    assert_eq!(view(&mut rig.dir, None).all.total, 4);
    for name in rig.dir.names() {
        if let Some(bytes) = rig.dir.file(&name) {
            assert!(
                !bytes
                    .windows(PRIVATE.len())
                    .any(|w| w == PRIVATE.as_bytes()),
                "prompt persisted in {name}"
            );
        }
    }
}

#[test]
fn s2_old_prepare_merges_event_that_did_not_move_cursor() {
    let mut rig = Rig::new(
        vec![Feed::live("a", Some(S), birth(S, 1))],
        4 << 20,
        4 << 20,
    );
    rig.catchup(0);
    let initial = rig.feeds[0].bytes.len() as u64;
    rig.feeds[0].bytes.extend(birth(S, 2));
    let later = rig.feeds[0].bytes.len() as u64;
    let event = birth(S, 3);
    rig.feeds[0].bytes.extend(&event);
    rig.sync(0);
    // Prepare only birth 2. Birth 3 will be inserted ahead of the same cursor.
    rig.limits.chunk = later - initial;
    let work = rig.ready(0);
    assert_eq!(work.span(), (initial, later));
    rig.event(0, later, &event[..event.len() - 1]);
    rig.apply(0, &work);
    assert_eq!(view(&mut rig.dir, None).all.total, 3);
    rig.limits = limits();
    rig.catchup(0);
    assert_eq!(
        view(&mut rig.dir, None).all.total,
        3,
        "sequential replay is idempotent"
    );
}

#[test]
fn s2_two_old_prepares_only_first_advances() {
    let mut rig = Rig::new(
        vec![Feed::live("a", Some(S), birth(S, 1))],
        4 << 20,
        4 << 20,
    );
    let first = rig.ready(0);
    let second = rig.ready(0);
    rig.apply(0, &first);
    let before = disk::shown(&mut rig.dir, None);
    assert!(matches!(
        disk::publish(&mut rig.dir, &mut rig.journals, &second).expect("stale decision"),
        Decision::Discard(Why::Stamp)
    ));
    assert_eq!(disk::shown(&mut rig.dir, None), before);
}

#[test]
fn s2_fresh_witness_rejects_change_after_prepare_and_reset_can_commit() {
    for change in 0..3 {
        let mut rig = Rig::new(
            vec![Feed::live("a", Some(S), birth(S, 1))],
            4 << 20,
            4 << 20,
        );
        rig.catchup(0);
        rig.feeds[0].bytes.extend(birth(S, 2));
        rig.sync(0);
        let old = rig.ready(0);
        let before = disk::shown(&mut rig.dir, None);
        match change {
            0 => {
                rig.feeds[0].ino += 1;
                rig.feeds[0].bytes = birth(S, 7);
            }
            1 => rig.feeds[0].bytes.truncate(8),
            _ => {
                let at = rig.feeds[0]
                    .bytes
                    .iter()
                    .rposition(|b| *b == b'x')
                    .unwrap_or(20);
                rig.feeds[0].bytes[at] ^= 1;
            }
        }
        rig.sync(0);
        assert!(matches!(
            disk::publish(&mut rig.dir, &mut rig.journals, &old).expect("changed source decision"),
            Decision::Discard(Why::Witness)
        ));
        assert_eq!(disk::shown(&mut rig.dir, None), before);
        // A canonical replacement now resets successfully from the OLD stamp.
        rig.feeds[0].ino += 1;
        rig.feeds[0].bytes = birth(S, 9);
        rig.sync(0);
        let reset = rig.ready(0);
        assert!(reset.is_reset());
        let competing = rig.ready(0);
        rig.apply(0, &reset);
        assert!(matches!(
            disk::publish(&mut rig.dir, &mut rig.journals, &competing).expect("competing reset"),
            Decision::Discard(Why::Stamp)
        ));
        assert_eq!(view(&mut rig.dir, None).all.total, 1);
    }
}

#[test]
fn s2_append_growth_preserves_a_prepared_prefix() {
    let mut rig = Rig::new(
        vec![Feed::live("a", Some(S), birth(S, 1))],
        4 << 20,
        4 << 20,
    );
    let work = rig.ready(0);
    rig.feeds[0].bytes.extend(birth(S, 2));
    rig.sync(0);
    rig.apply(0, &work);
    rig.catchup(0);
}

#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "persisted layout suffixes are case-sensitive protocol names"
)]
fn crash_publish_matrix(dir: &mut dyn Dir, cut: usize, recovery_cut: Option<usize>) -> usize {
    let (mut journals, mut feeds) = seed(dir);
    let work = pending_foreign(dir, &mut journals, &mut feeds);
    let plan = published(disk::publish(dir, &mut journals, &work).expect("crash publish"));
    let size = plan.ops.len();
    let intent = plan
        .ops
        .iter()
        .position(|o| matches!(o,Op::Rename{to,..} if to=="stats/intent"))
        .expect("intent commit point");
    let first_view = plan
        .ops
        .iter()
        .position(|o| matches!(o,Op::Rename{to,..} if to.ends_with(".view")))
        .expect("session view rename");
    let root_view = plan
        .ops
        .iter()
        .position(|o| matches!(o,Op::Rename{to,..} if to=="stats/view"))
        .expect("root view rename");
    let clear = plan
        .ops
        .iter()
        .position(|o| matches!(o,Op::Remove(n) if n=="stats/intent"))
        .expect("intent removal");
    assert!(
        intent < primary_index(&plan)
            && primary_index(&plan) < first_view
            && first_view < root_view
            && root_view < clear
    );
    for (i, op) in plan.ops.iter().enumerate() {
        if matches!(op,Op::Rename{to,..} if to.ends_with(".evidence")) {
            assert!(i > intent && i < first_view);
        }
        if matches!(op,Op::Rename{to,..} if to.ends_with(".view")) {
            assert!(i < root_view);
        }
        if matches!(op, Op::Write { .. }) {
            assert!(i < intent, "all temp writes precede intent publication");
        }
    }
    assert!(
        plan.ops
            .iter()
            .filter(|op| matches!(op, Op::Rename { to, .. } if to.ends_with(".view")))
            .count()
            >= 2,
        "multi-group session renames"
    );
    plan.apply(dir, cut.min(size)).expect("crash prefix");
    let committed = cut > primary_index(&plan);
    if committed {
        feeds[0].cursor = work.span().1;
    }
    if let Some(prefix) = recovery_cut {
        let redo = disk::recovery(dir).expect("first recovery plan");
        redo.apply(dir, prefix.min(redo.ops.len()))
            .expect("interrupted recovery");
    }
    recover(dir);
    check(dir, &feeds, limits().line);
    // Uncommitted work remains in the journal and catches up exactly once.
    if !committed {
        full(dir, &mut journals, &mut feeds[0]);
    }
    check(dir, &feeds, limits().line);
    size
}

#[test]
fn s2_publish_crash_every_op_memory_and_files() {
    let count = crash_publish_matrix(&mut Memory::new(), 0, None);
    for cut in 0..=count {
        crash_publish_matrix(&mut Memory::new(), cut, None);
    }
    let root = Root::new();
    let count = crash_publish_matrix(&mut Files::at(&root.0), 0, None);
    for cut in 0..=count {
        let root = Root::new();
        let payload = Payload::at(&root.0);
        let _lock = payload.lock(Duration::ZERO).expect("private root lock");
        crash_publish_matrix(&mut Files::at(&root.0), cut, None);
    }
}

#[test]
fn s2_recovery_can_crash_after_each_of_its_ops() {
    let mut dir = Memory::new();
    let (mut journals, mut feeds) = seed(&mut dir);
    let work = pending_foreign(&mut dir, &mut journals, &mut feeds);
    let plan = published(disk::publish(&mut dir, &mut journals, &work).expect("publish"));
    let cut = primary_index(&plan) + 1;
    plan.apply(&mut dir, cut).expect("evidence committed");
    let count = disk::recovery(&mut dir).expect("recovery").ops.len();
    for recovery_cut in 0..=count {
        crash_publish_matrix(&mut Memory::new(), cut, Some(recovery_cut));
    }
}

fn repair_matrix(dir: &mut dyn Dir, cut: usize) -> usize {
    let (mut journals, feeds) = seed(dir);
    let before = view(dir, None);
    let old_epoch = epoch(dir);
    let plan = disk::repair(dir).expect("repair plan");
    let moved = plan
        .ops
        .iter()
        .position(|op| matches!(op, Op::Rename { from, .. } if from == "stats"))
        .expect("payload move");
    let count = plan.ops.len();
    plan.apply(dir, cut.min(count)).expect("repair prefix");
    recover(dir);
    if cut > moved {
        assert_eq!(epoch(dir), old_epoch + 1);
        match disk::shown(dir, None) {
            Shown::Ready { view, .. } => assert_eq!(view.all.total, 0),
            Shown::Building => {} // A repaired payload may not have a materialized view yet.
            other @ Shown::Refused(_) => {
                panic!("expected an empty repaired payload, got {other:?}");
            }
        }
        for mut feed in feeds {
            feed.cursor = 0;
            full(dir, &mut journals, &mut feed);
        }
    } else {
        assert_eq!(epoch(dir), old_epoch);
        assert_eq!(view(dir, None), before);
    }
    count
}

#[test]
fn s2_repair_crash_every_op_memory_and_files() {
    let count = repair_matrix(&mut Memory::new(), 0);
    for cut in 0..=count {
        repair_matrix(&mut Memory::new(), cut);
    }
    let root = Root::new();
    let count = repair_matrix(&mut Files::at(&root.0), 0);
    for cut in 0..=count {
        let root = Root::new();
        let payload = Payload::at(&root.0);
        let _lock = payload.lock(Duration::ZERO).expect("private lock");
        repair_matrix(&mut Files::at(&root.0), cut);
        #[allow(
            clippy::disallowed_methods,
            reason = "acceptance inspects the lock file in its private temporary root"
        )]
        let lock_is_file = root.0.join("stats.lock").is_file();
        assert!(lock_is_file, "repair preserves lock node");
    }
}

#[test]
fn s2_repair_discards_prepared_work_from_old_epoch() {
    let mut rig = Rig::new(
        vec![Feed::live("a", Some(S), birth(S, 1))],
        4 << 20,
        4 << 20,
    );
    rig.catchup(0);
    rig.feeds[0].bytes.extend(birth(S, 2));
    rig.sync(0);
    let old = rig.ready(0);
    let plan = disk::repair(&mut rig.dir).expect("repair");
    complete(&plan, &mut rig.dir);
    assert!(matches!(
        disk::publish(&mut rig.dir, &mut rig.journals, &old).expect("old epoch decision"),
        Decision::Discard(Why::Epoch)
    ));
}

#[test]
fn s2_write_plans_refuse_unrecovered_intent() {
    let mut dir = Memory::new();
    let (mut journals, mut feeds) = seed(&mut dir);
    let work = pending_foreign(&mut dir, &mut journals, &mut feeds);
    let plan = published(disk::publish(&mut dir, &mut journals, &work).expect("publish"));
    let cut = plan
        .ops
        .iter()
        .position(|op| matches!(op, Op::Rename { to, .. } if to == "stats/intent"))
        .expect("intent rename")
        + 1;
    plan.apply(&mut dir, cut).expect("intent only");
    assert!(matches!(
        disk::publish(&mut dir, &mut journals, &work),
        Err(Refused::RecoveryOwed)
    ));
    assert!(matches!(disk::repair(&mut dir), Err(Refused::RecoveryOwed)));
    let line = birth(S, 9);
    assert!(matches!(
        disk::event(&mut dir, Some(S), "a", 9999, &line[..line.len() - 1]),
        Err(Refused::RecoveryOwed)
    ));
    recover(&mut dir);
}

#[test]
fn s2_files_and_memory_agree_on_exclusive_create_rename_and_absence() {
    let root = Root::new();
    let mut files = Files::at(&root.0);
    let mut memory = Memory::new();
    let ops = vec![
        Op::MakeDir("stats".to_owned()),
        Op::Write {
            name: "stats/a".to_owned(),
            bytes: b"a".to_vec(),
        },
        Op::Write {
            name: "stats/a".to_owned(),
            bytes: b"must not overwrite".to_vec(),
        },
        Op::Write {
            name: "stats/b".to_owned(),
            bytes: b"b".to_vec(),
        },
        Op::Rename {
            from: "stats/a".to_owned(),
            to: "stats/b".to_owned(),
        },
        Op::Remove("stats/absent".to_owned()),
        Op::Remove("stats/b".to_owned()),
    ];
    for op in ops {
        assert_eq!(
            files.apply(&op).map_err(|e| e.kind()),
            memory.apply(&op).map_err(|e| e.kind()),
            "operation {op:?}"
        );
        for name in ["stats/a", "stats/b", "stats/absent"] {
            assert_eq!(
                files.read(name, 64),
                memory.read(name, 64),
                "after {op:?}: {name}"
            );
        }
    }
}

#[test]
fn s2_every_payload_cap_refuses_before_its_parser_on_real_files() {
    for (name, cap, doc) in [
        ("stats/view".to_owned(), disk::ROOT_VIEW_CAP, Doc::RootView),
        (
            format!("stats/session/{S}.view"),
            disk::SESSION_VIEW_CAP,
            Doc::SessionView,
        ),
        (
            format!("stats/session/{S}.evidence"),
            disk::EVIDENCE_CAP,
            Doc::Evidence,
        ),
        ("stats/intent".to_owned(), disk::INTENT_CAP, Doc::Intent),
        ("stats/epoch".to_owned(), disk::EPOCH_CAP, Doc::Epoch),
    ] {
        let root = Root::new();
        let mut dir = Files::at(&root.0);
        let (mut journals, mut feeds) = seed(&mut dir);
        let work = pending_foreign(&mut dir, &mut journals, &mut feeds);
        let path = root.0.join(&name);
        // Keep any valid prefix, then grow sparsely. No allocation proportional to the evidence cap.
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .expect("cap fixture");
        file.set_len(cap + 1).expect("cap+1");
        let before = parse_count(doc);
        let refused = if doc == Doc::Evidence {
            disk::publish(&mut dir, &mut journals, &work).err()
        } else if matches!(doc, Doc::Intent | Doc::Epoch) {
            disk::recovery(&mut dir).err()
        } else {
            match disk::shown(&mut dir, Some(S)) {
                Shown::Refused(e) => Some(e),
                _ => None,
            }
        };
        assert!(
            matches!(refused, Some(Refused::OverCap { file, cap: c }) if file.ends_with(&name) && c == cap),
            "named over-cap refusal for {name}"
        );
        assert_eq!(
            parse_count(doc),
            before,
            "oversized {name} reached its parser"
        );
        file.set_len(cap).expect("exact cap");
        assert!(matches!(dir.read(&name, cap), Read::Bytes(bytes) if bytes.len() as u64 == cap));
        file.set_len(cap + 1).expect("cap+1 again");
        assert_eq!(dir.read(&name, cap), Read::Oversized);
    }
}

fn run_to_idle(maintainer: &mut Maintainer, bound: usize) -> Vec<Step> {
    let mut steps = Vec::new();
    for _ in 0..bound {
        let s = maintainer.step();
        assert!(s.read <= 4 << 20, "new-byte chunk bound");
        assert!(maint::pause(&s) >= s.active);
        let idle = matches!(s.outcome, Outcome::Idle);
        steps.push(s);
        if idle {
            return steps;
        }
    }
    panic!("maintainer did not settle within fixture bound")
}

#[test]
fn s2_more_than_one_chunk_restart_and_round_robin() {
    let root = Root::new();
    let mut big = Vec::new();
    for i in 0..15 {
        big.extend(birth(S, i));
    }
    root.live("a", S, &big);
    root.live("z", T, &birth(T, 99));
    let lim = Limits {
        chunk: 1024,
        line: 4096,
    };
    let mut first = Maintainer::new(&root.0, lim, Duration::ZERO);
    let one = first.step();
    assert!(one.read <= lim.chunk);
    let Outcome::Published { to: first_end, .. } = one.outcome else {
        panic!("first LF-rich chunk publishes");
    };
    assert!(first_end > 0 && first_end < big.len() as u64);
    drop(first);
    let mut second = Maintainer::new(&root.0, lim, Duration::ZERO);
    let steps = run_to_idle(&mut second, 100);
    assert!(
        steps
            .iter()
            .any(|s| matches!(s.outcome, Outcome::Published { from, .. } if from == first_end)),
        "resumed durable cursor"
    );
    let small = steps
        .iter()
        .position(|s| {
            s.source.as_deref() == Some("live/z") && matches!(s.outcome, Outcome::Published { .. })
        })
        .expect("small source published");
    let last_big = steps
        .iter()
        .rposition(|s| {
            s.source.as_deref() == Some("live/a") && matches!(s.outcome, Outcome::Published { .. })
        })
        .expect("large source completed");
    assert!(small < last_big, "large source cannot strand later one");
    let feeds = vec![
        Feed {
            cursor: big.len() as u64,
            active: true,
            ..Feed::live("a", Some(S), big)
        },
        Feed {
            cursor: birth(T, 99).len() as u64,
            active: true,
            ..Feed::live("z", Some(T), birth(T, 99))
        },
    ];
    check(&mut Files::at(&root.0), &feeds, lim.line);
}

#[test]
fn s2_real_two_maintainers_and_external_root_lock() {
    let root = Root::new();
    root.live("a", S, &birth(S, 1));
    let mut a = Maintainer::new(&root.0, limits(), Duration::ZERO);
    let mut b = Maintainer::new(&root.0, limits(), Duration::ZERO);
    let wa = a.prepare().expect("first prepare");
    let wb = b.prepare().expect("second prepare");
    assert!(matches!(a.commit(wa).outcome, Outcome::Published { .. }));
    let duplicate = b.commit(wb);
    assert!(matches!(duplicate.outcome, Outcome::Discarded(Why::Stamp)));
    assert!(duplicate.active > Duration::ZERO);
    assert!(maint::pause(&duplicate) >= duplicate.active);
    let bytes = [birth(S, 1), birth(S, 2)].concat();
    std::fs::write(root.0.join("sessions/a/events.jsonl"), &bytes).expect("append fixture");
    let work = a.prepare().expect("append prepare");
    let payload = Payload::at(&root.0);
    let held = payload.lock(Duration::ZERO).expect("external lock");
    assert!(matches!(a.commit(work).outcome, Outcome::Deferred));
    drop(held);
    run_to_idle(&mut a, 10);
    assert_eq!(view(&mut Files::at(&root.0), None).all.total, 2);
}

#[test]
fn s2_maintenance_never_waits_for_a_journal_lock() {
    let root = Root::new();
    root.live("a", S, &birth(S, 1));
    let journal = root.0.join("sessions/a/events.jsonl");
    let _held = crate::store::lock(&crate::store::lock_path(&journal), Duration::ZERO)
        .expect("journal lock");
    let mut maintainer = Maintainer::new(&root.0, limits(), Duration::ZERO);
    let steps = run_to_idle(&mut maintainer, 10);
    assert!(
        steps
            .iter()
            .any(|s| matches!(s.outcome, Outcome::Published { .. }))
    );
}

#[test]
fn s2_live_partial_tail_survives_observation_and_restart() {
    let root = Root::new();
    let complete_line = birth(S, 1);
    let split = complete_line.len() / 2;
    root.live("a", S, &complete_line[..split]);
    let mut a = Maintainer::new(&root.0, limits(), Duration::ZERO);
    run_to_idle(&mut a, 10);
    drop(a);
    let mut b = Maintainer::new(&root.0, limits(), Duration::ZERO);
    run_to_idle(&mut b, 10);
    let shown = disk::shown(&mut Files::at(&root.0), None);
    let Shown::Ready {
        view: pending_view,
        pending,
        ..
    } = shown
    else {
        panic!("expected registered pending source, got {shown:?}");
    };
    assert_eq!(pending_view.all.total, 0);
    assert!(pending > 0);
    std::fs::write(root.0.join("sessions/a/events.jsonl"), &complete_line)
        .expect("complete live line");
    run_to_idle(&mut b, 10);
    let actual = view(&mut Files::at(&root.0), None);
    let expected = crate::stats::fold(
        vec![crate::stats::Source::live(
            "a",
            format!("session_id={S}\n").as_bytes(),
            Ok(complete_line),
        )],
        None,
    );
    assert_eq!(actual, expected, "no phantom malformed prefix");
}

#[test]
fn s2_line_boundaries_skip_restart_and_bounded_holding_turn() {
    for width in [511, 512, 513, 2048] {
        let root = Root::new();
        let mut bytes = cases::dated(width);
        bytes.extend(birth(S, 1));
        root.live("a", S, &bytes);
        root.live("z", T, &birth(T, 2));
        let lim = Limits {
            chunk: 128,
            line: 512,
        };
        let mut m = Maintainer::new(&root.0, lim, Duration::ZERO);
        let mut steps = Vec::new();
        for _ in 0..6 {
            let s = m.step();
            assert!(s.read <= lim.chunk);
            steps.push(s);
        }
        assert!(
            steps.iter().any(|s| s.source.as_deref() == Some("live/z")),
            "waiting source gets a turn after ceil(line/chunk)+1"
        );
        drop(m);
        let mut m = Maintainer::new(&root.0, lim, Duration::ZERO);
        run_to_idle(&mut m, 100);
        let feeds = vec![
            Feed {
                cursor: bytes.len() as u64,
                active: true,
                ..Feed::live("a", Some(S), bytes)
            },
            Feed {
                cursor: birth(T, 2).len() as u64,
                active: true,
                ..Feed::live("z", Some(T), birth(T, 2))
            },
        ];
        check(&mut Files::at(&root.0), &feeds, lim.line);
    }
}

#[test]
fn s2_more_than_512_sources_are_eventually_published() {
    let root = Root::new();
    // 513 real discovered sources: 510 singletons plus 3 sharing session S.
    // Splitting groups keeps discovery/cardinality at 513 without a
    // 513-record single-group aggregate; max group size 3 keeps debug cost low.
    for n in 0..510u32 {
        let session = format!("{:08x}-0000-4000-8000-{:012x}", n, n);
        root.live(&format!("s{n:04}"), &session, &birth(&session, n));
    }
    for n in 510..513u32 {
        root.live(&format!("s{n:04}"), S, &birth(S, n));
    }
    let mut m = Maintainer::new(&root.0, limits(), Duration::ZERO);
    run_to_idle(&mut m, 1100);
    let mut files = Files::at(&root.0);
    let actual = view(&mut files, None);
    assert_eq!(
        actual.all.total, 513,
        "independent birth oracle: one birth per source"
    );
    assert_eq!(
        actual.all.sources_total, 513,
        "cardinality oracle: all 513 discovered sources materialized"
    );
    let shared = view(&mut files, Some(S));
    let crate::stats::ThisSession::Ready(shared_scope) = &shared.this else {
        panic!("shared session scope ready");
    };
    assert_eq!(
        shared_scope.total, 3,
        "same-group aggregation folds all births in scope"
    );
    assert_eq!(shared_scope.sources_total, 3, "same-group cardinality");
    let solo = format!("{:08x}-0000-4000-8000-{:012x}", 0u32, 0u32);
    let singleton = view(&mut files, Some(solo.as_str()));
    let crate::stats::ThisSession::Ready(single_scope) = &singleton.this else {
        panic!("singleton session scope ready");
    };
    assert_eq!(
        single_scope.total, 1,
        "singleton group does not leak into the shared group"
    );
    assert_eq!(single_scope.sources_total, 1, "singleton group cardinality");
}

#[test]
fn s2_pacing_covers_every_outcome() {
    let active = Duration::from_secs(7);
    let outcomes = vec![
        Outcome::Published {
            from: 0,
            to: 1,
            r#gen: 1,
        },
        Outcome::Holding { from: 0, to: 1 },
        Outcome::Checked { at: 1 },
        Outcome::Discarded(Why::Stamp),
        Outcome::Deferred,
        Outcome::Idle,
        Outcome::Refused(Refused::Io("fixture".to_owned())),
        Outcome::Repaired {
            file: "stats/view".to_owned(),
            reason: "fixture".to_owned(),
        },
    ];
    for outcome in outcomes {
        let step = Step {
            source: None,
            outcome,
            read: 1,
            active,
            held: Duration::ZERO,
        };
        assert!(maint::pause(&step) >= active, "{step:?}");
    }
}

#[test]
fn s2_production_source_exceeds_four_mib_and_publishes_each_chunk() {
    let root = Root::new();
    let mut bytes = Vec::new();
    for n in 0..5 {
        bytes.extend(cases::dated(1 << 20));
        bytes.extend(birth(S, n));
    }
    root.live("a", S, &bytes);
    let mut m = Maintainer::new(&root.0, limits(), Duration::ZERO);
    let first = m.step();
    assert!(first.read <= 4 << 20);
    let Outcome::Published { to, .. } = first.outcome else {
        panic!("first production chunk publishes");
    };
    assert!(to > 0 && to < bytes.len() as u64);
    #[allow(
        clippy::disallowed_methods,
        reason = "acceptance inspects the published view in its private temporary root"
    )]
    let view_is_file = root.0.join("stats/view").is_file();
    assert!(view_is_file, "first chunk durable before next read");
    let steps = run_to_idle(&mut m, 20);
    assert!(
        steps
            .iter()
            .any(|s| matches!(s.outcome,Outcome::Published{from,..} if from==to))
    );
    let f = Feed {
        cursor: bytes.len() as u64,
        active: true,
        ..Feed::live("a", Some(S), bytes)
    };
    check(&mut Files::at(&root.0), &[f], limits().line);
}

#[test]
fn s2_archive_final_non_lf_line_is_counted() {
    let mut feed = Feed::live("archive-a", Some(S), birth(S, 1));
    feed.bytes.pop();
    feed.spec = SourceSpec::Archive {
        id: "archive-a".to_owned(),
        uuid: S.to_owned(),
    };
    let mut rig = Rig::new(vec![feed], 4096, 4096);
    rig.catchup(0);
    assert_eq!(view(&mut rig.dir, None).all.total, 1);
}

#[test]
fn s2_reset_prefix_keeps_coverage_pending_until_caught_up() {
    let mut rig = Rig::new(vec![Feed::live("a", Some(S), birth(S, 1))], 4096, 4096);
    rig.catchup(0);
    let mut replacement = Vec::new();
    for n in 10..16 {
        replacement.extend(birth(S, n));
    }
    rig.feeds[0].bytes = replacement;
    rig.feeds[0].ino += 1;
    rig.sync(0);
    rig.limits.chunk = birth(S, 10).len() as u64;
    let work = rig.ready(0);
    assert!(work.is_reset());
    rig.apply(0, &work);
    assert!(matches!(disk::shown(&mut rig.dir,None),Shown::Ready{pending,..} if pending>0));
    rig.catchup(0);
    assert!(matches!(
        disk::shown(&mut rig.dir, None),
        Shown::Ready {
            pending: 0,
            updating: false,
            ..
        }
    ));
}

#[test]
fn s2_payload_recovery_uses_highest_old_epoch_and_then_cleans_up() {
    let mut dir = Memory::new();
    let (mut journals, mut feeds) = seed(&mut dir);
    let old_zero: Vec<_> = dir
        .names()
        .into_iter()
        .filter_map(|n| dir.file(&n).map(|b| (n.clone(), b.to_vec())))
        .collect();
    let plan = disk::repair(&mut dir).expect("first repair");
    complete(&plan, &mut dir);
    assert_eq!(epoch(&mut dir), 1);
    for f in &mut feeds {
        f.cursor = 0;
        full(&mut dir, &mut journals, f);
    }
    let plan = disk::repair(&mut dir).expect("second repair");
    let cut = plan
        .ops
        .iter()
        .position(|o| matches!(o,Op::Rename{from,..} if from=="stats"))
        .expect("payload move")
        + 1;
    plan.apply(&mut dir, cut).expect("mid second swap");
    for (name, bytes) in old_zero {
        if let Some(suffix) = name.strip_prefix("stats/") {
            dir.plant(&format!("stats.old.0/{suffix}"), &bytes);
        }
    }
    recover(&mut dir);
    assert_eq!(epoch(&mut dir), 2);
    assert!(
        dir.names()
            .iter()
            .all(|n| !n.starts_with("stats.old.") && !n.starts_with("stats.new"))
    );
}

#[test]
fn s2_swap_staging_with_wrong_epoch_restores_old_payload() {
    let mut dir = Memory::new();
    let (_journals, feeds) = seed(&mut dir);
    let old = epoch(&mut dir);
    let plan = disk::repair(&mut dir).expect("repair");
    let moved = plan
        .ops
        .iter()
        .position(|o| matches!(o,Op::Rename{from,..} if from=="stats"))
        .expect("move old")
        + 1;
    plan.apply(&mut dir, moved)
        .expect("gap between payload renames");
    dir.plant("stats.new/epoch", b"999999\n");
    recover(&mut dir);
    assert_eq!(epoch(&mut dir), old);
    check(&mut dir, &feeds, limits().line);
}

#[test]
fn s2_damaged_writer_repairs_once_then_stops_on_repeated_damage() {
    let root = Root::new();
    root.live("a", S, &birth(S, 1));
    let mut m = Maintainer::new(&root.0, limits(), Duration::ZERO);
    run_to_idle(&mut m, 10);
    std::fs::write(root.0.join("stats/view"), b"corrupt-one").expect("damage root view");
    let mut bytes = birth(S, 1);
    bytes.extend(birth(S, 2));
    root.live("a", S, &bytes);
    let repaired = (0..10)
        .map(|_| m.step())
        .find(|s| matches!(s.outcome, Outcome::Repaired { .. }))
        .expect("first corruption reports repair");
    assert!(
        matches!(repaired.outcome,Outcome::Repaired{file,reason} if file.ends_with("stats/view") && !reason.is_empty())
    );
    // Damage again before this rebuild can finish. The same repair episode must stop.
    std::fs::write(root.0.join("stats/view"), b"corrupt-two").expect("damage rebuilt root");
    bytes.extend(birth(S, 3));
    root.live("a", S, &bytes);
    let mut refused = false;
    for _ in 0..10 {
        let step = m.step();
        assert!(
            !matches!(step.outcome, Outcome::Repaired { .. }),
            "repair must not loop"
        );
        if matches!(step.outcome, Outcome::Refused(_)) {
            refused = true;
            break;
        }
    }
    assert!(refused, "repeated damage is reported and stops rebuilding");
}
