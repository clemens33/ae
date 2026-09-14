//! Tracked requests — the `ask` and `review` helpers.
//!
//! Up to the paste: the body is refused when blank; the caller is
//! `AE_SENDER_OVERRIDE` or the pane's own stamp (no identity at all falls back
//! to a plain `send`, with a warning); an external sink (`telegram:*`,
//! `discord:*`, `ae:compact:*`) is event-only; any other target is resolved —
//! `%pane` passthrough, `@session:agent` across sessions when explicitly
//! authorized, exact `alias:name`, else a unique alias, else a unique bare name;
//! a request id is minted (`<prefix>-<YYYYMMDDTHHMMSSZ>-<8 hex>`); the message
//! is composed with the header, the optional review instructions and the
//! REQUIRED reply footer whose command names the resolved target, the id and
//! the reply label.
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::inventory::ServerId;
use crate::json::Value;
use crate::meta;
use crate::requests::is_slot;
use crate::state::{self, EXIT_FAILED, EXIT_USAGE};
use crate::store;
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

/// The `ask` usage text.
pub const ASK_USAGE: &str = "Usage: ask [--cross-session] <agent-name|pane-id|@session:agent> <question>\n  Like send, but embeds your identity and reply command in the message.\n";

/// The `review` usage text.
pub const REVIEW_USAGE: &str = "Usage: review [--cross-session] <agent-name|pane-id|@session:agent> <request>\n  Ask another agent for a critical review and require a reply via send.\n";

/// The explicit capability flag for a delivery outside the helper's session.
pub const CROSS_SESSION_FLAG: &str = "--cross-session";

/// The review instructions — its continuation lines carry four spaces of
/// indentation.
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
    /// Whether the caller states that the human explicitly authorized a
    /// cross-session delivery.
    pub cross_session: bool,
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
    let (cross_session, tail) = split_cross_session_flag(tail);
    match tail {
        [target, words @ ..] if !words.is_empty() => Ok(Parsed {
            cross_session,
            target: target.clone(),
            body: words.join(" "),
        }),
        _ => Err(Usage),
    }
}

/// Remove the one supported leading delivery capability flag.
#[must_use]
pub fn split_cross_session_flag(tail: &[String]) -> (bool, &[String]) {
    match tail.split_first() {
        Some((flag, rest)) if flag == CROSS_SESSION_FLAG => (true, rest),
        _ => (false, tail),
    }
}

/// `ae_require_body`'s test: nothing but `[[:space:]]` — space, tab, newline,
/// vertical tab, form feed, carriage return.
#[must_use]
pub fn is_blank(body: &str) -> bool {
    body.chars()
        .all(|c| c.is_ascii_whitespace() || c == '\u{b}')
}

/// The refusal for a blank body — two lines, stderr, exit
/// [`EXIT_FAILED`], nothing sent.
#[must_use]
pub fn refusal(action: &str) -> String {
    format!(
        "ae: {action} REFUSED — the message body is empty (or only whitespace). Nothing was sent.\nae: a delivered header with no body reads as a message that was received and said nothing.\n"
    )
}

/// The warning when the caller has no identity, before the fallback
/// to a plain `send`.
pub const NO_IDENTITY_WARNING: &str = "Warning: could not detect caller identity (no @ae_agent on this pane). Using 'send' instead.\n";

/// Whether `target` is an event-only sink that is never resolved:
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

/// Whether `text` is a request id this ae minted for a production ask or
/// review: `<ae|review>-<YYYYMMDDTHHMMSSZ>-<8 lowercase hex>`, at most 32
/// chars. The grammar's one owner, beside the minter; R15's budget check
/// calls this, never a copy.
///
/// DELIBERATELY narrower than [`request_id`]: the public minter takes any
/// prefix and any [`Timestamp`], so it can produce foreign prefixes and
/// five-digit-year stamps — this validator REJECTS those. Prefixes come from
/// [`Kind::id_prefix`], the one owner of the production words. The stamp is
/// digit SHAPE only, not calendar truth: a non-calendar digit stamp cannot be
/// minted (the clock only emits real dates) and is clean bounded ASCII either
/// way, so shape is the whole of the bind.
///
/// ```
/// use ae::time::Timestamp;
/// use ae::tracked::{Kind, is_request_id, request_id};
///
/// let now = Timestamp::parse("2026-08-27T07:11:12Z").unwrap();
/// let minted = request_id(Kind::Ask.id_prefix(), now, 7);
/// assert!(is_request_id(&minted));
/// assert!(!is_request_id("ae-x"));
/// assert!(!is_request_id("xx-20260827T071112Z-00000007"));
/// ```
#[must_use]
pub fn is_request_id(text: &str) -> bool {
    let after_prefix = [Kind::Ask, Kind::Review]
        .iter()
        .find_map(|kind| text.strip_prefix(kind.id_prefix()));
    let Some(rest) = after_prefix.and_then(|tail| tail.strip_prefix('-')) else {
        return false;
    };
    // `<YYYYMMDD>T<HHMMSS>Z-<8 lowercase hex>`: 25 chars exactly, so the
    // whole id is 28 (`ae`) or 32 (`review`) — the stated bound holds by
    // construction, no separate length check.
    let stamp = rest.as_bytes();
    if stamp.len() != 25 {
        return false;
    }
    let digits = |part: &[u8]| part.iter().all(u8::is_ascii_digit);
    let hex = |part: &[u8]| part.iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    digits(&stamp[0..8])
        && stamp[8] == b'T'
        && digits(&stamp[9..15])
        && stamp[15] == b'Z'
        && stamp[16] == b'-'
        && hex(&stamp[17..25])
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

/// The delivered text, before the provenance envelope `send` prepends: `<header> <id> from <sender>: <body>`, the instructions block
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

// ---- resolution -----------------------------------------------------------

/// What the resolver produces.
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

/// Why a target did not resolve — each with its own error line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// `@something` without a `:`.
    CrossSessionShape(String),
    /// `@:agent` or `@session:`.
    CrossSessionEmpty,
    /// The named session is not on the server.
    SessionNotFound(String),
    /// More than one pane carries the target identity.
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
    /// Missing/Ambiguous, or its meta is unreadable).
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
            Self::Ambiguous { target, session } => {
                format!("Error: ambiguous name '{target}' in session '{session}'")
            }
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
        /// Whether the session was NAMED (`@session:agent`).
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
    // A stamp is the BARE NAME and the agent-name grammar forbids a `:` inside
    // one — so `<session>:<name>` means across sessions without the `@`,
    // unambiguously, and `aedev:lead` addresses the same pane `@aedev:lead`
    // does. `fable5:lead` is therefore a session named `fable5`, and does not
    // reach a pane stamped `lead`.
    if let Some((session, name)) = target.split_once(':')
        && !session.is_empty()
        && !name.is_empty()
        && !name.contains(':')
    {
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

/// The pick over a roster: the pane whose `@ae_agent` stamp IS the target,
/// exactly.
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
    let mut matches = roster.iter().filter(|row| row.agent == target);
    let first = matches.next();
    let second = matches.next();
    let target = target.to_owned();
    let session = session.to_owned();
    match (first, second) {
        (Some(row), None) => Ok((row.pane.as_str(), display(&row.agent))),
        (Some(_), Some(_)) => Err(ResolveError::Ambiguous { target, session }),
        (None, _) => Err(ResolveError::NotFound { target, session }),
    }
}

/// `ae_resolve`, against the ambient server.
///
/// # Errors
///
/// [`ResolveError`] — see its variants. A pane named by id always resolves
/// (`send` fails later if it is not there); its stamps are simply empty when
/// they cannot be read.
pub fn resolve(target: &str, own_session: &str, dir: &Path) -> Result<Resolved, ResolveError> {
    resolve_on(target, own_session, dir).map(|(resolved, _)| resolved)
}

/// [`resolve`], and the SERVER the target was resolved on.
///
/// # Errors
///
/// [`ResolveError`] — see its variants, exactly as [`resolve`] reports them.
pub fn resolve_on(
    target: &str,
    own_session: &str,
    dir: &Path,
) -> Result<(Resolved, ServerId), ResolveError> {
    #[cfg(test)]
    if let Some(hit) = take_test_resolve() {
        return Ok(hit);
    }
    let (server, pane, agent) = match lookup(target, own_session)? {
        Lookup::Pane(pane) => {
            // A raw pane id is an unambiguous address on its own server, so there
            // is nothing to enumerate and no name to collide: the recorded server
            // only lets its stamps be read, and an unusable one leaves them empty
            // (stamps are simply empty when they cannot be read) rather than
            // refusing. No mis-route is possible, so this never fails.
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
    Ok((
        Resolved {
            pane,
            agent,
            slot: observed
                .slot
                .filter(|slot| is_slot(slot))
                .unwrap_or_default(),
            session: observed.session.unwrap_or_default(),
        },
        server,
    ))
}

/// The server for reading a RAW PANE target's stamps: the caller session's
/// recorded one when it is usable, else the ambient server.
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
pub(crate) fn named_server(
    dir: &Path,
    session: &str,
    own_session: &str,
) -> Result<ServerId, ResolveError> {
    let meta_dir = if session == own_session {
        dir.to_path_buf()
    } else {
        match dir.parent() {
            Some(root) => root.join(session),
            None => return Err(ResolveError::SessionNotFound(session.to_owned())),
        }
    };
    // Through `meta.rs`'s inventoried `read_bytes` door.
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

// ---- caller / target incarnation ------------------------------------------

/// The three facts that prove a pane's session incarnation across servers.
///
/// Empty members never match: a gap is not an identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityTriple {
    /// Canonical tmux socket path for the pane's server.
    pub server: String,
    /// Pane id (`%N`).
    pub pane: String,
    /// Canonical `@ae_session_uuid` / meta `session_id`.
    pub session_uuid: String,
}

impl IdentityTriple {
    /// The one equality: nonempty server, pane and uuid, all three equal.
    /// Public callers use the two named questions, not this predicate.
    fn same_incarnation(&self, other: &Self) -> bool {
        !self.server.is_empty()
            && !self.pane.is_empty()
            && !self.session_uuid.is_empty()
            && self.server == other.server
            && self.pane == other.pane
            && self.session_uuid == other.session_uuid
    }
}

/// Why a pane UUID did not correlate with a session meta. Each variant is a
/// named gap — unreadable, vacant and mismatch never collapse into each other.
/// [`Self::NoSession`] is not a gap: it says the question never arose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorrelationGap {
    /// No session identity existed to correlate: the caller had no pane
    /// context, or neither the pane nor the `meta` recorded an identity.
    /// Nothing failed, so nothing is named.
    NoSession,
    /// The pane-keyed query did not answer.
    Unreadable,
    /// The query answered and `@ae_session_uuid` was empty.
    Vacant,
    /// The option was set but is not a canonical UUID.
    Invalid,
    /// Option and meta both name a UUID, and they differ.
    Mismatch,
    /// No `meta` file.
    MetaMissing,
    /// `meta` exists but is not a regular file — symlink, directory, FIFO.
    MetaNonregular,
    /// A regular `meta` that could not be read.
    MetaUnreadable,
    /// A regular `meta` with no usable `session_id` row.
    MetaEmpty,
    /// `session_id` is present but is not a canonical UUID.
    MetaMalformed,
    /// Two `session_id` rows; the document does not say one thing.
    MetaDuplicate,
}

impl CorrelationGap {
    /// The gap name a refusal quotes.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::NoSession => "no session identity to correlate",
            Self::Unreadable => "session identity unreadable",
            Self::Vacant => "session identity not recorded",
            Self::Invalid => "session identity invalid",
            Self::Mismatch => "session identity mismatch",
            Self::MetaMissing => "meta: missing",
            Self::MetaNonregular => "meta: not a regular file",
            Self::MetaUnreadable => "meta: unreadable",
            Self::MetaEmpty => "meta: no identity",
            Self::MetaMalformed => "meta: identity malformed",
            Self::MetaDuplicate => "meta: duplicate identity",
        }
    }
}

