//! Launch arguments: the `_launch` argv grammar, the working-copy mode and the environment the launch reads.

use super::{PathBuf, ServerId, TEST_PRE_LOCK_MARKER};

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
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Git => "git",
            Self::Full => "full",
        }
    }

    /// The mode a recorded value names, or `None` when it names none.
    pub(super) fn parse(value: &str) -> Option<Self> {
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
    pub(super) test_pre_lock_marker: Option<PathBuf>,
}

impl Env {
    /// The sessions root.
    pub(super) fn sessions(&self) -> PathBuf {
        self.home.join("sessions")
    }

    /// The managed working-copy root.
    pub(super) fn worktrees(&self) -> PathBuf {
        self.home.join("worktrees")
    }

    /// The server every tmux call in this launch addresses.
    pub(super) fn server(&self) -> ServerId {
        ServerId::from_typed_flags(&self.server_kind, &self.server_value)
            .unwrap_or(ServerId::Ambient)
    }

    /// The config files, in overlay order, for the document renders.
    pub(super) fn config_files(&self) -> Vec<PathBuf> {
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
pub(super) enum EnvError {
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
pub(super) fn read_env(tail: &[String]) -> Result<(Env, Vec<String>), EnvError> {
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
