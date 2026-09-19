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
pub mod agy_transcript;
pub mod claude;
pub mod codex;
pub mod follow;
pub mod grok;
pub mod muse;
pub mod opencode;

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
    /// Which conversation this turn comes from: 0 is the seat's current one,
    /// n ≥ 1 its nth recorded predecessor, nearest first. Readers always
    /// build 0; the seat read stamps the generation it read.
    pub generation: u8,
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

/// One seat's hidden count: ae-injected turns [`hidden`] removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hidden {
    /// `session:seat`, from the meta roster.
    pub actor: String,
    /// How many of this seat's turns were hidden.
    pub count: usize,
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

/// THE ae-turn filter, after every reader and before [`collect`]. A row is
/// HIDDEN when its body's FIRST line is one of the marker spellings
/// [`crate::provenance`] owns — its predicate, never a respelling — or when the
/// whole body IS this seat's Codex passive launch turn ([`passive_turn`]).
/// Assistant rows are never hidden: a model may legitimately quote a marker.
#[must_use]
fn hidden(row: &Row, passive: Option<&str>) -> bool {
    if row.role != Role::Human {
        return false;
    }
    let first = row.body.lines().next().unwrap_or_default();
    crate::provenance::is_ae_turn(first) || passive.is_some_and(|turn| row.body == turn)
}

/// The body of one seat's passive launch turn, where its tool has one: the text
/// [`crate::launch::initial_prompt_for`] renders under its own marker line.
fn passive_turn(tool: ToolKind, meta_dir: &Path, slot: &str) -> Option<String> {
    let prompt = crate::launch::initial_prompt_for(tool, meta_dir, slot);
    let (_, body) = prompt.split_once('\n')?;
    Some(body.to_owned())
}

