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
    overlay_current_rank,
};
mod ticker;
use ticker::{
    MotionState, MotionVerdict, Pace, PaneMark, STEP_BACKSTOP, StepPace, menu_open_expired,
    tick_pane_duties, wait_between_cycles, wait_between_cycles_with,
};
mod quota_carry;
pub(crate) use quota_carry::in_lead_pair;
use quota_carry::{
    GO_USAGE_FLYING, QuotaAction, QuotaCarry, QuotaDelivery, QuotaInputs, ask_receipt,
    delivered_receipts, history_clock, history_same, limit_wake_in, live_model,
    quota_ask_candidates, quota_observation_due, quota_recipients, quota_samples_at,
    start_pass_due, throttle_quota_line,
};
mod accounting;
#[cfg(doc)]
use accounting::waiting_agent_escalates;
pub use accounting::{
    Accounting, LIMIT_WAKE_GRACE_SECS, NO_EVENT_AGE, WaitCheck, account, account_settling,
    account_with_activity, account_with_trust, age_secs, declaration_age, declared_clause,
    done_challenge_text, idle_nudge_text, idle_nudge_text_waiting, last_actor_event_age,
    limit_wake_text, nudge_text, nudge_words, record_nudge, stale_display, target_moved_text,
    wait_challenge_text,
};
use accounting::{
    capture_pane, epoch_second, held_seats, last_done_event_at, last_working_declaration_at,
    naming, observed_option, restore_idle, slot_latched, sweep_effects, system_time_from_epoch,
};
mod deadnotice;
pub use deadnotice::{Death, DeathEpisode};
use deadnotice::{Reading, Standing};
mod blocknotice;
mod charter;
mod continuation;
mod cycle_apply;
mod cycle_observe;
mod cycle_publish;
mod reset_expiry;
mod self_reseat;

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
    /// pane (a re-run in place), or on a closer of its death episode in the
    /// journal (its own later activity, a spawn, reseat or `ae _run` start)
    /// — which alerts nothing further and clears it with one `dead-cleared`
    /// (`pane-back` when the standing alert says the pane was gone).
    /// A probe gap is not evidence of life, so an UNKNOWN snapshot keeps the
    /// latch. A seat that dies again after a clear is alerted again. The
    /// memory is only half the latch: a dead alert the journal holds for the
    /// seat's current episode ([`Activity::death`]) latches a fresh carry, so a
    /// restarted daemon never alerts the same death twice.
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
    /// The visible dialog held by the orchestrator's sweep, independent of redraws.
    pub sweep_prompt: Option<u64>,
    /// The launch whose continuation the helper last proved refused before paste.
    pub continuation_refused: Option<String>,
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
    /// The desk-stale change this cycle's observation booked for the
    /// orchestrator main's delivery, taken by the turn that carries it and
    /// cleared after the pane's effects. Never carried into a next cycle.
    pub desk: Option<crate::desktick::Booking>,
}

/// What the cycle knows BESIDE its [`Observation`] (#225, #283): carried as a
/// separate value so the observation's shape — built literally by many specs
/// — does not change. [`Activity::default`] is exactly today's accounting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Activity {
    /// [`crate::procs::task_running`] for the seat this cycle.
    pub task_running: Option<bool>,
    /// The TRACKED quiet state's proof reading (done or wait). `None` leaves
    /// the proof frame carry untouched and holds nothing.
    pub proof: Option<crate::watchdog::ProofQuiet>,
    /// The watchdog's newest painting delivery to the seat, epoch seconds.
    pub painting: Option<i64>,
    /// The journal half of the dead latch: whether the human's dead alert
    /// already stands in the seat's current death episode.
    pub death: DeathEpisode,
    /// The seat's held-on target this cycle ([`crate::wait_on::held_on`])
    /// with its trusted motion (#225). `None` buys no target hold.
    pub wait_target: Option<WaitTarget>,
    /// The wait episode's `wait-held` witness
    /// ([`crate::watchdog::WaitReading::held`]).
    pub wait_witness: Option<crate::watchdog::HoldWitness>,
    /// The seat's direct same-session target holds a current `working`
    /// declaration ([`crate::wait_on::on_current_working`]): its
    /// `waiting-agent` ceiling is [`crate::watchdog::waiting_agent_ceiling_for`]'s
    /// longer one. `false` keeps the base ceiling.
    pub wait_on_working: bool,
}

