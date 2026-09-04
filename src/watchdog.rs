//! The watchdog's per-agent decisions — the pure classification the daemon loop
//! makes each cycle over normalized pane observations.
//!
//! No tmux, no I/O, no clock: the loop gathers observations (a pane's foreground
//! command, whether an agent process runs beneath it, its recent rendered output)
//! and delivers effects (a nudge through the session's own `send` helper, the
//! tmux status options); THIS module only decides. That split is P4.1's clean cut
//! — Rust owns the interpretation and the decisions, bash keeps the daemon
//! start/stop glue and the pane/tmux delivery.
//!
//! The behavior authority is the frozen **bash** watchdog (`ae`'s
//! `command_is_shell`, `_pane_agent_is_dead`/`_pane_has_descendant_named`,
//! `_buf_shows_throttle`, `_watchdog_quiet_hash`), NOT `contrib/aewatch`'s Python
//! cycle — which the mapping found lacks the bash quiet-stabilization budget and
//! is not a byte-for-byte oracle. Each ported rule cites its bash site.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::events::Event;
use crate::procs::Descendancy;

/// The shells the dead-check treats as "no agent in the foreground" — bash's
/// `command_is_shell` (ae:428). The empty string counts: a pane momentarily
/// between processes reports no command, and that is not an agent either.
#[must_use]
pub fn command_is_shell(cmd: &str) -> bool {
    matches!(cmd, "bash" | "zsh" | "fish" | "sh" | "dash" | "")
}

/// Whether a pane's agent has DIED — dropped to a bare shell with no agent
/// process beneath it. (The watchdog's branch 1, ae:16503-16521.)
///
/// The two-part guard is bash's: the foreground must be a known shell
/// ([`command_is_shell`]) AND no process named by the slot's `agent_bin` may run
/// as a descendant of the pane's pid. Both halves are required because a
/// `bash -lc <tool>` wrapper shows a shell in the foreground while the real agent
/// runs underneath — dropping the descendant half read every such live agent as
/// dead (the measured macOS regression the bash comment records).
///
/// # The third state is a DIVERGENCE from bash, not a port of it
///
/// bash has no `Unknown`. `_pane_has_descendant_named` (ae:16228-16247) answers
/// "no descendant" for an empty pane pid, an empty agent binary, a failed
/// `pgrep` and a failed `ps` alike — so a probe that could not RUN is
/// indistinguishable from one that ran and found nothing, and the agent is
/// alerted DEAD either way.
///
/// The colead's binding for this port makes an unusable snapshot
/// [`Descendancy::Unknown`], and Unknown is NOT dead. The direction is the whole
/// point: a missed dead is silent and self-heals on the next cycle, while a
/// FALSE dead raises a process-died alert against a live agent and latches it.
#[must_use]
pub fn classify_dead(current_command: &str, descendant: Descendancy) -> bool {
    command_is_shell(current_command) && matches!(descendant, Descendancy::Absent)
}

/// The throttle phrases keyed by agent BINARY — the catalogs of bash's
/// `_buf_shows_throttle` (ae:16256-16301), at MODULE level rather than inside
/// [`shows_throttle`]: a `const` declared after that function's empty-buffer
/// guard is `clippy::items_after_statements`, and this crate gates on
/// `-D warnings`.
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

/// Whether the captured pane buffer shows upstream throttling for the agent whose
/// binary is `agent_bin` — bash's `_buf_shows_throttle` (ae:16256-16301).
///
/// The catalogs are deliberately narrow phrases, keyed by the agent BINARY name
/// (never the user-chosen alias). `opencode` is the full union because it is a TUI
/// over configurable providers; the generic `429`/`503` pair applies to every
/// tool, and an unknown binary matches only those generics. An empty buffer is
/// never throttled.
#[must_use]
pub fn shows_throttle(buf: &str, agent_bin: &str) -> bool {
    if buf.is_empty() {
        return false;
    }
    // opencode is the union — refactor here, never duplicate, so the branches
    // cannot drift (the bash comment's rule, kept).
    let tool: &[&[&str]] = match agent_bin {
        "claude" => &[CLAUDE],
        "codex" => &[CODEX],
        "gemini" => &[GEMINI],
        "opencode" => &[CLAUDE, CODEX, GEMINI],
        _ => &[],
    };
    tool.iter()
        .flat_map(|set| set.iter())
        .chain(GENERIC.iter())
        .any(|pattern| buf.contains(pattern))
}

/// Whether an agent is STALE — the composite the watchdog's branches 4, 5 and 6
/// have to ALL decline before branch 7 fires (ae:16821-16867).
///
/// Read the bash as a sieve: each earlier branch `continue`s, so "stale" is what
/// is left when every escape has been refused.
///
/// * `is_quiet` — branch 2 already skipped a held quiet state (ae:16739). The
///   caller passes the RESOLVED answer: `done` is event-only quiet, while
///   `waiting-user`/`blocked` are quiet only while their pane-hash baseline
///   holds, so a yielded declaration arrives here as `false`.
/// * `is_throttled` — branch 3 (ae:16787). Throttling is upstream's fault and a
///   nudge cannot fix it.
/// * `hash_unchanged` — branch 4 (ae:16821). A changed pane is activity.
/// * `hash_change_age_secs` — branch 5 (ae:16840): a pane that changed inside the
///   window is "recently visible", which covers a human reading a static answer.
/// * `last_actor_event_age_secs` — branch 6 (ae:16850): recent ae activity.
///
/// # The boundary is bash's, exactly
///
/// Both age branches skip on `age < stale_secs`, so `age == stale_secs` falls
/// THROUGH and is stale. Equality is stale; one second under is not.
///
/// # `last_change == 0`
///
/// Branch 5 guards on `last_change > 0` before comparing, for a pane whose hash
/// has never been recorded. That guard changes no verdict — `now - 0` is an epoch
/// count, which is never below any real stale window — so a caller with no
/// recorded change passes the age it computes and gets the same answer bash gives.
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
// ---------------------------------------------------------------------------
//
// The escape hatch for `waiting-user`/`blocked` is a per-pane baseline hash: the
// declaration's own echo is already on screen when the baseline is armed, so the
// hash HOLDS while the pane still shows only that, and YIELDS the moment anything
// else lands (a human's reply, the agent resuming). For that to work the view
// must have the watchdog's OWN footprints removed — its nudge and the `state`
// helper's echo — or the watchdog would keep waking itself up.
//
// The port is byte-faithful to the awk in bash's `_watchdog_quiet_hash`
// (ae:15675-15740) because a filter bug is not a cosmetic defect: dropping too
// much makes the watchdog deaf to a human reply, dropping too little makes it
// spam a quiet agent. Hand-rolled, no regex crate — zero-dep doctrine, and the
// five predicates are small enough to read against the awk line by line.
//
// THE SPECIMENS ARE CAPTURED RENDERINGS, not delivered bytes: this hashes what
// the TUI drew (`capture-pane -p -J`), and an earlier bash version that matched
// the bytes ae *sends* filtered nothing at all. The bash comment above the awk
// holds the captures; the tests below replay them.

/// The origin envelope the send helper stamps on a watchdog-delivered message
/// (#39) — the discriminator that separates a real nudge from an agent QUOTING
/// one, since quoted text renders as prose with no envelope above it.
const NUDGE_ENVELOPE: &str = "⟦ae:msg from watchdog⟧";

/// The nudge's own sentence, for the panes that render it unornamented.
const NUDGE_SENTENCE: &str = "Status check: if you have more work, continue. \
     Otherwise declare your state so I stop nudging: ";

/// The invitation the nudge ends with. The meta-dir path in front of it is the
/// awk's `.*`, so only the tail is fixed text (ae:16893 builds the line).
const NUDGE_TAIL: &str = "/state <waiting-user|blocked|done> \"<reason>\"";

/// The optional prefix a nudge carries when the session has a goal (ae:16887).
const NUDGE_GOAL_PREFIX: &str = "Session goal: ";

/// The state words a `state` echo can name — the alternation in the awk's
/// `is_echo`, and NOT the quiet set: `working` echoes are footprints too.
const ECHO_STATES: [&str; 4] = ["working", "waiting-user", "blocked", "done"];

/// POSIX `[[:space:]]` in the C locale — the class the awk is written against.
/// Deliberately NOT [`char::is_whitespace`], which is Unicode-wide: a
/// non-breaking space in pane output must not silently count as an indent.
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
/// (awk `submit_hdr`, ae:15706.)
///
/// The header is the discriminator rather than the body because the body wraps
/// at whatever width the pane had, and the wrap point is not stable. `›` is
/// codex's ornament, `❯` claude's; both were captured 2026-08-15.
fn submit_hdr(line: &str) -> bool {
    let body = trim_start_space(trim_end_space(line));
    let mut chars = body.chars();
    if !matches!(chars.next(), Some('›' | '❯')) {
        return false;
    }
    let rest = chars.as_str();
    // `[[:space:]]+` — at least one, then the envelope and nothing else.
    rest.starts_with(is_space) && trim_start_space(rest) == NUDGE_ENVELOPE
}

/// Two leading whitespace characters — the wrapped body of a rendered block.
/// (awk `indented`, ae:15707.)
///
/// A BLANK line is not indented, so it ends the block: the swallow is bounded by
/// the render, not by a line budget.
fn indented(line: &str) -> bool {
    let mut chars = line.chars();
    matches!(chars.next(), Some(c) if is_space(c)) && matches!(chars.next(), Some(c) if is_space(c))
}

/// The nudge as DELIVERED text — an unmodeled pane, or the legacy watchdog.
/// (awk `raw_nudge`, ae:15708-15710.)
fn raw_nudge(line: &str) -> bool {
    let Some(body) = trim_end_space(line).strip_suffix(NUDGE_TAIL) else {
        return false;
    };
    if body.starts_with(NUDGE_SENTENCE) {
        return true;
    }
    // `(Session goal: .*\. )?` — any goal text, ending at a `". "` the sentence
    // then follows. The awk backtracks over every candidate split; so does this,
    // because a goal may itself contain sentence-ending punctuation.
    let Some(goal) = body.strip_prefix(NUDGE_GOAL_PREFIX) else {
        return false;
    };
    goal.match_indices(". ").any(|(at, sep)| {
        goal.get(at + sep.len()..)
            .is_some_and(|tail| tail.starts_with(NUDGE_SENTENCE))
    })
}

