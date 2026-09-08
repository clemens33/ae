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

use crate::json::Value;
use crate::time::Timestamp;
use crate::tool::{QuotaSource, ToolKind};

const CLAUDE_MAX_BYTES: u64 = 4 * 1024 * 1024;
const CODEX_TAIL_BYTES: u64 = 1024 * 1024;
const CODEX_MAX_ENTRIES: usize = 16_384;

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
    scope: String,
    rows: Vec<Row>,
    hint: Option<&'static str>,
}

enum ReadRows {
    Rows(Vec<Row>),
    Missing,
    Failed,
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
    for scope in &scopes {
        let quota = scope.tool.adapter().quota;
        match quota.source {
            QuotaSource::ClaudeCache => groups.push(Group {
                profiles: scope.profiles.clone(),
                scope: scope_label(scope, inputs.home, None),
                rows: rows_or_placeholder(read_claude(scope, inputs.now)),
                hint: None,
            }),
            QuotaSource::CodexRollouts => {
                groups.extend(codex_groups(scope, inputs.meta, inputs.home, inputs.now));
            }
            QuotaSource::Unsupported => groups.push(Group {
                profiles: scope.profiles.clone(),
                scope: scope_label(scope, inputs.home, None),
                rows: vec![placeholder(Status::Unsupported)],
                hint: quota.unsupported_hint,
            }),
        }
    }
    write!(out, "{}", render_at(&groups, inputs.now))?;
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
    let assigned = quota
        .config_home_env
        .and_then(|name| assigned_config_home(command, name, home));
    let path = assigned.or_else(|| {
        quota
            .default_home
            .and_then(|name| home.map(|base| base.join(name)))
    })?;
    Some(if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    })
}

fn assigned_config_home(command: &str, variable: &str, home: Option<&Path>) -> Option<PathBuf> {
    let split = crate::launch_cmd::split_binary(command)?;
    let words = crate::words::split_words(&split.prefix, &|name| {
        (name == "HOME")
            .then(|| home.map(|path| path.to_string_lossy().into_owned()))
            .flatten()
    })
    .ok()?;
    let mut value: Option<String> = None;
    for word in words {
        if word.assignment
            && let Some((name, next)) = word.value.split_once('=')
            && name == variable
        {
            // `_run` applies every collected unset before every collected set,
            // so an explicit assignment wins regardless of its position around
            // the one peeled `env` word. Scope keying must match that exec.
            value = Some(next.to_owned());
        }
    }
    value.map(PathBuf::from)
}

fn read_claude(scope: &Scope, now: i64) -> ReadRows {
    let Some(home) = scope.home.as_deref() else {
        return ReadRows::Missing;
    };
    let path = home.with_extension("json");
    let bytes = match bounded_whole_file(&path, CLAUDE_MAX_BYTES) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return ReadRows::Missing,
        Err(_) => return ReadRows::Failed,
    };
    match claude::parse(&bytes, now) {
        Ok(Some(snapshot)) if !snapshot.rows.is_empty() => ReadRows::Rows(snapshot.rows),
        Ok(_) => ReadRows::Missing,
        Err(_) => ReadRows::Failed,
    }
}

fn codex_groups(
    scope: &Scope,
    meta: Option<&crate::meta::Meta>,
    display_home: Option<&Path>,
    now: i64,
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
            let rows = seat
                .harness_session
                .as_deref()
                .map_or(ReadRows::Missing, |id| {
                    read_codex(scope.home.as_deref(), id, now)
                });
            groups.push(Group {
                profiles: scope.profiles.clone(),
                scope: scope_label(scope, display_home, Some(&seat.name)),
                rows: rows_or_placeholder(rows),
                hint: None,
            });
        }
    }
    if groups.is_empty() {
        groups.push(Group {
            profiles: scope.profiles.clone(),
            scope: scope_label(scope, display_home, None),
            rows: vec![placeholder(Status::Unknown)],
            hint: None,
        });
    }
    groups
}

