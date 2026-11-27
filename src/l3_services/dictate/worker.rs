//! One dictation on a real sidecar: the hold circuit, then the read-only
//! settle on the same control client, on a thread of its own.
//!
//! A normal end (the planned hold, the owner's stop, or a frame that no
//! longer proves a recording) settles on the same client, then closes it.
//! A stalled read, a missed deadline, an aged witness or an uncertain
//! acknowledgement drops the client at once: it is killed and reaped and
//! nothing more is read. The owner that leaves drops it too.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::{
    Ack, CAP, DEADLINE, DispatchGate, Doubt, HoldClock, HoldOutcome, HoldPlan, HoldWire, Lapse,
    Progress, READ_BOUND, Reading, Refusal, SETTLE_DEADLINE, Settle,
};
use crate::inventory::ServerId;
use crate::tmux::control::{Proven, SidecarCommand};
use crate::transport::{Sidecar, SidecarLost, SidecarReply};

/// The attach's whole budget: the hook read, the spawn and the attach block.
const ATTACH_BUDGET: Duration = Duration::from_secs(2);

/// The most one settle read waits for its reply, within the settle's own
/// deadline.
const SETTLE_READ: Duration = Duration::from_secs(1);

/// What the worker tells its owner, in order; the last is always
/// [`Note::Finished`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Note {
    Progress(Progress),
    Finished(Finish),
}

/// How one dictation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    /// The sidecar did not attach, so no key was sent.
    NotAttached(SidecarLost),
    /// The circuit ran. `settled` is what the read-only settle saw, `None`
    /// when the way the hold ended allows no further read.
    Held {
        outcome: HoldOutcome,
        settled: Option<Settle>,
    },
    /// The owner left: the client was reaped and nothing was settled.
    Abandoned,
}

/// Whether a hold that ended this way may still be read on its client.
const fn settles(outcome: &HoldOutcome) -> bool {
    matches!(
        outcome,
        HoldOutcome::Lapsed {
            why: Lapse::Planned | Lapse::Stopped | Lapse::NotRecording,
            ..
        }
    )
}

/// Whether a start that sent no burst ended in an ordinary way, so its client
/// may close by itself rather than be killed: a well-formed frame that was not
/// idle, a stop, or no warmup. A stalled first read is a miss and is killed.
const fn closes(outcome: &HoldOutcome) -> bool {
    matches!(
        outcome,
        HoldOutcome::Refused(Refusal::Stopped | Refusal::NotIdle | Refusal::ManagedSettings)
            | HoldOutcome::Unstarted {
                why: Lapse::NotRecording | Lapse::Stopped,
                ..
            }
    )
}

/// Whether the client may close by itself after the circuit, rather than be
/// killed and reaped at once. Only an ordinary end on a wire still whole: a
/// settle that saw the insert, an empty composer or a well-formed frame
/// without `Transcribing…`, or an ordinary start miss with nothing to settle.
/// A settle past its deadline or with an unreadable frame, and a lost wire,
/// are killed: the lead's ruling keeps no grace for errors and misses.
const fn ends_by_close(outcome: &HoldOutcome, settled: Option<Settle>, lost: bool) -> bool {
    if lost {
        return false;
    }
    match settled {
        Some(Settle::Inserted | Settle::Empty | Settle::Unproved(Doubt::NoTranscribing)) => true,
        Some(Settle::Unproved(Doubt::Unreadable | Doubt::Deadline)) => false,
        None => closes(outcome),
    }
}

/// The circuit's wire on a sidecar. Each read is one Frame command, each key
/// one Space command; a refused or failed reply ends the sidecar.
struct SidecarWire<'a> {
    sidecar: &'a mut Sidecar,
    /// The settle's deadline, while it runs; the circuit's reads have their
    /// own bound.
    settle_until: Option<Instant>,
    left: &'a AtomicBool,
    /// The extra managed-settings directory the gate checks beside the host's.
    managed: Option<&'a std::path::Path>,
}

