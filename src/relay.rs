//! The orchestrator seat's privileged, unenveloped relay.
//!
//! `relay <session[:agent]> <text…>` joins the remaining argv with one space,
//! resolves the target on that session's recorded tmux server, and pastes the
//! text verbatim. Bare text carries human authority, so this helper fails
//! closed unless tmux proves the caller is inside a session whose meta says
//! exactly one `meta_agent=true`. Every attempt with a proven caller session is
//! recorded only there; the target's ledger is never touched.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config;
use crate::deliver;
use crate::inventory::ServerId;
use crate::meta;
use crate::requests::is_slot;
use crate::session_launch::name::is_session_name;
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::store;
use crate::time::Timestamp;
use crate::tmux::ObservedViewer;
use crate::tracked::{self, EventFields};

/// The helper's argv contract.
pub const USAGE: &str = "Usage: relay <session[:agent]> <text…>\n";

/// The event action and recovery-body stem.
pub const ACTION: &str = "relay";

/// Parsed relay argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// The session, or session-and-agent, as typed.
    pub target: String,
    /// Remaining argv joined by one space.
    pub text: String,
}

/// A refused argv.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage;

/// Parse `<target> <text…>`; one quoted text argument and many words are
/// equivalent after the shell has produced argv.
///
/// # Errors
///
/// [`Usage`] when either part is absent.
pub fn parse(tail: &[String]) -> Result<Parsed, Usage> {
    match tail {
        [target, words @ ..] if !words.is_empty() => Ok(Parsed {
            target: target.clone(),
            text: words.join(" "),
        }),
        _ => Err(Usage),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Caller {
    dir: PathBuf,
    session: String,
    actor: String,
    slot: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    session: String,
    display: String,
}

/// Run one relay end to end.
///
/// `dir` is the helper directory used to enter the core; `actual` is the
/// caller identity and server read from the actual `$TMUX` socket.
///
/// # Errors
///
/// Only a failure to write `err`.
#[allow(
    clippy::too_many_arguments,
    reason = "the helper entry point's explicit authority and delivery inputs"
)]
pub fn run(
    dir: &Path,
    tail: &[String],
    actual: Option<(&ObservedViewer, &ServerId)>,
    now: Timestamp,
    defer: Duration,
    err: &mut impl Write,
) -> io::Result<u8> {
    let Ok(parsed) = parse(tail) else {
        write!(err, "{USAGE}")?;
        return Ok(EXIT_USAGE);
    };
    let (caller, caller_meta) = match caller(dir, actual) {
        Ok(caller) => caller,
        Err(reason) => {
            // With no proven caller there is no trustworthy session ledger to
            // write. The refusal itself still stays one line and fails closed.
            writeln!(err, "ae: relay REFUSED — {reason}. Nothing was sent.")?;
            return Ok(EXIT_FAILED);
        }
    };
    if meta::sole_value(&caller_meta, "meta_agent") != Some(b"true".as_slice()) {
        return refuse(
            &caller,
            &parsed,
            "caller session is not an orchestrator",
            now,
            err,
        );
    }
    if parsed.text.is_empty() {
        return refuse(&caller, &parsed, "text is empty", now, err);
    }
    if parsed.text.len() as u64 > crate::deliver::notice::LIMIT {
        return refuse(
            &caller,
            &parsed,
            &format!(
                "message is {} B; verbatim relay limit is {} B",
                parsed.text.len(),
                crate::deliver::notice::LIMIT
            ),
            now,
            err,
        );
    }
    let target = match target(&caller, &parsed.target) {
        Ok(target) => target,
        Err(reason) => return refuse(&caller, &parsed, &reason, now, err),
    };
    let (resolved, server) =
        match tracked::resolve_on(&target.display, &caller.session, &caller.dir) {
            Ok(answer) => answer,
            Err(why) => return refuse(&caller, &parsed, &why.message(), now, err),
        };
    let fields = EventFields {
        ts: now,
        actor: &caller.actor,
        action: ACTION,
        target: &target.display,
        reference: "",
        actor_slot: &caller.slot,
        actor_session: &caller.session,
        target_slot: &resolved.slot,
        target_session: &target.session,
        summary: &parsed.text,
        body_file: "",
    };
    let request = deliver::Request {
        dir: &caller.dir,
        server: &server,
        pane: &resolved.pane,
        logged_target: &target.display,
        target_session: &target.session,
        pane_slot: &resolved.slot,
        own_session: &caller.session,
        action: ACTION,
        reference: "",
        actor: &caller.actor,
        body: &parsed.text,
        shape: deliver::Shape::Relay,
        defer,
    };
    let delivery = deliver::deliver(&request, err)?;
    match delivery {
        Ok(delivered) => audit_landed(&caller, &fields, &delivered.body_file, false, err),
        Err(deliver::Failure::Unconfirmed {
            body_file,
            notice: false,
            ..
        }) => audit_landed(&caller, &fields, &body_file, true, err),
        Err(failure) => audit_delivery_failure(&caller, &fields, &failure, err),
    }
}

