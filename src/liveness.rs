//! Whether a candidate is running — and what ae says when it cannot tell.
//!
//! **SC-017k** — a durable candidate is `running` only when a SUCCESSFUL query of
//! its own recorded server returns the EXACT session name with positive
//! ae-ownership; it is `stopped` only when that same successful query proves the
//! exact name absent. An unreachable, missing or ambiguous recorded server, a
//! failed query, or an exact live name carrying no ownership tag yields
//! `unknown`, never `stopped` and never absence.
//!
//! **Ownership is marker PRESENCE, not a name match** — see
//! [`positively_owned`], which carries the measurement that changed it. A tag
//! whose value differs from the session name is still a tag; identity is settled
//! by the exact name match against the session's own recorded server, which is a
//! stronger check than re-reading it out of a variable that never carried it.
//!
//! # What is NOT a liveness fact
//!
//! Ambient-server membership. Prefix success. Which renderer block printed the
//! row. All three were how #105 answered the question, and none of them survives
//! into this module: the classifier takes a candidate's own typed selector, asks
//! that server, and matches the exact name. There is no ambient fallback to
//! write, because nothing here knows which server is ambient.
//!
//! # The snapshot proof, and why it is never re-queried
//!
//! A candidate that phase 1 saw ALIVE — tmux-only, or dual-provenance where
//! SC-017j coalesced a positively owned exact-name sighting from that
//! candidate's own recorded server — is `running` on that discovery fact alone.
//! SC-017k says so for both shapes, and the reason is **monotonicity**: a later
//! or redundant query may not retroactively replace an accepted snapshot fact,
//! or adding a durable record to a session ae just watched answer would let a
//! redundant failure turn it `unknown`. Learning more would reduce what is
//! known.
//!
//! That is held by construction here — a candidate with a positively owned
//! sighting is never put in the query set at all, so there is no code path on
//! which a second answer could overwrite the first.
//!
//! # Ownership: the phase-2 half of a question phase 1 deferred
//!
//! Phase 1 admits a live session to the inventory on marker PRESENCE, and said
//! why in [`crate::inventory::DiscoveredSession::marker`]: guessing stricter
//! there could only REMOVE a session from the inventory. Phase 2 now applies the
//! SAME test rather than a stricter one — the two phases agreeing is the point.
//! The stricter phase-2 rule was not a safer version of this one; it was a
//! different question (identity) asked of evidence that does not answer it, and
//! it made `running` unreachable for every session ae has ever created.
//!
//! # No rediscovery
//!
//! This module reads no file and enumerates no root. Every durable fact it uses
//! — the typed selector, the read outcome, the provenance — was established by
//! phase 1 and arrives in the [`Inventory`]. The only I/O it can reach is the
//! backend query, through the same [`Discovery`] port phase 1 used, so it cannot
//! contact a server phase 1 was not entitled to ask.

use crate::attention::Reason;
use crate::digest::Status;
use crate::inventory::{
    Candidate, DiscoveredSession, Discovery, FailedSource, Inventory, QueryFailed, ServerId,
};
use crate::meta::ServerSelector;
use crate::session::AgentRuntime;
use crate::tmux::{ObservedPane, SlotObservation, slot_observation};

/// One candidate, with its liveness decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    /// The candidate exactly as phase 1 established it.
    ///
    /// Kept rather than consumed: SC-509's digest still needs the identity, and
    /// phase 3 needs the name. Nothing here mutates it — a classifier that
    /// rewrote its own input would make the phase boundary unobservable.
    pub candidate: Candidate,
    /// What ae knows about whether it is running.
    pub status: Status,
}

/// A classified snapshot: every phase-1 identity, and what was lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// One entry per phase-1 candidate. Same identities, same count.
    pub sessions: Vec<Classified>,
    /// SC-017o's loss facts, carried through unchanged.
    ///
    /// Classification neither clears nor recomputes these: they are facts about
    /// ENUMERATION, decided before this phase ran, and an incomplete snapshot is
    /// not a session-level condition. Mapping one to `unknown` would turn "I
    /// could not look everywhere" into a claim about a particular session.
    pub incomplete: Vec<FailedSource>,
}

impl Snapshot {
    /// Whether every SC-017j enumeration completed — SC-017o's `inventory_complete`.
    #[must_use]
    pub const fn complete(&self) -> bool {
        self.incomplete.is_empty()
    }
}

/// Whether `marker` is positive ae-ownership evidence for a session called
/// `name`.
///
/// The marker answers ONE question — "is this tmux session ae's?" — and its
/// presence is the answer. It is a tag, not a name: the real producer writes
/// `AE_SESSION=1`, so a rule requiring the value to equal the session name can
/// never be satisfied by any session ae has ever created, and every session
/// classifies `unknown` for want of evidence that exists. Measured on a live
/// machine 2026-08-26: five real sessions, all tagged, all `unknown`, the whole
/// status column dead.
///
/// The identity question the old rule was reaching for is already answered
/// upstream and better: the caller matched this session by EXACT name against
/// its OWN recorded server's `list-sessions`. Re-deriving identity from a tag
/// that does not carry it added no safety and cost the entire feature.
///
/// `name` is retained because ownership is asked ABOUT a session and a future
/// marker grammar may carry identity; it is deliberately unused today.
#[must_use]
pub fn positively_owned(name: &str, marker: Option<&str>) -> bool {
    let _ = name;
    marker.is_some_and(|marker| !marker.is_empty())
}

/// The snapshot fact a candidate already carries, if it is proof of life.
///
/// SC-017k's two reuse cases are one predicate: a positively owned exact-name
/// sighting IS the successful own-server query for this snapshot, whether or not
/// a durable record was later matched to it.
fn already_proven_running(candidate: &Candidate) -> bool {
    candidate.live.as_ref().is_some_and(|live| {
        live.name == candidate.name && positively_owned(&live.name, Some(&live.marker))
    })
}

