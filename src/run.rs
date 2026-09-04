//! `_run <session-dir> <slot>` — what a pane runs, and the whole of it.
//!
//! Until slice Z2 a pane ran `launch.<slot>.sh`, a generated bash file that
//! baked one composed command line, tested a marker file to choose between a
//! create form and a re-run form, and handed the winner to `bash -lc`. The
//! script is gone. The pane's command is `<core> _run <session-dir> <slot>`,
//! and the core does in Rust what the script did in bash: read the seat, build
//! the tool command with the SAME builders the launch always used, decide
//! create-vs-resume, then `exec` the tool.
//!
//! # Why `exec`, and why one argv
//!
//! `execve` replaces this process, so `pane_current_command` reports the TOOL
//! rather than bash or ae — which is the fact the send path's whole TUI model
//! rests on. The frozen script had the same requirement and met it by making
//! both branches of its shell `if` end in `exec`; here there is no shell to
//! replace, and no `||` chain that would leave one behind.
//!
//! # What moved with it
//!
//! The two decisions the script left to the pane's shell are the core's now:
//! the create-vs-resume marker test, and the resume PROBE (does this recorded
//! conversation actually exist on disk). Both were shell tests written into the
//! script; both are ordinary filesystem questions.
//!
//! # What is read fresh, and why that is an improvement
//!
//! The command is composed at RUN time from the session meta and the config,
//! not baked at launch time. A seat whose harness session id was captured
//! minutes after the launch — codex, gemini and opencode all are — used to
//! carry a script that could never mention it. Here the id is simply read.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::launch;
use crate::launch_cmd::ToolKind;

/// Which form of the tool command a run builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// No start marker: this seat has no conversation yet.
    Create,
    /// The marker is there: this seat has run before.
    Resume,
}

impl Mode {
    /// The word `--print` reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Resume => "resume",
        }
    }
}

/// Everything an `exec` needs: the environment deltas peeled off the command's
/// `env` prefix, and the argv itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Create or resume.
    pub mode: Mode,
    /// The tool the seat launches.
    pub tool: ToolKind,
    /// Names the `env -u` prefix removes.
    pub unset: Vec<String>,
    /// Assignments the `env` prefix makes, in order.
    pub set: Vec<(String, String)>,
    /// The tool and its arguments — never empty.
    pub argv: Vec<String>,
}

impl Plan {
    /// The one JSON line `--print` emits.
    #[must_use]
    pub fn render(&self) -> String {
        use crate::json::Value;
        Value::Obj(vec![
            ("mode".to_owned(), Value::Str(self.mode.as_str().to_owned())),
            ("tool".to_owned(), Value::Str(self.tool.as_str().to_owned())),
            (
                "env_unset".to_owned(),
                Value::Arr(self.unset.iter().cloned().map(Value::Str).collect()),
            ),
            (
                "env_set".to_owned(),
                Value::Obj(
                    self.set
                        .iter()
                        .map(|(key, value)| (key.clone(), Value::Str(value.clone())))
                        .collect(),
                ),
            ),
            (
                "argv".to_owned(),
                Value::Arr(self.argv.iter().cloned().map(Value::Str).collect()),
            ),
        ])
        .render()
    }
}

/// Forget whatever a previous occupant of `slot` left behind: the start marker
/// and the recorded first message.
///
/// Both are keyed by SLOT, and a spawned slot number is reused after a retire —
/// so a seat that inherited them would resume a conversation belonging to the
/// agent it replaces. An absent artifact is success: absence is the state being
/// established.
///
/// # Errors
///
/// A removal that failed for any reason but absence.
pub fn clear_slot(dir: &Path, slot: &str) -> std::io::Result<()> {
    for path in [started_marker(dir, slot), prompt_file(dir, slot)] {
        match std::fs::remove_file(&path) {
            Err(why) if why.kind() != std::io::ErrorKind::NotFound => return Err(why),
            _ => {}
        }
    }
    Ok(())
}

