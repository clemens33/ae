//! The `requests` read surface — SC-212c, SC-518, SC-1306d, SC-211d.
//!
//! Two things, kept apart the way the frozen implementation keeps them apart:
//! the request SENSOR ([`states`]) and the TABLE the helper prints
//! ([`render`]). The frozen `requests` helper carries `_ar_request_states`
//! beside it by `declare -f` rather than owning a second copy, and the frozen
//! source says why — the two copies had already drifted once, one checking both
//! ends of a reply and the other only the actor end. So the sensor is a public
//! function here, not a private detail of the table.
//!
//! # The frozen argv, and the successor spelling
//!
//! The corpus rows invoke a per-session script: `<AE_HOME>/sessions/tg1/requests
//! all`. The successor spelling ruled for it (lead ruling on D1, 2026-08-24) is
//!
//! ```text
//! <AE_HOME>/sessions/<name>/requests [mine|inbox|all]
//!   ->  ae _requests <AE_HOME>/sessions/<name> [mine|inbox|all]
//! ```
//!
//! The leading underscore is load-bearing: `_validate_session_name` forbids a
//! leading `_`, so an underscore-prefixed subcommand can never shadow a legal
//! session name and SC-022's "a top-level bare word is a launch candidate"
//! loses nothing. This mapping is a FIXED DECLARED INPUT to a parity run, not
//! something a runner improvises per row — phase-4 criterion 14 requires the
//! effective normalised argv of every invocation.
//!
//! # What this surface deliberately does not do
//!
//! `mine` and `inbox` filter against the CALLER's pane identity, which the
//! frozen helper reads from tmux through `ae_current_agent_ref` and
//! `ae_current_slot`. That sensor is a different ownership record and has not
//! flipped, so this build supplies an empty [`Viewer`] and both modes take the
//! frozen "could not detect current agent identity" refusal. That is not a stub
//! answer dressed as a real one: the message states exactly what is true of this
//! build, it is the same refusal the frozen helper gives outside a pane, and all
//! 24 corpus rows for those two modes pin `rc=1` with those bytes. The typed API
//! takes a `Viewer` so the identity slice can supply one without touching this
//! module.
//!
//! # Pending, replied, cancelled
//!
//! **SC-518 and SC-518a together, and a terminal must satisfy BOTH.**
//!
//! - the NEWEST `ask`/`review` per `ref` opens the request shown;
//! - **SC-518 (identity)** — a `reply` closes it only on a FULL MIRROR: the
//!   reply's actor is the request's target AND the reply's target is the
//!   request's sender, each compared by [`Identity::matches`]. Routing keys
//!   compare to routing keys, display names to display names, and a MIXED pair
//!   matches nothing — as does an `Unassociated` side, including against another
//!   `Unassociated`. Two events that each failed to say where they came from
//!   have not thereby said the same thing;
//! - **SC-518a (order)** — a terminal terminates only the NEWEST PRECEDING
//!   opening with that ref. A terminal that PRECEDES its opening closes
//!   nothing: causality is not a matching condition that `ref` equality can
//!   satisfy. A later re-`ask` opens a NEW lifecycle and an earlier terminal
//!   cannot reach forward into it;
//! - a `cancel` is a terminal and not a third thing, so **SC-518a's order rule
//!   governs it exactly as it governs a reply**. Its AUTHORIZATION is a
//!   different matter and **has no row at all**: SC-518 defines a REPLY MIRROR,
//!   and a cancel has no target end to mirror. What this module does — accept a
//!   cancel whose actor identity is the request's own sender — is therefore an
//!   UNAUTHORIZED INTERIM, not an implementation of SC-518, and is marked as
//!   such at [`Opening::withdrawn_by`]. No closure is claimed for cancel until a
//!   ruling exists.
//!
//!   **AND NO GATING TEST ASSERTS IT.** The contract's rule is ENFORCEMENT and
//!   not labelling: a gate that fails under the other policy has ratified this
//!   one whatever its comment says. So the gated suite asserts only the
//!   OUTCOME-NEUTRAL half — that a cancel placed before its opening leaves the
//!   row identical to a container with no cancel at all, which is true under
//!   every possible authorization — and the current policy is recorded in
//!   `#[ignore]`d diagnostics that no lane runs. PROVEN, not asserted: replacing
//!   the interim policy with accept-anyone, accept-nobody or accept-the-target
//!   leaves `just rust-check` GREEN in all three cases;
//! - every candidate is retained and validated afterwards. Keeping only the
//!   newest raw one lets an INVALID newer event discard a VALID older one —
//!   measured in the frozen tree: ask, valid withdrawal, then a stranger's
//!   cancel rendered `pending`.
//!
//! The safety direction is the ruling's reason, and it is worth stating because
//! it decides every close call above: a false PENDING is LOUD and costs a human
//! a second glance, while a false CLOSURE silently erases a real request.
//!
//! # WHERE THIS DIVERGES FROM THE FROZEN CAPTURES, DELIBERATELY
//!
//! Twelve corpus rows. Frozen bash entered its routed comparison on exactly two
//! selectors — `request.target_slot` nonempty AND `reply.actor_slot` nonempty,
//! all four keys compared only after that — and fell back to display names
//! otherwise, so it closed five of the six mixed shapes. And it matched on `ref`
//! with no ordering test at all, so it let a reply close a request it preceded.
//! The seats ruled both DEFECTS on 2026-08-24. This surface therefore no longer
//! reproduces `A7` 405j pair session-only / keyless / one-empty / all-empty,
//! `G5/m6-mixed-routed-display`, or `G5/m2-wrong-ref` — each `-ro` and `-rw`.
//!
//! That divergence is asserted as precisely as the parity is, in
//! `tests/it/helper_corpus.rs`: the twelve must differ, must differ only in the
//! status token and the summary, and must be the ONLY rows that differ.
//!
//! # WHAT IS STILL UNRULED, NAMED SO IT IS NOT DECIDED BY ACCIDENT
//!
//! Four shapes, and the corpus can never speak to three of them — it contains
//! **zero** `cancel` events across all 6,862 files, so no future capture will
//! settle those either. Each is pinned by a successor test instead, because a
//! successor test is the only evidence they will ever have:
//!
//! 1. **the inverse mixed pair** — a display-only opening and a routed reply.
//!    Every corpus specimen mixes the OTHER way, so "both directions" is a
//!    ruling and not a measurement, and the test exercises the direction the
//!    corpus lacks so a directional implementation fails;
//! 2. **a re-ask after a terminal** — the re-ask is `pending` and the earlier
//!    lifecycle stays closed by its own terminal;
//! 3. **cancel causality** — a cancel before its opening closes nothing;
//! 4. **a `cancel` AND a `reply` both after one opening.** SC-518a ATTACHES
//!    terminals to openings; it does not choose between two that both attach to
//!    one. **Undecided, and the frozen behavior here is MEASURED IS AND NEVER
//!    NORMATIVE** (colead ruling, 2026-08-24). This module has to do something,
//!    so it resolves by KIND — cancellation wins, on frozen's reason that a
//!    straggler answer must not reopen a request nobody is waiting on.
//!
//!    **THE GATE SAYS NOTHING ABOUT THIS SHAPE, and the second attempt is why.**
//!    A test asserting only that the outcome is INDEPENDENT of the two
//!    terminals' arrival order looks neutral and is not: "the later terminal
//!    wins" is a legitimate unresolved policy, under which the two orders
//!    DISAGREE, so an equality assertion forbids it. Arrival-order independence
//!    is itself a precedence law — resolve-by-kind rather than resolve-by-
//!    recency — one level up from the winner it declines to name. There is
//!    therefore no outcome-neutral gating assertion available here at all, and
//!    the whole shape lives in an `#[ignore]`d diagnostic. If the product needs
//!    a winner, that is a separate joint ruling to request.
//!
//! And one CHOICE consequent on the ruling rather than contained in it: the
//! `mine`/`inbox` filter uses the same identity rule as closure, because "is
//! this row's sender me?" is the same question. No corpus row constrains it —
//! all 24 of those rows are identity refusals — so it is named at
//! [`Request::shown_to`] where a seat can overrule it visibly.
//!
//! # SC-1306d
//!
//! The scan is snapshot-semantic. This module reads the container once and
//! answers from those bytes, so a reply appended after the read leaves this
//! invocation's row `pending` and a clean rerun reports `replied`.

