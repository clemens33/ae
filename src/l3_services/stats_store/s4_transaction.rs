//! S4 option-A acceptance. Frozen S4 v1; navigator revision and lead gate required for changes.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and assertions"
)]

use super::disk::{self, Attempt, Dir, Files, Read, Refused};
use super::maint::Outcome;
use super::s4_behavior::{Fixture, S, T, birth, drain, source};
use std::fs;
use std::path::Path;
use std::time::Duration;

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
    let _ = disk::take_payload_reads();
    let _ = disk::take_journal_reads();
    let _ = disk::take_file_events();
    let _ = disk::take_attempts();
    Controls
}

fn no_locked_reads() {
    let reads = disk::take_payload_reads();
    assert_eq!(
        reads.opened_under_lock, 0,
        "no steady-state payload content opens: {reads:?}"
    );
    assert_eq!(
        reads.listed_under_lock, 0,
        "no steady-state directory listings: {reads:?}"
    );
    assert_eq!(
        reads.opened_under_lock_exception, 0,
        "fixture has no owed intent or damage"
    );
    assert_eq!(
        reads.listed_under_lock_exception, 0,
        "fixture has no owed intent or damage"
    );
    assert_eq!(
        disk::take_journal_reads().under_lock,
        0,
        "no held witness content reads"
    );
}

#[test]
fn s4_observer_counts_a_real_payload_open_and_a_real_directory_listing() {
    let root = Fixture::new();
    root.live("alpha", S, &birth(S, 1));
    drain(&mut root.maintainer());
    let _controls = reset();
    let mut files = Files::at(&root.0);
    assert!(matches!(
        files.read("stats/view", disk::ROOT_VIEW_CAP),
        Read::Bytes(_)
    ));
    assert!(
        !files
            .list("stats/session")
            .expect("real directory")
            .is_empty()
    );
    let reads = disk::take_payload_reads();
    assert!(
        reads.opened > 0 && reads.listed > 0,
        "observer must see the actual opens: {reads:?}"
    );
    assert_eq!(reads.opened_under_lock, 0);
    assert_eq!(reads.listed_under_lock, 0);
}

#[test]
fn s4_unchanged_dependencies_publish_on_the_first_attempt_without_locked_reads() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    let mut maintainer = root.maintainer();
    let work = maintainer.prepare().expect("first registration");
    let _controls = reset();
    let step = maintainer.commit(work);
    assert!(
        matches!(step.outcome, Outcome::Published { .. }),
        "{step:?}"
    );
    assert_eq!(
        disk::take_attempts(),
        vec![Attempt::Applied],
        "neutral control: first attempt is usable"
    );
    no_locked_reads();
    drain(&mut maintainer);
    root.check(|| vec![source(&dir, "alpha")]);
}

fn changed_dependency(change: impl Fn(&Path) + 'static) {
    let root = Fixture::new();
    let first = birth(S, 1);
    let dir = root.live("alpha", S, &first);
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    fs::write(dir.join("events.jsonl"), [first, birth(S, 2)].concat()).expect("append fixture");
    let work = maintainer.prepare().expect("incremental preparation");
    let _controls = reset();
    disk::before_lock(move |root, attempt| {
        if attempt == 1 {
            change(root);
        } else {
            assert!(
                disk::take_file_events().is_empty(),
                "a stale attempt must apply no operation"
            );
        }
    });
    let step = maintainer.commit(work);
    assert!(
        matches!(step.outcome, Outcome::Published { .. }),
        "retry must publish valid prepared data: {step:?}"
    );
    assert_eq!(
        disk::take_attempts(),
        vec![Attempt::Stale, Attempt::Applied]
    );
    no_locked_reads();
    disk::clear_before_lock();
    drain(&mut maintainer);
    root.check(|| vec![source(&dir, "alpha")]);
}

#[test]
fn s4_replaced_payload_dependency_retries_outside_the_lock() {
    changed_dependency(|root| {
        let view = root.join("stats/view");
        let bytes = fs::read(&view).expect("readable old root view");
        fs::write(root.join("replacement"), bytes).expect("fresh inode");
        fs::rename(root.join("replacement"), view).expect("atomic equivalent replacement");
    });
}