fn caller(
    dir: &Path,
    actual: Option<(&ObservedViewer, &ServerId)>,
) -> Result<(Caller, Vec<u8>), &'static str> {
    let Some((viewer, actual_server)) = actual else {
        return Err("caller pane could not be verified");
    };
    let (Some(session), Some(actor)) = (viewer.session.as_deref(), viewer.agent.as_deref()) else {
        return Err("caller pane has no verified ae identity");
    };
    if !is_session_name(session) || !config::is_agent_name(actor) {
        return Err("caller pane has an invalid ae identity");
    }
    let Some(root) = dir.parent() else {
        return Err("caller session directory could not be resolved");
    };
    let caller_dir = root.join(session);
    let Ok(bytes) = meta::read_bytes(&caller_dir) else {
        return Err("caller session meta could not be read");
    };
    if meta::sole_value(&bytes, "session") != Some(session.as_bytes()) {
        return Err("caller session meta does not match its pane");
    }
    let parsed = meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let meta::ServerSelector::Positive(selector) = parsed.server_selector() else {
        return Err("caller session is not the recorded one");
    };
    let recorded_server = ServerId::Selected(selector);
    let mut sockets = crate::SocketPaths::asking(crate::transport::observe_socket_path);
    if !sockets.proven_same(actual_server, &recorded_server) {
        return Err("caller session is not the recorded one");
    }
    Ok((
        Caller {
            dir: caller_dir,
            session: session.to_owned(),
            actor: actor.to_owned(),
            slot: viewer
                .slot
                .as_deref()
                .filter(|slot| is_slot(slot))
                .unwrap_or_default()
                .to_owned(),
        },
        bytes,
    ))
}

fn target(caller: &Caller, raw: &str) -> Result<Target, String> {
    let (session, named_agent) = match raw.split_once(':') {
        Some((session, agent)) if !agent.contains(':') => (session, Some(agent)),
        Some(_) => return Err("target must be session or session:agent".to_owned()),
        None => (raw, None),
    };
    if !is_session_name(session) {
        return Err("target session name is invalid".to_owned());
    }
    if session == caller.session {
        return Err("target is the caller session".to_owned());
    }
    let agent = match named_agent {
        Some(agent) if config::is_agent_name(agent) => agent.to_owned(),
        Some(_) => return Err("target agent name is invalid".to_owned()),
        None => target_main(&caller.dir, session)?,
    };
    Ok(Target {
        session: session.to_owned(),
        display: format!("{session}:{agent}"),
    })
}

fn target_main(caller_dir: &Path, session: &str) -> Result<String, String> {
    let Some(root) = caller_dir.parent() else {
        return Err("target session could not be resolved".to_owned());
    };
    let target_dir = root.join(session);
    let bytes = meta::read_bytes(&target_dir)
        .map_err(|_| format!("target session '{session}' meta could not be read"))?;
    if meta::sole_value(&bytes, "session") != Some(session.as_bytes()) {
        return Err(format!("target session '{session}' meta does not match"));
    }
    let parsed = meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let mut mains = parsed.roster().iter().filter(|entry| entry.slot == "main");
    let Some(main) = mains.next() else {
        return Err(format!("target session '{session}' has no main agent"));
    };
    if mains.next().is_some() || !config::is_agent_name(&main.name) {
        return Err(format!(
            "target session '{session}' has no unique main agent"
        ));
    }
    Ok(main.name.clone())
}

