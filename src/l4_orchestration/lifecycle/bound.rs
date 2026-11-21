//! `_stop <name> --expect-uuid <uuid>`: the bound stop, the one stop the
//! orchestrator seat may run.
//!
//! It stops a session only when that name resolves, under the session's
//! lifecycle lock, to exactly the session uuid the caller read from the fleet.
//! The order is the contract:
//!
//! 1. the caller is read from `$TMUX_PANE` alone, and the caller's own session
//!    and the fleet word `all` are refused. ONE observation classes the pane
//!    (`relay::seat_class`): a pane in the orchestrator seat's session that
//!    fails the dispatcher proof is refused; the proved seat is refused its
//!    own session with the owner's exact command; `--owner-input` is the
//!    proved seat's alone, and its id must be stored owner words in the
//!    seat's own journal that name `stop` and this session;
//! 2. under the lifecycle lock, with no write: the name is a real session
//!    directory (never a rename alias), its meta names exactly that uuid, its
//!    record names a positive server, and ONE isolated read of the live
//!    session of that name proves the server incarnation, the session
//!    incarnation and its `@ae_session_uuid` stamp together. Only then does
//!    the proved seat write its dispatch request to its OWN journal (a failed
//!    append, or an owner input an earlier request already used, refuses);
//!    its dispatch result follows the stop;
//! 3. still without a write, each seat's pane text and launch guard are read;
//! 4. the event container's lock is taken and held: the roster is read again
//!    from the current meta, a roster ae doubts or cannot read refuses, and
//!    any seat whose latest declaration is not `done` or `waiting-user`
//!    refuses, except that the seat's owner input that approved this stop
//!    admits `working`, `waiting-agent` and `blocked`, and a journal ae
//!    cannot read refuses;
//! 5. still holding it: the stop request (a failed append refuses), the
//!    client handoff, the kill and the stop result. The tmux server checks
//!    every handoff and kill against the incarnation step 2 proved, in the
//!    same command that acts, so a saved `$<n>` never reaches a session a
//!    replacement server gave the same id;
//! 6. after the release, still under the lifecycle lock: the migration note
//!    and the model publication from step 3, all written to the caller only
//!    after they completed.
//!
//! A target proven already stopped crosses the same guard in step 4 before
//! its receipt is appended through the held lock. A refusal before step 5
//! writes nothing in the target's journal. A declaration either precedes the
//! read in step 4 and is judged there, or waits on the held lock until the
//! kill is done. This process never holds the event lock and the meta lock
//! together, so it keeps the meta-then-events order of every other writer.
//! Both events carry the bound uuid as their target and the caller as ONE
//! observation proved it, or the named gap when it did not. Their actor is
//! `ae/orchestrator` for the proved orchestrator seat, `human` for a caller
//! outside tmux (the owner's own
//! terminal, as that gate reads it), and the neutral `ae:stop` for every
//! other pane, whose facts the correlation fields carry.

use std::io::{self, Write};
use std::path::Path;

use super::{
    ALREADY_STOPPED_SUMMARY, CallerGap, STOP_REQUEST_ACTION, STOP_RESULT_ACTION, StopOutcome,
    caller_correlation, capture_models, kill_under_lock, lock, name_is_usable, publish_models,
    server_of, sessions_dir, stop_summary,
};
use crate::doors::CallerServer;
use crate::inventory::ServerId;
use crate::meta::{self, ServerSelector};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::store::{EventsHold, SourceRead};
use crate::tmux::{SessionStamp, StopProbe};
use crate::tracked::{EventFields, IdentityTriple};
use crate::transport;

/// The declarations a bound stop admits. A seat that never declared is
/// admitted too; every other value, a future one included, refuses.
const IDLE_STATES: [&str; 2] = ["done", "waiting-user"];

/// The busy declarations stored owner words may waive (G3). A value outside
/// these and [`IDLE_STATES`] refuses even an approved stop.
const WAIVABLE_STATES: [&str; 3] = ["working", "waiting-agent", "blocked"];

/// The gap a bound event names when the caller's pane correlates with its
/// session but that session's recorded tmux server is not proved to be the
/// caller's.
const UNPROVED_SERVER: &str = "caller server unproved";

/// The actor of a bound stop run outside tmux: the owner's own terminal.
const HUMAN_ACTOR: &str = "human";

/// The neutral actor of a bound stop run from a pane that is not the proved
/// orchestrator seat: a pane proves where the command ran, not who typed it.
const PANE_ACTOR: &str = "ae:stop";

