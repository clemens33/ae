//! `_launch`: a whole session, created or resumed, as ONE core operation.
//!
//! The whole launch path: the flag parse, the explicit session name, the
//! teardown tombstones, the `--from` preflight, the
//! working-directory modes, the tmux session and its layout, the meta publish,
//! the session assets, the pane commands and their readiness-gated paste, the
//! monitor panes, and the attach.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{self, IdentityConfig, Seat};
use crate::inventory::ServerId;
use crate::launch::{self, PENDING};
use crate::meta::{self, Meta, ServerSelector};
use crate::session_tmux::{
    Op, Split, TmuxArgv, argv, interpret_pane_id, picker_launcher, status_bindings_argv,
};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::tool::ToolKind;
use crate::{deliver, roster, tmux, transport};

pub(crate) mod assets;
pub(crate) mod capture;
pub(crate) mod name;

/// The usage line for the core entry.
pub const USAGE: &str = "Usage: _launch --home <ae-home> --cwd <dir> [--global <cfg>] [--local <cfg>] [--server-kind <kind>] [--server <value>] [--caller-socket <path>] [--attach|--no-attach] [--no-autostart] [--] [--worktree|--copy|--local] [--dir <path>] [--no-attach] [--solo] [--from <uuid>] [--seat <agent>=<profile>] [--lead <profile>] [--colead <profile>] [use <name>] <session-name>";

/// The public refusal for a launch argv that names no session.
pub const MISSING_NAME: &str = "Error: a session needs a name: ae <name> [...]";

/// The public launch usage line paired with [`MISSING_NAME`].
pub const PUBLIC_USAGE: &str = "Usage: ae <name> [--local|--copy|--worktree] [--dir <path>] [--no-attach] [--solo] [--from <archive-uuid>] [--seat <agent>=<profile>] [--lead <profile>] [--colead <profile>] [use <agent>]";

/// How long a freshly created pane's shell is given to draw its prompt before
/// anything is pasted.
const SHELL_SETTLE: Duration = Duration::from_millis(300);

/// How many polls the launch prompt's readiness wait takes.
const LAUNCH_READY_POLLS: u32 = 90;

/// How many polls the tool-process wait takes.
const START_POLLS: u32 = 10;

/// The pause between those polls.
const START_POLL: Duration = Duration::from_millis(100);

/// How long to hold the lifecycle lock for before giving up.
const LIFECYCLE_WAIT: Duration = Duration::from_secs(15);

/// The role state captured by a settings-menu launch row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExpectedState {
    /// No saved or live canonical role seat existed.
    AbsentCanonical,
    /// This exact role identity existed and was stopped.
    StoppedRole { uuid: String },
    /// This exact session name was proven stopped on the invoking server when
    /// the picker row was drawn. The clicker's identity and the deadline are
    /// the whole contribution: the launch's own meta re-read and absence proof
    /// decide everything about the session itself, under the lifecycle lock.
    StoppedSession,
}

/// The fixed clicker and role expectation carried into the launch lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExpectedLaunch {
    state: ExpectedState,
    server: ServerId,
    server_pid: String,
    server_start: String,
    client: String,
    client_pid: String,
    deadline: i64,
    #[cfg(debug_assertions)]
    test_pre_lock_marker: bool,
}

impl ExpectedLaunch {
    #[allow(clippy::too_many_arguments, reason = "one captured identity tuple")]
    pub(crate) fn new(
        state: ExpectedState,
        server: ServerId,
        server_pid: String,
        server_start: String,
        client: String,
        client_pid: String,
        deadline: i64,
    ) -> Self {
        Self {
            state,
            server,
            server_pid,
            server_start,
            client,
            client_pid,
            deadline,
            #[cfg(debug_assertions)]
            test_pre_lock_marker: false,
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn enable_test_pre_lock_marker(&mut self) {
        self.test_pre_lock_marker = true;
    }

    /// Prove that the captured row is still inside its validity window, then
    /// re-prove the server and attachment that selected it.
    pub(crate) fn check_action(&self, now: i64) -> Result<(), String> {
        if now > self.deadline {
            return Err("the menu action expired".to_owned());
        }
        if now < self.deadline - crate::session_menu::CONFIRM_WINDOW_SECS {
            return Err("the menu action is stamped in the future".to_owned());
        }
        self.check_attachment()
    }

    /// Re-prove only the server and attachment that selected the row.
    ///
    /// Failure reports use this narrower proof: an expired action may tell its
    /// still-attached clicker why it refused, but a forged or replaced client
    /// may never receive a message.
    pub(crate) fn check_attachment(&self) -> Result<(), String> {
        let Some(server) = transport::observe_server_identity(&self.server) else {
            return Err("the invoking tmux server no longer answers".to_owned());
        };
        if server.pid != self.server_pid || server.start != self.server_start {
            return Err("the invoking tmux server was replaced".to_owned());
        }
        let Some(client) = transport::observe_menu_client(&self.server, &self.client) else {
            return Err("the invoking client is no longer attached".to_owned());
        };
        if client.pid != self.client_pid {
            return Err("the invoking client is a different attachment now".to_owned());
        }
        Ok(())
    }

    const fn is_start(&self) -> bool {
        matches!(self.state, ExpectedState::AbsentCanonical)
    }

    const fn is_resume(&self) -> bool {
        matches!(
            self.state,
            ExpectedState::StoppedRole { .. } | ExpectedState::StoppedSession
        )
    }
}

/// Debug-build synchronization point for lifecycle-lock race tests.
#[doc(hidden)]
pub const TEST_PRE_LOCK_MARKER: &str = ".test-launch-before-lifecycle-lock";

/// The event log's resume-time retention, in lines.
const EVENTS_KEEP: usize = 1000;

/// The `launch-delivery-failed` action: the watchdog and the digest both key
/// on it.
const LAUNCH_FAILED_ACTION: &str = "launch-delivery-failed";

// ---------------------------------------------------------------------------
// argv
// ---------------------------------------------------------------------------

/// Which working copy a launch creates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `--local`: work in the caller's own directory, copy nothing.
    Local,
    /// `--worktree`: a detached git worktree under the worktrees root.
    Git,
    /// `--copy`: a full recursive copy, untracked files included.
    Full,
}

impl Mode {
    /// The spelling the meta records — the `mode=` values.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Git => "git",
            Self::Full => "full",
        }
    }

    /// The mode a recorded value names, or `None` when it names none.
    fn parse(value: &str) -> Option<Self> {
        match value {
            "local" => Some(Self::Local),
            "git" => Some(Self::Git),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

/// What the user's own argv said.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The mode flag, when one was given.
    pub mode: Option<Mode>,
    /// The session name, when one was typed.
    pub name: Option<String>,
    /// `use <name>` — the main-seat override for this launch.
    pub main: Option<String>,
    /// `--from <uuid>` — the archive this session explicitly continues.
    pub from: Option<String>,
    /// The FROZEN worker list, when a relaunch supplies one.
    pub workers: Option<String>,
    /// Per-agent profile replacements for this launch, in command-line order.
    pub seat_profiles: Vec<(String, String)>,
    /// `--dir <path>` — the explicit origin for this launch.
    pub dir: Option<String>,
    /// A public attach override. `None` keeps the preamble's default.
    pub attach: Option<bool>,
}

/// Parse the user's launch argv.
///
/// # Errors
///
/// The refusal line, ready for stderr.
pub fn parse_plan(args: &[String]) -> Result<Plan, String> {
    parse_plan_with_attach(args, false)
}

/// Parse the same grammar for the orchestrator seat, whose existing public
/// surface also permits an explicit `--attach`.
#[allow(
    clippy::too_many_lines,
    reason = "one owner for the complete public launch grammar and its duplicate checks"
)]
pub(crate) fn parse_plan_with_attach(args: &[String], allow_attach: bool) -> Result<Plan, String> {
    let mut plan = Plan::default();
    let mut colead_override = false;
    let mut rest = args;
    while let [word, tail @ ..] = rest {
        rest = tail;
        match word.as_str() {
            "--worktree" => plan.mode = Some(Mode::Git),
            "--copy" => plan.mode = Some(Mode::Full),
            "--local" => plan.mode = Some(Mode::Local),
            "--attach" if allow_attach => plan.attach = Some(true),
            "--no-attach" => plan.attach = Some(false),
            "--solo" if !allow_attach => plan.workers = Some(String::new()),
            _ if word == "--dir" || word.starts_with("--dir=") => {
                if plan.dir.is_some() {
                    return Err(
                        "Error: --dir may be given only once — a launch has one origin.".to_owned(),
                    );
                }
                let value = if let Some(inline) = word.strip_prefix("--dir=") {
                    inline.to_owned()
                } else {
                    match rest.split_first() {
                        Some((value, tail)) => {
                            rest = tail;
                            value.clone()
                        }
                        None => String::new(),
                    }
                };
                if value.is_empty() {
                    return Err("Error: --dir requires a path.".to_owned());
                }
                plan.dir = Some(value);
            }
            "use" => {
                let Some((value, tail)) = rest.split_first() else {
                    return Err("Error: 'use' requires an agent NAME bound in [roster] (e.g. ae myproject use colead)".to_owned());
                };
                plan.main = Some(value.clone());
                rest = tail;
            }
            _ if !allow_attach
                && (word == "--lead"
                    || word.starts_with("--lead=")
                    || word == "--colead"
                    || word.starts_with("--colead=")
                    || word == "--seat"
                    || word.starts_with("--seat=")) =>
            {
                let (agent, inline) = if let Some(value) = word.strip_prefix("--lead=") {
                    ("lead", Some(value))
                } else if word == "--lead" {
                    ("lead", None)
                } else if let Some(value) = word.strip_prefix("--colead=") {
                    colead_override = true;
                    ("colead", Some(value))
                } else if word == "--colead" {
                    colead_override = true;
                    ("colead", None)
                } else {
                    let raw = if let Some(value) = word.strip_prefix("--seat=") {
                        value.to_owned()
                    } else {
                        match rest.split_first() {
                            Some((value, tail)) => {
                                rest = tail;
                                value.clone()
                            }
                            None => String::new(),
                        }
                    };
                    let Some((agent, profile)) = raw.split_once('=') else {
                        return Err(
                            "Error: --seat requires <agent>=<profile> (e.g. --seat lead=solx)."
                                .to_owned(),
                        );
                    };
                    if agent.is_empty() || profile.is_empty() {
                        return Err(
                            "Error: --seat requires non-empty <agent>=<profile>.".to_owned()
                        );
                    }
                    if plan.seat_profiles.iter().any(|(seen, _)| seen == agent) {
                        return Err(format!(
                            "Error: profile override for agent '{agent}' may be given only once."
                        ));
                    }
                    plan.seat_profiles
                        .push((agent.to_owned(), profile.to_owned()));
                    continue;
                };
                let profile = if let Some(value) = inline {
                    value.to_owned()
                } else {
                    match rest.split_first() {
                        Some((value, tail)) => {
                            rest = tail;
                            value.clone()
                        }
                        None => String::new(),
                    }
                };
                if profile.is_empty() {
                    return Err(format!("Error: --{agent} requires a profile."));
                }
                if plan.seat_profiles.iter().any(|(seen, _)| seen == agent) {
                    return Err(format!(
                        "Error: profile override for agent '{agent}' may be given only once."
                    ));
                }
                plan.seat_profiles.push((agent.to_owned(), profile));
            }
            _ if word == "--from" || word.starts_with("--from=") => {
                if plan.from.is_some() {
                    return Err("Error: --from may be given only once — a session inherits from exactly one archive.".to_owned());
                }
                let value = if let Some(inline) = word.strip_prefix("--from=") {
                    inline.to_owned()
                } else {
                    match rest.split_first() {
                        Some((value, tail)) => {
                            rest = tail;
                            value.clone()
                        }
                        None => String::new(),
                    }
                };
                if value.is_empty() {
                    return Err(
                        "Error: --from requires an archive UUID (list them: ls ~/.ae/archive)."
                            .to_owned(),
                    );
                }
                plan.from = Some(value);
            }
            _ if word.starts_with("--") => {
                return Err(format!(
                    "Error: unknown flag '{word}'. Use --worktree, --copy, --local, --dir <path>, --no-attach, --solo, --from <archive-uuid>, --seat <agent>=<profile>, --lead <profile>, or --colead <profile>."
                ));
            }
            // Last positional wins.
            _ => plan.name = Some(word.clone()),
        }
    }
    if colead_override && plan.workers.as_deref() == Some("") {
        return Err(
            "Error: '--solo' starts the lead alone; drop --colead/--seat <worker>.".to_owned(),
        );
    }
    Ok(plan)
}

/// The facts the glue hands in — everything the core would otherwise have to
/// read out of the environment, which the capability boundary denies it.
#[derive(Debug, Clone)]
pub struct Env {
    /// `AE_HOME`.
    pub home: PathBuf,
    /// The caller's working directory.
    pub cwd: PathBuf,
    /// The global config file, when one is selected.
    pub global: Option<PathBuf>,
    /// The origin-local `.ae/config`, when one is selected.
    pub local: Option<PathBuf>,
    /// `socket`, `name`, `ambiguous` or empty — the recorded server's kind.
    pub server_kind: String,
    /// The recorded server's value.
    pub server_value: String,
    /// The server whose tmux client invoked ae, when the caller is in tmux.
    pub caller_server: Option<ServerId>,
    /// Whether the caller is inside tmux, which decides attach vs switch.
    pub inside_tmux: bool,
    /// Whether to attach when the session is up.
    pub attach: bool,
    /// The core the glue RESOLVED (`_ae_core_bind`), recorded as `ae_core` with
    /// the version it reported — the pin every helper re-resolves from meta.
    pub core: Option<PathBuf>,
    /// The version the resolved core reported, when the caller measured it.
    pub core_version: Option<String>,
    /// `--no-autostart`: suppress the Telegram bridge.
    pub no_autostart: bool,
    /// Internal debug-build test seam: publish [`TEST_PRE_LOCK_MARKER`] after
    /// preflight and immediately before waiting for the lifecycle lock.
    test_pre_lock_marker: Option<PathBuf>,
}

impl Env {
    /// The sessions root.
    fn sessions(&self) -> PathBuf {
        self.home.join("sessions")
    }

    /// The managed working-copy root.
    fn worktrees(&self) -> PathBuf {
        self.home.join("worktrees")
    }

    /// The server every tmux call in this launch addresses.
    fn server(&self) -> ServerId {
        ServerId::from_typed_flags(&self.server_kind, &self.server_value)
            .unwrap_or(ServerId::Ambient)
    }

    /// The config files, in overlay order, for the document renders.
    fn config_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if let Some(global) = &self.global {
            files.push(global.clone());
        }
        if let Some(local) = &self.local {
            files.push(local.clone());
        }
        files
    }
}

/// Why a preamble could not be read.
enum EnvError {
    /// A flag ae has no arm for, or one missing its value.
    OffendingWord(String),
    /// A well-formed pair ae will not act on, already phrased for the operator.
    Refused(String),
}

/// Read the preamble flags, then the user's own argv after `--`.
///
/// # Errors
///
/// The offending word, or the refusal.
fn read_env(tail: &[String]) -> Result<(Env, Vec<String>), EnvError> {
    let mut env = Env {
        home: PathBuf::new(),
        cwd: PathBuf::new(),
        global: None,
        local: None,
        server_kind: String::new(),
        server_value: String::new(),
        caller_server: None,
        inside_tmux: false,
        attach: true,
        no_autostart: false,
        test_pre_lock_marker: None,
        core: None,
        core_version: None,
    };
    let mut rest = tail;
    while let [flag, after @ ..] = rest {
        match flag.as_str() {
            "--" => {
                rest = after;
                break;
            }
            "--attach" => {
                env.attach = true;
                rest = after;
                continue;
            }
            "--no-attach" => {
                env.attach = false;
                rest = after;
                continue;
            }
            "--inside-tmux" => {
                env.inside_tmux = true;
                rest = after;
                continue;
            }
            "--no-autostart" => {
                env.no_autostart = true;
                rest = after;
                continue;
            }
            "--test-pre-lock-marker" if cfg!(debug_assertions) => {
                env.test_pre_lock_marker = Some(TEST_PRE_LOCK_MARKER.into());
                rest = after;
                continue;
            }
            _ => {}
        }
        let Some((value, after)) = after.split_first() else {
            return Err(EnvError::OffendingWord(flag.clone()));
        };
        match flag.as_str() {
            "--home" => env.home = value.into(),
            "--cwd" => env.cwd = value.into(),
            "--global" => env.global = Some(value.into()),
            "--local-config" => env.local = Some(value.into()),
            "--server-kind" => env.server_kind.clone_from(value),
            "--server" => env.server_value.clone_from(value),
            "--caller-socket" => {
                env.caller_server =
                    Some(ServerId::from_typed_flags("socket", value).map_err(EnvError::Refused)?);
            }
            "--core" => env.core = Some(value.into()),
            "--core-version" => env.core_version = Some(value.clone()),
            _ => return Err(EnvError::OffendingWord(flag.clone())),
        }
        rest = after;
    }
    if env.home.as_os_str().is_empty() || env.cwd.as_os_str().is_empty() {
        return Err(EnvError::OffendingWord(
            "--home and --cwd are required".to_owned(),
        ));
    }
    // BEFORE ANY EFFECT, and it is the reason this lives in the parse: a kind
    // ae cannot type must NOT fall through to the ambient server: the launch
    // would build the session THERE and record the unusable pair in meta.
    if let Err(why) = ServerId::from_typed_flags(&env.server_kind, &env.server_value) {
        // The refusal says what could not be used; this line says what to do:
        // the pair arrives from the glue's AE_TMUX_SERVER* re-export, so the
        // human fixes it in the environment, not on this command line.
        return Err(EnvError::Refused(format!(
            "{why}\n  Set AE_TMUX_SERVER_KIND=name|socket with AE_TMUX_SERVER a server name or an absolute socket path, or launch from outside tmux."
        )));
    }
    Ok((env, rest.to_vec()))
}

// ---------------------------------------------------------------------------
// entry
// ---------------------------------------------------------------------------

/// `_launch …` — create or resume a session.
///
/// # Errors
///
/// Only a failure to write `out` or `err`. Every refusal is an exit code.
pub fn run(tail: &[String], out: &mut impl Write, err: &mut impl Write) -> crate::Result<u8> {
    run_with_expected(tail, None, out, err)
}

/// Settings-only entry into the same launch owner with a locked expectation.
pub(crate) fn run_expected(
    preamble: &crate::entry::Preamble,
    user: &[String],
    expected: &ExpectedLaunch,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let argv = expected_launch_argv(preamble, user, expected);
    run_with_expected(&argv[1..], Some(expected), out, err)
}

fn expected_launch_argv(
    preamble: &crate::entry::Preamble,
    user: &[String],
    expected: &ExpectedLaunch,
) -> Vec<String> {
    #[cfg(not(debug_assertions))]
    let _ = expected;
    let argv = preamble.launch_argv(user);
    #[cfg(debug_assertions)]
    let argv = {
        let mut argv = argv;
        if expected.test_pre_lock_marker
            && let Some(separator) = argv.iter().position(|word| word == "--")
        {
            argv.insert(separator, "--test-pre-lock-marker".to_owned());
        }
        argv
    };
    argv
}

fn run_with_expected(
    tail: &[String],
    expected: Option<&ExpectedLaunch>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let (mut env, args) = match read_env(tail) {
        Ok(pair) => pair,
        Err(EnvError::OffendingWord(word)) => {
            writeln!(err, "ae: {USAGE}")?;
            writeln!(err, "ae: offending word: {word}")?;
            return Ok(EXIT_USAGE);
        }
        Err(EnvError::Refused(why)) => {
            writeln!(err, "{why}")?;
            return Ok(EXIT_USAGE);
        }
    };
    let plan = match parse_plan(&args) {
        Ok(plan) => plan,
        // EXIT_USAGE, not EXIT_FAILED: every refusal `parse_plan` makes is a
        // caller who asked wrong — an unknown flag, `use` with no agent name, a
        // second `--from` — and the crate's exit contract keeps 2 ("you asked
        // wrong") distinct from 1 ("it went wrong") precisely so a script can
        // tell them apart. The same holds for an unknown top-level option:
        // `ae --frobnicate` reaches the launch grammar as the first parser
        // that defines a flag set, so answering 1 there would make the rule
        // unsatisfiable through the public binary.
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(EXIT_USAGE);
        }
    };
    if plan.name.is_none() {
        writeln!(err, "{MISSING_NAME}")?;
        writeln!(err, "{PUBLIC_USAGE}")?;
        return Ok(EXIT_USAGE);
    }
    if let Some(path) = plan.dir.as_deref() {
        let Some(origin) = canonical_directory(Path::new(path)) else {
            writeln!(
                err,
                "Error: --dir path '{path}' does not exist or is not a directory."
            )?;
            return Ok(EXIT_FAILED);
        };
        env.local = crate::doors::local_config(&origin);
        env.cwd = origin;
    }
    if let Some(attach) = plan.attach {
        env.attach = attach;
    }
    launch(&env, &plan, None, expected, out, err)
}

/// The facts a `compact` relaunch carries across the boundary.
pub struct Relaunch<'a> {
    /// The ae home the child's state lives under.
    pub home: &'a Path,
    /// The child's session name.
    pub name: &'a str,
    /// Its working-copy mode, as the frozen tuple recorded it.
    pub mode: &'a str,
    /// The directory the child runs in — the source's recorded origin.
    pub origin: &'a str,
    /// The global config the source session recorded, or empty.
    pub config: &'a str,
    /// The archive the child inherits.
    pub uuid: &'a str,
    /// The `--from` proof compact took at the boundary, `\t`-separated.
    pub proof: &'a str,
    /// `main=<name> workers=<a,b|->` — the roster the freeze resolved.
    pub roster: &'a str,
    /// The recorded tmux server, so the child lands where its parent was.
    pub server_kind: &'a str,
    /// That server's value.
    pub server_value: &'a str,
}

/// Start the child of a `compact`, in this process.
///
/// # Errors
///
/// Only a failure to write `out` or `err`.
pub fn relaunch(
    plan: &Relaunch<'_>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let env = Env {
        home: plan.home.to_path_buf(),
        cwd: PathBuf::from(plan.origin),
        global: (!plan.config.is_empty()).then(|| PathBuf::from(plan.config)),
        local: {
            let candidate = Path::new(plan.origin).join(".ae").join("config");
            node_exists(&candidate).then_some(candidate)
        },
        server_kind: plan.server_kind.to_owned(),
        server_value: plan.server_value.to_owned(),
        caller_server: None,
        inside_tmux: false,
        attach: false,
        // A relaunch IS a launch (compact's child), so Telegram autostart is
        // decided exactly as it is for one typed by hand.
        no_autostart: false,
        test_pre_lock_marker: None,
        core: None,
        core_version: None,
    };
    let (main, workers) = split_frozen_roster(plan.roster);
    let launch_plan = Plan {
        mode: Mode::parse(plan.mode),
        name: Some(plan.name.to_owned()),
        main,
        from: Some(plan.uuid.to_owned()),
        workers,
        seat_profiles: Vec::new(),
        dir: None,
        attach: None,
    };
    launch(&env, &launch_plan, Some(plan.proof), None, out, err)
}

/// Split `main=<name> workers=<a,b|->` into its two overrides.
fn split_frozen_roster(roster: &str) -> (Option<String>, Option<String>) {
    let Some(rest) = roster.strip_prefix("main=") else {
        return (None, None);
    };
    let (main, workers) = match rest.split_once(" workers=") {
        Some((main, workers)) => (main, workers),
        None => (rest, "-"),
    };
    if main.is_empty() {
        return (None, None);
    }
    let workers = if workers == "-" { "" } else { workers };
    (Some(main.to_owned()), Some(workers.to_owned()))
}

/// The floor refusal a launch of `session` on `server` must print, or `None`
/// when the launch may proceed.
///
/// THE ONE GATE, so the public path (which seeds a first-run config) and the
/// `_launch` entry (which migrates a resumable session's meta) refuse in the
/// same place with the same words, each BEFORE its own first write.
///
/// A session that is already RUNNING is exempt: it was created by a tmux that
/// cleared whatever floor stood then, reattaching to it writes nothing, and
/// refusing would strand the agents inside it with no way to reach them.
#[must_use]
pub(crate) fn floor_refusal(server: &ServerId, session: &str) -> Option<String> {
    if !session.is_empty() && transport::session_exists(server, session) {
        return None;
    }
    let probe = transport::observe_tmux_floor(server);
    (!probe.clears_floor()).then(|| crate::tmux_floor::refusal("launch", &probe, server))
}

/// One command-line seat override that could not be honored.
#[derive(Debug)]
enum SeatOverrideRefusal {
    /// The user named a profile or launch seat that does not exist, or asked a
    /// stopped conversation to cross harnesses.
    Usage(String),
    /// The selected config or profile command itself is not launchable.
    Failed(String),
}

/// One override resolved through the exact config snapshot preflight read.
struct ResolvedSeatOverride {
    agent: String,
    /// The bare profile label — `profile.<slot>` never carries `@`.
    profile: String,
    /// The honored `profile@client` label, if the selection named one.
    client: Option<String>,
    command: config::ResolvedCommand,
    parsed: crate::launch_cmd::SimpleCommand,
    /// Whether the CURRENT resolution proved the label pairing: false when it
    /// resolved `Unknown`, in which case the build honors the seat but records
    /// no new pairing.
    store_proven: bool,
}

/// Split a `--lead`/`--colead`/`--seat` value into its profile and optional
/// `profile@client` override label.
///
/// Neither the profile grammar (`is_config_key`) nor the client grammar
/// (`is_agent_name`) admits `@`, so one `@` splits unambiguously; an empty
/// half or a second `@` is a usage error naming the value.
fn split_seat_override(value: &str) -> Result<(String, Option<String>), String> {
    let Some((profile, client)) = value.split_once('@') else {
        return Ok((value.to_owned(), None));
    };
    if profile.is_empty() || client.is_empty() || client.contains('@') {
        return Err(format!(
            "Error: --seat selection '{value}' is not <profile> or <profile>@<client> \
             (e.g. --seat lead=solx, --lead fablex@cc-mic)."
        ));
    }
    Ok((profile.to_owned(), Some(client.to_owned())))
}

