//! Where an inbound Telegram message goes — and nothing about how it gets
//! there.
//!
//! Everything in this module is a PURE FUNCTION of a message and a snapshot of
//! the machine's running sessions. The network is [`super`]'s, the offset and
//! the cycle are [`super::inbound`]'s, and the threads are
//! [`super::bridge`]'s. Splitting it this way is what makes the routing rules —
//! the part a human argues about — testable without a socket, a tmux server or
//! a clock.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The file the sticky `/use` target is stored in, inside the machine-global
/// telegram state directory.
pub const TARGET_FILE: &str = "current_target";

/// One RUNNING ae session, as routing sees it.
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
    /// Its lead pair's seat names, main first: where its shared chat asks.
    /// Empty when it has none, and then it takes no ask.
    pub pair: Vec<String>,
    /// Its newest event's epoch, for `/list`'s age column.
    pub last_active: Option<i64>,
}

/// The machine's running sessions.
pub trait World {
    /// Every running session, sampled now.
    fn running(&self) -> Vec<RunningSession>;
}

/// A lookup that must distinguish "no match" from "more than one".
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

/// The sticky `/use` override, as it stands on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sticky {
    /// Nothing selected: plain text is answered, never delivered.
    Unset,
    /// An override the operator set.
    Set {
        /// The session reference it names — REVALIDATED on every use, never
        /// trusted because it was valid when it was written.
        session: String,
        /// The agent reference it names.
        agent: String,
    },
    /// The file is there and is not a target.
    Corrupt,
}

/// Read the sticky target from `path`.
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

/// Where a shared-chat ask goes: one running session's chat, as the session
/// was resolved when the message was routed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The session's name.
    pub session: String,
    /// Its state directory.
    pub dir: PathBuf,
    /// The `session_id` it was resolved under; the submit proves it again.
    pub session_id: String,
    /// Its lead pair, main first; the submit proves it again.
    pub pair: Vec<String>,
    /// The seat the human named, or `None` for the chat's own router.
    pub seat: Option<String>,
}

/// How the session was chosen, for the acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chosen {
    /// `@session`, or `/session`.
    Explicit,
    /// The `/use` selection.
    Sticky,
    /// The session the replied-to message came from.
    Reply,
}

/// What one authorized message should cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Ask `target`'s shared chat, through the chat's own submission.
    Chat {
        /// Where the ask goes.
        target: Target,
        /// The message body.
        text: String,
        /// How the session was chosen.
        chosen: Chosen,
    },
    /// Select a session (and optionally a seat) for plain messages; revalidated
    /// on every use.
    Use {
        /// The resolved session's name.
        session: String,
        /// The seat, or [`ANY_SEAT`] for the chat's own router.
        agent: String,
    },
    /// Clear the selection (`/use clear`).
    Unuse,
    /// Read the explicitly bound quota stores; the decision itself does no I/O.
    Quota,
    /// Answer the chat and change nothing else.
    Answer(String),
}

/// The stored seat of a `/use` that named none: the chat's own router picks.
pub const ANY_SEAT: &str = "*";

/// Decide where a message goes. Nothing is guessed: plain text with no
/// selection, no reply and no named session is answered, never delivered.
#[must_use]
pub fn decide(
    message: Inbound<'_>,
    sessions: &[RunningSession],
    sticky: &Sticky,
    now: i64,
) -> Route {
    // Trim FIRST, and trim both ends.
    let text = message.text.trim();

    // Precedence, and the order is the rule: a slash command is a
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
        "/quota" => Route::Quota,
        "/use" => use_route(rest, sessions, sticky),
        "/session" => session_route(rest, sessions),
        other if other.starts_with('/') => {
            Route::Answer(format!("Unknown command: {other} — try /help"))
        }
        // Plain text: the selected session, never a guess.
        _ => sticky_route(text, sessions, sticky),
    }
}