/// Record the first user message this seat's tool is to be launched with.
///
/// Only a spawn writes one, and only for a tool whose brief must ride the
/// launch command itself; [`build`] reads it back in the create branch.
///
/// # Errors
///
/// The publication failure, named.
pub fn publish_prompt(dir: &Path, slot: &str, text: &str) -> Result<(), String> {
    launch::publish_data(&prompt_file(dir, slot), text.as_bytes())
}

/// The line a pane runs — the core, this entry, the session and the seat.
///
/// It is pasted into the pane's own shell, so every operand is shell-quoted;
/// what the shell then execs is the core, once, with nothing after it to
/// interpret. A human who arrow-ups this line gets the same create-once,
/// resume-later semantics the generated script gave them, because the decision
/// is the marker rather than anything baked into the words.
#[must_use]
pub fn pane_command(core: &Path, dir: &Path, slot: &str) -> String {
    format!(
        "{} {} {} {}",
        launch::shell_quote(&core.display().to_string()),
        crate::cli::RUN,
        launch::shell_quote(&dir.display().to_string()),
        launch::shell_quote(slot)
    )
}

/// The exit code for a `_run` that could not build or start its agent.
const EXIT_FAILED: u8 = 1;

/// The marker whose presence means "this seat has already been launched once".
#[must_use]
pub fn started_marker(dir: &Path, slot: &str) -> PathBuf {
    dir.join(format!("launch.{}.started", launch::safe_slot(slot)))
}

/// The optional first user message a spawn recorded for this seat.
///
/// Only codex has one: it needs a user turn to act on its
/// `developer_instructions`, and a spawn's brief rides in it. Every other tool
/// takes its brief through the readiness-gated paste the spawn does itself.
#[must_use]
pub fn prompt_file(dir: &Path, slot: &str) -> PathBuf {
    dir.join(format!("launch.{}.prompt", launch::safe_slot(slot)))
}

/// `_run <session-dir> <slot>` — build this seat's command and become it.
///
/// Returns only on failure, or when `print` asked for the plan instead of the
/// agent: a successful run is an `execve`.
///
/// # Errors
///
/// Propagates a write failure on the caller's streams.
pub fn run(
    dir: &Path,
    slot: &str,
    print: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let plan = match build(dir, slot) {
        Ok(plan) => plan,
        Err(why) => {
            writeln!(err, "ae: {why}")?;
            err.flush()?;
            return Ok(EXIT_FAILED);
        }
    };
    if print {
        writeln!(out, "{}", plan.render())?;
        out.flush()?;
        return Ok(0);
    }
    // BEFORE the exec, because after it there is no "after". Best effort, as
    // the frozen script's `: > marker 2>/dev/null || true` was: a marker that
    // cannot be written costs a second run that creates instead of resuming,
    // and refusing to start the agent at all costs more.
    let marker = started_marker(dir, slot);
    if plan.mode == Mode::Create {
        let _ = std::fs::File::create(&marker);
    }
    let why = exec(&plan);
    // Reached only because the exec did NOT happen, so this seat has not been
    // launched after all: take the marker back rather than leave a seat that
    // never started looking like one that did.
    if plan.mode == Mode::Create {
        let _ = std::fs::remove_file(&marker);
    }
    writeln!(err, "ae: could not start {} ({why})", plan.argv[0])?;
    err.flush()?;
    Ok(EXIT_FAILED)
}

/// Replace this process with the planned command. Returns only on failure.
fn exec(plan: &Plan) -> std::io::Error {
    use std::os::unix::process::CommandExt as _;

    #[allow(
        clippy::disallowed_types,
        reason = "the pane's own exec: _run BECOMES the tool, which is what keeps pane_current_command reporting it"
    )]
    let mut command = std::process::Command::new(&plan.argv[0]);
    command.args(&plan.argv[1..]);
    for name in &plan.unset {
        command.env_remove(name);
    }
    for (name, value) in &plan.set {
        command.env(name, value);
    }
    command.exec()
}

