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
    /// Does the prefix start from an EMPTY environment (`env -i`)?
    ///
    /// Peeling `-i` without honouring it was a silent inheritance: the pane's
    /// whole environment reached a tool the operator had asked to start clean
    /// (colead Z2 BLOCKER-1).
    pub clear: bool,
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
            ("env_clear".to_owned(), Value::Bool(self.clear)),
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

/// What a resuming run says before it becomes its tool.
pub const RESUMING: &str = "ae: re-run — resuming this agent, not creating a second session.";

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
    // BEFORE the exec, because after it there is no "after" — and REFUSING when
    // it cannot be written, which the frozen script's
    // `: > marker 2>/dev/null || true` did not. The marker is the whole
    // create-once discriminator: a run that starts the agent without it leaves
    // a seat that a re-run creates a SECOND time, and for the upfront-UUID
    // tools that second create collides on `--session-id` and the pane dies.
    // Costing one launch is the cheap half of that trade (colead Z2 BLOCKER-2).
    let marker = started_marker(dir, slot);
    if plan.mode == Mode::Create {
        if let Err(why) = publish_marker(&marker) {
            writeln!(
                err,
                "ae: could not record the start marker {} ({why}) — refusing to launch, because a re-run of this pane would create a second conversation",
                marker.display()
            )?;
            err.flush()?;
            return Ok(EXIT_FAILED);
        }
    } else {
        // The frozen script said this on its re-run branch, and it is worth
        // saying on every resume: a human who arrow-upped the pane's command
        // gets an answer to "did that just start a second conversation?", and a
        // resumed pane says why it is not empty. It survives on screen only
        // until the tool draws over it.
        writeln!(err, "{RESUMING}")?;
        err.flush()?;
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

/// Create the start marker DURABLY, or say why it could not be.
///
/// Write, `fsync`, rename — the shape `launch::publish` froze, plus the file
/// sync generated data does not need and this artifact does: the marker's whole
/// content is its EXISTENCE, so bytes still in the page cache are a seat that
/// re-creates instead of resuming. The failure this guard exists for is the
/// ordinary one — an unwritable session directory, a read-only or full
/// filesystem — and every step above reports it by name.
///
/// What it does NOT claim: the rename's own directory entry is not synced, so
/// this is durability against a failed write, not against a power cut between
/// the rename and the `exec`.
///
/// # Errors
///
/// The path that could not be written and the reason, ready to print.
fn publish_marker(marker: &Path) -> Result<(), String> {
    let temp = PathBuf::from(format!("{}.tmp.{}", marker.display(), std::process::id()));
    let write = std::fs::File::create(&temp).and_then(|file| file.sync_all());
    if let Err(why) = write {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("{} — {why}", temp.display()));
    }
    if let Err(why) = std::fs::rename(&temp, marker) {
        let _ = std::fs::remove_file(&temp);
        return Err(why.to_string());
    }
    Ok(())
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
    // `env -i` FIRST, so the ordered unsets and sets that follow it are applied
    // to the empty environment the operator asked for rather than to the pane's.
    if plan.clear {
        command.env_clear();
    }
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
        clear: prefix.clear,
        unset: prefix.unset,
        set: prefix.assign,
        argv,
    })
}

