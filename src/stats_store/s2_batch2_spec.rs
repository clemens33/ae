//! Batch-2 R1-R5 acceptance, including lead amendments 16 and 17.
#![allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "persisted payload suffixes are case-sensitive protocol names"
)]

use super::{
    Dir, Duration, Feed, Files, Limits, Maintainer, Memory, MemoryJournals, Next, Op, Outcome,
    Payload, Prepared, Read, Root, S, T, birth, cases, check, complete, disk, epoch, full, limits,
    published, recover, run_to_idle, view,
};
use crate::stats_store::doc;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::os::unix::fs::MetadataExt as _;
use std::time::{Instant, SystemTime};

fn backend(files: bool, test: impl FnOnce(&mut dyn Dir)) {
    if files {
        let root = Root::new();
        let payload = Payload::at(&root.0);
        let _lock = payload
            .lock(Duration::ZERO)
            .expect("private Files root lock");
        test(&mut Files::at(&root.0));
    } else {
        test(&mut Memory::new());
    }
}

fn new_primary_cut(dir: &mut dyn Dir, publish_cut: usize, redo_cut: usize) -> (usize, usize) {
    recover(dir);
    let mut journals = MemoryJournals::new();
    let mut a = Feed::live("a", Some(S), birth(S, 1));
    journals.set(a.id(), 1, a.ino, &a.bytes);
    full(dir, &mut journals, &mut a);
    let mut b = Feed::live("b", Some(T), birth(T, 2));
    b.bytes.extend(birth(S, 3));
    journals.set(b.id(), 1, b.ino, &b.bytes);
    let Next::Work(work) = Prepared::read(dir, &mut journals, &b.spec, limits(), None)
        .expect("new primary preparation")
    else {
        panic!("new primary has complete journal lines");
    };
    let plan = published(disk::publish(dir, &mut journals, &work).expect("new primary plan"));
    let primary = plan
        .ops
        .iter()
        .position(|op| {
            matches!(op,
        Op::Rename { to, .. } if to == &format!("stats/session/{T}.evidence"))
        })
        .expect("new primary evidence commit");
    plan.apply(dir, publish_cut.min(primary))
        .expect("publication stops before new primary commits");
    let redo = disk::recovery(dir).expect("uncommitted new primary is recoverable");
    let n = redo.ops.len();
    redo.apply(dir, redo_cut.min(n)).expect("recovery prefix");
    let again = disk::recovery(dir).unwrap_or_else(|why| {
        panic!("second crash is not missing-primary damage: publish={publish_cut}, redo={redo_cut}: {why:?}")
    });
    complete(&again, dir);
    recover(dir);
    check(dir, std::slice::from_ref(&a), limits().line);
    assert_eq!(
        dir.read("stats/intent", disk::INTENT_CAP),
        Read::Absent,
        "recovery removes the intent"
    );
    full(dir, &mut journals, &mut b);
    check(dir, &[a, b], limits().line);
    (primary, n)
}

#[test]
fn s2_r1_new_primary_survives_every_recovery_prefix_before_commit() {
    for files in [false, true] {
        let mut primary = 0;
        backend(files, |dir| primary = new_primary_cut(dir, 0, 0).0);
        for cut in 0..=primary {
            let mut redos = 0;
            backend(files, |dir| redos = new_primary_cut(dir, cut, 0).1);
            for redo in 0..=redos {
                backend(files, |dir| {
                    new_primary_cut(dir, cut, redo);
                });
            }
        }
    }
}

fn put(dir: &mut dyn Dir, name: &str, bytes: Vec<u8>) {
    dir.apply(&Op::Write {
        name: name.to_owned(),
        bytes,
    })
    .expect("exclusive fixture file");
}

fn epoch_bytes(dir: &mut dyn Dir, name: &str) -> Option<u64> {
    match dir.read(name, disk::EPOCH_CAP) {
        Read::Bytes(bytes) => doc::parse_epoch(&bytes).ok(),
        Read::Absent | Read::Oversized | Read::Invalid(_) => None,
        Read::Unreadable(why) => panic!("unexpected fixture IO at {name}: {why}"),
    }
}

