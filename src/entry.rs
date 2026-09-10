//! The product's ENTRY: the ambient facts an invocation carries, and the
//! dispatch every word is routed by.

use std::path::PathBuf;

use crate::inventory::ServerId;
use crate::meta::Selector;

/// One profile `ae init` may offer, tied to the client/executable whose
/// discovery makes it usable. Client and profile commands stay in
/// [`DEFAULT_CONFIG`], so first-run seeding and init have one command catalog
/// rather than two copies that can drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    /// Config key under `[profiles]`.
    pub name: &'static str,
    /// Executable looked up through doctor's PATH resolver.
    pub harness: &'static str,
    /// Model provider when the profile establishes one; `None` means an init
    /// proposal cannot prove provider diversity from the command alone.
    pub provider: Option<&'static str>,
}

/// The client/profile catalog shared by the seeded config and `ae init`.
pub const PROFILE_CATALOG: &[Profile] = &[
    Profile {
        name: "opus5",
        harness: "claude",
        provider: Some("Anthropic"),
    },
    Profile {
        name: "fable5",
        harness: "claude",
        provider: Some("Anthropic"),
    },
    Profile {
        name: "fablex",
        harness: "claude",
        provider: Some("Anthropic"),
    },
    Profile {
        name: "opusx",
        harness: "claude",
        provider: Some("Anthropic"),
    },
    Profile {
        name: "gpt56sol",
        harness: "codex",
        provider: Some("OpenAI"),
    },
    Profile {
        name: "gpt6astra",
        harness: "codex",
        provider: Some("OpenAI"),
    },
    Profile {
        name: "gpt56luna",
        harness: "codex",
        provider: Some("OpenAI"),
    },
    Profile {
        name: "gpt56terra",
        harness: "codex",
        provider: Some("OpenAI"),
    },
    Profile {
        name: "astrax",
        harness: "codex",
        provider: Some("OpenAI"),
    },
    Profile {
        name: "solx",
        harness: "codex",
        provider: Some("OpenAI"),
    },
    Profile {
        name: "gpt56solx",
        harness: "codex",
        provider: Some("OpenAI"),
    },
    Profile {
        name: "grok46",
        harness: "grok",
        provider: Some("xAI"),
    },
    Profile {
        name: "agy",
        harness: "agy",
        provider: Some("Google"),
    },
    Profile {
        name: "opencode",
        harness: "opencode",
        provider: None,
    },
    Profile {
        name: "gemini",
        harness: "gemini",
        provider: None,
    },
];

/// The config `ae` writes on a first run.
pub const DEFAULT_CONFIG: &str = r##"# ae config — auto-created on first run, yours to edit. Also mirrored in the repo as
# config.sample. INI-style: [section] headers, key = value, "#" starts a comment.
# Run `ae init` to discover installed harnesses and propose a smaller starting roster.
# New sessions read this whole file. Stopping + resuming an existing session (ae stop <name>;
# ae <name>) relaunches its preserved agents with their CURRENT command + [prompt] — but the
# roster (main/workers), layout, and watchdog stay pinned in session meta, so edits to those
# take effect for NEW sessions only. (ae doctor --refresh regenerates the on-disk session
# helpers + workspace.md after you upgrade ae; it changes neither running agents nor config.)

[clients]
# A CLIENT names one installed CLI executable. Profiles below build model flags and permissions
# on these names. `ae init` keeps one no-op client alias per executable it finds on PATH.
# Add `config_home=$HOME/<dir>` to a SECOND Claude or Codex client to give it an independent
# login and conversation store; see docs/getting-started/config.md. Do not add config_home to
# the default client.
claude = claude
codex = codex
grok = grok
agy = agy
opencode = opencode
gemini = gemini

