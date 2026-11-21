//! `rename <old> <new> --expect-uuid <uuid> [--owner-input <ask-id>]`: the
//! bound rename, the one rename the orchestrator seat runs on another session.
//!
//! It renames a session only when its name resolves, under both lifecycle
//! locks, to exactly the session uuid the caller read from the fleet. The
//! order is the contract:
//!
//! 1. before any lock (the entry gate already refused an ordinary agent
//!    seat's rename of another session): the entry's ONE observation of the
//!    calling pane and the class its gate judged, reused here with the strict
//!    marker checks. A pane in the orchestrator seat's session that fails the
//!    dispatcher proof is refused; the proved seat is refused its own session,
//!    by name or by uuid, with the owner's exact command; `--owner-input` is
//!    the proved seat's alone, and its id must be stored owner words in the
//!    seat's own journal that name `rename` and the old name;
//! 2. both lifecycle locks, name-sorted and held to the end. An ordinary
//!    agent seat the entry gate admitted must still own the old session
//!    under them, or it is refused as the gate refuses it. With no write:
//!    the new name is no node of any kind in the sessions directory, so a
//!    rename alias is never reclaimed; the old name is a real session
//!    directory whose meta names exactly that uuid on a positive server, and
//!    ONE isolated read proves the live session's server and session
//!    incarnation and its stamp, or its positive absence; no tmux session
//!    holds the new name;
//! 3. the old session's event lock is taken and held: every seat's latest
//!    declaration is `done` or `waiting-user` (a seat that never declared
//!    too), or a busy one the owner input waives, and no pending request
//!    would be stranded;
//! 4. the proved seat's dispatch request in its OWN journal: a failed append,
//!    or an owner input an earlier request already used, refuses;
//! 5. a live session, still holding: the fenced rename of the proved
//!    incarnation, ONE read of the new name that must show the same
//!    incarnation and stamp, the state directory move; released, the plain
//!    rename's publication (meta, manifest, rename record, monitors, chats,
//!    look, the alias). A stopped session: the stopped transaction takes the
//!    event lock over; its preflight proves a positive absence (never the
//!    boot-time fallback, also on a recovered retry) and the pending
//!    requests again, the moves run under the lock, and the transaction
//!    releases it before its meta step, the lock order of an archive
//!    publisher (meta, memo, events);
//! 6. the dispatch result, written before anything reaches the caller.

use std::io::Write;
use std::path::Path;

use crate::inventory::ServerId;
use crate::lifecycle::bound::{self as guard, Class, Proof, Verb};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::store::{EventsHold, SourcePresence};
use crate::tmux::SessionStamp;

/// The bound form's usage.
pub(super) const USAGE: &str =
    "Usage: ae rename <old-name> <new-name> --expect-uuid <uuid> [--owner-input <ask-id>]";

/// The word that binds the rename to the session uuid.
const EXPECT_UUID: &str = "--expect-uuid";

/// The word that cites the owner's stored words.
const OWNER_INPUT: &str = "--owner-input";

/// The bound rename as its refusals spell it.
const RENAME: Verb = Verb {
    word: "rename",
    gerund: "renaming",
    nothing: "Nothing was renamed.",
};

/// The bound rename's argv, as [`bound_form`] read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    /// The session's current name.
    pub(crate) old: String,
    /// The name it takes.
    pub(crate) new: String,
    /// The canonical session uuid the caller read from the fleet.
    pub(crate) uuid: String,
    /// `--owner-input <ask-id>`: the orchestrator seat's stored owner words
    /// that approve this rename of a busy session.
    pub(crate) owner_input: Option<String>,
}

/// The bound form of a `rename` tail: `None` when the tail names neither
/// `--expect-uuid` nor `--owner-input` (the plain rename), else exactly two
/// names, one canonical `--expect-uuid` and at most one `--owner-input`, in
/// any order, or the clause that refuses the tail.
///
/// # Errors
///
/// Inside the `Some`: the clause for a malformed bound tail.
pub(crate) fn bound_form(tail: &[String]) -> Option<Result<Request, String>> {
    tail.iter()
        .any(|word| word == EXPECT_UUID || word == OWNER_INPUT)
        .then(|| parse(tail))
}

