//! Frozen S4 upkeep policy: passive refusals wait as long as an idle round.

use super::disk::Refused;
use super::maint::{self, Outcome, Step};
use std::time::Duration;

#[test]
fn s4_snapshot_and_io_refusals_keep_the_idle_refresh_floor() {
    for outcome in [
        Outcome::Idle,
        Outcome::Refused(Refused::Io("stable IO failure".to_owned())),
        Outcome::Refused(Refused::Snapshot {
            recorded: 2,
            cap: 1,
        }),
    ] {
        let mut step = Step {
            source: None,
            outcome,
            read: 0,
            active: Duration::from_millis(1),
            held: Duration::ZERO,
        };
        assert_eq!(maint::pause(&step), maint::REFRESH);
        step.active = maint::REFRESH + Duration::from_secs(1);
        assert_eq!(maint::pause(&step), step.active);
    }
}