/// Correlate a pane-keyed `@ae_session_uuid` reading with `meta` bytes.
///
/// The option is judged first: a failed observation is never treated as
/// vacant. A UUID used here is proved with [`crate::archive::canonical_uuid`].
///
/// # Errors
///
/// [`CorrelationGap`] naming which leg failed.
pub fn correlate_uuid(
    option: &crate::tmux::OptionReading,
    meta: &[u8],
) -> Result<String, CorrelationGap> {
    let option_uuid = match option {
        crate::tmux::OptionReading::Unknown => return Err(CorrelationGap::Unreadable),
        crate::tmux::OptionReading::Vacant => {
            // The pane records no identity. With no identity in `meta` either,
            // there is nothing to correlate and no failed correlation to name.
            // A meta that records one is a real failed proof — keep naming it.
            return match meta::first_value(meta, "session_id") {
                Some(raw) if !raw.is_empty() => Err(CorrelationGap::Vacant),
                _ => Err(CorrelationGap::NoSession),
            };
        }
        crate::tmux::OptionReading::Set(value) => {
            let canonical = crate::archive::canonical_uuid(value);
            if canonical.is_empty() {
                return Err(CorrelationGap::Invalid);
            }
            canonical
        }
    };
    match (
        meta::sole_value(meta, "session_id"),
        meta::first_value(meta, "session_id"),
    ) {
        (None, Some(_)) => Err(CorrelationGap::MetaDuplicate),
        (None, None) | (Some(b""), _) => Err(CorrelationGap::MetaEmpty),
        (Some(raw), _) => {
            let meta_uuid = crate::archive::canonical_uuid(&String::from_utf8_lossy(raw));
            if meta_uuid.is_empty() {
                Err(CorrelationGap::MetaMalformed)
            } else if meta_uuid == option_uuid {
                Ok(option_uuid)
            } else {
                Err(CorrelationGap::Mismatch)
            }
        }
    }
}

/// What a proof-before-cut plus proof-after-cut decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorrelationOutcome {
    /// The same nonempty triple was proved on both sides.
    Correlated(IdentityTriple),
    /// Both sides proved a triple, and they differ.
    Changed,
    /// At least one side could not prove a triple it had to prove.
    Failed(CorrelationGap),
    /// No session identity existed to correlate: the call carried no session
    /// context, or neither side recorded an identity. The question never
    /// arose; there is no failed correlation to name.
    NoSession,
}

impl CorrelationOutcome {
    /// Classify ONE observation, as [`retain_correlation`] classifies a cut: a
    /// missing session identity is not a failed correlation.
    #[must_use]
    pub fn from_observation(result: Result<IdentityTriple, CorrelationGap>) -> Self {
        match result {
            Ok(triple) => Self::Correlated(triple),
            Err(CorrelationGap::NoSession) => Self::NoSession,
            Err(gap) => Self::Failed(gap),
        }
    }

    /// The triple to write, if correlation held.
    #[must_use]
    pub fn triple(&self) -> Option<&IdentityTriple> {
        match self {
            Self::Correlated(triple) => Some(triple),
            Self::Changed | Self::Failed(_) | Self::NoSession => None,
        }
    }

    /// Named failed leg, or `None` when correlated OR when no session identity
    /// existed to correlate. Writers must report this.
    #[must_use]
    pub fn name(&self) -> Option<&'static str> {
        match self {
            Self::Correlated(_) | Self::NoSession => None,
            Self::Changed => Some("session identity changed"),
            Self::Failed(gap) => Some(gap.name()),
        }
    }
}

/// Keep correlated facts only when the same triple is proved on both sides of
/// a durable cut. A change, or a gap on either side, writes no correlated event.
/// When NEITHER side had a session identity there was no question to answer:
/// the outcome is [`CorrelationOutcome::NoSession`], and no gap is named. One
/// side absent while the other proved an identity is still a failed correlation.
#[must_use]
pub fn retain_correlation(
    before: &Result<IdentityTriple, CorrelationGap>,
    after: &Result<IdentityTriple, CorrelationGap>,
) -> CorrelationOutcome {
    match (before, after) {
        (Ok(left), Ok(right)) if left.same_incarnation(right) => {
            CorrelationOutcome::Correlated(left.clone())
        }
        (Ok(_), Ok(_)) => CorrelationOutcome::Changed,
        (Err(CorrelationGap::NoSession), Err(CorrelationGap::NoSession)) => {
            CorrelationOutcome::NoSession
        }
        (_, Err(gap)) | (Err(gap), _) => CorrelationOutcome::Failed(*gap),
    }
}

/// (a) Checkpoint integrity: is this record from the incarnation that is here
/// NOW? Compactseats' question. Do not pass the opening target triple here.
#[must_use]
pub fn caller_matches_live(caller: &IdentityTriple, live: &IdentityTriple) -> bool {
    caller.same_incarnation(live)
}

/// (b) Request-routing integrity: is the responder the incarnation this
/// request was opened against? The waiter's question. Do not pass live
/// session identity here.
#[must_use]
pub fn caller_matches_recorded_target(caller: &IdentityTriple, recorded: &IdentityTriple) -> bool {
    caller.same_incarnation(recorded)
}

/// Build the triple from ONE viewer observation. The server fact is the
/// format's `#{socket_path}` field — never the selector used to address tmux.
///
/// # Errors
///
/// [`CorrelationGap`] naming which leg failed.
pub fn triple_from_viewer(
    observed: &crate::tmux::ObservedViewer,
    pane: &str,
    meta_dir: &Path,
) -> Result<IdentityTriple, CorrelationGap> {
    let Some(server) = observed
        .socket_path
        .as_deref()
        .filter(|path| !path.is_empty())
    else {
        return Err(CorrelationGap::Unreadable);
    };
    if pane.is_empty() {
        return Err(CorrelationGap::Unreadable);
    }
    let session_uuid = correlate_uuid_in(&observed.session_uuid, meta_dir)?;
    Ok(IdentityTriple {
        server: server.to_owned(),
        pane: pane.to_owned(),
        session_uuid,
    })
}

/// Correlate a pane-keyed uuid reading with the classified `meta` node.
pub(crate) fn correlate_uuid_in(
    option: &crate::tmux::OptionReading,
    meta_dir: &Path,
) -> Result<String, CorrelationGap> {
    match crate::store::read_source(&crate::store::open(meta_dir).meta_path()) {
        crate::store::SourceRead::Absent => Err(CorrelationGap::MetaMissing),
        crate::store::SourceRead::Invalid(_) => Err(CorrelationGap::MetaNonregular),
        crate::store::SourceRead::Unreadable(_) => Err(CorrelationGap::MetaUnreadable),
        crate::store::SourceRead::Ready(bytes) => correlate_uuid(option, &bytes),
    }
}

