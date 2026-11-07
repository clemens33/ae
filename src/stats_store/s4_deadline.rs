//! Frozen S4 deadline addendum: an expired stale attempt pays no second warm.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and contract assertions"
)]

use super::disk::{self, Attempt, Counted, Seen, SourceSpec};
use super::s4_behavior::{Fixture, S, birth, drain};
use std::cell::Cell;
use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::rc::Rc;
use std::time::Duration;

struct Controls;
impl Drop for Controls {
    fn drop(&mut self) {
        disk::clear_before_lock();
    }
}

#[test]
fn s4_expired_stale_counted_call_is_busy_before_a_second_warm() {
    let root = Fixture::new();
    let first = birth(S, 1);
    let dir = root.live("alpha", S, &first);
    drain(&mut root.maintainer());
    let before = fs::read(root.0.join("stats/view")).expect("caught-up root view");
    let second = birth(S, 2);
    fs::write(dir.join("events.jsonl"), [&first[..], &second[..]].concat())
        .expect("one uncounted birth in the same journal");
    let meta = fs::metadata(dir.join("events.jsonl")).expect("journal identity");
    let seen = Seen {
        dev: meta.dev(),
        ino: meta.ino(),
        len: meta.len(),
    };
    let spec = SourceSpec::Live {
        id: "live/alpha".to_owned(),
        label: "alpha".to_owned(),
        meta: fs::read(dir.join("meta")).expect("source meta"),
    };
    let _controls = Controls;
    disk::clear_before_lock();
    let _ = disk::take_payload_reads();
    let _ = disk::take_journal_reads();
    let _ = disk::take_file_events();
    let _ = disk::take_attempts();
    let _ = disk::take_holds();
    let calls = Rc::new(Cell::new(0_u32));
    let observed = Rc::clone(&calls);
    disk::before_lock(move |root, attempt| {
        observed.set(observed.get() + 1);
        if attempt == 1 {
            let warm = disk::take_payload_reads();
            assert!(warm.opened > 0, "first warm read real payload dependencies");
            let bytes = fs::read(root.join("stats/view")).expect("unchanged view bytes");
            fs::write(root.join("replacement"), bytes).expect("equivalent fresh inode");
            fs::rename(root.join("replacement"), root.join("stats/view"))
                .expect("invalidate the first warm without changing the view");
        }
    });

    // A zero poll budget still permits the first try on an uncontended lock.
    // Once that real hold finishes stale, its deadline has passed without a sleep.
    let result = disk::counted(
        &root.0,
        &spec,
        first.len() as u64,
        &second[..second.len() - 1],
        seen,
        Duration::ZERO,
    );
    disk::clear_before_lock();
    assert_eq!(
        result,
        Counted::Busy,
        "expired stale work stays in the journal"
    );
    let attempts = disk::take_attempts();
    assert_eq!(attempts.first(), Some(&Attempt::Stale));
    assert_eq!(disk::take_holds().len(), 1, "only the first real lock hold");
    assert_eq!(calls.get(), 1, "no second warm reaches the lock hook");
    let reads = disk::take_payload_reads();
    assert_eq!(reads.opened, 0, "no payload re-open after the first warm");
    assert_eq!(reads.listed, 0, "no directory re-list after the first warm");
    assert_eq!(disk::take_journal_reads().total, 0, "no history debt paid");
    assert!(disk::take_file_events().is_empty(), "no operation applied");
    assert_eq!(
        fs::read(root.0.join("stats/view")).expect("durable view kept"),
        before,
        "neither the stale attempt nor the deadline publishes the new birth"
    );
}
