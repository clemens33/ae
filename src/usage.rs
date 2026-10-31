//! Offline API-equivalent spend from agent-owned transcripts.

pub mod claude;
pub mod codex;
pub mod hourly;
pub mod prices;

use std::fs::File;
use std::io::{BufRead, BufReader, Read as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::meta::{
    RecordedClient, RecordedConfigHome, RecordedConfigHomeBase, RecordedWorkDir, RosterEntry,
};
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
    pub hourly: bool,
}

/// Parse `ae usage [session…] [--hourly] [--json]` without reading the world.
///
/// # Errors
///
/// Returns the first duplicate, unknown option, or invalid session name.
pub fn parse_args(tail: &[String]) -> Result<Args, String> {
    let mut parsed = Args::default();
    for word in tail {
        match word.as_str() {
            "--json" if !parsed.json => parsed.json = true,
            "--hourly" if !parsed.hourly => parsed.hourly = true,
            "--json" | "--hourly" => return Err(word.clone()),
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
    pub legacy_retired_unlocated: usize,
    pub meta_scan_failure: Option<String>,
    pub retired_scan_failure: Option<String>,
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
        let (mut seats, meta_scan_failure) = match read_meta(&input.path, &mut meta_budget) {
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
                        None,
                    ));
                }
                (seats, None)
            }
            Err(coverage) => (Vec::new(), Some(scan_failure_reason(coverage))),
        };
        let mut retired_budget = Budget::new();
        let retired_scan = add_retired(
            &input.path,
            inputs,
            &mut retired_budget,
            &mut seats,
            &mut unpriced,
        );
        let mut total = total_of(&seats);
        if (seats.is_empty() && retired_scan.legacy_unlocated == 0)
            || retired_scan.truncated
            || meta_scan_failure.is_some()
            || retired_scan.failure.is_some()
        {
            total.partial = true;
        }
        sessions.push(SessionUsage {
            name: input.name.clone(),
            seats,
            total,
            retired_scan_truncated: retired_scan.truncated,
            legacy_retired_unlocated: retired_scan.legacy_unlocated,
            meta_scan_failure,
            retired_scan_failure: retired_scan.failure,
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

fn claim_transcript(left: Option<&mut u64>, bytes: u64) -> bool {
    let Some(left) = left else {
        return true;
    };
    if bytes > *left {
        return false;
    }
    *left -= bytes;
    true
}

/// Host inventory without a liveness probe; every read uses the existing doors.
pub(crate) fn profile_seats(
    inputs: &crate::quota::Inputs<'_>,
    mut transcript: u64,
    budget: &mut Budget,
) -> crate::profile_view::Audit {
    use crate::profile_view::{Audit, Seat};
    let mut audit = Audit::default();
    let Some(root) = inputs.sessions else {
        return audit;
    };
    let inventory = profile_inventory(root, budget, &mut audit.gaps);
    let book = match prices::read(inputs.global, inputs.local) {
        Ok(book) => book,
        Err(error) => {
            audit
                .gaps
                .push(format!("prices unreadable: {error}; totals partial"));
            prices::Book::default()
        }
    };
    let usage_inputs = Inputs {
        home: inputs.home,
        sessions: &[],
        prices: &book,
        now: inputs.now,
    };
    let mut skipped = 0_usize;
    for (path, session, meta, _) in inventory {
        let mut candidates = profile_current(&meta, inputs.now);
        match read_regular(&path.join(crate::store::EVENTS), 16 * 1024 * 1024, budget) {
            Ok(Some((bytes, _))) => {
                profile_retired(&bytes, &session, &mut candidates, &mut audit);
            }
            Err(coverage) => audit.gaps.push(format!(
                "{session}: journal skipped {}; budget or read failure; totals partial",
                scan_failure_reason(coverage)
            )),
            Ok(None) => {}
        }
        candidates.sort_by(|a, b| a.0.slot.cmp(&b.0.slot).then(a.3.cmp(&b.3)));
        let mut identities = std::collections::BTreeSet::new();
        for (entry, lifetime, drift, retired) in candidates {
            if !identities.insert((
                entry.slot.clone(),
                entry.name.clone(),
                entry.harness_session.clone(),
                retired,
            )) {
                audit.gaps.push(format!(
                    "{session}/{}: duplicate seat; totals partial",
                    entry.name
                ));
                continue;
            }
            let Some(profile) = entry.profile.clone() else {
                audit.gaps.push(format!(
                    "{session}/{}: unattributed seat=1; totals partial",
                    entry.name
                ));
                continue;
            };
            if audit.seats.len() >= 256 {
                skipped += 1;
                audit
                    .gaps
                    .push(format!("{session}/{}: not read (seat budget)", entry.name));
                continue;
            }
            let source = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or(""))
                .adapter()
                .usage
                .source;
            let rows = observe_entry(
                &entry,
                &usage_inputs,
                &mut Budget::local(16 * 1024 * 1024),
                &mut Vec::new(),
                retired,
                Some(&mut transcript),
            );
            audit.seats.push(Seat {
                profile,
                session: session.clone(),
                name: entry.name,
                slot: entry.slot,
                source,
                rows,
                drift,
                lifetime,
                retired,
            });
        }
    }
    if skipped > 0 {
        audit.gaps.push(format!(
            "skipped {skipped} seats: seat budget; totals partial"
        ));
    }
    audit
}

fn profile_current(meta: &crate::meta::Meta, now: i64) -> Vec<ProfileCandidate> {
    meta.roster()
        .iter()
        .cloned()
        .map(|entry| {
            let lifetime = meta
                .launch_epoch(&entry.slot)
                .filter(|at| *at <= now)
                .and_then(|at| u64::try_from(now - at).ok());
            let drift = meta.observed_model(&entry.slot).map(str::to_owned);
            (entry, lifetime, drift, false)
        })
        .collect()
}

type ProfileInventory = Vec<(PathBuf, String, crate::meta::Meta, Option<i64>)>;

fn profile_inventory(root: &Path, budget: &mut Budget, gaps: &mut Vec<String>) -> ProfileInventory {
    let Ok(Bounded::Ready(paths)) = crate::quota::session_paths(root, budget) else {
        gaps.push(
            "sessions skipped: directory unreadable or file budget; totals partial (at least)"
                .to_owned(),
        );
        return Vec::new();
    };
    let mut inventory = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        match read_regular(
            &path.join(crate::meta::FILE),
            SESSION_META_MAX_BYTES,
            budget,
        ) {
            Ok(Some((bytes, _))) => {
                let Ok(text) = std::str::from_utf8(&bytes) else {
                    gaps.push(format!(
                        "{name}: damaged meta=1 (not UTF-8); totals partial"
                    ));
                    continue;
                };
                let meta = crate::meta::Meta::parse(text);
                if !meta.anomalies().is_empty() {
                    gaps.push(format!(
                        "{name}: damaged meta={} anomalies; totals partial",
                        meta.anomalies().len()
                    ));
                }
                let live = crate::inventory::last_live_with_meta_evidence(
                    &path,
                    crate::inventory::launch_epochs(&bytes),
                );
                let stamp = match live {
                    crate::tmux::Evidence::At(at) => Some(at),
                    _ => None,
                };
                inventory.push((path, name, meta, stamp));
            }
            Err(Coverage::Truncated) => gaps.push(format!(
                "{name}: meta not read (budget or size cap); skipped, totals partial"
            )),
            Err(coverage) => gaps.push(format!(
                "{name}: damaged meta=1 {}; skipped, totals partial",
                scan_failure_reason(coverage)
            )),
            Ok(None) => gaps.push(format!("{name}: damaged meta=1 missing; totals partial")),
        }
    }
    inventory.sort_by(|a, b| b.3.cmp(&a.3).then(a.1.cmp(&b.1)));
    if inventory.len() > 128 {
        gaps.push(format!(
            "skipped {} sessions: session budget; totals partial",
            inventory.len() - 128
        ));
        for (_, name, _, _) in &inventory[128..] {
            gaps.push(format!("{name}: not read (session budget)"));
        }
        inventory.truncate(128);
    }
    inventory
}