fn document_epoch(dir: &mut dyn Dir, name: &str, session: bool) -> Option<u64> {
    let cap = if session {
        disk::SESSION_VIEW_CAP
    } else {
        disk::ROOT_VIEW_CAP
    };
    match dir.read(name, cap) {
        Read::Bytes(bytes) => {
            let text = std::str::from_utf8(&bytes).ok()?;
            if session {
                doc::parse_session(text).ok().map(|view| view.epoch)
            } else {
                doc::parse_root(text).ok().map(|root| root.epoch)
            }
        }
        Read::Absent | Read::Oversized | Read::Invalid(_) => None,
        Read::Unreadable(why) => panic!("unexpected fixture IO at {name}: {why}"),
    }
}

// Read the documents amendment 12 names. Expected watermarks are fixture constants;
// this does not call the authority/recovery/repair decisions being judged.
fn maximum_proof(dir: &mut dyn Dir) -> Option<u64> {
    let mut proofs = vec![
        epoch_bytes(dir, "stats/epoch"),
        document_epoch(dir, "stats/view", false),
    ];
    for name in dir.list("stats/session").expect("fixture session listing") {
        if name.ends_with(".view") {
            proofs.push(document_epoch(dir, &format!("stats/session/{name}"), true));
        }
    }
    for name in dir.list("").expect("fixture top listing") {
        if let Some(old) = name
            .strip_prefix("stats.old.")
            .and_then(|n| n.parse::<u64>().ok())
            && epoch_bytes(dir, &format!("{name}/epoch")) == Some(old)
        {
            proofs.push(Some(old));
        }
    }
    // stats.new/epoch is deliberately NOT amendment-12 evidence.
    proofs.into_iter().flatten().max()
}

fn authority_fixture(dir: &mut dyn Dir, active: Option<u64>, old: u64, view_epoch: Option<u64>) {
    dir.apply(&Op::MakeDir("stats".to_owned()))
        .expect("fixture payload");
    dir.apply(&Op::MakeDir("stats/session".to_owned()))
        .expect("fixture sessions");
    put(
        dir,
        "stats/epoch",
        active.map_or_else(
            || b"damaged\n".to_vec(),
            |e| doc::render_epoch(e).into_bytes(),
        ),
    );
    if let Some(value) = view_epoch {
        put(
            dir,
            "stats/view",
            doc::render_root(&doc::Root::empty(value)).into_bytes(),
        );
    }
    dir.apply(&Op::MakeDir(format!("stats.old.{old}")))
        .expect("retired fixture");
    put(
        dir,
        &format!("stats.old.{old}/epoch"),
        doc::render_epoch(old).into_bytes(),
    );
}

fn settle_after_cut(dir: &mut dyn Dir, required: u64) {
    for _ in 0..4 {
        match disk::recovery(dir) {
            Ok(plan) => complete(&plan, dir),
            Err(why) => {
                assert!(
                    why.damage().is_some(),
                    "fixture recovery refusal is damage: {why:?}"
                );
                complete(
                    &disk::repair(dir).expect("takeover keeps epoch authority"),
                    dir,
                );
            }
        }
        if epoch_bytes(dir, "stats/epoch").is_some_and(|e| e >= required) {
            return;
        }
        complete(&disk::repair(dir).expect("resume unfinished repair"), dir);
    }
    panic!("recovery and repair never reached the required epoch {required}");
}

fn authority_cut(dir: &mut dyn Dir, active: Option<u64>, lower_view: bool, cut: usize) -> usize {
    authority_fixture(dir, active, 9, lower_view.then_some(3));
    put(dir, "stats/epoch.tmp", b"unpublished epoch\n".to_vec());
    assert_eq!(maximum_proof(dir), Some(9));
    let plan = disk::repair(dir).expect("retired payload proves the takeover epoch");
    let n = plan.ops.len();
    plan.apply(dir, cut.min(n)).expect("takeover crash prefix");
    assert!(
        maximum_proof(dir).is_some_and(|e| e >= 9),
        "every takeover prefix preserves the proven watermark: active={active:?}, view={lower_view}, cut={cut}"
    );
    if cut >= n {
        assert_eq!(epoch(dir), 10, "full repair is max proof plus one");
    }
    settle_after_cut(dir, 10);
    assert!(epoch(dir) >= 10, "a retry cannot reuse an older epoch");
    n
}

