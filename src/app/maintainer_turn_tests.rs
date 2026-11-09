//! #288 P4 r1 findings P4-2 and P4-3: the upkeep turn runs Tools only in a
//! proven S5 epoch, and Tools units never make S5 step before its own pause
//! has passed. The turns run on a simulated clock: each next turn is at the
//! wait the last one asked for. S5 steps are synthetic where a test names an
//! outcome; the Tools reader and the store are real.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and bounded test loops"
)]

use super::{TOOLS_FAIR, UpkeepStep};
use crate::stats_store::disk::{self, Refused};
use crate::stats_store::maint::{self, Maintainer, Outcome, Step};
use crate::tool_stats_fixture::{self as f, Fixture};
use crate::usage::calls_read::UnitKind;
use std::time::{Duration, Instant};

/// One live session whose journal holds a record, and its Claude transcript.
fn fixture() -> Fixture {
    let root = Fixture::new();
    root.live(
        "alpha",
        &root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a"),
        &f::event("ordinary", "epoch fixture"),
    );
    root.claude("store-a", f::C, f::CLASSES.as_bytes());
    root
}

/// The fixture with its epoch made by a real S5 first build.
fn proven() -> Fixture {
    let root = fixture();
    let mut s5 = Maintainer::new(&root.0, disk::LIMITS, Duration::from_millis(100));
    for _ in 0..100 {
        let _ = s5.step();
        if root.0.join("stats/epoch").is_file() {
            break;
        }
    }
    assert!(
        root.0.join("stats/epoch").is_file(),
        "positive control: a real S5 build proved an epoch"
    );
    root
}

fn synthetic(outcome: &Outcome, active: Duration) -> Step {
    Step {
        source: None,
        outcome: outcome.clone(),
        read: 0,
        active,
        held: Duration::ZERO,
    }
}

/// P4-2: a first turn after a deferred S5 step, with no epoch built, does no
/// Tools work but the epoch read; the real S5 build that follows proves the
/// epoch, and Tools then reads the transcript.
#[test]
fn p4_2_tools_work_waits_for_a_proven_s5_epoch() {
    let root = fixture();
    let mut step = UpkeepStep::new(&root.0);
    let start = Instant::now();
    let mut now = start;
    let mut units = Vec::new();
    while now < start + Duration::from_secs(3) {
        let turn = step.beat_with(now, |_| {
            synthetic(&Outcome::Deferred, Duration::from_millis(1))
        });
        units.extend(turn.tools);
        now += step.wait(now).max(Duration::from_millis(1));
    }
    assert!(
        !root.0.join("stats/epoch").exists(),
        "control: nothing built an epoch"
    );
    assert!(
        !units.is_empty(),
        "control: Tools was asked (the first turn is fair)"
    );
    assert!(
        units
            .iter()
            .all(|unit| unit.kind == UnitKind::Load && unit.bytes == 0),
        "without a proven epoch Tools only reads the epoch: {units:?}"
    );
    let mut read = false;
    for _ in 0..2_000 {
        let turn = step.beat(now);
        if turn
            .tools
            .is_some_and(|unit| unit.kind == UnitKind::Read && unit.bytes > 0)
        {
            read = true;
            break;
        }
        now += step.wait(now).max(Duration::from_millis(1));
    }
    assert!(
        root.0.join("stats/epoch").is_file(),
        "the real S5 build proved an epoch"
    );
    assert!(read, "Tools reads the transcript once the epoch is proven");
}

/// The S5 call times and the Tools unit times of `window` of simulated
/// turns over a proven store, S5 answering `outcome` after `active` each
/// time it is called.
fn drive(outcome: &Outcome, active: Duration, window: Duration) -> (Vec<Instant>, Vec<Instant>) {
    let root = proven();
    let mut step = UpkeepStep::new(&root.0);
    let start = Instant::now();
    let mut now = start;
    let (mut s5, mut tools) = (Vec::new(), Vec::new());
    for _ in 0..100_000 {
        if now >= start + window {
            return (s5, tools);
        }
        let mut called = false;
        let turn = step.beat_with(now, |_| {
            called = true;
            synthetic(outcome, active)
        });
        if called {
            s5.push(now);
        }
        if turn.tools.is_some() {
            tools.push(now);
        }
        now += step.wait(now);
    }
    panic!("the simulated clock did not reach the window's end")
}

/// P4-3: S5 steps again only once its own pause after the last step has
/// passed (`active` + [`maint::pause`]), whatever Tools does meanwhile; and
/// Tools still runs while S5 has work, at most [`TOOLS_FAIR`] apart.
#[test]
fn p4_3_tools_units_never_rerun_s5_before_its_pause() {
    let cases = [
        ("busy", Outcome::Deferred, Duration::ZERO),
        (
            "active",
            Outcome::Published {
                from: 0,
                to: 1,
                r#gen: 1,
            },
            Duration::from_millis(100),
        ),
        (
            "always busy",
            Outcome::Published {
                from: 0,
                to: 1,
                r#gen: 1,
            },
            Duration::ZERO,
        ),
        (
            "IO refused",
            Outcome::Refused(Refused::Io("fixture".into())),
            Duration::ZERO,
        ),
        ("idle", Outcome::Idle, Duration::ZERO),
    ];
    for (name, outcome, active) in cases {
        let due = active + maint::pause(&synthetic(&outcome, active));
        let window = (due * 3).max(Duration::from_secs(3));
        let (s5, tools) = drive(&outcome, active, window);
        assert!(s5.len() >= 2, "{name}: control, S5 stepped again");
        for pair in s5.windows(2) {
            assert_eq!(
                pair[1] - pair[0],
                due,
                "{name}: S5 steps again exactly at its own pause, {due:?}"
            );
        }
        assert!(tools.len() >= 2, "{name}: control, Tools ran");
        for pair in tools.windows(2).take(2) {
            assert!(
                pair[1] - pair[0] <= TOOLS_FAIR,
                "{name}: Tools runs at most {TOOLS_FAIR:?} apart while S5 has work: {:?}",
                pair[1] - pair[0]
            );
        }
    }
}
