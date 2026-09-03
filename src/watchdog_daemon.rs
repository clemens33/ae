//! The watchdog daemon — the loop that observes a session's panes each cycle,
//! asks [`crate::watchdog`] what it is looking at, and applies the answers.
//!
//! # The clean cut
//!
//! [`crate::watchdog`] DECIDES and holds no state; this module OBSERVES,
//! ACCOUNTS and delivers. Bash keeps what it is best at: starting and stopping
//! this process, and publishing the tmux options from the status lines printed
//! here. Rust interprets, bash publishes.
//!
//! # Two halves, and the seam is deliberate
//!
//! [`account`] is PURE — prior pane state plus this cycle's observation in,
//! next state plus a list of [`Effect`]s out. Every branch of the frozen loop
//! (ae:16490-16947) is therefore unit-testable without a tmux server, a
//! process table or a clock. The loop's own job is reduced to gathering the
//! observation and applying the effects, which is the part a test cannot reach
//! anyway.
//!
//! # The meta-agent sweep
//!
//! The frozen watchdog's orchestrator branch (ae:16738-16897) IS ported: sweep
//! cadence, heartbeat wedge detection and its own two alerts. It applies to the
//! orchestrator MAIN agent alone (SC-935) and it replaces every ordinary branch for
//! that pane — an orchestrator idling between sweeps is a service at rest, not a
//! stale agent. The decisions are [`crate::watchdog`]'s
//! ([`crate::watchdog::sweep_step`] / [`crate::watchdog::record_sweep`]); this
//! module reads the heartbeat, delivers the prompt and books what happened.
//!
//! # What else stays in bash this slice
//!
//! The Telegram supervise and the pending tool-session-id recovery. They are not
//! ported here and this module must not be read as replacing them.

use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::events::Event;
use crate::meta::{Meta, RosterEntry, ServerSelector};
use crate::procs::{self, Descendancy};
use crate::state;
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

/// The event actor every watchdog emission carries (ae:16515 and its siblings).
const ACTOR: &str = "watchdog";

/// The panes that are not agents: unstamped, tmux's own null, this daemon, the
/// events pane, and the two legacy names a pre-rename session can still carry
/// (ae:16481-16483).
const NON_AGENT_PANES: [&str; 5] = ["(null)", "_watchdog", "_events", "_shepherd", "_loop"];

/// The bound on consecutive unusable process snapshots before the daemon says
/// so. Small on purpose: this alert exists to make a watchdog that has quietly
/// stopped watching VISIBLE, and a bound so high it never fires is the failure
/// it is meant to prevent.
const UNKNOWN_ALERT_CYCLES: u32 = 5;

/// The tunables, all of them (ae:16331-16373). Defaults are the frozen ones.
///
/// They arrive as CLI ARGUMENTS rather than environment: this crate's clippy
/// policy disallows `std::env::var` outright, so bash reads `AE_WATCHDOG_*`
/// (with its `AE_LOOP_*` fallback) and passes what it read. The defaults live
/// here so a bash side that passes nothing still runs the frozen cadence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Knobs {
    /// Seconds slept at the end of every cycle.
    pub interval_secs: u64,
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
    /// The orchestrator sweep cadence, retry and bound (ae:16435-16448).
    pub sweep: SweepKnobs,
    /// Seconds between best-effort Telegram bridge revives (ae:14297-14299).
    /// `0` disables supervision outright.
    pub tg_supervise_secs: u64,
}

impl Default for Knobs {
    fn default() -> Self {
        Self {
            interval_secs: 60,
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

/// What one pane carries from cycle to cycle (bash's per-pane associative
/// arrays, ae:16409-16430, gathered into one value).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneState {
    /// Dead is LATCHED: once alerted, the pane is skipped every later cycle and
    /// there is no watchdog-emitted clear (ae:16490-16495).
    pub dead_latched: bool,
    /// The previous cycle's filtered pane hash; `None` before the first.
    pub prev_hash: Option<u64>,
    /// When the hash last changed, in epoch seconds; `None` if it never has.
    pub last_hash_change: Option<i64>,
    /// DELIVERIES, never attempts (ae:16874-16899).
    pub nudge_count: u32,
    /// Consecutive throttled cycles.
    pub throttle_streak: u32,
    /// Consecutive nudges that did not land.
    pub undelivered_streak: u32,
    /// Consecutive cycles whose process snapshot was unusable.
    pub unknown_streak: u32,
    /// The persistent-unknown alert is raised once per streak, not per cycle.
    pub unknown_alerted: bool,
    /// The armed quiet baseline: the declaration's full-tuple key and the hash
    /// the pane settled on.
    pub quiet_base: Option<(String, u64)>,
    /// The orchestrator sweep branch's carry. Untouched for every pane that is not
    /// the orchestrator main (SC-935).
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
    /// [`classify_dead`]'s answer.
    pub is_dead: bool,
    /// [`shows_throttle`]'s answer.
    pub is_throttled: bool,
    /// The RESOLVED quiet suppression: `Done` always, `WaitingUser`/`Blocked`
    /// only while their baseline holds. `None` when the agent is not quiet —
    /// including a declaration that YIELDED or failed to settle, which bash
    /// falls through on (ae:16781).
    pub quiet: Option<QuietKind>,
    /// Whether a process named the agent binary runs under the pane.
    pub descendancy: Descendancy,
    /// Age of the newest event this agent is the ACTOR of (ae:15520-15537).
    pub last_actor_event_age_secs: u64,
    /// The orchestrator sweep reading, `Some` ONLY for the orchestrator main agent with
    /// the cadence enabled (SC-935 + SC-1405b). Every other pane carries `None`
    /// and is judged by the ordinary branches.
    pub sweep: Option<SweepObservation>,
}

/// The roster glyph a pane earned this cycle — derived only from branches that
/// were actually judged (ae:16985-17032).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The agent's process is gone, or its pane is.
    Dead,
    /// A declared quiet state.
    Quiet(QuietKind),
    /// Upstream is rate-limiting this agent.
    Throttled,
    /// Silent past the window, with nothing recent anywhere.
    Stale,
    /// Moving, recently moved, or recently active in the log.
    Active,
    /// The orchestrator main, judged by its own cadence rather than by silence.
    Meta(SweepVerdict),
}

impl Verdict {
    /// The glyph the frozen roster publishes for this verdict (ae:16086-16218).
    #[must_use]
    pub const fn glyph(self) -> &'static str {
        match self {
            Self::Dead => "✖",
            Self::Quiet(QuietKind::Done) => "✔",
            Self::Quiet(QuietKind::WaitingUser) => "⏳",
            Self::Quiet(QuietKind::Blocked) => "⛔",
            Self::Throttled => "⚡",
            Self::Stale => "◌",
            Self::Active => "●",
            Self::Meta(verdict) => verdict.glyph(),
        }
    }
}

/// Something the loop must DO. The accounting decides these; only the loop
/// performs them, which is what keeps the branch logic testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Append one event for this agent. `action` is the frozen action name and
    /// `summary` its frozen text.
    Emit {
        /// `alert` / `throttled` / `throttle-cleared`.
        action: &'static str,
        /// The summary, quoted from the frozen loop.
        summary: String,
    },
    /// Deliver one nudge through the session's own send helper. The helper
    /// emits the `nudge` event itself, exactly as it does for bash.
    Nudge,
    /// A line for the human, which bash publishes with `display-message`
    /// (ae:16516 and siblings) — the watchdog interprets, bash shows.
    Notify(String),
    /// Deliver one SWEEP prompt to the orchestrator (ae:16833-16841). A separate
    /// effect from [`Effect::Nudge`]: different text, a different frozen event
    /// summary, and a different post-delivery accounting.
    SweepNudge,
    /// Reconcile the durable event log against a wedge alert this daemon does
    /// not remember raising (ae:16768-16774) — the post-restart clear.
    ReconcileWedge,
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
}

/// `idle <n>m`, or `no recent events` when the age is absurd (ae:16862-16867).
///
/// The absurd case is the sentinel: an agent with no event at all reports
/// 999999 seconds, and "idle 16666m" is a worse thing to publish than the truth.
#[must_use]
pub fn stale_display(event_age_secs: u64) -> String {
    let minutes = event_age_secs / 60;
    if minutes > 9999 {
        "no recent events".to_owned()
    } else {
        format!("idle {minutes}m")
    }
}

/// The nudge, exactly as ae:16884-16893 composes it: the session goal when the
/// meta carries one, then the status sentence, then the path to this session's
/// own `state` helper.
#[must_use]
pub fn nudge_text(goal: Option<&str>, meta_dir: &Path) -> String {
    let prefix = goal.map_or_else(String::new, |goal| format!("Session goal: {goal}. "));
    format!(
        "{prefix}Status check: if you have more work, continue. Otherwise declare your state so \
         I stop nudging: {}/state <waiting-user|blocked|done> \"<reason>\"",
        meta_dir.display()
    )
}

/// Count consecutive unusable process snapshots, and say so once.
///
/// This runs on EVERY observed cycle, including the ones [`account`] returns
/// early from: the point is to notice a probe that never works, and the dead
/// branch — where an unusable probe now means "not dead" — is exactly where it
/// would otherwise stay hidden.
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

/// The throttled branch (ae:16787-16818).
///
/// The streak's FIRST cycle says so once, and the cycle that reaches the alert
/// bound says so once more; the hash bookkeeping is updated even though no
/// nudge follows, so that a later recovery reads as activity rather than as a
/// stale first-time difference. The delivered count resets: a throttled agent
/// has not ignored anything.
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
            summary: "upstream throttling detected — pausing nudges".to_owned(),
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
    next.nudge_count = 0;
}

