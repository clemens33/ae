//! The watchdog's per-agent decisions — the pure classification the daemon loop
//! makes each cycle over normalized pane observations.
//!
//! No tmux, no I/O, no clock: the loop gathers observations (a pane's foreground
//! command, whether an agent process runs beneath it, its recent rendered output)
//! and delivers effects (a nudge through the session's own `send` helper, the
//! tmux status options); THIS module only decides.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::events::{Event, Identity, RoutingMember};
use crate::procs::Descendancy;

/// The shells the dead-check treats as "no agent in the foreground".
#[must_use]
pub fn command_is_shell(cmd: &str) -> bool {
    matches!(cmd, "bash" | "zsh" | "fish" | "sh" | "dash" | "")
}

/// Whether a pane's agent has DIED — dropped to a bare shell with no agent
/// process beneath it.
#[must_use]
pub fn classify_dead(current_command: &str, descendant: Descendancy) -> bool {
    command_is_shell(current_command) && matches!(descendant, Descendancy::Absent)
}

/// Which class of upstream trouble a pane shows: a TRANSIENT throttle clears
/// upstream on its own; the vendor's usage limit waits for a reset or re-login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Throttle {
    /// Transient rate limiting or overload — upstream recovers on its own.
    Throttled,
    /// The vendor's own usage limit, which persists until a reset or a
    /// re-login.
    LimitReached,
}

/// The TRANSIENT throttle phrases keyed by agent BINARY, at MODULE level rather
/// than inside [`throttle_class`]: a `const` declared after that function's
/// empty-buffer guard is `clippy::items_after_statements`, and this crate gates
/// on `-D warnings`.
const CLAUDE: &[&str] = &[
    "Server is temporarily limiting requests",
    "API Error: Overloaded",
    "Anthropic API error",
];
const CODEX: &[&str] = &[
    "Rate limit exceeded",
    "RateLimitError",
    "ratelimit_exceeded",
];
const GEMINI: &[&str] = &["RESOURCE_EXHAUSTED", "Quota exceeded"];
/// The pair that applies to EVERY tool — an unknown binary matches only these.
const GENERIC: &[&str] = &["429 Too Many Requests", "503 Service Unavailable"];

/// The USAGE-LIMIT phrases keyed by agent BINARY, MEASURED from each tool's own
/// strings (provenance: `.local/limitstate-evidence.md`). No measurement, no
/// phrase. `You've hit your` is the vendor's own `_nr` prefix matcher.
const CLAUDE_LIMIT: &[&str] = &[
    "You've hit your",
    "You're out of usage credits",
    "Your org is out of usage",
    "usage limit reached",
];
const CODEX_LIMIT: &[&str] = &[
    "You've hit your usage limit",
    "You've reached your usage limit",
    "Quota exceeded. Check your plan",
];

/// A prompt only the HUMAN may answer, and what it takes to answer it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanPrompt {
    /// The question row, trimmed — WHAT is being asked.
    pub question: String,
    /// The key-hint row, trimmed — WHAT the human must press. This is the whole
    /// point of naming the prompt: a seat nobody can act on is not news.
    pub keys: String,
}

/// Whether `buf` shows a prompt only the HUMAN may answer, for `agent_bin` —
/// the ONE detector, pure, and it NEVER sends a key.
///
/// The gate is an exact binary name, like [`throttle_class`], looked up in the
/// adapter table: a tool's row names its prompt (`tool::PromptSpec`)
/// and its composer, so a second tool is a second ROW, never a second branch
/// here, and no tool is named in this half. A renamed binary, or a tool with
/// no prompt measured, reads as no prompt, which degrades SAFE (no notify, and
/// readiness still refuses what it cannot prove). The window is a fixed count
/// of ROWS up from the last non-blank row, not a trailing run of non-blank
/// rows: a modal carries blank rows between its parts, and agy's ends on a
/// status line, so a run degenerates to that one line. Height-independent, it
/// covers a fresh-pane modal and a mid-session one the same way. Everything
/// must hold together inside it: a question row, a selected option row with
/// at least one sibling, a key-hint row after them, the row's title under its
/// rule where it names one, and NO composer — that last read by the
/// composer's own owner rather than a second copy of its fence, so the two
/// cannot drift apart.
#[must_use]
pub fn human_prompt_class(buf: &str, agent_bin: &str) -> Option<HumanPrompt> {
    let adapter = crate::tool::ToolKind::from_known_binary_name(agent_bin)?.adapter();
    let spec = adapter.prompt?;
    let rows: Vec<&str> = buf.lines().collect();
    let last = rows.iter().rposition(|row| !row.trim().is_empty())?;
    let window = &rows[last.saturating_sub(spec.window.saturating_sub(1))..=last];
    // Scoped to the WINDOW, not the buffer: a modal drawn BELOW a composer is
    // the case this must still catch. The inverse — a draft whose own text is
    // shaped like a modal, with the fence above the window — is accepted.
    let input = adapter.input;
    if crate::deliver::region::composer_drawn(&window.join("\n"), input.model, input.composed) {
        return None;
    }
    // EVERY question row is tried, top-down, and the first COMPLETE shape wins.
    // A single-shot first match would blind the detector on its own evidence: a
    // transcript row above the modal that merely ENDS in `?`, paired with the
    // real hint row below it, encloses no selected option — and the live modal
    // under it would read as nothing at all. One residual is named rather than
    // fixed: when leaked scrollback inside the window carries its own question,
    // that row can be the one REPORTED, while the latch and the keys stay the
    // modal's.
    window
        .iter()
        .enumerate()
        .filter(|(_, row)| asks(spec.question, row))
        .find_map(|(question, _)| {
            if spec
                .title
                .is_some_and(|title| !titled(window, question, title))
            {
                return None;
            }
            let after = &window[question + 1..];
            let keys = after
                .iter()
                .position(|row| spec.keys.iter().any(|key| row.contains(key)))?;
            // TWO option rows at least, and one of them SELECTED: a lone
            // paragraph between a question and a hint is prose, not a choice.
            // The sibling may sit on EITHER side of the selected row — the
            // selection travels with the human's arrow keys, and on the last
            // option there is nothing below it to find.
            let options = &after[..keys];
            let selected = options
                .iter()
                .position(|row| row.trim_start().starts_with(spec.selected))?;
            let sibling = |rows: &[&str]| rows.iter().any(|row| !row.trim().is_empty());
            if !sibling(&options[..selected]) && !sibling(&options[selected + 1..]) {
                return None;
            }
            Some(HumanPrompt {
                question: reported(spec.question, window[question]),
                keys: after[keys].trim().to_owned(),
            })
        })
}

/// Whether `row` is the prompt's question row.
fn asks(question: crate::tool::Question, row: &str) -> bool {
    match question {
        crate::tool::Question::EndsWith(end) => row.trim_end().ends_with(end),
        crate::tool::Question::StartsWith(start) => row.trim_start().starts_with(start),
    }
}

/// The question as the human is told it: the whole row, or — for one that
/// runs on past its question — the row through its first `?`, which reads the
/// same at every width that keeps the `?` on the row.
fn reported(question: crate::tool::Question, row: &str) -> String {
    let row = row.trim();
    match question {
        crate::tool::Question::EndsWith(_) => row.to_owned(),
        crate::tool::Question::StartsWith(_) => {
            row.split_inclusive('?').next().unwrap_or(row).to_owned()
        }
    }
}

/// Whether a `title` row sits ABOVE the question at `question`, directly under
/// a rule of `─` at least as wide as every row from it to the window's end —
/// the modal's own frame, which a transcript quoting its prose does not draw.
/// The rows ABOVE the rule do not count: a watchdog capture joins a wrapped
/// launch line into one row wider than the pane.
fn titled(window: &[&str], question: usize, title: &str) -> bool {
    (1..question).any(|at| {
        let rule = window[at - 1];
        let width = rule.chars().count();
        window[at].trim_start().starts_with(title)
            && width > 0
            && rule.chars().all(|ch| ch == '─')
            && window[at..].iter().all(|row| row.chars().count() <= width)
    })
}

/// Which class of upstream trouble `buf` shows for `agent_bin`, if any — the
/// ONE classifier, of which [`shows_throttle`] is the union answer. A
/// `LimitReached` phrase wins: the usage limit outlives the cycle.
#[must_use]
pub fn throttle_class(buf: &str, agent_bin: &str) -> Option<Throttle> {
    if buf.is_empty() {
        return None;
    }
    // opencode is the union — refactor here, never duplicate, so the branches
    // cannot drift.
    let (transient, limit): (&[&[&str]], &[&[&str]]) = match agent_bin {
        "claude" => (&[CLAUDE], &[CLAUDE_LIMIT]),
        "codex" => (&[CODEX], &[CODEX_LIMIT]),
        "gemini" => (&[GEMINI], &[]),
        "opencode" => (&[CLAUDE, CODEX, GEMINI], &[CLAUDE_LIMIT, CODEX_LIMIT]),
        _ => (&[], &[]),
    };
    let shows = |sets: &[&[&str]]| {
        sets.iter()
            .flat_map(|set| set.iter())
            .any(|pattern| buf.contains(pattern))
    };
    if shows(limit) {
        return Some(Throttle::LimitReached);
    }
    if shows(transient) || GENERIC.iter().any(|pattern| buf.contains(pattern)) {
        return Some(Throttle::Throttled);
    }
    None
}

/// Whether the captured pane buffer shows upstream throttling of EITHER class
/// for the agent whose binary is `agent_bin`.
#[must_use]
pub fn shows_throttle(buf: &str, agent_bin: &str) -> bool {
    throttle_class(buf, agent_bin).is_some()
}

/// Whether an agent is STALE — the composite the watchdog's branches 4, 5 and 6
/// have to ALL decline before branch 7 fires.
#[must_use]
pub fn stale_composite(
    hash_unchanged: bool,
    hash_change_age_secs: u64,
    last_actor_event_age_secs: u64,
    stale_secs: u64,
    is_quiet: bool,
    is_throttled: bool,
) -> bool {
    !is_quiet
        && !is_throttled
        && hash_unchanged
        && hash_change_age_secs >= stale_secs
        && last_actor_event_age_secs >= stale_secs
}

// ---------------------------------------------------------------------------
// Quiet detection — what a quiet state's pane baseline is hashed FROM.

/// The origin envelope the send helper stamps on a watchdog-delivered message
/// — the discriminator that separates a real nudge from an agent QUOTING one,
/// since quoted text renders as prose with no envelope above it. Rendered by
/// the one provenance owner, so the spelling cannot drift from what `send`
/// actually emits.
fn nudge_envelope() -> String {
    crate::provenance::peer(WATCHDOG_ACTOR)
}

/// The nudge's own sentence, for the panes that render it unornamented.
const NUDGE_SENTENCE: &str =
    "Continue the assigned work now. Do not re-plan or ask unless blocked. ";
const NUDGE_SENTENCE_LEGACY: &str = "Status check: if you have more work, continue. \
     Otherwise declare your state so I stop nudging: ";

/// The invitation the nudge ends with, in the current vocabulary.
const NUDGE_TAIL: &str = "/state <waiting-user|waiting-agent|blocked|done> \"<reason>\"";

/// The pre-`waiting-agent` invitation. A pane can still carry the nudge the
/// previous core delivered, so the footprint filter strips BOTH spellings.
const NUDGE_TAIL_LEGACY: &str = "/state <waiting-user|blocked|done> \"<reason>\"";

/// The optional prefix a nudge carries when the session has a goal.
const NUDGE_GOAL_PREFIX: &str = "Session goal: ";

/// The state words a `state` echo can name — the alternation in the awk's
/// `is_echo`, and NOT the quiet set: `working` echoes are footprints too. It
/// is the ONE vocabulary ([`crate::state::VALUES`]), so a state the helper
/// accepts can never be a state the echo filter fails to recognize.
const ECHO_STATES: [&str; 5] = crate::state::VALUES;

/// POSIX `[[:space:]]` in the C locale — the class the awk is written against.
const fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r')
}

fn trim_start_space(line: &str) -> &str {
    line.trim_start_matches(is_space)
}

fn trim_end_space(line: &str) -> &str {
    line.trim_end_matches(is_space)
}

/// A rendered nudge's HEADER: a submit ornament, then the envelope ALONE.
fn submit_hdr(line: &str) -> bool {
    let body = trim_start_space(trim_end_space(line));
    let mut chars = body.chars();
    if !matches!(chars.next(), Some('›' | '❯')) {
        return false;
    }
    let rest = chars.as_str();
    // `[[:space:]]+` — at least one, then the envelope and nothing else.
    rest.starts_with(is_space) && trim_start_space(rest) == nudge_envelope()
}

/// Two leading whitespace characters — the wrapped body of a rendered block.
fn indented(line: &str) -> bool {
    let mut chars = line.chars();
    matches!(chars.next(), Some(c) if is_space(c)) && matches!(chars.next(), Some(c) if is_space(c))
}

/// The nudge as DELIVERED text, with no origin envelope above it.
fn raw_nudge(line: &str) -> bool {
    let body = trim_end_space(line);
    let Some(body) = body
        .strip_suffix(NUDGE_TAIL)
        .or_else(|| body.strip_suffix(NUDGE_TAIL_LEGACY))
    else {
        return false;
    };
    if [NUDGE_SENTENCE, NUDGE_SENTENCE_LEGACY]
        .iter()
        .any(|sentence| body.starts_with(sentence))
    {
        return true;
    }
    // `(Session goal: .*\. )?` — any goal text, ending at a `". "` the sentence
    // then follows.
    let Some(goal) = body.strip_prefix(NUDGE_GOAL_PREFIX) else {
        return false;
    };
    goal.match_indices(". ").any(|(at, sep)| {
        goal.get(at + sep.len()..).is_some_and(|tail| {
            [NUDGE_SENTENCE, NUDGE_SENTENCE_LEGACY]
                .iter()
                .any(|sentence| tail.starts_with(sentence))
        })
    })
}

/// The envelope ALONE on its line — an unmodeled pane's pair form, where the
/// nudge follows on the next line instead of being wrapped under an ornament.
fn raw_env(line: &str) -> bool {
    trim_end_space(line) == nudge_envelope()
}

/// `Marked <agent> <state>` and its optional `:`/`.` remainder — the tail every
/// echo form ends with.
fn echo_tail(rest: &str) -> bool {
    let Some(sep) = rest.find(' ') else {
        return false;
    };
    if sep == 0 {
        return false; // `[^ ]+` needs at least one character
    }
    let Some(after_agent) = rest.get(sep + 1..) else {
        return false;
    };
    ECHO_STATES.iter().any(|state| {
        after_agent
            .strip_prefix(state)
            .is_some_and(|tail| tail.is_empty() || tail.starts_with([':', '.']))
    })
}

/// `HH:MM`, the claude echo's timestamp — `[0-9][0-9]:[0-9][0-9]` exactly.
fn is_hhmm(clock: &str) -> bool {
    let b = clock.as_bytes();
    b.len() == 5
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2] == b':'
        && b[3].is_ascii_digit()
        && b[4].is_ascii_digit()
}

/// The `state` helper's own echo, in the three CAPTURED renderings.
fn is_echo(line: &str) -> bool {
    // codex:   `  └ Marked <agent> <state>: …`
    let boxed = trim_start_space(line);
    if let Some(after_glyph) = boxed.strip_prefix('└')
        && after_glyph.starts_with(is_space)
        && trim_start_space(after_glyph)
            .strip_prefix("Marked ")
            .is_some_and(echo_tail)
    {
        return true;
    }
    // claude:  `⏺ [HH:MM] Done — output: Marked <agent> <state>: …`
    if let Some(after_open) = line.strip_prefix("⏺ [")
        && let Some((clock, tail)) = after_open.split_at_checked(5)
        && is_hhmm(clock)
        && tail
            .strip_prefix("] Done — output: Marked ")
            .is_some_and(echo_tail)
    {
        return true;
    }
    // Unmodeled pane, no TUI: the bare line.
    line.strip_prefix("Marked ").is_some_and(echo_tail)
}

/// The captured buffer split the way awk splits records: on `\n`, with a final
/// trailing newline being a terminator rather than an empty last record.
fn records(buf: &str) -> impl Iterator<Item = &str> {
    let body = buf.strip_suffix('\n').unwrap_or(buf);
    // `printf '%s' ""` feeds awk zero records, not one empty one.
    let records = if buf.is_empty() { None } else { Some(body) };
    records.into_iter().flat_map(|b| b.split('\n'))
}

/// The pane view with the watchdog's own footprints removed.
#[must_use]
pub fn quiet_filter(buf: &str) -> String {
    let mut out = String::new();
    let mut in_block = false;
    let mut held: Option<&str> = None;
    let mut keep = |line: &str| {
        out.push_str(line);
        out.push('\n');
    };
    for line in records(buf) {
        // Inside a rendered nudge block: swallow its indented body.
        if in_block {
            if indented(line) {
                continue;
            }
            in_block = false;
        }
        // A held raw envelope is only dropped when the raw nudge follows it.
        if let Some(envelope) = held.take() {
            if raw_nudge(line) {
                continue;
            }
            keep(envelope);
        }
        if submit_hdr(line) {
            in_block = true; // rendered, both modeled TUIs
            continue;
        }
        if raw_env(line) {
            held = Some(line); // unmodeled pane: pair form
            continue;
        }
        if raw_nudge(line) || is_echo(line) {
            continue; // unmodeled / legacy watchdog, and the state echo
        }
        keep(line);
    }
    if let Some(envelope) = held {
        keep(envelope);
    }
    out
}

/// The baseline hash of a pane, over [`quiet_filter`]'s output.
#[must_use]
pub fn quiet_hash(buf: &str) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for byte in quiet_filter(buf).as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// The actor every watchdog-originated event carries, and the action its nudge
/// carries.
pub const WATCHDOG_ACTOR: &str = "watchdog";
const NUDGE_ACTION: &str = "nudge";
/// The action the orchestrator's fleet-overview prompt carries: its own, so a
/// changed overview is never read as the idle nudge that lapses a `done`.
pub(crate) const SWEEP_NUDGE_ACTION: &str = "sweep-nudge";

/// Whether `event`'s actor IS the seat at `slot` / `agent` in `session` — the
/// ROUTING KEY when the record carries one, the display name ONLY for a
/// keyless legacy record. THE rule the daemon and the read side share: a
/// declaration and the event that proves its currency must be judged by the
/// same key, or a rename-back history lets one incarnation's event stand in
/// for another's.
#[must_use]
pub fn event_is_actor(event: &Event, session: &str, slot: &str, agent: &str) -> bool {
    match (&event.actor_slot, &event.actor_session) {
        (RoutingMember::Value(event_slot), RoutingMember::Value(event_session)) => {
            event_slot == slot && event_session == session
        }
        // No routing key at all: the display name is all there is.
        (RoutingMember::Absent, RoutingMember::Absent) => event.actor == agent,
        // Partial, or present-and-empty: routed, to nobody nameable.
        _ => false,
    }
}

/// Whether `event` is ADDRESSED TO that seat — the mirror of
/// [`event_is_actor`] on the target side, same routing-key rule.
#[must_use]
pub fn event_is_addressed_to(event: &Event, session: &str, slot: &str, agent: &str) -> bool {
    match event.target_identity() {
        Some(Identity::Routed {
            slot: event_slot,
            session: event_session,
        }) => event_slot == slot && event_session == session,
        Some(Identity::Display(name)) => {
            name == agent || is_cross_session_form(name, session, agent)
        }
        // Half a routing key addresses nobody, and neither does no target.
        Some(Identity::Unassociated) | None => false,
    }
}

/// Whether `name` is the `@<session>:<agent>` spelling of THIS session's agent.
fn is_cross_session_form(name: &str, session: &str, agent: &str) -> bool {
    name.strip_prefix('@')
        .and_then(|rest| rest.strip_prefix(session))
        .and_then(|rest| rest.strip_prefix(':'))
        .is_some_and(|rest| rest == agent)
}

/// One relevant record TOGETHER WITH the verdict the routing owner already
/// made about it.
///
/// The verdict travels with the record so no consumer re-derives ownership
/// with a weaker rule: `is_own` is [`event_is_actor`]'s answer, computed ONCE
/// here. A rename-style STALE DISPLAY with correct routing is the case that
/// kills a re-derivation — the owner says own, a display check says inbound,
/// and two consumers would split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "selection facts are consumed independently by distinct declaration-state arms"
)]
pub struct Relevant<'a> {
    /// The selected record.
    pub event: &'a Event,
    /// [`event_is_actor`]'s answer: this record's ACTOR is the seat, by its
    /// routing key when one is present, by display only for a keyless legacy
    /// record.
    pub is_own: bool,
    /// Whether the walk stepped past the watchdog's own nudges to reach it.
    pub looked_past_nudge: bool,
    /// Whether the walk stepped past a delivered done challenge.
    pub looked_past_done_challenge: bool,
    /// Whether the walk stepped past a delivered wait challenge. Ignored for
    /// a MATCHING wait declaration (the challenge is part of its episode) but
    /// ends `waiting-user` currency, like every other crossed challenge.
    pub looked_past_wait_challenge: bool,
    /// Whether the walk stepped past an abandoned watchdog delivery.
    pub looked_past_abandoned: bool,
}

pub const DEFAULT_DONE_CONFIRMATIONS: u8 = 2;
pub(crate) const DONE_CHALLENGE_ACTION: &str = "done-challenge";
/// The watchdog's proof challenge for a `waiting-agent`/`blocked` declaration.
/// A NEW wire action (not `done-challenge` reused): the two folds match
/// challenges by action, and sharing one would let a wait challenge arm a done
/// confirmation. Both wait states share it, so the summary names the state.
pub(crate) const WAIT_CHALLENGE_ACTION: &str = "wait-challenge";

/// Which proof challenge a watchdog record carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Challenge {
    /// A `done-challenge`.
    Done,
    /// A `wait-challenge` for this declared wait state.
    Wait(WaitState),
}

/// A challenge record's summary: `confirmation n/m`, behind the wait state's
/// word for a wait challenge. [`challenge_named`] is the one reader.
#[must_use]
pub(crate) fn challenge_summary(challenge: Challenge, number: u8, required: u8) -> String {
    match challenge {
        Challenge::Done => format!("confirmation {number}/{required}"),
        Challenge::Wait(state) => format!("{} confirmation {number}/{required}", state.as_str()),
    }
}

