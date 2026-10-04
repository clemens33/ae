//! `ae chat` — the human's lane of a session, as a program.
//!
//! A READ view of data ae already keeps: it answers nothing, and what it
//! writes is its own — an ask it submits and `/close`'s withdrawal of one.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::board::{self, Inputs, Replies, Row, follow::Follow, terminal_text};
use crate::events::Event;
use crate::store::{self, Oversized, SourceRead};
use crate::{
    archive, doors, inventory, lifecycle, meta, reply, session, theme, tracked, transport, usage,
    watchdog_daemon,
};
use lane::Seat;

pub mod input;
pub mod lane;
pub mod needs;
pub mod open;
pub mod submit;
pub(crate) mod term;
pub(crate) mod toggle;
pub mod view;
pub(crate) mod wrap;

/// The usage text.
pub const USAGE: &str = "Usage: ae chat [session] [--follow] [--all]\n\n  session   the session to read (default: the session this pane belongs to)\n  --follow  keep printing what is new every 5 s until interrupted (Ctrl-C to stop)\n  --all     show every assistant reply of the lead pair (default: only the replies to lines you typed in its pane)\n";

/// A parsed `ae chat` argv.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    pub session: Option<String>,
    pub follow: bool,
    pub all: bool,
    /// Hidden: take input from the terminal, as the console window does.
    pub input: bool,
}

/// The argv did not parse; the offending token, when there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage(pub Option<String>);

impl Usage {
    /// The stderr text, offending token first when one was named.
    #[must_use]
    pub fn render(&self) -> String {
        match &self.0 {
            Some(token) => format!("ae chat: unexpected {token}\n{USAGE}"),
            None => USAGE.to_owned(),
        }
    }
}

/// Read `tail` — everything after the word `chat` (or its alias `console`).
///
/// # Errors
///
/// [`Usage`] for an unknown flag or a second session name.
pub fn parse(tail: &[String]) -> Result<Args, Usage> {
    let mut args = Args::default();
    for token in tail {
        match token.as_str() {
            "--follow" => args.follow = true,
            "--all" => args.all = true,
            "--input" => args.input = true,
            flag if flag.starts_with('-') => return Err(Usage(Some(token.clone()))),
            name if args.session.is_none() => args.session = Some(name.to_owned()),
            _ => return Err(Usage(Some(token.clone()))),
        }
    }
    Ok(args)
}

/// One console: the session it is bound to and what it has printed of it.
pub(crate) struct Console {
    name: String,
    dir: PathBuf,
    uuid: String,
    replies: Replies,
    home: Option<PathBuf>,
    printed: view::Printed,
    rows: Vec<Row>,
    follow: Option<Follow>,
    /// The last journal read whole, which every later read must begin with.
    journal: Option<Vec<Event>>,
    /// Every roster seat as the last meta read named them: what `/open` takes.
    roster: Option<Vec<needs::SeatRef>>,
    /// A view that redraws its whole lane each read (`ae app`): its lane
    /// carries every coverage reason that still stands, not only the new ones
    /// the chat prints once.
    standing: bool,
}

/// One read of a console's session: the lane, the "needs you" section and
/// what the print needs to know about how settled the read was.
pub(crate) struct Read {
    pub(crate) lane: lane::Lane,
    pub(crate) needs: Result<needs::Section, String>,
    board_gaps: usize,
    settled: bool,
    /// The journal was rewritten: the last read's length and this one's.
    rebased: Option<(usize, usize)>,
}

impl Console {
    /// A console on session `name` recorded at `dir`, nothing read yet.
    pub(crate) fn open(name: String, dir: PathBuf) -> Self {
        Self {
            uuid: recorded_uuid(&dir),
            name,
            dir,
            replies: Replies::ToHuman,
            home: doors::home(),
            printed: view::Printed::default(),
            rows: Vec::new(),
            follow: None,
            journal: None,
            roster: None,
            standing: false,
        }
    }

    /// [`Console::open`] for a view that redraws its lane on every read.
    pub(crate) fn open_standing(name: String, dir: PathBuf) -> Self {
        Self {
            standing: true,
            ..Self::open(name, dir)
        }
    }

    /// The session this console follows.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// The session's state directory.
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// The session's identity and lead-pair seats, read from its meta NOW:
    /// `Err` says why this console no longer follows the session it opened.
    pub(crate) fn seats(&self) -> Result<Vec<Seat>, String> {
        let bytes =
            meta::read_bytes(&self.dir).map_err(|err| format!("meta unreadable ({err})"))?;
        let uuid = archive::canonical_uuid(&lifecycle::meta_value(&bytes, "session_id"));
        if uuid.is_empty() {
            return Err("session_id is missing or malformed, so it cannot be bound".to_owned());
        }
        if uuid != self.uuid {
            return Err("the session was replaced or renamed".to_owned());
        }
        let lead_pair = lifecycle::meta_value(&bytes, "layout") == "lead-pair";
        let meta = meta::Meta::parse(&String::from_utf8_lossy(&bytes));
        Ok(meta
            .roster()
            .iter()
            .filter(|entry| watchdog_daemon::in_lead_pair(&entry.slot, lead_pair))
            .map(|entry| Seat {
                slot: entry.slot.clone(),
                name: entry.name.clone(),
                profile: entry.profile.clone(),
            })
            .collect())
    }

