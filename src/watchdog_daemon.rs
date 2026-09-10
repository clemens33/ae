//! The watchdog daemon — the loop that observes a session's panes each cycle,
//! asks [`crate::watchdog`] what it is looking at, and applies the answers.

use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::digest::Status;
use crate::events::Event;
use crate::harness_state::HarnessState;
use crate::meta::{Meta, RosterEntry, ServerSelector};
use crate::procs::{self, Descendancy};
use crate::store;
use crate::theme::{self, Look, Mark};
use crate::time::Timestamp;
use crate::tmux::{self, OptionScope, StopProbe};
use crate::tracked::{self, EventFields};
use crate::transport;
use crate::watchdog::{
    QuietCycle, QuietKind, QuietPane, SweepAlert, SweepEffect, SweepKnobs, SweepObservation,
    SweepState, SweepVerdict, classify_dead, declaration_key, is_sweep_target,
    latest_relevant_event, quiet_hash, quiet_pane_decision, quiet_reason, quiet_stabilize,
    record_sweep, shows_throttle, stale_composite, sweep_step,
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
            idle_nudge_secs: 300,
            stale_secs: 900,
            max_nudges: 2,
            throttle_alert_cycles: 5,
            undelivered_max: 3,
            quiet_beat_ms: 1000,
            quiet_tries: 4,
            quiet_panes_per_cycle: 2,
            sweep: SweepKnobs::default(),
            tg_supervise_secs: 120,
        }
    }
}

/// What one pane carries from cycle to cycle, gathered into one value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneState {
    /// The slot+agent generation this carry belongs to.
    pub identity: Option<u64>,
    /// Dead is LATCHED: once alerted, the pane is skipped every later cycle and
    /// there is no watchdog-emitted clear.
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
    /// [`shows_throttle`]'s answer.
    pub is_throttled: bool,
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
    /// SIX marks for ten verdicts: the accent and the reason word beside it
    /// carry the difference, and a status bar that spent a distinct glyph on
    /// each verdict asked its reader to learn a private alphabet. A gone
    /// process keeps its own mark, because "this will never move again" is not
    /// the same news as "this is waiting for you".
    #[must_use]
    pub const fn mark(self) -> Mark {
        match self {
            Self::Dead => Mark::Dead,
            Self::Quiet(QuietKind::WaitingUser | QuietKind::Blocked)
            | Self::Throttled
            | Self::Meta(SweepVerdict::MetaWedged) => Mark::NeedsYou,
            Self::Quiet(QuietKind::Done) => Mark::Done,
            Self::Idle => Mark::Idle,
            Self::Stale | Self::Meta(SweepVerdict::MetaStarting) => Mark::Stale,
            Self::Active | Self::Meta(SweepVerdict::MetaSweeping) => Mark::Working,
        }
    }

    /// The word the pane border prints after the glyph.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Dead => "dead",
            Self::Quiet(QuietKind::Done) => "done",
            Self::Quiet(QuietKind::WaitingUser) => "waiting-user",
            Self::Quiet(QuietKind::Blocked) => "blocked",
            Self::Throttled => "throttled",
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
        /// `alert` / `throttled` / `throttle-cleared`.
        action: &'static str,
        /// The event summary.
        summary: String,
    },
    /// Deliver one nudge through the session's own send helper.
    Nudge,
    /// A line for the human, published with `display-message`.
    Notify(String),
    /// Deliver one SWEEP prompt to the orchestrator.
    SweepNudge,
    /// Reconcile the durable event log against a wedge alert this daemon does
    /// not remember raising — the post-restart clear.
    ReconcileWedge,
}

/// Advisory state for one stable vendor quota key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum QuotaLevel {
    Headroom,
    Low,
    Critical,
}

impl QuotaLevel {
    const fn label(self) -> &'static str {
        match self {
            Self::Headroom => "headroom",
            Self::Low => "low",
            Self::Critical => "critical",
        }
    }
}

/// Labels are not identity: the canonical source and vendor dimensions are.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct QuotaKey {
    source: std::path::PathBuf,
    rollout: Option<String>,
    bucket: String,
    qualifier: Option<String>,
    window_minutes: Option<u32>,
}

