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
use std::time::{Duration, Instant, SystemTime};

use crate::json::Value;
use crate::time::Timestamp;
use crate::tool::{QuotaSource, ToolKind};

const CLAUDE_MAX_BYTES: u64 = 4 * 1024 * 1024;
const CODEX_TAIL_BYTES: u64 = 256 * 1024;
const SESSION_META_MAX_BYTES: u64 = 1024 * 1024;
const QUOTA_MAX_FILES: usize = 4_096;
const QUOTA_MAX_BYTES: u64 = 16 * 1024 * 1024;
const QUOTA_MAX_ELAPSED: Duration = Duration::from_secs(2);
const CODEX_DISPLAY_ROLLOUTS: usize = 3;
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
    /// Selected global ae config.
    pub global: Option<&'a Path>,
    /// Selected project-local ae config.
    pub local: Option<&'a Path>,
    /// Canonical ae sessions directory whose durable metas name Codex rollouts.
    pub sessions: Option<&'a Path>,
    /// Observation instant.
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Scope {
    tool: ToolKind,
    home: Option<PathBuf>,
    source: Option<PathBuf>,
    source_key: Option<PathBuf>,
    profiles: Vec<String>,
    clients: Vec<String>,
    hint: Option<String>,
}

struct ScopePaths {
    home: Option<PathBuf>,
    source: Option<PathBuf>,
    source_key: Option<PathBuf>,
    hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Group {
    profiles: Vec<String>,
    tool: ToolKind,
    home: Option<PathBuf>,
    clients: Vec<String>,
    rollout: Option<String>,
    owner: Option<String>,
    rows: Vec<Row>,
    hint: Option<String>,
    summary: Option<RolloutSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RolloutSummary {
    hidden: usize,
    unreadable: usize,
    oldest_observed: Option<i64>,
    not_read: bool,
    status: Option<Status>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct FleetRollout {
    owner: String,
    profile: String,
    id: String,
    tool: ToolKind,
    location: RolloutLocation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RolloutLocation {
    Configured,
    Recorded {
        home: PathBuf,
        source: PathBuf,
        source_key: PathBuf,
    },
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FleetStatus {
    Complete,
    Failed,
    Truncated,
}

struct FleetRollouts {
    rollouts: Vec<FleetRollout>,
    status: FleetStatus,
}

struct RolloutFile {
    path: PathBuf,
    metadata: std::fs::Metadata,
    modified: Option<SystemTime>,
}

enum RolloutSource {
    File(RolloutFile),
    Missing,
    Failed,
}

struct LocatedRollout<'a> {
    rollout: &'a FleetRollout,
    source: RolloutSource,
    modified: Option<SystemTime>,
}

struct RankedGroup {
    group: Group,
    observed: Option<i64>,
}

enum RenderLine {
    Cells([String; 8]),
    Summary { label: String, status: String },
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
    let mut scopes = configured_scopes(&cfg, inputs.home);
    let mut groups = Vec::new();
    let mut budget = Budget::new();
    let fleet = fleet_rollouts(inputs.sessions, &mut budget);
    add_recorded_codex_scopes(&mut scopes, &fleet);
    for scope in &scopes {
        let quota = scope.tool.adapter().quota;
        match quota.source {
            QuotaSource::ClaudeCache => groups.push(Group {
                profiles: scope.profiles.clone(),
                tool: scope.tool,
                home: scope.home.clone(),
                clients: scope.clients.clone(),
                rollout: None,
                owner: None,
                rows: rows_or_placeholder(read_claude(scope, inputs.now, &mut budget)),
                hint: scope.hint.clone(),
                summary: None,
            }),
            QuotaSource::CodexRollouts => {
                groups.extend(codex_groups(scope, &fleet, inputs.now, &mut budget));
            }
            QuotaSource::Unsupported => groups.push(Group {
                profiles: scope.profiles.clone(),
                tool: scope.tool,
                home: scope.home.clone(),
                clients: scope.clients.clone(),
                rollout: None,
                owner: None,
                rows: vec![placeholder(Status::Unsupported)],
                hint: scope
                    .hint
                    .clone()
                    .or_else(|| quota.unsupported_hint.map(str::to_owned)),
                summary: None,
            }),
        }
    }
    let canonical_operator_home = inputs.home.and_then(|home| {
        crate::run::canonical_config_home(&crate::launch_cmd::Resolved::Path(home.to_path_buf()))
            .ok()
            .and_then(|resolved| match resolved {
                crate::launch_cmd::Resolved::Path(path) => Some(path),
                crate::launch_cmd::Resolved::Absent | crate::launch_cmd::Resolved::Unknown(_) => {
                    None
                }
            })
    });
    write!(
        out,
        "{}",
        render_at(
            &groups,
            canonical_operator_home.as_deref().or(inputs.home),
            inputs.now
        )
    )?;
    Ok(0)
}

fn configured_scopes(cfg: &crate::config::IdentityConfig, home: Option<&Path>) -> Vec<Scope> {
    let mut scopes: Vec<Scope> = Vec::new();
    for (profile, raw) in &cfg.profiles {
        let resolved = match cfg.command(profile, home) {
            Ok(Some(resolved)) => resolved,
            Ok(None) => {
                scopes.push(unknown_scope(profile, resolved_tool(""), None, None));
                continue;
            }
            Err(error) => {
                let client = config_error_client(&error);
                let tool = client
                    .and_then(|label| cfg.client(label))
                    .map_or_else(|| resolved_tool(raw), |client| client.tool);
                let client = client.and_then(|label| displayed_client(cfg, label, tool));
                scopes.push(unknown_scope(
                    profile,
                    tool,
                    client,
                    Some(error.to_string()),
                ));
                continue;
            }
        };
        let tool = resolved_tool(resolved.as_str());
        let client = resolved
            .client_label()
            .and_then(|label| displayed_client(cfg, label, tool));
        let unknown_variable = std::cell::RefCell::new(None);
        let account_variable = tool.adapter().config_home_env;
        let resolution = crate::launch_cmd::config_home_resolution(&resolved, tool, &|name| {
            if name == "HOME" {
                return home.map(|path| path.display().to_string());
            }
            if account_variable == Some(name) {
                return None;
            }
            let mut unknown = unknown_variable.borrow_mut();
            if unknown.is_none() {
                *unknown = Some(name.to_owned());
            }
            None
        });
        if let Some(variable) = unknown_variable.into_inner() {
            scopes.push(unknown_scope(
                profile,
                tool,
                client,
                Some(format!("depends on pane variable {variable}")),
            ));
            continue;
        }
        let paths = match resolved_scope_paths(tool, &resolution, home) {
            Ok(paths) => paths,
            Err(error) => {
                scopes.push(unknown_scope(profile, tool, client, Some(error)));
                continue;
            }
        };
        let ScopePaths {
            home: config_home,
            source,
            source_key,
            hint,
        } = paths;
        if let Some(scope) = scopes.iter_mut().find(|scope| {
            scope.tool == tool && scope.source_key == source_key && scope.hint == hint
        }) {
            scope.profiles.push(profile.clone());
            if let Some(client) = client
                && !scope.clients.contains(&client)
            {
                scope.clients.push(client);
            }
        } else {
            scopes.push(Scope {
                tool,
                home: config_home,
                source,
                source_key,
                profiles: vec![profile.clone()],
                clients: client.into_iter().collect(),
                hint,
            });
        }
    }
    scopes
}

fn resolved_tool(command: &str) -> ToolKind {
    crate::launch_cmd::lex_simple_command(command)
        .map_or_else(|_| ToolKind::from_binary_name(""), |parsed| parsed.tool())
}

fn config_error_client(error: &crate::config::ConfigError) -> Option<&str> {
    match error {
        crate::config::ConfigError::ClientEnvConflict { client, .. }
        | crate::config::ConfigError::ClientHome { client, .. } => Some(client),
        _ => None,
    }
}

fn displayed_client(
    cfg: &crate::config::IdentityConfig,
    label: &str,
    tool: ToolKind,
) -> Option<String> {
    let client = cfg.client(label)?;
    let default_alias =
        label == tool.as_str() && client.executable == label && client.config_home.is_none();
    (!default_alias).then(|| label.to_owned())
}

fn resolved_scope_paths(
    tool: ToolKind,
    resolution: &crate::launch_cmd::ConfigHomeResolution,
    operator_home: Option<&Path>,
) -> Result<ScopePaths, String> {
    let canonical_home = crate::run::canonical_config_home(&resolution.home)?;
    let (home, mut hint) = match canonical_home {
        crate::launch_cmd::Resolved::Path(path) => (Some(path), None),
        crate::launch_cmd::Resolved::Absent => (None, None),
        crate::launch_cmd::Resolved::Unknown(reason) => (None, Some(reason)),
    };
    if tool.adapter().quota.source == QuotaSource::Unsupported {
        let fallback = tool
            .adapter()
            .quota
            .default_home
            .and_then(|name| operator_home.map(|base| base.join(name)));
        let fallback = match fallback {
            Some(path) => {
                match crate::run::canonical_config_home(&crate::launch_cmd::Resolved::Path(path))? {
                    crate::launch_cmd::Resolved::Path(path) => Some(path),
                    crate::launch_cmd::Resolved::Absent
                    | crate::launch_cmd::Resolved::Unknown(_) => None,
                }
            }
            None => None,
        };
        return Ok(ScopePaths {
            home: fallback,
            source: None,
            source_key: None,
            hint,
        });
    }
    let source = match tool.adapter().quota.source {
        QuotaSource::ClaudeCache if resolution.explicit => home
            .as_ref()
            .map(|config_home| config_home.join(".claude.json")),
        QuotaSource::ClaudeCache => match &resolution.base {
            crate::launch_cmd::Resolved::Path(_) => {
                match crate::run::canonical_config_home(&resolution.base)? {
                    crate::launch_cmd::Resolved::Path(base) => Some(base.join(".claude.json")),
                    crate::launch_cmd::Resolved::Absent
                    | crate::launch_cmd::Resolved::Unknown(_) => None,
                }
            }
            crate::launch_cmd::Resolved::Absent => None,
            crate::launch_cmd::Resolved::Unknown(reason) => {
                if hint.is_none() {
                    hint = Some(reason.clone());
                }
                None
            }
        },
        QuotaSource::CodexRollouts => home
            .as_ref()
            .map(|config_home| config_home.join("sessions")),
        QuotaSource::Unsupported => None,
    };
    let source_key = source.clone().map(canonical_source).transpose()?;
    Ok(ScopePaths {
        home,
        source,
        source_key,
        hint,
    })
}

fn canonical_source(path: PathBuf) -> Result<PathBuf, String> {
    match crate::run::canonical_config_home(&crate::launch_cmd::Resolved::Path(path))? {
        crate::launch_cmd::Resolved::Path(path) => Ok(path),
        crate::launch_cmd::Resolved::Absent | crate::launch_cmd::Resolved::Unknown(_) => {
            Err("quota source did not resolve to a path".to_owned())
        }
    }
}

fn unknown_scope(
    profile: &str,
    tool: ToolKind,
    client: Option<String>,
    hint: Option<String>,
) -> Scope {
    Scope {
        tool,
        home: None,
        source: None,
        source_key: None,
        profiles: vec![profile.to_owned()],
        clients: client.into_iter().collect(),
        hint,
    }
}

fn read_claude(scope: &Scope, now: i64, budget: &mut Budget) -> ReadRows {
    let Some(path) = scope.source.as_deref() else {
        return ReadRows::Missing;
    };
    let bytes = match bounded_whole_file(path, CLAUDE_MAX_BYTES, budget) {
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

fn fleet_rollouts(sessions: Option<&Path>, budget: &mut Budget) -> FleetRollouts {
    let Some(sessions) = sessions else {
        return FleetRollouts {
            rollouts: Vec::new(),
            status: FleetStatus::Complete,
        };
    };
    let paths = match session_paths(sessions, budget) {
        Ok(Bounded::Ready(paths)) => paths,
        Ok(Bounded::Truncated) => {
            return FleetRollouts {
                rollouts: Vec::new(),
                status: FleetStatus::Truncated,
            };
        }
        Err(_) => {
            return FleetRollouts {
                rollouts: Vec::new(),
                status: FleetStatus::Failed,
            };
        }
    };
    let mut rollouts = Vec::new();
    let mut status = FleetStatus::Complete;
    for path in paths {
        let meta_path = crate::store::open(&path).meta_path();
        let bytes = match bounded_whole_file(&meta_path, SESSION_META_MAX_BYTES, budget) {
            Ok(Bounded::Ready(Some(bytes))) => bytes,
            Ok(Bounded::Ready(None)) => continue,
            Ok(Bounded::Truncated) => {
                status = FleetStatus::Truncated;
                break;
            }
            Err(_) => {
                status = FleetStatus::Failed;
                continue;
            }
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            status = FleetStatus::Failed;
            continue;
        };
        let meta = crate::meta::Meta::parse(text);
        let session = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        for seat in meta.roster() {
            let Some(binary) = seat.binary.as_deref() else {
                continue;
            };
            let tool = ToolKind::from_binary_name(binary);
            if tool.adapter().quota.source != QuotaSource::CodexRollouts {
                continue;
            }
            let (Some(profile), Some(id)) = (&seat.profile, &seat.harness_session) else {
                continue;
            };
            let location = recorded_rollout_location(&seat.config_home);
            rollouts.push(FleetRollout {
                owner: format!("{session}:{}", seat.name),
                profile: profile.clone(),
                id: id.clone(),
                tool,
                location,
            });
        }
    }
    FleetRollouts { rollouts, status }
}

fn recorded_rollout_location(home: &crate::meta::RecordedConfigHome) -> RolloutLocation {
    match home {
        crate::meta::RecordedConfigHome::Missing => RolloutLocation::Configured,
        crate::meta::RecordedConfigHome::Path(home)
        | crate::meta::RecordedConfigHome::Implicit(home) => {
            let source = home.join("sessions");
            match canonical_source(source.clone()) {
                Ok(source_key) => RolloutLocation::Recorded {
                    home: home.clone(),
                    source,
                    source_key,
                },
                Err(reason) => RolloutLocation::Unknown(reason),
            }
        }
        crate::meta::RecordedConfigHome::Absent => {
            RolloutLocation::Unknown("recorded config home is absent".to_owned())
        }
        crate::meta::RecordedConfigHome::Unknown => {
            RolloutLocation::Unknown("recorded config home is unknown".to_owned())
        }
        crate::meta::RecordedConfigHome::Invalid => {
            RolloutLocation::Unknown("recorded config home is invalid".to_owned())
        }
    }
}

fn add_recorded_codex_scopes(scopes: &mut Vec<Scope>, fleet: &FleetRollouts) {
    for rollout in &fleet.rollouts {
        match &rollout.location {
            RolloutLocation::Configured => {}
            RolloutLocation::Recorded {
                home,
                source,
                source_key,
            } => {
                if let Some(scope) = scopes.iter_mut().find(|scope| {
                    scope.tool == rollout.tool && scope.source_key.as_ref() == Some(source_key)
                }) {
                    if !scope.profiles.contains(&rollout.profile) {
                        scope.profiles.push(rollout.profile.clone());
                    }
                } else {
                    scopes.push(Scope {
                        tool: rollout.tool,
                        home: Some(home.clone()),
                        source: Some(source.clone()),
                        source_key: Some(source_key.clone()),
                        profiles: vec![rollout.profile.clone()],
                        clients: Vec::new(),
                        hint: None,
                    });
                }
            }
            RolloutLocation::Unknown(reason) => {
                if let Some(scope) = scopes.iter_mut().find(|scope| {
                    scope.tool == rollout.tool
                        && scope.source.is_none()
                        && scope.hint.as_ref() == Some(reason)
                }) {
                    if !scope.profiles.contains(&rollout.profile) {
                        scope.profiles.push(rollout.profile.clone());
                    }
                } else {
                    scopes.push(Scope {
                        tool: rollout.tool,
                        home: None,
                        source: None,
                        source_key: None,
                        profiles: vec![rollout.profile.clone()],
                        clients: Vec::new(),
                        hint: Some(reason.clone()),
                    });
                }
            }
        }
    }
}

fn session_paths(root: &Path, budget: &mut Budget) -> io::Result<Bounded<Vec<PathBuf>>> {
    if !budget.claim_file() {
        return Ok(Bounded::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: bounded canonical-session enumeration for quota rollout provenance"
    )]
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Bounded::Ready(Vec::new()));
        }
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "sessions root is not a directory",
        ));
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: enumerates only bounded direct children of the canonical ae sessions root"
    )]
    let entries = std::fs::read_dir(root)?;
    let mut paths = Vec::new();
    for entry in entries {
        if !budget.claim_file() {
            return Ok(Bounded::Truncated);
        }
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() && !kind.is_symlink() {
            paths.push(entry.path());
        }
    }
    if budget.expired() {
        return Ok(Bounded::Truncated);
    }
    paths.sort();
    Ok(Bounded::Ready(paths))
}

