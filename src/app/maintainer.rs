//! The Statistics store's upkeep thread in `ae app`: one thread,
//! `ae-app-stats`, steps [`Maintainer`] and waits [`maint::pause`] between
//! steps on its stop channel, so a stop wakes it at once. The app keeps the
//! [`Running`] handle for as long as it runs; dropping it, on any return,
//! stops the thread and waits up to [`STOP_BOUND`] for it. A step still
//! running then is abandoned: the process ending mid-publication is a crash
//! the store's recovery finishes.
//!
//! A repair, and the stop after damage met again, reach the app as one
//! [`Wake::Notice`] each, naming the file and why.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
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
    #[cfg_attr(not(test), allow(dead_code, reason = "observed only by tests"))]
    watch: Watch,
}

/// What tests observe of the thread: nothing in the product.
struct Watch {
    #[cfg(test)]
    ended: std::sync::Weak<()>,
    #[cfg(test)]
    parked: Arc<std::sync::atomic::AtomicBool>,
}

impl Watch {
    /// The completion `token` the thread holds, and its `probe`.
    #[cfg_attr(not(test), allow(unused_variables, reason = "observed only by tests"))]
    fn of(token: &Arc<()>, probe: &Probe) -> Self {
        Self {
            #[cfg(test)]
            ended: Arc::downgrade(token),
            #[cfg(test)]
            parked: Arc::clone(&probe.parked),
        }
    }
}

/// Whether the thread waits between steps: observed only by tests.
#[derive(Clone, Default)]
struct Probe {
    #[cfg(test)]
    parked: Arc<std::sync::atomic::AtomicBool>,
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
}

/// Start the upkeep of the store under `root`, its notices sent on `notices`.
pub(super) fn start(root: PathBuf, notices: Sender<Wake>) -> std::io::Result<Running> {
    let (stop, stopped) = mpsc::channel();
    let (finished, done) = mpsc::channel();
    let token = Arc::new(());
    let probe = Probe::default();
    let watch = Watch::of(&token, &probe);
    let worker = std::thread::Builder::new()
        .name("ae-app-stats".to_owned())
        .spawn(move || {
            upkeep(&root, &stopped, &notices, &probe);
            // Completion is the token gone, before the done signal.
            drop(token);
            let _ = finished.send(());
        })?;
    Ok(Running {
        stop: Some(stop),
        done: Some(done),
        worker: Some(worker),
        watch,
    })
}

/// Step until stopped.
fn upkeep(root: &Path, stop: &Receiver<()>, notices: &Sender<Wake>, probe: &Probe) {
    let mut maintainer = Maintainer::new(root, LIMITS, LOCK_WAIT);
    let mut told = false;
    loop {
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
    pub(super) fn stop(mut self, bound: Duration) -> bool {
        self.halt(bound)
    }

    fn halt(&mut self, bound: Duration) -> bool {
        let Some(stop) = self.stop.take() else {
            return true;
        };
        let _ = stop.send(());
        drop(stop);
        let ended = self.done.take().is_some_and(|done| {
            matches!(
                done.recv_timeout(bound),
                Ok(()) | Err(RecvTimeoutError::Disconnected)
            )
        });
        if let Some(worker) = self.worker.take()
            && ended
        {
            let _ = worker.join();
        }
        ended
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.halt(STOP_BOUND);
    }
}

/// The app's one owner of the upkeep thread, when it could start one.
pub(super) struct Upkeep {
    running: Option<Running>,
}

impl Upkeep {
    /// Start the thread; an app that cannot start it runs without it.
    pub(super) fn start(root: PathBuf, notices: Sender<Wake>) -> Self {
        Self {
            running: start(root, notices).ok(),
        }
    }

    /// Stop the thread within `bound`.
    pub(super) fn stop(self, bound: Duration) {
        if let Some(running) = self.running {
            running.stop(bound);
        }
    }

    /// The thread this owner runs, if any.
    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "observed by the r6 spec's exec-restart regressions"
    )]
    pub(super) fn running(&self) -> Option<&Running> {
        self.running.as_ref()
    }
}

#[cfg(test)]
impl Running {
    /// The thread's completion, observable after the handle is gone.
    pub(super) fn ended(&self) -> Ended {
        Ended(self.watch.ended.clone())
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
}

#[cfg(test)]
#[path = "maintainer_s2_spec.rs"]
mod spec;
