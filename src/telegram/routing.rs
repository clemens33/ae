//! Where an inbound Telegram message goes — and nothing about how it gets
//! there.
//!
//! Everything in this module is a PURE FUNCTION of a message and a snapshot of
//! the machine's running sessions. The network is [`super`]'s, the offset and
//! the cycle are [`super::inbound`]'s, and the threads are
//! [`super::bridge`]'s. Splitting it this way is what makes the routing rules —
//! the part a human argues about — testable without a socket, a tmux server or
//! a clock.
//!
//! # The rules, and where each comes from
//!
//! * **A slash command is always a command** (SC-945's precedence), even when
//!   it arrives as a reply. Leading whitespace is trimmed BEFORE the slash is
//!   looked for, so a message that begins with a newline is still a command.
//! * **A plain message that is a reply to a forwarded event goes to that
//!   event's agent** — the header line the outbound half wrote is the routing
//!   key, and [`parse_reply_target`] reads it.
//! * **`/use <session> <agent>` sets a sticky override; `/use clear` restores
//!   orchestrator routing** (SC-939e).
//! * **A plain message with no reply and no override goes to the running
//!   ORCHESTRATOR** (SC-939d); with no orchestrator running it gets start
//!   guidance rather than silence.
//! * **`hub` is a deprecated alias and `orchestrator` is canonical**
//!   (SC-939f): a session named `orchestrator` wins outright, a session named
//!   `hub` is still accepted, and any other meta-agent session is the last
//!   resort.
//! * **Every route is revalidated against the session it names** (SC-946/949):
//!   a target is only ever a canonical `alias:name` that this run just found in
//!   THAT session's roster, so `%pane-id`, `@other-session:agent` and
//!   `telegram:123` cannot match and therefore cannot escape.
//! * **Only RUNNING sessions are addressable** (SC-947), and a session
//!   resolves by exact name or by a unique `session_id` prefix (SC-948).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The file the sticky `/use` target is stored in, inside the machine-global
/// telegram state directory.
///
/// The NAME and the FORMAT are the frozen bash daemon's, deliberately: the
/// store is machine-global shared state (locks-census-3-aewatch I3), and a Rust
/// bridge that invented a second name would silently forget an override the
/// operator set through the bash one.
pub const TARGET_FILE: &str = "current_target";

/// The canonical orchestrator session name (#52 policy ruling, SC-939f).
const ORCHESTRATOR: &str = "orchestrator";

/// The deprecated alias, still accepted (SC-939f).
const HUB: &str = "hub";

/// One RUNNING ae session, as routing sees it.
///
/// A snapshot: every field was read at the same moment, and routing never asks
/// the world a second question part-way through a decision. Its liveness is
/// part of the snapshot too — a `RunningSession` that exists is one that was
/// running when the world was sampled (SC-947).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningSession {
    /// The session name — its directory leaf and its tmux session.
    pub name: String,
    /// Its state directory, which is where its `send` helper lives.
    pub dir: PathBuf,
    /// The `session_id` from its meta, or empty when it has none.
    pub session_id: String,
    /// Whether its meta says `meta_agent=true` — an orchestrator.
    pub meta_agent: bool,
    /// Its `main` agent as a canonical `alias:name`, when it has one.
    pub main: Option<String>,
    /// Every roster agent as a canonical `alias:name`.
    pub agents: Vec<String>,
    /// Its newest event's epoch, for `/list`'s age column.
    pub last_active: Option<i64>,
}

/// The machine's running sessions.
///
/// A trait so the routing tests can hand [`decide`] a world they wrote, rather
/// than a world they had to build out of real directories and a real tmux.
pub trait World {
    /// Every running session, sampled now.
    fn running(&self) -> Vec<RunningSession>;
}

/// A lookup that must distinguish "no match" from "more than one".
///
/// Two failures, not one, because they deserve different answers: nothing to
/// address is the operator naming something that is not there, and several
/// things to address is ae refusing to guess between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved<T> {
    /// Exactly one.
    One(T),
    /// Nothing matched.
    Missing,
    /// More than one matched, and choosing would be a guess.
    Ambiguous,
}

/// Resolve a chat-supplied session reference against the running sessions.
///
/// **The reference NEVER becomes a path.** It is matched against names and
/// session ids that were discovered by enumerating the sessions root, and the
/// directory that comes back is the one the enumeration found — so `../`,
/// an absolute path, or any other traversal simply fails to match (SC-946).
///
/// Exact name first, then a unique `session_id` prefix (SC-948). An exact name
/// wins even when some other session's id also starts with it: a name is what
/// the operator typed, and an id prefix is a convenience.
#[must_use]
pub fn resolve_session<'a>(
    sessions: &'a [RunningSession],
    reference: &str,
) -> Resolved<&'a RunningSession> {
    if reference.is_empty() {
        return Resolved::Missing;
    }
    if let Some(exact) = sessions.iter().find(|session| session.name == reference) {
        return Resolved::One(exact);
    }
    let mut prefixed = sessions.iter().filter(|session| {
        !session.session_id.is_empty() && session.session_id.starts_with(reference)
    });
    match (prefixed.next(), prefixed.next()) {
        (Some(one), None) => Resolved::One(one),
        (Some(_), Some(_)) => Resolved::Ambiguous,
        _ => Resolved::Missing,
    }
}

