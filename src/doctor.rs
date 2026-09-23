//! `ae doctor` — the environment report, plus the two internal entries beside
//! it: `_check-deps` (the launch prelude's hard-dependency gate) and
//! `_shims-render` (the session helper set, republished).

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::state::{EXIT_FAILED, EXIT_USAGE};

/// The usage line, quoted verbatim by every refusal that raises it.
pub const USAGE: &str = "Usage: ae doctor [--refresh [all|<session>]]";

/// The `_shims-render` usage line.
pub const SHIMS_USAGE: &str = "Usage: _shims-render <session-dir>";

/// The `_check-deps` usage line.
pub const CHECK_DEPS_USAGE: &str = "Usage: _check-deps";

/// The `_check-deps` refusal for a missing tmux.
pub const NO_TMUX: &str =
    "Error: tmux not found in PATH. Install tmux or run 'ae doctor' for details.";

/// How a row reads, and what it costs the exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Nothing to do.
    Ok,
    /// Worth knowing; the exit code is unaffected.
    Warn,
    /// A broken installation.
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
    /// The subject — the second column.
    pub label: String,
    /// The detail — the free-text remainder, always last.
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

    /// The document — the `ae doctor` header, the rows in their
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

    /// `0` with no failure, `1` with any.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        if self.failures() == 0 { 0 } else { EXIT_FAILED }
    }
}

/// One row in those columns.
fn render_row(row: &Row) -> String {
    format!(
        "{:<5} {:<14} {}\n",
        row.level.as_str(),
        row.label,
        row.detail
    )
}

/// One ae-owned server's input-map verdict, as `ae doctor` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingsFacts {
    /// How the server is named back (`-L ae`, `-S …`).
    pub server: String,
    /// The tmux that answered there.
    pub version: String,
    /// The verdict.
    pub status: BindingsStatus,
}

/// What one server's key tables said about ae's input map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingsStatus {
    /// Every expected key is bound to this ae, and no stale one lingers.
    Intact,
    /// One problem per broken key — `missing <table> <key>`, `<table> <key>
    /// is bound to a foreign command`, `<table> <key> is bound by another ae:
    /// <word>`, `stale <table> <key> is still bound`.
    Broken(Vec<String>),
    /// The map could not be judged — below the floor, or `list-keys` refused.
    Unreadable {
        /// Why, in the row's own words.
        why: String,
    },
}

/// What one session's record says about the binaries it was built against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFacts {
    /// The session name — the state directory's leaf.
    pub name: String,
    /// Whether it is live on its OWN recorded server.
    pub live: bool,
    /// The session's own recorded server — `Ambient` when the record names
    /// none, which the bindings check never reads.
    pub server: crate::inventory::ServerId,
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
    /// When this session last did something only a LIVE session does — the
    /// evidence a reboot refusal rests on. See
    /// [`crate::inventory::last_live`].
    pub last_live: crate::tmux::Evidence,
}

/// One LIVE seat: what its records say, and what its pane was observed to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatFacts {
    /// The session the seat belongs to.
    pub session: String,
    /// `seat.<slot>`.
    pub agent: String,
    /// The pane stamped with the seat's slot, empty when none was found.
    pub pane: String,
    /// `agent_bin.<slot>`, empty when unrecorded.
    pub binary: String,
    /// `profile.<slot>`, empty when unrecorded.
    pub profile: String,
    /// The reading — `Unknown` when the pane could not be found or read.
    pub observed: crate::procs::Observed,
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
    /// Why client expansion failed before an executable could be inspected.
    pub resolution_error: Option<String>,
}

/// Everything the report is a function of, read exactly once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// The version of the binary answering.
    pub version: String,
    /// The binary answering — `current_exe()`, which IS the public `ae`, so this
    /// is the resolved core path the operator wants to see.
    pub core: Option<PathBuf>,
    /// Whether that binary is WRITABLE.
    pub core_writable: Option<bool>,
    /// Whether this binary was PUBLISHED by the installer — the shape that
    /// makes a writable core a deviation rather than the normal state.
    pub core_published: bool,
    /// `tmux`, resolved on `PATH`.
    pub tmux: Option<PathBuf>,
    /// WHICH tmux would run ae's surfaces, and what it answered — the floor
    /// row's whole input.
    pub tmux_floor: crate::tmux_floor::Probe,
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
    /// The raw global `[workspace] fleet_order`, or `""` when the human set none.
    pub fleet_order: String,
    /// Why that line could not be read at all, when it could not — a BROKEN LINE
    /// loses the whole key, and silence reads like choosing no order.
    pub fleet_order_error: Option<String>,
    /// The profile inventory, sorted by key.
    pub profiles: Vec<ProfileFacts>,
    /// When this host booted, epoch seconds — the other half of that evidence.
    pub boot: Option<i64>,
    /// `<AE_HOME>/sessions`.
    pub sessions_dir: PathBuf,
    /// `<AE_HOME>/worktrees`.
    pub worktrees_dir: PathBuf,
    /// Every durable session, in name order.
    pub sessions: Vec<SessionFacts>,
    /// Every seat of a LIVE session, in session then roster order.
    pub seats: Vec<SeatFacts>,
    /// The input-map verdict per ae-owned server that answered — empty when
    /// none did, which reads as one neutral row, never a warning.
    pub bindings: Vec<BindingsFacts>,
}

/// The `workspace.fleet_order` row: the human's chosen strip order, and the ONE
/// place ae says an entry of it went nowhere — the strip stays silent, because a
/// typo there must never cost anyone their status line. Three things earn the
/// warning: an illegal session name, a name repeated after it already placed,
/// and a name matching no durable record, which is the typo the allowlist cannot
/// catch (`aedve` is perfectly legal). A recorded session that is merely STOPPED
/// is never flagged: coming back the same way across restarts is the point.
fn fleet_order_row(facts: &Facts, out: &mut Report) {
    // A BROKEN LINE outranks everything below: the key is gone, and "no chosen
    // order" would describe a choice the human never made.
    if let Some(why) = &facts.fleet_order_error {
        out.push(Level::Warn, "workspace.fleet_order", why);
        return;
    }
    if facts.fleet_order.trim().is_empty() {
        out.push(
            Level::Ok,
            "workspace.fleet_order",
            "no chosen order — the fleet strip is in creation order",
        );
        return;
    }
    let (names, mut ignored) = crate::config::fleet_order_entries(&facts.fleet_order);
    let placed: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|name| {
            let recorded = facts.sessions.iter().any(|session| session.name == *name);
            if !recorded {
                ignored.push((*name).to_owned());
            }
            recorded
        })
        .collect();
    let detail = placed.join(", ");
    if ignored.is_empty() {
        out.push(Level::Ok, "workspace.fleet_order", &detail);
    } else {
        let detail = if detail.is_empty() {
            format!(
                "no entry named a session ae has a record of; ignored: {}",
                ignored.join(", ")
            )
        } else {
            format!("{detail} — ignored: {}", ignored.join(", "))
        };
        out.push(Level::Warn, "workspace.fleet_order", &detail);
    }
}

/// The report for `facts` — pure, so every row is testable without a machine.
#[must_use]
pub fn report(facts: &Facts) -> Report {
    let mut out = Report::default();
    install_rows(facts, &mut out);
    session_rows(facts, &mut out);
    seat_identity_rows(&facts.seats, &mut out);
    bindings_rows(facts, &mut out);
    out
}