/// The challenge a record's summary carries, or `None`: the ONE grammar. The
/// challenge summary is the whole LAST `; ` clause — a delivery-abandoned
/// record puts it after its refusal reason — optionally behind the
/// `[unconfirmed] ` head, and a refused-pre-paste tail
/// ([`crate::send::REFUSED_PRE_PASTE`]) is cut off first. Exact: the digits
/// are digits, and nothing may sit around the challenge in its clause.
#[must_use]
pub fn challenge_named(summary: &str) -> Option<Challenge> {
    let body = summary
        .split_once(crate::send::REFUSED_PRE_PASTE)
        .map_or(summary, |(body, _)| body);
    let clause = body.rsplit_once("; ").map_or(body, |(_, clause)| clause);
    let unconfirmed = crate::tracked::unconfirmed_summary("");
    let clause = clause.strip_prefix(unconfirmed.as_str()).unwrap_or(clause);
    let (challenge, count) = if let Some(count) = clause.strip_prefix("confirmation ") {
        (Challenge::Done, count)
    } else {
        let (word, count) = clause.split_once(" confirmation ")?;
        let state = [WaitState::WaitingAgent, WaitState::Blocked]
            .into_iter()
            .find(|state| state.as_str() == word)?;
        (Challenge::Wait(state), count)
    };
    let (number, required) = count.split_once('/')?;
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    (digits(number) && digits(required)).then_some(challenge)
}

/// Whether a `wait-challenge` record, or its abandonment, names `state`.
fn wait_challenge_names(summary: Option<&str>, state: WaitState) -> bool {
    summary.and_then(challenge_named) == Some(Challenge::Wait(state))
}

/// The actor prefixes of the chat bridges: a delivery carrying one is a human
/// writing to the seat from outside the terminal.
const HUMAN_BRIDGE_ACTORS: [&str; 2] = ["telegram:", "discord:"];

/// Whether `event`, relevant to the seat and NEWER than its `state`
/// declaration, ends that declaration — the ONE table the currency walk and
/// both episode folds read, so the daemon and `ae list` cannot split.
///
/// - the watchdog's own records never end one: it is neither the seat nor the
///   human, and its challenge footprints are judged by the `looked_past_*`
///   flags instead;
/// - the seat's own newer declaration always does;
/// - a human through a chat bridge always does;
/// - `waiting-user` and `blocked` hold through everything else — the seat's
///   own chasing, a peer's message, pane churn;
/// - `waiting-agent` also ends on a reply to one of the seat's OWN asks or
///   reviews (`own_requests`, their request ids) — the answer it waits for;
/// - `done`, and every state that quiets nothing, end on any other record.
#[must_use]
pub fn ends_quiet(state: &str, event: &Event, is_own: bool, own_requests: &[&str]) -> bool {
    if event.actor == WATCHDOG_ACTOR {
        return false;
    }
    if is_own && event.declared_state().is_some() {
        return true;
    }
    if !is_own
        && HUMAN_BRIDGE_ACTORS
            .iter()
            .any(|prefix| event.actor.starts_with(prefix))
    {
        return true;
    }
    match state {
        "waiting-user" | "blocked" => false,
        "waiting-agent" => {
            !is_own
                && event.action == crate::reply::ACTION
                && event
                    .reference
                    .as_deref()
                    .is_some_and(|id| own_requests.contains(&id))
        }
        _ => true,
    }
}

/// Whether `event` is one of the watchdog's challenges. A fold judges its own
/// kind above and a CROSSED one resets the episode (#144), so a fold asks
/// this before [`ends_quiet`], which leaves every watchdog record standing.
fn is_challenge(event: &Event) -> bool {
    event.actor == WATCHDOG_ACTOR
        && (event.action == DONE_CHALLENGE_ACTION || event.action == WAIT_CHALLENGE_ACTION)
}

/// Whether `event` is the seat's own ask or review, whose request id a reply
/// answers.
fn is_own_request(event: &Event, is_own: bool) -> bool {
    is_own
        && (event.action == crate::tracked::Kind::Ask.action()
            || event.action == crate::tracked::Kind::Review.action())
}

/// The newest event relevant to the seat at `slot`/`agent` in `session`, with
/// the ownership verdict and whether the walk stepped past any of the
/// watchdog's own nudges to reach it — the SELECTION half of the quiet
/// decision that [`quiet_reason`] then classifies, and the read side's
/// currency proof.
///
/// The walk is STATE-AWARE: it steps past every record newer than the seat's
/// own newest declaration that [`ends_quiet`] says leaves that declaration
/// standing, so what it returns is the declaration itself or the record that
/// ended it. The watchdog's own records are never returned.
///
/// ONE owner: the daemon and `session::agent_entries` both call this, with the
/// same routing key the declaration itself is matched by. The verdict is part
/// of the return value, so it is the ONLY ownership derivation a consumer can
/// use.
#[must_use]
pub fn latest_relevant_event<'a>(
    events: &'a [Event],
    session: &str,
    slot: &str,
    agent: &str,
) -> Option<Relevant<'a>> {
    let declared = events
        .iter()
        .rev()
        .find(|event| {
            event.declared_state().is_some() && event_is_actor(event, session, slot, agent)
        })
        .and_then(Event::declared_state);
    let own_requests: Vec<&str> = if declared == Some("waiting-agent") {
        events
            .iter()
            .filter(|event| is_own_request(event, event_is_actor(event, session, slot, agent)))
            .filter_map(|event| event.reference.as_deref())
            .collect()
    } else {
        Vec::new()
    };
    let mut looked_past_nudge = false;
    let mut looked_past_done_challenge = false;
    let mut looked_past_wait_challenge = false;
    let mut looked_past_abandoned = false;
    for event in events.iter().rev() {
        let is_own = event_is_actor(event, session, slot, agent);
        if !is_own && !event_is_addressed_to(event, session, slot, agent) {
            continue;
        }
        if event.actor == WATCHDOG_ACTOR {
            match event.action.as_str() {
                NUDGE_ACTION => looked_past_nudge = true,
                DONE_CHALLENGE_ACTION => looked_past_done_challenge = true,
                WAIT_CHALLENGE_ACTION => looked_past_wait_challenge = true,
                // Only an abandoned CHALLENGE is a footprint; an abandoned
                // nudge or quota ask is the watchdog's own traffic.
                crate::tracked::ABANDONED_ACTION
                    if event.summary.as_deref().and_then(challenge_named).is_some() =>
                {
                    looked_past_abandoned = true;
                }
                _ => {}
            }
            continue;
        }
        if declared.is_some_and(|state| !ends_quiet(state, event, is_own, &own_requests)) {
            continue;
        }
        return Some(Relevant {
            event,
            is_own,
            looked_past_nudge,
            looked_past_done_challenge,
            looked_past_wait_challenge,
            looked_past_abandoned,
        });
    }
    None
}

/// A self-declared state that tells the watchdog to stop nudging, in its four
/// answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuietKind {
    /// `done` — complete or paused.
    Done,
    /// `waiting-user` — needs human input.
    WaitingUser,
    /// `waiting-agent` — waiting on another ae agent, while fresh.
    WaitingAgent,
    /// `blocked` — stuck on an external dependency, or a `waiting-agent`
    /// past its ceiling.
    Blocked,
}

/// The quiet state a seat's LATEST RELEVANT event declares, or `None`.
///
/// Ownership is the VERDICT's ([`Relevant::is_own`]) — this function never
/// compares the event's actor to a name again, because that display check is
/// exactly the weaker rule a routing-aware owner exists to replace: a
/// rename-style stale display with correct routing would be condemned as
/// "inbound news" here while the read side calls it the seat's own
/// declaration.
#[must_use]
pub fn quiet_reason(relevant: &Relevant<'_>) -> Option<QuietKind> {
    if !declaration_current(relevant) {
        return None;
    }
    // `declared_state` already folds a bare `action = done` record into `done`.
    let kind = match relevant.event.declared_state()? {
        "done" => QuietKind::Done,
        "waiting-user" => QuietKind::WaitingUser,
        "waiting-agent" => QuietKind::WaitingAgent,
        "blocked" => QuietKind::Blocked,
        _ => return None, // `working`, or a ref that declares no state
    };
    Some(kind)
}

/// Whether the selected record is the seat's still-current declaration.
#[must_use]
pub fn declaration_current(relevant: &Relevant<'_>) -> bool {
    if !relevant.is_own {
        return false;
    }
    let Some(state) = relevant.event.declared_state() else {
        return false;
    };
    // Each state's OWN challenge footprints — delivered or abandoned — are
    // part of its episode and ignored; a CROSSED or foreign footprint ends
    // currency, so the read side can never print a plain confirmed state over
    // a challenge the fold already reset.
    if state == "done" {
        !relevant.looked_past_nudge && !relevant.looked_past_wait_challenge
    } else if state == "waiting-agent" || state == "blocked" {
        !relevant.looked_past_done_challenge
    } else {
        !relevant.looked_past_done_challenge
            && !relevant.looked_past_wait_challenge
            && !relevant.looked_past_abandoned
    }
}

/// The watchdog's newest delivery to the seat that may have reached its pane —
/// an idle or sweep nudge, a challenge, a quota advisory or checkpoint ask,
/// unconfirmed ones included because they may have landed. A challenge
/// refused before its paste painted nothing, so it is not one.
#[must_use]
pub fn last_watchdog_delivery(
    events: &[Event],
    session: &str,
    slot: &str,
    agent: &str,
) -> Option<crate::time::Timestamp> {
    let painting = [
        NUDGE_ACTION,
        SWEEP_NUDGE_ACTION,
        DONE_CHALLENGE_ACTION,
        WAIT_CHALLENGE_ACTION,
        crate::quota::action::ADVISORY,
        crate::quota::action::CHECKPOINT,
    ];
    let refused = |event: &Event| {
        event
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains(crate::send::REFUSED_PRE_PASTE))
    };
    events
        .iter()
        .rev()
        .find(|event| {
            event.actor == WATCHDOG_ACTOR
                && painting.contains(&event.action.as_str())
                && event_is_addressed_to(event, session, slot, agent)
                && !refused(event)
        })
        .map(|event| event.ts)
}

/// Whether the human's own input in the seat's pane ends its wait: a client
/// viewing the pane gave input (`activity`, epoch seconds) STRICTLY after both
/// the declaration and the watchdog's newest delivery there. The delivery
/// bound caps what a stray input costs at one nudge: the nudge it lets through
/// re-arms the hold.
#[must_use]
pub fn human_input_ends_wait(
    activity: Option<u64>,
    declared: crate::time::Timestamp,
    last_delivery: Option<crate::time::Timestamp>,
) -> bool {
    let bound = last_delivery.map_or(declared.epoch(), |at| at.epoch().max(declared.epoch()));
    activity
        .and_then(|epoch| i64::try_from(epoch).ok())
        .is_some_and(|epoch| epoch > bound)
}

/// Journal-derived progress for the current seat incarnation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoneProgress {
    None,
    Provisional {
        confirmations: u8,
        required: u8,
    },
    ChallengeDue {
        confirmations: u8,
        required: u8,
        done_age_secs: u64,
        /// Failed challenge deliveries reconstructed from this episode's
        /// journal, so a daemon restart cannot reopen the delivery budget.
        attempts: u32,
    },
    Challenged {
        confirmations: u8,
        required: u8,
    },
    Lapsed {
        confirmations: u8,
        required: u8,
    },
    Confirmed,
}

/// Fold existing parsed records. Without a launch witness, only ref-less
/// challenges count; lifecycle/inbound boundaries still prevent inheritance.
#[must_use]
#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "one forward fold keeps the episode, its challenges and what ends it in one pass"
)]
pub fn done_progress(
    events: &[Event],
    session: &str,
    slot: &str,
    agent: &str,
    launch: Option<&str>,
    now: crate::time::Timestamp,
    cadence: u64,
    required: u8,
) -> DoneProgress {
    let mut episode: Option<(
        u8,
        crate::time::Timestamp,
        Option<crate::time::Timestamp>,
        u32,
    )> = None;
    let mut last_done: Option<(crate::time::Timestamp, Option<&str>)> = None;
    for event in events {
        let own = event_is_actor(event, session, slot, agent);
        let addressed = event_is_addressed_to(event, session, slot, agent);
        if !own && !addressed {
            continue;
        }
        if event.action == DONE_CHALLENGE_ACTION && event.actor == WATCHDOG_ACTOR {
            // Every failed push is durable with this launch reference: an
            // unconfirmed done-challenge event (a refused-pre-paste one
            // included) or a delivery-abandoned event, so attempts bound the
            // pushes.
            let matching = launch.map_or(event.reference.is_none(), |id| {
                event.reference.as_deref() == Some(id)
            });
            if !matching {
                episode = None;
                last_done = None;
                continue;
            }
            if crate::tracked::summary_is_unconfirmed(event.summary.as_deref()) {
                if let Some((_, _, _, attempts)) = episode.as_mut() {
                    *attempts = attempts.saturating_add(1);
                }
            } else if let Some((_, _, outstanding, _)) = episode.as_mut()
                && outstanding.is_none()
            {
                *outstanding = Some(event.ts);
            }
            continue;
        }
        if event.actor == WATCHDOG_ACTOR && event.action == crate::tracked::ABANDONED_ACTION {
            let matching = launch.map_or(event.reference.is_none(), |id| {
                event.reference.as_deref() == Some(id)
            });
            if matching {
                // With no launch witness, a ref-less abandoned ordinary nudge
                // also counts. That degraded mode suppresses challenges sooner,
                // the conservative direction, and still raises the alert.
                if let Some((_, _, _, attempts)) = episode.as_mut() {
                    *attempts = attempts.saturating_add(1);
                }
            }
            continue;
        }
        if event.actor == WATCHDOG_ACTOR && event.action == NUDGE_ACTION {
            continue;
        }
        if own && event.declared_state() == Some("done") {
            let signature = (event.ts, event.summary.as_deref());
            if last_done == Some(signature) {
                continue;
            }
            last_done = Some(signature);
            match episode.as_mut() {
                Some((count, done_at, outstanding @ Some(_), _)) => {
                    *count = count.saturating_add(1);
                    *done_at = event.ts;
                    *outstanding = None;
                }
                None => episode = Some((0, event.ts, None, 0)),
                _ => {}
            }
            continue;
        }
        if !is_challenge(event) && !ends_quiet("done", event, own, &[]) {
            continue;
        }
        episode = None;
        last_done = None;
    }
    let Some((confirmations, done_at, outstanding, attempts)) = episode else {
        return DoneProgress::None;
    };
    if required == 0 || cadence == 0 || confirmations >= required {
        return DoneProgress::Confirmed;
    }
    if let Some(challenged_at) = outstanding {
        return if challenged_at.seconds_until(now).max(0).cast_unsigned() >= cadence {
            DoneProgress::Lapsed {
                confirmations,
                required,
            }
        } else {
            DoneProgress::Challenged {
                confirmations,
                required,
            }
        };
    }
    let age = done_at.seconds_until(now).max(0).cast_unsigned();
    if age >= cadence {
        DoneProgress::ChallengeDue {
            confirmations,
            required,
            done_age_secs: age,
            attempts,
        }
    } else {
        DoneProgress::Provisional {
            confirmations,
            required,
        }
    }
}

/// Which wait state a [`wait_progress`] episode tracks. STRICTLY one: a
/// `waiting-agent` proof confirms only a `waiting-agent` episode, a `blocked`
/// proof only a `blocked` one, and a declaration of the other state supersedes
/// into a fresh episode of its own. Escalation never crosses this line — it
/// changes the effective verdict, never the raw declaration's identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitState {
    WaitingAgent,
    Blocked,
}

impl WaitState {
    /// The declared-state word this episode tracks.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WaitingAgent => "waiting-agent",
            Self::Blocked => "blocked",
        }
    }
}

/// Journal-derived proof progress for a `waiting-agent`/`blocked` declaration.
///
/// [`done_progress`]'s shape MINUS `Confirmed`: a wait is never terminally
/// proven — the Nth proof re-arms a fresh episode, so a stale self-declared
/// wait is challenged again rather than honoured forever. Disabled knobs
/// (`required == 0` or `cadence == 0`) yield `None`: quiet as before, through
/// the pane baseline alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitProgress {
    /// No episode: no declaration, or superseded news.
    None,
    /// Declared, challenge not yet due.
    Provisional { confirmations: u8, required: u8 },
    /// Declared past the cadence with no outstanding challenge.
    ChallengeDue {
        confirmations: u8,
        required: u8,
        wait_age_secs: u64,
        /// Failed challenge deliveries reconstructed from this episode's
        /// journal, so a daemon restart cannot reopen the delivery budget.
        attempts: u32,
    },
    /// A delivered challenge awaits proof.
    Challenged { confirmations: u8, required: u8 },
    /// The outstanding challenge went unanswered past a further cadence. The
    /// seat KEEPS its quiet verdict — unlike a lapsed done — while the
    /// ordinary nudge budget resumes beside it, so the daemon and `ae list`
    /// never split on classification.
    Lapsed { confirmations: u8, required: u8 },
}

/// Fold the wait episode for `state` over the existing parsed records.
///
/// The contract, beside [`done_progress`]: every appended same-state
/// declaration line counts by APPEND ORDER — no `(ts, summary)` dedup, which
/// exists only for done's dual legacy emit — and a re-declaration with no
/// outstanding challenge refreshes the anchor without credit, while a
/// declaration of the other wait state, `working`, `waiting-user` or `done`,
/// and any other record [`ends_quiet`] says ends the wait, supersede the
/// episode. Watchdog nudge footprints and
/// matching abandoned-challenge footprints are SKIPPED (the abandoned ones
/// still count as attempts): a lapsed challenge's own nudge must not kill the
/// re-ask it belongs to. A `done-challenge` record resets the episode, and a
/// `wait-challenge` record resets a done episode — the safe direction for a
/// pairing no consistent journal produces.
#[must_use]
#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "one forward fold keeps the episode, its challenges and what ends it in one pass"
)]
pub fn wait_progress(
    events: &[Event],
    session: &str,
    slot: &str,
    agent: &str,
    launch: Option<&str>,
    now: crate::time::Timestamp,
    cadence: u64,
    required: u8,
    state: WaitState,
) -> WaitProgress {
    if required == 0 || cadence == 0 {
        return WaitProgress::None;
    }
    let want = state.as_str();
    let mut episode: Option<(
        u8,
        crate::time::Timestamp,
        Option<crate::time::Timestamp>,
        u32,
    )> = None;
    let mut own_requests: Vec<&str> = Vec::new();
    for event in events {
        let own = event_is_actor(event, session, slot, agent);
        let addressed = event_is_addressed_to(event, session, slot, agent);
        if !own && !addressed {
            continue;
        }
        if is_own_request(event, own)
            && let Some(id) = event.reference.as_deref()
        {
            own_requests.push(id);
        }
        if event.action == WAIT_CHALLENGE_ACTION && event.actor == WATCHDOG_ACTOR {
            // Same journal bound as `done_progress`: every failed push is an
            // unconfirmed record here or its abandonment below.
            if !wait_challenge_names(event.summary.as_deref(), state) {
                continue; // another state's challenge: not ours, not news
            }
            let matching = launch.map_or(event.reference.is_none(), |id| {
                event.reference.as_deref() == Some(id)
            });
            if !matching {
                episode = None;
                continue;
            }
            if crate::tracked::summary_is_unconfirmed(event.summary.as_deref()) {
                if let Some((_, _, _, attempts)) = episode.as_mut() {
                    *attempts = attempts.saturating_add(1);
                }
            } else if let Some((_, _, outstanding, _)) = episode.as_mut()
                && outstanding.is_none()
            {
                *outstanding = Some(event.ts);
            }
            continue;
        }
        if event.actor == WATCHDOG_ACTOR && event.action == crate::tracked::ABANDONED_ACTION {
            let matching = launch.map_or(event.reference.is_none(), |id| {
                event.reference.as_deref() == Some(id)
            }) && wait_challenge_names(event.summary.as_deref(), state);
            if matching && let Some((_, _, _, attempts)) = episode.as_mut() {
                *attempts = attempts.saturating_add(1);
            }
            continue;
        }
        if event.actor == WATCHDOG_ACTOR && event.action == NUDGE_ACTION {
            continue;
        }
        if own && event.declared_state() == Some(want) {
            let rearms = matches!(episode, Some((count, _, Some(_), _))
                if count.saturating_add(1) >= required);
            if rearms {
                // The Nth proof re-arms a fresh episode anchored at this
                // proof: waits are never terminally proven.
                episode = Some((0, event.ts, None, 0));
            } else {
                match episode.as_mut() {
                    Some((count, wait_at, outstanding @ Some(_), _)) => {
                        *count = count.saturating_add(1);
                        *wait_at = event.ts;
                        *outstanding = None;
                    }
                    None => episode = Some((0, event.ts, None, 0)),
                    // A proactive re-declaration refreshes the anchor only.
                    Some((_, wait_at, None, _)) => *wait_at = event.ts,
                }
            }
            continue;
        }
        if !is_challenge(event) && !ends_quiet(want, event, own, &own_requests) {
            continue;
        }
        episode = None;
    }
    let Some((confirmations, wait_at, outstanding, attempts)) = episode else {
        return WaitProgress::None;
    };
    if let Some(challenged_at) = outstanding {
        return if challenged_at.seconds_until(now).max(0).cast_unsigned() >= cadence {
            WaitProgress::Lapsed {
                confirmations,
                required,
            }
        } else {
            WaitProgress::Challenged {
                confirmations,
                required,
            }
        };
    }
    let age = wait_at.seconds_until(now).max(0).cast_unsigned();
    if age >= cadence {
        WaitProgress::ChallengeDue {
            confirmations,
            required,
            wait_age_secs: age,
            attempts,
        }
    } else {
        WaitProgress::Provisional {
            confirmations,
            required,
        }
    }
}

/// Seconds of continuously observed idle before the state reminder; zero
/// disables it. THE default: the watchdog's own `Knobs` and every reader of a
/// session with no `idle_nudge_secs` pin resolve it here, so a `waiting-agent`
/// ceiling can never be computed from two different cadences.
pub const DEFAULT_IDLE_NUDGE_SECS: u64 = 300;