use std::collections::HashMap;
use std::path::Path;

use crate::event_text::{
    CONTAINER, Member, event_line, extract, member, pad_left_aligned, read_container, read_lines,
    reversed,
};
use crate::events::Identity;

/// `requests [mine|inbox|all]` — SC-212c's signature, defaulting to `mine`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Requests this agent opened.
    #[default]
    Mine,
    /// Requests addressed to this agent.
    Inbox,
    /// Every request in the session.
    All,
}

impl Mode {
    /// The mode a token selects, or `None` for a token that is not one.
    ///
    /// `None` for the argument is `mine`: the frozen helper's `${1:-mine}`.
    /// A token that is not one of the three is a USAGE ERROR and is not this
    /// type's to describe — see [`crate::cli::Request`], which carries it to the
    /// crate's one usage exit code.
    ///
    /// ```
    /// use ae::requests::Mode;
    ///
    /// assert_eq!(Mode::parse(None), Some(Mode::Mine));
    /// assert_eq!(Mode::parse(Some("all")), Some(Mode::All));
    /// assert_eq!(Mode::parse(Some("Mine")), None, "the tokens are exact");
    /// ```
    #[must_use]
    pub fn parse(token: Option<&str>) -> Option<Self> {
        match token.unwrap_or("mine") {
            "mine" => Some(Self::Mine),
            "inbox" => Some(Self::Inbox),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    /// Whether this mode needs to know who is asking.
    #[must_use]
    pub fn needs_identity(self) -> bool {
        self != Self::All
    }
}

/// Who is asking — the pane identity the frozen helper reads from tmux.
///
/// Empty [`Viewer::display`] is "could not detect", which is the state this
/// build is always in; see the module docs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Viewer {
    /// `ae_current_slot` — the routing key.
    pub slot: String,
    /// The tmux session (`#S`) the calling pane belongs to.
    pub session: String,
    /// `ae_current_agent_ref` — the display `alias:name`.
    pub display: String,
}

impl Viewer {
    /// Whether an identity was detected at all.
    ///
    /// The frozen helper tests `-z "$self"` — the DISPLAY ref — and refuses
    /// before printing the header. A slot without a display ref is not an
    /// identity for this test, which is why this reads one field and not three.
    #[must_use]
    pub fn is_known(&self) -> bool {
        !self.display.is_empty()
    }

    /// This viewer, classified by the same rule the rows are.
    ///
    /// A pane either has both routing halves or neither: `ae_current_slot` and
    /// the tmux session are read together, so the `Unassociated` middle is not
    /// reachable from a real pane. It is still spelled out rather than assumed,
    /// because a viewer assembled from partial data must not silently become a
    /// display-name match.
    fn identity(&self) -> Identity<'_> {
        match (self.slot.is_empty(), self.session.is_empty()) {
            (false, false) => Identity::Routed {
                slot: &self.slot,
                session: &self.session,
            },
            (true, true) => Identity::Display(&self.display),
            _ => Identity::Unassociated,
        }
    }
}

/// One request, as the sensor emits it.
///
/// The frozen sensor emits a unit-separated row — `\x1f`, deliberately not tabs,
/// because tab is an IFS *whitespace* character and an empty field between two
/// tabs silently shifts every later field left. A struct has no framing to get
/// wrong, so the separator does not survive the port; the FIELD SET and its
/// order do, and the row's own reason for existing (`summary` last, because free
/// text with no separator in it must land intact) is a property of the frozen
/// serialisation rather than of the data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// `pending`, `replied` or `cancelled`.
    pub status: Status,
    /// `ask` or `review`, verbatim from the opening event.
    pub kind: Vec<u8>,
    /// The request id (`ref`).
    pub id: Vec<u8>,
    /// The opening event's `actor`.
    pub from: Vec<u8>,
    /// The opening event's `target`.
    pub to: Vec<u8>,
    /// The opening event's `ts`.
    pub at: Vec<u8>,
    /// The opening event's `body_file`.
    pub body_file: Vec<u8>,
    /// Routing key of the sender.
    ///
    /// A three-state member, not bytes. Under the ruled identity rule an ABSENT
    /// key and one present-but-EMPTY are different identities — the first falls
    /// back to a display name, the second names nobody — so a published row that
    /// flattened them would have thrown away what the comparison reads.
    pub from_slot: Key,
    /// Routing key of the target.
    pub to_slot: Key,
    /// Session of the sender's routing key.
    pub from_session: Key,
    /// Session of the target's routing key.
    pub to_session: Key,
    /// The DISPLAY summary: the request's own text while pending, the closing
    /// event's text once closed.
    pub summary: Vec<u8>,
}

/// The three terminal states of a request — SC-518.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// No valid closing event.
    Pending,
    /// Closed by a full-mirror reply.
    Replied,
    /// Withdrawn by its own sender. Terminal against a later reply.
    Cancelled,
}

impl Status {
    /// The token the table prints.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Replied => "replied",
            Self::Cancelled => "cancelled",
        }
    }
}

impl Request {
    /// Whether `mode` shows this row to `viewer`.
    ///
    /// **The SAME identity rule as closure**, and that is a CHOICE consequent on
    /// the SC-518 ruling rather than a thing the ruling says. "Is this row's
    /// sender me?" is an identity comparison between two participants, so
    /// answering it by a different rule than "did this reply come from the
    /// agent I asked?" would put two identity semantics in one module — and the
    /// frozen source's own warning is about exactly that kind of second copy.
    ///
    /// No corpus row constrains it: all 24 `mine`/`inbox` rows are identity
    /// refusals, so the filter never ran with a viewer at all. Named here as a
    /// choice so a seat can overrule it visibly.
    #[must_use]
    pub fn shown_to(&self, mode: Mode, viewer: &Viewer) -> bool {
        match mode {
            Mode::All => true,
            Mode::Mine => self.asker_identity().matches(viewer.identity()),
            Mode::Inbox => self.askee_identity().matches(viewer.identity()),
        }
    }

    /// This row's sender, as an identity.
    #[must_use]
    fn asker_identity(&self) -> Identity<'_> {
        identity_of(&self.from_slot, &self.from_session, &self.from)
    }

    /// This row's target, as an identity.
    #[must_use]
    fn askee_identity(&self) -> Identity<'_> {
        identity_of(&self.to_slot, &self.to_session, &self.to)
    }

    /// The table line for this row, `\n` included.
    fn write_line(&self, out: &mut Vec<u8>) {
        write_row(
            out,
            self.status.token().as_bytes(),
            &self.kind,
            &self.id,
            &self.from,
            &self.to,
            &self.summary,
        );
    }
}

