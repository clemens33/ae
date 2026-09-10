//! Offline API-equivalent spend from agent-owned transcripts.

pub mod claude;
pub mod codex;
pub mod prices;

use std::fs::File;
use std::io::{BufRead, BufReader, Read as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::meta::{RecordedConfigHome, RecordedConfigHomeBase, RosterEntry};
use crate::quota::{Bounded, Budget};
use crate::tool::{ToolKind, UsageSource};

const EVENTS_MAX_BYTES: u64 = 4 * 1024 * 1024;
const CLAUDE_SEAT_MAX_BYTES: u64 = 512 * 1024 * 1024;
const CLAUDE_LINE_MAX_BYTES: usize = 1024 * 1024;
const SESSION_META_MAX_BYTES: u64 = 1024 * 1024;
const CODEX_HEAD_BYTES: u64 = 256 * 1024;
const CODEX_TAIL_BYTES: u64 = 256 * 1024;
const MODEL_MAX_BYTES: usize = 256;

/// Billable token categories shared by transcript adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tokens {
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
}

impl Tokens {
    pub(crate) const fn saturating_add(self, other: Self) -> Self {
        Self {
            input: self.input.saturating_add(other.input),
            cache_write: self.cache_write.saturating_add(other.cache_write),
            cache_read: self.cache_read.saturating_add(other.cache_read),
            output: self.output.saturating_add(other.output),
        }
    }

    pub(crate) const fn total(self) -> u64 {
        self.input
            .saturating_add(self.cache_write)
            .saturating_add(self.cache_read)
            .saturating_add(self.output)
    }
}

/// One live session selected by the command boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInput {
    pub name: String,
    pub path: PathBuf,
}

/// Validated public `usage` arguments.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    pub sessions: Vec<String>,
    pub json: bool,
}

/// Parse `ae usage [session…] [--json]` without reading the world.
///
/// # Errors
///
/// Returns the first duplicate, unknown option, or invalid session name.
pub fn parse_args(tail: &[String]) -> Result<Args, String> {
    let mut parsed = Args::default();
    for word in tail {
        match word.as_str() {
            "--json" if !parsed.json => parsed.json = true,
            "--json" => return Err(word.clone()),
            option if option.starts_with('-') => return Err(word.clone()),
            name if crate::session_launch::name::is_session_name(name) => {
                if parsed.sessions.iter().any(|known| known == name) {
                    return Err(word.clone());
                }
                parsed.sessions.push(name.to_owned());
            }
            _ => return Err(word.clone()),
        }
    }
    Ok(parsed)
}

/// Preserve caller order while selecting only positively live sessions.
///
/// # Errors
///
/// Returns the first requested name absent from `live`.
pub fn select_sessions(
    live: &[SessionInput],
    requested: &[String],
) -> Result<Vec<SessionInput>, String> {
    if requested.is_empty() {
        return Ok(live.to_vec());
    }
    let mut selected = Vec::new();
    for name in requested {
        let Some(session) = live.iter().find(|session| &session.name == name) else {
            return Err(name.clone());
        };
        selected.push(session.clone());
    }
    Ok(selected)
}

/// Filesystem roots and already-classified live sessions.
pub struct Inputs<'a> {
    pub home: Option<&'a Path>,
    pub sessions: &'a [SessionInput],
    pub prices: &'a prices::Book,
    pub now: i64,
}

/// Whether a seat's full transcript was available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Coverage {
    Read,
    Unreadable(String),
    Unsupported,
    Unlocated,
    Truncated,
}

/// One model row for one live or retired seat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatUsage {
    pub seat: String,
    pub slot: String,
    pub tool: String,
    pub model: String,
    pub tokens: Tokens,
    pub usd_micro: Option<u64>,
    pub observed_at: Option<i64>,
    pub retired: bool,
    pub coverage: Coverage,
    pub approximate: bool,
}

/// Aggregated rows, with missing supported coverage made explicit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsageTotal {
    pub tokens: Tokens,
    pub usd_micro: u64,
    pub partial: bool,
}

/// One live session and all attributable seats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionUsage {
    pub name: String,
    pub seats: Vec<SeatUsage>,
    pub total: UsageTotal,
    pub retired_scan_truncated: bool,
}

/// Stable data API consumed by the command and later watchdog integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub sessions: Vec<SessionUsage>,
    pub unpriced: Vec<String>,
    pub now: i64,
}