/// [`bound_form`] of a tail that names one of its words.
fn parse(tail: &[String]) -> Result<Request, String> {
    let (mut uuid, mut owner_input, mut names) = (None, None, Vec::new());
    let mut words = tail.iter();
    while let Some(word) = words.next() {
        let slot = match word.as_str() {
            EXPECT_UUID => &mut uuid,
            OWNER_INPUT => &mut owner_input,
            flag if flag.starts_with('-') => {
                return Err(format!(
                    "'{}' is not a word of the bound rename.",
                    flag.escape_debug()
                ));
            }
            name => {
                names.push(name.to_owned());
                continue;
            }
        };
        if slot.is_some() {
            return Err(format!("{word} may be given only once."));
        }
        let Some(value) = words.next() else {
            return Err(format!("{word} requires a value."));
        };
        *slot = Some(value.clone());
    }
    let Some(raw) = uuid else {
        return Err(format!("{OWNER_INPUT} travels only with {EXPECT_UUID}."));
    };
    let uuid = crate::archive::canonical_uuid(&raw);
    if uuid.is_empty() {
        return Err(format!("'{}' is not a session uuid.", raw.escape_debug()));
    }
    let Ok([old, new]) = <[String; 2]>::try_from(names) else {
        return Err("a bound rename names the session and its new name, once each.".to_owned());
    };
    Ok(Request {
        old,
        new,
        uuid,
        owner_input,
    })
}

/// G1: the orchestrator seat never renames the session it runs in; the
/// owner's exact command is the remedy.
pub(crate) fn own_session_refusal(own: &str, new: &str) -> String {
    format!(
        "ae: rename REFUSED — '{own}' is the orchestrator seat's own session, and the seat never renames it. The owner runs, in the owner's own terminal: ae rename {own} {new}. Nothing was renamed."
    )
}

