//! The launch-command builders: what a pane is told to run, and the
//! re-runnable script that runs it.
//!
//! Ported from `ae`'s `inject_session_id`, `inject_ae_context`,
//! `initial_prompt_for_cmd`, `build_launch_command`, `launch_rerun_command`,
//! `_emit_launch_script` and `write_launch_script` — the composition half of a
//! spawn (and, when the launch move lands, of a launch). Every decision below
//! is the frozen one; the reasoning is kept at the site that needs it.
//!
//! # Two facts are TRANSPORTED, never re-parsed
//!
//! By the time a command reaches [`build_launch_command`] it carries kilobytes
//! of injected agent-facing prose — which contains the words `resume`,
//! `--session-id` and every flag name the docs mention. So:
//!
//! * the resume id is passed in, never searched for. The frozen bug: ae's own
//!   codex context says "Enable session resume by running:", and a
//!   `codex*resume*` glob matched every FRESH command.
//! * the injection boundary is passed in as the PRE-INJECTION command, which is
//!   a literal prefix of the built one. String surgery may only ever touch that
//!   head; the tail is copied, never edited.
//!
//! # Shell quoting
//!
//! The frozen builder quotes with bash's `printf %q`. This one emits the POSIX
//! single-quoted form instead — same guarantee (every byte literal to the
//! shell that runs it), different bytes for inputs `%q` would leave bare. The
//! consumers are `bash -lc <cmd>` and `[ -e <path> ]` inside a generated
//! script, both of which read a single-quoted word identically.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::launch_cmd::ToolKind;

/// The POSIX single-quoted form of `text` — safe in any shell word position.
#[must_use]
pub fn shell_quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for ch in text.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Escape a value about to be interpolated into a SINGLE-QUOTED shell word
/// that the caller closes itself — the frozen `${ctx//\'/\'\\\'\'}`.
fn single_quote_escape(text: &str) -> String {
    text.replace('\'', "'\\''")
}

/// Does this tool need a post-launch capture handshake — the frozen
/// `tool_kind_supports_launch_id`?
#[must_use]
pub const fn supports_launch_id(tool: ToolKind) -> bool {
    matches!(
        tool,
        ToolKind::Codex | ToolKind::OpenCode | ToolKind::Gemini
    )
}

/// Does this tool take an ae-generated session id at LAUNCH?
///
/// Claude and grok do, so their conversation id is known upfront and resume is
/// exact from the first cycle. Everything else is `pending` until a capture
/// answers — the frozen `resolve_agent_session_id`.
#[must_use]
pub const fn takes_launch_session_id(tool: ToolKind) -> bool {
    matches!(tool, ToolKind::Claude | ToolKind::Grok)
}

/// The absent session id, as the frozen roster spells it.
pub const PENDING: &str = "pending";

/// A fresh RFC 4122 version-4 UUID, lowercase and hyphenated — the frozen
/// `gen_uuid`.
///
/// The frozen helper shells out to `uuidgen` (normalising macOS's uppercase)
/// or reads `/proc/sys/kernel/random/uuid`; neither is available to a core that
/// must run identically on both platforms without a subprocess. The bits come
/// from the same source the request-id suffix uses — `RandomState`, seeded per
/// process by the OS — mixed with the clock, which is the quality this id
/// needs: it NAMES a conversation, and a collision costs a resumed transcript,
/// not a security property.
#[must_use]
pub fn generate_uuid() -> String {
    use std::hash::{BuildHasher, RandomState};
    use std::time::{SystemTime, UNIX_EPOCH};

    let clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() ^ u64::from(since.subsec_nanos()));
    let high = RandomState::new().hash_one(clock);
    let low = RandomState::new().hash_one(high ^ u64::from(std::process::id()));
    // Version 4 in the high nibble of byte 6, variant 10xx in byte 8.
    let high = (high & 0xffff_ffff_ffff_0fff) | 0x0000_0000_0000_4000;
    let low = (low & 0x3fff_ffff_ffff_ffff) | 0x8000_0000_0000_0000;
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        high >> 32,
        (high >> 16) & 0xffff,
        high & 0xffff,
        low >> 48,
        low & 0xffff_ffff_ffff,
    )
}

// ---- flag stripping -------------------------------------------------------

/// Split on runs of spaces, dropping empties — what the frozen strippers'
/// trailing `s/ +/ /g; s/^ //; s/ $//` collapses to.
fn words(cmd: &str) -> Vec<&str> {
    cmd.split(' ').filter(|word| !word.is_empty()).collect()
}

