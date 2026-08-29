//! Tracked requests — the `ask` and `review` helpers (P2.5a, the request-write
//! tracer).
//!
//! What the frozen `ae_tracked_send` does, kept exactly, up to the paste:
//! the body is refused when blank; the caller is `AE_SENDER_OVERRIDE` or the
//! pane's own stamp (no identity at all falls back to a plain `send`, with the
//! frozen warning); an external sink (`telegram:*`, `discord:*`,
//! `ae:compact:*`) is event-only; any other target is resolved the way
//! `ae_resolve` resolves it — `%pane` passthrough, `@session:agent` across
//! sessions, exact `alias:name`, else a unique alias, else a unique bare name;
//! a request id is minted (`<prefix>-<YYYYMMDDTHHMMSSZ>-<8 hex>`); the message
//! is composed with the header, the optional review instructions and the
//! REQUIRED reply footer whose command names the resolved target, the id and
//! the reply label.
//!
//! # What is the core's, and what stays frozen
//!
//! The PASTE is the frozen `send` body, run by [`crate::transport::deliver`]
//! through the session's INTERNAL `_send-deliver` helper — the same body
//! behind a delivery-only entry point: the dead-pane guard, the provenance
//! envelope, the per-target lock, the busy deferral and the submit
//! verification are measured TUI behaviour this crate does not re-implement,
//! and every loud line they print reaches the caller verbatim because the
//! helper's stderr is inherited. That entry still STORES the delivered text
//! beside the session (the recovery record the event points at) and prints
//! its path instead of emitting the event. The public `send` is untouched by
//! this: its event is pinned by its own entry point, not by anything in the
//! environment — the P2.5a review's ruling — so the no-identity fallback
//! below, which IS a plain `send`, records itself as it always did.
//!
//! The EVENT is the core's: the `ask`/`review` line — actor, target, ref, the
//! four routing members (`actor_slot`, `actor_session`, `target_slot`,
//! `target_session`), the capped summary and `body_file` — is written under
//! [`crate::state::emit`]'s locked, synced transaction, AFTER the paste and
//! the body store, in the frozen order. Bash `reply` then finds the request by
//! `ref` and routes by the stored slots — which is the proof this tracer
//! exists to give.
//!
//! # Loud failure, in the frozen shape
//!
//! A refused body, a target that does not resolve, a paste the helper refused
//! or could not confirm: stderr, non-zero, and no event — the frozen helper
//! wrote nothing in those cases either. The one new sentence is the gap
//! between the two steps: a request that was delivered but whose event could
//! not be written is reported as exactly that, never as "nothing was sent".
use std::io::{self, Write};
use std::path::Path;

use crate::inventory::ServerId;
use crate::json::Value;
use crate::meta;
use crate::requests::is_slot;
use crate::state::{self, EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;
use crate::tmux::ObservedAgent;
use crate::transport;

/// The two tracked-request helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `ask`.
    Ask,
    /// `review`.
    Review,
}

/// The frozen `ask` usage text.
pub const ASK_USAGE: &str = "Usage: ask <agent-name|pane-id|@session:agent> <question>\n  Like send, but embeds your identity and reply command in the message.\n";

/// The frozen `review` usage text.
pub const REVIEW_USAGE: &str = "Usage: review <agent-name|pane-id|@session:agent> <request>\n  Ask another agent for a critical review and require a reply via send.\n";

/// The frozen `REVIEW_INSTRUCTIONS` literal — its continuation lines carry
/// the four spaces of source indentation the bash literal carries.
pub const REVIEW_INSTRUCTIONS: &str = "Review instructions:\n    - Focus on correctness, regressions, edge cases, missing tests, and callers/docs needing updates.\n    - Findings first. Keep summaries brief.\n    - Use severity labels: BLOCKER, IMPORTANT, NIT.\n    - If no issues are found, say \"No findings\" explicitly.";

impl Kind {
    /// The event action.
    #[must_use]
    pub const fn action(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Review => "review",
        }
    }

    /// The request id prefix — `ae` for a question, `review` for a review.
    #[must_use]
    pub const fn id_prefix(self) -> &'static str {
        match self {
            Self::Ask => "ae",
            Self::Review => "review",
        }
    }

    /// The message header.
    #[must_use]
    pub const fn header(self) -> &'static str {
        match self {
            Self::Ask => "REQUEST",
            Self::Review => "REVIEW REQUEST",
        }
    }

    /// The instructions block a review carries and a question does not.
    #[must_use]
    pub const fn instructions(self) -> Option<&'static str> {
        match self {
            Self::Ask => None,
            Self::Review => Some(REVIEW_INSTRUCTIONS),
        }
    }

    /// The placeholder in the reply command.
    #[must_use]
    pub const fn reply_label(self) -> &'static str {
        match self {
            Self::Ask => "<your reply>",
            Self::Review => "<your review>",
        }
    }

    /// The usage text.
    #[must_use]
    pub const fn usage(self) -> &'static str {
        match self {
            Self::Ask => ASK_USAGE,
            Self::Review => REVIEW_USAGE,
        }
    }
}