/// The envelope ALONE on its line — an unmodeled pane's pair form, where the
/// nudge follows on the next line instead of being wrapped under an ornament.
/// (awk `raw_env`, ae:15711.)
fn raw_env(line: &str) -> bool {
    trim_end_space(line) == NUDGE_ENVELOPE
}

/// `Marked <agent> <state>` and its optional `:`/`.` remainder — the tail every
/// echo form ends with.
///
/// The awk's `[^ ]+` cannot cross a space, so the agent field is EXACTLY the
/// first space-delimited token: `Marked two words done` is not an echo, and must
/// survive into the hash.
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
/// (awk `is_echo`, ae:15712-15723.)
///
/// The claude form is the captured status grammar, not "anything mentioning
/// `output:`" — the loose version swallowed ordinary assistant prose, which is
/// deafness in the YIELD direction and the one failure this filter must never
/// introduce.
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
    // unmodeled pane, no TUI: the bare line.
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

/// The pane view with the watchdog's own footprints removed — the awk of bash's
/// `_watchdog_quiet_hash` (ae:15726-15738), line for line.
///
/// Everything else — the human's reply, the agent's real output — stays, and the
/// output is newline-TERMINATED exactly as awk's `print` leaves it.
///
/// The state machine is the part that must not drift:
/// * a rendered nudge's ornamented header opens a BLOCK whose indented body is
///   swallowed until the first unindented (or blank) line;
/// * a bare envelope is HELD, and dropped only if the raw nudge follows it —
///   otherwise it is a real message from the watchdog and gets printed;
/// * a QUOTED nudge, having neither ornament nor envelope, SURVIVES and still
///   wakes the watchdog.
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
///
/// FNV-1a/64 rather than bash's `md5`, and rather than [`std::hash`]'s default
/// hasher, which is RANDOMLY SEEDED per process: a baseline armed in one cycle is
/// compared to a hash taken in a later one, so the function must be stable for
/// the life of the daemon and identical across restarts.
///
/// Only EQUALITY is ever asked of it. It is deliberately not the bash digest —
/// nothing compares a Rust hash to a bash one across the clean cut, and a
/// baseline never leaves the daemon's own memory.
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
/// carries. Together they are the ONE event shape [`latest_relevant_event`] walks
/// past: a watchdog `alert` is news and stops the walk, and a peer's event with
/// action `nudge` is not the watchdog's.
const NUDGE_ACTOR: &str = "watchdog";
const NUDGE_ACTION: &str = "nudge";

/// Whether an event is RELEVANT to `agent` — bash's match at ae:15570-15572.
///
/// Three forms, because a target is written in three ways: the agent's own
/// events (actor), events aimed at it in this session (target), and events aimed
/// at it by the cross-session `@session:agent` spelling.
fn mentions(event: &Event, agent: &str, cross_session: &str) -> bool {
    if event.actor == agent {
        return true;
    }
    event
        .target
        .as_deref()
        .is_some_and(|target| target == agent || target == cross_session)
}

/// The newest event relevant to `agent`, plus whether the walk stepped past any
/// of the watchdog's own nudges to reach it — bash's `_latest_relevant_event`
/// (ae:15549-15590), which is the SELECTION half of the quiet decision that
/// [`quiet_reason`] then classifies.
///
/// # `events` must be in APPEND ORDER, and that is the whole contract
///
/// Oldest first, exactly as the container was written; the walk is this slice
/// reversed. Ordering by TIMESTAMP is not the same thing and is the bug bash
/// records at ae:15540-15547: `events.jsonl` timestamps are second-resolution, so
/// a declaration and the nudge that answered it in the same second compare EQUAL,
/// and a ts-bounded look-back skipped the declaration along with the nudge.
/// Append order is the truth here; the timestamp is its lossy rendering, and
/// reasoning about order from a display form is the same family of mistake as
/// reading state out of rendered text.
///
/// So the caller reads with a reader that PRESERVES that order —
/// [`crate::event_text::reversed`] and the events-tail path do; anything that
/// sorts does not — and this function never looks at [`Event::ts`] at all.
///
/// # The look-back is unbounded, by design
///
/// A quiet state stays valid until a NEWER event for this agent invalidates it,
/// however many unrelated events follow it (bash says so at ae:15556-15558).
/// Unrelated events are stepped over, never a stop.
///
/// # Walking past the watchdog's own nudges
///
/// One reverse pass consumes the consecutive run of watchdog nudges at the top
/// and returns the relevant event underneath, reporting that it did. A nudge is
/// an inbound event, so without this the watchdog invalidated the very state it
/// was asking about: one nudge and the agent stopped being quiet no matter what
/// the pane said, so it nudged again, and again (measured 2026-08-02). The flag
/// is what [`quiet_reason`] needs for its one exception — `done` IS cleared by a
/// nudge, because its contract is "honoured until a newer message arrives".
///
/// Only the watchdog's OWN nudges are skipped: an `alert` from the watchdog, or a
/// `nudge` from anyone else, is news like any other event.
///
/// bash takes the skip as an optional flag with a non-skipping arm; that arm has
/// no caller (`_agent_quiet_reason` at ae:15606 is the only one, and it passes
/// `skip`), so the ported shape is the one in use rather than the one written.
#[must_use]
pub fn latest_relevant_event<'a>(
    events: &'a [Event],
    agent: &str,
    session: &str,
) -> Option<(&'a Event, bool)> {
    let cross_session = format!("@{session}:{agent}");
    let mut looked_past_nudge = false;
    for event in events.iter().rev() {
        if !mentions(event, agent, &cross_session) {
            continue;
        }
        if event.actor == NUDGE_ACTOR && event.action == NUDGE_ACTION {
            looked_past_nudge = true;
            continue;
        }
        return Some((event, looked_past_nudge));
    }
    None
}

/// A self-declared state that tells the watchdog to stop nudging — bash's
/// `_agent_quiet_reason` (ae:15592-15657) reduced to its three answers.
///
/// The KIND is all this module decides. How each one is honoured is the daemon's:
/// `Done` is EVENT-ONLY quiet (pane churn — scrollback, resize, redraws — never
/// invalidates it), while `WaitingUser` and `Blocked` are honoured only while
/// their pane-hash baseline holds, because the human usually replies in the pane
/// and that reply exists nowhere else (ae:16724-16739).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuietKind {
    /// `done` — complete or paused.
    Done,
    /// `waiting-user` — needs human input.
    WaitingUser,
    /// `blocked` — stuck on an external dependency.
    Blocked,
}

/// The quiet state an agent's LATEST RELEVANT event declares, or `None`.
///
/// `latest` is the newest event whose actor or target is this agent, with the
/// watchdog's own nudges walked past; `looked_past_nudge` is whether any were.
/// Selecting it is the caller's job (bash splits it the same way, between
/// `_latest_relevant_event` at ae:15549 and the classification here).
///
/// The three rules, all bash's:
///
/// 1. **Only the agent's OWN declaration is quiet.** If the newest relevant event
///    is INBOUND — a peer or a human writing to the agent — the prior quiet state
///    is invalidated, because a quiet state must be broken by NEWS.
/// 2. **The watchdog asking is not news.** Its nudge is an inbound event, so it
///    invalidated the very state it was asking about; one nudge and the agent
///    stopped being quiet no matter what the pane said, so the watchdog nudged
///    again, and again (measured 2026-08-02). Hence the caller's skip.
/// 3. **`done` is the ONE exception to rule 2.** Its documented contract is
///    "honoured until a newer MESSAGE arrives", and a nudge is a message, so
///    clearing `done` on one is pinned behaviour rather than an oversight — which
///    is why `looked_past_nudge` un-quiets `Done` and nothing else.
///
/// `working` is a declaration but not a quiet one, and a `state` event with no
/// `ref` has declared nothing.
///
/// # Two typed readers deliberately NOT used here
///
/// * [`crate::state::latest`] answers a different question: it scans for an
///   actor's own `state`/`done` events and SKIPS everything else, so an inbound
///   message could never invalidate — rule 1 would be unreachable.
/// * [`Event::actor_identity`] prefers the churn-proof routing key, but the bash
///   decision compares the DISPLAY name it read off the pane's `@ae_agent`, and
///   a routed compare needs the agent's slot and session — inputs this decision
///   never had. Kept as bash has it; widening it is a decision, not a detail.
#[must_use]
pub fn quiet_reason(latest: &Event, agent: &str, looked_past_nudge: bool) -> Option<QuietKind> {
    if latest.actor != agent {
        return None; // inbound: news, and news ends a quiet state
    }
    // `declared_state` already folds in the legacy `action = done` record, which
    // bash maps to `done` at ae:15637.
    let kind = match latest.declared_state()? {
        "done" => QuietKind::Done,
        "waiting-user" => QuietKind::WaitingUser,
        "blocked" => QuietKind::Blocked,
        _ => return None, // `working`, or a ref that declares no state
    };
    if looked_past_nudge && kind == QuietKind::Done {
        return None;
    }
    Some(kind)
}

/// The declaration's identity, as [`quiet_pane_decision`] compares it —
/// `action|ts|ref|actor|summary`, bash's key at ae:15656.
///
/// The FULL tuple rather than the timestamp: two declarations can land in the
/// same second (ts is second-resolution), and any differing field — usually the
/// summary — still re-arms the baseline. Byte-identical same-second re-declares
/// remain indistinguishable, which bash accepts and so does this.
///
/// An absent `ref` or `summary` renders as empty, exactly as the bash key does.
/// The timestamp round-trips: [`crate::time::Timestamp`] parses only the one
/// documented spelling and renders it back, so the key holds the event's own ts.
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

/// What a pane's current hash means for a declared quiet state.
/// (bash `_quiet_pane_decision`, ae:15761-15771.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuietPane {
    /// No baseline for THIS declaration yet — take one.
    Arm,
    /// The pane still shows only what it showed when the baseline was armed.
    Hold,
    /// Something new landed: a human's reply, or the agent resuming.
    Yield,
}

/// The escape hatch's verdict for one pane.
///
/// `armed` is the baseline in force, as `(declaration key, hash)`; `None` is an
/// unarmed pane, which bash spells as an empty `armed_key` that can never equal
/// a real declaration.
///
/// The declaration is identified by its FULL tuple key (action|ts|ref|actor|
/// summary, built by `_agent_quiet_reason` at ae:15656) rather than by its
/// timestamp, so a genuinely new declaration re-arms even inside the same
/// second.
#[must_use]
pub fn quiet_pane_decision(cur_hash: u64, armed: Option<(&str, u64)>, decl_key: &str) -> QuietPane {
    match armed {
        Some((armed_key, armed_hash)) if armed_key == decl_key => {
            if cur_hash == armed_hash {
                QuietPane::Hold
            } else {
                QuietPane::Yield
            }
        }
        _ => QuietPane::Arm,
    }
}