#[derive(Debug, Clone)]
struct QuotaSample<'a> {
    key: QuotaKey,
    group: &'a crate::quota::Group,
    row: &'a crate::quota::Row,
    used: f64,
    observed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QuotaTracked {
    key: QuotaKey,
    observed_at: i64,
    level: QuotaLevel,
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum QuotaAction {
    Deliver(Box<PendingAdvisory>),
    Dropped { recipient: String, summary: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuotaDelivery {
    Delivered,
    Retryable,
    Uncertain,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct QuotaCarry {
    sweeps_until_observe: u64,
    tracked: Vec<QuotaTracked>,
    pending: Vec<PendingAdvisory>,
    last_observation: Option<crate::quota::Observation>,
}

fn classify_quota(used: f64, prior: Option<QuotaLevel>) -> QuotaLevel {
    match prior {
        Some(QuotaLevel::Critical) if used >= 90.0 => QuotaLevel::Critical,
        Some(QuotaLevel::Critical | QuotaLevel::Low) if used >= 75.0 => {
            if used >= 95.0 {
                QuotaLevel::Critical
            } else {
                QuotaLevel::Low
            }
        }
        _ if used >= 95.0 => QuotaLevel::Critical,
        _ if used >= 80.0 => QuotaLevel::Low,
        _ => QuotaLevel::Headroom,
    }
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
            let (Some(used), Some(observed_at)) = (
                row.used_percent
                    .as_deref()
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|value| value.is_finite()),
                row.observed_at,
            ) else {
                continue;
            };
            if now.saturating_sub(observed_at) > 60 * 60 {
                continue;
            }
            let key = QuotaKey {
                source: source.clone(),
                rollout: group.rollout.clone(),
                bucket: row.bucket.clone(),
                qualifier: row.qualifier.clone(),
                window_minutes: row.window_minutes,
            };
            let sample = QuotaSample {
                key: key.clone(),
                group,
                row,
                used,
                observed_at,
            };
            if let Some(existing) = samples.iter_mut().find(|sample| sample.key == key) {
                if sample.observed_at > existing.observed_at {
                    *existing = sample;
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

    fn reconcile(
        &mut self,
        observation: &crate::quota::Observation,
        recipients: &[QuotaRecipient],
        meta_dir: &Path,
    ) -> Vec<QuotaAction> {
        let mut actions = Vec::new();
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
            observation.now,
            &mut actions,
        );

        for sample in samples {
            let previous = self
                .tracked
                .iter()
                .position(|tracked| tracked.key == sample.key);
            let Some(index) = previous else {
                self.tracked.push(QuotaTracked {
                    key: sample.key,
                    observed_at: sample.observed_at,
                    level: classify_quota(sample.used, None),
                });
                continue;
            };
            if sample.observed_at <= self.tracked[index].observed_at {
                continue;
            }
            let before = self.tracked[index].level;
            let after = classify_quota(sample.used, Some(before));
            self.tracked[index].observed_at = sample.observed_at;
            self.tracked[index].level = after;
            if before == after {
                continue;
            }
            self.cancel_where(
                |pending| pending.key == sample.key,
                "superseded by newer quota transition",
                meta_dir,
                observation.now,
                &mut actions,
            );
            let state = if after == QuotaLevel::Headroom {
                "back to headroom"
            } else {
                after.label()
            };
            let advisory = observation.advisory(sample.group, sample.row, state);
            for recipient in recipients {
                self.pending.push(PendingAdvisory {
                    key: sample.key.clone(),
                    observed_at: sample.observed_at,
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
        self.last_observation = Some(observation.clone());
        actions
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
}

fn throttle_quota_line(
    observation: &crate::quota::Observation,
    tracked: &[QuotaTracked],
    entry: &RosterEntry,
    meta_dir: &Path,
    now: i64,
) -> Option<String> {
    let identity = crate::quota::recorded_identity(entry)?;
    let mut matches: Vec<QuotaSample<'_>> = quota_samples_at(observation, now)
        .into_iter()
        .filter(|sample| {
            sample.group.tool == identity.tool
                && sample.key.source == identity.source
                && sample.key.rollout == identity.rollout
        })
        .collect();
    matches.sort_by(|left, right| {
        let level = |sample: &QuotaSample<'_>| {
            tracked
                .iter()
                .find(|held| held.key == sample.key)
                .map_or_else(|| classify_quota(sample.used, None), |held| held.level)
        };
        let left_level = level(left);
        let right_level = level(right);
        right_level
            .cmp(&left_level)
            .then_with(|| right.used.total_cmp(&left.used))
            .then_with(|| left.key.cmp(&right.key))
    });
    let sample = matches.first()?;
    let level = tracked
        .iter()
        .find(|held| held.key == sample.key)
        .map_or_else(|| classify_quota(sample.used, None), |held| held.level);
    Some(observation.advisory_line_at(sample.group, sample.row, level.label(), meta_dir, now))
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
#[must_use]
pub fn nudge_text(goal: Option<&str>, meta_dir: &Path) -> String {
    let prefix = goal.map_or_else(String::new, |goal| format!("Session goal: {goal}. "));
    format!(
        "{prefix}Status check: if you have more work, continue. Otherwise declare your state so \
         I stop nudging: {}/state <waiting-user|blocked|done> \"<reason>\"",
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
         <waiting-user|blocked|done> \"<reason>\"",
        meta_dir.display()
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

/// Account for one pane in one cycle — the branch order, and the only place
/// any of it is decided.
#[must_use]
pub fn account(prior: &PaneState, seen: &Observation, knobs: &Knobs) -> Accounting {
    let reset = PaneState::default();
    let prior = if prior
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

    // 1. Already dead: no second alert, no further judgement.
    if prior.dead_latched {
        return Accounting {
            next,
            effects,
            verdict: Verdict::Dead,
            moved: false,
        };
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
    if !seen.is_throttled && prior.throttle_streak > 0 {
        effects.push(Effect::Emit {
            action: "throttle-cleared",
            summary: format!("throttling cleared after {} cycles", prior.throttle_streak),
        });
        next.throttle_streak = 0;
    }

    // 6.
    if let Some(kind) = seen.quiet {
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

    // 7.
    if seen.is_throttled {
        book_throttle(&mut next, &mut effects, seen, knobs);
        return Accounting {
            next,
            effects,
            verdict: Verdict::Throttled,
            moved: false,
        };
    }

    // 8. Harness frames outrank the legacy motion heuristic.
    if let Some(verdict) = account_harness(prior, &mut next, &mut effects, seen, knobs) {
        return Accounting {
            next,
            effects,
            verdict,
            moved: false,
        };
    }

    // 9. Unknown frames retain the legacy motion and actor-event rule.
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
            if knobs.idle_nudge_secs > 0 && idle_age >= knobs.idle_nudge_secs {
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

/// The age of the newest event this agent is the ACTOR of.
#[must_use]
pub fn last_actor_event_age(events: &[Event], agent: &str, now_epoch: i64) -> u64 {
    events
        .iter()
        .rev()
        .find(|event| event.actor == agent)
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

/// Resolve the session-pinned idle reminder cadence before the flag/default.
fn idle_nudge_seconds(meta_bytes: &[u8], fallback: u64) -> Result<u64, String> {
    pinned_seconds(meta_bytes, "idle_nudge_secs", fallback)
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
        sessions: &[tmux::FleetSession],
        session: &str,
    ) {
        self.panes = panes;
        self.replace_fleet(sessions, session);
    }

    /// Replace the fleet half of an observation and remember which exact
    /// session table owns the strip.
    fn replace_fleet(&mut self, sessions: &[tmux::FleetSession], session: &str) {
        self.fleet_target = sessions
            .iter()
            .find(|entry| entry.name == session)
            .map(|entry| entry.id.clone());
        self.fleet = sessions
            .iter()
            .map(|entry| theme::FleetRow {
                name: entry.name.clone(),
                id: entry.id.clone(),
                mark: Mark::from_rank(&entry.rank),
                current: entry.name == session,
            })
            .collect();
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
        let strip = theme::fleet_strip(look, &self.fleet, working_frame);
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
                &theme::pane_state_frame(&frame, "working"),
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
}

/// The fleet target the orchestrator segment may jump to: absent for this session
/// when it is itself the orchestrator, or when the fleet has none. The exact
/// canonical seat name is intentional: a renamed seat loses the click target.
fn orchestrator_id_for<'a>(sessions: &'a [tmux::FleetSession], session: &str) -> Option<&'a str> {
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
const fn motion_ticker_enabled(look: Look) -> bool {
    look.drawn && look.motion
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
                    let cycle = Cycle {
                        knobs,
                        meta_dir,
                        helper,
                        server: &server,
                        session,
                        goal: meta.goal().map(ToOwned::to_owned),
                        local_config: meta
                            .origin()
                            .and_then(|origin| crate::config::local_overlay(meta_dir, origin)),
                        lead_pair: crate::lifecycle::meta_value(bytes, "layout") == "lead-pair",
                        // Re-read EVERY cycle, like the goal and the roster: a
                        // session can be promoted to orchestrator, or its main
                        // replaced, while this daemon runs.
                        meta_agent: is_meta_agent(bytes),
                        roster: meta.roster().to_vec(),
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
    let Some(look) = carry.look.filter(|look| motion_ticker_enabled(*look)) else {
        std::thread::sleep(interval);
        return;
    };
    let started = Instant::now();
    let mut cadence = ATTACHED_MOTION_TICK;
    let mut ticks_since_observation = MOTION_OBSERVATION_TICKS;
    let mut failures = 0_u8;
    loop {
        let remaining = interval.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        let mut next = carry.motion.clone();
        let observed = motion_observation_due(ticks_since_observation);
        if observed {
            let reading = transport::observe_motion_panes(server, session);
            let fleet = transport::observe_fleet_sessions(server);
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
            next.replace_observation(reading, &fleet, session);
        }
        let writes = next.step(&look);
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
    // EVERY session-scoped value this daemon publishes. A fleet strip or an
    // attention rank left behind would keep asserting a session nobody is
    // watching — and every OTHER session on the server reads those two.
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
    /// This cycle's filtered pane hash.
    hash: u64,
    /// The pane's 1-based position in this cycle's traversal, for the budget.
    index: usize,
    /// The pane to re-capture while settling a baseline.
    pane_id: &'a str,
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
    /// `meta_agent=true` — this session is the fleet orchestrator.
    meta_agent: bool,
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
            }
        }
        Ok(())
    }

    fn refresh_quota(
        &self,
        carry: &mut QuotaCarry,
        now: i64,
        err: &mut impl Write,
    ) -> crate::Result<()> {
        if !quota_observation_due(carry, &self.knobs) {
            return Ok(());
        }
        match self.quota_observation(now) {
            Ok(observation) => {
                let recipients = quota_recipients(&self.roster, self.lead_pair);
                let actions = carry.reconcile(&observation, &recipients, self.meta_dir);
                self.apply_quota_actions(carry, actions, now, err)
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

    fn throttle_quota(&self, quota: &QuotaCarry, slot: &str, now: i64) -> Option<String> {
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

    /// One pass over the session's panes.
    fn run(&self, carry: &mut Carry, err: &mut impl Write) -> crate::Result<()> {
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
        self.refresh_quota(&mut carry.quota, now, err)?;

        carry.quiet.begin();
        let mut index = 0_usize;
        let mut live: Vec<String> = Vec::new();
        let mut counts = Counts::default();
        let mut by_slot: Vec<(String, Verdict)> = Vec::new();
        let mut by_agent: Vec<(String, String, Verdict)> = Vec::new();
        let mut by_pane: Vec<PaneMark> = Vec::new();

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

            // The main loop tolerates a failed capture: an unreadable pane
            // hashes as empty here.
            let capture = transport::capture_pane(self.server, &pane.pane_id).unwrap_or_default();
            let hash = quiet_hash(&capture);
            let is_throttled = shows_throttle(&capture, agent_bin.as_deref().unwrap_or_default());
            let throttle_quota = is_throttled
                .then(|| self.throttle_quota(&carry.quota, &slot, now))
                .flatten();
            let identity = quiet_hash(&format!("{slot}\n{agent}"));
            let carried = entry_mut(&mut carry.panes, &pane.pane_id);
            restore_idle(carried, &pane.observed, identity);
            let seen = Observation {
                now_epoch: now,
                hash,
                harness: self.harness_observation(&capture, tool, &events, &slot, agent),
                identity,
                is_dead: classify_dead(
                    &pane.current_command,
                    descendancy_of(table.as_deref(), pane.pane_pid, agent_bin.as_deref()),
                ),
                is_throttled,
                throttle_quota,
                quiet: self.resolve_quiet(
                    &QuietQuery {
                        events: &events,
                        agent,
                        hash,
                        index,
                        pane_id: &pane.pane_id,
                    },
                    carried,
                    &mut carry.quiet,
                ),
                descendancy: descendancy_of(table.as_deref(), pane.pane_pid, agent_bin.as_deref()),
                last_actor_event_age_secs: last_actor_event_age(&events, agent, now),
                // Decided HERE, once, and the type carries the answer: a pane
                // that is not the orchestrator main gets `None` and no sweep
                // branch can reach it.
                sweep: self.sweep_observation(&slot, agent, &events, overview.as_ref(), now),
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
            for effect in &booked.effects {
                self.apply(effect, &acting, carried, err)?;
            }
            counts.record(booked.verdict);
            by_slot.push((slot.clone(), booked.verdict));
            by_agent.push((slot, pane.pane_id.clone(), booked.verdict));
            by_pane.push(PaneMark {
                pane: pane.pane_id.clone(),
                verdict: booked.verdict,
                observed: observed_option(seen.harness.frame, carried),
            });
        }
        carry.quiet.end(index);
        self.close(
            carry, &counts, &by_slot, &by_agent, &by_pane, &live, now, err,
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
        by_agent: &[(String, String, Verdict)],
        by_pane: &[PaneMark],
        live: &[String],
        now_epoch: i64,
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
        let agents = agents_fact(&self.roster, by_agent, now_epoch, self.knobs.interval_secs);
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
            &mut carry.motion,
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
    fn publish(&self, published: &Published<'_>, motion: &mut MotionState) {
        let Some(session_id) = transport::observe_session_id(self.server, self.session) else {
            return;
        };
        if let Some(opened) =
            transport::observe_session_option(self.server, self.session, theme::MENU_OPEN_OPTION)
            && menu_open_expired(&opened, Timestamp::now().epoch(), self.knobs.interval_secs)
        {
            let _ = transport::clear_option(
                self.server,
                OptionScope::Session,
                &session_id,
                theme::MENU_OPEN_OPTION,
            );
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
        self.publish_fleet(look, motion);
        self.publish_windows(published, motion);
    }

    /// The fleet strip: every ae session on THIS server, as each one's own
    /// watchdog described itself.
    fn publish_fleet(&self, look: &Look, motion: &mut MotionState) {
        let Some(sessions) = transport::observe_fleet_sessions(self.server) else {
            return;
        };
        let orchestrator_id = orchestrator_id_for(&sessions, self.session);
        let mut next = motion.clone();
        next.replace_fleet(&sessions, self.session);
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
                    .clone_from(&motion.published_orchestrator_strip);
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
                    .clone_from(&motion.published_orchestrator_id);
            }
        }
        if writes.is_empty() || transport::publish_options(self.server, &writes) {
            *motion = next;
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
        let (event, looked_past) = latest_relevant_event(query.events, query.agent, self.session)?;
        let kind = quiet_reason(event, query.agent, looked_past)?;
        if kind == QuietKind::Done {
            return Some(kind);
        }
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
                let text = if idle_age.is_some() {
                    idle_nudge_text(self.goal.as_deref(), self.meta_dir)
                } else {
                    nudge_text(self.goal.as_deref(), self.meta_dir)
                };
                let summary = if idle_age.is_some() {
                    format!("{display}, harness waiting at input")
                } else {
                    format!("{display}, no recent ae activity")
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

/// The watchdog-owned agent fact in recorded roster order.
///
/// Present panes carry the verdict this cycle already computed. A roster seat
/// with no pane remains visible as `dead` with an empty navigation hint. Any
/// unrepresentable recorded identity rejects the whole value rather than
/// publishing a partial roster the picker could mistake for complete.
fn agents_fact(
    roster: &[RosterEntry],
    by_slot: &[(String, String, Verdict)],
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
    let mut value = format!("v1;{now_epoch};{interval_secs}");
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
        let found = by_slot.iter().find(|(slot, _, _)| *slot == entry.slot);
        let (state, pane) = found.map_or(("dead", ""), |(_, pane, verdict)| {
            (verdict.reason(), pane.as_str())
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
        if value.len() > tmux::PICKER_AGENTS_MAX_BYTES {
            return None;
        }
    }
    Some(value)
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
    crate::meta::sole_value(meta_bytes, "meta_agent") == Some(b"true".as_slice())
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
        ACTOR, Carry, Continuation, Cycle, Effect, HarnessObservation, Journal, Knobs,
        MissingState, MotionState, MotionVerdict, Observation, OverviewReading, PaneState,
        PendingAdvisory, QuietCycle, QuietQuery, QuotaAction, QuotaCarry, QuotaDelivery,
        QuotaLevel, QuotaRecipient, Rebind, SendHelper, UNKNOWN_ALERT_CYCLES, Verdict, account,
        adopt_server, age_secs, agents_fact, bar_glyph, classify_quota, continuation, entry_mut,
        idle_nudge_seconds, idle_nudge_text, is_meta_agent, last_actor_event_age,
        last_done_event_at, last_working_declaration_at, motion_cadence, motion_failure,
        motion_observation_due, motion_publish_failure, motion_ticker_enabled, nudge_text,
        observed_option, quota_delivery, quota_observation_due, quota_recipients, quota_seconds,
        read_events, rebind, record_nudge, restore_idle, session_name, slot_mark, stale_display,
        sweep_effects, sweep_seconds, system_time_from_epoch, throttle_quota_line,
        window_agents_line,
    };
    use super::{Look, Mark, PaneMark, session_mark};
    use crate::events::Event;
    use crate::inventory::ServerId;
    use crate::meta::{Meta, RecordedConfigHome, RecordedConfigHomeBase, RosterEntry, Selector};
    use crate::procs::Descendancy;
    use crate::tmux::StopProbe;
    use crate::watchdog::{
        QuietKind, SweepAlert, SweepEffect, SweepObservation, SweepVerdict, WedgeDetail,
        declaration_key,
    };
    use std::io::ErrorKind;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
            is_throttled: false,
            throttle_quota: None,
            quiet: None,
            descendancy: Descendancy::Present,
            last_actor_event_age_secs: 0,
            sweep: None,
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
        }
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
                QuotaAction::Dropped { .. } => None,
            })
            .collect()
    }

    #[test]
    fn quota_classification_hysteresis_has_exact_boundaries() {
        let mut state = None;
        let mut seen = Vec::new();
        for used in [79.0, 80.0, 95.0, 94.0, 89.0, 74.0] {
            let next = classify_quota(used, state);
            seen.push(next);
            state = Some(next);
        }
        assert_eq!(
            seen,
            [
                QuotaLevel::Headroom,
                QuotaLevel::Low,
                QuotaLevel::Critical,
                QuotaLevel::Critical,
                QuotaLevel::Low,
                QuotaLevel::Headroom,
            ]
        );
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
        assert_eq!(carry.tracked[0].level, QuotaLevel::Headroom);
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

    #[test]
    fn throttle_quota_respects_cached_row_reset_boundary() {
        let rollout = "018f1f70-7b2c-7000-8000-000000000001";
        let entry = RosterEntry {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: None,
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
    fn quota_keys_separate_rollouts_and_scoped_qualifiers() {
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
        assert_eq!(carry.tracked.len(), 3);
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
            observation.advisory_line(sample.group, sample.row, "low", Path::new("/m/demo")),
            "quota: codex · cx · demo:lead weekly_scoped Fable 5h 80% (low, observed 2m ago), resets in 1h00m — prefer another client for new spawns; table: /m/demo/quota"
        );

        let reset = quota_for("74", 9_902);
        assert_eq!(
            reset.advisory_line(
                &reset.groups[0],
                &reset.groups[0].rows[0],
                "back to headroom",
                Path::new("/m/demo")
            ),
            "quota: codex · cx · demo:lead codex pro 5h 74% (headroom, observed 1m ago), resets in 1h00m — back to headroom; table: /m/demo/quota"
        );

        let mut precise = observation.clone();
        precise.groups[0].rows[0].used_percent = Some(format!("80.{}", "1".repeat(300)));
        let line = precise.advisory_line(
            &precise.groups[0],
            &precise.groups[0].rows[0],
            "low",
            Path::new("/m"),
        );
        assert!(line.contains(" 80.1% (low,"), "{line}");
        assert!(line.len() < 240, "numeric display stayed bounded: {line}");

        let mut hostile = observation.clone();
        hostile.groups[0].rows[0].bucket = format!("bad\u{1b}[2J\r\n{}", "x".repeat(300));
        hostile.groups[0].rows[0].qualifier = Some("q\u{7f}\n".repeat(100));
        let sample = &super::quota_samples(&hostile)[0];
        let line = hostile.advisory_line(sample.group, sample.row, "low", Path::new("/m"));
        assert!(!line.contains('\u{1b}'));
        assert!(!line.contains('\r'));
        assert!(!line.contains('\n'));
        assert!(line.len() < 240, "hostile labels stayed bounded: {line}");
    }

    #[test]
    fn throttle_quota_uses_worst_exact_recorded_row_and_first_cycle_only() {
        let rollout = "018f1f70-7b2c-7000-8000-000000000001";
        let mut entry = RosterEntry {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: Some("changed-profile".to_owned()),
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
        observed.is_throttled = true;
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
            .is_none()
        );
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
                "#[fg=#537187]●#[default] working",
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
        assert!(motion_ticker_enabled(Look::DEFAULT));
        assert!(!motion_ticker_enabled(Look {
            motion: false,
            ..Look::DEFAULT
        }));
        assert!(!motion_ticker_enabled(Look {
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
        let session = |rank: &str| crate::tmux::FleetSession {
            name: "current".to_owned(),
            id: "$7".to_owned(),
            rank: rank.to_owned(),
        };
        let mut state = MotionState::default();
        state.replace_observation(vec![motion("%1", "lead")], &[session("1")], "current");

        let first = state.step(&Look::DEFAULT);
        assert_eq!(first.len(), 1, "the first static strip is new");
        assert!(
            state.step(&Look::DEFAULT).is_empty(),
            "unchanged static strip"
        );

        state.replace_fleet(&[session("4")], "current");
        assert_eq!(state.step(&Look::DEFAULT).len(), 1, "changed rank");
        assert!(state.step(&Look::DEFAULT).is_empty(), "unchanged attention");

        state.replace_fleet(&[session("2")], "current");
        let first_frame =
            crate::tmux::set_options_args(&ServerId::Ambient, &state.step(&Look::DEFAULT));
        let next_frame =
            crate::tmux::set_options_args(&ServerId::Ambient, &state.step(&Look::DEFAULT));
        assert!(first_frame.iter().any(|word| word.contains('●')));
        assert!(next_frame.iter().any(|word| word.contains('●')));
        assert_ne!(first_frame, next_frame, "pulse colour changes each tick");
    }

    #[test]
    fn current_orchestrator_publishes_its_segment_and_keeps_it_on_ticker() {
        let session = |name: &str, id: &str, rank: &str| crate::tmux::FleetSession {
            name: name.to_owned(),
            id: id.to_owned(),
            rank: rank.to_owned(),
        };
        let mut state = MotionState::default();
        state.replace_observation(
            vec![motion("%1", "lead")],
            &[
                session("worker", "$4", "1"),
                session("orchestrator", "$7", "0"),
            ],
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

    #[test]
    fn verdict_cycle_current_orchestrator_publishes_segment_and_excludes_fleet_row() {
        let sessions = [
            crate::tmux::FleetSession {
                name: "worker".to_owned(),
                id: "$4".to_owned(),
                rank: "1".to_owned(),
            },
            crate::tmux::FleetSession {
                name: "orchestrator".to_owned(),
                id: "$7".to_owned(),
                rank: "0".to_owned(),
            },
        ];
        let mut state = MotionState::default();
        state.replace_fleet(&sessions, "orchestrator");
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
            crate::tmux::FleetSession {
                name: "worker".to_owned(),
                id: "$4".to_owned(),
                rank: "2".to_owned(),
            },
            crate::tmux::FleetSession {
                name: "orchestrator".to_owned(),
                id: "$7".to_owned(),
                rank: "1".to_owned(),
            },
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
    fn orchestrator_strip_composes_before_version_only_when_present() {
        let session = |name: &str, id: &str, rank: &str| crate::tmux::FleetSession {
            name: name.to_owned(),
            id: id.to_owned(),
            rank: rank.to_owned(),
        };
        let mut with = MotionState::default();
        with.replace_fleet(
            &[
                session("worker", "$4", "2"),
                session("orchestrator", "$7", "3"),
            ],
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
        assert!(args.iter().any(|word| word.contains("orchestrator")));

        let mut without = MotionState::default();
        without.replace_fleet(&[session("worker", "$4", "2")], "worker");
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
        all.is_throttled = true;
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
        assert_eq!(
            account(&PaneState::default(), &all, &Knobs::default()).verdict,
            Verdict::Throttled
        );
        all.is_throttled = false;
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
        // There is no watchdog-emitted clear, and no second alert.
        let second = account(&first.next, &observed, &Knobs::default());
        assert_eq!(second.verdict, Verdict::Dead);
        assert!(emitted(&second.effects).is_empty(), "alerted twice");
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
    fn throttling_says_so_once_alerts_at_the_bound_and_clears_on_recovery() {
        let knobs = Knobs::default();
        let mut observed = seen();
        observed.is_throttled = true;
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
            meta_agent: false,
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
            meta_agent: false,
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
        let (latest, looked_past) =
            crate::watchdog::latest_relevant_event(&events, "codex:agent", "demo")
                .expect("the later memo is relevant");
        assert_eq!(latest.action, "memo");
        assert_eq!(
            crate::watchdog::quiet_reason(latest, "codex:agent", looked_past),
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
        assert!(
            plain.ends_with(
                "/home/x/.ae/sessions/demo/state <waiting-user|blocked|done> \"<reason>\""
            )
        );
        let goaled = nudge_text(Some("ship P4.1"), meta);
        assert!(goaled.starts_with("Session goal: ship P4.1. Status check:"));
        let idle = idle_nudge_text(Some("ship P4.1"), meta);
        assert!(idle.contains("you look idle: declare state or continue"));
        assert!(
            idle.ends_with(
                "/home/x/.ae/sessions/demo/state <waiting-user|blocked|done> \"<reason>\""
            )
        );
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
        assert_eq!(last_actor_event_age(&events, "opus5:builder", now), 60);
        assert_eq!(
            last_actor_event_age(&events, "nobody:here", now),
            super::NO_EVENT_AGE,
            "no event at all is the sentinel, not an age"
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

    /// Ten verdicts, six marks: the mapping is the whole vocabulary the status
    /// bar, the pane borders and the picker share.
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
                Verdict::Quiet(QuietKind::Blocked),
                Mark::NeedsYou,
                "blocked",
            ),
            (Verdict::Throttled, Mark::NeedsYou, "throttled"),
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
        assert_eq!(Mark::Done.glyph(true), "✓");
        assert_eq!(Mark::Stale.glyph(true), "◌");
        assert_eq!(Mark::Idle.glyph(true), "·");
        assert_eq!(Mark::NeedsYou.glyph(false), "!");
        assert_eq!(Mark::Working.glyph(false), "*");
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
            (
                "worker.0".to_owned(),
                "%8".to_owned(),
                Verdict::Quiet(QuietKind::Done),
            ),
            ("main".to_owned(), "%3".to_owned(), Verdict::Active),
        ];
        assert_eq!(
            agents_fact(&roster, &observed, 2_000, 60),
            Some(
                "v1;2000;60;lead:fable5:working:%3;builder:gpt56sol:done:%8;tests:gpt56luna:dead:"
                    .to_owned()
            )
        );
        assert_eq!(agents_fact(&roster, &observed, 2_000, 0), None);
        assert_eq!(agents_fact(&roster, &observed, 2_000, 3_601), None);
        assert_eq!(
            crate::theme::AGENTS_OPTION,
            "@ae_agents",
            "one session-scoped fact, never per-agent options"
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
    fn the_orchestrator_flag_is_read_strictly_and_fails_closed_on_a_doubled_record() {
        // A session that gets the sweep branch stops being escalated for
        // silence, so the flag is EXACTLY ONE record saying EXACTLY `true`.
        let cases: [(&str, bool); 9] = [
            ("session=x\nmeta_agent=true\n", true),
            // The two that a first-value read got WRONG, in the dangerous
            // direction: it answered "orchestrator" and switched OFF stale
            // escalation on the strength of a record whose meaning is in doubt.
            ("meta_agent=true\nmeta_agent=false\n", false),
            ("meta_agent=true\nmeta_agent=true\n", false),
            ("meta_agent=false\nmeta_agent=true\n", false),
            ("meta_agent=false\n", false),
            ("session=x\n", false),
            ("meta_agent=True\n", false),
            ("meta_agent=1\n", false),
            ("meta_agent=yes\n", false),
        ];
        for (meta, want) in cases {
            assert_eq!(
                is_meta_agent(meta.as_bytes()),
                want,
                "{meta:?} must read orchestrator={want}"
            );
        }
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
}
