//! Clock fixtures for the real hold and settle loops. The wire supplies timed
//! observations and records requests. The product owns every decision.

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use crate::dictate::{
    Ack, DispatchGate, HoldClock, HoldOutcome, HoldPlan, HoldWire, Lapse, Reading, Refusal, Settle,
    hold, settle,
};

const IDLE: &str = include_str!("claude-voice-startup-effort-2.1.332-120x40.txt");
const NOTICE_IDLE: &str = include_str!("claude-voice-startup-notices-2.1.332-120x41.txt");
const BUSY: &str = include_str!("claude-busy-167x40.txt");
const WARM: &str = include_str!("claude-voice-warming-2.1.332-120x40.txt");
const HOLD: &str = include_str!("claude-voice-holding-2.1.332-120x41.esc");
const PLAIN_HOLD: &str = include_str!("claude-voice-holding-2.1.332-120x41.txt");
const INTERIM: &str = include_str!("claude-voice-interim-2.1.332-120x41.esc");
const TAP: &str = include_str!("claude-voice-tap-2.1.332-120x50.txt");
const TRANSCRIBING: &str = include_str!("claude-voice-transcribing-2.1.332-120x41.esc");

#[derive(Clone, Copy)]
struct Sample<'a> {
    frame: Option<&'a str>,
    latency: Duration,
    pause_after_receipt: Duration,
    cancel: bool,
}

impl<'a> Sample<'a> {
    const fn frame(frame: &'a str) -> Self {
        Self {
            frame: Some(frame),
            latency: Duration::ZERO,
            pause_after_receipt: Duration::ZERO,
            cancel: false,
        }
    }

    const fn delayed(mut self, millis: u64) -> Self {
        self.latency = Duration::from_millis(millis);
        self
    }
}

#[derive(Clone, Default)]
struct Clock {
    elapsed: Rc<Cell<Duration>>,
    pause_after_query: Rc<Cell<Duration>>,
}

impl HoldClock for Clock {
    fn now(&self) -> Duration {
        let observed = self.elapsed.get();
        self.elapsed.set(observed + self.pause_after_query.take());
        observed
    }

    fn pause(&mut self, gap: Duration) {
        self.elapsed.set(self.elapsed.get() + gap);
    }
}

#[derive(Debug)]
enum Operation {
    Read {
        from: Duration,
        until: Duration,
        holding: bool,
    },
    ManagedPolicy,
    Write {
        from: Duration,
    },
    Ack {
        until: Duration,
    },
}

struct Circuit<'a> {
    clock: Clock,
    gate: Arc<DispatchGate>,
    reads: VecDeque<Sample<'a>>,
    tail: Sample<'a>,
    replies: VecDeque<(Duration, Ack)>,
    stop_on_ack: Option<usize>,
    stop_on_write: Option<usize>,
    stop_finished: Option<Receiver<()>>,
    acks: usize,
    managed_settings_present: bool,
    trace: Vec<Operation>,
}

impl<'a> Circuit<'a> {
    fn start_with(reads: impl IntoIterator<Item = Sample<'a>>) -> Self {
        let mut samples = VecDeque::from([Sample::frame(IDLE), Sample::frame(WARM)]);
        samples.extend(reads);
        Self {
            clock: Clock::default(),
            gate: Arc::new(DispatchGate::new()),
            reads: samples,
            tail: Sample::frame(TRANSCRIBING),
            replies: VecDeque::new(),
            stop_on_ack: None,
            stop_on_write: None,
            stop_finished: None,
            acks: 0,
            managed_settings_present: false,
            trace: Vec::new(),
        }
    }

    fn reply_to_space(&mut self, index: usize, delay: Duration, ack: Ack) {
        assert!(index > self.replies.len());
        self.replies
            .resize_with(index - 1, || (Duration::ZERO, Ack::Ok));
        self.replies.push_back((delay, ack));
    }

    fn run(&mut self, hold_for: Duration) -> HoldOutcome {
        let plan = HoldPlan {
            hold: hold_for,
            gate: Arc::clone(&self.gate),
        };
        let mut clock = self.clock.clone();
        hold(self, &mut clock, &plan, &mut |_| {})
    }