/// The `seat-identity` rows: every live seat's RECORDS against what its pane
/// was observed to run.
///
/// DIAGNOSIS ONLY. A seat's identity is its records, which is what keeps a
/// stop from ending a tool nobody proved; doctor never rewrites one. What it
/// owes a human is the disagreement a verb would otherwise refuse without
/// explaining — and the COVERAGE, because a seat it could not read is not a
/// seat it checked.
fn seat_identity_rows(seats: &[SeatFacts], out: &mut Report) {
    use crate::procs::Observed;
    if seats.is_empty() {
        return;
    }
    let (mut matched, mut disagree) = (0_usize, 0_usize);
    let mut uncovered: Vec<String> = Vec::new();
    for seat in seats {
        let name = format!("{}:{}", seat.session, seat.agent);
        let tools = match &seat.observed {
            Observed::Harness(tools) => tools,
            Observed::Shell => {
                uncovered.push(format!("{name} at a shell"));
                continue;
            }
            Observed::Unrecognised(word) => {
                uncovered.push(format!("{name} runs '{word}'"));
                continue;
            }
            Observed::Unknown => {
                uncovered.push(format!("{name} unreadable"));
                continue;
            }
        };
        let own = tools
            .iter()
            .any(|tool| crate::procs::name_matches(&seat.binary, tool.as_str()));
        if own && tools.len() == 1 {
            matched += 1;
            continue;
        }
        disagree += 1;
        let names: Vec<&str> = tools.iter().map(|tool| tool.as_str()).collect();
        out.push(
            Level::Warn,
            "seat-identity",
            &format!(
                "{name} (pane {}) records {} ({}); its pane runs {}",
                seat.pane,
                blank(&seat.binary),
                if seat.profile.is_empty() {
                    "no profile".to_owned()
                } else {
                    format!("profile '{}'", seat.profile)
                },
                names.join(" and ")
            ),
        );
    }
    if disagree > 0 {
        out.push(
            Level::Warn,
            "seat-identity-hint",
            "records are a seat's identity and doctor never rewrites them: reseat and relaunch \
             refuse such a seat — end the other tool in its pane, then reseat to the profile you \
             want",
        );
    }
    let verdict = match (matched + disagree, disagree) {
        (0, _) => String::new(),
        (_, 0) => "; every checked seat runs the tool it records".to_owned(),
        _ => format!("; {matched} run(s) the tool it records, {disagree} disagree"),
    };
    let coverage = if uncovered.is_empty() {
        String::new()
    } else {
        format!("; uncovered: {}", uncovered.join(", "))
    };
    out.push(
        Level::Ok,
        "seat-identity",
        &format!(
            "{} of {} live seat(s) checked{verdict}{coverage}",
            matched + disagree,
            seats.len()
        ),
    );
}

