//! The orchestrator seat's tracked relay into another session's chat.
//!
//! `relay <session[:agent]> <text…>` joins the remaining argv with one space
//! and carries it as ONE chat ask of a lead-pair seat of that session (bare:
//! its main seat) — the target chat's own ask, framed as the human's, whose
//! opening records the orchestrator's session id as `relay_origin`. It fails
//! closed unless tmux proves the caller is inside a session whose meta says
//! exactly one `meta_agent=true`.
//!
//! One invocation, in this order. Every refusal before the claim records a
//! ref-less `relay` row in the caller's journal alone and prints nothing on
//! stdout. The claim — the exact text in the caller's `messages/` and one
//! `relay-attempt` — is durable before any target key. Then the target chat's
//! writer lease and admission lock are taken without waiting, and under them
//! the caller, the target incarnation, the lead pair and the relay cap are
//! proved again; the ask is carried by the console's own tracked path, which
//! is the only writer in the target. Its request is read back, the locks are
//! dropped, and the caller's journal gets one `relay-result` and at most one
//! `ae:relay` chat echo. stdout is the receipt, three lines: the id,
//! `outcome: sent|uncertain|not-delivered` and
//! `request: pending|accepted|replied|cancelled|retired|absent|unknown`.
//!
//! `relay --status <id>` reads one claimed relay back and never delivers. It
//! writes nothing in the target, only the caller's missing echoes.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::config;
use crate::console::{self, Console, lane::Seat, term};
use crate::deliver;
use crate::events::{Event, SessionKey};
use crate::inventory::ServerId;
use crate::meta;
use crate::requests::{self, Status, is_slot};
use crate::session_launch::name::is_session_name;
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::store::{self, SourceRead};
use crate::time::Timestamp;
use crate::tmux::ObservedViewer;
use crate::tracked::{self, EventFields, Kind, Receipt};
use crate::{archive, lifecycle};

/// The helper's argv contract.
pub const USAGE: &str =
    "Usage: relay <session[:agent]> <text…>\n       relay --status <request-id>\n";

/// The flag that reads a relay back instead of sending one.
const STATUS_FLAG: &str = "--status";

/// A relay refused before it claimed anything: the legacy ref-less row.
pub const ACTION: &str = "relay";

/// The claim, durable before any target key.
pub const ATTEMPT: &str = "relay-attempt";

/// What the delivery did, recorded once the request is read back.
pub const RESULT: &str = "relay-result";

/// The actor of the caller's chat echo.
pub const ECHO_ACTOR: &str = "ae:relay";

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

/// The caller and what its meta grants it: the role, the canonical
/// `session_id` and the seat's `launch_id`, each empty when absent. Proved
/// before the claim and proved equal again under the target's locks.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Grant {
    caller: Caller,
    session_id: String,
    launch: String,
    role: meta::MetaAgentRole,
}

