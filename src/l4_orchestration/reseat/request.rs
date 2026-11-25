//! The orchestrator seat's OWN reseat, as a request the watchdog runs after
//! the seat's turn.
//!
//! The seat cannot move itself in place: the tool running `ae reseat` is the
//! tool a move replaces (caller rule 7). So from the PROVED orchestrator seat,
//! `ae reseat <own session> <own agent> --using <profile[@client]>` runs every
//! check a reseat runs before its lock, plus the one an orchestrator move
//! needs — its conversation must travel ([`super::travels`]) — and then writes
//! ONE request, [`FILE`], in its session directory, bound to the session's
//! uuid, the slot, the agent and the launch that asked. It touches no pane.
//! `--cancel` withdraws it, and a second request replaces the first.
//!
//! The watchdog CLAIMS a request at the first cycle that sees the seat idle
//! (the trigger, [`trigger`]: a rename to [`CLAIMED`] and one `attempt`
//! record, under the seat's lock and the session's lifecycle lock), then hands
//! it to a detached leg ([`run_leg`]) that moves the seat through the one
//! executor the automatic move takes ([`super::run_requested`]). Every ending
//! past the claim is journaled and terminal; a refusal or a failure is owed to
//! the seat as its next turn ([`owed_notice`]).
//!
//! The file is hostile persisted state: [`parse`] is strict, capped and
//! fuzzed, and a request that does not parse, or whose binding no longer
//! names the seat as it stands, never executes ([`sweep`]).

use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use crate::events::Event;
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::store::SourceRead;
use crate::time::Timestamp;
use crate::tracked::EventFields;
use crate::watchdog::{MAIN_SLOT, WATCHDOG_ACTOR};

use super::{Binding, Ended, StopPolicy};

/// The pending request, in the session directory.
pub const FILE: &str = "reseat-request";
/// The request the watchdog has claimed and is running.
pub const CLAIMED: &str = "reseat-request.claimed";

/// The seat queued (or replaced) its request.
pub const QUEUED_ACTION: &str = "self-reseat-queued";
/// The request was withdrawn before the watchdog claimed it.
pub const WITHDRAWN_ACTION: &str = "self-reseat-withdrawn";
/// The watchdog claimed the request and starts the move.
pub const ATTEMPT_ACTION: &str = "self-reseat-attempt";
/// The seat moved.
pub const DONE_ACTION: &str = "self-reseat-done";
/// The request did not run, and will not: nothing was stopped.
pub const REFUSED_ACTION: &str = "self-reseat-refused";
/// The move started and did not finish, and will not run again.
pub const FAILED_ACTION: &str = "self-reseat-failed";
/// A refusal or failure was delivered to the seat as its turn.
pub const NOTICE_ACTION: &str = "self-reseat-notice";

/// The first line of every request.
const MAGIC: &str = "ae-self-reseat v1";
/// The most bytes a request may hold.
pub const CAP: u64 = 1024;
/// The longest `to` value, in bytes.
const TO_MAX: usize = 160;
/// How long an attempt may stay open with no leg holding the seat before the
/// watchdog books it failed: the automatic leg's own bound.
pub(crate) const OVERDUE_SECS: i64 = 180;

/// How long the detached leg waits for the seat's lock. The claim hands the
/// recorded attempt to the leg past every lock, so a short holder (this
/// cycle's sweep, an automatic trigger that yields) may still hold the seat
/// when the leg starts; the leg waits it out and re-proves its attempt
/// inside the lock. A leg that still finds the seat held leaves the attempt
/// open for the sweep's overdue rule, never for a replay.
const LEG_LOCK_WAIT: Duration = Duration::from_secs(10);
/// The hidden leg's usage line.
pub const LEG_USAGE: &str = "Usage: ae _self-reseat <dir> <slot> <id>";
/// The longest summary a record carries.
const SUMMARY_MAX: usize = 400;

/// One request, as written and as parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The session's canonical uuid when the request was queued.
    pub uuid: String,
    /// The seat's slot: always `main`.
    pub slot: String,
    /// The seat's agent name.
    pub agent: String,
    /// The launch that asked: `launch_id.main` when the request was queued.
    pub launch: String,
    /// Where the seat moves: `profile` or `profile@client`.
    pub to: String,
    /// The request's own id: a canonical uuid every record of it refers to.
    pub id: String,
    /// When it was queued.
    pub at: Timestamp,
}

impl Request {
    /// The seat this request is bound to.
    pub(crate) fn binding(&self) -> Binding {
        Binding {
            uuid: self.uuid.clone(),
            slot: self.slot.clone(),
            agent: self.agent.clone(),
            launch: self.launch.clone(),
        }
    }
}

/// Render `request` as the bytes [`parse`] reads back.
#[must_use]
pub fn render(request: &Request) -> String {
    format!(
        "{MAGIC}\nsession {}\nslot {}\nagent {}\nlaunch {}\nto {}\nid {}\nat {}\n",
        request.uuid,
        request.slot,
        request.agent,
        request.launch,
        request.to,
        request.id,
        request.at
    )
}

/// Parse one request: exactly eight LF-terminated lines in order, one space
/// after each key, every value in its own grammar, nothing else.
///
/// # Errors
///
/// Why the bytes are no request, in words a refusal record carries.
pub fn parse(bytes: &[u8]) -> Result<Request, String> {
    if u64::try_from(bytes.len()).map_or(true, |len| len > CAP) {
        return Err(format!("it holds more than {CAP} bytes"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "it is not UTF-8".to_owned())?;
    let Some(body) = text.strip_suffix('\n') else {
        return Err("it does not end with a newline".to_owned());
    };
    let lines: Vec<&str> = body.split('\n').collect();
    let [magic, uuid, slot, agent, launch, to, id, at] = lines.as_slice() else {
        return Err(format!("it holds {} lines, not 8", lines.len()));
    };
    if *magic != MAGIC {
        return Err(format!("its first line is not '{MAGIC}'"));
    }
    let uuid = value(uuid, "session").filter(|v| crate::archive::is_canonical_uuid(v));
    let uuid = uuid.ok_or("its session line is not a canonical uuid")?;
    let slot = value(slot, "slot").filter(|v| *v == MAIN_SLOT);
    let slot = slot.ok_or("its slot line is not 'slot main'")?;
    let agent = value(agent, "agent").filter(|v| crate::config::is_agent_name(v));
    let agent = agent.ok_or("its agent line is not an agent name")?;
    let launch = value(launch, "launch").filter(|v| crate::archive::is_canonical_uuid(v));
    let launch = launch.ok_or("its launch line is not a canonical uuid")?;
    let to = value(to, "to").filter(|v| is_destination(v));
    let to = to.ok_or("its to line is not <profile> or <profile>@<client>")?;
    let id = value(id, "id").filter(|v| crate::archive::is_canonical_uuid(v));
    let id = id.ok_or("its id line is not a canonical uuid")?;
    let at = value(at, "at")
        .and_then(|v| Timestamp::parse(v).filter(|stamp| stamp.to_string() == v))
        .ok_or("its at line is not a timestamp")?;
    Ok(Request {
        uuid: uuid.to_owned(),
        slot: slot.to_owned(),
        agent: agent.to_owned(),
        launch: launch.to_owned(),
        to: to.to_owned(),
        id: id.to_owned(),
        at,
    })
}

/// The value of `line` under `key`: one space after the key, then a value
/// with no whitespace and no control character.
fn value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let value = line.strip_prefix(key)?.strip_prefix(' ')?;
    (!value.is_empty() && value.chars().all(|c| !c.is_whitespace() && !c.is_control()))
        .then_some(value)
}

/// `profile` or `profile@client`: one `@` at most, both sides non-empty
/// printable ASCII with no `=`, the whole at most [`TO_MAX`] bytes.
fn is_destination(value: &str) -> bool {
    let part = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_graphic() && b != b'@' && b != b'=')
    };
    value.len() <= TO_MAX
        && match value.split_once('@') {
            Some((profile, client)) => part(profile) && part(client),
            None => part(value),
        }
}

/// What is at a request's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Found {
    Absent,
    /// Something is there and it is no request: why.
    Damaged(String),
    Ready(Request),
}

/// Read the request at `name` in `dir`, capped, never following a link.
pub(crate) fn read(dir: &Path, name: &str) -> Found {
    match crate::store::read_capped_in(dir, name, CAP) {
        Err(_) => Found::Damaged(format!("it holds more than {CAP} bytes")),
        Ok(SourceRead::Absent) => Found::Absent,
        Ok(SourceRead::Ready(bytes)) => parse(&bytes).map_or_else(Found::Damaged, Found::Ready),
        Ok(SourceRead::Invalid(what)) => Found::Damaged(format!("it is {what}")),
        Ok(SourceRead::Unreadable(why)) => Found::Damaged(format!("it cannot be read ({why})")),
    }
}