[profiles]
# Register any CLI tool as a PROFILE: profile = "the shell command that launches it". A
# profile is a reusable launch recipe, not an identity: an agent IS its NAME, bound to a
# profile in [roster] below. The command must be ONE simple command (env assignments plus
# one argv; no ; | & # redirections or $(…) outside quotes) — ae refuses anything else.
# Model-named aliases: the prefix names the model + major, so the running model is legible at
# a glance. STRICT pins (never --model best) — a model-named alias must not silently run a
# different model; that would make the name lie and blind the model-drift alarm. Exact IDs
# beat family aliases where they would drift: --model opus moved to Opus 5 the day it shipped.
# STRONG DEV (build slices): gpt56sol xhigh or opus5 xhigh.
opus5 = "claude --permission-mode bypassPermissions --model claude-opus-5 --effort xhigh"
gpt56sol = "codex --yolo -m gpt-5.6-sol -c model_reasoning_effort=xhigh"
# BRAINPOWER (lead/colead seats, plans, rulings, hard debugging): fable5 xhigh or gpt6astra xhigh.
fable5 = "claude --permission-mode bypassPermissions --model fable --effort xhigh"
gpt6astra = "codex --yolo -m gpt-6-astra -c model_reasoning_effort=xhigh"
# CHORES/tests/simple slices: gpt56luna xhigh; it also runs the orchestrator seat.
gpt56luna = "codex -m gpt-5.6-luna -c model_reasoning_effort=xhigh -a never"
# REVIEWER when usage allows: grok46.
grok46 = "grok --always-approve -m grok-4.6 --effort high"
gpt56terra = "codex --yolo -m gpt-5.6-terra -c model_reasoning_effort=xhigh"
opencode = "opencode"
# `ae init` uses these seat shortcuts and additional harness profiles.
fablex = "claude --permission-mode bypassPermissions --model fable --effort xhigh"
opusx = "claude --permission-mode bypassPermissions --model claude-opus-5 --effort xhigh"
astrax = "codex --yolo -m gpt-6-astra -c model_reasoning_effort=xhigh"
solx = "codex --yolo -m gpt-5.6-sol -c model_reasoning_effort=xhigh"
gpt56solx = "codex -m gpt-5.6-sol -c model_reasoning_effort=xhigh -a never"
agy = "agy --dangerously-skip-permissions"
gemini = "gemini"

[roster]
# The agents promised to launch: name = profile. The NAME is the identity of the agent — it is
# addressed as <name> in the session and <session>:<name> across sessions; the profile is
# metadata (`ae list` shows it). Every seat in main/workers below must be bound here — ae
# refuses the launch otherwise and lists every violation. A name bound here but not seated
# is legal: `ae <session> use <name>` starts it as main instead. Spawn on demand with:
# spawn <name> --using <profile> [prompt].
lead = fable5
colead = gpt6astra
orchestrator = gpt56luna

[workspace]
# main = the standing main seat (a [roster] NAME) — under lead-pair a technical lifecycle
# anchor, not a rank. workers = comma-separated [roster] names launched at start.
# layout = vertical | horizontal | lead-solo | lead-pair. watchdog = true nudges stale/idle agents.
# The leads delegate by rule (see docs/reference/delegation.md — spawn workers on demand).
# Standing seats are the JUDGMENT PAIR only: under lead-pair the FIRST worker (worker.0)
# is the COLEAD seat — an EQUAL leadership peer of the lead (interchangeable, same level,
# sharing the leads window 0:leads); main stays the technical lifecycle anchor (compact
# handover), which is infrastructure, not seniority. Builders and reviewers are NOT
# standing seats: either peer spawns them per slice (spawn builder --using opus5 / spawn
# reviewer --using grok46) and retires its own spawns when the work is verified — every spawn
# ends in a retire. This keeps idle panes at zero, makes the retire contract do real
# work, and still keeps judgment (the pair) separate from review (a spawned seat) on
# every slice.
main = lead
workers = colead
layout = lead-pair
watchdog = true
# Installed ae checks for newer releases during validated ordinary use. This
# machine policy is global-only; a project's .ae/config cannot override it.
# auto_upgrade = on

[prompt]
# ae already injects the full workspace protocol into every agent — the roster, the helper
# commands (send/ask/spawn/…), the delegation and comms rules — see the generated
# ~/.ae/sessions/<name>/workspace.md. Anything set here is APPENDED on top of that; per-project
# .ae/config overrides the global one. Uncomment to add your own house rules:
# instructions = "Always write tests. Prefer TypeScript."
"##;

/// The text `ae help` prints — the glue's `cmd_help`, verbatim.
pub const HELP: &str = r"ae - agentic engineering: tmux multi-agent workspace

