//! The launch-command builders: what a pane is told to run, and the
//! re-runnable script that runs it.
//!
//! The composition half of a spawn or a launch: the session-id injection, the
//! context injection, the initial user turn, the launch command and the
//! re-runnable script. The reasoning is kept at the site that needs it.

use std::path::{Path, PathBuf};

use crate::launch_cmd::ToolKind;
use crate::tool::{CommandForm, ContextChannel, IdStyle, InitialTurn, SessionFlags};

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
/// that the caller closes itself.
fn single_quote_escape(text: &str) -> String {
    text.replace('\'', "'\\''")
}

/// The launch marker shared with capture readers, or no marker when either
/// component is absent.
fn launch_marker_text(prefix: Option<&str>, launch_id: &str, slot: &str) -> String {
    if launch_id.is_empty() {
        return String::new();
    }
    prefix.map_or_else(String::new, |prefix| {
        format!("\nAE_{prefix}_LAUNCH_ID={launch_id}\nAE_{prefix}_SLOT={slot}")
    })
}

/// Does this tool need a post-launch capture handshake?
#[must_use]
pub const fn supports_launch_id(tool: ToolKind) -> bool {
    tool.adapter().capture.is_needed()
}

/// Does this tool take an ae-generated session id at LAUNCH?
#[must_use]
pub const fn takes_launch_session_id(tool: ToolKind) -> bool {
    tool.adapter().launch.takes_session_id()
}

/// The absent session id, as the roster spells it.
pub const PENDING: &str = "pending";

/// A fresh RFC 4122 version-4 UUID, lowercase and hyphenated.
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

/// Split on runs of spaces, dropping empties.
fn words(cmd: &str) -> Vec<&str> {
    cmd.split(' ').filter(|word| !word.is_empty()).collect()
}

/// Strip `--session-id`, `--resume` and `--continue`, whole-token so
/// `--continue-on-error` survives.
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

