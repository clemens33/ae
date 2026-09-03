//! `ae doctor` — the environment report, plus the two internal entries beside
//! it: `_check-deps` (the launch prelude's hard-dependency gate) and
//! `_shims-render` (the session helper set, republished).
//!
//! # What was ported, and what was dropped
//!
//! The frozen `cmd_doctor` reported on the BASH runtime as much as on ae: a
//! `bash >= 4.0` row for the `declare -f` helper bodies, `flock`/`timeout` rows
//! for the optional coreutils the glue degraded without, and a `userland` row
//! naming which portability shim was active. Under the B move none of those are
//! ae's dependencies any more — a session helper is a one-line `exec` that runs
//! under bash 3.2, locking is the core's own `flock(2)`, and there is no shim
//! layer to name. They are dropped rather than reported as permanently OK.
//!
//! What is kept is what the report is FOR: the two hard dependencies (`tmux`,
//! `git`), whether the config parses and names a startup roster whose profiles
//! resolve to real executables, whether the state root's sessions are coherent,
//! and whether each session's recorded core and glue agree with the binary
//! answering right now. The pending-session-id recovery the frozen refresh ran
//! is dropped outright: the launch captures ids itself.
//!
//! # The rows are facts first, text second
//!
//! [`gather`] performs every read and answers with [`Facts`]; [`report`] is a
//! pure function of that value. A row that would need a second look at the
//! world to render is a row that can disagree with the one above it.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::state::{EXIT_FAILED, EXIT_USAGE};

/// The frozen usage line, quoted verbatim by every refusal that raises it.
pub const USAGE: &str = "Usage: ae doctor [--refresh [all|<session>]] [--bash-major <n>]";

/// The `_shims-render` usage line.
pub const SHIMS_USAGE: &str = "Usage: _shims-render <session-dir>";

/// The `_check-deps` usage line.
pub const CHECK_DEPS_USAGE: &str = "Usage: _check-deps [--bash-major <n>]";

/// The frozen `check_deps` refusal for a missing tmux.
pub const NO_TMUX: &str =
    "Error: tmux not found in PATH. Install tmux or run 'ae doctor' for details.";

/// How a row reads, and what it costs the exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Nothing to do.
    Ok,
    /// Worth knowing; the exit code is unaffected.
    Warn,
    /// A broken installation. One of these makes `ae doctor` exit 1.
    Fail,
}

impl Level {
    /// The word the row prints.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }
}

/// One report row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// How it reads.
    pub level: Level,
    /// The subject — the frozen second column.
    pub label: String,
    /// The detail — the frozen free-text remainder, always last.
    pub detail: String,
}

/// The whole report: the rows, in the order they were found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    rows: Vec<Row>,
}

impl Report {
    /// Append one row.
    pub fn push(&mut self, level: Level, label: &str, detail: &str) {
        self.rows.push(Row {
            level,
            label: label.to_owned(),
            detail: detail.to_owned(),
        });
    }

    /// Every row, in order.
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// How many rows are `FAIL`.
    #[must_use]
    pub fn failures(&self) -> usize {
        self.count(Level::Fail)
    }

    /// How many rows are `WARN`.
    #[must_use]
    pub fn warnings(&self) -> usize {
        self.count(Level::Warn)
    }

    fn count(&self, level: Level) -> usize {
        self.rows.iter().filter(|row| row.level == level).count()
    }

    /// The document — the frozen `ae doctor` header, the rows in the frozen
    /// `%-5s %-14s %s` columns, and the summary line.
    #[must_use]
    pub fn render(&self) -> String {
        use std::fmt::Write as _;
        let mut text = String::from("ae doctor\n\n");
        for row in &self.rows {
            text.push_str(&render_row(row));
        }
        text.push('\n');
        let _ = writeln!(
            text,
            "Summary: {} failure(s), {} warning(s)",
            self.failures(),
            self.warnings()
        );
        text
    }

    /// `0` with no failure, `1` with any — the frozen
    /// `((DOCTOR_FAILURES == 0))`.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        if self.failures() == 0 { 0 } else { EXIT_FAILED }
    }
}

/// One row in the frozen columns. The trailing space of an empty detail is
/// kept: the frozen `printf` pads the label whatever follows it.
fn render_row(row: &Row) -> String {
    format!(
        "{:<5} {:<14} {}\n",
        row.level.as_str(),
        row.label,
        row.detail
    )
}

