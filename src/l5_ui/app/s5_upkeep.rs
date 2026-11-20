//! S5 reset delivery survives pause and the maintainer's refused state.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and bounded acceptance polling"
)]
use super::{Handed, Upkeep};
use crate::app::loader::Wake;
use crate::stats_store::disk::{self, Files, Glance, Reset};
use crate::stats_store::maint::Outcome;
use crate::stats_store::s5_fixture::{Fixture, S, birth, oracle};
use std::fs;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

fn reset_wake(rx: &Receiver<Wake>, generation: u64) -> Result<Reset, disk::Refused> {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if let Ok(Wake::StatsReset {
            generation: got,
            outcome,
        }) = rx.recv_timeout(Duration::from_millis(20))
        {
            assert_eq!(got, generation);
            return outcome;
        }
    }
    panic!("reset request must produce one named outcome");
}
fn no_duplicate(rx: &Receiver<Wake>) {
    let end = Instant::now() + Duration::from_millis(120);
    while Instant::now() < end {
        assert!(
            !matches!(
                rx.recv_timeout(Duration::from_millis(10)),
                Ok(Wake::StatsReset { .. })
            ),
            "a reset must not be replayed after its outcome"
        );
    }
}

#[test]
fn s5_paused_upkeep_retains_reset_and_delivers_it_once_after_resume() {
    let root = Fixture::new();
    root.live("alpha", S, &birth(S, 1));
    root.build();
    let (tx, rx) = mpsc::channel();
    let (mut upkeep, hold) = Upkeep::start_holding(root.0.clone(), tx);
    assert!(
        hold.entered(Duration::from_secs(3)),
        "worker reached the controlled step boundary"
    );
    upkeep.pause(Duration::ZERO);
    assert_eq!(upkeep.reset(7), Handed::Held);
    assert_eq!(upkeep.owed_reset(), Some(7));
    assert!(
        rx.try_iter()
            .all(|wake| !matches!(wake, Wake::StatsReset { .. })),
        "paused owner cannot fabricate success"
    );
    drop(hold);
    upkeep.resume();
    let end = Instant::now() + Duration::from_secs(5);
    while upkeep.owed_reset().is_some() && Instant::now() < end {
        upkeep.tend();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        upkeep.owed_reset(),
        None,
        "resumed worker receives the held request"
    );
    assert!(matches!(reset_wake(&rx, 7), Ok(Reset::Rebuilt { .. })));
    no_duplicate(&rx);
    upkeep.stop(Duration::from_secs(1));
}

#[test]
fn s5_reset_queued_before_stop_executes_once_before_the_thread_ends() {
    let root = Fixture::new();
    root.live("alpha", S, &birth(S, 1));
    root.build();
    let (tx, rx) = mpsc::channel();
    let (mut upkeep, hold) = Upkeep::start_holding(root.0.clone(), tx);
    assert!(hold.entered(Duration::from_secs(3)));
    assert_eq!(upkeep.reset(8), Handed::Sent);
    upkeep.pause(Duration::ZERO);
    drop(hold);
    assert!(matches!(reset_wake(&rx, 8), Ok(Reset::Rebuilt { .. })));
    no_duplicate(&rx);
    upkeep.stop(Duration::from_secs(1));
}

#[test]
fn s5_busy_upkeep_reset_returns_named_busy_without_replacing_the_payload() {
    let root = Fixture::new();
    root.live("alpha", S, &birth(S, 1));
    root.build();
    let before = fs::read(root.0.join("stats/view")).expect("built root");
    let held = disk::Payload::at(&root.0)
        .lock(Duration::ZERO)
        .expect("competing writer");
    let (tx, rx) = mpsc::channel();
    let mut upkeep = Upkeep::start(root.0.clone(), tx);
    assert_eq!(upkeep.reset(9), Handed::Sent);
    assert_eq!(reset_wake(&rx, 9), Ok(Reset::Busy));
    assert_eq!(
        fs::read(root.0.join("stats/view")).expect("unchanged root"),
        before
    );
    drop(held);
    no_duplicate(&rx);
    upkeep.stop(Duration::from_secs(1));
}

#[test]
fn s5_repeated_damage_stopped_maintainer_still_serves_explicit_reset() {
    let root = Fixture::new();
    let mut journal = birth(S, 1);
    let dir = root.live("alpha", S, &journal);
    fs::remove_dir(root.0.join("archive")).expect("replace archive namespace");
    fs::write(root.0.join("archive"), b"not a directory")
        .expect("incomplete discovery keeps repair episode open");
    let mut maintainer = root.maintainer();
    crate::stats_store::s5_fixture::drain(&mut maintainer);
    fs::write(root.0.join("stats/view"), b"first damaged root\n").expect("first damage");
    journal.extend(birth(S, 2));
    fs::write(dir.join("events.jsonl"), &journal).expect("trigger real source work");
    assert!(
        (0..32).any(|_| matches!(maintainer.step().outcome, Outcome::Repaired { .. })),
        "real automatic repair occurs once"
    );
    crate::stats_store::s5_fixture::drain(&mut maintainer);
    fs::write(root.0.join("stats/view"), b"second damaged root\n").expect("damage before catch-up");
    journal.extend(birth(S, 3));
    fs::write(dir.join("events.jsonl"), &journal).expect("trigger repeated damage");
    for _ in 0..32 {
        let _ = maintainer.step();
        if maintainer.stopped() {
            break;
        }
    }
    assert!(
        maintainer.stopped(),
        "real repeated damage must stop the maintainer before handover"
    );
    fs::remove_file(root.0.join("archive"))
        .expect("heal discovery without reviving stopped object");
    fs::create_dir(root.0.join("archive")).expect("healthy archive namespace");
    let expected = oracle(&dir, "alpha", Some(S));
    let (tx, rx) = mpsc::channel();
    let (mut upkeep, hold) = Upkeep::start_with_holding(root.0.clone(), tx, maintainer);
    assert!(hold.entered(Duration::from_secs(3)));
    assert_eq!(upkeep.reset(10), Handed::Sent);
    drop(hold);
    assert!(matches!(reset_wake(&rx, 10), Ok(Reset::Rebuilt { .. })));
    let end = Instant::now() + Duration::from_secs(5);
    let mut rebuilt = false;
    while Instant::now() < end {
        if let Glance::Ready { view, all, .. } = disk::glance(&mut Files::at(&root.0), Some(S))
            && all.pending == 0
            && view == expected
        {
            rebuilt = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        rebuilt,
        "reset installs a fresh maintainer, not the old stopped instance"
    );
    no_duplicate(&rx);
    upkeep.stop(Duration::from_secs(1));
}
