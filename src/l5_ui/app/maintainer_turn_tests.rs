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

use super::{TOOLS_BREATH, TOOLS_FAIR, TOOLS_ROUND, UpkeepStep, notice};
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
    // The clock is simulated end to end: the real time a unit takes (a load
    // stall) never enters the schedule these specs pin.
    step.still = true;
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

/// The next Tools unit is due once the wake has really ended (the S5 step
/// and the unit, counted from the wake's start) plus a rest as long as the
/// unit took, so Tools holds at most about half a core; only a step held
/// still leaves that real time out of the simulated schedule. A stall in the
/// S5 step and one in the unit move `tools_due` by both terms on the real
/// clock, and by nothing on the still one (issue 329).
#[test]
fn a_real_stall_moves_the_tools_schedule_only_on_the_real_clock() {
    let wake = Duration::from_millis(60);
    let unit = Duration::from_millis(100);
    for still in [true, false] {
        let root = proven();
        let mut step = UpkeepStep::new(&root.0);
        step.still = still;
        step.unit_stall = unit;
        let now = Instant::now();
        let turn = step.turn_with(now, |_| {
            std::thread::sleep(wake);
            synthetic(&Outcome::Deferred, Duration::ZERO)
        });
        assert!(
            turn.tools.is_some(),
            "control: the first turn runs a Tools unit (still: {still})"
        );
        let due = step
            .tools_due
            .expect("a unit ran, so the next has a due time");
        if still {
            assert_eq!(
                due,
                now + TOOLS_BREATH,
                "a still step charges the stall nothing: the unit rests its breath"
            );
        } else {
            // The wake's S5 stall and the unit's stall are charged as the
            // wake's length, and the unit's stall once more as its rest.
            assert!(
                due >= now + wake + unit + unit,
                "a real step charges the wake and then rests as long as the unit took: {:?}",
                due.saturating_duration_since(now)
            );
        }
    }
}

// --- #288 mutants slot 1 survivors (maintainer.rs:246, 300, 422) ----------

/// The Tools unit times of `window` of beats every 1 ms of simulated time,
/// far more often than [`UpkeepStep::wait`] asks (a command wakes the thread
/// at any time), S5 answering `outcome` each time it is due.
fn over_waking(outcome: &Outcome, window: Duration) -> Vec<Instant> {
    let root = proven();
    let mut step = UpkeepStep::new(&root.0);
    let start = Instant::now();
    let mut now = start;
    let mut tools = Vec::new();
    while now < start + window {
        let turn = step.beat_with(now, |_| synthetic(outcome, Duration::ZERO));
        if turn.tools.is_some() {
            tools.push(now);
        }
        now += Duration::from_millis(1);
    }
    tools
}

/// maintainer.rs:246: a wake between S5 steps runs a Tools unit only when
/// the unit is owed: never before the last unit's own wait
/// ([`TOOLS_BREATH`] at least), and while S5 has work never sooner than
/// [`TOOLS_FAIR`] after the last.
#[test]
fn an_early_wake_never_runs_a_tools_unit_before_it_is_owed() {
    for (name, outcome, least) in [
        ("idle", Outcome::Idle, TOOLS_BREATH),
        ("busy", Outcome::Deferred, TOOLS_FAIR),
    ] {
        let tools = over_waking(&outcome, TOOLS_FAIR * 4);
        assert!(tools.len() >= 2, "{name}: control, Tools ran: {tools:?}");
        for pair in tools.windows(2) {
            assert!(
                pair[1] - pair[0] >= least,
                "{name}: Tools units {:?} apart, under {least:?}",
                pair[1] - pair[0]
            );
        }
    }
}

/// maintainer.rs:300: once a round has ended, the next starts at the first
/// S5 turn [`TOOLS_ROUND`] after it, and not one turn sooner.
#[test]
fn a_tools_round_starts_again_exactly_one_round_pause_after_the_last() {
    let idle = |_: &mut Maintainer| synthetic(&Outcome::Idle, Duration::ZERO);
    let root = proven();
    let mut step = UpkeepStep::new(&root.0);
    let mut now = Instant::now();
    for _ in 0..10_000 {
        let _ = step.beat_with(now, idle);
        if step.rested.is_some() {
            break;
        }
        now += step.wait(now).max(Duration::from_millis(1));
    }
    let rested = step.rested.expect("control: the first round ended");
    assert!(!step.tools.busy(), "control: no round in progress");
    let early = step.turn_with(
        (rested + TOOLS_ROUND)
            .checked_sub(Duration::from_millis(1))
            .expect("after the start"),
        idle,
    );
    assert!(
        early.tools.is_none() && !step.tools.busy(),
        "no round starts before {TOOLS_ROUND:?}: {:?}",
        early.tools
    );
    let due = step.turn_with(rested + TOOLS_ROUND, idle);
    assert!(
        due.tools.is_some_and(|unit| unit.kind == UnitKind::Load),
        "the round starts at {TOOLS_ROUND:?}: {:?}",
        due.tools
    );
}

/// maintainer.rs:422: a repair is told each time; a refusal is told only
/// when it stopped the upkeep, and that stop only once.
#[test]
fn the_stop_notice_is_told_once_and_only_for_a_stop() {
    let refused = Outcome::Refused(Refused::Io("fixture".into()));
    let mut told = false;
    assert_eq!(
        notice(Some(&refused), false, &mut told),
        None,
        "a refusal that stops nothing is not told"
    );
    assert!(
        notice(Some(&refused), true, &mut told).is_some_and(|line| line.contains("upkeep stopped")),
        "the stop is told"
    );
    assert_eq!(
        notice(Some(&refused), true, &mut told),
        None,
        "the stop is told once"
    );
    let repaired = Outcome::Repaired {
        file: "stats/view".to_owned(),
        reason: "fixture".to_owned(),
    };
    assert!(
        notice(Some(&repaired), true, &mut told)
            .is_some_and(|line| line.contains("rebuilding the store")),
        "a repair is told"
    );
    assert_eq!(
        notice(None, true, &mut false),
        None,
        "no S5 step, no notice"
    );
}