/// The bound stop's argv, as the one parser read it.
pub(super) struct Request<'a> {
    /// The session name.
    pub(super) name: &'a str,
    /// The canonical session uuid the caller read from the fleet.
    pub(super) uuid: &'a str,
    /// `--owner-input <ask-id>`: the orchestrator seat's stored owner words
    /// that approve this stop of a busy session.
    pub(super) owner_input: Option<&'a str>,
}

/// `_stop <name> --expect-uuid <uuid> [--owner-input <ask-id>]`; `panes` is
/// every `--pane` word of the tail, in order.
pub(super) fn run(
    root: &Path,
    request: &Request<'_>,
    panes: &[Option<String>],
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let (name, uuid) = (request.name, request.uuid);
    if name == "all" {
        writeln!(
            err,
            "Error: 'all' is the fleet word; a bound stop names one session, so it never stops the fleet. Nothing was stopped."
        )?;
        return Ok(EXIT_USAGE);
    }
    let caller = match caller_session(root, panes) {
        Ok(caller) => caller,
        Err((code, line)) => {
            writeln!(err, "Error: {line} Nothing was stopped.")?;
            return Ok(code);
        }
    };
    let seen = observe(root, &caller);
    let seat = match &seen.class {
        Class::Unproven(why) => {
            writeln!(
                err,
                "ae: stop REFUSED — this pane is in the orchestrator seat's session but ae cannot prove it is the seat ({why}). Nothing was stopped."
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
        let own = &seat.session;
        writeln!(
            err,
            "ae: stop REFUSED — '{own}' is the orchestrator seat's own session, and the seat never stops it. The owner runs, in the owner's own terminal: ae stop {own} -y. Nothing was stopped."
        )?;
        return Ok(EXIT_FAILED);
    }
    if caller.session() == Some(name) {
        writeln!(
            err,
            "Error: '{name}' is the session you are in; a bound stop never stops its caller's own session (that is ae stop --self). Nothing was stopped."
        )?;
        return Ok(EXIT_FAILED);
    }
    let Some(approved) = owner_approval(seat, STOP, name, request.owner_input, err)? else {
        return Ok(EXIT_FAILED);
    };
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
    let proof = prove(&dir, name, uuid, STOP.word);
    if let Proof::Refused(line) = &proof {
        writeln!(err, "Error: {line}. Nothing was stopped.")?;
        return Ok(EXIT_FAILED);
    }
    let dispatch = seat.map(|seat| stop_dispatch(seat, request));
    if let Some(dispatch) = &dispatch
        && let Err(why) = dispatch.request()
    {
        writeln!(
            err,
            "ae: stop REFUSED — ae could not record the dispatch in the orchestrator's journal ({why}). Nothing was stopped."
        )?;
        return Ok(EXIT_FAILED);
    }
    // Both streams are buffered, so the dispatch result is settled before
    // anything reaches a caller whose stream may be closed.
    let guards = Guards { approved };
    let (mut held_out, mut held_err) = (Vec::new(), Vec::new());
    let code = stopped(
        &dir,
        request,
        &guards,
        proof,
        seen.provenance,
        &mut held_out,
        &mut held_err,
    )?;
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

/// A bound lifecycle verb as its refusals spell it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Verb {
    /// The verb the owner words must name: `stop` or `rename`.
    pub(crate) word: &'static str,
    /// Its gerund in an owner-input refusal.
    pub(crate) gerund: &'static str,
    /// The closing sentence of each refusal.
    pub(crate) nothing: &'static str,
}

/// The bound stop.
const STOP: Verb = Verb {
    word: "stop",
    gerund: "stopping",
    nothing: "Nothing was stopped.",
};

/// Whether the `verb` of `name` carries owner words that approve it:
/// `Some(false)` without `--owner-input`, `Some(true)` when the proved seat
/// cites stored owner words naming the verb and this session, and `None` once
/// a refusal is written: another caller cites them, or the words do not
/// approve it.
pub(crate) fn owner_approval(
    seat: Option<&crate::relay::Dispatcher>,
    verb: Verb,
    name: &str,
    owner_input: Option<&str>,
    err: &mut impl Write,
) -> io::Result<Option<bool>> {
    let Verb {
        word,
        gerund,
        nothing,
    } = verb;
    match (seat, owner_input) {
        (_, None) => Ok(Some(false)),
        (None, Some(_)) => {
            writeln!(
                err,
                "ae: {word} REFUSED — --owner-input cites the owner's stored words, and only the proved orchestrator seat may cite them. {nothing}"
            )?;
            Ok(None)
        }
        (Some(seat), Some(id)) => {
            let approval = crate::relay::stored_owner_words(&seat.dir, &seat.uuid, id)
                .and_then(|words| crate::relay::words_name(&words, word, name));
            if let Err(why) = approval {
                writeln!(
                    err,
                    "ae: {word} REFUSED — owner input {id} does not approve {gerund} '{name}': {why}. {nothing}"
                )?;
                return Ok(None);
            }
            Ok(Some(true))
        }
    }
}

/// The proved seat's audit of this bound stop (G4), not yet written.
fn stop_dispatch(
    seat: &crate::relay::Dispatcher,
    request: &Request<'_>,
) -> crate::tracked::Dispatch {
    crate::tracked::Dispatch {
        dir: seat.dir.clone(),
        session: seat.session.clone(),
        uuid: seat.uuid.clone(),
        slot: seat.slot.clone(),
        pane: seat.pane.clone(),
        server: seat.server.clone(),
        reference: crate::tracked::Dispatch::reference(crate::time::Timestamp::now(), entropy()),
        verb: "stop",
        target: request.name.to_owned(),
        target_session: request.name.to_owned(),
        target_uuid: request.uuid.to_owned(),
        owner_input: request.owner_input.map(ToOwned::to_owned),
        requested: false,
    }
}

/// What the bound stop may waive once the caller and its owner words are
/// proved.
struct Guards {
    /// The orchestrator seat cited stored owner words that name `stop` and
    /// this session: a seat that declares working, waiting-agent or blocked
    /// does not refuse (G3). Any other declared value, a doubtful roster or an
    /// unreadable journal still does.
    approved: bool,
}

/// The bound stop after the proof under the lifecycle lock: the capture, the
/// events hold, the held writes and the kill, then the buffered publication.
#[allow(
    clippy::too_many_lines,
    reason = "the bound stop's one linear order: capture, events hold, the held writes and kill, then the buffered publication; each refusal reads beside the step that raises it"
)]
fn stopped(
    dir: &Path,
    request: &Request<'_>,
    guards: &Guards,
    proof: Proof,
    provenance: Provenance,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let (name, uuid) = (request.name, request.uuid);
    let (server, stamp) = match proof {
        Proof::Proven { server, stamp } => (server, stamp),
        Proof::Absent { .. } => {
            let bound = Bound::new(name, uuid, provenance);
            return already_stopped(dir, &bound, guards, err);
        }
        Proof::Refused(line) => {
            writeln!(err, "Error: {line}. Nothing was stopped.")?;
            return Ok(EXIT_FAILED);
        }
    };
    let bound = Bound::new(name, uuid, provenance);
    let mut held_err = Vec::new();
    let captured = match capture_models(dir, &server, name) {
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
    let hold = match crate::store::open(dir).hold_events() {
        Ok(hold) => hold,
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was stopped.")?;
            return Ok(EXIT_FAILED);
        }
    };
    if let Err(line) = seats_idle(dir, name, &hold, guards.approved, STOP.word) {
        writeln!(err, "Error: {line}. Nothing was stopped.")?;
        return Ok(EXIT_FAILED);
    }
    let requested = format!(
        "stop requested, bound to session {uuid} ({})",
        stamp.session.id
    );
    if let Err(why) = hold.append_event(&bound.event_line(STOP_REQUEST_ACTION, &requested)) {
        writeln!(
            err,
            "Error: ae could not record the stop request ({why}). Nothing was stopped."
        )?;
        return Ok(EXIT_FAILED);
    }
    let mut held_out = Vec::new();
    let outcome = kill_under_lock(
        &server,
        name,
        &stamp.session.id,
        Some(&stamp),
        &mut held_out,
        &mut held_err,
    )?;
    let result = stop_summary(&outcome, &held_err);
    if let Err(why) = hold.append_event(&bound.event_line(STOP_RESULT_ACTION, &result)) {
        let found = match &outcome {
            StopOutcome::Stopped => "was stopped",
            StopOutcome::AlreadyStopped => "is not running",
            StopOutcome::Failed | StopOutcome::Refused(_) => "was not stopped",
        };
        writeln!(
            held_err,
            "Warning: '{name}' {found}, but ae could not record the stop result ({why})."
        )?;
    }
    drop(hold);
    // The rest completes before anything reaches the caller, so a closed
    // stream cannot skip the migration note or the model publication.
    if let Some(note) = crate::migrate::session_noted(dir, name) {
        writeln!(held_err, "{note}")?;
    }
    for gap in publish_models(dir, captured) {
        warn_model(&mut held_err, name, &gap)?;
    }
    let code = u8::from(!matches!(outcome, StopOutcome::Stopped));
    let output = crate::error::with_status(code, out.write_all(&held_out));
    err.write_all(&held_err)?;
    output?;
    Ok(code)
}

