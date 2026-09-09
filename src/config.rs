//! The two readers over ae's INI config.
//!
//! **Compact's three `[workspace]` values** (`main`, `workers`,
//! `purge_agent_history`) — [`read_workspace`], the original reader, kept exactly
//! as it was: compact needs three keys, so it reads three keys.
//!
//! **Identity v2** — [`read_identity`] and [`launch_plan`]: the `[clients]`
//! executable/config-home inventory, `[profiles]` commands, `[roster]`
//! name→profile bindings and the workspace seats, read with one per-line grammar
//! and validated BOTH directions into a typed [`LaunchPlan`] before any session
//! side effect exists. The identity plan (alias-free names, profile as metadata)
//! is the authority for the rules pinned here.
//!
//! Still not a general config framework: `[prompt]`, `[telegram]`, the layout
//! and copy-mode keys stay with the glue's reader.

use std::fmt;
use std::path::{Path, PathBuf};

/// The persisted path of a nonstandard per-session config overlay.
pub(crate) const LOCAL_CONFIG_KEY: &str = "local_config";

/// Resolve a session's local config overlay.
///
/// A recorded nonstandard path wins. Sessions without that row keep the
/// historical `<origin>/.ae/config` lookup, selected only while it exists.
#[must_use]
pub(crate) fn local_overlay(meta_dir: &Path, origin: &str) -> Option<PathBuf> {
    let recorded = crate::meta::read_bytes(meta_dir)
        .ok()
        .map(|bytes| crate::lifecycle::meta_value(&bytes, LOCAL_CONFIG_KEY))
        .unwrap_or_default();
    if !recorded.is_empty() {
        return Some(PathBuf::from(recorded));
    }
    let local = Path::new(if origin.is_empty() { "." } else { origin })
        .join(".ae")
        .join("config");
    crate::lifecycle::path_exists(&local).then_some(local)
}

/// The `[workspace]` values compact resolves.
#[derive(Debug)]
pub(crate) struct Workspace {
    pub(crate) main: Option<String>,
    pub(crate) workers: Option<String>,
    pub(crate) purge_agent_history: bool,
}

/// Read `[workspace].{main,workers,purge_agent_history}`, layering `local` over
/// `global` so the LAST file to set a key wins — matching `get_config`'s "last
/// match wins, local overrides global".
pub(crate) fn read_workspace(
    global: Option<&Path>,
    local: Option<&Path>,
) -> Result<Workspace, PathBuf> {
    let mut main = None;
    let mut workers = None;
    let mut purge = None;
    for file in [global, local].into_iter().flatten() {
        if apply_file(file, &mut main, &mut workers, &mut purge).is_err() {
            return Err(file.to_owned());
        }
    }
    Ok(Workspace {
        main,
        workers,
        purge_agent_history: matches!(purge.as_deref(), Some("true" | "1" | "yes" | "on")),
    })
}

/// Overlay one SELECTED config file's `[workspace]` keys onto the accumulators.
fn apply_file(
    file: &Path,
    main: &mut Option<String>,
    workers: &mut Option<String>,
    purge: &mut Option<String>,
) -> Result<(), ()> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: reads the INI config the frozen parse_config reads — see clippy.toml"
    )]
    let read = std::fs::read_to_string(file);
    let text = read.map_err(|_| ())?;
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = section_header(line) {
            section = name;
            continue;
        }
        if section != "workspace" {
            continue;
        }
        if let Some((key, value)) = parse_entry(line) {
            match key {
                "main" => *main = Some(value),
                "workers" => *workers = Some(value),
                "purge_agent_history" => *purge = Some(value),
                _ => {}
            }
        }
    }
    Ok(())
}

/// A `[section]` header line → the section name.
fn section_header(line: &str) -> Option<String> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    if !inner.is_empty()
        && inner
            .bytes()
            .all(|b| b.is_ascii_alphabetic() || b == b'_' || b == b'-')
    {
        Some(inner.to_owned())
    } else {
        None
    }
}

/// A `key = value` line → `(key, value)`.
fn parse_entry(line: &str) -> Option<(&str, String)> {
    parse_entry_with(line, is_config_key)
}

/// [`parse_entry`] with the key grammar as a parameter: `[roster]` keys are
/// agent NAMES (digit-leading allowed), every other section keeps the
/// config-key grammar.
fn parse_entry_with(line: &str, key_ok: fn(&str) -> bool) -> Option<(&str, String)> {
    let eq = line.find('=')?;
    let key = line[..eq].trim_end();
    if !key_ok(key) {
        return None;
    }
    Some((key, entry_value(line)?))
}

/// The raw KEY a line claims: the text before its first `=`, trimmed — `None`
/// for a line with no `=` or a `#` comment (a commented-out `# old = x` claims
/// nothing).
fn key_claim(line: &str) -> Option<&str> {
    let eq = line.find('=')?;
    let key = line[..eq].trim_end();
    if key.starts_with('#') {
        None
    } else {
        Some(key)
    }
}

/// The VALUE half of a `key = value` line: a `"..."` keeps its inner bytes
/// verbatim; an unquoted value strips a trailing `#comment` then whitespace and
/// must be non-empty.
fn entry_value(line: &str) -> Option<String> {
    let eq = line.find('=')?;
    let rhs = line[eq + 1..].trim_start();
    // The line is already whole-line-trimmed, so a fully-quoted value ends at
    // the final byte.
    if let Some(rest) = rhs.strip_prefix('"')
        && let Some(inner) = rest.strip_suffix('"')
    {
        return Some(inner.to_owned());
    }
    let val = match rhs.find('#') {
        Some(hash) => &rhs[..hash],
        None => rhs,
    };
    let val = val.trim_end();
    if val.is_empty() {
        return None;
    }
    Some(val.to_owned())
}

/// The key grammar `^[a-zA-Z_][a-zA-Z0-9_-]*`.
fn is_config_key(s: &str) -> bool {
    let mut bytes = s.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_alphabetic() || b == b'_' => {}
        _ => return false,
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The agent-name grammar, spelled as a refusal prints it.
pub const AGENT_NAME_GRAMMAR: &str = "^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$";

/// Whether `name` is an agent name: a letter or digit, then up to 63 of
/// letters, digits, `_` or `-`.
#[must_use]
pub fn is_agent_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    name.len() <= 64 && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// One configured CLI instance that profiles may build on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    /// The executable word, preserving operator-authored quoting.
    pub executable: String,
    /// The optional config-home path, preserving its launch-time expansion.
    pub config_home: Option<String>,
    /// The harness selected by the executable basename.
    pub tool: crate::tool::ToolKind,
}

/// A profile command after its optional client label has been expanded once.
/// The private field prevents resolved snapshots from re-entering expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCommand(String);

impl ResolvedCommand {
    /// The command text consumed by launch-command validators and builders.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the typed command into its transport representation.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

/// The identity v2 config: `[clients]`, `[profiles]`, `[roster]`, and the two
/// workspace seat keys.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IdentityConfig {
    /// `[clients] <label> = <executable> [config_home=<path>]`.
    pub clients: Vec<(String, Client)>,
    /// `[profiles] <profile> = <launch command>` — the reusable inventory.
    pub profiles: Vec<(String, String)>,
    /// `[roster] <name> = <profile>` — the agents promised to launch.
    pub roster: Vec<(String, String)>,
    /// `[workspace] main`, raw.
    pub main: Option<String>,
    /// `[workspace] workers`, raw (comma-separated).
    pub workers: Option<String>,
}

impl IdentityConfig {
    /// The raw launch command bound to `profile`, if defined.
    #[must_use]
    pub fn profile(&self, profile: &str) -> Option<&str> {
        self.profiles
            .iter()
            .find(|(key, _)| key == profile)
            .map(|(_, cmd)| cmd.as_str())
    }

    /// The configured CLI instance bound to `label`, if defined.
    #[must_use]
    pub fn client(&self, label: &str) -> Option<&Client> {
        self.clients
            .iter()
            .find(|(key, _)| key == label)
            .map(|(_, client)| client)
    }

    /// Resolve the profile's client label exactly once.
    ///
    /// # Errors
    ///
    /// A `$HOME`-rooted client path when `home` is unavailable or not absolute.
    pub fn command(
        &self,
        profile: &str,
        home: Option<&Path>,
    ) -> Result<Option<ResolvedCommand>, ConfigError> {
        let Some(raw) = self.profile(profile) else {
            return Ok(None);
        };
        let Ok(parsed) = crate::launch_cmd::lex_simple_command(raw) else {
            return Ok(Some(ResolvedCommand(raw.to_owned())));
        };
        let binary = crate::launch_cmd::launch_binary(&parsed);
        if binary.word.contains('/') {
            return Ok(Some(ResolvedCommand(raw.to_owned())));
        }
        let Some(client) = self.client(binary.word) else {
            return Ok(Some(ResolvedCommand(raw.to_owned())));
        };
        let mut replacement = client.executable.clone();
        if let Some(config_home) = &client.config_home {
            let Some(variable) = client.tool.adapter().config_home_env else {
                return Err(ConfigError::ClientHome {
                    profile: profile.to_owned(),
                    client: binary.word.to_owned(),
                    reason: format!(
                        "config_home is not supported for {}: its account variable is unverified",
                        client.tool.as_str()
                    ),
                });
            };
            let expanded =
                expand_client_home(profile, binary.word, config_home, client.tool, home)?;
            replacement = format!(
                "{variable}={} {replacement}",
                crate::launch::shell_quote(&expanded)
            );
        }
        let mut command = String::with_capacity(raw.len() + replacement.len());
        command.push_str(&raw[..binary.span.0]);
        command.push_str(&replacement);
        command.push_str(&raw[binary.span.1..]);
        Ok(Some(ResolvedCommand(command)))
    }

