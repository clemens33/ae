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
//!
//! The Statistics tab's `r` reaches the thread as a [`Command::Reset`] on the
//! same channel as its stop, so one handed before a stop still runs. The
//! thread resets the store under its lock, waiting at most
//! [`disk::RESET_WAIT`], starts over with a fresh maintainer when the reset
//! took (one stopped on damage included), and answers exactly one
//! [`Wake::StatsReset`] per reset it was handed.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::loader::Wake;
use crate::stats_store::disk::{self, LIMITS};
use crate::stats_store::maint::{self, Maintainer, Outcome};

/// How long a stop waits for the thread to finish its step.
pub(super) const STOP_BOUND: Duration = Duration::from_millis(250);

/// How long one publication waits for the store's lock.
const LOCK_WAIT: Duration = Duration::from_millis(100);

/// The running upkeep thread.
pub(super) struct Running {
    /// The command channel; taken once the thread is told to stop.
    stop: Option<Sender<Command>>,
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
    #[cfg(test)]
    turns: Arc<std::sync::Mutex<Vec<Beat>>>,
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

    /// Keep `turn` for [`Self::take_turns`] (tests only).
    #[cfg_attr(
        not(test),
        allow(clippy::unused_self, reason = "observed only by tests")
    )]
    fn record(&self, turn: &Beat) {
        #[cfg(test)]
        if let Ok(mut turns) = self.turns.lock() {
            turns.push(turn.clone());
        }
        #[cfg(not(test))]
        let _ = turn;
    }

    /// The turns recorded since the last call.
    #[cfg(test)]
    fn take_turns(&self) -> Vec<Beat> {
        self.turns
            .lock()
            .map(|mut turns| std::mem::take(&mut *turns))
            .unwrap_or_default()
    }
}

/// While S5 has work, the longest Tools waits for its next unit.
const TOOLS_FAIR: Duration = Duration::from_secs(1);
/// How long a finished Tools round rests before the next one starts.
const TOOLS_ROUND: Duration = Duration::from_secs(30);
/// The wait between turns while a Tools round is in progress.
const TOOLS_BREATH: Duration = Duration::from_millis(10);

/// One upkeep turn: the S5 step, then at most one Tools unit, on `thread`.
#[derive(Debug, Clone)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the Tools unit and thread are observed only by tests"
    )
)]
pub(super) struct Turn {
    pub s5: maint::Step,
    pub tools: Option<crate::usage::calls_read::Unit>,
    pub thread: std::thread::ThreadId,
}

/// What one wake of the upkeep thread did: a [`Turn`] when S5 was due (its
/// step), else at most one Tools unit. What tests observe.
#[derive(Debug, Clone)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the Tools unit and thread are observed only by tests"
    )
)]
pub(super) struct Beat {
    pub s5: Option<maint::Step>,
    pub tools: Option<crate::usage::calls_read::Unit>,
    pub thread: std::thread::ThreadId,
}

impl From<Turn> for Beat {
    fn from(turn: Turn) -> Self {
        Self {
            s5: Some(turn.s5),
            tools: turn.tools,
            thread: turn.thread,
        }
    }
}

/// The loop body (#288 plan r4 §5). S5 steps when its own pause after its
/// last step has passed: [`maint::pause`] after the step's end, at least
/// [`maint::REFRESH`] once it stopped. That step is a [`Turn`]: the S5 step
/// first, then at most one Tools unit. Between S5 steps, a Tools round in
/// progress takes one unit per wake, never an S5 step. Tools runs only when
/// S5 last found nothing to do, or when it has not run for [`TOOLS_FAIR`]:
/// a busy S5 keeps its priority, and cannot starve Tools. Each Tools unit
/// waits at least as long as it took (and [`TOOLS_BREATH`]) before the next,
/// so Tools too holds at most about half a core. A round's first unit reads
/// the S5 epoch: without a proven one the round ends there, before any
/// discovery or transcript read. This thread is the only reader of
/// transcripts.
pub(super) struct UpkeepStep {
    maintainer: Maintainer,
    tools: crate::usage::calls_read::Reader,
    /// When the last Tools unit ran.
    ran: Option<Instant>,
    /// When the last Tools round ended.
    rested: Option<Instant>,
    /// When S5 steps next: `None` at once.
    s5_due: Option<Instant>,
    /// Whether S5's last step found nothing to do.
    s5_idle: bool,
    /// When the next Tools unit may run: `None` at once.
    tools_due: Option<Instant>,
}

