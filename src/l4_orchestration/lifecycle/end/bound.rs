//! `end <name> --keep-history --expect-uuid <uuid> [--owner-input <ask-id>]`:
//! the bound end, the one end the orchestrator seat may run. It always keeps
//! the history; purging stays the owner's.
//!
//! The order is the contract:
//!
//! 1. before any lock: the entry's ONE observation of the calling pane and
//!    the class its gate judged, reused here with the strict marker checks. A
//!    pane in the orchestrator seat's session that fails the dispatcher proof
//!    is refused; the proved seat is refused its own session by name or by
//!    uuid; the caller's own session and the fleet word `all` are refused;
//!    `--owner-input` is the proved seat's alone, and its id must be stored
//!    owner words that name `end` and this session;
//! 2. under the session's lifecycle lock, with no write: the name is a real
//!    session directory whose meta names exactly that uuid on a positive
//!    server, and ONE isolated read proves the live incarnation and its
//!    `@ae_session_uuid` stamp, or a strict probe proves it is not running;
//!    then the read-only checks of the end itself (the retained snapshot
//!    nodes, the Keep plan, an archive retry, the working directory of a
//!    managed mode), so no refusal of those comes after the stop;
//! 3. the session's event lock is taken and held. For the proved seat, the
//!    admission: its own joined create or start of this uuid (a request and
//!    its `ok` result with one reference, in its own journal, never by
//!    name), else every seat of the current roster declares `done` and no
//!    request is pending in the session, else the owner input from step 1;
//!    the owner outside tmux needs none. Then the seat's dispatch request
//!    (a failed append, or an owner input an earlier request used, refuses);
//! 4. still holding it: the fenced client handoff and the fenced, verified
//!    kill of the proved incarnation; then the event lock is released;
//! 5. the end of the stopped session as an ordinary Keep end runs it, with no
//!    second kill. Under the archive's source holds (meta, memo, events) the
//!    admission that admitted it is judged again, so a request written while
//!    the end waited refuses before anything is published; a refusal from
//!    here on names the stopped, registered session and how to resume it;
//! 6. the seat's dispatch result.

use std::io::{self, Write};
use std::path::Path;

use super::{
    Action, commit_pending, early_finish, nonlossless_workdir, preflight_refused, push_work,
    resolve_plan, resolve_workdir, transaction, workdir_refused,
};
use crate::lifecycle::bound::{self as guard, Class, Proof, Verb};
use crate::lifecycle::{lock, meta_value, name_is_usable, sessions_dir};
use crate::meta;
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::store::{EventsHold, SourceRead};

/// The uuid word of the bound end.
const EXPECT_UUID: &str = "--expect-uuid";

/// The owner-input word of the bound end.
const OWNER_INPUT: &str = "--owner-input";

/// The history word every bound end names.
const KEEP_HISTORY: &str = "--keep-history";

/// The bound end's one admitted form, as its refusals name it.
pub(crate) const FORM: &str = "ae end <name> --keep-history --expect-uuid <uuid>";

/// The bound end as owner words and refusals spell it.
const END: Verb = Verb {
    word: "end",
    gerund: "ending",
    nothing: "Nothing was ended.",
};

/// The bound end's argv, as [`bound_form`] read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    /// The session name.
    pub(crate) name: String,
    /// The canonical session uuid the caller read from the fleet.
    pub(crate) uuid: String,
    /// `--owner-input <ask-id>`: the orchestrator seat's stored owner words
    /// that approve this end.
    pub(crate) owner_input: Option<String>,
}

/// The bound form of an `_end` tail: `None` when it names neither
/// `--expect-uuid` nor `--owner-input`, else the request or why the tail is
/// not the one admitted form. An `rm` never has a bound form.
pub(crate) fn bound_form(tail: &[String]) -> Option<Result<Request, String>> {
    tail.iter()
        .any(|word| word == EXPECT_UUID || word == OWNER_INPUT)
        .then(|| parse(tail))
}

