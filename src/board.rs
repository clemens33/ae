//! The ae message board — the filtered cross-fleet record, derived on read.
//!
//! One schema, thin per-harness readers. Each reader is a pure function over
//! the transcript bytes it is handed; locating the file and opening it belong
//! to a later slice. The view is DERIVED: [`collect`] is the one place rows
//! from many files merge, sort and dedup. Nothing writes a board file.
//!
//! Row identity is the SOURCE RECORD — (`file`, `offset`) — never the actor
//! or the body: two seats may type the same words in the same microsecond, and
//! that is two rows, not one.

pub mod agy;
pub mod claude;
pub mod codex;
pub(crate) mod follow;
pub mod grok;
pub mod muse;

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufRead, BufReader, Read as _, Seek as _};
use std::path::{Path, PathBuf};

use crate::json::Value;
use crate::quota::{Bounded, Budget};
use crate::tool::{ToolKind, UsageSource};

/// Who speaks in a board row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A genuine human turn: bare of any ae marker.
    Human,
    /// A model turn from the harness's own transcript, read only with
    /// `--assistant`: text parts joined, thinking and tool calls never read.
    Assistant,
}

/// One board row: one turn — a human's, or with `--assistant` a model's text
/// reply — from one harness transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Epoch micros, in the source store's native precision.
    pub ts: i64,
    /// `session:seat`, from the meta roster.
    pub actor: String,
    /// Who speaks.
    pub role: Role,
    /// The turn's body, trimmed, reminders stripped.
    pub body: String,
    /// Which harness wrote the transcript.
    pub source: ToolKind,
    /// Source file identity, as the caller names it.
    pub file: String,
    /// Byte offset of the record's first byte in that file.
    pub offset: u64,
}

/// A per-seat coverage row: the board is INCOMPLETE for this seat, and says
/// why. A partial board must never look whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// `session:seat`, from the meta roster.
    pub actor: String,
    /// Why coverage is incomplete (`torn last record`, a tripped line cap, …).
    pub reason: String,
}

/// THE one sort/dedup point: stable sort by (`ts`, `file`, `offset`), then
/// drop an identical (`file`, `offset`) read twice. Same-timestamp rows from
/// different files all survive; so do two seats' identical words.
#[must_use]
pub fn collect(mut rows: Vec<Row>) -> Vec<Row> {
    rows.sort_by(|left, right| {
        (left.ts, &left.file, left.offset).cmp(&(right.ts, &right.file, right.offset))
    });
    rows.dedup_by(|later, first| first.file == later.file && first.offset == later.offset);
    rows
}

/// A line longer than this is hostile, not a record: the door reports its
/// true length without buffering it, and the reader covers it, never parses it.
pub(crate) const LINE_CAP: usize = 1024 * 1024;

/// What `ae board` prints when its argv does not parse.
pub const USAGE: &str = "Usage: ae board [session…] [--since <ts>] [--json] [--follow] [--lines <n>] [--assistant]\n\n  session       read this session (repeatable; default is every running session)\n  --since <ts>  keep rows at or after <ts>, strict YYYY-MM-DDTHH:MM:SSZ\n  --json        NDJSON: one scope line, coverage lines, then row lines\n  --follow      keep printing new rows and coverage changes every 5 s until\n                interrupted; the selection is fixed at start (Ctrl-C to stop)\n  --lines <n>   text only: clip each body to its first <n> lines; the dropped\n                remainder prints one `… +k lines` marker\n  --assistant   add the model's replies (text only; off by default)\n";

const LINES_TEXT_ONLY: &str = "--lines is text-only";

/// A parsed `ae board` argv.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    /// Explicit sessions, caller order, deduplicated by rejection.
    pub sessions: Vec<String>,
    /// `--since`, in epoch micros — rows at or after this survive.
    pub since_micros: Option<i64>,
    /// `--json`: NDJSON instead of text.
    pub json: bool,
    /// `--follow`: print the one-shot board, then keep printing new rows.
    pub follow: bool,
    /// `--lines <n>`: text rows clip each body to its first `n` lines.
    pub lines: Option<usize>,
    /// `--assistant`: also read the model's replies (text only).
    pub assistant: bool,
}

/// The argv did not parse; the offending token, when there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage(pub Option<String>);

impl Usage {
    /// The stderr text, offending token first when one was named.
    #[must_use]
    pub fn render(&self) -> String {
        match &self.0 {
            Some(token) => format!("ae board: unexpected {token}\n{USAGE}"),
            None => USAGE.to_owned(),
        }
    }
}

