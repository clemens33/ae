//! Claude Code's hold-mode dictation, driven from outside the terminal: the
//! hold circuit and the read-only settle after it.
//!
//! A hold-mode recording lasts while Claude receives Space key repeats. ae
//! sends each Space as one command and acknowledges it before the next read.
//! A Space is earned only by a fresh, styled frame that proves a recording
//! ([`Dictation::Holding`]): every other frame ends the circuit for good.
//! The time bounds below are engineering bounds, informed by measurements of
//! Claude Code 2.1.332 (docs/app.md).
//!
//! The wire and the clock are traits, so the circuit runs on a real control
//! client and, in its acceptance, on a virtual clock. The [`DispatchGate`]
//! fences each write against the owner's stop: a stop waits for a write in
//! flight and never for an acknowledgement.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::deliver::region::{self, Occupancy};
use crate::harness_state::{self, Dictation, HarnessState};
use crate::tool::ToolKind;

pub mod worker;

/// The one tool whose hold mode ae drives.
const TOOL: ToolKind = ToolKind::hold_dictation();
const _: () = assert!(
    TOOL.dictates(),
    "the hold circuit reads only a dictating tool"
);
/// A read that takes longer (from its start to its receipt) shows a stale
/// frame, whatever it shows: it ends the circuit, and it refuses the start.
const READ_BOUND: Duration = Duration::from_millis(150);
/// No Space once this long has passed since the latest Holding read began.
const HOLD_AGE: Duration = Duration::from_millis(200);
/// From a Space's acknowledgement to the next keep-alive read's start.
const INTERVAL: Duration = Duration::from_millis(50);
/// No keep-alive Space once this long has passed since the previous one's
/// dispatch (Claude ends the hold about 200 ms after the last key).
const DEADLINE: Duration = Duration::from_millis(150);
/// The first keep-alive Space comes this soon after the first burst Space's
/// dispatch (Claude's no-repeat fallback is 600 ms). Warming may still show
/// within the same window.
const FIRST_DEADLINE: Duration = Duration::from_millis(450);
/// The warmup must show in a read that returned this soon after the first
/// probe Space's dispatch.
const WARM_SEEN: Duration = Duration::from_millis(60);
/// Each burst Space is dispatched this soon after the first probe Space's
/// dispatch, and is acknowledged within [`BURST_ACK`].
const BURST_START: Duration = Duration::from_millis(90);
const BURST_ACK: Duration = Duration::from_millis(100);
/// Between two keep-alive dispatches: above the 68 ms redraw lag measured once.
const MIN_GAP: Duration = Duration::from_millis(80);
/// No Space at or after this long since the first burst Space's dispatch.
const CAP: Duration = Duration::from_mins(5);
/// The pause between two reads that wait for the warmup or the recording.
const POLL: Duration = Duration::from_millis(2);
/// How long after a missing warmup the probe's result is read.
const PROBE_LOOK: Duration = Duration::from_millis(150);
/// The settle's hard deadline and its pause between reads.
const SETTLE_DEADLINE: Duration = Duration::from_secs(15);
const SETTLE_POLL: Duration = Duration::from_millis(50);
/// The cells the probe's two Spaces advance the cursor, and the burst's Spaces.
const PROBE_CELLS: u16 = 2;
const BURST: usize = 3;

/// The line for a host where Claude's managed settings may be in force.
pub const MANAGED: &str =
    "managed Claude settings present; ae cannot prove the dictation stays unsent; no key was sent";

/// The directory Claude Code reads its managed settings files from on this
/// host (<https://code.claude.com/docs/en/managed-settings>), or `None` on a
/// system that page names no such directory for.
#[must_use]
pub fn managed_root() -> Option<&'static std::path::Path> {
    let root = if cfg!(target_os = "macos") {
        "/Library/Application Support/ClaudeCode"
    } else if cfg!(target_os = "linux") {
        "/etc/claude-code"
    } else {
        return None;
    };
    Some(std::path::Path::new(root))
}

