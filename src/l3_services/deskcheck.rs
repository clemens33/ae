//! `desk check`: the board against what ae itself owes the owner.
//!
//! A read-only report. It lists the obligations ae can prove open and says
//! which of them no board item names, so the orchestrator can see what the
//! board has dropped. It writes nothing, takes no lock and appends no journal
//! record: every read is the fleet read the live part takes plus one strict
//! read of each session's journal, the read `relay --status` takes before its
//! echo writes. A journal of a stopped session is read too, because a
//! dispatcher that stopped leaves its relay owed.
//!
//! Three kinds of obligation exist:
//!
//! * `owner-ask`: an ask the owner opened in a running session's chat that
//!   nobody has replied to. The line lists the relays that cite it, through
//!   a bound relay's own record, with the state each one reads, so a reply
//!   owed and a relay still pending stay apart. An ask open in a session that
//!   is not running is one coverage line, not an obligation.
//! * `relay`: a claimed `relay-attempt` of any readable session, followed to
//!   its target by session uuid and read as that target's request state. A
//!   pending, accepted or never-opened request is open; a replied, cancelled
//!   or retired one is closed. A target that is stopped, purged, ambiguous,
//!   unreadable or whose record lost the evidence of its uuid or journal is
//!   never counted as answered: it is unresolved coverage.
//! * `seat`: a seat whose newest declaration is `waiting-user`, bound to its
//!   session uuid and its seat launch: the session meta's `launch_id.<slot>`
//!   names the launch and its `launch_time.<slot>` baseline proves the
//!   declaration was made after it. Both stamps are whole seconds, so a
//!   declaration in the second of the baseline is unresolved, never bound.
//!
//! A board item matches an obligation only by its typed id at a token
//! boundary in the item's text or link: the request id for an ask or a relay,
//! `session:agent` for a seat. The board holds no typed references yet, so a
//! match is a `hint` and never an answer; a hinted obligation leaves
//! `unboarded` and is listed on its own. Only `unboarded` decides the exit
//! status: 0 when it is empty, 1 when it lists something, 2 when the check
//! itself failed. An unboarded line is a candidate: the board may name the
//! obligation in other words, so the orchestrator adjudicates each one.

use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::path::Path;

use crate::desk::{self as record, Desk, Item};
use crate::deskstore;
use crate::digest::Status;
use crate::events::{Event, SessionKey};
use crate::inventory::{FailedSource, Roots};
use crate::meta::Anomaly;
use crate::relay::{self, Claim};
use crate::requests::{self, Request};
use crate::session::{self, RecordSnapshot};
use crate::time::Timestamp;
use crate::tracked;

#[cfg(test)]
#[path = "desk/check_tests.rs"]
mod check_tests;

/// The exit status of a report that lists an unboarded obligation.
pub(crate) const EXIT_UNBOARDED: u8 = 1;
/// The exit status of a check that could not run.
pub(crate) const EXIT_CHECK_FAILED: u8 = 2;
/// The known-closed entries a default report prints.
pub(crate) const CLOSED_SHOWN: usize = 20;

/// One session's journal, read strictly: the records, the request fold of
/// those same bytes, and an index of the records a relay reads. The fold and
/// the index are taken once, so that a claim never walks the journal again and
/// every relay to the session reads its state from the one fold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Journal {
    pub(crate) events: Vec<Event>,
    pub(crate) states: Vec<Request>,
    /// Where the `relay-attempt` and `relay-result` records of each reference sit.
    relay_at: HashMap<String, Vec<usize>>,
    /// Where the console ask openings of each reference sit.
    ask_at: HashMap<String, Vec<usize>>,
    /// The reference of each claimed relay, in the order of its first attempt.
    attempts: Vec<String>,
}