fn codex_groups(scope: &Scope, fleet: &FleetRollouts, now: i64, budget: &mut Budget) -> Vec<Group> {
    let candidates = scope_rollouts(scope, fleet);
    let total = candidates.len();
    let mut truncated = fleet.status == FleetStatus::Truncated;
    let located = locate_rollouts(scope, &candidates, budget, &mut truncated);

    let mut ranked = Vec::new();
    for located in located {
        let rows = match located.source {
            RolloutSource::File(file) => read_codex_file(&file, now, budget),
            RolloutSource::Missing => ReadRows::Missing,
            RolloutSource::Failed => ReadRows::Failed,
        };
        if matches!(rows, ReadRows::Truncated) {
            truncated = true;
            break;
        }
        let rows = rows_or_placeholder(rows);
        let observed = rows.iter().filter_map(|row| row.observed_at).max();
        ranked.push(RankedGroup {
            group: Group {
                profiles: scope.profiles.clone(),
                tool: scope.tool,
                home: scope.home.clone(),
                clients: scope.clients.clone(),
                rollout: Some(located.rollout.id.clone()),
                owner: Some(located.rollout.owner.clone()),
                rows,
                hint: scope.hint.clone(),
                summary: None,
            },
            observed,
        });
    }
    order_ranked_groups(&mut ranked);

    summarize_codex_groups(scope, fleet.status, ranked, total, truncated)
}