/// How many multiples of `idle_nudge_secs` one outstanding item may age before
/// the deferral gives way. Generous on purpose: the waiting seat is not the
/// problem, and the ceiling exists for the seat that is.
///
/// Two consumers, one value: the own-work deferral in `watchdog_daemon`, and
/// the `waiting-agent` escalation below — both describe the case where the
/// seat's own work has itself gone wrong.
pub const OWN_WORK_AGE_CAP: u64 = 4;

/// The age ceiling for a `waiting-agent` declaration: `idle_nudge_secs *
/// OWN_WORK_AGE_CAP`, with ONE deliberate exception stated here so no doc can
/// claim a shared EFFECTIVE cap.
///
/// At `idle_nudge_secs == 0` the ceiling scales from the documented default
/// ([`DEFAULT_IDLE_NUDGE_SECS`]), i.e. 1200s — NOT the own-work deferral's
/// `0 * 4`. The two are governing different things and the divergence is
/// correct: the deferral is VACUOUS when nudging is off (there is no nudge to
/// defer), while the attention MARKER is not — zero suppresses the nudge and
/// keeps the claim, exactly as `waiting-user`/`blocked` keep claiming the
/// human at zero, and an over-age `waiting-agent` must not become a silent
/// stall. Prose that says the escalation uses "the same cap" as the deferral
/// is wrong at zero.
#[must_use]
pub fn waiting_agent_cap_secs(idle_nudge_secs: u64) -> u64 {
    let cadence = if idle_nudge_secs == 0 {
        DEFAULT_IDLE_NUDGE_SECS
    } else {
        idle_nudge_secs
    };
    cadence.saturating_mul(OWN_WORK_AGE_CAP)
}

/// Whether a `waiting-agent` declaration of `age_secs` reads as `blocked`.
///
/// PURE, and the ONE ceiling every consumer shares: the read side
/// (`session::declared_reason`) and the watchdog's own nudge decision both ask
/// this, so a surface can never disagree with the pane about escalation.
#[must_use]
pub fn waiting_agent_escalated(age_secs: u64, idle_nudge_secs: u64) -> bool {
    age_secs >= waiting_agent_cap_secs(idle_nudge_secs)
}

/// The declaration's identity — `action|ts|ref|actor|summary`, so a
/// same-second re-declaration is a new one.
#[must_use]
pub fn declaration_key(event: &Event) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        event.action,
        event.ts,
        event.reference.as_deref().unwrap_or(""),
        event.actor,
        event.summary.as_deref().unwrap_or("")
    )
}

// ---------------------------------------------------------------------------
// The orchestrator (meta-agent) changed-overview cadence.

/// Keep fleet overviews out of a seat that just declared active human work.
/// A finite hold protects the instruction without letting a stuck declaration
/// starve the human forever.
pub const OVERVIEW_HOLD_WHILE_WORKING_SECS: u64 = 600;

/// The orchestrator sweep tunables, with their defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepKnobs {
    /// Minimum seconds between changed-overview prompts.
    pub sweep_secs: u64,
    /// How soon an UNDELIVERED prompt is retried instead of burning a whole
    /// cadence window.
    pub retry_secs: u64,
    /// How many FAST retries are allowed before the branch falls back to the
    /// normal cadence and escalates once.
    pub retry_max: u32,
}

impl Default for SweepKnobs {
    fn default() -> Self {
        Self {
            sweep_secs: 120,
            retry_secs: 30,
            retry_max: 6,
        }
    }
}

impl SweepKnobs {
    /// Whether the sweep branch runs at all.
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.sweep_secs > 0
    }

    /// The overview acknowledgement window: `SWEEP_SECS * 2 + 60`.
    #[must_use]
    pub const fn wedge_secs(&self) -> u64 {
        self.sweep_secs.saturating_mul(2).saturating_add(60)
    }
}

/// Seconds ELAPSED from `then` to `now`, clamped at zero.
fn secs_between(now: SystemTime, then: SystemTime) -> u64 {
    now.duration_since(then).map_or(0, |d| d.as_secs())
}

/// `now` moved `secs` into the past.
fn back_date(now: SystemTime, secs: u64) -> SystemTime {
    now.checked_sub(Duration::from_secs(secs))
        .unwrap_or(UNIX_EPOCH)
}

/// The roster slot the orchestrator cadence belongs to.
pub const MAIN_SLOT: &str = "main";

/// Whether this pane is the one the overview cadence applies to, keyed by SLOT
/// rather than by display name.
#[must_use]
pub fn is_sweep_target(meta_agent: bool, slot: &str) -> bool {
    meta_agent && slot == MAIN_SLOT
}

/// What the orchestrator's row shows this cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepVerdict {
    /// Every delivered overview has a later `state done` acknowledgement.
    MetaSweeping,
    /// A delivered overview passed the grace window without `state done`.
    MetaWedged,
    /// No overview yet, or its acknowledgement is still inside the grace.
    MetaStarting,
}

impl SweepVerdict {
    /// The glyph the roster bar publishes for this verdict.
    #[must_use]
    pub const fn glyph(self) -> &'static str {
        match self {
            Self::MetaSweeping => "👁",
            Self::MetaWedged => "◌",
            Self::MetaStarting => "·",
        }
    }
}

/// Which missing overview acknowledgement the wedge alert is reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WedgeDetail {
    /// The seat has acknowledged before, but not since the oldest outstanding
    /// delivery.
    Stalled {
        /// Seconds since the oldest unacknowledged overview landed.
        age_secs: u64,
    },
    /// No `done` event exists despite one or more delivered overviews.
    Never {
        /// Delivered overviews since this daemon started, at least one.
        deliveries: u32,
    },
}

/// An alert the sweep branch raises or clears, as a TRANSITION rather than as
/// prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepAlert {
    /// Live but not acknowledging delivered overviews.
    RaiseWedge(WedgeDetail),
    /// A `state done` acknowledgement landed.
    ClearWedge,
    /// Sweep prompts stopped landing altogether.
    RaiseUnreachable {
        /// Consecutive undelivered prompts at the moment of escalation.
        undelivered: u32,
    },
    /// A prompt landed again.
    ClearUnreachable,
}

impl SweepAlert {
    /// The event action this transition appends.
    #[must_use]
    pub const fn action(self) -> &'static str {
        match self {
            Self::RaiseWedge(_) | Self::RaiseUnreachable { .. } => "alert",
            Self::ClearWedge | Self::ClearUnreachable => "alert-cleared",
        }
    }

    /// The summary text, from the branch that emits it.
    #[must_use]
    pub fn summary(self) -> String {
        match self {
            Self::RaiseWedge(WedgeDetail::Stalled { age_secs }) => format!(
                "meta-agent not acknowledging overviews — oldest outstanding overview \
                 unacknowledged for {}m (may be stuck)",
                age_secs / 60
            ),
            Self::RaiseWedge(WedgeDetail::Never { deliveries }) => format!(
                "meta-agent not acknowledging overviews — {deliveries} delivered, zero done \
                 events (may be stuck)"
            ),
            Self::ClearWedge => {
                "meta-agent acknowledging overviews again (done received)".to_owned()
            }
            Self::RaiseUnreachable { undelivered } => format!(
                "meta-agent unreachable — {undelivered} sweep nudges undelivered (not sweeping)"
            ),
            Self::ClearUnreachable => {
                "meta-agent reachable again (sweep nudge delivered)".to_owned()
            }
        }
    }

    /// The `display-message` line for the human, or `None` when the transition
    /// is log-only.
    #[must_use]
    pub const fn notify(self) -> Option<&'static str> {
        match self {
            Self::RaiseWedge(_) => Some("(meta-agent) not acknowledging overviews — may be stuck"),
            Self::RaiseUnreachable { .. } => {
                Some("(meta-agent) unreachable — sweep nudges undelivered")
            }
            Self::ClearWedge | Self::ClearUnreachable => None,
        }
    }
}

/// Something the loop must DO for the orchestrator this cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepEffect {
    /// Deliver one sweep prompt through the session's own `send` helper, then
    /// report the outcome to [`record_sweep`].
    FireSweepNudge,
    /// Raise or clear one alert.
    Alert(SweepAlert),
    /// ONCE per daemon lifetime, on the first acknowledged overview with no
    /// latched wedge: read the DURABLE event log and, if it still shows an
    /// active alert for this agent, emit [`SweepAlert::ClearWedge`]'s event.
    ReconcileWedge,
}

/// What the orchestrator pane carries from cycle to cycle, gathered into one
/// value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepState {
    /// When the cadence was last satisfied.
    pub last_sweep: Option<SystemTime>,
    /// When the oldest unacknowledged overview landed — the fixed origin of
    /// the acknowledgement grace until a `done` event clears it.
    pub outstanding_since: Option<SystemTime>,
    /// When the latest overview landed — the acknowledgement boundary. A
    /// `done` event must follow this delivery before it can clear the batch.
    pub last_delivered: Option<SystemTime>,
    /// Successful overview deliveries not yet followed by `state done`.
    pub unacknowledged_deliveries: u32,
    /// Consecutive undelivered prompts.
    pub fails: u32,
    /// The wedge alert is raised once per wedge, not once per cycle.
    pub wedge_alerted: bool,
    /// The unreachable alert is raised once per unreachable run.
    pub unreachable_alerted: bool,
    /// Whether the once-per-lifetime durable reconcile has been offered.
    pub reconciled: bool,
}

/// What the cycle observed about the orchestrator and its durable checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepObservation {
    /// Wall clock for this cycle.
    pub now: SystemTime,
    /// Newest `state done` event by the orchestrator main, if one exists.
    pub last_done: Option<SystemTime>,
    /// When the orchestrator main's current `working` declaration landed.
    pub working_since: Option<SystemTime>,
    /// Whether the newly rendered overview differs from the last one that
    /// landed in the orchestrator pane.
    pub overview_changed: bool,
    /// Oldest unacknowledged delivery recovered from durable state.
    pub persisted_outstanding_since: Option<SystemTime>,
    /// Latest delivered overview recovered for durable minimum spacing.
    pub persisted_last_delivery: Option<SystemTime>,
}

impl SweepObservation {
    /// Start an observation from the seat's newest `state done` event.
    #[must_use]
    pub const fn new(now: SystemTime, last_done: Option<SystemTime>) -> Self {
        Self {
            now,
            last_done,
            working_since: None,
            overview_changed: true,
            persisted_outstanding_since: None,
            persisted_last_delivery: None,
        }
    }

    /// Add the change gate and durable cadence checkpoint used by the fleet
    /// overview. The plain constructor keeps the original always-changed
    /// behaviour for callers that do not render one.
    #[must_use]
    pub const fn with_overview(
        mut self,
        changed: bool,
        persisted_outstanding_since: Option<SystemTime>,
        persisted_last_delivery: Option<SystemTime>,
    ) -> Self {
        self.overview_changed = changed;
        self.persisted_outstanding_since = persisted_outstanding_since;
        self.persisted_last_delivery = persisted_last_delivery;
        self
    }

    /// Add the active-human-work hold recovered from the main seat's newest
    /// declaration. `None` means its current declaration is not `working`.
    #[must_use]
    pub const fn with_working_since(mut self, working_since: Option<SystemTime>) -> Self {
        self.working_since = working_since;
        self
    }
}

/// The result of one sweep cycle's accounting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepAccounting {
    /// The state to carry into the next cycle — or into [`record_sweep`] first,
    /// when the effects contain [`SweepEffect::FireSweepNudge`].
    pub next: SweepState,
    /// What the loop must do, in order.
    pub effects: Vec<SweepEffect>,
    /// The glyph verdict for the status line.
    pub verdict: SweepVerdict,
}

/// Account for the orchestrator main in one cycle, and the only place any of it
/// is decided.
#[must_use]
pub fn sweep_step(
    prior: &SweepState,
    seen: &SweepObservation,
    knobs: &SweepKnobs,
) -> Option<SweepAccounting> {
    if !knobs.enabled() {
        return None; // Not this branch at all.
    }
    let mut next = *prior;
    let mut effects = Vec::new();

    // 1. The verdict. The watchdog's own checkpoint proves only that THIS loop
    //    ran. Seat liveness comes from the `state done` event the charter
    //    requires after every delivered overview. Repeated deliveries retain
    //    the oldest outstanding deadline. The grace comparison is
    //    strict `>`, so an age exactly equal to the window is still starting.
    let outstanding_since = match (prior.outstanding_since, seen.persisted_outstanding_since) {
        (Some(memory), Some(persisted)) => Some(memory.min(persisted)),
        (memory, persisted) => memory.or(persisted),
    };
    next.outstanding_since = outstanding_since;
    let recovered_last_delivered = match (prior.last_delivered, seen.persisted_last_delivery) {
        (Some(memory), Some(persisted)) => Some(memory.max(persisted)),
        (memory, persisted) => memory.or(persisted),
    };
    let last_delivered = match (recovered_last_delivered, outstanding_since) {
        (Some(latest), Some(oldest)) => Some(latest.max(oldest)),
        (latest, oldest) => latest.or(oldest),
    };
    next.last_delivered = last_delivered;
    let acknowledged = outstanding_since.is_some()
        && last_delivered.is_some_and(|delivery| {
            seen.last_done
                .is_some_and(|done| done.duration_since(delivery).is_ok())
        });
    let grace_secs = outstanding_since.map(|at| secs_between(seen.now, at));
    let verdict = match (outstanding_since, acknowledged, grace_secs) {
        (Some(_), true, _) => SweepVerdict::MetaSweeping,
        (Some(_), false, Some(elapsed)) if elapsed > knobs.wedge_secs() => SweepVerdict::MetaWedged,
        _ => SweepVerdict::MetaStarting,
    };
    if acknowledged {
        next.outstanding_since = None;
        next.unacknowledged_deliveries = 0;
    } else if outstanding_since.is_some() && next.unacknowledged_deliveries == 0 {
        // A persisted delivery survived a daemon restart; its in-memory count
        // did not, but the durable timestamp proves there was at least one.
        next.unacknowledged_deliveries = 1;
    }

    // 2.
    match verdict {
        SweepVerdict::MetaSweeping => {
            if prior.wedge_alerted {
                next.wedge_alerted = false;
                next.reconciled = true;
                effects.push(SweepEffect::Alert(SweepAlert::ClearWedge));
            } else if !prior.reconciled {
                next.reconciled = true;
                effects.push(SweepEffect::ReconcileWedge);
            }
        }
        SweepVerdict::MetaWedged => {
            if !prior.wedge_alerted {
                next.wedge_alerted = true;
                let detail = match seen.last_done {
                    Some(_) => WedgeDetail::Stalled {
                        age_secs: grace_secs.unwrap_or(0),
                    },
                    None => WedgeDetail::Never {
                        deliveries: next.unacknowledged_deliveries,
                    },
                };
                effects.push(SweepEffect::Alert(SweepAlert::RaiseWedge(detail)));
            }
        }
        SweepVerdict::MetaStarting => {}
    }

    // 3.
    let last_sweep = match (prior.last_sweep, seen.persisted_last_delivery) {
        (Some(memory), Some(persisted)) => Some(memory.max(persisted)),
        (memory, persisted) => memory.or(persisted),
    };
    let due = last_sweep.is_none_or(|at| secs_between(seen.now, at) >= knobs.sweep_secs);
    let holding_for_work = seen
        .working_since
        .is_some_and(|at| secs_between(seen.now, at) < OVERVIEW_HOLD_WHILE_WORKING_SECS);
    if due && seen.overview_changed && !holding_for_work {
        // A held overview is not an attempted delivery: no booking, hash or
        // spacing clock advances, so the next cycle sees the same change.
        effects.push(SweepEffect::FireSweepNudge);
    }

    Some(SweepAccounting {
        next,
        effects,
        verdict,
    })
}

