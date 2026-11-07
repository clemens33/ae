//! Frozen S4 stale chore retries: keep pending work and avoid obsolete duplicates.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and contract assertions"
)]

use super::disk::{self, Attempt};
use super::maint::{Maintainer, Outcome};
use super::s4_behavior::{Fixture, S, T, birth, drain, source};
use std::fs;
use std::time::Duration;

struct Controls;
impl Drop for Controls {
    fn drop(&mut self) {
        disk::clear_before_lock();
    }
}

fn stale_every_attempt() -> Controls {
    disk::clear_before_lock();
    let _ = disk::take_attempts();
    let _ = disk::take_file_events();
    disk::before_lock(|root, _| {
        let bytes = fs::read(root.join("stats/view")).expect("current root view");
        fs::write(root.join("replacement"), bytes).expect("fresh root inode");
        fs::rename(root.join("replacement"), root.join("stats/view"))
            .expect("invalidate this attempt's view dependency");
    });
    Controls
}

fn deferred_without_writes(maintainer: &mut Maintainer) {
    assert_eq!(maintainer.step().outcome, Outcome::Deferred);
    assert_eq!(disk::take_attempts(), vec![Attempt::Stale; 3]);
    assert!(disk::take_file_events().is_empty());
    disk::clear_before_lock();
}

#[test]
fn s4_vanished_source_removal_survives_stale_exhaustion_until_the_next_step() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    drain(&mut root.maintainer());
    fs::remove_dir_all(dir).expect("source vanished");
    let mut maintainer = Maintainer::new(&root.0, disk::LIMITS, Duration::ZERO);
    let _controls = stale_every_attempt();
    deferred_without_writes(&mut maintainer);
    assert!(
        matches!(maintainer.step().outcome, Outcome::Removed { .. }),
        "resolved contention must resume the pending removal before idle"
    );
    drain(&mut maintainer);
    root.check(Vec::new);
}

#[test]
fn s4_moved_source_retry_does_not_leave_a_duplicate_old_owner_removal() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    drain(&mut root.maintainer());
    fs::write(dir.join("meta"), format!("session=alpha\nsession_id={T}\n"))
        .expect("same source moved to another group");
    let mut maintainer = Maintainer::new(&root.0, disk::LIMITS, Duration::ZERO);
    let _controls = stale_every_attempt();
    deferred_without_writes(&mut maintainer);
    assert!(matches!(maintainer.step().outcome, Outcome::Removed { .. }));
    assert!(matches!(
        maintainer.step().outcome,
        Outcome::Published { .. }
    ));
    assert_eq!(
        maintainer.step().outcome,
        Outcome::Idle,
        "one successful move leaves no obsolete removal to report again"
    );
    root.check(|| vec![source(&dir, "alpha")]);
}