    fn finish(&mut self) -> Settle {
        let mut clock = self.clock.clone();
        settle(self, &mut clock, &mut |_| {})
    }

    fn spaces(&self) -> usize {
        self.trace
            .iter()
            .filter(|op| matches!(op, Operation::Write { .. }))
            .count()
    }
}

impl HoldWire for Circuit<'_> {
    fn managed_settings_present(&mut self) -> bool {
        self.trace.push(Operation::ManagedPolicy);
        self.managed_settings_present
    }

    fn read(&mut self) -> Option<Reading> {
        assert!(
            self.trace.len() < 20_000,
            "the controller did not terminate"
        );
        let from = self.clock.now();
        let sample = self.reads.pop_front().unwrap_or(self.tail);
        self.clock.pause(sample.latency);
        if sample.cancel {
            self.gate.stop();
        }
        self.trace.push(Operation::Read {
            from,
            until: self.clock.now(),
            holding: matches!(sample.frame, Some(HOLD | INTERIM)),
        });
        // The controller can timestamp this receipt before a scheduler
        // pause. Every later clock observation sees the elapsed pause.
        self.clock.pause_after_query.set(sample.pause_after_receipt);
        sample.frame.map(|frame| Reading {
            frame: frame.to_owned(),
            cursor: Some((2, 2)),
        })
    }

    fn write(&mut self, keys: &[&str]) -> bool {
        assert_eq!(keys, ["Space"], "one command sends exactly one Space");
        let from = self.clock.now();
        if self.stop_on_write == Some(self.acks + 1) {
            let (attempt, started) = mpsc::channel();
            let (finished, done) = mpsc::channel();
            let gate = Arc::clone(&self.gate);
            std::thread::spawn(move || {
                let _ = attempt.send(());
                gate.stop();
                let _ = finished.send(());
            });
            assert!(started.recv_timeout(Duration::from_secs(2)).is_ok());
            assert_eq!(
                done.recv_timeout(Duration::from_millis(100)),
                Err(RecvTimeoutError::Timeout),
                "stop acknowledged while the key write still held the pipe"
            );
            self.stop_finished = Some(done);
        }
        self.trace.push(Operation::Write { from });
        true
    }

    fn ack(&mut self) -> Ack {
        self.acks += 1;
        if let Some(done) = self.stop_finished.take() {
            assert!(done.recv_timeout(Duration::from_secs(2)).is_ok());
        }
        if self.stop_on_ack == Some(self.acks) {
            let (tx, rx) = mpsc::channel();
            let gate = Arc::clone(&self.gate);
            let stopper = std::thread::spawn(move || {
                gate.stop();
                let _ = tx.send(());
            });
            assert!(
                rx.recv_timeout(Duration::from_secs(2)).is_ok(),
                "the pending acknowledgement held the dispatch guard"
            );
            assert!(stopper.join().is_ok());
        }
        let (latency, status) = self
            .replies
            .pop_front()
            .unwrap_or((Duration::ZERO, Ack::Ok));
        self.clock.pause(latency);
        self.trace.push(Operation::Ack {
            until: self.clock.now(),
        });
        status
    }
}

