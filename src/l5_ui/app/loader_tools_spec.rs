//! #288 A8 exercises the real loader scheduler and actual usage-door receipts.
#![allow(
    clippy::expect_used,
    clippy::disallowed_methods,
    reason = "frozen acceptance assertions and owned scheduler fixture IO"
)]
use super::{Answer, Job, Reader, Request};
use crate::tool_stats::{Ask, Filter, Page, Scope};
use crate::tool_stats_fixture::{self as f, Fixture};
use crate::usage::calls_read;
use std::time::{Duration, Instant};

fn ask() -> Ask {
    Ask {
        page: Page::Tools,
        scope: Scope::All,
        filter: Filter::All,
    }
}

fn reader(root: &Fixture, now: Instant) -> Reader {
    let mut reader = Reader::new(root.0.clone(), None, None);
    reader.fleet_at = Some(now);
    reader.settings = Some((7, Some(now)));
    reader.settings_session = Some("alpha".into());
    reader.ids.insert("alpha".into(), f::S.into());
    reader
}

fn arm(reader: &mut Reader, generation: u64, shown: bool) {
    reader.take(Request::Tools {
        generation,
        shown,
        ask: ask(),
    });
}

fn seed() -> Fixture {
    let root = Fixture::new();
    root.live(
        "alpha",
        &root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a"),
        "",
    );
    root.claude("store-a", f::C, f::CLASSES.as_bytes());
    root
}

fn refresh_visible(reader: &mut Reader, now: Instant) {
    reader.fleet_at = Some(now);
    reader.settings = Some((7, Some(now)));
}

#[test]
fn a8_first_tools_answer_is_no_io_and_s5_first_glance_precedes_it() {
    let root = seed();
    let now = Instant::now();
    let mut reader = reader(&root, now);
    reader.take(Request::Stats {
        generation: 7,
        shown: true,
    });
    arm(&mut reader, 7, true);
    assert!(
        matches!(reader.next(now), Job::Stats),
        "S5 first glance owns its existing slot"
    );
    assert!(matches!(
        reader.stats_read(now),
        Some(Answer::Stats { generation: 7, .. })
    ));
    assert!(matches!(reader.next(now), Job::Tools));
    let _ = calls_read::take_units();
    assert!(
        matches!(
            reader.tools_step(now),
            Some(Answer::Tools { generation: 7, .. })
        ),
        "first answer shows cache or progress"
    );
    assert!(
        calls_read::take_units().is_empty(),
        "the first-answer slot does no discovery or transcript IO"
    );
}

#[test]
fn a8_tools_runs_within_one_second_under_a_saturated_background_sweep() {
    let root = seed();
    let now = Instant::now();
    Fixture::rewrite(
        &root.0.join("sessions/alpha/events.jsonl"),
        f::event("ordinary", "epoch fixture").as_bytes(),
    );
    let mut maintainer = crate::stats_store::maint::Maintainer::new(
        &root.0,
        crate::stats_store::disk::LIMITS,
        Duration::from_millis(100),
    );
    for _ in 0..100 {
        let _ = maintainer.step();
        if root.0.join("stats/epoch").is_file() {
            break;
        }
    }
    assert!(
        root.0.join("stats/epoch").is_file(),
        "real S5 epoch before Tools fairness"
    );
    drop(maintainer);
    let mut upkeep = crate::app::maintainer::UpkeepStep::new(&root.0);
    let journal = root.0.join("sessions/alpha/events.jsonl");
    let mut last_tools = now;
    let mut seen = 0;
    let mut saw_s5_read = false;
    for index in 0..=20 {
        use std::io::Write as _;
        // An actual new journal record makes S5 busy before EACH turn.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&journal)
            .expect("existing journal")
            .write_all(f::event("ordinary", "scheduler fixture").as_bytes())
            .expect("new S5 work");
        let time = now + Duration::from_millis(index * 100);
        let _ = calls_read::take_units();
        let turn = upkeep.turn(time);
        assert_ne!(
            turn.s5.outcome,
            crate::stats_store::maint::Outcome::Idle,
            "saturated S5 precondition"
        );
        saw_s5_read |= turn.s5.read > 0;
        assert_eq!(turn.thread, std::thread::current().id());
        let units = calls_read::take_units();
        if let Some(unit) = turn.tools {
            assert_eq!(units, vec![unit], "one real unit then yield");
            assert!(unit.bytes <= calls_read::STEP && unit.lstats <= calls_read::LSTATS);
            assert!(
                index == 0 || time.duration_since(last_tools) >= Duration::from_secs(1),
                "busy S5 keeps its priority before one second"
            );
            last_tools = time;
            seen += 1;
        } else {
            assert!(units.is_empty());
            assert!(
                time.duration_since(last_tools) < Duration::from_secs(1),
                "busy S5 cannot starve Tools beyond one second"
            );
        }
    }
    assert!(
        seen >= 2,
        "the saturated fixture crosses two fairness deadlines"
    );
    assert!(
        saw_s5_read,
        "positive control: S5 actually reads its new work"
    );
}

