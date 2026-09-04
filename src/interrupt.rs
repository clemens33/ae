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
//! * **It is not framed.** An interrupt is a control action, not transcript
//!   chat, so no provenance envelope leads it. The envelope's actor still
//!   names the oversize notice, because a pointer has to say who is asking.
//!
//! A MESSAGE-less interrupt is just the two cancel keystrokes, and it is
//! deliberately allowed against a pane whose agent has died: there is nothing
//! there for a stray Enter to execute. With a message the dead-pane guard is
//! the send's, verbatim — a paste plus Enter into a shell EXECUTES it.

use std::io::{self, Write};
use std::path::Path;

use crate::deliver::{self, Shape};
use crate::state::{self, EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;
use crate::tracked::{self, EventFields};
use crate::transport;

/// The frozen usage text.
pub const USAGE: &str = "Usage: interrupt <agent-name|pane-id|@session:agent> [message]\n  Examples: interrupt codex:reviewer\n           interrupt @my-feature:claude:lead \"Stop — try a different approach\"\n";

/// The event action.
pub const ACTION: &str = "interrupt";

/// What the argv said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
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
    match tail {
        [target, message @ ..] => Ok(Parsed {
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
    own_session: &str,
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<u8> {
    let Ok(parsed) = parse(tail) else {
        write!(err, "{USAGE}")?;
        return Ok(EXIT_USAGE);
    };
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
    if parsed.message.is_empty() {
        // Cancel keystrokes only.
        let _ = transport::send_key(&server, &resolved.pane, crate::tmux::Key::CancelCopyMode);
        let _ = transport::send_key(&server, &resolved.pane, crate::tmux::Key::Escape);
        return record(dir, &target_name, "", now, actor, err);
    }
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
    };
    let delivered = match deliver::deliver(&request, err)? {
        Ok(delivered) => delivered,
        Err(failure) => {
            // A message interrupt must NOT record success on an unconfirmed
            // submit: the delivery has already said what happened and where the
            // body is.
            if let deliver::Failure::Unconfirmed {
                body_file,
                notice: true,
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
        now,
        actor,
        err,
    )
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
        summary: &format!(
            "UNCONFIRMED notice; published body: {body_file}; interrupt submit proof failed"
        ),
        body_file: "",
    });
    state::emit(dir, &line)
}

/// The `interrupt` event, with no recovery record — a bare cancel stores
/// nothing.
fn record(
    dir: &Path,
    target: &str,
    summary: &str,
    now: Timestamp,
    actor: &str,
    err: &mut impl Write,
) -> io::Result<u8> {
    record_with_body(dir, target, summary, "", now, actor, err)
}

/// The `interrupt` event.
fn record_with_body(
    dir: &Path,
    target: &str,
    summary: &str,
    body_file: &str,
    now: Timestamp,
    actor: &str,
    err: &mut impl Write,
) -> io::Result<u8> {
    let actor = if actor.is_empty() { "human" } else { actor };
    let line = tracked::event_line(&EventFields {
        ts: now,
        actor,
        action: ACTION,
        target,
        reference: "",
        actor_slot: "",
        actor_session: "",
        target_slot: "",
        target_session: "",
        summary,
        body_file,
    });
    if let Err(why) = state::emit(dir, &line) {
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
                target: "reviewer".into(),
                message: String::new()
            }),
            "a bare cancel is the common case, not a usage error"
        );
        assert_eq!(
            parse(&words(&["reviewer", "try", "another", "approach"])),
            Ok(Parsed {
                target: "reviewer".into(),
                message: "try another approach".into()
            })
        );
    }
}
