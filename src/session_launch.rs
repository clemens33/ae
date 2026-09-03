//! `_launch`: a whole session, created or resumed, as ONE core operation.
//!
//! Ported from the frozen script's launch path — the dispatcher's fall-through
//! (`ae:12911` onwards): `_launch_parse_flags`, the name derivation and its
//! ownership guard, the teardown tombstones, the `--from` preflight, the
//! working-directory modes, the tmux session and its layout, the meta publish,
//! the session assets, the launch scripts and their readiness-gated paste, the
//! monitor panes, and the attach.
//!
//! # What this module is NOT a port of
//!
//! * **`sync_session_assets`** (422 lines) and its `_lib` library. Under the B
//!   move a session helper is a shim that execs the core, so there is no helper
//!   LOGIC left in bash to generate. See [`assets`].
//! * **The `ae orchestrator` / `ae hub` trampoline** (`ae:12782`), the Telegram
//!   and orchestrator autostarts, and `ae transfer`. Each is its own operation
//!   with its own move; a launch that quietly grew them would be three moves in
//!   one diff.
//!
//! # The order is the contract
//!
//! 1. every refusal that can be decided from argv and the filesystem, BEFORE
//!    the first tmux call or the first `mkdir` — a bad launch leaves nothing;
//! 2. the working copy;
//! 3. the lifecycle lock, held from the first tmux call through the last asset
//!    write, so a concurrent `ae end` cannot delete a half-built session
//!    directory that the next atomic rename would then recreate;
//! 4. the session, its panes and their stamps;
//! 5. the meta, published as ONE document — the first observable meta is the
//!    complete one;
//! 6. the assets;
//! 7. the launch scripts and the prompts;
//! 8. the monitor panes, then the attach.
//!
//! Any failure from step 4 onwards ROLLS BACK: the tmux session is killed, and
//! the session directory is removed only when THIS attempt created it.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{self, IdentityConfig, Seat};
use crate::deliver::region::Tool;
use crate::inventory::ServerId;
use crate::launch::{self, PENDING};
use crate::launch_cmd::ToolKind;
use crate::meta::{self, Meta, ServerSelector};
use crate::session_tmux::{Op, Split, argv, interpret_pane_id};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::{deliver, roster, tmux, transport};

pub(crate) mod assets;
pub(crate) mod capture;
pub(crate) mod name;

/// The frozen usage line for the core entry.
pub const USAGE: &str = "Usage: _launch --home <ae-home> --cwd <dir> [--global <cfg>] [--local <cfg>] [--server-kind <kind>] [--server <value>] [--attach|--no-attach] [--] [--worktree|--copy|--local] [--from <uuid>] [use <name>] [<session-name>]";

/// How long a freshly created pane's shell is given to draw its prompt before
/// anything is pasted — the frozen `sleep 0.3` at `ae:14130`.
const SHELL_SETTLE: Duration = Duration::from_millis(300);

/// How many polls the launch prompt's readiness wait takes. The frozen
/// `_deliver_launch_prompt` waits 90 half-seconds — past codex's own 30-second
/// per-server MCP `startup_timeout_sec`, deliberately, so the window does not
/// expire at the moment the tool gives up and settles.
const LAUNCH_READY_POLLS: u32 = 90;

/// How many polls the tool-process wait takes — the frozen `10`.
const START_POLLS: u32 = 10;

/// The pause between those polls — the frozen `sleep 0.1`.
const START_POLL: Duration = Duration::from_millis(100);

/// How long to hold the lifecycle lock for before giving up — the frozen
/// `flock -w 15`.
const LIFECYCLE_WAIT: Duration = Duration::from_secs(15);

/// The event log's resume-time retention, in lines — the frozen
/// `AE_EVENTS_KEEP` default.
const EVENTS_KEEP: usize = 1000;

/// The `launch-delivery-failed` action, kept byte-identical to the frozen one:
/// the watchdog and the digest both key on it.
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
    /// The spelling the meta records — the frozen `mode=` values.
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

/// What the user's own argv said — the frozen `_launch_parse_flags`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The mode flag, when one was given. Absent means config, then `local`.
    pub mode: Option<Mode>,
    /// The session name, when one was typed.
    pub name: Option<String>,
    /// `use <name>` — the main-seat override for this launch.
    pub main: Option<String>,
    /// `--from <uuid>` — the archive this session explicitly continues.
    pub from: Option<String>,
    /// The FROZEN worker list, when a relaunch supplies one. Never settable
    /// from the launch line: `-` and the empty string both mean NO workers,
    /// which is a different thing from the field being absent (that keeps the
    /// config's own list).
    pub workers: Option<String>,
}