/// Resolve a chat-supplied agent reference INSIDE one session, and canonicalise
/// it.
///
/// **This is the escape gate (SC-949).** The session helpers' own grammar is
/// broader than this one — it accepts `%pane-id`, `@session:agent` and external
/// prefixes — so a raw chat value handed to the helper could address a pane in
/// a session the operator never named. Nothing raw is ever passed on: the value
/// must match an `alias:name` this session's roster actually holds, and what
/// gets delivered is the roster's spelling rather than the chat's.
///
/// Exact `alias:name`, else a unique bare `name`. Two agents sharing a bare
/// name is [`Resolved::Ambiguous`] rather than a coin flip.
#[must_use]
pub fn resolve_agent(session: &RunningSession, want: &str) -> Resolved<String> {
    if want.is_empty() {
        return Resolved::Missing;
    }
    if let Some(exact) = session.agents.iter().find(|agent| *agent == want) {
        return Resolved::One(exact.clone());
    }
    let mut bare = session
        .agents
        .iter()
        .filter(|agent| agent.split_once(':').is_some_and(|(_, name)| name == want));
    match (bare.next(), bare.next()) {
        (Some(one), None) => Resolved::One(one.clone()),
        (Some(_), Some(_)) => Resolved::Ambiguous,
        _ => Resolved::Missing,
    }
}

/// The running orchestrator plain text defaults to (SC-939d), or `None`.
///
/// **SC-939f's precedence, and it is a precedence rather than a rename.** A
/// session actually named `orchestrator` wins outright; a session named `hub`
/// is the deprecated alias and still works; any other meta-agent session is the
/// last resort, so a machine whose orchestrator is called something else is not
/// left unaddressable. A session with no `main` agent is not an orchestrator
/// anyone can talk to and is skipped.
#[must_use]
pub fn find_orchestrator(sessions: &[RunningSession]) -> Option<&RunningSession> {
    let eligible = || {
        sessions
            .iter()
            .filter(|session| session.meta_agent && session.main.is_some())
    };
    eligible()
        .find(|session| session.name == ORCHESTRATOR)
        .or_else(|| eligible().find(|session| session.name == HUB))
        .or_else(|| eligible().next())
}

/// The sticky `/use` override, as it stands on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sticky {
    /// No override: plain text goes to the orchestrator.
    Unset,
    /// An override the operator set.
    Set {
        /// The session reference it names — REVALIDATED on every use, never
        /// trusted because it was valid when it was written.
        session: String,
        /// The agent reference it names.
        agent: String,
    },
    /// The file is there and is not a target. Reported rather than ignored:
    /// silently falling back to the orchestrator would route a message
    /// somewhere the operator did not choose.
    Corrupt,
}

/// Read the sticky target from `path`.
///
/// The frozen `<session>\t<agent>` line. A file that is absent is
/// [`Sticky::Unset`]; a file that is present and not that shape is
/// [`Sticky::Corrupt`] — and an unreadable one is corrupt too, because "I
/// cannot tell what the override is" and "there is no override" are different
/// facts and only one of them permits routing to the orchestrator.
///
/// # The empty field that vanishes
///
/// Split on the tab EXACTLY ONCE and require both halves non-empty. A
/// whitespace-run split would turn `"\tagent"` into a single field and read the
/// agent as the session — the frozen bash TSV framing hazard, one layer up.
#[must_use]
pub fn read_sticky(path: &Path) -> Sticky {
    match super::read_regular_file(path) {
        Ok((text, _)) => parse_sticky(&text),
        Err(super::NotRegular::Absent) => Sticky::Unset,
        Err(_) => Sticky::Corrupt,
    }
}

/// [`read_sticky`]'s parse, over text that is already in hand.
#[must_use]
pub fn parse_sticky(text: &str) -> Sticky {
    let line = text.lines().next().unwrap_or_default();
    match line.split_once('\t') {
        Some((session, agent)) if !session.is_empty() && !agent.is_empty() => Sticky::Set {
            session: session.to_owned(),
            agent: agent.to_owned(),
        },
        _ => Sticky::Corrupt,
    }
}

/// The line a sticky target is stored as.
#[must_use]
pub fn render_sticky(session: &str, agent: &str) -> String {
    format!("{session}\t{agent}\n")
}

/// One inbound message, reduced to what routing actually reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inbound<'a> {
    /// The message text, as sent.
    pub text: &'a str,
    /// The text of the message this one replies to, when it is a reply.
    pub reply_to: Option<&'a str>,
}

/// Which helper a delivery runs — the operator's verb, carried, not collapsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// Deliver the message and record a `send`. Every route but an explicit
    /// `/session <ref> ask …` is this.
    Send,
    /// Open a TRACKED REQUEST: a request id, an `ask` event, and a reply
    /// command embedded in the message so the agent can answer back through
    /// the chat. Not a decorated send — a different helper with different
    /// semantics, which is why the verb has to survive routing rather than be
    /// validated and discarded.
    Ask,
}

impl Verb {
    /// The helper this verb runs, as a LITERAL.
    ///
    /// The two names are the only two values this can ever produce, so a verb
    /// arriving from a chat message can select between them and can never
    /// become one: see [`super::bridge::Helper`], where the result is joined
    /// onto the session directory.
    #[must_use]
    pub const fn helper(self) -> &'static str {
        match self {
            Self::Send => "send",
            Self::Ask => "ask",
        }
    }

    /// How an acknowledgement names what it did.
    #[must_use]
    pub const fn past(self) -> &'static str {
        match self {
            Self::Send => "send delivered",
            Self::Ask => "ask opened",
        }
    }
}

