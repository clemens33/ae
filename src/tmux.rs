//! Addressing a tmux server, and reading what it said.
//!
//! Everything here is PURE: argument derivation in, text interpretation out.
//! Running the child process is the caller's job, and deliberately not this
//! crate's — `clippy.toml` denies `std::process::Command` outside two pinned
//! doors, and that boundary holds only while the library cannot spawn. So the
//! product owns the two decisions that can be wrong — WHICH server an argument
//! list addresses, and WHAT a completed run means — and the exec is a detail
//! around them.
//!
//! # Handover: what exists here, and what the next slice must add
//!
//! **The pure half is done and proven.** Typed `Selector` to argv is here
//! ([`server_args`], [`list_sessions_args`], [`marker_args`]); completed-run
//! interpretation is here ([`interpret_sessions`], [`interpret_marker`]), with
//! the exit status deciding and the payload bytes never doing so. Both halves
//! are exercised against two REAL isolated tmux servers — one `-L` named, one
//! `-S` socket — each answering only with its own session, plus a real
//! `AE_SESSION` round trip and a socket-that-is-not-a-server failure arm. See
//! `tests/it/phase2.rs`, criterion 20.
//!
//! **The missing piece is exactly one thing: THE EXEC.** Nothing here spawns,
//! because `clippy.toml` denies `std::process::Command` outside two pinned test
//! doors and the library keeps that boundary. So a real transport is one
//! deliberate crossing — a third door, enumerated like the others — and NOT a
//! rewrite: the decisions that can be wrong (which server an argument list
//! addresses, what a completed run means) already live in this module and are
//! already tested.
//!
//! **What that unlocks, in order:**
//!
//! * SC-017k/SC-017l — session liveness stops being universally `unknown`,
//!   because [`interpret_sessions`] finally receives a real answer.
//! * SC-017p/SC-017q — per-agent liveness. This module does NOT yet derive
//!   panes: `list-panes` argv and its interpretation are the next functions to
//!   write here, and they need the same treatment — exact association to a
//!   roster slot, and ambiguity routed to `unknown` rather than to `dead`.
//! * SC-017o — an entitled server that can actually be enumerated stops
//!   counting as a failed source, so `inventory_complete` stops being false on
//!   every machine that records a server.
//!
//! Until then [`crate::run`]'s transport fails every query by construction, and
//! every one of those three surfaces reports the honest `unknown`.
//!
//! # Why the exit status decides and the bytes do not
//!
//! **SC-017k** grants `running`/`stopped` only to a SUCCESSFUL query, and
//! **SC-017l** sends every failure to `unknown`. A failed `tmux list-sessions`
//! can still print something — a partial line, a warning, nothing at all — and
//! empty output from a failed query looks exactly like empty output from a
//! server with no sessions. One of those proves absence and the other proves
//! nothing, so [`interpret_sessions`] takes the transport result FIRST and reads
//! the bytes only after it knows they mean anything.

use std::path::Path;

use crate::inventory::{QueryFailed, ServerId};
use crate::meta::Selector;

/// The format `list-sessions` is asked for: one exact session name per line.
///
/// `#{session_name}` and nothing else. A format that also carried, say, the
/// pane count would make every parse position-dependent for no gain, and the
/// only field this phase needs is the one it matches EXACTLY.
pub const SESSION_NAME_FORMAT: &str = "#{session_name}";

/// The ae-ownership marker, read from a session's own tmux environment.
pub const OWNERSHIP_VARIABLE: &str = "AE_SESSION";

/// The arguments that address `server`, before any subcommand.
///
/// The typed selector IS the routing: `-L` addresses a server by name and `-S`
/// by socket path, and they are different servers even when the strings match.
/// [`ServerId::Ambient`] adds nothing — the ordinary transport's own server is
/// whatever `tmux` alone talks to, and SC-1410c owns how that was selected.
#[must_use]
pub fn server_args(server: &ServerId) -> Vec<String> {
    match server {
        ServerId::Ambient => Vec::new(),
        ServerId::Selected(Selector::Name(name)) => vec!["-L".to_owned(), name.clone()],
        ServerId::Selected(Selector::Socket(path)) => {
            vec!["-S".to_owned(), path.display().to_string()]
        }
    }
}

/// The full argument list for enumerating `server`'s sessions.
#[must_use]
pub fn list_sessions_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.push("list-sessions".to_owned());
    args.push("-F".to_owned());
    args.push(SESSION_NAME_FORMAT.to_owned());
    args
}

/// The full argument list for reading one session's ownership marker.
///
/// `-t <name>` is an EXACT target here only because the name came from
/// `list-sessions` on this same server; tmux itself would prefix-match it. That
/// is why nothing in this crate ever asks tmux whether a name exists — see
/// [`crate::liveness`], where the exact match is done on ae's side of the wire.
#[must_use]
pub fn marker_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.push("show-environment".to_owned());
    args.push("-t".to_owned());
    args.push(session.to_owned());
    args.push(OWNERSHIP_VARIABLE.to_owned());
    args
}

