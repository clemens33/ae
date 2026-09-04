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
        ToolKind::Codex | ToolKind::OpenCode | ToolKind::Gemini | ToolKind::Agy
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

/// Strip agy's session surface: `--conversation <id>`, `--continue` and `-c`.
///
/// agy resumes by a flag NEITHER generic stripper knows — `--conversation`, not
/// `--resume` — and its `-c` is the short spelling of `--continue`, so the
/// generic stripper would leave an operator-pinned conversation standing on a
/// FRESH launch and stack a second `--conversation` on a resume. Measured
/// against `agy --help` (1.1.25, 2026-09-04): those three are the whole of it,
/// there is no `-s`/`-r`, and `--conversation` always takes a value.
///
/// `-i`/`--prompt-interactive` is deliberately NOT stripped: it is the context
/// channel, not a session flag, and agy's flag parser takes the LAST spelling —
/// which is ae's, because ae appends. An operator's own initial prompt loses to
/// the workspace context, which is the outcome the injection wants.
#[must_use]
pub fn strip_agy_session_flags(cmd: &str) -> String {
    let words = words(cmd);
    let mut kept: Vec<&str> = Vec::with_capacity(words.len());
    let mut index = 0;
    while index < words.len() {
        let word = words[index];
        if word.starts_with("--conversation=") {
            index += 1;
            continue;
        }
        match word {
            // The flag GOES whether or not it had a value, which is grok's
            // stripper's rule rather than the generic one's. The generic
            // stripper leaves a trailing `--resume` standing because its frozen
            // regex needed the separator to be there at all; here a trailing
            // `--conversation` that survived would swallow the `--conversation`
            // ae is about to append and resume a conversation named
            // `--conversation`.
            "--conversation" => {
                index += usize::from(index + 1 < words.len()) + 1;
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
    let clean = if ToolKind::from_cmd(cmd) == ToolKind::Agy {
        // agy's session surface is spelled differently, so the generic
        // stripper leaves it standing; a fresh launch that inherited an
        // operator's `--conversation` would resume someone else's transcript.
        strip_agy_session_flags(cmd)
    } else {
        strip_session_flags(cmd)
    };
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
        // agy has NO append-style system-prompt flag either — `agy --help`
        // (1.1.25, measured 2026-09-04) lists none at all — so the context
        // rides `-i/--prompt-interactive` as a USER TURN, gemini-shaped down to
        // the wait suffix. The marker keeps ITS OWN spelling because it is what
        // the agy capture greps for, in a different store: the token is written
        // into the conversation's own SQLite file, and a shared name would let
        // a gemini seat's token match an agy seat's conversation.
        ToolKind::Agy => {
            let marker = if launch_id.is_empty() {
                String::new()
            } else {
                format!("\nAE_AGY_LAUNCH_ID={launch_id}\nAE_AGY_SLOT={slot}")
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
pub(crate) fn publish_data(dest: &Path, bytes: &[u8]) -> Result<(), String> {
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

// ---- the launch command ---------------------------------------------------

/// The command a pane runs to start its agent — the frozen
/// `build_launch_command`, minus the shell it no longer has.
///
/// The frozen builder could return a shell `if … then exec … else exec … fi`:
/// the resume PROBE was a shell test, so the choice between a resume form and
/// its fallback had to be made in the pane. Slice Z2 moved that decision into
/// [`crate::run`], where it is two filesystem questions, so this builder now
/// returns ONE command in every case — and both callers hand it a form that has
/// already been chosen.
#[must_use]
pub fn build_launch_command(cmd: &str, prompt: &str) -> String {
    // opencode's resume rides its own flag surface and it has no inline first
    // message; nothing is appended to it, ever.
    if ToolKind::from_cmd(cmd) == ToolKind::OpenCode {
        return cmd.to_owned();
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
    if !prompt.is_empty() {
        launch_cmd.push(' ');
        launch_cmd.push_str(&shell_quote(prompt));
    }
    launch_cmd
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests read back what the door wrote; the boundary is on product code — see clippy.toml"
)]
mod tests {
    use super::{
        PENDING, build_launch_command, generate_uuid, id_probeable, initial_prompt_for,
        inject_ae_context, inject_session_id, opencode_context_files, shell_quote,
        strip_agy_session_flags, strip_grok_session_flags, strip_session_flags,
    };
    use crate::launch_cmd::ToolKind;
    use std::path::PathBuf;

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
        // The agy trap: its resume flag is spelled `--conversation`, which
        // neither stripper above knows, and its `-c` is `--continue`.
        assert_eq!(
            strip_agy_session_flags("agy --conversation ID -c --dangerously-skip-permissions"),
            "agy --dangerously-skip-permissions"
        );
        assert_eq!(
            strip_agy_session_flags("agy --conversation=ID --continue --effort high"),
            "agy --effort high"
        );
        assert_eq!(
            strip_agy_session_flags("agy --conversation"),
            "agy",
            "a trailing --conversation GOES: left standing it would swallow the one ae appends"
        );
        // The context channel is NOT a session flag: ae appends its own `-i`
        // and agy takes the last, so the operator's initial prompt survives the
        // strip and simply loses to the workspace context.
        assert_eq!(
            strip_agy_session_flags("agy -i 'hello' --mode plan"),
            "agy -i 'hello' --mode plan"
        );
        // A fresh agy launch is stripped by the agy-aware stripper, so an
        // operator's pinned conversation cannot survive into it.
        assert_eq!(
            inject_session_id("agy --conversation ID -c --flag", PENDING),
            "agy --flag"
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
            ToolKind::Agy,
            ToolKind::Grok,
            ToolKind::OpenCode,
            ToolKind::Unknown,
        ] {
            assert_eq!(initial_prompt_for(tool), "", "{tool:?}");
        }
    }

    #[test]
    fn a_launch_command_is_the_generic_tail_per_tool() {
        // claude: the nesting/ghost env wrapper, and no inline prompt.
        assert_eq!(
            build_launch_command("claude --session-id u1", ""),
            "env -u CLAUDECODE -u CLAUDE_CODE_SESSION CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=0 claude --session-id u1"
        );
        // codex: the generic tail with the inline first message quoted onto it.
        assert_eq!(
            build_launch_command("codex -c developer_instructions='x'", "Go --- task"),
            "codex -c developer_instructions='x' 'Go --- task'"
        );
        // opencode: returned untouched — its context rides OPENCODE_CONFIG and
        // there is nothing to paste.
        assert_eq!(
            build_launch_command("env OPENCODE_CONFIG='/x' opencode", "ignored"),
            "env OPENCODE_CONFIG='/x' opencode"
        );
        // grok: the positional context is already on the command; nothing else.
        assert_eq!(
            build_launch_command("grok --session-id u2 'ctx'", ""),
            "grok --session-id u2 'ctx'"
        );
    }

    #[test]
    fn a_resume_form_is_wrapped_exactly_as_a_fresh_one_and_never_chained() {
        // The decider is gone: a resume form arrives ALREADY CHOSEN, so the
        // builder wraps it and stops. Anything else would put a shell operator
        // in a command line that no shell will ever read.
        let built = build_launch_command(
            "claude --resume u3 --append-system-prompt 'ctx mentioning --resume u3'",
            "",
        );
        assert_eq!(
            built,
            "env -u CLAUDECODE -u CLAUDE_CODE_SESSION CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=0 claude --resume u3 --append-system-prompt 'ctx mentioning --resume u3'"
        );
        for shell_word in ["if ", "; then", "; else", "||", "&&"] {
            assert!(!built.contains(shell_word), "{shell_word} in {built}");
        }
        assert!(!id_probeable("has space"));
        assert!(!id_probeable("pending"));
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