/// What one authorized message should cause.
///
/// A VALUE, not an effect: [`decide`] chooses, and the caller performs. The
/// separation is what lets every routing rule be tested without delivering
/// anything anywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Deliver `text` to `agent` in the session at `dir`, through that
    /// session's own `send` helper.
    Deliver {
        /// Which helper runs — the verb the operator actually typed, carried
        /// through rather than collapsed (a `/session … ask` that ran `send`
        /// would silently drop the tracked request the operator asked for).
        verb: Verb,
        /// The resolved session's name, for the acknowledgement.
        session: String,
        /// Its state directory — where the helper lives.
        dir: PathBuf,
        /// The canonical `alias:name`, as the roster spells it.
        agent: String,
        /// The message body.
        text: String,
    },
    /// Set the sticky override to a target that has just been revalidated.
    Use {
        /// The resolved session's name.
        session: String,
        /// The canonical agent reference.
        agent: String,
    },
    /// Clear the sticky override (SC-939e's `/use clear`).
    Unuse,
    /// Answer the chat and change nothing else.
    Answer(String),
}

/// Decide where a message goes.
///
/// `now` is passed in rather than read, so `/list`'s ages are a function of the
/// caller's clock and this stays pure.
#[must_use]
pub fn decide(
    message: Inbound<'_>,
    sessions: &[RunningSession],
    sticky: &Sticky,
    now: i64,
) -> Route {
    // Trim FIRST, and trim both ends. The slash test below decides whether this
    // is a command at all, and a message that begins with a newline — which is
    // what a phone keyboard produces more often than anyone expects — must not
    // stop being one.
    let text = message.text.trim();

    // SC-945's precedence, and the order is the rule: a slash command is a
    // command even when it is sent as a reply, so the reply branch is only
    // reachable for text that is not one.
    if !text.starts_with('/')
        && let Some(quoted) = message.reply_to.filter(|quoted| !quoted.is_empty())
    {
        return reply_route(quoted, text, sessions);
    }

    if let Some(rest) = text.strip_prefix('@') {
        return at_route(rest, sessions);
    }

    let (word, rest) = split_word(text);
    // Tolerate `/cmd@botname`, which is what Telegram sends in a group even
    // when the bot is addressed directly.
    let command = word.split_once('@').map_or(word, |(head, _)| head);
    match command {
        "/help" | "" => Route::Answer(HELP.to_owned()),
        "/list" => Route::Answer(render_list(sessions, now)),
        "/use" => use_route(rest, sessions, sticky),
        "/session" => session_route(rest, sessions),
        other if other.starts_with('/') => {
            Route::Answer(format!("Unknown command: {other} — try /help"))
        }
        // Plain text: the sticky override if one is set, else the orchestrator.
        _ => sticky_route(text, sessions, sticky),
    }
}

/// The `/help` body.
///
/// ORCHESTRATOR throughout: `hub` is accepted as an alias but is never taught
/// (SC-939f). A help text is where a deprecated name gets a second life.
const HELP: &str = "ae telegram:\n\
     • Plain text → your orchestrator (auto-default; just talk to it).\n\
     • Reply to any forwarded message → answers that agent (no command).\n\
     • @session:agent <msg> → send to an agent directly.\n\
     • /use <session> <agent> → redirect plain messages elsewhere; /use clear → back to the \
     orchestrator.\n\
     /list\n\
     /session <name|id-prefix> send|ask <agent> <msg>";

/// A plain message sent as a REPLY to a forwarded event: route it to the agent
/// that event came from.
fn reply_route(quoted: &str, text: &str, sessions: &[RunningSession]) -> Route {
    let Some((session, actor)) = parse_reply_target(quoted) else {
        return Route::Answer(
            "Couldn't tell which agent that was a reply to — use /session <name> send <agent> \
             <msg>"
                .to_owned(),
        );
    };
    deliver_route(Verb::Send, &session, &actor, text, sessions)
}

/// `@session:agent <msg>` — the compact prefix.
///
/// The session is everything up to the FIRST colon and the agent is all the
/// rest, so an `alias:name` keeps its own colon.
fn at_route(rest: &str, sessions: &[RunningSession]) -> Route {
    let (token, message) = split_word(rest);
    let Some((session, agent)) = token.split_once(':') else {
        return Route::Answer(
            "Usage: @session:agent <msg>  (e.g. @mdk:claude:lead do X)".to_owned(),
        );
    };
    if message.is_empty() {
        return Route::Answer("Nothing to send — @session:agent <msg>".to_owned());
    }
    deliver_route(Verb::Send, session, agent, message, sessions)
}

/// `/session <ref> <send|ask> <agent> <msg…>`.
///
/// THE VERB IS ACTED ON. `send` runs the session's `send` helper; `ask` runs
/// its `ask` helper, which opens a tracked request — a request id, an `ask`
/// event, and a reply command embedded in the message, which is the route by
/// which the agent's answer comes back to the chat. An earlier version accepted
/// `ask` and ran `send`, so the request the operator asked for was never opened
/// and the documented reply route could not happen; validating a verb and then
/// discarding it is worse than rejecting it, because the operator is told it
/// worked.
fn session_route(args: &str, sessions: &[RunningSession]) -> Route {
    let (reference, rest) = split_word(args);
    let (verb, rest) = split_word(rest);
    let (agent, message) = split_word(rest);
    // THE MESSAGE IS THE ONLY PART THAT CAN DECIDE, and testing the other three
    // would be testing nothing. [`split_word`] consumes left to right, so a
    // part that is missing empties every part AFTER it: a non-empty `message`
    // is proof that the reference, the verb and the agent were all present.
    // An earlier version checked all four with `||`, which reads as four
    // independent conditions; cargo-mutants showed two of those `||`s were
    // EQUIVALENT MUTANTS — no input reachable through [`decide`] can tell them
    // apart, because a caller cannot supply an empty reference alongside a
    // non-empty message. Unreachable logic is worth deleting, not covering.
    if message.is_empty() {
        return Route::Answer(
            "Usage: /session <name|id-prefix> <send|ask> <agent> <msg>".to_owned(),
        );
    }
    let verb = match verb {
        "send" => Verb::Send,
        "ask" => Verb::Ask,
        other => return Route::Answer(format!("Unknown verb '{other}' (use send or ask)")),
    };
    deliver_route(verb, reference, agent, message, sessions)
}