impl Grant {
    fn of(dir: &Path, actual: Option<&(ObservedViewer, ServerId)>) -> Result<Self, &'static str> {
        let (caller, bytes) = caller(dir, actual.map(|(viewer, server)| (viewer, server)))?;
        let session_id = archive::canonical_uuid(&lifecycle::meta_value(&bytes, "session_id"));
        let launch = tracked::launch_id_at(&caller.dir, &caller.slot, &session_id);
        Ok(Self {
            caller,
            session_id,
            launch,
            role: meta::meta_agent_role(&bytes),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    session: String,
    display: String,
}

impl Target {
    fn agent(&self) -> &str {
        self.display.split_once(':').map_or("", |(_, agent)| agent)
    }
}

/// Everything proved before the claim: the target chat, its lead pair, the
/// seat asked and the ask ready to carry.
struct Prepared {
    target: Target,
    console: Console,
    pair: Vec<Seat>,
    seat: Seat,
    carry: term::Carry,
}

/// Run one relay end to end.
///
/// `dir` is the helper directory used to enter the core; `actual` reads the
/// caller identity and server from the actual `$TMUX` socket, once before the
/// claim and again under the target's locks.
///
/// # Errors
///
/// Only a failure to write `out` or `err`.
pub fn run(
    dir: &Path,
    tail: &[String],
    actual: impl Fn() -> Option<(ObservedViewer, ServerId)>,
    now: Timestamp,
    entropy: u64,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    match tail {
        [flag, id] if flag == STATUS_FLAG => {
            return status(dir, id, actual().as_ref(), now, out, err);
        }
        [flag, ..] if flag.starts_with('-') => {
            write!(err, "{USAGE}")?;
            return Ok(EXIT_USAGE);
        }
        _ => {}
    }
    let Ok(parsed) = parse(tail) else {
        write!(err, "{USAGE}")?;
        return Ok(EXIT_USAGE);
    };
    let grant = match Grant::of(dir, actual().as_ref()) {
        Ok(grant) => grant,
        Err(reason) => {
            // With no proven caller there is no trustworthy session ledger to
            // write. The refusal itself still stays one line and fails closed.
            writeln!(err, "ae: relay REFUSED — {reason}. Nothing was sent.")?;
            return Ok(EXIT_FAILED);
        }
    };
    let caller = &grant.caller;
    let prepared = match prepare(&grant, &parsed, now, entropy) {
        Ok(prepared) => prepared,
        Err(reason) => return refuse(caller, &parsed, &reason, now, err),
    };
    let id = prepared.carry.id().to_owned();
    if let Err(why) = claim(&grant, &prepared, &parsed, now) {
        writeln!(
            err,
            "ae: relay REFUSED — the attempt could not be claimed ({why}). Nothing was sent."
        )?;
        return Ok(EXIT_FAILED);
    }
    let Prepared {
        target,
        console,
        pair,
        seat,
        carry,
    } = prepared;
    let again = || {
        let now = Grant::of(dir, actual().as_ref())
            .map_err(|why| format!("the orchestrator caller is unproven now ({why})"))?;
        if now == grant {
            Ok(())
        } else {
            Err("the orchestrator caller changed since the claim".to_owned())
        }
    };
    let (receipt, request) = match term::admit_relay(&console, &pair, &seat.name, again) {
        Err(why) => {
            writeln!(err, "ae: relay {id} not delivered: {why}")?;
            (Receipt::NotDelivered, read_back(&console, &id))
        }
        Ok(admitted) => {
            let (said, receipt) = carry.run_typed();
            err.write_all(said.as_bytes())?;
            let request = read_back(&console, &id);
            drop(admitted);
            (receipt.unwrap_or(Receipt::NotDelivered), request)
        }
    };
    let outcome = receipt.word();
    let request = request.unwrap_or_else(|gap| {
        let _ = writeln!(err, "ae: relay {id}: request read gap: {gap}");
        "unknown"
    });
    let said = Said {
        target: &target,
        seat: &seat,
        id: &id,
        text: &parsed.text,
    };
    record(caller, &said, outcome, request, err)?;
    writeln!(out, "{id}\noutcome: {outcome}\nrequest: {request}")?;
    Ok(if receipt == Receipt::Sent {
        0
    } else {
        EXIT_FAILED
    })
}

/// What one relay said to whom, for the records that close it.
struct Said<'a> {
    target: &'a Target,
    seat: &'a Seat,
    id: &'a str,
    text: &'a str,
}

/// The caller's closing records: one `relay-result` carrying the transport
/// outcome, then the `ae:relay` chat echo unless one names this id already.
/// Either failing is said on stderr; the receipt still prints.
fn record(
    caller: &Caller,
    said: &Said<'_>,
    outcome: &str,
    request: &str,
    err: &mut impl Write,
) -> io::Result<()> {
    let id = said.id;
    let summary = format!("{outcome}; request {request}");
    let fields = EventFields::new(
        Timestamp::now(),
        &caller.actor,
        RESULT,
        &said.target.display,
        id,
        &caller.slot,
        &caller.session,
        &said.seat.slot,
        &said.target.session,
        &summary,
        "",
    );
    let line = tracked::event_line_with(&fields, "", &[("outcome", outcome)]);
    let journal = store::open(&caller.dir);
    let result = journal.append_event_decided(|read| console::snapshot(read).map(|_| line));
    if let Err(why) = flatten(result) {
        writeln!(
            err,
            "ae: relay {id} ({outcome}): its result was not recorded: {why}"
        )?;
    }
    let line = receipt_echo(&said.target.session, said.text, outcome, id);
    match echo(&journal, id, line, |summary| {
        summary.starts_with(RECEIPT_ECHO)
    }) {
        Err(why) => writeln!(
            err,
            "ae: relay {id} ({outcome}): its chat echo was not recorded: {why}"
        ),
        Ok(()) => Ok(()),
    }
}

/// How a receipt echo's summary begins.
const RECEIPT_ECHO: &str = "routed to ";

/// How a reply echo's summary begins, before the answer's body nonce.
const REPLY_ECHO: &str = "reply ";

/// The caller's receipt echo of relay `id` to `session`.
fn receipt_echo(session: &str, text: &str, outcome: &str, id: &str) -> String {
    let echo = format!(
        "{RECEIPT_ECHO}{session}: {} ({outcome}), request {id}",
        console::excerpt(text)
    );
    crate::state::event_line(Timestamp::now(), ECHO_ACTOR, "chat", id, &echo)
}

/// Append echo `line` of relay `id` unless the journal, read strictly under its
/// lock, already holds an echo of `id` whose summary `same` recognises.
fn echo(
    journal: &store::SessionStore,
    id: &str,
    line: String,
    same: impl Fn(&str) -> bool,
) -> Result<(), String> {
    let decided = journal.append_event_decided(|read| {
        let events = console::snapshot(read)?;
        if events
            .iter()
            .filter_map(|event| echo_of(event, id))
            .any(&same)
        {
            return Err(String::new());
        }
        Ok(line)
    });
    match flatten(decided) {
        Err(why) if !why.is_empty() => Err(why),
        _ => Ok(()),
    }
}

/// The summary of `event` when it is an echo of relay `id`.
fn echo_of<'a>(event: &'a Event, id: &str) -> Option<&'a str> {
    let echo = event.action == "chat"
        && event.actor == ECHO_ACTOR
        && event.reference.as_deref() == Some(id);
    echo.then(|| event.summary.as_deref().unwrap_or(""))
}