/// The bound rename of `request`, its names already valid for a rename.
/// `crash` is the plain rename's armed crash seam; `entry` is the entry's
/// ONE reading of the calling pane and its class (`None`: this worker reads
/// the pane itself), and `tail` the words the caller typed.
///
/// # Errors
///
/// A write failure on the caller's streams.
#[allow(
    clippy::too_many_lines,
    reason = "one ordered bound transaction: caller, G1, owner input, locks, the seat's own proof, admission, audit, effect and result read top to bottom"
)]
pub(super) fn run(
    root: &Path,
    request: &Request,
    crash: Option<&str>,
    entry: Option<&super::Entry>,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    let (old, new) = (request.old.as_str(), request.new.as_str());
    if old == new {
        writeln!(
            err,
            "Error: '{old}' is already the session's name; a bound rename names a new one. Nothing was renamed."
        )?;
        return Ok(EXIT_USAGE);
    }
    let reading = entry.map_or(guard::Reading::Own, |entry| {
        guard::Reading::Entry(entry.seen.as_ref())
    });
    let caller = match guard::caller_session_from(root, &[], reading) {
        Ok(caller) => caller,
        Err((code, line)) => {
            writeln!(err, "Error: {line} Nothing was renamed.")?;
            return Ok(code);
        }
    };
    let judged = entry.map(|entry| entry.class.as_ref().map_or(Class::Other, guard::class_of));
    let seen = guard::observe_as(root, &caller, judged);
    let seat = match &seen.class {
        Class::Unproven(why) => {
            writeln!(
                err,
                "ae: rename REFUSED — this pane is in the orchestrator seat's session but ae cannot prove it is the seat ({why}). Nothing was renamed."
            )?;
            return Ok(EXIT_FAILED);
        }
        Class::Seat(seat) => Some(seat),
        Class::Other => None,
    };
    // G1, by name or by uuid; the remedy names the session ae proved the seat
    // runs in, never the requested name.
    if let Some(seat) = seat
        && (seat.session == old || seat.uuid == request.uuid)
    {
        writeln!(err, "{}", own_session_refusal(&seat.session, new))?;
        return Ok(EXIT_FAILED);
    }
    let owner_input = request.owner_input.as_deref();
    let Some(approved) = guard::owner_approval(seat, RENAME, old, owner_input, err)? else {
        return Ok(EXIT_FAILED);
    };
    let (first, second) = if old < new { (old, new) } else { (new, old) };
    let locks = crate::lifecycle::lock(root, first)
        .and_then(|one| crate::lifecycle::lock(root, second).map(|two| (one, two)));
    let Ok(pair) = locks else {
        writeln!(
            err,
            "Error: another lifecycle operation (start/resume/stop/end) is in progress for '{old}' or '{new}' — retry shortly. Nothing was renamed."
        )?;
        return Ok(EXIT_FAILED);
    };
    let ordinary = entry.and_then(super::Entry::seat);
    if let Some(pane) = ordinary.as_ref()
        && !super::owns(root, old, pane)
    {
        drop(pair);
        return Ok(super::seat_refusal(root, old, pane, tail, err)?);
    }
    let Admitted { server, proved } = match admit(root, request, crash, approved) {
        Ok(admitted) => admitted,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(EXIT_FAILED);
        }
    };
    // The dispatch request is written while the old session's event lock is
    // held, so nothing the admission read can change before the effect.
    let dispatch = seat.map(|seat| dispatch_of(seat, request));
    if let Some(dispatch) = &dispatch
        && let Err(why) = dispatch.request()
    {
        writeln!(
            err,
            "ae: rename REFUSED — ae could not record the dispatch in the orchestrator's journal ({why}). Nothing was renamed."
        )?;
        return Ok(EXIT_FAILED);
    }
    // Both streams are buffered, so the dispatch result is settled before
    // anything reaches a caller whose stream may be closed.
    let (mut held_out, mut held_err) = (Vec::new(), Vec::new());
    let code = match proved {
        Proved::Live(live) => renamed_live(
            root,
            request,
            &server,
            (*live, seen.actor()),
            &mut held_out,
            &mut held_err,
        )?,
        // The stopped transaction takes the event lock over: its fresh strict
        // absence, its pending read and the durable moves run under it, and it
        // releases the lock before its meta step.
        Proved::Stopped(hold) => super::stopped(
            root,
            old,
            new,
            &server,
            (crash, seen.actor()),
            super::Absence::Strict,
            &mut Some(hold),
            &mut held_out,
            &mut held_err,
        )?,
    };
    if let Some(dispatch) = &dispatch
        && let Err(why) = dispatch.result(&crate::tracked::dispatch_outcome(code))
    {
        writeln!(
            held_err,
            "Warning: '{old}': ae could not record the dispatch result in the orchestrator's journal ({why})."
        )?;
    }
    let output = crate::error::with_status(code, out.write_all(&held_out));
    err.write_all(&held_err)?;
    output?;
    Ok(code)
}

/// What [`admit`] proved under both lifecycle locks.
struct Admitted {
    /// The old session's recorded server.
    server: ServerId,
    /// Whether it runs, with the old session's event lock held either way.
    proved: Proved,
}

/// The old session as [`admit`] proved it.
enum Proved {
    /// Live: its proved identity and look, and the held event lock; boxed,
    /// because the look is the one large reading.
    Live(Box<Live>),
    /// Positively not running on its recorded server.
    Stopped(EventsHold),
}

/// A live session's proved identity, the look read before any mutation and
/// the event lock held over the effect.
struct Live {
    stamp: SessionStamp,
    look: Option<crate::theme::Look>,
    hold: EventsHold,
}

