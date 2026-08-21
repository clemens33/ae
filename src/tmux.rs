//! Addressing a tmux server, and reading what it said.
//!
//! Everything here is PURE: argument derivation in, text interpretation out.
//! Running the child process is [`crate::transport`]'s job, and deliberately not
//! this module's. The split is what keeps the two decisions that can be WRONG —
//! WHICH server an argument list addresses, and WHAT a completed run means —
//! unit-testable without a process anywhere near them; the exec is a detail
//! around them, behind the one door `clippy.toml` opens in product code.
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
//! **The exec has LANDED.** [`crate::transport::Tmux`] runs these argument lists
//! and feeds these interpreters, so what this module derives now reaches a real
//! server on the ordinary `ae list` route. It is one deliberate crossing of the
//! `std::process::Command` deny — a third door, enumerated in `clippy.toml`
//! beside the two test ones — and it changed nothing here: the transport derives
//! no argv of its own and interprets no bytes of its own.
//!
//! **What that unlocked, and what it did not:**
//!
//! * SC-017k/SC-017l — DONE. Session liveness is no longer universally
//!   `unknown`: [`interpret_sessions`] receives a real answer, and a candidate is
//!   `running` or `stopped` on it. A server that does not answer is still
//!   `unknown`, which is the row's whole point.
//! * SC-017o — DONE for the same reason: an entitled server that can be
//!   enumerated stops counting as a failed source, so `inventory_complete` is no
//!   longer false on every machine that records a server.
//! * SC-017p/SC-017q — the DERIVATION AND THE READING ARE HERE NOW
//!   ([`list_panes_args`], [`interpret_panes`], [`slot_observation`]), with
//!   association on SC-602's `@ae_slot` and ambiguity representable rather than
//!   collapsed. THE VERDICT IS NOT, and deliberately: SC-017p grants `alive`
//!   only on an observation that "positively recognizes its agent process as
//!   live", and no ratified row defines that predicate — the phrase occurs once
//!   in the contract, inside the row that depends on it. SC-906 is the only
//!   candidate, is unratified, and is a DEAD predicate rather than a live one.
//!   So nothing here is wired to a liveness answer, and no seam is left for one:
//!   a seam built toward an unratified predicate is a decision, not preparation.
//!   Do not add `#{pane_current_command}` to the format to "get ready" — that
//!   field IS the live predicate, and adding it is the decision.
//!
//!   What a future slice needs from here is nothing: the facts are complete for
//!   every route SC-017p describes. What it needs is the ratified predicate.
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

/// The format `list-panes` is asked for: one slot marker per pane.
///
/// **`@ae_slot`, not `@ae_agent`** — SC-602 rules that the slot option carries
/// IDENTITY and `@ae_agent` is display. The frozen script associated on the
/// display field (`cmd_list`'s alive map is keyed on `#{@ae_agent}`, ae:4207),
/// which is a second defect in the code SC-017p/q/r already indict.
///
/// One field, deliberately. `#{pane_current_command}` would ride the same
/// enumeration for free and is exactly what a live predicate would need — which
/// is why it is NOT here: no ratified row defines "positively recognizes its
/// agent process as live", and a format that carried the field anyway would be
/// building toward a decision nobody has made.
pub const PANE_SLOT_FORMAT: &str = "#{@ae_slot}";

/// The full argument list for enumerating one session's panes.
///
/// `-s` is session-wide: every pane in the session, not just the active
/// window's. SC-017p's negative proof needs the COMPLETE enumeration, and a
/// window-scoped answer would prove absence from one window while reading like
/// absence from the session.
///
/// **`-t <session>` PREFIX-MATCHES, and that is measured rather than assumed**:
/// against a server holding `probe`, `list-panes -s -t prob` returns `probe`'s
/// panes and exits 0. So this argument list is exact only when `session` is a
/// name a `list-sessions` answer from THIS server already returned exactly —
/// the same precondition as [`marker_args`], and the same hazard that makes a
/// prefix sibling issue #105 rather than a neighbour of it.
#[must_use]
pub fn list_panes_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.push("list-panes".to_owned());
    args.push("-s".to_owned());
    args.push("-t".to_owned());
    args.push(session.to_owned());
    args.push("-F".to_owned());
    args.push(PANE_SLOT_FORMAT.to_owned());
    args
}

