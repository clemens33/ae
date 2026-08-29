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
//! # KNOWN TEMPORARY GAP: the meta-agent sweep
//!
//! The frozen watchdog has a steward branch (ae:16523-16695) — sweep cadence,
//! heartbeat wedge detection, its own alerts — and it is NOT ported here.
//! Colead deferred it to the steward slice. Until then a meta-agent session is
//! watched by the ordinary rules, which will read a steward idling between
//! sweeps as stale. Deliberate, dated, and not a silent omission.
//!
//! # What else stays in bash this slice
//!
//! The Telegram supervise and the pending tool-session-id recovery. They are not
//! ported here and this module must not be read as replacing them.

use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use crate::events::Event;
use crate::meta::{Meta, RosterEntry, ServerSelector};
use crate::procs::{self, Descendancy};
use crate::state;
use crate::time::Timestamp;
use crate::tmux::{self, OptionScope, StopProbe};
use crate::tracked::{self, EventFields};
use crate::transport;
use crate::watchdog::{
    QuietCycle, QuietKind, QuietPane, classify_dead, declaration_key, latest_relevant_event,
    quiet_hash, quiet_pane_decision, quiet_reason, quiet_stabilize, shows_throttle,
    stale_composite,
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

/// Account for one pane in one cycle — the frozen loop's branch order, and the
/// only place any of it is decided.
///
/// The order is bash's, and it is load-bearing rather than incidental:
///
/// 1. a LATCHED dead pane is skipped entirely (ae:16490-16495);
/// 2. the dead check, which latches and alerts once (ae:16503-16519);
/// 3. the throttle streak CLEAR, which runs before every later branch so a
///    quiet agent's throttle still clears (ae:16713-16723);
/// 4. quiet suppression, which resets the delivered count and counts active
///    (ae:16739-16790);
/// 5. throttle, reacted to BEFORE active/recent so the alert cadence starts
///    immediately rather than after the whole stale window (ae:16787-16818);
/// 6. active / recently visible / recently alive (ae:16821-16856);
/// 7. stale: nudge, or the one max-nudges alert (ae:16858-16922).
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

    // 3. The throttle streak clears on ANY non-throttled cycle, before the
    //    branches below can return.
    if !seen.is_throttled && prior.throttle_streak > 0 {
        effects.push(Effect::Emit {
            action: "throttle-cleared",
            summary: format!("throttling cleared after {} cycles", prior.throttle_streak),
        });
        next.throttle_streak = 0;
    }

    // 4. A held quiet state is "leave me alone": no nudge, and the delivered
    //    count resets so a later stale run starts fresh.
    if let Some(kind) = seen.quiet {
        next.nudge_count = 0;
        return Accounting {
            next,
            effects,
            verdict: Verdict::Quiet(kind),
        };
    }

    // 5. Throttled: a nudge cannot fix upstream, and the hash bookkeeping is
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

    // 6. Active — the pane moved since the last cycle. A moving pane is also
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

    // 7. Stale, or one of the two "recent" escapes. One definition of the
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

    let display = stale_display(seen.last_actor_event_age_secs);
    if prior.undelivered_streak >= knobs.undelivered_max {
        // Bounded: an unreachable pane must stop costing cycles, and each send
        // can hold the loop for its whole defer window.
    } else if prior.nudge_count < knobs.max_nudges {
        effects.push(Effect::Nudge);
    } else if prior.nudge_count == knobs.max_nudges {
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
    Accounting {
        next,
        effects,
        verdict: Verdict::Stale,
    }
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
pub fn run(meta_dir: &Path, knobs: Knobs, err: &mut impl Write) -> crate::Result<u8> {
    let Ok(bytes) = crate::meta::read_bytes(meta_dir) else {
        writeln!(
            err,
            "ae: watchdog: no session state at {}",
            meta_dir.display()
        )?;
        return Ok(1);
    };
    let meta = Meta::parse(&String::from_utf8_lossy(&bytes));
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
    let mut panes: Vec<(String, PaneState)> = Vec::new();
    let mut missing: Vec<(String, MissingState)> = Vec::new();
    let mut quiet_cycle = QuietCycle::new(knobs.quiet_panes_per_cycle);

    loop {
        let read = crate::meta::read_bytes(meta_dir);
        let probe = transport::verify_session_absent(&server, &session);
        match continuation(read.as_ref().err().map(io::Error::kind), &probe) {
            Continuation::Stop => {
                // PROVEN gone. Only here is the bar cleared.
                clear_published(&server, &session);
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
            // anyone expects to take.
            Continuation::Run => {
                if let Ok(bytes) = &read {
                    let meta = Meta::parse(&String::from_utf8_lossy(bytes));
                    let cycle = Cycle {
                        knobs,
                        meta_dir,
                        helper: &helper,
                        server: &server,
                        session: &session,
                        goal: meta.goal().map(ToOwned::to_owned),
                        roster: meta.roster().to_vec(),
                    };
                    cycle.run(&mut panes, &mut missing, &mut quiet_cycle, err)?;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(knobs.interval_secs));
    }
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

/// Remove everything this daemon published, on its own clean exit.
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
fn clear_published(server: &crate::inventory::ServerId, session: &str) {
    let Some(session_id) = transport::observe_session_id(server, session) else {
        return;
    };
    for name in [tmux::WATCHDOG_STATUS_OPTION, tmux::AGENTS_STATUS_OPTION] {
        let _ = transport::clear_option(server, OptionScope::Session, &session_id, name);
    }
    let Some(panes) = transport::observe_window_panes(server, session) else {
        return;
    };
    let mut cleared: Vec<String> = Vec::new();
    for pane in panes {
        if cleared.contains(&pane.window_id) {
            continue;
        }
        cleared.push(pane.window_id.clone());
        let _ = transport::clear_option(
            server,
            OptionScope::Window,
            &pane.window_id,
            tmux::WINDOW_STATUS_OPTION,
        );
    }
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
}

impl Cycle<'_> {
    /// One pass over the session's panes.
    fn run(
        &self,
        panes: &mut Vec<(String, PaneState)>,
        missing: &mut Vec<(String, MissingState)>,
        quiet_cycle: &mut QuietCycle,
        err: &mut impl Write,
    ) -> crate::Result<()> {
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

        quiet_cycle.begin();
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
            let carried = entry_mut(panes, &pane.pane_id);
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
                    quiet_cycle,
                ),
                descendancy: descendancy_of(table.as_deref(), pane.pane_pid, agent_bin.as_deref()),
                last_actor_event_age_secs: last_actor_event_age(&events, agent, now),
            };
            let booked = account(carried, &seen, &self.knobs);
            *carried = booked.next;
            for effect in &booked.effects {
                self.apply(effect, agent, &seen, carried, err)?;
            }
            match booked.verdict {
                Verdict::Dead => dead += 1,
                Verdict::Stale => stale += 1,
                _ => active += 1,
            }
            by_slot.push((slot, booked.verdict));
            by_pane.push((pane.pane_id.clone(), booked.verdict));
        }
        quiet_cycle.end(index);
        // The roster is composed from the PRIOR debounce state, then the state is
        // advanced — the frozen order (ae:16999-17022), so a slot's first absent
        // cycle renders neutral and only the second renders ✖.
        let roster = self.roster_line(&by_slot, missing);
        self.sweep_missing(&live, missing, err)?;
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

    /// Perform one effect. The nudge arm is the security-critical one.
    fn apply(
        &self,
        effect: &Effect,
        agent: &str,
        seen: &Observation,
        state: &mut PaneState,
        err: &mut impl Write,
    ) -> crate::Result<()> {
        match effect {
            Effect::Emit { action, summary } => self.emit(action, agent, summary, err),
            Effect::Notify(text) => {
                self.notify(agent, text);
                Ok(())
            }
            Effect::Nudge => {
                let display = stale_display(seen.last_actor_event_age_secs);
                let text = nudge_text(self.goal.as_deref(), self.meta_dir);
                let summary = format!("{display}, no recent ae activity");
                // THE HELPER PATH IS A LITERAL. It is `<meta-dir>/send` and
                // nothing else: never a program name read from meta, config,
                // a pane, or anywhere else a value can be planted. The send
                // helper is what carries ae's input-busy, staged-detection and
                // defer modelling, and it emits the `nudge` event itself — so
                // the event exists only when the paste actually landed, exactly
                // as it does for bash.
                let delivery = transport::deliver(
                    self.helper.path(),
                    agent,
                    &text,
                    &[
                        ("AE_SENDER_OVERRIDE", ACTOR),
                        ("_AE_EVENT_ACTION", "nudge"),
                        ("_AE_EVENT_SUMMARY", &summary),
                    ],
                );
                let delivered = delivery.code == Some(0);
                for effect in record_nudge(state, delivered, &self.knobs, &display) {
                    self.apply(&effect, agent, seen, state, err)?;
                }
                Ok(())
            }
        }
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
        let line = tracked::event_line(&EventFields {
            ts: Timestamp::now(),
            actor: ACTOR,
            action,
            target: agent,
            reference: "",
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
                "ae: watchdog: {action} for {agent} not recorded: {why}"
            )?;
        }
        Ok(())
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
        Continuation, Effect, Knobs, MissingState, Observation, PaneState, UNKNOWN_ALERT_CYCLES,
        Verdict, account, age_secs, bar_glyph, continuation, last_actor_event_age, nudge_text,
        record_nudge, roster_label, roster_line, session_name, stale_display,
    };
    use crate::events::Event;
    use crate::meta::RosterEntry;
    use crate::procs::Descendancy;
    use crate::tmux::StopProbe;
    use crate::watchdog::QuietKind;
    use std::io::ErrorKind;
    use std::path::Path;

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
            "a second delivery site is a second thing to audit"
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
                .matches(concat!("clear_published(&", "server"))
                .count(),
            1,
            "the bar is cleared on ONE path, and it is the proven-absence one"
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
            alias: alias.to_owned(),
            name: name.to_owned(),
            session_id: None,
            binary: None,
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
}
