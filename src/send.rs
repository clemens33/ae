//! The public `send` helper (P2.6): the chokepoint every other helper and
//! several `ae`-internal paths exec into, composed, DELIVERED and recorded by
//! the core — the paste included, since B move 1 ([`crate::deliver`]).
//!
//! What the frozen `helper_send_body` does around the paste, kept exactly.
//! `send <target> <message…>`: fewer than two words is usage; a blank message
//! is refused. The EVENT's fields come from the caller's environment where
//! the frozen emitter read them, and nowhere else — this is the contract the
//! bash `ask`/`review`/`reply` fallback bodies, `ae cancel`, the watchdog's
//! `nudge` and the telegram bridge all exec `send` under, ruled preserved
//! (colead, 2026-08-27): `AE_SENDER_OVERRIDE` names the actor (the explicit,
//! never ambient, door for callers with no pane); `_AE_EVENT_ACTION` (else
//! `send`), `_AE_EVENT_REF` (else the `[ae-…]`/`[review-…]` id found in the
//! text, else none), `_AE_EVENT_SUMMARY` (else the text) and the four routing
//! members `_AE_EVENT_ACTOR_SLOT`/`_ACTOR_SESSION`/`_TARGET_SLOT`/
//! `_TARGET_SESSION`, each present in the event only when non-empty. With no
//! override the actor is the pane's verified stamp, and `human` when there is
//! no pane. No NEW authority exists: nothing on the argv names an actor, and
//! nothing in the environment selects how the body finishes.
//!
//! The default summary is of what was DELIVERED, not of what was typed: the
//! frozen body reassigns `msg` to the framed text — provenance envelope,
//! newline, message — before its finisher runs, so the envelope leads a pane
//! send's summary and counts against its cap, while a sink's is recorded
//! before framing and stays bare. The framed text now comes back from the
//! delivery in hand rather than being read out of the record it wrote. See
//! [`delivered_summary`]. The text is
//! handed raw to [`crate::tracked::event_line`], which renders it for the
//! action as the frozen emitter's two arms do — flattened and capped at 200,
//! or, under `_AE_EVENT_ACTION=chat`, lines and tabs kept under the 3500 cap.
//!
//! An external sink (`telegram:*`, `discord:*`, `ae:compact:*`) is event-only.
//! Any other target is resolved as `ae_resolve` resolves it and delivered by
//! [`crate::deliver`] — dead-pane guard, provenance envelope (which takes the
//! same `AE_SENDER_OVERRIDE` when it is set), body store, per-target lock,
//! busy deferral, submit verification — which prints every loud line itself
//! and records NOTHING. Only a confirmed delivery is followed by the ONE
//! event, under [`crate::state::emit`]'s locked, synced transaction; an event
//! that could not be written after a confirmed delivery is reported as
//! exactly that gap and exits non-zero — and because the shim in the helper
//! `exec`s the core, there is no bash body left to re-deliver.
use std::io::{self, Write};
use std::path::Path;

use crate::deliver;
use crate::state::{self, EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;
use crate::tracked::{self, EventFields};

/// The frozen usage text.
pub const USAGE: &str = "Usage: send <agent-name|pane-id|@session:agent> <message>\n  Examples: send claude:lead \"hello\"\n           send @my-feature:claude:lead \"hello\"\n";

/// The default action.
pub const ACTION: &str = "send";

/// What the argv said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// The target as typed.
    pub target: String,
    /// The message: the remaining words joined by one space (`"$*"`).
    pub message: String,
}

/// A refused argv: [`USAGE`] to stderr, the usage exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage;

/// Parse the argv after the meta directory: fewer than two words is usage.
///
/// # Errors
///
/// [`Usage`].
pub fn parse(tail: &[String]) -> Result<Parsed, Usage> {
    match tail {
        [target, message @ ..] if !message.is_empty() => Ok(Parsed {
            target: target.clone(),
            message: message.join(" "),
        }),
        _ => Err(Usage),
    }
}