/// Count hidden rows into one [`Hidden`] per seat, first-encountered order.
fn count_hidden(rows: Vec<Row>) -> Vec<Hidden> {
    let mut counted: Vec<Hidden> = Vec::new();
    for row in rows {
        match counted.iter_mut().find(|item| item.actor == row.actor) {
            Some(item) => item.count += 1,
            None => counted.push(Hidden {
                actor: row.actor,
                count: 1,
            }),
        }
    }
    counted
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
#[allow(
    clippy::struct_excessive_bools,
    reason = "torn is splitter state; assistant and the two agy context bits are independent caller-bound defaults, and a state machine would couple every reader's construction to agy's two arms"
)]
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
    /// The caller already produced assistant rows for this read (agy's
    /// transcript leg runs first and reports back). False until bound, so
    /// the history reader covers the missing replies exactly when none
    /// were found — the line prints when it is true, never otherwise.
    pub(crate) assistant_rows_found: bool,
    /// This stream is a follow poll's tail, and the seat's assistant rows
    /// were read once on the first pass and are not followed. False until
    /// the caller binds it; only the agy follow arm ever does.
    pub(crate) assistant_read_once: bool,
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

    /// Bind whether the caller already produced assistant rows for this
    /// read: the history reader's missing-replies line answers to it.
    #[must_use]
    pub fn with_assistant_rows_found(mut self, found: bool) -> Self {
        self.assistant_rows_found = found;
        self
    }

    /// Bind that this stream is a follow poll's tail whose assistant rows
    /// were read once and are not followed.
    #[must_use]
    pub fn with_assistant_read_once(mut self, once: bool) -> Self {
        self.assistant_read_once = once;
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
            assistant_rows_found: false,
            assistant_read_once: false,
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

/// ONE clock for the text board: the UTC day and the wall time at second
/// precision, fraction dropped. The day drives the `# YYYY-MM-DD UTC` divider,
/// the time the `## HH:MM:SS` header. Built on the one timestamp spelling, so
/// a negative (pre-epoch) instant still renders.
pub(crate) fn clock_text(micros: i64) -> (String, String) {
    let stamped = crate::time::Timestamp::from_epoch(micros.div_euclid(1_000_000)).to_string();
    let day: String = stamped.chars().take(10).collect();
    let time: String = stamped.chars().skip(11).take(8).collect();
    (day, time)
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
pub struct SeatSeed {
    /// `session:seat`, from the meta roster.
    pub(crate) actor: String,
    /// The located file's dev+inode.
    pub(crate) identity: (u64, u64),
    /// The located file's mtime, when the OS reports one.
    pub(crate) mtime: Option<std::time::SystemTime>,
    /// Absolute position after the last complete line of the read.
    pub(crate) committed: u64,
    /// The newest row timestamp this read observed, if it observed any row.
    /// The follow seeds its divider day from these, `--since` applied there.
    pub(crate) last_row_ts: Option<i64>,
}

/// One board: every row read plus every seat that could not be read fully.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Observation {
    /// Board turns — human, and with `--assistant` the model's text replies —
    /// oldest first through [`collect`], `--since` applied.
    pub rows: Vec<Row>,
    /// One row per seat the board could not read fully, caller order.
    pub coverage: Vec<Coverage>,
    /// One row per seat whose read hid ae-injected turns, `--since` applied.
    pub hidden: Vec<Hidden>,
    /// What each successfully streamed seat's read observed; the follow seeds
    /// itself from these and a batch carries none.
    pub seeds: Vec<SeatSeed>,
    /// The UTC day printed before this observation's first row — `None` on a
    /// one-shot, the follow's held day on a batch. The text divider prints
    /// before the first row unless it names this day; JSON never reads it.
    pub(crate) day_floor: Option<String>,
}

/// Read every Claude, Codex, Grok, Muse and Antigravity seat of the handed-in
/// sessions. A seat that cannot be read — unknown tool, unlocated store,
/// unreadable transcript — becomes a [`Coverage`], never a silent subset; ae's
/// own injected turns are removed by [`hidden`] and counted per seat.
#[must_use]
pub fn observe(inputs: &Inputs<'_>, since_micros: Option<i64>) -> Observation {
    let mut rows = Vec::new();
    let mut coverage = Vec::new();
    let mut seeds = Vec::new();
    let mut hidden_rows = Vec::new();
    for session in inputs.sessions {
        let Ok(meta) = crate::session::read_meta(&session.path) else {
            coverage.push(Coverage {
                actor: format!("{}:?", session.name),
                reason: "session meta unreadable".to_owned(),
            });
            continue;
        };
        for entry in meta.roster() {
            let priors = meta.harness_session_prior(&entry.slot);
            hidden_rows.extend(observe_seat(
                session,
                entry,
                inputs.home,
                inputs.assistant,
                &priors,
                &mut rows,
                &mut coverage,
                &mut seeds,
            ));
        }
    }
    let mut rows = collect(rows);
    if let Some(since) = since_micros {
        rows.retain(|row| row.ts >= since);
        hidden_rows.retain(|row| row.ts >= since);
    }
    Observation {
        rows,
        coverage,
        hidden: count_hidden(hidden_rows),
        seeds,
        day_floor: None,
    }
}

/// ONE seat's own conversation, oldest first: the turns a seed pack carries.
///
/// Generation 0 ONLY. A predecessor's words belong to a conversation this seat
/// has already left behind, and what a successor takes over is the one running
/// now; [`observe_seat`] is where the whole chain is read instead.
///
/// The model's replies are always read, because half an exchange tells a
/// successor less than a short one — and [`Role::Assistant`] is text parts
/// alone, so no thinking and no tool call can reach a pack through here.
///
/// READ-ONLY and seedless: nothing follows this read, so the generation-0
/// follow seed is dropped rather than offered to a caller with no poll to feed.
/// Coverage comes back rather than being swallowed: a seat whose tool has no
/// reader must SAY so in the pack, or a successor reads "no turns" as "nothing
/// was said".
pub(crate) fn observe_seat_turns(
    session: &crate::usage::SessionInput,
    entry: &crate::meta::RosterEntry,
    home: Option<&Path>,
) -> (Vec<Row>, Vec<Coverage>) {
    let mut rows = Vec::new();
    let mut coverage = Vec::new();
    let mut seeds = Vec::new();
    // The hidden rows are already OUT of `rows`; the returned vec is the count
    // the board reports per seat, and a pack reports no such number.
    let _hidden = observe_generation(
        session,
        entry,
        home,
        true,
        0,
        &mut rows,
        &mut coverage,
        &mut seeds,
    );
    (collect(rows), coverage)
}

/// Read one roster seat: its current conversation, then — nearest first, at
/// most [`crate::meta::PRIOR_MAX`] — its recorded predecessors. Every
/// generation runs through the ONE read below; the current seat's id may be
/// `pending` after a fallback, and then its coverage row prints as today while
/// the predecessors still read. Returns every generation's hidden rows, so the
/// caller counts them under the one actor after `--since`.
#[allow(
    clippy::too_many_arguments,
    reason = "the seat read's eight facts — session, entry, home, flag, priors, rows, coverage, seeds — are each a distinct borrow"
)]
fn observe_seat(
    session: &crate::usage::SessionInput,
    entry: &crate::meta::RosterEntry,
    home: Option<&Path>,
    assistant: bool,
    priors: &[&str],
    rows: &mut Vec<Row>,
    coverage: &mut Vec<Coverage>,
    seeds: &mut Vec<SeatSeed>,
) -> Vec<Row> {
    let mut hidden = observe_generation(session, entry, home, assistant, 0, rows, coverage, seeds);
    let mut generation: u8 = 0;
    for element in priors.iter().rev().take(crate::meta::PRIOR_MAX) {
        generation += 1;
        let mut prior = entry.clone();
        // THE TAG DECIDES THE TOOL. A seat that was reseated onto another CLI
        // records predecessors from more than one store, so the locator and the
        // reader below are chosen by the element's OWN tool and only fall back
        // to the slot's current binary for a legacy untagged id — the chain's
        // rule for one, applied here rather than assumed. An element this
        // reader cannot judge is passed through as it stands, so the locator
        // still names it in a coverage row instead of it vanishing.
        //
        // The KNOWN GAP: `config_home` is the slot's current one. A predecessor
        // whose tool ALSO ran under another config home is looked for in the
        // wrong store and reported as not found, because a reseat replaces that
        // row and ae never recorded the old one.
        match crate::meta::prior_parts(element) {
            Some(parsed) => {
                prior.harness_session = Some(parsed.id.to_owned());
                if let Some(tool) = parsed.tool {
                    prior.binary = Some(tool.to_owned());
                }
            }
            None => prior.harness_session = Some((*element).to_owned()),
        }
        hidden.extend(observe_generation(
            session, &prior, home, assistant, generation, rows, coverage, seeds,
        ));
    }
    hidden
}