type ProfileCandidate = (RosterEntry, Option<u64>, Option<String>, bool);

fn profile_retired(
    bytes: &[u8],
    session: &str,
    seats: &mut Vec<ProfileCandidate>,
    audit: &mut crate::profile_view::Audit,
) {
    let mut malformed = 0_usize;
    let mut unattributed = 0_usize;
    let mut spawns: Vec<(String, String, i64, usize)> = Vec::new();
    for line in String::from_utf8_lossy(bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let Ok(event) = crate::events::Event::parse_line(line) else {
            malformed += 1;
            continue;
        };
        if !matches!(event.action.as_str(), "spawn" | "retire") {
            continue;
        }
        let name = event
            .target
            .as_deref()
            .filter(|name| crate::config::is_agent_name(name));
        let slot = event
            .target_slot
            .value()
            .filter(|slot| crate::requests::is_slot(slot));
        if event.action == "spawn" {
            if let (Some(name), Some(slot)) = (name, slot) {
                if let Some(found) = spawns.iter_mut().find(|row| row.0 == slot && row.1 == name) {
                    found.3 += 1;
                } else {
                    spawns.push((slot.to_owned(), name.to_owned(), event.ts.epoch(), 1));
                }
            }
            continue;
        }
        let fields = event.summary.as_deref().and_then(retired_fields);
        let (Some(name), Some((tool, profile, home, base))) = (name, fields) else {
            unattributed += 1;
            continue;
        };
        let lifetime = slot
            .and_then(|slot| spawns.iter_mut().find(|row| row.0 == slot && row.1 == name))
            .and_then(|row| {
                let elapsed = (row.3 == 1 && event.ts.epoch() >= row.2)
                    .then(|| u64::try_from(event.ts.epoch() - row.2).ok())
                    .flatten();
                row.3 += 1;
                elapsed
            });
        if lifetime.is_none() {
            audit.notes.push(format!(
                "{session}/{name}: retired lifetime unknown=1 (missing, reused or overlapping pair)"
            ));
        }
        let id = event
            .reference
            .filter(|id| crate::archive::is_canonical_uuid(id));
        let entry = RosterEntry {
            slot: slot.unwrap_or("?").to_owned(),
            name: name.to_owned(),
            profile: Some(profile),
            client: RecordedClient::Missing,
            harness_session: id,
            config_home: if home.is_empty() {
                RecordedConfigHome::Missing
            } else {
                RecordedConfigHome::parse(&home)
            },
            config_home_base: if base.is_empty() {
                RecordedConfigHomeBase::Missing
            } else {
                RecordedConfigHomeBase::parse(&base)
            },
            binary: Some(tool),
            work_dir: RecordedWorkDir::Missing,
        };
        seats.push((entry, lifetime, None, true));
    }
    if malformed > 0 {
        audit.gaps.push(format!(
            "{session}: malformed journal lines={malformed}; totals partial"
        ));
    }
    if unattributed > 0 {
        audit.gaps.push(format!(
            "{session}: unattributed retire records={unattributed}; totals partial"
        ));
    }
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
    transcript: Option<&mut u64>,
) -> Vec<SeatUsage> {
    let tool = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or(""));
    match tool.adapter().usage.source {
        UsageSource::ClaudeTranscripts => {
            observe_claude(entry, tool, inputs, budget, unpriced, retired, transcript)
        }
        UsageSource::CodexRollout => {
            vec![observe_codex(
                entry, tool, inputs, budget, unpriced, retired, transcript,
            )]
        }
        UsageSource::Unsupported => vec![unsupported(entry, tool, retired)],
    }
}

