//! Vendor quota snapshots parsed from local client-owned caches.
//!
//! Parsing stays separate from discovery and rendering: the client files are
//! hostile persisted state, while this module is a pure bytes-to-rows boundary.

pub mod claude;
pub mod codex;

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::json::Value;
use crate::time::Timestamp;
use crate::tool::{QuotaSource, ToolKind};

const CLAUDE_MAX_BYTES: u64 = 4 * 1024 * 1024;
const CODEX_TAIL_BYTES: u64 = 1024 * 1024;
const QUOTA_MAX_FILES: usize = 4_096;
const QUOTA_MAX_BYTES: u64 = 16 * 1024 * 1024;
const QUOTA_MAX_ELAPSED: Duration = Duration::from_secs(2);
const TABLE_MAX_WIDTHS: [usize; 8] = [40, 35, 22, 6, 5, 9, 9, 20];

/// Maximum age of an observation that may be called fresh.
pub const FRESH_SECS: i64 = 15 * 60;

/// Future clock skew at which an observation becomes unknown.
pub const FUTURE_SKEW_SECS: i64 = 5 * 60;

/// Whether a parsed quota row is safe to use as current evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Observed no more than 15 minutes ago.
    Fresh,
    /// Observed more than 15 minutes ago, before its reset.
    Stale,
    /// Missing, expired, skewed, or otherwise not applicable.
    Unknown,
    /// The client exposes no usable local quota state.
    Unsupported,
    /// A present source could not be read or parsed.
    ReadError,
    /// The invocation-wide file, byte, or wall-clock budget was exhausted.
    Truncated,
}

impl Status {
    /// Stable spelling used in the operator table.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Unknown => "unknown",
            Self::Unsupported => "unsupported",
            Self::ReadError => "read-error",
            Self::Truncated => "truncated",
        }
    }
}

/// One vendor bucket and one of its windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Vendor bucket identifier (`weekly_all`, `codex_bengalfox`, and so on).
    pub bucket: String,
    /// Optional vendor qualifier: Claude model scope or Codex plan.
    pub qualifier: Option<String>,
    /// Window duration reported or defined by the vendor, in minutes.
    pub window_minutes: Option<u32>,
    /// Vendor utilization numeric literal, without a percent sign.
    pub used_percent: Option<String>,
    /// Window reset as Unix epoch seconds.
    pub resets_at: Option<i64>,
    /// Cache or rollout observation as Unix epoch seconds.
    pub observed_at: Option<i64>,
    /// Freshness and applicability verdict for this row.
    pub status: Status,
}

/// Parsing failed before a trustworthy snapshot could be produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Input was not valid UTF-8.
    Utf8,
    /// A complete record was not valid JSON.
    Json,
    /// A named quota container had an incompatible shape.
    Shape,
}

/// Derive a freshness verdict from the record's own clocks.
#[must_use]
pub const fn freshness(observed_at: Option<i64>, resets_at: Option<i64>, now: i64) -> Status {
    let (Some(observed), Some(reset)) = (observed_at, resets_at) else {
        return Status::Unknown;
    };
    if reset <= now || observed.saturating_sub(now) >= FUTURE_SKEW_SECS {
        return Status::Unknown;
    }
    if now.saturating_sub(observed) <= FRESH_SECS {
        Status::Fresh
    } else {
        Status::Stale
    }
}

/// Read one non-negative numeric JSON literal for later display.
pub(crate) fn percent(value: Option<&Value>) -> Option<String> {
    let literal = match value? {
        Value::Num(number) => number.to_string(),
        Value::Raw(raw) => raw.clone(),
        _ => return None,
    };
    literal
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map(|_| literal)
}

/// Read an integral JSON number as an epoch.
pub(crate) fn epoch(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Num(number) => Some(*number),
        Value::Raw(raw) => raw.parse().ok(),
        _ => None,
    }
}

/// Read a positive integral JSON number as minutes.
pub(crate) fn minutes(value: Option<&Value>) -> Option<u32> {
    let number = epoch(value)?;
    u32::try_from(number).ok().filter(|minutes| *minutes > 0)
}

/// Read the UTC RFC 3339 spellings present in vendor caches.
pub(crate) fn vendor_timestamp(text: &str) -> Option<i64> {
    let date = text.get(..19)?;
    let suffix = text.get(19..)?;
    let utc = suffix == "Z"
        || suffix == "+00:00"
        || suffix.strip_prefix('.').is_some_and(|tail| {
            tail.strip_suffix('Z')
                .or_else(|| tail.strip_suffix("+00:00"))
                .is_some_and(|fraction| {
                    !fraction.is_empty() && fraction.bytes().all(|byte| byte.is_ascii_digit())
                })
        });
    if !utc {
        return None;
    }
    let mut canonical = String::with_capacity(20);
    canonical.push_str(date);
    canonical.push('Z');
    Timestamp::parse(&canonical).map(Timestamp::epoch)
}

