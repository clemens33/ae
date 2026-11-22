//! The orchestrator autostart: when the owner starts ae, the orchestrator seat
//! is started or resumed if it is not running.
//!
//! ONE guard at the chokepoint (docs/design-patterns.md §2): every route that
//! counts as "ae starts" — a typed launch, bare `ae`, `ae app` — reaches the
//! seat through [`verdict`] and [`start`] alone, and only after that route has
//! taken its own decision. The start itself is the ordinary launch owner, run
//! as a child `ae orchestrator --no-attach --autostart`: it takes the seat's
//! lifecycle lock, places the record on its recorded server and leaves a live
//! seat exactly as it is. A failure never fails the invocation that asked: it
//! is ONE line, returned to that invocation, and nothing retries it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::{self, OrchestratorAutostart, RosterEditRefusal};
use crate::orchestrator::{AUTOSTART_FLAG, ORCHESTRATOR_SESSION};

/// The global `[workspace]` key that switches it off.
pub const SETTING: &str = "orchestrator_autostart";

/// The tail of every failure line: how the owner starts the seat by hand.
pub const BY_HAND: &str = "start it by hand: ae orchestrator";

/// The child's exit when the seat is up but a seat sits on a measured startup
/// dialog only a human may answer (a missing harness login, for one): the
/// launch's own "needs a human" report, carried as a status rather than read
/// back out of its words.
pub const NEEDS_HUMAN: u8 = 3;

/// What the invoking route knows about this invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kick {
    /// The global config file, when one is selected.
    pub global: Option<PathBuf>,
    /// The operator's `AE_NO_AUTOSTART=1` for this invocation.
    pub no_autostart: bool,
}

/// Whether this invocation starts the seat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing to do, and nothing said.
    Skip,
    /// Nothing started; this one line says why.
    Note(String),
    /// Run the ordinary launch owner.
    Start,
}

/// The guards, cheapest first, none of which writes anything: the per-launch
/// suppression, the global switch (an unusable value fails CLOSED), and a
/// `[roster] orchestrator` row in the global config. No row is the owner who
/// configured no orchestrator, so nothing is said; any other reader trouble
/// starts, and the launch owner's own refusal names it.
#[must_use]
pub fn verdict(kick: &Kick) -> Verdict {
    if kick.no_autostart {
        return Verdict::Skip;
    }
    match config::global_orchestrator_autostart(kick.global.as_deref()) {
        OrchestratorAutostart::On => {}
        OrchestratorAutostart::Off => return Verdict::Skip,
        OrchestratorAutostart::Unusable(why) => {
            return Verdict::Note(format!(
                "ae: orchestrator autostart: [workspace] {SETTING} {why}; nothing was started."
            ));
        }
    }
    let Some(global) = kick.global.as_deref() else {
        return Verdict::Skip;
    };
    match config::read_global_text(global) {
        Ok(None) => Verdict::Skip,
        Ok(Some(text)) => match config::roster_row_value(&text, ORCHESTRATOR_SESSION) {
            Err(RosterEditRefusal::Absent) => Verdict::Skip,
            _ => Verdict::Start,
        },
        Err(_) => Verdict::Start,
    }
}

/// Whether this process may still kick: at most once per invocation, however
/// many routes it crosses (a launch that lands in the app is both).
#[must_use]
pub fn claim_once() -> bool {
    static KICKED: AtomicBool = AtomicBool::new(false);
    !KICKED.swap(true, Ordering::SeqCst)
}

/// Whether a route may kick: only for a `human` caller with both streams on a
/// `terminal`, and only then is the once-per-invocation `claim` taken.
#[must_use]
pub fn may_kick(human: bool, terminal: bool, claim: impl FnOnce() -> bool) -> bool {
    human && terminal && claim()
}

/// Whether bare `ae`'s attach must name its target instead of letting tmux
/// choose: only when this invocation `started` the seat's launch and the seat
/// was not running before it (`had_seat`), because only then can tmux's own
/// choice land on a seat this invocation created.
#[must_use]
pub fn names_the_attach(had_seat: bool, started: bool) -> bool {
    started && !had_seat
}

/// The argv the child runs after the binary.
#[must_use]
pub fn child_args() -> [&'static str; 3] {
    [ORCHESTRATOR_SESSION, "--no-attach", AUTOSTART_FLAG]
}