/// Steps 2 and 3 under both lifecycle locks, all without a write: the free
/// new name, the proved old session, then its event lock, the declarations
/// and the pending requests. `Err` is the whole refusal line.
fn admit(
    root: &Path,
    request: &Request,
    crash: Option<&str>,
    approved: bool,
) -> Result<Admitted, String> {
    let (old, new) = (request.old.as_str(), request.new.as_str());
    let refused = |line: String| format!("Error: {line}. Nothing was renamed.");
    let sessions = crate::lifecycle::sessions_dir(root);
    let old_dir = sessions.join(old);
    if !matches!(
        crate::store::source_presence(&sessions.join(new)),
        SourcePresence::Absent
    ) {
        return Err(refused(format!(
            "'{new}' is already taken in the sessions directory (a session, a rename alias or another entry), and a bound rename never reclaims or overwrites it"
        )));
    }
    let (server, stamp) = match guard::prove(&old_dir, old, &request.uuid, RENAME.word) {
        Proof::Proven { server, stamp } => (server, Some(stamp)),
        Proof::Absent { server } => (server, None),
        Proof::Refused(line) => return Err(refused(line)),
    };
    if crate::transport::session_exists(&server, new) {
        return Err(refused(format!("session '{new}' already exists")));
    }
    // The crash seam is a stopped-rename instrument.
    if stamp.is_some()
        && let Err(refusal) = super::armed_live(crash, old)
    {
        return Err(refusal.0);
    }
    // The look is read before any mutation, as the plain live rename reads it.
    let look = stamp
        .as_ref()
        .and_then(|_| crate::session_launch::look_of(&server, old));
    let hold = crate::store::open(&old_dir)
        .hold_events()
        .map_err(|why| refused(why.to_string()))?;
    guard::seats_idle(&old_dir, old, &hold, approved, RENAME.word)
        .and_then(|()| super::stranding(root, old, &old_dir))
        .map_err(refused)?;
    let proved = match stamp {
        Some(stamp) => Proved::Live(Box::new(Live { stamp, look, hold })),
        None => Proved::Stopped(hold),
    };
    Ok(Admitted { server, proved })
}

/// Step 5 for a live session: the fenced rename of the proved incarnation,
/// ONE read of the new name that must show that incarnation and stamp, and
/// the state directory move, all under the event lock; then the plain
/// rename's publication without it, because the rename record is appended
/// to the moved journal.
fn renamed_live(
    root: &Path,
    request: &Request,
    server: &ServerId,
    (live, actor): (Live, &str),
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    let (old, new) = (request.old.as_str(), request.new.as_str());
    let Live { stamp, look, hold } = live;
    let renamed = crate::transport::rename_fenced(server, &stamp, new)
        && crate::transport::observe_session_stamp(server, new).as_ref() == Some(&stamp);
    if !renamed {
        writeln!(err, "{}", fence_refusal(server, request, &stamp))?;
        return Ok(EXIT_FAILED);
    }
    let sessions = crate::lifecycle::sessions_dir(root);
    if let Err(refusal) = super::move_state_dir(&sessions.join(old), &sessions.join(new), old, new)
    {
        return super::refuse(err, &refusal);
    }
    drop(hold);
    super::republish(root, server, (old, new), (look.as_ref(), actor), out, err)
}

/// The refusal when the new name does not read back as the proved session.
/// While the old name still reads back as exactly the proved session, tmux
/// moved nothing; otherwise ae cannot tell where the proved session went,
/// and says so.
fn fence_refusal(server: &ServerId, request: &Request, stamp: &SessionStamp) -> String {
    let (old, new, uuid) = (&request.old, &request.new, &request.uuid);
    if crate::transport::observe_session_stamp(server, old).as_ref() == Some(stamp) {
        format!(
            "Error: session '{old}' still reads back as the session {uuid} ({}) that ae proved, and '{new}' does not, so the fenced rename did not move it. Nothing was renamed.",
            stamp.session.id
        )
    } else {
        format!(
            "Error: the fenced rename of '{old}' could not be confirmed: neither '{old}' nor '{new}' reads back as the session {uuid} ({}). Its state directory was not moved; check the tmux session before any retry.",
            stamp.session.id
        )
    }
}