/// World facts selected by the command entry before quota reads vendor state.
pub struct Inputs<'a> {
    /// The operator's home, where default client config homes live.
    pub home: Option<&'a Path>,
    /// The working directory used to resolve a relative config-home assignment.
    pub cwd: &'a Path,
    /// Selected global ae config.
    pub global: Option<&'a Path>,
    /// Selected project-local ae config.
    pub local: Option<&'a Path>,
    /// Current ae session meta, when invoked through a helper or from a pane.
    pub meta: Option<&'a crate::meta::Meta>,
    /// Observation instant.
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Scope {
    tool: ToolKind,
    home: Option<PathBuf>,
    profiles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Group {
    profiles: Vec<String>,
    tool: ToolKind,
    home: Option<PathBuf>,
    rollout: Option<String>,
    seat: Option<String>,
    rows: Vec<Row>,
    hint: Option<&'static str>,
}

enum ReadRows {
    Rows(Vec<Row>),
    Missing,
    Failed,
    Truncated,
}

enum Bounded<T> {
    Ready(T),
    Truncated,
}

struct Budget {
    files_left: usize,
    bytes_left: u64,
    started: Instant,
    max_elapsed: Duration,
}

impl Budget {
    fn new() -> Self {
        Self {
            files_left: QUOTA_MAX_FILES,
            bytes_left: QUOTA_MAX_BYTES,
            started: Instant::now(),
            max_elapsed: QUOTA_MAX_ELAPSED,
        }
    }

    fn expired(&self) -> bool {
        self.started.elapsed() >= self.max_elapsed
    }

    fn claim_file(&mut self) -> bool {
        if self.expired() || self.files_left == 0 {
            return false;
        }
        self.files_left -= 1;
        true
    }

    fn reserve_bytes(&mut self, bytes: u64) -> bool {
        if self.expired() || bytes > self.bytes_left {
            return false;
        }
        self.bytes_left -= bytes;
        true
    }

    fn refund_bytes(&mut self, bytes: u64) {
        self.bytes_left = self.bytes_left.saturating_add(bytes);
    }
}

/// Read configured client scopes and print the local quota table.
///
/// The operation is deliberately observational: it opens only client cache
/// files, writes only to the supplied streams, and never starts a process.
///
/// # Errors
///
/// Returns an I/O error only when the supplied output stream cannot be written.
pub fn run(inputs: &Inputs<'_>, out: &mut impl Write, err: &mut impl Write) -> crate::Result<u8> {
    let cfg = match crate::config::read_identity(inputs.global, inputs.local) {
        Ok(cfg) => cfg,
        Err(error) => {
            writeln!(err, "{error}")?;
            return Ok(1);
        }
    };
    let scopes = configured_scopes(&cfg, inputs.home, inputs.cwd);
    let mut groups = Vec::new();
    let mut budget = Budget::new();
    for scope in &scopes {
        let quota = scope.tool.adapter().quota;
        match quota.source {
            QuotaSource::ClaudeCache => groups.push(Group {
                profiles: scope.profiles.clone(),
                tool: scope.tool,
                home: scope.home.clone(),
                rollout: None,
                seat: None,
                rows: rows_or_placeholder(read_claude(scope, inputs.home, inputs.now, &mut budget)),
                hint: None,
            }),
            QuotaSource::CodexRollouts => {
                groups.extend(codex_groups(scope, inputs.meta, inputs.now, &mut budget));
            }
            QuotaSource::Unsupported => groups.push(Group {
                profiles: scope.profiles.clone(),
                tool: scope.tool,
                home: scope.home.clone(),
                rollout: None,
                seat: None,
                rows: vec![placeholder(Status::Unsupported)],
                hint: quota.unsupported_hint,
            }),
        }
    }
    write!(out, "{}", render_at(&groups, inputs.home, inputs.now))?;
    Ok(0)
}

fn configured_scopes(
    cfg: &crate::config::IdentityConfig,
    home: Option<&Path>,
    cwd: &Path,
) -> Vec<Scope> {
    let mut scopes: Vec<Scope> = Vec::new();
    for (profile, command) in &cfg.profiles {
        let tool = ToolKind::from_cmd(command);
        let config_home = config_home(tool, command, home, cwd);
        if let Some(scope) = scopes
            .iter_mut()
            .find(|scope| scope.tool == tool && scope.home == config_home)
        {
            scope.profiles.push(profile.clone());
        } else {
            scopes.push(Scope {
                tool,
                home: config_home,
                profiles: vec![profile.clone()],
            });
        }
    }
    scopes
}

fn config_home(tool: ToolKind, command: &str, home: Option<&Path>, cwd: &Path) -> Option<PathBuf> {
    let quota = tool.adapter().quota;
    if ambiguous_config_prefix(command, quota.config_home_env, home) {
        return None;
    }
    let path = quota
        .default_home
        .and_then(|name| home.map(|base| base.join(name)))?;
    Some(if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    })
}

