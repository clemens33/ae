//! The watchdog daemon — the loop that observes a session's panes each cycle,
//! asks [`crate::watchdog`] what it is looking at, and applies the answers.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
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
    QuietCycle, QuietKind, QuietPane, SweepAlert, SweepEffect, SweepKnobs, SweepObservation,
    SweepState, SweepVerdict, Throttle, classify_dead, declaration_key, is_sweep_target,
    latest_relevant_event, quiet_hash, quiet_pane_decision, quiet_reason, quiet_stabilize,
    record_sweep, stale_composite, sweep_step, throttle_class,
};

/// The event actor every watchdog emission carries.
const ACTOR: &str = "watchdog";

/// The panes that are not agents: unstamped, tmux's own null, this daemon, the
/// events pane, and the two older names a session can still carry.
const NON_AGENT_PANES: [&str; 5] = ["(null)", "_watchdog", "_events", "_shepherd", "_loop"];

/// The bound on consecutive unusable process snapshots before the daemon says
/// so.
const UNKNOWN_ALERT_CYCLES: u32 = 5;

/// Motion cadence while at least one client can see the session.
const ATTACHED_MOTION_TICK: Duration = Duration::from_millis(100);

/// Attached animation frames between process and fleet observations.
const MOTION_OBSERVATION_TICKS: u8 = 5;

/// Motion cadence while nobody can see the session.
const DETACHED_MOTION_TICK: Duration = Duration::from_secs(2);

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
    /// The beat between the two captures a quiet baseline must match across.
    pub quiet_beat_ms: u64,
    /// How many re-captures the stabilizer may take before giving up.
    pub quiet_tries: usize,
    /// How many panes may pay that beat in one cycle.
    pub quiet_panes_per_cycle: usize,
    /// The orchestrator changed-overview spacing, retry and bound.
    pub sweep: SweepKnobs,
    /// Seconds between best-effort Telegram bridge revives.
    pub tg_supervise_secs: u64,
}

