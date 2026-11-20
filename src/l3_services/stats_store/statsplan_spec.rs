//! statsplan acceptance, independently authored by the navigator.
//! Oracle: held-only publication on the same final store state.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and independent acceptance assertions"
)]

use super::disk::{self, Dir, Files, Op, Plan, Read, Tools};
use super::s4_behavior::Fixture;
use crate::tool_stats::doc;
use crate::tool_stats::incr::{self, Conv, Coverage, Published};
use crate::tool_stats::{Adapter, store};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::time::Duration;

type Built = Result<Plan, String>;

struct Controls;

impl Drop for Controls {
    fn drop(&mut self) {
        disk::clear_before_lock();
        disk::set_snapshot_cap(None);
    }
}

fn reset() -> Controls {
    disk::clear_before_lock();
    disk::set_snapshot_cap(None);
    let _ = disk::take_attempts();
    let _ = disk::take_file_events();
    let _ = disk::take_payload_reads();
    let _ = disk::take_holds();
    Controls
}

fn unlocked_reads() {
    let reads = disk::take_payload_reads();
    assert_eq!(reads.opened_under_lock, 0, "{reads:?}");
    assert_eq!(reads.listed_under_lock, 0, "{reads:?}");
    assert_eq!(reads.opened_under_lock_exception, 0, "{reads:?}");
    assert_eq!(reads.listed_under_lock_exception, 0, "{reads:?}");
}

fn documents(generation: u64) -> (Conv, Published, Coverage) {
    let mut conv = Conv::new(Adapter::Claude, "statsplan-fixture");
    if generation > 1 {
        conv.mark_rebuilt();
    }
    let view = Published {
        generation,
        convs: vec![incr::publish(&conv.summary, None)],
        over_cap: 0,
    };
    let coverage = Coverage {
        generation,
        hidden: generation,
        ..Coverage::default()
    };
    (conv, view, coverage)
}

fn seed() -> Fixture {
    let root = Fixture::new();
    fs::create_dir_all(root.0.join("stats")).expect("owned payload");
    fs::write(root.0.join("stats/epoch"), b"1\n").expect("valid epoch");
    let lock = disk::Payload::at(&root.0)
        .lock(Duration::ZERO)
        .expect("fixture owns the lock");
    let mut dir = Files::at(&root.0);
    let (conv, view, coverage) = documents(1);
    store::publish_with(&mut dir, &conv, &view, &coverage)
        .expect("initial publication")
        .apply(&mut dir, usize::MAX)
        .expect("initial bytes");
    drop(lock);
    root
}

fn publication(dir: &mut dyn Dir, _epoch: u64) -> Built {
    let repair = store::recovery(dir).map_err(|why| format!("{why:?}"))?;
    if !repair.ops.is_empty() {
        return Ok(repair);
    }
    let (conv, view, coverage) = documents(2);
    store::publish_with(dir, &conv, &view, &coverage).map_err(|why| format!("{why:?}"))
}

fn run(root: &Path, build: impl Fn(&mut dyn Dir, u64) -> Built) -> (u64, Built) {
    let result = disk::tools(root, Duration::from_secs(1), |dir, epoch| {
        let made = build(dir, epoch);
        (made.as_ref().ok().cloned(), made)
    });
    match result {
        Tools::Done { epoch, value } => (epoch, value),
        other => panic!("transaction must decide: {other:?}"),
    }
}

// This oracle deliberately reads the real current state under its lock.
// It bypasses both the warm cache and the optimized tools executor.
fn held_only(root: &Path, build: impl Fn(&mut dyn Dir, u64) -> Built) -> (u64, Built) {
    let lock = disk::Payload::at(root)
        .lock(Duration::ZERO)
        .expect("held-only reference lock");
    let mut dir = Files::at(root);
    let epoch = disk::active_epoch(&mut dir)
        .expect("readable epoch")
        .expect("active payload");
    let made = build(&mut dir, epoch);
    if let Ok(plan) = &made {
        plan.apply(&mut dir, usize::MAX)
            .expect("held-only reference applies");
    }
    drop(lock);
    (epoch, made)
}

fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, at: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(at).expect("owned directory") {
            let path = entry.expect("owned entry").path();
            if path.is_dir() {
                walk(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root)
                        .expect("contained fixture")
                        .to_string_lossy()
                        .into_owned(),
                    fs::read(path).expect("owned file"),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(root, &root.join("stats/tools"), &mut files);
    files
}

#[test]
fn statsplan_unchanged_publication_builds_once_outside_the_lock() {
    let root = seed();
    let reference = seed();
    let _controls = reset();
    let calls = Cell::new(0);
    let actual = run(&root.0, |dir, epoch| {
        calls.set(calls.get() + 1);
        let path = root.0.clone();
        assert!(
            std::thread::spawn(move || { disk::Payload::at(&path).lock(Duration::ZERO).is_ok() })
                .join()
                .expect("lock observer"),
            "the builder must run without stats.lock"
        );
        publication(dir, epoch)
    });
    assert_eq!(calls.get(), 1, "one build for one unchanged attempt");
    assert_eq!(disk::take_attempts(), vec![disk::Attempt::Applied]);
    unlocked_reads();
    assert_eq!(actual, held_only(&reference.0, publication));
    assert_eq!(snapshot(&root.0), snapshot(&reference.0));
}

#[test]
fn statsplan_actual_document_plans_equal_the_held_only_reference_byte_for_byte() {
    for state in ["normal", "absent", "malformed", "oversized"] {
        let root = seed();
        let reference = seed();
        for path in [&root.0, &reference.0] {
            let view = path.join("stats/tools/view");
            match state {
                "absent" => fs::remove_file(view).expect("absent view"),
                "malformed" => fs::write(view, b"not a view\n").expect("damaged view"),
                "oversized" => fs::OpenOptions::new()
                    .write(true)
                    .open(view)
                    .expect("existing view")
                    .set_len(doc::VIEW_CAP + 1)
                    .expect("oversized sparse view"),
                _ => {}
            }
        }
        let _controls = reset();
        let actual = run(&root.0, publication);
        unlocked_reads();
        assert_eq!(actual, held_only(&reference.0, publication), "{state}");
        assert_eq!(snapshot(&root.0), snapshot(&reference.0), "{state}");
    }
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "the probe builders return its Built as their own"
)]
fn answer(bytes: Vec<u8>) -> Built {
    Ok(Plan::of(vec![
        Op::Write {
            name: "stats/tools/answer.tmp".to_owned(),
            bytes,
        },
        Op::Rename {
            from: "stats/tools/answer.tmp".to_owned(),
            to: "stats/tools/answer".to_owned(),
        },
    ]))
}

fn probe(dir: &mut dyn Dir, epoch: u64) -> Built {
    let read = dir.read("stats/tools/dependency", 64);
    let names = dir.list("stats/tools/watched").map_err(|why| why.kind());
    answer(format!("{epoch}:{read:?}:{names:?}").into_bytes())
}

#[derive(Debug, Clone, Copy)]
enum Change {
    Bytes,
    Absent,
    Shape,
    Oversized,
    Epoch,
    Names,
    AbsentDir,
    FailedDir,
}

fn prepare(root: &Path, change: Change) {
    let file = root.join("stats/tools/dependency");
    let dir = root.join("stats/tools/watched");
    fs::write(&file, b"old").expect("read dependency");
    fs::create_dir(&dir).expect("listing dependency");
    match change {
        Change::Absent => fs::remove_file(file).expect("absent read dependency"),
        Change::Shape => {
            fs::remove_file(&file).expect("replace regular node");
            symlink("missing-target", file).expect("refused symlink dependency");
        }
        Change::Oversized => fs::write(file, [b'x'; 65]).expect("over read cap"),
        Change::AbsentDir => fs::remove_dir(dir).expect("absent listing"),
        Change::FailedDir => {
            fs::remove_dir(&dir).expect("replace directory");
            fs::write(dir, b"regular node").expect("listing fails on regular node");
        }
        Change::Bytes | Change::Epoch | Change::Names => {}
    }
}

fn replace(root: &Path, name: &str, bytes: &[u8]) {
    fs::write(root.join("replacement"), bytes).expect("fresh replacement inode");
    fs::rename(root.join("replacement"), root.join(name)).expect("atomic replacement");
}

fn mutate(root: &Path, change: Change) {
    match change {
        Change::Epoch => replace(root, "stats/epoch", b"2\n"),
        Change::Shape => {
            fs::remove_file(root.join("stats/tools/dependency")).expect("old symlink");
            fs::create_dir(root.join("stats/tools/dependency")).expect("different shape");
        }
        Change::Names | Change::AbsentDir | Change::FailedDir => {
            let dir = root.join("stats/tools/watched");
            if matches!(change, Change::FailedDir) {
                fs::remove_file(&dir).expect("replace failed listing node");
            }
            if !dir.exists() {
                fs::create_dir(&dir).expect("directory appears");
            }
            fs::write(dir.join("new-entry"), b"new").expect("membership changes");
        }
        Change::Bytes | Change::Absent | Change::Oversized => {
            replace(root, "stats/tools/dependency", b"new");
        }
    }
}