/// Identity plus parsed override commands carried unchanged through the lock.
struct SeatOverrideSnapshot {
    cfg: IdentityConfig,
    overrides: Vec<ResolvedSeatOverride>,
}

/// The launch-agent names an override may target, in seat order.
fn override_agents(cfg: &IdentityConfig, plan: &Plan, dir: &Path, resuming: bool) -> Vec<String> {
    if resuming {
        let Ok(bytes) = meta::read_bytes(dir) else {
            return Vec::new();
        };
        return Meta::parse(&String::from_utf8_lossy(&bytes))
            .roster()
            .iter()
            .filter(|entry| entry.slot == "main" || entry.slot.starts_with("worker."))
            .map(|entry| entry.name.clone())
            .collect();
    }

    let mut agents = Vec::new();
    if let Some(main) = plan.main.as_deref().or(cfg.main.as_deref()) {
        let main = main.trim();
        if !main.is_empty() && cfg.roster_profile(main).is_some() {
            agents.push(main.to_owned());
        }
    }
    let workers = plan.workers.as_deref().or(cfg.workers.as_deref());
    if let Some(workers) = workers {
        agents.extend(
            workers
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty() && cfg.roster_profile(name).is_some())
                .map(str::to_owned),
        );
    }
    agents
}

/// Read the config this launch will use without changing session state.
fn override_identity(
    env: &Env,
    dir: &Path,
    resuming: bool,
) -> Result<IdentityConfig, SeatOverrideRefusal> {
    let mut global = env.global.clone();
    let mut local = env.local.clone();
    if resuming {
        if let Some(stored) = meta_value(dir, "config").filter(|value| !value.is_empty()) {
            global = Some(PathBuf::from(stored));
        }
        let origin = meta_value(dir, "origin").unwrap_or_default();
        local = config::local_overlay(dir, &origin);
    }
    let read = if resuming {
        config::read_identity(global.as_deref(), local.as_deref())
    } else {
        config::read_identity_with_global_default(
            global.as_deref(),
            local.as_deref(),
            crate::entry::DEFAULT_CONFIG,
        )
    };
    read.map_err(|why| SeatOverrideRefusal::Failed(why.to_string()))
}

fn running_override_refusal(session: &str) -> String {
    format!("Error: session '{session}' is running; stop it before changing a seat profile.")
}

fn solo_resume_refusal(plan: &Plan, dir: &Path, resuming: bool) -> Option<String> {
    if plan.workers.as_deref() != Some("") || !resuming {
        return None;
    }
    let workers = meta::read_bytes(dir)
        .ok()
        .map(|bytes| Meta::parse(&String::from_utf8_lossy(&bytes)))
        .map(|parsed| {
            parsed
                .roster()
                .iter()
                .filter(|entry| entry.slot.starts_with("worker."))
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    (!workers.is_empty()).then(|| {
        format!(
            "Error: '{}' already has workers ({workers}); '--solo' applies to a first launch.",
            plan.name.as_deref().unwrap_or_default()
        )
    })
}

fn solo_worker_override_refusal(plan: &Plan, cfg: &IdentityConfig) -> Option<String> {
    if plan.workers.as_deref() != Some("") {
        return None;
    }
    let main = plan.main.as_deref().or(cfg.main.as_deref()).map(str::trim);
    let configured_workers = cfg
        .workers
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty() && Some(*name) != main);
    plan.seat_profiles
        .iter()
        .any(|(agent, _)| configured_workers.clone().any(|worker| worker == agent))
        .then(|| "Error: '--solo' starts the lead alone; drop --colead/--seat <worker>.".to_owned())
}

/// The recorded main identity when the standing roster has no worker seats.
///
/// Such a roster is the only durable fact a later launch has that this session
/// was created solo. Carrying it into the existing config snapshot prevents a
/// configured worker list from growing the roster on resume, while keeping the
/// command paired with the profile that the main seat recorded.
fn recorded_solo_identity(dir: &Path) -> Result<Option<(String, String)>, String> {
    let bytes = meta::read_bytes(dir).map_err(|why| format!("cannot read meta ({why})"))?;
    let parsed = Meta::parse(&String::from_utf8_lossy(&bytes));
    if parsed.schema() != Some("2") {
        return Err("missing schema=2".to_owned());
    }
    if let Some(anomaly) = parsed
        .anomalies()
        .iter()
        .find(|item| roster::roster_doubting(item))
    {
        return Err(anomaly.to_string());
    }
    let Some(main) = parsed.roster().iter().find(|entry| entry.slot == "main") else {
        return Err("missing seat.main".to_owned());
    };
    let Some(profile) = main.profile.as_ref().filter(|profile| !profile.is_empty()) else {
        return Err("missing profile.main".to_owned());
    };
    if parsed
        .roster()
        .iter()
        .any(|entry| entry.slot.starts_with("worker."))
    {
        return Ok(None);
    }
    Ok(Some((main.name.clone(), profile.clone())))
}

fn doubtful_solo_meta_refusal(plan: &Plan, anomaly: &str) -> String {
    format!(
        "Error: session '{}' has doubtful roster metadata ({anomaly}); run 'ae doctor' before resuming.",
        plan.name.as_deref().unwrap_or_default()
    )
}

fn frozen_solo_identity(
    plan: &Plan,
    dir: &Path,
    resuming: bool,
    running: bool,
) -> Result<Option<(String, String)>, SeatOverrideRefusal> {
    // The dedicated orchestrator already has a separate global-identity rule;
    // forcing its single-seat roster through this snapshot would re-enable
    // legacy identity rows from its local workspace overlay.
    if !resuming
        || running
        || plan.name.as_deref() == Some(crate::orchestrator::ORCHESTRATOR_SESSION)
    {
        return Ok(None);
    }
    recorded_solo_identity(dir)
        .map_err(|anomaly| SeatOverrideRefusal::Failed(doubtful_solo_meta_refusal(plan, &anomaly)))
}

fn freeze_solo_config(
    cfg: &mut IdentityConfig,
    plan: &Plan,
    recorded_main: &str,
    recorded_profile: &str,
) -> Result<(), SeatOverrideRefusal> {
    cfg.workers = Some(String::new());
    let selected_main = plan.main.as_deref().or(cfg.main.as_deref()).map(str::trim);
    let Some((_, bound_profile)) = cfg
        .roster
        .iter_mut()
        .find(|(name, _)| Some(name.as_str()) == selected_main)
    else {
        return Err(SeatOverrideRefusal::Failed(format!(
            "Error: recorded solo main '{recorded_main}' is not in [roster] of the current config."
        )));
    };
    recorded_profile.clone_into(bound_profile);
    Ok(())
}

/// Resolve one `profile@client` selection: both sides must exist, the pair
/// must substitute — and then the R1 gate: BOTH binaries must be the SAME
/// KNOWN harness. `Unknown` on either side refuses, because
/// `ToolKind::from_binary_name` is `from_known_binary_name(...).unwrap_or(Unknown)`
/// and any two unknowns would compare equal through it.
fn resolve_client_override(
    cfg: &IdentityConfig,
    agent: &str,
    profile: &str,
    label: &str,
    home: Option<&Path>,
    known_profiles: &str,
    known_clients: &str,
) -> Result<(config::ResolvedCommand, crate::launch_cmd::SimpleCommand), SeatOverrideRefusal> {
    let resolved = cfg
        .command_with_client(profile, label, home)
        .map_err(|why| match why {
            config::OverrideError::UnknownProfile => SeatOverrideRefusal::Usage(format!(
                "Error: unknown profile '{profile}' in --seat. Known profiles: {known_profiles}."
            )),
            config::OverrideError::UnknownClient => SeatOverrideRefusal::Usage(format!(
                "Error: unknown client '{label}' in --seat. Known clients: {known_clients}."
            )),
            config::OverrideError::ProfileNotSimple(why) => SeatOverrideRefusal::Failed(format!(
                "Error: [profiles] {profile} (seat '{agent}'): the launch command must be one simple command — it has {why}."
            )),
            config::OverrideError::Refused(why) => SeatOverrideRefusal::Failed(why.to_string()),
        })?;
    let parsed = crate::launch_cmd::lex_simple_command(resolved.command.as_str()).map_err(|why| {
        SeatOverrideRefusal::Failed(format!(
            "Error: [profiles] {profile} (seat '{agent}'): the launch command must be one simple command — it has {why}."
        ))
    })?;
    let original = ToolKind::from_known_binary_name(&resolved.original_binary);
    let switched = ToolKind::from_known_binary_name(&parsed.binary);
    if original.is_none() || switched.is_none() || original != switched {
        let from = match resolved.original_client.as_deref() {
            Some(client) => format!("profile '{profile}' (client '{client}')"),
            None => format!("profile '{profile}'"),
        };
        return Err(SeatOverrideRefusal::Usage(client_adapter_refusal(
            agent,
            profile,
            label,
            &from,
            (&resolved.original_binary, original),
            (&parsed.binary, switched),
        )));
    }
    Ok((resolved.command, parsed))
}

/// What one side of an `@` pair runs, for the refusal's two values.
fn adapter_word(binary: &str, known: Option<ToolKind>) -> String {
    match known {
        Some(kind) => kind.as_str().to_owned(),
        None => format!("an unrecognized binary '{binary}'"),
    }
}

fn client_adapter_refusal(
    agent: &str,
    profile: &str,
    label: &str,
    from: &str,
    original: (&str, Option<ToolKind>),
    switched: (&str, Option<ToolKind>),
) -> String {
    format!(
        "Error: cannot launch seat '{agent}' as '{profile}@{label}': {from} runs {} but client '{label}' runs {} — \
         a client override may change the account, never the harness adapter ('@' needs the same known harness on both sides). \
         Use a duplicate profile to cross harnesses.",
        adapter_word(original.0, original.1),
        adapter_word(switched.0, switched.1)
    )
}

/// A recorded `client.<slot>` row resolved against the CURRENT config.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordedSelection {
    /// No row: no override was recorded for this seat.
    Missing,
    /// Exactly this label was recorded, and it still names a client.
    Label(String),
}

/// A recorded row no launch may interpret: empty, duplicated or malformed.
fn invalid_client_refusal(session: &str, agent: &str, slot: &str) -> String {
    format!(
        "Error: session '{session}' seat '{agent}' records an unusable client (client.{slot} is empty, duplicated or malformed) — \
         fix the meta row, or end the session (ae end {session})."
    )
}

/// A recorded label that selects a different client than this launch.
fn different_label_refusal(
    session: &str,
    agent: &str,
    profile: &str,
    known: &str,
    label: &str,
) -> String {
    format!(
        "Error: session '{session}' seat '{agent}' recorded client override '{known}' but this launch selects '{label}' — \
         a recorded client is write-once. Resume with '{profile}@{known}', restore the '{known}' client, or end the session (ae end {session})."
    )
}

/// Resolve one recorded seat's client row, or refuse with its remedy.
///
/// Shape-Invalid and config-absent are BOTH unusable, but only the absent
/// label has a cheap remedy: restoring the removed or renamed `[clients]` row
/// is safe, because the row then reads `Label` again with its identity
/// unchanged. A malformed row must be fixed by hand. Either way `ae end` is
/// the last resort, never a silent fallback.
fn recorded_selection(
    session: &str,
    cfg: &IdentityConfig,
    entry: &meta::RosterEntry,
) -> Result<RecordedSelection, String> {
    match &entry.client {
        crate::meta::RecordedClient::Missing => Ok(RecordedSelection::Missing),
        crate::meta::RecordedClient::Label(label) if cfg.client(label).is_some() => {
            Ok(RecordedSelection::Label(label.clone()))
        }
        crate::meta::RecordedClient::Label(label) => Err(format!(
            "Error: session '{session}' seat '{}' recorded client override '{label}' but no [clients] row names it now — \
             restore the '{label}' client, or end the session (ae end {session}).",
            entry.name
        )),
        crate::meta::RecordedClient::Invalid => {
            Err(invalid_client_refusal(session, &entry.name, &entry.slot))
        }
    }
}

/// Refuse an override the recorded seat cannot honor.
///
/// A recorded client is write-once: only its exact label is accepted, any
/// different label refuses whatever its facts (config is mutable, so an
/// equality proof cannot cross a config edit), and a bare profile cannot drop
/// it. Whatever the labels say, the recorded STORE triple is re-taken against
/// the current resolution, which catches a label whose definition moved. The
/// `bool` answers whether that re-take PROVED the pairing — `false` under an
/// `Unknown` current resolution, which honors the seat but records nothing.
#[allow(
    clippy::too_many_arguments,
    reason = "one refusal site for one override against one recorded seat"
)]
fn refuse_recorded_client_conflict(
    session: &str,
    agent: &str,
    profile: &str,
    client: Option<&str>,
    entry: &meta::RosterEntry,
    cfg: &IdentityConfig,
    command: &config::ResolvedCommand,
    tool: ToolKind,
    home: Option<&Path>,
) -> Result<bool, SeatOverrideRefusal> {
    let recorded = recorded_selection(session, cfg, entry).map_err(SeatOverrideRefusal::Failed)?;
    match (recorded, client) {
        // No recorded override and none selected: the legacy path, untouched.
        (RecordedSelection::Missing, None) => Ok(true),
        // First override for this seat: the recorded store still rules.
        (RecordedSelection::Missing, Some(label)) => refuse_store_conflict(
            session,
            agent,
            &format!("{profile}@{label}"),
            entry,
            command,
            tool,
            home,
        ),
        // A bare profile names no label, so it cannot satisfy write-once.
        (RecordedSelection::Label(known), None) => Err(SeatOverrideRefusal::Usage(format!(
            "Error: session '{session}' seat '{agent}' recorded client override '{known}' — \
             re-pair with '{profile}@{known}' to keep it, or end the session (ae end {session}). \
             A bare profile cannot drop a recorded client."
        ))),
        // The exact label, re-proved against the current store below.
        (RecordedSelection::Label(known), Some(label)) if known == label => refuse_store_conflict(
            session,
            agent,
            &format!("{profile}@{label}"),
            entry,
            command,
            tool,
            home,
        ),
        // Any different label refuses, whatever its facts today.
        (RecordedSelection::Label(known), Some(label)) => Err(SeatOverrideRefusal::Usage(
            different_label_refusal(session, agent, profile, &known, label),
        )),
    }
}

/// One side of the R4 identity comparison, as the refusal names it.
fn recorded_store_shown(
    home: &crate::meta::RecordedConfigHome,
    base: &crate::meta::RecordedConfigHomeBase,
) -> String {
    match home {
        crate::meta::RecordedConfigHome::Path(path) => {
            format!("{} (explicit)", path.display())
        }
        crate::meta::RecordedConfigHome::Implicit(path) => match base {
            crate::meta::RecordedConfigHomeBase::Path(home) => {
                format!("{} (implicit, base {})", path.display(), home.display())
            }
            _ => format!("{} (implicit)", path.display()),
        },
        crate::meta::RecordedConfigHome::Absent => "absent".to_owned(),
        crate::meta::RecordedConfigHome::Unknown
        | crate::meta::RecordedConfigHome::Invalid
        | crate::meta::RecordedConfigHome::Missing => "unusable".to_owned(),
    }
}

/// The CURRENT resolution's proof of a label pairing: `None` when proved,
/// else the `Unknown` reason. Decided on the RAW resolution, so a first start
/// learns it without canonicalizing anything it will not compare.
fn current_pairing_proof(
    command: &config::ResolvedCommand,
    tool: ToolKind,
    home: Option<&Path>,
) -> Option<String> {
    let home_value = home.map(|path| path.display().to_string());
    let resolution = crate::launch_cmd::config_home_resolution(command, tool, &|name| {
        if name == "HOME" {
            home_value.clone()
        } else {
            None
        }
    });
    match resolution.home {
        crate::launch_cmd::Resolved::Unknown(reason) => Some(reason),
        _ => None,
    }
}

/// F6: an override whose current resolution is `Unknown` on a seat with no
/// retained store refuses — proceeding would ignore an explicit flag while
/// recording nothing recoverable, and a first start strands nothing.
fn unknown_store_refusal(agent: &str, spelling: &str, reason: &str) -> SeatOverrideRefusal {
    SeatOverrideRefusal::Usage(format!(
        "Error: cannot launch seat '{agent}' as '{spelling}': the override resolves to an unknown conversation store ({reason}) — \
         the launcher cannot verify where this conversation would live. Make the account path absolute, then retry."
    ))
}

/// Re-take the R4 identity check for one override: the recorded store triple
/// (MODE + PATH + BASE — never path alone, an implicit and an explicit row
/// over one path are different accounts) against the CURRENT resolution of
/// the command this launch would exec.
///
/// The lookup is controlled, not ambient: `HOME` is the launch home, every
/// other variable unset. An explicit client assigns its own variable, so its
/// resolution never consults the environment at all; an implicit one needs
/// exactly `HOME`. A CURRENT resolution of `Unknown` (a pane-only `$VAR` the
/// launcher cannot see) proves no conflict — `_run` retains the recorded
/// store with a notice, so the seat stays safe without this gate refusing —
/// but it proves no pairing either, and the `false` carries exactly that: the
/// caller honors the seat and records no new label fact. That mercy belongs
/// to a RESUME — a seat with a retained store to proceed on. A first start
/// (no recorded store) that resolves `Unknown` refuses outright instead: F6,
/// because proceeding would ignore an explicit flag and strand nothing.
fn refuse_store_conflict(
    session: &str,
    agent: &str,
    spelling: &str,
    entry: &meta::RosterEntry,
    command: &config::ResolvedCommand,
    tool: ToolKind,
    home: Option<&Path>,
) -> Result<bool, SeatOverrideRefusal> {
    if entry.config_home == crate::meta::RecordedConfigHome::Invalid {
        return Err(SeatOverrideRefusal::Failed(format!(
            "Error: cannot resume seat '{agent}' as '{spelling}': seat '{}' has malformed or duplicate config_home metadata — \
             fix the meta row, or end the session (ae end {session}).",
            entry.slot
        )));
    }
    if entry.config_home == crate::meta::RecordedConfigHome::Unknown {
        return Err(SeatOverrideRefusal::Failed(format!(
            "Error: cannot resume seat '{agent}' as '{spelling}': the recorded conversation store (config_home.{}) is unknown — \
             end the session and start over (ae end {session}).",
            entry.slot
        )));
    }
    let home_value = home.map(|path| path.display().to_string());
    let resolution = crate::launch_cmd::config_home_resolution(command, tool, &|name| {
        if name == "HOME" {
            home_value.clone()
        } else {
            None
        }
    });
    if entry.config_home == crate::meta::RecordedConfigHome::Missing {
        // First start for this seat: the override participates in the one
        // resolution and is recorded normally. But an `Unknown` resolution
        // refuses — F6: nothing retained means nothing stranded, and
        // proceeding would silently ignore the explicit flag.
        if let crate::launch_cmd::Resolved::Unknown(reason) = &resolution.home {
            return Err(unknown_store_refusal(agent, spelling, reason));
        }
        return Ok(true);
    }
    let current =
        crate::run::canonical_config_home(&resolution.home).map_err(SeatOverrideRefusal::Failed)?;
    let current_base =
        crate::run::canonical_config_home(&resolution.base).map_err(SeatOverrideRefusal::Failed)?;
    if matches!(current, crate::launch_cmd::Resolved::Unknown(_)) {
        return Ok(false);
    }
    let matches = match (&entry.config_home, &current) {
        (crate::meta::RecordedConfigHome::Absent, crate::launch_cmd::Resolved::Absent) => true,
        (
            crate::meta::RecordedConfigHome::Path(recorded),
            crate::launch_cmd::Resolved::Path(now),
        ) => resolution.explicit && recorded == now,
        (
            crate::meta::RecordedConfigHome::Implicit(recorded),
            crate::launch_cmd::Resolved::Path(now),
        ) => {
            !resolution.explicit
                && recorded == now
                && matches!(
                    (&entry.config_home_base, &current_base),
                    (
                        crate::meta::RecordedConfigHomeBase::Path(recorded),
                        crate::launch_cmd::Resolved::Path(now)
                    ) if recorded == now
                )
        }
        _ => false,
    };
    if matches {
        return Ok(true);
    }
    let now = match (&current, resolution.explicit) {
        (crate::launch_cmd::Resolved::Path(path), true) => {
            format!("{} (explicit)", path.display())
        }
        (crate::launch_cmd::Resolved::Path(path), false) => match &current_base {
            crate::launch_cmd::Resolved::Path(base) => {
                format!("{} (implicit, base {})", path.display(), base.display())
            }
            _ => format!("{} (implicit)", path.display()),
        },
        (crate::launch_cmd::Resolved::Absent, _) => "absent".to_owned(),
        (crate::launch_cmd::Resolved::Unknown(_), _) => "unresolvable".to_owned(),
    };
    // A store the pane selects through an exported account variable reads
    // exactly like a mode flip here: the launcher cannot see pane-only
    // environment, so the refusal names the escape — pinning the store in the
    // row makes the resolution observable and the refusal exact again.
    let exported_hint = if !resolution.explicit
        && matches!(entry.config_home, crate::meta::RecordedConfigHome::Path(_))
    {
        " If this seat's store is selected by an exported account variable the launcher cannot see, \
         pin it in the [clients] row as config_home=<path> instead."
    } else {
        ""
    };
    Err(SeatOverrideRefusal::Usage(format!(
        "Error: cannot resume seat '{agent}' as '{spelling}': the recorded conversation lives in {}, \
         but this launch resolves to {now} — a client override cannot move a retained conversation. \
         End the session to adopt the new account (ae end {session}).{exported_hint}",
        recorded_store_shown(&entry.config_home, &entry.config_home_base)
    )))
}

/// The preflight snapshot one `--seat` value resolves against.
struct OverrideCtx<'a> {
    cfg: &'a IdentityConfig,
    agents: &'a [String],
    known_agents: &'a str,
    known_profiles: &'a str,
    known_clients: &'a str,
    recorded: Option<&'a Meta>,
    session: &'a str,
    home: Option<&'a Path>,
}

/// Resolve one `--seat <agent>=<profile[@client]>` value: the agent must be a
/// launch seat, the selection must resolve (with the R1 gate for `@`), and a
/// recorded seat must honor it (write-once client, re-taken store, same tool).
fn resolve_one_override(
    ctx: &OverrideCtx,
    agent: &str,
    value: &str,
) -> Result<ResolvedSeatOverride, SeatOverrideRefusal> {
    if !ctx.agents.iter().any(|known| known == agent) {
        return Err(SeatOverrideRefusal::Usage(format!(
            "Error: unknown launch agent '{agent}' in --seat. Known agents: {}.",
            ctx.known_agents
        )));
    }
    let (profile, client) = split_seat_override(value).map_err(SeatOverrideRefusal::Usage)?;
    let (command, parsed) = if let Some(label) = client.as_deref() {
        resolve_client_override(
            ctx.cfg,
            agent,
            &profile,
            label,
            ctx.home,
            ctx.known_profiles,
            ctx.known_clients,
        )?
    } else {
        let command = ctx
            .cfg
            .command(&profile, ctx.home)
            .map_err(|why| SeatOverrideRefusal::Failed(why.to_string()))?;
        let Some(command) = command else {
            return Err(SeatOverrideRefusal::Usage(format!(
                "Error: unknown profile '{profile}' in --seat. Known profiles: {}.",
                ctx.known_profiles
            )));
        };
        let parsed = crate::launch_cmd::lex_simple_command(command.as_str()).map_err(|why| {
            SeatOverrideRefusal::Failed(format!(
                "Error: [profiles] {profile} (seat '{agent}'): the launch command must be one simple command — it has {why}."
            ))
        })?;
        (command, parsed)
    };
    let store_proven = if let Some(parsed_meta) = ctx.recorded
        && let Some(entry) = parsed_meta
            .roster()
            .iter()
            .find(|entry| entry.name == agent)
    {
        let proven = refuse_recorded_client_conflict(
            ctx.session,
            agent,
            &profile,
            client.as_deref(),
            entry,
            ctx.cfg,
            &command,
            parsed.tool(),
            ctx.home,
        )?;
        let old = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or_default());
        let new = parsed.tool();
        if old != new {
            return Err(SeatOverrideRefusal::Usage(format!(
                "Error: cannot change agent '{agent}' from {} to {} on resume — its recorded conversation cannot cross tool kinds.",
                old.as_str(),
                new.as_str()
            )));
        }
        proven
    } else {
        // No recorded seat: nothing to compare. But a first start that
        // cannot resolve its override refuses — F6: nothing is stranded, and
        // proceeding would ignore an explicit flag. A bare profile keeps the
        // legacy path (nothing to record either way).
        match current_pairing_proof(&command, parsed.tool(), ctx.home) {
            None => true,
            Some(reason) => {
                if let Some(label) = client.as_deref() {
                    return Err(unknown_store_refusal(
                        agent,
                        &format!("{profile}@{label}"),
                        &reason,
                    ));
                }
                false
            }
        }
    };
    Ok(ResolvedSeatOverride {
        agent: agent.to_owned(),
        profile,
        client,
        command,
        parsed,
        store_proven,
    })
}

/// Validate every explicit seat profile before the launch's first write.
/// Comma list of known names for a refusal, or `<none>` when empty.
fn known_list(names: &[&str]) -> String {
    if names.is_empty() {
        "<none>".to_owned()
    } else {
        names.join(", ")
    }
}