#[test]
fn s2_r2_takeover_preserves_epoch_authority_at_every_prefix() {
    for files in [false, true] {
        for active in [None, Some(3)] {
            for lower_view in [false, true] {
                let mut n = 0;
                backend(files, |dir| {
                    n = authority_cut(dir, active, lower_view, usize::MAX);
                });
                for cut in 0..=n {
                    backend(files, |dir| {
                        authority_cut(dir, active, lower_view, cut);
                    });
                }
            }
        }
    }
}

fn snapshot(dir: &mut dyn Dir) -> BTreeMap<String, Read> {
    let mut result = BTreeMap::new();
    let mut todo = vec![String::new()];
    while let Some(parent) = todo.pop() {
        for child in dir.list(&parent).expect("fixture snapshot listing") {
            let name = if parent.is_empty() {
                child
            } else {
                format!("{parent}/{child}")
            };
            let value = dir.read(&name, u64::MAX);
            if matches!(value, Read::Invalid(_)) {
                todo.push(name.clone());
            }
            assert!(!matches!(value, Read::Unreadable(_)), "snapshot IO");
            result.insert(name, value);
        }
    }
    result
}

#[test]
fn s2_r2b_retired_epoch_at_or_above_active_refuses_without_mutation() {
    for files in [false, true] {
        for old in [3, 9] {
            backend(files, |dir| {
                authority_fixture(dir, Some(3), old, Some(3));
                let before = snapshot(dir);
                let why = disk::recovery(dir).expect_err("equal or higher retired proof is damage");
                assert!(why.damage().is_some());
                let text = why.describe();
                assert!(
                    text.contains('3')
                        && text.contains(&old.to_string())
                        && text.contains(&format!("stats.old.{old}")),
                    "damage names both epochs and the retired path: {text}"
                );
                assert_eq!(snapshot(dir), before, "refusal keeps every file");
                let plan = disk::repair(dir).expect("amendment 15 takes over");
                complete(&plan, dir);
                assert_eq!(epoch(dir), old + 1);
                assert!(
                    dir.list("")
                        .expect("final root")
                        .iter()
                        .all(|name| !name.starts_with("stats.old."))
                );
            });
        }
        backend(files, |dir| {
            authority_fixture(dir, Some(3), 1, Some(3));
            recover(dir);
            assert_eq!(epoch(dir), 3, "lower retired proof is ordinary cleanup");
            assert_eq!(dir.read("stats.old.1/epoch", disk::EPOCH_CAP), Read::Absent);
        });
    }
}

#[test]
fn s2_r3_cold_caught_up_fleet_checks_one_witness_per_step() {
    let root = Root::new();
    let mut journal = cases::dated(8192);
    journal.extend(birth(S, 1));
    for name in ["a", "b", "c"] {
        root.live(name, S, &journal);
    }
    let lim = Limits {
        chunk: 1024,
        line: 4 << 20,
    };
    let mut first = Maintainer::new(&root.0, lim, Duration::ZERO);
    run_to_idle(&mut first, 80);
    drop(first);
    let mut cold = Maintainer::new(&root.0, lim, Duration::ZERO);
    let mut observed = BTreeSet::new();
    let _ = disk::take_journal_reads();
    for _ in 0..3 {
        let step = cold.step();
        let reads = disk::take_journal_reads();
        assert!(
            reads.total > 0 && reads.total <= 4096,
            "one cold source witness per step, never fleet-wide accumulation: {step:?}, {reads:?}"
        );
        assert_eq!(reads.under_lock, 0);
        assert_eq!(step.read, 0, "all source prefixes were already consumed");
        assert!(matches!(step.outcome, Outcome::Checked { at } if at == journal.len() as u64));
        assert!(super::maint::pause(&step) >= step.active);
        assert!(observed.insert(step.source.expect("checked source is named")));
    }
    assert_eq!(
        observed,
        BTreeSet::from([
            "live/a".to_owned(),
            "live/b".to_owned(),
            "live/c".to_owned()
        ])
    );
    for _ in 0..8 {
        let step = cold.step();
        assert!(matches!(step.outcome, Outcome::Idle));
        assert_eq!(
            disk::take_journal_reads().total,
            0,
            "warm unchanged stat reads no content"
        );
    }
}

