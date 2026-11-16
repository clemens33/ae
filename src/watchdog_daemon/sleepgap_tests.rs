//! The wake settle's cycle-level carry (#238), beside the frozen acceptance.

use super::release_refresh;

/// A seat leaving its limit inside the settle requests the one quota refresh
/// on the cycle its latch clears, and no later cycle requests it again. The
/// held request runs on the first free cycle, once.
#[test]
fn a_limit_release_refresh_held_by_the_settle_runs_on_the_first_free_cycle() {
    let mut owed = false;
    // G: the latch clears inside the settle.
    assert!(!release_refresh(&mut owed, true, true), "held");
    // G+1: the stable cycle, still held; nothing requests it again.
    assert!(!release_refresh(&mut owed, false, true), "still held");
    // G+2: the first free cycle runs the owed refresh.
    assert!(
        release_refresh(&mut owed, false, false),
        "owed refresh runs"
    );
    // Once.
    assert!(!release_refresh(&mut owed, false, false), "spent");
}

/// Outside any settle the request runs on its own cycle, as before.
#[test]
fn a_limit_release_refresh_outside_the_settle_runs_at_once() {
    let mut owed = false;
    assert!(release_refresh(&mut owed, true, false));
    assert!(!release_refresh(&mut owed, false, false));
    assert!(!owed);
}