/// `printf "%-8s %-8s %-28s %-20s %-20s %s\n"` — the one format, used for the
/// header and every row, so the two can never drift apart.
fn write_row(
    out: &mut Vec<u8>,
    status: &[u8],
    kind: &[u8],
    id: &[u8],
    from: &[u8],
    to: &[u8],
    summary: &[u8],
) {
    for (field, width) in [(status, 8), (kind, 8), (id, 28), (from, 20), (to, 20)] {
        pad_left_aligned(out, field, width);
        out.push(b' ');
    }
    out.extend_from_slice(summary);
    out.push(b'\n');
}

/// The refusal for `mine`/`inbox` when no identity could be detected.
///
/// Frozen bytes; the 24 A6 corpus rows pin them together with `rc=1`.
pub const NO_IDENTITY: &str =
    "Error: could not detect current agent identity; use 'requests all' outside an ae pane";

/// The exit status of the identity refusal.
///
/// **PINNED, not chosen.** 24 corpus rows record `rc=1` for it. The crate's
/// usage code stays `2` and belongs to the bad-mode token, which no corpus row
/// exercises — that split is the lead's D2 ruling, and it is why this constant
/// lives here rather than being folded into the usage path.
pub const EXIT_NO_IDENTITY: u8 = 1;

/// The header line, byte for byte.
///
/// Not a literal: the same [`write_row`] the rows go through, so a change to the
/// column widths cannot move the rows without moving the header with them.
#[must_use]
pub fn header() -> Vec<u8> {
    let mut out = Vec::new();
    write_row(
        &mut out, b"STATUS", b"TYPE", b"ID", b"FROM", b"TO", b"SUMMARY",
    );
    out
}

/// What the surface writes, and the status it exits with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// The table. Empty on a refusal — the frozen helper refuses BEFORE the
    /// header, so a refused invocation writes nothing to stdout at all.
    pub stdout: Vec<u8>,
    /// The refusal. Empty on success.
    pub stderr: Vec<u8>,
    /// `0` for a table, [`EXIT_NO_IDENTITY`] for the refusal.
    pub code: u8,
}

/// Render the surface over the session meta directory `dir`.
///
/// Infallible on purpose. The frozen sensor tests `[[ -f "$file" ]]` and returns
/// no rows when the container is absent, and its reader is
/// `_ae_tac "$file" 2>/dev/null || true` — so an existing container that cannot
/// be read produces no rows, no diagnostic and `rc=0`, exactly like an absent
/// one. That is SC-519's quiet-empty direction for this surface and NOT
/// SC-509b's degraded direction, which belongs to the typed reader in
/// [`crate::events`]: this table has nowhere to publish a degradation and the
/// frozen bytes show none.
#[must_use]
pub fn render(dir: &Path, mode: Mode, viewer: &Viewer) -> Output {
    if mode.needs_identity() && !viewer.is_known() {
        return Output {
            stdout: Vec::new(),
            stderr: format!("{NO_IDENTITY}\n").into_bytes(),
            code: EXIT_NO_IDENTITY,
        };
    }
    let container = read_container(&dir.join(CONTAINER));
    Output {
        stdout: table(&container, mode, viewer),
        stderr: Vec::new(),
        code: 0,
    }
}

/// The table for one container's bytes — the pure half of [`render`].
#[must_use]
pub fn table(container: &[u8], mode: Mode, viewer: &Viewer) -> Vec<u8> {
    let mut out = header();
    for request in states(container) {
        if request.shown_to(mode, viewer) {
            request.write_line(&mut out);
        }
    }
    out
}

/// THE request sensor — one definition, chronological, one row per request.
///
/// Reads `container` through the reversed framing the frozen helper reads it
/// through (see [`crate::event_text::reversed`]), so the scan is newest-first
/// and the opening `ask` — the oldest record for a ref — is reached last, which
/// is exactly why every terminal candidate has to be retained rather than
/// decided on sight.
#[must_use]
pub fn states(container: &[u8]) -> Vec<Request> {
    let stream = reversed(container);
    // Every retained record carries its SCAN ORDINAL — its position in the
    // container, i.e. LEDGER ORDER, which is APPEND ORDER and NEVER `ts`
    // (colead ruling, 2026-08-24). `ts` is read and published as a field and is
    // never compared: a writer's clock is not the ledger, and two records
    // sharing a timestamp still have an order while two records with skewed
    // clocks do not have the order their timestamps claim.
    //
    // The scan runs newest first, so a SMALLER ordinal is a NEWER event — which
    // is the only comparison SC-518a needs.
    let mut opened: HashMap<Vec<u8>, (usize, Opening)> = HashMap::new();
    let mut replies: HashMap<Vec<u8>, Vec<(usize, Closing)>> = HashMap::new();
    let mut cancels: HashMap<Vec<u8>, Vec<(usize, Closing)>> = HashMap::new();
    // Newest first, because that is the order they are met in.
    let mut refs: Vec<Vec<u8>> = Vec::new();
    let mut scan = 0_usize;

    for line in read_lines(&stream) {
        let Some(line) = event_line(line) else {
            continue;
        };
        // Counted for every brace-prefixed line, before the `ref` test, exactly
        // where the frozen sensor counts it.
        scan += 1;
        let reference = extract(line, "ref");
        if reference.is_empty() {
            continue;
        }
        match extract(line, "action").as_slice() {
            action @ (b"ask" | b"review") => {
                // First seen in a newest-first scan is the NEWEST opening.
                if opened.contains_key(&reference) {
                    continue;
                }
                opened.insert(reference.clone(), (scan, Opening::read(line, action)));
                refs.push(reference);
            }
            b"reply" => replies
                .entry(reference)
                .or_default()
                .push((scan, Closing::read(line))),
            b"cancel" => cancels
                .entry(reference)
                .or_default()
                .push((scan, Closing::read(line))),
            _ => {}
        }
    }

    // Reversing the newest-first encounter order gives chronological output.
    refs.reverse();
    refs.into_iter()
        .filter_map(|reference| {
            let (opened_at, opening) = opened.remove(&reference)?;
            let no_candidates = Vec::new();
            // **SC-518a** — a terminal event terminates only the newest opening
            // that PRECEDES it. The row shown for a ref is its newest opening,
            // so a terminal attaches to it exactly when the terminal came
            // after it: no newer opening can sit between them. A terminal
            // BEFORE this opening belonged to an earlier lifecycle of the same
            // ref and cannot reach forward into this one.
            //
            // `after` reads as `<` because the scan is newest-first.
            let after = |(at, _): &&(usize, Closing)| *at < opened_at;
            let cancelled = cancels.get(&reference).unwrap_or(&no_candidates);
            let answered = replies.get(&reference).unwrap_or(&no_candidates);
            // Candidates were appended newest-first, so the FIRST that passes
            // both tests is the newest that counts — and an invalid newer one
            // cannot bury a valid older one, because validity is decided here
            // and not during the scan.
            let cancel = cancelled
                .iter()
                .filter(after)
                .find(|(_, candidate)| opening.withdrawn_by(candidate));
            let reply = answered
                .iter()
                .filter(after)
                .find(|(_, candidate)| opening.answered_by(candidate));
            // A valid withdrawal wins over any reply, however late: a straggler
            // answer must not reopen a request nobody is waiting on.
            //
            // **UNRULED, AND NAMED AS SUCH.** Which KIND wins when a valid
            // cancel and a valid reply both follow one opening is a different
            // question from SC-518a, which decides which OPENING a terminal
            // attaches to. There is no corpus specimen — the 6,862-file corpus
            // contains no `cancel` event at all — so this keeps the frozen
            // precedence together with the frozen reason, and the choice is
            // recorded in the module docs rather than made invisibly.
            let (status, summary) = match (cancel, reply) {
                (Some((_, closing)), _) => (Status::Cancelled, closing.summary.clone()),
                (None, Some((_, closing))) => (Status::Replied, closing.summary.clone()),
                (None, None) => (Status::Pending, opening.summary.clone()),
            };
            Some(Request {
                status,
                kind: opening.kind,
                id: reference,
                from: opening.from,
                to: opening.to,
                at: opening.at,
                body_file: opening.body_file,
                from_slot: opening.from_slot,
                to_slot: opening.to_slot,
                from_session: opening.from_session,
                to_session: opening.to_session,
                summary,
            })
        })
        .collect()
}

