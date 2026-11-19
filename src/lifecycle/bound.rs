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
//!    record names a positive server, and ONE isolated read of the live
//!    session of that name proves the server incarnation, the session
//!    incarnation and its `@ae_session_uuid` stamp together;
//! 3. still without a write, each seat's pane text and launch guard are read;
//! 4. the event container's lock is taken and held: the roster is read again
//!    from the current meta, a roster ae doubts or cannot read refuses, and
//!    any seat whose latest declaration is not `done` or `waiting-user`
//!    refuses, as does a journal ae cannot read;
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
//! writes nothing. A declaration either precedes the
//! read in step 4 and is judged there, or waits on the held lock until the
//! kill is done. This process never holds the event lock and the meta lock
//! together, so it keeps the meta-then-events order of every other writer.
//! Both events carry the bound uuid as their target and the caller as ONE
//! observation proved it, or the named gap when it did not. Their actor is
//! `ae/orchestrator` for a pane the dispatcher gate's proof recognizes as the
//! orchestrator seat, `human` for a caller outside tmux (the owner's own
//! terminal, as that gate reads it), and the neutral `ae:stop` for every
//! other pane, whose facts the correlation fields carry.

use std::io::{self, Write};
use std::path::Path;

use super::{
    ALREADY_STOPPED_SUMMARY, CallerGap, STOP_REQUEST_ACTION, STOP_RESULT_ACTION, StopOutcome,
    caller_resolution, capture_models, kill_under_lock, lock, name_is_usable, publish_models,
    server_of, sessions_dir, stop_summary,
};
use crate::doors::CallerServer;
use crate::inventory::ServerId;
use crate::meta::{self, ServerSelector};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::store::{EventsHold, SourceRead};
use crate::tmux::{ObservedViewer, SessionStamp, StopProbe};
use crate::tracked::{CorrelationGap, EventFields, IdentityTriple};
use crate::transport;

/// The declarations a bound stop admits. A seat that never declared is
/// admitted too; every other value, a future one included, refuses.
const IDLE_STATES: [&str; 2] = ["done", "waiting-user"];

/// The gap a bound event names when the caller's pane correlates with its
/// session but that session's recorded tmux server is not proved to be the
/// caller's.
const UNPROVED_SERVER: &str = "caller server unproved";

/// The actor of a bound stop run outside tmux: the owner's own terminal.
const HUMAN_ACTOR: &str = "human";