/// The frozen event-field contract, as read off the caller's environment.
/// Every member is what the frozen `${VAR:-}` reads: an unset or EMPTY
/// variable is `None`/empty. Built by the one door in `lib.rs`; tests build
/// it directly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    /// `AE_SENDER_OVERRIDE`.
    pub sender_override: Option<String>,
    /// `_AE_EVENT_ACTION`.
    pub action: Option<String>,
    /// `_AE_EVENT_REF`.
    pub reference: Option<String>,
    /// `_AE_EVENT_SUMMARY`.
    pub summary: Option<String>,
    /// `_AE_EVENT_ACTOR_SLOT`, or empty.
    pub actor_slot: String,
    /// `_AE_EVENT_ACTOR_SESSION`, or empty.
    pub actor_session: String,
    /// `_AE_EVENT_TARGET_SLOT`, or empty.
    pub target_slot: String,
    /// `_AE_EVENT_TARGET_SESSION`, or empty.
    pub target_session: String,
}

/// The frozen `ae_extract_req_id`: the first `[ae-…]` or `[review-…]`
/// bracket in `text` whose id is one or more of `A-Za-z0-9._:-`, or `None`.
///
/// ```
/// use ae::send::extract_req_id;
///
/// assert_eq!(extract_req_id("[ae-20260827T1Z-abcd] done").as_deref(), Some("ae-20260827T1Z-abcd"));
/// assert_eq!(extract_req_id("re [review-x.y:z] and [ae-2]").as_deref(), Some("review-x.y:z"));
/// assert_eq!(extract_req_id("[aex-1] [ae-] [ae-1"), None);
/// ```
#[must_use]
pub fn extract_req_id(text: &str) -> Option<String> {
    let is_id_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-');
    for (start, _) in text.match_indices('[') {
        let after = &text[start + 1..];
        let Some(rest) = after
            .strip_prefix("ae-")
            .or_else(|| after.strip_prefix("review-"))
        else {
            continue;
        };
        let run = rest.chars().take_while(|c| is_id_char(*c)).count();
        let id_end = rest
            .char_indices()
            .nth(run)
            .map_or(rest.len(), |(index, _)| index);
        if run > 0 && rest[id_end..].starts_with(']') {
            let prefix_len = after.len() - rest.len();
            return Some(after[..prefix_len + id_end].to_owned());
        }
    }
    None
}

/// The frozen emitter's actor: the explicit override, else the pane's
/// display ref, else `human`.
#[must_use]
pub fn actor(env: &Env, display: &str) -> String {
    match env.sender_override.as_deref() {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ if display.is_empty() => "human".to_owned(),
        _ => display.to_owned(),
    }
}

/// Who the provenance ENVELOPE names — the explicit override, else the pane's
/// verified stamp, else nobody.
///
/// Deliberately NOT [`actor`], whose `human` fallback belongs to the EVENT.
/// The envelope's blank is `unverified`, because bare is the human's signature:
/// they type raw and never mark anything, so a helper-delivered message that
/// could not bind its caller must be marked unverified rather than promoted to
/// look like the human. That asymmetry is the whole authority model.
///
/// ```
/// use ae::send::{Env, envelope_sender};
///
/// let bridge = Env { sender_override: Some("telegram".into()), ..Env::default() };
/// assert_eq!(envelope_sender(&bridge, "cl:lead"), "telegram");
/// assert_eq!(envelope_sender(&Env::default(), "cl:lead"), "cl:lead");
/// assert_eq!(envelope_sender(&Env::default(), ""), "", "no claim to make");
/// ```
#[must_use]
pub fn envelope_sender<'a>(env: &'a Env, display: &'a str) -> &'a str {
    match env.sender_override.as_deref() {
        Some(name) if !name.is_empty() => name,
        _ => display,
    }
}