/// The body nonce a reply echo's summary names.
fn echoed_nonce(summary: &str) -> Option<&str> {
    let nonce = summary.strip_prefix(REPLY_ECHO)?.split(' ').next()?;
    console::is_body_nonce(nonce).then_some(nonce)
}

/// Every proof before the claim, in order: the orchestrator role, a text with
/// words, the caller's own session id, the target grammar, the target chat's
/// incarnation and lead pair with the seat in it, and the exact framed ask
/// within the paste bound.
fn prepare(
    grant: &Grant,
    parsed: &Parsed,
    now: Timestamp,
    entropy: u64,
) -> Result<Prepared, String> {
    if grant.role != meta::MetaAgentRole::Role {
        return Err("caller session is not an orchestrator".to_owned());
    }
    if parsed.text.trim().is_empty() {
        return Err("text is empty".to_owned());
    }
    if grant.session_id.is_empty() {
        return Err("caller session records no usable session id".to_owned());
    }
    let caller = &grant.caller;
    let target = target(caller, &parsed.target)?;
    let target_dir = caller
        .dir
        .parent()
        .map(|root| root.join(&target.session))
        .ok_or("target session could not be resolved")?;
    let session = &target.session;
    let bytes = meta::read_bytes(&target_dir)
        .map_err(|_| format!("target session '{session}' meta could not be read"))?;
    let uuid = archive::canonical_uuid(&lifecycle::meta_value(&bytes, "session_id"));
    if uuid.is_empty() {
        return Err(format!(
            "target session '{session}' records no usable session id"
        ));
    }
    let console = Console::bound(session.clone(), target_dir.clone(), uuid);
    let pair = console
        .seats()
        .and_then(term::pair_of)
        .map_err(|why| format!("target session '{session}' has no lead pair ({why})"))?;
    let Some(seat) = pair
        .iter()
        .find(|seat| seat.name == target.agent())
        .cloned()
    else {
        let names: Vec<_> = pair.iter().map(|seat| seat.name.as_str()).collect();
        return Err(format!(
            "target must be a lead-pair seat ({})",
            names.join(", ")
        ));
    };
    let carry = term::Carry::relayed(
        &console,
        &seat.name,
        parsed.text.clone(),
        (now, entropy),
        grant.session_id.clone(),
    );
    let id = carry.id();
    let own = crate::own_session(&target_dir);
    let label = Kind::Ask.reply_label();
    let footer = tracked::reply_footer_command(&target_dir, &own, &seat.name, id, label);
    let ask = tracked::compose(
        Kind::Ask,
        id,
        tracked::CONSOLE_DISPLAY,
        &parsed.text,
        &footer,
    );
    let framed = deliver::framed_send(tracked::CONSOLE_DISPLAY, &ask).len();
    if framed as u64 > deliver::notice::LIMIT {
        return Err(format!(
            "the relayed ask frames to {framed} B; the relay limit is {} B",
            deliver::notice::LIMIT
        ));
    }
    Ok(Prepared {
        target,
        console,
        pair,
        seat,
        carry,
    })
}

