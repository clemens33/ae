//! `_peek`, `_agents` and `_focus`: the three read-mostly pane surfaces the
//! session helpers still needed in bash.
//!
//! Ported from `ae`'s `helper_peek_main`, `helper_agents_main` and
//! `helper_focus_main`. None of them had any logic worth keeping in bash — a
//! resolve, one tmux call, and a `printf` table — and every one of them was the
//! last reason its helper had to `source _lib`. Under the B move the helpers
//! are shims, so these are the entries they exec.
//!
//! `focus` is the ONE surface allowed to select a pane. The launch and delivery
//! paths deliberately do not: `paste-buffer -t` writes to the NAMED pane, and
//! selecting mid-send routes the human's in-flight keystrokes into the target.

use std::io::{self, Write};
use std::path::Path;

use crate::session_tmux::{Op, argv};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::{tracked, transport};

/// The frozen `peek` usage, both lines.
pub const PEEK_USAGE: &str = "Usage: peek <agent-name|pane-id|@session:agent> [lines]\n  Examples: peek codex:reviewer\n           peek @my-feature:claude:lead 50\n";

/// The frozen `focus` usage, both lines.
pub const FOCUS_USAGE: &str = "Usage: focus <agent-name|pane-id|@session:agent>\n  Examples: focus codex:reviewer\n           focus @my-feature:claude:lead\n";

/// The frozen capture ceiling and floor: at most 2000 lines, at least 1.
const MAX_LINES: u32 = 2000;

/// The frozen default window.
const DEFAULT_LINES: u32 = 80;

/// `_peek <dir> <target> [lines]` — the recent output of another agent's pane.
///
/// # Errors
///
/// Only a failure to write `out` or `err`.
pub fn peek(
    dir: &Path,
    tail: &[String],
    own_session: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let [target, rest @ ..] = tail else {
        write!(err, "{PEEK_USAGE}")?;
        return Ok(EXIT_FAILED);
    };
    let lines = match rest.first() {
        None => DEFAULT_LINES,
        Some(word) => {
            // The frozen guard is `^[0-9]+$` — a bare digit run, so a signed or
            // spaced value is a refusal rather than a silently clamped window.
            let Ok(value) = word.parse::<u64>() else {
                writeln!(err, "Error: lines must be a number, got '{word}'")?;
                return Ok(EXIT_FAILED);
            };
            u32::try_from(value.clamp(1, u64::from(MAX_LINES))).unwrap_or(MAX_LINES)
        }
    };
    let (resolved, server) = match tracked::resolve_on(target, own_session, dir) {
        Ok(pair) => pair,
        Err(why) => {
            writeln!(err, "{}", why.message())?;
            return Ok(EXIT_FAILED);
        }
    };
    let (succeeded, stdout) = transport::run_tmux_op(&argv(
        &server,
        &Op::CapturePane {
            pane: &resolved.pane,
            lines,
        },
    ));
    if !succeeded {
        writeln!(
            err,
            "Error: could not capture pane {} of '{target}'",
            resolved.pane
        )?;
        return Ok(EXIT_FAILED);
    }
    out.write_all(stdout.as_bytes())?;
    Ok(0)
}

/// `_agents <dir> [--all]` — the session's panes, or every ae session's.
///
/// # Errors
///
/// Only a failure to write `out` or `err`.
pub fn agents(
    dir: &Path,
    tail: &[String],
    own_session: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    match tail.first().map(String::as_str) {
        None => rows_for(dir, own_session, None, out),
        Some("--all") => {
            writeln!(
                out,
                "{:<20} {:<25} {:<8} PROCESS",
                "SESSION", "AGENT", "PANE"
            )?;
            for name in crate::session_launch::running_ae_sessions(dir) {
                let meta_dir = crate::session_launch::sibling_session_dir(dir, &name);
                rows_for(&meta_dir, &name, Some(&name), out)?;
            }
            Ok(0)
        }
        Some(word) => {
            writeln!(err, "Usage: agents [--all]  (got '{word}')")?;
            Ok(EXIT_USAGE)
        }
    }
}