/// A proven, already stopped target: the same declaration guard as a kill,
/// then the durable receipt through the held lock. A seat a plain stop left
/// working, or a journal ae cannot read, refuses and writes nothing.
fn already_stopped(
    dir: &Path,
    bound: &Bound<'_>,
    guards: &Guards,
    err: &mut impl Write,
) -> io::Result<u8> {
    let name = bound.name;
    let hold = match crate::store::open(dir).hold_events() {
        Ok(hold) => hold,
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was stopped.")?;
            return Ok(EXIT_FAILED);
        }
    };
    if let Err(line) = seats_idle(dir, name, &hold, guards.approved, STOP.word) {
        writeln!(err, "Error: {line}. Nothing was stopped.")?;
        return Ok(EXIT_FAILED);
    }
    let recorded =
        hold.append_event(&bound.event_line(STOP_RESULT_ACTION, ALREADY_STOPPED_SUMMARY));
    drop(hold);
    writeln!(err, "Session '{name}' is not running.")?;
    if let Err(why) = recorded {
        writeln!(
            err,
            "Warning: '{name}' is not running, but ae could not record the stop result ({why})."
        )?;
    }
    Ok(EXIT_FAILED)
}

/// The one warning shape of a model observation the stop could not make.
fn warn_model(err: &mut impl Write, name: &str, why: &str) -> io::Result<()> {
    writeln!(
        err,
        "Warning: could not observe the seat models before stopping '{name}': {why}"
    )
}