/// Strip `--session-id`, `--resume` and `--continue` — the frozen
/// `strip_session_flags`, whole-token so `--continue-on-error` survives.
///
/// A value-taking flag consumes the next word only when there IS one: the
/// frozen `[= ][^ ]*` needs the `=` or the space to be there at all, so a
/// trailing `--resume` with nothing after it is left alone.
#[must_use]
pub fn strip_session_flags(cmd: &str) -> String {
    let words = words(cmd);
    let mut kept: Vec<&str> = Vec::with_capacity(words.len());
    let mut index = 0;
    while index < words.len() {
        let word = words[index];
        let takes_value = word == "--session-id" || word == "--resume";
        let attached = word.starts_with("--session-id=") || word.starts_with("--resume=");
        if attached || word == "--continue" {
            index += 1;
            continue;
        }
        if takes_value && index + 1 < words.len() {
            index += 2;
            continue;
        }
        kept.push(word);
        index += 1;
    }
    kept.join(" ")
}

/// Strip grok's session surface — the frozen `strip_grok_session_flags`.
///
/// Grok's `-r/--resume` takes an OPTIONAL value, which is the trap the generic
/// stripper cannot handle: eat the next token only when it is not itself a
/// flag, so `--resume --effort high` keeps `--effort`.
#[must_use]
pub fn strip_grok_session_flags(cmd: &str) -> String {
    let words = words(cmd);
    let mut kept: Vec<&str> = Vec::with_capacity(words.len());
    let mut index = 0;
    while index < words.len() {
        let word = words[index];
        // Attached value, equals form or clap's attached short form.
        let attached = word.starts_with("--session-id=")
            || word.starts_with("--resume=")
            || word.starts_with("-s=")
            || word.starts_with("-r=")
            || (word.len() > 2 && (word.starts_with("-s") || word.starts_with("-r")));
        if attached {
            index += 1;
            continue;
        }
        match word {
            "--session-id" | "-s" => {
                index += if index + 1 < words.len() { 2 } else { 1 };
            }
            "--resume" | "-r" => {
                let eats = index + 1 < words.len() && !words[index + 1].starts_with('-');
                index += usize::from(eats) + 1;
            }
            "--continue" | "-c" => index += 1,
            _ => {
                kept.push(word);
                index += 1;
            }
        }
    }
    kept.join(" ")
}

// ---- session id injection -------------------------------------------------

/// Put ae's generated session id on the command — the frozen
/// `inject_session_id`.
///
/// `pending` is no id at all: the flags are stripped and nothing is added, so
/// a capture-tool launch is clean. The tool is read with THE classifier, so an
/// env-prefixed binary or an absolute path is still seen.
#[must_use]
pub fn inject_session_id(cmd: &str, session_id: &str) -> String {
    let session_id = if session_id == PENDING {
        ""
    } else {
        session_id
    };
    let clean = strip_session_flags(cmd);
    if session_id.is_empty() {
        return clean;
    }
    match ToolKind::from_cmd(cmd) {
        // Re-normalised with the grok-complete stripper so a pre-existing
        // -s/-r/-c cannot stack with, or swallow, the launch --session-id.
        ToolKind::Grok => format!(
            "{} --session-id {session_id}",
            strip_grok_session_flags(cmd)
        ),
        ToolKind::Claude => format!("{clean} --session-id {session_id}"),
        _ => clean,
    }
}

// ---- context injection ----------------------------------------------------

/// A command with ae's workspace context injected, plus whatever the caller
/// must be told about a degraded injection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Injected {
    /// The command to launch.
    pub cmd: String,
    /// A warning line for stderr — the opencode arm that could not publish its
    /// context files and launches without them.
    pub warning: Option<String>,
}