/// Book a sweep prompt's outcome.
pub fn record_sweep(
    state: &mut SweepState,
    delivered: bool,
    settled_now: SystemTime,
    knobs: &SweepKnobs,
) -> Vec<SweepEffect> {
    if delivered {
        state.fails = 0;
        state.last_sweep = Some(settled_now);
        state.last_delivered = Some(settled_now);
        if state.outstanding_since.is_none() {
            state.outstanding_since = Some(settled_now);
            state.unacknowledged_deliveries = 1;
        } else {
            state.unacknowledged_deliveries = state.unacknowledged_deliveries.saturating_add(1);
        }
        if !state.unreachable_alerted {
            return Vec::new();
        }
        state.unreachable_alerted = false;
        // The latch drops either way; only the EVENT is conditional.
        if state.wedge_alerted {
            return Vec::new();
        }
        return vec![SweepEffect::Alert(SweepAlert::ClearUnreachable)];
    }

    state.fails = state.fails.saturating_add(1);
    if state.fails <= knobs.retry_max {
        // Don't consume the cadence slot — make the retry due `retry` seconds
        // after THIS failure, by back-dating the cadence rather than by
        // carrying a second schedule.
        let hastened_by = knobs.sweep_secs.saturating_sub(knobs.retry_secs);
        state.last_sweep = Some(back_date(settled_now, hastened_by));
        return Vec::new();
    }

    // Bounded.
    state.last_sweep = Some(settled_now);
    if state.unreachable_alerted {
        return Vec::new();
    }
    state.unreachable_alerted = true;
    vec![SweepEffect::Alert(SweepAlert::RaiseUnreachable {
        undelivered: state.fails,
    })]
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::WaitState::{Blocked, WaitingAgent};
    use super::{
        DEFAULT_IDLE_NUDGE_SECS, DoneProgress, OVERVIEW_HOLD_WHILE_WORKING_SECS, OWN_WORK_AGE_CAP,
        QuietKind, SweepAlert, SweepEffect, SweepKnobs, SweepObservation, SweepState, SweepVerdict,
        Throttle, WaitProgress, WaitState, WedgeDetail, classify_dead, command_is_shell,
        declaration_current, declaration_key, done_progress, indented, is_echo, is_sweep_target,
        latest_relevant_event, quiet_filter, quiet_hash, quiet_reason, raw_nudge, record_sweep,
        shows_throttle, stale_composite, submit_hdr, sweep_step, throttle_class, wait_progress,
        waiting_agent_cap_secs, waiting_agent_escalated,
    };
    use crate::events::Event;
    use crate::procs::Descendancy;
    use crate::time::Timestamp;

    /// Build an event through the TYPED reader, so these tests exercise the same
    /// parse the daemon will.
    fn event(line: &str) -> Event {
        Event::parse_line(line).expect("the specimen is a well-formed event")
    }

    #[test]
    fn the_shell_set_is_exactly_the_bash_command_is_shell_case() {
        for shell in ["bash", "zsh", "fish", "sh", "dash", ""] {
            assert!(command_is_shell(shell), "{shell:?} is a shell");
        }
        for other in [
            "claude",
            "codex",
            "opencode.exe",
            "python",
            "node",
            "bashx",
            "ssh",
        ] {
            assert!(!command_is_shell(other), "{other:?} is not a shell");
        }
    }

    #[test]
    fn dead_requires_a_shell_foreground_and_a_proven_absent_agent() {
        // The whole point of the two-part guard: a shell wrapper with the agent
        // running underneath is ALIVE, not dead.
        assert!(
            classify_dead("bash", Descendancy::Absent),
            "shell + a good snapshot showing nothing under it = dead"
        );
        assert!(
            !classify_dead("bash", Descendancy::Present),
            "shell BUT agent underneath = alive"
        );
        assert!(
            !classify_dead("claude", Descendancy::Absent),
            "a real agent foreground = alive"
        );
        assert!(
            !classify_dead("claude", Descendancy::Present),
            "agent foreground and a descendant = alive"
        );
        assert!(
            classify_dead("", Descendancy::Absent),
            "an empty foreground with nothing under it = dead"
        );
    }

    #[test]
    fn an_unusable_snapshot_never_classifies_an_agent_dead() {
        // An unusable ps snapshot is not evidence of absence: a probe that
        // FAILED must never read the same as one that ran and found nothing.
        for foreground in ["bash", "zsh", "fish", "sh", "dash", "", "claude", "codex"] {
            assert!(
                !classify_dead(foreground, Descendancy::Unknown),
                "{foreground:?} + an unusable ps snapshot must not read as dead"
            );
        }
    }

    #[test]
    fn stale_is_what_is_left_when_every_earlier_branch_declines() {
        // Not quiet, not throttled, pane unchanged, and both ages past the window.
        assert!(stale_composite(true, 900, 900, 900, false, false));
        assert!(stale_composite(true, 4000, 3600, 900, false, false));
    }

    #[test]
    fn the_stale_boundary_is_bash_strict_less_than() {
        // `age < STALE_SECS` skips, so equality falls through and IS stale.
        assert!(
            stale_composite(true, 900, 900, 900, false, false),
            "age == stale_secs is stale"
        );
        assert!(
            !stale_composite(true, 899, 900, 900, false, false),
            "a recently visible pane (branch 5) is not stale"
        );
        assert!(
            !stale_composite(true, 900, 899, 900, false, false),
            "recent ae activity (branch 6) is not stale"
        );
    }

    #[test]
    fn quiet_throttled_or_a_moving_pane_each_force_not_stale() {
        assert!(
            !stale_composite(true, 900, 900, 900, true, false),
            "a held quiet state was already skipped at branch 2"
        );
        assert!(
            !stale_composite(true, 900, 900, 900, false, true),
            "throttling is upstream's fault — branch 3"
        );
        assert!(
            !stale_composite(false, 900, 900, 900, false, false),
            "a changed pane hash is activity — branch 4"
        );
    }

    #[test]
    fn only_the_agents_own_declaration_is_a_quiet_state() {
        let agent = "opus5:builder";
        // The owner selects and carries the verdict; the classifier consumes it.
        let classify = |line: &str| {
            let events = log(&[line]);
            let found = latest_relevant_event(&events, "aerewrite", "main", agent)?;
            assert!(found.is_own, "{line:?} is the seat's own record");
            quiet_reason(&found)
        };
        assert_eq!(
            classify(
                r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"done","summary":"shipped"}"#
            ),
            Some(QuietKind::Done)
        );
        assert_eq!(
            classify(
                r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-user","summary":"needs review"}"#
            ),
            Some(QuietKind::WaitingUser)
        );
        assert_eq!(
            classify(
                r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-agent","summary":"waiting on colead's re-review"}"#
            ),
            Some(QuietKind::WaitingAgent),
            "the fifth state is quiet while fresh (R2)"
        );
        assert_eq!(
            classify(
                r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"blocked","summary":"waiting on CI"}"#
            ),
            Some(QuietKind::Blocked)
        );
        // A bare `action = done` record with no ref maps to Done.
        assert_eq!(
            classify(r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"done"}"#),
            Some(QuietKind::Done)
        );
    }

    #[test]
    fn news_from_anyone_else_ends_a_quiet_state() {
        let agent = "opus5:builder";
        // An inbound message TARGETING the agent is the newest relevant event,
        // the owner selects it with `is_own: false`, and the classifier yields.
        let inbound = r#"{"ts":"2026-08-29T04:01:00Z","actor":"fable5:lead","action":"send","target":"opus5:builder","summary":"review please"}"#;
        let events = log(&[inbound]);
        let found = latest_relevant_event(&events, "aerewrite", "main", agent)
            .expect("an addressed record is relevant");
        assert!(!found.is_own, "its actor is somebody else");
        assert_eq!(quiet_reason(&found), None);
        // Even an inbound event that would otherwise LOOK like a declaration.
        let inbound_state = r#"{"ts":"2026-08-29T04:01:00Z","actor":"fable5:lead","action":"state","ref":"done","target":"opus5:builder"}"#;
        let events = log(&[inbound_state]);
        let found = latest_relevant_event(&events, "aerewrite", "main", agent)
            .expect("addressed state record");
        assert!(!found.is_own);
        assert_eq!(quiet_reason(&found), None);
    }

    #[test]
    fn working_and_a_refless_state_declare_no_quiet_state() {
        let agent = "opus5:builder";
        let classify = |line: &str| {
            let events = log(&[line]);
            let found = latest_relevant_event(&events, "aerewrite", "main", agent)?;
            quiet_reason(&found)
        };
        assert_eq!(
            classify(
                r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"working","summary":"on it"}"#
            ),
            None
        );
        assert_eq!(
            classify(r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state"}"#),
            None,
            "declared nothing"
        );
        assert_eq!(
            classify(
                r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"memo","ref":"arch"}"#
            ),
            None
        );
    }

    #[test]
    fn a_nudge_walked_past_clears_done_and_nothing_else() {
        let agent = "opus5:builder";
        let done = r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"done","summary":"shipped"}"#;
        let nudge = r#"{"ts":"2026-08-29T04:00:01Z","actor":"watchdog","action":"nudge","target":"opus5:builder"}"#;

        // Without a nudge, done is a quiet hold.
        let events = log(&[done]);
        let found = latest_relevant_event(&events, "aerewrite", "main", agent)
            .expect("the declaration is relevant");
        assert_eq!(quiet_reason(&found), Some(QuietKind::Done));

        // With one, the walk steps past it and the verdict carries that fact.
        let classify = |line: &str| {
            let events = log(&[line, nudge]);
            let found = latest_relevant_event(&events, "aerewrite", "main", agent)
                .expect("the declaration is under the nudge");
            assert!(found.is_own && found.looked_past_nudge);
            quiet_reason(&found)
        };
        assert_eq!(
            classify(done),
            None,
            "done is honoured until a newer MESSAGE arrives, and a nudge is one"
        );
        // The look-past is scoped to the states it exists for: they are pane
        // holds, and a nudge must not break them.
        for (line, kind) in [
            (
                r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-user"}"#,
                QuietKind::WaitingUser,
            ),
            (
                r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-agent"}"#,
                QuietKind::WaitingAgent,
            ),
            (
                r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"blocked","summary":"dep"}"#,
                QuietKind::Blocked,
            ),
        ] {
            assert_eq!(classify(line), Some(kind));
        }
    }

    fn progress(lines: &[&str], now: &str, required: u8, cadence: u64) -> DoneProgress {
        done_progress(
            &log(lines),
            "aerewrite",
            "main",
            "opus5:builder",
            Some("launch-1"),
            Timestamp::parse(now).expect("test time"),
            cadence,
            required,
        )
    }

    const DONE_0: &str = r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"done","summary":"complete","actor_slot":"main","actor_session":"aerewrite"}"#;
    const CHALLENGE_1: &str = r#"{"ts":"2026-08-29T04:01:00Z","actor":"watchdog","action":"done-challenge","target":"opus5:builder","ref":"launch-1","target_slot":"main","target_session":"aerewrite"}"#;
    const DONE_1: &str = r#"{"ts":"2026-08-29T04:01:30Z","actor":"opus5:builder","action":"state","ref":"done","summary":"proof one","actor_slot":"main","actor_session":"aerewrite"}"#;
    const CHALLENGE_2: &str = r#"{"ts":"2026-08-29T04:02:30Z","actor":"watchdog","action":"done-challenge","target":"opus5:builder","ref":"launch-1","target_slot":"main","target_session":"aerewrite"}"#;
    const DONE_2: &str = r#"{"ts":"2026-08-29T04:03:00Z","actor":"opus5:builder","action":"state","ref":"done","summary":"proof two","actor_slot":"main","actor_session":"aerewrite"}"#;

    #[test]
    fn done_is_challenged_twice_then_confirmed_across_restart() {
        assert_eq!(
            progress(&[DONE_0], "2026-08-29T04:00:59Z", 2, 60),
            DoneProgress::Provisional {
                confirmations: 0,
                required: 2,
            }
        );
        assert_eq!(
            progress(&[DONE_0], "2026-08-29T04:01:00Z", 2, 60),
            DoneProgress::ChallengeDue {
                confirmations: 0,
                required: 2,
                done_age_secs: 60,
                attempts: 0,
            }
        );
        assert_eq!(
            progress(&[DONE_0, CHALLENGE_1], "2026-08-29T04:01:59Z", 2, 60),
            DoneProgress::Challenged {
                confirmations: 0,
                required: 2,
            },
            "journal reconstruction survives a daemon restart"
        );
        assert_eq!(
            progress(
                &[DONE_0, CHALLENGE_1, DONE_1],
                "2026-08-29T04:02:30Z",
                2,
                60
            ),
            DoneProgress::ChallengeDue {
                confirmations: 1,
                required: 2,
                done_age_secs: 60,
                attempts: 0,
            }
        );
        assert_eq!(
            progress(
                &[DONE_0, CHALLENGE_1, DONE_1, CHALLENGE_2, DONE_2],
                "2026-08-29T14:03:00Z",
                2,
                60,
            ),
            DoneProgress::Confirmed
        );
    }

    fn wprog(lines: &[&str], now: &str, state: WaitState) -> WaitProgress {
        wprog_as(lines, now, 2, 60, state, Some("launch-1"))
    }

    fn wprog_as(
        lines: &[&str],
        now: &str,
        required: u8,
        cadence: u64,
        state: WaitState,
        launch: Option<&str>,
    ) -> WaitProgress {
        wait_progress(
            &log(lines),
            "aerewrite",
            "main",
            "opus5:builder",
            launch,
            Timestamp::parse(now).expect("test time"),
            cadence,
            required,
            state,
        )
    }

    /// One-line `WaitProgress` constructors: every case runs at required 2
    /// except the zero-knob test, which asserts `None`.
    fn prov(c: u8) -> WaitProgress {
        WaitProgress::Provisional {
            confirmations: c,
            required: 2,
        }
    }
    fn due(c: u8, age: u64, attempts: u32) -> WaitProgress {
        WaitProgress::ChallengeDue {
            confirmations: c,
            required: 2,
            wait_age_secs: age,
            attempts,
        }
    }
    fn chall(c: u8) -> WaitProgress {
        WaitProgress::Challenged {
            confirmations: c,
            required: 2,
        }
    }
    fn lapsed(c: u8) -> WaitProgress {
        WaitProgress::Lapsed {
            confirmations: c,
            required: 2,
        }
    }

    const BLOCKED_0: &str = r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"blocked","summary":"dep down","actor_slot":"main","actor_session":"aerewrite"}"#;
    const WCHALLENGE_1: &str = r#"{"ts":"2026-08-29T04:01:00Z","actor":"watchdog","action":"wait-challenge","target":"opus5:builder","ref":"launch-1","target_slot":"main","target_session":"aerewrite","summary":"blocked confirmation 1/2"}"#;
    const BLOCKED_1: &str = r#"{"ts":"2026-08-29T04:01:30Z","actor":"opus5:builder","action":"state","ref":"blocked","summary":"dep still down","actor_slot":"main","actor_session":"aerewrite"}"#;
    const WCHALLENGE_2: &str = r#"{"ts":"2026-08-29T04:02:30Z","actor":"watchdog","action":"wait-challenge","target":"opus5:builder","ref":"launch-1","target_slot":"main","target_session":"aerewrite","summary":"blocked confirmation 2/2"}"#;
    const BLOCKED_2: &str = r#"{"ts":"2026-08-29T04:03:00Z","actor":"opus5:builder","action":"state","ref":"blocked","summary":"dep down, probe 2","actor_slot":"main","actor_session":"aerewrite"}"#;
    const WAGENT_0: &str = r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-agent","summary":"on colead","actor_slot":"main","actor_session":"aerewrite"}"#;
    const DECL_USER: &str = r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-user"}"#;
    const DECL_WAGENT: &str = r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-agent"}"#;
    const DECL_BLOCKED: &str = r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"blocked","summary":"dep"}"#;
    const DECL_DONE: &str = r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"done","summary":"shipped"}"#;
    const DECL_WORKING: &str = r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"working","summary":"on it"}"#;
    const T00_30: &str = "2026-08-29T04:00:30Z";
    const T00_59: &str = "2026-08-29T04:00:59Z";
    const T01_00: &str = "2026-08-29T04:01:00Z";
    const T01_59: &str = "2026-08-29T04:01:59Z";
    const T02_00: &str = "2026-08-29T04:02:00Z";
    const T02_30: &str = "2026-08-29T04:02:30Z";
    const T03_30: &str = "2026-08-29T04:03:30Z";
    const T04_00: &str = "2026-08-29T04:04:00Z";
    const NEXT_DAY: &str = "2026-08-30T04:00:00Z";

    #[test]
    fn wait_is_challenged_twice_then_rearmed_never_confirmed() {
        let b = Blocked;
        assert_eq!(wprog(&[BLOCKED_0], T00_59, b), prov(0));
        assert_eq!(wprog(&[BLOCKED_0], T01_00, b), due(0, 60, 0));
        assert_eq!(wprog(&[BLOCKED_0, BLOCKED_1], T02_00, b), prov(0));
        assert_eq!(wprog(&[BLOCKED_0, BLOCKED_1], T02_30, b), due(0, 60, 0));
        assert_eq!(wprog(&[BLOCKED_0, WCHALLENGE_1], T01_59, b), chall(0));
        let one = &[BLOCKED_0, WCHALLENGE_1, BLOCKED_1];
        assert_eq!(wprog(one, T02_30, b), due(1, 60, 0));
        // The Nth proof re-arms a fresh episode — waits are never terminally
        // proven — and the re-ask recurs one cadence after that proof.
        let full = &[BLOCKED_0, WCHALLENGE_1, BLOCKED_1, WCHALLENGE_2, BLOCKED_2];
        assert_eq!(wprog(full, T03_30, b), prov(0));
        assert_eq!(wprog(full, T04_00, b), due(0, 60, 0));
    }

    #[test]
    fn an_unanswered_wait_challenge_lapses_and_late_proof_keeps_credit() {
        let b = Blocked;
        assert_eq!(wprog(&[BLOCKED_0, WCHALLENGE_1], T02_00, b), lapsed(0));
        let late = BLOCKED_1.replace("04:01:30", "04:03:00");
        assert_eq!(wprog(&[BLOCKED_0, WCHALLENGE_1, &late], T03_30, b), prov(1));
    }

    #[test]
    fn wait_confirmation_is_strict_same_state() {
        let wagent = WAGENT_0.replace("04:00:00", "04:01:30");
        let stale = WCHALLENGE_1.replace("04:01:00", "04:02:00");
        let legs = &[BLOCKED_0, WCHALLENGE_1, &wagent, &stale];
        // The other wait state's declaration supersedes the blocked episode;
        // a stale challenge for the old state leaves the new one alone.
        assert_eq!(wprog(legs, T02_30, Blocked), WaitProgress::None);
        assert_eq!(wprog(&legs[1..4], T02_00, WaitingAgent), prov(0));
    }

    #[test]
    fn wait_counts_every_appended_line_without_dedup() {
        // Identical declarations around a same-second challenge: append order
        // rules, where done's legacy dedup would skip the second line.
        let b = Blocked;
        let challenge = WCHALLENGE_1.replace("04:01:00", "04:00:00");
        assert_eq!(
            wprog(&[BLOCKED_0, &challenge, BLOCKED_0], T00_30, b),
            prov(1)
        );
    }

    #[test]
    fn wait_survives_nudge_and_abandoned_footprints() {
        let b = Blocked;
        let nudge = r#"{"ts":"2026-08-29T04:01:15Z","actor":"watchdog","action":"nudge","target":"opus5:builder","target_slot":"main","target_session":"aerewrite"}"#;
        let legs = &[BLOCKED_0, WCHALLENGE_1, nudge, BLOCKED_1];
        assert_eq!(wprog(legs, T02_30, b), due(1, 60, 0));
        let unconfirmed = WCHALLENGE_1.replacen("blocked", "[unconfirmed] blocked", 1);
        assert_eq!(wprog(&[BLOCKED_0, &unconfirmed], T02_00, b), due(0, 120, 1));
        let abandoned = r#"{"ts":"2026-08-29T04:01:15Z","actor":"watchdog","action":"delivery-abandoned","target":"opus5:builder","ref":"launch-1","target_slot":"main","target_session":"aerewrite","summary":"refused: busy pane; blocked confirmation 1/2"}"#;
        assert_eq!(
            wprog(&[BLOCKED_0, abandoned, WCHALLENGE_1], T01_59, b),
            chall(0)
        );
        let f = abandoned.replace("launch-1", "launch-9");
        assert_eq!(wprog(&[BLOCKED_0, &f], T02_00, b), due(0, 120, 0));
    }

    #[test]
    fn own_news_supersedes_a_wait_episode() {
        let b = Blocked;
        for state in ["working", "waiting-user", "done"] {
            let news = BLOCKED_1
                .replace("04:01:30", "04:02:00")
                .replace("blocked", state);
            let legs = &[BLOCKED_0, WCHALLENGE_1, &news];
            assert_eq!(
                wprog(legs, T02_30, b),
                WaitProgress::None,
                "{state} supersedes"
            );
        }
        // Inbound, only a human through a chat bridge ends a `blocked` wait: a
        // peer's message leaves the episode, and its challenge, standing.
        let human = r#"{"ts":"2026-08-29T04:02:00Z","actor":"telegram:42","action":"send","target":"opus5:builder","target_slot":"main","target_session":"aerewrite"}"#;
        let late = BLOCKED_1.replace("04:01:30", "04:03:00");
        let legs = &[BLOCKED_0, WCHALLENGE_1, human, &late];
        assert_eq!(wprog(legs, T03_30, b), prov(0));
        let peer = human.replace("telegram:42", "lead");
        let legs = &[BLOCKED_0, WCHALLENGE_1, &peer, &late];
        assert_eq!(wprog(legs, T03_30, b), prov(1));
    }

    #[test]
    fn crossed_challenges_reset_the_other_episode() {
        let done_challenge = WCHALLENGE_1.replace("wait-challenge", "done-challenge");
        assert_eq!(
            wprog(&[BLOCKED_0, &done_challenge], T02_00, Blocked),
            WaitProgress::None
        );
        // A wait challenge resets a done episode: the safe direction, with no
        // done-fold change.
        assert_eq!(
            progress(&[DONE_0, WCHALLENGE_1], T02_00, 2, 60),
            DoneProgress::None
        );
    }

    #[test]
    fn zero_knobs_leave_waits_unchallenged() {
        let l = Some("launch-1");
        assert_eq!(
            wprog_as(&[BLOCKED_0], NEXT_DAY, 0, 60, Blocked, l),
            WaitProgress::None
        );
        assert_eq!(
            wprog_as(&[WAGENT_0], NEXT_DAY, 2, 0, WaitingAgent, l),
            WaitProgress::None
        );
    }

    #[test]
    fn a_wait_challenge_walked_past_keeps_matching_wait_currency_only() {
        let cases = [
            (DECL_USER, None),
            (DECL_WAGENT, Some(QuietKind::WaitingAgent)),
            (DECL_BLOCKED, Some(QuietKind::Blocked)),
            (DECL_DONE, None),
        ];
        for (declaration, want) in cases {
            let events = log(&[declaration, WCHALLENGE_1]);
            let found = latest_relevant_event(&events, "aerewrite", "main", "opus5:builder")
                .expect("the declaration stays selected under the challenge");
            assert_eq!(quiet_reason(&found), want);
        }
        // `working` declares no quiet state, so the pin is on currency itself.
        let events = log(&[DECL_WORKING, WCHALLENGE_1]);
        let found = latest_relevant_event(&events, "aerewrite", "main", "opus5:builder")
            .expect("the working declaration stays selected");
        assert!(!declaration_current(&found));
    }

    #[test]
    fn wait_launch_matching_mirrors_done() {
        let b = Blocked;
        let wrong = WCHALLENGE_1.replace("launch-1", "launch-2");
        assert_eq!(wprog(&[BLOCKED_0, &wrong], T02_00, b), WaitProgress::None);
        let refless = WCHALLENGE_1.replace(r#","ref":"launch-1""#, "");
        assert_eq!(
            wprog_as(&[BLOCKED_0, &refless], T01_59, 2, 60, b, None),
            chall(0)
        );
    }

    #[test]
    fn zero_knobs_preserve_todays_done_without_challenges() {
        assert_eq!(
            progress(&[DONE_0], "2026-08-30T04:00:00Z", 0, 60),
            DoneProgress::Confirmed
        );
        assert_eq!(
            progress(&[DONE_0], "2026-08-30T04:00:00Z", 2, 0),
            DoneProgress::Confirmed
        );
    }

    #[test]
    fn done_cannot_preconfirm_and_same_second_order_is_append_order() {
        let repeated = DONE_1.replace("04:01:30", "04:00:00");
        assert_eq!(
            progress(&[DONE_0, &repeated], "2026-08-29T04:00:30Z", 2, 60),
            DoneProgress::Provisional {
                confirmations: 0,
                required: 2,
            }
        );
        let challenge = CHALLENGE_1.replace("04:01:00", "04:00:00");
        assert_eq!(
            progress(
                &[DONE_0, &challenge, &repeated],
                "2026-08-29T04:00:30Z",
                2,
                60,
            ),
            DoneProgress::Provisional {
                confirmations: 1,
                required: 2,
            }
        );
        assert_eq!(
            progress(
                &[DONE_0, &repeated, &challenge],
                "2026-08-29T04:00:30Z",
                2,
                60,
            ),
            DoneProgress::Challenged {
                confirmations: 0,
                required: 2,
            }
        );
    }

    #[test]
    fn an_unanswered_challenge_lapses_to_the_idle_path() {
        assert_eq!(
            progress(&[DONE_0, CHALLENGE_1], "2026-08-29T04:02:00Z", 2, 60),
            DoneProgress::Lapsed {
                confirmations: 0,
                required: 2,
            }
        );
    }

    #[test]
    fn a_late_answer_keeps_credit_until_an_inbound_boundary() {
        let late = DONE_1.replace("04:01:30", "04:03:00");
        assert_eq!(
            progress(&[DONE_0, CHALLENGE_1, &late], "2026-08-29T04:03:30Z", 2, 60,),
            DoneProgress::Provisional {
                confirmations: 1,
                required: 2,
            }
        );
        let inbound = r#"{"ts":"2026-08-29T04:02:00Z","actor":"lead","action":"send","target":"opus5:builder","target_slot":"main","target_session":"aerewrite"}"#;
        assert_eq!(
            progress(
                &[DONE_0, CHALLENGE_1, inbound, &late],
                "2026-08-29T04:03:30Z",
                2,
                60,
            ),
            DoneProgress::Provisional {
                confirmations: 0,
                required: 2,
            }
        );
    }

    #[test]
    fn an_own_non_done_declaration_ends_the_episode_without_credit() {
        let working = r#"{"ts":"2026-08-29T04:01:30Z","actor":"opus5:builder","action":"state","ref":"working","summary":"finishing tests","actor_slot":"main","actor_session":"aerewrite"}"#;
        assert_eq!(
            progress(
                &[DONE_0, CHALLENGE_1, working],
                "2026-08-29T04:02:00Z",
                2,
                60,
            ),
            DoneProgress::None,
            "only a later done can consume an outstanding challenge"
        );
    }

    #[test]
    fn a_successor_or_missing_launch_witness_inherits_no_confirmation() {
        let events = log(&[DONE_0, CHALLENGE_1, DONE_1]);
        assert_eq!(
            done_progress(
                &events,
                "aerewrite",
                "main",
                "opus5:builder",
                Some("launch-2"),
                Timestamp::parse("2026-08-29T04:02:00Z").expect("test time"),
                60,
                2,
            ),
            DoneProgress::Provisional {
                confirmations: 0,
                required: 2,
            }
        );
        assert_eq!(
            done_progress(
                &events,
                "aerewrite",
                "main",
                "opus5:builder",
                None,
                Timestamp::parse("2026-08-29T04:02:00Z").expect("test time"),
                60,
                2,
            ),
            DoneProgress::Provisional {
                confirmations: 0,
                required: 2
            },
            "no incarnation witness must not inherit journal credit"
        );
    }

    #[test]
    fn unconfirmed_delivery_does_not_arm_a_confirmation() {
        let unconfirmed =
            CHALLENGE_1.replacen('}', r#","summary":"[unconfirmed] done challenge"}"#, 1);
        assert!(crate::tracked::summary_is_unconfirmed(
            event(&unconfirmed).summary.as_deref()
        ));
        assert!(matches!(
            progress(&[DONE_0, &unconfirmed], "2026-08-29T04:01:30Z", 2, 60),
            DoneProgress::ChallengeDue { attempts: 1, .. }
        ));
    }

    #[test]
    fn failed_challenge_attempts_survive_restart_and_ignore_abandoned_nudges() {
        let first = CHALLENGE_1.replacen('}', r#","summary":"[unconfirmed] first"}"#, 1);
        let second = CHALLENGE_2.replacen('}', r#","summary":"[unconfirmed] second"}"#, 1);
        assert!(matches!(
            progress(&[DONE_0, &first, &second], "2026-08-29T04:03:30Z", 5, 60),
            DoneProgress::ChallengeDue { attempts: 2, .. }
        ));

        let abandoned_nudge = r#"{"ts":"2026-08-29T04:01:00Z","actor":"watchdog","action":"delivery-abandoned","target":"opus5:builder","target_slot":"main","target_session":"aerewrite"}"#;
        assert!(matches!(
            progress(&[DONE_0, abandoned_nudge], "2026-08-29T04:02:00Z", 5, 60),
            DoneProgress::ChallengeDue { attempts: 0, .. }
        ));
    }

    #[test]
    fn a_ref_less_watchdog_nudge_preserves_the_degraded_episode() {
        let challenge = CHALLENGE_1.replace(r#","ref":"launch-1""#, "");
        let nudge = r#"{"ts":"2026-08-29T04:02:00Z","actor":"watchdog","action":"nudge","target":"opus5:builder","target_slot":"main","target_session":"aerewrite"}"#;
        assert_eq!(
            done_progress(
                &log(&[DONE_0, &challenge, DONE_1, nudge]),
                "aerewrite",
                "main",
                "opus5:builder",
                None,
                Timestamp::parse("2026-08-29T04:02:30Z").expect("test time"),
                60,
                5,
            ),
            DoneProgress::ChallengeDue {
                confirmations: 1,
                required: 5,
                done_age_secs: 60,
                attempts: 0,
            },
            "a delivered nudge preserves both credit and the failed-attempt budget"
        );
    }

    #[test]
    fn delivered_challenges_never_spend_the_failure_budget() {
        let mut owned = vec![DONE_0.to_owned()];
        for round in 1..=3 {
            owned.push(format!(
                r#"{{"ts":"2026-08-29T04:{:02}:00Z","actor":"watchdog","action":"done-challenge","target":"opus5:builder","ref":"launch-1","target_slot":"main","target_session":"aerewrite"}}"#,
                round * 2 - 1
            ));
            owned.push(format!(
                r#"{{"ts":"2026-08-29T04:{:02}:30Z","actor":"opus5:builder","action":"state","ref":"done","summary":"proof {round}","actor_slot":"main","actor_session":"aerewrite"}}"#,
                round * 2 - 1
            ));
        }
        let lines: Vec<&str> = owned.iter().map(String::as_str).collect();
        assert_eq!(
            progress(&lines, "2026-08-29T04:06:30Z", 5, 60),
            DoneProgress::ChallengeDue {
                confirmations: 3,
                required: 5,
                done_age_secs: 60,
                attempts: 0,
            },
            "healthy delivered challenges never spend the failure budget"
        );
    }

    #[test]
    fn done_challenge_is_walked_past_without_ending_done() {
        let events = log(&[DONE_0, CHALLENGE_1]);
        let found = latest_relevant_event(&events, "aerewrite", "main", "opus5:builder")
            .expect("done remains relevant");
        assert!(found.looked_past_done_challenge);
        assert_eq!(quiet_reason(&found), Some(QuietKind::Done));
    }

    #[test]
    fn abandoned_delivery_is_walked_past_without_ending_done() {
        let abandoned = r#"{"ts":"2026-08-29T04:01:00Z","actor":"watchdog","action":"delivery-abandoned","target":"opus5:builder","target_slot":"main","target_session":"aerewrite","summary":"refused: busy pane; confirmation 1/2"}"#;
        let events = log(&[DONE_0, abandoned]);
        let found = latest_relevant_event(&events, "aerewrite", "main", "opus5:builder")
            .expect("done remains relevant");
        assert!(found.looked_past_abandoned);
        assert_eq!(quiet_reason(&found), Some(QuietKind::Done));
    }

    #[test]
    fn challenge_and_abandonment_do_not_extend_foreign_states() {
        for action in ["done-challenge", "delivery-abandoned"] {
            for state in ["waiting-user", "waiting-agent", "blocked"] {
                let declaration = format!(
                    r#"{{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"{state}"}}"#
                );
                let footprint = format!(
                    r#"{{"ts":"2026-08-29T04:01:00Z","actor":"watchdog","action":"{action}","target":"opus5:builder","ref":"launch-1","summary":"refused: busy pane; confirmation 1/2"}}"#
                );
                let events = log(&[&declaration, &footprint]);
                let found = latest_relevant_event(&events, "aerewrite", "main", "opus5:builder")
                    .expect("declaration is walked back to");
                // An abandoned delivery is the episode's own footprint for a
                // matching wait, like a delivered challenge; foreign to rest.
                let want = match (action, state) {
                    ("delivery-abandoned", "waiting-agent") => Some(QuietKind::WaitingAgent),
                    ("delivery-abandoned", "blocked") => Some(QuietKind::Blocked),
                    _ => None,
                };
                assert_eq!(quiet_reason(&found), want, "{action} after {state}");
            }
        }
    }

    #[test]
    fn declaration_currency_has_one_owner_distinct_from_quietness() {
        let current = |declaration: &str, footprint: Option<&str>| {
            let mut lines = vec![declaration];
            lines.extend(footprint);
            let events = log(&lines);
            let found = latest_relevant_event(&events, "aerewrite", "main", "opus5:builder")
                .expect("declaration selected");
            declaration_current(&found)
        };
        let working = r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"working"}"#;
        let nudge = r#"{"ts":"2026-08-29T04:01:00Z","actor":"watchdog","action":"nudge","target":"opus5:builder"}"#;
        assert!(current(working, None), "working is current but not quiet");
        assert!(
            !current(DONE_0, Some(nudge)),
            "ordinary nudge invalidates done"
        );
        assert!(
            current(DONE_0, Some(CHALLENGE_1)),
            "challenge preserves done currency"
        );
    }

    #[test]
    fn the_waiting_agent_ceiling_is_the_own_work_cap_times_the_nudge_cadence() {
        assert_eq!(OWN_WORK_AGE_CAP, 4, "the reused cap, unchanged");
        assert_eq!(waiting_agent_cap_secs(300), 1_200);
        assert_eq!(waiting_agent_cap_secs(60), 240);
        assert_eq!(
            waiting_agent_cap_secs(0),
            DEFAULT_IDLE_NUDGE_SECS * OWN_WORK_AGE_CAP,
            "0 switches nudging off and leaves the attention half on the default cadence"
        );
        assert!(
            !waiting_agent_escalated(1_199, 300),
            "one second short of the ceiling is still fresh"
        );
        assert!(
            waiting_agent_escalated(1_200, 300),
            "the boundary escalates"
        );
        assert!(
            waiting_agent_escalated(u64::MAX, 300),
            "saturates, never panics"
        );
        assert!(
            waiting_agent_escalated(1_200, 0),
            "a zero nudge knob must not delete the attention path"
        );
    }

    #[test]
    fn the_declaration_key_is_the_full_tuple_so_a_same_second_redeclare_re_arms() {
        let first = event(
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-user","summary":"review"}"#,
        );
        assert_eq!(
            declaration_key(&first),
            "state|2026-08-29T04:00:00Z|waiting-user|opus5:builder|review"
        );
        // Same second, same state, different reason — a genuinely new declaration.
        let second = event(
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-user","summary":"now something else"}"#,
        );
        assert_ne!(declaration_key(&first), declaration_key(&second));
        // An absent ref and summary render empty.
        let bare =
            event(r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"done"}"#);
        assert_eq!(
            declaration_key(&bare),
            "done|2026-08-29T04:00:00Z||opus5:builder|"
        );
    }

    #[test]
    fn throttle_matches_the_right_catalog_per_binary() {
        assert!(shows_throttle(
            "... Server is temporarily limiting requests ...",
            "claude"
        ));
        assert!(shows_throttle("boom RateLimitError happened", "codex"));
        assert!(shows_throttle("RESOURCE_EXHAUSTED now", "gemini"));
        // A claude phrase must NOT trip a codex pane (per-tool catalogs).
        assert!(
            !shows_throttle("Server is temporarily limiting requests", "codex"),
            "claude's phrase is not codex's"
        );
    }

    #[test]
    fn opencode_is_the_union_of_every_provider_catalog() {
        for phrase in [
            "Server is temporarily limiting requests", // claude
            "ratelimit_exceeded",                      // codex
            "Quota exceeded",                          // gemini
        ] {
            assert!(
                shows_throttle(phrase, "opencode"),
                "opencode union misses {phrase:?}"
            );
        }
    }

    #[test]
    fn the_generic_pair_applies_to_every_tool_including_unknown_ones() {
        for bin in [
            "claude",
            "codex",
            "gemini",
            "opencode",
            "grok",
            "somethingelse",
        ] {
            assert!(
                shows_throttle("HTTP 429 Too Many Requests", bin),
                "{bin} misses 429"
            );
            assert!(
                shows_throttle("got 503 Service Unavailable", bin),
                "{bin} misses 503"
            );
        }
    }

    #[test]
    fn an_empty_buffer_and_ordinary_prose_are_never_throttled() {
        assert!(!shows_throttle("", "claude"));
        assert!(
            !shows_throttle("working on the task, all normal here, no errors", "claude"),
            "ordinary prose is not throttling"
        );
        // An unknown binary sees only the generics, so a tool-specific phrase misses.
        assert!(
            !shows_throttle("RateLimitError", "grok"),
            "unknown bin sees only generics"
        );
    }

    #[test]
    fn throttle_classes_split_measured_limit_phrases_from_transient_ones() {
        // Measured phrases, claude 2.1.274 then codex 0.154.0.
        for phrase in [
            "You've hit your 5-hour limit \u{b7} resets in 2h",
            "You're out of usage credits. /model to switch models.",
            "Your org is out of usage \u{b7} contact your admin",
            "Goal paused \u{b7} usage limit reached \u{b7} send a message",
        ] {
            assert_eq!(
                throttle_class(phrase, "claude"),
                Some(Throttle::LimitReached)
            );
        }
        for phrase in [
            "You've hit your usage limit. Upgrade to Pro to continue",
            "You've reached your usage limit",
            "Quota exceeded. Check your plan and billing details.",
        ] {
            assert_eq!(
                throttle_class(phrase, "codex"),
                Some(Throttle::LimitReached)
            );
        }
        // A phrase of one catalog misses a binary that never measured it;
        // gemini's OWN transient phrase is a prefix of codex's — still a
        // transient claim — and opencode is the union of both limit catalogs.
        assert_eq!(throttle_class("You're out of usage credits", "codex"), None);
        assert_eq!(
            throttle_class("Quota exceeded. Check your plan", "claude"),
            None
        );
        assert_eq!(
            throttle_class("Quota exceeded. Check your plan", "gemini"),
            Some(Throttle::Throttled)
        );
        for phrase in [
            "You're out of usage credits",
            "Quota exceeded. Check your plan",
        ] {
            assert_eq!(
                throttle_class(phrase, "opencode"),
                Some(Throttle::LimitReached)
            );
        }
        // Every existing transient phrase keeps its class.
        for (phrase, bin) in [
            ("Server is temporarily limiting requests", "claude"),
            ("API Error: Overloaded", "claude"),
            ("Anthropic API error", "claude"),
            ("Rate limit exceeded", "codex"),
            ("RateLimitError", "codex"),
            ("ratelimit_exceeded", "codex"),
            ("RESOURCE_EXHAUSTED", "gemini"),
            ("Quota exceeded", "gemini"),
            ("HTTP 429 Too Many Requests", "grok"),
            ("503 Service Unavailable", "somethingelse"),
        ] {
            assert_eq!(throttle_class(phrase, bin), Some(Throttle::Throttled));
        }
        for bin in ["claude", "codex", "gemini", "opencode", "grok"] {
            assert_eq!(throttle_class("", bin), None, "{bin}: empty buffer");
            assert_eq!(
                throttle_class("all normal, no errors", bin),
                None,
                "{bin}: prose"
            );
        }
    }

    /// The container as APPEND ORDER gives it: oldest first, the way every
    /// caller must hand it over.
    fn log(lines: &[&str]) -> Vec<Event> {
        lines.iter().map(|line| event(line)).collect()
    }

    #[test]
    fn the_newest_relevant_event_wins_and_unrelated_ones_are_stepped_over() {
        let events = log(&[
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"working"}"#,
            r#"{"ts":"2026-08-29T04:00:01Z","actor":"opus5:builder","action":"state","ref":"waiting-user","summary":"review"}"#,
            r#"{"ts":"2026-08-29T04:00:02Z","actor":"gpt56sol:colead","action":"memo","ref":"arch"}"#,
            r#"{"ts":"2026-08-29T04:00:03Z","actor":"fable5:lead","action":"state","ref":"working"}"#,
        ]);
        let found = latest_relevant_event(&events, "aerewrite", "main", "opus5:builder")
            .expect("the declaration is relevant");
        assert!(found.is_own, "it is the seat's own record");
        assert_eq!(found.event.reference.as_deref(), Some("waiting-user"));
        assert!(!found.looked_past_nudge, "no nudge was walked past");
    }

    #[test]
    fn the_look_back_is_unbounded() {
        // A quiet state stays valid until a NEWER event FOR THIS AGENT arrives,
        // however much unrelated traffic follows it.
        let mut lines = vec![
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"blocked","summary":"dep"}"#,
        ];
        let filler =
            r#"{"ts":"2026-08-29T04:00:01Z","actor":"someone:else","action":"memo","ref":"t"}"#;
        lines.extend(std::iter::repeat_n(filler, 500));
        let events = log(&lines);
        let found = latest_relevant_event(&events, "aerewrite", "main", "opus5:builder")
            .expect("500 unrelated events do not end the walk");
        assert_eq!(found.event.reference.as_deref(), Some("blocked"));
    }

    #[test]
    fn all_three_relevance_forms_are_matched() {
        let own = log(&[
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"done"}"#,
        ]);
        let targeted = log(&[
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"fable5:lead","action":"send","target":"opus5:builder","summary":"hi"}"#,
        ]);
        let cross_session = log(&[
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"fable5:lead","action":"send","target":"@aerewrite:opus5:builder","summary":"hi"}"#,
        ]);
        for (events, form) in [
            (&own, "actor"),
            (&targeted, "target"),
            (&cross_session, "@session:agent target"),
        ] {
            assert!(
                latest_relevant_event(events, "aerewrite", "main", "opus5:builder").is_some(),
                "the {form} form is relevant"
            );
        }
        // A different session's spelling of the same name is NOT this agent's.
        assert!(
            latest_relevant_event(&cross_session, "other", "main", "opus5:builder").is_none(),
            "the cross-session form is keyed to the session"
        );
        // Nothing mentioning the agent at all.
        let unrelated = log(&[
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"fable5:lead","action":"send","target":"gpt56sol:colead"}"#,
        ]);
        assert!(latest_relevant_event(&unrelated, "aerewrite", "main", "opus5:builder").is_none());
    }

    /// A rename-back history must not let one incarnation's event stand in for
    /// another's: alpha declares `waiting-agent`, the session lives as beta
    /// (the SAME display actor declares `working` there), and it comes back as
    /// alpha. Relevance is judged by the ROUTING KEY, so beta's declaration is
    /// neither alpha's news nor alpha's currency proof — the read side and the
    /// daemon must select alpha's own declaration, together.
    #[test]
    fn relevance_is_routing_aware_so_a_rename_back_cannot_borrow_another_incarnations_event() {
        let events = log(&[
            r#"{"ts":"2026-09-13T08:00:00Z","actor":"lead","action":"state","ref":"waiting-agent","summary":"on colead","actor_slot":"main","actor_session":"alpha"}"#,
            r#"{"ts":"2026-09-13T08:10:00Z","actor":"lead","action":"state","ref":"working","actor_slot":"main","actor_session":"beta"}"#,
        ]);
        let found = latest_relevant_event(&events, "alpha", "main", "lead")
            .expect("alpha's declaration is relevant to alpha");
        assert!(found.is_own);
        assert!(!found.looked_past_nudge);
        assert_eq!(
            found.event.reference.as_deref(),
            Some("waiting-agent"),
            "beta's `working` belongs to another incarnation; alpha's own \
             declaration is the newest relevant event"
        );
    }

    /// The case the rename-back test structurally cannot see: routing is
    /// CORRECT and the actor DISPLAY is stale (a rename-style display that no
    /// longer matches the roster reference). The owner says "own"; a display
    /// re-derivation would say "inbound"; the verdict travels with the record
    /// so the classifier cannot make the weaker call.
    #[test]
    fn a_stale_display_actor_never_becomes_inbound_news() {
        let events = log(&[
            r#"{"ts":"2026-09-13T08:00:00Z","actor":"old-lead","action":"state","ref":"waiting-agent","summary":"on colead","actor_slot":"main","actor_session":"live"}"#,
        ]);
        let found = latest_relevant_event(&events, "live", "main", "lead")
            .expect("the routing key says this seat");
        assert!(
            found.is_own,
            "routing wins over a stale display, as `event_is_actor` says"
        );
        assert_eq!(
            quiet_reason(&found),
            Some(QuietKind::WaitingAgent),
            "and the classifier consumes that verdict instead of re-deriving"
        );
    }

    #[test]
    fn a_declaration_and_the_nudge_answering_it_in_the_same_second_both_survive() {
        // Second-resolution timestamps make these two compare EQUAL, so a
        // ts-bounded look-back would skip the declaration along with the nudge.
        let events = log(&[
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-user","summary":"review"}"#,
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"watchdog","action":"nudge","target":"opus5:builder","summary":"idle 15m"}"#,
        ]);
        let found = latest_relevant_event(&events, "aerewrite", "main", "opus5:builder")
            .expect("the declaration is underneath the nudge");
        assert_eq!(found.event.reference.as_deref(), Some("waiting-user"));
        assert!(found.looked_past_nudge, "a nudge WAS walked past");
        // And the two halves compose the way the daemon will use them.
        assert_eq!(
            quiet_reason(&found),
            Some(QuietKind::WaitingUser),
            "the nudge must not break the hold it was asking about"
        );
    }

    #[test]
    fn however_many_nudges_are_stacked_up_the_walk_consumes_them() {
        let mut lines = vec![
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"done","summary":"shipped"}"#,
        ];
        let nudge = r#"{"ts":"2026-08-29T04:05:00Z","actor":"watchdog","action":"nudge","target":"opus5:builder"}"#;
        lines.extend(std::iter::repeat_n(nudge, 5));
        let events = log(&lines);
        let found = latest_relevant_event(&events, "aerewrite", "main", "opus5:builder")
            .expect("the done is under five nudges");
        assert_eq!(found.event.action, "done");
        assert!(found.is_own && found.looked_past_nudge);
        // Done is the ONE kind a walked-past nudge clears.
        assert_eq!(quiet_reason(&found), None);
    }

    #[test]
    fn only_the_watchdogs_own_nudges_are_walked_past() {
        // A watchdog ALERT is walked past without a footprint: the watchdog
        // is neither the seat nor the human.
        let alerted = log(&[
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"done"}"#,
            r#"{"ts":"2026-08-29T04:05:00Z","actor":"watchdog","action":"alert","target":"opus5:builder","summary":"stale"}"#,
        ]);
        let found = latest_relevant_event(&alerted, "aerewrite", "main", "opus5:builder")
            .expect("the declaration is relevant");
        assert_eq!(found.event.action, "state");
        assert!(found.is_own);
        assert!(!found.looked_past_nudge);
        assert_eq!(quiet_reason(&found), Some(QuietKind::Done));
        // A `nudge` from a PEER is not the watchdog's, and is news to done.
        let peer = log(&[
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"done"}"#,
            r#"{"ts":"2026-08-29T04:05:00Z","actor":"fable5:lead","action":"nudge","target":"opus5:builder"}"#,
        ]);
        let found = latest_relevant_event(&peer, "aerewrite", "main", "opus5:builder")
            .expect("the peer event is relevant");
        assert_eq!(found.event.actor, "fable5:lead");
        assert!(!found.is_own, "a peer writing to the seat is inbound");
        assert!(!found.looked_past_nudge);
        assert_eq!(
            quiet_reason(&found),
            None,
            "a peer writing to a done agent is news, and news ends done"
        );
    }

    #[test]
    fn a_log_of_nothing_but_nudges_selects_nothing() {
        let events = log(&[
            r#"{"ts":"2026-08-29T04:05:00Z","actor":"watchdog","action":"nudge","target":"opus5:builder"}"#,
        ]);
        assert!(latest_relevant_event(&events, "aerewrite", "main", "opus5:builder").is_none());
        assert!(latest_relevant_event(&[], "aerewrite", "main", "opus5:builder").is_none());
    }

    #[test]
    fn selection_follows_append_order_even_when_the_timestamps_disagree() {
        // The ONE property that a ts-ordered implementation would get wrong while
        // every other test still passed: the newest APPENDED event carries an
        // OLDER timestamp than the one before it (clock skew, or a container
        // stitched from two generations).
        let events = log(&[
            r#"{"ts":"2026-08-29T04:09:00Z","actor":"opus5:builder","action":"state","ref":"done"}"#,
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"fable5:lead","action":"send","target":"opus5:builder","summary":"answered"}"#,
        ]);
        let found = latest_relevant_event(&events, "aerewrite", "main", "opus5:builder")
            .expect("something is relevant");
        assert_eq!(
            found.event.actor, "fable5:lead",
            "the LAST APPENDED relevant event wins, whatever its ts says"
        );
    }

    /// One record the table below stamps after every declaration it makes:
    /// the seat's own when `actor` is the seat, else one addressed to it.
    fn after_declaration(actor: &str, action: &str, extra: &str) -> String {
        let routing = if actor == "opus5:builder" {
            r#""actor_slot":"main","actor_session":"aerewrite""#
        } else {
            r#""target":"opus5:builder","target_slot":"main","target_session":"aerewrite""#
        };
        format!(
            r#"{{"ts":"2026-08-29T04:05:00Z","actor":"{actor}","action":"{action}",{routing}{extra}}}"#
        )
    }

    /// Whether the seat's `state` declaration is still current after `record`:
    /// the walk's verdict, and — for a record [`ends_quiet`] judges — the
    /// episode fold's, which must agree with it.
    fn stands_after(state: &str, record: &str, fold: bool) -> bool {
        let own_ask = r#"{"ts":"2026-08-29T03:59:00Z","actor":"opus5:builder","action":"ask","ref":"ae-1","target":"colead","actor_slot":"main","actor_session":"aerewrite"}"#;
        let own_review = r#"{"ts":"2026-08-29T03:59:10Z","actor":"opus5:builder","action":"review","ref":"review-1","target":"colead","actor_slot":"main","actor_session":"aerewrite"}"#;
        // Another seat's ask: a reply to it is no answer to this seat.
        let peers_ask = r#"{"ts":"2026-08-29T03:59:30Z","actor":"colead","action":"ask","ref":"ae-9","target":"lead"}"#;
        let declaration = format!(
            r#"{{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"{state}","actor_slot":"main","actor_session":"aerewrite"}}"#
        );
        let events = log(&[own_ask, own_review, peers_ask, &declaration, record]);
        let current = latest_relevant_event(&events, "aerewrite", "main", "opus5:builder")
            .is_some_and(|found| {
                declaration_current(&found) && found.event.declared_state() == Some(state)
            });
        if fold {
            let now = Timestamp::parse("2026-08-29T04:05:30Z").expect("test time");
            let (session, slot, agent, launch) =
                ("aerewrite", "main", "opus5:builder", Some("launch-1"));
            let episode = match state {
                "done" => {
                    done_progress(&events, session, slot, agent, launch, now, 60, 2)
                        != DoneProgress::None
                }
                "waiting-agent" | "blocked" => {
                    let want = if state == "blocked" {
                        WaitState::Blocked
                    } else {
                        WaitState::WaitingAgent
                    };
                    wait_progress(&events, session, slot, agent, launch, now, 60, 2, want)
                        != WaitProgress::None
                }
                _ => current,
            };
            assert_eq!(episode, current, "{state} after {record}: fold vs walk");
        }
        current
    }

    /// T1: every record a quiet seat can see after its declaration, against
    /// every quiet state, both ways. `ends` reads done, waiting-user,
    /// waiting-agent, blocked; `fold` marks the records [`ends_quiet`] judges,
    /// where the episode fold must agree — the watchdog's challenge footprints
    /// keep their own episode rules (#144), so only the walk is pinned there.
    #[test]
    fn what_ends_each_quiet_state_is_one_table() {
        const STATES: [&str; 4] = ["done", "waiting-user", "waiting-agent", "blocked"];
        for (record, ends, fold) in &quiet_rows() {
            for (state, ends) in STATES.iter().zip(ends) {
                assert_eq!(
                    stands_after(state, record, *fold),
                    !ends,
                    "{state} after {record}"
                );
            }
        }
    }

    /// T1's rows: a record, whether it ends done, waiting-user, waiting-agent
    /// and blocked, and whether the episode fold is pinned beside the walk.
    fn quiet_rows() -> Vec<(String, [bool; 4], bool)> {
        const DONE_ONLY: [bool; 4] = [true, false, false, false];
        let me = "opus5:builder";
        let judged = [
            (me, "state", r#","ref":"working""#, [true; 4]),
            (me, "memo", r#","ref":"arch""#, DONE_ONLY),
            (me, "send", r#","target":"colead""#, DONE_ONLY),
            (me, "ask", r#","ref":"ae-2","target":"colead""#, DONE_ONLY),
            (me, "reply", r#","ref":"ae-7","target":"lead""#, DONE_ONLY),
            (
                "lead",
                "reply",
                r#","ref":"ae-1""#,
                [true, false, true, false],
            ),
            ("lead", "reply", r#","ref":"ae-9""#, DONE_ONLY),
            (
                "lead",
                "reply",
                r#","ref":"review-1""#,
                [true, false, true, false],
            ),
            ("telegram:42", "send", "", [true; 4]),
            ("discord:42", "send", "", [true; 4]),
            ("ae:compact:0199c0de", "ask", r#","ref":"ae-3""#, DONE_ONLY),
        ];
        let blocked_tag = r#","ref":"launch-1","summary":"blocked confirmation 1/2""#;
        let footprints = [
            ("nudge", "", DONE_ONLY),
            (
                "done-challenge",
                r#","ref":"launch-1""#,
                [false, true, true, true],
            ),
            ("wait-challenge", blocked_tag, [true, true, false, false]),
            (
                "delivery-abandoned",
                blocked_tag,
                [false, true, false, false],
            ),
        ];
        let mut rows: Vec<(String, [bool; 4], bool)> = judged
            .iter()
            .map(|(actor, action, extra, ends)| {
                (after_declaration(actor, action, extra), *ends, true)
            })
            .collect();
        rows.extend(footprints.iter().map(|(action, extra, ends)| {
            (after_declaration("watchdog", action, extra), *ends, false)
        }));
        // Every other peer delivery, and the brief records its spawner writes.
        for action in [
            "send",
            "ask",
            "review",
            "interrupt",
            "brief-delivered",
            "brief-gave-up",
        ] {
            rows.push((
                after_declaration("lead", action, r#","ref":"ae-8""#),
                DONE_ONLY,
                true,
            ));
        }
        // Every record the watchdog writes about the seat that is no footprint.
        for action in [
            "alert",
            "alert-cleared",
            "dead-cleared",
            "human-prompt",
            "human-prompt-cleared",
            "limit",
            "throttled",
            "throttle-cleared",
            "recover",
            super::SWEEP_NUDGE_ACTION,
            crate::quota::action::ADVISORY,
            crate::quota::action::ADVISORY_DROPPED,
            crate::quota::action::CHECKPOINT,
            crate::quota::action::CHECKPOINT_DROPPED,
        ] {
            rows.push((after_declaration("watchdog", action, ""), [false; 4], true));
        }
        rows
    }

    /// T1's footprint rows, by position: a challenge footprint OLDER than the
    /// declaration never counts, and one NEWER counts even when the walk had
    /// to step past a record the wait outlives to reach the declaration.
    #[test]
    fn a_footprint_counts_only_between_the_declaration_and_now() {
        let crossed = r#"{"ts":"2026-08-29T03:58:00Z","actor":"watchdog","action":"done-challenge","target":"opus5:builder","ref":"launch-1","target_slot":"main","target_session":"aerewrite"}"#;
        let peer = r#"{"ts":"2026-08-29T04:05:00Z","actor":"lead","action":"send","target":"opus5:builder","target_slot":"main","target_session":"aerewrite"}"#;
        let current = |lines: &[&str]| {
            latest_relevant_event(&log(lines), "aerewrite", "main", "opus5:builder")
                .is_some_and(|found| declaration_current(&found))
        };
        assert!(current(&[crossed, BLOCKED_0, peer]));
        let newer = crossed.replace("03:58:00", "04:02:00");
        assert!(!current(&[BLOCKED_0, &newer, peer]));
    }

    /// T5 (#115): a `waiting-user` outlives the seat's own chasing and its
    /// peers' traffic together, and only the human or the seat ends it.
    #[test]
    fn a_waiting_user_outlives_its_own_chasing_and_its_peers() {
        let me = "opus5:builder";
        let own_reply = after_declaration(me, "reply", r#","ref":"ae-7","target":"lead""#);
        let own_send = after_declaration(me, "send", r#","target":"lead""#);
        let peer = after_declaration("lead", "send", "");
        let nudge = after_declaration("watchdog", "nudge", "");
        let mut lines = vec![DECL_USER, &own_reply, &peer, &own_send, &nudge, &peer];
        let reason = |lines: &[&str]| {
            latest_relevant_event(&log(lines), "aerewrite", "main", me)
                .and_then(|found| quiet_reason(&found))
        };
        assert_eq!(reason(&lines), Some(QuietKind::WaitingUser));
        let human = after_declaration("telegram:42", "send", "");
        lines.push(&human);
        assert_eq!(reason(&lines), None, "the human answered");
        lines.pop();
        let working = after_declaration(me, "state", r#","ref":"working""#);
        lines.push(&working);
        assert_eq!(reason(&lines), None, "the seat moved on");
    }

    /// P1: the watchdog's newest delivery that may have reached the pane — any
    /// painting action, confirmed or not, addressed to this seat. A challenge
    /// refused before its paste, an abandoned delivery, a watchdog record that
    /// delivers nothing, a peer's send and another seat's nudge are not one.
    #[test]
    fn the_last_watchdog_delivery_is_only_what_may_have_reached_the_pane() {
        let me = "opus5:builder";
        let last = |record: &str| {
            let events = log(&[DECL_USER, record]);
            super::last_watchdog_delivery(&events, "aerewrite", "main", me)
                .map(|found| found.to_string())
        };
        let painted = Some("2026-08-29T04:05:00Z".to_owned());
        for action in [
            "nudge",
            super::SWEEP_NUDGE_ACTION,
            "done-challenge",
            "wait-challenge",
            crate::quota::action::ADVISORY,
            crate::quota::action::CHECKPOINT,
        ] {
            let record = after_declaration("watchdog", action, r#","summary":"[unconfirmed] x""#);
            assert_eq!(last(&record), painted, "{action}");
        }
        let refused = format!(
            r#","summary":"[unconfirmed] blocked confirmation 1/2{}dead pane""#,
            crate::send::REFUSED_PRE_PASTE
        );
        for record in [
            after_declaration("watchdog", "wait-challenge", &refused),
            after_declaration("watchdog", crate::tracked::ABANDONED_ACTION, ""),
            after_declaration("watchdog", "alert", ""),
            after_declaration("lead", "send", ""),
            after_declaration("watchdog", "nudge", "")
                .replace(r#""target_slot":"main""#, r#""target_slot":"spawned.1""#),
        ] {
            assert_eq!(last(&record), None, "{record}");
        }
    }

    /// P1: input from a client viewing the pane ends a wait only when it is
    /// STRICTLY newer than both the declaration and the last delivery there.
    #[test]
    fn human_input_ends_a_wait_only_after_the_declaration_and_the_last_delivery() {
        let declared = Timestamp::from_epoch(1_000);
        let delivered = Some(Timestamp::from_epoch(1_300));
        for (activity, delivery, ends) in [
            (None, None, false),
            (Some(999), None, false),
            (Some(1_000), None, false),
            (Some(1_001), None, true),
            (Some(1_200), delivered, false),
            (Some(1_300), delivered, false),
            (Some(1_301), delivered, true),
            (Some(1_001), Some(Timestamp::from_epoch(500)), true),
            (Some(u64::MAX), None, false),
        ] {
            assert_eq!(
                super::human_input_ends_wait(activity, declared, delivery),
                ends,
                "{activity:?} after {delivery:?}"
            );
        }
    }

    /// Every summary ae writes for a challenge reads back as that challenge
    /// through the ONE grammar: as written, unconfirmed, refused before its
    /// paste, and abandoned — by the real abandoned-delivery writer, under
    /// every hold it can name.
    #[test]
    fn one_grammar_reads_back_every_challenge_summary_ae_writes() {
        use super::{Challenge, challenge_named, challenge_summary};
        use crate::deliver::DeferHeld;
        let dir = std::env::temp_dir().join(format!("ae-wd-grammar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the scratch ledger");
        let holds = [
            DeferHeld::ComposerOccupied,
            DeferHeld::ComposerUnreadable,
            DeferHeld::Viewed,
            DeferHeld::OccupiedAndViewed,
            DeferHeld::UnreadableAndViewed,
        ];
        let mut expected = Vec::new();
        for kind in [
            Challenge::Done,
            Challenge::Wait(WaitingAgent),
            Challenge::Wait(Blocked),
        ] {
            for (number, required) in [(1, 2), (2, 2), (10, 12)] {
                let written = challenge_summary(kind, number, required);
                let refused = format!("{written}{}dead pane", crate::send::REFUSED_PRE_PASTE);
                for summary in [
                    written.clone(),
                    crate::tracked::unconfirmed_summary(&written),
                    crate::tracked::unconfirmed_summary(&refused),
                ] {
                    assert_eq!(challenge_named(&summary), Some(kind), "{summary}");
                }
                for held in holds {
                    let fields = crate::tracked::EventFields {
                        ts: Timestamp::parse("2026-08-29T04:01:00Z").expect("test time"),
                        actor: "watchdog",
                        action: "wait-challenge",
                        target: "opus5:builder",
                        reference: "launch-1",
                        actor_slot: "",
                        actor_session: "aerewrite",
                        target_slot: "main",
                        target_session: "aerewrite",
                        target_server: "",
                        target_pane: "",
                        target_session_uuid: "",
                        caller_server: "",
                        caller_pane: "",
                        caller_session_uuid: "",
                        identity_gap: "",
                        summary: &written,
                        body_file: "",
                    };
                    crate::tracked::record_abandoned_delivery(&dir, &fields, held, &mut Vec::new())
                        .expect("the refusal is recorded");
                    expected.push(kind);
                }
            }
        }
        let events = crate::session::SessionRead::open(&dir)
            .expect("the ledger reads")
            .events;
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(events.len(), expected.len());
        for (event, kind) in events.iter().zip(expected) {
            let summary = event.summary.as_deref();
            assert_eq!(summary.and_then(challenge_named), Some(kind), "{summary:?}");
            for state in [WaitingAgent, Blocked] {
                assert_eq!(
                    super::wait_challenge_names(summary, state),
                    kind == Challenge::Wait(state)
                );
            }
        }
    }

    /// C3 (#145): only an abandoned CHALLENGE is a footprint that ends a
    /// `waiting-user`. The watchdog's other abandoned traffic — an idle nudge,
    /// a quota ask with its receipt or without — leaves it standing, and so
    /// does a challenge's words anywhere but the whole last clause.
    #[test]
    fn only_an_abandoned_challenge_ends_a_waiting_user() {
        let abandoned = |reference: &str, summary: &str| {
            let reference = if reference.is_empty() {
                String::new()
            } else {
                format!(r#","ref":"{reference}""#)
            };
            after_declaration(
                "watchdog",
                crate::tracked::ABANDONED_ACTION,
                &format!(r#"{reference},"summary":"refused: busy pane; {summary}""#),
            )
        };
        for (reference, summary) in [
            ("", "idle 5m, harness waiting at input"),
            (
                "quota-ask-0123456789abcdef",
                "quota claude · Fable — low at 85%",
            ),
            ("", "quota claude · Fable — low at 85%"),
            ("", "quota text asking confirmation 1/2 of a checkpoint"),
            ("launch-1", "waiting-user confirmation 1/2"),
            ("launch-1", "confirmation 1/2 extra"),
            ("launch-1", "blocked confirmation 1/2/3"),
            ("launch-1", "confirmation 1/x"),
            ("launch-1", "confirmation /2"),
        ] {
            let record = abandoned(reference, summary);
            assert!(stands_after("waiting-user", &record, false), "{summary}");
        }
        let lookalike = abandoned("", "idle 5m").replace("busy pane", "confirmation 1/2");
        assert!(
            stands_after("waiting-user", &lookalike, false),
            "{lookalike}"
        );
        for (reference, summary) in [
            ("launch-1", "confirmation 1/2"),
            ("", "confirmation 2/2"),
            ("", "waiting-agent confirmation 1/2"),
            ("launch-1", "blocked confirmation 1/2"),
        ] {
            let record = abandoned(reference, summary);
            assert!(!stands_after("waiting-user", &record, false), "{summary}");
        }
    }

    // ---- Quiet detection --------------------------------------------------
    //
    // PANE_SPECIMEN carries every nudge rendering the filter must strip, plus
    // the near-misses that must survive. AWK_ORACLE is the filtered result the
    // specimen is pinned against.

    const PANE_SPECIMEN: &str = r#"some ordinary prose from the agent

› ⟦ae:msg from watchdog⟧
  Status check: if you have more work, continue. Otherwise declare
  your state so I stop nudging: /Users/ckriech/.ae/sessions/aerewrite/
  state <waiting-user|blocked|done> "<reason>"
back to real output after the block
  ❯ ⟦ae:msg from watchdog⟧
  Session goal: ship P4.1. Status check: if you have more work,
  continue. Otherwise declare your state so I stop nudging:
  /Users/ckriech/.ae/sessions/aerewrite/state <waiting-user|blocked|done>
  "<reason>"
more real output
⟦ae:msg from watchdog⟧
Status check: if you have more work, continue. Otherwise declare your state so I stop nudging: /Users/ckriech/.ae/sessions/aerewrite/state <waiting-user|blocked|done> "<reason>"
⟦ae:msg from watchdog⟧
a human message that is not a nudge
Session goal: ship the watchdog. Status check: if you have more work, continue. Otherwise declare your state so I stop nudging: /Users/ckriech/.ae/sessions/aerewrite/state <waiting-user|blocked|done> "<reason>"
  └ Marked gpt56sol:reviewer done: wrapped it up
⏺ [21:28] Done — output: Marked opus5:builder waiting-user: needs review
Marked opus5:builder waiting-user: needs review
Marked opus5:builder working
I was asked: Status check: if you have more work, continue. Otherwise declare your state so I stop nudging: /x/state <waiting-user|blocked|done> "<reason>" and I replied
Marked opus5:builder sleeping
Marked two words done
the agent Marked opus5:builder done in passing
› ⟦ae:msg from watchdog⟧
  swallowed body line

  this indented line is AFTER the blank, so it survives
tail line
"#;

    const AWK_ORACLE: &str = r#"some ordinary prose from the agent

back to real output after the block
more real output
⟦ae:msg from watchdog⟧
a human message that is not a nudge
I was asked: Status check: if you have more work, continue. Otherwise declare your state so I stop nudging: /x/state <waiting-user|blocked|done> "<reason>" and I replied
Marked opus5:builder sleeping
Marked two words done
the agent Marked opus5:builder done in passing

  this indented line is AFTER the blank, so it survives
tail line
"#;

    /// The nudge exactly as the watchdog composes it, meta-dir path and
    /// all.
    const RAW_NUDGE: &str = "Status check: if you have more work, continue. \
         Otherwise declare your state so I stop nudging: \
         /Users/ckriech/.ae/sessions/aerewrite/state \
         <waiting-user|blocked|done> \"<reason>\"";

    #[test]
    fn the_frozen_awk_is_reproduced_byte_for_byte() {
        assert_eq!(
            quiet_filter(PANE_SPECIMEN),
            AWK_ORACLE,
            "the port drifted from the awk it is a port OF"
        );
    }

    #[test]
    fn awk_record_splitting_is_reproduced_at_the_edges() {
        // `printf '%s' ""` feeds awk no records at all.
        assert_eq!(quiet_filter(""), "", "empty input is empty output");
        // A record with no terminator is still a record, and `print` terminates it.
        assert_eq!(quiet_filter("no trailing newline"), "no trailing newline\n");
        assert_eq!(quiet_filter("a\nb\n"), "a\nb\n");
        // A blank line is a record of its own and survives.
        assert_eq!(quiet_filter("\n"), "\n");
        assert_eq!(quiet_filter("a\n\nb"), "a\n\nb\n");
    }

    #[test]
    fn the_submit_header_is_the_ornament_plus_the_envelope_alone() {
        for header in [
            "› ⟦ae:msg from watchdog⟧",      // codex, captured
            "❯ ⟦ae:msg from watchdog⟧",      // claude, captured
            "  ❯  ⟦ae:msg from watchdog⟧  ", // indented, padded, trailing space
            "\t›\t⟦ae:msg from watchdog⟧",   // tabs are `[[:space:]]` too
        ] {
            assert!(submit_hdr(header), "{header:?} is a rendered nudge header");
        }
        for other in [
            "❮ ⟦ae:msg from watchdog⟧",          // a DIFFERENT arrow
            "> ⟦ae:msg from watchdog⟧",          // ascii, not an ornament
            "›⟦ae:msg from watchdog⟧",           // `[[:space:]]+` needs one
            "› ⟦ae:msg from lead⟧",              // another sender
            "› ⟦ae:msg from watchdog⟧ and more", // not alone on its line
            "⟦ae:msg from watchdog⟧",            // the bare envelope form
            "",
        ] {
            assert!(!submit_hdr(other), "{other:?} is not a rendered header");
        }
    }

    #[test]
    fn only_two_leading_whitespace_characters_are_an_indent() {
        assert!(indented("  wrapped body"));
        assert!(indented("\t\tstill indented"));
        assert!(indented(" \tmixed"));
        assert!(!indented(" one space only"));
        assert!(!indented("flush left"));
        // A BLANK line is not indented — which is what ends a rendered block.
        assert!(!indented(""));
        assert!(!indented(" "));
    }

    #[test]
    fn the_raw_nudge_matches_with_and_without_the_session_goal_prefix() {
        assert!(raw_nudge(RAW_NUDGE));
        assert!(
            raw_nudge(&format!("{RAW_NUDGE}   ")),
            "trailing space is allowed"
        );
        assert!(raw_nudge(&format!("Session goal: ship P4.1. {RAW_NUDGE}")));
        // A goal that itself contains sentence punctuation: the awk backtracks
        // over every `". "` split, so the LAST one still lands on the sentence.
        assert!(raw_nudge(&format!(
            "Session goal: land it. then rest. {RAW_NUDGE}"
        )));
        for other in [
            format!("Goal: ship. {RAW_NUDGE}"),      // not the goal prefix
            format!("I was told: {RAW_NUDGE}"),      // arbitrary prose in front
            format!("{RAW_NUDGE} and I replied"),    // the tail must END the line
            RAW_NUDGE.replace("<reason>", "reason"), // the invitation is fixed text
            "Status check: if you have more work, continue.".to_owned(),
        ] {
            assert!(!raw_nudge(&other), "{other:?} is not a raw nudge");
        }
    }

    #[test]
    fn a_quoted_nudge_survives_because_it_has_neither_ornament_nor_envelope() {
        // The property the whole filter exists to preserve: an agent quoting the
        // nudge is real pane content, and must still wake the watchdog.
        let quoted = format!("I was asked: {RAW_NUDGE} and I answered");
        assert_eq!(quiet_filter(&quoted), format!("{quoted}\n"));
        assert_ne!(
            quiet_hash(&quoted),
            quiet_hash(""),
            "a quoted nudge is content, not a footprint"
        );
    }

    #[test]
    fn the_bare_envelope_is_held_and_dropped_only_when_the_nudge_follows() {
        // The pair form (unmodeled pane): both lines go.
        let pair = format!("before\n⟦ae:msg from watchdog⟧\n{RAW_NUDGE}\nafter\n");
        assert_eq!(quiet_filter(&pair), "before\nafter\n");
        // An envelope with anything else under it is a REAL watchdog message and
        // survives, envelope included.
        let lone = "before\n⟦ae:msg from watchdog⟧\nread the handover\n";
        assert_eq!(quiet_filter(lone), lone);
        // Held at end of input: the END block flushes it.
        assert_eq!(
            quiet_filter("before\n⟦ae:msg from watchdog⟧\n"),
            "before\n⟦ae:msg from watchdog⟧\n"
        );
    }

    #[test]
    fn a_rendered_block_swallows_its_indented_body_and_stops_at_the_first_flush_line() {
        let rendered = concat!(
            "real output\n",
            "› ⟦ae:msg from watchdog⟧\n",
            "  Status check: if you have more work, continue. Otherwise declare\n",
            "  your state so I stop nudging: /x/state <waiting-user|blocked|done>\n",
            "back to real output\n",
        );
        assert_eq!(quiet_filter(rendered), "real output\nback to real output\n");
        // A blank line ends the block — the swallow is bounded by the render.
        let blanked = "› ⟦ae:msg from watchdog⟧\n  body\n\n  later indented line\n";
        assert_eq!(quiet_filter(blanked), "\n  later indented line\n");
    }

    #[test]
    fn the_three_captured_echo_forms_are_dropped() {
        for echo in [
            "  └ Marked gpt56sol:reviewer done: wrapped it up", // codex
            "└ Marked gpt56sol:reviewer working",               // codex, no detail
            "⏺ [21:28] Done — output: Marked opus5:builder waiting-user: needs review", // claude
            "Marked opus5:builder waiting-user: needs review",  // unmodeled pane
            "Marked opus5:builder waiting-agent: on colead",    // the fifth state
            "Marked opus5:builder blocked.",                    // `.` remainder
            "Marked opus5:builder done",                        // bare
        ] {
            assert!(is_echo(echo), "{echo:?} is a state echo");
            assert_eq!(quiet_filter(echo), "", "{echo:?} must not reach the hash");
        }
    }

    #[test]
    fn echo_near_misses_survive_because_deafness_is_the_worse_failure() {
        for prose in [
            "Marked opus5:builder sleeping",       // not a state word
            "Marked two words done",               // `[^ ]+` cannot cross a space
            "the agent Marked opus5:builder done", // not anchored at the start
            "Marked opus5:builder doneish",        // remainder must start `:` or `.`
            "Marked  opus5:builder done",          // the agent field must be non-empty
            "⏺ [9:28] Done — output: Marked opus5:builder done", // HH must be two digits
            "⏺ [21:28] Done - output: Marked opus5:builder done", // ascii dash, not U+2014
            "└Marked opus5:builder done",          // the glyph needs whitespace after
        ] {
            assert!(!is_echo(prose), "{prose:?} is ordinary content");
            assert_eq!(quiet_filter(prose), format!("{prose}\n"));
        }
    }

    #[test]
    fn the_hash_is_stable_over_identical_content_and_moves_on_real_output() {
        let pane = "the agent is thinking\n";
        assert_eq!(quiet_hash(pane), quiet_hash(pane), "deterministic");
        assert_eq!(
            quiet_hash(pane),
            quiet_hash("the agent is thinking\n"),
            "equal content, equal hash"
        );
        assert_ne!(
            quiet_hash(pane),
            quiet_hash("the agent is thinking\nand answered\n"),
            "new content must move the hash — this is the YIELD signal"
        );
    }

    #[test]
    fn the_watchdogs_own_footprints_do_not_move_the_hash() {
        // The reason the filter exists: the watchdog must not wake itself up.
        let quiet_pane = "waiting on review\nMarked opus5:builder waiting-user: review\n";
        let after_nudge = format!(
            "{quiet_pane}› ⟦ae:msg from watchdog⟧\n  Status check: if you have more\n  work, continue.\n"
        );
        assert_eq!(
            quiet_hash(quiet_pane),
            quiet_hash(&after_nudge),
            "a delivered nudge and the state echo are footprints, not news"
        );
        // A human's reply IS news.
        let after_human = format!("{after_nudge}yes, please continue\n");
        assert_ne!(quiet_hash(quiet_pane), quiet_hash(&after_human));
    }

    // -- the orchestrator overview cadence -----------------------------------

    /// A fixed clock.
    const BASE: u64 = 1_700_000_000;

    fn at(offset_secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(BASE + offset_secs)
    }

    /// Stable 300s specimens for the cadence and wedge boundary tests.
    fn knobs() -> SweepKnobs {
        SweepKnobs {
            sweep_secs: 300,
            ..SweepKnobs::default()
        }
    }

    fn seen(now_offset: u64, last_done: Option<u64>, _k: &SweepKnobs) -> SweepObservation {
        SweepObservation::new(at(now_offset), last_done.map(at))
    }

    #[test]
    fn the_overview_defaults_to_a_two_minute_minimum_spacing() {
        let k = SweepKnobs::default();
        assert_eq!(k.sweep_secs, 120, "minimum spacing");
        assert_eq!(k.retry_secs, 30, "retry floor");
        assert_eq!(k.retry_max, 6, "retry ceiling");
        assert_eq!(k.wedge_secs(), 300, "SWEEP_SECS * 2 + 60");
        assert!(k.enabled());
    }

    #[test]
    fn a_delivered_overview_without_a_done_event_eventually_wedges() {
        let k = knobs();
        let prior = SweepState::default();
        let observed = seen(661, None, &k).with_overview(false, Some(at(0)), Some(at(0)));

        let booked = sweep_step(&prior, &observed, &k).expect("enabled");

        assert_eq!(booked.verdict, SweepVerdict::MetaWedged);
        assert!(
            booked
                .effects
                .contains(&SweepEffect::Alert(SweepAlert::RaiseWedge(
                    WedgeDetail::Never { deliveries: 1 }
                )))
        );
    }

    #[test]
    fn a_done_event_after_delivery_is_fresh_even_long_after_the_grace() {
        let k = knobs();
        let prior = SweepState {
            outstanding_since: Some(at(0)),
            unacknowledged_deliveries: 1,
            ..SweepState::default()
        };

        let booked = sweep_step(&prior, &seen(5_000, Some(1), &k), &k).expect("enabled");

        assert_eq!(booked.verdict, SweepVerdict::MetaSweeping);
        assert_eq!(booked.next.unacknowledged_deliveries, 0);
    }

    #[test]
    fn the_daemons_own_timestamps_clamp_rather_than_taking_a_distance() {
        // The SPLIT, pinned.
        let k = knobs();
        let jumped = SweepState {
            // Both stamped an hour ahead of the cycle clock.
            outstanding_since: Some(at(3600)),
            last_sweep: Some(at(3600)),
            ..SweepState::default()
        };
        let acc = sweep_step(&jumped, &seen(0, None, &k), &k).expect("enabled");
        assert_eq!(
            acc.verdict,
            SweepVerdict::MetaStarting,
            "a backwards clock jump must not read as an hour of elapsed grace"
        );
        assert!(
            !acc.effects
                .iter()
                .any(|e| matches!(e, SweepEffect::Alert(_))),
            "and it must raise nothing"
        );
        assert!(
            !acc.effects.contains(&SweepEffect::FireSweepNudge),
            "nor make the cadence due"
        );
    }

    #[test]
    fn only_the_orchestrator_main_slot_gets_the_cadence() {
        // Workers and spawned agents in an orchestrator session keep the normal
        // watchdog.
        assert!(is_sweep_target(true, "main"));
        for other in [
            "worker.1",
            "worker.0",
            "spawned.3",
            "Main",
            "main.1",
            " main",
        ] {
            assert!(
                !is_sweep_target(true, other),
                "{other:?} is not the orchestrator main slot"
            );
        }
        assert!(
            !is_sweep_target(false, "main"),
            "a session that is not the orchestrator has no sweep branch"
        );
        assert!(
            !is_sweep_target(true, ""),
            "an UNSTAMPED pane has no slot and keeps the ordinary watchdog"
        );
    }

    #[test]
    fn a_zero_cadence_removes_the_branch_rather_than_emptying_it() {
        // The disabled cadence, with its own control: the SAME inputs that
        // produce a wedge alert on a live cadence produce NO BRANCH at all
        // on `0` — the caller falls through to the normal watchdog.
        let prior = SweepState {
            outstanding_since: Some(at(0)),
            unacknowledged_deliveries: 1,
            ..SweepState::default()
        };
        let off = SweepKnobs {
            sweep_secs: 0,
            ..knobs()
        };
        assert!(!off.enabled());
        assert_eq!(
            sweep_step(&prior, &seen(5000, None, &off), &off),
            None,
            "sweep_secs 0 is not a branch"
        );

        let on = knobs();
        let acc = sweep_step(&prior, &seen(5000, None, &on), &on).expect("the control runs");
        assert_eq!(acc.verdict, SweepVerdict::MetaWedged);
        assert!(
            acc.effects
                .contains(&SweepEffect::Alert(SweepAlert::RaiseWedge(
                    WedgeDetail::Never { deliveries: 1 }
                ))),
            "the control proves the disabled case was disabled, not merely quiet"
        );
    }

    #[test]
    fn the_cadence_fires_on_the_first_cycle_and_then_on_the_window_boundary() {
        let k = knobs();
        let fresh = SweepState::default();
        let first = sweep_step(&fresh, &seen(0, None, &k), &k).expect("enabled");
        assert!(
            first.effects.contains(&SweepEffect::FireSweepNudge),
            "an absent last_sweep is bash's 0 — the first cycle prompts"
        );

        let prompted = SweepState {
            last_sweep: Some(at(0)),
            ..fresh
        };
        // The boundary flips the decision: 299s holds, 300s fires.
        let held = sweep_step(&prompted, &seen(299, None, &k), &k).expect("enabled");
        assert!(!held.effects.contains(&SweepEffect::FireSweepNudge));
        let due = sweep_step(&prompted, &seen(300, None, &k), &k).expect("enabled");
        assert!(due.effects.contains(&SweepEffect::FireSweepNudge));
    }

    #[test]
    fn an_unchanged_overview_never_spends_a_seat_turn() {
        let k = knobs();
        let seen = seen(900, Some(900), &k).with_overview(false, Some(at(0)), Some(at(0)));
        let booked = sweep_step(&SweepState::default(), &seen, &k).expect("enabled");
        assert!(
            !booked.effects.contains(&SweepEffect::FireSweepNudge),
            "elapsed cadence alone is not a reason to wake the seat"
        );
    }

    #[test]
    fn a_changed_overview_waits_for_the_persisted_minimum_spacing() {
        let k = knobs();
        let too_soon = seen(299, Some(299), &k).with_overview(true, Some(at(0)), Some(at(0)));
        assert!(
            !sweep_step(&SweepState::default(), &too_soon, &k)
                .expect("enabled")
                .effects
                .contains(&SweepEffect::FireSweepNudge)
        );

        let due = seen(300, Some(300), &k).with_overview(true, Some(at(0)), Some(at(0)));
        assert!(
            sweep_step(&SweepState::default(), &due, &k)
                .expect("enabled")
                .effects
                .contains(&SweepEffect::FireSweepNudge),
            "a changed overview fires exactly at the durable boundary"
        );
    }

    #[test]
    fn a_fresh_working_declaration_holds_a_changed_overview_without_booking_it() {
        let k = knobs();
        let prior = SweepState::default();
        let observed =
            seen(OVERVIEW_HOLD_WHILE_WORKING_SECS - 1, None, &k).with_working_since(Some(at(0)));

        let booked = sweep_step(&prior, &observed, &k).expect("enabled");

        assert!(
            !booked.effects.contains(&SweepEffect::FireSweepNudge),
            "human work owns the seat during the hold"
        );
        assert_eq!(
            booked.next, prior,
            "a held overview spends no delivery, retry, or spacing state"
        );
    }

    #[test]
    fn a_stale_working_declaration_no_longer_holds_the_overview() {
        let k = knobs();
        for age in [
            OVERVIEW_HOLD_WHILE_WORKING_SECS,
            OVERVIEW_HOLD_WHILE_WORKING_SECS + 1,
        ] {
            let observed = seen(age, None, &k).with_working_since(Some(at(0)));
            let booked = sweep_step(&SweepState::default(), &observed, &k).expect("enabled");

            assert!(
                booked.effects.contains(&SweepEffect::FireSweepNudge),
                "a stuck working seat must not starve the human at age {age}"
            );
        }
    }

    #[test]
    fn done_or_idle_does_not_hold_a_changed_overview() {
        let k = knobs();
        for last_done in [Some(0), None] {
            let observed = seen(1, last_done, &k);
            let booked = sweep_step(&SweepState::default(), &observed, &k).expect("enabled");
            assert!(booked.effects.contains(&SweepEffect::FireSweepNudge));
        }
    }

    #[test]
    fn an_undelivered_prompt_retries_fast_without_consuming_the_cadence_slot() {
        // The retry is a FLOOR: due 30s after the failure, on the first
        // poll at or after that point.
        let k = knobs();
        let mut state = SweepState {
            last_sweep: Some(at(0)),
            ..SweepState::default()
        };
        let effects = record_sweep(&mut state, false, at(302), &k);
        assert!(effects.is_empty(), "a fast retry escalates nothing");
        assert_eq!(state.fails, 1);
        assert_eq!(
            state.outstanding_since, None,
            "the acknowledgement grace never starts on an attempt"
        );

        // Scheduled off the SETTLED clock (302), not the cycle clock (300).
        let early = sweep_step(&state, &seen(331, None, &k), &k).expect("enabled");
        assert!(
            !early.effects.contains(&SweepEffect::FireSweepNudge),
            "one second before the retry floor"
        );
        let ready = sweep_step(&state, &seen(332, None, &k), &k).expect("enabled");
        assert!(
            ready.effects.contains(&SweepEffect::FireSweepNudge),
            "302 + 30 = 332"
        );
    }

    #[test]
    fn the_retry_interval_is_clamped_to_the_cadence() {
        // An unclamped 600s retry against a 300s cadence would push
        // last_sweep into the FUTURE and DELAY the next prompt to +600.
        let k = SweepKnobs {
            retry_secs: 600,
            ..knobs()
        };
        let mut state = SweepState::default();
        assert!(record_sweep(&mut state, false, at(0), &k).is_empty());
        assert!(
            !sweep_step(&state, &seen(299, None, &k), &k)
                .expect("enabled")
                .effects
                .contains(&SweepEffect::FireSweepNudge)
        );
        assert!(
            sweep_step(&state, &seen(300, None, &k), &k)
                .expect("enabled")
                .effects
                .contains(&SweepEffect::FireSweepNudge),
            "clamped to the cadence, not delayed to the unclamped 600"
        );
    }

    #[test]
    fn past_the_retry_maximum_the_branch_alerts_once_and_returns_to_the_cadence() {
        // The retry ceiling.
        let k = knobs();
        let mut state = SweepState::default();
        for attempt in 1..=k.retry_max {
            let effects = record_sweep(&mut state, false, at(0), &k);
            assert!(effects.is_empty(), "fast retry {attempt} escalates nothing");
            assert!(!state.unreachable_alerted);
        }
        let escalation = record_sweep(&mut state, false, at(0), &k);
        assert_eq!(
            escalation,
            vec![SweepEffect::Alert(SweepAlert::RaiseUnreachable {
                undelivered: 7
            })]
        );
        assert!(state.unreachable_alerted);
        assert_eq!(
            state.last_sweep,
            Some(at(0)),
            "back to the normal cadence — no more back-dating"
        );

        // ONE alert: the next failure escalates nothing.
        assert!(
            record_sweep(&mut state, false, at(0), &k).is_empty(),
            "the unreachable alert is raised once per run"
        );

        // Cleared on a landed delivery.
        let cleared = record_sweep(&mut state, true, at(901), &k);
        assert_eq!(
            cleared,
            vec![SweepEffect::Alert(SweepAlert::ClearUnreachable)]
        );
        assert!(!state.unreachable_alerted);
        assert_eq!(state.fails, 0);
        assert_eq!(
            state.last_sweep,
            Some(at(901)),
            "a landed prompt schedules off the successful submit clock"
        );
        assert_eq!(state.outstanding_since, Some(at(901)));
        assert_eq!(state.unacknowledged_deliveries, 1);
    }

    #[test]
    fn a_retry_maximum_of_zero_escalates_on_the_first_failure() {
        // `^(0|[1-9][0-9]*)$` accepts 0, so the branch has to survive it: no
        // fast retry at all, straight to the bounded cadence.
        let k = SweepKnobs {
            retry_max: 0,
            ..knobs()
        };
        let mut state = SweepState::default();
        assert_eq!(
            record_sweep(&mut state, false, at(0), &k),
            vec![SweepEffect::Alert(SweepAlert::RaiseUnreachable {
                undelivered: 1
            })]
        );
        assert_eq!(state.last_sweep, Some(at(0)));
    }

    #[test]
    fn the_unreachable_clear_is_withheld_while_a_wedge_alert_is_still_latched() {
        // `alert-cleared` is untyped: emitting one here would erase a live "not
        // sweeping" that could then never re-fire.
        let k = knobs();
        let mut state = SweepState {
            unreachable_alerted: true,
            wedge_alerted: true,
            ..SweepState::default()
        };
        assert!(
            record_sweep(&mut state, true, at(0), &k).is_empty(),
            "reachable again, but the wedge alert owns its own clear"
        );
        assert!(!state.unreachable_alerted);
        assert!(state.wedge_alerted);
    }

    #[test]
    fn repeated_deliveries_keep_the_oldest_unacknowledged_deadline_and_eventually_wedge() {
        let k = knobs();
        let mut state = SweepState::default();
        assert!(record_sweep(&mut state, true, at(10), &k).is_empty());
        assert_eq!(state.outstanding_since, Some(at(10)), "first delivery");
        assert_eq!(state.unacknowledged_deliveries, 1);
        assert!(record_sweep(&mut state, true, at(310), &k).is_empty());
        assert_eq!(
            state.outstanding_since,
            Some(at(10)),
            "another overview cannot reset the acknowledgement grace"
        );
        assert_eq!(state.unacknowledged_deliveries, 2);

        let booked = sweep_step(&state, &seen(671, None, &k), &k).expect("enabled");
        assert_eq!(booked.verdict, SweepVerdict::MetaWedged);
        assert!(
            booked
                .effects
                .contains(&SweepEffect::Alert(SweepAlert::RaiseWedge(
                    WedgeDetail::Never { deliveries: 2 }
                )))
        );
    }

    #[test]
    fn a_deferred_delivery_uses_submit_time_for_acknowledgement_and_spacing() {
        let k = knobs();
        let mut state = SweepState::default();

        // The cycle began at t=0, a done event landed at t=90, and the
        // delivery did not actually settle until t=100.
        assert!(record_sweep(&mut state, true, at(100), &k).is_empty());
        assert_eq!(state.last_sweep, Some(at(100)));
        assert_eq!(state.outstanding_since, Some(at(100)));

        let before_spacing = sweep_step(&state, &seen(399, Some(90), &k), &k).expect("enabled");
        assert_eq!(before_spacing.verdict, SweepVerdict::MetaStarting);
        assert!(
            !before_spacing
                .effects
                .contains(&SweepEffect::FireSweepNudge),
            "spacing is anchored at the successful submit, not cycle start"
        );
        let due = sweep_step(&state, &seen(400, Some(90), &k), &k).expect("enabled");
        assert!(due.effects.contains(&SweepEffect::FireSweepNudge));
    }

    #[test]
    fn a_restart_cannot_ack_a_deferred_second_delivery_with_an_earlier_done() {
        let k = knobs();
        let mut before_restart = SweepState::default();
        assert!(record_sweep(&mut before_restart, true, at(0), &k).is_empty());

        // A later changed overview was due at t=120. While its submit was
        // deferred, the first turn acknowledged at t=150; the second delivery
        // did not actually settle until t=200.
        assert!(record_sweep(&mut before_restart, true, at(200), &k).is_empty());
        assert_eq!(before_restart.outstanding_since, Some(at(0)));
        assert_eq!(before_restart.last_sweep, Some(at(200)));
        assert_eq!(before_restart.last_delivered, Some(at(200)));

        // Lose all in-memory accounting to model a daemon restart. The two
        // distinct durable clocks are the only delivery evidence left.
        let restarted = SweepState::default();
        let persisted = seen(201, Some(150), &k).with_overview(
            false,
            before_restart.outstanding_since,
            before_restart.last_delivered,
        );
        let booked = sweep_step(&restarted, &persisted, &k).expect("enabled");
        assert_eq!(
            booked.verdict,
            SweepVerdict::MetaStarting,
            "a done before the latest delivery cannot acknowledge it"
        );
        assert_eq!(booked.next.outstanding_since, Some(at(0)));
        assert_eq!(booked.next.unacknowledged_deliveries, 1);

        let overdue = seen(661, Some(150), &k).with_overview(
            false,
            before_restart.outstanding_since,
            before_restart.last_delivered,
        );
        assert_eq!(
            sweep_step(&booked.next, &overdue, &k)
                .expect("enabled")
                .verdict,
            SweepVerdict::MetaWedged,
            "the oldest outstanding deadline still governs the wedge"
        );
    }

    #[test]
    fn the_wedge_raises_once_past_the_grace_and_the_boundary_is_strict() {
        // The grace runs from the oldest UNACKNOWLEDGED overview; the
        // comparison is `>`, so an elapsed exactly at the window is still
        // starting up.
        let k = knobs();
        let prior = SweepState {
            outstanding_since: Some(at(0)),
            unacknowledged_deliveries: 1,
            ..SweepState::default()
        };
        let edge = sweep_step(&prior, &seen(660, None, &k), &k).expect("enabled");
        assert_eq!(edge.verdict, SweepVerdict::MetaStarting);
        assert!(
            !edge
                .effects
                .iter()
                .any(|e| matches!(e, SweepEffect::Alert(SweepAlert::RaiseWedge(_)))),
            "no liveness claim is invented inside the grace"
        );

        let over = sweep_step(&prior, &seen(661, None, &k), &k).expect("enabled");
        assert_eq!(over.verdict, SweepVerdict::MetaWedged);
        assert!(
            over.effects
                .contains(&SweepEffect::Alert(SweepAlert::RaiseWedge(
                    WedgeDetail::Never { deliveries: 1 }
                ))),
            "one second past the window flips the decision"
        );
        assert!(over.next.wedge_alerted);

        // ONE alert per wedge.
        let again = sweep_step(&over.next, &seen(1200, None, &k), &k).expect("enabled");
        assert_eq!(again.verdict, SweepVerdict::MetaWedged);
        assert!(
            !again
                .effects
                .iter()
                .any(|e| matches!(e, SweepEffect::Alert(_))),
            "the wedge alert is raised once, not once per cycle"
        );
    }

    #[test]
    fn an_old_done_and_no_done_wedge_with_different_details() {
        let k = knobs();
        let prior = SweepState {
            outstanding_since: Some(at(100)),
            unacknowledged_deliveries: 2,
            ..SweepState::default()
        };
        let stalled = sweep_step(&prior, &seen(800, Some(50), &k), &k).expect("enabled");
        assert_eq!(stalled.verdict, SweepVerdict::MetaWedged);
        assert!(
            stalled
                .effects
                .contains(&SweepEffect::Alert(SweepAlert::RaiseWedge(
                    WedgeDetail::Stalled { age_secs: 700 }
                )))
        );
        let never = sweep_step(&prior, &seen(800, None, &k), &k).expect("enabled");
        assert!(
            never
                .effects
                .contains(&SweepEffect::Alert(SweepAlert::RaiseWedge(
                    WedgeDetail::Never { deliveries: 2 }
                )))
        );
    }

    #[test]
    fn a_done_after_delivery_clears_a_latched_wedge_and_reports_sweeping() {
        let k = knobs();
        let wedged = SweepState {
            outstanding_since: Some(at(700)),
            unacknowledged_deliveries: 1,
            last_sweep: Some(at(700)),
            wedge_alerted: true,
            ..SweepState::default()
        };
        let recovered = sweep_step(&wedged, &seen(800, Some(790), &k), &k).expect("enabled");
        assert_eq!(recovered.verdict, SweepVerdict::MetaSweeping);
        assert_eq!(
            recovered.effects,
            vec![SweepEffect::Alert(SweepAlert::ClearWedge)],
            "the watchdog raised it, so the watchdog clears it"
        );
        assert!(!recovered.next.wedge_alerted);
        assert!(
            recovered.next.reconciled,
            "an in-memory clear also settles the durable reconcile"
        );

        // Idempotent: no second clear.
        let steady = sweep_step(&recovered.next, &seen(900, Some(790), &k), &k).expect("enabled");
        assert!(
            !steady
                .effects
                .iter()
                .any(|e| matches!(e, SweepEffect::Alert(_) | SweepEffect::ReconcileWedge))
        );
    }

    #[test]
    fn the_durable_reconcile_is_offered_once_per_daemon_lifetime() {
        // A watchdog restarted after alerting has lost the latch, so the first
        // acknowledged overview has to reach for the event log — once.
        let k = knobs();
        let restarted = SweepState::default();
        let first_seen = seen(1, Some(1), &k).with_overview(true, Some(at(0)), Some(at(0)));
        let first = sweep_step(&restarted, &first_seen, &k).expect("enabled");
        assert_eq!(first.verdict, SweepVerdict::MetaSweeping);
        assert!(first.effects.contains(&SweepEffect::ReconcileWedge));
        assert!(first.next.reconciled);

        let second_seen = seen(400, Some(1), &k).with_overview(true, Some(at(0)), Some(at(0)));
        let second = sweep_step(&first.next, &second_seen, &k).expect("enabled");
        assert!(
            !second.effects.contains(&SweepEffect::ReconcileWedge),
            "the log is read lazily, once"
        );
    }

    #[test]
    fn the_roster_glyphs_and_alert_texts_are_the_frozen_ones() {
        assert_eq!(SweepVerdict::MetaSweeping.glyph(), "👁");
        assert_eq!(SweepVerdict::MetaWedged.glyph(), "◌");
        assert_eq!(SweepVerdict::MetaStarting.glyph(), "·");

        let wedge = SweepAlert::RaiseWedge(WedgeDetail::Stalled { age_secs: 700 });
        assert_eq!(wedge.action(), "alert");
        assert_eq!(
            wedge.summary(),
            "meta-agent not acknowledging overviews — oldest outstanding overview \
             unacknowledged for 11m (may be stuck)"
        );
        assert_eq!(
            wedge.notify(),
            Some("(meta-agent) not acknowledging overviews — may be stuck")
        );
        for text in [
            wedge.summary(),
            SweepAlert::RaiseUnreachable { undelivered: 7 }.summary(),
        ] {
            for banned in ["throttl", "dead", "missing"] {
                assert!(
                    !text.contains(banned),
                    "{text:?} must not carry {banned:?} — it would outrank the stale alert class"
                );
            }
        }
        assert_eq!(
            SweepAlert::RaiseWedge(WedgeDetail::Never { deliveries: 3 }).summary(),
            "meta-agent not acknowledging overviews — 3 delivered, zero done events (may be \
             stuck)"
        );
        let clear = SweepAlert::ClearWedge;
        assert_eq!(clear.action(), "alert-cleared");
        assert_eq!(
            clear.summary(),
            "meta-agent acknowledging overviews again (done received)"
        );
        assert_eq!(clear.notify(), None);
        let unreachable = SweepAlert::RaiseUnreachable { undelivered: 7 };
        assert_eq!(unreachable.action(), "alert");
        assert_eq!(
            unreachable.summary(),
            "meta-agent unreachable — 7 sweep nudges undelivered (not sweeping)"
        );
        assert_eq!(
            unreachable.notify(),
            Some("(meta-agent) unreachable — sweep nudges undelivered")
        );
        let reachable = SweepAlert::ClearUnreachable;
        assert_eq!(reachable.action(), "alert-cleared");
        assert_eq!(
            reachable.summary(),
            "meta-agent reachable again (sweep nudge delivered)"
        );
        assert_eq!(reachable.notify(), None);
    }

    #[test]
    fn only_a_done_at_or_after_delivery_is_reported_as_sweeping() {
        let k = knobs();
        let prior = SweepState {
            outstanding_since: Some(at(1000)),
            unacknowledged_deliveries: 1,
            ..SweepState::default()
        };
        let cases: [(Option<u64>, u64, SweepVerdict); 5] = [
            (None, 1100, SweepVerdict::MetaStarting),
            (None, 1660, SweepVerdict::MetaStarting),
            (None, 1661, SweepVerdict::MetaWedged),
            (Some(999), 2000, SweepVerdict::MetaWedged),
            (Some(1000), 5000, SweepVerdict::MetaSweeping),
        ];
        for (done, now, want) in cases {
            let acc = sweep_step(&prior, &seen(now, done, &k), &k).expect("enabled");
            assert_eq!(acc.verdict, want, "done={done:?} now={now}");
        }
    }

    /// The frozen agy frames, read from the repository so a pin cannot drift
    /// from the UI it claims to describe.
    #[expect(
        clippy::disallowed_methods,
        reason = "a fixture read in TEST code; the capability boundary in \
                  tests/it/phase3.rs inventories PRODUCT lines only"
    )]
    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("tests/fixtures/agy-composer/{name}.txt"))
            .unwrap_or_else(|why| panic!("the {name} fixture should be readable: {why}"))
    }

    /// The modal rows, as the frozen frame draws them TOP TO BOTTOM: question,
    /// then the options, then the key hint.
    const MODAL: &[&str] = &[
        "Do you trust the contents of this project?",
        "",
        "Antigravity CLI requires permission to read, edit, and execute files here.",
        "",
        "> Yes, I trust this folder",
        "  No, exit",
        "",
        "  ↑/↓ Navigate · enter Confirm",
        "Gemini 3.8 Flash · high",
    ];

    /// agy's window, read from its own adapter row.
    const HUMAN_PROMPT_WINDOW: usize = match crate::tool::ToolKind::Agy.adapter().prompt {
        Some(spec) => spec.window,
        None => 0,
    };

    /// The detector as the watchdog calls it.
    fn classify(buf: &str, agent_bin: &str) -> Option<super::HumanPrompt> {
        super::human_prompt_class(buf, agent_bin)
    }

    fn buffer(rows: &[&str]) -> String {
        rows.join("\n")
    }

    /// The REAL frame, and the text the seat's human needs. Notify says what to
    /// press, so the returned rows are the product, not a side effect.
    #[test]
    fn the_real_agy_trust_modal_is_named_with_the_question_and_the_keys_to_press() {
        let found = classify(&fixture("agy-trust-modal-frame"), "agy");
        let found = found.expect("the frozen trust modal should classify");
        assert_eq!(found.question, "Do you trust the contents of this project?");
        assert_eq!(found.keys, "↑/↓ Navigate · enter Confirm");
    }

    /// EVERY other frozen agy frame, including the two DRAFT frames where a
    /// human has already typed. A composer is not a prompt ae may not answer.
    #[test]
    fn no_composed_or_booting_agy_frame_is_ever_read_as_a_human_only_prompt() {
        for name in [
            "agy-composed-frame",
            "agy-composed-frame-80x24",
            "agy-draft-frame-80x24",
            "agy-wrapped-draft-frame-80x24",
            "agy-boot-frame",
        ] {
            assert_eq!(
                classify(&fixture(name), "agy"),
                None,
                "{name} must never read as a human-only prompt"
            );
        }
    }

    /// The TOOL GATE, on the byte-identical buffer: only the binary differs.
    #[test]
    fn the_same_modal_under_another_binary_is_not_a_human_only_prompt() {
        let modal = fixture("agy-trust-modal-frame");
        assert!(classify(&modal, "agy").is_some(), "agy does");
        for other in ["claude", "codex", "gemini", "opencode", "grok", ""] {
            assert_eq!(
                classify(&modal, other),
                None,
                "{other} must not classify the identical buffer"
            );
        }
    }

    /// The ANCHOR, and why it is the load-bearing half. The whole shape sits in
    /// SCROLLBACK — a transcript that DISCUSSES the modal — with ordinary rows
    /// trailing it. Without the window this classifies, so the pin is what
    /// makes dropping the anchor fail.
    #[test]
    fn the_whole_shape_in_scrollback_under_ordinary_rows_is_not_a_prompt() {
        let mut rows: Vec<&str> = MODAL.to_vec();
        rows.extend(vec![
            "and then the agent explained what the modal had said";
            HUMAN_PROMPT_WINDOW
        ]);
        assert_eq!(classify(&buffer(&rows), "agy"), None);
        // Same rows, same order — only the DISTANCE from the bottom differs.
        assert!(classify(&buffer(MODAL), "agy").is_some());
    }

    /// The window's exact edge, in both directions, so its value is a fact and
    /// not a spare margin.
    #[test]
    fn a_modal_at_the_window_edge_latches_and_one_row_past_it_does_not() {
        let pad = |extra: usize| {
            let mut rows: Vec<&str> = MODAL.to_vec();
            while rows.len() < HUMAN_PROMPT_WINDOW + extra {
                rows.push("status row");
            }
            buffer(&rows)
        };
        assert!(
            classify(&pad(0), "agy").is_some(),
            "a question on the window's first row still counts"
        );
        assert_eq!(
            classify(&pad(1), "agy"),
            None,
            "one row further up it is out of the window"
        );
    }

    /// The FENCE guard, scoped to the window. The modal shape is drawn INSIDE a
    /// composer here, which no frozen frame does — so this is the pin that
    /// makes removing the composer check fail rather than pass by luck.
    #[test]
    fn the_modal_shape_drawn_inside_a_live_composer_is_not_a_human_only_prompt() {
        let fence = "─".repeat(80);
        let rows = vec![
            "Do you trust the contents of this project?",
            "> Yes, I trust this folder",
            "  No, exit",
            "  ↑/↓ Navigate · enter Confirm",
            &fence,
            ">",
            &fence,
            "? for shortcuts                       Gemini 3.8 Flash · high",
        ];
        assert_eq!(classify(&buffer(&rows), "agy"), None);
    }

    /// The SHAPE, one part removed at a time: each is required, none is
    /// decorative.
    #[test]
    fn a_prompt_missing_any_one_of_its_parts_is_not_classified() {
        let without = |drop: &str| {
            let rows: Vec<&str> = MODAL.iter().copied().filter(|row| *row != drop).collect();
            classify(&buffer(&rows), "agy")
        };
        assert_eq!(without("Do you trust the contents of this project?"), None);
        assert_eq!(without("  ↑/↓ Navigate · enter Confirm"), None);
        assert_eq!(without("> Yes, I trust this folder"), None, "none selected");
        // The SELECTED row is the load-bearing discriminator; a sibling that is
        // really prose still counts, which errs toward naming a seat rather
        // than leaving a human waiting on one nobody mentions.
        assert!(
            without("  No, exit").is_some(),
            "the description is a sibling"
        );
    }

    /// ONE option is not a choice. Every MODAL fixture carries a description
    /// row that counts as a sibling, so without this the sibling check could be
    /// deleted and every other pin would stay green — the check is the only
    /// thing standing between a question-plus-one-line and a named seat.
    #[test]
    fn a_question_with_a_single_option_row_and_no_sibling_is_not_a_prompt() {
        let rows = [
            "Do you trust the contents of this project?",
            "> Yes, I trust this folder",
            "  ↑/↓ Navigate · enter Confirm",
            "Gemini 3.8 Flash · high",
        ];
        assert_eq!(classify(&buffer(&rows), "agy"), None);
        // The SAME frame with one sibling restored is a choice, so the pin is
        // about the sibling and not about the rest of the shape.
        let with_sibling = [
            "Do you trust the contents of this project?",
            "> Yes, I trust this folder",
            "  No, exit",
            "  ↑/↓ Navigate · enter Confirm",
            "Gemini 3.8 Flash · high",
        ];
        assert!(classify(&buffer(&with_sibling), "agy").is_some());
    }

    /// The selection TRAVELS: a human pressing ↓ puts `>` on the last option,
    /// where there is no row beneath it. Reading the sibling on one side only
    /// would lose the modal exactly when someone is working through it.
    #[test]
    fn a_modal_whose_selection_sits_on_its_last_option_is_still_a_prompt() {
        let moved: Vec<&str> = MODAL
            .iter()
            .map(|row| match *row {
                "> Yes, I trust this folder" => "  Yes, I trust this folder",
                "  No, exit" => "> No, exit",
                other => other,
            })
            .collect();
        let found = classify(&buffer(&moved), "agy");
        assert!(found.is_some(), "selection on the last option still counts");
        assert_eq!(
            found.map(|prompt| prompt.keys).unwrap_or_default(),
            "↑/↓ Navigate · enter Confirm"
        );
    }

    /// The PAIRING, over a window that carries its own `?` row above the modal.
    /// A single-shot first match pairs that row with the real hint, finds no
    /// selected option between them, and reports nothing — losing a live modal
    /// because of text that merely sits above it.
    #[test]
    fn a_question_row_above_the_modal_does_not_hide_the_modal_below_it() {
        // The leaked hint literal is what makes this bite: the chatter question
        // pairs with IT, encloses no selected row, and a single-shot match
        // stops there — with the real modal sitting right underneath.
        let mut rows = vec![
            "so I asked it, do you want me to continue?",
            "it said to press ↑/↓ Navigate · enter Confirm at the modal",
        ];
        rows.extend_from_slice(MODAL);
        let found = classify(&buffer(&rows), "agy");
        let found = found.expect("the modal below the chatter still classifies");
        assert_eq!(found.keys, "↑/↓ Navigate · enter Confirm");
    }

    /// A claude frame from the repository: the measured trust modal, or a
    /// harness frame whose composer is drawn.
    #[expect(
        clippy::disallowed_methods,
        reason = "a fixture read in TEST code; the capability boundary in \
                  tests/it/phase3.rs inventories PRODUCT lines only"
    )]
    fn claude_frame(path: &str) -> String {
        std::fs::read_to_string(format!("tests/fixtures/{path}"))
            .unwrap_or_else(|why| panic!("the {path} fixture should be readable: {why}"))
    }

    /// The detector as the watchdog calls it for a claude seat.
    fn claude(buf: &str) -> Option<super::HumanPrompt> {
        super::human_prompt_class(buf, "claude")
    }

    /// claude's window, read from its own adapter row.
    const CLAUDE_WINDOW: usize = match crate::tool::ToolKind::Claude.adapter().prompt {
        Some(spec) => spec.window,
        None => 0,
    };

    /// The REAL claude frames at both measured sizes. The question row runs on
    /// into the next sentence wherever the width wraps it, so what is reported
    /// is the question through its `?`, the same at every width.
    #[test]
    fn the_real_claude_trust_modal_is_named_at_both_measured_sizes() {
        for name in ["80x24", "200x50"] {
            let frame = claude_frame(&format!("claude-trust/claude-trust-modal-{name}.txt"));
            let found = claude(&frame).unwrap_or_else(|| panic!("{name} should classify"));
            assert_eq!(
                found.question,
                "Quick safety check: Is this a project you created or one you trust?",
                "{name}"
            );
            assert_eq!(found.keys, "Enter to confirm · Esc to cancel", "{name}");
        }
    }

    /// The TOOL GATE both ways: claude's modal under any other binary, and
    /// agy's modal under claude, are not prompts.
    #[test]
    fn the_claude_modal_is_named_only_for_the_claude_binary() {
        let modal = claude_frame("claude-trust/claude-trust-modal-80x24.txt");
        assert!(claude(&modal).is_some(), "claude does");
        for other in [
            "agy", "codex", "gemini", "opencode", "grok", "muse", "claude-2", "",
        ] {
            assert_eq!(
                super::human_prompt_class(&modal, other),
                None,
                "{other} must not classify the identical buffer"
            );
        }
        assert_eq!(claude(&fixture("agy-trust-modal-frame")), None);
    }

    /// Composed claude frames, idle and busy: a composer is not a prompt.
    #[test]
    fn no_composed_claude_frame_is_read_as_a_human_only_prompt() {
        for name in [
            "claude-idle-167x40",
            "claude-resumed-idle-149x37",
            "claude-busy-167x40",
            "claude-busy-nbsp-tip-101x41",
        ] {
            assert_eq!(
                claude(&claude_frame(&format!("harness-state/{name}.txt"))),
                None,
                "{name}"
            );
        }
    }

    /// The whole modal, anchor included, QUOTED above a live claude composer.
    /// Only the composer guard stands between this and a named seat.
    #[test]
    fn the_claude_modal_quoted_above_a_live_composer_is_not_a_prompt() {
        let rule = "─".repeat(80);
        let modal = [
            rule.as_str(),
            " Accessing workspace:",
            " Quick safety check: Is this a project you created or one you trust?",
            " ❯ No, exit",
            "   Yes, I trust this folder",
            " Enter to confirm · Esc to cancel",
        ];
        let composer = [rule.as_str(), "❯ ", rule.as_str(), "  🧠 Opus 5.5 (xhigh)"];
        assert!(claude(&buffer(&modal)).is_some(), "the modal alone is one");
        let quoted: Vec<&str> = modal.iter().chain(composer.iter()).copied().collect();
        assert_eq!(claude(&buffer(&quoted)), None);
    }

    /// The ANCHOR. Question, selection, sibling and hint are all prose a
    /// transcript can carry with no composer under it; the title directly
    /// under a full-width rule is the modal's own. Each case breaks it once.
    #[test]
    fn a_claude_modal_without_its_title_under_a_full_width_rule_is_not_a_prompt() {
        let rule = "─".repeat(80);
        let indented = format!("  {}", "─".repeat(78));
        let short = "─".repeat(40);
        let title = " Accessing workspace:";
        let body = [
            " Quick safety check: Is this a project you created or one you trust? (Like your",
            " ❯ No, exit",
            "   Yes, I trust this folder",
            "",
            " Enter to confirm · Esc to cancel",
        ];
        let with = |head: &[&str]| {
            let rows: Vec<&str> = head.iter().chain(body.iter()).copied().collect();
            claude(&buffer(&rows))
        };
        assert!(with(&[&rule, title]).is_some(), "the anchored shape is one");
        assert_eq!(with(&[]), None, "no title and no rule");
        assert_eq!(with(&[title]), None, "a title under no rule");
        assert_eq!(with(&[&rule]), None, "a rule with no title");
        assert_eq!(with(&[&rule, "", title]), None, "a rule not DIRECTLY above");
        assert_eq!(with(&[&indented, title]), None, "an indented rule");
        assert_eq!(with(&[&short, title]), None, "a rule narrower than a row");
        let below = [body[0], &rule, title, body[1], body[2], body[4]];
        assert_eq!(claude(&buffer(&below)), None, "a title below the question");
    }

    /// claude's WINDOW, measured: rule to hint is 17 rows at 80x24, and the
    /// rule — the anchor's top — must sit inside the window. At its first row
    /// it latches; one row further up it does not.
    #[test]
    fn a_claude_modal_whose_rule_is_on_the_window_edge_latches_and_one_row_past_it_does_not() {
        let frame = claude_frame("claude-trust/claude-trust-modal-80x24.txt");
        let rows: Vec<&str> = frame.lines().collect();
        let rule = rows.iter().position(|row| row.starts_with('─'));
        let hint = rows.iter().rposition(|row| !row.trim().is_empty());
        let (Some(rule), Some(hint)) = (rule, hint) else {
            panic!("the fixture carries a rule and a hint");
        };
        assert_eq!(hint - rule + 1, 17, "the measured span");
        let pad = |extra: usize| {
            let mut modal = rows[rule..=hint].to_vec();
            while modal.len() < CLAUDE_WINDOW + extra {
                modal.push("status row");
            }
            buffer(&modal)
        };
        assert!(
            claude(&pad(0)).is_some(),
            "the rule on the window's first row"
        );
        assert_eq!(claude(&pad(1)), None, "one row further up it is out");
    }
}