/// The claim: the exact text stored in the caller's `messages/`, then one
/// `relay-attempt` under that id, appended only to a strictly whole journal
/// that has never claimed it.
fn claim(
    grant: &Grant,
    prepared: &Prepared,
    parsed: &Parsed,
    now: Timestamp,
) -> Result<(), String> {
    let caller = &grant.caller;
    let id = prepared.carry.id();
    let body_file = deliver::store_body(&caller.dir, id, ATTEMPT, &parsed.text)?;
    let body_file = body_file.to_string_lossy();
    let uuid = prepared.console.uuid();
    let observed = tracked::observe_caller(&caller.dir);
    let fields = EventFields {
        actor_session_id: &grant.session_id,
        target_session_id: uuid,
        target_session_uuid: uuid,
        identity_gap: observed.as_ref().err().map_or("", |gap| gap.name()),
        ..EventFields::new(
            now,
            &caller.actor,
            ATTEMPT,
            &prepared.target.display,
            id,
            &caller.slot,
            &caller.session,
            &prepared.seat.slot,
            &prepared.target.session,
            &parsed.text,
            &body_file,
        )
        .with_caller(observed.as_ref().ok())
    };
    let line = tracked::event_line_with(&fields, &grant.launch, &[]);
    let decided = store::open(&caller.dir).append_event_decided(|read| {
        let events = console::snapshot(read)?;
        let claimed =
            |event: &Event| event.action == ATTEMPT && event.reference.as_deref() == Some(id);
        if events.iter().any(claimed) {
            return Err(format!("{id} is already claimed"));
        }
        Ok(line)
    });
    flatten(decided)
}

fn flatten(decided: Result<Result<(), String>, store::Error>) -> Result<(), String> {
    decided.map_err(|why| why.to_string())?
}

/// What the target's journal, read strictly and without a lock, says of
/// request `id` in the claimed session: the word, or the read gap.
fn read_back(console: &Console, id: &str) -> Result<&'static str, String> {
    let (bytes, _) = target_journal(console)?;
    Ok(request_in(console, &bytes, id))
}

/// The target's journal, read strictly and without a lock: bytes and records.
fn target_journal(console: &Console) -> Result<(Vec<u8>, Vec<Event>), String> {
    let read = store::read_source(&store::open(console.dir()).events_path());
    let bytes = match &read {
        SourceRead::Ready(bytes) => bytes.clone(),
        _ => Vec::new(),
    };
    Ok((bytes, console::snapshot(read)?))
}

/// The state of request `id` in the target journal `bytes` of `console`.
fn request_in(console: &Console, bytes: &[u8], id: &str) -> &'static str {
    let states = requests::states_in(bytes, SessionKey::new(console.name(), console.uuid()));
    let Some(request) = states.iter().find(|request| request.id == id.as_bytes()) else {
        return "absent";
    };
    match request.status {
        Status::Pending if request.accepted.is_some() => "accepted",
        Status::Pending => "pending",
        Status::Replied => "replied",
        Status::Cancelled => "cancelled",
        Status::Retired => "retired",
    }
}

/// One relay as its orchestrator's own journal claimed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    /// The target's name when the relay ran: shown, never followed.
    pub target_session: String,
    /// The target's `session_id` when the relay ran: its identity.
    pub target_uuid: String,
    pub target_slot: String,
    pub text: String,
    /// The recorded outcome; `None`, read as uncertain, when none was.
    pub outcome: Option<&'static str>,
}