/// Observe one pane on one server and correlate its uuid with `meta_dir`.
///
/// ONE pane-keyed tmux query (viewer fields + uuid + `socket_path`). A failed
/// query is unreadable, never vacant.
///
/// # Errors
///
/// [`CorrelationGap`] naming which leg failed.
pub fn observe_triple(
    server: &ServerId,
    pane: &str,
    meta_dir: &Path,
) -> Result<IdentityTriple, CorrelationGap> {
    #[cfg(test)]
    if let Some(hit) = take_observe() {
        return hit;
    }
    let Some(observed) = transport::observe_viewer(server, pane) else {
        return Err(CorrelationGap::Unreadable);
    };
    triple_from_viewer(&observed, pane, meta_dir)
}

/// Observe the calling pane on the caller's server against the helper directory.
///
/// # Errors
///
/// [`CorrelationGap`] naming which leg failed.
pub fn observe_caller(dir: &Path) -> Result<IdentityTriple, CorrelationGap> {
    #[cfg(test)]
    if let Some(hit) = take_observe() {
        return hit;
    }
    // No pane and no server is no session context at all: there is nothing to
    // correlate against, and an absent context is not a failed correlation.
    let pane = crate::doors::calling_pane_id().ok_or(CorrelationGap::NoSession)?;
    let server = crate::doors::caller_server().ok_or(CorrelationGap::NoSession)?;
    observe_triple(&server, &pane, dir)
}

#[cfg(test)]
thread_local! {
    static NEXT_OBSERVE: std::cell::RefCell<Vec<Result<IdentityTriple, CorrelationGap>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) fn queue_observe(result: Result<IdentityTriple, CorrelationGap>) {
    NEXT_OBSERVE.with(|queue| queue.borrow_mut().push(result));
}

#[cfg(test)]
fn take_observe() -> Option<Result<IdentityTriple, CorrelationGap>> {
    NEXT_OBSERVE.with(|queue| {
        let mut queue = queue.borrow_mut();
        (!queue.is_empty()).then(|| queue.remove(0))
    })
}

#[cfg(test)]
pub(crate) fn clear_observe() {
    NEXT_OBSERVE.with(|queue| queue.borrow_mut().clear());
}

#[cfg(test)]
thread_local! {
    static TEST_RESOLVE: std::cell::RefCell<Option<(Resolved, ServerId)>> =
        const { std::cell::RefCell::new(None) };
    static TEST_DELIVER: std::cell::RefCell<
        Option<Result<crate::deliver::Delivered, crate::deliver::Failure>>,
    > = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_test_resolve(resolved: Resolved, server: ServerId) {
    TEST_RESOLVE.with(|slot| *slot.borrow_mut() = Some((resolved, server)));
}

#[cfg(test)]
pub(crate) fn set_test_delivery(
    result: Result<crate::deliver::Delivered, crate::deliver::Failure>,
) {
    TEST_DELIVER.with(|slot| *slot.borrow_mut() = Some(result));
}

#[cfg(test)]
fn take_test_resolve() -> Option<(Resolved, ServerId)> {
    TEST_RESOLVE.with(|slot| slot.borrow_mut().take())
}

#[cfg(test)]
fn take_test_delivery() -> Option<Result<crate::deliver::Delivered, crate::deliver::Failure>> {
    TEST_DELIVER.with(|slot| slot.borrow_mut().take())
}

#[cfg(test)]
pub(crate) fn clear_test_hooks() {
    clear_observe();
    TEST_RESOLVE.with(|slot| *slot.borrow_mut() = None);
    TEST_DELIVER.with(|slot| *slot.borrow_mut() = None);
}

/// Delivery used by ask/review/reply so tests can inject a completed paste.
pub(crate) fn deliver_request(
    request: &crate::deliver::Request<'_>,
    err: &mut impl Write,
) -> io::Result<Result<crate::deliver::Delivered, crate::deliver::Failure>> {
    #[cfg(test)]
    if let Some(hit) = take_test_delivery() {
        return Ok(hit);
    }
    crate::deliver::deliver(request, err)
}

/// Observe, run `cut`, re-observe. Correlation is kept only when the triple is
/// unchanged across the cut.
fn across_cut<T>(
    server: &ServerId,
    pane: &str,
    meta_dir: &Path,
    cut: impl FnOnce() -> T,
) -> (T, CorrelationOutcome) {
    across_cut_with(|| observe_triple(server, pane, meta_dir), cut)
}

/// The testable cut: inject the observation.
pub(crate) fn across_cut_with<T>(
    mut observe: impl FnMut() -> Result<IdentityTriple, CorrelationGap>,
    cut: impl FnOnce() -> T,
) -> (T, CorrelationOutcome) {
    let before = observe();
    let value = cut();
    let after = observe();
    (value, retain_correlation(&before, &after))
}

/// Observe the caller, run `cut`, re-observe.
pub(crate) fn caller_across_cut<T>(dir: &Path, cut: impl FnOnce() -> T) -> (T, CorrelationOutcome) {
    across_cut_with(|| observe_caller(dir), cut)
}

/// Meta directory for an admitted target session.
fn target_meta_dir(dir: &Path, resolved_session: &str, own_session: &str) -> PathBuf {
    if resolved_session.is_empty() || resolved_session == own_session {
        dir.to_path_buf()
    } else {
        dir.parent()
            .map_or_else(|| dir.to_path_buf(), |root| root.join(resolved_session))
    }
}

// ---- the event ------------------------------------------------------------

/// Every member `ae_emit_event` writes for a tracked request, in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Canonical socket path of the admitted target server, or empty.
    pub target_server: &'a str,
    /// Pane id of the admitted target, or empty.
    pub target_pane: &'a str,
    /// Canonical UUID of the admitted target session, or empty.
    pub target_session_uuid: &'a str,
    /// Canonical socket path of the calling pane's server, or empty.
    pub caller_server: &'a str,
    /// Calling pane id, or empty.
    pub caller_pane: &'a str,
    /// Canonical UUID of the calling pane's session, or empty.
    pub caller_session_uuid: &'a str,
    /// Named failed correlation (`CorrelationOutcome::name`): an identity
    /// existed, correlation was attempted and failed. Empty when correlated,
    /// and empty when no session identity existed to correlate — the key is
    /// then absent from the record, never `""` or `"unknown"`.
    pub identity_gap: &'a str,
    /// The raw body; flattened and capped here as the emitter does.
    pub summary: &'a str,
    /// `body_file` — the stored delivered text, or empty.
    pub body_file: &'a str,
}

impl<'a> EventFields<'a> {
    /// An event with no incarnation facts. Empty identity keys stay unwritten.
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "mirrors the event members; identity facts default empty"
    )]
    pub fn new(
        ts: Timestamp,
        actor: &'a str,
        action: &'a str,
        target: &'a str,
        reference: &'a str,
        actor_slot: &'a str,
        actor_session: &'a str,
        target_slot: &'a str,
        target_session: &'a str,
        summary: &'a str,
        body_file: &'a str,
    ) -> Self {
        Self {
            ts,
            actor,
            action,
            target,
            reference,
            actor_slot,
            actor_session,
            target_slot,
            target_session,
            target_server: "",
            target_pane: "",
            target_session_uuid: "",
            caller_server: "",
            caller_pane: "",
            caller_session_uuid: "",
            identity_gap: "",
            summary,
            body_file,
        }
    }

    /// Fill caller incarnation facts proved on both sides of a durable cut.
    #[must_use]
    pub fn with_caller(self, triple: Option<&'a IdentityTriple>) -> Self {
        let Some(triple) = triple else {
            return self;
        };
        Self {
            caller_server: &triple.server,
            caller_pane: &triple.pane,
            caller_session_uuid: &triple.session_uuid,
            ..self
        }
    }

    /// Fill target incarnation facts proved on both sides of a durable cut.
    #[must_use]
    pub fn with_target(self, triple: Option<&'a IdentityTriple>) -> Self {
        let Some(triple) = triple else {
            return self;
        };
        Self {
            target_server: &triple.server,
            target_pane: &triple.pane,
            target_session_uuid: &triple.session_uuid,
            ..self
        }
    }
}

/// Report a failed or changed proof on the writer boundary, then stamp target.
pub(crate) fn stamp_target<'a>(
    fields: &EventFields<'a>,
    outcome: &'a CorrelationOutcome,
    err: &mut impl Write,
) -> EventFields<'a> {
    report_identity(err, fields.action, outcome);
    EventFields {
        identity_gap: outcome.name().unwrap_or(""),
        ..fields.with_target(outcome.triple())
    }
}

/// Report a failed or changed proof on the writer boundary, then stamp caller.
pub(crate) fn stamp_caller<'a>(
    fields: &EventFields<'a>,
    outcome: &'a CorrelationOutcome,
    err: &mut impl Write,
) -> EventFields<'a> {
    report_identity(err, fields.action, outcome);
    EventFields {
        identity_gap: outcome.name().unwrap_or(""),
        ..fields.with_caller(outcome.triple())
    }
}

fn report_identity(err: &mut impl Write, action: &str, outcome: &CorrelationOutcome) {
    // Failed (unreadable/vacant/mismatch) omits the fields and stays silent:
    // an unseeded helper is the common path, and frozen CLI tests pin empty
    // stderr on success. Changed across the cut is the anomaly the operator
    // must see.
    if matches!(outcome, CorrelationOutcome::Changed) {
        let _ = writeln!(
            err,
            "ae: {action} identity not correlated (session identity changed)"
        );
    }
}