/// One routing member, owned — [`Member`]'s three states, kept past the life of
/// the line they were read from.
///
/// [`Member`] is a VIEW: it borrows from the event line on the fast path, which
/// is right for an extractor and useless for a sensor that outlives the scan.
/// The three states survive the copy because they are what the identity rule
/// compares; collapsing `Absent` and `Empty` into "no bytes" here would undo
/// exactly what [`Member`] exists to preserve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// The key is not in the record.
    Absent,
    /// The key is present and resolves to nothing.
    Empty,
    /// The key carries a value.
    Value(Vec<u8>),
}

impl Key {
    /// The value, or `None` when the key is absent; `Empty` answers `Some(&[])`.
    #[must_use]
    pub fn value(&self) -> Option<&[u8]> {
        match self {
            Self::Absent => None,
            Self::Empty => Some(&[]),
            Self::Value(bytes) => Some(bytes),
        }
    }

    /// Read one routing member off an event line.
    fn read(line: &[u8], name: &str) -> Self {
        match member(line, name) {
            Member::Absent => Self::Absent,
            Member::Empty => Self::Empty,
            Member::Value(bytes) => Self::Value(bytes.into_owned()),
        }
    }
}

/// This side's participant, as [`Identity`] names one — or `Unassociated` when
/// the bytes cannot be one.
///
/// **The classification and the comparison both live in [`crate::events`]**, on
/// the type whose own doc has always stated the rule. There is no second copy
/// here: this function only converts what an OPAQUE read produced into the input
/// that shared rule takes. `session.rs` and this module now reach the same
/// [`Identity::matches`], which is what the seats ruled when they ruled SC-518
/// strict.
///
/// **The UTF-8 gate is a decision, and it is the ruling's own direction.**
/// [`Identity`] compares `&str`; this surface reads arbitrary bytes. A routing
/// member or display name that is not valid UTF-8 was never written by `ae` —
/// slots are `main`/`worker.<n>`/`spawned.<n>`, and both agent and session names
/// are ASCII allowlists — so this is unreachable through any real writer, and
/// no corpus fixture contains one. When it happens anyway the value cannot be
/// established as an identity, and SC-518's direction says what to do with an
/// identity you cannot establish: refuse to close. It answers
/// [`Identity::Unassociated`], which matches nothing. Comparing the raw bytes
/// instead would let two equally-unreadable slots close a request on the
/// strength of two values neither of which names an agent.
fn identity_of<'a>(slot: &'a Key, session: &'a Key, display: &'a [u8]) -> Identity<'a> {
    let text = |bytes: &'a [u8]| std::str::from_utf8(bytes).ok();
    match (slot, session) {
        (Key::Value(slot), Key::Value(session)) => match (text(slot), text(session)) {
            (Some(slot), Some(session)) => Identity::Routed { slot, session },
            _ => Identity::Unassociated,
        },
        (Key::Absent, Key::Absent) => match text(display) {
            Some(display) => Identity::Display(display),
            None => Identity::Unassociated,
        },
        _ => Identity::Unassociated,
    }
}

/// The `ask`/`review` that opened a request.
struct Opening {
    kind: Vec<u8>,
    from: Vec<u8>,
    to: Vec<u8>,
    at: Vec<u8>,
    body_file: Vec<u8>,
    /// Kept as KEYS rather than bytes: the ruled identity rule needs absent and
    /// present-but-empty told apart, and bytes cannot.
    from_slot: Key,
    to_slot: Key,
    from_session: Key,
    to_session: Key,
    summary: Vec<u8>,
}

/// A candidate `reply` or `cancel`. One shape for both: a cancel simply has no
/// target side, and reading the fields it does not carry as empty is what the
/// frozen sensor does too.
struct Closing {
    actor: Vec<u8>,
    target: Vec<u8>,
    actor_slot: Key,
    target_slot: Key,
    actor_session: Key,
    target_session: Key,
    summary: Vec<u8>,
}

impl Opening {
    fn read(line: &[u8], action: &[u8]) -> Self {
        Self {
            kind: action.to_vec(),
            from: extract(line, "actor"),
            to: extract(line, "target"),
            at: extract(line, "ts"),
            body_file: extract(line, "body_file"),
            from_slot: Key::read(line, "actor_slot"),
            to_slot: Key::read(line, "target_slot"),
            from_session: Key::read(line, "actor_session"),
            to_session: Key::read(line, "target_session"),
            summary: fold_newlines(extract(line, "summary")),
        }
    }

    /// This request's SENDER.
    fn asker(&self) -> Identity<'_> {
        identity_of(&self.from_slot, &self.from_session, &self.from)
    }

    /// This request's TARGET.
    fn askee(&self) -> Identity<'_> {
        identity_of(&self.to_slot, &self.to_session, &self.to)
    }

    /// **SC-518, strict** — the full mirror: the reply's actor is this
    /// request's TARGET and the reply's target is this request's SENDER, with
    /// both comparisons made by [`Identity::matches`]. A mixed pair closes
    /// nothing.
    fn answered_by(&self, reply: &Closing) -> bool {
        self.askee().matches(reply.actor_identity())
            && self.asker().matches(reply.target_identity())
    }

    /// **UNAUTHORIZED INTERIM — no row rules this.** SC-518 defines a REPLY
    /// MIRROR; a cancel has no target end to mirror, and cancel authorization
    /// has no row at all. Corrected contract text is awaited.
    ///
    /// What this does meanwhile: accept a cancel whose actor identity is the
    /// request's own sender, by the same strict [`Identity::matches`] the reply
    /// mirror uses. That is the narrowest thing that is not a guess — a state
    /// which closes a request is as consequential whether it closes it with an
    /// answer or without one — and it is the actor HALF of a rule whose other
    /// half does not apply, rather than an application of that rule.
    ///
    /// SC-518a's ORDER rule does govern cancel, and that part IS ruled. So the
    /// causality tests for cancel prove causality CONDITIONAL on whatever
    /// authorization is eventually ratified; they do not prove the
    /// authorization.
    fn withdrawn_by(&self, cancel: &Closing) -> bool {
        self.asker().matches(cancel.actor_identity())
    }
}

impl Closing {
    fn actor_identity(&self) -> Identity<'_> {
        identity_of(&self.actor_slot, &self.actor_session, &self.actor)
    }

    fn target_identity(&self) -> Identity<'_> {
        identity_of(&self.target_slot, &self.target_session, &self.target)
    }

    fn read(line: &[u8]) -> Self {
        Self {
            actor: extract(line, "actor"),
            target: extract(line, "target"),
            actor_slot: Key::read(line, "actor_slot"),
            target_slot: Key::read(line, "target_slot"),
            actor_session: Key::read(line, "actor_session"),
            target_session: Key::read(line, "target_session"),
            summary: fold_newlines(extract(line, "summary")),
        }
    }
}