impl Journal {
    pub(crate) fn new(events: Vec<Event>, states: Vec<Request>) -> Self {
        let mut relay_at: HashMap<String, Vec<usize>> = HashMap::new();
        let mut ask_at: HashMap<String, Vec<usize>> = HashMap::new();
        let mut attempts: Vec<String> = Vec::new();
        let mut claimed: HashSet<String> = HashSet::new();
        for (at, event) in events.iter().enumerate() {
            let Some(id) = event.reference.as_deref().filter(|id| !id.is_empty()) else {
                continue;
            };
            if event.action == relay::ATTEMPT || event.action == relay::RESULT {
                relay_at.entry(id.to_owned()).or_default().push(at);
            }
            if event.action == relay::ATTEMPT && claimed.insert(id.to_owned()) {
                attempts.push(id.to_owned());
            }
            if event.actor == tracked::CONSOLE_SINK && event.action == tracked::Kind::Ask.action() {
                ask_at.entry(id.to_owned()).or_default().push(at);
            }
        }
        Self {
            events,
            states,
            relay_at,
            ask_at,
            attempts,
        }
    }

    fn picked(&self, positions: Option<&Vec<usize>>) -> Vec<Event> {
        positions
            .into_iter()
            .flatten()
            .filter_map(|at| self.events.get(*at))
            .cloned()
            .collect()
    }

    /// The attempts and results of relay `id`, in journal order.
    fn relay_events(&self, id: &str) -> Vec<Event> {
        self.picked(self.relay_at.get(id))
    }

    /// The console ask openings of `id`, in journal order.
    fn ask_events(&self, id: &str) -> Vec<Event> {
        self.picked(self.ask_at.get(id))
    }
}

/// A seat declared `waiting-user`, with the facts that bind it to a launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Wait {
    pub(crate) agent: String,
    /// When the declaration was made.
    pub(crate) since: Option<Timestamp>,
    /// The roster slot the agent holds in the session meta.
    pub(crate) slot: Option<String>,
    /// The meta's single `launch_id.<slot>` row.
    pub(crate) launch_id: Option<String>,
    /// The meta's single `launch_time.<slot>` baseline, epoch seconds.
    pub(crate) launch_epoch: Option<i64>,
}

/// What the fleet read and the strict journal read hold of one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionInput {
    pub(crate) name: String,
    /// The session's own uuid, lower-case, when its meta names one.
    pub(crate) uuid: Option<String>,
    pub(crate) status: Status,
    /// Why the evidence that identifies the session or its journal is
    /// damaged, when it is: a lost uuid or journal proof, never a degradation
    /// of another field.
    pub(crate) record_gap: Option<String>,
    /// The journal, read strictly whatever the liveness, or why it cannot be.
    pub(crate) journal: Result<Journal, String>,
    /// The `waiting-user` seats, or why seat states are not exact.
    pub(crate) waits: Result<Vec<Wait>, String>,
}

impl SessionInput {
    /// Why this session's identity or journal cannot prove an obligation.
    fn proof_gap(&self) -> Option<String> {
        match (&self.record_gap, &self.journal) {
            (Some(why), _) => Some(why.clone()),
            (None, Err(why)) => Some(format!("journal unreadable: {why}")),
            (None, Ok(_)) => None,
        }
    }
}

/// Which obligation a row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    OwnerAsk,
    Relay,
    Seat,
}

impl Kind {
    const fn word(self) -> &'static str {
        match self {
            Self::OwnerAsk => "owner-ask",
            Self::Relay => "relay",
            Self::Seat => "seat",
        }
    }
}

/// One obligation line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    kind: Kind,
    /// The typed id a board item can name.
    key: String,
    /// Where it stands: a session, or `origin -> target:slot` for a relay.
    place: String,
    /// When it began, for its age and its order.
    at: Option<Timestamp>,
    detail: String,
}

/// The item that names an obligation, and in which field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hint {
    item: String,
    via: &'static str,
}

/// Something ae could not establish, never counted as answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Gap {
    subject: String,
    reason: String,
}

/// The result of one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Report {
    as_of: Timestamp,
    /// The board's item count and stamp, `None` when no board is set.
    board: Option<(usize, Timestamp)>,
    read: usize,
    stopped: usize,
    unreadable: usize,
    unboarded: Vec<Row>,
    hinted: Vec<(Row, Hint)>,
    closed: Vec<Row>,
    unresolved: Vec<Gap>,
}

impl Report {
    /// The exit status the report decides.
    pub(crate) const fn code(&self) -> u8 {
        if self.unboarded.is_empty() {
            0
        } else {
            EXIT_UNBOARDED
        }
    }
}

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':')
}