/// [`bound_form`] of a tail that names one of its words.
fn parse(tail: &[String]) -> Result<Request, String> {
    let (mut uuid, mut owner_input, mut keep, mut names) = (None, None, false, Vec::new());
    let mut words = tail.iter();
    while let Some(word) = words.next() {
        let slot = match word.as_str() {
            super::RM_MARKER => {
                return Err(format!(
                    "ae rm has no bound form; the one bound end is {FORM}."
                ));
            }
            KEEP_HISTORY if keep => return Err(format!("{KEEP_HISTORY} may be given only once.")),
            KEEP_HISTORY => {
                keep = true;
                continue;
            }
            EXPECT_UUID => &mut uuid,
            OWNER_INPUT => &mut owner_input,
            "--purge-history" => {
                return Err(format!(
                    "a bound end keeps the history, and purging it is the owner's: {FORM}."
                ));
            }
            flag if flag.starts_with('-') => {
                return Err(format!(
                    "'{}' is not a word of the bound end, which is {FORM}.",
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
    if !keep {
        return Err(format!("a bound end names {KEEP_HISTORY}: {FORM}."));
    }
    let Ok([name]) = <[String; 1]>::try_from(names) else {
        return Err("a bound end names one session, once.".to_owned());
    };
    Ok(Request {
        name,
        uuid,
        owner_input,
    })
}

/// The bound end of `request`. `entry` is the entry's ONE reading of the
/// calling pane and its class (`None`: this worker reads the pane itself).
#[allow(
    clippy::too_many_lines,
    reason = "one ordered bound end: caller, G1, owner input, lock, proof, the read-only end checks, admission, audit, effect and result read top to bottom"
)]
pub(crate) fn run(
    root: &Path,
    request: &Request,
    reason: &str,
    entry: Option<&crate::rename::Entry>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let (name, uuid) = (request.name.as_str(), request.uuid.as_str());
    if name == "all" {
        writeln!(
            err,
            "Error: 'all' is the fleet word; a bound end names one session, so it never ends the fleet. Nothing was ended."
        )?;
        return Ok(EXIT_USAGE);
    }
    let reading = entry.map_or(guard::Reading::Own, |entry| {
        guard::Reading::Entry(entry.seen.as_ref())
    });
    let caller = match guard::caller_session_from(root, &[], reading) {
        Ok(caller) => caller,
        Err((code, line)) => {
            writeln!(err, "Error: {line} Nothing was ended.")?;
            return Ok(code);
        }
    };
    let judged = entry.map(|entry| entry.class.as_ref().map_or(Class::Other, guard::class_of));
    let seen = guard::observe_as(root, &caller, judged);
    let seat = match &seen.class {
        Class::Unproven(why) => {
            writeln!(
                err,
                "ae: end REFUSED — this pane is in the orchestrator seat's session but ae cannot prove it is the seat ({why}). Nothing was ended."
            )?;
            return Ok(EXIT_FAILED);
        }
        Class::Seat(seat) => Some(seat),
        Class::Other => None,
    };
    // G1, by name or by uuid; the remedy names the session ae proved the seat
    // runs in, never the requested name.
    if let Some(seat) = seat
        && (seat.session == name || seat.uuid == uuid)
    {
        writeln!(err, "{}", own_session_refusal(&seat.session))?;
        return Ok(EXIT_FAILED);
    }
    if caller.session() == Some(name) {
        writeln!(
            err,
            "Error: '{name}' is the session you are in; a bound end never ends its caller's own session (that is ae end {name}). Nothing was ended."
        )?;
        return Ok(EXIT_FAILED);
    }
    let owner_input = request.owner_input.as_deref();
    let Some(approved) = guard::owner_approval(seat, END, name, owner_input, err)? else {
        return Ok(EXIT_FAILED);
    };
    if !name_is_usable(root, name) {
        writeln!(err, "ae: '{name}' is not a usable session name.")?;
        return Ok(EXIT_FAILED);
    }
    let Ok(_guard) = lock(root, name) else {
        writeln!(
            err,
            "Error: another lifecycle operation (start/resume/stop/end) is in progress for '{name}' — retry shortly. Nothing was ended."
        )?;
        return Ok(EXIT_FAILED);
    };
    if let Some(blocked) = crate::rename::intent_blocks(root, name) {
        writeln!(err, "Error: {blocked}. Nothing was ended.")?;
        return Ok(EXIT_FAILED);
    }
    let dir = sessions_dir(root).join(name);
    let (server, stamp) = match guard::prove(&dir, name, uuid, END.word) {
        Proof::Proven { server, stamp } => (server, Some(stamp)),
        Proof::Absent { server } => (server, None),
        Proof::Refused(line) => {
            writeln!(err, "Error: {line}. Nothing was ended.")?;
            return Ok(EXIT_FAILED);
        }
    };
    let Some(checked) = checked_before_stop(root, name, &dir, err)? else {
        return Ok(EXIT_FAILED);
    };
    let hold = match crate::store::open(&dir).hold_events() {
        Ok(hold) => hold,
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was ended.")?;
            return Ok(EXIT_FAILED);
        }
    };
    let admission = match seat {
        Some(seat) => match admit(seat, &dir, name, uuid, &hold, approved) {
            Ok(admission) => admission,
            Err(line) => {
                writeln!(err, "ae: end REFUSED — {line}. Nothing was ended.")?;
                return Ok(EXIT_FAILED);
            }
        },
        None => Admission::Owner,
    };
    // The dispatch request is written while the session's event lock is held,
    // so nothing the admission read can change before the effect.
    let dispatch = seat.map(|seat| dispatch_of(seat, request));
    if let Some(dispatch) = &dispatch
        && let Err(why) = dispatch.request()
    {
        writeln!(
            err,
            "ae: end REFUSED — ae could not record the dispatch in the orchestrator's journal ({why}). Nothing was ended."
        )?;
        return Ok(EXIT_FAILED);
    }
    // Both streams are buffered, so the dispatch result is settled before
    // anything reaches a caller whose stream may be closed.
    let (mut held_out, mut held_err) = (Vec::new(), Vec::new());
    let observed = entry.map_or_else(|| "human".to_owned(), crate::rename::Entry::actor);
    let who = crate::lifecycle::Stopper {
        actor: guard::fleet_actor(seen.actor(), &observed),
        reason: reason.to_owned(),
    };
    let stopped = Stopped {
        root,
        request,
        who: &who,
        server: &server,
        stamp: stamp.as_ref(),
        checked,
        admission: &admission,
    };
    // An error after the admission is still a settled, non-ok dispatch, and
    // its diagnostic still reaches the caller.
    let code = match stopped.end(hold, &mut held_out, &mut held_err) {
        Ok(code) => code,
        Err(why) => {
            writeln!(
                held_err,
                "Error: ae could not finish ending '{name}': {why}. Run 'ae list --all' to see what is left, and 'ae doctor' for a pending end."
            )?;
            EXIT_FAILED
        }
    };
    if let Some(dispatch) = &dispatch
        && let Err(why) = dispatch.result(&crate::tracked::dispatch_outcome(code))
    {
        writeln!(
            held_err,
            "Warning: '{name}': ae could not record the dispatch result in the orchestrator's journal ({why})."
        )?;
    }
    let output = crate::error::with_status(code, out.write_all(&held_out));
    err.write_all(&held_err)?;
    output?;
    Ok(code)
}

/// G1: the orchestrator seat never ends the session it runs in; the owner's
/// exact command is the remedy.
pub(crate) fn own_session_refusal(own: &str) -> String {
    format!(
        "ae: end REFUSED — '{own}' is the orchestrator seat's own session, and the seat never ends it. The owner runs, in the owner's own terminal: ae end {own}. Nothing was ended."
    )
}

/// What the read-only end checks of step 2 resolved.
struct Checked {
    /// The meta the end runs under.
    bytes: Vec<u8>,
    /// The Keep plan.
    plan: super::Plan,
    /// The early finish of a local session or an archive retry, or `None`
    /// for the git path.
    early: Option<(super::Git, &'static str)>,
}

/// Step 2's read-only end checks: the retained snapshot nodes, a Keep plan
/// whatever the session's purge default says, an archive retry, and the
/// working directory of a managed mode. `None` once the refusal is written.
fn checked_before_stop(
    root: &Path,
    name: &str,
    dir: &Path,
    err: &mut impl Write,
) -> io::Result<Option<Checked>> {
    let plan = resolve_plan(root, name, Some(false));
    if preflight_refused(root, dir, &plan, err)? {
        return Ok(None);
    }
    if plan.action != Action::Keep {
        writeln!(
            err,
            "Error: a bound end keeps the session in an archive, and '{name}' cannot be kept ({}). Nothing was ended.",
            plan.detail
        )?;
        return Ok(None);
    }
    let bytes = meta::read_bytes(dir).unwrap_or_default();
    let retry = match transaction::inspect(dir, &bytes) {
        Ok(retry) => retry,
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was ended.")?;
            return Ok(None);
        }
    };
    let early = early_finish(&meta_value(&bytes, "mode"), retry);
    if early.is_none() {
        let work_dir = resolve_workdir(root, &bytes, name);
        let wdir = work_dir.as_os_str().as_encoded_bytes().to_vec();
        if workdir_refused(&work_dir, &wdir, err)? {
            return Ok(None);
        }
    }
    Ok(Some(Checked { bytes, plan, early }))
}

/// How the bound end was admitted, judged again under the archive holds.
enum Admission {
    /// The owner outside tmux: no admission.
    Owner,
    /// The proved seat's own joined create or start of this uuid.
    Granted,
    /// Stored owner words that name `end` and this session.
    Approved,
    /// Every seat declared `done` and no request was pending.
    AllDone,
}

impl Admission {
    /// The admission again, on the sources as they are now: only the
    /// all-done admission reads anything, because a create, a start and an
    /// owner word cannot be taken back.
    fn recheck(&self, dir: &Path, name: &str) -> Result<(), String> {
        match self {
            Self::Owner | Self::Granted | Self::Approved => Ok(()),
            Self::AllDone => {
                let journal =
                    match crate::store::read_source(&crate::store::open(dir).events_path()) {
                        SourceRead::Absent => Vec::new(),
                        SourceRead::Ready(bytes) => bytes,
                        SourceRead::Invalid(what) => {
                            return Err(format!("the journal of '{name}' is {what}"));
                        }
                        SourceRead::Unreadable(why) => {
                            return Err(format!("ae cannot read the journal of '{name}': {why}"));
                        }
                    };
                all_done(dir, name, &journal).and_then(|()| nothing_pending(dir, name))
            }
        }
    }
}

/// Step 3's admission of the proved seat, under the held event lock.
fn admit(
    seat: &crate::relay::Dispatcher,
    dir: &Path,
    name: &str,
    uuid: &str,
    hold: &EventsHold,
    approved: bool,
) -> Result<Admission, String> {
    if granted(seat, uuid) {
        return Ok(Admission::Granted);
    }
    if approved {
        return Ok(Admission::Approved);
    }
    let journal = match hold.read() {
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
    all_done(dir, name, &journal)?;
    nothing_pending(dir, name)?;
    Ok(Admission::AllDone)
}

/// Whether the proved seat's own journal shows its joined create or start of
/// session `uuid`: an `ok` dispatch result for that uuid and the dispatch
/// request with the same reference and verb, both written by this seat's
/// session uuid. A name never grants anything.
fn granted(seat: &crate::relay::Dispatcher, uuid: &str) -> bool {
    let SourceRead::Ready(bytes) =
        crate::store::read_source(&crate::store::open(&seat.dir).events_path())
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&bytes);
    let rows: Vec<crate::events::Event> = text
        .lines()
        .filter_map(|line| crate::events::Event::parse_line(line).ok())
        .filter(|row| {
            row.actor == crate::tracked::ORCHESTRATOR_DISPLAY
                && row.actor_session_id.value() == Some(seat.uuid.as_str())
        })
        .collect();
    rows.iter()
        .filter(|row| {
            row.action == crate::tracked::DISPATCH_RESULT
                && row.target_session_uuid.as_deref() == Some(uuid)
        })
        .filter_map(|row| {
            let verb = grant_verb(row.summary.as_deref()?)?;
            row.summary
                .as_deref()?
                .ends_with(": ok")
                .then_some((row.reference.as_deref()?, verb))
        })
        .any(|(reference, verb)| {
            rows.iter().any(|row| {
                row.action == crate::tracked::DISPATCH_REQUEST
                    && row.reference.as_deref() == Some(reference)
                    && row
                        .summary
                        .as_deref()
                        .and_then(grant_verb)
                        .is_some_and(|said| said == verb)
                    && row
                        .target_session_uuid
                        .as_deref()
                        .is_none_or(|said| said.is_empty() || said == uuid)
            })
        })
}

/// The verb of a dispatch summary when it is one that grants an end.
fn grant_verb(summary: &str) -> Option<&str> {
    summary
        .split_once(' ')
        .map(|(verb, _)| verb)
        .filter(|verb| matches!(*verb, "create" | "start"))
}

/// Every seat of the current roster declares `done`, by its newest
/// declaration in `journal`; a seat that never declared is not done.
fn all_done(dir: &Path, name: &str, journal: &[u8]) -> Result<(), String> {
    let actors = guard::roster_actors(dir, name)?;
    let latest = crate::state::latest_for_all(journal, &actors);
    let open: Vec<String> = actors
        .iter()
        .filter_map(|actor| {
            let shown = actor.escape_debug();
            match latest.iter().find(|(said, _)| said == actor) {
                None => Some(format!("{shown} never declared")),
                Some((_, latest)) if latest.value != b"done" => Some(format!(
                    "{shown} is {}",
                    String::from_utf8_lossy(&latest.value).escape_debug()
                )),
                Some(_) => None,
            }
        })
        .collect();
    if open.is_empty() {
        return Ok(());
    }
    Err(format!(
        "'{name}' has seats that are not done ({}); without its own create or start of the session, the orchestrator ends it only when every seat declares done, or with --owner-input <ask-id>",
        open.join(", ")
    ))
}

/// No request is pending in the session.
fn nothing_pending(dir: &Path, name: &str) -> Result<(), String> {
    let read = crate::session::SessionRead::open(dir)
        .map_err(|why| format!("ae cannot read the requests of '{name}' ({why})"))?;
    if read.pending.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = read
        .pending
        .iter()
        .map(|pending| format!("{} {}", pending.id, pending.action))
        .collect();
    Err(format!(
        "'{name}' has {} pending request(s) ({}); the orchestrator ends it only once they are answered, or with --owner-input <ask-id>",
        ids.len(),
        ids.join(", ")
    ))
}

/// The proved seat's audit of this bound end (G4), not yet written.
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
        verb: "end",
        target: request.name.clone(),
        target_session: request.name.clone(),
        target_uuid: request.uuid.clone(),
        owner_input: request.owner_input.clone(),
        requested: false,
    }
}