/// Observe only the roots and live sessions handed in by the command boundary.
#[must_use]
pub fn observe(inputs: &Inputs<'_>) -> Observation {
    let mut unpriced = Vec::new();
    let mut sessions = Vec::new();
    for input in inputs.sessions {
        let mut meta_budget = Budget::new();
        let mut seats = match read_meta(&input.path, &mut meta_budget) {
            Ok(meta) => {
                let mut seats = Vec::new();
                for entry in meta.roster() {
                    let mut seat_budget = Budget::new();
                    seats.extend(observe_entry(
                        entry,
                        inputs,
                        &mut seat_budget,
                        &mut unpriced,
                        false,
                    ));
                }
                seats
            }
            Err(_) => Vec::new(),
        };
        let mut retired_budget = Budget::new();
        let retired_scan_truncated = add_retired(
            &input.path,
            inputs,
            &mut retired_budget,
            &mut seats,
            &mut unpriced,
        );
        let mut total = total_of(&seats);
        if seats.is_empty() || retired_scan_truncated {
            total.partial = true;
        }
        sessions.push(SessionUsage {
            name: input.name.clone(),
            seats,
            total,
            retired_scan_truncated,
        });
    }
    Observation {
        sessions,
        unpriced,
        now: inputs.now,
    }
}

pub(crate) fn local_config(dir: &Path) -> Option<PathBuf> {
    let mut budget = Budget::new();
    read_meta(dir, &mut budget)
        .ok()
        .and_then(|meta| meta.origin().map(str::to_owned))
        .and_then(|origin| crate::config::local_overlay(dir, &origin))
}

fn read_meta(dir: &Path, budget: &mut Budget) -> Result<crate::meta::Meta, Coverage> {
    let path = dir.join(crate::meta::FILE);
    let Some((bytes, _)) = read_regular(&path, SESSION_META_MAX_BYTES, budget)? else {
        return Err(Coverage::Unreadable("session meta not found".to_owned()));
    };
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| Coverage::Unreadable("session meta is not UTF-8".to_owned()))?;
    Ok(crate::meta::Meta::parse(text))
}

pub(crate) fn is_model_id(model: &str) -> bool {
    !model.is_empty() && model.len() <= MODEL_MAX_BYTES && !model.chars().any(char::is_control)
}

fn observe_entry(
    entry: &RosterEntry,
    inputs: &Inputs<'_>,
    budget: &mut Budget,
    unpriced: &mut Vec<String>,
    retired: bool,
) -> Vec<SeatUsage> {
    let tool = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or(""));
    match tool.adapter().usage.source {
        UsageSource::ClaudeTranscripts => {
            observe_claude(entry, tool, inputs, budget, unpriced, retired)
        }
        UsageSource::CodexRollout => {
            vec![observe_codex(
                entry, tool, inputs, budget, unpriced, retired,
            )]
        }
        UsageSource::Unsupported => vec![unsupported(entry, tool, retired)],
    }
}

fn source_for(
    entry: &RosterEntry,
    source: UsageSource,
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(identity) = crate::quota::recorded_identity(entry) {
        return match (source, &entry.config_home) {
            (
                UsageSource::ClaudeTranscripts,
                RecordedConfigHome::Path(store) | RecordedConfigHome::Implicit(store),
            ) => Ok(store.clone()),
            (UsageSource::CodexRollout, _) => Ok(identity.source),
            (UsageSource::ClaudeTranscripts | UsageSource::Unsupported, _) => {
                Err("unsupported or inconsistent usage source".to_owned())
            }
        };
    }
    if matches!(entry.config_home, RecordedConfigHome::Missing)
        && matches!(entry.config_home_base, RecordedConfigHomeBase::Missing)
    {
        let home = home.ok_or_else(|| "legacy config home unavailable".to_owned())?;
        return Ok(match source {
            UsageSource::ClaudeTranscripts => home.join(".claude"),
            UsageSource::CodexRollout => home.join(".codex").join("sessions"),
            UsageSource::Unsupported => return Err("unsupported tool".to_owned()),
        });
    }
    Err("recorded config home is absent, unknown, invalid, or inconsistent".to_owned())
}

fn observe_claude(
    entry: &RosterEntry,
    tool: ToolKind,
    inputs: &Inputs<'_>,
    budget: &mut Budget,
    unpriced: &mut Vec<String>,
    retired: bool,
) -> Vec<SeatUsage> {
    let Some(id) = valid_id(entry.harness_session.as_deref()) else {
        return vec![if retired {
            coverage_row(entry, tool, Coverage::Unlocated, true)
        } else {
            unreadable(entry, tool, "invalid or missing conversation id", false)
        }];
    };
    let store = match source_for(entry, UsageSource::ClaudeTranscripts, inputs.home) {
        Ok(store) => store,
        Err(reason) => return vec![unreadable(entry, tool, &reason, retired)],
    };
    let found = match claude_files(&store, &id, budget) {
        Ok(found) => found,
        Err(coverage) => return vec![coverage_row(entry, tool, coverage, retired)],
    };
    let Some((parent, sidechains, observed_at)) = found else {
        return vec![unreadable(entry, tool, "transcript not found", retired)];
    };
    let models = claude::reduce(&parent, &sidechains);
    if models.is_empty() {
        return vec![unreadable(entry, tool, "transcript has no usage", retired)];
    }
    models
        .into_iter()
        .map(|model| {
            priced_row(
                entry,
                model.model,
                model.tokens,
                RowFacts {
                    tool,
                    observed_at,
                    retired,
                    approximate: false,
                },
                inputs.prices,
                unpriced,
            )
        })
        .collect()
}