/// Parse the user's launch argv.
///
/// # Errors
///
/// The refusal line, ready for stderr. Every one is the frozen text.
pub fn parse_plan(args: &[String]) -> Result<Plan, String> {
    let mut plan = Plan::default();
    let mut rest = args;
    while let [word, tail @ ..] = rest {
        rest = tail;
        match word.as_str() {
            "--worktree" => plan.mode = Some(Mode::Git),
            "--copy" => plan.mode = Some(Mode::Full),
            "--local" => plan.mode = Some(Mode::Local),
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
                    "Error: unknown flag '{word}'. Use --worktree, --copy, --local, or --from <archive-uuid>."
                ));
            }
            // Last positional wins, exactly as the frozen loop's `*)` arm does.
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
    /// Whether the caller is inside tmux, which decides attach vs switch.
    pub inside_tmux: bool,
    /// Whether to attach when the session is up.
    pub attach: bool,
    /// The core the glue RESOLVED (`_ae_core_bind`), recorded as `ae_core` with
    /// the version it reported — the pin every helper re-resolves from meta.
    /// The running binary stands in only when the caller named none.
    pub core: Option<PathBuf>,
    /// The version the resolved core reported, when the caller measured it.
    pub core_version: Option<String>,
    /// The glue's own path, recorded as `ae_path` — the `ae` COMMAND a helper or
    /// the watchdog re-execs (`ae telegram _supervise`, `ae _recover-pending`),
    /// which under a versioned install is a different file from the core.
    pub glue: Option<PathBuf>,
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
        match self.server_kind.as_str() {
            "socket" if !self.server_value.is_empty() => ServerId::Selected(
                crate::meta::Selector::Socket(PathBuf::from(&self.server_value)),
            ),
            "name" if !self.server_value.is_empty() => {
                ServerId::Selected(crate::meta::Selector::Name(self.server_value.clone()))
            }
            _ => ServerId::Ambient,
        }
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

/// Read the preamble flags, then the user's own argv after `--`.
///
/// # Errors
///
/// The offending word.
fn read_env(tail: &[String]) -> Result<(Env, Vec<String>), String> {
    let mut env = Env {
        home: PathBuf::new(),
        cwd: PathBuf::new(),
        global: None,
        local: None,
        server_kind: String::new(),
        server_value: String::new(),
        inside_tmux: false,
        attach: true,
        glue: None,
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
            _ => {}
        }
        let Some((value, after)) = after.split_first() else {
            return Err(flag.clone());
        };
        match flag.as_str() {
            "--home" => env.home = value.into(),
            "--cwd" => env.cwd = value.into(),
            "--global" => env.global = Some(value.into()),
            "--local-config" => env.local = Some(value.into()),
            "--server-kind" => env.server_kind.clone_from(value),
            "--server" => env.server_value.clone_from(value),
            "--glue" => env.glue = Some(value.into()),
            "--core" => env.core = Some(value.into()),
            "--core-version" => env.core_version = Some(value.clone()),
            _ => return Err(flag.clone()),
        }
        rest = after;
    }
    if env.home.as_os_str().is_empty() || env.cwd.as_os_str().is_empty() {
        return Err("--home and --cwd are required".to_owned());
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
    let (env, args) = match read_env(tail) {
        Ok(pair) => pair,
        Err(word) => {
            writeln!(err, "ae: {USAGE}")?;
            writeln!(err, "ae: offending word: {word}")?;
            return Ok(EXIT_USAGE);
        }
    };
    let plan = match parse_plan(&args) {
        Ok(plan) => plan,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(EXIT_FAILED);
        }
    };
    launch(&env, &plan, None, out, err)
}

/// The facts a `compact` relaunch carries across the boundary.
///
/// THE ROSTER IS THE FROZEN ONE. The child starts the agents the human was
/// SHOWN at the prompt, never a config re-read after the boundary: the source
/// session is already archived and gone by then, so a child that started the
/// wrong roster could not be undone.
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
    /// The `--from` proof compact took at the boundary, `\t`-separated. The
    /// child re-proves the archive and compares BYTE FOR BYTE: between the two
    /// proofs the archive could have been purged, claimed or corrupted.
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
        inside_tmux: false,
        attach: false,
        glue: None,
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
    };
    launch(&env, &launch_plan, Some(plan.proof), out, err)
}

/// Split `main=<name> workers=<a,b|->` into its two overrides.
///
/// A missing `main=` yields `None`, which lets the config's own answer stand —
/// the frozen gate fires only when the freeze actually resolved one.
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

/// A session's resolved shape, once every refusal has passed.
struct Session {
    name: String,
    mode: Mode,
    work_dir: PathBuf,
    origin: PathBuf,
    layout: String,
    resuming: bool,
    /// Whether THIS attempt created the session directory — the ownership fact
    /// the rollback keys on, recorded at the `mkdir` and never derived from
    /// `resuming`. A half-written directory from an earlier attempt reads as
    /// fresh, and the old proxy would have deleted it.
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
    let home = env.home.display().to_string();
    let sessions = env.sessions();
    let worktrees = env.worktrees();

    // ---- the name, and the guards that must precede every side effect ----
    let derived = plan.name.is_none();
    let session = plan
        .name
        .clone()
        .unwrap_or_else(|| name::default_session_name(&cwd, &home, 6));
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