/// Compose this seat's plan from the session's own state.
///
/// # Errors
///
/// The reason, ready to print after `ae: ` — a missing session, an unknown
/// seat, a profile this machine does not configure, or a command line the
/// direct exec cannot run.
pub fn build(dir: &Path, slot: &str) -> Result<Plan, String> {
    let seat = read_seat(dir, slot)?;
    let mode = if crate::lifecycle::path_exists(&started_marker(dir, slot)) {
        Mode::Resume
    } else {
        Mode::Create
    };
    let ctx = crate::render::context_document(
        dir,
        &seat.session,
        &seat.work_dir,
        slot,
        &seat.config_files,
    );
    let composed = compose(dir, slot, &seat, &ctx, mode);
    let words = crate::words::split(&composed, &env_lookup)?;
    let (prefix, argv) = peel_env(words)?;
    Ok(Plan {
        mode,
        tool: seat.tool,
        unset: prefix.unset,
        set: prefix.assign,
        argv,
    })
}

/// The composed shell command line — the frozen builders, in the frozen order.
///
/// INJECT FIRST, DECIDE SECOND. Every builder reads the tool off the first word
/// of the command, so a resume form that was wrapped before it was injected
/// classified as `Unknown` and launched with no context and no identity — the
/// glue-cut-2 finding, and the reason both branches are injected as plain tool
/// commands here before either is chosen.
fn compose(dir: &Path, slot: &str, seat: &Seat, ctx: &str, mode: Mode) -> String {
    if mode == Mode::Resume {
        let (resume_form, fallback_form) =
            resume_forms(&seat.command, seat.tool, &seat.harness_session);
        let inj_r = launch::inject_ae_context(&resume_form, dir, slot, ctx, &seat.launch_id);
        let inj_f = launch::inject_ae_context(&fallback_form, dir, slot, ctx, &seat.launch_id);
        // A resume carries no inline first message: codex's is delivered once
        // its UI returns, and no other tool has one.
        let resume_cmd = launch::build_launch_command(&inj_r.cmd, "");
        let fallback_cmd = launch::build_launch_command(&inj_f.cmd, "");
        return if resumable(seat.tool, &seat.harness_session) {
            resume_cmd
        } else {
            fallback_cmd
        };
    }
    let pre = launch::inject_session_id(&seat.command, &seat.harness_session);
    let injected = launch::inject_ae_context(&pre, dir, slot, ctx, &seat.launch_id);
    let prompt =
        read_prompt(dir, slot).unwrap_or_else(|| launch::initial_prompt_for(seat.tool).to_owned());
    launch::build_launch_command(&injected.cmd, &prompt)
}

/// Is the recorded conversation actually there to resume?
///
/// The frozen `resume_probe` wrote this as a shell test the pane evaluated;
/// with no pane shell left it is the same question asked directly.
///
/// **Only claude and codex have a probe, so only they can answer YES.** grok,
/// gemini and opencode take the fallback form on every resume — `--continue`,
/// `--resume latest`, `--continue` — however good the recorded id is. That is
/// the frozen decider's behaviour carried over unchanged (its `None` arm
/// emitted the FALLBACK), and it is carried over deliberately rather than
/// improved here, because this slice ports the decision and does not re-rule
/// it. It DISAGREES with the capability table in AGENTS.md, which says grok's
/// resume is UUID-scoped from the first cycle and gemini uses exact resume once
/// captured; the disagreement predates slice Z2 and belongs to whoever rules on
/// it. Flagged here so the next reader sees the conflict rather than the code.
fn resumable(tool: ToolKind, id: &str) -> bool {
    if !launch::id_probeable(id) {
        return false;
    }
    let Some(home) = env_lookup("HOME") else {
        return false;
    };
    match tool {
        // Claude keeps a transcript per conversation at a path derived from the
        // working directory, so the file's existence IS the answer.
        ToolKind::Claude => {
            let Some(cwd) = working_dir() else {
                return false;
            };
            let key: String = cwd
                .display()
                .to_string()
                .chars()
                .map(|ch| if ch == '/' { '-' } else { ch })
                .collect();
            crate::lifecycle::path_exists(
                &Path::new(&home)
                    .join(".claude/projects")
                    .join(key)
                    .join(format!("{id}.jsonl")),
            )
        }
        // Codex records under dated directories, so the id is searched for.
        ToolKind::Codex => contains_id(&Path::new(&home).join(".codex/sessions"), id, 4),
        _ => false,
    }
}