pub(crate) fn source_for(
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
    transcript: Option<&mut u64>,
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
    let found = match claude_files(&store, &id, budget, transcript) {
        Ok(found) => found,
        Err(coverage) => return vec![coverage_row(entry, tool, coverage, retired)],
    };
    let Some((parent, sidechains, observed_at, truncated)) = found else {
        return vec![unreadable(entry, tool, "transcript not found", retired)];
    };
    let models = claude::reduce(&parent, &sidechains);
    if models.is_empty() {
        if truncated {
            return vec![coverage_row(entry, tool, Coverage::Truncated, retired)];
        }
        return vec![unreadable(entry, tool, "transcript has no usage", retired)];
    }
    models
        .into_iter()
        .map(|model| {
            let mut row = priced_row(
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
            );
            if truncated {
                row.coverage = Coverage::Truncated;
                row.approximate = true;
            }
            row
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
    transcript: Option<&mut u64>,
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
    let needed = file.len().min(CODEX_TAIL_BYTES)
        + if file.len() > CODEX_TAIL_BYTES {
            file.len().min(CODEX_HEAD_BYTES)
        } else {
            0
        };
    if !claim_transcript(transcript, needed) {
        return unreadable(entry, tool, "not read (budget)", retired);
    }
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
    if !parsed.has_token_count() {
        return if parsed.malformed_token_count() {
            unreadable(entry, tool, "malformed token usage", retired)
        } else if head.is_empty() {
            unreadable(entry, tool, "rollout has no token usage", retired)
        } else {
            coverage_row(entry, tool, Coverage::Truncated, retired)
        };
    }
    match parsed.last_model {
        Some(model) => priced_row(
            entry,
            model,
            parsed.tokens,
            RowFacts {
                tool,
                observed_at,
                retired,
                approximate: parsed.approximate || parsed.model_changed,
            },
            inputs.prices,
            unpriced,
        ),
        None => unreadable(entry, tool, "rollout has no model", retired),
    }
}

type ClaudeFiles = Option<(claude::Parsed, Vec<claude::Parsed>, Option<i64>, bool)>;

pub(crate) struct TranscriptFile {
    pub(crate) path: PathBuf,
    pub(crate) metadata: std::fs::Metadata,
    pub(crate) modified: Option<SystemTime>,
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

/// What the transcript walk found: the parent every consumer needs, plus the
/// sidechains only usage prices.
struct LocatedClaude {
    parent: Option<TranscriptFile>,
    sidechains: Vec<TranscriptFile>,
    observed_at: Option<i64>,
}

fn claude_files(
    store: &Path,
    id: &str,
    budget: &mut Budget,
    transcript: Option<&mut u64>,
) -> Result<ClaudeFiles, Coverage> {
    let located = locate_claude(store, id, budget, true)?;
    let LocatedClaude {
        parent,
        sidechains,
        observed_at,
    } = located;
    let Some(parent) = parent else {
        return Ok(None);
    };
    let bytes = sidechains.iter().fold(parent.metadata.len(), |sum, file| {
        sum.saturating_add(file.metadata.len())
    });
    if transcript.is_some()
        && (bytes > CLAUDE_SEAT_MAX_BYTES || !claim_transcript(transcript, bytes))
    {
        return Err(Coverage::Unreadable(
            "not read (budget or seat cap)".to_owned(),
        ));
    }
    let mut transcript_budget = TranscriptBudget::new();
    let (parent, mut truncated) = parse_transcript(&parent, &mut transcript_budget)?;
    let mut parsed_sidechains = Vec::new();
    for sidechain in sidechains {
        let (parsed, sidechain_truncated) = parse_transcript(&sidechain, &mut transcript_budget)?;
        truncated |= sidechain_truncated;
        parsed_sidechains.push(parsed);
    }
    Ok(Some((parent, parsed_sidechains, observed_at, truncated)))
}

/// Locate one seat's parent Claude transcript: the LOCATE half of
/// [`claude_files`], shared with the message board, which streams the located
/// file through its own door. Returns the located file with its lstat metadata
/// so the reader can prove the file it opens is the file that was found.
pub(crate) fn locate_claude_parent(
    store: &Path,
    id: &str,
    budget: &mut Budget,
) -> Result<Option<TranscriptFile>, Coverage> {
    // Skip `subagents/` entirely: subagent transcripts carry the parent's
    // prompts as `user` turns and would masquerade as the human on the board.
    locate_claude(store, id, budget, false).map(|located| located.parent)
}

/// The walk [`claude_files`] and [`locate_claude_parent`] share. With
/// sidechains it is usage's walk byte for byte — same claims, same order,
/// same errors; without them it never descends into `subagents/`.
fn locate_claude(
    store: &Path,
    id: &str,
    budget: &mut Budget,
    want_sidechains: bool,
) -> Result<LocatedClaude, Coverage> {
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
        if !want_sidechains {
            continue;
        }
        let Some(conversation) = derived_directory(&project, id, budget)? else {
            continue;
        };
        let Some(subagents) = derived_directory(&conversation, "subagents", budget)? else {
            continue;
        };
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
    Ok(LocatedClaude {
        parent,
        sidechains,
        observed_at,
    })
}

fn derived_directory(
    parent: &Path,
    name: &str,
    budget: &mut Budget,
) -> Result<Option<PathBuf>, Coverage> {
    let path = parent.join(name);
    if path.parent() != Some(parent) || !budget.claim_file() {
        return Err(Coverage::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: every derived usage directory is lstat-checked before enumeration"
    )]
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Coverage::Unreadable("directory unreadable".to_owned())),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(Coverage::Unreadable(
            "directory is not a regular directory".to_owned(),
        ));
    }
    Ok(Some(path))
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

fn checked_transcript(path: &Path, metadata: &std::fs::Metadata) -> Result<File, Coverage> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: streams only the lstat-checked agent transcript"
    )]
    let file =
        File::open(path).map_err(|_| Coverage::Unreadable("transcript unreadable".to_owned()))?;
    let opened = file
        .metadata()
        .map_err(|_| Coverage::Unreadable("transcript unreadable".to_owned()))?;
    if !opened.file_type().is_file()
        || opened.len() < metadata.len()
        || !same_file(metadata, &opened)
    {
        return Err(Coverage::Unreadable(
            "transcript changed during read".to_owned(),
        ));
    }
    Ok(file)
}