    /// One pass: [`Console::read`], then the text this console has not
    /// printed yet.
    fn pass(&mut self) -> Result<String, String> {
        let read = self.read()?;
        let mut text = read
            .rebased
            .map(|(last, now)| self.printed.rebase(last, now))
            .unwrap_or_default();
        let size = pane_size(&self.printed);
        self.printed.set_width(size.map(|size| size.width));
        text.push_str(&self.printed.step(&read.lane, read.board_gaps, read.settled));
        let now = crate::time::Timestamp::now();
        text.push_str(&self.printed.needs(&read.needs, size, now));
        Ok(text)
    }

    /// One read: the first reads everything, each later one asks the board's
    /// follow for the new transcript bytes and re-reads the whole journal, so a
    /// replaced or shrunk journal can never hide a record.
    pub(crate) fn read(&mut self) -> Result<Read, String> {
        let seats = self.seats()?;
        let slots: Vec<&str> = seats.iter().map(|seat| seat.slot.as_str()).collect();
        let keep = |entry: &meta::RosterEntry| slots.contains(&entry.slot.as_str());
        let sessions = [usage::SessionInput {
            name: self.name.clone(),
            path: self.dir.clone(),
        }];
        let inputs = Inputs {
            home: self.home.as_deref(),
            sessions: &sessions,
            replies: self.replies,
        };
        let seen = match self.follow.as_mut() {
            None => {
                let first = board::observe_selected(&inputs, None, None, &keep);
                self.follow = Some(Follow::seeded(&first.seeds, &first.coverage, None));
                first
            }
            Some(follow) => board::follow_poll_selected(&inputs, follow, &keep),
        };
        // A rescan re-reads a rewritten transcript, so a record at the same file
        // and offset is the NEW words: the shared collect keeps the first.
        let fresh: BTreeSet<(&str, u64)> = seen
            .rows
            .iter()
            .map(|row| (row.file.as_str(), row.offset))
            .collect();
        self.rows
            .retain(|row| !fresh.contains(&(row.file.as_str(), row.offset)));
        self.rows.extend(seen.rows.iter().cloned());
        self.rows = board::collect(std::mem::take(&mut self.rows));
        let board_gaps = seen.coverage.len();
        let coverage = match (&self.follow, self.standing) {
            (Some(follow), true) => standing_coverage(follow, &seats, &self.name, seen.coverage),
            _ => seen.coverage,
        };
        let observation = board::Observation {
            rows: self.rows.clone(),
            coverage,
            ..board::Observation::default()
        };
        let snapshot = session::RecordSnapshot::read(&self.dir);
        let needs = self.needs(&snapshot, &seats);
        // Only a settled read replaces the seats `/open` may name.
        if let (Ok(_), Some(meta)) = (&needs, &snapshot.meta) {
            let seat = |entry: &meta::RosterEntry| needs::SeatRef {
                slot: entry.slot.clone(),
                name: entry.name.clone(),
            };
            self.roster = Some(meta.roster().iter().map(seat).collect());
        }
        let (events, skipped, read_gap) = match snapshot.events {
            Some(read) => (read.events, read.skipped.len(), None),
            None => (Vec::new(), 0, Some("journal — unreadable".to_owned())),
        };
        // A journal no longer beginning with the last read was rewritten (resume
        // trims its head): positions name other records, so the thread reprints.
        let rebased = match (&read_gap, &self.journal) {
            (None, Some(last)) if !events.starts_with(last) => Some((last.len(), events.len())),
            _ => None,
        };
        let body = |event: &Event| body_for(&self.dir, event);
        let mut lane = lane::fold(&self.name, &seats, &events, &observation, skipped, &body);
        let settled = read_gap.is_none();
        lane.coverage.extend(read_gap);
        if settled {
            self.journal = Some(events);
        }
        Ok(Read {
            lane,
            needs,
            board_gaps,
            settled,
            rebased,
        })
    }

    /// The "needs you" section: EVERY roster seat's verdict, as `ae list` reads
    /// it, from the records `snapshot` holds, what tmux lists of the session's
    /// panes now and the watchdog's beat. The conversation stays the lead pair's.
    fn needs(
        &self,
        snapshot: &session::RecordSnapshot,
        seats: &[Seat],
    ) -> Result<needs::Section, String> {
        // The seats are the lead pair, so `worker.0` is among them exactly
        // when the layout is `lead-pair`.
        let lead_pair = seats.iter().any(|seat| seat.slot == "worker.0");
        needs_in(&self.name, &self.dir, snapshot, lead_pair)
    }
}

/// The "needs you" section of session `name` at `dir`, from the records
/// `snapshot` holds: [`Console::needs`]'s fold, for a caller with no console.
pub(crate) fn needs_in(
    name: &str,
    dir: &Path,
    snapshot: &session::RecordSnapshot,
    lead_pair: bool,
) -> Result<needs::Section, String> {
    let meta = snapshot.meta.as_ref();
    let server = meta
        .and_then(|meta| meta.server_selector().entitles().cloned())
        .map(inventory::ServerId::Selected);
    let agents = server
        .as_ref()
        .zip(meta)
        .and_then(|(server, meta)| crate::observed_agents(server, name, meta));
    let runtime_read = agents.is_some();
    let mut runtime = session::SessionRuntime::new(if runtime_read {
        crate::digest::Status::Running
    } else {
        crate::digest::Status::Unknown
    });
    runtime.agents = agents.unwrap_or_default();
    let now = crate::time::Timestamp::now();
    let unanswered = session::DEFAULT_UNANSWERED_SECS;
    let entry = session::entry_from(snapshot, name, &runtime, now, unanswered);
    needs::fold(&needs::Inputs {
        session: name,
        snapshot,
        entry: &entry,
        runtime: &runtime,
        runtime_read,
        beat: crate::watchdog_glue::beat_modified(dir),
        lead_pair,
        now,
    })
}