/// Who runs the bound stop, as `$TMUX` and `$TMUX_PANE` place it.
pub(crate) enum Caller {
    /// No tmux markers: a plain terminal, so no pane and no session.
    Outside,
    /// The ambient pane on the server `$TMUX` names, in `session`; `proved`
    /// when that session's recorded tmux server is proved to be the caller's.
    /// `viewer` is the ONE observation of the pane that named `session`, and
    /// the class and the provenance read the same one; boxed, because it is
    /// the one large reading and `Outside` carries nothing.
    Pane {
        server: ServerId,
        pane: String,
        session: String,
        proved: bool,
        viewer: Box<crate::tmux::ObservedViewer>,
    },
}

impl Caller {
    /// The session the caller's pane is in.
    pub(crate) fn session(&self) -> Option<&str> {
        match self {
            Self::Outside => None,
            Self::Pane { session, .. } => Some(session),
        }
    }
}

/// The session the caller runs in, read from `$TMUX_PANE` on the server
/// `$TMUX` names and never from a `--pane` word: a `--pane` may only repeat
/// that ambient pane, once. A caller outside tmux has no session. `Err`
/// carries the exit code and the refusal.
pub(crate) fn caller_session(
    root: &Path,
    panes: &[Option<String>],
) -> Result<Caller, (u8, &'static str)> {
    caller_session_from(root, panes, Reading::Own)
}

/// Where the ONE reading of the calling pane comes from.
#[derive(Clone, Copy)]
pub(crate) enum Reading<'a> {
    /// The worker reads the pane itself.
    Own,
    /// The entry already read it (`None`: no pane, outside tmux), and the
    /// worker reuses that reading instead of a second one.
    Entry(Option<&'a (crate::tmux::ObservedViewer, ServerId)>),
}