/// Whether `text` names `id` as a whole token: the characters around a match
/// are not part of an id, so `<id>-backup`, `prefix-<id>` and a longer id
/// that merely begins with `id` do not match, while `(<id>)`, `<id>.` and a
/// URL path ending in `<id>` do.
fn mentions(text: &str, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    text.match_indices(id).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let mut after = text[at + id.len()..].chars();
        let next = after.next();
        let continues = match next {
            Some('.') => after.next().is_some_and(is_id_char),
            Some(c) => is_id_char(c),
            None => false,
        };
        before.is_none_or(|c| !is_id_char(c) && c != '.') && !continues
    })
}

/// The first item that names `key`, and where.
fn hint(items: &[Item], key: &str) -> Option<Hint> {
    items.iter().find_map(|item| {
        let in_text = mentions(&item.text, key);
        let in_link = item.links.iter().any(|link| mentions(link, key));
        let via = match (in_text, in_link) {
            (true, true) => "text and link",
            (true, false) => "text",
            (false, true) => "link",
            (false, false) => return None,
        };
        Some(Hint {
            item: item.id.to_string(),
            via,
        })
    })
}

/// What a claimed relay reads as.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Fate {
    /// Not answered: the request word and a label for the line.
    Open(&'static str, String),
    /// Answered, cancelled or retired.
    Closed(&'static str),
    /// Not established, and why.
    Unresolved(String),
}

impl Fate {
    fn word(&self) -> &str {
        match self {
            Self::Open(word, _) | Self::Closed(word) => word,
            Self::Unresolved(_) => "unresolved",
        }
    }
}

/// The state of relay `id`, claimed by `origin`, read from its target by
/// session uuid. A stopped, purged, ambiguous or unreadable target is
/// unresolved whatever a retained row says.
fn fate(claim: &Claim, id: &str, origin: &str, sessions: &[SessionInput]) -> Fate {
    let holders: Vec<&SessionInput> = sessions
        .iter()
        .filter(|session| session.uuid.as_deref() == Some(claim.target_uuid.as_str()))
        .collect();
    let target = match holders.as_slice() {
        [] => {
            return Fate::Unresolved(format!(
                "no session in the fleet holds the target id {}",
                claim.target_uuid
            ));
        }
        [one] => *one,
        many => {
            return Fate::Unresolved(format!(
                "{} sessions hold the target id {}",
                many.len(),
                claim.target_uuid
            ));
        }
    };
    match target.status {
        Status::Stopped => return Fate::Unresolved("target session stopped".to_owned()),
        Status::Unknown => return Fate::Unresolved("target liveness unknown".to_owned()),
        Status::Running => {}
    }
    if let Some(why) = &target.record_gap {
        return Fate::Unresolved(format!("the target's record: {why}"));
    }
    let journal = match &target.journal {
        Ok(journal) => journal,
        Err(why) => return Fate::Unresolved(format!("the target journal: {why}")),
    };
    match relay::opening(&journal.ask_events(id), id, origin, claim) {
        Err(why) => Fate::Unresolved(why),
        Ok(None) => Fate::Open("absent", absent_label(claim)),
        Ok(Some(_)) => match relay::request_word(&journal.states, id) {
            word @ ("pending" | "accepted") => Fate::Open(word, String::new()),
            word @ ("replied" | "cancelled" | "retired") => Fate::Closed(word),
            other => Fate::Unresolved(format!(
                "the target holds the opening and its request reads as {other}"
            )),
        },
    }
}

/// The words for a relay whose request the target never opened.
fn absent_label(claim: &Claim) -> String {
    match claim.outcome {
        Some("not-delivered") => "undelivered, no opening".to_owned(),
        Some(outcome) => format!("no opening, outcome {outcome}"),
        None => "no opening, outcome unrecorded".to_owned(),
    }
}

/// The owner ask a relay says it answers, as its attempt records it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Join {
    /// The attempt cites no owner ask and claims none.
    Absent,
    /// A bound relay that cites this ask.
    Bound(String),
    /// A join that cannot be read as a bound one, and why.
    Malformed(String),
}