/// Why a dictation may not start under Claude's managed settings: the host's
/// directory (`None` refuses) and, when given, one extra directory, each
/// checked alike. The panel asks before the attach, and the hold asks once
/// more before its first Space.
#[must_use]
pub fn managed_policy_refusal(
    host: Option<&std::path::Path>,
    extra: Option<&std::path::Path>,
) -> Option<&'static str> {
    managed_refusal(host).or_else(|| extra.and_then(|root| managed_refusal(Some(root))))
}

/// Why a dictation may not start under the managed settings at `root`, by
/// existence alone: no listing, no read. Managed settings outrank ae's
/// `--settings` layer and can turn its `autoSubmit: false` around. Only a
/// root where both `managed-settings.json` and its sibling `managed-settings.d`
/// are positively absent passes. A node of any kind there, one ae cannot
/// classify, or no root at all refuses with [`MANAGED`].
fn managed_refusal(root: Option<&std::path::Path>) -> Option<&'static str> {
    use crate::store::{SourcePresence, source_presence};

    let Some(root) = root else {
        return Some(MANAGED);
    };
    let absent = |name: &str| source_presence(&root.join(name)) == SourcePresence::Absent;
    (!(absent("managed-settings.json") && absent("managed-settings.d"))).then_some(MANAGED)
}

/// One capture of the orchestrator's pane and its cursor (column, row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub frame: String,
    pub cursor: Option<(u16, u16)>,
}

/// How a key command was acknowledged. Only [`Ack::Ok`] lets the circuit
/// go on; neither of the others is ever retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ack {
    Ok,
    Failed,
    Timeout,
}

/// The pane as the circuit sees it.
pub trait HoldWire {
    /// One fresh capture, or `None` when it could not be read.
    fn read(&mut self) -> Option<Reading>;
    /// Dispatch one key command; it does not wait for its acknowledgement.
    fn write(&mut self, keys: &[&str]) -> bool;
    /// The acknowledgement of the last dispatched command.
    fn ack(&mut self) -> Ack;
    /// Whether Claude's managed settings may be in force
    /// ([`managed_policy_refusal`]). The hold asks once, after the idle read
    /// that admits its start and before its first Space.
    fn managed_settings_present(&mut self) -> bool;
    /// Whether the wire can never answer again: after a timeout or an
    /// uncertain acknowledgement, its client is gone and nothing more is read.
    fn lost(&self) -> bool {
        false
    }
}

/// A monotonic clock the circuit reads and pauses on.
pub trait HoldClock {
    fn now(&self) -> Duration;
    fn pause(&mut self, gap: Duration);
}

/// The fence between the circuit's key writes and the owner's stop. A write
/// runs with the gate held, so [`DispatchGate::stop`] returns only once no
/// write is in flight, and no write starts after it.
#[derive(Debug, Default)]
pub struct DispatchGate {
    stopped: Mutex<bool>,
}

impl DispatchGate {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stopped: Mutex::new(false),
        }
    }

    /// Stop the circuit: no key is written after this returns.
    pub fn stop(&self) {
        *self.held() = true;
    }

    #[must_use]
    pub fn stopped(&self) -> bool {
        *self.held()
    }

    /// The gate, held. A gate that a panic poisoned reads stopped.
    fn held(&self) -> MutexGuard<'_, bool> {
        self.stopped.lock().unwrap_or_else(|poisoned| {
            let mut stopped = poisoned.into_inner();
            *stopped = true;
            stopped
        })
    }
}

/// One planned hold: how long to keep the recording, and its stop fence.
#[derive(Debug, Clone)]
pub struct HoldPlan {
    pub hold: Duration,
    pub gate: Arc<DispatchGate>,
}

