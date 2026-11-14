//! `_launch`: a whole session, created or resumed, as ONE core operation.
//!
//! The whole launch path: the flag parse, the explicit session name, the
//! teardown tombstones, the `--from` preflight, the
//! working-directory modes, the tmux session and its layout, the meta publish,
//! the session assets, the pane commands and their readiness-gated paste, the
//! monitor panes, and the attach.

use std::io::{self, Write};
use std::ops::ControlFlow;
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
#[cfg(test)]
#[path = "session_launch/effort_trim_bounded_spec.rs"]
mod effort_trim_bounded_spec;
#[cfg(test)]
#[path = "session_launch/effort_trim_damage_spec.rs"]
mod effort_trim_damage_spec;
#[cfg(test)]
#[path = "session_launch/effort_trim_spec.rs"]
mod effort_trim_spec;
pub(crate) mod name;
#[cfg(test)]
#[path = "../tests/it/planattach_spec.rs"]
mod planattach_spec;
mod preflight;
mod ready;
mod record;
mod stamp;
#[cfg(test)]
mod start_spec;
#[cfg(test)]
mod start_wiring_spec;
#[cfg(test)]
mod watch_spec;

pub use capture::developer_record_carries;
#[cfg(test)]
use preflight::check_resume_target;
use preflight::{
    adopt_recorded_config, carries_own_proof, check_launch_kind, check_name, check_resumed_roster,
    check_seat_overrides, check_spawned_labels, check_stopped_near_miss, lex_spawned_seats,
    lifecycle_busy, preflight_server, prepare_working_copy, prove_lineage, read_workspace_values,
    resolve_seats, resolve_server, resolve_watchdog_cadences, restamp_running, restore_roster,
    seed_orchestrator_seat,
};
pub(crate) use ready::{
    OnHumanPrompt, TurnOutcome, deliver_launch_turn, launch_token, launch_turn_is_pasted,
    wait_for_agent_start,
};
#[cfg(test)]
use ready::{Readiness, ReadyRead, Start, await_start, pane_start, settle};
use ready::{chunk_polls, start_agent};
use record::meta_document;
pub use record::{
    ProvenIdentity, SeedOutcome, pane_proven_identity, publish_meta_and_seed_uuid,
    seed_session_uuid,
};
use stamp::{
    apply_layout, assert_status_bindings, open_first_chat, stamp_pane, stamp_session, stamp_windows,
};
pub(crate) use stamp::{
    ensure_events_pane, look_of, name_agent_window, rebind_monitor_panes, redress_status_bar,
    stamp_client_session_hook, stamp_lead_pair_policy, stamp_main_pane, stamp_window, status_paths,
    watchdog_enabled_for_session,
};
#[cfg(test)]
use stamp::{
    layout_argvs, lead_pair_policy_argvs, replace_watchdog_registration, window_zero_chat,
};

/// The usage line for the core entry.
pub const USAGE: &str = "Usage: _launch --home <ae-home> --cwd <dir> [--global <cfg>] [--local <cfg>] [--server-kind <kind>] [--server <value>] [--caller-socket <path>] [--attach|--no-attach] [--no-autostart] [--] [--worktree|--copy|--local] [--dir <path>] [--no-attach] [--tmux] [--resume] [--solo] [--from <uuid>] [--seat <agent>=<profile>] [--lead <profile>] [--colead <profile>] [use <name>] <session-name>";

/// The public refusal for a launch argv that names no session.
pub const MISSING_NAME: &str = "Error: a session needs a name: ae <name> [...]";

/// The public launch usage line paired with [`MISSING_NAME`].
pub const PUBLIC_USAGE: &str = "Usage: ae <name> [--local|--copy|--worktree] [--dir <path>] [--no-attach] [--tmux] [--resume] [--solo] [--from <archive-uuid>] [--seat <agent>=<profile>] [--lead <profile>] [--colead <profile>] [use <agent>]";

/// How long a freshly created pane's shell is given to draw its prompt before
/// anything is pasted.
const SHELL_SETTLE: Duration = Duration::from_millis(300);

/// How many polls the launch prompt's readiness wait takes.
const LAUNCH_READY_POLLS: u32 = 90;

/// How many readiness polls one chunk takes before the pane is read for a
/// human-only prompt.
const READY_CHUNK: u32 = 2;

/// How many chunks the chunked wait takes: the whole launch budget.
const READY_CHUNKS: u32 = LAUNCH_READY_POLLS / READY_CHUNK;

/// How many polls the tool-process wait takes.
const START_POLLS: u32 = 10;

/// How many readings of tmux's forked child that wait takes on top: a pane reads
/// so until its child execs, which a stalling session bus holds up for a second
/// (#267). They spend none of `START_POLLS`, so a tool that never starts costs
/// no more on a host that does not stall, and at most this many polls on one
/// that does.
const FORK_POLLS: u32 = 15;

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

/// Re-prove a captured server: it answers with the same pid and start time.
pub(crate) fn prove_server(
    server: &ServerId,
    server_pid: &str,
    server_start: &str,
) -> Result<(), String> {
    let Some(observed) = transport::observe_server_identity(server) else {
        return Err("the invoking tmux server no longer answers".to_owned());
    };
    if observed.pid != server_pid || observed.start != server_start {
        return Err("the invoking tmux server was replaced".to_owned());
    }
    Ok(())
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
        prove_server(&self.server, &self.server_pid, &self.server_start)?;
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

/// The `chat-window-failed` action: a launch or a rename that could not leave
/// the session's chat window running names the failure here.
pub(crate) const CHAT_FAILED_ACTION: &str = "chat-window-failed";

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
    /// `--tmux` — the tmux view, never the app. The entry reads it; `_launch`
    /// accepts it in the user argv and ignores it, so the launch is today's.
    pub tmux: bool,
    /// `--resume` — the caller means an EXISTING session: it never creates
    /// one, and it is the explicit intent that lets a session named like an
    /// ae command resume (`ae sessions` alone never does, #284).
    pub resume: bool,
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
    PlanParser {
        plan: Plan::default(),
        rest: args,
        allow_attach,
        colead_override: false,
    }
    .parse()
}

/// The launch grammar's cursor and accumulated decisions. Values are consumed
/// verbatim; only a word in the option position is interpreted as a flag.
struct PlanParser<'a> {
    plan: Plan,
    rest: &'a [String],
    allow_attach: bool,
    colead_override: bool,
}

impl PlanParser<'_> {
    fn parse(mut self) -> Result<Plan, String> {
        while let [word, tail @ ..] = self.rest {
            self.rest = tail;
            self.parse_word(word)?;
        }
        if self.colead_override && self.plan.workers.as_deref() == Some("") {
            return Err(
                "Error: '--solo' starts the lead alone; drop --colead/--seat <worker>.".to_owned(),
            );
        }
        Ok(self.plan)
    }

    fn parse_word(&mut self, word: &str) -> Result<(), String> {
        match word {
            "--worktree" => self.plan.mode = Some(Mode::Git),
            "--copy" => self.plan.mode = Some(Mode::Full),
            "--local" => self.plan.mode = Some(Mode::Local),
            "--attach" if self.allow_attach => self.plan.attach = Some(true),
            "--no-attach" => self.plan.attach = Some(false),
            "--tmux" if !self.allow_attach => self.plan.tmux = true,
            "--resume" if !self.allow_attach => self.plan.resume = true,
            "--solo" if !self.allow_attach => self.plan.workers = Some(String::new()),
            _ if word == "--dir" || word.starts_with("--dir=") => {
                self.directory(word.strip_prefix("--dir="))?;
            }
            "use" => {
                let Some((value, tail)) = self.rest.split_first() else {
                    return Err("Error: 'use' requires an agent NAME bound in [roster] (e.g. ae myproject use colead)".to_owned());
                };
                self.plan.main = Some(value.clone());
                self.rest = tail;
            }
            _ if !self.allow_attach
                && (word == "--lead"
                    || word.starts_with("--lead=")
                    || word == "--colead"
                    || word.starts_with("--colead=")
                    || word == "--seat"
                    || word.starts_with("--seat=")) =>
            {
                self.seat_profile(word)?;
            }
            _ if word == "--from" || word.starts_with("--from=") => {
                self.archive(word.strip_prefix("--from="))?;
            }
            _ if word.starts_with("--") => {
                return Err(format!(
                    "Error: unknown flag '{word}'. Use --worktree, --copy, --local, --dir <path>, --no-attach, --tmux, --resume, --solo, --from <archive-uuid>, --seat <agent>=<profile>, --lead <profile>, or --colead <profile>."
                ));
            }
            // A launch takes exactly one session name: a second positional is
            // a usage error, never a rename of the first.
            _ => {
                if let Some(first) = self.plan.name.as_deref() {
                    return Err(second_positional_refusal(first, word));
                }
                self.plan.name = Some(word.to_owned());
            }
        }
        Ok(())
    }

    /// Inline values never advance the cursor. A missing split value is empty;
    /// each flag owns the validation and the order of its duplicate check.
    fn value(&mut self, inline: Option<&str>) -> String {
        if let Some(value) = inline {
            return value.to_owned();
        }
        match self.rest.split_first() {
            Some((value, tail)) => {
                self.rest = tail;
                value.clone()
            }
            None => String::new(),
        }
    }

    fn directory(&mut self, inline: Option<&str>) -> Result<(), String> {
        if self.plan.dir.is_some() {
            return Err(
                "Error: --dir may be given only once — a launch has one origin.".to_owned(),
            );
        }
        let value = self.value(inline);
        if value.is_empty() {
            return Err("Error: --dir requires a path.".to_owned());
        }
        self.plan.dir = Some(value);
        Ok(())
    }

    fn archive(&mut self, inline: Option<&str>) -> Result<(), String> {
        if self.plan.from.is_some() {
            return Err("Error: --from may be given only once — a session inherits from exactly one archive.".to_owned());
        }
        let value = self.value(inline);
        if value.is_empty() {
            return Err(
                "Error: --from requires an archive UUID (list them: ls ~/.ae/archive).".to_owned(),
            );
        }
        self.plan.from = Some(value);
        Ok(())
    }

    fn seat_profile(&mut self, word: &str) -> Result<(), String> {
        if word == "--seat" || word.starts_with("--seat=") {
            let raw = self.value(word.strip_prefix("--seat="));
            let Some((agent, profile)) = raw.split_once('=') else {
                return Err(
                    "Error: --seat requires <agent>=<profile> (e.g. --seat lead=solx).".to_owned(),
                );
            };
            if agent.is_empty() || profile.is_empty() {
                return Err("Error: --seat requires non-empty <agent>=<profile>.".to_owned());
            }
            return self.add_profile(agent, profile.to_owned());
        }
        let (agent, inline) = if word == "--lead" || word.starts_with("--lead=") {
            ("lead", word.strip_prefix("--lead="))
        } else {
            // Only the --colead sugar sets this parse-time solo conflict.
            self.colead_override = true;
            ("colead", word.strip_prefix("--colead="))
        };
        let profile = self.value(inline);
        if profile.is_empty() {
            return Err(format!("Error: --{agent} requires a profile."));
        }
        self.add_profile(agent, profile)
    }

    fn add_profile(&mut self, agent: &str, profile: String) -> Result<(), String> {
        if self
            .plan
            .seat_profiles
            .iter()
            .any(|(seen, _)| seen == agent)
        {
            return Err(format!(
                "Error: profile override for agent '{agent}' may be given only once."
            ));
        }
        self.plan.seat_profiles.push((agent.to_owned(), profile));
        Ok(())
    }
}