/// The join an attempt records: a bound relay and the ask it cites together,
/// the two facts `relay_verify` requires. One without the other is malformed.
fn join(attempt: &Event) -> Join {
    let mode = attempt.relay_mode.as_deref();
    let cited = attempt
        .owner_input
        .as_deref()
        .filter(|cited| !cited.is_empty());
    match (mode, cited) {
        (Some("bound"), Some(cited)) => Join::Bound(cited.to_owned()),
        (Some("bound"), None) => Join::Malformed("a bound relay that cites no ask".to_owned()),
        (mode, Some(cited)) => Join::Malformed(format!(
            "cites ask {cited} but its mode is {}, not bound",
            mode.unwrap_or("unrecorded")
        )),
        (_, None) => Join::Absent,
    }
}

/// One claimed relay of a session.
struct Relay {
    id: String,
    join: Join,
    at: Timestamp,
    place: String,
    fate: Fate,
}

/// Every relay claimed in `session`'s journal, each followed to its target.
fn relays(
    session: &SessionInput,
    origin: &str,
    journal: &Journal,
    sessions: &[SessionInput],
    gaps: &mut Vec<Gap>,
) -> Vec<Relay> {
    let mut found = Vec::new();
    for id in &journal.attempts {
        let records = journal.relay_events(id);
        let Some(attempt) = records.iter().find(|event| event.action == relay::ATTEMPT) else {
            continue;
        };
        match relay::claimed(&records, id, origin) {
            Err(why) => gaps.push(Gap {
                subject: format!("relay {id} ({})", session.name),
                reason: why,
            }),
            Ok(claim) => {
                let place = format!(
                    "{} -> {}:{}",
                    session.name, claim.target_session, claim.target_slot
                );
                found.push(Relay {
                    id: id.clone(),
                    join: join(attempt),
                    at: attempt.ts,
                    place,
                    fate: fate(&claim, id, origin, sessions),
                });
            }
        }
    }
    found
}

/// The open ask openings the owner made in `session`: console-opened, no
/// relay produced them, addressed to this session's id.
fn owner_asks<'a>(journal: &'a Journal, name: &str, uuid: &str) -> Vec<&'a Event> {
    crate::session::open_requests(&journal.events, SessionKey::new(name, uuid))
        .into_iter()
        .filter(|event| {
            let pane = event
                .target_session_uuid
                .as_deref()
                .filter(|pane| !pane.is_empty());
            event.action == tracked::Kind::Ask.action()
                && event.actor == tracked::CONSOLE_SINK
                && event.relay_origin.is_none()
                && event.relay_mode.is_none()
                && event.owner_input.is_none()
                && event.target_session_id.value() == Some(uuid)
                && pane.is_none_or(|pane| pane == uuid)
        })
        .collect()
}

/// The owner asks of a session that is not running, in one gap line: nobody
/// can answer them now, and they are not obligations a board can chase.
fn idle_asks(session: &SessionInput, asks: &[&Event], unresolved: &mut Vec<Gap>) {
    const NAMED: usize = 3;
    if asks.is_empty() {
        return;
    }
    let mut ids: Vec<&str> = asks
        .iter()
        .filter_map(|ask| ask.reference.as_deref())
        .take(NAMED)
        .collect();
    let more = asks.len().saturating_sub(ids.len());
    let word = match session.status {
        Status::Stopped => "stopped",
        _ => "of unknown liveness",
    };
    let tail = if more == 0 {
        String::new()
    } else {
        ids.push("...");
        format!(" and {more} more")
    };
    unresolved.push(Gap {
        subject: format!("session {}", session.name),
        reason: format!(
            "{} open owner ask(s) in a session {word}, not chased: {}{tail}",
            asks.len(),
            ids.join(", ")
        ),
    });
}