fn observe_codex(
    entry: &RosterEntry,
    tool: ToolKind,
    inputs: &Inputs<'_>,
    budget: &mut Budget,
    unpriced: &mut Vec<String>,
    retired: bool,
) -> SeatUsage {
    let Some(id) = valid_id(entry.harness_session.as_deref()) else {
        return if retired {
            coverage_row(entry, tool, Coverage::Unlocated, true)
        } else {
            unreadable(entry, tool, "invalid or missing conversation id", false)
        };
    };
    let root = match source_for(entry, UsageSource::CodexRollout, inputs.home) {
        Ok(root) => root,
        Err(reason) => return unreadable(entry, tool, &reason, retired),
    };
    let file = match crate::quota::find_codex_rollout(&root, &id, budget) {
        Ok(Bounded::Ready(Some(file))) => file,
        Ok(Bounded::Ready(None)) => {
            return unreadable(entry, tool, "rollout not found", retired);
        }
        Ok(Bounded::Truncated) => {
            return coverage_row(entry, tool, Coverage::Truncated, retired);
        }
        Err(_) => return unreadable(entry, tool, "rollout unreadable", retired),
    };
    let observed_at = file.modified().and_then(epoch);
    let head = if file.len() > CODEX_TAIL_BYTES {
        match crate::quota::bounded_head_after_lstat(&file, CODEX_HEAD_BYTES, budget) {
            Ok(Bounded::Ready(head)) => head,
            Ok(Bounded::Truncated) => {
                return coverage_row(entry, tool, Coverage::Truncated, retired);
            }
            Err(_) => return unreadable(entry, tool, "rollout unreadable", retired),
        }
    } else {
        Vec::new()
    };
    let (bytes, boundary) =
        match crate::quota::bounded_tail_after_lstat(&file, CODEX_TAIL_BYTES, budget) {
            Ok(Bounded::Ready(found)) => found,
            Ok(Bounded::Truncated) => {
                return coverage_row(entry, tool, Coverage::Truncated, retired);
            }
            Err(_) => return unreadable(entry, tool, "rollout unreadable", retired),
        };
    let parsed = codex::parse_with_head(&head, &bytes, boundary);
    match parsed.models.as_slice() {
        [model] => priced_row(
            entry,
            model.clone(),
            parsed.tokens,
            RowFacts {
                tool,
                observed_at,
                retired,
                approximate: parsed.approximate,
            },
            inputs.prices,
            unpriced,
        ),
        [] => unreadable(entry, tool, "rollout has no model", retired),
        _ => SeatUsage {
            seat: if retired {
                format!("{} (retired)", entry.name)
            } else {
                entry.name.clone()
            },
            slot: entry.slot.clone(),
            tool: tool.as_str().to_owned(),
            model: "mixed models".to_owned(),
            tokens: parsed.tokens,
            usd_micro: None,
            observed_at,
            retired,
            coverage: Coverage::Read,
            approximate: parsed.approximate,
        },
    }
}

type ClaudeFiles = Option<(claude::Parsed, Vec<claude::Parsed>, Option<i64>)>;

struct TranscriptFile {
    path: PathBuf,
    metadata: std::fs::Metadata,
    modified: Option<SystemTime>,
}

struct TranscriptBudget {
    bytes_left: u64,
}

impl TranscriptBudget {
    const fn new() -> Self {
        Self {
            bytes_left: CLAUDE_SEAT_MAX_BYTES,
        }
    }

    fn claim(&mut self, bytes: u64) -> bool {
        if bytes > self.bytes_left {
            return false;
        }
        self.bytes_left -= bytes;
        true
    }
}

