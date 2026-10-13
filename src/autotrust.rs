//! `auto_trust`: the ONE prompt ae may answer — a tool's MEASURED folder-trust
//! block, for the seat's own recorded folder, at most once per tool start.
//!
//! [`crate::watchdog::human_prompt_class`] sees the block and
//! [`crate::deliver::answer_trust`] presses the keys. This module owns what
//! both rest on: the proof that the pane is the seat's and the folder its own,
//! the episode, the strict journal read and the records. An episode is one
//! tool start, `<slot>:<launch_id>:<tool pid>`: `_run` execs the tool, so its
//! pid is the start, and two live starts never share one.

use std::path::{Path, PathBuf};

use crate::events::Event;
use crate::inventory::ServerId;
use crate::procs::{Lineage, Proc};
use crate::store::SourceRead;
use crate::time::Timestamp;
use crate::tmux::{ObservedViewer, OptionReading};
use crate::watchdog::{TrustSighting, WATCHDOG_ACTOR};

/// Reserves one tool start before its first key. The reservation ALONE spends
/// the start: whatever follows, no second attempt is made for it.
pub const RESERVED: &str = "auto-trust-reserved";
/// One key sent, or refused by tmux.
pub const KEY: &str = "auto-trust-key";
/// The trust modal is gone after the Enter.
pub const TRUSTED: &str = "auto-trust";
/// A reserved start that stopped before the modal was gone.
pub const FAILED: &str = "auto-trust-failed";

/// The seat an answer is for, as its caller placed it. Every field is proven
/// again against the meta, tmux and the process table before each key.
pub(crate) struct Seat<'a> {
    /// The session's meta directory: journal, meta and send-lock root.
    pub(crate) dir: &'a Path,
    /// The recorded server the keys go to.
    pub(crate) server: &'a ServerId,
    pub(crate) session: &'a str,
    pub(crate) slot: &'a str,
    pub(crate) pane: &'a str,
    /// The recorded binary, `agent_bin.<slot>`.
    pub(crate) tool: &'a str,
}

/// The facts every key re-proves; two proofs must be EQUAL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Proof {
    name: String,
    episode: String,
    folder: PathBuf,
}

/// Whether a drawn trust block is this seat's to answer now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Due {
    /// Proven, and the journal holds no reservation for this start.
    Yes,
    /// Not ae's to answer: another folder, an unproven pane, a spent start.
    No,
    /// The journal cannot be read strictly, so nothing is answered; the why.
    Doubt(String),
}

/// Whether `sighting` is this seat's to answer. It reads the pane's identity,
/// the process table and the journal; it presses nothing and waits for nothing.
pub(crate) fn due(seat: &Seat<'_>, sighting: &TrustSighting) -> Due {
    let Ok(proof) = prove(seat, &sighting.folder) else {
        return Due::No;
    };
    owed(crate::store::open(seat.dir).events_source(), &proof.episode)
}

/// The journal's verdict on a proven start: doubt, spent, or owed an answer.
fn owed(source: SourceRead, episode: &str) -> Due {
    match ledger(source) {
        Err(why) => Due::Doubt(why),
        Ok(events) if spent(&events, episode) => Due::No,
        Ok(_) => Due::Yes,
    }
}

/// Whether the global `auto_trust` switch the session's watchdog reads is on.
pub(crate) fn on(dir: &Path) -> bool {
    crate::config::global_auto_trust(crate::global_config_of(dir).as_deref()).answers()
}

/// What an in-wait answer did, for the wait it ran in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InWait {
    /// Nothing was tried, or not yet: a later chunk may try again.
    Again,
    /// The modal is gone after the Enter.
    Trusted,
    /// Tried and not answered, said on `err`: this wait tries no more.
    Stop,
}