/// Read `tail` — everything after the word `board`.
///
/// ```
/// use ae::board::{Args, parse};
///
/// let words = |items: &[&str]| items.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
/// assert_eq!(parse(&[]), Ok(Args::default()));
/// assert_eq!(
///     parse(&words(&["aedev", "--json"])),
///     Ok(Args { sessions: vec!["aedev".to_owned()], since_micros: None, json: true, follow: false, lines: None, assistant: false })
/// );
/// assert!(parse(&words(&["--follow"])).is_ok_and(|args| args.follow));
/// assert_eq!(parse(&words(&["--lines", "3"])).map(|args| args.lines), Ok(Some(3)));
/// assert!(parse(&words(&["--assistant"])).is_ok_and(|args| args.assistant));
/// assert!(parse(&words(&["--frobnicate"])).is_err());
/// ```
///
/// # Errors
///
/// [`Usage`] for an unknown flag, a repeated session name, `--since` without a
/// strict `YYYY-MM-DDTHH:MM:SSZ` value, `--lines` without a positive decimal
/// value or combined with `--json`, or a value missing entirely.
pub fn parse(tail: &[String]) -> Result<Args, Usage> {
    let mut args = Args::default();
    let mut index = 0;
    while let Some(token) = tail.get(index) {
        match token.as_str() {
            "--json" => args.json = true,
            "--follow" => args.follow = true,
            "--assistant" => args.assistant = true,
            "--lines" => {
                let value = tail.get(index + 1).ok_or(Usage(Some(token.clone())))?;
                if value.as_str() == "--json" || args.json {
                    return Err(Usage(Some(LINES_TEXT_ONLY.to_owned())));
                }
                let count: usize = value.parse().map_err(|_| Usage(Some(value.clone())))?;
                if count == 0 {
                    return Err(Usage(Some(value.clone())));
                }
                args.lines = Some(count);
                index += 1;
            }
            "--since" => {
                let value = tail.get(index + 1).ok_or(Usage(Some(token.clone())))?;
                let micros = crate::time::Timestamp::parse(value)
                    .map(|moment| moment.epoch().saturating_mul(1_000_000))
                    .ok_or(Usage(Some(value.clone())))?;
                args.since_micros = Some(micros);
                index += 1;
            }
            // A `-`/`--` token nothing above defines is a usage error, exactly
            // as a `brief` tail treats one. A bare word is a session name.
            flag if flag.starts_with('-') => return Err(Usage(Some(token.clone()))),
            name => {
                if args.sessions.iter().any(|known| known == name) {
                    return Err(Usage(Some(token.clone())));
                }
                args.sessions.push(name.to_owned());
            }
        }
        index += 1;
    }
    if args.json && args.lines.is_some() {
        return Err(Usage(Some(LINES_TEXT_ONLY.to_owned())));
    }
    Ok(args)
}

/// One streamed transcript line: path in, lines out — the door carries no
/// harness knowledge, so every later reader reuses it.
#[derive(Debug)]
pub struct Line {
    /// Byte offset of the line's first byte in the file.
    pub(crate) offset: u64,
    /// The line's body, or its true length when it tripped the cap.
    pub(crate) body: LineBody,
}

/// A line's body: bytes the reader may parse, or a length it must cover.
#[derive(Debug)]
pub enum LineBody {
    /// A newline-terminated line, newline excluded, within the cap.
    Full(Vec<u8>),
    /// A line past [`LINE_CAP`]: its TRUE length in bytes, newline excluded.
    /// The bytes were discarded while reading, never buffered.
    Overlong(usize),
}

/// One transcript, streamed: its lines plus whether the tail was torn.
#[derive(Debug)]
pub struct Streamed {
    /// Every newline-terminated line, in file order with its byte offset.
    pub(crate) lines: Vec<Line>,
    /// Trailing bytes without a newline were seen and NOT trusted.
    pub(crate) torn: bool,
    /// Absolute position after the last newline-terminated line — the
    /// splitter's base when none — so a follow's next read starts there.
    pub(crate) committed: u64,
    /// The seat's harness conversation id, when the caller bound one. A store
    /// that interleaves conversations (agy) filters on it; every other reader
    /// ignores it. The splitter stays pure, so this is bound by the caller.
    pub(crate) seat_id: String,
    /// `--assistant`: read the model's replies too, text parts only. False
    /// until the caller binds it, so a reader with the flag off takes exactly
    /// the human-only path it took before the flag existed.
    pub(crate) assistant: bool,
}

impl Streamed {
    /// Bind the seat's harness conversation id to this read. ONE store — agy's
    /// history — carries several seats' turns, so its reader matches each
    /// record against this id and nothing else.
    #[must_use]
    pub fn for_seat(mut self, seat_id: &str) -> Self {
        seat_id.clone_into(&mut self.seat_id);
        self
    }

    /// Bind `--assistant` to this read: readers emit the model's replies
    /// beside the human turns, text parts only.
    #[must_use]
    pub fn with_assistant(mut self, assistant: bool) -> Self {
        self.assistant = assistant;
        self
    }
}

/// THE line splitter: bytes in, lines out, no I/O. The door feeds it buffer
/// chunks and the whole-bytes reader feeds it one slice — one splitter, so
/// the fuzz target covers the code hostile transcripts hit.
#[derive(Debug, Default)]
pub struct Splitter {
    lines: Vec<Line>,
    line: Vec<u8>,
    start: u64,
    cursor: u64,
    overlong: bool,
    line_len: usize,
}

impl Splitter {
    /// An empty splitter at byte zero.
    #[must_use]
    pub fn new() -> Self {
        Self::at(0)
    }

    /// An empty splitter whose line offsets start at `base`: a follow streams
    /// only a tail and every offset stays ABSOLUTE in the file.
    #[must_use]
    pub fn at(base: u64) -> Self {
        Self {
            start: base,
            cursor: base,
            ..Self::default()
        }
    }

    /// Feed one chunk: newlines end lines, the cap trips mid-chunk, and a
    /// partial line carries over to the next feed.
    pub fn feed(&mut self, chunk: &[u8]) {
        let mut rest = chunk;
        while !rest.is_empty() {
            let Some(at) = rest.iter().position(|byte| *byte == b'\n') else {
                self.push_body(rest);
                self.cursor += rest.len() as u64;
                break;
            };
            self.push_body(&rest[..at]);
            self.cursor += (at + 1) as u64;
            self.end_line();
            rest = &rest[at + 1..];
        }
    }