    /// Type a command snapshot already resolved by [`Self::command`].
    #[must_use]
    pub(crate) fn resolved_snapshot(command: &str) -> ResolvedCommand {
        ResolvedCommand(command.to_owned())
    }

    /// The profile `name` is bound to in `[roster]`, if any.
    #[must_use]
    pub fn roster_profile(&self, name: &str) -> Option<&str> {
        self.roster
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, profile)| profile.as_str())
    }
}

/// Why a config could not be READ as identity v2 — each refuses the whole read
/// before any plan is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// A SELECTED file that cannot be read or decoded (never treated as empty).
    Unreadable(PathBuf),
    /// One file names one identity key twice.
    DuplicateKey {
        /// The file.
        file: PathBuf,
        /// `profiles`, `roster` or `workspace`.
        section: String,
        /// The key as written.
        key: String,
        /// 1-based line of the repeat.
        line: usize,
    },
    /// The file still carries an `[agents]` section: the v1 shape, which has no
    /// v2 meaning and would leave the operator's intent split across two
    /// vocabularies.
    LegacyAgents {
        /// The file.
        file: PathBuf,
        /// 1-based line of the header.
        line: usize,
    },
    /// A `[roster]` key that is not an agent name.
    RosterKey {
        /// The file.
        file: PathBuf,
        /// The key as written.
        key: String,
        /// 1-based line.
        line: usize,
    },
    /// A `[clients]` label outside the agent-name grammar.
    ClientKey {
        /// The file.
        file: PathBuf,
        /// The label as written.
        key: String,
        /// 1-based line.
        line: usize,
    },
    /// A `[clients]` value outside the client grammar.
    ClientEntry {
        /// The file.
        file: PathBuf,
        /// The client label.
        client: String,
        /// 1-based line.
        line: usize,
        /// The specific refusal.
        reason: String,
    },
    /// A profile and its selected client both set the client's account variable.
    ClientEnvConflict {
        /// The profile key.
        profile: String,
        /// The client label.
        client: String,
        /// The conflicting variable.
        variable: String,
    },
    /// A `$HOME`-rooted client path could not be resolved for one profile.
    ClientHome {
        /// The profile key.
        profile: String,
        /// The client label.
        client: String,
        /// The specific refusal.
        reason: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(path) => {
                write!(f, "Error: config {} cannot be read.", path.display())
            }
            Self::DuplicateKey {
                file,
                section,
                key,
                line,
            } => write!(
                f,
                "Error: {}:{line}: duplicate key '{key}' in [{section}] — a v2 config names each key once per file.",
                file.display()
            ),
            Self::LegacyAgents { file, line } => write!(
                f,
                "Error: {}:{line}: [agents] is not a v2 section — move each alias to [profiles] and bind agent names to profiles in [roster].",
                file.display()
            ),
            Self::RosterKey { file, key, line } => write!(
                f,
                "Error: {}:{line}: invalid agent name '{key}' in [roster]. Names must match {AGENT_NAME_GRAMMAR}.",
                file.display()
            ),
            Self::ClientKey { file, key, line } => write!(
                f,
                "Error: {}:{line}: invalid client label '{key}' in [clients]. Labels must match {AGENT_NAME_GRAMMAR}.",
                file.display()
            ),
            Self::ClientEntry {
                file,
                client,
                line,
                reason,
            } => write!(
                f,
                "Error: {}:{line}: invalid [clients] entry '{client}': {reason}",
                file.display()
            ),
            Self::ClientEnvConflict {
                profile,
                client,
                variable,
            } => write!(
                f,
                "Error: [profiles] {profile}: both profile and client '{client}' set {variable}."
            ),
            Self::ClientHome {
                profile,
                client,
                reason,
            } => write!(
                f,
                "Error: [profiles] {profile} via client '{client}': {reason}"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Read the identity v2 config, layering `local` over `global` key by key —
/// a later file's value replaces an earlier file's for the same key, in the
/// same section; keys the later file does not name survive.
///
/// # Errors
///
/// [`ConfigError`] — a selected file that cannot be read, a same-file
/// duplicate identity key, a surviving `[agents]` section, or a `[roster]` key
/// outside the agent-name grammar. Absence is `None`, never an error.
pub fn read_identity(
    global: Option<&Path>,
    local: Option<&Path>,
) -> Result<IdentityConfig, ConfigError> {
    let mut cfg = IdentityConfig::default();
    for file in [global, local].into_iter().flatten() {
        overlay_identity(file, &mut cfg)?;
    }
    validate_client_conflicts(&cfg)?;
    Ok(cfg)
}

/// Parse one in-memory identity v2 config.
///
/// This is the pure parser behind [`read_identity`], exposed so hostile config
/// text can be exercised without putting a filesystem door in the fuzz loop.
///
/// # Errors
///
/// [`ConfigError`] — an invalid identity row or duplicate key. Diagnostics use
/// `<config>` as the source path because this input has no file.
pub fn parse_identity(text: &str) -> Result<IdentityConfig, ConfigError> {
    let mut cfg = IdentityConfig::default();
    overlay_identity_text(Path::new("<config>"), text, &mut cfg)?;
    Ok(cfg)
}

/// Read identity like [`read_identity`], but parse `global_default` when the
/// selected global file does not exist yet. A first launch uses this to
/// validate against the exact config text it will seed after preflight.
///
/// # Errors
///
/// The same errors as [`read_identity`]. An existing but unreadable global
/// file is still refused; only `NotFound` selects the in-memory default.
pub fn read_identity_with_global_default(
    global: Option<&Path>,
    local: Option<&Path>,
    global_default: &str,
) -> Result<IdentityConfig, ConfigError> {
    let mut cfg = IdentityConfig::default();
    if let Some(file) = global {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: reads the selected INI config or uses the exact first-run snapshot — see clippy.toml"
        )]
        match std::fs::read_to_string(file) {
            Ok(text) => overlay_identity_text(file, &text, &mut cfg)?,
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => {
                overlay_identity_text(file, global_default, &mut cfg)?;
            }
            Err(_) => return Err(ConfigError::Unreadable(file.to_owned())),
        }
    }
    if let Some(file) = local {
        overlay_identity(file, &mut cfg)?;
    }
    validate_client_conflicts(&cfg)?;
    Ok(cfg)
}

/// The four identity sections.
const IDENTITY_SECTIONS: [&str; 4] = ["clients", "profiles", "roster", "workspace"];

fn overlay_identity(file: &Path, cfg: &mut IdentityConfig) -> Result<(), ConfigError> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: reads the INI config the frozen parse_config reads — see clippy.toml"
    )]
    let read = std::fs::read_to_string(file);
    let text = read.map_err(|_| ConfigError::Unreadable(file.to_owned()))?;
    overlay_identity_text(file, &text, cfg)
}

fn overlay_identity_text(
    file: &Path,
    text: &str,
    cfg: &mut IdentityConfig,
) -> Result<(), ConfigError> {
    let mut section = String::new();
    let mut seen: Vec<(String, String)> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(name) = section_header(trimmed) {
            if name == "agents" {
                return Err(ConfigError::LegacyAgents {
                    file: file.to_owned(),
                    line,
                });
            }
            section = name;
            continue;
        }
        if !IDENTITY_SECTIONS.contains(&section.as_str()) {
            continue;
        }
        // The raw KEY CLAIM first (comments excluded): a `[roster]` key outside
        // the agent grammar refuses here, before any value parse, and a key
        // named twice refuses even when one claim carries no value.
        let Some(key) = key_claim(trimmed) else {
            continue;
        };
        if section == "roster" || section == "clients" {
            if !is_agent_name(key) {
                return Err(if section == "roster" {
                    ConfigError::RosterKey {
                        file: file.to_owned(),
                        key: key.to_owned(),
                        line,
                    }
                } else {
                    ConfigError::ClientKey {
                        file: file.to_owned(),
                        key: key.to_owned(),
                        line,
                    }
                });
            }
            if section == "clients" && key == "env" {
                return Err(client_error(
                    file,
                    key,
                    line,
                    "offending word 'env': the label is reserved prefix syntax".to_owned(),
                ));
            }
        } else if !is_config_key(key) {
            // A non-key line contributes nothing.
            continue;
        }
        if section == "workspace" && key != "main" && key != "workers" {
            continue;
        }
        if seen.iter().any(|(s, k)| s == &section && k == key) {
            return Err(ConfigError::DuplicateKey {
                file: file.to_owned(),
                section: section.clone(),
                key: key.to_owned(),
                line,
            });
        }
        seen.push((section.clone(), key.to_owned()));
        // Now the VALUE.
        let Some(value) = entry_value(trimmed) else {
            continue;
        };
        match section.as_str() {
            "clients" => {
                let client = parse_client(file, key, line, &value)?;
                upsert_client(&mut cfg.clients, key, client);
            }
            "profiles" => upsert(&mut cfg.profiles, key, value),
            "roster" => upsert(&mut cfg.roster, key, value),
            _ => {
                if key == "main" {
                    cfg.main = Some(value);
                } else {
                    cfg.workers = Some(value);
                }
            }
        }
    }
    Ok(())
}