/// Classify every candidate in `inventory`, asking `backend` only about the
/// servers that still owe an answer.
///
/// Total and cardinality-preserving: one classified entry per input candidate,
/// in input order, with no identity added, dropped, merged, split or rewritten.
/// A backend failure decides one candidate's status and never the set's fate.
///
/// Grouping is by recorded server, so N candidates on one server cost one query
/// — SC-017k permits exactly that, and requires what this preserves: every
/// candidate's answer still comes from its own server and its own exact name.
///
/// ```
/// use ae::digest::Status;
/// use ae::inventory::{DurableScan, Inventory, ServerId, take};
/// use ae::liveness::classify;
///
/// # struct Nothing;
/// # impl ae::inventory::Discovery for Nothing {
/// #     fn enumerate(&self, _: &ServerId)
/// #         -> Result<Vec<ae::inventory::DiscoveredSession>, ae::inventory::QueryFailed>
/// #     { Ok(Vec::new()) }
/// # }
/// let inventory = take(DurableScan::default(), None, &Nothing);
/// let snapshot = classify(inventory, &Nothing);
/// assert!(snapshot.sessions.is_empty());
/// assert!(snapshot.complete(), "an empty snapshot still answers the question");
/// ```
#[must_use]
pub fn classify<D: Discovery + ?Sized>(inventory: Inventory, backend: &D) -> Snapshot {
    let answers = Answers::gather(&inventory.candidates, backend);
    let sessions = inventory
        .candidates
        .into_iter()
        .map(|candidate| {
            let status = decide(&candidate, &answers);
            Classified { candidate, status }
        })
        .collect();
    Snapshot {
        sessions,
        // Carried, never recomputed: see the field's own docs.
        incomplete: inventory.incomplete,
    }
}

/// What each queried server said, asked once per distinct server.
struct Answers(Vec<(ServerId, Result<Vec<DiscoveredSession>, QueryFailed>)>);

impl Answers {
    /// Query every server that some candidate still needs an answer from.
    ///
    /// A candidate already carrying a snapshot proof contributes no server here,
    /// which is what makes the monotonicity rule structural rather than a check:
    /// its answer cannot be replaced by a query that is never made.
    fn gather<D: Discovery + ?Sized>(candidates: &[Candidate], backend: &D) -> Self {
        let mut answers: Vec<(ServerId, Result<Vec<DiscoveredSession>, QueryFailed>)> = Vec::new();
        for candidate in candidates {
            if already_proven_running(candidate) {
                continue;
            }
            let Some(server) = candidate
                .durable
                .as_ref()
                .and_then(|record| record.server.entitles())
                .map(|selector| ServerId::Selected(selector.clone()))
            else {
                continue;
            };
            if answers.iter().any(|(known, _)| *known == server) {
                continue;
            }
            let answer = backend.enumerate(&server);
            answers.push((server, answer));
        }
        Self(answers)
    }

    /// What `server` said, if it was asked.
    fn get(&self, server: &ServerId) -> Option<&Result<Vec<DiscoveredSession>, QueryFailed>> {
        self.0
            .iter()
            .find(|(known, _)| known == server)
            .map(|(_, answer)| answer)
    }
}

/// The status of one candidate, given what its own server said.
fn decide(candidate: &Candidate, answers: &Answers) -> Status {
    // SC-017k's snapshot proof, for tmux-only and dual-provenance alike.
    if already_proven_running(candidate) {
        return Status::Running;
    }
    let Some(record) = &candidate.durable else {
        // Live-only, and the sighting was not positive proof — a marker that is
        // missing or names another session. There is no recorded server to ask,
        // so nothing can be established either way. SC-017l: unknown.
        return Status::Unknown;
    };
    let selector = match &record.server {
        ServerSelector::Positive(selector) => selector,
        // SC-405l missing or ambiguous. No target is derived from those bytes —
        // an ambiguous pointer is not a pointer, and guessing one would query a
        // server ae was never entitled to ask.
        ServerSelector::Missing | ServerSelector::Ambiguous => return Status::Unknown,
    };
    let server = ServerId::Selected(selector.clone());
    match answers.get(&server) {
        // The query failed. Inability to verify is not absence (SC-816/SC-835c).
        Some(Err(QueryFailed)) | None => Status::Unknown,
        Some(Ok(sessions)) => {
            // EXACT name. A prefix sibling answers for nobody but itself — that
            // substitution is the tmux-primitive hazard behind #105.
            match sessions.iter().find(|session| session.name == record.name) {
                // A successful query that proves the exact name absent. This is
                // the ONLY route to `stopped`.
                None => Status::Stopped,
                Some(session) => {
                    if positively_owned(&record.name, session.marker.as_deref()) {
                        Status::Running
                    } else {
                        // There, but not provably ae's. Not stopped: it is
                        // demonstrably present.
                        Status::Unknown
                    }
                }
            }
        }
    }
}

/// Whether one observed pane is running an agent — **SC-017s**, both conjuncts.
///
/// A pane tmux reports DEAD is not alive whatever command it names: a
/// `remain-on-exit` pane keeps reporting the exited process, so the command
/// alone reads `alive` for a corpse (#109). An unreadable `pane_dead` is not a
/// readable `0` and therefore proves nothing, which is why the positive answer
/// requires `Some(false)` rather than merely "not `Some(true)`".
///
/// The command half is [`crate::watchdog::command_is_shell`] — ONE definition of
/// the shell set for the watchdog and the listing, because two would drift.
/// An absent command reading counts as a shell for the reason SC-017s gives:
/// absence of evidence must not become a positive alive.
fn pane_alive(pane: &ObservedPane) -> bool {
    pane.dead == Some(false)
        && !crate::watchdog::command_is_shell(pane.command.as_deref().unwrap_or_default())
}