/// The rows about the INSTALL: the binary, its dependencies, its config.
fn install_rows(facts: &Facts, out: &mut Report) {
    out.push(Level::Ok, "ae", &format!("version {}", facts.version));
    match (&facts.core, facts.core_writable.filter(|_| facts.core_published)) {
        (Some(path), Some(true)) => out.push(
            Level::Warn,
            "core",
            &format!(
                "{} is writable — the installer publishes it read-only (0555), and every session helper is a symlink to it, so one stray redirection truncates the binary every session on this machine is bound to",
                path.display()
            ),
        ),
        (Some(path), _) => out.push(Level::Ok, "core", &path.display().to_string()),
        (None, _) => out.push(Level::Warn, "core", "this binary cannot name its own path"),
    }

    for (label, found) in [("tmux", &facts.tmux), ("git", &facts.git)] {
        match found {
            Some(path) => out.push(Level::Ok, label, &path.display().to_string()),
            None => out.push(Level::Fail, label, &format!("{label} not found in PATH")),
        }
    }

    // WARN, never FAIL: an ae below the floor still lists, reports and upgrades
    // itself, and a FAIL here would make `ae doctor` exit non-zero on a machine
    // whose install is perfectly sound.
    let floor = &facts.tmux_floor;
    out.push(
        match floor.verdict() {
            crate::tmux_floor::Verdict::Ok => Level::Ok,
            _ => Level::Warn,
        },
        "tmux-floor",
        &crate::tmux_floor::row_detail(floor),
    );

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

    fleet_order_row(facts, out);

    if facts.profiles.is_empty() {
        out.push(Level::Fail, "profiles", "no [profiles] entries found");
    } else {
        for profile in &facts.profiles {
            let label = format!("agent:{}", profile.profile);
            if let Some(why) = &profile.resolution_error {
                out.push(Level::Fail, &label, why);
                continue;
            }
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

    reboot_rows(facts, out);

    // Orphans: state on disk with no running session.
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
    // core-owned command — `end` included, which leaves it unendable.
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
    // the pinned one refuses it.
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

/// Compare the expected set against `(table, entries)` pairs as `list-keys`
/// printed them — the pure half of the check, so every verdict is testable
/// without a server. One problem string per broken key; empty means intact.
///
/// "ae's" is present-and-names-the-launcher: tmux re-quotes the bound command
/// when it prints it, so byte-equality with the owner's argv would couple this
/// check to tmux's serializer. The launcher is the executable word — the
/// command link for an installed ae, the core path for a checkout.
fn compare_bindings(
    expected: &[crate::session_tmux::ExpectedBinding],
    listed: &[(String, Vec<crate::tmux::KeyBinding>)],
    launcher: &str,
) -> Vec<String> {
    let mut problems = Vec::new();
    for entry in expected {
        let name = format!("{} {}", entry.table, entry.key);
        let command = listed
            .iter()
            .find(|(table, _)| *table == entry.table)
            .and_then(|(_, keys)| keys.iter().find(|listed| listed.key == entry.key))
            .map(|listed| listed.command.as_str());
        match (command, entry.absent) {
            (Some(_), true) => problems.push(format!("stale {name} is still bound")),
            (Some(command), false) if entry.names_launcher && !command.contains(launcher) => {
                match another_ae_word(command) {
                    Some(word) => {
                        problems.push(format!("{name} is bound by another ae: {word}"));
                    }
                    None => problems.push(format!("{name} is bound to a foreign command")),
                }
            }
            // A present launcher-less entry (the 3.4 Down pair) is
            // presence-checked: its command names no launcher by design.
            (Some(_), false) | (None, true) => {}
            (None, false) => problems.push(format!("missing {name}")),
        }
    }
    problems
}

/// The executable word of ANOTHER ae's launcher named in `command`, if any.
///
/// Cheap token scan: a word whose basename is `ae` or `ae-core` beside one of
/// ae's own subcommand words. Requiring both keeps a stray `ae` word — or a
/// foreign command merely mentioning one — from reading as another ae.
fn another_ae_word(command: &str) -> Option<String> {
    if !command.contains("orchestrator") && !command.contains("_session-menu") {
        return None;
    }
    command.split_whitespace().find_map(|token| {
        let word = token.trim_matches(|ch| ch == '\'' || ch == '"');
        let base = word.rsplit('/').next().unwrap_or_default();
        (base == "ae" || base == "ae-core").then(|| word.to_owned())
    })
}

/// The rows about the INPUT MAP: whether ae's status-line clicks and picker
/// hotkey are still bound on every ae-owned server that answered. Report only
/// — doctor repairs nothing. Warn, never Fail.
fn bindings_rows(facts: &Facts, out: &mut Report) {
    if facts.bindings.is_empty() {
        out.push(
            Level::Ok,
            "tmux.bindings",
            "no reachable ae-owned server — nothing to check",
        );
        return;
    }
    for found in &facts.bindings {
        match &found.status {
            BindingsStatus::Intact => out.push(
                Level::Ok,
                "tmux.bindings",
                &format!(
                    "{}: status-line clicks and the picker hotkey are bound to this ae (tmux {})",
                    found.server, found.version
                ),
            ),
            BindingsStatus::Broken(problems) => out.push(
                Level::Warn,
                "tmux.bindings",
                &format!(
                    "{}: {}; reassert with 'ae <session>' or 'ae upgrade'",
                    found.server,
                    problems.join("; ")
                ),
            ),
            BindingsStatus::Unreadable { why } => out.push(
                Level::Warn,
                "tmux.bindings",
                &format!("{}: key bindings unreadable ({why})", found.server),
            ),
        }
    }
}

/// An empty recorded version reads as `-`, never as a blank column.
fn blank(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

/// The executable word of a launch command, skipping a leading `env` and any
/// `VAR=val` prefix.
#[must_use]
pub fn executable_of(command: &str) -> Option<String> {
    crate::launch_cmd::split_binary(command).map(|split| split.binary)
}

/// Resolve `program` the way `command -v` does: an absolute or relative name
/// with a `/` is taken as given, anything else is looked up along `PATH`.
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

/// Whether anyone may WRITE `path` — `None` when it cannot be classified.
fn is_writable(path: &Path) -> Option<bool> {
    use std::os::unix::fs::PermissionsExt as _;
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: doctor reports whether the published core is still read-only, which is a fact about the file's mode — see clippy.toml"
    )]
    let probe = std::fs::symlink_metadata(path);
    probe
        .ok()
        .map(|meta| meta.permissions().mode() & 0o222 != 0)
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
#[must_use]
pub fn gather(root: &Path, global: Option<&Path>, local: Option<&Path>) -> Facts {
    let roots = crate::inventory::Roots::under(root);
    let config = global.map_or_else(|| root.join("config"), Path::to_path_buf);
    let read = crate::config::read_identity(Some(&config), local);
    let (mut config_error, identity) = match read {
        Ok(identity) => (None, identity),
        Err(why) => (
            Some(why.to_string()),
            crate::config::IdentityConfig::default(),
        ),
    };
    let home = crate::doors::home();
    let mut profiles: Vec<ProfileFacts> = Vec::with_capacity(identity.profiles.len());
    for (profile, raw) in &identity.profiles {
        let command = match identity.command(profile, home.as_deref()) {
            Ok(Some(command)) => command,
            Ok(None) => continue,
            Err(why) => {
                let why = why.to_string();
                if let Some(existing) = &mut config_error {
                    existing.push_str(" | ");
                    existing.push_str(&why);
                } else {
                    config_error = Some(why.clone());
                }
                profiles.push(ProfileFacts {
                    profile: profile.clone(),
                    command: raw.clone(),
                    executable: None,
                    resolves: false,
                    resolution_error: Some(why),
                });
                continue;
            }
        };
        let executable = executable_of(command.as_str());
        let resolves = executable
            .as_deref()
            .is_some_and(|exec| resolve_on_path(exec).is_some());
        profiles.push(ProfileFacts {
            profile: profile.clone(),
            command: command.into_string(),
            executable,
            resolves,
            resolution_error: None,
        });
    }
    profiles.sort_by(|left, right| left.profile.cmp(&right.profile));

    let core = crate::shape::resolved_exe();
    // GLOBAL only, like `auto_upgrade`, and read before `config` is moved. The
    // RAW reader, not `global_fleet_order`, which folds a broken line into "no
    // order" — correct for the bar, useless for a report.
    let (fleet_order, fleet_order_error) =
        match crate::config::read_global_workspace_key(&config, "fleet_order") {
            Ok(value) => (value.unwrap_or_default(), None),
            Err(why) => (String::new(), Some(why)),
        };
    let sessions = session_facts(root);
    let seats = seat_facts(root, &sessions);
    let bindings = bindings_facts(&sessions, root, &config, core.as_deref());
    Facts {
        version: crate::VERSION.to_owned(),
        core_writable: core.as_deref().and_then(is_writable),
        core_published: matches!(
            crate::shape::current(),
            crate::shape::Shape::Installed { .. }
        ),
        core,
        tmux: resolve_on_path("tmux"),
        // The DECLARED server, not the ambient one: a checkout honours the
        // `AE_TMUX_SERVER` pair, so that is the server a launch from here would
        // land on and the only one whose version answers the operator's
        // question. An ambiguous pair leaves the ambient server, which is what
        // every other command falls back to.
        tmux_floor: crate::transport::observe_tmux_floor(
            &crate::doors::probe_target(
                crate::doors::declared_server(crate::shape::current()).as_ref(),
            )
            .unwrap_or(crate::inventory::ServerId::Ambient),
        ),
        git: resolve_on_path("git"),
        config,
        config_error,
        local_config: local.map(Path::to_path_buf),
        main: identity.main.filter(|value| !value.is_empty()),
        workers: identity.workers.filter(|value| !value.is_empty()),
        fleet_order,
        fleet_order_error,
        profiles,
        sessions_dir: roots.sessions().to_owned(),
        worktrees_dir: roots.worktrees().to_owned(),
        sessions,
        seats,
        bindings,
        boot: crate::doors::boot_time(crate::shape::current()),
    }
}

/// PURE: what doctor may call a seat's reading. A seat is COVERED only when
/// the whole tree was read, its root included, and no harness is left that ae
/// cannot place — the same readings a reseat's stop refuses on.
fn seat_reading(
    probe: Option<&crate::tmux::ObservedPaneProbe>,
    table: Option<&[crate::procs::Proc]>,
) -> crate::procs::Observed {
    use crate::procs::{Lineage, harness_rows};
    let placed = |rows: &[_], pid| {
        !harness_rows(rows, pid)
            .iter()
            .any(|row| row.2 != Lineage::Under)
    };
    match (probe, table, probe.and_then(|probe| probe.pid)) {
        (Some(probe), Some(rows), Some(pid))
            if rows.iter().any(|row| row.pid == pid) && placed(rows, pid) =>
        {
            crate::procs::observed_harness(&probe.command, probe.pid, Some(rows))
        }
        _ => crate::procs::Observed::Unknown,
    }
}

/// Every live session's seats against their panes: ONE process table for the
/// whole run, and one pane read per seat. Reads only — nothing here writes.
fn seat_facts(root: &Path, sessions: &[SessionFacts]) -> Vec<SeatFacts> {
    use crate::procs::Observed;
    if !sessions.iter().any(|session| session.live) {
        return Vec::new();
    }
    let dirs = discovered(root);
    let table = crate::procs::snapshot();
    let mut out = Vec::new();
    for session in sessions.iter().filter(|session| session.live) {
        let Some((_, dir)) = dirs.iter().find(|(name, _)| *name == session.name) else {
            continue;
        };
        let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
        let slots =
            crate::transport::observe_slots(&session.server, &session.name).unwrap_or_default();
        for row in crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes)).roster() {
            let pane = slots
                .iter()
                .find(|stamped| stamped.slot == row.slot)
                .map(|stamped| stamped.pane.clone())
                .unwrap_or_default();
            let observed = if pane.is_empty() {
                Observed::Unknown
            } else {
                seat_reading(
                    crate::transport::observe_pane_probe(&session.server, &pane).as_ref(),
                    table.as_deref(),
                )
            };
            out.push(SeatFacts {
                session: session.name.clone(),
                agent: row.name.clone(),
                pane,
                binary: row.binary.clone().unwrap_or_default(),
                profile: row.profile.clone().unwrap_or_default(),
                observed,
            });
        }
    }
    out
}

/// The rows behind a reboot refusal: when the host booted, and when each
/// session last did something only a live session does.
///
/// A resume that refuses with "cannot verify whether tmux session '<name>' is
/// absent" is comparing these two numbers, so this is where a human reads them
/// rather than guessing.
fn reboot_rows(facts: &Facts, out: &mut Report) {
    let iso = |epoch: i64| crate::time::Timestamp::from_epoch(epoch).to_string();
    match facts.boot {
        Some(boot) => out.push(Level::Ok, "boot", &format!("host booted {}", iso(boot))),
        None => out.push(
            Level::Warn,
            "boot",
            "this host's boot time could not be read — a session whose tmux socket \
             has vanished cannot be proven gone",
        ),
    }
    for session in &facts.sessions {
        let label = format!("last-live:{}", session.name);
        match (session.last_live, facts.boot) {
            (crate::tmux::Evidence::At(last_live), Some(boot)) if last_live < boot => out.push(
                Level::Ok,
                &label,
                &format!("{} — before this boot", iso(last_live)),
            ),
            (crate::tmux::Evidence::At(last_live), _) => out.push(
                Level::Ok,
                &label,
                &format!("{} — this boot", iso(last_live)),
            ),
            (crate::tmux::Evidence::Silent, _) => out.push(
                Level::Warn,
                &label,
                "no recorded live activity — a vanished tmux socket cannot be proven gone",
            ),
            // DAMAGE, not absence: a record that is there and unreadable is the
            // one a human can actually repair, so it says so rather than
            // rendering as "nothing recorded".
            (crate::tmux::Evidence::Unreadable, _) => out.push(
                Level::Warn,
                &label,
                "a record of this session's own activity is there and could not be read — \
                 a vanished tmux socket cannot be proven gone",
            ),
        }
    }
}

/// Every durable session's record facts, in name order.
fn session_facts(root: &Path) -> Vec<SessionFacts> {
    let mut out: Vec<SessionFacts> = discovered(root)
        .into_iter()
        .map(|(name, dir)| {
            let bytes = crate::meta::read_bytes(&dir).unwrap_or_default();
            let core_bin = crate::lifecycle::meta_value(&bytes, "ae_core");
            // Liveness is asked of the session's OWN recorded server.
            let server = match crate::lifecycle::server_of(&bytes) {
                crate::meta::ServerSelector::Positive(selector) => {
                    crate::inventory::ServerId::Selected(selector)
                }
                _ => crate::inventory::ServerId::Ambient,
            };
            SessionFacts {
                live: crate::transport::session_exists(&server, &name),
                server,
                core_usable: !core_bin.is_empty() && is_executable_file(Path::new(&core_bin)),
                core_version: crate::lifecycle::meta_value(&bytes, "ae_core_version"),
                glue_version: crate::lifecycle::meta_value(&bytes, "ae_version"),
                last_live: crate::inventory::last_live(&dir),
                core_bin,
                name,
            }
        })
        .collect();
    out.sort_by(|left, right| left.name.cmp(&right.name));
    out
}

/// Every durable session directory the state root holds, both layouts,
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

/// The servers the input-map check reads: the running sessions' recorded
/// servers plus this invocation's launch target — the ambient server never.
/// A `None` target (an ambiguous declared pair) contributes nothing.
fn bindings_servers(
    sessions: &[SessionFacts],
    default: Option<crate::inventory::ServerId>,
) -> Vec<crate::inventory::ServerId> {
    let mut servers: Vec<crate::inventory::ServerId> = Vec::new();
    for session in sessions.iter().filter(|session| session.live) {
        let crate::inventory::ServerId::Selected(_) = &session.server else {
            continue;
        };
        if !servers.contains(&session.server) {
            servers.push(session.server.clone());
        }
    }
    if let Some(default) = default
        && !servers.contains(&default)
    {
        servers.push(default);
    }
    servers
}

/// One row per server: two spellings tmux proves are one socket keep the
/// first (recorded servers precede the default, so a record keeps its own
/// spelling). Unprovable equivalence keeps both rows — truthful. The proof is
/// the fleet's own [`crate::SocketPaths`], no second notion.
fn dedupe_servers(
    servers: Vec<crate::inventory::ServerId>,
    sockets: &mut crate::SocketPaths,
) -> Vec<crate::inventory::ServerId> {
    let mut distinct: Vec<crate::inventory::ServerId> = Vec::new();
    for server in servers {
        if distinct
            .iter()
            .any(|known| sockets.proven_same(known, &server))
        {
            continue;
        }
        distinct.push(server);
    }
    distinct
}

/// The input-map verdict for every server [`bindings_servers`] names —
/// read-only, through the existing tmux door. Only a server that positively
/// ANSWERED earns a row: an absent or unreachable one is skipped, never
/// warned about.
fn bindings_facts(
    sessions: &[SessionFacts],
    root: &Path,
    config: &Path,
    core: Option<&Path>,
) -> Vec<BindingsFacts> {
    let declared = crate::doors::declared_server(crate::shape::current());
    let servers = bindings_servers(sessions, crate::doors::launch_target(declared.as_ref()));
    let mut sockets = crate::SocketPaths::asking(crate::transport::observe_socket_path);
    let servers = dedupe_servers(servers, &mut sockets);
    let mut out = Vec::new();
    for server in &servers {
        let crate::tmux_floor::Probe::Server(found) = crate::transport::observe_tmux_floor(server)
        else {
            continue;
        };
        out.push(check_server_bindings(server, &found, root, config, core));
    }
    out
}

/// Judge one answered server's key tables against the expected set for its
/// own capability — the same probe the launch's assert reads.
fn check_server_bindings(
    server: &crate::inventory::ServerId,
    found: &str,
    root: &Path,
    config: &Path,
    core: Option<&Path>,
) -> BindingsFacts {
    BindingsFacts {
        server: crate::tmux_floor::server_label(server),
        version: found.trim().to_owned(),
        status: judge_server_bindings(server, found, root, config, core),
    }
}

fn judge_server_bindings(
    server: &crate::inventory::ServerId,
    found: &str,
    root: &Path,
    config: &Path,
    core: Option<&Path>,
) -> BindingsStatus {
    let unreadable = |why: String| BindingsStatus::Unreadable { why };
    let probe = crate::tmux_floor::Probe::Server(found.to_owned());
    // `verdict`, not `clears_floor`: doctor REPORTS the floor and refuses
    // nothing, which is why the gate pin does not count this site.
    if probe.verdict() != crate::tmux_floor::Verdict::Ok {
        return unreadable(format!(
            "tmux {found} is below the {} floor ae launches on",
            crate::tmux_floor::REQUIRED
        ));
    }
    let Some(core) = core else {
        return unreadable("this binary cannot name its own path".to_owned());
    };
    let launcher =
        crate::session_tmux::picker_launcher(crate::shape::current(), core, root, config, server);
    let Some(own) = launcher.last() else {
        return unreadable("this ae names no launcher to judge the map by".to_owned());
    };
    let expected = crate::session_tmux::expected_status_bindings(probe.menu_mouse());
    let mut tables: Vec<(String, Vec<crate::tmux::KeyBinding>)> = Vec::new();
    for entry in &expected {
        if tables.iter().any(|(table, _)| *table == entry.table) {
            continue;
        }
        let Some(keys) = crate::transport::observe_key_bindings(server, &entry.table) else {
            return unreadable(format!("list-keys -T {} did not answer", entry.table));
        };
        tables.push((entry.table.clone(), keys));
    }
    match compare_bindings(&expected, &tables, own).as_slice() {
        [] => BindingsStatus::Intact,
        problems => BindingsStatus::Broken(problems.to_vec()),
    }
}

/// What `doctor`'s argv asked for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Args {
    refresh: Option<String>,
    global: Option<PathBuf>,
    local: Option<PathBuf>,
}

