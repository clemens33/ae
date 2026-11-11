//! Drive a watchdog started with `_watchdog-run --step-channel <channel>` one
//! verdict cycle at a time.
//!
//! The daemon runs its startup cycle on its own, signals `<channel>-done`, and
//! then waits for a signal on `<channel>`. Each further cycle needs one signal
//! and acknowledges itself with one done. tmux keeps a signal that finds no
//! waiter for the next one, so neither side can miss the other. Every wait
//! client this helper starts ends by a signal on its own channel and is then
//! reaped: a killed `wait-for` client keeps a server descriptor until its
//! channel's next signal.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::cli::OwnedChild;
use super::phase2::run_tmux;

/// How long one cycle may take to acknowledge itself. A stepped cycle can
/// still hold the send helper's 30-second defer twice.
const DONE_BOUND: Duration = Duration::from_secs(90);

/// How long [`Stepper::step_until`] keeps stepping, in real time.
const UNTIL_BOUND: Duration = Duration::from_secs(90);

/// How long a released wait client gets to exit before it is killed: only a
/// server that stopped answering takes that long.
const RELEASE_BOUND: Duration = Duration::from_secs(5);

/// How often a pending wait client is looked at.
const POLL: Duration = Duration::from_millis(10);

/// One stepped daemon's two channels on one tmux server.
pub(super) struct Stepper {
    socket: PathBuf,
    channel: String,
    done: String,
    bound: Duration,
    cycles: u64,
    /// Set once [`Stepper::step_until_within`] gave up with a step in flight:
    /// that step's done may still come, so no later count would be exact.
    lost: bool,
}

impl Stepper {
    /// A stepper for the daemon on `socket` that steps on `channel`.
    pub(super) fn new(socket: &Path, channel: &str) -> Self {
        Self::with_timeout(socket, channel, DONE_BOUND)
    }

    /// [`Stepper::new`] whose every done wait gives up after `bound`.
    pub(super) fn with_timeout(socket: &Path, channel: &str, bound: Duration) -> Self {
        Self {
            socket: socket.to_owned(),
            channel: channel.to_owned(),
            done: format!("{channel}-done"),
            bound,
            cycles: 0,
            lost: false,
        }
    }

    /// Consume the startup cycle's done. It is the origin: [`Stepper::cycles`]
    /// stays zero.
    pub(super) fn ready(&mut self) {
        self.still_counting();
        self.await_done(None);
    }

    /// Run exactly one more verdict cycle and return once it has completed.
    pub(super) fn step(&mut self) {
        self.still_counting();
        self.signal(&self.channel);
        self.await_done(None);
        self.cycles += 1;
    }

    /// Run exactly `n` more verdict cycles, one after another.
    pub(super) fn steps(&mut self, n: u64) {
        for _ in 0..n {
            self.step();
        }
    }

    /// Check `ready`, and step while it is false. Returns the cycles this call
    /// used, or `None` once [`UNTIL_BOUND`] of real time has passed, so the
    /// caller fails with its own diagnostics.
    pub(super) fn step_until(&mut self, ready: impl Fn() -> bool) -> Option<u64> {
        self.step_until_within(UNTIL_BOUND, ready)
    }

    /// [`Stepper::step_until`] with its own real-time `bound`. The bound holds
    /// inside a step too: a done still missing when it passes is released and
    /// reaped, and the call returns `None`.
    pub(super) fn step_until_within(
        &mut self,
        bound: Duration,
        ready: impl Fn() -> bool,
    ) -> Option<u64> {
        self.still_counting();
        let deadline = Instant::now() + bound;
        let mut used = 0;
        loop {
            if ready() {
                return Some(used);
            }
            if Instant::now() >= deadline {
                return None;
            }
            self.signal(&self.channel);
            if !self.await_done(Some(deadline)) {
                self.lost = true;
                return None;
            }
            self.cycles += 1;
            used += 1;
        }
    }

    /// The cycles stepped since [`Stepper::ready`].
    pub(super) const fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Refuse to go on after [`Stepper::step_until_within`] gave up with a
    /// step in flight, before any signal or predicate.
    fn still_counting(&self) {
        assert!(
            !self.lost,
            "watchdog step: {} gave up with a step in flight after {} completed cycles; \
             it cannot count further",
            self.channel, self.cycles
        );
    }

    /// Wait for one done, until this stepper's own bound or `until`, whichever
    /// comes first. Either way the wait client is released and reaped first.
    /// Past `until` the answer is `false`; past the own bound the wait fails,
    /// naming the channel and the cycles completed.
    fn await_done(&self, until: Option<Instant>) -> bool {
        let mut waiter = super::cli::tmux_waiter(&self.socket, &self.done)
            .unwrap_or_else(|why| panic!("watchdog step: no wait client on {}: {why}", self.done));
        let own = Instant::now() + self.bound;
        let deadline = until.map_or(own, |until| until.min(own));
        loop {
            match waiter.try_wait() {
                Ok(Some(status)) => {
                    assert!(
                        status.success(),
                        "watchdog step: the wait on {} ended without a signal ({status}) after {} completed cycles",
                        self.done,
                        self.cycles
                    );
                    return true;
                }
                Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
                Ok(None) | Err(_) => break,
            }
        }
        self.release(&self.done, &mut waiter);
        if until.is_some_and(|until| until < own) {
            return false;
        }
        panic!(
            "watchdog step: {} not signalled within {:?} after {} completed cycles",
            self.done, self.bound, self.cycles
        );
    }

