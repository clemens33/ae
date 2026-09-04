//! The product's ENTRY: the ambient facts an invocation carries, and the
//! dispatch that used to be `ae-glue`'s `case` statement.

use std::path::PathBuf;

/// The config `ae` writes on a first run, verbatim from the glue's
/// `DEFAULT_CONFIG` heredoc.
pub const DEFAULT_CONFIG: &str = r##"# ae config — auto-created on first run, yours to edit. Also mirrored in the repo as
# config.sample. INI-style: [section] headers, key = value, "#" starts a comment.
# New sessions read this whole file. Stopping + resuming an existing session (ae stop <name>;
# ae <name>) relaunches its preserved agents with their CURRENT command + [prompt] — but the
# roster (main/workers), layout, and watchdog stay pinned in session meta, so edits to those
# take effect for NEW sessions only. (ae doctor --refresh regenerates the on-disk session
# helpers + workspace.md after you upgrade ae; it changes neither running agents nor config.)

[profiles]
# Register any CLI tool as a PROFILE: profile = "the shell command that launches it". A
# profile is a reusable launch recipe, not an identity: an agent IS its NAME, bound to a
# profile in [roster] below. The command must be ONE simple command (env assignments plus
# one argv; no ; | & # redirections or $(…) outside quotes) — ae refuses anything else.
# Model-named aliases: the prefix names the model + major, so the running model is legible at
# a glance. STRICT pins (never --model best) — a model-named alias must not silently run a
# different model; that would make the name lie and blind the model-drift alarm. Exact IDs
# beat family aliases where they would drift: --model opus moved to Opus 5 the day it shipped.
# opus5 = everyday builder tier (xhigh is a deliberate opt-up; Opus 5 defaults to high).
opus5 = "claude --permission-mode bypassPermissions --model claude-opus-5 --effort xhigh"
# fable5 = frontier lead tier (default lead seat; pairs with a cross-model colead as its equal peer).
fable5 = "claude --permission-mode bypassPermissions --model fable --effort xhigh"
sonnet5 = "claude --permission-mode bypassPermissions --model sonnet --effort low"
gpt56sol = "codex --yolo -m gpt-5.6-sol -c model_reasoning_effort=xhigh"
gpt56terra = "codex --yolo -m gpt-5.6-terra -c model_reasoning_effort=xhigh"
gpt56luna = "codex -m gpt-5.6-luna -c model_reasoning_effort=low -a never"
opencode = "opencode"

[roster]
# The agents promised to launch: name = profile. The NAME is the identity of the agent — it is
# addressed as <name> in the session and <session>:<name> across sessions; the profile is
# metadata (`ae list` shows it). Every seat in main/workers below must be bound here — ae
# refuses the launch otherwise and lists every violation. A name bound here but not seated
# is legal: `ae <session> use <name>` starts it as main instead. Spawn on demand with:
# spawn <name> --using <profile> [prompt].
lead = fable5
colead = gpt56sol

