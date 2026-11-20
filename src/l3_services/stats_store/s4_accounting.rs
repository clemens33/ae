//! Frozen S4 batch-review regressions: every acquired hold remains observable.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and contract assertions"
)]

use super::disk::{self, Attempt, Payload};
use super::maint::{Maintainer, Outcome, Step};
use super::s4_behavior::{Fixture, S, birth, drain};
use std::cell::RefCell;
use std::fs;
use std::rc::Rc;
use std::time::Duration;

struct Controls;
impl Drop for Controls {
    fn drop(&mut self) {
        disk::clear_before_lock();
    }
}

fn stale_then_busy(run: impl FnOnce() -> Step) {
    let _controls = Controls;
    let held = Rc::new(RefCell::new(None));
    let blocker = Rc::clone(&held);
    let _ = disk::take_attempts();
    let _ = disk::take_holds();
    let _ = disk::take_file_events();
    disk::before_lock(move |root, attempt| match attempt {
        1 => {
            let bytes = fs::read(root.join("stats/view")).expect("valid root view");
            fs::write(root.join("replacement"), bytes).expect("fresh inode");
            fs::rename(root.join("replacement"), root.join("stats/view"))
                .expect("invalidate the first warm");
        }
        2 => {
            assert!(disk::take_file_events().is_empty(), "stale writes nothing");
            *blocker.borrow_mut() = Some(
                Payload::at(root)
                    .lock(Duration::ZERO)
                    .expect("hold the second attempt busy"),
            );
        }
        _ => panic!("a busy lock ends this call"),
    });
    let step = run();
    disk::clear_before_lock();
    drop(held.borrow_mut().take());
    assert_eq!(step.outcome, Outcome::Deferred);
    assert_eq!(disk::take_attempts(), [Attempt::Stale, Attempt::Busy]);
    let holds = disk::take_holds();
    assert_eq!(holds.len(), 1, "one actual transaction hold before Busy");
    let actual: Duration = holds.iter().map(|hold| hold.held).sum();
    assert!(
        actual > Duration::ZERO,
        "the first hold did real validation"
    );
    assert_eq!(step.held, actual, "Busy must preserve every earlier hold");
}

#[test]
fn s4_publish_stale_then_busy_preserves_the_earlier_hold() {
    let root = Fixture::new();
    let first = birth(S, 1);
    let dir = root.live("alpha", S, &first);
    drain(&mut root.maintainer());
    fs::write(dir.join("events.jsonl"), [first, birth(S, 2)].concat()).expect("incremental work");
    let mut maintainer = Maintainer::new(&root.0, disk::LIMITS, Duration::ZERO);
    let work = maintainer.prepare().expect("prepared second birth");
    stale_then_busy(|| maintainer.commit(work));
}

#[test]
fn s4_chore_stale_then_busy_preserves_the_earlier_hold() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    drain(&mut root.maintainer());
    fs::remove_dir_all(dir).expect("vanished source schedules a removal chore");
    let mut maintainer = Maintainer::new(&root.0, disk::LIMITS, Duration::ZERO);
    stale_then_busy(|| maintainer.step());
}