/// What one session's record says about the binaries it was built against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFacts {
    /// The session name — the state directory's leaf.
    pub name: String,
    /// Whether it is live on its OWN recorded server.
    pub live: bool,
    /// `ae_core` — the pinned core path, empty when unset.
    pub core_bin: String,
    /// Whether that path is an executable file right now.
    pub core_usable: bool,
    /// `ae_core_version` — the version the pinned core reported, empty when
    /// unset.
    pub core_version: String,
    /// `ae_version` — the glue version that built the session, empty when
    /// unset.
    pub glue_version: String,
}

/// One `[profiles]` entry, as the report needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileFacts {
    /// The profile key.
    pub profile: String,
    /// The launch command as configured — kept so an unextractable one can be
    /// quoted back.
    pub command: String,
    /// The executable word, or `None` when the command carries none.
    pub executable: Option<String>,
    /// Whether that word resolves on `PATH`.
    pub resolves: bool,
}

/// Everything the report is a function of, read exactly once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// The version of the binary answering.
    pub version: String,
    /// The binary answering — the core the glue resolved and exec'd, so this
    /// is the resolved core path the operator wants to see.
    pub core: Option<PathBuf>,
    /// The major version of the bash the glue runs under, when the glue said
    /// (`--bash-major <n>`); the core cannot see it for itself.
    pub bash_major: Option<u32>,
    /// `tmux`, resolved on `PATH`.
    pub tmux: Option<PathBuf>,
    /// `git`, resolved on `PATH`.
    pub git: Option<PathBuf>,
    /// The global config file this invocation reads.
    pub config: PathBuf,
    /// Why the config could not be read as identity v2, when it could not.
    pub config_error: Option<String>,
    /// The project override, when one was named.
    pub local_config: Option<PathBuf>,
    /// `[workspace] main`.
    pub main: Option<String>,
    /// `[workspace] workers`.
    pub workers: Option<String>,
    /// The profile inventory, sorted by key.
    pub profiles: Vec<ProfileFacts>,
    /// `<AE_HOME>/sessions`.
    pub sessions_dir: PathBuf,
    /// `<AE_HOME>/worktrees`.
    pub worktrees_dir: PathBuf,
    /// Every durable session, in name order.
    pub sessions: Vec<SessionFacts>,
}

/// The report for `facts` — pure, so every row is testable without a machine.
#[must_use]
pub fn report(facts: &Facts) -> Report {
    let mut out = Report::default();
    install_rows(facts, &mut out);
    session_rows(facts, &mut out);
    out
}

/// The rows about the INSTALL: the binary, its dependencies, its config.
fn install_rows(facts: &Facts, out: &mut Report) {
    out.push(Level::Ok, "ae", &format!("version {}", facts.version));
    match &facts.core {
        Some(path) => out.push(Level::Ok, "core", &path.display().to_string()),
        None => out.push(Level::Warn, "core", "this binary cannot name its own path"),
    }
    match facts.bash_major {
        Some(major) if major >= 4 => out.push(Level::Ok, "bash", &format!("bash {major}")),
        Some(major) => out.push(
            Level::Fail,
            "bash",
            &format!("bash {major} (ae needs bash >= 4)"),
        ),
        None => {}
    }

    for (label, found) in [("tmux", &facts.tmux), ("git", &facts.git)] {
        match found {
            Some(path) => out.push(Level::Ok, label, &path.display().to_string()),
            None => out.push(Level::Fail, label, &format!("{label} not found in PATH")),
        }
    }

    if let Some(why) = &facts.config_error {
        out.push(Level::Fail, "config", why);
    } else {
        out.push(Level::Ok, "config", &facts.config.display().to_string());
    }
    match &facts.local_config {
        Some(path) => out.push(Level::Ok, "local-config", &path.display().to_string()),
        None => out.push(
            Level::Warn,
            "local-config",
            "no project override (.ae/config) was named",
        ),
    }

    match &facts.main {
        Some(main) => out.push(Level::Ok, "workspace.main", main),
        None => out.push(Level::Fail, "workspace.main", "not set in config"),
    }
    match &facts.workers {
        Some(workers) => out.push(Level::Ok, "workspace.workers", workers),
        None => out.push(
            Level::Warn,
            "workspace.workers",
            "no startup workers configured",
        ),
    }

    if facts.profiles.is_empty() {
        out.push(Level::Fail, "profiles", "no [profiles] entries found");
    } else {
        for profile in &facts.profiles {
            let label = format!("agent:{}", profile.profile);
            match (&profile.executable, profile.resolves) {
                (None, _) => out.push(
                    Level::Fail,
                    &label,
                    &format!("could not determine executable from '{}'", profile.command),
                ),
                (Some(exec), true) => out.push(Level::Ok, &label, exec),
                (Some(exec), false) => {
                    out.push(Level::Fail, &label, &format!("command '{exec}' not found"));
                }
            }
        }
    }
}

