//! Frozen S2 thread ownership checks. `App::run/end` wiring is also reviewed structurally.
#![allow(clippy::expect_used, reason = "acceptance fixture and asserted spawn")]

use super::{STOP_BOUND, start};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[path = "../stats_store/s2_cases.rs"]
#[allow(
    clippy::duplicate_mod,
    reason = "independent unit and thread specs share the frozen fixture and fold oracle"
)]
mod cases;

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "ae-s2-thread-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).expect("private thread root");
        Self(p)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn s2_parked_thread_stop_wakes_and_proves_completion() {
    let root = Root::new();
    let (tx, _rx) = mpsc::channel();
    let running = start(root.0.clone(), tx).expect("start one thread");
    let ended = running.ended();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !running.parked() {
        assert!(
            Instant::now() < deadline,
            "thread did not reach parked wait"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        running.stop(STOP_BOUND),
        "parked stop exceeded the required bound"
    );
    assert!(ended.is_ended(), "the actual worker owns completion");
    #[allow(
        clippy::disallowed_methods,
        reason = "acceptance inventories its private temporary root to prove no files were created"
    )]
    let entries = std::fs::read_dir(&root.0)
        .expect("empty fleet root")
        .count();
    assert_eq!(entries, 0, "empty fleet writes nothing");
}

#[test]
fn s2_owner_drop_signals_and_joins_the_worker() {
    let root = Root::new();
    let (tx, _rx) = mpsc::channel();
    let running = start(root.0.clone(), tx).expect("start thread");
    let ended = running.ended();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !running.parked() {
        assert!(Instant::now() < deadline, "worker never parked");
        std::thread::sleep(Duration::from_millis(1));
    }
    drop(running);
    assert!(
        ended.is_ended(),
        "owner drop covers early/error returns from app::run"
    );
}

#[test]
fn s2_repair_notice_names_the_file_and_reason() {
    use crate::app::loader::Wake;
    use crate::stats_store::disk::Limits;
    use crate::stats_store::maint::{Maintainer, Outcome};
    let root = Root::new();
    let session = root.0.join("sessions/a");
    std::fs::create_dir_all(&session).expect("fixture session");
    std::fs::write(session.join("meta"), format!("session_id={}\n", cases::S))
        .expect("fixture meta");
    let mut bytes = cases::birth(cases::S, 1);
    std::fs::write(session.join("events.jsonl"), &bytes).expect("fixture journal");
    let mut m = Maintainer::new(
        &root.0,
        Limits {
            chunk: 4 << 20,
            line: 4 << 20,
        },
        Duration::ZERO,
    );
    let mut idle = false;
    for _ in 0..10 {
        if matches!(m.step().outcome, Outcome::Idle) {
            idle = true;
            break;
        }
    }
    assert!(idle);
    drop(m);
    std::fs::write(root.0.join("stats/view"), b"broken-view").expect("damage fixture");
    bytes.extend(cases::birth(cases::S, 2));
    std::fs::write(session.join("events.jsonl"), &bytes).expect("append fixture");
    let (tx, rx) = mpsc::channel();
    let running = start(root.0.clone(), tx).expect("start repair worker");
    let notice = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("repair notice");
    let Wake::Notice(text) = notice else {
        panic!("repair reports through the existing app notice path");
    };
    assert!(text.contains("stats/view"));
    assert!(text.len() > "stats/view".len(), "repair names a reason");
    assert!(running.stop(STOP_BOUND));
}