    /// The streamed transcript: every newline-terminated line, plus whether a
    /// torn tail was seen and not trusted, plus the offset after the last
    /// complete line. A fresh stream carries no seat id; [`Streamed::for_seat`]
    /// binds one, [`Streamed::with_assistant`] the reply flag.
    #[must_use]
    pub fn finish(self) -> Streamed {
        Streamed {
            lines: self.lines,
            torn: self.cursor != self.start,
            committed: self.start,
            seat_id: String::new(),
            assistant: false,
        }
    }

    fn push_body(&mut self, body: &[u8]) {
        if !self.overlong {
            if self.line.len() + body.len() > LINE_CAP {
                self.line.clear();
                self.overlong = true;
            } else {
                self.line.extend_from_slice(body);
            }
        }
        self.line_len += body.len();
    }

    fn end_line(&mut self) {
        let body = if self.overlong {
            LineBody::Overlong(self.line_len)
        } else {
            LineBody::Full(std::mem::take(&mut self.line))
        };
        self.lines.push(Line {
            offset: self.start,
            body,
        });
        self.start = self.cursor;
        self.overlong = false;
        self.line_len = 0;
    }
}

/// Why the streaming door refused a located transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DoorError {
    /// The file could not be opened or read.
    Unreadable,
    /// The opened file is not the located one, or shrank under the read.
    Changed,
    /// The opened file is not a regular file.
    NotRegular,
}

/// THE streaming door: open a located transcript and feed it to the splitter.
///
/// The caller located the file and hands over its lstat metadata; this
/// function proves the opened file IS that file — regular, same dev+inode,
/// length not shrunk — reads at most the located length, and feeds buffer
/// chunks to the ONE [`Splitter`]. `from` is the absolute byte offset the
/// caller already committed to (`0` in the one-shot): the read starts there
/// and every line offset stays absolute. No splitting logic and no harness
/// knowledge inside: open, prove, feed.
pub(crate) fn stream_transcript(
    path: &Path,
    expected: &std::fs::Metadata,
    from: u64,
) -> Result<Streamed, DoorError> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: streams only the lstat-checked board transcript"
    )]
    let mut file = File::open(path).map_err(|_| DoorError::Unreadable)?;
    let opened = file.metadata().map_err(|_| DoorError::Unreadable)?;
    if !opened.file_type().is_file() {
        return Err(DoorError::NotRegular);
    }
    if opened.len() < expected.len() || expected.len() < from || !same_file(expected, &opened) {
        return Err(DoorError::Changed);
    }
    file.seek(std::io::SeekFrom::Start(from))
        .map_err(|_| DoorError::Unreadable)?;
    let mut reader = BufReader::new(file.take(expected.len() - from));
    let mut splitter = Splitter::at(from);
    loop {
        let chunk = reader.fill_buf().map_err(|_| DoorError::Unreadable)?;
        if chunk.is_empty() {
            break;
        }
        let consumed = chunk.len();
        splitter.feed(chunk);
        reader.consume(consumed);
    }
    if reader.into_inner().limit() != 0 {
        return Err(DoorError::Changed);
    }
    Ok(splitter.finish())
}

#[cfg(unix)]
fn same_file(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_file(_: &std::fs::Metadata, _: &std::fs::Metadata) -> bool {
    true
}

/// Filesystem roots and already-selected sessions.
pub struct Inputs<'a> {
    /// The effective home, for legacy stores no meta row pins down.
    pub home: Option<&'a Path>,
    /// The sessions to read, caller order.
    pub sessions: &'a [crate::usage::SessionInput],
    /// `--assistant`: read the model's replies (text only) beside the humans.
    pub assistant: bool,
}

/// One seat's read facts, exactly as a read observed them: the follow seed, so
/// the first pass's read and the offsets it binds are ONE read, never two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeatSeed {
    /// `session:seat`, from the meta roster.
    pub(crate) actor: String,
    /// The located file's dev+inode.
    pub(crate) identity: (u64, u64),
    /// The located file's mtime, when the OS reports one.
    pub(crate) mtime: Option<std::time::SystemTime>,
    /// Absolute position after the last complete line of the read.
    pub(crate) committed: u64,
}

/// One board: every row read plus every seat that could not be read fully.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Observation {
    /// Board turns — human, and with `--assistant` the model's text replies —
    /// oldest first through [`collect`], `--since` applied.
    pub rows: Vec<Row>,
    /// One row per seat the board could not read fully, caller order.
    pub coverage: Vec<Coverage>,
    /// What each successfully streamed seat's read observed; the follow seeds
    /// itself from these and a batch carries none.
    pub(crate) seeds: Vec<SeatSeed>,
}