/// A parsed argv: the target as typed, the body as `"$*"` joins it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// The target name, pane id or `@session:agent`.
    pub target: String,
    /// The remaining words joined by one space.
    pub body: String,
}

/// Fewer than two words after the meta directory: the usage text, exit
/// [`EXIT_USAGE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage;

/// Parse the argv after the meta directory: `<target> <word…>`.
///
/// # Errors
///
/// [`Usage`] for fewer than two words.
pub fn parse(tail: &[String]) -> Result<Parsed, Usage> {
    match tail {
        [target, words @ ..] if !words.is_empty() => Ok(Parsed {
            target: target.clone(),
            body: words.join(" "),
        }),
        _ => Err(Usage),
    }
}

/// `ae_require_body`'s test: nothing but `[[:space:]]` — space, tab, newline,
/// vertical tab, form feed, carriage return.
#[must_use]
pub fn is_blank(body: &str) -> bool {
    body.chars()
        .all(|c| c.is_ascii_whitespace() || c == '\u{b}')
}

/// The frozen refusal for a blank body — two lines, stderr, exit
/// [`EXIT_FAILED`], nothing sent.
#[must_use]
pub fn refusal(action: &str) -> String {
    format!(
        "ae: {action} REFUSED — the message body is empty (or only whitespace). Nothing was sent.\nae: a delivered header with no body reads as a message that was received and said nothing.\n"
    )
}

/// The frozen warning when the caller has no identity, before the fallback
/// to a plain `send`.
pub const NO_IDENTITY_WARNING: &str = "Warning: could not detect caller identity (no @ae_agent on this pane). Using 'send' instead.\n";

/// Whether `target` is an event-only sink the frozen helper never resolves:
/// `telegram:*`, `discord:*` or exactly the `ae:compact:` prefix — a
/// whitelist, because the failure this family can produce is a silent no-op
/// delivery, so an `ae:`-shaped typo must still fail loudly.
#[must_use]
pub fn is_external(target: &str) -> bool {
    target.starts_with("telegram:")
        || target.starts_with("discord:")
        || target.starts_with("ae:compact:")
}

/// `ae_make_req_id`: `<prefix>-<YYYYMMDDTHHMMSSZ>-<8 lowercase hex>`. The
/// suffix is the low 32 bits of `entropy`; dash-free by construction, because
/// the id is parsed on `-`.
///
/// ```
/// use ae::time::Timestamp;
/// use ae::tracked::request_id;
///
/// let now = Timestamp::parse("2026-08-27T07:11:12Z").unwrap();
/// assert_eq!(request_id("ae", now, 0x1234_5678_9abc_def0), "ae-20260827T071112Z-9abcdef0");
/// assert_eq!(request_id("review", now, 7), "review-20260827T071112Z-00000007");
/// ```
#[must_use]
pub fn request_id(prefix: &str, now: Timestamp, entropy: u64) -> String {
    let compact: String = now
        .to_string()
        .chars()
        .filter(|c| *c != '-' && *c != ':')
        .collect();
    format!("{prefix}-{compact}-{:08x}", entropy & 0xffff_ffff)
}

/// The exact reply command the footer carries:
/// `<dir>/reply --as "<target>" "<id>" "<label>"`.
#[must_use]
pub fn reply_command(dir: &Path, target_name: &str, req_id: &str, label: &str) -> String {
    format!(
        "{}/reply --as \"{target_name}\" \"{req_id}\" \"{label}\"",
        dir.display()
    )
}

/// The delivered text, before the provenance envelope the frozen `send` body
/// prepends: `<header> <id> from <sender>: <body>`, the instructions block
/// for a review, and the REQUIRED footer.
///
/// ```
/// use ae::tracked::{Kind, compose};
///
/// let text = compose(Kind::Ask, "ae-1", "cl:lead", "why?", "/s/reply --as \"cl:w\" \"ae-1\" \"<your reply>\"");
/// assert_eq!(
///     text,
///     "REQUEST ae-1 from cl:lead: why?\n\nREQUIRED: When you have finished, you MUST run this exact command to reply:\n/s/reply --as \"cl:w\" \"ae-1\" \"<your reply>\"\nDo not reply any other way. Do NOT use peek/peak as a reply mechanism."
/// );
/// assert!(compose(Kind::Review, "review-1", "a", "b", "c").contains("\n\nReview instructions:\n    - Focus on"));
/// ```
#[must_use]
pub fn compose(kind: Kind, req_id: &str, sender: &str, body: &str, reply_cmd: &str) -> String {
    let mut text = format!("{} {req_id} from {sender}: {body}", kind.header());
    if let Some(instructions) = kind.instructions() {
        text.push_str("\n\n");
        text.push_str(instructions);
    }
    text.push_str(
        "\n\nREQUIRED: When you have finished, you MUST run this exact command to reply:\n",
    );
    text.push_str(reply_cmd);
    text.push_str("\nDo not reply any other way. Do NOT use peek/peak as a reply mechanism.");
    text
}