/// What a stale pane earns: a nudge, the one max-nudges alert, or nothing
/// (ae:16858-16922).
///
/// Three states, not two. Past the undelivered bound the pane earns NOTHING:
/// an unreachable pane must stop costing cycles, and each send can hold the
/// loop for its whole defer window. The alert fires on the cycle the delivered
/// count EQUALS the bound and the count then advances past it, so it is raised
/// exactly once however long the pane stays stale.
fn book_stale(
    prior: &PaneState,
    next: &mut PaneState,
    effects: &mut Vec<Effect>,
    seen: &Observation,
    knobs: &Knobs,
) {
    if prior.undelivered_streak >= knobs.undelivered_max {
        return;
    }
    if prior.nudge_count < knobs.max_nudges {
        effects.push(Effect::Nudge);
    } else if prior.nudge_count == knobs.max_nudges {
        let display = stale_display(seen.last_actor_event_age_secs);
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

/// The orchestrator main's sweep branch (ae:16738-16897), or `None` when this pane
/// is not it.
///
/// A `Some` verdict means the cadence DECIDED this pane and every branch after
/// it is skipped: no quiet suppression, no throttle streak, no stale nudge.
/// "Idle between sweeps" is what a monitor at rest looks like, and nudging it
/// to declare a state would be nudging the thing that watches the nudging. The
/// dead check runs BEFORE this and the missing-pane sweep runs after the loop,
/// so a crashed or vanished orchestrator is still caught.
///
/// Two separate `None`s, and they mean the same thing to the caller — run the
/// ordinary branches — for different reasons. `seen.sweep` is `None` for every
/// pane that is not the orchestrator main (SC-935, decided at the observation).
/// [`sweep_step`] returns `None` for SC-1405b's disabled cadence. Neither is an
/// empty accounting, which would suppress the stale watchdog as well.
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

/// Account for one pane in one cycle — the frozen loop's branch order, and the
/// only place any of it is decided.
///
/// The order is bash's, and it is load-bearing rather than incidental:
///
/// 1. a LATCHED dead pane is skipped entirely (ae:16490-16495);
/// 2. the dead check, which latches and alerts once (ae:16503-16519);
/// 3. the orchestrator main's sweep cadence, which REPLACES every branch below it
///    for that one pane (ae:16738-16897);
/// 4. the throttle streak CLEAR, which runs before every later branch so a
///    quiet agent's throttle still clears (ae:16713-16723);
/// 5. quiet suppression, which resets the delivered count and counts active
///    (ae:16739-16790);
/// 6. throttle, reacted to BEFORE active/recent so the alert cadence starts
///    immediately rather than after the whole stale window (ae:16787-16818);
/// 7. active / recently visible / recently alive (ae:16821-16856);
/// 8. stale: nudge, or the one max-nudges alert (ae:16858-16922).
///
/// The persistent-Unknown accounting has no bash counterpart — it is the
/// visibility this port's [`Descendancy::Unknown`] ruling requires, since an
/// unusable snapshot now means "not dead" and a permanently failing probe would
/// otherwise be a watchdog that had silently stopped watching.
#[must_use]
pub fn account(prior: &PaneState, seen: &Observation, knobs: &Knobs) -> Accounting {
    let mut next = prior.clone();
    let mut effects = Vec::new();
    book_unknown(&mut next, &mut effects, seen.descendancy);

    // 1. Already dead: no second alert, no further judgement.
    if prior.dead_latched {
        return Accounting {
            next,
            effects,
            verdict: Verdict::Dead,
        };
    }

    // 2. Dead — latched, alerted once, and there is no watchdog-emitted clear.
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
        };
    }

    // 3. The orchestrator main's own cadence, which returns rather than falling
    //    through — see `book_sweep`.
    if let Some(verdict) = book_sweep(prior, &mut next, &mut effects, seen, knobs) {
        return Accounting {
            next,
            effects,
            verdict,
        };
    }

    // 4. The throttle streak clears on ANY non-throttled cycle, before the
    //    branches below can return.
    if !seen.is_throttled && prior.throttle_streak > 0 {
        effects.push(Effect::Emit {
            action: "throttle-cleared",
            summary: format!("throttling cleared after {} cycles", prior.throttle_streak),
        });
        next.throttle_streak = 0;
    }

    // 5. A held quiet state is "leave me alone": no nudge, and the delivered
    //    count resets so a later stale run starts fresh.
    if let Some(kind) = seen.quiet {
        next.nudge_count = 0;
        return Accounting {
            next,
            effects,
            verdict: Verdict::Quiet(kind),
        };
    }

    // 6. Throttled: a nudge cannot fix upstream, and the hash bookkeeping is
    //    updated anyway so a later recovery reads as activity rather than as a
    //    stale first-time difference.
    if seen.is_throttled {
        book_throttle(&mut next, &mut effects, seen, knobs);
        return Accounting {
            next,
            effects,
            verdict: Verdict::Throttled,
        };
    }

    // 7. Active — the pane moved since the last cycle. A moving pane is also
    //    positive evidence that whatever blocked delivery is gone, so the
    //    undelivered bound re-arms here rather than only on a delivery.
    let hash_unchanged = prior.prev_hash == Some(seen.hash);
    if !hash_unchanged {
        next.prev_hash = Some(seen.hash);
        next.last_hash_change = Some(seen.now_epoch);
        next.nudge_count = 0;
        next.undelivered_streak = 0;
        return Accounting {
            next,
            effects,
            verdict: Verdict::Active,
        };
    }

    // 8. Stale, or one of the two "recent" escapes. One definition of the
    //    composite, in `watchdog`, driven from here.
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
        };
    }

    book_stale(prior, &mut next, &mut effects, seen, knobs);
    Accounting {
        next,
        effects,
        verdict: Verdict::Stale,
    }
}

/// Render the sweep layer's decisions as this loop's effects.
///
/// The alert TRANSITION carries its own frozen action, summary and
/// display-message text ([`crate::watchdog::SweepAlert`]), so this is a
/// rendering step and never a policy one — there is nothing here to disagree
/// with the decision about. Both producers of sweep effects go through it, so
/// the two cannot drift.
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

/// Book a nudge attempt's outcome (ae:16894-16913).
///
/// The counter counts DELIVERIES: a nudge the send helper refused (a dead
/// shell) or abandoned (a busy target, or the human mid-type) is an attempt,
/// not a nudge, and bumping the delivered count on one made the watchdog report
/// asks that never landed. The undelivered streak is separate for the same
/// reason, and the attempt that REACHES the bound raises its own distinct alert
/// — "delivered and ignored" and "could not be delivered" are different
/// problems with different fixes.
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
///
/// A timestamp in the FUTURE — clock skew, a container written by another host
/// — must not underflow into an age of billions of seconds, which would read as
/// "nothing has happened in this agent's whole history" and nudge a busy agent.
#[must_use]
pub fn age_secs(now_epoch: i64, at_epoch: i64) -> u64 {
    u64::try_from(now_epoch.saturating_sub(at_epoch)).unwrap_or(0)
}

/// The age of the newest event this agent is the ACTOR of — bash's
/// `_last_event_age` (ae:15520-15537).
///
/// Actor only, never target, and by APPEND POSITION (bash takes `tail -1` of
/// the matching lines, not the maximum timestamp). `NO_EVENT_AGE` when the
/// agent has no event at all, which is the sentinel the stale display turns
/// into "no recent events" rather than an absurd minute count.
#[must_use]
pub fn last_actor_event_age(events: &[Event], agent: &str, now_epoch: i64) -> u64 {
    events
        .iter()
        .rev()
        .find(|event| event.actor == agent)
        .map_or(NO_EVENT_AGE, |event| age_secs(now_epoch, event.ts.epoch()))
}

/// What bash prints when an agent has no event at all (ae:15523).
pub const NO_EVENT_AGE: u64 = 999_999;

// ---------------------------------------------------------------------------
// The loop — observation and effects. Everything above this line is pure.
// ---------------------------------------------------------------------------

/// The generated helper a nudge is delivered through. FIXED, and the name is a
/// literal in this file.
const HELPER_NAME: &str = "send";

/// The orchestrator's heartbeat, at the FIXED name `<meta-dir>/meta-agent-state.json`
/// (ae:16747).
///
/// This is the file `contrib/aemonitor` rewrites atomically on each real sweep.
/// The two MUST agree: point the monitor's `--state` somewhere else and the
/// wedge check watches a file nobody writes, which false-alarms forever.
const HEARTBEAT_NAME: &str = "meta-agent-state.json";

/// The sweep prompt, exactly as ae:16836-16840 composes it.
///
/// A fixed string with nothing interpolated: unlike the stale nudge it carries
/// no goal and no path, so there is no value in it that could come from meta,
/// config or a pane.
const SWEEP_PROMPT: &str = "Run your sweep now: ae list --json, diff your state file, and report \
                            ONLY new/changed attention to Clemens via say (stay silent if nothing \
                            changed). Stay in 'working'.";