/// Inject the rendered context on the channel this tool actually has — the
/// frozen `inject_ae_context`.
///
/// `launch_id` is the capture token already recorded for the slot (empty when
/// the tool needs none); `meta_dir` and `slot` are what codex's `_register-sid`
/// instruction names and where opencode's config files land.
#[must_use]
pub fn inject_ae_context(
    cmd: &str,
    meta_dir: &Path,
    slot: &str,
    ctx: &str,
    launch_id: &str,
) -> Injected {
    let dir = meta_dir.display();
    match ToolKind::from_cmd(cmd) {
        ToolKind::Claude => Injected {
            cmd: format!(
                "{cmd} --append-system-prompt '{}'",
                single_quote_escape(ctx)
            ),
            warning: None,
        },
        ToolKind::Codex => {
            let slot_arg = if slot.is_empty() {
                String::new()
            } else {
                format!(" {slot}")
            };
            // Meta keys are generic (`launch_id.<slot>`); the persisted marker
            // stays tool-specific, because each capture searches a different
            // external store.
            let marker = if launch_id.is_empty() {
                String::new()
            } else {
                format!("\nAE_CODEX_LAUNCH_ID={launch_id}\nAE_CODEX_SLOT={slot}")
            };
            let full = format!(
                "{ctx} --- CRITICAL FIRST TASK: Enable session resume by running: {dir}/_register-sid{slot_arg} — do this NOW before anything else.{marker}"
            );
            Injected {
                cmd: format!(
                    "{cmd} -c developer_instructions='{}'",
                    single_quote_escape(&full)
                ),
                warning: None,
            }
        }
        ToolKind::Gemini => {
            let marker = if launch_id.is_empty() {
                String::new()
            } else {
                format!("\nAE_GEMINI_LAUNCH_ID={launch_id}\nAE_GEMINI_SLOT={slot}")
            };
            let full = format!("{ctx}{marker}{WAIT_SUFFIX}");
            Injected {
                cmd: format!("{cmd} -i '{}'", single_quote_escape(&full)),
                warning: None,
            }
        }
        // grok has NO append-style system-prompt flag: `--system-prompt-override`
        // REPLACES the agent's own prompt, which its tooling depends on. The
        // context rides in as the POSITIONAL [PROMPT] argument instead.
        ToolKind::Grok => {
            let full = format!("{ctx}{WAIT_SUFFIX}");
            Injected {
                cmd: format!("{cmd} '{}'", single_quote_escape(&full)),
                warning: None,
            }
        }
        // No wait-suffix here: "this is context only" exists because gemini and
        // grok receive a USER TURN. This is system-level content and reads
        // oddly as an instruction to itself.
        ToolKind::OpenCode => match opencode_context_files(meta_dir, slot, ctx) {
            Ok(config) => Injected {
                cmd: format!(
                    "env OPENCODE_CONFIG={} {cmd}",
                    shell_quote(&config.display().to_string())
                ),
                warning: None,
            },
            // Launch anyway rather than not at all, and say so: a contextless
            // agent is recoverable, a missing one is not.
            Err(_) => Injected {
                cmd: cmd.to_owned(),
                warning: Some(format!(
                    "ae: could not write opencode context for '{}' — launching without ae context",
                    if slot.is_empty() { "main" } else { slot }
                )),
            },
        },
        ToolKind::Unknown => Injected {
            cmd: cmd.to_owned(),
            warning: None,
        },
    }
}

/// The suffix that keeps a USER-TURN context from being acted on.
const WAIT_SUFFIX: &str =
    " --- IMPORTANT: This is context only. Do NOT act on it. Wait for the user to give you a task.";

/// Publish opencode's context pair and return the config path — the frozen
/// `_opencode_context_files`.
///
/// Both files are published temp + mode + rename at 0600, for the reason the
/// data chokepoint exists: a writer that dies mid-write must leave the previous
/// artifact whole rather than a truncated one in place.
///
/// # Errors
///
/// The file that could not be published, named.
pub fn opencode_context_files(meta_dir: &Path, slot: &str, ctx: &str) -> Result<PathBuf, String> {
    let safe = safe_slot(if slot.is_empty() { "main" } else { slot });
    let ctx_file = meta_dir.join(format!("opencode.{safe}.md"));
    let cfg_file = meta_dir.join(format!("opencode.{safe}.json"));
    publish_data(&ctx_file, format!("{ctx}\n").as_bytes())?;
    let pointer = crate::json::Value::Str(ctx_file.display().to_string()).render();
    publish_data(
        &cfg_file,
        format!("{{\"instructions\":[{pointer}]}}\n").as_bytes(),
    )?;
    Ok(cfg_file)
}

/// The frozen `${slot//[^A-Za-z0-9._-]/_}` — a slot is a filename component.
#[must_use]
pub fn safe_slot(slot: &str) -> String {
    slot.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

/// Publish one non-executable generated artifact atomically at 0600.
fn publish_data(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    publish(dest, bytes, 0o600)
}

/// Publish one generated artifact atomically: write a temp, set its mode
/// there, then rename over the destination.
fn publish(dest: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;

    let temp = dest.with_extension(format!("tmp.{}", std::process::id()));
    let write = std::fs::File::create(&temp).and_then(|mut file| {
        file.write_all(bytes)?;
        file.set_permissions(std::fs::Permissions::from_mode(mode))
    });
    if let Err(why) = write {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("could not write {} ({why})", temp.display()));
    }
    if let Err(why) = std::fs::rename(&temp, dest) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("could not publish {} ({why})", dest.display()));
    }
    Ok(())
}