/// The refusal for a launch argv naming two sessions: it names both words,
/// and when one of them is a session helper it says how a helper is called —
/// a mistyped helper call must fail closed, not open a session named after
/// the last word.
fn second_positional_refusal(first: &str, second: &str) -> String {
    let helper = [first, second]
        .into_iter()
        .find(|word| crate::shim::lookup(word).is_some());
    match helper {
        Some(helper) => format!(
            "Error: a launch takes exactly one session name, not two ('{first}' and '{second}'). \
             '{helper}' is a session helper — run it by its full path \
             (<state-root>/sessions/<session>/{helper} …) or as 'ae @<session> {helper} …'."
        ),
        None => format!(
            "Error: a launch takes exactly one session name, not two ('{first}' and '{second}')."
        ),
    }
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
    /// `--no-autostart`: start no companion — neither the watchdog nor the
    /// Telegram bridge.
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
pub fn run(
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    run_with_expected(tail, None, None, out, err)
}

/// Settings-only entry into the same launch owner with a locked expectation.
pub(crate) fn run_expected(
    preamble: &crate::entry::Preamble,
    user: &[String],
    expected: &ExpectedLaunch,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    let argv = expected_launch_argv(preamble, user, expected);
    run_with_expected(&argv[1..], Some(expected), None, out, err)
}

/// One restored session: the ordinary launch owner, told the saved facts the
/// scan judged by so it can re-check them under the lifecycle lock.
pub(crate) fn run_restore(
    preamble: &crate::entry::Preamble,
    name: &str,
    attempt: &crate::restore::Attempt,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    let user = [name.to_owned(), "--no-attach".to_owned()];
    let argv = preamble.launch_argv(&user);
    run_with_expected(&argv[1..], None, Some(attempt), out, err)
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
    restore: Option<&crate::restore::Attempt>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
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
        env.local = crate::doors::local_config(&origin, env.global.as_deref());
        env.cwd = origin;
    }
    if let Some(attach) = plan.attach {
        env.attach = attach;
    }
    launch(&env, &plan, None, expected, restore, out, err)
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
) -> crate::error::Result<u8> {
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
        tmux: false,
        resume: false,
    };
    launch(&env, &launch_plan, Some(plan.proof), None, None, out, err)
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
    split_profile_client(value).ok_or_else(|| {
        format!(
            "Error: --seat selection '{value}' is not <profile> or <profile>@<client> \
             (e.g. --seat lead=solx, --lead fablex@cc-mic)."
        )
    })
}