fn claude_files(store: &Path, id: &str, budget: &mut Budget) -> Result<ClaudeFiles, Coverage> {
    if !budget.claim_file() {
        return Err(Coverage::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the typed Claude store itself must be a real directory, never a symlink"
    )]
    let metadata = std::fs::symlink_metadata(store)
        .map_err(|_| Coverage::Unreadable("Claude store unreadable".to_owned()))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(Coverage::Unreadable(
            "Claude store is not a regular directory".to_owned(),
        ));
    }
    let projects = store.join("projects");
    let dirs = child_dirs(&projects, budget)?;
    let mut parent: Option<TranscriptFile> = None;
    let mut sidechains: Vec<TranscriptFile> = Vec::new();
    let mut observed_at = None;
    for project in dirs {
        let main = project.join(format!("{id}.jsonl"));
        if let Some(found) = locate_transcript(&main, budget)? {
            if parent.is_some() {
                return Err(Coverage::Unreadable(
                    "conversation id is not unique".to_owned(),
                ));
            }
            observed_at = newer(observed_at, found.modified.and_then(epoch));
            parent = Some(found);
        }
        let subagents = project.join(id).join("subagents");
        for path in child_files(&subagents, budget)? {
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some(found) = locate_transcript(&path, budget)? {
                observed_at = newer(observed_at, found.modified.and_then(epoch));
                sidechains.push(found);
            }
        }
    }
    let Some(parent) = parent else {
        return Ok(None);
    };
    let mut transcript_budget = TranscriptBudget::new();
    let parent = parse_transcript(&parent, &mut transcript_budget)?;
    let mut parsed_sidechains = Vec::new();
    for sidechain in sidechains {
        parsed_sidechains.push(parse_transcript(&sidechain, &mut transcript_budget)?);
    }
    Ok(Some((parent, parsed_sidechains, observed_at)))
}

fn locate_transcript(path: &Path, budget: &mut Budget) -> Result<Option<TranscriptFile>, Coverage> {
    if !budget.claim_file() {
        return Err(Coverage::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: usage lstat validates an agent-owned transcript before opening"
    )]
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Coverage::Unreadable("transcript unreadable".to_owned())),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(Coverage::Unreadable(
            "transcript is not a regular file".to_owned(),
        ));
    }
    let modified = metadata.modified().ok();
    Ok(Some(TranscriptFile {
        path: path.to_owned(),
        metadata,
        modified,
    }))
}

fn parse_transcript(
    transcript: &TranscriptFile,
    budget: &mut TranscriptBudget,
) -> Result<claude::Parsed, Coverage> {
    if !budget.claim(transcript.metadata.len()) {
        return Err(Coverage::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: streams only the lstat-checked agent transcript"
    )]
    let file = File::open(&transcript.path)
        .map_err(|_| Coverage::Unreadable("transcript unreadable".to_owned()))?;
    let opened = file
        .metadata()
        .map_err(|_| Coverage::Unreadable("transcript unreadable".to_owned()))?;
    if !opened.file_type().is_file()
        || opened.len() != transcript.metadata.len()
        || !same_file(&transcript.metadata, &opened)
    {
        return Err(Coverage::Unreadable(
            "transcript changed during read".to_owned(),
        ));
    }
    let mut reader = BufReader::new(file.take(opened.len()));
    let mut line = Vec::new();
    let mut parser = claude::Parser::default();
    while let Some(usable) = read_capped_line(&mut reader, &mut line)
        .map_err(|_| Coverage::Unreadable("transcript unreadable".to_owned()))?
    {
        if usable {
            parser.push(&line);
        }
    }
    if reader.into_inner().limit() != 0 {
        return Err(Coverage::Unreadable(
            "transcript changed during read".to_owned(),
        ));
    }
    Ok(parser.finish())
}

fn read_capped_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
) -> std::io::Result<Option<bool>> {
    line.clear();
    let mut overlong = false;
    let mut saw_bytes = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok((saw_bytes || overlong).then_some(!overlong));
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(buffer.len(), |at| at + 1);
        let body = newline.map_or(buffer, |at| &buffer[..at]);
        saw_bytes |= !body.is_empty();
        if !overlong {
            if line.len().saturating_add(body.len()) > CLAUDE_LINE_MAX_BYTES {
                line.clear();
                overlong = true;
            } else {
                line.extend_from_slice(body);
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(!overlong));
        }
    }
}

fn child_dirs(root: &Path, budget: &mut Budget) -> Result<Vec<PathBuf>, Coverage> {
    child_paths(root, budget, true)
}

fn child_files(root: &Path, budget: &mut Budget) -> Result<Vec<PathBuf>, Coverage> {
    child_paths(root, budget, false)
}