fn scope_rollouts<'a>(scope: &Scope, fleet: &'a FleetRollouts) -> Vec<&'a FleetRollout> {
    fleet
        .rollouts
        .iter()
        .filter(|rollout| match &rollout.location {
            RolloutLocation::Configured => scope.profiles.contains(&rollout.profile),
            RolloutLocation::Recorded { source_key, .. } => {
                scope.source_key.as_ref() == Some(source_key)
            }
            RolloutLocation::Unknown(reason) => {
                scope.source.is_none()
                    && scope.hint.as_ref() == Some(reason)
                    && scope.profiles.contains(&rollout.profile)
            }
        })
        .fold(Vec::new(), |mut unique, rollout| {
            if !unique
                .iter()
                .any(|seen: &&FleetRollout| seen.id == rollout.id)
            {
                unique.push(rollout);
            }
            unique
        })
}

fn locate_rollouts<'a>(
    scope: &Scope,
    candidates: &[&'a FleetRollout],
    budget: &mut Budget,
    truncated: &mut bool,
) -> Vec<LocatedRollout<'a>> {
    let mut located = Vec::new();
    for &rollout in candidates {
        let sessions = match &rollout.location {
            RolloutLocation::Configured => scope.source.as_deref(),
            RolloutLocation::Recorded { source, .. } => Some(source.as_path()),
            RolloutLocation::Unknown(_) => None,
        };
        let source = match sessions {
            Some(sessions) => match find_codex_rollout(sessions, &rollout.id, budget) {
                Ok(Bounded::Ready(Some(file))) => RolloutSource::File(file),
                Ok(Bounded::Ready(None)) => RolloutSource::Missing,
                Ok(Bounded::Truncated) => {
                    *truncated = true;
                    break;
                }
                Err(_) => RolloutSource::Failed,
            },
            None => RolloutSource::Missing,
        };
        let modified = match &source {
            RolloutSource::File(file) => file.modified,
            RolloutSource::Missing | RolloutSource::Failed => None,
        };
        located.push(LocatedRollout {
            rollout,
            source,
            modified,
        });
    }
    order_located_rollouts(&mut located);
    located
}