/// Answer a trust block drawn on a seat's pane INSIDE a caller's own wait — a
/// spawn's brief, a launch turn — where no watchdog may be running. Nothing is
/// tried with `auto_trust` off, a seat whose records name no tool, or no
/// measured trust block drawn.
pub(crate) fn in_wait(
    dir: &Path,
    server: &ServerId,
    slot: &str,
    pane: &str,
    err: &mut impl std::io::Write,
) -> InWait {
    let answer = on(dir)
        .then(|| try_in_wait(dir, server, slot, pane, &mut *err))
        .flatten();
    match answer {
        None | Some(crate::deliver::TrustAnswer::NotYet(_)) => InWait::Again,
        Some(crate::deliver::TrustAnswer::Trusted(_)) => InWait::Trusted,
        Some(
            crate::deliver::TrustAnswer::Declined(why) | crate::deliver::TrustAnswer::Failed(why),
        ) => {
            let _ = writeln!(err, "ae: auto_trust did not answer the trust prompt: {why}");
            InWait::Stop
        }
    }
}

fn try_in_wait(
    dir: &Path,
    server: &ServerId,
    slot: &str,
    pane: &str,
    err: &mut impl std::io::Write,
) -> Option<crate::deliver::TrustAnswer> {
    let session = dir.file_name()?.to_str()?;
    let meta = meta(dir).ok()?;
    let tool = meta
        .roster()
        .iter()
        .find(|entry| entry.slot == slot)?
        .binary
        .clone()?;
    let frame = crate::transport::capture_pane(server, pane)?;
    crate::watchdog::human_prompt_class(&frame, &tool)?.trust?;
    let seat = Seat {
        dir,
        server,
        session,
        slot,
        pane,
        tool: &tool,
    };
    Some(crate::deliver::answer_trust(&seat, err))
}

/// The seat, proven from its records and its pane: the recorded server and
/// binary, the pane stamped as this slot of this session incarnation and the
/// only pane of the slot, one live tool process under it, and the seat's
/// folder equal to `drawn`, both canonical.
pub(crate) fn prove(seat: &Seat<'_>, drawn: &str) -> Result<Proof, String> {
    let meta = meta(seat.dir)?;
    match meta.server_selector() {
        crate::meta::ServerSelector::Positive(named)
            if ServerId::Selected(named.clone()) == *seat.server => {}
        _ => return Err("the pane is not on the recorded server".to_owned()),
    }
    let entry = meta
        .roster()
        .iter()
        .find(|entry| entry.slot == seat.slot)
        .ok_or("the slot is not a roster seat")?;
    if entry.binary.as_deref() != Some(seat.tool) {
        return Err("the seat's recorded binary changed".to_owned());
    }
    let launch = meta
        .launch_id(seat.slot)
        .filter(|id| !id.is_empty())
        .ok_or("the seat has no launch id")?;
    let session_id = meta
        .session_id()
        .filter(|id| !id.is_empty())
        .ok_or("the session has no id")?;
    let viewer =
        crate::transport::observe_viewer(seat.server, seat.pane).ok_or("pane unreadable")?;
    if !is_seat(&viewer, seat.slot, seat.session, session_id) {
        return Err("the pane is not this seat's".to_owned());
    }
    let slots =
        crate::transport::observe_slots(seat.server, seat.session).ok_or("slots unreadable")?;
    if !slots
        .iter()
        .filter(|pane| pane.slot == seat.slot)
        .map(|pane| pane.pane.as_str())
        .eq([seat.pane])
    {
        return Err("the slot is not on exactly this one pane".to_owned());
    }
    let pane_pid = crate::transport::observe_pane_probe(seat.server, seat.pane)
        .and_then(|probe| probe.pid)
        .ok_or("pane pid unreadable")?;
    let table = crate::procs::snapshot().ok_or("process table unreadable")?;
    let pid = tool_pid(&table, pane_pid, seat.tool)?;
    let recorded = crate::meta::resolve_seat_dir(&meta, seat.slot)?;
    if !Path::new(&recorded).is_absolute() {
        return Err("the seat records no folder".to_owned());
    }
    let folder = crate::doors::canonical_strict_dir(Path::new(&recorded))
        .map_err(|why| format!("the seat's folder {}", place(&why)))?;
    let shown = crate::doors::canonical_strict_dir(Path::new(drawn))
        .map_err(|why| format!("the drawn folder {}", place(&why)))?;
    if folder != shown {
        return Err(format!("the modal names {drawn}, not the seat's folder"));
    }
    Ok(Proof {
        name: entry.name.clone(),
        episode: format!("{}:{launch}:{pid}", seat.slot),
        folder,
    })
}