#[test]
fn statsplan_each_changed_file_epoch_or_listing_rewarms_before_any_write() {
    for change in [
        Change::Bytes,
        Change::Absent,
        Change::Shape,
        Change::Oversized,
        Change::Epoch,
        Change::Names,
        Change::AbsentDir,
        Change::FailedDir,
    ] {
        let root = seed();
        let reference = seed();
        prepare(&root.0, change);
        prepare(&reference.0, change);
        mutate(&reference.0, change);
        let _controls = reset();
        disk::before_lock(move |path, attempt| {
            assert!(
                disk::take_file_events().is_empty(),
                "a stale attempt must write nothing"
            );
            if attempt == 1 {
                mutate(path, change);
            }
        });
        let calls = Cell::new(0);
        let actual = run(&root.0, |dir, epoch| {
            calls.set(calls.get() + 1);
            probe(dir, epoch)
        });
        disk::clear_before_lock();
        assert_eq!(calls.get(), 2, "one build per warm: {change:?}");
        assert_eq!(
            disk::take_attempts(),
            vec![disk::Attempt::Stale, disk::Attempt::Applied],
            "{change:?}"
        );
        unlocked_reads();
        assert_eq!(actual, held_only(&reference.0, probe), "{change:?}");
        assert_eq!(snapshot(&root.0), snapshot(&reference.0), "{change:?}");
    }
}

#[test]
fn statsplan_exhausted_stale_attempts_apply_nothing() {
    let root = seed();
    prepare(&root.0, Change::Bytes);
    let _controls = reset();
    disk::before_lock(|path, _| {
        assert!(disk::take_file_events().is_empty());
        replace(path, "stats/tools/dependency", b"new");
    });
    let calls = Cell::new(0);
    let result = disk::tools(&root.0, Duration::from_secs(1), |dir, epoch| {
        calls.set(calls.get() + 1);
        let built = probe(dir, epoch).expect("probe plan");
        (Some(built), ())
    });
    assert!(matches!(result, Tools::Busy), "{result:?}");
    assert_eq!(calls.get(), 3, "three bounded warm builds");
    assert_eq!(disk::take_attempts(), vec![disk::Attempt::Stale; 3]);
    assert!(disk::take_file_events().is_empty());
    assert!(!root.0.join("stats/tools/answer").exists());
    unlocked_reads();
}

struct Race<'a> {
    inner: &'a mut dyn Dir,
    root: &'a Path,
    fired: &'a Cell<bool>,
    listing: bool,
}

fn intervene(root: &Path, listing: bool) {
    let root = root.to_path_buf();
    std::thread::spawn(move || {
        let result = disk::tools(&root, Duration::from_secs(1), |_, _| {
            let ops = if listing {
                vec![Op::Write {
                    name: "stats/tools/view.tmp".to_owned(),
                    bytes: b"interrupted publication".to_vec(),
                }]
            } else {
                vec![
                    Op::Write {
                        name: "stats/tools/change.tmp".to_owned(),
                        bytes: b"new".to_vec(),
                    },
                    Op::Rename {
                        from: "stats/tools/change.tmp".to_owned(),
                        to: "stats/tools/dependency".to_owned(),
                    },
                ]
            };
            (Some(Plan::of(ops)), ())
        });
        assert!(matches!(result, Tools::Done { .. }), "{result:?}");
    })
    .join()
    .expect("a real competing writer completes under stats.lock");
}

impl Dir for Race<'_> {
    fn read(&mut self, name: &str, cap: u64) -> Read {
        let read = self.inner.read(name, cap);
        if !self.listing && name == "stats/tools/dependency" && !self.fired.replace(true) {
            intervene(self.root, false);
        }
        read
    }

    fn list(&mut self, name: &str) -> std::io::Result<Vec<String>> {
        let names = self.inner.list(name);
        if self.listing && name == "stats/tools" && !self.fired.replace(true) {
            intervene(self.root, true);
        }
        names
    }

    fn apply(&mut self, op: &Op) -> std::io::Result<()> {
        self.inner.apply(op)
    }
}

fn repeated_read(dir: &mut dyn Dir, epoch: u64) -> Built {
    let first = dir.read("stats/tools/dependency", 64);
    let second = dir.read("stats/tools/dependency", 64);
    answer(format!("{epoch}:{first:?}:{second:?}").into_bytes())
}