fn summarize_codex_groups(
    scope: &Scope,
    fleet_status: FleetStatus,
    ranked: Vec<RankedGroup>,
    total: usize,
    truncated: bool,
) -> Vec<Group> {
    let shown = ranked.len().min(CODEX_DISPLAY_ROLLOUTS);
    let hidden = total.saturating_sub(shown);
    let oldest_observed = ranked
        .iter()
        .skip(shown)
        .flat_map(|ranked| ranked.group.rows.iter())
        .filter_map(|row| row.observed_at)
        .min();
    let unreadable = ranked
        .iter()
        .skip(shown)
        .filter(|ranked| {
            ranked
                .group
                .rows
                .iter()
                .any(|row| row.status == Status::ReadError)
        })
        .count();
    let mut groups: Vec<Group> = ranked
        .into_iter()
        .take(CODEX_DISPLAY_ROLLOUTS)
        .map(|ranked| ranked.group)
        .collect();
    if groups.is_empty() {
        groups.push(Group {
            profiles: scope.profiles.clone(),
            tool: scope.tool,
            home: scope.home.clone(),
            clients: scope.clients.clone(),
            rollout: None,
            owner: None,
            rows: vec![placeholder(Status::Unknown)],
            hint: scope.hint.clone(),
            summary: None,
        });
    }
    let summary_status = if truncated {
        Some(Status::Truncated)
    } else if fleet_status == FleetStatus::Failed || unreadable > 0 {
        Some(Status::ReadError)
    } else {
        None
    };
    if hidden > 0 || summary_status.is_some() {
        groups.push(Group {
            profiles: Vec::new(),
            tool: scope.tool,
            home: scope.home.clone(),
            clients: scope.clients.clone(),
            rollout: None,
            owner: None,
            rows: Vec::new(),
            hint: None,
            summary: Some(RolloutSummary {
                hidden,
                unreadable,
                oldest_observed,
                not_read: truncated && shown == 0,
                status: summary_status,
            }),
        });
    }
    groups
}