/// The record under the archive holds names exactly one session uuid, and it
/// is the one the request proved: a second, conflicting row written after the
/// stop refuses before anything is published.
fn sole_identity(dir: &Path, name: &str, uuid: &str) -> Result<(), String> {
    let bytes = crate::meta::read_bytes(dir)
        .map_err(|why| format!("ae cannot read the record of '{name}': {why}"))?;
    let sole = crate::meta::sole_value(&bytes, "session_id")
        .map(|value| crate::archive::canonical_uuid(&String::from_utf8_lossy(value)));
    if sole.as_deref() == Some(uuid) {
        return Ok(());
    }
    Err(format!(
        "the record of '{name}' no longer names exactly the session {uuid} that ae proved"
    ))
}

/// The session runs nowhere ae can see: its name is absent from its tmux
/// server, and no pane view recorded for its uuid holds its windows. Read
/// only, so nothing is killed by an id the server may since have reused. It
/// is judged after the audit and again under the archive holds, because a
/// session started meanwhile would be archived and unregistered.
fn not_running(server: &crate::inventory::ServerId, name: &str, uuid: &str) -> Result<(), String> {
    match super::sight(server, name) {
        super::TmuxSight::Absent => {}
        super::TmuxSight::Live { .. } => {
            return Err(format!(
                "'{name}' runs on its tmux server again since ae proved it stopped"
            ));
        }
        super::TmuxSight::Unknown => {
            return Err(format!(
                "the tmux server of '{name}' did not answer, so ae cannot prove that '{name}' is not running"
            ));
        }
    }
    let retry = format!("ae end {name} --keep-history --expect-uuid {uuid}");
    match crate::transport::orphan_view_holders(server, uuid) {
        crate::transport::StopViews::Clear => Ok(()),
        crate::transport::StopViews::Held(held) => Err(format!(
            "pane view session(s) {} still hold the windows of '{name}', so its agents still run — state preserved. Close them, then retry '{retry}'",
            held.join(", ")
        )),
        crate::transport::StopViews::Unreadable => Err(format!(
            "ae cannot verify that no pane view of '{name}' still holds its windows — state preserved. Retry '{retry}'"
        )),
    }
}