fn intra_warm(listing: bool) {
    let root = seed();
    let reference = seed();
    if !listing {
        prepare(&root.0, Change::Bytes);
        prepare(&reference.0, Change::Bytes);
    }
    intervene(&reference.0, listing);
    let _controls = reset();
    let fired = Cell::new(false);
    let calls = Cell::new(0);
    let actual = run(&root.0, |dir, epoch| {
        calls.set(calls.get() + 1);
        let mut race = Race {
            inner: dir,
            root: &root.0,
            fired: &fired,
            listing,
        };
        if listing {
            publication(&mut race, epoch)
        } else {
            repeated_read(&mut race, epoch)
        }
    });
    assert!(fired.get(), "the opposing writer actually ran");
    assert_eq!(calls.get(), 2, "torn warm is discarded: listing={listing}");
    assert_eq!(
        disk::take_attempts(),
        vec![disk::Attempt::Stale, disk::Attempt::Applied],
        "listing={listing}"
    );
    unlocked_reads();
    let expected = if listing {
        held_only(&reference.0, publication)
    } else {
        held_only(&reference.0, repeated_read)
    };
    assert_eq!(actual, expected, "listing={listing}");
    assert_eq!(
        snapshot(&root.0),
        snapshot(&reference.0),
        "listing={listing}"
    );
}

#[test]
fn statsplan_an_intra_warm_file_change_cannot_hide_behind_the_last_record() {
    intra_warm(false);
}

#[test]
fn statsplan_an_intra_warm_listing_change_cannot_hide_behind_the_last_record() {
    intra_warm(true);
}

#[test]
fn statsplan_a_stable_unreadable_document_is_refused_without_writes() {
    let root = seed();
    let view = root.0.join("stats/tools/view");
    fs::remove_file(&view).expect("replace the old view");
    symlink("missing-target", &view).expect("unreadable view shape");
    let _controls = reset();
    let (_, result) = run(&root.0, publication);
    assert!(result.is_err(), "an IO refusal is never a rebuild");
    assert_eq!(disk::take_attempts(), vec![disk::Attempt::Decided]);
    assert!(disk::take_file_events().is_empty());
    assert_eq!(
        fs::read_link(view).expect("the refused node survives"),
        Path::new("missing-target")
    );
    unlocked_reads();
}

#[test]
fn statsplan_snapshot_over_cap_refuses_before_taking_the_lock() {
    let root = seed();
    let _controls = reset();
    disk::set_snapshot_cap(Some(0));
    let result = disk::tools(&root.0, Duration::from_secs(1), |dir, epoch| {
        let built = publication(dir, epoch);
        (built.as_ref().ok().cloned(), built)
    });
    assert!(
        matches!(result, Tools::Failed(ref why) if why.contains("over the cap")),
        "{result:?}"
    );
    assert!(disk::take_attempts().is_empty());
    assert!(disk::take_holds().is_empty());
    assert!(disk::take_file_events().is_empty());
    unlocked_reads();
}

#[test]
fn statsplan_a_symlinked_tools_directory_keeps_the_held_refusal_and_every_byte() {
    let root = seed();
    let reference = seed();
    for path in [&root.0, &reference.0] {
        fs::rename(path.join("stats/tools"), path.join("tools-target"))
            .expect("move the real store");
        symlink("../tools-target", path.join("stats/tools")).expect("directory symlink");
        assert!(
            Files::at(path).list("stats/tools").is_ok(),
            "plain reads can list the link; held transactions refuse it"
        );
    }
    let before = snapshot(&root.0);
    let _controls = reset();
    let mut held = Duration::ZERO;
    let original = disk::transact(
        &reference.0,
        &mut Files::at(&reference.0),
        &mut disk::FileJournals::at(&reference.0),
        Duration::from_secs(1),
        &mut held,
        |files, _| {
            let epoch = disk::active_epoch(files)
                .expect("reference epoch")
                .expect("reference payload");
            let made = publication(files, epoch);
            if let Ok(plan) = &made {
                plan.apply(files, usize::MAX)
                    .expect("warm is inert; held refuses before a plan");
            }
            made
        },
    );
    assert!(
        matches!(original, disk::Taken::Done(Err(_))),
        "the original transaction decides a listing refusal: {original:?}"
    );
    assert!(disk::take_file_events().is_empty());
    unlocked_reads();
    let _candidate_controls = reset();
    let (_, result) = run(&root.0, publication);
    assert!(
        result.is_err(),
        "the warm success cannot replace held refusal"
    );
    assert_eq!(disk::take_attempts(), vec![disk::Attempt::Decided]);
    assert!(disk::take_file_events().is_empty());
    assert_eq!(
        snapshot(&root.0),
        before,
        "every target-store byte survives"
    );
    assert_eq!(
        fs::read_link(root.0.join("stats/tools")).expect("the link survives"),
        Path::new("../tools-target")
    );
    unlocked_reads();
}