/// Honor every recorded `client.<slot>` label a flagless resume resumes.
///
/// `ae s` with no seat flags is "resume as recorded": each carried label is
/// resolved through the SAME path an explicit `@` selection takes — the R1
/// adapter gate, then the R4 re-take against the recorded store — so a
/// hostile or moved label refuses here, before any write, exactly as if it
/// had been spelled. Seats an explicit flag names are skipped: the explicit
/// selection already judged them (a bare profile refuses, an exact label
/// re-takes), and judging twice would double-report.
fn synthesize_recorded_honors(
    ctx: &OverrideCtx,
    seat_profiles: &[(String, String)],
) -> Result<Vec<ResolvedSeatOverride>, SeatOverrideRefusal> {
    let Some(recorded) = ctx.recorded else {
        return Ok(Vec::new());
    };
    let mut honored = Vec::new();
    for entry in recorded.roster() {
        if entry.slot != "main" && !entry.slot.starts_with("worker.") {
            continue;
        }
        if !ctx.agents.iter().any(|known| known == &entry.name) {
            continue;
        }
        if seat_profiles.iter().any(|(agent, _)| agent == &entry.name) {
            continue;
        }
        // The usability gate above already refused every unusable row, so a
        // `Missing` here is a legacy seat to skip — anything else that is not
        // a provable label refuses with its own remedy, never silently.
        let RecordedSelection::Label(label) =
            recorded_selection(ctx.session, ctx.cfg, entry).map_err(SeatOverrideRefusal::Failed)?
        else {
            continue;
        };
        let Some(profile) = entry.profile.as_deref().filter(|name| !name.is_empty()) else {
            return Err(SeatOverrideRefusal::Failed(format!(
                "Error: session '{}' seat '{}' recorded client override '{label}' but no profile — \
                 fix the meta row, or end the session (ae end {}).",
                ctx.session, entry.name, ctx.session
            )));
        };
        let (command, parsed) = resolve_client_override(
            ctx.cfg,
            &entry.name,
            profile,
            &label,
            ctx.home,
            ctx.known_profiles,
            ctx.known_clients,
        )?;
        let proven = refuse_recorded_client_conflict(
            ctx.session,
            &entry.name,
            profile,
            Some(&label),
            entry,
            ctx.cfg,
            &command,
            parsed.tool(),
            ctx.home,
        )?;
        honored.push(ResolvedSeatOverride {
            agent: entry.name.clone(),
            profile: profile.to_owned(),
            client: Some(label),
            command,
            parsed,
            store_proven: proven,
        });
    }
    Ok(honored)
}

/// Whether a flagless resume carries labels it must honor: a `Label` row on a
/// launch seat. Shape alone decides, so a legacy resume still returns before
/// any config is read.
fn flagless_honor_needed(recorded: Option<&Meta>, resuming: bool, running: bool) -> bool {
    resuming
        && !running
        && recorded.is_some_and(|parsed| {
            parsed.roster().iter().any(|entry| {
                (entry.slot == "main" || entry.slot.starts_with("worker."))
                    && matches!(entry.client, crate::meta::RecordedClient::Label(_))
            })
        })
}

fn validate_seat_overrides(
    env: &Env,
    plan: &Plan,
    dir: &Path,
    resuming: bool,
    running: bool,
) -> Result<Option<SeatOverrideSnapshot>, SeatOverrideRefusal> {
    if let Some(line) = solo_resume_refusal(plan, dir, resuming) {
        return Err(SeatOverrideRefusal::Usage(line));
    }
    // A recorded `client.<slot>` row in an unusable SHAPE refuses here with
    // its own remedy — before the solo-freeze check below, whose generic
    // doubtful-roster refusal would otherwise claim every damaged row first.
    // Shape alone decides, so no config is needed; the config-absent leg
    // lives with the override checks further down. A running session takes
    // the running refusal instead, never this one.
    if resuming && !running {
        let session = plan.name.as_deref().unwrap_or_default();
        let refusal = meta::read_bytes(dir)
            .ok()
            .map(|bytes| Meta::parse(&String::from_utf8_lossy(&bytes)))
            .as_ref()
            .and_then(|parsed| {
                parsed
                    .roster()
                    .iter()
                    .find_map(|entry| match &entry.client {
                        crate::meta::RecordedClient::Invalid => {
                            Some(invalid_client_refusal(session, &entry.name, &entry.slot))
                        }
                        _ => None,
                    })
            });
        if let Some(line) = refusal {
            return Err(SeatOverrideRefusal::Failed(line));
        }
    }
    let frozen_solo_identity = frozen_solo_identity(plan, dir, resuming, running)?;
    let recorded = if resuming {
        meta::read_bytes(dir)
            .ok()
            .map(|bytes| Meta::parse(&String::from_utf8_lossy(&bytes)))
    } else {
        None
    };
    if plan.seat_profiles.is_empty()
        && frozen_solo_identity.is_none()
        && !flagless_honor_needed(recorded.as_ref(), resuming, running)
    {
        return Ok(None);
    }
    if running && !plan.seat_profiles.is_empty() {
        let session = plan.name.as_deref().unwrap_or_default();
        return Err(SeatOverrideRefusal::Usage(running_override_refusal(
            session,
        )));
    }

    let mut cfg = override_identity(env, dir, resuming)?;
    if let Some((recorded_main, recorded_profile)) = frozen_solo_identity {
        freeze_solo_config(&mut cfg, plan, &recorded_main, &recorded_profile)?;
    }
    if let Some(line) = solo_worker_override_refusal(plan, &cfg) {
        return Err(SeatOverrideRefusal::Usage(line));
    }
    let agents = override_agents(&cfg, plan, dir, resuming);
    let agent_names: Vec<&str> = agents.iter().map(String::as_str).collect();
    let known_agents = known_list(&agent_names);
    let profile_names: Vec<&str> = cfg
        .profiles
        .iter()
        .map(|(profile, _)| profile.as_str())
        .collect();
    let known_profiles = known_list(&profile_names);
    let client_names: Vec<&str> = cfg
        .clients
        .iter()
        .map(|(client, _)| client.as_str())
        .collect();
    let known_clients = known_list(&client_names);
    let session = plan.name.as_deref().unwrap_or_default();
    // Every recorded seat's client row must be usable before any rewrite,
    // whether or not this launch names that seat: the meta rewrite below
    // would otherwise launder an Invalid row into a carried Label or drop it
    // toward Missing. A launch without overrides takes the same gate past the
    // lock, beside the doubtful-roster refusal.
    if let Some(parsed_meta) = recorded.as_ref() {
        for entry in parsed_meta.roster() {
            recorded_selection(session, &cfg, entry).map_err(SeatOverrideRefusal::Failed)?;
        }
    }

    let mut overrides = Vec::with_capacity(plan.seat_profiles.len());
    let home = crate::doors::home();
    let ctx = OverrideCtx {
        cfg: &cfg,
        agents: &agents,
        known_agents: &known_agents,
        known_profiles: &known_profiles,
        known_clients: &known_clients,
        recorded: recorded.as_ref(),
        session,
        home: home.as_deref(),
    };
    for (agent, value) in &plan.seat_profiles {
        overrides.push(resolve_one_override(&ctx, agent, value)?);
    }
    // Flagless seats last: the explicit loop above already judged every seat
    // a flag names, so these are exactly the carried labels the flags left
    // alone — honored through the same resolve + re-take.
    overrides.extend(synthesize_recorded_honors(&ctx, &plan.seat_profiles)?);
    for replacement in &overrides {
        if let Some((_, bound)) = cfg
            .roster
            .iter_mut()
            .find(|(name, _)| name == &replacement.agent)
        {
            bound.clone_from(&replacement.profile);
        }
    }
    Ok(Some(SeatOverrideSnapshot { cfg, overrides }))
}

/// Replace one resolved seat's profile and every command-derived field.
fn reprofile_seat(seat: &mut Seat, replacement: &ResolvedSeatOverride) {
    replacement.profile.clone_into(&mut seat.profile);
    seat.client_override.clone_from(&replacement.client);
    seat.store_proven = replacement.store_proven;
    replacement.command.clone_into(&mut seat.command);
    seat.assign_span.clone_from(&replacement.parsed.assign_span);
    seat.argv_span.clone_from(&replacement.parsed.argv_span);
    seat.binary.clone_from(&replacement.parsed.binary);
    seat.tool = replacement.parsed.tool();
}

/// Apply already-preflighted overrides to final, possibly restored seats.
fn apply_seat_overrides(snapshot: &SeatOverrideSnapshot, seats: &mut [Seat]) -> Result<(), String> {
    for replacement in &snapshot.overrides {
        let Some(seat) = seats.iter_mut().find(|seat| seat.name == replacement.agent) else {
            return Err(format!(
                "Error: launch agent '{}' disappeared after preflight.",
                replacement.agent
            ));
        };
        reprofile_seat(seat, replacement);
    }
    Ok(())
}

/// A session's resolved shape, once every refusal has passed.
struct Session {
    name: String,
    mode: Mode,
    work_dir: PathBuf,
    origin: PathBuf,
    layout: String,
    /// What this session is drawn in, and whether it is drawn at all —
    /// `[workspace]`'s `palette`, `icons`, `theme` and `motion`.
    look: crate::theme::Look,
    resuming: bool,
    /// Whether THIS attempt created the session directory — the ownership fact
    /// the rollback keys on, recorded at the `mkdir` and never derived from
    /// `resuming`.
    dir_created: bool,
}

#[allow(
    clippy::too_many_lines,
    reason = "the frozen order, kept in one place — see the module docs"
)]
fn launch(
    env: &Env,
    plan: &Plan,
    expected_proof: Option<&str>,
    expected_launch: Option<&ExpectedLaunch>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let cwd = env.cwd.display().to_string();
    let sessions = env.sessions();
    let worktrees = env.worktrees();

    // ---- the name, and the guards that must precede every side effect ----
    let Some(session) = plan.name.clone() else {
        writeln!(err, "{MISSING_NAME}")?;
        writeln!(err, "{PUBLIC_USAGE}")?;
        return Ok(EXIT_USAGE);
    };
    if !name::is_session_name(&session) {
        writeln!(
            err,
            "Error: invalid session name '{session}'. Names must match {}.",
            name::SESSION_NAME_GRAMMAR
        )?;
        return Ok(EXIT_FAILED);
    }
    let dir = sessions.join(&session);
    let work_root = worktrees.join(&session);

    // A pending rename transaction owns this name until its proved retry
    // converges: a launch must not read or mutate a mixed generation as
    // ordinary state.
    if let Some(blocked) = crate::rename::intent_blocks(&env.home, &session) {
        writeln!(err, "Error: {blocked}. Nothing was launched.")?;
        return Ok(EXIT_FAILED);
    }

    for (root, note) in [
        (&sessions, "the session state is parked under it"),
        (&worktrees, "a working copy is parked under it"),
    ] {
        let tombstone = root.join(format!(".ending.{session}"));
        if node_exists(&tombstone) {
            writeln!(
                err,
                "Error: a previous teardown of '{session}' did not complete."
            )?;
            writeln!(
                err,
                "       {} is standing — {note}, not under '{session}'.",
                tombstone.display()
            )?;
            writeln!(err, "       Inspect it, then remove it by hand and retry:")?;
            writeln!(err, "           rm -rf {}", tombstone.display())?;
            return Ok(EXIT_FAILED);
        }
    }

    let proposed_server = env.server();
    let settings_start = expected_launch.is_some_and(ExpectedLaunch::is_start);
    if settings_start && session != crate::orchestrator::ORCHESTRATOR_SESSION {
        writeln!(
            err,
            "Error: settings Start may create only the canonical orchestrator."
        )?;
        return Ok(EXIT_FAILED);
    }
    // Start may not inspect canonical saved state until it owns the canonical
    // lifecycle lock below. Resume may inspect only to select the recorded
    // server for the tmux floor; absence makes the captured action stale and
    // is refused before a lock-file or any other effect.
    let preflight_meta_present = !settings_start && node_exists(&dir.join(crate::store::META));
    if expected_launch.is_some_and(ExpectedLaunch::is_resume) && !preflight_meta_present {
        writeln!(
            err,
            "Error: role target '{session}' disappeared before Resume acquired its lifecycle lock. Nothing was resumed."
        )?;
        return Ok(EXIT_FAILED);
    }

    // ---- THE tmux FLOOR, before the first thing this launch would write ----
    // AHEAD of the lifecycle lock, migration chain and config seed. A resume
    // first takes a read-only look at its recorded server so the floor is asked
    // of the server this attempt will actually use; the decision is repeated
    // under the lifecycle lock below before anything acts on it.
    let floor_server = if preflight_meta_present {
        let Ok(bytes) = meta::read_bytes(&dir) else {
            writeln!(err, "Error: could not read metadata for '{session}'.")?;
            return Ok(EXIT_FAILED);
        };
        match Meta::parse(&String::from_utf8_lossy(&bytes)).server_selector() {
            ServerSelector::Positive(selector) => {
                let recorded = ServerId::Selected(selector);
                match resume_absence(&recorded, &session, &dir) {
                    (tmux::StopProbe::Absent, _)
                        if expected_launch.is_some_and(ExpectedLaunch::is_resume) =>
                    {
                        recorded
                    }
                    (tmux::StopProbe::Absent, _) => proposed_server.clone(),
                    (tmux::StopProbe::Present, _) => recorded,
                    (tmux::StopProbe::Unknown, why) => {
                        writeln!(
                            err,
                            "Error: cannot verify whether tmux session '{session}' is absent on its recorded server. Metadata was not changed."
                        )?;
                        if let Some(why) = why {
                            writeln!(err, "       {why}")?;
                        }
                        return Ok(EXIT_FAILED);
                    }
                }
            }
            ServerSelector::Missing => {
                let historical = crate::doors::historical_server();
                match resume_absence(&historical, &session, &dir) {
                    (tmux::StopProbe::Absent, _) => proposed_server.clone(),
                    (tmux::StopProbe::Present, _) => historical,
                    (tmux::StopProbe::Unknown, why) => {
                        writeln!(
                            err,
                            "Error: cannot verify whether legacy tmux session '{session}' is absent on the historical default server. Metadata was not changed."
                        )?;
                        if let Some(why) = why {
                            writeln!(err, "       {why}")?;
                        }
                        return Ok(EXIT_FAILED);
                    }
                }
            }
            ServerSelector::Ambiguous => {
                writeln!(err, "Error: '{session}' {AMBIGUOUS_SERVER}.")?;
                return Ok(EXIT_FAILED);
            }
        }
    } else {
        proposed_server.clone()
    };
    let running_preflight =
        preflight_meta_present && transport::session_exists(&floor_server, &session);
    if let Some(refusal) = floor_refusal(&floor_server, &session) {
        write!(err, "{refusal}")?;
        return Ok(crate::tmux_floor::EXIT_REFUSED);
    }
    let seat_overrides =
        match validate_seat_overrides(env, plan, &dir, preflight_meta_present, running_preflight) {
            Ok(snapshot) => snapshot,
            Err(SeatOverrideRefusal::Usage(line)) => {
                writeln!(err, "{line}")?;
                return Ok(EXIT_USAGE);
            }
            Err(SeatOverrideRefusal::Failed(line)) => {
                writeln!(err, "{line}")?;
                return Ok(EXIT_FAILED);
            }
        };

    #[cfg(debug_assertions)]
    if let Some(marker) = &env.test_pre_lock_marker {
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(env.home.join(marker));
    }

    // Settings Start owns the canonical name lock before its raw-role census
    // or canonical saved/live absence proof. Expected Resume owns the exact
    // captured target lock even though preflight saw its state. The read-only
    // preflight above exists only to place the tmux floor before this lock-file
    // write; every expected fact is re-read while the lock is held.
    let mut lifecycle = if settings_start
        || expected_launch.is_some_and(ExpectedLaunch::is_resume)
        || preflight_meta_present
    {
        if let Ok(held) = crate::store::lock(
            &sessions.join(format!(".lifecycle.{session}.lock")),
            LIFECYCLE_WAIT,
        ) {
            Some(held)
        } else {
            let detail = if settings_start {
                " — stale Start refused"
            } else {
                " — retry shortly"
            };
            writeln!(
                err,
                "Error: another lifecycle operation is in progress for '{session}'{detail}."
            )?;
            return Ok(EXIT_FAILED);
        }
    } else {
        None
    };
    let meta_present = if expected_launch.is_some() {
        node_exists(&dir.join(crate::store::META))
    } else {
        preflight_meta_present
    };

    if let Some(expected) = expected_launch {
        if let Err(why) = expected.check_action(crate::time::Timestamp::now().epoch()) {
            writeln!(err, "Error: {why}; menu action refused.")?;
            return Ok(EXIT_FAILED);
        }
        match &expected.state {
            ExpectedState::AbsentCanonical => {
                if let Err(why) =
                    crate::settings_menu::prove_absent_start(&env.home, &proposed_server)
                {
                    writeln!(err, "Error: {why}. Nothing was started.")?;
                    return Ok(EXIT_FAILED);
                }
            }
            ExpectedState::StoppedRole { uuid } => {
                match crate::settings_menu::prove_stopped_role(&env.home, &session, uuid) {
                    Ok(_) => {}
                    Err(why) => {
                        writeln!(err, "Error: {why}. Nothing was resumed.")?;
                        return Ok(EXIT_FAILED);
                    }
                }
            }
            // The picker's row: the session's own record and its absence on the
            // recorded server are re-read below, under the lifecycle lock, by
            // the same code every resume crosses. Nothing extra to prove here.
            ExpectedState::StoppedSession => {}
        }
    }

    let mut env = env.clone();
    let mut running_server = None;
    if meta_present {
        let Ok(bytes) = meta::read_bytes(&dir) else {
            writeln!(err, "Error: could not read metadata for '{session}'.")?;
            return Ok(EXIT_FAILED);
        };
        // An explicit origin is a claim about this exact session, including a
        // live one. It must agree with the recorded owner before reattach can
        // migrate, seed, or backfill anything below.
        if plan.dir.is_some() {
            let recorded = meta::first_value(&bytes, "origin")
                .map(|value| String::from_utf8_lossy(value).into_owned())
                .unwrap_or_default();
            if recorded.is_empty() || !same_directory(&recorded, &cwd) {
                writeln!(
                    err,
                    "Error: session '{session}' exists with a different origin."
                )?;
                writeln!(
                    err,
                    "       recorded: {}",
                    if recorded.is_empty() {
                        "<none>"
                    } else {
                        &recorded
                    }
                )?;
                writeln!(err, "       requested: {cwd}")?;
                return Ok(EXIT_FAILED);
            }
        }
        match Meta::parse(&String::from_utf8_lossy(&bytes)).server_selector() {
            ServerSelector::Positive(selector) => {
                let recorded = ServerId::Selected(selector);
                match resume_absence(&recorded, &session, &dir) {
                    (tmux::StopProbe::Present, _) => {
                        if !plan.seat_profiles.is_empty() {
                            writeln!(err, "{}", running_override_refusal(&session))?;
                            return Ok(EXIT_USAGE);
                        }
                        set_env_server(&mut env, &recorded);
                        running_server = Some(recorded);
                    }
                    (tmux::StopProbe::Absent, _) => {
                        let destination = if expected_launch.is_some_and(ExpectedLaunch::is_resume)
                        {
                            &recorded
                        } else {
                            &proposed_server
                        };
                        if !destination_is_absent(destination, &session, err)? {
                            return Ok(EXIT_FAILED);
                        }
                        if expected_launch.is_some_and(ExpectedLaunch::is_resume) {
                            set_env_server(&mut env, &recorded);
                        }
                    }
                    (tmux::StopProbe::Unknown, why) => {
                        writeln!(
                            err,
                            "Error: cannot verify whether tmux session '{session}' is absent on its recorded server. Metadata was not changed."
                        )?;
                        if let Some(why) = why {
                            writeln!(err, "       {why}")?;
                        }
                        return Ok(EXIT_FAILED);
                    }
                }
            }
            ServerSelector::Missing => {
                let historical = crate::doors::historical_server();
                match resume_absence(&historical, &session, &dir) {
                    (tmux::StopProbe::Present, _) => {
                        let ownership = transport::observe_session_ownership(&historical, &session);
                        let main_pane = meta_value(&dir, "main_pane").unwrap_or_default();
                        let pane_belongs = !main_pane.is_empty()
                            && transport::observe_agents(&historical, &session).is_some_and(
                                |panes| panes.into_iter().any(|pane| pane.pane == main_pane),
                            );
                        let owned = ownership.is_some_and(|ownership| {
                            !ownership.marker.is_empty()
                                && existing_directories_match(Path::new(&ownership.home), &env.home)
                        }) && pane_belongs;
                        let socket = owned
                            .then(|| transport::observe_socket_path(&historical))
                            .flatten();
                        let Some(socket) = socket else {
                            writeln!(
                                err,
                                "Error: tmux session '{session}' exists on the historical default server, but ae could not prove it belongs to this state root. Metadata was not changed."
                            )?;
                            return Ok(EXIT_FAILED);
                        };
                        if !plan.seat_profiles.is_empty() {
                            writeln!(err, "{}", running_override_refusal(&session))?;
                            return Ok(EXIT_USAGE);
                        }
                        if let Err(why) = meta::backfill_server_socket(&dir, &socket) {
                            writeln!(
                                err,
                                "Error: could not backfill the tmux server for '{session}' ({}).",
                                why.cause()
                            )?;
                            return Ok(EXIT_FAILED);
                        }
                        let recorded = ServerId::Selected(meta::Selector::Socket(socket.into()));
                        set_env_server(&mut env, &recorded);
                        running_server = Some(recorded);
                    }
                    (tmux::StopProbe::Absent, _) => {
                        if !destination_is_absent(&proposed_server, &session, err)? {
                            return Ok(EXIT_FAILED);
                        }
                    }
                    (tmux::StopProbe::Unknown, why) => {
                        writeln!(
                            err,
                            "Error: cannot verify whether legacy tmux session '{session}' is absent on the historical default server. Metadata was not changed."
                        )?;
                        if let Some(why) = why {
                            writeln!(err, "       {why}")?;
                        }
                        return Ok(EXIT_FAILED);
                    }
                }
            }
            ServerSelector::Ambiguous => {
                writeln!(err, "Error: '{session}' {AMBIGUOUS_SERVER}.")?;
                return Ok(EXIT_FAILED);
            }
        }
    } else if transport::session_exists(&proposed_server, &session) {
        writeln!(
            err,
            "Error: tmux session '{session}' exists but is not an ae session."
        )?;
        return Ok(EXIT_FAILED);
    }

    let orchestrator_config = env.home.join(crate::orchestrator::CONFIG_FILE);
    let seeds_orchestrator_config = session == crate::orchestrator::ORCHESTRATOR_SESSION
        && env.local.as_deref() == Some(orchestrator_config.as_path());

    // The FIRST write a launch makes, and it happens here rather than on the
    // public path so that nothing at all can land between the floor decision
    // and it. One gate, one decision, and every write below it.
    if let Some(global) = env.global.as_ref()
        && let Some(code) =
            crate::seed_default_config(global, crate::entry::DEFAULT_CONFIG, "default", err)?
    {
        return Ok(code);
    }
    // THE CHAIN, before anything reads a field of this meta: a resume or a
    // reattach is ae touching a session, and a shape it cannot place is one it
    // must not act on.
    if meta_present && let Err(refusal) = crate::migrate::session(&dir) {
        writeln!(err, "Error: {}", refusal.line(&session))?;
        return Ok(EXIT_FAILED);
    }
    // On resume the RECORDED config wins: an agent's aliases must resolve from
    // the file the session was created with, not from wherever the caller is.
    if meta_present {
        if let Some(stored) = meta_value(&dir, "config").filter(|v| !v.is_empty()) {
            env.global = Some(PathBuf::from(stored));
        }
        let origin = meta_value(&dir, "origin").unwrap_or_default();
        env.local = config::local_overlay(&dir, &origin);
    }
    let orchestrator_seat = env
        .local
        .as_deref()
        .is_some_and(|local| crate::orchestrator::is_seat_overlay(local, &env.home));
    if orchestrator_seat && seat_overrides.is_none() {
        let (_, has_profiles, has_roster) =
            config::read_workspace_keys_with_identity_sections(None, env.local.as_deref(), &[]);
        if (has_profiles || has_roster)
            && let (Some(local), Some(global)) = (env.local.as_deref(), env.global.as_deref())
        {
            writeln!(
                err,
                "ae orchestrator: ignoring [roster]/[profiles] in {}: the seat profile is [roster] orchestrator in {}",
                local.display(),
                global.display()
            )?;
        }
    }
    // ---- explicit lineage, proved before anything is created ----
    let mut parent: Option<FromProof> = None;
    if let Some(uuid) = &plan.from {
        let root = env.home.join("archive");
        match from_preflight(&root, uuid) {
            Ok(proof) => {
                if transport::session_exists(&proposed_server, &session)
                    || dir_exists(&dir)
                    || dir_exists(&work_root)
                {
                    writeln!(
                        err,
                        "Error: --from is only valid for a NEW session, and '{session}' already exists."
                    )?;
                    writeln!(
                        err,
                        "       Inheriting an archive into a running or resumable session would mean two"
                    )?;
                    writeln!(
                        err,
                        "       different things (replace its history? merge it?) with no safe default."
                    )?;
                    writeln!(err, "       Start the continuation under its own name:")?;
                    writeln!(err, "           ae <new-name> --from {}", proof.id)?;
                    return Ok(EXIT_FAILED);
                }
                // THE SECOND PROOF. compact took the first at its boundary;
                // between the two the archive could have been purged, claimed
                // or corrupted, and a child published with a lineage pointer to
                // something that is no longer there is exactly the case
                // `--from` exists to refuse.
                let now = format!("{}\t{}\t{}", proof.id, proof.handover, proof.pending);
                if let Some(expected) = expected_proof
                    && now != expected
                {
                    writeln!(
                        err,
                        "Error: parent archive {} changed while this session was being created.",
                        proof.id
                    )?;
                    writeln!(err, "  Expected: {}", expected.replace('\t', " | "))?;
                    writeln!(err, "  Now:      {}", now.replace('\t', " | "))?;
                    return Ok(EXIT_FAILED);
                }
                parent = Some(proof);
            }
            Err(line) => {
                writeln!(err, "{line}")?;
                return Ok(EXIT_FAILED);
            }
        }
    }

    // ---- a session that is already running is reattached, never rebuilt ----
    if let Some(server) = running_server {
        let Some(observed) = transport::observe_agents(&server, &session) else {
            writeln!(
                err,
                "Error: tmux session '{session}' exists but is not an ae session."
            )?;
            return Ok(EXIT_FAILED);
        };
        if let Some(core) = env.core.clone().or_else(crate::shape::resolved_exe) {
            assert_status_bindings(&server, &env, &core);
        }
        let layout = meta_value(&dir, "layout").unwrap_or_default();
        let main_pane = meta_value(&dir, "main_pane").unwrap_or_default();
        let pane_belongs = observed.iter().any(|pane| pane.pane == main_pane);
        stamp_main_pane(&server, &session, &main_pane, pane_belongs);
        if pane_belongs {
            stamp_lead_pair_policy(&server, &layout, &main_pane);
        }
        // The guard protects only the resume decision and its verification.
        // Attaching may block for the client's whole lifetime; holding the
        // guard across it would make stop/end refuse while the user is there.
        drop(lifecycle.take());
        crate::autoupgrade::schedule();
        if !env.attach {
            writeln!(
                out,
                "Session '{session}' is running. Attach with: {}",
                attach_hint(&server, &session)
            )?;
            return Ok(0);
        }
        return Ok(attach(&server, &env, &session, out)?);
    }

    if seeds_orchestrator_config && let Some(global) = env.global.as_deref() {
        // The global roster chooses the seat's profile. Check it after the
        // global first-run seed, but only after the live-session reattach
        // branch: a running seat is never rebuilt or revalidated.
        let cfg = match config::read_identity(Some(global), None) {
            Ok(cfg) => cfg,
            Err(why) => {
                writeln!(err, "{why}")?;
                return Ok(EXIT_FAILED);
            }
        };
        if cfg
            .roster_profile(crate::orchestrator::ORCHESTRATOR_SESSION)
            .is_none()
        {
            writeln!(
                err,
                "ae orchestrator: no profile for the seat — add \"orchestrator = <profile>\" under [roster] in {}",
                global.display()
            )?;
            return Ok(EXIT_FAILED);
        }
    }
    if seeds_orchestrator_config
        && let Some(code) = crate::seed_default_config(
            &orchestrator_config,
            crate::orchestrator::DEFAULT_CONFIG,
            "orchestrator",
            err,
        )?
    {
        return Ok(code);
    }

    // ---- config, roster, workspace values ----
    let mut cfg: IdentityConfig = if let Some(snapshot) = seat_overrides.as_ref() {
        snapshot.cfg.clone()
    } else {
        match if orchestrator_seat {
            config::read_identity(env.global.as_deref(), None)
        } else {
            config::read_identity(env.global.as_deref(), env.local.as_deref())
        } {
            Ok(cfg) => cfg,
            Err(why) => {
                writeln!(err, "{why}")?;
                return Ok(EXIT_FAILED);
            }
        }
    };
    if orchestrator_seat && seat_overrides.is_none() {
        // Identity is global-only, while the dedicated seat still overlays its
        // workspace choices. Read those two keys without parsing legacy local
        // profiles or roster rows.
        let local_workspace =
            config::read_workspace_keys(None, env.local.as_deref(), &["main", "workers"]);
        if local_workspace[0].is_some() {
            cfg.main.clone_from(&local_workspace[0]);
        }
        if local_workspace[1].is_some() {
            cfg.workers.clone_from(&local_workspace[1]);
        }
    }
    let extras = config::read_workspace_keys(
        env.global.as_deref(),
        env.local.as_deref(),
        &[
            "layout",
            "copy",
            "watchdog",
            "loop",
            "orchestrator",
            "hub",
            "meta",
            "palette",
            "icons",
            "theme",
            "motion",
            "sweep",
            "quota",
        ],
    );
    let config_layout = extras[0].clone().unwrap_or_default();
    let config_copy = extras[1].clone().unwrap_or_default();
    let look = crate::theme::Look::read(
        &extras[8].clone().unwrap_or_default(),
        &extras[7].clone().unwrap_or_default(),
        &extras[9].clone().unwrap_or_default(),
        &extras[10].clone().unwrap_or_default(),
    );
    let meta_agent = ["true", "1", "yes", "on"].contains(
        &extras[4]
            .clone()
            .or_else(|| extras[5].clone())
            .or_else(|| extras[6].clone())
            .unwrap_or_default()
            .as_str(),
    );
    let sweep_sec = extras[11]
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.to_string());
    let recorded_quota_every_secs = if meta_present {
        meta_value(&dir, "quota_every_secs")
    } else {
        None
    };
    let configured_quota_every_secs = if recorded_quota_every_secs.is_none() {
        match configured_quota_every_secs(env.global.as_deref(), env.local.as_deref()) {
            Ok(value) => value,
            Err(value) => {
                writeln!(
                    err,
                    "Error: [workspace] quota_every_secs must be an unsigned integer in seconds; got '{value}'."
                )?;
                return Ok(EXIT_USAGE);
            }
        }
    } else {
        None
    };
    let quota_every_secs = recorded_quota_every_secs
        .or(configured_quota_every_secs)
        .unwrap_or_else(|| {
            crate::watchdog_daemon::Knobs::default()
                .quota_every_secs
                .to_string()
        });
    if quota_every_secs.parse::<u64>().is_err() {
        writeln!(
            err,
            "Error: [workspace] quota_every_secs must be an unsigned integer in seconds; got '{quota_every_secs}'."
        )?;
        return Ok(EXIT_USAGE);
    }
    let recorded_idle_nudge_secs = if meta_present {
        meta_value(&dir, "idle_nudge_secs")
    } else {
        None
    };
    let configured_idle_nudge_secs = if recorded_idle_nudge_secs.is_none() {
        match configured_unsigned_workspace_seconds(
            env.global.as_deref(),
            env.local.as_deref(),
            "idle_nudge_secs",
        ) {
            Ok(value) => value,
            Err(value) => {
                writeln!(
                    err,
                    "Error: [workspace] idle_nudge_secs must be an unsigned integer in seconds; got '{value}'."
                )?;
                return Ok(EXIT_USAGE);
            }
        }
    } else {
        None
    };
    let idle_nudge_secs = recorded_idle_nudge_secs
        .or(configured_idle_nudge_secs)
        .unwrap_or_else(|| {
            crate::watchdog_daemon::Knobs::default()
                .idle_nudge_secs
                .to_string()
        });
    if idle_nudge_secs.parse::<u64>().is_err() {
        writeln!(
            err,
            "Error: [workspace] idle_nudge_secs must be an unsigned integer in seconds; got '{idle_nudge_secs}'."
        )?;
        return Ok(EXIT_USAGE);
    }
    // `quota = off` is pinned like `quota_every_secs`: the recorded meta wins
    // on resume so a config flip applies to NEW sessions only. Absent means ON
    // — exactly today's behaviour — through the ONE `config::quota_aware`
    // predicate. `quota = off` WINS over `quota_every_secs`: a cadence is
    // meaningless when the feature is off, and the watchdog enforces that.
    let recorded_quota = if meta_present {
        meta_value(&dir, "quota")
    } else {
        None
    };
    let configured_quota = if recorded_quota.is_none() {
        extras[12].clone().unwrap_or_default()
    } else {
        String::new()
    };
    let quota_raw = recorded_quota.unwrap_or(configured_quota);
    let quota = if crate::config::quota_aware(&quota_raw) {
        "on"
    } else {
        "off"
    }
    .to_owned();

    if let Some(workers) = &plan.workers {
        cfg.workers = Some(workers.clone());
    }
    let home = crate::doors::home();
    let mut seats = match config::launch_plan(&cfg, plan.main.as_deref(), home.as_deref()) {
        Ok(resolved) => resolved.seats,
        Err(violations) => {
            write!(err, "{}", config::render_violations(&violations))?;
            return Ok(EXIT_FAILED);
        }
    };

    // ---- the mode: flag, then the recorded one, then config, then local ----
    let mut mode = plan
        .mode
        .unwrap_or_else(|| Mode::parse(&config_copy).unwrap_or(Mode::Local));
    if plan.mode.is_none()
        && meta_present
        && let Some(stored) = meta_value(&dir, "mode").as_deref().and_then(Mode::parse)
    {
        mode = stored;
    }

    // ---- the working copy ----
    let mut origin = env.cwd.clone();
    let mut work_dir = work_root.clone();
    // The session document is the ONE resume discriminator. A worktree path is
    // not identity: rename deliberately leaves it under the old name, and an
    // unrelated directory under the new name must never turn a fresh launch
    // into a resume.
    let resuming = meta_present;
    if resuming {
        let parsed = meta::read_bytes(&dir)
            .ok()
            .map(|bytes| Meta::parse(&String::from_utf8_lossy(&bytes)));
        // The recorded-client gate for launches WITHOUT overrides (an
        // override launch took it in preflight): an unusable row refuses with
        // its own remedy before the generic doubtful-roster refusal below,
        // which stays as the backstop for every other anomaly.
        if let Some(refusal) = parsed.as_ref().and_then(|meta| {
            meta.roster()
                .iter()
                .find_map(|entry| recorded_selection(&session, &cfg, entry).err())
        }) {
            writeln!(err, "{refusal}")?;
            return Ok(EXIT_FAILED);
        }
        if let Some(anomaly) = parsed.as_ref().and_then(|meta| {
            meta.anomalies()
                .iter()
                .find(|item| roster::roster_doubting(item))
        }) {
            writeln!(
                err,
                "Error: session '{session}' has doubtful roster metadata ({anomaly}); run 'ae doctor' before resuming."
            )?;
            return Ok(EXIT_FAILED);
        }
    }
    if mode == Mode::Local {
        if dir_exists(&work_root) {
            writeln!(
                err,
                "Error: stopped session '{session}' exists as worktree/copy. End it first (ae end {session}), or use a different name."
            )?;
            return Ok(EXIT_FAILED);
        }
        work_dir.clone_from(&env.cwd);
        if resuming {
            if let Some(stored) = meta_value(&dir, "work_dir").filter(|v| dir_exists(Path::new(v)))
            {
                work_dir = PathBuf::from(stored);
            }
            if let Some(stored) = meta_value(&dir, "origin").filter(|v| !v.is_empty()) {
                origin = PathBuf::from(stored);
            }
            writeln!(
                out,
                "Resuming session {session} (dir: {})...",
                work_dir.display()
            )?;
        }
    } else if resuming {
        let stored_work = meta_value(&dir, "work_dir").filter(|value| !value.is_empty());
        if let Some(stored) = stored_work.as_deref() {
            if !dir_exists(Path::new(stored)) {
                writeln!(
                    err,
                    "Error: '{session}' records its working copy at {stored} but it is gone — restore it or end the session (ae end {session})"
                )?;
                return Ok(EXIT_FAILED);
            }
            work_dir = PathBuf::from(stored);
        } else if !dir_exists(&work_root) {
            writeln!(
                err,
                "Error: '{session}' records its working copy at {} but it is gone — restore it or end the session (ae end {session})",
                work_root.display()
            )?;
            return Ok(EXIT_FAILED);
        }
        if let Some(stored) = meta_value(&dir, "origin").filter(|v| !v.is_empty()) {
            origin = PathBuf::from(stored);
        }
        writeln!(
            out,
            "Resuming session {session} (dir: {})...",
            work_dir.display()
        )?;
    } else {
        if dir_exists(&work_root) {
            writeln!(
                err,
                "Error: working copy '{}' exists without session metadata; refusing to overwrite it.",
                work_root.display()
            )?;
            return Ok(EXIT_FAILED);
        }
        if let Err(why) = std::fs::create_dir_all(&work_root) {
            writeln!(
                err,
                "Error: could not create {} ({why})",
                work_root.display()
            )?;
            return Ok(EXIT_FAILED);
        }
        match mode {
            Mode::Git if crate::git::is_work_tree(path_bytes(&env.cwd)) => {
                writeln!(out, "Creating git worktree...")?;
                if !crate::git::worktree_add_detached(path_bytes(&env.cwd), path_bytes(&work_root))
                {
                    let _ = std::fs::remove_dir_all(&work_root);
                    writeln!(
                        err,
                        "Error: could not create a git worktree at {}",
                        work_root.display()
                    )?;
                    return Ok(EXIT_FAILED);
                }
            }
            Mode::Git => {
                writeln!(err, "Warning: not a git repo, falling back to full copy.")?;
                mode = Mode::Full;
                writeln!(out, "Creating full copy...")?;
                if let Err(why) = copy_tree(&env.cwd, &work_root) {
                    let _ = std::fs::remove_dir_all(&work_root);
                    writeln!(err, "Error: could not copy the working tree ({why})")?;
                    return Ok(EXIT_FAILED);
                }
            }
            Mode::Full => {
                writeln!(out, "Creating full copy...")?;
                if let Err(why) = copy_tree(&env.cwd, &work_root) {
                    let _ = std::fs::remove_dir_all(&work_root);
                    writeln!(err, "Error: could not copy the working tree ({why})")?;
                    return Ok(EXIT_FAILED);
                }
            }
            Mode::Local => unreachable!("local mode is handled above"),
        }
        writeln!(out, "Working copy ready.")?;
    }

    // Layout is META-WINS on resume: pinned at launch, so a config flip applies
    // to NEW sessions only, mirroring agent identity.
    let mut layout = if config_layout.is_empty() {
        "vertical".to_owned()
    } else {
        config_layout
    };
    // A RESUME RESTORES THE ROSTER IT SAVED.
    if resuming {
        for seat in &mut seats {
            if let Some(saved) =
                meta_value(&dir, &format!("seat.{}", seat.slot)).filter(|v| !v.is_empty())
            {
                seat.name = saved;
            }
            if let Some(saved) =
                meta_value(&dir, &format!("profile.{}", seat.slot)).filter(|v| !v.is_empty())
            {
                seat.profile = saved;
            }
        }
    }
    if let Some(snapshot) = seat_overrides.as_ref()
        && let Err(line) = apply_seat_overrides(snapshot, &mut seats)
    {
        writeln!(err, "{line}")?;
        return Ok(EXIT_FAILED);
    }
    if resuming && let Some(saved) = meta_value(&dir, "layout").filter(|v| !v.is_empty()) {
        layout = saved;
    }

    // ---- restored spawned seats, LEXED before anything is created ----
    //
    // A spawned seat records a PROFILE NAME, and the resume looks that name up
    // in whatever config is current — so the command it resolves to is one no
    // earlier validation ever saw. `config::launch_plan` lexes the [roster]
    // seats and `_spawn` lexes the profile it is handed, but this path did
    // neither: it read `cfg.command()`, opened a pane, and let the pane shell
    // run the string. A profile holding `touch m ; tail -f /dev/null` therefore
    // executed BOTH commands on resume — the same defect the spawn gate closed
    // (colead gate b5d60fec), reached through the restore instead.
    if resuming {
        let spawned = match spawned_entries(&dir, &session) {
            Ok(spawned) => spawned,
            Err(line) => {
                writeln!(err, "{line}")?;
                return Ok(EXIT_FAILED);
            }
        };
        for entry in spawned {
            // An unconfigured profile is not a refusal: the seat is preserved
            // verbatim and never launched, which the restore already handles.
            let command = match cfg.command(&entry.profile, home.as_deref()) {
                Ok(command) => command,
                Err(why) => {
                    writeln!(err, "{why}")?;
                    return Ok(EXIT_FAILED);
                }
            };
            let Some(command) = command.filter(|command| !command.as_str().is_empty()) else {
                continue;
            };
            if let Err(why) = crate::launch_cmd::lex_simple_command(command.as_str()) {
                writeln!(
                    err,
                    "Error: seat '{}' ({}) — profile '{}' refused — {why}. Nothing was resumed.",
                    entry.name, entry.slot, entry.profile
                )?;
                return Ok(EXIT_FAILED);
            }
        }
    }

    let mut shape = Session {
        name: session.clone(),
        mode,
        work_dir,
        origin,
        layout,
        look,
        resuming,
        dir_created: false,
    };

    build(
        &env,
        &mut shape,
        &seats,
        &cfg,
        seat_overrides.as_ref(),
        WatchdogFacts {
            meta_agent,
            sweep_sec: sweep_sec.as_deref(),
            quota_every_secs: &quota_every_secs,
            idle_nudge_secs: &idle_nudge_secs,
            quota: &quota,
        },
        parent.as_ref(),
        lifecycle.take(),
        out,
        err,
    )
}