/// Read every Claude, Codex, Grok, Muse and Antigravity seat of the handed-in
/// sessions. A seat that cannot be read — unknown tool, unlocated store,
/// unreadable transcript — becomes a [`Coverage`], never a silent subset.
#[must_use]
pub fn observe(inputs: &Inputs<'_>, since_micros: Option<i64>) -> Observation {
    let mut rows = Vec::new();
    let mut coverage = Vec::new();
    let mut seeds = Vec::new();
    for session in inputs.sessions {
        let Ok(meta) = crate::session::read_meta(&session.path) else {
            coverage.push(Coverage {
                actor: format!("{}:?", session.name),
                reason: "session meta unreadable".to_owned(),
            });
            continue;
        };
        for entry in meta.roster() {
            observe_seat(
                &session.name,
                entry,
                inputs.home,
                inputs.assistant,
                &mut rows,
                &mut coverage,
                &mut seeds,
            );
        }
    }
    let mut rows = collect(rows);
    if let Some(since) = since_micros {
        rows.retain(|row| row.ts >= since);
    }
    Observation {
        rows,
        coverage,
        seeds,
    }
}

/// Read one roster seat: locate it through THE one locator, stream it through
/// the door, read it with the one dispatch for its tool. Every refusal is a
/// coverage row, never a silent subset.
fn observe_seat(
    session: &str,
    entry: &crate::meta::RosterEntry,
    home: Option<&Path>,
    assistant: bool,
    rows: &mut Vec<Row>,
    coverage: &mut Vec<Coverage>,
    seeds: &mut Vec<SeatSeed>,
) {
    let actor = format!("{}:{}", session, entry.name);
    let tool = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or(""));
    let (path, metadata) = match locate_seat(entry, home) {
        Ok(located) => located,
        Err(reason) => {
            coverage.push(Coverage { actor, reason });
            return;
        }
    };
    let streamed = match stream_transcript(&path, &metadata, 0) {
        Ok(streamed) => streamed
            .for_seat(entry.harness_session.as_deref().unwrap_or_default())
            .with_assistant(assistant),
        Err(failure) => {
            coverage.push(Coverage {
                actor,
                reason: door_reason(failure).to_owned(),
            });
            return;
        }
    };
    let file = file_identity(&path, &metadata);
    seeds.push(seed(&actor, &metadata, &streamed));
    let (mut seat_rows, mut seat_coverage) = reader_for(tool)(&streamed, &actor, &file, tool);
    rows.append(&mut seat_rows);
    coverage.append(&mut seat_coverage);
}
/// One harness reader: the door's stream in, rows and coverage out.
pub(crate) type Reader = fn(&Streamed, &str, &str, ToolKind) -> (Vec<Row>, Vec<Coverage>);

/// The reader for one transcript stream, chosen by its tool: ONE dispatch for
/// the one-shot read and the follow's reassembled batches alike.
pub(crate) fn reader_for(source: ToolKind) -> Reader {
    if matches!(source.adapter().usage.source, UsageSource::CodexRollout) {
        return codex::read_stream;
    }
    // String dispatch, as `unsupported_reason` does: literals live in tool.rs.
    match source.adapter().name {
        "agy" => agy::read_stream,
        "grok" => grok::read_stream,
        "muse" => muse::read_stream,
        _ => claude::read_stream,
    }
}

/// ONE lstat for every board locator: the candidate proved a regular file
/// without following a link. A missing candidate (or a non-directory on its
/// path) is `Ok(None)` — the locator keeps looking; a symlink or non-file
/// refuses; any other failure is unreadable.
fn lstat_regular(candidate: &Path) -> Result<Option<std::fs::Metadata>, String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: board lstat proves the transcript candidate a regular file, never a symlink"
    )]
    let metadata = match std::fs::symlink_metadata(candidate) {
        Ok(metadata) => metadata,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(_) => return Err("transcript unreadable".to_owned()),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err("transcript is not a regular file".to_owned());
    }
    Ok(Some(metadata))
}