fn ambiguous_config_prefix(command: &str, variable: Option<&str>, home: Option<&Path>) -> bool {
    let Some(split) = crate::launch_cmd::split_binary(command) else {
        return true;
    };
    let Ok(words) = crate::words::split_words(&split.prefix, &|name| {
        (name == "HOME")
            .then(|| home.map(|path| path.to_string_lossy().into_owned()))
            .flatten()
    }) else {
        return true;
    };
    words.iter().any(|word| {
        matches!(word.value.as_str(), "-i" | "-u")
            || (word.assignment
                && word
                    .value
                    .split_once('=')
                    .is_some_and(|(name, _)| Some(name) == variable))
    })
}

fn read_claude(
    scope: &Scope,
    operator_home: Option<&Path>,
    now: i64,
    budget: &mut Budget,
) -> ReadRows {
    let Some(home) = scope.home.as_deref() else {
        return ReadRows::Missing;
    };
    let path = claude_cache_path(home, operator_home);
    let bytes = match bounded_whole_file(&path, CLAUDE_MAX_BYTES, budget) {
        Ok(Bounded::Ready(Some(bytes))) => bytes,
        Ok(Bounded::Ready(None)) => return ReadRows::Missing,
        Ok(Bounded::Truncated) => return ReadRows::Truncated,
        Err(_) => return ReadRows::Failed,
    };
    match claude::parse(&bytes, now) {
        Ok(Some(snapshot)) if !snapshot.rows.is_empty() => ReadRows::Rows(snapshot.rows),
        Ok(_) => ReadRows::Missing,
        Err(_) => ReadRows::Failed,
    }
}

fn claude_cache_path(config_home: &Path, operator_home: Option<&Path>) -> PathBuf {
    if operator_home.is_some_and(|home| config_home == home.join(".claude")) {
        config_home.with_extension("json")
    } else {
        config_home.join(".claude.json")
    }
}

fn codex_groups(
    scope: &Scope,
    meta: Option<&crate::meta::Meta>,
    now: i64,
    budget: &mut Budget,
) -> Vec<Group> {
    let mut groups = Vec::new();
    if let Some(meta) = meta {
        for seat in meta.roster() {
            if !seat
                .profile
                .as_ref()
                .is_some_and(|profile| scope.profiles.contains(profile))
            {
                continue;
            }
            let rollout = seat.harness_session.clone();
            if rollout.as_ref().is_some_and(|id| {
                groups
                    .iter()
                    .any(|group: &Group| group.rollout.as_ref() == Some(id))
            }) {
                continue;
            }
            let rows = rollout.as_deref().map_or(ReadRows::Missing, |id| {
                read_codex(scope.home.as_deref(), id, now, budget)
            });
            groups.push(Group {
                profiles: scope.profiles.clone(),
                tool: scope.tool,
                home: scope.home.clone(),
                rollout,
                seat: Some(seat.name.clone()),
                rows: rows_or_placeholder(rows),
                hint: None,
            });
        }
    }
    if groups.is_empty() {
        groups.push(Group {
            profiles: scope.profiles.clone(),
            tool: scope.tool,
            home: scope.home.clone(),
            rollout: None,
            seat: None,
            rows: vec![placeholder(Status::Unknown)],
            hint: None,
        });
    }
    groups
}

fn read_codex(home: Option<&Path>, id: &str, now: i64, budget: &mut Budget) -> ReadRows {
    let Some(home) = home else {
        return ReadRows::Missing;
    };
    if crate::archive::canonical_uuid(id) != id {
        return ReadRows::Failed;
    }
    let path = match find_codex_rollout(&home.join("sessions"), id, budget) {
        Ok(Bounded::Ready(Some(path))) => path,
        Ok(Bounded::Ready(None)) => return ReadRows::Missing,
        Ok(Bounded::Truncated) => return ReadRows::Truncated,
        Err(_) => return ReadRows::Failed,
    };
    let (bytes, starts_at_boundary) = match bounded_tail(&path, CODEX_TAIL_BYTES, budget) {
        Ok(Bounded::Ready(tail)) => tail,
        Ok(Bounded::Truncated) => return ReadRows::Truncated,
        Err(_) => return ReadRows::Failed,
    };
    match codex::parse(&bytes, starts_at_boundary, now) {
        Ok(rows) if !rows.is_empty() => ReadRows::Rows(rows),
        Ok(_) => ReadRows::Missing,
        Err(_) => ReadRows::Failed,
    }
}

