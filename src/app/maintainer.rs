//! The Statistics store's upkeep thread in `ae app`: one thread,
//! `ae-app-stats`, steps [`Maintainer`] and waits [`maint::pause`] between
//! steps on its stop channel, so a stop wakes it at once. The app owns the
//! thread through [`Upkeep`] for as long as it runs: it stops the thread
//! before an exec replaces the process and starts it again when the exec
//! failed. A thread still in its step when that stop's bound passes stays
//! owned until it ends, and no other starts before then, so at most one runs.
//! Dropping the [`Running`] handle, on any return, stops the thread and waits
//! up to [`STOP_BOUND`] for it. A step still running then is abandoned: the
//! process ending mid-publication is a crash the store's recovery finishes.
//!
//! A repair, and the stop after damage met again, reach the app as one
//! [`Wake::Notice`] each, naming the file and why.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use super::loader::Wake;
use crate::stats_store::disk::LIMITS;
use crate::stats_store::maint::{self, Maintainer, Outcome};

/// How long a stop waits for the thread to finish its step.
pub(super) const STOP_BOUND: Duration = Duration::from_millis(250);

/// How long one publication waits for the store's lock.
const LOCK_WAIT: Duration = Duration::from_millis(100);

/// The running upkeep thread.
pub(super) struct Running {
    stop: Option<Sender<()>>,
    done: Option<Receiver<()>>,
    worker: Option<JoinHandle<()>>,
    /// The completion token the thread holds: gone once its loop is over.
    token: Weak<()>,
    #[cfg_attr(not(test), allow(dead_code, reason = "observed only by tests"))]
    watch: Watch,
}

/// What tests observe of the thread: nothing in the product.
struct Watch {
    #[cfg(test)]
    parked: Arc<std::sync::atomic::AtomicBool>,
}

impl Watch {
    /// What the thread's `probe` shows.
    #[cfg_attr(not(test), allow(unused_variables, reason = "observed only by tests"))]
    fn of(probe: &Probe) -> Self {
        Self {
            #[cfg(test)]
            parked: Arc::clone(&probe.parked),
        }
    }
}

/// Whether the thread waits between steps, and the gate a test holds it at:
/// observed only by tests.
#[derive(Clone, Default)]
struct Probe {
    #[cfg(test)]
    parked: Arc<std::sync::atomic::AtomicBool>,
    #[cfg(test)]
    gate: Arc<Gate>,
}

impl Probe {
    #[cfg_attr(
        not(test),
        allow(clippy::unused_self, reason = "observed only by tests")
    )]
    fn park(&self, on: bool) {
        #[cfg(test)]
        {
            self.parked.store(on, std::sync::atomic::Ordering::SeqCst);
        }
        #[cfg(not(test))]
        let _ = on;
    }

    /// The top of a step: a test's armed gate holds the thread here, active,
    /// until its [`Hold`] is dropped.
    #[cfg_attr(not(test), allow(clippy::unused_self, reason = "held only by tests"))]
    fn pass(&self) {
        #[cfg(test)]
        {
            self.gate.pass();
        }
    }
}

/// Start the upkeep of the store under `root`, its notices sent on `notices`.
pub(super) fn start(root: PathBuf, notices: Sender<Wake>) -> std::io::Result<Running> {
    spawn(root, notices, Probe::default(), None)
}

/// What the app hands the thread: stop, or reset the store for the
/// Statistics tab of an overlay generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Command {
    Stop,
    Reset(u64),
}

/// Where a reset went: to a running thread, or held until one runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Handed {
    Sent,
    Held,
}

/// Start the thread over `seed` when given, else over a new maintainer
/// made inside the thread.
fn spawn(
    root: PathBuf,
    notices: Sender<Wake>,
    probe: Probe,
    seed: Option<Maintainer>,
) -> std::io::Result<Running> {
    let (stop, stopped) = mpsc::channel();
    let (finished, done) = mpsc::channel();
    let token = Arc::new(());
    let ended = Arc::downgrade(&token);
    let watch = Watch::of(&probe);
    let worker = std::thread::Builder::new()
        .name("ae-app-stats".to_owned())
        .spawn(move || {
            upkeep(&root, seed, &stopped, &notices, &probe);
            // Completion is the token gone, before the done signal.
            drop(token);
            let _ = finished.send(());
        })?;
    Ok(Running {
        stop: Some(stop),
        done: Some(done),
        worker: Some(worker),
        token: ended,
        watch,
    })
}

