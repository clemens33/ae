//! `_stop <name> --expect-uuid <uuid>`: the bound stop, the one stop the
//! orchestrator seat may run.
//!
//! It stops a session only when that name resolves, under the session's
//! lifecycle lock, to exactly the session uuid the caller read from the fleet.
//! The order is the contract:
//!
//! 1. the caller is read from `$TMUX_PANE` alone, and the caller's own session
//!    and the fleet word `all` are refused;
//! 2. under the lifecycle lock, with no write: the name is a real session
//!    directory (never a rename alias), its meta names exactly that uuid, its
//!    record names a positive server, and the live session of that name on it
//!    carries that uuid as its `@ae_session_uuid` stamp, read with its `$<n>`
//!    id in one listing;
//! 3. still without a write, each seat's pane text and launch guard are read;
//! 4. the event container's lock is taken and held: the roster is read again
//!    from the current meta, and any seat whose latest declaration is not
//!    `done` or `waiting-user` refuses, as does a journal ae cannot read;
//! 5. still holding it: the stop request, the client handoff, the kill of
//!    exactly that `$<n>` id and the stop result;
//! 6. after the release, still under the lifecycle lock: the migration note
//!    and the model publication from step 3.
//!
//! A refusal before step 5 writes nothing. A declaration either precedes the
//! read in step 4 and is judged there, or waits on the held lock until the
//! kill is done. This process never holds the event lock and the meta lock
//! together, so it keeps the meta-then-events order of every other writer.

use std::io::{self, Write};
use std::path::Path;

use super::{
    ALREADY_STOPPED_SUMMARY, CallerGap, STOP_REQUEST_ACTION, STOP_RESULT_ACTION, StopOutcome,
    caller_resolution, capture_models, emit_stop_event, kill_under_lock, lifecycle_event_line,
    lock, name_is_usable, publish_models, server_of, sessions_dir, stop_summary,
};
use crate::doors::CallerServer;
use crate::inventory::ServerId;
use crate::meta::{self, ServerSelector};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::store::SourceRead;
use crate::tmux::StopProbe;
use crate::transport;

/// The declarations a bound stop admits. A seat that never declared is
/// admitted too; every other value, a future one included, refuses.
const IDLE_STATES: [&str; 2] = ["done", "waiting-user"];

/// `_stop <name> --expect-uuid <uuid>`; `panes` is every `--pane` word of the
/// tail, in order.
pub(super) fn run(
    root: &Path,
    name: &str,
    uuid: &str,
    panes: &[Option<String>],
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    if name == "all" {
        writeln!(
            err,
            "Error: 'all' is the fleet word; a bound stop names one session, so it never stops the fleet. Nothing was stopped."
        )?;
        return Ok(EXIT_USAGE);
    }
    let own = match caller_session(root, panes) {
        Ok(own) => own,
        Err((code, line)) => {
            writeln!(err, "Error: {line} Nothing was stopped.")?;
            return Ok(code);
        }
    };
    if own.as_deref() == Some(name) {
        writeln!(
            err,
            "Error: '{name}' is the session you are in; a bound stop never stops its caller's own session (that is ae stop --self). Nothing was stopped."
        )?;
        return Ok(EXIT_FAILED);
    }
    if !name_is_usable(root, name) {
        writeln!(err, "ae: '{name}' is not a usable session name.")?;
        return Ok(EXIT_FAILED);
    }
    let Ok(_guard) = lock(root, name) else {
        writeln!(
            err,
            "Error: another lifecycle operation (start/resume/end) is in progress for '{name}' — retry shortly. Nothing was stopped."
        )?;
        return Ok(EXIT_FAILED);
    };
    let dir = sessions_dir(root).join(name);
    let (server, id) = match prove(&dir, name, uuid) {
        Proof::Proven { server, id } => (server, id),
        Proof::Absent => {
            // Proven this uuid's directory and proven gone: the durable
            // result belongs in it, as for a confirmed stop.
            emit_stop_event(&dir, name, STOP_RESULT_ACTION, ALREADY_STOPPED_SUMMARY);
            writeln!(err, "Session '{name}' is not running.")?;
            return Ok(EXIT_FAILED);
        }
        Proof::Refused(line) => {
            writeln!(err, "Error: {line}. Nothing was stopped.")?;
            return Ok(EXIT_FAILED);
        }
    };
    let mut held_err = Vec::new();
    let captured = match capture_models(&dir, &server, name) {
        Ok((captures, gaps)) => {
            for gap in gaps {
                warn_model(&mut held_err, name, &gap)?;
            }
            captures
        }
        Err(why) => {
            warn_model(&mut held_err, name, &why)?;
            Vec::new()
        }
    };
    let hold = match crate::store::open(&dir).hold_events() {
        Ok(hold) => hold,
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was stopped.")?;
            return Ok(EXIT_FAILED);
        }
    };
    if let Err(line) = seats_idle(&dir, name, &hold) {
        writeln!(err, "Error: {line}. Nothing was stopped.")?;
        return Ok(EXIT_FAILED);
    }
    let _ = hold.append_event(&lifecycle_event_line(
        name,
        STOP_REQUEST_ACTION,
        &format!("stop requested, bound to session {uuid} ({id})"),
    ));
    let mut held_out = Vec::new();
    let outcome = kill_under_lock(&server, name, &id, &mut held_out, &mut held_err)?;
    let _ = hold.append_event(&lifecycle_event_line(
        name,
        STOP_RESULT_ACTION,
        &stop_summary(&outcome, &held_err),
    ));
    drop(hold);
    let code = u8::from(!matches!(outcome, StopOutcome::Stopped));
    let output = crate::error::with_status(code, out.write_all(&held_out));
    err.write_all(&held_err)?;
    output?;
    if let Some(note) = crate::migrate::session_noted(&dir, name) {
        writeln!(err, "{note}")?;
    }
    for gap in publish_models(&dir, captured) {
        warn_model(err, name, &gap)?;
    }
    Ok(code)
}

