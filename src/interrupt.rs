//! The `interrupt` helper: cancel what a target is doing, and optionally hand
//! it new instructions.
//!
//! Ported from `ae`'s `helper_interrupt_main`. It is a SEND with three
//! deliberate differences, each of them the point of the command:
//!
//! * **It does not wait for a quiet input box.** The whole reason to
//!   interrupt is that the target is mid-generation, so the deferral that
//!   protects an ordinary send would defeat this one.
//! * **It cancels first.** Copy mode, then `Escape` — and only then, after a
//!   settle, is any message pasted.
//! * **It is framed as the control action it is.** The body is led by the
//!   `⟦ae:interrupt from <agent>⟧` marker — never the peer envelope, because
//!   an interrupt is a control action, not transcript chat. The envelope's
//!   actor still names the oversize notice, because a pointer has to say who
//!   is asking.
//!
//! A MESSAGE-less interrupt is the two cancel keystrokes — under the target
//! send-lock a message interrupt takes, after the same human-only-prompt read
//! — and it is deliberately allowed against a pane whose agent has died: there
//! is nothing there for a stray Enter to execute. With a message the dead-pane
//! guard is the send's, verbatim — a paste plus Enter into a shell EXECUTES it.

use std::io::{self, Write};
use std::path::Path;

use crate::deliver::{self, DeferHeld, Shape};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::store;
use crate::time::Timestamp;
use crate::tool::InputModel;
use crate::tracked::{self, EventFields};
use crate::transport;

/// The frozen usage text.
pub const USAGE: &str = "Usage: interrupt [--cross-session] <agent-name|pane-id|@session:agent> [message]\n  Examples: interrupt codex:reviewer\n           interrupt --cross-session @my-feature:claude:lead \"Stop — try a different approach\"\n";

/// The event action.
pub const ACTION: &str = "interrupt";

/// What the argv said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// Whether the caller states that the human explicitly authorized a
    /// cross-session delivery.
    pub cross_session: bool,
    /// The target as typed.
    pub target: String,
    /// The message: the remaining words joined by one space, or empty.
    pub message: String,
}

/// A refused argv: [`USAGE`] to stderr, the usage exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage;

/// Parse the argv after the meta directory.
///
/// # Errors
///
/// [`Usage`] for no target at all.
pub fn parse(tail: &[String]) -> Result<Parsed, Usage> {
    let (cross_session, tail) = tracked::split_cross_session_flag(tail);
    match tail {
        [target, message @ ..] => Ok(Parsed {
            cross_session,
            target: target.clone(),
            message: message.join(" "),
        }),
        [] => Err(Usage),
    }
}

/// Interrupt end to end.
///
/// # Errors
///
/// Only a failure to write `err`.
pub fn run(
    dir: &Path,
    tail: &[String],
    actor: &str,
    caller_session: &str,
    own_session: &str,
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<u8> {
    let Ok(parsed) = parse(tail) else {
        write!(err, "{USAGE}")?;
        return Ok(EXIT_USAGE);
    };
    let caller_session = if caller_session.is_empty() {
        own_session
    } else {
        caller_session
    };
    let (resolved, server) = match tracked::resolve_on(&parsed.target, own_session, dir) {
        Ok(resolved) => resolved,
        Err(why) => {
            writeln!(err, "{}", why.message())?;
            return Ok(EXIT_FAILED);
        }
    };
    if tracked::refuse_cross_session(
        dir,
        ACTION,
        parsed.cross_session,
        &parsed.target,
        &resolved,
        actor,
        "",
        caller_session,
        now,
        err,
    )? {
        return Ok(EXIT_FAILED);
    }
    let cross = (resolved.session != caller_session).then_some(CrossDelivery {
        caller_session,
        target_slot: &resolved.slot,
        target_session: &resolved.session,
    });
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
        action: ACTION,
        reference: "",
        actor,
        body: &parsed.message,
        shape: Shape::Interrupt,
        defer: deliver::DEFAULT_DEFER,
        composed: crate::tool::Composed::NONE,
    };
    if parsed.message.is_empty() {
        return bare_cancel(&request, &target_name, now, cross, err);
    }
    let delivered = match deliver::deliver(&request, err)? {
        Ok(delivered) => delivered,
        // Refused before its paste on a human-only prompt: the shared
        // diagnostic, never an `interrupt` record.
        Err(deliver::Failure::Abandoned { held }) => {
            return record_with_body(
                dir,
                &target_name,
                &parsed.message,
                "",
                Err(held),
                now,
                actor,
                cross,
                err,
            );
        }
        Err(failure) => {
            // A message interrupt must NOT record success on an unconfirmed
            // submit: the delivery has already said what happened and where the
            // body is.
            if let deliver::Failure::Unconfirmed {
                body_file,
                notice: true,
                ..
            } = &failure
            {
                let _ = record_delivery_failure(dir, &target_name, body_file, now, actor);
            }
            return Ok(EXIT_FAILED);
        }
    };
    record_with_body(
        dir,
        &target_name,
        &parsed.message,
        &delivered.body_file,
        Ok(delivered.verification),
        now,
        actor,
        cross,
        err,
    )
}