/// `/use`, `/use clear`, `/use <session> <agent>` (SC-939e).
fn use_route(args: &str, sessions: &[RunningSession], sticky: &Sticky) -> Route {
    let (reference, rest) = split_word(args);
    if reference.is_empty() {
        return Route::Answer(describe_sticky(sessions, sticky));
    }
    if matches!(reference, "clear" | "off" | "none") {
        return Route::Unuse;
    }
    let (agent, _) = split_word(rest);
    if agent.is_empty() {
        return Route::Answer("Usage: /use <session> <agent>   (or /use clear)".to_owned());
    }
    match revalidate(reference, agent, sessions) {
        Ok((session, canonical)) => Route::Use {
            session: session.name.clone(),
            agent: canonical,
        },
        Err(refusal) => Route::Answer(refusal),
    }
}

/// What `/use` with no arguments reports.
fn describe_sticky(sessions: &[RunningSession], sticky: &Sticky) -> String {
    match sticky {
        Sticky::Set { session, agent } => format!(
            "Current target: {session} → {agent}  (plain messages go here; /use clear to fall \
             back to the orchestrator)"
        ),
        Sticky::Corrupt => "Current target is unset/corrupt — /use <session> <agent>".to_owned(),
        Sticky::Unset => match find_orchestrator(sessions) {
            Some(session) => format!(
                "No override set — plain messages go to the orchestrator ({} → {}). /use \
                 <session> <agent> to redirect.",
                session.name,
                session.main.as_deref().unwrap_or_default()
            ),
            None => "No override set and no orchestrator running. Start one with 'ae \
                     orchestrator', or /use <session> <agent>."
                .to_owned(),
        },
    }
}

/// Plain text with no reply: the sticky override, else the orchestrator
/// (SC-939d).
fn sticky_route(text: &str, sessions: &[RunningSession], sticky: &Sticky) -> Route {
    match sticky {
        // REVALIDATED, every time. An override is a note about the past: the
        // session it names may have stopped, and its agent may have been
        // retired, since it was written.
        Sticky::Set { session, agent } => deliver_route(Verb::Send, session, agent, text, sessions),
        Sticky::Corrupt => {
            Route::Answer("Current target is unset/corrupt — /use <session> <agent>".to_owned())
        }
        Sticky::Unset => match find_orchestrator(sessions) {
            Some(session) => deliver_route(
                Verb::Send,
                &session.name,
                session.main.as_deref().unwrap_or_default(),
                text,
                sessions,
            ),
            None => Route::Answer(
                "No orchestrator running and no /use target set. Start one with 'ae \
                 orchestrator' (then just talk to it), reply to a message, use @session:agent \
                 <msg>, or /use <session> <agent>. (/help)"
                    .to_owned(),
            ),
        },
    }
}

/// Resolve a session+agent pair and build the delivery, or the refusal that
/// explains which half did not resolve.
fn deliver_route(
    verb: Verb,
    reference: &str,
    agent: &str,
    text: &str,
    sessions: &[RunningSession],
) -> Route {
    match revalidate(reference, agent, sessions) {
        Ok((session, canonical)) => Route::Deliver {
            verb,
            session: session.name.clone(),
            dir: session.dir.clone(),
            agent: canonical,
            text: text.to_owned(),
        },
        Err(refusal) => Route::Answer(refusal),
    }
}

/// **THE ONE REVALIDATION** every route passes through (SC-946).
///
/// Session first, then agent WITHIN that session. One function rather than one
/// per command, because a route that skipped it would be a route that could
/// address a pane the operator never named — and the way that happens is a
/// second copy of this logic that forgot a case.
fn revalidate<'a>(
    reference: &str,
    agent: &str,
    sessions: &'a [RunningSession],
) -> Result<(&'a RunningSession, String), String> {
    let session = match resolve_session(sessions, reference) {
        Resolved::One(session) => session,
        Resolved::Ambiguous => {
            return Err(format!(
                "Ambiguous session '{reference}' — be more specific"
            ));
        }
        Resolved::Missing => {
            return Err(format!("No running session matching '{reference}'"));
        }
    };
    match resolve_agent(session, agent) {
        Resolved::One(canonical) => Ok((session, canonical)),
        Resolved::Ambiguous => Err(format!(
            "Ambiguous agent '{agent}' in {} — use alias:name",
            session.name
        )),
        Resolved::Missing => Err(format!("No agent '{agent}' in session {}", session.name)),
    }
}