// ---------------------------------------------------------------------------
// the build: tmux, meta, assets, agents
// ---------------------------------------------------------------------------

/// Read the quota cadence through the existing strict workspace-key reader,
/// layering the project-local occurrence over the global one. That reader
/// preserves an explicit empty assignment as an error; here its logical value
/// is the empty string so the launch diagnostic can name it exactly.
fn configured_quota_every_secs(
    global: Option<&Path>,
    local: Option<&Path>,
) -> Result<Option<String>, String> {
    let read = |file: Option<&Path>| -> Result<Option<String>, String> {
        let Some(file) = file else {
            return Ok(None);
        };
        crate::config::read_global_workspace_key(file, "quota_every_secs").map_err(|why| {
            if why == "quota_every_secs has an invalid value" {
                String::new()
            } else {
                why
            }
        })
    };
    let global = read(global);
    match read(local) {
        Ok(Some(value)) => Ok(Some(value)),
        Ok(None) => global,
        Err(value) => Err(value),
    }
}

/// Read a strict unsigned-seconds knob while preserving malformed assignment
/// text for the launch diagnostic.
fn configured_unsigned_workspace_seconds(
    global: Option<&Path>,
    local: Option<&Path>,
    key: &str,
) -> Result<Option<String>, String> {
    let read = |file: Option<&Path>| -> Result<Option<String>, String> {
        let Some(file) = file else {
            return Ok(None);
        };
        crate::config::read_global_workspace_key(file, key).map_err(|why| {
            if why == format!("{key} has an invalid value") {
                String::new()
            } else {
                why
            }
        })
    };
    let global = read(global);
    match read(local) {
        Ok(Some(value)) => Ok(Some(value)),
        Ok(None) => global,
        Err(value) => Err(value),
    }
}

/// One seat, resolved to what the launch needs to start it.
struct Launching {
    slot: String,
    name: String,
    profile: String,
    /// The client override honored for this seat, if any: an explicit `@`
    /// selection this launch, else the recorded `client.<slot>` carried
    /// forward on a resume. `None` leaves the row absent.
    client: Option<String>,
    binary: String,
    tool: ToolKind,
    session_id: String,
    config_home: Option<String>,
    config_home_base: Option<String>,
    launch_id: String,
    pane: String,
    command_snapshot: Option<config::ResolvedCommand>,
}

/// Watchdog launch facts published atomically with the rest of session meta.
#[derive(Clone, Copy)]
struct WatchdogFacts<'a> {
    meta_agent: bool,
    sweep_sec: Option<&'a str>,
    quota_every_secs: &'a str,
    idle_nudge_secs: &'a str,
    quota: &'a str,
}