/// Parse one client definition with the launch-command lexer. `$HOME` expands
/// to a sentinel only for structural validation; the caller's HOME is applied
/// later by [`IdentityConfig::command`].
fn parse_client(file: &Path, label: &str, line: usize, value: &str) -> Result<Client, ConfigError> {
    let parsed = crate::launch_cmd::lex_simple_command(value).map_err(|why| {
        let offending = value.split_ascii_whitespace().next().unwrap_or(value);
        client_error(
            file,
            label,
            line,
            format!("offending word '{offending}': client value is not one simple command ({why})"),
        )
    })?;
    if let Some(offending) = parsed.assignments.first() {
        return Err(client_error(
            file,
            label,
            line,
            format!("offending word '{offending}': an env prefix is not an executable"),
        ));
    }
    let executable = crate::launch_cmd::launch_binary(&parsed);
    if executable.index != 0 || executable.word == "env" {
        let offending = parsed.words.first().map_or("", String::as_str);
        return Err(client_error(
            file,
            label,
            line,
            format!("offending word '{offending}': an env prefix is not an executable"),
        ));
    }
    let executable_source = parsed.words.first().map_or("", String::as_str);
    if executable.word.is_empty()
        || (executable.word.contains('/') && !outer_quote_body(executable_source).starts_with('/'))
    {
        return Err(client_error(
            file,
            label,
            line,
            format!(
                "offending word '{executable_source}': executable must be a basename or absolute path"
            ),
        ));
    }
    let binary_name = executable
        .word
        .rsplit('/')
        .next()
        .unwrap_or(executable.word);
    let tool = crate::tool::ToolKind::from_binary_name(binary_name);
    let config_home = match parsed.words.get(1) {
        None => None,
        Some(word) => {
            if parsed.words.len() > 2 {
                let offending = &parsed.words[2];
                return Err(client_error(
                    file,
                    label,
                    line,
                    format!(
                        "offending word '{offending}': flags and extra words belong to [profiles]; quote config_home paths containing spaces"
                    ),
                ));
            }
            Some(parse_client_home(file, label, line, word, tool)?)
        }
    };
    Ok(Client {
        executable: executable_source.to_owned(),
        config_home,
        tool,
    })
}

fn parse_client_home(
    file: &Path,
    label: &str,
    line: usize,
    word: &str,
    tool: crate::tool::ToolKind,
) -> Result<String, ConfigError> {
    use std::cell::Cell;

    let Some(raw_path) = word.strip_prefix("config_home=") else {
        return Err(client_error(
            file,
            label,
            line,
            format!("offending word '{word}': only config_home=<path> may follow the executable"),
        ));
    };
    if outer_quote_body(raw_path).starts_with('~') {
        return Err(client_error(
            file,
            label,
            line,
            format!("offending word '{word}': config_home must be absolute or $HOME-rooted"),
        ));
    }
    let other_variable = Cell::new(false);
    let expanded = crate::words::split(raw_path, &|name| {
        if name == "HOME" {
            Some("/__ae_home__".to_owned())
        } else {
            other_variable.set(true);
            None
        }
    })
    .map_err(|reason| client_error(file, label, line, reason))?;
    if other_variable.get() {
        return Err(client_error(
            file,
            label,
            line,
            format!("offending word '{word}': config_home may reference only $HOME or ${{HOME}}"),
        ));
    }
    let Some(expanded) = expanded.first().filter(|_| expanded.len() == 1) else {
        return Err(client_error(
            file,
            label,
            line,
            format!("offending word '{word}': config_home must be one path word"),
        ));
    };
    if !Path::new(expanded).is_absolute() {
        return Err(client_error(
            file,
            label,
            line,
            format!("offending word '{word}': config_home must be absolute or $HOME-rooted"),
        ));
    }
    if tool.adapter().config_home_env.is_none() {
        return Err(client_error(
            file,
            label,
            line,
            format!(
                "config_home is not supported for {}: its account variable is unverified",
                tool.as_str()
            ),
        ));
    }
    if tool
        .adapter()
        .config_home_default
        .is_some_and(|default| Path::new(expanded) == Path::new("/__ae_home__").join(default))
    {
        return Err(client_error(
            file,
            label,
            line,
            default_client_home_refusal(tool),
        ));
    }
    Ok(raw_path.to_owned())
}

fn default_client_home_refusal(tool: crate::tool::ToolKind) -> String {
    tool.adapter().config_home_default_refusal.to_owned()
}

fn client_error(file: &Path, client: &str, line: usize, reason: String) -> ConfigError {
    ConfigError::ClientEntry {
        file: file.to_owned(),
        client: client.to_owned(),
        line,
        reason,
    }
}

/// Strip one matching outer quote pair for structural path checks only.
fn outer_quote_body(word: &str) -> &str {
    if word.len() >= 2
        && ((word.starts_with('\'') && word.ends_with('\''))
            || (word.starts_with('"') && word.ends_with('"')))
    {
        &word[1..word.len() - 1]
    } else {
        word
    }
}

