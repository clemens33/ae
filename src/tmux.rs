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
//! * SC-017p/SC-017q/SC-017s — the DERIVATION AND THE READING ARE HERE NOW
//!   ([`list_panes_args`], [`interpret_panes`], [`slot_observation`]).
//!   Association is SC-602's `@ae_slot`; ambiguity is representable rather than
//!   collapsed; and both of SC-017s's conjuncts — `#{pane_dead}` and
//!   `#{pane_current_command}` — are carried out of the read intact.
//!
//!   **THE PREDICATE IS RATIFIED.** An earlier version of this handover said no
//!   ratified row defined "positively recognizes its agent process as live", and
//!   told the next seat the work was blocked. SC-017s supplies it: the pane
//!   proves `alive` iff `pane_dead` is `0` AND the command is outside the closed
//!   shell set `bash`/`zsh`/`fish`/`sh`/`dash` AND the empty string. That
//!   sentence was true when written and expired without anyone touching this
//!   file — which is the argument for saying what a module OBSERVES rather than
//!   what the world happens to lack.
//!
//!   **WHAT IS STILL OPEN, and it is not the predicate:**
//!   * the PRODUCT ROUTE from this observation to an `alive` verdict — argv,
//!     interpretation and association are here, and nothing wires them to a
//!     status. That sequencing is a separate decision.
//!   * SC-017p's `dead` half. SC-017s grants `alive` ONLY: a shell foreground
//!     proves nothing and leaves the agent `unknown` (SC-017q). The watchdog's
//!     dead test is a CONJUNCTION of shell-foreground and no-agent-descendant,
//!     and negating one conjunct is sound in one direction only.
//!   * the process-inspection capability, which stays RESERVED and unused.
//!     SC-017s observes tmux format fields and asserts nothing about processes
//!     or ancestry; a symmetric dead predicate would re-import the unratified
//!     SC-906 and the ancestry observation with it. Do not reach for `pgrep` or
//!     a parent walk here — if a change seems to need one, that is the signal to
//!     stop, not to add it.
//!
//!   A known FALSE NEGATIVE is recorded in SC-017s rather than fixed: under
//!   SC-812 a `cmd || fallback` resume chain leaves bash as the pane process, so
//!   a genuinely live agent reports `bash` and lands in `unknown`.
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

/// The format `list-panes` is asked for: three fields per pane, tab-separated.
///
/// **`@ae_slot`, not `@ae_agent`** — SC-602 rules that the slot option carries
/// IDENTITY and `@ae_agent` is display. The frozen script associated on the
/// display field (`cmd_list`'s alive map is keyed on `#{@ae_agent}`, ae:4207),
/// which is a second defect in the code SC-017p/q/r already indict (#106).
///
/// **`pane_dead` and `pane_current_command` are SC-017s's fields**, and that row
/// is why they are here: it ratifies the only route to `alive`, reading exactly
/// these two beside the identity marker, in one query that was already being
/// made. They were deliberately ABSENT while the predicate was unratified —
/// carrying them then would have been a decision wearing the shape of a format
/// string. The row made the decision; the field list follows it.
///
/// **`pane_dead` IS FIRST, and the order is a safety argument rather than
/// taste.** It is the conjunct whose loss produces a FALSE ALIVE: measured on a
/// real server, a `remain-on-exit` pane whose process has exited reports
/// `pane_dead=1` with `pane_current_command=true`, and `true` is not in the
/// shell set — so the command field ALONE proves a dead agent alive, which is
/// #109. Putting it before every ae- or system-controlled field means nothing
/// upstream can shift it out of position.
pub const PANE_FORMAT: &str = "#{pane_dead}\t#{@ae_slot}\t#{pane_current_command}";

/// How many tab-separated fields [`PANE_FORMAT`] produces per pane.
pub const PANE_FIELDS: usize = 3;

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
    args.push(PANE_FORMAT.to_owned());
    args
}