/// An obligation proven open, before the board is consulted. Every session
/// whose journal reads claims its relays, whatever its liveness: a dispatcher
/// that stopped leaves its relay owed. The liveness restriction sits on the
/// TARGET of a relay and on who can still answer an owner ask.
fn open_rows(
    sessions: &[SessionInput],
    unresolved: &mut Vec<Gap>,
    closed: &mut Vec<Row>,
) -> Vec<Row> {
    let mut open = Vec::new();
    for session in sessions {
        if session.record_gap.is_some() {
            continue;
        }
        let Ok(journal) = &session.journal else {
            continue;
        };
        let Some(uuid) = session.uuid.as_deref() else {
            if !journal.attempts.is_empty() || session.status == Status::Running {
                unresolved.push(Gap {
                    subject: format!("session {}", session.name),
                    reason: "no session id recorded, so its journal cannot be bound".to_owned(),
                });
            }
            continue;
        };
        let claimed = relays(session, uuid, journal, sessions, unresolved);
        let asks = owner_asks(journal, &session.name, uuid);
        if session.status == Status::Running {
            for ask in asks {
                let Some(id) = ask.reference.as_deref() else {
                    continue;
                };
                let cited: Vec<String> = claimed
                    .iter()
                    .filter(|relay| matches!(&relay.join, Join::Bound(cited) if cited == id))
                    .map(|relay| format!("{} {}", relay.id, relay.fate.word()))
                    .collect();
                let detail = if cited.is_empty() {
                    "reply owed, no relay".to_owned()
                } else {
                    format!("reply owed, relays {}", cited.join(", "))
                };
                open.push(Row {
                    kind: Kind::OwnerAsk,
                    key: id.to_owned(),
                    place: session.name.clone(),
                    at: Some(ask.ts),
                    detail,
                });
            }
        } else {
            idle_asks(session, &asks, unresolved);
        }
        for relay in claimed {
            let subject = format!("relay {} ({})", relay.id, relay.place);
            match &relay.join {
                Join::Absent => {}
                Join::Malformed(why) => unresolved.push(Gap {
                    subject: subject.clone(),
                    reason: format!("owner join not read: {why}"),
                }),
                Join::Bound(cited) if !journal.ask_at.contains_key(cited) => {
                    unresolved.push(Gap {
                        subject: subject.clone(),
                        reason: format!(
                            "owner join not read: it cites ask {cited}, which this journal does not hold"
                        ),
                    });
                }
                Join::Bound(_) => {}
            }
            let row = |detail: String| Row {
                kind: Kind::Relay,
                key: relay.id.clone(),
                place: relay.place.clone(),
                at: Some(relay.at),
                detail,
            };
            match &relay.fate {
                Fate::Open(word, label) => {
                    let detail = if label.is_empty() {
                        (*word).to_owned()
                    } else {
                        format!("{word}, {label}")
                    };
                    open.push(row(detail));
                }
                Fate::Closed(word) => closed.push(row((*word).to_owned())),
                Fate::Unresolved(reason) => unresolved.push(Gap {
                    subject: format!("relay {} ({})", relay.id, relay.place),
                    reason: reason.clone(),
                }),
            }
        }
    }
    open
}

/// The seats waiting on the owner that are bound to their current launch.
fn wait_rows(sessions: &[SessionInput], unresolved: &mut Vec<Gap>) -> Vec<Row> {
    let mut open = Vec::new();
    for session in sessions {
        if session.status != Status::Running || session.record_gap.is_some() {
            continue;
        }
        let waits = match &session.waits {
            Ok(waits) => waits,
            Err(why) => {
                unresolved.push(Gap {
                    subject: format!("seats of {}", session.name),
                    reason: why.clone(),
                });
                continue;
            }
        };
        for wait in waits {
            let key = format!("{}:{}", session.name, wait.agent);
            let mut gap = |reason: String| {
                unresolved.push(Gap {
                    subject: format!("seat {key}"),
                    reason,
                });
            };
            let Some(uuid) = session.uuid.as_deref() else {
                gap("waiting-user, but the session records no session id".to_owned());
                continue;
            };
            let Some(slot) = wait.slot.as_deref() else {
                gap("waiting-user, but the roster names no slot for the seat".to_owned());
                continue;
            };
            let Some(launch) = wait.launch_id.as_deref() else {
                gap(format!(
                    "waiting-user, but meta has no single launch_id.{slot}"
                ));
                continue;
            };
            let Some(epoch) = wait.launch_epoch else {
                gap(format!(
                    "waiting-user, but meta has no completed launch baseline launch_time.{slot}"
                ));
                continue;
            };
            let Some(since) = wait.since else {
                gap("waiting-user, but the declaration time is unreadable".to_owned());
                continue;
            };
            // Both stamps are whole seconds, so an old declaration and its
            // replacement's launch can share one: only a declaration strictly
            // after the launch baseline proves the current incarnation.
            if since.epoch() <= epoch {
                let launched = Timestamp::from_epoch(epoch);
                gap(if since.epoch() == epoch {
                    format!(
                        "waiting-user declared {since} in the same second as the seat's current launch {launched}; the order of the two is not proved"
                    )
                } else {
                    format!(
                        "waiting-user declared {since} before the seat's current launch {launched}"
                    )
                });
                continue;
            }
            open.push(Row {
                kind: Kind::Seat,
                key,
                place: format!("uuid {uuid}  launch {launch}"),
                at: Some(since),
                detail: format!("waiting-user since {since}"),
            });
        }
    }
    open
}