/// Step until stopped.
fn upkeep(
    root: &Path,
    seed: Option<Maintainer>,
    stop: &Receiver<()>,
    notices: &Sender<Wake>,
    probe: &Probe,
) {
    let mut maintainer = seed.unwrap_or_else(|| Maintainer::new(root, LIMITS, LOCK_WAIT));
    let mut told = false;
    loop {
        probe.pass();
        let turn = maintainer.step();
        match &turn.outcome {
            Outcome::Repaired { file, reason } => {
                let line =
                    format!("Statistics: {file} was damaged ({reason}); rebuilding the store");
                let _ = notices.send(Wake::Notice(line));
            }
            Outcome::Refused(why) if maintainer.stopped() && !told => {
                told = true;
                let line = format!(
                    "Statistics: the store was damaged again ({}); upkeep stopped",
                    why.describe()
                );
                let _ = notices.send(Wake::Notice(line));
            }
            _ => {}
        }
        let wait = if maintainer.stopped() {
            maint::pause(&turn).max(maint::REFRESH)
        } else {
            maint::pause(&turn)
        };
        probe.park(true);
        let woke = stop.recv_timeout(wait);
        probe.park(false);
        if !matches!(woke, Err(RecvTimeoutError::Timeout)) {
            return;
        }
    }
}

impl Running {
    /// Stop the thread: `true` when it ended within `bound`.
    #[cfg(test)]
    pub(super) fn stop(mut self, bound: Duration) -> bool {
        self.halt(bound)
    }

    /// Tell the thread to stop, once, and wait up to `bound` for it: `true`
    /// when it ended, joined. A thread still in its step keeps its handle,
    /// so a later call waits for the same thread again.
    fn halt(&mut self, bound: Duration) -> bool {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let Some(done) = self.done.take() else {
            return self.worker.is_none();
        };
        if done.recv_timeout(bound) == Err(RecvTimeoutError::Timeout) {
            self.done = Some(done);
            return false;
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        true
    }

    /// Whether the thread's loop is over: its token is gone.
    fn finished(&self) -> bool {
        self.token.upgrade().is_none()
    }
}

impl Drop for Running {
    /// A thread never told to stop gets the whole bound; one already told
    /// and waited for is only looked at once more.
    fn drop(&mut self) {
        let bound = if self.stop.is_some() {
            STOP_BOUND
        } else {
            Duration::ZERO
        };
        self.halt(bound);
    }
}

/// The app's one owner of the upkeep thread, when it could start one: it
/// stops the thread before the process is replaced, and starts it again when
/// that failed, never while the stopped one is still in its step.
pub(super) struct Upkeep {
    running: Option<Running>,
    /// A thread told to stop that was still in its step when the bound
    /// passed: still owned, until it ends.
    stopping: Option<Running>,
    /// A thread is owed again: an exec failed after the stop.
    owed: bool,
    root: PathBuf,
    notices: Sender<Wake>,
}

impl Upkeep {
    /// Start the thread; an app that cannot start it runs without it.
    pub(super) fn start(root: PathBuf, notices: Sender<Wake>) -> Self {
        Self {
            running: start(root.clone(), notices.clone()).ok(),
            stopping: None,
            owed: false,
            root,
            notices,
        }
    }

    /// Stop the thread within `bound`, before an exec replaces the process:
    /// a thread still in its step past it goes on to the exec, kept as
    /// stopping so it stays owned if the exec fails.
    pub(super) fn pause(&mut self, bound: Duration) {
        if let Some(mut running) = self.running.take()
            && !running.halt(bound)
        {
            self.stopping = Some(running);
        }
    }

    /// The exec failed: a thread is owed again, started at once unless the
    /// stopped one is still in its step.
    pub(super) fn resume(&mut self) {
        self.owed = true;
        self.tend();
    }

    /// Every app turn: a stopped thread that has ended is reaped, and only
    /// then is an owed thread started.
    pub(super) fn tend(&mut self) {
        if self.stopping.as_ref().is_some_and(Running::finished)
            && let Some(mut stopped) = self.stopping.take()
        {
            // Its loop is over: what is left is the send of its done signal.
            stopped.halt(STOP_BOUND);
        }
        if self.owed && self.stopping.is_none() && self.running.is_none() {
            self.owed = false;
            self.running = start(self.root.clone(), self.notices.clone()).ok();
        }
    }

    /// Stop the thread within `bound`: the running one, or else the one
    /// still stopping.
    pub(super) fn stop(mut self, bound: Duration) {
        if let Some(mut running) = self.running.take() {
            running.halt(bound);
        } else if let Some(mut stopped) = self.stopping.take() {
            stopped.halt(bound);
        }
    }

