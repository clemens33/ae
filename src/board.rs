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

pub mod claude;
pub mod codex;

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufRead, BufReader, Read as _};
use std::path::Path;

use crate::json::Value;
use crate::quota::{Bounded, Budget};
use crate::tool::{ToolKind, UsageSource};

/// Who speaks in a board row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A genuine human turn: bare of any ae marker.
    Human,
    // Phase 7 adds `Assistant`. It is absent on purpose until then: a variant
    // nothing constructs would be dead code the gate refuses.
}

/// One board row: one human turn from one harness transcript.
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
pub const USAGE: &str = "Usage: ae board [session…] [--since <ts>] [--json]\n\n  session       read this session (repeatable; default is every running session)\n  --since <ts>  keep rows at or after <ts>, strict YYYY-MM-DDTHH:MM:SSZ\n  --json        NDJSON: one scope line, coverage lines, then row lines\n";

/// A parsed `ae board` argv.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    /// Explicit sessions, caller order, deduplicated by rejection.
    pub sessions: Vec<String>,
    /// `--since`, in epoch micros — rows at or after this survive.
    pub since_micros: Option<i64>,
    /// `--json`: NDJSON instead of text.
    pub json: bool,
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
///     Ok(Args { sessions: vec!["aedev".to_owned()], since_micros: None, json: true })
/// );
/// assert!(parse(&words(&["--frobnicate"])).is_err());
/// ```
///
/// # Errors
///
/// [`Usage`] for an unknown flag, a repeated session name, `--since` without a
/// strict `YYYY-MM-DDTHH:MM:SSZ` value, or a value missing entirely.
pub fn parse(tail: &[String]) -> Result<Args, Usage> {
    let mut args = Args::default();
    let mut index = 0;
    while let Some(token) = tail.get(index) {
        match token.as_str() {
            "--json" => args.json = true,
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
        Self::default()
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
    /// torn tail was seen and not trusted.
    #[must_use]
    pub fn finish(self) -> Streamed {
        Streamed {
            lines: self.lines,
            torn: self.cursor != self.start,
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
/// chunks to the ONE [`Splitter`]. No splitting logic and no harness
/// knowledge inside: open, prove, feed.
pub(crate) fn stream_transcript(
    path: &Path,
    expected: &std::fs::Metadata,
) -> Result<Streamed, DoorError> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: streams only the lstat-checked board transcript"
    )]
    let file = File::open(path).map_err(|_| DoorError::Unreadable)?;
    let opened = file.metadata().map_err(|_| DoorError::Unreadable)?;
    if !opened.file_type().is_file() {
        return Err(DoorError::NotRegular);
    }
    if opened.len() < expected.len() || !same_file(expected, &opened) {
        return Err(DoorError::Changed);
    }
    let mut reader = BufReader::new(file.take(expected.len()));
    let mut splitter = Splitter::new();
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
}

/// One board: every row read plus every seat that could not be read fully.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Observation {
    /// Human turns, oldest first through [`collect`], `--since` applied.
    pub rows: Vec<Row>,
    /// One row per seat the board could not read fully, caller order.
    pub coverage: Vec<Coverage>,
}

/// Read every Claude and Codex seat of the handed-in sessions. A seat that
/// cannot be read — unknown tool, unlocated store, unreadable transcript —
/// becomes a [`Coverage`], never a silent subset.
#[must_use]
pub fn observe(inputs: &Inputs<'_>, since_micros: Option<i64>) -> Observation {
    let mut rows = Vec::new();
    let mut coverage = Vec::new();
    for session in inputs.sessions {
        let Ok(meta) = crate::session::read_meta(&session.path) else {
            coverage.push(Coverage {
                actor: format!("{}:?", session.name),
                reason: "session meta unreadable".to_owned(),
            });
            continue;
        };
        for entry in meta.roster() {
            observe_seat(&session.name, entry, inputs.home, &mut rows, &mut coverage);
        }
    }
    let mut rows = collect(rows);
    if let Some(since) = since_micros {
        rows.retain(|row| row.ts >= since);
    }
    Observation { rows, coverage }
}

/// Read one roster seat: Claude and Codex transcripts stream through the door
/// into their reader; every other harness names its phase in a coverage row.
fn observe_seat(
    session: &str,
    entry: &crate::meta::RosterEntry,
    home: Option<&Path>,
    rows: &mut Vec<Row>,
    coverage: &mut Vec<Coverage>,
) {
    let actor = format!("{}:{}", session, entry.name);
    let tool = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or(""));
    let source = tool.adapter().usage.source;
    if matches!(source, UsageSource::CodexRollout) {
        observe_codex_seat(entry, home, &actor, tool, rows, coverage);
        return;
    }
    if !matches!(source, UsageSource::ClaudeTranscripts) {
        coverage.push(Coverage {
            actor,
            reason: unsupported_reason(tool).to_owned(),
        });
        return;
    }
    let cover = |reason: String| Coverage {
        actor: actor.clone(),
        reason,
    };
    let Some(id) = crate::usage::valid_id(entry.harness_session.as_deref()) else {
        coverage.push(cover("invalid or missing conversation id".to_owned()));
        return;
    };
    let store = match crate::usage::source_for(entry, UsageSource::ClaudeTranscripts, home) {
        Ok(store) => store,
        Err(reason) => {
            coverage.push(cover(reason));
            return;
        }
    };
    let mut budget = Budget::new();
    let located = match crate::usage::locate_claude_parent(&store, &id, &mut budget) {
        Ok(located) => located,
        Err(failure) => {
            coverage.push(cover(locate_reason(&failure)));
            return;
        }
    };
    let Some(transcript) = located else {
        coverage.push(cover("transcript not found".to_owned()));
        return;
    };
    let streamed = match stream_transcript(&transcript.path, &transcript.metadata) {
        Ok(streamed) => streamed,
        Err(failure) => {
            coverage.push(cover(door_reason(failure).to_owned()));
            return;
        }
    };
    let file = file_identity(&transcript.path, &transcript.metadata);
    let (mut seat_rows, mut seat_coverage) = claude::read_stream(&streamed, &actor, &file, tool);
    rows.append(&mut seat_rows);
    coverage.append(&mut seat_coverage);
}

