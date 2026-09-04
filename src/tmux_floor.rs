//! The tmux version floor, and the refusal below it.
//!
//! ae's newer surfaces are drawn by tmux itself — `display-menu` and the
//! popup positions it accepts — so the version of the SERVER decides whether
//! they can be drawn at all. The check is a read (`display-message -p
//! '#{version}'`) and a comparison; it never restarts a server and never
//! upgrades one, because both are the operator's call on a machine that may be
//! running other people's sessions.

use crate::inventory::ServerId;
use crate::meta::Selector;

/// The oldest tmux ae draws a menu on.
pub const REQUIRED: Version = Version { major: 3, minor: 4 };

/// The exit a refused gate takes — "everything else", not a usage error.
pub const EXIT_REFUSED: u8 = 1;

/// The `major.minor` of a tmux release, which is all the floor compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    /// `3` of `3.4`.
    pub major: u32,
    /// `4` of `3.4`.
    pub minor: u32,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// What tmux answered `#{version}` with, once read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// A release ae can compare — `3.4`, `3.5a`, `next-3.6`.
    Release(Version),
    /// A development build that is ahead of every release by construction.
    Development,
    /// A spelling this module will not guess at.
    Unreadable,
}

/// The development spelling tmux reports from its own git tip.
const DEVELOPMENT: &str = "master";

/// The prefix tmux's pre-release builds carry before their release number.
const NEXT_PREFIX: &str = "next-";

