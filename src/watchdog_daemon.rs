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
mod facts;
pub(crate) use facts::read_events;
use facts::{
    agents_fact, announced, bar_glyph, descendancy_of, entry_mut, is_meta_agent,
    schedule_automatic_upgrade, seat_client_label, session_name, slot_mark, watch_segment,
    window_agents_line,
};
mod acts;
use acts::{AutoAct, AutoSeat, armed_reading, auto_acts_with, reads_reset, waiting_forecast};
mod adoption;
use adoption::{
    ADOPTION_TICK, Adoption, adoption_due, adoption_writes, enumerate_adoption, fleet_rows,
    orchestrator_id_for, overlay_current_rank,
};
mod ticker;
use ticker::{
    MotionState, MotionVerdict, Pace, PaneMark, PublishedOrchestratorStrip, STEP_BACKSTOP,
    StepPace, menu_open_expired, tick_pane_duties, wait_between_cycles, wait_between_cycles_with,
};
mod quota_carry;
pub(crate) use quota_carry::in_lead_pair;
use quota_carry::{
    GO_USAGE_FLYING, QuotaAction, QuotaCarry, QuotaDelivery, QuotaInputs, ask_receipt,
    delivered_receipts, history_clock, history_same, limit_wake_in, live_model,
    quota_ask_candidates, quota_observation_due, quota_recipients, quota_samples_at,
    start_pass_due, throttle_quota_line,
};

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

#[cfg(test)]
use facts::{FactRung, fact_at};

#[cfg(test)]
#[path = "watchdog_limitwake_spec.rs"]
mod limitwake_spec;

#[cfg(test)]
use quota_carry::{
    PendingAdvisory, PendingAsk, QuotaAskCandidate, QuotaKey, QuotaRecipient, quota_samples,
};

#[cfg(test)]
use ticker::{
    MotionTicker, TickerMode, motion_cadence, motion_failure, motion_observation_due,
    motion_publish_failure, motion_ticker_enabled, static_observe_cadence, ticker_mode,
};

#[cfg(test)]
use adoption::{Adopted, AdoptionBackend, WatchdogPresence, adoption_from, proven_ownership};

#[cfg(test)]
use acts::{Headroom, headroom_route, name_hold, unmapped_act};

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