fn rows_or_placeholder(read: ReadRows) -> Vec<Row> {
    match read {
        ReadRows::Rows(rows) => rows,
        ReadRows::Missing => vec![placeholder(Status::Unknown)],
        ReadRows::Failed => vec![placeholder(Status::ReadError)],
        ReadRows::Truncated => vec![placeholder(Status::Truncated)],
    }
}

fn placeholder(status: Status) -> Row {
    Row {
        bucket: "-".to_owned(),
        qualifier: None,
        window_minutes: None,
        used_percent: None,
        resets_at: None,
        observed_at: None,
        status,
    }
}

fn scope_label(group: &Group, home: Option<&Path>) -> String {
    let path = group
        .home
        .as_deref()
        .map_or_else(|| "unknown".to_owned(), |path| short_path(path, home));
    let mut label = format!("{} · {path}", group.tool.as_str());
    if group.tool.adapter().quota.source == QuotaSource::CodexRollouts {
        label.push_str(" · unidentified");
        if let Some(seat) = group.seat.as_deref() {
            let _ = write!(label, " ({seat})");
        }
    }
    label
}

fn short_path(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(home)
    {
        if rest.as_os_str().is_empty() {
            return "~".to_owned();
        }
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

fn render_at(groups: &[Group], home: Option<&Path>, now: i64) -> String {
    const HEADER: [&str; 8] = [
        "PROFILES", "SCOPE", "BUCKET", "WINDOW", "USED", "RESETS", "OBSERVED", "STATUS",
    ];
    let mut table: Vec<[String; 8]> = vec![HEADER.map(str::to_owned)];
    for group in groups {
        for (index, row) in group.rows.iter().enumerate() {
            let trustworthy = matches!(row.status, Status::Fresh | Status::Stale);
            let status = group.hint.map_or_else(
                || row.status.as_str().to_owned(),
                |hint| format!("{} ({hint})", row.status.as_str()),
            );
            table.push([
                if index == 0 {
                    profiles_label(&group.profiles)
                } else {
                    String::new()
                },
                if index == 0 {
                    scope_label(group, home)
                } else {
                    String::new()
                },
                bucket_label(row),
                trustworthy
                    .then(|| row.window_minutes.map(window_label))
                    .flatten()
                    .unwrap_or_else(|| "-".to_owned()),
                trustworthy
                    .then(|| row.used_percent.as_deref().map(percent_label))
                    .flatten()
                    .unwrap_or_else(|| "-".to_owned()),
                trustworthy
                    .then(|| row.resets_at.map(|reset| reset_label(reset, now)))
                    .flatten()
                    .unwrap_or_else(|| "-".to_owned()),
                trustworthy
                    .then(|| row.observed_at.map(|observed| age_label(now - observed)))
                    .flatten()
                    .unwrap_or_else(|| "-".to_owned()),
                status,
            ]);
        }
    }
    render_table(&table)
}

fn render_table(table: &[[String; 8]]) -> String {
    let mut widths = [0_usize; 8];
    for row in table {
        for (column, value) in row.iter().enumerate() {
            widths[column] = widths[column]
                .max(value.chars().count())
                .min(TABLE_MAX_WIDTHS[column]);
        }
    }
    let mut out = String::new();
    for row in table {
        let wrapped: Vec<Vec<String>> = row
            .iter()
            .enumerate()
            .map(|(column, value)| wrap_cell(value, widths[column]))
            .collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for line in 0..height {
            let line_start = out.len();
            for (column, values) in wrapped.iter().enumerate() {
                if column > 0 {
                    out.push_str("  ");
                }
                let value = values.get(line).map_or("", String::as_str);
                out.push_str(value);
                if column + 1 < row.len() {
                    out.extend(std::iter::repeat_n(
                        ' ',
                        widths[column] - value.chars().count(),
                    ));
                }
            }
            let line_end = out.trim_end().len().max(line_start);
            out.truncate(line_end);
            out.push('\n');
        }
    }
    out
}

fn profiles_label(profiles: &[String]) -> String {
    let joined = profiles.join(" ");
    if joined.chars().count() <= TABLE_MAX_WIDTHS[0] {
        return joined;
    }
    format!(
        "{} profiles: {}",
        profiles.len(),
        profiles
            .iter()
            .take(3)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn wrap_cell(value: &str, width: usize) -> Vec<String> {
    if value.is_empty() || width == 0 {
        return vec![String::new()];
    }
    let mut rest: Vec<char> = value.chars().collect();
    let mut lines = Vec::new();
    while rest.len() > width {
        let cut = rest[..=width]
            .iter()
            .rposition(|ch| ch.is_whitespace())
            .filter(|cut| *cut > 0)
            .unwrap_or(width);
        lines.push(rest[..cut].iter().collect());
        rest.drain(..cut);
        while rest.first().is_some_and(|ch| ch.is_whitespace()) {
            rest.remove(0);
        }
    }
    lines.push(rest.iter().collect());
    lines
}

fn bucket_label(row: &Row) -> String {
    match row.qualifier.as_deref() {
        Some(qualifier) if row.bucket == "weekly_scoped" => {
            format!("{} {qualifier}", row.bucket)
        }
        Some(qualifier) => format!("{} ({qualifier})", row.bucket),
        None => row.bucket.clone(),
    }
}

fn window_label(minutes: u32) -> String {
    match minutes {
        10_080 => "7d".to_owned(),
        minutes if minutes.is_multiple_of(1_440) => format!("{}d", minutes / 1_440),
        minutes if minutes.is_multiple_of(60) => format!("{}h", minutes / 60),
        minutes => format!("{minutes}m"),
    }
}

fn percent_label(value: &str) -> String {
    let trimmed = if value.contains('.') {
        value.trim_end_matches('0').trim_end_matches('.')
    } else {
        value
    };
    format!("{}%", if trimmed.is_empty() { "0" } else { trimmed })
}

fn reset_label(reset: i64, now: i64) -> String {
    format!("in {}", span_label(reset.saturating_sub(now)))
}

fn age_label(seconds: i64) -> String {
    format!("{} ago", span_label(seconds.max(0)))
}

fn span_label(seconds: i64) -> String {
    let minutes = seconds.max(0) / 60;
    let days = minutes / 1_440;
    let hours = (minutes % 1_440) / 60;
    let mins = minutes % 60;
    if days > 0 {
        if hours > 0 {
            format!("{days}d {hours}h")
        } else {
            format!("{days}d")
        }
    } else if hours > 0 {
        format!("{hours}h{mins:02}m")
    } else {
        format!("{mins}m")
    }
}

fn bounded_whole_file(
    path: &Path,
    cap: u64,
    budget: &mut Budget,
) -> io::Result<Bounded<Option<Vec<u8>>>> {
    if !budget.claim_file() {
        return Ok(Bounded::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: quota lstat refuses symlinks and oversized hostile client caches before opening them"
    )]
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Bounded::Ready(None));
        }
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() || metadata.len() > cap
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "quota cache is not a bounded regular file",
        ));
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: opens only the lstat-checked Claude quota cache, bounded again while reading"
    )]
    let file = File::open(path)?;
    let opened = file.metadata()?;
    if !opened.file_type().is_file() || !same_file(&metadata, &opened) || opened.len() > cap {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "quota cache changed identity or size before the bounded read",
        ));
    }
    if !budget.reserve_bytes(opened.len()) {
        return Ok(Bounded::Truncated);
    }
    let mut bytes = Vec::new();
    file.take(opened.len()).read_to_end(&mut bytes)?;
    let actual = u64::try_from(bytes.len()).unwrap_or(opened.len());
    budget.refund_bytes(opened.len().saturating_sub(actual));
    if budget.expired() {
        return Ok(Bounded::Truncated);
    }
    Ok(Bounded::Ready(Some(bytes)))
}