/// Cancel keystrokes only — never into a human-only prompt, where an Escape
/// would answer it, nor between another sender's paste and its Enter: the read
/// and both keys go under the target send-lock.
fn bare_cancel(
    request: &deliver::Request<'_>,
    target: &str,
    now: Timestamp,
    cross: Option<CrossDelivery<'_>>,
    err: &mut impl Write,
) -> io::Result<u8> {
    let Some(_lock) = deliver::target_lock(request, err)? else {
        return Ok(EXIT_FAILED);
    };
    let (dir, server, pane, actor) = (request.dir, request.server, request.pane, request.actor);
    let tool = deliver::prompt_tool(request);
    let (_, read) = deliver::prompt_hold(server, pane, InputModel::Unmodelled, tool);
    if let Some(held) = deliver::prompt_refusal(request, false, &read, err)? {
        return record(dir, target, Some(held), now, actor, cross, err);
    }
    let _ = transport::send_key(server, pane, crate::tmux::Key::CancelCopyMode);
    let _ = transport::send_key(server, pane, crate::tmux::Key::Escape);
    record(dir, target, None, now, actor, cross, err)
}

/// Routing facts needed only when an interrupt crosses a session boundary.
#[derive(Debug, Clone, Copy)]
struct CrossDelivery<'a> {
    caller_session: &'a str,
    target_slot: &'a str,
    target_session: &'a str,
}

/// The frozen `_notice_emit_failure`: a `delivery-failed` line naming the
/// published body and why the pointer was not submitted.
fn record_delivery_failure(
    dir: &Path,
    target: &str,
    body_file: &str,
    now: Timestamp,
    actor: &str,
) -> io::Result<()> {
    let actor = if actor.is_empty() { "human" } else { actor };
    let line = tracked::event_line(&EventFields {
        ts: now,
        actor,
        action: "delivery-failed",
        target,
        reference: "",
        actor_slot: "",
        actor_session: "",
        target_slot: "",
        target_session: "",
        actor_session_id: "",
        target_session_id: "",
        target_server: "",
        target_pane: "",
        target_session_uuid: "",
        caller_server: "",
        caller_pane: "",
        caller_session_uuid: "",
        identity_gap: "",
        summary: &format!(
            "UNCONFIRMED notice; published body: {body_file}; interrupt submit proof failed"
        ),
        body_file: "",
    });
    store::open(dir)
        .append_event(&line)
        .map_err(io::Error::from)
}