/// Read `doctor`'s flags.
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
    // The two roots are CREATED here: doctor is where a fresh install acquires
    // them.
    let roots = crate::inventory::Roots::under(root);
    let _ = std::fs::create_dir_all(roots.sessions());
    let _ = std::fs::create_dir_all(roots.worktrees());

    let facts = gather(root, args.global.as_deref(), args.local.as_deref());
    let mut document = report(&facts);
    append_autoupgrade_status(crate::shape::current(), &mut document);
    report_pending_renames(root, &mut document);
    if let Some(target) = &args.refresh {
        refresh(root, target, args.global.as_deref(), &mut document);
    }
    write!(out, "{}", document.render())?;
    Ok(document.exit_code())
}

/// Pending rename transactions, if any: each names its phase and the proved
/// retry. Read-only — doctor never converges one itself (`doctor --refresh`
/// acquires no rename powers).
fn report_pending_renames(root: &Path, document: &mut Report) {
    for intent in crate::rename::pending_intents(root) {
        document.push(
            Level::Fail,
            &format!("rename:{}:{}", intent.old_name(), intent.new_name()),
            &format!(
                "rename '{}' → '{}' is in progress at phase '{}' — retry 'ae rename {} {}' to converge it forward",
                intent.old_name(),
                intent.new_name(),
                intent.phase(),
                intent.old_name(),
                intent.new_name()
            ),
        );
    }
    for damaged in crate::rename::pending_damaged(root) {
        let scope = match (&damaged.old, &damaged.new) {
            (Some(old), Some(new)) => format!("for '{old}' → '{new}'"),
            _ => "that names no attributable pair".to_owned(),
        };
        document.push(
            Level::Fail,
            "rename:damaged",
            &format!(
                "a damaged rename carrier {scope} is pending ({}) — repair or remove it by hand; no lifecycle operation may proceed over it",
                damaged.why
            ),
        );
    }
}