/// The heartbeat's modification time, or `None` when there is nothing this
/// watchdog is willing to trust.
///
/// # `lstat`, never `stat` — the safety pin
///
/// `symlink_metadata` does NOT follow symlinks, and a symlink's own metadata
/// reports `is_file() == false`, so a link is refused here whatever it points
/// at. The frozen bash uses `[[ -f ]]`, which DOES follow: anything able to
/// write in the session directory could aim the heartbeat at a file some other
/// process touches often, and a wedged orchestrator would be silenced by a link.
/// This port refuses that, and the refusal is the reason the reading is a
/// tri-state rather than a number — see [`crate::watchdog::Heartbeat`].
///
/// Every other failure — absent, a directory, a fifo, an unreadable parent, a
/// filesystem with no mtime — is the same `None`, and `None` is never health.
fn heartbeat_mtime(meta_dir: &Path) -> Option<SystemTime> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: lstat of the orchestrator's heartbeat, so a symlinked state file is \
                  refused rather than trusted — see clippy.toml"
    )]
    let lstat = std::fs::symlink_metadata(meta_dir.join(HEARTBEAT_NAME));
    let at = lstat.ok()?;
    if !at.is_file() {
        return None;
    }
    at.modified().ok()
}

/// The session's own send helper, at the FIXED path `<meta-dir>/send`.
///
/// A newtype with ONE constructor, because this is the security boundary the
/// slice was cut around: the daemon must never take the program it EXECUTES
/// from meta, config, `/tmp`, or pane content, where an attacker-chosen value
/// could be planted. Making the path unforgeable by construction turns "every
/// reviewer must check every call site" into "there is one constructor, and it
/// joins a literal onto the session directory the core was handed".
///
/// The helper is also what carries ae's input-busy modelling, staged-paste
/// detection and defer behaviour, and it emits the `nudge` event itself — so
/// routing around it would lose the protections AND the honest event.
struct SendHelper(std::path::PathBuf);

impl SendHelper {
    /// `<meta-dir>/send`, and nothing else.
    fn for_session(meta_dir: &Path) -> Self {
        Self(meta_dir.join(HELPER_NAME))
    }

    /// The path to spawn. Reading is harmless; CONSTRUCTION is what is sealed.
    fn path(&self) -> &Path {
        &self.0
    }
}

/// The glyph for a verdict nobody reached — an unstamped slot, or a roster entry
/// whose pane this cycle never judged.
///
/// Neutral on purpose (ae:16210-16212): borrowing `●` for "we did not decide"
/// would manufacture a liveness claim out of a gap in our own logic.
const NEUTRAL_GLYPH: &str = "·";

/// Run the watchdog for one session until its session or its state goes away.
///
/// # Fail closed on the server
///
/// The RECORDED selector is resolved ONCE, at startup, and a record that does
/// not name exactly one server is fatal. There is no ambient fallback: an
/// ambient server is a different server, and this daemon sends keys into panes.
/// The frozen bash never had to make this choice because it ran inside the
/// session it watched; a core entry does.
///
/// # Self-termination
///
/// Each cycle re-reads the session's `meta` and asks the recorded server whether
/// the session is still there. Either being gone ends the daemon cleanly
/// (ae:16433-16441) — a watchdog for a session that no longer exists is a
/// process nobody will ever stop.
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
) -> crate::Result<u8> {
    let Ok(bytes) = crate::meta::read_bytes(meta_dir) else {
        writeln!(
            err,
            "ae: watchdog: no session state at {}",
            meta_dir.display()
        )?;
        return Ok(1);
    };
    let meta = Meta::parse(&String::from_utf8_lossy(&bytes));
    // The INITIAL resolution, kept as the fast refuse. `rebind` re-asks it
    // every cycle from that cycle's own meta — see its doc for why.
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

    // ── The pane's own duties, which were the bash wrapper's until slice A.3 ──
    // Order is the frozen one (ae:14346-14372): the pidfile FIRST, because the
    // start path's registration wait is what releases the start lock; then the
    // bars, so a pane that is up says so before its first cycle; then the banner.
    let pidfile = match crate::watchdog_glue::PidFile::publish(meta_dir) {
        Ok(published) => Some(published),
        Err(why) => {
            // Reported, not fatal. A watchdog that refuses to watch a live
            // session because it could not write its own bookkeeping has turned
            // a cosmetic failure into an outage; `watchdog status` degrades to
            // "not running" instead, which is exactly what a missing pidfile has
            // always meant.
            writeln!(err, "ae: watchdog: pidfile not published: {why}")?;
            None
        }
    };
    // The pre-rename reap, which was `_watchdog_start`'s first act (ae:13302-13306).
    // It happens HERE now, because the pane IS the daemon: a `doctor --refresh`
    // rewrites helpers but does not restart a running watchdog, so a legacy
    // `_shepherd` / `_loop` process can outlive the rename and we must never run
    // two watchdogs over one session — nor leave a stray legacy pane this loop
    // would then mis-classify as an agent.
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
        crate::watchdog_glue::recorded_ae_path(&bytes),
        recorded_server_value(&meta),
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
    // OWNERSHIP-CHECKED, and only on a return this daemon chose. A stop/start in
    // quick succession can leave this cleanup running after the replacement is
    // already registered, and removing its pidfile would be the dying process
    // vandalising its successor (ae:13996-14000).
    if let Some(pidfile) = &pidfile {
        let _ = pidfile.release();
    }
    code
}

/// Publish the two things a pane that is UP says before its first cycle: the
/// starting health segment and the branch pair (ae:14361-14362).
///
/// Separate from the loop's own publication because it must land BEFORE the
/// first observation: a watchdog that publishes nothing until its first cycle
/// completes leaves the bar reading whatever the last one left there.
fn announce_start(server: &crate::inventory::ServerId, session: &str, work_dir: Option<&str>) {
    let Some(session_id) = transport::observe_session_id(server, session) else {
        return;
    };
    let _ = transport::publish_option(
        server,
        OptionScope::Session,
        &session_id,
        tmux::WATCHDOG_STATUS_OPTION,
        "[watch ◌ starting]",
    );
    crate::watchdog_glue::publish_branch(
        server,
        &session_id,
        crate::watchdog_glue::branch_reading(work_dir).as_ref(),
    );
}

/// The raw `tmux_server` value to export as `AE_TMUX_SERVER` when the deferred
/// tick runs the recorded `ae` binary, so a supervise reaches the session's own
/// server rather than an ambient one (ae:14453-14458).
fn recorded_server_value(meta: &Meta) -> Option<String> {
    match meta.server_selector() {
        ServerSelector::Positive(crate::meta::Selector::Name(name)) => Some(name),
        ServerSelector::Positive(crate::meta::Selector::Socket(path)) => {
            Some(path.display().to_string())
        }
        ServerSelector::Missing | ServerSelector::Ambiguous => None,
    }
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
        match continuation(read.as_ref().err().map(io::Error::kind), &probe) {
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
            }
            // `Run` is only returned when the read succeeded, so this `if let`
            // is how that is spelled without an unwrap rather than a branch
            // anyone expects to take. The parse is the one taken above, not a
            // second one: two parses of one file could disagree about which
            // server this cycle is on.
            Continuation::Run => {
                if let (Ok(bytes), Some(meta)) = (&read, &parsed) {
                    let cycle = Cycle {
                        knobs,
                        meta_dir,
                        helper,
                        server: &server,
                        session,
                        goal: meta.goal().map(ToOwned::to_owned),
                        // Re-read EVERY cycle, like the goal and the roster: a
                        // session can be promoted to orchestrator, or its main
                        // replaced, while this daemon runs.
                        meta_agent: is_meta_agent(bytes),
                        roster: meta.roster().to_vec(),
                    };
                    cycle.run(&mut carry, err)?;
                    // The pane's own per-cycle duties, in the frozen wrapper's
                    // order (ae:14431-14459): the branch pair, which is a git
                    // read no cycle owns, then the two deferred ticks. AFTER the
                    // cycle deliberately — a nudge is what this process is for,
                    // and a git or a supervise that is slow must not delay one.
                    tick_pane_duties(&server, session, meta, deferred, journal, err)?;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(knobs.interval_secs));
    }
}

/// The branch publication and the two deferred bash-only ticks, once per cycle.
///
/// Every one degrades in place: a failed git read publishes "no branch", a
/// `_recover-pending` that could not run recovers nothing, and a supervise is
/// best-effort by construction. None of them is a reason to stop watching.
fn tick_pane_duties(
    server: &crate::inventory::ServerId,
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
    if !deferred.armed() {
        return Ok(());
    }
    for row in deferred.recover(session) {
        // The DURABLE record of a post-launch capture. Emitted here rather than
        // left to `_recover-pending` because the recovery itself is silent: the
        // id lands in meta and nothing else would ever say it had.
        journal.record_referring(
            "recover",
            &row.agent,
            &row.captured,
            &crate::watchdog_glue::recovered_summary(&row),
            err,
        )?;
    }
    deferred.supervise(session, SystemTime::now());
    Ok(())
}

/// Which tmux server this cycle must OBSERVE and PUBLISH on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rebind {
    /// The record MOVED: adopt this server, which is never the one already in
    /// force. Adopting is not a reassignment — see [`adopt_server`] for the
    /// retraction and the state reset that have to happen with it.
    Use(crate::inventory::ServerId),
    /// The record no longer names exactly one server. Stop watching.
    Refuse,
    /// Nothing to do: the record still names the server already in force, or
    /// it could not be read at all. An unreadable file is not a move, and
    /// neither is a record that says what it said last cycle.
    Keep,
}