fn parse_transcript(
    transcript: &TranscriptFile,
    budget: &mut TranscriptBudget,
) -> Result<(claude::Parsed, bool), Coverage> {
    if !budget.claim(transcript.metadata.len()) {
        return Err(Coverage::Truncated);
    }
    let file = checked_transcript(&transcript.path, &transcript.metadata)?;
    let mut reader = BufReader::new(file.take(transcript.metadata.len()));
    let mut line = Vec::new();
    let mut parser = claude::Parser::default();
    let mut truncated = false;
    while let Some(usable) = read_capped_line(&mut reader, &mut line)
        .map_err(|_| Coverage::Unreadable("transcript unreadable".to_owned()))?
    {
        if usable {
            parser.push(&line);
        } else {
            truncated = true;
        }
    }
    if reader.into_inner().limit() != 0 {
        return Err(Coverage::Unreadable(
            "transcript changed during read".to_owned(),
        ));
    }
    Ok((parser.finish(), truncated))
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

pub(crate) fn valid_id(id: Option<&str>) -> Option<String> {
    id.filter(|id| crate::archive::is_canonical_uuid(id))
        .map(str::to_owned)
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
        match seat.coverage {
            Coverage::Read | Coverage::Truncated => {
                total.tokens = total.tokens.saturating_add(seat.tokens);
                match seat.usd_micro {
                    Some(cost) => total.usd_micro = total.usd_micro.saturating_add(cost),
                    None => total.partial = true,
                }
                total.partial |= matches!(seat.coverage, Coverage::Truncated);
            }
            Coverage::Unreadable(_) | Coverage::Unlocated => total.partial = true,
            Coverage::Unsupported => {}
        }
    }
    total
}