/// Remove the request at `name`; an absent one is already gone.
fn remove(dir: &Path, name: &str) -> io::Result<()> {
    match std::fs::remove_file(dir.join(name)) {
        Err(why) if why.kind() != io::ErrorKind::NotFound => Err(why),
        _ => Ok(()),
    }
}

/// The session lifecycle lock of the session whose directory is `dir`: the
/// lock every lifecycle writer takes, here taken from the directory itself.
fn lifecycle_lock(dir: &Path, wait: Duration) -> io::Result<std::fs::File> {
    let (Some(sessions), Some(name)) = (dir.parent(), dir.file_name().and_then(|n| n.to_str()))
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a session directory",
        ));
    };
    crate::lifecycle::lock_in(sessions, name, wait)
}

/// The seat's own lock, the one every auto reseat writer and leg takes,
/// without waiting.
fn seat_lock(dir: &Path) -> io::Result<std::fs::File> {
    seat_lock_within(dir, Duration::ZERO)
}

/// [`seat_lock`], waiting at most `wait` for a holder to let go.
fn seat_lock_within(dir: &Path, wait: Duration) -> io::Result<std::fs::File> {
    crate::store::lock(&crate::autoreseat::lock_path(dir, MAIN_SLOT), wait)
}

/// One record of the family, addressed to the seat at `main`: whether the
/// journal took it, durably (the store's append syncs before it answers).
/// Every state change a record stands for waits on this answer: no file is
/// claimed, released or removed on a record that was lost.
#[must_use]
fn record(
    dir: &Path,
    (session, agent): (&str, &str),
    (actor, actor_slot): (&str, &str),
    (action, reference, summary): (&str, &str, &str),
    now: Timestamp,
) -> bool {
    let summary = bounded(summary);
    let line = crate::tracked::event_line(&EventFields {
        ts: now,
        actor,
        action,
        target: agent,
        reference,
        actor_slot,
        actor_session: if actor_slot.is_empty() { "" } else { session },
        target_slot: MAIN_SLOT,
        target_session: session,
        actor_session_id: "",
        target_session_id: "",
        target_server: "",
        target_pane: "",
        target_session_uuid: "",
        caller_server: "",
        caller_pane: "",
        caller_session_uuid: "",
        identity_gap: "",
        summary: &summary,
        body_file: "",
    });
    crate::store::open(dir).append_event(&line).is_ok()
}

/// A watchdog record of the family: whether the journal took it.
#[must_use]
fn watchdog_record(
    dir: &Path,
    seat: (&str, &str),
    (action, reference, summary): (&str, &str, &str),
    now: Timestamp,
) -> bool {
    record(
        dir,
        seat,
        (WATCHDOG_ACTOR, ""),
        (action, reference, summary),
        now,
    )
}

/// One clean line of at most [`SUMMARY_MAX`] characters.
fn bounded(text: &str) -> String {
    let clean: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let clean = clean.trim();
    if clean.chars().count() <= SUMMARY_MAX {
        return clean.to_owned();
    }
    let mut cut: String = clean.chars().take(SUMMARY_MAX - 1).collect();
    cut.push('…');
    cut
}

/// Say `text` once on the owner's chat, as the watchdog: a line for the
/// owner, never state, so a lost one changes nothing.
fn said(dir: &Path, now: Timestamp, text: &str) {
    let line = crate::tracked::chat_line_for(now, WATCHDOG_ACTOR, "", "", text);
    let _ = crate::store::open(dir).append_event(&line);
}

/// The agent the session's meta seats at `main`, or empty.
fn main_agent(bytes: &[u8]) -> String {
    crate::lifecycle::meta_value(bytes, &format!("seat.{MAIN_SLOT}"))
}

// ---------------------------------------------------------------------------
// The command: queue, replace, withdraw.
// ---------------------------------------------------------------------------

/// The seat the gate proved: what the queue needs of it.
pub(crate) struct Seat<'a> {
    pub(crate) session: &'a str,
    pub(crate) dir: &'a Path,
    pub(crate) uuid: &'a str,
    pub(crate) slot: &'a str,
    /// Whether a name the argv gives is this seat's own session (its name or
    /// a rename alias of it).
    pub(crate) own: &'a dyn Fn(&str) -> bool,
}

/// `ae reseat <own session> <own agent> --using <profile[@client]>` or
/// `--cancel`, from the PROVED orchestrator seat.
///
/// # Errors
///
/// Only a failure to write `out` or `err`.
pub(crate) fn run(
    root: &Path,
    seat: &Seat<'_>,
    tail: &[String],
    now: Timestamp,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    if tail.iter().any(|word| word == CANCEL_FLAG) {
        return run_cancel(seat, tail, now, out, err);
    }
    match checked(seat, tail, now, err)? {
        Ok(request) => queue(root, seat, &request, out, err),
        Err(code) => Ok(code),
    }
}

/// `--cancel` from the proved seat: only its own request.
fn run_cancel(
    seat: &Seat<'_>,
    tail: &[String],
    now: Timestamp,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    match cancel_names(tail) {
        Ok((session, agent)) => {
            if let Some(line) = not_own(seat, session, agent) {
                writeln!(err, "{line}")?;
                return Ok(EXIT_FAILED);
            }
            cancel(
                seat.dir,
                (seat.session, agent),
                (agent, seat.slot),
                now,
                out,
                err,
            )
        }
        Err(line) => {
            writeln!(err, "{line}")?;
            Ok(EXIT_USAGE)
        }
    }
}

/// Every check a reseat runs before its lock, for the seat's own move: the
/// request it would write, or the exit code of the refusal it printed.
fn checked(
    seat: &Seat<'_>,
    tail: &[String],
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<Result<Request, u8>> {
    let parsed = match super::parse(tail) {
        Ok(parsed) => parsed,
        Err(line) => {
            writeln!(err, "{line}")?;
            if line != super::RESEAT_USAGE {
                writeln!(err, "{}", super::RESEAT_USAGE)?;
            }
            return Ok(Err(EXIT_USAGE));
        }
    };
    if parsed.stop != StopPolicy::Proven {
        writeln!(
            err,
            "Error: a queued reseat proves the seat's frame itself, after this turn — drop \
             --stop-unknown. Nothing was queued."
        )?;
        return Ok(Err(EXIT_USAGE));
    }
    if let Some(line) = not_own(seat, &parsed.session, &parsed.agent) {
        writeln!(err, "{line}")?;
        return Ok(Err(EXIT_FAILED));
    }
    let bytes = crate::meta::read_bytes(seat.dir).unwrap_or_default();
    let to = match moved_to(seat, &bytes, &parsed) {
        Ok((to, warning)) => {
            if let Some(line) = warning {
                writeln!(err, "{line}")?;
            }
            to
        }
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(Err(EXIT_FAILED));
        }
    };
    let launch = crate::lifecycle::meta_value(&bytes, &format!("launch_id.{}", seat.slot));
    if !crate::archive::is_canonical_uuid(&launch) {
        writeln!(
            err,
            "Error: '{}' records no launch id a request can be bound to — nothing was queued.",
            parsed.agent
        )?;
        return Ok(Err(EXIT_FAILED));
    }
    Ok(Ok(Request {
        uuid: seat.uuid.to_owned(),
        slot: seat.slot.to_owned(),
        agent: parsed.agent.clone(),
        launch,
        to,
        id: crate::launch::generate_uuid(),
        at: now,
    }))
}

/// The roster, the destination and its conversation: where the seat would
/// move (`profile` or `profile@client`) and the cover warning to print, or
/// the refusal line.
fn moved_to(
    seat: &Seat<'_>,
    bytes: &[u8],
    parsed: &super::Parsed,
) -> Result<(String, Option<String>), String> {
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(bytes));
    let Some(seated) = meta.roster().iter().find(|row| row.name == parsed.agent) else {
        let roster: Vec<&str> = meta.roster().iter().map(|row| row.name.as_str()).collect();
        return Err(format!(
            "Error: '{}' has no seat named '{}'; its roster is {}.",
            seat.session,
            parsed.agent,
            if roster.is_empty() {
                "empty".to_owned()
            } else {
                roster.join(", ")
            }
        ));
    };
    if seated.slot != seat.slot {
        return Err(only_itself(seat.session, &main_agent(bytes)));
    }
    let recorded = seated.profile.clone().unwrap_or_default();
    let moving = super::destination(seat.dir, bytes, parsed, seated, &recorded)?;
    let warning = super::cover_warning(
        seat.dir,
        bytes,
        &parsed.profile,
        moving.pair.is_some().then_some(moving.command.as_str()),
    );
    let to = parsed.client.as_deref().map_or_else(
        || parsed.profile.clone(),
        |label| format!("{}@{label}", parsed.profile),
    );
    // THE SAME PORTABLE ADAPTER, on its own (lead's ruling): a keep on a
    // nonportable tool's own store passes `travels` and is refused here.
    super::portable_same_adapter(seat.dir, bytes, seat.slot, &to)?;
    // THE CONVERSATION TRAVELS, or nothing is queued: the watchdog moves the
    // orchestrator only onto a destination that keeps or carries it, so a
    // request that cannot is refused now rather than at the turn's end.
    if let Err(why) = super::travels(seat.dir, bytes, seat.slot, &to) {
        return Err(format!(
            "Error: a queued reseat must carry the orchestrator's conversation, and '{to}' \
             cannot ({why}) — pick a profile of the same tool, or give the owner the command. \
             Nothing was queued."
        ));
    }
    Ok((to, warning))
}