fn bounded_tail(
    path: &Path,
    cap: u64,
    budget: &mut Budget,
) -> io::Result<Bounded<(Vec<u8>, bool)>> {
    if cap == 0 || !budget.claim_file() {
        return Ok(Bounded::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: quota lstat refuses a symlink or non-file before opening one ae-owned Codex rollout"
    )]
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rollout is not a regular file",
        ));
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: opens only the exact rollout named by an ae-recorded harness session id"
    )]
    let mut file = File::open(path)?;
    let opened = file.metadata()?;
    if !opened.file_type().is_file() || !same_file(&metadata, &opened) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rollout changed identity before the bounded read",
        ));
    }
    read_bounded_tail(&mut file, opened.len(), cap, budget)
}

fn read_bounded_tail(
    file: &mut (impl Read + Seek),
    opened_len: u64,
    cap: u64,
    budget: &mut Budget,
) -> io::Result<Bounded<(Vec<u8>, bool)>> {
    let planned = opened_len.min(cap);
    if !budget.reserve_bytes(planned) {
        return Ok(Bounded::Truncated);
    }
    let starts_at_file = opened_len <= cap;
    if !starts_at_file {
        file.seek(io::SeekFrom::Start(opened_len - cap))?;
    }
    let mut raw = Vec::new();
    Read::by_ref(file).take(planned).read_to_end(&mut raw)?;
    let actual = u64::try_from(raw.len()).unwrap_or(planned);
    budget.refund_bytes(planned.saturating_sub(actual));
    if budget.expired() {
        return Ok(Bounded::Truncated);
    }
    if starts_at_file {
        return Ok(Bounded::Ready((raw, true)));
    }
    let starts_at_boundary = raw.first() == Some(&b'\n');
    let bytes = raw.get(1..).unwrap_or_default().to_vec();
    Ok(Bounded::Ready((bytes, starts_at_boundary)))
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

fn find_codex_rollout(
    root: &Path,
    id: &str,
    budget: &mut Budget,
) -> io::Result<Bounded<Option<PathBuf>>> {
    let Some(dir) = codex_rollout_dir(root, id) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recorded Codex id carries no usable start time",
        ));
    };
    if !budget.claim_file() {
        return Ok(Bounded::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: quota lstat classifies only the Codex day encoded by the ae-recorded session id"
    )]
    let metadata = match std::fs::symlink_metadata(&dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Bounded::Ready(None));
        }
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rollout root is not a directory",
        ));
    }
    let mut found = None;
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: bounded enumeration of one recorded Codex start day, selecting only its exact session id"
    )]
    let entries = std::fs::read_dir(&dir)?;
    for entry in entries {
        if !budget.claim_file() {
            return Ok(Bounded::Truncated);
        }
        let entry = entry?;
        let kind = entry.file_type()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(&format!("-{id}.jsonl")) {
            if !kind.is_file() || kind.is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "recorded rollout is not a regular file",
                ));
            }
            if found.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "recorded rollout id is not unique",
                ));
            }
            found = Some(entry.path());
        }
    }
    if budget.expired() {
        return Ok(Bounded::Truncated);
    }
    Ok(Bounded::Ready(found))
}