/// Read the header line of a message THIS bridge forwarded, and recover the
/// session and agent it came from.
///
/// # The grammar is the RENDERER's, not a guess
///
/// The Rust outbound half writes `[<session>] <actor>` as line one
/// (`Outbound::render`), so the actor is the first token after `] `. The frozen
/// bash bridge wrote `[<session>] <action>  <actor> [→ <target>]` — two spaces
/// between action and actor — and messages in that shape can still be sitting
/// in the operator's chat history, replyable, on the day the Rust bridge takes
/// over. Both are accepted, and the DOUBLE SPACE is what tells them apart: it
/// is in the frozen format by construction and cannot occur in the Rust one,
/// whose two fields are separated by exactly one.
///
/// A header this cannot read yields `None`, and the caller says so. Guessing
/// would mean delivering a human's message to an agent picked by accident.
#[must_use]
pub fn parse_reply_target(quoted: &str) -> Option<(String, String)> {
    let first = quoted.lines().next()?;
    let inner = first.strip_prefix('[')?;
    let (session, rest) = inner.split_once("] ")?;
    if session.is_empty() {
        return None;
    }
    let actor = match rest.split_once("  ") {
        // The frozen shape: skip the action, take the actor.
        Some((_, after)) => split_word(after).0,
        // The Rust shape: the actor is all there is.
        None => split_word(rest).0,
    };
    if actor.is_empty() {
        return None;
    }
    Some((session.to_owned(), actor.to_owned()))
}

/// `/list`: the running sessions, with their short ids and ages.
fn render_list(sessions: &[RunningSession], now: i64) -> String {
    if sessions.is_empty() {
        return "Running sessions:\n(no running sessions)".to_owned();
    }
    let mut out = String::from("Running sessions:\n");
    for session in sessions {
        let short: String = session.session_id.chars().take(8).collect();
        let age = relative_age(now, session.last_active);
        // Writing to a `String` cannot fail; the result is discarded rather
        // than unwrapped so this stays free of a panic path.
        let _ = writeln!(out, "• {}  [{short}]  {age}", session.name);
    }
    out
}

/// The frozen telegram daemon's age wording, which is NOT the listing's: this
/// surface calls anything under ninety seconds "just now".
fn relative_age(now: i64, at: Option<i64>) -> String {
    let Some(at) = at else {
        return "-".to_owned();
    };
    let delta = now.saturating_sub(at);
    if delta < 90 {
        "just now".to_owned()
    } else if delta < 3_600 {
        format!("{}m ago", delta / 60)
    } else if delta < 86_400 {
        format!("{}h ago", delta / 3_600)
    } else {
        format!("{}d ago", delta / 86_400)
    }
}