impl HoldWire for SidecarWire<'_> {
    fn managed_settings_present(&mut self) -> bool {
        super::managed_policy_refusal(super::managed_root(), self.managed).is_some()
    }

    fn read(&mut self) -> Option<Reading> {
        let bound = self.settle_until.map_or(READ_BOUND, |until| {
            until
                .saturating_duration_since(Instant::now())
                .min(SETTLE_READ)
        });
        if bound.is_zero() || !self.sidecar.send(SidecarCommand::Frame) {
            return None;
        }
        match self.sidecar.reply(bound) {
            Ok(SidecarReply::Frame { facts, capture }) => Some(Reading {
                frame: capture,
                cursor: Some(facts.cursor),
            }),
            Ok(SidecarReply::Spaced | SidecarReply::Refused(_)) | Err(_) => None,
        }
    }

    fn write(&mut self, keys: &[&str]) -> bool {
        keys == ["Space"] && self.sidecar.send(SidecarCommand::Space)
    }

    fn ack(&mut self) -> Ack {
        match self.sidecar.reply(DEADLINE) {
            Ok(SidecarReply::Spaced) => Ack::Ok,
            Err(SidecarLost::Timeout) => Ack::Timeout,
            Ok(SidecarReply::Frame { .. } | SidecarReply::Refused(_)) | Err(_) => Ack::Failed,
        }
    }

    fn lost(&self) -> bool {
        self.sidecar.lost() || self.left.load(Ordering::Acquire)
    }
}

/// The real clock: monotonic, from the worker's start.
struct Monotonic(Instant);

impl HoldClock for Monotonic {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }

    fn pause(&mut self, gap: Duration) {
        std::thread::sleep(gap);
    }
}

/// One running dictation, owned. Dropping it stops the circuit, skips the
/// settle and reaps the client before it returns.
#[derive(Debug)]
pub struct Worker {
    gate: Arc<DispatchGate>,
    left: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// What a proved start hands its worker.
#[derive(Debug)]
pub struct Start {
    pub server: ServerId,
    pub proven: Proven,
    /// The pane's send lock, taken before the start's proof.
    pub held: std::fs::File,
    /// The extra directory the managed-settings gate checks beside the host's
    /// (`doors::claude_managed_test_root`); `None` in every installed shape.
    pub managed: Option<std::path::PathBuf>,
}

impl Worker {
    /// Start one dictation on `start`. The worker keeps its send lock until
    /// its client is closed or reaped, so no delivery runs during the hold or
    /// the settle. `tell` hears every [`Note`] on the worker's thread, the
    /// last a [`Note::Finished`], sent once the lock is let go.
    ///
    /// # Errors
    ///
    /// The thread could not start; nothing was attached or sent.
    pub fn start(start: Start, tell: impl Fn(Note) + Send + 'static) -> std::io::Result<Self> {
        let gate = Arc::new(DispatchGate::new());
        let left = Arc::new(AtomicBool::new(false));
        let plan = HoldPlan {
            hold: CAP,
            gate: Arc::clone(&gate),
        };
        let leaving = Arc::clone(&left);
        let thread = std::thread::Builder::new()
            .name("ae-dictation".to_owned())
            .spawn(move || {
                let Start {
                    server,
                    proven,
                    held,
                    managed,
                } = start;
                let finish = dictate(&server, &proven, managed.as_deref(), &plan, &leaving, &tell);
                drop(held);
                tell(Note::Finished(finish));
            })?;
        Ok(Self {
            gate,
            left,
            thread: Some(thread),
        })
    }