Usage:
  ae                     Attach to the ae tmux server
  ae <name>              Start or reattach a named session
  ae <name> --solo       lead only, no colead
  ae <name> --dir <path> Start or reattach using an explicit origin directory
  ae <name> --no-attach  Start or reattach without attaching; print attach command
  ae <name> use <name>   Start session with a specific agent as main
  ae <name> --seat <agent>=<profile>
                         Use a different profile for one launch seat (repeatable)
  ae <name> --lead <profile> --colead <profile>
                         Shortcuts for the named lead and colead seats
  ae --local <name>      Start session in current directory (default)
  ae --copy <name>       Start session with full copy (includes untracked files)
  ae --worktree <name>   Start session with git worktree (tracked files only)
  ae <new-name> --from <archive-uuid>
                         Start a NEW session that explicitly continues an archived one
                         (the main agent is told to read that archive's digest first)
  ae list [--running|--all|--stopped|--needs-attn|--active] [--json]
                         List sessions (running by default). 'ae list --help' has the
                         full filter set and what --json carries
  ae next [--attach]     Name the top session needing attention (--attach jumps to it)
  ae brief [name] [--all] [--since <dur>]
                         Card one session or the fleet: goal, the latest note per memo
                         topic, each agent's declared state, and who is waiting on you
  ae quota               Show local cached quota windows for configured agent profiles
  ae usage [name…] [--json]
                         Show API-equivalent list-price usage for live sessions
  ae orchestrator        Start or reattach the orchestrator seat (a session named
                         orchestrator; config: ~/.ae/orchestrator.config)
  ae orchestrator --popup --client <name>
                         Pick a session, then one of its agents, in a tmux menu and
                         hand this client to that agent's pane (needs tmux 3.4+).
                         Status button and prefix a bindings supply the client name
  ae doctor [--refresh [name|all]]
                         Check local environment and optionally refresh existing session helpers
  ae init [--yes]        Discover installed harnesses and propose a global config
  ae rename [old] <new>  Rename a running session
  ae watchdog <start|stop|status> [name]
                         Toggle the stale-agent watchdog for a session
  ae telegram <start|stop|status>
                         Machine-global Telegram bridge (forwards events to a chat)
  ae stop [name]         Pause session, keep ae + agent conversation state for resume
                         (or 'ae stop all')
  ae compact [-f] [--keep-history] [--digest-only] [name]
                         Hand this session over to a fresh one: freeze the roster, archive
                         the memory, end it, and relaunch the same agents against that
                         archive. --digest-only writes nothing and prints what it would say
  ae archive preview [name]
                         Print the digest an end would archive (read-only; writes nothing)
  ae end|rm [-f] [--purge-history|--keep-history] [name]
                         End session: commit, push to ae/<name>, ARCHIVE its memory to
                         ~/.ae/archive/<session-uuid>/, then remove ae state. The archive
                         is mandatory: if it cannot be written, the end fails and nothing
                         is deleted. KEEPS the claude/codex/agy conversation files by default
                         (token history); --purge-history deletes them AND writes no
                         archive (removing any existing one) ([workspace]
                         purge_agent_history sets the default). (or 'ae end all')
  ae version             Show version, tmux floor, auto-upgrade policy and last check
  ae help                Show this help

Modes: --local (default), --copy (full cp -a), --worktree (git worktree).
Sessions persist across reboots. Agents with session support resume conversations; others start fresh.
When inside an ae session, stop/end/compact work without specifying the name.

Config: ~/.ae/config (per-project override: .ae/config in project dir)
Session helpers, in every session dir: send, relay, ask, review, reply, requests, state,
  mark-done, goal, memo, say, peek (peak), agents, quota, usage, focus, interrupt, spawn, retire.
Run 'ae doctor' after install or agent CLI upgrades.
Run 'ae doctor --refresh' after updating ae to regenerate existing session helpers.
";

/// The text `ae list --help` prints — the glue's `LISTHELP` heredoc, verbatim.
pub const LIST_HELP: &str = r"Usage: ae list [--running | --all | --stopped | --needs-attn | --active] [--json]
  (default)    running sessions only
  --running    running sessions only (explicit)
  --all        running sessions, then stopped ones
  --stopped    stopped sessions only
  --needs-attn only running sessions with an attn reason — declared
               waiting-user/blocked, watchdog-derived dead/stale/throttled, or an
               unanswered inter-agent ask/review (older than 30m); implies
               running-only
               (aliases: --needs-me, --needs, --attn)
  --active     only running sessions with recent activity (an ae event in the
               last 5 min). Implies running-only (alias: --busy)
  --json       machine-readable digest (schema_version, per-session
               needs_attention + attention reason, per-agent state); honours
               the filters above.
";

/// `ae status` — CUT, and the arm is the refusal.
pub const RETIRED_STATUS: &str =
    "Error: 'ae status' was retired. Use 'ae list' (add --json for the full record).\n";

/// `ae hub` — not a command, and the arm is the refusal that says so.
pub const RETIRED_ORCHESTRATOR: &str = "Error: 'ae hub' was retired.\nThe orchestrator seat is 'ae orchestrator'; the fleet picker is 'ae orchestrator --popup'.\n";

/// `ae transfer` — CUT rather than ported, and the arm is the refusal.
pub const RETIRED_TRANSFER: &str = "Error: 'ae transfer' was cut, not ported — ae does no cross-machine session sync.\nMove the WORK instead: 'ae end <name>' commits and pushes it to the 'ae/<name>' branch,\nthen start a session from that branch on the other machine.\n";

/// `ae archive`'s usage, for a second word that is not `preview`.
pub const ARCHIVE_USAGE: &str = "Usage: ae archive preview [session-name]\n";

/// The refusal a nameless `ae archive preview` gets outside a session.
pub const ARCHIVE_PREVIEW_USAGE: &str = "Usage: ae archive preview [session-name]\n(Run inside an ae tmux session to preview it without naming it.)\n";

/// The exit code a usage error takes — the crate's, kept distinct from `1`.
pub const EXIT_USAGE: u8 = 2;

/// The exit code an operation that could not be carried out takes.
pub const EXIT_FAILED: u8 = 1;

/// The ambient facts an invocation carries — every one of them resolved from a
/// door in [`crate::doors`], and none of them readable from the argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preamble {
    /// `AE_HOME` — where every piece of ae state lives.
    pub home: PathBuf,
    /// The caller's working directory.
    pub cwd: PathBuf,
    /// The global config file, when one was selected.
    pub global: Option<PathBuf>,
    /// The origin-local `.ae/config`, when there is one.
    pub local: Option<PathBuf>,
    /// `socket`, `name`, `ambiguous` or empty — the resolved server's kind.
    pub server_kind: String,
    /// The resolved server's value.
    pub server_value: String,
    /// The launch target before a running named server is upgraded to its
    /// socket spelling. Bare `ae` keeps this spelling in attach hints.
    pub launch_target: Option<ServerId>,
    /// The tmux server whose client invoked ae, resolved from `$TMUX` alone.
    pub caller_server: Option<ServerId>,
    /// Whether the caller is genuinely inside a tmux pane (attach vs switch).
    pub inside_tmux: bool,
    /// Whether to attach once the session is up.
    pub attach: bool,
    /// The operator's `AE_NO_AUTOSTART=1`: start no companion — neither the
    /// watchdog nor the Telegram bridge.
    pub no_autostart: bool,
}