/// The report of one check: the obligations of `sessions` against `board`.
pub(crate) fn evaluate(
    board: Option<&Desk>,
    sessions: &[SessionInput],
    fleet: Vec<Gap>,
    as_of: Timestamp,
) -> Report {
    let mut unresolved: Vec<Gap> = fleet;
    let mut closed: Vec<Row> = Vec::new();
    let (mut read, mut stopped, mut unreadable) = (0, 0, 0);
    for session in sessions {
        let reason = match (session.status, session.proof_gap()) {
            (Status::Unknown, _) => Some("liveness unknown".to_owned()),
            (Status::Stopped, Some(why)) => Some(format!("stopped, and {why}")),
            (Status::Running, Some(why)) => Some(why),
            (Status::Stopped, None) => {
                stopped += 1;
                None
            }
            (Status::Running, None) => {
                read += 1;
                None
            }
        };
        if let Some(reason) = reason {
            unreadable += 1;
            unresolved.push(Gap {
                subject: format!("session {}", session.name),
                reason,
            });
        }
    }
    let mut open = open_rows(sessions, &mut unresolved, &mut closed);
    open.extend(wait_rows(sessions, &mut unresolved));
    let items: &[Item] = board.map_or(&[], |desk| desk.items.as_slice());
    let (mut unboarded, mut hinted) = (Vec::new(), Vec::new());
    for row in open {
        match hint(items, &row.key) {
            Some(hint) => hinted.push((row, hint)),
            None => unboarded.push(row),
        }
    }
    let order = |left: &Row, right: &Row| {
        let by_time = left
            .at
            .map(Timestamp::epoch)
            .cmp(&right.at.map(Timestamp::epoch));
        by_time
            .then(left.kind.cmp(&right.kind))
            .then_with(|| left.key.cmp(&right.key))
    };
    unboarded.sort_by(order);
    hinted.sort_by(|left, right| order(&left.0, &right.0));
    closed.sort_by(|left, right| order(right, left));
    Report {
        as_of,
        board: board.map(|desk| (desk.items.len(), desk.updated)),
        read,
        stopped,
        unreadable,
        unboarded,
        hinted,
        closed,
        unresolved,
    }
}