/// Refuse raw profiles whose selected client and prefix fight over ownership
/// of the harness's account variable.
fn validate_client_conflicts(cfg: &IdentityConfig) -> Result<(), ConfigError> {
    for (profile, command) in &cfg.profiles {
        let Ok(parsed) = crate::launch_cmd::lex_simple_command(command) else {
            continue;
        };
        let binary = crate::launch_cmd::launch_binary(&parsed);
        if binary.word.contains('/') {
            continue;
        }
        let Some((label, client)) = cfg.clients.iter().find(|(key, _)| key == binary.word) else {
            continue;
        };
        if client.config_home.is_some() {
            let Some(variable) = client.tool.adapter().config_home_env else {
                continue;
            };
            if crate::launch_cmd::prefix_mentions(&parsed, variable) {
                return Err(ConfigError::ClientEnvConflict {
                    profile: profile.clone(),
                    client: label.clone(),
                    variable: variable.to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn expand_client_home(
    profile: &str,
    client: &str,
    raw: &str,
    tool: crate::tool::ToolKind,
    home: Option<&Path>,
) -> Result<String, ConfigError> {
    use std::cell::Cell;

    let home_unavailable = Cell::new(false);
    let expanded = crate::words::split(raw, &|name| {
        if name == "HOME" {
            if let Some(path) = home {
                Some(path.to_string_lossy().into_owned())
            } else {
                home_unavailable.set(true);
                None
            }
        } else {
            None
        }
    })
    .map_err(|reason| ConfigError::ClientHome {
        profile: profile.to_owned(),
        client: client.to_owned(),
        reason,
    })?;
    if home_unavailable.get() {
        return Err(ConfigError::ClientHome {
            profile: profile.to_owned(),
            client: client.to_owned(),
            reason: "HOME unavailable".to_owned(),
        });
    }
    let Some(path) = expanded.first().filter(|_| expanded.len() == 1) else {
        return Err(ConfigError::ClientHome {
            profile: profile.to_owned(),
            client: client.to_owned(),
            reason: "config_home did not resolve to one path".to_owned(),
        });
    };
    if !Path::new(path).is_absolute() {
        return Err(ConfigError::ClientHome {
            profile: profile.to_owned(),
            client: client.to_owned(),
            reason: "config_home did not resolve to an absolute path".to_owned(),
        });
    }
    if let (Some(home), Some(default)) = (home, tool.adapter().config_home_default)
        && Path::new(path) == home.join(default)
    {
        return Err(ConfigError::ClientHome {
            profile: profile.to_owned(),
            client: client.to_owned(),
            reason: default_client_home_refusal(tool),
        });
    }
    Ok(path.clone())
}

/// Replace `key`'s value in place (keeping its position), else append.
fn upsert(rows: &mut Vec<(String, String)>, key: &str, value: String) {
    match rows.iter_mut().find(|(k, _)| k == key) {
        Some((_, existing)) => *existing = value,
        None => rows.push((key.to_owned(), value)),
    }
}

fn upsert_client(rows: &mut Vec<(String, Client)>, key: &str, value: Client) {
    match rows.iter_mut().find(|(k, _)| k == key) {
        Some((_, existing)) => *existing = value,
        None => rows.push((key.to_owned(), value)),
    }
}

/// One seat of a launch: the slot it takes, its identity, and the profile
/// resolved into the command it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seat {
    /// `main` or `worker.<n>`.
    pub slot: String,
    /// The agent's name — its identity.
    pub name: String,
    /// The profile bound in `[roster]`.
    pub profile: String,
    /// The profile's launch command after one-shot client expansion.
    pub command: ResolvedCommand,
    /// The RAW leading-assignment span (`cmd.assign`), byte-exact from the
    /// command — empty when there are none.
    pub assign_span: String,
    /// The RAW argv span (`cmd.argv`), byte-exact from the command.
    pub argv_span: String,
    /// The binary name the validated parse found (`agent_bin.<slot>`), path
    /// stripped, `env` prefix peeled.
    pub binary: String,
    /// Which harness that is.
    pub tool: crate::tool::ToolKind,
}

/// The seats a launch will create, main first, workers in config order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    /// `seats[0]` is the main seat.
    pub seats: Vec<Seat>,
}

/// One way the workspace roster is not launchable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// `[workspace] main` never set (and no override).
    MainMissing,
    /// `[workspace] workers` has an empty entry (`a,,b`, a trailing comma).
    EmptyWorker {
        /// 1-based position in the list.
        position: usize,
    },
    /// A seat name outside the agent-name grammar (an `alias:name` included).
    BadName {
        /// `workspace.main`, `workspace.workers` or `use`.
        seat: String,
        /// The name as written.
        name: String,
    },
    /// A seat name with no `[roster]` binding.
    NotInRoster {
        /// Where it was named.
        seat: String,
        /// The name.
        name: String,
    },
    /// A name taking two seats.
    NameTwice {
        /// Where the repeat was named.
        seat: String,
        /// The name.
        name: String,
    },
    /// A roster binding to a profile `[profiles]` does not define.
    ProfileMissing {
        /// The roster name.
        name: String,
        /// The profile it names.
        profile: String,
    },
    /// A launch command that is not ONE SIMPLE COMMAND (the command execution
    /// contract): an operator, comment, redirection, substitution or grouping
    /// outside quotes would detach the fixed suffix; a malformed line or one
    /// with no command word cannot run at all.
    CommandRefused {
        /// The seat name.
        name: String,
        /// The profile whose command failed.
        profile: String,
        /// Why.
        why: crate::launch_cmd::Refusal,
    },
    /// A selected client command could not resolve its config home.
    CommandResolution {
        /// The seat name.
        name: String,
        /// The profile whose command failed.
        profile: String,
        /// Why resolution failed.
        why: String,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MainMissing => {
                write!(
                    f,
                    "workspace.main is not set — name the standing main seat."
                )
            }
            Self::EmptyWorker { position } => {
                write!(
                    f,
                    "workspace.workers has an empty entry at position {position}."
                )
            }
            Self::BadName { seat, name } => write!(
                f,
                "{seat}: invalid agent name '{name}'. Names must match {AGENT_NAME_GRAMMAR} — a v2 seat is a bare name bound in [roster], never alias:name."
            ),
            Self::NotInRoster { seat, name } => {
                write!(f, "{seat}: '{name}' is not bound to a profile in [roster].")
            }
            Self::NameTwice { seat, name } => write!(
                f,
                "{seat}: '{name}' is named more than once across main and workers — a name is one seat."
            ),
            Self::ProfileMissing { name, profile } => write!(
                f,
                "[roster] {name} = {profile}: profile '{profile}' is not defined in [profiles]."
            ),
            Self::CommandRefused { name, profile, why } => write!(
                f,
                "[profiles] {profile} (seat '{name}'): the launch command must be one simple command — it has {why}."
            ),
            Self::CommandResolution { name, profile, why } => write!(
                f,
                "[profiles] {profile} (seat '{name}'): the client command could not be resolved — {why}."
            ),
        }
    }
}

/// The refusal a launcher prints: one line per violation, in config order.
#[must_use]
pub fn render_violations(violations: &[Violation]) -> String {
    let mut out = String::from("Error: the workspace roster is not launchable:\n");
    for violation in violations {
        out.push_str("  - ");
        out.push_str(&violation.to_string());
        out.push('\n');
    }
    out
}

/// One named seat → its [`Seat`], or `Ok(None)` when the seat's profile is
/// undefined (its [`Violation::ProfileMissing`] is raised by the independent
/// roster pass in [`launch_plan`], so this returns no seat and no duplicate).
fn resolve_seat(
    cfg: &IdentityConfig,
    seat: &str,
    name: &str,
    slot: String,
    home: Option<&Path>,
) -> Result<Option<Seat>, Violation> {
    let Some(profile) = cfg.roster_profile(name) else {
        return Err(Violation::NotInRoster {
            seat: seat.to_owned(),
            name: name.to_owned(),
        });
    };
    let command = cfg.command(profile, home).map_err(|error| {
        let why = match error {
            ConfigError::ClientHome { client, reason, .. } => {
                format!("client '{client}': {reason}")
            }
            other => other.to_string(),
        };
        Violation::CommandResolution {
            name: name.to_owned(),
            profile: profile.to_owned(),
            why,
        }
    })?;
    let Some(command) = command else {
        // Undefined profile: reported once by the independent roster pass.
        return Ok(None);
    };
    let parsed = crate::launch_cmd::lex_simple_command(command.as_str()).map_err(|why| {
        Violation::CommandRefused {
            name: name.to_owned(),
            profile: profile.to_owned(),
            why,
        }
    })?;
    let tool = parsed.tool();
    Ok(Some(Seat {
        slot,
        name: name.to_owned(),
        profile: profile.to_owned(),
        command,
        assign_span: parsed.assign_span,
        argv_span: parsed.argv_span,
        binary: parsed.binary,
        tool,
    }))
}

/// Resolve the workspace seats into a [`LaunchPlan`], validating both
/// directions: every seat name is in the grammar, bound once in `[roster]`,
/// bound to a defined and lexable profile, and named for one seat only; and
/// every `[roster]` row names a defined profile (a row bound to no seat is
/// legal — it is what `use <name>` selects).
///
/// # Errors
///
/// Every [`Violation`] found, in the order the config states things — never
/// just the first.
pub fn launch_plan(
    cfg: &IdentityConfig,
    main_override: Option<&str>,
    home: Option<&Path>,
) -> Result<LaunchPlan, Vec<Violation>> {
    let mut violations = Vec::new();
    let mut named: Vec<(String, String)> = Vec::new(); // (seat label, name)
    let main_seat = if main_override.is_some() {
        "use"
    } else {
        "workspace.main"
    };
    match main_override
        .map(str::trim)
        .or(cfg.main.as_deref().map(str::trim))
    {
        Some(name) if !name.is_empty() => named.push((main_seat.to_owned(), name.to_owned())),
        _ => violations.push(Violation::MainMissing),
    }
    if let Some(workers) = cfg.workers.as_deref() {
        for (index, entry) in workers.split(',').enumerate() {
            let name = entry.trim();
            if name.is_empty() {
                // A lone empty value (`workers =` never parses; but `workers = ""`
                // does) means "no workers", not an empty seat.
                if workers.trim().is_empty() {
                    break;
                }
                violations.push(Violation::EmptyWorker {
                    position: index + 1,
                });
                continue;
            }
            named.push(("workspace.workers".to_owned(), name.to_owned()));
        }
    }
    // Validate EVERY roster profile binding independently, in config order,
    // BEFORE resolving seats — so an unseated row with a missing profile is still
    // reported (colead IMPORTANT-4), and a seat's profile is reported exactly once.
    for (name, profile) in &cfg.roster {
        if cfg.profile(profile).is_none() {
            violations.push(Violation::ProfileMissing {
                name: name.clone(),
                profile: profile.clone(),
            });
        }
    }
    let mut seats = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    let mut worker_index = 0usize;
    for (seat, name) in &named {
        if !is_agent_name(name) {
            violations.push(Violation::BadName {
                seat: seat.clone(),
                name: name.clone(),
            });
            continue;
        }
        if seen.contains(&name.as_str()) {
            violations.push(Violation::NameTwice {
                seat: seat.clone(),
                name: name.clone(),
            });
            continue;
        }
        seen.push(name);
        let slot = if seat == "workspace.workers" {
            format!("worker.{worker_index}")
        } else {
            "main".to_owned()
        };
        match resolve_seat(cfg, seat, name, slot, home) {
            // A resolved seat consumes its worker index; a profile-missing seat
            // (Ok(None), already reported above) does not create a worker.
            Ok(Some(resolved)) => {
                if seat == "workspace.workers" {
                    worker_index += 1;
                }
                seats.push(resolved);
            }
            Ok(None) => {}
            Err(violation) => violations.push(violation),
        }
    }
    // A roster row bound to no seat is NOT a violation (ruled 2026-09-02, reversing
    // the v5 "dormant refuses" ruling): `[roster]` is the set of named agents this
    // workspace MAY launch, main/workers pick the defaults, and `use <name>` picks
    // another — which is only possible if an unseated row is legal. Its profile is
    // still validated above, so a typo in the binding is still caught.
    if violations.is_empty() {
        Ok(LaunchPlan { seats })
    } else {
        Err(violations)
    }
}

/// Read arbitrary `[workspace]` keys, layering `local` over `global` — the
/// launch operation's own `get_config workspace.<key>` reads (`layout`, `copy`,
/// `watchdog`/`loop`, `orchestrator`/`hub`/`meta`).
#[must_use]
pub fn read_workspace_keys(
    global: Option<&Path>,
    local: Option<&Path>,
    keys: &[&str],
) -> Vec<Option<String>> {
    read_workspace_keys_with_identity_sections(global, local, keys).0
}

/// Read one global `[workspace]` key while preserving an explicit malformed
/// value. Generic config overlays intentionally skip malformed entries; a
/// default-on safety policy must distinguish those entries from absence.
///
/// # Errors
///
/// The selected file could not be read, or an explicit occurrence of `key`
/// does not use the shared config entry grammar.
pub fn read_global_workspace_key(file: &Path, key: &str) -> Result<Option<String>, String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: reads the same selected global INI config as the frozen config parser — see clippy.toml"
    )]
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(why) => return Err(format!("could not read global config: {why}")),
    };
    let mut section = String::new();
    let mut found = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = section_header(line) {
            section = name;
            continue;
        }
        if section != "workspace" || line.starts_with('#') {
            continue;
        }
        if key_claim(line) == Some(key) {
            found = Some(
                parse_entry(line)
                    .filter(|(parsed, _)| *parsed == key)
                    .map(|(_, value)| value)
                    .ok_or_else(|| format!("{key} has an invalid value")),
            );
            continue;
        }
        if let Some(rest) = line.strip_prefix(key)
            && (rest.is_empty()
                || rest
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_whitespace()))
        {
            found = Some(Err(format!("{key} is malformed")));
        }
    }
    found.transpose()
}

