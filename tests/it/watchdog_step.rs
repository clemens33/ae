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
        }
    }

    /// Consume the startup cycle's done. It is the origin: [`Stepper::cycles`]
    /// stays zero.
    pub(super) fn ready(&mut self) {
        self.await_done();
    }

    /// Run exactly one more verdict cycle and return once it has completed.
    pub(super) fn step(&mut self) {
        self.signal(&self.channel);
        self.await_done();
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
        let deadline = Instant::now() + UNTIL_BOUND;
        let mut used = 0;
        loop {
            if ready() {
                return Some(used);
            }
            if Instant::now() >= deadline {
                return None;
            }
            self.step();
            used += 1;
        }
    }

    /// The cycles stepped since [`Stepper::ready`].
    pub(super) const fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Wait for one done. A timeout releases and reaps this wait client
    /// first, then fails naming the channel and the cycles completed.
    fn await_done(&self) {
        let mut waiter = super::cli::tmux_waiter(&self.socket, &self.done)
            .unwrap_or_else(|why| panic!("watchdog step: no wait client on {}: {why}", self.done));
        let deadline = Instant::now() + self.bound;
        loop {
            match waiter.try_wait() {
                Ok(Some(status)) => {
                    assert!(
                        status.success(),
                        "watchdog step: the wait on {} ended without a signal ({status}) after {} completed cycles",
                        self.done,
                        self.cycles
                    );
                    return;
                }
                Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
                Ok(None) | Err(_) => break,
            }
        }
        self.release(&self.done, &mut waiter);
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
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(self.socket.clone()),
        ));
        args.extend(["wait-for".to_owned(), "-S".to_owned(), channel.to_owned()]);
        let scratch = self.socket.parent().unwrap_or_else(|| Path::new("."));
        assert!(
            run_tmux(&args, scratch).0,
            "watchdog step: signal {channel} on {}",
            self.socket.display()
        );
    }
}

impl Drop for Stepper {
    /// One signal on the step channel, so a step wait a killed daemon left
    /// behind is released rather than left holding a server descriptor.
    fn drop(&mut self) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(self.socket.clone()),
        ));
        args.extend(["wait-for".to_owned(), "-S".to_owned(), self.channel.clone()]);
        let scratch = self.socket.parent().unwrap_or_else(|| Path::new("."));
        let _ = run_tmux(&args, scratch);
    }
}