fn child_paths(
    root: &Path,
    budget: &mut Budget,
    directories: bool,
) -> Result<Vec<PathBuf>, Coverage> {
    if !budget.claim_file() {
        return Err(Coverage::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: bounded usage enumeration under a validated client store"
    )]
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(Coverage::Unreadable("directory unreadable".to_owned())),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(Coverage::Unreadable(
            "directory is not a regular directory".to_owned(),
        ));
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: bounded usage enumeration under a validated client store"
    )]
    let listing = std::fs::read_dir(root)
        .map_err(|_| Coverage::Unreadable("directory unreadable".to_owned()))?;
    let mut paths = Vec::new();
    for entry in listing {
        if !budget.claim_file() {
            return Err(Coverage::Truncated);
        }
        let entry =
            entry.map_err(|_| Coverage::Unreadable("directory entry unreadable".to_owned()))?;
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: usage lstat refuses symlinked client entries"
        )]
        let metadata = std::fs::symlink_metadata(entry.path())
            .map_err(|_| Coverage::Unreadable("directory entry unreadable".to_owned()))?;
        let wanted = if directories {
            metadata.file_type().is_dir()
        } else {
            metadata.file_type().is_file()
        };
        if wanted && !metadata.file_type().is_symlink() {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

type RegularRead = Option<(Vec<u8>, Option<SystemTime>)>;

fn read_regular(path: &Path, cap: u64, budget: &mut Budget) -> Result<RegularRead, Coverage> {
    if !budget.claim_file() {
        return Err(Coverage::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: usage lstat validates a bounded transcript before opening"
    )]
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Coverage::Unreadable("transcript unreadable".to_owned())),
    };
    if metadata.len() > cap {
        return Err(Coverage::Truncated);
    }
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(Coverage::Unreadable(
            "transcript is not a regular file".to_owned(),
        ));
    }
    if !budget.reserve_bytes(metadata.len()) {
        return Err(Coverage::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: opens only a bounded lstat-validated usage transcript"
    )]
    let file =
        File::open(path).map_err(|_| Coverage::Unreadable("transcript unreadable".to_owned()))?;
    let opened = file
        .metadata()
        .map_err(|_| Coverage::Unreadable("transcript unreadable".to_owned()))?;
    if !opened.file_type().is_file()
        || opened.len() != metadata.len()
        || !same_file(&metadata, &opened)
    {
        return Err(Coverage::Unreadable(
            "transcript changed during read".to_owned(),
        ));
    }
    let mut bytes = Vec::new();
    file.take(opened.len())
        .read_to_end(&mut bytes)
        .map_err(|_| Coverage::Unreadable("transcript unreadable".to_owned()))?;
    let actual = u64::try_from(bytes.len()).unwrap_or(opened.len());
    if actual != opened.len() {
        return Err(Coverage::Unreadable(
            "transcript changed during read".to_owned(),
        ));
    }
    budget.refund_bytes(opened.len().saturating_sub(actual));
    if budget.expired() {
        return Err(Coverage::Truncated);
    }
    Ok(Some((bytes, metadata.modified().ok())))
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

fn valid_id(id: Option<&str>) -> Option<String> {
    let id = id?;
    let canonical = crate::archive::canonical_uuid(id);
    (canonical == id).then_some(canonical)
}

fn epoch(time: SystemTime) -> Option<i64> {
    let seconds = time.duration_since(UNIX_EPOCH).ok()?.as_secs();
    i64::try_from(seconds).ok()
}

fn newer(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    left.into_iter().chain(right).max()
}

#[derive(Debug, Clone, Copy)]
struct RowFacts {
    tool: ToolKind,
    observed_at: Option<i64>,
    retired: bool,
    approximate: bool,
}

fn priced_row(
    entry: &RosterEntry,
    model: String,
    tokens: Tokens,
    facts: RowFacts,
    book: &prices::Book,
    unpriced: &mut Vec<String>,
) -> SeatUsage {
    let usd_micro = book
        .price(&model)
        .and_then(|price| prices::cost(tokens, price));
    if usd_micro.is_none() && !unpriced.iter().any(|known| known == &model) {
        unpriced.push(model.clone());
    }
    SeatUsage {
        seat: if facts.retired {
            format!("{} (retired)", entry.name)
        } else {
            entry.name.clone()
        },
        slot: entry.slot.clone(),
        tool: facts.tool.as_str().to_owned(),
        model,
        tokens,
        usd_micro,
        observed_at: facts.observed_at,
        retired: facts.retired,
        coverage: Coverage::Read,
        approximate: facts.approximate,
    }
}

fn unreadable(entry: &RosterEntry, tool: ToolKind, reason: &str, retired: bool) -> SeatUsage {
    coverage_row(
        entry,
        tool,
        Coverage::Unreadable(reason.to_owned()),
        retired,
    )
}

fn coverage_row(
    entry: &RosterEntry,
    tool: ToolKind,
    coverage: Coverage,
    retired: bool,
) -> SeatUsage {
    SeatUsage {
        seat: if retired {
            format!("{} (retired)", entry.name)
        } else {
            entry.name.clone()
        },
        slot: entry.slot.clone(),
        tool: tool.as_str().to_owned(),
        model: "?".to_owned(),
        tokens: Tokens::default(),
        usd_micro: None,
        observed_at: None,
        retired,
        coverage,
        approximate: false,
    }
}