/// One pane the server reported, as three readings.
///
/// A pane with no usable reading is still A PANE. Every line of a successful
/// enumeration becomes one of these, whatever it contained — dropping a line
/// would delete exactly the evidence SC-017q needs to refuse a `dead`.
///
/// NO VERDICT IS COMPUTED HERE. SC-017s's predicate (`pane_dead` is `0` AND the
/// command is outside the closed shell set, the empty string included) is
/// deliberately NOT applied in this module: the route from observation to an
/// `alive` verdict is a separate, unsequenced decision. What this type
/// guarantees is that both conjuncts SURVIVE the read, so no downstream
/// predicate can be forced to guess one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedPane {
    /// `#{pane_dead}` — `Some(true)` for a dead pane, `Some(false)` for a live
    /// one, `None` when the field was not a readable `0`/`1`.
    ///
    /// SC-017s's first conjunct, and the one that stops #109: an exited pane
    /// retained by `remain-on-exit` keeps reporting the exited process's
    /// command, so the command field alone would read `alive` for a dead agent.
    pub dead: Option<bool>,
    /// The `@ae_slot` value, when the pane carries a usable one.
    ///
    /// `None` covers unset AND set-to-empty, which are byte-identical in a
    /// format expansion (measured), so ae does not pretend to tell apart two
    /// states tmux reports identically.
    pub slot: Option<String>,
    /// `#{pane_current_command}`, when the field carried one.
    ///
    /// `None` for an empty reading. SC-017s puts the empty string IN the
    /// not-alive set for the reason this crate keeps rediscovering: an
    /// unreadable field is the absence of evidence, and reading absence as
    /// positive proof is #105 itself. The frozen script's set omitted it
    /// (ae:4201-4206 versus `command_is_shell` at ae:428-434), so an absent
    /// reading fell to the non-shell arm and yielded a positive alive.
    pub command: Option<String>,
}

/// What a completed `list-panes` run means.
///
/// # Errors
///
/// [`QueryFailed`] whenever the run did not succeed, whatever it printed —
/// SC-017q sends a failed pane query to `unknown`, exactly as SC-017l does for
/// sessions. Measured: a `-t` naming no session exits 1.
///
/// **EVERY LINE IS A PANE.** A line that does not split into exactly
/// [`PANE_FIELDS`] fields still yields an [`ObservedPane`] — one with no usable
/// reading at all — rather than being dropped. Dropping it would delete the
/// pane whose existence is what keeps a missing roster agent `unknown` instead
/// of `dead`, which is the same defect as the frozen script's
/// `[[ -n "$ae_agent" ]] || continue` (ae:4202, #107), and the same one this
/// module already refused when the format had a single field.
///
/// **ARITY IS EXACT, AND THAT IS A GUARD RATHER THAN TIDINESS.** None of the
/// three fields may legitimately contain a tab, so a line with more than
/// [`PANE_FIELDS`] fields is a reading nothing should trust. It matters because
/// a slot carrying an embedded tab could otherwise split into a PREFIX that
/// matches a real roster slot while pushing the rest of that slot into the
/// command field — forging a non-shell command for a pane that is running a
/// shell, which is a fabricated `alive` for the wrong agent. Refusing the whole
/// line answers `unknown` instead (SC-017q), which is the direction that cannot
/// assert.
pub fn interpret_panes(succeeded: bool, stdout: &str) -> Result<Vec<ObservedPane>, QueryFailed> {
    if !succeeded {
        return Err(QueryFailed);
    }
    Ok(stdout.lines().map(read_pane).collect())
}