/// The coverage a redrawn lane carries: every reason the follow holds as
/// standing, in roster order, then any this read alone reported (a rescan).
fn standing_coverage(
    follow: &Follow,
    seats: &[Seat],
    session: &str,
    fresh: Vec<board::Coverage>,
) -> Vec<board::Coverage> {
    let rank = |actor: &str| {
        seats
            .iter()
            .position(|seat| actor == format!("{session}:{}", seat.name))
            .unwrap_or(usize::MAX)
    };
    let mut all = follow.standing();
    all.sort_by_key(|item| rank(&item.actor));
    for item in fresh {
        if !all.contains(&item) {
            all.push(item);
        }
    }
    all
}

/// The "needs you" section of session `name` at `dir`, read now: the meta
/// says whether it runs a lead pair, the records say the rest.
pub(crate) fn needs_of(name: &str, dir: &Path) -> Result<needs::Section, String> {
    let bytes = meta::read_bytes(dir).map_err(|err| format!("meta unreadable ({err})"))?;
    let lead_pair = lifecycle::meta_value(&bytes, "layout") == "lead-pair";
    needs_in(name, dir, &session::RecordSnapshot::read(dir), lead_pair)
}

/// A console reply's stored body, read only from THIS session's `messages/`:
/// the recorded path must end in a `messages` directory and a name the reply
/// writer gives this request, and the read takes that name from `dir` itself,
/// never the recorded directory, so no record can point the console elsewhere.
fn body_for(dir: &Path, event: &Event) -> lane::Body {
    const CAP: u64 = reply::CONSOLE_BODY_CAP as u64;
    let Some(recorded) = event.body_file.as_deref().map(Path::new) else {
        return lane::Body::OldCore;
    };
    let id = event.reference.as_deref().unwrap_or("");
    let in_messages = recorded.parent().and_then(Path::file_name) == Some("messages".as_ref());
    let name = recorded.file_name().and_then(|name| name.to_str());
    let Some(name) = name.filter(|name| in_messages && is_reply_body_name(id, name)) else {
        return lane::Body::Refused("not a reply body in this session's messages".to_owned());
    };
    match store::read_capped_in(&dir.join("messages"), name, CAP) {
        Ok(SourceRead::Ready(bytes)) => {
            String::from_utf8(bytes).map_or(lane::Body::NotUtf8, lane::Body::Whole)
        }
        Ok(SourceRead::Absent) => lane::Body::Missing,
        Ok(SourceRead::Invalid(what)) => lane::Body::Refused(what),
        Ok(SourceRead::Unreadable(why)) => lane::Body::Refused(format!("unreadable ({why})")),
        Err(Oversized) => lane::Body::Oversized,
    }
}

/// `<id>.reply.<six lowercase hex>.txt` — the name `deliver::store_body`
/// publishes a reply to request `id` under.
fn is_reply_body_name(id: &str, name: &str) -> bool {
    let hex = name
        .strip_prefix(id)
        .and_then(|rest| rest.strip_prefix(".reply."))
        .and_then(|rest| rest.strip_suffix(".txt"));
    let lower_hex = |byte: u8| matches!(byte, b'0'..=b'9' | b'a'..=b'f');
    !id.is_empty() && hex.is_some_and(|hex| hex.len() == 6 && hex.bytes().all(lower_hex))
}

/// `/close [id]`: one `cancel` for the console's own request `which`, or the
/// newest of its open asks in `session` when there is none, appended only when
/// the journal read under its lock holds that ask unanswered and unclosed. The
/// newest is chosen from that same read, so a reply landing meanwhile cannot
/// make it name an ask already closed.
///
/// # Errors
///
/// Why nothing was appended, by name.
pub fn close(dir: &Path, session: &str, which: Option<&str>) -> Result<(), String> {
    let decided = store::open(dir).append_event_decided(|journal| {
        let events = snapshot(journal)?;
        let newest = lane::open_asks(&events, session).last().copied();
        let id = which
            .or(newest)
            .ok_or_else(|| "no open ask to close".to_owned())?;
        lane::may_close(&events, id)?;
        let (now, summary) = (crate::time::Timestamp::now(), "closed from the console");
        let line = crate::state::event_line(now, tracked::CONSOLE_SINK, lane::CANCEL, id, summary);
        Ok(line)
    });
    decided.map_err(|why| why.to_string())?
}

/// The journal as `/close` decides on it: every complete line an [`Event`],
/// or a refusal naming the first that is not — never a guess past it.
fn snapshot(journal: SourceRead) -> Result<Vec<Event>, String> {
    let bytes = match journal {
        SourceRead::Ready(bytes) => bytes,
        SourceRead::Absent => Vec::new(),
        SourceRead::Invalid(what) => return Err(format!("the journal is {what}")),
        SourceRead::Unreadable(why) => return Err(format!("the journal is unreadable ({why})")),
    };
    let text = String::from_utf8(bytes).map_err(|_| "the journal is not UTF-8".to_owned())?;
    if !text.is_empty() && !text.ends_with('\n') {
        return Err("the journal ends in a partial record".to_owned());
    }
    let lines = text.lines().enumerate();
    lines
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(at, line)| {
            Event::parse_line(line)
                .map_err(|why| format!("journal line {} is not a record ({why})", at + 1))
        })
        .collect()
}