/// Strip grok's session surface.
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
///
/// **`--` ends the strip.** agy is a Go `flag` program, so `--` terminates ITS
/// parsing too: a word after it is an operand, and `agy -- --continue` asks for
/// the literal operand `--continue`, not for a resume. A stripper that lexed
/// past the delimiter would delete an operand the operator explicitly protected,
/// so everything from `--` onward is copied through untouched.
///
/// What this does NOT fix, said plainly rather than implied: ae APPENDS its own
/// `--conversation <id>` and `-i <ctx>` to the end of the command, so a profile
/// that ends inside an operand list would have them appended as operands too.
/// That is true of every tool ae composes for, not just agy, and a profile whose
/// command ends in `--` is unsupported for all of them.
#[must_use]
pub fn strip_agy_session_flags(cmd: &str) -> String {
    let words = words(cmd);
    let mut kept: Vec<&str> = Vec::with_capacity(words.len());
    let mut index = 0;
    while index < words.len() {
        let word = words[index];
        if word == "--" {
            kept.extend_from_slice(&words[index..]);
            break;
        }
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

/// Strip the session grammar selected by a tool adapter.
pub(crate) fn strip_session_grammar(cmd: &str, grammar: SessionFlags) -> String {
    match grammar {
        SessionFlags::Common => strip_session_flags(cmd),
        SessionFlags::Conversation => strip_agy_session_flags(cmd),
        SessionFlags::ShortAliases => strip_grok_session_flags(cmd),
    }
}

// ---- session id injection -------------------------------------------------

/// Put ae's generated session id on the command.
#[must_use]
pub fn inject_session_id(cmd: &str, session_id: &str) -> String {
    let session_id = if session_id == PENDING {
        ""
    } else {
        session_id
    };
    let adapter = ToolKind::from_cmd(cmd).adapter();
    let clean = strip_session_grammar(cmd, adapter.launch.session_flags);
    if session_id.is_empty() {
        return clean;
    }
    match adapter.launch.id {
        IdStyle::Flag { flag, grammar } => format!(
            "{} {flag} {session_id}",
            strip_session_grammar(cmd, grammar)
        ),
        IdStyle::None => clean,
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
#[must_use]
pub fn inject_ae_context(
    cmd: &str,
    meta_dir: &Path,
    slot: &str,
    ctx: &str,
    launch_id: &str,
) -> Injected {
    let dir = meta_dir.display();
    let adapter = ToolKind::from_cmd(cmd).adapter();
    match adapter.launch.context {
        ContextChannel::SystemPromptFlag(flag) => Injected {
            cmd: format!("{cmd} {flag} '{}'", single_quote_escape(ctx)),
            warning: None,
        },
        ContextChannel::DeveloperInstructions => {
            let slot_arg = if slot.is_empty() {
                String::new()
            } else {
                format!(" {slot}")
            };
            // Meta keys are generic (`launch_id.<slot>`); the persisted marker
            // stays tool-specific, because each capture searches a different
            // external store.
            let marker = launch_marker_text(adapter.launch_marker, launch_id, slot);
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
        ContextChannel::UserTurn { flag } => {
            let marker = launch_marker_text(adapter.launch_marker, launch_id, slot);
            let full = format!("{ctx}{marker}{WAIT_SUFFIX}");
            let turn = single_quote_escape(&full);
            Injected {
                cmd: flag.map_or_else(
                    || format!("{cmd} '{turn}'"),
                    |flag| format!("{cmd} {flag} '{turn}'"),
                ),
                warning: None,
            }
        }
        // No wait-suffix here: "this is context only" exists because gemini and
        // grok receive a USER TURN.
        ContextChannel::ConfigFile => match opencode_context_files(meta_dir, slot, ctx) {
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
        ContextChannel::None => Injected {
            cmd: cmd.to_owned(),
            warning: None,
        },
    }
}

/// The suffix that keeps a USER-TURN context from being acted on.
const WAIT_SUFFIX: &str =
    " --- IMPORTANT: This is context only. Do NOT act on it. Wait for the user to give you a task.";

/// Publish opencode's context pair and return the config path.
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

/// `[^A-Za-z0-9._-]` becomes `_`: a slot is a filename component.
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
/// a system-prompt channel.
///
/// **codex needs a turn, and the turn must be PASSIVE.** Measured against
/// codex-cli 0.153.2 on 2026-09-04, launched with ae's exact argv shape but no
/// positional prompt: after 30s NO rollout exists under
/// `~/.codex/sessions/<day>/` at all, and the TUI header carries no session id
/// either — so neither the launch-token scan nor the header scrape has anything
/// to find, and the seat's `harness_session` would stay `pending` for good. The
/// first USER turn is what makes the rollout exist; that is why this turn is
/// sent, and why it cannot simply be dropped.
///
/// The wording is the other half of the measurement. An imperative with no
/// object — a bare `Go` — makes codex invent work for itself, so the turn names
/// the ONE action it exists to cause, the `_register-sid` handshake, and then
/// tells the agent to wait. It is the user-turn twin of `WAIT_SUFFIX`, which
/// does the same job for the tools whose whole context arrives as a turn.
///
/// The command is spelled out here rather than referred to, because a turn that
/// points at the system prompt is a turn the agent has to go looking for.
#[must_use]
pub fn initial_prompt_for(tool: ToolKind, meta_dir: &Path, slot: &str) -> String {
    if tool.adapter().launch.initial_turn != InitialTurn::RegisterSessionId {
        return String::new();
    }
    let slot_arg = if slot.is_empty() {
        String::new()
    } else {
        format!(" {slot}")
    };
    format!(
        "ae: this is your workspace context, delivered at start. Run {}/_register-sid{slot_arg} once so this session can resume, then WAIT — do not start any work until a task arrives from the human or a peer.",
        meta_dir.display()
    )
}

/// The spawn turn: the passive launch turn, then the brief it is waiting for.
///
/// A spawn's brief rides the SAME turn as the launch prompt for codex, and only
/// for codex — every other tool takes its context on a separate channel and its
/// brief as a second, pasted turn. So the join has to say that the task the
/// launch turn told the agent to wait for is the text right after it; a bare
/// separator leaves the agent holding a "WAIT" and a task at once.
#[must_use]
pub fn initial_turn_with_brief(prompt: &str, brief: &str) -> String {
    if prompt.is_empty() {
        return String::new();
    }
    format!("{prompt} --- That task has arrived, from the peer that spawned you: {brief}")
}

// ---- the resume predicate -------------------------------------------------

/// Is `id` usable in a command the PANE will run?
#[must_use]
pub fn id_probeable(id: &str) -> bool {
    !id.is_empty()
        && id != PENDING
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

// ---- the launch command ---------------------------------------------------

/// The command a pane runs to start its agent.
#[must_use]
pub fn build_launch_command(cmd: &str, prompt: &str) -> String {
    let mut launch_cmd = match ToolKind::from_cmd(cmd).adapter().launch.command {
        // Keep Claude Code from detecting nesting when ae runs from inside a
        // claude session, and disable the input-box ghost SUGGESTION: the
        // content-based input-region sensor needs "idle == bare ornament" to
        // hold.
        CommandForm::SanitizedEnvironment => format!(
            "env -u CLAUDECODE -u CLAUDE_CODE_SESSION CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=0 {cmd}"
        ),
        CommandForm::InlinePrompt => cmd.to_owned(),
        // OpenCode's resume rides its own flag surface and it has no inline
        // first message; nothing is appended to it, ever.
        CommandForm::NoInlinePrompt => return cmd.to_owned(),
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
        initial_turn_with_brief, inject_ae_context, inject_session_id, opencode_context_files,
        shell_quote, strip_agy_session_flags, strip_grok_session_flags, strip_session_flags,
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
        // `--` ends the strip: agy is a Go `flag` program, so a word past the
        // delimiter is an OPERAND and `--continue` there is a literal argument,
        // not a resume. Deleting it would lose what the operator protected.
        assert_eq!(
            strip_agy_session_flags("agy -c -- --continue --conversation ID"),
            "agy -- --continue --conversation ID"
        );
        // And the delimiter does not disarm the stripper ahead of itself.
        assert_eq!(
            strip_agy_session_flags("agy --conversation OLD -- keep --continue"),
            "agy -- keep --continue"
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
        assert_eq!(
            inject_session_id("grok -r --effort high", PENDING),
            "grok -r --effort high",
            "without an id only the common long-form grammar is stripped"
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
    fn only_codex_needs_a_first_user_turn_and_that_turn_is_passive() {
        let dir = PathBuf::from("/meta");
        let turn = initial_prompt_for(ToolKind::Codex, &dir, "spawned.0");
        // The ONE action the turn exists to cause, spelled out rather than
        // referred to — codex creates no rollout until a user turn lands.
        assert!(turn.contains("/meta/_register-sid spawned.0"), "{turn}");
        // And nothing else: the bare `Go` it replaced made codex invent work.
        assert!(
            turn.contains("do not start any work until a task arrives"),
            "{turn}"
        );
        assert!(!turn.starts_with("Go"), "{turn}");
        // An empty slot leaves the command bare rather than trailing a space.
        assert!(
            initial_prompt_for(ToolKind::Codex, &dir, "").contains("/meta/_register-sid once"),
            "no slot, no argument"
        );
        for tool in [
            ToolKind::Claude,
            ToolKind::Gemini,
            ToolKind::Agy,
            ToolKind::Grok,
            ToolKind::OpenCode,
            ToolKind::Unknown,
        ] {
            assert_eq!(initial_prompt_for(tool, &dir, "spawned.0"), "", "{tool:?}");
        }
    }

    #[test]
    fn a_spawn_brief_arrives_as_the_task_the_passive_turn_waits_for() {
        let dir = PathBuf::from("/meta");
        let turn = initial_prompt_for(ToolKind::Codex, &dir, "spawned.1");
        let joined = initial_turn_with_brief(&turn, "review the diff");
        assert!(joined.starts_with(&turn), "{joined}");
        assert!(
            joined.ends_with(
                "That task has arrived, from the peer that spawned you: review the diff"
            ),
            "{joined}"
        );
        // A tool with no first turn has no joined turn either: its brief is
        // pasted separately.
        assert_eq!(initial_turn_with_brief("", "review the diff"), "");
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
            build_launch_command("codex -c developer_instructions='x'", "ae: ctx --- task"),
            "codex -c developer_instructions='x' 'ae: ctx --- task'"
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
        // builder wraps it and stops.
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