impl Default for Preamble {
    /// `attach` is the only field whose zero value is wrong: a launch attaches
    /// unless it is told not to, which is what the glue's `ATTACH_ON_READY`
    /// meant.
    fn default() -> Self {
        Self {
            home: PathBuf::new(),
            cwd: PathBuf::new(),
            global: None,
            local: None,
            server_kind: String::new(),
            server_value: String::new(),
            launch_target: None,
            caller_server: None,
            inside_tmux: false,
            attach: true,
            no_autostart: false,
        }
    }
}

impl Preamble {
    /// The launch's own preamble, rebuilt as `_launch`'s flags.
    #[must_use]
    pub fn launch_argv(&self, user: &[String]) -> Vec<String> {
        let mut argv = vec![crate::cli::LAUNCH.to_owned()];
        argv.push("--home".to_owned());
        argv.push(self.home.to_string_lossy().into_owned());
        argv.push("--cwd".to_owned());
        argv.push(self.cwd.to_string_lossy().into_owned());
        if let Some(global) = &self.global {
            argv.push("--global".to_owned());
            argv.push(global.to_string_lossy().into_owned());
        }
        if let Some(local) = &self.local {
            argv.push("--local-config".to_owned());
            argv.push(local.to_string_lossy().into_owned());
        }
        if self.no_autostart {
            argv.push("--no-autostart".to_owned());
        }
        argv.extend(self.server_argv());
        if let Some(ServerId::Selected(Selector::Socket(socket))) = &self.caller_server {
            argv.push("--caller-socket".to_owned());
            argv.push(socket.to_string_lossy().into_owned());
        }
        argv.push(
            if self.attach {
                "--attach"
            } else {
                "--no-attach"
            }
            .to_owned(),
        );
        if self.inside_tmux {
            argv.push("--inside-tmux".to_owned());
        }
        argv.push("--".to_owned());
        argv.extend_from_slice(user);
        argv
    }