fn order_located_rollouts(rollouts: &mut [LocatedRollout<'_>]) {
    rollouts.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| left.rollout.owner.cmp(&right.rollout.owner))
            .then_with(|| left.rollout.id.cmp(&right.rollout.id))
    });
}

fn order_ranked_groups(groups: &mut [RankedGroup]) {
    groups.sort_by(|left, right| {
        right
            .observed
            .cmp(&left.observed)
            .then_with(|| left.group.owner.cmp(&right.group.owner))
            .then_with(|| left.group.rollout.cmp(&right.group.rollout))
    });
}

fn read_codex_file(file: &RolloutFile, now: i64, budget: &mut Budget) -> ReadRows {
    let (bytes, starts_at_boundary) = match bounded_tail_after_lstat(file, CODEX_TAIL_BYTES, budget)
    {
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
    let identity = if group.clients.is_empty() {
        group
            .home
            .as_deref()
            .map_or_else(|| "unknown".to_owned(), |path| short_path(path, home))
    } else {
        group.clients.join(", ")
    };
    let mut label = format!("{} · {identity}", group.tool.as_str());
    if group.tool.adapter().quota.source == QuotaSource::CodexRollouts {
        label.push_str(" · unidentified");
        if let Some(owner) = group.owner.as_deref() {
            let _ = write!(label, " ({owner})");
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
    let mut table = vec![RenderLine::Cells(HEADER.map(str::to_owned))];
    for group in groups {
        if let Some(summary) = &group.summary {
            table.push(RenderLine::Summary {
                label: rollout_summary_label(summary, now),
                status: summary
                    .status
                    .map_or_else(String::new, |status| status.as_str().to_owned()),
            });
            continue;
        }
        for (index, row) in group.rows.iter().enumerate() {
            let trustworthy = matches!(row.status, Status::Fresh | Status::Stale);
            let status = group.hint.as_deref().map_or_else(
                || row.status.as_str().to_owned(),
                |hint| format!("{} ({hint})", row.status.as_str()),
            );
            table.push(RenderLine::Cells([
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
            ]));
        }
    }
    render_table(&table)
}

fn rollout_summary_label(summary: &RolloutSummary, now: i64) -> String {
    if summary.hidden == 0 {
        return "rollout inventory incomplete".to_owned();
    }
    let noun = if summary.hidden == 1 {
        "rollout"
    } else {
        "rollouts"
    };
    let disposition = if summary.not_read {
        "not read"
    } else {
        "not shown"
    };
    let mut label = format!("+{} {noun} {disposition}", summary.hidden);
    if summary.unreadable > 0 {
        let _ = write!(label, " ({} unreadable)", summary.unreadable);
    }
    if let Some(observed) = summary.oldest_observed {
        let _ = write!(
            label,
            " (oldest observed {})",
            age_label(now.saturating_sub(observed))
        );
    }
    label
}

fn render_table(table: &[RenderLine]) -> String {
    let sanitized: Vec<RenderLine> = table
        .iter()
        .map(|line| match line {
            RenderLine::Cells(row) => {
                RenderLine::Cells(std::array::from_fn(|column| sanitize_cell(&row[column])))
            }
            RenderLine::Summary { label, status } => RenderLine::Summary {
                label: sanitize_cell(label),
                status: sanitize_cell(status),
            },
        })
        .collect();
    let table = sanitized.as_slice();
    let mut widths = [0_usize; 8];
    for row in table.iter().filter_map(|line| match line {
        RenderLine::Cells(row) => Some(row),
        RenderLine::Summary { .. } => None,
    }) {
        for (column, value) in row.iter().enumerate() {
            widths[column] = widths[column]
                .max(value.chars().count())
                .min(TABLE_MAX_WIDTHS[column]);
        }
    }
    let mut out = String::new();
    for line in table {
        let row = match line {
            RenderLine::Cells(row) => row,
            RenderLine::Summary { label, status } => {
                let indent = widths[0] + 2;
                out.extend(std::iter::repeat_n(' ', indent));
                out.push_str(label);
                if !status.is_empty() {
                    let status_column = widths[..7].iter().sum::<usize>() + 2 * 7;
                    let used = indent + label.chars().count();
                    out.extend(std::iter::repeat_n(
                        ' ',
                        status_column.saturating_sub(used).max(2),
                    ));
                    out.push_str(status);
                }
                out.push('\n');
                continue;
            }
        };
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

fn sanitize_cell(text: &str) -> String {
    let mut chars = text.chars().peekable();
    let mut clean = String::with_capacity(text.len());
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            clean.push('?');
            consume_escape(&mut chars);
        } else if ch.is_control() {
            clean.push('?');
        } else {
            clean.push(ch);
        }
    }
    clean
}

fn consume_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.next() {
        Some('[') => {
            for ch in chars.by_ref() {
                if ('@'..='~').contains(&ch) {
                    break;
                }
            }
        }
        Some(']') => {
            while let Some(ch) = chars.next() {
                if ch == '\u{7}' {
                    break;
                }
                if ch == '\u{1b}' && chars.next_if_eq(&'\\').is_some() {
                    break;
                }
            }
        }
        Some(_) | None => {}
    }
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

#[cfg(test)]
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
    let modified = metadata.modified().ok();
    bounded_tail_after_lstat(
        &RolloutFile {
            path: path.to_owned(),
            metadata,
            modified,
        },
        cap,
        budget,
    )
}

fn bounded_tail_after_lstat(
    rollout: &RolloutFile,
    cap: u64,
    budget: &mut Budget,
) -> io::Result<Bounded<(Vec<u8>, bool)>> {
    if cap == 0 {
        return Ok(Bounded::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: opens only the exact rollout named by an ae-recorded harness session id"
    )]
    let mut file = File::open(&rollout.path)?;
    let opened = file.metadata()?;
    if !opened.file_type().is_file() || !same_file(&rollout.metadata, &opened) {
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
) -> io::Result<Bounded<Option<RolloutFile>>> {
    let Some(dirs) = codex_rollout_dirs(root, id) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recorded Codex id carries no usable start time",
        ));
    };
    let mut found = None;
    let suffix = format!("-{id}.jsonl");
    for dir in dirs {
        if !budget.claim_file() {
            return Ok(Bounded::Truncated);
        }
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: quota lstat classifies only the three UTC neighbours of the Codex id day"
        )]
        let metadata = match std::fs::symlink_metadata(&dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "rollout root is not a directory",
            ));
        }
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: bounded enumeration of three possible local-clock days for one exact Codex id"
        )]
        let entries = std::fs::read_dir(&dir)?;
        for entry in entries {
            if !budget.claim_file() {
                return Ok(Bounded::Truncated);
            }
            let entry = entry?;
            let name = entry.file_name();
            if name.to_string_lossy().ends_with(&suffix) {
                let path = entry.path();
                #[allow(
                    clippy::disallowed_methods,
                    reason = "a door: one budgeted lstat validates and ranks an exact ae-owned Codex rollout"
                )]
                let metadata = std::fs::symlink_metadata(&path)?;
                if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
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
                let modified = metadata.modified().ok();
                found = Some(RolloutFile {
                    path,
                    metadata,
                    modified,
                });
            }
        }
    }
    if budget.expired() {
        return Ok(Bounded::Truncated);
    }
    Ok(Bounded::Ready(found))
}