// ---- the initial user turn ------------------------------------------------

/// The first USER message a tool needs, for tools whose context does not ride
/// a system-prompt channel — the frozen `initial_prompt_for_cmd`.
///
/// Only codex has one: it needs a user turn to act on `developer_instructions`.
/// claude, gemini and grok take their context by flag or argv; opencode's
/// arrives as system-level `instructions`, so there is nothing to paste and
/// nothing to wait for.
#[must_use]
pub const fn initial_prompt_for(tool: ToolKind) -> &'static str {
    match tool {
        ToolKind::Codex => "Go",
        _ => "",
    }
}

// ---- the resume predicate -------------------------------------------------

/// Is `id` usable in a command the PANE will run?
///
/// Conservative on purpose: the value is interpolated into a shell command, so
/// anything outside the charset is treated as "no id" — which costs a
/// conversation, never a pane.
#[must_use]
pub fn id_probeable(id: &str) -> bool {
    !id.is_empty()
        && id != PENDING
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// The part of `cmd` ae composed itself — the frozen `_launch_injected_head`.
///
/// `None` means "cannot classify", and the caller then changes nothing at all.
#[must_use]
pub fn injected_head<'a>(cmd: &'a str, pre: &str) -> Option<&'a str> {
    (!pre.is_empty() && cmd.starts_with(pre)).then(|| &cmd[..pre.len()])
}

/// THE resume predicate — the frozen `_launch_is_resume`.
///
/// One question, one answer, from transported facts only. Two classifiers
/// answering it from different evidence is what shipped: a fresh codex got its
/// prompt inline AND deferred, and acted on the same instruction twice.
#[must_use]
pub fn is_resume(cmd: &str, id: &str, pre: &str) -> bool {
    if !id_probeable(id) {
        return false;
    }
    let Some(head) = injected_head(cmd, pre) else {
        return false;
    };
    // The tool is read off the HEAD, which is ae's own composition — never off
    // the tail, which is prose.
    let token = match ToolKind::from_cmd(head) {
        ToolKind::Codex => format!(" resume {id}"),
        ToolKind::Claude => format!(" --resume {id}"),
        _ => return false,
    };
    head.contains(&token)
}

/// A shell TEST, evaluated in the pane at launch, that answers "is this
/// conversation actually resumable" — or `None` for "no probe".
#[must_use]
pub fn resume_probe(tool: ToolKind, uuid: &str) -> Option<String> {
    if !id_probeable(uuid) {
        return None;
    }
    match tool {
        // Claude keeps transcripts at
        // ~/.claude/projects/<cwd with / turned into ->/<uuid>.jsonl, so the
        // file's existence IS the answer. $PWD is read at run time rather than
        // baked: a stale baked path would choose the fallback forever.
        ToolKind::Claude => Some(format!(
            r#"test -f "$HOME/.claude/projects/$(printf %s "$PWD" | tr / -)/{uuid}.jsonl""#
        )),
        // Codex records under dated directories, so the id is searched for.
        // The pattern is QUOTED in the emitted command: unquoted, the pane's
        // shell globs it against the work dir before find sees it.
        ToolKind::Codex => Some(format!(
            r#"test -n "$(find "$HOME/.codex/sessions" -maxdepth 4 -name "*{uuid}*.jsonl" -print -quit 2>/dev/null)""#
        )),
        _ => None,
    }
}

/// Choose between the resume command and the fallback IN THE PANE — the frozen
/// `_launch_resume_decider`.
///
/// Both branches `exec` a SINGLE command, so whichever runs replaces the shell
/// and `pane_current_command` reports the TOOL. A `A || B` chain keeps bash as
/// the pane process to evaluate the `||`, which silently disables the send
/// path's whole TUI model on every resumed agent (measured).
#[must_use]
pub fn resume_decider(probe: Option<&str>, resume: &str, fallback: &str) -> String {
    match probe {
        None => format!("exec {fallback}"),
        Some(probe) => format!("if {probe}; then exec {resume}; else exec {fallback}; fi"),
    }
}

// ---- the launch command ---------------------------------------------------