#[test]
fn each_repeat_requires_a_distinct_holding_read_after_the_previous_key_return() {
    let mut script = Circuit::start_with([Sample::frame(HOLD), Sample::frame(INTERIM)]);
    let _ = script.run(Duration::from_secs(1));
    assert_eq!(script.spaces(), 7);
    let mut last_end = Duration::ZERO;
    let mut fresh_hold = None;
    let mut previous_repeat = None;
    let mut writes = 0;
    let mut pending_ack = false;
    for operation in &script.trace {
        match operation {
            Operation::Read {
                from,
                until,
                holding: true,
            } => {
                assert!(
                    !pending_ack,
                    "a Holding read overtook the Space acknowledgement"
                );
                assert!(*from >= last_end, "{operation:?}");
                fresh_hold = Some(*until);
            }
            Operation::Write { from } => {
                assert!(
                    !pending_ack,
                    "a Space overtook its predecessor's acknowledgement"
                );
                pending_ack = true;
                writes += 1;
                if writes > 5 {
                    assert!(fresh_hold.take().is_some_and(|read| read <= *from));
                    if let Some(previous) = previous_repeat {
                        assert!(*from - previous >= Duration::from_millis(80));
                    }
                    previous_repeat = Some(*from);
                }
            }
            Operation::Ack { until } => {
                assert!(pending_ack, "an acknowledgement has no preceding Space");
                pending_ack = false;
                last_end = *until;
            }
            Operation::ManagedPolicy => {}
            Operation::Read { .. } => {
                assert!(!pending_ack, "a read overtook the Space acknowledgement");
                fresh_hold = None;
            }
        }
    }
    assert!(!pending_ack, "the last Space was not acknowledged");
}

#[test]
fn every_non_holding_read_after_a_repeat_ends_the_space_circuit() {
    let label_only = HOLD.replacen('▁', "", 1);
    for frame in [
        Some(IDLE),
        Some(WARM),
        Some(TAP),
        Some(TRANSCRIBING),
        Some(PLAIN_HOLD),
        Some(""),
        Some(&label_only),
        None,
    ] {
        let mut script = Circuit::start_with([
            Sample::frame(HOLD),
            Sample {
                frame,
                latency: Duration::ZERO,
                pause_after_receipt: Duration::ZERO,
                cancel: false,
            },
            Sample::frame(HOLD),
        ]);
        let _ = script.run(Duration::from_secs(1));
        assert_eq!(script.spaces(), 6, "{frame:?}");
    }
}

#[test]
fn a_holding_read_returned_past_the_deadline_cannot_authorize_a_repeat() {
    let mut script = Circuit::start_with([Sample::frame(HOLD), Sample::frame(HOLD).delayed(151)]);
    let _ = script.run(Duration::from_secs(1));
    assert_eq!(script.spaces(), 6);
    assert!(script.trace.iter().any(|effect| {
        matches!(effect, Operation::Read { from, until, holding: true }
            if *until - *from >= Duration::from_millis(151))
    }));
}

#[test]
fn a_late_first_holding_read_cannot_start_the_keep_alive_circuit() {
    let mut script = Circuit::start_with([
        Sample::frame(WARM).delayed(140),
        Sample::frame(WARM).delayed(140),
        Sample::frame(WARM).delayed(140),
        Sample::frame(HOLD).delayed(50),
    ]);
    let _ = script.run(Duration::from_secs(1));
    assert_eq!(script.spaces(), 5);
}

#[test]
fn the_first_holding_read_has_its_own_150_millisecond_stall_bound() {
    let mut timely = Circuit::start_with([Sample::frame(HOLD).delayed(149)]);
    let _ = timely.run(Duration::from_secs(1));
    assert_eq!(timely.spaces(), 6);

    let mut stalled = Circuit::start_with([Sample::frame(HOLD).delayed(151), Sample::frame(HOLD)]);
    let _ = stalled.run(Duration::from_secs(1));
    assert_eq!(stalled.spaces(), 5);
}

#[test]
fn a_holding_witness_aged_200_milliseconds_cannot_authorize_a_space() {
    let mut aged = Sample::frame(HOLD);
    aged.pause_after_receipt = Duration::from_millis(200);
    let mut script = Circuit::start_with([aged, Sample::frame(HOLD)]);
    let _ = script.run(Duration::from_secs(1));
    assert_eq!(script.spaces(), 5);
    assert!(script.trace.iter().any(|effect| {
        matches!(effect, Operation::Read { from, until, holding: true } if from == until)
    }));
}

