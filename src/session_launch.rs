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
use crate::session_tmux::{Op, Split, argv, interpret_pane_id, mouse_down_status_binding_argv};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::tool::ToolKind;
use crate::{deliver, roster, tmux, transport};

pub(crate) mod assets;
pub(crate) mod capture;
pub(crate) mod name;

/// The usage line for the core entry.
pub const USAGE: &str = "Usage: _launch --home <ae-home> --cwd <dir> [--global <cfg>] [--local <cfg>] [--server-kind <kind>] [--server <value>] [--caller-socket <path>] [--attach|--no-attach] [--no-autostart] [--] [--worktree|--copy|--local] [--dir <path>] [--no-attach] [--from <uuid>] [use <name>] <session-name>";

/// The public refusal for a launch argv that names no session.
pub const MISSING_NAME: &str = "Error: a session needs a name: ae <name> [...]";

/// The public launch usage line paired with [`MISSING_NAME`].
pub const PUBLIC_USAGE: &str = "Usage: ae <name> [--local|--copy|--worktree] [--dir <path>] [--no-attach] [--from <archive-uuid>] [use <agent>]";

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
pub(crate) fn parse_plan_with_attach(args: &[String], allow_attach: bool) -> Result<Plan, String> {
    let mut plan = Plan::default();
    let mut rest = args;
    while let [word, tail @ ..] = rest {
        rest = tail;
        match word.as_str() {
            "--worktree" => plan.mode = Some(Mode::Git),
            "--copy" => plan.mode = Some(Mode::Full),
            "--local" => plan.mode = Some(Mode::Local),
            "--attach" if allow_attach => plan.attach = Some(true),
            "--no-attach" => plan.attach = Some(false),
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
                    "Error: unknown flag '{word}'. Use --worktree, --copy, --local, --dir <path>, --no-attach, or --from <archive-uuid>."
                ));
            }
            // Last positional wins.
            _ => plan.name = Some(word.clone()),
        }
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
    launch(&env, &plan, None, out, err)
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
        dir: None,
        attach: None,
    };
    launch(&env, &launch_plan, Some(plan.proof), out, err)
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
    let meta_present = node_exists(&dir.join(crate::store::META));

    // ---- THE tmux FLOOR, before the first thing this launch would write ----
    // AHEAD of the lifecycle lock, migration chain and config seed. A resume
    // first takes a read-only look at its recorded server so the floor is asked
    // of the server this attempt will actually use; the decision is repeated
    // under the lifecycle lock below before anything acts on it.
    let floor_server = if meta_present {
        let Ok(bytes) = meta::read_bytes(&dir) else {
            writeln!(err, "Error: could not read metadata for '{session}'.")?;
            return Ok(EXIT_FAILED);
        };
        match Meta::parse(&String::from_utf8_lossy(&bytes)).server_selector() {
            ServerSelector::Positive(selector) => {
                let recorded = ServerId::Selected(selector);
                match transport::verify_session_absent(&recorded, &session) {
                    tmux::StopProbe::Absent => proposed_server.clone(),
                    tmux::StopProbe::Present => recorded,
                    tmux::StopProbe::Unknown => {
                        writeln!(
                            err,
                            "Error: cannot verify whether tmux session '{session}' is absent on its recorded server. Metadata was not changed."
                        )?;
                        return Ok(EXIT_FAILED);
                    }
                }
            }
            ServerSelector::Missing => {
                let historical = crate::doors::historical_server();
                match transport::verify_session_absent(&historical, &session) {
                    tmux::StopProbe::Absent => proposed_server.clone(),
                    tmux::StopProbe::Present => historical,
                    tmux::StopProbe::Unknown => {
                        writeln!(
                            err,
                            "Error: cannot verify whether legacy tmux session '{session}' is absent on the historical default server. Metadata was not changed."
                        )?;
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
    if let Some(refusal) = floor_refusal(&floor_server, &session) {
        write!(err, "{refusal}")?;
        return Ok(crate::tmux_floor::EXIT_REFUSED);
    }

    // A resume's liveness decision and any legacy backfill share the lock
    // stop/end use. The preflight above exists only to place the floor before
    // this lock-file write; every fact is re-read while the lock is held.
    let mut lifecycle = if meta_present {
        if let Ok(held) = crate::store::lock(
            &sessions.join(format!(".lifecycle.{session}.lock")),
            LIFECYCLE_WAIT,
        ) {
            Some(held)
        } else {
            writeln!(
                err,
                "Error: another lifecycle operation (end) is in progress for '{session}' — retry shortly."
            )?;
            return Ok(EXIT_FAILED);
        }
    } else {
        None
    };

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
                match transport::verify_session_absent(&recorded, &session) {
                    tmux::StopProbe::Present => {
                        set_env_server(&mut env, &recorded);
                        running_server = Some(recorded);
                    }
                    tmux::StopProbe::Absent => {
                        if !destination_is_absent(&proposed_server, &session, err)? {
                            return Ok(EXIT_FAILED);
                        }
                    }
                    tmux::StopProbe::Unknown => {
                        writeln!(
                            err,
                            "Error: cannot verify whether tmux session '{session}' is absent on its recorded server. Metadata was not changed."
                        )?;
                        return Ok(EXIT_FAILED);
                    }
                }
            }
            ServerSelector::Missing => {
                let historical = crate::doors::historical_server();
                match transport::verify_session_absent(&historical, &session) {
                    tmux::StopProbe::Present => {
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
                    tmux::StopProbe::Absent => {
                        if !destination_is_absent(&proposed_server, &session, err)? {
                            return Ok(EXIT_FAILED);
                        }
                    }
                    tmux::StopProbe::Unknown => {
                        writeln!(
                            err,
                            "Error: cannot verify whether legacy tmux session '{session}' is absent on the historical default server. Metadata was not changed."
                        )?;
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
    if orchestrator_seat {
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
        if transport::observe_agents(&server, &session).is_none() {
            writeln!(
                err,
                "Error: tmux session '{session}' exists but is not an ae session."
            )?;
            return Ok(EXIT_FAILED);
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
    let mut cfg: IdentityConfig = match if orchestrator_seat {
        config::read_identity(env.global.as_deref(), None)
    } else {
        config::read_identity(env.global.as_deref(), env.local.as_deref())
    } {
        Ok(cfg) => cfg,
        Err(why) => {
            writeln!(err, "{why}")?;
            return Ok(EXIT_FAILED);
        }
    };
    if orchestrator_seat {
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

    if let Some(workers) = &plan.workers {
        cfg.workers = Some(workers.clone());
    }
    let mut seats = match config::launch_plan(&cfg, plan.main.as_deref()) {
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
    let mut resuming = false;
    if mode == Mode::Local {
        if dir_exists(&work_root) {
            writeln!(
                err,
                "Error: stopped session '{session}' exists as worktree/copy. End it first (ae end {session}), or use a different name."
            )?;
            return Ok(EXIT_FAILED);
        }
        work_dir.clone_from(&env.cwd);
        if meta_present {
            resuming = true;
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
    } else if dir_exists(&work_root) {
        resuming = true;
        if let Some(stored) = meta_value(&dir, "origin").filter(|v| !v.is_empty()) {
            origin = PathBuf::from(stored);
        }
        writeln!(out, "Resuming session {session}...")?;
    } else {
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
    if resuming && let Some(saved) = meta_value(&dir, "layout").filter(|v| !v.is_empty()) {
        layout = saved;
    }

    // ---- restored spawned seats, LEXED before anything is created ----
    //
    // A spawned seat records a PROFILE NAME, and the resume looks that name up
    // in whatever config is current — so the command it resolves to is one no
    // earlier validation ever saw. `config::launch_plan` lexes the [roster]
    // seats and `_spawn` lexes the profile it is handed, but this path did
    // neither: it read `cfg.profile()`, opened a pane, and let the pane shell
    // run the string. A profile holding `touch m ; tail -f /dev/null` therefore
    // executed BOTH commands on resume — the same defect the spawn gate closed
    // (colead gate b5d60fec), reached through the restore instead.
    if resuming {
        for entry in spawned_entries(&dir) {
            // An unconfigured profile is not a refusal: the seat is preserved
            // verbatim and never launched, which the restore already handles.
            let Some(command) = cfg.profile(&entry.profile).filter(|c| !c.is_empty()) else {
                continue;
            };
            if let Err(why) = crate::launch_cmd::lex_simple_command(command) {
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
        meta_agent,
        sweep_sec.as_deref(),
        parent.as_ref(),
        lifecycle.take(),
        out,
        err,
    )
}

// ---------------------------------------------------------------------------
// the build: tmux, meta, assets, agents
// ---------------------------------------------------------------------------

/// One seat, resolved to what the launch needs to start it.
struct Launching {
    slot: String,
    name: String,
    profile: String,
    binary: String,
    tool: ToolKind,
    session_id: String,
    launch_id: String,
    pane: String,
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
    meta_agent: bool,
    sweep_sec: Option<&str>,
    parent: Option<&FromProof>,
    lifecycle: Option<std::fs::File>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let server = env.server();
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

    // ---- the session and its first pane ----
    let Some(main_pane) = new_session(&server, &shape.name, &work_dir) else {
        writeln!(err, "Error: could not create tmux session '{}'", shape.name)?;
        return Ok(EXIT_FAILED);
    };
    // Ownership as an explicit FACT, recorded at the mkdir.
    shape.dir_created = !node_exists(&dir);
    if let Err(why) = create_private_dir(&dir) {
        let _ = kill_session(&server, &shape.name);
        writeln!(err, "Error: could not create {} ({why})", dir.display())?;
        return Ok(EXIT_FAILED);
    }

    stamp_session(&server, env, shape, &main_pane);
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
    let mut launching: Vec<Launching> = Vec::new();
    for (index, seat) in seats.iter().enumerate() {
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
            binary: seat.binary.clone(),
            tool: seat.tool,
            session_id,
            launch_id,
            pane: panes[index].clone(),
        });
    }

    // ---- spawned agents, restored on resume ----
    if shape.resuming {
        for entry in spawned_entries(&dir) {
            // A profile that is not configured on THIS machine keeps its seat
            // VERBATIM at its original index — a later resume with the profile
            // configured restores the worker, and preserving the index keeps
            // the slot key stable for any in-flight request addressed to it.
            let command = cfg.profile(&entry.profile).unwrap_or_default().to_owned();
            let pane = if command.is_empty() {
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
            let tool = ToolKind::from_cmd(&command);
            let launch_id =
                launch_token(tool, meta_value(&dir, &format!("launch_id.{}", entry.slot)));
            launching.push(Launching {
                slot: entry.slot,
                name: entry.name,
                profile: entry.profile,
                binary: entry.binary,
                tool,
                session_id: entry.harness_session,
                launch_id,
                pane,
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
    let document = match meta_document(env, shape, &launching, meta_agent, sweep_sec, parent) {
        Ok(document) => document,
        Err(why) => return rollback_launch(shape, &dir, &server, &format!("Error: {why}"), err),
    };
    let published = if shape.resuming {
        meta::replace(&dir, &document)
    } else {
        meta::init(&dir, &document)
    };
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
    let Some(core) = crate::shape::resolved_exe() else {
        return rollback_launch(
            shape,
            &dir,
            &server,
            "Error: the core could not name its own binary.",
            err,
        );
    };
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
        start_watchdog_pane(env, shape, &dir, &server, anchor);
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

/// Create the session and its first pane, and report that pane's id.
fn new_session(server: &ServerId, name: &str, work_dir: &str) -> Option<String> {
    let (succeeded, stdout) =
        transport::run_tmux_op(&argv(server, &Op::NewSession { name, work_dir }));
    interpret_pane_id(succeeded, &stdout)
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
    match transport::observe_session_id(server, name) {
        Some(id) => transport::kill_session(server, &id),
        None => false,
    }
}

/// The environment, options and status bar every ae session carries.
fn stamp_session(server: &ServerId, env: &Env, shape: &Session, main_pane: &str) {
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
    if let Some(binding) = mouse_down_status_binding_argv(server) {
        let _ = transport::run_tmux_op(&binding);
    }
    let _ = transport::run_tmux_op(&argv(server, &Op::SetClientSessionHook { pane: main_pane }));
    let _ = transport::run_tmux_op(&argv(server, &Op::SelectPane { pane: main_pane }));
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
    if panes.len() <= 1 {
        return;
    }
    let select = |target: &str, layout: &str| {
        let _ = transport::run_tmux_op(&argv(server, &Op::SelectLayout { target, layout }));
    };
    match shape.layout.as_str() {
        "lead-pair" => {
            select(&panes[0], "even-horizontal");
            if workers > 2 {
                select(&panes[2], "even-vertical");
            }
        }
        "lead-solo" => {
            if workers > 1 {
                select(&panes[1], "even-vertical");
            }
        }
        "vertical" => select(
            &format!("{}:", tmux::session_target(&shape.name)),
            "even-horizontal",
        ),
        _ => select(
            &format!("{}:", tmux::session_target(&shape.name)),
            "even-vertical",
        ),
    }
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
    meta_agent: bool,
    sweep_sec: Option<&str>,
    parent: Option<&FromProof>,
) -> Result<String, String> {
    let dir = env.sessions().join(&shape.name);
    let main_pane = launching
        .first()
        .map(|agent| agent.pane.clone())
        .unwrap_or_default();
    // Launch facts that must survive every rewrite.
    let preserved = |key: &str| meta_value(&dir, key).filter(|v| !v.is_empty());
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
    if meta_agent {
        row("meta_agent", "true");
        if let Some(seconds) = sweep_sec {
            row("sweep_sec", seconds);
        }
    }
    for agent in launching {
        if !agent.launch_id.is_empty() {
            row(&format!("launch_id.{}", agent.slot), &agent.launch_id);
        }
    }

    let seats: Vec<roster::SeatLines> = launching
        .iter()
        .map(|agent| roster::SeatLines {
            slot: agent.slot.clone(),
            name: agent.name.clone(),
            profile: agent.profile.clone(),
            binary: (!agent.binary.is_empty()).then(|| agent.binary.clone()),
            harness_session: (!agent.session_id.is_empty()).then(|| agent.session_id.clone()),
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
    let _ = deliver::submit_shell_text(
        server,
        &agent.pane,
        &crate::run::pane_command(core, dir, &agent.slot),
    );
    wait_for_agent_start(server, &agent.pane, agent.tool);
    if agent.tool.adapter().capture.is_needed() {
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

/// The launch TOKEN this seat launches with: the one it already has, else a
/// fresh one, and none at all for a tool with no post-launch capture.
///
/// The token is what tells two seats apart INSIDE the tool's own store, so it
/// has to outlive a resume the way `harness_session` does. A resumed seat with
/// an empty token carries no `AE_..._LAUNCH_ID` in its instructions, and its
/// capture is reduced to matching the newest log in the working directory.
/// Measured 2026-09-04: two codex seats resumed while both were still `pending`
/// then raced onto ONE rollout and recorded the SAME id twice.
fn launch_token(tool: ToolKind, stored: Option<String>) -> String {
    if !tool.adapter().capture.is_needed() {
        return String::new();
    }
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
    let reason = if deliver::wait_input_ready(server, &agent.pane, model, LAUNCH_READY_POLLS) {
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
            Ok(()) if deliver::submit_staged(server, &agent.pane, model) => return Ok(()),
            Ok(()) => "submit UNCONFIRMED — the turn is staged unsent in the input box".to_owned(),
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
fn start_watchdog_pane(env: &Env, shape: &Session, dir: &Path, server: &ServerId, anchor: &str) {
    let recorded = meta_value(dir, "watchdog").or_else(|| meta_value(dir, "loop"));
    let configured = config::read_workspace_keys(
        env.global.as_deref(),
        env.local.as_deref(),
        &["watchdog", "loop"],
    );
    let enabled = recorded
        .or_else(|| configured[0].clone())
        .or_else(|| configured[1].clone())
        .unwrap_or_default();
    if matches!(
        enabled.to_ascii_lowercase().as_str(),
        "false" | "no" | "off" | "0"
    ) {
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
    binary: String,
    harness_session: String,
}

/// The `spawned.<n>` seats a resuming session's meta records, in slot order.
fn spawned_entries(dir: &Path) -> Vec<Spawned> {
    let Ok(bytes) = meta::read_bytes(dir) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let parsed = Meta::parse(&text);
    let mut out: Vec<Spawned> = parsed
        .roster()
        .iter()
        .filter(|entry| entry.slot.starts_with("spawned."))
        .map(|entry| Spawned {
            slot: entry.slot.clone(),
            name: entry.name.clone(),
            profile: entry.profile.clone().unwrap_or_default(),
            binary: entry.binary.clone().unwrap_or_default(),
            harness_session: entry.harness_session.clone().unwrap_or_default(),
        })
        .collect();
    out.sort_by_key(|entry| {
        entry
            .slot
            .rsplit_once('.')
            .and_then(|(_, index)| index.parse::<u32>().ok())
            .unwrap_or(u32::MAX)
    });
    out
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
        AttachAction, EVENTS_KEEP, ToolKind, attach_action, launch_token, launch_turn_is_pasted,
        parse_plan, server_attach_hint, trim_events,
    };
    use crate::inventory::ServerId;
    use std::fmt::Write as _;
    use std::path::PathBuf;

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
    fn scratch(tag: &str) -> PathBuf {
        let dir = PathBuf::from(format!("/tmp/ae-launch-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
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

    /// A seat KEEPS its launch token across a resume; only a seat without one
    /// is given a fresh one, and a tool with no capture is given none.
    #[test]
    fn a_resumed_seat_keeps_the_launch_token_that_names_it_in_the_tools_store() {
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
            assert_eq!(launch_token(tool, Some("tok-1".to_owned())), "", "{tool:?}");
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
}
