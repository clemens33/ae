//! Who runs a launch, the seat guard, and the start journal.
//!
//! Every launch knows its caller from the ONE observation its entry took, as
//! a [`LaunchCaller`]; nothing below the entry observes again. The launch
//! owner judges that caller under the target's lifecycle lock:
//!
//! - an ordinary agent seat runs no launch form on another session: create,
//!   start, resume, attach and switch-client are refused before any write to
//!   the target, with the owner's command, and the refusal is journaled once
//!   in the seat's own record when that record correlates with the
//!   observation ([`refuse`]);
//! - every session a launch builds is journaled in the target's own
//!   `events.jsonl` with its actor and caller pane, under the lock and before
//!   the session is handed over ([`journal_start`]).

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::inventory::ServerId;
use crate::relay::{Dispatcher, PaneIdentity};
use crate::state::EXIT_USAGE;
use crate::tracked::{EventFields, IdentityTriple};

/// The journal action of a session a launch built.
const START_ACTION: &str = "session-start";

/// The journal action of a launch the seat guard refused.
const REFUSED_ACTION: &str = "launch-refused";

/// The `@ae_agent` values ae writes on a pane without `@ae_slot`: the watchdog
/// pane and the event monitor pane. Any other agent stamp without a slot is a
/// seat whose slot stamp is missing.
const PLUMBING: [&str; 2] = ["_watchdog", "_events"];

/// Who runs a launch, as the ONE observation its entry took says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LaunchCaller {
    /// No tmux markers: the owner's own terminal, a tmux binding or a popup.
    Outside,
    /// A tmux pane that is not an agent seat: a shell, the app or the chat.
    Pane(PaneIdentity),
    /// An ordinary agent seat.
    Seat(PaneIdentity),
    /// The proved orchestrator seat, whose launch its audit records.
    Orchestrator(Dispatcher),
    /// The child of `ae reboot`: the pane that asked ended with the session
    /// it rebooted. `archive` is the archive the child inherits.
    Reboot {
        /// The archive uuid.
        archive: String,
    },
}

impl LaunchCaller {
    /// The class of an ordinary pane: an agent seat when it carries a slot
    /// stamp, or an agent stamp that is not one of ae's plumbing panes.
    pub(crate) fn of(identity: PaneIdentity) -> Self {
        let slot = identity
            .slot
            .as_deref()
            .is_some_and(|slot| !slot.is_empty());
        let agent = identity
            .display
            .as_deref()
            .is_some_and(|name| !name.is_empty() && !PLUMBING.contains(&name));
        if identity.session.is_some() && (slot || agent) {
            Self::Seat(identity)
        } else {
            Self::Pane(identity)
        }
    }
}

/// The route a launch came in by, as its journal rows spell it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchRoute {
    /// `ae <name> …` and bare `ae`'s first use.
    Entry,
    /// `ae <name> --resume --no-attach --expect-uuid <uuid>`.
    Bound,
    /// A served `ae _launch …` argv.
    Served,
    /// The owner's `ae orchestrator`.
    OrchestratorStart,
    /// The orchestrator picker row's resume.
    Picker,
    /// The settings menu's start or resume.
    Settings,
    /// Bare `ae`'s restore after a host reboot.
    Restore,
    /// The child of `ae reboot`.
    Reboot,
    /// The orchestrator seat's own launch (S1).
    Dispatch,
}

impl LaunchRoute {
    /// The word a journal row names the route by.
    pub(crate) const fn word(self) -> &'static str {
        match self {
            Self::Entry => "entry",
            Self::Bound => "bound",
            Self::Served => "served",
            Self::OrchestratorStart => "orchestrator-start",
            Self::Picker => "picker",
            Self::Settings => "settings",
            Self::Restore => "restore",
            Self::Reboot => "reboot",
            Self::Dispatch => "dispatch",
        }
    }
}