/// Read one Codex roster seat through usage's source, quota's finder and the
/// EXISTING door — the locator hands over its lstat, no second stat.
fn observe_codex_seat(
    entry: &crate::meta::RosterEntry,
    home: Option<&Path>,
    actor: &str,
    tool: ToolKind,
    rows: &mut Vec<Row>,
    coverage: &mut Vec<Coverage>,
) {
    let cover = |reason: String| Coverage {
        actor: actor.to_owned(),
        reason,
    };
    let Some(id) = crate::usage::valid_id(entry.harness_session.as_deref()) else {
        coverage.push(cover("invalid or missing conversation id".to_owned()));
        return;
    };
    let root = match crate::usage::source_for(entry, UsageSource::CodexRollout, home) {
        Ok(root) => root,
        Err(reason) => {
            coverage.push(cover(reason));
            return;
        }
    };
    let mut budget = Budget::new();
    let rollout = match crate::quota::find_codex_rollout(&root, &id, &mut budget) {
        Ok(Bounded::Ready(Some(rollout))) => rollout,
        Ok(Bounded::Ready(None)) => {
            coverage.push(cover("rollout not found".to_owned()));
            return;
        }
        Ok(Bounded::Truncated) => {
            coverage.push(cover("rollout scan truncated".to_owned()));
            return;
        }
        // Quota's errors name the cause already; the message carries through.
        Err(error) => {
            coverage.push(cover(error.to_string()));
            return;
        }
    };
    let streamed = match stream_transcript(rollout.path(), rollout.metadata()) {
        Ok(streamed) => streamed,
        Err(failure) => {
            coverage.push(cover(door_reason(failure).to_owned()));
            return;
        }
    };
    let file = file_identity(rollout.path(), rollout.metadata());
    let (mut seat_rows, mut seat_coverage) = codex::read_stream(&streamed, actor, &file, tool);
    rows.append(&mut seat_rows);
    coverage.append(&mut seat_coverage);
}

/// The phase (or ruling) behind a harness the board cannot read yet.
fn unsupported_reason(tool: ToolKind) -> &'static str {
    // String dispatch, never `ToolKind::` arms: production tool literals live
    // in `src/tool.rs` alone (`per_tool_branches_live_only_in_the_adapter_rows`).
    match tool.adapter().name {
        "grok" => "grok: phase 3a",
        "muse" => "muse: phase 3b",
        "agy" => "agy: phase 5",
        "opencode" => "opencode: ruling pending",
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
#[must_use]
pub fn render(observation: &Observation, json: bool) -> String {
    if json {
        render_json(observation)
    } else {
        render_text(observation)
    }
}

fn render_text(observation: &Observation) -> String {
    let mut out = String::from(SCOPE_TEXT);
    out.push('\n');
    for item in &observation.coverage {
        let _ = writeln!(out, "coverage incomplete: {} — {}", item.actor, item.reason);
    }
    for row in &observation.rows {
        let _ = writeln!(
            out,
            "## {} {}\n{}\n",
            format_micros(row.ts),
            row.actor,
            row.body
        );
    }
    out
}

fn render_json(observation: &Observation) -> String {
    let mut out = Value::obj([
        ("kind", Value::str("scope")),
        ("scope", Value::str("current-conversations")),
        ("phase", Value::str("1b")),
    ])
    .render();
    out.push('\n');
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

/// The source file's identity: path plus dev+inode, so a replaced file is a
/// different file even at the same path.
#[cfg(unix)]
fn file_identity(path: &Path, metadata: &std::fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt as _;
    format!("{}#{}:{}", path.display(), metadata.dev(), metadata.ino())
}

/// The source file's identity where no dev+inode exists: the path alone.
#[cfg(not(unix))]
fn file_identity(path: &Path, _: &std::fs::Metadata) -> String {
    path.display().to_string()
}

/// A `u64` for the JSON stream: an integer while it fits, the raw literal past
/// it — never a clipped different number.
fn json_u64(value: u64) -> Value {
    i64::try_from(value).map_or_else(|_| Value::Raw(value.to_string()), Value::Num)
}

#[cfg(test)]
mod tests {
    use super::{Coverage, LINE_CAP, LineBody, Role, Row, Splitter, collect, format_micros};
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
}