#[test]
fn s4_removed_payload_dependency_retries_outside_the_lock() {
    changed_dependency(|root| fs::remove_file(root.join("stats/view")).expect("root view removed"));
}

#[test]
fn s4_changed_directory_membership_retries_outside_the_lock() {
    changed_dependency(|root| {
        fs::write(root.join("stats/session/unused-entry"), b"inert")
            .expect("directory entry added");
    });
}

#[test]
fn s4_journal_append_after_warm_retries_without_a_held_witness_open() {
    changed_dependency(|root| {
        crate::store::open(&root.join("sessions/alpha"))
            .append_event(std::str::from_utf8(&birth(S, 3)).expect("fixture JSON"))
            .expect("journal append through its actual lock");
    });
}

#[test]
fn s4_every_changed_attempt_defers_without_any_publication() {
    let root = Fixture::new();
    let first = birth(S, 1);
    let dir = root.live("alpha", S, &first);
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    fs::write(dir.join("events.jsonl"), [first, birth(S, 2)].concat()).expect("append fixture");
    let work = maintainer.prepare().expect("incremental work");
    let before = fs::read(root.0.join("stats/view")).expect("pre-state");
    let _controls = reset();
    disk::before_lock(|root, _| {
        assert!(
            disk::take_file_events().is_empty(),
            "no writes before retry exhaustion"
        );
        let bytes = fs::read(root.join("stats/view")).expect("unchanged bytes");
        fs::write(root.join("replacement"), bytes).expect("fresh inode");
        fs::rename(root.join("replacement"), root.join("stats/view"))
            .expect("replace on every attempt");
    });
    let step = maintainer.commit(work);
    assert_eq!(step.outcome, Outcome::Deferred, "bounded stale retries");
    assert_eq!(disk::take_attempts(), vec![Attempt::Stale; 3]);
    assert!(
        disk::take_file_events().is_empty(),
        "no partial plan may apply"
    );
    assert_eq!(
        fs::read(root.0.join("stats/view")).expect("unchanged view"),
        before
    );
    no_locked_reads();
    disk::clear_before_lock();
    drain(&mut maintainer);
    root.check(|| vec![source(&dir, "alpha")]);
}

#[test]
fn s4_dependency_appearing_after_absence_preserves_the_other_writer() {
    let root = Fixture::new();
    let alpha = root.live("alpha", S, &birth(S, 1));
    let mut maintainer = root.maintainer();
    let work = maintainer.prepare().expect("absent-store preparation");
    let _controls = reset();
    disk::before_lock(|root, attempt| {
        if attempt == 1 {
            let root = root.to_path_buf();
            std::thread::spawn(move || {
                let beta = root.join("sessions/beta");
                fs::create_dir_all(&beta).expect("second source");
                fs::write(beta.join("meta"), format!("session=beta\nsession_id={T}\n"))
                    .expect("second UUID");
                fs::write(beta.join("events.jsonl"), birth(T, 2)).expect("second birth");
                let mut other =
                    super::maint::Maintainer::new(&root, disk::LIMITS, Duration::from_millis(100));
                drain(&mut other);
            })
            .join()
            .expect("foreign writer completes");
        } else {
            assert!(
                disk::take_file_events().is_empty(),
                "stale absence must publish nothing"
            );
        }
    });
    let step = maintainer.commit(work);
    assert!(
        matches!(step.outcome, Outcome::Discarded(_)),
        "the foreign writer already consumed this source: {step:?}"
    );
    let attempts = disk::take_attempts();
    assert_eq!(attempts.first(), Some(&Attempt::Stale));
    no_locked_reads();
    disk::clear_before_lock();
    drain(&mut maintainer);
    root.check(|| {
        vec![
            source(&alpha, "alpha"),
            source(&root.0.join("sessions/beta"), "beta"),
        ]
    });
}