/// The rows about the STATE ROOT: where it is, what is in it, and whether each
/// session's recorded binaries agree with the one answering.
fn session_rows(facts: &Facts, out: &mut Report) {
    out.push(
        Level::Ok,
        "sessions",
        &facts.sessions_dir.display().to_string(),
    );
    out.push(
        Level::Ok,
        "worktrees",
        &facts.worktrees_dir.display().to_string(),
    );

    // Orphans: state on disk with no running session. They accumulate when a
    // wind-down declares done but never runs `ae end`, and every
    // running-scoped sensor is structurally blind to them. WARN, not FAIL: a
    // deliberately stopped session is legitimately here too, and doctor cannot
    // read intent.
    let orphans: Vec<&str> = facts
        .sessions
        .iter()
        .filter(|session| !session.live)
        .map(|session| session.name.as_str())
        .collect();
    if orphans.is_empty() {
        out.push(Level::Ok, "orphans", "no stopped or orphaned session dirs");
    } else {
        out.push(
            Level::Warn,
            "orphans",
            &format!(
                "{} session dir(s)/worktree(s) with no running session: {}",
                orphans.len(),
                orphans.join(", ")
            ),
        );
        out.push(
            Level::Warn,
            "orphans-hint",
            "resume with 'ae <name>' or finish teardown with 'ae end <name>'",
        );
    }

    // The core is REQUIRED, so a session with no usable pin refuses every
    // core-owned command — `end` included, which leaves it unendable. WARN
    // rather than FAIL: `doctor --refresh` repairs it and the rest of the
    // install is fine.
    let unbound: Vec<&str> = facts
        .sessions
        .iter()
        .filter(|session| {
            session.core_bin.is_empty() || session.core_version.is_empty() || !session.core_usable
        })
        .map(|session| session.name.as_str())
        .collect();
    if !unbound.is_empty() {
        out.push(
            Level::Warn,
            "core-pin",
            &format!(
                "session {} has no core bound; end/archive will refuse",
                unbound.join(", ")
            ),
        );
        out.push(
            Level::Warn,
            "core-pin-hint",
            "repair with 'ae doctor --refresh'",
        );
    }

    // The pin is a PAIR, and a helper that finds a core whose version is not
    // the pinned one refuses it. A session pinned to a different version than
    // the binary answering here is therefore reported by name — this is the
    // "core and glue agree" check, read off the only place either version is
    // recorded.
    let drifted: Vec<String> = facts
        .sessions
        .iter()
        .filter(|session| {
            let core = !session.core_version.is_empty() && session.core_version != facts.version;
            let glue = !session.glue_version.is_empty() && session.glue_version != facts.version;
            core || glue
        })
        .map(|session| {
            format!(
                "{} (core {}, glue {})",
                session.name,
                blank(&session.core_version),
                blank(&session.glue_version)
            )
        })
        .collect();
    if !drifted.is_empty() {
        out.push(
            Level::Warn,
            "core-version",
            &format!(
                "this core is {}; pinned elsewhere: {}",
                facts.version,
                drifted.join(", ")
            ),
        );
        out.push(
            Level::Warn,
            "core-version-hint",
            "repair with 'ae doctor --refresh'",
        );
    }
}