/// The pane's own readings name `slot` of `session`, in the incarnation
/// `session_id`: every one must hold, each on its own.
fn is_seat(viewer: &ObservedViewer, slot: &str, session: &str, session_id: &str) -> bool {
    viewer.slot.as_deref() == Some(slot)
        && viewer.session.as_deref() == Some(session)
        && viewer.session_uuid == OptionReading::Set(session_id.to_owned())
}

/// The session's meta, read whole and strictly UTF-8.
fn meta(dir: &Path) -> Result<crate::meta::Meta, String> {
    let bytes = crate::meta::read_bytes(dir).map_err(|why| format!("meta: {why}"))?;
    let text = String::from_utf8(bytes).map_err(|_| "meta: not UTF-8".to_owned())?;
    Ok(crate::meta::Meta::parse(&text))
}

/// Why a folder is not one, in words.
fn place(why: &crate::doors::StrictDirError) -> &'static str {
    match why {
        crate::doors::StrictDirError::Absent { .. } => "is gone",
        crate::doors::StrictDirError::Unreadable { .. } => "is unreadable",
        crate::doors::StrictDirError::NotDirectory { .. } => "is not a directory",
    }
}

/// The ONE live `tool` process under the pane: its outermost row there. None,
/// two, or one ae cannot place is no episode.
fn tool_pid(table: &[Proc], pane_pid: u32, tool: &str) -> Result<u32, String> {
    let kind = crate::tool::ToolKind::from_known_binary_name(tool).ok_or("unknown tool")?;
    let rows: Vec<(&Proc, Lineage)> = crate::procs::harness_rows(table, pane_pid)
        .into_iter()
        .filter(|(found, _, _)| *found == kind)
        .map(|(_, proc, place)| (proc, place))
        .collect();
    if rows.iter().any(|(_, place)| *place != Lineage::Under) {
        return Err(format!("a {tool} process ae cannot place"));
    }
    let outer: Vec<u32> = rows
        .iter()
        .filter(|(proc, _)| !rows.iter().any(|(parent, _)| parent.pid == proc.ppid))
        .map(|(proc, _)| proc.pid)
        .collect();
    match outer.as_slice() {
        [pid] => Ok(*pid),
        [] => Err(format!("no live {tool} under the pane")),
        _ => Err(format!("more than one {tool} under the pane")),
    }
}

/// The journal, read strictly: absent, unreadable, not a regular file, not
/// UTF-8, or ANY non-blank line [`Event::parse_line`] refuses is doubt, and
/// doubt never answers.
pub(crate) fn ledger(source: SourceRead) -> Result<Vec<Event>, String> {
    let bytes = match source {
        SourceRead::Ready(bytes) => bytes,
        SourceRead::Absent => return Err("the journal does not exist yet".to_owned()),
        SourceRead::Invalid(what) => return Err(format!("the journal is {what}")),
        SourceRead::Unreadable(why) => return Err(format!("the journal is unreadable: {why}")),
    };
    let text = String::from_utf8(bytes).map_err(|_| "the journal is not UTF-8".to_owned())?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            Event::parse_line(line).map_err(|_| "the journal holds a damaged line".to_owned())
        })
        .collect()
}

/// Whether `episode` is already reserved.
fn spent(events: &[Event], episode: &str) -> bool {
    events
        .iter()
        .any(|event| event.action == RESERVED && event.reference.as_deref() == Some(episode))
}

