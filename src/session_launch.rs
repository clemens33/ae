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
pub const USAGE: &str = "Usage: _launch --home <ae-home> --cwd <dir> [--global <cfg>] [--local <cfg>] [--server-kind <kind>] [--server <value>] [--attach|--no-attach] [--no-autostart] [--] [--worktree|--copy|--local] [--from <uuid>] [use <name>] [<session-name>]";

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
    /// `--no-autostart`: start NEITHER companion. The frozen
    /// `AE_NO_AUTOSTART=1`, which the core cannot read for itself — the
    /// operator's opt-out crosses as a flag like every other environment fact.
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
    ///
    /// The mapping is [`ServerId::from_typed_flags`] and nothing else — the
    /// same rule `read_env` refused on, so the fallback below is unreachable
    /// for anything that got past the parse. Spelling it twice is how the two
    /// drift apart, and the drift is what let `--server-kind ambiguous` record
    /// an unusable pair in meta while building the session somewhere else.
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
///
/// Two shapes, because they read differently to an operator: a word ae does not
/// know is answered with the usage line and the word, while a pair ae knows and
/// REFUSES is answered with the reason and nothing else — printing "offending
/// word: Error: …" in front of a full sentence helps nobody.
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
    // ae cannot type used to fall through to the ambient server, so the launch
    // built the session THERE and recorded the unusable pair in meta. The
    // result was a session on one server described by a record naming another —
    // and since every lifecycle operation now refuses that record, nothing
    // could rename, stop or watch it afterwards.
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
    let (env, args) = match read_env(tail) {
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
        // A relaunch IS a launch (compact's child), so the companions are
        // decided exactly as they are for one typed by hand.
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
                "Session '{session}' is running. Attach with: {}",
                attach_hint(&server, &session)
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
    //
    // The refusal is the WHOLE resume, before the first tmux or filesystem
    // effect, because a session that starts with one seat silently dropped is a
    // session whose roster no longer matches its meta.
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
        if let Err(why) = start_agent(shape, &dir, &core, agent, &server, err)? {
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

    // ---- the companions ----
    //
    // LAST, and after the session exists, deliberately: both guards are
    // TRI-STATE, and a tmux probe taken before any server is running answers
    // UNKNOWN — which refuses every time and would silently disable both
    // companions on a cold machine. The session created above is what makes the
    // server answerable. Both are best-effort and strictly non-fatal (the frozen
    // path ran each behind `2>/dev/null || true`): a session that is up is never
    // failed by a bridge that is not.
    //
    // This runs on RESUME too — same path, same order — which is what makes a
    // reattach revive a bridge that died since the last launch.
    if !env.no_autostart {
        autostart_telegram(env, shape, &dir, &server, err);
        autostart_orchestrator(env, shape, &server, out, err);
    }

    let _ = transport::run_tmux_op(&argv(&server, &Op::SelectPane { pane: &main_pane }));
    if !env.attach {
        writeln!(
            out,
            "Session '{}' started. Attach with: {}",
            shape.name,
            attach_hint(&server, &shape.name)
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
    apply_status_bar(
        server,
        &shape.name,
        &status_paths(
            shape.mode.as_str(),
            &shape.origin.display().to_string(),
            &shape.work_dir.display().to_string(),
        ),
    );
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
/// `paths` is the location segment [`status_paths`] composed; `name` is the
/// session. Takes the two facts rather than a launch `Session` because
/// `ae rename` re-applies the same bar under a new name, and a second copy of
/// this option list is a second thing to keep in step.
pub(crate) fn apply_status_bar(server: &ServerId, name: &str, paths: &str) {
    let paths = tmux::format_literal(paths);
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
pub(crate) fn status_paths(mode: &str, origin: &str, work_dir: &str) -> String {
    match Mode::parse(mode) {
        // An unrecorded or unknown mode reads as local: the frozen `case`'s
        // default arm shows the work dir alone rather than an arrow to nothing.
        Some(Mode::Local) | None => work_dir.to_owned(),
        Some(Mode::Git | Mode::Full) => format!("{origin} → {work_dir}"),
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
        //
        // `ae_path` — the recorded `ae` COMMAND — is NOT written any more. Its
        // one reader was the watchdog's Telegram revive, which shelled back
        // into the glue through it; the revive is the core's own call now, so
        // the row named a binary nobody ran. The flag that fed it (`--glue`)
        // went with it, and an unknown flag is refused exactly as before.
        let ae_core = env.core.as_ref().unwrap_or(&core);
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

/// Hand one agent's pane the command that BECOMES its agent, and wait for the
/// tool to take the pane over.
///
/// `Ok(Ok(()))` started; `Ok(Err(reason))` is the launch-failing refusal the
/// caller rolls back on. A prompt that could not be DELIVERED is not one of
/// them: it is loud and durable, exactly as the frozen path reports it, but the
/// pane is live and the session stands.
///
/// Everything this function used to compose — the context injection, the
/// session-id or resume form, the re-run script and the shell `if` that chose
/// between them — is [`crate::run`]'s now, and runs IN the pane from this
/// session's own state. What is left here is the three things only the
/// launching process can do: guarantee the seat's start marker says what this
/// launch means, paste the line, and wait for the tool to own the pane.
fn start_agent(
    shape: &Session,
    dir: &Path,
    core: &Path,
    agent: &Launching,
    server: &ServerId,
    err: &mut impl Write,
) -> io::Result<Result<(), String>> {
    // THE MARKER IS THE CREATE-VS-RESUME DISCRIMINATOR, so a fresh seat must
    // not inherit one. A stale marker that cannot be removed would make `_run`
    // resume a conversation this launch is not resuming, which is a wrong
    // conversation rather than a missing one — hence a refusal, not a best
    // effort. A RESUMING launch leaves the marker exactly as it found it: that
    // is what says the seat has a conversation at all.
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
    if launch::supports_launch_id(agent.tool) {
        let _ = meta::rewrite(
            dir,
            &format!("launch_time.{}", agent.slot),
            Some(&crate::time::Timestamp::now().epoch().to_string()),
        );
    }
    // codex resume takes no inline prompt, so the prompt is delivered once its
    // UI returns. Every other shape carries it inline, inside the command
    // `_run` composed.
    let prompt = launch::initial_prompt_for(agent.tool);
    if !prompt.is_empty()
        && agent.tool == ToolKind::Codex
        && resuming_seat
        && launch::id_probeable(&agent.session_id)
    {
        deliver_launch_prompt(dir, server, agent, prompt, err)?;
    }
    Ok(Ok(()))
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
    // The pane runs the CORE before it runs the tool: `_run` composes the
    // command and `exec`s it, so for a moment `pane_current_command` is ae's
    // own binary. That is "not started yet" in exactly the way the generated
    // script's `bash` used to be — and without this the not-a-shell arm below
    // would answer READY the instant the paste took.
    let core = std::env::current_exe()
        .ok()
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

/// The monitor window's events pane, created if it is not already there.
///
/// Pinned at window index 99 so it stays RIGHTMOST in the footer — spawned
/// worker windows slot in between it and the main window. Falls back to
/// append-after-current when 99 is somehow taken.
pub(crate) fn ensure_events_pane(server: &ServerId, session: &str, dir: &Path) -> Option<String> {
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

/// The Telegram bridge, revived if `[telegram] enabled` asks for one.
///
/// The config read is the GLOBAL one — the file the daemon itself will be
/// handed as `--config`. An origin-local `.ae/config` is deliberately not
/// overlaid here: the frozen glue decided intent from the overlay but started
/// the daemon on the global file, so a project-local `enabled = true` produced a
/// bridge reading a config that never said so.
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

/// The orchestrator companion, started in the background when a scaffold exists.
///
/// Opt-in purely by EXISTING STATE — a scaffold from `ae orchestrator --init`,
/// canonical or the legacy hub layout — so there is no new config key and a
/// machine that never scaffolded one is never touched. The `AE_ORCHESTRATOR_DIR`
/// / `AE_HUB_DIR` overrides the frozen probe honoured are dropped: the core does
/// not read the environment, and an override nobody in this process can see is
/// not a probe, it is a guess.
fn autostart_orchestrator(
    env: &Env,
    shape: &Session,
    server: &ServerId,
    out: &mut impl Write,
    err: &mut impl Write,
) {
    // The recursion guard, and it is structural rather than an environment
    // variable: the companion IS an `ae orchestrator` launch, which lands right
    // here, and a session already named for the scaffold does not start itself.
    if matches!(shape.name.as_str(), "orchestrator" | "hub") {
        return;
    }
    // The SCAFFOLD decides the session name: a legacy `hub.config` keeps running
    // as `hub`, because its baked charter paths and its resume state are that
    // name's. Canonical first, exactly as the frozen probe ordered them.
    let scaffolds = [
        (
            "orchestrator",
            env.home.join("orchestrator/orchestrator.config"),
        ),
        ("hub", env.home.join("meta-hub/hub.config")),
    ];
    let Some((session, config)) = scaffolds
        .iter()
        .find(|(_, config)| crate::lifecycle::path_exists(config))
    else {
        return;
    };
    // TRI-STATE (#28): only a VERIFIED absence may start a companion. An
    // unanswerable server cannot rule out an orchestrator that is already
    // running, and the cost of guessing wrong is a duplicate session whose
    // detached launch dies with its output on /dev/null. `verify_session_absent`
    // is the probe with that third answer — and it reads a cleanly EXITED server
    // as absence, which a plain enumeration cannot.
    let mut unknown = false;
    for (name, _) in &scaffolds {
        match transport::verify_session_absent(server, name) {
            tmux::StopProbe::Present => return,
            tmux::StopProbe::Unknown => unknown = true,
            tmux::StopProbe::Absent => {}
        }
    }
    if unknown {
        let _ = writeln!(
            err,
            "Orchestrator autostart skipped — tmux did not answer, so a running orchestrator cannot be ruled out."
        );
        return;
    }
    // The child is a LAUNCH BY THIS CORE, not a re-entry through the glue: the
    // glue's `orchestrator` arm was retired and refuses, so the frozen
    // trampoline's `CONFIG_FILE` + `cd` rewrite has to cross as the flags
    // `_launch` already reads. The scaffold's own directory is its cwd.
    let Some(dir) = config.parent() else {
        return;
    };
    let Some(argv) = crate::lifecycle::orchestrator_argv(&crate::lifecycle::Companion {
        home: &env.home,
        dir,
        config,
        server_kind: &env.server_kind,
        server_value: &env.server_value,
        session,
    }) else {
        return;
    };
    let _ = writeln!(
        out,
        "Starting orchestrator companion session in the background (AE_NO_AUTOSTART=1 skips)."
    );
    let _ = transport::run_detached(&argv);
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
///
/// It used to say `ae orchestrator --attach`, which is not a command any more:
/// the orchestrator trampoline was retired in the glue cut and that word now
/// REFUSES. It was wrong before that too — it named the companion session
/// rather than the one just started, so a human who followed it landed
/// somewhere else or nowhere.
///
/// The server is spelled out because a session on a named or socket server is
/// invisible to a bare `tmux attach`, and this line is meant to be pasted.
fn attach_hint(server: &ServerId, session: &str) -> String {
    let session = paste_safe(session);
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

/// `word` as it can be pasted into a shell: bare when nothing in it is
/// significant there, quoted when anything is.
///
/// Unconditional quoting would be just as CORRECT and worse to read — a session
/// name is allowlisted to `[A-Za-z0-9_-]` and can never need it, so quoting one
/// only teaches the reader that ae does not know its own grammar. A socket path
/// is not allowlisted, so it is checked rather than assumed.
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
///
/// TOLERANT, and it must stay that way: its callers are OBSERVERS — the pane
/// surfaces, the capture child — for which a record that names no server is
/// answered correctly by the ambient one. A lifecycle operation that ACTS on a
/// session wants [`recorded_server_resolved`] instead.
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
///
/// `None` is the fail-closed answer, and only for [`ServerSelector::Ambiguous`]:
/// an unknown kind, a typed empty value, a relative socket path, an empty kind
/// beside a value, or duplicate selector keys. Such a record names a server ae
/// cannot identify, and falling back to the ambient one is not a smaller
/// answer — it is a DIFFERENT server, where a same-named session belonging to
/// somebody else will answer to a rename, a kill or a watchdog.
///
/// [`ServerSelector::Missing`] is NOT refused, and that is the whole reason
/// this is not simply `entitles()`. A launch records the two rows only when the
/// caller resolved a server (`meta_document`, from `--server-kind`), and the
/// glue resolves none for a caller that is not inside tmux and sets no
/// `AE_TMUX_SERVER` — so a session started from a plain terminal legitimately
/// records nothing, and the ambient server is exactly the one it is on.
/// Refusing that would make `rename` and the watchdog commands fail for the
/// most ordinary session there is.
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