fn refuse(
    caller: &Caller,
    parsed: &Parsed,
    reason: &str,
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<u8> {
    let summary = format!("refused: {reason}; {}", parsed.text);
    let line = tracked::event_line(&EventFields {
        ts: now,
        actor: &caller.actor,
        action: ACTION,
        target: &parsed.target,
        reference: "",
        actor_slot: &caller.slot,
        actor_session: &caller.session,
        target_slot: "",
        target_session: "",
        summary: &summary,
        body_file: "",
    });
    let audit = store::open(&caller.dir).append_event(&line);
    match audit {
        Ok(()) => writeln!(err, "ae: relay REFUSED — {reason}. Nothing was sent.")?,
        Err(why) => writeln!(
            err,
            "ae: relay REFUSED — {reason}; attempt audit failed: {why}. Nothing was sent."
        )?,
    }
    Ok(EXIT_FAILED)
}

fn audit_delivery_failure(
    caller: &Caller,
    fields: &EventFields<'_>,
    failure: &deliver::Failure,
    err: &mut impl Write,
) -> io::Result<u8> {
    let reason = match failure {
        deliver::Failure::DeadPane => "target pane is not a running agent",
        deliver::Failure::Storage => "recovery body could not be stored",
        deliver::Failure::Lock => "target delivery lock was not acquired",
        deliver::Failure::NoticeRefused { .. } => "verbatim relay limit was exceeded",
        deliver::Failure::Abandoned => "target stayed busy",
        deliver::Failure::Paste { .. } => "paste failed",
        deliver::Failure::Unconfirmed { .. } => "submit was not confirmed",
    };
    let summary = format!("refused: {reason}; {}", fields.summary);
    let line = tracked::event_line(&EventFields {
        ts: fields.ts,
        actor: fields.actor,
        action: fields.action,
        target: fields.target,
        reference: fields.reference,
        actor_slot: fields.actor_slot,
        actor_session: fields.actor_session,
        target_slot: fields.target_slot,
        target_session: fields.target_session,
        summary: &summary,
        body_file: failure.body_file(),
    });
    if let Err(why) = store::open(&caller.dir).append_event(&line) {
        writeln!(err, "ae: relay attempt audit failed: {why}")?;
    }
    Ok(EXIT_FAILED)
}

fn audit_landed(
    caller: &Caller,
    fields: &EventFields<'_>,
    body_file: &str,
    unconfirmed: bool,
    err: &mut impl Write,
) -> io::Result<u8> {
    let fields = EventFields {
        ts: fields.ts,
        actor: fields.actor,
        action: fields.action,
        target: fields.target,
        reference: fields.reference,
        actor_slot: fields.actor_slot,
        actor_session: fields.actor_session,
        target_slot: fields.target_slot,
        target_session: fields.target_session,
        summary: fields.summary,
        body_file,
    };
    let line = if unconfirmed {
        tracked::unconfirmed_event_line(&fields)
    } else {
        tracked::event_line(&fields)
    };
    if let Err(why) = store::open(&caller.dir).append_event(&line) {
        writeln!(
            err,
            "ae: relay to {} reached the pane but its caller audit was not emitted: {why}",
            fields.target
        )?;
        return Ok(EXIT_FAILED);
    }
    if unconfirmed {
        writeln!(
            err,
            "ae: relay to {} was audited unconfirmed; retry only if peek shows the text still in the input box.",
            fields.target
        )?;
    }
    Ok(0)
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests build and inspect isolated session metadata"
)]
mod tests {
    use super::{Caller, Parsed, Target, Usage, parse, target};
    use std::path::PathBuf;

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn argv_accepts_quoted_or_split_text_and_requires_both_parts() {
        assert_eq!(
            parse(&words(&["target", "two", "words"])),
            Ok(Parsed {
                target: "target".to_owned(),
                text: "two words".to_owned(),
            })
        );
        assert_eq!(
            parse(&words(&["target", "two words"])),
            Ok(Parsed {
                target: "target".to_owned(),
                text: "two words".to_owned(),
            })
        );
        assert_eq!(parse(&words(&["target"])), Err(Usage));
        assert_eq!(parse(&[]), Err(Usage));
    }

    #[test]
    fn target_is_cross_session_only_and_world_targets_are_not_a_shape() {
        let caller = Caller {
            dir: PathBuf::from("/sessions/orchestrator"),
            session: "orchestrator".to_owned(),
            actor: "lead".to_owned(),
            slot: "main".to_owned(),
        };
        assert_eq!(
            target(&caller, "work:reviewer"),
            Ok(Target {
                session: "work".to_owned(),
                display: "work:reviewer".to_owned(),
            })
        );
        assert_eq!(
            target(&caller, "orchestrator:lead"),
            Err("target is the caller session".to_owned())
        );
        assert_eq!(
            target(&caller, "@work:reviewer"),
            Err("target session name is invalid".to_owned())
        );
        assert_eq!(
            target(&caller, "world:*"),
            Err("target agent name is invalid".to_owned())
        );
    }
}