#[test]
fn s4_snapshot_budget_refuses_before_a_lock_or_a_write() {
    let root = Fixture::new();
    let first = birth(S, 1);
    let dir = root.live("alpha", S, &first);
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    fs::write(dir.join("events.jsonl"), [first, birth(S, 2)].concat()).expect("append fixture");
    let work = maintainer.prepare().expect("work before the cap override");
    let _controls = reset();
    disk::set_snapshot_cap(Some(1));
    let step = maintainer.commit(work);
    assert!(
        matches!(step.outcome, Outcome::Refused(Refused::Snapshot { .. })),
        "complete snapshot cannot fit one byte: {step:?}"
    );
    assert!(
        disk::take_attempts().is_empty(),
        "no lock-taking attempt past snapshot cap"
    );
    assert!(
        disk::take_file_events().is_empty(),
        "no incomplete snapshot publication"
    );
    no_locked_reads();
    disk::set_snapshot_cap(None);
    drain(&mut maintainer);
    root.check(|| vec![source(&dir, "alpha")]);
}

#[test]
fn s4_purge_retries_a_changed_dependency_and_removes_its_foreign_fragment() {
    let root = Fixture::new();
    let aid = S;
    let archive = root.archive(aid, &[birth(S, 1), birth(T, 2)].concat());
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    fs::remove_dir_all(archive).expect("archive bytes removed before the store purge step");
    let _controls = reset();
    disk::before_lock(|root, attempt| {
        if attempt == 1 {
            let view = root.join("stats/view");
            fs::write(
                root.join("replacement"),
                fs::read(&view).expect("readable view"),
            )
            .expect("replacement");
            fs::rename(root.join("replacement"), view).expect("replace dependency");
        } else {
            assert!(
                disk::take_file_events().is_empty(),
                "stale purge writes nothing"
            );
        }
    });
    assert_eq!(
        disk::purged(&root.0, aid, Duration::from_millis(100)).expect("purge retries"),
        disk::Purged::Removed
    );
    assert_eq!(
        disk::take_attempts(),
        vec![Attempt::Stale, Attempt::Applied]
    );
    no_locked_reads();
    for scope in [None, Some(S), Some(T)] {
        let disk::Shown::Ready { view, .. } = disk::shown(&mut Files::at(&root.0), scope) else {
            panic!("empty durable root");
        };
        assert_eq!(
            view,
            crate::stats::fold(Vec::new(), scope),
            "all archive assertions removed"
        );
    }
}

#[test]
fn s4_purge_exhaustion_returns_the_named_non_success_without_writing() {
    let root = Fixture::new();
    let archive = root.archive(S, &birth(S, 1));
    drain(&mut root.maintainer());
    fs::remove_dir_all(archive).expect("archive bytes removed before store purge");
    let before = fs::read(root.0.join("stats/view")).expect("counted state");
    let _controls = reset();
    disk::before_lock(|root, _| {
        assert!(disk::take_file_events().is_empty());
        fs::write(
            root.join("replacement"),
            fs::read(root.join("stats/view")).expect("view"),
        )
        .expect("fresh inode");
        fs::rename(root.join("replacement"), root.join("stats/view"))
            .expect("changed each attempt");
    });
    assert_eq!(
        disk::purged(&root.0, S, Duration::from_millis(100)),
        Err(Refused::Contended)
    );
    assert_eq!(disk::take_attempts(), vec![Attempt::Stale; 3]);
    assert!(disk::take_file_events().is_empty());
    assert_eq!(fs::read(root.0.join("stats/view")).expect("kept"), before);
    no_locked_reads();
}

#[test]
fn s4_maintainer_vanished_source_chore_and_prune_open_nothing_under_lock() {
    let root = Fixture::new();
    let alpha = root.live("alpha", S, &[birth(S, 1), birth(T, 2)].concat());
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    fs::remove_dir_all(alpha).expect("vanished live source");
    let _controls = reset();
    drain(&mut maintainer);
    no_locked_reads();
    assert!(
        disk::take_attempts().contains(&Attempt::Applied),
        "the disappearance exercised a write"
    );
    root.check(Vec::new);
}