#[allow(
    clippy::too_many_lines,
    clippy::too_many_arguments,
    reason = "the frozen order, kept in one place — see the module docs"
)]
fn build(
    env: &Env,
    shape: &mut Session,
    seats: &[Seat],
    cfg: &IdentityConfig,
    seat_overrides: Option<&SeatOverrideSnapshot>,
    watchdog: WatchdogFacts<'_>,
    parent: Option<&FromProof>,
    lifecycle: Option<std::fs::File>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let server = env.server();
    let home = crate::doors::home();
    let sessions = env.sessions();
    let dir = sessions.join(&shape.name);
    let work_dir = shape.work_dir.display().to_string();

    if let Err(why) = std::fs::create_dir_all(&sessions) {
        writeln!(
            err,
            "Error: could not create {} ({why})",
            sessions.display()
        )?;
        return Ok(EXIT_FAILED);
    }
    // The LIFECYCLE LOCK: a resume carries the one under which it chose this
    // destination; a new session takes it here before its first tmux write.
    let lifecycle = if let Some(held) = lifecycle {
        held
    } else {
        let Ok(held) = crate::store::lock(
            &sessions.join(format!(".lifecycle.{}.lock", shape.name)),
            LIFECYCLE_WAIT,
        ) else {
            writeln!(
                err,
                "Error: another lifecycle operation (end) is in progress for '{}' — retry shortly.",
                shape.name
            )?;
            return Ok(EXIT_FAILED);
        };
        held
    };

    let Some(core) = crate::shape::resolved_exe() else {
        writeln!(err, "Error: the core could not name its own binary.")?;
        return Ok(EXIT_FAILED);
    };

    // Ownership as an explicit FACT, decided BEFORE the stamp below needs the
    // directory to exist — a state dir this attempt found is never a state dir
    // this attempt may remove.
    shape.dir_created = !node_exists(&dir);
    if let Err(why) = create_private_dir(&dir) {
        writeln!(err, "Error: could not create {} ({why})", dir.display())?;
        return Ok(EXIT_FAILED);
    }
    // THE LAUNCH-ATTEMPT STAMP, and its position is the whole point: under the
    // lifecycle lock, AFTER the state directory exists and BEFORE the first
    // tmux create. A launch that then dies between the create and the meta
    // publication has still left the moment behind, which is what lets a later
    // reboot proof say "ae has not touched this session since the host booted"
    // without trusting facts the publication may never have reached.
    //
    // CHECKED, never best-effort: a stamp that could not be written would make
    // the next reboot silently unprovable, so the launch is refused instead.
    if let Err(why) =
        crate::store::open(&dir).stamp_launch_attempt(crate::time::Timestamp::now().epoch())
    {
        writeln!(
            err,
            "Error: could not record the launch attempt for '{}' ({why}). No tmux session was created.",
            shape.name
        )?;
        rollback_dir(shape, &dir, err)?;
        return Ok(EXIT_FAILED);
    }

    // ---- the session and its first pane ----
    // The identity and the pane come out of the ONE command that created the
    // session; nothing is captured later through a reusable id.
    let Some((main_pane, live_session_id)) = new_session(&server, &shape.name, &work_dir) else {
        writeln!(err, "Error: could not create tmux session '{}'", shape.name)?;
        rollback_dir(shape, &dir, err)?;
        return Ok(EXIT_FAILED);
    };

    stamp_session(&server, env, shape, &main_pane, &core);
    if let Some(main) = seats.first() {
        name_agent_window(&server, &main_pane, &main.name);
    }

    // ---- worker panes, by layout ----
    let mut panes = vec![main_pane.clone()];
    let workers = seats.len().saturating_sub(1);
    for (index, seat) in seats.iter().skip(1).enumerate() {
        let created = match shape.layout.as_str() {
            "lead-pair" => match index {
                0 => split(&server, &panes[0], &work_dir, Split::Horizontal),
                1 => new_window(
                    &server,
                    &format!("{}:", tmux::session_target(&shape.name)),
                    &seat.name,
                    &work_dir,
                ),
                _ => split(&server, &panes[panes.len() - 1], &work_dir, Split::Vertical),
            },
            "lead-solo" => match index {
                0 => new_window(
                    &server,
                    &format!("{}:", tmux::session_target(&shape.name)),
                    &seat.name,
                    &work_dir,
                ),
                _ => split(&server, &panes[panes.len() - 1], &work_dir, Split::Vertical),
            },
            "vertical" => split(
                &server,
                &format!("{}:", tmux::session_target(&shape.name)),
                &work_dir,
                Split::Horizontal,
            ),
            _ => split(
                &server,
                &format!("{}:", tmux::session_target(&shape.name)),
                &work_dir,
                Split::Vertical,
            ),
        };
        let Some(pane) = created else {
            return rollback_launch(
                shape,
                &dir,
                &server,
                &format!("Error: could not create a pane for worker {index}"),
                err,
            );
        };
        panes.push(pane);
    }
    apply_layout(&server, shape, &panes, workers);

    // Pane stamps.
    for (index, seat) in seats.iter().enumerate() {
        stamp_pane(
            &server,
            &panes[index],
            &seat.name,
            &seat.slot,
            &seat.profile,
        );
    }

    // ---- the roster, and the ids each seat launches with ----
    // The recorded client rows, re-read under the lifecycle lock: preflight
    // (or the post-lock gate for a launch without overrides) already refused
    // every unusable row, so an Invalid met here is a mid-flight hand edit —
    // and it still refuses rather than laundering toward Missing.
    let recorded_meta: Option<Meta> = if shape.resuming {
        meta::read_bytes(&dir)
            .ok()
            .map(|bytes| Meta::parse(&String::from_utf8_lossy(&bytes)))
    } else {
        None
    };
    let mut launching: Vec<Launching> = Vec::new();
    for (index, seat) in seats.iter().enumerate() {
        let carried: Option<String> = match recorded_meta
            .as_ref()
            .and_then(|meta| meta.roster().iter().find(|entry| entry.slot == seat.slot))
        {
            None => None,
            Some(entry) => match &entry.client {
                crate::meta::RecordedClient::Missing => None,
                crate::meta::RecordedClient::Label(label) => Some(label.clone()),
                crate::meta::RecordedClient::Invalid => {
                    return rollback_launch(
                        shape,
                        &dir,
                        &server,
                        &invalid_client_refusal(&shape.name, &seat.name, &seat.slot),
                        err,
                    );
                }
            },
        };
        if let (Some(selected), Some(known)) = (seat.client_override.as_deref(), carried.as_deref())
            && selected != known
        {
            return rollback_launch(
                shape,
                &dir,
                &server,
                &different_label_refusal(&shape.name, &seat.name, &seat.profile, known, selected),
                err,
            );
        }
        if seat.client_override.is_none()
            && let Some(added) = carried.as_deref()
        {
            // A label preflight never saw: hand-added (or a concurrent
            // launch's) between preflight and this lock. It carries no R1/R4
            // proof, and this seat was composed without it — retrying
            // re-runs preflight, which honors it properly.
            return rollback_launch(
                shape,
                &dir,
                &server,
                &format!(
                    "Error: session '{}' seat '{}' recorded client override '{added}' while this launch was starting — \
                     nothing was resumed; retry the launch.",
                    shape.name, seat.name
                ),
                err,
            );
        }
        // An unproven pairing honors the pane but mints no label fact: only a
        // carried row — an earlier PROVEN start's fact, republished unchanged —
        // may survive it.
        let client = if seat.store_proven {
            seat.client_override.clone().or(carried)
        } else {
            carried
        };
        let stored = shape
            .resuming
            .then(|| meta_value(&dir, &format!("harness_session.{}", seat.slot)))
            .flatten()
            .filter(|id| !id.is_empty() && id != PENDING);
        let session_id = stored.unwrap_or_else(|| {
            if launch::takes_launch_session_id(seat.tool) {
                launch::generate_uuid()
            } else {
                PENDING.to_owned()
            }
        });
        let launch_id = launch_token(
            seat.tool,
            shape
                .resuming
                .then(|| meta_value(&dir, &format!("launch_id.{}", seat.slot)))
                .flatten(),
        );
        launching.push(Launching {
            slot: seat.slot.clone(),
            name: seat.name.clone(),
            profile: seat.profile.clone(),
            client,
            binary: seat.binary.clone(),
            tool: seat.tool,
            session_id,
            config_home: shape
                .resuming
                .then(|| meta_value(&dir, &format!("config_home.{}", seat.slot)))
                .flatten()
                .filter(|value| !value.is_empty()),
            config_home_base: shape
                .resuming
                .then(|| meta_value(&dir, &format!("config_home_base.{}", seat.slot)))
                .flatten()
                .filter(|value| !value.is_empty()),
            launch_id,
            pane: panes[index].clone(),
            command_snapshot: seat_overrides.map(|_| seat.command.clone()),
        });
    }

    // ---- spawned agents, restored on resume ----
    if shape.resuming {
        let spawned = match spawned_entries(&dir, &shape.name) {
            Ok(spawned) => spawned,
            Err(line) => return rollback_launch(shape, &dir, &server, &line, err),
        };
        for entry in spawned {
            // A profile that is not configured on THIS machine keeps its seat
            // VERBATIM at its original index — a later resume with the profile
            // configured restores the worker, and preserving the index keeps
            // the slot key stable for any in-flight request addressed to it.
            let command = match cfg.command(&entry.profile, home.as_deref()) {
                Ok(Some(command)) => command,
                Ok(None) => IdentityConfig::resolved_snapshot(""),
                Err(why) => {
                    return rollback_launch(shape, &dir, &server, &why.to_string(), err);
                }
            };
            let pane = if command.as_str().is_empty() {
                String::new()
            } else {
                match new_window(
                    &server,
                    &format!("{}:", tmux::session_target(&shape.name)),
                    &entry.name,
                    &work_dir,
                ) {
                    Some(pane) => {
                        stamp_pane(&server, &pane, &entry.name, &entry.slot, &entry.profile);
                        panes.push(pane.clone());
                        pane
                    }
                    None => String::new(),
                }
            };
            let tool = ToolKind::from_cmd(command.as_str());
            let launch_id =
                launch_token(tool, meta_value(&dir, &format!("launch_id.{}", entry.slot)));
            launching.push(Launching {
                slot: entry.slot,
                name: entry.name,
                profile: entry.profile,
                client: entry.client,
                binary: entry.binary,
                tool,
                session_id: entry.harness_session,
                config_home: entry.config_home,
                config_home_base: entry.config_home_base,
                launch_id,
                pane,
                command_snapshot: seat_overrides
                    .and_then(|_| (!command.as_str().is_empty()).then(|| command.clone())),
            });
        }
        // Rebalance only the SPLIT layouts: a spawned agent gets its own window,
        // so window 0 gains no panes, and even-vertical would stack lead-pair's
        // side-by-side leads.
        if shape.layout != "lead-solo" && shape.layout != "lead-pair" {
            let target = format!("{}:", tmux::session_target(&shape.name));
            let layout = if shape.layout == "vertical" {
                "even-horizontal"
            } else {
                "even-vertical"
            };
            let _ = transport::run_tmux_op(&argv(
                &server,
                &Op::SelectLayout {
                    target: &target,
                    layout,
                },
            ));
        }
    }

    // ---- the meta, published as ONE document ----
    let document = match meta_document(env, shape, &launching, watchdog, parent) {
        Ok(document) => document,
        Err(why) => return rollback_launch(shape, &dir, &server, &format!("Error: {why}"), err),
    };
    let published = publish_meta_and_seed_uuid(
        &server,
        Some(&live_session_id),
        &dir,
        shape.resuming,
        &document,
    );
    if published.is_err() {
        return rollback_launch(
            shape,
            &dir,
            &server,
            "Error: the session meta could not be published.",
            err,
        );
    }

    // ---- assets ----
    // RESOLVED, not as invoked: the helper links published below must name the
    // immutable core, not `~/.local/bin/ae`. macOS answers `current_exe()` with
    // the path this process was exec'd BY, so on an installed machine the raw
    // answer is the command symlink — and a session whose helpers pointed at it
    // would follow the next `ae upgrade` to a core it was never built against.
    if let Err(why) = assets::write_helpers(&dir, &core) {
        return rollback_launch(shape, &dir, &server, &format!("Error: {why}"), err);
    }
    let manifest = crate::render::manifest_document(
        &dir,
        &shape.name,
        &work_dir,
        &shape.origin.display().to_string(),
        shape.mode.as_str(),
        &main_pane,
        &env.config_files(),
    );
    if let Err(why) = assets::publish_document(&dir.join("workspace.md"), &manifest) {
        return rollback_launch(
            shape,
            &dir,
            &server,
            &format!("Error: could not write the workspace manifest ({why})."),
            err,
        );
    }

    // Resume-only event retention, before the events pane starts tailing: the
    // log is append-only, so a long-lived session accumulates every resume.
    if shape.resuming {
        trim_events(&dir);
    }

    // Freshly created panes can still be inside shell init when tmux returns.
    std::thread::sleep(SHELL_SETTLE);

    // ---- launch scripts, and the paste into each pane's shell ----
    for agent in &launching {
        // A seat with no pane is a preserved roster row, not an agent to start.
        if agent.pane.is_empty() {
            continue;
        }
        if let Err(why) = start_agent(shape, &dir, &core, agent, &server, err)? {
            return rollback_launch(shape, &dir, &server, &format!("Error: {why}"), err);
        }
    }

    // The session is fully on disk; `ae end` may safely delete it, and the
    // capture threads below must not inherit the hold.
    drop(lifecycle);

    // ---- post-launch capture, and the deferred codex prompt ----
    capture::start(&dir, &launching_capture(&launching, shape.resuming));

    // ---- the monitor panes ----
    let events_pane = ensure_events_pane(&server, &shape.name, &dir);
    if let Some(anchor) = &events_pane {
        start_watchdog_pane(shape, &dir, &server, anchor);
    }

    // ---- the per-window half of the look ----
    // LAST, so every window a layout, a seat or a monitor created is stamped.
    // The watchdog restamps whatever appears after this.
    stamp_windows(&server, &shape.name, &shape.look);

    // ---- the Telegram bridge ----
    //
    // LAST, and after the session exists, deliberately: the bridge is
    // best-effort and strictly non-fatal. A session that is up is never failed
    // by a bridge that is not.
    if !env.no_autostart {
        autostart_telegram(env, shape, &dir, &server, err);
    }

    let _ = transport::run_tmux_op(&argv(&server, &Op::SelectPane { pane: &main_pane }));
    crate::autoupgrade::schedule();
    if !env.attach {
        writeln!(
            out,
            "Session '{}' started. Attach with: {}",
            shape.name,
            attach_hint(&server, &shape.name)
        )?;
        return Ok(0);
    }
    Ok(attach(&server, env, &shape.name, out)?)
}

// ---------------------------------------------------------------------------
// the pieces
// ---------------------------------------------------------------------------

/// Create the session and its first pane; report the created pane AND the full
/// identity the SAME command printed.
///
/// There is no later capture to race: the server reads the identity while it
/// creates the session, so a replacement can never become the "proven" one.
fn new_session(server: &ServerId, name: &str, work_dir: &str) -> Option<(String, ProvenIdentity)> {
    let (succeeded, stdout) =
        transport::run_tmux_op(&argv(server, &Op::NewSession { name, work_dir }));
    let created = tmux::interpret_new_session(succeeded, &stdout)?;
    Some((
        created.pane,
        ProvenIdentity {
            server: created.server,
            session: created.session,
        },
    ))
}

/// Split `target` and report the new pane's id.
fn split(server: &ServerId, target: &str, work_dir: &str, split: Split) -> Option<String> {
    let (succeeded, stdout) = transport::run_tmux_op(&argv(
        server,
        &Op::SplitWindow {
            target,
            work_dir,
            split,
            command: &[],
        },
    ));
    interpret_pane_id(succeeded, &stdout)
}

/// Create a detached window and report its pane's id.
fn new_window(server: &ServerId, target: &str, name: &str, work_dir: &str) -> Option<String> {
    let (succeeded, stdout) = transport::run_tmux_op(&argv(
        server,
        &Op::NewWindow {
            target,
            name: "",
            work_dir,
            command: &[],
        },
    ));
    let pane = interpret_pane_id(succeeded, &stdout)?;
    name_agent_window(server, &pane, name);
    Some(pane)
}

/// Kill the session this launch created — the rollback's first step.
fn kill_session(server: &ServerId, name: &str) -> bool {
    // `transport` adds tmux's `=` exact-target marker. This build just created
    // the session under the lifecycle lock, so a fallible list-sessions lookup
    // would only add a way for rollback to strand it.
    transport::kill_session(server, name)
}

/// The environment, options and status bar every ae session carries.
fn stamp_session(server: &ServerId, env: &Env, shape: &Session, main_pane: &str, core: &Path) {
    let name = shape.name.as_str();
    // Claude Code refuses to start with these set inside tmux.
    for key in ["CLAUDECODE", "CLAUDE_CODE_SESSION"] {
        let _ = transport::run_tmux_op(&argv(server, &Op::UnsetEnv { session: name, key }));
    }
    let work_dir = shape.work_dir.display().to_string();
    let origin = shape.origin.display().to_string();
    let home = env.home.display().to_string();
    for (key, value) in [
        ("AE_SESSION", "1"),
        ("AE_ORIGIN", origin.as_str()),
        ("AE_DIR", work_dir.as_str()),
        ("AE_MODE", shape.mode.as_str()),
        ("AE_HOME", home.as_str()),
    ] {
        let _ = transport::run_tmux_op(&argv(
            server,
            &Op::SetEnv {
                session: name,
                key,
                value,
            },
        ));
    }
    for (option, value) in [
        ("mouse", "on"),
        ("focus-events", "on"),
        ("history-limit", "50000"),
        ("automatic-rename", "off"),
    ] {
        let _ = transport::publish_option(server, tmux::OptionScope::Session, name, option, value);
    }
    stamp_main_pane(server, name, main_pane, true);
    apply_status_bar(
        server,
        &shape.name,
        &status_paths(
            shape.mode.as_str(),
            &shape.origin.display().to_string(),
            &shape.work_dir.display().to_string(),
            &home,
        ),
        &shape.look,
    );
    assert_status_bindings(server, env, core);
    let _ = transport::run_tmux_op(&argv(server, &Op::SelectPane { pane: main_pane }));
}

/// Publish the lead-pane navigation fact only after membership was proven.
///
/// A stale hint is actively unset: leaving one behind would let a future pane
/// with the same text appear authoritative even though this pass disproved it.
pub(crate) fn stamp_main_pane(
    server: &ServerId,
    session: &str,
    main_pane: &str,
    pane_belongs: bool,
) {
    if pane_belongs && !main_pane.is_empty() {
        let _ = transport::publish_option(
            server,
            tmux::OptionScope::Session,
            session,
            crate::theme::MAIN_PANE_OPTION,
            main_pane,
        );
    } else {
        let _ = transport::clear_option(
            server,
            tmux::OptionScope::Session,
            session,
            crate::theme::MAIN_PANE_OPTION,
        );
    }
    stamp_client_session_hook(server, session, main_pane, pane_belongs);
}

/// Install the session focus hook with the session id captured NOW.
///
/// The hook fires after `switch-client`; its pane may have moved by then. The
/// runtime predicate prevents that stale pane from selecting a foreign
/// session's window. Reattach and migration pass through here too, replacing
/// the unguarded hook published by an older core.
fn stamp_client_session_hook(
    server: &ServerId,
    session: &str,
    main_pane: &str,
    pane_belongs: bool,
) {
    let session_target = tmux::session_target(session);
    let Some(session_id) = transport::observe_session_id(server, session) else {
        let _ = transport::run_tmux_op(&argv(
            server,
            &Op::UnsetClientSessionHook {
                target: &session_target,
            },
        ));
        return;
    };
    if pane_belongs && !main_pane.is_empty() {
        let _ = transport::run_tmux_op(&argv(
            server,
            &Op::SetClientSessionHook {
                session_id: &session_id,
                pane: main_pane,
            },
        ));
    } else {
        let _ = transport::run_tmux_op(&argv(
            server,
            &Op::UnsetClientSessionHook {
                target: &session_id,
            },
        ));
    }
}

fn assert_status_bindings(server: &ServerId, env: &Env, core: &Path) {
    let config = env
        .global
        .clone()
        .unwrap_or_else(|| env.home.join("config"));
    let launcher = picker_launcher(crate::shape::current(), core, &env.home, &config, server);
    let menu_mouse = transport::observe_tmux_floor(server).menu_mouse();
    for binding in status_bindings_argv(server, &launcher, menu_mouse) {
        let _ = transport::run_tmux_op(&binding);
    }
}

/// The SESSION-scoped look: the two status lines ae owns, and the `@ae_*`
/// values that fill them before the watchdog's first cycle.
///
/// Session scope throughout, so a non-ae session on the same server keeps the
/// user's own theme. The per-window half — pane borders, menu and popup styles
/// — is [`stamp_window`]'s, because tmux keeps those in the WINDOW table.
pub(crate) fn apply_status_bar(
    server: &ServerId,
    name: &str,
    paths: &str,
    look: &crate::theme::Look,
) {
    write_options(server, name, crate::theme::session_options(look, paths));
}

/// The same look, WITHOUT the attention seed — for a session that is already
/// running and already has a verdict the watchdog published.
pub(crate) fn redress_status_bar(
    server: &ServerId,
    name: &str,
    paths: &str,
    look: &crate::theme::Look,
) {
    write_options(server, name, crate::theme::redress_options(look, paths));
}

/// Write one option set at SESSION scope.
fn write_options(server: &ServerId, name: &str, options: Vec<(String, String)>) {
    for (option, value) in options {
        let _ =
            transport::publish_option(server, tmux::OptionScope::Session, name, &option, &value);
    }
}

/// The WINDOW-scoped look, stamped on one window.
pub(crate) fn stamp_window(server: &ServerId, target: &str, look: &crate::theme::Look) -> bool {
    if !look.drawn {
        // `[workspace] theme = off`: the window keeps whatever the user's own
        // tmux configuration gave it, stamp included, so nothing here is ever
        // written and the watchdog finds nothing to restamp.
        return true;
    }
    // `&=`, never `&&`: one refused option must not skip the rest of the set.
    let mut ok = true;
    let set = |name: &str, value: &str| {
        let (succeeded, _) = transport::run_tmux_op(&argv(
            server,
            &Op::SetWindowOption {
                target,
                name,
                value,
            },
        ));
        succeeded
    };
    for (option, value) in crate::theme::window_options(look) {
        ok &= set(&option, &value);
    }
    // The STAMP is the claim that all of the above happened, so it is written
    // LAST and only when they did. A stamp over a half-dressed window would
    // tell every later cycle there was nothing left to do.
    if !ok {
        return false;
    }
    set(
        crate::theme::WINDOW_STAMP_OPTION,
        &crate::theme::window_stamp(look),
    )
}

/// Stamp every window this session has — called once the layout, the seats and
/// the monitor panes have all created theirs.
pub(crate) fn stamp_windows(server: &ServerId, session: &str, look: &crate::theme::Look) {
    let Some(panes) = transport::observe_window_panes(server, session) else {
        return;
    };
    let mut done: Vec<String> = Vec::new();
    for pane in panes {
        if done.contains(&pane.window_id) {
            continue;
        }
        done.push(pane.window_id.clone());
        stamp_window(server, &pane.window_id, look);
    }
}

/// The location segment of the status bar, whose shape is mode-aware.
///
/// SHORTENED against `home`: the bar shares one line with the branch and the
/// watch count, and a worktree path spelled in full pushes both off the end.
pub(crate) fn status_paths(mode: &str, origin: &str, work_dir: &str, home: &str) -> String {
    let short = |path: &str| crate::theme::short_path(home, path);
    match Mode::parse(mode) {
        // An unrecorded or unknown mode reads as local: the work dir alone
        // rather than an arrow to nothing.
        Some(Mode::Local) | None => short(work_dir),
        Some(Mode::Git | Mode::Full) => format!("{} → {}", short(origin), short(work_dir)),
    }
}

/// Distribute the panes the layout put in each window.
fn apply_layout(server: &ServerId, shape: &Session, panes: &[String], workers: usize) {
    if let Some(main_pane) = panes.first() {
        stamp_lead_pair_policy(server, shape.layout.as_str(), main_pane);
    }
    for command in layout_argvs(server, shape.layout.as_str(), &shape.name, panes, workers) {
        let _ = transport::run_tmux_op(&command);
    }
}

/// Idempotently install and apply the complete lead-pair window policy.
pub(crate) fn stamp_lead_pair_policy(server: &ServerId, layout: &str, main_pane: &str) {
    for command in lead_pair_policy_argvs(server, layout, main_pane) {
        let _ = transport::run_tmux_op(&command);
    }
}

fn lead_pair_policy_argvs(server: &ServerId, layout: &str, main_pane: &str) -> Vec<TmuxArgv> {
    if layout != "lead-pair" || main_pane.is_empty() {
        return Vec::new();
    }
    vec![
        argv(server, &Op::SetLeadPairWidth { target: main_pane }),
        argv(server, &Op::SelectLeadPairLayout { pane: main_pane }),
        argv(server, &Op::SetLeadPairResizeHook { pane: main_pane }),
    ]
}

/// Build the complete, ordered layout program before touching tmux.
fn layout_argvs(
    server: &ServerId,
    layout: &str,
    session: &str,
    panes: &[String],
    workers: usize,
) -> Vec<TmuxArgv> {
    if panes.len() <= 1 {
        return Vec::new();
    }
    let mut commands = Vec::new();
    let select = |target: &str, layout: &str| argv(server, &Op::SelectLayout { target, layout });
    match layout {
        "lead-pair" => {
            if workers > 2 {
                commands.push(select(&panes[2], "even-vertical"));
            }
        }
        "lead-solo" => {
            if workers > 1 {
                commands.push(select(&panes[1], "even-vertical"));
            }
        }
        "vertical" => commands.push(select(
            &format!("{}:", tmux::session_target(session)),
            "even-horizontal",
        )),
        _ => commands.push(select(
            &format!("{}:", tmux::session_target(session)),
            "even-vertical",
        )),
    }
    commands
}

/// Name a window for the first agent placed in it and freeze that name.
///
/// Later splits do not call this: once a second agent joins, the watchdog's
/// `@ae_window_agents` value carries both identities while the routing name
/// stays stable.
pub(crate) fn name_agent_window(server: &ServerId, pane: &str, name: &str) {
    let _ = transport::publish_option(
        server,
        tmux::OptionScope::Window,
        pane,
        "automatic-rename",
        "off",
    );
    let _ = transport::rename_window(server, pane, &tmux::format_literal(name));
}

/// Label one pane with the identity it holds.
///
/// The PROFILE goes on the pane too: the border title names what an agent is
/// as well as who, and the roster it would otherwise be read from is in the
/// meta, not in tmux.
fn stamp_pane(server: &ServerId, pane: &str, name: &str, slot: &str, profile: &str) {
    let _ = transport::set_pane_title(server, pane, &format!("ae:{name}"));
    // The IDENTITY, verbatim: this is what the roster, the monitor's own names
    // and every pane lookup match against, so it is never rewritten.
    let _ = transport::publish_option(server, tmux::OptionScope::Pane, pane, "@ae_agent", name);
    // The same name as DRAWN. A seat name comes back off a hand-editable meta,
    // and the border format reads this one, so what a drawer would take for a
    // style is dropped here rather than in the identity.
    let _ = transport::publish_option(
        server,
        tmux::OptionScope::Pane,
        pane,
        crate::theme::AGENT_LABEL_OPTION,
        &crate::theme::agent_label(name),
    );
    let _ = transport::publish_option(server, tmux::OptionScope::Pane, pane, "@ae_slot", slot);
    let _ = transport::publish_option(
        server,
        tmux::OptionScope::Pane,
        pane,
        crate::theme::PROFILE_OPTION,
        // SANITISED at the sink: a profile name comes back off a hand-editable
        // meta, and the drawer reads `#[…]` out of an option value, so a
        // profile carrying one would restyle the pane border it names.
        &crate::theme::bar_text(profile, crate::theme::PROFILE_WIDTH),
    );
}

/// Build the WHOLE initial meta — base facts, then the v2 roster block.
///
/// # Errors
///
/// The refusal text when a resolved value carries a byte that would corrupt a
/// record.
#[allow(
    clippy::too_many_lines,
    reason = "the frozen meta, built in one place so the document has one author"
)]
fn meta_document(
    env: &Env,
    shape: &Session,
    launching: &[Launching],
    watchdog: WatchdogFacts<'_>,
    parent: Option<&FromProof>,
) -> Result<String, String> {
    let dir = env.sessions().join(&shape.name);
    let main_pane = launching
        .first()
        .map(|agent| agent.pane.clone())
        .unwrap_or_default();
    // Launch facts that must survive every rewrite.
    let preserved = |key: &str| meta_value(&dir, key).filter(|v| !v.is_empty());
    // A row named twice says nothing; never copy one forward.
    let sole_preserved = |key: &str| {
        meta::sole_value(&meta::read_bytes(&dir).ok()?, key)
            .map(|value| String::from_utf8_lossy(value).into_owned())
            .filter(|value| !value.is_empty())
    };
    let started = crate::time::Timestamp::now().epoch().to_string();
    let created = if shape.resuming {
        preserved("created")
            .or_else(|| crate::session::legacy_created_epoch(&dir).map(|epoch| epoch.to_string()))
            .or_else(|| {
                preserved("launch_time.main")
                    .filter(|epoch| epoch.parse::<i64>().is_ok_and(i64::is_positive))
            })
    } else {
        Some(started.clone())
    };
    let session_id = preserved("session_id").unwrap_or_else(launch::generate_uuid);
    let git_base = preserved("git_base_commit").or_else(|| {
        (!shape.resuming && shape.mode != Mode::Local)
            .then(|| crate::git::head(path_bytes(&shape.work_dir)))
            .filter(|head| head != "-")
    });
    let (parent_id, handover, pending) = match parent {
        Some(proof) => (
            Some(proof.id.clone()),
            Some(proof.handover.clone()),
            Some(proof.pending.clone()),
        ),
        None => (
            preserved("parent_archive_id"),
            preserved("parent_archive_handover_count"),
            preserved("parent_archive_pending_count"),
        ),
    };

    let mut body = String::new();
    let mut row = |key: &str, value: &str| {
        body.push_str(key);
        body.push('=');
        body.push_str(value);
        body.push('\n');
    };
    // The SHAPE row, first: it says how everything below it is to be read, and
    // its absence is what tells a later ae that this meta pre-dates the chain.
    row(crate::migrate::KEY, &crate::migrate::CURRENT.to_string());
    row("mode", shape.mode.as_str());
    row("origin", &shape.origin.display().to_string());
    row("session", &shape.name);
    row("session_id", &session_id);
    if let Some(created) = created {
        row("created", &created);
    }
    row("started", &started);
    row("work_dir", &shape.work_dir.display().to_string());
    row("layout", &shape.layout);
    row(
        "config",
        &env.global
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
    );
    let origin_local = shape.origin.join(".ae").join("config");
    if let Some(local) = env.local.as_ref().filter(|local| *local != &origin_local) {
        row(
            crate::config::LOCAL_CONFIG_KEY,
            &local.display().to_string(),
        );
    }
    row("main_pane", &main_pane);
    row("ae_version", crate::VERSION);
    if let Some(core) = crate::shape::resolved_exe() {
        // The core binding, pinned per session as a PAIR: a helper that found a
        // binary whose version disagreed with the session's would be running a
        // different contract than the one this session was built against.
        let ae_core = env.core.as_ref().unwrap_or(&core);
        row("ae_core", &ae_core.display().to_string());
        row(
            "ae_core_version",
            env.core_version.as_deref().unwrap_or(crate::VERSION),
        );
    }
    row("tmux_server", &env.server_value);
    row("tmux_server_kind", &env.server_kind);
    for (key, value) in [
        (
            "watchdog",
            preserved("watchdog").or_else(|| preserved("loop")),
        ),
        ("goal", preserved("goal")),
        ("git_base_commit", git_base),
        ("session_id_origin", preserved("session_id_origin")),
    ] {
        if let Some(value) = value {
            row(key, &value);
        }
    }
    row("quota_every_secs", watchdog.quota_every_secs);
    row("idle_nudge_secs", watchdog.idle_nudge_secs);
    row("quota", watchdog.quota);
    if let Some(id) = parent_id {
        row("parent_archive_id", &id);
        row(
            "parent_archive_handover_count",
            handover.as_deref().unwrap_or("0"),
        );
        row(
            "parent_archive_pending_count",
            pending.as_deref().unwrap_or("0"),
        );
    }
    if watchdog.meta_agent {
        row("meta_agent", "true");
        if let Some(seconds) = watchdog.sweep_sec {
            row("sweep_sec", seconds);
        }
    }
    for agent in launching {
        if !agent.launch_id.is_empty() {
            row(&format!("launch_id.{}", agent.slot), &agent.launch_id);
        }
        if agent.tool.adapter().capture.is_needed() {
            let key = format!("capture_floor.{}", agent.slot);
            let retained = shape.resuming && launch::id_probeable(&agent.session_id);
            let floor = if retained {
                // A retained exact conversation was born under the original
                // floor. Metadata predating this row gets 0 so a later resume
                // instant cannot exclude that already-proved origin.
                preserved(&key)
                    .filter(|value| value.parse::<i64>().is_ok_and(|epoch| epoch >= 0))
                    .unwrap_or_else(|| "0".to_owned())
            } else {
                started.clone()
            };
            row(&key, &floor);
        }
        // The observed-model pair is EVIDENCE a later resume applies. This
        // document is the WHOLE meta, so a pair not enumerated here is deleted
        // by the first resume — before it could ever be honored.
        for key in [
            format!("{}{}", crate::meta::OBSERVED_MODEL_PREFIX, agent.slot),
            format!("{}{}", crate::meta::OBSERVED_MODEL_PIN_PREFIX, agent.slot),
        ] {
            if let Some(value) = sole_preserved(&key) {
                row(&key, &value);
            }
        }
    }

    let seats: Vec<roster::SeatLines> = launching
        .iter()
        .map(|agent| roster::SeatLines {
            slot: agent.slot.clone(),
            name: agent.name.clone(),
            profile: agent.profile.clone(),
            client: agent.client.clone(),
            binary: (!agent.binary.is_empty()).then(|| agent.binary.clone()),
            harness_session: (!agent.session_id.is_empty()).then(|| agent.session_id.clone()),
            config_home: agent.config_home.clone(),
            config_home_base: agent.config_home_base.clone(),
        })
        .collect();
    if let Some(bad) = seats
        .iter()
        .find(|seat| seat.name.contains('\n') || seat.slot.contains('\n'))
    {
        return Err(format!(
            "a resolved identity contains a newline that would corrupt the meta: {:?}",
            seat_label(bad)
        ));
    }
    body.push_str(&roster::render(&seats));
    Ok(body)
}