/// One enumeration line as an [`ObservedPane`].
///
/// Unreadable in any respect yields the all-`None` pane: present, and saying
/// nothing. Each field is independent — an unreadable command does not discard
/// a perfectly good `pane_dead`.
fn read_pane(line: &str) -> ObservedPane {
    let blank = ObservedPane {
        dead: None,
        slot: None,
        command: None,
    };
    let fields: Vec<&str> = line.trim_end_matches('\r').split('\t').collect();
    if fields.len() != PANE_FIELDS {
        return blank;
    }
    let usable = |value: &str| Some(value.to_owned()).filter(|v| !v.is_empty());
    ObservedPane {
        // `0`/`1` and nothing else. A field that is neither is not a reading
        // saying "alive" — it is no reading, and SC-017s refuses to build a
        // positive proof on one.
        dead: match fields[0] {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        },
        slot: usable(fields[1].trim_end()),
        command: usable(fields[2].trim_end()),
    }
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

/// The format the viewer query asks for: the calling pane's routing slot, its
/// tmux session and its display ref — the three readings `ae_current_slot`,
/// `#S` and `ae_current_agent` take in the frozen helper, in ONE round trip
/// rather than three, so the pane cannot change identity between them.
///
/// Tab-separated. None of the three may contain a tab: slots are a closed
/// grammar, session and agent names are ASCII allowlists. An unset user option
/// expands to the empty string (measured), which is why the interpreter treats
/// empty and unset alike.
pub const VIEWER_FORMAT: &str = "#{@ae_slot}\t#{session_name}\t#{@ae_agent}";

/// The number of fields [`VIEWER_FORMAT`] yields.
const VIEWER_FIELDS: usize = 3;

/// The arguments that read [`VIEWER_FORMAT`] off `pane` on `server`.
///
/// `display-message -p` prints the expansion instead of showing it, and `-t`
/// names the pane whose options and session are expanded. There is no
/// no-target form: a query that let tmux pick "the current pane" would answer
/// with whichever pane the server last touched, which the frozen helper does
/// and which is a misattribution rather than an identity.
#[must_use]
pub fn viewer_args(server: &ServerId, pane: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", "-t", pane, VIEWER_FORMAT].map(ToOwned::to_owned));
    args
}

/// The calling pane's three identity readings. `None` is unset-or-empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObservedViewer {
    /// `@ae_slot` — the routing key, unvalidated here.
    pub slot: Option<String>,
    /// `#{session_name}` of the pane.
    pub session: Option<String>,
    /// `@ae_agent` — the display `alias:name`.
    pub agent: Option<String>,
}