/// Which server this cycle addresses, from the meta this cycle read.
///
/// # Why this is re-asked EVERY cycle, and not pinned at startup
///
/// Raised by both reviewers of the P4.2 slice. The daemon's own reads
/// (`list-panes`, `capture-pane`) and its status writes go to a server it
/// resolved ONCE; but delivery does not go through this daemon at all — it goes
/// through the fixed `<meta-dir>/send` helper, whose `_lib` re-reads
/// `tmux_server` from the CURRENT meta on every call (ae:18349). The two
/// therefore drift the moment the record changes: the daemon would observe and
/// publish on the OLD server while its nudges landed on the NEW one, and it
/// would judge panes that no longer exist against a session it can no longer
/// see. Re-resolving here makes the daemon's own server EQUAL the one delivery
/// already uses, from the same file, on the same cycle.
///
/// It is also why the resolution runs BEFORE this cycle's liveness probe: a
/// probe aimed at the abandoned server reports the session absent, and absence
/// is the one reading that ends the daemon and clears the bar. Pinning would
/// turn a selector edit into a self-terminating watchdog.
///
/// # HARD CONSTRAINT, stated where someone would be tempted
///
/// This decides THIS DAEMON'S observation target and nothing else. The send
/// helper is never handed a server: it stays the fixed, audited, no-ambient
/// path that reads current meta for itself. Passing one would replace an
/// audited resolution with this daemon's guess at it.
///
/// [`Rebind::Refuse`] mirrors the startup refusal rather than degrading: an
/// ambiguous record is also one the send helper refuses to deliver against
/// (ae:18350), so a daemon that continued would be watching a session nothing
/// can reach. Stopping is recoverable — bash's start machinery relaunches, and
/// startup re-refuses until the record is repaired.
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
///
/// A value rather than two loose arguments because both the cycle and the
/// server handover append through it, and ONE definition of the event's shape
/// is what keeps a handover diagnostic looking like every other watchdog
/// record.
struct Journal<'a> {
    meta_dir: &'a Path,
    session: &'a str,
}

impl Journal<'_> {
    /// Append one watchdog event.
    ///
    /// A failure to record is reported and the caller continues: a watchdog
    /// that exits because one append failed stops watching a live session over
    /// a full disk.
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
    /// artifact the record is about. Split from [`Journal::record`] rather than
    /// added to it because every other emission has nothing to refer to, and a
    /// `""` at each of those call sites is a field nobody reads.
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
        if let Err(why) = state::emit(self.meta_dir, &line) {
            writeln!(
                err,
                "ae: watchdog: {action} for {target} not recorded: {why}"
            )?;
        }
        Ok(())
    }
}

/// Everything this daemon carries between cycles that is scoped to ONE SERVER.
///
/// Gathered into a value so the move can drop it in a single call that cannot
/// forget a member — three loose locals were three chances to reset two.
struct Carry {
    /// Per-pane history, keyed by `pane_id`.
    panes: Vec<(String, PaneState)>,
    /// The missing-pane debounce, keyed by roster slot.
    missing: Vec<(String, MissingState)>,
    /// The rotating stabilization budget, whose cursor indexes THIS server's
    /// pane enumeration.
    quiet: QuietCycle,
}

impl Carry {
    fn new(knobs: &Knobs) -> Self {
        Self {
            panes: Vec::new(),
            missing: Vec::new(),
            quiet: QuietCycle::new(knobs.quiet_panes_per_cycle),
        }
    }

    /// Drop every carry, because the server they are scoped to is being left.
    ///
    /// PANE IDS ARE SERVER-LOCAL AND REUSABLE. `%0` on server A and `%0` on
    /// server B are different panes that happen to share a name, and every one
    /// of these collections is keyed by that name: [`PaneState`] (dead latch,
    /// delivered nudge count, undelivered streak, throttle streak, quiet
    /// baseline, the whole sweep cadence), the missing-pane debounce, and the
    /// stabilization cursor. Carrying them across a move applies the OLD
    /// server's history to whatever the new one calls `%0` — latching a live
    /// pane dead, suppressing a nudge that is due, or resuming a sweep cadence
    /// for a pane that is not the orchestrator.
    ///
    /// Nothing is salvageable by matching on something else, either: the
    /// display ref is not unique (`spawn` uniquifies only the numeric slot), so
    /// no key survives the move. The only correct carry is none.
    ///
    /// Whole-value reassignment rather than field-by-field clearing, so a field
    /// ADDED to this struct is reset by construction instead of by remembering.
    fn reset(&mut self, knobs: &Knobs) {
        *self = Self::new(knobs);
    }
}

/// Move this daemon from one server to another, in the ONE order that is safe.
///
/// Adopting a moved selector is three steps, and a caller that performs two of
/// them leaves a defect no output shows: the abandoned server keeps this
/// daemon's `@ae_*` options FOREVER (nothing ever targets it again), and the
/// new server's panes inherit the old server's per-pane history. Both were
/// found in review of the first version of this fix, which assigned the server
/// and did neither.
///
/// `leaving` is taken BY VALUE and the new server is RETURNED, so a caller
/// cannot keep addressing the old one: the borrow checker enforces the handover
/// that a `&mut` assignment left optional.
///
/// # The two halves fail differently, and that is the whole design
///
/// **Retraction is BEST-EFFORT presentation.** The server being left may be
/// DEAD — often it is the very reason the selector moved — so `retract` is
/// attempted, and a failure is reported LOUDLY (a stderr line and a durable
/// event) and then passed over. It must never gate the move: refusing to adopt
/// because we could not tidy a server that is gone would abandon a live session
/// to no watchdog at all, over a cosmetic leftover on a server nobody is
/// looking at.
///
/// **The reset is CORRECTNESS-CRITICAL and UNCONDITIONAL.** It runs whether or
/// not the retraction worked, always before anything observes the new server,
/// because the alternative is judging B's panes with A's history. There is no
/// failure mode in which carrying that state is the better answer.
///
/// # Why adopt at all, rather than refusing a move
///
/// bash's tick loop does NOT restart this daemon after it exits (ae:16627 reaps
/// orphans, then `exit 0`), so stopping on a VALID selector move would lose the
/// watchdog permanently. [`Rebind::Refuse`] stays right for Missing/Ambiguous —
/// that record is broken, the send helper refuses delivery against it too, and
/// stopping until it is repaired is the honest answer — but a well-formed move
/// is a session that is still there and still wants watching.
///
/// `retract` is a parameter rather than a direct call so the whole handover is
/// exercisable without a tmux server, including the case that matters most: the
/// old server unreachable.
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
        // reads. Targeted at `watchdog` — the subject really is this daemon,
        // and a bare name can never collide with a roster ref, which is always
        // `alias:name`, so this cannot hijack an agent's attention marker.
        journal.record(
            "alert",
            ACTOR,
            "watchdog moved servers but could not clear its options on the old one",
            err,
        )?;
    }
    drop(leaving); // the old server is unaddressable from here on, by construction
    // 2. UNCONDITIONAL: nothing keyed by a server-local pane id may survive,
    //    whether or not step 1 succeeded.
    carry.reset(knobs);
    Ok(joining)
}

/// What this cycle's own liveness readings mean for the DAEMON'S life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Continuation {
    /// Both readings are good: run the cycle.
    Run,
    /// A reading FAILED. Nothing is proven, so nothing is decided — sleep and
    /// ask again, with the bar left exactly as last published.
    Retry,
    /// Proven gone. Clear what we published and exit cleanly.
    Stop,
}