/// The composed shell command line — the frozen builders, in the frozen order.
///
/// A tool command is injected as a PLAIN TOOL COMMAND, never as something with
/// a shell construct in front of it: every builder reads the tool off the first
/// word, so a resume form that was wrapped before it was injected classified as
/// `Unknown` and launched with no context and no identity (the glue-cut-2
/// finding). Nothing wraps anything here any more, which is what lets the
/// resume arm be chosen before it is injected rather than after.
fn compose(dir: &Path, slot: &str, seat: &Seat, ctx: &str, mode: Mode) -> String {
    if mode == Mode::Resume {
        let (resume_form, fallback_form) =
            resume_forms(&seat.command, seat.tool, &seat.harness_session);
        // DECIDE, THEN INJECT — which is the frozen order read forward, not a
        // departure from it. The frozen path had to inject BOTH forms because
        // the decider was a shell `if` that carried both arms into the pane;
        // injecting after the wrap was the bug it was written against (a
        // pre-wrapped resume command classified as `Unknown` and launched with
        // no context and no identity). With the decision made here there is one
        // arm, so injecting it once is the same rule with the second arm gone —
        // and opencode's context pair is published once rather than twice.
        let form = if resumable(seat.tool, &seat.harness_session) {
            resume_form
        } else {
            fallback_form
        };
        let injected = launch::inject_ae_context(&form, dir, slot, ctx, &seat.launch_id);
        // A resume carries no inline first message: codex's is delivered once
        // its UI returns, and no other tool has one.
        return launch::build_launch_command(&injected.cmd, "");
    }
    let pre = launch::inject_session_id(&seat.command, &seat.harness_session);
    let injected = launch::inject_ae_context(&pre, dir, slot, ctx, &seat.launch_id);
    let prompt =
        read_prompt(dir, slot).unwrap_or_else(|| launch::initial_prompt_for(seat.tool).to_owned());
    launch::build_launch_command(&injected.cmd, &prompt)
}