#[derive(Debug, Clone, Default)]
struct RetiredScan {
    truncated: bool,
    legacy_unlocated: usize,
    failure: Option<String>,
}

fn add_retired(
    dir: &Path,
    inputs: &Inputs<'_>,
    budget: &mut Budget,
    seats: &mut Vec<SeatUsage>,
    unpriced: &mut Vec<String>,
) -> RetiredScan {
    let events = dir.join(crate::store::EVENTS);
    let bytes = match read_regular(&events, EVENTS_MAX_BYTES, budget) {
        Ok(Some((bytes, _))) => bytes,
        Err(Coverage::Truncated) => {
            return RetiredScan {
                truncated: true,
                legacy_unlocated: 0,
                failure: None,
            };
        }
        Ok(None) => return RetiredScan::default(),
        Err(coverage) => {
            return RetiredScan {
                failure: Some(scan_failure_reason(coverage)),
                ..RetiredScan::default()
            };
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut scan = RetiredScan::default();
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
        if event.reference.is_none() && event.summary.is_none() {
            scan.legacy_unlocated = scan.legacy_unlocated.saturating_add(1);
            continue;
        }
        let slot = event
            .target_slot
            .value()
            .filter(|slot| crate::requests::is_slot(slot))
            .map(str::to_owned);
        let fields = event.summary.as_deref().and_then(retired_fields);
        let reference = event
            .reference
            .filter(|id| crate::archive::is_canonical_uuid(id));
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
            // The retire summary predates the client row and names no
            // override; the recorded store below carries the attribution.
            client: RecordedClient::Missing,
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
            // A retired seat is attributed, not located: its target lives in
            // the retire event text, never in this struct.
            work_dir: RecordedWorkDir::Missing,
        };
        let mut seat_budget = Budget::new();
        seats.extend(observe_entry(
            &entry,
            inputs,
            &mut seat_budget,
            unpriced,
            true,
            None,
        ));
    }
    scan
}