/// Whether the daemon keeps running, retries, or exits — the tri-state the rest
/// of this port already uses, applied to the daemon's OWN liveness.
///
/// # Unproven absence is not death
///
/// A long-lived daemon must survive a failed query. A `list-sessions` that could
/// not reach the server, or a `meta` read that failed for any reason other than
/// the file being gone, proves NOTHING — and a daemon that exits on one is a
/// watchdog a network hiccup can switch off, silently, for the rest of the
/// session. Same discipline as [`crate::procs::Descendancy::Unknown`] (an
/// unusable snapshot never classifies an agent dead) and the compact gate's
/// [`StopProbe`] (an unreachable server is never read as a stopped session).
///
/// # What IS proof
///
/// * [`StopProbe::Absent`] — the server answered without the name, or reported
///   the stale-socket diagnostic that only a clean server exit produces. This
///   wins over any meta reading, because the session itself is gone.
/// * [`io::ErrorKind::NotFound`] on the meta read. [`crate::meta::rewrite`]
///   publishes through a temp file and `rename`, so `meta` is never MOMENTARILY
///   absent during a write — a `NotFound` means teardown removed it, which is the
///   frozen loop's other self-termination condition (ae:16433-16441).
///
/// Everything else is [`Continuation::Retry`]: permission, a busy filesystem, an
/// unreachable server. The bar keeps its last publication, because a stale bar
/// is not a reason to stop watching.
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
///
/// # The return value is about the CLEARS, not only about reaching the server
///
/// `false` from either DISCOVERY call means the server could not be addressed
/// at all. `false` from a clear means it was addressable and the unset FAILED.
/// An earlier version discarded every clear result and answered a literal
/// `true` whenever both discoveries succeeded — so an addressable server that
/// failed to drop a bar reported success, [`adopt_server`] emitted neither its
/// stderr line nor its durable alert, and the leak was SILENT. That is the
/// exact failure the best-effort refinement exists to make visible, so the
/// accumulator is the contract and a literal `true` here is a defect.
///
/// # Why an already-clear bar does not false-positive — MEASURED, not assumed
///
/// The aggregation would be useless if unsetting an option that is already
/// unset counted as a failure: every move would emit a spurious diagnostic and
/// the signal would be noise. Probed on a throwaway socket, tmux 3.7b:
/// `set-option -u` of a NEVER-SET `@ae_*` exits 0 (session scope and window
/// scope alike), unsetting an already-cleared one exits 0 again, and only a
/// genuinely bad target (`no such session`) exits 1. So a `false` from
/// [`transport::clear_option`] on an addressable server is a REAL failure worth
/// the diagnostic. In practice this daemon republishes its bars every cycle, so
/// on a move the old server's options are set and the unset has real work to do.
///
/// # No ownership check — a known gap, named rather than hidden
///
/// The frozen exit path checks the pidfile before cleaning up (ae:15340-15352),
/// because a stop/start in quick succession can leave the OLD watchdog's trap
/// running while the NEW one is already publishing — unconditional cleanup then
/// has the dying process wipe its replacement's bar. This daemon does not own
/// that pidfile (bash does), so P4.1 clears unconditionally and the race is the
/// bash glue's to close by not starting a replacement until this one is gone.
///
/// When the SESSION is what went away there is nothing to clear — its options
/// died with it — and the id resolution simply finds nothing.
fn clear_published(server: &crate::inventory::ServerId, session: &str) -> bool {
    let Some(session_id) = transport::observe_session_id(server, session) else {
        return false;
    };
    // `&=`, NEVER `&&` and never an early return: every option must still be
    // attempted after one of them fails. Short-circuiting here would leave the
    // rest of this daemon's bars on a server nothing targets again, which is
    // the leak the aggregation exists to report.
    let mut ok = true;
    for name in [tmux::WATCHDOG_STATUS_OPTION, tmux::AGENTS_STATUS_OPTION] {
        ok &= transport::clear_option(server, OptionScope::Session, &session_id, name);
    }
    // The branch pair is published by THIS daemon too since slice A.3, so it is
    // retracted with everything else: a stopped watchdog that left `@ae_branch_*`
    // behind would keep asserting a branch nobody is watching (ae:14010).
    ok &= crate::watchdog_glue::clear_branch(server, &session_id);
    let Some(panes) = transport::observe_window_panes(server, session) else {
        return false;
    };
    let mut cleared: Vec<String> = Vec::new();
    for pane in panes {
        if cleared.contains(&pane.window_id) {
            continue;
        }
        cleared.push(pane.window_id.clone());
        ok &= transport::clear_option(
            server,
            OptionScope::Window,
            &pane.window_id,
            tmux::WINDOW_STATUS_OPTION,
        );
    }
    ok
}

/// The debounce for a roster agent whose pane is not in the session.
///
/// The EVENT fires on the first absent snapshot and the latch is for the
/// daemon's lifetime (there is no missing-cleared event); the GLYPH waits for a
/// second consecutive absence, so one unlucky enumeration does not paint a live
/// agent as gone (ae:16929-16939, ae:16991-17022).
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
    /// `meta_agent=true` — this session is the fleet orchestrator. WHICH pane gets
    /// the cadence is the pane's own `@ae_slot`, not anything from meta.
    meta_agent: bool,
}

/// What [`Cycle::apply`] is acting on: one pane, and the cycle-wide readings an
/// effect may need. A struct rather than five more parameters, because the
/// nudge arms recurse into `apply` with the same context.
struct Acting<'a> {
    agent: &'a str,
    /// The pane's `@ae_slot`, for the routed identity an event may carry.
    slot: &'a str,
    seen: &'a Observation,
    /// This cycle's events, in APPEND order — what the durable reconcile reads.
    events: &'a [Event],
}