/// The one warning shape of a model observation the stop could not make.
fn warn_model(err: &mut impl Write, name: &str, why: &str) -> io::Result<()> {
    writeln!(
        err,
        "Warning: could not observe the seat models before stopping '{name}': {why}"
    )
}

/// The session the caller runs in, read from `$TMUX_PANE` on the server
/// `$TMUX` names and never from a `--pane` word: a `--pane` may only repeat
/// that ambient pane, once. `None` is a caller outside tmux, or one whose pane
/// is in a session ae does not record. `Err` carries the exit code and the
/// refusal.
fn caller_session(
    root: &Path,
    panes: &[Option<String>],
) -> Result<Option<String>, (u8, &'static str)> {
    let server = match crate::doors::caller_server_reading() {
        CallerServer::Absent if panes.is_empty() => return Ok(None),
        CallerServer::Absent => {
            return Err((
                EXIT_USAGE,
                "--pane names a caller pane, and this caller is outside tmux; a bound stop reads its caller from $TMUX_PANE only.",
            ));
        }
        CallerServer::Malformed => {
            return Err((
                EXIT_FAILED,
                "$TMUX names no usable socket, so ae cannot tell which session this caller is in.",
            ));
        }
        CallerServer::Resolved(server) => server,
    };
    let Some(ambient) = crate::doors::calling_pane_id() else {
        return Err((
            EXIT_FAILED,
            "$TMUX_PANE is unset inside tmux, so ae cannot tell which session this caller is in.",
        ));
    };
    match panes {
        [] => {}
        [Some(pane)] if *pane == ambient => {}
        [_] => {
            return Err((
                EXIT_USAGE,
                "--pane must name this caller's own pane ($TMUX_PANE); a bound stop never acts for another pane.",
            ));
        }
        _ => {
            return Err((
                EXIT_USAGE,
                "--pane may be given at most once on a bound stop.",
            ));
        }
    }
    match caller_resolution(root, Some(&server), &ambient) {
        Ok(session)
        | Err(CallerGap::NotAnAeSession(session) | CallerGap::ServerUnproved(session)) => {
            // An unproven owner still names its session: a target of that
            // name is refused as the caller's own rather than guessed apart.
            Ok(Some(session))
        }
        Err(CallerGap::PaneUnreadable) => Err((
            EXIT_FAILED,
            "tmux did not report this caller's pane, so ae cannot tell which session it is in.",
        )),
        Err(CallerGap::NotInTmux | CallerGap::NoPane) => Err((
            EXIT_FAILED,
            "ae cannot tell which session this caller is in.",
        )),
    }
}

/// What proving the bound identity under the lifecycle lock found.
enum Proof {
    /// The live session to kill: its server and its exact `$<n>` id.
    Proven { server: ServerId, id: String },
    /// The recorded session is this uuid and provably not running.
    Absent,
    /// The refusal, one clause; nothing may be written.
    Refused(String),
}