/// One session's stamped panes. `label` prints the session column; `None` is
/// the single-session table, which does not have one.
fn rows_for(
    dir: &Path,
    session: &str,
    label: Option<&str>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let server = crate::session_launch::recorded_server(dir);
    if label.is_none() {
        writeln!(out, "{:<25} {:<8} PROCESS", "AGENT", "PANE")?;
    }
    for pane in transport::observe_watch_panes(&server, session).unwrap_or_default() {
        let Some(agent) = pane.agent.as_deref().filter(|name| !name.is_empty()) else {
            continue;
        };
        match label {
            Some(name) => writeln!(
                out,
                "{name:<20} {agent:<25} {:<8} {}",
                pane.pane_id, pane.current_command
            )?,
            None => writeln!(
                out,
                "{agent:<25} {:<8} {}",
                pane.pane_id, pane.current_command
            )?,
        }
    }
    Ok(0)
}

/// `_focus <dir> <target>` — switch the client to another agent's pane.
///
/// # Errors
///
/// Only a failure to write `err`.
pub fn focus(
    dir: &Path,
    tail: &[String],
    own_session: &str,
    now: crate::time::Timestamp,
    err: &mut impl Write,
) -> io::Result<u8> {
    let [target, ..] = tail else {
        write!(err, "{FOCUS_USAGE}")?;
        return Ok(EXIT_FAILED);
    };
    let (resolved, server) = match tracked::resolve_on(target, own_session, dir) {
        Ok(pair) => pair,
        Err(why) => {
            writeln!(err, "{}", why.message())?;
            return Ok(EXIT_FAILED);
        }
    };
    // The WINDOW first: a worker lives in its own window, and `select-pane`
    // alone does not change which window is viewed. Tolerated on failure,
    // exactly as the frozen `2>/dev/null || true`.
    let _ = transport::run_tmux_op(&argv(
        &server,
        &Op::SelectWindow {
            pane: &resolved.pane,
        },
    ));
    let (selected, _) = transport::run_tmux_op(&argv(
        &server,
        &Op::SelectPane {
            pane: &resolved.pane,
        },
    ));
    if !selected {
        writeln!(err, "Error: could not focus pane {}", resolved.pane)?;
        return Ok(EXIT_FAILED);
    }
    let shown = if resolved.agent.is_empty() {
        target.as_str()
    } else {
        resolved.agent.as_str()
    };
    let _ = crate::state::emit(
        dir,
        &crate::tracked::event_line(&crate::tracked::EventFields {
            ts: now,
            actor: "human",
            action: "focus",
            target: shown,
            reference: "",
            actor_slot: "",
            actor_session: "",
            target_slot: &resolved.slot,
            target_session: &resolved.session,
            summary: "",
            body_file: "",
        }),
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peek_refuses_a_non_numeric_window() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let tail = ["lead".to_owned(), "many".to_owned()];
        let code = peek(Path::new("/nope"), &tail, "s", &mut out, &mut err).unwrap();
        assert_eq!(code, EXIT_FAILED);
        assert_eq!(
            String::from_utf8_lossy(&err),
            "Error: lines must be a number, got 'many'\n"
        );
    }

    #[test]
    fn peek_with_no_target_prints_its_usage() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = peek(Path::new("/nope"), &[], "s", &mut out, &mut err).unwrap();
        assert_eq!(code, EXIT_FAILED);
        assert_eq!(String::from_utf8_lossy(&err), PEEK_USAGE);
    }

    #[test]
    fn focus_with_no_target_prints_its_usage() {
        let mut err = Vec::new();
        let code = focus(
            Path::new("/nope"),
            &[],
            "s",
            crate::time::Timestamp::now(),
            &mut err,
        )
        .unwrap();
        assert_eq!(code, EXIT_FAILED);
        assert_eq!(String::from_utf8_lossy(&err), FOCUS_USAGE);
    }
}