/// Read one generation of one roster seat: locate it through THE one locator,
/// stream it through the door, read it with the one dispatch for its tool,
/// then hide every ae-injected turn this generation's read produced. Every
/// refusal is a coverage row, never a silent subset. Generation 0 seeds the
/// follow; predecessors never do — an abandoned conversation never grows, so
/// no poll revisits one. Every predecessor row is stamped with its generation
/// and every predecessor coverage reason wears its `predecessor n:` prefix,
/// actor unchanged.
#[allow(
    clippy::too_many_arguments,
    reason = "one generation's nine facts — the seat read's eight plus its number — are each a distinct borrow"
)]
fn observe_generation(
    session: &crate::usage::SessionInput,
    entry: &crate::meta::RosterEntry,
    home: Option<&Path>,
    assistant: bool,
    generation: u8,
    rows: &mut Vec<Row>,
    coverage: &mut Vec<Coverage>,
    seeds: &mut Vec<SeatSeed>,
) -> Vec<Row> {
    let actor = format!("{}:{}", session.name, entry.name);
    let tool = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or(""));
    let mut cover = |reason: String| {
        coverage.push(Coverage {
            actor: actor.clone(),
            reason: if generation == 0 {
                reason
            } else {
                format!("predecessor {generation}: {reason}")
            },
        });
    };
    // OpenCode keeps its conversation in SQLite, which the board never opens:
    // the export leg runs the CLI (`opencode export <id>`) and the pure reader
    // takes its bytes. No file, no follow seed — D1 reads it once per board
    // and `follow_seat` covers it after that. String dispatch, as `reader_for`
    // does: the adapter's own name, never a `ToolKind::` arm.
    if tool.adapter().name == "opencode" {
        let (seat_rows, seat_coverage) = read_opencode(&actor, entry, tool, assistant);
        let (mut seat_rows, hidden_rows): (Vec<Row>, Vec<Row>) =
            seat_rows.into_iter().partition(|row| !hidden(row, None));
        for row in &mut seat_rows {
            row.generation = generation;
        }
        rows.append(&mut seat_rows);
        for item in seat_coverage {
            cover(item.reason);
        }
        return hidden_rows;
    }
    let (path, metadata) = match locate_seat(entry, home) {
        Ok(located) => located,
        Err(reason) => {
            cover(reason);
            return Vec::new();
        }
    };
    // The agy transcript leg runs before the history READ (never before its
    // locate, so an invalid id still refuses exactly once): it reports
    // whether assistant rows were produced, and the history stream binds
    // that verdict. With the flag off the leg never runs.
    let (leg_rows, leg_coverage) = if assistant && tool.adapter().name == "agy" {
        read_agy_transcript(&actor, entry, home, tool)
    } else {
        (Vec::new(), Vec::new())
    };
    let streamed = match stream_transcript(&path, &metadata, 0) {
        Ok(streamed) => streamed
            .for_seat(entry.harness_session.as_deref().unwrap_or_default())
            .with_assistant(assistant)
            .with_assistant_rows_found(!leg_rows.is_empty()),
        Err(failure) => {
            cover(door_reason(failure).to_owned());
            return Vec::new();
        }
    };
    let file = file_identity(&path, &metadata);
    let passive = passive_turn(tool, &session.path, &entry.slot);
    let (mut seat_rows, mut seat_coverage) = reader_for(tool)(&streamed, &actor, &file, tool);
    seat_rows.extend(leg_rows);
    seat_coverage.extend(leg_coverage);
    let (mut seat_rows, hidden_rows): (Vec<Row>, Vec<Row>) = seat_rows
        .into_iter()
        .partition(|row| !hidden(row, passive.as_deref()));
    for row in &mut seat_rows {
        row.generation = generation;
    }
    if generation == 0 {
        seeds.push(seed(&actor, &metadata, &streamed, tool, &seat_rows));
    }
    rows.append(&mut seat_rows);
    for item in seat_coverage {
        cover(item.reason);
    }
    hidden_rows
}

/// One agy generation's transcript leg: locate the seat's own transcript,
/// stream it through the ONE door, and read it with the transcript reader.
/// Absence is rows, not coverage — the caller binds the rows-found verdict
/// on the history stream, which covers the missing replies. Only a refused
/// door is a coverage row of this leg's own.
fn read_agy_transcript(
    actor: &str,
    entry: &crate::meta::RosterEntry,
    home: Option<&Path>,
    tool: ToolKind,
) -> (Vec<Row>, Vec<Coverage>) {
    let id = entry.harness_session.as_deref().unwrap_or_default();
    let found = match locate_agy_transcript(tool, home, id) {
        Ok(Some(found)) => found,
        Ok(None) => return (Vec::new(), Vec::new()),
        Err(reason) => {
            return (
                Vec::new(),
                vec![Coverage {
                    actor: actor.to_owned(),
                    reason,
                }],
            );
        }
    };
    let (path, metadata, truncated) = found;
    let streamed = match stream_transcript(&path, &metadata, 0) {
        Ok(streamed) => streamed.for_seat(id).with_assistant(true),
        Err(failure) => {
            return (
                Vec::new(),
                vec![Coverage {
                    actor: actor.to_owned(),
                    reason: door_reason(failure).to_owned(),
                }],
            );
        }
    };
    let file = format!("agy:{id}");
    agy_transcript::read_stream(&streamed, actor, &file, tool, truncated)
}

