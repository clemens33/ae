//! P4 exercises the actual spawned upkeep thread; no scheduling mock.
#![allow(
    clippy::expect_used,
    reason = "frozen background acceptance assertions"
)]

use super::{Probe, spawn};
use crate::tool_stats_fixture::{self as f, Fixture};
use crate::usage::calls_read;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[test]
fn p4_actual_upkeep_thread_reads_transcripts_and_loader_thread_does_not() {
    let root = Fixture::new();
    root.live(
        "alpha",
        &root.meta("alpha", f::S, "claude", f::C, Some("work"), "store-a"),
        &f::event("ordinary", "epoch fixture"),
    );
    root.claude(
        "store-a",
        f::C,
        f::call("one", "one", "Read", "model-a", "{}").as_bytes(),
    );
    let (notices, _received) = mpsc::channel();
    let probe = Probe::default();
    let _ = calls_read::take_units();
    let running = spawn(root.0.clone(), notices, probe.clone(), None).expect("real upkeep thread");
    let until = Instant::now() + Duration::from_secs(10);
    let mut turns = Vec::new();
    while Instant::now() < until {
        turns.extend(probe.take_turns());
        if turns.iter().any(|turn| {
            turn.tools
                .is_some_and(|unit| unit.kind == calls_read::UnitKind::Read && unit.bytes > 0)
        }) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        running.stop(Duration::from_secs(2)),
        "owned thread stops before fixture removal"
    );
    turns.extend(probe.take_turns());
    assert!(
        turns.iter().any(|turn| turn
            .tools
            .is_some_and(|unit| unit.kind == calls_read::UnitKind::Read && unit.bytes > 0)),
        "positive control: the actual background reader did work"
    );
    let thread = turns[0].thread;
    assert_ne!(thread, std::thread::current().id());
    assert!(
        turns.iter().all(|turn| turn.thread == thread),
        "all observed upkeep turns use one thread"
    );
    for unit in turns.iter().filter_map(|turn| turn.tools) {
        assert!(unit.bytes <= calls_read::STEP && unit.lstats <= calls_read::LSTATS);
    }
    assert!(
        calls_read::take_units().is_empty(),
        "the caller/loader thread performs no transcript unit"
    );
}