/// One event line, `\n` included.
///
/// The summary is rendered HERE, for the event's action, by
/// [`crate::state::summary_for`] — flattened and capped at 200 characters for
/// ordinary actions, keeps `chat` lines and tabs under the 3500 cap, and keeps
/// a `relay` caller audit exact — as the emitter's arms render them. Callers hand the text raw; a
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
///     target_server: "", target_pane: "", target_session_uuid: "",
///     caller_server: "", caller_pane: "", caller_session_uuid: "", identity_gap: "",
///     summary: "a\tq", body_file: "/s/messages/ae-1.ask.x.txt",
/// });
/// assert_eq!(
///     line,
///     "{\"ts\":\"2026-08-27T07:11:12Z\",\"actor\":\"cl:lead\",\"action\":\"ask\",\"target\":\"cl:w\",\"ref\":\"ae-1\",\"actor_slot\":\"main\",\"actor_session\":\"s\",\"target_session\":\"s\",\"summary\":\"a q\",\"body_file\":\"/s/messages/ae-1.ask.x.txt\"}\n"
/// );
/// ```
#[must_use]
pub fn event_line(fields: &EventFields<'_>) -> String {
    render_event_line(fields, false, None)
}

/// Render a delivered event, retaining submit uncertainty as an additive record
/// field rather than a broadcast summary.
#[must_use]
pub(crate) fn delivery_event_line(
    fields: &EventFields<'_>,
    verification: crate::deliver::DeliveryVerification,
    cross_session: bool,
) -> String {
    render_event_line(fields, cross_session, verification.unverifiable_marker())
}

/// Render the common event shape, optionally carrying delivery and route facts.
fn render_event_line(
    fields: &EventFields<'_>,
    cross_session: bool,
    unverifiable: Option<&str>,
) -> String {
    let mut members = vec![("ts", Value::Str(fields.ts.to_string()))];
    if let Some(reason) = unverifiable {
        members.push(("unverifiable", Value::Str(reason.to_owned())));
    }
    members.extend([
        ("actor", Value::Str(fields.actor.to_owned())),
        ("action", Value::Str(fields.action.to_owned())),
    ]);
    let summary = state::summary_for(fields.action, fields.summary);
    for (key, value) in [
        ("target", fields.target),
        ("ref", fields.reference),
        ("actor_slot", fields.actor_slot),
        ("actor_session", fields.actor_session),
        ("target_slot", fields.target_slot),
        ("target_session", fields.target_session),
        ("target_server", fields.target_server),
        ("target_pane", fields.target_pane),
        ("target_session_uuid", fields.target_session_uuid),
        ("caller_server", fields.caller_server),
        ("caller_pane", fields.caller_pane),
        ("caller_session_uuid", fields.caller_session_uuid),
        ("identity_gap", fields.identity_gap),
        ("summary", summary.as_str()),
        ("body_file", fields.body_file),
    ] {
        if !value.is_empty() {
            members.push((key, Value::Str(value.to_owned())));
        }
    }
    if cross_session {
        members.push(("cross_session", Value::Bool(true)));
    }
    let mut line = Value::obj(members).render();
    line.push('\n');
    line
}

/// Render the summary of a delivery whose submit was not confirmed.
///
/// The body file remains the recovery source, while the event makes the
/// uncertainty visible to `requests` and `events-tail` without changing the
/// frozen event shape.
#[must_use]
pub(crate) fn unconfirmed_summary(summary: &str) -> String {
    format!("[unconfirmed] {summary}")
}

/// Render an event for a delivery whose submit was not confirmed.
#[must_use]
pub(crate) fn unconfirmed_event_line(fields: &EventFields<'_>) -> String {
    render_unconfirmed_event_line(fields, false)
}

/// Render an unconfirmed cross-session event.
#[must_use]
pub(crate) fn cross_session_unconfirmed_event_line(fields: &EventFields<'_>) -> String {
    render_unconfirmed_event_line(fields, true)
}

fn render_unconfirmed_event_line(fields: &EventFields<'_>, cross_session: bool) -> String {
    let summary = unconfirmed_summary(fields.summary);
    let fields = EventFields {
        summary: &summary,
        ..*fields
    };
    render_event_line(&fields, cross_session, None)
}

/// Refuse a resolved target outside `caller_session` unless the caller supplied
/// [`CROSS_SESSION_FLAG`]. Resolution deliberately precedes this check, so an
/// unknown target keeps its existing resolution error.
///
/// # Errors
///
/// Only a failure to write the refusal to `err`.
#[allow(
    clippy::too_many_arguments,
    reason = "the boundary audit needs the resolved route and caller identity spelled out"
)]
pub fn refuse_cross_session(
    dir: &Path,
    helper: &str,
    allowed: bool,
    typed_target: &str,
    resolved: &Resolved,
    actor: &str,
    actor_slot: &str,
    caller_session: &str,
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<bool> {
    if allowed || resolved.session.is_empty() || resolved.session == caller_session {
        return Ok(false);
    }
    let canonical = resolved.agent.strip_prefix('@').map_or_else(
        || {
            let agent = if resolved.agent.is_empty() {
                resolved.pane.as_str()
            } else {
                resolved.agent.as_str()
            };
            format!("{}:{agent}", resolved.session)
        },
        ToOwned::to_owned,
    );
    let refusal = format!(
        "ae: {helper} to {canonical} (typed {typed_target}) REFUSED — another ae session; pass --cross-session only when the human explicitly instructed it"
    );
    let line = event_line(&EventFields {
        ts: now,
        actor,
        action: "refused",
        target: &canonical,
        reference: "",
        actor_slot,
        actor_session: caller_session,
        target_slot: &resolved.slot,
        target_session: &resolved.session,
        target_server: "",
        target_pane: "",
        target_session_uuid: "",
        caller_server: "",
        caller_pane: "",
        caller_session_uuid: "",
        identity_gap: "",
        summary: &refusal,
        body_file: "",
    });
    // The refusal itself remains the one promised diagnostic even if its
    // audit append cannot be made; no delivery is attempted either way.
    let _ = participant_dir(dir, caller_session).and_then(|caller_dir| {
        store::open(&caller_dir)
            .append_event(&line)
            .map_err(io::Error::from)
    });
    writeln!(err, "{refusal}")?;
    Ok(true)
}

/// The two participants in one authorized cross-session delivery.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CrossSession<'a> {
    /// The physical caller's session, or the helper session for a pane-less
    /// caller.
    pub caller: &'a str,
    /// The resolved target pane's session.
    pub target: &'a str,
}

/// Append one ordinary event to the helper ledger, or one cross-session event
/// to both participant ledgers.
pub(crate) fn append_delivery_event(
    dir: &Path,
    line: &str,
    cross_session: Option<CrossSession<'_>>,
) -> io::Result<()> {
    let Some(cross_session) = cross_session else {
        return store::open(dir).append_event(line).map_err(io::Error::from);
    };
    let caller_dir = participant_dir(dir, cross_session.caller)?;
    let target_dir = participant_dir(dir, cross_session.target)?;
    store::open(&caller_dir)
        .append_event(line)
        .map_err(io::Error::from)?;
    if target_dir == caller_dir {
        return Ok(());
    }
    store::open(&target_dir)
        .append_event(line)
        .map_err(io::Error::from)
}

/// One participant's directory under the helper's sessions root.
fn participant_dir(dir: &Path, session: &str) -> io::Result<std::path::PathBuf> {
    if !crate::session_launch::name::is_session_name(session) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid participant session '{session}'"),
        ));
    }
    let Some(sessions) = dir.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "session directory has no parent",
        ));
    };
    Ok(sessions.join(session))
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
    /// The physical caller's tmux session, or empty for a pane-less actor.
    pub session: String,
}

/// The public `send` helper: the no-identity fallback, which records its own
/// event.
const SEND_HELPER: &str = "send";

/// The physical caller's session, or the helper session for a pane-less actor.
fn sender_session<'a>(sender: &'a Sender, own_session: &'a str) -> &'a str {
    if sender.session.is_empty() {
        own_session
    } else {
        &sender.session
    }
}