/// The grammar every `profile@client` spelling shares (launch `--seat`, spawn
/// `--using`): one `@` splits a profile from its client, no `@` is a bare
/// profile, and an empty half or a second `@` is `None` for the caller to
/// word.
pub(crate) fn split_profile_client(value: &str) -> Option<(String, Option<String>)> {
    let Some((profile, client)) = value.split_once('@') else {
        return Some((value.to_owned(), None));
    };
    if profile.is_empty() || client.is_empty() || client.contains('@') {
        return None;
    }
    Some((profile.to_owned(), Some(client.to_owned())))
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

/// The one read-only launch-config resolver. Preflight supplies the defaults
/// that the launch will seed later; a resume reads its recorded files as-is.
fn launch_identity(
    env: &Env,
    snapshot: Option<&SeatOverrideSnapshot>,
    before_seed: bool,
) -> Result<IdentityConfig, config::ConfigError> {
    if let Some(snapshot) = snapshot {
        return Ok(snapshot.cfg.clone());
    }
    let orchestrator_seat = env
        .local
        .as_deref()
        .is_some_and(|local| crate::orchestrator::is_seat_overlay(local, &env.home));
    let local = if orchestrator_seat {
        None
    } else {
        env.local.as_deref()
    };
    let mut cfg = if before_seed {
        config::read_identity_with_global_default(
            env.global.as_deref(),
            local,
            crate::entry::DEFAULT_CONFIG,
        )?
    } else {
        config::read_identity(env.global.as_deref(), local)?
    };
    if orchestrator_seat {
        // The dedicated overlay contributes only workspace choices. Its legacy
        // profile/roster sections are deliberately ignored at both call sites.
        let defaults = if before_seed && !env.local.as_deref().is_some_and(node_exists) {
            config::parse_identity(crate::orchestrator::DEFAULT_CONFIG)?
        } else {
            IdentityConfig::default()
        };
        let keys = config::read_workspace_keys(None, env.local.as_deref(), &["main", "workers"]);
        cfg.main = keys[0].clone().or(defaults.main).or(cfg.main);
        cfg.workers = keys[1].clone().or(defaults.workers).or(cfg.workers);
    }
    Ok(cfg)
}

/// Existing metadata deliberately bypasses this creation-only reservation.
fn reserved_launch_actor(
    env: &Env,
    plan: &Plan,
    snapshot: Option<&SeatOverrideSnapshot>,
) -> Option<String> {
    let mut cfg = launch_identity(env, snapshot, true).ok()?;
    if let Some(workers) = &plan.workers {
        cfg.workers = Some(workers.clone());
    }
    config::launch_plan(&cfg, plan.main.as_deref(), crate::doors::home().as_deref())
        .ok()?
        .seats
        .into_iter()
        .find(|seat| config::is_reserved_actor(&seat.name))
        .map(|seat| seat.name)
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

/// The same gate for a spawn: `profile@client` for the agent `agent`, a seat
/// with no recorded store — a FIRST START, so a pairing whose store the
/// launcher cannot verify refuses (F6) instead of recording an unproven fact.
///
/// This is [`resolve_client_override`] itself plus the first-start store proof
/// the launch takes in `resolve_one_override`; a spawn never has its own copy
/// of the adapter gate. The caller has already judged approval and existence,
/// so the unknown-name texts (which speak of `--seat`) are not reached here.
///
/// # Errors
///
/// The refusal line, whichever of the adapter gate, the expansion or the store
/// proof refused.
pub(crate) fn resolve_spawn_pair(
    cfg: &IdentityConfig,
    agent: &str,
    profile: &str,
    label: &str,
    home: Option<&Path>,
) -> Result<(config::ResolvedCommand, crate::launch_cmd::SimpleCommand), String> {
    let line = |refusal: SeatOverrideRefusal| match refusal {
        SeatOverrideRefusal::Usage(line) | SeatOverrideRefusal::Failed(line) => line,
    };
    let profiles: Vec<&str> = cfg.profiles.iter().map(|(name, _)| name.as_str()).collect();
    let clients: Vec<&str> = cfg.clients.iter().map(|(name, _)| name.as_str()).collect();
    let (command, parsed) = resolve_client_override(
        cfg,
        agent,
        profile,
        label,
        home,
        &known_list(&profiles),
        &known_list(&clients),
    )
    .map_err(line)?;
    if let Some(reason) = current_pairing_proof(&command, parsed.tool(), home) {
        return Err(line(unknown_store_refusal(
            agent,
            &format!("{profile}@{label}"),
            &reason,
        )));
    }
    Ok((command, parsed))
}

/// A `profile@client` selection for a NEW choice — `spawn --using` or
/// `reseat --using` — resolved to its command, or refused. The order is the
/// contract, every step before any effect:
///
/// 1. the session's recorded GLOBAL config is the only identity read — a
///    project overlay can neither approve a pair nor redefine an approved
///    name, so a pair means exactly what the human's own config says;
/// 2. the pair must be on the global `[client_pairs]` list — POLICY before
///    existence, so an unapproved pair learns nothing about which names exist;
/// 3. the profile and the client must exist;
/// 4. the launch's own adapter gate and first-start store proof
///    ([`resolve_spawn_pair`]).
///
/// `nothing` closes the refusals that name the selection, in the caller's own
/// verb (`Nothing was spawned.`).
///
/// # Errors
///
/// The refusal line, whichever step refused.
pub(crate) fn approved_pair(
    global: Option<&Path>,
    agent: &str,
    profile: &str,
    label: &str,
    home: Option<&Path>,
    nothing: &str,
) -> Result<(config::ResolvedCommand, crate::launch_cmd::SimpleCommand), String> {
    let spelling = format!("{profile}@{label}");
    let Some(global) = global else {
        return Err(format!(
            "Error: --using '{spelling}': this session records no global config, so no pair can be approved. {nothing}"
        ));
    };
    // ONE read of the global file feeds both the identity and the approved
    // list, so an approval and the command it approves cannot straddle an edit.
    let text = match config::read_selected(global) {
        Ok(text) => text,
        Err(config::ConfigError::Unreadable(path)) => {
            return Err(format!(
                "Error: --using '{spelling}': the global config {} cannot be read, so no pair can be approved. {nothing}",
                path.display()
            ));
        }
        Err(why) => return Err(why.to_string()),
    };
    let cfg = config::parse_identity_in(global, &text).map_err(|why| why.to_string())?;
    let verdict = crate::pairs::parse_in(global, &text).judge(profile, label);
    if let Some(why) = verdict.refusal(profile, global) {
        return Err(format!("Error: --using '{spelling}' {why} {nothing}"));
    }
    if cfg.profile(profile).is_none() {
        return Err(format!(
            "Error: profile '{profile}' not defined in [profiles] of {}",
            global.display()
        ));
    }
    if cfg.client(label).is_none() {
        return Err(format!(
            "Error: client '{label}' is not defined in [clients] of {}.",
            global.display()
        ));
    }
    resolve_spawn_pair(&cfg, agent, profile, label, home)
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
            return Err(SeatOverrideRefusal::Usage(cross_tool_refusal(
                agent, old, new,
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

/// A resumed seat whose command now launches another tool than it recorded.
fn cross_tool_refusal(agent: &str, old: ToolKind, new: ToolKind) -> String {
    format!(
        "Error: cannot change agent '{agent}' from {} to {} on resume — its recorded conversation cannot cross tool kinds.",
        old.as_str(),
        new.as_str()
    )
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

/// Every refusal a launch owes BEFORE it writes anything: its seat overrides,
/// then — for a resume of a stopped session — its restored profiles.
fn validate_seat_overrides(
    env: &Env,
    plan: &Plan,
    dir: &Path,
    resuming: bool,
    running: bool,
) -> Result<Option<SeatOverrideSnapshot>, SeatOverrideRefusal> {
    let snapshot = seat_override_snapshot(env, plan, dir, resuming, running)?;
    if resuming && !running {
        let refusal = match &snapshot {
            Some(held) => restored_profiles_refusal(plan, dir, &held.cfg, &held.overrides),
            // NAMED RESIDUAL: ANY session named orchestrator skips this (keyed on
            // the name, not the seat overlay); the dedicated seat reads its
            // identity from the global config alone, so only its own launch
            // refuses, after the migration writes (version step, v2 `tag_priors`).
            None if plan.name.as_deref() == Some(crate::orchestrator::ORCHESTRATOR_SESSION) => None,
            None => {
                restored_profiles_refusal(plan, dir, &override_identity(env, dir, resuming)?, &[])
            }
        };
        if let Some(line) = refusal {
            return Err(SeatOverrideRefusal::Failed(line));
        }
    }
    Ok(snapshot)
}

fn seat_override_snapshot(
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

/// The refusal a resume owes for a restored profile that no longer resolves,
/// decided READ-ONLY, before the migration chain writes the meta: the same
/// [`restore_saved_roster`] the launch runs, over a scratch copy of the seats
/// `cfg` plans. It reads only the `seat.<slot>` and `profile.<slot>` rows,
/// which the chain never moves. A config whose roster is not launchable is
/// refused here with the launch's own text, since that refusal would otherwise
/// come after the write.
fn restored_profiles_refusal(
    plan: &Plan,
    dir: &Path,
    cfg: &IdentityConfig,
    replaced: &[ResolvedSeatOverride],
) -> Option<String> {
    let mut planned = cfg.clone();
    if let Some(workers) = &plan.workers {
        planned.workers = Some(workers.clone());
    }
    let home = crate::doors::home();
    let mut seats = match config::launch_plan(&planned, plan.main.as_deref(), home.as_deref()) {
        Ok(resolved) => resolved.seats,
        Err(violations) => {
            return Some(config::render_violations(&violations).trim_end().to_owned());
        }
    };
    let session = plan.name.as_deref().unwrap_or_default();
    let replaced: Vec<&str> = replaced.iter().map(|o| o.agent.as_str()).collect();
    restore_saved_roster(dir, session, cfg, home.as_deref(), &replaced, &mut seats).err()
}

/// Replace every command-derived field of `seat` with a resolved profile's.
fn rebind_command(
    seat: &mut Seat,
    command: &config::ResolvedCommand,
    parsed: &crate::launch_cmd::SimpleCommand,
) {
    command.clone_into(&mut seat.command);
    seat.assign_span.clone_from(&parsed.assign_span);
    seat.argv_span.clone_from(&parsed.argv_span);
    seat.binary.clone_from(&parsed.binary);
    seat.tool = parsed.tool();
}

/// Replace one resolved seat's profile and every command-derived field.
fn reprofile_seat(seat: &mut Seat, replacement: &ResolvedSeatOverride) {
    replacement.profile.clone_into(&mut seat.profile);
    seat.client_override.clone_from(&replacement.client);
    seat.store_proven = replacement.store_proven;
    rebind_command(seat, &replacement.command, &replacement.parsed);
}

/// A RESUME RESTORES THE ROSTER IT SAVED: each seat's recorded name and profile
/// replace what the current config binds, and the seat's command-derived facts
/// (command, spans, binary, tool) are re-resolved from that RESTORED profile —
/// they are what `agent_bin.<slot>` and every launch decision read, so they must
/// name the tool the recorded profile actually launches, never the tool the
/// config's own binding for the slot would. A seat an explicit or recorded-label
/// override will replace is left to that override.
///
/// # Errors
///
/// The refusal line for a restored profile that no longer resolves, before any
/// write.
fn restore_saved_roster(
    dir: &Path,
    session: &str,
    cfg: &IdentityConfig,
    home: Option<&Path>,
    overridden: &[&str],
    seats: &mut [Seat],
) -> Result<(), String> {
    for seat in seats {
        if let Some(saved) =
            meta_value(dir, &format!("seat.{}", seat.slot)).filter(|v| !v.is_empty())
        {
            seat.name = saved;
        }
        let Some(saved) =
            meta_value(dir, &format!("profile.{}", seat.slot)).filter(|v| !v.is_empty())
        else {
            continue;
        };
        if saved != seat.profile && !overridden.contains(&seat.name.as_str()) {
            let refuse = |cause: String| {
                format!(
                    "Error: session '{session}' cannot resume: seat '{}' ({}) records profile \
                     '{saved}', but {cause} — nothing was resumed. Restore the profile in the \
                     config, or resume with an explicit override (ae {session} --seat {}=<profile>).",
                    seat.name, seat.slot, seat.name
                )
            };
            let command = cfg
                .command(&saved, home)
                .map_err(|why| refuse(why.to_string()))?;
            let command =
                command.ok_or_else(|| refuse("[profiles] no longer defines it".into()))?;
            let parsed = crate::launch_cmd::lex_simple_command(command.as_str())
                .map_err(|why| refuse(format!("it is not one simple command — it has {why}")))?;
            rebind_command(seat, &command, &parsed);
        }
        saved.clone_into(&mut seat.profile);
    }
    Ok(())
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

/// The tmux server a saved session's record names, as a launch probes it.
#[derive(Debug, PartialEq, Eq)]
struct RecordedServer {
    server: ServerId,
    /// A pre-pair record with no server rows: it is probed on the historical
    /// default server, and nothing there may be assumed to be ours.
    legacy: bool,
}

impl RecordedServer {
    /// `None` for a record whose server rows do not name exactly one server.
    fn of(selector: ServerSelector) -> Option<Self> {
        match selector {
            ServerSelector::Positive(selector) => Some(Self {
                server: ServerId::Selected(selector),
                legacy: false,
            }),
            ServerSelector::Missing => Some(Self {
                server: crate::doors::historical_server(),
                legacy: true,
            }),
            ServerSelector::Ambiguous => None,
        }
    }
}

/// Where a saved session stands on its recorded server, and so which server a
/// launch of it uses.
#[derive(Debug, PartialEq, Eq)]
enum Placement {
    /// Still running there: reattach, never rebuild.
    Running,
    /// Proved absent there: rebuild on the destination.
    Stopped(Destination),
    /// Neither proved: refuse, never assume absent.
    Unproven,
}

/// The server a stopped session is rebuilt on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Destination {
    /// The recorded server: an in-place resume (the picker's or settings'
    /// captured Resume, or a crash restore) lands where it was.
    Recorded,
    /// The server this launch was asked on: an ordinary resume re-pairs there,
    /// and a legacy record has no server of its own to return to.
    Proposed,
}

/// What placing a saved session needs besides its record: the server this
/// launch was asked on, whether the launch resumes in place, and the crash
/// restore it serves, if any.
#[derive(Clone, Copy)]
struct Placing<'a> {
    proposed: &'a ServerId,
    in_place: bool,
    restore: Option<&'a crate::restore::Attempt>,
}

/// Decide a saved session's [`Placement`] from the absence probe of its
/// recorded server.
fn placement(probe: &tmux::StopProbe, in_place: bool, legacy: bool) -> Placement {
    match probe {
        tmux::StopProbe::Present => Placement::Running,
        tmux::StopProbe::Absent if in_place && !legacy => Placement::Stopped(Destination::Recorded),
        tmux::StopProbe::Absent => Placement::Stopped(Destination::Proposed),
        tmux::StopProbe::Unknown => Placement::Unproven,
    }
}

/// The refusal of a launch whose recorded server answered neither way. A crash
/// restore skips a positively recorded session instead; a legacy one refuses
/// even there.
fn refuse_unproven(
    restore: Option<&crate::restore::Attempt>,
    recorded: &RecordedServer,
    session: &str,
    why: Option<String>,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    if !recorded.legacy
        && let Some(attempt) = restore
    {
        attempt.skip(crate::restore::unproven(why.as_deref()));
        return Ok(0);
    }
    if recorded.legacy {
        writeln!(
            err,
            "Error: cannot verify whether legacy tmux session '{session}' is absent on the historical default server. Metadata was not changed."
        )?;
    } else {
        writeln!(
            err,
            "Error: cannot verify whether tmux session '{session}' is absent on its recorded server. Metadata was not changed."
        )?;
    }
    if let Some(why) = why {
        writeln!(err, "       {why}")?;
    }
    Ok(EXIT_FAILED)
}

/// A watchdog cadence in seconds, META-WINS: the session's recorded row on a
/// resume, else the configured value, else `default`. `configured` runs only
/// when nothing is recorded, so a resume never reads, nor refuses on, the
/// current config's value. Whichever wins must be unsigned seconds; `Err`
/// carries the offending text, the configured read's own included.
fn recorded_wins_seconds(
    recorded: Option<String>,
    configured: impl FnOnce() -> Result<Option<String>, String>,
    default: u64,
) -> Result<String, String> {
    let configured = if recorded.is_none() {
        configured()?
    } else {
        None
    };
    let value = recorded
        .or(configured)
        .unwrap_or_else(|| default.to_string());
    if value.parse::<u64>().is_err() {
        return Err(value);
    }
    Ok(value)
}

/// A launch's working-copy mode: the flag, then the recorded one, then the
/// configured `copy`, then local.
fn launch_mode(flag: Option<Mode>, recorded: Option<Mode>, configured: &str) -> Mode {
    flag.or(recorded)
        .unwrap_or_else(|| Mode::parse(configured).unwrap_or(Mode::Local))
}

/// Whether the chat opens as the session's first window, and the reason for
/// the one note a malformed `[workspace] chat` earns. A meta agent's session
/// and the orchestrator seat never open it, and then the setting is not read
/// at all: `read` runs only when neither suppresses it. A malformed setting
/// opens the chat, as when it is unset.
fn chat_opens(
    meta_agent: bool,
    orchestrator_seat: bool,
    read: impl FnOnce() -> config::ChatWindow,
) -> (bool, Option<String>) {
    if meta_agent || orchestrator_seat {
        return (false, None);
    }
    match read() {
        config::ChatWindow::On | config::ChatWindow::App => (true, None),
        config::ChatWindow::Off => (false, None),
        config::ChatWindow::Unusable(why) => (true, Some(why)),
    }
}

/// A session's layout. META-WINS on resume: a non-empty recorded layout was
/// pinned at launch, so a config flip applies to NEW sessions only, mirroring
/// agent identity. Otherwise the configured one, and `vertical` when none is.
fn session_layout(configured: String, saved: Option<String>) -> String {
    saved.filter(|value| !value.is_empty()).unwrap_or_else(|| {
        if configured.is_empty() {
            "vertical".to_owned()
        } else {
            configured
        }
    })
}

/// Why a crash restore skips once the lifecycle lock is decided. Without the
/// lock (a restore takes it only over a record it saw) or without the record,
/// the record was gone before the lock: that reason, and the guard is never
/// asked. Otherwise the guard's own verdict, read afresh under the lock.
fn restore_skip(
    held: bool,
    meta_present: bool,
    guard: impl FnOnce() -> Option<&'static str>,
) -> Option<&'static str> {
    if !held || !meta_present {
        return Some("saved record gone before the lifecycle lock");
    }
    guard()
}

/// `?` for a launch step: a step that refused has already written its own
/// diagnostic, so the launch ends with the exit code it returned.
macro_rules! step {
    ($step:expr) => {
        match $step? {
            ControlFlow::Continue(value) => value,
            ControlFlow::Break(code) => return Ok(code),
        }
    };
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
    /// Whether the chat opens as the session's first window — `[workspace]
    /// chat`, never for the orchestrator's own session.
    chat: bool,
    resuming: bool,
    /// Whether THIS attempt created the session directory — the ownership fact
    /// the rollback keys on, recorded at the `mkdir` and never derived from
    /// `resuming`.
    dir_created: bool,
}

#[allow(
    clippy::too_many_lines,
    reason = "the frozen order, kept in one place: every step is called here in order, and the \
              lifecycle lock, the expected-launch proofs and the reattach stay inline where the \
              source-order unit pins read them"
)]
fn launch(
    env: &Env,
    plan: &Plan,
    expected_proof: Option<&str>,
    expected_launch: Option<&ExpectedLaunch>,
    restore: Option<&crate::restore::Attempt>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    let sessions = env.sessions();
    let worktrees = env.worktrees();

    // ---- the name, and the guards that must precede every side effect ----
    let session = step!(check_name(env, plan, expected_launch, err));
    let dir = sessions.join(&session);
    let work_root = worktrees.join(&session);

    let proposed_server = env.server();
    let expected_resume = expected_launch.is_some_and(ExpectedLaunch::is_resume);
    let placing = Placing {
        proposed: &proposed_server,
        in_place: expected_resume || restore.is_some(),
        restore,
    };
    let settings_start = expected_launch.is_some_and(ExpectedLaunch::is_start);
    let preflight_meta_present = step!(check_launch_kind(
        &session,
        &dir,
        expected_launch,
        restore,
        plan.resume,
        expected_resume,
        settings_start,
        err
    ));

    // ---- THE tmux FLOOR, before the first thing this launch would write ----
    // AHEAD of the lifecycle lock, migration chain and config seed. A resume
    // first takes a read-only look at its recorded server so the floor is asked
    // of the server this attempt will actually use; the decision is repeated
    // under the lifecycle lock below before anything acts on it.
    let floor_server = step!(preflight_server(
        &dir,
        &session,
        placing,
        preflight_meta_present,
        err
    ));
    let running_preflight =
        preflight_meta_present && transport::session_exists(&floor_server, &session);
    if let Some(refusal) = floor_refusal(&floor_server, &session) {
        write!(err, "{refusal}")?;
        return Ok(crate::tmux_floor::EXIT_REFUSED);
    }
    step!(check_stopped_near_miss(
        &session,
        plan.resume || carries_own_proof(expected_launch, restore),
        preflight_meta_present,
        running_preflight,
        err
    ));
    // A labeled spawned seat's recorded authority is judged first: a missing
    // global config is that seat's own refusal, not the roster's.
    let spawned_labels = step!(check_spawned_labels(
        plan,
        &dir,
        preflight_meta_present,
        running_preflight,
        err
    ));
    let seat_overrides = step!(check_seat_overrides(
        env,
        plan,
        &dir,
        preflight_meta_present,
        running_preflight,
        err
    ));

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
        || expected_resume
        || preflight_meta_present
        || crate::lifecycle::aliases::target(&dir).is_some()
    {
        let Ok(held) = crate::store::lock(
            &sessions.join(format!(".lifecycle.{session}.lock")),
            LIFECYCLE_WAIT,
        ) else {
            return Ok(lifecycle_busy(&session, settings_start, err)?);
        };
        Some(held)
    } else {
        None
    };
    if lifecycle.is_some() {
        step!(reclaim_launch_alias(&dir, err));
    }
    let meta_present = if expected_launch.is_some() {
        node_exists(&dir.join(crate::store::META))
    } else {
        preflight_meta_present
    };
    if let Some(attempt) = restore
        && let Some(why) = restore_skip(lifecycle.is_some(), meta_present, || {
            attempt.guard.refuse(&dir)
        })
    {
        attempt.skip(why);
        return Ok(0);
    }

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
    let running_server = step!(resolve_server(
        &mut env,
        plan,
        &dir,
        &session,
        placing,
        meta_present,
        err
    ));

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
    let orchestrator_seat = step!(adopt_recorded_config(
        &mut env,
        &dir,
        &session,
        meta_present,
        seat_overrides.is_some(),
        err
    ));
    // ---- explicit lineage, proved before anything is created ----
    let mut parent: Option<FromProof> = None;
    if let Some(uuid) = &plan.from {
        parent = Some(step!(prove_lineage(
            &env.home.join("archive"),
            uuid,
            expected_proof,
            || {
                transport::session_exists(&proposed_server, &session)
                    || dir_exists(&dir)
                    || dir_exists(&work_root)
            },
            &session,
            err
        )));
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
        restamp_running(&server, &env, &dir, &session, &observed);
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

    if seeds_orchestrator_config {
        step!(seed_orchestrator_seat(&env, &orchestrator_config, err));
    }

    // ---- config, roster, workspace values ----
    let mut cfg = match launch_identity(&env, seat_overrides.as_ref(), false) {
        Ok(cfg) => cfg,
        Err(why) => {
            writeln!(err, "{why}")?;
            return Ok(EXIT_FAILED);
        }
    };
    let values = read_workspace_values(&env);
    let cadences = step!(resolve_watchdog_cadences(
        &env,
        meta_present.then_some(dir.as_path()),
        &values.quota,
        err
    ));

    let home = crate::doors::home();
    let mut seats = step!(resolve_seats(&mut cfg, plan, home.as_deref(), err));

    // ---- the mode: flag, then the recorded one, then config, then local ----
    let recorded_mode = if plan.mode.is_none() && meta_present {
        meta_value(&dir, "mode").as_deref().and_then(Mode::parse)
    } else {
        None
    };
    let mode = launch_mode(plan.mode, recorded_mode, &values.copy);

    // ---- the working copy ----
    // The session document is the ONE resume discriminator. A worktree path is
    // not identity: rename deliberately leaves it under the old name, and an
    // unrelated directory under the new name must never turn a fresh launch
    // into a resume.
    let resuming = meta_present;
    if resuming {
        step!(check_resumed_roster(&dir, &session, &cfg, err));
    }
    let copy = step!(prepare_working_copy(
        &env.cwd,
        &work_root,
        &session,
        mode,
        resuming.then_some(dir.as_path()),
        out,
        err
    ));

    step!(restore_roster(
        resuming.then_some(dir.as_path()),
        &session,
        &cfg,
        home.as_deref(),
        seat_overrides.as_ref(),
        &mut seats,
        err
    ));
    // Read only on resume, and only once the roster has been restored.
    let saved_layout = resuming.then(|| meta_value(&dir, "layout")).flatten();
    let layout = session_layout(values.layout, saved_layout);

    // ---- restored spawned seats, LEXED before anything is created ----
    if resuming {
        step!(lex_spawned_seats(
            &dir,
            &session,
            &cfg,
            &spawned_labels,
            home.as_deref(),
            err
        ));
    }

    let (chat, unusable) = chat_opens(values.meta_agent, orchestrator_seat, || {
        config::workspace_chat_window(env.global.as_deref(), env.local.as_deref())
    });
    if let Some(why) = unusable {
        writeln!(
            err,
            "Note: [workspace] chat: {why}; the chat window opens, as when it is unset."
        )?;
    }
    let mut shape = Session {
        name: session.clone(),
        mode: copy.mode,
        work_dir: copy.work_dir,
        origin: copy.origin,
        layout,
        look: values.look,
        chat,
        resuming,
        dir_created: false,
    };

    build(
        &env,
        &mut shape,
        &seats,
        &cfg,
        (seat_overrides.as_ref(), &spawned_labels),
        WatchdogFacts {
            meta_agent: values.meta_agent,
            sweep_sec: values.sweep_sec.as_deref(),
            quota_every_secs: &cadences.quota_every_secs,
            idle_nudge_secs: &cadences.idle_nudge_secs,
            done_confirmations: &cadences.done_confirmations,
            quota: &cadences.quota,
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
    done_confirmations: &'a str,
    quota: &'a str,
}

/// The conversation id a launch carries for a seat: the stored one, or the
/// honest unknown.
///
/// A REBUILD never MINTS for a flag tool: a seat whose row is absent or
/// `pending` has no conversation to name, and a minted UUID would name one
/// that does not exist — the seat takes the fresh-start fallback instead, and
/// `_run` mints the new conversation's id at its own pre-exec. A
/// CREATE still mints: there the id IS the conversation about to open.
fn launch_session_id(stored: Option<String>, resuming: bool, tool: ToolKind) -> String {
    match stored {
        Some(id) => id,
        None if resuming => PENDING.to_owned(),
        None if launch::takes_launch_session_id(tool) => launch::generate_uuid(),
        None => PENDING.to_owned(),
    }
}

/// The stored conversation id a resume reuses for `slot`: an absent, empty
/// or `pending` row is no conversation at all, never a name.
fn stored_session_id(dir: &Path, slot: &str, resuming: bool) -> Option<String> {
    resuming
        .then(|| meta_value(dir, &format!("harness_session.{slot}")))
        .flatten()
        .filter(|id| !id.is_empty() && id != PENDING)
}

#[allow(
    clippy::too_many_arguments,
    reason = "the frozen order, kept in one place — see the module docs"
)]
fn build(
    env: &Env,
    shape: &mut Session,
    seats: &[Seat],
    cfg: &IdentityConfig,
    (seat_overrides, spawned_labels): (Option<&SeatOverrideSnapshot>, &[JudgedSpawned]),
    watchdog: WatchdogFacts<'_>,
    parent: Option<&FromProof>,
    lifecycle: Option<std::fs::File>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    let server = env.server();
    let home = crate::doors::home();
    let sessions = env.sessions();
    let dir = sessions.join(&shape.name);
    let work_dir = shape.work_dir.display().to_string();

    let (lifecycle, core) = step!(claim_state(shape, &sessions, &dir, lifecycle, err));
    let at = Building {
        server: &server,
        dir: &dir,
        work_dir: &work_dir,
        core: &core,
    };

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

    // From here on every refusal rolls the whole launch back: the tmux
    // session is killed, and the state directory removed if this attempt
    // created it.
    let panes = match open_panes(at, shape, seats, &main_pane) {
        Ok(panes) => panes,
        Err(line) => return rollback_launch(shape, &dir, &server, &line, err),
    };
    let mut launching = match seat_roster(at, shape, seats, &panes, seat_overrides) {
        Ok(launching) => launching,
        Err(line) => return rollback_launch(shape, &dir, &server, &line, err),
    };
    if shape.resuming
        && let Err(line) = restore_spawned(
            at,
            shape,
            cfg,
            home.as_deref(),
            (seat_overrides, spawned_labels),
            &mut launching,
        )
    {
        return rollback_launch(shape, &dir, &server, &line, err);
    }
    if let Err(line) = publish_meta(
        at,
        env,
        shape,
        &launching,
        watchdog,
        parent,
        &live_session_id,
    ) {
        return rollback_launch(shape, &dir, &server, &line, err);
    }
    if let Err(line) = write_assets(at, env, shape, &main_pane) {
        return rollback_launch(shape, &dir, &server, &line, err);
    }

    // Resume-only event retention, before the events pane starts tailing: the
    // log is append-only, so a long-lived session accumulates every resume.
    if shape.resuming {
        trim_events(&dir);
    }

    // Freshly created panes can still be inside shell init when tmux returns.
    std::thread::sleep(SHELL_SETTLE);

    let pending = match exec_seats(at, shape, &launching, err)? {
        Ok(pending) => pending,
        Err(line) => return rollback_launch(shape, &dir, &server, &line, err),
    };

    // The session is fully on disk; `ae end` may safely delete it, and the
    // capture children below are spawned detached past the hold.
    drop(lifecycle);

    // Rollback is unreachable past this point.
    let chat = open_views(at, env, shape, &main_pane, err)?;
    watch_trust(at, shape, &launching, &pending, err);
    deliver_and_settle(at, shape, pending, err)?;

    // ---- post-launch capture ----
    // AFTER the deliveries, as before: the capture children poll the tools'
    // stores for a bounded 30 s, so the rollout the turn creates must already
    // be on disk when they start looking.
    capture::start(&dir, &launching_capture(&launching, shape.resuming));

    // ---- the Telegram bridge ----
    //
    // LAST, and after the session exists, deliberately: the bridge is
    // best-effort and strictly non-fatal. A session that is up is never failed
    // by a bridge that is not.
    if !env.no_autostart {
        autostart_telegram(env, shape, &dir, &server, err);
    }

    let _ = transport::run_tmux_op(&argv(&server, &Op::SelectPane { pane: &main_pane }));
    if let Some(chat) = &chat {
        let _ = transport::run_tmux_op(&argv(&server, &Op::SelectWindow { pane: chat }));
    }
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

/// The facts every step of one launch's build addresses: the server, the
/// session's state directory, its working directory and the resolved core.
#[derive(Clone, Copy)]
struct Building<'a> {
    server: &'a ServerId,
    dir: &'a Path,
    work_dir: &'a str,
    core: &'a Path,
}

/// Reclaim after the floor and under the name lock. Recheck at the final
/// directory claim too, so a fresh-name launch never writes through a link.
fn reclaim_launch_alias(dir: &Path, err: &mut impl Write) -> crate::error::Result<ControlFlow<u8>> {
    if let Err(why) = crate::lifecycle::aliases::reclaim(dir) {
        writeln!(
            err,
            "Error: could not reclaim the session alias '{}' ({why}).",
            dir.display()
        )?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    if matches!(
        crate::teardown::classify_dir(dir),
        crate::teardown::DirKind::Symlink
    ) {
        writeln!(
            err,
            "Error: session path '{}' is a symlink; refusing to launch through it.",
            dir.display()
        )?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    Ok(ControlFlow::Continue(()))
}

/// The launch's claim on its state, before any tmux write: the sessions root,
/// the lifecycle lock, the core's own path, the session directory and the
/// launch-attempt stamp, refusing at the first that fails. Answers the held
/// lock and the core.
fn claim_state(
    shape: &mut Session,
    sessions: &Path,
    dir: &Path,
    lifecycle: Option<std::fs::File>,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, (std::fs::File, PathBuf)>> {
    if let Err(why) = std::fs::create_dir_all(sessions) {
        writeln!(
            err,
            "Error: could not create {} ({why})",
            sessions.display()
        )?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
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
            return Ok(ControlFlow::Break(EXIT_FAILED));
        };
        held
    };

    if let ControlFlow::Break(code) = reclaim_launch_alias(dir, err)? {
        return Ok(ControlFlow::Break(code));
    }

    let Some(core) = crate::shape::resolved_exe() else {
        writeln!(err, "Error: the core could not name its own binary.")?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    };

    // Ownership as an explicit FACT, decided BEFORE the stamp below needs the
    // directory to exist — a state dir this attempt found is never a state dir
    // this attempt may remove.
    shape.dir_created = !node_exists(dir);
    if let Err(why) = create_private_dir(dir) {
        writeln!(err, "Error: could not create {} ({why})", dir.display())?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
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
        crate::store::open(dir).stamp_launch_attempt(crate::time::Timestamp::now().epoch())
    {
        writeln!(
            err,
            "Error: could not record the launch attempt for '{}' ({why}). No tmux session was created.",
            shape.name
        )?;
        rollback_dir(shape, dir, err)?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    Ok(ControlFlow::Continue((lifecycle, core)))
}

/// Every seat's pane: the main pane, then one per worker by layout, the
/// layout applied, and each pane stamped with its seat. Answers the panes in
/// seat order, or the line a failed worker pane refuses with.
fn open_panes(
    at: Building<'_>,
    shape: &Session,
    seats: &[Seat],
    main_pane: &str,
) -> Result<Vec<String>, String> {
    let mut panes = vec![main_pane.to_owned()];
    let workers = seats.len().saturating_sub(1);
    let window = format!("{}:", tmux::session_target(&shape.name));
    for (index, seat) in seats.iter().skip(1).enumerate() {
        let created = match worker_placement(&shape.layout, index) {
            WorkerPane::NewWindow => new_window(at.server, &window, &seat.name, at.work_dir),
            WorkerPane::Split { of, dir } => {
                let target = match of {
                    PaneRef::First => &panes[0],
                    PaneRef::Last => &panes[panes.len() - 1],
                    PaneRef::Window => &window,
                };
                split(at.server, target, at.work_dir, dir)
            }
        };
        let Some(pane) = created else {
            return Err(format!("Error: could not create a pane for worker {index}"));
        };
        panes.push(pane);
    }
    apply_layout(at.server, shape, &panes, workers);

    // Pane stamps.
    for (index, seat) in seats.iter().enumerate() {
        stamp_pane(
            at.server,
            &panes[index],
            &seat.name,
            &seat.slot,
            &seat.profile,
        );
    }
    Ok(panes)
}

/// The roster, and the ids each seat launches with: the recorded client rows,
/// re-read under the lifecycle lock on a resume, decide each seat's client;
/// a resume reuses each seat's recorded ids and homes. Answers one
/// [`Launching`] per seat, or the line the first refusal prints.
fn seat_roster(
    at: Building<'_>,
    shape: &Session,
    seats: &[Seat],
    panes: &[String],
    seat_overrides: Option<&SeatOverrideSnapshot>,
) -> Result<Vec<Launching>, String> {
    let recorded_meta: Option<Meta> = if shape.resuming {
        // A read failure refuses, never launders toward None: a later
        // successful raw rebuild would drop the client rows and switch the
        // seats' accounts.
        let bytes = meta::read_bytes(at.dir)
            .map_err(|why| format!("cannot read the meta to carry recorded clients: {why}"))?;
        Some(Meta::parse(&String::from_utf8_lossy(&bytes)))
    } else {
        None
    };
    let mut launching: Vec<Launching> = Vec::new();
    for (index, seat) in seats.iter().enumerate() {
        let client = seat_client(&shape.name, seat, recorded_meta.as_ref())?;
        let stored = stored_session_id(at.dir, &seat.slot, shape.resuming);
        let session_id = launch_session_id(stored, shape.resuming, seat.tool);
        let launch_id = launch_token(
            seat.tool,
            shape
                .resuming
                .then(|| meta_value(at.dir, &format!("launch_id.{}", seat.slot)))
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
                .then(|| meta_value(at.dir, &format!("config_home.{}", seat.slot)))
                .flatten()
                .filter(|value| !value.is_empty()),
            config_home_base: shape
                .resuming
                .then(|| meta_value(at.dir, &format!("config_home_base.{}", seat.slot)))
                .flatten()
                .filter(|value| !value.is_empty()),
            launch_id,
            pane: panes[index].clone(),
            command_snapshot: seat_overrides.map(|_| seat.command.clone()),
        });
    }
    Ok(launching)
}

/// A resume's spawned agents, restored in slot order after the seats, each in
/// a window of its own; the split layouts are then rebalanced. Answers the
/// line the first refusal prints.
fn restore_spawned(
    at: Building<'_>,
    shape: &Session,
    cfg: &IdentityConfig,
    home: Option<&Path>,
    (seat_overrides, judged): (Option<&SeatOverrideSnapshot>, &[JudgedSpawned]),
    launching: &mut Vec<Launching>,
) -> Result<(), String> {
    let spawned = spawned_entries(at.dir, &shape.name)?;
    for entry in spawned {
        // A profile that is not configured on THIS machine keeps its seat
        // VERBATIM at its original index — a later resume with the profile
        // configured restores the worker, and preserving the index keeps
        // the slot key stable for any in-flight request addressed to it.
        let command = restored_command(cfg, &entry, judged, &shape.name, home)?
            .unwrap_or_else(|| IdentityConfig::resolved_snapshot(""));
        let pane = if command.as_str().is_empty() {
            String::new()
        } else {
            match new_window(
                at.server,
                &format!("{}:", tmux::session_target(&shape.name)),
                &entry.name,
                at.work_dir,
            ) {
                Some(pane) => {
                    stamp_pane(at.server, &pane, &entry.name, &entry.slot, &entry.profile);
                    pane
                }
                None => String::new(),
            }
        };
        let tool = ToolKind::from_cmd(command.as_str());
        let launch_id = launch_token(
            tool,
            meta_value(at.dir, &format!("launch_id.{}", entry.slot)),
        );
        // A labeled seat runs exactly the command preflight judged: `_run`
        // never resolves its pair a second time.
        let command_snapshot =
            (seat_overrides.is_some() || entry.client.is_some()) && !command.as_str().is_empty();
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
            command_snapshot: command_snapshot.then_some(command),
        });
    }
    if let Some(layout) = respawn_layout(&shape.layout) {
        let target = format!("{}:", tmux::session_target(&shape.name));
        let _ = transport::run_tmux_op(&argv(
            at.server,
            &Op::SelectLayout {
                target: &target,
                layout,
            },
        ));
    }
    Ok(())
}

/// The meta, published as ONE document. Answers the line a refusal prints.
fn publish_meta(
    at: Building<'_>,
    env: &Env,
    shape: &Session,
    launching: &[Launching],
    watchdog: WatchdogFacts<'_>,
    parent: Option<&FromProof>,
    live_session_id: &ProvenIdentity,
) -> Result<(), String> {
    let document = meta_document(env, shape, launching, watchdog, parent)
        .map_err(|why| format!("Error: {why}"))?;
    let published = publish_meta_and_seed_uuid(
        at.server,
        Some(live_session_id),
        at.dir,
        shape.resuming,
        &document,
    );
    if published.is_err() {
        return Err("Error: the session meta could not be published.".to_owned());
    }
    Ok(())
}

/// The session's assets: the helper links and the workspace manifest.
/// Answers the line a refusal prints.
fn write_assets(
    at: Building<'_>,
    env: &Env,
    shape: &Session,
    main_pane: &str,
) -> Result<(), String> {
    // RESOLVED, not as invoked: the helper links published below must name the
    // immutable core, not `~/.local/bin/ae`. macOS answers `current_exe()` with
    // the path this process was exec'd BY, so on an installed machine the raw
    // answer is the command symlink — and a session whose helpers pointed at it
    // would follow the next `ae upgrade` to a core it was never built against.
    assets::write_helpers(at.dir, at.core).map_err(|why| format!("Error: {why}"))?;
    let manifest = crate::render::manifest_document(
        at.dir,
        &shape.name,
        at.work_dir,
        &shape.origin.display().to_string(),
        shape.mode.as_str(),
        main_pane,
        &env.config_files(),
    );
    assets::publish_document(&at.dir.join("workspace.md"), &manifest)
        .map_err(|why| format!("Error: could not write the workspace manifest ({why})."))
}

/// Launch scripts, and the paste into each pane's shell: phase 1 of the start,
/// the EXEC half only. Answers each seat's carried launch turn, in roster
/// order, or the line a refusal prints.
///
/// Each seat's tool is started and its launch turn, if any, is CARRIED, never
/// delivered. The deliveries wait below the monitors, because a gated prompt
/// delivery blocks up to 45 s per seat on the tool's input readiness and the
/// monitor panes must not wait on any seat. The gate is evaluated here, at
/// exec time: `_run` publishes the start marker pre-exec on Create, so a
/// later re-read of `resuming_seat` would wrongly paste a fresh seat.
fn exec_seats<'l>(
    at: Building<'_>,
    shape: &Session,
    launching: &'l [Launching],
    err: &mut impl Write,
) -> io::Result<Result<Vec<(&'l Launching, String)>, String>> {
    let mut pending: Vec<(&Launching, String)> = Vec::with_capacity(launching.len());
    for agent in launching {
        // A seat with no pane is a preserved roster row, not an agent to start.
        if agent.pane.is_empty() {
            continue;
        }
        match start_agent(shape, at.dir, at.core, agent, at.server, err)? {
            Err(why) => return Ok(Err(format!("Error: {why}"))),
            Ok(Some(prompt)) => pending.push((agent, prompt)),
            Ok(None) => {}
        }
    }
    Ok(Ok(pending))
}

/// The session's views, opened past the lock and before the phase-2
/// deliveries: the monitor panes, the chat as the first window, and the
/// per-window half of the look. Answers the chat's pane, if one opened.
fn open_views(
    at: Building<'_>,
    env: &Env,
    shape: &Session,
    main_pane: &str,
    err: &mut impl Write,
) -> io::Result<Option<String>> {
    // ---- the monitor panes ----
    // BEFORE the phase-2 deliveries: every fleet-line value is
    // watchdog-published, so a launch blocked in a readiness wait must still
    // show a live status line. Rollback is unreachable past phase 1, so these
    // panes cannot orphan.
    let events_pane = ensure_events_pane(at.server, &shape.name, at.dir);
    if let Some(anchor) = &events_pane {
        start_watchdog_pane(shape, at.dir, at.server, anchor, env.no_autostart);
    }

    // ---- the chat, as the session's first window ----
    // A VIEW, not a seat: opened once every seat and monitor window exists,
    // ahead of the look sweep and the phase-2 waits. One that cannot be
    // opened leaves the launch standing, named on stderr and in the journal.
    let chat = if shape.chat {
        let config = env
            .global
            .clone()
            .unwrap_or_else(|| env.home.join("config"));
        let home = crate::console::toggle::Home::of(&env.home, &config, Some(at.dir));
        open_first_chat(at.server, &shape.name, at.dir, home, err)?
    } else {
        None
    };
    if chat.is_some() {
        stamp_client_session_hook(at.server, &shape.name, main_pane, true);
    }

    // ---- the per-window half of the look ----
    // Every window a layout, a seat or a monitor created is stamped; the
    // phase-2 deliveries create no window, so stamping ahead of them keeps
    // the contract and the look off the readiness wait. The watchdog
    // restamps whatever appears after this.
    stamp_windows(at.server, &shape.name, &shape.look);
    Ok(chat)
}

/// The launch-wide budget a FRESH launch spends watching its turnless seats
/// for a measured folder-trust block. Seats are watched in turn under ONE
/// deadline: no attempt STARTS past it, and an attempt already started is never
/// cut short, so the attach waits at most this plus one answer sequence.
const TRUST_WATCH: std::time::Duration = std::time::Duration::from_secs(15);

/// `auto_trust`'s answer for a FRESH launch's seats that get no launch turn:
/// a turnless seat has no wait of its own, and the watchdog would reach it a
/// cycle later at best. Each seat whose tool measured a trust block is read
/// until it shows its composer, its block is answered or declined through
/// [`crate::autotrust::in_wait`], another human-only prompt holds, or
/// [`TRUST_WATCH`] ends. A resumed launch is not watched: its seats already
/// ran in their folders, and a restore must not pay every seat's boot; a
/// resumed seat's block is the watchdog's, from its first sight.
fn watch_trust(
    at: Building<'_>,
    shape: &Session,
    launching: &[Launching],
    pending: &[(&Launching, String)],
    err: &mut impl Write,
) {
    let answering = crate::autotrust::on(at.dir);
    let deadline = trust_deadline(std::time::Instant::now());
    for agent in launching.iter().filter(|agent| {
        watches(
            (shape.resuming, answering),
            &agent.pane,
            agent.tool,
            pending.iter().any(|(turn, _)| turn.slot == agent.slot),
        )
    }) {
        let input = agent.tool.adapter().input;
        watched(
            || {
                if deliver::wait_input_ready(
                    at.server,
                    &agent.pane,
                    input.model,
                    input.composed,
                    chunk_polls(input.model),
                ) {
                    return WatchRead::Ready;
                }
                let prompt =
                    transport::capture_screen(at.server, &agent.pane, tmux::Styling::Plain)
                        .and_then(|frame| {
                            crate::watchdog::human_prompt_class(&frame, agent.tool.adapter().name)
                        });
                classify(prompt, || {
                    crate::autotrust::in_wait(at.dir, at.server, &agent.slot, &agent.pane, err)
                })
            },
            || before(std::time::Instant::now(), deadline),
        );
    }
}

/// Whether a launch watches one seat: a FRESH launch with `auto_trust` on
/// (`launch`: resuming, answering), a seat with a pane, whose tool measured a
/// trust block, and that gets no launch turn of its own.
fn watches(launch: (bool, bool), pane: &str, tool: ToolKind, turn: bool) -> bool {
    let (resuming, answering) = launch;
    !resuming
        && answering
        && !pane.is_empty()
        && tool
            .adapter()
            .prompt
            .is_some_and(|spec| spec.trust.is_some())
        && !turn
}

/// The one deadline a launch's whole watch runs under.
fn trust_deadline(now: std::time::Instant) -> std::time::Instant {
    now + TRUST_WATCH
}

/// Whether `now` is still before `deadline`: another read may START.
fn before(now: std::time::Instant, deadline: std::time::Instant) -> bool {
    now < deadline
}

/// What a non-ready read shows: a measured trust block, handed to `answer`
/// (the in-wait answerer, asked only then); another prompt by its question;
/// or nothing yet.
fn classify(
    prompt: Option<crate::watchdog::HumanPrompt>,
    answer: impl FnOnce() -> crate::autotrust::InWait,
) -> WatchRead {
    match prompt {
        Some(asked) if asked.trust.is_some() => WatchRead::Trust(answer()),
        Some(asked) => WatchRead::Other(asked.question),
        None => WatchRead::Nothing,
    }
}

/// One read of a watched seat.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WatchRead {
    /// The composer is drawn: nothing to answer.
    Ready,
    /// A measured trust block was drawn and handed to the in-wait answer.
    Trust(crate::autotrust::InWait),
    /// Another human-only prompt, by its question: the watchdog names it.
    Other(String),
    /// Neither yet: the tool is still drawing.
    Nothing,
}

/// Where a watch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Watched {
    Ready,
    Trusted,
    Stopped,
    Other,
    Deadline,
}

/// Fold a seat's reads, in order, while `open` allows another read — pure, so
/// the budget and every ending are pinnable without a pane. `open` is asked
/// BEFORE each read and never during one: a started answer runs to its end.
/// The same other prompt on two consecutive reads ends the watch.
fn watched(mut read: impl FnMut() -> WatchRead, mut open: impl FnMut() -> bool) -> Watched {
    let mut previous: Option<String> = None;
    while open() {
        match read() {
            WatchRead::Ready => return Watched::Ready,
            WatchRead::Trust(crate::autotrust::InWait::Trusted) => return Watched::Trusted,
            WatchRead::Trust(crate::autotrust::InWait::Stop) => return Watched::Stopped,
            WatchRead::Trust(crate::autotrust::InWait::Again) | WatchRead::Nothing => {
                previous = None;
            }
            WatchRead::Other(question) => {
                if previous.as_ref() == Some(&question) {
                    return Watched::Other;
                }
                previous = Some(question);
            }
        }
    }
    Watched::Deadline
}

/// Phase 2: the gated launch-prompt deliveries, then a resume's settled mark.
fn deliver_and_settle(
    at: Building<'_>,
    shape: &Session,
    pending: Vec<(&Launching, String)>,
    err: &mut impl Write,
) -> io::Result<()> {
    // Past the monitors and past the lock: a vanished directory or pane takes
    // the durable failure inside `deliver_launch_turn`, whose errors are
    // swallowed, so no state is recreated and nothing panics. A concurrent
    // launch against this live session reattaches, never rebuilds.
    for (agent, prompt) in pending {
        // The outcome is DELIBERATELY dropped here: a launch answers for the
        // whole roster and each turn already failed durably on its own. Only a
        // single-slot caller maps it onto an exit code.
        let _ = deliver_launch_turn(
            at.dir,
            at.server,
            &agent.slot,
            &agent.pane,
            agent.tool,
            &prompt,
            OnHumanPrompt::Wait,
            err,
        )?;
    }

    // ---- the resume has settled ----
    // Every launch turn has ended, however it ended. The rebuild above wrote
    // no such row, so a crash before this line leaves none: without it the
    // watchdog treats the seats as it always has.
    if shape.resuming {
        let settled = crate::time::Timestamp::now().epoch().to_string();
        if crate::meta::rewrite(at.dir, crate::meta::LAUNCH_SETTLED_KEY, Some(&settled)).is_err() {
            writeln!(err, "ae: the resume's settled mark was not written.")?;
        }
    }
    Ok(())
}

/// Which existing pane or window a worker split divides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaneRef {
    /// The session's first pane.
    First,
    /// The pane created last.
    Last,
    /// The session's current window.
    Window,
}

/// Where a launch puts one worker seat's pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerPane {
    /// A split of an existing pane or window.
    Split { of: PaneRef, dir: Split },
    /// A window of its own, named after the seat.
    NewWindow,
}

/// Where worker `index` (0 = the first seat after main) goes in `layout`.
///
/// `lead-pair` puts the colead beside the lead and the rest in a second
/// window; `lead-solo` puts every worker in that second window; `vertical`
/// and every other layout split the session's window.
fn worker_placement(layout: &str, index: usize) -> WorkerPane {
    match (layout, index) {
        ("lead-pair", 0) => WorkerPane::Split {
            of: PaneRef::First,
            dir: Split::Horizontal,
        },
        ("lead-pair", 1) | ("lead-solo", 0) => WorkerPane::NewWindow,
        ("lead-pair" | "lead-solo", _) => WorkerPane::Split {
            of: PaneRef::Last,
            dir: Split::Vertical,
        },
        ("vertical", _) => WorkerPane::Split {
            of: PaneRef::Window,
            dir: Split::Horizontal,
        },
        _ => WorkerPane::Split {
            of: PaneRef::Window,
            dir: Split::Vertical,
        },
    }
}

/// The client row `seat` launches with, or the line its refusal prints.
///
/// The recorded client rows were re-read under the lifecycle lock: preflight
/// (or the post-lock gate for a launch without overrides) already refused
/// every unusable row, so an Invalid met here is a mid-flight hand edit — and
/// it still refuses rather than laundering toward Missing.
fn seat_client(
    session: &str,
    seat: &Seat,
    recorded: Option<&Meta>,
) -> Result<Option<String>, String> {
    let carried: Option<String> = match recorded
        .and_then(|meta| meta.roster().iter().find(|entry| entry.slot == seat.slot))
    {
        None => None,
        Some(entry) => match &entry.client {
            crate::meta::RecordedClient::Missing => None,
            crate::meta::RecordedClient::Label(label) => Some(label.clone()),
            crate::meta::RecordedClient::Invalid => {
                return Err(invalid_client_refusal(session, &seat.name, &seat.slot));
            }
        },
    };
    if let (Some(selected), Some(known)) = (seat.client_override.as_deref(), carried.as_deref())
        && selected != known
    {
        return Err(different_label_refusal(
            session,
            &seat.name,
            &seat.profile,
            known,
            selected,
        ));
    }
    if seat.client_override.is_none()
        && let Some(added) = carried.as_deref()
    {
        // A label preflight never saw: hand-added (or a concurrent
        // launch's) between preflight and this lock. It carries no R1/R4
        // proof, and this seat was composed without it — retrying
        // re-runs preflight, which honors it properly.
        return Err(format!(
            "Error: session '{session}' seat '{}' recorded client override '{added}' while this launch was starting — \
             nothing was resumed; retry the launch.",
            seat.name
        ));
    }
    // An unproven pairing honors the pane but mints no label fact: only a
    // carried row — an earlier PROVEN start's fact, republished unchanged —
    // may survive it.
    Ok(if seat.store_proven {
        seat.client_override.clone().or(carried)
    } else {
        carried
    })
}

/// The layout a resume re-applies once its spawned agents are back, if any.
///
/// Only the SPLIT layouts rebalance: a spawned agent gets its own window, so
/// window 0 gains no panes, and even-vertical would stack lead-pair's
/// side-by-side leads.
fn respawn_layout(layout: &str) -> Option<&'static str> {
    match layout {
        "lead-solo" | "lead-pair" => None,
        "vertical" => Some("even-horizontal"),
        _ => Some("even-vertical"),
    }
}

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
            before: false,
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