// ---- resolution: ae_resolve, ported --------------------------------------

/// What the frozen resolver leaves in `AE_RESOLVED_*`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    /// The pane id.
    pub pane: String,
    /// The display ref (`alias:name`, `@session:` prefixed across sessions),
    /// or empty for an unstamped pane named by id.
    pub agent: String,
    /// The pane's `@ae_slot` when it is one of the closed grammar, else empty.
    pub slot: String,
    /// The pane's session, or empty when it could not be read.
    pub session: String,
}

/// Why a target did not resolve — each with the frozen error line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// `@something` without a `:`.
    CrossSessionShape(String),
    /// `@:agent` or `@session:`.
    CrossSessionEmpty,
    /// The named session is not on the server.
    SessionNotFound(String),
    /// More than one alias or bare-name match.
    Ambiguous {
        /// The name as typed.
        target: String,
        /// The session searched.
        session: String,
    },
    /// No match at all.
    NotFound {
        /// The name as typed.
        target: String,
        /// The session searched.
        session: String,
    },
    /// The target session records no usable tmux server (its selector is
    /// Missing/Ambiguous, or its meta is unreadable). Resolution FAILS CLOSED
    /// rather than falling back to the ambient server: a pane-less caller's
    /// ambient is not the recorded server, so a fallback would enumerate the
    /// wrong server and mis-route silently — the clean cut refuses instead and
    /// says how to repair it.
    UnresolvableServer {
        /// The session whose server pointer could not be trusted.
        session: String,
    },
}

impl ResolveError {
    /// The stderr line, exactly as `ae_resolve` prints it.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::CrossSessionShape(target) => {
                format!("Error: cross-session target must be @session:agent, got '{target}'")
            }
            Self::CrossSessionEmpty => {
                "Error: cross-session target must be @session:agent".to_owned()
            }
            Self::SessionNotFound(session) => format!("Error: session '{session}' not found"),
            Self::Ambiguous { target, session } => format!(
                "Error: ambiguous name '{target}' in session '{session}' — use alias:name format"
            ),
            Self::NotFound { target, session } => {
                format!("Error: agent '{target}' not found in session '{session}'")
            }
            Self::UnresolvableServer { session } => format!(
                "Error: session '{session}' records no usable tmux server — refresh or migrate the session, then retry"
            ),
        }
    }
}

/// Where a target is looked up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// `%<id>` — passed through, its stamps read off the pane.
    Pane(String),
    /// A name, searched in one session's roster.
    Named {
        /// The session to search — the caller's own, or the `@session` named.
        session: String,
        /// The name to match.
        target: String,
        /// Whether the session was NAMED (`@session:agent`). The frozen
        /// resolver runs `has-session` for every `@` form, the caller's own
        /// session included, and only for those.
        explicit: bool,
    },
}

/// Classify a target the way `ae_resolve` does before it reads anything.
///
/// # Errors
///
/// The two cross-session shape errors.
pub fn lookup(target: &str, own_session: &str) -> Result<Lookup, ResolveError> {
    if target.starts_with('%') {
        return Ok(Lookup::Pane(target.to_owned()));
    }
    if let Some(rest) = target.strip_prefix('@') {
        let Some((session, name)) = rest.split_once(':') else {
            return Err(ResolveError::CrossSessionShape(target.to_owned()));
        };
        if session.is_empty() || name.is_empty() {
            return Err(ResolveError::CrossSessionEmpty);
        }
        return Ok(Lookup::Named {
            session: session.to_owned(),
            target: name.to_owned(),
            explicit: true,
        });
    }
    Ok(Lookup::Named {
        session: own_session.to_owned(),
        target: target.to_owned(),
        explicit: false,
    })
}

/// The frozen loop's pick over a roster: the first exact `alias:name` match;
/// else the one alias-only match; else the one bare-name match. Two or more
/// of either is ambiguous; none is not found. The display ref is `@session:`
/// prefixed when `session` is not `own_session`.
///
/// # Errors
///
/// [`ResolveError::Ambiguous`] or [`ResolveError::NotFound`].
pub fn pick<'a>(
    roster: &'a [ObservedAgent],
    target: &str,
    session: &str,
    own_session: &str,
) -> Result<(&'a str, String), ResolveError> {
    let display = |agent: &str| {
        if session == own_session {
            agent.to_owned()
        } else {
            format!("@{session}:{agent}")
        }
    };
    let mut alias_matches: Vec<&ObservedAgent> = Vec::new();
    let mut bare_matches: Vec<&ObservedAgent> = Vec::new();
    for row in roster {
        if row.agent == target {
            return Ok((row.pane.as_str(), display(&row.agent)));
        }
        let alias = row
            .agent
            .split_once(':')
            .map_or(row.agent.as_str(), |(alias, _)| alias);
        let name = row
            .agent
            .split_once(':')
            .map_or(row.agent.as_str(), |(_, name)| name);
        if alias == target {
            alias_matches.push(row);
        }
        if name == target {
            bare_matches.push(row);
        }
    }
    if let [row] = alias_matches.as_slice() {
        return Ok((row.pane.as_str(), display(&row.agent)));
    }
    if let [row] = bare_matches.as_slice() {
        return Ok((row.pane.as_str(), display(&row.agent)));
    }
    let target = target.to_owned();
    let session = session.to_owned();
    if alias_matches.len() > 1 || bare_matches.len() > 1 {
        Err(ResolveError::Ambiguous { target, session })
    } else {
        Err(ResolveError::NotFound { target, session })
    }
}