/// The settled baseline for a pane, or `None` if it never held still — bash's
/// `_quiet_stabilize` (ae:15862-15884), with the captures hoisted out.
///
/// # Why settle at all
///
/// An agent declares its state with a TOOL CALL and emits its final message
/// AFTER — every harness works that way. So the pane is guaranteed to change
/// once more right after the declaration, and baselining the first observed pane
/// meant the agent broke its own hold (measured live: a `waiting-user`
/// declaration at 09:05:55Z nudged at 10:36:43Z through a branch reachable only
/// after the quiet branch had yielded). Settling happens WITHIN the arming cycle,
/// so a change in any LATER cycle is unambiguously not the declaration's own echo
/// and the verdict stays the crisp three.
///
/// # Purity
///
/// The daemon supplies the SAMPLES — bash sleeps a beat between captures, and
/// that beat is the loop's business, not this decision's. `samples` is the
/// arming capture followed by up to `tries` re-captures.
///
/// `None` covers both bash exits: the pane never settled, and a capture that
/// FAILED (an unreadable or dying pane, which the caller must not hand on as a
/// sample — two empty captures hash equal and would settle instantly).
#[must_use]
pub fn quiet_stabilize(samples: &[&str], tries: usize) -> Option<u64> {
    let mut captures = samples.iter();
    let mut prev = quiet_hash(captures.next()?);
    for _ in 0..tries {
        let cur = quiet_hash(captures.next()?);
        if cur == prev {
            return Some(cur);
        }
        prev = cur;
    }
    None
}

/// Whether a pane may pay the stabilization beat this cycle — bash's
/// `_quiet_stabilize_allowed` (ae:15846-15851).
///
/// Budget AND position. The budget alone is bounded but not FAIR: `list-panes` is
/// traversed in the same order every cycle and an unsettled attempt stores no
/// baseline, so the first two permanently unsettled panes consumed both slots
/// forever and panes 3+ were never attempted — they aged out and got nudged while
/// holding a perfectly good declaration, which is this slice's bug displaced onto
/// later panes.
#[must_use]
pub const fn quiet_stabilize_allowed(spent: usize, max: usize, idx: usize, cursor: usize) -> bool {
    spent < max && idx >= cursor
}

/// Where the next cycle starts — bash's `_quiet_cursor_advance` (ae:15855-15863).
///
/// Wrap when this cycle did not spend its budget (everyone reachable was offered
/// a turn) or when the cursor has run past the last pane, so the rotation cannot
/// itself starve: every pane is reached within one full rotation.
#[must_use]
pub const fn quiet_cursor_advance(cursor: usize, spent: usize, max: usize, seen: usize) -> usize {
    if spent < max || cursor > seen {
        0
    } else {
        cursor
    }
}

/// The rotating stabilization budget, as ONE machine.
///
/// Bash owns the same wiring in `_quiet_cycle_begin`/`_quiet_cycle_step`/
/// `_quiet_cycle_end` (ae:15866-15884) and says why: driving the predicate and
/// the wrap rule separately is not the same as driving the machine — a test that
/// advances its own cursor and calls the wrap itself passes even when the product
/// does neither, which is how the fairness fix ended up with no regression
/// protection at all. One definition, both callers.
#[derive(Debug, Clone, Copy)]
pub struct QuietCycle {
    max: usize,
    cursor: usize,
    spent: usize,
}

impl QuietCycle {
    /// A budget of `max` stabilizing panes per cycle, starting at the first pane.
    #[must_use]
    pub const fn new(max: usize) -> Self {
        Self {
            max,
            cursor: 0,
            spent: 0,
        }
    }

    /// Start of a watchdog cycle: the budget refills, the cursor does not move.
    pub const fn begin(&mut self) {
        self.spent = 0;
    }

    /// May the pane at `idx` stabilize now? Spending is recorded here, so the
    /// caller cannot forget to.
    pub const fn step(&mut self, idx: usize) -> bool {
        if !quiet_stabilize_allowed(self.spent, self.max, idx, self.cursor) {
            return false;
        }
        self.spent += 1;
        self.cursor = idx + 1; // the next cycle resumes after this pane
        true
    }

    /// End of the cycle: rotate for the next one.
    pub const fn end(&mut self, panes_seen: usize) {
        self.cursor = quiet_cursor_advance(self.cursor, self.spent, self.max, panes_seen);
    }

    /// Where the next cycle will resume — the rotation's only observable state.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }
}

// ---------------------------------------------------------------------------
// The orchestrator (meta-agent) sweep cadence — ae:16727-16899.
//
// An orchestrator session is a long-running SERVICE, so "idle between sweeps" is
// NORMAL rather than stale: its main agent leaves the stale-nudge watchdog
// entirely (SC-935) for an explicit cadence — prompt a sweep every
// `sweep_secs`, and guard liveness with the orchestrator's OWN heartbeat file
// instead of with pane silence.
//
// Everything below is the DECISION half. No clock, no filesystem, no send: the
// daemon reads the heartbeat's mtime, performs the delivery, and reports back
// what happened, exactly as it already does for the stale-nudge branch
// (`watchdog_daemon::account` / `record_nudge`).
// ---------------------------------------------------------------------------

/// The orchestrator sweep tunables — ae:16435-16448, with the frozen defaults.
///
/// bash validates these out of `AE_WATCHDOG_SWEEP_*` (with the `AE_LOOP_*`
/// legacy fallback, SC-1408a) before any arithmetic, and passes what it read;
/// the defaults live here so a caller that passes nothing runs the frozen
/// cadence (SC-1405a, SC-1406a, SC-1407a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepKnobs {
    /// Seconds between sweep prompts. `0` disables the branch (SC-1405b).
    pub sweep_secs: u64,
    /// How soon an UNDELIVERED prompt is retried instead of burning a whole
    /// cadence window (SC-937).
    pub retry_secs: u64,
    /// How many FAST retries are allowed before the branch falls back to the
    /// normal cadence and escalates once (SC-1407b).
    pub retry_max: u32,
}

impl Default for SweepKnobs {
    fn default() -> Self {
        Self {
            sweep_secs: 300,
            retry_secs: 30,
            retry_max: 6,
        }
    }
}

impl SweepKnobs {
    /// Whether the sweep branch runs at all.
    ///
    /// SC-1405b: `0` is not "sweep immediately", it is "there is no sweep
    /// branch" — the orchestrator main falls back to the normal watchdog. bash
    /// spells it as the `((SWEEP_SECS > 0))` conjunct on the branch guard
    /// (ae:16738), so a zero never reaches any of the arithmetic below.
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.sweep_secs > 0
    }

    /// The heartbeat window: `SWEEP_SECS * 2 + 60` (ae:16750).
    ///
    /// Two cadences plus a minute — one missed sweep is not a wedge, and the
    /// minute absorbs the poll interval that the second one is judged across.
    /// Saturating because the knob is caller-supplied: an absurd cadence must
    /// widen the window, never wrap it into a narrow one that alerts on
    /// everything.
    #[must_use]
    pub const fn wedge_secs(&self) -> u64 {
        self.sweep_secs.saturating_mul(2).saturating_add(60)
    }
}

/// Seconds ELAPSED from `then` to `now`, clamped at zero.
///
/// For the instants this daemon WROTE ITSELF — the cadence mark and the grace
/// origin. A future value there means our own clock went backwards between two
/// cycles, and the clamp is what bash does with it: `now - last_sweep` goes
/// negative, a negative is below any window, so the cadence is simply not due
/// yet and the grace has simply not elapsed. Both answers are the quiet ones.
///
/// # Why this is NOT the heartbeat's function — read before merging them
///
/// The heartbeat uses [`distance_secs`] instead, and the split is the point.
/// Applying the ABSOLUTE difference here would make a backwards clock jump
/// compute an enormous elapsed grace and raise a wedge alert against an orchestrator
/// that is sweeping perfectly well — a NEW false-wedge path, which is the exact
/// failure the heartbeat's symmetry was chosen to avoid. Applying the CLAMP to
/// the heartbeat would restore the indefinite-masking hole. Neither is a
/// generalisation of the other: one measures a value we wrote, the other a
/// value another process — possibly on another host — wrote.
fn secs_between(now: SystemTime, then: SystemTime) -> u64 {
    now.duration_since(then).map_or(0, |d| d.as_secs())
}

/// The ABSOLUTE distance between two instants, in seconds — direction dropped.
///
/// For the heartbeat alone, and only because a heartbeat is written by a
/// DIFFERENT process, possibly on a different host with a different clock. See
/// [`secs_between`] for why the two are not merged.
fn distance_secs(now: SystemTime, then: SystemTime) -> u64 {
    match now.duration_since(then) {
        Ok(elapsed) => elapsed.as_secs(),
        Err(ahead) => ahead.duration().as_secs(),
    }
}

/// How far a trusted heartbeat sits from now, and ON WHICH SIDE.
///
/// The side is kept because the WORDING depends on it, and a wrong word here is
/// a lie to a human debugging an outage: a timestamp in the FUTURE was not
/// "written N minutes ago", and saying so sends someone looking for a stall
/// that never happened. [`classify_heartbeat`] does not care about the side —
/// it judges the distance — so the two facts are separate on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeartbeatOffset {
    /// Written this many seconds ago. The ordinary case.
    Behind {
        /// Seconds since the heartbeat was written.
        secs: u64,
    },
    /// Stamped this many seconds in the FUTURE — clock skew between this host
    /// and whatever wrote the file, or a clock that was set forward.
    Ahead {
        /// Seconds by which the heartbeat leads this clock.
        secs: u64,
    },
}

impl HeartbeatOffset {
    /// The distance, with the side dropped — what the freshness window judges.
    #[must_use]
    pub const fn distance_secs(self) -> u64 {
        match self {
            Self::Behind { secs } | Self::Ahead { secs } => secs,
        }
    }
}