/// The motion the watchdog already trusts for a waited-on seat (#225), each
/// half independent: its own newest journal record, its pane's newest
/// motion (a busy frame or a changed hash, from its carry) and a live own
/// job within the task lease.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TargetMotion {
    /// Epoch of the target's newest own journal record.
    pub journal: Option<i64>,
    /// Epoch of the target pane's newest motion (`PaneState::last_hash_change`).
    pub pane: Option<i64>,
    /// The target's tool runs a live task within its lease this cycle.
    pub job: bool,
}

impl TargetMotion {
    /// The newest motion epoch: a live job counts as `now`.
    #[must_use]
    pub fn newest(&self, now: i64) -> Option<i64> {
        [self.journal, self.pane, self.job.then_some(now)]
            .into_iter()
            .flatten()
            .max()
    }
}

/// A waited-on seat a due wait challenge may be held on (#225).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitTarget {
    /// The waited-on seat's name.
    pub agent: String,
    /// Its trusted motion this cycle.
    pub motion: TargetMotion,
}

/// The target's motion from `events` (its own records, by
/// [`crate::watchdog::event_is_actor`]) beside the pane and job halves the
/// daemon reads from the target's carry and the process table (#225).
#[must_use]
pub fn target_motion(
    events: &[Event],
    session: &str,
    slot: &str,
    agent: &str,
    pane: Option<i64>,
    job: bool,
) -> TargetMotion {
    let journal = events
        .iter()
        .rev()
        .find(|event| crate::watchdog::event_is_actor(event, session, slot, agent))
        .map(|event| event.ts.epoch());
    TargetMotion { journal, pane, job }
}

/// Whether a target hold holds the due `waiting-agent` challenge this cycle
/// (#225): the episode is due and younger than
/// [`crate::watchdog::wait_hold_cap`], and the target's newest motion is
/// younger than [`crate::watchdog::quiet_ceiling`].
#[must_use]
pub fn target_held(
    progress: WaitProgress,
    target: Option<&WaitTarget>,
    now: i64,
    cadence: u64,
) -> bool {
    let due = matches!(
        progress,
        WaitProgress::ChallengeDue { wait_age_secs, .. }
            if wait_age_secs < crate::watchdog::wait_hold_cap(cadence)
    );
    due && target
        .and_then(|target| target.motion.newest(now))
        .is_some_and(|at| age_secs(now, at) < crate::watchdog::quiet_ceiling(cadence))
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
    /// The orchestrator sweep reading, `Some` only for the orchestrator main.
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
/// disagree with another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SeatVerdict {
    verdict: Verdict,
    /// The frame read this cycle lets the pane border name a Codex service wait.
    frame: HarnessState,
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
        match (self.verdict, self.frame) {
            (Verdict::Active, HarnessState::CodexServiceWait) => {
                crate::harness_state::CODEX_SERVICE_WAIT_REASON
            }
            _ => self.verdict.reason(),
        }
    }
}

