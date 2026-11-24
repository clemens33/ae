//! `desk check`: the board against what ae itself owes the owner.
//!
//! A read-only report. It lists the obligations ae can prove open and says
//! which of them no board item names, so the orchestrator can see what the
//! board has dropped. It writes nothing, takes no lock and appends no journal
//! record: every read is the fleet read the live part takes plus one strict
//! read of each running session's journal, the read `relay --status` takes
//! before its echo writes.
//!
//! Three kinds of obligation exist:
//!
//! * `owner-ask`: an ask the owner opened in a session's chat that nobody has
//!   replied to. The line lists the relays that cite it with the state each
//!   one reads, so a reply owed and a relay still pending stay apart.
//! * `relay`: a claimed `relay-attempt`, followed to its target by session
//!   uuid and read as that target's request state. A pending, accepted or
//!   never-opened request is open; a replied, cancelled or retired one is
//!   closed. A target that is stopped, purged, ambiguous or unreadable is
//!   never counted as answered: it is unresolved coverage.
//! * `seat`: a seat whose newest declaration is `waiting-user`, bound to its
//!   session uuid and its seat launch: the session meta's `launch_id.<slot>`
//!   names the launch and its `launch_time.<slot>` baseline proves the
//!   declaration was made inside it. The `state` writer records no launch of
//!   its own, so the baseline is the one boundary the record carries.
//!
//! A board item matches an obligation only by its typed id at a token
//! boundary in the item's text or link: the request id for an ask or a relay,
//! `session:agent` for a seat. The board holds no typed references yet, so a
//! match is a `hint` and never an answer; a hinted obligation leaves
//! `unboarded` and is listed on its own. Only `unboarded` decides the exit
//! status: 0 when it is empty, 1 when it lists something, 2 when the check
//! itself failed.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::desk::{self as record, Desk, Item};
use crate::deskstore;
use crate::digest::Status;
use crate::events::{Event, SessionKey};
use crate::relay::{self, Claim};
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