/// Relay `id` as the journal of the orchestrator whose `session_id` is
/// `origin` claims it: one `relay-attempt` of that origin, at most one
/// `relay-result` with a known outcome. A ref-less `relay` refusal never
/// matches.
///
/// # Errors
///
/// No such claim, or an ambiguous one.
pub fn claimed(events: &[Event], id: &str, origin: &str) -> Result<Claim, String> {
    let of_id = |action: &'static str| {
        let of =
            move |event: &&Event| event.action == action && event.reference.as_deref() == Some(id);
        events.iter().filter(of)
    };
    let mut attempts = of_id(ATTEMPT);
    let (Some(attempt), None) = (attempts.next().filter(|_| !id.is_empty()), attempts.next())
    else {
        return Err(format!("relay {id} is not claimed exactly once here"));
    };
    if origin.is_empty() || attempt.actor_session_id.value() != Some(origin) {
        return Err(format!("relay {id} was claimed by another orchestrator"));
    }
    let uuid = attempt.target_session_id.value();
    let uuid = uuid.filter(|uuid| crate::events::is_session_id(uuid));
    let slot = attempt.target_slot.value().filter(|slot| is_slot(slot));
    let session = attempt.target_session.value().unwrap_or_default();
    let (Some(uuid), Some(slot)) = (uuid, slot) else {
        return Err(format!("the claim of relay {id} names no usable target"));
    };
    let mut results = of_id(RESULT);
    let outcome = match (results.next(), results.next()) {
        (None, _) => None,
        (Some(result), None) => [Receipt::Sent, Receipt::Uncertain, Receipt::NotDelivered]
            .into_iter()
            .map(Receipt::word)
            .find(|word| result.outcome.as_deref() == Some(*word))
            .map(Some)
            .ok_or_else(|| format!("relay {id} records an unknown outcome"))?,
        (Some(_), Some(_)) => return Err(format!("relay {id} has more than one result")),
    };
    Ok(Claim {
        target_session: session.to_owned(),
        target_uuid: uuid.to_owned(),
        target_slot: slot.to_owned(),
        text: attempt.summary.clone().unwrap_or_default(),
        outcome,
    })
}

/// Where in the target's `events` the one console ask opening relay `id`
/// sits, bound to `claim`: relayed by `origin`, to its seat, and every session
/// stamp it carries, at least one, the claimed one. `None` when the target has
/// no opening: a relay that crashed before its ask was recorded.
///
/// # Errors
///
/// A duplicated, foreign or mis-stamped opening.
pub fn opening(
    events: &[Event],
    id: &str,
    origin: &str,
    claim: &Claim,
) -> Result<Option<usize>, String> {
    let mut asks = events.iter().enumerate().filter(|(_, event)| {
        event.actor == tracked::CONSOLE_SINK
            && event.action == Kind::Ask.action()
            && event.reference.as_deref() == Some(id)
    });
    let Some((at, ask)) = asks.next() else {
        return Ok(None);
    };
    let pane = ask.target_session_uuid.as_deref().filter(|s| !s.is_empty());
    let routed = ask.target_session_id.value().filter(|s| !s.is_empty());
    let uuid = Some(claim.target_uuid.as_str());
    // At least one session stamp, and every one the claimed session.
    let bound = asks.next().is_none()
        && !origin.is_empty()
        && ask.relay_origin.as_deref() == Some(origin)
        && ask.target_slot.value() == Some(claim.target_slot.as_str())
        && pane.or(routed) == uuid
        && routed.or(pane) == uuid;
    if !bound {
        return Err(format!("request {id} is not this relay's one opening"));
    }
    Ok(Some(at))
}

/// An admitted answer, read whole.
struct Answer {
    seat: String,
    ts: Timestamp,
    nonce: String,
    body: String,
}

/// What `relay --status` read: the claim, then what its target says so far,
/// up to `gap`, the first read after the claim that failed.
struct Found {
    caller: PathBuf,
    claim: Claim,
    request: &'static str,
    answers: Vec<Answer>,
    gap: Option<String>,
}