/// Should this seat be resumed with the id its meta records?
///
/// TWO QUESTIONS, and only one of them is a probe. The first is whether there
/// is an id at all: a seat with none — a capture tool whose id never arrived,
/// or a `pending` placeholder — has nothing to resume BY, and the fallback form
/// is what a tool offers for exactly that (`--continue`, `--resume latest`).
/// The second is whether the recorded conversation still exists, and it can
/// only be asked where the tool leaves evidence on disk: claude writes a
/// transcript per conversation, codex writes a dated session log. Where that
/// evidence exists, a missing file means the id names a conversation that is
/// gone and the fallback is the right form.
///
/// **A tool with NO probe answers YES.** The absence of a probe is not evidence
/// of absence — it is ae having no way to look — and the recorded id is still
/// this seat's own conversation. grok, gemini and opencode therefore resume
/// with `--resume <id>`, `--resume <id>` and `--session <id>` whenever meta
/// holds one, which is what their rows in AGENTS.md's capability table have
/// always said. The frozen decider read the other way (its `None` arm emitted
/// the FALLBACK), so those three never resumed by id however good the id was;
/// that was a defect, ruled and fixed here rather than ported.
fn resumable(tool: ToolKind, id: &str) -> bool {
    if !launch::id_probeable(id) {
        return false;
    }
    match tool {
        // Claude keeps a transcript per conversation at a path derived from the
        // working directory, so the file's existence IS the answer.
        ToolKind::Claude => {
            let (Some(home), Some(cwd)) = (env_lookup("HOME"), working_dir()) else {
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
        ToolKind::Codex => {
            let Some(home) = env_lookup("HOME") else {
                return false;
            };
            contains_id(&Path::new(&home).join(".codex/sessions"), id, 4)
        }
        _ => true,
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

/// An `env` prefix, split into what it clears, removes and assigns.
#[derive(Debug, Default, PartialEq, Eq)]
struct EnvPrefix {
    /// `env -i`: start from an empty environment.
    clear: bool,
    /// Names `env -u` removes.
    unset: Vec<String>,
    /// `NAME=value` assignments, in order.
    assign: Vec<(String, String)>,
}

/// Peel the environment prefix off a split command line.
///
/// TWO forms, and the shell applies them in this order: the bare leading
/// `NAME=value` assignments a shell reads before the command word, and then the
/// `env` command with its `-i`, `-u NAME` and `NAME=value` operands. Both are
/// what ae itself composes (claude's nesting guard, opencode's config pointer),
/// and the vocabulary is EXACTLY [`crate::launch_cmd`]'s `launch_binary` —
/// deliberately, down to the spellings it does not know. `env --` and
/// `env --ignore-environment` are not peeled here BECAUSE they are not peeled
/// there: a word one module treats as an operand and the other as the binary is
/// a command classified as one tool and `exec`ed as another. Widen both or
/// neither.
///
/// The bare form was missing, and its absence was not a degraded launch but a
/// wrong one: `A=1 codex --yolo` classified as codex, planned as codex, and
/// then `exec`ed a binary literally named `A=1` (colead Z2 BLOCKER-1). `-i` was
/// consumed and dropped on the floor, which is the same defect pointed the
/// other way — the environment the operator asked to clear was inherited whole.
///
/// # Errors
///
/// A command line that is nothing but an environment prefix — there is no
/// binary to exec, and guessing one is how a mis-shaped command reaches a live
/// pane.
fn peel_env(words: Vec<String>) -> Result<(EnvPrefix, Vec<String>), String> {
    let mut prefix = EnvPrefix::default();
    let mut rest = words.into_iter().peekable();
    loop {
        // A shell's own prefix: assignments up to the command word.
        while rest
            .peek()
            .is_some_and(|word| crate::launch_cmd::is_assignment(word))
        {
            let Some(word) = rest.next() else { break };
            let Some((name, value)) = word.split_once('=') else {
                break;
            };
            prefix.assign.push((name.to_owned(), value.to_owned()));
        }
        if rest.peek().is_none_or(|word| word != "env") {
            break;
        }
        rest.next();
        // `env`'s own operands. An unrecognised one ENDS the peel: it is the
        // command word, and a prefix that guessed at it would exec something
        // the operator did not write.
        while let Some(word) = rest.peek() {
            match word.as_str() {
                "-i" => {
                    prefix.clear = true;
                    rest.next();
                }
                "-u" => {
                    rest.next();
                    match rest.next() {
                        Some(name) => prefix.unset.push(name),
                        None => {
                            return Err("an `env -u` with no name in a launch command".to_owned());
                        }
                    }
                }
                word if crate::launch_cmd::is_assignment(word) => {
                    let Some(word) = rest.next() else { break };
                    if let Some((name, value)) = word.split_once('=') {
                        prefix.assign.push((name.to_owned(), value.to_owned()));
                    }
                }
                _ => break,
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
    // The profile is read FRESH here, so the launch-time validation of it is a
    // fact about a file that may since have changed. Re-ask the SAME validator
    // rather than trust the older answer: without this a profile edited after
    // its session started reached the `exec` unvalidated, and a construct the
    // validator refuses — brace expansion, a word-initial comment — ran with
    // whatever literal meaning this lexer happened to give it (colead Z2
    // BLOCKER-3).
    if let Err(why) = crate::launch_cmd::lex_simple_command(command) {
        return Err(format!(
            "profile '{profile}' is not one simple command — {why} — '{name}' cannot be launched"
        ));
    }
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
            clear: false,
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
    fn a_bare_leading_assignment_is_an_environment_delta_not_the_binary() {
        // Colead Z2 BLOCKER-1: `A=1 codex --yolo` classified as codex and then
        // `exec`ed a binary literally named `A=1`.
        let words = crate::words::split("A=1 B=2 codex --yolo", &|_| None).expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert!(!prefix.clear);
        assert_eq!(
            prefix.assign,
            [
                ("A".to_owned(), "1".to_owned()),
                ("B".to_owned(), "2".to_owned())
            ]
        );
        assert_eq!(argv, ["codex", "--yolo"]);
        // …and the two forms compose, in the order a shell applies them.
        let words =
            crate::words::split("A=1 env -u KEEPOUT B=2 claude", &|_| None).expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert_eq!(prefix.unset, ["KEEPOUT"]);
        assert_eq!(
            prefix.assign,
            [
                ("A".to_owned(), "1".to_owned()),
                ("B".to_owned(), "2".to_owned())
            ]
        );
        assert_eq!(argv, ["claude"]);
        // The env vocabulary is the CLASSIFIER's, down to what it does not
        // know: `--` is a command word to `launch_binary`, so it is one here.
        let words = crate::words::split("env -- claude", &|_| None).expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert_eq!(prefix.assign, []);
        assert_eq!(argv, ["--", "claude"]);
    }

    #[test]
    fn an_env_dash_i_clears_the_environment_instead_of_being_consumed() {
        // Colead Z2 BLOCKER-1, the other half: `-i` was peeled and dropped, so
        // the pane's whole environment reached a tool asked to start clean.
        let words = crate::words::split("env -i claude", &|_| None).expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert!(prefix.clear);
        assert_eq!(argv, ["claude"]);
        let words = crate::words::split("claude", &|_| None).expect("splits");
        let (prefix, _) = peel_env(words).expect("peels");
        assert!(!prefix.clear, "a plain command clears nothing");
    }

    #[test]
    fn the_binary_this_peel_leaves_is_the_one_the_classifier_named() {
        // The classifier decides `agent_bin` and the tool kind; this peel
        // decides what is `exec`ed. A command classified as one binary and run
        // as another is the B1 defect in its general form, so the invariant is
        // pinned over every prefix shape rather than over the one that shipped.
        for cmd in [
            "claude",
            "/usr/bin/claude --flag",
            "A=1 codex --yolo",
            "A=1 B=2 codex",
            "env -u CLAUDECODE claude",
            "env -i claude",
            "env -i -u A B=2 claude",
            "A=1 env -u B C=3 claude",
            "env -- claude",
            "env --ignore-environment claude",
            "--flag=x claude",
        ] {
            let named = crate::launch_cmd::lex_simple_command(cmd)
                .map(|parsed| parsed.binary)
                .unwrap_or_default();
            let words = crate::words::split(cmd, &|_| None).expect("splits");
            let (_, argv) = peel_env(words).expect("peels");
            let run = argv[0].rsplit('/').next().unwrap_or(&argv[0]).to_owned();
            assert_eq!(named, run, "{cmd:?}: classified one way, exec'ed another");
        }
    }

    #[test]
    fn the_printed_plan_reports_whether_the_environment_is_cleared() {
        let plan = Plan {
            mode: Mode::Create,
            tool: ToolKind::Unknown,
            clear: true,
            unset: Vec::new(),
            set: Vec::new(),
            argv: vec!["/usr/bin/env".to_owned()],
        };
        assert!(
            plan.render().contains(r#""env_clear":true"#),
            "{}",
            plan.render()
        );
    }

    #[test]
    fn a_start_marker_that_cannot_be_published_is_an_error_not_a_shrug() {
        // Colead Z2 BLOCKER-2: the marker is the create-once discriminator, so
        // a failure to write it has to reach the caller rather than be shrugged
        // off into a seat that re-creates on its next run.
        use std::os::unix::fs::PermissionsExt as _;

        let dir = PathBuf::from(format!("/tmp/aemarker.{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a fixture dir");
        assert!(publish_marker(&started_marker(&dir, "main")).is_ok());
        std::fs::remove_file(started_marker(&dir, "main")).expect("the marker is there");

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555))
            .expect("a read-only fixture dir");
        let refused = publish_marker(&started_marker(&dir, "main"));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
            .expect("restored so the fixture can be removed");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(refused.is_err(), "an unwritable session directory refuses");
    }

    #[test]
    fn a_missing_id_is_the_only_thing_that_makes_a_seat_unresumable_without_a_probe() {
        // No probe exists for these three, and that is not evidence of
        // absence: the recorded id is still this seat's own conversation.
        for tool in [ToolKind::Gemini, ToolKind::Grok, ToolKind::OpenCode] {
            assert!(resumable(tool, "3f2a-1"), "{}", tool.as_str());
            assert!(!resumable(tool, "pending"), "{}", tool.as_str());
            assert!(!resumable(tool, ""), "{}", tool.as_str());
        }
        // A tool ae CAN probe still has to pass it, and a `pending` id never
        // reaches the probe at all.
        assert!(!resumable(ToolKind::Claude, "pending"));
        assert!(!resumable(ToolKind::Codex, ""));
    }
}