#[test]
fn only_warming_can_wait_for_another_read_before_the_first_repeat() {
    let mut warm = Circuit::start_with([Sample::frame(WARM), Sample::frame(HOLD)]);
    let _ = warm.run(Duration::from_secs(1));
    assert_eq!(warm.spaces(), 6);

    for frame in [Some(""), None] {
        let mut torn = Circuit::start_with([
            Sample {
                frame,
                latency: Duration::ZERO,
                pause_after_receipt: Duration::ZERO,
                cancel: false,
            },
            Sample::frame(HOLD),
        ]);
        let _ = torn.run(Duration::from_secs(1));
        assert_eq!(torn.spaces(), 5, "torn first read: {frame:?}");
    }
}

#[test]
fn a_late_warmup_read_cannot_authorize_the_last_three_start_spaces() {
    let mut script = Circuit::start_with([Sample::frame(HOLD)]);
    script.reads[1] = Sample::frame(WARM).delayed(61);
    let _ = script.run(Duration::from_secs(1));
    assert_eq!(script.spaces(), 2);
}

#[test]
fn a_stalled_initial_idle_frame_cannot_authorize_the_start_probe() {
    let mut script = Circuit::start_with([Sample::frame(HOLD)]);
    script.reads[0] = Sample::frame(IDLE).delayed(151);
    let _ = script.run(Duration::from_secs(1));
    assert_eq!(script.spaces(), 0);
}

#[test]
fn the_final_idle_read_precedes_the_managed_policy_check_and_every_probe() {
    let mut refused = Circuit::start_with([Sample::frame(HOLD)]);
    refused.managed_settings_present = true;
    assert!(
        matches!(
            refused.run(Duration::from_secs(1)),
            HoldOutcome::Refused(Refusal::ManagedSettings)
        ),
        "a policy that appears after the app preflight prevents the start"
    );
    assert!(
        matches!(
            refused.trace.as_slice(),
            [
                Operation::Read { holding: false, .. },
                Operation::ManagedPolicy
            ]
        ),
        "{:?}",
        refused.trace
    );

    let mut allowed = Circuit::start_with([Sample::frame(HOLD)]);
    let _ = allowed.run(Duration::from_secs(1));
    assert!(
        matches!(
            allowed.trace.as_slice(),
            [
                Operation::Read { holding: false, .. },
                Operation::ManagedPolicy,
                Operation::Write { .. },
                ..
            ]
        ),
        "{:?}",
        allowed.trace
    );
    assert_eq!(
        allowed
            .trace
            .iter()
            .filter(|event| matches!(event, Operation::ManagedPolicy))
            .count(),
        1,
        "the circuit checks policy only before the first Space"
    );
}

#[test]
fn installed_shapes_never_read_the_managed_policy_fixture() {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use crate::doors::claude_managed_test_root_with;
    use crate::shape::{Shape, classify};

    let scratch = Path::new("/fixtures/extra-policy");
    let published = Path::new("/owner/.ae/versions/v2026.11.126/ae-core");
    assert_eq!(
        claude_managed_test_root_with(&Shape::Checkout, || Some(OsString::from(scratch))),
        Some(PathBuf::from(scratch))
    );
    assert_eq!(
        claude_managed_test_root_with(&Shape::Checkout, || Some(OsString::new())),
        None
    );

    let installed = classify(published, Some(Path::new("/owner")));
    let displaced = classify(published, Some(Path::new("/elsewhere")));
    assert!(matches!(installed, Shape::Installed { .. }));
    assert!(matches!(displaced, Shape::Displaced { .. }));
    for shape in [installed, displaced] {
        assert_eq!(
            claude_managed_test_root_with(&shape, || panic!("published ae read the fixture")),
            None
        );
    }
}

