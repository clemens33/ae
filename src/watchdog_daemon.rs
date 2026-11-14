//! The watchdog daemon — the loop that observes a session's panes each cycle,
//! asks [`crate::watchdog`] what it is looking at, and applies the answers.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::digest::Status;
use crate::events::Event;
use crate::harness_state::HarnessState;
use crate::meta::{Meta, RecordedClient, RosterEntry, ServerSelector};
use crate::procs::{self, Descendancy};
use crate::quota::QuotaLevel;
use crate::session::Seat;
use crate::store;
use crate::theme::{self, Look, Mark};
use crate::time::Timestamp;
use crate::tmux::{self, OptionScope, StopProbe};
use crate::tracked::{self, EventFields};
use crate::transport;
use crate::watchdog::{
    DoneProgress, NUDGE_GOAL_PREFIX, NUDGE_IDLE_PREFIX, NUDGE_SENTENCE, NUDGE_TAIL,
    NUDGE_WAIT_OPEN, NUDGE_WAITING_CLOSE, NUDGE_WAITING_OPEN, QuietKind, SweepAlert, SweepEffect,
    SweepKnobs, SweepObservation, SweepState, SweepVerdict, Throttle, WaitProgress, WaitState,
    classify_dead, declaration_key, is_sweep_target, latest_relevant_event, quiet_hash,
    quiet_reason, record_sweep, stale_composite, sweep_step, throttle_class,
};

/// The event actor every watchdog emission carries.
const ACTOR: &str = crate::watchdog::WATCHDOG_ACTOR;

/// The panes that are not agents: unstamped, tmux's own null, this daemon, the
/// events pane, and the two older names a session can still carry.
const NON_AGENT_PANES: [&str; 5] = ["(null)", "_watchdog", "_events", "_shepherd", "_loop"];

/// The bound on consecutive unusable process snapshots before the daemon says
/// so.
const UNKNOWN_ALERT_CYCLES: u32 = 5;

/// Launch-settle grace in cycles: 120s at the 60s default, 20x over the
/// spawn overhead and tool startup it covers.
const LAUNCH_GRACE_CYCLES: u64 = 2;

/// Whether the launch still settles: a young `.launch-attempt` means a seat
/// starting. Absent, damaged, future or aged out all judge.
#[must_use]
fn launch_grace(stamp: crate::tmux::Evidence, now_epoch: i64, interval_secs: u64) -> bool {
    let crate::tmux::Evidence::At(at) = stamp else {
        return false;
    };
    let bound = i64::try_from(LAUNCH_GRACE_CYCLES.saturating_mul(interval_secs.max(1))).ok();
    bound.is_some_and(|bound| at <= now_epoch && now_epoch - at < bound)
}

/// Motion cadence while a client can see a working surface: 5 fps.
const ATTACHED_MOTION_TICK: Duration = Duration::from_millis(200);

/// The pulse phases one frame advances: every second phase of the 20-phase
/// [`theme::working_frame`] pulse, so 5 fps keeps its two-second period and
/// still reaches both the dim and the bright end.
const FRAME_PHASES: u64 = 2;

/// The attached observation cadence when no frame carries a read: as often as
/// a client can see the session, and no more.
const ATTACHED_OBSERVATION: Duration = Duration::from_millis(500);

/// Motion cadence while nobody can see the session.
const DETACHED_MOTION_TICK: Duration = Duration::from_secs(2);

/// The least time between two waiters of one interval: a stale signal, which
/// returns a waiter at once, costs at most what polling did.
const WAKE_REARM: Duration = DETACHED_MOTION_TICK;

/// Consecutive ticker failures before the daemon preserves the rest of the
/// verdict interval without further motion work.
const MOTION_FAILURE_LIMIT: u8 = 3;

/// The tunables, all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Knobs {
    /// Seconds slept at the end of every cycle.
    pub interval_secs: u64,
    /// Seconds between bounded local quota observations; zero disables them.
    pub quota_every_secs: u64,
    /// Whether ae acts on vendor quota at all: `[workspace] quota`, absent
    /// means ON. Re-resolved EVERY cycle in `watch` through the ONE
    /// `config::resolve_quota_aware` precedence — never the raw string, never
    /// a second grammar, never a startup value. OFF wins over
    /// `quota_every_secs`: a cadence is meaningless when the feature is off.
    /// Usage machinery (`ae usage`) is NOT gated by this.
    pub quota_aware: bool,
    /// Seconds of continuously observed idle before the state reminder; zero disables it.
    pub idle_nudge_secs: u64,
    /// Later done declarations required after verified challenges.
    pub done_confirmations: u8,
    /// The window under which a pane change or an event counts as recent.
    pub stale_secs: u64,
    /// How many nudges may be DELIVERED before the alert replaces them.
    pub max_nudges: u32,
    /// Consecutive throttled cycles before the persistent-throttle alert.
    pub throttle_alert_cycles: u32,
    /// Consecutive undelivered nudges before attempts stop and one alert fires.
    pub undelivered_max: u32,
    /// Consecutive cycles a human-only prompt must hold before it is NAMED.
    /// Two: one cycle is a redraw, two is a seat that is actually stuck.
    pub human_prompt_cycles: u32,
    /// `--quiet-beat-ms`, `--quiet-tries` and `--quiet-panes-per-cycle`:
    /// accepted, unread since #115 retired the pane-churn yield they tuned.
    pub quiet_beat_ms: u64,
    pub quiet_tries: usize,
    pub quiet_panes_per_cycle: usize,
    /// The orchestrator changed-overview spacing, retry and bound.
    pub sweep: SweepKnobs,
    /// Seconds between best-effort Telegram bridge revives.
    pub tg_supervise_secs: u64,
}

/// Seconds between two verdict cycles when no `--interval` says otherwise.
pub(crate) const DEFAULT_INTERVAL_SECS: u64 = 60;

impl Default for Knobs {
    fn default() -> Self {
        Self {
            interval_secs: DEFAULT_INTERVAL_SECS,
            quota_every_secs: 300,
            quota_aware: true,
            idle_nudge_secs: crate::watchdog::DEFAULT_IDLE_NUDGE_SECS,
            done_confirmations: crate::watchdog::DEFAULT_DONE_CONFIRMATIONS,
            stale_secs: 900,
            max_nudges: 2,
            throttle_alert_cycles: 5,
            undelivered_max: 3,
            human_prompt_cycles: 2,
            quiet_beat_ms: 1000,
            quiet_tries: 4,
            quiet_panes_per_cycle: 2,
            sweep: SweepKnobs::default(),
            tg_supervise_secs: 120,
        }
    }
}

/// What one seat's own frame proved about the model it is running.
///
/// DISPLAY only: nothing here reaches the meta. The durable drift rows keep
/// their own observer, [`crate::model_drift`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeatIdentity {
    /// The model label the frame drew.
    pub model: Option<String>,
    /// The effort that frame drew, which no other surface records.
    pub effort: Option<String>,
    /// Whether that model differs from the profile's own pin. Only ever true
    /// for a tool whose live model ae observes today.
    pub drift: bool,
}

/// What one live pane contributed to this cycle's roster fact.
struct AgentObservation {
    slot: String,
    pane: String,
    verdict: SeatVerdict,
    /// What its own frame proved about the model it is running.
    identity: SeatIdentity,
}

/// One pane's inputs to [`Cycle::resolve_identity`], gathered so the call
/// stays one question.
struct ResolveIdentity<'a> {
    capture: &'a str,
    tool: crate::tool::ToolKind,
    slot: &'a str,
    /// The profile's model pin, from [`Cycle::seat_pin`].
    pin: Option<&'a str>,
    verdict: Verdict,
}

/// The newest identity a pane proved, carried while its frame is unreadable.
///
/// A turn in flight, a human's draft and a failed capture all hide the
/// composer; without this the model cell would flap to the declared profile
/// and back every time a seat got busy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityHold {
    /// The `launch_id.<slot>` current when this was observed. A retire plus a
    /// respawn can reuse both slot and agent name, so the pane-identity reset
    /// is not on its own enough to prove the conversation is the same one.
    pub launch: String,
    /// Cycles since the observation: `0` on the cycle that made it.
    pub age: u32,
    /// What that frame proved.
    pub identity: SeatIdentity,
}

/// How many cycles an identity may be shown after the last frame that proved
/// it.
///
/// At the default 60-second cadence, half an hour: long enough to cover a long
/// turn or a draft left sitting in the box, short enough that a label nobody
/// can re-confirm stops being asserted. Counted in CYCLES because the cadence
/// is the daemon's own knob.
const HOLD_MAX_CYCLES: u32 = 30;

/// What one pane carries from cycle to cycle, gathered into one value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneState {
    /// The slot+agent generation this carry belongs to.
    pub identity: Option<u64>,
    /// The newest identity this pane proved, while it is still worth showing.
    ///
    /// Memory only, like every other field here: a restart simply observes
    /// again. `account`'s own identity reset drops it with the rest of the
    /// carry when the seat changes underneath a reused pane id.
    pub held_identity: Option<IdentityHold>,
    /// Last successful capture's clock, decoded state and launch, never a quota clock.
    pub history_capture: Option<(i64, String, Option<String>)>,
    /// Dead is LATCHED once alerted, and the latch ends exactly once: on a
    /// POSITIVE process reading that shows the seat's harness back under the
    /// pane (a re-run in place), or on the seat's own later journal activity
    /// — which alerts nothing further and clears it with one `dead-cleared`.
    /// A probe gap is not evidence of life, so an UNKNOWN snapshot keeps the
    /// latch. A seat that dies again after a clear is alerted again.
    pub dead_latched: bool,
    /// The previous cycle's filtered pane hash; `None` before the first.
    pub prev_hash: Option<u64>,
    /// When the hash last changed, in epoch seconds; `None` if it never has.
    pub last_hash_change: Option<i64>,
    /// First positive idle observation in the current uninterrupted episode.
    pub idle_since_epoch: Option<i64>,
    /// Fingerprint of the newest declaration that already reset that episode.
    pub last_declaration: Option<u64>,
    /// DELIVERIES, never attempts.
    pub nudge_count: u32,
    /// Consecutive throttled cycles.
    pub throttle_streak: u32,
    /// The usage-limit latch, as the epoch of the episode's FIRST sight — the
    /// instant of its one `limit` record, which a quiet declaration is judged
    /// against. Its ONE release is a cycle judged at all that no longer shows
    /// the notice: it retracts the verdict, requests recovery.
    pub limit_since: Option<i64>,
    /// The latched episode's binding reset, as the newest fresh reading named
    /// it. Carried because the reading itself goes unknown the moment the
    /// reset passes; a cycle with no reading never clears it, the release and
    /// a death's clear do.
    pub limit_reset: Option<crate::autoreseat::LimitReset>,
    /// The reset instant whose wake was delivered (or left unconfirmed): it is
    /// never woken again. It survives the release, so a re-hit on the SAME
    /// reset starts no second wake.
    pub limit_woken: Option<i64>,
    /// Refused wakes for the carried reset, bounded by `undelivered_max`; a new
    /// reset starts its own count.
    pub limit_wake_attempts: u32,
    /// The reset the tool's OWN resume was seen armed for. The vendor owns that
    /// reset: ae never wakes it, even after the footer goes when the vendor
    /// fires. Only a footer read gone BEFORE the reset — the human cancelled —
    /// hands it back.
    pub limit_armed: Option<i64>,
    /// Consecutive cycles showing a human-only prompt. It lives HERE, in the
    /// carry, so a dead seat's reset clears it for free and a restart begins
    /// again — exactly like the two latches above it. There is NO separate
    /// latch flag: the streak IS the latch, reaching the bound exactly once,
    /// which is how `throttle_streak` already names a persistent throttle.
    pub human_prompt_streak: u32,
    /// The tool start the prompt was last seen on: a different proven start
    /// is a new episode, named again.
    pub prompt_start: Option<String>,
    /// Consecutive nudges that did not land.
    pub undelivered_streak: u32,
    /// Consecutive cycles whose process snapshot was unusable.
    pub unknown_streak: u32,
    /// The persistent-unknown alert is raised once per streak, not per cycle.
    pub unknown_alerted: bool,
    /// The newest input a client viewing this pane gave, as `(identity,
    /// epoch)`: kept across cycles so a detach does not erase it, and keyed by
    /// the seat identity so a pane id reused by another seat starts empty.
    pub client_activity: Option<(u64, u64)>,
    /// The orchestrator sweep branch's carry.
    pub sweep: SweepState,
    /// The pane half of "nothing changed since the seat's last proof" (#225),
    /// [`proof_frame_step`]'s carry. `None` is NO knowledge — a fresh carry or
    /// a restart — which never proves an unchanged frame.
    pub proof_frame: Option<ProofFrame>,
    /// The first cycle of the unbroken run in which the seat's tool was seen
    /// running a task ([`task_lease`], #283). Memory only: a restart grants
    /// at most one more cap.
    pub task_since: Option<i64>,
}

/// What the cycle knows BESIDE its [`Observation`] (#225, #283): carried as a
/// separate value so the observation's shape — built literally by many specs
/// — does not change. [`Activity::default`] is exactly today's accounting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Activity {
    /// [`crate::procs::task_running`] for the seat this cycle.
    pub task_running: Option<bool>,
    /// The TRACKED quiet state's proof reading (done or wait). `None` leaves
    /// the proof frame carry untouched and holds nothing.
    pub proof: Option<crate::watchdog::ProofQuiet>,
    /// The watchdog's newest painting delivery to the seat, epoch seconds.
    pub painting: Option<i64>,
}

/// The pane frame of one proof (#225): when it settled and whether it moved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProofFrame {
    /// Epoch of the proof this frame belongs to.
    pub proof: Option<i64>,
    /// The settle point: newest of the proof, information and the
    /// watchdog's own painting delivery.
    pub settle: Option<i64>,
    /// The frame hash taken one interval after the settle point.
    pub baseline: Option<u64>,
    /// A baseline has been taken at least once for this proof.
    pub settled: bool,
    /// Motion seen after a baseline. STICKY for the proof: only a new proof,
    /// a new incarnation or a restart forgets it.
    pub moved: bool,
}

/// What [`proof_frame_step`] proves about the pane since the proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameVerdict {
    /// Motion since the proof, or no way to know.
    Moved,
    /// No baseline to compare yet.
    Settling,
    /// No baseline to compare yet, but this proof had a clean one and only
    /// the settle point moved since (#315): information retakes the
    /// baseline, so the quiet it already proved still holds.
    Resettling,
    /// This cycle's read matches the baseline and nothing moved before it.
    Unmoved,
}

/// One cycle's pane reading for [`proof_frame_step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSample {
    /// The capture succeeded.
    pub capture_ok: bool,
    /// The harness frame is positively Busy.
    pub busy: bool,
    /// [`crate::watchdog::quiet_hash`] of the capture.
    pub hash: u64,
}

/// One step of the proof frame (#225). Rules, in order: no proof → a known
/// empty frame; no prior knowledge → MOVED (a restart cannot know what
/// happened since an old proof); a new proof → a fresh frame; a moved settle
/// point → retake the baseline, keeping `moved` and `settled`. Then, on a
/// successful read only: Busy past the settle window moves; the first non-Busy
/// read past it takes the baseline; a hash off the baseline moves. While a
/// once-settled, unmoved frame waits for its retaken baseline it reads
/// Resettling (#315), never a first Settling. A failed read changes nothing
/// and never proves Unmoved or Resettling.
#[must_use]
pub fn proof_frame_step(
    prior: Option<ProofFrame>,
    proof: Option<i64>,
    settle: Option<i64>,
    sample: FrameSample,
    now: i64,
    interval_secs: u64,
) -> (Option<ProofFrame>, FrameVerdict) {
    if proof.is_none() {
        return (Some(ProofFrame::default()), FrameVerdict::Moved);
    }
    let mut frame = match prior {
        None => ProofFrame {
            proof,
            settle,
            moved: true,
            ..ProofFrame::default()
        },
        // Two proofs in the same second share an epoch and so a frame. That
        // reuse is harmless: a baseline is taken only an interval after the
        // proof, so within that second the frame holds none, and `moved` can
        // only be the conservative restart value.
        Some(held) if held.proof != proof => ProofFrame {
            proof,
            settle,
            ..ProofFrame::default()
        },
        Some(held) if held.settle != settle => ProofFrame {
            settle,
            baseline: None,
            ..held
        },
        Some(held) => held,
    };
    if !sample.capture_ok {
        let frame = prior.filter(|held| held.proof == proof).unwrap_or(frame);
        let verdict = if frame.moved {
            FrameVerdict::Moved
        } else {
            FrameVerdict::Settling
        };
        return (Some(frame), verdict);
    }
    let settled_at = frame.settle.or(frame.proof).map_or(now, |at| {
        at.saturating_add(i64::try_from(interval_secs).unwrap_or(i64::MAX))
    });
    let past = now >= settled_at;
    if sample.busy && past {
        frame.moved = true;
    }
    match frame.baseline {
        None if past && !sample.busy => {
            frame.baseline = Some(sample.hash);
            frame.settled = true;
        }
        Some(baseline) if baseline != sample.hash => frame.moved = true,
        _ => {}
    }
    let verdict = if frame.moved {
        FrameVerdict::Moved
    } else if frame.baseline.is_some() {
        FrameVerdict::Unmoved
    } else if frame.settled {
        FrameVerdict::Resettling
    } else {
        FrameVerdict::Settling
    };
    (Some(frame), verdict)
}

/// Whether the pane half proves the frame unchanged since the proof (#225):
/// a read matching the baseline, or a clean frame whose baseline only
/// information retook (#315). Settling and Moved never do.
const fn frame_unchanged(frame: FrameVerdict) -> bool {
    matches!(frame, FrameVerdict::Unmoved | FrameVerdict::Resettling)
}

/// Whether an UNCHANGED wait holds its due challenge this cycle (#225, D1):
/// no actionable record since the proof, a frame proven unmoved, and the
/// proof younger than [`crate::watchdog::quiet_ceiling`].
#[must_use]
pub fn wait_held(
    progress: WaitProgress,
    proof: crate::watchdog::ProofQuiet,
    frame: FrameVerdict,
    cadence: u64,
) -> bool {
    matches!(
        progress,
        WaitProgress::ChallengeDue { wait_age_secs, .. }
            if wait_age_secs < crate::watchdog::quiet_ceiling(cadence)
    ) && proof.unchanged
        && frame_unchanged(frame)
}

/// Whether the due done challenge is the LAST confirmation of an unchanged
/// proof after a real answered one, so `done-held` is journaled instead
/// (#225, D2/P3). The first challenge is always asked.
#[must_use]
pub fn done_held_due(
    progress: DoneProgress,
    proof: crate::watchdog::ProofQuiet,
    frame: FrameVerdict,
) -> bool {
    matches!(
        progress,
        DoneProgress::ChallengeDue { confirmations, required, .. }
            if confirmations >= 1 && confirmations.saturating_add(1) == required
    ) && proof.unchanged
        && frame_unchanged(frame)
}

/// The task lease's start (#283): a running task keeps the first sighting, a
/// positively absent one clears it, an unknown reading keeps it unrenewed.
#[must_use]
pub const fn task_lease(prior: Option<i64>, running: Option<bool>, now: i64) -> Option<i64> {
    match running {
        Some(true) => match prior {
            Some(since) => Some(since),
            None => Some(now),
        },
        Some(false) => None,
        None => prior,
    }
}

/// Whether a live task defers the idle wording this cycle (#283): running
/// now, and its lease younger than [`crate::watchdog::CHILD_ACTIVITY_CAP`]
/// cadences. Zero `idle_nudge_secs` has no nudge to defer.
#[must_use]
pub fn task_deferred(
    task_since: Option<i64>,
    running: Option<bool>,
    now: i64,
    knobs: &Knobs,
) -> bool {
    if running != Some(true) || knobs.idle_nudge_secs == 0 {
        return false;
    }
    let cap = knobs
        .idle_nudge_secs
        .saturating_mul(crate::watchdog::CHILD_ACTIVITY_CAP);
    task_since.is_some_and(|since| age_secs(now, since) < cap)
}

/// What the cycle saw about one pane, after [`crate::watchdog`]'s pure
/// classifiers ran over it.
#[derive(Debug, Clone)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent per-cycle readings, each judged by its own ladder step"
)]
pub struct Observation {
    /// Wall clock for this cycle, epoch seconds.
    pub now_epoch: i64,
    /// The filtered pane hash.
    pub hash: u64,
    /// Facts derived from this cycle's existing harness capture.
    pub harness: HarnessObservation,
    /// Stable hash of this pane's slot+agent identity.
    pub identity: u64,
    /// [`classify_dead`]'s answer.
    pub is_dead: bool,
    /// [`crate::watchdog::throttle_class`]'s answer — which trouble, if any.
    pub throttle: Option<Throttle>,
    /// [`crate::watchdog::limit_notice`]'s answer: the vendor's own limit
    /// cell, which holds the reset time the named notice quotes.
    pub limit_notice: Option<String>,
    /// The seat's own binding reset as this cycle's held quota observation
    /// reads it ([`crate::autoreseat::leg::own_reset`]), read only while the
    /// seat is on its limit; `None` when unknown or not read.
    pub limit_reset: Option<crate::autoreseat::LimitReset>,
    /// The reset epoch of this seat's newest delivered or unconfirmed wake the
    /// journal holds ([`limit_woke_journal`]): what a restarted daemon, with
    /// no carry, must not wake again.
    pub limit_woke_journal: Option<i64>,
    /// The tool's own resume is armed on this cycle's frame
    /// ([`crate::harness_state::auto_continue_armed`]); `false` on a failed
    /// capture, which the latch never reads as a cancel.
    pub auto_continue: bool,
    /// Whether THIS cycle's pane capture SUCCEEDED. A failed read is an
    /// absence of evidence: the usage-limit latch may not clear on it, exactly
    /// as the dead latch may not.
    pub capture_ok: bool,
    /// [`crate::watchdog::human_prompt_class`]'s answer for THIS cycle's
    /// capture: a prompt only the human may answer, and what to press.
    pub human_prompt: Option<crate::watchdog::HumanPrompt>,
    /// The worst exact-match row from the last scheduled quota observation.
    pub throttle_quota: Option<String>,
    /// The RESOLVED quiet suppression: `Done` always, `WaitingUser`/`Blocked`
    /// only while their baseline holds.
    pub quiet: Option<QuietKind>,
    /// The wait THIS cycle ended on the human's input in the pane, if any —
    /// the fact the nudge that follows names, from the SAME resolution.
    pub ended_wait: Option<WaitEnded>,
    /// The shared journal-derived done episode verdict.
    pub done_progress: DoneProgress,
    /// Wait episode verdict for the seat's CURRENT wait declaration; `None`
    /// for every other state, including a pane-yielded wait.
    pub wait_progress: WaitProgress,
    /// The wait episode's backoff rung ([`crate::watchdog::WaitReading`]):
    /// it sets the `waiting-agent` ceiling; 0 for every other state.
    pub wait_rung: u8,
    /// The seat's typed wait chain ends in another session
    /// ([`crate::wait_on::ends_elsewhere`]): its ceiling does not escalate.
    pub wait_ends_elsewhere: bool,
    /// What the seat's `--on` target did since the wait began, raw:
    /// [`account`] decides whether it is owed ([`crate::wait_on::moved`]).
    pub target_moved: Option<crate::wait_on::Moved>,
    /// Whether a process named the agent binary runs under the pane.
    pub descendancy: Descendancy,
    /// This cycle's `.launch-attempt` stamp; the grace rule judges it.
    pub launch_attempt: crate::tmux::Evidence,
    /// The alert the log still shows this seat, if any.
    pub outstanding_alert: Option<crate::attention::Reason>,
    /// The seat's one unreachable failure stands in the log
    /// ([`crate::session::unreachable_standing`]).
    pub unreachable: bool,
    /// That failure was booked against another proven tool start, and so
    /// ended with it.
    pub failed_elsewhere: bool,
    /// A prompt's naming stands in the log that no clear has followed.
    pub prompt_named: bool,
    /// The tool start the cycle's process table shows under the pane
    /// ([`crate::autotrust::start_of`]): what a panel's naming and an
    /// unreachable failure are keyed to.
    pub start: Option<String>,
    /// Age of the newest event this agent is the ACTOR of.
    pub last_actor_event_age_secs: u64,
    /// AWAKE age of this seat's newest declaration
    /// ([`declaration_age`]): what a `waiting-agent` ceiling measures,
    /// the same age `ae list` reads.
    pub declared_age_secs: u64,
    /// The journal's host suspensions (I15): slept seconds the idle clock and
    /// the limit-versus-declaration tie do not count.
    pub suspended: crate::watchdog::Suspensions,
    /// The orchestrator sweep reading, `Some` ONLY for the orchestrator main
    /// agent with the cadence enabled.
    pub sweep: Option<SweepObservation>,
    /// What this seat is still owed by somebody else: requests it SENT that
    /// nobody answered, agents it SPAWNED that still hold a seat.
    pub own_work: crate::session::OwnWork,
    /// An auto reseat attempt runs for this seat inside its bound, and what
    /// opened it: the shell its respawn leaves is the move, not a death — and,
    /// for a latched seat's limit move, not a release either. A headroom move
    /// latches no limit.
    pub auto_in_flight: Option<crate::autoreseat::Trigger>,
    /// Where and when auto reseat will move this seat, for the notice of its
    /// limit's first sight; `None` when auto reseat may not move it.
    pub auto_deadline: Option<String>,
}

/// Related facts derived without another pane capture or transcript read.
#[derive(Debug, Clone, Copy)]
pub struct HarnessObservation {
    /// Positive harness-frame recognition.
    pub frame: HarnessState,
    /// A modeled current input box contains human-authored draft text.
    pub human_draft: bool,
    /// A prior max-nudges stale alert still stands in the durable event log.
    pub durable_stale: bool,
    /// Stable fingerprint of the latest relevant self-declaration, if current.
    pub declaration: Option<u64>,
}

/// The roster glyph a pane earned this cycle — derived only from branches that
/// were actually judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The agent's process is gone, or its pane is.
    Dead,
    /// A declared quiet state.
    Quiet(QuietKind),
    /// Upstream is rate-limiting this agent.
    Throttled,
    /// The vendor's own usage limit: waits on a reset or a re-login.
    Limit,
    /// A prompt only the HUMAN may answer is on screen. ae never answers it;
    /// naming it IS the whole feature.
    HumanPrompt,
    /// The modeled harness is positively waiting at an empty input box.
    Idle,
    /// Silent past the window, with nothing recent anywhere.
    Stale,
    /// Proof challenges stopped reaching the seat: its one unreachable failure
    /// stands, and it outranks the declaration it would hide.
    Unreachable,
    /// Moving, recently moved, or recently active in the log.
    Active,
    /// The orchestrator main, judged by its own cadence rather than by silence.
    Meta(SweepVerdict),
}

/// A cycle verdict with the seat tool's proven job beside it: the ONE carrier
/// every published mark reads — pane border, window entry, ticker, session and
/// slot rollups, the roster fact — so no surface can draw the bare verdict and
/// disagree with another. The word stays the verdict's, which keeps the
/// declaration readable beside the [`Mark::Background`] glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SeatVerdict {
    verdict: Verdict,
    /// [`crate::procs::task_running`] proved a live job of the seat's tool this
    /// cycle. An unknown reading is `false`: a display claim needs proof.
    task: bool,
}

impl SeatVerdict {
    /// The mark this seat draws.
    const fn mark(self) -> Mark {
        mark_with_task(self.verdict, Some(self.task))
    }

    /// The word the pane border prints after the glyph.
    const fn reason(self) -> &'static str {
        self.verdict.reason()
    }
}

impl From<Verdict> for SeatVerdict {
    fn from(verdict: Verdict) -> Self {
        Self {
            verdict,
            task: false,
        }
    }
}

/// The mark `verdict` draws when the seat tool's job reading is `task`:
/// [`Mark::with_task`] over the verdict's own mark, and only a proven job
/// (`Some(true)`) composes.
#[must_use]
pub const fn mark_with_task(verdict: Verdict, task: Option<bool>) -> Mark {
    verdict.mark().with_task(matches!(task, Some(true)))
}

impl Verdict {
    /// The theme mark this verdict is drawn as.
    ///
    /// SEVEN marks for twelve verdicts: the accent and the reason word beside
    /// it carry the difference, and a status bar that spent a distinct glyph
    /// on each verdict asked its reader to learn a private alphabet. A gone
    /// process keeps its own mark, because "this will never move again" is not
    /// the same news as "this is waiting for you".
    #[must_use]
    pub const fn mark(self) -> Mark {
        match self {
            Self::Dead => Mark::Dead,
            Self::Quiet(QuietKind::WaitingUser | QuietKind::Blocked)
            | Self::Throttled
            | Self::Limit
            | Self::HumanPrompt
            | Self::Meta(SweepVerdict::MetaWedged) => Mark::NeedsYou,
            // A FRESH `waiting-agent` is quiet but no longer borrows Working's
            // mark: it draws the seventh glyph, statically — the ticker below
            // only repaints Working marks. An escalated one arrives here as
            // `Quiet(Blocked)` and draws NeedsYou above.
            Self::Quiet(QuietKind::WaitingAgent) => Mark::WaitingAgent,
            Self::Active | Self::Meta(SweepVerdict::MetaSweeping) => Mark::Working,
            Self::Quiet(QuietKind::Done) => Mark::Done,
            Self::Idle => Mark::Idle,
            Self::Stale | Self::Unreachable | Self::Meta(SweepVerdict::MetaStarting) => Mark::Stale,
        }
    }

    /// The word the pane border prints after the glyph.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Dead => "dead",
            Self::Quiet(QuietKind::Done) => "done",
            Self::Quiet(QuietKind::WaitingUser) => "waiting-user",
            Self::Quiet(QuietKind::WaitingAgent) => "waiting-agent",
            Self::Quiet(QuietKind::Blocked) => "blocked",
            Self::Throttled => "throttled",
            Self::Limit => "limit",
            Self::HumanPrompt => "prompt",
            Self::Idle => "idle",
            Self::Stale => "stale",
            Self::Unreachable => "unreachable",
            Self::Active => "working",
            Self::Meta(SweepVerdict::MetaSweeping) => "sweeping",
            Self::Meta(SweepVerdict::MetaWedged) => "wedged",
            Self::Meta(SweepVerdict::MetaStarting) => "starting",
        }
    }

    /// The glyph the roster bar publishes for this verdict.
    #[must_use]
    pub const fn glyph(self, icons: bool) -> &'static str {
        self.mark().glyph(icons)
    }
}

/// Something the loop must DO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Append one event for this agent.
    Emit {
        /// `alert` / `throttled` / `throttle-cleared` / `limit` / `alert-cleared`.
        action: &'static str,
        /// The event summary.
        summary: String,
    },
    /// Deliver one nudge through the session's own send helper.
    Nudge,
    /// Deliver proof challenge without consuming the ordinary nudge budget.
    DoneChallenge {
        confirmations: u8,
        required: u8,
        done_age_secs: u64,
    },
    /// Journal `done-held` instead of the LAST done challenge (#225): an
    /// unchanged proof fills it. Paints nothing.
    DoneHeld { confirmations: u8, required: u8 },
    /// Deliver a wait proof challenge for the RAW declared `state`;
    /// `escalated` is [`waiting_agent_escalates`]'s answer for this cycle.
    WaitChallenge {
        confirmations: u8,
        required: u8,
        wait_age_secs: u64,
        state: WaitState,
        escalated: bool,
    },
    /// Tell a `waiting-agent` seat its `--on` target moved: one paste per
    /// transition, booked under `moved.ordinal`.
    TargetMoved {
        moved: crate::wait_on::Moved,
        age_secs: u64,
    },
    /// A line for the human, published with `display-message`.
    Notify(String),
    /// ONE quota pass outside the cadence, on the cycle a seat left the usage
    /// limit. `run` collects it and performs it once; `quota = off` runs none.
    QuotaRefresh,
    /// Deliver the ONE continuation turn a limited seat is owed once its
    /// known `reset` has passed, through the guarded send like a nudge.
    LimitWake { reset: i64 },
    /// Deliver one SWEEP prompt to the orchestrator.
    SweepNudge,
    /// Reconcile the durable event log against a wedge alert this daemon does
    /// not remember raising — the post-restart clear.
    ReconcileWedge,
    /// Answer the trust block on screen through [`crate::deliver::answer_trust`];
    /// `name` is whether the prompt is still to be named, which an answer that
    /// does not land then does exactly as today.
    AnswerTrust { name: bool },
    /// One line on the watchdog's own stderr.
    Note(String),
}

/// Labels are not identity: the canonical source and vendor dimensions are.
///
/// A window belongs to the CLIENT SCOPE, not to the conversation that happened
/// to observe it, so the rollout is deliberately absent: several rollouts under
/// one config home report one fact and must produce one advisory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct QuotaKey {
    source: std::path::PathBuf,
    bucket: String,
    qualifier: Option<String>,
    window_minutes: Option<u32>,
}

#[derive(Debug, Clone)]
struct QuotaSample<'a> {
    key: QuotaKey,
    /// The scope this observation was read from. Its LABELS are all this
    /// daemon takes from it; the numbers and the policy live on the reading.
    group: &'a crate::quota::Group,
    /// The observation and the policy it was judged under, as one value.
    reading: crate::quota::Reading,
}

#[derive(Debug, Clone, PartialEq)]
struct QuotaTracked {
    key: QuotaKey,
    /// The classified observation: the level, the row it was decided from, the
    /// policy it was judged under, and the provenance of both.
    ///
    /// ONE value, because a level and the numbers behind it must not be able to
    /// drift apart. Every consumer — the next classification, the notice it
    /// books, the throttle line a seat reads — takes all of them from here, and
    /// `Classified::adopt` is the only thing that moves any of them.
    classified: crate::quota::Classified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QuotaRecipient {
    slot: String,
    agent: String,
    harness_session: Option<String>,
    config_home: crate::meta::RecordedConfigHome,
    config_home_base: crate::meta::RecordedConfigHomeBase,
    binary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingAdvisory {
    key: QuotaKey,
    observed_at: i64,
    level: QuotaLevel,
    recipient: QuotaRecipient,
    advisory: crate::quota::Advisory,
    attempts: u8,
}

/// One booked CHECKPOINT ASK, waiting for its seat.
///
/// Keyed by scope key AND seat slot, because the fan-out is per seat: two seats
/// on one scope hold two of these, and each retries on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingAsk {
    key: QuotaKey,
    /// The stamp of the HELD observation the level was decided from.
    observed_at: i64,
    recipient: QuotaRecipient,
    /// Built by [`crate::quota::Observation::transition`] from the classified
    /// reading that decided the level, so the text cannot quote another
    /// sample's numbers.
    advisory: crate::quota::Advisory,
    attempts: u8,
}

/// A roster seat PROVEN to sit on one client scope, with a live pane to speak
/// into.
///
/// The identity is [`crate::quota::recorded_identity`]'s — the one owner of the
/// tool-kind-plus-canonical-config-home join, which `throttle_quota_line`
/// already matches a seat by. Nothing here re-derives a source path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct QuotaAskCandidate {
    recipient: QuotaRecipient,
    identity: crate::quota::RecordedIdentity,
    /// The model the seat's own frame last proved, from [`live_model`];
    /// `None` when nothing current proves one.
    live: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum QuotaAction {
    Deliver(Box<PendingAdvisory>),
    /// Deliver one advisory-only checkpoint ask to one seat on the scope.
    Ask(Box<PendingAsk>),
    Dropped {
        recipient: String,
        summary: String,
    },
    /// A booked checkpoint ask cancelled before it was delivered, journaled as
    /// the ask's own drop rather than the advisory's.
    AskDropped {
        recipient: String,
        summary: String,
    },
    /// A booked ask withdrawn because its seat now runs another model family,
    /// and the summary that names it. Journaled with NO target: see
    /// [`crate::quota::action::CHECKPOINT_CANCELLED`].
    Cancelled(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuotaDelivery {
    Delivered,
    Retryable,
    Uncertain,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct QuotaCarry {
    sweeps_until_observe: u64,
    tracked: Vec<QuotaTracked>,
    pending: Vec<PendingAdvisory>,
    /// Booked checkpoint asks, one per scope key and seat slot. Separate from
    /// `pending` on purpose: the advisory recipients are the lead pair, the ask
    /// recipients are every seat on the scope, and neither set may widen the
    /// other.
    asks: Vec<PendingAsk>,
    /// The seats already asked about a window in its CURRENT low episode,
    /// booked or delivered. A real entry into the band clears the window's
    /// marks, and so does every cancellation before delivery: a booking is
    /// not a delivery.
    settled: Vec<(QuotaKey, QuotaRecipient)>,
    last_observation: Option<crate::quota::Observation>,
    /// When this daemon's first pass ran.
    first_pass_at: Option<i64>,
    /// The resume whose follow-up pass has run.
    start_pass: Option<i64>,
    /// Only successfully appended samples, bounded to one current pass (256 keys).
    history_seen: Vec<crate::quota::history::Sample>,
    history_note_day: Option<i64>,
}

/// Whether this classification ENTERS the checkpoint band.
///
/// `before` is `None` at a window's first sight, which counts as entering: a
/// daemon that starts with a scope already Low would otherwise stay silent
/// until the next transition, which is exactly when the seat can no longer
/// speak. `Low -> Critical` and `Critical -> Low` are both already inside the
/// band and ask nothing more; the band is left, and can be entered again, only
/// when [`crate::quota::classify`]'s own hysteresis says so.
const fn entered_low(before: Option<QuotaLevel>, after: QuotaLevel) -> bool {
    matches!(after, QuotaLevel::Low | QuotaLevel::Critical)
        && matches!(before, None | Some(QuotaLevel::Headroom))
}

fn quota_samples_at(observation: &crate::quota::Observation, now: i64) -> Vec<QuotaSample<'_>> {
    let mut samples: Vec<QuotaSample<'_>> = Vec::new();
    for group in &observation.groups {
        let Some(source) = group.source.clone() else {
            continue;
        };
        for row in &group.rows {
            if !matches!(
                row.status,
                crate::quota::Status::Fresh | crate::quota::Status::Stale
            ) {
                continue;
            }
            if !matches!(
                crate::quota::freshness(row.observed_at, row.resets_at, now),
                crate::quota::Status::Fresh | crate::quota::Status::Stale
            ) {
                continue;
            }
            let Some(reading) = crate::quota::Reading::of(group.policy.clone(), row.clone()) else {
                continue;
            };
            if now.saturating_sub(reading.observed_at()) > 60 * 60 {
                continue;
            }
            let key = QuotaKey {
                source: source.clone(),
                bucket: row.bucket.clone(),
                qualifier: row.qualifier.clone(),
                window_minutes: row.window_minutes,
            };
            let sample = QuotaSample {
                key: key.clone(),
                group,
                reading,
            };
            if let Some(existing) = samples.iter_mut().find(|sample| sample.key == key) {
                // One scope can be read through several rollouts in one scan,
                // and they report ONE account. So the same rules apply here as
                // across cycles: the newest row wins, every account fact wins
                // on its own stamp, and an accountless rollout takes nothing
                // away from a sibling that reported a cap.
                if existing.reading.adopt(&sample.reading) == crate::quota::Adopted::Observation {
                    existing.group = sample.group;
                }
            } else {
                samples.push(sample);
            }
        }
    }
    samples.sort_by(|left, right| left.key.cmp(&right.key));
    samples
}

fn quota_samples(observation: &crate::quota::Observation) -> Vec<QuotaSample<'_>> {
    quota_samples_at(observation, observation.now)
}

fn history_clock(at: Option<i64>, now: i64) -> Option<i64> {
    at.filter(|at| {
        *at > 0 && *at < now && now.saturating_sub(*at) <= crate::quota::history::ALIGN_SECS
    })
}

fn history_same(a: &crate::quota::history::Sample, b: &crate::quota::history::Sample) -> bool {
    a.scope == b.scope
        && a.bucket == b.bucket
        && a.qualifier == b.qualifier
        && a.window_minutes == b.window_minutes
        && a.resets_at == b.resets_at
        && a.observed_at == b.observed_at
        && a.policy == b.policy
}

/// Whether the seat at `slot` is one of the lead pair: the main seat, and
/// worker.0 in a lead-pair layout.
pub(crate) fn in_lead_pair(slot: &str, lead_pair: bool) -> bool {
    slot == "main" || (lead_pair && slot == "worker.0")
}

fn quota_recipients(roster: &[RosterEntry], lead_pair: bool) -> Vec<QuotaRecipient> {
    roster
        .iter()
        .filter(|entry| in_lead_pair(&entry.slot, lead_pair))
        .map(|entry| QuotaRecipient {
            slot: entry.slot.clone(),
            agent: entry.name.clone(),
            harness_session: entry.harness_session.clone(),
            config_home: entry.config_home.clone(),
            config_home_base: entry.config_home_base.clone(),
            binary: entry.binary.clone(),
        })
        .collect()
}

/// Every roster seat the checkpoint ask may reach: one that sits on a quota
/// scope ae can read, and has a live pane to speak into.
///
/// This is deliberately NOT [`quota_recipients`]. The advisory goes to the lead
/// pair, who choose profiles; the ask goes to whoever is about to lose its
/// voice, fixed seats and spawned seats alike.
///
/// Three reasons a seat is skipped, all fail-closed:
///
/// * [`crate::quota::recorded_identity`] answers `None` — the tool has no quota
///   parser (muse, opencode, gemini), or the seat's recorded config home is
///   incomplete, or a Codex seat has no recorded conversation. An unproven
///   identity is never matched to a scope, exactly as for the throttle line.
/// * No pane carries the seat's slot. A seat ae cannot see is a seat ae cannot
///   prove is listening, and the delivery would fail anyway.
/// * The pane sits at a shell. That covers the DEAD pane (a shell with the tool
///   gone) and the pane whose tool has merely been quit; neither can read.
///
/// RESIDUAL, named rather than fixed: on the tools whose TUI ae does not model,
/// the guarded send cannot see a human's half-typed draft, so an ask may land
/// mid-input there. The modelled tools defer on that draft through the existing
/// readiness check. Closing it needs a frame model per tool, which is a
/// different slice.
fn quota_ask_candidates(
    roster: &[RosterEntry],
    panes: &[crate::tmux::WatchPane],
) -> Vec<QuotaAskCandidate> {
    roster
        .iter()
        .filter(|entry| {
            panes.iter().any(|pane| {
                pane.slot.as_deref() == Some(entry.slot.as_str())
                    && !crate::watchdog::command_is_shell(&pane.current_command)
            })
        })
        .filter_map(|entry| {
            Some(QuotaAskCandidate {
                recipient: QuotaRecipient {
                    slot: entry.slot.clone(),
                    agent: entry.name.clone(),
                    harness_session: entry.harness_session.clone(),
                    config_home: entry.config_home.clone(),
                    config_home_base: entry.config_home_base.clone(),
                    binary: entry.binary.clone(),
                },
                identity: crate::quota::recorded_identity(entry)?,
                live: None,
            })
        })
        .collect()
}

/// The model this seat's own frame last proved, as [`Cycle::resolve_identity`]
/// holds it for the picker: launch-bound, composer-gated and bounded by
/// [`HOLD_MAX_CYCLES`]. Never the profile's pin.
///
/// The quota pass runs before the pane loop, so this is the PREVIOUS cycle's
/// hold. A carry that belongs to another seat generation, a hold from another
/// launch, or a seat with no launch id at all answers `None`, which asks.
fn live_model(
    recipient: &QuotaRecipient,
    panes: &[crate::tmux::WatchPane],
    carried: &[(String, PaneState)],
    launch_ids: &[(String, String)],
) -> Option<String> {
    let launch = launch_id_for(launch_ids, &recipient.slot)?;
    let generation = quiet_hash(&format!("{}\n{}", recipient.slot, recipient.agent));
    panes
        .iter()
        .filter(|pane| pane.slot.as_deref() == Some(recipient.slot.as_str()))
        .find_map(|pane| {
            let (_, state) = carried.iter().find(|(id, _)| *id == pane.pane_id)?;
            let hold = state.held_identity.as_ref()?;
            (state.identity == Some(generation)
                && hold.launch == launch
                && hold.age < HOLD_MAX_CYCLES)
                .then(|| hold.identity.model.clone())
                .flatten()
        })
}

/// The RECEIPT a delivered checkpoint ask leaves in the journal: the `ref` of
/// its `quota-checkpoint` event, which the send helper writes only when the
/// paste was delivered or left unconfirmed.
///
/// An IDENTITY, compared by exact equality and never decoded: FNV-1a 64 over a
/// version tag and the window, its reset and the seat's conversation, each
/// field length-prefixed so no two tuples share bytes. `None` — no receipt, so
/// a recovered first sight asks again — when the vendor stated no reset or the
/// seat has no recorded conversation.
fn ask_receipt(
    key: &QuotaKey,
    resets_at: Option<i64>,
    recipient: &QuotaRecipient,
) -> Option<String> {
    let resets = resets_at?.to_le_bytes();
    let conversation = crate::meta::prior_parts(recipient.harness_session.as_deref()?)?.id;
    let window = key.window_minutes.map(u32::to_le_bytes);
    let mut bytes = b"ae-quota-ask-v1".to_vec();
    for field in [
        Some(key.source.as_os_str().as_encoded_bytes()),
        Some(key.bucket.as_bytes()),
        key.qualifier.as_deref().map(str::as_bytes),
        window.as_ref().map(<[u8; 4]>::as_slice),
        Some(resets.as_slice()),
        Some(conversation.as_bytes()),
    ] {
        match field {
            None => bytes.push(0),
            Some(value) => {
                bytes.push(1);
                bytes.extend(u64::try_from(value.len()).unwrap_or(u64::MAX).to_le_bytes());
                bytes.extend_from_slice(value);
            }
        }
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    Some(format!("quota-ask-{hash:016x}"))
}

/// Every checkpoint-ask receipt the journal holds: the `ref` of a
/// `quota-checkpoint` event this daemon's actor wrote. A `delivery-abandoned`
/// record carries the same ref and is NOT one — nothing reached the seat.
fn delivered_receipts(events: &[Event]) -> Vec<&str> {
    events
        .iter()
        .filter(|event| event.action == crate::quota::action::CHECKPOINT && event.actor == ACTOR)
        .filter_map(|event| event.reference.as_deref())
        .collect()
}

/// The reset epoch of the seat's newest wake the journal holds: a
/// `limit-wake` row this daemon's actor wrote to the seat, whose `ref` parses
/// as an epoch. With `landed`, only one [`crate::events::landed`] reads as a
/// confirmed paste; without, an unconfirmed one counts too, because it may
/// have reached the seat. The send helper writes neither for a refusal.
fn limit_wake_in(
    events: &[Event],
    (session, slot, agent): (&str, &str, &str),
    landed: bool,
) -> Option<i64> {
    events
        .iter()
        .rev()
        .filter(|event| {
            event.action == crate::watchdog::LIMIT_WAKE_ACTION
                && event.actor == ACTOR
                && crate::watchdog::event_is_addressed_to(event, session, slot, agent)
                && (!landed || crate::events::landed(event))
        })
        .find_map(|event| event.reference.as_deref()?.parse::<i64>().ok())
}

/// What one quota pass reads beside its own carry.
struct QuotaInputs<'a> {
    panes: &'a [crate::tmux::WatchPane],
    /// The pane carry, for each seat's held live model.
    held: &'a [(String, PaneState)],
    /// This cycle's journal, for the checkpoint asks' receipts.
    events: &'a [Event],
}

/// Whether this daemon's `OpenCode` Go usage refresh is in flight.
static GO_USAGE_FLYING: AtomicBool = AtomicBool::new(false);

fn quota_sweep_count(knobs: &Knobs) -> Option<u64> {
    (knobs.quota_every_secs > 0).then(|| {
        knobs
            .quota_every_secs
            .div_ceil(knobs.interval_secs.max(1))
            .max(1)
    })
}

fn quota_observation_due(carry: &mut QuotaCarry, knobs: &Knobs) -> bool {
    let Some(sweeps) = quota_sweep_count(knobs) else {
        return false;
    };
    if carry.sweeps_until_observe > 0 {
        carry.sweeps_until_observe -= 1;
        return false;
    }
    carry.sweeps_until_observe = sweeps.saturating_sub(1);
    true
}

/// How long after a resume settles its one follow-up pass waits: a seat's
/// statusline feeds its first reading 2-3 s into its start (measured).
const START_PASS_DELAY_SECS: i64 = 5;

/// Whether this cycle owes the resume settled at `settled` its one follow-up
/// quota pass: the first pass may have run before the seat's first reading,
/// and the regular cadence may not come back inside the start window.
/// Separate from [`quota_observation_due`], whose counter this never advances.
fn start_pass_due(
    first_pass_at: Option<i64>,
    taken: Option<i64>,
    settled: Option<i64>,
    now: i64,
) -> bool {
    settled.is_some_and(|settled| {
        taken != Some(settled)
            && first_pass_at.is_some_and(|at| at < now)
            && settled.saturating_add(START_PASS_DELAY_SECS) <= now
            && now <= settled.saturating_add(crate::autoreseat::START_WINDOW_SECS)
    })
}

impl QuotaCarry {
    /// Drop everything a quota observation ever taught: tracked windows,
    /// pending notices and the last observation — but NOT the shared due
    /// counter, which paces the advisory passes. Called every cycle while
    /// unaware, so the property holds: while unaware, nothing holds a quota
    /// observation for any consumer (throttle line included) to inject.
    /// Losing hysteresis across an OFF/ON cycle is correct and
    /// direction-symmetric with reset declarations clearing a critical.
    fn clear_held(&mut self) {
        self.tracked.clear();
        self.pending.clear();
        // An ask is held quota knowledge too: while unaware nothing may stay
        // booked to fire the moment awareness returns. So is a mark: a later
        // first sight must consult the journal, not a memory from before.
        self.asks.clear();
        self.settled.clear();
        self.last_observation = None;
        self.history_seen.clear();
    }

    fn cancel_where(
        &mut self,
        predicate: impl Fn(&PendingAdvisory) -> bool,
        reason: &str,
        meta_dir: &Path,
        now: i64,
        actions: &mut Vec<QuotaAction>,
    ) {
        let mut retained = Vec::new();
        for pending in self.pending.drain(..) {
            if predicate(&pending) {
                actions.push(QuotaAction::Dropped {
                    recipient: pending.recipient.agent,
                    summary: format!("{reason}: {}", pending.advisory.render(meta_dir, now)),
                });
            } else {
                retained.push(pending);
            }
        }
        self.pending = retained;
    }

    /// The ask's own cancellation, kept apart from the advisory's so that
    /// neither recipient set can reach the other's bookings. A cancelled
    /// booking leaves no mark: it never reached the seat.
    fn cancel_asks_where(
        &mut self,
        predicate: impl Fn(&PendingAsk) -> bool,
        reason: &str,
        meta_dir: &Path,
        now: i64,
        actions: &mut Vec<QuotaAction>,
    ) {
        let mut retained = Vec::new();
        for ask in self.asks.drain(..) {
            if predicate(&ask) {
                self.settled
                    .retain(|(key, recipient)| !(*key == ask.key && *recipient == ask.recipient));
                actions.push(QuotaAction::AskDropped {
                    recipient: ask.recipient.agent,
                    summary: format!("{reason}: {}", ask.advisory.checkpoint_ask(meta_dir, now)),
                });
            } else {
                retained.push(ask);
            }
        }
        self.asks = retained;
    }

    /// The account-wide weekly reading of `sample`'s scope — the same tool and
    /// the same canonical source — as the cell its notice names beside its
    /// reset. Each reading is the one HELD, offered this pass's observation:
    /// a weekly window sorted after `sample` is still seen, and a row the raw
    /// clock refused never replaces a held one.
    fn held_weekly(
        &self,
        samples: &[QuotaSample<'_>],
        sample: &QuotaSample<'_>,
        trigger: &crate::quota::Classified,
    ) -> Option<String> {
        let held: Vec<crate::quota::Classified> = samples
            .iter()
            .filter(|other| {
                other.key != sample.key
                    && other.group.tool == sample.group.tool
                    && other.key.source == sample.key.source
            })
            .map(|other| {
                let Some(tracked) = self.tracked.iter().find(|held| held.key == other.key) else {
                    return crate::quota::Classified::first(other.reading.clone());
                };
                let mut classified = tracked.classified.clone();
                let _ = classified.adopt(&other.reading);
                classified
            })
            .collect();
        crate::quota::weekly_context(sample.group.tool, trigger, &held)
    }

    /// Book ONE checkpoint ask for every seat on a scope whose window sits in
    /// the band, unless that seat is already settled for this episode or its
    /// live model is proven to be of another family than the window's.
    ///
    /// Run every pass, not only at an entry, so a seat that switches INTO the
    /// window's family mid-episode is asked too. A seat that already holds an
    /// undelivered ask for the key has it REPLACED rather than doubled: a band
    /// that cleared and was entered again while the first ask was still
    /// deferred must still produce exactly one ask, carrying the newer facts.
    fn book_asks(
        &mut self,
        observation: &crate::quota::Observation,
        samples: &[QuotaSample<'_>],
        candidates: &[QuotaAskCandidate],
    ) {
        for sample in samples {
            let Some(held) = self
                .tracked
                .iter()
                .find(|tracked| tracked.key == sample.key)
                .map(|tracked| tracked.classified.clone())
                .filter(|held| matches!(held.level(), QuotaLevel::Low | QuotaLevel::Critical))
            else {
                continue;
            };
            let key = &sample.key;
            let unasked: Vec<&QuotaAskCandidate> = candidates
                .iter()
                .filter(|candidate| {
                    candidate.identity.tool == sample.group.tool
                        && candidate.identity.source == key.source
                        && !self
                            .settled
                            .contains(&(key.clone(), candidate.recipient.clone()))
                        && !crate::quota::scoped_elsewhere(
                            candidate.identity.tool,
                            key.qualifier.as_deref(),
                            candidate.live.as_deref(),
                        )
                })
                .collect();
            for candidate in unasked {
                self.asks
                    .retain(|ask| !(ask.key == *key && ask.recipient == candidate.recipient));
                self.asks.push(PendingAsk {
                    key: key.clone(),
                    observed_at: held.observed_at(),
                    recipient: candidate.recipient.clone(),
                    advisory: observation
                        .transition(sample.group, &held)
                        .with_weekly(self.held_weekly(samples, sample, &held)),
                    attempts: 0,
                });
                self.settled
                    .push((key.clone(), candidate.recipient.clone()));
            }
        }
    }

    /// Withdraw every booked ask whose seat's live model is now PROVEN to be
    /// of another family than its window, clearing the mark so a switch back
    /// is asked again. Named, and journaled with no target.
    ///
    /// The live model is the previous cycle's hold, so a switch inside one
    /// pass cannot be seen here: this reaches a DEFERRED ask, one pass later.
    fn withdraw_switched(
        &mut self,
        candidates: &[QuotaAskCandidate],
        meta_dir: &Path,
        now: i64,
        actions: &mut Vec<QuotaAction>,
    ) {
        let mut retained = Vec::new();
        for ask in self.asks.drain(..) {
            let switched = candidates.iter().find(|candidate| {
                candidate.recipient == ask.recipient
                    && crate::quota::scoped_elsewhere(
                        candidate.identity.tool,
                        ask.key.qualifier.as_deref(),
                        candidate.live.as_deref(),
                    )
            });
            let Some(live) = switched.and_then(|candidate| candidate.live.as_deref()) else {
                retained.push(ask);
                continue;
            };
            self.settled
                .retain(|(key, recipient)| !(*key == ask.key && *recipient == ask.recipient));
            actions.push(QuotaAction::Cancelled(format!(
                "{}: checkpoint seat now runs {live}, not {}: {}",
                ask.recipient.agent,
                ask.key.qualifier.as_deref().unwrap_or_default(),
                ask.advisory.checkpoint_ask(meta_dir, now)
            )));
        }
        self.asks = retained;
    }

    /// Mark every seat on a window's scope whose receipt the journal already
    /// holds, at a FIRST sight inside the band: a restart or a recovery from
    /// silence inside an episode that seat was already asked about.
    fn recover_settled(
        &mut self,
        key: &QuotaKey,
        advisory: &crate::quota::Advisory,
        candidates: &[QuotaAskCandidate],
        receipts: &[&str],
    ) {
        for candidate in candidates.iter().filter(|candidate| {
            ask_receipt(key, advisory.resets_at(), &candidate.recipient)
                .is_some_and(|receipt| receipts.contains(&receipt.as_str()))
        }) {
            self.settled
                .push((key.clone(), candidate.recipient.clone()));
        }
    }

    /// Drop the booked asks this pass may no longer deliver: one whose seat is
    /// gone from the candidate set, and one whose facts have aged out.
    ///
    /// Deliberately NOT keyed on the level: an ask booked at an entry survives
    /// every later move inside the band, because cancelling it there would mean
    /// the seat is never asked at all.
    fn cancel_stale_asks(
        &mut self,
        candidates: &[QuotaAskCandidate],
        now: i64,
        meta_dir: &Path,
        actions: &mut Vec<QuotaAction>,
    ) {
        self.cancel_asks_where(
            |ask| {
                !candidates
                    .iter()
                    .any(|candidate| candidate.recipient == ask.recipient)
            },
            "checkpoint recipient is gone",
            meta_dir,
            now,
            actions,
        );
        self.cancel_asks_where(
            |ask| !ask.advisory.current_at(now),
            "checkpoint ask expired",
            meta_dir,
            now,
            actions,
        );
    }

    /// One quota pass. `receipts` are the journal's [`delivered_receipts`],
    /// consulted ONLY at a window's first sight inside the band.
    ///
    /// The order is load-bearing: stale asks are cancelled first, so a seat
    /// that is eligible again is booked afresh by the same pass; the sample
    /// loop then decides entries and recoveries; a switched seat's deferred
    /// ask is withdrawn; and only then are the unasked seats booked.
    fn reconcile_pass(
        &mut self,
        observation: &crate::quota::Observation,
        recipients: &[QuotaRecipient],
        candidates: &[QuotaAskCandidate],
        receipts: &[&str],
        meta_dir: &Path,
    ) -> Vec<QuotaAction> {
        let mut actions = Vec::new();
        self.cancel_stale_asks(candidates, observation.now, meta_dir, &mut actions);
        self.cancel_where(
            |pending| !recipients.contains(&pending.recipient),
            "recipient identity changed",
            meta_dir,
            observation.now,
            &mut actions,
        );
        self.cancel_where(
            |pending| !pending.advisory.current_at(observation.now),
            "quota advisory expired",
            meta_dir,
            observation.now,
            &mut actions,
        );

        let samples = quota_samples(observation);
        self.drop_silent_keys(&samples, meta_dir, observation.now, &mut actions);

        for sample in &samples {
            let previous = self
                .tracked
                .iter()
                .position(|tracked| tracked.key == sample.key);
            let Some(index) = previous else {
                // The first sight of a window is classified and SILENT: there
                // is no transition to report yet.
                let classified = crate::quota::Classified::first(sample.reading.clone());
                // The ASK is not silent here, and that is the whole point: a
                // daemon that starts with the scope already Low has seats which
                // may never get another transition to be warned by. The
                // advisory's own first-sight silence above is untouched. A
                // first sight may also be a RECOVERY — a restart, or a window
                // back from silence — inside an episode a seat was already
                // asked about; its receipt in the journal says so.
                if entered_low(None, classified.level()) {
                    let advisory = observation.transition(sample.group, &classified);
                    self.recover_settled(&sample.key, &advisory, candidates, receipts);
                }
                self.tracked.push(QuotaTracked {
                    key: sample.key.clone(),
                    classified,
                });
                continue;
            };
            // Two clocks, one classification, and the classified reading keeps
            // both: the RAW observation is replaced only by a strictly newer
            // stamp, whatever the policy says, while the declaration and the
            // account facts merge on their own provenance. Either way the level
            // that comes back was decided from the observation now HELD.
            let before = self.tracked[index].classified.level();
            let adopted = self.tracked[index].classified.adopt(&sample.reading);
            if adopted == crate::quota::Adopted::Nothing {
                continue;
            }
            let after = self.tracked[index].classified.level();
            if before == after {
                continue;
            }
            self.cancel_where(
                |pending| pending.key == sample.key,
                if adopted == crate::quota::Adopted::Observation {
                    "superseded by newer quota transition"
                } else {
                    "superseded by a quota policy change"
                },
                meta_dir,
                observation.now,
                &mut actions,
            );
            // The notice describes the HELD observation, which is also what
            // decided the level, so its numbers and the provenance it is booked
            // under cannot be another sample's.
            let held = self.tracked[index].classified.clone();
            let advisory = observation
                .transition(sample.group, &held)
                .with_weekly(self.held_weekly(&samples, sample, &held));
            // ENTERING the band opens a new episode: every seat on the scope is
            // asked once more. Moving around inside it does not, and neither
            // does leaving it; only a re-entry after `classify`'s hysteresis
            // let the window clear asks again. A declaration change re-derives
            // the held row and reaches here the same way, so a withdrawn reset
            // re-arms the ask exactly as it re-arms the advisory. A real entry
            // never consults the journal.
            if entered_low(Some(before), after) {
                self.settled.retain(|(key, _)| *key != sample.key);
            }
            for recipient in recipients {
                self.pending.push(PendingAdvisory {
                    key: sample.key.clone(),
                    observed_at: held.observed_at(),
                    level: after,
                    recipient: recipient.clone(),
                    advisory: advisory.clone(),
                    attempts: 0,
                });
            }
        }
        self.withdraw_switched(candidates, meta_dir, observation.now, &mut actions);
        self.book_asks(observation, &samples, candidates);

        actions.extend(
            self.pending
                .iter()
                .cloned()
                .map(Box::new)
                .map(QuotaAction::Deliver),
        );
        actions.extend(
            self.asks
                .iter()
                .cloned()
                .map(Box::new)
                .map(QuotaAction::Ask),
        );
        self.last_observation = Some(observation.clone());
        actions
    }

    /// Forget every key this observation no longer reports, and cancel the
    /// notices that were still waiting on them.
    fn drop_silent_keys(
        &mut self,
        samples: &[QuotaSample<'_>],
        meta_dir: &Path,
        now: i64,
        actions: &mut Vec<QuotaAction>,
    ) {
        let live_keys: Vec<QuotaKey> = samples.iter().map(|sample| sample.key.clone()).collect();
        let silent_keys: Vec<QuotaKey> = self
            .tracked
            .iter()
            .filter(|tracked| !live_keys.contains(&tracked.key))
            .map(|tracked| tracked.key.clone())
            .collect();
        self.tracked
            .retain(|tracked| live_keys.contains(&tracked.key));
        // Its marks go with it: the window's return is a first sight, and a
        // first sight answers to the journal alone.
        self.settled.retain(|(key, _)| live_keys.contains(key));
        self.cancel_where(
            |pending| silent_keys.contains(&pending.key),
            "quota observation went silent",
            meta_dir,
            now,
            actions,
        );
        self.cancel_asks_where(
            |ask| silent_keys.contains(&ask.key),
            "quota observation went silent",
            meta_dir,
            now,
            actions,
        );
    }

    fn record_delivery(
        &mut self,
        delivered: &PendingAdvisory,
        result: QuotaDelivery,
        meta_dir: &Path,
        now: i64,
    ) -> Option<QuotaAction> {
        let index = self.pending.iter().position(|pending| {
            pending.key == delivered.key
                && pending.observed_at == delivered.observed_at
                && pending.level == delivered.level
                && pending.recipient == delivered.recipient
        })?;
        match result {
            QuotaDelivery::Delivered => {
                self.pending.remove(index);
                return None;
            }
            QuotaDelivery::Uncertain => {
                let pending = self.pending.remove(index);
                return Some(QuotaAction::Dropped {
                    recipient: pending.recipient.agent,
                    summary: format!(
                        "delivery result uncertain; not retrying: {}",
                        pending.advisory.render(meta_dir, now)
                    ),
                });
            }
            QuotaDelivery::Retryable => {}
        }
        self.pending[index].attempts = self.pending[index].attempts.saturating_add(1);
        if self.pending[index].attempts < 2 {
            return None;
        }
        let pending = self.pending.remove(index);
        Some(QuotaAction::Dropped {
            recipient: pending.recipient.agent,
            summary: format!(
                "delivery failed twice: {}",
                pending.advisory.render(meta_dir, now)
            ),
        })
    }

    /// The ask's delivery bookkeeping, mirroring [`Self::record_delivery`]: a
    /// delivered ask is FORGOTTEN, so the deferral that got it through cannot
    /// be followed by a duplicate; a proven pre-submit refusal is retried once;
    /// anything ambiguous is dropped rather than risk pasting it twice.
    fn record_ask_delivery(
        &mut self,
        delivered: &PendingAsk,
        result: QuotaDelivery,
        meta_dir: &Path,
        now: i64,
    ) -> Option<QuotaAction> {
        let index = self.asks.iter().position(|ask| {
            ask.key == delivered.key
                && ask.observed_at == delivered.observed_at
                && ask.recipient == delivered.recipient
        })?;
        match result {
            QuotaDelivery::Delivered => {
                self.asks.remove(index);
                return None;
            }
            QuotaDelivery::Uncertain => {
                let ask = self.asks.remove(index);
                return Some(QuotaAction::Dropped {
                    recipient: ask.recipient.agent,
                    summary: format!(
                        "delivery result uncertain; not retrying: {}",
                        ask.advisory.checkpoint_ask(meta_dir, now)
                    ),
                });
            }
            QuotaDelivery::Retryable => {}
        }
        self.asks[index].attempts = self.asks[index].attempts.saturating_add(1);
        if self.asks[index].attempts < 2 {
            return None;
        }
        let ask = self.asks.remove(index);
        Some(QuotaAction::Dropped {
            recipient: ask.recipient.agent,
            summary: format!(
                "delivery failed twice: {}",
                ask.advisory.checkpoint_ask(meta_dir, now)
            ),
        })
    }
}

/// One candidate for the throttle line: the classified observation itself, so
/// ranking and rendering cannot read different ones.
#[derive(Debug, Clone)]
struct QuotaReadout<'a> {
    key: QuotaKey,
    group: &'a crate::quota::Group,
    classified: crate::quota::Classified,
}

fn throttle_quota_line(
    observation: &crate::quota::Observation,
    tracked: &[QuotaTracked],
    entry: &RosterEntry,
    meta_dir: &Path,
    now: i64,
) -> Option<String> {
    let identity = crate::quota::recorded_identity(entry)?;
    // Whatever provenance decides the LEVEL supplies the numbers rendered with
    // it. For a scope this daemon already tracks that is the HELD reading, so a
    // sample the raw clock refused is never what a throttled seat is shown.
    let mut matches: Vec<QuotaReadout<'_>> = quota_samples_at(observation, now)
        .into_iter()
        .filter(|sample| sample.group.tool == identity.tool && sample.key.source == identity.source)
        .map(|sample| {
            let classified = match tracked.iter().find(|held| held.key == sample.key) {
                Some(held) => held.classified.clone(),
                None => crate::quota::Classified::first(sample.reading),
            };
            QuotaReadout {
                key: sample.key,
                group: sample.group,
                classified,
            }
        })
        .collect();
    matches.sort_by(|left, right| {
        right
            .classified
            .level()
            .cmp(&left.classified.level())
            .then_with(|| {
                right
                    .classified
                    .judged()
                    .total_cmp(&left.classified.judged())
            })
            .then_with(|| left.key.cmp(&right.key))
    });
    let worst = matches.first()?;
    Some(observation.state_line_at(worst.group, &worst.classified, meta_dir, now))
}

/// The result of accounting for one pane in one cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accounting {
    /// The state to carry into the next cycle.
    pub next: PaneState,
    /// What the loop must do, in order.
    pub effects: Vec<Effect>,
    /// The glyph verdict for the status line.
    pub verdict: Verdict,
    /// Whether the pane changed since the last liveness capture.
    pub moved: bool,
}

/// `idle <n>m`, or `no recent events` when the age is absurd.
#[must_use]
pub fn stale_display(event_age_secs: u64) -> String {
    let minutes = event_age_secs / 60;
    if minutes > 9999 {
        "no recent events".to_owned()
    } else {
        format!("idle {minutes}m")
    }
}

/// The ended-wait note's slot in a nudge: one leading space and the note, or
/// nothing at all — the ONE insertion, between the nudge sentence and the
/// declaration invitation, so a noted nudge is the plain one plus a clause.
fn note_slot(note: Option<&str>) -> String {
    note.map_or_else(String::new, |note| format!(" {note}"))
}

/// A goal clipped to one short line: the crate's one cutter, head-kept.
const GOAL_CLIP_WIDTH: usize = 80;

/// The nudge's goal prefix, shared by all five generators: a fitting goal
/// keeps its `Session goal: …. ` form, a long one is clipped to
/// [`GOAL_CLIP_WIDTH`] cells ending in the cutter's mark with no added period.
fn goal_prefix(goal: Option<&str>) -> String {
    goal.map_or_else(String::new, |goal| {
        let clipped = crate::event_text::clip_head(goal, GOAL_CLIP_WIDTH);
        if clipped == goal {
            format!("{NUDGE_GOAL_PREFIX}{goal}. ")
        } else {
            format!("{NUDGE_GOAL_PREFIX}{clipped} ")
        }
    })
}

/// The nudge: the session goal when the meta carries one, then the status
/// sentence, then the path to this session's own `state` helper.
///
/// The sentence, the goal prefix and the invitation are spelled through the
/// same constants `watchdog::raw_nudge` strips from a pane baseline, so the
/// delivered bytes and the footprint grammar can never disagree about what a
/// nudge looks like.
#[must_use]
pub fn nudge_text(goal: Option<&str>, meta_dir: &Path, note: Option<&str>) -> String {
    let prefix = goal_prefix(goal);
    let note = note_slot(note);
    format!(
        "{prefix}{NUDGE_SENTENCE}{note} Declare state: {}{NUDGE_TAIL}",
        meta_dir.display()
    )
}

/// The limit wake: the nudge, with one note naming the reset that passed. It
/// is spelled through [`nudge_text`] so the footprint grammar strips it like
/// any nudge and a delivered wake never reads as the seat's own output.
#[must_use]
pub fn limit_wake_text(goal: Option<&str>, meta_dir: &Path, reset: i64) -> String {
    let note = format!(
        "Your vendor usage limit reset at {}.",
        Timestamp::from_epoch(reset)
    );
    nudge_text(goal, meta_dir, Some(&note))
}

/// The idle reminder uses the same delivery path but names the positive
/// observation that started its independent clock.
#[must_use]
pub fn idle_nudge_text(goal: Option<&str>, meta_dir: &Path, note: Option<&str>) -> String {
    let prefix = goal_prefix(goal);
    let note = note_slot(note);
    format!(
        "{prefix}{NUDGE_IDLE_PREFIX}{NUDGE_SENTENCE}{note} Declare state: {}{NUDGE_TAIL}",
        meta_dir.display()
    )
}

#[must_use]
pub fn done_challenge_text(
    goal: Option<&str>,
    meta_dir: &Path,
    age: u64,
    confirmations: u8,
    required: u8,
) -> String {
    let prefix = goal_prefix(goal);
    let minutes = age / 60;
    format!(
        "{prefix}{NUDGE_SENTENCE} Done was \
         declared {minutes}m ago; confirmation {} of {required}. State how each deliverable was \
         verified. Anything unverified: declare working and finish assigned work NOW. Awaiting \
         owner review: invent no scope; do not commit or edit solely for this challenge. Else \
         re-declare done with work and proof, held review included. Self-attestation, not owner \
         approval. Declare state: {}{NUDGE_TAIL}",
        confirmations.saturating_add(1),
        meta_dir.display()
    )
}

/// One wait challenge's facts: what [`wait_challenge_text`] says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitCheck<'a> {
    /// The wait's age in awake seconds.
    pub age: u64,
    /// Confirmations already credited in this episode.
    pub confirmations: u8,
    /// Confirmations an episode asks for.
    pub required: u8,
    /// The RAW declared state the proof re-declares.
    pub state: WaitState,
    /// Whether the wait reads past its ceiling this cycle.
    pub escalated: bool,
    /// The `--on` target the declaration names, if any.
    pub target: Option<&'a str>,
}

/// Proof challenge for a `waiting-agent`/`blocked` declaration (I14): one
/// line naming the raw state, its target, its age and the confirmation, then
/// the proof owed and the invitation. It opens with
/// [`crate::watchdog::NUDGE_WAIT_OPEN`] where a nudge has its sentence, so the
/// footprint filter strips it. `goal` is passed only when the seat has not
/// seen it ([`crate::watchdog::wait_goal_owed`]).
#[must_use]
pub fn wait_challenge_text(goal: Option<&str>, meta_dir: &Path, check: &WaitCheck<'_>) -> String {
    let prefix = goal_prefix(goal);
    let minutes = check.age / 60;
    let state = check.state.as_str();
    let declared = match (check.state, check.target) {
        (WaitState::WaitingAgent, Some(target)) => format!("{state} on {target}"),
        _ => state.to_owned(),
    };
    let proof = match check.state {
        WaitState::WaitingAgent => "with result and last chase",
        WaitState::Blocked => "naming blocker, owner and last check",
    };
    let ceiling = if check.escalated {
        " Reads blocked: past the ceiling."
    } else {
        ""
    };
    format!(
        "{prefix}{NUDGE_WAIT_OPEN}{declared}, {minutes}m, confirmation {}/{}.{ceiling} Still true? Re-declare \
         {proof}; else declare working. Declare state: {}{NUDGE_TAIL}",
        check.confirmations.saturating_add(1),
        check.required,
        meta_dir.display()
    )
}

/// The reason cap in a moved-target nudge, in cells.
const MOVED_REASON_WIDTH: usize = 200;

/// Tell a `waiting-agent` seat that the `target` it waits on moved, in the
/// nudge's envelope so the footprint filter strips it, and the fixed journal
/// summary of that move. The target's own reason is quoted through the one
/// pasted-field neutraliser and the pasted-text cutter; the nudge claims no
/// verdict, no closed request and no ended task, and ends no declaration.
#[must_use]
pub fn target_moved_text(
    goal: Option<&str>,
    meta_dir: &Path,
    age: u64,
    target: &str,
    moved: &crate::wait_on::Moved,
) -> (String, String) {
    use crate::wait_on::Class;
    let prefix = goal_prefix(goal);
    let (minutes, word) = (age / 60, moved.class.word());
    let quote = pasted_reason(&moved.reason, MOVED_REASON_WIDTH);
    let news = match (moved.class, quote.trim().is_empty()) {
        (Class::Retired, _) => format!("{target} was retired {minutes}m ago."),
        (_, true) => format!("{target} declared {word} {minutes}m ago."),
        (_, false) => format!("{target} declared {word} {minutes}m ago: \"{quote}\"."),
    };
    let text = format!(
        "{prefix}{NUDGE_SENTENCE} {news} That is its record, not a verdict or a closed request, \
         and it ends nothing: check it yourself, then re-declare waiting-agent or declare \
         working. Declare state: {}{NUDGE_TAIL}",
        meta_dir.display()
    );
    (text, moved.class.summary())
}

/// The same reminder for a seat whose OWN work is outstanding — it reached the
/// deferral ceiling, so it is told WHAT ae thinks it is waiting on rather than
/// being asked a question it already answered.
#[must_use]
pub fn idle_nudge_text_waiting(
    goal: Option<&str>,
    meta_dir: &Path,
    reason: &str,
    note: Option<&str>,
) -> String {
    format!(
        "{}{NUDGE_WAITING_OPEN}{reason}{NUDGE_WAITING_CLOSE}",
        idle_nudge_text(goal, meta_dir, note)
    )
}

/// The ended wait's own clause: `waiting-user ended on input in the pane 2m
/// ago`. The journal summary carries it verbatim.
#[must_use]
fn wait_ended_clause(ended: WaitEnded) -> String {
    format!(
        "{} ended on input in the pane {}m ago",
        ended.state_word(),
        ended.input_age_secs / 60
    )
}

/// The mid-nudge note for a wait this cycle ended on input: WHAT ended, WHEN
/// the input came, that input is all ae saw (never an answer), and the next
/// step while the wait still holds.
#[must_use]
fn wait_ended_note(ended: WaitEnded) -> String {
    let state = ended.state_word();
    format!(
        "Client input in your pane ended your {state} {}m ago (keypress, click, scroll or switch \
         — not proof of an answer). Still waiting: re-declare {state} with the current reason.",
        ended.input_age_secs / 60
    )
}

/// The seat's own declared words in a nudge, in cells.
const TASK_QUOTE_WIDTH: usize = 120;

/// A seat's words quoted into pasted text: one line, a leading `⟦ae:…⟧` marker
/// quoted rather than live, head-kept to `width` cells ending in the cutter's
/// mark.
fn pasted_reason(reason: &str, width: usize) -> String {
    crate::event_text::clip_head(
        &crate::seatpack::neutralise(&crate::goal::printable(reason)),
        width,
    )
}

/// What a stale nudge says the seat is working on: the words of its newest
/// declaration and the state they were declared with, quoted — never an
/// inferred task. `None` unless the journal holds a declaration of this seat's
/// own, in one of [`crate::state::VALUES`], that left words, so an incomplete or
/// hostile record fabricates and injects nothing.
#[must_use]
pub fn declared_clause(events: &[Event], session: &str, slot: &str, agent: &str) -> Option<String> {
    let (event, _) = crate::session::declaration_of(events, session, slot, agent)?;
    // The `ref` is persisted input: only the helper's own vocabulary is pasted.
    let state = event
        .declared_state()
        .filter(|state| crate::state::VALUES.contains(state))?;
    let words = pasted_reason(event.summary.as_deref()?, TASK_QUOTE_WIDTH);
    let words = words.trim();
    (!words.is_empty()).then(|| format!("You last declared {state}: \"{words}\"."))
}

/// The text and the journal summary of one nudge: which of the three
/// generators speaks, and the ended-wait note that rides it. ONE owner, so
/// what the seat reads and what the journal keeps cannot disagree about why
/// it was nudged.
#[must_use]
pub fn nudge_words(
    goal: Option<&str>,
    meta_dir: &Path,
    idle_age: Option<u64>,
    display: &str,
    waiting: Option<&str>,
    task: Option<&str>,
    ended: Option<WaitEnded>,
) -> (String, String) {
    let notes: Vec<String> = task
        .map(ToOwned::to_owned)
        .into_iter()
        .chain(ended.map(wait_ended_note))
        .collect();
    let note = (!notes.is_empty()).then(|| notes.join(" "));
    let text = match (idle_age.is_some(), waiting) {
        (true, Some(reason)) => idle_nudge_text_waiting(goal, meta_dir, reason, note.as_deref()),
        (true, None) => idle_nudge_text(goal, meta_dir, note.as_deref()),
        (false, _) => nudge_text(goal, meta_dir, note.as_deref()),
    };
    let summary = match (idle_age.is_some(), waiting) {
        (true, Some(reason)) => format!("{display}, {reason}"),
        (true, None) => format!("{display}, harness waiting at input"),
        (false, _) => format!("{display}, no recent ae activity"),
    };
    let summary = match ended {
        Some(ended) => format!("{summary}; {}", wait_ended_clause(ended)),
        None => summary,
    };
    (text, summary)
}

/// Count consecutive unusable process snapshots, and say so once.
fn book_unknown(next: &mut PaneState, effects: &mut Vec<Effect>, descendancy: Descendancy) {
    if !matches!(descendancy, Descendancy::Unknown) {
        next.unknown_streak = 0;
        next.unknown_alerted = false;
        return;
    }
    next.unknown_streak = next.unknown_streak.saturating_add(1);
    if next.unknown_streak >= UNKNOWN_ALERT_CYCLES && !next.unknown_alerted {
        next.unknown_alerted = true;
        effects.push(Effect::Emit {
            action: "alert",
            summary: format!(
                "process probe unusable for {} cycles — liveness unverifiable",
                next.unknown_streak
            ),
        });
    }
}

/// The throttled branch.
fn book_throttle(
    next: &mut PaneState,
    effects: &mut Vec<Effect>,
    seen: &Observation,
    knobs: &Knobs,
) {
    let previous = next.throttle_streak;
    next.throttle_streak = previous.saturating_add(1);
    if previous == 0 {
        effects.push(Effect::Emit {
            action: "throttled",
            summary: seen.throttle_quota.as_deref().map_or_else(
                || "upstream throttling detected — pausing nudges".to_owned(),
                |line| format!("upstream throttling detected — pausing nudges; {line}"),
            ),
        });
    }
    if next.throttle_streak == knobs.throttle_alert_cycles {
        let seconds = u64::from(knobs.throttle_alert_cycles) * knobs.interval_secs;
        effects.push(Effect::Emit {
            action: "alert",
            summary: format!("throttled for {seconds}s — may need attention"),
        });
        effects.push(Effect::Notify("throttled persistently".to_owned()));
    }
    next.prev_hash = Some(seen.hash);
    next.last_hash_change = Some(seen.now_epoch);
    next.idle_since_epoch = None;
    next.nudge_count = 0;
}

/// Whether THIS cycle judged `slot` to be waiting on a human-only prompt —
/// channel two of the retry's two, and pure so it can be pinned without a pane.
/// A slot the cycle did not judge at all is not latched: absence of a verdict
/// is not a verdict, and channel one still asks the pane for itself.
fn slot_latched(verdicts: &[(String, SeatVerdict)], slot: &str) -> bool {
    verdicts
        .iter()
        .any(|(at, seat)| at == slot && seat.verdict == Verdict::HumanPrompt)
}

/// Count a human-only prompt, and NAME it once it has held. `None` means the
/// pane shows none this cycle and the branch does not apply.
///
/// The stability count is the whole false-positive bound: a menu a human is
/// scrolling through redraws, and one cycle of it is not a seat that is stuck.
/// It also absorbs the detector's one known miss: a modal painted before its
/// selection lands carries no `>` row and classifies as nothing, which costs a
/// cycle and never a false name.
/// The event and the Notify line name WHICH seat and WHAT to press, because a
/// verdict nobody can act on is not news. ae NEVER sends the key here: the
/// one prompt it may answer, a trust block [`crate::autotrust::due`] proved the
/// seat's, is booked as [`Effect::AnswerTrust`] and answered in `apply`.
fn book_human_prompt(
    prior: &PaneState,
    next: &mut PaneState,
    effects: &mut Vec<Effect>,
    (seen, trust): (&Observation, Option<&crate::autotrust::Due>),
    knobs: &Knobs,
) -> Option<Verdict> {
    let prompt = seen.human_prompt.as_ref()?;
    let restarted = prior.prompt_start.is_some() && seen.start.is_some();
    let streak = if restarted && prior.prompt_start != seen.start {
        0
    } else {
        prior.human_prompt_streak
    };
    next.human_prompt_streak = streak.saturating_add(1);
    next.prompt_start.clone_from(&seen.start);
    if next.human_prompt_streak < knobs.human_prompt_cycles {
        // A trust block proven this seat's is answered from its FIRST sight:
        // the answerer proves the frame steady itself (two equal reads), so
        // the bound only times the NAMING, which stays at the bound.
        if !prompt.dismiss && trust == Some(&crate::autotrust::Due::Yes) {
            effects.push(Effect::AnswerTrust { name: false });
        }
        return None;
    }
    // EXACTLY at the bound, so the episode is named once however long it
    // lasts — `book_throttle`'s rule, and the reason no latch flag is needed.
    let name = next.human_prompt_streak == knobs.human_prompt_cycles;
    match trust {
        // A panel is named first: its naming is the first sight its quiet
        // gate ages from, so the Escape always follows a record.
        Some(crate::autotrust::Due::Yes) if prompt.dismiss => {
            if name {
                effects.extend(naming(prompt));
            }
            effects.push(Effect::AnswerTrust { name: false });
        }
        // A trust block that is this seat's to answer is answered on every
        // cycle from the bound on until its start is spent, and named only
        // when the answer does not land.
        Some(crate::autotrust::Due::Yes) => effects.push(Effect::AnswerTrust { name }),
        Some(crate::autotrust::Due::Doubt(why)) if name => {
            effects.push(Effect::Note(format!(
                "auto_trust did not answer the trust prompt: {why}"
            )));
            effects.extend(naming(prompt));
        }
        _ if name => effects.extend(naming(prompt)),
        _ => {}
    }
    Some(Verdict::HumanPrompt)
}

/// The ONE naming of a human-only prompt: its event and its Notify line.
fn naming(prompt: &crate::watchdog::HumanPrompt) -> [Effect; 2] {
    [
        Effect::Emit {
            action: "human-prompt",
            summary: format!("{} — press: {}", prompt.question, prompt.keys),
        },
        Effect::Notify(format!(
            "waiting for you: {} — press: {}",
            prompt.question, prompt.keys
        )),
    ]
}

/// Whether this cycle is inside a usage-limit episode: the notice is on
/// screen, or a latched seat's capture FAILED — no reading is not a clearing.
fn limited(prior: &PaneState, seen: &Observation) -> bool {
    seen.throttle == Some(Throttle::LimitReached)
        || (prior.limit_since.is_some() && !seen.capture_ok)
}

/// The usage-limit branch: throttling's nudge suppression, plus ONE durable
/// `limit` event — the word `ae list` reads — and ONE named notice per
/// episode, both at its first sight and both quoting the vendor's own cell.
fn book_limit(next: &mut PaneState, effects: &mut Vec<Effect>, seen: &Observation) {
    if next.limit_since.is_none() {
        next.limit_since = Some(seen.now_epoch);
        let cell = seen.limit_notice.as_deref();
        let quoted = cell.map(|cell| format!(": {cell}")).unwrap_or_default();
        effects.push(Effect::Emit {
            action: "limit",
            summary: format!(
                "vendor usage limit reached — waits for a reset or a re-login{quoted}"
            ),
        });
        let forecast = seen
            .auto_deadline
            .as_deref()
            .map(|line| format!("; {line}"))
            .unwrap_or_default();
        effects.push(Effect::Notify(format!(
            "hit its vendor usage limit{quoted}{forecast}"
        )));
    }
    // The limit episode ends any transient streak: a return to plain
    // throttling is news again, not a continuation.
    next.throttle_streak = 0;
    next.prev_hash = Some(seen.hash);
    next.last_hash_change = Some(seen.now_epoch);
    next.idle_since_epoch = None;
    next.nudge_count = 0;
}

/// The grace a passed reset is given before its wake, for the vendor's clock.
pub const LIMIT_WAKE_GRACE_SECS: i64 = 60;

/// The limit latch's ONE continuation turn per reset, booked on a cycle that
/// keeps the `limit` verdict and changes nothing else about the latch.
///
/// This cycle's reading of the binding reset, when there is one, replaces the
/// carried one (a new instant starts its own refused-attempt count); no
/// reading clears nothing, because a passed reset no longer reads at all. The
/// wake is due from [`LIMIT_WAKE_GRACE_SECS`] past the carried reset, unless
/// that instant is spent — woken in this daemon's carry, or in the journal a
/// restarted one reads — or its refused attempts reached `undelivered_max`.
/// No reset known, no wake: a time is never guessed. Only a pane READ this
/// cycle and not busy, drafted, prompting or mid-move is woken; an unknown
/// frame is, because the guarded delivery refuses for itself.
fn book_limit_wake(
    next: &mut PaneState,
    effects: &mut Vec<Effect>,
    seen: &Observation,
    knobs: &Knobs,
) {
    if let Some(reading) = seen.limit_reset {
        if next.limit_reset.map(|held| held.at) != Some(reading.at) {
            next.limit_wake_attempts = 0;
        }
        next.limit_reset = Some(reading);
    }
    let Some(reset) = next.limit_reset else {
        return;
    };
    // A blind cycle neither arms nor disarms. A footer first read gone at or
    // after the reset cannot be told from the vendor having fired: it stands.
    if seen.capture_ok {
        if seen.auto_continue {
            next.limit_armed = Some(reset.at);
        } else if next.limit_armed == Some(reset.at) && seen.now_epoch < reset.at {
            next.limit_armed = None;
        }
    }
    let spent = next.limit_woken == Some(reset.at)
        || seen.limit_woke_journal == Some(reset.at)
        || next.limit_armed == Some(reset.at);
    let ready = seen.capture_ok
        && seen.harness.frame != HarnessState::Busy
        && !seen.harness.human_draft
        && seen.human_prompt.is_none()
        && seen.auto_in_flight.is_none();
    if !spent
        && ready
        && seen.now_epoch >= reset.at.saturating_add(LIMIT_WAKE_GRACE_SECS)
        && next.limit_wake_attempts < knobs.undelivered_max
    {
        effects.push(Effect::LimitWake { reset: reset.at });
    }
}

/// What a stale pane earns: a nudge, the one max-nudges alert, or nothing.
fn book_stale(
    prior: &PaneState,
    next: &mut PaneState,
    effects: &mut Vec<Effect>,
    knobs: &Knobs,
    display_age_secs: u64,
) {
    if prior.undelivered_streak >= knobs.undelivered_max {
        return;
    }
    if prior.nudge_count < knobs.max_nudges {
        effects.push(Effect::Nudge);
    } else if prior.nudge_count == knobs.max_nudges {
        let display = stale_display(display_age_secs);
        effects.push(Effect::Emit {
            action: "alert",
            summary: format!("max nudges reached ({display}), needs attention"),
        });
        effects.push(Effect::Notify(format!(
            "may need attention — stale after {} nudges",
            knobs.max_nudges
        )));
        next.nudge_count = prior.nudge_count.saturating_add(1);
    }
}

/// The R4 escalation of a quiet `waiting-agent`, or `None` while its hold
/// stands. The ceiling measures `seen.declared_age_secs`, the age of the
/// declaration the hold was armed from: the seat's own later traffic leaves
/// the hold standing, so the newest own event is not that declaration.
///
/// Past the ceiling the seat becomes exactly `blocked`: the verdict published
/// is `Quiet(Blocked)` — the attention half, on the same ceiling the read
/// surfaces derive from the same pin. The nudge budget resumes (`book_stale`,
/// off when `idle_nudge_secs` is zero) UNLESS an active wait episode owns the
/// next delivery — then the challenge is the one order; only a lapsed or
/// untracked episode resumes the budget.
fn book_waiting_agent_escalation(
    prior: &PaneState,
    next: &mut PaneState,
    effects: &mut Vec<Effect>,
    seen: &Observation,
    knobs: &Knobs,
) -> Option<Verdict> {
    if !waiting_agent_escalates(seen, knobs) {
        return None;
    }
    // An ACTIVE wait episode owns the next delivery: challenge, not nudge. A
    // lapsed or untracked episode resumes the ordinary budget instead.
    let active = matches!(
        seen.wait_progress,
        WaitProgress::Provisional { .. }
            | WaitProgress::ChallengeDue { .. }
            | WaitProgress::Challenged { .. }
    );
    if knobs.idle_nudge_secs > 0 && !active {
        book_stale(prior, next, effects, knobs, seen.last_actor_event_age_secs);
    }
    Some(Verdict::Quiet(QuietKind::Blocked))
}

/// The owed move of a fresh or escalated `waiting-agent`'s `--on` target: its
/// paste, bounded as a challenge is.
fn book_target_moved(
    seen: &Observation,
    knobs: &Knobs,
    wait_state: Option<WaitState>,
) -> Vec<Effect> {
    let Some(found) = seen.target_moved.as_ref().filter(|found| {
        wait_state == Some(WaitState::WaitingAgent) && found.ordinal > found.booked
    }) else {
        return Vec::new();
    };
    let age = age_secs(seen.now_epoch, found.at.epoch());
    let told = Effect::TargetMoved {
        moved: found.clone(),
        age_secs: age,
    };
    bounded(found.attempts, seen, knobs, told, age)
}

/// Whether this cycle's quiet `waiting-agent` is past its ceiling — the one
/// answer the escalated verdict and the challenge text both read.
fn waiting_agent_escalates(seen: &Observation, knobs: &Knobs) -> bool {
    seen.quiet == Some(QuietKind::WaitingAgent)
        && !seen.wait_ends_elsewhere
        && crate::watchdog::waiting_agent_escalated_at(
            seen.declared_age_secs,
            knobs.idle_nudge_secs,
            seen.wait_rung,
        )
}

/// The orchestrator main's sweep branch, or `None` when this pane is not it.
fn book_sweep(
    prior: &PaneState,
    next: &mut PaneState,
    effects: &mut Vec<Effect>,
    seen: &Observation,
    knobs: &Knobs,
) -> Option<Verdict> {
    let booked = seen
        .sweep
        .as_ref()
        .and_then(|observed| sweep_step(&prior.sweep, observed, &knobs.sweep))?;
    next.sweep = booked.next;
    effects.extend(sweep_effects(booked.effects));
    Some(Verdict::Meta(booked.verdict))
}

/// The TWO ends a death latch has: a POSITIVE process-tree reading shows the
/// seat's harness back under the pane (the human's re-run in place), or the
/// seat's own later journal activity cleared the alert — a dead agent journals
/// nothing, so its own record is proof of life the process name cannot give.
/// An UNKNOWN snapshot is not evidence of life, so a probe gap — and a
/// reading that says the process is still gone — keeps the latch and returns
/// `None`. On a clear it emits the one `dead-cleared` and hands back the
/// episode the ordinary judgement must run on: identity kept, and every clock,
/// hash and nudge field the death interrupted reset, because a pre-death hash
/// or idle clock must not feed a stale verdict. No hysteresis and no
/// second-alert suppression: the caller alerts again if the seat dies again,
/// because that is a real event each time.
fn clear_death_latch(
    prior: &PaneState,
    next: &PaneState,
    seen: &Observation,
    effects: &mut Vec<Effect>,
) -> Option<PaneState> {
    if !prior.dead_latched {
        return None;
    }
    let back = matches!(seen.descendancy, Descendancy::Present) || seen.outstanding_alert.is_none();
    if !back {
        return None;
    }
    effects.push(Effect::Emit {
        action: "dead-cleared",
        summary: "agent process back — resumed in place".to_owned(),
    });
    effects.push(Effect::Notify("is BACK — process resumed".to_owned()));
    Some(PaneState {
        dead_latched: false,
        prev_hash: None,
        last_hash_change: None,
        idle_since_epoch: None,
        nudge_count: 0,
        undelivered_streak: 0,
        throttle_streak: 0,
        limit_since: None,
        limit_reset: None,
        limit_wake_attempts: 0,
        limit_armed: None,
        human_prompt_streak: 0,
        proof_frame: None,
        task_since: None,
        ..next.clone()
    })
}

/// Account for one pane in one cycle — the branch order, and the only place
/// any of it is decided.
#[must_use]
pub fn account(prior: &PaneState, seen: &Observation, knobs: &Knobs) -> Accounting {
    account_with_trust(prior, seen, knobs, None)
}

/// [`account`], with [`crate::autotrust::due`]'s answer for the trust block
/// this cycle's prompt is — `None` for any other prompt, no prompt, and
/// `auto_trust` off, which is exactly [`account`].
#[must_use]
pub fn account_with_trust(
    prior: &PaneState,
    seen: &Observation,
    knobs: &Knobs,
    trust: Option<&crate::autotrust::Due>,
) -> Accounting {
    account_with_activity(prior, seen, knobs, trust, &Activity::default())
}

/// [`account_with_trust`], with the cycle's [`Activity`]: the proof holds
/// (#225) and the task lease (#283). `Activity::default()` is exactly
/// [`account_with_trust`].
#[must_use]
pub fn account_with_activity(
    prior: &PaneState,
    seen: &Observation,
    knobs: &Knobs,
    trust: Option<&crate::autotrust::Due>,
    activity: &Activity,
) -> Accounting {
    let reset = PaneState::default();
    let mut prior = if prior
        .identity
        .is_some_and(|identity| identity != seen.identity)
    {
        &reset
    } else {
        prior
    };
    let mut next = prior.clone();
    next.identity = Some(seen.identity);
    next.task_since = task_lease(prior.task_since, activity.task_running, seen.now_epoch);
    let mut effects = Vec::new();
    book_unknown(&mut next, &mut effects, seen.descendancy);
    if seen.failed_elsewhere {
        effects.push(Effect::Emit {
            action: "alert-cleared",
            summary: format!("{} — new tool start", crate::events::REACHABLE_AGAIN),
        });
    }

    // 1. Already dead: no second alert, no further judgement — until the one
    //    clear rule holds (`clear_death_latch`), which this cycle then judges.
    let unlatched;
    match clear_death_latch(prior, &next, seen, &mut effects) {
        Some(state) => {
            unlatched = state;
            prior = &unlatched;
            next = unlatched.clone();
        }
        None if prior.dead_latched => {
            return Accounting {
                next,
                effects,
                verdict: Verdict::Dead,
                moved: false,
            };
        }
        None => {}
    }

    // 2. The shell an auto reseat's respawn leaves is the move, not a death;
    //    and a shell read while the launch still settles is a seat starting.
    let graced = launch_grace(seen.launch_attempt, seen.now_epoch, knobs.interval_secs);
    if seen.is_dead && seen.auto_in_flight.is_none() && !graced {
        next.dead_latched = true;
        effects.push(Effect::Emit {
            action: "alert",
            summary: "agent process dead — dropped to shell".to_owned(),
        });
        effects.push(Effect::Notify(
            "is DEAD — process dropped to shell".to_owned(),
        ));
        return Accounting {
            next,
            effects,
            verdict: Verdict::Dead,
            moved: false,
        };
    }

    let mut accounted = account_ordinary(prior, next, effects, (seen, trust, activity), knobs);
    // The ONE unreachable failure outranks every verdict but dead. A prompt
    // keeps its own here for the brief and notice gates; the cycle publishes
    // it as unreachable.
    if seen.unreachable
        && !matches!(
            accounted.verdict,
            Verdict::Dead | Verdict::Meta(_) | Verdict::HumanPrompt
        )
    {
        accounted.verdict = Verdict::Unreachable;
    }
    accounted
}

/// Steps 3 through 10 — the ordinary judgement of a pane that is not dead (any
/// more). `prior` is the carry the verdict is judged against: the reset
/// episode a clear just returned, or the standing carry.
#[allow(
    clippy::too_many_lines,
    reason = "ordered accounting ladder mirrors the documented watchdog steps"
)]
fn account_ordinary(
    prior: &PaneState,
    mut next: PaneState,
    mut effects: Vec<Effect>,
    (seen, trust, activity): (&Observation, Option<&crate::autotrust::Due>, &Activity),
    knobs: &Knobs,
) -> Accounting {
    // The pane half of "unchanged since the proof" (#225), every cycle.
    let quiet_proof = activity.proof.unwrap_or_default();
    let frame = match activity.proof {
        None => FrameVerdict::Moved,
        Some(proof) => {
            let settle = proof
                .settle
                .map(crate::time::Timestamp::epoch)
                .max(activity.painting);
            let sample = FrameSample {
                capture_ok: seen.capture_ok,
                busy: seen.harness.frame == HarnessState::Busy,
                hash: seen.hash,
            };
            let (carried, verdict) = proof_frame_step(
                prior.proof_frame,
                proof.proof.map(crate::time::Timestamp::epoch),
                settle,
                sample,
                seen.now_epoch,
                knobs.interval_secs,
            );
            next.proof_frame = carried;
            verdict
        }
    };

    // 3.
    if let Some(verdict) = book_sweep(prior, &mut next, &mut effects, seen, knobs) {
        next.idle_since_epoch = None;
        return Accounting {
            next,
            effects,
            verdict,
            moved: false,
        };
    }

    // 4. A declaration starts a new idle episode exactly once. Its fingerprint
    // rides the observed carry so a daemon restart cannot spend the reset again.
    if let Some(declaration) = seen.harness.declaration
        && prior.last_declaration != Some(declaration)
    {
        next.last_declaration = Some(declaration);
        next.idle_since_epoch = None;
        next.nudge_count = 0;
        next.undelivered_streak = 0;
    }

    // 5.
    if seen.throttle.is_none() && prior.throttle_streak > 0 {
        effects.push(Effect::Emit {
            action: "throttle-cleared",
            summary: format!("throttling cleared after {} cycles", prior.throttle_streak),
        });
        next.throttle_streak = 0;
    }

    // 5b. The limit latch's ONE release: this cycle READ the pane, judged it,
    //     and the phrase is gone, so the verdict is retracted and recovery
    //     requested. A FAILED capture is an absence of evidence, not evidence
    //     of absence: the latch holds and the verdict stays `limit` below. An
    //     auto reseat in flight holds it too: its shell is no release.
    if prior.limit_since.is_some()
        && seen.capture_ok
        && seen.throttle != Some(Throttle::LimitReached)
        && seen.auto_in_flight != Some(crate::autoreseat::Trigger::Limit)
    {
        next.limit_since = None;
        // The episode's reset leaves with it; the spent mark stays, so a
        // re-hit on the same reset is not woken twice.
        next.limit_reset = None;
        next.limit_wake_attempts = 0;
        next.limit_armed = None;
        effects.push(Effect::Emit {
            action: "alert-cleared",
            summary: "usage limit cleared — pane no longer shows it".to_owned(),
        });
        effects.push(Effect::QuotaRefresh);
    }

    // 5c. The human-prompt latch's ONE release, on 5b's rule: a cycle that
    //     READ the pane and no longer shows the prompt. A FAILED capture is an
    //     absence of evidence — clearing on it would flap the latch with
    //     clear-and-relatch pairs while a live modal sits untouched.
    //     A restarted daemon clears the naming it did not count.
    if seen.capture_ok && seen.human_prompt.is_none() {
        if prior.human_prompt_streak >= knobs.human_prompt_cycles || seen.prompt_named {
            effects.push(Effect::Emit {
                action: "human-prompt-cleared",
                summary: "the human-only prompt is gone from the pane".to_owned(),
            });
        }
        next.human_prompt_streak = 0;
        next.prompt_start = None;
    }

    if let DoneProgress::ChallengeDue {
        confirmations,
        required,
        done_age_secs,
        attempts,
    } = seen.done_progress
    {
        if done_held_due(seen.done_progress, quiet_proof, frame) {
            effects.push(Effect::DoneHeld {
                confirmations,
                required,
            });
        } else {
            let challenge = Effect::DoneChallenge {
                confirmations,
                required,
                done_age_secs,
            };
            effects.extend(bounded(attempts, seen, knobs, challenge, done_age_secs));
        }
    }

    // The wait push, gated on the pane hold; the state comes from the hold
    // itself, so the challenge always names the RAW declaration.
    let wait_state = match seen.quiet {
        Some(QuietKind::WaitingAgent) => Some(WaitState::WaitingAgent),
        Some(QuietKind::Blocked) => Some(WaitState::Blocked),
        _ => None,
    };
    // A moved `--on` target is told once per transition, and the challenge
    // yields only to a paste that is actually attempted this cycle. The ONE
    // unreachable alert is booked once a cycle: when the move booked it, the
    // challenge only pastes.
    let moves = book_target_moved(seen, knobs, wait_state);
    let pasting = moves
        .iter()
        .any(|effect| matches!(effect, Effect::TargetMoved { .. }));
    let alerted = !moves.is_empty() && !pasting;
    effects.extend(moves);
    let held = wait_held(
        seen.wait_progress,
        quiet_proof,
        frame,
        knobs.idle_nudge_secs,
    );
    if !pasting
        && !held
        && let (
            Some(wait_state),
            WaitProgress::ChallengeDue {
                confirmations,
                required,
                wait_age_secs,
                attempts,
            },
        ) = (wait_state, seen.wait_progress)
    {
        let challenge = Effect::WaitChallenge {
            confirmations,
            required,
            wait_age_secs,
            state: wait_state,
            escalated: waiting_agent_escalates(seen, knobs),
        };
        let mut waits = bounded(attempts, seen, knobs, challenge, wait_age_secs);
        if alerted {
            waits.retain(|effect| matches!(effect, Effect::WaitChallenge { .. }));
        }
        effects.extend(waits);
    }

    // 6. A quiet declaration. A FRESH `waiting-agent` holds like the other
    // quiet states; past its ceiling, measured from the declaration itself,
    // it escalates (see the helper). The declaration keeps the bar over a
    // human-only prompt, but the prompt is still BOOKED under it: named once
    // per episode, so a seat stuck on a modal still summons the human.
    if let Some(kind) = seen.quiet {
        book_human_prompt(prior, &mut next, &mut effects, (seen, trust), knobs);
        // A usage limit FIRST SEEN no earlier than the declaration outranks it
        // and ends nothing: its release hands the bar straight back. One that
        // was already showing when the seat declared stays under it. A tie
        // goes to the human.
        if limited(prior, seen)
            && seen
                .suspended
                .awake_secs(prior.limit_since.unwrap_or(seen.now_epoch), seen.now_epoch)
                <= seen.declared_age_secs
        {
            book_limit(&mut next, &mut effects, seen);
            book_limit_wake(&mut next, &mut effects, seen, knobs);
            next.undelivered_streak = 0;
            return Accounting {
                next,
                effects,
                verdict: Verdict::Limit,
                moved: false,
            };
        }
        if let Some(verdict) =
            book_waiting_agent_escalation(prior, &mut next, &mut effects, seen, knobs)
        {
            return Accounting {
                next,
                effects,
                verdict,
                moved: false,
            };
        }
        // A lapsed wait keeps its verdict while the nudge budget resumes.
        // Escalation returned above, so this arm books exactly once.
        if matches!(kind, QuietKind::WaitingAgent | QuietKind::Blocked)
            && matches!(seen.wait_progress, WaitProgress::Lapsed { .. })
        {
            book_stale(
                prior,
                &mut next,
                &mut effects,
                knobs,
                seen.last_actor_event_age_secs,
            );
            return Accounting {
                next,
                effects,
                verdict: Verdict::Quiet(kind),
                moved: false,
            };
        }
        next.nudge_count = 0;
        next.undelivered_streak = 0;
        next.idle_since_epoch = None;
        return Accounting {
            next,
            effects,
            verdict: Verdict::Quiet(kind),
            moved: false,
        };
    }

    // 7. The vendor's usage limit outranks a transient throttle. A latched seat
    //    whose capture FAILED this cycle keeps the verdict too: no reading is
    //    not a clearing. So does a seat an auto reseat is moving, latched
    //    again by a daemon that restarted mid-move.
    if limited(prior, seen) || seen.auto_in_flight == Some(crate::autoreseat::Trigger::Limit) {
        book_limit(&mut next, &mut effects, seen);
        book_limit_wake(&mut next, &mut effects, seen, knobs);
        return Accounting {
            next,
            effects,
            verdict: Verdict::Limit,
            moved: false,
        };
    }

    // 8.
    if seen.throttle.is_some() {
        book_throttle(&mut next, &mut effects, seen, knobs);
        return Accounting {
            next,
            effects,
            verdict: Verdict::Throttled,
            moved: false,
        };
    }

    // 8.5. A prompt only the HUMAN may answer. It sits BELOW dead, the sweep,
    //      a declaration and the two vendor verdicts — all of which are worse
    //      news — and ABOVE the harness frames, because those are where `Stale`
    //      is decided and a seat waiting on a modal is silent BY NATURE. If
    //      stale won here the feature would never draw.
    if let Some(prompt) = book_human_prompt(prior, &mut next, &mut effects, (seen, trust), knobs) {
        return Accounting {
            next,
            effects,
            verdict: prompt,
            moved: false,
        };
    }

    // 9. Harness frames outrank the legacy motion heuristic.
    if let Some(verdict) = account_harness(prior, &mut next, &mut effects, (seen, activity), knobs)
    {
        return Accounting {
            next,
            effects,
            verdict,
            moved: false,
        };
    }

    // 10. Unknown frames retain the legacy motion and actor-event rule.
    account_unknown(prior, next, effects, (seen, activity), knobs)
}

/// Positive harness-frame branches. `None` means the caller must use the
/// conservative legacy heuristic.
fn account_harness(
    prior: &PaneState,
    next: &mut PaneState,
    effects: &mut Vec<Effect>,
    (seen, activity): (&Observation, &Activity),
    knobs: &Knobs,
) -> Option<Verdict> {
    // A human draft is recovery of the idle episode but remains Unknown as an
    // execution fact.
    if seen.harness.human_draft {
        next.idle_since_epoch = None;
        next.nudge_count = 0;
        next.undelivered_streak = 0;
        return Some(if seen.harness.durable_stale {
            Verdict::Stale
        } else {
            Verdict::Active
        });
    }
    match seen.harness.frame {
        HarnessState::Busy => {
            next.prev_hash = Some(seen.hash);
            next.last_hash_change = Some(seen.now_epoch);
            next.idle_since_epoch = None;
            next.nudge_count = 0;
            next.undelivered_streak = 0;
            // Motion is no delivery: an unreachable failure stands.
            if seen.harness.durable_stale && !seen.unreachable {
                effects.push(Effect::Emit {
                    action: "alert-cleared",
                    summary: "agent busy again — stale alert cleared".to_owned(),
                });
            }
            Some(Verdict::Active)
        }
        HarnessState::Idle => {
            next.prev_hash = Some(seen.hash);
            let idle_since = next.idle_since_epoch.get_or_insert(seen.now_epoch);
            // Awake seconds: a host suspension is not idle time (I15).
            let idle_age = seen.suspended.awake_secs(*idle_since, seen.now_epoch);
            if seen.harness.durable_stale || next.nudge_count > knobs.max_nudges {
                return Some(Verdict::Stale);
            }
            // An empty input box is the right reading of the PIXELS and the
            // wrong reading of the FACTS when the seat is the one everybody
            // else is waiting on. Defer the nudge; never suppress it for good.
            if knobs.idle_nudge_secs > 0
                && idle_age >= knobs.idle_nudge_secs
                && !deferred(seen.own_work, seen.now_epoch, idle_age, knobs)
                && !task_deferred(
                    next.task_since,
                    activity.task_running,
                    seen.now_epoch,
                    knobs,
                )
            {
                book_stale(prior, next, effects, knobs, idle_age);
            }
            Some(if next.nudge_count > knobs.max_nudges {
                Verdict::Stale
            } else {
                Verdict::Idle
            })
        }
        HarnessState::Unknown => {
            if seen.harness.durable_stale {
                Some(Verdict::Stale)
            } else {
                None
            }
        }
    }
}

/// The pre-frame watchdog heuristic, retained for unsupported or ambiguous
/// current frames.
fn account_unknown(
    prior: &PaneState,
    mut next: PaneState,
    mut effects: Vec<Effect>,
    (seen, activity): (&Observation, &Activity),
    knobs: &Knobs,
) -> Accounting {
    let hash_unchanged = prior.prev_hash == Some(seen.hash);
    if !hash_unchanged {
        next.prev_hash = Some(seen.hash);
        next.last_hash_change = Some(seen.now_epoch);
        if prior.idle_since_epoch.is_none() {
            next.nudge_count = 0;
            next.undelivered_streak = 0;
        }
        return Accounting {
            next,
            effects,
            verdict: Verdict::Active,
            // THE motion signal: this cycle's capture differs from the last.
            moved: true,
        };
    }

    let hash_change_age = prior
        .last_hash_change
        .map_or(u64::MAX, |at| age_secs(seen.now_epoch, at));
    let stale = stale_composite(
        hash_unchanged,
        hash_change_age,
        seen.last_actor_event_age_secs,
        knobs.stale_secs,
        false, // the quiet branch already returned
        false, // and so did the throttled one
    );
    // A live task of the seat's tool is activity the frame cannot show
    // (#283), within its lease.
    if !stale
        || task_deferred(
            next.task_since,
            activity.task_running,
            seen.now_epoch,
            knobs,
        )
    {
        return Accounting {
            next,
            effects,
            verdict: Verdict::Active,
            moved: false,
        };
    }

    book_stale(
        prior,
        &mut next,
        &mut effects,
        knobs,
        seen.last_actor_event_age_secs,
    );
    Accounting {
        next,
        effects,
        verdict: Verdict::Stale,
        moved: false,
    }
}

/// One pane's text and whether the READ succeeded. The main loop tolerates a
/// failed capture (an unreadable pane hashes as empty), but the text of a
/// failed read is an absence of evidence, not an absence of the phrase — the
/// usage-limit latch may not clear on it.
fn capture_pane(server: &crate::inventory::ServerId, pane: &str) -> (String, bool) {
    match transport::capture_pane(server, pane) {
        Some(text) => (text, true),
        None => (String::new(), false),
    }
}

/// Restore only the idle clock fields owned by the public observed option.
/// The identity guard prevents a reused pane id inheriting another seat's clock.
fn restore_idle(state: &mut PaneState, raw: &str, identity: u64) {
    if state.identity.is_some() {
        return;
    }
    let Some(carry) =
        crate::harness_state::decode_idle(raw).filter(|carry| carry.identity == identity)
    else {
        return;
    };
    state.identity = Some(identity);
    state.idle_since_epoch = Some(carry.since_epoch);
    state.nudge_count = carry.nudges;
    state.undelivered_streak = carry.undelivered;
    state.last_declaration = carry.declaration;
}

/// Publish the current frame and, for idle, enough episode state to survive a
/// daemon restart without inventing a second observation call.
fn observed_option(frame: HarnessState, state: &PaneState) -> String {
    if frame == HarnessState::Busy {
        return frame.as_str().to_owned();
    }
    match (state.idle_since_epoch, state.identity) {
        (Some(since_epoch), Some(identity)) => crate::harness_state::encode_carry(
            frame,
            crate::harness_state::IdleCarry {
                since_epoch,
                nudges: state.nudge_count,
                undelivered: state.undelivered_streak,
                identity,
                declaration: state.last_declaration,
            },
        ),
        _ => frame.as_str().to_owned(),
    }
}

/// Render the sweep layer's decisions as this loop's effects.
fn sweep_effects(booked: Vec<SweepEffect>) -> Vec<Effect> {
    let mut out = Vec::new();
    for effect in booked {
        match effect {
            SweepEffect::FireSweepNudge => out.push(Effect::SweepNudge),
            SweepEffect::ReconcileWedge => out.push(Effect::ReconcileWedge),
            SweepEffect::Alert(alert) => {
                out.push(Effect::Emit {
                    action: alert.action(),
                    summary: alert.summary(),
                });
                if let Some(text) = alert.notify() {
                    out.push(Effect::Notify(text.to_owned()));
                }
            }
        }
    }
    out
}

/// Whether one enumerated pane still HOLDS its seat — the deferral's half of
/// the liveness question, decided on this cycle's own evidence and no probe of
/// its own.
///
/// Two conjuncts, each with an owner elsewhere, because a named pane is not a
/// working agent: [`classify_dead`] is the watchdog's own Dead verdict, and the
/// bare-shell test is the second conjunct `liveness::pane_alive` already
/// renders in `ae list` — so the seat the publisher defers for and the seat the
/// human is shown are the same seat. A spawned tool that exited into its
/// retained shell holds nothing, and its owner is not excused by it.
///
/// UNKNOWN POLICY, and it is TWO cases rather than one blanket rule, because
/// the pane's own foreground command is read first and already decides most of
/// them:
///
/// - a pane at a BARE SHELL holds no seat whatever the snapshot says, so an
///   uncertain snapshot never rescues it and its owner keeps its nudge;
/// - a pane running a FOREGROUND TOOL keeps its seat when the snapshot cannot
///   confirm the process, because `classify_dead` demands positive absence and
///   a probe gap is not proof of death. That deferral is RETAINED, not
///   unbounded: [`deferred`]'s two clocks end it like any other.
#[must_use]
fn holds_seat(current_command: &str, descendancy: Descendancy) -> bool {
    !classify_dead(current_command, descendancy)
        && !crate::watchdog::command_is_shell(current_command)
}

/// The seats this enumeration proves are still held, by [`holds_seat`].
///
/// The monitor panes are not agents and never hold anybody's work.
#[must_use]
fn held_seats(
    observed: &[crate::tmux::WatchPane],
    table: Option<&[procs::Proc]>,
    bin_of: &impl Fn(&str) -> Option<String>,
) -> Vec<String> {
    observed
        .iter()
        .filter_map(|pane| {
            let agent = pane
                .agent
                .as_deref()
                .filter(|agent| !agent.is_empty() && !NON_AGENT_PANES.contains(agent))?;
            let bin = bin_of(pane.slot.as_deref().unwrap_or_default());
            let descendancy = descendancy_of(table, pane.pane_pid, bin.as_deref());
            holds_seat(&pane.current_command, descendancy).then(|| agent.to_owned())
        })
        .collect()
}

/// Whether a seat that reached its nudge age keeps its quiet a while longer.
///
/// Suppression is a DEFERRAL, never silence. A seat with outstanding own work
/// spends the SAME bounded nudge budget as any other; it just starts spending
/// it later, so a genuinely wedged lead is still caught. Two ceilings end the
/// deferral, whichever comes first:
///
/// - the idle episode has run as long as the whole nudge budget would have
///   taken — one deferred nudge opportunity per `idle_nudge_secs`, `max_nudges`
///   of them — measured on the clock rather than in a counter, so a daemon
///   restart cannot forget it;
/// - the oldest outstanding item is older than [`crate::watchdog::OWN_WORK_AGE_CAP`]
///   nudge periods, which is the case where the seat's own work has itself gone
///   wrong. `waiting-agent` escalation reuses the same MULTIPLIER, with ONE
///   stated exception: at `idle_nudge_secs == 0` this deferral is vacuous
///   (there is no nudge to defer) while the attention ceiling scales from the
///   documented default ([`crate::watchdog::waiting_agent_cap_secs`]), because
///   zero keeps the marker and only suppresses the nudge.
///
/// PURE: it reads the observation and the knobs and nothing else.
#[must_use]
fn deferred(own: crate::session::OwnWork, now_epoch: i64, idle_age: u64, knobs: &Knobs) -> bool {
    if !own.outstanding() || knobs.idle_nudge_secs == 0 {
        return false;
    }
    let budget = knobs
        .idle_nudge_secs
        .saturating_mul(u64::from(knobs.max_nudges).saturating_add(1));
    let cap = knobs
        .idle_nudge_secs
        .saturating_mul(crate::watchdog::OWN_WORK_AGE_CAP);
    idle_age < budget && own.oldest_secs(now_epoch) < cap
}

/// Book a nudge attempt's outcome.
#[must_use]
pub fn record_nudge(
    state: &mut PaneState,
    delivered: bool,
    knobs: &Knobs,
    display: &str,
) -> Vec<Effect> {
    if delivered {
        state.nudge_count = state.nudge_count.saturating_add(1);
        state.undelivered_streak = 0;
        return Vec::new();
    }
    state.undelivered_streak = state.undelivered_streak.saturating_add(1);
    if state.undelivered_streak == knobs.undelivered_max {
        return unreachable_effects(state.undelivered_streak, display);
    }
    Vec::new()
}

/// A due proof challenge at `attempts`: tried below the bound; AT it the ONE
/// unreachable failure is booked unless it stands; at or past it the challenge
/// is tried again only at a drawn, empty, idle composer — the recovery.
fn bounded(
    attempts: u32,
    seen: &Observation,
    knobs: &Knobs,
    challenge: Effect,
    age_secs: u64,
) -> Vec<Effect> {
    let idle = seen.harness.frame == HarnessState::Idle && !seen.harness.human_draft;
    if attempts < knobs.undelivered_max || (seen.unreachable && idle) {
        vec![challenge]
    } else if attempts == knobs.undelivered_max && !seen.unreachable {
        unreachable_effects(attempts, &stale_display(age_secs))
    } else {
        Vec::new()
    }
}

fn unreachable_effects(attempts: u32, display: &str) -> Vec<Effect> {
    vec![
        Effect::Emit {
            action: "alert",
            summary: format!(
                "{} — {attempts} undelivered attempts ({display})",
                crate::events::UNREACHABLE
            ),
        },
        Effect::Notify(format!(
            "unreachable — {attempts} nudges could not be delivered"
        )),
    ]
}

/// Seconds between `at` and `now`, clamped at zero.
#[must_use]
pub fn age_secs(now_epoch: i64, at_epoch: i64) -> u64 {
    u64::try_from(now_epoch.saturating_sub(at_epoch)).unwrap_or(0)
}

/// The age of the newest event this SEAT is the actor of — judged by the ONE
/// routing-aware actor rule ([`crate::watchdog::event_is_actor`]), so a
/// same-display event from another incarnation is not this seat's activity.
/// Staleness reads it; a `waiting-agent` ceiling reads [`declaration_age`].
#[must_use]
pub fn last_actor_event_age(
    events: &[Event],
    session: &str,
    slot: &str,
    agent: &str,
    now_epoch: i64,
) -> u64 {
    events
        .iter()
        .rev()
        .find(|event| crate::watchdog::event_is_actor(event, session, slot, agent))
        .map_or(NO_EVENT_AGE, |event| age_secs(now_epoch, event.ts.epoch()))
}

/// The AWAKE age of this seat's newest declaration — its wall age minus the
/// journal's host suspensions ([`crate::watchdog::Suspensions`]) — or
/// [`NO_EVENT_AGE`] when it has none: the declaration
/// [`crate::session::latest_declaration_in`] finds for `ae list` too, so both
/// surfaces escalate a wait on the same second.
#[must_use]
pub fn declaration_age(
    events: &[Event],
    session: &str,
    slot: &str,
    agent: &str,
    now_epoch: i64,
) -> u64 {
    crate::session::latest_declaration_in(events, session, slot, agent)
        .map_or(NO_EVENT_AGE, |event| {
            crate::watchdog::Suspensions::read(events).awake_secs(event.ts.epoch(), now_epoch)
        })
}

/// The newest `state done` acknowledgement by `agent`, as a wall-clock value
/// the pure sweep accounting can compare with the delivered overview time.
fn last_done_event_at(events: &[Event], session: &str, agent: &str) -> Option<SystemTime> {
    let epoch = events
        .iter()
        .rev()
        .find(|event| main_actor(event, session, agent) && event.declared_state() == Some("done"))?
        .ts
        .epoch();
    system_time_from_epoch(epoch)
}

/// Whether an event belongs to this session's main seat. New routed records
/// use slot + session; display identity keeps old event logs readable.
fn main_actor(event: &Event, session: &str, agent: &str) -> bool {
    crate::watchdog::event_is_actor(event, session, crate::watchdog::MAIN_SLOT, agent)
}

/// The current `working` declaration's wall clock, if the orchestrator main's
/// newest declaration is still `working`.
fn last_working_declaration_at(events: &[Event], session: &str, agent: &str) -> Option<SystemTime> {
    let latest = events
        .iter()
        .rev()
        .find(|event| main_actor(event, session, agent) && event.declared_state().is_some())?;
    if latest.declared_state() != Some("working") {
        return None;
    }
    system_time_from_epoch(latest.ts.epoch())
}

fn system_time_from_epoch(epoch: i64) -> Option<SystemTime> {
    if epoch >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(u64::try_from(epoch).ok()?))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(epoch.unsigned_abs()))
    }
}

fn epoch_second(at: SystemTime) -> i64 {
    match at.duration_since(UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        Err(before) => {
            i64::try_from(before.duration().as_secs()).map_or(i64::MIN, i64::saturating_neg)
        }
    }
}

/// The age reported for an agent with no event at all.
pub const NO_EVENT_AGE: u64 = 999_999;

// ---------------------------------------------------------------------------
// The loop — observation and effects.

/// The generated helper a nudge is delivered through.
const HELPER_NAME: &str = "send";

/// The message word the brief-retry exec carries. It reaches nothing: the
/// helper takes the brief's text and its actor from the durable record, and
/// this exists only because the helper's argv grammar needs a word.
const RETRY_PLACEHOLDER: &str = "brief-retry";

/// The message word the auto reseat trigger carries, for the same reason: the
/// trigger reads every fact it acts on for itself.
const AUTO_PLACEHOLDER: &str = "auto-reseat";

/// The summary an attempt past its bound is closed with.
const OVERDUE_SUMMARY: &str = "failed: no outcome recorded";

/// The orchestrator watchdog's checkpoint and heartbeat, at the FIXED name
/// `<meta-dir>/meta-agent-state.json`.
pub(crate) const HEARTBEAT_NAME: &str = "meta-agent-state.json";

/// The normal verdict interval and smallest useful positive overview spacing.
/// Zero remains the explicit off switch.
const MIN_SWEEP_SECS: u64 = 60;

/// The process-wide fallback for an orchestrator session with no persisted
/// `sweep_sec`. Kept as a door at its only use site: launch-time config is the
/// durable owner, while this remains useful for development and recovery.
fn sweep_env() -> Option<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: AE_WATCHDOG_SWEEP_SEC is the documented watchdog-wide sweep fallback"
    )]
    let raw = std::env::var_os("AE_WATCHDOG_SWEEP_SEC");
    raw.and_then(|value| value.into_string().ok())
}

/// Resolve overview spacing in authority order: the session's persisted launch
/// fact, the watchdog-wide environment fallback, then the supplied internal
/// knob (120 in production defaults; CLI flags may override it in tests).
fn sweep_seconds(meta_bytes: &[u8], env: Option<&str>, fallback: u64) -> u64 {
    let resolved = crate::meta::sole_value(meta_bytes, "sweep_sec")
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| env.and_then(|value| value.parse::<u64>().ok()))
        .unwrap_or(fallback);
    if resolved == 0 {
        0
    } else {
        resolved.max(MIN_SWEEP_SECS)
    }
}

/// Resolve the session-pinned quota cadence before the internal flag/default.
pub(crate) fn quota_seconds(meta_bytes: &[u8], fallback: u64) -> Result<u64, String> {
    pinned_seconds(meta_bytes, "quota_every_secs", fallback)
}

/// Resolve the session-pinned quota awareness through the ONE
/// `config::resolve_quota_aware` precedence: pin, else live config, else ON.
/// Absent everywhere means ON — exactly today's behaviour — for sessions that
/// pre-date the knob. The live-config fallback is what lets a pre-knob session
/// honour the setting at all; a running session keeps its pin.
fn quota_awareness(meta_bytes: &[u8], global: Option<&Path>, local: Option<&Path>) -> bool {
    let pinned = crate::meta::first_value(meta_bytes, "quota")
        .map(|value| String::from_utf8_lossy(value).into_owned());
    let configured = crate::config::read_workspace_keys(global, local, &["quota"])
        .into_iter()
        .next()
        .flatten();
    crate::config::resolve_quota_aware(pinned.as_deref(), configured.as_deref())
}

/// Resolve the session-pinned idle reminder cadence before the flag/default.
///
/// THE rule is [`crate::meta::resolve_idle_nudge_secs`] — the read surfaces
/// call the same function, so a hostile meta can never give the pane and the
/// human surface two different cadences. This wrapper only extracts the
/// resolver's inputs from the raw bytes and keeps the daemon's loud refusal.
fn idle_nudge_seconds(meta_bytes: &[u8], fallback: u64) -> Result<u64, String> {
    let sole = crate::meta::sole_value(meta_bytes, "idle_nudge_secs");
    let recorded = sole.map(|value| String::from_utf8_lossy(value).into_owned());
    let doubled =
        recorded.is_none() && crate::meta::first_value(meta_bytes, "idle_nudge_secs").is_some();
    crate::meta::resolve_idle_nudge_secs(recorded.as_deref(), doubled, fallback).ok_or_else(|| {
        let raw = sole
            .or_else(|| crate::meta::first_value(meta_bytes, "idle_nudge_secs"))
            .unwrap_or_default();
        String::from_utf8_lossy(raw).into_owned()
    })
}

fn pinned_seconds(meta_bytes: &[u8], key: &str, fallback: u64) -> Result<u64, String> {
    let Some(raw) = crate::meta::sole_value(meta_bytes, key) else {
        return match crate::meta::first_value(meta_bytes, key) {
            Some(value) => Err(String::from_utf8_lossy(value).into_owned()),
            None => Ok(fallback),
        };
    };
    let value = String::from_utf8_lossy(raw);
    value.parse::<u64>().map_err(|_| value.into_owned())
}

fn quota_delivery(delivery: &transport::Delivery) -> QuotaDelivery {
    if delivery.code == Some(0) {
        QuotaDelivery::Delivered
    } else if delivery
        .stdout
        .lines()
        .any(|line| line == crate::send::RETRYABLE_MARKER)
    {
        QuotaDelivery::Retryable
    } else {
        QuotaDelivery::Uncertain
    }
}

/// The session's own send helper, at the FIXED path `<meta-dir>/send`.
struct SendHelper(std::path::PathBuf);

impl SendHelper {
    /// `<meta-dir>/send`, and nothing else.
    fn for_session(meta_dir: &Path) -> Self {
        Self(meta_dir.join(HELPER_NAME))
    }

    /// The path to spawn.
    fn path(&self) -> &Path {
        &self.0
    }
}

/// What one pass counted, gathered so the publish call stays one statement.
#[derive(Default)]
struct Counts {
    /// Panes that were neither dead nor stale.
    active: usize,
    /// Panes judged at all.
    total: usize,
    /// Panes whose process is gone.
    dead: usize,
    /// Panes silent past the window.
    stale: usize,
}

impl Counts {
    fn record(&mut self, verdict: Verdict) {
        self.total += 1;
        match verdict {
            Verdict::Dead => self.dead += 1,
            Verdict::Stale | Verdict::Unreachable => self.stale += 1,
            _ => self.active += 1,
        }
    }
}

/// One pane's verdict this cycle.
struct PaneMark {
    /// The `%<n>` pane id.
    pane: String,
    /// What the accounting made of it, with the seat tool's job beside it.
    verdict: SeatVerdict,
    /// The watchdog-owned public observation, including restart carry.
    observed: String,
}

/// One cycle verdict in the window layout the ticker animates between cycles.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MotionVerdict {
    pane: String,
    window: String,
    verdict: SeatVerdict,
}

/// The orchestrator target publication state, including not-yet-published.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum PublishedOrchestrator {
    /// No fleet observation has been published since this daemon attached.
    #[default]
    Unknown,
    /// The observed fleet has no orchestrator target.
    Unset,
    /// The observed fleet's orchestrator session id.
    Target(String),
}

/// The orchestrator strip publication state, including not-yet-published.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum PublishedOrchestratorStrip {
    /// No fleet observation has been published since this daemon attached.
    #[default]
    Unknown,
    /// The observed fleet has no orchestrator segment.
    Unset,
    /// The rendered orchestrator segment.
    Value(String),
}

/// The ticker's carry: the most recent cycle verdicts and working frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MotionState {
    verdicts: Vec<MotionVerdict>,
    panes: Vec<tmux::MotionPane>,
    fleet: Vec<theme::FleetRow>,
    fleet_target: Option<String>,
    /// The human's `[workspace] fleet_order`, STORED rather than read here. The
    /// ticker redraws at motion cadence and must neither open the config nor draw
    /// a different order from the one the cycle just published — that would flap.
    /// The cycle refreshes this once per cycle; BOTH writers read it from here.
    fleet_order: theme::FleetOrder,
    published_fleet: Option<String>,
    /// The last orchestrator target publication, including a successful unset.
    published_orchestrator_id: PublishedOrchestrator,
    /// The last orchestrator segment publication, including a successful unset.
    published_orchestrator_strip: PublishedOrchestratorStrip,
    spin: u64,
}

impl MotionState {
    /// Replace only the cycle-owned verdicts. The cycle already published the
    /// corresponding static window labels.
    fn replace_verdicts(&mut self, verdicts: Vec<MotionVerdict>) {
        self.verdicts = verdicts;
    }

    /// Replace the ticker's two observations together: the panes decide
    /// visibility and local animation, while the fleet drives line two.
    fn replace_observation(
        &mut self,
        panes: Vec<tmux::MotionPane>,
        sessions: &[tmux::FleetListingRow],
        known: &[String],
        session: &str,
    ) {
        self.panes = panes;
        self.replace_fleet(sessions, known, session);
    }

    /// Take this cycle's fleet order. Called from the VERDICT path only: the
    /// ticker preserves whatever the last cycle left here, which is what keeps
    /// the motion frames and the verdict write drawing one order.
    fn set_fleet_order(&mut self, order: &theme::FleetOrder) {
        self.fleet_order.clone_from(order);
    }

    /// Replace the fleet half of an observation and remember which exact
    /// session table owns the strip. The stored order is deliberately untouched
    /// — an observation says who is on the server, never how to arrange them.
    ///
    /// `known` is the adoption scan's answer to "which of these does ae's own
    /// record vouch for", which is what lets a RANKLESS running session be
    /// drawn Stale here instead of vanishing from this session's own strip.
    fn replace_fleet(
        &mut self,
        sessions: &[tmux::FleetListingRow],
        known: &[String],
        session: &str,
    ) {
        self.fleet = fleet_rows(sessions, known, session);
        // The target comes off the DRAWN rows, never the raw listing: a session
        // that draws no row of its own — rankless and not vouched for — has no
        // strip to own, exactly as when the reader dropped rankless rows for it.
        self.fleet_target = self
            .fleet
            .iter()
            .find(|row| row.current)
            .map(|row| row.id.clone());
    }

    /// Add the fleet strip when its text has changed. A working frame makes
    /// that true on every animation step; a static fleet writes only after a
    /// rank, name or order change.
    fn push_fleet_write(
        &mut self,
        writes: &mut Vec<tmux::OptionWrite>,
        look: &Look,
        working_frame: Option<&theme::WorkingFrame>,
    ) {
        let Some(target) = self.fleet_target.as_deref() else {
            return;
        };
        let strip = theme::fleet_strip(look, &self.fleet, working_frame, &self.fleet_order);
        if self.published_fleet.as_deref() == Some(&strip) {
            return;
        }
        writes.push(tmux::OptionWrite::new(
            OptionScope::Session,
            target,
            theme::FLEET_STRIP_OPTION,
            &strip,
        ));
        self.published_fleet = Some(strip);
    }

    /// Add the orchestrator target only when it changed. A missing target is
    /// still recorded so the caller can unset it once, rather than spawning a
    /// tmux process on every motion tick.
    fn push_orchestrator_id_write(
        &mut self,
        writes: &mut Vec<tmux::OptionWrite>,
        target: &str,
        desired: Option<&str>,
    ) -> bool {
        let desired = desired.map_or(PublishedOrchestrator::Unset, |id| {
            PublishedOrchestrator::Target(id.to_owned())
        });
        if self.published_orchestrator_id == desired {
            return false;
        }
        if let PublishedOrchestrator::Target(id) = &desired {
            writes.push(tmux::OptionWrite::new(
                OptionScope::Session,
                target,
                theme::ORCHESTRATOR_ID_OPTION,
                id,
            ));
        }
        self.published_orchestrator_id = desired;
        true
    }

    /// Add the orchestrator segment only when its rendered text changed. A
    /// missing orchestrator is recorded so the caller can unset stale output
    /// once rather than spawning a tmux process on every motion tick.
    fn push_orchestrator_strip_write(
        &mut self,
        writes: &mut Vec<tmux::OptionWrite>,
        look: &Look,
        target: &str,
        row: Option<&theme::FleetRow>,
        working_frame: Option<&theme::WorkingFrame>,
    ) -> bool {
        let desired = row.map_or(PublishedOrchestratorStrip::Unset, |row| {
            PublishedOrchestratorStrip::Value(theme::orchestrator_strip(look, row, working_frame))
        });
        if self.published_orchestrator_strip == desired {
            return false;
        }
        if let PublishedOrchestratorStrip::Value(value) = &desired {
            writes.push(tmux::OptionWrite::new(
                OptionScope::Session,
                target,
                theme::ORCHESTRATOR_STRIP_OPTION,
                value,
            ));
        }
        self.published_orchestrator_strip = desired;
        true
    }

    /// The cached verdicts whose pane is a live agent drawing the working mark.
    /// Field by field, so a caller may advance the spin while it holds them.
    fn working<'a>(
        verdicts: &'a [MotionVerdict],
        panes: &[tmux::MotionPane],
    ) -> Vec<&'a MotionVerdict> {
        verdicts
            .iter()
            .filter(|entry| entry.verdict.mark() == Mark::Working)
            .filter(|entry| {
                panes.iter().any(|pane| {
                    pane.pane_id == entry.pane
                        && pane.agent.as_deref().is_some_and(|agent| {
                            !agent.is_empty() && !NON_AGENT_PANES.contains(&agent)
                        })
                })
            })
            .collect()
    }

    /// Whether the fleet strip carries a working row, which pulses too.
    fn fleet_working(&self) -> bool {
        self.fleet.iter().any(|row| row.mark == Mark::Working)
    }

    /// Whether [`MotionState::step`] draws a frame from this observation: a
    /// client can see the session, and something on it is working.
    fn animates(&self) -> bool {
        session_attached(&self.panes)
            && (!Self::working(&self.verdicts, &self.panes).is_empty() || self.fleet_working())
    }

    /// Advance every attached working surface from the cached observation.
    fn step(&mut self, look: &Look) -> Vec<tmux::OptionWrite> {
        if !self.panes.iter().any(|pane| pane.session_attached > 0) {
            return Vec::new();
        }
        let working = Self::working(&self.verdicts, &self.panes);
        let fleet_working = self.fleet_working();
        if !working.is_empty() || fleet_working {
            self.spin = self.spin.wrapping_add(FRAME_PHASES);
        }
        let frame = theme::working_frame(self.spin, &look.palette, look.icons);
        let mut writes = Vec::new();
        let mut windows = Vec::new();
        for entry in &working {
            writes.push(tmux::OptionWrite::new(
                OptionScope::Pane,
                &entry.pane,
                theme::PANE_STATE_OPTION,
                // The VERDICT's word, never a literal: `Active` and
                // `Meta(MetaSweeping)` share the Working mark, and a hardcoded
                // "working" here repainted a just-published `● sweeping` as
                // `● working` within one tick. A fresh `waiting-agent` is not
                // repainted at all — its mark is static, so the published
                // `◔ waiting-agent` stands until the next verdict cycle.
                &theme::pane_state_frame(&frame, entry.verdict.reason()),
            ));
            if !windows.contains(&entry.window) {
                windows.push(entry.window.clone());
            }
        }
        for window in &windows {
            let agents: Vec<(String, Mark)> = self
                .verdicts
                .iter()
                .filter(|entry| entry.window == *window)
                .filter_map(|entry| {
                    let agent = self
                        .panes
                        .iter()
                        .find(|pane| pane.pane_id == entry.pane)?
                        .agent
                        .as_deref()
                        .filter(|agent| !agent.is_empty() && !NON_AGENT_PANES.contains(agent))?;
                    Some((theme::agent_label(agent), entry.verdict.mark()))
                })
                .collect();
            writes.push(tmux::OptionWrite::new(
                OptionScope::Window,
                window,
                theme::WINDOW_AGENTS_OPTION,
                &window_agents_line(&agents, look, Some(&frame)),
            ));
        }
        self.push_fleet_write(&mut writes, look, fleet_working.then_some(&frame));
        if let Some(target) = self.fleet_target.clone() {
            let orchestrator = self
                .fleet
                .iter()
                .find(|row| row.name == crate::orchestrator::ORCHESTRATOR_SESSION)
                .cloned();
            if let Some(row) = orchestrator.as_ref() {
                let _ = self.push_orchestrator_strip_write(
                    &mut writes,
                    look,
                    &target,
                    Some(row),
                    fleet_working.then_some(&frame),
                );
            }
        }
        writes
    }

    /// The static half of `step` for `motion = off`: fleet and orchestrator
    /// publications without animation frames. Same attached gate and
    /// write-on-change caching as `step`; F6's unset lands here too.
    fn step_static(&mut self, look: &Look) -> Vec<tmux::OptionWrite> {
        if !self.panes.iter().any(|pane| pane.session_attached > 0) {
            return Vec::new();
        }
        let mut writes = Vec::new();
        self.push_fleet_write(&mut writes, look, None);
        if let Some(target) = self.fleet_target.clone() {
            let orchestrator = self
                .fleet
                .iter()
                .find(|row| row.name == crate::orchestrator::ORCHESTRATOR_SESSION)
                .cloned();
            if let Some(row) = orchestrator.as_ref() {
                let _ =
                    self.push_orchestrator_strip_write(&mut writes, look, &target, Some(row), None);
            }
        }
        writes
    }
}

// ---------------------------------------------------------------------------
// ADOPTION: the fleet strip of a session whose own watchdog is not running.
// ---------------------------------------------------------------------------

/// How often a daemon redraws the strips it has adopted.
///
/// Adoption's OWN cadence, deliberately not the ticker's: [`MotionState::step`]
/// draws nothing while no client is attached, and the peer being filled is
/// exactly the session somebody IS looking at. A detached adopter must keep
/// drawing, so this runs attached or not, in every [`TickerMode`].
const ADOPTION_TICK: Duration = DETACHED_MOTION_TICK;

/// One session this daemon fills the fleet strip for, because nothing else is.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Adopted {
    /// The session name, as ae's own records and the server agreed on it.
    name: String,
    /// The `$<n>` the enumeration proved, and the ONLY target ever written to.
    /// An id, never a name: tmux never reuses one while the server runs, so a
    /// write cannot land on a session that took the name over since.
    id: String,
    /// Its state directory — read for the pidfile, never written.
    meta_dir: PathBuf,
    /// The watchdog pid this session's pidfile named at enumeration, `None`
    /// when it had none. The pause test compares against THIS rather than
    /// against presence: a dead daemon leaves its pidfile behind, and a target
    /// that paused on a stale file would never be filled at all.
    enum_pid: Option<u32>,
    /// The strip text last published here — the write-on-change memory, one
    /// per target.
    published: Option<String>,
}

/// Who this daemon is drawing the fleet strip for besides itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Adoption {
    /// The adopted sessions, refreshed once per verdict cycle.
    targets: Vec<Adopted>,
    /// Every session ae's OWN records prove is a running ae session of this
    /// state root — adopted or not.
    ///
    /// The strip's rank rule cannot answer this: a session nobody measures
    /// publishes no rank, and a rank is a tmux option a stranger could set too.
    /// So the rows a rankless session is drawn in are admitted on ae's records
    /// and the ownership proof, never on the option.
    known: Vec<String>,
}

/// Whether a session's own watchdog is running, from its pidfile and ONE
/// process table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatchdogPresence {
    /// A pidfile naming a process the table lists.
    Live,
    /// No pidfile, or one naming a process the table does not list — with the
    /// pid it named, so a later tick can tell a lingering file from a new one.
    Absent(Option<u32>),
    /// There was no table to ask. Never adopted: the peer's own daemon may be
    /// right there.
    Unknown,
}

/// Read `meta_dir`'s pidfile and judge it against `table`.
///
/// READ-ONLY, deliberately not [`crate::watchdog_lifecycle::presence`]: that one
/// deletes a stale pidfile, and this is another session's state directory. The
/// pane proof that `presence` also makes is a tmux process per peer, which is
/// what this whole path exists to avoid; a pidfile plus the cycle's own table
/// is the same two facts minus the third.
fn watchdog_presence(meta_dir: &Path, table: Option<&[procs::Proc]>) -> WatchdogPresence {
    let Some(pid) = crate::watchdog_glue::read_pid(meta_dir) else {
        return WatchdogPresence::Absent(None);
    };
    match table {
        None => WatchdogPresence::Unknown,
        // ACCEPTED: a recycled pid makes a dead watchdog read as live, which
        // costs the peer its adopted strip until something reuses the pid no
        // longer. `watchdog stop`'s own stale-pidfile cleanup narrows it.
        Some(table) if table.iter().any(|proc| proc.pid == pid) => WatchdogPresence::Live,
        Some(_) => WatchdogPresence::Absent(Some(pid)),
    }
}

/// The liveness backend for the adoption scan: it answers only THIS server, and
/// only from the ownership proof already read for each name.
///
/// The picker's stopped rows are enumerated the same way and admit a live name
/// on ae's rank row; this one cannot, because the sessions it is looking for are
/// exactly the ones that publish no rank. So the marker IS
/// `seed_unwatched`'s proof — an `AE_SESSION` marker plus an `AE_HOME` naming
/// this state root — which is also what decides whether a peer may be written
/// to at all. One read, one rule, both questions.
struct AdoptionBackend<'a> {
    server: &'a crate::inventory::ServerId,
    sockets: &'a crate::fleet_read::SocketPaths,
    proven: &'a [crate::inventory::DiscoveredSession],
}

impl crate::inventory::Discovery for AdoptionBackend<'_> {
    fn enumerate(
        &self,
        server: &crate::inventory::ServerId,
    ) -> std::result::Result<Vec<crate::inventory::DiscoveredSession>, crate::inventory::QueryFailed>
    {
        if !self.sockets.equivalent(server, self.server) {
            return Err(crate::inventory::QueryFailed);
        }
        Ok(self.proven.to_vec())
    }
}

/// The ownership a peer must prove before this daemon writes a display fact
/// into it — `seed_unwatched`'s proof, and for the same reason.
///
/// A NAME is not an identity. The marker says a session is ae's; the home says
/// WHICH ae state root launched it. Without both, a stranger who took the name
/// over on a shared server would be handed this fleet's strip, and a session
/// belonging to another `AE_HOME` would be drawn into a fleet it is not part of.
fn proven_ownership(owned: Option<&transport::SessionOwnership>, root: &Path) -> Option<String> {
    owned
        .filter(|owned| !owned.marker.is_empty() && Path::new(&owned.home) == root)
        .map(|owned| owned.marker.clone())
}

/// Who this daemon owes a fleet strip, once per verdict cycle.
///
/// `None` when there is no state root to scan, which is not evidence that
/// anything changed: the caller keeps the adoption it already had.
///
/// `table` is the cycle's OWN process snapshot, passed in rather than taken:
/// one `ps` per cycle regardless of how many peers there are, and none at all
/// on the 2 s tick.
fn enumerate_adoption(
    server: &crate::inventory::ServerId,
    session: &str,
    listing: &[tmux::FleetListingRow],
    table: Option<&[procs::Proc]>,
    prior: &Adoption,
) -> Option<Adoption> {
    let root = crate::doors::state_root(crate::shape::current())?;
    // META ONLY: the picker's read. Identities and the `placed` rule, never a
    // session's journal — nothing here is derived from events.
    let scan = crate::inventory::durable_meta_records(&crate::inventory::Roots::under(&root));
    // The OWNERSHIP proof, made once per name this server is actually showing.
    // A record whose session is not on this server costs nothing, and neither
    // does this daemon's own: it is never a target and never vouched for, so
    // probing it would buy two tmux spawns a cycle and no answer anyone reads.
    let names: Vec<String> = scan
        .records
        .iter()
        .map(|record| record.name.clone())
        .filter(|name| name != session)
        .filter(|name| listing.iter().any(|row| &row.name == name))
        .collect();
    let read = transport::observe_peer_ownership(server, &names);
    let mut sockets = match read.socket {
        Some(path) => crate::fleet_read::SocketPaths::for_caller(server, Some(path)),
        None => crate::fleet_read::SocketPaths::live(),
    };
    // Warm the cache ONLY for a spelling that is not already ours: two records
    // naming the same server the same way need no tmux probe to be equivalent.
    for candidate in &scan.records {
        if let Some(selector) = candidate.server.entitles() {
            let recorded = crate::inventory::ServerId::Selected(selector.clone());
            if &recorded != server {
                let _ = sockets.proven_same(server, &recorded);
            }
        }
    }
    let proven: Vec<crate::inventory::DiscoveredSession> = read
        .peers
        .into_iter()
        .map(|(name, owned)| crate::inventory::DiscoveredSession {
            marker: proven_ownership(owned.as_ref(), &root),
            name,
        })
        .collect();
    let backend = AdoptionBackend {
        server,
        sockets: &sockets,
        proven: &proven,
    };
    let inventory = crate::inventory::Inventory {
        candidates: scan
            .records
            .into_iter()
            .map(crate::inventory::Candidate::durable)
            .collect(),
        // Carried, never recomputed: an incomplete scan is a REPORT, not a
        // refusal. The sessions ae could enumerate are adopted; the ones it
        // could not are simply not targets this cycle.
        incomplete: scan.incomplete,
    };
    Some(adoption_from(
        crate::liveness::classify(inventory, &backend).sessions,
        session,
        listing,
        prior,
        |meta_dir| watchdog_presence(meta_dir, table),
    ))
}

/// The adoption a classified scan decides — the whole rule, with no world in it.
///
/// RUNNING only, never this session itself, never a candidate the server is not
/// currently showing, and never one whose own watchdog is live or unproven. An
/// `unknown` candidate is simply not a target: the classifier says `unknown`
/// exactly when it could not prove the session, and an incomplete scan is
/// therefore adopt-the-known, skip-the-unknown rather than a refusal.
fn adoption_from(
    classified: Vec<crate::liveness::Classified>,
    session: &str,
    listing: &[tmux::FleetListingRow],
    prior: &Adoption,
    presence: impl Fn(&Path) -> WatchdogPresence,
) -> Adoption {
    let mut next = Adoption::default();
    for classified in classified {
        if classified.status != Status::Running {
            continue;
        }
        let Some(record) = classified.candidate.durable else {
            continue;
        };
        // This daemon's own session is never a target: it publishes its own
        // strip, and a second writer would fight it every tick.
        if record.name == session {
            continue;
        }
        next.known.push(record.name.clone());
        let Some(row) = listing.iter().find(|row| row.name == record.name) else {
            continue;
        };
        let WatchdogPresence::Absent(enum_pid) = presence(&record.path) else {
            continue;
        };
        next.targets.push(Adopted {
            name: record.name.clone(),
            id: row.id.clone(),
            meta_dir: record.path,
            enum_pid,
            // Carried across the re-enumeration so a target that has not
            // changed is not rewritten once a minute for nothing.
            published: prior
                .targets
                .iter()
                .find(|held| held.id == row.id && held.name == record.name)
                .and_then(|held| held.published.clone()),
        });
    }
    next
}

/// The rows one strip draws, from ONE listing.
///
/// A ranked row draws its rank. A rankless row draws [`Mark::Stale`] when ae's
/// own records vouch for it — that is the rule "a session that RUNS is always a
/// row, and a row nobody is measuring says so", and it holds on every strip this
/// daemon writes, its own included. A rankless row nothing vouches for is
/// DROPPED: it is a session ae did not create, and the strip draws no strangers.
fn fleet_rows(
    listing: &[tmux::FleetListingRow],
    known: &[String],
    current: &str,
) -> Vec<theme::FleetRow> {
    listing
        .iter()
        .filter_map(|row| {
            let mark = match row.rank.as_deref() {
                Some(rank) => Mark::from_rank(rank),
                None if known.iter().any(|name| name == &row.name) => Mark::Stale,
                None => return None,
            };
            Some(theme::FleetRow {
                name: row.name.clone(),
                id: row.id.clone(),
                mark,
                current: row.name == current,
            })
        })
        .collect()
}

/// The strips this daemon owes its adopted peers — and NOTHING else.
///
/// Every write this returns sets [`theme::FLEET_STRIP_OPTION`] on a peer, and
/// the pin beside it turns red if a second option ever joins them. That is the
/// whole of the fourth writer's licence: a rank, a glyph, a health segment or a
/// roster written here would be this daemon vouching for a session it is not
/// measuring.
///
/// Each strip is drawn STATIC, in the TARGET's look, with the TARGET as the
/// current row — a strip that cannot show you where you are is not a map — and
/// in the fleet order every writer shares.
fn adoption_writes(
    adoption: &mut Adoption,
    listing: &[tmux::FleetListingRow],
    order: &theme::FleetOrder,
) -> Vec<tmux::OptionWrite> {
    let Adoption { targets, known } = adoption;
    let mut writes = Vec::new();
    for target in targets.iter_mut() {
        // The OWNER is back. Stop writing and let it publish over this; a
        // pidfile naming the SAME pid as at enumeration is the dead daemon's
        // leftover, not a live one, and must not pause the adoption.
        let pid = crate::watchdog_glue::read_pid(&target.meta_dir);
        if pid.is_some() && pid != target.enum_pid {
            continue;
        }
        // Proven again from THIS listing, by id and name together: a target
        // that has gone is skipped rather than written to, and a look ae could
        // not read is a target ae leaves alone this tick.
        let Some(row) = listing
            .iter()
            .find(|row| row.id == target.id && row.name == target.name)
        else {
            continue;
        };
        let look = Look::read(
            &row.look.icons,
            &row.look.palette,
            &row.look.drawn,
            &row.look.motion,
        );
        // `theme = off` on the TARGET does not stop this: ae fills `@ae_*` on
        // an undrawn session too, so a hand-written `status-right` still has
        // the strip to put in it.
        let strip = theme::fleet_strip(
            &look,
            &fleet_rows(listing, known, &target.name),
            None,
            order,
        );
        if target.published.as_deref() == Some(&strip) {
            continue;
        }
        writes.push(tmux::OptionWrite::new(
            OptionScope::Session,
            &target.id,
            theme::FLEET_STRIP_OPTION,
            &strip,
        ));
        target.published = Some(strip);
    }
    writes
}

/// Whether the adoption cadence is due now, stamping it when it is.
fn adoption_due(last: &mut Option<Instant>, now: Instant) -> bool {
    if last.is_some_and(|stamp| now.duration_since(stamp) < ADOPTION_TICK) {
        return false;
    }
    *last = Some(now);
    true
}

/// Draw THIS session's row of a fleet listing with the rank this cycle
/// decided, whatever the listing caught of the published option.
///
/// The cycle sends its writes as one batch at its end, so the listing it reads
/// still carries LAST cycle's rank. The rank is the only cycle-written fact the
/// listing feeds back into the strip and the adoption scan. A peer's row is never
/// touched: this daemon does not vouch for another session's verdict, and a
/// listing without this session's id gains no row.
fn overlay_current_rank(rows: &mut [tmux::FleetListingRow], session_id: &str, rank: Mark) {
    for row in rows.iter_mut().filter(|row| row.id == session_id) {
        row.rank = Some(rank.rank().to_string());
    }
}

/// The fleet target the orchestrator segment may jump to: absent for this session
/// when it is itself the orchestrator, or when the fleet has none. The exact
/// canonical seat name is intentional: a renamed seat loses the click target.
fn orchestrator_id_for<'a>(
    sessions: &'a [tmux::FleetListingRow],
    session: &str,
) -> Option<&'a str> {
    if session == crate::orchestrator::ORCHESTRATOR_SESSION {
        return None;
    }
    sessions
        .iter()
        .find(|entry| entry.name == crate::orchestrator::ORCHESTRATOR_SESSION)
        .map(|entry| entry.id.as_str())
}

/// Whether a tick that carries no write must still observe: none has been
/// committed yet, or the last is [`ATTACHED_OBSERVATION`] old. A tick that
/// writes always observes, because its read rides the same invocation.
fn motion_observation_due(observed_at: Option<Instant>, now: Instant) -> bool {
    observed_at.is_none_or(|at| now.saturating_duration_since(at) >= ATTACHED_OBSERVATION)
}

/// Whether this look permits periodic redraws at all.
const fn motion_ticker_enabled(look: &Look) -> bool {
    look.drawn && look.motion
}

/// Which wait a verdict interval gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TickerMode {
    /// `theme = off`, or no look ever read: the cycle owns `@ae_*`.
    Idle,
    /// `motion = off`: re-observation without animation frames.
    Static,
    /// Drawn and animated: the full motion ticker.
    Animated,
}

/// Motion gates ANIMATION FRAMES only: drawn-but-still keeps its fleet
/// strip fresh, and only `theme = off` sleeps the whole interval.
const fn ticker_mode(look: Option<&Look>) -> TickerMode {
    match look {
        Some(look) if motion_ticker_enabled(look) => TickerMode::Animated,
        Some(look) if look.drawn => TickerMode::Static,
        _ => TickerMode::Idle,
    }
}

/// The observation cadence when motion is off: the SAME cadence the
/// animated ticker observes at when nothing animates.
fn static_observe_cadence(panes: &[tmux::MotionPane]) -> Duration {
    if session_attached(panes) {
        ATTACHED_OBSERVATION
    } else {
        DETACHED_MOTION_TICK
    }
}

/// Whether an observation saw a client on this session.
fn session_attached(panes: &[tmux::MotionPane]) -> bool {
    panes.iter().any(|pane| pane.session_attached > 0)
}

/// The animated ticker's next rest from its committed observation: a frame
/// every [`ATTACHED_MOTION_TICK`] while something visible is working, else
/// the static observer's cadence.
fn motion_cadence(state: &MotionState) -> Duration {
    if state.animates() {
        ATTACHED_MOTION_TICK
    } else {
        static_observe_cadence(&state.panes)
    }
}

/// Advance the bounded failure streak and say whether ticking must stop for
/// this verdict interval.
const fn motion_failure(prior: u8) -> (u8, bool) {
    let next = prior.saturating_add(1);
    (next, next >= MOTION_FAILURE_LIMIT)
}

/// A write against cached pane identities may fail because the fleet changed
/// since observation. Refresh it without spending the transport failure
/// budget; only a write against fresh identities proves a ticker failure.
///
/// Every write rides a read and is drawn from the observation before it, so
/// the identities are fresh once the observation that came with a refused
/// write has been committed: `observed` is whether the previous publication
/// was refused too.
const fn motion_publish_failure(prior: u8, observed: bool) -> (u8, bool) {
    if observed {
        motion_failure(prior)
    } else {
        (prior, false)
    }
}

/// Whether a fleet-picker marker has reached half a watchdog interval.
/// Malformed transient state expires too, so it cannot pin the highlight.
fn menu_open_expired(opened: &str, now_epoch: i64, interval_secs: u64) -> bool {
    let Ok(opened_epoch) = opened.parse::<i64>() else {
        return true;
    };
    let Ok(interval) = i64::try_from(interval_secs / 2) else {
        return false;
    };
    now_epoch.saturating_sub(opened_epoch) >= interval
}

/// The session options the cycle reads beside its look, in ONE queued read
/// ([`Cycle::look`]): the stamp the layout was written for, then the two menu
/// markers [`Cycle::publish`] expires.
const CYCLE_LOOK_OPTIONS: [&str; 3] = [
    theme::LOOK_STAMP_OPTION,
    theme::MENU_OPEN_OPTION,
    theme::SETTINGS_OPEN_OPTION,
];

/// What the cycle's one look read found.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CycleLookRead {
    /// The look, or `None` when its read did not answer.
    look: Option<Look>,
    /// [`theme::LOOK_STAMP_OPTION`], `None` when unset or not answered.
    stamp: Option<String>,
    /// [`theme::MENU_OPEN_OPTION`], then [`theme::SETTINGS_OPEN_OPTION`], each
    /// `None` when unset or not answered.
    opened: [Option<String>; 2],
}

/// Everything one cycle publishes, gathered so the call reads as one statement.
struct Published<'a> {
    /// The watch bar's own glyph.
    bar: &'a str,
    /// How many panes were neither dead nor stale.
    active: usize,
    /// How many panes were judged at all.
    total: usize,
    /// Per-pane verdicts, in pane order.
    by_pane: &'a [PaneMark],
    /// The whole versioned roster snapshot, absent when the recorded roster
    /// cannot be represented by the strict picker grammar.
    agents: Option<&'a str>,
    /// The session's rolled-up mark.
    attention: Mark,
    /// The look to draw all of it in.
    look: &'a Look,
}

/// The session's own mark: the most actionable thing any of its surfaces is
/// saying, and [`Mark::Idle`] when none of them says anything.
///
/// BOTH inputs, because they do not cover the same ground: `by_pane` is what
/// the panes that are there are doing, and `roster` carries the slots whose
/// pane is NOT there. A rollup that read only the first would leave the fleet
/// strip calling a session idle while its roster says an agent is missing.
fn session_mark(by_pane: &[PaneMark], roster: &[Mark]) -> Mark {
    Mark::most_actionable(
        by_pane
            .iter()
            .map(|entry| entry.verdict.mark())
            .chain(roster.iter().copied()),
    )
}

fn launch_id_for<'a>(ids: &'a [(String, String)], slot: &str) -> Option<&'a str> {
    ids.iter()
        .find(|(seat, _)| seat.as_str() == slot)
        .map(|(_, id)| id.as_str())
}

/// Run the watchdog for one session until its session or its state goes away.
///
/// # Errors
///
/// Only writing the status stream; every observation failure degrades within
/// the cycle instead.
pub fn run(
    meta_dir: &Path,
    knobs: Knobs,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    run_paced(meta_dir, knobs, None, out, err)
}

/// [`run`] driven by a test, one cycle per signal on `channel`
/// (`_watchdog-run --step-channel <channel>`).
///
/// The startup cycle runs at once. After every completed iteration the daemon
/// signals `<channel>-done` and waits for the next signal on `channel`; real
/// time, an attach or another channel never starts a cycle. No return path
/// signals done. A rebind of the recorded server stops it with 1.
///
/// # Errors
///
/// As [`run`].
pub fn run_stepped(
    meta_dir: &Path,
    knobs: Knobs,
    channel: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    run_paced(meta_dir, knobs, Some(channel), out, err)
}

/// The body of [`run`] and [`run_stepped`].
fn run_paced(
    meta_dir: &Path,
    mut knobs: Knobs,
    step: Option<&str>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    let Ok(bytes) = crate::meta::read_bytes(meta_dir) else {
        writeln!(
            err,
            "ae: watchdog: no session state at {}",
            meta_dir.display()
        )?;
        return Ok(1);
    };
    knobs.sweep.sweep_secs = sweep_seconds(&bytes, sweep_env().as_deref(), knobs.sweep.sweep_secs);
    knobs.quota_every_secs = match quota_seconds(&bytes, knobs.quota_every_secs) {
        Ok(seconds) => seconds,
        Err(value) => {
            writeln!(
                err,
                "ae: watchdog: quota_every_secs must be an unsigned integer in seconds; got '{value}'."
            )?;
            return Ok(crate::state::EXIT_USAGE);
        }
    };
    // Awareness is re-resolved EVERY cycle in `watch` (a pin wins and holds
    // the session stable; with no pin the live config is honoured at use),
    // so nothing is pinned here.
    knobs.idle_nudge_secs = match idle_nudge_seconds(&bytes, knobs.idle_nudge_secs) {
        Ok(seconds) => seconds,
        Err(value) => {
            writeln!(
                err,
                "ae: watchdog: idle_nudge_secs must be an unsigned integer in seconds; got '{value}'."
            )?;
            return Ok(crate::state::EXIT_USAGE);
        }
    };
    let raw = crate::meta::sole_value(&bytes, "done_confirmations")
        .and_then(|value| std::str::from_utf8(value).ok());
    let doubled = raw.is_none() && crate::meta::first_value(&bytes, "done_confirmations").is_some();
    let (count, note) =
        crate::meta::resolve_done_confirmations(raw, doubled, knobs.done_confirmations);
    knobs.done_confirmations = count;
    if let Some(note) = note {
        writeln!(err, "ae: watchdog: {note}")?;
    }
    let meta = Meta::parse(&String::from_utf8_lossy(&bytes));
    // The INITIAL resolution, kept as the fast refuse.
    let server = match meta.server_selector() {
        ServerSelector::Positive(selector) => crate::inventory::ServerId::Selected(selector),
        ServerSelector::Missing | ServerSelector::Ambiguous => {
            writeln!(
                err,
                "ae: watchdog: no positive tmux server recorded — refusing to watch an \
                 ambient server"
            )?;
            return Ok(1);
        }
    };
    let session = session_name(&bytes, meta_dir);
    let helper = SendHelper::for_session(meta_dir);
    let journal = Journal {
        meta_dir,
        session: &session,
    };

    // ── The pane's own duties, in the order that matters: the pidfile FIRST,
    // because the start path's registration wait is what releases the start
    // lock; then the bars, so a pane that is up says so before its first
    // cycle; then the banner.
    let pidfile = match crate::watchdog_glue::PidFile::publish(meta_dir) {
        Ok(published) => Some(published),
        Err(why) => {
            // Reported, not fatal.
            writeln!(err, "ae: watchdog: pidfile not published: {why}")?;
            None
        }
    };
    // The pre-rename reap, which was `_watchdog_start`'s first act. A launch
    // or a resume has no caller to refuse to, so the journal names what it
    // found or could not list.
    let scan = crate::watchdog_glue::reap_legacy(&server, &session, meta_dir, err)?;
    if let Some(note) = crate::watchdog_glue::legacy_reap_note(&session, &scan) {
        journal.record("legacy-reap", "", &note, err)?;
    }
    announce_start(&server, &session, meta.work_dir());
    write!(
        out,
        "{}",
        crate::watchdog_glue::banner(
            &session,
            knobs.interval_secs,
            knobs.stale_secs,
            knobs.max_nudges
        )
    )?;
    out.flush()?;
    let mut deferred = crate::watchdog_glue::Deferred::new(
        meta_dir,
        Some(crate::lifecycle::meta_value(&bytes, "config").as_str()),
        knobs.tg_supervise_secs,
    );

    let mut stepped = step.map(|channel| StepPace::new(server.clone(), channel));
    let code = watch(
        meta_dir,
        knobs,
        server,
        &session,
        &helper,
        &journal,
        &mut deferred,
        &mut stepped,
        err,
    );
    drop(stepped);
    // The pidfile is released by `PidFile`'s Drop — ownership-checked, so a
    // stop/start in quick succession never lets the dying process vandalise its
    // successor's registration — and Drop, not an explicit call, so EVERY
    // return after publish releases it, the `?` exits above included.
    drop(pidfile);
    code
}

/// One verdict cycle, then the pane's own duties, with everything both decide
/// to publish sent as ONE batch at the end.
fn run_cycle(
    cycle: &Cycle<'_>,
    carry: &mut Carry,
    meta: &Meta,
    deferred: &mut crate::watchdog_glue::Deferred,
    journal: &Journal<'_>,
    err: &mut impl Write,
) -> crate::error::Result<()> {
    // ONE identity read per cycle, handed to every writer.
    carry.pending = PendingCycle::new(transport::observe_session_identity(
        cycle.server,
        cycle.session,
    ));
    let ran = cycle.run(carry, err);
    // The pane's own per-cycle duties, in order: the branch pair, which is a
    // git read no cycle owns, then the recovery and the revive.
    let duties = if ran.is_ok() {
        tick_pane_duties(
            cycle.server,
            cycle.meta_dir,
            cycle.session,
            meta,
            deferred,
            journal,
            &mut carry.pending,
            err,
        )
    } else {
        Ok(())
    };
    // What the cycle decided goes out even when a later step failed: each
    // write used to land the moment it was made.
    flush_cycle(cycle.server, carry);
    ran?;
    duties
}

/// Send the cycle's ONE publication batch, then commit what it proved: the
/// fleet's write-on-change memories once every fleet operation landed, and the
/// wake channel once its hook is proven installed.
fn flush_cycle(server: &crate::inventory::ServerId, carry: &mut Carry) {
    let pending = std::mem::take(&mut carry.pending);
    let Some(identity) = pending.identity else {
        // Nothing was decided without an identity, and no hook was installed.
        carry.wake = None;
        return;
    };
    // The hook rides first in the batch on this incarnation's own channel.
    let channel = tmux::wake_channel(&identity);
    let batch = tmux::CycleBatch {
        session: identity.id,
        channel,
        ops: pending.ops,
    };
    let outcome = transport::publish_cycle(server, &batch);
    if let Some(staged) = pending.fleet
        && outcome
            .landed
            .get(staged.ops.clone())
            .is_some_and(|landed| landed.iter().all(|ok| *ok))
    {
        carry.motion = staged.motion;
        carry.adoption = staged.adoption;
    }
    carry.wake = batch.channel.filter(|_| outcome.hook_proven);
}

/// Publish the two things a pane that is UP says before its first cycle: the
/// starting health segment and the branch pair.
fn announce_start(server: &crate::inventory::ServerId, session: &str, work_dir: Option<&str>) {
    let Some(session_id) = transport::observe_session_id(server, session) else {
        return;
    };
    // The session's OWN look, not a frozen glyph: a session running the ASCII
    // fallback would otherwise show one braille character until the first
    // cycle replaced it. A look that did NOT answer publishes nothing here: the
    // first cycle says it instead, in a look it actually read.
    if let Some(read) = transport::observe_look(server, session) {
        let look = Look::read(&read.icons, &read.palette, &read.drawn, &read.motion);
        let _ = transport::publish_option(
            server,
            OptionScope::Session,
            &session_id,
            tmux::WATCHDOG_STATUS_OPTION,
            &format!(
                "#[fg={}]{} starting",
                look.palette.dim,
                Mark::Stale.glyph(look.icons)
            ),
        );
    }
    crate::watchdog_glue::publish_branch(
        server,
        &session_id,
        crate::watchdog_glue::branch_reading(work_dir).as_ref(),
    );
}

/// The loop itself, split from [`run`] so the pidfile it publishes is released
/// on EVERY return rather than on the ones someone remembered.
#[allow(
    clippy::too_many_arguments,
    reason = "the loop's context, kept as parameters so `run` owns the pidfile's lifetime; \
              gathering them into a struct would move that ownership back inside the loop"
)]
fn watch(
    meta_dir: &Path,
    knobs: Knobs,
    mut server: crate::inventory::ServerId,
    session: &str,
    helper: &SendHelper,
    journal: &Journal<'_>,
    deferred: &mut crate::watchdog_glue::Deferred,
    step: &mut Option<StepPace>,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    let mut carry = Carry::new();
    // The global config PATH is stable for this daemon's life; its CONTENT is
    // re-read every cycle below, so a config flip — quota awareness, or the
    // human's fleet order — reaches an unpinned session within one cycle.
    let global_config = crate::global_config_of(meta_dir);
    let mut beat_failed = false;
    loop {
        let read = crate::meta::read_bytes(meta_dir);
        // ONE parse per cycle, and it happens BEFORE the probe because the
        // probe has to be aimed at the server this cycle's record names.
        let parsed = read
            .as_ref()
            .ok()
            .map(|bytes| Meta::parse(&String::from_utf8_lossy(bytes)));
        match rebind(&server, parsed.as_ref()) {
            Rebind::Keep => {}
            Rebind::Use(_) if step.is_some() => return stop_stepped_rebind(&server, session, err),
            Rebind::Use(named) => {
                server = adopt_server(
                    server,
                    named,
                    &mut carry,
                    |leaving| clear_published(leaving, session),
                    journal,
                    err,
                )?;
            }
            Rebind::Refuse => {
                // Retract what we published, on the server we published it to,
                // then stop exactly as startup would have.
                let _ = clear_published(&server, session);
                // The RECORD stopped naming one server; the session did not
                // stop. It is about to have no watchdog at all, so it says so
                // and keeps the rank that makes it a row — under the ownership
                // proof, because the name alone cannot tell this session from a
                // stranger that took it over.
                if let Some(root) = meta_dir.parent().and_then(Path::parent) {
                    let _ = seed_unwatched(&server, session, root);
                }
                writeln!(
                    err,
                    "ae: watchdog: the recorded tmux server stopped naming exactly one \
                     server — stopping rather than watching an ambient one"
                )?;
                return Ok(1);
            }
        }
        let probe = transport::verify_session_absent(&server, session);
        let ran_cycle = match continuation(read.as_ref().err().map(io::Error::kind), &probe) {
            Continuation::Stop => {
                // PROVEN gone.
                let _ = clear_published(&server, session);
                return Ok(0);
            }
            Continuation::Retry => {
                writeln!(
                    err,
                    "ae: watchdog: liveness unproven this cycle — retrying, bar left as published"
                )?;
                false
            }
            // `Run` is only returned when the read succeeded, so this `if let`
            // is how that is spelled without an unwrap rather than a branch
            // anyone expects to take.
            Continuation::Run => {
                // The restore beat: the ONE writer, once presence is proven.
                refresh_beat(meta_dir, &mut beat_failed, err)?;
                if let (Ok(bytes), Some(meta)) = (&read, &parsed) {
                    let local_config = meta
                        .origin()
                        .and_then(|origin| crate::config::local_overlay(meta_dir, origin));
                    // Awareness AT USE, every cycle: a pinned row wins and
                    // holds the session stable across config flips; with no
                    // pin the live config is honoured now, so all consumers
                    // flip together within one cycle. Never a startup value.
                    let mut cycle_knobs = knobs;
                    cycle_knobs.quota_aware =
                        quota_awareness(bytes, global_config.as_deref(), local_config.as_deref());
                    let cycle = Cycle {
                        knobs: cycle_knobs,
                        meta_dir,
                        helper,
                        server: &server,
                        session,
                        goal: meta.goal().map(ToOwned::to_owned),
                        local_config,
                        lead_pair: crate::lifecycle::meta_value(bytes, "layout") == "lead-pair",
                        fleet_order: crate::fleet_order_at(global_config.as_deref()),
                        auto: crate::autoreseat::settings(global_config.as_deref()),
                        auto_trust: crate::config::global_auto_trust(global_config.as_deref()),
                        // Re-read EVERY cycle, like the goal and the roster: a
                        // session can be promoted to orchestrator, or its main
                        // replaced, while this daemon runs.
                        meta_agent: is_meta_agent(bytes),
                        roster: meta.roster().to_vec(),
                        started: meta.started_epoch().or_else(|| meta.latest_launch_epoch()),
                        settled: crate::autoreseat::start_fact(
                            meta.started_epoch(),
                            meta.launch_settled_epoch(),
                            Timestamp::now().epoch(),
                        ),
                        launch_ids: meta
                            .roster()
                            .iter()
                            .filter_map(|entry| {
                                let key = format!("launch_id.{}", entry.slot);
                                let value = crate::meta::sole_value(bytes, &key)
                                    .map(String::from_utf8_lossy)?;
                                (!value.is_empty())
                                    .then(|| (entry.slot.clone(), value.into_owned()))
                            })
                            .collect(),
                    };
                    run_cycle(&cycle, &mut carry, meta, deferred, journal, err)?;
                }
                true
            }
        };
        rest(&server, session, &mut carry, knobs, ran_cycle, step);
    }
}

/// Rest between two iterations of [`watch`]: the interval loop after a verdict
/// cycle, one plain sleep after a retry. A stepped daemon first says the
/// iteration completed, then rests until its next step.
fn rest(
    server: &crate::inventory::ServerId,
    session: &str,
    carry: &mut Carry,
    knobs: Knobs,
    ran_cycle: bool,
    step: &mut Option<StepPace>,
) {
    let interval_secs = knobs.interval_secs;
    match step {
        Some(pace) => {
            pace.acknowledge();
            if ran_cycle {
                wait_between_cycles_with(server, session, carry, STEP_BACKSTOP, pace);
            } else {
                pace.sleep(STEP_BACKSTOP);
            }
        }
        None if ran_cycle => wait_between_cycles(server, session, carry, interval_secs),
        None => std::thread::sleep(Duration::from_secs(interval_secs)),
    }
}

/// A stepped daemon whose record names another server stops instead of moving:
/// its step channel lives on the server it started on, and the move would make
/// that server unaddressable.
fn stop_stepped_rebind(
    server: &crate::inventory::ServerId,
    session: &str,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    let _ = clear_published(server, session);
    writeln!(
        err,
        "ae: watchdog: step mode does not follow a server rebind — stopping"
    )?;
    Ok(1)
}

/// Refresh the restore beat; a failure never stops the watcher and is said once.
fn refresh_beat(
    meta_dir: &Path,
    failing: &mut bool,
    err: &mut impl Write,
) -> crate::error::Result<()> {
    match crate::watchdog_glue::touch_beat(meta_dir) {
        Ok(()) => *failing = false,
        Err(why) if !*failing => {
            *failing = true;
            writeln!(err, "ae: watchdog: beat not refreshed: {why}")?;
        }
        Err(_) => {}
    }
    Ok(())
}

/// Wait for the next verdict cycle while publishing motion at its own cadence.
/// A failed tick still sleeps its cadence, so transient tmux failures cannot
/// shorten the verdict interval. Three consecutive failures stop the ticker
/// and sleep the remainder before the next liveness proof.
fn wait_between_cycles(
    server: &crate::inventory::ServerId,
    session: &str,
    carry: &mut Carry,
    interval_secs: u64,
) {
    wait_between_cycles_with(
        server,
        session,
        carry,
        Duration::from_secs(interval_secs),
        &mut RealPace,
    );
}

/// The clock and the external wait the interval loop runs on.
///
/// Production is [`RealPace`]; a test replaces time and the wait's answer
/// only, never a ticker decision.
trait Pace {
    /// The current instant.
    fn now(&self) -> Instant;
    /// Rest for `duration`.
    fn sleep(&mut self, duration: Duration);
    /// Wait until `session` has a client, `channel` is signalled, or
    /// `deadline` passes.
    fn wait(
        &mut self,
        server: &crate::inventory::ServerId,
        session: &str,
        channel: &str,
        deadline: Instant,
    ) -> transport::Waited;
}

/// The wall clock and the real tmux wait.
struct RealPace;

impl Pace for RealPace {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn wait(
        &mut self,
        server: &crate::inventory::ServerId,
        session: &str,
        channel: &str,
        deadline: Instant,
    ) -> transport::Waited {
        transport::run_tmux_wait(server, session, channel, deadline)
    }
}

/// The longest a stepped daemon waits for its next step before it runs a
/// cycle anyway: a safety bound far outside any test's wait, never a clock.
const STEP_BACKSTOP: Duration = Duration::from_mins(10);

/// The test-only pace of `_watchdog-run --step-channel <channel>`: real time,
/// except that a signal on `channel` ends the current interval.
///
/// The interval loop is given [`STEP_BACKSTOP`] as its interval and runs
/// unchanged, motion ticker included. Every rest it takes waits on ONE plain
/// `wait-for <channel>` client, kept across rests until it exits, so a release
/// can never race a step already on its way. A step moves this clock past the
/// interval's deadline, and the loop leaves through its own deadline check.
/// `interval_secs` keeps every arithmetic role it has.
struct StepPace {
    server: crate::inventory::ServerId,
    channel: String,
    /// `<channel>-done`, signalled after every completed iteration.
    done: String,
    /// How far this clock runs ahead of the wall clock: one backstop per step.
    skew: Duration,
    waiter: Option<transport::ChannelWaiter>,
}

impl StepPace {
    fn new(server: crate::inventory::ServerId, channel: &str) -> Self {
        Self {
            server,
            channel: channel.to_owned(),
            done: format!("{channel}-done"),
            skew: Duration::ZERO,
            waiter: None,
        }
    }

    /// Say that one loop iteration completed.
    fn acknowledge(&self) {
        let _ = transport::signal_channel(&self.server, &self.done);
    }

    /// Wait for a step until the real instant `until`. A step, or a waiter
    /// that cannot wait (its server is gone), ends the interval: the next
    /// liveness probe decides from there.
    fn await_step(&mut self, until: Instant) -> transport::Waited {
        if self.waiter.is_none() {
            self.waiter = transport::ChannelWaiter::start(&self.server, &self.channel);
        }
        let waited = self
            .waiter
            .as_mut()
            .map_or(transport::Waited::Failed, |waiter| waiter.wait_until(until));
        if waited != transport::Waited::TimedOut {
            self.waiter = None;
            self.skew = self.skew.saturating_add(STEP_BACKSTOP);
        }
        waited
    }
}

impl Pace for StepPace {
    fn now(&self) -> Instant {
        let now = Instant::now();
        now.checked_add(self.skew).unwrap_or(now)
    }

    fn sleep(&mut self, duration: Duration) {
        let until = Instant::now()
            .checked_add(duration)
            .unwrap_or_else(Instant::now);
        let _ = self.await_step(until);
    }

    fn wait(
        &mut self,
        _server: &crate::inventory::ServerId,
        _session: &str,
        _channel: &str,
        deadline: Instant,
    ) -> transport::Waited {
        let until = deadline.checked_sub(self.skew).unwrap_or_else(Instant::now);
        self.await_step(until)
    }
}

impl Drop for StepPace {
    /// A waiter still waiting when the daemon stops is released, never killed.
    fn drop(&mut self) {
        if let Some(waiter) = self.waiter.take() {
            waiter.release(&self.server);
        }
    }
}

/// [`wait_between_cycles`] over an explicit [`Pace`]: the ONE interval loop
/// production runs. The deadline is the interval's end, whatever happens in
/// between.
fn wait_between_cycles_with(
    server: &crate::inventory::ServerId,
    session: &str,
    carry: &mut Carry,
    interval: Duration,
    pace: &mut impl Pace,
) {
    let Some(look) = carry.look else {
        // No look has EVER answered, so this session draws nothing of its own —
        // but a peer it is filling still has a line to keep.
        wait_idle_between_cycles(server, carry, interval, pace);
        return;
    };
    match ticker_mode(carry.look.as_ref()) {
        TickerMode::Idle => wait_idle_between_cycles(server, carry, interval, pace),
        TickerMode::Static => {
            wait_static_between_cycles(server, session, carry, &look, interval, pace);
        }
        TickerMode::Animated => run_ticker(server, session, carry, &look, interval, pace),
    }
}

/// Drive one [`MotionTicker`] to the end of the interval, then hand its
/// state back to the carry.
///
/// While the session is detached and this cycle proved its wake hook, the
/// loop holds ONE waiter instead of observing every two seconds. A wake, or a
/// client the waiter's own read counted, proves nothing by itself: the next
/// tick observes fresh, and that observation decides between motion and the
/// next wait, which starts no sooner than [`WAKE_REARM`] after the last. The
/// interval ends at its deadline whatever happens in between, so a wake never
/// starts a verdict cycle early. A failed wait polls for the rest of the
/// interval; an adopter always polls, because its peer's strip is a duty no
/// hook of this session announces.
fn run_ticker(
    server: &crate::inventory::ServerId,
    session: &str,
    carry: &mut Carry,
    look: &Look,
    interval: Duration,
    pace: &mut impl Pace,
) {
    let started = pace.now();
    let deadline = started + interval;
    let mut ticker = MotionTicker::new(carry.motion.clone(), carry.adoption.clone());
    let mut wake = carry
        .wake
        .clone()
        .filter(|_| carry.adoption.targets.is_empty());
    let mut waited_at: Option<Instant> = None;
    loop {
        let now = pace.now();
        if now >= deadline {
            break;
        }
        if let Some(channel) = wake.as_deref()
            && !session_attached(&ticker.motion.panes)
        {
            if let Some(earliest) = waited_at.map(|at| at + WAKE_REARM)
                && now < earliest
            {
                pace.sleep(earliest.min(deadline).saturating_duration_since(now));
                continue;
            }
            waited_at = Some(now);
            match pace.wait(server, session, channel, deadline) {
                transport::Waited::TimedOut => continue,
                transport::Waited::Failed => {
                    wake = None;
                    continue;
                }
                transport::Waited::Woke | transport::Waited::Attached => {}
            }
        }
        let remaining = deadline.saturating_duration_since(pace.now());
        if remaining.is_zero() {
            break;
        }
        let next = ticker.tick(server, session, look, pace.now());
        let remaining = interval.saturating_sub(pace.now().saturating_duration_since(started));
        if next.stopped {
            pace.sleep(remaining);
            break;
        }
        pace.sleep(next.after.min(remaining));
    }
    carry.motion = ticker.motion;
    carry.adoption = ticker.adoption;
}

/// Wait for the next verdict cycle with nothing of this session's OWN to draw.
///
/// `theme = off`, or no look has ever answered: the verdict cycle owns every
/// `@ae_*` this session publishes, so the whole interval is one sleep — which is
/// what it costs when this daemon has adopted nobody, the common case.
///
/// A daemon that IS filling a watchdog-less peer's fleet strip owes that duty to
/// the TARGET's look, not to its own, so it cannot sleep through the interval:
/// it wakes at the adoption cadence for one listing and at most one write per
/// tick, and goes back to the whole-interval sleep as soon as the peer's own
/// watchdog comes back.
fn wait_idle_between_cycles(
    server: &crate::inventory::ServerId,
    carry: &mut Carry,
    interval: Duration,
    pace: &mut impl Pace,
) {
    // Decided ONCE: the target set is the verdict cycle's to change, so a
    // daemon with none sleeps exactly as it did before adoption existed.
    if carry.adoption.targets.is_empty() {
        pace.sleep(interval);
        return;
    }
    let started = pace.now();
    let mut failures = 0_u8;
    loop {
        let remaining = interval.saturating_sub(pace.now().saturating_duration_since(started));
        if remaining.is_zero() {
            break;
        }
        let mut adoption = carry.adoption.clone();
        let landed = match transport::observe_fleet_listing(server) {
            Some(listing) => {
                let writes = adoption_writes(&mut adoption, &listing, &carry.motion.fleet_order);
                writes.is_empty() || transport::publish_options(server, &writes)
            }
            None => false,
        };
        if !landed {
            let failed = motion_failure(failures);
            failures = failed.0;
            let remaining = interval.saturating_sub(pace.now().saturating_duration_since(started));
            if failed.1 {
                pace.sleep(remaining);
                break;
            }
            pace.sleep(ADOPTION_TICK.min(remaining));
            continue;
        }
        failures = 0;
        carry.adoption = adoption;
        let remaining = interval.saturating_sub(pace.now().saturating_duration_since(started));
        pace.sleep(ADOPTION_TICK.min(remaining));
    }
}

/// Wait for the next verdict cycle while keeping the fleet strip fresh at
/// the observation cadence, without animation frames. Same three-failure
/// rule as the animated ticker: the same [`MotionTicker`], which takes its
/// static arm from the look.
fn wait_static_between_cycles(
    server: &crate::inventory::ServerId,
    session: &str,
    carry: &mut Carry,
    look: &Look,
    interval: Duration,
    pace: &mut impl Pace,
) {
    run_ticker(server, session, carry, look, interval, pace);
}

/// What one ticker iteration asks of the interval loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TickNext {
    /// Rest this long before the next tick; the loop caps it at the interval's
    /// remainder.
    after: Duration,
    /// The failure limit was reached: rest the remainder of the interval
    /// without further motion work.
    stopped: bool,
}

/// The motion ticker between two verdict cycles: the state it commits and
/// the memory it schedules by. One is made per interval from the carry.
#[derive(Debug, Clone)]
struct MotionTicker {
    /// The committed motion observation and write-on-change memory.
    motion: MotionState,
    /// The committed adoption targets and their write-on-change memory.
    adoption: Adoption,
    /// Consecutive failed ticks.
    failures: u8,
    /// The cadence the last observation chose; `None` before the first tick.
    cadence: Option<Duration>,
    /// When the last observation committed; `None` before the first.
    observed_at: Option<Instant>,
    /// The last publication was refused, so the observation committed with
    /// it is the fresh one the next write is drawn from.
    refused: bool,
    /// When the adopted strips were last drawn.
    adopted_at: Option<Instant>,
}

impl MotionTicker {
    /// A ticker starting from the carry's committed state.
    const fn new(motion: MotionState, adoption: Adoption) -> Self {
        Self {
            motion,
            adoption,
            failures: 0,
            cadence: None,
            observed_at: None,
            refused: false,
            adopted_at: None,
        }
    }

    /// One tick: publish what the committed observation draws, observe in
    /// the same invocation, commit what landed. `look` picks the arm —
    /// animated frames, or the static observer for `motion = off`.
    fn tick(
        &mut self,
        server: &crate::inventory::ServerId,
        session: &str,
        look: &Look,
        now: Instant,
    ) -> TickNext {
        if ticker_mode(Some(look)) == TickerMode::Static {
            self.tick_static(server, session, look, now)
        } else {
            self.tick_animated(server, session, look, now)
        }
    }

    fn tick_animated(
        &mut self,
        server: &crate::inventory::ServerId,
        session: &str,
        look: &Look,
        now: Instant,
    ) -> TickNext {
        let cadence = *self.cadence.get_or_insert(ATTACHED_MOTION_TICK);
        // STAGED: the frame is drawn from the committed observation, and its
        // write-on-change memory commits only once tmux took the writes.
        let mut drawn = self.motion.clone();
        let writes = drawn.step(look);
        if writes.is_empty() && !motion_observation_due(self.observed_at, now) {
            return TickNext {
                after: cadence,
                stopped: false,
            };
        }
        let exchange = transport::exchange_motion(server, session, true, &writes);
        self.commit(server, session, exchange, drawn, now, motion_cadence)
    }

    fn tick_static(
        &mut self,
        server: &crate::inventory::ServerId,
        session: &str,
        look: &Look,
        now: Instant,
    ) -> TickNext {
        self.cadence
            .get_or_insert_with(|| static_observe_cadence(&self.motion.panes));
        let mut drawn = self.motion.clone();
        let writes = drawn.step_static(look);
        let exchange = transport::exchange_motion(server, session, true, &writes);
        self.commit(server, session, exchange, drawn, now, |state| {
            static_observe_cadence(&state.panes)
        })
    }

    /// Commit what one exchange proved, and nothing it did not.
    ///
    /// - The reading is unusable: a read did not complete (tmux then ran none
    ///   of the writes), or today's interpreter refused a section (the writes
    ///   may have landed). No state commits, and the failure counts; a write
    ///   that landed anyway is asserted again by the next tick.
    /// - It is usable: the observation commits, whatever became of the writes.
    /// - The writes landed (or there were none): `drawn`'s spin and
    ///   write-on-change memory commit too, and the adopted strips are drawn
    ///   from this fresh listing when their cadence is due.
    /// - The writes were refused: the spin and the memory stay, so the next
    ///   tick draws the same phase again from the fresh observation.
    fn commit(
        &mut self,
        server: &crate::inventory::ServerId,
        session: &str,
        exchange: transport::MotionExchange,
        drawn: MotionState,
        now: Instant,
        cadence_of: impl Fn(&MotionState) -> Duration,
    ) -> TickNext {
        let cadence = self.cadence.unwrap_or(ATTACHED_MOTION_TICK);
        let Some((panes, fleet)) = exchange.reading else {
            let failed = motion_failure(self.failures);
            self.failures = failed.0;
            return TickNext {
                after: cadence,
                stopped: failed.1,
            };
        };
        let landed = exchange.published != Some(false);
        if landed {
            self.motion = drawn;
        }
        self.motion
            .replace_observation(panes, &fleet, &self.adoption.known, session);
        self.observed_at = Some(now);
        let cadence = cadence_of(&self.motion);
        self.cadence = Some(cadence);
        if !landed {
            let failed = motion_publish_failure(self.failures, self.refused);
            self.failures = failed.0;
            self.refused = true;
            return TickNext {
                after: cadence,
                stopped: failed.1,
            };
        }
        self.refused = false;
        // Adoption's own cadence, off the listing this tick just read: never
        // a frame this daemon's own attachment gates. Its writes change only
        // when a strip does, so they cost an invocation of their own only then.
        if adoption_due(&mut self.adopted_at, now) {
            let mut adoption = self.adoption.clone();
            let writes = adoption_writes(&mut adoption, &fleet, &self.motion.fleet_order);
            if !writes.is_empty() && !transport::publish_options(server, &writes) {
                let failed = motion_failure(self.failures);
                self.failures = failed.0;
                return TickNext {
                    after: cadence,
                    stopped: failed.1,
                };
            }
            self.adoption = adoption;
        }
        self.failures = 0;
        TickNext {
            after: cadence,
            stopped: false,
        }
    }
}

/// The branch publication, the pending-id recovery and the Telegram revive,
/// once per cycle.
#[allow(
    clippy::too_many_arguments,
    reason = "the cycle's context, passed rather than gathered: each is a fact the loop \
              already owns, and a struct would only rename them"
)]
fn tick_pane_duties(
    server: &crate::inventory::ServerId,
    meta_dir: &Path,
    session: &str,
    meta: &Meta,
    deferred: &mut crate::watchdog_glue::Deferred,
    journal: &Journal<'_>,
    pending: &mut PendingCycle,
    err: &mut impl Write,
) -> crate::error::Result<()> {
    if let Some(session_id) = pending.session_id() {
        let ops = crate::watchdog_glue::branch_ops(
            session_id,
            crate::watchdog_glue::branch_reading(meta.work_dir()).as_ref(),
        );
        pending.ops.extend(ops);
    }
    for row in crate::watchdog_glue::recover(meta_dir, meta.roster()) {
        // The DURABLE record of a post-launch capture.
        journal.record_referring(
            "recover",
            &row.agent,
            &row.captured,
            &crate::watchdog_glue::recovered_summary(&row),
            err,
        )?;
    }
    deferred.supervise(server, session, SystemTime::now());
    Ok(())
}

/// Which tmux server this cycle must OBSERVE and PUBLISH on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rebind {
    /// The record MOVED: adopt this server, which is never the one already in
    /// force.
    Use(crate::inventory::ServerId),
    /// The record does not name exactly one server.
    Refuse,
    /// Nothing to do: the record still names the server already in force, or it
    /// could not be read at all.
    Keep,
}

/// Which server this cycle addresses, from the meta this cycle read.
#[must_use]
pub fn rebind(current: &crate::inventory::ServerId, meta: Option<&Meta>) -> Rebind {
    let Some(meta) = meta else {
        return Rebind::Keep;
    };
    match meta.server_selector() {
        ServerSelector::Positive(selector) => {
            let named = crate::inventory::ServerId::Selected(selector);
            if named == *current {
                Rebind::Keep
            } else {
                Rebind::Use(named)
            }
        }
        ServerSelector::Missing | ServerSelector::Ambiguous => Rebind::Refuse,
    }
}

/// Where this daemon records what it did — the session's own event log.
struct Journal<'a> {
    meta_dir: &'a Path,
    session: &'a str,
}

impl Journal<'_> {
    /// Append one watchdog event.
    fn record(
        &self,
        action: &str,
        target: &str,
        summary: &str,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        self.record_referring(action, target, "", summary, err)
    }

    /// Append one watchdog event that names a REFERENCE — the id, request or
    /// artifact the record is about.
    fn record_referring(
        &self,
        action: &str,
        target: &str,
        reference: &str,
        summary: &str,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        if let Err(why) = self.append_referring(action, target, reference, summary) {
            writeln!(
                err,
                "ae: watchdog: {action} for {target} not recorded: {why}"
            )?;
        }
        Ok(())
    }

    /// [`Self::record_referring`], but the append outcome is RETURNED: the
    /// give-up path must not rename before its event is proven durable, so it
    /// is the one caller that sees a failed append. Every other caller keeps
    /// the swallow above. The ONE owner of the watchdog's event-line shape.
    fn append_referring(
        &self,
        action: &str,
        target: &str,
        reference: &str,
        summary: &str,
    ) -> Result<(), store::Error> {
        let line = tracked::event_line(&EventFields {
            ts: Timestamp::now(),
            actor: ACTOR,
            action,
            target,
            reference,
            actor_slot: "",
            actor_session: self.session,
            target_slot: "",
            target_session: "",
            actor_session_id: "",
            target_session_id: "",
            target_server: "",
            target_pane: "",
            target_session_uuid: "",
            caller_server: "",
            caller_pane: "",
            caller_session_uuid: "",
            identity_gap: "",
            summary,
            body_file: "",
        });
        store::open(self.meta_dir).append_event(&line)
    }
}

/// Everything this daemon carries between cycles that is scoped to ONE SERVER.
struct Carry {
    /// Per-pane history, keyed by `pane_id`.
    panes: Vec<(String, PaneState)>,
    /// The missing-pane debounce, keyed by roster slot.
    missing: Vec<(String, MissingState)>,
    /// The faster publisher that runs between verdict cycles.
    motion: MotionState,
    /// Session-local quota transitions, pending per-recipient deliveries, and
    /// the last bounded observation available to the throttle branch.
    quota: QuotaCarry,
    /// The peers whose fleet strip this daemon fills, and the names ae's own
    /// records vouch for. Server-scoped like everything else here, so
    /// [`Carry::reset`] drops it with the rest when the daemon moves servers —
    /// an id proved on one server means nothing on another.
    adoption: Adoption,
    /// The slot whose brief retry was tried LAST, and the whole of the
    /// rotation that keeps one stuck record from starving the others.
    ///
    /// In memory on purpose: this is scheduling, not a correctness latch, so a
    /// restart simply restarts the rotation.
    brief_cursor: Option<String>,
    /// Each auto reseat hold already named, per seat, episode and reason. In
    /// memory on purpose: a restart names each once more, as `limit` re-books.
    /// A headroom hold is also not named again while the journal's newest
    /// record of its episode is that same hold.
    auto_named: Vec<(String, Timestamp, crate::autoreseat::HoldReason)>,
    /// Last cycle's capture of each seat a resume reads as exhausted, by pane
    /// and launch, for the quiet pair. Taken whole at each cycle's start, so a
    /// failed read, a seat no longer exhausted, a new pane or a new incarnation
    /// leaves nothing to compare.
    start_frames: Vec<((String, Option<String>), String)>,
    /// The `auto_reseat_at` and `auto_reseat_lead_at` notes this daemon journaled:
    /// each once per start.
    auto_noted: Vec<String>,
    /// The unusable `auto_trust` values this daemon named: each once per start.
    trust_noted: Vec<String>,
    /// The effort step's note and recent legs.
    effort: crate::effort::leg::Carry,
    /// The last look this daemon actually READ, and `None` until one answers.
    ///
    /// Carried so that a cycle whose read failed draws in the look it saw last
    /// rather than in a guess: the alternative is a session with the theme off,
    /// or in another palette, being repainted in ae's default because one tmux
    /// call did not answer. Before the FIRST successful read there is no last
    /// look either, and a default would be that same guess — so nothing which
    /// depends on the look is published at all until one arrives.
    look: Option<Look>,
    /// The wake channel this cycle's publication proved installed on this
    /// session, and `None` when it did not: no waiter is held without it.
    wake: Option<String>,
    /// The publication the running cycle has decided and not yet sent.
    pending: PendingCycle,
    /// The previous verdict cycle's two clock readings, for the suspension
    /// detector ([`crate::watchdog::host_suspended`]). In memory on purpose:
    /// the RECORD it writes is the durable fact; a restart only misses a
    /// suspension it did not watch.
    clock: Option<crate::watchdog::CycleClock>,
}

/// One verdict cycle's publication while the cycle is still deciding it: every
/// option write and removal the cycle and its pane duties make, sent as ONE
/// tmux invocation once both are done ([`flush_cycle`]).
#[derive(Default)]
struct PendingCycle {
    /// This session's immutable identity, read once at the cycle's start and
    /// handed to every writer; `None` when it did not answer, which publishes
    /// nothing this cycle.
    identity: Option<tmux::SessionIdentity>,
    /// The operations, in the order the cycle decided them.
    ops: Vec<tmux::CycleOp>,
    /// The fleet publication's write-on-change memories, waiting for the batch.
    fleet: Option<StagedFleet>,
}

impl PendingCycle {
    /// A publication for the session `identity` names.
    fn new(identity: Option<tmux::SessionIdentity>) -> Self {
        Self {
            identity,
            ops: Vec::new(),
            fleet: None,
        }
    }

    /// This session's `$<n>` id, when the cycle's identity read answered.
    fn session_id(&self) -> Option<&str> {
        self.identity.as_ref().map(|identity| identity.id.as_str())
    }
}

/// The motion and adoption state the fleet publication computed, committed
/// only once every operation it rides on is proven landed: a write-on-change
/// memory advanced past a write tmux refused would never retry it.
struct StagedFleet {
    motion: MotionState,
    adoption: Adoption,
    /// The fleet's operations inside [`PendingCycle::ops`].
    ops: std::ops::Range<usize>,
}

impl Carry {
    fn new() -> Self {
        Self {
            panes: Vec::new(),
            missing: Vec::new(),
            motion: MotionState::default(),
            quota: QuotaCarry::default(),
            adoption: Adoption::default(),
            brief_cursor: None,
            auto_named: Vec::new(),
            start_frames: Vec::new(),
            auto_noted: Vec::new(),
            trust_noted: Vec::new(),
            effort: crate::effort::leg::Carry::default(),
            look: None,
            wake: None,
            pending: PendingCycle::default(),
            clock: None,
        }
    }

    /// Drop every carry, because the server they are scoped to is being left.
    fn reset(&mut self) {
        *self = Self::new();
    }
}

/// Move this daemon from one server to another, in the ONE order that is safe.
fn adopt_server(
    leaving: crate::inventory::ServerId,
    joining: crate::inventory::ServerId,
    carry: &mut Carry,
    retract: impl FnOnce(&crate::inventory::ServerId) -> bool,
    journal: &Journal<'_>,
    err: &mut impl Write,
) -> crate::error::Result<crate::inventory::ServerId> {
    // 1. BEST-EFFORT: retract our bars while the old server is still
    //    addressable. Nothing targets it after this function returns.
    if !retract(&leaving) {
        writeln!(
            err,
            "ae: watchdog: could not retract this daemon's status options from the server \
             it is leaving — they may persist there; proceeding with the move"
        )?;
        // Durable too: a stderr line in a detached daemon is a line nobody
        // reads.
        journal.record(
            "alert",
            ACTOR,
            "watchdog moved servers but could not clear its options on the old one",
            err,
        )?;
    }
    drop(leaving); // the old server is unaddressable from here on, by construction
    // 2.
    carry.reset();
    Ok(joining)
}

/// What this cycle's own liveness readings mean for the DAEMON'S life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Continuation {
    /// Both readings are good: run the cycle.
    Run,
    /// A reading FAILED.
    Retry,
    /// Proven gone.
    Stop,
}

/// Whether the daemon keeps running, retries, or exits — the tri-state the rest
/// of this port already uses, applied to the daemon's OWN liveness.
#[must_use]
pub fn continuation(meta_error: Option<io::ErrorKind>, session: &StopProbe) -> Continuation {
    match (meta_error, session) {
        // Proof, from either side: the session is gone, or its state is.
        (_, StopProbe::Absent) | (Some(io::ErrorKind::NotFound), _) => Continuation::Stop,
        (Some(_), _) | (None, StopProbe::Unknown) => Continuation::Retry,
        (None, StopProbe::Present) => Continuation::Run,
    }
}

/// Remove everything this daemon published, and report whether it all came off.
pub(crate) fn clear_published(server: &crate::inventory::ServerId, session: &str) -> bool {
    let Some(session_id) = transport::observe_session_id(server, session) else {
        return false;
    };
    // `&=`, NEVER `&&` and never an early return: every option must still be
    // attempted after one of them fails.
    let mut ok = true;
    // EVERY session-scoped value this daemon publishes, the three attention
    // options included: only a live daemon can vouch for any of them, and every
    // OTHER session on the server reads the rank and the strip. A caller whose
    // session is still RUNNING puts the launch seed back over the three,
    // immediately after the retraction — see `seed_unwatched` below for why an
    // unwatched session says Stale rather than nothing.
    for name in [
        tmux::WATCHDOG_STATUS_OPTION,
        theme::ATTENTION_GLYPH_OPTION,
        theme::ATTENTION_RANK_OPTION,
        theme::ATTENTION_STYLE_OPTION,
        theme::AGENTS_OPTION,
        theme::FLEET_STRIP_OPTION,
        theme::ORCHESTRATOR_STRIP_OPTION,
        theme::ORCHESTRATOR_ID_OPTION,
        theme::GOAL_OPTION,
        theme::VERSION_OPTION,
    ] {
        ok &= transport::clear_option(server, OptionScope::Session, &session_id, name);
    }
    // The branch pair is published by THIS daemon too, so it is retracted with
    // everything else: a stopped watchdog that left
    // `@ae_branch_*` behind would keep asserting a branch nobody is watching.
    ok &= crate::watchdog_glue::clear_branch(server, &session_id);
    let Some(panes) = transport::observe_window_panes(server, session) else {
        return false;
    };
    let mut cleared: Vec<String> = Vec::new();
    for pane in &panes {
        if cleared.contains(&pane.window_id) {
            continue;
        }
        cleared.push(pane.window_id.clone());
        ok &= transport::clear_option(
            server,
            OptionScope::Window,
            &pane.window_id,
            theme::WINDOW_AGENTS_OPTION,
        );
    }
    // The per-pane half: a border title that outlived its watchdog would keep
    // naming a state nothing is judging any more.
    for pane in &panes {
        for name in [
            theme::PANE_STATE_OPTION,
            theme::PANE_ACCENT_OPTION,
            theme::OBSERVED_OPTION,
        ] {
            ok &= transport::clear_option(server, OptionScope::Pane, &pane.pane_id, name);
        }
    }
    ok
}

/// Hand a session that is still RUNNING back to its launch SEED.
///
/// The counterpart of the retraction above, for the paths where the watchdog
/// goes away and the session does not: the three attention options go back to
/// exactly what a launch writes, and the health segment says nothing is
/// watching. Unset is never the answer — every fleet reader drops a session
/// that publishes no rank, so an unwatched one would vanish from every other
/// session's strip and from the picker while `ae list` still called it running.
/// A frozen last verdict is not the answer either: Stale is already what
/// "nobody is measuring this" means everywhere else in the bar.
///
/// `root` is the state root whose launch created the session, because the seed
/// is an ae fact: a name is not an identity, and a same-name session belonging
/// to somebody else must never be handed an ae rank that puts it on every strip.
///
/// Reports whether every write landed, `false` included when there was nothing
/// addressable to write to.
pub(crate) fn seed_unwatched(
    server: &crate::inventory::ServerId,
    session: &str,
    root: &Path,
) -> bool {
    let Some(session_id) = transport::observe_session_id(server, session) else {
        return false;
    };
    // OWNERSHIP, read the way the UUID backfill reads it: the marker proves an
    // ae session, and the home names the state root whose launch created it.
    let owned = transport::observe_session_ownership(server, session).is_some_and(|ownership| {
        !ownership.marker.is_empty() && Path::new(&ownership.home) == root
    });
    if !owned {
        return false;
    }
    // The session's OWN look, never a frozen one: a session running the ASCII
    // fallback would otherwise be handed a braille glyph. A look that did not
    // answer writes nothing, exactly as the start announcement decides — a
    // server that cannot be read cannot be written either.
    let Some(read) = transport::observe_look(server, session) else {
        return false;
    };
    let look = Look::read(&read.icons, &read.palette, &read.drawn, &read.motion);
    // `&=`, NEVER `&&`: one refused option must not skip the rest of the seed,
    // and a rank without its glyph draws a row nobody can read.
    let mut ok = true;
    for (name, value) in theme::seed_options(&look) {
        ok &= transport::publish_option(server, OptionScope::Session, &session_id, &name, &value);
    }
    ok &= transport::publish_option(
        server,
        OptionScope::Session,
        &session_id,
        tmux::WATCHDOG_STATUS_OPTION,
        &theme::watchdog_off_segment(&look),
    );
    ok
}

/// The debounce for a roster agent whose pane is not in the session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MissingState {
    streak: u32,
    alerted: bool,
}

/// A wait the cycle's quiet resolution ENDED on the human's input in the pane:
/// the state that ended and the age of that input. [`Cycle::resolve_quiet`] is
/// the ONE reader of client activity that produces it, and the nudge that
/// follows names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitEnded {
    /// The wait state the declaration named.
    pub kind: QuietKind,
    /// The age, in seconds, of the input that ended it.
    pub input_age_secs: u64,
}

impl WaitEnded {
    /// The state word the note and the summary both name — the SAME word the
    /// pane border and the `state` helper use.
    #[must_use]
    fn state_word(self) -> &'static str {
        Verdict::Quiet(self.kind).reason()
    }
}

/// What one pane's quiet resolution answered for a cycle: the suppression that
/// HOLDS, and the wait THIS cycle ended on input in the pane. Both come from
/// ONE reading of client activity, so no consumer derives either itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuietResolution {
    /// The declaration that holds, if any.
    pub quiet: Option<QuietKind>,
    /// The wait this cycle ended on input, if any.
    pub ended: Option<WaitEnded>,
}

impl QuietResolution {
    /// Nothing resolved: no declaration, or one that is not current.
    const NONE: Self = Self {
        quiet: None,
        ended: None,
    };
}

/// What one pane's quiet resolution needs, gathered so the call reads as one
/// question.
struct QuietQuery<'a> {
    /// The session's events, oldest first.
    events: &'a [Event],
    /// The pane's `@ae_agent` display ref.
    agent: &'a str,
    /// The pane's roster slot — half of the routing key the ONE relevance
    /// owner judges actor and target by.
    slot: &'a str,
    /// The newest input a client viewing the pane gave, epoch seconds.
    activity: Option<u64>,
    /// This cycle's wall clock, epoch seconds: what an input's age is
    /// measured from.
    now: i64,
}

/// The newest input any client viewing `pane` gave, or `None` when none did
/// or the client read failed.
pub(crate) fn viewing_input(clients: Option<&[tmux::ObservedClient]>, pane: &str) -> Option<u64> {
    clients?
        .iter()
        .filter(|client| client.pane == pane)
        .filter_map(|client| client.activity)
        .max()
}

/// Fold this cycle's `observed` input into the pane's memory for the seat
/// `identity`: a memory held for another seat is dropped, never read.
fn remember_input(
    held: Option<(u64, u64)>,
    identity: u64,
    observed: Option<u64>,
) -> Option<(u64, u64)> {
    let kept = held
        .filter(|(owner, _)| *owner == identity)
        .map(|(_, at)| at);
    kept.max(observed).map(|at| (identity, at))
}

/// One seat after this cycle's accounting, as the auto reseat step reads it.
#[derive(Debug, Clone)]
struct AutoSeat {
    slot: String,
    agent: String,
    profile: String,
    pane: crate::autoreseat::Pane,
    /// Its usage-limit latch stands: the seat is the limit path's alone.
    limited: bool,
    /// Its own account against the headroom threshold, from this cycle's held
    /// quota observation.
    room: crate::autoreseat::Room,
    /// Its `[auto_reseat]` row names nothing, but its own family does.
    derives: bool,
    /// This cycle's read of it against the session's resume.
    start: crate::autoreseat::StartRead,
    /// Its latched episode's carried binding reset: a near short one waits
    /// for the wake instead of moving ([`crate::autoreseat::reset_wait`]).
    reset: Option<crate::autoreseat::LimitReset>,
    /// The reset epoch of its newest wake the journal proves LANDED: the seat
    /// resumed on that reset, which is no reason to move it.
    wake_landed: Option<i64>,
    /// The reset its carry saw the tool's own resume armed for: the vendor
    /// resumes it, which plays the landed role.
    armed: Option<i64>,
}

/// What the auto reseat step does this cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AutoAct {
    /// Hand `agent` to the trigger.
    Trigger { agent: String },
    /// Say, once per episode, that the limited seat has no candidate at all.
    Unmapped {
        slot: String,
        agent: String,
        key: Timestamp,
        profile: String,
    },
    /// Say, once per episode, that the last round was refused or failed and
    /// no attempt follows.
    GaveUp {
        slot: String,
        agent: String,
        key: Timestamp,
        profile: String,
    },
    /// Name why the due seat is not moved, once per episode and reason.
    Held {
        slot: String,
        agent: String,
        key: Timestamp,
        reason: crate::autoreseat::HoldReason,
        trigger: crate::autoreseat::Trigger,
    },
    /// Close an attempt that passed its bound with no outcome.
    Overdue {
        slot: String,
        agent: String,
        key: Timestamp,
        trigger: crate::autoreseat::Trigger,
    },
    /// Open a headroom episode: `cause` names the crossing.
    Open {
        slot: String,
        agent: String,
        cause: String,
    },
    /// End the headroom episode keyed `key`: the account reads relieved.
    Rearm {
        slot: String,
        agent: String,
        key: Timestamp,
        summary: String,
    },
    /// Open a return episode: `cause` names the origin and its room (#305).
    OpenReturn {
        slot: String,
        agent: String,
        cause: String,
    },
    /// Say, once per chain keyed by its newest move `key`, that the seat went
    /// home too often to go home again by itself.
    Flapped {
        slot: String,
        agent: String,
        key: Timestamp,
        origin: String,
    },
}

/// The auto reseat step's decisions, PURE: every latched seat the switch
/// admits is decided by [`crate::autoreseat::decide`], each hold is named once
/// per episode and reason (`named` is that memory), and at most ONE seat is
/// triggered — the oldest episode, the slot breaking a tie.
///
/// A seat whose latch does not stand is the HEADROOM path's: its episode opens
/// on a crossing, re-arms on relief, and is decided like a limit episode while
/// its account reads at or past the CURRENT threshold; within the gap below it
/// the episode stays open and moves nothing (#311). A headroom hold the
/// journal already ends on is not named again, a restart included.
///
/// A seat no limit or headroom path holds goes home by [`return_route`] when
/// `homes` names its slot (#305); a seat with no such entry decides as it
/// always did.
#[allow(clippy::too_many_lines, reason = "one ordered decision per seat")]
fn auto_acts_with(
    settings: &crate::autoreseat::Settings,
    (seats, homes): (&[AutoSeat], &[(String, crate::autoreseat::leg::Home)]),
    events: &[Event],
    (session, orchestrator, lead_pair): (&str, bool, bool),
    now: i64,
    named: &mut Vec<(String, Timestamp, crate::autoreseat::HoldReason)>,
) -> Vec<AutoAct> {
    use crate::autoreseat::{Decision, Ineligible, Room, Trigger};
    named.retain(|(slot, _, _)| seats.iter().any(|seat| seat.slot == *slot));
    let home = |seat: &AutoSeat| {
        homes
            .iter()
            .find(|(slot, _)| *slot == seat.slot)
            .map(|(_, found)| found)
    };
    let mut acts = Vec::new();
    let mut due: Option<(Timestamp, &AutoSeat)> = None;
    for seat in seats {
        // Every decision below is the seat's own: the lead pair reads its own
        // headroom threshold when the row is set.
        let view = settings.seat_view(lead_pair, &seat.slot);
        let settings = &*view;
        let asked = crate::autoreseat::Seat {
            session,
            slot: &seat.slot,
            agent: &seat.agent,
            profile: &seat.profile,
            orchestrator,
        };
        let back = crate::autoreseat::return_episode(events, session, &seat.slot, &seat.agent);
        // The origin is a return's one candidate, so a seat with no row of its
        // own may still go home: only the return path decides it then.
        let homeward_only = match crate::autoreseat::eligible(settings, &asked) {
            Err(Ineligible::Unmapped) if seat.derives => false,
            Err(Ineligible::Unmapped) if seat.limited => {
                acts.extend(unmapped_act(settings, seat, events, session, now));
                continue;
            }
            Err(Ineligible::Unmapped) if home(seat).is_some() || back.is_some() => true,
            Err(_) => continue,
            Ok(_) => false,
        };
        let limit = crate::autoreseat::episode(events, session, &seat.slot, &seat.agent);
        let headroom =
            crate::autoreseat::headroom_episode(events, session, &seat.slot, &seat.agent);
        // An attempt in flight, of either kind, decides the seat until it ends.
        let flying =
            crate::autoreseat::flying_any(&[limit.as_ref(), headroom.as_ref(), back.as_ref()]);
        // A seat its own account presses is the headroom path's: it goes home
        // only with no headroom episode at all (#305).
        let pressed = headroom.is_some();
        let found = if homeward_only {
            match flying {
                Some(open) => (open.trigger == Trigger::Return).then_some(open),
                None if pressed => None,
                None => match return_route(seat, home(seat), back) {
                    Homeward::Act(act) => {
                        acts.push(act);
                        continue;
                    }
                    Homeward::Decide(found) => found,
                },
            }
        } else if flying.is_some() {
            flying
        } else if seat.limited {
            limit
        } else {
            match headroom_route(settings, seat, headroom, now) {
                Headroom::Act(act) => {
                    acts.push(act);
                    continue;
                }
                Headroom::Decide(None) if !pressed => match return_route(seat, home(seat), back) {
                    Homeward::Act(act) => {
                        acts.push(act);
                        continue;
                    }
                    Homeward::Decide(found) => found,
                },
                Headroom::Decide(found) => found,
            }
        };
        let Some(found) = found else {
            continue;
        };
        if found.gave_up() {
            if !found.holds_with(crate::autoreseat::GAVE_UP_SUMMARY) {
                let (slot, agent) = (seat.slot.clone(), seat.agent.clone());
                let (key, profile) = (found.key, seat.profile.clone());
                acts.push(AutoAct::GaveUp {
                    slot,
                    agent,
                    key,
                    profile,
                });
            }
            continue;
        }
        if waits_for_reset(seat, &found, now) {
            continue;
        }
        let (slot, agent, key, trigger) = (
            seat.slot.clone(),
            seat.agent.clone(),
            found.key,
            found.trigger,
        );
        // A headroom episode moves only while its account reads at or past the
        // CURRENT threshold; within the gap below it the episode stays open and
        // waits (#311). An attempt already open is judged whatever it reads.
        let urgent = seat.start.urgent(key.epoch());
        let near = urgent || matches!(seat.room, Room::Crossed(_));
        // A return takes the plain grace and the plain frame: a resume's
        // waivers are for an exhausted start, never for going home.
        let homeward = trigger == Trigger::Return;
        let waits = if homeward {
            crate::autoreseat::Waits::plain(settings.grace_secs)
        } else {
            seat.start.waits(key.epoch(), settings.grace_secs)
        };
        let mut pane = seat.pane;
        if !homeward {
            pane.frame = seat.start.frame(key.epoch(), pane.frame);
        }
        match crate::autoreseat::decide_with_waits(Some(&found), waits, &pane, now) {
            Decision::Overdue => acts.push(AutoAct::Overdue {
                slot,
                agent,
                key,
                trigger,
            }),
            Decision::Hold(_) | Decision::Attempt if trigger == Trigger::Headroom && !near => {}
            // A return's holds are silent: a working seat is no news (M6).
            Decision::Hold(_) if homeward => {}
            Decision::Hold(reason) => {
                let said = trigger == Trigger::Headroom
                    && found
                        .holds_with(&crate::autoreseat::leg::held_summary(reason, found.cause()));
                if name_hold(named, &slot, key, reason, said) {
                    acts.push(AutoAct::Held {
                        slot,
                        agent,
                        key,
                        reason,
                        trigger,
                    });
                }
            }
            Decision::Attempt => {
                if due.is_none_or(|(at, first)| (key, &seat.slot) < (at, &first.slot)) {
                    due = Some((key, seat));
                }
            }
            Decision::Rest | Decision::Wait { .. } | Decision::InFlight => {}
        }
    }
    acts.extend(due.map(|(_, seat)| AutoAct::Trigger {
        agent: seat.agent.clone(),
    }));
    acts
}

/// Whether the hold of `slot`'s episode `key` for `reason` is named this
/// cycle, remembering it in `named` if so. The slot's memory of any other
/// episode is dropped first, whatever is said; a hold the journal already
/// ends on (`said`) or one named before is not named again.
fn name_hold(
    named: &mut Vec<(String, Timestamp, crate::autoreseat::HoldReason)>,
    slot: &str,
    key: Timestamp,
    reason: crate::autoreseat::HoldReason,
    said: bool,
) -> bool {
    named.retain(|(at, held, _)| at != slot || *held == key);
    let fresh = !said && !named.contains(&(slot.to_owned(), key, reason));
    if fresh {
        named.push((slot.to_owned(), key, reason));
    }
    fresh
}

/// What the auto reseat step says this cycle of a limited seat with no
/// candidate at all: named once per episode, while that limit episode is
/// still owed a move — a terminal episode, one the journal already ends on
/// that hold, or one left to its reset's wake says nothing, and neither does
/// one inside the grace or with an attempt open.
fn unmapped_act(
    settings: &crate::autoreseat::Settings,
    seat: &AutoSeat,
    events: &[Event],
    session: &str,
    now: i64,
) -> Option<AutoAct> {
    use crate::autoreseat::{Decision, Ineligible};
    let said = crate::autoreseat::leg::ineligible_record(Ineligible::Unmapped);
    let open =
        crate::autoreseat::episode(events, session, &seat.slot, &seat.agent).filter(|open| {
            open.terminal.is_none() && !open.holds_with(&said) && !waits_for_reset(seat, open, now)
        })?;
    let decision = crate::autoreseat::decide(Some(&open), settings.grace_secs, &seat.pane, now);
    matches!(decision, Decision::Attempt | Decision::Hold(_)).then(|| AutoAct::Unmapped {
        slot: seat.slot.clone(),
        agent: seat.agent.clone(),
        key: open.key,
        profile: seat.profile.clone(),
    })
}

/// Where the return path leaves a seat no limit or headroom path holds.
#[derive(Debug, PartialEq)]
enum Homeward {
    /// Record this act: the seat is done this cycle.
    Act(AutoAct),
    /// Decide this return episode like a limit one; none leaves the seat be.
    Decide(Option<crate::autoreseat::Episode>),
}

/// The return path of one seat, given its return episode `back` (#305): a
/// chain whose origin proves room opens one where none is — or, past
/// [`crate::autoreseat::MAX_RETURNS`], holds once instead — and an open one is
/// decided only while that proof stands. Anything less is silent.
fn return_route(
    seat: &AutoSeat,
    home: Option<&crate::autoreseat::leg::Home>,
    back: Option<crate::autoreseat::Episode>,
) -> Homeward {
    let Some(home) = home else {
        return Homeward::Decide(None);
    };
    let (slot, agent) = (seat.slot.clone(), seat.agent.clone());
    match (back, home.ready.as_ref()) {
        (None, Some(_)) if home.flapped => Homeward::Act(AutoAct::Flapped {
            slot,
            agent,
            key: home.newest,
            origin: home.origin.clone(),
        }),
        (None, Some(window)) => Homeward::Act(AutoAct::OpenReturn {
            slot,
            agent,
            cause: crate::autoreseat::return_words(&home.origin, window),
        }),
        (Some(open), Some(_)) => Homeward::Decide(Some(open)),
        (Some(open), None) if open.gave_up() => Homeward::Decide(Some(open)),
        _ => Homeward::Decide(None),
    }
}

/// Where the headroom path leaves a seat whose latch does not stand.
#[derive(Debug, PartialEq)]
enum Headroom {
    /// Record this act: the seat is done this cycle.
    Act(AutoAct),
    /// Decide this episode like a limit one; none leaves the seat be.
    Decide(Option<crate::autoreseat::Episode>),
}

/// The headroom path of one seat, given its headroom episode `found`: a
/// start's urgent read or a crossing opens an episode where none is; an
/// earlier start's episode ends when a new start reads its own account, and
/// relief ends one no attempt moves while no start is urgent. Otherwise the
/// episode is decided, but with no threshold only a start's own episode is.
fn headroom_route(
    settings: &crate::autoreseat::Settings,
    seat: &AutoSeat,
    found: Option<crate::autoreseat::Episode>,
    now: i64,
) -> Headroom {
    use crate::autoreseat::Room;
    let (slot, agent) = (seat.slot.clone(), seat.agent.clone());
    let opening = seat.start.urgent(now);
    let earlier = |open: &crate::autoreseat::Episode| {
        opening && seat.start.settled.is_some_and(|at| open.key.epoch() < at)
    };
    match (&found, &seat.room) {
        (None, _) if opening => {
            let cause = seat
                .start
                .exhausted
                .as_ref()
                .map(crate::autoreseat::headroom_words)
                .unwrap_or_default();
            return Headroom::Act(AutoAct::Open { slot, agent, cause });
        }
        (None, Room::Crossed(window)) if settings.headroom_at.is_some() => {
            let cause = crate::autoreseat::headroom_words(window);
            return Headroom::Act(AutoAct::Open { slot, agent, cause });
        }
        // Each start answers for itself: an earlier start's episode
        // ends, and the next cycle opens this one's.
        (Some(open), _) if open.open.is_none() && earlier(open) => {
            let summary = crate::autoreseat::RESTARTED_SUMMARY.to_owned();
            let key = open.key;
            return Headroom::Act(AutoAct::Rearm {
                slot,
                agent,
                key,
                summary,
            });
        }
        (Some(open), Room::Relieved(window)) if open.open.is_none() && !opening => {
            let summary = crate::autoreseat::relieved_words(window);
            let key = open.key;
            return Headroom::Act(AutoAct::Rearm {
                slot,
                agent,
                key,
                summary,
            });
        }
        _ => {}
    }
    // With no threshold, only a start's own episode moves.
    if settings.headroom_at.is_none()
        && !found
            .as_ref()
            .is_some_and(|open| seat.start.urgent(open.key.epoch()))
    {
        return Headroom::Decide(None);
    }
    Headroom::Decide(found)
}

/// Whether a limit episode with no attempt open is left to the seat's wake
/// this cycle, silently — no trigger and no naming: its known short reset is
/// near or not yet past its fallback ([`crate::autoreseat::reset_wait`]), or
/// the carried reset has passed and the seat resumed on it — the journal
/// proves its wake LANDED, or the tool's own resume was armed for it. That
/// second hold never widens the wait to a reset still ahead, and a fresh
/// reading naming a new binding reset ends it. An open attempt is never
/// masked, and headroom is not this path's.
fn waits_for_reset(seat: &AutoSeat, episode: &crate::autoreseat::Episode, now: i64) -> bool {
    let resumed = |reset: crate::autoreseat::LimitReset| {
        now >= reset.at && (seat.wake_landed == Some(reset.at) || seat.armed == Some(reset.at))
    };
    episode.trigger == crate::autoreseat::Trigger::Limit
        && episode.open.is_none()
        && (crate::autoreseat::reset_wait(seat.reset, now).is_some()
            || seat.reset.is_some_and(resumed))
}

/// Whether a cycle reads the seat's binding reset: only on its limit — the
/// notice drawn, or the latch standing — the one window a wake judges.
fn reads_reset(throttle: Option<Throttle>, latched: bool) -> bool {
    throttle == Some(Throttle::LimitReached) || latched
}

/// The tool's own resume as this cycle READ it: a failed capture reads
/// nothing armed, and the latch never takes it for a cancel either.
fn armed_reading(capture_ok: bool, capture: &str, tool: crate::tool::ToolKind) -> bool {
    capture_ok && crate::harness_state::auto_continue_armed(capture, tool)
}

/// The first-sight forecast of a seat auto reseat leaves to its reset's wake
/// ([`crate::autoreseat::reset_wait`]), or `None` when the move applies.
fn waiting_forecast(
    agent: &str,
    reset: Option<crate::autoreseat::LimitReset>,
    now: i64,
) -> Option<String> {
    let held = reset?;
    crate::autoreseat::reset_wait(reset, now)?;
    Some(format!(
        "ae waits for {agent}'s limit to reset at {} and wakes it then; it moves only if the wake does not land",
        Timestamp::from_epoch(held.at)
    ))
}

/// Everything one cycle needs that does not change within it.
struct Cycle<'a> {
    knobs: Knobs,
    meta_dir: &'a Path,
    helper: &'a SendHelper,
    server: &'a crate::inventory::ServerId,
    session: &'a str,
    goal: Option<String>,
    roster: Vec<RosterEntry>,
    local_config: Option<std::path::PathBuf>,
    lead_pair: bool,
    /// The human's fleet order as the GLOBAL config spells it right now. Read
    /// per cycle beside the quota awareness, so a config edit reaches a running
    /// session within one cycle and never needs a relaunch.
    fleet_order: theme::FleetOrder,
    /// `meta_agent=true` — this session is the fleet orchestrator.
    meta_agent: bool,
    /// Each seat's recorded `launch_id.<slot>` — the compare-and-swap guard an
    /// observed-model write publishes under, so a re-created slot cannot
    /// inherit the old seat's observation.
    launch_ids: Vec<(String, String)>,
    /// The session's start, as `ae list` reads it: alerts older are not news.
    started: Option<i64>,
    /// When the resume this meta records settled.
    settled: Option<i64>,
    /// The global auto reseat settings, read per cycle beside the fleet order,
    /// so turning the switch off stops the next move.
    auto: crate::autoreseat::Settings,
    /// The global `auto_trust` switch, read per cycle beside them.
    auto_trust: crate::config::AutoTrust,
}

/// The fleet text and durable gate observed once for the orchestrator cycle.
#[derive(Debug, Clone)]
struct OverviewReading {
    rendered: String,
    hash: String,
    checkpoint: crate::monitor::OverviewCheckpoint,
}

impl OverviewReading {
    fn changed(&self) -> bool {
        self.checkpoint.hash.as_deref() != Some(self.hash.as_str())
    }

    fn persisted_outstanding_since(&self) -> Option<SystemTime> {
        system_time_from_epoch(self.checkpoint.outstanding_since?)
    }

    fn persisted_last_delivery(&self) -> Option<SystemTime> {
        let epoch = self
            .checkpoint
            .last_delivered_at
            .or(self.checkpoint.outstanding_since)?;
        system_time_from_epoch(epoch)
    }

    fn body(&self) -> String {
        crate::overview::nudge_body(&self.rendered)
    }
}

/// What [`Cycle::apply`] is acting on: one pane, and the cycle-wide readings an
/// effect may need.
struct Acting<'a> {
    agent: &'a str,
    /// The pane's `@ae_slot`, for the routed identity an event may carry.
    slot: &'a str,
    /// The pane id, which a trust answer presses keys into.
    pane: &'a str,
    seen: &'a Observation,
    /// This cycle's events, in APPEND order — what the durable reconcile reads.
    events: &'a [Event],
    /// The cycle-wide fleet overview, present only for the orchestrator main.
    overview: Option<&'a OverviewReading>,
}

impl Cycle<'_> {
    /// Apply one pane's booked effects, collecting the cycle-level quota
    /// request: the recovery pass belongs to the sweep as a whole.
    fn apply_booked(
        &self,
        effects: &[Effect],
        on: &Acting<'_>,
        state: &mut PaneState,
        quota_refresh: &mut bool,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        for effect in effects {
            if matches!(effect, Effect::QuotaRefresh) {
                *quota_refresh = true;
                continue;
            }
            self.apply(effect, on, state, err)?;
        }
        Ok(())
    }

    /// The ONE recovery pass a release requests: the same refresh the due
    /// path calls, invoked directly — never through the due counter — so the
    /// cadence keeps its own schedule. `quota = off` runs none.
    fn refresh_after_limit_release(
        &self,
        requested: bool,
        carry: &mut QuotaCarry,
        inputs: &QuotaInputs<'_>,
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        if requested && self.knobs.quota_aware {
            self.refresh_quota(carry, inputs, now, err)?;
        }
        Ok(())
    }

    /// The state root, the global config and the home the session-relative
    /// quota inputs are read from.
    fn quota_paths(&self) -> (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>) {
        let root = crate::doors::state_root(crate::shape::current()).or_else(|| {
            self.meta_dir
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        });
        let global = root
            .as_deref()
            .map(|root| crate::doors::config_file(crate::shape::current(), root));
        (root, global, crate::doors::home())
    }

    /// The same session-relative quota inputs as the generated `quota` helper.
    fn quota_observation(
        &self,
        now: i64,
    ) -> Result<crate::quota::Observation, crate::config::ConfigError> {
        let (root, global, home) = self.quota_paths();
        let roots = root.as_deref().map(crate::inventory::Roots::under);
        crate::quota::observe(&crate::quota::Inputs {
            home: home.as_deref(),
            global: global.as_deref(),
            local: self.local_config.as_deref(),
            sessions: roots.as_ref().map(crate::inventory::Roots::sessions),
            now,
        })
    }

    fn apply_quota_actions(
        &self,
        carry: &mut QuotaCarry,
        actions: Vec<QuotaAction>,
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        for action in actions {
            match action {
                QuotaAction::Dropped { recipient, summary } => {
                    self.emit(
                        crate::quota::action::ADVISORY_DROPPED,
                        &recipient,
                        &summary,
                        err,
                    )?;
                }
                QuotaAction::AskDropped { recipient, summary } => {
                    self.emit(
                        crate::quota::action::CHECKPOINT_DROPPED,
                        &recipient,
                        &summary,
                        err,
                    )?;
                }
                QuotaAction::Cancelled(summary) => {
                    // NO target: news to nobody, so no fold reads it.
                    self.emit(
                        crate::quota::action::CHECKPOINT_CANCELLED,
                        "",
                        &summary,
                        err,
                    )?;
                }
                QuotaAction::Deliver(pending) => {
                    let text = pending.advisory.render(self.meta_dir, now);
                    let delivery = self.deliver(
                        &pending.recipient.agent,
                        &text,
                        crate::quota::action::ADVISORY,
                        &text,
                        None,
                    );
                    if let Some(QuotaAction::Dropped { recipient, summary }) = carry
                        .record_delivery(&pending, quota_delivery(&delivery), self.meta_dir, now)
                    {
                        self.emit(
                            crate::quota::action::ADVISORY_DROPPED,
                            &recipient,
                            &summary,
                            err,
                        )?;
                    }
                }
                QuotaAction::Ask(ask) => {
                    // The SAME sender as the advisory — one delivery owner, so
                    // the seat reads one marker, written by `provenance`. The
                    // receipt rides the event the helper writes on delivery.
                    let text = ask.advisory.checkpoint_ask(self.meta_dir, now);
                    let receipt = ask_receipt(&ask.key, ask.advisory.resets_at(), &ask.recipient);
                    let delivery = self.deliver(
                        &ask.recipient.agent,
                        &text,
                        crate::quota::action::CHECKPOINT,
                        &text,
                        receipt.as_deref(),
                    );
                    if let Some(QuotaAction::Dropped { recipient, summary }) = carry
                        .record_ask_delivery(&ask, quota_delivery(&delivery), self.meta_dir, now)
                    {
                        self.emit(
                            crate::quota::action::CHECKPOINT_DROPPED,
                            &recipient,
                            &summary,
                            err,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    fn history_seat(
        &self,
        entry: &RosterEntry,
        inputs: &QuotaInputs<'_>,
        now: i64,
    ) -> (crate::quota::history::Seat, Option<i64>) {
        let launch = launch_id_for(&self.launch_ids, &entry.slot);
        let held = inputs
            .panes
            .iter()
            .find(|pane| {
                pane.slot.as_deref() == Some(entry.slot.as_str())
                    && pane.agent.as_deref() == Some(entry.name.as_str())
            })
            .and_then(|pane| {
                inputs
                    .held
                    .iter()
                    .find(|(id, _)| id == &pane.pane_id)
                    .map(|(_, state)| state)
            });
        let capture = held
            .and_then(|state| state.history_capture.as_ref())
            .filter(|(_, _, id)| id.as_deref() == launch);
        let clock = history_clock(capture.map(|(at, _, _)| *at), now);
        let identity = held
            .and_then(|state| state.held_identity.as_ref())
            .filter(|hold| Some(hold.launch.as_str()) == launch && clock.is_some());
        let declared = latest_relevant_event(inputs.events, self.session, &entry.slot, &entry.name)
            .filter(crate::watchdog::declaration_current)
            .and_then(|relevant| relevant.event.declared_state())
            .map(str::to_owned);
        (
            crate::quota::history::Seat {
                slot: entry.slot.clone(),
                agent: entry.name.clone(),
                launch: launch.map(str::to_owned),
                observed: clock
                    .and(capture)
                    .map_or("unknown", |(_, state, _)| state.as_str())
                    .to_owned(),
                declared,
                model: identity.and_then(|hold| hold.identity.model.clone()),
                effort: identity.and_then(|hold| hold.identity.effort.clone()),
            },
            clock,
        )
    }

    fn write_quota_history(
        &self,
        carry: &mut QuotaCarry,
        observation: &crate::quota::Observation,
        inputs: &QuotaInputs<'_>,
        now: i64,
        err: &mut impl Write,
    ) {
        use crate::quota::history::{self, Row, Workload};
        if !self.knobs.quota_aware || self.knobs.quota_every_secs == 0 {
            return;
        }
        let incarnation = crate::meta::session_id_in(self.meta_dir);
        let mut current: Vec<_> = quota_samples_at(observation, now)
            .iter()
            .filter_map(|s| {
                crate::quota::history_sample(s.group, &s.reading, self.session, &incarnation, now)
            })
            .collect();
        let mut rows: Vec<_> = current
            .iter()
            .filter(|s| !carry.history_seen.iter().any(|held| history_same(held, s)))
            .cloned()
            .map(Row::Sample)
            .collect();
        let scopes: std::collections::BTreeSet<_> = observation
            .groups
            .iter()
            .filter_map(crate::quota::history_scope)
            .collect();
        let identities: Vec<_> = self
            .roster
            .iter()
            .map(|entry| (entry, crate::quota::recorded_identity(entry)))
            .collect();
        let unscoped = identities
            .iter()
            .filter(|(_, identity)| identity.is_none())
            .count();
        for scope in scopes {
            let mut seats = Vec::new();
            let mut observed_at: Option<i64> = None;
            for (entry, _) in identities.iter().filter(|(_, identity)| {
                identity.as_ref().is_some_and(|id| {
                    id.tool.as_str() == scope.tool
                        && id.source.to_str() == Some(scope.source.as_str())
                })
            }) {
                let (seat, clock) = self.history_seat(entry, inputs, now);
                if let Some(at) = clock {
                    observed_at = Some(observed_at.map_or(at, |old| old.min(at)));
                }
                seats.push(seat);
            }
            rows.push(Row::Workload(Workload {
                producer: self.session.to_owned(),
                incarnation: incarnation.clone(),
                collected_at: now,
                scope,
                observed_at,
                seats,
                unscoped,
            }));
        }
        match history::append(self.meta_dir, &rows, now) {
            Ok(()) => {
                current.truncate(256);
                carry.history_seen = current;
            }
            Err(why) => {
                let day = now.div_euclid(86_400);
                if carry.history_note_day != Some(day) {
                    carry.history_note_day = Some(day);
                    let _ = writeln!(err, "ae: watchdog: quota history skipped: {why}");
                }
            }
        }
    }

    fn refresh_quota(
        &self,
        carry: &mut QuotaCarry,
        inputs: &QuotaInputs<'_>,
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        match self.quota_observation(now) {
            Ok(observation) => {
                let mut recipients = quota_recipients(&self.roster, self.lead_pair);
                // TWO sets, never one: the advisory's lead pair above, and
                // every seat proven to sit on a readable scope below.
                let mut candidates = quota_ask_candidates(&self.roster, inputs.panes);
                // A seat this cycle's auto step moves hears neither: a turn
                // on its account could only fail.
                let urgent = self.urgent_slots(&observation, inputs.panes, now);
                recipients.retain(|seat| !urgent.contains(&seat.slot));
                candidates.retain(|seat| !urgent.contains(&seat.recipient.slot));
                for candidate in &mut candidates {
                    candidate.live = live_model(
                        &candidate.recipient,
                        inputs.panes,
                        inputs.held,
                        &self.launch_ids,
                    );
                }
                let actions = carry.reconcile_pass(
                    &observation,
                    &recipients,
                    &candidates,
                    &delivered_receipts(inputs.events),
                    self.meta_dir,
                );
                let summary = Self::observed_summary(&observation, actions.len());
                // LAST: the trace attests a COMPLETED refresh — observation,
                // booking, and delivery all done — so awaiting the line
                // proves everything the test subsequently reads has already
                // happened. Tracing before delivery would attest booking
                // while the receipt was still absent.
                self.apply_quota_actions(carry, actions, now, err)?;
                self.write_quota_history(carry, &observation, inputs, now, err);
                Self::trace_quota(&summary);
                Ok(())
            }
            Err(why) => {
                writeln!(
                    err,
                    "ae: watchdog: quota observation failed — skipped: {why}"
                )?;
                Ok(())
            }
        }
    }

    /// Read the fleet through [`crate::fleet_read::current_world`], enrich it through the
    /// same card collector as `ae brief --all`, then touch the watchdog-owned
    /// checkpoint without advancing the last-delivered hash.
    fn overview(&self, now: i64, err: &mut impl Write) -> Option<OverviewReading> {
        let Some(root) = crate::doors::state_root(crate::shape::current()) else {
            let _ = writeln!(
                err,
                "ae: watchdog: no state root for fleet overview — skipped"
            );
            return None;
        };
        let (_, world) = crate::fleet_read::current_world(&root);
        let sessions = crate::inventory::Roots::under(&root);
        let cards: Vec<crate::brief::Card> = world
            .sessions
            .iter()
            .filter(|entry| entry.status == Status::Running && entry.name != self.session)
            .map(|entry| {
                crate::brief::card_for(
                    entry,
                    &sessions.sessions().join(&entry.name),
                    None,
                    false,
                    world.now,
                    None,
                )
            })
            .collect();
        let rendered = crate::overview::render(&cards, self.session);
        let hash = crate::overview::semantic_hash(&cards, self.session);
        let checkpoint = match crate::monitor::overview_heartbeat(self.meta_dir, now) {
            Ok(checkpoint) => checkpoint,
            Err(why) => {
                let _ = writeln!(
                    err,
                    "ae: watchdog: fleet overview checkpoint failed — skipped: {why}"
                );
                return None;
            }
        };
        Some(OverviewReading {
            rendered,
            hash,
            checkpoint,
        })
    }

    /// The orchestrator-main-only observation, including the durable hash and
    /// spacing gate recovered with this cycle's overview.
    fn sweep_observation(
        &self,
        slot: &str,
        agent: &str,
        events: &[Event],
        overview: Option<&OverviewReading>,
        now: i64,
    ) -> Option<SweepObservation> {
        is_sweep_target(self.meta_agent, slot).then(|| {
            SweepObservation::new(
                system_time_from_epoch(now).unwrap_or(UNIX_EPOCH),
                last_done_event_at(events, self.session, agent),
            )
            .with_working_since(last_working_declaration_at(events, self.session, agent))
            .with_overview(
                overview.is_some_and(OverviewReading::changed),
                overview.and_then(OverviewReading::persisted_outstanding_since),
                overview.and_then(OverviewReading::persisted_last_delivery),
            )
        })
    }

    /// The worst exact-match quota row for this slot — `None` unless the pane
    /// is ACTUALLY throttled, because that is the only reading it explains.
    /// Unaware sessions never hold an observation (the cycle skips the quota
    /// read entirely), so no quota line can render there either.
    fn throttle_quota(
        &self,
        quota: &QuotaCarry,
        slot: &str,
        now: i64,
        throttled: bool,
    ) -> Option<String> {
        if !throttled {
            return None;
        }
        self.roster
            .iter()
            .find(|entry| entry.slot == slot)
            .and_then(|entry| {
                quota.last_observation.as_ref().and_then(|observation| {
                    throttle_quota_line(observation, &quota.tracked, entry, self.meta_dir, now)
                })
            })
    }

    fn harness_observation(
        &self,
        capture: &str,
        tool: crate::tool::ToolKind,
        events: &[Event],
        slot: &str,
        agent: &str,
    ) -> HarnessObservation {
        let declaration = crate::session::latest_declaration_in(events, self.session, slot, agent)
            .map(|event| quiet_hash(&declaration_key(event)));
        HarnessObservation {
            frame: crate::harness_state::classify(capture, tool),
            human_draft: crate::harness_state::has_human_draft(capture, tool),
            durable_stale: crate::session::alert_reason_in(events, self.session, slot, agent)
                == Some(crate::attention::Reason::Stale),
            declaration,
        }
    }

    /// Record this pane's observed model, when the tool's live model is
    /// readable. Best-effort: a refused write (a stale guard, a failed read) is
    /// the next cycle's problem, never this cycle's.
    ///
    /// `pin` is [`Self::seat_pin`]'s answer for THIS cycle, read fresh there
    /// and handed to both consumers so one config read serves both; an
    /// operator edit while the session runs is visible on the next cycle.
    fn note_model(
        &self,
        capture: &str,
        tool: crate::tool::ToolKind,
        slot: &str,
        pins: &crate::model_drift::Pins,
    ) {
        if !tool.adapter().model.observes() {
            return;
        }
        let Some(entry) = self.roster.iter().find(|entry| entry.slot == slot) else {
            return;
        };
        // A seat with no profile has no pin to disagree with, and the durable
        // row is about that disagreement: it writes nothing here, as it never
        // has. `pin` cannot stand in — it is `None` for an unpinned profile too.
        if entry.profile.is_none() {
            return;
        }
        let Some((_, launch_id)) = self.launch_ids.iter().find(|(seat, _)| seat == slot) else {
            return;
        };
        let _ = crate::model_drift::observe(
            self.meta_dir,
            (slot, &entry.name),
            tool,
            capture,
            launch_id,
            pins,
        );
    }

    /// This seat's profile model pin for this cycle, or `None` when the tool's
    /// live model is not one ae observes at all.
    ///
    /// Read ONCE per pane per cycle: the durable drift row and the picker's
    /// drift mark are two questions about the same pin.
    fn seat_pin(&self, slot: &str, tool: crate::tool::ToolKind) -> crate::model_drift::Pins {
        let profile = self
            .roster
            .iter()
            .find(|entry| entry.slot == slot)
            .and_then(|entry| entry.profile.as_deref());
        match profile {
            Some(profile) if tool.adapter().model.observes() => self.profile_pins(profile, tool),
            _ => crate::model_drift::Pins::default(),
        }
    }

    /// What this cycle will SHOW for one seat's model, holding the last proven
    /// answer while the frame cannot be read.
    ///
    /// The read is the GATED one: a picker cell is a claim about what the seat
    /// is running now, so a frame whose composer is absent proves nothing here
    /// even when its grammar would parse. The hold then covers the ordinary
    /// gaps — a turn in flight, a draft in the box, one failed capture —
    /// bounded by [`HOLD_MAX_CYCLES`] and by the seat's own launch id. A DEAD
    /// seat clears it outright: whatever it was running, it is not now.
    fn resolve_identity(
        &self,
        carried: &mut PaneState,
        seen: &ResolveIdentity<'_>,
    ) -> SeatIdentity {
        if seen.verdict == Verdict::Dead {
            carried.held_identity = None;
            return SeatIdentity::default();
        }
        let launch = self
            .launch_ids
            .iter()
            .find(|(seat, _)| seat == seen.slot)
            .map(|(_, launch)| launch.clone());
        let observed = crate::harness_state::observed_identity(seen.capture, seen.tool);
        if let Some(model) = observed.model {
            let identity = SeatIdentity {
                drift: matches!(
                    crate::model_drift::decide(
                        &crate::harness_state::HarnessIdentity {
                            model: Some(model.clone()),
                            effort: None,
                        },
                        seen.pin,
                        seen.tool.adapter().pin_match,
                    ),
                    // A profile that pins NO model has nothing to drift from;
                    // that is a report, not a disagreement.
                    crate::model_drift::Decision::Record { pin: Some(_), .. }
                ),
                model: Some(model),
                effort: observed.effort,
            };
            carried.held_identity = launch.map(|launch| IdentityHold {
                launch,
                age: 0,
                identity: identity.clone(),
            });
            return identity;
        }
        match (&mut carried.held_identity, launch) {
            (Some(hold), Some(launch)) if hold.launch == launch && hold.age < HOLD_MAX_CYCLES => {
                hold.age = hold.age.saturating_add(1);
                hold.identity.clone()
            }
            (held, _) => {
                *held = None;
                SeatIdentity::default()
            }
        }
    }

    /// The profile's model flag value, from the same config files `_run`
    /// reads. An unreadable config is `None`, never a guess.
    fn profile_pins(&self, profile: &str, tool: crate::tool::ToolKind) -> crate::model_drift::Pins {
        let root = crate::doors::state_root(crate::shape::current());
        let global = root
            .as_deref()
            .map(|root| crate::doors::config_file(crate::shape::current(), root));
        crate::launch_cmd::profile_pins(
            global.as_deref(),
            self.local_config.as_deref(),
            profile,
            tool,
        )
    }

    /// The line the first-sight notice of `agent`'s limit gains when auto
    /// reseat may move it, ranked over this cycle's held quota observation.
    fn auto_deadline(
        &self,
        quota: &QuotaCarry,
        events: &[Event],
        (slot, agent): (&str, &str),
        now: i64,
    ) -> Option<String> {
        let profile = self.seat_profile(slot);
        let seat = crate::autoreseat::Seat {
            session: self.session,
            slot,
            agent,
            profile: &profile,
            orchestrator: self.meta_agent,
        };
        let declared = match crate::autoreseat::eligible(&self.auto, &seat) {
            Ok(list) => list,
            Err(crate::autoreseat::Ineligible::Unmapped) => &[],
            Err(_) => return None,
        };
        let row = self.roster.iter().find(|row| row.slot == slot)?;
        // A move the human's observed choice holds is never forecast (#269).
        if crate::run::observed_standing(self.meta_dir, slot).is_some() {
            return None;
        }
        let bytes = crate::meta::read_bytes(self.meta_dir).unwrap_or_default();
        let list = crate::autoreseat::leg::listed(self.meta_dir, &bytes, row, declared);
        if list.is_empty() {
            return None;
        }
        let left = crate::autoreseat::left_profiles(events, self.session, slot, agent);
        let left_on_headroom =
            crate::autoreseat::left_on_headroom(events, self.session, slot, agent);
        let latched =
            crate::autoreseat::latched_identities(&self.roster, events, self.session, slot);
        // `quota = off` holds no observation here: `QuotaCarry::clear_held`.
        let judged = crate::autoreseat::leg::candidates(
            &list,
            |to| crate::reseat::resolved(self.meta_dir, to),
            (&left, &left_on_headroom),
            quota.last_observation.as_ref(),
            &latched,
            now,
        );
        // The pair guard forecasts what the legs will choose (#305).
        let pair = crate::autoreseat::leg::pair_of(
            self.meta_dir,
            &bytes,
            &self.roster,
            self.lead_pair,
            slot,
        );
        // The seat's own view carries the lead floor, so the forecast never
        // promises a move the leg would refuse (#309).
        let auto = self.auto.seat_view(self.lead_pair, slot);
        let route = crate::autoreseat::Route::Move {
            seat_profile: &profile,
        };
        let choice =
            crate::autoreseat::choose_seat(&auto, route, &judged, now, None, pair.as_ref());
        Some(crate::autoreseat::leg::deadline(
            agent,
            &choice,
            self.auto.grace_secs,
        ))
    }

    /// Whether the seat at `slot`, eligible but for an `[auto_reseat]` row that
    /// names nothing, has a candidate in its own family. Asked only while the
    /// seat could act — `active` (on its limit, or near or past its threshold),
    /// or inside an episode — so a quiet or filtered seat costs no config read.
    fn derives(&self, events: &[Event], (slot, agent): (&str, &str), active: bool) -> bool {
        use crate::autoreseat::{Ineligible, Seat, eligible, episode, headroom_episode};
        let profile = self.seat_profile(slot);
        let seat = Seat {
            session: self.session,
            slot,
            agent,
            profile: &profile,
            orchestrator: self.meta_agent,
        };
        if eligible(&self.auto, &seat) != Err(Ineligible::Unmapped) {
            return false;
        }
        let live = active
            || headroom_episode(events, self.session, slot, agent).is_some()
            || episode(events, self.session, slot, agent).is_some_and(|found| found.open.is_some());
        live && self
            .roster
            .iter()
            .find(|row| row.slot == slot)
            .is_some_and(|row| {
                let bytes = crate::meta::read_bytes(self.meta_dir).unwrap_or_default();
                crate::autoreseat::leg::derivable(self.meta_dir, &bytes, row)
            })
    }

    /// One fresh read of the seat at `slot` against this session's resume.
    fn start_read(
        &self,
        quota: Option<&crate::quota::Observation>,
        (slot, pane): (&str, &str),
        pin: Option<&str>,
        now: i64,
    ) -> crate::autoreseat::StartRead {
        let Some(row) = self.roster.iter().find(|row| row.slot == slot) else {
            return crate::autoreseat::StartRead::default();
        };
        let profile = self.seat_profile(slot);
        let at = crate::autoreseat::leg::StartSeat {
            dir: self.meta_dir,
            row,
            seat: crate::autoreseat::Seat {
                session: self.session,
                slot,
                agent: &row.name,
                profile: &profile,
                orchestrator: self.meta_agent,
            },
            settled: self.settled,
            pane: Some(pane),
            pin,
        };
        crate::autoreseat::leg::start_read(&self.auto, &at, quota, now)
    }

    /// Whether auto reseat may move any seat of this session.
    fn any_seat_reachable(&self) -> bool {
        self.roster.iter().any(|row| {
            let profile = self.seat_profile(&row.slot);
            let seat = crate::autoreseat::Seat {
                session: self.session,
                slot: &row.slot,
                agent: &row.name,
                profile: &profile,
                orchestrator: self.meta_agent,
            };
            let bytes = crate::meta::read_bytes(self.meta_dir).unwrap_or_default();
            crate::autoreseat::leg::reachable(&self.auto, &seat, (self.meta_dir, &bytes), row)
        })
    }

    /// The slots a resume's exhausted reading moves now, by this observation.
    fn urgent_slots(
        &self,
        observation: &crate::quota::Observation,
        panes: &[crate::tmux::WatchPane],
        now: i64,
    ) -> Vec<String> {
        if self.settled.is_none() {
            return Vec::new();
        }
        panes
            .iter()
            .filter_map(|pane| {
                pane.slot
                    .as_deref()
                    .map(|slot| (slot, pane.pane_id.as_str()))
            })
            .filter(|(slot, pane)| {
                let bin = self.agent_bin(slot).unwrap_or_default();
                let tool = crate::tool::ToolKind::from_binary_name(&bin);
                let pin = self.seat_pin(slot, tool).model;
                self.start_read(Some(observation), (slot, pane), pin.as_deref(), now)
                    .urgent(now)
            })
            .map(|(slot, _)| slot.to_owned())
            .collect()
    }

    /// One quota-cadence pass: the vendor-quota observation and advisory
    /// booking, and only when aware. `quota = off` WINS over
    /// `quota_every_secs`: no quota read is even attempted unaware, while the
    /// cadence's due counter still advances.
    fn run_quota_cadence(
        &self,
        carry: &mut QuotaCarry,
        inputs: &QuotaInputs<'_>,
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let regular = quota_observation_due(carry, &self.knobs);
        // Zero disables the periodic pass, and this one with it; a session with
        // no seat auto reseat may move reads exactly as it did.
        let follow_up = self.knobs.quota_every_secs > 0
            && start_pass_due(carry.first_pass_at, carry.start_pass, self.settled, now)
            && self.any_seat_reachable();
        if follow_up {
            carry.start_pass = self.settled;
        }
        if regular || follow_up {
            carry.first_pass_at.get_or_insert(now);
            self.spawn_go_usage(now);
            if self.knobs.quota_aware {
                self.refresh_quota(carry, inputs, now, err)?;
            } else {
                Self::trace_quota("skipped");
            }
        }
        Ok(())
    }

    /// Refresh the shared `OpenCode` Go usage cache on a detached thread, at
    /// most one in flight. The thread's answer reaches every reader through
    /// that cache alone; this pass observes whatever the cache held before it.
    fn spawn_go_usage(&self, now: i64) {
        let (Some(root), global, home) = self.quota_paths() else {
            return;
        };
        let local = self.local_config.as_deref();
        let roots = crate::inventory::Roots::under(&root);
        let key = crate::quota::go_key(&crate::quota::Inputs {
            home: home.as_deref(),
            global: global.as_deref(),
            local,
            sessions: Some(roots.sessions()),
            now,
        });
        let due = crate::go_usage::watchdog_due(
            crate::quota::configured_quota_on(global.as_deref(), local),
            self.knobs.quota_aware,
            key.is_some(),
            GO_USAGE_FLYING.load(Ordering::SeqCst),
        );
        let Some(key) = key.filter(|_| due) else {
            return;
        };
        let flying = crate::go_usage::Flying::arm(&GO_USAGE_FLYING);
        // A refused spawn drops the closure, and with it the guard.
        let _ = std::thread::Builder::new()
            .name("ae-go-usage".to_owned())
            .spawn(move || {
                let _flying = flying;
                let _ = crate::go_usage::refresh(&root, &key, now);
            });
    }

    /// Append one attestation line to the test-only quota trace named by
    /// `AE_TEST_QUOTA_TRACE`. A `skipped` line marks a due pass that
    /// correctly performed no quota read; an `observed ...` line marks a
    /// COMPLETED refresh — observation, booking, AND delivery — and names
    /// the maximum window percentage it saw, so the it-test can await proof
    /// that everything it subsequently reads has already happened instead
    /// of racing the daemon with fixed sleeps. One line carries exactly one
    /// meaning: the observed line never stands for delivery, nor skipped
    /// for observation.
    ///
    /// CHECKOUT ONLY, exactly like `AE_TEST_BOOT_TIME` (`src/doors.rs`):
    /// the published shape returns before reading the variable, so the
    /// shipped watchdog has no such door. The integration tests drive the
    /// checkout binary, which is why their trace assertions keep working.
    ///
    /// Deliberately unexamined write, stated plainly: it opens the
    /// environment-given path with create+append and no `O_EXCL`, mode, or
    /// path validation. That is acceptable ONLY behind the checkout gate
    /// above and because the daemon already runs as the invoking user — a
    /// hostile path can at most append trace lines to a file that user could
    /// write anyway. See `AGENTS.md`'s doors table.
    fn trace_quota(line: &str) {
        if !crate::shape::current().honours_environment() {
            return;
        }
        #[allow(
            clippy::disallowed_methods,
            reason = "a checkout-only test door like AE_TEST_BOOT_TIME: unset in production, read on due passes only — see clippy.toml"
        )]
        let path = std::env::var_os("AE_TEST_QUOTA_TRACE");
        let Some(path) = path else { return };
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        else {
            return;
        };
        let _ = writeln!(file, "{line}");
    }

    /// The attestation one completed refresh reports to the test trace: the
    /// maximum window percentage it observed (`none` when no row stated one)
    /// and how many quota actions that observation booked — advisories,
    /// checkpoint asks and cancellations alike. The count is a progress signal
    /// for a test awaiting a completed pass, never a per-kind assertion.
    fn observed_summary(observation: &crate::quota::Observation, booked: usize) -> String {
        let mut max: Option<f64> = None;
        for group in &observation.groups {
            for row in &group.rows {
                if let Some(pct) = row
                    .used_percent
                    .as_deref()
                    .and_then(|text| text.parse::<f64>().ok())
                {
                    max = Some(max.map_or(pct, |held: f64| held.max(pct)));
                }
            }
        }
        format!(
            "observed max={} booked={booked}",
            max.map_or_else(|| "none".to_owned(), |value| format!("{value}"))
        )
    }

    /// One pass over the session's panes.
    #[allow(
        clippy::too_many_lines,
        reason = "the cycle's branch order and its per-pane handoff read as one place; the \
                  cycle-wide quota request is the one addition past the line limit"
    )]
    fn run(&self, carry: &mut Carry, err: &mut impl Write) -> crate::error::Result<()> {
        // FIRST, so this cycle's journal read already folds the record (I15).
        let clock = crate::watchdog::CycleClock {
            wall: Timestamp::now().epoch(),
            mono: Instant::now(),
        };
        if let Some(found) = carry
            .clock
            .replace(clock)
            .and_then(|prev| crate::watchdog::host_suspended(prev, clock, self.knobs.interval_secs))
        {
            Journal {
                meta_dir: self.meta_dir,
                session: self.session,
            }
            .record(
                crate::watchdog::HOST_SUSPENDED_ACTION,
                "",
                &crate::watchdog::suspended_summary(found),
                err,
            )?;
        }
        if !self.knobs.quota_aware {
            // Property: while unaware, nothing holds a quota observation. A
            // flip OFF drops whatever the aware phase held, every cycle, so
            // no later consumer — the throttle line included — can inject a
            // row from before the flip. See `QuotaCarry::clear_held`.
            carry.quota.clear_held();
        }
        // An enumeration that FAILED is not evidence that anything is gone.
        let Some(observed) = transport::observe_watch_panes(self.server, self.session) else {
            writeln!(
                err,
                "ae: watchdog: pane enumeration failed — skipping cycle"
            )?;
            return Ok(());
        };
        // ONE `ps` per cycle, with each process's state flags: the task
        // sensor (#283) needs them, every other reader takes the projection.
        let stat_table = procs::stat_snapshot();
        let table = procs::plain_table(stat_table.as_deref());
        let events = read_events(self.meta_dir);
        let suspended = crate::watchdog::Suspensions::read(&events);
        let now = Timestamp::now().epoch();
        let overview = if self.meta_agent && self.knobs.sweep.enabled() {
            self.overview(now, err)
        } else {
            None
        };
        // ONE cadence, one counter: `quota_every_secs` paces the periodic
        // quota pass, and zero disables it.
        let inputs = QuotaInputs {
            panes: &observed,
            held: &carry.panes,
            events: &events,
        };
        self.run_quota_cadence(&mut carry.quota, &inputs, now, err)?;

        let seats = held_seats(&observed, table.as_deref(), &|slot| self.agent_bin(slot));
        let session_id = crate::meta::session_id_in(self.meta_dir);
        let key = crate::events::SessionKey::new(self.session, &session_id);
        let outstanding = crate::session::Outstanding::read_in(&events, key, &seats);

        // ONE wait-chain reading per cycle, over the seats this cycle holds.
        let held: Vec<(&str, &str, &str)> = observed
            .iter()
            .filter_map(|pane| {
                let agent = pane.agent.as_deref()?;
                seats
                    .iter()
                    .any(|seat| seat == agent)
                    .then(|| (pane.slot.as_deref().unwrap_or_default(), agent, agent))
            })
            .collect();
        let wait_facts = crate::wait_on::Facts::read(&events, self.session, &held);
        let wait_links = crate::wait_on::Facts::links(&wait_facts);

        // ONE client read per cycle; a failed read changes no pane's memory.
        let clients = transport::observe_clients(self.server);
        let mut live: Vec<String> = Vec::new();
        let mut counts = Counts::default();
        let mut by_slot: Vec<(String, SeatVerdict)> = Vec::new();
        let mut by_agent: Vec<AgentObservation> = Vec::new();
        let mut by_pane: Vec<PaneMark> = Vec::new();
        // Cycle-wide: any seat leaving the limit this sweep requests ONE pass.
        let mut quota_refresh = false;
        // Every seat whose limit latch stands after its accounting.
        let mut auto_seats: Vec<AutoSeat> = Vec::new();
        // The return each seat on an automatic chain may make (#305), by slot.
        let mut homes: Vec<(String, crate::autoreseat::leg::Home)> = Vec::new();
        // The lead-pair seats the effort step judges.
        let mut effort_seats: Vec<crate::effort::leg::Sight> = Vec::new();
        // ONE stamp read per cycle; every pane shares the session's grace.
        let attempt = crate::store::open(self.meta_dir).launch_attempt();
        let start_frames = std::mem::take(&mut carry.start_frames);

        for pane in &observed {
            let Some(agent) = pane.agent.as_deref().filter(|name| !name.is_empty()) else {
                continue;
            };
            if NON_AGENT_PANES.contains(&agent) {
                continue;
            }
            live.push(agent.to_owned());
            let slot = pane.slot.clone().unwrap_or_default();
            let agent_bin = self.agent_bin(&slot);
            let tool =
                crate::tool::ToolKind::from_binary_name(agent_bin.as_deref().unwrap_or_default());

            let (capture, capture_ok) = capture_pane(self.server, &pane.pane_id);
            let capture_at = capture_ok.then(|| Timestamp::now().epoch());
            let hash = quiet_hash(&capture);
            // Model drift rides the SAME capture, under the seat's launch
            // guard, and shares this cycle's one reading of the profile pin
            // with the picker's own drift mark.
            let pins = self.seat_pin(&slot, tool);
            let pin = pins.model.clone();
            self.note_model(&capture, tool, &slot, &pins);
            let throttle = throttle_class(&capture, agent_bin.as_deref().unwrap_or_default());
            let throttle_quota = self.throttle_quota(&carry.quota, &slot, now, throttle.is_some());
            let own_room = self.own_room(&carry.quota, &slot, pin.as_deref(), now);
            let mut resume = self.start_read(
                carry.quota.last_observation.as_ref(),
                (&slot, &pane.pane_id),
                pin.as_deref(),
                now,
            );
            if resume.exhausted.is_some() && capture_ok {
                let seat = (
                    pane.pane_id.clone(),
                    launch_id_for(&self.launch_ids, &slot).map(str::to_owned),
                );
                let before = start_frames.iter().find(|(held, _)| *held == seat);
                resume.quiet = crate::autoreseat::quiet_pair(
                    before.map(|(_, frame)| frame.as_str()),
                    Some(&capture),
                    agent_bin.as_deref().unwrap_or_default(),
                );
                carry.start_frames.push((seat, capture.clone()));
            }
            // ONE process-tree reading: the dead verdict and the unknown-snapshot
            // counter are two questions about the same answer.
            let descendancy = descendancy_of(table.as_deref(), pane.pane_pid, agent_bin.as_deref());
            let identity = quiet_hash(&format!("{slot}\n{agent}"));
            let carried = entry_mut(&mut carry.panes, &pane.pane_id);
            restore_idle(carried, &pane.observed, identity);
            let launch = launch_id_for(&self.launch_ids, &slot);
            let (done_progress, done_proof) = crate::watchdog::done_progress_proof(
                &events,
                self.session,
                &slot,
                agent,
                launch,
                Timestamp::from_epoch(now),
                self.knobs.idle_nudge_secs,
                self.knobs.done_confirmations,
            );
            carried.client_activity = remember_input(
                carried.client_activity,
                identity,
                viewing_input(clients.as_deref(), &pane.pane_id),
            );
            let query = QuietQuery {
                events: &events,
                agent,
                slot: &slot,
                activity: carried.client_activity.map(|(_, at)| at),
                now,
            };
            let resolved = self.resolve_quiet(&query, done_progress);
            let quiet = resolved.quiet;
            let wait_state = match quiet {
                Some(QuietKind::WaitingAgent) => Some(WaitState::WaitingAgent),
                Some(QuietKind::Blocked) => Some(WaitState::Blocked),
                _ => None,
            };
            let crate::watchdog::WaitReading {
                progress: wait_progress,
                quiet: wait_proof,
                rung: wait_rung,
            } = wait_state.map_or(
                crate::watchdog::WaitReading {
                    progress: WaitProgress::None,
                    quiet: crate::watchdog::ProofQuiet::default(),
                    rung: 0,
                },
                |state| {
                    crate::watchdog::wait_reading(
                        &events,
                        self.session,
                        &slot,
                        agent,
                        launch,
                        Timestamp::from_epoch(now),
                        self.knobs.idle_nudge_secs,
                        self.knobs.done_confirmations,
                        state,
                    )
                },
            );
            // The tracked quiet state's proof, the watchdog's last paint and
            // the seat tool's task: what `account_with_activity` adds (#225,
            // #283).
            let activity = Activity {
                task_running: pane
                    .pane_pid
                    .zip(agent_bin.as_deref())
                    .and_then(|(pid, bin)| procs::task_running(stat_table.as_deref(), pid, bin)),
                proof: Some(if wait_state.is_some() {
                    wait_proof
                } else {
                    done_proof
                }),
                painting: crate::watchdog::last_watchdog_delivery(
                    &events,
                    self.session,
                    &slot,
                    agent,
                )
                .map(Timestamp::epoch),
            };
            let alerts =
                crate::session::alerts_since_in(&events, self.session, &slot, agent, self.started);
            let start = table
                .as_deref()
                .zip(pane.pane_pid)
                .zip(launch.zip(agent_bin.as_deref()))
                .and_then(|((table, pid), (launch, bin))| {
                    crate::autotrust::start_of(table, pid, (&slot, launch), bin)
                });
            // A failure keyed to a start, read against the one proven now.
            let failed_elsewhere = alerts.unreachable()
                && alerts
                    .failure
                    .and_then(|at| events[at].reference.as_deref())
                    .is_some_and(|booked| !booked.is_empty() && start.as_deref() != Some(booked))
                && start.is_some();
            let limit_reset = reads_reset(throttle, carried.limit_since.is_some())
                .then(|| self.own_reset(&carry.quota, &slot, pin.as_deref(), now))
                .flatten();
            let seen = Observation {
                now_epoch: now,
                hash,
                harness: self.harness_observation(&capture, tool, &events, &slot, agent),
                identity,
                is_dead: classify_dead(&pane.current_command, descendancy),
                throttle,
                limit_notice: crate::watchdog::limit_notice(
                    &capture,
                    agent_bin.as_deref().unwrap_or_default(),
                ),
                limit_reset,
                limit_woke_journal: limit_wake_in(&events, (self.session, &slot, agent), false),
                auto_continue: armed_reading(capture_ok, &capture, tool),
                capture_ok,
                human_prompt: crate::watchdog::human_prompt_class(
                    &capture,
                    agent_bin.as_deref().unwrap_or_default(),
                ),
                throttle_quota,
                quiet,
                ended_wait: resolved.ended,
                done_progress,
                wait_progress,
                wait_rung,
                wait_ends_elsewhere: crate::wait_on::ends_elsewhere(&wait_links, agent),
                target_moved: wait_facts
                    .iter()
                    .find(|fact| fact.name() == agent)
                    .and_then(crate::wait_on::Facts::on)
                    .and_then(|target| {
                        let waiter = (slot.as_str(), agent, agent);
                        crate::wait_on::moved(&events, self.session, waiter, target, &held)
                    }),
                descendancy,
                launch_attempt: attempt,
                outstanding_alert: alerts.winner().map(|(_, reason)| reason),
                unreachable: alerts.unreachable() && !failed_elsewhere,
                failed_elsewhere,
                prompt_named: alerts.named,
                start,
                last_actor_event_age_secs: last_actor_event_age(
                    &events,
                    self.session,
                    &slot,
                    agent,
                    now,
                ),
                declared_age_secs: declaration_age(&events, self.session, &slot, agent, now),
                suspended: suspended.clone(),
                // Decided HERE, once, and the type carries the answer: a pane
                // that is not the orchestrator main gets `None` and no sweep
                // branch can reach it.
                sweep: self.sweep_observation(&slot, agent, &events, overview.as_ref(), now),
                own_work: outstanding.of(Seat::new(self.session, &slot, agent)),
                auto_in_flight: if crate::autoreseat::in_flight(
                    &self.auto,
                    &events,
                    (self.session, &slot, agent),
                    now,
                ) {
                    Some(crate::autoreseat::Trigger::Limit)
                } else {
                    crate::autoreseat::headroom_in_flight(
                        &self.auto,
                        &events,
                        (self.session, &slot, agent),
                        now,
                    )
                    .then_some(crate::autoreseat::Trigger::Headroom)
                    .or_else(|| {
                        crate::autoreseat::return_in_flight(
                            &self.auto,
                            &events,
                            (self.session, &slot, agent),
                            now,
                        )
                        .then_some(crate::autoreseat::Trigger::Return)
                    })
                },
                // Forecast once, at the limit's first sight: the Notify that
                // carries it is booked only then.
                // A seat left to its reset's wake is never promised a move.
                auto_deadline: (throttle == Some(Throttle::LimitReached)
                    && carried.limit_since.is_none())
                .then(|| self.auto_deadline(&carry.quota, &events, (&slot, agent), now))
                .flatten()
                .map(|line| waiting_forecast(agent, limit_reset, now).unwrap_or(line)),
            };
            let acting = Acting {
                agent,
                slot: &slot,
                pane: &pane.pane_id,
                seen: &seen,
                events: &events,
                overview: seen.sweep.as_ref().and(overview.as_ref()),
            };
            let input = carried
                .client_activity
                .and_then(|(_, at)| i64::try_from(at).ok());
            let trust = self.trust_due(&seen, (&slot, &pane.pane_id), agent_bin.as_deref(), input);
            let booked =
                account_with_activity(carried, &seen, &self.knobs, trust.as_ref(), &activity);
            *carried = booked.next;
            self.apply_booked(&booked.effects, &acting, carried, &mut quota_refresh, err)?;
            if crate::effort::leg::eligible(&slot) && !self.meta_agent {
                effort_seats.push(crate::effort::leg::Sight {
                    slot: slot.clone(),
                    capture: capture.clone(),
                    // A resume's exhausted seat is moved, never restarted on
                    // that account.
                    limited: carried.limit_since.is_some() || resume.urgent(now),
                    point: crate::effort::leg::Point {
                        ready: capture_ok
                            && seen.harness.frame == HarnessState::Idle
                            && !seen.harness.human_draft
                            && seen.human_prompt.is_none(),
                        input,
                        reseating: seen.auto_in_flight.is_some(),
                        pane_pid: pane.pane_pid,
                    },
                });
            }
            {
                let frame = seen.harness.frame;
                auto_seats.push(AutoSeat {
                    slot: slot.clone(),
                    agent: agent.to_owned(),
                    profile: self.seat_profile(&slot),
                    pane: crate::autoreseat::Pane {
                        frame: crate::autoreseat::frame_of(
                            capture_ok,
                            frame,
                            seen.harness.human_draft,
                        ),
                        human_prompt: seen.human_prompt.is_some(),
                        client_input: carried
                            .client_activity
                            .and_then(|(_, at)| i64::try_from(at).ok()),
                    },
                    limited: carried.limit_since.is_some(),
                    derives: self.derives(
                        &events,
                        (&slot, agent),
                        carried.limit_since.is_some()
                            || resume.exhausted.is_some()
                            || matches!(
                                own_room,
                                crate::autoreseat::Room::Crossed(_) | crate::autoreseat::Room::Near
                            ),
                    ),
                    room: own_room,
                    start: resume,
                    reset: carried.limit_reset,
                    wake_landed: limit_wake_in(&events, (self.session, &slot, agent), true),
                    armed: carried.limit_armed,
                });
                let unlatched = carried.limit_since.is_none();
                if let Some(home) = self.home(&events, &slot, &carry.quota, unlatched, now) {
                    homes.push((slot.clone(), home));
                }
            }
            // A prompt over a standing failure is published as the failure.
            let shown = if seen.unreachable && booked.verdict == Verdict::HumanPrompt {
                Verdict::Unreachable
            } else {
                booked.verdict
            };
            counts.record(shown);
            // AFTER the accounting, so the identity reset and the dead verdict
            // this cycle just decided are the ones the hold answers to.
            let seat_identity = self.resolve_identity(
                carried,
                &ResolveIdentity {
                    capture: &capture,
                    tool,
                    slot: &slot,
                    pin: pin.as_deref(),
                    verdict: booked.verdict,
                },
            );
            // The next quota pass reads this successful capture, never its own
            // collection clock. Failed captures clear knowledge rather than age it anew.
            carried.history_capture = capture_at.map(|at| {
                (
                    at,
                    if booked.verdict == Verdict::Dead {
                        "unknown"
                    } else {
                        seen.harness.frame.as_str()
                    }
                    .to_owned(),
                    launch.map(str::to_owned),
                )
            });
            // The seat tool's job, as this cycle proved it: what every
            // published mark composes with (`SeatVerdict`).
            let task = activity.task_running == Some(true);
            by_slot.push((
                slot.clone(),
                SeatVerdict {
                    verdict: booked.verdict,
                    task,
                },
            ));
            let shown = SeatVerdict {
                verdict: shown,
                task,
            };
            by_agent.push(AgentObservation {
                slot,
                pane: pane.pane_id.clone(),
                verdict: shown,
                identity: seat_identity,
            });
            by_pane.push(PaneMark {
                pane: pane.pane_id.clone(),
                verdict: shown,
                observed: observed_option(seen.harness.frame, carried),
            });
        }
        let inputs = QuotaInputs {
            panes: &observed,
            held: &carry.panes,
            events: &events,
        };
        self.refresh_after_limit_release(quota_refresh, &mut carry.quota, &inputs, now, err)?;
        self.retry_briefs(&mut carry.brief_cursor, &by_slot, &events, now, err)?;
        self.tell_seats(&by_slot, &events, now);
        self.auto_note(&mut carry.auto_noted, err)?;
        self.trust_note(&mut carry.trust_noted, err)?;
        let effort = crate::effort::leg::Cycle {
            dir: self.meta_dir,
            session: self.session,
            events: &events,
            quota: carry.quota.last_observation.as_ref(),
            auto: &self.auto,
            now,
        };
        crate::effort::leg::step(&effort, &effort_seats, &mut carry.effort, err)?;
        self.auto_reseat(
            &mut carry.auto_named,
            (&auto_seats, &homes),
            &events,
            now,
            err,
        )?;
        self.close(
            carry,
            &counts,
            &by_slot,
            &by_agent,
            &by_pane,
            &live,
            now,
            table.as_deref(),
            err,
        )
        .inspect(|()| schedule_automatic_upgrade())
    }

    /// The cycle's last step: compose the strips in this session's look, then
    /// publish everything one pass produced.
    #[allow(
        clippy::too_many_arguments,
        reason = "one cycle handoff keeps every derived verdict slice borrowed"
    )]
    fn close(
        &self,
        carry: &mut Carry,
        counts: &Counts,
        by_slot: &[(String, SeatVerdict)],
        by_agent: &[AgentObservation],
        by_pane: &[PaneMark],
        live: &[String],
        now_epoch: i64,
        table: Option<&[procs::Proc]>,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        // The LOOK is re-read every cycle, so flipping `@ae_icons` on a live
        // session takes effect on the next one rather than at the next launch.
        // A read that did not answer keeps the last one and RECONCILES NOTHING:
        // rewriting a layout from a look ae did not actually read is how a
        // theme-off session gets repainted by the daemon that is meant to
        // respect it.
        let read = self.look();
        let Some(look) = read.look.or(carry.look) else {
            // No look has EVER answered on this session. Publishing now would
            // mean choosing colours ae was never told to use, and restamping
            // every window in them; the next cycle asks again.
            return Ok(());
        };
        carry.look = Some(look);
        if read.look.is_some()
            && let Some(session_id) = carry.pending.session_id()
        {
            self.reconcile_look(session_id, &look, read.stamp.as_deref(), &mut carry.motion);
        }
        // The roster still contributes slots whose panes are missing to the
        // session rollup. It is no longer drawn as a separate session strip:
        // live panes are named in their own window entries instead.
        let slots: Vec<Mark> = self
            .roster
            .iter()
            .map(|entry| slot_mark(entry, by_slot, &carry.missing))
            .collect();
        self.sweep_missing(live, &mut carry.missing, err)?;
        // The client is RECORDED, so every seat has it — including one whose
        // pane is gone. One config read per cycle serves all seats; `None`
        // falls every seat back to its binary name, never a guess.
        let root = crate::doors::state_root(crate::shape::current());
        let global = root
            .as_deref()
            .map(|root| crate::doors::config_file(crate::shape::current(), root));
        let cfg =
            crate::config::read_identity(global.as_deref(), self.local_config.as_deref()).ok();
        let clients: Vec<(&str, String)> = self
            .roster
            .iter()
            .map(|entry| (entry.slot.as_str(), seat_client_label(entry, cfg.as_ref())))
            .collect();
        let agents = agents_fact(
            &self.roster,
            by_agent,
            &clients,
            now_epoch,
            self.knobs.interval_secs,
        );
        self.publish(
            &Published {
                bar: bar_glyph(counts.dead, counts.stale, look.icons),
                active: counts.active,
                total: counts.total,
                by_pane,
                agents: agents.as_deref(),
                attention: session_mark(by_pane, &slots),
                look: &look,
            },
            &read.opened,
            carry,
            table,
        );
        Ok(())
    }

    /// Rewrite the LAYOUT when the look has moved under it.
    ///
    /// The values this daemon publishes follow the look every cycle, but the
    /// two status lines and the per-window styles are written once, at launch.
    /// So a `@ae_palette` or `@ae_look` changed on a live session would leave
    /// the bar half in the old look for as long as the session ran. The stamp
    /// is what the layout was written FOR; when it and the live look disagree,
    /// the layout is written again — or taken off, which unsets the session
    /// options and hands the user's own global status line back.
    ///
    /// `stamped` is the stamp the cycle's look read found ([`Cycle::look`]); a
    /// stamp that is unset or did not answer reads as empty, as it always has.
    fn reconcile_look(
        &self,
        session_id: &str,
        look: &Look,
        stamped: Option<&str>,
        motion: &mut MotionState,
    ) {
        if stamped.unwrap_or_default() == look.stamp() {
            return;
        }
        // The strip value may be unchanged while tmux's layout was removed or
        // repainted. Forget the local equality proof so this cycle republishes
        // it into the new surface.
        motion.published_fleet = None;
        // `&=`, never `&&`: every option is attempted even after one fails, and
        // the STAMP is only advanced when all of them landed. A stamp written
        // over a partial repaint would tell every later cycle the work was
        // done — which is how a session keeps ae's borders after `theme = off`.
        let mut ok = true;
        if look.drawn {
            for (option, value) in theme::layout_options(look) {
                ok &= transport::publish_option(
                    self.server,
                    OptionScope::Session,
                    session_id,
                    &option,
                    &value,
                );
            }
        } else {
            for option in theme::LAYOUT_OPTIONS {
                ok &=
                    transport::clear_option(self.server, OptionScope::Session, session_id, option);
            }
        }
        ok &= self.reconcile_windows(look);
        if !ok {
            return;
        }
        let _ = transport::publish_option(
            self.server,
            OptionScope::Session,
            session_id,
            theme::LOOK_STAMP_OPTION,
            &look.stamp(),
        );
    }

    /// The window half of [`Cycle::reconcile_look`]: restamp every window in
    /// the new look, or unset the options the old one wrote.
    fn reconcile_windows(&self, look: &Look) -> bool {
        // An enumeration that did not RUN is not an empty session: reporting
        // success here would advance the stamp over windows nobody looked at.
        let Some(panes) = transport::observe_window_panes(self.server, self.session) else {
            return false;
        };
        let mut ok = true;
        let mut done: Vec<&str> = Vec::new();
        for pane in &panes {
            if done.contains(&pane.window_id.as_str()) {
                continue;
            }
            done.push(&pane.window_id);
            if look.drawn {
                ok &= crate::session_launch::stamp_window(self.server, &pane.window_id, look);
            } else {
                for option in theme::window_option_names() {
                    ok &= transport::clear_option(
                        self.server,
                        OptionScope::Window,
                        &pane.window_id,
                        &option,
                    );
                }
            }
        }
        ok
    }

    /// The look this session is drawn in, as its own options declare it, with
    /// the session options the rest of the cycle reads beside it: ONE process.
    fn look(&self) -> CycleLookRead {
        let read = transport::observe_session_look(self.server, self.session, &CYCLE_LOOK_OPTIONS);
        let mut options = read.options.into_iter();
        let mut next = || options.next().flatten();
        CycleLookRead {
            look: read
                .look
                .map(|read| Look::read(&read.icons, &read.palette, &read.drawn, &read.motion)),
            stamp: next(),
            opened: [next(), next()],
        }
    }

    /// Publish this cycle's verdicts as tmux user options.
    ///
    /// Nothing is sent here: every value joins the cycle's ONE batch
    /// ([`flush_cycle`]), and every value is asserted again each cycle. The
    /// only immediate write is the clear of an expired menu marker, judged on
    /// `opened`, the markers the cycle's look read found.
    fn publish(
        &self,
        published: &Published<'_>,
        opened: &[Option<String>; 2],
        carry: &mut Carry,
        table: Option<&[procs::Proc]>,
    ) {
        let Some(session_id) = carry.pending.session_id().map(str::to_owned) else {
            return;
        };
        for (option, opened) in [theme::MENU_OPEN_OPTION, theme::SETTINGS_OPEN_OPTION]
            .into_iter()
            .zip(opened)
        {
            if let Some(opened) = opened
                && menu_open_expired(opened, Timestamp::now().epoch(), self.knobs.interval_secs)
            {
                let _ =
                    transport::clear_option(self.server, OptionScope::Session, &session_id, option);
            }
        }
        let look = published.look;
        let set = |name: &str, value: &str| {
            tmux::CycleOp::Set(tmux::OptionWrite::new(
                OptionScope::Session,
                &session_id,
                name,
                value,
            ))
        };
        let unset = |name: &str| tmux::CycleOp::Unset {
            scope: OptionScope::Session,
            target: session_id.clone(),
            name: name.to_owned(),
        };
        let mut ops = vec![set(tmux::WATCHDOG_STATUS_OPTION, &watch_segment(published))];
        // THE SESSION'S OWN ATTENTION, published as three facts: the glyph it
        // draws with, the rank another session's strip sorts on, and the style
        // its name segment is drawn in. Any session on this server can read
        // them, which is what makes the strip one tmux call rather than a walk
        // of every session's state.
        ops.push(set(
            theme::ATTENTION_GLYPH_OPTION,
            published.attention.glyph(look.icons),
        ));
        ops.push(set(
            theme::ATTENTION_RANK_OPTION,
            &published.attention.rank().to_string(),
        ));
        ops.push(set(
            theme::ATTENTION_STYLE_OPTION,
            &theme::attention_style(&look.palette, published.attention),
        ));
        ops.push(match published.agents {
            Some(agents) => set(theme::AGENTS_OPTION, agents),
            None => unset(theme::AGENTS_OPTION),
        });
        // The GOAL, ahead of the path on the right: what this session is for
        // outranks where its files are, and the path is the fact the reader's
        // own shell prompt already carries.
        ops.push(
            match self
                .goal
                .as_deref()
                .map(str::trim)
                .filter(|g| !g.is_empty())
            {
                Some(goal) => set(
                    theme::GOAL_OPTION,
                    &format!(" {}", theme::bar_text(goal, theme::GOAL_WIDTH)),
                ),
                None => unset(theme::GOAL_OPTION),
            },
        );
        // The core THIS daemon runs on. An upgrade restarts the daemon on the
        // new core, so the value moves with the install and never with a launch.
        ops.push(set(theme::VERSION_OPTION, &crate::shape::version_line()));
        carry.pending.ops.extend(ops);
        // The windows first: they decide this cycle's verdicts, and the fleet
        // stages a copy of the motion state that must already hold them.
        self.publish_windows(published, &mut carry.motion, &mut carry.pending.ops);
        self.publish_fleet(look, carry, table, &session_id, published.attention);
    }

    /// The fleet strip: every ae session on THIS server, as each one's own
    /// watchdog described itself — plus the strip of every same-server ae
    /// session that has no live watchdog to describe it.
    ///
    /// ONE listing answers both, and the adoption scan hangs off it: this is
    /// the read that happens every cycle whatever the ticker is doing, so it is
    /// the only place a daemon with no targets — and therefore no tick of its
    /// own — can ever discover its first one.
    fn publish_fleet(
        &self,
        look: &Look,
        carry: &mut Carry,
        table: Option<&[procs::Proc]>,
        session_id: &str,
        attention: Mark,
    ) {
        let Some(mut sessions) = transport::observe_fleet_listing(self.server) else {
            return;
        };
        // This cycle's own rank, whatever the listing caught of it.
        overlay_current_rank(&mut sessions, session_id, attention);
        // A listing that did not answer is not evidence that a peer is gone, so
        // the enumeration is simply not run and the adoption already held
        // stands until the next cycle.
        if let Some(next) =
            enumerate_adoption(self.server, self.session, &sessions, table, &carry.adoption)
        {
            carry.adoption = next;
        }
        let orchestrator_id = orchestrator_id_for(&sessions, self.session);
        let mut next = carry.motion.clone();
        next.replace_fleet(&sessions, &carry.adoption.known, self.session);
        // ONCE per verdict cycle, from the content this cycle read: a config
        // edit reaches a running session here, and the ticker inherits it.
        next.set_fleet_order(&self.fleet_order);
        let mut writes = Vec::new();
        next.push_fleet_write(&mut writes, look, None);
        if let Some(target) = next.fleet_target.clone() {
            let orchestrator = next
                .fleet
                .iter()
                .find(|row| row.name == crate::orchestrator::ORCHESTRATOR_SESSION)
                .cloned();
            let strip_changed = next.push_orchestrator_strip_write(
                &mut writes,
                look,
                &target,
                orchestrator.as_ref(),
                None,
            );
            if strip_changed
                && matches!(
                    &next.published_orchestrator_strip,
                    PublishedOrchestratorStrip::Unset
                )
                && !transport::clear_option(
                    self.server,
                    OptionScope::Session,
                    &target,
                    theme::ORCHESTRATOR_STRIP_OPTION,
                )
            {
                // The fleet strip is independent and still gets published;
                // restore the cache so the failed unset is retried next cycle.
                next.published_orchestrator_strip
                    .clone_from(&carry.motion.published_orchestrator_strip);
            }
            let id_changed = next.push_orchestrator_id_write(&mut writes, &target, orchestrator_id);
            if id_changed
                && orchestrator_id.is_none()
                && !transport::clear_option(
                    self.server,
                    OptionScope::Session,
                    &target,
                    theme::ORCHESTRATOR_ID_OPTION,
                )
            {
                // The fleet strip is independent and still gets published;
                // restore the cache so the failed unset is retried next cycle.
                next.published_orchestrator_id
                    .clone_from(&carry.motion.published_orchestrator_id);
            }
        }
        // The adopted strips ride the SAME batch: one tmux process carries
        // this session's own line and every line it is filling for a peer.
        let mut adoption = carry.adoption.clone();
        writes.extend(adoption_writes(&mut adoption, &sessions, &next.fleet_order));
        // STAGED, not committed: the batch runs at the cycle's end, and the
        // memories advance only once its writes are proven landed.
        let start = carry.pending.ops.len();
        carry
            .pending
            .ops
            .extend(writes.into_iter().map(tmux::CycleOp::Set));
        carry.pending.fleet = Some(StagedFleet {
            motion: next,
            adoption,
            ops: start..carry.pending.ops.len(),
        });
    }

    /// Per-window marks and per-pane state, grouped from the SAME per-pane
    /// verdicts in pane order — and the theme, restamped on any window that
    /// appeared since the launch dressed the session.
    fn publish_windows(
        &self,
        published: &Published<'_>,
        motion: &mut MotionState,
        ops: &mut Vec<tmux::CycleOp>,
    ) {
        let Some(panes) = transport::observe_window_panes(self.server, self.session) else {
            return;
        };
        let look = published.look;
        let mut windows: Vec<(String, Vec<(String, Mark)>)> = Vec::new();
        let mut verdicts = Vec::new();
        for pane in &panes {
            let agents = entry_mut(&mut windows, &pane.window_id);
            let Some(agent) = pane.agent.as_deref().filter(|name| !name.is_empty()) else {
                continue;
            };
            // The DRAWN name, BEFORE the agent filter and every cycle: a
            // session upgraded in place carries an identity and no label, and
            // the border format reads the label — so an unbackfilled pane, the
            // monitor's own included, would draw a blank title from the moment
            // the look reached it.
            ops.push(tmux::CycleOp::Set(tmux::OptionWrite::new(
                OptionScope::Pane,
                &pane.pane_id,
                theme::AGENT_LABEL_OPTION,
                &theme::agent_label(agent),
            )));
            if NON_AGENT_PANES.contains(&agent) {
                continue;
            }
            let found = published
                .by_pane
                .iter()
                .find(|entry| entry.pane == pane.pane_id);
            let mark = found.map_or(Mark::Idle, |entry| entry.verdict.mark());
            agents.push((theme::agent_label(agent), mark));
            ops.extend(
                Self::pane_state_writes(&pane.pane_id, found, look)
                    .into_iter()
                    .map(tmux::CycleOp::Set),
            );
            if let Some(entry) = found {
                verdicts.push(MotionVerdict {
                    pane: pane.pane_id.clone(),
                    window: pane.window_id.clone(),
                    verdict: entry.verdict,
                });
            }
        }
        for (window_id, agents) in &windows {
            let line = window_agents_line(agents, look, None);
            ops.push(if line.is_empty() {
                tmux::CycleOp::Unset {
                    scope: OptionScope::Window,
                    target: window_id.clone(),
                    name: theme::WINDOW_AGENTS_OPTION.to_owned(),
                }
            } else {
                tmux::CycleOp::Set(tmux::OptionWrite::new(
                    OptionScope::Window,
                    window_id,
                    theme::WINDOW_AGENTS_OPTION,
                    &line,
                ))
            });
        }
        // A window created after the launch — by a spawn on an older core, or
        // by the human — carries no stamp, so it is dressed here rather than
        // left on the user's global window table.
        let mut dressed: Vec<&str> = Vec::new();
        for pane in &panes {
            if pane.theme == theme::window_stamp(look) || dressed.contains(&pane.window_id.as_str())
            {
                continue;
            }
            dressed.push(&pane.window_id);
            crate::session_launch::stamp_window(self.server, &pane.window_id, look);
        }
        motion.replace_verdicts(verdicts);
    }

    /// One pane's border state: its mark, and the word behind it. Nothing
    /// when the pane has no verdict this cycle.
    fn pane_state_writes(
        pane: &str,
        found: Option<&PaneMark>,
        look: &Look,
    ) -> Vec<tmux::OptionWrite> {
        let Some(entry) = found else {
            return Vec::new();
        };
        let mark = entry.verdict.mark();
        let state = theme::pane_state(
            &look.palette,
            mark,
            mark.glyph(look.icons),
            entry.verdict.reason(),
        );
        // The ACCENT alone, for the active border: a style option is
        // format-expanded, so the border colour follows the pane it belongs to
        // without a style written per pane.
        vec![
            tmux::OptionWrite::new(OptionScope::Pane, pane, theme::PANE_STATE_OPTION, &state),
            tmux::OptionWrite::new(
                OptionScope::Pane,
                pane,
                theme::PANE_ACCENT_OPTION,
                look.palette.accent(mark),
            ),
            tmux::OptionWrite::new(
                OptionScope::Pane,
                pane,
                theme::OBSERVED_OPTION,
                &entry.observed,
            ),
        ]
    }

    /// The recorded binary for a slot, or `None` when the roster has none —
    /// which the dead check must read as UNKNOWN, never as absent.
    /// The profile the roster records for `slot`, or `""`.
    fn seat_profile(&self, slot: &str) -> String {
        self.roster
            .iter()
            .find(|entry| entry.slot == slot)
            .and_then(|entry| entry.profile.clone())
            .unwrap_or_default()
    }

    fn agent_bin(&self, slot: &str) -> Option<String> {
        self.roster
            .iter()
            .find(|entry| entry.slot == slot)
            .and_then(|entry| entry.binary.clone())
            .filter(|binary| !binary.is_empty())
    }

    /// The RESOLVED quiet suppression for one pane: `done` while its episode
    /// is live, a wait until the human's own input in the pane ends it. A wait
    /// that ended THIS cycle is returned with the suppression's absence, so
    /// the nudge that follows can name it.
    fn resolve_quiet(
        &self,
        query: &QuietQuery<'_>,
        done_progress: DoneProgress,
    ) -> QuietResolution {
        let Some(relevant) =
            latest_relevant_event(query.events, self.session, query.slot, query.agent)
        else {
            return QuietResolution::NONE;
        };
        let Some(kind) = quiet_reason(&relevant) else {
            return QuietResolution::NONE;
        };
        if kind == QuietKind::Done {
            return QuietResolution {
                quiet: (!matches!(
                    done_progress,
                    DoneProgress::None | DoneProgress::Lapsed { .. }
                ))
                .then_some(kind),
                ended: None,
            };
        }
        let delivered = crate::watchdog::last_watchdog_delivery(
            query.events,
            self.session,
            query.slot,
            query.agent,
        );
        if crate::watchdog::human_input_ends_wait(query.activity, relevant.event.ts, delivered) {
            let input_age_secs = query
                .activity
                .and_then(|epoch| i64::try_from(epoch).ok())
                .map_or(0, |epoch| age_secs(query.now, epoch));
            return QuietResolution {
                quiet: None,
                ended: Some(WaitEnded {
                    kind,
                    input_age_secs,
                }),
            };
        }
        QuietResolution {
            quiet: Some(kind),
            ended: None,
        }
    }

    /// Perform one effect.
    #[allow(
        clippy::too_many_lines,
        reason = "one arm per effect, each the effect's whole delivery and record"
    )]
    fn apply(
        &self,
        effect: &Effect,
        on: &Acting<'_>,
        state: &mut PaneState,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let agent = on.agent;
        match effect {
            // A panel's naming and an unreachable failure are keyed to their
            // tool start: what the quiet gate ages from, what a new start ends.
            Effect::Emit { action, summary }
                if crate::autotrust::panel_named(summary)
                    || (*action == "alert" && summary.starts_with(crate::events::UNREACHABLE)) =>
            {
                Journal {
                    meta_dir: self.meta_dir,
                    session: self.session,
                }
                .record_referring(
                    action,
                    agent,
                    on.seen.start.as_deref().unwrap_or_default(),
                    summary,
                    err,
                )
            }
            Effect::Emit { action, summary } => self.emit(action, agent, summary, err),
            Effect::Notify(text) => {
                self.notify(agent, text);
                Ok(())
            }
            Effect::Note(text) => {
                writeln!(err, "ae: watchdog: {agent}: {text}")?;
                Ok(())
            }
            Effect::AnswerTrust { name } => self.answer_trust(*name, on, state, err),
            Effect::SweepNudge => self.sweep_nudge(on, state, err),
            // Cycle-level by design: `run` collects it and performs the ONE
            // pass after the sweep, outside every per-pane path.
            Effect::QuotaRefresh => Ok(()),
            Effect::ReconcileWedge => {
                // The DURABLE half of the wedge clear.
                if crate::session::alert_reason_in(on.events, self.session, on.slot, agent)
                    .is_some()
                {
                    let alert = SweepAlert::ClearWedge;
                    self.emit(alert.action(), agent, &alert.summary(), err)?;
                }
                Ok(())
            }
            Effect::Nudge => {
                let idle_age = (on.seen.harness.frame == HarnessState::Idle)
                    .then(|| {
                        state
                            .idle_since_epoch
                            .map(|at| age_secs(on.seen.now_epoch, at))
                    })
                    .flatten();
                let display = stale_display(idle_age.unwrap_or(on.seen.last_actor_event_age_secs));
                let waiting = on.seen.own_work.reason();
                let (text, summary) = nudge_words(
                    self.goal.as_deref(),
                    self.meta_dir,
                    idle_age,
                    &display,
                    waiting.as_deref(),
                    declared_clause(on.events, self.session, on.slot, agent).as_deref(),
                    on.seen.ended_wait,
                );
                let code = self.deliver(agent, &text, "nudge", &summary, None).code;
                self.reachable_again(on, "nudge", code, err)?;
                let delivered = code == Some(0);
                for effect in record_nudge(state, delivered, &self.knobs, &display) {
                    self.apply(&effect, on, state, err)?;
                }
                Ok(())
            }
            Effect::LimitWake { reset } => {
                let text = limit_wake_text(self.goal.as_deref(), self.meta_dir, *reset);
                let summary = format!(
                    "usage limit reset at {} — continuation turn",
                    Timestamp::from_epoch(*reset)
                );
                let reference = reset.to_string();
                let code = self
                    .deliver(
                        agent,
                        &text,
                        crate::watchdog::LIMIT_WAKE_ACTION,
                        &summary,
                        Some(&reference),
                    )
                    .code;
                self.reachable_again(on, crate::watchdog::LIMIT_WAKE_ACTION, code, err)?;
                // The helper's success covers a delivered AND an unconfirmed
                // paste: either may have reached the seat, so the reset is
                // spent. A refusal pasted nothing and counts one attempt. The
                // nudge budget is not this path's: neither touches it.
                if code == Some(0) {
                    state.limit_woken = Some(*reset);
                } else {
                    state.limit_wake_attempts = state.limit_wake_attempts.saturating_add(1);
                }
                Ok(())
            }
            Effect::DoneChallenge {
                confirmations,
                required,
                done_age_secs,
            } => {
                let text = done_challenge_text(
                    self.goal.as_deref(),
                    self.meta_dir,
                    *done_age_secs,
                    *confirmations,
                    *required,
                );
                let summary = crate::watchdog::challenge_summary(
                    crate::watchdog::Challenge::Done,
                    confirmations.saturating_add(1),
                    *required,
                );
                let launch = launch_id_for(&self.launch_ids, on.slot);
                let code = self
                    .deliver(agent, &text, "done-challenge", &summary, launch)
                    .code;
                self.reachable_again(on, "done-challenge", code, err)
            }
            Effect::DoneHeld {
                confirmations,
                required,
            } => {
                // Journal only: the record IS the credit the fold admits, so
                // it carries the launch witness exactly as a challenge does.
                let summary = crate::watchdog::challenge_summary(
                    crate::watchdog::Challenge::Done,
                    confirmations.saturating_add(1),
                    *required,
                );
                Journal {
                    meta_dir: self.meta_dir,
                    session: self.session,
                }
                .record_referring(
                    crate::watchdog::DONE_HELD_ACTION,
                    agent,
                    launch_id_for(&self.launch_ids, on.slot).unwrap_or_default(),
                    &summary,
                    err,
                )
            }
            Effect::TargetMoved { moved, age_secs } => {
                let target =
                    crate::session::declaration_of(on.events, self.session, on.slot, agent)
                        .and_then(|(declared, _)| declared.wait_on.as_deref())
                        .unwrap_or("its target");
                let (text, summary) = target_moved_text(
                    self.goal.as_deref(),
                    self.meta_dir,
                    *age_secs,
                    target,
                    moved,
                );
                let action = crate::watchdog::TARGET_MOVED_ACTION;
                let number = moved.ordinal.to_string();
                let code = self
                    .deliver(agent, &text, action, &summary, Some(&number))
                    .code;
                self.reachable_again(on, action, code, err)
            }
            Effect::WaitChallenge {
                confirmations,
                required,
                wait_age_secs,
                state,
                escalated,
            } => {
                let launch = launch_id_for(&self.launch_ids, on.slot);
                let owed = crate::watchdog::wait_goal_owed(
                    on.events,
                    self.session,
                    on.slot,
                    agent,
                    launch,
                );
                let target =
                    crate::session::declaration_of(on.events, self.session, on.slot, agent)
                        .and_then(|(declared, _)| declared.wait_on.as_deref());
                let text = wait_challenge_text(
                    self.goal.as_deref().filter(|_| owed),
                    self.meta_dir,
                    &WaitCheck {
                        age: *wait_age_secs,
                        confirmations: *confirmations,
                        required: *required,
                        state: *state,
                        escalated: *escalated,
                        target,
                    },
                );
                let num = confirmations.saturating_add(1);
                let summary = crate::watchdog::challenge_summary(
                    crate::watchdog::Challenge::Wait(*state),
                    num,
                    *required,
                );
                let code = self
                    .deliver(agent, &text, "wait-challenge", &summary, launch)
                    .code;
                self.reachable_again(on, "wait-challenge", code, err)
            }
        }
    }

    /// After a delivery to a seat whose unreachable failure stands: ONE
    /// `alert-cleared` once the helper's own record of `action` says it landed.
    fn reachable_again(
        &self,
        on: &Acting<'_>,
        action: &str,
        code: Option<i32>,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        if !on.seen.unreachable || code != Some(0) {
            return Ok(());
        }
        let events = read_events(self.meta_dir);
        let landed = events.iter().rev().find(|event| {
            event.action == action
                && crate::watchdog::event_is_addressed_to(event, self.session, on.slot, on.agent)
        });
        if landed.is_some_and(|event| !tracked::summary_is_unconfirmed(event.summary.as_deref())) {
            let summary = format!("{} — {action} landed", crate::events::REACHABLE_AGAIN);
            self.emit("alert-cleared", on.agent, &summary, err)?;
        }
        Ok(())
    }

    /// Deliver one sweep prompt and book what happened.
    fn sweep_nudge(
        &self,
        on: &Acting<'_>,
        state: &mut PaneState,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let Some(_observed) = on.seen.sweep.as_ref() else {
            // Unreachable by construction: only the sweep branch emits this
            // effect, and it runs only where the observation exists.
            writeln!(
                err,
                "ae: watchdog: sweep prompt for {} had no sweep reading — skipped",
                on.agent
            )?;
            return Ok(());
        };
        let Some(overview) = on.overview else {
            // A failed fleet read suppresses the change gate, so this is also
            // unreachable by construction. Keep the refusal loud if those two
            // paths ever drift.
            writeln!(
                err,
                "ae: watchdog: sweep nudge for {} had no fleet overview — skipped",
                on.agent
            )?;
            return Ok(());
        };
        // Delivery is CHECKED.
        let body = overview.body();
        let delivered = self
            .deliver(
                on.agent,
                &body,
                crate::watchdog::SWEEP_NUDGE_ACTION,
                "fleet overview changed",
                None,
            )
            .code
            == Some(0);
        // This is intentionally AFTER the checked delivery. A deferred submit
        // cannot be acknowledged by a `done` event that preceded the paste,
        // and the minimum spacing begins when the paste actually landed.
        let settled_epoch = Timestamp::now().epoch();
        let settled_now = system_time_from_epoch(settled_epoch).unwrap_or(UNIX_EPOCH);
        let booked = record_sweep(&mut state.sweep, delivered, settled_now, &self.knobs.sweep);
        if delivered {
            let outstanding_since = state
                .sweep
                .outstanding_since
                .map_or_else(|| epoch_second(settled_now), epoch_second);
            if let Err(why) = crate::monitor::record_overview_delivery(
                self.meta_dir,
                &overview.hash,
                outstanding_since,
                settled_epoch,
            ) {
                writeln!(
                    err,
                    "ae: watchdog: delivered fleet overview but could not persist its hash: {why}"
                )?;
            }
        }
        for effect in sweep_effects(booked) {
            self.apply(&effect, on, state, err)?;
        }
        Ok(())
    }

    /// THE ONE PLACE THIS DAEMON EXECUTES ANYTHING, and the reason both nudges
    /// route through it rather than each spawning for itself: a second delivery
    /// site is a second thing to audit, and a unit guard in this file holds the
    /// count at one.
    fn deliver(
        &self,
        agent: &str,
        text: &str,
        action: &str,
        summary: &str,
        reference: Option<&str>,
    ) -> transport::Delivery {
        let mut vars = vec![
            ("AE_SENDER_OVERRIDE", ACTOR),
            ("_AE_EVENT_ACTION", action),
            ("_AE_EVENT_SUMMARY", summary),
        ];
        vars.extend(reference.map(|value| ("_AE_EVENT_REF", value)));
        transport::deliver(self.helper.path(), agent, text, false, &vars)
    }

    /// The auto reseat step: journal each hold and overdue failure the pure
    /// core decides, then hand its ONE due seat to the trigger through the one
    /// exec site.
    fn auto_reseat(
        &self,
        named: &mut Vec<(String, Timestamp, crate::autoreseat::HoldReason)>,
        seats: (&[AutoSeat], &[(String, crate::autoreseat::leg::Home)]),
        events: &[Event],
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let context = (self.session, self.meta_agent, self.lead_pair);
        for act in auto_acts_with(&self.auto, seats, events, context, now, named) {
            let AutoAct::Trigger { agent } = &act else {
                self.auto_record(&act, now, err)?;
                continue;
            };
            let attempt = crate::autoreseat::ATTEMPT_ACTION;
            let delivery = self.deliver(agent, AUTO_PLACEHOLDER, attempt, "", None);
            if delivery.code != Some(0) {
                writeln!(
                    err,
                    "ae: watchdog: auto reseat of {agent} not started this cycle"
                )?;
            }
        }
        Ok(())
    }

    /// Journal one hold, overdue failure, headroom opening, re-arm or give-up
    /// under the seat's lock, and only while the journal as it reads NOW still
    /// owes it: a trigger may have opened an attempt, or the leg ended one,
    /// since this cycle's read.
    #[allow(
        clippy::too_many_lines,
        reason = "one ordered writer per act, each re-reading the journal under the lock"
    )]
    fn auto_record(
        &self,
        act: &AutoAct,
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        use crate::autoreseat::Trigger;
        let (slot, agent) = match act {
            AutoAct::Held { slot, agent, .. }
            | AutoAct::Overdue { slot, agent, .. }
            | AutoAct::Open { slot, agent, .. }
            | AutoAct::Rearm { slot, agent, .. }
            | AutoAct::GaveUp { slot, agent, .. }
            | AutoAct::OpenReturn { slot, agent, .. }
            | AutoAct::Flapped { slot, agent, .. }
            | AutoAct::Unmapped { slot, agent, .. } => (slot, agent),
            AutoAct::Trigger { .. } => return Ok(()),
        };
        let lock = crate::autoreseat::lock_path(self.meta_dir, slot);
        let Ok(_held) = store::lock(&lock, std::time::Duration::ZERO) else {
            return Ok(());
        };
        let events = read_events(self.meta_dir);
        let journal = Journal {
            meta_dir: self.meta_dir,
            session: self.session,
        };
        let headroom = crate::autoreseat::headroom_episode(&events, self.session, slot, agent);
        match act {
            AutoAct::Open { cause, .. } => {
                if headroom.is_none() {
                    journal.record(crate::autoreseat::HEADROOM_ACTION, agent, cause, err)?;
                }
                return Ok(());
            }
            AutoAct::Rearm { key, summary, .. } => {
                if headroom.is_some_and(|open| open.key == *key && open.open.is_none()) {
                    let action = crate::autoreseat::REARMED_ACTION;
                    journal.record_referring(action, agent, &key.to_string(), summary, err)?;
                }
                return Ok(());
            }
            AutoAct::OpenReturn { cause, .. } => {
                let back = crate::autoreseat::return_episode(&events, self.session, slot, agent);
                let chain = crate::autoreseat::chain(&events, self.session, slot, agent);
                if back.is_none() && chain.is_some() {
                    journal.record(crate::autoreseat::RETURN_ACTION, agent, cause, err)?;
                }
                return Ok(());
            }
            AutoAct::Flapped { key, origin, .. } => {
                let said = crate::autoreseat::flapped_summary(self.session, agent, origin);
                let reference = key.to_string();
                let held = events.iter().any(|event| {
                    event.action == crate::autoreseat::HELD_ACTION
                        && event.actor == crate::watchdog::WATCHDOG_ACTOR
                        && event.reference.as_deref() == Some(reference.as_str())
                        && event.summary.as_deref() == Some(said.as_str())
                        && crate::watchdog::event_is_addressed_to(event, self.session, slot, agent)
                });
                if !held {
                    let action = crate::autoreseat::HELD_ACTION;
                    journal.record_referring(action, agent, &reference, &said, err)?;
                    crate::autoreseat::leg::tell_held(
                        self.meta_dir,
                        (self.session, slot, agent),
                        *key,
                        Timestamp::from_epoch(now),
                        &said,
                    );
                }
                return Ok(());
            }
            AutoAct::GaveUp { key, .. } => {
                let said = crate::autoreseat::GAVE_UP_SUMMARY;
                let found = crate::autoreseat::episode(&events, self.session, slot, agent)
                    .filter(|open| open.key == *key)
                    .or_else(|| {
                        crate::autoreseat::return_episode(&events, self.session, slot, agent)
                    });
                if found.is_some_and(|open| {
                    open.key == *key && open.gave_up() && !open.holds_with(said)
                }) {
                    let action = crate::autoreseat::HELD_ACTION;
                    journal.record_referring(action, agent, &key.to_string(), said, err)?;
                    crate::autoreseat::leg::tell_gave_up(
                        self.meta_dir,
                        (self.session, slot, agent),
                        *key,
                        Timestamp::from_epoch(now),
                    );
                }
                return Ok(());
            }
            AutoAct::Unmapped { key, profile, .. } => {
                let summary = crate::autoreseat::leg::ineligible_record(
                    crate::autoreseat::Ineligible::Unmapped,
                );
                let found = crate::autoreseat::episode(&events, self.session, slot, agent);
                if crate::autoreseat::owed(found.as_ref(), *key, false, now)
                    && !found.as_ref().is_some_and(|open| open.holds_with(&summary))
                {
                    journal.record_referring(
                        crate::autoreseat::HELD_ACTION,
                        agent,
                        &key.to_string(),
                        &summary,
                        err,
                    )?;
                    crate::autoreseat::leg::tell_unmapped(
                        self.meta_dir,
                        (self.session, slot, agent),
                        profile,
                        *key,
                        Timestamp::from_epoch(now),
                    );
                }
                return Ok(());
            }
            AutoAct::Held { .. } | AutoAct::Overdue { .. } | AutoAct::Trigger { .. } => {}
        }
        let (key, overdue, action, summary, trigger) = match act {
            AutoAct::Held {
                key,
                reason,
                trigger,
                ..
            } => (
                *key,
                false,
                crate::autoreseat::HELD_ACTION,
                reason.record(),
                *trigger,
            ),
            AutoAct::Overdue { key, trigger, .. } => (
                *key,
                true,
                crate::autoreseat::FAILED_ACTION,
                crate::autoreseat::CauseClass::Other.lead(OVERDUE_SUMMARY),
                *trigger,
            ),
            _ => return Ok(()),
        };
        let found = match trigger {
            Trigger::Limit => crate::autoreseat::episode(&events, self.session, slot, agent),
            Trigger::Headroom => headroom,
            Trigger::Return => {
                crate::autoreseat::return_episode(&events, self.session, slot, agent)
            }
        };
        let cause = found
            .as_ref()
            .map(|open| open.cause().to_owned())
            .unwrap_or_default();
        let summary = crate::autoreseat::leg::tagged(&summary, &cause);
        let said = trigger == Trigger::Headroom
            && !overdue
            && found.as_ref().is_some_and(|open| open.holds_with(&summary));
        if !said && crate::autoreseat::owed(found.as_ref(), key, overdue, now) {
            journal.record_referring(action, agent, &key.to_string(), &summary, err)?;
        }
        Ok(())
    }

    /// Journal the note an unusable `auto_reseat_at`, `auto_reseat_lead_at` or
    /// `auto_reseat_lead_floor` left, target-less, once per daemon start and
    /// note: the operator sees the threshold ae fell back to, or the floor that
    /// holds every lead seat, without a move having to say it.
    fn auto_note(&self, noted: &mut Vec<String>, err: &mut impl Write) -> crate::error::Result<()> {
        let journal = Journal {
            meta_dir: self.meta_dir,
            session: self.session,
        };
        for note in [
            &self.auto.headroom_note,
            &self.auto.lead.note,
            &self.auto.floor_note,
        ]
        .into_iter()
        .flatten()
        {
            if noted.contains(note) {
                continue;
            }
            noted.push(note.clone());
            journal.record(crate::autoreseat::NOTICE_ACTION, "", note, err)?;
        }
        Ok(())
    }

    /// Name an unusable `auto_trust` value once per daemon start: it answers
    /// nothing, exactly like `off`.
    fn trust_note(
        &self,
        noted: &mut Vec<String>,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let crate::config::AutoTrust::Unusable(note) = &self.auto_trust else {
            return Ok(());
        };
        if noted.contains(note) {
            return Ok(());
        }
        noted.push(note.clone());
        writeln!(err, "ae: watchdog: auto_trust {note}; it answers nothing")?;
        Ok(())
    }

    /// Whether this cycle's prompt is a trust block that is this seat's to
    /// answer — asked only with `auto_trust` on and a measured block drawn.
    fn trust_due(
        &self,
        seen: &Observation,
        (slot, pane): (&str, &str),
        tool: Option<&str>,
        input: Option<i64>,
    ) -> Option<crate::autotrust::Due> {
        let prompt = seen
            .human_prompt
            .as_ref()
            .filter(|_| self.auto_trust.answers())?;
        let seat = self.trust_seat(slot, pane, tool?);
        if prompt.dismiss {
            return Some(crate::autotrust::dismiss_due(&seat, input));
        }
        Some(crate::autotrust::due(&seat, prompt.trust.as_ref()?))
    }

    fn trust_seat<'s>(
        &'s self,
        slot: &'s str,
        pane: &'s str,
        tool: &'s str,
    ) -> crate::autotrust::Seat<'s> {
        crate::autotrust::Seat {
            dir: self.meta_dir,
            server: self.server,
            session: self.session,
            slot,
            pane,
            tool,
        }
    }

    /// Answer the trust block on `on`'s pane; an answer that does not land
    /// names the prompt exactly as today when it is still to be named.
    fn answer_trust(
        &self,
        name: bool,
        on: &Acting<'_>,
        state: &mut PaneState,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let tool = self.agent_bin(on.slot).unwrap_or_default();
        let seat = self.trust_seat(on.slot, on.pane, &tool);
        // A panel is named at the bound already (`name` is false): the clear
        // that follows a landed Escape is 5c's, so the streak is left to it.
        let (ask, word) = if on.seen.human_prompt.as_ref().is_some_and(|it| it.dismiss) {
            let input = state
                .client_activity
                .and_then(|(_, at)| i64::try_from(at).ok());
            (crate::deliver::Ask::Panel(input), "auto-dismiss")
        } else {
            (crate::deliver::Ask::Trust, "auto-trust")
        };
        match crate::deliver::answer_trust(&seat, ask, err) {
            crate::deliver::TrustAnswer::Trusted(summary) => {
                self.notify(on.agent, &format!("{word}: {summary}"));
                // Never named, so nothing is to clear: a modal still drawn is
                // counted again and, its start now spent, named at the bound.
                if name {
                    state.human_prompt_streak = 0;
                }
                return Ok(());
            }
            crate::deliver::TrustAnswer::Declined(why) if name => writeln!(
                err,
                "ae: watchdog: {}: auto_trust did not answer the trust prompt: {why}",
                on.agent
            )?,
            _ => {}
        }
        if let Some(prompt) = on.seen.human_prompt.as_ref().filter(|_| name) {
            for effect in naming(prompt) {
                self.apply(&effect, on, state, err)?;
            }
        }
        Ok(())
    }

    /// The seat at `slot`'s own binding reset, from the held quota
    /// observation; `None` when no roster row or reading names one.
    fn own_reset(
        &self,
        quota: &QuotaCarry,
        slot: &str,
        pin: Option<&str>,
        now: i64,
    ) -> Option<crate::autoreseat::LimitReset> {
        let entry = self.roster.iter().find(|entry| entry.slot == slot)?;
        crate::autoreseat::leg::own_reset(quota.last_observation.as_ref(), entry, pin, now)
    }

    /// The return the seat at `slot` may make this cycle, from the held quota
    /// observation: none while auto reseat is off or the seat is latched.
    fn home(
        &self,
        events: &[Event],
        slot: &str,
        quota: &QuotaCarry,
        unlatched: bool,
        now: i64,
    ) -> Option<crate::autoreseat::leg::Home> {
        if self.auto.switch == crate::autoreseat::Switch::Off || !unlatched {
            return None;
        }
        let row = self.roster.iter().find(|entry| entry.slot == slot)?;
        let latched =
            crate::autoreseat::latched_identities(&self.roster, events, self.session, slot);
        let read = (quota.last_observation.as_ref(), latched.as_slice());
        let auto = self.auto.seat_view(self.lead_pair, slot);
        crate::autoreseat::leg::home(self.meta_dir, events, (self.session, row), &auto, read, now)
    }

    /// The seat at `slot`'s own account against the headroom threshold, from
    /// the held quota observation; unread whenever headroom may not act.
    fn own_room(
        &self,
        quota: &QuotaCarry,
        slot: &str,
        pin: Option<&str>,
        now: i64,
    ) -> crate::autoreseat::Room {
        let observed = quota.last_observation.as_ref();
        let on = self.auto.switch != crate::autoreseat::Switch::Off;
        let auto = self.auto.seat_view(self.lead_pair, slot);
        match (on, self.roster.iter().find(|entry| entry.slot == slot)) {
            (true, Some(entry)) => match auto.headroom_at {
                Some(at) => crate::autoreseat::leg::own_room(observed, entry, pin, at, now),
                None => crate::autoreseat::leg::own_relief(observed, entry, pin, now),
            },
            _ => crate::autoreseat::Room::Unread,
        }
    }

    /// One brief-retry pass: set aside what is permanently damaged, then spend
    /// this cycle's ONE delivery attempt on the next record in the rotation.
    ///
    /// Records are found by NAMING each roster slot's own path — never by
    /// listing the directory — so a legacy `undelivered.*.txt`, which carries
    /// no record, stays inert forever, and a file planted at any other name is
    /// never read.
    ///
    /// Damage classification is deliberately OUTSIDE the one-per-cycle budget.
    /// A single unreadable record that happens to be the oldest would otherwise
    /// eat the cycle's only attempt every cycle and starve every deliverable
    /// brief behind it for the whole half hour.
    ///
    /// THE ROTATION, and why oldest-first alone is not enough. Oldest-created
    /// goes first, because the brief that has waited longest should. But the
    /// leg answers `Skip` for a seat that is not ready, and a Skip mutates
    /// NOTHING — no attempt, no mark, no timestamp — so a stuck oldest record
    /// would be recomputed and re-picked every cycle, and every newer
    /// deliverable brief behind it would wait out the whole 30-minute bound.
    /// One stuck seat in a batch spawn is enough to do it. So `cursor` holds
    /// the SLOT tried last and the scan starts after it, wrapping: still one
    /// delivery per cycle, and every record gets its turn within roster-size
    /// cycles. The slot rather than an index, because the roster grows and
    /// shrinks between cycles; a slot that is gone starts the scan at the
    /// front.
    fn retry_briefs(
        &self,
        cursor: &mut Option<String>,
        verdicts: &[(String, SeatVerdict)],
        events: &[Event],
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let mut ready: Vec<(i64, String, String)> = Vec::new();
        for entry in &self.roster {
            // CHANNEL TWO of two. This cycle already judged the seat, so a
            // latched human-only prompt costs no capture to honour here. The
            // leg asks the pane again for itself — that is channel one, and it
            // is what covers a trigger that never came through this daemon.
            if slot_latched(verdicts, &entry.slot) {
                continue;
            }
            let Some(reading) = crate::brief_retry::read(self.meta_dir, &entry.slot) else {
                continue;
            };
            match reading {
                Ok(record) => ready.push((record.created, entry.slot.clone(), entry.name.clone())),
                Err(damaged) => {
                    self.set_damaged_aside(&entry.name, &entry.slot, damaged, events, now, err)?;
                }
            }
        }
        // The slot breaks a tie, so the order is TOTAL: two records written in
        // the same second must not swap places between cycles, or the rotation
        // could step over one of them forever.
        ready.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        if ready.is_empty() {
            return Ok(());
        }
        let after = cursor
            .as_deref()
            .and_then(|slot| ready.iter().position(|(_, at, _)| at == slot))
            .map_or(0, |at| at + 1);
        let (_, slot, name) = &ready[after % ready.len()];
        *cursor = Some(slot.clone());
        // The argv carries nothing that reaches the pane: the helper reads the
        // text AND the actor from the record. It is here only because the
        // helper's own grammar needs a message word.
        let delivery = self.deliver(
            name,
            RETRY_PLACEHOLDER,
            crate::brief_retry::RETRY_ACTION,
            "",
            None,
        );
        if delivery.code != Some(0) {
            // Every outcome the helper acts on, it has already recorded and
            // said out loud. A non-zero exit is the ordinary skip.
            writeln!(
                err,
                "ae: watchdog: brief retry for {name} did not deliver this cycle"
            )?;
        }
        Ok(())
    }

    /// The session-notice step: trigger the leg for every live, unlatched seat
    /// the fold finds a notice due for. The leg folds again under the target
    /// lock and journals every outcome, so a skip costs nothing here.
    fn tell_seats(&self, verdicts: &[(String, SeatVerdict)], events: &[Event], now: i64) {
        let bytes = crate::meta::read_bytes(self.meta_dir).unwrap_or_default();
        let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
        let interval = self.knobs.interval_secs;
        let bound = i64::try_from(interval).map_or(i64::MAX, |secs| secs.saturating_mul(2));
        for entry in &self.roster {
            let live = verdicts.iter().any(|(slot, seat)| {
                *slot == entry.slot && !matches!(seat.verdict, Verdict::Dead | Verdict::HumanPrompt)
            });
            let seat = crate::session_notice::Seat::read(&meta, &bytes, &entry.slot, self.session);
            if live && crate::session_notice::due(events, &seat, now, bound).is_some() {
                let action = crate::session_notice::TRIGGER_ACTION;
                let _ = self.deliver(&entry.name, action, action, &interval.to_string(), None);
            }
        }
    }

    /// Move a record ae will never read aside, once, and say so in the ledger.
    ///
    /// Only PERMANENT damage is destroyed. A read that merely failed may be a
    /// passing `EMFILE`, and the next cycle may read it perfectly well, so it
    /// is left exactly where it is until its own mtime proves it was never
    /// going to be read.
    ///
    /// THE ORDER IS THE PROOF: the give-up is durable BEFORE the rename. A
    /// daemon killed between the two leaves the record in place WITH its
    /// event — the next cycle's dedupe finds the ref, skips the append and
    /// only renames, so nothing is silent and nothing is repeated. A failed
    /// append likewise leaves the record where it is, said on stderr, for the
    /// next cycle to retry.
    fn set_damaged_aside(
        &self,
        name: &str,
        slot: &str,
        damaged: crate::brief_retry::Damaged,
        events: &[Event],
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        if !crate::brief_retry::should_destroy(&damaged, now) {
            writeln!(
                err,
                "ae: watchdog: {name}'s brief record could not be read ({}) — left alone, it may read next cycle",
                damaged.kind
            )?;
            return Ok(());
        }
        // THE DESTINATION IS STABLE: the record's own mtime dates the fallback
        // name, so the ref the event carries names a destination every later
        // cycle recomputes exactly — the dedupe below cannot drift into a
        // second event. No usable stamp fails closed to plain-only.
        let destination =
            match crate::brief_retry::destination(self.meta_dir, slot, damaged.modified) {
                Ok(destination) => destination,
                // Never a loop: it says so once per cycle and changes nothing.
                Err(why) => {
                    writeln!(err, "ae: watchdog: {why}")?;
                    return Ok(());
                }
            };
        let destination_ref = destination.display().to_string();
        if !announced(events, &destination_ref) {
            // THE REF IS THE IDEMPOTENCE KEY: it names the destination, is
            // written uncapped, and the next cycle matches it exactly.
            let appended = Journal {
                meta_dir: self.meta_dir,
                session: self.session,
            }
            .append_referring(
                crate::brief_retry::GAVE_UP_ACTION,
                name,
                &destination_ref,
                &format!(
                    "brief record damaged ({}) — given up; the record goes to {}; the brief is \
                     preserved at undelivered.{name}.txt",
                    damaged.kind, destination_ref
                ),
            );
            if let Err(why) = appended {
                writeln!(
                    err,
                    "ae: watchdog: brief-gave-up for {name} not recorded: {why} — the record \
                     stays in place",
                )?;
                return Ok(());
            }
        }
        if let Err(why) = crate::brief_retry::move_aside(self.meta_dir, slot, &destination) {
            writeln!(err, "ae: watchdog: {why}")?;
        }
        Ok(())
    }

    /// Append one watchdog event for `agent`.
    fn emit(
        &self,
        action: &str,
        agent: &str,
        summary: &str,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        Journal {
            meta_dir: self.meta_dir,
            session: self.session,
        }
        .record(action, agent, summary, err)
    }

    /// Roster agents with no live pane.
    fn sweep_missing(
        &self,
        live: &[String],
        missing: &mut Vec<(String, MissingState)>,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        for entry in &self.roster {
            let reference = entry.reference();
            if live.contains(&reference) {
                if let Some((_, state)) = missing.iter_mut().find(|(key, _)| *key == entry.slot) {
                    state.streak = 0;
                }
                continue;
            }
            // The GLYPH debounce is keyed by SLOT while the ALERT is keyed by
            // the display ref: two registrations can share a ref, but never a
            // slot.
            let state = entry_mut(missing, &entry.slot);
            state.streak = state.streak.saturating_add(1);
            if !state.alerted {
                state.alerted = true;
                self.emit(
                    "alert",
                    &reference,
                    "pane missing — agent no longer visible in session",
                    err,
                )?;
                self.notify(&reference, "pane is MISSING");
            }
        }
        Ok(())
    }

    /// The transient alert shown beside every watchdog event
    /// (`display-message -d 10000`).
    fn notify(&self, agent: &str, text: &str) {
        let Some(session_id) = transport::observe_session_id(self.server, self.session) else {
            return;
        };
        let message = tmux::format_literal(&format!("[ae watchdog] {agent} {text}"));
        let _ = transport::display_message(self.server, &session_id, &message);
    }
}

/// A completed verdict cycle is a foreground-safe scheduling edge. The 200 ms
/// motion ticker never calls this; cadence prevents redundant later children.
fn schedule_automatic_upgrade() {
    crate::autoupgrade::schedule();
}

/// The agents one window entry draws, in pane order.
///
/// Labels have already passed through [`theme::agent_label`]. A mark comes
/// first so a window's state is visible before its names: the mark itself
/// separates adjacent agents, so no brackets are needed. `working_frame`
/// replaces only working marks, allowing the ticker to animate the whole label
/// without re-reading a verdict.
fn window_agents_line(
    agents: &[(String, Mark)],
    look: &Look,
    working_frame: Option<&theme::WorkingFrame>,
) -> String {
    agents
        .iter()
        .map(|(agent, mark)| {
            let (glyph, fg) = if *mark == Mark::Working {
                working_frame.map_or(
                    (mark.glyph(look.icons), look.palette.accent(*mark)),
                    |frame| (frame.glyph, frame.fg.as_str()),
                )
            } else {
                (mark.glyph(look.icons), look.palette.accent(*mark))
            };
            format!("#[fg={fg}]{glyph}#[default]{agent}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What ONE roster entry is saying.
///
/// The single owner of that judgement: the session's rolled-up attention and
/// therefore every other session's fleet strip read it here, so a slot whose
/// pane has gone missing cannot disappear from session attention.
fn slot_mark(
    entry: &RosterEntry,
    by_slot: &[(String, SeatVerdict)],
    missing: &[(String, MissingState)],
) -> Mark {
    let found = by_slot.iter().find(|(slot, _)| *slot == entry.slot);
    found.map_or_else(
        || {
            let seen_absent = missing
                .iter()
                .any(|(slot, state)| *slot == entry.slot && state.streak > 0);
            if seen_absent {
                Mark::NeedsYou
            } else {
                Mark::Idle
            }
        },
        |(_, verdict)| verdict.mark(),
    )
}

/// How much of an entry's observed cells one attempt at the fact writes.
///
/// A roster that will not fit is degraded in RUNGS, and every rung is
/// fleet-wide: the model cell is a COLUMN, and one dropped for some rows and
/// not others is one the human cannot read down. The order spends the least
/// useful fact first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FactRung {
    /// Everything this cycle observed.
    Full,
    /// Models and clients, without the effort or the drift mark.
    NoEffort,
    /// The four v1 fields, the client and the task, with no observed model,
    /// effort or drift. The client and the task are the cheapest cells on the
    /// row and the ones that never drop.
    ClientOnly,
    /// Exactly today's v1 bytes.
    ///
    /// The LAST rung before nothing, and the reason it exists: v3 costs five
    /// more separators per entry, so without it a roster that fits today could
    /// vanish BECAUSE model cells were added — and a fact ae cannot publish is
    /// a roster the picker reports as unavailable.
    Legacy,
}

impl FactRung {
    /// The rungs, most informative first.
    const LADDER: [Self; 4] = [Self::Full, Self::NoEffort, Self::ClientOnly, Self::Legacy];

    /// The version word this rung announces.
    const fn version(self) -> &'static str {
        match self {
            Self::Full | Self::NoEffort | Self::ClientOnly => "v3",
            Self::Legacy => "v1",
        }
    }
}

/// What one roster seat's client cell shows: the operator's own `[clients]`
/// label, fully written.
///
/// Recorded truth first, current config second, nothing written to the meta:
/// the recorded `client.<slot>` override (the precedence `run::read_seat`
/// honors — the seat RUNS that client even when its row left the config, so
/// no lookup gates it), then the profile's `[clients]` row today, then the
/// recorded binary name, then today's short code. Every rung allowlists and
/// falls through; the answer is always emittable.
fn seat_client_label(entry: &RosterEntry, cfg: Option<&crate::config::IdentityConfig>) -> String {
    if let RecordedClient::Label(label) = &entry.client
        && crate::config::is_client_label(label)
    {
        return label.clone();
    }
    if let (Some(cfg), Some(profile)) = (cfg, entry.profile.as_deref())
        && let Some(label) = cfg.profile_client_label(profile)
        && crate::config::is_client_label(&label)
    {
        return label;
    }
    if let Some(binary) = entry.binary.as_deref()
        && crate::config::is_client_label(binary)
    {
        return binary.to_owned();
    }
    crate::tool::ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or_default())
        .client_token()
        .to_owned()
}

/// The watchdog-owned agent fact in recorded roster order.
///
/// Present panes carry the verdict this cycle already computed. A roster seat
/// with no pane remains visible as `dead` with an empty navigation hint. Any
/// unrepresentable recorded identity rejects the whole value rather than
/// publishing a partial roster the picker could mistake for complete.
///
/// A seat's observed cells are its own: a model ae cannot represent — over
/// cap, or carrying a byte the grammar forbids — empties that ENTRY's trio and
/// nothing else ([`observed_cells`]). Only a whole fact that will not fit
/// walks the [`FactRung`] ladder.
fn agents_fact(
    roster: &[RosterEntry],
    by_slot: &[AgentObservation],
    clients: &[(&str, String)],
    now_epoch: i64,
    interval_secs: u64,
) -> Option<String> {
    if roster.is_empty()
        || roster.len() > tmux::PICKER_AGENTS_MAX_COUNT
        || now_epoch < 0
        || !(1..=tmux::PICKER_AGENTS_MAX_INTERVAL_SECS).contains(&interval_secs)
    {
        return None;
    }
    FactRung::LADDER
        .into_iter()
        .find_map(|rung| fact_at(roster, by_slot, clients, now_epoch, interval_secs, rung))
}

/// One attempt at the fact, at exactly `rung`'s fidelity.
///
/// `None` means either a roster this writer must never publish at all — a
/// recorded identity it cannot spell — or simply one that does not fit at this
/// rung, which the ladder above answers by trying a plainer one.
#[allow(
    clippy::too_many_arguments,
    reason = "one composition of the cycle's already-derived roster slices"
)]
fn fact_at(
    roster: &[RosterEntry],
    by_slot: &[AgentObservation],
    clients: &[(&str, String)],
    now_epoch: i64,
    interval_secs: u64,
    rung: FactRung,
) -> Option<String> {
    let mut value = format!("{};{now_epoch};{interval_secs}", rung.version());
    let mut names: Vec<&str> = Vec::new();
    for entry in roster {
        let profile = entry.profile.as_deref()?;
        if !crate::config::is_agent_name(&entry.name)
            || !crate::config::is_config_key(profile)
            || names.contains(&entry.name.as_str())
        {
            return None;
        }
        names.push(&entry.name);
        let found = by_slot.iter().find(|seen| seen.slot == entry.slot);
        let (state, pane) = found.map_or(("dead", ""), |seen| {
            (seen.verdict.reason(), seen.pane.as_str())
        });
        if !pane.is_empty() && !tmux::pane_id_is_valid(pane) {
            return None;
        }
        value.push(';');
        value.push_str(&entry.name);
        value.push(':');
        value.push_str(profile);
        value.push(':');
        value.push_str(state);
        value.push(':');
        value.push_str(pane);
        if rung != FactRung::Legacy {
            let client = clients
                .iter()
                .find(|(slot, _)| *slot == entry.slot)
                .map_or("-", |(_, client)| client.as_str());
            let (model, effort, drift) = observed_cells(found.map(|seen| &seen.identity), rung);
            value.push(':');
            value.push_str(client);
            value.push(':');
            value.push_str(model);
            value.push(':');
            value.push_str(effort);
            value.push(':');
            value.push_str(drift);
            // The cheapest cell, so every rung but Legacy keeps it: `1` when
            // the seat tool's job is proven, else empty.
            value.push(':');
            if found.is_some_and(|seen| seen.verdict.task) {
                value.push('1');
            }
        }
        if value.len() > tmux::PICKER_AGENTS_MAX_BYTES {
            return None;
        }
    }
    Some(value)
}

/// One seat's three observed cells at `rung`, empty where nothing may be said.
///
/// The trio is a UNIT. A model this fact cannot carry takes the effort and the
/// drift mark with it, because an effort beside a profile alias would read as
/// that profile's effort, and a drift mark with no model names no
/// disagreement. Nothing is escaped or truncated: a label ae cannot spell
/// EXACTLY is one it does not show.
fn observed_cells(identity: Option<&SeatIdentity>, rung: FactRung) -> (&str, &str, &'static str) {
    let empty = ("", "", "");
    if rung == FactRung::ClientOnly {
        return empty;
    }
    let Some(identity) = identity else {
        return empty;
    };
    let Some(model) = identity
        .model
        .as_deref()
        .filter(|model| model_is_writable(model))
    else {
        return empty;
    };
    if rung == FactRung::NoEffort {
        return (model, "", "");
    }
    (
        model,
        identity
            .effort
            .as_deref()
            .filter(|effort| crate::harness_state::is_effort_word(effort))
            .unwrap_or_default(),
        if identity.drift { "!" } else { "" },
    )
}

/// Whether a model label can be spelled in the fact EXACTLY as observed.
///
/// The separators and the tmux style bytes are the fact's own grammar, and a
/// vendor label is free text never promised to avoid them. A label carrying
/// one is dropped rather than escaped: an escape would be a second grammar for
/// the picker to get wrong, and the parser's arity check would refuse the
/// whole roster over one vendor's punctuation.
fn model_is_writable(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= tmux::PICKER_AGENTS_MAX_MODEL
        && !model.starts_with(' ')
        && !model.ends_with(' ')
        && model.bytes().all(|byte| {
            (b' '..=b'~').contains(&byte) && !matches!(byte, b'|' | b',' | b'#' | b':' | b';')
        })
}

/// The carried state for `key`, created on first sight.
fn entry_mut<'a, V: Default>(list: &'a mut Vec<(String, V)>, key: &str) -> &'a mut V {
    let index = list
        .iter()
        .position(|(held, _)| held == key)
        .unwrap_or_else(|| {
            list.push((key.to_owned(), V::default()));
            list.len() - 1
        });
    let Some((_, value)) = list.get_mut(index) else {
        unreachable!("the index either came from this vector or was just pushed");
    };
    value
}

/// The descendancy for a pane, with BOTH unusable inputs mapped to `Unknown`.
fn descendancy_of(
    table: Option<&[procs::Proc]>,
    pane_pid: Option<u32>,
    bin: Option<&str>,
) -> Descendancy {
    match (pane_pid, bin) {
        (Some(pid), Some(binary)) => procs::descendancy(table, pid, binary),
        _ => Descendancy::Unknown,
    }
}

/// The session's events in APPEND ORDER, oldest first.
pub(crate) fn read_events(meta_dir: &Path) -> Vec<Event> {
    let bytes = store::open(meta_dir).container();
    crate::event_text::read_lines(&bytes)
        .into_iter()
        .filter_map(|line| Event::parse_line(&String::from_utf8_lossy(line)).ok())
        .collect()
}

/// Whether a give-up whose `ref` names `destination` exactly is already in the
/// ledger — the idempotence that makes the give-up's two steps (append, then
/// rename) safe to interrupt: the ref is written uncapped and matched exactly,
/// never through the capped summary.
fn announced(events: &[Event], destination: &str) -> bool {
    events.iter().any(|event| {
        event.action == crate::brief_retry::GAVE_UP_ACTION
            && event.reference.as_deref() == Some(destination)
    })
}

/// Whether the meta declares this session the fleet orchestrator.
fn is_meta_agent(meta_bytes: &[u8]) -> bool {
    crate::meta::meta_agent_role(meta_bytes) == crate::meta::MetaAgentRole::Role
}

/// The session this meta directory serves — its `session=` key, or the
/// directory's own name, which is what the directory IS named.
fn session_name(meta_bytes: &[u8], meta_dir: &Path) -> String {
    String::from_utf8_lossy(meta_bytes)
        .lines()
        .find_map(|line| line.strip_prefix("session="))
        .filter(|value| !value.is_empty())
        .map_or_else(
            || {
                meta_dir
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            },
            ToOwned::to_owned,
        )
}

/// The watchdog bar's glyph, from the cycle's COUNTS.
/// The watch segment of the bar.
///
/// A HEALTHY watch says only that it is watching: the counts are a monitoring
/// fact, and the bar is a hierarchy where the session, its windows and its
/// agents come first. The moment a pane is dead or stale the counts come back,
/// in the mark's own accent, because then they are the news.
fn watch_segment(published: &Published<'_>) -> String {
    let look = published.look;
    let healthy = published.active == published.total;
    if healthy {
        return format!("#[fg={}]{}", look.palette.dim, published.bar);
    }
    format!(
        "#[fg={}]{} {}/{}#[default]",
        look.palette.accent(published.attention),
        published.bar,
        published.active,
        published.total,
    )
}

fn bar_glyph(dead: usize, stale: usize, icons: bool) -> &'static str {
    if dead > 0 {
        Verdict::Dead.glyph(icons)
    } else if stale > 0 {
        Verdict::Stale.glyph(icons)
    } else {
        Verdict::Active.glyph(icons)
    }
}

#[cfg(test)]
#[path = "watchdog_limitwake_spec.rs"]
mod limitwake_spec;

#[cfg(test)]
#[path = "watchdog_autoacts_spec.rs"]
mod autoacts_spec;

#[cfg(test)]
#[path = "watchdog_pairguard_spec.rs"]
mod pairguard_spec;

#[cfg(test)]
#[path = "watchdog_daemon/return_tests.rs"]
mod return_tests;

/// The rests between iterations, on a private server: a stepped rest waits
/// for its step, an unstepped retry is one plain sleep.
#[cfg(test)]
mod rest_tests;

/// [`auto_acts_with`] with no seat on an automatic chain: the step's pins read it.
#[cfg(test)]
fn auto_acts(
    settings: &crate::autoreseat::Settings,
    seats: &[AutoSeat],
    events: &[Event],
    context: (&str, bool),
    now: i64,
    named: &mut Vec<(String, Timestamp, crate::autoreseat::HoldReason)>,
) -> Vec<AutoAct> {
    let context = (context.0, context.1, false);
    auto_acts_with(settings, (seats, &[]), events, context, now, named)
}

#[cfg(test)]
mod limitwake_readers;

#[cfg(test)]
#[path = "watchdog_daemon/activity225_spec.rs"]
mod activity225_spec;
#[cfg(test)]
#[path = "watchdog_daemon/holdend315_spec.rs"]
mod holdend315_spec;
#[cfg(test)]
#[path = "watchdog_daemon/leadfloor2_notice_spec.rs"]
mod leadfloor2_notice_spec;
#[cfg(test)]
#[path = "watchdog_daemon/protowd_spec.rs"]
mod protowd_spec;
#[cfg(test)]
mod quietwd_spec;

#[cfg(test)]
mod tests;