/// The `/help` body.
const HELP: &str = "ae telegram — each message is asked in a session's shared chat:\n\
     • /use <session> → select a session; plain messages go to its chat. /use clear unselects.\n\
     • @session <msg> (or @session:seat <msg>) → ask that session's chat once.\n\
     • Reply to any forwarded message → asks the chat it came from.\n\
     /list\n\
     /quota — cached quota by account\n\
     /session <name|id-prefix> ask <seat> <msg>";

/// What a plain message with nowhere to go is answered.
const CHOOSE: &str = "No session selected — nothing was sent. /use <session> selects one, \
     @session <msg> asks one once, or reply to a forwarded message. (/list)";

/// A plain message sent as a REPLY to a forwarded row: ask the chat that row
/// came from, and the seat that spoke it when it is a lead-pair seat.
fn reply_route(quoted: &str, text: &str, sessions: &[RunningSession]) -> Route {
    let Some((session, actor)) = parse_reply_target(quoted) else {
        return Route::Answer(
            "Couldn't tell which session that was a reply to — use @session <msg>".to_owned(),
        );
    };
    let seat = match resolve_session(sessions, &session) {
        Resolved::One(found) => seat_named(found, &actor).ok().flatten(),
        _ => None,
    };
    chat_route(&session, seat.as_deref(), text, sessions, Chosen::Reply)
}

/// `@session <msg>` or `@session:seat <msg>` — the compact prefix.
fn at_route(rest: &str, sessions: &[RunningSession]) -> Route {
    let (token, message) = split_word(rest);
    let (session, seat) = match token.split_once(':') {
        Some((session, seat)) => (session, Some(seat)),
        None => (token, None),
    };
    if session.is_empty() {
        return Route::Answer("Usage: @session <msg>  (or @session:seat <msg>)".to_owned());
    }
    if message.is_empty() {
        return Route::Answer("Nothing to send — @session <msg>".to_owned());
    }
    chat_route(session, seat, message, sessions, Chosen::Explicit)
}

/// `/session <ref> <send|ask> <seat> <msg…>`: both verbs ask the chat.
fn session_route(args: &str, sessions: &[RunningSession]) -> Route {
    let (reference, rest) = split_word(args);
    let (verb, rest) = split_word(rest);
    let (seat, message) = split_word(rest);
    // THE MESSAGE IS THE ONLY PART THAT CAN DECIDE, and testing the other three
    // would be testing nothing.
    if message.is_empty() {
        return Route::Answer("Usage: /session <name|id-prefix> ask <seat> <msg>".to_owned());
    }
    if !matches!(verb, "send" | "ask") {
        return Route::Answer(format!("Unknown verb '{verb}' (use ask)"));
    }
    chat_route(reference, Some(seat), message, sessions, Chosen::Explicit)
}

/// `/use`, `/use clear`, `/use <session> [seat]`.
fn use_route(args: &str, sessions: &[RunningSession], sticky: &Sticky) -> Route {
    let (reference, rest) = split_word(args);
    if reference.is_empty() {
        return Route::Answer(describe_sticky(sticky));
    }
    if matches!(reference, "clear" | "off" | "none") {
        return Route::Unuse;
    }
    let (seat, _) = split_word(rest);
    let seat = Some(seat).filter(|seat| !seat.is_empty());
    match revalidate(reference, seat, sessions) {
        Ok(target) => Route::Use {
            session: target.session,
            agent: target.seat.unwrap_or_else(|| ANY_SEAT.to_owned()),
        },
        Err(refusal) => Route::Answer(refusal),
    }
}

/// What `/use` with no arguments reports.
fn describe_sticky(sticky: &Sticky) -> String {
    match sticky {
        Sticky::Set { session, agent } if agent == ANY_SEAT => {
            format!("Selected: {session} (plain messages ask its chat; /use clear unselects)")
        }
        Sticky::Set { session, agent } => format!(
            "Selected: {session}, seat {agent} (plain messages ask its chat; /use clear \
             unselects)"
        ),
        Sticky::Corrupt => "The selection is unreadable — /use <session>".to_owned(),
        Sticky::Unset => "No session selected — /use <session>. (/list)".to_owned(),
    }
}