#[test]
fn a8_hidden_closed_and_stale_requests_do_not_schedule_or_read_tools() {
    for mode in ["hidden", "closed", "stale"] {
        let root = seed();
        let now = Instant::now();
        let mut reader = reader(&root, now);
        arm(&mut reader, 7, true);
        let _ = reader.tools_step(now);
        match mode {
            "hidden" => arm(&mut reader, 7, false),
            "closed" => reader.take(Request::Settings {
                open: false,
                generation: 7,
            }),
            _ => {
                reader.take(Request::Settings {
                    open: true,
                    generation: 8,
                });
                arm(&mut reader, 7, true);
            }
        }
        let _ = calls_read::take_units();
        for second in [1, 5, 30, 60] {
            let time = now + Duration::from_secs(second);
            assert!(
                reader.tools_step(time).is_none(),
                "{mode}: no hidden answer"
            );
            assert!(
                !matches!(reader.next(time), Job::Tools),
                "{mode}: no hidden work scheduled"
            );
        }
        assert!(calls_read::take_units().is_empty(), "{mode}: zero door IO");
    }
}

#[test]
fn a8_s5_five_second_glance_deadline_remains_five_seconds_with_tools_active() {
    let root = seed();
    let now = Instant::now();
    let mut reader = reader(&root, now);
    reader.take(Request::Stats {
        generation: 7,
        shown: true,
    });
    let _ = reader.stats_read(now);
    arm(&mut reader, 7, true);
    let _ = reader.tools_step(now);
    let almost = (now + Duration::from_secs(5))
        .checked_sub(Duration::from_nanos(1))
        .expect("fixture deadline has one nanosecond to subtract");
    refresh_visible(&mut reader, almost);
    assert!(
        !matches!(reader.next(almost), Job::Stats),
        "no premature S5 refresh"
    );
    let due = now + Duration::from_secs(5);
    refresh_visible(&mut reader, due);
    // An anti-starvation Tools unit may win once, but S5 must remain due
    // immediately afterwards. No elapsed-clock measurement or sleep is used.
    if matches!(reader.next(due), Job::Tools) {
        let _ = reader.tools_step(due);
    }
    assert!(
        matches!(reader.next(due), Job::Stats),
        "S5 remains due at its unchanged five-second deadline"
    );
}

#[test]
fn a8_steady_shown_tools_answer_yields_to_a_waiting_view_without_transcript_io() {
    let root = seed();
    let now = Instant::now();
    let mut reader = reader(&root, now);
    // Cold unread sessions are continuously due. Selection and fleet already
    // received their first glance, so these are background sweep candidates.
    for index in 0..300 {
        let name = format!("back-{index:03}");
        reader
            .dirs
            .insert(name.clone(), root.0.join("sessions").join(&name));
        reader.rows.push(name);
    }
    arm(&mut reader, 7, true);
    assert!(
        matches!(reader.next(now), Job::Tools),
        "first cached answer precedes the background sweep"
    );
    let _ = reader.tools_step(now);
    let due = now + Duration::from_secs(1);
    refresh_visible(&mut reader, due);
    assert!(
        matches!(reader.next(due), Job::Tools),
        "an always-due fleet cannot starve one bounded unit"
    );
    let _ = calls_read::take_units();
    let _ = reader.tools_step(due);
    let units = calls_read::take_units();
    assert!(
        units.is_empty(),
        "steady loader answer reads only the compact view"
    );
    assert!(
        matches!(reader.next(due), Job::View(_)),
        "yield to the waiting session after a Tools unit"
    );
}
