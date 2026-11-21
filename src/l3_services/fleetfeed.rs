//! The fleet lifecycle feed: the session-level events of EVERY session, in one
//! bounded journal under the state root, for the orchestrator chat to draw.
//!
//! WHAT it holds (ruling 89): a session was created, resumed, stopped, ended
//! (Keep or purge) or renamed, by whom, and why when a reason was given. Auto
//! reseats and dead-seat relaunches stay in their own journals.
//!
//! ONE writer: [`record`]. Every lifecycle verb calls it once, after its effect
//! is proved, so a refused, failed or already-done operation writes nothing.
//! It appends to `<root>/fleet/events.jsonl` with
//! [`crate::store::SessionStore::append_event_bounded`]: append and bound in one
//! lock hold and one atomic replace, so many processes may write at once and no
//! reader sees more than [`KEEP`] rows or a torn one. A failed append is
//! reported by [`record_or_warn`] and never blocks the verb. Residuals, named:
//! a crash between a verb's effect and its row, or a lock not taken within the
//! store's wait, loses that one event; a retry never writes a second.
//!
//! The reader is [`read`], one file read per refresh, cached by the journal's
//! identity ([`Feed`]). The file is hostile input. It is read through the
//! store's capped, classified read, which proves the directory and the node
//! BEFORE any open (a FIFO never blocks the chat) and never reads past the cap,
//! and parsed by the existing event parser. A row that is not one of ours,
//! repeats an earlier row's `ref` or has no name is skipped and counted, never
//! drawn. A session name is only text here: a name from before the name
//! grammar is drawn sanitized, and no path is ever built from it.
//!
//! Every row carries a unique `ref` (a fresh uuid). The chat's item identity is
//! that ref, so a trim of the journal's head cannot change which rows a
//! follower has already printed.

use std::io::Write;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::Path;

use crate::events::Event;
use crate::time::Timestamp;
use crate::tracked::EventFields;

/// The directory of the journal, under the state root.
pub const DIR: &str = "fleet";

/// The most rows the journal ever holds.
pub const KEEP: usize = 400;

/// The most characters of a stored reason, parked excerpt, name or actor word.
pub const TEXT_CAP: usize = 200;

/// The most characters of an actor word.
pub const ACTOR_CAP: usize = 64;

/// The most characters of a session name drawn.
const NAME_CAP: usize = 128;

/// A chat shows at most this many earlier rows when it opens...
pub const BACKLOG_ROWS: usize = 10;

/// ...none older than this many seconds.
pub const BACKLOG_SECS: i64 = 24 * 60 * 60;

/// The prefix a parking memo carries; the feed shows what follows it.
const PARKING_PREFIX: &str = "resume here:";

/// What happened to a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    /// A launch built the session.
    Created,
    /// A launch started a stopped session again.
    Resumed,
    /// A stop killed the session.
    Stopped,
    /// An end removed the session (Keep or purge).
    Ended,
    /// A rename changed its name.
    Renamed,
}

impl What {
    /// The journal action of the row.
    #[must_use]
    pub const fn action(self) -> &'static str {
        match self {
            Self::Created => "fleet-created",
            Self::Resumed => "fleet-resumed",
            Self::Stopped => "fleet-stopped",
            Self::Ended => "fleet-ended",
            Self::Renamed => "fleet-renamed",
        }
    }

    /// The word the feed line names the event by.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Resumed => "resumed",
            Self::Stopped => "stopped",
            Self::Ended => "ended",
            Self::Renamed => "renamed",
        }
    }

    fn of_action(action: &str) -> Option<Self> {
        [
            Self::Created,
            Self::Resumed,
            Self::Stopped,
            Self::Ended,
            Self::Renamed,
        ]
        .into_iter()
        .find(|what| what.action() == action)
    }
}

/// One event to record. `actor` is the spelling the lifecycle journals use
/// (`human`, `ae/orchestrator`, a seat's display name, `slot:<slot>`, ...);
/// [`actor_word`] turns it into the word the feed shows. `note` is the reason
/// of a stop or end, the parked excerpt of a resume, the old name of a rename.
#[derive(Debug, Clone, Copy)]
pub struct Fact<'a> {
    /// What happened.
    pub what: What,
    /// The session, by the name it had when it happened (the new name of a rename).
    pub session: &'a str,
    /// Its canonical session uuid, or empty when none is known.
    pub uuid: &'a str,
    /// Who did it, as the journals spell it.
    pub actor: &'a str,
    /// The reason, parked excerpt or previous name; empty when none.
    pub note: &'a str,
}