/// A launch's caller, its route and the user words it typed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Caller<'a> {
    /// Who runs the launch.
    pub(crate) who: &'a LaunchCaller,
    /// How it came in.
    pub(crate) route: LaunchRoute,
    /// The user words, which the refusal gives back as the owner's command.
    pub(crate) words: &'a [String],
}

/// Whether `dir` is the seat's own session. The target is no alias (a launch
/// on an alias reclaims a fresh name). A pane whose session carries a uuid
/// stamp owns the target when its meta records exactly one canonical session
/// uuid, the stamp's; a stamp that is no canonical uuid proves nothing and
/// owns nothing. A pane whose session carries no stamp at all (a vacant or
/// unobserved uuid seed, or an older session) owns it only when the target is
/// a real directory whose meta names the session the pane is in, on the
/// pane's own server.
pub(super) fn is_own(dir: &Path, seat: &PaneIdentity) -> bool {
    if crate::lifecycle::aliases::target(dir).is_some() {
        return false;
    }
    match seat.session_uuid.as_deref() {
        Some(stamp) => sole_uuid(dir).is_some_and(|recorded| recorded == stamp),
        None => is_recorded_own(dir, seat),
    }
}

/// The proof [`is_own`] takes when the pane carries no session uuid: `dir`
/// is the real directory of the session the pane is in, and its meta names
/// that session on the pane's own server.
fn is_recorded_own(dir: &Path, seat: &PaneIdentity) -> bool {
    let (Some(sessions), Some(name)) = (dir.parent(), dir.file_name().and_then(|n| n.to_str()))
    else {
        return false;
    };
    let Some(server) = pane_server(seat) else {
        return false;
    };
    seat.session.as_deref() == Some(name)
        && is_real_dir(dir)
        && crate::relay::recorded_session(sessions, name, &server).is_ok()
}

/// The server the pane's own observation names, by its socket.
fn pane_server(seat: &PaneIdentity) -> Option<ServerId> {
    let socket = seat.server.as_deref().filter(|socket| !socket.is_empty())?;
    Some(ServerId::Selected(crate::meta::Selector::Socket(
        PathBuf::from(socket),
    )))
}

/// Whether `path` is a real directory, never a link of any kind.
fn is_real_dir(path: &Path) -> bool {
    matches!(
        crate::teardown::classify_dir(path),
        crate::teardown::DirKind::RealDir
    )
}

/// The refusal of a seat's launch on another session: the line with the
/// owner's command, then one row in the seat's own record. `typed` is the
/// session as typed, `target` the locked name and `dir` its directory.
///
/// # Errors
///
/// Only a failure to write `err`.
pub(super) fn refuse(
    sessions: &Path,
    target: &str,
    dir: &Path,
    typed: &str,
    seat: &PaneIdentity,
    caller: Caller<'_>,
    err: &mut impl Write,
) -> io::Result<u8> {
    let actor = seat_actor(seat);
    let own = seat.session.as_deref().unwrap_or_default();
    let slot = seat
        .slot
        .as_deref()
        .filter(|slot| !slot.is_empty())
        .unwrap_or("-");
    let words: Vec<String> = caller.words.iter().map(|word| owner_word(word)).collect();
    writeln!(
        err,
        "ae: launch REFUSED — agent seat {actor} ({own}:{slot}) runs no launch form on another session, and '{typed}' is not its own. The owner runs, in the owner's own terminal: ae {}; or the orchestrator seat runs it on the owner's word. Nothing was changed.",
        words.join(" ")
    )?;
    if let Err(why) = journal_refusal(sessions, target, dir, typed, seat, caller.route) {
        writeln!(
            err,
            "ae: the refusal could not be journaled in session '{own}': {why}."
        )?;
    }
    err.flush()?;
    Ok(EXIT_USAGE)
}