/// Why a started hold ended, or why a start sent no burst.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lapse {
    /// The planned hold or the 300 s cap ran out.
    Planned,
    /// The owner stopped it.
    Stopped,
    /// A read did not prove a recording: another state, a torn or a missing
    /// frame, or a warmup that did not show in time.
    NotRecording,
    /// A read took longer than its bound.
    Stalled,
    /// The next Space could not be dispatched in time.
    Deadline,
    /// The latest Holding read was too old at the write.
    Aged,
    /// A key command was not acknowledged as done.
    Unacknowledged,
}

/// Why a start sent no key at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The plan was stopped before the probe.
    Stopped,
    /// The first read took longer than its bound.
    Stalled,
    /// The first read showed no empty idle Claude composer with a cursor.
    NotIdle,
    /// Claude's managed settings may be in force ([`managed_policy_refusal`]).
    ManagedSettings,
}

/// How a hold ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldOutcome {
    /// No key reached the pane.
    Refused(Refusal),
    /// The probe went out but no burst followed. `visible` is whether
    /// exactly the two probe Spaces then showed in the empty composer.
    Unstarted { why: Lapse, visible: bool },
    /// The warmup showed, so the burst began; `spaces` keep-alive Spaces
    /// followed it.
    Lapsed { why: Lapse, spaces: usize },
}

/// What the circuit and the settle report while they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// The burst went out: the recording may run.
    Holding,
    /// Claude shows the measured `Transcribing…` label.
    Transcribing,
}

/// What the settle observed after the hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle {
    /// After `Transcribing…`, two equal reads of a composer with text.
    Inserted,
    /// After `Transcribing…`, two equal reads of an empty composer.
    Empty,
    /// The settle proved neither.
    Unproved(Doubt),
}

/// Why the settle proved no result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doubt {
    /// A composer showed without the measured `Transcribing…` before it.
    NoTranscribing,
    /// A frame could not be read.
    Unreadable,
    /// The settle's deadline passed.
    Deadline,
}

/// The classes of frame the circuit tells apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    /// An empty idle composer.
    Idle,
    /// The hold-mode warmup.
    Warming,
    /// A styled frame that proves a recording.
    Holding,
    /// Anything else, a torn or a missing frame included.
    Other,
}

fn seen(reading: Option<&Reading>) -> Seen {
    let Some(reading) = reading else {
        return Seen::Other;
    };
    let frame = reading.frame.as_str();
    // Only the styled read proves the recording a Space keeps: a plain
    // capture cannot tell the interim text's colours apart.
    if harness_state::styled_dictation(frame, TOOL) == Dictation::Holding {
        return Seen::Holding;
    }
    match harness_state::dictation(frame, TOOL) {
        Dictation::Warming => Seen::Warming,
        Dictation::Clear if region::occupancy(frame, TOOL.input_model()) == Occupancy::Idle => {
            Seen::Idle
        }
        _ => Seen::Other,
    }
}

/// Whether `reading` may start a hold: an empty idle composer in a Claude
/// that the dictation start reads as idle ([`harness_state::dictation_start_state`]).
/// An empty box alone is no proof, since a busy Claude draws one under its
/// spinner too, and a frame the grammar cannot place is no idle witness.
fn startable(reading: Option<&Reading>) -> bool {
    seen(reading) == Seen::Idle
        && reading.is_some_and(|reading| {
            let plain = region::plain_text(&reading.frame);
            harness_state::dictation_start_state(&plain, TOOL) == HarnessState::Idle
        })
}

/// One read with its start (`r0`) and its receipt (`r1`) on the clock.
struct Timed {
    r0: Duration,
    r1: Duration,
    reading: Option<Reading>,
}

impl Timed {
    fn took(&self) -> Duration {
        self.r1.saturating_sub(self.r0)
    }

    fn seen(&self) -> Seen {
        seen(self.reading.as_ref())
    }
}

/// The circuit's wire, its clock and its stop gate.
struct Circuit<'a, W, C> {
    wire: &'a mut W,
    clock: &'a mut C,
    gate: &'a DispatchGate,
}