/// The session directory `name` records, or why there is none.
pub(crate) fn locate(root: &Path, name: &str) -> Option<PathBuf> {
    let roots = inventory::Roots::under(root);
    let scan = inventory::durable_meta_records(&roots);
    scan.records
        .iter()
        .find(|record| record.name == name)
        .map(|record| record.path.clone())
}

/// The canonical `session_id` the session's meta records, or empty.
pub(crate) fn recorded_uuid(dir: &Path) -> String {
    meta::read_bytes(dir)
        .map(|bytes| archive::canonical_uuid(&lifecycle::meta_value(&bytes, "session_id")))
        .unwrap_or_default()
}

/// How this chat is drawn: plain unless stdout is a terminal and the session's
/// look is drawn. Only then does it ask tmux — for the session's palette and
/// icons, and once for the viewer's clock zone.
fn dress(name: &str, seats: &[Seat]) -> view::Style {
    use std::io::IsTerminal as _;
    let tty = std::io::stdout().is_terminal();
    let (look, zone) = look_of(name, tty);
    view::Style::resolve(tty, look, zone.as_deref(), lead_name(seats))
}

/// Session `name`'s drawn look and the viewer's zone, asked of tmux only for
/// a terminal (`tty`) and the zone only for a look that is drawn: the ONE
/// reading the chat and `ae app` both dress by.
pub(crate) fn look_of(name: &str, tty: bool) -> (Option<theme::Look>, Option<String>) {
    let declared = doors::declared_server(crate::shape::current());
    let server = doors::launch_target(declared.as_ref()).filter(|_| tty);
    let look = server
        .as_ref()
        .and_then(|server| transport::observe_look(server, name));
    let look =
        look.map(|read| theme::Look::read(&read.icons, &read.palette, &read.drawn, &read.motion));
    let zone = || {
        server
            .as_ref()
            .and_then(|server| transport::observe_zone(server, name))
    };
    let zone = view::Style::wanted(tty, look).then(zone).flatten();
    (look, zone)
}

/// The name of the seat in the `main` slot, whose rows wear the lead hue; empty
/// when no seat holds it.
fn lead_name(seats: &[Seat]) -> &str {
    seats
        .iter()
        .find(|seat| seat.slot == "main")
        .map_or("", |seat| seat.name.as_str())
}

/// `ae chat [session] [--follow] [--all]`.
///
/// # Errors
///
/// [`crate::Error::Io`] when `out` or `err` cannot be written.
pub fn run(tail: &[String], out: &mut impl Write, err: &mut impl Write) -> crate::Result<u8> {
    let args = match parse(tail) {
        Ok(args) => args,
        Err(usage) => {
            write!(err, "{}", terminal_text(&usage.render()))?;
            return Ok(crate::entry::EXIT_USAGE);
        }
    };
    let Some(root) = crate::state_root() else {
        writeln!(err, "ae: {}", crate::NO_STATE_ROOT)?;
        return Ok(crate::EXIT_UNAVAILABLE);
    };
    let Some(name) = args.session.clone().or_else(crate::calling_session_name) else {
        writeln!(
            err,
            "ae chat: no session named and this pane is in none\n{USAGE}"
        )?;
        return Ok(crate::EXIT_UNAVAILABLE);
    };
    let Some(dir) = locate(&root, &name) else {
        writeln!(
            err,
            "{}",
            terminal_text(&format!("ae chat: no session named {name}"))
        )?;
        return Ok(crate::EXIT_UNAVAILABLE);
    };
    let mut console = Console::open(name, dir);
    if args.all {
        console.replies = Replies::All;
    }
    match console.seats() {
        Ok(seats) => {
            console.printed = view::Printed::styled(dress(&console.name, &seats));
            write!(out, "{}", console.printed.headline(&console.name, &seats))?;
        }
        Err(why) => {
            writeln!(
                err,
                "{}",
                terminal_text(&format!("ae chat: {}: {why}", console.name))
            )?;
            return Ok(crate::EXIT_UNAVAILABLE);
        }
    }
    let mut term = None;
    if args.input {
        match term::Term::start(&console) {
            Ok(started) => term = args.follow.then_some(started),
            Err(off) => write!(out, "{}", console.printed.style().dim(&off))?,
        }
    }
    let code = pump(&mut console, term.as_mut(), args.follow, out, err);
    if term.is_some() {
        write!(out, "{}", input::RESTORE)?;
        out.flush()?;
    }
    code
}

/// The size in cells of the pane this chat draws in, asked of tmux through the
/// pane-size door; `None` when the console is not dressed, is in no tmux pane or
/// tmux does not answer, and then its rows are not wrapped.
fn pane_size(printed: &view::Printed) -> Option<input::Size> {
    if !printed.dressed() {
        return None;
    }
    let declared = doors::declared_server(crate::shape::current());
    let server = doors::launch_target(declared.as_ref())?;
    let pane = doors::calling_pane_id()?;
    let size = transport::observe_pane_size(&server, &pane)?;
    Some(input::Size {
        width: size.width,
        height: size.height,
    })
}