/// The command a pane runs to start its agent — the frozen
/// `build_launch_command`.
#[must_use]
pub fn build_launch_command(cmd: &str, prompt: &str, resume_id: &str, pre: &str) -> String {
    match ToolKind::from_cmd(cmd) {
        ToolKind::Codex if is_resume(cmd, resume_id, pre) => {
            // Head only, EXACT token — the injected tail is appended untouched.
            let head = injected_head(cmd, pre).unwrap_or_default();
            let fallback = format!(
                "{}{}",
                head.replacen(&format!(" resume {resume_id}"), "", 1),
                &cmd[head.len()..]
            );
            return resume_decider(
                resume_probe(ToolKind::Codex, resume_id).as_deref(),
                cmd,
                &fallback,
            );
        }
        // opencode's resume rides its own flag surface; nothing is appended.
        ToolKind::OpenCode => return cmd.to_owned(),
        _ => {}
    }
    // Keep Claude Code from detecting nesting when ae runs from inside a claude
    // session, and disable the input-box ghost SUGGESTION: the input-region
    // sensor is content-based and needs "idle == bare ornament" to hold.
    let mut launch_cmd = if ToolKind::from_cmd(cmd) == ToolKind::Claude {
        format!(
            "env -u CLAUDECODE -u CLAUDE_CODE_SESSION CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=0 {cmd}"
        )
    } else {
        cmd.to_owned()
    };
    let suffix = if prompt.is_empty() {
        String::new()
    } else {
        format!(" {}", shell_quote(prompt))
    };
    if is_resume(cmd, resume_id, pre) {
        let fallback = launch_cmd.replacen(&format!(" --resume {resume_id}"), " --continue", 1);
        return resume_decider(
            resume_probe(ToolKind::Claude, resume_id).as_deref(),
            &format!("{launch_cmd}{suffix}"),
            &format!("{fallback}{suffix}"),
        );
    }
    launch_cmd.push_str(&suffix);
    launch_cmd
}

/// The `--resume` form of a launch command, for the script's SECOND run — the
/// frozen `launch_rerun_command`.
///
/// `None` is "no re-run form", which is a refusal rather than a guess: the id
/// may be `pending`, the tool may not take one, or the head may not sit in the
/// built command exactly once. The cost of refusing is a second run colliding
/// loudly on "Session ID already in use", never a wrong conversation.
#[must_use]
pub fn rerun_command(launch_cmd: &str, id: &str, pre: &str) -> Option<String> {
    if !id_probeable(id) || pre.is_empty() {
        return None;
    }
    // The tool is read off ae's OWN composition, which has no prose in it.
    if !matches!(ToolKind::from_cmd(pre), ToolKind::Claude | ToolKind::Grok) {
        return None;
    }
    let token = format!(" --session-id {id}");
    if !pre.contains(&token) {
        return None;
    }
    // THE EDIT HAPPENS INSIDE THE HEAD, then the head is spliced back over its
    // own span — so the flag's position in the built command is never guessed
    // and the injected tail cannot be touched even when it names the same flag.
    let rerun_pre = pre.replacen(&token, &format!(" --resume {id}"), 1);
    // Containment AND uniqueness: a head appearing twice has no unambiguous
    // span to splice.
    let (before, after) = launch_cmd.split_once(pre)?;
    if after.contains(pre) {
        return None;
    }
    Some(format!("{before}{rerun_pre}{after}"))
}

// ---- the launch script ----------------------------------------------------

/// The interpreter a generated launch script names.
///
/// Named explicitly for the reason the helper shebangs are: `#!/usr/bin/env
/// bash` resolves via PATH, and under a macOS login shell that is bash 3.2.
pub const SHELL: &str = "/bin/bash";

/// The body of `launch.<slot>.sh` — the frozen `_emit_launch_script`.
///
/// With a re-run form the script answers "has THIS script already launched its
/// agent?" with a file test, not by parsing an error. Both branches still
/// `exec` a SINGLE command, which is what keeps `pane_current_command`
/// reporting the tool instead of bash.
#[must_use]
pub fn script_body(shell: &str, started: &Path, rerun: Option<&str>, launch_cmd: &str) -> String {
    let started = shell_quote(&started.display().to_string());
    let mut body = format!("#!{shell}\n");
    if let Some(rerun) = rerun {
        let _ = write!(
            body,
            "if [ -e {started} ]; then\n    printf '%s\\n' 'ae: re-run — resuming this agent, not creating a second session.' >&2\n    exec {shell} -lc {}\nfi\n: > {started} 2>/dev/null || true\n",
            shell_quote(rerun)
        );
    }
    let _ = writeln!(body, "exec {shell} -lc {}", shell_quote(launch_cmd));
    body
}

