//! F3: drive the real restart decision and replace only terminal/exec effects.
#![allow(
    clippy::expect_used,
    reason = "acceptance fixtures and thread observations"
)]

use super::{Root, exec, housed, upgrade_by};
use crate::app::{Outside, Step, maintainer, step};
use std::io;
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct FailedExec {
    old: maintainer::Ended,
    calls: Vec<&'static str>,
    stopped_at_exec: Option<bool>,
    lose_terminal: bool,
}
impl Outside for FailedExec {
    fn suspend(&mut self) {
        self.calls.push("suspend");
    }
    fn exec(&mut self, argv: &[String]) -> io::Error {
        self.calls.push("exec");
        assert!(!argv.is_empty(), "real restart built the exec arguments");
        self.stopped_at_exec = Some(self.old.is_ended());
        io::Error::other("s2 exec failure")
    }
    fn resume(&mut self) -> Result<(), String> {
        self.calls.push("resume");
        if self.lose_terminal {
            Err("s2 terminal failure".to_owned())
        } else {
            Ok(())
        }
    }
    fn clear(&mut self) -> io::Result<()> {
        self.calls.push("clear");
        Ok(())
    }
}

#[test]
fn s2_f3_real_exec_stops_before_attempt_and_restores_owner_on_failure() {
    let root = Root::new("s2-real-exec");
    let (mut app, _reader) = housed(&root);
    app.answer(upgrade_by(Ok(exec())));
    let (tx, _rx) = mpsc::channel();
    let mut upkeep = maintainer::Upkeep::start(root.0.clone(), tx);
    let old = upkeep.running().expect("old upkeep starts");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !old.parked() {
        assert!(Instant::now() < deadline, "old worker must park");
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut outside = FailedExec {
        old: old.ended(),
        calls: Vec::new(),
        stopped_at_exec: None,
        lose_terminal: false,
    };
    let mut sink = Vec::new();
    let result =
        step(&mut app, &mut outside, &mut upkeep, &[], &mut sink).expect("restart decision");
    assert!(
        matches!(result, Step::Stay(true)),
        "failed exec resumes app"
    );
    assert_eq!(outside.calls, ["suspend", "exec", "resume", "clear"]);
    assert_eq!(
        outside.stopped_at_exec,
        Some(true),
        "stop precedes the actual exec boundary"
    );
    assert_eq!(
        app.failed,
        Some(("2026.10.99".to_owned(), "s2 exec failure".to_owned()))
    );
    let new = upkeep
        .running()
        .expect("failed exec restores one owned upkeep")
        .ended();
    assert!(!new.is_ended());
    outside.calls.clear();
    assert!(matches!(
        step(&mut app, &mut outside, &mut upkeep, &[], &mut sink).expect("suppressed retry"),
        Step::Stay(false)
    ));
    assert!(
        outside.calls.is_empty(),
        "failed version is not attempted twice"
    );
    assert!(!new.is_ended(), "no-restart path keeps the restored owner");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !upkeep.running().expect("restored owner").parked() {
        assert!(
            Instant::now() < deadline,
            "restored worker must park before bounded stop assertion"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    upkeep.stop(maintainer::STOP_BOUND);
    assert!(new.is_ended(), "the restored owner is stopped on app end");
}

#[test]
fn s2_f3_terminal_restore_failure_quits_without_restarting_upkeep() {
    let root = Root::new("s2-exec-lost-terminal");
    let (mut app, _reader) = housed(&root);
    app.answer(upgrade_by(Ok(exec())));
    let (tx, _rx) = mpsc::channel();
    let mut upkeep = maintainer::Upkeep::start(root.0.clone(), tx);
    let old = upkeep.running().expect("old upkeep starts");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !old.parked() {
        assert!(Instant::now() < deadline, "old worker must park");
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut outside = FailedExec {
        old: old.ended(),
        calls: Vec::new(),
        stopped_at_exec: None,
        lose_terminal: true,
    };
    let mut sink = Vec::new();
    assert!(matches!(
        step(&mut app, &mut outside, &mut upkeep, &[], &mut sink).expect("quit decision"),
        Step::Quit(crate::entry::EXIT_FAILED)
    ));
    assert_eq!(outside.calls, ["suspend", "exec", "resume"]);
    assert_eq!(outside.stopped_at_exec, Some(true));
    assert!(upkeep.running().is_none(), "quit does not restart a worker");
    upkeep.stop(maintainer::STOP_BOUND);
}