    /// The typed server pair, or nothing when neither half resolved.
    fn server_argv(&self) -> Vec<String> {
        if self.server_kind.is_empty() && self.server_value.is_empty() {
            return Vec::new();
        }
        vec![
            "--server-kind".to_owned(),
            self.server_kind.clone(),
            "--server".to_owned(),
            self.server_value.clone(),
        ]
    }

    /// The sessions root under this home.
    #[must_use]
    pub fn sessions(&self) -> PathBuf {
        self.home.join("sessions")
    }
}

/// What a human-typed invocation resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// `ae help` — [`HELP`] on stdout, exit 0.
    Help,
    /// `ae version` — the version line on stdout, exit 0.
    Version,
    /// `ae list --help` — [`LIST_HELP`] on stderr, exit 0.
    ListHelp,
    /// One of the four cut words: the text on stderr, exit 2.
    Retired(&'static str),
    /// `ae archive preview [name]` — the name still has to be resolved and
    /// path-checked against the live world, which is the caller's job.
    ArchivePreview(Option<String>),
    /// `ae archive <anything else>` — [`ARCHIVE_USAGE`], exit 1.
    ArchiveUsage,
    /// A word the core already answers: the effective argv, environmental facts
    /// appended, for the ordinary dispatch.
    Core(Vec<String>),
    /// An EMPTY argv: attach to the ae tmux server or list from inside it.
    Attach,
    /// Everything else: create or resume an explicitly named session.
    Launch(Vec<String>),
}

/// Which route the user's argv takes — the glue's `case` statement, whole.
///
/// A `_`-prefixed first word never arrives: [`crate::run`] dispatches the core's
/// own namespace before this is called.
///
/// `pane` is `$TMUX_PANE`, which only two words need: `stop` and `watchdog`
/// both answer "is the target the session I am in", and that question starts
/// from the pane this process sits in. An explicit `--pane` in the caller's own
/// argv WINS, because in a `run-shell` child the inherited variable names a
/// FOREIGN pane and the expanded `#{pane_id}` is the only trustworthy answer.
///
/// ```
/// use ae::entry::{Preamble, Route, route};
/// let preamble = Preamble::default();
/// assert_eq!(route(&preamble, &[], None), Route::Attach);
/// assert!(matches!(route(&preamble, &["status".to_owned()], None), Route::Retired(_)));
/// ```
#[must_use]
pub fn route(preamble: &Preamble, argv: &[String], pane: Option<&str>) -> Route {
    let tail = || argv[1..].to_vec();
    match argv.first().map(String::as_str) {
        None => Route::Attach,
        Some("list" | "ls") => {
            if argv[1..]
                .iter()
                .any(|word| word == "-h" || word == "--help")
            {
                Route::ListHelp
            } else {
                Route::Core(with_head("list", &tail()))
            }
        }
        Some("next" | "jump") => Route::Core(with_head("next", &tail())),
        Some("brief") => Route::Core(with_head("brief", &tail())),
        Some("quota") => Route::Core(with_head("quota", &tail())),
        Some("usage") => Route::Core(with_head("usage", &tail())),
        Some("compact") => Route::Core(with_head(crate::cli::COMPACT, &tail())),
        Some("archive") => match argv.get(1).map(String::as_str) {
            Some("preview") => Route::ArchivePreview(argv.get(2).cloned()),
            _ => Route::ArchiveUsage,
        },
        Some("doctor") => Route::Core(with_head(crate::cli::DOCTOR, &tail())),
        Some("init") => Route::Core(with_head(crate::cli::INIT, &tail())),
        Some("stop") => Route::Core(with_head(crate::cli::STOP, &with_pane(&tail(), pane))),
        Some("rename") => Route::Core(with_head(crate::cli::RENAME, &tail())),
        // `loop` is the deprecated spelling of the renamed feature, kept as an
        // alias for sessions created before it.
        Some("watchdog" | "loop") => {
            Route::Core(with_head(crate::cli::WATCHDOG, &with_pane(&tail(), pane)))
        }
        Some("telegram") => Route::Core(with_head(
            crate::cli::TELEGRAM,
            &telegram_tail(preamble, &tail()),
        )),
        Some("end" | "rm") => Route::Core(with_head(crate::cli::END, &tail())),
        Some("status") => Route::Retired(RETIRED_STATUS),
        // The BARE word is a launch of the orchestrator seat, which needs the
        // preamble like any launch; with a tail (`--popup`) it is the core's picker.
        Some("orchestrator") => {
            let tail = tail();
            if crate::orchestrator::launch_tail_is_valid(&tail) {
                let mut launch = crate::orchestrator::seat_launch_args();
                launch.extend(tail);
                Route::Launch(launch)
            } else {
                Route::Core(with_head("orchestrator", &tail))
            }
        }
        Some("hub") => Route::Retired(RETIRED_ORCHESTRATOR),
        Some("transfer") => Route::Retired(RETIRED_TRANSFER),
        Some("help" | "-h" | "--help") => Route::Help,
        Some("version" | "--version" | "-V") => Route::Version,
        Some(_) => Route::Launch(argv.to_vec()),
    }
}