/// The event's action, ref and summary, resolved from the environment and
/// the message the way the frozen body resolves them BEFORE the paste — the
/// shape a sink records. A pane send re-derives its summary from the delivery
/// with [`delivered_summary`]. The summary is the raw text: the emitter
/// renders it for the action.
#[must_use]
pub fn fields(env: &Env, message: &str) -> (String, String, String) {
    let action = env
        .action
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ACTION.to_owned());
    let reference = env
        .reference
        .clone()
        .filter(|value| !value.is_empty())
        .or_else(|| extract_req_id(message))
        .unwrap_or_default();
    let summary = env
        .summary
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| message.to_owned());
    (action, reference, summary)
}

/// The summary of a delivered pane send, raw: an explicit `_AE_EVENT_SUMMARY`
/// as given, else the text the entry pasted, for [`crate::tracked::event_line`]
/// to render for the action. The frozen body summarises `msg` AFTER
/// reassigning it to the framed text, so the provenance envelope leads the
/// summary (`⟦ae:msg from …⟧`, a newline, the text) and counts against the
/// cap; `ask`, `review`, `reply` and `cancel` are exact without this only
/// because each passes `_AE_EVENT_SUMMARY`. The core does
/// not re-derive the provenance line — the recovery record the entry just
/// wrote IS that text byte for byte, and is read back through the one gated
/// read door, [`crate::event_text::read_container`] (a record is never empty,
/// so empty means unreadable). Without a readable record (the entry stores
/// nothing when the messages directory cannot be written), the summary is of
/// the message as typed and stderr says so: a summary is not worth losing
/// the record of a delivered message over.
fn delivered_summary(env: &Env, framed: &str) -> String {
    match env.summary.as_deref().filter(|value| !value.is_empty()) {
        Some(explicit) => explicit.to_owned(),
        None => framed.to_owned(),
    }
}

/// The environment a delivery that still runs a HELPER is given: the action
/// and ref the body store names the recovery file after (the ref only when
/// there is one — an empty variable would read as none anyway), and the
/// caller's explicit override when it gave one, so the envelope names the same
/// actor the event does. Nothing else.
#[must_use]
pub fn delivery_env<'a>(
    env: &'a Env,
    action: &'a str,
    reference: &'a str,
) -> Vec<(&'a str, &'a str)> {
    let mut pairs = vec![("_AE_EVENT_ACTION", action)];
    if !reference.is_empty() {
        pairs.push(("_AE_EVENT_REF", reference));
    }
    if let Some(name) = env
        .sender_override
        .as_deref()
        .filter(|name| !name.is_empty())
    {
        pairs.push(("AE_SENDER_OVERRIDE", name));
    }
    pairs
}