/// Is there a `*<id>*.jsonl` anywhere within `depth` levels of `root`?
///
/// The frozen `find -maxdepth 4 -name '*<uuid>*.jsonl' -print -quit`, walked
/// directly. Unreadable entries are absence, not an error: the question is
/// "can this be resumed", and an answer of "no" costs a fresh conversation.
fn contains_id(root: &Path, id: &str, depth: usize) -> bool {
    if depth == 0 {
        return false;
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the resume probe reads the TOOL's own session store, which is the only evidence that a conversation exists"
    )]
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let log = Path::new(&name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"));
        if log && name.contains(id) {
            return true;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_dir())
            && contains_id(&entry.path(), id, depth - 1)
        {
            return true;
        }
    }
    false
}

/// The resume form of a profile's command, and the form to use when the
/// conversation cannot be found — the frozen `resume_cmd_from_cmd`.
#[must_use]
pub fn resume_forms(cmd: &str, tool: ToolKind, session_id: &str) -> (String, String) {
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

/// An `env` prefix, split into what it removes and what it assigns.
#[derive(Debug, Default, PartialEq, Eq)]
struct EnvPrefix {
    /// Names `env -u` removes.
    unset: Vec<String>,
    /// `NAME=value` assignments, in order.
    assign: Vec<(String, String)>,
}

/// Peel the `env` prefix off a split command line.
///
/// `env -u NAME`, `-i` and `NAME=value` are the three forms ae itself composes
/// (claude's nesting guard, opencode's config pointer) and the three
/// [`crate::launch_cmd`] already skips when it classifies a command, so the two
/// modules agree about where the binary word starts.
///
/// # Errors
///
/// A command line that is nothing but an env prefix — there is no binary to
/// exec, and guessing one is how a mis-shaped command reaches a live pane.
fn peel_env(words: Vec<String>) -> Result<(EnvPrefix, Vec<String>), String> {
    let mut prefix = EnvPrefix::default();
    let mut rest = words.into_iter().peekable();
    while rest.peek().is_some_and(|word| word == "env") {
        rest.next();
        while let Some(word) = rest.peek() {
            if word == "-i" {
                rest.next();
            } else if word == "-u" {
                rest.next();
                match rest.next() {
                    Some(name) => prefix.unset.push(name),
                    None => return Err("an `env -u` with no name in a launch command".to_owned()),
                }
            } else if word == "--" {
                rest.next();
                break;
            } else if let Some((name, value)) = word.split_once('=')
                && !name.is_empty()
            {
                let pair = (name.to_owned(), value.to_owned());
                rest.next();
                prefix.assign.push(pair);
            } else {
                break;
            }
        }
    }
    let argv: Vec<String> = rest.collect();
    if argv.is_empty() {
        return Err("a launch command with no binary to run".to_owned());
    }
    Ok((prefix, argv))
}

/// One seat, read back out of the session's own state.
struct Seat {
    session: String,
    work_dir: String,
    config_files: Vec<PathBuf>,
    command: String,
    tool: ToolKind,
    harness_session: String,
    launch_id: String,
}

/// Read the seat `slot` names, refusing anything that is not launchable.
fn read_seat(dir: &Path, slot: &str) -> Result<Seat, String> {
    if !crate::lifecycle::dir_exists(dir) {
        return Err(format!("no session state at {}", dir.display()));
    }
    let bytes = crate::meta::read_bytes(dir)
        .map_err(|why| format!("could not read the session meta ({why})"))?;
    let value = |key: &str| crate::lifecycle::meta_value(&bytes, key);
    let name = value(&format!("seat.{slot}"));
    if name.is_empty() {
        return Err(format!("no seat '{slot}' in {}", dir.display()));
    }
    let profile = value(&format!("profile.{slot}"));
    if profile.is_empty() {
        return Err(format!("seat '{slot}' has no profile recorded"));
    }
    let origin = value("origin");
    let mut config_files: Vec<PathBuf> = Vec::new();
    let global = value("config");
    if !global.is_empty() {
        config_files.push(PathBuf::from(&global));
    }
    let local = Path::new(if origin.is_empty() { "." } else { &origin })
        .join(".ae")
        .join("config");
    config_files.push(local.clone());
    let cfg = crate::config::read_identity(
        (!global.is_empty()).then(|| Path::new(&global)),
        crate::lifecycle::path_exists(&local).then_some(local.as_path()),
    )
    .map_err(|why| why.to_string())?;
    let Some(command) = cfg.profile(&profile).filter(|cmd| !cmd.trim().is_empty()) else {
        return Err(format!(
            "profile '{profile}' is not configured on this machine — '{name}' cannot be launched"
        ));
    };
    Ok(Seat {
        session: value("session"),
        work_dir: value("work_dir"),
        config_files,
        tool: ToolKind::from_cmd(command),
        command: command.to_owned(),
        harness_session: value(&format!("harness_session.{slot}")),
        launch_id: value(&format!("launch_id.{slot}")),
    })
}

/// The first user message a spawn recorded for this seat, if it recorded one.
fn read_prompt(dir: &Path, slot: &str) -> Option<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the spawn's own recorded first message, published beside the session meta"
    )]
    let text = std::fs::read_to_string(prompt_file(dir, slot));
    text.ok().filter(|body| !body.is_empty())
}