/// An empty recorded version reads as `-`, never as a blank column.
fn blank(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

/// The executable word of a launch command — the frozen `doctor_extract_exec`,
/// which skips a leading `env` and any `VAR=val` prefix.
///
/// Delegates to [`crate::launch_cmd::split_binary`] so the report and the
/// launch cannot disagree about which word is the binary.
#[must_use]
pub fn executable_of(command: &str) -> Option<String> {
    crate::launch_cmd::split_binary(command).map(|split| split.binary)
}

/// Resolve `program` the way `command -v` does: an absolute or relative name
/// with a `/` is taken as given, anything else is looked up along `PATH`.
///
/// # The door
///
/// This is the one place this module reads the world outside ae's own state,
/// and it reads exactly two things: the `PATH` variable, and whether each
/// candidate is an executable file. Registered in the phase-3 door inventory.
#[must_use]
pub fn resolve_on_path(program: &str) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    if program.contains('/') {
        let path = PathBuf::from(program);
        return is_executable_file(&path).then_some(path);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: `command -v` resolves a program along PATH, and doctor's whole job is saying whether the hard dependencies are there — see clippy.toml"
    )]
    let raw = std::env::var_os("PATH");
    let raw = raw?;
    std::env::split_paths(&raw)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable_file(candidate))
}

/// Whether `path` is a file anyone may execute.
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the executable-bit half of `command -v` — see clippy.toml"
    )]
    let probe = std::fs::metadata(path);
    probe.is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// Read every fact the report needs.
///
/// `global` and `local` are the config files the CALLER selected — the glue
/// hands them in exactly as it does for a launch, because the core does not
/// read the environment to find them.
#[must_use]
pub fn gather(
    root: &Path,
    global: Option<&Path>,
    local: Option<&Path>,
    bash_major: Option<u32>,
) -> Facts {
    let roots = crate::inventory::Roots::under(root);
    let config = global.map_or_else(|| root.join("config"), Path::to_path_buf);
    let read = crate::config::read_identity(Some(&config), local);
    let (config_error, identity) = match read {
        Ok(identity) => (None, identity),
        Err(why) => (
            Some(why.to_string()),
            crate::config::IdentityConfig::default(),
        ),
    };
    let mut profiles: Vec<ProfileFacts> = identity
        .profiles
        .iter()
        .map(|(profile, command)| {
            let executable = executable_of(command);
            let resolves = executable
                .as_deref()
                .is_some_and(|exec| resolve_on_path(exec).is_some());
            ProfileFacts {
                profile: profile.clone(),
                command: command.clone(),
                executable,
                resolves,
            }
        })
        .collect();
    profiles.sort_by(|left, right| left.profile.cmp(&right.profile));

    Facts {
        version: crate::VERSION.to_owned(),
        core: std::env::current_exe().ok(),
        bash_major,
        tmux: resolve_on_path("tmux"),
        git: resolve_on_path("git"),
        config,
        config_error,
        local_config: local.map(Path::to_path_buf),
        main: identity.main.filter(|value| !value.is_empty()),
        workers: identity.workers.filter(|value| !value.is_empty()),
        profiles,
        sessions_dir: roots.sessions().to_owned(),
        worktrees_dir: roots.worktrees().to_owned(),
        sessions: session_facts(root),
    }
}

/// Every durable session's record facts, in name order.
fn session_facts(root: &Path) -> Vec<SessionFacts> {
    let mut out: Vec<SessionFacts> = discovered(root)
        .into_iter()
        .map(|(name, dir)| {
            let bytes = crate::meta::read_bytes(&dir).unwrap_or_default();
            let core_bin = crate::lifecycle::meta_value(&bytes, "ae_core");
            // Liveness is asked of the session's OWN recorded server. Asking the
            // ambient one false-orphans every session created on a named server.
            let server = match crate::lifecycle::server_of(&bytes) {
                crate::meta::ServerSelector::Positive(selector) => {
                    crate::inventory::ServerId::Selected(selector)
                }
                _ => crate::inventory::ServerId::Ambient,
            };
            SessionFacts {
                live: crate::transport::session_exists(&server, &name),
                core_usable: !core_bin.is_empty() && is_executable_file(Path::new(&core_bin)),
                core_version: crate::lifecycle::meta_value(&bytes, "ae_core_version"),
                glue_version: crate::lifecycle::meta_value(&bytes, "ae_version"),
                core_bin,
                name,
            }
        })
        .collect();
    out.sort_by(|left, right| left.name.cmp(&right.name));
    out
}