/// THE seat locator, one owner for the one-shot read and every follow poll:
/// resolve the harness's own store, then its current transcript. The returned
/// reason IS the coverage row's text.
fn locate_seat(
    entry: &crate::meta::RosterEntry,
    home: Option<&Path>,
) -> Result<(PathBuf, std::fs::Metadata), String> {
    let tool = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or(""));
    let source = tool.adapter().usage.source;
    if matches!(source, UsageSource::CodexRollout) {
        let Some(id) = crate::usage::valid_id(entry.harness_session.as_deref()) else {
            return Err("invalid or missing conversation id".to_owned());
        };
        let root = crate::usage::source_for(entry, UsageSource::CodexRollout, home)?;
        let mut budget = Budget::new();
        return match crate::quota::find_codex_rollout(&root, &id, &mut budget) {
            Ok(Bounded::Ready(Some(rollout))) => {
                Ok((rollout.path().to_owned(), rollout.metadata().clone()))
            }
            Ok(Bounded::Ready(None)) => Err("rollout not found".to_owned()),
            Ok(Bounded::Truncated) => Err("rollout scan truncated".to_owned()),
            Err(error) => Err(error.to_string()),
        };
    }
    if tool.adapter().name == "grok" {
        let Some(id) = crate::usage::valid_id(entry.harness_session.as_deref()) else {
            return Err("invalid or missing conversation id".to_owned());
        };
        let Some(home) = home else {
            return Err("legacy config home unavailable".to_owned());
        };
        let Some(dir) = tool.adapter().quota.default_home else {
            return Err("unsupported tool".to_owned());
        };
        let mut budget = Budget::new();
        return match locate_grok_updates(&home.join(dir).join("sessions"), &id, &mut budget)? {
            Some(found) => Ok((found.path, found.metadata)),
            None => Err("transcript not found".to_owned()),
        };
    }
    if tool.adapter().name == "muse" {
        let id = entry.harness_session.as_deref().unwrap_or_default();
        if !crate::session_launch::capture::is_lowercase_uuid(id) {
            return Err("invalid or missing conversation id".to_owned());
        }
        let Some(home) = home else {
            return Err("legacy config home unavailable".to_owned());
        };
        let mut budget = Budget::new();
        let Some(path) =
            crate::session_launch::capture::find_muse_session_file(home, id, &mut budget)?
        else {
            return Err("transcript not found".to_owned());
        };
        return match lstat_regular(&path)? {
            Some(metadata) => Ok((path, metadata)),
            None => Err("transcript not found".to_owned()),
        };
    }
    if tool.adapter().name == "agy" {
        // ONE history file per home carries every agy conversation, so the
        // reader — not the locator — separates this seat's turns. The home
        // names the store; the literal lives in tool.rs's adapter row alone.
        let id = entry.harness_session.as_deref().unwrap_or_default();
        if !crate::session_launch::capture::is_lowercase_uuid(id) {
            return Err("invalid or missing conversation id".to_owned());
        }
        let Some(home) = home else {
            return Err("legacy config home unavailable".to_owned());
        };
        let Some(dir) = tool.adapter().quota.default_home else {
            return Err("unsupported tool".to_owned());
        };
        let path = home.join(dir).join("history.jsonl");
        return match lstat_regular(&path)? {
            Some(metadata) => Ok((path, metadata)),
            None => Err("transcript not found".to_owned()),
        };
    }
    if !matches!(source, UsageSource::ClaudeTranscripts) {
        return Err(unsupported_reason(tool).to_owned());
    }
    let Some(id) = crate::usage::valid_id(entry.harness_session.as_deref()) else {
        return Err("invalid or missing conversation id".to_owned());
    };
    let store = crate::usage::source_for(entry, UsageSource::ClaudeTranscripts, home)?;
    let mut budget = Budget::new();
    let located = crate::usage::locate_claude_parent(&store, &id, &mut budget)
        .map_err(|failure| locate_reason(&failure))?;
    match located {
        Some(transcript) => Ok((transcript.path, transcript.metadata)),
        None => Err("transcript not found".to_owned()),
    }
}

/// One follow poll: re-resolve every selected session's roster, re-locate every
/// seat, read the bytes the held offsets ask for through the ONE door, and step
/// the state. Selection itself is the caller's and is fixed for the follow.
pub(crate) fn follow_poll(inputs: &Inputs<'_>, follow: &mut follow::Follow) -> Observation {
    let mut snapshots = Vec::new();
    for session in inputs.sessions {
        let Ok(meta) = crate::session::read_meta(&session.path) else {
            snapshots.push(follow::Snapshot {
                actor: format!("{}:?", session.name),
                located: Err("session meta unreadable".to_owned()),
                streamed: None,
            });
            continue;
        };
        for entry in meta.roster() {
            snapshots.push(follow_seat(
                &session.name,
                entry,
                inputs.home,
                follow,
                inputs.assistant,
            ));
        }
    }
    follow.step(snapshots)
}

/// One seat's polled snapshot: locate, then read exactly the tail the held
/// offset asks for. A refusal is carried as the reason, never as a silent skip.
fn follow_seat(
    session: &str,
    entry: &crate::meta::RosterEntry,
    home: Option<&Path>,
    follow: &follow::Follow,
    assistant: bool,
) -> follow::Snapshot {
    let actor = format!("{}:{}", session, entry.name);
    let tool = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or(""));
    let (path, metadata) = match locate_seat(entry, home) {
        Ok(located) => located,
        Err(reason) => {
            return follow::Snapshot {
                actor,
                located: Err(reason),
                streamed: None,
            };
        }
    };
    let observed = follow::Located::of(&metadata);
    let streamed = match follow.plan(&actor, &observed) {
        follow::Plan::Hold => None,
        follow::Plan::Read(from) => Some(
            stream_transcript(&path, &metadata, from)
                .map_err(door_reason)
                .map(|streamed| {
                    streamed
                        .for_seat(entry.harness_session.as_deref().unwrap_or_default())
                        .with_assistant(assistant)
                }),
        ),
    };
    follow::Snapshot {
        actor,
        located: Ok(follow::Loaded {
            file: file_identity(&path, &metadata),
            source: tool,
            observed,
        }),
        streamed,
    }
}

/// Scan `<sessions root>/*/<uuid>/updates.jsonl` for one seat's conversation.
/// Only the candidate file is classified — a missing candidate (or a
/// non-directory on its path) skips; two hits refuse; a symlink refuses.
fn locate_grok_updates(
    root: &Path,
    id: &str,
    budget: &mut Budget,
) -> Result<Option<crate::usage::TranscriptFile>, String> {
    if !budget.claim_file() {
        return Err("transcript scan truncated".to_owned());
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: bounded board enumeration of the grok sessions root"
    )]
    let listing = match std::fs::read_dir(root) {
        Ok(listing) => listing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("transcript unreadable".to_owned()),
    };
    let mut found: Option<crate::usage::TranscriptFile> = None;
    for entry in listing {
        // One claim per probe bounds the scan: survivors reach it, failures abort.
        if !budget.claim_file() {
            return Err("transcript scan truncated".to_owned());
        }
        let entry = entry.map_err(|_| "directory entry unreadable".to_owned())?;
        let candidate = entry.path().join(id).join("updates.jsonl");
        let Some(metadata) = lstat_regular(&candidate)? else {
            continue;
        };
        if found.is_some() {
            return Err("conversation id is not unique".to_owned());
        }
        found = Some(crate::usage::TranscriptFile {
            path: candidate,
            metadata,
            modified: None,
        });
    }
    Ok(found)
}