/// The one refusal row, in the seat's own record only. The seat's own
/// lifecycle lock is taken without waiting, because the launch already holds
/// the target's and a wait could invert a rename's name-sorted order. Held
/// through the append, it keeps a rename or a reclaim from replacing the
/// record between the correlation and the row. Under it the record must be
/// the real session directory its pane names, record that session on the
/// pane's server, and record exactly one canonical session uuid, the live one,
/// all read from one snapshot of its meta.
fn journal_refusal(
    sessions: &Path,
    target: &str,
    dir: &Path,
    typed: &str,
    seat: &PaneIdentity,
    route: LaunchRoute,
) -> Result<(), String> {
    let session = caller_session(seat).map_err(str::to_owned)?;
    let _hold = crate::store::lock(
        &sessions.join(format!(".lifecycle.{session}.lock")),
        Duration::ZERO,
    )
    .map_err(|_| "the caller's session is in another lifecycle operation".to_owned())?;
    let (record, uuid) = correlated_record(sessions, session, seat).map_err(str::to_owned)?;
    let target_uuid = if crate::lifecycle::aliases::target(dir).is_some() {
        String::new()
    } else {
        sole_uuid(dir).unwrap_or_default()
    };
    let actor = seat_actor(seat);
    let own = seat.session.as_deref().unwrap_or_default();
    let slot = seat.slot.as_deref().unwrap_or_default();
    let summary = format!(
        "launch refused via {}: an agent seat runs no launch form on another session",
        route.word()
    );
    let triple = IdentityTriple {
        server: seat.server.clone().unwrap_or_default(),
        pane: seat.pane.clone(),
        session_uuid: uuid.clone(),
    };
    let fields = EventFields {
        actor_session_id: &uuid,
        target_session_id: &target_uuid,
        target_session_uuid: &target_uuid,
        ..EventFields::new(
            crate::time::Timestamp::now(),
            &actor,
            REFUSED_ACTION,
            typed,
            "",
            slot,
            own,
            "",
            target,
            &summary,
            "",
        )
    };
    crate::store::open(&record)
        .append_event(&crate::tracked::event_line(
            &fields.with_caller(Some(&triple)),
        ))
        .map_err(|why| why.to_string())
}

/// The session the seat's pane names, when it is a valid session name.
fn caller_session(seat: &PaneIdentity) -> Result<&str, &'static str> {
    seat.session
        .as_deref()
        .filter(|session| crate::config::is_session_name(session))
        .ok_or("the caller pane names no session")
}

/// The seat's own record, positively correlated with the observation: its
/// directory and its canonical uuid. The directory is a real one, never a
/// link of any kind, and the server and the uuid come from one meta snapshot.
fn correlated_record(
    sessions: &Path,
    session: &str,
    seat: &PaneIdentity,
) -> Result<(PathBuf, String), &'static str> {
    let live = live_uuid(seat).ok_or("the caller pane carries no session uuid")?;
    let server = pane_server(seat).ok_or("the caller pane's server is unknown")?;
    if !is_real_dir(&sessions.join(session)) {
        return Err("the caller's session record is not a real directory");
    }
    let (record, bytes) = crate::relay::recorded_session(sessions, session, &server)?;
    if sole_uuid_of(&bytes).as_deref() != Some(live) {
        return Err(
            "the caller's record does not hold the pane's session uuid as its one session id",
        );
    }
    Ok((record, live.to_owned()))
}

/// The start row of a session this launch built: `session` with uuid `uuid`
/// on `server`, `resumed` when its record existed before this launch, and
/// `reference` the seat's dispatch reference (empty for every other caller).
/// `server` is the socket path of a socket-selected server
/// ([`socket_spelling`]). A named or the ambient server, the default launch's
/// included, has no socket the launch resolves, and its row carries no
/// `target_server`: resolving it would add a tmux round trip to every launch.
///
/// # Errors
///
/// Why the row could not be appended.
pub(super) fn journal_start(
    dir: &Path,
    session: &str,
    uuid: &str,
    server: &str,
    resumed: bool,
    reference: &str,
    caller: Caller<'_>,
) -> Result<(), String> {
    let summary = format!(
        "{} via {}",
        if resumed { "resumed" } else { "created" },
        caller.route.word()
    );
    let facts = Facts::of(caller.who);
    let reference = if reference.is_empty() {
        facts.reference
    } else {
        reference
    };
    let fields = EventFields {
        actor_session_id: facts.session_uuid,
        target_session_id: uuid,
        target_session_uuid: uuid,
        target_server: server,
        identity_gap: facts.gap,
        ..EventFields::new(
            crate::time::Timestamp::now(),
            &facts.actor,
            START_ACTION,
            session,
            reference,
            facts.slot,
            facts.session,
            "",
            session,
            &summary,
            "",
        )
    };
    crate::store::open(dir)
        .append_event(&crate::tracked::event_line(
            &fields.with_caller(facts.triple.as_ref()),
        ))
        .map_err(|why| why.to_string())
}