impl From<Verdict> for SeatVerdict {
    fn from(verdict: Verdict) -> Self {
        Self {
            verdict,
            frame: HarnessState::Unknown,
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
    /// Journal `wait-held` (#225): the due wait challenge is held for
    /// `reason`, a stronger reason than the episode's witness; `on` names
    /// the waited-on seat of a target hold. Paints nothing.
    WaitHeld {
        reason: crate::watchdog::HoldReason,
        on: Option<String>,
    },
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
    /// Write the block record of this seat's client scope: the first sighting
    /// of an account block. The record is the first writer's, so a repeat
    /// changes nothing; only `ae quota clear` removes it.
    BlockScope {
        /// What the vendor refused.
        kind: crate::vendorblock::Kind,
        /// The harness whose line was read: the scope of a seat the roster no
        /// longer names is the one configured client of this tool.
        tool: crate::tool::ToolKind,
        /// The vendor's own reason, from the notice.
        reason: String,
    },
    /// ONE quota pass outside the cadence, on the cycle a seat left the usage
    /// limit. `run` collects it and performs it once; `quota = off` runs none.
    QuotaRefresh,
    /// Deliver the ONE continuation turn a limited seat is owed once its
    /// known `reset` has passed, through the guarded send like a nudge.
    LimitWake { reset: i64 },
    /// Deliver one SWEEP prompt to the orchestrator: the changed overview,
    /// with a desk-stale line when one is due, or the standalone desk-stale
    /// tick when only that is due.
    SweepNudge,
    /// Reconcile or attempt this launch's continuation, with a due overview if booked.
    ReseatContinuation {
        /// The durable phase and exact launch this turn belongs to.
        owed: crate::meta::ReseatContinuation,
        /// Whether to include and book the overview in the same checked turn.
        overview: bool,
        /// Whether the move that started `owed.launch` was the orchestrator's
        /// own request, read with `owed` from the same meta bytes.
        requested: bool,
    },
    /// Claim the orchestrator's pending reseat request: the trigger through
    /// the one exec site ([`crate::reseat::request::trigger`]).
    SelfReseatClaim,
    /// Deliver the orchestrator the refusal or failure its request ended in,
    /// named by the ending record's `reference`.
    SelfReseatNotice { reference: String, text: String },
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

impl Effect {
    /// Whether the wake settle (#238) holds this effect: every paste, key
    /// answer or journal credit that stands for one, which the next free cycle
    /// books again. Records, notices and notes pass.
    #[must_use]
    pub const fn held_while_settling(&self) -> bool {
        matches!(
            self,
            Self::Nudge
                | Self::DoneChallenge { .. }
                | Self::DoneHeld { .. }
                | Self::WaitHeld { .. }
                | Self::WaitChallenge { .. }
                | Self::TargetMoved { .. }
                | Self::LimitWake { .. }
                | Self::SweepNudge
                | Self::AnswerTrust { .. }
                | Self::SelfReseatClaim
                | Self::SelfReseatNotice { .. }
        ) || matches!(
            self,
            Self::ReseatContinuation {
                owed: crate::meta::ReseatContinuation {
                    phase: crate::meta::ContinuationPhase::Owed,
                    ..
                },
                ..
            }
        )
    }
}

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
/// knob (300 in production defaults; CLI flags may override it in tests).
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

/// The launch row a snapshot's `ids` hold for `slot`: what a leg to that
/// seat is asked to prove before anything runs. Every other slot's row says
/// nothing about it.
fn launch_row<'a>(ids: &'a [(String, SeatLaunch)], slot: &str) -> crate::tracked::LaunchRow<'a> {
    match ids.iter().find(|(at, _)| at == slot) {
        None => crate::tracked::LaunchRow::Legacy,
        Some((_, SeatLaunch::Id(id))) => crate::tracked::LaunchRow::Token(id),
        Some((_, SeatLaunch::Unproven(why))) => crate::tracked::LaunchRow::Damaged(why),
    }
}

fn launch_id_for<'a>(ids: &'a [(String, SeatLaunch)], slot: &str) -> Option<&'a str> {
    ids.iter()
        .find(|(seat, _)| seat.as_str() == slot)
        .and_then(|(_, launch)| match launch {
            SeatLaunch::Id(id) => Some(id.as_str()),
            SeatLaunch::Unproven(_) => None,
        })
}

