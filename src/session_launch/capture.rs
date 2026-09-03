//! Post-launch session-id capture for the tools with no launch-time id flag.
//!
//! Ported from `ae`'s `start_capture_session_id` / `capture_session_id`
//! (`ae:1934`-`1952`) and the codex arm of `capture_codex_session_id`. Codex,
//! opencode and gemini all learn their conversation id only after they start,
//! so ae asks each of them a different way; codex is asked by TELLING it — its
//! `developer_instructions` run the `_register-sid` helper, which writes
//! `codex.<slot>.sid` next to the session — and the capture is the poll that
//! collects the answer.
//!
//! Runs on ITS OWN THREAD, never on the launch's: the frozen path backgrounds
//! it with `&` for the same reason, so a tool that takes half a minute to print
//! its id does not delay the attach. OS threads rather than an async runtime —
//! one bounded loop per capture tool does not justify a scheduler.
//!
//! # What is NOT ported here
//!
//! The codex launch-token and CWD scans of `~/.codex/sessions/**/*.jsonl`, the
//! opencode `session list` / db query, and the gemini chat-history scan. All
//! three need the caller's HOME, which the core is not handed, and all three
//! are fallbacks BEHIND the handshake this module does implement. A slot they
//! would have rescued stays `pending`, which is a state ae already renders and
//! a later resume re-attempts.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::inventory::ServerId;
use crate::launch_cmd::ToolKind;

/// How many times the sid file is polled — the frozen `for _attempt in 1..6`.
const POLLS: u32 = 6;

/// The pause between polls — the frozen `sleep 5`.
const POLL: Duration = Duration::from_secs(5);

/// One agent whose id must be captured after it starts.
#[derive(Debug, Clone)]
pub(crate) struct Target {
    /// The seat's slot — the roster key the captured id is written under.
    pub(crate) slot: String,
    /// Which harness it is.
    pub(crate) tool: ToolKind,
    /// The pane, for the TUI fallback.
    pub(crate) pane: String,
    /// The launch token, where one was minted.
    pub(crate) launch_id: String,
}

/// A capture argv minted ONLY by [`argv`], so the detached process door cannot
/// be handed an arbitrary command line.
pub struct CaptureArgv(Vec<String>);

impl CaptureArgv {
    /// The argv for the door to spawn.
    pub(crate) fn as_args(&self) -> &[String] {
        &self.0
    }
}

/// The argv that captures one target: `_capture-sid <dir> <slot> <pane>`.
fn argv(dir: &Path, target: &Target) -> CaptureArgv {
    CaptureArgv(vec![
        crate::cli::CAPTURE_SID.to_owned(),
        dir.display().to_string(),
        target.slot.clone(),
        target.pane.clone(),
    ])
}

/// Start one DETACHED capture per target that needs one.
///
/// A child process, not a thread: the launch returns as soon as the session is
/// up — `--no-attach` returns immediately — and a thread would die with it,
/// which is exactly why the frozen path backgrounded a subshell with `&`. A
/// capture that never answers leaves the slot `pending`, which is what
/// `pending` means.
pub(crate) fn start(dir: &Path, targets: &[Target]) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    for target in targets {
        if target.tool != ToolKind::Codex {
            // opencode and gemini capture is not ported in this move — see the
            // module docs. Their slots stay `pending`.
            continue;
        }
        let _ = crate::transport::spawn_detached(&exe, &argv(dir, target));
    }
}

/// `_capture-sid <dir> <slot> <pane>` — the detached child's whole job.
///
/// Answers 0 whatever happens: nothing reads its status, and a capture that
/// found no id is not a failure of the session it was launched for.
pub fn run(dir: &Path, slot: &str, pane: &str, server: &ServerId) -> u8 {
    let target = Target {
        slot: slot.to_owned(),
        tool: ToolKind::Codex,
        pane: pane.to_owned(),
        launch_id: String::new(),
    };
    if let Some(id) = capture_codex(dir, server, &target) {
        register(dir, slot, &id);
    }
    0
}

/// The path codex's own `_register-sid` handshake writes to.
fn sid_file(dir: &Path, slot: &str) -> PathBuf {
    dir.join(format!("codex.{slot}.sid"))
}

/// Poll for the self-registered id, then fall back to scraping the TUI.
fn capture_codex(dir: &Path, server: &ServerId, target: &Target) -> Option<String> {
    let file = sid_file(dir, &target.slot);
    for _ in 0..POLLS {
        std::thread::sleep(POLL);
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: reads the id codex's own _register-sid handshake wrote — see clippy.toml"
        )]
        let read = std::fs::read_to_string(&file);
        if let Ok(text) = read {
            let _ = std::fs::remove_file(&file);
            let id: String = text.chars().filter(|c| !c.is_whitespace()).collect();
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    let _ = &target.launch_id;
    // The TUI scrape, least reliable and therefore last: codex prints
    // `session id: <uuid>` once in its header.
    let screen = crate::transport::capture_pane(server, &target.pane)?;
    scrape_session_id(&screen)
}

/// The first `session id: <hex-and-dashes>` a screen carries.
pub(crate) fn scrape_session_id(screen: &str) -> Option<String> {
    for line in screen.lines() {
        let Some(rest) = line.split("session id: ").nth(1) else {
            continue;
        };
        let id: String = rest
            .chars()
            .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
            .collect();
        if !id.is_empty() {
            return Some(id);
        }
    }
    None
}

/// Write the captured id into the roster, under the meta lock the core holds.
fn register(dir: &Path, slot: &str, id: &str) {
    let tail = [
        "set-harness-session".to_owned(),
        slot.to_owned(),
        id.to_owned(),
    ];
    let mut out = Vec::new();
    let mut err = Vec::new();
    let _ = crate::identity::roster(dir, &tail, &mut out, &mut err);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scrape_takes_the_first_id_and_stops_at_the_first_other_byte() {
        let screen = "codex v1\n  session id: 0f9c-4a2b xyz\nmore\n";
        assert_eq!(scrape_session_id(screen).as_deref(), Some("0f9c-4a2b"));
    }

    #[test]
    fn a_screen_with_no_id_captures_nothing() {
        assert_eq!(scrape_session_id("nothing here\n"), None);
    }
}