/// Run the ordinary launch owner for the seat and wait for it. `None` is
/// success, a live seat left alone or a peer's start this one coalesced into;
/// `Some` is the one failure line.
#[must_use]
pub fn start() -> Option<String> {
    let Some(binary) = crate::shape::resolved_exe() else {
        return Some(failure_line("ae could not name its own binary"));
    };
    let Some(output) = crate::transport::run_background(&binary, &child_args()) else {
        return Some(failure_line("the launch did not run"));
    };
    if output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stderr);
    if output.status.code() == Some(i32::from(NEEDS_HUMAN)) {
        let why = text
            .lines()
            .map(str::trim)
            .find(|line| line.contains(NEEDS_HUMAN_WORDS))
            .unwrap_or("a seat sits on a startup dialog");
        return Some(needs_human_line(why));
    }
    let why = first_reason(&text).map_or_else(
        || match output.status.code() {
            Some(code) => format!("the launch exited {code}"),
            None => "the launch was stopped by a signal".to_owned(),
        },
        ToOwned::to_owned,
    );
    Some(failure_line(&why))
}

/// The words of the launch's startup-dialog report this module looks for once
/// the status has said there is one.
pub const NEEDS_HUMAN_WORDS: &str = "needs a human";

/// The line for a seat that started but waits on the owner.
#[must_use]
pub fn needs_human_line(why: &str) -> String {
    format!(
        "ae: orchestrator autostart: {}; attach with: ae orchestrator",
        one_line(why)
    )
}

/// ONE line on any terminal: every control character, a newline included,
/// becomes a space.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

/// The line that names a failed start and how to start the seat by hand.
#[must_use]
pub fn failure_line(why: &str) -> String {
    format!(
        "ae: orchestrator autostart failed: {}; {BY_HAND}",
        one_line(why)
    )
}