    let meta_present = node_exists(&dir.join("meta"));
    // DERIVED-NAME OWNERSHIP. A name the user TYPED means "that session,
    // wherever I am". A name ae DERIVED means "the session for this directory",
    // and reusing it is only correct if the existing session belongs here — the
    // derived body is lossy, so a name match is evidence, not proof.
    if derived && meta_present {
        let recorded = meta_value(&dir, "origin");
        match recorded.as_deref() {
            None | Some("") => {
                writeln!(
                    err,
                    "note: '{session}' records no origin (pre-dates ownership tracking) — assuming it belongs to {cwd}."
                )?;
            }
            Some(owner) if !dir_exists(Path::new(owner)) => {
                writeln!(
                    err,
                    "Error: '{session}' records origin {owner}, which no longer exists."
                )?;
                writeln!(
                    err,
                    "       A MOVED project and a name COLLISION with a DELETED one look identical"
                )?;
                writeln!(
                    err,
                    "       from here, and guessing wrong resumes someone else's conversation."
                )?;
                writeln!(
                    err,
                    "       If that session is this project's, claim it by name:"
                )?;
                writeln!(err, "           ae {session}")?;
                writeln!(
                    err,
                    "       If it is not, start this one under a distinguished name:"
                )?;
                writeln!(
                    err,
                    "           ae {}",
                    name::default_session_name(&cwd, &home, 12)
                )?;
                return Ok(EXIT_FAILED);
            }
            Some(owner) if !same_directory(owner, &cwd) => {
                writeln!(
                    err,
                    "Error: '{session}' is the derived name of a DIFFERENT directory."
                )?;
                writeln!(err, "       it belongs to:  {owner}")?;
                writeln!(err, "       you are in:     {cwd}")?;
                writeln!(
                    err,
                    "       Two directories can reduce to the same derived name; attaching would join"
                )?;
                writeln!(
                    err,
                    "       that project's session and its conversation. Launch this one under a name"
                )?;
                writeln!(err, "       carrying more of the path hash:")?;
                writeln!(
                    err,
                    "           ae {}",
                    name::default_session_name(&cwd, &home, 12)
                )?;
                return Ok(EXIT_FAILED);
            }
            Some(_) => {}
        }
    }