/// Publish `launch.<slot>.sh` and return its path — the frozen
/// `write_launch_script`.
///
/// The marker is cleared AFTER the publish succeeds, and only then: a failed
/// publish leaves the PREVIOUS script in place, and clearing the marker anyway
/// would reclassify that survivor as never-run, so its next run would go back
/// to creating and collide.
///
/// # Errors
///
/// The publication failure, named. Nothing is reported on success but the path.
pub fn write_launch_script(
    meta_dir: &Path,
    slot: &str,
    launch_cmd: &str,
    session_id: &str,
    pre: &str,
) -> Result<PathBuf, String> {
    let safe = safe_slot(slot);
    let script = meta_dir.join(format!("launch.{safe}.sh"));
    let started = meta_dir.join(format!("launch.{safe}.started"));
    let rerun = rerun_command(launch_cmd, session_id, pre);
    let body = script_body(SHELL, &started, rerun.as_deref(), launch_cmd);
    publish(&script, body.as_bytes(), 0o700)?;
    // A rewritten script has never run.
    let _ = std::fs::remove_file(&started);
    Ok(script)
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests read back what the door wrote; the boundary is on product code — see clippy.toml"
)]
mod tests {
    use super::{
        build_launch_command, generate_uuid, id_probeable, initial_prompt_for, inject_ae_context,
        inject_session_id, is_resume, opencode_context_files, rerun_command, script_body,
        shell_quote, strip_grok_session_flags, strip_session_flags, write_launch_script,
    };
    use crate::launch_cmd::ToolKind;
    use std::path::{Path, PathBuf};