/// Every durable session directory the state root holds, both SC-400d layouts,
/// first record per name.
fn discovered(root: &Path) -> Vec<(String, PathBuf)> {
    let scan = crate::inventory::durable_records(&crate::inventory::Roots::under(root));
    let mut seen: Vec<(String, PathBuf)> = Vec::new();
    for record in scan.records {
        if !seen.iter().any(|(name, _)| *name == record.name) {
            seen.push((record.name, record.path));
        }
    }
    seen
}

/// What `doctor`'s argv asked for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Args {
    refresh: Option<String>,
    global: Option<PathBuf>,
    local: Option<PathBuf>,
    bash_major: Option<u32>,
}

/// Read `doctor`'s flags. The offending word on refusal.
fn parse(tail: &[String]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut rest = tail;
    while let [word, after @ ..] = rest {
        match word.as_str() {
            "--refresh" => {
                // The operand is OPTIONAL and defaults to `all`, so a following
                // flag is not swallowed as a session name.
                match after {
                    [target, tail @ ..] if !target.starts_with('-') => {
                        args.refresh = Some(target.clone());
                        rest = tail;
                    }
                    _ => {
                        args.refresh = Some("all".to_owned());
                        rest = after;
                    }
                }
            }
            "--bash-major" => match after {
                [value, tail @ ..] => {
                    args.bash_major = Some(value.parse().map_err(|_| word.clone())?);
                    rest = tail;
                }
                [] => return Err(word.clone()),
            },
            "--global" | "--local" => match after {
                [value, tail @ ..] => {
                    if word == "--global" {
                        args.global = Some(PathBuf::from(value));
                    } else {
                        args.local = Some(PathBuf::from(value));
                    }
                    rest = tail;
                }
                [] => return Err(word.clone()),
            },
            other => return Err(other.to_owned()),
        }
    }
    Ok(args)
}

/// `doctor [--refresh [all|<session>]] [--global <f>] [--local <f>]` — the
/// whole report, and the refresh when it was asked for.
///
/// # Errors
///
/// Propagates a write failure on the caller's streams.
pub fn run(
    root: &Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let Ok(args) = parse(tail) else {
        writeln!(err, "{USAGE}")?;
        return Ok(EXIT_USAGE);
    };
    // The two roots are CREATED, exactly as the frozen `mkdir -p` did: doctor
    // is where a fresh install acquires them.
    let roots = crate::inventory::Roots::under(root);
    let _ = std::fs::create_dir_all(roots.sessions());
    let _ = std::fs::create_dir_all(roots.worktrees());

    let facts = gather(
        root,
        args.global.as_deref(),
        args.local.as_deref(),
        args.bash_major,
    );
    let mut document = report(&facts);
    if let Some(target) = &args.refresh {
        refresh(root, target, args.global.as_deref(), &mut document);
    }
    write!(out, "{}", document.render())?;
    Ok(document.exit_code())
}

/// Republish one session's assets, or every session's.
fn refresh(root: &Path, target: &str, global: Option<&Path>, document: &mut Report) {
    let core = match std::env::current_exe() {
        Ok(core) => core,
        Err(why) => {
            document.push(
                Level::Fail,
                "refresh",
                &format!("the core could not name its own binary ({why})"),
            );
            return;
        }
    };
    let mut found = discovered(root);
    found.sort_by(|left, right| left.0.cmp(&right.0));
    if target == "all" {
        if found.is_empty() {
            document.push(
                Level::Warn,
                "refresh",
                &format!(
                    "no existing sessions found in {}",
                    crate::inventory::Roots::under(root).sessions().display()
                ),
            );
            return;
        }
        for (name, dir) in found {
            refresh_one(&name, &dir, &core, global, document);
        }
        return;
    }
    match found.into_iter().find(|(name, _)| name == target) {
        Some((name, dir)) => refresh_one(&name, &dir, &core, global, document),
        None => document.push(
            Level::Fail,
            &format!("refresh:{target}"),
            &format!("session '{target}' not found"),
        ),
    }
}