/// `ae_resolve`, against the ambient server.
///
/// # Errors
///
/// [`ResolveError`] — see its variants. A pane named by id always resolves
/// (the frozen helper returns 0 for one, and `send` fails later if it is not
/// there); its stamps are simply empty when they cannot be read.
pub fn resolve(target: &str, own_session: &str, dir: &Path) -> Result<Resolved, ResolveError> {
    let (server, pane, agent) = match lookup(target, own_session)? {
        Lookup::Pane(pane) => {
            // A raw pane id is an unambiguous address on its own server, so there
            // is nothing to enumerate and no name to collide: the recorded server
            // only lets its stamps be read, and an unusable one leaves them empty
            // (the frozen "stamps are simply empty when they cannot be read")
            // rather than refusing. No mis-route is possible, so this never fails.
            let server = pane_server(dir);
            let observed = transport::observe_viewer(&server, &pane).unwrap_or_default();
            let agent = match (observed.agent, observed.session) {
                (Some(agent), Some(session)) if session != own_session => {
                    format!("@{session}:{agent}")
                }
                (Some(agent), _) => agent,
                (None, _) => String::new(),
            };
            (server, pane, agent)
        }
        Lookup::Named {
            session,
            target,
            explicit,
        } => {
            // Enumerate on the TARGET session's own recorded server, not the
            // caller's: `@session:agent` may name a session on a different tmux
            // server, and a same-session target resolves to the same server anyway.
            // FAILS CLOSED — enumerating a colliding name on the wrong (ambient)
            // server would mis-route silently, so a session with no usable server
            // pointer refuses here rather than falling back.
            let server = named_server(dir, &session, own_session)?;
            if explicit && !transport::session_exists(&server, &session) {
                return Err(ResolveError::SessionNotFound(session));
            }
            let roster = transport::observe_agents(&server, &session).unwrap_or_default();
            let (pane, agent) = pick(&roster, &target, &session, own_session)?;
            (server, pane.to_owned(), agent)
        }
    };
    let observed = transport::observe_viewer(&server, &pane).unwrap_or_default();
    Ok(Resolved {
        pane,
        agent,
        slot: observed
            .slot
            .filter(|slot| is_slot(slot))
            .unwrap_or_default(),
        session: observed.session.unwrap_or_default(),
    })
}

/// The server for reading a RAW PANE target's stamps: the caller session's
/// recorded one when it is usable, else the ambient server. Never fails — a pane
/// id addresses one pane unambiguously, so an unusable server only means its
/// stamps read empty, never a mis-route (contrast [`named_server`], where a
/// wrong server enumerates a wrong roster).
fn pane_server(dir: &Path) -> ServerId {
    // Through `meta.rs`'s inventoried `read_bytes` door (the same one compact
    // reads meta through), not a raw fs call here — a new world-reading site is
    // a line in a review, not a diff nobody read.
    let selector =
        meta::read_bytes(dir).map(|bytes| meta::Meta::parse(&String::from_utf8_lossy(&bytes)));
    match selector.map(|parsed| parsed.server_selector()) {
        Ok(meta::ServerSelector::Positive(selector)) => ServerId::Selected(selector),
        _ => ServerId::Ambient,
    }
}

/// The tmux server a NAMED target must be enumerated on — the TARGET session's
/// own recorded server, from its meta (the caller's own directory for an
/// unqualified name, a sibling directory under the same sessions root for
/// `@session:agent`).
///
/// FAILS CLOSED, two ways: a session whose meta cannot be read is
/// [`ResolveError::SessionNotFound`] (it is not a session ae can locate); one
/// whose selector is Missing/Ambiguous is [`ResolveError::UnresolvableServer`].
/// Never a silent fall back to the ambient server — the ambient server is the
/// caller's, and enumerating a colliding name on it (pane-less, or cross-server)
/// is the exact mis-route this refuses. A real launch records an absolute socket
/// selector, so only legacy/corrupted meta reaches the refusal, which
/// `doctor --refresh` repairs.
fn named_server(dir: &Path, session: &str, own_session: &str) -> Result<ServerId, ResolveError> {
    let meta_dir = if session == own_session {
        dir.to_path_buf()
    } else {
        match dir.parent() {
            Some(root) => root.join(session),
            None => return Err(ResolveError::SessionNotFound(session.to_owned())),
        }
    };
    // Through `meta.rs`'s inventoried `read_bytes` door. An unreadable meta
    // (absent included) is a session ae cannot locate — SessionNotFound.
    let Ok(bytes) = meta::read_bytes(&meta_dir) else {
        return Err(ResolveError::SessionNotFound(session.to_owned()));
    };
    match meta::Meta::parse(&String::from_utf8_lossy(&bytes)).server_selector() {
        meta::ServerSelector::Positive(selector) => Ok(ServerId::Selected(selector)),
        meta::ServerSelector::Missing | meta::ServerSelector::Ambiguous => {
            Err(ResolveError::UnresolvableServer {
                session: session.to_owned(),
            })
        }
    }
}