/// Read `[workspace]` keys and report whether the local file carries legacy
/// identity sections. The latter are metadata for compatibility notices only:
/// callers that own identity globally must not parse or overlay those rows.
#[must_use]
pub fn read_workspace_keys_with_identity_sections(
    global: Option<&Path>,
    local: Option<&Path>,
    keys: &[&str],
) -> (Vec<Option<String>>, bool, bool) {
    let mut found: Vec<Option<String>> = vec![None; keys.len()];
    let mut local_has_profiles = false;
    let mut local_has_roster = false;
    for (is_local, file) in [(false, global), (true, local)] {
        let Some(file) = file else { continue };
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: reads the INI config the frozen parse_config reads — see clippy.toml"
        )]
        let read = std::fs::read_to_string(file);
        let Ok(text) = read else { continue };
        let mut section = String::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = section_header(line) {
                if is_local {
                    local_has_profiles |= name == "profiles";
                    local_has_roster |= name == "roster";
                }
                section = name;
                continue;
            }
            if section != "workspace" {
                continue;
            }
            if let Some((key, value)) = parse_entry(line)
                && let Some(index) = keys.iter().position(|wanted| *wanted == key)
            {
                found[index] = Some(value);
            }
        }
    }
    (found, local_has_profiles, local_has_roster)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// Write `text` to a fresh temp file and return its path (kept alive by the
    /// returned `NamedTemp`).
    struct NamedTemp(std::path::PathBuf);
    impl NamedTemp {
        fn new(tag: &str, text: &str) -> Self {
            // Unique per INSTANCE (pid + atomic counter), not per tag: plain
            // `cargo test` runs these in threads, and a shared `v2` path made
            // tests overwrite each other's file (colead round-2 IMPORTANT-3).
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "ae-config-{tag}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let mut f = std::fs::File::create(&path).expect("temp config");
            f.write_all(text.as_bytes()).expect("write config");
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for NamedTemp {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// A fresh directory tree removed with the test that owns it.
    struct NamedDir(std::path::PathBuf);
    impl NamedDir {
        fn new(tag: &str) -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "ae-config-dir-{tag}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("temp config tree");
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for NamedDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn local_overlay_prefers_the_record_then_falls_back_then_answers_none() {
        let recorded = NamedDir::new("recorded");
        let recorded_session = recorded.path().join("session");
        let recorded_path = recorded.path().join("seat.config");
        std::fs::create_dir_all(&recorded_session).expect("session dir");
        std::fs::write(
            recorded_session.join("meta"),
            format!("{}={}\n", LOCAL_CONFIG_KEY, recorded_path.display()),
        )
        .expect("recorded overlay");
        assert_eq!(
            local_overlay(&recorded_session, "/ignored/origin"),
            Some(recorded_path),
            "a recorded path wins even when it is currently missing"
        );

        let fallback = NamedDir::new("fallback");
        let fallback_session = fallback.path().join("session");
        let fallback_origin = fallback.path().join("origin");
        let fallback_path = fallback_origin.join(".ae/config");
        std::fs::create_dir_all(&fallback_session).expect("session dir");
        std::fs::create_dir_all(fallback_path.parent().expect("config parent"))
            .expect("local config dir");
        std::fs::write(&fallback_path, "").expect("local config");
        assert_eq!(
            local_overlay(&fallback_session, &fallback_origin.display().to_string()),
            Some(fallback_path)
        );

        let absent = NamedDir::new("absent");
        let absent_session = absent.path().join("session");
        let absent_origin = absent.path().join("origin");
        std::fs::create_dir_all(&absent_session).expect("session dir");
        assert_eq!(
            local_overlay(&absent_session, &absent_origin.display().to_string()),
            None
        );
    }

    #[test]
    fn extracts_workspace_main_workers_and_purge() {
        let c = NamedTemp::new(
            "basic",
            "[agents]\ncl = \"claude\"\n[workspace]\nmain = cl\nworkers = a, b\npurge_agent_history = true\n",
        );
        let w = read_workspace(Some(c.path()), None).expect("readable config");
        assert_eq!(w.main.as_deref(), Some("cl"));
        assert_eq!(w.workers.as_deref(), Some("a, b"));
        assert!(w.purge_agent_history);
    }

    #[test]
    fn keys_outside_workspace_are_ignored() {
        let c = NamedTemp::new(
            "sections",
            "[agents]\nmain = not-this\n[prompt]\nworkers = nor-this\n",
        );
        let w = read_workspace(Some(c.path()), None).expect("readable config");
        assert_eq!(w.main, None);
        assert_eq!(w.workers, None);
        assert!(!w.purge_agent_history);
    }

    #[test]
    fn a_quoted_value_keeps_its_inner_bytes_and_hash() {
        let c = NamedTemp::new("quoted", "[workspace]\nmain = \"cl #1\"\n");
        let w = read_workspace(Some(c.path()), None).expect("readable config");
        assert_eq!(w.main.as_deref(), Some("cl #1"));
    }

    #[test]
    fn an_unquoted_value_strips_a_trailing_comment() {
        let c = NamedTemp::new("comment", "[workspace]\nmain = cl   # the main\n");
        let w = read_workspace(Some(c.path()), None).expect("readable config");
        assert_eq!(w.main.as_deref(), Some("cl"));
    }

    #[test]
    fn local_overrides_global_last_wins() {
        let g = NamedTemp::new("g", "[workspace]\nmain = global\nworkers = gw\n");
        let l = NamedTemp::new("l", "[workspace]\nmain = local\n");
        let w = read_workspace(Some(g.path()), Some(l.path())).expect("readable config");
        // local set main → local wins; local left workers → global's survives.
        assert_eq!(w.main.as_deref(), Some("local"));
        assert_eq!(w.workers.as_deref(), Some("gw"));
    }

    #[test]
    fn absence_is_none_but_a_selected_unreadable_file_refuses() {
        // Absence is expressed by passing `None`, not by a path that fails to read.
        let w = read_workspace(None, None).expect("no files is empty, not an error");
        assert_eq!(w.main, None);
        assert!(!w.purge_agent_history);
        // A file the caller SELECTED (Some) that cannot be read fails closed —
        // this is the purge-bypass guard: a present-but-unreadable config must
        // never read as empty.
        let missing = Path::new("/no/such/config");
        assert_eq!(
            read_workspace(Some(missing), None).unwrap_err(),
            missing.to_path_buf(),
        );
    }

    #[test]
    fn a_present_but_unreadable_config_refuses() {
        // A real regular file whose bytes cannot be read (here: not valid UTF-8) must
        // refuse, not silently contribute nothing — the config carried purge=true.
        let c = NamedTemp::new("unreadable", "");
        std::fs::write(c.path(), [0xff, 0xfe, 0x00, 0x9c]).expect("write non-utf8");
        assert_eq!(
            read_workspace(Some(c.path()), None).unwrap_err(),
            c.path().to_path_buf(),
        );
    }

    #[test]
    fn purge_truthy_and_falsey_values() {
        for v in ["true", "1", "yes", "on"] {
            let c = NamedTemp::new("pt", &format!("[workspace]\npurge_agent_history = {v}\n"));
            assert!(
                read_workspace(Some(c.path()), None)
                    .expect("readable config")
                    .purge_agent_history,
                "'{v}' is truthy"
            );
        }
        for v in ["false", "0", "no", "off", "TRUE", "maybe"] {
            let c = NamedTemp::new("pf", &format!("[workspace]\npurge_agent_history = {v}\n"));
            assert!(
                !read_workspace(Some(c.path()), None)
                    .expect("readable config")
                    .purge_agent_history,
                "'{v}' is not truthy"
            );
        }
    }

    // ---- identity v2 ---------------------------------------------------

    const V2: &str = "[profiles]\n\
        fable5 = \"claude --permission-mode bypassPermissions --model fable --effort xhigh\"\n\
        gpt56sol = \"codex --yolo -m gpt-5.6-sol -c model_reasoning_effort=xhigh\"\n\
        mic = \"CLAUDE_CONFIG_DIR=$HOME/.claude-mic claude --model fable\"\n\
        [roster]\n\
        lead = fable5\n\
        colead = gpt56sol\n\
        [workspace]\n\
        main = lead\n\
        workers = colead\n\
        layout = lead-pair\n";

    fn v2(text: &str) -> (NamedTemp, IdentityConfig) {
        let file = NamedTemp::new("v2", text);
        let cfg = read_identity(Some(file.path()), None).expect("readable v2 config");
        (file, cfg)
    }

    #[test]
    fn the_agent_name_grammar_is_the_frozen_one() {
        for ok in ["lead", "2nd", "a", "x-y_z", &"n".repeat(64), "A9"] {
            assert!(is_agent_name(ok), "{ok:?}");
        }
        for bad in [
            "",
            "_x",
            "-x",
            "a:b",
            "a b",
            "ä",
            &"n".repeat(65),
            "a/b",
            "a.b",
        ] {
            assert!(!is_agent_name(bad), "{bad:?}");
        }
        assert_eq!(AGENT_NAME_GRAMMAR, "^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$");
    }

    #[test]
    fn identity_reads_profiles_roster_and_seats_in_order() {
        let (_f, cfg) = v2(V2);
        assert_eq!(
            cfg.profiles
                .iter()
                .map(|(k, _)| k.as_str())
                .collect::<Vec<_>>(),
            ["fable5", "gpt56sol", "mic"]
        );
        assert_eq!(
            cfg.profile("mic"),
            Some("CLAUDE_CONFIG_DIR=$HOME/.claude-mic claude --model fable"),
            "a quoted value keeps its inner bytes, $HOME included"
        );
        assert_eq!(
            cfg.roster,
            [
                ("lead".to_owned(), "fable5".to_owned()),
                ("colead".to_owned(), "gpt56sol".to_owned())
            ]
        );
        assert_eq!(cfg.main.as_deref(), Some("lead"));
        assert_eq!(cfg.workers.as_deref(), Some("colead"));
        let plan = launch_plan(&cfg, None, None).expect("launchable");
        assert_eq!(plan.seats.len(), 2);
        assert_eq!(plan.seats[0].slot, "main");
        assert_eq!(plan.seats[0].name, "lead");
        assert_eq!(plan.seats[0].profile, "fable5");
        assert_eq!(plan.seats[0].binary, "claude");
        assert_eq!(plan.seats[0].tool, crate::tool::ToolKind::Claude);
        assert_eq!(plan.seats[1].slot, "worker.0");
        assert_eq!(plan.seats[1].name, "colead");
        assert_eq!(plan.seats[1].tool, crate::tool::ToolKind::Codex);
        assert!(plan.seats[1].command.as_str().starts_with("codex --yolo"));
    }

    fn command(cfg: &IdentityConfig, profile: &str, home: Option<&Path>) -> String {
        cfg.command(profile, home)
            .expect("client path resolves")
            .expect("profile exists")
            .into_string()
    }

    #[test]
    fn clients_expand_the_executor_selected_full_word_once() {
        let (_f, cfg) = v2("[clients]\ncc = /opt/claude\nclaude = codex\n\
             [profiles]\ndirect = cc --model fable\nraw = other --flag\n\
             path = /usr/bin/cc --flag\nenv = env -i -u KEEP cc --effort high\n\
             unicode = NOTE=grüß cc --unicode\nquoted = 'cc' --quoted\nescaped = c\\c --escaped\n\
             [roster]\nlead = direct\n[workspace]\nmain = lead\n");
        assert_eq!(command(&cfg, "direct", None), "/opt/claude --model fable");
        assert_eq!(command(&cfg, "raw", None), "other --flag");
        assert_eq!(command(&cfg, "path", None), "/usr/bin/cc --flag");
        assert_eq!(
            command(&cfg, "env", None),
            "env -i -u KEEP /opt/claude --effort high"
        );
        assert_eq!(
            command(&cfg, "unicode", None),
            "NOTE=grüß /opt/claude --unicode",
            "rewriting a later word uses byte offsets, not character offsets"
        );
        assert_eq!(command(&cfg, "quoted", None), "/opt/claude --quoted");
        assert_eq!(command(&cfg, "escaped", None), "/opt/claude --escaped");
        let (_f, no_chain) = v2(
            "[clients]\ncc = claude\nclaude = codex\n[profiles]\np = cc --x\n\
             [roster]\nlead = p\n[workspace]\nmain = lead\n",
        );
        assert_eq!(
            command(&no_chain, "p", None),
            "claude --x",
            "the replacement executable is never looked up as another label"
        );

        let (_f, cfg) = v2(
            "[clients]\nclaude = /custom/claude\n[profiles]\np = claude --x\n\
             [roster]\nlead = p\n[workspace]\nmain = lead\n",
        );
        assert_eq!(command(&cfg, "p", None), "/custom/claude --x");
    }

    #[test]
    fn client_config_homes_insert_the_owned_variable_before_the_executable() {
        let (_f, cfg) = v2(
            "[clients]\ncc = claude config_home=\"$HOME/Library/Application Support/Claude\"\n\
             cx = codex config_home=/srv/codex\ncxhome = codex config_home=${HOME}/.codex-work\n\
             [profiles]\nfable = cc --model fable\nsol = env -i cx -m sol\nsolhome = cxhome -m sol\n\
             [roster]\nlead = fable\n[workspace]\nmain = lead\n",
        );
        assert_eq!(
            command(&cfg, "fable", Some(Path::new("/Users/a"))),
            "CLAUDE_CONFIG_DIR='/Users/a/Library/Application Support/Claude' claude --model fable"
        );
        assert_eq!(
            command(&cfg, "sol", Some(Path::new("/Users/a"))),
            "env -i CODEX_HOME='/srv/codex' codex -m sol"
        );
        assert_eq!(
            command(&cfg, "solhome", Some(Path::new("/Users/a"))),
            "CODEX_HOME='/Users/a/.codex-work' codex -m sol"
        );
        let err = cfg.command("fable", None).unwrap_err();
        assert!(err.to_string().contains("HOME unavailable"), "{err}");
    }

    #[test]
    fn client_resolution_uses_the_fully_overlaid_definition() {
        let global = NamedTemp::new(
            "client-overlay-global",
            "[clients]\ncc = claude config_home=/global\n\
             [profiles]\np = CLAUDE_CONFIG_DIR=/profile cc --x\n",
        );
        let local = NamedTemp::new(
            "client-overlay-local",
            "[clients]\ncc = codex config_home=${HOME}/local\n",
        );
        let cfg = read_identity(Some(global.path()), Some(local.path())).expect("overlaid config");
        assert_eq!(
            command(&cfg, "p", Some(Path::new("/home/a"))),
            "CLAUDE_CONFIG_DIR=/profile CODEX_HOME='/home/a/local' codex --x"
        );
    }

    #[test]
    fn invalid_client_values_refuse_at_config_load_with_the_offending_word() {
        for (value, offending) in [
            ("env -i claude", "env"),
            ("A=1 claude", "A=1"),
            ("A=1", "A=1"),
            ("claude --model", "--model"),
            ("./claude", "./claude"),
            ("claude config_home=relative", "config_home=relative"),
            ("claude config_home=~/.claude", "config_home=~/.claude"),
            ("claude config_home=$OTHER/x", "config_home=$OTHER/x"),
        ] {
            let file = NamedTemp::new("bad-client", &format!("[clients]\ncc = {value}\n"));
            let err = read_identity(Some(file.path()), None).unwrap_err();
            let line = err.to_string();
            assert!(line.contains(offending), "{value:?}: {line}");
        }
        let file = NamedTemp::new("env-label", "[clients]\nenv = claude\n");
        assert!(
            read_identity(Some(file.path()), None)
                .unwrap_err()
                .to_string()
                .contains("label is reserved prefix syntax")
        );
        let empty = NamedTemp::new("empty-env-label", "[clients]\nenv =\n");
        assert!(
            read_identity(Some(empty.path()), None)
                .unwrap_err()
                .to_string()
                .contains("label is reserved prefix syntax")
        );
        let key = NamedTemp::new("bad-client-label", "[clients]\n_bad = claude\n");
        assert!(matches!(
            read_identity(Some(key.path()), None).unwrap_err(),
            ConfigError::ClientKey { .. }
        ));
    }

    #[test]
    fn unsupported_client_homes_use_the_frozen_refusal() {
        for tool in ["grok", "agy", "opencode", "gemini"] {
            let file = NamedTemp::new(
                "unsupported-home",
                &format!("[clients]\nx = {tool} config_home=/tmp/{tool}\n"),
            );
            let err = read_identity(Some(file.path()), None).unwrap_err();
            assert!(
                err.to_string().contains(&format!(
                    "config_home is not supported for {tool}: its account variable is unverified"
                )),
                "{err}"
            );
        }
    }

    #[test]
    fn a_client_cannot_relocate_to_its_tools_default_home() {
        for (tool, default) in [("claude", ".claude"), ("codex", ".codex")] {
            let rooted = NamedTemp::new(
                "client-default-home",
                &format!("[clients]\nx = {tool} config_home=$HOME/{default}\n"),
            );
            let err = read_identity(Some(rooted.path()), None).unwrap_err();
            assert!(
                err.to_string().contains("use the default client"),
                "{tool}: {err}"
            );

            let absolute = NamedTemp::new(
                "client-absolute-default-home",
                &format!(
                    "[clients]\nx = {tool} config_home=/home/person/{default}\n\
                     [profiles]\np = \"x\"\n"
                ),
            );
            let cfg =
                read_identity(Some(absolute.path()), None).expect("structurally valid config");
            let err = cfg
                .command("p", Some(Path::new("/home/person")))
                .unwrap_err();
            assert!(
                err.to_string().contains("use the default client"),
                "{tool}: {err}"
            );
        }
    }

    #[test]
    fn a_profile_cannot_assign_or_unset_its_clients_account_variable() {
        for command in [
            "CLAUDE_CONFIG_DIR=/other cc",
            "env -u CLAUDE_CONFIG_DIR cc",
            "env CLAUDE_CONFIG_DIR=/other cc",
        ] {
            let file = NamedTemp::new(
                "client-conflict",
                &format!("[clients]\ncc = claude config_home=/client\n[profiles]\np = {command}\n"),
            );
            let err = read_identity(Some(file.path()), None).unwrap_err();
            assert!(
                matches!(err, ConfigError::ClientEnvConflict { ref variable, .. } if variable == "CLAUDE_CONFIG_DIR"),
                "{command:?}: {err}"
            );
        }
    }

    #[test]
    fn a_same_file_duplicate_client_label_refuses_like_a_duplicate_profile() {
        let file = NamedTemp::new("dup-client", "[clients]\ncc = claude\ncc = codex\n");
        assert!(matches!(
            read_identity(Some(file.path()), None).unwrap_err(),
            ConfigError::DuplicateKey { section, key, line: 3, .. }
                if section == "clients" && key == "cc"
        ));
    }

    #[test]
    fn client_parse_and_command_resolution_are_deterministic_for_10000_inputs() {
        let mut state = 0x4d59_5df4_d0f3_3173_u64;
        for case in 0..10_000 {
            let len = usize::try_from(state % 96).unwrap_or_default() + 1;
            let mut bytes = Vec::with_capacity(len);
            for _ in 0..len {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                bytes.push(state.to_le_bytes()[0]);
            }
            let arbitrary = String::from_utf8_lossy(&bytes).into_owned();
            let path = Path::new("property");
            assert_eq!(
                parse_client(path, "cc", case + 1, &arbitrary),
                parse_client(path, "cc", case + 1, &arbitrary)
            );

            let cfg = IdentityConfig {
                clients: vec![(
                    "cc".to_owned(),
                    Client {
                        executable: "claude".to_owned(),
                        config_home: None,
                        tool: crate::tool::ToolKind::Claude,
                    },
                )],
                profiles: vec![
                    ("candidate".to_owned(), format!("cc {arbitrary}")),
                    ("variable".to_owned(), arbitrary.clone()),
                    ("raw".to_owned(), format!("/raw/tool {arbitrary}")),
                ],
                ..IdentityConfig::default()
            };
            assert_eq!(
                cfg.command("candidate", Some(Path::new("/home/a"))),
                cfg.command("candidate", Some(Path::new("/home/a")))
            );
            assert_eq!(
                cfg.command("variable", Some(Path::new("/home/a"))),
                cfg.command("variable", Some(Path::new("/home/a")))
            );
            let raw = cfg.profile("raw").unwrap_or_default();
            let resolved = cfg
                .command("raw", Some(Path::new("/home/a")))
                .expect("raw resolution cannot fail")
                .expect("raw profile exists");
            assert_eq!(resolved.as_str(), raw, "case {case}");
        }
    }

    #[test]
    fn an_absent_global_can_be_parsed_from_a_default_without_writing_it() {
        let root = NamedDir::new("identity-default");
        let missing = root.path().join("config");
        let cfg = read_identity_with_global_default(
            Some(&missing),
            None,
            "[profiles]\nfable = \"claude\"\n[roster]\nlead = fable\n[workspace]\nmain = lead\n",
        )
        .expect("the embedded default parses");
        assert_eq!(cfg.roster_profile("lead"), Some("fable"));
        assert_eq!(cfg.profile("fable"), Some("claude"));
        #[allow(
            clippy::disallowed_methods,
            reason = "the unit fixture proves its absent input stayed absent"
        )]
        let missing_stayed_absent = !missing.exists();
        assert!(
            missing_stayed_absent,
            "parsing the fallback has zero effects"
        );
    }

    #[test]
    fn identity_overlay_is_key_wise_with_local_winning_and_order_kept() {
        let g = NamedTemp::new(
            "g2",
            "[profiles]\na = \"one\"\nb = \"two\"\n[roster]\nlead = a\n[workspace]\nmain = lead\n",
        );
        let l = NamedTemp::new(
            "l2",
            "[profiles]\nb = \"TWO\"\nc = \"three\"\n[roster]\nlead = b\n",
        );
        let cfg = read_identity(Some(g.path()), Some(l.path())).expect("readable");
        assert_eq!(
            cfg.profiles,
            [
                ("a".to_owned(), "one".to_owned()),
                ("b".to_owned(), "TWO".to_owned()),
                ("c".to_owned(), "three".to_owned())
            ],
            "local replaces b in place and appends c"
        );
        assert_eq!(cfg.roster_profile("lead"), Some("b"));
        assert_eq!(cfg.main.as_deref(), Some("lead"), "global's main survives");
        // The same key in BOTH files is an overlay, not a duplicate.
        assert!(launch_plan(&cfg, None, None).is_ok());
    }

    #[test]
    fn identity_refuses_a_same_file_duplicate_a_legacy_section_and_a_bad_roster_key() {
        let dup = NamedTemp::new("dup", "[profiles]\na = \"one\"\na = \"two\"\n");
        assert_eq!(
            read_identity(Some(dup.path()), None).unwrap_err(),
            ConfigError::DuplicateKey {
                file: dup.path().to_owned(),
                section: "profiles".to_owned(),
                key: "a".to_owned(),
                line: 3
            }
        );
        let dup = NamedTemp::new("dupw", "[workspace]\nmain = a\nlayout = x\nmain = b\n");
        assert!(matches!(
            read_identity(Some(dup.path()), None).unwrap_err(),
            ConfigError::DuplicateKey { section, key, line: 4, .. } if section == "workspace" && key == "main"
        ));
        // Keys outside the identity set never collide (the glue reads them).
        let ok = NamedTemp::new(
            "dupl",
            "[workspace]\nlayout = x\nlayout = y\n[prompt]\na = 1\na = 2\n",
        );
        assert!(read_identity(Some(ok.path()), None).is_ok());
        let legacy = NamedTemp::new("legacy", "[profiles]\na = \"x\"\n\n[agents]\nb = \"y\"\n");
        let err = read_identity(Some(legacy.path()), None).unwrap_err();
        assert_eq!(
            err,
            ConfigError::LegacyAgents {
                file: legacy.path().to_owned(),
                line: 4
            }
        );
        assert!(
            err.to_string().contains("[agents] is not a v2 section"),
            "{err}"
        );
        let bad = NamedTemp::new("badkey", "[roster]\n2nd = a\nno:colons = b\n");
        assert_eq!(
            read_identity(Some(bad.path()), None).unwrap_err(),
            ConfigError::RosterKey {
                file: bad.path().to_owned(),
                key: "no:colons".to_owned(),
                line: 3
            },
            "a digit-leading roster key is legal; a colon is not"
        );
        let missing = Path::new("/no/such/v2");
        assert_eq!(
            read_identity(Some(missing), None).unwrap_err(),
            ConfigError::Unreadable(missing.to_path_buf())
        );
        assert_eq!(
            read_identity(None, None).expect("absence is empty"),
            IdentityConfig::default()
        );
    }

    #[test]
    fn parse_identity_agrees_with_the_file_reader_and_names_its_source_config() {
        let text = "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n";
        let file = NamedTemp::new("pure", text);
        assert_eq!(
            parse_identity(text).expect("valid v2 text"),
            read_identity(Some(file.path()), None).expect("the same text, in a file"),
            "the pure parser is the one behind read_identity, not a second grammar"
        );
        // This input has no file, so a diagnostic names the source `<config>`.
        assert_eq!(
            parse_identity("[profiles]\na = \"one\"\na = \"two\"\n").unwrap_err(),
            ConfigError::DuplicateKey {
                file: Path::new("<config>").to_owned(),
                section: "profiles".to_owned(),
                key: "a".to_owned(),
                line: 3
            }
        );
    }

    #[test]
    fn identity_plan_collects_every_violation_before_refusing() {
        let text = "[profiles]\ncc = \"claude\"\nbroken = \"'unterminated\"\n\
            [roster]\nlead = cc\nghost = nope\nsleepy = cc\nbad = broken\n\
            [workspace]\nmain = lead\nworkers = lead, ghost, , nobody, x:y, bad\n";
        let (_f, cfg) = v2(text);
        let violations = launch_plan(&cfg, None, None).unwrap_err();
        assert_eq!(
            violations,
            [
                Violation::EmptyWorker { position: 3 },
                // ProfileMissing is checked in the independent roster pass, so
                // it precedes the per-seat violations (colead IMPORTANT-4).
                Violation::ProfileMissing {
                    name: "ghost".to_owned(),
                    profile: "nope".to_owned()
                },
                Violation::NameTwice {
                    seat: "workspace.workers".to_owned(),
                    name: "lead".to_owned()
                },
                Violation::NotInRoster {
                    seat: "workspace.workers".to_owned(),
                    name: "nobody".to_owned()
                },
                Violation::BadName {
                    seat: "workspace.workers".to_owned(),
                    name: "x:y".to_owned()
                },
                Violation::CommandRefused {
                    name: "bad".to_owned(),
                    profile: "broken".to_owned(),
                    why: crate::launch_cmd::Refusal::UnterminatedQuote,
                },
            ]
        );
        let rendered = render_violations(&violations);
        assert!(rendered.starts_with("Error: the workspace roster is not launchable:\n  - "));
        assert_eq!(rendered.lines().count(), 7);
        assert!(
            rendered.contains("'x:y'. Names must match ^[A-Za-z0-9]"),
            "{rendered}"
        );
        assert!(
            !rendered.contains("sleepy"),
            "an unseated roster row is legal (it is what `use` selects): {rendered}"
        );
        // No main at all.
        let (_f, cfg) =
            v2("[profiles]\ncc = \"claude\"\n[roster]\nlead = cc\n[workspace]\nworkers = lead\n");
        assert_eq!(
            launch_plan(&cfg, None, None).unwrap_err(),
            [Violation::MainMissing]
        );
    }

    #[test]
    fn an_unseated_roster_row_is_legal_but_its_profile_is_still_checked() {
        // Colead IMPORTANT-4 kept its half: the independent pass reports a
        // missing profile on a row nobody seats.
        let (_f, cfg) = v2(
            "[profiles]\ncc = \"claude\"\n[roster]\nlead = cc\nunused = missing\n\
             [workspace]\nmain = lead\n",
        );
        assert_eq!(
            launch_plan(&cfg, None, None).unwrap_err(),
            [Violation::ProfileMissing {
                name: "unused".to_owned(),
                profile: "missing".to_owned()
            }]
        );
        let (_f, cfg) = v2(
            "[profiles]\ncc = \"claude\"\n[roster]\nlead = cc\nspare = cc\n[workspace]\nmain = lead\n",
        );
        let plan = launch_plan(&cfg, None, None).expect("an unseated row is not a violation");
        assert_eq!(plan.seats.len(), 1);
        // And `use spare` seats it as main, displacing lead — the config's own main
        // is then the unseated row, and that is fine too.
        let plan = launch_plan(&cfg, Some("spare"), None).expect("`use` selects the unseated row");
        assert_eq!(plan.seats[0].name, "spare");
        assert_eq!(plan.seats[0].slot, "main");
    }

    #[test]
    fn a_roster_comment_and_an_empty_value_are_not_bad_names() {
        // A commented-out binding (an `=` inside it) is SKIPPED, never a hard
        // config refusal.
        let (_f, cfg) = v2(
            "[profiles]\ncc = \"claude\"\n[roster]\n# old = fable5\nlead = cc\n\
             [workspace]\nmain = lead\n",
        );
        assert!(
            launch_plan(&cfg, None, None).is_ok(),
            "the comment is ignored"
        );
        assert_eq!(cfg.roster_profile("lead"), Some("cc"));
        assert_eq!(cfg.roster_profile("old"), None, "the comment bound nothing");
        // A good key with an EMPTY value binds nothing — the seat then reports
        // NotInRoster, never `invalid agent name 'lead'`.
        let empty = NamedTemp::new(
            "emptyval",
            "[profiles]\ncc = \"claude\"\n[roster]\nlead =\n[workspace]\nmain = lead\n",
        );
        let cfg = read_identity(Some(empty.path()), None).expect("an empty value is not a bad key");
        assert_eq!(cfg.roster_profile("lead"), None);
        assert_eq!(
            launch_plan(&cfg, None, None).unwrap_err(),
            [Violation::NotInRoster {
                seat: "workspace.main".to_owned(),
                name: "lead".to_owned()
            }]
        );
        // An empty value hidden behind a comment (`lead = # note`) is the same.
        let commented = NamedTemp::new(
            "emptycomment",
            "[profiles]\ncc = \"claude\"\n[roster]\nlead = # note\n[workspace]\nmain = lead\n",
        );
        let cfg = read_identity(Some(commented.path()), None).expect("readable");
        assert_eq!(cfg.roster_profile("lead"), None);
        // A genuinely bad KEY still refuses.
        let bad = NamedTemp::new("badname", "[roster]\nno:colons = cc\n");
        assert!(matches!(
            read_identity(Some(bad.path()), None).unwrap_err(),
            ConfigError::RosterKey { key, .. } if key == "no:colons"
        ));
    }

    #[test]
    fn a_key_claimed_twice_refuses_even_when_one_claim_has_no_value() {
        // Colead round-2 IMPORTANT-1: the duplicate gate sees the raw claim.
        let cases = [
            ("profiles", "[profiles]\na =\na = \"x\"\n", "a"),
            ("profiles", "[profiles]\na = \"x\"\na =\n", "a"),
            ("roster", "[roster]\nlead =\nlead = cc\n", "lead"),
            ("roster", "[roster]\nlead = cc\nlead = # note\n", "lead"),
            ("workspace", "[workspace]\nmain =\nmain = lead\n", "main"),
            ("workspace", "[workspace]\nmain = lead\nmain =\n", "main"),
        ];
        for (section, text, key) in cases {
            let f = NamedTemp::new("dupclaim", text);
            assert_eq!(
                read_identity(Some(f.path()), None).unwrap_err(),
                ConfigError::DuplicateKey {
                    file: f.path().to_owned(),
                    section: section.to_owned(),
                    key: key.to_owned(),
                    line: 3
                },
                "{text:?}"
            );
        }
        // A comment is not a claim — control.
        let f = NamedTemp::new("dupcomment", "[roster]\n# lead = old\nlead = cc\n");
        assert!(read_identity(Some(f.path()), None).is_ok());
    }

    #[test]
    fn a_prefix_only_profile_is_refused_at_plan_level() {
        // Colead round-2 IMPORTANT-2, at the LaunchPlan level: no Seat with an
        // empty agent_bin ever exists.
        for cmd in ["env", "env -i", "env -u FOO", "env A=1"] {
            let (_f, cfg) = v2(&format!(
                "[profiles]\np = \"{cmd}\"\n[roster]\nlead = p\n[workspace]\nmain = lead\n"
            ));
            assert_eq!(
                launch_plan(&cfg, None, None).unwrap_err(),
                [Violation::CommandRefused {
                    name: "lead".to_owned(),
                    profile: "p".to_owned(),
                    why: crate::launch_cmd::Refusal::NoCommand
                }],
                "{cmd:?}"
            );
        }
    }

    #[test]
    fn a_seat_carries_the_byte_exact_launch_spans_from_one_parse() {
        // Colead IMPORTANT-1: the validated assign/argv spans are transported
        // on the Seat, not reparsed downstream.
        let (_f, cfg) = v2(
            "[profiles]\nmic = \"A=1  B=2 env -u C claude --model\tfable\"\n\
             [roster]\nlead = mic\n[workspace]\nmain = lead\n",
        );
        let plan = launch_plan(&cfg, None, None).expect("launchable");
        let seat = &plan.seats[0];
        assert_eq!(seat.assign_span, "A=1  B=2");
        assert_eq!(seat.argv_span, "env -u C claude --model\tfable");
        assert_eq!(seat.binary, "claude", "env prefix peeled by the parse");
        assert_eq!(seat.tool, crate::tool::ToolKind::Claude);
    }

    #[test]
    fn identity_plan_honours_a_use_override_and_no_workers() {
        let (_f, cfg) = v2(V2);
        let plan = launch_plan(&cfg, Some("colead"), None).unwrap_err();
        assert_eq!(
            plan,
            [Violation::NameTwice {
                seat: "workspace.workers".to_owned(),
                name: "colead".to_owned()
            }],
            "`use colead` with colead still a worker: one name, two seats"
        );
        assert_eq!(
            launch_plan(&cfg, Some("cl:lead"), None).unwrap_err()[0],
            Violation::BadName {
                seat: "use".to_owned(),
                name: "cl:lead".to_owned()
            },
            "the v1 spelling is refused with the grammar, not misread"
        );
        let (_f, cfg) =
            v2("[profiles]\ncc = \"claude\"\n[roster]\nsolo = cc\n[workspace]\nmain = solo\n");
        let plan = launch_plan(&cfg, None, None).expect("a workspace with no workers launches");
        assert_eq!(plan.seats.len(), 1);
        let (_f, cfg) = v2(
            "[profiles]\ncc = \"claude\"\n[roster]\nsolo = cc\n[workspace]\nmain = solo\nworkers = \"\"\n",
        );
        assert_eq!(
            launch_plan(&cfg, None, None)
                .expect("an explicitly empty workers list")
                .seats
                .len(),
            1
        );
        // Whitespace around names is not part of them.
        let (_f, cfg) = v2(
            "[profiles]\ncc = \"claude\"\n[roster]\na = cc\nb = cc\n[workspace]\nmain = a\nworkers =  b , \n",
        );
        assert_eq!(
            launch_plan(&cfg, None, None).unwrap_err(),
            [Violation::EmptyWorker { position: 2 }],
            "a trailing comma is an empty seat, not silence"
        );
    }
}