fn append_autoupgrade_status(shape: &crate::shape::Shape, document: &mut Report) {
    let status = crate::autoupgrade::status(shape);
    for (label, row) in [
        ("auto-upgrade", status.policy),
        ("upgrade-check", status.check),
    ] {
        document.push(
            if row.warning { Level::Warn } else { Level::Ok },
            label,
            &row.detail,
        );
    }
}

/// Republish one session's assets, or every session's.
fn refresh(root: &Path, target: &str, global: Option<&Path>, document: &mut Report) {
    let Some(core) = crate::shape::resolved_exe() else {
        document.push(
            Level::Fail,
            "refresh",
            "the core could not name its own binary",
        );
        return;
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

    // The pin is rebound to the binary DOING the refresh.
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

    // The manifest reads the session's OWN recorded config and local overlay,
    // which is why a refresh renders the same document a launch does.
    let origin = or_dot(value("origin"));
    let work_dir = or_dot(value("work_dir"));
    let mut config_files: Vec<PathBuf> = Vec::new();
    let recorded = value("config");
    if !recorded.is_empty() {
        config_files.push(PathBuf::from(recorded));
    } else if let Some(global) = global {
        config_files.push(global.to_path_buf());
    }
    if let Some(local) = crate::config::local_overlay(dir, &origin) {
        config_files.push(local);
    }
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

/// A missing path fact renders as `.`.
fn or_dot(value: String) -> String {
    if value.is_empty() {
        ".".to_owned()
    } else {
        value
    }
}

/// `_check-deps` — the launch prelude's gate.
///
/// # Errors
///
/// Propagates a write failure on the caller's streams.
pub fn check_deps(tail: &[String], err: &mut impl Write) -> crate::Result<u8> {
    if !tail.is_empty() {
        writeln!(err, "{CHECK_DEPS_USAGE}")?;
        return Ok(EXIT_USAGE);
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
    let Some(core) = crate::shape::resolved_exe() else {
        writeln!(err, "ae: the core could not name its own binary.")?;
        return Ok(EXIT_FAILED);
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
            core_writable: Some(false),
            core_published: true,
            tmux: Some(PathBuf::from("/usr/bin/tmux")),
            tmux_floor: crate::tmux_floor::Probe::Executable("3.7b".to_owned()),
            boot: Some(1_789_105_855),
            git: Some(PathBuf::from("/usr/bin/git")),
            config: PathBuf::from("/home/me/.ae/config"),
            config_error: None,
            local_config: None,
            main: Some("lead".to_owned()),
            workers: Some("colead".to_owned()),
            fleet_order: String::new(),
            fleet_order_error: None,
            profiles: vec![ProfileFacts {
                profile: "cl".to_owned(),
                command: "claude --dangerously-skip-permissions".to_owned(),
                executable: Some("claude".to_owned()),
                resolves: true,
                resolution_error: None,
            }],
            sessions_dir: PathBuf::from("/home/me/.ae/sessions"),
            worktrees_dir: PathBuf::from("/home/me/.ae/worktrees"),
            sessions: Vec::new(),
            seats: Vec::new(),
            bindings: Vec::new(),
        }
    }

    #[test]
    fn a_seat_is_covered_only_when_its_whole_tree_was_read() {
        use crate::procs::{Observed, Proc};
        use crate::tmux::ObservedPaneProbe;
        let probe = |pid| ObservedPaneProbe {
            command: "codex".to_owned(),
            pid,
        };
        let row = |pid, ppid, comm: &str| Proc {
            pid,
            ppid,
            comm: comm.to_owned(),
        };
        let table = [row(100, 1, "codex")];
        let unplaced = [row(100, 1, "codex"), row(900, 850, "claude")];
        assert_eq!(
            seat_reading(Some(&probe(Some(100))), Some(&table)),
            Observed::Harness(vec![crate::tool::ToolKind::Codex])
        );
        // A harness-named foreground over an UNREAD tree is not a check — nor
        // over a table missing the pane's root, or holding a harness ae cannot
        // place, which is exactly what a reseat's stop refuses on.
        for (probe, rows) in [
            (Some(probe(None)), Some(&table[..])),
            (Some(probe(Some(100))), None),
            (Some(probe(Some(100))), Some(&[][..])),
            (Some(probe(Some(100))), Some(&unplaced[..])),
            (None, Some(&table[..])),
        ] {
            assert_eq!(seat_reading(probe.as_ref(), rows), Observed::Unknown);
        }
    }

    #[test]
    fn a_seat_whose_pane_runs_another_harness_is_named_and_uncovered_ones_are_counted() {
        use crate::procs::Observed;
        use crate::tool::ToolKind;
        let seat = |agent: &str, binary: &str, observed| SeatFacts {
            session: "infra".to_owned(),
            agent: agent.to_owned(),
            pane: format!("%{}", agent.len()),
            binary: binary.to_owned(),
            profile: format!("p-{agent}"),
            observed,
        };
        let rows_of = |seats: &[SeatFacts]| {
            let mut out = Report::default();
            seat_identity_rows(seats, &mut out);
            out.rows
        };
        // NO LIVE SEAT, NO ROW: nothing was there to check.
        assert!(rows_of(&[]).is_empty());
        let rows = rows_of(&[
            seat("lead", "claude", Observed::Harness(vec![ToolKind::Claude])),
            // THE INCIDENT: records codex, the pane runs claude.
            seat("colead", "codex", Observed::Harness(vec![ToolKind::Claude])),
            // Recorded and foreign together is never clean.
            seat(
                "w1",
                "codex",
                Observed::Harness(vec![ToolKind::Claude, ToolKind::Codex]),
            ),
            seat("w2", "grok", Observed::Shell),
            seat("w3", "grok", Observed::Unrecognised("vim".to_owned())),
            seat("w4", "grok", Observed::Unknown),
        ]);
        let found = |needle: &str| {
            rows.iter()
                .find(|row| row.detail.contains(needle))
                .map(|row| (row.level, row.label.clone()))
        };
        assert_eq!(
            found(
                "infra:colead (pane %6) records codex (profile 'p-colead'); its pane runs claude"
            ),
            Some((Level::Warn, "seat-identity".to_owned())),
            "{rows:?}"
        );
        assert_eq!(
            found(
                "infra:w1 (pane %2) records codex (profile 'p-w1'); its pane runs claude and codex"
            ),
            Some((Level::Warn, "seat-identity".to_owned())),
            "{rows:?}"
        );
        assert!(
            !rows.iter().any(|row| row.detail.contains("infra:lead")),
            "a matching seat is counted, never listed: {rows:?}"
        );
        assert!(
            rows.iter().any(|row| row.label == "seat-identity-hint"
                && row.level == Level::Warn
                && row.detail.contains("never rewrites")),
            "{rows:?}"
        );
        assert_eq!(
            found("3 of 6 live seat(s) checked"),
            Some((Level::Ok, "seat-identity".to_owned())),
            "{rows:?}"
        );
        assert!(
            found("uncovered: infra:w2 at a shell, infra:w3 runs 'vim', infra:w4 unreadable")
                .is_some(),
            "{rows:?}"
        );
        // All covered and all matching: one quiet line, no hint.
        let clean = rows_of(&[seat(
            "lead",
            "claude",
            Observed::Harness(vec![ToolKind::Claude]),
        )]);
        assert_eq!(clean.len(), 1, "{clean:?}");
        assert_eq!(clean[0].level, Level::Ok);
        assert_eq!(
            clean[0].detail,
            "1 of 1 live seat(s) checked; every checked seat runs the tool it records"
        );
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

    /// The floor row REPORTS, it never fails the report: an ae below the floor
    /// still lists, still says its version and still upgrades itself, and a
    /// FAIL here would make `ae doctor` exit non-zero on a sound install.
    #[test]
    fn the_tmux_floor_row_warns_below_the_floor_and_never_fails_the_report() {
        use crate::tmux_floor::Probe;

        let row_named = |input: &Facts| {
            report(input)
                .rows()
                .iter()
                .find(|row| row.label == "tmux-floor")
                .cloned()
                .unwrap_or_else(|| panic!("the report carries a tmux-floor row"))
        };

        let clean = row_named(&facts());
        assert_eq!(clean.level, Level::Ok);
        assert!(clean.detail.contains("3.7b"), "{}", clean.detail);
        assert!(clean.detail.contains("clears"), "{}", clean.detail);

        let mut old = facts();
        old.tmux_floor = Probe::Server("3.3a".to_owned());
        let below = row_named(&old);
        assert_eq!(below.level, Level::Warn);
        assert!(below.detail.contains("BELOW"), "{}", below.detail);
        assert!(
            below.detail.contains("brew install tmux"),
            "the row carries the fix: {}",
            below.detail
        );
        assert_eq!(
            report(&old).exit_code(),
            0,
            "a machine below the floor has a sound install"
        );

        let mut none = facts();
        none.tmux_floor = Probe::Silent;
        let missing = row_named(&none);
        assert_eq!(missing.level, Level::Warn);
        assert!(
            missing.detail.starts_with("tmux not found"),
            "{}",
            missing.detail
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
    fn a_client_resolution_failure_keeps_its_profile_and_reason() {
        let mut input = facts();
        input.profiles[0].resolution_error = Some("HOME unavailable".to_owned());
        let document = report(&input);
        assert!(
            document
                .render()
                .contains("FAIL  agent:cl       HOME unavailable"),
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

    /// PIN: a BROKEN `fleet_order` line is reported, not folded into "no order".
    /// The bar stays silent by design, so doctor is the only place that can say
    /// the key was LOST rather than never written.
    #[test]
    fn doctor_reports_a_fleet_order_line_it_could_not_read_at_all() {
        let mut input = facts();
        input.fleet_order_error = Some("fleet_order is malformed".to_owned());
        let text = report(&input).render();
        assert!(
            text.contains("WARN  workspace.fleet_order fleet_order is malformed\n"),
            "{text}"
        );
        assert!(
            !text.contains("no chosen order"),
            "a lost key must not read as a deliberate one: {text}"
        );
    }

    /// PIN: `ae doctor` is the ONE place a dropped `fleet_order` entry is named,
    /// and a session that is merely STOPPED is never one of them — ordering a
    /// fleet so it returns the same way across restarts is the point of the key.
    #[test]
    fn doctor_names_the_dropped_fleet_order_entries_but_never_a_stopped_session() {
        let recorded = |name: &str, live: bool| SessionFacts {
            name: name.to_owned(),
            live,
            server: crate::inventory::ServerId::Ambient,
            core_bin: "/c".to_owned(),
            core_usable: true,
            core_version: "2026.9.1".to_owned(),
            glue_version: "2026.9.1".to_owned(),
            last_live: crate::tmux::Evidence::Silent,
        };
        let mut input = facts();
        input.sessions = vec![recorded("aedev", true), recorded("infra", false)];
        // `aedve` is a legal session NAME and a typo all the same; `aedev`
        // repeats; `not a name` is illegal. `infra` is stopped and legitimate.
        // The grammar drops are named first, then the names ae has no record of.
        input.fleet_order = "aedev, infra, aedve, aedev, not a name".to_owned();
        let text = report(&input).render();
        assert!(
            text.contains("workspace.fleet_order aedev, infra — ignored: aedev, not a name, aedve"),
            "{text}"
        );
        // Clean order, clean row — and a stopped session still places.
        let mut input = facts();
        input.sessions = vec![recorded("aedev", true), recorded("infra", false)];
        input.fleet_order = "aedev, infra".to_owned();
        let document = report(&input);
        assert_eq!(document.failures(), 0);
        assert!(
            document
                .render()
                .contains("OK    workspace.fleet_order aedev, infra\n"),
            "{}",
            document.render()
        );
    }

    #[test]
    fn a_stopped_session_dir_is_an_orphan_warning_with_a_hint() {
        let mut input = facts();
        input.sessions.push(SessionFacts {
            name: "left".to_owned(),
            live: false,
            server: crate::inventory::ServerId::Ambient,
            core_bin: "/c".to_owned(),
            core_usable: true,
            core_version: "2026.9.1".to_owned(),
            glue_version: "2026.9.1".to_owned(),
            last_live: crate::tmux::Evidence::Silent,
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
            server: crate::inventory::ServerId::Ambient,
            core_bin: String::new(),
            core_usable: false,
            core_version: String::new(),
            glue_version: "2026.9.1".to_owned(),
            last_live: crate::tmux::Evidence::Silent,
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
            server: crate::inventory::ServerId::Ambient,
            core_bin: "/c".to_owned(),
            core_usable: true,
            core_version: "2026.8.4".to_owned(),
            glue_version: "2026.8.4".to_owned(),
            last_live: crate::tmux::Evidence::Silent,
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
    fn check_deps_takes_no_arguments_at_all() {
        // `_check-deps` takes no flags at all.
        let mut err = Vec::new();
        let code = check_deps(&["--bash-major".to_owned(), "5".to_owned()], &mut err).unwrap();
        assert_eq!(code, EXIT_USAGE);
        assert!(
            String::from_utf8_lossy(&err).contains(CHECK_DEPS_USAGE),
            "{}",
            String::from_utf8_lossy(&err)
        );
    }

    #[test]
    fn a_writable_core_is_a_warning_and_a_read_only_one_is_not() {
        let mut writable = facts();
        writable.core_writable = Some(true);
        let checkout = Facts {
            core_published: false,
            ..writable.clone()
        };
        let row = report(&writable)
            .rows
            .into_iter()
            .find(|row| row.label == "core")
            .expect("a core row");
        assert_eq!(row.level, Level::Warn);
        assert!(row.detail.contains("writable"), "{}", row.detail);

        let row = report(&facts())
            .rows
            .into_iter()
            .find(|row| row.label == "core")
            .expect("a core row");
        assert_eq!(row.level, Level::Ok);

        // A checkout build is writable because `cargo build` writes it.
        let row = report(&checkout)
            .rows
            .into_iter()
            .find(|row| row.label == "core")
            .expect("a core row");
        assert_eq!(row.level, Level::Ok);
    }

    #[test]
    fn no_row_reports_on_an_interpreter_ae_no_longer_ships() {
        let document = report(&facts());
        assert!(
            !document.rows.iter().any(|row| row.label == "bash"),
            "ae ships no bash, so there is no bash row to fill"
        );
    }

    /// PIN (a): the verdict half of the input-map check — intact, one key
    /// missing, a key bound to a foreign command, a stale Up binding present.
    #[test]
    fn the_input_map_compare_names_what_broke_and_nothing_else() {
        use crate::session_tmux::ExpectedBinding;
        use crate::tmux::KeyBinding;
        let entry = |table: &str, key: &str, absent: bool, names_launcher: bool| ExpectedBinding {
            table: table.to_owned(),
            key: key.to_owned(),
            absent,
            names_launcher,
        };
        let bound = |key: &str, command: &str| KeyBinding {
            key: key.to_owned(),
            command: command.to_owned(),
        };
        let expected = vec![
            entry("root", "MouseDown1Status", false, true),
            entry("root", "MouseUp1Status", true, false),
            entry("prefix", "a", false, true),
        ];
        let ae = "run-shell '/opt/ae' 'orchestrator'";
        let down = bound("MouseDown1Status", ae);
        let hotkey = bound("a", ae);
        let table = |root: Vec<KeyBinding>| {
            vec![
                ("root".to_owned(), root),
                ("prefix".to_owned(), vec![hotkey.clone()]),
            ]
        };
        assert!(compare_bindings(&expected, &table(vec![down.clone()]), "/opt/ae").is_empty());
        assert_eq!(
            compare_bindings(&expected, &table(Vec::new()), "/opt/ae"),
            vec!["missing root MouseDown1Status"]
        );
        assert_eq!(
            compare_bindings(
                &expected,
                &table(vec![bound("MouseDown1Status", "select-window -t {mouse}")]),
                "/opt/ae"
            ),
            vec!["root MouseDown1Status is bound to a foreign command"]
        );
        assert_eq!(
            compare_bindings(
                &expected,
                &table(vec![down, bound("MouseUp1Status", ae)]),
                "/opt/ae"
            ),
            vec!["stale root MouseUp1Status is still bound"]
        );
        // Presence-only: a launcher-less entry (the 3.4 Down pair) never reads
        // as foreign, whatever its command is.
        let legacy = vec![entry("root", "MouseDown1Status", false, false)];
        assert!(
            compare_bindings(
                &legacy,
                &table(vec![bound("MouseDown1Status", "select-window -t {mouse}")]),
                "/opt/ae"
            )
            .is_empty()
        );
    }

    /// PIN (a2): the owner's own argv round-trips clean on BOTH capabilities —
    /// the 3.4 Down pair carries no launcher and must not read as foreign —
    /// and a foreign command on a launcher-naming key still warns per row.
    #[test]
    fn the_owner_round_trips_clean_on_both_capabilities() {
        use crate::session_tmux::{expected_status_bindings, status_bindings_argv};
        use crate::tmux::KeyBinding;
        let server =
            crate::inventory::ServerId::Selected(crate::meta::Selector::Name("ae".to_owned()));
        let launcher = vec!["/opt/ae".to_owned()];
        for menu_mouse in [true, false] {
            let mut tables: Vec<(String, Vec<KeyBinding>)> = vec![
                ("root".to_owned(), Vec::new()),
                ("prefix".to_owned(), Vec::new()),
            ];
            for binding in status_bindings_argv(&server, &launcher, menu_mouse) {
                let words = binding.as_args();
                let Some(at) = words.iter().position(|word| word == "bind-key") else {
                    continue;
                };
                let (Some(table), Some(key)) = (words.get(at + 2), words.get(at + 3)) else {
                    continue;
                };
                tables
                    .iter_mut()
                    .find(|(name, _)| name == table)
                    .expect("the owner binds root and prefix only")
                    .1
                    .push(KeyBinding {
                        key: key.clone(),
                        command: words[(at + 4)..].join(" "),
                    });
            }
            let expected = expected_status_bindings(menu_mouse);
            assert!(
                compare_bindings(&expected, &tables, "/opt/ae").is_empty(),
                "menu_mouse={menu_mouse}"
            );
            let foreign_key = if menu_mouse {
                "MouseDown1Status"
            } else {
                "MouseUp1Status"
            };
            let mut dirty = tables.clone();
            dirty
                .iter_mut()
                .find(|(name, _)| name == "root")
                .expect("a root table")
                .1
                .iter_mut()
                .find(|listed| listed.key == foreign_key)
                .expect("the launcher-naming key")
                .command = "select-window -t {mouse}".to_owned();
            assert_eq!(
                compare_bindings(&expected, &dirty, "/opt/ae"),
                vec![format!("root {foreign_key} is bound to a foreign command")],
                "menu_mouse={menu_mouse}"
            );
        }
    }

    /// PIN: the check reads running recorded servers plus this invocation's
    /// launch target — stopped sessions, ambient records and an ambiguous
    /// target contribute nothing.
    #[test]
    fn the_bindings_check_reads_running_servers_and_the_launch_target() {
        use crate::inventory::ServerId;
        use crate::meta::Selector;
        let named = |name: &str| ServerId::Selected(Selector::Name(name.to_owned()));
        let recorded = |name: &str, live: bool, server: ServerId| SessionFacts {
            name: name.to_owned(),
            live,
            server,
            core_bin: "/c".to_owned(),
            core_usable: true,
            core_version: "2026.9.1".to_owned(),
            glue_version: "2026.9.1".to_owned(),
            last_live: crate::tmux::Evidence::Silent,
        };
        let sessions = vec![
            recorded("one", true, named("sock-a")),
            recorded("two", true, named("sock-a")),
            recorded("parked", false, named("sock-b")),
            recorded("loose", true, ServerId::Ambient),
        ];
        assert_eq!(
            bindings_servers(&sessions, Some(named("ae"))),
            vec![named("sock-a"), named("ae")]
        );
        assert_eq!(bindings_servers(&sessions, None), vec![named("sock-a")]);
        assert_eq!(
            bindings_servers(&sessions, Some(named("sock-a"))),
            vec![named("sock-a")]
        );
        assert!(bindings_servers(&[], None).is_empty());
    }

    /// PIN (bindshape, b): one row per server — two spellings tmux proves are
    /// one socket keep the first; unprovable equivalence keeps both rows.
    #[test]
    fn the_bindings_check_reports_one_server_once() {
        use crate::inventory::ServerId;
        use crate::meta::Selector;
        fn observed(server: &ServerId) -> Option<String> {
            match server {
                ServerId::Selected(Selector::Name(name)) if name == "elsewhere" => None,
                _ => Some("/private/tmp/tmux-501/ae".to_owned()),
            }
        }
        let named = ServerId::Selected(Selector::Name("ae".to_owned()));
        let socket = ServerId::Selected(Selector::Socket("/private/tmp/tmux-501/ae".into()));
        let elsewhere = ServerId::Selected(Selector::Name("elsewhere".to_owned()));
        let mut proven = crate::SocketPaths::asking(observed);
        assert_eq!(
            dedupe_servers(vec![named.clone(), socket.clone()], &mut proven),
            vec![named.clone()]
        );
        let mut proven = crate::SocketPaths::asking(observed);
        assert_eq!(
            dedupe_servers(vec![socket.clone(), named.clone()], &mut proven),
            vec![socket]
        );
        let mut unproven = crate::SocketPaths::asking(observed);
        assert_eq!(
            dedupe_servers(vec![named.clone(), elsewhere.clone()], &mut unproven),
            vec![named, elsewhere]
        );
    }

    /// PIN (bindshape, c): a key bound to another ae's launcher names that ae —
    /// only a command naming no ae at all reads as foreign.
    #[test]
    fn the_input_map_compare_names_another_ae_instead_of_crying_foreign() {
        use crate::session_tmux::ExpectedBinding;
        use crate::tmux::KeyBinding;
        let expected = vec![ExpectedBinding {
            table: "root".to_owned(),
            key: "MouseDown1Status".to_owned(),
            absent: false,
            names_launcher: true,
        }];
        let listed = |command: &str| {
            vec![(
                "root".to_owned(),
                vec![KeyBinding {
                    key: "MouseDown1Status".to_owned(),
                    command: command.to_owned(),
                }],
            )]
        };
        // Another ae, checkout shape: not this ae's link, but an ae launcher.
        assert_eq!(
            compare_bindings(
                &expected,
                &listed(
                    "run-shell -b 'env' 'AE_HOME=/h/.ae' \
                     '/h/.ae/versions/2026.9.117/ae-core' 'orchestrator' '--popup'"
                ),
                "/h/.local/bin/ae"
            ),
            vec![
                "root MouseDown1Status is bound by another ae: \
                 /h/.ae/versions/2026.9.117/ae-core"
            ]
        );
        // This ae's own launcher stays intact.
        assert!(
            compare_bindings(
                &expected,
                &listed("run-shell -b '/h/.local/bin/ae' 'orchestrator' '--popup'"),
                "/h/.local/bin/ae"
            )
            .is_empty()
        );
        // No ae word at all stays foreign.
        assert_eq!(
            compare_bindings(
                &expected,
                &listed("select-window -t {mouse}"),
                "/h/.local/bin/ae"
            ),
            vec!["root MouseDown1Status is bound to a foreign command"]
        );
    }

    /// PIN (c): the `tmux.bindings` row text — intact, missing, foreign,
    /// unreadable, and no server at all.
    #[test]
    fn the_bindings_row_names_the_server_the_key_and_the_repair() {
        let row_named = |input: &Facts| {
            report(input)
                .rows()
                .iter()
                .find(|row| row.label == "tmux.bindings")
                .cloned()
                .unwrap_or_else(|| panic!("the report carries a tmux.bindings row"))
        };
        let quiet = row_named(&facts());
        assert_eq!(quiet.level, Level::Ok);
        assert!(
            quiet.detail.contains("nothing to check"),
            "{}",
            quiet.detail
        );

        let found = |status| BindingsFacts {
            server: "-L ae".to_owned(),
            version: "3.7b".to_owned(),
            status,
        };
        // (status, level, needle): every row names its server, and a broken
        // one names the key and the repair — Warn, never Fail.
        for (status, level, needle) in [
            (BindingsStatus::Intact, Level::Ok, "bound to this ae"),
            (
                BindingsStatus::Broken(vec!["missing root MouseDown1Status".to_owned()]),
                Level::Warn,
                "missing root MouseDown1Status",
            ),
            (
                BindingsStatus::Broken(vec!["prefix a is bound to a foreign command".to_owned()]),
                Level::Warn,
                "prefix a is bound to a foreign command",
            ),
            (
                BindingsStatus::Broken(vec!["stale root MouseUp1Status is still bound".to_owned()]),
                Level::Warn,
                "stale root MouseUp1Status is still bound",
            ),
            (
                BindingsStatus::Unreadable {
                    why: "tmux 3.3a is below the 3.4 floor ae launches on".to_owned(),
                },
                Level::Warn,
                "key bindings unreadable (tmux 3.3a",
            ),
        ] {
            let mut input = facts();
            input.bindings = vec![found(status)];
            let row = row_named(&input);
            assert_eq!(row.level, level, "{}", row.detail);
            assert!(row.detail.contains("-L ae"), "{}", row.detail);
            assert!(row.detail.contains(needle), "{}", row.detail);
            if level == Level::Warn && !needle.contains("unreadable") {
                assert!(row.detail.contains("reassert with"), "{}", row.detail);
            }
            assert_eq!(
                report(&input).failures(),
                0,
                "never a failure: {}",
                row.detail
            );
        }
        let mut input = facts();
        input.bindings = vec![found(BindingsStatus::Unreadable {
            why: "list-keys -T root did not answer".to_owned(),
        })];
        assert!(
            !row_named(&input).detail.contains("intact"),
            "never a false intact"
        );
    }
}