/// [`caller_session`] on `reading`. The marker checks are the same for both;
/// an entry reading must have been taken on the server `$TMUX` names.
pub(crate) fn caller_session_from(
    root: &Path,
    panes: &[Option<String>],
    reading: Reading<'_>,
) -> Result<Caller, (u8, &'static str)> {
    let server = match crate::doors::caller_server_reading() {
        // Outside tmux only when BOTH markers are positively absent: a pane
        // marker without its server is a failed observation, never the
        // owner's terminal, whichever entry lifted it into `--pane`.
        CallerServer::Absent if crate::doors::calling_pane_id().is_some() => {
            return Err((
                EXIT_FAILED,
                "$TMUX_PANE is set but $TMUX is not, so ae cannot tell which session this caller is in.",
            ));
        }
        CallerServer::Absent if panes.is_empty() => return Ok(Caller::Outside),
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
    // ONE observation of the validated pane: the session it names, the class
    // and the provenance all come from it.
    let read = match reading {
        Reading::Own => transport::observe_viewer(&server, &ambient),
        Reading::Entry(Some((viewer, read_on))) if *read_on == server => Some(viewer.clone()),
        Reading::Entry(_) => {
            return Err((
                EXIT_FAILED,
                "the entry read this caller on another tmux server than $TMUX names, so ae cannot tell which session it is in.",
            ));
        }
    };
    let Some((viewer, session)) = read.and_then(|viewer| {
        let session = viewer
            .session
            .clone()
            .filter(|session| !session.is_empty())?;
        Some((viewer, session))
    }) else {
        return Err((
            EXIT_FAILED,
            "tmux did not report this caller's pane, so ae cannot tell which session it is in.",
        ));
    };
    let (session, proved) = match caller_correlation(root, &server, session) {
        Ok(session) => (session, true),
        // An unproven owner still names its session: a target of that name
        // is refused as the caller's own rather than guessed apart.
        Err(CallerGap::NotAnAeSession(session) | CallerGap::ServerUnproved(session)) => {
            (session, false)
        }
        Err(CallerGap::PaneUnreadable | CallerGap::NotInTmux | CallerGap::NoPane) => {
            return Err((
                EXIT_FAILED,
                "ae cannot tell which session this caller is in.",
            ));
        }
    };
    Ok(Caller::Pane {
        server,
        pane: ambient,
        session,
        proved,
        viewer: Box::new(viewer),
    })
}

/// The caller as the bound events record it, under the existing keys.
pub(crate) struct Provenance {
    /// `actor`: see the module documentation.
    actor: &'static str,
    /// `actor_session`: the caller's session, only when it is proved.
    session: String,
    /// The caller's server, pane and session uuid, only when proved.
    triple: Option<IdentityTriple>,
    /// `identity_gap`: why a caller in tmux is not proved; empty when it is,
    /// and empty outside tmux, where no session identity exists.
    gap: &'static str,
}

impl Provenance {
    /// A pane ae could not prove: the neutral actor and the named gap.
    fn unproved(gap: &'static str) -> Self {
        Self {
            actor: PANE_ACTOR,
            session: String::new(),
            triple: None,
            gap,
        }
    }
}

/// The calling pane's class, as the dispatcher gate reads it.
pub(crate) enum Class {
    /// The owner's terminal, or a pane that is not the seat.
    Other,
    /// The proved orchestrator seat.
    Seat(crate::relay::Dispatcher),
    /// A pane positively in the seat's session that fails the dispatcher
    /// proof: neither the seat's powers nor an ordinary pane's.
    Unproven(&'static str),
}

/// What ONE observation of the calling pane proves: its class and the
/// provenance both bound events record.
pub(crate) struct Seen {
    pub(crate) class: Class,
    provenance: Provenance,
}

/// ONE observation of the calling pane, correlated with its session's meta:
/// the class `relay::seat_class` gives it, and the actor it supports.
pub(crate) fn observe(root: &Path, caller: &Caller) -> Seen {
    observe_as(root, caller, None)
}

/// The class of the calling pane as `relay::seat_class` judged it.
pub(crate) fn class_of(class: &crate::relay::SeatClass) -> Class {
    match class {
        crate::relay::SeatClass::Ordinary(_) => Class::Other,
        crate::relay::SeatClass::Dispatcher(seat) => Class::Seat(seat.clone()),
        crate::relay::SeatClass::Unproven(why) => Class::Unproven(why),
    }
}

/// [`observe`] with the class the entry gate already judged from the same
/// reading, so permission, the own-session checks and the audit read one
/// class; `None` judges it here.
pub(crate) fn observe_as(root: &Path, caller: &Caller, judged: Option<Class>) -> Seen {
    let Caller::Pane {
        server,
        pane,
        session,
        proved,
        viewer,
    } = caller
    else {
        return Seen {
            class: judged.unwrap_or(Class::Other),
            provenance: Provenance {
                actor: HUMAN_ACTOR,
                session: String::new(),
                triple: None,
                gap: "",
            },
        };
    };
    let dir = sessions_dir(root).join(session);
    let class =
        judged.unwrap_or_else(|| class_of(&crate::relay::seat_class(root, viewer, server, pane)));
    let triple = match crate::tracked::triple_from_viewer(viewer, pane, &dir) {
        Ok(triple) => triple,
        Err(gap) => {
            return Seen {
                class,
                provenance: Provenance::unproved(gap.name()),
            };
        }
    };
    if !*proved {
        return Seen {
            class,
            provenance: Provenance::unproved(UNPROVED_SERVER),
        };
    }
    let actor = if matches!(class, Class::Seat(_)) {
        crate::tracked::ORCHESTRATOR_DISPLAY
    } else {
        PANE_ACTOR
    };
    Seen {
        class,
        provenance: Provenance {
            actor,
            session: session.clone(),
            triple: Some(triple),
            gap: "",
        },
    }
}

/// Per-call entropy for a dispatch `ref`.
pub(crate) fn entropy() -> u64 {
    use std::hash::{BuildHasher as _, RandomState};
    RandomState::new().hash_one((std::process::id(), crate::time::Timestamp::now().epoch()))
}

/// The facts both bound events carry.
struct Bound<'a> {
    name: &'a str,
    uuid: &'a str,
    caller: Provenance,
}

impl<'a> Bound<'a> {
    fn new(name: &'a str, uuid: &'a str, caller: Provenance) -> Self {
        Self { name, uuid, caller }
    }

    /// One lifecycle event about the bound target: the caller's actor, the
    /// target's uuid in `target_session_id` and `target_session_uuid`, and the
    /// caller's proved facts or its named gap.
    fn event_line(&self, action: &str, summary: &str) -> String {
        let fields = EventFields {
            target_session_id: self.uuid,
            target_session_uuid: self.uuid,
            identity_gap: self.caller.gap,
            ..EventFields::new(
                crate::time::Timestamp::now(),
                self.caller.actor,
                action,
                self.name,
                "",
                "",
                &self.caller.session,
                "",
                "",
                summary,
                "",
            )
        };
        crate::tracked::event_line(&fields.with_caller(self.caller.triple.as_ref()))
    }
}

/// What proving the bound identity under the lifecycle lock found.
pub(crate) enum Proof {
    /// The live session to kill: its server and its whole proved identity.
    Proven {
        server: ServerId,
        stamp: SessionStamp,
    },
    /// The recorded session is this uuid and provably not running on its
    /// recorded server.
    Absent { server: ServerId },
    /// The refusal, one clause; nothing may be written.
    Refused(String),
}

/// Prove `name` is the session `uuid`, reading only: the raw directory, the
/// raw meta, the recorded server, and ONE isolated read of the live session.
/// `verb` is the bound form the refusal names.
pub(crate) fn prove(dir: &Path, name: &str, uuid: &str, verb: &str) -> Proof {
    match crate::teardown::classify_dir(dir) {
        crate::teardown::DirKind::RealDir => {}
        crate::teardown::DirKind::Symlink => {
            return Proof::Refused(format!(
                "'{name}' is a rename alias, not a session; a bound {verb} names the session by its current name"
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
    let Some(stamp) = transport::observe_session_stamp(&server, name) else {
        return match transport::verify_session_absent(&server, name) {
            StopProbe::Absent => Proof::Absent { server },
            StopProbe::Present | StopProbe::Unknown => Proof::Refused(format!(
                "ae cannot read session '{name}' on its recorded tmux server"
            )),
        };
    };
    if stamp.uuid.is_empty() {
        return Proof::Refused(format!(
            "tmux session '{name}' ({}) carries no ae session uuid, so ae cannot prove it is session {uuid}",
            stamp.session.id
        ));
    }
    if stamp.uuid != uuid {
        return Proof::Refused(format!(
            "tmux session '{name}' ({}) is session {}, not {uuid}",
            stamp.session.id,
            stamp.uuid.escape_debug()
        ));
    }
    Proof::Proven { server, stamp }
}

/// Under the held event lock: the CURRENT roster is whole and every seat of
/// it has no declaration or a latest one ae admits. `Err` names a doubtful or
/// unreadable roster, each other seat with its state, or the journal ae could
/// not read. `approved` (stored owner words that name this stop) admits a seat
/// that declares working, waiting-agent or blocked; it never admits any other
/// value, a roster or a journal ae doubts.
pub(crate) fn seats_idle(
    dir: &Path,
    name: &str,
    hold: &EventsHold,
    approved: bool,
    verb: &str,
) -> Result<(), String> {
    let actors = roster_actors(dir, name)?;
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
        .filter(|(_, latest)| !is_idle(&latest.value))
        .filter(|(_, latest)| !(approved && is_waivable(&latest.value)))
        .map(|(actor, latest)| {
            format!(
                "{} is {}",
                actor.escape_debug(),
                String::from_utf8_lossy(&latest.value).escape_debug()
            )
        })
        .collect();
    if busy.is_empty() {
        return Ok(());
    }
    Err(format!(
        "'{name}' has seats that are not idle ({}); a bound {verb} waits until each seat declares done or waiting-user",
        busy.join(", ")
    ))
}

/// The CURRENT roster of `name` as its meta records it, whole: `Err` names
/// an unreadable, doubtful or empty roster.
pub(crate) fn roster_actors(dir: &Path, name: &str) -> Result<Vec<String>, String> {
    let Ok(bytes) = meta::read_bytes(dir) else {
        return Err(format!("ae cannot read the roster of '{name}'"));
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return Err(format!(
            "the metadata of '{name}' is not UTF-8, so ae cannot read its roster"
        ));
    };
    let parsed = meta::Meta::parse(&text);
    if let Some(anomaly) = parsed
        .anomalies()
        .iter()
        .find(|anomaly| crate::roster::roster_doubting(anomaly))
    {
        return Err(format!(
            "'{name}' has doubtful roster metadata ({}); run 'ae doctor' first",
            anomaly.to_string().escape_debug()
        ));
    }
    let actors: Vec<String> = parsed
        .roster()
        .iter()
        .map(meta::RosterEntry::reference)
        .collect();
    if actors.is_empty() {
        return Err(format!(
            "'{name}' records no seats, so ae cannot read their states"
        ));
    }
    Ok(actors)
}

/// Whether a raw declared value is one a bound stop admits, compared byte for
/// byte with nothing normalized.
fn is_idle(value: &[u8]) -> bool {
    IDLE_STATES.iter().any(|idle| idle.as_bytes() == value)
}

/// Whether a raw declared value is one stored owner words may waive, compared
/// byte for byte with nothing normalized.
fn is_waivable(value: &[u8]) -> bool {
    WAIVABLE_STATES.iter().any(|busy| busy.as_bytes() == value)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_stamp_read_ends_with_the_session_uuid_option() {
        assert!(
            crate::tmux::SESSION_STAMP_FORMAT
                .ends_with(&format!("#{{{}}}", crate::theme::SESSION_ID_OPTION))
        );
    }

    #[test]
    fn only_done_and_waiting_user_are_idle() {
        let idle: Vec<&str> = crate::state::VALUES
            .into_iter()
            .filter(|value| super::is_idle(value.as_bytes()))
            .collect();
        assert_eq!(idle, ["waiting-user", "done"]);
    }

    /// D4 and the provenance ruling: both bound receipts name the target by
    /// the bound uuid under the existing target keys, and an unproved caller
    /// names its gap; a proved caller carries no gap.
    #[test]
    fn a_bound_receipt_names_the_bound_uuid_and_the_caller_gap() {
        const UUID: &str = "11111111-2222-4333-8444-555555555555";
        let field = |line: &str, key: &str| {
            crate::json::parse(line.trim_end())
                .ok()
                .and_then(|value| value.get_str(key).map(ToOwned::to_owned))
        };
        let unproved = super::Bound::new(
            "target",
            UUID,
            super::Provenance::unproved(super::UNPROVED_SERVER),
        )
        .event_line("stop-request", "bound stop");
        assert_eq!(field(&unproved, "target_session_id").as_deref(), Some(UUID));
        assert_eq!(
            field(&unproved, "target_session_uuid").as_deref(),
            Some(UUID)
        );
        assert_eq!(
            field(&unproved, "identity_gap").as_deref(),
            Some(super::UNPROVED_SERVER)
        );
        assert_eq!(
            field(&unproved, "actor").as_deref(),
            Some(super::PANE_ACTOR)
        );
        let proved = super::Bound::new(
            "target",
            UUID,
            super::Provenance {
                actor: super::PANE_ACTOR,
                session: "caller".to_owned(),
                triple: Some(crate::tracked::IdentityTriple {
                    server: "/private/srv".to_owned(),
                    pane: "%3".to_owned(),
                    session_uuid: "11111111-2222-4333-8444-666666666666".to_owned(),
                }),
                gap: "",
            },
        )
        .event_line("stop-result", "bound stop");
        assert_eq!(field(&proved, "target_session_uuid").as_deref(), Some(UUID));
        assert_eq!(field(&proved, "identity_gap"), None);
        assert_eq!(field(&proved, "actor_session").as_deref(), Some("caller"));
    }

    #[test]
    fn an_idle_value_is_compared_raw() {
        assert!(super::is_idle(b"done"));
        assert!(!super::is_idle(b"done "));
        assert!(!super::is_idle(b"Done"));
        assert!(!super::is_idle(b"done\xff"));
        assert!(!super::is_idle(b""));
    }

    /// Plan A's one observation: the class and the provenance read the
    /// snapshot the caller carries. Its server socket does not exist, so a
    /// second tmux read could only fail.
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test writes its isolated session meta"
    )]
    fn the_class_and_the_provenance_read_the_one_observation_the_caller_carries() {
        const UUID: &str = "11111111-2222-4333-8444-777777777777";
        let root = std::env::temp_dir().join(format!("ae-bound-snapshot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = super::sessions_dir(&root).join("caller");
        std::fs::create_dir_all(&dir).expect("isolated session directory");
        std::fs::write(
            dir.join("meta"),
            format!("session=caller\nsession_id={UUID}\n"),
        )
        .expect("isolated meta");
        let socket = root.join("no-server");
        let viewer = crate::tmux::ObservedViewer {
            slot: Some("main".to_owned()),
            session: Some("caller".to_owned()),
            agent: Some("lead".to_owned()),
            session_uuid: crate::tmux::OptionReading::Set(UUID.to_owned()),
            socket_path: Some(socket.display().to_string()),
        };
        let caller = super::Caller::Pane {
            server: crate::inventory::ServerId::Selected(crate::meta::Selector::Socket(
                socket.clone(),
            )),
            pane: "%9".to_owned(),
            session: "caller".to_owned(),
            proved: true,
            viewer: Box::new(viewer),
        };
        let seen = super::observe(&root, &caller);
        let _ = std::fs::remove_dir_all(&root);
        assert!(matches!(seen.class, super::Class::Other));
        assert_eq!(seen.provenance.actor, super::PANE_ACTOR);
        assert_eq!(seen.provenance.gap, "");
        assert_eq!(seen.provenance.session, "caller");
        let triple = seen
            .provenance
            .triple
            .expect("the snapshot's identity triple");
        assert_eq!(triple.server, socket.display().to_string());
        assert_eq!(triple.pane, "%9");
        assert_eq!(triple.session_uuid, UUID);
    }

    /// The bound rename reuses the class its entry gate judged from the same
    /// reading: the fixture alone would judge the pane an ordinary one, and
    /// the provenance still comes from the reading.
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test writes its isolated session meta"
    )]
    fn a_judged_class_decides_and_the_provenance_still_reads_the_one_observation() {
        const UUID: &str = "11111111-2222-4333-8444-888888888888";
        let root = std::env::temp_dir().join(format!("ae-bound-judged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = super::sessions_dir(&root).join("caller");
        std::fs::create_dir_all(&dir).expect("isolated session directory");
        std::fs::write(
            dir.join("meta"),
            format!("session=caller\nsession_id={UUID}\n"),
        )
        .expect("isolated meta");
        let socket = root.join("no-server");
        let viewer = crate::tmux::ObservedViewer {
            slot: Some("main".to_owned()),
            session: Some("caller".to_owned()),
            agent: Some("lead".to_owned()),
            session_uuid: crate::tmux::OptionReading::Set(UUID.to_owned()),
            socket_path: Some(socket.display().to_string()),
        };
        let caller = super::Caller::Pane {
            server: crate::inventory::ServerId::Selected(crate::meta::Selector::Socket(
                socket.clone(),
            )),
            pane: "%9".to_owned(),
            session: "caller".to_owned(),
            proved: true,
            viewer: Box::new(viewer),
        };
        let own = super::observe(&root, &caller);
        let judged = super::observe_as(&root, &caller, Some(super::Class::Unproven("judged")));
        let _ = std::fs::remove_dir_all(&root);
        assert!(matches!(own.class, super::Class::Other));
        assert!(matches!(judged.class, super::Class::Unproven("judged")));
        assert_eq!(judged.provenance.actor, super::PANE_ACTOR);
        assert_eq!(judged.provenance.session, "caller");
        let triple = judged
            .provenance
            .triple
            .expect("the reading's identity triple");
        assert_eq!(triple.pane, "%9");
        assert_eq!(triple.session_uuid, UUID);
    }
}