/// One environment variable of the process this run inherits.
fn env_lookup(name: &str) -> Option<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the pane's own environment is what `bash -lc` expanded a profile command against"
    )]
    let value = std::env::var_os(name);
    value.map(|value| value.to_string_lossy().into_owned())
}

/// The pane's working directory — what claude derives its transcript path from.
fn working_dir() -> Option<PathBuf> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the resume probe's `$PWD`, read at run time exactly as the frozen shell test read it"
    )]
    let cwd = std::env::current_dir();
    cwd.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_env_prefix_becomes_environment_deltas_and_the_tool_becomes_argv() {
        let words = crate::words::split(
            "env -u CLAUDECODE -u CLAUDE_CODE_SESSION CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=0 claude --session-id x",
            &|_| None,
        )
        .expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert_eq!(prefix.unset, ["CLAUDECODE", "CLAUDE_CODE_SESSION"]);
        assert_eq!(
            prefix.assign,
            [(
                "CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION".to_owned(),
                "0".to_owned()
            )]
        );
        assert_eq!(argv, ["claude", "--session-id", "x"]);
    }

    #[test]
    fn a_command_line_that_is_only_a_prefix_has_no_binary_to_exec() {
        let words = crate::words::split("env -u A", &|_| None).expect("splits");
        assert!(peel_env(words).is_err());
    }

    #[test]
    fn a_plain_command_keeps_its_whole_argv_and_touches_no_environment() {
        let words = crate::words::split("codex --yolo", &|_| None).expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert_eq!(prefix, EnvPrefix::default());
        assert_eq!(argv, ["codex", "--yolo"]);
    }

    #[test]
    fn the_printed_plan_is_one_decodable_json_line() {
        let plan = Plan {
            mode: Mode::Resume,
            tool: ToolKind::Claude,
            unset: vec!["CLAUDECODE".to_owned()],
            set: vec![("K".to_owned(), "0".to_owned())],
            argv: vec!["claude".to_owned(), "a\nb".to_owned()],
        };
        let line = plan.render();
        assert!(!line.contains('\n'), "{line}");
        assert!(line.contains(r#""mode":"resume""#), "{line}");
        assert!(line.contains(r#""tool":"claude""#), "{line}");
        assert!(line.contains(r#""argv":["claude","a\nb"]"#), "{line}");
    }

    #[test]
    fn a_tool_ae_cannot_probe_is_not_resumable_evidence() {
        assert!(!resumable(ToolKind::Gemini, "3f2a-1"));
        assert!(!resumable(ToolKind::Claude, "pending"));
        assert!(!resumable(ToolKind::Claude, ""));
    }
}
