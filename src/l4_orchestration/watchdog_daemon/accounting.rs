//! The pure accounting: the nudge and challenge texts, the book_* rules and the per-pane account_* steps.

use super::deadnotice::{BACK_ACTION, Death, DeathEpisode, SHELL_ALERT};
use super::{
    Activity, Descendancy, DoneProgress, Duration, Effect, Event, FrameSample, FrameVerdict,
    HarnessState, Knobs, NON_AGENT_PANES, NUDGE_GOAL_PREFIX, NUDGE_IDLE_PREFIX, NUDGE_SENTENCE,
    NUDGE_TAIL, NUDGE_WAIT_OPEN, NUDGE_WAITING_CLOSE, NUDGE_WAITING_OPEN, Observation, PaneState,
    Path, QuietKind, SeatVerdict, SweepEffect, SystemTime, Throttle, Timestamp, UNIX_EPOCH,
    UNKNOWN_ALERT_CYCLES, Verdict, WaitEnded, WaitProgress, WaitState, classify_dead,
    descendancy_of, done_held_due, launch_grace, procs, proof_frame_step, stale_composite,
    sweep_step, target_held, task_deferred, task_lease, transport, wait_held,
};

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
pub(super) fn slot_latched(verdicts: &[(String, SeatVerdict)], slot: &str) -> bool {
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
    let restarted =
        prior.prompt_start.is_some() && seen.start.is_some() && prior.prompt_start != seen.start;
    let dialog = seen.sweep.as_ref().map(|_| prompt_identity(prompt));
    let changed = dialog.is_some() && prior.sweep_prompt != dialog;
    let streak = if restarted || changed {
        0
    } else {
        prior.human_prompt_streak
    };
    next.human_prompt_streak = streak.saturating_add(1);
    next.prompt_start.clone_from(&seen.start);
    next.sweep_prompt = dialog;
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
pub(super) fn naming(prompt: &crate::watchdog::HumanPrompt) -> [Effect; 2] {
    let summary = prompt_summary(prompt);
    [
        Effect::Emit {
            action: "human-prompt",
            summary: summary.clone(),
        },
        Effect::Notify(format!("waiting for you: {summary}")),
    ]
}

fn prompt_summary(prompt: &crate::watchdog::HumanPrompt) -> String {
    let mut summary = format!("{} — press: {}", prompt.question, prompt.keys);
    if let Some(notice) = prompt.startup_notice() {
        summary.push_str(" — ");
        summary.push_str(&notice);
    }
    crate::board::terminal_text(&summary)
}

/// Only the question and its action identify the kind of dialog being held.
/// Wrapped or refreshed warning text still appears in the notice.
pub(super) fn prompt_identity(prompt: &crate::watchdog::HumanPrompt) -> u64 {
    super::quiet_hash(&crate::board::terminal_text(&format!(
        "{} — {} — {}",
        prompt.kind.label(),
        prompt.question,
        prompt.keys
    )))
}

/// A read of the clear pane retracts the naming, including one from a previous daemon.
fn clear_prompt(
    prior: &PaneState,
    next: &mut PaneState,
    effects: &mut Vec<Effect>,
    seen: &Observation,
    knobs: &Knobs,
) {
    if seen.capture_ok && seen.human_prompt.is_none() {
        if prior.human_prompt_streak >= knobs.human_prompt_cycles || seen.prompt_named {
            effects.push(Effect::Emit {
                action: "human-prompt-cleared",
                summary: "the human-only prompt is gone from the pane".to_owned(),
            });
        }
        next.human_prompt_streak = 0;
        next.prompt_start = None;
        next.sweep_prompt = None;
    }
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
        && !seen.harness.frame.is_busy()
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
    rung: u8,
) -> Option<Verdict> {
    if !waiting_agent_escalates(seen, knobs, rung) {
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

/// Whether this cycle's quiet `waiting-agent` is past its ceiling on `rung`
/// ([`book_wait_hold`]'s promoted rung) — the one answer the escalated
/// verdict and the challenge text both read.
pub(super) fn waiting_agent_escalates(seen: &Observation, knobs: &Knobs, rung: u8) -> bool {
    seen.quiet == Some(QuietKind::WaitingAgent)
        && !seen.wait_ends_elsewhere
        && crate::watchdog::waiting_agent_escalated_at(
            seen.declared_age_secs,
            knobs.idle_nudge_secs,
            rung,
        )
}

/// One cycle's holds of a due wait challenge (#225).
pub(super) struct WaitHold {
    /// The due challenge waits this cycle.
    pub(super) held: bool,
    /// The rung the `waiting-agent` ceiling is read on this cycle.
    pub(super) rung: u8,
    /// The `wait-held` record this cycle owes, when its reason is stronger
    /// than the episode's witness.
    pub(super) record: Option<Effect>,
}

/// The two holds of a due wait (#225): the unchanged-proof hold
/// ([`wait_held`]) and, for `waiting-agent` only, the target hold
/// ([`target_held`]) on a direct same-session target that works and moves.
///
/// Two rules make every end of a target hold journal a challenge in the same
/// cycle, the record that closes it for `ae list`: no target hold on a spent
/// delivery budget, and once an episode is target-held the unchanged hold no
/// longer applies and a closed target hold never reopens. The promotion to
/// the top rung reads the journaled witness, or this cycle's own record.
pub(super) fn book_wait_hold(
    seen: &Observation,
    activity: &Activity,
    (proof, frame): (crate::watchdog::ProofQuiet, FrameVerdict),
    knobs: &Knobs,
    wait_state: Option<WaitState>,
) -> WaitHold {
    use crate::watchdog::{HoldReason, HoldWitness};
    let cadence = knobs.idle_nudge_secs;
    let witness = activity.wait_witness;
    let target_seen = witness.is_some_and(|known| known.reason == HoldReason::Target);
    let spent = matches!(
        seen.wait_progress,
        WaitProgress::ChallengeDue { attempts, .. } if attempts >= knobs.undelivered_max
    );
    let target = wait_state == Some(WaitState::WaitingAgent)
        && !spent
        && witness.is_none_or(|known| known.reason != HoldReason::Target || known.open)
        && target_held(
            seen.wait_progress,
            activity.wait_target.as_ref(),
            seen.now_epoch,
            cadence,
        );
    let unchanged = !target_seen && wait_held(seen.wait_progress, proof, frame, cadence);
    let reason = if target {
        Some(HoldReason::Target)
    } else if unchanged {
        Some(HoldReason::Unchanged)
    } else {
        None
    };
    let owed = reason.filter(|reason| {
        wait_state.is_some() && witness.is_none_or(|known| *reason > known.reason)
    });
    let record = owed.map(|reason| Effect::WaitHeld {
        reason,
        on: activity
            .wait_target
            .as_ref()
            .filter(|_| reason == HoldReason::Target)
            .map(|held| held.agent.clone()),
    });
    let judged = if owed == Some(HoldReason::Target) {
        Some(HoldWitness {
            reason: HoldReason::Target,
            open: true,
        })
    } else {
        witness
    };
    WaitHold {
        held: target || unchanged,
        rung: crate::watchdog::promoted_rung(
            seen.wait_progress,
            seen.wait_rung,
            judged,
            target,
            cadence,
        ),
        record,
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

/// Whether the seat is latched dead before this cycle's booking: the carried
/// latch, or a dead alert the journal holds for its current episode, which is
/// how a restarted daemon learns a death it already alerted.
fn dead_before(prior: &PaneState, activity: &Activity) -> bool {
    prior.dead_latched || matches!(activity.death, DeathEpisode::Open(_))
}

/// The TWO ends a death latch has: a POSITIVE process-tree reading shows the
/// seat's harness back under the pane (the human's re-run in place), or a
/// closer ended the episode in the journal — the seat's own later activity (a
/// dead agent journals nothing, so its own record is proof of life the process
/// name cannot give), a spawn, reseat or `ae _run` start of the seat. The
/// journal is read without the session-start cut-off the outstanding alert
/// has, so a latch rebuilt after a restart never clears on that cut-off alone;
/// [`DeathEpisode::Unread`] keeps the outstanding-alert rule.
/// An UNKNOWN snapshot is not evidence of life, so a probe gap — and a
/// reading that says the process is still gone — keeps the latch and returns
/// `None`. On a clear it emits the one closer and hands back the episode the
/// ordinary judgement must run on: identity kept, and every clock, hash and
/// nudge field the death interrupted reset, because a pre-death hash or idle
/// clock must not feed a stale verdict. The closer is `dead-cleared`, which
/// retracts the human's alert, unless the standing alert says the pane was
/// gone: then it is `pane-back`, which ends the episode and leaves the
/// pane-missing alert alone. No hysteresis and no second-alert suppression:
/// the caller alerts again if the seat dies again, because that is a real
/// event each time.
fn clear_death_latch(
    prior: &PaneState,
    next: &PaneState,
    (seen, activity): (&Observation, &Activity),
    effects: &mut Vec<Effect>,
) -> Option<PaneState> {
    if !dead_before(prior, activity) {
        return None;
    }
    let closed = match activity.death {
        DeathEpisode::Unread => seen.outstanding_alert.is_none(),
        DeathEpisode::Open(_) => false,
        DeathEpisode::Closed => true,
    };
    let back = matches!(seen.descendancy, Descendancy::Present) || closed;
    if !back {
        return None;
    }
    if activity.death == DeathEpisode::Open(Death::PaneGone) {
        effects.push(Effect::Emit {
            action: BACK_ACTION,
            summary: "pane back — agent visible in session again".to_owned(),
        });
    } else {
        effects.push(Effect::Emit {
            action: "dead-cleared",
            summary: "agent process back — resumed in place".to_owned(),
        });
        effects.push(Effect::Notify("is BACK — process resumed".to_owned()));
    }
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
    account_settling(prior, seen, knobs, trust, activity, false)
}

/// [`account_with_activity`], inside or outside the wake settle (#238):
/// `settling` is [`crate::watchdog::settle`]'s hold for this cycle. While it
/// holds, every effect that reaches the seat ([`Effect::held_while_settling`])
/// is dropped before anything applies it, and no delivery counter moves: each
/// one is booked only when its delivery is applied, so the next free cycle
/// books the same delivery again. Observation counters such as the
/// human-prompt streak advance as on any cycle. `false` is exactly [`account_with_activity`].
#[must_use]
pub fn account_settling(
    prior: &PaneState,
    seen: &Observation,
    knobs: &Knobs,
    trust: Option<&crate::autotrust::Due>,
    activity: &Activity,
    settling: bool,
) -> Accounting {
    let mut booked = book_pane(prior, seen, knobs, trust, activity);
    if settling {
        // A held trust answer that was to name its prompt names it now: the
        // naming cycle is the bound, and the streak passes it either way.
        let named = booked.effects.contains(&Effect::AnswerTrust { name: true });
        booked
            .effects
            .retain(|effect| !effect.held_while_settling());
        if named && let Some(prompt) = seen.human_prompt.as_ref() {
            booked.effects.extend(naming(prompt));
        }
    }
    booked
}

/// The branch order of one pane in one cycle, outside the wake settle.
fn book_pane(
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
    //    A dead alert the journal already holds for this episode latches a
    //    fresh carry too, so a restart never alerts the same death again.
    let unlatched;
    match clear_death_latch(prior, &next, (seen, activity), &mut effects) {
        Some(state) => {
            unlatched = state;
            prior = &unlatched;
            next = unlatched.clone();
        }
        None if dead_before(prior, activity) => {
            next.dead_latched = true;
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
            summary: SHELL_ALERT.to_owned(),
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
                busy: seen.harness.frame.is_busy(),
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

    // 3. A sweep holds behind the prompt, without moving its cadence or budget.
    //    Only a clear read releases a held prompt. The existing trust proof
    //    still applies, and the first clear cycle can book its overview.
    if seen.sweep.is_some() && knobs.sweep.enabled() {
        if seen.human_prompt.is_some() {
            book_human_prompt(prior, &mut next, &mut effects, (seen, trust), knobs);
        }
        if seen.human_prompt.is_some()
            || (!seen.capture_ok && (prior.human_prompt_streak > 0 || seen.prompt_named))
        {
            next.idle_since_epoch = None;
            return Accounting {
                next,
                effects,
                verdict: Verdict::HumanPrompt,
                moved: false,
            };
        }
        clear_prompt(prior, &mut next, &mut effects, seen, knobs);
    }
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
    clear_prompt(prior, &mut next, &mut effects, seen, knobs);

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
    let hold = book_wait_hold(seen, activity, (quiet_proof, frame), knobs, wait_state);
    effects.extend(hold.record);
    if !pasting
        && !hold.held
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
            escalated: waiting_agent_escalates(seen, knobs, hold.rung),
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
            book_waiting_agent_escalation(prior, &mut next, &mut effects, seen, knobs, hold.rung)
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

    // 8.5. The ordinary pane's human-only prompt sits below dead,
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
        HarnessState::Busy | HarnessState::CodexServiceWait => {
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
pub(super) fn capture_pane(server: &crate::inventory::ServerId, pane: &str) -> (String, bool) {
    match transport::capture_pane(server, pane) {
        Some(text) => (text, true),
        None => (String::new(), false),
    }
}

/// Restore only the idle clock fields owned by the public observed option.
/// The identity guard prevents a reused pane id inheriting another seat's clock.
pub(super) fn restore_idle(state: &mut PaneState, raw: &str, identity: u64) {
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
pub(super) fn observed_option(frame: HarnessState, state: &PaneState) -> String {
    if frame.is_busy() {
        return frame.option_word().to_owned();
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
pub(super) fn sweep_effects(booked: Vec<SweepEffect>) -> Vec<Effect> {
    let mut out = Vec::new();
    for effect in booked {
        match effect {
            // One delivery effect: the turn decides at apply whether it is an
            // overview or the standalone desk-stale tick.
            SweepEffect::FireSweepNudge | SweepEffect::FireDeskTick => out.push(Effect::SweepNudge),
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
pub(super) fn holds_seat(current_command: &str, descendancy: Descendancy) -> bool {
    !classify_dead(current_command, descendancy)
        && !crate::watchdog::command_is_shell(current_command)
}

/// The seats this enumeration proves are still held, by [`holds_seat`].
///
/// The monitor panes are not agents and never hold anybody's work.
#[must_use]
pub(super) fn held_seats(
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
pub(super) fn deferred(
    own: crate::session::OwnWork,
    now_epoch: i64,
    idle_age: u64,
    knobs: &Knobs,
) -> bool {
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
pub(super) fn last_done_event_at(
    events: &[Event],
    session: &str,
    agent: &str,
) -> Option<SystemTime> {
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
pub(super) fn last_working_declaration_at(
    events: &[Event],
    session: &str,
    agent: &str,
) -> Option<SystemTime> {
    let latest = events
        .iter()
        .rev()
        .find(|event| main_actor(event, session, agent) && event.declared_state().is_some())?;
    if latest.declared_state() != Some("working") {
        return None;
    }
    system_time_from_epoch(latest.ts.epoch())
}

pub(super) fn system_time_from_epoch(epoch: i64) -> Option<SystemTime> {
    if epoch >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(u64::try_from(epoch).ok()?))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(epoch.unsigned_abs()))
    }
}

pub(super) fn epoch_second(at: SystemTime) -> i64 {
    match at.duration_since(UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        Err(before) => {
            i64::try_from(before.duration().as_secs()).map_or(i64::MIN, i64::saturating_neg)
        }
    }
}

/// The age reported for an agent with no event at all.
pub const NO_EVENT_AGE: u64 = 999_999;