/// The word the feed shows for a journal actor spelling: `owner` for the
/// human (outside tmux, or in a pane that is not an agent seat), `orchestrator`,
/// `watchdog`, `reboot`; a seat's own display name or `slot:<slot>` stays as it
/// is; an empty spelling is `unknown`. Folded to one line and capped.
#[must_use]
pub fn actor_word(journal: &str) -> String {
    match journal {
        "human" | "ae:launch" | "ae:stop" | "ae:rename" => "owner".to_owned(),
        crate::tracked::ORCHESTRATOR_DISPLAY => "orchestrator".to_owned(),
        "ae:reboot" => "reboot".to_owned(),
        other => {
            let word = clean(other, ACTOR_CAP);
            if word.is_empty() {
                "unknown".to_owned()
            } else {
                word
            }
        }
    }
}

/// `text` as one line: every run of whitespace and control characters is one
/// space, the ends are trimmed, and a text longer than `cap` characters is cut
/// to `cap - 1` characters and an ellipsis. The one sanitizer of everything
/// the feed stores or draws.
#[must_use]
pub fn clean(text: &str, cap: usize) -> String {
    let flat = crate::brief::clean_text(text);
    // Bidirectional overrides and isolates reorder the line around them.
    let flat: String = flat
        .chars()
        .map(|ch| {
            if matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                ' '
            } else {
                ch
            }
        })
        .collect();
    let flat = flat.trim();
    if flat.chars().count() <= cap {
        return flat.to_owned();
    }
    let mut cut: String = flat.chars().take(cap.saturating_sub(1)).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

/// The parked excerpt of a session: its latest `parking` memo with the
/// `resume here:` lead taken off, one line, capped. `None` when it has none.
#[must_use]
pub fn parked(memo: &[u8]) -> Option<String> {
    let filed = crate::brief::filed(memo);
    let text = &filed.iter().find(|line| line.topic == "parking")?.text;
    let text = clean(text, TEXT_CAP);
    let text = match text.get(..PARKING_PREFIX.len()) {
        Some(lead) if lead.eq_ignore_ascii_case(PARKING_PREFIX) => {
            text[PARKING_PREFIX.len()..].trim().to_owned()
        }
        _ => text,
    };
    (!text.is_empty()).then_some(text)
}

/// What the session in `dir` parked: [`parked`] of its memo, read quietly.
#[must_use]
pub fn parked_in(dir: &Path) -> Option<String> {
    parked(&crate::store::open(dir).memo_bytes_or_empty())
}

/// Record `fact` as one new row. The row's `ref` is a fresh uuid.
///
/// # Errors
///
/// Why the row could not be appended: the directory, the lock or the write.
pub fn record(root: &Path, fact: &Fact<'_>) -> Result<(), String> {
    let dir = root.join(DIR);
    ensure_dir(&dir)?;
    let actor = actor_word(fact.actor);
    let note = clean(fact.note, TEXT_CAP);
    let id = crate::launch::generate_uuid();
    let line = crate::tracked::event_line(&EventFields {
        ts: Timestamp::now(),
        actor: &actor,
        action: fact.what.action(),
        target: fact.session,
        reference: &id,
        actor_slot: "",
        actor_session: "",
        target_slot: "",
        target_session: "",
        actor_session_id: "",
        target_session_id: fact.uuid,
        target_server: "",
        target_pane: "",
        target_session_uuid: "",
        caller_server: "",
        caller_pane: "",
        caller_session_uuid: "",
        identity_gap: "",
        summary: &note,
        body_file: "",
    });
    crate::store::open(&dir)
        .append_event_bounded(&line, KEEP)
        .map_err(|why| why.to_string())
}

/// [`record`], saying on `err` why it failed. The feed is observability: a
/// failure never blocks or undoes the verb that called it.
pub fn record_or_warn(root: &Path, fact: &Fact<'_>, err: &mut impl Write) {
    if let Err(why) = record(root, fact) {
        let _ = writeln!(
            err,
            "Warning: the fleet feed did not record that '{}' was {} ({why}).",
            fact.session,
            fact.what.word()
        );
    }
}