impl<W: HoldWire, C: HoldClock> Circuit<'_, W, C> {
    fn read(&mut self) -> Timed {
        let r0 = self.clock.now();
        let reading = self.wire.read();
        let r1 = self.clock.now();
        Timed { r0, r1, reading }
    }

    /// Dispatch one Space when `allowed` passes at the dispatch instant, with
    /// the gate held, then wait for its acknowledgement outside the gate.
    /// The dispatch time (K0) and the acknowledgement time (K1).
    fn space(
        &mut self,
        allowed: impl FnOnce(Duration) -> Result<(), Lapse>,
    ) -> Result<(Duration, Duration), Lapse> {
        let k0 = {
            let stopped = self.gate.held();
            if *stopped {
                return Err(Lapse::Stopped);
            }
            let k0 = self.clock.now();
            allowed(k0)?;
            if !self.wire.write(&["Space"]) {
                return Err(Lapse::Unacknowledged);
            }
            k0
        };
        let ack = self.wire.ack();
        let k1 = self.clock.now();
        match ack {
            Ack::Ok => Ok((k0, k1)),
            Ack::Failed | Ack::Timeout => Err(Lapse::Unacknowledged),
        }
    }

    fn pause_until(&mut self, at: Duration) {
        let gap = at.saturating_sub(self.clock.now());
        if !gap.is_zero() {
            self.clock.pause(gap);
        }
    }
}

/// Run one hold on `wire`: a probe of two Spaces on an empty idle composer,
/// the warmup they draw, a burst of three Spaces, then one Space per fresh
/// Holding read until the plan, a stop or any other frame ends it.
pub fn hold(
    wire: &mut impl HoldWire,
    clock: &mut impl HoldClock,
    plan: &HoldPlan,
    progress: &mut impl FnMut(Progress),
) -> HoldOutcome {
    let mut circuit = Circuit {
        wire,
        clock,
        gate: &plan.gate,
    };
    if circuit.gate.stopped() {
        return HoldOutcome::Refused(Refusal::Stopped);
    }
    let first = circuit.read();
    if first.took() > READ_BOUND {
        return HoldOutcome::Refused(Refusal::Stalled);
    }
    let cursor = first.reading.as_ref().and_then(|reading| reading.cursor);
    let (true, Some(cursor)) = (startable(first.reading.as_ref()), cursor) else {
        return HoldOutcome::Refused(Refusal::NotIdle);
    };
    // The last check before the first key: a managed setting that appeared
    // after the panel's own check still keeps every Space unsent.
    if circuit.wire.managed_settings_present() {
        return HoldOutcome::Refused(Refusal::ManagedSettings);
    }
    let (probe0, _) = match circuit.space(|_| Ok(())) {
        Ok(times) => times,
        // A stop that came during the first read wrote nothing.
        Err(Lapse::Stopped) => return HoldOutcome::Refused(Refusal::Stopped),
        Err(why) => return unstarted(&mut circuit, why, cursor),
    };
    // A first acknowledgement that used up the warmup's whole window leaves
    // no read that could still show it in time: no second Space then.
    let second = circuit.space(|k0| {
        if k0.saturating_sub(probe0) >= WARM_SEEN {
            Err(Lapse::Deadline)
        } else {
            Ok(())
        }
    });
    if let Err(why) = second {
        return unstarted(&mut circuit, why, cursor);
    }
    // The warmup must show in a read that returned in time. Only a positively
    // idle frame may be read again: any other frame, a torn or unrecognized
    // one included, is no evidence that the same composer warms up.
    let mut warm = Err(Lapse::NotRecording);
    while circuit.clock.now().saturating_sub(probe0) < WARM_SEEN {
        let read = circuit.read();
        if read.took() > READ_BOUND {
            warm = Err(Lapse::Stalled);
            break;
        }
        match read.seen() {
            Seen::Warming => {
                warm = if read.r1.saturating_sub(probe0) <= WARM_SEEN {
                    Ok(())
                } else {
                    Err(Lapse::Deadline)
                };
                break;
            }
            Seen::Idle if startable(read.reading.as_ref()) => circuit.clock.pause(POLL),
            Seen::Idle | Seen::Holding | Seen::Other => break,
        }
    }
    if let Err(why) = warm {
        return unstarted(&mut circuit, why, cursor);
    }
    let mut burst = None;
    let mut last = (probe0, probe0);
    for _ in 0..BURST {
        let sent = circuit.space(|k0| {
            if k0.saturating_sub(probe0) > BURST_START {
                Err(Lapse::Deadline)
            } else {
                Ok(())
            }
        });
        let (k0, k1) = match sent {
            Ok(times) => times,
            Err(why) => return HoldOutcome::Lapsed { why, spaces: 0 },
        };
        burst.get_or_insert(k0);
        last = (k0, k1);
        if k1.saturating_sub(probe0) > BURST_ACK {
            return HoldOutcome::Lapsed {
                why: Lapse::Deadline,
                spaces: 0,
            };
        }
    }
    let burst = burst.unwrap_or(probe0);
    progress(Progress::Holding);
    keep(&mut circuit, plan.hold, burst, last)
}