// ---- the event ------------------------------------------------------------

/// Every member `ae_emit_event` writes for a tracked request, in its order.
/// An empty field is an absent member, as the frozen `[[ -n … ]] &&` guards
/// make it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventFields<'a> {
    /// `ts`.
    pub ts: Timestamp,
    /// `actor`.
    pub actor: &'a str,
    /// `action` — `ask` or `review`.
    pub action: &'a str,
    /// `target` — the resolved display ref, or the literal for a pane id
    /// without a stamp or an external sink.
    pub target: &'a str,
    /// `ref` — the request id.
    pub reference: &'a str,
    /// `actor_slot`.
    pub actor_slot: &'a str,
    /// `actor_session`.
    pub actor_session: &'a str,
    /// `target_slot`.
    pub target_slot: &'a str,
    /// `target_session`.
    pub target_session: &'a str,
    /// The raw body; flattened and capped here as the emitter does.
    pub summary: &'a str,
    /// `body_file` — the stored delivered text, or empty.
    pub body_file: &'a str,
}

/// One event line, `\n` included.
///
/// The summary is rendered HERE, for the event's action, by
/// [`crate::state::summary_for`] — flattened and capped at 200 characters for
/// every action but `chat`, which keeps its lines and tabs under the 3500 cap —
/// as `ae_emit_event`'s two arms render it. Callers hand the text raw; a
/// summary rendered twice would flatten a chat that the first pass had kept.
///
/// ```
/// use ae::time::Timestamp;
/// use ae::tracked::{EventFields, event_line};
///
/// let line = event_line(&EventFields {
///     ts: Timestamp::parse("2026-08-27T07:11:12Z").unwrap(),
///     actor: "cl:lead", action: "ask", target: "cl:w", reference: "ae-1",
///     actor_slot: "main", actor_session: "s", target_slot: "", target_session: "s",
///     summary: "a\tq", body_file: "/s/messages/ae-1.ask.x.txt",
/// });
/// assert_eq!(
///     line,
///     "{\"ts\":\"2026-08-27T07:11:12Z\",\"actor\":\"cl:lead\",\"action\":\"ask\",\"target\":\"cl:w\",\"ref\":\"ae-1\",\"actor_slot\":\"main\",\"actor_session\":\"s\",\"target_session\":\"s\",\"summary\":\"a q\",\"body_file\":\"/s/messages/ae-1.ask.x.txt\"}\n"
/// );
/// ```
#[must_use]
pub fn event_line(fields: &EventFields<'_>) -> String {
    let mut members = vec![
        ("ts", fields.ts.to_string()),
        ("actor", fields.actor.to_owned()),
        ("action", fields.action.to_owned()),
    ];
    let summary = state::summary_for(fields.action, fields.summary);
    for (key, value) in [
        ("target", fields.target),
        ("ref", fields.reference),
        ("actor_slot", fields.actor_slot),
        ("actor_session", fields.actor_session),
        ("target_slot", fields.target_slot),
        ("target_session", fields.target_session),
        ("summary", summary.as_str()),
        ("body_file", fields.body_file),
    ] {
        if !value.is_empty() {
            members.push((key, value.to_owned()));
        }
    }
    let mut line = Value::obj(
        members
            .into_iter()
            .map(|(key, value)| (key, Value::Str(value))),
    )
    .render();
    line.push('\n');
    line
}

// ---- the run --------------------------------------------------------------

/// Who is asking: the display ref and the routing slot (empty for an
/// `AE_SENDER_OVERRIDE` caller, which has no pane).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sender {
    /// The event's `actor` and the message's `from`.
    pub display: String,
    /// The event's `actor_slot`, or empty.
    pub slot: String,
}

/// The public `send` helper: the no-identity fallback, which records its own
/// event.
const SEND_HELPER: &str = "send";

/// The internal delivery-only entry to the send body: pastes, stores the
/// recovery file and prints its path; never emits. Generated beside `send` by
/// the same ae that bound this core, so a session whose helpers predate it is
/// repaired by `ae doctor --refresh`.
pub(crate) const DELIVER_HELPER: &str = "_send-deliver";