/// The journal's directory, made private when absent; a link or a file there
/// is refused, never followed.
fn ensure_dir(dir: &Path) -> Result<(), String> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => Ok(()),
        Err(why) if why.kind() == std::io::ErrorKind::AlreadyExists => {
            crate::store::prove_real_directory(dir)
                .map_err(|why| format!("{}: {why}", dir.display()))
        }
        Err(why) => Err(format!("{}: {why}", dir.display())),
    }
}

/// One row as the reader keeps it. Every text is already clean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// When it was recorded.
    pub ts: Timestamp,
    /// The row's unique `ref`: the item identity of a chat.
    pub id: String,
    /// What happened.
    pub what: What,
    /// The session, by the name it had then.
    pub session: String,
    /// The actor word.
    pub actor: String,
    /// The reason, parked excerpt or previous name; empty when none.
    pub note: String,
}

impl Row {
    /// The line without its time: `session event by actor — reason`. The
    /// reason is left out when there is none; a resume reads `— parked: …`; a
    /// rename names the old name, `new renamed (was old) by actor`.
    #[must_use]
    pub fn words(&self) -> String {
        let Self {
            what,
            session,
            actor,
            note,
            ..
        } = self;
        let word = what.word();
        match (what, note.is_empty()) {
            (What::Renamed, false) => format!("{session} {word} (was {note}) by {actor}"),
            (What::Resumed, false) => format!("{session} {word} by {actor} — parked: {note}"),
            (What::Stopped | What::Ended, false) => format!("{session} {word} by {actor} — {note}"),
            _ => format!("{session} {word} by {actor}"),
        }
    }
}

/// What one read of the journal found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Read {
    /// The valid rows, oldest first.
    pub rows: Vec<Row>,
    /// Lines and rows that were not valid and are not drawn.
    pub skipped: usize,
    /// The journal exists and cannot be read.
    pub unreadable: bool,
}

/// Read the journal under `root`. A missing journal is an empty read; a node
/// that is not a regular file, a directory that is not a real directory, a
/// journal over `store::BOUNDED_CAP` and a failed read are
/// `unreadable`.
#[must_use]
pub fn read(root: &Path) -> Read {
    use crate::store::SourceRead;
    let read = crate::store::read_capped_in(
        &root.join(DIR),
        crate::store::EVENTS,
        crate::store::BOUNDED_CAP,
    );
    match read {
        Ok(SourceRead::Absent) => Read::default(),
        Ok(SourceRead::Ready(bytes)) => {
            let drain =
                crate::events::EventLog::parse_bytes(&bytes, crate::events::Cursor::default());
            rows_of(&drain.events, drain.skipped.len())
        }
        Ok(SourceRead::Invalid(_) | SourceRead::Unreadable(_)) | Err(_) => Read {
            unreadable: true,
            ..Read::default()
        },
    }
}

/// The valid fleet rows among `events`, with `lines_skipped` the unparseable
/// lines the reader already counted. A row of another action is not ours and
/// is ignored; a fleet row that fails its grammar or repeats a `ref` is
/// counted.
fn rows_of(events: &[Event], lines_skipped: usize) -> Read {
    let mut read = Read {
        skipped: lines_skipped,
        ..Read::default()
    };
    let mut seen = std::collections::BTreeSet::new();
    for event in events {
        let Some(what) = What::of_action(&event.action) else {
            continue;
        };
        let (Some(id), Some(session)) = (event.reference.as_deref(), event.target.as_deref())
        else {
            read.skipped += 1;
            continue;
        };
        // A name is text: one written before the name grammar is drawn clean.
        let session = clean(session, NAME_CAP);
        if !crate::archive::is_canonical_uuid(id)
            || session.is_empty()
            || !seen.insert(id.to_owned())
        {
            read.skipped += 1;
            continue;
        }
        read.rows.push(Row {
            ts: event.ts,
            id: id.to_owned(),
            what,
            session,
            actor: actor_word(&event.actor),
            note: clean(event.summary.as_deref().unwrap_or_default(), TEXT_CAP),
        });
    }
    read
}

/// Which rows a chat shows. The first read of a chat shows its backlog: the
/// newest [`BACKLOG_ROWS`] of the rows no older than [`BACKLOG_SECS`], chosen
/// row by row, never by where the first old row sits (a writer stamps its
/// time before it waits for the journal's lock, so an old stamp can sit among
/// recent rows). Every read after it shows the backlog and every row recorded
/// since, however many arrived between two reads. The cursor holds the `ref`
/// of the newest row at the first read, so a trim of the journal's head cannot
/// move it; a backlog row the bound has since evicted is gone from the view.
#[derive(Debug, Clone, Default)]
pub struct Cursor {
    opened: bool,
    backlog: Vec<String>,
    after: Option<String>,
}