/// `now` moved `secs` into the past.
///
/// The fallback direction is deliberate: an instant that cannot be represented
/// makes the next prompt due IMMEDIATELY rather than a full cadence away. A
/// duplicate sweep prompt costs one redundant sweep; a dropped one costs a
/// blind spot (SC-939a's at-least-once trade, applied to the arithmetic).
fn back_date(now: SystemTime, secs: u64) -> SystemTime {
    now.checked_sub(Duration::from_secs(secs))
        .unwrap_or(UNIX_EPOCH)
}

/// What the orchestrator's heartbeat file says about it — the third state is the
/// point.
///
/// The orchestrator rewrites `<session>/meta-agent-state.json` on every real sweep
/// (the core entry `_monitor sweep` — see [`crate::monitor`]), so a heartbeat that stops
/// advancing while the watchdog keeps prompting is an orchestrator that is LIVE but
/// NOT SWEEPING: a model stall, an upstream throttle, a wedge (SC-939b). The
/// dead-pane and missing-pane checks cannot see that — the process is fine.
///
/// # Why three states and not two
///
/// bash reads the heartbeat as ONE number: `hb=0` unless `[[ -f ]]`, then
/// `_ae_stat mtime` (whose own failure also lands as `0`). So "no file",
/// "not a regular file", "a stat that could not run" and "a genuinely ancient
/// heartbeat" are all `hb=0`, distinguished only in the alert's wording.
///
/// This port keeps them apart because the SAFETY PIN needs them apart. The
/// mtime that reaches [`classify_heartbeat`] is trusted only when it came from
/// an `lstat` of a NON-SYMLINK REGULAR file: `[[ -f ]]` FOLLOWS symlinks, so
/// bash accepts a heartbeat that points anywhere, and anything writable in the
/// session directory could then be aimed at a file that some other process
/// touches often — a wedged orchestrator silenced by a link. `None` is every
/// untrusted reading, and `None` is [`Heartbeat::Untrusted`], never
/// [`Heartbeat::Fresh`].
///
/// Neither [`Heartbeat::Stale`] nor [`Heartbeat::Untrusted`] is a health claim:
/// both mean "not sweeping" and only the startup grace decides whether that is
/// worth an alert yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heartbeat {
    /// A trusted mtime inside the window — the orchestrator is sweeping.
    Fresh,
    /// A trusted mtime OUTSIDE the window, on either side: one that stopped
    /// advancing, or one stamped so far ahead of this clock that it cannot be
    /// read as liveness.
    Stale,
    /// No trusted mtime at all: missing, a symlink, not a regular file, or a
    /// reading that could not be taken. NOT evidence of health.
    Untrusted,
}

/// Classify an ALREADY-VALIDATED heartbeat mtime — ae:16748-16758.
///
/// `mtime` is `Some` only for a non-symlink regular file's modification time
/// (the caller's `lstat`, per the safety pin on [`Heartbeat`]); every other
/// reading — missing, symlink, directory, fifo, unreadable — arrives as `None`
/// and classifies [`Heartbeat::Untrusted`].
///
/// # The window is SYMMETRIC, and that is a deliberate divergence from bash
///
/// The boundary is bash's on the past side — `now - hb <= wedge_secs`, so an
/// age EQUAL to the window is still fresh and one second past it is not — but
/// this judges the DISTANCE, so it applies on the future side too:
///
/// * a NEAR-future mtime, inside the window, is **Fresh**. Deliberate skew
///   tolerance: a heartbeat can land on an NFS mount or a second host whose
///   clock leads ours by seconds, so every fresh write looks slightly future.
///   Refusing those would false-wedge a perfectly healthy orchestrator on every
///   sweep — worse than the hole it closes.
/// * a FAR-future mtime, beyond the window, is **Stale**. bash reads it Fresh
///   forever, because its `now - hb` goes negative and a negative is below any
///   window — so a timestamp set far ahead masks a wedged orchestrator
///   INDEFINITELY. Symmetry closes that: the further out the stamp, the sooner
///   it stops counting as liveness.
///
/// [`Heartbeat::Untrusted`] stays exactly what the READ LAYER could not vouch
/// for. A clock is not a trust question, and conflating the two is what makes
/// skew look like an attack.
#[must_use]
pub fn classify_heartbeat(
    mtime: Option<SystemTime>,
    now: SystemTime,
    wedge_secs: u64,
) -> Heartbeat {
    match mtime {
        None => Heartbeat::Untrusted,
        Some(at) if distance_secs(now, at) <= wedge_secs => Heartbeat::Fresh,
        Some(_) => Heartbeat::Stale,
    }
}

/// Where a trusted heartbeat sits relative to `now`, or `None` when there is no
/// trusted reading.
///
/// Separate from [`classify_heartbeat`] because an offset is not a
/// classification: this carries no policy and no window. The wedge alert's
/// wording needs the distance AND the side (ae:16780-16784), and
/// [`SweepObservation::new`] derives this and the classification from the SAME
/// reading so the two can never describe different files.
#[must_use]
pub fn heartbeat_offset(mtime: Option<SystemTime>, now: SystemTime) -> Option<HeartbeatOffset> {
    let at = mtime?;
    Some(match now.duration_since(at) {
        Ok(elapsed) => HeartbeatOffset::Behind {
            secs: elapsed.as_secs(),
        },
        Err(ahead) => HeartbeatOffset::Ahead {
            secs: ahead.duration().as_secs(),
        },
    })
}

/// The roster slot the orchestrator cadence belongs to. One spelling, here.
pub const MAIN_SLOT: &str = "main";

/// Whether this pane is the one the sweep cadence applies to — bash's
/// `[[ "$META_AGENT" == "true" && "$agent" == "$META_MAIN_AGENT" ]]`
/// (ae:16738), keyed by SLOT rather than by display name.
///
/// SC-935: the cadence is the orchestrator MAIN agent's alone. Workers and spawned
/// agents in an orchestrator session keep the normal watchdog — they are ordinary
/// agents that happen to share a session with a monitor.
///
/// # Why the SLOT, and not the `alias:name` bash compares
///
/// Same reason the roster line is keyed by slot (ae:16986-17024): `spawn`
/// uniquifies only the NUMERIC slot, so two registrations CAN share one
/// `alias:name`. A reference-keyed gate would hand the sweep cadence to BOTH
/// panes — and the cadence REPLACES stale escalation, so the second pane would
/// silently stop being watched. The slot cannot alias, and it is the key every
/// other per-pane decision in this daemon already uses.
///
/// It also removes a whole class of question rather than answering it. Keying
/// on the reference meant deriving it from `agent.main` in meta, which made the
/// gate depend on that record being unambiguous — one more thing to read, to
/// fail closed on, and to keep in step with bash's own multiline-scalar
/// behaviour. The pane's own `@ae_slot` is the fact, and ae stamped it.
///
/// An UNSTAMPED pane has no slot and is therefore never the target, which is
/// the fail-closed direction: an unstamped pane keeps the ordinary watchdog.
#[must_use]
pub fn is_sweep_target(meta_agent: bool, slot: &str) -> bool {
    meta_agent && slot == MAIN_SLOT
}

/// What the orchestrator's row shows this cycle — ae:16755-16765.
///
/// bash derives the glyph from the SAME two conditions the alert branch judges,
/// deliberately: the expressions are identical so the glyph can never disagree
/// with the alert. Here they are one expression, computed once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepVerdict {
    /// A fresh heartbeat — the orchestrator is sweeping.
    MetaSweeping,
    /// Prompted past the window with no fresh heartbeat — live but not sweeping.
    MetaWedged,
    /// Inside the startup grace with no heartbeat yet: genuinely undecided, and
    /// the glyph says exactly that rather than inventing a liveness claim.
    MetaStarting,
}

impl SweepVerdict {
    /// The glyph the frozen roster publishes for this verdict (ae:16202-16204).
    #[must_use]
    pub const fn glyph(self) -> &'static str {
        match self {
            Self::MetaSweeping => "👁",
            Self::MetaWedged => "◌",
            Self::MetaStarting => "·",
        }
    }
}

/// Which "not sweeping" the wedge alert is reporting — ae:16780-16784.
///
/// The two readings need different words because they send a human to different
/// places: a heartbeat that STOPPED points at the orchestrator's own loop, while one
/// that never existed points at the state file's path or its writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WedgeDetail {
    /// A trusted heartbeat exists but stopped advancing this long ago.
    Stalled {
        /// Seconds since the heartbeat last moved.
        age_secs: u64,
    },
    /// A trusted heartbeat stamped this far AHEAD of this clock — far enough
    /// that it is outside the freshness window and cannot be read as liveness.
    ///
    /// Its own variant rather than a signed `Stalled`, because the WORDING is
    /// the whole reason the side is carried: a future timestamp was not written
    /// "N minutes ago", and telling a human it was sends them hunting a stall
    /// that never happened. The past wording is bash's, frozen, and untouched.
    Ahead {
        /// Seconds by which the heartbeat leads this clock.
        ahead_secs: u64,
    },
    /// No trusted heartbeat has ever been read, across this much prompting.
    ///
    /// bash reaches this wording for a missing file. This port also reaches it
    /// for a heartbeat REFUSED by the safety pin (a symlink, a non-regular
    /// file) — where "never wrote a heartbeat" is the alert's honest reading of
    /// what the watchdog is willing to trust, not a claim about the file's
    /// contents.
    Never {
        /// Seconds of delivered sweep prompts with nothing to show for them.
        prompting_secs: u64,
    },
}

/// An alert the sweep branch raises or clears, as a TRANSITION rather than as
/// prose.
///
/// The two alerts are separate on purpose and bash says why (ae:16843-16856):
/// `alert-cleared` is UNTYPED — anything that clears clears THE alert — so a
/// clear emitted while the other alert is still latched would erase a live
/// warning that could then never re-fire. Keeping the transitions typed here is
/// what lets the accounting refuse that combination in one readable place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepAlert {
    /// Live but not sweeping (SC-939b). Raised ONCE per wedge.
    RaiseWedge(WedgeDetail),
    /// The heartbeat resumed.
    ClearWedge,
    /// Sweep prompts stopped landing altogether (SC-938/SC-1407b). Raised ONCE.
    RaiseUnreachable {
        /// Consecutive undelivered prompts at the moment of escalation.
        undelivered: u32,
    },
    /// A prompt landed again.
    ClearUnreachable,
}