impl Default for Knobs {
    fn default() -> Self {
        Self {
            interval_secs: 60,
            quota_every_secs: 300,
            quota_aware: true,
            idle_nudge_secs: crate::watchdog::DEFAULT_IDLE_NUDGE_SECS,
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
    verdict: Verdict,
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
    /// Dead is LATCHED once alerted, and the latch ends exactly once: on a
    /// POSITIVE process reading that shows the seat's harness back under the
    /// pane (a re-run in place), which alerts nothing further and clears it
    /// with one `dead-cleared`. A probe gap is not evidence of life, so an
    /// UNKNOWN snapshot keeps the latch. A seat that dies again after a clear
    /// is alerted again.
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
    /// The usage-limit latch. Its ONE release is a cycle judged at all that
    /// no longer shows the phrase: it retracts the verdict, requests recovery.
    pub limit_latched: bool,
    /// Consecutive cycles showing a human-only prompt. It lives HERE, in the
    /// carry, so a dead seat's reset clears it for free and a restart begins
    /// again — exactly like the two latches above it. There is NO separate
    /// latch flag: the streak IS the latch, reaching the bound exactly once,
    /// which is how `throttle_streak` already names a persistent throttle.
    pub human_prompt_streak: u32,
    /// Consecutive nudges that did not land.
    pub undelivered_streak: u32,
    /// Consecutive cycles whose process snapshot was unusable.
    pub unknown_streak: u32,
    /// The persistent-unknown alert is raised once per streak, not per cycle.
    pub unknown_alerted: bool,
    /// The armed quiet baseline: declaration key, settled hash, and the number
    /// of consecutive cycles whose hash differed from the prior baseline.
    pub quiet_base: Option<(String, u64, u8)>,
    /// The orchestrator sweep branch's carry.
    pub sweep: SweepState,
}

/// What the cycle saw about one pane, after [`crate::watchdog`]'s pure
/// classifiers ran over it.
#[derive(Debug, Clone)]
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
    /// Whether a process named the agent binary runs under the pane.
    pub descendancy: Descendancy,
    /// Age of the newest event this agent is the ACTOR of.
    pub last_actor_event_age_secs: u64,
    /// The orchestrator sweep reading, `Some` ONLY for the orchestrator main
    /// agent with the cadence enabled.
    pub sweep: Option<SweepObservation>,
    /// What this seat is still owed by somebody else: requests it SENT that
    /// nobody answered, agents it SPAWNED that still hold a seat.
    pub own_work: crate::session::OwnWork,
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
    /// Moving, recently moved, or recently active in the log.
    Active,
    /// The orchestrator main, judged by its own cadence rather than by silence.
    Meta(SweepVerdict),
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
            Self::Stale | Self::Meta(SweepVerdict::MetaStarting) => Mark::Stale,
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
    /// A line for the human, published with `display-message`.
    Notify(String),
    /// ONE quota pass outside the cadence, on the cycle a seat left the usage
    /// limit. `run` collects it and performs it once; `quota = off` runs none.
    QuotaRefresh,
    /// Deliver one SWEEP prompt to the orchestrator.
    SweepNudge,
    /// Reconcile the durable event log against a wedge alert this daemon does
    /// not remember raising — the post-restart clear.
    ReconcileWedge,
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
    last_observation: Option<crate::quota::Observation>,
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

fn quota_recipients(roster: &[RosterEntry], lead_pair: bool) -> Vec<QuotaRecipient> {
    roster
        .iter()
        .filter(|entry| entry.slot == "main" || (lead_pair && entry.slot == "worker.0"))
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
            })
        })
        .collect()
}

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
        // booked to fire the moment awareness returns.
        self.asks.clear();
        self.last_observation = None;
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
    /// neither recipient set can reach the other's bookings.
    fn cancel_asks_where(
        &mut self,
        predicate: impl Fn(&PendingAsk) -> bool,
        reason: &str,
        meta_dir: &Path,
        actions: &mut Vec<QuotaAction>,
    ) {
        let mut retained = Vec::new();
        for ask in self.asks.drain(..) {
            if predicate(&ask) {
                actions.push(QuotaAction::Dropped {
                    recipient: ask.recipient.agent,
                    summary: format!("{reason}: {}", ask.advisory.checkpoint_ask(meta_dir)),
                });
            } else {
                retained.push(ask);
            }
        }
        self.asks = retained;
    }

    /// Book ONE checkpoint ask per seat on the scope this window belongs to.
    ///
    /// Called only from an ENTRY into the band. A seat that already holds an
    /// undelivered ask for this key has it REPLACED rather than doubled: a band
    /// that cleared and was entered again while the first ask was still
    /// deferred must still produce exactly one ask, carrying the newer facts.
    fn book_asks(
        &mut self,
        key: &QuotaKey,
        held: &crate::quota::Classified,
        advisory: &crate::quota::Advisory,
        candidates: &[QuotaAskCandidate],
        group: &crate::quota::Group,
    ) {
        for candidate in candidates.iter().filter(|candidate| {
            candidate.identity.tool == group.tool && candidate.identity.source == key.source
        }) {
            self.asks
                .retain(|ask| !(ask.key == *key && ask.recipient == candidate.recipient));
            self.asks.push(PendingAsk {
                key: key.clone(),
                observed_at: held.observed_at(),
                recipient: candidate.recipient.clone(),
                advisory: advisory.clone(),
                attempts: 0,
            });
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
            actions,
        );
        self.cancel_asks_where(
            |ask| !ask.advisory.current_at(now),
            "checkpoint ask expired",
            meta_dir,
            actions,
        );
    }

    fn reconcile_with_candidates(
        &mut self,
        observation: &crate::quota::Observation,
        recipients: &[QuotaRecipient],
        candidates: &[QuotaAskCandidate],
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

        for sample in samples {
            let previous = self
                .tracked
                .iter()
                .position(|tracked| tracked.key == sample.key);
            let Some(index) = previous else {
                // The first sight of a window is classified and SILENT: there
                // is no transition to report yet.
                let classified = crate::quota::Classified::first(sample.reading);
                // The ASK is not silent here, and that is the whole point: a
                // daemon that starts with the scope already Low has seats which
                // may never get another transition to be warned by. The
                // advisory's own first-sight silence above is untouched.
                if entered_low(None, classified.level()) {
                    let advisory = observation.transition(sample.group, &classified);
                    self.book_asks(
                        &sample.key,
                        &classified,
                        &advisory,
                        candidates,
                        sample.group,
                    );
                }
                self.tracked.push(QuotaTracked {
                    key: sample.key,
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
            let advisory = observation.transition(sample.group, &held);
            // ENTERING the band asks every seat on the scope once. Moving
            // around inside it does not, and neither does leaving it; only a
            // re-entry after `classify`'s hysteresis let the window clear asks
            // again. A declaration change re-derives the held row and reaches
            // here the same way, so a withdrawn reset re-arms the ask exactly
            // as it re-arms the advisory.
            if entered_low(Some(before), after) {
                self.book_asks(&sample.key, &held, &advisory, candidates, sample.group);
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
                        ask.advisory.checkpoint_ask(meta_dir)
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
                ask.advisory.checkpoint_ask(meta_dir)
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

/// The nudge: the session goal when the meta carries one, then the status
/// sentence, then the path to this session's own `state` helper.
///
/// The invitation spells the SAME tail `watchdog::raw_nudge` strips from a
/// pane baseline, so the two can never disagree about what a nudge looks like.
#[must_use]
pub fn nudge_text(goal: Option<&str>, meta_dir: &Path) -> String {
    let prefix = goal.map_or_else(String::new, |goal| format!("Session goal: {goal}. "));
    format!(
        "{prefix}Status check: if you have more work, continue. Otherwise declare your state so \
         I stop nudging: {}/state <waiting-user|waiting-agent|blocked|done> \"<reason>\"",
        meta_dir.display()
    )
}

/// The idle reminder uses the same delivery path but names the positive
/// observation that started its independent clock.
#[must_use]
pub fn idle_nudge_text(goal: Option<&str>, meta_dir: &Path) -> String {
    let prefix = goal.map_or_else(String::new, |goal| format!("Session goal: {goal}. "));
    format!(
        "{prefix}you look idle: declare state or continue. State helper: {}/state \
         <waiting-user|waiting-agent|blocked|done> \"<reason>\"",
        meta_dir.display()
    )
}

/// The same reminder for a seat whose OWN work is outstanding — it reached the
/// deferral ceiling, so it is told WHAT ae thinks it is waiting on rather than
/// being asked a question it already answered.
#[must_use]
pub fn idle_nudge_text_waiting(goal: Option<&str>, meta_dir: &Path, reason: &str) -> String {
    format!(
        "{} ae still shows you {reason} — chase them, or declare state.",
        idle_nudge_text(goal, meta_dir)
    )
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

/// The usage-limit branch: throttling's nudge suppression, plus ONE durable
/// `limit` event per episode — the word `ae list` reads.
/// Whether THIS cycle judged `slot` to be waiting on a human-only prompt —
/// channel two of the retry's two, and pure so it can be pinned without a pane.
/// A slot the cycle did not judge at all is not latched: absence of a verdict
/// is not a verdict, and channel one still asks the pane for itself.
fn slot_latched(verdicts: &[(String, Verdict)], slot: &str) -> bool {
    verdicts
        .iter()
        .any(|(at, verdict)| at == slot && *verdict == Verdict::HumanPrompt)
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
/// verdict nobody can act on is not news. ae NEVER sends the key.
fn book_human_prompt(
    prior: &PaneState,
    next: &mut PaneState,
    effects: &mut Vec<Effect>,
    seen: &Observation,
    knobs: &Knobs,
) -> Option<Verdict> {
    let prompt = seen.human_prompt.as_ref()?;
    next.human_prompt_streak = prior.human_prompt_streak.saturating_add(1);
    if next.human_prompt_streak < knobs.human_prompt_cycles {
        return None;
    }
    // EXACTLY at the bound, so the episode is named once however long it
    // lasts — `book_throttle`'s rule, and the reason no latch flag is needed.
    if next.human_prompt_streak == knobs.human_prompt_cycles {
        effects.push(Effect::Emit {
            action: "human-prompt",
            summary: format!("{} — press: {}", prompt.question, prompt.keys),
        });
        effects.push(Effect::Notify(format!(
            "waiting for you: {} — press: {}",
            prompt.question, prompt.keys
        )));
    }
    Some(Verdict::HumanPrompt)
}

fn book_limit(next: &mut PaneState, effects: &mut Vec<Effect>, seen: &Observation) {
    if !next.limit_latched {
        next.limit_latched = true;
        effects.push(Effect::Emit {
            action: "limit",
            summary: "vendor usage limit reached — waits for a reset or a re-login".to_owned(),
        });
    }
    // The limit episode ends any transient streak: a return to plain
    // throttling is news again, not a continuation.
    next.throttle_streak = 0;
    next.prev_hash = Some(seen.hash);
    next.last_hash_change = Some(seen.now_epoch);
    next.idle_since_epoch = None;
    next.nudge_count = 0;
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
/// stands. The declaration's age is `seen.last_actor_event_age_secs` because
/// the newest event the seat is the actor of IS the declaration the hold was
/// armed from — any newer one ends the hold before this branch is reached.
///
/// Past the ceiling the seat becomes exactly `blocked`: the ordinary nudge
/// budget resumes (`book_stale`; the nudge half, off when `idle_nudge_secs` is
/// zero like every nudge), and the verdict published is `Quiet(Blocked)` — the
/// attention half, on the same ceiling the read surfaces derive from the same
/// pin, so a human surface can never disagree with the pane.
fn book_waiting_agent_escalation(
    prior: &PaneState,
    next: &mut PaneState,
    effects: &mut Vec<Effect>,
    seen: &Observation,
    knobs: &Knobs,
) -> Option<Verdict> {
    if seen.quiet != Some(QuietKind::WaitingAgent)
        || !crate::watchdog::waiting_agent_escalated(
            seen.last_actor_event_age_secs,
            knobs.idle_nudge_secs,
        )
    {
        return None;
    }
    if knobs.idle_nudge_secs > 0 {
        book_stale(prior, next, effects, knobs, seen.last_actor_event_age_secs);
    }
    Some(Verdict::Quiet(QuietKind::Blocked))
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

/// The ONE end a death latch has: a POSITIVE process-tree reading shows the
/// seat's harness back under the pane (the human's re-run in place). An
/// UNKNOWN snapshot is not evidence of life, so a probe gap — and a reading
/// that says the process is still gone — keeps the latch and returns `None`.
/// On a clear it emits the one `dead-cleared` and hands back the episode the
/// ordinary judgement must run on: identity kept, and every clock, hash and
/// nudge field the death interrupted reset, because a pre-death hash or idle
/// clock must not feed a stale verdict. No hysteresis and no second-alert
/// suppression: the caller alerts again if the seat dies again, because that
/// is a real event each time.
fn clear_death_latch(
    prior: &PaneState,
    next: &PaneState,
    seen: &Observation,
    effects: &mut Vec<Effect>,
) -> Option<PaneState> {
    if !prior.dead_latched || !matches!(seen.descendancy, Descendancy::Present) {
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
        limit_latched: false,
        human_prompt_streak: 0,
        ..next.clone()
    })
}

/// Account for one pane in one cycle — the branch order, and the only place
/// any of it is decided.
#[must_use]
pub fn account(prior: &PaneState, seen: &Observation, knobs: &Knobs) -> Accounting {
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
    let mut effects = Vec::new();
    book_unknown(&mut next, &mut effects, seen.descendancy);

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

    // 2.
    if seen.is_dead {
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

    account_ordinary(prior, next, effects, seen, knobs)
}

/// Steps 3 through 10 — the ordinary judgement of a pane that is not dead (any
/// more). `prior` is the carry the verdict is judged against: the reset
/// episode a clear just returned, or the standing carry.
fn account_ordinary(
    prior: &PaneState,
    mut next: PaneState,
    mut effects: Vec<Effect>,
    seen: &Observation,
    knobs: &Knobs,
) -> Accounting {
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
    //     of absence: the latch holds and the verdict stays `limit` below.
    if prior.limit_latched && seen.capture_ok && seen.throttle != Some(Throttle::LimitReached) {
        next.limit_latched = false;
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
    if seen.capture_ok && seen.human_prompt.is_none() {
        if prior.human_prompt_streak >= knobs.human_prompt_cycles {
            effects.push(Effect::Emit {
                action: "human-prompt-cleared",
                summary: "the human-only prompt is gone from the pane".to_owned(),
            });
        }
        next.human_prompt_streak = 0;
    }

    // 6. A quiet declaration. A FRESH `waiting-agent` holds like the other
    // quiet states; past its ceiling it escalates (see the helper). While the
    // hold stands, the newest event this agent is the actor of IS its
    // declaration: any newer one would have ended the quiet state in
    // `resolve_quiet`.
    if let Some(kind) = seen.quiet {
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
    //    not a clearing.
    if seen.throttle == Some(Throttle::LimitReached) || (prior.limit_latched && !seen.capture_ok) {
        book_limit(&mut next, &mut effects, seen);
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
    if let Some(prompt) = book_human_prompt(prior, &mut next, &mut effects, seen, knobs) {
        return Accounting {
            next,
            effects,
            verdict: prompt,
            moved: false,
        };
    }

    // 9. Harness frames outrank the legacy motion heuristic.
    if let Some(verdict) = account_harness(prior, &mut next, &mut effects, seen, knobs) {
        return Accounting {
            next,
            effects,
            verdict,
            moved: false,
        };
    }

    // 10. Unknown frames retain the legacy motion and actor-event rule.
    account_unknown(prior, next, effects, seen, knobs)
}

/// Positive harness-frame branches. `None` means the caller must use the
/// conservative legacy heuristic.
fn account_harness(
    prior: &PaneState,
    next: &mut PaneState,
    effects: &mut Vec<Effect>,
    seen: &Observation,
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
            if seen.harness.durable_stale {
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
            let idle_age = age_secs(seen.now_epoch, *idle_since);
            if seen.harness.durable_stale || next.nudge_count > knobs.max_nudges {
                return Some(Verdict::Stale);
            }
            // An empty input box is the right reading of the PIXELS and the
            // wrong reading of the FACTS when the seat is the one everybody
            // else is waiting on. Defer the nudge; never suppress it for good.
            if knobs.idle_nudge_secs > 0
                && idle_age >= knobs.idle_nudge_secs
                && !deferred(seen.own_work, seen.now_epoch, idle_age, knobs)
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
    seen: &Observation,
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
    if !stale {
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
        return vec![
            Effect::Emit {
                action: "alert",
                summary: format!(
                    "nudge unreachable/occupied — {} undelivered attempts ({display})",
                    state.undelivered_streak
                ),
            },
            Effect::Notify(format!(
                "unreachable — {} nudges could not be delivered",
                state.undelivered_streak
            )),
        ];
    }
    Vec::new()
}

/// Seconds between `at` and `now`, clamped at zero.
#[must_use]
pub fn age_secs(now_epoch: i64, at_epoch: i64) -> u64 {
    u64::try_from(now_epoch.saturating_sub(at_epoch)).unwrap_or(0)
}

/// The age of the newest event this SEAT is the actor of — judged by the ONE
/// routing-aware actor rule ([`crate::watchdog::event_is_actor`]), so a
/// same-display event from another incarnation is not this seat's activity.
/// The `waiting-agent` escalation measures its ceiling on this age: the
/// declaration the quiet hold was armed from IS the newest own event.
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
    match event.actor_identity() {
        crate::events::Identity::Routed {
            slot,
            session: owner,
        } => slot == crate::watchdog::MAIN_SLOT && owner == session,
        crate::events::Identity::Display(display) => display == agent,
        crate::events::Identity::Unassociated => false,
    }
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
fn quota_seconds(meta_bytes: &[u8], fallback: u64) -> Result<u64, String> {
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
            Verdict::Stale => self.stale += 1,
            _ => self.active += 1,
        }
    }
}

/// One pane's verdict this cycle.
struct PaneMark {
    /// The `%<n>` pane id.
    pane: String,
    /// What the accounting made of it.
    verdict: Verdict,
    /// The watchdog-owned public observation, including restart carry.
    observed: String,
}

/// One cycle verdict in the window layout the ticker animates between cycles.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MotionVerdict {
    pane: String,
    window: String,
    verdict: Verdict,
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

    /// Advance every attached working surface from the cached observation.
    fn step(&mut self, look: &Look) -> Vec<tmux::OptionWrite> {
        if !self.panes.iter().any(|pane| pane.session_attached > 0) {
            return Vec::new();
        }
        let working: Vec<&MotionVerdict> = self
            .verdicts
            .iter()
            .filter(|entry| entry.verdict.mark() == Mark::Working)
            .filter(|entry| {
                self.panes.iter().any(|pane| {
                    pane.pane_id == entry.pane
                        && pane.agent.as_deref().is_some_and(|agent| {
                            !agent.is_empty() && !NON_AGENT_PANES.contains(&agent)
                        })
                })
            })
            .collect();
        let fleet_working = self.fleet.iter().any(|row| row.mark == Mark::Working);
        if !working.is_empty() || fleet_working {
            self.spin = self.spin.wrapping_add(1);
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
    sockets: &'a crate::SocketPaths,
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
    let root = crate::state_root()?;
    // META ONLY: the picker's read. Identities and the `placed` rule, never a
    // session's journal — nothing here is derived from events.
    let scan = crate::inventory::durable_meta_records(&crate::inventory::Roots::under(&root));
    let mut sockets = crate::SocketPaths::asking(transport::observe_socket_path);
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
    // The OWNERSHIP proof, made once per name this server is actually showing.
    // A record whose session is not on this server costs nothing, and neither
    // does this daemon's own: it is never a target and never vouched for, so
    // probing it would buy two tmux spawns a cycle and no answer anyone reads.
    let proven: Vec<crate::inventory::DiscoveredSession> = scan
        .records
        .iter()
        .map(|record| record.name.clone())
        .filter(|name| name != session)
        .filter(|name| listing.iter().any(|row| &row.name == name))
        .map(|name| crate::inventory::DiscoveredSession {
            marker: proven_ownership(
                transport::observe_session_ownership(server, &name).as_ref(),
                &root,
            ),
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

/// Whether the attached ticker must refresh its cached observations now.
const fn motion_observation_due(ticks_since_observation: u8) -> bool {
    ticks_since_observation >= MOTION_OBSERVATION_TICKS
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
/// animated ticker observes at — every fifth attached tick, every detached.
fn static_observe_cadence(panes: &[tmux::MotionPane]) -> Duration {
    if panes.iter().any(|pane| pane.session_attached > 0) {
        ATTACHED_MOTION_TICK.saturating_mul(u32::from(MOTION_OBSERVATION_TICKS))
    } else {
        DETACHED_MOTION_TICK
    }
}

/// The next observation cadence from the current attachment reading.
fn motion_cadence(panes: &[tmux::MotionPane]) -> Duration {
    if panes.iter().any(|pane| pane.session_attached > 0) {
        ATTACHED_MOTION_TICK
    } else {
        DETACHED_MOTION_TICK
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
    by_pane
        .iter()
        .map(|entry| entry.verdict.mark())
        .chain(roster.iter().copied())
        .max_by_key(|mark| mark.rank())
        .unwrap_or(Mark::Idle)
}

/// Run the watchdog for one session until its session or its state goes away.
///
/// # Errors
///
/// Only writing the status stream; every observation failure degrades within
/// the cycle instead.
pub fn run(
    meta_dir: &Path,
    mut knobs: Knobs,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
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
    // The pre-rename reap, which was `_watchdog_start`'s first act.
    crate::watchdog_glue::reap_legacy(&server, &session, meta_dir, err)?;
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

    let code = watch(
        meta_dir,
        knobs,
        server,
        &session,
        &helper,
        &journal,
        &mut deferred,
        err,
    );
    // The pidfile is released by `PidFile`'s Drop — ownership-checked, so a
    // stop/start in quick succession never lets the dying process vandalise its
    // successor's registration — and Drop, not an explicit call, so EVERY
    // return after publish releases it, the `?` exits above included.
    drop(pidfile);
    code
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
    err: &mut impl Write,
) -> crate::Result<u8> {
    let mut carry = Carry::new(&knobs);
    // The global config PATH is stable for this daemon's life; its CONTENT is
    // re-read every cycle below, so a config flip — quota awareness, or the
    // human's fleet order — reaches an unpinned session within one cycle.
    let global_config = crate::state_root()
        .or_else(|| {
            meta_dir
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        })
        .map(|root| crate::doors::config_file(crate::shape::current(), &root));
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
            Rebind::Use(named) => {
                server = adopt_server(
                    server,
                    named,
                    &mut carry,
                    &knobs,
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
                        // Re-read EVERY cycle, like the goal and the roster: a
                        // session can be promoted to orchestrator, or its main
                        // replaced, while this daemon runs.
                        meta_agent: is_meta_agent(bytes),
                        roster: meta.roster().to_vec(),
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
                    cycle.run(&mut carry, err)?;
                    // The pane's own per-cycle duties, in order: the branch
                    // pair, which is a git read no cycle owns, then the
                    // recovery and the revive.
                    tick_pane_duties(&server, meta_dir, session, meta, deferred, journal, err)?;
                }
                true
            }
        };
        if ran_cycle {
            wait_between_cycles(&server, session, &mut carry, knobs.interval_secs);
        } else {
            std::thread::sleep(Duration::from_secs(knobs.interval_secs));
        }
    }
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
    let interval = Duration::from_secs(interval_secs);
    let Some(look) = carry.look else {
        // No look has EVER answered, so this session draws nothing of its own —
        // but a peer it is filling still has a line to keep.
        wait_idle_between_cycles(server, carry, interval);
        return;
    };
    match ticker_mode(carry.look.as_ref()) {
        TickerMode::Idle => {
            wait_idle_between_cycles(server, carry, interval);
            return;
        }
        TickerMode::Static => {
            wait_static_between_cycles(server, session, carry, &look, interval);
            return;
        }
        TickerMode::Animated => {}
    }
    let started = Instant::now();
    let mut cadence = ATTACHED_MOTION_TICK;
    let mut ticks_since_observation = MOTION_OBSERVATION_TICKS;
    let mut failures = 0_u8;
    let mut adopted_at: Option<Instant> = None;
    loop {
        let remaining = interval.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        let mut next = carry.motion.clone();
        // STAGED like the motion carry beside it: a batch tmux refused must
        // leave both write-on-change memories where they were.
        let mut adoption = carry.adoption.clone();
        let mut adopted: Vec<tmux::OptionWrite> = Vec::new();
        let observed = motion_observation_due(ticks_since_observation);
        if observed {
            let reading = transport::observe_motion_panes(server, session);
            let fleet = transport::observe_fleet_listing(server);
            let (Some(reading), Some(fleet)) = (reading, fleet) else {
                let failed = motion_failure(failures);
                failures = failed.0;
                let remaining = interval.saturating_sub(started.elapsed());
                if failed.1 {
                    std::thread::sleep(remaining);
                    break;
                }
                std::thread::sleep(cadence.min(remaining));
                continue;
            };
            cadence = motion_cadence(&reading);
            next.replace_observation(reading, &fleet, &carry.adoption.known, session);
            // Adoption's own cadence, inside the branch that just read the
            // fleet: one listing per tick, never two, and never a frame this
            // daemon's own attachment gates.
            if adoption_due(&mut adopted_at, Instant::now()) {
                adopted = adoption_writes(&mut adoption, &fleet, &next.fleet_order);
            }
        }
        let mut writes = next.step(&look);
        writes.extend(adopted);
        if !writes.is_empty() && !transport::publish_options(server, &writes) {
            ticks_since_observation = MOTION_OBSERVATION_TICKS;
            let failed = motion_publish_failure(failures, observed);
            failures = failed.0;
            let remaining = interval.saturating_sub(started.elapsed());
            if failed.1 {
                std::thread::sleep(remaining);
                break;
            }
            std::thread::sleep(cadence.min(remaining));
            continue;
        }
        failures = 0;
        carry.motion = next;
        carry.adoption = adoption;
        ticks_since_observation = if cadence == DETACHED_MOTION_TICK {
            MOTION_OBSERVATION_TICKS
        } else if observed {
            1
        } else {
            ticks_since_observation.saturating_add(1)
        };
        let remaining = interval.saturating_sub(started.elapsed());
        std::thread::sleep(cadence.min(remaining));
    }
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
) {
    // Decided ONCE: the target set is the verdict cycle's to change, so a
    // daemon with none sleeps exactly as it did before adoption existed.
    if carry.adoption.targets.is_empty() {
        std::thread::sleep(interval);
        return;
    }
    let started = Instant::now();
    let mut failures = 0_u8;
    loop {
        let remaining = interval.saturating_sub(started.elapsed());
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
            let remaining = interval.saturating_sub(started.elapsed());
            if failed.1 {
                std::thread::sleep(remaining);
                break;
            }
            std::thread::sleep(ADOPTION_TICK.min(remaining));
            continue;
        }
        failures = 0;
        carry.adoption = adoption;
        let remaining = interval.saturating_sub(started.elapsed());
        std::thread::sleep(ADOPTION_TICK.min(remaining));
    }
}

/// Wait for the next verdict cycle while keeping the fleet strip fresh at
/// the observation cadence, without animation frames. Same three-failure
/// rule as the animated ticker.
fn wait_static_between_cycles(
    server: &crate::inventory::ServerId,
    session: &str,
    carry: &mut Carry,
    look: &Look,
    interval: Duration,
) {
    let started = Instant::now();
    let mut cadence = static_observe_cadence(&carry.motion.panes);
    let mut failures = 0_u8;
    let mut adopted_at: Option<Instant> = None;
    loop {
        let remaining = interval.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        let mut next = carry.motion.clone();
        let mut adoption = carry.adoption.clone();
        let reading = transport::observe_motion_panes(server, session);
        let fleet = transport::observe_fleet_listing(server);
        let (Some(reading), Some(fleet)) = (reading, fleet) else {
            let failed = motion_failure(failures);
            failures = failed.0;
            let remaining = interval.saturating_sub(started.elapsed());
            if failed.1 {
                std::thread::sleep(remaining);
                break;
            }
            std::thread::sleep(cadence.min(remaining));
            continue;
        };
        cadence = static_observe_cadence(&reading);
        next.replace_observation(reading, &fleet, &carry.adoption.known, session);
        let mut writes = next.step_static(look);
        if adoption_due(&mut adopted_at, Instant::now()) {
            writes.extend(adoption_writes(&mut adoption, &fleet, &next.fleet_order));
        }
        if !writes.is_empty() && !transport::publish_options(server, &writes) {
            let failed = motion_publish_failure(failures, true);
            failures = failed.0;
            let remaining = interval.saturating_sub(started.elapsed());
            if failed.1 {
                std::thread::sleep(remaining);
                break;
            }
            std::thread::sleep(cadence.min(remaining));
            continue;
        }
        failures = 0;
        carry.motion = next;
        carry.adoption = adoption;
        let remaining = interval.saturating_sub(started.elapsed());
        std::thread::sleep(cadence.min(remaining));
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
    err: &mut impl Write,
) -> crate::Result<()> {
    if let Some(session_id) = transport::observe_session_id(server, session) {
        crate::watchdog_glue::publish_branch(
            server,
            &session_id,
            crate::watchdog_glue::branch_reading(meta.work_dir()).as_ref(),
        );
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
    ) -> crate::Result<()> {
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
    ) -> crate::Result<()> {
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
        if let Err(why) = store::open(self.meta_dir).append_event(&line) {
            writeln!(
                err,
                "ae: watchdog: {action} for {target} not recorded: {why}"
            )?;
        }
        Ok(())
    }
}

/// Everything this daemon carries between cycles that is scoped to ONE SERVER.
struct Carry {
    /// Per-pane history, keyed by `pane_id`.
    panes: Vec<(String, PaneState)>,
    /// The missing-pane debounce, keyed by roster slot.
    missing: Vec<(String, MissingState)>,
    /// The rotating stabilization budget, whose cursor indexes THIS server's
    /// pane enumeration.
    quiet: QuietCycle,
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
    /// The last look this daemon actually READ, and `None` until one answers.
    ///
    /// Carried so that a cycle whose read failed draws in the look it saw last
    /// rather than in a guess: the alternative is a session with the theme off,
    /// or in another palette, being repainted in ae's default because one tmux
    /// call did not answer. Before the FIRST successful read there is no last
    /// look either, and a default would be that same guess — so nothing which
    /// depends on the look is published at all until one arrives.
    look: Option<Look>,
}

impl Carry {
    fn new(knobs: &Knobs) -> Self {
        Self {
            panes: Vec::new(),
            missing: Vec::new(),
            quiet: QuietCycle::new(knobs.quiet_panes_per_cycle),
            motion: MotionState::default(),
            quota: QuotaCarry::default(),
            adoption: Adoption::default(),
            brief_cursor: None,
            look: None,
        }
    }

    /// Drop every carry, because the server they are scoped to is being left.
    fn reset(&mut self, knobs: &Knobs) {
        *self = Self::new(knobs);
    }
}

/// Move this daemon from one server to another, in the ONE order that is safe.
fn adopt_server(
    leaving: crate::inventory::ServerId,
    joining: crate::inventory::ServerId,
    carry: &mut Carry,
    knobs: &Knobs,
    retract: impl FnOnce(&crate::inventory::ServerId) -> bool,
    journal: &Journal<'_>,
    err: &mut impl Write,
) -> crate::Result<crate::inventory::ServerId> {
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
    carry.reset(knobs);
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

/// What one pane's quiet resolution needs, gathered so the call reads as one
/// question rather than eight positional arguments.
struct QuietQuery<'a> {
    /// The session's events, oldest first.
    events: &'a [Event],
    /// The pane's `@ae_agent` display ref.
    agent: &'a str,
    /// The pane's roster slot — half of the routing key the ONE relevance
    /// owner judges actor and target by.
    slot: &'a str,
    /// This cycle's filtered pane hash.
    hash: u64,
    /// The pane's 1-based position in this cycle's traversal, for the budget.
    index: usize,
    /// The pane to re-capture while settling a baseline.
    pane_id: &'a str,
}

/// One pane's quiet question, as the collection site assembles it.
fn quiet_query<'a>(
    events: &'a [Event],
    agent: &'a str,
    slot: &'a str,
    hash: u64,
    index: usize,
    pane_id: &'a str,
) -> QuietQuery<'a> {
    QuietQuery {
        events,
        agent,
        slot,
        hash,
        index,
        pane_id,
    }
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
    ) -> crate::Result<()> {
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
        panes: &[crate::tmux::WatchPane],
        now: i64,
        err: &mut impl Write,
    ) -> crate::Result<()> {
        if requested && self.knobs.quota_aware {
            self.refresh_quota(carry, panes, now, err)?;
        }
        Ok(())
    }

    /// The same session-relative quota inputs as the generated `quota` helper.
    fn quota_observation(
        &self,
        now: i64,
    ) -> Result<crate::quota::Observation, crate::config::ConfigError> {
        let root = crate::state_root().or_else(|| {
            self.meta_dir
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        });
        let global = root
            .as_deref()
            .map(|root| crate::doors::config_file(crate::shape::current(), root));
        let home = crate::doors::home();
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
    ) -> crate::Result<()> {
        for action in actions {
            match action {
                QuotaAction::Dropped { recipient, summary } => {
                    self.emit("quota-advisory-dropped", &recipient, &summary, err)?;
                }
                QuotaAction::Deliver(pending) => {
                    let text = pending.advisory.render(self.meta_dir, now);
                    let delivery =
                        self.deliver(&pending.recipient.agent, &text, "quota-advisory", &text);
                    if let Some(QuotaAction::Dropped { recipient, summary }) = carry
                        .record_delivery(&pending, quota_delivery(&delivery), self.meta_dir, now)
                    {
                        self.emit("quota-advisory-dropped", &recipient, &summary, err)?;
                    }
                }
                QuotaAction::Ask(ask) => {
                    // The SAME sender as the advisory — one delivery owner, so
                    // the seat reads one marker, written by `provenance`.
                    let text = ask.advisory.checkpoint_ask(self.meta_dir);
                    let delivery =
                        self.deliver(&ask.recipient.agent, &text, "quota-checkpoint", &text);
                    if let Some(QuotaAction::Dropped { recipient, summary }) =
                        carry.record_ask_delivery(&ask, quota_delivery(&delivery), self.meta_dir)
                    {
                        self.emit("quota-checkpoint-dropped", &recipient, &summary, err)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn refresh_quota(
        &self,
        carry: &mut QuotaCarry,
        panes: &[crate::tmux::WatchPane],
        now: i64,
        err: &mut impl Write,
    ) -> crate::Result<()> {
        match self.quota_observation(now) {
            Ok(observation) => {
                let recipients = quota_recipients(&self.roster, self.lead_pair);
                // TWO sets, never one: the advisory's lead pair above, and
                // every seat proven to sit on a readable scope below.
                let candidates = quota_ask_candidates(&self.roster, panes);
                let actions = carry.reconcile_with_candidates(
                    &observation,
                    &recipients,
                    &candidates,
                    self.meta_dir,
                );
                let summary = Self::observed_summary(&observation, actions.len());
                // LAST: the trace attests a COMPLETED refresh — observation,
                // booking, and delivery all done — so awaiting the line
                // proves everything the test subsequently reads has already
                // happened. Tracing before delivery would attest booking
                // while the receipt was still absent.
                self.apply_quota_actions(carry, actions, now, err)?;
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

    /// Read the fleet through [`crate::current_world`], enrich it through the
    /// same card collector as `ae brief --all`, then touch the watchdog-owned
    /// checkpoint without advancing the last-delivered hash.
    fn overview(&self, now: i64, err: &mut impl Write) -> Option<OverviewReading> {
        let Some(root) = crate::state_root() else {
            let _ = writeln!(
                err,
                "ae: watchdog: no state root for fleet overview — skipped"
            );
            return None;
        };
        let (_, world) = crate::current_world(&root);
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
        pin: Option<&str>,
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
            slot,
            &entry.name,
            tool,
            capture,
            launch_id,
            pin,
        );
    }

    /// This seat's profile model pin for this cycle, or `None` when the tool's
    /// live model is not one ae observes at all.
    ///
    /// Read ONCE per pane per cycle: the durable drift row and the picker's
    /// drift mark are two questions about the same pin.
    fn seat_pin(&self, slot: &str, tool: crate::tool::ToolKind) -> Option<String> {
        if !tool.adapter().model.observes() {
            return None;
        }
        let profile = self
            .roster
            .iter()
            .find(|entry| entry.slot == slot)?
            .profile
            .as_deref()?;
        self.profile_model_pin(profile, tool)
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
    fn profile_model_pin(&self, profile: &str, tool: crate::tool::ToolKind) -> Option<String> {
        let root = crate::state_root();
        let global = root
            .as_deref()
            .map(|root| crate::doors::config_file(crate::shape::current(), root));
        crate::launch_cmd::profile_model_pin(
            global.as_deref(),
            self.local_config.as_deref(),
            profile,
            tool,
        )
    }

    /// One quota-cadence pass: the vendor-quota observation and advisory
    /// booking, and only when aware. `quota = off` WINS over
    /// `quota_every_secs`: no quota read is even attempted unaware, while the
    /// cadence's due counter still advances.
    fn run_quota_cadence(
        &self,
        carry: &mut QuotaCarry,
        panes: &[crate::tmux::WatchPane],
        now: i64,
        err: &mut impl Write,
    ) -> crate::Result<()> {
        if quota_observation_due(carry, &self.knobs) {
            if self.knobs.quota_aware {
                self.refresh_quota(carry, panes, now, err)?;
            } else {
                Self::trace_quota("skipped");
            }
        }
        Ok(())
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
    fn run(&self, carry: &mut Carry, err: &mut impl Write) -> crate::Result<()> {
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
        let table = procs::snapshot();
        let events = read_events(self.meta_dir);
        let now = Timestamp::now().epoch();
        let overview = if self.meta_agent && self.knobs.sweep.enabled() {
            self.overview(now, err)
        } else {
            None
        };
        // ONE cadence, one counter: `quota_every_secs` paces the periodic
        // quota pass, and zero disables it.
        self.run_quota_cadence(&mut carry.quota, &observed, now, err)?;

        let seats = held_seats(&observed, table.as_deref(), &|slot| self.agent_bin(slot));
        let outstanding = crate::session::Outstanding::read(&events, self.session, &seats);

        carry.quiet.begin();
        let mut index = 0_usize;
        let mut live: Vec<String> = Vec::new();
        let mut counts = Counts::default();
        let mut by_slot: Vec<(String, Verdict)> = Vec::new();
        let mut by_agent: Vec<AgentObservation> = Vec::new();
        let mut by_pane: Vec<PaneMark> = Vec::new();
        // Cycle-wide: any seat leaving the limit this sweep requests ONE pass.
        let mut quota_refresh = false;

        for pane in &observed {
            let Some(agent) = pane.agent.as_deref().filter(|name| !name.is_empty()) else {
                continue;
            };
            if NON_AGENT_PANES.contains(&agent) {
                continue;
            }
            index += 1;
            live.push(agent.to_owned());
            let slot = pane.slot.clone().unwrap_or_default();
            let agent_bin = self.agent_bin(&slot);
            let tool =
                crate::tool::ToolKind::from_binary_name(agent_bin.as_deref().unwrap_or_default());

            let (capture, capture_ok) = capture_pane(self.server, &pane.pane_id);
            let hash = quiet_hash(&capture);
            // Model drift rides the SAME capture, under the seat's launch
            // guard, and shares this cycle's one reading of the profile pin
            // with the picker's own drift mark.
            let pin = self.seat_pin(&slot, tool);
            self.note_model(&capture, tool, &slot, pin.as_deref());
            let throttle = throttle_class(&capture, agent_bin.as_deref().unwrap_or_default());
            let throttle_quota = self.throttle_quota(&carry.quota, &slot, now, throttle.is_some());
            // ONE process-tree reading: the dead verdict and the unknown-snapshot
            // counter are two questions about the same answer.
            let descendancy = descendancy_of(table.as_deref(), pane.pane_pid, agent_bin.as_deref());
            let identity = quiet_hash(&format!("{slot}\n{agent}"));
            let carried = entry_mut(&mut carry.panes, &pane.pane_id);
            restore_idle(carried, &pane.observed, identity);
            let seen = Observation {
                now_epoch: now,
                hash,
                harness: self.harness_observation(&capture, tool, &events, &slot, agent),
                identity,
                is_dead: classify_dead(&pane.current_command, descendancy),
                throttle,
                capture_ok,
                human_prompt: crate::watchdog::human_prompt_class(
                    &capture,
                    agent_bin.as_deref().unwrap_or_default(),
                    tool.adapter().input.composed,
                ),
                throttle_quota,
                quiet: self.resolve_quiet(
                    &quiet_query(&events, agent, &slot, hash, index, &pane.pane_id),
                    carried,
                    &mut carry.quiet,
                ),
                descendancy,
                last_actor_event_age_secs: last_actor_event_age(
                    &events,
                    self.session,
                    &slot,
                    agent,
                    now,
                ),
                // Decided HERE, once, and the type carries the answer: a pane
                // that is not the orchestrator main gets `None` and no sweep
                // branch can reach it.
                sweep: self.sweep_observation(&slot, agent, &events, overview.as_ref(), now),
                own_work: outstanding.of(Seat::new(self.session, &slot, agent)),
            };
            let acting = Acting {
                agent,
                slot: &slot,
                seen: &seen,
                events: &events,
                overview: seen.sweep.as_ref().and(overview.as_ref()),
            };
            let booked = account(carried, &seen, &self.knobs);
            *carried = booked.next;
            self.apply_booked(&booked.effects, &acting, carried, &mut quota_refresh, err)?;
            counts.record(booked.verdict);
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
            by_slot.push((slot.clone(), booked.verdict));
            by_agent.push(AgentObservation {
                slot,
                pane: pane.pane_id.clone(),
                verdict: booked.verdict,
                identity: seat_identity,
            });
            by_pane.push(PaneMark {
                pane: pane.pane_id.clone(),
                verdict: booked.verdict,
                observed: observed_option(seen.harness.frame, carried),
            });
        }
        carry.quiet.end(index);
        self.refresh_after_limit_release(quota_refresh, &mut carry.quota, &observed, now, err)?;
        self.retry_briefs(&mut carry.brief_cursor, &by_slot, now, err)?;
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
        by_slot: &[(String, Verdict)],
        by_agent: &[AgentObservation],
        by_pane: &[PaneMark],
        live: &[String],
        now_epoch: i64,
        table: Option<&[procs::Proc]>,
        err: &mut impl Write,
    ) -> crate::Result<()> {
        // The LOOK is re-read every cycle, so flipping `@ae_icons` on a live
        // session takes effect on the next one rather than at the next launch.
        // A read that did not answer keeps the last one and RECONCILES NOTHING:
        // rewriting a layout from a look ae did not actually read is how a
        // theme-off session gets repainted by the daemon that is meant to
        // respect it.
        let read = self.look();
        let Some(look) = read.or(carry.look) else {
            // No look has EVER answered on this session. Publishing now would
            // mean choosing colours ae was never told to use, and restamping
            // every window in them; the next cycle asks again.
            return Ok(());
        };
        carry.look = Some(look);
        if read.is_some() {
            self.reconcile_look(&look, &mut carry.motion);
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
        let root = crate::state_root();
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
    fn reconcile_look(&self, look: &Look, motion: &mut MotionState) {
        let Some(session_id) = transport::observe_session_id(self.server, self.session) else {
            return;
        };
        let stamped =
            transport::observe_session_option(self.server, self.session, theme::LOOK_STAMP_OPTION)
                .unwrap_or_default();
        if stamped == look.stamp() {
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
                    &session_id,
                    &option,
                    &value,
                );
            }
        } else {
            for option in theme::LAYOUT_OPTIONS {
                ok &=
                    transport::clear_option(self.server, OptionScope::Session, &session_id, option);
            }
        }
        ok &= self.reconcile_windows(look);
        if !ok {
            return;
        }
        let _ = transport::publish_option(
            self.server,
            OptionScope::Session,
            &session_id,
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

    /// The look this session is drawn in, as its own options declare it.
    fn look(&self) -> Option<Look> {
        let read = transport::observe_look(self.server, self.session)?;
        Some(Look::read(
            &read.icons,
            &read.palette,
            &read.drawn,
            &read.motion,
        ))
    }

    /// Publish this cycle's verdicts as tmux user options.
    fn publish(&self, published: &Published<'_>, carry: &mut Carry, table: Option<&[procs::Proc]>) {
        let Some(session_id) = transport::observe_session_id(self.server, self.session) else {
            return;
        };
        for option in [theme::MENU_OPEN_OPTION, theme::SETTINGS_OPEN_OPTION] {
            if let Some(opened) =
                transport::observe_session_option(self.server, self.session, option)
                && menu_open_expired(&opened, Timestamp::now().epoch(), self.knobs.interval_secs)
            {
                let _ =
                    transport::clear_option(self.server, OptionScope::Session, &session_id, option);
            }
        }
        let look = published.look;
        let set = |name: &str, value: &str| {
            let _ = transport::publish_option(
                self.server,
                OptionScope::Session,
                &session_id,
                name,
                value,
            );
        };
        set(tmux::WATCHDOG_STATUS_OPTION, &watch_segment(published));
        // THE SESSION'S OWN ATTENTION, published as three facts: the glyph it
        // draws with, the rank another session's strip sorts on, and the style
        // its name segment is drawn in. Any session on this server can read
        // them, which is what makes the strip one tmux call rather than a walk
        // of every session's state.
        set(
            theme::ATTENTION_GLYPH_OPTION,
            published.attention.glyph(look.icons),
        );
        set(
            theme::ATTENTION_RANK_OPTION,
            &published.attention.rank().to_string(),
        );
        set(
            theme::ATTENTION_STYLE_OPTION,
            &theme::attention_style(&look.palette, published.attention),
        );
        match published.agents {
            Some(agents) => set(theme::AGENTS_OPTION, agents),
            None => {
                let _ = transport::clear_option(
                    self.server,
                    OptionScope::Session,
                    &session_id,
                    theme::AGENTS_OPTION,
                );
            }
        }
        // The GOAL, ahead of the path on the right: what this session is for
        // outranks where its files are, and the path is the fact the reader's
        // own shell prompt already carries.
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
            None => {
                let _ = transport::clear_option(
                    self.server,
                    OptionScope::Session,
                    &session_id,
                    theme::GOAL_OPTION,
                );
            }
        }
        // The core THIS daemon runs on. An upgrade restarts the daemon on the
        // new core, so the value moves with the install and never with a launch.
        set(theme::VERSION_OPTION, &crate::version_line());
        self.publish_fleet(look, carry, table);
        self.publish_windows(published, &mut carry.motion);
    }

    /// The fleet strip: every ae session on THIS server, as each one's own
    /// watchdog described itself — plus the strip of every same-server ae
    /// session that has no live watchdog to describe it.
    ///
    /// ONE listing answers both, and the adoption scan hangs off it: this is
    /// the read that happens every cycle whatever the ticker is doing, so it is
    /// the only place a daemon with no targets — and therefore no tick of its
    /// own — can ever discover its first one.
    fn publish_fleet(&self, look: &Look, carry: &mut Carry, table: Option<&[procs::Proc]>) {
        let Some(sessions) = transport::observe_fleet_listing(self.server) else {
            return;
        };
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
        if writes.is_empty() || transport::publish_options(self.server, &writes) {
            carry.motion = next;
            // Committed only on a landed batch: a write-on-change memory
            // advanced past a write tmux refused would never retry it.
            carry.adoption = adoption;
        }
    }

    /// Per-window marks and per-pane state, grouped from the SAME per-pane
    /// verdicts in pane order — and the theme, restamped on any window that
    /// appeared since the launch dressed the session.
    fn publish_windows(&self, published: &Published<'_>, motion: &mut MotionState) {
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
            let _ = transport::publish_option(
                self.server,
                OptionScope::Pane,
                &pane.pane_id,
                theme::AGENT_LABEL_OPTION,
                &theme::agent_label(agent),
            );
            if NON_AGENT_PANES.contains(&agent) {
                continue;
            }
            let found = published
                .by_pane
                .iter()
                .find(|entry| entry.pane == pane.pane_id);
            let mark = found.map_or(Mark::Idle, |entry| entry.verdict.mark());
            agents.push((theme::agent_label(agent), mark));
            self.publish_pane_state(&pane.pane_id, found, look);
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
            let _ = if line.is_empty() {
                transport::clear_option(
                    self.server,
                    OptionScope::Window,
                    window_id,
                    theme::WINDOW_AGENTS_OPTION,
                )
            } else {
                transport::publish_option(
                    self.server,
                    OptionScope::Window,
                    window_id,
                    theme::WINDOW_AGENTS_OPTION,
                    &line,
                )
            };
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

    /// One pane's border state: its mark, and the word behind it.
    fn publish_pane_state(&self, pane: &str, found: Option<&PaneMark>, look: &Look) {
        let Some(entry) = found else {
            return;
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
        let writes = [
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
        ];
        let _ = transport::publish_options(self.server, &writes);
    }

    /// The recorded binary for a slot, or `None` when the roster has none —
    /// which the dead check must read as UNKNOWN, never as absent.
    fn agent_bin(&self, slot: &str) -> Option<String> {
        self.roster
            .iter()
            .find(|entry| entry.slot == slot)
            .and_then(|entry| entry.binary.clone())
            .filter(|binary| !binary.is_empty())
    }

    /// The RESOLVED quiet suppression for one pane.
    fn resolve_quiet(
        &self,
        query: &QuietQuery<'_>,
        state: &mut PaneState,
        quiet_cycle: &mut QuietCycle,
    ) -> Option<QuietKind> {
        let relevant = latest_relevant_event(query.events, self.session, query.slot, query.agent)?;
        let kind = quiet_reason(&relevant)?;
        if kind == QuietKind::Done {
            return Some(kind);
        }
        let event = relevant.event;
        let key = declaration_key(event);
        let armed = state
            .quiet_base
            .as_ref()
            .map(|(armed_key, armed_hash, changed_streak)| {
                (armed_key.as_str(), *armed_hash, *changed_streak)
            });
        match quiet_pane_decision(query.hash, armed, &key) {
            QuietPane::Hold => {
                state.quiet_base = Some((key, query.hash, 0));
                Some(kind)
            }
            QuietPane::Yield => None,
            QuietPane::Rearm(hash) => {
                let changed_streak = state
                    .quiet_base
                    .as_ref()
                    .map_or(1, |(_, _, streak)| streak.saturating_add(1));
                state.quiet_base = Some((key, hash, changed_streak));
                Some(kind)
            }
            QuietPane::Arm => {
                if !quiet_cycle.step(query.index) {
                    return None; // budget spent this cycle; try again next one
                }
                let samples = self.settle(query.pane_id);
                let borrowed: Vec<&str> = samples.iter().map(String::as_str).collect();
                let settled = quiet_stabilize(&borrowed, self.knobs.quiet_tries)?;
                state.quiet_base = Some((key, settled, 0));
                Some(kind)
            }
        }
    }

    /// The samples a baseline must settle across: one capture, then a beat and
    /// another, up to `quiet_tries` times.
    fn settle(&self, pane_id: &str) -> Vec<String> {
        let mut samples = Vec::new();
        let Some(first) = transport::capture_pane(self.server, pane_id) else {
            return samples;
        };
        samples.push(first);
        for _ in 0..self.knobs.quiet_tries {
            std::thread::sleep(Duration::from_millis(self.knobs.quiet_beat_ms));
            let Some(next) = transport::capture_pane(self.server, pane_id) else {
                return samples;
            };
            samples.push(next);
        }
        samples
    }

    /// Perform one effect.
    fn apply(
        &self,
        effect: &Effect,
        on: &Acting<'_>,
        state: &mut PaneState,
        err: &mut impl Write,
    ) -> crate::Result<()> {
        let agent = on.agent;
        match effect {
            Effect::Emit { action, summary } => self.emit(action, agent, summary, err),
            Effect::Notify(text) => {
                self.notify(agent, text);
                Ok(())
            }
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
                let text = match (idle_age.is_some(), waiting.as_deref()) {
                    (true, Some(reason)) => {
                        idle_nudge_text_waiting(self.goal.as_deref(), self.meta_dir, reason)
                    }
                    (true, None) => idle_nudge_text(self.goal.as_deref(), self.meta_dir),
                    (false, _) => nudge_text(self.goal.as_deref(), self.meta_dir),
                };
                let summary = match (idle_age.is_some(), waiting.as_deref()) {
                    (true, Some(reason)) => format!("{display}, {reason}"),
                    (true, None) => format!("{display}, harness waiting at input"),
                    (false, _) => format!("{display}, no recent ae activity"),
                };
                let delivered = self.deliver(agent, &text, "nudge", &summary).code == Some(0);
                for effect in record_nudge(state, delivered, &self.knobs, &display) {
                    self.apply(&effect, on, state, err)?;
                }
                Ok(())
            }
        }
    }

    /// Deliver one sweep prompt and book what happened.
    fn sweep_nudge(
        &self,
        on: &Acting<'_>,
        state: &mut PaneState,
        err: &mut impl Write,
    ) -> crate::Result<()> {
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
            .deliver(on.agent, &body, "nudge", "fleet overview changed")
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
    fn deliver(&self, agent: &str, text: &str, action: &str, summary: &str) -> transport::Delivery {
        transport::deliver(
            self.helper.path(),
            agent,
            text,
            false,
            &[
                ("AE_SENDER_OVERRIDE", ACTOR),
                ("_AE_EVENT_ACTION", action),
                ("_AE_EVENT_SUMMARY", summary),
            ],
        )
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
        verdicts: &[(String, Verdict)],
        now: i64,
        err: &mut impl Write,
    ) -> crate::Result<()> {
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
                    self.set_damaged_aside(&entry.name, &entry.slot, damaged, now, err)?;
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

    /// Move a record ae will never read aside, once, and say so in the ledger.
    ///
    /// Only PERMANENT damage is destroyed. A read that merely failed may be a
    /// passing `EMFILE`, and the next cycle may read it perfectly well, so it
    /// is left exactly where it is until its own mtime proves it was never
    /// going to be read.
    fn set_damaged_aside(
        &self,
        name: &str,
        slot: &str,
        damaged: crate::brief_retry::Damaged,
        now: i64,
        err: &mut impl Write,
    ) -> crate::Result<()> {
        if !crate::brief_retry::should_destroy(&damaged, now) {
            writeln!(
                err,
                "ae: watchdog: {name}'s brief record could not be read ({}) — left alone, it may read next cycle",
                damaged.kind
            )?;
            return Ok(());
        }
        match crate::brief_retry::mark_damaged(self.meta_dir, slot, now) {
            Ok(moved) => {
                self.emit(
                    crate::brief_retry::GAVE_UP_ACTION,
                    name,
                    &format!(
                        "brief record damaged ({}) — set aside at {}; the brief is preserved at undelivered.{name}.txt",
                        damaged.kind,
                        moved.display()
                    ),
                    err,
                )?;
            }
            // Never a loop: it says so once per cycle and changes nothing.
            Err(why) => writeln!(err, "ae: watchdog: {why}")?,
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
    ) -> crate::Result<()> {
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
    ) -> crate::Result<()> {
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

/// A completed verdict cycle is a foreground-safe scheduling edge. The 100 ms
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
    by_slot: &[(String, Verdict)],
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
    /// The four v1 fields and the client, with nothing observed. The client
    /// is the cheapest cell on the row and the one that never drops.
    ClientOnly,
    /// Exactly today's v1 bytes.
    ///
    /// The LAST rung before nothing, and the reason it exists: v2 costs four
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
            Self::Full | Self::NoEffort | Self::ClientOnly => "v2",
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
fn read_events(meta_dir: &Path) -> Vec<Event> {
    let bytes = store::open(meta_dir).container();
    crate::event_text::read_lines(&bytes)
        .into_iter()
        .filter_map(|line| Event::parse_line(&String::from_utf8_lossy(line)).ok())
        .collect()
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
mod tests {
    use super::{
        ACTOR, ADOPTION_TICK, Adopted, Adoption, AdoptionBackend, AgentObservation, Carry,
        Continuation, Cycle, DETACHED_MOTION_TICK, Effect, FactRung, HOLD_MAX_CYCLES,
        HarnessObservation, Journal, Knobs, MissingState, MotionState, MotionVerdict, Observation,
        OverviewReading, PaneState, PendingAdvisory, PendingAsk, QuietCycle, QuietQuery,
        QuotaAction, QuotaCarry, QuotaDelivery, QuotaLevel, QuotaRecipient, Rebind,
        ResolveIdentity, SeatIdentity, SendHelper, TickerMode, UNKNOWN_ALERT_CYCLES, Verdict,
        WatchdogPresence, account, adopt_server, adoption_due, adoption_from, adoption_writes,
        age_secs, agents_fact, bar_glyph, continuation, deferred, entry_mut, fact_at, fleet_rows,
        held_seats, holds_seat, idle_nudge_seconds, idle_nudge_text, idle_nudge_text_waiting,
        is_meta_agent, last_actor_event_age, last_done_event_at, last_working_declaration_at,
        motion_cadence, motion_failure, motion_observation_due, motion_publish_failure,
        motion_ticker_enabled, nudge_text, observed_option, proven_ownership, quota_ask_candidates,
        quota_delivery, quota_observation_due, quota_recipients, quota_seconds, read_events,
        rebind, record_nudge, restore_idle, session_name, slot_latched, slot_mark, stale_display,
        static_observe_cadence, sweep_effects, sweep_seconds, system_time_from_epoch,
        throttle_quota_line, ticker_mode, window_agents_line,
    };
    use super::{Look, Mark, PaneMark, session_mark};
    use crate::events::Event;
    use crate::inventory::ServerId;
    use crate::meta::{
        Meta, RecordedClient, RecordedConfigHome, RecordedConfigHomeBase, RosterEntry, Selector,
    };
    use crate::procs::Descendancy;
    use crate::session::OwnWork;
    use crate::tmux::StopProbe;
    use crate::watchdog::{
        QuietKind, SweepAlert, SweepEffect, SweepObservation, SweepVerdict, Throttle, WedgeDetail,
        declaration_key, quiet_filter, quiet_hash,
    };
    use std::io::ErrorKind;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn prompt() -> crate::watchdog::HumanPrompt {
        crate::watchdog::HumanPrompt {
            question: "Do you trust the contents of this project?".to_owned(),
            keys: "↑/↓ Navigate · enter Confirm".to_owned(),
        }
    }

    fn on_a_modal() -> Observation {
        Observation {
            human_prompt: Some(prompt()),
            ..seen()
        }
    }

    fn actions(effects: &[Effect]) -> Vec<&str> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Emit { action, .. } => Some(*action),
                _ => None,
            })
            .collect()
    }

    /// EXACTLY two cycles, and the episode is named EXACTLY once. One cycle is
    /// a redraw; naming every cycle would page the human on a loop.
    #[test]
    fn a_human_only_prompt_is_named_on_its_second_cycle_and_only_once() {
        let knobs = Knobs::default();
        let first = account(&PaneState::default(), &on_a_modal(), &knobs);
        assert_ne!(first.verdict, Verdict::HumanPrompt, "one cycle is a redraw");
        assert!(actions(&first.effects).is_empty(), "nothing said yet");

        let second = account(&first.next, &on_a_modal(), &knobs);
        assert_eq!(second.verdict, Verdict::HumanPrompt);
        assert_eq!(actions(&second.effects), ["human-prompt"], "named once");

        let third = account(&second.next, &on_a_modal(), &knobs);
        assert_eq!(third.verdict, Verdict::HumanPrompt, "still waiting");
        assert!(actions(&third.effects).is_empty(), "not named again");
    }

    /// The Notify line and the event both say WHICH question and WHAT to press.
    /// A verdict the human cannot act on is not news.
    #[test]
    fn naming_a_human_only_prompt_says_the_question_and_the_keys() {
        let knobs = Knobs::default();
        let first = account(&PaneState::default(), &on_a_modal(), &knobs);
        let named = account(&first.next, &on_a_modal(), &knobs);
        let said = format!("{:?}", named.effects);
        assert!(
            said.contains("Do you trust the contents of this project?"),
            "{said}"
        );
        assert!(said.contains("↑/↓ Navigate · enter Confirm"), "{said}");
        assert!(
            named
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::Notify(_))),
            "the human is told: {said}"
        );
    }

    /// The mark is SET while it waits and UNSET when it goes, and the clear is
    /// emitted exactly once — not on every quiet cycle afterwards.
    #[test]
    fn a_cleared_human_prompt_retracts_the_mark_once() {
        let knobs = Knobs::default();
        let first = account(&PaneState::default(), &on_a_modal(), &knobs);
        let named = account(&first.next, &on_a_modal(), &knobs);
        assert_eq!(named.verdict.mark(), crate::theme::Mark::NeedsYou);

        let gone = account(&named.next, &seen(), &knobs);
        assert_eq!(actions(&gone.effects), ["human-prompt-cleared"]);
        assert_ne!(gone.verdict, Verdict::HumanPrompt);
        assert_ne!(gone.verdict.mark(), crate::theme::Mark::NeedsYou);

        let still_gone = account(&gone.next, &seen(), &knobs);
        assert!(actions(&still_gone.effects).is_empty(), "retracted once");
    }

    /// A FAILED capture is an absence of evidence, not evidence of absence —
    /// branch 5b's rule. Clearing on it would flap the latch against a live
    /// modal nobody has touched.
    #[test]
    fn a_failed_capture_neither_clears_nor_relatches_a_human_prompt() {
        let knobs = Knobs::default();
        let first = account(&PaneState::default(), &on_a_modal(), &knobs);
        let named = account(&first.next, &on_a_modal(), &knobs);

        let blind = account(
            &named.next,
            &Observation {
                capture_ok: false,
                ..seen()
            },
            &knobs,
        );
        assert!(actions(&blind.effects).is_empty(), "it says nothing");
        assert_eq!(
            blind.next.human_prompt_streak, named.next.human_prompt_streak,
            "the streak is untouched, so the next real read decides"
        );
    }

    /// The seat is SILENT by nature — that is what a modal does — so `Stale`
    /// must not win. The carry here is a seat that WOULD read stale: same hash
    /// as last cycle, no motion for ages, no actor event for ages. With the
    /// branch where it belongs the seat is NAMED; move it below the harness
    /// frames and this same seat goes quietly stale on the bar, which is the
    /// feature failing silently.
    #[test]
    fn a_silent_seat_on_a_modal_is_named_rather_than_called_stale() {
        let knobs = Knobs::default();
        // A reading branch 9 answers STALE for on its own — that is the whole
        // point: the modal must be named over a verdict that would otherwise
        // win, not merely over silence nothing decides.
        let stale_reading = HarnessObservation {
            human_draft: true,
            durable_stale: true,
            ..seen().harness
        };
        let silent = Observation {
            harness: stale_reading,
            last_actor_event_age_secs: knobs.stale_secs * 4,
            ..on_a_modal()
        };
        // One cycle of the modal already behind it, so THIS cycle names it.
        let prior = PaneState {
            identity: Some(silent.identity),
            human_prompt_streak: 1,
            ..PaneState::default()
        };
        assert_eq!(
            account(&prior, &silent, &knobs).verdict,
            Verdict::HumanPrompt
        );
        // The control: the identical seat with no modal really does read stale,
        // so the pin above is about PRECEDENCE and not about the fixture.
        let no_modal = Observation {
            human_prompt: None,
            ..silent
        };
        assert_eq!(account(&prior, &no_modal, &knobs).verdict, Verdict::Stale);
    }

    /// CHANNEL TWO, pinned without a pane. The daemon must not spend its one
    /// delivery a cycle on a seat it has just judged to be waiting on a human,
    /// and a slot it did not judge at all is NOT latched — absence of a verdict
    /// is not a verdict, and channel one still asks the pane for itself.
    #[test]
    fn the_retry_skips_exactly_the_slots_this_cycle_judged_to_be_on_a_prompt() {
        let judged = [
            ("spawned.1".to_owned(), Verdict::HumanPrompt),
            ("spawned.2".to_owned(), Verdict::Idle),
            ("worker.0".to_owned(), Verdict::Dead),
        ];
        assert!(slot_latched(&judged, "spawned.1"));
        assert!(
            !slot_latched(&judged, "spawned.2"),
            "an idle seat is retried"
        );
        assert!(
            !slot_latched(&judged, "worker.0"),
            "a dead seat is not this"
        );
        assert!(
            !slot_latched(&judged, "main"),
            "an unjudged slot is not latched"
        );
        assert!(!slot_latched(&[], "spawned.1"), "no verdicts, no latch");
    }

    /// DEAD outranks it. A pane whose agent is gone may still be showing the
    /// modal it died on; "this will never move again" is the worse news.
    #[test]
    fn a_dead_seat_showing_a_modal_is_reported_dead_and_not_named() {
        let knobs = Knobs::default();
        let dead = Observation {
            is_dead: true,
            ..on_a_modal()
        };
        let first = account(&PaneState::default(), &dead, &knobs);
        let second = account(&first.next, &dead, &knobs);
        assert_eq!(second.verdict, Verdict::Dead);
        assert!(
            !actions(&second.effects).contains(&"human-prompt"),
            "a dead seat is not paged about a prompt: {:?}",
            second.effects
        );
    }

    /// The detector path NEVER sends a key — invariant 3, and the reason this
    /// feature is safe to run against a modal at all. The types already say so
    /// (none of these takes a server or a pane), so this reads the SOURCE: the
    /// day one of them gains a handle, this is what fails.
    #[test]
    fn nothing_in_the_human_prompt_path_can_reach_a_pane() {
        #[expect(
            clippy::disallowed_methods,
            reason = "a source scan in TEST code; tests/it/phase3.rs inventories \
                      PRODUCT lines only"
        )]
        fn source(file: &str) -> String {
            std::fs::read_to_string(file).unwrap_or_else(|why| panic!("{file}: {why}"))
        }
        let detector = source("src/watchdog.rs");
        let daemon = source("src/watchdog_daemon.rs");
        let region = |text: &str, from: &str, to: &str| {
            let start = text
                .find(from)
                .unwrap_or_else(|| panic!("{from} should exist"));
            let rest = &text[start..];
            let end = rest.find(to).unwrap_or(rest.len());
            rest[..end].to_owned()
        };
        let paths = [
            region(&detector, "pub fn human_prompt_class", "\n/// "),
            region(&daemon, "fn book_human_prompt", "\nfn book_limit"),
            region(&daemon, "// 5c. The human-prompt latch", "// 6. A quiet"),
        ];
        for path in &paths {
            for reach in ["transport::", "send_key", "capture_pane", "ServerId"] {
                assert!(
                    !path.contains(reach),
                    "the human-prompt path must not reach a pane, found {reach} in:\n{path}"
                );
            }
        }
    }

    /// A DECLARED quiet state outranks the modal: branch 6 returns above 8.5.
    /// An agent that said `done` is not news because its pane draws a menu.
    #[test]
    fn a_declared_quiet_seat_on_a_modal_stays_quiet_and_is_never_named() {
        let knobs = Knobs::default();
        let quiet = Observation {
            quiet: Some(crate::watchdog::QuietKind::Done),
            ..on_a_modal()
        };
        let first = account(&PaneState::default(), &quiet, &knobs);
        let second = account(&first.next, &quiet, &knobs);
        assert_eq!(
            second.verdict,
            Verdict::Quiet(crate::watchdog::QuietKind::Done)
        );
        assert!(
            !actions(&second.effects).contains(&"human-prompt"),
            "a declared-quiet seat is not paged: {:?}",
            second.effects
        );
    }

    /// The two vendor verdicts are WORSE news and keep their rank above it.
    #[test]
    fn a_throttled_or_limited_seat_on_a_modal_keeps_the_vendor_verdict() {
        let knobs = Knobs::default();
        for (throttle, expected) in [
            (crate::watchdog::Throttle::Throttled, Verdict::Throttled),
            (crate::watchdog::Throttle::LimitReached, Verdict::Limit),
        ] {
            let both = Observation {
                throttle: Some(throttle),
                ..on_a_modal()
            };
            let first = account(&PaneState::default(), &both, &knobs);
            let second = account(&first.next, &both, &knobs);
            assert_eq!(second.verdict, expected, "{throttle:?} outranks the modal");
        }
    }

    /// A pane nobody has seen before, with nothing wrong.
    fn seen() -> Observation {
        Observation {
            now_epoch: 10_000,
            hash: 7,
            harness: HarnessObservation {
                frame: crate::harness_state::HarnessState::Unknown,
                human_draft: false,
                durable_stale: false,
                declaration: None,
            },
            identity: 1,
            is_dead: false,
            throttle: None,
            human_prompt: None,
            capture_ok: true,
            throttle_quota: None,
            quiet: None,
            descendancy: Descendancy::Present,
            last_actor_event_age_secs: 0,
            sweep: None,
            own_work: crate::session::OwnWork::default(),
        }
    }

    impl QuotaCarry {
        /// Reconcile with no ask candidates — the ADVISORY path alone.
        ///
        /// Production always calls
        /// [`QuotaCarry::reconcile_with_candidates`]; this shape keeps every
        /// test that only ever cared about advisories saying exactly that, and
        /// booking no ask.
        ///
        /// It lives HERE, in the test module, rather than carrying a
        /// `#[cfg(test)]` beside its sibling: the structural pins of this file
        /// read "production" as everything before the first `#[cfg(test)]`, so
        /// one such attribute up there silently truncates what they inspect.
        fn reconcile(
            &mut self,
            observation: &crate::quota::Observation,
            recipients: &[QuotaRecipient],
            meta_dir: &Path,
        ) -> Vec<QuotaAction> {
            self.reconcile_with_candidates(observation, recipients, &[], meta_dir)
        }
    }

    fn quota_row(
        bucket: &str,
        qualifier: Option<&str>,
        used: &str,
        observed_at: i64,
        status: crate::quota::Status,
    ) -> crate::quota::Row {
        crate::quota::Row {
            bucket: bucket.to_owned(),
            qualifier: qualifier.map(str::to_owned),
            window_minutes: Some(300),
            used_percent: Some(used.to_owned()),
            resets_at: Some(13_600),
            observed_at: Some(observed_at),
            status,
        }
    }

    fn quota_group(
        source: &Path,
        rollout: Option<&str>,
        owner: Option<&str>,
        rows: Vec<crate::quota::Row>,
    ) -> crate::quota::Group {
        quota_scope_group(
            source,
            rollout,
            owner,
            rows,
            None,
            crate::quota::Account::default(),
        )
    }

    fn quota_scope_group(
        source: &Path,
        rollout: Option<&str>,
        owner: Option<&str>,
        rows: Vec<crate::quota::Row>,
        manual_resets: Option<u8>,
        account: crate::quota::Account,
    ) -> crate::quota::Group {
        crate::quota::Group {
            profiles: vec!["sol".to_owned()],
            tool: crate::tool::ToolKind::Codex,
            home: source.parent().map(Path::to_path_buf),
            source: Some(source.to_path_buf()),
            clients: vec!["cx".to_owned()],
            rollout: rollout.map(str::to_owned),
            owner: owner.map(str::to_owned),
            rows,
            hint: None,
            summary: None,
            policy: crate::quota::Policy::for_tests(manual_resets, account),
            notes: Vec::new(),
        }
    }

    /// The reading a test renders an advisory from: one group first row,
    /// judged under that group own policy.
    fn quota_classified(group: &crate::quota::Group) -> crate::quota::Classified {
        crate::quota::Classified::first(quota_reading(group))
    }

    fn quota_reading(group: &crate::quota::Group) -> crate::quota::Reading {
        crate::quota::Reading::of(group.policy.clone(), group.rows[0].clone())
            .expect("a test row states a percentage and a stamp")
    }

    fn quota_observation(groups: Vec<crate::quota::Group>, now: i64) -> crate::quota::Observation {
        crate::quota::Observation {
            rendered: groups.clone(),
            groups,
            home: Some(PathBuf::from("/home/test")),
            now,
        }
    }

    fn quota_recipient(slot: &str, agent: &str) -> QuotaRecipient {
        QuotaRecipient {
            slot: slot.to_owned(),
            agent: agent.to_owned(),
            harness_session: None,
            config_home: RecordedConfigHome::Missing,
            config_home_base: RecordedConfigHomeBase::Missing,
            binary: None,
        }
    }

    fn quota_for(used: &str, observed_at: i64) -> crate::quota::Observation {
        quota_observation(
            vec![quota_group(
                Path::new("/tmp/cx/sessions"),
                Some("018f1f70-7b2c-7000-8000-000000000001"),
                Some("demo:lead"),
                vec![quota_row(
                    "codex",
                    Some("pro"),
                    used,
                    observed_at,
                    crate::quota::Status::Fresh,
                )],
            )],
            10_000,
        )
    }

    fn transition_deliveries(actions: &[QuotaAction]) -> Vec<&PendingAdvisory> {
        actions
            .iter()
            .filter_map(|action| match action {
                QuotaAction::Deliver(pending) => Some(pending.as_ref()),
                QuotaAction::Ask(_) | QuotaAction::Dropped { .. } => None,
            })
            .collect()
    }

    /// The booked CHECKPOINT ASKS of one pass, in booking order.
    fn checkpoint_asks(actions: &[QuotaAction]) -> Vec<&PendingAsk> {
        actions
            .iter()
            .filter_map(|action| match action {
                QuotaAction::Ask(ask) => Some(ask.as_ref()),
                QuotaAction::Deliver(_) | QuotaAction::Dropped { .. } => None,
            })
            .collect()
    }

    /// The agents one pass asked to checkpoint, in booking order.
    fn asked_agents(actions: &[QuotaAction]) -> Vec<String> {
        checkpoint_asks(actions)
            .iter()
            .map(|ask| ask.recipient.agent.clone())
            .collect()
    }

    #[test]
    fn quota_first_silent_and_old_samples_never_advance_state() {
        let recipients = [quota_recipient("main", "lead")];
        let mut carry = QuotaCarry::default();
        assert!(
            carry
                .reconcile(&quota_for("79", 9_900), &recipients, Path::new("/m"))
                .is_empty(),
            "first sample is baseline only"
        );
        assert!(
            carry
                .reconcile(&quota_for("80", 9_900), &recipients, Path::new("/m"))
                .is_empty(),
            "equal observation is ignored"
        );
        assert!(
            carry
                .reconcile(&quota_for("95", 9_899), &recipients, Path::new("/m"))
                .is_empty(),
            "older observation is ignored"
        );
        assert_eq!(carry.tracked[0].classified.level(), QuotaLevel::Headroom);
        assert_eq!(
            transition_deliveries(&carry.reconcile(
                &quota_for("80", 9_901),
                &recipients,
                Path::new("/m")
            ))
            .len(),
            1
        );
    }

    /// A roster seat for the checkpoint fan-out. `rollout` is what makes a
    /// Codex identity provable; a Claude seat does not need one.
    fn ask_entry(
        slot: &str,
        name: &str,
        binary: &str,
        config_home: RecordedConfigHome,
        config_home_base: RecordedConfigHomeBase,
        rollout: Option<&str>,
    ) -> RosterEntry {
        RosterEntry {
            slot: slot.to_owned(),
            name: name.to_owned(),
            profile: Some("sol".to_owned()),
            client: RecordedClient::Missing,
            harness_session: rollout.map(str::to_owned),
            config_home,
            config_home_base,
            binary: Some(binary.to_owned()),
        }
    }

    /// A Codex seat on `/tmp/cx`, the scope [`quota_for`] observes.
    fn codex_seat(slot: &str, name: &str, home: &str) -> RosterEntry {
        ask_entry(
            slot,
            name,
            "codex",
            RecordedConfigHome::Path(PathBuf::from(home)),
            RecordedConfigHomeBase::Missing,
            Some("018f1f70-7b2c-7000-8000-000000000001"),
        )
    }

    /// The scope a set of Codex seats is observed on, built from the seat's own
    /// resolved identity.
    ///
    /// A `Group` carries the CANONICAL source (`scope.source_key`), which is
    /// what [`crate::quota::recorded_identity`] resolves too — so a fixture that
    /// spelled the raw path would compare two different strings and prove
    /// nothing. The existing throttle-line fixtures build their source the same
    /// way.
    fn codex_scope(on: &RosterEntry, used: &str, observed_at: i64) -> crate::quota::Observation {
        let identity = crate::quota::recorded_identity(on).expect("a recorded Codex identity");
        quota_observation(
            vec![quota_group(
                &identity.source,
                Some("018f1f70-7b2c-7000-8000-000000000001"),
                Some("demo:lead"),
                vec![quota_row(
                    "codex",
                    Some("pro"),
                    used,
                    observed_at,
                    crate::quota::Status::Fresh,
                )],
            )],
            10_000,
        )
    }

    /// A live pane carrying one seat's slot. `cmd` decides whether the seat can
    /// be spoken to at all.
    fn ask_pane(slot: Option<&str>, agent: &str, cmd: &str) -> crate::tmux::WatchPane {
        crate::tmux::WatchPane {
            pane_id: format!("%{}", agent.len()),
            slot: slot.map(str::to_owned),
            agent: Some(agent.to_owned()),
            current_command: cmd.to_owned(),
            pane_pid: Some(4242),
            observed: String::new(),
        }
    }

    /// Every seat of `roster` with a live pane, in roster order.
    fn live_panes(roster: &[RosterEntry]) -> Vec<crate::tmux::WatchPane> {
        roster
            .iter()
            .map(|entry| ask_pane(Some(&entry.slot), &entry.name, "node"))
            .collect()
    }

    /// One Claude scope, whose source is the `.claude.json` under `base`.
    fn claude_group(on: &RosterEntry, used: &str, observed_at: i64) -> crate::quota::Group {
        let identity = crate::quota::recorded_identity(on).expect("a recorded Claude identity");
        let mut group = quota_group(
            &identity.source,
            None,
            None,
            vec![quota_row(
                "claude",
                None,
                used,
                observed_at,
                crate::quota::Status::Fresh,
            )],
        );
        group.tool = crate::tool::ToolKind::Claude;
        group
    }

    #[test]
    fn entering_low_asks_once_then_stays_silent_until_it_clears_and_is_entered_again() {
        // The invariant in one sequence: ONE ask per ENTRY into the band. Not
        // one per cycle, not one per step inside the band, and not none after
        // the band clears and is entered again.
        let roster = [codex_seat("main", "lead", "/tmp/cx")];
        let panes = live_panes(&roster);
        let candidates = quota_ask_candidates(&roster, &panes);
        assert_eq!(candidates.len(), 1, "the seat sits on the observed scope");
        let recipients = [quota_recipient("main", "lead")];
        let meta = Path::new("/m");
        let mut carry = QuotaCarry::default();
        let pass = |observation: crate::quota::Observation, carry: &mut QuotaCarry| {
            let actions =
                carry.reconcile_with_candidates(&observation, &recipients, &candidates, meta);
            asked_agents(&actions)
        };

        assert!(
            pass(codex_scope(&roster[0], "79", 9_900), &mut carry).is_empty(),
            "headroom asks nobody"
        );
        assert_eq!(
            pass(codex_scope(&roster[0], "80", 9_901), &mut carry),
            vec!["lead".to_owned()],
            "entering the band asks the seat once"
        );
        // Deliver it, exactly as the cycle would, so the next passes speak only
        // about what they themselves booked.
        let booked = carry.asks[0].clone();
        assert!(
            carry
                .record_ask_delivery(&booked, QuotaDelivery::Delivered, meta)
                .is_none(),
            "a delivered ask is simply forgotten"
        );
        assert!(carry.asks.is_empty(), "nothing is left to re-deliver");

        assert!(
            pass(codex_scope(&roster[0], "96", 9_902), &mut carry).is_empty(),
            "low to critical is still inside the band: no second ask"
        );
        assert_eq!(
            carry.tracked[0].classified.level(),
            QuotaLevel::Critical,
            "the level did move, so silence here is the rule and not a miss"
        );
        assert!(
            pass(codex_scope(&roster[0], "50", 9_903), &mut carry).is_empty(),
            "leaving the band asks nothing"
        );
        assert_eq!(
            pass(codex_scope(&roster[0], "85", 9_904), &mut carry),
            vec!["lead".to_owned()],
            "a band entered again asks again"
        );
    }

    #[test]
    fn the_ask_fans_out_to_every_seat_on_the_scope_and_no_other() {
        // The ask follows the SCOPE, not the lead pair: fixed seats, spawned
        // seats and a seat whose only recorded tool is its binary all qualify,
        // while another config home and another tool do not.
        let roster = [
            codex_seat("main", "lead", "/tmp/cx"),
            codex_seat("worker.0", "colead", "/tmp/cx-other"),
            codex_seat("spawned.0", "helper", "/tmp/cx"),
            ask_entry(
                "worker.1",
                "legacy",
                "codex",
                RecordedConfigHome::Path(PathBuf::from("/tmp/cx")),
                RecordedConfigHomeBase::Missing,
                Some("018f1f70-7b2c-7000-8000-000000000002"),
            ),
            ask_entry(
                "worker.2",
                "painter",
                "claude",
                RecordedConfigHome::Path(PathBuf::from("/tmp/cx")),
                RecordedConfigHomeBase::Missing,
                None,
            ),
        ];
        assert_eq!(
            roster[3].client,
            RecordedClient::Missing,
            "the legacy seat records no client label, so only its binary can \
             say which scope it is on"
        );
        let mut panes = live_panes(&roster);
        // A pane nobody on the roster owns must not become a recipient.
        panes.push(ask_pane(None, "_watchdog", "ae"));

        let candidates = quota_ask_candidates(&roster, &panes);
        let mut carry = QuotaCarry::default();
        let meta = Path::new("/m");
        assert!(
            carry
                .reconcile_with_candidates(
                    &codex_scope(&roster[0], "79", 9_900),
                    &[quota_recipient("main", "lead")],
                    &candidates,
                    meta,
                )
                .is_empty(),
            "the baseline books nothing"
        );
        let actions = carry.reconcile_with_candidates(
            &codex_scope(&roster[0], "81", 9_901),
            &[quota_recipient("main", "lead")],
            &candidates,
            meta,
        );
        assert_eq!(
            asked_agents(&actions),
            vec!["lead".to_owned(), "helper".to_owned(), "legacy".to_owned()],
            "every seat on the scope, and only those"
        );
        assert_eq!(
            transition_deliveries(&actions).len(),
            1,
            "the ADVISORY still goes to its own recipient set alone"
        );
    }

    #[test]
    fn an_implicit_config_home_matches_only_through_its_recorded_base() {
        // An implicit home names no store of its own: the effective HOME that
        // selected it is what identifies the Claude cache. Two seats can share
        // the implicit path and still be on different scopes.
        let roster = [
            ask_entry(
                "main",
                "here",
                "claude",
                RecordedConfigHome::Implicit(PathBuf::from("/tmp/cfg")),
                RecordedConfigHomeBase::Path(PathBuf::from("/tmp/home-a")),
                None,
            ),
            ask_entry(
                "worker.0",
                "elsewhere",
                "claude",
                RecordedConfigHome::Implicit(PathBuf::from("/tmp/cfg")),
                RecordedConfigHomeBase::Path(PathBuf::from("/tmp/home-b")),
                None,
            ),
        ];
        let panes = live_panes(&roster);
        let candidates = quota_ask_candidates(&roster, &panes);
        assert_eq!(candidates.len(), 2, "both seats have a provable identity");

        let meta = Path::new("/m");
        let mut carry = QuotaCarry::default();
        let observe = |used: &str, at: i64| {
            quota_observation(vec![claude_group(&roster[0], used, at)], 10_000)
        };
        assert!(
            carry
                .reconcile_with_candidates(&observe("70", 9_900), &[], &candidates, meta)
                .is_empty(),
            "the baseline books nothing"
        );
        assert_eq!(
            asked_agents(&carry.reconcile_with_candidates(
                &observe("82", 9_901),
                &[],
                &candidates,
                meta
            )),
            vec!["here".to_owned()],
            "only the seat whose recorded base names this cache"
        );
    }

    #[test]
    fn a_first_sight_at_low_asks_while_the_advisory_stays_silent() {
        // A daemon that starts with the scope already inside the band has seats
        // which may never see another transition. They are asked once; the
        // advisory's own first-sight silence is untouched.
        let roster = [codex_seat("main", "lead", "/tmp/cx")];
        let panes = live_panes(&roster);
        let candidates = quota_ask_candidates(&roster, &panes);
        let mut carry = QuotaCarry::default();
        let actions = carry.reconcile_with_candidates(
            &codex_scope(&roster[0], "97", 9_900),
            &[quota_recipient("main", "lead")],
            &candidates,
            Path::new("/m"),
        );
        assert_eq!(
            asked_agents(&actions),
            vec!["lead".to_owned()],
            "first sight inside the band asks"
        );
        assert!(
            transition_deliveries(&actions).is_empty(),
            "and the advisory still reports no transition it never saw"
        );
        assert_eq!(carry.tracked[0].classified.level(), QuotaLevel::Critical);
    }

    #[test]
    fn a_seat_without_a_live_pane_or_a_readable_scope_is_never_asked() {
        // Three fail-closed skips, and the unaware clear beside them.
        let roster = [
            codex_seat("main", "lead", "/tmp/cx"),
            codex_seat("worker.0", "quit", "/tmp/cx"),
            codex_seat("worker.1", "gone", "/tmp/cx"),
            ask_entry(
                "worker.2",
                "unparsed",
                "muse",
                RecordedConfigHome::Path(PathBuf::from("/tmp/cx")),
                RecordedConfigHomeBase::Missing,
                None,
            ),
            codex_seat("worker.3", "fresh", "/tmp/cx"),
        ];
        let panes = vec![
            ask_pane(Some("main"), "lead", "node"),
            // The tool was quit: the pane is back at a shell and reads nothing.
            ask_pane(Some("worker.0"), "quit", "bash"),
            // `worker.1` has no pane at all.
            ask_pane(Some("worker.2"), "unparsed", "node"),
            ask_pane(Some("worker.3"), "fresh", "node"),
        ];
        let mut roster = roster;
        // A Codex seat with no recorded conversation is not a proven identity,
        // the same rule the throttle line applies.
        roster[4].harness_session = None;

        let candidates = quota_ask_candidates(&roster, &panes);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.recipient.agent.clone())
                .collect::<Vec<_>>(),
            vec!["lead".to_owned()],
            "a shell pane, a missing pane, a tool with no quota parser and an \
             unproven identity are all skipped"
        );

        let meta = Path::new("/m");
        let mut carry = QuotaCarry::default();
        let actions = carry.reconcile_with_candidates(
            &codex_scope(&roster[0], "88", 9_900),
            &[],
            &candidates,
            meta,
        );
        assert_eq!(asked_agents(&actions), vec!["lead".to_owned()]);

        // While unaware nothing may stay booked to fire the moment awareness
        // returns: the ask is held quota knowledge like any other.
        assert!(!carry.asks.is_empty(), "there is something to clear");
        carry.clear_held();
        assert!(carry.asks.is_empty(), "an unaware cycle drops booked asks");
    }

    #[test]
    fn a_declaration_that_clears_the_band_lets_its_withdrawal_ask_again() {
        // A policy change carries no vendor clock: it RE-JUDGES the row already
        // held. That reaches the entry edge exactly as a new observation does,
        // so the ask is direction-symmetric with the advisory — declaring a
        // reset clears the band, withdrawing it re-enters, and the seat is
        // asked again because it is genuinely in trouble again.
        let roster = [codex_seat("main", "lead", "/tmp/cx")];
        let panes = live_panes(&roster);
        let candidates = quota_ask_candidates(&roster, &panes);
        let identity = crate::quota::recorded_identity(&roster[0]).expect("a recorded identity");
        let observation = |used: &str, at: i64, resets: Option<u8>, now: i64| {
            quota_observation(
                vec![quota_scope_group(
                    &identity.source,
                    Some("018f1f70-7b2c-7000-8000-000000000001"),
                    Some("demo:lead"),
                    vec![quota_row(
                        "codex",
                        Some("pro"),
                        used,
                        at,
                        crate::quota::Status::Fresh,
                    )],
                    resets,
                    crate::quota::Account::default(),
                )],
                now,
            )
        };
        let meta = Path::new("/m");
        let mut carry = QuotaCarry::default();
        let pass = |carry: &mut QuotaCarry, resets: Option<u8>, now: i64| {
            asked_agents(&carry.reconcile_with_candidates(
                &observation("95", 9_901, resets, now),
                &[],
                &candidates,
                meta,
            ))
        };

        // Enter the band on the observation alone, and clear the booking so the
        // passes that follow speak only about what they themselves booked.
        assert_eq!(
            pass(&mut carry, None, 10_000),
            vec!["lead".to_owned()],
            "first sight inside the band asks"
        );
        let booked = carry.asks[0].clone();
        assert!(
            carry
                .record_ask_delivery(&booked, QuotaDelivery::Delivered, meta)
                .is_none()
        );

        // DECLARE a reset: the same raw row is re-judged into headroom.
        assert!(
            pass(&mut carry, Some(1), 10_001).is_empty(),
            "leaving the band on a declaration asks nothing"
        );
        assert_eq!(
            carry.tracked[0].classified.level(),
            QuotaLevel::Headroom,
            "the declaration really did clear it"
        );
        assert_eq!(
            carry.tracked[0].classified.observed_at(),
            9_901,
            "and the raw clock did not move"
        );

        // WITHDRAW it: the same raw row is Critical again, which is an entry.
        assert_eq!(
            pass(&mut carry, None, 10_002),
            vec!["lead".to_owned()],
            "a withdrawn declaration re-arms the ask, as it re-arms the advisory"
        );
        assert_eq!(carry.tracked[0].classified.level(), QuotaLevel::Critical);
    }

    #[test]
    fn a_still_deferred_ask_survives_the_band_and_is_replaced_not_doubled() {
        // The seat has not read the ask yet — the delivery is still owed. Two
        // things must hold while it waits. Moving deeper into the band, or out
        // of it, must NOT cancel the booking, because cancelling would mean the
        // seat is never asked at all. And entering the band AGAIN must replace
        // that one booking rather than add a second, or one deferral becomes
        // two pastes.
        let roster = [codex_seat("main", "lead", "/tmp/cx")];
        let panes = live_panes(&roster);
        let candidates = quota_ask_candidates(&roster, &panes);
        let meta = Path::new("/m");
        let mut carry = QuotaCarry::default();
        let pass = |carry: &mut QuotaCarry, used: &str, at: i64| {
            asked_agents(&carry.reconcile_with_candidates(
                &codex_scope(&roster[0], used, at),
                &[],
                &candidates,
                meta,
            ))
        };

        assert!(pass(&mut carry, "70", 9_900).is_empty(), "the baseline");
        assert_eq!(pass(&mut carry, "85", 9_901), vec!["lead".to_owned()]);
        assert_eq!(carry.asks.len(), 1, "one booking, still undelivered");
        assert_eq!(carry.asks[0].observed_at, 9_901);

        // DEEPER into the band. Nothing new is asked, and the owed one stands.
        let _ = pass(&mut carry, "96", 9_902);
        assert_eq!(carry.tracked[0].classified.level(), QuotaLevel::Critical);
        assert_eq!(
            carry.asks.len(),
            1,
            "a later transition never cancels an owed ask"
        );
        assert_eq!(
            carry.asks[0].observed_at, 9_901,
            "and never rewrites its facts"
        );

        // OUT of the band. The seat still owes the checkpoint it was asked for.
        let _ = pass(&mut carry, "50", 9_903);
        assert_eq!(carry.tracked[0].classified.level(), QuotaLevel::Headroom);
        assert_eq!(carry.asks.len(), 1, "clearing does not retract it either");
        assert_eq!(carry.asks[0].observed_at, 9_901);

        // IN again, while the first is still owed: replaced, never doubled.
        assert_eq!(pass(&mut carry, "88", 9_904), vec!["lead".to_owned()]);
        assert_eq!(
            carry.asks.len(),
            1,
            "one seat and one scope hold ONE owed ask: {:?}",
            carry.asks
        );
        assert_eq!(
            carry.asks[0].observed_at, 9_904,
            "and it carries the newer entry"
        );
        let text = carry.asks[0].advisory.checkpoint_ask(meta);
        assert!(text.contains("88"), "the facts of the new entry: {text}");
        assert!(!text.contains("85"), "never the superseded booking: {text}");
    }

    #[test]
    fn the_ask_text_quotes_the_held_reading_and_stays_within_its_bound() {
        let roster = [codex_seat("main", "lead", "/tmp/cx")];
        let panes = live_panes(&roster);
        let candidates = quota_ask_candidates(&roster, &panes);
        let meta = Path::new("/m");
        let mut carry = QuotaCarry::default();
        assert!(
            carry
                .reconcile_with_candidates(
                    &codex_scope(&roster[0], "70", 9_900),
                    &[],
                    &candidates,
                    meta
                )
                .is_empty()
        );
        let actions = carry.reconcile_with_candidates(
            &codex_scope(&roster[0], "85", 9_901),
            &[],
            &candidates,
            meta,
        );
        assert_eq!(asked_agents(&actions), vec!["lead".to_owned()]);

        // An OLDER sample is refused by the raw clock, so it books nothing —
        // and, the point of this pin, it does not reach the text of the ask
        // already booked from the observation that DID decide the level.
        assert!(
            carry
                .reconcile_with_candidates(
                    &codex_scope(&roster[0], "99", 9_899),
                    &[],
                    &candidates,
                    meta
                )
                .iter()
                .filter(|action| matches!(action, QuotaAction::Dropped { .. }))
                .count()
                == 0,
            "a refused sample cancels nothing"
        );
        assert_eq!(carry.asks.len(), 1, "and books nothing new");

        let text = carry.asks[0].advisory.checkpoint_ask(meta);
        assert!(text.contains("low"), "the level it was judged at: {text}");
        assert!(text.contains("85"), "the judged percentage: {text}");
        assert!(
            !text.contains("99"),
            "never the sample the clock refused: {text}"
        );
        assert!(
            text.contains("/m/memo add --topic"),
            "the exact command that answers it: {text}"
        );
        assert!(
            text.contains("No reply needed"),
            "it opens no request: {text}"
        );
        assert!(
            text.chars().count() <= crate::quota::CHECKPOINT_ASK_MAX,
            "one paste, {} chars: {text}",
            text.chars().count()
        );
    }

    #[test]
    fn throttle_quota_respects_cached_row_reset_boundary() {
        let rollout = "018f1f70-7b2c-7000-8000-000000000001";
        let entry = RosterEntry {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: None,
            client: RecordedClient::Missing,
            harness_session: Some(rollout.to_owned()),
            config_home: RecordedConfigHome::Path(PathBuf::from("/tmp/cx")),
            config_home_base: RecordedConfigHomeBase::Missing,
            binary: Some("codex".to_owned()),
        };
        let identity = crate::quota::recorded_identity(&entry).expect("recorded identity");
        let mut observation = quota_observation(
            vec![quota_group(
                &identity.source,
                Some(rollout),
                Some("demo:lead"),
                vec![quota_row(
                    "weekly_scoped",
                    Some("pro"),
                    "96",
                    9_900,
                    crate::quota::Status::Fresh,
                )],
            )],
            10_000,
        );
        observation.groups[0].rows[0].resets_at = Some(10_050);
        let mut carry = QuotaCarry::default();
        let _ = carry.reconcile(&observation, &[], Path::new("/m"));
        assert!(
            throttle_quota_line(
                &observation,
                &carry.tracked,
                &entry,
                Path::new("/m"),
                10_049
            )
            .is_some()
        );
        assert!(
            throttle_quota_line(
                &observation,
                &carry.tracked,
                &entry,
                Path::new("/m"),
                10_050
            )
            .is_none()
        );
    }

    #[test]
    fn silent_or_expired_rows_drop_state_and_make_recovery_a_first_sample() {
        let recipients = [quota_recipient("main", "lead")];
        let mut carry = QuotaCarry::default();
        let _ = carry.reconcile(&quota_for("79", 9_900), &recipients, Path::new("/m"));
        let mut silent = quota_for("80", 9_901);
        silent.groups[0].rows[0].status = crate::quota::Status::Unknown;
        let actions = carry.reconcile(&silent, &recipients, Path::new("/m"));
        assert!(
            actions.is_empty(),
            "no pending transition existed to cancel"
        );
        assert!(carry.tracked.is_empty());
        assert!(
            carry
                .reconcile(&quota_for("95", 9_902), &recipients, Path::new("/m"))
                .is_empty(),
            "fresh recovery is a new baseline"
        );

        let mut expired = quota_for("95", 3_999);
        expired.now = 10_000;
        let _ = carry.reconcile(&expired, &recipients, Path::new("/m"));
        assert!(carry.tracked.is_empty(), "stale beyond 60m is silent");
    }

    #[test]
    fn one_config_home_seen_by_many_rollouts_advises_once_per_transition() {
        let source = Path::new("/tmp/cx/sessions");
        let owners = ["demo:lead", "demo:colead", "demo:reviewer"];
        let observation = |used: &str, at: i64| {
            quota_observation(
                owners
                    .iter()
                    .enumerate()
                    .map(|(index, owner)| {
                        quota_group(
                            source,
                            Some(&format!("018f1f70-7b2c-7000-8000-00000000000{index}")),
                            Some(owner),
                            vec![quota_row(
                                "codex",
                                Some("pro"),
                                used,
                                at,
                                crate::quota::Status::Fresh,
                            )],
                        )
                    })
                    .collect(),
                10_000,
            )
        };
        let recipients = [quota_recipient("main", "lead")];
        let mut carry = QuotaCarry::default();
        let _ = carry.reconcile(&observation("79", 9_900), &recipients, Path::new("/m"));
        assert_eq!(
            carry.tracked.len(),
            1,
            "one window fact, whichever rollout observed it"
        );
        let actions = carry.reconcile(&observation("95", 9_901), &recipients, Path::new("/m"));
        let deliveries = transition_deliveries(&actions);
        assert_eq!(
            deliveries.len(),
            1,
            "three owners on one config home are one interrupt: {:?}",
            deliveries
                .iter()
                .map(|pending| pending.advisory.render(Path::new("/m"), 10_000))
                .collect::<Vec<_>>()
        );
        let delivered = deliveries[0].clone();
        assert!(
            carry
                .record_delivery(
                    &delivered,
                    QuotaDelivery::Delivered,
                    Path::new("/m"),
                    10_000
                )
                .is_none()
        );
        assert!(
            carry
                .reconcile(&observation("96", 9_902), &recipients, Path::new("/m"))
                .is_empty(),
            "an unchanged state re-arms nothing"
        );
    }

    #[test]
    fn a_policy_change_reclassifies_the_current_snapshot_without_moving_the_raw_clock() {
        let source = Path::new("/tmp/cx/sessions");
        let observation = |used: &str, at: i64, resets: Option<u8>, now: i64| {
            quota_observation(
                vec![quota_scope_group(
                    source,
                    Some("018f1f70-7b2c-7000-8000-000000000001"),
                    Some("demo:lead"),
                    vec![quota_row(
                        "codex",
                        Some("pro"),
                        used,
                        at,
                        crate::quota::Status::Fresh,
                    )],
                    resets,
                    crate::quota::Account::default(),
                )],
                now,
            )
        };
        let recipients = [quota_recipient("main", "lead")];
        let dir = Path::new("/m");

        // 0 -> 1 clears: the measured sequence, now resolved at the same
        // vendor observation instead of waiting for a newer one.
        let mut carry = QuotaCarry::default();
        let _ = carry.reconcile(&observation("79", 9_900, None, 10_000), &recipients, dir);
        let booked = carry.reconcile(&observation("95", 9_901, None, 10_000), &recipients, dir);
        assert_eq!(
            transition_deliveries(&booked).len(),
            1,
            "control: it fires once"
        );
        assert_eq!(carry.tracked[0].classified.level(), QuotaLevel::Critical);

        let relieved =
            carry.reconcile(&observation("95", 9_901, Some(1), 10_001), &recipients, dir);
        assert_eq!(
            carry.tracked[0].classified.level(),
            QuotaLevel::Headroom,
            "a declaration rebuilds the classification from the current snapshot"
        );
        assert_eq!(
            carry.tracked[0].classified.observed_at(),
            9_901,
            "the raw clock does not move on a policy change"
        );
        let dropped: Vec<&String> = relieved
            .iter()
            .filter_map(|action| match action {
                QuotaAction::Dropped { summary, .. } => Some(summary),
                QuotaAction::Ask(_) | QuotaAction::Deliver(_) => None,
            })
            .collect();
        assert_eq!(
            dropped.len(),
            1,
            "the stale notice is cancelled: {dropped:?}"
        );
        let delivered = transition_deliveries(&relieved);
        assert_eq!(delivered.len(), 1, "and one current notice replaces it");
        assert_eq!(delivered[0].level, QuotaLevel::Headroom);
        let text = delivered[0].advisory.render(dir, 10_001);
        assert!(
            text.contains("effective 47.5% over 1+1 declared resets"),
            "{text}"
        );
        assert!(
            carry
                .pending
                .iter()
                .all(|pending| pending.level == QuotaLevel::Headroom),
            "no Critical notice survives the change"
        );

        // An older raw observation is still refused afterwards.
        let older = carry.reconcile(&observation("95", 9_800, Some(1), 10_002), &recipients, dir);
        assert_eq!(
            carry.tracked[0].classified.observed_at(),
            9_901,
            "older raw stays refused"
        );
        assert!(
            transition_deliveries(&older)
                .iter()
                .all(|pending| pending.level == QuotaLevel::Headroom),
            "an older raw sample books nothing new"
        );

        // 1 -> 0 re-arms, symmetrically.
        let rearmed = carry.reconcile(&observation("95", 9_901, Some(0), 10_003), &recipients, dir);
        assert_eq!(
            carry.tracked[0].classified.level(),
            QuotaLevel::Critical,
            "withdrawing the declaration re-arms the same snapshot"
        );
        assert_eq!(carry.tracked[0].classified.observed_at(), 9_901);
        let delivered = transition_deliveries(&rearmed);
        assert!(
            delivered
                .iter()
                .any(|pending| pending.level == QuotaLevel::Critical),
            "the critical notice comes back: {:?}",
            delivered
                .iter()
                .map(|pending| pending.advisory.render(dir, 10_003))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_older_raw_sample_is_refused_even_when_the_policy_changes_with_it() {
        let source = Path::new("/tmp/cx/sessions");
        let observation = |used: &str, at: i64, resets: Option<u8>, now: i64| {
            quota_observation(
                vec![quota_scope_group(
                    source,
                    Some("018f1f70-7b2c-7000-8000-000000000001"),
                    Some("demo:lead"),
                    vec![quota_row(
                        "codex",
                        Some("pro"),
                        used,
                        at,
                        crate::quota::Status::Fresh,
                    )],
                    resets,
                    crate::quota::Account::default(),
                )],
                now,
            )
        };
        let recipients = [quota_recipient("main", "lead")];
        let dir = Path::new("/m");

        // Held 95% at 9901 under one declared reset is Headroom. Withdrawing
        // the declaration must classify THAT observation, not the older 20%
        // that arrives with it.
        let mut carry = QuotaCarry::default();
        let _ = carry.reconcile(&observation("95", 9_901, Some(1), 10_000), &recipients, dir);
        assert_eq!(carry.tracked[0].classified.level(), QuotaLevel::Headroom);
        let actions = carry.reconcile(&observation("20", 9_800, Some(0), 10_001), &recipients, dir);
        assert_eq!(
            carry.tracked[0].classified.observed_at(),
            9_901,
            "an older raw observation is refused by timestamp alone"
        );
        assert_eq!(
            carry.tracked[0].classified.level(),
            QuotaLevel::Critical,
            "the held 95% under no declared reset is critical"
        );
        let booked = transition_deliveries(&actions);
        assert_eq!(booked.len(), 1, "and that is what is booked");
        assert_eq!(booked[0].observed_at, 9_901, "from the held provenance");
        let text = booked[0].advisory.render(dir, 10_001);
        assert!(
            text.contains(" 95% (critical,"),
            "the held number is quoted: {text}"
        );
        assert!(
            !text.contains("20%"),
            "the refused number never appears: {text}"
        );

        // The mirror: a held critical relieved by a declaration must quote the
        // held observation too, never the refused older sample.
        let mut mirror = QuotaCarry::default();
        let _ = mirror.reconcile(&observation("40", 9_900, Some(0), 10_000), &recipients, dir);
        let _ = mirror.reconcile(&observation("95", 9_901, Some(0), 10_001), &recipients, dir);
        assert_eq!(mirror.tracked[0].classified.level(), QuotaLevel::Critical);
        let relieved =
            mirror.reconcile(&observation("20", 9_800, Some(1), 10_002), &recipients, dir);
        assert_eq!(
            mirror.tracked[0].classified.observed_at(),
            9_901,
            "older raw still refused"
        );
        assert_eq!(mirror.tracked[0].classified.level(), QuotaLevel::Headroom);
        let booked = transition_deliveries(&relieved);
        let text = booked
            .iter()
            .map(|pending| pending.advisory.render(dir, 10_002))
            .find(|line| line.contains("back to headroom"))
            .expect("one relieved notice");
        assert!(
            text.contains(" 95% (headroom; effective 47.5% over 1+1 declared resets"),
            "the held observation is what was relieved: {text}"
        );
        assert!(!text.contains("20%"), "{text}");
        assert!(!text.contains("effective 10%"), "{text}");
    }

    #[test]
    fn the_threshold_reads_the_same_derivation_the_table_renders() {
        let source = Path::new("/tmp/cx/sessions");
        let observation = quota_observation(
            vec![quota_scope_group(
                source,
                Some("018f1f70-7b2c-7000-8000-000000000001"),
                Some("demo:lead"),
                vec![quota_row(
                    "codex",
                    Some("pro"),
                    "95",
                    9_900,
                    crate::quota::Status::Fresh,
                )],
                Some(1),
                crate::quota::Account::default(),
            )],
            10_000,
        );
        let sample = &super::quota_samples(&observation)[0];
        let derivation =
            crate::quota::derived(&sample.group.policy, &sample.group.rows[0]).expect("derivable");
        assert_eq!(
            sample.reading.judged().to_bits(),
            derivation.judged().to_bits(),
            "the threshold reads the very same value, bit for bit"
        );
        assert_eq!(
            derivation.cell(),
            "47.5% x1",
            "and the very same value is what the table renders"
        );
    }

    #[test]
    fn a_declared_manual_reset_judges_the_scope_by_its_effective_headroom() {
        let source = Path::new("/tmp/cx/sessions");
        let observation = |used: &str, at: i64, resets: Option<u8>| {
            quota_observation(
                vec![quota_scope_group(
                    source,
                    Some("018f1f70-7b2c-7000-8000-000000000001"),
                    Some("demo:lead"),
                    vec![quota_row(
                        "codex",
                        Some("pro"),
                        used,
                        at,
                        crate::quota::Status::Fresh,
                    )],
                    resets,
                    crate::quota::Account::default(),
                )],
                10_000,
            )
        };
        let recipients = [quota_recipient("main", "lead")];
        let mut declared = QuotaCarry::default();
        let _ = declared.reconcile(
            &observation("40", 9_900, Some(1)),
            &recipients,
            Path::new("/m"),
        );
        assert!(
            declared
                .reconcile(
                    &observation("95", 9_901, Some(1)),
                    &recipients,
                    Path::new("/m")
                )
                .is_empty(),
            "95% of one window is 47.5% of two: no advisory"
        );
        let mut raw = QuotaCarry::default();
        let _ = raw.reconcile(
            &observation("40", 9_900, None),
            &recipients,
            Path::new("/m"),
        );
        let actions = raw.reconcile(
            &observation("95", 9_901, None),
            &recipients,
            Path::new("/m"),
        );
        assert_eq!(
            transition_deliveries(&actions).len(),
            1,
            "an undeclared scope still fires on the raw window"
        );

        let mut crossing = QuotaCarry::default();
        let _ = crossing.reconcile(
            &observation("100", 9_900, Some(0)),
            &recipients,
            Path::new("/m"),
        );
        let text = observation("100", 9_901, Some(1)).state_line(
            &observation("100", 9_901, Some(1)).groups[0],
            &quota_classified(&observation("100", 9_901, Some(1)).groups[0]),
            Path::new("/m"),
        );
        assert!(
            text.contains("effective 50% over 1+1 declared resets"),
            "{text}"
        );
    }

    #[test]
    fn a_spend_capped_scope_is_critical_and_says_which_constraint_binds() {
        let source = Path::new("/tmp/cx/sessions");
        let capped = crate::quota::Account {
            credits: crate::quota::Credits::Exhausted,
            credits_observed_at: Some(9_900),
            spend_control_reached: Some(true),
            spend_observed_at: Some(9_900),
        };
        let observation = quota_observation(
            vec![quota_scope_group(
                source,
                Some("018f1f70-7b2c-7000-8000-000000000001"),
                Some("demo:lead"),
                vec![quota_row(
                    "codex",
                    Some("pro"),
                    "3",
                    9_900,
                    crate::quota::Status::Fresh,
                )],
                Some(2),
                capped,
            )],
            10_000,
        );
        let sample = &super::quota_samples(&observation)[0];
        assert_eq!(
            crate::quota::Classified::first(sample.reading.clone()).level(),
            QuotaLevel::Critical,
            "a spend cap is not headroom at 3% of a window"
        );
        let line = observation.state_line(
            sample.group,
            &crate::quota::Classified::first(sample.reading.clone()),
            Path::new("/m"),
        );
        assert!(line.contains("spend cap reached"), "{line}");
        assert!(
            line.contains("not a window reset"),
            "the advice must not send the reader to a reset: {line}"
        );
        assert!(
            !line.contains("declared resets"),
            "two declared resets must not describe a spend-capped client: {line}"
        );
        assert!(
            !line.contains("headroom"),
            "a spend cap is never headroom: {line}"
        );
    }

    #[test]
    fn quota_keys_separate_scoped_qualifiers_but_never_rollouts_of_one_scope() {
        let source = Path::new("/tmp/cx/sessions");
        let groups = vec![
            quota_group(
                source,
                Some("018f1f70-7b2c-7000-8000-000000000001"),
                Some("demo:lead"),
                vec![
                    quota_row(
                        "weekly_scoped",
                        Some("Fable"),
                        "10",
                        9_900,
                        crate::quota::Status::Fresh,
                    ),
                    quota_row(
                        "weekly_scoped",
                        Some("Opus"),
                        "20",
                        9_900,
                        crate::quota::Status::Fresh,
                    ),
                ],
            ),
            quota_group(
                source,
                Some("018f1f70-7b2c-7000-8000-000000000002"),
                Some("demo:colead"),
                vec![quota_row(
                    "weekly_scoped",
                    Some("Fable"),
                    "30",
                    9_900,
                    crate::quota::Status::Fresh,
                )],
            ),
        ];
        let mut carry = QuotaCarry::default();
        assert!(
            carry
                .reconcile(&quota_observation(groups, 10_000), &[], Path::new("/m"))
                .is_empty()
        );
        assert_eq!(
            carry.tracked.len(),
            2,
            "qualifiers stay separate; two rollouts of one scope do not"
        );
    }

    #[test]
    fn pending_advisories_book_per_recipient_retry_once_and_never_resend_success() {
        let recipients = [
            quota_recipient("main", "lead"),
            quota_recipient("worker.0", "colead"),
        ];
        let mut carry = QuotaCarry::default();
        let _ = carry.reconcile(&quota_for("79", 9_900), &recipients, Path::new("/m"));
        let first = carry.reconcile(&quota_for("80", 9_901), &recipients, Path::new("/m"));
        let deliveries = transition_deliveries(&first);
        assert_eq!(deliveries.len(), 2);
        let lead = deliveries[0].clone();
        let colead = deliveries[1].clone();
        assert!(
            carry
                .record_delivery(&lead, QuotaDelivery::Delivered, Path::new("/m"), 10_000)
                .is_none()
        );
        assert!(
            carry
                .record_delivery(&colead, QuotaDelivery::Retryable, Path::new("/m"), 10_000)
                .is_none(),
            "first refusal stays pending"
        );
        let retry = carry.reconcile(&quota_for("81", 9_902), &recipients, Path::new("/m"));
        let retry = transition_deliveries(&retry);
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].recipient.agent, "colead");
        assert!(matches!(
            carry.record_delivery(
                retry[0],
                QuotaDelivery::Retryable,
                Path::new("/m"),
                10_000
            ),
            Some(QuotaAction::Dropped { recipient, .. }) if recipient == "colead"
        ));
        assert!(carry.pending.is_empty(), "there is no third attempt");
    }

    #[test]
    fn quota_delivery_retries_only_the_helpers_proven_refusal_marker() {
        assert_eq!(
            quota_delivery(&crate::transport::Delivery {
                code: Some(0),
                stdout: String::new(),
            }),
            QuotaDelivery::Delivered,
            "confirmed and UNCONFIRMED sends both exit zero"
        );
        assert_eq!(
            quota_delivery(&crate::transport::Delivery {
                code: Some(1),
                stdout: format!("{}\n", crate::send::RETRYABLE_MARKER),
            }),
            QuotaDelivery::Retryable
        );
        assert_eq!(
            quota_delivery(&crate::transport::Delivery {
                code: Some(1),
                stdout: String::new(),
            }),
            QuotaDelivery::Uncertain,
            "post-submit logging failure must not duplicate a paste"
        );

        let recipients = [quota_recipient("main", "lead")];
        let mut carry = QuotaCarry::default();
        let _ = carry.reconcile(&quota_for("79", 9_900), &recipients, Path::new("/m"));
        let first = carry.reconcile(&quota_for("80", 9_901), &recipients, Path::new("/m"));
        let pending = transition_deliveries(&first)[0];
        assert!(matches!(
            carry.record_delivery(
                pending,
                QuotaDelivery::Uncertain,
                Path::new("/m"),
                10_000
            ),
            Some(QuotaAction::Dropped { recipient, summary })
                if recipient == "lead" && summary.contains("not retrying")
        ));
        assert!(carry.pending.is_empty());
    }

    #[test]
    fn expired_quota_pending_drops_old_booking_while_fresh_same_level_advances_state() {
        let recipients = [quota_recipient("main", "lead")];
        let mut carry = QuotaCarry::default();
        let _ = carry.reconcile(&quota_for("79", 9_900), &recipients, Path::new("/m"));
        let first = carry.reconcile(&quota_for("80", 9_901), &recipients, Path::new("/m"));
        let pending = transition_deliveries(&first)[0].clone();
        let old_text = pending.advisory.render(Path::new("/m"), 10_000);
        assert!(
            carry
                .record_delivery(&pending, QuotaDelivery::Retryable, Path::new("/m"), 10_000)
                .is_none()
        );

        let mut control = QuotaCarry::default();
        let _ = control.reconcile(&quota_for("79", 9_900), &recipients, Path::new("/m"));
        let control_first =
            control.reconcile(&quota_for("80", 9_901), &recipients, Path::new("/m"));
        let control_pending = transition_deliveries(&control_first)[0].clone();
        assert!(
            control
                .record_delivery(
                    &control_pending,
                    QuotaDelivery::Retryable,
                    Path::new("/m"),
                    10_000
                )
                .is_none()
        );
        let control_retry =
            control.reconcile(&quota_for("80", 9_902), &recipients, Path::new("/m"));
        assert_eq!(transition_deliveries(&control_retry).len(), 1);
        assert!(
            transition_deliveries(&control_retry)[0]
                .advisory
                .render(Path::new("/m"), 10_001)
                .contains("resets in 59m"),
            "retry text is rendered against its delivery clock"
        );

        let mut expired = quota_for("80", 13_601);
        expired.now = 13_602;
        expired.groups[0].rows[0].resets_at = Some(14_000);
        expired.groups[0].rows[0].status = crate::quota::freshness(
            expired.groups[0].rows[0].observed_at,
            expired.groups[0].rows[0].resets_at,
            expired.now,
        );
        assert_eq!(
            expired.groups[0].rows[0].status,
            crate::quota::Status::Fresh
        );
        let expired_actions = carry.reconcile(&expired, &recipients, Path::new("/m"));
        assert!(
            expired_actions
                .iter()
                .all(|action| !matches!(action, QuotaAction::Deliver(_)))
        );
        assert!(matches!(
            expired_actions.as_slice(),
            [QuotaAction::Dropped { recipient, summary }]
                if recipient == "lead"
                    && summary.starts_with("quota advisory expired: quota:")
                    && !summary.contains(&old_text)
        ));
    }

    #[test]
    fn changed_roster_identity_cancels_old_quota_pending() {
        let base = RosterEntry {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: None,
            client: RecordedClient::Missing,
            harness_session: Some("018f1f70-7b2c-7000-8000-000000000001".to_owned()),
            config_home: RecordedConfigHome::Path(PathBuf::from("/tmp/cx")),
            config_home_base: RecordedConfigHomeBase::Missing,
            binary: Some("codex".to_owned()),
        };
        let mut changed = base.clone();
        changed.harness_session = Some("018f1f70-7b2c-7000-8000-000000000002".to_owned());
        changed.config_home = RecordedConfigHome::Path(PathBuf::from("/tmp/cx-new"));
        let old_recipients = quota_recipients(std::slice::from_ref(&base), false);
        let new_recipients = quota_recipients(std::slice::from_ref(&changed), false);
        let mut profile_only = base.clone();
        profile_only.profile = Some("changed-profile".to_owned());
        assert_eq!(
            old_recipients,
            quota_recipients(std::slice::from_ref(&profile_only), false),
            "profile labels do not replace a recorded conversation"
        );

        let mut control = QuotaCarry::default();
        let _ = control.reconcile(&quota_for("79", 9_900), &old_recipients, Path::new("/m"));
        let control_first =
            control.reconcile(&quota_for("80", 9_901), &old_recipients, Path::new("/m"));
        let control_pending = transition_deliveries(&control_first)[0].clone();
        assert!(
            control
                .record_delivery(
                    &control_pending,
                    QuotaDelivery::Retryable,
                    Path::new("/m"),
                    10_000
                )
                .is_none()
        );
        let control_retry =
            control.reconcile(&quota_for("80", 9_902), &old_recipients, Path::new("/m"));
        assert_eq!(transition_deliveries(&control_retry).len(), 1);

        let mut carry = QuotaCarry::default();
        let _ = carry.reconcile(&quota_for("79", 9_900), &old_recipients, Path::new("/m"));
        let first = carry.reconcile(&quota_for("80", 9_901), &old_recipients, Path::new("/m"));
        let pending = transition_deliveries(&first)[0].clone();
        assert!(
            carry
                .record_delivery(&pending, QuotaDelivery::Retryable, Path::new("/m"), 10_000)
                .is_none()
        );
        let changed_actions =
            carry.reconcile(&quota_for("80", 9_902), &new_recipients, Path::new("/m"));
        assert!(
            changed_actions
                .iter()
                .all(|action| !matches!(action, QuotaAction::Deliver(_)))
        );
        assert!(matches!(
            changed_actions.as_slice(),
            [QuotaAction::Dropped { recipient, .. }] if recipient == "lead"
        ));
    }

    #[test]
    fn quota_samples_borrow_their_group_instead_of_cloning_its_rows() {
        let rows: Vec<_> = (0..64)
            .map(|index| {
                quota_row(
                    "weekly_scoped",
                    Some(&format!("qualifier-{index:02}")),
                    "10",
                    9_900,
                    crate::quota::Status::Fresh,
                )
            })
            .collect();
        let observation = quota_observation(
            vec![quota_group(
                Path::new("/tmp/cx/sessions"),
                Some("018f1f70-7b2c-7000-8000-000000000001"),
                None,
                rows,
            )],
            10_000,
        );
        let samples = super::quota_samples(&observation);
        assert_eq!(samples.len(), 64);
        let original = &raw const observation.groups[0];
        assert!(
            samples
                .iter()
                .all(|sample| { std::ptr::eq(std::ptr::from_ref(sample.group), original) })
        );
    }

    #[test]
    fn newer_transition_replaces_retry_and_expiry_cancels_it() {
        let recipients = [quota_recipient("main", "lead")];
        let mut carry = QuotaCarry::default();
        let _ = carry.reconcile(&quota_for("79", 9_900), &recipients, Path::new("/m"));
        let low = carry.reconcile(&quota_for("80", 9_901), &recipients, Path::new("/m"));
        assert!(
            carry
                .record_delivery(
                    transition_deliveries(&low)[0],
                    QuotaDelivery::Retryable,
                    Path::new("/m"),
                    10_000
                )
                .is_none()
        );
        let reset = carry.reconcile(&quota_for("74", 9_902), &recipients, Path::new("/m"));
        assert_eq!(
            reset
                .iter()
                .filter(|action| matches!(action, QuotaAction::Dropped { .. }))
                .count(),
            1
        );
        let reset_delivery = transition_deliveries(&reset);
        assert_eq!(reset_delivery.len(), 1);
        assert!(
            reset_delivery[0]
                .advisory
                .render(Path::new("/m"), 10_000)
                .contains("back to headroom")
        );
        assert!(
            carry
                .record_delivery(
                    reset_delivery[0],
                    QuotaDelivery::Retryable,
                    Path::new("/m"),
                    10_000
                )
                .is_none()
        );
        let mut expired = quota_for("74", 3_000);
        expired.now = 10_000;
        let cancelled = carry.reconcile(&expired, &recipients, Path::new("/m"));
        assert!(matches!(
            cancelled.as_slice(),
            [QuotaAction::Dropped { recipient, .. }] if recipient == "lead"
        ));
        assert!(carry.pending.is_empty());
    }

    #[test]
    fn leadership_recipients_are_session_local_and_layout_scoped() {
        let entry = |slot: &str, name: &str| RosterEntry {
            slot: slot.to_owned(),
            name: name.to_owned(),
            profile: None,
            client: RecordedClient::Missing,
            harness_session: None,
            config_home: RecordedConfigHome::Missing,
            config_home_base: RecordedConfigHomeBase::Missing,
            binary: None,
        };
        let roster = [
            entry("main", "lead"),
            entry("worker.0", "colead"),
            entry("worker.1", "builder"),
            entry("spawned.0", "reviewer"),
        ];
        assert_eq!(
            quota_recipients(&roster, false),
            [quota_recipient("main", "lead")]
        );
        assert_eq!(
            quota_recipients(&roster, true),
            [
                quota_recipient("main", "lead"),
                quota_recipient("worker.0", "colead")
            ]
        );
        let orchestrator = [entry("main", "orchestrator")];
        assert_eq!(
            quota_recipients(&orchestrator, false),
            [quota_recipient("main", "orchestrator")]
        );
    }

    #[test]
    fn quota_cadence_rounds_up_to_whole_sweeps_and_zero_disables() {
        let mut carry = QuotaCarry::default();
        let knobs = Knobs {
            interval_secs: 60,
            quota_every_secs: 301,
            ..Knobs::default()
        };
        let due: Vec<bool> = (0..7)
            .map(|_| quota_observation_due(&mut carry, &knobs))
            .collect();
        assert_eq!(due, [true, false, false, false, false, false, true]);
        let mut disabled = QuotaCarry::default();
        assert!(!(0..10).any(|_| quota_observation_due(
            &mut disabled,
            &Knobs {
                quota_every_secs: 0,
                ..Knobs::default()
            }
        )));
    }

    #[test]
    fn picker_marker_expires_at_half_a_watchdog_interval() {
        assert!(super::menu_open_expired("100", 130, 60));
        assert!(!super::menu_open_expired("100", 129, 60));
        assert!(!super::menu_open_expired("200", 161, 60));
        assert!(
            super::menu_open_expired("not-an-epoch", 161, 60),
            "malformed transient state must not light the button forever"
        );
    }

    #[test]
    fn persisted_quota_awareness_defaults_on_reads_the_pin_and_honours_live_config() {
        let scratch = Scratch::new("quota-awareness");
        let off = scratch.0.join("off");
        let on = scratch.0.join("on");
        std::fs::write(&off, "[workspace]\nquota = off\n").unwrap();
        std::fs::write(&on, "[workspace]\nquota = on\n").unwrap();
        let ask = |meta: &[u8], config: Option<&std::path::Path>| {
            super::quota_awareness(meta, config, None)
        };
        // Absent means ON — exactly today's behaviour for pre-knob sessions.
        assert!(ask(b"session=demo\n", None));
        assert!(ask(b"", None));
        assert!(ask(b"session=demo\nquota=on\n", Some(off.as_path())));
        // Unknown spellings stay ON, like the look knobs they share grammar with.
        assert!(ask(b"session=demo\nquota=bogus\n", None));
        for off_meta in [
            b"quota=off\n".as_slice(),
            b"quota=OFF\n",
            b"quota=0\n",
            b"quota=no\n",
        ] {
            assert!(!ask(off_meta, None), "{off_meta:?} must mean unaware");
        }
        // The off-diagonal: a pre-knob session (no pin) honours live config.
        assert!(
            !ask(b"session=demo\n", Some(off.as_path())),
            "no pin plus config off is unaware everywhere"
        );
        assert!(ask(b"session=demo\n", Some(on.as_path())));
        // Awareness is ON by default in fresh knobs.
        assert!(Knobs::default().quota_aware);
    }

    /// An OFF flip drops everything the aware phase held, before any consumer
    /// runs: tracked windows, pending notices and the last observation. No
    /// tmux here, so the pane walk is skipped — the clear is what is pinned.
    /// Deleting it leaves the held rows behind and fails every assertion.
    #[test]
    fn unaware_cycle_clears_held_quota_state_before_any_consumer() {
        let recipients = [quota_recipient("main", "lead")];
        let dir = Path::new("/m");
        let mut carry = Carry::new(&Knobs::default());
        assert!(
            carry
                .quota
                .reconcile(&quota_for("79", 9_900), &recipients, dir)
                .is_empty(),
            "first sample is baseline only"
        );
        assert_eq!(
            transition_deliveries(&carry.quota.reconcile(
                &quota_for("95", 9_901),
                &recipients,
                dir
            ))
            .len(),
            1,
            "the aware phase holds a booked transition"
        );
        assert!(carry.quota.last_observation.is_some());
        let scratch = Scratch::new("unaware-clear");
        let helper = SendHelper::for_session(&scratch.0);
        let server = ServerId::Ambient;
        let cycle = Cycle {
            knobs: Knobs {
                quota_aware: false,
                ..Knobs::default()
            },
            meta_dir: &scratch.0,
            helper: &helper,
            server: &server,
            session: "demo",
            goal: None,
            roster: Vec::new(),
            local_config: None,
            lead_pair: false,
            fleet_order: crate::theme::FleetOrder::EMPTY,
            meta_agent: false,
            launch_ids: Vec::new(),
        };
        let mut err = Vec::new();
        cycle
            .run(&mut carry, &mut err)
            .expect("a skipped enumeration is not a failure");
        assert!(carry.quota.tracked.is_empty(), "tracked rows dropped");
        assert!(carry.quota.pending.is_empty(), "pending notices dropped");
        assert!(
            carry.quota.last_observation.is_none(),
            "no held observation for the throttle line"
        );
    }

    /// A carry holding no observation renders no throttle line even for a
    /// throttled seat. That is the whole unaware throttle story now: the
    /// cycle skips the quota read entirely when unaware, so there is never a
    /// held observation to render — `run_quota_cadence`'s guard is the gate
    /// and the unaware-daemon integration test pins it at production level.
    #[test]
    fn a_throttled_seat_without_a_held_observation_renders_no_quota_line() {
        let rollout = "018f1f70-7b2c-7000-8000-000000000001";
        let entry = RosterEntry {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: None,
            client: RecordedClient::Missing,
            harness_session: Some(rollout.to_owned()),
            config_home: RecordedConfigHome::Path(PathBuf::from("/tmp/cx")),
            config_home_base: RecordedConfigHomeBase::Missing,
            binary: Some("codex".to_owned()),
        };
        let identity = crate::quota::recorded_identity(&entry).expect("recorded identity");
        let observation = quota_observation(
            vec![quota_group(
                &identity.source,
                Some(rollout),
                Some("demo:lead"),
                vec![quota_row(
                    "weekly_scoped",
                    Some("pro"),
                    "96",
                    9_900,
                    crate::quota::Status::Fresh,
                )],
            )],
            10_000,
        );
        let mut carry = QuotaCarry::default();
        let _ = carry.reconcile(&observation, &[], Path::new("/m"));
        let scratch = Scratch::new("unaware-throttle");
        let helper = SendHelper::for_session(&scratch.0);
        let server = ServerId::Ambient;
        let cycle = Cycle {
            knobs: Knobs::default(),
            meta_dir: &scratch.0,
            helper: &helper,
            server: &server,
            session: "demo",
            goal: None,
            roster: vec![entry.clone()],
            local_config: None,
            lead_pair: false,
            fleet_order: crate::theme::FleetOrder::EMPTY,
            meta_agent: false,
            launch_ids: Vec::new(),
        };
        assert!(
            cycle.throttle_quota(&carry, "main", 10_000, true).is_some(),
            "a held observation renders the line for a throttled seat"
        );
        assert!(
            cycle
                .throttle_quota(&QuotaCarry::default(), "main", 10_000, true)
                .is_none(),
            "no held observation, no quota line — however throttled the pane"
        );
    }

    #[test]
    fn persisted_quota_cadence_wins_and_invalid_state_is_refused() {
        assert_eq!(quota_seconds(b"quota_every_secs=420\n", 300), Ok(420));
        assert_eq!(quota_seconds(b"quota_every_secs=0\n", 300), Ok(0));
        assert_eq!(quota_seconds(b"session=demo\n", 300), Ok(300));
        assert_eq!(
            quota_seconds(b"quota_every_secs=soon\n", 300),
            Err("soon".to_owned())
        );
        assert_eq!(
            quota_seconds(b"quota_every_secs=60\nquota_every_secs=120\n", 300),
            Err("60".to_owned()),
            "ambiguous persisted state must not enable an arbitrary cadence"
        );
    }

    #[test]
    fn persisted_idle_nudge_cadence_wins_and_invalid_state_is_refused() {
        assert_eq!(idle_nudge_seconds(b"idle_nudge_secs=420\n", 300), Ok(420));
        assert_eq!(idle_nudge_seconds(b"idle_nudge_secs=0\n", 300), Ok(0));
        assert_eq!(idle_nudge_seconds(b"session=demo\n", 300), Ok(300));
        assert_eq!(
            idle_nudge_seconds(b"idle_nudge_secs=soon\n", 300),
            Err("soon".to_owned())
        );
        assert_eq!(
            idle_nudge_seconds(b"idle_nudge_secs=60\nidle_nudge_secs=120\n", 300),
            Err("60".to_owned()),
            "ambiguous persisted state must not enable an arbitrary cadence"
        );
    }

    #[test]
    fn advisory_text_is_exact_and_hostile_vendor_fields_are_bounded() {
        let source = Path::new("/tmp/cx/sessions");
        let observation = quota_observation(
            vec![quota_group(
                source,
                Some("018f1f70-7b2c-7000-8000-000000000001"),
                Some("demo:lead"),
                vec![quota_row(
                    "weekly_scoped",
                    Some("Fable"),
                    "80.0",
                    9_880,
                    crate::quota::Status::Stale,
                )],
            )],
            10_000,
        );
        let sample = &super::quota_samples(&observation)[0];
        assert_eq!(
            observation.state_line(
                sample.group,
                &crate::quota::Classified::first(sample.reading.clone()),
                Path::new("/m/demo")
            ),
            "quota: codex · cx · demo:lead weekly_scoped Fable 5h 80% (low, observed 2m ago), resets in 1h00m — prefer another client for new spawns; table: /m/demo/quota"
        );

        let reset = quota_for("74", 9_902);
        assert_eq!(
            reset
                .transition(&reset.groups[0], &quota_classified(&reset.groups[0]))
                .render(Path::new("/m/demo"), reset.now),
            "quota: codex · cx · demo:lead codex pro 5h 74% (headroom, observed 1m ago), resets in 1h00m — back to headroom; table: /m/demo/quota"
        );

        let mut precise = observation.clone();
        precise.groups[0].rows[0].used_percent = Some(format!("80.{}", "1".repeat(300)));
        let line = precise.state_line(
            &precise.groups[0],
            &quota_classified(&precise.groups[0]),
            Path::new("/m"),
        );
        assert!(line.contains(" 80.1% (low,"), "{line}");
        assert!(line.len() < 240, "numeric display stayed bounded: {line}");

        let mut hostile = observation.clone();
        hostile.groups[0].rows[0].bucket = format!("bad\u{1b}[2J\r\n{}", "x".repeat(300));
        hostile.groups[0].rows[0].qualifier = Some("q\u{7f}\n".repeat(100));
        let sample = &super::quota_samples(&hostile)[0];
        let line = hostile.state_line(
            sample.group,
            &crate::quota::Classified::first(sample.reading.clone()),
            Path::new("/m"),
        );
        assert!(!line.contains('\u{1b}'));
        assert!(!line.contains('\r'));
        assert!(!line.contains('\n'));
        assert!(line.len() < 240, "hostile labels stayed bounded: {line}");
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one watchdog identity, expiry and rendered/groups divergence story"
    )]
    fn throttle_quota_uses_worst_exact_recorded_row_and_first_cycle_only() {
        let rollout = "018f1f70-7b2c-7000-8000-000000000001";
        let mut entry = RosterEntry {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: Some("changed-profile".to_owned()),
            client: RecordedClient::Missing,
            harness_session: Some(rollout.to_owned()),
            config_home: RecordedConfigHome::Path(PathBuf::from("/tmp/cx")),
            config_home_base: RecordedConfigHomeBase::Missing,
            binary: Some("codex".to_owned()),
        };
        let identity = crate::quota::recorded_identity(&entry).expect("recorded identity");
        let mut observation = quota_observation(
            vec![quota_group(
                &identity.source,
                Some(rollout),
                Some("demo:lead"),
                vec![
                    quota_row("z", None, "96", 9_900, crate::quota::Status::Fresh),
                    quota_row("a", None, "96", 9_900, crate::quota::Status::Fresh),
                    quota_row("b", None, "85", 9_900, crate::quota::Status::Fresh),
                ],
            )],
            10_000,
        );
        let mirrored_rendering = observation.clone();
        // Production Codex observations may carry a completeness summary in
        // `rendered` which is absent from `groups`. The watchdog consumes only
        // groups; settings may read that additive rendering metadata without
        // changing this output.
        observation.rendered.clear();
        let mut quota = QuotaCarry::default();
        let _ = quota.reconcile(&observation, &[], Path::new("/m"));
        let line = throttle_quota_line(
            &observation,
            &quota.tracked,
            &entry,
            Path::new("/m"),
            10_000,
        )
        .expect("exact row");
        assert_eq!(
            line,
            throttle_quota_line(
                &mirrored_rendering,
                &quota.tracked,
                &entry,
                Path::new("/m"),
                10_000,
            )
            .expect("the same exact row"),
            "rendered/groups divergence is invisible to watchdog output"
        );
        assert!(
            line.contains(" a 5h 96% (critical,"),
            "tie picks key ascending: {line}"
        );
        assert!(
            throttle_quota_line(
                &observation,
                &quota.tracked,
                &entry,
                Path::new("/m"),
                13_501,
            )
            .is_none(),
            "the last observation expires against the current cycle clock"
        );

        let mut observed = seen();
        observed.throttle = Some(Throttle::Throttled);
        observed.throttle_quota = Some(line.clone());
        let first = account(&PaneState::default(), &observed, &Knobs::default());
        assert_eq!(emitted(&first.effects)[0].0, "throttled");
        assert!(emitted(&first.effects)[0].1.contains(&line));
        let second = account(&first.next, &observed, &Knobs::default());
        assert!(
            emitted(&second.effects).is_empty(),
            "later cycles do not repeat it"
        );

        entry.harness_session = Some("018f1f70-7b2c-7000-8000-000000000002".to_owned());
        assert!(
            throttle_quota_line(
                &observation,
                &quota.tracked,
                &entry,
                Path::new("/m"),
                10_000,
            )
            .is_some(),
            "another conversation under the same config home reads the same window"
        );
        entry.harness_session = None;
        assert!(
            throttle_quota_line(
                &observation,
                &quota.tracked,
                &entry,
                Path::new("/m"),
                10_000,
            )
            .is_none(),
            "a Codex seat with no recorded conversation is not a proven identity"
        );
        entry.harness_session = Some(rollout.to_owned());
        entry.config_home = RecordedConfigHome::Missing;
        assert!(
            throttle_quota_line(
                &observation,
                &quota.tracked,
                &entry,
                Path::new("/m"),
                10_000,
            )
            .is_none()
        );
    }

    #[test]
    fn throttle_quota_never_falls_back_from_the_recorded_identity() {
        let rollout = "018f1f70-7b2c-7000-8000-000000000001";
        let entry = RosterEntry {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: Some("changed-profile".to_owned()),
            client: RecordedClient::Missing,
            harness_session: Some(rollout.to_owned()),
            config_home: RecordedConfigHome::Path(PathBuf::from("/tmp/cx-recorded")),
            config_home_base: RecordedConfigHomeBase::Missing,
            binary: Some("codex".to_owned()),
        };
        let recorded = crate::quota::recorded_identity(&entry).expect("recorded identity");
        let current_source = Path::new("/tmp/cx-current/sessions");
        let current = quota_group(
            current_source,
            Some(rollout),
            None,
            vec![quota_row(
                "codex",
                None,
                "99",
                9_900,
                crate::quota::Status::Fresh,
            )],
        );
        let recorded_group = quota_group(
            &recorded.source,
            Some(rollout),
            None,
            vec![quota_row(
                "codex",
                None,
                "85",
                9_900,
                crate::quota::Status::Fresh,
            )],
        );
        let observation = quota_observation(vec![current.clone(), recorded_group], 10_000);
        let line = throttle_quota_line(&observation, &[], &entry, Path::new("/m"), 10_000)
            .expect("recorded source row");
        assert!(line.contains(" 85% (low,"), "{line}");

        assert!(
            throttle_quota_line(
                &quota_observation(vec![current], 10_000),
                &[],
                &entry,
                Path::new("/m"),
                10_000
            )
            .is_none(),
            "current profile source never replaces an absent recorded row"
        );
        for status in [
            crate::quota::Status::Unknown,
            crate::quota::Status::ReadError,
            crate::quota::Status::Truncated,
        ] {
            let silent = quota_observation(
                vec![quota_group(
                    &recorded.source,
                    Some(rollout),
                    None,
                    vec![quota_row("codex", None, "99", 9_900, status)],
                )],
                10_000,
            );
            assert!(
                throttle_quota_line(&silent, &[], &entry, Path::new("/m"), 10_000).is_none(),
                "{status:?} row must stay silent"
            );
        }

        let mut default_home = entry.clone();
        default_home.config_home = RecordedConfigHome::Missing;
        assert!(
            throttle_quota_line(
                &quota_observation(
                    vec![quota_group(
                        Path::new("/home/test/.codex/sessions"),
                        Some(rollout),
                        None,
                        vec![quota_row(
                            "codex",
                            None,
                            "99",
                            9_900,
                            crate::quota::Status::Fresh,
                        )],
                    )],
                    10_000,
                ),
                &[],
                &default_home,
                Path::new("/m"),
                10_000
            )
            .is_none(),
            "default home is never an identity fallback"
        );
    }

    #[test]
    fn throttle_identity_requires_the_recorded_home_mode_and_base() {
        let entry = |binary: &str,
                     home: RecordedConfigHome,
                     base: RecordedConfigHomeBase,
                     session: Option<&str>| RosterEntry {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: Some("changed".to_owned()),
            client: RecordedClient::Missing,
            harness_session: session.map(str::to_owned),
            config_home: home,
            config_home_base: base,
            binary: Some(binary.to_owned()),
        };
        let explicit = crate::quota::recorded_identity(&entry(
            "claude",
            RecordedConfigHome::Path(PathBuf::from("/tmp/claude-explicit")),
            RecordedConfigHomeBase::Missing,
            None,
        ))
        .expect("explicit Claude identity");
        assert!(explicit.source.ends_with("claude-explicit/.claude.json"));
        let implicit = crate::quota::recorded_identity(&entry(
            "claude",
            RecordedConfigHome::Implicit(PathBuf::from("/tmp/person/.claude")),
            RecordedConfigHomeBase::Path(PathBuf::from("/tmp/person")),
            None,
        ))
        .expect("implicit Claude identity");
        assert!(implicit.source.ends_with("person/.claude.json"));
        assert!(
            crate::quota::recorded_identity(&entry(
                "claude",
                RecordedConfigHome::Implicit(PathBuf::from("/tmp/person/.claude")),
                RecordedConfigHomeBase::Missing,
                None,
            ))
            .is_none(),
            "an incomplete implicit identity must not guess the default home"
        );
    }

    /// A ranked listing row in the default look — the shape the ticker reads.
    fn listed(name: &str, id: &str, rank: &str) -> crate::tmux::FleetListingRow {
        crate::tmux::FleetListingRow {
            name: name.to_owned(),
            id: id.to_owned(),
            rank: Some(rank.to_owned()),
            look: crate::tmux::LookOptions::default(),
        }
    }

    /// The same row for a session that published NO rank.
    fn unranked(name: &str, id: &str) -> crate::tmux::FleetListingRow {
        crate::tmux::FleetListingRow {
            rank: None,
            ..listed(name, id, "0")
        }
    }

    fn motion(pane_id: &str, agent: &str) -> crate::tmux::MotionPane {
        crate::tmux::MotionPane {
            pane_id: pane_id.to_owned(),
            agent: Some(agent.to_owned()),
            session_attached: 1,
        }
    }

    #[test]
    fn every_working_agent_spins_each_tick_and_other_marks_never_do() {
        let current = [
            motion("%1", "active"),
            motion("%2", "done"),
            motion("%3", "blocked"),
            motion("%4", "dead"),
            motion("%5", "idle"),
            motion("%6", "sweeping"),
            motion("%7", "_watchdog"),
        ];
        let mut state = MotionState {
            verdicts: vec![
                MotionVerdict {
                    pane: "%1".to_owned(),
                    window: "@7".to_owned(),
                    verdict: Verdict::Active,
                },
                MotionVerdict {
                    pane: "%2".to_owned(),
                    window: "@7".to_owned(),
                    verdict: Verdict::Quiet(QuietKind::Done),
                },
                MotionVerdict {
                    pane: "%3".to_owned(),
                    window: "@7".to_owned(),
                    verdict: Verdict::Quiet(QuietKind::Blocked),
                },
                MotionVerdict {
                    pane: "%4".to_owned(),
                    window: "@7".to_owned(),
                    verdict: Verdict::Dead,
                },
                MotionVerdict {
                    pane: "%6".to_owned(),
                    window: "@7".to_owned(),
                    verdict: Verdict::Meta(SweepVerdict::MetaSweeping),
                },
            ],
            panes: current.to_vec(),
            ..MotionState::default()
        };
        let first = crate::tmux::set_options_args(&ServerId::Ambient, &state.step(&Look::DEFAULT));
        assert_eq!(
            first,
            [
                "set-option",
                "-p",
                "-t",
                "%1",
                "@ae_pane_state",
                "#[fg=#537187]●#[default] working",
                ";",
                "set-option",
                "-p",
                "-t",
                "%6",
                "@ae_pane_state",
                "#[fg=#537187]●#[default] sweeping",
                ";",
                "set-option",
                "-w",
                "-t",
                "@7",
                "@ae_window_agents",
                "#[fg=#537187]●#[default]active #[fg=#6A8759]✓#[default]done #[fg=#CC7832]⚠#[default]blocked #[fg=#FF6B68]✖#[default]dead #[fg=#537187]●#[default]sweeping",
            ]
        );
        let second = crate::tmux::set_options_args(&ServerId::Ambient, &state.step(&Look::DEFAULT));
        assert!(second.iter().any(|word| word.contains('●')));
        assert!(
            second
                .iter()
                .all(|word| !matches!(word.as_str(), "%2" | "%3" | "%4" | "%5" | "%7"))
        );
    }

    /// The ticker animates the WORKING mark only: `Active` and
    /// `MetaSweeping` share the glyph, and the word after it is the VERDICT's —
    /// a literal "working" written over both repainted a pane that had just
    /// published `● sweeping` as `● working` a hundred milliseconds later, and
    /// made two panes with different observed values render identically. A
    /// fresh `Quiet(WaitingAgent)` is NOT repainted: its seventh mark is
    /// static, so the published `◔ waiting-agent` stands.
    #[test]
    fn a_ticked_pane_keeps_the_word_its_own_verdict_declares() {
        let mut state = MotionState {
            verdicts: vec![
                MotionVerdict {
                    pane: "%1".to_owned(),
                    window: "@7".to_owned(),
                    verdict: Verdict::Active,
                },
                MotionVerdict {
                    pane: "%2".to_owned(),
                    window: "@7".to_owned(),
                    verdict: Verdict::Quiet(QuietKind::WaitingAgent),
                },
                MotionVerdict {
                    pane: "%3".to_owned(),
                    window: "@7".to_owned(),
                    verdict: Verdict::Meta(SweepVerdict::MetaSweeping),
                },
            ],
            panes: vec![
                motion("%1", "lead"),
                motion("%2", "colead"),
                motion("%3", "orchestrator"),
            ],
            ..MotionState::default()
        };
        let args = crate::tmux::set_options_args(&ServerId::Ambient, &state.step(&Look::DEFAULT));
        for expected in [
            "#[fg=#537187]●#[default] working",
            "#[fg=#537187]●#[default] sweeping",
        ] {
            assert!(
                args.iter().any(|word| word == expected),
                "missing {expected:?} in {args:?}"
            );
        }
        assert!(
            !args.iter().any(|word| word == "%2"),
            "the waiting-agent pane is never repainted by the ticker: {args:?}"
        );
    }

    #[test]
    fn detached_sessions_never_write_working_frames() {
        let mut pane = motion("%1", "lead");
        pane.session_attached = 0;
        let mut state = MotionState {
            verdicts: vec![MotionVerdict {
                pane: "%1".to_owned(),
                window: "@7".to_owned(),
                verdict: Verdict::Active,
            }],
            panes: vec![pane],
            ..MotionState::default()
        };
        assert!(state.step(&Look::DEFAULT).is_empty());
        assert_eq!(state.spin, 0, "an invisible ticker does not advance");
    }

    #[test]
    fn ticker_cadence_follows_visibility_and_both_opt_outs_disable_it() {
        let attached = motion("%1", "lead");
        let mut detached = attached.clone();
        detached.session_attached = 0;
        assert_eq!(motion_cadence(&[attached]), Duration::from_millis(100));
        assert_eq!(motion_cadence(&[detached]), Duration::from_secs(2));
        assert!(motion_ticker_enabled(&Look::DEFAULT));
        assert!(!motion_ticker_enabled(&Look {
            motion: false,
            ..Look::DEFAULT
        }));
        assert!(!motion_ticker_enabled(&Look {
            drawn: false,
            ..Look::DEFAULT
        }));
    }

    #[test]
    fn attached_observation_is_every_fifth_motion_tick() {
        assert_eq!(super::MOTION_OBSERVATION_TICKS, 5);
        let mut ticks_since_observation = super::MOTION_OBSERVATION_TICKS;
        let observed: Vec<u8> = (0..=10)
            .filter(|_| {
                let due = motion_observation_due(ticks_since_observation);
                if due {
                    ticks_since_observation = 0;
                }
                ticks_since_observation = ticks_since_observation.saturating_add(1);
                due
            })
            .collect();
        assert_eq!(observed, [0, 5, 10]);
    }

    #[test]
    fn a_failed_cached_write_refreshes_without_spending_the_failure_budget() {
        let (failures, stop) = motion_publish_failure(2, false);
        assert_eq!(failures, 2);
        assert!(!stop);
        assert!(motion_observation_due(super::MOTION_OBSERVATION_TICKS));

        let (failures, stop) = motion_publish_failure(failures, true);
        assert_eq!(failures, 3);
        assert!(
            stop,
            "the same failure against fresh identities still stops"
        );
    }

    #[test]
    fn fleet_strip_writes_only_for_changed_text_or_working_frames() {
        let session = |rank: &str| listed("current", "$7", rank);
        let mut state = MotionState::default();
        state.replace_observation(vec![motion("%1", "lead")], &[session("1")], &[], "current");

        let first = state.step(&Look::DEFAULT);
        assert_eq!(first.len(), 1, "the first static strip is new");
        assert!(
            state.step(&Look::DEFAULT).is_empty(),
            "unchanged static strip"
        );

        state.replace_fleet(&[session("4")], &[], "current");
        assert_eq!(state.step(&Look::DEFAULT).len(), 1, "changed rank");
        assert!(state.step(&Look::DEFAULT).is_empty(), "unchanged attention");

        state.replace_fleet(&[session("2")], &[], "current");
        let first_frame =
            crate::tmux::set_options_args(&ServerId::Ambient, &state.step(&Look::DEFAULT));
        let next_frame =
            crate::tmux::set_options_args(&ServerId::Ambient, &state.step(&Look::DEFAULT));
        assert!(first_frame.iter().any(|word| word.contains('●')));
        assert!(next_frame.iter().any(|word| word.contains('●')));
        assert_ne!(first_frame, next_frame, "pulse colour changes each tick");
    }

    #[test]
    fn a_session_with_no_row_of_its_own_still_owns_no_strip() {
        let rows = [unranked("current", "$7"), listed("peer", "$8", "1")];
        let mut state = MotionState::default();
        state.replace_observation(vec![motion("%1", "lead")], &rows, &[], "current");
        assert!(
            state.step(&Look::DEFAULT).is_empty(),
            "a rankless session ae cannot vouch for draws no row, so it owns no strip"
        );

        state.replace_fleet(&rows, &["current".to_owned()], "current");
        assert_eq!(
            state.step(&Look::DEFAULT).len(),
            1,
            "vouched for, it draws itself Stale and owns the strip again"
        );
    }

    #[test]
    fn motion_off_takes_the_static_observer_not_the_whole_sleep() {
        let still = Look::read("", "", "", "off");
        let undrawn = Look::read("", "", "off", "");
        assert_eq!(ticker_mode(Some(&Look::DEFAULT)), TickerMode::Animated);
        assert_eq!(ticker_mode(Some(&still)), TickerMode::Static);
        assert_eq!(ticker_mode(Some(&undrawn)), TickerMode::Idle);
        assert_eq!(ticker_mode(None), TickerMode::Idle);
    }

    #[test]
    fn motion_off_republishes_a_changed_fleet_statically_and_writes_nothing_when_still() {
        let session = |rank: &str| listed("current", "$7", rank);
        let mut state = MotionState::default();
        state.replace_observation(vec![motion("%1", "lead")], &[session("1")], &[], "current");

        assert_eq!(state.step_static(&Look::DEFAULT).len(), 1, "first strip");
        assert!(
            state.step_static(&Look::DEFAULT).is_empty(),
            "unchanged strip"
        );

        state.replace_fleet(&[session("4")], &[], "current");
        assert_eq!(state.step_static(&Look::DEFAULT).len(), 1, "changed rank");
        assert!(
            state.step_static(&Look::DEFAULT).is_empty(),
            "unchanged attention"
        );

        state.replace_fleet(&[session("2")], &[], "current");
        assert_eq!(
            state.step_static(&Look::DEFAULT).len(),
            1,
            "working row, once"
        );
        assert!(
            state.step_static(&Look::DEFAULT).is_empty(),
            "no per-tick frame"
        );
        let expected =
            crate::theme::fleet_strip(&Look::DEFAULT, &state.fleet, None, &state.fleet_order);
        assert_eq!(state.published_fleet.as_deref(), Some(expected.as_str()));

        let mut detached = motion("%1", "lead");
        detached.session_attached = 0;
        state.replace_observation(vec![detached], &[session("4")], &[], "current");
        assert!(
            state.step_static(&Look::DEFAULT).is_empty(),
            "detached: no viewer"
        );
    }

    #[test]
    fn motion_off_never_ticks_faster_than_the_observation_cadence() {
        let attached = [motion("%1", "lead")];
        assert_eq!(
            static_observe_cadence(&attached),
            super::ATTACHED_MOTION_TICK.saturating_mul(u32::from(super::MOTION_OBSERVATION_TICKS)),
        );
        let mut pane = motion("%1", "lead");
        pane.session_attached = 0;
        let detached = [pane];
        assert_eq!(
            static_observe_cadence(&detached),
            super::DETACHED_MOTION_TICK
        );
    }

    #[test]
    fn current_orchestrator_publishes_its_segment_and_keeps_it_on_ticker() {
        let session = |name: &str, id: &str, rank: &str| listed(name, id, rank);
        let mut state = MotionState::default();
        state.replace_observation(
            vec![motion("%1", "lead")],
            &[
                session("worker", "$4", "1"),
                session("orchestrator", "$7", "0"),
            ],
            &[],
            "orchestrator",
        );

        let first = crate::tmux::set_options_args(&ServerId::Ambient, &state.step(&Look::DEFAULT));
        assert!(
            first
                .iter()
                .any(|word| word == crate::theme::ORCHESTRATOR_STRIP_OPTION),
            "orchestrator option is published: {first:?}"
        );
        assert!(
            first
                .iter()
                .any(|word| { word.contains("bg=#214283") && word.contains("range=session|$7") }),
            "current orchestrator segment is published: {first:?}"
        );
        assert!(
            first
                .iter()
                .any(|word| word == crate::theme::FLEET_STRIP_OPTION),
            "fleet strip still publishes separately: {first:?}"
        );
        assert!(
            !state
                .published_fleet
                .as_deref()
                .is_some_and(|strip| strip.contains("orchestrator")),
            "fleet strip excludes pinned orchestrator: {:?}",
            state.published_fleet
        );
        let second = state.step(&Look::DEFAULT);
        assert!(
            second.is_empty(),
            "unchanged current segment stays cached: {second:?}"
        );
    }

    /// PIN: the ticker DRAWS the strip at motion cadence and reads no config to
    /// do it — it reuses the order the verdict cycle stored. Without that its
    /// frames would publish the unordered strip over the ordered one and the
    /// fleet would flap; reading the file itself would open it ten times a
    /// second. An observation refreshes WHO is on the server and nothing else.
    #[test]
    fn the_ticker_keeps_the_order_the_verdict_cycle_stored() {
        let sessions = [listed("alpha", "$1", "0"), listed("beta", "$2", "0")];
        let order =
            crate::theme::FleetOrder::from_validated(vec!["beta".to_owned(), "alpha".to_owned()]);
        let mut state = MotionState::default();
        // The verdict cycle: fleet AND order.
        state.replace_fleet(&sessions, &[], "alpha");
        state.set_fleet_order(&order);
        let mut writes = Vec::new();
        state.push_fleet_write(&mut writes, &Look::DEFAULT, None);
        let cycle_strip = state.published_fleet.clone().expect("a published strip");
        assert!(
            cycle_strip.find("beta") < cycle_strip.find("alpha"),
            "the cycle drew the human's order: {cycle_strip}"
        );
        // A ticker observation — panes and fleet, never an order.
        state.replace_observation(Vec::new(), &sessions, &[], "alpha");
        assert_eq!(state.fleet_order, order, "the observation left it alone");
        let mut writes = Vec::new();
        state.push_fleet_write(&mut writes, &Look::DEFAULT, None);
        assert!(
            writes.is_empty(),
            "same order, same text, no write: {writes:?}"
        );
    }

    #[test]
    fn verdict_cycle_current_orchestrator_publishes_segment_and_excludes_fleet_row() {
        let sessions = [
            listed("worker", "$4", "1"),
            listed("orchestrator", "$7", "0"),
        ];
        let mut state = MotionState::default();
        state.replace_fleet(&sessions, &[], "orchestrator");
        let mut writes = Vec::new();
        state.push_fleet_write(&mut writes, &Look::DEFAULT, None);
        let row = state
            .fleet
            .iter()
            .find(|row| row.name == crate::orchestrator::ORCHESTRATOR_SESSION)
            .cloned();
        assert!(state.push_orchestrator_strip_write(
            &mut writes,
            &Look::DEFAULT,
            "$7",
            row.as_ref(),
            None,
        ));
        let args = crate::tmux::set_options_args(&ServerId::Ambient, &writes);
        assert!(
            args.iter()
                .any(|word| word == crate::theme::ORCHESTRATOR_STRIP_OPTION)
        );
        assert!(
            args.iter()
                .any(|word| { word.contains("bg=#214283") && word.contains("range=session|$7") })
        );
        assert!(
            !state
                .published_fleet
                .as_deref()
                .is_some_and(|strip| strip.contains("orchestrator"))
        );
        assert!(!matches!(
            state.published_orchestrator_strip,
            super::PublishedOrchestratorStrip::Unset
        ));
    }

    #[test]
    fn orchestrator_id_targets_other_sessions_only() {
        let fleet = [
            listed("worker", "$4", "2"),
            listed("orchestrator", "$7", "1"),
        ];
        assert_eq!(super::orchestrator_id_for(&fleet, "worker"), Some("$7"));
        assert_eq!(
            super::orchestrator_id_for(&fleet, "orchestrator"),
            None,
            "the orchestrator has nowhere else to jump"
        );
        assert_eq!(
            super::orchestrator_id_for(&fleet[..1], "worker"),
            None,
            "a fleet without an orchestrator leaves the option unset"
        );
    }

    #[test]
    fn an_unchanged_orchestrator_target_never_writes_or_clears_again() {
        let mut state = MotionState::default();
        let mut writes = Vec::new();
        assert!(state.push_orchestrator_id_write(&mut writes, "$4", Some("$7")));
        assert_eq!(writes.len(), 1);
        assert!(!state.push_orchestrator_id_write(&mut writes, "$4", Some("$7")));
        assert_eq!(writes.len(), 1, "unchanged target is a no-op");

        let mut transitions = MotionState::default();
        let mut writes = Vec::new();
        assert!(transitions.push_orchestrator_id_write(&mut writes, "$4", Some("$7")));
        writes.clear();
        assert!(transitions.push_orchestrator_id_write(&mut writes, "$4", None));
        assert_eq!(writes.len(), 0, "an unset target has no set write");
        assert!(!transitions.push_orchestrator_id_write(&mut writes, "$4", None));
        assert_eq!(writes.len(), 0, "an unset target has no set write");
        assert!(transitions.push_orchestrator_id_write(&mut writes, "$4", Some("$9")));
        assert_eq!(writes.len(), 1, "a new target gets one set write");
    }

    #[test]
    fn orchestrator_strip_publishes_only_when_present() {
        let session = |name: &str, id: &str, rank: &str| listed(name, id, rank);
        let mut with = MotionState::default();
        with.replace_fleet(
            &[
                session("worker", "$4", "2"),
                session("orchestrator", "$7", "3"),
            ],
            &[],
            "worker",
        );
        let mut writes = Vec::new();
        let row = with
            .fleet
            .iter()
            .find(|row| row.name == crate::orchestrator::ORCHESTRATOR_SESSION)
            .cloned();
        assert!(with.push_orchestrator_strip_write(
            &mut writes,
            &Look::DEFAULT,
            "$4",
            row.as_ref(),
            None,
        ));
        let args = crate::tmux::set_options_args(&ServerId::Ambient, &writes);
        assert!(
            args.iter()
                .any(|word| word == crate::theme::ORCHESTRATOR_STRIP_OPTION)
        );
        assert_eq!(
            with.published_orchestrator_strip,
            super::PublishedOrchestratorStrip::Value(crate::theme::orchestrator_strip(
                &Look::DEFAULT,
                row.as_ref().expect("orchestrator row is in the fleet"),
                None,
            )),
            "the published value is the theme's segment for the found row",
        );

        let mut without = MotionState::default();
        without.replace_fleet(&[session("worker", "$4", "2")], &[], "worker");
        let mut writes = Vec::new();
        assert!(without.push_orchestrator_strip_write(
            &mut writes,
            &Look::DEFAULT,
            "$4",
            None,
            None,
        ));
        assert!(writes.is_empty(), "caller performs the one required unset");
    }

    // -----------------------------------------------------------------------
    // ADOPTION: the fourth writer of the look, and the only one that writes
    // into a session other than its own.
    // -----------------------------------------------------------------------

    /// A classified candidate, as the adoption scan hands it on.
    fn candidate(name: &str, status: crate::digest::Status) -> crate::liveness::Classified {
        crate::liveness::Classified {
            candidate: crate::inventory::Candidate::durable(crate::inventory::DurableRecord {
                name: name.to_owned(),
                path: PathBuf::from("/state/sessions").join(name),
                layout: crate::inventory::Layout::Canonical,
                server: crate::meta::ServerSelector::Positive(crate::meta::Selector::Name(
                    "ae".to_owned(),
                )),
                meta_read: crate::inventory::MetaRead::Parsed,
                snapshot: crate::session::RecordSnapshot::default(),
            }),
            status,
        }
    }

    /// PIN a: who gets adopted, and who never does.
    ///
    /// Every skip here is a session this daemon would otherwise be publishing a
    /// display fact into without having proved it may.
    #[test]
    fn adoption_takes_running_unwatched_peers_and_nothing_else() {
        let listing = [
            listed("self", "$1", "2"),
            listed("unwatched", "$2", "3"),
            listed("watched", "$3", "2"),
            listed("unprovable", "$4", "2"),
            unranked("leftover", "$5"),
            listed("stopped", "$6", "0"),
            listed("stranger", "$9", "2"),
        ];
        let classified = vec![
            // Proven ae-owned and running, but this daemon's own session.
            candidate("self", crate::digest::Status::Running),
            candidate("unwatched", crate::digest::Status::Running),
            candidate("watched", crate::digest::Status::Running),
            candidate("unprovable", crate::digest::Status::Running),
            candidate("leftover", crate::digest::Status::Running),
            // The classifier already refused these two: a stopped session, and
            // one on a server this daemon could not prove is its own.
            candidate("stopped", crate::digest::Status::Stopped),
            candidate("elsewhere", crate::digest::Status::Unknown),
            // Running and proven, but the server is not showing it right now.
            candidate("vanished", crate::digest::Status::Running),
        ];
        let adoption = adoption_from(
            classified,
            "self",
            &listing,
            &Adoption::default(),
            |meta_dir| match meta_dir.file_name().and_then(std::ffi::OsStr::to_str) {
                Some("watched") => WatchdogPresence::Live,
                Some("unprovable") => WatchdogPresence::Unknown,
                Some("leftover") => WatchdogPresence::Absent(Some(4242)),
                _ => WatchdogPresence::Absent(None),
            },
        );
        assert_eq!(
            adoption
                .targets
                .iter()
                .map(|target| (target.name.as_str(), target.id.as_str()))
                .collect::<Vec<_>>(),
            vec![("unwatched", "$2"), ("leftover", "$5")],
            "a live watchdog, an unprovable one, a stopped or foreign session \
             and a name the server is not showing are all skipped"
        );
        assert_eq!(
            adoption.targets[1].enum_pid,
            Some(4242),
            "the pid seen at enumeration travels with the target"
        );
        // `known` is the wider set: every RUNNING peer ae's records vouch for,
        // adopted or not, because a rankless one of them still has to be drawn.
        assert_eq!(
            adoption.known,
            vec!["unwatched", "watched", "unprovable", "leftover", "vanished"],
            "and this daemon's own session is in neither list"
        );
    }

    /// PIN a: nothing is written into a peer that has not proved BOTH halves of
    /// its ownership.
    ///
    /// The marker alone is not enough — it says "an ae session", not "an ae
    /// session of THIS fleet" — and the name is not evidence at all. A peer that
    /// fails either half classifies `unknown`, which is never a target.
    #[test]
    fn a_peer_proves_an_ae_marker_and_this_state_root_or_it_is_not_adopted() {
        let root = Path::new("/state");
        let owned = |marker: &str, home: &str| crate::transport::SessionOwnership {
            marker: marker.to_owned(),
            home: home.to_owned(),
        };
        assert_eq!(
            proven_ownership(Some(&owned("1", "/state")), root),
            Some("1".to_owned()),
            "an ae marker and this state root"
        );
        assert_eq!(
            proven_ownership(Some(&owned("", "/state")), root),
            None,
            "an empty marker is not an ae session"
        );
        assert_eq!(
            proven_ownership(Some(&owned("1", "/other")), root),
            None,
            "another state root's session belongs to another fleet"
        );
        assert_eq!(
            proven_ownership(None, root),
            None,
            "and a read that did not answer proves nothing"
        );
        // `positively_owned` is what the classifier then asks of the marker, so
        // the two halves cannot drift apart into "proved here, refused there".
        assert!(crate::liveness::positively_owned(
            "peer",
            proven_ownership(Some(&owned("1", "/state")), root).as_deref()
        ));
        assert!(!crate::liveness::positively_owned(
            "peer",
            proven_ownership(Some(&owned("1", "/other")), root).as_deref()
        ));
    }

    /// PIN a: the scan's backend answers THIS server and no other.
    ///
    /// A candidate recorded elsewhere must come back `unknown`, never
    /// `running`, or a session on another tmux server could be written into.
    #[test]
    fn the_adoption_backend_refuses_every_server_but_its_own() {
        use crate::inventory::Discovery as _;

        let sockets = crate::SocketPaths::asking(|_| None);
        let mine = ServerId::Selected(crate::meta::Selector::Name("ae".to_owned()));
        let proven = [crate::inventory::DiscoveredSession {
            name: "peer".to_owned(),
            marker: Some("1".to_owned()),
        }];
        let backend = AdoptionBackend {
            server: &mine,
            sockets: &sockets,
            proven: &proven,
        };
        assert!(
            backend.enumerate(&mine).is_ok(),
            "the daemon's own server answers"
        );
        assert!(
            backend
                .enumerate(&ServerId::Selected(crate::meta::Selector::Name(
                    "other".to_owned()
                )))
                .is_err(),
            "an unproven spelling is a FAILED query, which is `unknown` and never a target"
        );
    }

    /// PIN b: the adopted strip is the TARGET's, in every respect.
    ///
    /// Drawn in the target's look, with the target as the current row, static,
    /// and in the fleet order every writer shares. Rendering it in the
    /// ADOPTER's look would hand a session running the ASCII fallback a
    /// braille glyph, and marking the adopter current would draw a map with the
    /// "you are here" pin in the wrong place.
    #[test]
    fn an_adopted_strip_is_drawn_in_the_targets_look_with_the_target_current() {
        let listing = [
            crate::tmux::FleetListingRow {
                look: crate::tmux::LookOptions {
                    icons: "off".to_owned(),
                    palette: "a".to_owned(),
                    drawn: "on".to_owned(),
                    motion: "on".to_owned(),
                },
                ..listed("peer", "$2", "2")
            },
            listed("adopter", "$1", "2"),
        ];
        let mut adoption = Adoption {
            targets: vec![Adopted {
                name: "peer".to_owned(),
                id: "$2".to_owned(),
                meta_dir: PathBuf::from("/nonexistent/peer"),
                enum_pid: None,
                published: None,
            }],
            known: vec!["peer".to_owned()],
        };
        let order = crate::theme::FleetOrder::from_validated(vec!["peer".to_owned()]);
        let writes = adoption_writes(&mut adoption, &listing, &order);
        let strip = adoption.targets[0]
            .published
            .clone()
            .expect("the peer's strip was composed");
        assert_eq!(writes.len(), 1, "one write, for the one target");
        assert_eq!(
            strip,
            crate::theme::fleet_strip(
                &Look {
                    palette: crate::theme::Palette::NEUTRAL,
                    icons: false,
                    ..Look::DEFAULT
                },
                &[
                    crate::theme::FleetRow {
                        name: "peer".to_owned(),
                        id: "$2".to_owned(),
                        mark: Mark::Working,
                        current: true,
                    },
                    crate::theme::FleetRow {
                        name: "adopter".to_owned(),
                        id: "$1".to_owned(),
                        mark: Mark::Working,
                        current: false,
                    },
                ],
                None,
                &order,
            ),
            "the TARGET's palette and glyph set, the TARGET current, no working frame"
        );
        assert!(
            strip.contains(Mark::Working.glyph(false))
                && !strip.contains(Mark::Working.glyph(true)),
            "and the ASCII fallback the target asked for: {strip}"
        );
    }

    /// PIN b (lead condition C5): `theme = off` on the TARGET still gets a
    /// strip. ae fills `@ae_*` on an undrawn session too, so a hand-written
    /// `status-right` keeps working — refusing here would take the fleet line
    /// away from exactly the readers who built their own.
    #[test]
    fn a_target_with_the_theme_off_is_still_given_its_strip() {
        let listing = [crate::tmux::FleetListingRow {
            look: crate::tmux::LookOptions {
                drawn: "off".to_owned(),
                ..crate::tmux::LookOptions::default()
            },
            ..listed("peer", "$2", "2")
        }];
        let mut adoption = Adoption {
            targets: vec![Adopted {
                name: "peer".to_owned(),
                id: "$2".to_owned(),
                meta_dir: PathBuf::from("/nonexistent/peer"),
                enum_pid: None,
                published: None,
            }],
            known: Vec::new(),
        };
        assert_eq!(
            adoption_writes(&mut adoption, &listing, &crate::theme::FleetOrder::EMPTY).len(),
            1,
            "an undrawn session still carries the fact"
        );
    }

    /// PIN b: a RUNNING session with no rank is a row, on every strip this
    /// daemon writes — its own included.
    ///
    /// The rank rule alone cannot admit it: a session nobody measures publishes
    /// no rank, and dropping it is what hid a running session from every other
    /// session's strip. Only ae's own records may vouch for one, so a rankless
    /// session nothing vouches for stays out.
    #[test]
    fn a_rankless_session_ae_vouches_for_is_drawn_stale_everywhere() {
        let listing = [
            listed("adopter", "$1", "2"),
            unranked("leftover", "$2"),
            unranked("stranger", "$3"),
        ];
        let known = ["leftover".to_owned()];
        let own = fleet_rows(&listing, &known, "adopter");
        assert_eq!(
            own.iter()
                .map(|row| (row.name.as_str(), row.mark, row.current))
                .collect::<Vec<_>>(),
            vec![
                ("adopter", Mark::Working, true),
                ("leftover", Mark::Stale, false)
            ],
            "the adopter's OWN strip carries the Stale row, and no stranger"
        );
        let adopted = fleet_rows(&listing, &known, "leftover");
        assert_eq!(
            adopted
                .iter()
                .map(|row| (row.name.as_str(), row.mark, row.current))
                .collect::<Vec<_>>(),
            vec![
                ("adopter", Mark::Working, false),
                ("leftover", Mark::Stale, true)
            ],
            "and the adopted strip is the same rows with the pin moved"
        );
    }

    /// PIN c: the fourth writer's ENTIRE licence is one option.
    ///
    /// A rank, a glyph, a health segment or a roster written into a peer would
    /// be this daemon vouching for a session it is not measuring. This turns
    /// red the moment a second option joins the adoption batch.
    #[test]
    fn an_adopter_writes_the_fleet_strip_into_a_peer_and_nothing_else() {
        let listing = [listed("peer", "$2", "2"), listed("other", "$3", "4")];
        let mut adoption = Adoption {
            targets: vec![
                Adopted {
                    name: "peer".to_owned(),
                    id: "$2".to_owned(),
                    meta_dir: PathBuf::from("/nonexistent/peer"),
                    enum_pid: None,
                    published: None,
                },
                Adopted {
                    name: "other".to_owned(),
                    id: "$3".to_owned(),
                    meta_dir: PathBuf::from("/nonexistent/other"),
                    enum_pid: None,
                    published: None,
                },
            ],
            known: Vec::new(),
        };
        let order = crate::theme::FleetOrder::EMPTY;
        let writes = adoption_writes(&mut adoption, &listing, &order);
        assert_eq!(writes.len(), 2, "one strip each");
        for write in &writes {
            assert_eq!(
                write.option_name(),
                crate::theme::FLEET_STRIP_OPTION,
                "the adopter may publish ONE option into a session it does not own"
            );
        }
        // PIN c, second half: write-on-change, per target.
        assert!(
            adoption_writes(&mut adoption, &listing, &order).is_empty(),
            "an unchanged strip is not rewritten every tick"
        );
        let moved = [listed("peer", "$2", "2"), listed("other", "$3", "5")];
        let writes = adoption_writes(&mut adoption, &moved, &order);
        assert_eq!(
            writes.len(),
            2,
            "both strips changed, because both list `other`"
        );
        assert!(
            adoption_writes(&mut adoption, &moved, &order).is_empty(),
            "and settle again"
        );
    }

    /// PIN c: a target the current listing no longer shows under the SAME id is
    /// skipped, not written to.
    ///
    /// The id is the identity: tmux never reuses a `$<n>` while the server
    /// runs, so a name that came back on a NEW id belongs to somebody else, and
    /// a write aimed at the old one would land nowhere or, worse, on a stranger.
    #[test]
    fn a_target_that_is_no_longer_itself_is_skipped_rather_than_written_to() {
        let mut adoption = Adoption {
            targets: vec![Adopted {
                name: "peer".to_owned(),
                id: "$2".to_owned(),
                meta_dir: PathBuf::from("/nonexistent/peer"),
                enum_pid: None,
                published: None,
            }],
            known: Vec::new(),
        };
        let order = crate::theme::FleetOrder::EMPTY;
        assert!(
            adoption_writes(&mut adoption, &[listed("peer", "$9", "2")], &order).is_empty(),
            "same name, new id: not the session that was proved"
        );
        assert!(
            adoption_writes(&mut adoption, &[listed("other", "$2", "2")], &order).is_empty(),
            "same id, new name: likewise"
        );
        assert!(
            adoption_writes(&mut adoption, &[], &order).is_empty(),
            "and a session that is simply gone is left alone"
        );
    }

    /// PIN d: adoption does NOT ride the motion ticker's own frame.
    ///
    /// [`MotionState::step`] draws nothing while no client is attached — and
    /// the session being filled is exactly the one somebody IS looking at. An
    /// adopter that rode `step` would go quiet the moment its own window lost
    /// focus, which is the common case for a monitor session.
    #[test]
    fn a_detached_adopter_still_fills_its_peers_line() {
        let listing = [listed("adopter", "$1", "2"), listed("peer", "$2", "2")];
        let mut detached = motion("%1", "lead");
        detached.session_attached = 0;
        let mut state = MotionState::default();
        state.replace_observation(vec![detached], &listing, &[], "adopter");
        assert!(
            state.step(&Look::DEFAULT).is_empty(),
            "nothing of this session's OWN is drawn while nobody is attached"
        );
        let mut adoption = Adoption {
            targets: vec![Adopted {
                name: "peer".to_owned(),
                id: "$2".to_owned(),
                meta_dir: PathBuf::from("/nonexistent/peer"),
                enum_pid: None,
                published: None,
            }],
            known: Vec::new(),
        };
        assert_eq!(
            adoption_writes(&mut adoption, &listing, &crate::theme::FleetOrder::EMPTY).len(),
            1,
            "and the peer's line is filled anyway"
        );
        // The cadence is adoption's own, and it is the DETACHED one: a tick
        // this daemon's attachment could switch off is the bug above.
        assert_eq!(ADOPTION_TICK, DETACHED_MOTION_TICK);
        let mut last = None;
        let now = std::time::Instant::now();
        assert!(adoption_due(&mut last, now), "the first tick is always due");
        assert!(
            !adoption_due(&mut last, now + ADOPTION_TICK / 2),
            "and nothing in between is"
        );
        assert!(adoption_due(&mut last, now + ADOPTION_TICK));
    }

    /// PIN (navigator B2): a DEAD watchdog leaves its pidfile behind, and the
    /// pause test must not read that as an owner coming back.
    ///
    /// Pausing on mere presence would mean the sessions this feature exists for
    /// — the ones whose daemon died — are the only ones it never fills.
    #[test]
    fn a_stale_pidfile_keeps_adopting_and_only_a_new_one_pauses() {
        let scratch = std::env::temp_dir().join(format!("ae-adopt-{}", std::process::id()));
        let meta_dir = scratch.join("peer");
        assert!(
            std::fs::create_dir_all(&meta_dir).is_ok(),
            "a scratch meta dir"
        );
        let pidfile = meta_dir.join(".watchdog.pid");
        let listing = [listed("peer", "$2", "2"), listed("adopter", "$1", "2")];
        let order = crate::theme::FleetOrder::EMPTY;
        let target = |enum_pid| Adoption {
            targets: vec![Adopted {
                name: "peer".to_owned(),
                id: "$2".to_owned(),
                meta_dir: meta_dir.clone(),
                enum_pid,
                published: None,
            }],
            known: Vec::new(),
        };

        // The enumeration saw a pidfile naming a process `ps` did not list.
        assert!(
            std::fs::write(&pidfile, "4242\n").is_ok(),
            "the stale pidfile"
        );
        let mut dead = target(Some(4242));
        assert_eq!(
            adoption_writes(&mut dead, &listing, &order).len(),
            1,
            "the same pid the enumeration proved dead is a corpse, not an owner"
        );

        // A DIFFERENT pid is a daemon that started since: stop writing and let
        // it publish over us.
        assert!(std::fs::write(&pidfile, "4243\n").is_ok(), "a new daemon");
        let mut replaced = target(Some(4242));
        assert!(
            adoption_writes(&mut replaced, &listing, &order).is_empty(),
            "a pidfile the enumeration did not see means the owner is back"
        );
        let mut fresh = target(None);
        assert!(
            adoption_writes(&mut fresh, &listing, &order).is_empty(),
            "and so does one appearing where there was none"
        );

        // No pidfile at all is the ordinary unwatched session.
        assert!(std::fs::remove_file(&pidfile).is_ok(), "the pidfile goes");
        let mut none = target(None);
        assert_eq!(
            adoption_writes(&mut none, &listing, &order).len(),
            1,
            "nothing is measuring the peer, so this daemon draws its line"
        );
        // The pidfile is READ, never written: another session's state dir is
        // not this daemon's to tidy.
        assert!(std::fs::write(&pidfile, "4242\n").is_ok(), "put it back");
        let mut again = target(Some(4242));
        let _ = adoption_writes(&mut again, &listing, &order);
        assert_eq!(
            crate::watchdog_glue::read_pid(&meta_dir),
            Some(4242),
            "the adopter never cleans up a peer's stale pidfile"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// PIN f: the process table is the CYCLE's, taken once, and no tick takes
    /// one at all.
    ///
    /// `procs::snapshot` spawns `ps`. One per peer would be a process per
    /// session per tick; one per tick would still be a process every two
    /// seconds on every machine running ae, for a feature whose steady state is
    /// usually zero targets.
    #[test]
    fn adoption_never_spawns_a_process_snapshot_of_its_own() {
        let source = include_str!("watchdog_daemon.rs");
        let between = |from: &str, to: &str| {
            source
                .split_once(from)
                .and_then(|(_, tail)| tail.split_once(to))
                .map(|(body, _)| body.to_owned())
                .expect("the bounded body")
        };
        for (name, body) in [
            (
                "enumerate_adoption",
                between("fn enumerate_adoption(", "\nfn adoption_from("),
            ),
            (
                "adoption_from",
                between("fn adoption_from(", "\n/// The rows one strip draws"),
            ),
            (
                "adoption_writes",
                between("fn adoption_writes(", "\n/// Whether the adoption cadence"),
            ),
            (
                "wait_idle_between_cycles",
                between(
                    "fn wait_idle_between_cycles(",
                    "\n/// Wait for the next verdict cycle while keeping",
                ),
            ),
            (
                "wait_between_cycles",
                between(
                    "fn wait_between_cycles(",
                    "\n/// Wait for the next verdict cycle with nothing",
                ),
            ),
            (
                "wait_static_between_cycles",
                between(
                    "fn wait_static_between_cycles(",
                    "\n/// The branch publication",
                ),
            ),
        ] {
            assert!(
                !body.contains("procs::snapshot"),
                "{name} must reuse the verdict cycle's one table, never take its own"
            );
        }
        // The cycle's table is PASSED, not re-taken: one `ps` per cycle for the
        // pane verdicts and the adoption scan together.
        assert!(
            between("fn publish_fleet(", "\n    /// Per-window marks").contains("table,"),
            "publish_fleet hands the cycle's own snapshot to the scan"
        );
    }

    /// PIN f, zero-target half: a daemon that has adopted nobody does no work
    /// and asks for no write. This is the common case on every machine.
    #[test]
    fn an_adopter_with_no_targets_composes_no_write_at_all() {
        let mut none = Adoption::default();
        assert!(
            adoption_writes(
                &mut none,
                &[listed("solo", "$1", "2")],
                &crate::theme::FleetOrder::EMPTY,
            )
            .is_empty(),
            "no targets, no writes — and the caller's `writes.is_empty()` guard \
             then makes no tmux call either"
        );
        assert_eq!(none, Adoption::default(), "and nothing is remembered");
    }

    #[test]
    fn three_consecutive_motion_failures_stop_ticking() {
        let (first, stop) = motion_failure(0);
        assert_eq!((first, stop), (1, false));
        let (second, stop) = motion_failure(first);
        assert_eq!((second, stop), (2, false));
        let (third, stop) = motion_failure(second);
        assert_eq!((third, stop), (3, true));
    }

    /// A pane that has been still and silent for longer than the window.
    fn stale_pane() -> (PaneState, Observation) {
        let prior = PaneState {
            prev_hash: Some(7),
            last_hash_change: Some(0),
            ..PaneState::default()
        };
        let mut observed = seen();
        observed.last_actor_event_age_secs = 10_000;
        (prior, observed)
    }

    fn emitted(effects: &[Effect]) -> Vec<(&str, &str)> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Emit { action, summary } => Some((*action, summary.as_str())),
                _ => None,
            })
            .collect()
    }

    /// The security invariant, checked STRUCTURALLY rather than by review.
    #[test]
    fn there_is_one_delivery_site_and_it_cannot_name_another_program_or_server() {
        let whole = include_str!("watchdog_daemon.rs");
        // Only the PRODUCT half: a test that exercises the constructor is not a
        // second place the daemon can name a program.
        let source = whole
            .split(concat!("#[cfg(", "test)]"))
            .next()
            .unwrap_or(whole);
        assert_eq!(
            source.matches(concat!("transport::", "deliver(")).count(),
            1,
            "a second delivery site is a second thing to audit — BOTH nudges (stale and the \
             orchestrator's sweep prompt) route through `Cycle::deliver`, which is that one site"
        );
        assert_eq!(
            source
                .matches(concat!("SendHelper::", "for_session("))
                .count(),
            1,
            "the helper path has exactly one constructor"
        );
        assert_eq!(
            source
                .matches(concat!("HELPER_NAME: &str = ", "\"send\""))
                .count(),
            1,
            "the helper name is a literal in this file, never a read value"
        );
        // The tmux FORMAT sink: one call, and it is escaped.
        assert_eq!(
            source
                .matches(concat!("transport::", "display_message("))
                .count(),
            1,
            "a second display-message site is a second thing to escape"
        );
        assert_eq!(
            source.matches(concat!("tmux::", "format_literal(")).count(),
            1,
            "the one display-message site escapes its text"
        );
        assert_eq!(
            source
                .matches(concat!("clear_published(", "leaving, session)"))
                .count(),
            1,
            "the move retracts from the server it is LEAVING, in exactly one place — the \
             adopt's best-effort attempt"
        );
        assert_eq!(
            source
                .matches(concat!("clear_published(&", "server, session)"))
                .count(),
            2,
            "the OTHER two clears address the server still in force: the session is proven \
             gone, and the recorded selector stopped naming one server so we stop. THREE \
             paths in total, each retracting ONLY what this daemon itself published. A \
             fourth is a decision, not a detail"
        );
    }

    #[test]
    fn the_cycle_rebinds_its_server_before_it_probes_and_actually_applies_the_answer() {
        // THE WIRING OF THE PER-CYCLE REBIND, which the pure `rebind` table
        // cannot see: a decision nothing consults is a decision that does not
        // happen.
        let whole = include_str!("watchdog_daemon.rs");
        let source = whole
            .split(concat!("#[cfg(", "test)]"))
            .next()
            .unwrap_or(whole);
        assert_eq!(
            source
                .matches(concat!("rebind(&", "server, parsed.as_ref())"))
                .count(),
            1,
            "the cycle asks which server it is on, exactly once, and asks it AGAINST the \
             one in force — a rebind that cannot compare cannot tell a move from a repeat"
        );
        assert_eq!(
            source.matches(concat!("mut ", "server")).count(),
            1,
            "and there is ONE binding for it to move, not a startup pin beside a cycle copy. \
             It is the LOOP's parameter since slice A.3: `run` resolves the server, hands it \
             over by value, and keeps only the pidfile's lifetime — so the compiler forbids \
             a startup copy outliving the move rather than this guard merely counting one"
        );
        // The answer is APPLIED, and applied THROUGH the adopt — a decision
        // nothing assigns is a decision that did not happen, and a move that
        // assigns without retracting and resetting is the pair of defects the
        // re-review found. `adopt_server` takes the old server BY VALUE, so the
        // compiler already forbids keeping it; these hold the rest.
        assert_eq!(
            source
                .matches(concat!("server = ", "adopt_server("))
                .count(),
            1,
            "the move goes through the adopt, and nothing else assigns the server"
        );
        assert_eq!(
            source.matches(concat!("carry.reset(", "knobs)")).count(),
            1,
            "the adopt drops every server-scoped carry — pane ids are server-local and \
             REUSABLE, so the old server's per-pane history must not reach the new one"
        );
        let adopt = source
            .split_once("fn adopt_server(")
            .map(|(_, body)| body)
            .expect("the adopt is defined in this file");
        let retracts = adopt
            .find("retract(&leaving)")
            .expect("the adopt attempts a retraction from the server it is leaving");
        let resets = adopt
            .find(concat!("carry.reset(", "knobs)"))
            .expect("the adopt resets the server-scoped carry");
        assert!(
            retracts < resets,
            "the retraction is ATTEMPTED first, while the old server is still addressable \
             — nothing targets it after the handover, so a clear deferred is a clear lost"
        );
    }

    #[test]
    fn the_adopts_reset_is_unconditional_while_its_retraction_is_best_effort() {
        // The failure boundary, held in the SOURCE because the ordering inside
        // `adopt_server` is what the unreachable-old-server test cannot see
        // from outside.
        let whole = include_str!("watchdog_daemon.rs");
        let source = whole
            .split(concat!("#[cfg(", "test)]"))
            .next()
            .unwrap_or(whole);
        let adopt = source
            .split_once("fn adopt_server(")
            .map(|(_, body)| body)
            .expect("the adopt is defined in this file");
        let (before_reset, _) = adopt
            .split_once(concat!("carry.reset(", "knobs)"))
            .expect("the reset is found above");
        assert_eq!(
            before_reset.matches("if !retract(&leaving) {").count(),
            1,
            "the retraction-failure branch exists"
        );
        assert!(
            before_reset.contains("drop(leaving);"),
            "and the handover between them proves it has closed — the reset is \
             unconditional, because carrying another server's pane history is never the \
             better answer"
        );
    }

    #[test]
    fn the_retraction_reports_failure_from_every_path_that_can_fail() {
        let whole = include_str!("watchdog_daemon.rs");
        let source = whole
            .split(concat!("#[cfg(", "test)]"))
            .next()
            .unwrap_or(whole);
        // The reachability signal itself: `clear_published` reports FALSE from
        // every path that could not address the server, and TRUE only after it
        // has finished.
        let cleared = source
            .split_once("fn clear_published(")
            .map(|(_, body)| body.split_once("\n}\n").map_or(body, |(head, _)| head))
            .expect("clear_published is defined in this file");
        assert_eq!(
            cleared.matches("return false;").count(),
            2,
            "both unaddressable exits report failure — a missing session id, and a window \
             enumeration that did not run"
        );
        // AND the clears themselves are counted.
        assert_eq!(
            cleared
                .matches(concat!("let _ = transport::", "clear_option"))
                .count(),
            0,
            "no clear result is discarded — a discarded failure is a bar left on a server \
             nothing will target again"
        );
        assert_eq!(
            cleared
                .matches(concat!("ok &= transport::", "clear_option"))
                .count(),
            3,
            "every clear folds into the accumulator, at ALL THREE scopes (session, window \
             and pane)"
        );
        assert_eq!(
            cleared
                .matches(concat!("&& transport::", "clear_option"))
                .count(),
            0,
            "with `&=`, never `&&`: short-circuiting would skip the remaining clears after \
             the first failure and leave MORE behind than it reported"
        );
        assert!(
            cleared.trim_end().ends_with("ok"),
            "and the answer is the ACCUMULATOR, never a literal — success is claimed only \
             after every option actually came off"
        );

        assert_eq!(
            source
                .matches(concat!("verify_session_absent(&", "server"))
                .count(),
            1,
            "one liveness probe, and it reads that binding"
        );
        let rebound = source
            .find(concat!("rebind(&", "server, parsed.as_ref())"))
            .expect("the rebind call is counted above");
        let probed = source
            .find(concat!("verify_session_absent(&", "server"))
            .expect("the probe is counted above");
        assert!(
            rebound < probed,
            "the rebind must precede the probe: a probe aimed at the ABANDONED server \
             reports the session absent, and absence is the one reading that ends this \
             daemon and clears the bar — pinning would make a selector edit self-terminating"
        );
        assert!(
            !source.contains(concat!("ServerId::", "Ambient")),
            "the daemon resolves the RECORDED server or refuses — no ambient fallback"
        );
        assert_eq!(
            source.matches(concat!("action: ", "\"nudge\"")).count(),
            0,
            "the send helper emits the nudge event itself — a second one here is \
             a double emit"
        );
        assert_eq!(
            source
                .matches(concat!("(\"_AE_EVENT_", "ACTION\", action)"))
                .count(),
            1,
            "the delivery carries the three frozen env vars, and this is the one \
             that names the event the helper writes"
        );
        assert!(
            !source.contains(concat!("env::", "var(")),
            "knobs arrive as arguments; this daemon reads no environment (the \
             crate-wide clippy deny is the enforcement — this is the local guard)"
        );
    }

    #[test]
    fn the_helper_is_the_session_directorys_own_send() {
        let dir = Path::new("/home/x/.ae/sessions/demo");
        assert_eq!(
            super::SendHelper::for_session(dir).path(),
            Path::new("/home/x/.ae/sessions/demo/send")
        );
    }

    #[test]
    fn the_defaults_are_the_frozen_ones() {
        // Ae:16331-16373.
        let knobs = Knobs::default();
        assert_eq!(knobs.interval_secs, 60);
        assert_eq!(knobs.stale_secs, 900);
        assert_eq!(knobs.max_nudges, 2);
        assert_eq!(knobs.throttle_alert_cycles, 5);
        assert_eq!(knobs.undelivered_max, 3);
        assert_eq!(knobs.quiet_beat_ms, 1000);
        assert_eq!(knobs.quiet_tries, 4);
        assert_eq!(knobs.quiet_panes_per_cycle, 2);
        assert_eq!(knobs.quota_every_secs, 300);
        assert_eq!(knobs.idle_nudge_secs, 300);
    }

    #[test]
    fn precedence_is_dead_meta_declared_throttled_idle_then_legacy() {
        let mut all = seen();
        all.is_dead = true;
        all.sweep = Some(SweepObservation::new(std::time::UNIX_EPOCH, None));
        all.quiet = Some(QuietKind::Done);
        all.throttle = Some(Throttle::Throttled);
        all.harness.frame = crate::harness_state::HarnessState::Idle;
        assert_eq!(
            account(&PaneState::default(), &all, &Knobs::default()).verdict,
            Verdict::Dead
        );

        all.is_dead = false;
        let idle_due = PaneState {
            identity: Some(all.identity),
            idle_since_epoch: Some(all.now_epoch - 300),
            ..PaneState::default()
        };
        let orchestrator = account(&idle_due, &all, &Knobs::default());
        assert!(matches!(orchestrator.verdict, Verdict::Meta(_)));
        assert!(
            !orchestrator.effects.contains(&Effect::Nudge),
            "the overview sweep owns orchestrator reminders"
        );
        all.sweep = None;
        assert_eq!(
            account(&PaneState::default(), &all, &Knobs::default()).verdict,
            Verdict::Quiet(QuietKind::Done)
        );
        all.quiet = None;
        // The usage limit outranks transient throttling on the same pane.
        all.throttle = Some(Throttle::LimitReached);
        assert_eq!(
            account(&PaneState::default(), &all, &Knobs::default()).verdict,
            Verdict::Limit
        );
        all.throttle = Some(Throttle::Throttled);
        assert_eq!(
            account(&PaneState::default(), &all, &Knobs::default()).verdict,
            Verdict::Throttled
        );
        all.throttle = None;
        assert_eq!(
            account(&PaneState::default(), &all, &Knobs::default()).verdict,
            Verdict::Idle
        );
        all.harness.frame = crate::harness_state::HarnessState::Unknown;
        assert_eq!(
            account(&PaneState::default(), &all, &Knobs::default()).verdict,
            Verdict::Active
        );
    }

    #[test]
    fn idle_carry_survives_restart_but_not_pane_identity_reuse() {
        let carry = PaneState {
            identity: Some(77),
            idle_since_epoch: Some(9_700),
            nudge_count: 1,
            undelivered_streak: 2,
            ..PaneState::default()
        };
        let raw = observed_option(crate::harness_state::HarnessState::Idle, &carry);
        let mut restarted = PaneState::default();
        restore_idle(&mut restarted, &raw, 77);
        assert_eq!(restarted.identity, Some(77));
        assert_eq!(restarted.idle_since_epoch, Some(9_700));
        assert_eq!(restarted.nudge_count, 1);
        assert_eq!(restarted.undelivered_streak, 2);

        let mut reused = PaneState::default();
        restore_idle(&mut reused, &raw, 78);
        assert_eq!(reused, PaneState::default());

        let unknown_raw = observed_option(crate::harness_state::HarnessState::Unknown, &carry);
        assert_eq!(
            crate::harness_state::observed_from_option(&unknown_raw),
            crate::harness_state::HarnessState::Unknown
        );
        let mut restarted_after_noise = PaneState::default();
        restore_idle(&mut restarted_after_noise, &unknown_raw, 77);
        assert_eq!(
            restarted_after_noise.idle_since_epoch, carry.idle_since_epoch,
            "an ambiguous capture does not erase the independent episode on restart"
        );
        assert_eq!(restarted_after_noise.nudge_count, carry.nudge_count);
        assert_eq!(
            observed_option(
                crate::harness_state::HarnessState::Unknown,
                &PaneState::default()
            ),
            "unknown"
        );

        let mut observed = seen();
        observed.identity = 77;
        observed.harness.frame = crate::harness_state::HarnessState::Idle;
        assert!(
            account(&restarted, &observed, &Knobs::default())
                .effects
                .contains(&Effect::Nudge),
            "the restored 300-second clock is already due"
        );
    }

    #[test]
    fn durable_stale_attention_survives_a_daemon_restart() {
        let mut observed = seen();
        observed.harness.frame = crate::harness_state::HarnessState::Idle;
        observed.harness.durable_stale = true;
        let booked = account(&PaneState::default(), &observed, &Knobs::default());
        assert_eq!(booked.verdict, Verdict::Stale);
        assert!(booked.effects.is_empty());
    }

    /// One outstanding item, as old as `age_secs`, for the seat under test.
    fn owed(requests: usize, spawns: usize, now_epoch: i64, age_secs: i64) -> OwnWork {
        OwnWork {
            requests,
            spawns,
            oldest_epoch: Some(now_epoch - age_secs),
        }
    }

    /// I-2: a NAMED pane is not a working agent. The seat the deferral counts
    /// is the seat `ae list` shows, so the publisher's decision and the human's
    /// explanation cannot contradict each other.
    ///
    /// The UNKNOWN column is the policy, stated: a snapshot gap never removes a
    /// seat on its own, and never rescues one whose pane sits at a bare shell.
    #[test]
    fn a_pane_holds_its_seat_only_while_it_is_running_something() {
        let cases: [(&str, &str, Descendancy, bool); 6] = [
            ("a live worker", "claude", Descendancy::Present, true),
            (
                "the pane is retained but the tool has gone",
                "bash",
                Descendancy::Absent,
                false,
            ),
            (
                "a shell with an unusable snapshot is still a shell",
                "bash",
                Descendancy::Unknown,
                false,
            ),
            (
                "a running tool with an unusable snapshot keeps its seat",
                "claude",
                Descendancy::Unknown,
                true,
            ),
            (
                "a tool whose process the snapshot cannot find keeps its pane",
                "codex",
                Descendancy::Absent,
                true,
            ),
            (
                "an empty command reads as a shell",
                "",
                Descendancy::Present,
                false,
            ),
        ];
        for (why, command, descendancy, held) in cases {
            assert_eq!(holds_seat(command, descendancy), held, "{why}");
        }
    }

    /// The same three rows END TO END, through the real seat list: an
    /// enumeration of panes decides what the spawner's ledger is allowed to
    /// defer for.
    #[test]
    fn a_spawn_defers_only_while_its_seat_is_actually_held() {
        let events: Vec<Event> = [concat!(
            r#"{"ts":"2026-05-29T09:00:00Z","actor":"lead","action":"spawn","#,
            r#""target":"hand","summary":"go"}"#
        )]
        .iter()
        .map(|line| Event::parse_line(line).expect("a fixture line"))
        .collect();
        let pane = |slot: &str, agent: &str, command: &str| crate::tmux::WatchPane {
            pane_id: format!("%{slot}"),
            slot: Some(slot.to_owned()),
            agent: Some(agent.to_owned()),
            current_command: command.to_owned(),
            pane_pid: None,
            observed: String::new(),
        };
        // No process table: the pane's own foreground command is the evidence.
        let spawns = |panes: &[crate::tmux::WatchPane]| {
            let seats = held_seats(panes, None, &|_| Some("claude".to_owned()));
            crate::session::Outstanding::read(&events, "live", &seats)
                .of(crate::session::Seat {
                    session: "live",
                    slot: "main",
                    reference: "lead",
                })
                .spawns
        };
        let lead = pane("main", "lead", "claude");
        assert_eq!(
            spawns(&[lead.clone(), pane("spawned.0", "hand", "claude")]),
            1,
            "a live worker defers"
        );
        assert_eq!(
            spawns(&[lead.clone(), pane("spawned.0", "hand", "bash")]),
            0,
            "the pane is retained but the tool is gone, so it excuses nobody"
        );
        assert_eq!(spawns(&[lead]), 0, "and neither does a retired one");
    }

    /// THE pain: the seat everybody else is waiting on reads as idle on the
    /// PIXELS and is nudged for it every cycle. Same frame, same clock, two
    /// ledgers.
    #[test]
    fn an_idle_seat_waiting_on_its_own_work_is_not_nudged_for_it() {
        let knobs = Knobs::default();
        let mut observed = seen();
        observed.harness.frame = crate::harness_state::HarnessState::Idle;
        let prior = PaneState {
            identity: Some(observed.identity),
            idle_since_epoch: Some(observed.now_epoch - 300),
            ..PaneState::default()
        };

        let alone = account(&prior, &observed, &knobs);
        assert_eq!(alone.verdict, Verdict::Idle);
        assert!(
            alone.effects.contains(&Effect::Nudge),
            "a seat with an empty ledger still gets its reminder"
        );

        observed.own_work = owed(1, 0, observed.now_epoch, 120);
        let waiting = account(&prior, &observed, &knobs);
        assert_eq!(
            waiting.verdict,
            Verdict::Idle,
            "the FRAME is unchanged — this gates the nudge, not the classification"
        );
        assert!(
            !waiting.effects.contains(&Effect::Nudge),
            "one pending sent request defers it"
        );

        observed.own_work = owed(0, 1, observed.now_epoch, 120);
        assert!(
            !account(&prior, &observed, &knobs)
                .effects
                .contains(&Effect::Nudge),
            "so does one live spawn"
        );
    }

    /// S2: the deferral is bounded on both clocks, so a wedged lead is caught.
    #[test]
    fn the_deferral_ceiling_is_the_nudge_budget_or_a_generous_age() {
        let knobs = Knobs::default();
        let now = 10_000_i64;
        // idle_nudge_secs 300, max_nudges 2 → budget 900s, age cap 1200s.
        let cases: [(&str, OwnWork, u64, bool); 7] = [
            (
                "nothing outstanding never defers",
                OwnWork::default(),
                300,
                false,
            ),
            (
                "fresh work, first due cycle",
                owed(2, 1, now, 60),
                300,
                true,
            ),
            ("still inside the budget", owed(2, 1, now, 60), 899, true),
            ("the budget is spent", owed(2, 1, now, 60), 900, false),
            (
                "work one second under the cap",
                owed(1, 0, now, 1199),
                300,
                true,
            ),
            ("work at the cap", owed(1, 0, now, 1200), 300, false),
            (
                "a disabled reminder defers nothing it was never going to send",
                owed(1, 0, now, 60),
                300,
                true,
            ),
        ];
        for (why, own, idle_age, want) in cases {
            assert_eq!(deferred(own, now, idle_age, &knobs), want, "{why}");
        }
        assert!(
            !deferred(
                owed(1, 0, now, 60),
                now,
                300,
                &Knobs {
                    idle_nudge_secs: 0,
                    ..Knobs::default()
                }
            ),
            "a disabled reminder defers nothing"
        );
    }

    /// Past the ceiling the seat spends its ORDINARY budget — deferral, never
    /// silence — and every reminder says what ae thinks it is waiting on.
    #[test]
    fn past_the_ceiling_the_bounded_budget_resumes_and_names_the_reason() {
        let knobs = Knobs::default();
        let mut observed = seen();
        observed.harness.frame = crate::harness_state::HarnessState::Idle;
        observed.own_work = owed(2, 1, observed.now_epoch, 60);
        let prior = PaneState {
            identity: Some(observed.identity),
            idle_since_epoch: Some(observed.now_epoch - 900),
            ..PaneState::default()
        };
        let booked = account(&prior, &observed, &knobs);
        assert!(
            booked.effects.contains(&Effect::Nudge),
            "the budget's worth of deferred opportunities is spent"
        );

        let exhausted = PaneState {
            nudge_count: knobs.max_nudges,
            ..prior.clone()
        };
        let alert = account(&exhausted, &observed, &knobs);
        assert!(
            alert.effects.iter().any(|effect| matches!(
                effect,
                Effect::Emit {
                    action: "alert",
                    ..
                }
            )),
            "and the ordinary alert still ends it"
        );

        // The OTHER escape hatch, on its own: the idle clock has barely
        // started, but the work itself has gone stale.
        let mut aged = seen();
        aged.harness.frame = crate::harness_state::HarnessState::Idle;
        aged.own_work = owed(2, 1, aged.now_epoch, 1200);
        let due = PaneState {
            identity: Some(aged.identity),
            idle_since_epoch: Some(aged.now_epoch - 300),
            ..PaneState::default()
        };
        assert!(
            account(&due, &aged, &knobs)
                .effects
                .contains(&Effect::Nudge),
            "work older than the cap is nudged even inside the budget"
        );

        let reason = observed.own_work.reason().expect("outstanding work");
        assert_eq!(reason, "waiting on 2 requests, 1 spawn");
        let text = idle_nudge_text_waiting(None, Path::new("/m"), &reason);
        assert!(text.contains(&reason), "the reminder names it: {text}");
        assert!(
            text.contains("you look idle"),
            "on top of the ordinary reminder, not instead of it: {text}"
        );
    }

    /// Outstanding work gates ONE branch. Every precedence above it is untouched.
    #[test]
    fn outstanding_work_never_outranks_dead_quiet_or_throttled() {
        let knobs = Knobs::default();
        let mut observed = seen();
        observed.harness.frame = crate::harness_state::HarnessState::Idle;
        observed.own_work = owed(3, 2, observed.now_epoch, 60);
        let prior = PaneState {
            identity: Some(observed.identity),
            idle_since_epoch: Some(observed.now_epoch - 300),
            ..PaneState::default()
        };

        let mut dead = observed.clone();
        dead.is_dead = true;
        assert_eq!(account(&prior, &dead, &knobs).verdict, Verdict::Dead);

        let mut quiet = observed.clone();
        quiet.quiet = Some(QuietKind::Done);
        assert_eq!(
            account(&prior, &quiet, &knobs).verdict,
            Verdict::Quiet(QuietKind::Done)
        );

        let mut throttled = observed.clone();
        throttled.throttle = Some(Throttle::Throttled);
        assert_eq!(
            account(&prior, &throttled, &knobs).verdict,
            Verdict::Throttled
        );

        let mut stale = observed.clone();
        stale.harness.durable_stale = true;
        assert_eq!(account(&prior, &stale, &knobs).verdict, Verdict::Stale);
    }

    /// S4: a message a peer delivered changes the PANE, not the seat's own
    /// declaration — so it must not hand the seat a fresh idle episode.
    #[test]
    fn a_delivered_message_does_not_re_arm_the_idle_clock() {
        let knobs = Knobs::default();
        let mut observed = seen();
        observed.harness.frame = crate::harness_state::HarnessState::Idle;
        observed.harness.declaration = Some(77);
        let armed = observed.now_epoch - 280;
        let prior = PaneState {
            identity: Some(observed.identity),
            idle_since_epoch: Some(armed),
            last_declaration: Some(77),
            ..PaneState::default()
        };

        // The delivery repaints the pane: a new hash, the same declaration.
        observed.hash = 999;
        let after = account(&prior, &observed, &knobs);
        assert_eq!(
            after.next.idle_since_epoch,
            Some(armed),
            "the episode survives somebody else's message"
        );

        // Nor does it rescue a seat whose clock HAS run out.
        let due = PaneState {
            idle_since_epoch: Some(observed.now_epoch - 300),
            ..prior.clone()
        };
        assert!(
            account(&due, &observed, &knobs)
                .effects
                .contains(&Effect::Nudge),
            "somebody else's message is not this seat's answer"
        );

        // The seat's OWN newer declaration is the one thing that restarts it.
        observed.harness.declaration = Some(78);
        let declared = account(&due, &observed, &knobs);
        assert_eq!(
            declared.next.idle_since_epoch,
            Some(observed.now_epoch),
            "a fresh episode, from the declaration forward"
        );
        assert!(
            !declared.effects.contains(&Effect::Nudge),
            "and only that buys the seat its full clock back"
        );
    }

    #[test]
    fn a_positive_idle_frame_is_idle_before_motion_can_call_it_working() {
        let mut observed = seen();
        observed.harness.frame = crate::harness_state::HarnessState::Idle;
        observed.last_actor_event_age_secs = 10_000;
        let booked = account(&PaneState::default(), &observed, &Knobs::default());
        assert_eq!(booked.verdict, Verdict::Idle);
        assert_eq!(booked.next.idle_since_epoch, Some(observed.now_epoch));
        assert!(!booked.effects.contains(&Effect::Nudge));
    }

    #[test]
    fn the_idle_nudge_clock_fires_at_300_not_299_seconds() {
        let mut observed = seen();
        observed.harness.frame = crate::harness_state::HarnessState::Idle;
        let prior = PaneState {
            identity: Some(observed.identity),
            idle_since_epoch: Some(observed.now_epoch - 299),
            ..PaneState::default()
        };
        let early = account(&prior, &observed, &Knobs::default());
        assert_eq!(early.verdict, Verdict::Idle);
        assert!(!early.effects.contains(&Effect::Nudge));

        let due = PaneState {
            idle_since_epoch: Some(observed.now_epoch - 300),
            ..early.next
        };
        let booked = account(&due, &observed, &Knobs::default());
        assert_eq!(booked.verdict, Verdict::Idle);
        assert!(booked.effects.contains(&Effect::Nudge));
        assert_eq!(booked.next.idle_since_epoch, due.idle_since_epoch);
    }

    #[test]
    fn repeated_idle_frames_exhaust_once_and_stay_stale_until_real_recovery() {
        let knobs = Knobs::default();
        let mut observed = seen();
        observed.harness.frame = crate::harness_state::HarnessState::Idle;
        let mut prior = PaneState {
            identity: Some(observed.identity),
            idle_since_epoch: Some(observed.now_epoch - 300),
            nudge_count: knobs.max_nudges,
            ..PaneState::default()
        };
        let exhausted = account(&prior, &observed, &knobs);
        assert_eq!(exhausted.verdict, Verdict::Stale);
        assert_eq!(
            emitted(&exhausted.effects),
            vec![("alert", "max nudges reached (idle 5m), needs attention")]
        );
        prior = exhausted.next;
        observed.hash = 99;
        let repeated = account(&prior, &observed, &knobs);
        assert_eq!(repeated.verdict, Verdict::Stale);
        assert!(repeated.effects.is_empty());

        observed.harness.frame = crate::harness_state::HarnessState::Busy;
        observed.harness.durable_stale = true;
        let recovered = account(&repeated.next, &observed, &knobs);
        assert_eq!(recovered.verdict, Verdict::Active);
        assert_eq!(recovered.next.idle_since_epoch, None);
        assert_eq!(recovered.next.nudge_count, 0);
        assert_eq!(
            emitted(&recovered.effects),
            vec![("alert-cleared", "agent busy again — stale alert cleared")]
        );
    }

    #[test]
    fn a_human_draft_resets_the_idle_episode_without_claiming_busy() {
        let mut observed = seen();
        observed.harness.frame = crate::harness_state::HarnessState::Unknown;
        observed.harness.human_draft = true;
        observed.last_actor_event_age_secs = 10_000;
        let prior = PaneState {
            identity: Some(observed.identity),
            prev_hash: Some(observed.hash),
            last_hash_change: Some(observed.now_epoch - 10_000),
            idle_since_epoch: Some(observed.now_epoch - 600),
            nudge_count: 2,
            undelivered_streak: 2,
            ..PaneState::default()
        };
        let booked = account(&prior, &observed, &Knobs::default());
        assert_eq!(booked.verdict, Verdict::Active);
        assert_eq!(booked.next.idle_since_epoch, None);
        assert_eq!(booked.next.nudge_count, 0);
        assert_eq!(booked.next.undelivered_streak, 0);
        assert!(booked.effects.is_empty(), "never paste over a human draft");
    }

    #[test]
    fn an_unknown_frame_preserves_but_does_not_spend_the_idle_episode() {
        let mut observed = seen();
        observed.harness.frame = crate::harness_state::HarnessState::Unknown;
        observed.hash = 99;
        let prior = PaneState {
            identity: Some(observed.identity),
            prev_hash: Some(7),
            last_hash_change: Some(observed.now_epoch - 600),
            idle_since_epoch: Some(observed.now_epoch - 299),
            nudge_count: 1,
            undelivered_streak: 2,
            ..PaneState::default()
        };
        let booked = account(&prior, &observed, &Knobs::default());
        assert_eq!(booked.verdict, Verdict::Active);
        assert_eq!(booked.next.idle_since_epoch, prior.idle_since_epoch);
        assert_eq!(booked.next.nudge_count, 1);
        assert_eq!(booked.next.undelivered_streak, 2);
        assert!(booked.effects.is_empty());
    }

    #[test]
    fn a_durable_idle_alert_survives_unknown_and_human_draft_frames() {
        let mut observed = seen();
        observed.harness.frame = crate::harness_state::HarnessState::Unknown;
        observed.harness.human_draft = true;
        observed.harness.durable_stale = true;
        let prior = PaneState {
            identity: Some(observed.identity),
            idle_since_epoch: Some(observed.now_epoch - 600),
            nudge_count: 3,
            ..PaneState::default()
        };
        let booked = account(&prior, &observed, &Knobs::default());
        assert_eq!(booked.verdict, Verdict::Stale);
        assert_eq!(booked.next.idle_since_epoch, None);
        assert_eq!(booked.next.nudge_count, 0);
        assert!(
            booked.effects.is_empty(),
            "only Busy or declaration clears it"
        );
    }

    #[test]
    fn a_dead_agent_is_alerted_once_and_then_skipped_forever() {
        let mut observed = seen();
        observed.is_dead = true;
        observed.descendancy = Descendancy::Absent;
        let first = account(&PaneState::default(), &observed, &Knobs::default());
        assert_eq!(first.verdict, Verdict::Dead);
        assert!(first.next.dead_latched);
        assert_eq!(
            emitted(&first.effects),
            vec![("alert", "agent process dead — dropped to shell")]
        );
        // Still positively gone: no second alert, and the latch holds.
        let second = account(&first.next, &observed, &Knobs::default());
        assert_eq!(second.verdict, Verdict::Dead);
        assert!(second.next.dead_latched);
        assert!(emitted(&second.effects).is_empty(), "alerted twice");
    }

    #[test]
    fn a_dead_agent_whose_process_is_back_is_cleared_once_and_judged_normally() {
        // The human's own recovery path — a re-run in the SAME pane — keeps
        // the identity, so only the process reading separates "still gone"
        // from "back"; and the cycle that clears is the cycle that judges.
        let knobs = Knobs::default();
        let mut dead = seen();
        dead.is_dead = true;
        dead.descendancy = Descendancy::Absent;
        let first = account(&PaneState::default(), &dead, &knobs);
        assert_eq!(first.verdict, Verdict::Dead);
        assert!(first.next.dead_latched);

        let back = seen();
        let mut carried = first.next.clone();
        // A pre-death episode that must not feed the resumed judgement.
        carried.prev_hash = Some(back.hash);
        carried.last_hash_change = Some(back.now_epoch - 5_000);
        carried.idle_since_epoch = Some(back.now_epoch - 5_000);
        carried.nudge_count = 2;
        carried.undelivered_streak = 3;
        carried.throttle_streak = 4;
        let cleared = account(&carried, &back, &knobs);
        assert_eq!(cleared.verdict, Verdict::Active);
        assert!(!cleared.next.dead_latched);
        assert_eq!(
            cleared.effects,
            vec![
                Effect::Emit {
                    action: "dead-cleared",
                    summary: "agent process back — resumed in place".to_owned(),
                },
                Effect::Notify("is BACK — process resumed".to_owned()),
            ]
        );
        // One episode, not the pre-death one: the hash clock restarted, the
        // idle clock is fresh, and no stale throttle clear leaks out.
        assert_eq!(cleared.next.prev_hash, Some(back.hash));
        assert_eq!(cleared.next.last_hash_change, Some(back.now_epoch));
        assert_eq!(cleared.next.idle_since_epoch, None);
        assert_eq!(cleared.next.nudge_count, 0);
        assert_eq!(cleared.next.undelivered_streak, 0);
        assert_eq!(cleared.next.throttle_streak, 0);

        // The SECOND cycle after the clear is ordinary: the verdict is not
        // Dead, nothing is emitted, and a real second death is a real event.
        let again = account(&cleared.next, &back, &knobs);
        assert_eq!(again.verdict, Verdict::Active, "the latch stayed cleared");
        assert!(emitted(&again.effects).is_empty(), "one clear per return");
        let died_again = account(&again.next, &dead, &knobs);
        assert_eq!(died_again.verdict, Verdict::Dead);
        assert_eq!(
            emitted(&died_again.effects),
            vec![("alert", "agent process dead — dropped to shell")],
            "a seat that dies again is alerted again"
        );
    }

    #[test]
    fn a_probe_gap_never_clears_a_dead_latch() {
        // `classify_dead` must not fire on an unusable snapshot, and neither
        // may this: returning false because the tree could not be read is not
        // evidence of life.
        let mut prior = PaneState {
            dead_latched: true,
            ..PaneState::default()
        };
        prior.identity = Some(seen().identity);
        let mut observed = seen();
        observed.is_dead = false;
        observed.descendancy = Descendancy::Unknown;
        let booked = account(&prior, &observed, &Knobs::default());
        assert_eq!(booked.verdict, Verdict::Dead);
        assert!(
            booked.next.dead_latched,
            "an unknown snapshot keeps the latch"
        );
        assert!(booked.effects.is_empty(), "and raises nothing new");
    }

    #[test]
    fn a_probe_that_never_works_is_alerted_once_and_the_streak_resets_on_a_good_one() {
        let mut observed = seen();
        observed.descendancy = Descendancy::Unknown;
        let knobs = Knobs::default();
        let mut state = PaneState::default();
        for cycle in 1..UNKNOWN_ALERT_CYCLES {
            let booked = account(&state, &observed, &knobs);
            state = booked.next;
            assert!(
                emitted(&booked.effects).is_empty(),
                "alerted early, at cycle {cycle}"
            );
        }
        let booked = account(&state, &observed, &knobs);
        state = booked.next;
        assert_eq!(
            emitted(&booked.effects),
            vec![(
                "alert",
                "process probe unusable for 5 cycles — liveness unverifiable"
            )]
        );
        // Once, not once per cycle.
        let again = account(&state, &observed, &knobs);
        assert!(emitted(&again.effects).is_empty());
        // A usable snapshot clears the streak, so a later outage alerts again.
        let recovered = account(&again.next, &seen(), &knobs);
        assert_eq!(recovered.next.unknown_streak, 0);
        assert!(!recovered.next.unknown_alerted);
    }

    #[test]
    fn a_usage_limit_pane_reads_limit_and_the_dead_branch_still_wins() {
        let knobs = Knobs::default();
        let mut observed = seen();
        observed.throttle = Some(Throttle::LimitReached);
        let first = account(&PaneState::default(), &observed, &knobs);
        assert_eq!(first.verdict, Verdict::Limit);
        assert_eq!(
            emitted(&first.effects),
            vec![(
                "limit",
                "vendor usage limit reached — waits for a reset or a re-login"
            )]
        );
        assert!(first.next.limit_latched);
        let second = account(&first.next, &observed, &knobs);
        assert_eq!(second.verdict, Verdict::Limit);
        assert!(emitted(&second.effects).is_empty(), "one limit event");
        assert!(!second.effects.contains(&Effect::QuotaRefresh), "no pass");
        // Dead is dead: the death branch returns before the limit branch.
        let mut dying = observed.clone();
        dying.is_dead = true;
        let dead = account(&PaneState::default(), &dying, &knobs);
        assert_eq!(dead.verdict, Verdict::Dead);
        assert!(!dead.next.limit_latched, "no limit episode was entered");
    }

    #[test]
    fn the_limit_release_retracts_once_and_requests_one_quota_pass() {
        let knobs = Knobs::default();
        let mut observed = seen();
        observed.throttle = Some(Throttle::LimitReached);
        let limited = account(&PaneState::default(), &observed, &knobs);
        let released = account(&limited.next, &seen(), &knobs);
        assert_eq!(released.verdict, Verdict::Active);
        assert_eq!(
            emitted(&released.effects),
            vec![(
                "alert-cleared",
                "usage limit cleared — pane no longer shows it"
            )]
        );
        assert!(released.effects.contains(&Effect::QuotaRefresh), "one pass");
        assert!(!released.next.limit_latched);
        // Two consecutive clear cycles: no second retraction, no second pass.
        let again = account(&released.next, &seen(), &knobs);
        assert!(emitted(&again.effects).is_empty(), "one retraction");
        assert!(
            !again.effects.contains(&Effect::QuotaRefresh),
            "no second pass"
        );
    }

    #[test]
    fn the_due_counter_has_one_caller_so_a_recovery_cannot_consume_the_cadence() {
        // STRUCTURAL: the counter advances only inside `quota_observation_due`,
        // and `refresh_quota` has exactly two callers beside its definition.
        let production = include_str!("watchdog_daemon.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default();
        assert_eq!(production.matches("quota_observation_due(").count(), 2);
        assert_eq!(production.matches("refresh_quota(").count(), 3);
    }

    #[test]
    fn a_failed_capture_never_releases_a_usage_limit() {
        let knobs = Knobs::default();
        let mut limited_seen = seen();
        limited_seen.throttle = Some(Throttle::LimitReached);
        let limited = account(&PaneState::default(), &limited_seen, &knobs);
        assert_eq!(limited.verdict, Verdict::Limit);

        // A failed tmux read: empty text, `capture_ok` false — no reading is
        // not a clearing.
        let mut failed = seen();
        failed.capture_ok = false;
        let held = account(&limited.next, &failed, &knobs);
        assert_eq!(held.verdict, Verdict::Limit, "no reading is not a clearing");
        assert!(held.next.limit_latched);
        assert!(emitted(&held.effects).is_empty(), "no retraction");
        assert!(!held.effects.contains(&Effect::QuotaRefresh), "no pass");

        // The next GOOD capture that still shows the phrase keeps it, silently.
        let again = account(&held.next, &limited_seen, &knobs);
        assert_eq!(again.verdict, Verdict::Limit);
        assert!(emitted(&again.effects).is_empty(), "still one episode");
        assert!(!again.effects.contains(&Effect::QuotaRefresh), "no pass");

        // A good capture WITHOUT the phrase releases with one pass.
        let released = account(&again.next, &seen(), &knobs);
        assert_eq!(released.verdict, Verdict::Active);
        assert!(released.effects.contains(&Effect::QuotaRefresh), "one pass");
    }

    #[test]
    fn throttling_says_so_once_alerts_at_the_bound_and_clears_on_recovery() {
        let knobs = Knobs::default();
        let mut observed = seen();
        observed.throttle = Some(Throttle::Throttled);
        let mut state = PaneState::default();
        let first = account(&state, &observed, &knobs);
        assert_eq!(first.verdict, Verdict::Throttled);
        assert_eq!(
            emitted(&first.effects),
            vec![("throttled", "upstream throttling detected — pausing nudges")]
        );
        state = first.next;
        for _ in 2..knobs.throttle_alert_cycles {
            let booked = account(&state, &observed, &knobs);
            assert!(emitted(&booked.effects).is_empty(), "one throttled event");
            state = booked.next;
        }
        let alerted = account(&state, &observed, &knobs);
        assert_eq!(
            emitted(&alerted.effects),
            vec![("alert", "throttled for 300s — may need attention")]
        );
        state = alerted.next;
        // A non-throttled cycle clears, with the streak it cleared.
        let cleared = account(&state, &seen(), &knobs);
        assert_eq!(
            emitted(&cleared.effects),
            vec![("throttle-cleared", "throttling cleared after 5 cycles")]
        );
        assert_eq!(cleared.next.throttle_streak, 0);
    }

    #[test]
    fn a_throttle_clears_even_for_an_agent_that_is_quiet() {
        // The clear runs BEFORE the quiet branch returns, so a quiet agent does
        // not carry a stale throttle streak forever.
        let prior = PaneState {
            throttle_streak: 2,
            ..PaneState::default()
        };
        let mut observed = seen();
        observed.quiet = Some(QuietKind::WaitingUser);
        let booked = account(&prior, &observed, &Knobs::default());
        assert_eq!(booked.verdict, Verdict::Quiet(QuietKind::WaitingUser));
        assert_eq!(
            emitted(&booked.effects),
            vec![("throttle-cleared", "throttling cleared after 2 cycles")]
        );
    }

    #[test]
    fn a_quiet_state_suppresses_the_nudge_and_resets_the_delivered_count() {
        let prior = PaneState {
            nudge_count: 2,
            prev_hash: Some(7),
            last_hash_change: Some(0),
            ..PaneState::default()
        };
        let mut observed = seen();
        observed.quiet = Some(QuietKind::Done);
        observed.last_actor_event_age_secs = 10_000;
        let booked = account(&prior, &observed, &Knobs::default());
        assert_eq!(booked.verdict, Verdict::Quiet(QuietKind::Done));
        assert_eq!(booked.next.nudge_count, 0);
        assert!(
            !booked.effects.contains(&Effect::Nudge),
            "a quiet agent is not nudged"
        );
    }

    /// A fresh `waiting-agent` holds exactly like the other quiet states; past
    /// its ceiling it becomes exactly `blocked` AND the nudge budget resumes.
    #[test]
    fn a_waiting_agent_holds_while_fresh_and_escalates_past_the_ceiling() {
        let knobs = Knobs::default(); // idle_nudge_secs 300 -> cap 1200s
        let mut observed = seen();
        observed.quiet = Some(QuietKind::WaitingAgent);

        observed.last_actor_event_age_secs = 1_199;
        let fresh = account(&PaneState::default(), &observed, &knobs);
        assert_eq!(
            fresh.verdict,
            Verdict::Quiet(QuietKind::WaitingAgent),
            "one second short of the ceiling is still a quiet hold"
        );
        assert!(
            !fresh.effects.contains(&Effect::Nudge),
            "the fresh half must not nudge"
        );

        observed.last_actor_event_age_secs = 1_200;
        let escalated = account(&PaneState::default(), &observed, &knobs);
        assert_eq!(
            escalated.verdict,
            Verdict::Quiet(QuietKind::Blocked),
            "past the ceiling it is exactly blocked"
        );
        assert!(
            escalated.effects.contains(&Effect::Nudge),
            "and the nudge budget resumes"
        );

        // The nudge half is off at a zero cadence; the attention half is not.
        let zero = Knobs {
            idle_nudge_secs: 0,
            ..knobs
        };
        let attention_only = account(&PaneState::default(), &observed, &zero);
        assert_eq!(
            attention_only.verdict,
            Verdict::Quiet(QuietKind::Blocked),
            "a zero knob switches off nudging, never the human marker"
        );
        assert!(!attention_only.effects.contains(&Effect::Nudge));
    }

    #[test]
    fn quiet_repaint_rearms_once_then_two_changes_activate() {
        let scratch = Scratch::new("quiet-streak");
        let helper = SendHelper::for_session(&scratch.0);
        let server = ServerId::Ambient;
        let cycle = Cycle {
            knobs: Knobs::default(),
            meta_dir: &scratch.0,
            helper: &helper,
            server: &server,
            session: "demo",
            goal: None,
            roster: Vec::new(),
            local_config: None,
            lead_pair: false,
            fleet_order: crate::theme::FleetOrder::EMPTY,
            meta_agent: false,
            launch_ids: Vec::new(),
        };
        let event = Event::parse_line(
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-user","summary":"review"}"#,
        )
        .expect("well-formed state event");
        let key = declaration_key(&event);
        let events = vec![event];
        let mut state = PaneState {
            quiet_base: Some((key.clone(), 7, 0)),
            ..PaneState::default()
        };
        let mut quiet_cycle = QuietCycle::new(4);
        let mut query = QuietQuery {
            events: &events,
            agent: "opus5:builder",
            slot: "main",
            hash: 9,
            index: 1,
            pane_id: "%1",
        };

        let first = cycle.resolve_quiet(&query, &mut state, &mut quiet_cycle);
        assert_eq!(first, Some(QuietKind::WaitingUser));
        assert_eq!(state.quiet_base, Some((key.clone(), 9, 1)));
        let mut observed = seen();
        observed.hash = 9;
        observed.quiet = first;
        assert_eq!(
            account(&PaneState::default(), &observed, &Knobs::default()).verdict,
            Verdict::Quiet(QuietKind::WaitingUser)
        );

        query.hash = 11;
        let second = cycle.resolve_quiet(&query, &mut state, &mut quiet_cycle);
        assert_eq!(second, None);
        observed.hash = 11;
        observed.quiet = second;
        assert_eq!(
            account(&state, &observed, &Knobs::default()).verdict,
            Verdict::Active
        );

        query.hash = 9;
        let settled = cycle.resolve_quiet(&query, &mut state, &mut quiet_cycle);
        assert_eq!(settled, Some(QuietKind::WaitingUser));
        assert_eq!(state.quiet_base, Some((key, 9, 0)));
    }

    fn witness_b_idle_after_new_working_then_memo() -> Observation {
        let scratch = Scratch::new("working-resets-idle");
        let helper = SendHelper::for_session(&scratch.0);
        let server = ServerId::Ambient;
        let cycle = Cycle {
            knobs: Knobs::default(),
            meta_dir: &scratch.0,
            helper: &helper,
            server: &server,
            session: "demo",
            goal: None,
            roster: Vec::new(),
            local_config: None,
            lead_pair: false,
            fleet_order: crate::theme::FleetOrder::EMPTY,
            meta_agent: false,
            launch_ids: Vec::new(),
        };
        let events = vec![
            Event::parse_line(
                r#"{"ts":"2026-09-10T14:59:00Z","actor":"_watchdog","action":"alert","target":"codex:agent","target_slot":"main","target_session":"demo","summary":"max nudges reached (idle 5m), needs attention"}"#,
            )
            .expect("well-formed addressed stale alert"),
            Event::parse_line(
                r#"{"ts":"2026-09-10T15:00:00Z","actor":"codex:agent","action":"state","ref":"working","actor_slot":"main","actor_session":"demo"}"#,
            )
            .expect("well-formed working declaration"),
            Event::parse_line(
                r#"{"ts":"2026-09-10T15:01:00Z","actor":"codex:agent","action":"memo","ref":"hstate","summary":"checkpoint","actor_slot":"main","actor_session":"demo"}"#,
            )
            .expect("well-formed later own memo"),
        ];
        assert_eq!(
            crate::session::alert_reason_in(&events[..1], "demo", "main", "codex:agent"),
            Some(crate::attention::Reason::Stale),
            "the addressed prior alert is the positive control"
        );
        let found = crate::watchdog::latest_relevant_event(&events, "demo", "main", "codex:agent")
            .expect("the later memo is relevant");
        assert_eq!(found.event.action, "memo");
        assert!(found.is_own, "the memo is the seat's own activity");
        assert_eq!(
            crate::watchdog::quiet_reason(&found),
            None,
            "the later memo is activity, not a quiet declaration"
        );
        let capture = include_str!("../tests/fixtures/harness-state/codex-idle-112x40.txt");
        let harness = cycle.harness_observation(
            capture,
            crate::tool::ToolKind::Codex,
            &events,
            "main",
            "codex:agent",
        );
        assert_eq!(harness.frame, crate::harness_state::HarnessState::Idle);
        assert!(
            !harness.durable_stale,
            "newer own activity clears the alert"
        );
        assert!(
            harness.declaration.is_some(),
            "a later own memo does not hide the newest own declaration"
        );
        let mut observed = seen();
        observed.now_epoch = events[2].ts.epoch();
        observed.harness = harness;
        observed.last_actor_event_age_secs = 0;
        observed
    }

    #[test]
    fn witness_b_new_working_clears_an_exhausted_idle_episode() {
        let knobs = Knobs::default();
        let observed = witness_b_idle_after_new_working_then_memo();
        let prior = PaneState {
            identity: Some(observed.identity),
            idle_since_epoch: Some(observed.now_epoch - 600),
            nudge_count: knobs.max_nudges + 1,
            ..PaneState::default()
        };
        let mut busy = observed.clone();
        busy.harness.frame = crate::harness_state::HarnessState::Busy;
        let positive = account(&prior, &busy, &knobs);
        assert_eq!(positive.verdict, Verdict::Active);
        assert_eq!(positive.next.idle_since_epoch, None);
        assert_eq!(positive.next.nudge_count, 0);
        let booked = account(&prior, &observed, &knobs);
        assert_eq!(booked.verdict, Verdict::Idle);
        assert_eq!(booked.next.idle_since_epoch, Some(observed.now_epoch));
        assert_eq!(booked.next.nudge_count, 0);
        assert!(!booked.effects.contains(&Effect::Nudge));

        let raw = observed_option(crate::harness_state::HarnessState::Idle, &booked.next);
        let mut restarted = PaneState::default();
        restore_idle(&mut restarted, &raw, observed.identity);
        assert_eq!(
            restarted.last_declaration, booked.next.last_declaration,
            "the applied declaration survives a daemon restart"
        );
        let mut later = observed.clone();
        later.now_epoch += 299;
        let continued = account(&restarted, &later, &knobs);
        assert_eq!(continued.next.idle_since_epoch, Some(observed.now_epoch));
        assert_eq!(continued.next.nudge_count, 0);
        assert!(!continued.effects.contains(&Effect::Nudge));
    }

    #[test]
    fn witness_b_new_working_restarts_a_due_nonexhausted_idle_clock() {
        let knobs = Knobs::default();
        let observed = witness_b_idle_after_new_working_then_memo();
        let prior = PaneState {
            identity: Some(observed.identity),
            idle_since_epoch: Some(observed.now_epoch - 600),
            nudge_count: 1,
            ..PaneState::default()
        };
        let mut busy = observed.clone();
        busy.harness.frame = crate::harness_state::HarnessState::Busy;
        let positive = account(&prior, &busy, &knobs);
        assert_eq!(positive.verdict, Verdict::Active);
        assert_eq!(positive.next.nudge_count, 0);
        let booked = account(&prior, &observed, &knobs);
        assert_eq!(booked.verdict, Verdict::Idle);
        assert_eq!(booked.next.idle_since_epoch, Some(observed.now_epoch));
        assert_eq!(booked.next.nudge_count, 0);
        assert!(!booked.effects.contains(&Effect::Nudge));
    }

    #[test]
    fn a_moving_pane_is_active_and_re_arms_delivery() {
        let prior = PaneState {
            prev_hash: Some(1),
            nudge_count: 2,
            undelivered_streak: 3,
            ..PaneState::default()
        };
        let booked = account(&prior, &seen(), &Knobs::default());
        assert_eq!(booked.verdict, Verdict::Active);
        assert_eq!(booked.next.prev_hash, Some(7));
        assert_eq!(booked.next.last_hash_change, Some(10_000));
        assert_eq!(booked.next.nudge_count, 0);
        assert_eq!(
            booked.next.undelivered_streak, 0,
            "a moving pane is evidence that whatever blocked delivery is gone"
        );
    }

    #[test]
    fn a_still_but_recently_changed_pane_is_not_stale() {
        let (mut prior, mut observed) = stale_pane();
        prior.last_hash_change = Some(observed.now_epoch - 10);
        observed.last_actor_event_age_secs = 10_000;
        let booked = account(&prior, &observed, &Knobs::default());
        assert_eq!(booked.verdict, Verdict::Active);
        assert!(!booked.effects.contains(&Effect::Nudge));
    }

    #[test]
    fn a_stale_agent_is_nudged_until_the_max_then_alerted_exactly_once() {
        let knobs = Knobs::default();
        let (mut prior, observed) = stale_pane();
        for delivered in 0..knobs.max_nudges {
            prior.nudge_count = delivered;
            let booked = account(&prior, &observed, &knobs);
            assert_eq!(booked.verdict, Verdict::Stale);
            assert!(
                booked.effects.contains(&Effect::Nudge),
                "nudge {delivered} not attempted"
            );
        }
        prior.nudge_count = knobs.max_nudges;
        let alerted = account(&prior, &observed, &knobs);
        assert_eq!(
            emitted(&alerted.effects),
            vec![("alert", "max nudges reached (idle 166m), needs attention")]
        );
        assert_eq!(
            alerted.next.nudge_count,
            knobs.max_nudges + 1,
            "the count passes the max so the alert cannot repeat"
        );
        prior.nudge_count = knobs.max_nudges + 1;
        let silent = account(&prior, &observed, &knobs);
        assert!(emitted(&silent.effects).is_empty(), "alerted twice");
        assert!(!silent.effects.contains(&Effect::Nudge));
    }

    #[test]
    fn an_unreachable_pane_stops_costing_cycles() {
        let knobs = Knobs::default();
        let (mut prior, observed) = stale_pane();
        prior.undelivered_streak = knobs.undelivered_max;
        let booked = account(&prior, &observed, &knobs);
        assert_eq!(booked.verdict, Verdict::Stale, "still counted stale");
        assert!(
            !booked.effects.contains(&Effect::Nudge),
            "the bound stops ATTEMPTS, not merely alerts"
        );
    }

    #[test]
    fn the_counter_counts_deliveries_and_the_streak_counts_attempts() {
        let knobs = Knobs::default();
        let mut state = PaneState::default();
        assert!(record_nudge(&mut state, true, &knobs, "idle 20m").is_empty());
        assert_eq!(state.nudge_count, 1);
        // A refused or abandoned send is an attempt, not a nudge.
        let mut undelivered = PaneState::default();
        for _ in 1..knobs.undelivered_max {
            assert!(record_nudge(&mut undelivered, false, &knobs, "idle 20m").is_empty());
        }
        assert_eq!(undelivered.nudge_count, 0, "nothing was delivered");
        let effects = record_nudge(&mut undelivered, false, &knobs, "idle 20m");
        assert_eq!(
            emitted(&effects),
            vec![(
                "alert",
                "nudge unreachable/occupied — 3 undelivered attempts (idle 20m)"
            )]
        );
        // Once at the bound, not on every attempt past it.
        assert!(record_nudge(&mut undelivered, false, &knobs, "idle 20m").is_empty());
        // And a delivery clears the streak.
        assert!(record_nudge(&mut undelivered, true, &knobs, "idle 20m").is_empty());
        assert_eq!(undelivered.undelivered_streak, 0);
    }

    #[test]
    fn the_stale_display_names_the_sentinel_rather_than_rendering_it() {
        assert_eq!(stale_display(0), "idle 0m");
        assert_eq!(stale_display(900), "idle 15m");
        assert_eq!(stale_display(9999 * 60), "idle 9999m");
        assert_eq!(stale_display(10_000 * 60), "no recent events");
        assert_eq!(stale_display(super::NO_EVENT_AGE), "no recent events");
    }

    #[test]
    fn the_nudge_names_this_sessions_own_state_helper() {
        let meta = Path::new("/home/x/.ae/sessions/demo");
        let plain = nudge_text(None, meta);
        assert!(plain.starts_with("Status check: if you have more work, continue."));
        assert!(plain.ends_with(
            "/home/x/.ae/sessions/demo/state <waiting-user|waiting-agent|blocked|done> \"<reason>\""
        ));
        let goaled = nudge_text(Some("ship P4.1"), meta);
        assert!(goaled.starts_with("Session goal: ship P4.1. Status check:"));
        let idle = idle_nudge_text(Some("ship P4.1"), meta);
        assert!(idle.contains("you look idle: declare state or continue"));
        assert!(idle.ends_with(
            "/home/x/.ae/sessions/demo/state <waiting-user|waiting-agent|blocked|done> \"<reason>\""
        ));
    }

    /// IMPORTANT 4: the CURRENT generator's exact bytes must survive the LIVE
    /// footprint filter. `watchdog.rs`'s `RAW_NUDGE` receipt exercises the
    /// pre-`waiting-agent` spelling only, so without this composition a
    /// deleted `strip_suffix(NUDGE_TAIL)` would leave every current-core nudge
    /// counting as pane activity — a quiet hold that never arms.
    ///
    /// `idle_nudge_text` is a DIFFERENT generator whose raw sentence the
    /// footprint filter has never recognized (pre-existing; named in
    /// `.local/waitagent-sites.md`), so this receipt pins the status generator
    /// the review named, in both its goal and no-goal forms.
    #[test]
    fn a_current_nudge_is_stripped_by_the_live_footprint_filter() {
        let meta = Path::new("/home/x/.ae/sessions/demo");
        for (label, text) in [
            ("status", nudge_text(None, meta)),
            ("status-goaled", nudge_text(Some("ship P4.1"), meta)),
        ] {
            assert_eq!(
                quiet_filter(&text),
                "",
                "{label}: the current nudge body is a footprint, not output"
            );
            assert_eq!(
                quiet_hash("live output\n"),
                quiet_hash(&format!("live output\n{text}\n")),
                "{label}: a delivered current nudge must not move the pane hash"
            );
        }
    }

    #[test]
    fn a_future_timestamp_clamps_to_zero_instead_of_underflowing() {
        assert_eq!(age_secs(100, 40), 60);
        assert_eq!(age_secs(100, 100), 0);
        assert_eq!(
            age_secs(100, 4_000),
            0,
            "a clock-skewed future event must not read as an eternity of silence"
        );
    }

    #[test]
    fn the_event_age_is_the_actors_own_newest_by_append_position() {
        let events: Vec<Event> = [
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"working"}"#,
            r#"{"ts":"2026-08-29T04:09:00Z","actor":"fable5:lead","action":"send","target":"opus5:builder"}"#,
            r#"{"ts":"2026-08-29T04:01:00Z","actor":"opus5:builder","action":"memo","ref":"t"}"#,
        ]
        .iter()
        .map(|line| Event::parse_line(line).expect("specimen"))
        .collect();
        let now = crate::time::Timestamp::parse("2026-08-29T04:02:00Z")
            .expect("specimen")
            .epoch();
        // The LAST appended event whose ACTOR is the agent — an inbound event
        // aimed at it is not its own activity, whatever its timestamp says.
        assert_eq!(
            last_actor_event_age(&events, "demo", "main", "opus5:builder", now),
            60
        );
        assert_eq!(
            last_actor_event_age(&events, "demo", "main", "nobody:here", now),
            super::NO_EVENT_AGE,
            "no event at all is the sentinel, not an age"
        );
        // A same-display event under another incarnation's routing key is not
        // this seat's activity.
        let routed: Vec<Event> = [
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"lead","action":"state","ref":"waiting-agent","actor_slot":"main","actor_session":"alpha"}"#,
            r#"{"ts":"2026-08-29T04:01:00Z","actor":"lead","action":"state","ref":"working","actor_slot":"main","actor_session":"beta"}"#,
        ]
        .iter()
        .map(|line| Event::parse_line(line).expect("specimen"))
        .collect();
        assert_eq!(
            last_actor_event_age(&routed, "alpha", "main", "lead", now),
            120,
            "alpha's own 04:00 declaration, not beta's newer 04:01 working"
        );
        assert_eq!(
            last_actor_event_age(&routed, "beta", "main", "lead", now),
            60,
            "and beta's incarnation still owns its own event"
        );
    }

    #[test]
    fn the_bar_has_only_three_faces_and_ranks_dead_over_stale() {
        for icons in [true, false] {
            assert_eq!(bar_glyph(0, 0, icons), Verdict::Active.glyph(icons));
            assert_eq!(bar_glyph(0, 3, icons), Verdict::Stale.glyph(icons));
            assert_eq!(bar_glyph(1, 3, icons), Verdict::Dead.glyph(icons));
            assert_eq!(bar_glyph(1, 0, icons), Verdict::Dead.glyph(icons));
        }
        // A dead pane keeps its own mark, and it outranks everything: a bar
        // showing a finish while a process is gone would be a lie.
        assert_eq!(bar_glyph(1, 3, true), Mark::Dead.glyph(true));
        assert_ne!(bar_glyph(1, 0, true), bar_glyph(0, 3, true));
    }

    /// Eleven verdicts, seven marks: the mapping is the whole vocabulary the
    /// status bar, the pane borders and the picker share.
    #[test]
    fn every_verdict_maps_onto_one_of_the_marks() {
        for (verdict, mark, reason) in [
            (Verdict::Dead, Mark::Dead, "dead"),
            (
                Verdict::Quiet(QuietKind::WaitingUser),
                Mark::NeedsYou,
                "waiting-user",
            ),
            (
                // A fresh waiting-agent is quiet with its own mark; the
                // escalated form arrives as `Quiet(Blocked)` and draws
                // NeedsYou instead.
                Verdict::Quiet(QuietKind::WaitingAgent),
                Mark::WaitingAgent,
                "waiting-agent",
            ),
            (
                Verdict::Quiet(QuietKind::Blocked),
                Mark::NeedsYou,
                "blocked",
            ),
            (Verdict::Throttled, Mark::NeedsYou, "throttled"),
            // The vendor's own usage limit: the existing NeedsYou mark, a new
            // word. A published VALUE, not a format.
            (Verdict::Limit, Mark::NeedsYou, "limit"),
            (Verdict::Quiet(QuietKind::Done), Mark::Done, "done"),
            (Verdict::Stale, Mark::Stale, "stale"),
            (Verdict::Active, Mark::Working, "working"),
            (
                Verdict::Meta(SweepVerdict::MetaSweeping),
                Mark::Working,
                "sweeping",
            ),
            (
                Verdict::Meta(SweepVerdict::MetaWedged),
                Mark::NeedsYou,
                "wedged",
            ),
            (
                Verdict::Meta(SweepVerdict::MetaStarting),
                Mark::Stale,
                "starting",
            ),
        ] {
            assert_eq!(verdict.mark(), mark, "{verdict:?}");
            assert_eq!(verdict.reason(), reason, "{verdict:?}");
            assert_eq!(verdict.glyph(true), mark.glyph(true), "{verdict:?}");
            assert_eq!(verdict.glyph(false), mark.glyph(false), "{verdict:?}");
        }
    }

    #[test]
    fn every_mark_publishes_the_frozen_glyph_and_its_ascii_fallback() {
        assert_eq!(Mark::Dead.glyph(true), "✖");
        assert_eq!(Mark::Dead.glyph(false), "x");
        assert_eq!(Mark::NeedsYou.glyph(true), "⚠");
        assert_eq!(Mark::Working.glyph(true), "●");
        assert_eq!(Mark::WaitingAgent.glyph(true), "◔");
        assert_eq!(Mark::Done.glyph(true), "✓");
        assert_eq!(Mark::Stale.glyph(true), "◌");
        assert_eq!(Mark::Idle.glyph(true), "·");
        assert_eq!(Mark::NeedsYou.glyph(false), "!");
        assert_eq!(Mark::Working.glyph(false), "*");
        assert_eq!(Mark::WaitingAgent.glyph(false), "~");
        assert_eq!(Mark::Done.glyph(false), "+");
        assert_eq!(Mark::Stale.glyph(false), "?");
        assert_eq!(Mark::Idle.glyph(false), "-");
    }

    /// A roster entry as `meta` records one.
    fn entry(slot: &str, alias: &str, name: &str) -> RosterEntry {
        RosterEntry {
            slot: slot.to_owned(),
            name: name.to_owned(),
            profile: Some(alias.to_owned()),
            client: RecordedClient::Missing,
            harness_session: None,
            config_home: crate::meta::RecordedConfigHome::Missing,
            config_home_base: crate::meta::RecordedConfigHomeBase::Missing,
            binary: None,
        }
    }

    #[test]
    fn window_agents_name_one_or_many_and_follow_the_look() {
        let one = vec![("act".to_owned(), Mark::Done)];
        assert_eq!(
            window_agents_line(&one, &look(), None),
            "#[fg=#7fbf6a]✓#[default]act"
        );

        let many = vec![
            ("lead".to_owned(), Mark::Done),
            ("colead".to_owned(), Mark::Working),
        ];
        assert_eq!(
            window_agents_line(&many, &look(), None),
            "#[fg=#7fbf6a]✓#[default]lead #[fg=#57b6c2]●#[default]colead"
        );
        let frame = crate::theme::WorkingFrame {
            glyph: "⠙",
            fg: "#57b6c2".to_owned(),
        };
        assert_eq!(
            window_agents_line(&many, &look(), Some(&frame)),
            "#[fg=#7fbf6a]✓#[default]lead #[fg=#57b6c2]⠙#[default]colead"
        );
        let ascii = Look {
            icons: false,
            ..look()
        };
        let ascii_frame = crate::theme::WorkingFrame {
            glyph: "/",
            fg: "#57b6c2".to_owned(),
        };
        assert_eq!(
            window_agents_line(&many, &ascii, Some(&ascii_frame)),
            "#[fg=#7fbf6a]+#[default]lead #[fg=#57b6c2]/#[default]colead"
        );

        let needs_you = vec![("lead".to_owned(), Mark::NeedsYou)];
        assert!(
            window_agents_line(&needs_you, &Look::DEFAULT, None)
                .contains("#[fg=#CC7832]⚠#[default]lead")
        );
    }

    #[test]
    fn a_hostile_seat_name_cannot_style_the_window_entry() {
        let agents = vec![(
            crate::theme::agent_label("evil#[bg=red,fg=black]"),
            Mark::Working,
        )];
        let drawn = window_agents_line(&agents, &look(), None);
        assert!(!drawn.contains("#[bg=red"), "{drawn}");
        assert!(drawn.contains("evil"), "{drawn}");
    }

    /// A daemon that has never READ a look publishes nothing that depends on
    /// one — no verdicts, no fleet strip, and above all no restamped windows.
    ///
    /// The carry's start is asserted directly; the ORDER is read off the source,
    /// because reaching `close` needs a live server and the property is about
    /// which statement comes first.
    #[test]
    fn a_daemon_that_has_never_read_a_look_publishes_nothing() {
        let knobs = Knobs::default();
        assert_eq!(
            Carry::new(&knobs).look,
            None,
            "a default look would be the same guess the fallback exists to avoid"
        );
        let source = include_str!("watchdog_daemon.rs");
        let close = source
            .split_once("fn close(")
            .map(|(_, body)| body.split_once("\n    }\n").map_or(body, |(head, _)| head))
            .expect("close is defined in this file");
        let bail = close
            .find("return Ok(());")
            .expect("close gives up when no look has ever answered");
        let publish = close
            .find(concat!("self.", "publish("))
            .expect("close publishes the cycle");
        assert!(
            bail < publish,
            "the give-up must come BEFORE anything look-dependent is published"
        );
        assert!(
            close.contains("carry.look = Some(look);"),
            "and a successful read is remembered for the next failed one"
        );
    }

    #[test]
    fn automatic_upgrade_scheduling_is_on_a_verdict_cycle_never_the_motion_ticker() {
        let source = include_str!("watchdog_daemon.rs");
        let cycle = source
            .split_once("fn run(&self, carry:")
            .and_then(|(_, tail)| tail.split_once("/// The cycle's last step"))
            .map(|(body, _)| body)
            .expect("Cycle::run is bounded by its close documentation");
        assert_eq!(
            cycle.matches("schedule_automatic_upgrade()").count(),
            1,
            "one scheduling edge follows each completed verdict cycle"
        );
        let ticker = source
            .split_once("fn wait_between_cycles(")
            .and_then(|(_, tail)| tail.split_once("/// The branch publication"))
            .map(|(body, _)| body)
            .expect("the motion ticker is bounded by tick_pane_duties");
        assert!(
            !ticker.contains("autoupgrade"),
            "the 100 ms motion ticker must never schedule a child"
        );
    }

    /// The drawn name is backfilled for EVERY pane, monitor panes included,
    /// before the filter that drops them from the mark rollup.
    #[test]
    fn the_drawn_name_is_backfilled_ahead_of_the_agent_filter() {
        let source = include_str!("watchdog_daemon.rs");
        let windows = source
            .split_once("fn publish_windows(")
            .map(|(_, body)| body.split_once("\n    }\n").map_or(body, |(head, _)| head))
            .expect("publish_windows is defined in this file");
        let label = windows
            .find("AGENT_LABEL_OPTION")
            .expect("the cycle backfills the drawn name");
        let filter = windows
            .find("NON_AGENT_PANES.contains")
            .expect("the cycle drops the monitor's own panes from the rollup");
        assert!(
            label < filter,
            "a monitor pane is filtered out of the MARKS and still draws a border title, \
             so its label must be written before the filter"
        );
    }

    /// The look every window-agent assertion below is written against.
    fn look() -> Look {
        Look {
            palette: crate::theme::Palette::NEUTRAL,
            ..Look::DEFAULT
        }
    }

    #[test]
    fn the_session_rollup_is_keyed_by_slot_not_by_the_display_ref() {
        let roster = [
            entry("main", "cl", "lead"),
            entry("worker.0", "cl", "twin"),
            entry("spawned.0", "cl", "twin"),
        ];
        // Two registrations SHARE a display ref and differ only by slot — the
        // case that makes ref-keying wrong.
        let by_slot = vec![
            ("main".to_owned(), Verdict::Active),
            ("worker.0".to_owned(), Verdict::Stale),
            ("spawned.0".to_owned(), Verdict::Quiet(QuietKind::Done)),
        ];
        let marks: Vec<Mark> = roster
            .iter()
            .map(|entry| slot_mark(entry, &by_slot, &[]))
            .collect();
        assert_eq!(marks, [Mark::Working, Mark::Stale, Mark::Done]);
    }

    #[test]
    fn picker_agents_fact_keeps_roster_order_and_replaces_missing_panes() {
        let roster = [
            entry("main", "fable5", "lead"),
            entry("worker.0", "gpt56sol", "builder"),
            entry("spawned.0", "gpt56luna", "tests"),
        ];
        let observed = vec![
            live_seat("worker.0", "%8", Verdict::Quiet(QuietKind::Done)),
            live_seat("main", "%3", Verdict::Active),
        ];
        let clients = [
            ("main", "cc".to_owned()),
            ("worker.0", "cx".to_owned()),
            ("spawned.0", "cx".to_owned()),
        ];
        assert_eq!(
            agents_fact(&roster, &observed, &clients, 2_000, 60),
            Some(
                "v2;2000;60;lead:fable5:working:%3:cc:::;builder:gpt56sol:done:%8:cx:::;\
                 tests:gpt56luna:dead::cx:::"
                    .to_owned()
            ),
            "a seat ae observed nothing about still names the client it is \
             recorded as running"
        );
        assert_eq!(agents_fact(&roster, &observed, &clients, 2_000, 0), None);
        assert_eq!(
            agents_fact(&roster, &observed, &clients, 2_000, 3_601),
            None
        );
        assert_eq!(
            crate::theme::AGENTS_OPTION,
            "@ae_agents",
            "one session-scoped fact, never per-agent options"
        );
    }

    /// One roster seat with a recorded client override and binary, for the
    /// client-cell resolver. The name derives from the slot.
    fn label_entry(
        slot: &str,
        profile: Option<&str>,
        client: RecordedClient,
        binary: Option<&str>,
    ) -> RosterEntry {
        RosterEntry {
            slot: slot.to_owned(),
            name: format!("{}-agent", slot.replace('.', "-")),
            profile: profile.map(str::to_owned),
            client,
            harness_session: None,
            config_home: crate::meta::RecordedConfigHome::Missing,
            config_home_base: crate::meta::RecordedConfigHomeBase::Missing,
            binary: binary.map(str::to_owned),
        }
    }

    /// The client cell shows the operator's own `[clients]` label — recorded
    /// override first (the precedence `run::read_seat` honors), then the
    /// profile's row, then the recorded binary name, then today's short code.
    /// Display-only: no rung writes the meta, and every rung that cannot spell
    /// its answer falls to the next rather than refusing the seat.
    #[test]
    fn seat_client_label_honors_override_profile_binary_and_token_in_order() {
        let long = "l".repeat(40);
        let cfg = crate::config::parse_identity(&format!(
            "[clients]\nclaude = claude\ncc-mic = claude config_home=$HOME/.claude-mic\n\
             codex = codex\n{long} = codex\n[profiles]\np1 = claude --model fable\n\
             p2 = cc-mic --model fable\np3 = /usr/bin/Muse --model fable\n\
             p4 = nosuchclient --x\np5 = {long} --full-auto\n"
        ))
        .expect("the fixture parses");
        let some = Some(&cfg);
        let missing = || RecordedClient::Missing;
        let label = |text: &str| RecordedClient::Label(text.to_owned());
        let at = |profile, client: RecordedClient, binary: Option<&str>| {
            super::seat_client_label(&label_entry("main", profile, client, binary), some)
        };
        // Headline: one binary, two config homes, two different labels.
        assert_eq!(at(Some("p1"), missing(), Some("claude")), "claude");
        assert_eq!(at(Some("p2"), missing(), Some("claude")), "cc-mic");
        for (profile, client, binary, expected, why) in [
            (
                Some("p1"),
                label("cc-mic"),
                Some("claude"),
                "cc-mic",
                "override",
            ),
            (
                Some("p1"),
                label("bad label"),
                Some("claude"),
                "claude",
                "hostile",
            ),
            (
                Some("p1"),
                RecordedClient::Invalid,
                Some("claude"),
                "claude",
                "bad row",
            ),
            (
                Some("deleted"),
                missing(),
                Some("claude"),
                "claude",
                "dead profile",
            ),
            (
                Some("p3"),
                missing(),
                Some("claude"),
                "claude",
                "path profile",
            ),
            (
                Some("p4"),
                missing(),
                Some("codex"),
                "codex",
                "unknown word",
            ),
            (
                Some("p5"),
                missing(),
                Some("codex"),
                "codex",
                "40-cell label",
            ),
            (None, missing(), Some("codex"), "codex", "no profile"),
            (Some("deleted"), missing(), None, "-", "nothing recorded"),
            (
                Some("deleted"),
                missing(),
                Some("weird binary!"),
                "-",
                "hostile bin",
            ),
            (
                Some("p1"),
                label("retired"),
                Some("claude"),
                "retired",
                "no live row",
            ),
        ] {
            let answered = at(profile, client, binary);
            assert_eq!(answered, expected, "{why}");
            let token = crate::tool::is_client_token(&answered);
            assert!(token || crate::config::is_client_label(&answered), "{why}");
        }
        // No config at all still names the recorded binary, never a guess.
        let entry = label_entry("main", Some("p1"), missing(), Some("claude"));
        assert_eq!(super::seat_client_label(&entry, None), "claude");
    }

    /// A roster profile carrying a `profile@client` spelling publishes no
    /// fact at all: the grammar admits no `@`, and a fact the picker would
    /// misread is worse than none.
    #[test]
    fn agents_fact_refuses_a_profile_at_spelling() {
        let roster = [entry("main", "fablex@cc-mic", "lead")];
        let observed = vec![live_seat("main", "%3", Verdict::Active)];
        assert_eq!(
            agents_fact(&roster, &observed, &[("main", "cc".to_owned())], 2_000, 60),
            None
        );
    }

    /// One live pane's contribution, with nothing observed about its model.
    fn live_seat(slot: &str, pane: &str, verdict: Verdict) -> AgentObservation {
        AgentObservation {
            slot: slot.to_owned(),
            pane: pane.to_owned(),
            verdict,
            identity: SeatIdentity::default(),
        }
    }

    /// The same, carrying what that pane's frame proved.
    fn seen_running(
        slot: &str,
        pane: &str,
        model: &str,
        effort: Option<&str>,
        drift: bool,
    ) -> AgentObservation {
        AgentObservation {
            slot: slot.to_owned(),
            pane: pane.to_owned(),
            verdict: Verdict::Active,
            identity: SeatIdentity {
                model: Some(model.to_owned()),
                effort: effort.map(str::to_owned),
                drift,
            },
        }
    }

    #[test]
    fn the_agents_fact_emits_v2_and_empties_an_unrepresentable_model() {
        let roster = [
            entry("main", "fable5", "lead"),
            entry("worker.0", "spark13m", "nav"),
            entry("spawned.0", "ocds", "runner"),
        ];
        let observed = vec![
            seen_running("main", "%3", "Fable 5.1", Some("xhigh"), true),
            seen_running("worker.0", "%8", "muse-spark-1.3", Some("max"), false),
            // A vendor label carrying the fact's OWN separator. Nothing here
            // may escape it: the entry loses its whole observed trio and keeps
            // everything ae actually recorded.
            seen_running("spawned.0", "%9", "weird:model", Some("high"), false),
        ];
        let clients = [
            ("main", "cc-mic".to_owned()),
            ("worker.0", "muse".to_owned()),
            ("spawned.0", "oc".to_owned()),
        ];
        let fact = agents_fact(&roster, &observed, &clients, 2_000, 60).expect("a v2 fact");
        assert_eq!(
            fact,
            "v2;2000;60;lead:fable5:working:%3:cc-mic:Fable 5.1:xhigh:!;\
             nav:spark13m:working:%8:muse:muse-spark-1.3:max:;\
             runner:ocds:working:%9:oc:::"
        );
        // The writer and the reader are one contract: whatever this publishes
        // must survive the strict parser that reads it back.
        let parsed = crate::tmux::parse_picker_agents(&fact, 2_000).expect("its own reader");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].client, "cc-mic");
        assert_eq!(parsed[0].model, "Fable 5.1");
        assert!(parsed[0].drift);
        assert_eq!(parsed[2].client, "oc", "the client survives the drop");
        assert_eq!(
            (
                parsed[2].model.as_str(),
                parsed[2].effort.as_str(),
                parsed[2].drift
            ),
            ("", "", false),
            "a model ae cannot spell takes its effort and drift mark with it"
        );
        // Every other unrepresentable model is dropped the same way, and none
        // of them refuses the roster.
        for hostile in [
            "semi;colon",
            "pipe|bar",
            "comma,model",
            "style#[fg=red]",
            " leading",
            "trailing ",
            "",
            &"M".repeat(crate::tmux::PICKER_AGENTS_MAX_MODEL + 1),
        ] {
            let one = [entry("main", "fable5", "lead")];
            let observation = vec![seen_running("main", "%3", hostile, Some("max"), false)];
            assert_eq!(
                agents_fact(&one, &observation, &[("main", "cc".to_owned())], 2_000, 60),
                Some("v2;2000;60;lead:fable5:working:%3:cc:::".to_owned()),
                "{hostile:?}"
            );
        }
        // At the cap exactly, it is published.
        let at_cap = "M".repeat(crate::tmux::PICKER_AGENTS_MAX_MODEL);
        let one = [entry("main", "fable5", "lead")];
        let observation = vec![seen_running("main", "%3", &at_cap, None, false)];
        assert_eq!(
            agents_fact(&one, &observation, &[("main", "cc".to_owned())], 2_000, 60),
            Some(format!("v2;2000;60;lead:fable5:working:%3:cc:{at_cap}::"))
        );
    }

    /// A full 64-seat roster whose name and profile are each `width` wide.
    ///
    /// One seat's entry costs `name + profile + 13` at v1 (state `working`,
    /// pane `%1`, four separators), `+ 6` for the client cell, `+ 6` more for
    /// the model, and `+ 6` more for the effort and drift mark. Against the
    /// 4096-byte bound that width alone decides which rung is first to fit.
    fn roster_of(width: usize) -> Vec<RosterEntry> {
        (0..64)
            .map(|index| {
                let tail = "x".repeat(width - 3);
                entry(
                    &format!("s{index:02}"),
                    &format!("p{index:02}{tail}"),
                    &format!("a{index:02}{tail}"),
                )
            })
            .collect()
    }

    /// Every seat of `roster` observed with a model, an effort and a drift mark.
    fn observed_of(roster: &[RosterEntry]) -> Vec<AgentObservation> {
        roster
            .iter()
            .map(|entry| seen_running(&entry.slot, "%1", "Opus 5", Some("xhigh"), true))
            .collect()
    }

    /// Every seat of `roster` on one client.
    fn clients_of(roster: &[RosterEntry]) -> Vec<(&str, String)> {
        roster
            .iter()
            .map(|entry| (entry.slot.as_str(), "cc".to_owned()))
            .collect()
    }

    /// The ladder, at the 64-seat limit, rung by rung.
    ///
    /// The property is one sentence: model cells must never be the reason a
    /// roster vanishes. Each width below is chosen so exactly one rung is the
    /// first that fits, and the last rung before nothing is today's v1 bytes.
    #[test]
    fn a_roster_that_cannot_fit_its_model_cells_degrades_before_it_vanishes() {
        for (width, expected, why) in [
            (8_usize, "v2", "everything fits"),
            (18, "v2", "the effort and the drift mark go first"),
            (21, "v2", "then the models"),
            (24, "v1", "then the whole v2 shape, back to today's bytes"),
        ] {
            let roster = roster_of(width);
            let clients = clients_of(&roster);
            let fact = agents_fact(&roster, &observed_of(&roster), &clients, 2_000, 60)
                .unwrap_or_else(|| panic!("width {width}: {why}"));
            assert!(
                fact.starts_with(&format!("{expected};")),
                "width {width} ({why}): {}",
                &fact[..fact.len().min(40)]
            );
            assert!(
                fact.len() <= crate::tmux::PICKER_AGENTS_MAX_BYTES,
                "width {width}"
            );
            assert_eq!(
                crate::tmux::parse_picker_agents(&fact, 2_000).map(|agents| agents.len()),
                Some(64),
                "width {width}: every rung publishes the WHOLE roster"
            );
        }
        // Rung by rung, the first that fits is the one taken.
        let roster = roster_of(18);
        let clients = clients_of(&roster);
        let observed = observed_of(&roster);
        let at = |rung| fact_at(&roster, &observed, &clients, 2_000, 60, rung);
        assert_eq!(at(FactRung::Full), None, "the full fact does not fit here");
        assert!(at(FactRung::NoEffort).is_some());
        assert_eq!(
            agents_fact(&roster, &observed, &clients, 2_000, 60),
            at(FactRung::NoEffort),
            "the ladder takes the first rung that fits, not a plainer one"
        );
        // The legacy rung is EXACTLY today's writer: no client cell, no
        // observed cells, no extra separators.
        let legacy = fact_at(&roster, &observed, &clients, 2_000, 60, FactRung::Legacy)
            .expect("the legacy rung");
        let expected = format!(
            "v1;2000;60;{}",
            roster
                .iter()
                .map(|entry| format!(
                    "{}:{}:working:%1",
                    entry.name,
                    entry.profile.as_deref().unwrap_or_default()
                ))
                .collect::<Vec<_>>()
                .join(";")
        );
        assert_eq!(legacy, expected, "the last rung is today's bytes");
        // Past every rung there is still nothing to publish, exactly as
        // before: 64 seats this wide cannot be spelled at all.
        let enormous = roster_of(26);
        let clients = clients_of(&enormous);
        assert_eq!(
            agents_fact(&enormous, &observed_of(&enormous), &clients, 2_000, 60),
            None
        );
    }

    /// The rung-3 to rung-4 edge, and what tips it.
    ///
    /// At width 24 the client-only rung is over the bound while the legacy
    /// rung is under it, and the whole difference between them is the six
    /// bytes an entry spends on `:<client>:::`. A roster that fits v1 lands on
    /// v1, never on nothing.
    #[test]
    fn the_client_cell_is_what_tips_the_last_v2_rung_onto_the_legacy_one() {
        let roster = roster_of(24);
        let clients = clients_of(&roster);
        let observed = observed_of(&roster);
        let at = |rung| fact_at(&roster, &observed, &clients, 2_000, 60, rung);
        let legacy = at(FactRung::Legacy).expect("v1 fits at the tipping width");
        assert_eq!(at(FactRung::ClientOnly), None, "the client cell tips it");
        assert!(legacy.len() <= crate::tmux::PICKER_AGENTS_MAX_BYTES);
        assert!(
            legacy.len() + 64 * ":cc:::".len() > crate::tmux::PICKER_AGENTS_MAX_BYTES,
            "and those six bytes an entry are the whole of the difference"
        );
    }

    /// A cycle whose only argument is the capture, so the hold and the gate
    /// can be driven one frame at a time.
    fn identity_cycle<'a>(
        scratch: &'a Scratch,
        helper: &'a SendHelper,
        server: &'a ServerId,
        launch: &str,
    ) -> Cycle<'a> {
        Cycle {
            knobs: Knobs::default(),
            meta_dir: &scratch.0,
            helper,
            server,
            session: "demo",
            goal: None,
            roster: vec![entry("main", "fable5", "lead")],
            local_config: None,
            lead_pair: false,
            fleet_order: crate::theme::FleetOrder::EMPTY,
            meta_agent: false,
            launch_ids: vec![("main".to_owned(), launch.to_owned())],
        }
    }

    /// One resolve, spelled once so the tests below read as frames.
    fn resolve(
        cycle: &Cycle<'_>,
        carried: &mut PaneState,
        capture: &str,
        pin: Option<&str>,
        verdict: Verdict,
    ) -> SeatIdentity {
        cycle.resolve_identity(
            carried,
            &ResolveIdentity {
                capture,
                tool: crate::tool::ToolKind::Claude,
                slot: "main",
                pin,
                verdict,
            },
        )
    }

    #[test]
    fn a_held_identity_survives_an_unreadable_frame_and_expires_by_age_incarnation_or_death() {
        let live = include_str!("../tests/fixtures/harness-state/claude-idle-167x40.txt");
        let scratch = Scratch::new("identity-hold");
        let helper = SendHelper::for_session(&scratch.0);
        let server = ServerId::Ambient;
        let cycle = identity_cycle(&scratch, &helper, &server, "L1");

        let mut carried = PaneState::default();
        let proven = resolve(&cycle, &mut carried, live, None, Verdict::Active);
        assert!(proven.model.is_some(), "the live fixture proves a model");
        assert_eq!(
            carried.held_identity.as_ref().map(|hold| hold.age),
            Some(0),
            "the cycle that observed it holds it at age zero"
        );

        // An unreadable frame is the ORDINARY case — a turn in flight, a draft
        // in the box, one failed capture — and must not flap the cell.
        for cycles in 1..=HOLD_MAX_CYCLES {
            assert_eq!(
                resolve(&cycle, &mut carried, "", None, Verdict::Active),
                proven,
                "unreadable cycle {cycles} still shows what the seat proved"
            );
            assert_eq!(
                carried.held_identity.as_ref().map(|hold| hold.age),
                Some(cycles)
            );
        }
        // One cycle past the bound, ae stops asserting a label nobody can
        // re-confirm.
        assert_eq!(
            resolve(&cycle, &mut carried, "", None, Verdict::Active),
            SeatIdentity::default(),
            "the {HOLD_MAX_CYCLES}th unreadable cycle is the last one held"
        );
        assert!(carried.held_identity.is_none());

        // A seat that came back under a NEW conversation inherits nothing: a
        // retire plus a respawn can reuse the slot, the agent name and the
        // pane id, so only the launch id separates the two.
        let mut carried = PaneState::default();
        let proven = resolve(&cycle, &mut carried, live, None, Verdict::Active);
        let respawned = identity_cycle(&scratch, &helper, &server, "L2");
        assert_eq!(
            resolve(&respawned, &mut carried, "", None, Verdict::Active),
            SeatIdentity::default(),
            "a new incarnation starts from nothing"
        );
        assert!(carried.held_identity.is_none());

        // Whatever a dead seat was running, it is not running it now.
        let mut carried = PaneState::default();
        assert_eq!(
            resolve(&cycle, &mut carried, live, None, Verdict::Active),
            proven
        );
        assert_eq!(
            resolve(&cycle, &mut carried, live, None, Verdict::Dead),
            SeatIdentity::default(),
            "a dead verdict clears the hold even on a readable frame"
        );
        assert!(carried.held_identity.is_none());

        // A seat with NO recorded launch id is shown what this cycle proves
        // and nothing longer: there is no incarnation to guard a hold with.
        let unguarded = identity_cycle(&scratch, &helper, &server, "");
        let mut carried = PaneState::default();
        let mut unguarded_cycle = unguarded;
        unguarded_cycle.launch_ids = Vec::new();
        assert_eq!(
            resolve(&unguarded_cycle, &mut carried, live, None, Verdict::Active),
            proven
        );
        assert!(carried.held_identity.is_none(), "nothing to hold it under");
        assert_eq!(
            resolve(&unguarded_cycle, &mut carried, "", None, Verdict::Active),
            SeatIdentity::default()
        );
    }

    #[test]
    fn the_drift_mark_needs_a_pin_to_disagree_with() {
        let live = include_str!("../tests/fixtures/harness-state/claude-idle-167x40.txt");
        let scratch = Scratch::new("identity-drift");
        let helper = SendHelper::for_session(&scratch.0);
        let server = ServerId::Ambient;
        let cycle = identity_cycle(&scratch, &helper, &server, "L1");
        let mut carried = PaneState::default();
        let proven = resolve(&cycle, &mut carried, live, None, Verdict::Active);
        let model = proven.model.clone().expect("the fixture's model");

        // No pin: the profile has no model flag to disagree with, so this is a
        // report, not a drift.
        assert!(!proven.drift);
        let mut carried = PaneState::default();
        assert!(
            !resolve(&cycle, &mut carried, live, Some(&model), Verdict::Active).drift,
            "the pin and the frame agree"
        );
        let mut carried = PaneState::default();
        assert!(
            resolve(
                &cycle,
                &mut carried,
                live,
                Some("Opus 4.8"),
                Verdict::Active
            )
            .drift,
            "the seat is running something its profile does not pin"
        );
        // The mark is held with the rest of the identity, so a busy seat does
        // not lose its drift warning mid-turn.
        assert!(resolve(&cycle, &mut carried, "", Some("Opus 4.8"), Verdict::Active).drift);
    }

    #[test]
    fn the_drift_mark_reads_a_claude_pin_through_family_version() {
        let idle = include_str!("../tests/fixtures/harness-state/claude-idle-167x40.txt");
        let busy = include_str!("../tests/fixtures/harness-state/claude-busy-nbsp-tip-101x41.txt");
        let scratch = Scratch::new("identity-drift-family");
        let helper = SendHelper::for_session(&scratch.0);
        let server = ServerId::Ambient;
        let cycle = identity_cycle(&scratch, &helper, &server, "L1");

        // The idle frame draws `Fable 5.1`: a `fable` pin satisfies it.
        let mut carried = PaneState::default();
        let proven = resolve(&cycle, &mut carried, idle, Some("fable"), Verdict::Active);
        assert_eq!(proven.model.as_deref(), Some("Fable 5.1"));
        assert!(!proven.drift);

        // The busy frame draws `Opus 5 (1M context)`: a `fable` pin does not.
        let mut carried = PaneState::default();
        let proven = resolve(&cycle, &mut carried, busy, Some("fable"), Verdict::Active);
        assert_eq!(proven.model.as_deref(), Some("Opus 5 (1M context)"));
        assert!(proven.drift);
    }

    /// The gate is the CYCLE's, not just the classifier's: a mutant that
    /// swapped this call site back to the ungated read would publish a model
    /// from a frame whose composer is gone.
    #[test]
    fn the_cycle_resolves_a_quoted_frame_as_unobserved() {
        let muse = include_str!("../tests/fixtures/runtime-identity/muse-idle-plain-80x24.txt");
        let quoted: String = muse
            .lines()
            .filter(|line| !line.trim_start().starts_with('\u{276f}'))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            crate::harness_state::current_identity(&quoted, crate::tool::ToolKind::Muse)
                .model
                .is_some(),
            "the ungated grammar still reads this frame, which is the whole \
             point: only the gate refuses it"
        );
        let scratch = Scratch::new("identity-quoted");
        let helper = SendHelper::for_session(&scratch.0);
        let server = ServerId::Ambient;
        let cycle = identity_cycle(&scratch, &helper, &server, "L1");
        let mut carried = PaneState::default();
        let resolved = cycle.resolve_identity(
            &mut carried,
            &ResolveIdentity {
                capture: &quoted,
                tool: crate::tool::ToolKind::Muse,
                slot: "main",
                pin: None,
                verdict: Verdict::Active,
            },
        );
        assert_eq!(resolved, SeatIdentity::default());
        assert!(
            carried.held_identity.is_none(),
            "a frame ae will not trust never becomes one it holds"
        );
    }

    #[test]
    fn a_slot_with_no_pane_is_neutral_on_its_first_absent_cycle_and_dead_on_its_second() {
        let roster = [entry("main", "cl", "lead"), entry("worker.0", "cl", "w")];
        let by_slot = vec![("main".to_owned(), Verdict::Active)];
        // First absence: the debounce has not recorded it yet.
        assert_eq!(slot_mark(&roster[1], &by_slot, &[]), Mark::Idle);
        // Second: the streak is recorded, and now it wants a human.
        let missing = vec![(
            "worker.0".to_owned(),
            MissingState {
                streak: 1,
                alerted: true,
            },
        )];
        assert_eq!(slot_mark(&roster[1], &by_slot, &missing), Mark::NeedsYou);
        // The debounce is keyed by SLOT: a streak against some other slot must
        // not make this one say so.
        let elsewhere = vec![(
            "spawned.9".to_owned(),
            MissingState {
                streak: 4,
                alerted: true,
            },
        )];
        assert_eq!(slot_mark(&roster[1], &by_slot, &elsewhere), Mark::Idle);
    }

    /// The session's own mark is the most actionable of its panes'.
    #[test]
    fn the_session_mark_is_the_rollup_of_its_panes() {
        let pane = |pane: &str, verdict| PaneMark {
            pane: pane.to_owned(),
            verdict,
            observed: "unknown".to_owned(),
        };
        assert_eq!(session_mark(&[], &[]), Mark::Idle);
        assert_eq!(
            session_mark(
                &[
                    pane("%1", Verdict::Quiet(QuietKind::Done)),
                    pane("%2", Verdict::Active),
                ],
                &[]
            ),
            Mark::Working
        );
        assert_eq!(
            session_mark(
                &[
                    pane("%1", Verdict::Active),
                    pane("%2", Verdict::Quiet(QuietKind::WaitingUser)),
                ],
                &[]
            ),
            Mark::NeedsYou,
            "one agent waiting on the human makes the whole session say so"
        );
        assert_eq!(
            session_mark(
                &[pane("%1", Verdict::Stale), pane("%2", Verdict::Active)],
                &[]
            ),
            Mark::Stale
        );
        // A slot whose PANE is gone still says needs-you in the session
        // rollup; a missing pane must not make the fleet strip look calm.
        assert_eq!(
            session_mark(
                &[pane("%1", Verdict::Active)],
                &[Mark::Idle, Mark::NeedsYou]
            ),
            Mark::NeedsYou,
            "a missing agent must not leave the fleet strip calling the session calm"
        );
        assert_eq!(
            session_mark(&[], &[Mark::Idle]),
            Mark::Idle,
            "a roster that is merely quiet says nothing"
        );
    }

    #[test]
    fn an_empty_window_agent_list_composes_to_nothing() {
        // The publisher clears an empty option so an ordinary tmux window
        // falls back to its own name.
        assert!(window_agents_line(&[], &look(), None).is_empty());
    }

    #[test]
    fn a_failed_session_query_does_not_end_the_daemon() {
        // THE BLOCKER THIS FIXES.
        assert_eq!(continuation(None, &StopProbe::Unknown), Continuation::Retry);
        // Retry is the only verdict that leaves the publication standing, which
        // is exactly why it must not collapse into Stop.
        assert_ne!(
            continuation(None, &StopProbe::Unknown),
            Continuation::Stop,
            "an unreachable server is not a dead session"
        );
    }

    #[test]
    fn a_proven_absent_session_ends_the_daemon() {
        assert_eq!(continuation(None, &StopProbe::Absent), Continuation::Stop);
        // Proof of the session's death outranks any meta reading: the thing
        // being watched is gone whatever its directory says.
        assert_eq!(
            continuation(Some(ErrorKind::PermissionDenied), &StopProbe::Absent),
            Continuation::Stop
        );
    }

    #[test]
    fn a_transient_meta_error_retries_while_a_missing_meta_ends_it() {
        // `meta::rewrite` publishes through a temp file and rename, so `meta` is
        // never MOMENTARILY absent during a write — NotFound means teardown took
        // it, which is the loop's other self-termination condition.
        assert_eq!(
            continuation(Some(ErrorKind::NotFound), &StopProbe::Present),
            Continuation::Stop
        );
        for transient in [
            ErrorKind::PermissionDenied,
            ErrorKind::Interrupted,
            ErrorKind::WouldBlock,
            ErrorKind::Other,
        ] {
            assert_eq!(
                continuation(Some(transient), &StopProbe::Present),
                Continuation::Retry,
                "{transient:?} is a failed read, not a dead session"
            );
        }
    }

    #[test]
    fn only_a_good_reading_of_both_runs_a_cycle() {
        // The whole matrix, so a later edit cannot quietly widen Run or Stop.
        for (meta, probe, expected) in [
            (None, StopProbe::Present, Continuation::Run),
            (None, StopProbe::Absent, Continuation::Stop),
            (None, StopProbe::Unknown, Continuation::Retry),
            (
                Some(ErrorKind::NotFound),
                StopProbe::Present,
                Continuation::Stop,
            ),
            (
                Some(ErrorKind::NotFound),
                StopProbe::Unknown,
                Continuation::Stop,
            ),
            (
                Some(ErrorKind::PermissionDenied),
                StopProbe::Present,
                Continuation::Retry,
            ),
            (
                Some(ErrorKind::PermissionDenied),
                StopProbe::Unknown,
                Continuation::Retry,
            ),
            (
                Some(ErrorKind::PermissionDenied),
                StopProbe::Absent,
                Continuation::Stop,
            ),
        ] {
            assert_eq!(
                continuation(meta, &probe),
                expected,
                "meta {meta:?} + session {probe:?}"
            );
        }
    }

    #[test]
    fn the_session_is_the_meta_key_or_the_directory_name() {
        let dir = Path::new("/home/x/.ae/sessions/demo");
        assert_eq!(session_name(b"mode=local\nsession=real\n", dir), "real");
        assert_eq!(
            session_name(b"mode=local\n", dir),
            "demo",
            "no key: the directory IS the name"
        );
        assert_eq!(
            session_name(b"session=\n", dir),
            "demo",
            "an empty key is not a name"
        );
    }

    // -- the orchestrator sweep branch ------------------------------------------

    /// A scratch directory, for the one reading this module takes from the
    /// filesystem.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ae-wd-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch");
            Self(dir)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_700_000_000 + secs)
    }

    #[test]
    fn the_orchestrator_main_is_judged_by_its_cadence_and_nothing_below_it() {
        // The CONTROL is the pair: one Observation, judged twice.
        let knobs = Knobs::default();
        let (prior, ordinary) = stale_pane();
        let plain = account(&prior, &ordinary, &knobs);
        assert_eq!(plain.verdict, Verdict::Stale);
        assert_eq!(plain.effects, vec![Effect::Nudge]);

        let mut orchestrator = ordinary.clone();
        orchestrator.sweep = Some(SweepObservation::new(at(0), None));
        let booked = account(&prior, &orchestrator, &knobs);
        assert_eq!(booked.verdict, Verdict::Meta(SweepVerdict::MetaStarting));
        assert_eq!(
            booked.effects,
            vec![Effect::SweepNudge],
            "the cadence prompts; it never nudges for a state declaration"
        );
    }

    #[test]
    fn a_disabled_cadence_returns_the_orchestrator_to_the_ordinary_watchdog() {
        // From the daemon's side: `sweep_step` answering `None` must FALL
        // THROUGH, not suppress.
        let knobs = Knobs {
            sweep: crate::watchdog::SweepKnobs {
                sweep_secs: 0,
                ..crate::watchdog::SweepKnobs::default()
            },
            ..Knobs::default()
        };
        let (prior, mut observed) = stale_pane();
        observed.sweep = Some(SweepObservation::new(at(0), None));
        let booked = account(&prior, &observed, &knobs);
        assert_eq!(booked.verdict, Verdict::Stale);
        assert_eq!(booked.effects, vec![Effect::Nudge]);
    }

    #[test]
    fn a_dead_orchestrator_is_dead_before_it_is_a_cadence() {
        // Branch order: the dead check runs BEFORE the sweep branch, so a
        // orchestrator that dropped to a shell still alerts instead of being
        // reported as starting up forever.
        let knobs = Knobs::default();
        let mut observed = seen();
        observed.is_dead = true;
        observed.descendancy = Descendancy::Absent;
        observed.sweep = Some(SweepObservation::new(at(0), None));
        let booked = account(&PaneState::default(), &observed, &knobs);
        assert_eq!(booked.verdict, Verdict::Dead);
        assert!(booked.next.dead_latched);
    }

    /// The sweep verdicts fold into the shared marks like every other verdict:
    /// the orchestrator's own eye glyph is gone, because a status bar with a
    /// private symbol for one session's one seat is a bar nobody can read.
    #[test]
    fn the_sweep_verdicts_render_the_shared_marks() {
        assert_eq!(
            Verdict::Meta(SweepVerdict::MetaSweeping).glyph(true),
            Mark::Working.glyph(true)
        );
        assert_eq!(
            Verdict::Meta(SweepVerdict::MetaWedged).glyph(true),
            Mark::NeedsYou.glyph(true)
        );
        assert_eq!(
            Verdict::Meta(SweepVerdict::MetaStarting).glyph(true),
            Mark::Stale.glyph(true)
        );
    }

    #[test]
    fn every_sweep_decision_renders_as_this_loops_effects() {
        // A rendering step, never a policy one: the alert transition carries
        // its own frozen text, and this only unpacks it.
        assert_eq!(
            sweep_effects(vec![SweepEffect::FireSweepNudge]),
            vec![Effect::SweepNudge]
        );
        assert_eq!(
            sweep_effects(vec![SweepEffect::ReconcileWedge]),
            vec![Effect::ReconcileWedge]
        );
        assert_eq!(
            sweep_effects(vec![SweepEffect::Alert(SweepAlert::RaiseWedge(
                WedgeDetail::Stalled { age_secs: 700 }
            ))]),
            vec![
                Effect::Emit {
                    action: "alert",
                    summary: "meta-agent not acknowledging overviews — oldest outstanding \
                              overview unacknowledged for 11m (may be stuck)"
                        .to_owned(),
                },
                Effect::Notify(
                    "(meta-agent) not acknowledging overviews — may be stuck".to_owned()
                ),
            ]
        );
        assert_eq!(
            sweep_effects(vec![SweepEffect::Alert(SweepAlert::ClearUnreachable)]),
            vec![Effect::Emit {
                action: "alert-cleared",
                summary: "meta-agent reachable again (sweep nudge delivered)".to_owned(),
            }],
            "a clear is log-only — no display-message"
        );
    }

    #[test]
    fn a_changed_overview_carries_the_rendered_text_as_the_nudge_body() {
        let reading = OverviewReading {
            rendered: "WORKING\n  alpha lead ship".to_owned(),
            hash: "0123456789abcdef".to_owned(),
            checkpoint: crate::monitor::OverviewCheckpoint {
                hash: Some("fedcba9876543210".to_owned()),
                outstanding_since: Some(240),
                last_delivered_at: Some(300),
            },
        };
        assert!(reading.changed());
        assert_eq!(
            reading.body(),
            "WORKING\n  alpha lead ship\n— overview; declare done."
        );
        assert_eq!(
            reading.persisted_last_delivery(),
            Some(UNIX_EPOCH + Duration::from_mins(5))
        );
        assert_eq!(
            reading.persisted_outstanding_since(),
            Some(UNIX_EPOCH + Duration::from_mins(4))
        );
    }

    #[test]
    fn a_done_after_delivery_stays_healthy_beyond_startup_grace() {
        let knobs = Knobs {
            sweep: crate::watchdog::SweepKnobs {
                sweep_secs: 120,
                ..crate::watchdog::SweepKnobs::default()
            },
            ..Knobs::default()
        };
        let mut prior = PaneState::default();
        prior.sweep.outstanding_since = Some(at(0));
        prior.sweep.unacknowledged_deliveries = 1;
        prior.sweep.last_sweep = Some(at(240));
        let mut observed = seen();
        observed.sweep = Some(SweepObservation::new(at(301), Some(at(300))));

        let booked = account(&prior, &observed, &knobs);

        assert_eq!(booked.verdict, Verdict::Meta(SweepVerdict::MetaSweeping));
        assert!(
            !booked.effects.iter().any(|effect| matches!(
                effect,
                Effect::Emit {
                    action: "alert",
                    ..
                }
            )),
            "the done acknowledgement prevents a wedge after the 300s grace: {:?}",
            booked.effects
        );
    }

    #[test]
    fn persisted_sweep_cadence_outranks_env_then_the_internal_default() {
        assert_eq!(sweep_seconds(b"sweep_sec=120\n", Some("240"), 300), 120);
        assert_eq!(sweep_seconds(b"session=x\n", Some("240"), 300), 240);
        assert_eq!(sweep_seconds(b"session=x\n", None, 300), 300);
        assert_eq!(
            sweep_seconds(b"sweep_sec=nope\n", Some("240"), 300),
            240,
            "an invalid persisted value does not shadow a valid fallback"
        );
        assert_eq!(
            sweep_seconds(b"sweep_sec=120\nsweep_sec=60\n", Some("240"), 300),
            240,
            "a duplicate persisted fact is not authoritative"
        );
        assert_eq!(
            sweep_seconds(b"session=x\n", Some("nope"), 300),
            300,
            "an invalid environment fallback does not replace the default"
        );
        assert_eq!(sweep_seconds(b"sweep_sec=0\n", Some("240"), 300), 0);
        assert_eq!(
            sweep_seconds(b"sweep_sec=1\n", Some("240"), 300),
            60,
            "a positive cadence cannot nudge more than once per normal verdict cycle"
        );
        assert_eq!(sweep_seconds(b"session=x\n", Some("59"), 300), 60);
    }

    #[test]
    fn the_orchestrator_flag_grants_only_the_unambiguous_shared_role() {
        // A session that gets the sweep branch stops being escalated for
        // silence, so the flag is EXACTLY ONE record saying EXACTLY `true`.
        let cases: [(&str, bool); 15] = [
            ("session=x\nmeta_agent=true\n", true),
            ("session=x\n", false),
            ("meta_agent\n", false),
            // A bare record makes the claim damaged even beside a valid value.
            ("meta_agent\nmeta_agent=true\n", false),
            ("meta_agent=true\nmeta_agent\n", false),
            ("meta_agent=\n", false),
            ("meta_agent=false\n", false),
            ("meta_agent=truth\n", false),
            ("meta_agent=TRUE\n", false),
            ("meta_agent=1\n", false),
            ("meta_agent=yes\n", false),
            ("meta_agent=true\r\n", false),
            ("meta_agent=true\nmeta_agent=false\n", false),
            ("meta_agent=true\nmeta_agent=true\n", false),
            ("meta_agent=false\nmeta_agent=true\n", false),
        ];
        let observed: Vec<(&str, bool)> = cases
            .iter()
            .map(|(meta, _)| (*meta, is_meta_agent(meta.as_bytes())))
            .collect();
        assert_eq!(
            observed,
            cases.to_vec(),
            "meta_agent authority observations"
        );
    }

    #[test]
    fn the_overview_acknowledgement_is_the_main_agents_newest_done_event() {
        let events: Vec<Event> = [
            r#"{"ts":"2026-09-07T09:00:00Z","actor":"seat:main","action":"state","ref":"done"}"#,
            r#"{"ts":"2026-09-07T09:01:00Z","actor":"seat:worker","action":"state","ref":"done"}"#,
            r#"{"ts":"2026-09-07T09:02:00Z","actor":"seat:main","action":"memo","ref":"x"}"#,
            r#"{"ts":"2026-09-07T09:04:00Z","actor":"seat:main","action":"done","actor_slot":"worker.0","actor_session":"orchestrator"}"#,
            r#"{"ts":"2026-09-07T09:03:00Z","actor":"seat:main","action":"done"}"#,
        ]
        .iter()
        .map(|line| Event::parse_line(line).expect("specimen"))
        .collect();
        let epoch = crate::time::Timestamp::parse("2026-09-07T09:03:00Z")
            .expect("specimen")
            .epoch();
        let expected = UNIX_EPOCH + Duration::from_secs(u64::try_from(epoch).expect("positive"));

        assert_eq!(
            last_done_event_at(&events, "orchestrator", "seat:main"),
            Some(expected),
            "a routed worker with the same display name is not the main slot"
        );
        assert_eq!(last_done_event_at(&events, "orchestrator", "nobody"), None);
    }

    #[test]
    fn only_the_main_agents_newest_working_declaration_holds_an_overview() {
        let events: Vec<Event> = [
            r#"{"ts":"2026-09-07T09:00:00Z","actor":"seat:main","action":"state","ref":"working"}"#,
            r#"{"ts":"2026-09-07T09:01:00Z","actor":"seat:main","action":"state","ref":"working","actor_slot":"worker.0","actor_session":"orchestrator"}"#,
        ]
        .iter()
        .map(|line| Event::parse_line(line).expect("specimen"))
        .collect();
        let expected_epoch = crate::time::Timestamp::parse("2026-09-07T09:00:00Z")
            .expect("specimen")
            .epoch();
        let expected = system_time_from_epoch(expected_epoch);

        assert_eq!(
            last_working_declaration_at(&events, "orchestrator", "seat:main"),
            expected,
            "a routed worker with the same display name cannot hold the main seat"
        );

        for latest in [
            r#"{"ts":"2026-09-07T09:02:00Z","actor":"seat:main","action":"state","ref":"done"}"#,
            r#"{"ts":"2026-09-07T09:02:00Z","actor":"seat:main","action":"state","ref":"blocked"}"#,
        ] {
            let mut superseded = events.clone();
            superseded.push(Event::parse_line(latest).expect("specimen"));
            assert_eq!(
                last_working_declaration_at(&superseded, "orchestrator", "seat:main"),
                None,
                "only the newest declaration defines the current state"
            );
        }
        assert_eq!(
            last_working_declaration_at(&[], "orchestrator", "seat:main"),
            None,
            "an idle seat declares no working hold"
        );
    }

    #[test]
    fn the_observation_server_follows_the_record_the_send_helper_reads() {
        // The drift both reviewers named: delivery goes through
        // `<meta-dir>/send`, whose `_lib` re-reads `tmux_server` from the
        // CURRENT meta on every call.
        let named =
            |value: &str| Meta::parse(&format!("tmux_server_kind=name\ntmux_server={value}\n"));
        let alpha = ServerId::Selected(Selector::Name("alpha".to_owned()));
        let beta = ServerId::Selected(Selector::Name("beta".to_owned()));

        // `Use` means a REAL MOVE and nothing else.
        assert_eq!(rebind(&alpha, Some(&named("alpha"))), Rebind::Keep);
        assert_eq!(
            rebind(&alpha, Some(&named("beta"))),
            Rebind::Use(beta.clone()),
            "observation follows the session exactly as delivery does"
        );
        // The CONTROL is the pair: one input, two current servers, two answers
        // — so the decision reads both, rather than echoing what it was handed.
        assert_ne!(
            rebind(&alpha, Some(&named("beta"))),
            rebind(&beta, Some(&named("beta")))
        );

        // A socket selector is a DIFFERENT identity from a name, even when a
        // human can see they address the same tmux — so it is a move.
        assert_eq!(
            rebind(
                &alpha,
                Some(&Meta::parse(
                    "tmux_server_kind=socket\ntmux_server=/tmp/s\n"
                ))
            ),
            Rebind::Use(ServerId::Selected(Selector::Socket("/tmp/s".into())))
        );

        // Missing or ambiguous: stop, mirroring the startup refusal.
        assert_eq!(
            rebind(&alpha, Some(&Meta::parse("session=d\n"))),
            Rebind::Refuse
        );
        assert_eq!(
            rebind(
                &alpha,
                Some(&Meta::parse(
                    "tmux_server=a\ntmux_server=b\ntmux_server_kind=name\n"
                ))
            ),
            Rebind::Refuse,
            "a duplicated selector is ambiguous"
        );
        assert_eq!(
            rebind(
                &alpha,
                Some(&Meta::parse("tmux_server_kind=socket\ntmux_server=rel\n"))
            ),
            Rebind::Refuse,
            "a relative socket path names no server"
        );

        // An unreadable meta says NOTHING about the server.
        assert_eq!(rebind(&alpha, None), Rebind::Keep);
    }

    /// A carry loaded with one server's history, on pane ids the next server
    /// will reuse.
    fn loaded(knobs: &Knobs) -> Carry {
        let mut carry = Carry::new(knobs);
        for pane_id in ["%0", "%1", "%2"] {
            let state = entry_mut(&mut carry.panes, pane_id);
            state.dead_latched = true;
            state.nudge_count = 2;
            state.undelivered_streak = 3;
            state.throttle_streak = 4;
            state.prev_hash = Some(99);
            state.last_hash_change = Some(1_000);
            state.quiet_base = Some(("alpha-declaration".to_owned(), 99, 0));
            state.sweep.wedge_alerted = true;
            state.sweep.unreachable_alerted = true;
            state.sweep.fails = 5;
        }
        entry_mut(&mut carry.missing, "main").alerted = true;
        // SPEND the budget: an unspent cycle wraps the cursor to 0 by design,
        // which would make the reset assertion below vacuous.
        for idx in 0..knobs.quiet_panes_per_cycle {
            assert!(carry.quiet.step(idx), "the budget allows pane {idx}");
        }
        carry.quiet.end(5);
        assert_ne!(carry.quiet.cursor(), 0, "the cursor really did move");
        carry
    }

    /// Every assertion that `%0` on the NEW server starts from nothing.
    fn assert_neutral(carry: &mut Carry) {
        assert!(carry.panes.is_empty(), "no pane history survives the move");
        assert!(
            carry.missing.is_empty(),
            "nor the missing-pane debounce, which latches for the daemon's life"
        );
        assert_eq!(carry.quiet.cursor(), 0, "nor the stabilization rotation");
        let fresh = entry_mut(&mut carry.panes, "%0").clone();
        assert_eq!(fresh, PaneState::default());
        assert!(!fresh.dead_latched, "a live pane is not inherited dead");
        assert_eq!(fresh.nudge_count, 0, "nor mid-way through its nudges");
        assert_eq!(fresh.undelivered_streak, 0);
        assert_eq!(fresh.throttle_streak, 0);
        assert_eq!(fresh.prev_hash, None, "the quiet baseline re-arms");
        assert_eq!(fresh.quiet_base, None);
        assert_eq!(
            fresh.sweep,
            crate::watchdog::SweepState::default(),
            "and the sweep cadence starts over rather than resuming another \
             server's wedge"
        );
    }

    #[test]
    fn a_server_move_retracts_the_old_bars_and_carries_no_pane_history() {
        // THE TWO DEFECTS the re-review found, together.
        let scratch = Scratch::new("adopt");
        let knobs = Knobs::default();
        let journal = Journal {
            meta_dir: &scratch.0,
            session: "demo",
        };
        let alpha = ServerId::Selected(Selector::Name("alpha".to_owned()));
        let beta = ServerId::Selected(Selector::Name("beta".to_owned()));
        let mut carry = loaded(&knobs);
        let mut retracted: Vec<ServerId> = Vec::new();
        let mut err = Vec::new();

        let now = adopt_server(
            alpha.clone(),
            beta.clone(),
            &mut carry,
            &knobs,
            |leaving| {
                retracted.push(leaving.clone());
                true
            },
            &journal,
            &mut err,
        )
        .expect("the move reports only a write failure, and there is none");

        assert_eq!(now, beta, "the daemon is on the new server");
        assert_eq!(
            retracted,
            vec![alpha],
            "and it retracted from the OLD one, while it could still address it"
        );
        assert_neutral(&mut carry);
        assert!(
            err.is_empty(),
            "a retraction that worked says nothing: {}",
            String::from_utf8_lossy(&err)
        );
    }

    #[test]
    fn an_unreachable_old_server_does_not_block_the_move() {
        // The failure boundary.
        let scratch = Scratch::new("adopt-dead");
        let knobs = Knobs::default();
        let journal = Journal {
            meta_dir: &scratch.0,
            session: "demo",
        };
        let alpha = ServerId::Selected(Selector::Name("alpha".to_owned()));
        let beta = ServerId::Selected(Selector::Name("beta".to_owned()));
        let mut carry = loaded(&knobs);
        let mut err = Vec::new();

        let now = adopt_server(
            alpha,
            beta.clone(),
            &mut carry,
            &knobs,
            |_| false, // the old server is gone
            &journal,
            &mut err,
        )
        .expect("an unreachable old server is not a write failure");

        assert_eq!(now, beta, "adoption PROCEEDS");
        assert_neutral(&mut carry);

        // And it is not silent — twice over, because a stderr line in a
        // detached daemon is a line nobody reads.
        let said = String::from_utf8_lossy(&err).into_owned();
        assert!(
            said.contains("could not retract"),
            "the diagnostic reaches stderr: {said:?}"
        );
        let recorded = read_events(&scratch.0);
        assert_eq!(recorded.len(), 1, "exactly one durable diagnostic");
        let event = &recorded[0];
        assert_eq!(event.action, "alert");
        assert_eq!(event.target.as_deref(), Some(ACTOR));
        assert!(
            event
                .summary
                .as_deref()
                .unwrap_or_default()
                .contains("could not clear its options on the old one"),
            "and it says what happened: {:?}",
            event.summary
        );
    }

    /// The recorded identity both caller pins are rendered under: one codex
    /// seat whose conversation and config home are both on record.
    fn quota_recorded_entry(rollout: &str) -> RosterEntry {
        RosterEntry {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: Some("sol".to_owned()),
            client: RecordedClient::Missing,
            harness_session: Some(rollout.to_owned()),
            config_home: RecordedConfigHome::Path(PathBuf::from("/tmp/cx")),
            config_home_base: RecordedConfigHomeBase::Missing,
            binary: Some("codex".to_owned()),
        }
    }

    #[test]
    fn an_older_explicit_spend_report_never_lifts_a_newer_held_cap() {
        let rollout = "018f1f70-7b2c-7000-8000-000000000001";
        let entry = quota_recorded_entry(rollout);
        let identity = crate::quota::recorded_identity(&entry).expect("recorded identity");
        let account = |reached: Option<bool>, at: i64| crate::quota::Account {
            credits: crate::quota::Credits::Unreported,
            credits_observed_at: None,
            spend_control_reached: reached,
            spend_observed_at: Some(at),
        };
        let observation = |at: i64, account: crate::quota::Account, now: i64| {
            quota_observation(
                vec![quota_scope_group(
                    &identity.source,
                    Some(rollout),
                    Some("demo:lead"),
                    vec![quota_row(
                        "codex",
                        Some("pro"),
                        "3",
                        at,
                        crate::quota::Status::Fresh,
                    )],
                    Some(1),
                    account,
                )],
                now,
            )
        };
        let recipients = [quota_recipient("main", "lead")];
        let dir = Path::new("/m");

        // A cap proven at 9901 makes a 3% window critical: no window reset
        // frees it, and one declared reset cannot either.
        let mut carry = QuotaCarry::default();
        let _ = carry.reconcile(
            &observation(9_901, account(Some(true), 9_901), 10_000),
            &recipients,
            dir,
        );
        assert_eq!(
            carry.tracked[0].classified.level(),
            QuotaLevel::Critical,
            "a proven spend cap is critical at any raw percentage"
        );

        // The refused older observation carries an explicit `false` stamped
        // EARLIER than the cap. The field's own provenance decides, not the
        // order the records reached ae.
        let older = observation(9_800, account(Some(false), 9_800), 10_001);
        let actions = carry.reconcile(&older, &recipients, dir);
        assert_eq!(
            carry.tracked[0].classified.level(),
            QuotaLevel::Critical,
            "an older explicit false loses to the newer held cap"
        );
        assert!(
            actions.is_empty(),
            "so nothing transitions and nothing is booked: {actions:?}"
        );
        let line = throttle_quota_line(&older, &carry.tracked, &entry, dir, 10_001)
            .expect("a line for the recorded scope");
        assert!(
            line.contains("spend cap reached"),
            "and the cap is still what a seat is told: {line}"
        );

        // Control: a genuinely newer explicit false does lift it.
        let lifted = carry.reconcile(
            &observation(9_901, account(Some(false), 9_902), 10_002),
            &recipients,
            dir,
        );
        assert_eq!(
            carry.tracked[0].classified.level(),
            QuotaLevel::Headroom,
            "a newer explicit false is the one report that lifts a cap"
        );
        let booked = transition_deliveries(&lifted);
        assert_eq!(booked.len(), 1, "one relieved notice: {booked:?}");
        let text = booked[0].advisory.render(dir, 10_002);
        assert!(text.contains("back to headroom"), "{text}");
    }

    #[test]
    fn the_throttle_line_quotes_the_observation_that_decided_the_level() {
        let rollout = "018f1f70-7b2c-7000-8000-000000000001";
        let entry = quota_recorded_entry(rollout);
        let identity = crate::quota::recorded_identity(&entry).expect("recorded identity");
        let observation = |used: &str, at: i64, resets: Option<u8>, now: i64| {
            quota_observation(
                vec![quota_scope_group(
                    &identity.source,
                    Some(rollout),
                    Some("demo:lead"),
                    vec![quota_row(
                        "codex",
                        Some("pro"),
                        used,
                        at,
                        crate::quota::Status::Fresh,
                    )],
                    resets,
                    crate::quota::Account::default(),
                )],
                now,
            )
        };
        let recipients = [quota_recipient("main", "lead")];
        let dir = Path::new("/m");

        // Control: an ordinary current observation renders itself.
        let mut control = QuotaCarry::default();
        let current = observation("95", 9_901, Some(0), 10_000);
        let _ = control.reconcile(&current, &recipients, dir);
        let line = throttle_quota_line(&current, &control.tracked, &entry, dir, 10_000)
            .expect("a line for the recorded scope");
        assert!(line.contains(" 95% (critical,"), "control: {line}");

        // Held 95% at 9901, then the older 20% at 9800 the raw clock refuses.
        // The level comes from the held observation, so the number, the
        // derivation and the age must come from it too.
        let mut carry = QuotaCarry::default();
        let _ = carry.reconcile(&observation("95", 9_901, Some(1), 10_000), &recipients, dir);
        let older = observation("20", 9_800, Some(0), 10_001);
        let _ = carry.reconcile(&older, &recipients, dir);
        assert_eq!(carry.tracked[0].classified.level(), QuotaLevel::Critical);
        let line = throttle_quota_line(&older, &carry.tracked, &entry, dir, 10_001)
            .expect("a line for the recorded scope");
        assert!(
            line.contains(" 95% (critical,"),
            "the level and the number come from one observation: {line}"
        );
        assert!(
            line.contains("observed 1m ago"),
            "and so does its age: {line}"
        );
        assert!(
            !line.contains("20%"),
            "the refused observation is never rendered: {line}"
        );
        assert!(!line.contains("observed 3m ago"), "{line}");
    }

    #[test]
    fn one_scope_read_through_two_rollouts_merges_their_account_facts() {
        let source = Path::new("/tmp/cx/sessions");
        let scope_group = |rollout: &str, at: i64, account: crate::quota::Account| {
            quota_scope_group(
                source,
                Some(rollout),
                Some("demo:lead"),
                vec![quota_row(
                    "codex",
                    Some("pro"),
                    "3",
                    at,
                    crate::quota::Status::Fresh,
                )],
                None,
                account,
            )
        };
        let capped = crate::quota::Account {
            credits: crate::quota::Credits::Unreported,
            credits_observed_at: None,
            spend_control_reached: Some(true),
            spend_observed_at: Some(9_900),
        };
        let recipients = [quota_recipient("main", "lead")];
        let dir = Path::new("/m");

        // Two rollouts under ONE config home report one account. The newer
        // record carries the window and no account facts at all, so it says
        // nothing about the cap its sibling proved.
        let mut carry = QuotaCarry::default();
        let _ = carry.reconcile(
            &quota_observation(
                vec![
                    scope_group("018f1f70-7b2c-7000-8000-000000000001", 9_900, capped),
                    scope_group(
                        "018f1f70-7b2c-7000-8000-000000000002",
                        9_901,
                        crate::quota::Account::default(),
                    ),
                ],
                10_000,
            ),
            &recipients,
            dir,
        );
        assert_eq!(carry.tracked.len(), 1, "one scope is one tracked window");
        assert_eq!(
            carry.tracked[0].classified.level(),
            QuotaLevel::Critical,
            "the cap survives a sibling rollout that reports no account"
        );
        assert_eq!(
            carry.tracked[0].classified.observed_at(),
            9_901,
            "while the newest observation is still the one held"
        );

        // Control: with neither rollout reporting a cap, 3% is headroom.
        let mut control = QuotaCarry::default();
        let _ = control.reconcile(
            &quota_observation(
                vec![
                    scope_group(
                        "018f1f70-7b2c-7000-8000-000000000001",
                        9_900,
                        crate::quota::Account::default(),
                    ),
                    scope_group(
                        "018f1f70-7b2c-7000-8000-000000000002",
                        9_901,
                        crate::quota::Account::default(),
                    ),
                ],
                10_000,
            ),
            &recipients,
            dir,
        );
        assert_eq!(control.tracked[0].classified.level(), QuotaLevel::Headroom);
    }
}