    /// Hand the thread a reset for the Statistics tab of `generation`; held
    /// once while no thread runs, and handed when one runs again.
    pub(super) fn reset(&mut self, generation: u64) -> Handed {
        // B0 seam: nothing handed yet.
        let _ = generation;
        Handed::Held
    }

    /// The reset held until a thread runs, if any.
    #[cfg(test)]
    pub(super) fn owed_reset(&self) -> Option<u64> {
        // B0 seam: nothing held yet.
        None
    }

    /// The thread this owner runs, if any.
    #[cfg(test)]
    pub(super) fn running(&self) -> Option<&Running> {
        self.running.as_ref()
    }

    /// The stopped thread this owner still waits for, if any.
    #[cfg(test)]
    pub(super) fn stopping(&self) -> Option<&Running> {
        self.stopping.as_ref()
    }

    /// [`Upkeep::start`], its thread held at the top of its first step until
    /// the returned [`Hold`] is dropped: armed before the thread starts.
    #[cfg(test)]
    pub(super) fn start_holding(root: PathBuf, notices: Sender<Wake>) -> (Self, Hold) {
        let probe = Probe::default();
        probe.gate.arm();
        let hold = Hold(Arc::clone(&probe.gate));
        let upkeep = Self {
            running: spawn(root.clone(), notices.clone(), probe, None).ok(),
            stopping: None,
            owed: false,
            root,
            notices,
        };
        (upkeep, hold)
    }

    /// [`Upkeep::start_holding`] over the given `maintainer` instead of a
    /// new one: a test drives a real one to the state it needs first.
    #[cfg(test)]
    pub(super) fn start_with_holding(
        root: PathBuf,
        notices: Sender<Wake>,
        maintainer: Maintainer,
    ) -> (Self, Hold) {
        let probe = Probe::default();
        probe.gate.arm();
        let hold = Hold(Arc::clone(&probe.gate));
        let upkeep = Self {
            running: spawn(root.clone(), notices.clone(), probe, Some(maintainer)).ok(),
            stopping: None,
            owed: false,
            root,
            notices,
        };
        (upkeep, hold)
    }
}

/// Where a test holds the thread: armed, entered, released.
#[cfg(test)]
#[derive(Default)]
struct Gate {
    state: std::sync::Mutex<GateState>,
    turned: std::sync::Condvar,
}

#[cfg(test)]
#[derive(Default)]
struct GateState {
    armed: bool,
    entered: bool,
    released: bool,
}

#[cfg(test)]
impl Gate {
    fn state(&self) -> std::sync::MutexGuard<'_, GateState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn arm(&self) {
        self.state().armed = true;
    }

    /// Hold the calling thread while armed and not released, once.
    fn pass(&self) {
        let mut state = self.state();
        if !state.armed {
            return;
        }
        state.entered = true;
        self.turned.notify_all();
        let mut state = self
            .turned
            .wait_while(state, |state| !state.released)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.armed = false;
    }

    fn release(&self) {
        self.state().released = true;
        self.turned.notify_all();
    }
}

/// A test's hold on the upkeep thread: dropping it, on any return or
/// unwind, lets the thread go on.
#[cfg(test)]
pub(super) struct Hold(Arc<Gate>);

#[cfg(test)]
impl Hold {
    /// Whether the thread reached the gate within `within`.
    pub(super) fn entered(&self, within: Duration) -> bool {
        let state = self.0.state();
        let (state, _) = self
            .0
            .turned
            .wait_timeout_while(state, within, |state| !state.entered)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.entered
    }
}

#[cfg(test)]
impl Drop for Hold {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[cfg(test)]
impl Running {
    /// The thread's completion, observable after the handle is gone.
    pub(super) fn ended(&self) -> Ended {
        Ended(self.token.clone())
    }

    /// Whether the thread is waiting between steps.
    pub(super) fn parked(&self) -> bool {
        self.watch.parked.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// The thread's completion.
#[cfg(test)]
pub(super) struct Ended(std::sync::Weak<()>);

#[cfg(test)]
impl Ended {
    pub(super) fn is_ended(&self) -> bool {
        self.0.upgrade().is_none()
    }

    /// Whether `other` is the completion of the same thread.
    pub(super) fn same_worker(&self, other: &Self) -> bool {
        std::sync::Weak::ptr_eq(&self.0, &other.0)
    }
}

#[cfg(test)]
#[path = "s5_upkeep.rs"]
mod s5_upkeep;
#[cfg(test)]
#[path = "maintainer_s2_spec.rs"]
mod spec;