/// Resolve and admit one tracked pane route before any delivery is attempted.
fn admitted_route(
    kind: Kind,
    dir: &Path,
    parsed: &Parsed,
    sender: &Sender,
    own_session: &str,
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<Result<(Resolved, ServerId, bool), u8>> {
    let (resolved, server) = match resolve_on(&parsed.target, own_session, dir) {
        Ok(resolved) => resolved,
        Err(why) => {
            writeln!(err, "{}", why.message())?;
            return Ok(Err(EXIT_FAILED));
        }
    };
    let caller_session = sender_session(sender, own_session);
    if refuse_cross_session(
        dir,
        kind.action(),
        parsed.cross_session,
        &parsed.target,
        &resolved,
        &sender.display,
        &sender.slot,
        caller_session,
        now,
        err,
    )? {
        return Ok(Err(EXIT_FAILED));
    }
    let cross_session = resolved.session != caller_session;
    Ok(Ok((resolved, server, cross_session)))
}

/// Run a tracked request end to end.
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
    defer: std::time::Duration,
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
        // The fallback: a plain send, which writes its own event.
        let helper = dir.join(SEND_HELPER);
        write!(err, "{NO_IDENTITY_WARNING}")?;
        let delivery = transport::deliver(
            &helper,
            &parsed.target,
            &parsed.body,
            parsed.cross_session,
            &[],
        );
        out.write_all(delivery.stdout.as_bytes())?;
        return delivery_code(&delivery, &helper, action, err);
    };
    let caller_session = sender_session(sender, own_session);
    let req_id = request_id(kind.id_prefix(), now, entropy);
    if is_external(&parsed.target) {
        // An event-only sink: emit and exit, pasting nothing and storing
        // nothing.
        let line = event_line(&EventFields::new(
            now,
            &sender.display,
            action,
            &parsed.target,
            &req_id,
            &sender.slot,
            caller_session,
            "",
            "",
            &parsed.body,
            "",
        ));
        if let Err(why) = store::open(dir).append_event(&line) {
            writeln!(err, "ae: {action} {req_id} not recorded: {why}")?;
            return Ok(EXIT_FAILED);
        }
        return Ok(0);
    }
    let (resolved, server, cross_session) =
        match admitted_route(kind, dir, &parsed, sender, own_session, now, err)? {
            Ok(route) => route,
            Err(code) => return Ok(code),
        };
    let target_name = if resolved.agent.is_empty() {
        parsed.target.clone()
    } else {
        resolved.agent.clone()
    };
    let reply_cmd = reply_command(dir, &target_name, &req_id, kind.reply_label());
    let message = compose(kind, &req_id, &sender.display, &parsed.body, &reply_cmd);
    // The action and ref name the recovery file the body store writes; the
    // envelope names the same VERIFIED sender the composed message and the
    // event do, so a request cannot be framed as coming from someone else.
    let request = crate::deliver::Request {
        dir,
        server: &server,
        pane: &resolved.pane,
        logged_target: &target_name,
        target_session: &resolved.session,
        pane_slot: &resolved.slot,
        own_session,
        action,
        reference: &req_id,
        actor: &sender.display,
        body: &message,
        shape: crate::deliver::Shape::Send,
        defer,
    };
    let meta_dir = target_meta_dir(dir, &resolved.session, own_session);
    let (delivery, outcome) = across_cut(&server, &resolved.pane, &meta_dir, || {
        deliver_request(&request, err)
    });
    let delivery = delivery?;
    let fields = stamp_target(
        &EventFields::new(
            now,
            &sender.display,
            action,
            &target_name,
            &req_id,
            &sender.slot,
            caller_session,
            &resolved.slot,
            &resolved.session,
            &parsed.body,
            "",
        ),
        &outcome,
        err,
    );
    let cross = cross_session.then_some(CrossSession {
        caller: caller_session,
        target: &resolved.session,
    });
    record_tracked_delivery(dir, &fields, delivery, cross, err)
}