/// A seat's label for a refusal.
fn seat_label(seat: &roster::SeatLines) -> String {
    format!("{}={}", seat.slot, seat.name)
}

/// What a vacant-only seed of the session UUID fact did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedOutcome {
    /// The option now holds this session's UUID — whether this call wrote it
    /// or it was already there. A post-read cannot tell those apart, and this
    /// type never pretends to: it reports the FINAL STATE, not a cause.
    Recorded,
    /// A DIFFERENT nonempty value is recorded; nothing was overwritten.
    Held,
    /// The option is vacant: the guarded branch refused (a replaced
    /// incarnation, a failed guard) or nothing was written. Again a state.
    Vacant,
    /// The option or the server could not be observed.
    Unobserved,
}

/// The full immutable identity a UUID write must reprove: the SERVER
/// incarnation (pid and start epoch) and the session's id and creation inside
/// it.
///
/// `#{session_id}` is REUSED once a server empties, and `#{session_created}` is
/// whole seconds, so neither alone survives a same-second replacement. The
/// server pair is the house model's discriminator: a replacement behind a
/// restarted or different server process is caught even when the reclaimed id
/// and the creation second coincide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenIdentity {
    /// The tmux server the session was proven on.
    pub server: crate::tmux::ServerIdentity,
    /// The session's id and creation instant on that server.
    pub session: crate::tmux::SessionIdentity,
}

/// The MIGRATION-side capture: the identity of the session ONE pane belongs
/// to, for a recorded main pane. It is NOT the launch capture — a launch takes
/// its identity from the creating command's own output — and a pane id is
/// reusable after a server restart, so this value alone cannot tell a
/// replacement from the proven session. The caller pairs it with the session's
/// ownership pair, and the guarded write reproves the full identity.
#[must_use]
pub fn pane_proven_identity(server: &ServerId, pane: &str) -> Option<ProvenIdentity> {
    transport::observe_server_identity(server)
        .zip(transport::observe_pane_session_identity(server, pane))
        .map(|(server, session)| ProvenIdentity { server, session })
}

/// Seed `SESSION_ID_OPTION` for one live tmux session through ONE guarded
/// server-side operation.
///
/// The guard (`server pid/start`, session `id/created`, option empty) and the
/// set live inside the same tmux invocation's queued command, so nothing can
/// interleave between them: neither a full-server replacement after a first
/// proof call nor a same-name recreation gets the proven incarnation's UUID by
/// a read-then-write window. The outcome is deliberately a FINAL-STATE reading:
/// an option that already held this UUID is `Recorded` exactly like one this
/// call wrote, a replacement carrying a nonempty value is `Held`, and a vacant
/// option is `Vacant` — the type never labels a cause the post-read cannot
/// know. The
/// write itself is the guarded set. The ONE writer of the fact; the watchdog
/// never calls it.
#[must_use]
pub fn seed_session_uuid(
    server: &ServerId,
    expected_identity: &ProvenIdentity,
    uuid: &str,
) -> SeedOutcome {
    if !transport::publish_guarded_session_option(
        server,
        &expected_identity.server,
        &expected_identity.session,
        crate::theme::SESSION_ID_OPTION,
        uuid,
    ) {
        return SeedOutcome::Unobserved;
    }
    match transport::observe_option_reading(
        server,
        &expected_identity.session.id,
        crate::theme::SESSION_ID_OPTION,
    ) {
        crate::tmux::OptionReading::Set(value)
            if crate::archive::canonical_uuid(&value) == uuid =>
        {
            SeedOutcome::Recorded
        }
        crate::tmux::OptionReading::Set(_) => SeedOutcome::Held,
        crate::tmux::OptionReading::Vacant => SeedOutcome::Vacant,
        crate::tmux::OptionReading::Unknown => SeedOutcome::Unobserved,
    }
}

/// Publish `document` as the session meta, then — ONLY on success — seed the
/// session UUID fact from the SAME in-memory document, onto the SAME tmux
/// incarnation the caller proved.
///
/// One helper so neither ordering nor identity can drift: a failed publication
/// returns before any option write, the identity stamped is the one in the
/// document just published (never a second read), and the WRITE itself reproves
/// `expected_session_id` before it touches anything.
///
/// # Errors
///
/// The meta publication's own error; no option is written on it.
pub fn publish_meta_and_seed_uuid(
    server: &ServerId,
    expected_identity: Option<&ProvenIdentity>,
    dir: &Path,
    resuming: bool,
    document: &str,
) -> Result<SeedOutcome, crate::meta::RewriteError> {
    if resuming {
        crate::meta::replace(dir, document)?;
    } else {
        crate::meta::init(dir, document)?;
    }
    let uuid = crate::meta::first_value(document.as_bytes(), "session_id")
        .map(|value| crate::archive::canonical_uuid(&String::from_utf8_lossy(value)))
        .unwrap_or_default();
    let Some(expected_identity) = expected_identity.filter(|_| !uuid.is_empty()) else {
        return Ok(SeedOutcome::Unobserved);
    };
    Ok(seed_session_uuid(server, expected_identity, &uuid))
}

/// Hand one agent's pane the command that BECOMES its agent, and wait for the
/// tool to take the pane over.
fn start_agent(
    shape: &Session,
    dir: &Path,
    core: &Path,
    agent: &Launching,
    server: &ServerId,
    err: &mut impl Write,
) -> io::Result<Result<(), String>> {
    // THE MARKER IS THE CREATE-VS-RESUME DISCRIMINATOR, so a fresh seat must
    // not inherit one.
    if !shape.resuming
        && let Err(why) = crate::run::clear_slot(dir, &agent.slot)
    {
        writeln!(
            err,
            "ae: could not clear the start marker for '{}' ({why}) — agent not started",
            agent.slot
        )?;
        return Ok(Err(format!("stale start marker for {}", agent.slot)));
    }
    let resuming_seat =
        crate::lifecycle::path_exists(&crate::run::started_marker(dir, &agent.slot));
    // Fire and forget: the reader here IS a shell, and an unconfirmed submit
    // must not abort a launch that may well have taken.
    let command = agent.command_snapshot.as_ref().map_or_else(
        || crate::run::pane_command(core, dir, &agent.slot),
        |snapshot| {
            crate::run::pane_command_with_snapshot(core, dir, &agent.slot, snapshot.as_str())
        },
    );
    let _ = deliver::submit_shell_text(server, &agent.pane, &command);
    wait_for_agent_start(server, &agent.pane, agent.tool);
    if agent.tool.adapter().capture.is_needed() {
        // Kept as the legacy/lifecycle launch stamp. Capture safety uses the
        // separate `capture_floor` already published before this exec.
        let _ = meta::rewrite(
            dir,
            &format!("launch_time.{}", agent.slot),
            Some(&crate::time::Timestamp::now().epoch().to_string()),
        );
    }
    let prompt = launch::initial_prompt_for(agent.tool, dir, &agent.slot);
    if !prompt.is_empty() && launch_turn_is_pasted(agent.tool, resuming_seat) {
        deliver_launch_prompt(dir, server, agent, &prompt, err)?;
    }
    Ok(Ok(()))
}

/// The launch ID this seat launches with: the one it already has, else a fresh
/// one. It has two jobs: disambiguating post-launch capture in a tool's own
/// store, and guarding observed-model writes against a re-created seat.
///
/// Every tool needs the second job. The first and marker injection remain
/// gated by the adapter's capture and marker capabilities. Measured 2026-09-04:
/// two codex seats resumed while both were still `pending` then raced onto ONE
/// rollout and recorded the SAME id twice.
pub(crate) fn launch_token(_tool: ToolKind, stored: Option<String>) -> String {
    stored
        .filter(|id| !id.is_empty())
        .unwrap_or_else(launch::generate_uuid)
}

/// Must this seat's first turn be PASTED rather than baked into its argv?
///
/// `_run` appends the inline first message on the CREATE path alone; EVERY
/// resume composes with an empty prompt — the exact one, and equally the fresh
/// fallback a seat whose recorded id fails its store probe has to take. So the start
/// marker decides this, and the id must not: a codex seat resumed while its
/// `harness_session` is still `pending` takes that fallback, and gating on a
/// probeable id would leave it with NO user turn at all. It would then write no
/// rollout (see `launch::initial_prompt_for` for the measurement), so the
/// re-capture that `launching_capture` deliberately schedules for exactly those
/// pending slots would find nothing, and the seat would stay unresumable for
/// the rest of its life.
const fn launch_turn_is_pasted(tool: ToolKind, resuming_seat: bool) -> bool {
    resuming_seat && tool.adapter().input.paste_initial_on_resume
}

/// The gated, loud, DURABLE launch-prompt delivery.
fn deliver_launch_prompt(
    dir: &Path,
    server: &ServerId,
    agent: &Launching,
    prompt: &str,
    err: &mut impl Write,
) -> io::Result<()> {
    let model = agent.tool.adapter().input.model;
    let composed = agent.tool.adapter().input.composed;
    let reason = if deliver::wait_input_ready(
        server,
        &agent.pane,
        model,
        composed,
        LAUNCH_READY_POLLS,
    ) {
        // NO select-pane: `paste-buffer -t` writes to the NAMED pane, and
        // selecting mid-send routes the human's in-flight keystrokes into the
        // target — acute under lead-pair, where two agents share window 0.
        match deliver::stage_and_paste(
            server,
            &format!("ae-launch-{}", agent.slot),
            prompt.as_bytes(),
            &agent.pane,
        ) {
            // A bare Enter is not a submit: a booting TUI swallows it, and for
            // a seat resumed while its id is still `pending` this turn is the
            // ONLY thing that will ever create a rollout to capture. So the
            // press is PROVEN, and a turn left in the box falls through to the
            // durable failure below rather than passing as delivered.
            Ok(()) => match deliver::submit_staged(server, &agent.pane, model) {
                deliver::SubmitState::Submitted | deliver::SubmitState::Unknown(_) => return Ok(()),
                deliver::SubmitState::StillStaged => {
                    "submit UNCONFIRMED — the turn is staged unsent in the input box".to_owned()
                }
            },
            Err(failure) => format!("submit UNCONFIRMED ({failure:?}) — it may be staged unsent"),
        }
    } else {
        "input never reached a confirmed-ready state within 45s (still initializing, busy, modal, or unreadable)".to_owned()
    };
    let file = dir.join(format!("undelivered.launch-{}.txt", agent.slot));
    let preserved = write_private(&file, prompt).is_ok();
    let _ = crate::store::open(dir).append_event(&crate::tracked::event_line(
        &crate::tracked::EventFields {
            ts: crate::time::Timestamp::now(),
            actor: "ae",
            action: LAUNCH_FAILED_ACTION,
            target: &agent.slot,
            reference: "",
            actor_slot: "",
            actor_session: "",
            target_slot: &agent.slot,
            target_session: "",
            target_server: "",
            target_pane: "",
            target_session_uuid: "",
            caller_server: "",
            caller_pane: "",
            caller_session_uuid: "",
            identity_gap: "",
            summary: &format!(
                "launch prompt NOT delivered to {} ({}, pane {}): {reason}",
                agent.slot,
                agent.tool.as_str(),
                agent.pane
            ),
            body_file: "",
        },
    ));
    writeln!(
        err,
        "ae: LAUNCH PROMPT NOT DELIVERED to {} (pane {}): {reason}",
        agent.slot, agent.pane
    )?;
    if preserved {
        writeln!(err, "ae: the text is preserved at {}", file.display())?;
    }
    Ok(())
}

/// Wait, briefly, for the tool's process to replace the pane's shell.
fn wait_for_agent_start(server: &ServerId, pane: &str, tool: ToolKind) {
    if !tool.adapter().input.wait_for_process {
        return;
    }
    // The pane runs the CORE before it runs the tool: `_run` composes the
    // command and `exec`s it, so for a moment `pane_current_command` is ae's
    // own binary.
    let core = crate::shape::resolved_exe()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    for _ in 0..START_POLLS {
        let current = transport::observe_pane_probe(server, pane)
            .map(|probe| probe.command)
            .unwrap_or_default();
        // opencode's process reports as `opencode.exe` — its bun-built launcher.
        if current.strip_suffix(".exe").unwrap_or(&current) == tool.as_str() {
            return;
        }
        if !crate::watchdog::command_is_shell(&current) && (core.is_empty() || current != core) {
            return;
        }
        std::thread::sleep(START_POLL);
    }
}

/// The look `session` declares, or `None` when the server did not answer.
///
/// Read from the session rather than from the config: the look is the
/// SESSION's, and a helper that ran with a different config must not redress it.
/// A failed READ is not a default look either — standing one in would stamp a
/// window of a theme-off session in ae's colours, and the stamp would then hide
/// the mismatch from every later cycle.
pub(crate) fn look_of(server: &ServerId, session: &str) -> Option<crate::theme::Look> {
    let read = transport::observe_look(server, session)?;
    Some(crate::theme::Look::read(
        &read.icons,
        &read.palette,
        &read.drawn,
        &read.motion,
    ))
}

/// The monitor window's events pane, created if it is not already there.
pub(crate) fn ensure_events_pane(server: &ServerId, session: &str, dir: &Path) -> Option<String> {
    if let Some(existing) = monitor_pane(server, session, "_events") {
        mark_plumbing_window(server, &existing);
        return Some(existing);
    }
    let command = vec![dir.join("events-tail").display().to_string()];
    let pane = new_window_running(
        server,
        &format!("{}:99", tmux::session_target(session)),
        crate::theme::MONITOR_WINDOW,
        &command,
    )
    .or_else(|| {
        new_window_running(
            server,
            &tmux::session_target(session),
            crate::theme::MONITOR_WINDOW,
            &command,
        )
    })?;
    mark_plumbing_window(server, &pane);
    for (option, value) in [
        ("@ae_agent", "_events".to_owned()),
        (
            crate::theme::AGENT_LABEL_OPTION,
            crate::theme::agent_label("_events"),
        ),
    ] {
        let _ = transport::publish_option(server, tmux::OptionScope::Pane, &pane, option, &value);
    }
    let _ = transport::set_pane_title(server, &pane, "ae events");
    // The monitor window's borders are the LOOK's, so they are written by the
    // look and by nothing else: with `[workspace] theme = off` this window is
    // left exactly as the user's own tmux configuration draws it, like every
    // other window of the session.
    if let Some(look) = look_of(server, session) {
        stamp_window(
            server,
            &format!("{}:ae-monitor", tmux::session_target(session)),
            &look,
        );
    }
    let _ = transport::run_tmux_op(&argv(server, &Op::DisablePane { pane: &pane }));
    Some(pane)
}

/// Replace both monitor processes after a session directory moves.
///
/// Existing panes are respawned in place so window 99 keeps its pane ids and
/// layout. A monitor pane that vanished before rename reached it is recreated
/// through the ordinary start path. An enabled watchdog is not considered
/// rebound until its pidfile, process and stamped pane agree.
pub(crate) fn rebind_monitor_panes(
    root: &Path,
    server: &ServerId,
    session: &str,
    dir: &Path,
) -> Result<Option<u32>, String> {
    let events_command = vec![dir.join("events-tail").display().to_string()];
    if let Some(events) = monitor_pane(server, session, "_events") {
        let (respawned, _) = transport::run_tmux_op(&argv(
            server,
            &Op::RespawnPane {
                pane: &events,
                command: &events_command,
            },
        ));
        if !respawned {
            return Err(format!("could not respawn events pane {events}"));
        }
    } else if ensure_events_pane(server, session, dir).is_none() {
        return Err("could not recreate the events pane".to_owned());
    }

    if !watchdog_enabled_for_session(dir) {
        return Ok(None);
    }
    if let Some(watchdog) = monitor_pane(server, session, "_watchdog") {
        let command = vec![dir.join("watchdog").display().to_string()];
        let (respawned, _) = replace_watchdog_registration(dir, || {
            transport::run_tmux_op(&argv(
                server,
                &Op::RespawnPane {
                    pane: &watchdog,
                    command: &command,
                },
            ))
        });
        if !respawned {
            return Err(format!("could not respawn watchdog pane {watchdog}"));
        }
    } else {
        let tail = ["start".to_owned(), session.to_owned()];
        let (mut out, mut err) = (Vec::new(), Vec::new());
        match crate::watchdog_lifecycle::run(root, &tail, &mut out, &mut err) {
            Ok(0) => {}
            Ok(_) | Err(_) => {
                return Err(format!(
                    "could not recreate the watchdog pane ({})",
                    String::from_utf8_lossy(&err).trim()
                ));
            }
        }
    }
    crate::watchdog_lifecycle::await_running(server, session, dir)
        .map(Some)
        .ok_or_else(|| "watchdog did not publish a live pid within the start bound".to_owned())
}

/// Replace a watchdog after releasing its old registration.
fn replace_watchdog_registration<F>(dir: &Path, respawn: F) -> (bool, String)
where
    F: FnOnce() -> (bool, String),
{
    let old_pid = crate::watchdog_glue::read_pid(dir);
    if let Some(pid) = old_pid {
        let _ = crate::watchdog_glue::clear_pid(dir, pid);
    }
    respawn()
}

/// Whether the session's persisted settings ask for a watchdog.
///
/// The meta flag wins over config, as it does at launch. Otherwise the global
/// config recorded in meta and the session's resolved local overlay recreate
/// the launch-time layering. No setting means the documented default: on.
pub(crate) fn watchdog_enabled_for_session(dir: &Path) -> bool {
    let bytes = meta::read_bytes(dir).unwrap_or_default();
    let value = |key: &str| crate::lifecycle::meta_value(&bytes, key);
    let recorded = [value("watchdog"), value("loop")]
        .into_iter()
        .find(|value| !value.is_empty());
    let configured = if recorded.is_none() {
        let global = value("config");
        let global = (!global.is_empty()).then(|| PathBuf::from(global));
        let origin = value("origin");
        let local = config::local_overlay(dir, &origin);
        let values =
            config::read_workspace_keys(global.as_deref(), local.as_deref(), &["watchdog", "loop"]);
        values.into_iter().flatten().next()
    } else {
        None
    };
    !matches!(
        recorded
            .or(configured)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "false" | "no" | "off" | "0"
    )
}

/// Mark the monitor pane's window as ae-owned plumbing for status rendering.
fn mark_plumbing_window(server: &ServerId, pane: &str) {
    let _ = transport::publish_option(
        server,
        tmux::OptionScope::Window,
        pane,
        crate::theme::WINDOW_PLUMBING_OPTION,
        "1",
    );
}

/// The watchdog pane, split ABOVE the events pane so the visual order stays
/// watchdog-on-top / events-below.
fn start_watchdog_pane(shape: &Session, dir: &Path, server: &ServerId, anchor: &str) {
    if !watchdog_enabled_for_session(dir) {
        return;
    }
    if monitor_pane(server, &shape.name, "_watchdog").is_some() {
        return;
    }
    let command = vec![dir.join("watchdog").display().to_string()];
    let (succeeded, stdout) = transport::run_tmux_op(&argv(
        server,
        &Op::SplitWindow {
            target: anchor,
            work_dir: "",
            split: Split::VerticalBefore,
            command: &command,
        },
    ));
    let Some(pane) = interpret_pane_id(succeeded, &stdout) else {
        return;
    };
    for (option, value) in [
        ("@ae_agent", "_watchdog".to_owned()),
        (
            crate::theme::AGENT_LABEL_OPTION,
            crate::theme::agent_label("_watchdog"),
        ),
    ] {
        let _ = transport::publish_option(server, tmux::OptionScope::Pane, &pane, option, &value);
    }
    let _ = transport::set_pane_title(server, &pane, "ae watchdog");
    let _ = transport::run_tmux_op(&argv(server, &Op::DisablePane { pane: &pane }));
    let _ = meta::rewrite(dir, "watchdog", Some("true"));
}

/// The Telegram bridge, revived if `[telegram] enabled` asks for one.
fn autostart_telegram(
    env: &Env,
    shape: &Session,
    dir: &Path,
    server: &ServerId,
    err: &mut impl Write,
) {
    let mut paths = crate::telegram::bridge::Paths::under(&env.home);
    if let Some(global) = &env.global {
        paths.config.clone_from(global);
    }
    let _ = crate::telegram_lifecycle::autostart(&paths, server, &shape.name, dir, err);
}

/// Create a detached window running `command`, and report its pane id.
fn new_window_running(
    server: &ServerId,
    target: &str,
    name: &str,
    command: &[String],
) -> Option<String> {
    let (succeeded, stdout) = transport::run_tmux_op(&argv(
        server,
        &Op::NewWindow {
            target,
            name,
            work_dir: "",
            command,
        },
    ));
    interpret_pane_id(succeeded, &stdout)
}

/// The pane of the monitor window stamped `agent`, if it is there.
fn monitor_pane(server: &ServerId, session: &str, agent: &str) -> Option<String> {
    transport::observe_agents(server, session)?
        .into_iter()
        .find(|pane| pane.agent == agent)
        .map(|pane| pane.pane)
}

/// The command that actually attaches to `session` on `server`.
fn attach_hint(server: &ServerId, session: &str) -> String {
    let session = format!("\"{}\"", tmux::session_target(session));
    match server {
        ServerId::Ambient => format!("tmux attach -t {session}"),
        ServerId::Selected(crate::meta::Selector::Name(name)) => {
            format!("tmux -L {} attach -t {session}", paste_safe(name))
        }
        ServerId::Selected(crate::meta::Selector::Socket(path)) => format!(
            "tmux -S {} attach -t {session}",
            paste_safe(&path.display().to_string())
        ),
    }
}

/// The command that attaches to `server` and lets tmux choose its most recent
/// session.
pub(crate) fn server_attach_hint(server: &ServerId) -> String {
    match server {
        ServerId::Ambient => "tmux attach".to_owned(),
        ServerId::Selected(crate::meta::Selector::Name(name)) => {
            format!("tmux -L {} attach", paste_safe(name))
        }
        ServerId::Selected(crate::meta::Selector::Socket(path)) => {
            format!("tmux -S {} attach", paste_safe(&path.display().to_string()))
        }
    }
}

/// `word` as it can be pasted into a shell: bare when nothing in it is
/// significant there, quoted when anything is.
fn paste_safe(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '@'));
    if plain {
        word.to_owned()
    } else {
        crate::launch::shell_quote(word)
    }
}

/// Whether an attach is already at its destination or needs a tmux focus
/// operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttachAction {
    /// The caller's pane is already in the target session.
    InPlace,
    /// Tmux must attach or switch the caller's client.
    Focus(tmux::FocusVerb),
    /// The caller is inside another tmux server and must attach explicitly.
    Hint,
}

/// Decide the attach action from already-observed facts.
fn attach_action(
    inside: bool,
    same_server: bool,
    caller_session: Option<&str>,
    target: &str,
) -> AttachAction {
    if !inside {
        AttachAction::Focus(tmux::FocusVerb::AttachSession)
    } else if !same_server {
        AttachAction::Hint
    } else if caller_session == Some(target) {
        AttachAction::InPlace
    } else {
        AttachAction::Focus(tmux::FocusVerb::SwitchClient)
    }
}

/// Attach or switch the client to `session`, unless the caller is already in
/// it, and report the exit code.
fn attach(server: &ServerId, env: &Env, session: &str, out: &mut impl Write) -> io::Result<u8> {
    attach_on(
        server,
        env.caller_server.as_ref(),
        env.inside_tmux,
        session,
        out,
    )
}