/// The keep-alive loop after the burst. `burst` is the first burst Space's
/// dispatch, `last` the last Space's dispatch and acknowledgement.
fn keep<W: HoldWire, C: HoldClock>(
    circuit: &mut Circuit<'_, W, C>,
    hold: Duration,
    burst: Duration,
    last: (Duration, Duration),
) -> HoldOutcome {
    let (mut k0, mut k1) = last;
    let end = k1.saturating_add(hold).min(burst.saturating_add(CAP));
    let mut spaces = 0;
    let mut read_end = k1;
    let lapsed = |why, spaces| HoldOutcome::Lapsed { why, spaces };
    loop {
        if circuit.clock.now() >= end {
            return lapsed(Lapse::Planned, spaces);
        }
        // The next read starts no sooner than INTERVAL after the last
        // acknowledgement, POLL after the last read, and MIN_GAP after the
        // last dispatch, so two dispatches are always MIN_GAP apart.
        circuit.pause_until((k1 + INTERVAL).max(read_end + POLL).max(k0 + MIN_GAP));
        // Re-check the budget after the pause and before any further read:
        // a read that starts too late could authorize nothing.
        let (origin, bound) = if spaces == 0 {
            (burst, FIRST_DEADLINE)
        } else {
            (k0, DEADLINE)
        };
        if circuit.clock.now().saturating_sub(origin) >= bound {
            return lapsed(Lapse::Deadline, spaces);
        }
        let read = circuit.read();
        read_end = read.r1;
        if circuit.gate.stopped() {
            return lapsed(Lapse::Stopped, spaces);
        }
        if read.took() > READ_BOUND {
            return lapsed(Lapse::Stalled, spaces);
        }
        let state = read.seen();
        // Only the measured warmup may still show before the first
        // keep-alive Space; any other frame that is not Holding ends it.
        if state == Seen::Warming
            && spaces == 0
            && circuit.clock.now().saturating_sub(burst) < FIRST_DEADLINE
        {
            continue;
        }
        if state != Seen::Holding {
            return lapsed(Lapse::NotRecording, spaces);
        }
        let witness = read.r0;
        let sent = circuit.space(|now| {
            if now.saturating_sub(origin) > bound {
                Err(Lapse::Deadline)
            } else if now >= end {
                Err(Lapse::Planned)
            } else if now.saturating_sub(witness) >= HOLD_AGE {
                Err(Lapse::Aged)
            } else {
                Ok(())
            }
        });
        match sent {
            Ok(times) => {
                spaces += 1;
                (k0, k1) = times;
            }
            // A Space whose write or acknowledgement failed may have landed:
            // it counts.
            Err(Lapse::Unacknowledged) => return lapsed(Lapse::Unacknowledged, spaces + 1),
            Err(why) => return lapsed(why, spaces),
        }
    }
}