/// The flag that withdraws a request.
pub const CANCEL_FLAG: &str = "--cancel";

/// `<session> <agent> --cancel`, the flag anywhere: the two names.
///
/// # Errors
///
/// The usage refusal.
pub(crate) fn cancel_names(tail: &[String]) -> Result<(&str, &str), String> {
    let names: Vec<&str> = tail
        .iter()
        .map(String::as_str)
        .filter(|word| *word != CANCEL_FLAG)
        .collect();
    match names.as_slice() {
        [session, agent] if !session.starts_with('-') && !agent.starts_with('-') => {
            Ok((session, agent))
        }
        _ => Err(format!(
            "Error: --cancel takes no other flag. Usage: ae reseat <session> <agent> {CANCEL_FLAG}"
        )),
    }
}

/// The refusal for a target that is not the seat itself, or `None`.
fn not_own(seat: &Seat<'_>, session: &str, agent: &str) -> Option<String> {
    let bytes = crate::meta::read_bytes(seat.dir).unwrap_or_default();
    let own = main_agent(&bytes);
    if (seat.own)(session) && agent == own {
        None
    } else {
        Some(only_itself(seat.session, &own))
    }
}

/// The one refusal of a reseat the seat may not ask for.
fn only_itself(session: &str, agent: &str) -> String {
    format!(
        "ae: reseat REFUSED — the orchestrator seat reseats only itself, as a request the \
         watchdog runs after this turn: ae reseat {session} {agent} --using <profile[@client]> \
         (or {CANCEL_FLAG}). For any other seat, give the owner the command. Nothing was changed."
    )
}

/// Write `request` under the session's lifecycle lock: the binding read again
/// there, no claimed request running, then the file and its record.
fn queue(
    root: &Path,
    seat: &Seat<'_>,
    request: &Request,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let Ok(_lifecycle) = crate::lifecycle::lock(root, seat.session) else {
        writeln!(
            err,
            "Error: another lifecycle operation holds '{}' — it waited and gave up. Nothing was \
             queued.",
            seat.session
        )?;
        return Ok(EXIT_FAILED);
    };
    let locked = crate::meta::read_bytes(seat.dir).unwrap_or_default();
    if let Some(why) = request.binding().changed(&locked) {
        writeln!(err, "Error: {why} — nothing was queued.")?;
        return Ok(EXIT_FAILED);
    }
    if read(seat.dir, CLAIMED) != Found::Absent {
        writeln!(err, "{RUNNING}")?;
        return Ok(EXIT_FAILED);
    }
    // THE WRITER WRITES ONLY WHAT THE READER READS: a value the parser's
    // grammar refuses is refused here, never queued for the watchdog to call
    // damaged.
    let rendered = render(request);
    if let Err(why) = parse(rendered.as_bytes()).and_then(|back| {
        (back == *request)
            .then_some(())
            .ok_or_else(|| "it does not read back as written".to_owned())
    }) {
        writeln!(
            err,
            "Error: this request cannot be recorded: {why}. Nothing was queued."
        )?;
        return Ok(EXIT_FAILED);
    }
    let replaced = match read(seat.dir, FILE) {
        Found::Ready(old) => format!("; replaces the request to {}", old.to),
        Found::Damaged(_) => "; replaces a damaged request".to_owned(),
        Found::Absent => String::new(),
    };
    if let Err(why) = crate::store::publish_private_in(seat.dir, FILE, rendered.as_bytes()) {
        writeln!(
            err,
            "Error: the request could not be written ({why}). Nothing was queued."
        )?;
        return Ok(EXIT_FAILED);
    }
    let journaled = record(
        seat.dir,
        (seat.session, &request.agent),
        (&request.agent, seat.slot),
        (
            QUEUED_ACTION,
            &request.id,
            &format!("to {} after this turn{replaced}", request.to),
        ),
        request.at,
    );
    // The published file is the request, as for a withdrawal: a record the
    // journal did not take is said, and changes nothing.
    if !journaled {
        writeln!(
            err,
            "ae: the journal did not take the {QUEUED_ACTION} record; the request stands."
        )?;
    }
    writeln!(out, "reseat queued: {} after this turn", request.to)?;
    Ok(0)
}

/// The refusal of a queue or a withdrawal while the watchdog runs a request.
const RUNNING: &str = "Error: the watchdog is running a requested reseat of this seat now; its \
     outcome is journaled as self-reseat-done, -refused or -failed. Nothing was changed.";

/// Withdraw the pending request of `agent`, under the lifecycle lock.
/// `actor` is who withdrew it.
///
/// # Errors
///
/// Only a failure to write `out` or `err`.
pub(crate) fn cancel(
    dir: &Path,
    (session, agent): (&str, &str),
    actor: (&str, &str),
    now: Timestamp,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let Ok(_lifecycle) = lifecycle_lock(dir, Duration::from_secs(15)) else {
        writeln!(
            err,
            "Error: another lifecycle operation holds '{session}' — it waited and gave up. \
             Nothing was withdrawn."
        )?;
        return Ok(EXIT_FAILED);
    };
    if read(dir, CLAIMED) != Found::Absent {
        writeln!(err, "{RUNNING}")?;
        return Ok(EXIT_FAILED);
    }
    let (reference, what) = match read(dir, FILE) {
        Found::Absent => {
            writeln!(out, "no reseat request is queued")?;
            return Ok(0);
        }
        Found::Ready(request) if request.agent != agent => {
            writeln!(out, "no reseat request is queued for '{agent}'")?;
            return Ok(0);
        }
        Found::Ready(request) => (request.id, request.to),
        Found::Damaged(_) => (String::new(), "a damaged request".to_owned()),
    };
    if let Err(why) = remove(dir, FILE) {
        writeln!(
            err,
            "Error: the request could not be removed ({why}). Nothing was withdrawn."
        )?;
        return Ok(EXIT_FAILED);
    }
    // The file is gone, so the request is withdrawn whatever the journal says.
    if !record(
        dir,
        (session, agent),
        actor,
        (
            WITHDRAWN_ACTION,
            &reference,
            &format!("withdrew the request to {what}"),
        ),
        now,
    ) {
        writeln!(err, "ae: the journal did not take the withdrawal record.")?;
    }
    writeln!(out, "reseat request withdrawn: {what}")?;
    Ok(0)
}

// ---------------------------------------------------------------------------
// The watchdog: sweep, claim, leg, notice.
// ---------------------------------------------------------------------------

/// Whether a record of `action` with `reference` is addressed to the seat.
fn is(event: &Event, (session, agent): (&str, &str), action: &str) -> bool {
    event.action == action
        && crate::watchdog::event_is_addressed_to(event, session, MAIN_SLOT, agent)
}

/// The outcome record of request `id`, when the journal holds one.
fn outcome_of<'a>(events: &'a [Event], seat: (&str, &str), id: &str) -> Option<&'a Event> {
    events.iter().rev().find(|event| {
        event.reference.as_deref() == Some(id)
            && [DONE_ACTION, REFUSED_ACTION, FAILED_ACTION]
                .iter()
                .any(|action| is(event, seat, action))
    })
}

/// The newest attempt that has no outcome: its id and when it opened.
fn open_attempt(events: &[Event], seat: (&str, &str)) -> Option<(String, Timestamp)> {
    let attempt = events
        .iter()
        .rev()
        .find(|event| is(event, seat, ATTEMPT_ACTION))?;
    let id = attempt.reference.clone().unwrap_or_default();
    (!id.is_empty() && outcome_of(events, seat, &id).is_none()).then_some((id, attempt.ts))
}

/// Whether request `id` was ever attempted: the edge past which it is never
/// run again, whatever else the journal shows.
fn attempted(events: &[Event], seat: (&str, &str), id: &str) -> bool {
    events
        .iter()
        .any(|event| event.reference.as_deref() == Some(id) && is(event, seat, ATTEMPT_ACTION))
}

/// The journal as a decision about attempts must read it: whole, or not at
/// all. A journal ae cannot read whole is no proof that an attempt is absent.
fn ledger(dir: &Path) -> Option<Vec<Event>> {
    crate::autoreseat::leg::orchestrator_ledger(dir).ok()
}

/// The reference a damaged file's refusal is journaled under: the digest of
/// its bytes, or of the reason when there are none to read. The same damage is
/// refused once, and its file removed on a later cycle when it outlived the
/// first.
fn damaged_reference(dir: &Path, name: &str, why: &str) -> String {
    let digest = match crate::store::read_capped_in(dir, name, CAP) {
        Ok(SourceRead::Ready(bytes)) => crate::install::sha256_hex(&bytes),
        _ => crate::install::sha256_hex(format!("{name}\n{why}").as_bytes()),
    };
    format!("damaged-{}", digest.get(..32).unwrap_or(&digest))
}