/// `head` followed by `tail` — the one shape every translated word takes.
fn with_head(head: &str, tail: &[String]) -> Vec<String> {
    let mut argv = Vec::with_capacity(tail.len() + 1);
    argv.push(head.to_owned());
    argv.extend_from_slice(tail);
    argv
}

/// Append `--pane <id>` unless the caller named one itself.
fn with_pane(tail: &[String], pane: Option<&str>) -> Vec<String> {
    let named = tail
        .iter()
        .any(|word| word == "--pane" || word.starts_with("--pane="));
    let mut words = tail.to_vec();
    if let Some(pane) = pane.filter(|id| !id.is_empty() && !named) {
        words.push("--pane".to_owned());
        words.push(pane.to_owned());
    }
    words
}

/// The telegram tail: the caller's words, then the environment the core will
/// not read for itself — which config to honour, which home to keep state
/// under, and which server the daemon's session belongs on.
fn telegram_tail(preamble: &Preamble, tail: &[String]) -> Vec<String> {
    let mut words = tail.to_vec();
    if let Some(global) = &preamble.global {
        words.push("--config".to_owned());
        words.push(global.to_string_lossy().into_owned());
    }
    words.push("--home".to_owned());
    words.push(preamble.home.to_string_lossy().into_owned());
    words.extend(preamble.server_argv());
    words
}