/// One running session's journal, read strictly: the bytes the request fold
/// reads and the records they parse to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Journal {
    pub(crate) bytes: Vec<u8>,
    pub(crate) events: Vec<Event>,
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
    /// Why the record is partly unreadable, when it is.
    pub(crate) record_gap: Option<String>,
    pub(crate) journal: Result<Journal, String>,
    /// The `waiting-user` seats, or why seat states are not exact.
    pub(crate) waits: Result<Vec<Wait>, String>,
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
    let journal = match &target.journal {
        Ok(journal) => journal,
        Err(why) => return Fate::Unresolved(format!("the target journal: {why}")),
    };
    match relay::opening(&journal.events, id, origin, claim) {
        Err(why) => Fate::Unresolved(why),
        Ok(None) => Fate::Open("absent", absent_label(claim)),
        Ok(Some(_)) => {
            match relay::request_state(&target.name, &claim.target_uuid, &journal.bytes, id) {
                word @ ("pending" | "accepted") => Fate::Open(word, String::new()),
                word @ ("replied" | "cancelled" | "retired") => Fate::Closed(word),
                "absent" => Fate::Open("absent", absent_label(claim)),
                other => Fate::Unresolved(format!("the request reads as {other}")),
            }
        }
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

/// One claimed relay of a running session.
struct Relay {
    id: String,
    owner_input: Option<String>,
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
    let mut ids: Vec<&str> = Vec::new();
    for event in &journal.events {
        if event.action == relay::ATTEMPT
            && let Some(id) = event.reference.as_deref().filter(|id| !id.is_empty())
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    let mut found = Vec::new();
    for id in ids {
        let attempt = journal
            .events
            .iter()
            .find(|event| event.action == relay::ATTEMPT && event.reference.as_deref() == Some(id));
        let (at, owner_input) = attempt.map_or((None, None), |event| {
            (Some(event.ts), event.owner_input.clone())
        });
        let Some(at) = at else { continue };
        match relay::claimed(&journal.events, id, origin) {
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
                    id: id.to_owned(),
                    owner_input,
                    at,
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

/// An obligation proven open, before the board is consulted.
fn open_rows(
    sessions: &[SessionInput],
    unresolved: &mut Vec<Gap>,
    closed: &mut Vec<Row>,
) -> Vec<Row> {
    let mut open = Vec::new();
    for session in sessions {
        if session.status != Status::Running {
            continue;
        }
        let Ok(journal) = &session.journal else {
            continue;
        };
        let Some(uuid) = session.uuid.as_deref() else {
            unresolved.push(Gap {
                subject: format!("session {}", session.name),
                reason: "no session id recorded, so its journal cannot be bound".to_owned(),
            });
            continue;
        };
        let claimed = relays(session, uuid, journal, sessions, unresolved);
        for ask in owner_asks(journal, &session.name, uuid) {
            let Some(id) = ask.reference.as_deref() else {
                continue;
            };
            let cited: Vec<String> = claimed
                .iter()
                .filter(|relay| relay.owner_input.as_deref() == Some(id))
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
        for relay in claimed {
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
            if since.epoch() < epoch {
                gap(format!(
                    "waiting-user declared {since} before the seat's current launch {}",
                    Timestamp::from_epoch(epoch)
                ));
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
    fleet_gap: Option<&str>,
    as_of: Timestamp,
) -> Report {
    let mut unresolved: Vec<Gap> = Vec::new();
    let mut closed: Vec<Row> = Vec::new();
    if let Some(why) = fleet_gap {
        unresolved.push(Gap {
            subject: "fleet".to_owned(),
            reason: why.to_owned(),
        });
    }
    let (mut read, mut stopped, mut unreadable) = (0, 0, 0);
    for session in sessions {
        let reason = match session.status {
            Status::Stopped => {
                stopped += 1;
                continue;
            }
            Status::Unknown => Some("liveness unknown".to_owned()),
            Status::Running => match (&session.record_gap, &session.journal) {
                (Some(why), _) => Some(why.clone()),
                (None, Err(why)) => Some(format!("journal unreadable: {why}")),
                (None, Ok(_)) => None,
            },
        };
        match reason {
            Some(reason) => {
                unreadable += 1;
                unresolved.push(Gap {
                    subject: format!("session {}", session.name),
                    reason,
                });
            }
            None => read += 1,
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

/// Pair each fleet entry with its durable record and read what the check
/// needs: the strict journal of a running session and its waiting seats.
fn inputs(read: &crate::fleet_read::WorldRead) -> (Vec<SessionInput>, Option<String>) {
    let classified = &read.snapshot.sessions;
    let entries = &read.world.sessions;
    if classified.len() != entries.len() {
        return (
            Vec::new(),
            Some("the fleet read did not pair its records".to_owned()),
        );
    }
    let as_of = read.world.now;
    let sessions = entries
        .iter()
        .zip(classified)
        .map(|(entry, classified)| {
            let durable = classified.candidate.durable.as_ref();
            let dir: Option<PathBuf> = durable.map(|record| record.path.clone());
            let meta = durable.and_then(|record| record.snapshot.meta.as_ref());
            let journal = match (&dir, entry.status) {
                (_, Status::Stopped) => Err("session stopped".to_owned()),
                (None, _) => Err("no durable record".to_owned()),
                (Some(dir), _) => {
                    relay::strict_journal(dir).map(|(bytes, events)| Journal { bytes, events })
                }
            };
            let waits = if entry.degraded {
                Err("record partly unreadable".to_owned())
            } else if !entry.agent_state_is_exact() {
                Err("seat states are not exact".to_owned())
            } else {
                Ok(crate::brief::agent_lines_from_world(entry, as_of)
                    .into_iter()
                    .filter(|line| line.state == "waiting-user")
                    .map(|line| {
                        let slot = meta
                            .and_then(|meta| {
                                meta.roster().iter().find(|seat| seat.name == line.name)
                            })
                            .map(|seat| seat.slot.clone());
                        let launch_id = slot
                            .as_deref()
                            .and_then(|slot| meta.and_then(|meta| meta.launch_id(slot)))
                            .map(str::to_owned);
                        let launch_epoch = slot
                            .as_deref()
                            .and_then(|slot| meta.and_then(|meta| meta.launch_epoch(slot)));
                        Wait {
                            since: line.age_secs.map(|age| {
                                Timestamp::from_epoch(as_of.epoch().saturating_sub(age))
                            }),
                            agent: line.name,
                            slot,
                            launch_id,
                            launch_epoch,
                        }
                    })
                    .collect())
            };
            SessionInput {
                name: entry.name.clone(),
                uuid: entry.session_uuid.clone(),
                status: entry.status,
                record_gap: entry
                    .degraded
                    .then(|| "record partly unreadable".to_owned()),
                journal,
                waits,
            }
        })
        .collect();
    (sessions, None)
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
            let line = format!(
                "desk check: the board record {} cannot be read: {why}",
                path.display()
            );
            writeln!(err, "{}", line.replace('\n', " "))?;
            return Ok(EXIT_CHECK_FAILED);
        }
    };
    let Some(root) = dir.parent().and_then(Path::parent) else {
        writeln!(
            err,
            "desk check: the state root of {} cannot be resolved",
            dir.display()
        )?;
        return Ok(EXIT_CHECK_FAILED);
    };
    let read = crate::fleet_read::world_read(root, &mut crate::session::RecordSnapshot::read);
    let (sessions, fleet_gap) = inputs(&read);
    let report = evaluate(
        board.as_ref(),
        &sessions,
        fleet_gap.as_deref(),
        read.world.now,
    );
    out.write_all(render(&report, all).as_bytes())?;
    Ok(report.code())
}