impl Cursor {
    /// The rows of `rows` this read shows, in journal order.
    pub fn select<'a>(&mut self, rows: &'a [Row], now: Timestamp) -> Vec<&'a Row> {
        if !self.opened {
            self.opened = true;
            let mut shown: Vec<&Row> = rows
                .iter()
                .rev()
                .filter(|row| now.epoch().saturating_sub(row.ts.epoch()) <= BACKLOG_SECS)
                .take(BACKLOG_ROWS)
                .collect();
            shown.reverse();
            self.backlog = shown.iter().map(|row| row.id.clone()).collect();
            self.after = rows.last().map(|row| row.id.clone());
            return shown;
        }
        // A cursor row the journal no longer holds was trimmed away: every row
        // left was recorded after it.
        let from = self
            .after
            .as_ref()
            .and_then(|after| rows.iter().position(|row| row.id == *after))
            .map_or(0, |at| at + 1);
        rows.iter()
            .enumerate()
            .filter(|(at, row)| *at >= from || self.backlog.contains(&row.id))
            .map(|(_, row)| row)
            .collect()
    }
}

/// A chat's feed: the cursor, and the last read kept while the journal's
/// identity stays the same, so a refresh of an unchanged journal is one stat.
#[derive(Default)]
pub struct Feed {
    cursor: Cursor,
    cached: Option<(Option<crate::store::Identity>, Read)>,
}