fn codex_rollout_dir(root: &Path, id: &str) -> Option<PathBuf> {
    if crate::archive::canonical_uuid(id) != id || id.as_bytes().get(14) != Some(&b'7') {
        return None;
    }
    let millis = u64::from_str_radix(&format!("{}{}", id.get(..8)?, id.get(9..13)?), 16).ok()?;
    let seconds = i64::try_from(millis / 1_000).ok()?;
    let timestamp = Timestamp::from_epoch(seconds).to_string();
    let (date, _) = timestamp.split_once('T')?;
    let mut fields = date.split('-');
    let (year, month, day) = (fields.next()?, fields.next()?, fields.next()?);
    if year.len() != 4 || fields.next().is_some() {
        return None;
    }
    Some(root.join(year).join(month).join(day))
}

#[cfg(test)]
mod tests {
    use super::{
        Bounded, Budget, CLAUDE_MAX_BYTES, FRESH_SECS, FUTURE_SKEW_SECS, ReadRows, Scope, Status,
        bounded_tail, bounded_whole_file, claude_cache_path, config_home, find_codex_rollout,
        freshness, percent_label, profiles_label, read_bounded_tail, read_claude, render_table,
        rows_or_placeholder, vendor_timestamp,
    };
    use crate::tool::ToolKind;

    #[test]
    fn freshness_boundaries_are_exact() {
        let now = 10_000;
        assert_eq!(
            freshness(Some(now - FRESH_SECS), Some(now + 1), now),
            Status::Fresh
        );
        assert_eq!(
            freshness(Some(now - FRESH_SECS - 1), Some(now + 1), now),
            Status::Stale
        );
        assert_eq!(freshness(Some(now), Some(now), now), Status::Unknown);
        assert_eq!(
            freshness(Some(now + FUTURE_SKEW_SECS), Some(now + 1_000), now),
            Status::Unknown
        );
        assert_eq!(
            freshness(Some(now + FUTURE_SKEW_SECS - 1), Some(now + 1_000), now),
            Status::Fresh
        );
        assert_eq!(freshness(None, Some(now + 1), now), Status::Unknown);
        assert_eq!(freshness(Some(now), None, now), Status::Unknown);
    }

    #[test]
    fn vendor_utc_timestamps_accept_the_two_observed_clients() {
        let expected = vendor_timestamp("2026-09-08T09:10:25Z");
        assert!(expected.is_some());
        assert_eq!(vendor_timestamp("2026-09-08T09:10:25.781Z"), expected);
        assert_eq!(
            vendor_timestamp("2026-09-08T09:10:25.759599+00:00"),
            expected
        );
        assert_eq!(vendor_timestamp("2026-09-08T09:10:25+02:00"), None);
    }