/// Read `#{version}` as the floor compares it.
///
/// The trailing letter of a point release (`3.5a`) is dropped: it never moves
/// the pair the floor is written in, and a floor that pretended otherwise
/// would refuse `3.4a` for being older than `3.4`.
///
/// ```
/// use ae::tmux_floor::{Reading, Version, read};
/// assert_eq!(read("3.4"), Reading::Release(Version { major: 3, minor: 4 }));
/// assert_eq!(read("3.5a"), Reading::Release(Version { major: 3, minor: 5 }));
/// assert_eq!(read("next-3.6"), Reading::Release(Version { major: 3, minor: 6 }));
/// assert_eq!(read("master"), Reading::Development);
/// assert_eq!(read("openbsd"), Reading::Unreadable);
/// ```
#[must_use]
pub fn read(raw: &str) -> Reading {
    let text = raw.trim();
    if text == DEVELOPMENT {
        return Reading::Development;
    }
    let text = text.strip_prefix(NEXT_PREFIX).unwrap_or(text);
    let Some((major, rest)) = text.split_once('.') else {
        return Reading::Unreadable;
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    // The remainder is ONE point-release letter or nothing: `3.5a` is a release,
    // `3.5-rc` and `3.5evil` are spellings this module has never seen.
    let tail = &rest[digits.len()..];
    let suffix_is_a_point_release =
        tail.is_empty() || (tail.len() == 1 && tail.starts_with(|c: char| c.is_ascii_alphabetic()));
    if digits.is_empty() || !suffix_is_a_point_release {
        return Reading::Unreadable;
    }
    match (major.parse(), digits.parse()) {
        (Ok(major), Ok(minor)) => Reading::Release(Version { major, minor }),
        _ => Reading::Unreadable,
    }
}

/// Whether `raw` clears [`REQUIRED`].
///
/// ```
/// assert!(ae::tmux_floor::clears("3.4"));
/// assert!(ae::tmux_floor::clears("4.0"));
/// assert!(!ae::tmux_floor::clears("3.3a"));
/// assert!(!ae::tmux_floor::clears("2.9"));
/// ```
#[must_use]
pub fn clears(raw: &str) -> bool {
    match read(raw) {
        Reading::Release(found) => found >= REQUIRED,
        Reading::Development => true,
        Reading::Unreadable => false,
    }
}

/// How a server is named back to the operator who has to fix it.
///
/// ```
/// use ae::inventory::ServerId;
/// assert_eq!(ae::tmux_floor::server_label(&ServerId::Ambient), "the current server ($TMUX)");
/// ```
#[must_use]
pub fn server_label(server: &ServerId) -> String {
    match server {
        ServerId::Ambient => "the current server ($TMUX)".to_owned(),
        ServerId::Selected(Selector::Name(name)) => format!("-L {name}"),
        ServerId::Selected(Selector::Socket(path)) => format!("-S {}", path.display()),
    }
}

/// What a server that did not answer `#{version}` is reported as.
pub const UNANSWERED: &str = "(no answer)";

/// The refusal `command` prints for a `found` version on `server`, terminating
/// newline included.
///
/// A server that answered nothing is a different failure from one that
/// answered too low a number, so the headline says which it was — the fix for
/// the first is to start a server, and for the second to upgrade one.
///
/// ```
/// use ae::inventory::ServerId;
/// let text = ae::tmux_floor::refusal("orchestrator", "3.3a", &ServerId::Ambient);
/// assert!(text.contains("found:    3.3a"));
/// assert!(text.contains("required: 3.4 or newer"));
/// ```
#[must_use]
pub fn refusal(command: &str, found: &str, server: &ServerId) -> String {
    let answered = !found.trim().is_empty();
    let (headline, guidance) = if answered {
        (
            "this tmux is too old to draw the menu",
            "Upgrade tmux (macOS: brew upgrade tmux; Debian/Ubuntu: apt install tmux), then start a\nNEW server with the upgraded binary — ae never restarts a running server for you.",
        )
    } else {
        (
            "no tmux server answered, so there is no client to draw the menu on",
            "Start that server, or run this from inside one of its panes — ae never starts or\nrestarts a server for you.",
        )
    };
    let found = if answered { found.trim() } else { UNANSWERED };
    format!(
        "ae {command}: {headline}.\n  \
         found:    {found}\n  \
         required: {REQUIRED} or newer\n  \
         server:   {}\n\
         {guidance}\n",
        server_label(server)
    )
}

#[cfg(test)]
mod tests {
    use super::{
        EXIT_REFUSED, REQUIRED, Reading, UNANSWERED, Version, clears, read, refusal, server_label,
    };
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    use std::path::PathBuf;

    #[test]
    fn the_floor_is_the_ruled_three_four() {
        assert_eq!(REQUIRED, Version { major: 3, minor: 4 });
        assert_eq!(REQUIRED.to_string(), "3.4");
    }

    #[test]
    fn a_point_release_letter_never_moves_the_pair_the_floor_compares() {
        // 3.4a is 3.4 with fixes, so a floor of 3.4 must admit it.
        for spelling in ["3.4", "3.4a", "3.4b"] {
            assert!(clears(spelling), "{spelling}");
        }
        for spelling in ["3.3", "3.3a", "2.9", "1.8"] {
            assert!(!clears(spelling), "{spelling}");
        }
    }

    #[test]
    fn a_release_ahead_of_the_floor_clears_it_on_either_number() {
        assert!(clears("3.5"));
        assert!(clears("3.7b"));
        assert!(clears("4.0"));
        assert!(clears("10.0"), "a two-digit major is not a shorter string");
    }

    #[test]
    fn the_two_development_spellings_are_ahead_of_every_release() {
        assert_eq!(read("master"), Reading::Development);
        assert!(clears("master"));
        assert_eq!(
            read("next-3.6"),
            Reading::Release(Version { major: 3, minor: 6 })
        );
        assert!(clears("next-3.6"));
    }

    #[test]
    fn surrounding_whitespace_is_the_capture_and_not_the_version() {
        assert!(clears(" 3.7b\n"));
        assert_eq!(read("\t3.4 "), Reading::Release(REQUIRED));
    }

    #[test]
    fn a_spelling_this_module_cannot_read_fails_closed() {
        for spelling in [
            "", "   ", "openbsd", "3", "3.", "3.x", "3.5-rc1", "x.y", "3.4evil", "3.4ab",
        ] {
            assert_eq!(read(spelling), Reading::Unreadable, "{spelling}");
            assert!(!clears(spelling), "{spelling}");
        }
    }

    #[test]
    fn the_refusal_carries_found_required_and_the_server_it_asked() {
        let text = refusal("orchestrator", "3.3a", &ServerId::Ambient);
        assert!(
            text.starts_with("ae orchestrator: this tmux is too old"),
            "{text}"
        );
        assert!(text.contains("found:    3.3a"), "{text}");
        assert!(text.contains("required: 3.4 or newer"), "{text}");
        assert!(
            text.contains("server:   the current server ($TMUX)"),
            "{text}"
        );
        assert!(text.contains("brew upgrade tmux"), "{text}");
        assert!(
            text.contains("ae never restarts a running server for you"),
            "the gate says what it will NOT do: {text}"
        );
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn a_server_that_did_not_answer_is_named_as_such_rather_than_as_a_blank() {
        let text = refusal("orchestrator", "  ", &ServerId::Ambient);
        assert!(text.contains(&format!("found:    {UNANSWERED}")), "{text}");
        assert!(
            text.contains("no tmux server answered"),
            "silence is not the same failure as an old release: {text}"
        );
        assert!(
            text.contains("ae never starts or\nrestarts a server for you"),
            "silence is fixed by starting a server, not by upgrading one: {text}"
        );
        let old = refusal("orchestrator", "3.3a", &ServerId::Ambient);
        assert!(
            !old.contains("no tmux server"),
            "an answered version is an upgrade problem"
        );
        assert!(old.contains("brew upgrade tmux"), "{old}");
    }

    #[test]
    fn every_addressed_server_names_the_flags_that_would_reach_it() {
        assert_eq!(
            server_label(&ServerId::Selected(Selector::Name("ae-dev".to_owned()))),
            "-L ae-dev"
        );
        assert_eq!(
            server_label(&ServerId::Selected(Selector::Socket(PathBuf::from(
                "/tmp/s"
            )))),
            "-S /tmp/s"
        );
        assert_eq!(
            server_label(&ServerId::Ambient),
            "the current server ($TMUX)"
        );
    }

    #[test]
    fn a_refused_gate_is_a_failure_and_not_a_usage_error() {
        assert_eq!(EXIT_REFUSED, 1);
    }
}