impl Feed {
    /// The rows to show now, and the read they come from.
    pub fn pick(&mut self, root: &Path, now: Timestamp) -> (Vec<Row>, usize, bool) {
        let identity = crate::store::body_identity(&root.join(DIR), crate::store::EVENTS);
        let stale = match &self.cached {
            Some((held, _)) => *held != identity || identity.is_none(),
            None => true,
        };
        if stale {
            self.cached = Some((identity, read(root)));
        }
        let Some((_, read)) = &self.cached else {
            return (Vec::new(), 0, false);
        };
        let shown = self
            .cursor
            .select(&read.rows, now)
            .into_iter()
            .cloned()
            .collect();
        (shown, read.skipped, read.unreadable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(n: usize, age: i64, now: Timestamp) -> Row {
        Row {
            ts: Timestamp::from_epoch(now.epoch() - age),
            id: format!("00000000-0000-4000-8000-{n:012}"),
            what: What::Stopped,
            session: "alpha".to_owned(),
            actor: "owner".to_owned(),
            note: String::new(),
        }
    }

    #[test]
    fn actor_words_name_the_roles_and_pass_a_seat_through() {
        let words = [
            ("human", "owner"),
            ("ae:launch", "owner"),
            ("ae/orchestrator", "orchestrator"),
            ("ae:reboot", "reboot"),
            ("watchdog", "watchdog"),
            ("lead", "lead"),
            ("slot:worker.0", "slot:worker.0"),
            ("", "unknown"),
            ("a\u{1b}[31mb\nc", "a [31mb c"),
        ];
        for (journal, shown) in words {
            assert_eq!(actor_word(journal), shown, "{journal:?}");
        }
        assert_eq!(actor_word(&"x".repeat(500)).chars().count(), ACTOR_CAP);
    }

    #[test]
    fn clean_folds_to_one_line_and_caps_with_an_ellipsis() {
        assert_eq!(clean("  a\tb\n\nc\r\n ", 50), "a b c");
        assert_eq!(clean("a\u{202e}b", 50), "a b");
        let long = format!("Check\tthe\nledger {}", "é".repeat(2_000));
        let cut = clean(&long, TEXT_CAP);
        assert_eq!(cut.chars().count(), TEXT_CAP);
        assert!(
            cut.starts_with("Check the ledger é") && cut.ends_with('…'),
            "{cut}"
        );
        assert_eq!(clean("", 5), "");
        assert_eq!(clean("abcde", 5), "abcde");
        assert_eq!(clean("abcdef", 5), "abcd…");
    }

    #[test]
    fn the_parked_excerpt_is_the_latest_parking_memo_without_its_lead() {
        let memo = concat!(
            "2026-11-21T07:00:00Z\tlead\tparking\tresume here: first idea\n",
            "2026-11-21T07:10:00Z\tlead\tdecision\tother\n",
            "2026-11-21T07:20:00Z\tlead\tparking\tResume HERE:  compare the   two ledgers\n",
        );
        assert_eq!(
            parked(memo.as_bytes()).as_deref(),
            Some("compare the two ledgers")
        );
        assert_eq!(parked(b"2026-11-21T07:10:00Z\tlead\tdecision\tx\n"), None);
        assert_eq!(parked(b""), None);
        assert_eq!(
            parked(b"2026-11-21T07:10:00Z\tlead\tparking\tno lead here\n").as_deref(),
            Some("no lead here")
        );
    }

    #[test]
    fn a_line_names_its_reason_only_when_there_is_one() {
        let now = Timestamp::from_epoch(1_800_000_000);
        let mut stopped = row(1, 0, now);
        assert_eq!(stopped.words(), "alpha stopped by owner");
        stopped.note = "pause".to_owned();
        assert_eq!(stopped.words(), "alpha stopped by owner — pause");
        stopped.what = What::Ended;
        assert_eq!(stopped.words(), "alpha ended by owner — pause");
        stopped.what = What::Resumed;
        assert_eq!(stopped.words(), "alpha resumed by owner — parked: pause");
        stopped.what = What::Renamed;
        assert_eq!(stopped.words(), "alpha renamed (was pause) by owner");
        stopped.note.clear();
        assert_eq!(stopped.words(), "alpha renamed by owner");
        stopped.what = What::Created;
        assert_eq!(stopped.words(), "alpha created by owner");
    }

    fn parsed(lines: &[String]) -> Vec<Event> {
        lines
            .iter()
            .filter_map(|line| Event::parse_line(line).ok())
            .collect()
    }

    fn json(action: &str, target: &str, reference: &str, actor: &str, summary: &str) -> String {
        format!(
            "{{\"ts\":\"2026-11-21T07:00:00Z\",\"actor\":\"{actor}\",\"action\":\"{action}\",\"target\":\"{target}\",\"ref\":\"{reference}\",\"summary\":\"{summary}\"}}"
        )
    }

    // The journal is hostile input: a row of ours with a bad ref, a bad
    // session name or a repeated ref is counted and never drawn; a row of
    // another action is not ours; control bytes and an over-long reason come
    // out clean and capped.
    #[test]
    fn a_hostile_journal_yields_only_whole_unique_rows() {
        let ok = "11111111-1111-4111-8111-111111111111";
        let long = "z".repeat(5_000);
        let lines = vec![
            json("fleet-stopped", "alpha", ok, "human", "fine"),
            json("fleet-stopped", "alpha", ok, "human", "same ref twice"),
            json("fleet-stopped", "alpha", "not-a-uuid", "human", "bad ref"),
            // A name from before the name grammar is drawn clean, never looked up.
            json(
                "fleet-stopped",
                "../../etc",
                "22222222-2222-4222-8222-222222222222",
                "human",
                "legacy name",
            ),
            json(
                "fleet-ended",
                "old.one\\n\\u001b[2J",
                "66666666-6666-4666-8666-666666666666",
                "human",
                "",
            ),
            json(
                "fleet-ended",
                "  ",
                "77777777-7777-4777-8777-777777777777",
                "human",
                "no name left",
            ),
            json(
                "fleet-stopped",
                "beta",
                "33333333-3333-4333-8333-333333333333",
                "human",
                "\\u001b[2Jx\\ny",
            ),
            json(
                "fleet-ended",
                "gamma",
                "44444444-4444-4444-8444-444444444444",
                "se\\u0007at",
                &long,
            ),
            json(
                "reseat",
                "alpha",
                "55555555-5555-4555-8555-555555555555",
                "watchdog",
                "not ours",
            ),
            "{\"ts\":\"2026-11-21T07:00:00Z\",\"actor\":\"h\",\"action\":\"fleet-created\"}"
                .to_owned(),
        ];
        let read = rows_of(&parsed(&lines), 2);
        let seen: Vec<(&str, &str)> = read
            .rows
            .iter()
            .map(|row| (row.session.as_str(), row.actor.as_str()))
            .collect();
        assert_eq!(
            seen,
            [
                ("alpha", "owner"),
                ("../../etc", "owner"),
                ("old.one [2J", "owner"),
                ("beta", "owner"),
                ("gamma", "se at")
            ]
        );
        // two lines the reader already skipped, plus: repeated ref, bad ref,
        // a name nothing is left of, and the fleet row with no target or ref.
        assert_eq!(read.skipped, 2 + 4);
        let beta = &read.rows[3];
        assert!(!beta.note.chars().any(char::is_control), "{:?}", beta.note);
        assert_eq!(read.rows[4].note.chars().count(), TEXT_CAP);
        assert!(!read.unreadable);
    }

    #[test]
    fn the_first_read_shows_a_bounded_backlog_and_every_read_after_shows_all_new_rows() {
        let now = Timestamp::from_epoch(1_800_000_000);
        let mut rows: Vec<Row> = (0..14)
            .map(|n| row(n, 100 - i64::try_from(n).expect("small"), now))
            .collect();
        let mut cursor = Cursor::default();
        let first = cursor.select(&rows, now);
        assert_eq!(first.len(), BACKLOG_ROWS);
        assert_eq!(first[0].id, rows[4].id);
        assert_eq!(first[BACKLOG_ROWS - 1].id, rows[13].id);
        // Fifteen new rows between two reads: all of them, none of the backlog's
        // excluded rows.
        rows.extend((14..29).map(|n| row(n, 0, now)));
        let next = cursor.select(&rows, now);
        assert_eq!(next.len(), 29 - 4);
        assert_eq!(next[0].id, rows[4].id);
        // A trim that drops the head, the cursor row included, shows what is left.
        let trimmed = rows.split_off(10);
        assert_eq!(cursor.select(&trimmed, now).len(), trimmed.len());
    }

    // A writer stamps its time before it waits for the journal's lock, so an
    // old stamp can sit anywhere among recent rows: the backlog is every
    // eligible row, newest ten, whatever order the ages come in.
    #[test]
    fn the_backlog_is_chosen_row_by_row_whatever_order_the_ages_come_in() {
        let now = Timestamp::from_epoch(1_800_000_000);
        let ages = [
            5,
            BACKLOG_SECS + 50,
            6,
            BACKLOG_SECS + 70,
            7,
            8,
            BACKLOG_SECS + 90,
            9,
            10,
            11,
            12,
            BACKLOG_SECS + 1,
            13,
            14,
            15,
            BACKLOG_SECS + 2,
        ];
        let mut rows: Vec<Row> = ages
            .iter()
            .enumerate()
            .map(|(n, age)| row(n, *age, now))
            .collect();
        let eligible: Vec<String> = rows
            .iter()
            .zip(ages)
            .filter(|(_, age)| *age <= BACKLOG_SECS)
            .map(|(row, _)| row.id.clone())
            .collect();
        assert!(eligible.len() > BACKLOG_ROWS, "{}", eligible.len());
        let mut cursor = Cursor::default();
        let ids = |shown: Vec<&Row>| shown.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
        let first = ids(cursor.select(&rows, now));
        assert_eq!(first, eligible[eligible.len() - BACKLOG_ROWS..]);
        // A late old row appended after the open is new, whatever its stamp;
        // the backlog stays, in journal order.
        rows.push(row(40, BACKLOG_SECS + 500, now));
        let next = ids(cursor.select(&rows, now));
        assert_eq!(next.len(), BACKLOG_ROWS + 1);
        assert_eq!(next[..BACKLOG_ROWS], first[..]);
        assert_eq!(next[BACKLOG_ROWS], rows[16].id);
    }

    #[test]
    fn the_backlog_leaves_out_rows_older_than_a_day_and_an_empty_start_shows_all_later() {
        let now = Timestamp::from_epoch(1_800_000_000);
        let rows = vec![
            row(1, BACKLOG_SECS + 1, now),
            row(2, BACKLOG_SECS, now),
            row(3, 5, now),
        ];
        let mut cursor = Cursor::default();
        assert_eq!(
            cursor
                .select(&rows, now)
                .iter()
                .map(|r| r.id.clone())
                .collect::<Vec<_>>(),
            [rows[1].id.clone(), rows[2].id.clone()]
        );
        let mut empty = Cursor::default();
        assert!(empty.select(&[], now).is_empty());
        assert_eq!(empty.select(&rows, now).len(), 3);
        // Every row too old: nothing shows now, and only later rows after.
        let old = vec![row(1, BACKLOG_SECS + 10, now)];
        let mut stale = Cursor::default();
        assert!(stale.select(&old, now).is_empty());
        let mut grown = old.clone();
        grown.push(row(2, 0, now));
        assert_eq!(stale.select(&grown, now).len(), 1);
    }
}