    #[test]
    fn uncertain_profile_prefixes_never_guess_a_config_home() {
        let home = std::path::Path::new("/users/c");
        assert_eq!(
            config_home(ToolKind::Claude, "claude", Some(home), home),
            Some(home.join(".claude"))
        );
        assert_eq!(
            config_home(
                ToolKind::Claude,
                "CLAUDE_CONFIG_DIR=\"$HOME/.claude-work\" claude",
                Some(home),
                home,
            ),
            None
        );
        assert_eq!(
            config_home(
                ToolKind::Codex,
                "env CODEX_HOME=relative codex",
                Some(home),
                std::path::Path::new("/work"),
            ),
            None
        );
        assert_eq!(
            config_home(
                ToolKind::Codex,
                "CODEX_HOME=/old env -u CODEX_HOME codex",
                Some(home),
                home,
            ),
            None
        );
        assert_eq!(
            config_home(ToolKind::Claude, "env -u OTHER claude", Some(home), home),
            None
        );
        assert_eq!(
            config_home(ToolKind::Claude, "env -i claude", Some(home), home),
            None
        );
        assert_eq!(
            config_home(ToolKind::Claude, "OTHER=1 claude", Some(home), home),
            Some(home.join(".claude"))
        );
    }

    #[test]
    fn default_and_custom_claude_homes_read_only_their_own_cache() {
        let root =
            std::path::PathBuf::from(format!("/tmp/ae-quota-claude-homes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let default = root.join(".claude");
        let custom = root.join(".claude-mic");
        std::fs::create_dir_all(&custom).expect("custom client home");
        let cache = |percent| {
            format!(
                "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":1000000,\"utilization\":{{\"limits\":[{{\"kind\":\"session\",\"percent\":{percent},\"resets_at\":\"1970-01-01T01:00:00Z\"}}]}}}}}}"
            )
        };
        std::fs::write(root.join(".claude.json"), cache(11)).expect("default cache");
        std::fs::write(custom.join(".claude.json"), cache(77)).expect("custom cache");
        assert_eq!(
            claude_cache_path(&default, Some(&root)),
            root.join(".claude.json")
        );
        assert_eq!(
            claude_cache_path(&custom, Some(&root)),
            custom.join(".claude.json")
        );
        let read = |home: std::path::PathBuf| {
            let scope = Scope {
                tool: ToolKind::Claude,
                home: Some(home),
                profiles: vec!["p".to_owned()],
            };
            match read_claude(&scope, Some(&root), 1_001, &mut Budget::new()) {
                ReadRows::Rows(rows) => rows[0].used_percent.clone(),
                _ => None,
            }
        };
        assert_eq!(read(default), Some("11".to_owned()));
        assert_eq!(read(custom), Some("77".to_owned()));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn percentages_trim_only_fractional_zeroes() {
        assert_eq!(percent_label("100"), "100%");
        assert_eq!(percent_label("12.0"), "12%");
        assert_eq!(percent_label("3.50"), "3.5%");
    }

    #[test]
    fn long_profile_lists_are_summarized_within_the_column_cap() {
        let profiles: Vec<_> = (0..50).map(|index| format!("profile-{index}")).collect();
        let label = profiles_label(&profiles);
        assert_eq!(
            label, "50 profiles: profile-0 profile-1 profile-2",
            "the first three profile names stay intact"
        );
        let rendered = render_table(&[[
            label,
            "scope".to_owned(),
            "bucket".to_owned(),
            "5h".to_owned(),
            "1%".to_owned(),
            "in 1h".to_owned(),
            "1m ago".to_owned(),
            "fresh".to_owned(),
        ]]);
        assert!(rendered.contains("profile-2"), "{rendered}");
        assert!(rendered.lines().all(|line| line.chars().count() <= 160));
    }

    #[cfg(unix)]
    #[test]
    fn quota_cache_reader_refuses_symlinks_and_oversized_regular_files() {
        use std::os::unix::fs::symlink;

        let dir = std::path::PathBuf::from(format!("/tmp/ae-quota-source-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("fixture directory");
        let regular = dir.join("regular");
        std::fs::write(&regular, b"{}").expect("regular source");
        let link = dir.join("link");
        symlink(&regular, &link).expect("source symlink");
        assert!(bounded_whole_file(&link, CLAUDE_MAX_BYTES, &mut Budget::new()).is_err());

        let oversized = dir.join("oversized");
        let file = std::fs::File::create(&oversized).expect("oversized source");
        file.set_len(CLAUDE_MAX_BYTES + 1)
            .expect("sparse oversized source");
        assert!(bounded_whole_file(&oversized, CLAUDE_MAX_BYTES, &mut Budget::new()).is_err());

        let rollouts = dir.join("sessions/2026/09/08");
        std::fs::create_dir_all(&rollouts).expect("rollout tree");
        let id = "01a08046-1974-7352-ade3-81a786200795";
        symlink(
            &regular,
            rollouts.join(format!("rollout-2026-09-08T09-00-00-{id}.jsonl")),
        )
        .expect("rollout symlink");
        assert!(find_codex_rollout(&dir.join("sessions"), id, &mut Budget::new()).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn codex_tail_defines_left_and_right_record_boundaries_within_its_cap() {
        let dir = std::path::PathBuf::from(format!("/tmp/ae-quota-tail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("tail fixture directory");
        let line = concat!(
            r#"{"timestamp":"2026-09-08T09:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":7,"window_minutes":300,"resets_at":1788861600}}}}"#,
            "\n"
        );
        let aligned = dir.join("aligned");
        std::fs::write(&aligned, format!("x\n{line}")).expect("aligned tail");
        let cap = u64::try_from(line.len() + 1).expect("bounded cap");
        let Bounded::Ready((bytes, starts)) =
            bounded_tail(&aligned, cap, &mut Budget::new()).expect("bounded aligned tail")
        else {
            panic!("aligned tail was truncated");
        };
        assert!(starts);
        assert_eq!(
            super::codex::parse(&bytes, starts, 1_788_858_600)
                .unwrap()
                .len(),
            1
        );

        let unaligned = dir.join("unaligned");
        std::fs::write(&unaligned, format!("abcdef\n{line}")).expect("unaligned tail");
        let cap = u64::try_from(line.len() + 4).expect("bounded cap");
        let Bounded::Ready((bytes, starts)) =
            bounded_tail(&unaligned, cap, &mut Budget::new()).expect("bounded unaligned tail")
        else {
            panic!("unaligned tail was truncated");
        };
        assert!(!starts);
        assert_eq!(
            super::codex::parse(&bytes, starts, 1_788_858_600)
                .unwrap()
                .len(),
            1
        );

        let growing = dir.join("growing");
        std::fs::write(&growing, vec![b'x'; 128]).expect("oversized record");
        let Bounded::Ready((bytes, starts)) =
            bounded_tail(&growing, 32, &mut Budget::new()).expect("bounded growing file")
        else {
            panic!("growing file was truncated by invocation budget");
        };
        assert!(bytes.len() <= 31);
        assert!(
            super::codex::parse(&bytes, starts, 1_788_858_600)
                .unwrap()
                .is_empty()
        );

        let eof = dir.join("eof");
        std::fs::write(&eof, line.trim_end()).expect("incomplete EOF record");
        let Bounded::Ready((bytes, starts)) =
            bounded_tail(&eof, 1_024, &mut Budget::new()).expect("bounded EOF tail")
        else {
            panic!("EOF file was truncated");
        };
        assert!(
            super::codex::parse(&bytes, starts, 1_788_858_600)
                .unwrap()
                .is_empty()
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn codex_tail_stays_bounded_when_the_open_file_grows_or_truncates() {
        let line = concat!(
            r#"{"timestamp":"2026-09-08T09:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":7,"window_minutes":300,"resets_at":1788861600}}}}"#,
            "\n"
        );
        let initial_len = u64::try_from(line.len()).expect("bounded fixture length");
        let mut grown = std::io::Cursor::new(format!("{line}incomplete growth").into_bytes());
        let Bounded::Ready((bytes, starts)) =
            read_bounded_tail(&mut grown, initial_len, 1_024, &mut Budget::new())
                .expect("bounded growth-race read")
        else {
            panic!("growth-race read was truncated");
        };
        assert_eq!(bytes.len(), line.len());
        assert_eq!(
            super::codex::parse(&bytes, starts, 1_788_858_600)
                .unwrap()
                .len(),
            1
        );

        let mut truncated = std::io::Cursor::new(Vec::new());
        let Bounded::Ready((bytes, starts)) =
            read_bounded_tail(&mut truncated, initial_len, 1_024, &mut Budget::new())
                .expect("bounded truncation-race read")
        else {
            panic!("truncation-race read was truncated");
        };
        assert!(bytes.is_empty());
        assert!(
            super::codex::parse(&bytes, starts, 1_788_858_600)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn every_invocation_budget_limit_has_an_explicit_truncated_row() {
        let mut budget = Budget {
            files_left: 0,
            bytes_left: 1,
            started: std::time::Instant::now(),
            max_elapsed: std::time::Duration::from_secs(1),
        };
        assert!(!budget.claim_file());
        budget.files_left = 1;
        assert!(!budget.reserve_bytes(2));
        budget.bytes_left = 2;
        budget.max_elapsed = std::time::Duration::ZERO;
        assert!(!budget.claim_file());
        assert_eq!(
            rows_or_placeholder(ReadRows::Truncated)[0].status,
            Status::Truncated
        );
    }
}