    fn scratch(tag: &str) -> PathBuf {
        let dir = PathBuf::from(format!("/tmp/ae-launch-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn shell_quote_makes_every_byte_literal_including_the_quote_itself() {
        assert_eq!(shell_quote("plain"), "'plain'");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a b$c`d\\e"), "'a b$c`d\\e'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn the_session_strippers_take_whole_tokens_and_leave_their_neighbours() {
        assert_eq!(
            strip_session_flags("claude --resume abc --continue --continue-on-error -x"),
            "claude --continue-on-error -x",
            "--continue-on-error is not --continue"
        );
        assert_eq!(
            strip_session_flags("claude --session-id=abc --resume=def tail"),
            "claude tail"
        );
        assert_eq!(
            strip_session_flags("claude --resume"),
            "claude --resume",
            "a value-taking flag with nothing after it is left alone"
        );
        // The grok trap: a bare --resume must not swallow the NEXT flag.
        assert_eq!(
            strip_grok_session_flags("grok --resume --effort high"),
            "grok --effort high"
        );
        assert_eq!(
            strip_grok_session_flags("grok -sUUID -r ID -c --always-approve"),
            "grok --always-approve"
        );
    }

    #[test]
    fn only_claude_and_grok_take_a_launch_time_session_id() {
        let uuid = "3f2a-1";
        assert_eq!(
            inject_session_id("claude --continue", uuid),
            format!("claude --session-id {uuid}")
        );
        assert_eq!(
            inject_session_id("grok -r --effort high", uuid),
            format!("grok --effort high --session-id {uuid}")
        );
        assert_eq!(inject_session_id("codex --yolo", uuid), "codex --yolo");
        assert_eq!(
            inject_session_id("claude --session-id old", super::PENDING),
            "claude",
            "pending is no id at all: the flags go and nothing is added"
        );
        // THE classifier: an env prefix and an absolute path are still seen.
        assert_eq!(
            inject_session_id("env FOO=1 /opt/bin/claude", uuid),
            format!("env FOO=1 /opt/bin/claude --session-id {uuid}")
        );
    }

    #[test]
    fn each_tool_gets_its_own_context_channel() {
        let dir = scratch("ctx");
        let ctx = "WORKSPACE ctx with a ' quote";
        let claude = inject_ae_context("claude", &dir, "spawned.0", ctx, "");
        assert!(
            claude
                .cmd
                .starts_with("claude --append-system-prompt 'WORKSPACE ctx with a '\\''"),
            "{}",
            claude.cmd
        );
        let codex = inject_ae_context("codex", &dir, "spawned.1", ctx, "tok-1");
        assert!(
            codex.cmd.contains("-c developer_instructions='"),
            "{}",
            codex.cmd
        );
        assert!(
            codex.cmd.contains(&format!(
                "Enable session resume by running: {}/_register-sid spawned.1",
                dir.display()
            )),
            "{}",
            codex.cmd
        );
        assert!(
            codex.cmd.contains("AE_CODEX_LAUNCH_ID=tok-1"),
            "the launch token rides the developer instructions"
        );
        let gemini = inject_ae_context("gemini", &dir, "spawned.2", ctx, "tok-2");
        assert!(gemini.cmd.contains(" -i '"), "{}", gemini.cmd);
        assert!(gemini.cmd.contains("AE_GEMINI_LAUNCH_ID=tok-2"));
        assert!(gemini.cmd.contains("This is context only"));
        // grok has no append-style flag: the context is the POSITIONAL prompt.
        let grok = inject_ae_context("grok", &dir, "spawned.3", ctx, "");
        assert!(!grok.cmd.contains("--system-prompt"), "{}", grok.cmd);
        assert!(grok.cmd.contains("This is context only"));
        // opencode gets FILES and an env prefix, not a flag.
        let opencode = inject_ae_context("opencode", &dir, "spawned.4", ctx, "");
        assert!(
            opencode.cmd.starts_with("env OPENCODE_CONFIG='"),
            "{}",
            opencode.cmd
        );
        assert!(opencode.cmd.ends_with(" opencode"), "{}", opencode.cmd);
        let published = std::fs::read_to_string(dir.join("opencode.spawned.4.md")).unwrap();
        assert_eq!(published, format!("{ctx}\n"));
        let config = std::fs::read_to_string(dir.join("opencode.spawned.4.json")).unwrap();
        assert!(config.contains("\"instructions\":["), "{config}");
        assert!(config.contains("opencode.spawned.4.md"), "{config}");
        // An unknown tool has no channel at all.
        assert_eq!(
            inject_ae_context("weirdtool", &dir, "spawned.5", ctx, "").cmd,
            "weirdtool"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_codex_needs_a_first_user_turn() {
        assert_eq!(initial_prompt_for(ToolKind::Codex), "Go");
        for tool in [
            ToolKind::Claude,
            ToolKind::Gemini,
            ToolKind::Grok,
            ToolKind::OpenCode,
            ToolKind::Unknown,
        ] {
            assert_eq!(initial_prompt_for(tool), "", "{tool:?}");
        }
    }

    #[test]
    fn a_fresh_launch_command_is_the_generic_tail_per_tool() {
        // claude: the nesting/ghost env wrapper, and no inline prompt.
        let claude =
            build_launch_command("claude --session-id u1", "", "u1", "claude --session-id u1");
        assert_eq!(
            claude,
            "env -u CLAUDECODE -u CLAUDE_CODE_SESSION CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=0 claude --session-id u1"
        );
        // codex: no resume token in the head, so the generic tail with the
        // inline first message quoted onto it.
        let codex = build_launch_command(
            "codex -c developer_instructions='x'",
            "Go --- task",
            "pending",
            "codex",
        );
        assert_eq!(codex, "codex -c developer_instructions='x' 'Go --- task'");
        // opencode: returned untouched — its context rides OPENCODE_CONFIG and
        // there is nothing to paste.
        assert_eq!(
            build_launch_command(
                "env OPENCODE_CONFIG='/x' opencode",
                "ignored",
                "pending",
                "opencode"
            ),
            "env OPENCODE_CONFIG='/x' opencode"
        );
        // grok: the positional context is already on the command; nothing else.
        assert_eq!(
            build_launch_command(
                "grok --session-id u2 'ctx'",
                "",
                "u2",
                "grok --session-id u2"
            ),
            "grok --session-id u2 'ctx'"
        );
    }

    #[test]
    fn a_resume_is_decided_in_the_pane_and_never_by_a_fallback_chain() {
        let pre = "claude --resume u3";
        let cmd = "claude --resume u3 --append-system-prompt 'ctx mentioning --resume u3'";
        assert!(is_resume(cmd, "u3", pre));
        let built = build_launch_command(cmd, "", "u3", pre);
        assert!(
            built.starts_with("if test -f \"$HOME/.claude/projects/"),
            "{built}"
        );
        assert!(built.contains("; then exec env -u CLAUDECODE"), "{built}");
        assert!(built.contains("; else exec env -u CLAUDECODE"), "{built}");
        assert!(
            built.matches(" --continue").count() == 1,
            "the fallback arm swaps --resume for --continue exactly once: {built}"
        );
        assert!(
            !built.contains("||"),
            "never a chain: bash would stay the pane process"
        );
        // The TAIL's mention of the id is untouched — only the head is edited.
        assert!(
            built.contains("ctx mentioning --resume u3"),
            "the injected prose is copied, never edited: {built}"
        );
        // A pending id is not a resume, whatever the prose says.
        assert!(!is_resume(cmd, "pending", pre));
        assert!(!id_probeable("has space"));
    }

    #[test]
    fn the_rerun_form_exists_only_where_a_launch_time_id_was_transported() {
        let pre = "claude --session-id u4";
        let built = format!("env -u CLAUDECODE {pre} --append-system-prompt 'ctx'");
        let rerun = rerun_command(&built, "u4", pre).expect("a claude re-run form");
        assert_eq!(
            rerun,
            "env -u CLAUDECODE claude --resume u4 --append-system-prompt 'ctx'"
        );
        // codex never gets one: no launch-time id flag exists.
        assert_eq!(
            rerun_command("codex resume u4", "u4", "codex resume u4"),
            None
        );
        // Nor does a pending id, nor a head that is not in the built command.
        assert_eq!(rerun_command(&built, "pending", pre), None);
        assert_eq!(rerun_command(&built, "u4", "grok --session-id u4"), None);
    }

    #[test]
    fn the_launch_script_is_rerunnable_only_when_a_rerun_form_exists() {
        let dir = scratch("script");
        let pre = "claude --session-id u5";
        let built = format!("env -u CLAUDECODE {pre}");
        let script = write_launch_script(&dir, "spawned.0", &built, "u5", pre).unwrap();
        let body = std::fs::read_to_string(&script).unwrap();
        assert!(body.starts_with("#!/bin/bash\n"), "{body}");
        assert!(body.contains("if [ -e '"), "{body}");
        assert!(body.contains("launch.spawned.0.started'"), "{body}");
        assert!(
            body.matches("exec /bin/bash -lc ").count() == 2,
            "both branches exec a SINGLE command: {body}"
        );
        assert!(body.contains("--resume u5"), "{body}");
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&script).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "the script is private and executable");
        }
        // A capture tool has no re-run form, so the script is the one-liner.
        let plain =
            write_launch_script(&dir, "spawned.1", "codex -c x='y'", "pending", "codex").unwrap();
        let plain_body = std::fs::read_to_string(&plain).unwrap();
        assert_eq!(
            plain_body,
            "#!/bin/bash\nexec /bin/bash -lc 'codex -c x='\\''y'\\'''\n"
        );
        // A rewrite clears the marker: a fresh launch must CREATE, not resume.
        let marker = dir.join("launch.spawned.0.started");
        std::fs::write(&marker, "").unwrap();
        write_launch_script(&dir, "spawned.0", &built, "u5", pre).unwrap();
        assert!(
            std::fs::read_to_string(&marker).is_err(),
            "the marker is cleared by a rewrite"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_script_body_names_its_interpreter_and_quotes_every_interpolation() {
        let body = script_body("/bin/bash", Path::new("/s/m.started"), None, "tool 'x'");
        assert_eq!(body, "#!/bin/bash\nexec /bin/bash -lc 'tool '\\''x'\\'''\n");
    }

    #[test]
    fn a_generated_uuid_is_a_well_formed_v4_and_two_of_them_differ() {
        let one = generate_uuid();
        let two = generate_uuid();
        assert_ne!(one, two);
        let parts: Vec<&str> = one.split('-').collect();
        assert_eq!(
            parts.iter().map(|part| part.len()).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12],
            "{one}"
        );
        assert!(
            one.starts_with(|_: char| true)
                && one.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-'),
            "{one}"
        );
        assert!(one.as_bytes()[14] == b'4', "version 4: {one}");
        assert!(
            matches!(one.as_bytes()[19], b'8' | b'9' | b'a' | b'b'),
            "variant: {one}"
        );
        assert!(id_probeable(&one));
    }

    #[test]
    fn an_unpublishable_opencode_context_degrades_the_launch_instead_of_failing_it() {
        let dir = scratch("oc-fail");
        // A DIRECTORY where the context file must go: publication cannot win.
        std::fs::create_dir_all(dir.join("opencode.spawned.0.md")).unwrap();
        assert!(opencode_context_files(&dir, "spawned.0", "ctx").is_err());
        let injected = inject_ae_context("opencode", &dir, "spawned.0", "ctx", "");
        assert_eq!(injected.cmd, "opencode", "launched anyway, contextless");
        assert!(
            injected
                .warning
                .as_deref()
                .is_some_and(|line| line.contains("launching without ae context")),
            "and it says so: {:?}",
            injected.warning
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