fn read_codex(home: Option<&Path>, id: &str, now: i64) -> ReadRows {
    let Some(home) = home else {
        return ReadRows::Missing;
    };
    if crate::archive::canonical_uuid(id) != id {
        return ReadRows::Failed;
    }
    let path = match find_codex_rollout(&home.join("sessions"), id) {
        Ok(Some(path)) => path,
        Ok(None) => return ReadRows::Missing,
        Err(_) => return ReadRows::Failed,
    };
    let Ok((bytes, starts_at_boundary)) = bounded_tail(&path, CODEX_TAIL_BYTES) else {
        return ReadRows::Failed;
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

fn scope_label(scope: &Scope, home: Option<&Path>, seat: Option<&str>) -> String {
    let path = scope
        .home
        .as_deref()
        .map_or_else(|| "?".to_owned(), |path| short_path(path, home));
    let mut label = format!("{} · {path}", scope.tool.as_str());
    if scope.tool.adapter().quota.source == QuotaSource::CodexRollouts {
        label.push_str(" · unidentified");
        if let Some(seat) = seat {
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

fn render_at(groups: &[Group], now: i64) -> String {
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
                    group.profiles.join(" ")
                } else {
                    String::new()
                },
                if index == 0 {
                    group.scope.clone()
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
            widths[column] = widths[column].max(value.chars().count());
        }
    }
    let mut out = String::new();
    for row in table {
        for (column, value) in row.iter().enumerate() {
            if column > 0 {
                out.push_str("  ");
            }
            out.push_str(value);
            if column + 1 < row.len() {
                out.extend(std::iter::repeat_n(
                    ' ',
                    widths[column] - value.chars().count(),
                ));
            }
        }
        out.push('\n');
    }
    out
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

fn bounded_whole_file(path: &Path, cap: u64) -> io::Result<Option<Vec<u8>>> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: quota lstat refuses symlinks and oversized hostile client caches before opening them"
    )]
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
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
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > cap) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "quota cache grew beyond its byte bound",
        ));
    }
    Ok(Some(bytes))
}

fn bounded_tail(path: &Path, cap: u64) -> io::Result<(Vec<u8>, bool)> {
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
    let starts_at_boundary = metadata.len() <= cap;
    if !starts_at_boundary {
        file.seek(io::SeekFrom::Start(metadata.len() - cap))?;
    }
    let mut bytes = Vec::new();
    file.take(cap).read_to_end(&mut bytes)?;
    Ok((bytes, starts_at_boundary))
}

fn find_codex_rollout(root: &Path, id: &str) -> io::Result<Option<PathBuf>> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: quota lstat classifies the Codex dated-rollout root without following a link"
    )]
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rollout root is not a directory",
        ));
    }
    let mut visited = 0_usize;
    let mut found = None;
    find_rollout_at(root, id, 0, &mut visited, &mut found)?;
    Ok(found)
}

fn find_rollout_at(
    dir: &Path,
    id: &str,
    depth: usize,
    visited: &mut usize,
    found: &mut Option<PathBuf>,
) -> io::Result<()> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: bounded enumeration of Codex's year/month/day rollout tree, selecting only an ae-recorded id"
    )]
    let entries = std::fs::read_dir(dir)?;
    for entry in entries {
        *visited += 1;
        if *visited > CODEX_MAX_ENTRIES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "rollout search exceeded its entry bound",
            ));
        }
        let entry = entry?;
        let kind = entry.file_type()?;
        if depth < 3 && kind.is_dir() && !kind.is_symlink() {
            find_rollout_at(&entry.path(), id, depth + 1, visited, found)?;
        } else if depth == 3 {
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
                *found = Some(entry.path());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CLAUDE_MAX_BYTES, FRESH_SECS, FUTURE_SKEW_SECS, Status, bounded_whole_file, config_home,
        find_codex_rollout, freshness, percent_label, vendor_timestamp,
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
    fn profile_assignments_key_distinct_config_homes() {
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
            Some(home.join(".claude-work"))
        );
        assert_eq!(
            config_home(
                ToolKind::Codex,
                "env CODEX_HOME=relative codex",
                Some(home),
                std::path::Path::new("/work"),
            ),
            Some(std::path::PathBuf::from("/work/relative"))
        );
        assert_eq!(
            config_home(
                ToolKind::Codex,
                "CODEX_HOME=/old env -u CODEX_HOME codex",
                Some(home),
                home,
            ),
            Some(std::path::PathBuf::from("/old"))
        );
    }

    #[test]
    fn percentages_trim_only_fractional_zeroes() {
        assert_eq!(percent_label("100"), "100%");
        assert_eq!(percent_label("12.0"), "12%");
        assert_eq!(percent_label("3.50"), "3.5%");
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
        assert!(bounded_whole_file(&link, CLAUDE_MAX_BYTES).is_err());

        let oversized = dir.join("oversized");
        let file = std::fs::File::create(&oversized).expect("oversized source");
        file.set_len(CLAUDE_MAX_BYTES + 1)
            .expect("sparse oversized source");
        assert!(bounded_whole_file(&oversized, CLAUDE_MAX_BYTES).is_err());

        let rollouts = dir.join("sessions/2026/09/08");
        std::fs::create_dir_all(&rollouts).expect("rollout tree");
        let id = "11111111-2222-4333-8444-555555555555";
        symlink(
            &regular,
            rollouts.join(format!("rollout-2026-09-08T09-00-00-{id}.jsonl")),
        )
        .expect("rollout symlink");
        assert!(find_codex_rollout(&dir.join("sessions"), id).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