/// Append the reservation only to a journal that reads strictly and holds none
/// for this start, under one hold of its lock; then read it back exactly once.
pub(crate) fn reserve(seat: &Seat<'_>, proof: &Proof, yes: bool) -> Result<(), String> {
    let selected = if yes { "yes" } else { "no" };
    let summary = format!(
        "{} {} selected {selected}",
        seat.tool,
        proof.folder.display()
    );
    let line = line(seat, proof, RESERVED, &summary);
    let store = crate::store::open(seat.dir);
    match store.append_event_decided(|source| match ledger(source) {
        Ok(events) if spent(&events, &proof.episode) => {
            Err(format!("{} is already spent", proof.episode))
        }
        Ok(_) => Ok(line),
        Err(why) => Err(why),
    }) {
        Err(why) => return Err(format!("reservation not written: {why}")),
        Ok(Err(why)) => return Err(why),
        Ok(Ok(())) => {}
    }
    let events = ledger(store.events_source())?;
    let reserved = events
        .iter()
        .filter(|event| {
            event.action == RESERVED && event.reference.as_deref() == Some(&proof.episode)
        })
        .count();
    if reserved == 1 {
        Ok(())
    } else {
        Err("the reservation did not read back exactly once".to_owned())
    }
}

/// The modal is gone: record it, then tell the chat in the record's words.
/// The summary, or why its record failed.
pub(crate) fn trusted(
    seat: &Seat<'_>,
    proof: &Proof,
    err: &mut impl std::io::Write,
) -> Result<String, String> {
    let summary = format!(
        "trusted {} for {} ({})",
        proof.folder.display(),
        proof.name,
        seat.tool
    );
    if let Err(why) = record(seat, proof, TRUSTED, &summary) {
        let _ = writeln!(err, "ae: auto_trust: {summary}; record failed: {why}");
        return Err(format!("journal: {why}"));
    }
    let chat = crate::chat_line(Timestamp::now(), WATCHDOG_ACTOR, "", &summary);
    if let Err(why) = crate::store::open(seat.dir).append_event(&chat) {
        let _ = writeln!(err, "ae: auto_trust: {summary}; chat failed: {why}");
    }
    Ok(summary)
}

/// Record a spent start's stop, `<step>: <why>`, and say so where the record
/// could not.
pub(crate) fn fail(
    seat: &Seat<'_>,
    proof: &Proof,
    (step, why): (&str, &str),
    err: &mut impl std::io::Write,
) -> crate::deliver::TrustAnswer {
    let summary = format!("{step}: {why}");
    if let Err(failed) = record(seat, proof, FAILED, &summary) {
        let _ = writeln!(
            err,
            "ae: auto_trust stopped ({summary}); record failed: {failed}"
        );
    }
    crate::deliver::TrustAnswer::Failed(summary)
}

/// Append one record of `action` for this start.
pub(crate) fn record(
    seat: &Seat<'_>,
    proof: &Proof,
    action: &str,
    summary: &str,
) -> Result<(), crate::store::Error> {
    crate::store::open(seat.dir).append_event(&line(seat, proof, action, summary))
}