fn codex_rollout_dirs(root: &Path, id: &str) -> Option<Vec<PathBuf>> {
    if crate::archive::canonical_uuid(id) != id || id.as_bytes().get(14) != Some(&b'7') {
        return None;
    }
    let millis = u64::from_str_radix(&format!("{}{}", id.get(..8)?, id.get(9..13)?), 16).ok()?;
    let seconds = i64::try_from(millis / 1_000).ok()?;
    [0, -86_400, 86_400]
        .into_iter()
        .map(|offset| dated_dir(root, seconds.saturating_add(offset)))
        .collect()
}

fn dated_dir(root: &Path, seconds: i64) -> Option<PathBuf> {
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
        Bounded, Budget, CLAUDE_MAX_BYTES, CODEX_TAIL_BYTES, FRESH_SECS, FUTURE_SKEW_SECS,
        FleetRollout, FleetRollouts, FleetStatus, LocatedRollout, ReadRows, RenderLine,
        RolloutLocation, RolloutSource, Scope, Status, bounded_tail, bounded_whole_file,
        codex_groups, codex_rollout_dirs, configured_scopes, find_codex_rollout, freshness,
        order_located_rollouts, percent_label, profiles_label, read_bounded_tail, read_claude,
        render_at, render_table, rows_or_placeholder, sanitize_cell, vendor_timestamp,
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
    fn codex_uuidv7_lookup_probes_its_utc_day_then_both_local_clock_neighbours() {
        let root = std::path::Path::new("/rollouts");
        let id = "01a08046-1974-7352-ade3-81a786200795";
        assert_eq!(
            codex_rollout_dirs(root, id),
            Some(vec![
                root.join("2026/09/08"),
                root.join("2026/09/07"),
                root.join("2026/09/09"),
            ])
        );
        assert_eq!(codex_rollout_dirs(root, "not-a-session-id"), None);
    }

    #[test]
    fn codex_uuidv7_lookup_finds_an_exact_rollout_on_a_neighbour_day() {
        let root = std::path::PathBuf::from(format!(
            "/tmp/ae-quota-neighbour-day-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let day = root.join("2026/09/07");
        std::fs::create_dir_all(&day).expect("neighbour day");
        let id = "01a08046-1974-7352-ade3-81a786200795";
        let rollout = day.join(format!("rollout-local-clock-{id}.jsonl"));
        std::fs::write(&rollout, b"{}\n").expect("neighbour rollout");
        let found = find_codex_rollout(&root, id, &mut Budget::new())
            .expect("bounded lookup on neighbours");
        let Bounded::Ready(Some(found)) = found else {
            panic!("neighbour rollout was not found");
        };
        assert_eq!(found.path, rollout);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_rollouts_read_newest_files_first_then_display_three_by_record_clock() {
        const NOW: i64 = 1_788_858_600;
        let root = std::path::PathBuf::from(format!(
            "/tmp/ae-quota-five-rollouts-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let day = root.join(".codex/sessions/2026/09/08");
        std::fs::create_dir_all(&day).expect("rollout day");
        let ids = [
            "01a08046-2000-7abc-8abc-000000000000",
            "01a08046-2001-7abc-8abc-000000000001",
            "01a08046-2002-7abc-8abc-000000000002",
            "01a08046-2003-7abc-8abc-000000000003",
            "01a08046-2004-7abc-8abc-000000000004",
        ];
        let timestamps = [
            "2026-09-08T09:00:00Z",
            "2026-09-08T09:01:00Z",
            "2026-09-08T09:02:00Z",
            "2026-09-08T09:03:00Z",
            "2026-09-08T09:04:00Z",
        ];
        let mut fleet = FleetRollouts {
            rollouts: Vec::new(),
            status: FleetStatus::Complete,
        };
        for (index, (id, timestamp)) in ids.into_iter().zip(timestamps).enumerate() {
            let record = format!(
                "{{\"timestamp\":\"{timestamp}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"rate_limits\":{{\"limit_id\":\"codex\",\"plan_type\":\"pro\",\"primary\":{{\"used_percent\":{index},\"window_minutes\":300,\"resets_at\":1788861600}}}}}}}}\n"
            );
            std::fs::write(day.join(format!("rollout-test-{id}.jsonl")), record)
                .expect("rollout record");
            fleet.rollouts.push(FleetRollout {
                owner: format!("session:seat-{index}"),
                profile: "codex-profile".to_owned(),
                id: id.to_owned(),
                tool: ToolKind::Codex,
                location: RolloutLocation::Configured,
            });
        }
        let scope = Scope {
            tool: ToolKind::Codex,
            home: Some(root.join(".codex")),
            source: Some(root.join(".codex/sessions")),
            source_key: Some(root.join(".codex/sessions")),
            profiles: vec!["codex-profile".to_owned()],
            clients: Vec::new(),
            hint: None,
        };
        let groups = codex_groups(&scope, &fleet, NOW, &mut Budget::new());
        let shown: Vec<_> = groups
            .iter()
            .filter(|group| group.summary.is_none())
            .filter_map(|group| group.owner.as_deref())
            .collect();
        assert_eq!(
            shown,
            ["session:seat-4", "session:seat-3", "session:seat-2"]
        );
        let rendered = render_at(&groups, Some(&root), NOW);
        assert!(rendered.contains("+2 rollouts not shown"), "{rendered}");
        assert!(rendered.contains("oldest observed 10m ago"), "{rendered}");
        assert!(!rendered.contains("session:seat-0"), "{rendered}");
        assert!(rendered.lines().all(|line| line.chars().count() <= 160));
        assert_eq!(CODEX_TAIL_BYTES, 256 * 1024);

        let older = FleetRollout {
            owner: "older".to_owned(),
            profile: "p".to_owned(),
            id: ids[0].to_owned(),
            tool: ToolKind::Codex,
            location: RolloutLocation::Configured,
        };
        let newer = FleetRollout {
            owner: "newer".to_owned(),
            profile: "p".to_owned(),
            id: ids[1].to_owned(),
            tool: ToolKind::Codex,
            location: RolloutLocation::Configured,
        };
        let epoch = std::time::UNIX_EPOCH;
        let mut located = [
            LocatedRollout {
                rollout: &older,
                source: RolloutSource::Missing,
                modified: Some(epoch),
            },
            LocatedRollout {
                rollout: &newer,
                source: RolloutSource::Missing,
                modified: Some(epoch + std::time::Duration::from_secs(1)),
            },
        ];
        order_located_rollouts(&mut located);
        assert_eq!(located[0].rollout.owner, "newer");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_budget_exhaustion_marks_the_summary_not_a_rollout_group() {
        const NOW: i64 = 1_788_858_600;
        let root = std::path::PathBuf::from(format!(
            "/tmp/ae-quota-truncated-summary-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let day = root.join(".codex/sessions/2026/09/08");
        std::fs::create_dir_all(&day).expect("rollout day");
        let mut fleet = FleetRollouts {
            rollouts: Vec::new(),
            status: FleetStatus::Complete,
        };
        for index in 0..5 {
            let id = format!("01a08046-2{index:03}-7abc-8abc-00000000000{index}");
            std::fs::write(day.join(format!("rollout-test-{id}.jsonl")), b"{}\n")
                .expect("rollout record");
            fleet.rollouts.push(FleetRollout {
                owner: format!("session:seat-{index}"),
                profile: "codex-profile".to_owned(),
                id,
                tool: ToolKind::Codex,
                location: RolloutLocation::Configured,
            });
        }
        let scope = Scope {
            tool: ToolKind::Codex,
            home: Some(root.join(".codex")),
            source: Some(root.join(".codex/sessions")),
            source_key: Some(root.join(".codex/sessions")),
            profiles: vec!["codex-profile".to_owned()],
            clients: Vec::new(),
            hint: None,
        };
        let mut budget = Budget {
            files_left: 4_096,
            bytes_left: 0,
            started: std::time::Instant::now(),
            max_elapsed: std::time::Duration::from_secs(1),
        };
        let groups = codex_groups(&scope, &fleet, NOW, &mut budget);
        assert!(groups.iter().all(|group| {
            group.summary.is_some() || group.rows.iter().all(|row| row.status != Status::Truncated)
        }));
        let summary = groups
            .iter()
            .find_map(|group| group.summary.as_ref())
            .expect("truncated summary");
        assert_eq!(summary.hidden, 5);
        assert_eq!(summary.unreadable, 0);
        assert!(summary.not_read);
        assert_eq!(summary.status, Some(Status::Truncated));
        let rendered = render_at(&groups, Some(&root), NOW);
        assert!(
            rendered.lines().any(|line| {
                line.contains("+5 rollouts not read") && line.contains("truncated")
            }),
            "{rendered}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn configured_scopes_use_resolved_client_identity_and_vendor_paths() {
        let root = std::path::PathBuf::from(format!(
            "/tmp/ae-quota-resolved-scopes-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("operator home");
        let canonical_root = match crate::run::canonical_config_home(
            &crate::launch_cmd::Resolved::Path(root.clone()),
        )
        .expect("canonical operator home")
        {
            crate::launch_cmd::Resolved::Path(path) => path,
            other => panic!("unexpected canonical home: {other:?}"),
        };
        let cfg = crate::config::IdentityConfig {
            clients: vec![
                (
                    "claude".to_owned(),
                    crate::config::Client {
                        executable: "claude".to_owned(),
                        config_home: None,
                        tool: ToolKind::Claude,
                    },
                ),
                (
                    "mic".to_owned(),
                    crate::config::Client {
                        executable: "claude".to_owned(),
                        config_home: Some("$HOME/.claude-mic".to_owned()),
                        tool: ToolKind::Claude,
                    },
                ),
            ],
            profiles: vec![
                ("default".to_owned(), "claude".to_owned()),
                ("custom".to_owned(), "mic".to_owned()),
                ("moved-home".to_owned(), "HOME=/other claude".to_owned()),
            ],
            ..crate::config::IdentityConfig::default()
        };
        let scopes = configured_scopes(&cfg, Some(&root));
        assert_eq!(scopes.len(), 3);
        assert_eq!(scopes[0].home, Some(canonical_root.join(".claude")));
        assert_eq!(scopes[0].source, Some(canonical_root.join(".claude.json")));
        assert!(scopes[0].clients.is_empty(), "default alias stays concise");
        assert_eq!(scopes[1].home, Some(canonical_root.join(".claude-mic")));
        assert_eq!(
            scopes[1].source,
            Some(canonical_root.join(".claude-mic/.claude.json"))
        );
        assert_eq!(scopes[1].clients, ["mic"]);
        assert_eq!(
            scopes[2].home,
            Some(std::path::PathBuf::from("/other/.claude"))
        );
        assert_eq!(
            scopes[2].source,
            Some(std::path::PathBuf::from("/other/.claude.json"))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn default_and_custom_claude_homes_read_only_their_own_cache() {
        let root =
            std::path::PathBuf::from(format!("/tmp/ae-quota-claude-homes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let custom = root.join(".claude-mic");
        std::fs::create_dir_all(&custom).expect("custom client home");
        let cache = |percent| {
            format!(
                "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":1000000,\"utilization\":{{\"limits\":[{{\"kind\":\"session\",\"percent\":{percent},\"resets_at\":\"1970-01-01T01:00:00Z\"}}]}}}}}}"
            )
        };
        std::fs::write(root.join(".claude.json"), cache(11)).expect("default cache");
        std::fs::write(custom.join(".claude.json"), cache(77)).expect("custom cache");
        let read = |home: std::path::PathBuf, source: std::path::PathBuf| {
            let scope = Scope {
                tool: ToolKind::Claude,
                home: Some(home),
                source: Some(source),
                source_key: None,
                profiles: vec!["p".to_owned()],
                clients: Vec::new(),
                hint: None,
            };
            match read_claude(&scope, 1_001, &mut Budget::new()) {
                ReadRows::Rows(rows) => rows[0].used_percent.clone(),
                _ => None,
            }
        };
        assert_eq!(
            read(root.join(".claude"), root.join(".claude.json")),
            Some("11".to_owned())
        );
        assert_eq!(
            read(custom.clone(), custom.join(".claude.json")),
            Some("77".to_owned())
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn percentages_trim_only_fractional_zeroes() {
        assert_eq!(percent_label("100"), "100%");
        assert_eq!(percent_label("12.0"), "12%");
        assert_eq!(percent_label("3.50"), "3.5%");
    }

    #[test]
    fn table_cells_replace_controls_and_terminal_escape_sequences() {
        assert_eq!(sanitize_cell("safe\u{1b}[2J\r\n\u{7f}tail"), "safe????tail");
        assert_eq!(sanitize_cell("a\u{1b}]0;title\u{7}b"), "a?b");
    }

    #[test]
    fn long_profile_lists_are_summarized_within_the_column_cap() {
        let profiles: Vec<_> = (0..50).map(|index| format!("profile-{index}")).collect();
        let label = profiles_label(&profiles);
        assert_eq!(
            label, "50 profiles: profile-0 profile-1 profile-2",
            "the first three profile names stay intact"
        );
        let rendered = render_table(&[RenderLine::Cells([
            label,
            "scope".to_owned(),
            "bucket".to_owned(),
            "5h".to_owned(),
            "1%".to_owned(),
            "in 1h".to_owned(),
            "1m ago".to_owned(),
            "fresh".to_owned(),
        ])]);
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