/// A shown field: ae-authored words and journal-borne names pass the
/// terminal-output gate and fold onto one line.
fn clean(text: &str) -> String {
    crate::board::terminal_text(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn age(at: Option<Timestamp>, as_of: Timestamp) -> String {
    crate::brief::age(at.map(|at| at.seconds_until(as_of)))
}

fn row_line(row: &Row, as_of: Timestamp) -> String {
    format!(
        "  {} {}  {}  {}  {}",
        row.kind.word(),
        clean(&row.key),
        clean(&row.place),
        age(row.at, as_of),
        clean(&row.detail)
    )
}

/// The report as `desk check` prints it. `all` lifts the known-closed cut.
pub(crate) fn render(report: &Report, all: bool) -> String {
    let as_of = report.as_of;
    let mut out = String::new();
    match report.board {
        None => out.push_str("board: absent\n"),
        Some((items, updated)) => {
            out.push_str(&format!("board: {items} items, updated {updated}\n"));
        }
    }
    out.push_str(&format!(
        "desk check  as of {as_of}  sessions {} read, {} stopped, {} unreadable\n",
        report.read, report.stopped, report.unreadable
    ));
    out.push_str(&format!("unboarded ({})\n", report.unboarded.len()));
    for row in &report.unboarded {
        out.push_str(&row_line(row, as_of));
        out.push('\n');
    }
    out.push_str(&format!("boarded (hint) ({})\n", report.hinted.len()));
    for (row, hint) in &report.hinted {
        out.push_str(&format!(
            "{}  hint: {} via {}\n",
            row_line(row, as_of),
            clean(&hint.item),
            hint.via
        ));
    }
    out.push_str(&format!("known closed ({})\n", report.closed.len()));
    let shown = if all {
        report.closed.len()
    } else {
        report.closed.len().min(CLOSED_SHOWN)
    };
    for row in &report.closed[..shown] {
        out.push_str(&row_line(row, as_of));
        out.push('\n');
    }
    if shown < report.closed.len() {
        out.push_str(&format!(
            "  {} older omitted (--all lists them)\n",
            report.closed.len() - shown
        ));
    }
    out.push_str(&format!(
        "unresolved coverage ({})\n",
        report.unresolved.len()
    ));
    for gap in &report.unresolved {
        out.push_str(&format!(
            "  {}  {}\n",
            clean(&gap.subject),
            clean(&gap.reason)
        ));
    }
    out
}

/// The sources the fleet read lost, one gap each, and the pairing check: a
/// fleet that lost a source lists none of the sessions it held, so the
/// absence of an obligation there proves nothing.
fn fleet_gaps(incomplete: &[FailedSource], complete: bool, paired: bool) -> Vec<Gap> {
    let mut gaps: Vec<Gap> = incomplete
        .iter()
        .map(|source| Gap {
            subject: "fleet source".to_owned(),
            reason: match source {
                FailedSource::CanonicalRoot(path) => {
                    format!("the session root {} would not enumerate", path.display())
                }
                FailedSource::WorktreeRoot(path) => {
                    format!("the worktree root {} would not enumerate", path.display())
                }
                FailedSource::WorktreeState(path) => {
                    format!("the worktree state {} would not enumerate", path.display())
                }
                FailedSource::Server(_) => {
                    "a tmux server did not answer, so the sessions it holds are not listed"
                        .to_owned()
                }
            },
        })
        .collect();
    if !complete && gaps.is_empty() {
        gaps.push(Gap {
            subject: "fleet".to_owned(),
            reason: "the inventory is incomplete and names no source".to_owned(),
        });
    }
    if !paired {
        gaps.push(Gap {
            subject: "fleet".to_owned(),
            reason: "the fleet read did not pair its records".to_owned(),
        });
    }
    gaps
}

/// Why the record of a session reads as degraded, and whether that loses the
/// evidence a relay is proved from: a lost meta, a malformed meta line, a
/// repeated `session_id` or lost journal records do. A repeated roster row or
/// an unusable idle-nudge pin degrade the seat states and nothing else.
fn degradation(snapshot: Option<&RecordSnapshot>) -> (String, bool) {
    let Some(snapshot) = snapshot else {
        return ("no durable record".to_owned(), true);
    };
    let mut reasons: Vec<String> = Vec::new();
    let mut proof = false;
    match snapshot.meta.as_ref() {
        None => {
            reasons.push("the meta is unreadable".to_owned());
            proof = true;
        }
        Some(meta) => {
            for anomaly in meta.anomalies() {
                if !session::anomalies_degrade(std::slice::from_ref(anomaly)) {
                    continue;
                }
                proof |= match anomaly {
                    Anomaly::MalformedLine { .. } => true,
                    Anomaly::DuplicateKey { key, .. } => key == "session_id",
                    _ => false,
                };
                reasons.push(anomaly.to_string());
            }
        }
    }
    if snapshot
        .events
        .as_ref()
        .is_none_or(session::SessionRead::lost_records)
    {
        reasons.push("journal records were lost".to_owned());
        proof = true;
    }
    if reasons.is_empty() {
        reasons.push("an unusable idle-nudge pin".to_owned());
    }
    (reasons.join("; "), proof)
}

/// The `waiting-user` seats of one fleet entry, each with the facts that bind
/// it to its launch.
fn waits_of(
    entry: &crate::digest::SessionEntry,
    snapshot: Option<&RecordSnapshot>,
    as_of: Timestamp,
) -> Vec<Wait> {
    let meta = snapshot.and_then(|snapshot| snapshot.meta.as_ref());
    crate::brief::agent_lines_from_world(entry, as_of)
        .into_iter()
        .filter(|line| line.state == "waiting-user")
        .map(|line| {
            let slot = meta
                .and_then(|meta| meta.roster().iter().find(|seat| seat.name == line.name))
                .map(|seat| seat.slot.clone());
            let launch_id = slot
                .as_deref()
                .and_then(|slot| meta.and_then(|meta| meta.launch_id(slot)))
                .map(str::to_owned);
            let launch_epoch = slot
                .as_deref()
                .and_then(|slot| meta.and_then(|meta| meta.launch_epoch(slot)));
            Wait {
                since: line
                    .age_secs
                    .map(|age| Timestamp::from_epoch(as_of.epoch().saturating_sub(age))),
                agent: line.name,
                slot,
                launch_id,
                launch_epoch,
            }
        })
        .collect()
}

/// Pair each fleet entry with its durable record and read what the check
/// needs: the strict journal of every session whose record exists, whatever
/// its liveness, and its waiting seats.
fn inputs(read: &crate::fleet_read::WorldRead) -> (Vec<SessionInput>, Vec<Gap>) {
    let classified = &read.snapshot.sessions;
    let entries = &read.world.sessions;
    let paired = classified.len() == entries.len();
    let gaps = fleet_gaps(
        &read.snapshot.incomplete,
        read.world.inventory_complete(),
        paired,
    );
    if !paired {
        return (Vec::new(), gaps);
    }
    let as_of = read.world.now;
    let sessions = entries
        .iter()
        .zip(classified)
        .map(|(entry, classified)| {
            let durable = classified.candidate.durable.as_ref();
            let snapshot = durable.map(|record| &record.snapshot);
            let journal = match durable {
                None => Err("no durable record".to_owned()),
                Some(record) => relay::strict_journal(&record.path).map(|(bytes, events)| {
                    let key = entry.session_uuid.as_deref().map_or_else(
                        || SessionKey::named(&entry.name),
                        |uuid| SessionKey::new(&entry.name, uuid),
                    );
                    Journal::new(events, requests::states_in(&bytes, key))
                }),
            };
            let (degraded, record_gap) = if entry.degraded {
                let (reason, proof) = degradation(snapshot);
                let words = format!("record degraded: {reason}");
                (Some(words.clone()), proof.then_some(words))
            } else {
                (None, None)
            };
            let waits = match degraded {
                Some(words) => Err(words),
                None if !entry.agent_state_is_exact() => {
                    Err("seat states are not exact".to_owned())
                }
                None => Ok(waits_of(entry, snapshot, as_of)),
            };
            SessionInput {
                name: entry.name.clone(),
                uuid: entry.session_uuid.clone(),
                status: entry.status,
                record_gap,
                journal,
                waits,
            }
        })
        .collect();
    (sessions, gaps)
}

/// The one stderr line of a board record that cannot be read: control text in
/// the path or the reason cannot alter it.
fn board_error_line(path: &Path, why: &dyn std::fmt::Display) -> String {
    clean(&format!(
        "desk check: the board record {} cannot be read: {why}",
        path.display()
    ))
}

/// `desk check [--all]`: read the board and the fleet, print the report.
///
/// # Errors
///
/// A failed write to `out` or `err`.
pub(crate) fn run(
    dir: &Path,
    all: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let board = match deskstore::read(dir) {
        Ok(board) => board,
        Err(why) => {
            let path = dir.join(record::RECORD_FILE);
            writeln!(err, "{}", board_error_line(&path, &why))?;
            return Ok(EXIT_CHECK_FAILED);
        }
    };
    // The board is this session's own record, wherever it sits; the fleet is
    // the whole of `AE_HOME`, found from either layout the record can be in.
    let Some(root) = Roots::home_of(dir) else {
        writeln!(
            err,
            "{}",
            clean(&format!(
                "desk check: the state root of {} cannot be resolved",
                dir.display()
            ))
        )?;
        return Ok(EXIT_CHECK_FAILED);
    };
    let read = crate::fleet_read::world_read(root, &mut session::RecordSnapshot::read);
    let (sessions, fleet) = inputs(&read);
    let report = evaluate(board.as_ref(), &sessions, fleet, read.world.now);
    out.write_all(render(&report, all).as_bytes())?;
    Ok(report.code())
}