/// The first actionable line of the child's output: its first `Error:` line,
/// else its first non-blank line, without the `Error:` word.
#[must_use]
pub fn first_reason(text: &str) -> Option<&str> {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let first = lines.clone().next()?;
    let chosen = lines
        .find(|line| line.starts_with("Error:"))
        .unwrap_or(first);
    Some(
        chosen
            .strip_prefix("Error:")
            .map_or(chosen, str::trim_start),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kick(global: Option<PathBuf>, no_autostart: bool) -> Kick {
        Kick {
            global,
            no_autostart,
        }
    }

    fn config(tag: &str, text: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "ae-orchauto-{tag}-{}-{}",
            std::process::id(),
            crate::time::Timestamp::now().epoch()
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let file = dir.join("config");
        std::fs::write(&file, text).expect("scratch config");
        (dir, file)
    }

    const ROSTER: &str = "[profiles]\nidle = /bin/sleep 600\n[roster]\norchestrator = idle\n";

    #[test]
    fn suppression_wins_over_a_configured_seat() {
        let (dir, file) = config("suppressed", ROSTER);
        assert_eq!(verdict(&kick(Some(file.clone()), true)), Verdict::Skip);
        assert_eq!(verdict(&kick(Some(file), false)), Verdict::Start);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_switch_is_on_by_default_and_fails_closed() {
        for (value, want) in [("on", Some(Verdict::Start)), ("off", Some(Verdict::Skip))] {
            let (dir, file) = config(
                value,
                &format!("{ROSTER}[workspace]\n{SETTING} = {value}\n"),
            );
            assert_eq!(Some(verdict(&kick(Some(file), false))), want, "{value}");
            let _ = std::fs::remove_dir_all(dir);
        }
        let (dir, file) = config(
            "garbage",
            &format!("{ROSTER}[workspace]\n{SETTING} = false\n"),
        );
        let Verdict::Note(line) = verdict(&kick(Some(file), false)) else {
            panic!("an unusable value must start nothing and say so");
        };
        assert!(line.contains(SETTING), "{line}");
        assert!(line.contains("\"false\""), "{line}");
        assert!(line.contains("nothing was started"), "{line}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn no_roster_row_or_no_global_config_is_silent() {
        let (dir, file) = config("norow", "[roster]\nlead = idle\n");
        assert_eq!(verdict(&kick(Some(file.clone()), false)), Verdict::Skip);
        assert_eq!(
            verdict(&kick(Some(dir.join("absent")), false)),
            Verdict::Skip
        );
        assert_eq!(verdict(&kick(None, false)), Verdict::Skip);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_doubled_row_is_left_to_the_launch_owner() {
        let (dir, file) = config(
            "dup",
            "[roster]\norchestrator = idle\norchestrator = idle\n",
        );
        assert_eq!(verdict(&kick(Some(file), false)), Verdict::Start);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_reason_is_the_first_error_line_else_the_first_line() {
        assert_eq!(
            first_reason("ae: note\nError: could not record the launch attempt.\nmore\n"),
            Some("could not record the launch attempt.")
        );
        assert_eq!(first_reason("\n  plain words \nnext"), Some("plain words"));
        assert_eq!(first_reason(" \n\n"), None);
    }

    #[test]
    fn the_failure_line_is_one_line_with_the_hand_start() {
        let line = failure_line("bad\nthing");
        assert!(!line.contains('\n'), "{line}");
        assert!(
            line.starts_with("ae: orchestrator autostart failed: "),
            "{line}"
        );
        assert!(
            line.ends_with("; start it by hand: ae orchestrator"),
            "{line}"
        );
    }

    #[test]
    fn a_seat_that_needs_a_human_is_named_with_the_attach() {
        let line =
            needs_human_line("Session 'orchestrator' seat lead (main) needs a human —\nlog in");
        assert!(!line.contains('\n'), "{line}");
        assert!(line.contains(NEEDS_HUMAN_WORDS), "{line}");
        assert!(line.ends_with("; attach with: ae orchestrator"), "{line}");
        assert!(!line.contains("failed"), "{line}");
    }

    #[test]
    fn one_invocation_claims_the_kick_once() {
        // nextest runs each test in its own process, so this is the first claim.
        assert!(claim_once(), "the first route may kick");
        assert!(
            !claim_once(),
            "a second route of the same invocation may not"
        );
        assert!(!claim_once(), "nor any later one");
    }

    #[test]
    fn only_a_human_on_a_terminal_takes_the_claim() {
        for (human, terminal, claimed, want) in [
            (true, true, true, true),
            (true, true, false, false),
            (true, false, true, false),
            (false, true, true, false),
            (false, false, true, false),
        ] {
            let mut asked = false;
            let got = may_kick(human, terminal, || {
                asked = true;
                claimed
            });
            assert_eq!(
                got, want,
                "human {human} terminal {terminal} claim {claimed}"
            );
            assert_eq!(asked, human && terminal, "the claim is taken only when due");
        }
    }

    #[test]
    fn only_a_start_that_may_have_created_the_seat_names_the_attach() {
        assert!(names_the_attach(false, true));
        assert!(
            !names_the_attach(true, true),
            "a seat that was running stays tmux's choice"
        );
        assert!(
            !names_the_attach(false, false),
            "no start, tmux's own choice"
        );
        assert!(!names_the_attach(true, false));
    }

    #[test]
    fn the_child_is_the_seat_launch_with_the_hidden_flag() {
        assert_eq!(child_args(), ["orchestrator", "--no-attach", "--autostart"]);
        let tail: Vec<String> = child_args()[1..].iter().map(|w| (*w).to_owned()).collect();
        let parsed = crate::orchestrator::parse_launch_tail(&tail).expect("seat flags");
        assert!(parsed.autostart);
        assert_eq!(parsed.attach, Some(false));
    }

    #[test]
    fn an_untargeted_attach_prefers_unattached_then_activity_then_name_order() {
        use crate::tmux::untargeted_attach_choice as pick;
        let rows = "0 100 alpha\n0 200 beta\n1 300 gamma\n0 400 orchestrator\n";
        // The newest unattached session wins, an attached one never beats it, and
        // the excluded name is not there at all.
        assert_eq!(pick(rows, "orchestrator").as_deref(), Some("beta"));
        assert_eq!(pick(rows, "nobody").as_deref(), Some("orchestrator"));
        // A tie keeps the first in name order.
        assert_eq!(pick("0 5 a\n0 5 b\n", "x").as_deref(), Some("a"));
        // Only attached sessions: the latest activity among them.
        assert_eq!(pick("1 5 a\n2 9 b\n", "x").as_deref(), Some("b"));
        // A name with a blank in it is the rest of the row.
        assert_eq!(pick("0 5 my work\n", "x").as_deref(), Some("my work"));
        // Nothing left, or a row that is not this format's.
        assert_eq!(pick("0 5 orchestrator\n", "orchestrator"), None);
        assert_eq!(pick("zero 5 a\n", "x"), None);
    }
}