/// What a start row says of its caller.
struct Facts<'a> {
    actor: String,
    slot: &'a str,
    session: &'a str,
    session_uuid: &'a str,
    triple: Option<IdentityTriple>,
    gap: &'static str,
    reference: &'a str,
}

impl<'a> Facts<'a> {
    fn of(who: &'a LaunchCaller) -> Self {
        let none = Self {
            actor: String::new(),
            slot: "",
            session: "",
            session_uuid: "",
            triple: None,
            gap: "",
            reference: "",
        };
        match who {
            LaunchCaller::Outside => Self {
                actor: "human".to_owned(),
                ..none
            },
            LaunchCaller::Pane(pane) => Self {
                actor: "ae:launch".to_owned(),
                ..Self::pane(pane)
            },
            LaunchCaller::Seat(seat) => Self {
                actor: seat_actor(seat),
                ..Self::pane(seat)
            },
            LaunchCaller::Orchestrator(seat) => Self {
                actor: crate::tracked::ORCHESTRATOR_DISPLAY.to_owned(),
                slot: &seat.slot,
                session: &seat.session,
                session_uuid: &seat.uuid,
                triple: Some(IdentityTriple {
                    server: seat.server.clone(),
                    pane: seat.pane.clone(),
                    session_uuid: seat.uuid.clone(),
                }),
                ..none
            },
            LaunchCaller::Reboot { archive } => Self {
                actor: "ae:reboot".to_owned(),
                gap: "caller pane ended with the rebooted session",
                reference: archive,
                ..none
            },
        }
    }

    /// The facts a pane's identity gives, whatever its actor.
    fn pane(pane: &'a PaneIdentity) -> Self {
        let uuid = live_uuid(pane).unwrap_or_default();
        Self {
            actor: String::new(),
            slot: pane.slot.as_deref().unwrap_or_default(),
            session: pane.session.as_deref().unwrap_or_default(),
            session_uuid: uuid,
            triple: Some(IdentityTriple {
                server: pane.server.clone().unwrap_or_default(),
                pane: pane.pane.clone(),
                session_uuid: uuid.to_owned(),
            }),
            gap: if uuid.is_empty() {
                "caller session uuid unset"
            } else {
                ""
            },
            reference: "",
        }
    }
}

/// The socket path a start row names its server by: the selected socket, or
/// empty (unwritten) for a named or the ambient server, whose socket the
/// launch does not resolve.
pub(super) fn socket_spelling(server: &ServerId) -> String {
    match server {
        ServerId::Selected(crate::meta::Selector::Socket(path)) => path.display().to_string(),
        ServerId::Selected(crate::meta::Selector::Name(_)) | ServerId::Ambient => String::new(),
    }
}

/// A seat's journal actor: its display name, else `slot:<slot>`, which no
/// agent name can spell.
fn seat_actor(seat: &PaneIdentity) -> String {
    match (seat.display.as_deref(), seat.slot.as_deref()) {
        (Some(name), _) if !name.is_empty() => name.to_owned(),
        (_, Some(slot)) if !slot.is_empty() => format!("slot:{slot}"),
        _ => "slot:-".to_owned(),
    }
}

/// The pane's live session uuid, when it carries a canonical one.
fn live_uuid(pane: &PaneIdentity) -> Option<&str> {
    pane.session_uuid.as_deref().filter(|uuid| !uuid.is_empty())
}