impl Cycle<'_> {
    /// One pass over the session's panes.
    fn run(&self, carry: &mut Carry, err: &mut impl Write) -> crate::Result<()> {
        // An enumeration that FAILED is not evidence that anything is gone.
        // bash reads nothing from a `2>/dev/null` failure and then walks the
        // roster looking for panes it never saw — which alerts every agent in
        // the session as missing. Same ruling as `Descendancy::Unknown`: an
        // unusable observation decides nothing.
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

        carry.quiet.begin();
        let mut index = 0_usize;
        let mut live: Vec<String> = Vec::new();
        let mut active = 0_usize;
        let mut total = 0_usize;
        let mut dead = 0_usize;
        let mut stale = 0_usize;
        let mut by_slot: Vec<(String, Verdict)> = Vec::new();
        let mut by_pane: Vec<(String, Verdict)> = Vec::new();

        for pane in &observed {
            let Some(agent) = pane.agent.as_deref().filter(|name| !name.is_empty()) else {
                continue;
            };
            if NON_AGENT_PANES.contains(&agent) {
                continue;
            }
            index += 1;
            total += 1;
            live.push(agent.to_owned());
            let slot = pane.slot.clone().unwrap_or_default();
            let agent_bin = self.agent_bin(&slot);

            // The main loop tolerates a failed capture the way bash does (`|| true`
            // at ae:16702): an unreadable pane hashes as empty here. The
            // STABILIZER does not — see `settle`.
            let capture = transport::capture_pane(self.server, &pane.pane_id).unwrap_or_default();
            let hash = quiet_hash(&capture);
            let carried = entry_mut(&mut carry.panes, &pane.pane_id);
            let seen = Observation {
                now_epoch: now,
                hash,
                is_dead: classify_dead(
                    &pane.current_command,
                    descendancy_of(table.as_deref(), pane.pane_pid, agent_bin.as_deref()),
                ),
                is_throttled: shows_throttle(&capture, agent_bin.as_deref().unwrap_or_default()),
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
                // SC-935 is decided HERE, once, and the type carries the
                // answer: a pane that is not the orchestrator main gets `None` and
                // no sweep branch can reach it. The heartbeat is read only for
                // the pane that is, so an ordinary session never stats a file
                // it has no use for.
                sweep: is_sweep_target(self.meta_agent, &slot).then(|| {
                    SweepObservation::new(
                        SystemTime::now(),
                        heartbeat_mtime(self.meta_dir),
                        &self.knobs.sweep,
                    )
                }),
            };
            let acting = Acting {
                agent,
                slot: &slot,
                seen: &seen,
                events: &events,
            };
            let booked = account(carried, &seen, &self.knobs);
            *carried = booked.next;
            for effect in &booked.effects {
                self.apply(effect, &acting, carried, err)?;
            }
            match booked.verdict {
                Verdict::Dead => dead += 1,
                Verdict::Stale => stale += 1,
                _ => active += 1,
            }
            by_slot.push((slot, booked.verdict));
            by_pane.push((pane.pane_id.clone(), booked.verdict));
        }
        carry.quiet.end(index);
        // The roster is composed from the PRIOR debounce state, then the state is
        // advanced — the frozen order (ae:16999-17022), so a slot's first absent
        // cycle renders neutral and only the second renders ✖.
        let roster = self.roster_line(&by_slot, &carry.missing);
        self.sweep_missing(&live, &mut carry.missing, err)?;
        self.publish(&roster, bar_glyph(dead, stale), active, total, &by_pane);
        Ok(())
    }

    /// This cycle's roster line, from the daemon's own roster.
    fn roster_line(
        &self,
        by_slot: &[(String, Verdict)],
        missing: &[(String, MissingState)],
    ) -> String {
        roster_line(&self.roster, by_slot, missing)
    }

    /// Publish this cycle's verdicts as tmux user options.
    ///
    /// Every write targets an EXACT id. The session id is re-resolved each cycle
    /// rather than cached: it costs one `list-sessions`, it is what the frozen
    /// does on every publish, and a stale cached id is a write aimed at whatever
    /// tmux now calls that number. No id means the session is gone — write
    /// NOTHING, because an empty `-t` lands on tmux's CURRENT session, which is
    /// somebody else's bar.
    ///
    /// A failed publication is a stale bar, never a reason to stop watching, so
    /// nothing here is fallible to the caller.
    fn publish(
        &self,
        roster: &str,
        glyph: &str,
        active: usize,
        total: usize,
        by_pane: &[(String, Verdict)],
    ) {
        let Some(session_id) = transport::observe_session_id(self.server, self.session) else {
            return;
        };
        let _ = transport::publish_option(
            self.server,
            OptionScope::Session,
            &session_id,
            tmux::WATCHDOG_STATUS_OPTION,
            &format!("[watch {glyph} {active}/{total}]"),
        );
        // An EMPTY roster is UNSET, never published as "": a roster outliving its
        // agents would keep asserting a fleet that no longer exists (ae:17026).
        let _ = if roster.is_empty() {
            transport::clear_option(
                self.server,
                OptionScope::Session,
                &session_id,
                tmux::AGENTS_STATUS_OPTION,
            )
        } else {
            transport::publish_option(
                self.server,
                OptionScope::Session,
                &session_id,
                tmux::AGENTS_STATUS_OPTION,
                roster,
            )
        };
        self.publish_windows(by_pane);
    }

    /// Per-window glyphs, grouped from the SAME per-pane verdicts in pane order.
    ///
    /// Targeted by WINDOW ID (`@N`), never by index: closing a window renumbers
    /// the ones after it, so glyphs published against an index land on the wrong
    /// window — and cleanup then misses one and leaves exactly the frozen glyphs
    /// cleanup exists to prevent (ae:17038-17045). `@N` also carries no session
    /// name, so the prefix-match hazard cannot reach it at all.
    ///
    /// A non-agent pane REGISTERS its window and contributes no glyph, so a
    /// window that no longer holds agents is published EMPTY rather than keeping
    /// last cycle's glyphs — and the monitor window reads `99:ae-monitor`, not
    /// `99:ae-monitor··`.
    fn publish_windows(&self, by_pane: &[(String, Verdict)]) {
        let Some(panes) = transport::observe_window_panes(self.server, self.session) else {
            return;
        };
        let mut windows: Vec<(String, String)> = Vec::new();
        for pane in &panes {
            let glyphs = entry_mut(&mut windows, &pane.window_id);
            let Some(agent) = pane.agent.as_deref().filter(|name| !name.is_empty()) else {
                continue;
            };
            if NON_AGENT_PANES.contains(&agent) {
                continue;
            }
            let glyph = by_pane
                .iter()
                .find(|(pane_id, _)| *pane_id == pane.pane_id)
                .map_or(NEUTRAL_GLYPH, |(_, verdict)| verdict.glyph());
            glyphs.push_str(glyph);
        }
        for (window_id, glyphs) in &windows {
            let _ = transport::publish_option(
                self.server,
                OptionScope::Window,
                window_id,
                tmux::WINDOW_STATUS_OPTION,
                glyphs,
            );
        }
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
    ///
    /// `Done` is event-only. `WaitingUser`/`Blocked` are pane holds: the
    /// baseline arms once (settled WITHIN this cycle, and only if the rotating
    /// budget allows this pane a turn), holds while the filtered hash matches,
    /// and yields the moment anything else lands. A yield or a failed
    /// stabilization is NOT quiet — bash falls through to the normal branches
    /// (ae:16781) and so does this.
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
            .map(|(armed_key, armed_hash)| (armed_key.as_str(), *armed_hash));
        match quiet_pane_decision(query.hash, armed, &key) {
            QuietPane::Hold => Some(kind),
            QuietPane::Yield => None,
            QuietPane::Arm => {
                if !quiet_cycle.step(query.index) {
                    return None; // budget spent this cycle; try again next one
                }
                let samples = self.settle(query.pane_id);
                let borrowed: Vec<&str> = samples.iter().map(String::as_str).collect();
                let settled = quiet_stabilize(&borrowed, self.knobs.quiet_tries)?;
                state.quiet_base = Some((key, settled));
                Some(kind)
            }
        }
    }

    /// The samples a baseline must settle across: one capture, then a beat and
    /// another, up to `quiet_tries` times.
    ///
    /// A capture that FAILS truncates the run rather than contributing an empty
    /// sample. Two empty captures hash EQUAL, so an unreadable or dying pane
    /// would otherwise settle instantly and be classified quiet — the bug bash
    /// records above `_watchdog_capture_pane`.
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

    /// Perform one effect. The two nudge arms are the security-critical ones.
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
                // The DURABLE half of the wedge clear (ae:16768-16774). A
                // watchdog restarted after alerting has lost the latch, so the
                // in-memory fast path cannot fire and the alert would stand
                // forever. Read once, and only when the log still shows one:
                // an unconditional clear would retract an alert nobody raised.
                if crate::session::alert_reason_in(on.events, self.session, on.slot, agent)
                    .is_some()
                {
                    let alert = SweepAlert::ClearWedge;
                    self.emit(alert.action(), agent, &alert.summary(), err)?;
                }
                Ok(())
            }
            Effect::Nudge => {
                let display = stale_display(on.seen.last_actor_event_age_secs);
                let text = nudge_text(self.goal.as_deref(), self.meta_dir);
                let summary = format!("{display}, no recent ae activity");
                let delivered = self.deliver(agent, &text, &summary);
                for effect in record_nudge(state, delivered, &self.knobs, &display) {
                    self.apply(&effect, on, state, err)?;
                }
                Ok(())
            }
        }
    }

    /// Deliver one sweep prompt and book what happened (ae:16833-16895).
    ///
    /// # The clock is re-read AFTER the delivery
    ///
    /// `deliver` BLOCKS while the send helper defers a busy target (up to
    /// `AE_SEND_DEFER_SEC`), so the cycle's clock is stale by exactly the
    /// interval a retry is scheduled against — off it, the retry would be due
    /// the moment `deliver` returned. bash re-reads for the same reason
    /// (ae:16866-16869), and [`record_sweep`] takes both clocks so neither
    /// caller can conflate them.
    fn sweep_nudge(
        &self,
        on: &Acting<'_>,
        state: &mut PaneState,
        err: &mut impl Write,
    ) -> crate::Result<()> {
        let Some(observed) = on.seen.sweep.as_ref() else {
            // Unreachable by construction: only the sweep branch emits this
            // effect, and it runs only where the observation exists. Reported
            // rather than assumed away, because a silent no-op here would be a
            // orchestrator that is never prompted again.
            writeln!(
                err,
                "ae: watchdog: sweep prompt for {} had no sweep reading — skipped",
                on.agent
            )?;
            return Ok(());
        };
        // Delivery is CHECKED (SC-936). The status means "delivered AND
        // logged", which makes this AT-LEAST-ONCE (SC-939a): an event-write
        // failure after a successful paste re-prompts an orchestrator that already
        // swept. A duplicate sweep is cheap; a dropped one is a blind spot.
        let delivered = self.deliver(on.agent, SWEEP_PROMPT, "sweep cadence");
        let booked = record_sweep(
            &mut state.sweep,
            delivered,
            observed.now,
            SystemTime::now(),
            &self.knobs.sweep,
        );
        for effect in sweep_effects(booked) {
            self.apply(&effect, on, state, err)?;
        }
        Ok(())
    }

    /// THE ONE PLACE THIS DAEMON EXECUTES ANYTHING, and the reason both nudges
    /// route through it rather than each spawning for itself: a second delivery
    /// site is a second thing to audit, and a unit guard in this file holds the
    /// count at one.
    ///
    /// THE HELPER PATH IS A LITERAL. It is `<meta-dir>/send` and nothing else:
    /// never a program name read from meta, config, a pane, or anywhere else a
    /// value can be planted. The helper is what carries ae's input-busy,
    /// staged-detection and defer modelling, and it emits the `nudge` event
    /// itself — so the event exists only when the paste actually landed,
    /// exactly as it does for bash.
    ///
    /// The frozen action is `nudge` for BOTH the stale nudge (ae:16884-16897)
    /// and the orchestrator's sweep prompt (ae:16833-16841); only the summary
    /// distinguishes them, which is why `summary` is the one thing that varies.
    ///
    /// `true` means delivered. It is the helper's own status, which means
    /// "delivered AND logged" — never re-read as anything weaker.
    fn deliver(&self, agent: &str, text: &str, summary: &str) -> bool {
        transport::deliver(
            self.helper.path(),
            agent,
            text,
            &[
                ("AE_SENDER_OVERRIDE", ACTOR),
                ("_AE_EVENT_ACTION", "nudge"),
                ("_AE_EVENT_SUMMARY", summary),
            ],
        )
        .code
            == Some(0)
    }

    /// Append one watchdog event for `agent`.
    ///
    /// A failure to record is reported and the cycle continues: a watchdog that
    /// exits because one append failed stops watching a live session over a
    /// full disk.
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

    /// Roster agents with no live pane (ae:16929-16939).
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
            // The GLYPH debounce is keyed by SLOT (ae:17015-17021) while the
            // ALERT is keyed by the display ref, exactly as the frozen splits
            // them: two registrations can share a ref, but never a slot.
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

    /// The transient alert the frozen shows beside every watchdog event
    /// (`display-message -d 10000`, ae:16516 and siblings).
    ///
    /// THE ESCAPE IS HERE, AT THE SINK, and it is the reason this is a function
    /// rather than an inline call: `display-message` reads its argument as a
    /// FORMAT, `#(cmd)` in a format RUNS A SHELL, and the text interpolates an
    /// `alias:name` — a value that reaches ae from `spawn`, i.e. from a peer.
    /// [`tmux::format_literal`] doubles `#` and `%` exactly as the frozen
    /// `_ae_tmux_format_literal` does.
    ///
    /// A failed display is a message nobody saw, never a reason to stop watching.
    fn notify(&self, agent: &str, text: &str) {
        let Some(session_id) = transport::observe_session_id(self.server, self.session) else {
            return;
        };
        let message = tmux::format_literal(&format!("[ae watchdog] {agent} {text}"));
        let _ = transport::display_message(self.server, &session_id, &message);
    }
}