/// The `interrupt` event, with no recovery record — a bare cancel stores
/// nothing. A `held` cancel was refused, and records that instead.
fn record(
    dir: &Path,
    target: &str,
    held: Option<DeferHeld>,
    now: Timestamp,
    actor: &str,
    cross: Option<CrossDelivery<'_>>,
    err: &mut impl Write,
) -> io::Result<u8> {
    record_with_body(
        dir,
        target,
        "",
        "",
        held.map_or(Ok(deliver::DeliveryVerification::Verified), Err),
        now,
        actor,
        cross,
        err,
    )
}

/// The `interrupt` event, or the refusal an `Err` names.
#[allow(
    clippy::too_many_arguments,
    reason = "the interrupt event and optional cross-session route are explicit inputs"
)]
fn record_with_body(
    dir: &Path,
    target: &str,
    summary: &str,
    body_file: &str,
    verification: Result<deliver::DeliveryVerification, DeferHeld>,
    now: Timestamp,
    actor: &str,
    cross: Option<CrossDelivery<'_>>,
    err: &mut impl Write,
) -> io::Result<u8> {
    let actor = if actor.is_empty() { "human" } else { actor };
    let (actor_session, target_slot, target_session) = cross.map_or(("", "", ""), |route| {
        (
            route.caller_session,
            route.target_slot,
            route.target_session,
        )
    });
    let fields = EventFields {
        ts: now,
        actor,
        action: ACTION,
        target,
        reference: "",
        actor_slot: "",
        actor_session,
        target_slot,
        target_session,
        actor_session_id: "",
        target_session_id: "",
        target_server: "",
        target_pane: "",
        target_session_uuid: "",
        caller_server: "",
        caller_pane: "",
        caller_session_uuid: "",
        identity_gap: "",
        summary,
        body_file,
    };
    let verification = match verification {
        Ok(verification) => verification,
        Err(held) => return tracked::record_abandoned_delivery(dir, &fields, held, err),
    };
    let line = tracked::delivery_event_line(&fields, verification, cross.is_some());
    let cross_session = cross.map(|route| tracked::CrossSession {
        caller: route.caller_session,
        target: route.target_session,
    });
    if let Err(why) = tracked::append_delivery_event(dir, &line, cross_session) {
        writeln!(err, "ae: interrupt of {target} not recorded: {why}")?;
        return Ok(EXIT_FAILED);
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::{Parsed, Usage, parse};

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn a_message_is_optional_but_a_target_is_not() {
        assert_eq!(parse(&[]), Err(Usage));
        assert_eq!(
            parse(&words(&["reviewer"])),
            Ok(Parsed {
                cross_session: false,
                target: "reviewer".into(),
                message: String::new()
            }),
            "a bare cancel is the common case, not a usage error"
        );
        assert_eq!(
            parse(&words(&["reviewer", "try", "another", "approach"])),
            Ok(Parsed {
                cross_session: false,
                target: "reviewer".into(),
                message: "try another approach".into()
            })
        );
        assert_eq!(
            parse(&words(&["--cross-session", "@other:reviewer"])),
            Ok(Parsed {
                cross_session: true,
                target: "@other:reviewer".into(),
                message: String::new(),
            })
        );
    }

    /// An unconfirmed message interrupt journals `delivery-failed` naming the
    /// published body (this fn's doc comment; deliver.rs `Failure::Unconfirmed`).
    #[test]
    fn an_unconfirmed_interrupt_journals_delivery_failed_naming_its_body() {
        let dir = std::env::temp_dir().join(format!("ae-interrupt-failed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let now = crate::time::Timestamp::from_epoch(1_791_000_000);
        super::record_delivery_failure(&dir, "worker", "/m/b.txt", now, "").unwrap();
        let events = crate::watchdog_daemon::read_events(&dir);
        let failed = events.last().expect("a delivery-failed record");
        assert_eq!(
            (failed.action.as_str(), failed.target.as_deref()),
            ("delivery-failed", Some("worker"))
        );
        assert!(
            failed
                .summary
                .as_deref()
                .is_some_and(|text| text.contains("/m/b.txt"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