/// The phase (or ruling) behind a harness the board cannot read yet.
fn unsupported_reason(tool: ToolKind) -> &'static str {
    // String dispatch, never `ToolKind::` arms: production tool literals live
    // in `src/tool.rs` alone (`per_tool_branches_live_only_in_the_adapter_rows`).
    match tool.adapter().name {
        "opencode" => "opencode: not read",
        "gemini" => "gemini: out of scope",
        _ => "unknown tool: out of scope",
    }
}

/// The locate failure, in usage's own vocabulary.
fn locate_reason(failure: &crate::usage::Coverage) -> String {
    match failure {
        crate::usage::Coverage::Unreadable(reason) => reason.clone(),
        crate::usage::Coverage::Truncated => "transcript scan truncated".to_owned(),
        crate::usage::Coverage::Unlocated => "transcript not located".to_owned(),
        crate::usage::Coverage::Unsupported => "transcript source unsupported".to_owned(),
        crate::usage::Coverage::Read => "transcript unreadable".to_owned(),
    }
}

/// The door failure, in the transcript vocabulary.
fn door_reason(failure: DoorError) -> &'static str {
    match failure {
        DoorError::Unreadable => "transcript unreadable",
        DoorError::Changed => "transcript changed during read",
        DoorError::NotRegular => "transcript is not a regular file",
    }
}

/// ONE coverage row for a seat's every over-cap line: the count plus the
/// largest true length. Both readers skip `Overlong` lines in their loop and
/// push this row after it — N huge lines name one row, never N.
pub(crate) fn overlong_coverage(streamed: &Streamed, actor: &str) -> Option<Coverage> {
    let mut count = 0;
    let mut largest = 0;
    for line in &streamed.lines {
        if let LineBody::Overlong(len) = &line.body {
            count += 1;
            largest = largest.max(*len);
        }
    }
    if count == 0 {
        return None;
    }
    let noun = if count == 1 {
        "line exceeds"
    } else {
        "lines exceed"
    };
    Some(Coverage {
        actor: actor.to_owned(),
        reason: format!("{count} {noun} the 1 MiB cap (largest {largest} bytes)"),
    })
}

/// The scope statement: line 1 of EVERY board, even an empty one.
const SCOPE_TEXT: &str = "scope: current conversations only (phase 1b) — a seat that resumed keeps only its current transcript";

/// Render the observation as text or NDJSON. Line 1 is ALWAYS the scope
/// statement; coverage rows precede body rows; JSON never carries a bare line.
/// `lines` clips text bodies only — NDJSON always carries the whole body.
#[must_use]
pub fn render(observation: &Observation, json: bool, lines: Option<usize>) -> String {
    let mut out = if json {
        Value::obj([
            ("kind", Value::str("scope")),
            ("scope", Value::str("current-conversations")),
            ("phase", Value::str("1b")),
        ])
        .render()
    } else {
        SCOPE_TEXT.to_owned()
    };
    out.push('\n');
    out.push_str(&render_batch(observation, json, lines));
    out
}

/// Render ONE board body — coverage rows, then body rows, no scope line. The
/// one-shot prepends the scope statement once; the follow appends one of these
/// per batch under it.
#[must_use]
pub(crate) fn render_batch(observation: &Observation, json: bool, lines: Option<usize>) -> String {
    if json {
        render_batch_json(observation)
    } else {
        render_batch_text(observation, lines)
    }
}

/// The indent every text body line wears, the clip marker included.
const BODY_INDENT: &str = "  ";

/// ONE text row renderer: the header, every body line indented, a blank line,
/// and — with a clip — one marker naming the dropped remainder. Both the
/// one-shot and every follow batch print rows through this function.
fn text_row(row: &Row, lines: Option<usize>) -> String {
    let mut out = format!("## {} {}", format_micros(row.ts), row.actor);
    if row.role == Role::Assistant {
        out.push_str(" · assistant");
    }
    out.push('\n');
    let total = row.body.lines().count();
    let shown = lines.map_or(total, |count| count.min(total));
    for line in row.body.lines().take(shown) {
        let _ = writeln!(out, "{BODY_INDENT}{line}");
    }
    if let Some(count) = lines
        && total > count
    {
        let _ = writeln!(out, "{BODY_INDENT}… +{} lines", total - count);
    }
    out.push('\n');
    out
}

fn render_batch_text(observation: &Observation, lines: Option<usize>) -> String {
    let mut out = String::new();
    for item in &observation.coverage {
        let _ = writeln!(out, "coverage incomplete: {} — {}", item.actor, item.reason);
    }
    for row in &observation.rows {
        out.push_str(&text_row(row, lines));
    }
    out
}