fn metadata(path: &std::path::Path) -> std::fs::Metadata {
    #[allow(
        clippy::disallowed_methods,
        reason = "acceptance fixture checks device/inode/length and nanosecond timestamp controls"
    )]
    let meta = std::fs::metadata(path).expect("fixture journal metadata");
    assert!(meta.is_file(), "the witness fixture is a regular file");
    meta
}

fn stamp(meta: &std::fs::Metadata) -> ((i64, i64), (i64, i64)) {
    (
        (meta.mtime(), meta.mtime_nsec()),
        (meta.ctime(), meta.ctime_nsec()),
    )
}

fn changed_cached_witness(keep_mtime: bool) {
    let root = Root::new();
    let original = birth(S, 1);
    let text = std::str::from_utf8(&original).expect("fixture text");
    let field = "\"birth_model\":\"sol\"";
    assert_eq!(text.matches(field).count(), 1);
    let changed = text.replace(field, "\"birth_model\":\"alt\"").into_bytes();
    assert_eq!(changed.len(), original.len());
    root.live("a", S, &original);
    let path = root.0.join("sessions/a/events.jsonl");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(false)
        .open(&path)
        .expect("in-place fixture handle");
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(10))
        .expect("distinct initial mtime");
    let mut m = Maintainer::new(&root.0, limits(), Duration::ZERO);
    run_to_idle(&mut m, 12);
    let before_view = view(&mut Files::at(&root.0), None);
    let _ = disk::take_journal_reads();
    for _ in 0..4 {
        assert!(matches!(m.step().outcome, Outcome::Idle));
    }
    assert_eq!(
        disk::take_journal_reads().total,
        0,
        "the real cache is warm"
    );
    let before = metadata(&path);
    let before_mtime = before.modified().expect("original modified time");
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        use std::io::{Seek as _, SeekFrom};
        file.seek(SeekFrom::Start(0))
            .expect("rewrite from byte zero");
        file.write_all(&changed)
            .expect("same-length rewrite inside witness");
        if keep_mtime {
            file.set_modified(before_mtime)
                .expect("preserve mtime so ctime must detect the rewrite");
        }
        let now = metadata(&path);
        assert_eq!(
            (now.dev(), now.ino(), now.len()),
            (before.dev(), before.ino(), before.len())
        );
        if keep_mtime {
            assert_eq!(now.modified().expect("preserved mtime"), before_mtime);
            if stamp(&now).1 != stamp(&before).1 {
                break;
            }
        } else {
            assert_ne!(stamp(&now).0, stamp(&before).0);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "fixture must produce a distinct ctime timestamp"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    let first = m.step();
    assert!(
        disk::take_journal_reads().total > 0,
        "a changed timestamp invalidates the quiet cache and checks the witness: {first:?}"
    );
    assert_eq!(first.source.as_deref(), Some("live/a"));
    run_to_idle(&mut m, 12);
    let mut feed = Feed::live("a", Some(S), changed);
    feed.active = true;
    feed.cursor = feed.bytes.len() as u64;
    check(&mut Files::at(&root.0), &[feed], limits().line);
    assert_ne!(
        view(&mut Files::at(&root.0), None),
        before_view,
        "the rewrite changes the fold answer"
    );
    let _ = disk::take_journal_reads();
    for _ in 0..4 {
        assert!(matches!(m.step().outcome, Outcome::Idle));
    }
    assert_eq!(
        disk::take_journal_reads().total,
        0,
        "unchanged updated identity stays stat-only"
    );
}

#[test]
fn s2_r5_warm_cache_detects_same_inode_length_rewrite_with_mtime_change() {
    changed_cached_witness(false);
}

#[test]
fn s2_r5_warm_cache_detects_ctime_change_when_mtime_is_preserved() {
    changed_cached_witness(true);
}