/// Send end to end. `display` is the calling pane's display ref (empty for
/// none); `own_session` is this session's name as P2.1b derives it; `defer`
/// is `AE_SEND_DEFER_SEC`. Nothing is printed on success.
///
/// # Errors
///
/// Only a failure to write `err`.
#[allow(
    clippy::too_many_arguments,
    reason = "the frozen helper's inputs, spelled out rather than bundled"
)]
pub fn run(
    dir: &Path,
    tail: &[String],
    env: &Env,
    display: &str,
    own_session: &str,
    now: Timestamp,
    defer: std::time::Duration,
    err: &mut impl Write,
) -> io::Result<u8> {
    let Ok(parsed) = parse(tail) else {
        write!(err, "{USAGE}")?;
        return Ok(EXIT_USAGE);
    };
    if tracked::is_blank(&parsed.message) {
        write!(err, "{}", tracked::refusal(ACTION))?;
        return Ok(EXIT_FAILED);
    }
    let (action, reference, summary) = fields(env, &parsed.message);
    let actor = actor(env, display);
    let mut event = EventFields {
        ts: now,
        actor: &actor,
        action: &action,
        target: &parsed.target,
        reference: &reference,
        actor_slot: &env.actor_slot,
        actor_session: &env.actor_session,
        target_slot: &env.target_slot,
        target_session: &env.target_session,
        summary: &summary,
        body_file: "",
    };
    if tracked::is_external(&parsed.target) {
        // An event-only sink: the frozen body records and exits, pasting
        // nothing and storing nothing.
        if let Err(why) = state::emit(dir, &tracked::event_line(&event)) {
            writeln!(err, "ae: {action} to {} not recorded: {why}", parsed.target)?;
            return Ok(EXIT_FAILED);
        }
        return Ok(0);
    }
    let (resolved, server) = match tracked::resolve_on(&parsed.target, own_session, dir) {
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
    let request = deliver::Request {
        dir,
        server: &server,
        pane: &resolved.pane,
        logged_target: &target_name,
        target_session: &resolved.session,
        pane_slot: &resolved.slot,
        own_session,
        action: &action,
        reference: &reference,
        actor: envelope_sender(env, display),
        body: &parsed.message,
        shape: deliver::Shape::Send,
        defer,
    };
    let Ok(delivered) = deliver::deliver(&request, err)? else {
        // Every arm of a refused delivery has already said what happened and
        // where the body is; nothing is recorded for one.
        return Ok(EXIT_FAILED);
    };
    let recorded = delivered_summary(env, &delivered.framed);
    event.target = &target_name;
    event.body_file = &delivered.body_file;
    event.summary = &recorded;
    if let Err(why) = state::emit(dir, &tracked::event_line(&event)) {
        writeln!(
            err,
            "ae: {action} to {target_name} was delivered but its event was not emitted: {why}"
        )?;
        return Ok(EXIT_FAILED);
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::{
        Env, Parsed, Usage, actor, delivered_summary, delivery_env, envelope_sender,
        extract_req_id, fields, parse,
    };

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn argv_reads_as_the_helper_reads_it() {
        assert_eq!(parse(&[]), Err(Usage));
        assert_eq!(parse(&words(&["cl:lead"])), Err(Usage), "$# -lt 2");
        assert_eq!(
            parse(&words(&["cl:lead", "hello", "there"])),
            Ok(Parsed {
                target: "cl:lead".into(),
                message: "hello there".into()
            })
        );
    }

    #[test]
    fn the_request_id_is_the_first_bracketed_ae_or_review_id() {
        assert_eq!(extract_req_id("plain text"), None);
        assert_eq!(
            extract_req_id("[ae-20260827T093000Z-ab12cd34] the answer").as_deref(),
            Some("ae-20260827T093000Z-ab12cd34")
        );
        assert_eq!(
            extract_req_id("see [review-1] then [ae-2]").as_deref(),
            Some("review-1"),
            "leftmost match"
        );
        assert_eq!(
            extract_req_id("[x] [ae-2]").as_deref(),
            Some("ae-2"),
            "a bracket that is not an id is skipped"
        );
        assert_eq!(extract_req_id("[ae-]"), None, "one id character at least");
        assert_eq!(
            extract_req_id("[ae-1 ]"),
            None,
            "the class ends at the bracket"
        );
        assert_eq!(
            extract_req_id("[ae-a.b_c:d-e]").as_deref(),
            Some("ae-a.b_c:d-e")
        );
        assert_eq!(extract_req_id("[ae-ü]"), None, "ASCII class only");
    }

    #[test]
    fn the_actor_is_the_override_else_the_pane_else_human() {
        let none = Env::default();
        assert_eq!(actor(&none, "cl:lead"), "cl:lead");
        assert_eq!(actor(&none, ""), "human");
        let bridge = Env {
            sender_override: Some("telegram:42".into()),
            ..Env::default()
        };
        assert_eq!(actor(&bridge, "cl:lead"), "telegram:42");
        let empty = Env {
            sender_override: Some(String::new()),
            ..Env::default()
        };
        assert_eq!(
            actor(&empty, ""),
            "human",
            "an empty override is no override"
        );
    }

    #[test]
    fn the_fields_default_as_the_body_defaults_them() {
        let none = Env::default();
        assert_eq!(
            fields(&none, "[ae-1] two\nlines"),
            ("send".into(), "ae-1".into(), "[ae-1] two\nlines".into()),
            "action send, ref from the text, summary raw — the emitter renders it"
        );
        let cancel = Env {
            action: Some("cancel".into()),
            reference: Some("ae-9".into()),
            summary: Some("withdrawn".into()),
            ..Env::default()
        };
        assert_eq!(
            fields(&cancel, "[ae-1] text"),
            ("cancel".into(), "ae-9".into(), "withdrawn".into()),
            "explicit members win over the text"
        );
        let long = "x".repeat(250);
        assert_eq!(fields(&none, &long).2.len(), 250, "not capped here");
    }

    #[test]
    fn the_entry_gets_the_names_for_the_recovery_file_and_the_explicit_override_only() {
        let none = Env::default();
        assert_eq!(
            delivery_env(&none, "send", ""),
            vec![("_AE_EVENT_ACTION", "send")]
        );
        assert_eq!(
            delivery_env(&none, "send", "ae-1"),
            vec![("_AE_EVENT_ACTION", "send"), ("_AE_EVENT_REF", "ae-1")]
        );
        let bridge = Env {
            sender_override: Some("telegram:42".into()),
            ..Env::default()
        };
        assert_eq!(
            delivery_env(&bridge, "send", ""),
            vec![
                ("_AE_EVENT_ACTION", "send"),
                ("AE_SENDER_OVERRIDE", "telegram:42")
            ],
            "an explicit override rides to the envelope"
        );
        let empty = Env {
            sender_override: Some(String::new()),
            ..Env::default()
        };
        assert_eq!(
            delivery_env(&empty, "send", ""),
            vec![("_AE_EVENT_ACTION", "send")],
            "an empty one is not invented"
        );
    }

    #[test]
    fn the_summary_is_of_the_delivered_text() {
        let framed = "⟦ae:msg from cl:lead⟧\nhello\tthere";
        assert_eq!(
            delivered_summary(&Env::default(), framed),
            framed,
            "the framed text, raw — the emitter renders it for the action"
        );
        let explicit = Env {
            summary: Some("withdrawn".into()),
            ..Env::default()
        };
        assert_eq!(
            delivered_summary(&explicit, framed),
            "withdrawn",
            "an explicit summary is not overridden by the delivery"
        );
        let blank = Env {
            summary: Some(String::new()),
            ..Env::default()
        };
        assert_eq!(
            delivered_summary(&blank, framed),
            framed,
            "an EMPTY _AE_EVENT_SUMMARY is none, as the frozen default-expansion reads it"
        );
    }

    #[test]
    fn the_envelope_names_the_verified_sender_and_never_guesses() {
        let bridge = Env {
            sender_override: Some("telegram:chat".into()),
            ..Env::default()
        };
        assert_eq!(envelope_sender(&bridge, "cl:lead"), "telegram:chat");
        assert_eq!(envelope_sender(&Env::default(), "cl:lead"), "cl:lead");
        assert_eq!(
            envelope_sender(&Env::default(), ""),
            "",
            "no pane, no override: the envelope has no claim to make, and deliver marks it unverified"
        );
        let empty_override = Env {
            sender_override: Some(String::new()),
            ..Env::default()
        };
        assert_eq!(
            envelope_sender(&empty_override, "cl:lead"),
            "cl:lead",
            "an EMPTY override is not an override — the frozen provenance reads it that way too"
        );
        assert_eq!(
            actor(&Env::default(), ""),
            "human",
            "the EVENT's blank is human; the envelope's is not — the asymmetry is the authority model"
        );
    }
}
