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

#[test]
fn s2_f10_damaged_intent_takeover_reports_intent_and_removes_temps() {
    use crate::app::loader::Wake;
    use crate::stats_store::disk::{self, Dir, Files, Limits};
    use crate::stats_store::maint::{Maintainer, Outcome};
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the fixed 64 KiB intent cap fits usize on every supported target"
    )]
    for hostile in [
        b"bad intent".to_vec(),
        vec![b'x'; disk::INTENT_CAP as usize + 1],
    ] {
        let root = Root::new();
        let session = root.0.join("sessions/a");
        std::fs::create_dir_all(&session).expect("fixture session");
        std::fs::write(session.join("meta"), format!("session_id={}\n", cases::S))
            .expect("fixture meta");
        std::fs::write(session.join("events.jsonl"), cases::birth(cases::S, 1))
            .expect("fixture journal");
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
        std::fs::write(root.0.join("stats/intent"), hostile).expect("damage intent");
        std::fs::write(root.0.join("stats/view.tmp"), b"root stage").expect("root temp");
        std::fs::write(
            root.0.join("stats/session/orphan.view.tmp"),
            b"session stage",
        )
        .expect("session temp");
        let (tx, rx) = mpsc::channel();
        let running = start(root.0.clone(), tx).expect("start damaged-intent worker");
        let Wake::Notice(notice) = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("repair notice")
        else {
            panic!("damaged intent reports through Notice");
        };
        assert!(
            notice.contains("stats/intent"),
            "takeover must name the intent: {notice}"
        );
        assert!(running.stop(STOP_BOUND));
        let mut files = Files::at(&root.0);
        assert_eq!(
            files.read("stats/intent", disk::INTENT_CAP),
            disk::Read::Absent
        );
        assert_eq!(
            files.read("stats/view.tmp", disk::ROOT_VIEW_CAP),
            disk::Read::Absent
        );
        assert_eq!(
            files.read("stats/session/orphan.view.tmp", disk::SESSION_VIEW_CAP),
            disk::Read::Absent
        );
    }
}

#[test]
fn s2_f10_valid_intent_takeover_notice_names_damaged_primary() {
    use crate::app::loader::Wake;
    use crate::stats_store::disk::{self, Dir, Files};
    let root = Root::new();
    let mut rig = cases::Rig::new(
        vec![cases::Feed::live(
            "a",
            Some(cases::S),
            cases::birth(cases::S, 1),
        )],
        4096,
        4096,
    );
    rig.catchup(0);
    rig.feeds[0].bytes.extend(cases::birth(cases::S, 2));
    rig.sync(0);
    let work = rig.ready(0);
    let plan = cases::published(
        disk::publish(&mut rig.dir, &mut rig.journals, &work).expect("valid intent plan"),
    );
    let primary = format!("stats/session/{}.evidence", cases::S);
    let cut = plan
        .ops
        .iter()
        .position(|op| matches!(op, disk::Op::Rename { to, .. } if to == &primary))
        .expect("primary rename")
        + 1;
    plan.apply(&mut rig.dir, cut)
        .expect("crash after primary publication");
    rig.dir.plant(&primary, b"damaged primary");
    for name in rig.dir.names() {
        let path = root.0.join(&name);
        std::fs::create_dir_all(path.parent().expect("fixture parent"))
            .expect("fixture payload directories");
        std::fs::write(path, rig.dir.file(&name).expect("fixture bytes"))
            .expect("install interrupted payload");
    }
    let (tx, rx) = mpsc::channel();
    let running = start(root.0.clone(), tx).expect("worker starts on interrupted payload");
    let Wake::Notice(text) = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("primary damage notice")
    else {
        panic!("repair Notice");
    };
    assert!(
        text.contains(&primary),
        "notice must name proven damage: {text}"
    );
    assert!(
        text.contains("rebuilding"),
        "damage takeover repairs once: {text}"
    );
    assert!(running.stop(STOP_BOUND));
    let mut files = Files::at(&root.0);
    assert_eq!(
        files.read("stats/intent", disk::INTENT_CAP),
        disk::Read::Absent
    );
    assert!(
        disk::recovery(&mut files)
            .expect("takeover complete")
            .ops
            .is_empty()
    );
}