/// The neutral actor of a bound stop run from a pane that is not the proved
/// orchestrator seat: a pane proves where the command ran, not who typed it.
const PANE_ACTOR: &str = "ae:stop";

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
    let caller = match caller_session(root, panes) {
        Ok(caller) => caller,
        Err((code, line)) => {
            writeln!(err, "Error: {line} Nothing was stopped.")?;
            return Ok(code);
        }
    };
    if caller.session() == Some(name) {
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
    let (server, stamp) = match prove(&dir, name, uuid) {
        Proof::Proven { server, stamp } => (server, stamp),
        Proof::Absent => {
            let bound = Bound::new(name, uuid, provenance(root, &caller));
            return already_stopped(&dir, &bound, err);
        }
        Proof::Refused(line) => {
            writeln!(err, "Error: {line}. Nothing was stopped.")?;
            return Ok(EXIT_FAILED);
        }
    };
    let bound = Bound::new(name, uuid, provenance(root, &caller));
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
    if let Some(note) = crate::migrate::session_noted(&dir, name) {
        writeln!(held_err, "{note}")?;
    }
    for gap in publish_models(&dir, captured) {
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
fn already_stopped(dir: &Path, bound: &Bound<'_>, err: &mut impl Write) -> io::Result<u8> {
    let name = bound.name;
    let hold = match crate::store::open(dir).hold_events() {
        Ok(hold) => hold,
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was stopped.")?;
            return Ok(EXIT_FAILED);
        }
    };
    if let Err(line) = seats_idle(dir, name, &hold) {
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
enum Caller {
    /// No tmux markers: a plain terminal, so no pane and no session.
    Outside,
    /// The ambient pane on the server `$TMUX` names, in `session`; `proved`
    /// when that session's recorded tmux server is proved to be the caller's.
    Pane {
        server: ServerId,
        pane: String,
        session: String,
        proved: bool,
    },
}

impl Caller {
    /// The session the caller's pane is in.
    fn session(&self) -> Option<&str> {
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
fn caller_session(root: &Path, panes: &[Option<String>]) -> Result<Caller, (u8, &'static str)> {
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
    match caller_resolution(root, Some(&server), &ambient) {
        Ok(session) => Ok(Caller::Pane {
            server,
            pane: ambient,
            session,
            proved: true,
        }),
        // An unproven owner still names its session: a target of that name
        // is refused as the caller's own rather than guessed apart.
        Err(CallerGap::NotAnAeSession(session) | CallerGap::ServerUnproved(session)) => {
            Ok(Caller::Pane {
                server,
                pane: ambient,
                session,
                proved: false,
            })
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

/// The caller as the bound events record it, under the existing keys.
struct Provenance {
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

/// ONE observation of the calling pane, correlated with its session's meta,
/// and the actor that observation supports.
fn provenance(root: &Path, caller: &Caller) -> Provenance {
    let Caller::Pane {
        server,
        pane,
        session,
        proved,
    } = caller
    else {
        return Provenance {
            actor: HUMAN_ACTOR,
            session: String::new(),
            triple: None,
            gap: "",
        };
    };
    let Some(viewer) = transport::observe_viewer(server, pane) else {
        return Provenance::unproved(CorrelationGap::Unreadable.name());
    };
    let dir = sessions_dir(root).join(session);
    let triple = match crate::tracked::triple_from_viewer(&viewer, pane, &dir) {
        Ok(triple) => triple,
        Err(gap) => return Provenance::unproved(gap.name()),
    };
    if !*proved {
        return Provenance::unproved(UNPROVED_SERVER);
    }
    let actor = if is_dispatcher(root, &viewer, server) {
        crate::tracked::ORCHESTRATOR_DISPLAY
    } else {
        PANE_ACTOR
    };
    Provenance {
        actor,
        session: session.clone(),
        triple: Some(triple),
        gap: "",
    }
}

/// Whether the observed calling pane is provably the orchestrator seat: the
/// proof the dispatcher gate (`refuse_dispatcher_seen` in src/lib.rs) takes
/// before it refuses a dispatcher, on the same observation.
fn is_dispatcher(root: &Path, viewer: &ObservedViewer, server: &ServerId) -> bool {
    let (Some(session), Some(_slot)) = (viewer.session.as_deref(), viewer.slot.as_deref()) else {
        return false;
    };
    crate::session_launch::name::is_session_name(session)
        && crate::relay::physical_session(&sessions_dir(root).join(session), viewer, server)
            .is_ok_and(|(_, bytes)| meta::is_seat(&bytes, root))
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
enum Proof {
    /// The live session to kill: its server and its whole proved identity.
    Proven {
        server: ServerId,
        stamp: SessionStamp,
    },
    /// The recorded session is this uuid and provably not running.
    Absent,
    /// The refusal, one clause; nothing may be written.
    Refused(String),
}

/// Prove `name` is the session `uuid`, reading only: the raw directory, the
/// raw meta, the recorded server, and ONE isolated read of the live session.
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
    let Some(stamp) = transport::observe_session_stamp(&server, name) else {
        return match transport::verify_session_absent(&server, name) {
            StopProbe::Absent => Proof::Absent,
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
/// not read.
fn seats_idle(dir: &Path, name: &str, hold: &EventsHold) -> Result<(), String> {
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
        "'{name}' has seats that are not idle ({}); a bound stop waits until each seat declares done or waiting-user",
        busy.join(", ")
    ))
}

/// Whether a raw declared value is one a bound stop admits, compared byte for
/// byte with nothing normalized.
fn is_idle(value: &[u8]) -> bool {
    IDLE_STATES.iter().any(|idle| idle.as_bytes() == value)
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
}