/// Record the event for a tracked delivery, including a delivery whose submit
/// was not confirmed after its body was pasted. For replies, this intentionally
/// closes the request on an unconfirmed paste because leaving it pending invites
/// a duplicate literal retry; the marker and notice preserve the uncertainty.
pub(crate) fn record_tracked_delivery(
    dir: &Path,
    fields: &EventFields<'_>,
    delivery: Result<crate::deliver::Delivered, crate::deliver::Failure>,
    cross_session: Option<CrossSession<'_>>,
    err: &mut impl Write,
) -> io::Result<u8> {
    let action = fields.action;
    let req_id = fields.reference;
    let target_name = fields.target;
    let (body_file, verification, unconfirmed) = match delivery {
        Ok(delivered) => (delivered.body_file, delivered.verification, false),
        Err(crate::deliver::Failure::Unconfirmed {
            body_file,
            notice: false,
            ..
        }) => (
            body_file,
            crate::deliver::DeliveryVerification::Verified,
            true,
        ),
        Err(_) => {
            // Every other refused delivery has already said what happened and
            // where the body is; nothing is recorded for one.
            return Ok(EXIT_FAILED);
        }
    };
    let fields = EventFields {
        body_file: &body_file,
        ..*fields
    };
    let line = if unconfirmed && cross_session.is_some() {
        cross_session_unconfirmed_event_line(&fields)
    } else if unconfirmed {
        unconfirmed_event_line(&fields)
    } else {
        delivery_event_line(&fields, verification, cross_session.is_some())
    };
    if let Err(why) = append_delivery_event(dir, &line, cross_session) {
        if unconfirmed {
            writeln!(
                err,
                "ae: {action} {req_id} was pasted to {target_name} but its event was not emitted: {why}"
            )?;
        } else {
            writeln!(
                err,
                "ae: {action} {req_id} was delivered to {target_name} but its event was not emitted: {why}"
            )?;
        }
        return Ok(EXIT_FAILED);
    }
    if unconfirmed {
        writeln!(
            err,
            "ae: {action} {req_id} recorded as pending; re-send only if peek shows the body still in the input box."
        )?;
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
        CorrelationGap, CorrelationOutcome, CrossSession, EventFields, IdentityTriple, Kind,
        Lookup, Parsed, ResolveError, Resolved, Sender, Usage, across_cut_with,
        caller_matches_live, caller_matches_recorded_target, compose, correlate_uuid, event_line,
        is_blank, is_external, is_request_id, lookup, named_server, pane_server, parse, pick,
        record_tracked_delivery, refusal, reply_command, request_id, retain_correlation, run,
        triple_from_viewer,
    };
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    use crate::time::Timestamp;
    use crate::tmux::ObservedAgent;

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    /// A sessions root under the temp dir with one session subdir per `(name,
    /// meta)`, each carrying the given meta text.
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
        // A cross-session target enumerates on the TARGET's server, read from
        // the TARGET's own meta — the whole point of correction 1.
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
                cross_session: false,
                target: "cl:w".to_owned(),
                body: "two words".to_owned()
            })
        );
        assert_eq!(parse(&words(&["cl:w"])), Err(Usage));
        assert_eq!(parse(&[]), Err(Usage));
        assert_eq!(
            parse(&words(&["--cross-session", "@other:cl:w", "question"])),
            Ok(Parsed {
                cross_session: true,
                target: "@other:cl:w".to_owned(),
                body: "question".to_owned(),
            })
        );
        assert_eq!(parse(&words(&["--cross-session", "cl:w"])), Err(Usage));
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
    #[allow(
        clippy::disallowed_methods,
        reason = "the test writes and reads its isolated event ledger"
    )]
    fn an_unconfirmed_request_event_remains_replyable() {
        let dir =
            std::env::temp_dir().join(format!("ae-tracked-unconfirmed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the isolated state directory");
        let body_file = dir.join("messages/ae-20260827T071112Z-00000001.ask.body.txt");
        let id = "ae-20260827T071112Z-00000001";
        let fields = EventFields {
            ts: Timestamp::parse("2026-08-27T07:11:12Z").expect("the timestamp parses"),
            actor: "lead",
            action: "ask",
            target: "worker",
            reference: id,
            actor_slot: "main",
            actor_session: "session",
            target_slot: "worker.0",
            target_session: "session",
            target_server: "",
            target_pane: "",
            target_session_uuid: "",
            caller_server: "",
            caller_pane: "",
            caller_session_uuid: "",
            identity_gap: "",
            summary: "the question",
            body_file: "",
        };
        let mut err = Vec::new();
        let code = record_tracked_delivery(
            &dir,
            &fields,
            Err(crate::deliver::Failure::Unconfirmed {
                body_file: body_file.display().to_string(),
                framed: "framed".to_owned(),
                notice: false,
            }),
            None,
            &mut err,
        )
        .expect("the pending request event is recorded");
        assert_eq!(code, 0);
        assert_eq!(
            String::from_utf8(err).expect("the pending notice is utf-8"),
            "ae: ask ae-20260827T071112Z-00000001 recorded as pending; re-send only if peek shows the body still in the input box.\n"
        );

        let event = std::fs::read_to_string(dir.join("events.jsonl")).expect("the event ledger");
        assert!(event.contains("\"summary\":\"[unconfirmed] the question\""));
        assert!(event.contains(&format!("\"body_file\":\"{}\"", body_file.display())));
        let found = crate::reply::find(&dir, id).expect("reply resolves an unconfirmed request id");
        assert_eq!(found.id, id.as_bytes());
        assert_eq!(found.status, crate::requests::Status::Pending);
        assert_eq!(found.summary, b"[unconfirmed] the question");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test forces a target-ledger write failure in isolated state"
    )]
    fn a_failed_target_mirror_is_loud_and_keeps_the_caller_event() {
        let root = std::env::temp_dir().join(format!(
            "ae-tracked-cross-audit-failure-{}",
            std::process::id()
        ));
        let caller = root.join("caller");
        let target = root.join("target");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&caller).expect("caller session directory");
        std::fs::create_dir_all(target.join("events.jsonl"))
            .expect("a directory blocks the target event file");
        let fields = EventFields {
            ts: Timestamp::parse("2026-08-27T07:11:12Z").expect("the timestamp parses"),
            actor: "lead",
            action: "ask",
            target: "@target:worker",
            reference: "ae-1",
            actor_slot: "main",
            actor_session: "caller",
            target_slot: "worker.0",
            target_session: "target",
            target_server: "",
            target_pane: "",
            target_session_uuid: "",
            caller_server: "",
            caller_pane: "",
            caller_session_uuid: "",
            identity_gap: "",
            summary: "question",
            body_file: "",
        };
        let mut err = Vec::new();
        let code = record_tracked_delivery(
            &caller,
            &fields,
            Ok(crate::deliver::Delivered {
                body_file: "/messages/ae-1.ask.body.txt".to_owned(),
                framed: "framed".to_owned(),
                verification: crate::deliver::DeliveryVerification::Verified,
            }),
            Some(CrossSession {
                caller: "caller",
                target: "target",
            }),
            &mut err,
        )
        .expect("the diagnostic is writable");
        assert_eq!(code, crate::state::EXIT_FAILED);
        let caller_event =
            std::fs::read_to_string(caller.join("events.jsonl")).expect("the caller audit remains");
        assert!(caller_event.contains("\"cross_session\":true"));
        let diagnostic = String::from_utf8(err).expect("the diagnostic is utf-8");
        assert_eq!(diagnostic.lines().count(), 1, "{diagnostic}");
        assert!(
            diagnostic.starts_with(
                "ae: ask ae-1 was delivered to @target:worker but its event was not emitted:"
            ),
            "{diagnostic}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test writes and reads its isolated event ledger"
    )]
    fn a_notice_unconfirmed_delivery_remains_unrecorded() {
        let dir = std::env::temp_dir().join(format!(
            "ae-tracked-notice-unconfirmed-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the isolated state directory");
        let fields = EventFields {
            ts: Timestamp::parse("2026-08-27T07:11:12Z").expect("the timestamp parses"),
            actor: "lead",
            action: "ask",
            target: "worker",
            reference: "ae-20260827T071112Z-00000002",
            actor_slot: "main",
            actor_session: "session",
            target_slot: "worker.0",
            target_session: "session",
            target_server: "",
            target_pane: "",
            target_session_uuid: "",
            caller_server: "",
            caller_pane: "",
            caller_session_uuid: "",
            identity_gap: "",
            summary: "the question",
            body_file: "",
        };
        let mut err = Vec::new();
        let code = record_tracked_delivery(
            &dir,
            &fields,
            Err(crate::deliver::Failure::Unconfirmed {
                body_file: dir
                    .join("messages/ae-20260827T071112Z-00000002.ask.body.txt")
                    .display()
                    .to_string(),
                framed: "framed".to_owned(),
                notice: true,
            }),
            None,
            &mut err,
        )
        .expect("the notice failure is handled");
        assert_eq!(code, crate::state::EXIT_FAILED);
        assert!(!dir.join("events.jsonl").exists());
        assert!(err.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test writes and reads its isolated event ledger"
    )]
    fn a_non_unconfirmed_delivery_remains_unrecorded() {
        let dir = std::env::temp_dir().join(format!("ae-tracked-refused-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the isolated state directory");
        let fields = EventFields {
            ts: Timestamp::parse("2026-08-27T07:11:12Z").expect("the timestamp parses"),
            actor: "lead",
            action: "ask",
            target: "worker",
            reference: "ae-20260827T071112Z-00000003",
            actor_slot: "main",
            actor_session: "session",
            target_slot: "worker.0",
            target_session: "session",
            target_server: "",
            target_pane: "",
            target_session_uuid: "",
            caller_server: "",
            caller_pane: "",
            caller_session_uuid: "",
            identity_gap: "",
            summary: "the question",
            body_file: "",
        };
        let mut err = Vec::new();
        let code = record_tracked_delivery(
            &dir,
            &fields,
            Err(crate::deliver::Failure::Abandoned),
            None,
            &mut err,
        )
        .expect("the refused delivery is handled");
        assert_eq!(code, crate::state::EXIT_FAILED);
        assert!(!dir.join("events.jsonl").exists());
        assert!(err.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
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
        // IDENTITY V2: one colon, no `@`, is the same cross-session address.
        assert_eq!(
            lookup("s:lead", "s"),
            Ok(Lookup::Named {
                session: "s".to_owned(),
                target: "lead".to_owned(),
                explicit: true
            }),
            "the own session named without the @ is still checked with has-session"
        );
        assert_eq!(
            lookup("other:lead", "s"),
            Ok(Lookup::Named {
                session: "other".to_owned(),
                target: "lead".to_owned(),
                explicit: true
            })
        );
        assert_eq!(
            lookup("fable5:lead", "s"),
            Ok(Lookup::Named {
                session: "fable5".to_owned(),
                target: "lead".to_owned(),
                explicit: true
            }),
            "an alias-shaped target is a session name now, and fails as one"
        );
        // A half that is empty, or a second colon, is NOT a cross-session
        // address: it stays a plain name and fails as a plain name, rather than
        // being answered with a shape error about an `@` nobody typed.
        for plain in [":lead", "s:", "cl:x:y"] {
            assert_eq!(
                lookup(plain, "s"),
                Ok(Lookup::Named {
                    session: "s".to_owned(),
                    target: plain.to_owned(),
                    explicit: false
                }),
                "{plain}"
            );
        }
        assert_eq!(
            ResolveError::CrossSessionShape("@x".to_owned()).message(),
            "Error: cross-session target must be @session:agent, got '@x'"
        );
    }

    #[test]
    fn the_pick_is_exact_and_the_alias_and_bare_name_arms_are_retired() {
        // A v2 roster: every stamp is the bare NAME.
        let rows = roster(&[
            ("%1", "lead"),
            ("%2", "colead"),
            ("%3", "cl:legacy"),
            ("%4", ""),
            ("%5", "gx:legacy"),
        ]);
        assert_eq!(pick(&rows, "lead", "s", "s"), Ok(("%1", "lead".to_owned())));
        assert_eq!(
            pick(&rows, "cl:legacy", "s", "s"),
            Ok(("%3", "cl:legacy".to_owned())),
            "a legacy stamp still resolves by its whole self"
        );
        // THE RETIREMENT, stated as the failures it causes.
        for partial in ["cl", "legacy", "gx"] {
            assert_eq!(
                pick(&rows, partial, "s", "s"),
                Err(ResolveError::NotFound {
                    target: partial.to_owned(),
                    session: "s".to_owned()
                }),
                "{partial}: a partial match is a guess, not an address"
            );
        }
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
            pick(&rows, "colead", "other", "s"),
            Ok(("%2", "@other:colead".to_owned())),
            "the display ref is unchanged — accepted input widened, output did not move"
        );
        // AMBIGUITY IS NOW THE PANE LAYER'S ALONE.
        let twins = roster(&[("%1", "lead"), ("%2", "lead")]);
        assert_eq!(
            pick(&twins, "lead", "s", "s"),
            Err(ResolveError::Ambiguous {
                target: "lead".to_owned(),
                session: "s".to_owned()
            })
        );
        assert_eq!(
            ResolveError::Ambiguous {
                target: "lead".to_owned(),
                session: "s".to_owned()
            }
            .message(),
            "Error: ambiguous name 'lead' in session 's'",
            "the alias:name advice is gone with the arm that made it advice"
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

    #[test]
    fn is_request_id_accepts_exactly_what_the_minter_mints_for_production() {
        // R15 pin, accept direction: every Kind, every canonical four-digit-
        // year Timestamp sampled, every entropy shape — minted with a
        // production prefix, accepted, and within 32 chars.
        let stamps = [
            "0000-01-01T00:00:00Z",
            "0001-12-31T23:59:59Z",
            "1969-12-31T23:59:59Z",
            "1970-01-01T00:00:00Z",
            "2000-02-29T12:00:00Z",
            "2026-08-27T07:11:12Z",
            "2026-12-31T23:59:59Z",
            "9999-12-31T23:59:59Z",
        ];
        let entropies = [0, 1, 0xffff_ffff, u64::MAX, 0x1234_5678_9abc_def0];
        let check = |at: Timestamp| {
            for kind in [Kind::Ask, Kind::Review] {
                for entropy in entropies {
                    let minted = request_id(kind.id_prefix(), at, entropy);
                    assert!(is_request_id(&minted), "minted {minted:?} must validate");
                    assert!(minted.len() <= 32, "minted {minted:?} over 32 chars");
                }
            }
        };
        for stamp in stamps {
            check(Timestamp::parse(stamp).unwrap());
        }
        // A sweep across one mid-range day's seconds keeps the minute/second
        // rendering honest without enumerating the calendar; offsets stay
        // in-day so no sweep crosses into a five-digit year.
        let mid = Timestamp::parse("2026-06-15T00:00:00Z").unwrap();
        for second in [0, 1, 3600, 61_200, 86_399] {
            check(Timestamp::from_epoch(mid.epoch() + second));
        }
    }

    #[test]
    fn is_request_id_rejects_what_the_minter_can_make_but_must_not_validate() {
        // R15 pin, reject direction. Each of these is producible by the
        // public minter (any prefix, any Timestamp) or one char past the
        // grammar — the validator must refuse them all.
        let five_digit = request_id("ae", Timestamp::from_epoch(253_402_300_800), 7);
        assert_eq!(five_digit, "ae-100000101T000000Z-00000007");
        let cases: Vec<String> = [
            "ae-x",
            "review-",
            "",
            "ae-20260827T071112Z-0000000G",
            "ae-20260827T071112Z-ABCDEF01",
            "AE-20260827T071112Z-abcdef01",
            "ae-20260827T071112Z-0000007",
            "ae-20260827T071112Z-000000007",
            "ae-2026082T071112Z-00000007",
            "ae-20260827T07111Z-00000007",
            "ae-20260827T071112Z00000007",
            "ae20260827T071112Z-00000007",
            "ae-20260827T071112Z-00000007\n",
            " ae-20260827T071112Z-00000007",
            "xx-20260827T071112Z-00000007",
            "ask-20260827T071112Z-00000007",
            "reviewx-20260827T071112Z-00000007",
            "review-20260827T071112Z-00000007x",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .chain([five_digit])
        .collect();
        assert_eq!(cases[17].len(), 33, "the 33rd-char case");
        for bad in &cases {
            assert!(!is_request_id(bad), "rejected {bad:?} must not validate");
        }
    }

    const UUID: &str = "1b4e28ba-2fa1-11d2-883f-0016d3cc4321";
    const UUID_B: &str = "550e8400-e29b-41d4-a716-446655440000";

    fn triple(server: &str, pane: &str, uuid: &str) -> IdentityTriple {
        IdentityTriple {
            server: server.to_owned(),
            pane: pane.to_owned(),
            session_uuid: uuid.to_owned(),
        }
    }

    #[test]
    fn unreadable_vacant_and_mismatch_each_refuse_and_never_collapse() {
        use crate::tmux::OptionReading;
        let meta = format!("session_id={UUID}\n");
        assert_eq!(
            correlate_uuid(&OptionReading::Unknown, meta.as_bytes()),
            Err(CorrelationGap::Unreadable)
        );
        assert_eq!(
            correlate_uuid(&OptionReading::Vacant, meta.as_bytes()),
            Err(CorrelationGap::Vacant)
        );
        assert_eq!(
            correlate_uuid(&OptionReading::Vacant, b"session=s\n"),
            Err(CorrelationGap::NoSession),
            "no identity in the pane and none in the meta: nothing to correlate"
        );
        assert_eq!(
            correlate_uuid(&OptionReading::Vacant, b"session_id=\n"),
            Err(CorrelationGap::NoSession),
            "an empty identity row records nothing"
        );
        assert_eq!(
            correlate_uuid(
                &OptionReading::Set("not-a-uuid".to_owned()),
                meta.as_bytes()
            ),
            Err(CorrelationGap::Invalid)
        );
        assert_eq!(
            correlate_uuid(&OptionReading::Set(UUID_B.to_owned()), meta.as_bytes()),
            Err(CorrelationGap::Mismatch)
        );
        assert_eq!(
            correlate_uuid(&OptionReading::Set(UUID.to_owned()), meta.as_bytes()),
            Ok(UUID.to_owned())
        );
        assert_ne!(CorrelationGap::NoSession, CorrelationGap::Vacant);
        assert_ne!(CorrelationGap::NoSession, CorrelationGap::Unreadable);
        assert_ne!(CorrelationGap::Unreadable, CorrelationGap::Vacant);
        assert_ne!(CorrelationGap::Vacant, CorrelationGap::Mismatch);
        assert_ne!(CorrelationGap::Unreadable, CorrelationGap::Mismatch);
    }

    #[test]
    fn a_duplicate_or_missing_meta_identity_is_a_named_gap() {
        use crate::tmux::OptionReading;
        let option = OptionReading::Set(UUID.to_owned());
        assert_eq!(
            correlate_uuid(&option, b"session=s\n"),
            Err(CorrelationGap::MetaEmpty)
        );
        assert_eq!(
            correlate_uuid(
                &option,
                format!("session_id={UUID}\nsession_id={UUID}\n").as_bytes()
            ),
            Err(CorrelationGap::MetaDuplicate)
        );
        assert_eq!(
            correlate_uuid(&option, b"session_id=\n"),
            Err(CorrelationGap::MetaEmpty)
        );
        assert_eq!(
            correlate_uuid(&option, b"session_id=not-a-uuid\n"),
            Err(CorrelationGap::MetaMalformed)
        );
    }

    #[test]
    fn an_empty_triple_never_matches_and_a_cut_change_drops_correlation() {
        let a = triple("/tmp/ae", "%1", UUID);
        let b = triple("/tmp/ae", "%1", UUID);
        let other_pane = triple("/tmp/ae", "%2", UUID);
        let empty = triple("", "%1", UUID);
        assert!(a.same_incarnation(&b));
        assert!(!a.same_incarnation(&other_pane));
        assert!(
            !empty.same_incarnation(&a),
            "empty server is not an identity"
        );
        assert!(
            !empty.same_incarnation(&empty),
            "empty vs empty is not an identity"
        );
        assert!(caller_matches_live(&a, &b));
        assert!(!caller_matches_live(&a, &other_pane));
        assert!(caller_matches_recorded_target(&a, &b));
        assert!(!caller_matches_recorded_target(&a, &other_pane));
        // The two questions take different counterparts. Same caller can match
        // LIVE and fail the RECORDED target: a pane that moved since open.
        let live = triple("/tmp/ae", "%1", UUID);
        let recorded_at_open = triple("/tmp/ae", "%2", UUID);
        assert!(
            caller_matches_live(&live, &live),
            "(a) the record is from the incarnation that is here NOW"
        );
        assert!(
            !caller_matches_recorded_target(&live, &recorded_at_open),
            "(b) the responder is not the incarnation the request opened against"
        );
        assert_eq!(
            retain_correlation(&Ok(a.clone()), &Ok(b.clone())),
            CorrelationOutcome::Correlated(a.clone())
        );
        assert_eq!(
            retain_correlation(&Ok(a.clone()), &Ok(other_pane)),
            CorrelationOutcome::Changed
        );
        assert_eq!(
            retain_correlation(&Ok(a.clone()), &Err(CorrelationGap::Unreadable)),
            CorrelationOutcome::Failed(CorrelationGap::Unreadable)
        );
        assert_eq!(
            retain_correlation(&Err(CorrelationGap::Vacant), &Ok(a)),
            CorrelationOutcome::Failed(CorrelationGap::Vacant)
        );
        assert_eq!(
            retain_correlation(
                &Err(CorrelationGap::NoSession),
                &Err(CorrelationGap::NoSession)
            ),
            CorrelationOutcome::NoSession,
            "no identity on either side of the cut: no question, no gap"
        );
        assert_eq!(
            retain_correlation(&Ok(b.clone()), &Err(CorrelationGap::NoSession)),
            CorrelationOutcome::Failed(CorrelationGap::NoSession),
            "an identity existed before the cut; a one-sided absence still fails"
        );
        assert_eq!(
            CorrelationOutcome::from_observation(Err(CorrelationGap::NoSession)),
            CorrelationOutcome::NoSession
        );
        assert_eq!(
            CorrelationOutcome::NoSession.name(),
            None,
            "the writer names nothing when there was nothing to correlate"
        );
        assert_eq!(CorrelationOutcome::NoSession.triple(), None);
        assert_eq!(
            CorrelationOutcome::Failed(CorrelationGap::Vacant).name(),
            Some("session identity not recorded")
        );
        assert_eq!(
            CorrelationOutcome::Changed.name(),
            Some("session identity changed")
        );
    }

    #[test]
    fn event_line_writes_identity_facts_only_when_they_are_nonempty() {
        let ts = Timestamp::parse("2026-08-27T07:11:12Z").expect("ts");
        let without = event_line(&EventFields::new(
            ts, "lead", "ask", "w", "ae-1", "main", "s", "worker.0", "s", "q", "",
        ));
        assert!(
            !without.contains("target_server"),
            "empty identity facts stay unwritten so older readers see the old shape"
        );
        let with = event_line(
            &EventFields::new(
                ts, "lead", "ask", "w", "ae-1", "main", "s", "worker.0", "s", "q", "",
            )
            .with_target(Some(&triple("/tmp/ae", "%1", UUID))),
        );
        assert!(with.contains(r#""target_server":"/tmp/ae""#));
        assert!(with.contains(r#""target_pane":"%1""#));
        assert!(with.contains(&format!(r#""target_session_uuid":"{UUID}""#)));
        let reply = event_line(
            &EventFields::new(
                ts, "w", "reply", "lead", "ae-1", "worker.0", "s", "main", "s", "a", "",
            )
            .with_caller(Some(&triple("/tmp/ae", "%1", UUID))),
        );
        assert!(reply.contains(r#""caller_server":"/tmp/ae""#));
        assert!(reply.contains(r#""caller_pane":"%1""#));
    }

    #[test]
    fn a_no_session_outcome_writes_no_gap_key_and_no_stderr() {
        let ts = Timestamp::parse("2026-08-27T07:11:12Z").expect("ts");
        let mut err = Vec::new();
        let memo = super::stamp_caller(
            &EventFields::new(
                ts, "human", "memo", "", "p2", "", "", "", "", "one line", "",
            ),
            &CorrelationOutcome::NoSession,
            &mut err,
        );
        let line = event_line(&memo);
        assert!(
            !line.contains("identity_gap"),
            "an absent session context earns no gap: {line}"
        );
        let ask = super::stamp_target(
            &EventFields::new(
                ts, "lead", "ask", "w", "ae-1", "main", "s", "worker.0", "s", "q", "",
            ),
            &CorrelationOutcome::NoSession,
            &mut err,
        );
        assert!(!event_line(&ask).contains("identity_gap"));
        assert!(
            err.is_empty(),
            "NoSession is not broadcast either: {}",
            String::from_utf8_lossy(&err)
        );
    }

    fn viewer_at(socket: &str, uuid: &str) -> crate::tmux::ObservedViewer {
        crate::tmux::ObservedViewer {
            slot: Some("main".to_owned()),
            session: Some("s".to_owned()),
            agent: Some("lead".to_owned()),
            session_uuid: crate::tmux::OptionReading::Set(uuid.to_owned()),
            socket_path: Some(socket.to_owned()),
        }
    }

    fn meta_dir(tag: &str, body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ae-idmeta.{}.{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("meta dir");
        if !body.is_empty() {
            std::fs::write(dir.join("meta"), body).expect("meta");
        }
        dir
    }

    #[test]
    fn the_server_fact_is_the_viewer_socket_path_not_a_selector_spelling() {
        let dir = meta_dir("sock", &format!("session_id={UUID}\n"));
        let via_alias = triple_from_viewer(&viewer_at("/tmp/alias", UUID), "%1", &dir)
            .expect("alias observation correlates");
        let via_real = triple_from_viewer(&viewer_at("/tmp/real", UUID), "%1", &dir)
            .expect("real observation correlates");
        assert_eq!(via_alias.server, "/tmp/alias");
        assert_eq!(via_real.server, "/tmp/real");
        assert_ne!(
            via_alias.server, via_real.server,
            "replacement or alias spelling cannot be collapsed by a second query"
        );
        let mut empty = viewer_at("/tmp/real", UUID);
        empty.socket_path = None;
        assert_eq!(
            triple_from_viewer(&empty, "%1", &dir),
            Err(CorrelationGap::Unreadable)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn classified_meta_refuses_nonregular_missing_empty_and_malformed() {
        use crate::tmux::OptionReading;
        let option = OptionReading::Set(UUID.to_owned());
        let missing = meta_dir("miss", "");
        std::fs::remove_file(missing.join("meta")).ok();
        assert_eq!(
            super::correlate_uuid_in(&option, &missing),
            Err(CorrelationGap::MetaMissing)
        );
        let dir = meta_dir("dir", "x");
        std::fs::remove_file(dir.join("meta")).unwrap();
        std::fs::create_dir(dir.join("meta")).unwrap();
        assert_eq!(
            super::correlate_uuid_in(&option, &dir),
            Err(CorrelationGap::MetaNonregular)
        );
        std::fs::remove_dir(dir.join("meta")).unwrap();
        let target = dir.join("elsewhere");
        std::fs::write(&target, format!("session_id={UUID}\n")).unwrap();
        std::os::unix::fs::symlink(&target, dir.join("meta")).unwrap();
        assert_eq!(
            super::correlate_uuid_in(&option, &dir),
            Err(CorrelationGap::MetaNonregular),
            "a symlink is never followed"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&missing);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test reads back the event ledger it just wrote"
    )]
    fn ask_event_bytes_across_a_durable_cut_are_stamped_from_the_outcome() {
        let ts = Timestamp::parse("2026-08-27T07:11:12Z").expect("ts");
        let held = triple("/tmp/ae", "%9", UUID);
        let mut n = 0;
        let observe = || {
            n += 1;
            Ok(held.clone())
        };
        let (cut, outcome) = across_cut_with(observe, || "delivered");
        assert_eq!(cut, "delivered");
        let dir = meta_dir("askcut", &format!("session_id={UUID}\n"));
        let mut err = Vec::new();
        let fields = super::stamp_target(
            &EventFields::new(
                ts, "lead", "ask", "w", "ae-1", "main", "s", "worker.0", "s", "q", "",
            ),
            &outcome,
            &mut err,
        );
        super::record_tracked_delivery(
            &dir,
            &fields,
            Ok(crate::deliver::Delivered {
                body_file: String::new(),
                framed: "q".to_owned(),
                verification: crate::deliver::DeliveryVerification::Verified,
            }),
            None,
            &mut err,
        )
        .expect("ask event writes");
        let events = std::fs::read_to_string(dir.join("events.jsonl")).expect("events");
        assert!(events.contains(r#""action":"ask""#));
        assert!(events.contains(r#""target_server":"/tmp/ae""#));
        assert!(events.contains(r#""target_pane":"%9""#));
        assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
        let mut n = 0;
        let observe = || {
            n += 1;
            if n == 1 {
                Ok(held.clone())
            } else {
                Ok(triple("/tmp/ae", "%8", UUID))
            }
        };
        let (_, changed) = across_cut_with(observe, || "delivered");
        let mut err = Vec::new();
        let omitted = event_line(&super::stamp_target(
            &EventFields::new(
                ts, "lead", "ask", "w", "ae-1", "main", "s", "worker.0", "s", "q", "",
            ),
            &changed,
            &mut err,
        ));
        assert!(
            !omitted.contains("target_server"),
            "a changed identity writes no correlated opening"
        );
        assert!(
            String::from_utf8_lossy(&err).contains("session identity changed"),
            "the writer names the failed leg: {}",
            String::from_utf8_lossy(&err)
        );
        let mut err = Vec::new();
        let reply_fields = super::stamp_caller(
            &EventFields::new(
                ts, "w", "reply", "lead", "ae-1", "worker.0", "s", "main", "s", "a", "",
            ),
            &outcome,
            &mut err,
        );
        std::fs::write(dir.join("events.jsonl"), b"").unwrap();
        super::record_tracked_delivery(
            &dir,
            &reply_fields,
            Ok(crate::deliver::Delivered {
                body_file: String::new(),
                framed: "a".to_owned(),
                verification: crate::deliver::DeliveryVerification::Verified,
            }),
            None,
            &mut err,
        )
        .expect("reply event writes");
        let events = std::fs::read_to_string(dir.join("events.jsonl")).expect("events");
        assert!(events.contains(r#""action":"reply""#));
        assert!(events.contains(r#""caller_server":"/tmp/ae""#));
        assert!(events.contains(r#""caller_pane":"%9""#));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test reads back the event ledger the production run wrote"
    )]
    fn ask_run_stamps_target_on_the_production_writer() {
        super::clear_test_hooks();
        let dir = meta_dir("askrun", &format!("session_id={UUID}\n"));
        let ts = Timestamp::parse("2026-08-27T07:11:12Z").expect("ts");
        let held = triple("/tmp/ae", "%9", UUID);
        super::queue_observe(Ok(held.clone()));
        super::queue_observe(Ok(held.clone()));
        super::set_test_resolve(
            Resolved {
                pane: "%9".to_owned(),
                agent: "w".to_owned(),
                slot: "worker.0".to_owned(),
                session: "s".to_owned(),
            },
            ServerId::Ambient,
        );
        super::set_test_delivery(Ok(crate::deliver::Delivered {
            body_file: String::new(),
            framed: "q".to_owned(),
            verification: crate::deliver::DeliveryVerification::Verified,
        }));
        let sender = Sender {
            display: "lead".to_owned(),
            slot: "main".to_owned(),
            session: "s".to_owned(),
        };
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(
            Kind::Ask,
            &dir,
            &["w".to_owned(), "q".to_owned()],
            Some(&sender),
            "s",
            ts,
            1,
            crate::deliver::DEFAULT_DEFER,
            &mut out,
            &mut err,
        )
        .expect("ask run");
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
        let events = std::fs::read_to_string(dir.join("events.jsonl")).expect("events");
        assert!(events.contains(r#""action":"ask""#), "{events}");
        assert!(
            events.contains(r#""target_server":"/tmp/ae""#),
            "deleting stamp_target from run() must drop this: {events}"
        );
        assert!(events.contains(r#""target_pane":"%9""#), "{events}");
        assert!(
            !events.contains("identity_gap"),
            "a correlated opening must not name a gap: {events}"
        );
        super::clear_test_hooks();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test reads back the event ledger the production run wrote"
    )]
    fn ask_run_records_a_failed_leg_and_does_not_broadcast_it() {
        super::clear_test_hooks();
        let dir = meta_dir("askgap", &format!("session_id={UUID}\n"));
        let ts = Timestamp::parse("2026-08-27T07:11:12Z").expect("ts");
        super::queue_observe(Err(CorrelationGap::Unreadable));
        super::queue_observe(Err(CorrelationGap::Unreadable));
        super::set_test_resolve(
            Resolved {
                pane: "%9".to_owned(),
                agent: "w".to_owned(),
                slot: "worker.0".to_owned(),
                session: "s".to_owned(),
            },
            ServerId::Ambient,
        );
        super::set_test_delivery(Ok(crate::deliver::Delivered {
            body_file: String::new(),
            framed: "q".to_owned(),
            verification: crate::deliver::DeliveryVerification::Verified,
        }));
        let sender = Sender {
            display: "lead".to_owned(),
            slot: "main".to_owned(),
            session: "s".to_owned(),
        };
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(
            Kind::Ask,
            &dir,
            &["w".to_owned(), "q".to_owned()],
            Some(&sender),
            "s",
            ts,
            2,
            crate::deliver::DEFAULT_DEFER,
            &mut out,
            &mut err,
        )
        .expect("ask run failed-gap");
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
        assert!(
            err.is_empty(),
            "Failed is recorded, not broadcast: {}",
            String::from_utf8_lossy(&err)
        );
        let events = std::fs::read_to_string(dir.join("events.jsonl")).expect("events");
        assert!(
            events.contains(r#""identity_gap":"session identity unreadable""#),
            "Failed must name the leg in the record: {events}"
        );
        assert!(
            !events.contains("target_server"),
            "Failed omits the triple: {events}"
        );
        super::clear_test_hooks();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