fn unsupported(entry: &RosterEntry, tool: ToolKind, retired: bool) -> SeatUsage {
    let mut row = coverage_row(entry, tool, Coverage::Unsupported, retired);
    "n/a".clone_into(&mut row.model);
    row
}

fn total_of(seats: &[SeatUsage]) -> UsageTotal {
    let mut total = UsageTotal::default();
    for seat in seats {
        if seat.coverage == Coverage::Read {
            total.tokens = total.tokens.saturating_add(seat.tokens);
            match seat.usd_micro {
                Some(cost) => total.usd_micro = total.usd_micro.saturating_add(cost),
                None => total.partial = true,
            }
        } else if !matches!(seat.coverage, Coverage::Unsupported) {
            total.partial = true;
        }
    }
    total
}

fn add_retired(
    dir: &Path,
    inputs: &Inputs<'_>,
    budget: &mut Budget,
    seats: &mut Vec<SeatUsage>,
    unpriced: &mut Vec<String>,
) -> bool {
    let events = dir.join(crate::store::EVENTS);
    let bytes = match read_regular(&events, EVENTS_MAX_BYTES, budget) {
        Ok(Some((bytes, _))) => bytes,
        Err(Coverage::Truncated) => return true,
        Ok(None) | Err(_) => return false,
    };
    let text = String::from_utf8_lossy(&bytes);
    for line in text.lines() {
        let Ok(event) = crate::events::Event::parse_line(line) else {
            continue;
        };
        if event.action != "retire" {
            continue;
        }
        let Some(name) = event
            .target
            .filter(|name| crate::config::is_agent_name(name))
        else {
            continue;
        };
        let slot = event
            .target_slot
            .value()
            .filter(|slot| crate::requests::is_slot(slot))
            .map(str::to_owned);
        let fields = event.summary.as_deref().and_then(retired_fields);
        let reference = event
            .reference
            .filter(|id| crate::archive::canonical_uuid(id) == *id);
        let Some((tool, profile, config_home, config_home_base)) = fields else {
            seats.push(unlocated_retired(&name, slot.as_deref().unwrap_or("?")));
            continue;
        };
        let Some(reference) = reference else {
            seats.push(unlocated_retired(&name, slot.as_deref().unwrap_or("?")));
            continue;
        };
        let Some(slot) = slot else {
            seats.push(unlocated_retired(&name, "?"));
            continue;
        };
        let entry = RosterEntry {
            slot,
            name,
            profile: Some(profile),
            harness_session: Some(reference),
            config_home: if config_home.is_empty() {
                RecordedConfigHome::Missing
            } else {
                RecordedConfigHome::parse(&config_home)
            },
            config_home_base: if config_home_base.is_empty() {
                RecordedConfigHomeBase::Missing
            } else {
                RecordedConfigHomeBase::parse(&config_home_base)
            },
            binary: Some(tool),
        };
        let mut seat_budget = Budget::new();
        seats.extend(observe_entry(
            &entry,
            inputs,
            &mut seat_budget,
            unpriced,
            true,
        ));
    }
    false
}

fn retired_fields(summary: &str) -> Option<(String, String, String, String)> {
    let rest = summary.strip_prefix("tool=")?;
    let (tool, rest) = rest.split_once(" profile=")?;
    let (profile, rest) = rest.split_once(" config_home=")?;
    let (config_home, config_home_base) = rest.rsplit_once(" config_home_base=")?;
    if ToolKind::from_known_binary_name(tool).is_none()
        || !crate::config::is_config_key(profile)
        || [tool, profile, config_home, config_home_base]
            .into_iter()
            .any(|value| value.chars().any(char::is_control))
    {
        return None;
    }
    Some((
        tool.to_owned(),
        profile.to_owned(),
        config_home.to_owned(),
        config_home_base.to_owned(),
    ))
}

fn unlocated_retired(name: &str, slot: &str) -> SeatUsage {
    SeatUsage {
        seat: format!("{name} (retired)"),
        slot: slot.to_owned(),
        tool: "?".to_owned(),
        model: "?".to_owned(),
        tokens: Tokens::default(),
        usd_micro: None,
        observed_at: None,
        retired: true,
        coverage: Coverage::Unlocated,
        approximate: false,
    }
}

/// Render the observation as a human table or stable JSON document.
#[must_use]
pub fn render(observation: &Observation, json: bool) -> String {
    if json {
        render_json(observation)
    } else {
        render_table(observation)
    }
}