/// Focus `session` only when the caller can reach its recorded server; a
/// foreign-server caller gets an explicit attach command instead.
pub(crate) fn attach_on(
    server: &ServerId,
    caller_server: Option<&ServerId>,
    inside: bool,
    session: &str,
    out: &mut impl Write,
) -> io::Result<u8> {
    let caller_session = if inside {
        caller_server.and_then(|caller| {
            crate::doors::calling_pane_id().and_then(|pane| {
                transport::observe_viewer(caller, &pane).and_then(|viewer| viewer.session)
            })
        })
    } else {
        None
    };
    let same_server = caller_server.is_some_and(|caller| {
        let mut sockets = crate::SocketPaths::asking(transport::observe_socket_path);
        sockets.proven_same(caller, server)
    });
    match attach_action(inside, same_server, caller_session.as_deref(), session) {
        AttachAction::InPlace => {
            writeln!(out, "you are in '{session}'")?;
            Ok(0)
        }
        AttachAction::Focus(verb) => Ok(transport::focus(server, verb, session)),
        AttachAction::Hint => {
            writeln!(
                out,
                "Session '{session}' is on another tmux server. Attach with: {}",
                attach_hint(server, session)
            )?;
            Ok(0)
        }
    }
}

// ---------------------------------------------------------------------------
// filesystem and meta reads
// ---------------------------------------------------------------------------

/// Carry `server` as the pair the existing whole-meta build publication will
/// write. This changes only the attempt's in-memory destination.
fn set_env_server(env: &mut Env, server: &ServerId) {
    match server {
        ServerId::Ambient => {
            env.server_kind.clear();
            env.server_value.clear();
        }
        ServerId::Selected(meta::Selector::Name(name)) => {
            "name".clone_into(&mut env.server_kind);
            env.server_value.clone_from(name);
        }
        ServerId::Selected(meta::Selector::Socket(path)) => {
            "socket".clone_into(&mut env.server_kind);
            env.server_value = path.display().to_string();
        }
    }
}

/// Prove the destination does not already hold this exact name before a
/// stopped session can be rebuilt there.
fn destination_is_absent(
    server: &ServerId,
    session: &str,
    err: &mut impl Write,
) -> io::Result<bool> {
    if transport::session_exists(server, session) {
        writeln!(
            err,
            "Error: tmux session '{session}' exists but is not an ae session."
        )?;
        Ok(false)
    } else {
        // A cold destination is expected: unlike the recorded-server probe,
        // this is only a collision guard. `new-session` creates a named or
        // socket-selected server and remains the final atomic race check.
        Ok(true)
    }
}

/// Whether two existing directories canonicalise to the same path. A failed
/// canonicalisation proves nothing and therefore never establishes ownership.
#[allow(
    clippy::disallowed_methods,
    reason = "a door: legacy ownership proof canonicalises both the stamped AE_HOME and this state root — see clippy.toml"
)]
fn existing_directories_match(one: &Path, other: &Path) -> bool {
    match (std::fs::canonicalize(one), std::fs::canonicalize(other)) {
        (Ok(one), Ok(other)) => one == other,
        _ => false,
    }
}

/// Whether `path` is any node at all, LINK INCLUDED.
fn node_exists(path: &Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the tombstone and meta-presence guards, which must see a dangling link as standing — see clippy.toml"
    )]
    let read = std::fs::symlink_metadata(path);
    read.is_ok()
}

/// Whether `path` resolves (following links) to a directory.
fn dir_exists(path: &Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: proves a recorded origin or an existing working copy is a directory — see clippy.toml"
    )]
    let read = std::fs::metadata(path);
    read.is_ok_and(|meta| meta.is_dir())
}

/// Whether two spellings name the same directory.
fn same_directory(one: &str, other: &str) -> bool {
    match (
        canonical_directory(Path::new(one)),
        canonical_directory(Path::new(other)),
    ) {
        (Some(one), Some(other)) => one == other,
        _ => false,
    }
}

/// One existing directory, canonicalised.
#[allow(
    clippy::disallowed_methods,
    reason = "a door: --dir and explicit-origin resume checks resolve canonical directories — see clippy.toml"
)]
fn canonical_directory(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path)
        .ok()
        .filter(|resolved| dir_exists(resolved))
}

/// One meta value of the session at `dir`, or `None`.
fn meta_value(dir: &Path, key: &str) -> Option<String> {
    let bytes = meta::read_bytes(dir).ok()?;
    meta::first_value(&bytes, key).map(|value| String::from_utf8_lossy(value).into_owned())
}

/// The session directory at 0700.
fn create_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// Write `text` at 0600 — the same material as the pane content.
fn write_private(path: &Path, text: &str) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut file = std::fs::File::create(path)?;
    file.write_all(text.as_bytes())?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

/// The RESUME'S absence verdict for `session` on `server`, and the line to
/// print when there is none.
///
/// The one place the boot-time proof is reached from a launch. `stop`, `end`
/// and `compact` keep [`transport::verify_session_absent`], whose verdict this
/// one can only WIDEN: every answer it gives beyond the strict one rests on
/// evidence ae itself wrote, and every gap in that evidence is
/// [`tmux::StopProbe::Unknown`].
fn resume_absence(
    server: &ServerId,
    session: &str,
    dir: &Path,
) -> (tmux::StopProbe, Option<String>) {
    let probe = transport::probe_absence(server, session);
    let evidence = crate::inventory::last_live(dir);
    let boot = crate::doors::boot_time(crate::shape::current());
    let now = crate::time::Timestamp::now().epoch();
    (
        tmux::classify_absence(probe, evidence, boot, now),
        tmux::unproven_reason(&server_label(server), probe, evidence, boot, now),
    )
}

/// How a server is named in a refusal — its socket path, or `-L <name>`.
fn server_label(server: &ServerId) -> String {
    match server {
        ServerId::Ambient => "the ambient server".to_owned(),
        ServerId::Selected(meta::Selector::Socket(path)) => path.display().to_string(),
        ServerId::Selected(meta::Selector::Name(name)) => format!("-L {name}"),
    }
}

/// Undo a launch that failed after the tmux session existed, and SAY SO.
fn rollback_launch(
    shape: &Session,
    dir: &Path,
    server: &ServerId,
    why: &str,
    err: &mut impl Write,
) -> crate::Result<u8> {
    writeln!(err, "ae: launch failed — rolling back '{}'.", shape.name)?;
    writeln!(err, "{why}")?;
    let _ = kill_session(server, &shape.name);
    rollback_dir(shape, dir, err)?;
    Ok(EXIT_FAILED)
}

/// Remove the session directory — but ONLY when this attempt created it.
fn rollback_dir(shape: &Session, dir: &Path, err: &mut impl Write) -> io::Result<()> {
    let Some(root) = dir.parent().map(|p| p.display().to_string()) else {
        return Ok(());
    };
    if !shape.dir_created {
        writeln!(
            err,
            "ae: session state KEPT — it predates this attempt; retry with 'ae {}'.",
            shape.name
        )?;
        return Ok(());
    }
    if !name::is_direct_child(&root, &dir.display().to_string()) {
        writeln!(
            err,
            "ae: session state KEPT — '{}' is not a direct child of {root}; refusing to delete.",
            dir.display()
        )?;
        return Ok(());
    }
    let _ = std::fs::remove_dir_all(dir);
    writeln!(
        err,
        "ae: session state removed — this launch created it; nothing left to reattach to."
    )
}

/// A recursive copy of `from` into `to`, minus one thing.
///
/// # Errors
///
/// The first entry that could not be copied.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::create_dir_all(to)?;
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the --copy mode's recursive copy of the caller's working tree — see clippy.toml"
    )]
    let entries = std::fs::read_dir(from)?;
    for entry in entries {
        let entry = entry?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: classifies each entry of the tree being copied — see clippy.toml"
        )]
        let kind = std::fs::symlink_metadata(&source)?;
        if kind.is_symlink() {
            let link = std::fs::read_link(&source)?;
            let _ = std::fs::remove_file(&target);
            std::os::unix::fs::symlink(link, &target)?;
        } else if kind.is_dir() {
            copy_tree(&source, &target)?;
            std::fs::set_permissions(&target, kind.permissions())?;
        } else if kind.is_file() {
            std::fs::copy(&source, &target)?;
            std::fs::set_permissions(
                &target,
                std::fs::Permissions::from_mode(kind.permissions().mode()),
            )?;
        }
        // Anything else — a fifo, a socket, a device — is deliberately skipped:
        // it is not project content, and recreating one needs `mknod`.
    }
    Ok(())
}

/// Cap `events.jsonl` to its newest lines on resume.
fn trim_events(dir: &Path) {
    crate::store::open(dir).retain_events(EVENTS_KEEP);
}

/// One spawned seat recovered from a resuming session's own meta.
struct Spawned {
    slot: String,
    name: String,
    profile: String,
    /// The recorded `client.<slot>` carried forward, if the seat records one.
    /// A spawn never writes this row, so a label here is a hand edit — but a
    /// carried label is evidence, and an Invalid row refuses rather than
    /// laundering toward Missing.
    client: Option<String>,
    binary: String,
    harness_session: String,
    config_home: Option<String>,
    config_home_base: Option<String>,
}