/// How the Keep end of a stopped session came out.
enum Kept {
    /// Archived and unregistered.
    Ended,
    /// Refused while the session is still proved stopped and registered.
    Stopped,
    /// Refused because the session may run again: the record stays, and ae
    /// does not say it is stopped.
    Unproven,
}

/// Steps 4 and 5: everything the bound end proved, ready for the effect.
struct Stopped<'a> {
    root: &'a Path,
    request: &'a Request,
    /// Who ends the session and why, for the fleet feed.
    who: &'a crate::lifecycle::Stopper,
    server: &'a crate::inventory::ServerId,
    /// The proved live incarnation, or `None` for a session proved stopped.
    stamp: Option<&'a crate::tmux::SessionStamp>,
    checked: Checked,
    admission: &'a Admission,
}

impl Stopped<'_> {
    /// The fenced stop under `hold`, then the Keep end of the stopped session.
    fn end(self, hold: EventsHold, out: &mut impl Write, err: &mut impl Write) -> io::Result<u8> {
        let (name, uuid) = (self.request.name.as_str(), self.request.uuid.as_str());
        if let Some(stamp) = self.stamp {
            if let Some(note) = crate::lifecycle::handoff_clients(self.server, name, Some(stamp)) {
                writeln!(err, "Warning: {note}")?;
            }
            let retry = format!("ae end {name} --keep-history --expect-uuid {uuid}");
            if !crate::lifecycle::kill_verified_with(
                self.server,
                name,
                &retry,
                &stamp.session.id,
                Some(stamp),
                err,
            )? {
                return Ok(EXIT_FAILED);
            }
        } else if let Err(why) = not_running(self.server, name, uuid) {
            writeln!(err, "Error: {why}.")?;
            writeln!(err, "  Nothing was stopped and nothing was deleted.")?;
            return Ok(EXIT_FAILED);
        }
        // The archive's source holds take meta, memo and events in that
        // order, so the event lock is released before them.
        drop(hold);
        // The owner's or the seat's words survive in the archive: a bound end
        // has no request row of its own, so with a reason one is recorded now,
        // ahead of the capture, under the completed-end guard every row takes.
        if !self.who.reason.is_empty() {
            transaction::emit(
                &sessions_dir(self.root).join(name),
                name,
                &crate::archive::canonical_uuid(uuid),
                self.who.journal_actor(),
                super::END_REQUEST_ACTION,
                &crate::lifecycle::with_reason("end requested (bound)", &self.who.reason),
            );
        }
        if let Some(note) = crate::migrate::session_noted(&sessions_dir(self.root).join(name), name)
        {
            writeln!(err, "{note}")?;
        }
        match self.kept(out, err)? {
            Kept::Ended => return Ok(0),
            Kept::Stopped => writeln!(
                err,
                "  '{name}' is stopped and still registered, and nothing was archived; resume it with: ae {name} --resume"
            )?,
            Kept::Unproven => writeln!(
                err,
                "  '{name}' is still registered, and nothing was archived. ae cannot prove it is stopped; see: ae list --all"
            )?,
        }
        Ok(EXIT_FAILED)
    }

    /// The Keep end of the stopped session, with the identity, the absence
    /// and the admission judged again under the archive holds.
    fn kept(&self, out: &mut impl Write, err: &mut impl Write) -> io::Result<Kept> {
        let (root, name) = (self.root, self.request.name.as_str());
        let Checked { bytes, plan, early } = &self.checked;
        let (admission, uuid) = (self.admission, self.request.uuid.as_str());
        let unproven = std::cell::Cell::new(false);
        let veto = |dir: &Path| {
            sole_identity(dir, name, uuid)?;
            not_running(self.server, name, uuid).inspect_err(|_| unproven.set(true))?;
            admission.recheck(dir, name)
        };
        let refused = || {
            if unproven.get() {
                Kept::Unproven
            } else {
                Kept::Stopped
            }
        };
        if let Some((git, ended)) = early {
            if !transaction::finish_vetoed(
                root,
                name,
                bytes,
                plan,
                git,
                Some(&veto),
                self.who,
                out,
                err,
            )? {
                return Ok(refused());
            }
            writeln!(out, "{ended} {name}")?;
            return Ok(Kept::Ended);
        }
        let work_dir = resolve_workdir(root, bytes, name);
        let wdir = work_dir.as_os_str().as_encoded_bytes().to_vec();
        if !commit_pending(&wdir, &work_dir, name, out, err)? {
            return Ok(Kept::Stopped);
        }
        let Some((mut git, mut preserve)) = push_work(&wdir, &work_dir, name, out, err)? else {
            return Ok(Kept::Stopped);
        };
        if !preserve && nonlossless_workdir(&work_dir, name, out)? {
            preserve = true;
            git.preserved = work_dir.display().to_string();
        }
        if !transaction::finish_vetoed(
            root,
            name,
            bytes,
            plan,
            &git,
            Some(&veto),
            self.who,
            out,
            err,
        )? {
            return Ok(refused());
        }
        if preserve {
            writeln!(out, "Ended {name} — working directory preserved.")?;
            writeln!(out, "  Directory preserved: {}", work_dir.display())?;
        } else {
            writeln!(out, "Ended {name}")?;
        }
        Ok(Kept::Ended)
    }
}