/// `relay --status <id>`: print what relay `id` became and repair the
/// caller's missing echoes. rc0 for a complete read whatever the outcome, rc1
/// for a gap in what it reads now, which repairs nothing, or an echo it could
/// not write.
fn status(
    dir: &Path,
    id: &str,
    actual: Option<&(ObservedViewer, ServerId)>,
    now: Timestamp,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let read = Grant::of(dir, actual).map_err(str::to_owned);
    let found = match read.and_then(|grant| read_status(&grant, id)) {
        Ok(found) => found,
        Err(gap) => {
            writeln!(err, "ae: relay --status {id} REFUSED — {gap}.")?;
            return Ok(EXIT_FAILED);
        }
    };
    let outcome = found.claim.outcome.unwrap_or(Receipt::Uncertain.word());
    let request = found.request;
    write!(out, "{id}\noutcome: {outcome}\nrequest: {request}\n")?;
    writeln!(out, "read: {now}")?;
    if found.claim.outcome.is_none() {
        writeln!(err, "ae: relay {id}: no result recorded, so uncertain")?;
    }
    for Answer { seat, ts, body, .. } in &found.answers {
        let body = crate::board::terminal_text(body);
        let end = if body.ends_with('\n') { "" } else { "\n" };
        write!(out, "reply from {seat} at {ts}:\n{body}{end}")?;
    }
    if let Some(gap) = &found.gap {
        writeln!(err, "ae: relay {id}: read gap: {gap}")?;
        return Ok(EXIT_FAILED);
    }
    if let Err(why) = repair(&found, id, outcome) {
        writeln!(err, "ae: relay {id}: an echo was not recorded: {why}")?;
        return Ok(EXIT_FAILED);
    }
    Ok(0)
}

/// Every strict read of `relay --status`, in order: the role and the caller's
/// own claim of `id`, without which nothing is said, then its target.
fn read_status(grant: &Grant, id: &str) -> Result<Found, String> {
    if grant.role != meta::MetaAgentRole::Role {
        return Err("caller session is not an orchestrator".to_owned());
    }
    let caller = &grant.caller;
    let journal = store::read_source(&store::open(&caller.dir).events_path());
    let events = console::snapshot(journal).map_err(|why| format!("its own journal: {why}"))?;
    let claim = claimed(&events, id, &grant.session_id)?;
    let mut found = Found {
        caller: caller.dir.clone(),
        claim,
        request: "unknown",
        answers: Vec::new(),
        gap: None,
    };
    found.gap = read_target(&mut found, id, &grant.session_id).err();
    Ok(found)
}

/// The target of `found`'s claim, by the session id it recorded: the one
/// opening, then the request state and each admitted answer whole.
fn read_target(found: &mut Found, id: &str, origin: &str) -> Result<(), String> {
    let console = bound_target(&found.caller, &found.claim.target_uuid)?;
    let (bytes, events) =
        target_journal(&console).map_err(|why| format!("the target journal: {why}"))?;
    if opening(&events, id, origin, &found.claim)?.is_none() {
        found.request = "absent";
        return Ok(());
    }
    found.request = request_in(&console, &bytes, id);
    for (at, seat) in console::lane::answers(console.name(), &events, id) {
        let reply = &events[at];
        let body = match console::body_for(console.dir(), reply) {
            console::lane::Body::Whole(body) => body,
            gap => {
                let why = console::lane::body_gap(&gap).unwrap_or_default();
                return Err(format!("the reply from {seat} cannot be read whole: {why}"));
            }
        };
        let Some(nonce) = console::stored_nonce(reply) else {
            return Err(format!("the reply from {seat} names no stored body"));
        };
        let (ts, nonce) = (reply.ts, nonce.to_owned());
        found.answers.push(Answer {
            seat,
            ts,
            nonce,
            body,
        });
    }
    Ok(())
}

/// The one session that holds `uuid` now; never the relay's recorded name.
fn bound_target(caller_dir: &Path, uuid: &str) -> Result<Console, String> {
    let root = caller_dir.parent().and_then(Path::parent);
    let root = root.ok_or("target session could not be resolved")?;
    let names = lifecycle::sessions_with_id(root, uuid)
        .map_err(|why| format!("cannot enumerate sessions to find {uuid} ({why})"))?;
    let [name] = names.as_slice() else {
        let held = names.len();
        return Err(format!(
            "{held} sessions hold the target session id {uuid}, not one"
        ));
    };
    let dir = lifecycle::sessions_dir(root).join(name);
    Ok(Console::bound(name.clone(), dir, uuid.to_owned()))
}