/// One `OpenCode` generation: prove the recorded id through the ONE grammar,
/// mint the export argv, run it through the EXISTING `opencode` leg, and hand
/// the stdout bytes to the pure reader. The export's wall time is the child's
/// and is charged to no shared budget — a slow `opencode` delays its own seat
/// only, never another seat's rows (D3).
fn read_opencode(
    actor: &str,
    entry: &crate::meta::RosterEntry,
    tool: ToolKind,
    assistant: bool,
) -> (Vec<Row>, Vec<Coverage>) {
    let refuse = |reason: &str| {
        (
            Vec::new(),
            vec![Coverage {
                actor: actor.to_owned(),
                reason: reason.to_owned(),
            }],
        )
    };
    let Some(id) = entry.harness_session.as_deref() else {
        return refuse("invalid or missing conversation id");
    };
    let Some(argv) = crate::session_launch::capture::opencode_export_argv(id) else {
        return refuse("invalid or missing conversation id");
    };
    // The caller's byte ceiling: the door returns at most EXPORT_CAP + 1
    // bytes, and the reader below refuses anything past its own cap with the
    // one budget reason.
    let (ran, exported) =
        crate::transport::run_opencode(&argv, crate::board::opencode::EXPORT_CAP as u64);
    if !ran {
        return refuse("export failed");
    }
    opencode::read(exported.as_bytes(), id, actor, tool, assistant)
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

/// One agy seat's transcript: `transcript_full.jsonl` first, else
/// `transcript.jsonl`, under the seat's own `brain/<id>/` store. The id is
/// grammar-proven before any path is built, so the join cannot traverse,
/// and each candidate goes through the ONE lstat — a symlink refuses, never
/// falls through. `Ok(None)` is an absent (or unprovable) store, which the
/// caller covers; the bool says the truncated sibling was opened.
fn locate_agy_transcript(
    tool: ToolKind,
    home: Option<&Path>,
    id: &str,
) -> Result<Option<(PathBuf, std::fs::Metadata, bool)>, String> {
    if !crate::session_launch::capture::is_lowercase_uuid(id) {
        return Ok(None);
    }
    let Some(home) = home else {
        return Ok(None);
    };
    let Some(dir) = tool.adapter().quota.default_home else {
        return Ok(None);
    };
    let logs = home
        .join(dir)
        .join("brain")
        .join(id)
        .join(".system_generated")
        .join("logs");
    let full = logs.join("transcript_full.jsonl");
    if let Some(metadata) = lstat_regular(&full)? {
        return Ok(Some((full, metadata, false)));
    }
    let tran = logs.join("transcript.jsonl");
    match lstat_regular(&tran)? {
        Some(metadata) => Ok(Some((tran, metadata, true))),
        None => Ok(None),
    }
}

/// One follow poll: re-resolve every selected session's roster, re-locate every
/// seat, read the bytes the held offsets ask for through the ONE door, and step
/// the state. Selection itself is the caller's and is fixed for the follow.
pub fn follow_poll(inputs: &Inputs<'_>, follow: &mut follow::Follow) -> Observation {
    let mut snapshots = Vec::new();
    for session in inputs.sessions {
        let Ok(meta) = crate::session::read_meta(&session.path) else {
            snapshots.push(follow::Snapshot {
                actor: format!("{}:?", session.name),
                located: Err("session meta unreadable".to_owned()),
                streamed: None,
                passive: None,
            });
            continue;
        };
        for entry in meta.roster() {
            snapshots.push(follow_seat(
                session,
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
/// The passive launch-turn body rides along: the batch's [`hidden`] filter
/// runs after the reader and an actor alone cannot carry it.
fn follow_seat(
    session: &crate::usage::SessionInput,
    entry: &crate::meta::RosterEntry,
    home: Option<&Path>,
    follow: &follow::Follow,
    assistant: bool,
) -> follow::Snapshot {
    let actor = format!("{}:{}", session.name, entry.name);
    let tool = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or(""));
    let passive = passive_turn(tool, &session.path, &entry.slot);
    // D1: an export per tick is a child process per seat per tick, and an
    // export has no append or offset semantics to follow — the one-shot read
    // stands, and this steady line says why no poll revisits it.
    if tool.adapter().name == "opencode" {
        return follow::Snapshot {
            actor,
            located: Err("opencode: read once, not followed".to_owned()),
            streamed: None,
            passive,
        };
    }
    let (path, metadata) = match locate_seat(entry, home) {
        Ok(located) => located,
        Err(reason) => {
            return follow::Snapshot {
                actor,
                located: Err(reason),
                streamed: None,
                passive,
            };
        }
    };
    // Agy + assistant: the transcript leg read once on the first pass and
    // polls tail history only. Store EXISTENCE (not a row count — the poll
    // reads nothing) decides the steady line; the reader's wins-rule keeps
    // it singular, and an empty store's first pass already said "no records".
    let read_once = assistant
        && tool.adapter().name == "agy"
        && matches!(
            locate_agy_transcript(
                tool,
                home,
                entry.harness_session.as_deref().unwrap_or_default()
            ),
            Ok(Some(_))
        );
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
                        .with_assistant_read_once(read_once)
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
        passive,
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
    // OpenCode is read through its export leg and never reaches this fallback.
    match tool.adapter().name {
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
const SCOPE_TEXT: &str = "scope: current conversations plus each seat's recorded predecessors (up to 4, newest first) — nothing is inferred from time";

/// Render the observation as text or NDJSON. Line 1 is ALWAYS the scope
/// statement; coverage rows precede the per-seat hidden lines, which precede
/// body rows; JSON never carries a bare line.
/// `lines` clips text bodies only — NDJSON always carries the whole body.
#[must_use]
pub fn render(observation: &Observation, json: bool, lines: Option<usize>) -> String {
    let mut out = if json {
        Value::obj([
            ("kind", Value::str("scope")),
            ("scope", Value::str("current-and-recorded-predecessors")),
            ("phase", Value::str("8b")),
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
pub fn render_batch(observation: &Observation, json: bool, lines: Option<usize>) -> String {
    if json {
        render_batch_json(observation)
    } else {
        render_batch_text(observation, lines)
    }
}

/// The indent every text body line wears, the clip marker included.
const BODY_INDENT: &str = "  ";

/// THE terminal-output owner for the text board: every C0 control except the
/// row structure's `\n` and `\t`, DEL, and every C1 control renders as
/// U+FFFD. A transcript body is hostile persisted state, and a raw ESC, BEL or
/// NUL would let a board read clear the screen, retitle the terminal or smuggle
/// an OSC 52 clipboard write; neutralising the introducer makes the rest of a
/// sequence inert text, so no escape grammar is parsed. Neither neighbour fits:
/// `goal::printable` DROPS controls and flattens to one line, and
/// `event_text::display_cell` keeps printable ASCII only, parses escape
/// sequences and bounds to a cell count; `sanitize::sanitize` is the compact
/// verb's terminal-INPUT strip (drops, normalizes CR and NEL, strict UTF-8).
/// This is the one owner for terminal OUTPUT, and [`render_batch_text`] is its
/// one call site.
#[must_use]
fn terminal_text(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch == '\n' || ch == '\t' || !ch.is_control() {
                ch
            } else {
                '\u{FFFD}'
            }
        })
        .collect()
}

/// ONE text row renderer: the header, every body line indented, a blank line,
/// and — with a clip — one marker naming the dropped remainder. Both the
/// one-shot and every follow batch print rows through this function.
fn text_row(row: &Row, lines: Option<usize>) -> String {
    let (_, time) = clock_text(row.ts);
    let mut out = format!("## {time} {}", row.actor);
    if row.generation > 0 {
        let _ = write!(out, " · prior {}", row.generation);
    }
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
    for item in &observation.hidden {
        let _ = writeln!(
            out,
            "hidden: {} — {} ae-injected turns",
            item.actor, item.count
        );
    }
    // The divider names the UTC day once and reprints only when the day moves
    // past the previously printed row's — within this batch, and across follow
    // batches through the observation's floor. Coverage stays above it.
    let mut last = observation.day_floor.clone();
    for row in &observation.rows {
        let (day, _) = clock_text(row.ts);
        if last.as_deref() != Some(day.as_str()) {
            let _ = writeln!(out, "# {day} UTC");
            last = Some(day);
        }
        out.push_str(&text_row(row, lines));
    }
    terminal_text(&out)
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
    for item in &observation.hidden {
        let line = Value::obj([
            ("kind", Value::str("hidden")),
            ("actor", Value::str(&item.actor)),
            (
                "count",
                json_u64(u64::try_from(item.count).unwrap_or(u64::MAX)),
            ),
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
            ("generation", Value::Num(i64::from(row.generation))),
        ])
        .render();
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Epoch micros as ISO 8601 UTC at micro precision. The text board reads the
/// clock now; this spelling stays for the tests that need identity.
#[cfg(test)]
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

/// The follow seed for one successful stream: what that read bound offsets
/// to, plus the newest row it observed for the divider day.
fn seed(
    actor: &str,
    metadata: &std::fs::Metadata,
    streamed: &Streamed,
    tool: ToolKind,
    seat_rows: &[Row],
) -> SeatSeed {
    SeatSeed {
        actor: actor.to_owned(),
        identity: identity_of(metadata),
        mtime: metadata.modified().ok(),
        committed: hold_at(streamed, tool).unwrap_or(streamed.committed),
        last_row_ts: seat_rows.iter().map(|row| row.ts).max(),
    }
}

/// The follow hold for a grok `--assistant` stream ending mid-turn: the open
/// run's first-chunk offset, where the commit point holds so the next poll
/// re-reads the whole turn. `None` everywhere else — other tools, the flag
/// off, or no open run — and the stream's own commit stands. String dispatch,
/// as `reader_for` does: no `ToolKind::` arms.
pub(crate) fn hold_at(streamed: &Streamed, source: ToolKind) -> Option<u64> {
    if !streamed.assistant || source.adapter().name != "grok" {
        return None;
    }
    grok::open_run_start(streamed)
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
    use std::path::{Path, PathBuf};

    fn row(ts: i64, file: &str, offset: u64, body: &str) -> Row {
        Row {
            ts,
            actor: "s:seat".to_owned(),
            role: Role::Human,
            body: body.to_owned(),
            source: ToolKind::Claude,
            file: file.to_owned(),
            offset,
            generation: 0,
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
    fn a_row_hides_when_its_first_line_is_an_ae_spelling() {
        for marker in [
            crate::provenance::peer("lead"),
            crate::provenance::ctx(),
            crate::provenance::brief("lead"),
            crate::provenance::interrupt("lead"),
        ] {
            assert!(super::hidden(&row(1, "a", 0, &marker), None), "{marker}");
            let buried = format!("human words\n{marker}");
            assert!(
                !super::hidden(&row(1, "a", 0, &buried), None),
                "a marker past line 1 is prose: {marker}"
            );
            let mut assistant = row(1, "a", 0, &marker);
            assistant.role = Role::Assistant;
            assert!(
                !super::hidden(&assistant, None),
                "assistant rows are never hidden: {marker}"
            );
        }
    }

    #[test]
    fn the_codex_passive_launch_turn_hides_by_its_exact_body() {
        let meta = Path::new("/tmp/ae-board-passive");
        let passive = super::passive_turn(ToolKind::Codex, meta, "main")
            .expect("codex carries a launch turn");
        assert!(
            !crate::provenance::is_ae_turn(&passive),
            "the marker line is not part of the body"
        );
        assert!(super::hidden(&row(1, "a", 0, &passive), Some(&passive)));
        let extended = format!("{passive} and one more word");
        assert!(
            !super::hidden(&row(1, "a", 0, &extended), Some(&passive)),
            "the body must match exactly"
        );
        assert!(super::passive_turn(ToolKind::Claude, meta, "main").is_none());
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
    fn a_control_laden_body_reaches_text_with_placeholders_and_no_control() {
        let body = "A\u{1b}[2J B\u{7} C\u{0} D\u{d} E\u{7f} F\u{9b}";
        let text = rendered(vec![row(1_789_549_200_500_000, "a", 0, body)], false, None);
        assert!(
            !text
                .chars()
                .any(|ch| ch.is_control() && ch != '\n' && ch != '\t'),
            "no forbidden byte reaches the terminal: {text:?}"
        );
        assert_eq!(text.matches('\u{FFFD}').count(), 6, "{text:?}");
        for visible in ["A", "B", "C", "D", "E", "F", "[2J"] {
            assert!(text.contains(visible), "{visible} kept: {text:?}");
        }
    }

    #[test]
    fn an_osc_clipboard_or_title_sequence_is_inert_text() {
        let body = "x\u{1b}]52;c;cGF5bG9hZA==\u{7}y\u{1b}]0;title\u{7}";
        let text = rendered(vec![row(1_789_549_200_500_000, "a", 0, body)], false, None);
        assert!(
            !text.contains('\u{1b}') && !text.contains('\u{7}'),
            "the introducer and terminator are gone: {text:?}"
        );
        assert!(
            text.contains("52;c;cGF5bG9hZA==") && text.contains("0;title"),
            "the payload stays as inert text: {text:?}"
        );
    }

    #[test]
    fn a_clean_multi_line_body_renders_byte_identically() {
        let text = rendered(
            vec![row(1_789_549_200_500_000, "a", 0, "first\nsecond")],
            false,
            None,
        );
        assert_eq!(
            text,
            "# 2026-09-16 UTC\n## 09:00:00 s:seat\n  first\n  second\n\n"
        );
    }

    #[test]
    fn the_json_row_for_a_control_laden_body_is_byte_identical() {
        // `--json` goes through json::escape_into: C0, DEL and C1 all leave as
        // `\u00XX` escapes (owner json.rs), so a hostile body cannot put a raw
        // control byte into the document.
        let body = "A\u{1b}[2J B\u{7} C\u{0} D\u{d} E\u{7f} F\u{9b}";
        let json = rendered(vec![row(1_789_549_200_500_000, "a", 0, body)], true, None);
        let expected = "{\"kind\":\"row\",\"ts\":1789549200500000,\"actor\":\"s:seat\",\"role\":\"human\",\"body\":\"A\\u001b[2J B\\u0007 C\\u0000 D\\r E\\u007f F\\u009b\",\"source\":\"claude\",\"file\":\"a\",\"offset\":0,\"generation\":0}\n";
        assert_eq!(json, expected);
    }

    #[test]
    fn a_text_row_indents_the_body_and_unclipped_prints_no_marker() {
        let text = rendered(vec![row(1, "a", 0, "one line")], false, None);
        assert!(
            text.contains("# 1970-01-01 UTC\n## 00:00:00 s:seat\n  one line\n\n"),
            "{text}"
        );
        let exact = rendered(vec![row(1, "a", 0, "one\ntwo")], false, Some(2));
        assert!(exact.contains("  one\n  two\n\n"), "{exact}");
    }

    #[test]
    fn the_clock_drops_the_fraction_and_survives_pre_epoch() {
        assert_eq!(
            super::clock_text(1_789_549_200_500_000),
            ("2026-09-16".to_owned(), "09:00:00".to_owned())
        );
        assert_eq!(
            super::clock_text(-1),
            ("1969-12-31".to_owned(), "23:59:59".to_owned())
        );
    }

    #[test]
    fn the_day_flips_exactly_at_midnight_utc() {
        // 2026-09-16T00:00:00Z, and the micro before it.
        assert_eq!(
            super::clock_text(1_789_516_800_000_000),
            ("2026-09-16".to_owned(), "00:00:00".to_owned())
        );
        assert_eq!(
            super::clock_text(1_789_516_799_999_999),
            ("2026-09-15".to_owned(), "23:59:59".to_owned())
        );
    }

    #[test]
    fn the_divider_prints_once_per_day_unless_the_floor_names_it() {
        let day_one = row(1_789_549_200_000_000, "a", 0, "morning");
        let day_two = row(1_789_636_800_000_000, "a", 1, "next day");
        let text = rendered(vec![day_one.clone(), day_two], false, None);
        assert_eq!(text.matches("# 2026-09-16 UTC\n").count(), 1, "{text}");
        assert_eq!(text.matches("# 2026-09-17 UTC\n").count(), 1, "{text}");
        let floored = super::Observation {
            rows: vec![day_one],
            day_floor: Some("2026-09-16".to_owned()),
            ..super::Observation::default()
        };
        let text = super::render_batch(&floored, false, None);
        assert!(!text.contains("# 2026-09-16"), "{text}");
        assert!(text.contains("## 09:00:00 s:seat"), "{text}");
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

    const PRIOR_OLD: &str = "0199c0de-1111-4890-abcd-ef0123456789";
    const PRIOR_NEW: &str = "0199c0de-2222-4890-abcd-ef0123456789";
    const CURRENT_ID: &str = "0199c0de-3333-4890-abcd-ef0123456789";

    /// A seat's store: readable transcripts for the handed ids, plus the meta
    /// carrying `current` as its id — `pending` plants no current transcript.
    #[allow(
        clippy::disallowed_methods,
        reason = "a test: plants its own scratch store to drive the seat read"
    )]
    fn prior_rig(tag: &str, current: &str, readable: &[&str]) -> (PathBuf, crate::meta::Meta) {
        let root =
            std::env::temp_dir().join(format!("ae-board-prior-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = root.join("claude");
        let dir = store.join("projects").join("work");
        std::fs::create_dir_all(&dir).expect("project dir");
        for &id in readable {
            std::fs::write(
                dir.join(format!("{id}.jsonl")),
                format!(
                    "{{\"type\":\"user\",\"timestamp\":\"2026-09-16T09:00:00Z\",\"message\":{{\"role\":\"user\",\"content\":\"words {id}\"}}}}\n"
                ),
            )
            .expect("transcript");
        }
        let meta = crate::meta::Meta::parse(&format!(
            "schema=2\nseat.main=lead\nharness_session.main={current}\nagent_bin.main=claude\nconfig_home.main={}\n",
            store.display()
        ));
        (root, meta)
    }

    fn read_seat(
        root: &std::path::Path,
        meta: &crate::meta::Meta,
        priors: &[&str],
    ) -> (Vec<Row>, Vec<Coverage>, Vec<super::SeatSeed>) {
        let entry = meta.roster().first().expect("one seat");
        let session = crate::usage::SessionInput {
            name: "s".to_owned(),
            path: root.join("sessions").join("s"),
        };
        let mut rows = Vec::new();
        let mut coverage = Vec::new();
        let mut seeds = Vec::new();
        let hidden = super::observe_seat(
            &session,
            entry,
            Some(root),
            false,
            priors,
            &mut rows,
            &mut coverage,
            &mut seeds,
        );
        assert!(hidden.is_empty(), "the fixtures inject no ae turn");
        (rows, coverage, seeds)
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test: removes its own scratch dir"
    )]
    fn predecessors_read_nearest_first_with_prefixed_coverage_and_no_seeds() {
        let (root, meta) = prior_rig("order", CURRENT_ID, &[CURRENT_ID, PRIOR_NEW]);
        let (rows, coverage, seeds) = read_seat(&root, &meta, &[PRIOR_OLD, PRIOR_NEW]);
        let mut pairs: Vec<(u8, &str)> = rows
            .iter()
            .map(|row| (row.generation, row.body.as_str()))
            .collect();
        pairs.sort_unstable();
        assert_eq!(
            pairs,
            [
                (0, &*format!("words {CURRENT_ID}")),
                (1, &*format!("words {PRIOR_NEW}")),
            ]
        );
        let reasons: Vec<&str> = coverage.iter().map(|item| item.reason.as_str()).collect();
        assert_eq!(reasons, ["predecessor 2: transcript not found"]);
        assert_eq!(coverage[0].actor, "s:lead", "actor unchanged");
        assert_eq!(seeds.len(), 1, "the current read alone seeds");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test: removes its own scratch dir"
    )]
    fn a_pending_current_covers_while_its_predecessor_reads() {
        let (root, meta) = prior_rig("pending", "pending", &[PRIOR_NEW]);
        let (rows, coverage, seeds) = read_seat(&root, &meta, &[PRIOR_NEW]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].generation, 1);
        let reasons: Vec<&str> = coverage.iter().map(|item| item.reason.as_str()).collect();
        assert_eq!(reasons, ["invalid or missing conversation id"]);
        assert!(seeds.is_empty(), "a covered read seeds nothing");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test: plants and removes its own scratch session"
    )]
    fn an_opencode_seat_is_read_once_and_every_follow_poll_says_so_once() {
        let root = std::env::temp_dir().join(format!("ae-board-oc-follow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("sessions").join("s");
        std::fs::create_dir_all(&dir).expect("session dir");
        std::fs::write(
            dir.join("meta"),
            "schema=2\nseat.main=oc\nharness_session.main=ses_00000000000000000000000000\nagent_bin.main=opencode\n",
        )
        .expect("meta");
        let sessions = vec![crate::usage::SessionInput {
            name: "s".to_owned(),
            path: dir,
        }];
        let inputs = crate::board::Inputs {
            home: Some(root.as_path()),
            sessions: &sessions,
            assistant: false,
        };
        let mut follow = super::follow::Follow::seeded(&[], &[], None);
        let batch = super::follow_poll(&inputs, &mut follow);
        assert!(batch.rows.is_empty(), "no export runs per tick");
        assert_eq!(batch.coverage.len(), 1);
        assert_eq!(
            batch.coverage[0].reason,
            "opencode: read once, not followed"
        );
        let batch = super::follow_poll(&inputs, &mut follow);
        assert!(
            batch.coverage.is_empty(),
            "the steady reason prints on change only"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test: plants and removes its own scratch session"
    )]
    fn an_agy_seat_streams_history_while_its_replies_read_once() {
        let id = "0199c0de-ffff-4890-abcd-ef0123456789";
        let root = std::env::temp_dir().join(format!("ae-board-agy-follow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("sessions").join("s");
        std::fs::create_dir_all(&dir).expect("session dir");
        std::fs::write(
            dir.join("meta"),
            format!("schema=2\nseat.main=lead\nharness_session.main={id}\nagent_bin.main=agy\n"),
        )
        .expect("meta");
        let store = root.join(".gemini/antigravity-cli");
        std::fs::create_dir_all(&store).expect("agy dir");
        std::fs::write(
            store.join("history.jsonl"),
            format!(
                "{{\"display\":\"synthetic human words\",\"timestamp\":1789549200500,\"conversationId\":\"{id}\",\"workspace\":\"/work\"}}\n"
            ),
        )
        .expect("history");
        let logs = store.join("brain").join(id).join(".system_generated/logs");
        std::fs::create_dir_all(&logs).expect("brain dir");
        std::fs::write(
            logs.join("transcript_full.jsonl"),
            "{\"step_index\":1,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"created_at\":\"2026-09-16T09:00:00Z\",\"content\":\"synthetic reply\"}\n",
        )
        .expect("transcript");
        let sessions = vec![crate::usage::SessionInput {
            name: "s".to_owned(),
            path: dir,
        }];
        let inputs = crate::board::Inputs {
            home: Some(root.as_path()),
            sessions: &sessions,
            assistant: true,
        };
        let mut follow = super::follow::Follow::seeded(&[], &[], None);
        let batch = super::follow_poll(&inputs, &mut follow);
        assert_eq!(batch.rows.len(), 1, "the human row streams");
        assert_eq!(batch.rows[0].body, "synthetic human words");
        assert_eq!(batch.coverage.len(), 1, "the once-read line prints");
        assert_eq!(
            batch.coverage[0].reason,
            "agy: assistant replies read once, not followed"
        );
        let batch = super::follow_poll(&inputs, &mut follow);
        assert!(batch.rows.is_empty() && batch.coverage.is_empty(), "steady");
        let off = crate::board::Inputs {
            home: Some(root.as_path()),
            sessions: &sessions,
            assistant: false,
        };
        let mut follow = super::follow::Follow::seeded(&[], &[], None);
        let batch = super::follow_poll(&off, &mut follow);
        assert_eq!(batch.rows.len(), 1);
        assert!(batch.coverage.is_empty(), "flag off: no once-read line");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_prior_header_composes_with_assistant_and_json_names_every_generation() {
        let mut prior = row(1_789_549_201_000_000, "a", 0, "old words");
        prior.generation = 1;
        prior.role = Role::Assistant;
        let text = rendered(vec![row(1, "b", 0, "new words"), prior], false, None);
        assert!(
            text.contains("## 09:00:01 s:seat · prior 1 · assistant\n  old words\n"),
            "{text}"
        );
        assert!(!text.contains("· prior 0"), "{text}");
        let json = rendered(
            vec![row(1, "b", 0, "new words"), {
                let mut prior = row(1_789_549_201_000_000, "a", 0, "old words");
                prior.generation = 1;
                prior
            }],
            true,
            None,
        );
        assert!(json.contains("\"generation\":0"), "{json}");
        assert!(json.contains("\"generation\":1"), "{json}");
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
