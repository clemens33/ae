//! The leg that MOVES a seat: `_auto-reseat <dir> <slot> <key>`.
//!
//! It re-derives everything the trigger decided from the records and the global
//! config, moves the seat through the one reseat, and journals ONE outcome. A
//! forged call can therefore do only what the watchdog would do at that moment.

use std::path::{Path, PathBuf};

use crate::reseat::Ended;
use crate::time::Timestamp;

use super::{Candidate, Ineligible};

/// The argv, proven before anything is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Argv {
    pub session: String,
    pub dir: PathBuf,
    pub slot: String,
    pub key: Timestamp,
}

/// Prove `<dir> <slot> <key>` against `sessions`, the state root's session
/// directory.
pub(crate) fn parse_argv(sessions: &Path, argv: &[String]) -> Result<Argv, String> {
    let _ = (sessions, argv);
    Err(String::new())
}

/// The record one outcome is journaled as: its action, `ref` and summary.
pub(crate) fn outcome(
    ended: Ended,
    key: &str,
    left: &str,
    to: &str,
    err: &str,
) -> (&'static str, String, String) {
    let _ = (ended, key, left, to, err);
    ("", String::new(), String::new())
}

/// The summary of the hold journaled when the seat is no longer eligible.
pub(crate) const fn ineligible_summary(why: Ineligible) -> &'static str {
    let _ = why;
    ""
}

/// The declared candidates as the chooser judges them.
pub(crate) fn candidates(
    list: &[String],
    resolves: impl Fn(&str) -> bool,
    left: &[(String, Timestamp)],
) -> Vec<Candidate> {
    let _ = (list, resolves, left);
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autoreseat::{
        DONE_ACTION, FAILED_ACTION, HELD_ACTION, REFUSED_ACTION, Skip, Tier, choose, left_profiles,
    };
    use crate::events::Event;

    const KEY: &str = "2026-09-25T10:00:00Z";

    /// A state root with one session directory, removed with the test.
    struct Root(PathBuf);

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn words(argv: &[&str]) -> Vec<String> {
        argv.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn the_argv_is_proven_before_anything_is_read() {
        let root = Root(std::env::temp_dir().join(format!("ae-leg-{}", std::process::id())));
        let sessions = root.0.join("sessions");
        let dir = sessions.join("aedev");
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
        assert!(std::fs::create_dir_all(sessions.join("bad name")).is_ok());
        assert!(std::os::unix::fs::symlink(&dir, sessions.join("alias")).is_ok());
        let path = |name: &str| sessions.join(name).display().to_string();
        let parsed = parse_argv(&sessions, &words(&[&path("aedev"), "spawned.3", KEY]));
        assert_eq!(
            parsed.map(|argv| (argv.session, argv.slot, argv.key.to_string())),
            Ok(("aedev".to_owned(), "spawned.3".to_owned(), KEY.to_owned()))
        );
        let climbs = format!("{}/../sessions/aedev", sessions.display());
        for argv in [
            words(&[&path("aedev"), "spawned.3"]),
            words(&[&path("aedev"), "spawned.3", KEY, "extra"]),
            words(&[&climbs, "spawned.3", KEY]),
            words(&[&path("alias"), "spawned.3", KEY]),
            words(&[&path("missing"), "spawned.3", KEY]),
            words(&[&path("bad name"), "spawned.3", KEY]),
            words(&["aedev", "spawned.3", KEY]),
            words(&[&path("aedev"), "worker.x", KEY]),
            words(&[&path("aedev"), "spawned.3", "yesterday"]),
        ] {
            assert!(parse_argv(&sessions, &argv).is_err(), "{argv:?}");
        }
    }

    #[test]
    fn each_ending_is_journaled_as_one_outcome_with_its_ref() {
        let left = "sol6x";
        for (ended, action, reference, summary) in [
            (
                Ended::Moved { carried: true },
                DONE_ACTION,
                left,
                "to opus55x, carried",
            ),
            (
                Ended::Moved { carried: false },
                DONE_ACTION,
                left,
                "to opus55x, seeded",
            ),
            (
                Ended::Refused { transient: true },
                HELD_ACTION,
                KEY,
                "Error: busy",
            ),
            (
                Ended::Refused { transient: false },
                REFUSED_ACTION,
                KEY,
                "Error: busy",
            ),
            (Ended::Failed, FAILED_ACTION, KEY, "Error: busy"),
        ] {
            assert_eq!(
                outcome(ended, KEY, left, "opus55x", "Error: busy\n  next line\n"),
                (action, reference.to_owned(), summary.to_owned()),
                "{ended:?}"
            );
        }
        // The summary is ONE line, control-free and bounded.
        let (_, _, summary) = outcome(
            Ended::Failed,
            KEY,
            left,
            "opus55x",
            &format!("\u{1b}[31mError: {}\n", "x".repeat(900)),
        );
        assert!(
            !summary.contains('\u{1b}') && !summary.contains('\n'),
            "{summary:?}"
        );
        assert!(summary.len() <= 200, "{}", summary.len());
    }

    #[test]
    fn each_reason_a_seat_is_no_longer_eligible_reads_as_its_own_hold() {
        let summaries = [
            Ineligible::Off,
            Ineligible::Orchestrator,
            Ineligible::Session,
            Ineligible::Slot,
            Ineligible::Class,
            Ineligible::Unmapped,
        ]
        .map(ineligible_summary);
        for (index, summary) in summaries.iter().enumerate() {
            assert!(summary.starts_with("held: "), "{summary:?}");
            assert!(summary.len() > "held: ".len(), "{summary:?}");
            assert!(!summaries[..index].contains(summary), "{summary:?}");
        }
    }

    #[test]
    fn with_no_quota_read_the_first_usable_declared_candidate_is_taken() {
        let done = Event::parse_line(&format!(
            r#"{{"ts":"{KEY}","actor":"watchdog","action":"auto-reseat-done","target":"scout","ref":"sol6x"}}"#
        ))
        .expect("a well-formed record");
        let left = left_profiles(&[done], "aedev", "spawned.3", "scout");
        let list = words(&["ghost", "sol6x", "opus55x", "astrax"]);
        let choice = choose(&candidates(&list, |profile| profile != "ghost", &left), 0);
        assert_eq!(choice.pick, Some(("opus55x".to_owned(), Tier::Unknown)));
        assert_eq!(
            choice.skipped,
            [
                ("ghost".to_owned(), Skip::Unconfigured),
                ("sol6x".to_owned(), Skip::LeftOnLimit),
            ]
        );
    }
}