/// The roster line for `@ae_agents_status`: `<label><glyph>`, space-joined, in
/// META ORDER (ae:16986-17024).
///
/// Keyed by SLOT, never by the display ref: `spawn` uniquifies only the numeric
/// slot, so two registrations CAN share one `alias:name`. Keying on the ref
/// would let one pane's verdict stand for both, and killing one pane would then
/// not render its slot ✖ — which is the whole point of the roster.
///
/// A slot with no live pane renders from the PRIOR debounce: neutral on its
/// first absent cycle (a spawn caught mid-stamp is not a failure, and saying so
/// would be the manufactured claim the roster exists to avoid), ✖ on the second.
fn roster_line(
    roster: &[RosterEntry],
    by_slot: &[(String, Verdict)],
    missing: &[(String, MissingState)],
) -> String {
    roster
        .iter()
        .map(|entry| {
            let glyph = by_slot
                .iter()
                .find(|(slot, _)| *slot == entry.slot)
                .map_or_else(
                    || {
                        let seen_absent = missing
                            .iter()
                            .any(|(slot, state)| *slot == entry.slot && state.streak > 0);
                        if seen_absent {
                            Verdict::Dead.glyph()
                        } else {
                            NEUTRAL_GLYPH
                        }
                    },
                    |(_, verdict)| verdict.glyph(),
                );
            format!("{}{glyph}", roster_label(&entry.reference()))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `opus5:builder` -> `builder`, with control bytes stripped (ae:16205-16218).
///
/// The roster is about WHO, not which model backs them — the alias is already
/// on the pane border. An agent name cannot contain `:`, so the suffix strip is
/// unambiguous. The value lands in a tmux USER option, which interpolates
/// LITERALLY, so `#` and `%` need no escaping here; control bytes are stripped
/// anyway, because they corrupt the bar's rendering rather than merely its text.
fn roster_label(reference: &str) -> String {
    reference
        .rsplit(':')
        .next()
        .unwrap_or(reference)
        .chars()
        .filter(|c| !c.is_control())
        .collect()
}

/// The carried state for `key`, created on first sight.
///
/// An association LIST rather than a map: a session has a handful of panes, the
/// traversal order is the one thing the rotating quiet budget depends on, and a
/// `HashMap` would buy nothing here but an iteration order nobody chose.
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
///
/// A slot with no recorded `agent_bin`, and a pane tmux gave no parseable pid
/// for, are both probes that cannot RUN. Reading either as `Absent` is how a
/// live agent gets alerted dead — and it is where the frozen send path's
/// `command_is_shell(agent_bin)` guard belongs in this design (lead's ruling).
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
///
/// No `reversed()`, no sort: [`latest_relevant_event`] reverses internally and
/// its whole contract is that position, not timestamp, is the truth. A line
/// that is not a readable event is dropped — bash's `grep` would still have
/// seen it, which matters only for `_last_event_age` and only for a line no
/// emitter writes.
fn read_events(meta_dir: &Path) -> Vec<Event> {
    let bytes = crate::event_text::read_container(&meta_dir.join(crate::event_text::CONTAINER));
    crate::event_text::read_lines(&bytes)
        .into_iter()
        .filter_map(|line| Event::parse_line(&String::from_utf8_lossy(line)).ok())
        .collect()
}

/// Whether the meta declares this session the fleet orchestrator — bash's
/// `META_AGENT` (ae:16458, compared at ae:16739).
///
/// EXACTLY ONE `meta_agent` record, whose value is EXACTLY `true`. Anything
/// else — absent, `1`, `yes`, `True`, or the key named TWICE — is not the
/// orchestrator.
///
/// # Why a duplicate must fail, and why that is the SAFE direction
///
/// bash captures `grep '^meta_agent=' meta | cut -d= -f2-` into a scalar, so
/// `meta_agent=true` twice, or `true` beside `false`, makes a value with a
/// NEWLINE in it — which equals neither, so `== "true"` is false and the pane
/// keeps the NORMAL watchdog. Reading only the FIRST record made
/// `true\nfalse` and `true\ntrue` both say orchestrator, which is a divergence in
/// the dangerous direction: the sweep branch REPLACES stale escalation, so a
/// misread flag turns off the watchdog for an ordinary agent on the strength
/// of a record whose meaning is in doubt. Failing closed keeps the escalation.
///
/// [`crate::meta::sole_value`] is where that rule lives, so it is the same
/// answer `Meta::parse` gives the keys it absorbs (SC-405e).
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

/// The watchdog bar's glyph, from the cycle's COUNTS (ae:16974-16984).
///
/// Dead beats stale beats active, and the bar has only those three faces: a
/// throttled or quiet agent is counted ACTIVE here, so `[watch ⚡ …]` is not a
/// thing the frozen bar can say and is not a thing this one says either. The
/// per-agent glyph is where those verdicts show.
fn bar_glyph(dead: usize, stale: usize) -> &'static str {
    if dead > 0 {
        Verdict::Dead.glyph()
    } else if stale > 0 {
        Verdict::Stale.glyph()
    } else {
        Verdict::Active.glyph()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ACTOR, Carry, Continuation, Effect, HEARTBEAT_NAME, Journal, Knobs, MissingState,
        Observation, PaneState, Rebind, SWEEP_PROMPT, UNKNOWN_ALERT_CYCLES, Verdict, account,
        adopt_server, age_secs, bar_glyph, continuation, entry_mut, heartbeat_mtime, is_meta_agent,
        last_actor_event_age, nudge_text, read_events, rebind, record_nudge, roster_label,
        roster_line, session_name, stale_display, sweep_effects,
    };
    use crate::events::Event;
    use crate::inventory::ServerId;
    use crate::meta::{Meta, RosterEntry, Selector};
    use crate::procs::Descendancy;
    use crate::tmux::StopProbe;
    use crate::watchdog::{
        Heartbeat, QuietKind, SweepAlert, SweepEffect, SweepObservation, SweepVerdict, WedgeDetail,
    };
    use std::io::ErrorKind;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    /// A pane nobody has seen before, with nothing wrong.
    fn seen() -> Observation {
        Observation {
            now_epoch: 10_000,
            hash: 7,
            is_dead: false,
            is_throttled: false,
            quiet: None,
            descendancy: Descendancy::Present,
            last_actor_event_age_secs: 0,
            sweep: None,
        }
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
    ///
    /// The needles are split with `concat!` so this test's own source does not
    /// match them — a guard that counts itself always passes.
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
        // The tmux FORMAT sink: one call, and it is escaped. `display-message`
        // reads a format and `#(cmd)` in one runs a shell, so an unescaped call
        // is remote code execution reachable from a peer-chosen agent name.
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
        // happen. Textual, with the limits every guard in this file has — it
        // holds the shape, not the semantics.
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
        // from outside. The reset must not sit inside the branch that handles a
        // retraction failure, or a DEAD old server would leave the new one
        // judging panes with the old one's history.
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
        // has finished. A version that answered `true` on an unaddressable
        // server would silence the diagnostic the refinement exists to emit,
        // and no unit test can see that — the answer is the transport's.
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
        // AND the clears themselves are counted. The previous version of this
        // guard stopped at the two discovery exits and the final literal, and a
        // guard that asserts "returns true at the end" cannot see that the
        // `true` is a LIE about work it never checked: every `clear_option`
        // result was discarded, so an addressable server that FAILED to drop a
        // bar reported success and the leak was silent.
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
            2,
            "every clear folds into the accumulator, at BOTH scopes (session and window)"
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
                .matches(concat!("(\"_AE_EVENT_", "ACTION\", \"nudge\")"))
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
        // ae:16331-16373. A bash side that passes no flags must run the cadence
        // the frozen script ran.
        let knobs = Knobs::default();
        assert_eq!(knobs.interval_secs, 60);
        assert_eq!(knobs.stale_secs, 900);
        assert_eq!(knobs.max_nudges, 2);
        assert_eq!(knobs.throttle_alert_cycles, 5);
        assert_eq!(knobs.undelivered_max, 3);
        assert_eq!(knobs.quiet_beat_ms, 1000);
        assert_eq!(knobs.quiet_tries, 4);
        assert_eq!(knobs.quiet_panes_per_cycle, 2);
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
        // The clear runs BEFORE the quiet branch returns (ae:16713-16723), so a
        // quiet agent does not carry a stale throttle streak forever.
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
        assert_eq!(bar_glyph(0, 0), Verdict::Active.glyph());
        assert_eq!(bar_glyph(0, 3), Verdict::Stale.glyph());
        assert_eq!(bar_glyph(1, 3), Verdict::Dead.glyph());
        assert_eq!(bar_glyph(1, 0), Verdict::Dead.glyph());
        // A throttled or quiet agent is counted ACTIVE for the bar, so no other
        // glyph can reach it — `[watch ⚡ …]` is not a thing the frozen bar says.
        for glyph in [
            Verdict::Throttled.glyph(),
            Verdict::Quiet(QuietKind::Done).glyph(),
        ] {
            assert_ne!(bar_glyph(0, 0), glyph);
            assert_ne!(bar_glyph(0, 1), glyph);
            assert_ne!(bar_glyph(1, 1), glyph);
        }
    }

    #[test]
    fn every_verdict_publishes_the_frozen_glyph() {
        assert_eq!(Verdict::Dead.glyph(), "✖");
        assert_eq!(Verdict::Quiet(QuietKind::Done).glyph(), "✔");
        assert_eq!(Verdict::Quiet(QuietKind::WaitingUser).glyph(), "⏳");
        assert_eq!(Verdict::Quiet(QuietKind::Blocked).glyph(), "⛔");
        assert_eq!(Verdict::Throttled.glyph(), "⚡");
        assert_eq!(Verdict::Stale.glyph(), "◌");
        assert_eq!(Verdict::Active.glyph(), "●");
    }

    /// A roster entry as `meta` records one.
    fn entry(slot: &str, alias: &str, name: &str) -> RosterEntry {
        RosterEntry {
            slot: slot.to_owned(),
            name: name.to_owned(),
            profile: Some(alias.to_owned()),
            harness_session: None,
            binary: None,
            schema: crate::meta::RosterSchema::V1,
        }
    }

    #[test]
    fn the_roster_label_is_the_name_half_with_control_bytes_stripped() {
        assert_eq!(roster_label("opus5:builder"), "builder");
        assert_eq!(roster_label("lead"), "lead");
        // Control bytes corrupt the bar's RENDERING, not merely its text.
        assert_eq!(roster_label("cl:bui\u{7}lder\u{1b}"), "builder");
        // `#` and `%` are NOT escaped here: a user option's value interpolates
        // literally, so doubling them would render doubled.
        assert_eq!(roster_label("cl:a#b%c"), "a#b%c");
    }

    #[test]
    fn the_roster_is_meta_order_keyed_by_slot_not_by_the_display_ref() {
        let roster = vec![
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
        assert_eq!(
            roster_line(&roster, &by_slot, &[]),
            "lead● twin◌ twin✔",
            "each slot renders its OWN verdict"
        );
    }

    #[test]
    fn a_slot_with_no_pane_is_neutral_on_its_first_absent_cycle_and_dead_on_its_second() {
        let roster = vec![entry("main", "cl", "lead"), entry("worker.0", "cl", "w")];
        let by_slot = vec![("main".to_owned(), Verdict::Active)];
        // First absence: the debounce has not recorded it yet. A spawn caught
        // mid-stamp is not a failure, and ✖ would be a manufactured claim.
        assert_eq!(roster_line(&roster, &by_slot, &[]), "lead● w·");
        // Second: the streak is recorded, and now it is ✖.
        let missing = vec![(
            "worker.0".to_owned(),
            MissingState {
                streak: 1,
                alerted: true,
            },
        )];
        assert_eq!(roster_line(&roster, &by_slot, &missing), "lead● w✖");
        // The debounce is keyed by SLOT: a streak against some other slot must
        // not make this one ✖.
        let elsewhere = vec![(
            "spawned.9".to_owned(),
            MissingState {
                streak: 4,
                alerted: true,
            },
        )];
        assert_eq!(roster_line(&roster, &by_slot, &elsewhere), "lead● w·");
    }

    #[test]
    fn an_empty_roster_composes_to_nothing_so_the_caller_can_unset_it() {
        // The caller UNSETS on empty rather than publishing "" — a roster
        // outliving its agents would keep asserting a fleet that is gone.
        assert!(roster_line(&[], &[], &[]).is_empty());
    }

    #[test]
    fn a_failed_session_query_does_not_end_the_daemon() {
        // THE BLOCKER THIS FIXES. A `list-sessions` that could not reach the
        // server proves nothing, and a daemon that exits on one is a watchdog a
        // hiccup can switch off for the rest of the session.
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
        // it, which is the frozen loop's other self-termination condition.
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
        // The CONTROL is the pair: one Observation, judged twice. With no sweep
        // reading the ordinary branches call this pane STALE and nudge it; with
        // one, the cadence decides and every branch below is skipped. That is
        // the whole point of SC-935 — a monitor idling between sweeps is a
        // service at rest, not an agent that stopped working.
        let knobs = Knobs::default();
        let (prior, ordinary) = stale_pane();
        let plain = account(&prior, &ordinary, &knobs);
        assert_eq!(plain.verdict, Verdict::Stale);
        assert_eq!(plain.effects, vec![Effect::Nudge]);

        let mut orchestrator = ordinary.clone();
        orchestrator.sweep = Some(SweepObservation::new(at(0), None, &knobs.sweep));
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
        // SC-1405b, from the daemon's side: `sweep_step` answering `None` must
        // FALL THROUGH, not suppress. An empty accounting here would leave a
        // orchestrator main watched by nothing at all.
        let knobs = Knobs {
            sweep: crate::watchdog::SweepKnobs {
                sweep_secs: 0,
                ..crate::watchdog::SweepKnobs::default()
            },
            ..Knobs::default()
        };
        let (prior, mut observed) = stale_pane();
        observed.sweep = Some(SweepObservation::new(at(0), None, &knobs.sweep));
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
        observed.sweep = Some(SweepObservation::new(at(0), None, &knobs.sweep));
        let booked = account(&PaneState::default(), &observed, &knobs);
        assert_eq!(booked.verdict, Verdict::Dead);
        assert!(booked.next.dead_latched);
    }

    #[test]
    fn the_sweep_verdicts_render_the_frozen_roster_glyphs() {
        assert_eq!(Verdict::Meta(SweepVerdict::MetaSweeping).glyph(), "👁");
        assert_eq!(Verdict::Meta(SweepVerdict::MetaWedged).glyph(), "◌");
        assert_eq!(Verdict::Meta(SweepVerdict::MetaStarting).glyph(), "·");
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
                    summary: "meta-agent not sweeping — no heartbeat for 11m (may be stuck)"
                        .to_owned(),
                },
                Effect::Notify("(meta-agent) not sweeping — may be stuck".to_owned()),
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
    fn the_sweep_prompt_is_the_frozen_sentence() {
        // The text an orchestrator acts on. Broken across source lines, so assert the
        // assembled value rather than trusting the continuations.
        assert_eq!(
            SWEEP_PROMPT,
            "Run your sweep now: ae list --json, diff your state file, and report ONLY \
             new/changed attention to Clemens via say (stay silent if nothing changed). Stay in \
             'working'."
        );
    }

    #[test]
    fn the_orchestrator_flag_is_read_strictly_and_fails_closed_on_a_doubled_record() {
        // A session that gets the sweep branch stops being escalated for
        // silence, so the flag is EXACTLY ONE record saying EXACTLY `true`.
        //
        // BASH PARITY, and it is the whole point of the duplicate rows: bash
        // captures `grep '^meta_agent=' | cut -d= -f2-` into a scalar, so two
        // records make a value with a newline in it that equals neither — every
        // `== "true"` fails and the pane keeps the normal watchdog.
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
    fn the_heartbeat_is_lstatted_so_a_symlink_is_never_trusted() {
        // THE SAFETY PIN. bash's `[[ -f ]]` FOLLOWS symlinks, so anything able
        // to write in the session directory could aim the heartbeat at a file
        // some other process touches often and silence a wedged orchestrator. The
        // control is the pair: the same target file, reached directly, IS
        // trusted; reached through a link, it is not.
        let scratch = Scratch::new("hb");
        let dir = &scratch.0;
        assert_eq!(heartbeat_mtime(dir), None, "absent is untrusted");

        let real = dir.join(HEARTBEAT_NAME);
        std::fs::write(&real, b"{}").expect("write the heartbeat");
        assert!(
            heartbeat_mtime(dir).is_some(),
            "a plain regular file is the trusted case"
        );

        let elsewhere = dir.join("decoy.json");
        std::fs::write(&elsewhere, b"{}").expect("write the decoy");
        std::fs::remove_file(&real).expect("clear the way for the link");
        std::os::unix::fs::symlink(&elsewhere, &real).expect("link");
        assert_eq!(
            heartbeat_mtime(dir),
            None,
            "a symlink is refused however good its target"
        );

        std::fs::remove_file(&real).expect("clear the link");
        std::fs::create_dir(&real).expect("a directory in its place");
        assert_eq!(heartbeat_mtime(dir), None, "a directory is not a heartbeat");
    }

    #[test]
    fn an_untrusted_heartbeat_reaches_the_branch_as_untrusted() {
        // The end-to-end of the pin: the reading the loop takes is the reading
        // the decision layer classifies, and a refused file is never Fresh.
        let scratch = Scratch::new("hbclass");
        let knobs = Knobs::default();
        let observed =
            SweepObservation::new(SystemTime::now(), heartbeat_mtime(&scratch.0), &knobs.sweep);
        assert_eq!(observed.heartbeat, Heartbeat::Untrusted);
        assert_eq!(observed.heartbeat_offset, None);
    }

    #[test]
    fn the_observation_server_follows_the_record_the_send_helper_reads() {
        // The drift both reviewers named: delivery goes through
        // `<meta-dir>/send`, whose `_lib` re-reads `tmux_server` from the
        // CURRENT meta on every call. A daemon pinned at startup would observe
        // and publish on the OLD server while its nudges landed on the NEW one.
        let named =
            |value: &str| Meta::parse(&format!("tmux_server_kind=name\ntmux_server={value}\n"));
        let alpha = ServerId::Selected(Selector::Name("alpha".to_owned()));
        let beta = ServerId::Selected(Selector::Name("beta".to_owned()));

        // `Use` means a REAL MOVE and nothing else. The record still naming the
        // server in force is not a move, and answering `Use` for it would run
        // the adopt — retracting our own live bars and throwing away every
        // pane's history — on every cycle of a session that never moved.
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

        // Missing or ambiguous: stop, mirroring the startup refusal. The send
        // helper refuses delivery on an ambiguous record too, so a daemon that
        // continued would be watching a session nothing can reach.
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
            "a duplicated selector is ambiguous by SC-405l"
        );
        assert_eq!(
            rebind(
                &alpha,
                Some(&Meta::parse("tmux_server_kind=socket\ntmux_server=rel\n"))
            ),
            Rebind::Refuse,
            "a relative socket path names no server"
        );

        // An unreadable meta says NOTHING about the server. Keeping what is in
        // force is not the same as refusing: `continuation` already owns what an
        // unreadable meta means for the daemon's life, and answering it twice,
        // differently, is how a transient read error becomes a dead watchdog.
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
            state.quiet_base = Some(("alpha-declaration".to_owned(), 99));
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
    ///
    /// Field by field, because "the vector is empty" and "the next lookup is
    /// clean" are two different claims and the loop depends on the second.
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
        // THE TWO DEFECTS the re-review found, together. `%0` on alpha and `%0`
        // on beta are DIFFERENT PANES sharing a name, and every carry is keyed
        // by that name — so alpha's history would be applied to beta's live
        // pane. And alpha, never targeted again, would keep our bars forever.
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
        // The failure boundary. The server being left may be DEAD — often that
        // is WHY the selector moved — so a retraction that cannot run must not
        // gate adoption: refusing would abandon a live session to no watchdog
        // at all, over a cosmetic leftover on a server nobody is looking at.
        // The reset is the opposite: correctness-critical and unconditional.
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