/// Observe and render already-selected session inputs.
///
/// # Errors
///
/// Returns an I/O error when the supplied output stream cannot be written.
pub fn run(inputs: &Inputs<'_>, json: bool, out: &mut impl std::io::Write) -> crate::Result<u8> {
    write!(out, "{}", render(&observe(inputs), json))?;
    Ok(0)
}

enum TableLine {
    Cells(Vec<String>),
    Note(String),
}

fn render_table(observation: &Observation) -> String {
    let mut lines = vec![TableLine::Cells(
        [
            "session", "seat", "tool", "model", "in", "cache-w", "cache-r", "out", "usd", "age",
        ]
        .map(str::to_owned)
        .into(),
    )];
    for session in &observation.sessions {
        for seat in &session.seats {
            lines.push(TableLine::Cells(seat_cells(
                &session.name,
                seat,
                observation.now,
            )));
        }
        lines.push(TableLine::Cells(total_cells(&session.name, &session.total)));
        if session.retired_scan_truncated {
            lines.push(TableLine::Note(
                "retired seats: unread (events scan truncated)".to_owned(),
            ));
        }
    }
    let fleet = observation
        .sessions
        .iter()
        .fold(UsageTotal::default(), |mut total, session| {
            total.tokens = total.tokens.saturating_add(session.total.tokens);
            total.usd_micro = total.usd_micro.saturating_add(session.total.usd_micro);
            total.partial |= session.total.partial;
            total
        });
    lines.push(TableLine::Cells(total_cells("fleet", &fleet)));
    aligned_table(lines)
}

fn seat_cells(session: &str, seat: &SeatUsage, now: i64) -> Vec<String> {
    let (input, write, read, output) = token_cells(seat);
    let usd = seat.usd_micro.map_or_else(
        || {
            if seat.coverage == Coverage::Unsupported {
                "n/a".to_owned()
            } else {
                "?".to_owned()
            }
        },
        dollars,
    );
    let observed = if seat.coverage == Coverage::Unsupported {
        "n/a".to_owned()
    } else {
        seat.observed_at
            .map_or_else(|| "?".to_owned(), |at| age(now.saturating_sub(at)))
    };
    let model = if seat.approximate {
        format!("~{}", seat.model)
    } else {
        seat.model.clone()
    };
    vec![
        session.to_owned(),
        seat.seat.clone(),
        seat.tool.clone(),
        model,
        input,
        write,
        read,
        output,
        usd,
        observed,
    ]
}

fn total_cells(label: &str, total: &UsageTotal) -> Vec<String> {
    let marker = if total.partial { " (partial)" } else { "" };
    vec![
        label.to_owned(),
        "total".to_owned(),
        "-".to_owned(),
        "-".to_owned(),
        human(total.tokens.input),
        human(total.tokens.cache_write),
        human(total.tokens.cache_read),
        human(total.tokens.output),
        format!("{}{marker}", dollars(total.usd_micro)),
        "-".to_owned(),
    ]
}

fn aligned_table(lines: Vec<TableLine>) -> String {
    let mut widths = [0_usize; 10];
    for row in lines.iter().filter_map(|line| match line {
        TableLine::Cells(row) => Some(row),
        TableLine::Note(_) => None,
    }) {
        for (column, value) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(column) {
                *width = (*width).max(value.chars().count());
            }
        }
    }
    let mut out = String::new();
    for line in lines {
        match line {
            TableLine::Cells(row) => {
                for (column, value) in row.iter().enumerate() {
                    if column > 0 {
                        out.push_str("  ");
                    }
                    out.push_str(value);
                    if column + 1 < row.len() {
                        out.extend(std::iter::repeat_n(
                            ' ',
                            widths
                                .get(column)
                                .copied()
                                .unwrap_or_default()
                                .saturating_sub(value.chars().count()),
                        ));
                    }
                }
                out.push('\n');
            }
            TableLine::Note(note) => {
                out.push_str(&note);
                out.push('\n');
            }
        }
    }
    out
}

fn token_cells(seat: &SeatUsage) -> (String, String, String, String) {
    match seat.coverage {
        Coverage::Read => (
            human(seat.tokens.input),
            human(seat.tokens.cache_write),
            human(seat.tokens.cache_read),
            human(seat.tokens.output),
        ),
        Coverage::Unsupported => (
            "n/a".to_owned(),
            "n/a".to_owned(),
            "n/a".to_owned(),
            "n/a".to_owned(),
        ),
        Coverage::Unreadable(_) | Coverage::Unlocated | Coverage::Truncated => (
            "?".to_owned(),
            "?".to_owned(),
            "?".to_owned(),
            "?".to_owned(),
        ),
    }
}

