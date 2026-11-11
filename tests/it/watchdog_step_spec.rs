//! Frozen P3b acceptance, authored before implementation by p3b-nav.
//! Real private tmux and real daemon. Quota-off trace lines count iterations;
//! publication checks independently prove done follows the cycle's effects.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures inspect their private filesystem and tmux processes"
)]

use std::fs;
use std::path::Path;
use std::process::{Child, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use super::cli::OwnedChild;
use super::seat_relaunch::Rig;
use super::watchdog_step::Stepper;

const BOUND: Duration = Duration::from_secs(10);

fn words(values: &[&str]) -> Vec<String> {
    values.iter().map(|word| (*word).to_owned()).collect()
}

fn fixture(tag: &str) -> Rig {
    let rig = Rig::new(tag);
    let path = rig.scratch.join("config");
    let config = fs::read_to_string(&path).expect("fixture config");
    fs::write(path, format!("{config}quota = off\n"))
        .expect("disable all vendor quota reads, retain cadence receipts");
    rig
}

fn receipts(rig: &Rig) -> usize {
    let text = fs::read_to_string(rig.scratch.join("step-trace")).unwrap_or_default();
    assert!(text.lines().all(|line| line == "skipped"), "{text}");
    text.lines().count()
}

fn signal(rig: &Rig, channel: &str) {
    assert!(rig.tmux(&["wait-for", "-S", channel]).0, "signal {channel}");
}

fn exited(child: &mut Child, context: &str) -> ExitStatus {
    let deadline = Instant::now() + BOUND;
    loop {
        if let Some(status) = child.try_wait().expect("read child exit") {
            return status;
        }
        assert!(Instant::now() < deadline, "{context}: child did not exit");
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct Daemon<'a> {
    rig: &'a Rig,
    channel: String,
    child: OwnedChild,
    stopped: bool,
}

impl<'a> Daemon<'a> {
    fn start(rig: &'a Rig, channel: &str) -> Self {
        fs::write(rig.scratch.join("step-trace"), "").expect("fresh receipt file");
        let child = super::cli::ae()
            .arg(ae::cli::WATCHDOG_RUN)
            .arg(&rig.dir)
            .args([
                "--interval",
                "1",
                "--step-channel",
                channel,
                "--quota-every-secs",
                "1",
                "--idle-nudge-secs",
                "0",
                "--tg-supervise-secs",
                "0",
            ])
            .env("HOME", &rig.scratch)
            .env("AE_HOME", &rig.scratch)
            .env("CONFIG_FILE", rig.scratch.join("config"))
            .env("AE_TEST_QUOTA_TRACE", rig.scratch.join("step-trace"))
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env_remove("AE_SENDER_OVERRIDE")
            .stdout(Stdio::null())
            .stderr(fs::File::create(rig.scratch.join("step-err")).expect("daemon log"))
            .spawn()
            .expect("real stepped watchdog");
        Self {
            rig,
            channel: channel.to_owned(),
            child,
            stopped: false,
        }
    }

    fn stop(&mut self) -> ExitStatus {
        let meta = self.rig.dir.join("meta");
        let saved = fs::read(&meta).expect("save meta before stopping");
        fs::remove_file(&meta).expect("request ordinary daemon stop");
        signal(self.rig, &self.channel);
        let status = exited(&mut self.child, "meta-delete stop");
        self.stopped = true;
        fs::write(meta, saved).expect("restore fixture meta");
        status
    }
}

impl Drop for Daemon<'_> {
    fn drop(&mut self) {
        // On assertion failure too: release the daemon's waiter before the
        // owned child can kill the daemon and before Rig kills the server.
        if !self.stopped {
            let _ = fs::remove_file(self.rig.dir.join("meta"));
            let _ = self.rig.tmux(&["wait-for", "-S", &self.channel]);
            let deadline = Instant::now() + BOUND;
            while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

/// Independent, bounded observer. Its timeout cleanup RELEASES and reaps the
/// wait-for client. `cli::tmux_signalled` kills timed-out waiters and is unsuitable.
struct DoneWait<'a> {
    rig: &'a Rig,
    channel: String,
    child: OwnedChild,
    reaped: bool,
}

impl<'a> DoneWait<'a> {
    fn new(rig: &'a Rig, channel: &str) -> Self {
        let child = super::cli::tmux_waiter(&rig.sock, channel).expect("private channel observer");
        Self {
            rig,
            channel: channel.to_owned(),
            child,
            reaped: false,
        }
    }

    fn signalled(&mut self, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        loop {
            if let Some(status) = self.child.try_wait().expect("channel observer exit") {
                self.reaped = true;
                assert!(status.success(), "a signal releases normally: {status}");
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for DoneWait<'_> {
    fn drop(&mut self) {
        if !self.reaped {
            signal(self.rig, &self.channel);
            let status = exited(&mut self.child, "released channel observer");
            assert!(status.success(), "released, never killed: {status}");
        }
    }
}

#[test]
fn hidden_step_channel_accepts_safe_names_among_numeric_knobs() {
    for channel in ["p3b", "P3b_7-safe"] {
        let request = ae::cli::Request::parse(&words(&[
            ae::cli::WATCHDOG_RUN,
            "/unused",
            "--interval",
            "7",
            "--step-channel",
            channel,
            "--quota-every-secs",
            "1",
        ]));
        assert!(
            matches!(request, ae::cli::Request::WatchdogRun { .. }),
            "the hidden daemon accepts the test knob: {request:?}"
        );
    }
}

#[test]
fn hidden_step_channel_refuses_empty_unsafe_and_missing_names() {
    for channel in ["", "has space", "semi;colon", "line\nbreak", "slash/name"] {
        let request = ae::cli::Request::parse(&words(&[
            ae::cli::WATCHDOG_RUN,
            "/unused",
            "--step-channel",
            channel,
        ]));
        assert!(matches!(request, ae::cli::Request::UsageError(_)));
    }
    assert!(matches!(
        ae::cli::Request::parse(&words(&[
            ae::cli::WATCHDOG_RUN,
            "/unused",
            "--step-channel"
        ])),
        ae::cli::Request::UsageError(_)
    ));
}

#[test]
fn public_lifecycle_verbs_refuse_step_channel_before_any_mutation() {
    let rig = fixture("steppublic");
    let meta = fs::read(rig.dir.join("meta")).expect("meta snapshot");
    let journal = rig.events();
    for verb in ["watchdog", "loop"] {
        let (code, _, error) = rig.run_top(
            &rig.main_pane,
            &[
                verb,
                "start",
                &rig.session,
                "--",
                "--step-channel",
                "forbidden",
            ],
        );
        assert_eq!(code, Some(2), "{verb}: {error}");
        assert!(error.contains("Usage:"), "a named usage refusal: {error}");
        assert_eq!(fs::read(rig.dir.join("meta")).expect("meta"), meta);
        assert_eq!(rig.events(), journal, "no lifecycle audit was booked");
        assert!(!rig.dir.join(".watchdog.pid").exists());
    }
}

#[test]
fn ready_and_steps_count_exactly_n_completed_cycles_with_effects_before_done() {
    let rig = fixture("stepcount");
    let mut daemon = Daemon::start(&rig, "p3b-count-1");
    let mut stepper = Stepper::new(&rig.sock, "p3b-count-1");
    stepper.ready();
    assert_eq!(
        receipts(&rig),
        1,
        "ready acknowledges the eager startup cycle"
    );
    assert_eq!(
        stepper.cycles(),
        0,
        "ready is the origin, not a requested step"
    );
    stepper.steps(0);
    assert_eq!(receipts(&rig), 1, "zero requested cycles means no work");
    for expected in 1..=3 {
        assert!(
            rig.tmux(&[
                "set-option",
                "-w",
                "-t",
                &rig.main_pane,
                ae::theme::WINDOW_AGENTS_OPTION,
                "p3b-unpublished",
            ])
            .0
        );
        stepper.step();
        assert_eq!(receipts(&rig), expected + 1, "one pass per requested cycle");
        let (ok, published) = rig.tmux(&[
            "show-options",
            "-wqv",
            "-t",
            &rig.main_pane,
            ae::theme::WINDOW_AGENTS_OPTION,
        ]);
        assert!(
            ok && published.contains("lead"),
            "done follows publication: {published}"
        );
        assert!(!published.contains("p3b-unpublished"));
        assert_eq!(stepper.cycles(), expected as u64);
    }
    stepper.steps(3);
    assert_eq!(receipts(&rig), 7);
    assert_eq!(stepper.cycles(), 6);
    assert!(daemon.stop().success());
}

/// A deliberate real interval crossing proves the daemon cannot free-run.
#[test]
fn elapsed_time_attach_and_an_unrelated_channel_do_not_start_a_stepped_cycle() {
    let rig = fixture("stepquiet");
    let mut daemon = Daemon::start(&rig, "p3b-quiet-1");
    let mut stepper = Stepper::new(&rig.sock, "p3b-quiet-1");
    stepper.ready();
    let _attached = super::cli::tmux_attached_client(&rig.sock, &rig.session)
        .expect("real attached control client");
    let deadline = Instant::now() + BOUND;
    while !rig
        .tmux(&["list-clients", "-F", "#{client_session}"])
        .1
        .lines()
        .any(|session| session == rig.session)
    {
        assert!(
            Instant::now() < deadline,
            "the control client really attached"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    signal(&rig, "p3b-unrelated");
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(
        receipts(&rig),
        1,
        "neither real interval, attachment nor other channel is a step"
    );
    let mut extra = DoneWait::new(&rig, "p3b-quiet-1-done");
    assert!(
        !extra.signalled(Duration::from_millis(100)),
        "no duplicate done"
    );
    drop(extra);
    stepper.step();
    assert_eq!(receipts(&rig), 2, "the actual step is the positive control");
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(
        receipts(&rig),
        2,
        "an expired step deadline must not stay latched after the first step"
    );
    assert!(daemon.stop().success());
}

#[test]
fn step_until_checks_before_stepping_and_returns_only_the_cycles_it_used() {
    let rig = fixture("stepuntil");
    let mut daemon = Daemon::start(&rig, "p3b-until-1");
    let mut stepper = Stepper::new(&rig.sock, "p3b-until-1");
    stepper.ready();
    assert_eq!(stepper.step_until(|| true), Some(0));
    assert_eq!(receipts(&rig), 1);
    assert_eq!(stepper.step_until(|| receipts(&rig) >= 4), Some(3));
    assert_eq!(receipts(&rig), 4);
    assert_eq!(stepper.cycles(), 3);
    assert!(daemon.stop().success());
}

#[test]
fn meta_delete_stops_without_a_false_cycle_done_acknowledgement() {
    let rig = fixture("stepstop");
    let mut daemon = Daemon::start(&rig, "p3b-stop-1");
    let mut stepper = Stepper::new(&rig.sock, "p3b-stop-1");
    stepper.ready();
    let mut terminal_done = DoneWait::new(&rig, "p3b-stop-1-done");
    assert!(daemon.stop().success());
    assert_eq!(receipts(&rig), 1, "stopping is not another completed cycle");
    assert!(!terminal_done.signalled(Duration::from_millis(200)));
    assert!(!rig.dir.join(".watchdog.pid").exists());
}

fn server_fds(rig: &Rig) -> usize {
    let (ok, pid) = rig.tmux(&["display-message", "-p", "#{pid}"]);
    assert!(ok);
    let pid = pid.trim().parse::<u32>().expect("real tmux server pid");
    super::cli::descriptor_count(pid)
}

#[test]
fn server_fd_detector_observes_and_releases_a_real_wait_client() {
    let rig = fixture("stepfddetector");
    let before = server_fds(&rig);
    assert!(before > 0, "the fd detector must see the actual server");
    // Calibration: this real, pending waiter must change the detector.
    let mut calibration = DoneWait::new(&rig, "p3b-fd-calibration");
    let deadline = Instant::now() + BOUND;
    while server_fds(&rig) <= before {
        assert!(
            Instant::now() < deadline,
            "detector did not see the live wait client"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!calibration.signalled(Duration::from_millis(20)));
    drop(calibration); // Releases a timed-out observer, never kills it.
    assert!(server_fds(&rig) <= before + 1);
}

/// Eight independent real daemon starts and 24 cycles exercise channel reuse
/// cleanup. Any runtime above ten seconds is process and tmux work, not a sleep.
#[test]
fn fresh_channel_restarts_leave_no_server_fd_growth() {
    let rig = fixture("stepfds");
    let before = server_fds(&rig);
    assert!(before > 0);
    for start in 0..8 {
        let channel = format!("p3b-fd-{start}");
        let mut daemon = Daemon::start(&rig, &channel);
        let mut stepper = Stepper::new(&rig.sock, &channel);
        stepper.ready();
        stepper.steps(2);
        assert_eq!(receipts(&rig), 3, "fresh channel means no prior-start work");
        assert!(daemon.stop().success());
        drop(stepper);
        assert!(
            server_fds(&rig) <= before + 1,
            "start {start}: a normally released wait client must not accumulate server fds"
        );
    }
}

#[test]
fn stepper_ready_timeout_releases_and_reaps_its_own_wait_client() {
    let rig = fixture("steptimeout");
    let before = server_fds(&rig);
    assert!(before > 0);
    for attempt in 0..8 {
        let channel = format!("p3b-timeout-{attempt}");
        let mut stepper = Stepper::with_timeout(&rig.sock, &channel, Duration::from_millis(100));
        let started = Instant::now();
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stepper.ready()));
        let payload = failure.expect_err("no daemon means no completion acknowledgement");
        let text = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .expect("timeout diagnostic is text");
        assert!(text.contains(&channel), "timeout names its channel: {text}");
        assert!(text.contains('0'), "timeout names its cycle count: {text}");
        assert!(
            started.elapsed() < BOUND,
            "short bound reaches its red promptly"
        );
        drop(stepper);
        assert!(
            server_fds(&rig) <= before + 1,
            "attempt {attempt}: a timed-out Stepper must release its actual done waiter"
        );
    }
}

#[test]
fn server_rebind_stops_the_step_transport_without_a_false_done() {
    let rig = fixture("steprebind");
    let other = fixture("stepother");
    let mut daemon = Daemon::start(&rig, "p3b-rebind-1");
    let mut stepper = Stepper::new(&rig.sock, "p3b-rebind-1");
    stepper.ready();
    let meta = rig.dir.join("meta");
    let old = fs::read_to_string(&meta).expect("fixture meta");
    let rebound = old.replace(
        &format!("tmux_server={}\n", rig.sock.display()),
        &format!("tmux_server={}\n", other.sock.display()),
    );
    assert_ne!(rebound, old, "the server selector actually changed");
    fs::write(&meta, rebound).expect("rebind fixture server");
    let mut done = DoneWait::new(&rig, "p3b-rebind-1-done");
    signal(&rig, "p3b-rebind-1");
    let status = exited(&mut daemon.child, "step-mode rebind refusal");
    daemon.stopped = true;
    assert_eq!(status.code(), Some(1));
    assert!(
        fs::read_to_string(rig.scratch.join("step-err"))
            .expect("daemon log")
            .contains("rebind")
    );
    assert!(!done.signalled(Duration::from_millis(200)));
    assert_eq!(receipts(&rig), 1);
    fs::write(meta, old).expect("restore fixture server");
}

// Retained real-interval anchors (the module migration must keep their complete
// bodies and required helpers byte-identical or supply a separately reviewed
// real-interval equivalent before changing them):
// start: watchdog_glue::a_starting_daemon_replaces_the_off_segment_a_stop_left_behind
// restart and stop-on-meta-delete: autotrust_spec::measured_yes_and_no_use_only_the_arrow_then_enter_once_across_watchdog_restart
// adopter: watchdog_glue::a_running_watchdog_fills_a_watchdogless_peers_fleet_strip_and_writes_nothing_else

// This import documents that all paths used by the helper contract are Path;
// the function also lets compilation validate the agreed constructor type.
const _: fn(&Path, &str) -> Stepper = Stepper::new;