/// Keep the simulated host and checkout roots separate. No test changes an
/// actual system policy path or the process environment.
#[test]
fn checkout_policy_can_add_a_refusal_but_cannot_mask_host_policy() {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::dictate::{MANAGED, managed_policy_refusal};

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "ae-d54-managed-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).expect("owned private scratch");
    let scratch = Scratch(path);
    let host = scratch.0.join("host");
    let extra = scratch.0.join("extra");
    fs::create_dir(&host).expect("simulated host root");
    fs::create_dir(&extra).expect("checkout-only root");

    assert_eq!(managed_policy_refusal(Some(&host), None), None);
    assert_eq!(managed_policy_refusal(Some(&host), Some(&extra)), None);
    assert_eq!(managed_policy_refusal(None, Some(&extra)), Some(MANAGED));

    let host_file = host.join("managed-settings.json");
    fs::write(&host_file, b"not JSON").expect("host file need only exist");
    assert_eq!(
        managed_policy_refusal(Some(&host), Some(&extra)),
        Some(MANAGED),
        "an absent checkout file cannot mask a host policy file"
    );
    fs::remove_file(host_file).expect("clear simulated host policy");

    let host_drop_ins = host.join("managed-settings.d");
    fs::create_dir(&host_drop_ins).expect("empty host policy directory");
    assert_eq!(
        managed_policy_refusal(Some(&host), Some(&extra)),
        Some(MANAGED),
        "an absent checkout directory cannot mask a host policy directory"
    );
    fs::remove_dir(host_drop_ins).expect("clear simulated host directory");

    let extra_file = extra.join("managed-settings.json");
    fs::write(&extra_file, b"not JSON").expect("checkout file need only exist");
    assert_eq!(
        managed_policy_refusal(Some(&host), Some(&extra)),
        Some(MANAGED)
    );
    fs::remove_file(&extra_file).expect("clear checkout file");

    fs::create_dir(extra.join("managed-settings.d")).expect("empty checkout policy directory");
    assert_eq!(
        managed_policy_refusal(Some(&host), Some(&extra)),
        Some(MANAGED)
    );
    assert_eq!(managed_policy_refusal(Some(&host), None), None);
}

#[test]
fn captured_startup_chrome_preserves_the_idle_proof() {
    for frame in [IDLE, NOTICE_IDLE] {
        let mut script = Circuit::start_with([]);
        script.reads[0] = Sample::frame(frame);
        let _ = script.run(Duration::from_secs(1));
        assert_eq!(script.spaces(), 5, "the captured startup is idle: {frame}");
    }
}

fn notice_idle_with_model(model: &str) -> String {
    let notice = NOTICE_IDLE
        .lines()
        .find(|row| row.contains("/model opus ·"))
        .expect("the captured weekly-limit notice");
    let width = notice.chars().count();
    let modified = notice
        .trim_start()
        .replacen("/model opus ·", &format!("/model {model} ·"), 1);
    assert_ne!(modified, notice.trim_start(), "the model token was varied");
    assert!(modified.chars().count() <= width, "the notice must fit");
    // The captured notice is right aligned in the 120-column frame.
    NOTICE_IDLE.replacen(notice, &format!("{modified:>width$}"), 1)
}

#[test]
fn punctuation_or_space_in_the_weekly_limit_model_cannot_prove_idle() {
    let writes = ["opus!", "opus 5"].map(|model| {
        let frame = notice_idle_with_model(model);
        let mut script = Circuit::start_with([]);
        script.reads[0] = Sample::frame(&frame);
        let _ = script.run(Duration::from_secs(1));
        script.spaces()
    });
    assert_eq!(writes, [0, 0], "neither model token proves idle");
}

#[test]
fn a_dotted_weekly_limit_model_preserves_the_idle_proof() {
    let frame = notice_idle_with_model("opus5.5");
    let mut script = Circuit::start_with([]);
    script.reads[0] = Sample::frame(&frame);
    let _ = script.run(Duration::from_secs(1));
    assert_eq!(script.spaces(), 5, "the dotted model token preserves idle");
}

#[test]
fn unfamiliar_rows_and_a_spinner_behind_notices_cannot_prove_idle() {
    let spinner = BUSY
        .lines()
        .find(|row| row.starts_with('✻'))
        .expect("captured busy row");
    let busy = NOTICE_IDLE.replacen(
        "⚠ Your login expires",
        &format!("{spinner}\n⚠ Your login expires"),
        1,
    );
    assert_ne!(busy, NOTICE_IDLE);
    let unfamiliar = IDLE.replacen("◐ medium · /effort", "Unrecognized vendor state", 1);
    assert_ne!(unfamiliar, IDLE);
    let notices_alone = NOTICE_IDLE
        .find("⚠ Your login expires")
        .map(|start| &NOTICE_IDLE[start..])
        .expect("the notices without the captured startup banner");
    for frame in [&busy, &unfamiliar, notices_alone] {
        let mut script = Circuit::start_with([]);
        script.reads[0] = Sample::frame(frame);
        assert!(matches!(
            script.run(Duration::from_secs(1)),
            HoldOutcome::Refused(_)
        ));
        assert_eq!(script.spaces(), 0, "no positive idle witness: {frame}");
    }
}