/// Split off the first whitespace-delimited word, returning it and the
/// remainder with its leading whitespace already trimmed.
fn split_word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    match text.find(char::is_whitespace) {
        Some(at) => (&text[..at], text[at..].trim_start()),
        None => (text, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Inbound, ORCHESTRATOR, Resolved, Route, RunningSession, Sticky, Verb, decide,
        find_orchestrator, parse_reply_target, parse_sticky, render_sticky, resolve_agent,
        resolve_session, split_word,
    };
    use std::path::PathBuf;

    const NOW: i64 = 1_800_000_000;

    fn session(name: &str, agents: &[&str]) -> RunningSession {
        RunningSession {
            name: name.to_owned(),
            dir: PathBuf::from("/sessions").join(name),
            session_id: format!("{name}-0123456789abcdef"),
            meta_agent: false,
            main: agents.first().map(|agent| (*agent).to_owned()),
            agents: agents.iter().map(|agent| (*agent).to_owned()).collect(),
            last_active: Some(NOW),
        }
    }

    fn orchestrator(name: &str) -> RunningSession {
        let mut session = session(name, &["claude:lead"]);
        session.meta_agent = true;
        session
    }

    fn plain(text: &str) -> Inbound<'_> {
        Inbound {
            text,
            reply_to: None,
        }
    }

    #[test]
    fn plain_text_with_no_override_goes_to_the_running_orchestrator() {
        // SC-939d.
        let world = vec![session("work", &["codex:dev"]), orchestrator(ORCHESTRATOR)];
        let route = decide(plain("status please"), &world, &Sticky::Unset, NOW);
        assert_eq!(
            route,
            Route::Deliver {
                verb: Verb::Send,
                session: ORCHESTRATOR.to_owned(),
                dir: PathBuf::from("/sessions/orchestrator"),
                agent: "claude:lead".to_owned(),
                text: "status please".to_owned(),
            }
        );
    }

    #[test]
    fn plain_text_with_no_orchestrator_is_start_guidance_rather_than_silence() {
        // SC-939d's second half: the absence has to SAY something.
        let world = vec![session("work", &["codex:dev"])];
        let Route::Answer(text) = decide(plain("hello"), &world, &Sticky::Unset, NOW) else {
            panic!("a message with nowhere to go must be answered, not delivered");
        };
        assert!(text.contains("No orchestrator running"), "{text}");
        assert!(text.contains("ae orchestrator"), "{text}");
    }

    #[test]
    fn the_canonical_orchestrator_beats_the_deprecated_hub_and_hub_beats_nothing() {
        // SC-939f: `hub` still WORKS, and `orchestrator` still WINS.
        let both = vec![orchestrator("hub"), orchestrator(ORCHESTRATOR)];
        assert_eq!(
            find_orchestrator(&both).map(|session| session.name.clone()),
            Some(ORCHESTRATOR.to_owned())
        );
        let legacy = vec![session("work", &["codex:dev"]), orchestrator("hub")];
        assert_eq!(
            find_orchestrator(&legacy).map(|session| session.name.clone()),
            Some("hub".to_owned()),
            "the deprecated alias must still be addressable"
        );
        // And a meta-agent session under any other name is the last resort,
        // so an operator who renamed theirs is not locked out.
        let other = vec![orchestrator("brain")];
        assert_eq!(
            find_orchestrator(&other).map(|session| session.name.clone()),
            Some("brain".to_owned())
        );
    }

    #[test]
    fn the_deprecated_hub_outranks_any_other_meta_agent_session() {
        // SC-939f's MIDDLE rung, which the last-resort fallback hides: with a
        // `hub` and some other meta-agent session both running, `hub` must win
        // — otherwise "the alias is still accepted" degrades into "whichever
        // session the scan happened to list first".
        let world = vec![orchestrator("brain"), orchestrator("hub")];
        assert_eq!(
            find_orchestrator(&world).map(|session| session.name.clone()),
            Some("hub".to_owned()),
            "the deprecated alias must outrank an unnamed meta-agent session"
        );
        // And the canonical name still beats both.
        let all = vec![
            orchestrator("brain"),
            orchestrator("hub"),
            orchestrator(ORCHESTRATOR),
        ];
        assert_eq!(
            find_orchestrator(&all).map(|session| session.name.clone()),
            Some(ORCHESTRATOR.to_owned())
        );
    }

    #[test]
    fn the_session_command_needs_every_one_of_its_four_parts() {
        // One missing part at a time. A usage check written with `&&` instead
        // of `||` would only refuse the command that omits EVERYTHING, and
        // would deliver an empty message for the rest.
        let world = vec![session("work", &["codex:dev"])];
        for (label, typed) in [
            ("no message", "/session work send dev"),
            ("no agent", "/session work send"),
            ("no verb", "/session work"),
            ("nothing at all", "/session"),
        ] {
            let route = decide(plain(typed), &world, &Sticky::Unset, NOW);
            let Route::Answer(text) = route else {
                panic!("{label}: an incomplete command must not deliver");
            };
            assert!(text.starts_with("Usage: /session"), "{label}: {text}");
        }
    }

    #[test]
    fn use_with_no_arguments_reports_where_plain_messages_are_going() {
        // All three states, because each is a different answer to the same
        // question and an empty string would be a plausible-looking bug.
        let world = vec![session("work", &["codex:dev"]), orchestrator(ORCHESTRATOR)];
        let set = Sticky::Set {
            session: "work".to_owned(),
            agent: "codex:dev".to_owned(),
        };
        let Route::Answer(current) = decide(plain("/use"), &world, &set, NOW) else {
            panic!("/use answers");
        };
        assert!(
            current.contains("Current target: work → codex:dev"),
            "{current}"
        );
        assert!(current.contains("/use clear"), "{current}");

        let Route::Answer(default) = decide(plain("/use"), &world, &Sticky::Unset, NOW) else {
            panic!("/use answers");
        };
        assert!(default.contains("No override set"), "{default}");
        assert!(default.contains("orchestrator → claude:lead"), "{default}");

        let Route::Answer(none) = decide(plain("/use"), &[], &Sticky::Unset, NOW) else {
            panic!("/use answers");
        };
        assert!(none.contains("no orchestrator running"), "{none}");

        let Route::Answer(broken) = decide(plain("/use"), &world, &Sticky::Corrupt, NOW) else {
            panic!("/use answers");
        };
        assert!(broken.contains("unset/corrupt"), "{broken}");
    }

    #[test]
    fn every_age_band_has_its_own_wording_and_its_own_boundary() {
        // The bands are what `/list` is FOR, and an off-by-one at a boundary
        // reads as a plausible age rather than as a bug.
        use super::relative_age;
        assert_eq!(relative_age(NOW, None), "-");
        assert_eq!(relative_age(NOW, Some(NOW)), "just now");
        assert_eq!(relative_age(NOW, Some(NOW - 89)), "just now");
        assert_eq!(relative_age(NOW, Some(NOW - 90)), "1m ago");
        assert_eq!(relative_age(NOW, Some(NOW - 3_599)), "59m ago");
        assert_eq!(relative_age(NOW, Some(NOW - 3_600)), "1h ago");
        assert_eq!(relative_age(NOW, Some(NOW - 86_399)), "23h ago");
        assert_eq!(relative_age(NOW, Some(NOW - 86_400)), "1d ago");
        assert_eq!(relative_age(NOW, Some(NOW - 604_800)), "7d ago");
        // A clock that went backwards is still under the first bound.
        assert_eq!(relative_age(NOW, Some(NOW + 60)), "just now");
    }

    #[test]
    fn an_orchestrator_with_no_main_agent_is_not_a_destination() {
        let mut headless = orchestrator(ORCHESTRATOR);
        headless.main = None;
        headless.agents.clear();
        assert!(find_orchestrator(&[headless]).is_none());
    }

    #[test]
    fn use_sets_an_override_and_clear_restores_orchestrator_routing() {
        // SC-939e, both halves.
        let world = vec![session("work", &["codex:dev"]), orchestrator(ORCHESTRATOR)];
        assert_eq!(
            decide(plain("/use work dev"), &world, &Sticky::Unset, NOW),
            Route::Use {
                session: "work".to_owned(),
                agent: "codex:dev".to_owned(),
            },
            "the override must store the CANONICAL agent, not what was typed"
        );
        for spelling in ["/use clear", "/use off", "/use none"] {
            assert_eq!(
                decide(plain(spelling), &world, &Sticky::Unset, NOW),
                Route::Unuse,
                "{spelling}"
            );
        }
    }

    #[test]
    fn an_override_redirects_plain_text_away_from_the_orchestrator() {
        let world = vec![session("work", &["codex:dev"]), orchestrator(ORCHESTRATOR)];
        let sticky = Sticky::Set {
            session: "work".to_owned(),
            agent: "codex:dev".to_owned(),
        };
        let Route::Deliver { session, agent, .. } = decide(plain("ping"), &world, &sticky, NOW)
        else {
            panic!("an override must route");
        };
        assert_eq!((session.as_str(), agent.as_str()), ("work", "codex:dev"));
    }

    #[test]
    fn an_override_naming_a_session_that_has_stopped_is_refused_not_followed() {
        // The override is revalidated on every use — it is a note about the
        // past, and the past is where a stopped session lives.
        let world = vec![orchestrator(ORCHESTRATOR)];
        let sticky = Sticky::Set {
            session: "gone".to_owned(),
            agent: "codex:dev".to_owned(),
        };
        let Route::Answer(text) = decide(plain("ping"), &world, &sticky, NOW) else {
            panic!("a stale override must not deliver");
        };
        assert!(
            text.contains("No running session matching 'gone'"),
            "{text}"
        );
    }

    #[test]
    fn a_slash_command_stays_a_command_even_as_a_reply() {
        // SC-945's precedence. The reply header below is a perfectly good one:
        // the point is that it is not consulted.
        let world = vec![session("work", &["codex:dev"])];
        let route = decide(
            Inbound {
                text: "  \n /list",
                reply_to: Some("[work] codex:dev\nsomething happened"),
            },
            &world,
            &Sticky::Unset,
            NOW,
        );
        let Route::Answer(text) = route else {
            panic!("a slash command sent as a reply must not be delivered as a reply");
        };
        assert!(text.starts_with("Running sessions:"), "{text}");
    }

    #[test]
    fn a_plain_reply_goes_to_the_agent_the_quoted_event_came_from() {
        let world = vec![session("work", &["codex:dev"]), orchestrator(ORCHESTRATOR)];
        let route = decide(
            Inbound {
                text: "yes, go ahead",
                reply_to: Some("[work] codex:dev\nready?"),
            },
            &world,
            &Sticky::Unset,
            NOW,
        );
        assert_eq!(
            route,
            Route::Deliver {
                verb: Verb::Send,
                session: "work".to_owned(),
                dir: PathBuf::from("/sessions/work"),
                agent: "codex:dev".to_owned(),
                text: "yes, go ahead".to_owned(),
            },
            "a reply must beat the orchestrator default"
        );
    }

    #[test]
    fn the_reply_header_is_read_in_both_the_rust_and_the_frozen_bash_shapes() {
        // The Rust renderer emits `[session] actor`; the frozen bash emitted
        // `[session] action  actor → target`, and those messages are still
        // replyable in the operator's chat on flip day.
        assert_eq!(
            parse_reply_target("[work] codex:dev\nbody"),
            Some(("work".to_owned(), "codex:dev".to_owned()))
        );
        assert_eq!(
            parse_reply_target("[work] send  codex:dev → claude:lead\nbody"),
            Some(("work".to_owned(), "codex:dev".to_owned())),
            "the frozen two-space header must not be read as an actor of 'send'"
        );
        for junk in ["", "no bracket", "[unterminated", "[work] ", "[] actor"] {
            assert_eq!(parse_reply_target(junk), None, "{junk:?}");
        }
    }

    #[test]
    fn an_unreadable_reply_header_is_answered_rather_than_guessed_at() {
        let world = vec![session("work", &["codex:dev"]), orchestrator(ORCHESTRATOR)];
        let Route::Answer(text) = decide(
            Inbound {
                text: "thanks",
                reply_to: Some("a message this bridge did not write"),
            },
            &world,
            &Sticky::Unset,
            NOW,
        ) else {
            panic!("an unreadable header must not fall through to a delivery");
        };
        assert!(text.contains("Couldn't tell which agent"), "{text}");
    }

    #[test]
    fn an_agent_reference_can_never_escape_the_session_it_was_resolved_in() {
        // SC-949. Every one of these is a legal target for the session
        // helpers' own grammar, and none of them may resolve here.
        let world = vec![session("work", &["codex:dev"]), session("other", &["cl:x"])];
        for escape in [
            "%3",
            "@other:cl:x",
            "telegram:123",
            "cl:x",
            "../other/send",
            "",
        ] {
            let route = decide(
                plain(&format!("@work:{escape} do it")),
                &world,
                &Sticky::Unset,
                NOW,
            );
            assert!(
                matches!(route, Route::Answer(_)),
                "{escape} resolved to a delivery: {route:?}"
            );
        }
    }

    #[test]
    fn a_bare_agent_name_resolves_only_while_it_is_unique() {
        let one = session("work", &["codex:dev", "claude:lead"]);
        assert_eq!(
            resolve_agent(&one, "dev"),
            Resolved::One("codex:dev".to_owned())
        );
        let two = session("work", &["codex:dev", "claude:dev"]);
        assert_eq!(resolve_agent(&two, "dev"), Resolved::Ambiguous);
        assert_eq!(
            resolve_agent(&two, "codex:dev"),
            Resolved::One("codex:dev".to_owned())
        );
        assert_eq!(resolve_agent(&two, "nobody"), Resolved::Missing);
    }

    #[test]
    fn a_session_resolves_by_exact_name_first_and_then_by_unique_id_prefix() {
        // SC-948, and the precedence within it: an exact NAME is what the
        // operator typed.
        let world = vec![session("work", &["codex:dev"]), session("other", &["cl:x"])];
        assert!(matches!(
            resolve_session(&world, "work"),
            Resolved::One(found) if found.name == "work"
        ));
        assert!(matches!(
            resolve_session(&world, "other-0123"),
            Resolved::One(found) if found.name == "other"
        ));
        assert_eq!(
            std::mem::discriminant(&resolve_session(&world, "nope")),
            std::mem::discriminant(&Resolved::Missing)
        );
    }

    #[test]
    fn two_sessions_sharing_an_id_prefix_are_ambiguous_rather_than_a_coin_flip() {
        let mut first = session("one", &["cl:a"]);
        let mut second = session("two", &["cl:b"]);
        first.session_id = "abc111".to_owned();
        second.session_id = "abc222".to_owned();
        assert_eq!(
            std::mem::discriminant(&resolve_session(&[first, second], "abc")),
            std::mem::discriminant(&Resolved::Ambiguous)
        );
    }

    #[test]
    fn a_session_with_no_id_is_never_matched_by_an_empty_looking_prefix() {
        // An empty `session_id` must not prefix-match everything.
        let mut nameless = session("work", &["cl:a"]);
        nameless.session_id = String::new();
        assert_eq!(
            std::mem::discriminant(&resolve_session(&[nameless], "x")),
            std::mem::discriminant(&Resolved::Missing)
        );
    }

    #[test]
    fn the_session_command_validates_its_verb_and_its_shape() {
        let world = vec![session("work", &["codex:dev"])];
        let Route::Answer(bad_verb) = decide(
            plain("/session work poke dev hi"),
            &world,
            &Sticky::Unset,
            NOW,
        ) else {
            panic!("an unknown verb must not deliver");
        };
        assert!(bad_verb.contains("Unknown verb 'poke'"), "{bad_verb}");
        let Route::Answer(usage) = decide(plain("/session work send"), &world, &Sticky::Unset, NOW)
        else {
            panic!("a short command must not deliver");
        };
        assert!(usage.starts_with("Usage: /session"), "{usage}");
        assert!(matches!(
            decide(
                plain("/session work send dev hello"),
                &world,
                &Sticky::Unset,
                NOW
            ),
            Route::Deliver { .. }
        ));
    }

    #[test]
    fn an_unknown_slash_command_is_named_back_rather_than_routed_as_text() {
        let world = vec![orchestrator(ORCHESTRATOR)];
        let Route::Answer(text) = decide(plain("/frobnicate"), &world, &Sticky::Unset, NOW) else {
            panic!("an unknown command must not become a message to the orchestrator");
        };
        assert!(text.contains("Unknown command: /frobnicate"), "{text}");
    }

    #[test]
    fn a_command_addressed_to_the_bot_by_name_is_still_that_command() {
        let world = vec![orchestrator(ORCHESTRATOR)];
        let Route::Answer(text) = decide(plain("/list@ae_bot"), &world, &Sticky::Unset, NOW) else {
            panic!("/list@bot must be /list");
        };
        assert!(text.starts_with("Running sessions:"), "{text}");
    }

    #[test]
    fn the_help_text_teaches_orchestrator_and_never_hub() {
        // SC-939f: the alias is accepted, not taught.
        let Route::Answer(help) = decide(plain("/help"), &[], &Sticky::Unset, NOW) else {
            panic!("/help answers");
        };
        assert!(help.contains("orchestrator"), "{help}");
        assert!(
            !help.contains("hub"),
            "the deprecated name is being taught: {help}"
        );
        // An empty message is help too, exactly as the frozen bridge had it.
        assert!(matches!(
            decide(plain("   "), &[], &Sticky::Unset, NOW),
            Route::Answer(_)
        ));
    }

    #[test]
    fn list_reports_every_running_session_with_its_short_id_and_age() {
        let mut stale = session("old", &["cl:a"]);
        stale.session_id = "0123456789abcdef".to_owned();
        stale.last_active = Some(NOW - 7_200);
        let Route::Answer(text) = decide(plain("/list"), &[stale], &Sticky::Unset, NOW) else {
            panic!("/list answers");
        };
        assert!(text.contains("• old  [01234567]  2h ago"), "{text}");
        let Route::Answer(empty) = decide(plain("/list"), &[], &Sticky::Unset, NOW) else {
            panic!("/list answers");
        };
        assert!(empty.contains("(no running sessions)"), "{empty}");
    }

    #[test]
    fn a_corrupt_override_is_reported_rather_than_silently_ignored() {
        // Falling back to the orchestrator here would route a message to
        // somewhere the operator did not choose.
        let world = vec![orchestrator(ORCHESTRATOR)];
        let Route::Answer(text) = decide(plain("hello"), &world, &Sticky::Corrupt, NOW) else {
            panic!("a corrupt override must not fall through to the orchestrator");
        };
        assert!(text.contains("unset/corrupt"), "{text}");
    }

    #[test]
    fn the_sticky_line_round_trips_and_refuses_a_half_empty_one() {
        assert_eq!(render_sticky("work", "codex:dev"), "work\tcodex:dev\n");
        assert_eq!(
            parse_sticky(&render_sticky("work", "codex:dev")),
            Sticky::Set {
                session: "work".to_owned(),
                agent: "codex:dev".to_owned(),
            }
        );
        // The TSV framing hazard: a leading empty field must not shift the
        // agent into the session's place.
        for broken in ["\tcodex:dev\n", "work\t\n", "work\n", "\n", ""] {
            assert_eq!(parse_sticky(broken), Sticky::Corrupt, "{broken:?}");
        }
    }

    #[test]
    fn splitting_a_word_trims_both_sides_of_the_gap() {
        assert_eq!(split_word("  one   two three "), ("one", "two three "));
        assert_eq!(split_word("solo"), ("solo", ""));
        assert_eq!(split_word("   "), ("", ""));
    }
}