/// What a completed viewer query means.
///
/// A failed run is no identity (`None`): a pane that does not exist, a server
/// that is not there, or a `tmux` that could not be spawned all leave the
/// caller unidentified, and the requests surface then refuses `mine`/`inbox`
/// the way the frozen helper does outside a pane.
///
/// **Exactly one record.** `display-message -p` prints one expansion and one
/// `\n`; stdout is that line with at most its terminating `\n`, and it must
/// split into exactly [`VIEWER_FIELDS`] fields. Anything beyond — a second
/// line, an embedded newline in a user option somebody set by hand — is a
/// reading nothing should trust, for the same reason [`interpret_panes`]
/// refuses odd arity: taking "the first line" would let injected content
/// choose which record is read.
#[must_use]
pub fn interpret_viewer(succeeded: bool, stdout: &str) -> Option<ObservedViewer> {
    if !succeeded {
        return None;
    }
    let line = stdout.strip_suffix('\n').unwrap_or(stdout);
    if line.contains('\n') {
        return None;
    }
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != VIEWER_FIELDS {
        return None;
    }
    let reading = |field: &str| (!field.is_empty()).then(|| field.to_owned());
    Some(ObservedViewer {
        slot: reading(fields[0]),
        session: reading(fields[1]),
        agent: reading(fields[2]),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_viewer_query_addresses_the_pane_and_asks_for_the_three_readings() {
        use super::{ObservedViewer, VIEWER_FORMAT, interpret_viewer, viewer_args};
        use crate::inventory::ServerId;
        assert_eq!(
            viewer_args(&ServerId::Ambient, "%7"),
            ["display-message", "-p", "-t", "%7", VIEWER_FORMAT]
        );
        // A stamped agent pane.
        assert_eq!(
            interpret_viewer(true, "main\taerewrite\tcl:lead\n"),
            Some(ObservedViewer {
                slot: Some("main".to_owned()),
                session: Some("aerewrite".to_owned()),
                agent: Some("cl:lead".to_owned()),
            })
        );
        // An unstamped pane: unset options expand to empty, and empty is None.
        assert_eq!(
            interpret_viewer(true, "\taerewrite\t\n"),
            Some(ObservedViewer {
                slot: None,
                session: Some("aerewrite".to_owned()),
                agent: None,
            })
        );
        // A failed run, a short line and a long line are all no identity.
        assert_eq!(interpret_viewer(false, "main\ts\ta:b\n"), None);
        // MORE THAN ONE RECORD is no identity either: a second line, however
        // well-formed the first, is content the query never asked for, and
        // reading the first would let it pick the record.
        assert_eq!(
            interpret_viewer(true, "main\ts\ta:b\nworker.0\ts\ta:c\n"),
            None
        );
        assert_eq!(interpret_viewer(true, "main\ts\ta:b\n\n"), None);
        assert_eq!(interpret_viewer(true, "main\ts\ta:b\nx"), None);
        // One record with or without its terminating newline is the same record.
        assert_eq!(
            interpret_viewer(true, "main\ts\ta:b"),
            interpret_viewer(true, "main\ts\ta:b\n")
        );
        assert_eq!(interpret_viewer(true, "main\ts\n"), None);
        assert_eq!(interpret_viewer(true, "main\ts\ta:b\textra\n"), None);
        assert_eq!(interpret_viewer(true, ""), None);
    }

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

    /// A pane reading, spelled the way the fields arrive.
    fn pane(dead: Option<bool>, slot: Option<&str>, command: Option<&str>) -> ObservedPane {
        ObservedPane {
            dead,
            slot: slot.map(ToOwned::to_owned),
            command: command.map(ToOwned::to_owned),
        }
    }

    /// A pane with a usable slot and nothing else said about it.
    fn slotted(slot: &str) -> ObservedPane {
        pane(Some(false), Some(slot), Some("claude"))
    }

    /// A pane carrying no usable identity.
    fn unslotted() -> ObservedPane {
        pane(Some(false), None, Some("zsh"))
    }

    #[test]
    fn pane_enumeration_is_session_wide_and_asks_for_sc_017s_s_three_fields() {
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
                "#{pane_dead}\t#{@ae_slot}\t#{pane_current_command}"
            ]
        );
    }

    #[test]
    fn the_pane_format_asks_for_identity_and_never_for_the_display_field() {
        // `@ae_agent` is DISPLAY (SC-602) and the frozen script associated on it
        // (#106). The two SC-017s fields ARE here now — the row that ratified
        // the predicate is what put them here, and their absence beforehand was
        // the same decision in the other direction.
        let args = list_panes_args(&ServerId::Ambient, "s").join(" ");
        assert!(args.contains("#{@ae_slot}"));
        assert!(args.contains("#{pane_dead}"), "{args}");
        assert!(args.contains("#{pane_current_command}"), "{args}");
        assert!(!args.contains("@ae_agent"), "{args}");
    }

    #[test]
    fn pane_dead_comes_first_so_nothing_upstream_can_shift_it() {
        // ORDER IS THE SAFETY PROPERTY. `pane_dead` is the conjunct whose loss
        // fabricates an `alive`; no ae- or system-controlled field precedes it.
        let fields: Vec<&str> = super::PANE_FORMAT.split('\t').collect();
        assert_eq!(fields.len(), super::PANE_FIELDS);
        assert_eq!(fields[0], "#{pane_dead}");
        assert_eq!(fields[2], "#{pane_current_command}", "free-est text last");
    }

    #[test]
    fn a_failed_pane_query_is_a_failure_whatever_it_printed() {
        for payload in ["", "0\tmain\tclaude\n", "can't find window: nosuch\n"] {
            assert_eq!(
                interpret_panes(false, payload),
                Err(QueryFailed),
                "{payload:?}"
            );
        }
    }

    #[test]
    fn an_unmarked_pane_is_a_pane_and_not_a_dropped_line() {
        // MEASURED against a real server. A three-pane session whose middle pane
        // carries no marker prints `0\tmain\tzsh\n0\t\tzsh\n1\t\ttrue\n`:
        // the unmarked pane is an EMPTY MIDDLE FIELD, not a missing line.
        assert_eq!(
            interpret_panes(true, "0\tmain\tzsh\n0\t\tzsh\n1\t\ttrue\n"),
            Ok(vec![
                pane(Some(false), Some("main"), Some("zsh")),
                pane(Some(false), None, Some("zsh")),
                pane(Some(true), None, Some("true")),
            ]),
            "three lines, three panes, and the unmarked one is still one"
        );
        assert_eq!(
            interpret_panes(true, ""),
            Ok(Vec::new()),
            "no output at all is no panes"
        );
    }

    #[test]
    fn an_exited_pane_keeps_both_facts_that_tell_it_apart_from_a_live_one() {
        // #109 IN ONE ASSERTION. A `remain-on-exit` pane reports the EXITED
        // process's command, and `true` is not in SC-017s's shell set — so the
        // command field alone reads like a live agent. The only thing that
        // separates it from a live pane is `pane_dead`, and this pins that the
        // read carries it rather than discarding it.
        //
        // Measured, not invented: `1\t\ttrue` is real output from a real
        // server whose pane ran `true` under `remain-on-exit on`.
        let exited = interpret_panes(true, "1\tworker\ttrue\n").expect("success");
        assert_eq!(exited, vec![pane(Some(true), Some("worker"), Some("true"))]);
        assert_eq!(
            exited[0].dead,
            Some(true),
            "the conjunct that stops a dead agent reading alive survived the read"
        );
        assert_eq!(
            exited[0].command.as_deref(),
            Some("true"),
            "and so did the command that would otherwise have proven it alive"
        );
    }

    #[test]
    fn a_marker_that_is_empty_or_blank_is_not_a_usable_identity() {
        // RESTORED BY NAME, and not merely as bookkeeping. This name vanished in
        // the three-field rewrite; its EMPTY-field half survived inside
        // `an_unreadable_field_...`, but the WHITESPACE-ONLY field had no
        // assertion anywhere, which panereview found by reading for the input
        // rather than for the name. A vanished name is worth restoring only when
        // something it covered is actually uncovered — here something was.
        //
        // Both spellings must reach the same answer: a slot is either usable or
        // it is absent, and "present but blank" is not a third thing. The
        // trailing-trim is what makes the whitespace case land, so the case is
        // the guard on the trim.
        assert_eq!(
            interpret_panes(true, "0\t\tzsh\n"),
            Ok(vec![pane(Some(false), None, Some("zsh"))]),
            "an empty slot field is no identity"
        );
        assert_eq!(
            interpret_panes(true, "0\t   \tzsh\n"),
            Ok(vec![pane(Some(false), None, Some("zsh"))]),
            "and neither is a whitespace-only one — same answer, different bytes"
        );
        assert_eq!(
            interpret_panes(true, "0\tmain\t   \n"),
            Ok(vec![pane(Some(false), Some("main"), None)]),
            "the command field normalizes the same way, and SC-017s puts an \
             unreadable command in the not-alive set for the same reason"
        );
    }

    #[test]
    fn an_unreadable_field_is_no_reading_rather_than_a_convenient_one() {
        // SC-017s: an empty or absent reading is NOT alive, because absence of
        // evidence is not evidence. Each field fails independently.
        assert_eq!(
            interpret_panes(true, "0\tmain\t\n"),
            Ok(vec![pane(Some(false), Some("main"), None)]),
            "an empty command is no command, not a non-shell one"
        );
        assert_eq!(
            interpret_panes(true, "\tmain\tclaude\n"),
            Ok(vec![pane(None, Some("main"), Some("claude"))]),
            "an empty pane_dead is no reading, and must not pass for `0`"
        );
        assert_eq!(
            interpret_panes(true, "2\tmain\tclaude\n"),
            Ok(vec![pane(None, Some("main"), Some("claude"))]),
            "and neither does anything else that is not 0 or 1"
        );
        assert_eq!(
            interpret_panes(true, "0\t\tclaude\n"),
            Ok(vec![pane(Some(false), None, Some("claude"))]),
            "an empty slot is no identity; the other two readings survive it"
        );
    }

    #[test]
    fn a_line_of_the_wrong_arity_is_a_pane_that_says_nothing() {
        // A TAB CANNOT BE SMUGGLED THROUGH A FIELD. If a slot carried one, the
        // line would split into four: the prefix could match a real roster slot
        // while the remainder was pushed into the command field, forging a
        // non-shell command for a pane running a shell — a fabricated `alive`
        // attached to the wrong agent. Refusing the line answers `unknown`.
        let forged = interpret_panes(true, "0\tmain\tevil\tzsh\n").expect("success");
        assert_eq!(
            forged,
            vec![pane(None, None, None)],
            "still one pane, and it says nothing at all"
        );
        assert_eq!(
            interpret_panes(true, "\n"),
            Ok(vec![pane(None, None, None)]),
            "a blank line is a pane with no reading, never a dropped pane"
        );
        assert_eq!(
            interpret_panes(true, "0\tmain\n"),
            Ok(vec![pane(None, None, None)]),
            "and too few fields is refused for the same reason as too many"
        );
    }

    #[test]
    fn the_two_interpreters_disagree_about_a_blank_line_on_purpose() {
        // A session called nothing does not exist; a pane that reported nothing
        // does. A shared filter would be the bug.
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
    fn an_empty_roster_slot_matches_no_pane() {
        // A ROSTER SLOT CAN BE EMPTY: `absorb_roster` validates alias and name
        // and never the slot, so `agent.=cl:lead` in a hand-edited meta yields a
        // roster entry whose slot is "".
        //
        // It must associate to nothing, and today it does — but only because
        // `read_pane` normalizes an empty field to `None`, so no pane ever
        // carries `Some("")`. THAT CORRECTNESS LIVES IN THE RELATION BETWEEN TWO
        // FUNCTIONS AND IN NEITHER OF THEM, which is invisible to a review that
        // reads either alone. Remove the normalization and an empty slot matches
        // EVERY unmarked pane: a corrupt roster entry reading its health off
        // somebody else's pane, which SC-017p forbids by name.
        //
        // NAMED FOR THE FACT, NOT THE FILTER — a test named after the guard gets
        // deleted by whoever deletes the guard, believing they are tidying.
        //
        // THE PANES COME FROM `interpret_panes`, NOT FROM THE HELPER. A first
        // draft built the post-normalization value by hand and stayed green
        // under the very deletion it existed to catch; grok46 caught that in
        // review. A fixture that builds the conclusion cannot observe the step
        // that produces it.
        //
        // DELETED AND RESTORED ONCE: a block rewrite for SC-017s's three-field
        // format swallowed this test whole, and only a test-NAME diff against
        // HEAD found it. A count that holds while a name vanishes is the shape
        // that hides a deletion.
        let panes =
            interpret_panes(true, "0\t\tzsh\n0\tmain\tclaude\n").expect("a successful enumeration");
        assert_eq!(
            slot_observation(&panes, ""),
            SlotObservation::Absent { unidentified: 1 },
            "an empty roster slot matches no pane, including the unmarked one"
        );
    }

    #[test]
    fn a_slot_is_found_only_by_exact_match() {
        let panes = [slotted("main"), slotted("worker")];
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
        let panes = [slotted("main"), slotted("main")];
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
            slot_observation(&[slotted("other")], "main"),
            SlotObservation::Absent { unidentified: 0 }
        );
        assert_eq!(
            slot_observation(&[slotted("other"), unslotted()], "main"),
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