fn human(value: u64) -> String {
    if value >= 1_000_000 {
        let tenths = (u128::from(value) + 50_000) / 100_000;
        format!("{}.{:01}M", tenths / 10, tenths % 10)
    } else if value >= 1_000 {
        format!("{}k", value / 1_000)
    } else {
        value.to_string()
    }
}

fn dollars(micro: u64) -> String {
    let cents = (u128::from(micro) + 5_000) / 10_000;
    format!("{}.{:02}", cents / 100, cents % 100)
}

fn age(delta: i64) -> String {
    if delta < 60 {
        format!("{}s", delta.max(0))
    } else if delta < 3_600 {
        format!("{}m", delta / 60)
    } else {
        format!("{}h", delta / 3_600)
    }
}

fn render_json(observation: &Observation) -> String {
    let sessions = observation
        .sessions
        .iter()
        .map(|session| {
            let seats = session
                .seats
                .iter()
                .map(|seat| {
                    crate::json::Value::obj([
                        ("seat", crate::json::Value::str(&seat.seat)),
                        ("slot", crate::json::Value::str(&seat.slot)),
                        ("tool", crate::json::Value::str(&seat.tool)),
                        ("model", crate::json::Value::str(&seat.model)),
                        ("input_tokens", json_u64(seat.tokens.input)),
                        ("cache_write_tokens", json_u64(seat.tokens.cache_write)),
                        ("cache_read_tokens", json_u64(seat.tokens.cache_read)),
                        ("output_tokens", json_u64(seat.tokens.output)),
                        (
                            "usd_micro",
                            seat.usd_micro.map_or(crate::json::Value::Null, json_u64),
                        ),
                        (
                            "observed_at",
                            seat.observed_at
                                .map_or(crate::json::Value::Null, crate::json::Value::Num),
                        ),
                        (
                            "coverage",
                            crate::json::Value::str(coverage_name(&seat.coverage)),
                        ),
                        (
                            "coverage_reason",
                            match &seat.coverage {
                                Coverage::Unreadable(reason) => crate::json::Value::str(reason),
                                _ => crate::json::Value::Null,
                            },
                        ),
                        ("retired", crate::json::Value::Bool(seat.retired)),
                        ("approximate", crate::json::Value::Bool(seat.approximate)),
                    ])
                })
                .collect();
            crate::json::Value::obj([
                ("name", crate::json::Value::str(&session.name)),
                ("seats", crate::json::Value::Arr(seats)),
                ("total_input_tokens", json_u64(session.total.tokens.input)),
                (
                    "total_cache_write_tokens",
                    json_u64(session.total.tokens.cache_write),
                ),
                (
                    "total_cache_read_tokens",
                    json_u64(session.total.tokens.cache_read),
                ),
                ("total_output_tokens", json_u64(session.total.tokens.output)),
                ("total_usd_micro", json_u64(session.total.usd_micro)),
                ("partial", crate::json::Value::Bool(session.total.partial)),
                (
                    "retired_scan_truncated",
                    crate::json::Value::Bool(session.retired_scan_truncated),
                ),
            ])
        })
        .collect();
    let fleet = observation
        .sessions
        .iter()
        .fold(UsageTotal::default(), |mut total, session| {
            total.tokens = total.tokens.saturating_add(session.total.tokens);
            total.usd_micro = total.usd_micro.saturating_add(session.total.usd_micro);
            total.partial |= session.total.partial;
            total
        });
    let root = crate::json::Value::obj([
        ("now", crate::json::Value::Num(observation.now)),
        ("sessions", crate::json::Value::Arr(sessions)),
        ("fleet_input_tokens", json_u64(fleet.tokens.input)),
        (
            "fleet_cache_write_tokens",
            json_u64(fleet.tokens.cache_write),
        ),
        ("fleet_cache_read_tokens", json_u64(fleet.tokens.cache_read)),
        ("fleet_output_tokens", json_u64(fleet.tokens.output)),
        ("fleet_usd_micro", json_u64(fleet.usd_micro)),
        ("fleet_partial", crate::json::Value::Bool(fleet.partial)),
        (
            "unpriced",
            crate::json::Value::Arr(
                observation
                    .unpriced
                    .iter()
                    .map(crate::json::Value::str)
                    .collect(),
            ),
        ),
    ]);
    format!("{}\n", root.render())
}

fn json_u64(value: u64) -> crate::json::Value {
    i64::try_from(value).map_or_else(
        |_| crate::json::Value::Raw(value.to_string()),
        crate::json::Value::Num,
    )
}

fn coverage_name(coverage: &Coverage) -> &'static str {
    match coverage {
        Coverage::Read => "read",
        Coverage::Unreadable(_) => "unreadable",
        Coverage::Unsupported => "unsupported",
        Coverage::Unlocated => "unlocated",
        Coverage::Truncated => "truncated",
    }
}