/// One `launch_id.<slot>` row of the cycle's meta snapshot: the incarnation every
/// delivery this cycle decides for that seat must still find (#218 2c).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SeatLaunch {
    /// Exactly one usable token.
    Id(String),
    /// Empty, duplicate or non-UTF-8: why no incarnation was proven.
    Unproven(&'static str),
}

impl From<&str> for SeatLaunch {
    fn from(id: &str) -> Self {
        Self::Id(id.to_owned())
    }
}

/// EVERY `launch_id.<slot>` row in `bytes`, in first-seen order, classified by the
/// seat proof's own reader. A slot it does not list had positively no row.
fn seat_launches(bytes: &[u8]) -> Vec<(String, SeatLaunch)> {
    let mut found: Vec<(String, SeatLaunch)> = Vec::new();
    for line in bytes.split(|byte| *byte == b'\n') {
        let Some((key, _)) = line.strip_prefix(b"launch_id.").and_then(|rest| {
            rest.iter()
                .position(|byte| *byte == b'=')
                .map(|at| rest.split_at(at))
        }) else {
            continue;
        };
        let Ok(slot) = std::str::from_utf8(key) else {
            continue;
        };
        if found.iter().any(|(seen, _)| seen == slot) {
            continue;
        }
        let launch = match tracked::launch_row(bytes, slot) {
            tracked::LaunchRow::Token(id) => SeatLaunch::Id(id.to_owned()),
            tracked::LaunchRow::Damaged(why) => SeatLaunch::Unproven(why),
            // Unreachable: the row was just seen.
            tracked::LaunchRow::Legacy => continue,
        };
        found.push((slot.to_owned(), launch));
    }
    found
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
                        launch_ids: seat_launches(bytes),
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
        self.record_routed(action, (target, ""), reference, summary, err)
    }

    /// Append one watchdog event addressed to one SEAT: `target` is its display
    /// name, and a non-empty `slot` routes the record by slot and this session,
    /// so a display name that another slot takes over later never reads this
    /// seat's record. An empty `slot` writes the keyless line every other
    /// record uses.
    fn record_routed(
        &self,
        action: &str,
        (target, slot): (&str, &str),
        reference: &str,
        summary: &str,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        if let Err(why) = self.append_referring(action, (target, slot), reference, summary) {
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
    /// A non-empty `slot` fills the routing key (`target_slot` and
    /// `target_session`, and `target_session_id` when the meta names a usable
    /// one, so the record still matches after a session rename): half a key
    /// addresses nobody, so they go together.
    fn append_referring(
        &self,
        action: &str,
        (target, slot): (&str, &str),
        reference: &str,
        summary: &str,
    ) -> Result<(), store::Error> {
        let session_id;
        let (target_session, target_session_id) = if slot.is_empty() {
            ("", "")
        } else {
            session_id = crate::meta::session_id_in(self.meta_dir);
            let key = crate::events::SessionKey::new(self.session, &session_id);
            (self.session, key.id.unwrap_or(""))
        };
        let line = tracked::event_line(&EventFields {
            ts: Timestamp::now(),
            actor: ACTOR,
            action,
            target,
            reference,
            actor_slot: "",
            actor_session: self.session,
            target_slot: slot,
            target_session,
            actor_session_id: "",
            target_session_id,
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
    /// Where each seat's spawner notice stands (#211), keyed by roster slot:
    /// ONE standing per seat, whichever way it dies (dropped to its shell, or
    /// its pane gone) and however the two alternate. Memory only; the journal
    /// holds what was delivered.
    notices: Vec<(String, Standing)>,
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
    /// The wake settle's held cycles left ([`crate::watchdog::settle`]). In
    /// memory like `clock`: a restarted daemon has no previous reading, so it
    /// detects no suspension to hold for.
    settle: u8,
    /// A limit-release quota refresh the wake settle held (#238): owed until
    /// the first free cycle's refresh returns.
    refresh_owed: bool,
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
            notices: Vec::new(),
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
            settle: 0,
            refresh_owed: false,
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

/// EVERY session-scoped value this daemon publishes, the three attention
/// options included: only a live daemon can vouch for any of them, and every
/// OTHER session on the server reads the rank and the strip. The two RETIRED
/// orchestrator options stay named: a session an older core dressed still
/// carries them, and the retraction is what unsets them (human ruling 1a,
/// 2026-11-17).
const RETRACTED_SESSION_OPTIONS: [&str; 10] = [
    tmux::WATCHDOG_STATUS_OPTION,
    theme::ATTENTION_GLYPH_OPTION,
    theme::ATTENTION_RANK_OPTION,
    theme::ATTENTION_STYLE_OPTION,
    theme::AGENTS_OPTION,
    theme::FLEET_STRIP_OPTION,
    theme::RETIRED_ORCHESTRATOR_STRIP_OPTION,
    theme::RETIRED_ORCHESTRATOR_ID_OPTION,
    theme::GOAL_OPTION,
    theme::VERSION_OPTION,
];

/// Remove everything this daemon published, and report whether it all came off.
pub(crate) fn clear_published(server: &crate::inventory::ServerId, session: &str) -> bool {
    let Some(session_id) = transport::observe_session_id(server, session) else {
        return false;
    };
    // `&=`, NEVER `&&` and never an early return: every option must still be
    // attempted after one of them fails.
    let mut ok = true;
    // A caller whose session is still RUNNING puts the launch seed back over
    // the three attention options, immediately after the retraction — see
    // `seed_unwatched` below for why an unwatched session says Stale rather
    // than nothing.
    for name in RETRACTED_SESSION_OPTIONS {
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

/// The debounce for a roster agent whose pane is not in the session. Whether
/// the human was alerted is not memory: the journal's death episode says it
/// ([`deadnotice::death_episode`]), so a restart never alerts the same death again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MissingState {
    streak: u32,
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
    /// inherit the old seat's observation, and the incarnation every delivery
    /// this cycle decides must still find. A slot it does not list had no row.
    launch_ids: Vec<(String, SeatLaunch)>,
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
    checkpoint: crate::watchdog_state::OverviewCheckpoint,
}

impl OverviewReading {
    fn changed(&self) -> bool {
        !crate::overview::same_overview(self.checkpoint.hash.as_deref(), &self.hash)
    }

    /// Whether the `NEEDS YOU` set differs from the last delivered overview's.
    fn needs_changed(&self) -> bool {
        crate::overview::needs_changed(self.checkpoint.hash.as_deref(), &self.hash)
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
        self.compose(None)
    }

    /// The overview turn with `line` after the fleet text, before the
    /// trailer. Unlike [`Self::with_remedy`] the hash stays the overview's
    /// own: the line is about the desk, not the fleet.
    fn body_with(&self, line: &str) -> String {
        self.compose(Some(line))
    }

    /// The one builder of both turns: the fleet text, an optional line after
    /// it, then the trailer. Without a line the bytes are the overview's own.
    fn compose(&self, line: Option<&str>) -> String {
        let rendered = match line {
            None => std::borrow::Cow::Borrowed(self.rendered.as_str()),
            Some(line) => {
                let mut rendered = self.rendered.clone();
                if !rendered.is_empty() && !rendered.ends_with('\n') {
                    rendered.push('\n');
                }
                rendered.push_str(line);
                std::borrow::Cow::Owned(rendered)
            }
        };
        crate::overview::nudge_body(&rendered)
    }

    /// This reading with `line` after the fleet text, before the trailer.
    /// The line is part of what the reading says, so it is part of its hash:
    /// an unchanged fleet with a new line is a changed overview.
    fn with_remedy(mut self, line: &str) -> Self {
        if !self.rendered.ends_with('\n') {
            self.rendered.push('\n');
        }
        self.rendered.push_str(line);
        self.hash = remedy_hash(&self.hash, line);
        self
    }
}

/// The overview hash `hash` extended by the line `line`, in the same 64-bit
/// FNV-1a and 16-digit form as [`crate::overview::semantic_hash`].
fn remedy_hash(hash: &str, line: &str) -> String {
    let mut value = 0xcbf2_9ce4_8422_2325_u64;
    for byte in hash.bytes().chain([0]).chain(line.bytes()) {
        value ^= u64::from(byte);
        value = value.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{value:016x}")
}

/// The owner's remedy while the orchestrator seat's charter turn is
/// unconfirmed: a restart of the seat, which only the owner runs. When the
/// seat's state directory `meta_dir` sits in a named instance's root, its
/// restart is that instance's own ae command, so no default-root command is
/// printed that would stop a different live seat.
fn charter_remedy(meta_dir: &Path, session: &str) -> String {
    let root = meta_dir.parent().and_then(Path::parent);
    match root.and_then(crate::instance::instance_of) {
        None => format!(
            "orchestrator charter update uncertain: ae stop {session}, then ae orchestrator"
        ),
        Some(name) => format!(
            "orchestrator charter update uncertain: restart the orchestrator seat of instance \
             {name} with that instance's own ae command"
        ),
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
    /// CHECKOUT ONLY, exactly like `AE_TEST_BOOT_TIME` (`src/l1_platform/doors.rs`):
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
        let found = carry.clock.replace(clock).and_then(|prev| {
            crate::watchdog::host_suspended(prev, clock, self.knobs.interval_secs)
        });
        // The wake settle (#238): this cycle and the next stable one deliver
        // nothing. Every held step below runs again on the first free cycle.
        carry.settle = crate::watchdog::settle(carry.settle, found);
        let settling = carry.settle > 0;
        if let Some(found) = found {
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
        let mut continuation = self.read_reseat_obligation();
        let continuation_owed = continuation.is_some();
        // An owed continuation gets the seat's turn. Otherwise, the charter's
        // attempt owns the cycle and an unresolved charter names the remedy.
        let charter = if continuation
            .as_ref()
            .is_some_and(|(row, _)| row.phase == crate::meta::ContinuationPhase::Owed)
        {
            charter::Charter::Idle
        } else {
            self.charter_update(&observed, settling, err)?
        };
        let (overview, desk) = if self.meta_agent
            && self.knobs.sweep.enabled()
            && charter != charter::Charter::Attempted
        {
            let ack = observed
                .iter()
                .find(|pane| pane.slot.as_deref() == Some(crate::watchdog::MAIN_SLOT))
                .and_then(|pane| pane.agent.as_deref());
            self.overview(now, ack, err)
                .map_or((None, None), |(reading, desk)| {
                    let reading = if charter == charter::Charter::Unresolved {
                        reading.with_remedy(&charter_remedy(self.meta_dir, self.session))
                    } else {
                        reading
                    };
                    (Some(reading), Some(desk))
                })
        } else {
            (None, None)
        };
        // ONE cadence, one counter: `quota_every_secs` paces the periodic
        // quota pass, and zero disables it.
        let inputs = QuotaInputs {
            panes: &observed,
            held: &carry.panes,
            events: &events,
        };
        if !settling {
            self.run_quota_cadence(&mut carry.quota, &inputs, now, err)?;
        }

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
        // ONE snapshot of every seat's pane and job motion before any pane is
        // accounted (#225): the carried motion of the same incarnation and
        // its task lease, so the accounting order cannot change a hold.
        let motions: Vec<(String, String, Option<i64>, bool)> = observed
            .iter()
            .filter_map(|pane| {
                let agent = pane.agent.as_deref().filter(|name| !name.is_empty())?;
                let slot = pane.slot.clone().unwrap_or_default();
                let identity = quiet_hash(&format!("{slot}\n{agent}"));
                let carried = carry
                    .panes
                    .iter()
                    .find(|(id, state)| *id == pane.pane_id && state.identity == Some(identity))
                    .map(|(_, state)| state);
                let running = pane
                    .pane_pid
                    .zip(self.agent_bin(&slot))
                    .and_then(|(pid, bin)| procs::task_running(stat_table.as_deref(), pid, &bin));
                let lease = task_lease(carried.and_then(|state| state.task_since), running, now);
                let job = task_deferred(lease, running, now, &self.knobs);
                let pane_motion = carried.and_then(|state| state.last_hash_change);
                Some((agent.to_owned(), slot, pane_motion, job))
            })
            .collect();

        // ONE client read per cycle; a failed read changes no pane's memory.
        let clients = transport::observe_clients(self.server);
        let mut live: Vec<String> = Vec::new();
        let mut counts = Counts::default();
        let mut by_slot: Vec<(String, SeatVerdict)> = Vec::new();
        let mut by_agent: Vec<AgentObservation> = Vec::new();
        let mut by_pane: Vec<PaneMark> = Vec::new();
        // Cycle-wide: any seat leaving the limit this sweep requests ONE pass.
        let mut quota_refresh = false;
        // A block a seat drew THIS cycle is told this cycle: its journal row
        // postdates the cycle's event snapshot.
        let mut drawn_block = false;
        // Every seat whose limit latch stands after its accounting.
        let mut auto_seats: Vec<AutoSeat> = Vec::new();
        // The return each seat on an automatic chain may make (#305), by slot.
        let mut homes: Vec<(String, crate::autoreseat::leg::HomeRead)> = Vec::new();
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
                held: wait_witness,
            } = wait_state.map_or(
                crate::watchdog::WaitReading {
                    progress: WaitProgress::None,
                    quiet: crate::watchdog::ProofQuiet::default(),
                    rung: 0,
                    held: None,
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
                death: deadnotice::death_episode(&events, key, (&slot, agent)),
                // The direct same-session target a `waiting-agent` may be
                // held on, with its three motion halves (#225).
                wait_target: (wait_state == Some(WaitState::WaitingAgent))
                    .then(|| crate::wait_on::held_on(&wait_links, agent))
                    .flatten()
                    .and_then(|name| motions.iter().find(|(seat, ..)| seat == name))
                    .map(|(name, target_slot, pane_motion, job)| WaitTarget {
                        agent: name.clone(),
                        motion: target_motion(
                            &events,
                            self.session,
                            target_slot,
                            name,
                            *pane_motion,
                            *job,
                        ),
                    }),
                wait_witness,
                wait_on_working: crate::wait_on::on_current_working(
                    &wait_links,
                    &wait_facts,
                    agent,
                ),
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
                sweep: self
                    .sweep_observation(
                        &slot,
                        agent,
                        &events,
                        (overview.as_ref(), desk.as_ref()),
                        now,
                    )
                    .map(|sweep| {
                        sweep.with_turn_running(
                            crate::harness_state::classify(&capture, tool)
                                == crate::harness_state::HarnessState::Busy,
                        )
                    }),
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
            let mut booked = account_settling(
                carried,
                &seen,
                &self.knobs,
                trust.as_ref(),
                &activity,
                settling,
            );
            if seen.sweep.is_some()
                && let Some(owed) = continuation.take()
            {
                continuation::book_from(&mut booked, &seen, owed, settling);
            }
            if seen.sweep.is_some() && self.meta_agent {
                let seat = (self.meta_dir, (self.session, agent));
                self_reseat::book(
                    &mut booked,
                    &seen,
                    &events,
                    seat,
                    (settling, continuation_owed),
                );
            }
            // The latch this cycle started from: the carried one, or the journal's
            // standing dead alert a fresh carry was rebuilt from.
            let was_latched =
                carried.dead_latched || matches!(activity.death, DeathEpisode::Open(_));
            let reading = Reading {
                was_latched,
                latched: booked.next.dead_latched,
                cleared: booked.effects.iter().any(|effect| {
                    matches!(
                        effect,
                        Effect::Emit {
                            action: "dead-cleared" | deadnotice::BACK_ACTION,
                            ..
                        }
                    )
                }),
                command: pane.current_command.as_str(),
            };
            *carried = booked.next;
            carried.desk = seen
                .sweep
                .as_ref()
                .and(desk.as_ref())
                .and_then(|sight| sight.booking.clone());
            drawn_block |= booked
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::BlockScope { .. }));
            self.apply_booked(&booked.effects, &acting, carried, &mut quota_refresh, err)?;
            carried.desk = None;
            let standing = entry_mut(&mut carry.notices, &slot);
            self.notice_step(&acting, &reading, (settling, standing), err)?;
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
                // ORCHESTRATOR MODEL GUARD: this cycle's read of its due move.
                let read = self.orchestrator_stays(
                    &carry.quota,
                    &events,
                    (&slot, agent),
                    (
                        &capture,
                        (
                            &own_room,
                            carried.limit_reset,
                            carried.limit_since.is_some() || resume.exhausted.is_some(),
                        ),
                    ),
                    now,
                );
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
                    stays: read.as_ref().and_then(|read| read.stays.clone()),
                    held_tail: read.map(|read| read.tail),
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
            let frame = seen.harness.frame;
            by_slot.push((
                slot.clone(),
                SeatVerdict {
                    verdict: booked.verdict,
                    frame,
                    task,
                },
            ));
            let shown = SeatVerdict {
                verdict: shown,
                frame,
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
        // A seat leaving its limit requests the refresh once, on the cycle its
        // latch clears: a request the wake settle holds stays owed until the
        // first free cycle, and clears only after that refresh returned.
        carry.refresh_owed |= quota_refresh;
        if !settling {
            self.refresh_after_limit_release(
                carry.refresh_owed,
                &mut carry.quota,
                &inputs,
                now,
                err,
            )?;
            carry.refresh_owed = false;
            self.retry_briefs(&mut carry.brief_cursor, &by_slot, &events, now, err)?;
            self.tell_seats(&by_slot, &events, now);
            self.tell_blocks((&by_slot, &observed), &events, drawn_block, now);
        }
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
        if !settling {
            crate::effort::leg::step(&effort, &effort_seats, &mut carry.effort, err)?;
            self.auto_reseat(
                &mut carry.auto_named,
                (&auto_seats, &homes),
                &events,
                now,
                err,
            )?;
        }
        if self.meta_agent {
            crate::reseat::request::sweep(
                self.meta_dir,
                self.session,
                Timestamp::from_epoch(now),
                err,
            )?;
        }
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
        self.sweep_missing(live, carry, err)?;
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
}

#[cfg(test)]
use facts::{FactRung, fact_at};

#[cfg(test)]
#[path = "watchdog_limitwake_spec.rs"]
mod limitwake_spec;

#[cfg(test)]
use accounting::{deferred, holds_seat};

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

#[cfg(test)]
#[path = "watchdog_daemon/orchcool_tests.rs"]
mod orchcool_tests;

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
#[path = "watchdog_daemon/banked_notice_spec.rs"]
mod banked_notice_spec;
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
mod sleepgap_cycle_spec;
#[cfg(test)]
mod sleepgap_refresh_spec;
#[cfg(test)]
mod sleepgap_spec;
#[cfg(test)]
mod vendorblock_roster_spec;
#[cfg(test)]
mod waithold_cycle_spec;

#[cfg(test)]
#[path = "watchdog_daemon/vendorblock_latch_spec.rs"]
mod vendorblock_latch_spec;
#[cfg(test)]
pub(crate) use vendorblock_latch_spec::apply_booked;

#[cfg(test)]
mod tests;