/// The receipt echo, then one echo per answer keyed by its body nonce, each
/// written only where the journal, read under its lock, has none.
fn repair(found: &Found, id: &str, outcome: &str) -> Result<(), String> {
    let journal = store::open(&found.caller);
    let claim = &found.claim;
    let line = receipt_echo(&claim.target_session, &claim.text, outcome, id);
    echo(&journal, id, line, |summary| {
        summary.starts_with(RECEIPT_ECHO)
    })?;
    for answer in &found.answers {
        let (nonce, excerpt) = (answer.nonce.as_str(), console::excerpt(&answer.body));
        let summary = format!("{REPLY_ECHO}{nonce} from {}: {excerpt}", answer.seat);
        let line = crate::state::event_line(Timestamp::now(), ECHO_ACTOR, "chat", id, &summary);
        echo(&journal, id, line, |summary| {
            echoed_nonce(summary) == Some(nonce)
        })?;
    }
    Ok(())
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
    let (caller_dir, bytes) = physical_session(dir, viewer, actual_server)?;
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

/// Session proof shared with agent-address admission; monitor panes need no
/// agent-name stamp. The relay itself additionally requires a legal agent.
pub(crate) fn physical_session(
    dir: &Path,
    viewer: &ObservedViewer,
    actual_server: &ServerId,
) -> Result<(PathBuf, Vec<u8>), &'static str> {
    let session = viewer
        .session
        .as_deref()
        .filter(|s| is_session_name(s))
        .ok_or("caller has no session")?;
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
    Ok((caller_dir, bytes))
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

/// A refusal before the claim: one ref-less `relay` row in the caller's
/// journal when that journal is whole, the refusal on stderr, nothing on
/// stdout.
fn refuse(
    caller: &Caller,
    parsed: &Parsed,
    reason: &str,
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<u8> {
    let summary = format!("refused: {reason}; {}", parsed.text);
    let line = tracked::event_line(&EventFields::new(
        now,
        &caller.actor,
        ACTION,
        &parsed.target,
        "",
        &caller.slot,
        &caller.session,
        "",
        "",
        &summary,
        "",
    ));
    let audit =
        store::open(&caller.dir).append_event_decided(|read| console::snapshot(read).map(|_| line));
    match flatten(audit) {
        Ok(()) => writeln!(err, "ae: relay REFUSED — {reason}. Nothing was sent.")?,
        Err(why) => writeln!(
            err,
            "ae: relay REFUSED — {reason}; attempt audit failed: {why}. Nothing was sent."
        )?,
    }
    Ok(EXIT_FAILED)
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
    fn r6_read_back_uses_the_claimed_session_for_retirement() {
        let dir = std::env::temp_dir().join(format!("ae-relay-retire-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("owned journal directory");
        let uuid = "0199c0de-aaaa-4890-abcd-ef0123456789";
        let other = "0199c0de-bbbb-4890-abcd-ef0123456789";
        let mut got = Vec::new();
        for (name, target_id) in [("work", uuid), ("renamed", uuid), ("work", other)] {
            let console =
                crate::console::Console::bound(name.to_owned(), dir.clone(), uuid.to_owned());
            let ask = format!(
                r#"{{"ts":"2026-10-27T10:00:00Z","actor":"console:local","action":"ask","target":"peer","ref":"q","target_slot":"worker.0","target_session":"work","target_session_id":"{target_id}"}}"#
            );
            let retire = r#"{"ts":"2026-10-27T10:00:01Z","actor":"ae","action":"retire","target":"peer","target_slot":"worker.0"}"#;
            std::fs::write(dir.join("events.jsonl"), format!("{ask}\n{retire}\n"))
                .expect("journal");
            got.push(super::read_back(&console, "q"));
        }
        std::fs::remove_dir_all(&dir).expect("remove owned journal directory");
        assert_eq!(got, [Ok("retired"), Ok("retired"), Ok("pending")]);
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