/// One session: rebind the core pin, republish the helper shims, re-render the
/// workspace manifest.
fn refresh_one(name: &str, dir: &Path, core: &Path, global: Option<&Path>, document: &mut Report) {
    let label = format!("refresh:{name}");
    let bytes = match crate::meta::read_bytes(dir) {
        Ok(bytes) => bytes,
        Err(why) => {
            document.push(
                Level::Fail,
                &label,
                &format!("could not read the session meta ({why})"),
            );
            return;
        }
    };
    let value = |key: &str| crate::lifecycle::meta_value(&bytes, key);
    let session = {
        let recorded = value("session");
        if recorded.is_empty() {
            name.to_owned()
        } else {
            recorded
        }
    };

    // The pin is rebound to the binary DOING the refresh. The frozen
    // `_ae_core_bind` re-evaluated the operator's input; the core answering here
    // IS a usable core, which is the only thing the pin has to establish, and
    // pinning what actually ran cannot pin something unusable.
    for (key, new) in [
        ("ae_core", core.display().to_string()),
        ("ae_core_version", crate::VERSION.to_owned()),
        ("ae_version", crate::VERSION.to_owned()),
    ] {
        if let Err(why) = crate::meta::rewrite(dir, key, Some(&new)) {
            document.push(
                Level::Fail,
                &label,
                &format!("could not rebind {key} ({})", why.cause()),
            );
            return;
        }
    }

    if let Err(why) = crate::session_launch::assets::write_helpers(dir, core) {
        document.push(Level::Fail, &label, &why);
        return;
    }

    // The manifest reads the session's OWN recorded config, layered under the
    // origin's project override — the frozen `sync_existing_session_assets`
    // hydration, which is why a refresh renders the same document a launch did.
    let origin = or_dot(value("origin"));
    let work_dir = or_dot(value("work_dir"));
    let mut config_files: Vec<PathBuf> = Vec::new();
    let recorded = value("config");
    if !recorded.is_empty() {
        config_files.push(PathBuf::from(recorded));
    } else if let Some(global) = global {
        config_files.push(global.to_path_buf());
    }
    config_files.push(Path::new(&origin).join(".ae").join("config"));
    let mode = value("mode");
    let mode = if mode.is_empty() {
        "local".to_owned()
    } else {
        mode
    };
    let main_pane = value("main_pane");
    let main_pane = if main_pane.is_empty() {
        "%0".to_owned()
    } else {
        main_pane
    };
    let manifest = crate::render::manifest_document(
        dir,
        &session,
        &work_dir,
        &origin,
        &mode,
        &main_pane,
        &config_files,
    );
    if let Err(why) =
        crate::session_launch::assets::publish_document(&dir.join("workspace.md"), &manifest)
    {
        document.push(
            Level::Fail,
            &label,
            &format!("could not write the workspace manifest ({why})"),
        );
        return;
    }
    document.push(Level::Ok, &label, "refreshed session helpers and workspace");
}

/// A missing path fact renders as `.`, the frozen default.
fn or_dot(value: String) -> String {
    if value.is_empty() {
        ".".to_owned()
    } else {
        value
    }
}

/// `_check-deps [--bash-major <n>]` — the launch prelude's gate.
///
/// tmux is ae's one PATH dependency at launch, and its absence is fatal before
/// any side effect. The bash major version is passed in rather than measured:
/// the glue knows its own `BASH_VERSINFO[0]` for free, and the core would have
/// to spawn a shell to learn it. With no `--bash-major` the check is simply not
/// made, because a version nobody supplied is not a version to refuse on.
///
/// # Errors
///
/// Propagates a write failure on the caller's streams.
pub fn check_deps(tail: &[String], err: &mut impl Write) -> crate::Result<u8> {
    let mut bash_major: Option<u32> = None;
    let mut rest: &[String] = tail;
    while let [word, after @ ..] = rest {
        let ("--bash-major", [value, next @ ..]) = (word.as_str(), after) else {
            writeln!(err, "{CHECK_DEPS_USAGE}")?;
            return Ok(EXIT_USAGE);
        };
        let Ok(parsed) = value.parse::<u32>() else {
            writeln!(err, "{CHECK_DEPS_USAGE}")?;
            return Ok(EXIT_USAGE);
        };
        bash_major = Some(parsed);
        rest = next;
    }
    if let Some(major) = bash_major
        && major < 4
    {
        writeln!(
            err,
            "Error: ae requires bash >= 4.0 (found {major}).\nmacOS: brew install bash"
        )?;
        return Ok(EXIT_FAILED);
    }
    if resolve_on_path("tmux").is_none() {
        writeln!(err, "{NO_TMUX}")?;
        return Ok(EXIT_FAILED);
    }
    Ok(0)
}