/// Print each pass, and between passes take the terminal's input when there
/// is a `term`, until the console stops following.
fn pump(
    console: &mut Console,
    mut term: Option<&mut term::Term>,
    follow: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let poll = std::time::Duration::from_secs(board::follow::POLL_SECS);
    let mut next = std::time::Instant::now();
    loop {
        if std::time::Instant::now() >= next {
            let text = match console.pass() {
                Ok(text) => text,
                Err(why) => {
                    let text = format!(
                        "ae chat: {}: {why} — this chat no longer follows it; reopen it with prefix h (or ae chat <session>)",
                        console.name
                    );
                    if let Some(term) = term.as_deref_mut() {
                        write!(out, "{}{}", term.settle(), console.printed.flush_outcomes())?;
                        out.flush()?;
                    }
                    writeln!(err, "{}", terminal_text(&text))?;
                    return Ok(crate::EXIT_UNAVAILABLE);
                }
            };
            match term.as_deref_mut() {
                Some(term) => {
                    let effects = term.tick(console);
                    write!(out, "{}", term.paint(&text, &effects))?;
                }
                None => write!(out, "{text}")?,
            }
            out.flush()?;
            if !follow {
                return Ok(0);
            }
            next = std::time::Instant::now() + poll;
        }
        match term.as_deref_mut() {
            Some(term) => {
                if let Some(effects) = term.wait(console, next) {
                    write!(out, "{}", term.paint("", &effects))?;
                    out.flush()?;
                }
            }
            None => std::thread::sleep(next.saturating_duration_since(std::time::Instant::now())),
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::{Console, Replies, lane::Body, view::Printed};
    use crate::events::Event;
    use std::fs;
    use std::path::PathBuf;

    pub(super) const ID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
    const REQ: &str = "ae-20260930T060000Z-0000abcd";

    /// A lead-pair session directory under a scratch that removes itself.
    pub(super) struct Rig(pub(super) PathBuf);

    impl Rig {
        pub(super) fn new(tag: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("ae-console-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("s")).expect("a scratch dir");
            fs::write(root.join("s/meta"), format!("schema=2\nsession_id={ID}\nlayout=lead-pair\nseat.main=lead\nagent_bin.main=claude\nharness_session.main={ID}\nconfig_home.main={}\n", root.join("claude").display())).expect("meta");
            Self(root)
        }

        pub(super) fn console(&self) -> Console {
            Console {
                name: "s".to_owned(),
                dir: self.0.join("s"),
                uuid: ID.to_owned(),
                replies: Replies::Off,
                home: Some(self.0.clone()),
                printed: Printed::default(),
                rows: Vec::new(),
                follow: None,
                journal: None,
                roster: None,
                standing: false,
            }
        }

        /// The lead's one-line transcript, its mtime `secs` after now.
        fn transcript(&self, words: &str, secs: u64) {
            let dir = self.0.join("claude/projects/w");
            fs::create_dir_all(&dir).expect("a project dir");
            let path = dir.join(format!("{ID}.jsonl"));
            let line = format!(
                r#"{{"type":"user","timestamp":"2026-09-30T06:00:00Z","message":{{"role":"user","content":"{words}"}}}}"#
            );
            fs::write(&path, line + "\n").expect("transcript");
            let later = std::time::SystemTime::now() + std::time::Duration::from_secs(secs);
            let file = fs::File::options().write(true).open(&path).expect("reopen");
            file.set_modified(later).expect("mtime");
        }

        fn journal(&self, lines: &[&str]) {
            fs::write(self.0.join("s/events.jsonl"), lines.join("\n") + "\n").expect("journal");
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A reply to console request `REQ` naming `path` as its body.
    fn with_body(path: &str) -> Event {
        let line = format!(
            r#"{{"ts":"2026-09-30T06:01:00Z","actor":"lead","action":"reply","target":"console:local","ref":"{REQ}","body_file":"{path}"}}"#
        );
        Event::parse_line(&line).expect("a reply record")
    }

    #[test]
    fn a_console_reply_body_is_read_only_from_this_sessions_messages() {
        let rig = Rig::new("bodies");
        let dir = rig.0.join("s");
        let name = |hex: &str| format!("{REQ}.reply.{hex}.txt");
        let messages = dir.join("messages");
        fs::create_dir_all(&messages).expect("messages");
        fs::write(messages.join(name("00000a")), "whole ü\n  indented").expect("a body");
        fs::write(messages.join(name("00000b")), vec![b'x'; 65_537]).expect("a big body");
        fs::write(messages.join(name("00000c")), [0xff, 0xfe]).expect("bytes");
        fs::write(rig.0.join("outside"), "secret").expect("outside");
        std::os::unix::fs::symlink(rig.0.join("outside"), messages.join(name("00000d")))
            .expect("link");
        let body = |path: &str| super::body_for(&dir, &with_body(path));
        let at = |hex: &str| format!("/elsewhere/messages/{}", name(hex));
        // Read from THIS session's messages, never from the recorded directory,
        // wherever it is: a foreign `messages`, or one reached through `..`.
        let foreign = rig.0.join("foreign/messages");
        fs::create_dir_all(&foreign).expect("a foreign messages dir");
        fs::write(foreign.join(name("00000a")), "FOREIGN").expect("a foreign body");
        let whole = Body::Whole("whole ü\n  indented".to_owned());
        let foreign = format!("{}/{}", foreign.display(), name("00000a"));
        for path in [
            at("00000a"),
            foreign,
            format!("/x/../y/messages/{}", name("00000a")),
        ] {
            assert_eq!(body(&path), whole, "{path}");
        }
        assert_eq!(body(&at("00000b")), Body::Oversized);
        assert_eq!(body(&at("00000c")), Body::NotUtf8);
        assert_eq!(body(&at("00000d")), Body::Refused("a symlink".to_owned()));
        assert_eq!(body(&at("00000e")), Body::Missing);
        let refused = Body::Refused("not a reply body in this session's messages".to_owned());
        for path in [
            format!("{}/../outside", messages.display()),
            format!("/x/messages/../{}", name("00000a")),
            format!("/x/other/{}", name("00000a")),
            format!("/x/messages/{REQ}.ask.00000a.txt"),
            format!("/x/messages/{REQ}.reply.00000A.txt"),
            "/x/messages/other.reply.00000a.txt".to_owned(),
        ] {
            assert_eq!(body(&path), refused, "{path}");
        }
        fs::rename(&messages, dir.join("real")).expect("move");
        std::os::unix::fs::symlink(dir.join("real"), &messages).expect("linked messages");
        let linked = Body::Refused("its directory is a symlink".to_owned());
        assert_eq!(body(&at("00000a")), linked);
        let old = r#"{"ts":"2026-09-30T06:01:00Z","actor":"lead","action":"reply","ref":"r"}"#;
        let old = Event::parse_line(old).expect("an old-core reply");
        assert_eq!(super::body_for(&dir, &old), Body::OldCore);
    }

    #[test]
    fn a_pass_shows_an_admitted_answer_whole_from_its_stored_body() {
        let rig = Rig::new("answer");
        let file = rig.0.join(format!("s/messages/{REQ}.reply.0000aa.txt"));
        fs::create_dir_all(rig.0.join("s/messages")).expect("messages");
        fs::write(&file, "line one\nline two").expect("a body");
        let ask = format!(
            r#"{{"ts":"2026-09-30T06:00:00Z","actor":"console:local","action":"ask","target":"lead","ref":"{REQ}","target_slot":"main","target_session":"s","target_server":"/t","target_pane":"%1","target_session_uuid":"{ID}","summary":"q"}}"#
        );
        let answer = format!(
            r#"{{"ts":"2026-09-30T06:01:00Z","actor":"lead","action":"reply","target":"console:local","ref":"{REQ}","actor_slot":"main","actor_session":"s","caller_server":"/t","caller_pane":"%1","caller_session_uuid":"{ID}","body_file":"{}","summary":"line one line two"}}"#,
            file.display()
        );
        rig.journal(&[&ask, &answer]);
        let shown = rig.console().pass().expect("a pass");
        let whole = "lead answers\n  line one\n  line two";
        assert!(shown.contains(whole), "{shown}");
    }

    /// `/close` appends one console `cancel` only to an open console ask of
    /// THIS journal; any refusal, a journal it cannot read whole included,
    /// leaves every journal byte-identical.
    #[test]
    fn a_close_withdraws_only_an_open_console_ask_of_this_journal() {
        let rig = Rig::new("close");
        let (dir, other) = (rig.0.join("s"), rig.0.join("t"));
        let journal = |dir: &PathBuf| crate::store::open(dir).container();
        let ask = format!(
            r#"{{"ts":"2026-09-30T06:00:00Z","actor":"console:local","action":"ask","target":"lead","ref":"{REQ}","target_slot":"main","target_session":"s","target_server":"/t","target_pane":"%1","target_session_uuid":"{ID}"}}"#
        );
        let answer = format!(
            r#"{{"ts":"2026-09-30T06:01:00Z","actor":"lead","action":"reply","target":"console:local","ref":"{REQ}","actor_slot":"main","actor_session":"s","caller_server":"/t","caller_pane":"%1","caller_session_uuid":"{ID}"}}"#
        );
        fs::create_dir_all(&other).expect("another session");
        fs::write(other.join("events.jsonl"), format!("{ask}\n")).expect("its journal");
        rig.journal(&[&ask]);
        assert_eq!(super::close(&dir, "s", Some(REQ)), Ok(()));
        let closed = journal(&dir);
        let cancel = format!(r#""actor":"console:local","action":"cancel","ref":"{REQ}","#);
        assert!(
            String::from_utf8_lossy(&closed)
                .lines()
                .last()
                .is_some_and(|l| l.contains(&cancel))
        );
        let states = crate::requests::states(&closed);
        assert_eq!(states.len(), 1, "{states:?}");
        assert_eq!(states[0].status, crate::requests::Status::Cancelled);
        for (bytes, why) in [
            (closed, "that ask is already closed".to_owned()),
            (
                format!("{WORK}\n\n").into(),
                "no chat ask with that id in this journal".to_owned(),
            ),
            (
                format!("{ask}\n{answer}\n").into(),
                "that ask is already answered".to_owned(),
            ),
            (
                format!("{ask}\n{{not json\n").into(),
                "journal line 2 is not a record".to_owned(),
            ),
            (
                format!("{ask}\n{{\"ts\"").into(),
                "the journal ends in a partial record".to_owned(),
            ),
            (
                b"{}\n\xff\n".to_vec(),
                "the journal is not UTF-8".to_owned(),
            ),
        ] {
            fs::write(dir.join("events.jsonl"), &bytes).expect("a journal");
            let result = super::close(&dir, "s", Some(REQ));
            assert!(
                result.as_ref().is_err_and(|got| got.starts_with(&why)),
                "{result:?}"
            );
            assert_eq!(journal(&dir), bytes, "a refusal writes nothing");
        }
        assert_eq!(
            journal(&other),
            format!("{ask}\n").as_bytes(),
            "never another session's"
        );
        let refless =
            b"{\"ts\":\"2026-09-30T06:00:00Z\",\"actor\":\"console:local\",\"action\":\"ask\"}\n";
        fs::write(dir.join("events.jsonl"), refless).expect("a journal");
        assert!(
            super::close(&dir, "s", Some("")).is_err(),
            "a close needs an id"
        );
        assert_eq!(journal(&dir), refless, "and writes nothing");
        fs::rename(dir.join("events.jsonl"), rig.0.join("real")).expect("move");
        std::os::unix::fs::symlink(rig.0.join("real"), dir.join("events.jsonl")).expect("link");
        let linked = Err("the journal is a symlink".to_owned());
        assert_eq!(super::close(&dir, "s", Some(REQ)), linked);
    }

    const ASK: &str = r#"{"ts":"2026-09-30T06:00:00Z","actor":"lead","action":"state","ref":"waiting-user","summary":"ship it?"}"#;
    const SAY_A: &str =
        r#"{"ts":"2026-09-30T06:01:00Z","actor":"lead","action":"chat","summary":"aaaa"}"#;
    const SAY_B: &str =
        r#"{"ts":"2026-09-30T06:01:00Z","actor":"lead","action":"chat","summary":"bbbb"}"#;
    const WORK: &str = r#"{"ts":"2026-09-30T06:02:00Z","actor":"lead","action":"state","ref":"working","summary":"go"}"#;

    #[test]
    fn a_replaced_same_size_journal_shows_its_new_record_and_a_cleared_card_closes() {
        let rig = Rig::new("journal");
        let mut console = rig.console();
        rig.journal(&[ASK, SAY_A]);
        let first = console.pass().expect("first pass");
        assert!(
            first.contains("DECISION lead") && first.contains("aaaa"),
            "{first}"
        );
        assert_eq!(console.pass().expect("quiet"), "");
        assert_eq!(SAY_A.len(), SAY_B.len());
        rig.journal(&[ASK, SAY_B]);
        let swapped = console.pass().expect("replaced");
        assert!(
            swapped.contains("bbbb") && !swapped.contains("aaaa"),
            "{swapped}"
        );
        rig.journal(&[ASK, SAY_B, WORK]);
        let closed = console.pass().expect("answered");
        assert!(
            closed.contains("-- closed: DECISION lead · waiting-user"),
            "{closed}"
        );
    }

    #[test]
    fn a_same_size_rewrite_of_a_transcript_prints_its_new_words_on_the_next_pass() {
        let rig = Rig::new("rewrite");
        let mut console = rig.console();
        rig.transcript("aaaa", 0);
        let first = console.pass().expect("first pass");
        assert!(first.contains("aaaa"), "{first}");
        rig.transcript("bbbb", 2);
        let second = console.pass().expect("rewritten");
        assert!(
            second.contains("bbbb") && second.contains("rescanned"),
            "{second}"
        );
    }

    /// Console ask `n`: its own request id and words.
    fn asked(n: u8, words: &str) -> String {
        format!(
            r#"{{"ts":"2026-09-30T06:0{n}:00Z","actor":"console:local","action":"ask","target":"lead","ref":"ae-20260930T06000{n}Z-0000abcd","summary":"{words}"}}"#
        )
    }

    const REWRITTEN: &str = "-- journal rewritten";

    #[test]
    fn a_journal_trimmed_at_its_head_is_named_and_hides_no_newer_console_row() {
        let (rig, three) = (Rig::new("trimmed"), asked(3, "charlie"));
        let mut console = rig.console();
        rig.journal(&[&asked(1, "alpha"), &asked(2, "bravo"), &three]);
        assert!(console.pass().expect("first").contains("charlie"));
        rig.journal(&[&three, &asked(4, "delta")]);
        let got = console.pass().expect("trimmed");
        let notice = format!("{REWRITTEN} (3 records before, 2 now)");
        assert!(got.contains(&notice) && got.contains("delta"), "{got}");
        assert!(!got.contains("alpha"), "{got}");
        let quiet = console.pass().expect("unchanged");
        assert_eq!(quiet, "", "one notice per rewrite");
    }

    #[test]
    fn a_rewritten_head_is_named_even_across_an_unreadable_read() {
        let (rig, two) = (Rig::new("head"), asked(2, "bravo"));
        let mut console = rig.console();
        rig.journal(&[&asked(1, "alpha"), &two]);
        assert!(console.pass().expect("first").contains("alpha"));
        rig.journal(&[&asked(5, "echo"), &two]);
        let got = console.pass().expect("swapped");
        assert!(got.contains(REWRITTEN) && got.contains("echo"), "{got}");
        let path = rig.0.join("s/events.jsonl");
        fs::remove_file(&path).expect("remove");
        fs::create_dir(&path).expect("a directory where the journal was");
        assert!(!console.pass().expect("unreadable").contains(REWRITTEN));
        fs::remove_dir(&path).expect("restore");
        rig.journal(&[&asked(6, "foxtrot"), &two]);
        let got = console.pass().expect("rewritten while unread");
        assert!(got.contains(REWRITTEN) && got.contains("foxtrot"), "{got}");
    }

    #[test]
    fn a_transiently_unreadable_journal_neither_closes_a_card_nor_reprints_it() {
        let rig = Rig::new("unreadable");
        let mut console = rig.console();
        rig.journal(&[ASK]);
        assert!(console.pass().expect("first").contains("DECISION lead"));
        let path = rig.0.join("s/events.jsonl");
        fs::remove_file(&path).expect("remove");
        fs::create_dir(&path).expect("a directory where the journal was");
        let blind = console.pass().expect("unreadable");
        assert!(
            blind.contains("journal — unreadable") && !blind.contains("closed"),
            "{blind}"
        );
        fs::remove_dir(&path).expect("restore");
        rig.journal(&[ASK]);
        assert_eq!(console.pass().expect("recovered"), "");
    }

    /// A following console sleeps its poll out between passes: the deadline
    /// it waits for is ahead of it, never behind.
    #[test]
    fn a_following_console_waits_its_poll_out_before_it_passes_again() {
        use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
        use std::time::{Duration, Instant};
        struct Flushes(Sender<Instant>);
        impl std::io::Write for Flushes {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                let gone = |_| std::io::ErrorKind::BrokenPipe.into();
                self.0.send(Instant::now()).map_err(gone)
            }
        }
        let rig = Rig::new("cadence");
        let (mut console, (send, flushed)) = (rig.console(), channel());
        let pump = std::thread::spawn(move || {
            let (out, err) = (&mut Flushes(send), &mut std::io::sink());
            super::pump(&mut console, None, true, out, err)
        });
        let first = flushed
            .recv_timeout(Duration::from_secs(30))
            .expect("a first pass");
        let again = flushed.recv_timeout(Duration::from_secs(1));
        assert_eq!(
            again,
            Err(RecvTimeoutError::Timeout),
            "a pass inside the poll"
        );
        fs::write(rig.0.join("s/meta"), "schema=2\n").expect("the session ends");
        let code = pump.join().expect("the pump returns").expect("its writes");
        assert_eq!(code, crate::EXIT_UNAVAILABLE);
        let poll = Duration::from_secs(super::board::follow::POLL_SECS);
        assert!(
            first.elapsed() >= poll,
            "the next pass waited the whole poll"
        );
    }

    #[test]
    fn a_console_stops_when_its_session_is_replaced_renamed_or_gone() {
        let rig = Rig::new("bound");
        let mut console = rig.console();
        assert!(console.pass().is_ok());
        let other =
            "session_id=0199c0de-bbbb-4890-abcd-ef0123456789\nlayout=lead-pair\nseat.main=lead\n";
        fs::write(rig.0.join("s/meta"), format!("schema=2\n{other}")).expect("replaced");
        assert!(
            console.pass().is_err(),
            "a new session id is not this session"
        );
        fs::remove_dir_all(rig.0.join("s")).expect("gone");
        assert!(
            console.pass().is_err(),
            "a missing directory is not this session"
        );
    }

    /// The whole argv grammar: every flag arm, the one-session guard and the
    /// unknown-flag arm, each with the token a refusal names.
    #[test]
    fn the_argv_grammar_accepts_flags_and_one_session_in_any_order_and_names_a_refusal() {
        let args = |session: Option<&str>, follow, all| {
            Ok(super::Args {
                session: session.map(str::to_owned),
                follow,
                all,
                input: false,
            })
        };
        let refused = |token: &str| Err(super::Usage(Some(token.to_owned())));
        let input = Ok(super::Args {
            input: true,
            ..super::Args::default()
        });
        let cases: [(&[&str], Result<super::Args, super::Usage>); 13] = [
            (&["--input"], input),
            (&[], args(None, false, false)),
            (&["s"], args(Some("s"), false, false)),
            (&["--follow"], args(None, true, false)),
            (&["--all"], args(None, false, true)),
            (&["s", "--follow", "--all"], args(Some("s"), true, true)),
            (&["--all", "--follow", "s"], args(Some("s"), true, true)),
            (&["--follow", "s", "--all"], args(Some("s"), true, true)),
            (&["--frob"], refused("--frob")),
            (&["-"], refused("-")),
            (&["s", "--frob"], refused("--frob")),
            (&["s", "t"], refused("t")),
            (&["--follow", "s", "t", "--all"], refused("t")),
        ];
        for (words, want) in cases {
            let words: Vec<String> = words.iter().map(|w| (*w).to_owned()).collect();
            assert_eq!(super::parse(&words), want, "{words:?}");
        }
    }

    #[test]
    fn the_lead_hue_belongs_to_the_main_slot_whatever_order_the_seats_come_in() {
        let seat = |slot: &str, name: &str| super::Seat {
            slot: slot.to_owned(),
            name: name.to_owned(),
            profile: None,
        };
        let pair = [seat("worker.0", "colead"), seat("main", "lead")];
        assert_eq!(super::lead_name(&pair), "lead");
        assert_eq!(super::lead_name(&pair[..1]), "");
        assert_eq!(super::lead_name(&[]), "");
    }
}