    /// End `waiter` the way it is meant to end, with a signal on its own
    /// channel, and reap it. Past [`RELEASE_BOUND`] the drop kills it.
    fn release(&self, channel: &str, waiter: &mut OwnedChild) {
        self.signal(channel);
        let deadline = Instant::now() + RELEASE_BOUND;
        while matches!(waiter.try_wait(), Ok(None)) && Instant::now() < deadline {
            std::thread::sleep(POLL);
        }
    }

    /// Signal `channel` on this stepper's server.
    fn signal(&self, channel: &str) {
        assert!(
            signal(&self.socket, channel),
            "watchdog step: signal {channel} on {}",
            self.socket.display()
        );
    }
}

impl Drop for Stepper {
    /// One signal on the step channel, so a step wait a killed daemon left
    /// behind is released rather than left holding a server descriptor.
    fn drop(&mut self) {
        let _ = signal(&self.socket, &self.channel);
    }
}

/// Signal `channel` once on the tmux server at `socket`.
fn signal(socket: &Path, channel: &str) -> bool {
    let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
        ae::meta::Selector::Socket(socket.to_owned()),
    ));
    args.extend(["wait-for", "-S", "--", channel].map(str::to_owned));
    run_tmux(&args, socket.parent().unwrap_or_else(|| Path::new("."))).0
}

/// The helper's own regressions, on a real private tmux server.
mod regressions {
    #![allow(
        clippy::disallowed_methods,
        clippy::expect_used,
        reason = "the regressions read and write only their own scratch fixture"
    )]

    use std::time::{Duration, Instant};

    use super::super::cli::OwnedChild;
    use super::{POLL, Stepper, signal};

    /// Start a quota-off stepped daemon for `rig` on `channel`, its quota trace in
    /// `step-trace`: one `skipped` line per iteration.
    fn stepped_daemon(rig: &super::super::seat_relaunch::Rig, channel: &str) -> OwnedChild {
        let config = rig.scratch.join("config");
        let text = std::fs::read_to_string(&config).expect("fixture config");
        std::fs::write(&config, format!("{text}quota = off\n")).expect("quota off, receipts kept");
        std::fs::write(rig.scratch.join("step-trace"), "").expect("fresh receipt file");
        super::super::cli::ae()
            .arg(ae::cli::WATCHDOG_RUN)
            .arg(&rig.dir)
            .args(["--interval", "1", "--step-channel", channel])
            .args([
                "--quota-every-secs",
                "1",
                "--idle-nudge-secs",
                "0",
                "--tg-supervise-secs",
                "0",
            ])
            .env("HOME", &rig.scratch)
            .env("AE_HOME", &rig.scratch)
            .env("CONFIG_FILE", &config)
            .env("AE_TEST_QUOTA_TRACE", rig.scratch.join("step-trace"))
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env_remove("AE_SENDER_OVERRIDE")
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(rig.scratch.join("step-err")).expect("daemon log"))
            .spawn()
            .expect("real stepped watchdog")
    }

    #[test]
    fn a_leading_dash_channel_steps_a_real_daemon_exactly() {
        // `-` may lead a channel name the grammar accepts; tmux must read it as
        // the channel, not as a flag, on every wait and every signal.
        let rig = super::super::seat_relaunch::Rig::new("stepdash");
        let channel = "-stepdash";
        let mut daemon = stepped_daemon(&rig, channel);
        let mut stepper = Stepper::with_timeout(&rig.sock, channel, Duration::from_secs(20));
        stepper.ready();
        stepper.steps(2);
        assert_eq!(stepper.cycles(), 2);
        let trace = std::fs::read_to_string(rig.scratch.join("step-trace")).expect("receipts");
        assert_eq!(
            trace,
            "skipped\n".repeat(3),
            "the startup cycle and two steps, nothing more"
        );
        std::fs::remove_file(rig.dir.join("meta")).expect("request the ordinary stop");
        assert!(signal(&rig.sock, channel), "signal {channel}");
        let deadline = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(status) = daemon.try_wait().expect("read daemon exit") {
                break status;
            }
            assert!(Instant::now() < deadline, "the stepped daemon did not stop");
            std::thread::sleep(POLL);
        };
        assert!(status.success(), "{status}");
    }

    #[test]
    fn step_until_returns_none_at_its_overall_bound_while_a_done_is_late() {
        // No daemon: the only done is one this test sends after the overall bound.
        let rig = super::super::seat_relaunch::Rig::new("stepbound");
        let late = {
            let socket = rig.sock.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(3));
                signal(&socket, "stepbound-done")
            })
        };
        let mut stepper = Stepper::with_timeout(&rig.sock, "stepbound", Duration::from_secs(30));
        let start = Instant::now();
        let got = stepper.step_until_within(Duration::from_secs(1), || false);
        let took = start.elapsed();
        assert_eq!(got, None);
        assert!(
            took >= Duration::from_secs(1) && took < Duration::from_secs(3),
            "returned at its own bound, before the late done: {took:?}"
        );
        assert_eq!(
            stepper.cycles(),
            0,
            "the late cycle never completed inside the bound"
        );
        assert!(
            late.join().expect("late signaller"),
            "the late done was sent"
        );
    }
}