impl SweepAlert {
    /// The frozen event action this transition appends.
    #[must_use]
    pub const fn action(self) -> &'static str {
        match self {
            Self::RaiseWedge(_) | Self::RaiseUnreachable { .. } => "alert",
            Self::ClearWedge | Self::ClearUnreachable => "alert-cleared",
        }
    }

    /// The frozen summary text, quoted from the branch that emits it.
    ///
    /// The wedge summary deliberately avoids the `throttl`/`dead`/`missing`
    /// substrings so `_agent_alert_reason` ranks it by its own "not sweeping"
    /// case (ae:16775-16777); the unreachable one carries "not sweeping" for
    /// the same reason. Do not reword either without checking that ranking.
    #[must_use]
    pub fn summary(self) -> String {
        match self {
            Self::RaiseWedge(WedgeDetail::Stalled { age_secs }) => format!(
                "meta-agent not sweeping — no heartbeat for {}m (may be stuck)",
                age_secs / 60
            ),
            Self::RaiseWedge(WedgeDetail::Ahead { ahead_secs }) => format!(
                "meta-agent not sweeping — heartbeat timestamp is {}m ahead of this clock (may \
                 be stuck)",
                ahead_secs / 60
            ),
            Self::RaiseWedge(WedgeDetail::Never { prompting_secs }) => format!(
                "meta-agent not sweeping — never wrote a heartbeat in {}m of sweep prompts (may \
                 be stuck)",
                prompting_secs / 60
            ),
            Self::ClearWedge => "meta-agent sweeping again (heartbeat resumed)".to_owned(),
            Self::RaiseUnreachable { undelivered } => format!(
                "meta-agent unreachable — {undelivered} sweep nudges undelivered (not sweeping)"
            ),
            Self::ClearUnreachable => {
                "meta-agent reachable again (sweep nudge delivered)".to_owned()
            }
        }
    }

    /// The `display-message` line for the human, or `None` when the transition
    /// is log-only. As with [`crate::watchdog_daemon::Effect::Notify`] this is
    /// the suffix; the loop prefixes the agent.
    #[must_use]
    pub const fn notify(self) -> Option<&'static str> {
        match self {
            Self::RaiseWedge(_) => Some("(meta-agent) not sweeping — may be stuck"),
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
    ///
    /// Delivery is CHECKED, never fire-and-forget (SC-936): `send` exits
    /// non-zero when it REFUSES a dead shell or ABANDONS a busy target, and a
    /// swallowed failure advanced the cadence while the orchestrator was never
    /// prompted (the measured 94-minute blind spot at ae:16812-16818).
    FireSweepNudge,
    /// Raise or clear one alert.
    Alert(SweepAlert),
    /// ONCE per daemon lifetime, on the first fresh heartbeat seen with no
    /// latched wedge: read the DURABLE event log and, if it still shows an
    /// active alert for this agent, emit [`SweepAlert::ClearWedge`]'s event
    /// (ae:16768-16774).
    ///
    /// The in-memory latch is lost across a watchdog restart, so a watchdog
    /// that alerted before the restart would otherwise never clear. Kept as an
    /// effect rather than as a `has_active_alert` input because bash reads that
    /// log LAZILY — once, and only on the cycle that can act on it.
    /// Idempotent: once cleared, the clear is the newest event and a re-scan
    /// finds nothing active.
    ReconcileWedge,
}

/// What the orchestrator pane carries from cycle to cycle — bash's
/// `last_sweep_nudge` / `first_sweep_nudge` / `sweep_nudge_fails` /
/// `meta_wedge_alerted` / `sweep_unreachable_alerted` / `meta_reconciled`
/// (ae:16409-16430), gathered into one value.
///
/// Default is the fresh pane bash starts with: every array unset, which its
/// `${...:-0}` reads as zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepState {
    /// When the cadence was last satisfied. `None` is bash's `0` — the first
    /// cycle is always due.
    ///
    /// A FAST RETRY back-dates this rather than storing a due time, exactly as
    /// bash does (`sweep_now - SWEEP_SECS + sweep_retry`), so one comparison
    /// serves both schedules.
    pub last_sweep: Option<SystemTime>,
    /// When the first prompt LANDED — the startup grace's origin.
    ///
    /// The wedge window ticks from a DELIVERED prompt, never from an attempt:
    /// starting it on an undelivered one would alert "not sweeping" against a
    /// orchestrator that was never actually reached.
    pub first_delivered: Option<SystemTime>,
    /// Consecutive undelivered prompts.
    pub fails: u32,
    /// The wedge alert is raised once per wedge, not once per cycle.
    pub wedge_alerted: bool,
    /// The unreachable alert is raised once per unreachable run.
    pub unreachable_alerted: bool,
    /// Whether the once-per-lifetime durable reconcile has been offered.
    pub reconciled: bool,
}

/// What the cycle observed about the orchestrator, after the heartbeat was read.
///
/// Build it with [`SweepObservation::new`]: the state and the age must come
/// from ONE reading, and a constructor is the only way to make describing two
/// different files impossible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepObservation {
    /// Wall clock for this cycle (bash's `now_epoch`).
    pub now: SystemTime,
    /// The heartbeat's tri-state.
    pub heartbeat: Heartbeat,
    /// Where it sits relative to `now`, `None` when there is no trusted
    /// reading. Carries the SIDE, because the alert's wording depends on it.
    pub heartbeat_offset: Option<HeartbeatOffset>,
}

impl SweepObservation {
    /// Derive the observation from one validated heartbeat reading.
    ///
    /// `heartbeat_mtime` is `Some` only for a non-symlink regular file — see
    /// the safety pin on [`Heartbeat`].
    #[must_use]
    pub fn new(now: SystemTime, heartbeat_mtime: Option<SystemTime>, knobs: &SweepKnobs) -> Self {
        Self {
            now,
            heartbeat: classify_heartbeat(heartbeat_mtime, now, knobs.wedge_secs()),
            heartbeat_offset: heartbeat_offset(heartbeat_mtime, now),
        }
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

/// Account for the orchestrator main in one cycle — ae:16738-16897, and the only
/// place any of it is decided.
///
/// `None` is SC-1405b: with `sweep_secs == 0` there is NO sweep branch, and the
/// caller must fall through to the normal watchdog for this pane. Returning an
/// `Option` rather than an empty accounting is the point — an empty accounting
/// would suppress the stale watchdog too, which is the opposite of the row.
///
/// The caller gates on [`is_sweep_target`] first (SC-935); this function
/// assumes the pane is the orchestrator main.
///
/// The order is bash's:
///
/// 1. the verdict, from the heartbeat and the startup grace;
/// 2. the wedge alert's raise / clear / post-restart reconcile;
/// 3. the cadence, which only ever DECIDES to prompt.
///
/// Step 3 never books its own outcome: the delivery happens in the loop and its
/// result comes back through [`record_sweep`]. That is the same split
/// [`crate::watchdog_daemon::account`] and
/// [`crate::watchdog_daemon::record_nudge`] already use, and it is what keeps
/// "a prompt was attempted" from being recorded as "a prompt landed" (SC-936).
#[must_use]
pub fn sweep_step(
    prior: &SweepState,
    seen: &SweepObservation,
    knobs: &SweepKnobs,
) -> Option<SweepAccounting> {
    if !knobs.enabled() {
        return None; // SC-1405b — not this branch at all.
    }
    let mut next = *prior;
    let mut effects = Vec::new();

    // 1. The verdict. A FRESH heartbeat is the only healthy reading; Stale and
    //    Untrusted both mean "not sweeping", and the startup grace is what
    //    decides whether that is worth an alert yet. The grace runs from the
    //    first DELIVERED prompt and the comparison is bash's strict `>`, so an
    //    age exactly equal to the window is still starting up.
    let grace_secs = prior.first_delivered.map(|at| secs_between(seen.now, at));
    let verdict = match (seen.heartbeat, grace_secs) {
        (Heartbeat::Fresh, _) => SweepVerdict::MetaSweeping,
        (_, Some(elapsed)) if elapsed > knobs.wedge_secs() => SweepVerdict::MetaWedged,
        _ => SweepVerdict::MetaStarting,
    };

    // 2. The wedge alert, judged off the SAME expression as the verdict.
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
                let detail = match seen.heartbeat_offset {
                    Some(HeartbeatOffset::Behind { secs }) => {
                        WedgeDetail::Stalled { age_secs: secs }
                    }
                    Some(HeartbeatOffset::Ahead { secs }) => {
                        WedgeDetail::Ahead { ahead_secs: secs }
                    }
                    None => WedgeDetail::Never {
                        prompting_secs: grace_secs.unwrap_or(0),
                    },
                };
                effects.push(SweepEffect::Alert(SweepAlert::RaiseWedge(detail)));
            }
        }
        SweepVerdict::MetaStarting => {}
    }

    // 3. The cadence. An absent `last_sweep` is bash's `0`: the first cycle
    //    prompts. The boundary is `>=`, so an elapsed exactly equal to the
    //    cadence is due.
    let due = prior
        .last_sweep
        .is_none_or(|at| secs_between(seen.now, at) >= knobs.sweep_secs);
    if due {
        effects.push(SweepEffect::FireSweepNudge);
    }

    Some(SweepAccounting {
        next,
        effects,
        verdict,
    })
}