fn scan_failure_reason(coverage: Coverage) -> String {
    match coverage {
        Coverage::Unreadable(reason) => reason,
        Coverage::Truncated => "scan truncated".to_owned(),
        Coverage::Read | Coverage::Unsupported | Coverage::Unlocated => {
            "scan coverage unavailable".to_owned()
        }
    }
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
        if let Some(reason) = &session.meta_scan_failure {
            lines.push(TableLine::Note(format!(
                "{}  seats: unread (session meta scan failed: {reason})",
                session.name
            )));
        }
        if let Some(reason) = &session.retired_scan_failure {
            lines.push(TableLine::Note(format!(
                "{}  retired seats: unread (events scan failed: {reason})",
                session.name
            )));
        }
        if session.legacy_retired_unlocated > 0 {
            lines.push(TableLine::Note(format!(
                "{}  retired: {} seats unlocated (legacy retire events)",
                session.name, session.legacy_retired_unlocated
            )));
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
    let sessions = observation.sessions.iter().map(session_json).collect();
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

fn session_json(session: &SessionUsage) -> crate::json::Value {
    crate::json::Value::obj([
        ("name", crate::json::Value::str(&session.name)),
        (
            "seats",
            crate::json::Value::Arr(session.seats.iter().map(seat_json).collect()),
        ),
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
        (
            "legacy_retired_unlocated",
            json_usize(session.legacy_retired_unlocated),
        ),
        (
            "meta_scan_failure",
            session
                .meta_scan_failure
                .as_ref()
                .map_or(crate::json::Value::Null, crate::json::Value::str),
        ),
        (
            "retired_scan_failure",
            session
                .retired_scan_failure
                .as_ref()
                .map_or(crate::json::Value::Null, crate::json::Value::str),
        ),
    ])
}

fn seat_json(seat: &SeatUsage) -> crate::json::Value {
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
}

fn json_u64(value: u64) -> crate::json::Value {
    i64::try_from(value).map_or_else(
        |_| crate::json::Value::Raw(value.to_string()),
        crate::json::Value::Num,
    )
}

fn json_usize(value: usize) -> crate::json::Value {
    i64::try_from(value).map_or(
        crate::json::Value::Raw(value.to_string()),
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

/// Roots and clock for the price-independent hourly observation.
pub struct HourlyInputs<'a> {
    pub home: Option<&'a Path>,
    pub sessions: &'a [SessionInput],
    pub now: i64,
}
/// Observe current conversations only; predecessor history stays excluded.
#[must_use]
pub fn observe_hourly(inputs: &HourlyInputs<'_>) -> hourly::Observation {
    let mut left = 512 * 1024 * 1024;
    let mut observation = hourly::Observation {
        now: inputs.now,
        sessions: Vec::new(),
        exclusions: Vec::new(),
    };
    let inventory = hourly_inventory(inputs, &mut observation.exclusions);
    for (session, entry, retired, source, key, prior) in &inventory {
        let codex = *source == UsageSource::CodexRollout;
        let conflict = key.is_some() && inventory.iter().filter(|r| r.4 == *key).count() > 1;
        let scan = if conflict {
            hourly::Scan::failed(32, codex)
        } else {
            let result = key
                .as_ref()
                .ok_or(Coverage::Unsupported)
                .and_then(|(root, id)| hourly_source(root, id, *source, &mut left, inputs.now));
            result.unwrap_or_else(|coverage| hourly::Scan::failed(hourly_code(&coverage), codex))
        };
        let reason = scan.failure();
        if let Some(reason) = &reason {
            observation
                .exclusions
                .push(format!("{session}:{}: {reason}", entry.name));
        }
        if *prior > 0 {
            observation.exclusions.push(format!(
                "{session}:{}: predecessors not read ({prior})",
                entry.name
            ));
        }
        let row = scan
            .finish(inputs.now)
            .seat(entry, *retired, *prior, reason, inputs.now);
        if observation.sessions.last().is_none_or(|r| r.0 != *session) {
            observation.sessions.push((session.clone(), Vec::new()));
        }
        if let Some((_, seats)) = observation.sessions.last_mut() {
            seats.push(row);
        }
        if *source != UsageSource::Unsupported {
            let caveat = if codex {
                "Codex internal subagents not read; inclusion unknown"
            } else {
                "Claude parent replay outside parent tail unmeasured"
            };
            observation.exclusions.push(caveat.to_owned());
        }
    }
    observation.exclusions.sort();
    observation.exclusions.dedup();
    observation
}
const HOURLY_TAIL: u64 = 64 * 1024 * 1024;
fn hourly_source(
    root: &Path,
    id: &str,
    source: UsageSource,
    left: &mut u64,
    now: i64,
) -> Result<hourly::Scan, Coverage> {
    let mut budget = Budget::new();
    let unreadable = || Coverage::Unreadable("unreadable transcript".to_owned());
    match source {
        UsageSource::CodexRollout => {
            let file = match crate::quota::find_codex_rollout(root, id, &mut budget) {
                Ok(Bounded::Ready(Some(file))) => file,
                Ok(Bounded::Truncated) => return Err(Coverage::Truncated),
                _ => return Err(unreadable()),
            };
            let (bytes, boundary, whole) = hourly_tail(file.path(), file.metadata(), left)?;
            Ok(hourly::parse(&bytes, boundary, whole, now, true))
        }
        UsageSource::ClaudeTranscripts => {
            let found = locate_claude(root, id, &mut budget, true)?;
            let parent = found.parent.ok_or_else(unreadable)?;
            let mut scan = {
                let (bytes, boundary, whole) = hourly_tail(&parent.path, &parent.metadata, left)?;
                hourly::parse(&bytes, boundary, whole, now, false)
            };
            let ids = scan.ids();
            let mut children = found.sidechains;
            children.sort_by(|a, b| a.path.cmp(&b.path));
            for child in children {
                let child = match hourly_tail(&child.path, &child.metadata, left) {
                    Ok((bytes, boundary, whole)) => {
                        hourly::parse(&bytes, boundary, whole, now, false)
                    }
                    Err(coverage) => hourly::Scan::failed(hourly_code(&coverage), false),
                };
                scan.merge(child, &ids);
            }
            Ok(scan)
        }
        UsageSource::Unsupported => Err(Coverage::Unsupported),
    }
}
fn hourly_tail(
    path: &Path,
    metadata: &std::fs::Metadata,
    left: &mut u64,
) -> Result<(Vec<u8>, bool, bool), Coverage> {
    use std::io::{Seek as _, SeekFrom};
    let unreadable = || Coverage::Unreadable("unreadable transcript".to_owned());
    let mut file = checked_transcript(path, metadata)?;
    let planned = metadata.len().min(HOURLY_TAIL);
    if !claim_transcript(Some(left), planned) {
        return Err(Coverage::Truncated);
    }
    let mut budget = Budget::local(HOURLY_TAIL);
    if !budget.reserve_bytes(planned) {
        return Err(unreadable());
    }
    file.seek(SeekFrom::Start(metadata.len() - planned))
        .map_err(|_| unreadable())?;
    let mut bytes = Vec::with_capacity(usize::try_from(planned).unwrap_or(0));
    file.take(planned)
        .read_to_end(&mut bytes)
        .map_err(|_| unreadable())?;
    if u64::try_from(bytes.len()).ok() != Some(planned) {
        return Err(unreadable());
    }
    let whole = metadata.len() <= HOURLY_TAIL;
    let boundary = whole || bytes.first() == Some(&b'\n');
    Ok((bytes, boundary, whole))
}

type HourlySeat = (
    String,
    RosterEntry,
    bool,
    UsageSource,
    Option<(PathBuf, String)>,
    usize,
);
fn hourly_inventory(inputs: &HourlyInputs<'_>, exclusions: &mut Vec<String>) -> Vec<HourlySeat> {
    let mut inventory = Vec::new();
    let mut sessions = inputs.sessions.iter().collect::<Vec<_>>();
    sessions.sort_by_key(|s| &s.name);
    for session in sessions {
        let mut budget = Budget::new();
        let Ok(meta) = read_meta(&session.path, &mut budget) else {
            exclusions.push(format!("{}: unreadable meta", session.name));
            continue;
        };
        let mut seats = profile_current(&meta, inputs.now);
        let mut audit = crate::profile_view::Audit::default();
        if let Ok(Some((bytes, _))) = read_regular(
            &session.path.join(crate::store::EVENTS),
            EVENTS_MAX_BYTES,
            &mut budget,
        ) {
            profile_retired(&bytes, &session.name, &mut seats, &mut audit);
        }
        exclusions.extend(audit.gaps);
        seats.sort_by(|a, b| (&a.0.slot, &a.0.name, a.3).cmp(&(&b.0.slot, &b.0.name, b.3)));
        for (entry, _, _, retired) in seats {
            let source = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or(""))
                .adapter()
                .usage
                .source;
            let key = source_for(&entry, source, inputs.home)
                .ok()
                .zip(valid_id(entry.harness_session.as_deref()));
            let prior = if retired {
                0
            } else {
                meta.harness_session_prior(&entry.slot).len()
            };
            if !inventory.iter().any(|r: &HourlySeat| {
                r.0 == session.name
                    && r.1.slot == entry.slot
                    && r.1.name == entry.name
                    && r.2 == retired
                    && r.4 == key
            }) {
                inventory.push((session.name.clone(), entry, retired, source, key, prior));
            }
        }
    }
    inventory
}

fn hourly_code(coverage: &Coverage) -> u8 {
    match coverage {
        Coverage::Truncated => 8,
        _ => 16,
    }
}

#[cfg(test)]
mod r3c_id_spec {
    #[test]
    fn r3c_valid_id_refuses_empty_and_preserves_uuid_grammar() {
        assert_eq!(super::valid_id(None), None);
        for id in [
            "",
            "not-a-uuid",
            "0199C0DE-1234-4890-abcd-ef0123456789",
            "0199c0de-1234-4890-abcd-ef012345678",
            "../0199c0de-1234-4890-abcd-ef0123456789",
        ] {
            assert_eq!(super::valid_id(Some(id)), None, "{id:?}");
        }
        let id = "0199c0de-1234-4890-abcd-ef0123456789";
        assert_eq!(super::valid_id(Some(id)).as_deref(), Some(id));
    }
}