/// The frozen sensor's `${_EV//$'\n'/ }` on every summary it stores.
///
/// PROVABLY A NO-OP through the frozen extractor, which already turns a `\n`
/// escape into a space, and a RAW newline inside a value would have ended the
/// line before the extractor ever saw it. Kept because the frozen code keeps
/// it and because this module's summaries are the last field of a line-framed
/// row: the day a value reaches here with a newline in it, one newline must not
/// become two rows.
fn fold_newlines(mut value: Vec<u8>) -> Vec<u8> {
    for byte in &mut value {
        if *byte == b'\n' {
            *byte = b' ';
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::{
        EXIT_NO_IDENTITY, Key, Mode, NO_IDENTITY, Status, Viewer, header, render, states, table,
    };
    use std::fs;
    use std::path::PathBuf;

    /// One session's events, as a `\n`-joined container.
    fn container(lines: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for line in lines {
            out.extend_from_slice(line.as_bytes());
            out.push(b'\n');
        }
        out
    }

    fn text(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).expect("test fixtures are utf-8")
    }

    const ASK: &str = r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","summary":"the question"}"#;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("ae-requests-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("scratch");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_header_is_the_frozen_bytes() {
        // Transcribed from the frozen capture
        // arms/A1/c01-healthy-ro/out/requests-all.stdout, whose first line this
        // is. Spelled out rather than built from the same widths the code uses,
        // because a header derived from the code under test asserts nothing.
        assert_eq!(
            text(&header()),
            "STATUS   TYPE     ID                           FROM                 TO                   SUMMARY\n"
        );
    }

    #[test]
    fn an_empty_container_is_the_header_alone() {
        assert_eq!(table(b"", Mode::All, &Viewer::default()), header());
    }

    #[test]
    fn a_missing_container_is_quiet_and_succeeds() {
        let scratch = Scratch::new("absent");
        let out = render(&scratch.0, Mode::All, &Viewer::default());
        assert_eq!(out.stdout, header());
        assert!(
            out.stderr.is_empty(),
            "SC-519: absent is quiet, not degraded"
        );
        assert_eq!(out.code, 0);
    }

    #[test]
    fn mine_and_inbox_refuse_before_the_header_when_no_identity_was_detected() {
        let scratch = Scratch::new("noid");
        fs::write(scratch.0.join("events.jsonl"), container(&[ASK])).expect("write");
        for mode in [Mode::Mine, Mode::Inbox] {
            let out = render(&scratch.0, mode, &Viewer::default());
            assert!(
                out.stdout.is_empty(),
                "{mode:?}: the refusal precedes the header"
            );
            assert_eq!(text(&out.stderr), format!("{NO_IDENTITY}\n"), "{mode:?}");
            assert_eq!(out.code, EXIT_NO_IDENTITY, "{mode:?}");
            assert_ne!(out.code, 2, "{mode:?}: pinned to 1, not the usage code");
        }
    }

    #[test]
    fn all_needs_no_identity() {
        let scratch = Scratch::new("all-noid");
        fs::write(scratch.0.join("events.jsonl"), container(&[ASK])).expect("write");
        let out = render(&scratch.0, Mode::All, &Viewer::default());
        assert_eq!(out.code, 0);
        assert!(text(&out.stdout).contains("the question"));
    }

    #[test]
    fn sc_518_a_full_mirror_reply_closes_and_carries_its_own_text() {
        let body = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"the answer"}"#,
        ]);
        let rows = states(&body);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, Status::Replied);
        assert_eq!(rows[0].summary, b"the answer", "the DISPLAY summary closes");
    }

    #[test]
    fn sc_518_a_reply_addressed_elsewhere_leaves_the_request_pending() {
        // Both halves of the mirror, one at a time. This is the exact drift the
        // frozen source records: a sensor that checked only the actor end.
        let wrong_target = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:third","ref":"r1","summary":"misaddressed"}"#,
        ]);
        let wrong_actor = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:third","action":"reply","target":"a:lead","ref":"r1","summary":"stranger"}"#,
        ]);
        for (body, why) in [(wrong_target, "target end"), (wrong_actor, "actor end")] {
            let rows = states(&body);
            assert_eq!(rows[0].status, Status::Pending, "{why}");
            assert_eq!(rows[0].summary, b"the question", "{why}");
        }
    }

    #[test]
    fn sc_518_routing_keys_decide_when_both_sides_carry_them() {
        // Same display names on both sides, opposed routing keys: the mirror
        // must fail on the keys, which name-matching alone would pass.
        let body = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","actor_slot":"main","actor_session":"s","target_slot":"worker.0","target_session":"s","summary":"q"}"#,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_slot":"spawned.3","actor_session":"s","target_slot":"main","target_session":"s","summary":"wrong slot"}"#,
        ]);
        assert_eq!(states(&body)[0].status, Status::Pending);

        let session_mismatch = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","actor_slot":"main","actor_session":"s","target_slot":"worker.0","target_session":"s","summary":"q"}"#,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_slot":"worker.0","actor_session":"other","target_slot":"main","target_session":"s","summary":"wrong session"}"#,
        ]);
        assert_eq!(states(&session_mismatch)[0].status, Status::Pending);
    }

    #[test]
    fn sc_518_a_keyless_request_is_not_closed_by_a_routed_reply() {
        // GAP 1, THE ASYMMETRIC DIRECTION — the shape the corpus does not have.
        //
        // Every mixed specimen in the corpus mixes ONE way: a fully routed
        // opening and an under-routed reply. So "mixed matches nothing in BOTH
        // directions" is a RULING and not a measurement, and a test that only
        // exercised the corpus's direction would pass on an implementation that
        // is directional. This is the other direction: a keyless (Display)
        // opening and a fully routed reply whose display names mirror it
        // perfectly. It closes NOTHING.
        //
        // Note what makes it sharp: the names DO mirror. An implementation that
        // reaches for the display fallback whenever either side lacks a key
        // closes this, and that is exactly the frozen defect the ruling
        // reverses.
        let body = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_slot":"worker.0","actor_session":"s","target_slot":"main","target_session":"s","summary":"routed reply"}"#,
        ]);
        let rows = states(&body);
        assert_eq!(rows[0].status, Status::Pending);
        assert_eq!(
            rows[0].summary, b"the question",
            "and it keeps the opening's own text"
        );

        // The control that makes the assertion mean something: the SAME opening
        // closed by a keyless reply, which is Display to Display and DOES
        // close. Without this arm the test would also pass on an
        // implementation that never closes anything.
        let both_keyless = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"keyless reply"}"#,
        ]);
        assert_eq!(states(&both_keyless)[0].status, Status::Replied);
    }

    #[test]
    fn sc_518_an_empty_routing_member_is_not_an_absent_one() {
        // GAP 5 (lexec's): the ONE shape where Empty and Absent diverge, and the
        // one a reader who thinks they are the same gets wrong while passing
        // every byte the corpus owns.
        //
        // Against a ROUTED opening both fail, which is why all four corpus
        // shapes agree. Against a DISPLAY-only opening they separate: an absent
        // pair is a Display identity and closes, while a present-and-empty pair
        // is Unassociated and closes nothing — a writer that meant to route and
        // did not say where has not thereby named the agent whose display name
        // happens to sit beside it.
        let keyless_reply = r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"absent keys"}"#;
        let empty_reply = r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_slot":"","actor_session":"","target_slot":"","target_session":"","summary":"empty keys"}"#;

        // ASK is keyless, so the opening is a Display identity.
        assert_eq!(
            states(&container(&[ASK, keyless_reply]))[0].status,
            Status::Replied,
            "absent on both sides is the display fallback"
        );
        assert_eq!(
            states(&container(&[ASK, empty_reply]))[0].status,
            Status::Pending,
            "present-and-empty is Unassociated, and matches nothing"
        );

        // And the pair that shows why the corpus could not tell them apart: a
        // routed opening refuses both, so every captured shape agrees.
        let routed_ask = r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","actor_slot":"main","actor_session":"s","target_slot":"worker.0","target_session":"s","summary":"q"}"#;
        for reply in [keyless_reply, empty_reply] {
            assert_eq!(
                states(&container(&[routed_ask, reply]))[0].status,
                Status::Pending,
                "a routed opening refuses both, which is why no capture separates them"
            );
        }
    }

    #[test]
    fn sc_518_half_a_routing_key_names_nobody_even_beside_a_display_opening() {
        // THE THIRD SHAPE IN THE Unassociated FAMILY, and the corpus cannot see
        // it either. `Empty` and `Absent` have a sibling: exactly ONE routing
        // member present. Against a ROUTED opening it fails like everything
        // else, which is why the A7 slot-only capture reads the same under both
        // the frozen rule and the ruling. Against a DISPLAY-only opening the
        // difference appears: `Unassociated` matches nothing, so it must NOT
        // fall through to the display name sitting beside it.
        //
        // Found by red-proof: an implementation that treats a half key as a
        // display identity passes every other test in this module and every one
        // of the 168 corpus rows.
        let half_keyed_reply = r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_slot":"worker.0","target_slot":"main","summary":"half a key"}"#;
        // ASK is keyless, so its participants are Display identities.
        assert_eq!(
            states(&container(&[ASK, half_keyed_reply]))[0].status,
            Status::Pending,
            "a slot with no session names nobody, and does not become its display name"
        );
        // The control, on the same opening: a fully keyless reply DOES close it,
        // so the assertion above is about the half key and not about the reply.
        let keyless_reply = r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"no keys"}"#;
        assert_eq!(
            states(&container(&[ASK, keyless_reply]))[0].status,
            Status::Replied
        );
        // And the other half of the pair, for completeness: a session with no
        // slot is the same species.
        let session_only_reply = r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_session":"s","target_session":"s","summary":"the other half"}"#;
        assert_eq!(
            states(&container(&[ASK, session_only_reply]))[0].status,
            Status::Pending
        );
    }

    #[test]
    fn an_identity_that_is_not_valid_utf8_names_nobody() {
        // THE UTF-8 GATE, as a test rather than as a paragraph. `Identity`
        // compares `&str` and this surface reads arbitrary bytes, so the
        // conversion needs a decision for the impossible case — and SC-518's
        // direction supplies it: an identity you cannot establish does not close
        // a request.
        //
        // Unreachable through any real writer (slots are `main`/`worker.<n>`/
        // `spawned.<n>`, agent and session names are ASCII allowlists, and no
        // fixture in the corpus carries one), which is exactly why it needs an
        // assertion: nothing else in the tree would ever notice if it silently
        // fell back to a display match.
        //
        // The two undecodable slots below are byte-IDENTICAL. A byte comparison
        // would close on them; the gate refuses, because two equally unreadable
        // values name no agent between them.
        let mut ask = Vec::from(
            &br#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","actor_slot":"main","actor_session":"s","target_slot":""#[..],
        );
        ask.extend_from_slice(&[0xFF, 0xFE]);
        ask.extend_from_slice(br#"","target_session":"s","summary":"q"}"#);
        let mut reply = Vec::from(
            &br#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","actor_slot":""#[..],
        );
        reply.extend_from_slice(&[0xFF, 0xFE]);
        reply.extend_from_slice(
            br#"","actor_session":"s","target_slot":"main","target_session":"s","summary":"a"}"#,
        );
        let mut body = ask;
        body.push(b'\n');
        body.extend_from_slice(&reply);
        body.push(b'\n');

        let rows = states(&body);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].status,
            Status::Pending,
            "identical undecodable slots are still not an identity"
        );
        // The member was READ — this is a gate on the comparison, not on the
        // extraction, and the published row still carries the bytes verbatim.
        assert_eq!(rows[0].to_slot, Key::Value(vec![0xFF, 0xFE]));
    }

    #[test]
    fn sc_518a_a_terminal_that_precedes_its_opening_closes_nothing() {
        // The ORDER rule alone, with identity deliberately PERFECT so a failure
        // can only be about ordering. This is the corpus's one specimen shape
        // (G5/m2) reduced to its mechanism.
        let reply_first = container(&[
            r#"{"ts":"t1","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"answered before it was asked"}"#,
            ASK,
        ]);
        let rows = states(&reply_first);
        assert_eq!(rows[0].status, Status::Pending);
        assert_eq!(rows[0].summary, b"the question");

        // The control, and it is what makes the assertion about ORDER: the same
        // two records the other way round DO close. Without this arm the test
        // would pass on an implementation whose identity rule rejected the
        // reply for an unrelated reason.
        let ask_first = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"answered after it was asked"}"#,
        ]);
        assert_eq!(states(&ask_first)[0].status, Status::Replied);
    }

    #[test]
    fn sc_518a_a_re_ask_opens_a_new_lifecycle_that_the_old_terminal_cannot_close() {
        // GAP 2. Identity is perfect throughout, so this test can ONLY fail on
        // the ordering rule — which is the point: a re-ask whose reply is also
        // identity-invalid would be pending for two reasons and could not tell
        // you which rule you had broken.
        let body = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"the first answer"}"#,
            r#"{"ts":"t3","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","summary":"asked again"}"#,
        ]);
        let rows = states(&body);
        assert_eq!(rows.len(), 1, "one row per ref");
        assert_eq!(
            rows[0].status,
            Status::Pending,
            "the new lifecycle is not born closed by the old lifecycle's reply"
        );
        assert_eq!(rows[0].summary, b"asked again");

        // And the earlier terminal is not merely ignored — a reply AFTER the
        // re-ask closes the new lifecycle, so the rule is about reach and not
        // about discarding replies.
        let answered_again = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"the first answer"}"#,
            r#"{"ts":"t3","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","summary":"asked again"}"#,
            r#"{"ts":"t4","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"the second answer"}"#,
        ]);
        let rows = states(&answered_again);
        assert_eq!(rows[0].status, Status::Replied);
        assert_eq!(rows[0].summary, b"the second answer");
    }

    #[test]
    fn sc_518a_a_cancel_before_its_opening_has_no_effect_whatever_authorization_says() {
        // GAP 3, AND OUTCOME-NEUTRAL BY CONSTRUCTION.
        //
        // SC-518a's ORDER rule is ruled and does govern cancel. Cancel
        // AUTHORIZATION is not ruled at all, so this test must not depend on
        // whether a given cancel would have been authorised — a gating test that
        // fails under a future authorization policy has ratified one BY
        // ENFORCEMENT whatever its comment says (semantic-contract.md, and the
        // rule came out of this very slice).
        //
        // The neutral formulation: a cancel placed BEFORE its opening leaves the
        // row IDENTICAL to a container that has no cancel in it at all. That is
        // exactly what "closes nothing" means, and it holds under every possible
        // authorization ruling, because a terminal that never reaches its
        // opening cannot be authorised to do anything to it. Whole rows are
        // compared, not statuses, so the summary cannot move either.
        let re_ask = r#"{"ts":"t3","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","summary":"asked again"}"#;
        let own =
            r#"{"ts":"t2","actor":"a:lead","action":"cancel","ref":"r1","summary":"withdrawn"}"#;
        let stranger =
            r#"{"ts":"t2","actor":"a:stranger","action":"cancel","ref":"r1","summary":"not mine"}"#;
        let anonymous = r#"{"ts":"t2","action":"cancel","ref":"r1","summary":"nobody"}"#;

        let without = states(&container(&[ASK]));
        for cancel in [own, stranger, anonymous] {
            // The cancel precedes the opening. Whoever sent it, it is a no-op.
            let with = states(&container(&[cancel, ASK]));
            assert_eq!(
                with, without,
                "a pre-opening cancel changed the row: {cancel}"
            );
        }

        // The same rule from the re-ask side: an earlier lifecycle's cancel
        // cannot reach forward, so the re-asked row is identical to the row of a
        // container holding only the re-ask. Also neutral — it says nothing
        // about what the cancel did to the lifecycle it COULD reach.
        let only_re_ask = states(&container(&[re_ask]));
        for cancel in [own, stranger, anonymous] {
            let re_asked = states(&container(&[ASK, cancel, re_ask]));
            assert_eq!(
                re_asked, only_re_ask,
                "an earlier lifecycle's cancel reached the re-ask: {cancel}"
            );
        }
    }

    /// **NON-GATING DIAGNOSTIC** — a `cancel` AND a `reply` both attached to one
    /// opening. GAP 4, and the gate now says NOTHING about it.
    ///
    /// My first attempt at this asserted that the outcome does not depend on the
    /// two terminals' arrival order, on the reasoning that naming no winner made
    /// it neutral. `gpt56terra:pubfp` showed that it does not: **"the later
    /// terminal wins" is a legitimate unresolved resolver policy**, and under it
    /// `cancel`-then-`reply` yields `Replied` while `reply`-then-`cancel` yields
    /// `Cancelled` — so the two are NOT equal, and an equality assertion fails
    /// that policy. Equality across arrival orders is itself a rule about which
    /// terminal controls the row: it rules out recency-based precedence. Which
    /// is exactly the undecided question, one level up from the one I thought I
    /// was avoiding.
    ///
    /// So there is no outcome-neutral gating assertion available for this shape.
    /// Both terminals attaching is already gated by the single-terminal tests,
    /// and the only observable of it here — the row being terminal at all — is
    /// witnessed by the ratified REPLY on its own. Nothing is lost by moving the
    /// whole shape out of the gate, and a precedence law would have been
    /// smuggled in by keeping any part of it.
    #[test]
    #[ignore = "records unratified two-terminal precedence behavior; not a gate"]
    fn diagnostic_two_terminals_on_one_opening() {
        let cancel_then_reply = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:lead","action":"cancel","ref":"r1","summary":"never mind"}"#,
            r#"{"ts":"t3","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"too late"}"#,
        ]);
        let reply_then_cancel = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"answered"}"#,
            r#"{"ts":"t3","actor":"a:lead","action":"cancel","ref":"r1","summary":"withdrawn anyway"}"#,
        ]);
        // IS, not contract, and BOTH orders are recorded rather than related:
        // this build resolves by KIND (cancellation wins) rather than by
        // recency, so the two agree — but that agreement is a property of the
        // current resolver and not of anything ruled.
        assert_eq!(states(&cancel_then_reply)[0].status, Status::Cancelled);
        assert_eq!(states(&reply_then_cancel)[0].status, Status::Cancelled);
    }

    /// **NON-GATING DIAGNOSTIC — `#[ignore]`d ON PURPOSE, and the attribute is
    /// the whole point.**
    ///
    /// This records what this build currently DOES about cancel authorization,
    /// which no row rules. `semantic-contract.md` permits a clearly non-gating
    /// diagnostic to record the current IS, and forbids a GATING test from
    /// asserting an unratified authorization outcome — because a gate that fails
    /// under the other policy has ratified this one by enforcement, whatever its
    /// comment says. `cargo nextest run` skips ignored tests and no lane passes
    /// `--run-ignored`, so nothing here can fail the gate; run it deliberately
    /// with `cargo nextest run --run-ignored all` to read the current behavior.
    ///
    /// When cancel authorization is ratified, this becomes a gating test by
    /// deleting one attribute — and if the ruling differs from what is below,
    /// this is the record of what had to change.
    #[test]
    #[ignore = "records unratified cancel-authorization behavior; not a gate"]
    fn diagnostic_the_interim_withdrawal_policy_this_build_applies() {
        let after = |actor: &str, summary: &str| {
            let cancel = format!(
                r#"{{"ts":"t2",{actor}"action":"cancel","ref":"r1","summary":"{summary}"}}"#
            );
            states(&container(&[ASK, &cancel]))[0].status
        };
        // IS, not contract: the request's own sender withdraws it; a stranger
        // and an unattributed cancel do not.
        assert_eq!(
            after(r#""actor":"a:lead","#, "withdrawn"),
            Status::Cancelled
        );
        assert_eq!(
            after(r#""actor":"a:stranger","#, "not mine"),
            Status::Pending
        );
        assert_eq!(after("", "nobody"), Status::Pending);
    }

    #[test]
    fn an_invalid_newer_reply_cannot_bury_a_valid_older_one() {
        // The measured regression the frozen sensor's retain-then-validate shape
        // exists to prevent: keeping only the newest RAW candidate and
        // validating it afterwards rendered this `pending`.
        //
        // REPLIES ONLY. The identical shape with two cancels is the same
        // mechanism, but asserting its outcome would assert who may cancel —
        // unratified — so it lives in the non-gating diagnostic below instead.
        // Reply authorization IS ratified (SC-518), so this half is a gate.
        let replies = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"valid answer"}"#,
            r#"{"ts":"t3","actor":"a:stranger","action":"reply","target":"a:lead","ref":"r1","summary":"stranger answer"}"#,
        ]);
        let rows = states(&replies);
        assert_eq!(rows[0].status, Status::Replied);
        assert_eq!(rows[0].summary, b"valid answer");
    }

    /// **NON-GATING DIAGNOSTIC** — the cancel half of retain-then-validate.
    /// Same mechanism as the reply half above; its outcome depends on who may
    /// cancel, which no row rules. See the other diagnostic for why the
    /// attribute rather than a comment is what keeps this out of the gate.
    #[test]
    #[ignore = "records unratified cancel-authorization behavior; not a gate"]
    fn diagnostic_retain_then_validate_over_two_cancels() {
        let body = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:lead","action":"cancel","ref":"r1","summary":"valid withdrawal"}"#,
            r#"{"ts":"t3","actor":"a:stranger","action":"cancel","ref":"r1","summary":"stranger cancel"}"#,
        ]);
        let rows = states(&body);
        assert_eq!(rows[0].status, Status::Cancelled);
        assert_eq!(rows[0].summary, b"valid withdrawal");
    }

    #[test]
    fn the_newest_valid_candidate_of_a_kind_is_the_one_that_counts() {
        let body = container(&[
            ASK,
            r#"{"ts":"t2","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"older answer"}"#,
            r#"{"ts":"t3","actor":"a:worker","action":"reply","target":"a:lead","ref":"r1","summary":"newer answer"}"#,
        ]);
        assert_eq!(states(&body)[0].summary, b"newer answer");
    }

    #[test]
    fn one_row_per_ref_ordered_by_the_opening_that_survived() {
        let body = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:w","ref":"r1","summary":"first"}"#,
            r#"{"ts":"t2","actor":"a:lead","action":"review","target":"a:w","ref":"r2","summary":"second"}"#,
            r#"{"ts":"t3","actor":"a:lead","action":"ask","target":"a:w","ref":"r1","summary":"reopened"}"#,
        ]);
        let rows = states(&body);
        assert_eq!(rows.len(), 2, "one row per ref");
        assert_eq!(rows[0].kind, b"review", "review opens a request too");
        assert_eq!(rows[1].kind, b"ask");
        // THE ORDER IS BY THE SURVIVING OPENING, NOT BY THE FIRST ONE. r1 was
        // asked at t1 and re-asked at t3; the row keeps the t3 opening, and its
        // POSITION follows t3 too — so r2 (t2) precedes r1 (t3) even though r1's
        // earliest ask is the oldest event in the container. Ordinary logs never
        // show this, because a ref is opened once and the ordering collapses to
        // chronological.
        //
        // Not derived from a reading: the frozen `_ar_request_states`, extracted
        // at 72c7293 and run over these exact bytes, emits r2 before r1. The
        // corpus does not cover a re-asked ref, so this expectation is IS from a
        // source-plus-execution read of the frozen sensor and NOT from a capture.
        assert_eq!(rows[0].id, b"r2");
        assert_eq!(rows[1].id, b"r1");
        assert_eq!(
            rows[1].summary, b"reopened",
            "the newest opening for a ref wins"
        );
    }

    #[test]
    fn a_line_without_a_ref_or_a_brace_is_not_a_request() {
        let body = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:w","summary":"no ref"}"#,
            r"not json at all",
            r#" {"ts":"t2","actor":"a:lead","action":"ask","ref":"r9","summary":"leading space"}"#,
            r#"{"ts":"t3","actor":"a:lead","action":"state","ref":"working","summary":"not a request"}"#,
        ]);
        assert!(states(&body).is_empty());
    }

    #[test]
    fn an_unterminated_tail_is_glued_to_the_line_before_it_and_not_dropped() {
        // The measured `tail -r` framing, at this surface: the remainder runs
        // into the previous line, so BOTH are lost as separate records — the
        // request that was on the previous line disappears.
        let mut body = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:w","ref":"r1","summary":"survives"}"#,
            r#"{"ts":"t2","actor":"a:lead","action":"ask","target":"a:w","ref":"r2","summary":"eaten"}"#,
        ]);
        body.extend_from_slice(br#"{"ts":"t3","actor":"a:lead","action":"as"#);
        let rows = states(&body);
        assert_eq!(rows.len(), 1, "r2 was consumed by the glue: {rows:?}");
        assert_eq!(rows[0].id, b"r1");
    }

    #[test]
    fn mine_and_inbox_select_opposite_sides_of_the_same_row() {
        let body = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:worker","ref":"r1","actor_slot":"main","actor_session":"s","target_slot":"worker.0","target_session":"s","summary":"q"}"#,
        ]);
        let lead = Viewer {
            slot: "main".to_owned(),
            session: "s".to_owned(),
            display: "someone-else".to_owned(),
        };
        let worker = Viewer {
            slot: "worker.0".to_owned(),
            session: "s".to_owned(),
            display: "someone-else".to_owned(),
        };
        assert!(text(&table(&body, Mode::Mine, &lead)).contains("r1"));
        assert_eq!(table(&body, Mode::Inbox, &lead), header());
        assert!(text(&table(&body, Mode::Inbox, &worker)).contains("r1"));
        assert_eq!(table(&body, Mode::Mine, &worker), header());
        // The routing keys decide even though the display ref matches neither.
        assert!(text(&table(&body, Mode::All, &Viewer::default())).contains("r1"));
    }

    #[test]
    fn the_filter_uses_the_same_identity_rule_as_closure() {
        // A CHOICE, not a ruling — see `Request::shown_to`. No corpus row
        // constrains the filter (all 24 mine/inbox rows are refusals), and this
        // pins the choice so a seat can overrule it visibly.
        let keyless = container(&[ASK]);
        // A keyless ROW is a Display identity; a ROUTED viewer is not it, even
        // though the display names agree.
        let routed_viewer = Viewer {
            slot: "main".to_owned(),
            session: "s".to_owned(),
            display: "a:lead".to_owned(),
        };
        assert_eq!(
            table(&keyless, Mode::Mine, &routed_viewer),
            header(),
            "mixed identity does not select a row either"
        );
        // A viewer with no routing keys IS a Display identity, and matches.
        let display_viewer = Viewer {
            slot: String::new(),
            session: String::new(),
            display: "a:lead".to_owned(),
        };
        assert!(text(&table(&keyless, Mode::Mine, &display_viewer)).contains("r1"));
        // And the same viewer does not collect somebody else's row.
        let stranger = Viewer {
            slot: String::new(),
            session: String::new(),
            display: "a:third".to_owned(),
        };
        assert_eq!(table(&keyless, Mode::Mine, &stranger), header());
    }

    #[test]
    fn the_summary_never_carries_a_newline_into_the_row() {
        let body = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:w","ref":"r1","summary":"one\ntwo"}"#,
        ]);
        let rendered = table(&body, Mode::All, &Viewer::default());
        assert_eq!(
            text(&rendered).lines().count(),
            2,
            "header plus exactly one row: {}",
            text(&rendered)
        );
        assert!(text(&rendered).contains("one two"));
    }

    #[test]
    fn an_over_long_field_overflows_its_column_instead_of_truncating() {
        let body = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"review","target":"a:w","ref":"review-20260820T161305Z-dc302d09","summary":"s"}"#,
        ]);
        assert!(
            text(&table(&body, Mode::All, &Viewer::default()))
                .contains("review-20260820T161305Z-dc302d09 a:lead"),
            "one space after an over-wide id, as the A6 capture shows"
        );
    }

    #[test]
    fn mode_tokens_are_exactly_the_three() {
        assert_eq!(Mode::parse(None), Some(Mode::Mine));
        assert_eq!(Mode::parse(Some("mine")), Some(Mode::Mine));
        assert_eq!(Mode::parse(Some("inbox")), Some(Mode::Inbox));
        assert_eq!(Mode::parse(Some("all")), Some(Mode::All));
        for token in ["", "MINE", "al", "alll", "--json", "-a"] {
            assert_eq!(Mode::parse(Some(token)), None, "{token}");
        }
        assert!(Mode::Mine.needs_identity());
        assert!(Mode::Inbox.needs_identity());
        assert!(!Mode::All.needs_identity());
    }

    #[test]
    fn every_status_has_its_own_token() {
        assert_eq!(Status::Pending.token(), "pending");
        assert_eq!(Status::Replied.token(), "replied");
        assert_eq!(Status::Cancelled.token(), "cancelled");
    }

    #[test]
    fn the_opening_fields_the_sensor_carries_are_all_read() {
        // The sensor's row is wider than the table. `ts` and `body_file` are
        // read by the archive digest from the same rows, so a port that dropped
        // them would silently narrow a shared sensor.
        let body = container(&[
            r#"{"ts":"2026-08-20T16:12:55Z","actor":"a:lead","action":"ask","target":"a:w","ref":"r1","body_file":"/m/r1.ask.txt","actor_slot":"main","target_slot":"worker.0","actor_session":"s","target_session":"s","summary":"q"}"#,
        ]);
        let row = &states(&body)[0];
        assert_eq!(row.at, b"2026-08-20T16:12:55Z");
        assert_eq!(row.body_file, b"/m/r1.ask.txt");
        assert_eq!(row.from_slot, Key::Value(b"main".to_vec()));
        assert_eq!(row.to_slot, Key::Value(b"worker.0".to_vec()));
        assert_eq!(row.from_session, Key::Value(b"s".to_vec()));
        assert_eq!(row.to_session, Key::Value(b"s".to_vec()));
        // And the three states are told apart on a published row, which is the
        // whole reason these are Keys and not bytes.
        let mixed = container(&[
            r#"{"ts":"t1","actor":"a:lead","action":"ask","target":"a:w","ref":"r2","actor_slot":"","summary":"q"}"#,
        ]);
        let row = &states(&mixed)[0];
        assert_eq!(row.from_slot, Key::Empty, "present and empty");
        assert_eq!(row.from_session, Key::Absent, "not in the record at all");
        assert_eq!(row.from_slot.value(), Some(b"".as_slice()));
        assert_eq!(row.from_session.value(), None);
    }
}