/// The one canonical session uuid the meta in `dir` records: `None` when it
/// records none, more than one, or no valid one.
fn sole_uuid(dir: &Path) -> Option<String> {
    sole_uuid_of(&crate::meta::read_bytes(dir).ok()?)
}

/// The one canonical session uuid the meta bytes `bytes` record.
fn sole_uuid_of(bytes: &[u8]) -> Option<String> {
    let raw = crate::meta::sole_value(bytes, "session_id")?;
    let uuid = crate::archive::canonical_uuid(std::str::from_utf8(raw).ok()?);
    (!uuid.is_empty()).then_some(uuid)
}

/// A user word as the owner retypes it: as is when the shell reads it so,
/// else single-quoted.
fn owner_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-./:=@%+,".contains(&b));
    if plain {
        word.to_owned()
    } else {
        crate::words::shell_quote(word)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(display: Option<&str>, slot: Option<&str>) -> PaneIdentity {
        PaneIdentity {
            display: display.map(str::to_owned),
            slot: slot.map(str::to_owned),
            session: Some("assistant".to_owned()),
            session_uuid: None,
            pane: "%1".to_owned(),
            server: None,
        }
    }

    #[test]
    fn a_slot_or_a_non_plumbing_agent_stamp_makes_a_seat_and_plumbing_does_not() {
        for (display, slot) in [
            (Some("lead"), Some("main")),
            (None, Some("spawned.2")),
            (Some("lead"), None),
            (Some("_chat"), None),
        ] {
            assert!(
                matches!(LaunchCaller::of(pane(display, slot)), LaunchCaller::Seat(_)),
                "{display:?} {slot:?}"
            );
        }
        for (display, slot) in [
            (None, None),
            (Some("_watchdog"), None),
            (Some("_events"), None),
            (Some(""), Some("")),
        ] {
            assert!(
                matches!(LaunchCaller::of(pane(display, slot)), LaunchCaller::Pane(_)),
                "{display:?} {slot:?}"
            );
        }
        let mut sessionless = pane(Some("lead"), Some("main"));
        sessionless.session = None;
        assert!(matches!(
            LaunchCaller::of(sessionless),
            LaunchCaller::Pane(_)
        ));
    }

    #[test]
    fn a_seat_actor_is_its_display_else_its_slot() {
        assert_eq!(seat_actor(&pane(Some("lead"), Some("main"))), "lead");
        assert_eq!(seat_actor(&pane(None, Some("main"))), "slot:main");
        assert_eq!(
            seat_actor(&pane(Some(""), Some("spawned.2"))),
            "slot:spawned.2"
        );
        assert_eq!(seat_actor(&pane(None, None)), "slot:-");
        for actor in ["slot:main", "slot:-"] {
            assert!(!crate::config::is_agent_name(actor), "{actor}");
        }
    }

    #[test]
    fn an_owner_word_is_quoted_only_when_the_shell_would_split_or_expand_it() {
        assert_eq!(owner_word("roster"), "roster");
        assert_eq!(owner_word("--dir"), "--dir");
        assert_eq!(owner_word("/tmp/a.b"), "/tmp/a.b");
        assert_eq!(owner_word("my dir"), "'my dir'");
        assert_eq!(owner_word("$HOME"), "'$HOME'");
        assert_eq!(owner_word("it's"), "'it'\\''s'");
        assert_eq!(owner_word(""), "''");
    }

    #[test]
    fn every_route_has_its_own_word() {
        let words = [
            LaunchRoute::Entry,
            LaunchRoute::Bound,
            LaunchRoute::Served,
            LaunchRoute::OrchestratorStart,
            LaunchRoute::Picker,
            LaunchRoute::Settings,
            LaunchRoute::Restore,
            LaunchRoute::Reboot,
            LaunchRoute::Dispatch,
        ]
        .map(LaunchRoute::word);
        let unique: std::collections::BTreeSet<_> = words.iter().collect();
        assert_eq!(unique.len(), words.len(), "{words:?}");
    }
}