/// Run a tracked request end to end. `own_session` is the session the helper
/// serves — the one source for the event's `actor_session`, the resolver's
/// notion of "here" AND the caller's display ref, so the three can never
/// disagree. The frozen `_AE_SESSION` is meta's `session=` and nothing else,
/// and is EMPTY for a meta without that key — a state in which every bash
/// helper of that session is already half-working (the display ref gains an
/// `@session:` prefix, `actor_session` vanishes); this crate derives the
/// session as P2.1b does (meta, else the directory's own name) and does not
/// copy that state. `entropy` seeds the request id.
///
/// # Errors
///
/// Only a failure to write `out` or `err`.
#[allow(
    clippy::too_many_arguments,
    reason = "the frozen helper's inputs, spelled out rather than bundled"
)]
pub fn run(
    kind: Kind,
    dir: &Path,
    tail: &[String],
    sender: Option<&Sender>,
    own_session: &str,
    now: Timestamp,
    entropy: u64,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let action = kind.action();
    let Ok(parsed) = parse(tail) else {
        write!(err, "{}", kind.usage())?;
        return Ok(EXIT_USAGE);
    };
    if is_blank(&parsed.body) {
        write!(err, "{}", refusal(action))?;
        return Ok(EXIT_FAILED);
    }
    let Some(sender) = sender else {
        // The frozen fallback: a plain send, which writes its own event.
        let helper = dir.join(SEND_HELPER);
        write!(err, "{NO_IDENTITY_WARNING}")?;
        let delivery = transport::deliver(&helper, &parsed.target, &parsed.body, &[]);
        out.write_all(delivery.stdout.as_bytes())?;
        return delivery_code(&delivery, &helper, action, err);
    };
    let req_id = request_id(kind.id_prefix(), now, entropy);
    if is_external(&parsed.target) {
        // An event-only sink: the frozen send emits and exits, pasting nothing
        // and storing nothing.
        let line = event_line(&EventFields {
            ts: now,
            actor: &sender.display,
            action,
            target: &parsed.target,
            reference: &req_id,
            actor_slot: &sender.slot,
            actor_session: own_session,
            target_slot: "",
            target_session: "",
            summary: &parsed.body,
            body_file: "",
        });
        if let Err(why) = state::emit(dir, &line) {
            writeln!(err, "ae: {action} {req_id} not recorded: {why}")?;
            return Ok(EXIT_FAILED);
        }
        return Ok(0);
    }
    let resolved = match resolve(&parsed.target, own_session, dir) {
        Ok(resolved) => resolved,
        Err(why) => {
            writeln!(err, "{}", why.message())?;
            return Ok(EXIT_FAILED);
        }
    };
    let target_name = if resolved.agent.is_empty() {
        parsed.target.clone()
    } else {
        resolved.agent.clone()
    };
    let reply_cmd = reply_command(dir, &target_name, &req_id, kind.reply_label());
    let message = compose(kind, &req_id, &sender.display, &parsed.body, &reply_cmd);
    // The action and ref name the recovery file the body store writes.
    let helper = dir.join(DELIVER_HELPER);
    let delivery = transport::deliver(
        &helper,
        &target_name,
        &message,
        &[("_AE_EVENT_ACTION", action), ("_AE_EVENT_REF", &req_id)],
    );
    if delivery.code != Some(0) {
        return delivery_code(&delivery, &helper, action, err);
    }
    let body_file = delivery.stdout.trim_end_matches('\n');
    let line = event_line(&EventFields {
        ts: now,
        actor: &sender.display,
        action,
        target: &target_name,
        reference: &req_id,
        actor_slot: &sender.slot,
        actor_session: own_session,
        target_slot: &resolved.slot,
        target_session: &resolved.session,
        summary: &parsed.body,
        body_file,
    });
    if let Err(why) = state::emit(dir, &line) {
        writeln!(
            err,
            "ae: {action} {req_id} was delivered to {target_name} but its event was not emitted: {why}"
        )?;
        return Ok(EXIT_FAILED);
    }
    Ok(0)
}

/// The exit code a `send` run hands back: its own, verbatim; a helper that
/// could not run at all is said so, at [`EXIT_FAILED`].
pub(crate) fn delivery_code(
    delivery: &transport::Delivery,
    helper: &Path,
    action: &str,
    err: &mut impl Write,
) -> io::Result<u8> {
    let Some(code) = delivery.code else {
        writeln!(
            err,
            "ae: {action} not delivered: could not run {} (a session's helpers are regenerated by `ae doctor --refresh`)",
            helper.display()
        )?;
        return Ok(EXIT_FAILED);
    };
    Ok(u8::try_from(code).unwrap_or(EXIT_FAILED))
}