    /// The owner's stop: once it returns, no Space is written. A write in
    /// flight finishes first; an acknowledgement is never waited for.
    pub fn stop(&self) {
        self.gate.stop();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.left.store(true, Ordering::Release);
        self.gate.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The worker's body: attach, hold, then settle or drop.
fn dictate(
    server: &ServerId,
    proven: &Proven,
    managed: Option<&std::path::Path>,
    plan: &HoldPlan,
    left: &AtomicBool,
    tell: &impl Fn(Note),
) -> Finish {
    let mut sidecar = match Sidecar::attach(server, proven, ATTACH_BUDGET) {
        Ok(sidecar) => sidecar,
        Err(why) => return Finish::NotAttached(why),
    };
    let mut wire = SidecarWire {
        sidecar: &mut sidecar,
        settle_until: None,
        left,
        managed,
    };
    let mut progress = |progress| tell(Note::Progress(progress));
    let outcome = super::hold(
        &mut wire,
        &mut Monotonic(Instant::now()),
        plan,
        &mut progress,
    );
    let settled = (settles(&outcome) && !left.load(Ordering::Acquire)).then(|| {
        wire.settle_until = Some(Instant::now() + SETTLE_DEADLINE);
        super::settle(&mut wire, &mut Monotonic(Instant::now()), &mut progress)
    });
    if left.load(Ordering::Acquire) {
        drop(sidecar);
        return Finish::Abandoned;
    }
    if ends_by_close(&outcome, settled, sidecar.lost()) {
        let _ = sidecar.close();
    } else {
        drop(sidecar);
    }
    Finish::Held { outcome, settled }
}

#[cfg(test)]
mod tests {
    use super::{Doubt, HoldOutcome, Lapse, Settle, closes, ends_by_close, settles};
    use crate::dictate::Refusal as Refused;

    /// Lead ruling ae-20261127T091146Z-8b30efc0: a normal stop settles on the
    /// same client; errors, misses and the owner's leaving reap at once.
    #[test]
    fn only_a_normal_end_settles_and_only_an_ordinary_miss_closes() {
        let lapsed = |why| HoldOutcome::Lapsed { why, spaces: 1 };
        let unstarted = |why| HoldOutcome::Unstarted {
            why,
            visible: false,
        };
        for why in [Lapse::Planned, Lapse::Stopped, Lapse::NotRecording] {
            assert!(settles(&lapsed(why)), "{why:?} settles");
            assert!(
                !settles(&unstarted(why)),
                "an unstarted {why:?} has nothing to settle"
            );
        }
        for why in [
            Lapse::Stalled,
            Lapse::Deadline,
            Lapse::Aged,
            Lapse::Unacknowledged,
        ] {
            assert!(!settles(&lapsed(why)), "{why:?} reaps at once");
            assert!(
                !closes(&lapsed(why)) && !closes(&unstarted(why)),
                "{why:?} is killed"
            );
        }
        for why in [Lapse::NotRecording, Lapse::Stopped] {
            assert!(closes(&unstarted(why)), "an ordinary {why:?} miss closes");
        }
        for refusal in [Refused::Stopped, Refused::NotIdle, Refused::ManagedSettings] {
            assert!(closes(&HoldOutcome::Refused(refusal)), "{refusal:?} closes");
        }
        assert!(
            !closes(&HoldOutcome::Refused(Refused::Stalled)),
            "a stalled first read is killed at once"
        );
        for refusal in [
            Refused::Stopped,
            Refused::Stalled,
            Refused::NotIdle,
            Refused::ManagedSettings,
        ] {
            assert!(!settles(&HoldOutcome::Refused(refusal)));
        }
    }

    /// d54-nav's selection: a settle past its deadline, an unreadable frame or
    /// a lost wire is killed at once; only an ordinary end on a whole wire
    /// takes the close grace.
    #[test]
    fn a_doubtful_settle_or_a_lost_wire_is_killed_and_only_an_ordinary_end_closes() {
        let stopped = HoldOutcome::Lapsed {
            why: Lapse::Stopped,
            spaces: 4,
        };
        for settle in [
            Settle::Inserted,
            Settle::Empty,
            Settle::Unproved(Doubt::NoTranscribing),
        ] {
            assert!(
                ends_by_close(&stopped, Some(settle), false),
                "{settle:?} closes"
            );
            assert!(
                !ends_by_close(&stopped, Some(settle), true),
                "{settle:?} on a lost wire"
            );
        }
        for doubt in [Doubt::Unreadable, Doubt::Deadline] {
            let settle = Some(Settle::Unproved(doubt));
            assert!(
                !ends_by_close(&stopped, settle, false),
                "{doubt:?} is killed"
            );
        }
        let missed = HoldOutcome::Unstarted {
            why: Lapse::NotRecording,
            visible: true,
        };
        assert!(ends_by_close(&missed, None, false));
        assert!(
            !ends_by_close(&missed, None, true),
            "a lost wire after a miss"
        );
        assert!(!ends_by_close(
            &HoldOutcome::Refused(Refused::Stalled),
            None,
            false
        ));
        assert!(
            !ends_by_close(&stopped, None, false),
            "a stop that did not settle"
        );
    }
}