fn render_batch_json(observation: &Observation) -> String {
    let mut out = String::new();
    for item in &observation.coverage {
        let line = Value::obj([
            ("kind", Value::str("coverage")),
            ("actor", Value::str(&item.actor)),
            ("reason", Value::str(&item.reason)),
        ])
        .render();
        out.push_str(&line);
        out.push('\n');
    }
    for row in &observation.rows {
        let role = match row.role {
            Role::Human => "human",
            Role::Assistant => "assistant",
        };
        let line = Value::obj([
            ("kind", Value::str("row")),
            ("ts", Value::Num(row.ts)),
            ("actor", Value::str(&row.actor)),
            ("role", Value::str(role)),
            ("body", Value::str(&row.body)),
            ("source", Value::str(row.source.as_str())),
            ("file", Value::str(&row.file)),
            ("offset", json_u64(row.offset)),
        ])
        .render();
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Epoch micros as ISO 8601 UTC at micro precision.
fn format_micros(micros: i64) -> String {
    let secs = micros.div_euclid(1_000_000);
    let fraction = micros.rem_euclid(1_000_000);
    let base = crate::time::Timestamp::from_epoch(secs).to_string();
    let date = base.strip_suffix('Z').unwrap_or(&base);
    format!("{date}.{fraction:06}Z")
}

/// The located file's dev+inode where the OS has them — the follow's identity,
/// and the `file` naming's second half. `(0, 0)` where the OS has neither.
#[cfg(unix)]
pub(crate) fn identity_of(metadata: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt as _;
    (metadata.dev(), metadata.ino())
}

#[cfg(not(unix))]
pub(crate) fn identity_of(_: &std::fs::Metadata) -> (u64, u64) {
    (0, 0)
}

/// The follow seed for one successful stream: what that read bound offsets to.
fn seed(actor: &str, metadata: &std::fs::Metadata, streamed: &Streamed) -> SeatSeed {
    SeatSeed {
        actor: actor.to_owned(),
        identity: identity_of(metadata),
        mtime: metadata.modified().ok(),
        committed: streamed.committed,
    }
}

/// The source file's identity: path plus dev+inode, so a replaced file is a
/// different file even at the same path.
fn file_identity(path: &Path, metadata: &std::fs::Metadata) -> String {
    let (dev, ino) = identity_of(metadata);
    format!("{}#{dev}:{ino}", path.display())
}

/// A `u64` for the JSON stream: an integer while it fits, the raw literal past
/// it — never a clipped different number.
fn json_u64(value: u64) -> Value {
    i64::try_from(value).map_or_else(|_| Value::Raw(value.to_string()), Value::Num)
}

#[cfg(test)]
mod tests {
    use super::{
        Coverage, DoorError, LINE_CAP, LineBody, Role, Row, Splitter, collect, format_micros,
        stream_transcript,
    };
    use crate::tool::ToolKind;

    fn row(ts: i64, file: &str, offset: u64, body: &str) -> Row {
        Row {
            ts,
            actor: "s:seat".to_owned(),
            role: Role::Human,
            body: body.to_owned(),
            source: ToolKind::Claude,
            file: file.to_owned(),
            offset,
        }
    }

    #[test]
    fn collect_sorts_by_timestamp_file_then_offset() {
        let rows = collect(vec![
            row(3, "b", 0, "fourth"),
            row(1, "b", 9, "third"),
            row(1, "a", 0, "first"),
            row(1, "a", 5, "second"),
        ]);
        let bodies: Vec<&str> = rows.iter().map(|row| row.body.as_str()).collect();
        assert_eq!(bodies, ["first", "second", "third", "fourth"]);
    }

    #[test]
    fn collect_keeps_same_microsecond_rows_from_two_files() {
        // Mutation 4's guard: sorting by (ts, actor) would still order these,
        // but sorting must never consult the actor at all — same actor, same
        // micro, two files, both survive in file order.
        let rows = collect(vec![row(7, "b", 0, "bee"), row(7, "a", 0, "aye")]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].body, "aye");
        assert_eq!(rows[1].body, "bee");
    }

    #[test]
    fn collect_dedups_only_the_same_source_record() {
        let twice = row(5, "a", 3, "same words");
        let rows = collect(vec![
            twice.clone(),
            row(5, "a", 9, "same words"),
            twice,
            row(5, "b", 3, "same words"),
        ]);
        // One (a,3) survives; the same words at another offset or in another
        // file are different records and all survive.
        let kept: Vec<(&str, u64)> = rows
            .iter()
            .map(|row| (row.file.as_str(), row.offset))
            .collect();
        assert_eq!(kept, [("a", 3), ("a", 9), ("b", 3)]);
    }

    #[test]
    fn coverage_rows_carry_their_reason_verbatim() {
        let coverage = Coverage {
            actor: "s:seat".to_owned(),
            reason: "torn last record".to_owned(),
        };
        assert_eq!(coverage.reason, "torn last record");
    }

    #[test]
    fn micros_format_as_iso_utc_with_a_zero_padded_fraction() {
        assert_eq!(
            format_micros(1_789_549_200_500_000),
            "2026-09-16T09:00:00.500000Z"
        );
        assert_eq!(
            format_micros(1_789_549_200_000_007),
            "2026-09-16T09:00:00.000007Z"
        );
    }

    #[test]
    fn a_newline_split_across_two_feeds_ends_one_line() {
        let mut splitter = Splitter::new();
        splitter.feed(b"ab");
        splitter.feed(b"c\nde\n");
        let streamed = splitter.finish();
        assert!(!streamed.torn);
        assert_eq!(streamed.lines.len(), 2);
        assert_eq!(streamed.lines[0].offset, 0);
        assert!(matches!(
            streamed.lines[0].body,
            LineBody::Full(ref bytes) if bytes == b"abc"
        ));
        assert_eq!(streamed.lines[1].offset, 4);
        assert!(matches!(
            streamed.lines[1].body,
            LineBody::Full(ref bytes) if bytes == b"de"
        ));
    }

    #[test]
    fn two_newlines_in_one_feed_end_two_lines() {
        let mut splitter = Splitter::new();
        splitter.feed(b"a\nb\n");
        let streamed = splitter.finish();
        assert!(!streamed.torn);
        assert_eq!(streamed.lines.len(), 2);
        assert_eq!(streamed.lines[0].offset, 0);
        assert!(matches!(
            streamed.lines[0].body,
            LineBody::Full(ref bytes) if bytes == b"a"
        ));
        assert_eq!(streamed.lines[1].offset, 2);
        assert!(matches!(
            streamed.lines[1].body,
            LineBody::Full(ref bytes) if bytes == b"b"
        ));
    }

    #[test]
    fn an_overlong_line_reports_and_the_next_line_starts_clean() {
        let mut splitter = Splitter::new();
        splitter.feed(&vec![b'x'; LINE_CAP + 1]);
        splitter.feed(b"\nok\n");
        let streamed = splitter.finish();
        assert!(!streamed.torn);
        assert_eq!(streamed.lines.len(), 2);
        assert_eq!(streamed.lines[0].offset, 0);
        assert!(matches!(
            streamed.lines[0].body,
            LineBody::Overlong(len) if len == LINE_CAP + 1
        ));
        assert_eq!(streamed.lines[1].offset, (LINE_CAP + 2) as u64);
        assert!(matches!(
            streamed.lines[1].body,
            LineBody::Full(ref bytes) if bytes == b"ok"
        ));
    }

    #[test]
    fn a_splitter_base_keeps_offsets_absolute_and_commit_after_the_last_line() {
        let mut splitter = Splitter::at(100);
        splitter.feed(b"ab\nc");
        let streamed = splitter.finish();
        assert!(streamed.torn);
        assert_eq!(streamed.lines.len(), 1);
        assert_eq!(streamed.lines[0].offset, 100);
        assert_eq!(streamed.committed, 103, "the partial tail does not commit");
        let mut splitter = Splitter::at(7);
        splitter.feed(b"");
        let streamed = splitter.finish();
        assert!(!streamed.torn);
        assert_eq!(streamed.committed, 7, "an empty read commits at its base");
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test: stats its own scratch file to drive the door"
    )]
    fn the_door_reads_from_the_offset_it_is_given_with_absolute_offsets() {
        let dir = std::env::temp_dir().join(format!("ae-board-door-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        std::fs::write(&path, b"first\nsecond\n").unwrap();
        let metadata = std::fs::metadata(&path).unwrap();

        let streamed = stream_transcript(&path, &metadata, 6).unwrap();
        assert!(!streamed.torn);
        assert_eq!(streamed.lines.len(), 1);
        assert_eq!(
            streamed.lines[0].offset, 6,
            "the tail's offsets stay absolute"
        );
        assert_eq!(streamed.committed, 13);

        let whole = stream_transcript(&path, &metadata, 0).unwrap();
        assert_eq!(whole.committed, 13);
        assert_eq!(whole.lines.len(), 2);

        assert_eq!(
            stream_transcript(&path, &metadata, 14).unwrap_err(),
            DoorError::Changed,
            "an offset past the located length is a refusal, not a clipped read"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn rendered(rows: Vec<Row>, json: bool, lines: Option<usize>) -> String {
        let board = super::Observation {
            rows,
            ..super::Observation::default()
        };
        super::render_batch(&board, json, lines)
    }

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn a_text_row_indents_the_body_and_unclipped_prints_no_marker() {
        let text = rendered(vec![row(1, "a", 0, "one line")], false, None);
        assert!(
            text.contains("## 1970-01-01T00:00:00.000001Z s:seat\n  one line\n\n"),
            "{text}"
        );
        let exact = rendered(vec![row(1, "a", 0, "one\ntwo")], false, Some(2));
        assert!(exact.contains("  one\n  two\n\n"), "{exact}");
    }

    #[test]
    fn a_five_line_body_clipped_to_two_names_the_three_dropped() {
        let body = "one\ntwo\nthree\nfour\nfive";
        let text = rendered(vec![row(1, "a", 0, body)], false, Some(2));
        assert!(text.contains("  one\n  two\n  … +3 lines\n"), "{text}");
        let json = rendered(vec![row(1, "a", 0, body)], true, Some(1));
        assert!(
            json.contains("\"body\":\"one\\ntwo\\nthree\\nfour\\nfive\""),
            "{json}"
        );
    }

    #[test]
    fn lines_parses_anywhere_and_refuses_bad_values_and_combinations() {
        let parsed = super::parse(&words(&["day", "--lines", "3"])).expect("clipped tail parses");
        assert_eq!(parsed.lines, Some(3));
        for tail in [
            words(&["--lines", "0"]),
            words(&["--lines", "x"]),
            words(&["--lines"]),
        ] {
            assert!(super::parse(&tail).is_err(), "{tail:?}");
        }
        for tail in [
            words(&["--json", "--lines", "2"]),
            words(&["--lines", "2", "--json"]),
        ] {
            let usage = super::parse(&tail).expect_err("must refuse").render();
            assert!(usage.contains("--lines is text-only"), "{usage}");
        }
        assert!(super::parse(&words(&["--lines", "--json"])).is_err());
    }
}