[workspace]
# main = the standing main seat (a [roster] NAME) — under lead-pair a technical lifecycle
# anchor, not a rank. workers = comma-separated [roster] names launched at start.
# layout = vertical | horizontal | lead-pair. watchdog = true nudges stale/idle agents.
# The leads delegate by rule (see docs/reference/delegation.md — spawn workers on demand).
# Standing seats are the JUDGMENT PAIR only: under lead-pair the FIRST worker (worker.0)
# is the COLEAD seat — an EQUAL leadership peer of the lead (interchangeable, same level,
# sharing the leads window 0:leads); main stays the technical lifecycle anchor (compact
# handover), which is infrastructure, not seniority. Builders and reviewers are NOT
# standing seats: either peer spawns them per slice (spawn builder --using opus5 / spawn
# reviewer --using gpt56sol) and retires its own spawns when the work is verified — every spawn
# ends in a retire. This keeps idle panes at zero, makes the retire contract do real
# work, and still keeps judgment (the pair) separate from review (a spawned seat) on
# every slice.
main = lead
workers = colead
layout = lead-pair
watchdog = true

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
  ae                     Start or reattach default session (local)
  ae <name>              Start or reattach a named session
  ae <name> use <name>   Start session with a specific agent as main
  ae --local [name]      Start session in current directory (default)
  ae --copy [name]       Start session with full copy (includes untracked files)
  ae --worktree [name]   Start session with git worktree (tracked files only)
  ae <new-name> --from <archive-uuid>
                         Start a NEW session that explicitly continues an archived one
                         (the main agent is told to read that archive's digest first)
  ae list [--running|--all|--stopped|--needs-attn|--active] [--json]
                         List sessions (running by default). 'ae list --help' has the
                         full filter set and what --json carries
  ae next [--attach]     Name the top session needing attention (--attach jumps to it)
  ae doctor [--refresh [name|all]]
                         Check local environment and optionally refresh existing session helpers
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
  ae version             Show version
  ae help                Show this help

Modes: --local (default), --copy (full cp -a), --worktree (git worktree).
Sessions persist across reboots. Agents with session support resume conversations; others start fresh.
When inside an ae session, stop/end/compact work without specifying the name.

Config: ~/.ae/config (per-project override: .ae/config in project dir)
Session helpers, in every session dir: send, ask, review, reply, requests, state,
  mark-done, goal, memo, say, peek (peak), agents, focus, interrupt, spawn, retire.
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

/// `ae orchestrator` / `ae hub` — CUT, and the arm is the refusal.
pub const RETIRED_ORCHESTRATOR: &str = "Error: 'ae orchestrator' was retired.\nRun it as an ordinary session against its own config:\n  cd ~/.ae/orchestrator && CONFIG_FILE=$PWD/orchestrator.config ae --local orchestrator\n";

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
    /// The global config file, when the wrapper selected one.
    pub global: Option<PathBuf>,
    /// The origin-local `.ae/config`, when there is one.
    pub local: Option<PathBuf>,
    /// `socket`, `name`, `ambiguous` or empty — the resolved server's kind.
    pub server_kind: String,
    /// The resolved server's value.
    pub server_value: String,
    /// Whether the caller is genuinely inside a tmux pane (attach vs switch).
    pub inside_tmux: bool,
    /// Whether to attach once the session is up.
    pub attach: bool,
    /// The operator's `AE_NO_AUTOSTART=1`: start NEITHER companion.
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

    /// The typed server pair, or nothing when the wrapper resolved neither
    /// half.
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
    /// Everything else, including an EMPTY argv: create or resume a session.
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
/// assert_eq!(route(&preamble, &[], None), Route::Launch(Vec::new()));
/// assert!(matches!(route(&preamble, &["status".to_owned()], None), Route::Retired(_)));
/// ```
#[must_use]
pub fn route(preamble: &Preamble, argv: &[String], pane: Option<&str>) -> Route {
    let tail = || argv[1..].to_vec();
    match argv.first().map(String::as_str) {
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
        Some("compact") => Route::Core(with_head(crate::cli::COMPACT, &tail())),
        Some("archive") => match argv.get(1).map(String::as_str) {
            Some("preview") => Route::ArchivePreview(argv.get(2).cloned()),
            _ => Route::ArchiveUsage,
        },
        Some("doctor") => Route::Core(with_head(crate::cli::DOCTOR, &tail())),
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
        Some("orchestrator" | "hub") => Route::Retired(RETIRED_ORCHESTRATOR),
        Some("transfer") => Route::Retired(RETIRED_TRANSFER),
        Some("help" | "-h" | "--help") => Route::Help,
        Some("version" | "--version" | "-V") => Route::Version,
        _ => Route::Launch(argv.to_vec()),
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

/// The session name a launch argv NAMES, or empty when it derives one.
///
/// ADVISORY, and deliberately not a grammar: the core owns the launch grammar
/// and every refusal it carries, so this scan accepts anything and answers with
/// the last positional exactly as that grammar's own fallback arm does. `use`
/// and `--from` consume the word after them — an agent name and an archive
/// uuid, neither of which is a session name.
///
/// ```
/// use ae::entry::session_hint;
/// let argv: Vec<String> = ["--worktree", "use", "lead", "feature"]
///     .iter()
///     .map(|w| (*w).to_owned())
///     .collect();
/// assert_eq!(session_hint(&argv), "feature");
/// assert_eq!(session_hint(&[]), "");
/// ```
#[must_use]
pub fn session_hint(argv: &[String]) -> String {
    let mut name = String::new();
    let mut rest = argv;
    while let [word, after @ ..] = rest {
        rest = after;
        match word.as_str() {
            "use" | "--from" => {
                if let [_consumed, next @ ..] = rest {
                    rest = next;
                }
            }
            flag if flag.starts_with("--") => {}
            positional => positional.clone_into(&mut name),
        }
    }
    name
}

/// Whether `name` could be a DIRECT CHILD of the sessions root, by pure string
/// structure — the belt to the grammar, before anything on disk is touched.
#[must_use]
pub fn is_direct_child_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && name != "." && name != ".."
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_CONFIG, HELP, LIST_HELP, Preamble, Route, is_direct_child_name, route, session_hint,
    };

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
    fn an_empty_argv_launches_rather_than_printing_help() {
        assert_eq!(route(&preamble(), &[], None), Route::Launch(Vec::new()));
    }

    #[test]
    fn the_cut_words_refuse_instead_of_becoming_session_names() {
        for word in ["status", "orchestrator", "hub", "transfer"] {
            let Route::Retired(text) = route(&preamble(), &argv(&[word]), None) else {
                panic!("'{word}' must refuse, not launch");
            };
            assert!(text.starts_with("Error: "), "{text}");
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
        // `--bash-major` went with the bash: ae ships no interpreter, so there
        // is no version of one to relay and doctor has no bash row to fill.
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
    fn the_hint_is_the_last_positional_and_skips_the_two_operand_words() {
        assert_eq!(session_hint(&argv(&["feature"])), "feature");
        assert_eq!(session_hint(&argv(&["use", "lead"])), "");
        assert_eq!(session_hint(&argv(&["--from", "uuid", "child"])), "child");
        assert_eq!(session_hint(&argv(&["--worktree"])), "");
        assert_eq!(session_hint(&argv(&["a", "b"])), "b");
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
        assert!(HELP.ends_with("regenerate existing session helpers.\n"));
        assert!(LIST_HELP.starts_with("Usage: ae list ["));
        assert!(LIST_HELP.contains("--needs-attn"));
        assert!(DEFAULT_CONFIG.starts_with("# ae config — auto-created on first run"));
        assert!(DEFAULT_CONFIG.contains("\n[profiles]\n"));
        assert!(DEFAULT_CONFIG.contains("\n[roster]\n"));
        assert!(DEFAULT_CONFIG.contains("\n[workspace]\n"));
        assert!(DEFAULT_CONFIG.contains("\n[prompt]\n"));
        assert!(DEFAULT_CONFIG.ends_with("Prefer TypeScript.\"\n"));
    }
}