    let server = env.server();
    // ---- explicit lineage, proved before anything is created ----
    let mut parent: Option<FromProof> = None;
    if let Some(uuid) = &plan.from {
        let root = env.home.join("archive");
        match from_preflight(&root, uuid) {
            Ok(proof) => {
                if transport::session_exists(&server, &session)
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
    if transport::session_exists(&server, &session) {
        if transport::observe_agents(&server, &session).is_none() {
            writeln!(
                err,
                "Error: tmux session '{session}' exists but is not an ae session."
            )?;
            return Ok(EXIT_FAILED);
        }
        if !env.attach {
            writeln!(
                out,
                "Session '{session}' is running. Use 'ae orchestrator --attach' to view."
            )?;
            return Ok(0);
        }
        return Ok(attach(&server, env, &session));
    }

    // ---- config, roster, workspace values ----
    // On resume the RECORDED config wins: an agent's aliases must resolve from
    // the file the session was created with, not from wherever the caller is.
    let mut env = env.clone();
    if meta_present {
        if let Some(stored) = meta_value(&dir, "config").filter(|v| !v.is_empty()) {
            env.global = Some(PathBuf::from(stored));
        }
        match meta_value(&dir, "origin").filter(|v| !v.is_empty()) {
            Some(origin) => {
                let candidate = Path::new(&origin).join(".ae").join("config");
                env.local = node_exists(&candidate).then_some(candidate);
            }
            None => env.local = None,
        }
    }
    let cfg: IdentityConfig =
        match config::read_identity(env.global.as_deref(), env.local.as_deref()) {
            Ok(cfg) => cfg,
            Err(why) => {
                writeln!(err, "{why}")?;
                return Ok(EXIT_FAILED);
            }
        };
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
        ],
    );
    let config_layout = extras[0].clone().unwrap_or_default();
    let config_copy = extras[1].clone().unwrap_or_default();
    let meta_agent = ["true", "1", "yes", "on"].contains(
        &extras[4]
            .clone()
            .or_else(|| extras[5].clone())
            .or_else(|| extras[6].clone())
            .unwrap_or_default()
            .as_str(),
    );

    let mut cfg = cfg;
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
    // A RESUME RESTORES THE ROSTER IT SAVED. The seat names (and profiles) in
    // meta are the session's identity; config is only where a FRESH launch reads
    // them from. Re-deriving on resume renamed `seat.main=chief` to the config's
    // current value and silently discarded a rename (glue cut 2 finding).
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

    let mut shape = Session {
        name: session.clone(),
        mode,
        work_dir,
        origin,
        layout,
        resuming,
        dir_created: false,
    };

    build(
        &env,
        &mut shape,
        &seats,
        &cfg,
        meta_agent,
        parent.as_ref(),
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
    command: String,
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
    parent: Option<&FromProof>,
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
    // The LIFECYCLE LOCK: mutual exclusion with `ae end`. Held from here through
    // the last asset write — released before the launch prompts, because the
    // capture threads below outlive this call and an inherited hold would make
    // an immediate `ae end` time out on a launch that already finished.
    let Ok(lifecycle) = crate::state::acquire(
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

    // ---- worker panes, by layout ----
    let mut panes = vec![main_pane.clone()];
    let workers = seats.len().saturating_sub(1);
    for index in 0..workers {
        let created = match shape.layout.as_str() {
            "lead-pair" => match index {
                0 => split(&server, &panes[0], &work_dir, Split::Horizontal),
                1 => new_window(&server, &format!("{}:", shape.name), "", &work_dir),
                _ => split(&server, &panes[panes.len() - 1], &work_dir, Split::Vertical),
            },
            "lead-solo" => match index {
                0 => new_window(&server, &format!("{}:", shape.name), "", &work_dir),
                _ => split(&server, &panes[panes.len() - 1], &work_dir, Split::Vertical),
            },
            "vertical" => split(&server, &shape.name, &work_dir, Split::Horizontal),
            _ => split(&server, &shape.name, &work_dir, Split::Vertical),
        };
        let Some(pane) = created else {
            let _ = kill_session(&server, &shape.name);
            rollback_dir(shape, &dir, err)?;
            writeln!(err, "Error: could not create a pane for worker {index}")?;
            return Ok(EXIT_FAILED);
        };
        panes.push(pane);
    }
    apply_layout(&server, shape, &panes, workers);

    // Pane stamps. `@ae_agent` IS the bare name under identity v2.
    for (index, seat) in seats.iter().enumerate() {
        stamp_pane(&server, &panes[index], &seat.name, &seat.slot);
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
        let launch_id = if !shape.resuming && launch::supports_launch_id(seat.tool) {
            launch::generate_uuid()
        } else {
            String::new()
        };
        launching.push(Launching {
            slot: seat.slot.clone(),
            name: seat.name.clone(),
            profile: seat.profile.clone(),
            binary: seat.binary.clone(),
            tool: seat.tool,
            command: seat.command.clone(),
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
            // It gets no pane and is never launched: an EMPTY pane is what the
            // rest of this function reads as "seat only".
            let command = cfg.profile(&entry.profile).unwrap_or_default().to_owned();
            let pane = if command.is_empty() {
                String::new()
            } else {
                match new_window(&server, &format!("{}:", shape.name), "", &work_dir) {
                    Some(pane) => {
                        let _ = transport::rename_window(
                            &server,
                            &pane,
                            &tmux::format_literal(&entry.name),
                        );
                        stamp_pane(&server, &pane, &entry.name, &entry.slot);
                        panes.push(pane.clone());
                        pane
                    }
                    None => String::new(),
                }
            };
            let tool = ToolKind::from_cmd(&command);
            launching.push(Launching {
                slot: entry.slot,
                name: entry.name,
                profile: entry.profile,
                binary: entry.binary,
                tool,
                command,
                session_id: entry.harness_session,
                launch_id: String::new(),
                pane,
            });
        }
        // Rebalance only the SPLIT layouts: a spawned agent gets its own window,
        // so window 0 gains no panes, and even-vertical would stack lead-pair's
        // side-by-side leads.
        if shape.layout != "lead-solo" && shape.layout != "lead-pair" {
            let target = shape.name.clone();
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
    let document = match meta_document(env, shape, &launching, meta_agent, parent) {
        Ok(document) => document,
        Err(why) => {
            let _ = kill_session(&server, &shape.name);
            rollback_dir(shape, &dir, err)?;
            writeln!(err, "Error: {why}")?;
            return Ok(EXIT_FAILED);
        }
    };
    let published = if shape.resuming {
        meta::replace(&dir, &document)
    } else {
        meta::init(&dir, &document)
    };
    if published.is_err() {
        let _ = kill_session(&server, &shape.name);
        rollback_dir(shape, &dir, err)?;
        writeln!(err, "Error: the session meta could not be published.")?;
        return Ok(EXIT_FAILED);
    }

    // ---- assets ----
    let core = match std::env::current_exe() {
        Ok(path) => path,
        Err(why) => {
            let _ = kill_session(&server, &shape.name);
            rollback_dir(shape, &dir, err)?;
            writeln!(
                err,
                "Error: the core could not name its own binary ({why})."
            )?;
            return Ok(EXIT_FAILED);
        }
    };
    if let Err(why) = assets::write_helpers(&dir, &core) {
        let _ = kill_session(&server, &shape.name);
        rollback_dir(shape, &dir, err)?;
        writeln!(err, "Error: {why}")?;
        return Ok(EXIT_FAILED);
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
        let _ = kill_session(&server, &shape.name);
        rollback_dir(shape, &dir, err)?;
        writeln!(
            err,
            "Error: could not write the workspace manifest ({why})."
        )?;
        return Ok(EXIT_FAILED);
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
        if let Err(why) = start_agent(env, shape, &dir, agent, &server, err)? {
            let _ = kill_session(&server, &shape.name);
            rollback_dir(shape, &dir, err)?;
            writeln!(
                err,
                "ae: launch failed — rolling back '{}'. ({why})",
                shape.name
            )?;
            return Ok(EXIT_FAILED);
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

    let _ = transport::run_tmux_op(&argv(&server, &Op::SelectPane { pane: &main_pane }));
    if !env.attach {
        writeln!(
            out,
            "Session '{}' started. Use 'ae orchestrator --attach' to view.",
            shape.name
        )?;
        return Ok(0);
    }
    Ok(attach(&server, env, &shape.name))
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
            name,
            work_dir,
            command: &[],
        },
    ));
    interpret_pane_id(succeeded, &stdout)
}

/// Kill the session this launch created — the rollback's first step.
///
/// By EXACT id, never by name: `kill-session -t` prefix-matches, so killing
/// `proj` would take a live `project` with it and report success. A session id
/// that cannot be resolved means there is nothing this launch may kill.
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
        ("pane-border-status", "top"),
        ("pane-border-format", " #{@ae_agent} "),
        ("automatic-rename", "off"),
    ] {
        let _ = transport::publish_option(server, tmux::OptionScope::Session, name, option, value);
    }
    apply_status_bar(server, shape);
    // The window carries the SESSION name; agent identity lives on `@ae_agent`.
    // Format-escaped, because a window name is a tmux FORMAT and `#(…)` runs a
    // shell.
    let _ = transport::run_tmux_op(&argv(
        server,
        &Op::RenameWindow {
            target: name,
            name: &tmux::format_literal(name),
        },
    ));
    let _ = transport::run_tmux_op(&argv(server, &Op::SelectPane { pane: main_pane }));
}

/// The two status lines ae owns.
///
/// The live segments arrive as USER OPTIONS (`@ae_branch_status`,
/// `@ae_watchdog_status`, `@ae_agents_status`), which tmux interpolates
/// literally — no `#()`, no shell — so a branch name with `)` or `#` in it is
/// harmless. The static path is `#`-escaped because it is not.
fn apply_status_bar(server: &ServerId, shape: &Session) {
    let name = shape.name.as_str();
    let paths = tmux::format_literal(&status_paths(shape));
    let session = tmux::format_literal(name);
    for (option, value) in [
        ("status-left", format!("[ae {session}] ")),
        ("status-left-length", "40".to_owned()),
        (
            "status-right",
            format!("[{paths}#{{@ae_branch_status}}] #{{@ae_watchdog_status}}"),
        ),
        ("status-right-length", "100".to_owned()),
        ("status-interval", "5".to_owned()),
        ("status", "2".to_owned()),
        // The per-window glyph the watchdog publishes into @ae_window_status
        // renders only through these two (ae:673-674); without them it is
        // published and never shown.
        (
            "window-status-format",
            "#I:#W#{@ae_window_status}#F".to_owned(),
        ),
        (
            "window-status-current-format",
            "#I:#W#{@ae_window_status}#F".to_owned(),
        ),
    ] {
        let _ = transport::publish_option(server, tmux::OptionScope::Session, name, option, &value);
    }
    // tmux ARRAY options do not inherit per index: setting `status-format[1]`
    // alone shadows the WHOLE global array and leaves index 0 — the standard bar
    // — empty at session scope. Copy the global [0] in alongside ours.
    let (found, global) = transport::run_tmux_op(&argv(
        server,
        &Op::ShowGlobalOption {
            name: "status-format[0]",
        },
    ));
    let global = global.trim_end_matches('\n');
    if found && !global.is_empty() {
        let _ = transport::publish_option(
            server,
            tmux::OptionScope::Session,
            name,
            "status-format[0]",
            global,
        );
    }
    let _ = transport::publish_option(
        server,
        tmux::OptionScope::Session,
        name,
        "status-format[1]",
        "#[align=left] #{@ae_agent}#[align=right]#{@ae_agents_status} ",
    );
}

/// The location segment of the status bar — the frozen
/// `_ae_status_left_paths`, whose shape is mode-aware.
fn status_paths(shape: &Session) -> String {
    match shape.mode {
        Mode::Local => shape.work_dir.display().to_string(),
        Mode::Git | Mode::Full => {
            format!("{} → {}", shape.origin.display(), shape.work_dir.display())
        }
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
        "vertical" => select(&shape.name, "even-horizontal"),
        _ => select(&shape.name, "even-vertical"),
    }
    // Role-based window names use FIXED literals — never an agent name, which
    // would feed the rename-window format sink.
    if shape.layout == "lead-solo" && workers > 0 {
        let _ = transport::run_tmux_op(&argv(
            server,
            &Op::RenameWindow {
                target: &panes[1],
                name: "workers",
            },
        ));
    }
    if shape.layout == "lead-pair" {
        let _ = transport::run_tmux_op(&argv(
            server,
            &Op::RenameWindow {
                target: &panes[0],
                name: "leads",
            },
        ));
        if workers > 1 {
            let _ = transport::run_tmux_op(&argv(
                server,
                &Op::RenameWindow {
                    target: &panes[2],
                    name: "workers",
                },
            ));
        }
    }
}

/// Label one pane with the identity it holds.
fn stamp_pane(server: &ServerId, pane: &str, name: &str, slot: &str) {
    let _ = transport::set_pane_title(server, pane, &format!("ae:{name}"));
    let _ = transport::publish_option(server, tmux::OptionScope::Pane, pane, "@ae_agent", name);
    let _ = transport::publish_option(server, tmux::OptionScope::Pane, pane, "@ae_slot", slot);
}

/// Build the WHOLE initial meta — base facts, then the v2 roster block.
///
/// One document, published in one rename: a direct write left the file visible
/// EMPTY between open and write, and post-publish appends left the roster
/// observably half-built, so meta existence was not a readiness fact.
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
    parent: Option<&FromProof>,
) -> Result<String, String> {
    let dir = env.sessions().join(&shape.name);
    let main_pane = launching
        .first()
        .map(|agent| agent.pane.clone())
        .unwrap_or_default();
    // Launch facts that must survive every rewrite. `git_base_commit` is
    // recorded ONCE, at the launch that created the tree: a resume that
    // re-derived it would silently re-base the session's commit range on
    // today's HEAD and make the archive's range a lie. Lineage is the same —
    // decided once, at birth.
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
    row("main_pane", &main_pane);
    row("ae_version", crate::VERSION);
    if let Ok(core) = std::env::current_exe() {
        // The core binding, pinned per session as a PAIR: a helper that found a
        // binary whose version disagreed with the session's would be running a
        // different contract than the one this session was built against.
        // ae_path is the recorded `ae` COMMAND, which is the glue when the caller
        // named it; the core only stands in for a caller that did not.
        let ae_path = env.glue.as_ref().unwrap_or(&core);
        let ae_core = env.core.as_ref().unwrap_or(&core);
        row("ae_path", &ae_path.display().to_string());
        row("ae_core", &ae_core.display().to_string());
        row(
            "ae_core_version",
            env.core_version.as_deref().unwrap_or(crate::VERSION),
        );
    }
    if !env.server_kind.is_empty() {
        row("tmux_server", &env.server_value);
        row("tmux_server_kind", &env.server_kind);
    }
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

/// Compose one agent's launch command, publish its script, and paste it.
///
/// `Ok(Ok(()))` started; `Ok(Err(reason))` is the launch-failing refusal the
/// caller rolls back on. A prompt that could not be DELIVERED is not one of
/// them: it is loud and durable, exactly as the frozen path reports it, but the
/// pane is live and the session stands.
fn start_agent(
    env: &Env,
    shape: &Session,
    dir: &Path,
    agent: &Launching,
    server: &ServerId,
    err: &mut impl Write,
) -> io::Result<Result<(), String>> {
    let ctx = crate::render::context_document(
        dir,
        &shape.name,
        &shape.work_dir.display().to_string(),
        &agent.slot,
        &env.config_files(),
    );
    // FRESH bakes the id in; RESUME asks for the recorded conversation. Both
    // capture the PRE-INJECTION command as the injection boundary — the launch
    // script's re-run form is built from it, and searching for it later in a
    // command carrying kilobytes of injected prose is the frozen bug class.
    let prompt = launch::initial_prompt_for(agent.tool).to_owned();
    // INJECT FIRST, WRAP THE DECIDER SECOND. The decider is a shell `if`, and
    // every builder reads the tool off the FIRST WORD; a pre-wrapped resume
    // command read as `if` → Unknown, and a resumed agent launched with no
    // context, no identity, no nesting guard (glue cut 2 finding). The frozen
    // order injects each branch as a plain tool command, then wraps them.
    let (pre, injected, launch_cmd) = if shape.resuming {
        let (resume_form, fallback_form) =
            resume_forms(&agent.command, agent.tool, &agent.session_id);
        let inj_r =
            launch::inject_ae_context(&resume_form, dir, &agent.slot, &ctx, &agent.launch_id);
        let inj_f =
            launch::inject_ae_context(&fallback_form, dir, &agent.slot, &ctx, &agent.launch_id);
        if let Some(warning) = &inj_r.warning {
            writeln!(err, "{warning}")?;
        }
        // codex resume takes no inline prompt (delivered once its UI returns).
        let inline = if agent.tool == ToolKind::Codex {
            String::new()
        } else {
            prompt.clone()
        };
        let resume_cmd = launch::build_launch_command(&inj_r.cmd, &inline, "", &resume_form);
        let fallback_cmd = launch::build_launch_command(&inj_f.cmd, &inline, "", &fallback_form);
        let decided = if launch::id_probeable(&agent.session_id) {
            launch::resume_decider(
                launch::resume_probe(agent.tool, &agent.session_id).as_deref(),
                &resume_cmd,
                &fallback_cmd,
            )
        } else {
            fallback_cmd
        };
        (resume_form, inj_r, decided)
    } else {
        let pre = launch::inject_session_id(&agent.command, &agent.session_id);
        let injected = launch::inject_ae_context(&pre, dir, &agent.slot, &ctx, &agent.launch_id);
        if let Some(warning) = &injected.warning {
            writeln!(err, "{warning}")?;
        }
        let launch_cmd =
            launch::build_launch_command(&injected.cmd, &prompt, &agent.session_id, &pre);
        (pre, injected, launch_cmd)
    };
    let script =
        match launch::write_launch_script(dir, &agent.slot, &launch_cmd, &agent.session_id, &pre) {
            Ok(script) => script,
            Err(why) => {
                writeln!(
                    err,
                    "ae: could not write the launch script for '{}' ({why}) — agent not started",
                    agent.slot
                )?;
                return Ok(Err(format!("no launch script for {}", agent.slot)));
            }
        };
    // Fire and forget: the reader here IS a shell, and an unconfirmed submit
    // must not abort a launch that may well have taken.
    let _ = deliver::submit_shell_text(
        server,
        &agent.pane,
        &launch::shell_quote(&script.display().to_string()),
    );
    wait_for_agent_start(server, &agent.pane, agent.tool);
    if launch::supports_launch_id(agent.tool) {
        let _ = meta::rewrite(
            dir,
            &format!("launch_time.{}", agent.slot),
            Some(&crate::time::Timestamp::now().epoch().to_string()),
        );
    }
    // codex resume takes no inline prompt, so the prompt is delivered once its
    // UI returns. Every other shape carried it inline already.
    if !prompt.is_empty()
        && agent.tool == ToolKind::Codex
        && launch::is_resume(&injected.cmd, &agent.session_id, &pre)
    {
        deliver_launch_prompt(dir, server, agent, &prompt, err)?;
    }
    Ok(Ok(()))
}

/// The resume form of a profile's command — the frozen `resume_cmd_from_cmd`.
fn resume_forms(cmd: &str, tool: ToolKind, session_id: &str) -> (String, String) {
    match tool {
        ToolKind::Claude => (
            format!("{cmd} --resume {session_id}"),
            format!("{cmd} --continue"),
        ),
        ToolKind::Grok => (
            format!(
                "{} --resume {session_id}",
                launch::strip_grok_session_flags(cmd)
            ),
            format!("{} --continue", launch::strip_grok_session_flags(cmd)),
        ),
        ToolKind::Codex => (
            format!("{} resume {session_id}", launch::strip_session_flags(cmd)),
            launch::strip_session_flags(cmd),
        ),
        ToolKind::Gemini => (
            format!("{cmd} --resume {session_id}"),
            format!("{cmd} --resume latest"),
        ),
        ToolKind::OpenCode => (
            format!("{cmd} --session {session_id}"),
            format!("{cmd} --continue"),
        ),
        ToolKind::Unknown => (cmd.to_owned(), cmd.to_owned()),
    }
}

/// The gated, loud, DURABLE launch-prompt delivery.
///
/// It runs where stderr reaches a pane nobody reads, so a failure is preserved
/// next to the session AND recorded as an event — the frozen contract, kept.
fn deliver_launch_prompt(
    dir: &Path,
    server: &ServerId,
    agent: &Launching,
    prompt: &str,
    err: &mut impl Write,
) -> io::Result<()> {
    let tool = match agent.tool {
        ToolKind::Claude => Tool::Claude,
        ToolKind::Codex => Tool::Codex,
        _ => Tool::Other,
    };
    let reason = if deliver::wait_input_ready(server, &agent.pane, tool, LAUNCH_READY_POLLS) {
        // NO select-pane: `paste-buffer -t` writes to the NAMED pane, and
        // selecting mid-send routes the human's in-flight keystrokes into the
        // target — acute under lead-pair, where two agents share window 0.
        match deliver::stage_and_paste(
            server,
            &format!("ae-launch-{}", agent.slot),
            prompt.as_bytes(),
            &agent.pane,
        ) {
            Ok(()) => {
                let _ = transport::send_key(server, &agent.pane, tmux::Key::Enter);
                return Ok(());
            }
            Err(failure) => format!("submit UNCONFIRMED ({failure:?}) — it may be staged unsent"),
        }
    } else {
        "input never reached a confirmed-ready state within 45s (still initializing, busy, modal, or unreadable)".to_owned()
    };
    let file = dir.join(format!("undelivered.launch-{}.txt", agent.slot));
    let preserved = write_private(&file, prompt).is_ok();
    let _ = crate::state::emit(
        dir,
        &crate::tracked::event_line(&crate::tracked::EventFields {
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
        }),
    );
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
    if !matches!(
        tool,
        ToolKind::Claude | ToolKind::Codex | ToolKind::OpenCode
    ) {
        return;
    }
    for _ in 0..START_POLLS {
        let current = transport::observe_pane_probe(server, pane)
            .map(|probe| probe.command)
            .unwrap_or_default();
        // opencode's process reports as `opencode.exe` — its bun-built launcher.
        if current.strip_suffix(".exe").unwrap_or(&current) == tool.as_str() {
            return;
        }
        if !crate::watchdog::command_is_shell(&current) {
            return;
        }
        std::thread::sleep(START_POLL);
    }
}

/// The monitor window's events pane, created if it is not already there.
///
/// Pinned at window index 99 so it stays RIGHTMOST in the footer — spawned
/// worker windows slot in between it and the main window. Falls back to
/// append-after-current when 99 is somehow taken.
fn ensure_events_pane(server: &ServerId, session: &str, dir: &Path) -> Option<String> {
    if let Some(existing) = monitor_pane(server, session, "_events") {
        return Some(existing);
    }
    let command = vec![dir.join("events-tail").display().to_string()];
    let pane = new_window_running(server, &format!("{session}:99"), "ae-monitor", &command)
        .or_else(|| new_window_running(server, session, "ae-monitor", &command))?;
    let _ = transport::publish_option(
        server,
        tmux::OptionScope::Pane,
        &pane,
        "@ae_agent",
        "_events",
    );
    let _ = transport::set_pane_title(server, &pane, "ae events");
    let _ = transport::run_tmux_op(&argv(
        server,
        &Op::SetWindowOption {
            target: &format!("{session}:ae-monitor"),
            name: "pane-border-status",
            value: "top",
        },
    ));
    let _ = transport::run_tmux_op(&argv(server, &Op::DisablePane { pane: &pane }));
    Some(pane)
}

/// The watchdog pane, split ABOVE the events pane so the visual order stays
/// watchdog-on-top / events-below.
///
/// Skipped when the session's own `watchdog` value, or the config's, says off —
/// the frozen precedence: per-session meta, then config, then default-on.
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
    let _ = transport::publish_option(
        server,
        tmux::OptionScope::Pane,
        &pane,
        "@ae_agent",
        "_watchdog",
    );
    let _ = transport::set_pane_title(server, &pane, "ae watchdog");
    let _ = transport::run_tmux_op(&argv(server, &Op::DisablePane { pane: &pane }));
    let _ = meta::rewrite(dir, "watchdog", Some("true"));
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

/// Attach or switch the client to `session`, and report the exit code.
fn attach(server: &ServerId, env: &Env, session: &str) -> u8 {
    transport::focus(
        server,
        tmux::FocusVerb::for_inside(env.inside_tmux),
        session,
    )
}

// ---------------------------------------------------------------------------
// filesystem and meta reads
// ---------------------------------------------------------------------------

/// Whether `path` is any node at all, LINK INCLUDED.
///
/// `symlink_metadata`, never `metadata`: a dangling symlink is a standing
/// tombstone, and `[[ -e ]]` alone reads one as absent. The teardown checks its
/// tombstones with `lstat` too, so the guard and the core agree.
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
///
/// CANONICAL paths, not raw strings: a symlinked alias and a trailing slash are
/// the same place, and a raw comparison refused the rightful owner and pointed
/// it at a duplicate. A path that cannot be canonicalised falls back to its raw
/// spelling rather than answering "different", which would be a refusal built
/// on a failed read.
fn same_directory(one: &str, other: &str) -> bool {
    canonical(one) == canonical(other)
}

/// One directory, canonicalised, falling back to its raw spelling.
#[allow(
    clippy::disallowed_methods,
    reason = "a door: the derived-name ownership guard compares canonical directories — see clippy.toml"
)]
fn canonical(path: &str) -> String {
    match std::fs::canonicalize(path) {
        Ok(resolved) => resolved.display().to_string(),
        Err(_) => path.trim_end_matches('/').to_owned(),
    }
}

/// One meta value of the session at `dir`, or `None`.
fn meta_value(dir: &Path, key: &str) -> Option<String> {
    let bytes = meta::read_bytes(dir).ok()?;
    meta::first_value(&bytes, key).map(|value| String::from_utf8_lossy(value).into_owned())
}

/// The session directory at 0700 — the frozen `mkdir -p && chmod 0700`.
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

/// Remove the session directory — but ONLY when this attempt created it.
///
/// THREE INDEPENDENT GUARDS stand between a failure and a recursive delete,
/// because the consequence of getting it wrong is unbounded: the name was
/// validated at entry, the path must still test as a direct child of the
/// sessions root, and the ownership flag must say this attempt created it. Any
/// one failing means nothing is deleted.
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

/// A recursive copy of `from` into `to` — the frozen `cp -a`, minus one thing.
///
/// Symlinks are recreated as symlinks and permission bits are preserved.
/// TIMESTAMPS ARE NOT: std offers no portable `utimensat`, and reaching for one
/// would mean `unsafe`, which the crate forbids. A working copy's mtimes are
/// build-system input, so a build in a `--copy` session may do more work than
/// the frozen path would have caused — recorded here rather than discovered.
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
///
/// Under the event log's own lock, so a concurrent append cannot land between
/// the read and the rename and be lost, and strictly non-fatally — a failed
/// trim must never break a resume.
fn trim_events(dir: &Path) {
    let path = dir.join("events.jsonl");
    let Ok(_guard) = crate::state::acquire(&dir.join("events.jsonl.lock"), Duration::from_secs(5))
    else {
        return;
    };
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the resume-time event-log retention reads the log it is about to trim — see clippy.toml"
    )]
    let read = std::fs::read_to_string(&path);
    let Ok(text) = read else { return };
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= EVENTS_KEEP {
        return;
    }
    let mut kept = String::new();
    for line in &lines[lines.len() - EVENTS_KEEP..] {
        kept.push_str(line);
        kept.push('\n');
    }
    let temp = dir.join(format!("events.jsonl.trim.{}", std::process::id()));
    if std::fs::write(&temp, kept).is_ok() && std::fs::rename(&temp, &path).is_ok() {
        return;
    }
    let _ = std::fs::remove_file(&temp);
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
///
/// Each survivor keeps its ORIGINAL index: a retire leaves a hole, and
/// renumbering would break the slot key and orphan any in-flight slotted
/// request addressed to it.
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
        .filter(|agent| launch::supports_launch_id(agent.tool))
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

/// A sibling session's meta directory, given one session's.
#[must_use]
pub fn sibling_session_dir(dir: &Path, session: &str) -> PathBuf {
    dir.parent()
        .map_or_else(|| PathBuf::from(session), |root| root.join(session))
}

/// Every ae session currently running on the same server as the one at `dir`.
///
/// Asked of TMUX, not of the sessions root: the answer wanted is "which of
/// these can I address right now", and a durable record whose server is down is
/// not one. The trade is recorded — a session on a DIFFERENT tmux server is not
/// listed, where the frozen `agents --all` walked `~/.ae/sessions/*/meta` and
/// would have found it.
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
/// One call into the archive module's own preflight, whose refusals are its to
/// word; this only reshapes the tab-separated proof into the three facts the
/// meta records.
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