#[test]
fn an_unfamiliar_row_above_the_startup_banner_refuses_before_the_probe() {
    let frame = format!("Unrecognized vendor state\n{IDLE}");
    let mut script = Circuit::start_with([]);
    script.reads[0] = Sample::frame(&frame);
    let _ = script.run(Duration::from_secs(1));
    assert_eq!(
        script.spaces(),
        0,
        "the startup banner cannot hide an unfamiliar row"
    );
}

#[test]
fn only_a_readable_idle_frame_can_wait_for_the_start_warmup() {
    let mut idle = Circuit::start_with([Sample::frame(HOLD)]);
    idle.reads.insert(1, Sample::frame(IDLE));
    let _ = idle.run(Duration::from_secs(1));
    assert_eq!(idle.spaces(), 6);

    let mut unread = Circuit::start_with([Sample::frame(HOLD)]);
    unread.reads.insert(1, Sample::frame(""));
    let _ = unread.run(Duration::from_secs(1));
    assert_eq!(unread.spaces(), 2);
}

#[test]
fn a_late_acknowledgement_aborts_the_start_burst() {
    let mut script = Circuit::start_with([Sample::frame(HOLD)]);
    script.reply_to_space(3, Duration::from_millis(101), Ack::Ok);
    let _ = script.run(Duration::from_secs(1));
    assert_eq!(script.spaces(), 3);
}

#[test]
fn each_space_of_a_start_burst_respects_stop_during_the_previous_ack() {
    for prior in 1..=5 {
        let mut script = Circuit::start_with([Sample::frame(HOLD)]);
        script.stop_on_ack = Some(prior);
        let _ = script.run(Duration::from_secs(1));
        assert!(script.gate.stopped());
        assert_eq!(script.spaces(), prior, "no Space after stop in ack {prior}");
    }
}

#[test]
fn stop_during_the_warmup_read_prevents_the_last_three_start_spaces() {
    let mut script = Circuit::start_with([Sample::frame(HOLD)]);
    script.reads[1].cancel = true;
    let _ = script.run(Duration::from_secs(1));
    assert!(script.gate.stopped());
    assert_eq!(script.spaces(), 2);
}

#[test]
fn the_first_probe_dispatch_starts_the_warmup_and_start_burst_deadlines() {
    let mut warmup = Circuit::start_with([Sample::frame(HOLD)]);
    warmup.reply_to_space(1, Duration::from_millis(40), Ack::Ok);
    warmup.reads[1] = Sample::frame(WARM).delayed(25);
    let _ = warmup.run(Duration::from_secs(1));
    assert_eq!(
        warmup.spaces(),
        2,
        "the warmup returned 65 ms after first probe"
    );

    let mut burst = Circuit::start_with([Sample::frame(HOLD)]);
    burst.reply_to_space(1, Duration::from_millis(50), Ack::Ok);
    burst.reply_to_space(4, Duration::from_millis(45), Ack::Ok);
    let _ = burst.run(Duration::from_secs(1));
    assert_eq!(
        burst.spaces(),
        4,
        "the fifth dispatch would start 95 ms after first probe"
    );
}