/// What a completed pane enumeration says about the roster slots in `slots`.
///
/// The agent half of the frozen `cmd_list`'s alive map and of
/// `_session_attn_rollup`'s dead verdict, ported as BEHAVIOUR under SC-017p/q
/// rather than copied:
///
/// * **one pane carries the slot** — its two readings decide `alive`, and the
///   agent raises nothing. A pane that dropped to a bare shell is `alive: false`
///   and NOT `dead`: the frozen marks it `!` in the table and leaves the
///   attention verdict to the watchdog's alert, which is the only party that
///   also checked for a descendant agent process.
/// * **several panes carry it** — the association is ambiguous, so nothing is
///   established. `unknown`, never a verdict picked from one of them.
/// * **no pane carries it, and every pane was identified** — a COMPLETE
///   enumeration that positively excludes the slot. That is SC-017p's negative
///   proof, so the agent is `alive: false` and raises [`Reason::Dead`], which is
///   the frozen's "a registered agent whose pane vanished".
/// * **no pane carries it, but some pane carries no marker at all** — one of
///   those could be this agent's, unstamped. `unknown`: SC-017q is explicit that
///   unprovable liveness is never removal, and this is the arm where the frozen
///   asserted `dead` on absence of evidence (#107).
///
/// A FAILED enumeration never reaches here — the caller passes no runtime at
/// all, leaving every slot `unknown`, which is SC-017q's other half.
#[must_use]
pub fn agent_runtimes(panes: &[ObservedPane], slots: &[String]) -> Vec<AgentRuntime> {
    slots
        .iter()
        .map(|slot| {
            let (alive, alert) = match slot_observation(panes, slot) {
                SlotObservation::Unique => (
                    panes
                        .iter()
                        .find(|pane| pane.slot.as_deref() == Some(slot.as_str()))
                        .map(pane_alive),
                    None,
                ),
                SlotObservation::Absent { unidentified: 0 } => (Some(false), Some(Reason::Dead)),
                // Two different unprovabilities, one answer: several panes carry
                // the slot so the association is ambiguous, or an unmarked pane
                // could be this agent's. Neither may become a verdict.
                SlotObservation::Duplicated { .. } | SlotObservation::Absent { .. } => (None, None),
            };
            AgentRuntime {
                slot: slot.clone(),
                alive,
                alert,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "fixtures build and inspect real directories; the boundary is about \
                  what PRODUCT code may reach"
    )]

    use super::{Answers, ObservedPane, Reason, Snapshot, classify, decide, positively_owned};
    use crate::digest::Status;
    use crate::inventory::{
        Candidate, DiscoveredSession, Discovery, DurableRecord, FailedSource, Inventory, Layout,
        LiveSighting, MetaRead, QueryFailed, ServerId,
    };
    use crate::meta::{Selector, ServerSelector};
    use crate::session::{AgentRuntime, RecordSnapshot};
    use std::cell::RefCell;
    use std::path::PathBuf;

    fn named(server: &str) -> ServerId {
        ServerId::Selected(Selector::Name(server.to_owned()))
    }

    fn socket(path: &str) -> ServerId {
        ServerId::Selected(Selector::Socket(PathBuf::from(path)))
    }

    fn positive(server: &str) -> ServerSelector {
        ServerSelector::Positive(Selector::Name(server.to_owned()))
    }

    /// A backend that records every server it is asked about.
    struct Backend {
        worlds: Vec<(ServerId, Result<Vec<DiscoveredSession>, QueryFailed>)>,
        trace: RefCell<Vec<ServerId>>,
    }

    impl Backend {
        fn new() -> Self {
            Self {
                worlds: Vec::new(),
                trace: RefCell::new(Vec::new()),
            }
        }

        fn live(mut self, server: ServerId, sessions: &[(&str, Option<&str>)]) -> Self {
            self.worlds.push((
                server,
                Ok(sessions
                    .iter()
                    .map(|(name, marker)| DiscoveredSession {
                        name: (*name).to_owned(),
                        marker: marker.map(ToOwned::to_owned),
                    })
                    .collect()),
            ));
            self
        }

        fn down(mut self, server: ServerId) -> Self {
            self.worlds.push((server, Err(QueryFailed)));
            self
        }

        /// The SET of servers contacted; query order and count are open choices
        /// (criterion 21), so no test here pins them.
        fn contacted(&self) -> Vec<String> {
            let mut seen: Vec<String> = self.trace.borrow().iter().map(spell).collect();
            seen.sort();
            seen.dedup();
            seen
        }
    }

    fn spell(server: &ServerId) -> String {
        match server {
            ServerId::Ambient => "ambient".to_owned(),
            ServerId::Selected(Selector::Name(name)) => format!("name:{name}"),
            ServerId::Selected(Selector::Socket(path)) => format!("socket:{}", path.display()),
        }
    }

    impl Discovery for Backend {
        /// **HAZARD, and it is deliberate.** An UNREGISTERED server answers
        /// `Ok(vec![])` — a SUCCESSFUL EMPTY query, which is the proof that a
        /// name is absent and therefore the route to `stopped`.
        ///
        /// Seven tests rely on that: `Backend::new()` with no worlds means
        /// "every server answers, and everything is empty", which is exactly
        /// what a stopped-everywhere fixture needs. So the fallback stays.
        ///
        /// The cost is that A MISTYPED SERVER NAME IS INDISTINGUISHABLE FROM A
        /// REGISTERED EMPTY ONE. A test that means to ask server B, typos it,
        /// and asserts `stopped` PASSES — while proving nothing about B. Same
        /// family as the shadowed second `live()` for one server: a fixture
        /// convenience that is right in the common case and silently wrong when
        /// a name is wrong. If a test's claim depends on WHICH server answered,
        /// assert `contacted()` as well as the status.
        fn enumerate(&self, server: &ServerId) -> Result<Vec<DiscoveredSession>, QueryFailed> {
            self.trace.borrow_mut().push(server.clone());
            self.worlds
                .iter()
                .find(|(known, _)| known == server)
                .map_or(Ok(Vec::new()), |(_, answer)| answer.clone())
        }
    }

    fn record(name: &str, server: ServerSelector) -> DurableRecord {
        DurableRecord {
            path: PathBuf::from(format!("/s/{name}")),
            name: name.to_owned(),
            layout: Layout::Canonical,
            server,
            meta_read: MetaRead::Parsed,
            snapshot: RecordSnapshot::default(),
        }
    }

    fn durable(name: &str, server: ServerSelector) -> Candidate {
        Candidate::durable(record(name, server))
    }

    fn sighting(server: ServerId, name: &str, marker: &str) -> LiveSighting {
        LiveSighting {
            server,
            name: name.to_owned(),
            marker: marker.to_owned(),
        }
    }

    fn inventory(candidates: Vec<Candidate>) -> Inventory {
        Inventory {
            candidates,
            incomplete: Vec::new(),
        }
    }

    fn status_of(snapshot: &Snapshot, name: &str) -> Status {
        snapshot
            .sessions
            .iter()
            .find(|classified| classified.candidate.name == name)
            .map_or_else(
                || panic!("no candidate named {name}"),
                |classified| classified.status,
            )
    }

    fn statuses(snapshot: &Snapshot) -> Vec<(String, Status)> {
        let mut seen: Vec<(String, Status)> = snapshot
            .sessions
            .iter()
            .map(|classified| (classified.candidate.name.clone(), classified.status))
            .collect();
        seen.sort_by(|left, right| left.0.cmp(&right.0));
        seen
    }

    // ---- criterion 3: recorded server beats ambient, in both directions ----

    #[test]
    fn criterion_3_the_answer_comes_from_the_recorded_server_and_never_from_ambient() {
        // Three opposed cells, run twice: once with a positive NAME selector and
        // once with a positive SOCKET selector, because the typed halves address
        // different servers and a classifier that flattened them would pass with
        // one and fail with the other.
        for (selector, server, spelling) in [
            (positive("B"), named("B"), "name:B"),
            (
                ServerSelector::Positive(Selector::Socket(PathBuf::from("/tmp/B.sock"))),
                socket("/tmp/B.sock"),
                "socket:/tmp/B.sock",
            ),
        ] {
            // (a) B has it, ambient says absent.
            let backend = Backend::new()
                .live(server.clone(), &[("mdk", Some("mdk"))])
                .live(ServerId::Ambient, &[]);
            let snapshot = classify(inventory(vec![durable("mdk", selector.clone())]), &backend);
            assert_eq!(
                status_of(&snapshot, "mdk"),
                Status::Running,
                "{spelling} (a)"
            );
            assert_eq!(
                backend.contacted(),
                [spelling],
                "attributed to its own server"
            );

            // (b) B proves absence, ambient has it.
            let backend = Backend::new()
                .live(server.clone(), &[])
                .live(ServerId::Ambient, &[("mdk", Some("mdk"))]);
            let snapshot = classify(inventory(vec![durable("mdk", selector.clone())]), &backend);
            assert_eq!(
                status_of(&snapshot, "mdk"),
                Status::Stopped,
                "{spelling} (b)"
            );
            assert_eq!(backend.contacted(), [spelling], "ambient was never asked");

            // (c) B fails, ambient has it.
            let backend = Backend::new()
                .down(server.clone())
                .live(ServerId::Ambient, &[("mdk", Some("mdk"))]);
            let snapshot = classify(inventory(vec![durable("mdk", selector)]), &backend);
            assert_eq!(
                status_of(&snapshot, "mdk"),
                Status::Unknown,
                "{spelling} (c)"
            );
            assert_eq!(
                backend.contacted(),
                [spelling],
                "a failed own-server query is not a reason to believe another server"
            );
        }
    }

    // ---- criterion 4: exact name versus prefix sibling ---------------------

    #[test]
    fn criterion_4_a_prefix_sibling_is_not_the_candidate_and_the_exact_control_flips_it() {
        let sibling_only = Backend::new().live(named("B"), &[("mdk-app", Some("mdk-app"))]);
        let snapshot = classify(
            inventory(vec![durable("mdk", positive("B"))]),
            &sibling_only,
        );
        assert_eq!(
            status_of(&snapshot, "mdk"),
            Status::Stopped,
            "the successful query proved the EXACT name absent; the sibling is not it"
        );

        let with_exact = Backend::new().live(
            named("B"),
            &[("mdk-app", Some("mdk-app")), ("mdk", Some("mdk"))],
        );
        let snapshot = classify(inventory(vec![durable("mdk", positive("B"))]), &with_exact);
        assert_eq!(
            status_of(&snapshot, "mdk"),
            Status::Running,
            "and adding the exact name is what changes the answer"
        );
    }

    // ---- criterion 5: ownership is part of the positive proof -------------

    #[test]
    fn criterion_5_ownership_decides_between_running_and_unknown() {
        // Recorded server, exact name and query success held fixed; only the
        // ownership evidence moves.
        for (marker, expected, cell) in [
            (Some("mdk"), Status::Running, "positive ae-ownership"),
            (None, Status::Unknown, "ownership missing"),
            (
                // A present-but-empty marker, which is what an unset variable
                // can render as. A DIFFERENT value is no longer a non-proof:
                // the tag says "ae's", and identity was settled by the exact
                // name match upstream.
                Some(""),
                Status::Unknown,
                "marker present but empty",
            ),
        ] {
            let backend = Backend::new().live(named("B"), &[("mdk", marker)]);
            let snapshot = classify(inventory(vec![durable("mdk", positive("B"))]), &backend);
            assert_eq!(status_of(&snapshot, "mdk"), expected, "{cell}");
        }
        // The fourth cell: the exact name is not there at all.
        let absent = Backend::new().live(named("B"), &[]);
        let snapshot = classify(inventory(vec![durable("mdk", positive("B"))]), &absent);
        assert_eq!(
            status_of(&snapshot, "mdk"),
            Status::Stopped,
            "exact name absent"
        );
    }

    #[test]
    fn the_ownership_predicate_is_the_presence_of_the_marker() {
        // THE MARKER IS A TAG, NOT A NAME. The real producer writes
        // `AE_SESSION=1`, so requiring the value to equal the session name made
        // the predicate unsatisfiable in the field and killed the whole status
        // column — five live sessions, all tagged, all `unknown`. Identity is
        // established upstream by an EXACT name match against this session's own
        // recorded server; the tag only answers "is this ae's?".
        assert!(positively_owned("mdk", Some("mdk")), "a name is a value");
        assert!(
            positively_owned("mdk", Some("1")),
            "the value the real producer actually writes is positive evidence"
        );
        assert!(
            positively_owned("mdk", Some("mdk-app")),
            "a different value is still a tag: this predicate does not adjudicate identity"
        );
        assert!(!positively_owned("mdk", None), "missing is not positive");
        assert!(
            !positively_owned("mdk", Some("")),
            "present-but-empty is not evidence — an unset variable can render this way"
        );
    }

    /// The regression this slice exists for: the live marker shape classifies.
    #[test]
    fn a_session_tagged_the_way_the_real_producer_tags_it_is_running() {
        let succeeded = Backend::new().live(named("B"), &[("mdk", Some("1"))]);
        assert_eq!(
            status_of(
                &classify(inventory(vec![durable("mdk", positive("B"))]), &succeeded),
                "mdk"
            ),
            Status::Running,
            "AE_SESSION=1 is what ae writes; it must classify running, not unknown"
        );
    }

    // ---- criterion 6: success and failure override identical payloads ------

    #[test]
    fn criterion_6_the_transport_result_decides_and_identical_payloads_do_not() {
        for (payload, on_success, why) in [
            (Vec::new(), Status::Stopped, "empty output"),
            (
                vec![("mdk", Some("mdk"))],
                Status::Running,
                "output containing the exact owned candidate",
            ),
        ] {
            let succeeded = Backend::new().live(named("B"), &payload);
            assert_eq!(
                status_of(
                    &classify(inventory(vec![durable("mdk", positive("B"))]), &succeeded),
                    "mdk"
                ),
                on_success,
                "{why}, query succeeded"
            );

            // The SAME payload bytes, behind a failed query. A failure carries no
            // proof in either direction, however complete its partial output
            // looks.
            let failed = Backend::new().down(named("B"));
            assert_eq!(
                status_of(
                    &classify(inventory(vec![durable("mdk", positive("B"))]), &failed),
                    "mdk"
                ),
                Status::Unknown,
                "{why}, query failed"
            );
        }
    }

    // ---- criterion 7: every non-proof routes to unknown, nothing deleted ---

    #[test]
    fn criterion_7_every_non_proof_is_unknown_and_no_candidate_is_deleted() {
        let mut readable_missing = record("readable-missing", ServerSelector::Missing);
        readable_missing.meta_read = MetaRead::Parsed;
        let mut absent_record = record("absent-record", ServerSelector::Missing);
        absent_record.meta_read = MetaRead::Absent;
        let mut unreadable_record = record("unreadable-record", ServerSelector::Missing);
        unreadable_record.meta_read = MetaRead::Unreadable;

        let candidates = vec![
            Candidate::durable(readable_missing),
            Candidate::durable(absent_record),
            Candidate::durable(unreadable_record),
            durable("ambiguous", ServerSelector::Ambiguous),
            durable("unreachable", positive("down")),
            durable("query-failed", positive("also-down")),
            durable("marker-missing", positive("up")),
            durable("marker-empty", positive("up")),
            Candidate::tmux_only(sighting(ServerId::Ambient, "live-unowned", "")),
        ];
        let backend = Backend::new()
            .down(named("down"))
            .down(named("also-down"))
            .live(
                named("up"),
                &[("marker-missing", None), ("marker-empty", Some(""))],
            );

        let snapshot = classify(inventory(candidates), &backend);

        assert_eq!(snapshot.sessions.len(), 9, "every candidate survives");
        assert!(
            snapshot
                .sessions
                .iter()
                .all(|classified| classified.status == Status::Unknown),
            "{:?}",
            statuses(&snapshot)
        );
        // Missing and ambiguous selectors derive NO backend target from their
        // raw bytes; the positive-but-doomed ones show an attempted query of
        // their typed target.
        assert_eq!(
            backend.contacted(),
            ["name:also-down", "name:down", "name:up"],
            "no target was invented for a selector that did not name one"
        );
        // The three `missing` cells agree on selector state and keep their
        // distinct phase-1 read facts (criterion 21's axis, still intact here).
        let reads: Vec<MetaRead> = ["readable-missing", "absent-record", "unreadable-record"]
            .iter()
            .map(|name| {
                snapshot
                    .sessions
                    .iter()
                    .find(|classified| classified.candidate.name == *name)
                    .and_then(|classified| classified.candidate.durable.as_ref())
                    .map(|durable| durable.meta_read)
                    .expect("the candidate survived")
            })
            .collect();
        assert_eq!(
            reads,
            [MetaRead::Parsed, MetaRead::Absent, MetaRead::Unreadable],
            "same selector state, three different record-read facts"
        );
    }

    // ---- criterion 8: a query failure is local to its server group ---------

    #[test]
    fn criterion_8_one_failing_server_does_not_decide_another_server_s_candidates() {
        let candidates = vec![
            durable("a-one", positive("A")),
            durable("a-two", positive("A")),
            durable("b-running", positive("B")),
            durable("b-stopped", positive("B")),
        ];
        let backend = Backend::new()
            .down(named("A"))
            .live(named("B"), &[("b-running", Some("b-running"))]);

        let snapshot = classify(inventory(candidates), &backend);

        assert_eq!(
            statuses(&snapshot),
            [
                ("a-one".to_owned(), Status::Unknown),
                ("a-two".to_owned(), Status::Unknown),
                ("b-running".to_owned(), Status::Running),
                ("b-stopped".to_owned(), Status::Stopped),
            ],
            "A's failure is A's; B's candidates are decided independently"
        );
        assert_eq!(snapshot.sessions.len(), 4, "the set is returned whole");
    }

    // ---- criteria 9 and 10: the snapshot proof is never re-queried ---------

    #[test]
    fn criterion_9_a_tmux_only_candidate_keeps_its_discovery_fact_when_the_server_goes_away() {
        // Phase 1 saw it alive on A. A is unavailable by the time we classify.
        let candidate = Candidate::tmux_only(sighting(named("A"), "ghost", "ghost"));
        let backend = Backend::new().down(named("A"));

        let snapshot = classify(inventory(vec![candidate]), &backend);

        assert_eq!(
            status_of(&snapshot, "ghost"),
            Status::Running,
            "the snapshot fact stands even though A is now unavailable"
        );
        let classified = &snapshot.sessions[0];
        assert!(
            classified.candidate.durable.is_none(),
            "and no durable record was fabricated for it"
        );
    }

    #[test]
    fn criterion_10_dual_provenance_keeps_the_matched_proof_and_a_durable_only_twin_does_not() {
        // The coalesced candidate: durable record with a positive selector for
        // B, plus the exact owned sighting phase 1 matched to it from B.
        let mut coalesced = durable("api", positive("B"));
        coalesced.live = Some(sighting(named("B"), "api", "api"));
        // The opposed control: same selector and name, no matched sighting.
        let alone = durable("api-alone", positive("B"));

        let backend = Backend::new().down(named("B"));
        let snapshot = classify(inventory(vec![coalesced, alone]), &backend);

        assert_eq!(
            status_of(&snapshot, "api"),
            Status::Running,
            "the matched sighting IS the successful own-server proof for this snapshot"
        );
        assert_eq!(
            status_of(&snapshot, "api-alone"),
            Status::Unknown,
            "without that proof, the same failed query decides nothing"
        );
        assert_eq!(
            snapshot.sessions.len(),
            2,
            "and no second candidate appeared"
        );
    }

    #[test]
    fn a_redundant_answer_cannot_replace_an_accepted_snapshot_proof() {
        // Monotonicity, stated as a test: the server that proved it alive now
        // says it is gone. Learning more must not reduce what is known, so the
        // accepted fact stands and this snapshot is not the place that changes.
        let mut coalesced = durable("api", positive("B"));
        coalesced.live = Some(sighting(named("B"), "api", "api"));
        let backend = Backend::new().live(named("B"), &[]);

        let snapshot = classify(inventory(vec![coalesced]), &backend);

        assert_eq!(
            status_of(&snapshot, "api"),
            Status::Running,
            "the CONTRADICTING answer did not replace the accepted proof"
        );
        // Deliberately NOT asserted: whether the query happened at all. This
        // implementation does not make it, but criterion 10 and criterion 21
        // both leave a redundant query an OPEN CHOICE — it may not CHANGE the
        // classification, and that is what the assertion above tests. A test
        // pinning the call count would reject a correct classifier for a choice
        // no row makes.
    }

    #[test]
    fn a_dual_candidate_whose_sighting_is_not_positively_owned_follows_the_query_rule() {
        // SC-017k's exception to the exception: only a POSITIVELY OWNED matched
        // sighting is the snapshot proof.
        let mut unowned = durable("api", positive("B"));
        unowned.live = Some(sighting(named("B"), "api", ""));
        let backend = Backend::new().live(named("B"), &[("api", Some("api"))]);

        let snapshot = classify(inventory(vec![unowned]), &backend);

        assert_eq!(
            status_of(&snapshot, "api"),
            Status::Running,
            "the answer came from the QUERY, not from the unowned sighting"
        );
        assert_eq!(
            backend.contacted(),
            ["name:B"],
            "it asked its own server, because its sighting was not proof"
        );
    }

    // ---- criterion 11: grouping shares work, never answers -----------------

    #[test]
    fn criterion_11_three_candidates_on_one_response_get_three_different_answers() {
        let candidates = vec![
            durable("alpha", positive("B")),
            durable("beta", positive("B")),
            durable("gamma", positive("B")),
        ];
        let backend = Backend::new().live(named("B"), &[("alpha", Some("alpha")), ("gamma", None)]);

        let snapshot = classify(inventory(candidates), &backend);

        assert_eq!(
            statuses(&snapshot),
            [
                ("alpha".to_owned(), Status::Running),
                ("beta".to_owned(), Status::Stopped),
                ("gamma".to_owned(), Status::Unknown),
            ],
            "one response, three exact-name answers"
        );
        assert_eq!(
            backend.contacted(),
            ["name:B"],
            "the semantic server set is one server; the call count is not asserted"
        );
    }

    // ---- criterion 12: the same name on different servers ------------------

    #[test]
    fn criterion_12_the_same_name_on_three_servers_never_shares_an_answer() {
        let candidates = vec![
            Candidate::durable(DurableRecord {
                path: PathBuf::from("/roots/a/shared"),
                name: "shared".to_owned(),
                layout: Layout::Canonical,
                server: positive("A"),
                meta_read: MetaRead::Parsed,
                snapshot: RecordSnapshot::default(),
            }),
            Candidate::durable(DurableRecord {
                path: PathBuf::from("/roots/b/shared"),
                name: "shared".to_owned(),
                layout: Layout::WorktreeNested,
                server: positive("B"),
                meta_read: MetaRead::Parsed,
                snapshot: RecordSnapshot::default(),
            }),
            Candidate::durable(DurableRecord {
                path: PathBuf::from("/roots/c/shared"),
                name: "shared".to_owned(),
                layout: Layout::WorktreeNested,
                server: positive("C"),
                meta_read: MetaRead::Parsed,
                snapshot: RecordSnapshot::default(),
            }),
        ];
        let backend = Backend::new()
            .live(named("A"), &[("shared", Some("shared"))])
            .live(named("B"), &[])
            .down(named("C"));

        let snapshot = classify(inventory(candidates), &backend);

        let by_path: Vec<(String, Status)> = snapshot
            .sessions
            .iter()
            .map(|classified| {
                (
                    classified
                        .candidate
                        .durable
                        .as_ref()
                        .map(|record| record.path.display().to_string())
                        .unwrap_or_default(),
                    classified.status,
                )
            })
            .collect();
        assert_eq!(
            by_path,
            [
                ("/roots/a/shared".to_owned(), Status::Running),
                ("/roots/b/shared".to_owned(), Status::Stopped),
                ("/roots/c/shared".to_owned(), Status::Unknown),
            ],
            "one name, three identities, three answers"
        );
    }

    // ---- criterion 2: total and cardinality-preserving ---------------------

    #[test]
    fn criterion_2_classification_is_total_and_changes_no_identity() {
        let mut coalesced = durable("dual", positive("B"));
        coalesced.live = Some(sighting(named("B"), "dual", "dual"));
        // An unreadable meta yields a MISSING selector — phase 1 derives no
        // pointer from bytes nobody could read — so this pairing is the only
        // consistent one, and it is `unknown` for the selector, not for the
        // damage. The two axes stay orthogonal; they are just not independent
        // in THIS direction, because the same failed read causes both.
        let mut damaged = record("damaged", ServerSelector::Missing);
        damaged.meta_read = MetaRead::Unreadable;
        let candidates = vec![
            durable("running-one", positive("B")),
            durable("stopped-one", positive("B")),
            durable("unknown-one", ServerSelector::Ambiguous),
            Candidate::durable(damaged),
            coalesced,
            Candidate::tmux_only(sighting(ServerId::Ambient, "live-only", "live-only")),
        ];
        let before: Vec<Candidate> = candidates.clone();
        let backend = Backend::new().live(named("B"), &[("running-one", Some("running-one"))]);

        let snapshot = classify(inventory(candidates), &backend);

        assert_eq!(
            snapshot.sessions.len(),
            before.len(),
            "cardinality preserved"
        );
        let after: Vec<Candidate> = snapshot
            .sessions
            .iter()
            .map(|classified| classified.candidate.clone())
            .collect();
        assert_eq!(
            after, before,
            "not one candidate was added, dropped, merged, split or rewritten"
        );
        assert_eq!(
            statuses(&snapshot),
            [
                ("damaged".to_owned(), Status::Unknown),
                ("dual".to_owned(), Status::Running),
                ("live-only".to_owned(), Status::Running),
                ("running-one".to_owned(), Status::Running),
                ("stopped-one".to_owned(), Status::Stopped),
                ("unknown-one".to_owned(), Status::Unknown),
            ],
            "and every status in the domain is reachable"
        );
    }

    // ---- criterion 23: completeness crosses the boundary unchanged ---------

    #[test]
    fn criterion_23_the_completeness_fact_crosses_classification_unchanged() {
        // The controlled pair: same candidates, same backend answers, differing
        // only in the phase-1 completeness fact.
        let losses = vec![
            FailedSource::CanonicalRoot(PathBuf::from("/home/x/.ae/sessions")),
            FailedSource::WorktreeRoot(PathBuf::from("/home/x/.ae/worktrees")),
        ];
        let build = |incomplete: Vec<FailedSource>| Inventory {
            candidates: vec![
                durable("running-one", positive("B")),
                durable("stopped-one", positive("B")),
            ],
            incomplete,
        };
        let answers = || Backend::new().live(named("B"), &[("running-one", Some("running-one"))]);

        let complete = classify(build(Vec::new()), &answers());
        let incomplete = classify(build(losses.clone()), &answers());

        assert_eq!(
            statuses(&complete),
            statuses(&incomplete),
            "identities, statuses and facts are identical; only completeness differs"
        );
        assert!(complete.complete());
        assert!(!incomplete.complete());
        assert_eq!(
            incomplete.incomplete, losses,
            "BOTH distinguishable loss facts crossed — retaining only the first is a failure"
        );

        // ...and with an empty candidate set, so an incomplete-empty snapshot
        // cannot masquerade as an authoritative empty one.
        let empty_complete = classify(inventory(Vec::new()), &Backend::new());
        let empty_incomplete = classify(
            Inventory {
                candidates: Vec::new(),
                incomplete: losses,
            },
            &Backend::new(),
        );
        assert!(empty_complete.sessions.is_empty() && empty_complete.complete());
        assert!(empty_incomplete.sessions.is_empty() && !empty_incomplete.complete());
    }

    #[test]
    fn an_incomplete_snapshot_never_becomes_a_session_level_condition() {
        // SC-017o: incompleteness is snapshot state, not a synthetic session and
        // not a status.
        let snapshot = classify(
            Inventory {
                candidates: vec![durable("fine", positive("B"))],
                incomplete: vec![FailedSource::Server(named("gone"))],
            },
            &Backend::new().live(named("B"), &[("fine", Some("fine"))]),
        );
        assert_eq!(status_of(&snapshot, "fine"), Status::Running);
        assert_eq!(snapshot.sessions.len(), 1, "no candidate was fabricated");
        assert!(!snapshot.complete());
    }

    // ---- criterion 22: entitlement is never widened ------------------------

    #[test]
    fn criterion_22_phase_2_contacts_only_servers_its_input_already_named() {
        // Every server reachable from here comes from a candidate's own typed
        // selector, so the entitled set is an upper bound by construction. The
        // planted servers below would answer if asked. They are not asked.
        let candidates = vec![
            durable("mine", positive("B")),
            durable("no-selector", ServerSelector::Missing),
            durable("ambiguous", ServerSelector::Ambiguous),
        ];
        let backend = Backend::new()
            .live(named("B"), &[("mine", Some("mine"))])
            .live(ServerId::Ambient, &[("tempting", Some("tempting"))])
            .live(
                socket("/tmp/tmux-1000/default"),
                &[("swept", Some("swept"))],
            );

        let snapshot = classify(inventory(candidates), &backend);

        assert_eq!(
            backend.contacted(),
            ["name:B"],
            "not the ambient server, not a plausible socket path — only what the input named"
        );
        assert_eq!(snapshot.sessions.len(), 3);
    }

    #[test]
    fn a_failed_query_is_unknown_rather_than_stopped() {
        // The FAILED-query branch. Named for what it reaches: the server was
        // asked and did not answer.
        let snapshot = classify(
            inventory(vec![durable("orphan", positive("down"))]),
            &Backend::new().down(named("down")),
        );
        assert_eq!(status_of(&snapshot, "orphan"), Status::Unknown);
    }

    #[test]
    fn a_candidate_whose_server_was_never_asked_is_unknown_rather_than_stopped() {
        // The MISSING-ANSWER branch, reached directly because no fixture can
        // reach it through `classify`: `Answers::gather` enumerates exactly the
        // servers the candidates name, so a candidate whose server has no entry
        // is an internal inconsistency rather than a reachable state.
        //
        // It is still worth pinning, and it has to be pinned HERE. An earlier
        // version of this test used a registered-and-failing server — which is
        // the branch above — so if this arm had returned `stopped`, that test
        // would have passed while naming a branch it never touched. A test whose
        // name describes a branch it cannot reach is the same class as a shape
        // that does not contain what its name says.
        let no_answers = Answers(Vec::new());
        let candidate = durable("orphan", positive("never-asked"));
        assert_eq!(
            decide(&candidate, &no_answers),
            Status::Unknown,
            "`stopped` requires a successful query, and a query that never \
             happened is not one"
        );
    }

    // ---- criterion 14: no rediscovery, asked of the source -----------------

    #[test]
    fn criterion_14_this_module_reaches_no_filesystem_and_rediscovers_nothing() {
        let source = include_str!("liveness.rs");
        let code: String = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let (module, tests) = code
            .split_once("#[cfg(test)]")
            .expect("this module has a test module");
        assert!(
            tests.contains("fn criterion_14") && module.contains("pub fn classify"),
            "the split landed where it was meant to"
        );
        for rediscovery in [
            concat!("fs", "::"),
            concat!("read_", "dir"),
            concat!("durable_", "records"),
            concat!("Meta", "::read"),
            concat!("Path", "::new"),
        ] {
            assert!(
                !module.contains(rediscovery),
                "phase 2 consumes phase-1 facts; it must not be able to {rediscovery}"
            );
        }
    }

    fn pane(dead: Option<bool>, slot: Option<&str>, command: Option<&str>) -> ObservedPane {
        ObservedPane {
            dead,
            slot: slot.map(ToOwned::to_owned),
            command: command.map(ToOwned::to_owned),
        }
    }

    fn only(panes: &[ObservedPane], slot: &str) -> AgentRuntime {
        let slots = vec![slot.to_owned()];
        let mut runtimes = super::agent_runtimes(panes, &slots);
        assert_eq!(runtimes.len(), 1, "one slot in, one runtime out");
        runtimes.remove(0)
    }

    #[test]
    fn a_seat_running_its_agent_is_alive_and_raises_nothing() {
        let observed = only(&[pane(Some(false), Some("main"), Some("claude"))], "main");
        assert_eq!(observed.alive, Some(true));
        assert_eq!(observed.alert, None);
    }

    #[test]
    fn a_seat_that_dropped_to_a_shell_is_not_alive_and_still_raises_nothing() {
        // Frozen's `!`: present, but no agent in the foreground. The DEAD
        // verdict stays the watchdog's, which also checks for a descendant
        // agent process — a `bash -lc <tool>` wrapper is a shell with a live
        // agent underneath it.
        let observed = only(&[pane(Some(false), Some("main"), Some("fish"))], "main");
        assert_eq!(observed.alive, Some(false));
        assert_eq!(observed.alert, None, "a shell pane is not a vanished pane");
    }

    #[test]
    fn a_pane_tmux_reports_dead_is_not_alive_whatever_command_it_names() {
        // #109: `remain-on-exit` keeps reporting the exited process, and `true`
        // is not in the shell set — so the command field alone reads alive.
        let observed = only(&[pane(Some(true), Some("main"), Some("true"))], "main");
        assert_eq!(observed.alive, Some(false));
    }

    #[test]
    fn an_unreadable_dead_field_proves_nothing_positive() {
        let observed = only(&[pane(None, Some("main"), Some("claude"))], "main");
        assert_eq!(
            observed.alive,
            Some(false),
            "a positive alive needs a readable `pane_dead` of 0"
        );
    }

    #[test]
    fn a_slot_no_pane_carries_is_dead_only_when_every_pane_was_identified() {
        let identified = [pane(Some(false), Some("main"), Some("claude"))];
        let vanished = only(&identified, "worker.0");
        assert_eq!(vanished.alive, Some(false));
        assert_eq!(
            vanished.alert,
            Some(Reason::Dead),
            "a complete enumeration that excludes the slot is SC-017p's negative proof"
        );

        // #107's arm: an unstamped pane could BE this agent, so absence of
        // evidence must not become the removal SC-017q forbids.
        let ambiguous = [
            pane(Some(false), Some("main"), Some("claude")),
            pane(Some(false), None, Some("fish")),
        ];
        let unknown = only(&ambiguous, "worker.0");
        assert_eq!(unknown.alive, None);
        assert_eq!(unknown.alert, None);
    }

    #[test]
    fn two_panes_carrying_one_slot_establish_nothing_rather_than_picking_one() {
        let duplicated = [
            pane(Some(false), Some("main"), Some("claude")),
            pane(Some(false), Some("main"), Some("fish")),
        ];
        let observed = only(&duplicated, "main");
        assert_eq!(observed.alive, None);
        assert_eq!(observed.alert, None);
    }

    #[test]
    fn every_slot_asked_about_gets_an_answer_in_the_order_it_was_asked() {
        let panes = [
            pane(Some(false), Some("worker.0"), Some("codex")),
            pane(Some(false), Some("main"), Some("claude")),
        ];
        let slots = ["main".to_owned(), "worker.0".to_owned()];
        let runtimes = super::agent_runtimes(&panes, &slots);
        assert_eq!(
            runtimes.iter().map(|r| r.slot.as_str()).collect::<Vec<_>>(),
            ["main", "worker.0"],
            "the roster's order, not the enumeration's"
        );
        assert!(runtimes.iter().all(|r| r.alive == Some(true)));
    }
}