/// Book a sweep prompt's outcome — ae:16833-16895.
///
/// Call this with the state [`sweep_step`] returned, only when its effects
/// contained [`SweepEffect::FireSweepNudge`], and only after the delivery
/// actually returned.
///
/// # The two clocks are not interchangeable
///
/// `cycle_now` is the clock the cycle opened with (bash's `now_epoch`);
/// `settled_now` is re-read AFTER the send returned (bash's `sweep_now`). The
/// send BLOCKS while it defers a busy target — up to `AE_SEND_DEFER_SEC` — so
/// scheduling a retry off `cycle_now` would make it due the moment `send`
/// returned, which is not a retry interval at all. A landed prompt keeps
/// `cycle_now`, because its schedule is the cadence and the cadence is the
/// cycle's.
///
/// # Delivery is at-least-once (SC-939a)
///
/// `send`'s status means "delivered AND logged" — its last command is the event
/// append — so an event-write failure after a successful paste reports failure
/// and this books a retry for a prompt the orchestrator already got. That is the
/// chosen trade: a duplicate sweep costs one redundant sweep, a dropped one
/// costs a blind spot. Do not "fix" it by reading a non-zero status as
/// delivered.
pub fn record_sweep(
    state: &mut SweepState,
    delivered: bool,
    cycle_now: SystemTime,
    settled_now: SystemTime,
    knobs: &SweepKnobs,
) -> Vec<SweepEffect> {
    if delivered {
        if state.first_delivered.is_none() {
            state.first_delivered = Some(cycle_now);
        }
        state.fails = 0;
        state.last_sweep = Some(cycle_now);
        if !state.unreachable_alerted {
            return Vec::new();
        }
        state.unreachable_alerted = false;
        // The latch drops either way; only the EVENT is conditional.
        // `alert-cleared` is untyped, so emitting one while the wedge alert is
        // still latched would erase a live "not sweeping" from `ae list` while
        // `wedge_alerted` stayed set — it could then never re-fire, and a
        // wedged orchestrator would go invisible. The wedge owns its own clear.
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
        //
        // SC-1406b: CLAMPED to the cadence, in ONE expression. A retry longer
        // than `sweep_secs` would push `last_sweep` into the FUTURE and DELAY
        // the next prompt instead of hastening it, so the amount the cadence is
        // hastened by SATURATES at zero: a retry at or past the cadence simply
        // does not hasten it. bash clamps `sweep_retry` and then subtracts;
        // that is the same arithmetic, but writing BOTH here left the clamp
        // undecidable — no input could tell the `min` from the saturation, so
        // no test could hold it (a mutation of the `min` alone stayed green).
        // It is a FLOOR, not a deadline: the watchdog polls on its own
        // interval, so the retry lands on the first poll at or after that
        // point, never sooner.
        let hastened_by = knobs.sweep_secs.saturating_sub(knobs.retry_secs);
        state.last_sweep = Some(back_date(settled_now, hastened_by));
        return Vec::new();
    }

    // SC-938/SC-1407b: bounded. A persistently unreachable orchestrator degrades to
    // the normal cadence and escalates ONCE rather than retry-spamming.
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

    use super::{
        Heartbeat, HeartbeatOffset, QuietCycle, QuietKind, QuietPane, SweepAlert, SweepEffect,
        SweepKnobs, SweepObservation, SweepState, SweepVerdict, WedgeDetail, classify_dead,
        classify_heartbeat, command_is_shell, declaration_key, heartbeat_offset, indented, is_echo,
        is_sweep_target, latest_relevant_event, quiet_cursor_advance, quiet_filter, quiet_hash,
        quiet_pane_decision, quiet_reason, quiet_stabilize, quiet_stabilize_allowed, raw_nudge,
        record_sweep, shows_throttle, stale_composite, submit_hdr, sweep_step,
    };
    use crate::events::Event;
    use crate::procs::Descendancy;

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
        // The divergence from bash, and the one that matters: bash cannot tell a
        // probe that FAILED from one that ran and found nothing, so it alerts a
        // live agent dead. Unknown is not dead for ANY foreground, shell or not.
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
        // bash skips on `age < STALE_SECS`, so equality falls through and IS stale.
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
        let own_done = event(
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"done","summary":"shipped"}"#,
        );
        assert_eq!(quiet_reason(&own_done, agent, false), Some(QuietKind::Done));
        let waiting = event(
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-user","summary":"needs review"}"#,
        );
        assert_eq!(
            quiet_reason(&waiting, agent, false),
            Some(QuietKind::WaitingUser)
        );
        let blocked = event(
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"blocked","summary":"waiting on CI"}"#,
        );
        assert_eq!(
            quiet_reason(&blocked, agent, false),
            Some(QuietKind::Blocked)
        );
        // The legacy record: `action = done` with no ref maps to Done.
        let legacy =
            event(r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"done"}"#);
        assert_eq!(quiet_reason(&legacy, agent, false), Some(QuietKind::Done));
    }

    #[test]
    fn news_from_anyone_else_ends_a_quiet_state() {
        let agent = "opus5:builder";
        // An inbound message TARGETING the agent is the newest relevant event, and
        // it invalidates whatever the agent last declared.
        let inbound = event(
            r#"{"ts":"2026-08-29T04:01:00Z","actor":"fable5:lead","action":"send","target":"opus5:builder","summary":"review please"}"#,
        );
        assert_eq!(quiet_reason(&inbound, agent, false), None);
        // Even an inbound event that would otherwise LOOK like a declaration.
        let inbound_state = event(
            r#"{"ts":"2026-08-29T04:01:00Z","actor":"fable5:lead","action":"state","ref":"done","target":"opus5:builder"}"#,
        );
        assert_eq!(quiet_reason(&inbound_state, agent, false), None);
    }

    #[test]
    fn working_and_a_refless_state_declare_no_quiet_state() {
        let agent = "opus5:builder";
        let working = event(
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"working","summary":"on it"}"#,
        );
        assert_eq!(quiet_reason(&working, agent, false), None);
        let refless =
            event(r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state"}"#);
        assert_eq!(
            quiet_reason(&refless, agent, false),
            None,
            "declared nothing"
        );
        let unrelated = event(
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"memo","ref":"arch"}"#,
        );
        assert_eq!(quiet_reason(&unrelated, agent, false), None);
    }

    #[test]
    fn a_nudge_walked_past_clears_done_and_nothing_else() {
        let agent = "opus5:builder";
        let done = event(
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"done","summary":"shipped"}"#,
        );
        assert_eq!(quiet_reason(&done, agent, false), Some(QuietKind::Done));
        assert_eq!(
            quiet_reason(&done, agent, true),
            None,
            "done is honoured until a newer MESSAGE arrives, and a nudge is one"
        );
        // The look-past is scoped to the two states it exists for: they are pane
        // holds, and a nudge must not break them.
        for (line, kind) in [
            (
                r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-user"}"#,
                QuietKind::WaitingUser,
            ),
            (
                r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"blocked","summary":"dep"}"#,
                QuietKind::Blocked,
            ),
        ] {
            assert_eq!(quiet_reason(&event(line), agent, true), Some(kind));
        }
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
        assert_eq!(
            quiet_pane_decision(
                7,
                Some((&declaration_key(&first), 7)),
                &declaration_key(&second)
            ),
            QuietPane::Arm,
            "a new declaration re-arms the baseline"
        );
        // An absent ref and summary render empty, exactly as the bash key does.
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
        let (found, looked_past) = latest_relevant_event(&events, "opus5:builder", "aerewrite")
            .expect("the declaration is relevant");
        assert_eq!(found.reference.as_deref(), Some("waiting-user"));
        assert!(!looked_past, "no nudge was walked past");
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
        let (found, _) = latest_relevant_event(&events, "opus5:builder", "aerewrite")
            .expect("500 unrelated events do not end the walk");
        assert_eq!(found.reference.as_deref(), Some("blocked"));
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
                latest_relevant_event(events, "opus5:builder", "aerewrite").is_some(),
                "the {form} form is relevant"
            );
        }
        // A different session's spelling of the same name is NOT this agent's.
        assert!(
            latest_relevant_event(&cross_session, "opus5:builder", "other").is_none(),
            "the cross-session form is keyed to the session"
        );
        // Nothing mentioning the agent at all.
        let unrelated = log(&[
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"fable5:lead","action":"send","target":"gpt56sol:colead"}"#,
        ]);
        assert!(latest_relevant_event(&unrelated, "opus5:builder", "aerewrite").is_none());
    }

    #[test]
    fn a_declaration_and_the_nudge_answering_it_in_the_same_second_both_survive() {
        // The bug bash records at ae:15540-15547: second-resolution timestamps make
        // these two compare EQUAL, so a ts-bounded look-back skipped the
        // declaration along with the nudge. Append order is the truth.
        let events = log(&[
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-user","summary":"review"}"#,
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"watchdog","action":"nudge","target":"opus5:builder","summary":"idle 15m"}"#,
        ]);
        let (found, looked_past) = latest_relevant_event(&events, "opus5:builder", "aerewrite")
            .expect("the declaration is underneath the nudge");
        assert_eq!(found.reference.as_deref(), Some("waiting-user"));
        assert!(looked_past, "a nudge WAS walked past");
        // And the two halves compose the way the daemon will use them.
        assert_eq!(
            quiet_reason(found, "opus5:builder", looked_past),
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
        let (found, looked_past) = latest_relevant_event(&events, "opus5:builder", "aerewrite")
            .expect("the done is under five nudges");
        assert_eq!(found.action, "done");
        assert!(looked_past);
        // done is the ONE kind a walked-past nudge clears.
        assert_eq!(quiet_reason(found, "opus5:builder", looked_past), None);
    }

    #[test]
    fn only_the_watchdogs_own_nudges_are_walked_past() {
        // A watchdog ALERT is news and stops the walk.
        let alerted = log(&[
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-user"}"#,
            r#"{"ts":"2026-08-29T04:05:00Z","actor":"watchdog","action":"alert","target":"opus5:builder","summary":"stale"}"#,
        ]);
        let (found, looked_past) = latest_relevant_event(&alerted, "opus5:builder", "aerewrite")
            .expect("the alert is relevant");
        assert_eq!(found.action, "alert");
        assert!(!looked_past);
        // A `nudge` from a PEER is not the watchdog's, and is news.
        let peer = log(&[
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"opus5:builder","action":"state","ref":"waiting-user"}"#,
            r#"{"ts":"2026-08-29T04:05:00Z","actor":"fable5:lead","action":"nudge","target":"opus5:builder"}"#,
        ]);
        let (found, looked_past) = latest_relevant_event(&peer, "opus5:builder", "aerewrite")
            .expect("the peer event is relevant");
        assert_eq!(found.actor, "fable5:lead");
        assert!(!looked_past);
        assert_eq!(
            quiet_reason(found, "opus5:builder", looked_past),
            None,
            "a peer writing to the agent is news, and news ends a quiet state"
        );
    }

    #[test]
    fn a_log_of_nothing_but_nudges_selects_nothing() {
        let events = log(&[
            r#"{"ts":"2026-08-29T04:05:00Z","actor":"watchdog","action":"nudge","target":"opus5:builder"}"#,
        ]);
        assert!(latest_relevant_event(&events, "opus5:builder", "aerewrite").is_none());
        assert!(latest_relevant_event(&[], "opus5:builder", "aerewrite").is_none());
    }

    #[test]
    fn selection_follows_append_order_even_when_the_timestamps_disagree() {
        // The ONE property that a ts-ordered implementation would get wrong while
        // every other test still passed: the newest APPENDED event carries an
        // OLDER timestamp than the one before it (clock skew, or a container
        // stitched from two generations).
        let events = log(&[
            r#"{"ts":"2026-08-29T04:09:00Z","actor":"opus5:builder","action":"state","ref":"waiting-user"}"#,
            r#"{"ts":"2026-08-29T04:00:00Z","actor":"fable5:lead","action":"send","target":"opus5:builder","summary":"answered"}"#,
        ]);
        let (found, _) = latest_relevant_event(&events, "opus5:builder", "aerewrite")
            .expect("something is relevant");
        assert_eq!(
            found.actor, "fable5:lead",
            "the LAST APPENDED relevant event wins, whatever its ts says"
        );
    }

    // ---- Quiet detection --------------------------------------------------
    //
    // PANE_SPECIMEN is written from the renderings CAPTURED in the bash comment
    // above `_watchdog_quiet_hash` (ae:15680-15700), plus the near-misses that
    // must survive. AWK_ORACLE is not what this port is expected to produce —
    // it is what the FROZEN BASH AWK actually produced when run over that
    // specimen (2026-08-29, `awk -f <the awk, extracted verbatim from ae>`), so
    // the expectation comes from the behavior authority and not from a second
    // reading of it by the same author.
    //
    // Both are column-0 raw strings: leading whitespace is semantic here (it is
    // what `indented` tests), so they must not be re-indented to match the block.

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

    /// The nudge exactly as ae:16893 composes it, meta-dir path and all.
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

    #[test]
    fn an_unarmed_pane_arms_then_holds_until_the_pane_changes() {
        let decl = "state|2026-08-29T04:00:00Z|waiting-user|opus5:builder|review";
        assert_eq!(
            quiet_pane_decision(7, None, decl),
            QuietPane::Arm,
            "no baseline yet"
        );
        assert_eq!(
            quiet_pane_decision(7, Some((decl, 7)), decl),
            QuietPane::Hold
        );
        assert_eq!(
            quiet_pane_decision(9, Some((decl, 7)), decl),
            QuietPane::Yield
        );
        // A NEW declaration re-arms even against a baseline that is still held —
        // the key is the full tuple, so two same-second declarations differ.
        let redeclared = "state|2026-08-29T04:00:00Z|waiting-user|opus5:builder|now blocked";
        assert_eq!(
            quiet_pane_decision(7, Some((decl, 7)), redeclared),
            QuietPane::Arm
        );
    }

    #[test]
    fn stabilization_settles_on_two_consecutive_matching_samples() {
        let settling = "declared\nMarked opus5:builder waiting-user: review\n";
        let samples = ["declared\n", settling, settling];
        let settled = quiet_stabilize(&samples, 2);
        assert_eq!(
            settled,
            Some(quiet_hash(settling)),
            "the settled baseline is the repeated sample's hash"
        );
    }

    #[test]
    fn a_pane_that_never_holds_still_has_no_baseline() {
        let samples = ["one\n", "two\n", "three\n", "four\n"];
        assert_eq!(
            quiet_stabilize(&samples, 3),
            None,
            "still emitting means working, not waiting"
        );
        // Fewer samples than tries is the failed-capture exit: no baseline, and
        // NOT an accidental settle on two empty captures.
        assert_eq!(quiet_stabilize(&["one\n"], 2), None);
        assert_eq!(quiet_stabilize(&[], 2), None);
        // Nudges and echoes are filtered out, so two samples that differ only by
        // the watchdog's own footprints DO settle.
        assert!(quiet_stabilize(&["idle\n", "idle\nMarked a done\n"], 1).is_some());
    }

    #[test]
    fn the_rotating_budget_reaches_every_pane_within_one_rotation() {
        // The fairness bug this rotation exists to prevent: with a plain budget,
        // the first two panes consume both slots EVERY cycle and panes 3+ are
        // never attempted. Indices are the loop's 1-based traversal position
        // (ae:16484 increments before the step).
        let mut cycle = QuietCycle::new(2);
        let mut reached: Vec<usize> = Vec::new();
        for _ in 0..3 {
            cycle.begin();
            for idx in 1..=5 {
                if cycle.step(idx) {
                    reached.push(idx);
                }
            }
            cycle.end(5);
        }
        reached.sort_unstable();
        assert_eq!(
            reached,
            vec![1, 2, 3, 4, 5],
            "every pane is offered a turn within one full rotation"
        );
    }

    #[test]
    fn the_cursor_wraps_when_the_budget_is_not_spent_or_the_panes_shrink() {
        // Everyone reachable got a turn: start over next cycle.
        assert_eq!(quiet_cursor_advance(3, 1, 2, 5), 0);
        // Budget spent and panes remain behind the cursor: resume there.
        assert_eq!(quiet_cursor_advance(3, 2, 2, 5), 3);
        // The cursor ran past the last pane (panes disappeared): start over.
        assert_eq!(quiet_cursor_advance(6, 2, 2, 5), 0);
    }

    #[test]
    fn the_budget_predicate_is_position_and_count_together() {
        assert!(
            quiet_stabilize_allowed(0, 2, 1, 0),
            "fresh cycle, first pane"
        );
        assert!(!quiet_stabilize_allowed(2, 2, 3, 0), "budget spent");
        assert!(!quiet_stabilize_allowed(0, 2, 1, 3), "behind the cursor");
        assert!(
            quiet_stabilize_allowed(1, 2, 3, 3),
            "at the cursor, budget left"
        );
    }

    // -- the orchestrator sweep cadence (SC-935..SC-939b, SC-1405b, SC-1406b) ------

    /// A fixed clock. Far from the epoch so a back-dated cadence is a real
    /// instant rather than a saturation artefact.
    const BASE: u64 = 1_700_000_000;

    fn at(offset_secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(BASE + offset_secs)
    }

    /// The frozen knobs: 300s cadence, 30s retry, 6 fast retries.
    fn knobs() -> SweepKnobs {
        SweepKnobs::default()
    }

    fn seen(now_offset: u64, heartbeat: Option<u64>, k: &SweepKnobs) -> SweepObservation {
        SweepObservation::new(at(now_offset), heartbeat.map(at), k)
    }

    #[test]
    fn the_frozen_sweep_defaults_are_the_bash_ones() {
        let k = knobs();
        assert_eq!(k.sweep_secs, 300, "SC-1405a");
        assert_eq!(k.retry_secs, 30, "SC-1406a");
        assert_eq!(k.retry_max, 6, "SC-1407a");
        assert_eq!(k.wedge_secs(), 660, "SWEEP_SECS * 2 + 60");
        assert!(k.enabled());
    }

    #[test]
    fn an_untrusted_reading_is_never_fresh_however_recent_the_clock_is() {
        // The safety pin, as a pair: the SAME instant classifies Fresh when the
        // mtime is trusted and Untrusted when it is not. Nothing about the
        // clock can turn a missing / symlinked / non-regular heartbeat into a
        // health claim.
        assert_eq!(
            classify_heartbeat(None, at(0), 660),
            Heartbeat::Untrusted,
            "no trusted mtime is never Fresh"
        );
        assert_eq!(
            classify_heartbeat(Some(at(0)), at(0), 660),
            Heartbeat::Fresh,
            "the control: a trusted mtime at the same instant IS fresh"
        );
        // Even a zero-width window cannot make the absent reading fresh.
        assert_eq!(classify_heartbeat(None, at(0), 0), Heartbeat::Untrusted);
        assert_eq!(heartbeat_offset(None, at(0)), None);
    }

    #[test]
    fn the_heartbeat_window_is_symmetric_so_skew_is_tolerated_but_never_forever() {
        // PAST SIDE — bash's boundary, unchanged: `now - hb <= wedge_secs`.
        assert_eq!(
            classify_heartbeat(Some(at(0)), at(660), 660),
            Heartbeat::Fresh,
            "an age exactly at the window is fresh"
        );
        assert_eq!(
            classify_heartbeat(Some(at(0)), at(661), 660),
            Heartbeat::Stale,
            "one second past it is not"
        );

        // FUTURE SIDE — the divergence, and it flips at the SAME boundary.
        // Inside the window a leading clock is tolerated, because an NFS mount
        // or a second host makes every fresh write look slightly future and
        // refusing those would false-wedge a healthy orchestrator on every sweep.
        assert_eq!(
            classify_heartbeat(Some(at(900)), at(300), 660),
            Heartbeat::Fresh,
            "600s ahead is inside the window — deliberate skew tolerance"
        );
        assert_eq!(
            classify_heartbeat(Some(at(960)), at(300), 660),
            Heartbeat::Fresh,
            "exactly at the window, on the future side too"
        );
        // Beyond it, it stops counting as liveness. THIS IS THE CLOSED HOLE:
        // bash reads any future stamp Fresh forever (its `now - hb` goes
        // negative and a negative is below any window), so a timestamp set far
        // ahead masks a wedged orchestrator INDEFINITELY.
        assert_eq!(
            classify_heartbeat(Some(at(961)), at(300), 660),
            Heartbeat::Stale,
            "one second beyond the window on the future side is not liveness"
        );
        assert_eq!(
            classify_heartbeat(Some(at(9_000_000)), at(300), 660),
            Heartbeat::Stale,
            "a far-future stamp can no longer mask a wedge"
        );

        // The OFFSET reports the real distance and the real side — never 0,
        // and never "behind" for a timestamp that is ahead.
        assert_eq!(
            heartbeat_offset(Some(at(900)), at(300)),
            Some(HeartbeatOffset::Ahead { secs: 600 }),
            "a future mtime reports its true distance, not zero"
        );
        assert_eq!(
            heartbeat_offset(Some(at(0)), at(700)),
            Some(HeartbeatOffset::Behind { secs: 700 })
        );
        assert_eq!(
            HeartbeatOffset::Ahead { secs: 600 }.distance_secs(),
            HeartbeatOffset::Behind { secs: 600 }.distance_secs(),
            "the window judges the distance; only the wording judges the side"
        );
    }

    #[test]
    fn a_far_future_heartbeat_is_never_described_as_having_been_written_ago() {
        // A timestamp in the FUTURE was not written N minutes ago, and telling
        // a human it was sends them hunting a stall that never happened.
        let k = knobs();
        let prior = SweepState {
            first_delivered: Some(at(0)),
            ..SweepState::default()
        };
        // Past the grace, with a heartbeat stamped 2000s AHEAD of now.
        let acc = sweep_step(&prior, &seen(700, Some(2700), &k), &k).expect("enabled");
        assert_eq!(acc.verdict, SweepVerdict::MetaWedged);
        assert!(
            acc.effects
                .contains(&SweepEffect::Alert(SweepAlert::RaiseWedge(
                    WedgeDetail::Ahead { ahead_secs: 2000 }
                ))),
            "the future side gets its own detail, not a signed Stalled"
        );
        let text = SweepAlert::RaiseWedge(WedgeDetail::Ahead { ahead_secs: 2000 }).summary();
        assert_eq!(
            text,
            "meta-agent not sweeping — heartbeat timestamp is 33m ahead of this clock (may be \
             stuck)"
        );
        for banned in ["ago", "no heartbeat for", "never wrote"] {
            assert!(
                !text.contains(banned),
                "{text:?} must not carry {banned:?} — the timestamp is AHEAD, not old"
            );
        }
        // Still an alert of the right CLASS: `_agent_alert_reason` ranks on
        // "not sweeping", and none of the higher-ranking substrings may appear.
        assert!(text.contains("not sweeping"));
        for banned in ["throttl", "dead", "missing"] {
            assert!(!text.contains(banned), "{text:?} must not carry {banned:?}");
        }
    }

    #[test]
    fn the_daemons_own_timestamps_clamp_rather_than_taking_a_distance() {
        // The SPLIT, pinned. `first_delivered` and `last_sweep` are values THIS
        // daemon wrote; a future one means our own clock went backwards. Taking
        // the absolute distance there would compute an enormous elapsed grace
        // and raise a wedge against an orchestrator that is sweeping perfectly well —
        // a NEW false-wedge path, which is the failure the heartbeat symmetry
        // exists to avoid. bash clamps both, and so does this.
        let k = knobs();
        let jumped = SweepState {
            // Both stamped an hour ahead of the cycle clock.
            first_delivered: Some(at(3600)),
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
        // SC-935: workers and spawned agents in an orchestrator session keep the
        // normal watchdog. Keyed by SLOT, which cannot alias — `spawn`
        // uniquifies only the numeric slot, so two registrations can share one
        // `alias:name`, and a reference-keyed gate would hand the cadence to
        // BOTH (silently un-watching the second, since the cadence replaces
        // stale escalation).
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
        // SC-1405b, with its own control: the SAME inputs that produce a wedge
        // alert on the frozen cadence produce NO BRANCH at all on `0` — the
        // caller falls through to the normal watchdog.
        let prior = SweepState {
            first_delivered: Some(at(0)),
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
                    WedgeDetail::Never {
                        prompting_secs: 5000
                    }
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
    fn an_undelivered_prompt_retries_fast_without_consuming_the_cadence_slot() {
        // SC-937. The retry is a FLOOR: due 30s after the failure, on the first
        // poll at or after that point.
        let k = knobs();
        let mut state = SweepState {
            last_sweep: Some(at(0)),
            ..SweepState::default()
        };
        let effects = record_sweep(&mut state, false, at(300), at(302), &k);
        assert!(effects.is_empty(), "a fast retry escalates nothing");
        assert_eq!(state.fails, 1);
        assert_eq!(
            state.first_delivered, None,
            "the wedge grace never starts on an attempt"
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
        // SC-1406b. An unclamped 600s retry against a 300s cadence would push
        // last_sweep into the FUTURE and DELAY the next prompt to +600.
        let k = SweepKnobs {
            retry_secs: 600,
            ..knobs()
        };
        let mut state = SweepState::default();
        assert!(record_sweep(&mut state, false, at(0), at(0), &k).is_empty());
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
        // SC-938 / SC-1407b.
        let k = knobs();
        let mut state = SweepState::default();
        for attempt in 1..=k.retry_max {
            let effects = record_sweep(&mut state, false, at(0), at(0), &k);
            assert!(effects.is_empty(), "fast retry {attempt} escalates nothing");
            assert!(!state.unreachable_alerted);
        }
        let escalation = record_sweep(&mut state, false, at(0), at(0), &k);
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
            record_sweep(&mut state, false, at(0), at(0), &k).is_empty(),
            "the unreachable alert is raised once per run"
        );

        // Cleared on a landed delivery.
        let cleared = record_sweep(&mut state, true, at(900), at(901), &k);
        assert_eq!(
            cleared,
            vec![SweepEffect::Alert(SweepAlert::ClearUnreachable)]
        );
        assert!(!state.unreachable_alerted);
        assert_eq!(state.fails, 0);
        assert_eq!(
            state.last_sweep,
            Some(at(900)),
            "a landed prompt schedules off the CYCLE clock"
        );
        assert_eq!(state.first_delivered, Some(at(900)));
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
            record_sweep(&mut state, false, at(0), at(0), &k),
            vec![SweepEffect::Alert(SweepAlert::RaiseUnreachable {
                undelivered: 1
            })]
        );
        assert_eq!(state.last_sweep, Some(at(0)));
    }

    #[test]
    fn the_unreachable_clear_is_withheld_while_a_wedge_alert_is_still_latched() {
        // `alert-cleared` is untyped: emitting one here would erase a live
        // "not sweeping" that could then never re-fire. The latch still drops.
        let k = knobs();
        let mut state = SweepState {
            unreachable_alerted: true,
            wedge_alerted: true,
            ..SweepState::default()
        };
        assert!(
            record_sweep(&mut state, true, at(0), at(0), &k).is_empty(),
            "reachable again, but the wedge alert owns its own clear"
        );
        assert!(!state.unreachable_alerted);
        assert!(state.wedge_alerted);
    }

    #[test]
    fn the_first_delivered_prompt_is_the_only_one_that_starts_the_grace() {
        let k = knobs();
        let mut state = SweepState::default();
        assert!(record_sweep(&mut state, true, at(10), at(11), &k).is_empty());
        assert_eq!(state.first_delivered, Some(at(10)));
        assert!(record_sweep(&mut state, true, at(310), at(311), &k).is_empty());
        assert_eq!(
            state.first_delivered,
            Some(at(10)),
            "the grace origin never moves"
        );
    }

    #[test]
    fn the_wedge_raises_once_past_the_grace_and_the_boundary_is_strict() {
        // SC-939b. The grace runs from the first DELIVERED prompt; bash's
        // comparison is `>`, so an elapsed exactly at the window is still
        // starting up.
        let k = knobs();
        let prior = SweepState {
            first_delivered: Some(at(0)),
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
                    WedgeDetail::Never {
                        prompting_secs: 661
                    }
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
    fn a_stalled_heartbeat_and_an_untrusted_one_wedge_with_different_words() {
        let k = knobs();
        let prior = SweepState {
            first_delivered: Some(at(0)),
            ..SweepState::default()
        };
        // A trusted heartbeat that stopped advancing.
        let stalled = sweep_step(&prior, &seen(700, Some(0), &k), &k).expect("enabled");
        assert_eq!(stalled.verdict, SweepVerdict::MetaWedged);
        assert!(
            stalled
                .effects
                .contains(&SweepEffect::Alert(SweepAlert::RaiseWedge(
                    WedgeDetail::Stalled { age_secs: 700 }
                )))
        );
        // No trusted heartbeat at all.
        let never = sweep_step(&prior, &seen(700, None, &k), &k).expect("enabled");
        assert!(
            never
                .effects
                .contains(&SweepEffect::Alert(SweepAlert::RaiseWedge(
                    WedgeDetail::Never {
                        prompting_secs: 700
                    }
                )))
        );
    }

    #[test]
    fn a_fresh_heartbeat_clears_a_latched_wedge_and_reports_sweeping() {
        let k = knobs();
        let wedged = SweepState {
            first_delivered: Some(at(0)),
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
        let steady = sweep_step(&recovered.next, &seen(900, Some(890), &k), &k).expect("enabled");
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
        // fresh heartbeat has to reach for the event log — once.
        let k = knobs();
        let restarted = SweepState::default();
        let first = sweep_step(&restarted, &seen(0, Some(0), &k), &k).expect("enabled");
        assert_eq!(first.verdict, SweepVerdict::MetaSweeping);
        assert!(first.effects.contains(&SweepEffect::ReconcileWedge));
        assert!(first.next.reconciled);

        let second = sweep_step(&first.next, &seen(400, Some(390), &k), &k).expect("enabled");
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
            "meta-agent not sweeping — no heartbeat for 11m (may be stuck)"
        );
        assert_eq!(
            wedge.notify(),
            Some("(meta-agent) not sweeping — may be stuck")
        );
        for text in [
            wedge.summary(),
            SweepAlert::RaiseUnreachable { undelivered: 7 }.summary(),
        ] {
            for banned in ["throttl", "dead", "missing"] {
                assert!(
                    !text.contains(banned),
                    "{text:?} must not carry {banned:?} — it would outrank its own \
                     'not sweeping' case in _agent_alert_reason"
                );
            }
        }
        assert_eq!(
            SweepAlert::RaiseWedge(WedgeDetail::Never {
                prompting_secs: 661
            })
            .summary(),
            "meta-agent not sweeping — never wrote a heartbeat in 11m of sweep prompts (may be \
             stuck)"
        );
        let clear = SweepAlert::ClearWedge;
        assert_eq!(clear.action(), "alert-cleared");
        assert_eq!(
            clear.summary(),
            "meta-agent sweeping again (heartbeat resumed)"
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
    fn a_stale_or_untrusted_heartbeat_is_never_reported_as_sweeping() {
        // The tri-state's whole point: only Fresh is a health claim. Inside the
        // grace the verdict is undecided (MetaStarting), past it wedged —
        // never sweeping.
        let k = knobs();
        let prior = SweepState {
            first_delivered: Some(at(0)),
            ..SweepState::default()
        };
        let cases: [(Option<u64>, u64, SweepVerdict); 6] = [
            (None, 100, SweepVerdict::MetaStarting),
            (None, 660, SweepVerdict::MetaStarting),
            (None, 661, SweepVerdict::MetaWedged),
            (None, 5000, SweepVerdict::MetaWedged),
            (Some(0), 700, SweepVerdict::MetaWedged),
            (Some(1000), 2000, SweepVerdict::MetaWedged),
        ];
        for (hb, now, want) in cases {
            let acc = sweep_step(&prior, &seen(now, hb, &k), &k).expect("enabled");
            assert_eq!(acc.verdict, want, "hb={hb:?} now={now}");
            assert_ne!(
                acc.verdict,
                SweepVerdict::MetaSweeping,
                "a non-fresh heartbeat is never health"
            );
        }
    }
}