/// The proved seat's audit of this bound rename (G4), not yet written: the
/// summary names both names.
fn dispatch_of(seat: &crate::relay::Dispatcher, request: &Request) -> crate::tracked::Dispatch {
    crate::tracked::Dispatch {
        dir: seat.dir.clone(),
        session: seat.session.clone(),
        uuid: seat.uuid.clone(),
        slot: seat.slot.clone(),
        pane: seat.pane.clone(),
        server: seat.server.clone(),
        reference: crate::tracked::Dispatch::reference(
            crate::time::Timestamp::now(),
            guard::entropy(),
        ),
        verb: "rename",
        target: format!("{} {}", request.old, request.new),
        target_session: request.old.clone(),
        target_uuid: request.uuid.clone(),
        owner_input: request.owner_input.clone(),
        requested: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{Request, bound_form};

    const UUID: &str = "11111111-2222-4333-8444-555555555555";

    fn tail(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    /// Two names, one canonical uuid and at most one owner input, in any
    /// order; the uuid is read in its canonical lower case.
    #[test]
    fn the_bound_form_takes_two_names_one_uuid_and_one_owner_input_in_any_order() {
        let read = |input: Option<&str>| {
            Some(Ok(Request {
                old: "a".to_owned(),
                new: "b".to_owned(),
                uuid: UUID.to_owned(),
                owner_input: input.map(ToOwned::to_owned),
            }))
        };
        assert_eq!(
            bound_form(&tail(&["a", "b", "--expect-uuid", UUID])),
            read(None)
        );
        assert_eq!(
            bound_form(&tail(&["--expect-uuid", UUID, "a", "b"])),
            read(None)
        );
        assert_eq!(
            bound_form(&tail(&[
                "a",
                "b",
                "--expect-uuid",
                &UUID.to_ascii_uppercase()
            ])),
            read(None)
        );
        assert_eq!(
            bound_form(&tail(&[
                "a",
                "--owner-input",
                "i",
                "b",
                "--expect-uuid",
                UUID
            ])),
            read(Some("i"))
        );
    }

    /// A tail without either word is the plain rename; a bound tail that is
    /// malformed is refused with the clause that says why.
    #[test]
    fn a_plain_tail_is_no_bound_form_and_each_malformed_bound_tail_says_why() {
        assert_eq!(bound_form(&tail(&["a", "b"])), None);
        assert_eq!(bound_form(&tail(&["b"])), None);
        assert_eq!(bound_form(&tail(&["a", "b", "-y"])), None);
        let names = "a bound rename names the session and its new name, once each.";
        for (words, why) in [
            (
                vec!["a", "b", "--expect-uuid"],
                "--expect-uuid requires a value.",
            ),
            (
                vec!["a", "b", "--owner-input"],
                "--owner-input requires a value.",
            ),
            (
                vec!["a", "b", "--owner-input", "i"],
                "--owner-input travels only with --expect-uuid.",
            ),
            (
                vec!["a", "b", "--expect-uuid", UUID, "--expect-uuid", UUID],
                "--expect-uuid may be given only once.",
            ),
            (
                vec![
                    "a",
                    "b",
                    "--expect-uuid",
                    UUID,
                    "--owner-input",
                    "i",
                    "--owner-input",
                    "j",
                ],
                "--owner-input may be given only once.",
            ),
            (
                vec!["a", "b", "--expect-uuid", "nope"],
                "'nope' is not a session uuid.",
            ),
            (vec!["a", "--expect-uuid", UUID], names),
            (vec!["a", "b", "c", "--expect-uuid", UUID], names),
            (vec!["--expect-uuid", UUID], names),
            (
                vec!["a", "b", "--expect-uuid", UUID, "-y"],
                "'-y' is not a word of the bound rename.",
            ),
        ] {
            assert_eq!(
                bound_form(&tail(&words)),
                Some(Err(why.to_owned())),
                "{words:?}"
            );
        }
    }
}