/// What a completed `list-sessions` run means.
///
/// # Errors
///
/// [`QueryFailed`] whenever the run did not succeed, whatever it printed. A
/// non-zero tmux is "no server running on …" as often as anything else, and
/// "there is no server" is not the same fact as "the server says there are no
/// sessions" — SC-017l is explicit that the first one is `unknown`.
pub fn interpret_sessions(succeeded: bool, stdout: &str) -> Result<Vec<String>, QueryFailed> {
    if !succeeded {
        return Err(QueryFailed);
    }
    Ok(stdout
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

/// The ownership marker a completed `show-environment` run reported.
///
/// `AE_SESSION=<value>` is the marker; tmux spells an UNSET variable
/// `-AE_SESSION`, which is evidence of absence rather than a value, and a failed
/// run is no evidence at all. Both answer `None`, and [`crate::liveness`] treats
/// that as ownership not established — never as proof the session is not ae's,
/// because those differ and only one of them could ever justify `stopped`.
#[must_use]
pub fn interpret_marker(succeeded: bool, stdout: &str) -> Option<String> {
    if !succeeded {
        return None;
    }
    stdout
        .lines()
        .find_map(|line| {
            line.trim_end()
                .strip_prefix(&format!("{OWNERSHIP_VARIABLE}="))
        })
        .map(ToOwned::to_owned)
}

/// Whether `path` can address a tmux server at all.
///
/// A relative socket path is SC-405l's `ambiguous`, refused before it becomes a
/// selector; this is the same question asked at the wire, so a path that arrived
/// some other way cannot become a target here either.
#[must_use]
pub fn is_addressable_socket(path: &Path) -> bool {
    path.is_absolute()
}

#[cfg(test)]
mod tests {
    use super::{
        interpret_marker, interpret_sessions, is_addressable_socket, list_sessions_args,
        marker_args, server_args,
    };
    use crate::inventory::{QueryFailed, ServerId};
    use crate::meta::Selector;
    use std::path::{Path, PathBuf};

    fn named(name: &str) -> ServerId {
        ServerId::Selected(Selector::Name(name.to_owned()))
    }

    fn socket(path: &str) -> ServerId {
        ServerId::Selected(Selector::Socket(PathBuf::from(path)))
    }

    #[test]
    fn the_typed_selector_chooses_the_tmux_routing_flag() {
        assert_eq!(server_args(&ServerId::Ambient), Vec::<String>::new());
        assert_eq!(server_args(&named("work")), ["-L", "work"]);
        assert_eq!(server_args(&socket("/tmp/ae.sock")), ["-S", "/tmp/ae.sock"]);
    }

    #[test]
    fn a_name_and_a_socket_of_the_same_spelling_address_different_servers() {
        // -L /tmp/x and -S /tmp/x are not the same tmux. The typed halves must
        // not collapse anywhere on the path from selector to argv.
        assert_ne!(
            server_args(&named("/tmp/x")),
            server_args(&socket("/tmp/x"))
        );
    }

    #[test]
    fn enumeration_asks_for_exact_names_and_nothing_else() {
        assert_eq!(
            list_sessions_args(&named("work")),
            ["-L", "work", "list-sessions", "-F", "#{session_name}"]
        );
        assert_eq!(
            list_sessions_args(&ServerId::Ambient),
            ["list-sessions", "-F", "#{session_name}"]
        );
    }

    #[test]
    fn the_marker_is_read_from_the_session_s_own_environment() {
        assert_eq!(
            marker_args(&socket("/tmp/ae.sock"), "my-feature"),
            [
                "-S",
                "/tmp/ae.sock",
                "show-environment",
                "-t",
                "my-feature",
                "AE_SESSION"
            ]
        );
    }

    #[test]
    fn a_failed_run_is_a_failure_whatever_it_printed() {
        // The bytes are identical in both arms; only the transport result moves.
        for payload in ["", "alpha\nbeta\n", "no server running on /tmp/x\n"] {
            assert_eq!(
                interpret_sessions(false, payload),
                Err(QueryFailed),
                "{payload:?}"
            );
        }
    }

    #[test]
    fn a_successful_run_yields_exactly_the_names_it_printed() {
        assert_eq!(
            interpret_sessions(true, "alpha\nbeta\n"),
            Ok(vec!["alpha".to_owned(), "beta".to_owned()])
        );
        assert_eq!(
            interpret_sessions(true, ""),
            Ok(Vec::new()),
            "an empty SUCCESS is the only thing that can prove a name absent"
        );
        assert_eq!(
            interpret_sessions(true, "solo\n\n"),
            Ok(vec!["solo".to_owned()]),
            "a blank line is not a session called nothing"
        );
    }

    #[test]
    fn a_name_containing_spaces_survives_intact() {
        // The format asks for one field, so the whole line is the name.
        assert_eq!(
            interpret_sessions(true, "my session\n"),
            Ok(vec!["my session".to_owned()])
        );
    }

    #[test]
    fn the_marker_is_the_value_and_an_unset_variable_is_not_one() {
        assert_eq!(
            interpret_marker(true, "AE_SESSION=my-feature\n"),
            Some("my-feature".to_owned())
        );
        assert_eq!(
            interpret_marker(true, "-AE_SESSION\n"),
            None,
            "tmux spells UNSET with a leading dash, and that is not a value"
        );
        assert_eq!(interpret_marker(true, ""), None);
        assert_eq!(
            interpret_marker(false, "AE_SESSION=my-feature\n"),
            None,
            "a failed run reports nothing, however convincing its output looks"
        );
        assert_eq!(
            interpret_marker(true, "AE_SESSION=\n"),
            Some(String::new()),
            "an empty value is a value; whether it PROVES ownership is liveness's question"
        );
    }

    #[test]
    fn a_relative_socket_path_cannot_address_a_server() {
        assert!(is_addressable_socket(Path::new("/tmp/ae.sock")));
        assert!(!is_addressable_socket(Path::new("relative/ae.sock")));
    }
}