/// The outcome of a probe that drew no burst. After an ordinary miss (no
/// warmup showed, or a stop) a short look shows whether exactly the two probe
/// Spaces landed in the empty composer. After a stalled read, a missed
/// deadline, an uncertain acknowledgement or on a lost wire nothing more is
/// read: the client is to be closed at once.
fn unstarted<W: HoldWire, C: HoldClock>(
    circuit: &mut Circuit<'_, W, C>,
    why: Lapse,
    (column, row): (u16, u16),
) -> HoldOutcome {
    if !matches!(why, Lapse::NotRecording | Lapse::Stopped) || circuit.wire.lost() {
        return HoldOutcome::Unstarted {
            why,
            visible: false,
        };
    }
    circuit.clock.pause(PROBE_LOOK);
    let look = circuit.read();
    let visible = look.seen() == Seen::Idle
        && look.reading.as_ref().and_then(|reading| reading.cursor)
            == Some((column.saturating_add(PROBE_CELLS), row));
    HoldOutcome::Unstarted { why, visible }
}

/// Read the pane after a hold until Claude has settled: the measured
/// `Transcribing…` label, then two equal reads of the composer. It writes
/// nothing. A read that returns after the deadline proves nothing, so the
/// wire's own read bound should not exceed the settle budget still left.
pub fn settle<W: HoldWire, C: HoldClock>(
    wire: &mut W,
    clock: &mut C,
    progress: &mut impl FnMut(Progress),
) -> Settle {
    let start = clock.now();
    let late = |clock: &C| clock.now().saturating_sub(start) >= SETTLE_DEADLINE;
    let mut transcribed = false;
    let mut previous: Option<String> = None;
    loop {
        if wire.lost() {
            return Settle::Unproved(Doubt::Unreadable);
        }
        if late(clock) {
            return Settle::Unproved(Doubt::Deadline);
        }
        let reading = wire.read();
        if late(clock) {
            return Settle::Unproved(Doubt::Deadline);
        }
        let Some(reading) = reading else {
            return Settle::Unproved(Doubt::Unreadable);
        };
        let frame = reading.frame.as_str();
        if harness_state::transcribing(frame) {
            if !transcribed {
                transcribed = true;
                progress(Progress::Transcribing);
            }
            previous = None;
        } else {
            match harness_state::dictation(frame, TOOL) {
                Dictation::Clear if transcribed => {
                    let Some(text) = region::content(frame, TOOL.input_model()) else {
                        return Settle::Unproved(Doubt::Unreadable);
                    };
                    if previous.as_ref() == Some(&text) {
                        return match region::occupancy(frame, TOOL.input_model()) {
                            Occupancy::Idle => Settle::Empty,
                            Occupancy::Occupied => Settle::Inserted,
                            Occupancy::Unreadable => Settle::Unproved(Doubt::Unreadable),
                        };
                    }
                    previous = Some(text);
                }
                // A composer before the measured label proves no insert: it
                // may be the owner's own text, or an insert ae missed.
                Dictation::Clear | Dictation::Tapping => {
                    return Settle::Unproved(Doubt::NoTranscribing);
                }
                Dictation::Unread => return Settle::Unproved(Doubt::Unreadable),
                // The hold's own frames, its warmup, and any lookalike that
                // still blocks every key: Claude has not settled yet.
                Dictation::Holding | Dictation::Warming | Dictation::Transcribing => {
                    previous = None;
                }
            }
        }
        clock.pause(SETTLE_POLL);
    }
}

#[cfg(test)]
#[allow(
    clippy::unchecked_time_subtraction,
    clippy::duration_suboptimal_units,
    reason = "the frozen acceptance aa41f039b subtracts trace times its own virtual clock orders, and names its cases' durations in the milliseconds they measure"
)]
#[path = "../../tests/fixtures/harness-state/d54-hold-clock.rs"]
mod clock;