#[test]
fn the_first_burst_dispatch_starts_the_first_repeat_deadline() {
    // The circuit reads again 2 ms after each Warming receipt.
    let mut script = Circuit::start_with([
        Sample::frame(WARM).delayed(140),
        Sample::frame(WARM).delayed(140),
        Sample::frame(HOLD).delayed(50),
    ]);
    script.reply_to_space(4, Duration::from_millis(40), Ack::Ok);
    script.reply_to_space(5, Duration::from_millis(35), Ack::Ok);
    let _ = script.run(Duration::from_secs(1));
    assert_eq!(script.spaces(), 5);
    assert!(
        script.trace.iter().any(|effect| {
            matches!(effect, Operation::Read { from, until, holding: true }
            if *until > Duration::from_millis(450)
                && *until < Duration::from_millis(490)
                && *until - *from <= Duration::from_millis(150))
        }),
        "the Holding read must miss only the first-burst deadline"
    );
}

#[test]
fn the_first_hold_never_sends_a_repeat_if_only_warmup_is_seen() {
    let mut script = Circuit::start_with([]);
    script.tail = Sample::frame(WARM);
    let result = script.run(Duration::from_secs(20));
    assert_eq!(script.spaces(), 5);
    assert!(matches!(result, HoldOutcome::Lapsed { spaces: 0, .. }));
    assert!(script.trace.iter().all(|effect| {
        !matches!(effect, Operation::Read { from, .. } if *from > Duration::from_millis(450))
    }));
}

#[test]
fn the_previous_spaces_dispatch_time_bounds_the_next_read_and_key() {
    for (read_ms, expected) in [(19, 7), (71, 6)] {
        let mut script =
            Circuit::start_with([Sample::frame(HOLD), Sample::frame(HOLD).delayed(read_ms)]);
        script.reply_to_space(6, Duration::from_millis(80), Ack::Ok);
        let _ = script.run(Duration::from_secs(1));
        assert_eq!(script.spaces(), expected, "{read_ms} ms read");
    }
}

#[test]
fn a_late_command_acknowledgement_does_not_grant_another_space_window() {
    let mut script = Circuit::start_with([
        Sample::frame(HOLD),
        Sample::frame(HOLD),
        Sample::frame(HOLD),
    ]);
    script.reply_to_space(7, Duration::from_millis(180), Ack::Ok);
    let _ = script.run(Duration::from_secs(1));
    assert_eq!(script.spaces(), 7);
}

#[test]
fn the_controller_never_retries_a_space_with_an_uncertain_acknowledgement() {
    for status in [Ack::Failed, Ack::Timeout] {
        let mut script = Circuit::start_with([Sample::frame(HOLD), Sample::frame(HOLD)]);
        script.reply_to_space(6, Duration::from_millis(1), status);
        let _ = script.run(Duration::from_secs(1));
        assert_eq!(script.spaces(), 6);
    }
}

#[test]
fn a_holding_read_that_crosses_the_planned_stop_cannot_authorize_a_space() {
    let mut script = Circuit::start_with([Sample::frame(HOLD).delayed(125)]);
    let _ = script.run(Duration::from_millis(100));
    assert!(script.trace.iter().any(|effect| {
        matches!(effect, Operation::Read { from, until, holding: true }
            if *from < Duration::from_millis(100)
                && *until > Duration::from_millis(100)
                && *until < Duration::from_millis(450)
                && *until - *from <= Duration::from_millis(150))
    }));
    assert_eq!(script.spaces(), 5);
}

#[test]
fn a_cancel_that_arrives_inside_a_holding_read_sends_zero_further_spaces() {
    let mut stopped = Sample::frame(HOLD);
    stopped.cancel = true;
    let mut script = Circuit::start_with([Sample::frame(HOLD), stopped, Sample::frame(HOLD)]);
    let result = script.run(Duration::from_secs(1));
    assert!(script.gate.stopped());
    assert_eq!(script.spaces(), 6);
    assert!(matches!(
        result,
        HoldOutcome::Lapsed {
            why: Lapse::Stopped,
            spaces: 1,
        }
    ));
}

#[test]
fn a_pre_cancelled_plan_sends_no_start_probe() {
    let mut script = Circuit::start_with([Sample::frame(HOLD)]);
    script.gate.stop();
    let _ = script.run(Duration::from_secs(1));
    assert_eq!(script.spaces(), 0);
}