/// Whether `name` could be a DIRECT CHILD of the sessions root, by pure string
/// structure — the belt to the grammar, before anything on disk is touched.
#[must_use]
pub fn is_direct_child_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && name != "." && name != ".."
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_CONFIG, HELP, LIST_HELP, Preamble, Route, is_direct_child_name, route};

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    fn preamble() -> Preamble {
        Preamble {
            home: "/h".into(),
            cwd: "/c".into(),
            global: Some("/h/config".into()),
            ..Preamble::default()
        }
    }

    #[test]
    fn an_empty_argv_takes_the_attach_route() {
        assert_eq!(route(&preamble(), &[], None), Route::Attach);
    }

    #[test]
    fn the_cut_words_refuse_instead_of_becoming_session_names() {
        for word in ["status", "hub", "transfer"] {
            let Route::Retired(text) = route(&preamble(), &argv(&[word]), None) else {
                panic!("'{word}' must refuse, not launch");
            };
            assert!(text.starts_with("Error: "), "{text}");
        }
    }

    #[test]
    fn init_is_a_core_command_not_a_session_name() {
        assert_eq!(
            route(&preamble(), &argv(&["init", "--yes"]), None),
            Route::Core(argv(&["init", "--yes"]))
        );
    }

    #[test]
    fn quota_is_carried_as_the_public_command_the_early_dispatch_answers() {
        assert_eq!(
            route(&preamble(), &argv(&["quota"]), None),
            Route::Core(argv(&["quota"]))
        );
        assert_eq!(
            route(&preamble(), &argv(&["quota", "extra"]), None),
            Route::Core(argv(&["quota", "extra"]))
        );
    }

    #[test]
    fn usage_is_carried_as_the_public_command_the_early_dispatch_answers() {
        assert_eq!(
            route(&preamble(), &argv(&["usage", "demo", "--json"]), None),
            Route::Core(argv(&["usage", "demo", "--json"]))
        );
    }

    #[test]
    fn the_orchestrator_word_reaches_the_core_and_hub_refuses() {
        assert_eq!(
            route(&preamble(), &argv(&["orchestrator", "--popup"]), None),
            Route::Core(argv(&["orchestrator", "--popup"]))
        );
        assert_eq!(
            route(&preamble(), &argv(&["orchestrator"]), None),
            Route::Launch(argv(&["orchestrator"]))
        );
        for flag in ["--attach", "--no-attach", "--inside-tmux", "--no-autostart"] {
            assert_eq!(
                route(&preamble(), &argv(&["orchestrator", flag]), None),
                Route::Launch(argv(&["orchestrator", flag]))
            );
        }
        for flag in ["--popup", "--copy", "--worktree", "--from", "use", "--nope"] {
            assert!(
                matches!(
                    route(&preamble(), &argv(&["orchestrator", flag]), None),
                    Route::Core(_)
                ),
                "{flag} must remain a core usage error"
            );
        }
    }

    #[test]
    fn an_underscore_word_never_reaches_the_router() {
        // The core's own namespace is dispatched before `route` is called, so
        // an internal word arriving here would be a caller bug.
        assert_eq!(
            route(&preamble(), &argv(&["_spawn", "/s/x", "helper"]), None),
            Route::Launch(argv(&["_spawn", "/s/x", "helper"]))
        );
    }

    #[test]
    fn the_human_words_translate_to_the_core_entries() {
        let table = [
            (argv(&["end", "-f", "x"]), argv(&["_end", "-f", "x"])),
            (argv(&["rm", "x"]), argv(&["_end", "x"])),
            (argv(&["compact", "x"]), argv(&["_compact", "x"])),
            (argv(&["rename", "a", "b"]), argv(&["rename", "a", "b"])),
            (argv(&["ls", "--all"]), argv(&["list", "--all"])),
            (argv(&["jump", "--attach"]), argv(&["next", "--attach"])),
            (argv(&["brief", "--all"]), argv(&["brief", "--all"])),
        ];
        for (typed, effective) in table {
            assert_eq!(route(&preamble(), &typed, None), Route::Core(effective));
        }
    }

    #[test]
    fn list_help_is_routed_out_of_the_core_flag_parser() {
        assert_eq!(
            route(&preamble(), &argv(&["list", "--help"]), None),
            Route::ListHelp
        );
        assert_eq!(
            route(&preamble(), &argv(&["ls", "-h"]), None),
            Route::ListHelp
        );
        // Any other tail stays the core's to parse, unknown flags included.
        assert_eq!(
            route(&preamble(), &argv(&["list", "--nope"]), None),
            Route::Core(argv(&["list", "--nope"]))
        );
    }

    #[test]
    fn the_pane_is_appended_only_when_the_caller_named_none() {
        assert_eq!(
            route(&preamble(), &argv(&["stop", "x"]), Some("%7")),
            Route::Core(argv(&["_stop", "x", "--pane", "%7"]))
        );
        assert_eq!(
            route(&preamble(), &argv(&["stop", "x", "--pane=%3"]), Some("%7")),
            Route::Core(argv(&["_stop", "x", "--pane=%3"]))
        );
        assert_eq!(
            route(&preamble(), &argv(&["watchdog", "status"]), None),
            Route::Core(argv(&["_watchdog", "status"]))
        );
        // An empty variable is no pane at all.
        assert_eq!(
            route(&preamble(), &argv(&["stop"]), Some("")),
            Route::Core(argv(&["_stop"]))
        );
    }

    #[test]
    fn doctor_carries_no_interpreter_fact_any_more() {
        // ae ships no interpreter, so there is no version of one to relay and
        // doctor has no interpreter row to fill.
        assert_eq!(
            route(&preamble(), &argv(&["doctor", "--refresh"]), None),
            Route::Core(argv(&["doctor", "--refresh"]))
        );
        assert_eq!(
            route(&preamble(), &argv(&["doctor"]), None),
            Route::Core(argv(&["doctor"]))
        );
    }

    #[test]
    fn telegram_carries_the_config_home_and_server_the_core_will_not_read() {
        let mut named = preamble();
        named.server_kind = "name".to_owned();
        named.server_value = "ae-dev".to_owned();
        assert_eq!(
            route(&named, &argv(&["telegram", "start"]), None),
            Route::Core(argv(&[
                "_telegram",
                "start",
                "--config",
                "/h/config",
                "--home",
                "/h",
                "--server-kind",
                "name",
                "--server",
                "ae-dev",
            ]))
        );
    }

    #[test]
    fn archive_takes_preview_and_refuses_everything_else() {
        assert_eq!(
            route(&preamble(), &argv(&["archive", "preview", "x"]), None),
            Route::ArchivePreview(Some("x".to_owned()))
        );
        assert_eq!(
            route(&preamble(), &argv(&["archive", "preview"]), None),
            Route::ArchivePreview(None)
        );
        assert_eq!(
            route(&preamble(), &argv(&["archive"]), None),
            Route::ArchiveUsage
        );
        assert_eq!(
            route(&preamble(), &argv(&["archive", "publish"]), None),
            Route::ArchiveUsage
        );
    }

    #[test]
    fn the_launch_argv_is_the_preamble_then_the_users_words_verbatim() {
        let mut pre = preamble();
        pre.local = Some("/c/.ae/config".into());
        pre.server_kind = "socket".to_owned();
        pre.server_value = "/tmp/s".to_owned();
        pre.caller_server = Some(crate::inventory::ServerId::Selected(
            crate::meta::Selector::Socket("/tmp/caller".into()),
        ));
        pre.inside_tmux = true;
        pre.no_autostart = true;
        assert_eq!(
            pre.launch_argv(&argv(&["--worktree", "feature"])),
            argv(&[
                "_launch",
                "--home",
                "/h",
                "--cwd",
                "/c",
                "--global",
                "/h/config",
                "--local-config",
                "/c/.ae/config",
                "--no-autostart",
                "--server-kind",
                "socket",
                "--server",
                "/tmp/s",
                "--caller-socket",
                "/tmp/caller",
                "--attach",
                "--inside-tmux",
                "--",
                "--worktree",
                "feature",
            ])
        );
    }

    #[test]
    fn the_launch_argv_names_no_core_flag() {
        // `current_exe()` is the answer under this shape, and a flag would only
        // be a second, staler one.
        assert!(!preamble().launch_argv(&[]).iter().any(|w| w == "--core"));
    }

    #[test]
    fn a_direct_child_name_has_no_separator_and_is_not_a_dot() {
        assert!(is_direct_child_name("ok"));
        assert!(!is_direct_child_name(""));
        assert!(!is_direct_child_name("a/b"));
        assert!(!is_direct_child_name(".."));
        assert!(!is_direct_child_name("."));
    }

    #[test]
    fn the_embedded_texts_are_the_ones_the_glue_printed() {
        assert!(HELP.starts_with("ae - agentic engineering: tmux multi-agent workspace\n"));
        assert!(HELP.contains("  ae compact [-f] [--keep-history] [--digest-only] [name]\n"));
        assert!(HELP.contains(
            "  ae init [--yes]        Discover installed harnesses and propose a global config\n"
        ));
        assert!(HELP.ends_with("regenerate existing session helpers.\n"));
        assert!(LIST_HELP.starts_with("Usage: ae list ["));
        assert!(LIST_HELP.contains("--needs-attn"));
        assert!(DEFAULT_CONFIG.starts_with("# ae config — auto-created on first run"));
        assert!(DEFAULT_CONFIG.contains("\n[clients]\n"));
        assert!(DEFAULT_CONFIG.contains("\n[profiles]\n"));
        assert!(DEFAULT_CONFIG.contains("\n[roster]\n"));
        assert!(DEFAULT_CONFIG.contains("\n[workspace]\n"));
        assert!(DEFAULT_CONFIG.contains("\n[prompt]\n"));
        assert!(DEFAULT_CONFIG.contains(
            "gpt6astra = \"codex --yolo -m gpt-6-astra -c model_reasoning_effort=xhigh\"\n"
        ));
        assert!(DEFAULT_CONFIG.contains("colead = gpt6astra\n"));
        assert!(DEFAULT_CONFIG.contains("orchestrator = gpt56luna\n"));
        assert!(DEFAULT_CONFIG.contains("model_reasoning_effort=xhigh"));
        assert!(!DEFAULT_CONFIG.contains("sonnet5"));
        assert!(DEFAULT_CONFIG.ends_with("Prefer TypeScript.\"\n"));
    }
}