/// Plain text with no reply: the selected session, REVALIDATED every time.
fn sticky_route(text: &str, sessions: &[RunningSession], sticky: &Sticky) -> Route {
    match sticky {
        Sticky::Set { session, agent } => {
            let seat = Some(agent.as_str()).filter(|agent| *agent != ANY_SEAT);
            chat_route(session, seat, text, sessions, Chosen::Sticky)
        }
        Sticky::Corrupt => Route::Answer(
            "The selection is unreadable — nothing was sent. /use <session>".to_owned(),
        ),
        Sticky::Unset => Route::Answer(CHOOSE.to_owned()),
    }
}

/// Resolve a session (and a seat, when one is named) and build the ask, or
/// the refusal that explains which half did not resolve.
fn chat_route(
    reference: &str,
    seat: Option<&str>,
    text: &str,
    sessions: &[RunningSession],
    chosen: Chosen,
) -> Route {
    match revalidate(reference, seat, sessions) {
        Ok(target) => Route::Chat {
            target,
            text: text.to_owned(),
            chosen,
        },
        Err(refusal) => Route::Answer(refusal),
    }
}

/// **THE ONE REVALIDATION** every route passes through: one running session
/// with a lead pair, and the named seat one of that pair.
fn revalidate(
    reference: &str,
    seat: Option<&str>,
    sessions: &[RunningSession],
) -> Result<Target, String> {
    let session = match resolve_session(sessions, reference) {
        Resolved::One(session) => session,
        Resolved::Ambiguous => {
            return Err(format!(
                "Ambiguous session '{reference}' — be more specific; nothing was sent"
            ));
        }
        Resolved::Missing => {
            return Err(format!(
                "No running session matching '{reference}' — nothing was sent"
            ));
        }
    };
    if session.pair.is_empty() {
        return Err(format!("{} has no lead pair to ask", session.name));
    }
    let seat = match seat {
        Some(want) => seat_named(session, want)?,
        None => None,
    };
    Ok(Target {
        session: session.name.clone(),
        dir: session.dir.clone(),
        session_id: session.session_id.clone(),
        pair: session.pair.clone(),
        seat,
    })
}

/// The lead-pair seat `want` names in `session` — by its name, or as the
/// roster's `alias:name` — or `None` for [`ANY_SEAT`].
fn seat_named(session: &RunningSession, want: &str) -> Result<Option<String>, String> {
    if want.is_empty() || want == ANY_SEAT {
        return Ok(None);
    }
    let name = want.rsplit_once(':').map_or(want, |(_, name)| name);
    if session.pair.iter().any(|seat| seat == name) {
        return Ok(Some(name.to_owned()));
    }
    Err(format!(
        "'{want}' is not a lead-pair seat of {} ({}) — nothing was sent",
        session.name,
        session.pair.join(", ")
    ))
}