#[cfg(test)]
mod tests {
    use super::{
        Kind, Lookup, Parsed, ResolveError, Usage, compose, is_blank, is_external, lookup,
        named_server, pane_server, parse, pick, refusal, reply_command, request_id,
    };
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    use crate::time::Timestamp;
    use crate::tmux::ObservedAgent;

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    /// A sessions root under the temp dir with one session subdir per `(name,
    /// meta)`, each carrying the given meta text. Returns the root.
    fn sessions_root(tag: &str, sessions: &[(&str, &str)]) -> std::path::PathBuf {
        let root = std::env::temp_dir()
            .join(format!("aetrsrv.{}.{tag}", std::process::id()))
            .join("sessions");
        let _ = std::fs::remove_dir_all(&root);
        for (name, meta) in sessions {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).expect("a session dir");
            std::fs::write(dir.join("meta"), meta).expect("a meta file");
        }
        std::fs::create_dir_all(&root).expect("a sessions root");
        root
    }

    const SOCK_A: &str = "session=a\ntmux_server_kind=socket\ntmux_server=/srv/a.sock\n";
    const SOCK_B: &str = "session=b\ntmux_server_kind=socket\ntmux_server=/srv/b.sock\n";

    #[test]
    fn named_server_reads_the_target_sessions_own_recorded_server_not_the_callers() {
        let root = sessions_root("named-ok", &[("a", SOCK_A), ("b", SOCK_B)]);
        let a = root.join("a");
        // An unqualified target enumerates on the CALLER's own recorded server.
        assert_eq!(
            named_server(&a, "a", "a"),
            Ok(ServerId::Selected(Selector::Socket("/srv/a.sock".into()))),
        );
        // A cross-session target enumerates on the TARGET's server, read from the
        // TARGET's own meta — the whole point of correction 1. If it read the
        // caller's meta instead it would answer /srv/a.sock here.
        assert_eq!(
            named_server(&a, "b", "a"),
            Ok(ServerId::Selected(Selector::Socket("/srv/b.sock".into()))),
        );
    }

    #[test]
    fn named_server_fails_closed_two_ways() {
        let root = sessions_root("named-bad", &[("a", SOCK_A), ("blank", "session=blank\n")]);
        let a = root.join("a");
        // A recorded selector that is Missing (no usable pointer) REFUSES rather
        // than falling back to the ambient server — the mis-route correction 2
        // closes.
        assert_eq!(
            named_server(&a, "blank", "a"),
            Err(ResolveError::UnresolvableServer {
                session: "blank".to_owned(),
            }),
        );
        // A session with no meta at all cannot be located — SessionNotFound, not
        // an ambient guess.
        assert_eq!(
            named_server(&a, "ghost", "a"),
            Err(ResolveError::SessionNotFound("ghost".to_owned())),
        );
    }

    #[test]
    fn pane_server_uses_the_recorded_server_when_usable_and_ambient_otherwise() {
        let root = sessions_root("pane", &[("a", SOCK_A), ("blank", "session=blank\n")]);
        // A raw pane is unambiguous, so a usable recorded server only reads its
        // stamps…
        assert_eq!(
            pane_server(&root.join("a")),
            ServerId::Selected(Selector::Socket("/srv/a.sock".into())),
        );
        // …and an unusable or absent one degrades to ambient rather than
        // refusing (no roster is enumerated, so no name can be mis-routed).
        assert_eq!(pane_server(&root.join("blank")), ServerId::Ambient);
        assert_eq!(pane_server(&root.join("absent")), ServerId::Ambient);
    }

    fn roster(rows: &[(&str, &str)]) -> Vec<ObservedAgent> {
        rows.iter()
            .map(|(pane, agent)| ObservedAgent {
                pane: (*pane).to_owned(),
                agent: (*agent).to_owned(),
            })
            .collect()
    }

    #[test]
    fn argv_reads_as_the_helper_reads_it() {
        assert_eq!(
            parse(&words(&["cl:w", "two", "words"])),
            Ok(Parsed {
                target: "cl:w".to_owned(),
                body: "two words".to_owned()
            })
        );
        assert_eq!(parse(&words(&["cl:w"])), Err(Usage));
        assert_eq!(parse(&[]), Err(Usage));
        assert!(is_blank(" \t\n\u{b}\u{c}\r"));
        assert!(is_blank(""));
        assert!(!is_blank(" x "));
        assert!(refusal("ask").starts_with("ae: ask REFUSED — the message body is empty"));
        assert!(
            is_external("telegram:123") && is_external("ae:compact:u") && !is_external("ae:other")
        );
    }

    #[test]
    fn the_id_has_the_frozen_shape_and_the_message_the_frozen_bytes() {
        let now = Timestamp::parse("2026-08-27T07:11:12Z").unwrap();
        let id = request_id("review", now, u64::MAX);
        assert_eq!(id, "review-20260827T071112Z-ffffffff");
        assert_eq!(id.split('-').count(), 3, "parsed on dashes");
        let cmd = reply_command(
            std::path::Path::new("/h/.ae/sessions/s"),
            "cl:w",
            &id,
            Kind::Review.reply_label(),
        );
        assert_eq!(
            cmd,
            "/h/.ae/sessions/s/reply --as \"cl:w\" \"review-20260827T071112Z-ffffffff\" \"<your review>\""
        );
        let text = compose(Kind::Review, &id, "cl:lead", "look at x", &cmd);
        assert_eq!(
            text,
            format!(
                "REVIEW REQUEST {id} from cl:lead: look at x\n\n{}\n\nREQUIRED: When you have finished, you MUST run this exact command to reply:\n{cmd}\nDo not reply any other way. Do NOT use peek/peak as a reply mechanism.",
                super::REVIEW_INSTRUCTIONS
            )
        );
    }

    #[test]
    fn a_target_is_classified_before_anything_is_read() {
        assert_eq!(lookup("%3", "s"), Ok(Lookup::Pane("%3".to_owned())));
        assert_eq!(
            lookup("worker", "s"),
            Ok(Lookup::Named {
                session: "s".to_owned(),
                target: "worker".to_owned(),
                explicit: false
            })
        );
        assert_eq!(
            lookup("@other:cl:w", "s"),
            Ok(Lookup::Named {
                session: "other".to_owned(),
                target: "cl:w".to_owned(),
                explicit: true
            })
        );
        assert_eq!(
            lookup("@s:cl:w", "s"),
            Ok(Lookup::Named {
                session: "s".to_owned(),
                target: "cl:w".to_owned(),
                explicit: true
            }),
            "the own session, NAMED, is still checked with has-session"
        );
        assert_eq!(
            lookup("@other", "s"),
            Err(ResolveError::CrossSessionShape("@other".to_owned()))
        );
        assert_eq!(lookup("@:w", "s"), Err(ResolveError::CrossSessionEmpty));
        assert_eq!(lookup("@s:", "s"), Err(ResolveError::CrossSessionEmpty));
        assert_eq!(
            ResolveError::CrossSessionShape("@x".to_owned()).message(),
            "Error: cross-session target must be @session:agent, got '@x'"
        );
    }

    #[test]
    fn the_pick_is_exact_then_unique_alias_then_unique_bare_name() {
        let rows = roster(&[
            ("%1", "cl:lead"),
            ("%2", "cl:worker"),
            ("%3", "gx:lead"),
            ("%4", ""),
            ("%5", "solo"),
        ]);
        assert_eq!(
            pick(&rows, "cl:worker", "s", "s"),
            Ok(("%2", "cl:worker".to_owned()))
        );
        assert_eq!(
            pick(&rows, "gx", "s", "s"),
            Ok(("%3", "gx:lead".to_owned())),
            "unique alias"
        );
        assert_eq!(
            pick(&rows, "worker", "s", "s"),
            Ok(("%2", "cl:worker".to_owned())),
            "unique bare name"
        );
        assert_eq!(
            pick(&rows, "solo", "s", "s"),
            Ok(("%5", "solo".to_owned())),
            "no colon: alias and name are the whole stamp"
        );
        assert_eq!(
            pick(&rows, "cl", "s", "s"),
            Err(ResolveError::Ambiguous {
                target: "cl".to_owned(),
                session: "s".to_owned()
            }),
            "two cl: panes"
        );
        assert_eq!(
            pick(&rows, "lead", "s", "s"),
            Err(ResolveError::Ambiguous {
                target: "lead".to_owned(),
                session: "s".to_owned()
            }),
            "two :lead panes"
        );
        assert_eq!(
            pick(&rows, "nobody", "s", "s"),
            Err(ResolveError::NotFound {
                target: "nobody".to_owned(),
                session: "s".to_owned()
            })
        );
        assert_eq!(
            pick(&rows, "", "s", "s"),
            Ok(("%4", String::new())),
            "the frozen quirk, kept: an empty name is an exact match for an unstamped pane"
        );
        assert_eq!(
            pick(&rows, "worker", "other", "s"),
            Ok(("%2", "@other:cl:worker".to_owned())),
            "cross-session display"
        );
        // An ambiguous alias does not hide a unique bare name: the frozen
        // ifs are sequential, and the second still runs.
        let rows = roster(&[("%1", "x:a"), ("%2", "x:b"), ("%3", "y:x")]);
        assert_eq!(pick(&rows, "x", "s", "s"), Ok(("%3", "y:x".to_owned())));
        assert_eq!(
            ResolveError::Ambiguous {
                target: "cl".to_owned(),
                session: "s".to_owned()
            }
            .message(),
            "Error: ambiguous name 'cl' in session 's' — use alias:name format"
        );
        assert_eq!(
            ResolveError::NotFound {
                target: "n".to_owned(),
                session: "s".to_owned()
            }
            .message(),
            "Error: agent 'n' not found in session 's'"
        );
        assert_eq!(
            ResolveError::SessionNotFound("o".to_owned()).message(),
            "Error: session 'o' not found"
        );
    }
}