/// `_shims-render <session-dir>` — republish one session's helper set, bound to
/// the binary answering.
///
/// The whole helper set, not a diff: a shim is a one-line `exec`, so rewriting
/// all of them costs nothing and a partial set is a session with a helper
/// missing.
///
/// # Errors
///
/// Propagates a write failure on the caller's streams.
pub fn shims_render(dir: &Path, tail: &[String], err: &mut impl Write) -> crate::Result<u8> {
    if let [extra, ..] = tail {
        writeln!(err, "ae: _shims-render: unexpected argument: {extra}")?;
        return Ok(EXIT_USAGE);
    }
    if !crate::lifecycle::dir_exists(dir) {
        writeln!(
            err,
            "ae: _shims-render: no session directory at {}",
            dir.display()
        )?;
        return Ok(EXIT_FAILED);
    }
    let core = match std::env::current_exe() {
        Ok(core) => core,
        Err(why) => {
            writeln!(err, "ae: the core could not name its own binary ({why}).")?;
            return Ok(EXIT_FAILED);
        }
    };
    match crate::session_launch::assets::write_helpers(dir, &core) {
        Ok(()) => Ok(0),
        Err(why) => {
            writeln!(err, "ae: {why}")?;
            Ok(EXIT_FAILED)
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests read back what the doors wrote; the boundary is on product code — see clippy.toml"
)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            version: "2026.9.1".to_owned(),
            core: Some(PathBuf::from("/opt/ae/versions/1/ae-core")),
            bash_major: None,
            tmux: Some(PathBuf::from("/usr/bin/tmux")),
            git: Some(PathBuf::from("/usr/bin/git")),
            config: PathBuf::from("/home/me/.ae/config"),
            config_error: None,
            local_config: None,
            main: Some("lead".to_owned()),
            workers: Some("colead".to_owned()),
            profiles: vec![ProfileFacts {
                profile: "cl".to_owned(),
                command: "claude --dangerously-skip-permissions".to_owned(),
                executable: Some("claude".to_owned()),
                resolves: true,
            }],
            sessions_dir: PathBuf::from("/home/me/.ae/sessions"),
            worktrees_dir: PathBuf::from("/home/me/.ae/worktrees"),
            sessions: Vec::new(),
        }
    }

    #[test]
    fn the_columns_are_the_frozen_widths() {
        let row = Row {
            level: Level::Warn,
            label: "orphans".to_owned(),
            detail: "one".to_owned(),
        };
        assert_eq!(render_row(&row), "WARN  orphans        one\n");
    }

    #[test]
    fn a_clean_environment_reports_no_failure_and_exits_zero() {
        let document = report(&facts());
        assert_eq!(document.failures(), 0);
        assert_eq!(document.exit_code(), 0);
        let text = document.render();
        assert!(text.starts_with("ae doctor\n\n"), "{text}");
        assert!(
            text.contains("OK    ae             version 2026.9.1\n"),
            "{text}"
        );
        assert!(
            text.ends_with("Summary: 0 failure(s), 1 warning(s)\n"),
            "{text}"
        );
    }

    #[test]
    fn a_missing_hard_dependency_fails_the_report() {
        let mut input = facts();
        input.tmux = None;
        let document = report(&input);
        assert_eq!(document.exit_code(), EXIT_FAILED);
        assert!(
            document
                .render()
                .contains("FAIL  tmux           tmux not found in PATH\n"),
            "{}",
            document.render()
        );
    }

    #[test]
    fn a_config_that_does_not_parse_is_one_failure_and_names_itself() {
        let mut input = facts();
        input.config_error = Some("Error: /c:3: invalid agent name".to_owned());
        let document = report(&input);
        assert!(
            document
                .rows()
                .iter()
                .any(|row| row.label == "config" && row.level == Level::Fail),
            "{:?}",
            document.rows()
        );
    }

    #[test]
    fn an_unresolvable_profile_names_the_command_it_could_not_find() {
        let mut input = facts();
        input.profiles[0].resolves = false;
        let document = report(&input);
        assert!(
            document.render().contains("command 'claude' not found"),
            "{}",
            document.render()
        );
    }

    #[test]
    fn a_command_with_no_executable_word_is_quoted_back() {
        let mut input = facts();
        input.profiles[0].executable = None;
        input.profiles[0].command = "FOO=bar".to_owned();
        assert!(
            report(&input)
                .render()
                .contains("could not determine executable from 'FOO=bar'"),
            "{}",
            report(&input).render()
        );
    }

    #[test]
    fn a_stopped_session_dir_is_an_orphan_warning_with_a_hint() {
        let mut input = facts();
        input.sessions.push(SessionFacts {
            name: "left".to_owned(),
            live: false,
            core_bin: "/c".to_owned(),
            core_usable: true,
            core_version: "2026.9.1".to_owned(),
            glue_version: "2026.9.1".to_owned(),
        });
        let document = report(&input);
        assert_eq!(document.failures(), 0);
        let text = document.render();
        assert!(
            text.contains("1 session dir(s)/worktree(s) with no running session: left"),
            "{text}"
        );
        assert!(text.contains("orphans-hint"), "{text}");
    }

    #[test]
    fn a_session_with_no_usable_core_is_named_with_the_repair() {
        let mut input = facts();
        input.sessions.push(SessionFacts {
            name: "unbound".to_owned(),
            live: true,
            core_bin: String::new(),
            core_usable: false,
            core_version: String::new(),
            glue_version: "2026.9.1".to_owned(),
        });
        let text = report(&input).render();
        assert!(text.contains("session unbound has no core bound"), "{text}");
        assert!(text.contains("repair with 'ae doctor --refresh'"), "{text}");
    }

    #[test]
    fn a_version_disagreement_between_this_core_and_a_session_pin_is_a_warning() {
        let mut input = facts();
        input.sessions.push(SessionFacts {
            name: "old".to_owned(),
            live: true,
            core_bin: "/c".to_owned(),
            core_usable: true,
            core_version: "2026.8.4".to_owned(),
            glue_version: "2026.8.4".to_owned(),
        });
        let text = report(&input).render();
        assert!(
            text.contains(
                "this core is 2026.9.1; pinned elsewhere: old (core 2026.8.4, glue 2026.8.4)"
            ),
            "{text}"
        );
        assert_eq!(report(&input).failures(), 0, "drift is never a failure");
    }

    #[test]
    fn the_executable_word_skips_an_env_prefix() {
        assert_eq!(
            executable_of("env OPENCODE_CONFIG=/x opencode"),
            Some("opencode".to_owned())
        );
        assert_eq!(executable_of("claude -p"), Some("claude".to_owned()));
        assert_eq!(executable_of("FOO=bar"), None);
    }

    #[test]
    fn refresh_takes_an_optional_target_and_never_eats_a_flag() {
        let words = |list: &[&str]| list.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
        assert_eq!(parse(&words(&[])).unwrap().refresh, None);
        assert_eq!(
            parse(&words(&["--refresh"])).unwrap().refresh,
            Some("all".to_owned())
        );
        assert_eq!(
            parse(&words(&["--refresh", "aedev"])).unwrap().refresh,
            Some("aedev".to_owned())
        );
        let parsed = parse(&words(&["--refresh", "--global", "/g"])).unwrap();
        assert_eq!(parsed.refresh, Some("all".to_owned()));
        assert_eq!(parsed.global, Some(PathBuf::from("/g")));
        assert_eq!(parse(&words(&["--wat"])), Err("--wat".to_owned()));
        assert_eq!(parse(&words(&["--global"])), Err("--global".to_owned()));
    }

    #[test]
    fn an_absolute_program_is_taken_as_given_and_never_looked_up() {
        assert_eq!(resolve_on_path("/definitely/not/here"), None);
        assert_eq!(resolve_on_path(""), None);
    }

    #[test]
    fn check_deps_refuses_a_bash_older_than_four() {
        let mut err = Vec::new();
        let code = check_deps(&["--bash-major".to_owned(), "3".to_owned()], &mut err).unwrap();
        assert_eq!(code, EXIT_FAILED);
        assert!(
            String::from_utf8_lossy(&err).contains("bash >= 4.0"),
            "{}",
            String::from_utf8_lossy(&err)
        );
    }

    #[test]
    fn check_deps_refuses_an_unparseable_version_as_usage() {
        let mut err = Vec::new();
        let code = check_deps(&["--bash-major".to_owned(), "four".to_owned()], &mut err).unwrap();
        assert_eq!(code, EXIT_USAGE);
    }
}