/// Read the header line of a message THIS bridge forwarded, and recover the
/// session and agent it came from.
#[must_use]
pub fn parse_reply_target(quoted: &str) -> Option<(String, String)> {
    let first = quoted.lines().next()?;
    let inner = first.strip_prefix('[')?;
    let (session, rest) = inner.split_once("] ")?;
    if session.is_empty() {
        return None;
    }
    let actor = match rest.split_once("  ") {
        // The long shape: skip the action, take the actor.
        Some((_, after)) => split_word(after).0,
        // The short shape: the actor is all there is.
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

/// The bridge's age wording, which is NOT the listing's: this surface calls
/// anything under ninety seconds "just now".
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

/// Plain phone-sized quota blocks. Every field passes through the terminal text
/// owner, and the reply reserves space for a counted omission before each block.
#[must_use]
pub fn render_quota(scopes: &[crate::quota::view::Scope]) -> String {
    let mut text = "Quota · cached only".to_owned();
    for (index, scope) in scopes.iter().enumerate() {
        let mut block = QuotaBlock::default();
        block.push("\n\n");
        block.push(&scope.title);
        for fact in [&scope.account, &scope.only].into_iter().flatten() {
            block.push("\n");
            block.push(fact);
        }
        for window in &scope.windows {
            block.push("\n");
            block.push(&window.name);
            block.push(": ");
            block.push(
                window
                    .counts
                    .as_deref()
                    .or(window.raw.as_deref())
                    .unwrap_or("?"),
            );
            if window.counts.is_some() || window.raw.is_some() {
                block.push(" effective used");
            }
            let note = crate::quota::view::note(window, true);
            if !note.is_empty() {
                block.push(" · ");
                block.push(&note);
            }
            block.push("\nreset in ");
            block.push(window.resets.as_deref().unwrap_or("?"));
            block.push(" · seen ");
            block.push(window.seen.as_deref().unwrap_or("?"));
            if window.seen.is_some() && !window.stale {
                block.push(" ago · fresh");
            }
            if block.cut {
                break;
            }
        }
        let remaining = scopes.len() - index;
        let reserve = if remaining > 1 {
            1 + quota_marker(remaining - 1).len()
        } else {
            0
        };
        if !block.cut && text.len() + block.text.len() + reserve <= QUOTA_BYTES {
            text.push_str(&block.text);
        } else {
            let marker = quota_marker(remaining);
            let room = QUOTA_BYTES.saturating_sub(text.len() + marker.len() + 1);
            let end = block.text.floor_char_boundary(room);
            text.push_str(&block.text[..end]);
            text.push('\n');
            text.push_str(&marker);
            break;
        }
    }
    text
}

const QUOTA_BYTES: usize = 3900;

fn quota_marker(count: usize) -> String {
    let noun = if count == 1 { "scope" } else { "scopes" };
    format!("…({count} {noun} not shown in full; run ae quota)")
}

/// Retain only enough of one block to decide whether it fits. Neutralisation can
/// expand a control scalar; cap after it as well, always at scalar boundaries.
#[derive(Default)]
struct QuotaBlock {
    text: String,
    cut: bool,
}

impl QuotaBlock {
    fn push(&mut self, value: &str) {
        if self.cut {
            return;
        }
        let room = (QUOTA_BYTES + 1).saturating_sub(self.text.len());
        let prefix = value.floor_char_boundary(room);
        let clean = crate::board::terminal_text(&value[..prefix]).replace('\t', " ");
        let end = clean.floor_char_boundary(room);
        self.text.push_str(&clean[..end]);
        self.cut = prefix < value.len() || end < clean.len();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ANY_SEAT, Chosen, Inbound, Resolved, Route, RunningSession, Sticky, Target, decide,
        parse_reply_target, parse_sticky, render_sticky, resolve_session, split_word,
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
            pair: agents
                .iter()
                .take(2)
                .map(|agent| {
                    agent
                        .rsplit_once(':')
                        .map_or(*agent, |(_, name)| name)
                        .to_owned()
                })
                .collect(),
            last_active: Some(NOW),
        }
    }

    fn plain(text: &str) -> Inbound<'_> {
        Inbound {
            text,
            reply_to: None,
        }
    }

    fn target(name: &str, pair: &[&str], seat: Option<&str>) -> Target {
        Target {
            session: name.to_owned(),
            dir: PathBuf::from("/sessions").join(name),
            session_id: format!("{name}-0123456789abcdef"),
            pair: pair.iter().map(|seat| (*seat).to_owned()).collect(),
            seat: seat.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn plain_text_with_nothing_selected_is_answered_and_never_delivered() {
        let world = vec![session("work", &["claude:lead", "codex:colead"])];
        let Route::Answer(text) = decide(plain("status please"), &world, &Sticky::Unset, NOW)
        else {
            panic!("plain text with no selection must not reach a chat");
        };
        assert!(text.contains("/use <session>"), "{text}");
        let Route::Answer(text) = decide(plain("status please"), &world, &Sticky::Corrupt, NOW)
        else {
            panic!("a corrupt selection must not reach a chat");
        };
        assert!(text.contains("nothing was sent"), "{text}");
    }

    #[test]
    fn use_selects_a_session_for_the_router_or_a_named_pair_seat_and_clear_unselects() {
        let world = vec![session(
            "work",
            &["claude:lead", "codex:colead", "codex:dev"],
        )];
        assert_eq!(
            decide(plain("/use work"), &world, &Sticky::Unset, NOW),
            Route::Use {
                session: "work".to_owned(),
                agent: ANY_SEAT.to_owned(),
            }
        );
        assert_eq!(
            decide(plain("/use work codex:colead"), &world, &Sticky::Unset, NOW),
            Route::Use {
                session: "work".to_owned(),
                agent: "colead".to_owned(),
            }
        );
        let Route::Answer(text) = decide(plain("/use work dev"), &world, &Sticky::Unset, NOW)
        else {
            panic!("a worker is not a chat seat");
        };
        assert!(text.contains("not a lead-pair seat"), "{text}");
        assert_eq!(
            decide(plain("/use clear"), &world, &Sticky::Unset, NOW),
            Route::Unuse
        );
        let any = Sticky::Set {
            session: "work".to_owned(),
            agent: ANY_SEAT.to_owned(),
        };
        assert_eq!(
            decide(plain("hello"), &world, &any, NOW),
            Route::Chat {
                target: target("work", &["lead", "colead"], None),
                text: "hello".to_owned(),
                chosen: Chosen::Sticky,
            }
        );
        let Route::Answer(selected) = decide(plain("/use"), &world, &any, NOW) else {
            panic!("/use answers");
        };
        assert!(selected.contains("Selected: work"), "{selected}");
    }

    #[test]
    fn a_reply_asks_the_chat_it_came_from_and_keeps_a_pair_seat_that_spoke_it() {
        let world = vec![
            session("work", &["claude:lead", "codex:colead"]),
            session("other", &["claude:lead"]),
        ];
        let reply = |quoted| Inbound {
            text: "yes, go ahead",
            reply_to: Some(quoted),
        };
        let other = Sticky::Set {
            session: "other".to_owned(),
            agent: ANY_SEAT.to_owned(),
        };
        assert_eq!(
            decide(reply("[work] colead\nready?"), &world, &other, NOW),
            Route::Chat {
                target: target("work", &["lead", "colead"], Some("colead")),
                text: "yes, go ahead".to_owned(),
                chosen: Chosen::Reply,
            }
        );
        assert_eq!(
            decide(reply("[work] dev\nworker said"), &world, &other, NOW),
            Route::Chat {
                target: target("work", &["lead", "colead"], None),
                text: "yes, go ahead".to_owned(),
                chosen: Chosen::Reply,
            },
            "a row a non-pair actor spoke goes to the chat's own router"
        );
        let Route::Answer(text) = decide(reply("not this bridge's"), &world, &other, NOW) else {
            panic!("an unreadable header must not fall through to a delivery");
        };
        assert!(text.contains("Couldn't tell which session"), "{text}");
    }

    #[test]
    fn the_at_prefix_asks_one_session_once_and_a_named_seat_must_be_of_its_pair() {
        let world = vec![session(
            "work",
            &["claude:lead", "codex:colead", "codex:dev"],
        )];
        assert_eq!(
            decide(plain("@work:claude:lead do X"), &world, &Sticky::Unset, NOW),
            Route::Chat {
                target: target("work", &["lead", "colead"], Some("lead")),
                text: "do X".to_owned(),
                chosen: Chosen::Explicit,
            }
        );
        for refused in ["@work:dev do X", "@work", "@nowhere do X"] {
            assert!(
                matches!(
                    decide(plain(refused), &world, &Sticky::Unset, NOW),
                    Route::Answer(_)
                ),
                "{refused}"
            );
        }
    }

    #[test]
    fn a_session_with_no_lead_pair_takes_no_ask() {
        let mut bare = session("bare", &[]);
        bare.pair.clear();
        let Route::Answer(text) = decide(plain("@bare hi"), &[bare], &Sticky::Unset, NOW) else {
            panic!("no pair, no ask");
        };
        assert!(text.contains("no lead pair"), "{text}");
    }

    #[test]
    fn the_session_command_needs_every_one_of_its_four_parts() {
        // One missing part at a time.
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
    fn a_slash_command_stays_a_command_even_as_a_reply() {
        // Precedence.
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
    fn the_reply_header_is_read_in_both_the_rust_and_the_frozen_bash_shapes() {
        // The renderer emits `[session] actor`; a header shaped
        // `[session] action  actor → target` is still read, so older messages
        // in the operator's chat stay replyable.
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
    fn a_session_resolves_by_exact_name_first_and_then_by_unique_id_prefix() {
        // The precedence: an exact NAME is what the
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
    fn an_unknown_slash_command_is_named_back_rather_than_routed_as_text() {
        let world = vec![session("work", &["claude:lead"])];
        let Route::Answer(text) = decide(plain("/frobnicate"), &world, &Sticky::Unset, NOW) else {
            panic!("an unknown command must not become a message to a chat");
        };
        assert!(text.contains("Unknown command: /frobnicate"), "{text}");
    }

    #[test]
    fn a_command_addressed_to_the_bot_by_name_is_still_that_command() {
        let world = vec![session("work", &["claude:lead"])];
        let Route::Answer(text) = decide(plain("/list@ae_bot"), &world, &Sticky::Unset, NOW) else {
            panic!("/list@bot must be /list");
        };
        assert!(text.starts_with("Running sessions:"), "{text}");
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

    // Frozen independent acceptance: tgchat-nav, OpenAI GPT-6.1-Sol.
    // Install byte-identically inside src/telegram/routing.rs::tests.
    fn nav_session(name: &str, id: &str) -> super::RunningSession {
        super::RunningSession {
            name: name.to_owned(),
            dir: std::path::PathBuf::from("/sessions").join(name),
            session_id: id.to_owned(),
            meta_agent: false,
            main: Some("claude:lead".to_owned()),
            agents: vec![
                "claude:lead".to_owned(),
                "codex:colead".to_owned(),
                "codex:worker".to_owned(),
            ],
            pair: vec!["lead".to_owned(), "colead".to_owned()],
            last_active: Some(1_792_141_200),
        }
    }

    #[test]
    fn nav_plain_text_never_guesses_even_a_single_orchestrator_session() {
        let mut only = nav_session("aedev", "abc-first");
        only.meta_agent = true;
        let route = super::decide(
            super::Inbound {
                text: "phone question",
                reply_to: None,
            },
            &[only],
            &super::Sticky::Unset,
            1_792_141_200,
        );
        assert!(
            matches!(route, super::Route::Answer(ref body) if body.contains("/use")),
            "plain text guessed a session: {route:?}"
        );
    }

    #[test]
    fn nav_explicit_session_wins_and_captures_shared_chat_identity() {
        let route = super::decide(
            super::Inbound {
                text: "@aedev phone question",
                reply_to: None,
            },
            &[
                nav_session("aedev", "abc-first"),
                nav_session("assistant", "xyz-second"),
            ],
            &super::Sticky::Set {
                session: "assistant".to_owned(),
                agent: "colead".to_owned(),
            },
            1_792_141_200,
        );
        match route {
            super::Route::Chat { target, text, .. } => {
                assert_eq!(target.session, "aedev");
                assert_eq!(target.session_id, "abc-first");
                assert_eq!(target.pair, vec!["lead", "colead"]);
                assert_eq!(
                    target.seat, None,
                    "unspecified seat must reach normal pair chooser"
                );
                assert_eq!(text, "phone question");
            }
            other => panic!("explicit session did not enter shared chat: {other:?}"),
        }
    }

    #[test]
    fn nav_reply_routes_original_session_over_current_sticky_selection() {
        let route = super::decide(
            super::Inbound {
                text: "follow up",
                reply_to: Some("[aedev] human:chat\noriginal question"),
            },
            &[
                nav_session("aedev", "abc-first"),
                nav_session("assistant", "xyz-second"),
            ],
            &super::Sticky::Set {
                session: "assistant".to_owned(),
                agent: "colead".to_owned(),
            },
            1_792_141_200,
        );
        match route {
            super::Route::Chat { target, text, .. } => {
                assert_eq!(target.session, "aedev");
                assert_eq!(
                    target.seat, None,
                    "human header must use normal pair routing"
                );
                assert_eq!(text, "follow up");
            }
            other => panic!("reply lost original session: {other:?}"),
        }
    }

    #[test]
    fn nav_sticky_routes_selected_session_and_stale_sticky_refuses() {
        let sticky = super::Sticky::Set {
            session: "assistant".to_owned(),
            agent: "colead".to_owned(),
        };
        let sessions = vec![
            nav_session("aedev", "abc-first"),
            nav_session("assistant", "xyz-second"),
        ];
        let selected = super::decide(
            super::Inbound {
                text: "selected question",
                reply_to: None,
            },
            &sessions,
            &sticky,
            1_792_141_200,
        );
        assert!(
            matches!(selected, super::Route::Chat { ref target, .. } if target.session == "assistant"),
            "sticky session lost: {selected:?}"
        );
        let stale = super::decide(
            super::Inbound {
                text: "selected question",
                reply_to: None,
            },
            &sessions[..1],
            &sticky,
            1_792_141_200,
        );
        assert!(
            matches!(stale, super::Route::Answer(_)),
            "stale sticky fell back to another session: {stale:?}"
        );
    }

    #[test]
    fn nav_ambiguous_session_corrupt_sticky_and_worker_dm_are_refused() {
        let sessions = vec![
            nav_session("aedev", "abc-first"),
            nav_session("assistant", "abc-second"),
        ];
        for (text, sticky) in [
            ("@abc phone question", super::Sticky::Unset),
            ("phone question", super::Sticky::Corrupt),
            (
                "/session aedev ask worker phone question",
                super::Sticky::Unset,
            ),
        ] {
            let route = super::decide(
                super::Inbound {
                    text,
                    reply_to: None,
                },
                &sessions,
                &sticky,
                1_792_141_200,
            );
            assert!(
                matches!(route, super::Route::Answer(_)),
                "unsafe route admitted: {text}: {route:?}"
            );
        }
    }

    #[test]
    fn nav_existing_session_command_still_routes_shared_chat_and_beats_reply() {
        let route = super::decide(
            super::Inbound {
                text: "/session assistant ask colead command question",
                reply_to: Some("[aedev] lead\nold answer"),
            },
            &[
                nav_session("aedev", "abc-first"),
                nav_session("assistant", "xyz-second"),
            ],
            &super::Sticky::Unset,
            1_792_141_200,
        );
        match route {
            super::Route::Chat { target, text, .. } => {
                assert_eq!(target.session, "assistant");
                assert_eq!(target.seat.as_deref(), Some("colead"));
                assert_eq!(text, "command question");
            }
            other => panic!("existing command lost or reply stole it: {other:?}"),
        }
    }

    #[test]
    fn bang_telegram_message_starting_with_exclamation_stays_plain_chat_text() {
        let sessions = vec![nav_session("aedev", "abc-first")];
        let sticky = super::Sticky::Set {
            session: "aedev".to_owned(),
            agent: super::ANY_SEAT.to_owned(),
        };
        for raw in ["!git status", "@aedev !git status"] {
            let route = super::decide(
                super::Inbound {
                    text: raw,
                    reply_to: None,
                },
                &sessions,
                &sticky,
                1_792_141_200,
            );
            match route {
                super::Route::Chat { target, text, .. } => {
                    assert_eq!(target.session, "aedev");
                    assert_eq!(text, "!git status");
                }
                other => panic!("Telegram ! message did not route as plain chat text: {other:?}"),
            }
        }
    }
}