#[test]
fn a_stop_can_complete_with_an_acknowledgement_still_pending() {
    let mut script = Circuit::start_with([Sample::frame(HOLD), Sample::frame(HOLD)]);
    script.stop_on_ack = Some(6);
    let _ = script.run(Duration::from_secs(1));
    assert!(script.gate.stopped());
    assert_eq!(script.spaces(), 6);
}

#[test]
fn a_stop_acknowledgement_waits_for_an_in_flight_key_write() {
    let mut script = Circuit::start_with([Sample::frame(HOLD), Sample::frame(HOLD)]);
    script.stop_on_write = Some(6);
    let _ = script.run(Duration::from_secs(1));
    assert!(script.gate.stopped());
    assert_eq!(script.spaces(), 6);
}

#[test]
fn a_long_hold_sends_no_space_at_or_after_the_300_second_cap() {
    let mut script = Circuit::start_with([]);
    script.tail = Sample::frame(HOLD);
    let _ = script.run(Duration::from_secs(900));
    let last = script
        .trace
        .iter()
        .filter_map(|effect| match effect {
            Operation::Write { from } => Some(*from),
            _ => None,
        })
        .next_back();
    assert!(
        last.is_some_and(|last| {
            last >= Duration::from_millis(299_850) && last < Duration::from_secs(300)
        }),
        "last repeat: {last:?}"
    );
}

#[test]
fn settle_waits_for_transcribing_and_two_equal_box_reads_before_reporting_an_insert() {
    let partial = IDLE.replacen("❯\u{a0}", "❯\u{a0}One", 1);
    let full = IDLE.replacen("❯\u{a0}", "❯\u{a0}One two three.", 1);
    assert_ne!(partial, IDLE);
    assert_ne!(full, IDLE);
    let mut script = Circuit::start_with([]);
    script.reads = [
        Sample::frame(HOLD),
        Sample::frame(TRANSCRIBING),
        Sample::frame(&partial),
        Sample::frame(&full),
        Sample::frame(&full),
    ]
    .into();
    assert!(matches!(script.finish(), Settle::Inserted));
    assert!(script.reads.is_empty(), "the second full read is required");
    assert_eq!(script.spaces(), 0, "settle only reads");
}

#[test]
fn a_draft_without_transcribing_evidence_never_reports_an_insert() {
    let foreign = IDLE.replacen("❯\u{a0}", "❯\u{a0}Owner typed in Claude.", 1);
    assert_ne!(foreign, IDLE);
    let label_only = HOLD.replacen('▁', "", 1);
    let clipped_bar = HOLD.lines().skip(1).collect::<Vec<_>>().join("\n");
    let footer_word = IDLE.replace(" · ← for agents", " · listening…");
    for lookalike in [&label_only, &clipped_bar, &footer_word] {
        let mut script = Circuit::start_with([]);
        script.reads = [Sample::frame(lookalike)].into();
        script.tail = Sample::frame(&foreign);
        assert!(matches!(script.finish(), Settle::Unproved(_)));
        assert_eq!(script.spaces(), 0, "settle only reads");
    }
}

#[test]
fn settle_reports_no_text_after_two_empty_reads_follow_transcribing() {
    let mut script = Circuit::start_with([]);
    script.reads = [
        Sample::frame(TRANSCRIBING),
        Sample::frame(IDLE),
        Sample::frame(IDLE),
    ]
    .into();
    assert!(matches!(script.finish(), Settle::Empty));
    assert!(script.reads.is_empty(), "the second empty read is required");
    assert_eq!(script.spaces(), 0, "settle only reads");
}

#[test]
fn settle_refuses_a_missing_or_unread_frame_after_transcribing() {
    for frame in [None, Some("")] {
        let mut script = Circuit::start_with([]);
        script.reads = [
            Sample::frame(TRANSCRIBING),
            Sample {
                frame,
                latency: Duration::ZERO,
                pause_after_receipt: Duration::ZERO,
                cancel: false,
            },
        ]
        .into();
        assert!(matches!(script.finish(), Settle::Unproved(_)));
        assert_eq!(script.spaces(), 0, "settle only reads");
    }
}