/// One record by the watchdog actor, addressed to the seat, keyed by episode.
fn line(seat: &Seat<'_>, proof: &Proof, action: &str, summary: &str) -> String {
    crate::tracked::event_line(&crate::tracked::EventFields::new(
        Timestamp::now(),
        WATCHDOG_ACTOR,
        action,
        &proof.name,
        &proof.episode,
        "",
        "",
        seat.slot,
        seat.session,
        summary,
        "",
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        Due, Proof, RESERVED, Seat, is_seat, ledger, owed, place, prove, reserve, spent, tool_pid,
    };
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    use crate::procs::Proc;
    use crate::store::SourceRead;
    use crate::tmux::{ObservedViewer, OptionReading};

    const RESERVATION: &str = r#"{"ts":"2026-10-11T00:00:00Z","actor":"watchdog","action":"auto-trust-reserved","target":"lead","target_slot":"main","ref":"main:start:7"}"#;

    fn proc(id: u32, parent: u32, comm: &str) -> Proc {
        Proc {
            pid: id,
            ppid: parent,
            comm: comm.to_owned(),
        }
    }

    /// Only a regular, UTF-8 journal whose every non-blank line parses is a
    /// ledger; anything else is doubt, and doubt never answers.
    #[test]
    fn the_ledger_is_strict_and_every_doubt_fails_closed() {
        let ready = |text: &str| ledger(SourceRead::Ready(text.as_bytes().to_vec()));
        let read = ready(&format!("\n{RESERVATION}\n  \n")).expect("an intact journal reads");
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].action, RESERVED);
        for doubt in [
            SourceRead::Absent,
            SourceRead::Invalid("a directory".to_owned()),
            SourceRead::Unreadable("permission denied".to_owned()),
            SourceRead::Ready(vec![0xff, b'\n']),
        ] {
            assert!(ledger(doubt).is_err());
        }
        for damaged in [
            "an unrelated damaged event",
            r#"{"ts":"bad","actor":"watchdog","action" : "auto-trust-reserved","ref":"main:start:7"}"#,
        ] {
            let journal = format!("{RESERVATION}\n{damaged}\n");
            assert!(ready(&journal).is_err(), "{damaged}");
        }
    }

    /// The reservation of THIS start spends it; another start's, or another
    /// record carrying the same ref, does not. Only an unspent start of a
    /// strict journal is owed an answer.
    #[test]
    fn only_a_reservation_of_the_same_start_spends_it() {
        let journal = || SourceRead::Ready(RESERVATION.as_bytes().to_vec());
        assert_eq!(owed(journal(), "main:start:7"), Due::No, "spent");
        assert_eq!(owed(journal(), "main:start:8"), Due::Yes, "unspent");
        assert!(matches!(
            owed(SourceRead::Absent, "main:start:8"),
            Due::Doubt(_)
        ));
        let events = ledger(journal()).expect("ledger");
        assert!(spent(&events, "main:start:7"));
        assert!(!spent(&events, "main:start:8"));
        let key = RESERVATION.replace("auto-trust-reserved", "auto-trust-key");
        let other = ledger(SourceRead::Ready(key.into_bytes())).expect("ledger");
        assert!(!spent(&other, "main:start:7"));
    }

    /// The first proof: keys go only to the server the record names. Another
    /// name, no selector or a doubled one is refused by that rule, while the
    /// recorded server passes it and is judged by the next one.
    #[test]
    fn only_the_recorded_server_passes_the_first_proof() {
        let dir =
            std::path::PathBuf::from(format!("/tmp/ae-autotrust-server-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let refusal = |meta: &str, server: &str| {
            std::fs::write(crate::store::open(&dir).meta_path(), meta).expect("meta");
            let server = ServerId::Selected(Selector::Name(server.to_owned()));
            let seat = Seat {
                dir: &dir,
                server: &server,
                session: "s",
                slot: "main",
                pane: "%0",
                tool: "claude",
            };
            prove(&seat, "/tmp").expect_err("no live pane to prove")
        };
        let roster = "seat.main=lead\nagent_bin.main=claude\n";
        let named = format!("tmux_server=ae-a\n{roster}");
        let refused = "the pane is not on the recorded server";
        assert_eq!(refusal(&named, "ae-b"), refused);
        assert_eq!(refusal(roster, "ae-a"), refused, "no selector");
        let doubled = format!("tmux_server=ae-a\ntmux_server=ae-a\n{roster}");
        assert_eq!(refusal(&doubled, "ae-a"), refused, "doubled");
        assert_eq!(refusal(&named, "ae-a"), "the seat has no launch id");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The pane's readings prove the seat only when slot, session and uuid
    /// all hold: each one wrong, alone, refuses, an unset or unread uuid too.
    #[test]
    fn the_pane_is_the_seat_only_when_slot_session_and_uuid_all_hold() {
        type Edit = fn(&mut ObservedViewer);
        let right = ObservedViewer {
            slot: Some("main".to_owned()),
            session: Some("s".to_owned()),
            session_uuid: OptionReading::Set("u-1".to_owned()),
            ..ObservedViewer::default()
        };
        assert!(is_seat(&right, "main", "s", "u-1"));
        let wrong: [(&str, Edit); 7] = [
            ("slot", |viewer| viewer.slot = Some("w1".to_owned())),
            ("no slot", |viewer| viewer.slot = None),
            ("session", |viewer| viewer.session = Some("t".to_owned())),
            ("no session", |viewer| viewer.session = None),
            ("uuid", |viewer| {
                viewer.session_uuid = OptionReading::Set("u-2".to_owned());
            }),
            ("uuid unset", |viewer| {
                viewer.session_uuid = OptionReading::Vacant;
            }),
            ("uuid unread", |viewer| {
                viewer.session_uuid = OptionReading::Unknown;
            }),
        ];
        for (what, edit) in wrong {
            let mut viewer = right.clone();
            edit(&mut viewer);
            assert!(!is_seat(&viewer, "main", "s", "u-1"), "{what}");
        }
    }

    /// A start already reserved is refused BEFORE anything is appended: an
    /// in-wait answer asks no `due`, so this is its only spent check.
    #[test]
    fn a_reserved_start_is_refused_without_a_second_reservation() {
        let dir =
            std::path::PathBuf::from(format!("/tmp/ae-autotrust-spent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let journal = crate::store::open(&dir).events_path();
        std::fs::write(&journal, format!("{RESERVATION}\n")).expect("journal");
        let server = ServerId::Selected(Selector::Name("ae-a".to_owned()));
        let seat = Seat {
            dir: &dir,
            server: &server,
            session: "s",
            slot: "main",
            pane: "%0",
            tool: "claude",
        };
        let proof = Proof {
            name: "lead".to_owned(),
            episode: "main:start:7".to_owned(),
            folder: std::path::PathBuf::from("/tmp"),
        };
        let refused = reserve(&seat, &proof, true).expect_err("already spent");
        assert_eq!(refused, "main:start:7 is already spent");
        assert!(
            matches!(crate::store::open(&dir).events_source(),
                SourceRead::Ready(bytes) if bytes == format!("{RESERVATION}\n").into_bytes()),
            "nothing appended"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The episode's pid is the ONE outermost tool row under the pane; none,
    /// two, or a row ae cannot place is no episode.
    #[test]
    fn the_start_is_the_one_outermost_tool_process_under_the_pane() {
        let pane = 100;
        assert_eq!(
            tool_pid(&[proc(pane, 1, "claude")], pane, "claude"),
            Ok(pane)
        );
        let nested = [
            proc(pane, 1, "zsh"),
            proc(200, pane, "claude"),
            proc(300, 200, "claude"),
        ];
        assert_eq!(tool_pid(&nested, pane, "claude"), Ok(200));
        let elsewhere = [
            proc(pane, 1, "zsh"),
            proc(200, pane, "claude"),
            proc(400, 1, "claude"),
        ];
        assert_eq!(
            tool_pid(&elsewhere, pane, "claude"),
            Ok(200),
            "outside the pane"
        );
        for (table, why) in [
            (vec![proc(pane, 1, "zsh")], "no live claude"),
            (
                vec![
                    proc(pane, 1, "zsh"),
                    proc(200, pane, "claude"),
                    proc(201, pane, "claude"),
                ],
                "more than one",
            ),
            (
                vec![
                    proc(pane, 1, "zsh"),
                    proc(200, pane, "claude"),
                    proc(500, 600, "claude"),
                ],
                "cannot place",
            ),
            (
                vec![proc(pane, 1, "zsh"), proc(200, pane, "agy")],
                "no live claude",
            ),
        ] {
            let refused = tool_pid(&table, pane, "claude").expect_err(why);
            assert!(refused.contains(why), "{refused}");
        }
        assert!(tool_pid(&[proc(pane, 1, "claude")], pane, "vim").is_err());
    }

    #[test]
    fn a_folder_that_is_not_one_is_said_in_words() {
        use crate::doors::StrictDirError::{Absent, NotDirectory, Unreadable};
        let path = std::path::PathBuf::from("/x");
        assert_eq!(place(&Absent { path: path.clone() }), "is gone");
        assert_eq!(
            place(&Unreadable {
                path: path.clone(),
                kind: std::io::ErrorKind::PermissionDenied,
            }),
            "is unreadable"
        );
        assert_eq!(place(&NotDirectory { path }), "is not a directory");
    }
}