/// The `spawned.<n>` seats a resuming session's meta records, in slot order.
///
/// # Errors
///
/// An Invalid recorded client row, with its remedy — the meta rewrite would
/// otherwise launder it.
fn spawned_entries(dir: &Path, session: &str) -> Result<Vec<Spawned>, String> {
    let Ok(bytes) = meta::read_bytes(dir) else {
        return Ok(Vec::new());
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let parsed = Meta::parse(&text);
    let mut out: Vec<Spawned> = Vec::new();
    for entry in parsed
        .roster()
        .iter()
        .filter(|entry| entry.slot.starts_with("spawned."))
    {
        let client = match &entry.client {
            crate::meta::RecordedClient::Missing => None,
            crate::meta::RecordedClient::Label(label) => Some(label.clone()),
            crate::meta::RecordedClient::Invalid => {
                return Err(invalid_client_refusal(session, &entry.name, &entry.slot));
            }
        };
        out.push(Spawned {
            slot: entry.slot.clone(),
            name: entry.name.clone(),
            profile: entry.profile.clone().unwrap_or_default(),
            client,
            binary: entry.binary.clone().unwrap_or_default(),
            harness_session: entry.harness_session.clone().unwrap_or_default(),
            config_home: entry.config_home.record_value(),
            config_home_base: entry.config_home_base.record_value(),
        });
    }
    out.sort_by_key(|entry| {
        entry
            .slot
            .rsplit_once('.')
            .and_then(|(_, index)| index.parse::<u32>().ok())
            .unwrap_or(u32::MAX)
    });
    Ok(out)
}

/// What the capture pass needs from each launched agent.
fn launching_capture(launching: &[Launching], resuming: bool) -> Vec<capture::Target> {
    launching
        .iter()
        .filter(|agent| !agent.pane.is_empty())
        .filter(|agent| agent.tool.adapter().capture.is_needed())
        // On a resume, only the slots still `pending` are re-captured: capture
        // is post-launch and may have failed a previous attempt, and without a
        // retry those slots stay pending forever.
        .filter(|agent| !resuming || agent.session_id == PENDING)
        .map(|agent| capture::Target {
            slot: agent.slot.clone(),
            tool: agent.tool,
            pane: agent.pane.clone(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// what the pane surfaces borrow
// ---------------------------------------------------------------------------

/// The tmux server the session at `dir` records, or the ambient one.
#[must_use]
pub fn recorded_server(dir: &Path) -> ServerId {
    let Ok(bytes) = meta::read_bytes(dir) else {
        return ServerId::Ambient;
    };
    match Meta::parse(&String::from_utf8_lossy(&bytes)).server_selector() {
        ServerSelector::Positive(selector) => ServerId::Selected(selector),
        ServerSelector::Missing | ServerSelector::Ambiguous => ServerId::Ambient,
    }
}

/// The refusal a lifecycle caller prints for a record whose server pointer does
/// not point — spelled once, because three commands share it and a message that
/// does not name the ROWS leaves an operator with nothing to fix.
pub const AMBIGUOUS_SERVER: &str = "records a tmux server ae cannot resolve — its meta rows 'tmux_server_kind' / 'tmux_server' do not name exactly one server";

/// The tmux server the session at `dir` records, REFUSING an ambiguous record.
#[must_use]
pub fn recorded_server_resolved(dir: &Path) -> Option<ServerId> {
    let Ok(bytes) = meta::read_bytes(dir) else {
        return Some(ServerId::Ambient);
    };
    match Meta::parse(&String::from_utf8_lossy(&bytes)).server_selector() {
        ServerSelector::Positive(selector) => Some(ServerId::Selected(selector)),
        ServerSelector::Missing => Some(ServerId::Ambient),
        ServerSelector::Ambiguous => None,
    }
}

/// A sibling session's meta directory, given one session's.
#[must_use]
pub fn sibling_session_dir(dir: &Path, session: &str) -> PathBuf {
    dir.parent()
        .map_or_else(|| PathBuf::from(session), |root| root.join(session))
}

/// Every ae session currently running on the same server as the one at `dir`.
#[must_use]
pub fn running_ae_sessions(dir: &Path) -> Vec<String> {
    use crate::inventory::Discovery as _;
    let server = recorded_server(dir);
    transport::Tmux
        .enumerate(&server)
        .map(|found| {
            found
                .into_iter()
                .filter(|session| session.marker.is_some())
                .map(|session| session.name)
                .collect()
        })
        .unwrap_or_default()
}

/// A path as the raw bytes the git leg takes — one `OsStr` argv element, so a
/// non-UTF-8 working tree survives intact and there is nothing to inject.
fn path_bytes(path: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt as _;
    path.as_os_str().as_bytes()
}

/// What `--from`'s preflight proved about the parent archive.
pub struct FromProof {
    /// The canonical archive UUID.
    pub id: String,
    /// Its handover count, as the preflight printed it.
    pub handover: String,
    /// Its pending count.
    pub pending: String,
}

/// Prove the archive `raw_uuid` names is inheritable, BEFORE any side effect.
///
/// # Errors
///
/// The refusal, ready for stderr.
fn from_preflight(root: &Path, raw_uuid: &str) -> Result<FromProof, String> {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = crate::archive::from::run(root, raw_uuid, &mut out, &mut err)
        .map_err(|why| format!("Error: the archive preflight could not run ({why})."))?;
    if code != 0 {
        return Err(String::from_utf8_lossy(&err).trim_end().to_owned());
    }
    let proof = String::from_utf8_lossy(&out).trim_end().to_owned();
    let mut fields = proof.split('\t');
    Ok(FromProof {
        id: fields.next().unwrap_or_default().to_owned(),
        handover: fields.next().unwrap_or_default().to_owned(),
        pending: fields.next().unwrap_or_default().to_owned(),
    })
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "a fixture builds and inspects a real directory; the capability boundary is \
              about what PRODUCT code may reach"
)]
mod tests {
    use super::{
        AttachAction, EVENTS_KEEP, ExpectedLaunch, ExpectedState, ToolKind, attach_action,
        expected_launch_argv, launch_token, launch_turn_is_pasted, layout_argvs,
        lead_pair_policy_argvs, parse_plan, replace_watchdog_registration, server_attach_hint,
        trim_events,
    };
    use crate::inventory::ServerId;
    use std::fmt::Write as _;
    use std::path::PathBuf;

    fn layout_words(layout: &str, panes: &[String], workers: usize) -> Vec<Vec<String>> {
        layout_argvs(&ServerId::Ambient, layout, "named", panes, workers)
            .iter()
            .map(|command| command.as_args().to_vec())
            .collect()
    }

    fn pair_policy_words(layout: &str, main_pane: &str) -> Vec<Vec<String>> {
        lead_pair_policy_argvs(&ServerId::Ambient, layout, main_pane)
            .iter()
            .map(|command| command.as_args().to_vec())
            .collect()
    }

    #[test]
    fn normal_expected_launch_argv_cannot_enable_the_debug_pre_lock_marker() {
        let preamble = crate::entry::Preamble::default();
        let user = ["orchestrator".to_owned()];
        let expected = ExpectedLaunch::new(
            ExpectedState::AbsentCanonical,
            ServerId::Ambient,
            "1".to_owned(),
            "2".to_owned(),
            "/dev/ttys004".to_owned(),
            "3".to_owned(),
            4,
        );
        let argv = expected_launch_argv(&preamble, &user, &expected);
        assert!(!argv.iter().any(|word| word == "--test-pre-lock-marker"));

        #[cfg(debug_assertions)]
        {
            let mut opted_in = expected;
            opted_in.enable_test_pre_lock_marker();
            let argv = expected_launch_argv(&preamble, &user, &opted_in);
            assert_eq!(
                argv.iter()
                    .filter(|word| word.as_str() == "--test-pre-lock-marker")
                    .count(),
                1
            );
            let marker_at = argv
                .iter()
                .position(|word| word == "--test-pre-lock-marker");
            let separator = argv.iter().position(|word| word == "--");
            assert!(marker_at < separator, "{argv:?}");
        }
    }

    #[test]
    fn the_lead_pair_sets_equal_width_before_selecting_the_main_vertical_layout() {
        let panes = ["%0".to_owned(), "%1".to_owned()];
        assert_eq!(
            pair_policy_words("lead-pair", &panes[0]),
            vec![
                vec!["set-window-option", "-t", "%0", "main-pane-width", "50%"],
                vec![
                    "if-shell",
                    "-F",
                    "-t",
                    "%0",
                    "#{==:#{window_zoomed_flag},0}",
                    "select-layout -t %0 main-vertical"
                ],
                vec![
                    "set-hook",
                    "-w",
                    "-t",
                    "%0",
                    "window-resized",
                    "if-shell -F -t %0 '#{==:#{window_zoomed_flag},0}' 'select-layout -t %0 main-vertical'"
                ],
            ]
        );
        assert!(pair_policy_words("vertical", "%0").is_empty());

        let panes = [
            "%0".to_owned(),
            "%1".to_owned(),
            "%2".to_owned(),
            "%3".to_owned(),
        ];
        assert_eq!(
            layout_words("lead-pair", &panes, 3),
            vec![vec!["select-layout", "-t", "%2", "even-vertical"]],
            "extra standing workers keep their separate stacked window"
        );
    }

    #[test]
    fn every_non_pair_layout_keeps_its_existing_layout_program() {
        let panes = ["%0".to_owned(), "%1".to_owned(), "%2".to_owned()];
        assert_eq!(
            layout_words("lead-solo", &panes, 2),
            vec![vec!["select-layout", "-t", "%1", "even-vertical"]]
        );
        assert_eq!(
            layout_words("vertical", &panes, 2),
            vec![vec!["select-layout", "-t", "=named:", "even-horizontal"]]
        );
        assert_eq!(
            layout_words("horizontal", &panes, 2),
            vec![vec!["select-layout", "-t", "=named:", "even-vertical"]]
        );
    }

    #[test]
    fn automatic_upgrade_hooks_follow_reattach_validation_and_fresh_launch_completion() {
        let source = include_str!("session_launch.rs");
        let reattach = source
            .split_once("// ---- a session that is already running is reattached")
            .and_then(|(_, tail)| tail.split_once("if seeds_orchestrator_config"))
            .map(|(body, _)| body)
            .expect("running-session branch");
        let validated = reattach
            .find("observe_agents")
            .expect("ae-session validation");
        let released = reattach
            .find("drop(lifecycle.take())")
            .expect("lifecycle guard release");
        let scheduled = reattach
            .find("autoupgrade::schedule")
            .expect("reattach scheduling edge");
        let attach = reattach.find("if !env.attach").expect("attach decision");
        assert!(
            validated < released && released < scheduled && scheduled < attach,
            "{reattach}"
        );

        let completed = source
            .split_once("// ---- the Telegram bridge ----")
            .and_then(|(_, tail)| tail.split_once("// ---------------------------------------------------------------------------"))
            .map(|(body, _)| body)
            .expect("completed launch tail");
        let selected = completed
            .find("Op::SelectPane")
            .expect("final pane selection");
        let scheduled = completed
            .find("autoupgrade::schedule")
            .expect("fresh-launch scheduling edge");
        let attach = completed.find("if !env.attach").expect("attach decision");
        assert!(selected < scheduled && scheduled < attach, "{completed}");
    }

    #[test]
    fn settings_launch_preserves_floor_then_locks_before_authoritative_proof_and_effects() {
        let source = include_str!("session_launch.rs");
        let launch = source
            .split_once("let proposed_server = env.server();")
            .map(|(_, tail)| tail)
            .expect("launch policy");
        let floor = launch.find("floor_refusal").expect("tmux floor");
        let locked = launch.find("crate::store::lock").expect("lifecycle lock");
        let meta_present = launch
            .find("let meta_present = if expected_launch.is_some()")
            .expect("locked state read");
        let clicker = launch.find("expected.check_action").expect("action proof");
        let census = launch
            .find("prove_absent_start")
            .expect("raw-role census and absence proof");
        let seeded = launch
            .find("seed_default_config")
            .expect("first-run config seed");
        let built = launch.find("build(").expect("existing launch build");
        assert!(
            floor < locked
                && locked < meta_present
                && meta_present < clicker
                && clicker < census
                && census < seeded
                && seeded < built,
            "{launch}"
        );

        let resume_guard = launch
            .find("expected_launch.is_some_and(ExpectedLaunch::is_resume)")
            .expect("expected Resume lock condition");
        let stopped = launch
            .find("prove_stopped_role")
            .expect("locked stopped-role proof");
        assert!(resume_guard < locked && locked < stopped, "{launch}");
    }
    fn scratch(tag: &str) -> PathBuf {
        let dir = PathBuf::from(format!("/tmp/ae-launch-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn watchdog_registration_is_cleared_before_a_fast_replacement_publishes() {
        let dir = scratch("watchdog-replacement-order");
        std::fs::write(crate::watchdog_glue::pidfile(&dir), "41\n").unwrap();

        let (replaced, _) = replace_watchdog_registration(&dir, || {
            assert_eq!(
                crate::watchdog_glue::read_pid(&dir),
                None,
                "the old registration must be gone before respawn can publish"
            );
            std::fs::write(crate::watchdog_glue::pidfile(&dir), "42\n").unwrap();
            (true, String::new())
        });

        assert!(replaced);
        assert_eq!(
            crate::watchdog_glue::read_pid(&dir),
            Some(42),
            "the replacement registration survives"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_public_launch_parser_owns_directory_and_attach_flags() {
        let args = ["named", "--dir", "/repo", "--no-attach", "--worktree"].map(str::to_owned);
        let plan = parse_plan(&args).expect("public launch flags");
        assert_eq!(plan.name.as_deref(), Some("named"));
        assert_eq!(plan.dir.as_deref(), Some("/repo"));
        assert_eq!(plan.attach, Some(false));
        assert_eq!(plan.mode, Some(super::Mode::Git));

        assert!(parse_plan(&["--dir".to_owned()]).is_err());
        assert!(parse_plan(&["--dir=".to_owned()]).is_err());
        assert!(parse_plan(&["--attach".to_owned()]).is_err());
        assert!(
            parse_plan(&[
                "--dir=/one".to_owned(),
                "--dir".to_owned(),
                "/two".to_owned(),
            ])
            .is_err()
        );
    }

    #[test]
    fn the_public_launch_parser_owns_general_and_sugar_seat_profiles() {
        let plan = parse_plan(
            &[
                "named",
                "--seat",
                "builder=terrax",
                "--lead",
                "solx",
                "--colead=astrax",
            ]
            .map(str::to_owned),
        )
        .expect("seat profile flags");
        assert_eq!(
            plan.seat_profiles,
            [
                ("builder".to_owned(), "terrax".to_owned()),
                ("lead".to_owned(), "solx".to_owned()),
                ("colead".to_owned(), "astrax".to_owned()),
            ]
        );

        for args in [
            vec!["--seat", "lead=one", "--seat=lead=two"],
            vec!["--lead", "one", "--seat", "lead=two"],
            vec!["--seat"],
            vec!["--seat", "lead"],
            vec!["--seat", "=profile"],
            vec!["--seat", "lead="],
            vec!["--colead"],
        ] {
            assert!(parse_plan(&args.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err());
        }
    }

    #[test]
    fn the_public_launch_parser_owns_the_solo_roster_override() {
        let plan = parse_plan(&["named", "--solo", "--lead", "solx"].map(str::to_owned))
            .expect("a solo launch with a lead profile override");
        assert_eq!(plan.workers.as_deref(), Some(""));
        assert_eq!(plan.seat_profiles, [("lead".to_owned(), "solx".to_owned())]);

        for args in [
            ["named", "--solo", "--colead", "astrax"],
            ["named", "--colead", "astrax", "--solo"],
        ] {
            assert_eq!(
                parse_plan(&args.map(str::to_owned)),
                Err(
                    "Error: '--solo' starts the lead alone; drop --colead/--seat <worker>."
                        .to_owned()
                )
            );
        }
    }

    #[test]
    fn switching_to_the_session_already_owning_the_caller_is_in_place() {
        assert_eq!(
            attach_action(true, true, Some("inside"), "inside"),
            AttachAction::InPlace
        );
        assert_eq!(
            attach_action(true, true, Some("inside"), "other"),
            AttachAction::Focus(crate::tmux::FocusVerb::SwitchClient)
        );
        assert_eq!(
            attach_action(false, false, Some("inside"), "inside"),
            AttachAction::Focus(crate::tmux::FocusVerb::AttachSession)
        );
        assert_eq!(
            attach_action(true, false, Some("inside"), "inside"),
            AttachAction::Hint
        );
    }

    #[test]
    fn fleet_attach_hints_keep_the_selected_server_spelling() {
        use crate::meta::Selector;

        assert_eq!(
            server_attach_hint(&ServerId::Selected(Selector::Name("ae".to_owned()))),
            "tmux -L ae attach"
        );
        assert_eq!(
            server_attach_hint(&ServerId::Selected(Selector::Socket("/tmp/ae.sock".into()))),
            "tmux -S /tmp/ae.sock attach"
        );
    }

    /// EVERY codex resume needs the turn pasted, including the one whose id is
    /// still `pending` and which therefore launches by the fresh fallback.
    #[test]
    fn every_codex_resume_gets_the_turn_pasted_and_no_fresh_start_does() {
        assert!(launch_turn_is_pasted(ToolKind::Codex, true));
        assert!(
            !launch_turn_is_pasted(ToolKind::Codex, false),
            "a create bakes the turn into argv; pasting it too would double it"
        );
        for tool in [
            ToolKind::Claude,
            ToolKind::Gemini,
            ToolKind::Agy,
            ToolKind::Grok,
            ToolKind::OpenCode,
            ToolKind::Unknown,
        ] {
            assert!(!launch_turn_is_pasted(tool, true), "{tool:?}");
        }
    }

    /// A seat KEEPS its launch id across a resume; only a seat without one is
    /// given a fresh one. Capture capability never changes that seat identity.
    #[test]
    fn a_resumed_seat_keeps_the_launch_id_that_guards_its_identity() {
        assert_eq!(
            launch_token(ToolKind::Codex, Some("tok-1".to_owned())),
            "tok-1",
            "dropping it leaves the capture matching by working directory alone"
        );
        let minted = launch_token(ToolKind::Codex, None);
        assert!(!minted.is_empty() && minted != "tok-1");
        assert!(
            !launch_token(ToolKind::Codex, Some(String::new())).is_empty(),
            "an empty row is no token at all: mint one"
        );
        for tool in [ToolKind::Claude, ToolKind::Grok, ToolKind::Unknown] {
            assert_eq!(
                launch_token(tool, Some("tok-1".to_owned())),
                "tok-1",
                "{tool:?}"
            );
            assert!(
                !launch_token(tool, None).is_empty(),
                "every seat gets a meta guard: {tool:?}"
            );
        }
    }

    /// A resume caps `events.jsonl` at its NEWEST lines, and the cut falls
    /// exactly at the retention boundary.
    #[test]
    fn a_trim_keeps_the_newest_events_and_cuts_at_the_boundary() {
        let dir = scratch("trim");
        let seeded = EVENTS_KEEP + 200;
        let mut text = String::new();
        for index in 1..=seeded {
            let _ = writeln!(text, "{{\"seq\":{index}}}");
        }
        std::fs::write(dir.join("events.jsonl"), &text).unwrap();

        trim_events(&dir);

        let kept = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
        let lines: Vec<&str> = kept.lines().collect();
        assert_eq!(lines.len(), EVENTS_KEEP);
        assert_eq!(lines.first().copied(), Some("{\"seq\":201}"));
        assert_eq!(
            lines.last().copied(),
            Some(format!("{{\"seq\":{seeded}}}").as_str())
        );
        assert!(!kept.contains("{\"seq\":200}"), "the boundary line is cut");
        // The temp the rename came from does not survive the operation.
        let residue: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(&format!("{}.trim.", crate::store::EVENTS)))
            .collect();
        assert!(residue.is_empty(), "no trim temp is left: {residue:?}");

        // A log already under the cap is left exactly as it was.
        std::fs::write(dir.join("events.jsonl"), "{\"seq\":1}\n").unwrap();
        trim_events(&dir);
        assert_eq!(
            std::fs::read_to_string(dir.join("events.jsonl")).unwrap(),
            "{\"seq\":1}\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DEFEATING MUTATION: delete the observed-model enumeration from
    /// `meta_document` and this test goes RED. The document is the WHOLE meta,
    /// so a pair not enumerated here is deleted by the first resume — before
    /// any resume could honor it.
    #[test]
    fn a_resume_document_carries_the_observed_model_pair_forward() {
        let root = scratch("observed-model-document");
        let home = root.join("home");
        let dir = home.join("sessions").join("s");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meta"),
            "schema=2\nmeta_version=2\nmode=local\nsession=s\nsession_id=SID\ncreated=1\n\
             started=1\nwork_dir=/w\norigin=/o\nseat.main=lead\nprofile.main=fable5\n\
             agent_bin.main=claude\nharness_session.main=sid\nlaunch_id.main=L1\n\
             observed_model.main=Opus 5 (1M context)\nobserved_model_pin.main=fable\n",
        )
        .unwrap();
        let env = super::Env {
            home: home.clone(),
            cwd: PathBuf::from("/w"),
            global: None,
            local: None,
            server_kind: String::new(),
            server_value: String::new(),
            caller_server: None,
            inside_tmux: false,
            attach: true,
            core: None,
            core_version: None,
            no_autostart: true,
            test_pre_lock_marker: None,
        };
        let shape = super::Session {
            name: "s".to_owned(),
            mode: super::Mode::Local,
            work_dir: PathBuf::from("/w"),
            origin: PathBuf::from("/o"),
            layout: "vertical".to_owned(),
            look: crate::theme::Look::DEFAULT,
            resuming: true,
            dir_created: false,
        };
        let launching = [super::Launching {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: "fable5".to_owned(),
            client: None,
            binary: "claude".to_owned(),
            tool: ToolKind::Claude,
            session_id: "sid".to_owned(),
            config_home: None,
            config_home_base: None,
            launch_id: "L1".to_owned(),
            pane: "%0".to_owned(),
            command_snapshot: None,
        }];
        let watchdog = super::WatchdogFacts {
            meta_agent: false,
            sweep_sec: None,
            quota_every_secs: "300",
            idle_nudge_secs: "300",
            quota: "on",
        };
        let document = super::meta_document(&env, &shape, &launching, watchdog, None)
            .expect("a resume document");
        assert!(
            document.contains("observed_model.main=Opus 5 (1M context)\n"),
            "{document}"
        );
        assert!(
            document.contains("observed_model_pin.main=fable\n"),
            "{document}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The `@` split: a bare profile selects no client, one `@` selects
    /// exactly one label, and an empty half or a second `@` is a usage error
    /// naming the value.
    #[test]
    fn seat_override_values_split_into_profile_and_optional_client() {
        assert_eq!(
            super::split_seat_override("fablex"),
            Ok(("fablex".to_owned(), None))
        );
        assert_eq!(
            super::split_seat_override("fablex@cc-mic"),
            Ok(("fablex".to_owned(), Some("cc-mic".to_owned())))
        );
        // (An empty VALUE never reaches the split: the plan parser refuses
        // empty `--lead` / `--seat` selections before preflight.)
        for bad in ["@cc-mic", "fablex@", "@", "fablex@cc@mic"] {
            let line = super::split_seat_override(bad).expect_err("usage");
            assert!(
                line.contains(bad) && line.contains("<profile>@<client>"),
                "{line:?}"
            );
        }
        // No `@` at all is never an error, even for a value no config names:
        // existence is the resolver's question, not the split's.
        assert_eq!(
            super::split_seat_override("missing"),
            Ok(("missing".to_owned(), None))
        );
    }

    fn override_cfg() -> crate::config::IdentityConfig {
        crate::config::parse_identity(
            "[clients]\n\
             claude = claude\n\
             cc-mic = claude config_home=$HOME/.claude-mic\n\
             cc-other = claude config_home=$HOME/.claude-other\n\
             codex = codex\n\
             u1 = sleep-u1\n\
             u2 = sleep-u2\n\
             [profiles]\n\
             fablex = \"claude --model fable --effort xhigh\"\n\
             solx = \"codex -m sol\"\n\
             weird1 = \"sleep-u1 --x\"\n\
             weird2 = \"sleep-u2 --x\"\n\
             [roster]\n\
             lead = fablex\n\
             [workspace]\n\
             main = lead\n",
        )
        .expect("readable override config")
    }

    /// R1: an override may change the account, never the harness adapter —
    /// both sides must be the SAME KNOWN harness, and `Unknown` on either
    /// side refuses. Gating on `from_binary_name` would let the two-unknown
    /// pair through, because any two unknowns compare equal there.
    #[test]
    fn client_override_resolves_same_adapter_and_refuses_anything_else() {
        let cfg = override_cfg();
        let home = PathBuf::from("/Users/a");
        let (command, parsed) = super::resolve_client_override(
            &cfg,
            "lead",
            "fablex",
            "cc-mic",
            Some(&home),
            "fablex, solx, weird1, weird2",
            "claude, cc-mic, cc-other, codex, u1, u2",
        )
        .expect("same-adapter override resolves");
        assert_eq!(
            command.as_str(),
            "CLAUDE_CONFIG_DIR='/Users/a/.claude-mic' claude --model fable --effort xhigh"
        );
        assert_eq!(command.client_label(), Some("cc-mic"));
        assert_eq!(parsed.binary, "claude");
        // Cross-adapter: both values named, duplicate profile as the way out.
        let line = match super::resolve_client_override(
            &cfg,
            "lead",
            "fablex",
            "codex",
            Some(&home),
            "fablex",
            "codex",
        ) {
            Err(super::SeatOverrideRefusal::Usage(line)) => line,
            other => panic!("cross-adapter must refuse Usage, got {other:?}"),
        };
        assert!(
            line.contains("fablex@codex")
                && line.contains("claude")
                && line.contains("codex")
                && line.contains("never the harness adapter")
                && line.contains("duplicate profile"),
            "{line:?}"
        );
        // Unknown on EITHER side refuses — including unknown on BOTH.
        for (profile, label) in [
            ("weird1", "u2"),
            ("weird2", "u1"),
            ("fablex", "u1"),
            ("weird1", "cc-mic"),
        ] {
            let refused = super::resolve_client_override(
                &cfg,
                "lead",
                profile,
                label,
                Some(&home),
                "fablex",
                "codex",
            );
            assert!(
                matches!(refused, Err(super::SeatOverrideRefusal::Usage(_))),
                "{profile}@{label} must refuse, got {refused:?}"
            );
        }
        // Unknown sides are named as unrecognized binaries, never as equal.
        let line = match super::resolve_client_override(
            &cfg,
            "lead",
            "weird1",
            "u2",
            Some(&home),
            "fablex",
            "codex",
        ) {
            Err(super::SeatOverrideRefusal::Usage(line)) => line,
            other => panic!("two unknowns must refuse, got {other:?}"),
        };
        assert!(
            line.contains("unrecognized binary 'sleep-u1'")
                && line.contains("unrecognized binary 'sleep-u2'"),
            "{line:?}"
        );
    }

    #[test]
    fn client_override_errors_mirror_the_unknown_profile_shape() {
        let cfg = override_cfg();
        let home = PathBuf::from("/Users/a");
        match super::resolve_client_override(
            &cfg,
            "lead",
            "missing",
            "cc-mic",
            Some(&home),
            "fablex, solx",
            "cc-mic, codex",
        ) {
            Err(super::SeatOverrideRefusal::Usage(line)) => assert_eq!(
                line,
                "Error: unknown profile 'missing' in --seat. Known profiles: fablex, solx."
            ),
            other => panic!("unknown profile must refuse Usage, got {other:?}"),
        }
        match super::resolve_client_override(
            &cfg,
            "lead",
            "fablex",
            "ghost",
            Some(&home),
            "fablex, solx",
            "cc-mic, codex",
        ) {
            Err(super::SeatOverrideRefusal::Usage(line)) => assert_eq!(
                line,
                "Error: unknown client 'ghost' in --seat. Known clients: cc-mic, codex."
            ),
            other => panic!("unknown client must refuse Usage, got {other:?}"),
        }
    }

    fn recorded_entry(meta: &str) -> crate::meta::RosterEntry {
        crate::meta::Meta::parse(meta).roster().to_vec().remove(0)
    }

    /// R2b at the launch boundary: `Missing` is no override, `Label` needs
    /// its label live in the current config, and `Invalid` fails closed.
    /// Only the absent label names the restore remedy.
    #[test]
    fn recorded_client_states_resolve_or_refuse_with_their_remedy() {
        let cfg = override_cfg();
        let entry = recorded_entry("seat.main=lead\nprofile.main=fablex\n");
        assert_eq!(
            super::recorded_selection("s", &cfg, &entry),
            Ok(super::RecordedSelection::Missing)
        );
        let entry = recorded_entry("seat.main=lead\nprofile.main=fablex\nclient.main=cc-mic\n");
        assert_eq!(
            super::recorded_selection("s", &cfg, &entry),
            Ok(super::RecordedSelection::Label("cc-mic".to_owned()))
        );
        let entry = recorded_entry("seat.main=lead\nprofile.main=fablex\nclient.main=ghost\n");
        let line = super::recorded_selection("s", &cfg, &entry).expect_err("absent refuses");
        assert!(
            line.contains("'ghost'")
                && line.contains("restore the 'ghost' client")
                && line.contains("ae end s"),
            "{line:?}"
        );
        for meta in [
            "seat.main=lead\nprofile.main=fablex\nclient.main=\n",
            "seat.main=lead\nprofile.main=fablex\nclient.main=cc mic\n",
            "seat.main=lead\nprofile.main=fablex\nclient.main=cc-mic\nclient.main=cc-other\n",
        ] {
            let entry = recorded_entry(meta);
            let line = super::recorded_selection("s", &cfg, &entry).expect_err("invalid refuses");
            assert!(
                line.contains("client.main")
                    && line.contains("fix the meta row")
                    && line.contains("ae end s"),
                "{meta:?} -> {line:?}"
            );
        }
    }

    /// R4b: once `Label(l)` is recorded, ONLY `l` is accepted. A different
    /// label refuses whatever its facts, and a bare profile cannot drop the
    /// row by saying nothing.
    #[test]
    fn recorded_label_accepts_only_its_exact_label() {
        let cfg = override_cfg();
        let home = PathBuf::from("/Users/a");
        let entry = recorded_entry("seat.main=lead\nprofile.main=fablex\nclient.main=cc-mic\n");
        let command = cfg
            .command_with_client("fablex", "cc-mic", Some(&home))
            .expect("override resolves")
            .command;
        // The exact label, with no recorded store yet: first start proceeds.
        super::refuse_recorded_client_conflict(
            "s",
            "lead",
            "fablex",
            Some("cc-mic"),
            &entry,
            &cfg,
            &command,
            ToolKind::Claude,
            Some(&home),
        )
        .expect("exact label proceeds");
        // A different label refuses, naming the recorded one and the profile
        // spelling that would satisfy write-once.
        let line = match super::refuse_recorded_client_conflict(
            "s",
            "lead",
            "fablex",
            Some("cc-other"),
            &entry,
            &cfg,
            &command,
            ToolKind::Claude,
            Some(&home),
        ) {
            Err(super::SeatOverrideRefusal::Usage(line)) => line,
            other => panic!("different label must refuse, got {other:?}"),
        };
        assert!(
            line.contains("'cc-mic'")
                && line.contains("'cc-other'")
                && line.contains("write-once")
                && line.contains("fablex@cc-mic"),
            "{line:?}"
        );
        // Identical facts today change nothing: the equality cannot cross a
        // config edit, so even a twin store refuses by label.
        let twin = crate::config::parse_identity(
            "[clients]\ncc-mic = claude config_home=/twin\ncc-twin = claude config_home=/twin\n\
             [profiles]\nfablex = \"claude --model fable\"\n\
             [roster]\nlead = fablex\n[workspace]\nmain = lead\n",
        )
        .expect("twin config");
        let twin_command = twin
            .command_with_client("fablex", "cc-twin", None)
            .expect("twin resolves")
            .command;
        let refused = super::refuse_recorded_client_conflict(
            "s",
            "lead",
            "fablex",
            Some("cc-twin"),
            &entry,
            &twin,
            &twin_command,
            ToolKind::Claude,
            None,
        );
        assert!(
            matches!(refused, Err(super::SeatOverrideRefusal::Usage(_))),
            "a twin store under another label still refuses, got {refused:?}"
        );
        // A bare profile cannot drop the row.
        let plain = cfg
            .command("fablex", Some(&home))
            .expect("profile resolves")
            .expect("profile exists");
        let line = match super::refuse_recorded_client_conflict(
            "s",
            "lead",
            "astrax",
            None,
            &entry,
            &cfg,
            &plain,
            ToolKind::Claude,
            Some(&home),
        ) {
            Err(super::SeatOverrideRefusal::Usage(line)) => line,
            other => panic!("bare re-pair must refuse, got {other:?}"),
        };
        assert!(
            line.contains("'cc-mic'") && line.contains("astrax@cc-mic"),
            "{line:?}"
        );
        // No recorded row and no selection: the legacy path is untouched.
        let legacy = recorded_entry("seat.main=lead\nprofile.main=fablex\n");
        super::refuse_recorded_client_conflict(
            "s",
            "lead",
            "astrax",
            None,
            &legacy,
            &cfg,
            &plain,
            ToolKind::Claude,
            Some(&home),
        )
        .expect("legacy bare override proceeds");
    }

    /// The R4 call site, not just the comparison: a FIRST override on a seat
    /// that never recorded one still re-takes the triple against the recorded
    /// store. Skipping the call would let a conflicting store through.
    #[test]
    fn first_override_against_a_recorded_store_retakes_the_triple() {
        let home = PathBuf::from("/home-op");
        let cfg = crate::config::parse_identity(
            "[clients]\ncc-a = claude config_home=/store-a\ncc-b = claude config_home=/store-b\n\
             [profiles]\nf = \"claude --model fable\"\n\
             [roster]\nlead = f\n[workspace]\nmain = lead\n",
        )
        .expect("two-store config");
        let entry = recorded_entry("seat.main=lead\nprofile.main=f\nconfig_home.main=/store-a\n");
        // Same store: proceeds, and the launch records the row from then on.
        let matching = cfg
            .command_with_client("f", "cc-a", Some(&home))
            .expect("matching resolves")
            .command;
        super::refuse_recorded_client_conflict(
            "s",
            "lead",
            "f",
            Some("cc-a"),
            &entry,
            &cfg,
            &matching,
            ToolKind::Claude,
            Some(&home),
        )
        .expect("matching first override proceeds");
        // Another store: refuses, naming both.
        let conflicting = cfg
            .command_with_client("f", "cc-b", Some(&home))
            .expect("conflicting resolves")
            .command;
        let line = match super::refuse_recorded_client_conflict(
            "s",
            "lead",
            "f",
            Some("cc-b"),
            &entry,
            &cfg,
            &conflicting,
            ToolKind::Claude,
            Some(&home),
        ) {
            Err(super::SeatOverrideRefusal::Usage(line)) => line,
            other => panic!("conflicting first override must refuse, got {other:?}"),
        };
        assert!(
            line.contains("/store-a") && line.contains("/store-b"),
            "{line:?}"
        );
    }

    /// R4: the recorded store triple (MODE + PATH + BASE) is re-taken against
    /// the current resolution at every launch. Path alone is not identity: an
    /// implicit and an explicit row over one path refuse each other.
    #[test]
    fn store_conflict_compares_mode_path_and_base_never_path_alone() {
        let home = PathBuf::from("/home-op");
        let mic = |store: &str| {
            crate::config::parse_identity(&format!(
                "[clients]\ncc = claude config_home={store}\n\
                 [profiles]\nf = \"claude --model fable\"\n\
                 [roster]\nlead = f\n[workspace]\nmain = lead\n",
            ))
            .expect("mic config")
        };
        let resolve = |cfg: &crate::config::IdentityConfig| {
            cfg.command_with_client("f", "cc", Some(&home))
                .expect("override resolves")
                .command
        };
        // Matching explicit store proceeds.
        let cfg = mic("/store-a");
        let command = resolve(&cfg);
        let entry = recorded_entry("seat.main=lead\nprofile.main=f\nconfig_home.main=/store-a\n");
        super::refuse_store_conflict(
            "s",
            "lead",
            "f@cc",
            &entry,
            &command,
            ToolKind::Claude,
            Some(&home),
        )
        .expect("matching store proceeds");
        // A definition that moved under the label refuses, naming both.
        let moved = mic("/store-b");
        let moved_command = resolve(&moved);
        let line = match super::refuse_store_conflict(
            "s",
            "lead",
            "f@cc",
            &entry,
            &moved_command,
            ToolKind::Claude,
            Some(&home),
        ) {
            Err(super::SeatOverrideRefusal::Usage(line)) => line,
            other => panic!("moved definition must refuse, got {other:?}"),
        };
        assert!(
            line.contains("/store-a")
                && line.contains("/store-b")
                && line.contains("cannot move a retained conversation")
                && line.contains("ae end s"),
            "{line:?}"
        );
        // Implicit versus explicit over ONE path: different accounts.
        let implicit_cfg = crate::config::parse_identity(
            "[clients]\ncc = claude\n[profiles]\nf = \"claude --model fable\"\n\
             [roster]\nlead = f\n[workspace]\nmain = lead\n",
        )
        .expect("implicit config");
        let implicit_command = implicit_cfg
            .command_with_client("f", "cc", Some(&home))
            .expect("implicit override resolves")
            .command;
        let entry =
            recorded_entry("seat.main=lead\nprofile.main=f\nconfig_home.main=/home-op/.claude\n");
        let refused = super::refuse_store_conflict(
            "s",
            "lead",
            "f@cc",
            &entry,
            &implicit_command,
            ToolKind::Claude,
            Some(&home),
        );
        assert!(
            matches!(refused, Err(super::SeatOverrideRefusal::Usage(_))),
            "explicit recorded versus implicit current refuses, got {refused:?}"
        );
        // The same path in the same mode proceeds.
        let entry = recorded_entry(
            "seat.main=lead\nprofile.main=f\nconfig_home.main=implicit:/home-op/.claude\nconfig_home_base.main=/home-op\n",
        );
        super::refuse_store_conflict(
            "s",
            "lead",
            "f@cc",
            &entry,
            &implicit_command,
            ToolKind::Claude,
            Some(&home),
        )
        .expect("matching implicit store proceeds");
        // ... but not under another base.
        let elsewhere = PathBuf::from("/home-elsewhere");
        let refused = super::refuse_store_conflict(
            "s",
            "lead",
            "f@cc",
            &entry,
            &implicit_command,
            ToolKind::Claude,
            Some(&elsewhere),
        );
        assert!(
            matches!(refused, Err(super::SeatOverrideRefusal::Usage(_))),
            "same path under another HOME refuses, got {refused:?}"
        );
    }

    /// The R4 edges: first start proceeds, a current `Unknown` proves no
    /// conflict (`_run` retains with a notice), and recorded `Unknown` /
    /// `Invalid` / mismatched `Absent` fail closed.
    #[test]
    fn store_conflict_edges_fail_closed_except_first_start_and_unknown_current() {
        let home = PathBuf::from("/Users/a");
        let cfg = override_cfg();
        let command = cfg
            .command_with_client("fablex", "cc-mic", Some(&home))
            .expect("override resolves")
            .command;
        // First start: no recorded store, the override is recorded normally.
        let entry = recorded_entry("seat.main=lead\nprofile.main=fablex\nclient.main=cc-mic\n");
        super::refuse_store_conflict(
            "s",
            "lead",
            "fablex@cc-mic",
            &entry,
            &command,
            ToolKind::Claude,
            Some(&home),
        )
        .expect("first start proceeds");
        // Recorded damage fails closed.
        for meta in [
            "seat.main=lead\nprofile.main=fablex\nconfig_home.main=unknown\n",
            "seat.main=lead\nprofile.main=fablex\nconfig_home.main=\n",
        ] {
            let entry = recorded_entry(meta);
            assert!(
                super::refuse_store_conflict(
                    "s",
                    "lead",
                    "fablex@cc-mic",
                    &entry,
                    &command,
                    ToolKind::Claude,
                    Some(&home),
                )
                .is_err(),
                "{meta:?} fails closed"
            );
        }
        // Recorded Absent matches only Absent.
        let entry =
            recorded_entry("seat.main=lead\nprofile.main=fablex\nconfig_home.main=absent\n");
        let refused = super::refuse_store_conflict(
            "s",
            "lead",
            "fablex@cc-mic",
            &entry,
            &command,
            ToolKind::Claude,
            Some(&home),
        );
        assert!(
            matches!(refused, Err(super::SeatOverrideRefusal::Usage(_))),
            "absent recorded versus explicit current refuses, got {refused:?}"
        );
        // A current Unknown (a pane-only relative assignment the launcher
        // cannot resolve) proves no conflict and proceeds. The override
        // client is implicit, so the pair takes no conflict — the Unknown
        // comes from the profile's own unresolvable assignment.
        let relative = crate::config::parse_identity(
            "[clients]\nplain = claude\n\
             [profiles]\nr = \"CLAUDE_CONFIG_DIR=relative/path claude --model fable\"\n\
             [roster]\nlead = r\n[workspace]\nmain = lead\n",
        )
        .expect("relative config");
        let relative_command = relative
            .command_with_client("r", "plain", Some(&home))
            .expect("relative override resolves")
            .command;
        let entry =
            recorded_entry("seat.main=lead\nprofile.main=r\nconfig_home.main=/Users/a/.claude\n");
        super::refuse_store_conflict(
            "s",
            "lead",
            "r@plain",
            &entry,
            &relative_command,
            ToolKind::Claude,
            Some(&home),
        )
        .expect("unknown current proceeds; _run retains");
    }

    /// F6: a first start whose override resolves `Unknown` refuses — nothing
    /// retained means nothing stranded, and proceeding would ignore the
    /// explicit flag.
    #[test]
    fn unresolvable_first_start_refuses_naming_the_override_and_why() {
        let home = PathBuf::from("/Users/a");
        let entry = recorded_entry("seat.main=lead\nprofile.main=varp\n");
        let blind = crate::config::parse_identity(
            "[clients]\nplain = claude\n\
             [profiles]\nvarp = \"CLAUDE_CONFIG_DIR=rel/store claude --model fable\"\n\
             [roster]\nlead = varp\n[workspace]\nmain = lead\n",
        )
        .expect("readable blind config");
        let blind_command = blind
            .command_with_client("varp", "plain", Some(&home))
            .expect("blind override resolves")
            .command;
        let line = match super::refuse_store_conflict(
            "s",
            "lead",
            "varp@plain",
            &entry,
            &blind_command,
            ToolKind::Claude,
            Some(&home),
        ) {
            Err(super::SeatOverrideRefusal::Usage(line)) => line,
            other => panic!("unresolvable first start must refuse, got {other:?}"),
        };
        assert!(
            line.contains("varp@plain")
                && line.contains("unknown conversation store")
                && line.contains("not an absolute path"),
            "{line:?}"
        );
    }
}