/// Whether refusal `reference` is journaled for the seat since its newest
/// queued request: a repeat finds it and only removes the file.
fn refused_already(events: &[Event], seat: (&str, &str), reference: &str) -> bool {
    events
        .iter()
        .rev()
        .take_while(|event| !is(event, seat, QUEUED_ACTION))
        .any(|event| {
            event.reference.as_deref() == Some(reference) && is(event, seat, REFUSED_ACTION)
        })
}

/// Refuse a request that never ran: its record and the owner's line, THEN the
/// file gone. A record the journal does not take leaves the file for the next
/// cycle; a refusal already journaled is not journaled again, so a file that
/// could not be removed is only removed then.
fn refuse_unrun(
    dir: &Path,
    seat: (&str, &str),
    events: &[Event],
    (name, reference): (&str, &str),
    why: &str,
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<()> {
    let reference = if reference.is_empty() {
        damaged_reference(dir, name, why)
    } else {
        reference.to_owned()
    };
    if !refused_already(events, seat, &reference) {
        if !watchdog_record(dir, seat, (REFUSED_ACTION, &reference, why), now) {
            writeln!(
                err,
                "ae: watchdog: the journal did not take the refusal of {name}; it stays for the \
                 next cycle: {why}"
            )?;
            return Ok(());
        }
        said(
            dir,
            now,
            &format!("self reseat: {}'s request was not run — {why}", seat.1),
        );
    }
    if let Err(cause) = remove(dir, name) {
        writeln!(
            err,
            "ae: watchdog: the refused reseat request {name} of {} could not be removed ({cause})",
            seat.1
        )?;
    }
    Ok(())
}

/// The watchdog's cycle-level pass over the seat's request files, whatever the
/// seat's frame: a damaged or stale pending request is refused and removed, and
/// a claimed request is settled by the ONE rule its crash window allows. Only
/// under the seat's lock and the lifecycle lock, neither waited for, and only
/// over a journal read whole: a running leg holds the first lock, a queue or a
/// move the second, and the next cycle asks again.
///
/// # Errors
///
/// Only a failure to write `err`.
pub(crate) fn sweep(
    dir: &Path,
    session: &str,
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<()> {
    let pending = crate::store::source_presence(&dir.join(FILE));
    let claimed = crate::store::source_presence(&dir.join(CLAIMED));
    let absent = crate::store::SourcePresence::Absent;
    if pending == absent && claimed == absent {
        return Ok(());
    }
    let (Ok(_seat), Ok(_lifecycle)) = (seat_lock(dir), lifecycle_lock(dir, Duration::ZERO)) else {
        return Ok(());
    };
    let Some(events) = ledger(dir) else {
        return Ok(());
    };
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let agent = main_agent(&bytes);
    let seat = (session, agent.as_str());
    match read(dir, CLAIMED) {
        Found::Absent => {}
        found => {
            recover(dir, (seat, &bytes), &events, found, now, err)?;
            // A pending request waits for the claimed one to be settled.
            return Ok(());
        }
    }
    match read(dir, FILE) {
        Found::Absent => {}
        Found::Damaged(why) => {
            let why = format!("damaged request: {why}");
            refuse_unrun(dir, seat, &events, (FILE, ""), &why, now, err)?;
        }
        Found::Ready(request) if outcome_of(&events, seat, &request.id).is_some() => {
            // Ended already; only its file outlived the ending.
            if let Err(why) = remove(dir, FILE) {
                writeln!(err, "ae: watchdog: {FILE} could not be removed ({why})")?;
            }
        }
        Found::Ready(request) => {
            if let Some(why) = request.binding().changed(&bytes) {
                let why = format!("stale request: {why}");
                refuse_unrun(dir, seat, &events, (FILE, &request.id), &why, now, err)?;
            }
        }
    }
    Ok(())
}

/// Settle a claimed request by its crash window, never running it twice and
/// never back to pending: the claim is the commitment.
///
/// - an outcome is journaled: remove the file;
/// - it was attempted (the edge): an attempt open past [`OVERDUE_SECS`] with
///   no leg holding the seat is booked failed, else the leg still reports;
/// - never attempted (the claim was interrupted before its record): refused,
///   as stale when the seat moved since;
/// - damaged: refused, or failed under an overdue open attempt.
fn recover(
    dir: &Path,
    (seat, bytes): ((&str, &str), &[u8]),
    events: &[Event],
    found: Found,
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<()> {
    let overdue = |opened: Timestamp| now.epoch() >= opened.epoch().saturating_add(OVERDUE_SECS);
    match found {
        Found::Absent => {}
        Found::Ready(request) if outcome_of(events, seat, &request.id).is_some() => {
            if let Err(why) = remove(dir, CLAIMED) {
                writeln!(err, "ae: watchdog: {CLAIMED} could not be removed ({why})")?;
            }
        }
        Found::Ready(request) if attempted(events, seat, &request.id) => {
            let opened = events
                .iter()
                .rev()
                .find(|event| {
                    event.reference.as_deref() == Some(request.id.as_str())
                        && is(event, seat, ATTEMPT_ACTION)
                })
                .map(|event| event.ts);
            if let Some(opened) = opened.filter(|opened| overdue(*opened)) {
                fail_overdue(dir, seat, &request.id, opened, now, err)?;
            }
        }
        Found::Ready(request) => {
            let why = request.binding().changed(bytes).map_or_else(
                || {
                    "refused: the claim was interrupted before its attempt was journaled; it \
                     will not run again"
                        .to_owned()
                },
                |why| format!("stale request: {why}"),
            );
            refuse_unrun(dir, seat, events, (CLAIMED, &request.id), &why, now, err)?;
        }
        Found::Damaged(why) => match open_attempt(events, seat) {
            Some((id, opened)) if overdue(opened) => {
                fail_overdue(dir, seat, &id, opened, now, err)?;
            }
            Some(_) => {}
            None => {
                let why = format!("damaged request: {why}");
                refuse_unrun(dir, seat, events, (CLAIMED, ""), &why, now, err)?;
            }
        },
    }
    Ok(())
}

/// Book an attempt no leg reported on as failed, THEN remove its claimed
/// file; a record the journal does not take leaves the file for the next cycle.
fn fail_overdue(
    dir: &Path,
    seat: (&str, &str),
    id: &str,
    opened: Timestamp,
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<()> {
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let why = format!(
        "failed: the move opened at {opened} never reported within {OVERDUE_SECS} s; ae now \
         records {}",
        records_now(&bytes)
    );
    if !watchdog_record(dir, seat, (FAILED_ACTION, id, &why), now) {
        writeln!(
            err,
            "ae: watchdog: the journal did not take the failure of request {id}; it stays for \
             the next cycle"
        )?;
        return Ok(());
    }
    said(dir, now, &format!("self reseat: {} — {why}", seat.1));
    if let Err(why) = remove(dir, CLAIMED) {
        writeln!(err, "ae: watchdog: {CLAIMED} could not be removed ({why})")?;
    }
    Ok(())
}

/// What the meta records of the seat now, for a failure that must say it.
fn records_now(bytes: &[u8]) -> String {
    let value = |key: &str| crate::lifecycle::meta_value(bytes, &format!("{key}.{MAIN_SLOT}"));
    format!(
        "profile '{}' (launch {})",
        value("profile"),
        value("launch_id")
    )
}

/// Whether an automatic move of the seat is in flight: its attempt open
/// under a limit, headroom or return episode. A request waits for it.
pub(crate) fn auto_in_flight(events: &[Event], (session, agent): (&str, &str)) -> bool {
    let limit = crate::autoreseat::episode(events, session, MAIN_SLOT, agent);
    let headroom = crate::autoreseat::headroom_episode(events, session, MAIN_SLOT, agent);
    let back = crate::autoreseat::return_episode(events, session, MAIN_SLOT, agent);
    crate::autoreseat::flying_any(&[limit.as_ref(), headroom.as_ref(), back.as_ref()]).is_some()
}

/// Why the automatic trigger yields at `slot`, or `None`: a requested reseat
/// of the seat is claimed, and its leg may not hold the seat's lock yet. The
/// automatic trigger asks under that same lock (orchself's one-line seam in
/// `autoreseat::leg::trigger`); this trigger asks the converse
/// ([`auto_in_flight`]) under it too.
/// Only in a session whose meta PROVES the orchestrator role, the only one
/// whose watchdog sweeps the file: a claim left behind by a removed role never
/// blocks an ordinary seat's automatic move.
#[must_use]
pub(crate) fn claimed_for(dir: &Path, slot: &str) -> Option<&'static str> {
    if slot != MAIN_SLOT
        || crate::store::source_presence(&dir.join(CLAIMED)) == crate::store::SourcePresence::Absent
    {
        return None;
    }
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    (crate::meta::meta_agent_role(&bytes) == crate::meta::MetaAgentRole::Role)
        .then_some("a requested reseat of this seat is claimed")
}

/// Whether a pending request waits: present and not damaged on its face. The
/// daemon's cheap question before it books a claim; the trigger reads it again.
pub(crate) fn pending(dir: &Path) -> bool {
    crate::store::source_presence(&dir.join(FILE)) == crate::store::SourcePresence::Regular
        && crate::store::source_presence(&dir.join(CLAIMED)) == crate::store::SourcePresence::Absent
}

/// The trigger: `send` under [`ATTEMPT_ACTION`], from the watchdog's cycle
/// that saw the seat idle. Under the seat's lock and the lifecycle lock, both
/// without waiting, it reads the request and the seat again, CLAIMS the
/// request (the rename to [`CLAIMED`]), journals the attempt, and only then,
/// past both locks, starts the leg. Anything that holds declines with one
/// line and writes nothing; a damaged or stale request is refused.
///
/// # Errors
///
/// Only a failure to write `err`.
pub(crate) fn trigger(
    dir: &Path,
    target: &str,
    own_session: &str,
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<u8> {
    let decline = |err: &mut dyn Write, why: &str| -> io::Result<u8> {
        writeln!(err, "ae: self reseat: {target} declined: {why}")?;
        Ok(EXIT_FAILED)
    };
    let (resolved, server) = match crate::tracked::resolve_on(target, own_session, dir) {
        Ok(found) if found.0.session == own_session && found.0.slot == MAIN_SLOT => found,
        _ => return decline(err, "not the main seat of this session"),
    };
    let Ok(seat_held) = seat_lock(dir) else {
        return decline(err, "another writer holds the seat");
    };
    let Ok(lifecycle) = lifecycle_lock(dir, Duration::ZERO) else {
        return decline(err, "a lifecycle operation holds the session");
    };
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    if crate::meta::meta_agent_role(&bytes) != crate::meta::MetaAgentRole::Role {
        return decline(err, "not an orchestrator session");
    }
    let agent = main_agent(&bytes);
    let seat = (own_session, agent.as_str());
    if read(dir, CLAIMED) != Found::Absent {
        return decline(err, "a claimed request is not settled");
    }
    let Some(events) = ledger(dir) else {
        return decline(err, "the journal cannot be read whole");
    };
    let request = match read(dir, FILE) {
        Found::Absent => return decline(err, "no request is queued"),
        Found::Damaged(why) => {
            let why = format!("damaged request: {why}");
            refuse_unrun(dir, seat, &events, (FILE, ""), &why, now, err)?;
            return Ok(EXIT_FAILED);
        }
        Found::Ready(request) => request,
    };
    if let Some(why) = request.binding().changed(&bytes) {
        let why = format!("stale request: {why}");
        refuse_unrun(dir, seat, &events, (FILE, &request.id), &why, now, err)?;
        return Ok(EXIT_FAILED);
    }
    // A request that already ended is never run, even when its binding reads
    // valid again: only its file is left, which goes now.
    if outcome_of(&events, seat, &request.id).is_some() {
        let left = remove(dir, FILE).err().map_or_else(String::new, |why| {
            format!("; {FILE} could not be removed ({why})")
        });
        return decline(err, &format!("this request already ended{left}"));
    }
    // A request already attempted is never claimed again.
    if attempted(&events, seat, &request.id) {
        let why = "refused: this request was already attempted";
        refuse_unrun(dir, seat, &events, (FILE, &request.id), why, now, err)?;
        return Ok(EXIT_FAILED);
    }
    // An automatic move journaled but not yet holding the seat's lock wins.
    if auto_in_flight(&events, seat) {
        return decline(err, "an automatic move of the seat is in flight");
    }
    // THE HOLDS, before the claim: nothing here is a refusal, and the request
    // stays pending for a later idle cycle.
    if let Some(what) =
        crate::autoreseat::leg::orchestrator_in_flight(dir, (own_session, MAIN_SLOT, &agent))
    {
        return decline(err, &what);
    }
    if let Some(why) = unsettled(&bytes, (&server, &resolved.pane), now) {
        return decline(err, why);
    }
    // THE CLAIM, under both locks: the rename, then the attempt record, the
    // durable edge past which the request never runs again. A record the
    // journal does not take leaves the claim for the sweep, which refuses it:
    // fail closed, nothing is started and nothing is released to run.
    if let Err(why) = std::fs::rename(dir.join(FILE), dir.join(CLAIMED)) {
        return decline(err, &format!("the request could not be claimed ({why})"));
    }
    let from = crate::lifecycle::meta_value(&bytes, &format!("profile.{MAIN_SLOT}"));
    if !watchdog_record(
        dir,
        seat,
        (
            ATTEMPT_ACTION,
            &request.id,
            &format!("from {from} to {}", request.to),
        ),
        now,
    ) {
        return decline(
            err,
            "the journal did not take the attempt; the sweep settles the claim",
        );
    }
    drop(lifecycle);
    drop(seat_held);
    start_leg(dir, seat, (&request.id, &bytes), now, err)
}

/// Start the detached leg of the attempt `id` just journaled, past both
/// locks. A leg that never started fails the attempt at once, under the
/// seat's lock again, while the journal still shows it open; a lock another
/// writer holds now leaves it to the sweep's overdue rule.
fn start_leg(
    dir: &Path,
    seat: (&str, &str),
    (id, bytes): (&str, &[u8]),
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<u8> {
    let exe = crate::shape::resolved_exe();
    let leg = crate::session_launch::capture::self_reseat_argv(dir, id);
    let started = exe
        .zip(leg)
        .is_some_and(|(exe, leg)| crate::transport::spawn_detached(&exe, &leg));
    if started {
        return Ok(0);
    }
    if let Ok(_held) = seat_lock(dir)
        && ledger(dir).is_some_and(|events| outcome_of(&events, seat, id).is_none())
    {
        let why = format!(
            "failed: the move could not be started; ae still records {}",
            records_now(bytes)
        );
        if watchdog_record(dir, seat, (FAILED_ACTION, id, &why), now) {
            said(dir, now, &format!("self reseat: {} — {why}", seat.1));
            let _ = remove(dir, CLAIMED);
        }
    }
    writeln!(err, "ae: self reseat: the move could not be started")?;
    Ok(EXIT_FAILED)
}

/// Why the seat is not settled for a move this moment, or `None`: one more
/// capture must read idle with no draft and no prompt only the human answers,
/// and no attached client may have typed within the auto reseat input grace.
fn unsettled(
    bytes: &[u8],
    (server, pane): (&crate::inventory::ServerId, &str),
    now: Timestamp,
) -> Option<&'static str> {
    let capture = crate::transport::capture_pane(server, pane);
    let bin = crate::lifecycle::meta_value(bytes, &format!("agent_bin.{MAIN_SLOT}"));
    let tool = crate::tool::ToolKind::from_binary_name(&bin);
    let text = capture.as_deref().unwrap_or_default();
    if capture.is_none()
        || crate::harness_state::classify(text, tool) != crate::harness_state::HarnessState::Idle
        || crate::harness_state::has_human_draft(text, tool)
    {
        return Some("the seat is not idle");
    }
    if crate::watchdog::human_prompt_class(text, &bin).is_some() {
        return Some("a prompt only the human answers is on screen");
    }
    let config = crate::doors::state_root(crate::shape::current())
        .map(|root| crate::doors::config_file(crate::shape::current(), &root));
    let grace = i64::try_from(crate::autoreseat::settings(config.as_deref()).grace_secs)
        .unwrap_or(i64::MAX);
    let clients = crate::transport::observe_clients(server);
    let input = crate::watchdog_daemon::viewing_input(clients.as_deref(), pane);
    typed_within(input, now, grace).then_some("a human typed in the pane within the input grace")
}

/// Whether input at `input` still holds the seat at `now`: until `grace`
/// seconds after it, that second itself excluded, as the automatic leg's
/// grace counts.
fn typed_within(input: Option<u64>, now: Timestamp, grace: i64) -> bool {
    input
        .and_then(|at| i64::try_from(at).ok())
        .is_some_and(|at| now.epoch() < at.saturating_add(grace))
}

/// The argv of the leg, proven before anything is read.
struct LegArgv {
    session: String,
    dir: std::path::PathBuf,
    id: String,
}

/// Prove `<dir> main <id>` against `sessions`, the state root's session
/// directory, as the automatic leg proves its own.
fn leg_argv(sessions: &Path, tail: &[String]) -> Result<LegArgv, String> {
    let usage = || LEG_USAGE.to_owned();
    let [dir, slot, id] = tail else {
        return Err(usage());
    };
    if slot != MAIN_SLOT || !crate::archive::is_canonical_uuid(id) {
        return Err(usage());
    }
    let dir = std::path::PathBuf::from(dir);
    if !dir.is_absolute()
        || dir.components().any(|part| {
            matches!(
                part,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(usage());
    }
    let session = dir
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .filter(|name| crate::session_launch::name::is_session_name(name))
        .ok_or_else(usage)?
        .to_owned();
    let root = crate::doors::canonical_strict_dir(sessions).map_err(|_| usage())?;
    let found = crate::doors::canonical_strict_dir(&dir).map_err(|_| usage())?;
    if found != root.join(&session) {
        return Err(usage());
    }
    Ok(LegArgv {
        session,
        dir: found,
        id: id.clone(),
    })
}

/// `_self-reseat <dir> main <id>`: the detached leg the trigger starts. It
/// holds the seat's lock for its whole run, acts only under the open attempt
/// `id` names and its claimed request, moves the seat through
/// [`super::run_requested`], and journals ONE ending, which is terminal.
///
/// # Errors
///
/// Only a failure to write `err`.
pub(crate) fn run_leg(
    root: &Path,
    tail: &[String],
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<u8> {
    let roots = crate::inventory::Roots::under(root);
    let argv = match leg_argv(roots.sessions(), tail) {
        Ok(argv) => argv,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(EXIT_USAGE);
        }
    };
    let dir = argv.dir.as_path();
    let Ok(_held) = seat_lock_within(dir, LEG_LOCK_WAIT) else {
        writeln!(
            err,
            "ae: self reseat: another writer holds {MAIN_SLOT} — nothing written."
        )?;
        return Ok(EXIT_FAILED);
    };
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let agent = main_agent(&bytes);
    let seat = (argv.session.as_str(), agent.as_str());
    let Some(events) = ledger(dir) else {
        writeln!(
            err,
            "ae: self reseat: the journal cannot be read whole — nothing was run."
        )?;
        return Ok(EXIT_FAILED);
    };
    if open_attempt(&events, seat).is_none_or(|(id, _)| id != argv.id) {
        writeln!(
            err,
            "ae: self reseat: no open attempt {} — nothing to do.",
            argv.id
        )?;
        return Ok(EXIT_FAILED);
    }
    // ONE ending per attempt, and the claimed file goes only once the journal
    // took it: a lost ending leaves the attempt open for the sweep's overdue
    // rule, never for a replay.
    let close = |action: &str, why: &str| {
        let journaled = watchdog_record(dir, seat, (action, &argv.id, why), now);
        if journaled {
            said(dir, now, &format!("self reseat: {agent} — {why}"));
        }
        journaled
    };
    let request = match read(dir, CLAIMED) {
        Found::Ready(request) if request.id == argv.id => request,
        Found::Ready(_) | Found::Absent => {
            let _ = close(
                REFUSED_ACTION,
                "refused: the claimed request is not this attempt's",
            );
            return Ok(EXIT_FAILED);
        }
        Found::Damaged(why) => {
            if close(REFUSED_ACTION, &format!("damaged request: {why}")) {
                let _ = remove(dir, CLAIMED);
            }
            return Ok(EXIT_FAILED);
        }
    };
    let from = crate::lifecycle::meta_value(&bytes, &format!("profile.{MAIN_SLOT}"));
    let (_, world) = crate::fleet_read::current_world(root);
    let mut said_bytes = Vec::new();
    let (ended, report) = super::run_requested(
        root,
        Some(&world),
        (&argv.session, &request.agent, &request.to),
        request.binding(),
        now,
        &mut io::sink(),
        &mut said_bytes,
    )?;
    err.write_all(&said_bytes)?;
    let (action, why, code) = ending(dir, (&from, &request.to), (ended, report), &said_bytes);
    let journaled = close(action, &why);
    if !journaled {
        writeln!(
            err,
            "ae: self reseat: the journal did not take the ending of {}; the attempt stays open \
             for the watchdog's overdue rule.",
            argv.id
        )?;
    } else if let Ok(_lifecycle) = lifecycle_lock(dir, Duration::from_secs(15)) {
        let _ = remove(dir, CLAIMED);
    }
    Ok(code)
}

/// The ONE ending record of a leg's move: its action, its summary (what the
/// executor said first, and for a failure what ae records now) and the exit.
fn ending(
    dir: &Path,
    (from, to): (&str, &str),
    (ended, report): (Ended, super::Reported),
    said_bytes: &[u8],
) -> (&'static str, String, u8) {
    let printed = String::from_utf8_lossy(said_bytes);
    let reason = printed
        .lines()
        .find(|line| line.starts_with("Error:"))
        .or_else(|| printed.lines().find(|line| !line.trim().is_empty()))
        .unwrap_or("the reseat printed no reason")
        .trim_start_matches("Error:")
        .trim()
        .to_owned();
    match ended {
        Ended::Moved { fate } => {
            let launch = report.minted.unwrap_or_default();
            let why = format!(
                "moved from {from} to {to} ({}){LAUNCH_TAIL}{launch}",
                fate.word()
            );
            (DONE_ACTION, why, 0)
        }
        Ended::Refused(class) => (
            REFUSED_ACTION,
            class.lead(&format!("refused: {reason}")),
            EXIT_FAILED,
        ),
        Ended::Failed => {
            let after = crate::meta::read_bytes(dir).unwrap_or_default();
            let why = format!("failed: {reason}; ae now records {}", records_now(&after));
            (FAILED_ACTION, why, EXIT_FAILED)
        }
    }
}

/// The refusal or failure the seat is owed as its next turn: the newest
/// terminal record whose reference no notice names yet.
pub(crate) fn owed_notice(events: &[Event], seat: (&str, &str)) -> Option<Event> {
    let ending = events.iter().rev().find(|event| {
        [REFUSED_ACTION, FAILED_ACTION]
            .iter()
            .any(|action| is(event, seat, action))
    })?;
    let reference = ending.reference.as_deref().filter(|id| !id.is_empty())?;
    let told = events.iter().any(|event| {
        event.action == NOTICE_ACTION && event.reference.as_deref() == Some(reference)
    });
    (!told).then(|| ending.clone())
}

/// The turn that tells the seat how its request ended, from the ending record.
#[must_use]
pub(crate) fn notice_text(ending: &Event, (session, agent): (&str, &str)) -> String {
    let why = ending
        .summary
        .as_deref()
        .unwrap_or("no reason was recorded");
    let again = format!("ae reseat {session} {agent} --using <profile[@client]>");
    if ending.action == FAILED_ACTION {
        format!(
            "ae: your requested reseat failed part-way and will not run again: {why}. If your \
             pane is back at a shell, the owner runs: ae @{session} relaunch {agent}. Queue a \
             new move with {again} once the cause is fixed."
        )
    } else {
        format!(
            "ae: your requested reseat was refused when the watchdog ran it, and it will not run \
             again: {why}. Nothing was stopped or moved. Queue it again with {again} once the \
             cause is fixed, or give the owner the command."
        )
    }
}

/// The continuation turn after a requested move, and the summary its record
/// carries.
#[must_use]
pub(crate) fn continuation(fate: crate::autoreseat::Fate) -> (String, String) {
    let word = fate.word();
    (
        format!(
            "your requested reseat ran: you moved to the profile you asked for, {}",
            crate::autoreseat::leg::CONTINUE
        ),
        format!("continuation turn after a requested {word} reseat"),
    )
}

/// The tail of a done record's summary that names the launch the move
/// started; the continuation reads the meta's own launch-bound row instead
/// ([`crate::meta::requested_continuation`]).
const LAUNCH_TAIL: &str = ", launch ";

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "d04bc042-f51e-4fea-a814-b8bcac07557e";
    const LAUNCH: &str = "b72543cb-c102-45c6-a4f6-81950a066560";
    const ID: &str = "0f4f1a2b-3c4d-4e5f-8a9b-0c1d2e3f4a5b";

    fn request() -> Request {
        Request {
            uuid: UUID.to_owned(),
            slot: MAIN_SLOT.to_owned(),
            agent: "orchestrator".to_owned(),
            launch: LAUNCH.to_owned(),
            to: "opus55x@cc-mic".to_owned(),
            id: ID.to_owned(),
            at: Timestamp::from_epoch(1_800_000_000),
        }
    }

    #[test]
    fn a_rendered_request_parses_back_to_itself() {
        let one = request();
        assert_eq!(parse(render(&one).as_bytes()), Ok(one));
    }

    #[test]
    fn the_exact_bytes_are_pinned() {
        assert_eq!(
            render(&request()),
            format!(
                "ae-self-reseat v1\nsession {UUID}\nslot main\nagent orchestrator\nlaunch \
                 {LAUNCH}\nto opus55x@cc-mic\nid {ID}\nat 2027-01-15T08:00:00Z\n"
            )
        );
    }

    #[test]
    fn every_damage_is_refused_with_its_reason() {
        let good = render(&request());
        let swap = |from: &str, to: &str| good.replacen(from, to, 1);
        let cases: Vec<(String, &str)> = vec![
            (String::new(), "does not end with a newline"),
            (good.trim_end().to_owned(), "does not end with a newline"),
            (format!("{good}\u{0}unexpected\n"), "lines, not 8"),
            (swap("ae-self-reseat v1", "ae-self-reseat v2"), "first line"),
            (swap("session ", "session  "), "session line"),
            (swap(UUID, &UUID.to_ascii_uppercase()), "session line"),
            (swap("slot main", "slot worker.1"), "slot line"),
            (swap("agent orchestrator", "agent -x"), "agent line"),
            (swap(&format!("launch {LAUNCH}"), "launch x"), "launch line"),
            (swap("to opus55x@cc-mic", "to a@b@c"), "to line"),
            (swap("to opus55x@cc-mic", "to @b"), "to line"),
            (swap("to opus55x@cc-mic", "to a=b"), "to line"),
            (swap(&format!("id {ID}"), "id 1"), "id line"),
            (swap("at 2027-01-15T08:00:00Z", "at yesterday"), "at line"),
            (swap("\nslot", "\r\nslot"), "session line"),
            (good.replace("agent orchestrator\n", ""), "lines, not 8"),
            ("x".repeat(1025), "more than 1024 bytes"),
            (swap("agent orchestrator", "agent orch\u{e9}"), "agent line"),
        ];
        for (bytes, reason) in cases {
            let parsed = parse(bytes.as_bytes());
            assert!(
                parsed.as_ref().is_err_and(|why| why.contains(reason)),
                "{bytes:?}: {parsed:?}"
            );
        }
        assert_eq!(parse(&[0xff, b'\n']), Err("it is not UTF-8".to_owned()));
    }

    #[test]
    fn a_destination_takes_one_client_at_most() {
        assert!(is_destination("opus55x"));
        assert!(is_destination("opus55x@cc-mic"));
        for bad in ["", "@", "a@", "@b", "a@b@c", "a b", "a=b", &"x".repeat(161)] {
            assert!(!is_destination(bad), "{bad:?}");
        }
        assert!(is_destination(&"x".repeat(160)));
    }

    #[test]
    fn a_binding_names_what_changed() {
        let bound = request().binding();
        let meta = |uuid: &str, seat: &str, launch: &str| {
            format!(
                "session_id={uuid}\nseat.main={seat}\nlaunch_id.main={launch}\nmeta_agent=true\n"
            )
        };
        assert_eq!(
            bound.changed(meta(UUID, "orchestrator", LAUNCH).as_bytes()),
            None
        );
        let changed = |bytes: String| bound.changed(bytes.as_bytes()).unwrap_or_default();
        assert!(changed(meta(ID, "orchestrator", LAUNCH)).contains("uuid"));
        assert!(changed(meta(UUID, "other", LAUNCH)).contains("now seats 'other'"));
        assert!(changed(meta(UUID, "orchestrator", ID)).contains("restarted"));
        assert!(changed(String::new()).contains("uuid"));
        let roleless =
            format!("session_id={UUID}\nseat.main=orchestrator\nlaunch_id.main={LAUNCH}\n");
        assert!(changed(roleless).contains("orchestrator role"));
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the fixture owns its scratch dir; the boundary is about what PRODUCT code \
                  may reach, and tests/it/phase3.rs cuts at the test module"
    )]
    fn removing_a_request_that_is_already_gone_is_done_and_any_other_failure_is_not() {
        let dir = std::env::temp_dir().join(format!("aerequest.{}.remove", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            std::fs::create_dir_all(dir.join("held")).is_ok(),
            "a scratch dir"
        );
        assert!(std::fs::write(dir.join(FILE), b"x").is_ok(), "a request");
        assert!(remove(&dir, FILE).is_ok(), "a present request");
        assert!(
            std::fs::symlink_metadata(dir.join(FILE)).is_err(),
            "is gone"
        );
        assert!(remove(&dir, FILE).is_ok(), "an absent one is already gone");
        assert!(
            remove(&dir, "held").is_err(),
            "a directory in its place is a failure, never done"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A damaged file's refusal is journaled under the digest of its bytes, or
    /// of its name and reason when there are none: the same damage is one
    /// reference, other damage another.
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the fixture owns its scratch dir; the boundary is about what PRODUCT code \
                  may reach, and tests/it/phase3.rs cuts at the test module"
    )]
    fn a_damaged_request_is_referenced_by_the_digest_of_its_bytes_or_its_reason() {
        let dir = std::env::temp_dir().join(format!("aerequest.{}.damaged", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a scratch dir");
        assert!(std::fs::write(dir.join(FILE), b"garbage").is_ok());
        let digest = crate::install::sha256_hex(b"garbage");
        let bytes = damaged_reference(&dir, FILE, "one reason");
        assert_eq!(
            bytes,
            format!("damaged-{}", digest.get(..32).unwrap_or_default())
        );
        assert_eq!(damaged_reference(&dir, FILE, "another reason"), bytes);
        assert!(std::fs::write(dir.join(FILE), b"other garbage").is_ok());
        assert_ne!(damaged_reference(&dir, FILE, "one reason"), bytes);
        let absent = damaged_reference(&dir, "absent", "why");
        let named = crate::install::sha256_hex(b"absent\nwhy");
        assert_eq!(
            absent,
            format!("damaged-{}", named.get(..32).unwrap_or_default())
        );
        assert_ne!(damaged_reference(&dir, "absent", "other"), absent);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The watchdog's pass looks at the seat whenever either request file is
    /// there: a damaged pending request with no claimed one beside it is
    /// refused on the journal and removed, and a seat with neither is left as
    /// it is, its journal untouched.
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the fixture owns its scratch dir; the boundary is about what PRODUCT code \
                  may reach, and tests/it/phase3.rs cuts at the test module"
    )]
    fn the_sweep_refuses_a_lone_damaged_request_and_leaves_an_empty_seat_alone() {
        let root = std::env::temp_dir().join(format!("aerequest.{}.sweep", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("sessions").join("work");
        assert!(
            std::fs::create_dir_all(&dir).is_ok(),
            "a scratch session dir"
        );
        let meta = crate::store::open(&dir).meta_path();
        assert!(std::fs::write(meta, "seat.main=orchestrator\n").is_ok());
        let now = Timestamp::from_epoch(1_800_000_000);
        let mut err = Vec::new();
        assert!(sweep(&dir, "work", now, &mut err).is_ok());
        assert_eq!(
            ledger(&dir).map(|events| events.len()),
            Some(0),
            "untouched"
        );
        assert!(std::fs::write(dir.join(FILE), b"garbage").is_ok());
        assert!(sweep(&dir, "work", now, &mut err).is_ok());
        assert!(
            std::fs::symlink_metadata(dir.join(FILE)).is_err(),
            "removed: {}",
            String::from_utf8_lossy(&err)
        );
        let events = ledger(&dir).unwrap_or_default();
        let seat = ("work", "orchestrator");
        assert!(
            events.iter().any(|event| is(event, seat, REFUSED_ACTION)
                && event
                    .reference
                    .as_deref()
                    .is_some_and(|reference| reference.starts_with("damaged-"))),
            "{events:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A claimed request whose attempt is journaled is booked failed only once
    /// that attempt has stood open for [`OVERDUE_SECS`], and its file goes with
    /// the record; before that the leg still reports and nothing changes.
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the fixture owns its scratch dir; the boundary is about what PRODUCT code \
                  may reach, and tests/it/phase3.rs cuts at the test module"
    )]
    fn an_open_attempt_is_failed_from_its_own_open_time_once_overdue() {
        let dir = std::env::temp_dir().join(format!("aerequest.{}.overdue", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a scratch dir");
        assert!(std::fs::write(dir.join(CLAIMED), b"claimed").is_ok());
        let meta = crate::store::open(&dir).meta_path();
        let records = format!("profile.main=claude-b\nlaunch_id.main={LAUNCH}\n");
        assert!(std::fs::write(meta, &records).is_ok());
        let seat = ("work", "orchestrator");
        let opened = Timestamp::from_epoch(1_800_000_000);
        assert!(watchdog_record(
            &dir,
            seat,
            (ATTEMPT_ACTION, ID, "attempt"),
            opened
        ));
        let events = ledger(&dir).unwrap_or_default();
        let settle = |now: i64| {
            let mut err = Vec::new();
            let now = Timestamp::from_epoch(opened.epoch().saturating_add(now));
            assert!(
                recover(
                    &dir,
                    (seat, b"".as_slice()),
                    &events,
                    Found::Ready(request()),
                    now,
                    &mut err
                )
                .is_ok()
            );
        };
        settle(OVERDUE_SECS - 1);
        assert!(
            std::fs::symlink_metadata(dir.join(CLAIMED)).is_ok(),
            "not yet overdue"
        );
        settle(OVERDUE_SECS);
        assert!(
            std::fs::symlink_metadata(dir.join(CLAIMED)).is_err(),
            "overdue: removed"
        );
        let failed = ledger(&dir).unwrap_or_default();
        assert!(
            failed
                .iter()
                .any(|event| event.reference.as_deref() == Some(ID)
                    && is(event, seat, FAILED_ACTION)
                    && event
                        .summary
                        .as_deref()
                        .is_some_and(|summary| summary.ends_with(&format!(
                            "ae now records profile 'claude-b' (launch {LAUNCH})"
                        )))),
            "{failed:?}"
        );
        assert_eq!(
            records_now(records.as_bytes()),
            format!("profile 'claude-b' (launch {LAUNCH})")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Input holds the seat for the grace after it, the closing second
    /// excluded; no input, or input past the clock's range, holds nothing.
    #[test]
    fn input_holds_the_seat_until_the_grace_after_it_ends() {
        let at = 1_800_000_000_u64;
        let now = |epoch: u64| Timestamp::from_epoch(i64::try_from(epoch).unwrap_or(i64::MAX));
        assert!(typed_within(Some(at), now(at), 30));
        assert!(typed_within(Some(at), now(at + 29), 30));
        assert!(
            !typed_within(Some(at), now(at + 30), 30),
            "the grace has ended"
        );
        assert!(!typed_within(None, now(at), 30));
        assert!(!typed_within(Some(u64::MAX), now(at), 30));
    }

    /// A requested move's continuation tells the seat its own request ran, and
    /// its record names the fate, apart from an automatic move's.
    #[test]
    fn a_requested_moves_continuation_says_the_request_ran_and_names_its_fate() {
        use crate::autoreseat::Fate;
        let ran = format!(
            "your requested reseat ran: you moved to the profile you asked for, {}",
            crate::autoreseat::leg::CONTINUE
        );
        for (fate, word) in [(Fate::Carried, "carried"), (Fate::Kept, "kept")] {
            let (body, summary) = continuation(fate);
            assert_eq!(body, ran);
            assert_eq!(
                summary,
                format!("continuation turn after a requested {word} reseat")
            );
            assert_ne!(summary, crate::autoreseat::leg::continuation(fate).1);
        }
    }

    /// The seat may name only itself: its own session and its own agent, both.
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the fixture owns its scratch dir; the boundary is about what PRODUCT code \
                  may reach, and tests/it/phase3.rs cuts at the test module"
    )]
    fn the_seat_names_only_itself_by_both_its_session_and_its_agent() {
        let dir = std::env::temp_dir().join(format!("aerequest.{}.own", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a scratch dir");
        let meta = crate::store::open(&dir).meta_path();
        assert!(std::fs::write(meta, "seat.main=orchestrator\n").is_ok());
        let own = |session: &str| session == "work";
        let seat = Seat {
            session: "work",
            dir: &dir,
            uuid: UUID,
            slot: MAIN_SLOT,
            own: &own,
        };
        assert_eq!(not_own(&seat, "work", "orchestrator"), None);
        let refusal = Some(only_itself("work", "orchestrator"));
        assert_eq!(not_own(&seat, "work", "lead"), refusal, "another agent");
        assert_eq!(
            not_own(&seat, "other", "orchestrator"),
            refusal,
            "another session"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A bad argv from the seat is refused before anything is read: the
    /// parse error, then the usage, and the usage alone exactly once.
    #[test]
    fn a_bad_queue_argv_prints_its_error_and_the_usage_once() {
        let own = |_: &str| true;
        let seat = Seat {
            session: "work",
            dir: Path::new("/nonexistent/orchself"),
            uuid: UUID,
            slot: MAIN_SLOT,
            own: &own,
        };
        let run = |words: &[&str]| {
            let tail: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
            let mut err = Vec::new();
            let verdict = checked(&seat, &tail, Timestamp::from_epoch(1_800_000_000), &mut err)
                .ok()
                .and_then(Result::err);
            (verdict, String::from_utf8_lossy(&err).into_owned())
        };
        let usage = super::super::RESEAT_USAGE;
        assert_eq!(run(&[]), (Some(EXIT_USAGE), format!("{usage}\n")));
        assert_eq!(
            run(&["work", "orchestrator"]),
            (
                Some(EXIT_USAGE),
                format!("Error: reseat needs --using <profile>.\n{usage}\n")
            )
        );
    }

    /// A record's summary is one clean line, kept whole up to the cap and cut
    /// with an ellipsis past it, so the cut line is exactly the cap long.
    #[test]
    fn a_summary_is_one_line_kept_whole_to_the_cap_and_cut_past_it() {
        assert_eq!(bounded(" moved\nto b\t"), "moved to b");
        let at_cap = "x".repeat(SUMMARY_MAX);
        assert_eq!(bounded(&at_cap), at_cap, "the cap itself is kept whole");
        let past = bounded(&"y".repeat(SUMMARY_MAX + 1));
        assert_eq!(past.chars().count(), SUMMARY_MAX, "{past}");
        assert_eq!(past, format!("{}…", "y".repeat(SUMMARY_MAX - 1)));
    }

    /// The line grammar on its own, apart from each field's filter: one space
    /// after the key, then one word with no whitespace and no control byte.
    #[test]
    fn a_value_is_one_nonempty_word_after_one_space() {
        assert_eq!(super::value("to fake-b", "to"), Some("fake-b"));
        assert_eq!(super::value("to ", "to"), None, "an empty value");
        assert_eq!(super::value("to a b", "to"), None, "a space inside");
        assert_eq!(super::value("to a\u{7}", "to"), None, "a control byte");
        assert_eq!(super::value("to  a", "to"), None, "two spaces");
        assert_eq!(super::value("tox a", "to"), None, "another key");
    }

    #[test]
    fn cancel_takes_two_names_and_no_other_flag() {
        let words = |list: &[&str]| list.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            cancel_names(&words(&["s", "--cancel", "a"])),
            Ok(("s", "a"))
        );
        for bad in [
            vec!["s", "--cancel"],
            vec!["s", "a", "b", "--cancel"],
            vec!["s", "a", "--cancel", "--using"],
            vec!["-s", "a", "--cancel"],
            vec!["s", "-a", "--cancel"],
        ] {
            assert!(cancel_names(&words(&bad)).is_err(), "{bad:?}");
        }
    }

    fn event(action: &str, reference: &str) -> Event {
        let line = format!(
            r#"{{"ts":"2027-01-15T08:00:00Z","actor":"watchdog","action":"{action}","target":"orchestrator","target_slot":"main","target_session":"s","ref":"{reference}","summary":"why"}}"#
        );
        Event::parse_line(&line).expect("a fixture event")
    }

    #[test]
    fn a_notice_is_owed_once_per_terminal_record() {
        let seat = ("s", "orchestrator");
        assert_eq!(owed_notice(&[], seat), None);
        let refused = event(REFUSED_ACTION, ID);
        assert_eq!(
            owed_notice(std::slice::from_ref(&refused), seat),
            Some(refused.clone())
        );
        let told = event(NOTICE_ACTION, ID);
        assert_eq!(owed_notice(&[refused.clone(), told.clone()], seat), None);
        let failed = event(FAILED_ACTION, LAUNCH);
        assert_eq!(
            owed_notice(&[refused, told, failed.clone()], seat),
            Some(failed)
        );
        assert_eq!(owed_notice(&[event(DONE_ACTION, ID)], seat), None);
    }

    #[test]
    fn an_open_attempt_is_one_with_no_outcome() {
        let seat = ("s", "orchestrator");
        let attempt = event(ATTEMPT_ACTION, ID);
        assert_eq!(
            open_attempt(std::slice::from_ref(&attempt), seat).map(|(id, _)| id),
            Some(ID.to_owned())
        );
        for ending in [DONE_ACTION, REFUSED_ACTION, FAILED_ACTION] {
            assert_eq!(
                open_attempt(&[attempt.clone(), event(ending, ID)], seat),
                None
            );
        }
        assert!(open_attempt(&[attempt, event(DONE_ACTION, LAUNCH)], seat).is_some());
    }

    #[test]
    fn an_attempt_is_the_edge_and_a_refusal_is_journaled_once_per_queue() {
        let seat = ("s", "orchestrator");
        assert!(!attempted(&[], seat, ID));
        assert!(attempted(&[event(ATTEMPT_ACTION, ID)], seat, ID));
        assert!(attempted(
            &[event(ATTEMPT_ACTION, ID), event(DONE_ACTION, ID)],
            seat,
            ID
        ));
        assert!(!attempted(&[event(ATTEMPT_ACTION, LAUNCH)], seat, ID));
        let refused = event(REFUSED_ACTION, "damaged-x");
        assert!(refused_already(
            std::slice::from_ref(&refused),
            seat,
            "damaged-x"
        ));
        assert!(!refused_already(
            std::slice::from_ref(&refused),
            seat,
            "damaged-y"
        ));
        // A new queue after the refusal opens a new generation.
        assert!(!refused_already(
            &[refused, event(QUEUED_ACTION, ID)],
            seat,
            "damaged-x"
        ));
    }
}
