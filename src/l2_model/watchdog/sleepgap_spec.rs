//! Independent #238 recovery timing. Real awake time remains real (I15/S4).

use super::{CycleClock, host_suspended, settle};
use std::time::{Duration, Instant};

#[test]
fn sleepgap_holds_the_gap_and_one_stable_cycle_then_reopens() {
    let start = Instant::now();
    let clock = |wall, mono| CycleClock {
        wall,
        mono: start + Duration::from_secs(mono),
    };
    let readings = [
        clock(1_400, 0),
        clock(8_600, 0), // Dark wake.
        clock(8_660, 60),
        clock(15_860, 60), // Real wake, another sleep re-arms the hold.
        clock(15_920, 120),
        clock(15_980, 180),
        clock(16_040, 240),
    ];
    let mut held = 0;
    let mut trace = Vec::new();
    for pair in readings.windows(2) {
        held = settle(held, host_suspended(pair[0], pair[1], 60));
        trace.push(held);
    }
    assert_eq!(trace, [2, 1, 2, 1, 0, 0]);
}

#[test]
fn sleepgap_uses_the_existing_inclusive_three_period_divergence() {
    let mono = Instant::now();
    let prior = CycleClock { wall: 1_000, mono };
    for (wall, elapsed, gap) in [
        (1_239, 60, false),
        (1_240, 60, true),
        (8_200, 7_200, false), // A busy watchdog is not a host suspension.
        (900, 60, false),      // The existing backwards-clock guard remains.
    ] {
        let now = CycleClock {
            wall,
            mono: mono + Duration::from_secs(elapsed),
        };
        assert_eq!(host_suspended(prior, now, 60).is_some(), gap);
        assert_eq!(settle(0, host_suspended(prior, now, 60)) > 0, gap);
    }
}