/// One pane the server reported, and the slot marker it carries.
///
/// Carries the marker and nothing else, because the marker is the only thing
/// this phase is entitled to read. A pane with no usable marker is still A
/// PANE — that is the whole reason this is a struct with an `Option` rather
/// than a list of names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedPane {
    /// The `@ae_slot` value, when the pane carries a usable one.
    ///
    /// `None` covers unset AND set-to-empty, which are byte-identical in a
    /// format expansion (measured: a session of three panes whose middle pane
    /// has no marker prints `main\n\nworker\n`, and setting that pane's option
    /// to the empty string changes nothing). ae does not pretend to tell apart
    /// two states tmux reports identically.
    pub slot: Option<String>,
}

/// What a completed `list-panes` run means.
///
/// # Errors
///
/// [`QueryFailed`] whenever the run did not succeed, whatever it printed —
/// SC-017q sends a failed pane query to `unknown`, exactly as SC-017l does for
/// sessions. Measured: a `-t` naming no session exits 1.
///
/// **AN EMPTY LINE IS A PANE HERE, and that is the opposite of
/// [`interpret_sessions`].** A blank line in a `list-sessions` answer is not a
/// session called nothing, so that reader drops it; a blank line in a
/// `list-panes` answer IS a pane whose slot option is unset, and dropping it
/// would delete exactly the evidence SC-017q needs — an unassociated pane is
/// what keeps a missing roster agent `unknown` instead of `dead`. The two
/// interpreters must not share a filter, however alike their shapes look.
pub fn interpret_panes(succeeded: bool, stdout: &str) -> Result<Vec<ObservedPane>, QueryFailed> {
    if !succeeded {
        return Err(QueryFailed);
    }
    Ok(stdout
        .lines()
        .map(|line| ObservedPane {
            slot: Some(line.trim_end().to_owned()).filter(|slot| !slot.is_empty()),
        })
        .collect())
}

/// What an enumeration says about one roster slot.
///
/// FACTS, NOT A VERDICT. Which of these licenses `alive`, `dead` or `unknown` is
/// SC-017p/q's question and is answered where liveness is decided — not here.
/// [`Self::Absent`] carries the unidentified-pane COUNT rather than a boolean
/// conclusion for that reason: whether zero unidentified panes is enough to
/// prove a roster agent absent is a reading of SC-017p, and this type refuses to
/// make it on the caller's behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotObservation {
    /// Exactly one observed pane carries this slot.
    Unique,
    /// More than one does, so the association is ambiguous.
    Duplicated {
        /// How many panes carry it.
        panes: usize,
    },
    /// No observed pane carries this slot.
    Absent {
        /// How many observed panes carry no usable marker at all.
        unidentified: usize,
    },
}