#[cfg(test)]
mod tests {
    use super::{Request, bound_form};

    const UUID: &str = "11111111-2222-4333-8444-555555555555";

    fn tail(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn only_the_keep_form_with_one_name_and_one_uuid_is_the_bound_end() {
        assert_eq!(bound_form(&tail(&["t", "--keep-history"])), None);
        assert_eq!(
            bound_form(&tail(&["t", "--keep-history", "--expect-uuid", UUID])),
            Some(Ok(Request {
                name: "t".to_owned(),
                uuid: UUID.to_owned(),
                owner_input: None,
            }))
        );
        assert_eq!(
            bound_form(&tail(&[
                "t",
                "--keep-history",
                "--expect-uuid",
                UUID,
                "--owner-input",
                "ask-1",
            ])),
            Some(Ok(Request {
                name: "t".to_owned(),
                uuid: UUID.to_owned(),
                owner_input: Some("ask-1".to_owned()),
            }))
        );
        for refused in [
            vec!["t", "--expect-uuid", UUID],
            vec!["t", "--purge-history", "--expect-uuid", UUID],
            vec!["t", "--keep-history", "--expect-uuid", UUID, "-f"],
            vec!["t", "--keep-history", "--expect-uuid", UUID, "--supervise"],
            vec!["t", "u", "--keep-history", "--expect-uuid", UUID],
            vec!["t", "--keep-history", "--expect-uuid", "not-a-uuid"],
            vec![
                "t",
                "--keep-history",
                "--keep-history",
                "--expect-uuid",
                UUID,
            ],
            vec![
                super::super::RM_MARKER,
                "t",
                "--keep-history",
                "--expect-uuid",
                UUID,
            ],
            vec!["--bound", "t", "--keep-history", "--expect-uuid", UUID],
            vec!["t", "--keep-history", "--owner-input", "ask-1"],
        ] {
            assert!(
                matches!(bound_form(&tail(&refused)), Some(Err(_))),
                "{refused:?}"
            );
        }
    }
}