impl UpkeepStep {
    #[cfg_attr(not(test), allow(dead_code, reason = "the thread starts from a seed"))]
    pub(super) fn new(root: &Path) -> Self {
        Self::with(root, Maintainer::new(root, LIMITS, LOCK_WAIT))
    }

    fn with(root: &Path, maintainer: Maintainer) -> Self {
        Self {
            maintainer,
            tools: crate::usage::calls_read::Reader::new(
                root.to_owned(),
                crate::doors::home(),
                crate::tool_stats::Caps::SLICE,
            ),
            ran: None,
            rested: None,
            s5_due: None,
            s5_idle: false,
            tools_due: None,
        }
    }

    /// The S5 step, then at most one Tools unit: what [`Self::beat`] does
    /// when S5 is due.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "the thread beats; the frozen specs turn")
    )]
    pub(super) fn turn(&mut self, now: Instant) -> Turn {
        self.turn_with(now, Maintainer::step)
    }

    /// One wake of the upkeep thread at `now`.
    fn beat(&mut self, now: Instant) -> Beat {
        self.beat_with(now, Maintainer::step)
    }

    /// [`Self::beat`] with the S5 step taken by `s5`: tests drive the
    /// schedule with synthetic steps on a simulated clock.
    fn beat_with(&mut self, now: Instant, s5: impl FnOnce(&mut Maintainer) -> maint::Step) -> Beat {
        if self.s5_due.is_none_or(|due| now >= due) {
            return self.turn_with(now, s5).into();
        }
        let began = Instant::now();
        let tools = if self.tools.busy() && self.tools_allowed(now) {
            self.tools_unit(now, began)
        } else {
            None
        };
        Beat {
            s5: None,
            tools,
            thread: std::thread::current().id(),
        }
    }

    fn turn_with(&mut self, now: Instant, s5: impl FnOnce(&mut Maintainer) -> maint::Step) -> Turn {
        let began = Instant::now();
        let s5 = s5(&mut self.maintainer);
        let pause = if self.maintainer.stopped() {
            maint::pause(&s5).max(maint::REFRESH)
        } else {
            maint::pause(&s5)
        };
        self.s5_due = now.checked_add(s5.active.saturating_add(pause));
        self.s5_idle = s5.outcome == Outcome::Idle;
        let tools = if self.tools_allowed(now) {
            self.tools_unit(now, began)
        } else {
            None
        };
        Turn {
            s5,
            tools,
            thread: std::thread::current().id(),
        }
    }

    /// Whether a Tools unit may run at `now`: S5 last idle or Tools owed its
    /// fair turn, and the last unit's own wait over.
    fn tools_allowed(&self, now: Instant) -> bool {
        let fair = self
            .ran
            .is_none_or(|ran| now.saturating_duration_since(ran) >= TOOLS_FAIR);
        (self.s5_idle || fair) && self.tools_due.is_none_or(|due| now >= due)
    }

    /// One unit of the Tools round, starting one once the last has rested.
    /// `began` is the real time the wake at `now` began, so the unit's own
    /// wait counts from its end.
    fn tools_unit(
        &mut self,
        now: Instant,
        began: Instant,
    ) -> Option<crate::usage::calls_read::Unit> {
        if !self.tools.busy() {
            if self
                .rested
                .is_some_and(|rested| now.saturating_duration_since(rested) < TOOLS_ROUND)
            {
                return None;
            }
            self.tools.round();
        }
        let started = Instant::now();
        let unit = self.tools.step();
        let spent = started.elapsed();
        if let Some(unit) = unit {
            self.ran = Some(now);
            let end = now.checked_add(began.elapsed());
            self.tools_due = end.and_then(|end| end.checked_add(spent.max(TOOLS_BREATH)));
            if unit.kind == crate::usage::calls_read::UnitKind::Load && !self.tools.proven() {
                // No proven S5 epoch: no discovery and no transcript read.
                self.tools.abandon();
            }
        } else {
            self.rested = Some(now);
            self.tools_due = None;
        }
        unit
    }

    /// The wait after a wake at `now`: until S5 is due, or sooner the next
    /// Tools unit of a round in progress.
    fn wait(&self, now: Instant) -> Duration {
        let mut wake = self.s5_due.unwrap_or(now);
        if self.tools.busy() {
            let due = self.tools_due.unwrap_or(now);
            let allowed = if self.s5_idle {
                due
            } else {
                due.max(
                    (self.ran)
                        .and_then(|ran| ran.checked_add(TOOLS_FAIR))
                        .unwrap_or(now),
                )
            };
            wake = wake.min(allowed);
        }
        wake.saturating_duration_since(now)
    }

    /// S5 steps at the next wake: its maintainer was replaced.
    fn restart_s5(&mut self) {
        self.s5_due = None;
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
    commands: &Receiver<Command>,
    notices: &Sender<Wake>,
    probe: &Probe,
) {
    let maintainer = seed.unwrap_or_else(|| Maintainer::new(root, LIMITS, LOCK_WAIT));
    let mut step = UpkeepStep::with(root, maintainer);
    let mut told = false;
    loop {
        probe.pass();
        let turn = step.beat(Instant::now());
        probe.record(&turn);
        let maintainer = &step.maintainer;
        match turn.s5.as_ref().map(|s5| &s5.outcome) {
            Some(Outcome::Repaired { file, reason }) => {
                let line =
                    format!("Statistics: {file} was damaged ({reason}); rebuilding the store");
                let _ = notices.send(Wake::Notice(line));
            }
            Some(Outcome::Refused(why)) if maintainer.stopped() && !told => {
                told = true;
                let line = format!(
                    "Statistics: the store was damaged again ({}); upkeep stopped",
                    why.describe()
                );
                let _ = notices.send(Wake::Notice(line));
            }
            _ => {}
        }
        let wait = step.wait(Instant::now());
        probe.park(true);
        let woke = commands.recv_timeout(wait);
        probe.park(false);
        match woke {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(Command::Reset(generation)) => {
                let outcome = disk::reset(root, disk::RESET_WAIT);
                if matches!(
                    outcome,
                    Ok(disk::Reset::Rebuilt { .. } | disk::Reset::Empty)
                ) {
                    // A fresh maintainer: a stopped one stays stopped otherwise.
                    // The Tools reader sees the new epoch at its next load.
                    step.maintainer = Maintainer::new(root, LIMITS, LOCK_WAIT);
                    step.restart_s5();
                    told = false;
                }
                let _ = notices.send(Wake::StatsReset {
                    generation,
                    outcome,
                });
            }
            Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => return,
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
            let _ = stop.send(Command::Stop);
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

    /// Hand the thread a reset: `false` when it was told to stop or is gone.
    /// Commands queue in order, so one handed before a stop runs first.
    fn reset(&self, generation: u64) -> bool {
        (self.stop.as_ref()).is_some_and(|stop| stop.send(Command::Reset(generation)).is_ok())
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
    /// The reset of the Statistics tab of this generation, held while no
    /// thread runs: handed, once, to the next one that does.
    reset_owed: Option<u64>,
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
            reset_owed: None,
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
        if let Some(generation) = self.reset_owed
            && (self.running.as_ref()).is_some_and(|running| running.reset(generation))
        {
            self.reset_owed = None;
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

    /// Hand the thread a reset for the Statistics tab of `generation`. With
    /// no thread to take it (paused, stopping, never started) it is held,
    /// the latest replacing an earlier one, and [`Upkeep::tend`] hands it
    /// once a thread runs again. The process ending first drops it.
    pub(super) fn reset(&mut self, generation: u64) -> Handed {
        if (self.running.as_ref()).is_some_and(|running| running.reset(generation)) {
            return Handed::Sent;
        }
        self.reset_owed = Some(generation);
        Handed::Held
    }

    /// The reset held until a thread runs, if any.
    #[cfg(test)]
    pub(super) const fn owed_reset(&self) -> Option<u64> {
        self.reset_owed
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
            reset_owed: None,
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
            reset_owed: None,
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
#[cfg(test)]
#[path = "maintainer_tools_spec.rs"]
mod tools_spec;
#[cfg(test)]
#[path = "maintainer_turn_tests.rs"]
mod turn_tests;