/// What `panes` says about `slot`.
///
/// EXACT equality against the roster slot. tmux prefix-matches targets, ae does
/// not: the comparison happens on ae's side of the wire, which is the same rule
/// [`crate::liveness`] applies to session names and for the same reason.
#[must_use]
pub fn slot_observation(panes: &[ObservedPane], slot: &str) -> SlotObservation {
    let carrying = panes
        .iter()
        .filter(|pane| pane.slot.as_deref() == Some(slot))
        .count();
    match carrying {
        0 => SlotObservation::Absent {
            unidentified: panes.iter().filter(|pane| pane.slot.is_none()).count(),
        },
        1 => SlotObservation::Unique,
        panes => SlotObservation::Duplicated { panes },
    }
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
        ObservedPane, SlotObservation, interpret_marker, interpret_panes, interpret_sessions,
        is_addressable_socket, list_panes_args, list_sessions_args, marker_args, server_args,
        slot_observation,
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

    fn pane(slot: Option<&str>) -> ObservedPane {
        ObservedPane {
            slot: slot.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn pane_enumeration_is_session_wide_and_asks_only_for_the_identity_marker() {
        assert_eq!(
            list_panes_args(&named("work"), "my-feature"),
            [
                "-L",
                "work",
                "list-panes",
                "-s",
                "-t",
                "my-feature",
                "-F",
                "#{@ae_slot}"
            ]
        );
    }

    #[test]
    fn the_pane_format_asks_for_identity_and_not_for_the_unratified_live_predicate() {
        // `@ae_agent` is DISPLAY (SC-602), and the frozen script associated on it.
        // `pane_current_command` is what a live predicate would need, and no row
        // defines one — carrying it here would be building toward an undecided
        // thing, which is a decision wearing the shape of a format string.
        let args = list_panes_args(&ServerId::Ambient, "s").join(" ");
        assert!(args.contains("#{@ae_slot}"));
        assert!(!args.contains("@ae_agent"), "{args}");
        assert!(!args.contains("pane_current_command"), "{args}");
    }

    #[test]
    fn a_failed_pane_query_is_a_failure_whatever_it_printed() {
        for payload in ["", "main\n", "can't find window: nosuch\n"] {
            assert_eq!(
                interpret_panes(false, payload),
                Err(QueryFailed),
                "{payload:?}"
            );
        }
    }

    #[test]
    fn an_unmarked_pane_is_a_pane_and_not_a_dropped_line() {
        // MEASURED against a real server: three panes whose middle one has no
        // marker print `main\n\nworker\n`. Dropping the blank would delete the
        // pane whose existence is what keeps a missing roster agent `unknown`.
        assert_eq!(
            interpret_panes(true, "main\n\nworker\n"),
            Ok(vec![pane(Some("main")), pane(None), pane(Some("worker"))]),
            "the blank line is the unmarked pane"
        );
        assert_eq!(
            interpret_panes(true, "\n"),
            Ok(vec![pane(None)]),
            "a lone blank line is one unmarked pane, not zero panes"
        );
        assert_eq!(
            interpret_panes(true, ""),
            Ok(Vec::new()),
            "and no output at all is no panes"
        );
    }

    #[test]
    fn the_two_interpreters_disagree_about_a_blank_line_on_purpose() {
        // The same bytes, opposite readings, and both are right: a session
        // called nothing does not exist, an unmarked pane does. A shared filter
        // would be the bug.
        assert_eq!(
            interpret_sessions(true, "a\n\nb\n"),
            Ok(vec!["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(
            interpret_panes(true, "a\n\nb\n").map(|panes| panes.len()),
            Ok(3)
        );
    }

    #[test]
    fn a_marker_that_is_empty_or_blank_is_not_a_usable_identity() {
        // WHAT THIS CAN PIN, AND WHAT IT CANNOT. That tmux reports unset and
        // set-to-empty IDENTICALLY is a fact about tmux, and no assertion over
        // this parser can establish it. An earlier version of this test tried,
        // by comparing this function's output to ITSELF — a test that cannot
        // fail, which is the one defect every other test in this file exists to
        // prevent. The platform fact is now proven where it is provable:
        // `sc_017p_a_real_enumeration_...` enumerates a real server twice, once
        // with a pane's option unset and once with it set to the empty string,
        // and requires the two answers to be identical.
        //
        // What IS this parser's to answer: neither spelling is a usable slot.
        assert_eq!(interpret_panes(true, "\n"), Ok(vec![pane(None)]));
        assert_eq!(
            interpret_panes(true, "  \n"),
            Ok(vec![pane(None)]),
            "whitespace is not an identity"
        );
        assert_eq!(
            interpret_panes(true, "main\n\n"),
            Ok(vec![pane(Some("main")), pane(None)]),
            "and a trailing blank is the last pane, not a trailing nothing"
        );
    }

    #[test]
    fn an_empty_roster_slot_matches_no_pane() {
        // A ROSTER SLOT CAN BE EMPTY: `absorb_roster` validates alias and name
        // and never the slot, so `agent.=cl:lead` in a hand-edited meta yields a
        // roster entry whose slot is "".
        //
        // It must associate to nothing, and today it does — but only because
        // `interpret_panes` normalizes an empty marker to `None`, so no pane
        // ever carries `Some("")`. THAT CORRECTNESS LIVES IN THE RELATION
        // BETWEEN TWO FUNCTIONS AND IN NEITHER OF THEM, which is invisible to a
        // review that reads either alone. Remove the normalization — it looks
        // like defensive noise the moment you forget that tmux reports unset and
        // set-to-empty identically — and an empty slot matches EVERY unmarked
        // pane: a corrupt roster entry reading its health off somebody else's
        // pane, which SC-017p forbids by name.
        //
        // NAMED FOR THE FACT, NOT THE FILTER. A test named after the guard gets
        // deleted by whoever deletes the guard, in the same breath, believing
        // they are tidying.
        //
        // THE PANES COME FROM `interpret_panes`, NOT FROM THE HELPER, and that
        // is the whole test. A first draft built `pane(None)` by hand — which
        // constructs the POST-NORMALIZATION value, so deleting the very filter
        // this test exists to guard leaves it green. A fixture that builds the
        // conclusion cannot observe the step that produces it. Caught in review
        // by grok46; kept as a comment because the wrong version looked right.
        let panes = interpret_panes(true, "\nmain\n").expect("a successful enumeration");
        assert_eq!(
            slot_observation(&panes, ""),
            SlotObservation::Absent { unidentified: 1 },
            "an empty roster slot matches no pane, including the unmarked one"
        );
    }

    #[test]
    fn a_slot_is_found_only_by_exact_match() {
        let panes = [pane(Some("main")), pane(Some("worker"))];
        assert_eq!(slot_observation(&panes, "main"), SlotObservation::Unique);
        assert_eq!(
            slot_observation(&panes, "mai"),
            SlotObservation::Absent { unidentified: 0 },
            "a PREFIX of a slot that is there is absent, not present"
        );
        assert_eq!(
            slot_observation(&panes, "main2"),
            SlotObservation::Absent { unidentified: 0 },
            "and so is a slot that merely extends one"
        );
    }

    #[test]
    fn a_duplicated_slot_is_ambiguous_rather_than_a_match() {
        let panes = [pane(Some("main")), pane(Some("main"))];
        assert_eq!(
            slot_observation(&panes, "main"),
            SlotObservation::Duplicated { panes: 2 },
            "two panes claiming one slot associate it to neither"
        );
    }

    #[test]
    fn absence_carries_how_many_panes_identified_nothing() {
        // The COUNT, not a conclusion. Whether zero unidentified panes proves a
        // roster agent absent is SC-017p's reading and is made where liveness is
        // decided; this reports what was seen.
        assert_eq!(
            slot_observation(&[pane(Some("other"))], "main"),
            SlotObservation::Absent { unidentified: 0 }
        );
        assert_eq!(
            slot_observation(&[pane(Some("other")), pane(None)], "main"),
            SlotObservation::Absent { unidentified: 1 },
            "an unassociated pane is exactly the fact SC-017q needs"
        );
        assert_eq!(
            slot_observation(&[], "main"),
            SlotObservation::Absent { unidentified: 0 },
            "no panes at all identifies nothing and hides nothing"
        );
    }

    #[test]
    fn a_relative_socket_path_cannot_address_a_server() {
        assert!(is_addressable_socket(Path::new("/tmp/ae.sock")));
        assert!(!is_addressable_socket(Path::new("relative/ae.sock")));
    }
}