/// The watchdog pane, split ABOVE the events pane so the visual order stays
/// watchdog-on-top / events-below.
///
/// `no_autostart` is the operator's `AE_NO_AUTOSTART=1`: it suppresses the
/// companion for THIS launch alone, because the variable is never persisted. A
/// daemon already in the pane keeps its verdicts — the off mark is for a
/// session nothing measures, and claiming a measured one is off would be a
/// false observable.
fn start_watchdog_pane(
    shape: &Session,
    dir: &Path,
    server: &ServerId,
    anchor: &str,
    no_autostart: bool,
) {
    if no_autostart {
        if monitor_pane(server, &shape.name, "_watchdog").is_none() {
            let _ = transport::publish_option(
                server,
                tmux::OptionScope::Session,
                &shape.name,
                tmux::WATCHDOG_STATUS_OPTION,
                &crate::theme::watchdog_off_segment(&shape.look),
            );
        }
        return;
    }
    if !watchdog_enabled_for_session(dir) {
        // Nothing will measure this session, so the cell a health segment would
        // fill says that instead of staying blank — a blank one reads like a
        // bar still filling in. The attention seed is already on the session
        // from the look, so the row is there; only the reason was missing.
        let _ = transport::publish_option(
            server,
            tmux::OptionScope::Session,
            &shape.name,
            tmux::WATCHDOG_STATUS_OPTION,
            &crate::theme::watchdog_off_segment(&shape.look),
        );
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
            before: false,
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
        let mut sockets = crate::fleet_read::SocketPaths::live();
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

/// The recorded origin an explicit `--dir` would contradict, with the
/// directory it named: `Some` only when `dir` holds a session record whose
/// `origin` is non-empty and `requested` is an existing, different directory.
/// Every other case is no verdict, and the launch answers it as it always has.
pub(crate) fn recorded_origin_conflict(dir: &Path, requested: &str) -> Option<(String, PathBuf)> {
    let requested = canonical_directory(Path::new(requested))?;
    let recorded = meta_value(dir, "origin").filter(|value| !value.is_empty())?;
    (!same_directory(&recorded, &requested.display().to_string())).then_some((recorded, requested))
}

/// One existing directory, canonicalised.
#[allow(
    clippy::disallowed_methods,
    reason = "a door: --dir and explicit-origin resume checks resolve canonical directories — see clippy.toml"
)]
pub(crate) fn canonical_directory(path: &Path) -> Option<PathBuf> {
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
pub(crate) fn resume_absence(
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
) -> crate::error::Result<u8> {
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

/// Cap `events.jsonl` to its newest lines on resume. A seat whose meta still
/// holds a live effort record keeps the journal evidence its episode stands on
/// ([`crate::effort::retained`]), or the resume that keeps the record would
/// fold a journal that no longer proves it and hold in doubt for good.
fn trim_events(dir: &Path) {
    // An absent meta holds no effort record. Any other failed read leaves no
    // telling which records the cut must keep proving, and retention is a
    // best effort step: skip it.
    let bytes = match meta::read_bytes(dir) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => return,
    };
    let parsed = Meta::parse(&String::from_utf8_lossy(&bytes));
    let live: Vec<crate::effort::Live<'_>> = parsed
        .roster()
        .iter()
        .filter_map(|seat| match crate::effort::stored(&bytes, &seat.slot) {
            crate::effort::Stored::Present(record) => Some(crate::effort::Live {
                slot: &seat.slot,
                agent: &seat.name,
                episode: record.episode,
            }),
            crate::effort::Stored::Absent | crate::effort::Stored::Damaged(_) => None,
        })
        .collect();
    let name = dir
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default();
    let session = crate::events::SessionKey::new(name, parsed.session_id().unwrap_or_default());
    crate::store::open(dir).retain_events_holding(
        EVENTS_KEEP,
        crate::restore::pins_clean_stop,
        |dropped, window| crate::effort::retained(dropped, window, session, &live),
    );
}

/// One spawned seat recovered from a resuming session's own meta.
struct Spawned {
    slot: String,
    name: String,
    profile: String,
    /// The recorded `client.<slot>` carried forward, if the seat records one:
    /// a `spawn --using profile@client` writes it, and it decides which
    /// account the restored seat runs. A carried label is evidence, and an
    /// Invalid row refuses rather than laundering toward Missing.
    client: Option<String>,
    binary: String,
    harness_session: String,
    config_home: Option<String>,
    config_home_base: Option<String>,
}

/// The launch command of one restored spawned seat, resolved through the
/// client it RECORDED — the one resolution both the pre-create lexing and the
/// restore take, so neither can start a seat on its profile's default account.
///
/// `Ok(None)` is a profile this machine does not configure: the seat is kept
/// verbatim, as it always was.
///
/// # Errors
///
/// The refusal line: a recorded client no `[clients]` row names now, a command
/// that is not one simple command, or an expansion that refuses.
fn spawned_command(
    cfg: &IdentityConfig,
    entry: &Spawned,
    home: Option<&Path>,
) -> Result<Option<config::ResolvedCommand>, String> {
    let Some(label) = entry.client.as_deref() else {
        return cfg
            .command(&entry.profile, home)
            .map_err(|why| why.to_string());
    };
    match cfg.command_with_client(&entry.profile, label, home) {
        Ok(resolved) => Ok(Some(resolved.command)),
        Err(config::OverrideError::UnknownProfile) => Ok(None),
        Err(config::OverrideError::UnknownClient) => Err(format!(
            "Error: seat '{}' ({}) recorded client override '{label}' but no [clients] row names it now — \
             restore the '{label}' client, or end the session.",
            entry.name, entry.slot
        )),
        Err(config::OverrideError::ProfileNotSimple(why)) => Err(format!(
            "Error: seat '{}' ({}) — profile '{}' refused — {why}. Nothing was resumed.",
            entry.name, entry.slot, entry.profile
        )),
        Err(config::OverrideError::Refused(why)) => Err(why.to_string()),
    }
}

/// The `spawned.<n>` seats a resuming session's meta records, in slot order.
///
/// Both callers resume-gate this: a fresh launch never asks. An unreadable
/// meta therefore refuses — an empty list would rebuild without the spawned
/// seats and drop their seat, dir and transcript rows for good.
///
/// # Errors
///
/// An unreadable meta, or an Invalid recorded client row with its remedy —
/// the meta rewrite would otherwise launder it.
fn spawned_entries(dir: &Path, session: &str) -> Result<Vec<Spawned>, String> {
    let bytes = meta::read_bytes(dir)
        .map_err(|why| format!("cannot read the meta to restore spawned seats: {why}"))?;
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
        out.push(Spawned::of(entry, client));
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

/// Every recorded fact of one spawned seat a judgement of its client is bound
/// to: a row that differs in any of them after the judgement is another seat.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SpawnedIdentity {
    slot: String,
    name: String,
    profile: String,
    client: Option<String>,
    binary: String,
    config_home: Option<String>,
    config_home_base: Option<String>,
}

impl Spawned {
    /// The seat roster row `entry` names, its client row already read.
    fn of(entry: &meta::RosterEntry, client: Option<String>) -> Self {
        Self {
            slot: entry.slot.clone(),
            name: entry.name.clone(),
            profile: entry.profile.clone().unwrap_or_default(),
            client,
            binary: entry.binary.clone().unwrap_or_default(),
            harness_session: entry.harness_session.clone().unwrap_or_default(),
            config_home: entry.config_home.record_value(),
            config_home_base: entry.config_home_base.record_value(),
        }
    }

    fn identity(&self) -> SpawnedIdentity {
        SpawnedIdentity {
            slot: self.slot.clone(),
            name: self.name.clone(),
            profile: self.profile.clone(),
            client: self.client.clone(),
            binary: self.binary.clone(),
            config_home: self.config_home.clone(),
            config_home_base: self.config_home_base.clone(),
        }
    }
}

/// A labeled spawned seat as the pre-lock judgement left it: the identity it
/// judged and the command that seat may run — `None` for a profile the
/// recorded global config does not define, which keeps the seat verbatim and
/// starts nothing.
#[derive(Debug, Clone)]
struct JudgedSpawned {
    identity: SpawnedIdentity,
    command: Option<config::ResolvedCommand>,
}

/// The refusal for a session whose labeled spawned seat has no global config
/// to resolve its recorded client in — the only config its pair was approved
/// by. An unavailable config is not an absent profile: nothing is assumed.
fn unavailable_authority_refusal(session: &str, entry: &meta::RosterEntry, why: &str) -> String {
    format!(
        "Error: session '{session}' seat '{}' ({}) records a client override, but {why} — \
         restore the session's global config, or end the session (ae end {session}). Nothing was resumed.",
        entry.name, entry.slot
    )
}

/// Judge every labeled `spawned.<n>` row of the meta at `dir` against the
/// session's recorded GLOBAL identity: the recorded client must still be a
/// global `[clients]` row (else the removed-client refusal, ahead of any
/// missing profile); a profile the global config lacks keeps the seat
/// unstarted; any other pair passes the launch's adapter gate, the recorded
/// store re-take and the recorded harness. Unlabeled rows are not judged.
///
/// A meta that does not read judges nothing: the restore then refuses every
/// labeled row as unjudged, and the existing restore refusal names the meta.
fn judge_spawned_labels(
    dir: &Path,
    session: &str,
    home: Option<&Path>,
) -> Result<Vec<JudgedSpawned>, SeatOverrideRefusal> {
    let Ok(bytes) = meta::read_bytes(dir) else {
        return Ok(Vec::new());
    };
    let parsed = Meta::parse(&String::from_utf8_lossy(&bytes));
    let labeled: Vec<&meta::RosterEntry> = parsed
        .roster()
        .iter()
        .filter(|entry| entry.slot.starts_with("spawned."))
        .filter(|entry| matches!(entry.client, crate::meta::RecordedClient::Label(_)))
        .collect();
    let Some(first) = labeled.first() else {
        return Ok(Vec::new());
    };
    let global = crate::lifecycle::meta_value(&bytes, "config");
    if global.is_empty() {
        return Err(SeatOverrideRefusal::Failed(unavailable_authority_refusal(
            session,
            first,
            "the session records no global config",
        )));
    }
    let cfg = config::read_identity(Some(Path::new(&global)), None).map_err(|why| {
        SeatOverrideRefusal::Failed(unavailable_authority_refusal(
            session,
            first,
            &format!("its global config {global} cannot be used ({why})"),
        ))
    })?;
    let profiles: Vec<&str> = cfg.profiles.iter().map(|(name, _)| name.as_str()).collect();
    let clients: Vec<&str> = cfg.clients.iter().map(|(name, _)| name.as_str()).collect();
    let (known_profiles, known_clients) = (known_list(&profiles), known_list(&clients));
    let mut judged = Vec::with_capacity(labeled.len());
    for entry in labeled {
        let RecordedSelection::Label(label) =
            recorded_selection(session, &cfg, entry).map_err(SeatOverrideRefusal::Failed)?
        else {
            continue;
        };
        let profile = entry.profile.as_deref().unwrap_or_default();
        let command = if cfg.profile(profile).is_none() {
            None
        } else {
            let (command, lexed) = resolve_client_override(
                &cfg,
                &entry.name,
                profile,
                &label,
                home,
                &known_profiles,
                &known_clients,
            )?;
            refuse_recorded_client_conflict(
                session,
                &entry.name,
                profile,
                Some(&label),
                entry,
                &cfg,
                &command,
                lexed.tool(),
                home,
            )?;
            let recorded = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or_default());
            if recorded != lexed.tool() {
                return Err(SeatOverrideRefusal::Usage(cross_tool_refusal(
                    &entry.name,
                    recorded,
                    lexed.tool(),
                )));
            }
            Some(command)
        };
        judged.push(JudgedSpawned {
            identity: Spawned::of(entry, Some(label)).identity(),
            command,
        });
    }
    Ok(judged)
}

/// The command a restored spawned seat runs. A labeled seat runs ONLY what its
/// pre-lock judgement allowed, and only while its recorded facts are the ones
/// judged; an unlabeled seat resolves as it always did. A label added, dropped
/// or changed — or any judged fact moved — between the judgement and this read
/// refuses: no unjudged resolution of a labeled seat ever runs.
///
/// # Errors
///
/// The refusal line: a stale or missing judgement, or [`spawned_command`]'s.
fn restored_command(
    cfg: &IdentityConfig,
    entry: &Spawned,
    judged: &[JudgedSpawned],
    session: &str,
    home: Option<&Path>,
) -> Result<Option<config::ResolvedCommand>, String> {
    let held = judged.iter().find(|held| held.identity.slot == entry.slot);
    match (&entry.client, held) {
        (None, None) => spawned_command(cfg, entry, home),
        (Some(_), Some(held)) if held.identity == entry.identity() => Ok(held.command.clone()),
        _ => Err(format!(
            "Error: session '{session}' seat '{}' ({}) changed its recorded client identity while this launch was starting — \
             nothing was resumed; retry the launch.",
            entry.name, entry.slot
        )),
    }
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
mod tests;