/// Prove `name` is the session `uuid`, reading only: the raw directory, the
/// raw meta, the recorded server, and the live id with its stamp.
fn prove(dir: &Path, name: &str, uuid: &str) -> Proof {
    match crate::teardown::classify_dir(dir) {
        crate::teardown::DirKind::RealDir => {}
        crate::teardown::DirKind::Symlink => {
            return Proof::Refused(format!(
                "'{name}' is a rename alias, not a session; a bound stop names the session by its current name"
            ));
        }
        crate::teardown::DirKind::Absent => {
            return Proof::Refused(format!("no session named '{name}' is recorded"));
        }
        crate::teardown::DirKind::NotDir | crate::teardown::DirKind::StatErr(_) => {
            return Proof::Refused(format!("'{name}' is not a readable session directory"));
        }
    }
    let Ok(bytes) = meta::read_bytes(dir) else {
        return Proof::Refused(format!("ae cannot read the metadata of '{name}'"));
    };
    let recorded = meta::sole_value(&bytes, "session_id")
        .map(|value| crate::archive::canonical_uuid(&String::from_utf8_lossy(value)))
        .filter(|value| !value.is_empty());
    let Some(recorded) = recorded else {
        return Proof::Refused(format!("'{name}' records no single session uuid"));
    };
    if recorded != uuid {
        return Proof::Refused(format!("'{name}' is session {recorded}, not {uuid}"));
    }
    let ServerSelector::Positive(selector) = server_of(&bytes) else {
        return Proof::Refused(format!(
            "'{name}' has no positive server record, so ae cannot tell which tmux server owns it"
        ));
    };
    let server = ServerId::Selected(selector);
    let Some(live) = transport::observe_session_stamp(&server, name) else {
        return match transport::verify_session_absent(&server, name) {
            StopProbe::Absent => Proof::Absent,
            StopProbe::Present | StopProbe::Unknown => Proof::Refused(format!(
                "ae cannot read session '{name}' on its recorded tmux server"
            )),
        };
    };
    let stamp = crate::archive::canonical_uuid(&live.uuid);
    if stamp.is_empty() {
        return Proof::Refused(format!(
            "tmux session '{name}' ({}) carries no ae session uuid, so ae cannot prove it is session {uuid}",
            live.id
        ));
    }
    if stamp != uuid {
        return Proof::Refused(format!(
            "tmux session '{name}' ({}) is session {stamp}, not {uuid}",
            live.id
        ));
    }
    Proof::Proven {
        server,
        id: live.id,
    }
}

/// Under the held event lock: every seat of the CURRENT roster has no
/// declaration or a latest one ae admits. `Err` names each other seat with its
/// state, or the journal ae could not read.
fn seats_idle(dir: &Path, name: &str, hold: &crate::store::EventsHold) -> Result<(), String> {
    let Ok(bytes) = meta::read_bytes(dir) else {
        return Err(format!("ae cannot read the roster of '{name}'"));
    };
    let parsed = meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let actors: Vec<String> = parsed
        .roster()
        .iter()
        .map(meta::RosterEntry::reference)
        .collect();
    let container = match hold.read() {
        SourceRead::Absent => Vec::new(),
        SourceRead::Ready(bytes) => bytes,
        SourceRead::Invalid(what) => {
            return Err(format!(
                "the declaration journal {} is {what}, so ae cannot read the seats' states",
                hold.path().display()
            ));
        }
        SourceRead::Unreadable(why) => {
            return Err(format!(
                "ae cannot read the declaration journal {}: {why}",
                hold.path().display()
            ));
        }
    };
    let busy: Vec<String> = crate::state::latest_for_all(&container, &actors)
        .into_iter()
        .filter_map(|(actor, latest)| {
            let value = String::from_utf8_lossy(&latest.value).into_owned();
            (!IDLE_STATES.contains(&value.as_str())).then(|| format!("{actor} is {value}"))
        })
        .collect();
    if busy.is_empty() {
        return Ok(());
    }
    Err(format!(
        "'{name}' has seats that are not idle ({}); a bound stop waits until each seat declares done or waiting-user",
        busy.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_stamp_listing_reads_the_session_uuid_option() {
        assert!(
            crate::tmux::SESSION_STAMP_FORMAT
                .ends_with(&format!("#{{{}}}", crate::theme::SESSION_ID_OPTION))
        );
    }

    #[test]
    fn only_done_and_waiting_user_are_idle() {
        let idle: Vec<&str> = crate::state::VALUES
            .into_iter()
            .filter(|value| super::IDLE_STATES.contains(value))
            .collect();
        assert_eq!(idle, ["waiting-user", "done"]);
    }
}
