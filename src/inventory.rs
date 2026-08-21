//! Which sessions EXIST, before anything asks whether they are running.
//!
//! **SC-017j** — the inventory is the union of (a) durable session state under
//! SC-400d's two readable layouts and (b) positively identified ae-owned live
//! tmux sessions on a server ae is already entitled to query. Archives are inert
//! and never enter it. Every durable candidate survives into classification: a
//! failed liveness query, a prefix-only name match, or a live exact-name session
//! whose ownership marker is missing cannot delete the candidate.
//!
//! # The invariant this module exists to hold
//!
//! > A candidate never disappears because a liveness query failed,
//! > prefix-matched, or found a live session without its marker.
//!
//! Collapsing discovery into liveness is what produced #105, where two disjoint
//! enumerators each removed the same durable directory for a different reason.
//! So **nothing here classifies liveness** — [`Candidate`] carries no status at
//! all, and there is no code path from this module to `digest`'s status type.
//!
//! Three structural consequences, all deliberate:
//!
//! * **The state DIRECTORY establishes the candidate; the `meta` only fills it
//!   in.** SC-400d: "presence of the state directory is sufficient for
//!   discovery". The record is built from the directory and the read only
//!   populates fields, so an absent, unreadable or malformed `meta` costs facts
//!   and never a candidate. What the read outcome WAS is carried ([`MetaRead`]),
//!   because SC-509b derives degradation from it later and re-running discovery
//!   to re-learn it would be the expensive kind of forgetting.
//! * **Identity is the root-qualified state directory, never the leaf.**
//!   SC-400d: equal leaves across paths never deduplicate. The `<session-name>`
//!   leaf is the inventory NAME; the path is the identity.
//! * **Discovery completes before reconciliation.** SC-017j says so in those
//!   words, and the reason is visible in the code: every sighting is gathered
//!   first, so which server answered first cannot decide which candidate a
//!   sighting joins.
//!
//! # Entitlement — a finite, pointer-derived set
//!
//! ae may enumerate a tmux server only when it already holds a pointer to it:
//! the ambient server this invocation's ordinary transport selected, or a
//! positive, unambiguous selector recorded by a durable candidate (SC-405l).
//! A missing or ambiguous selector confers no entitlement. Sweeping arbitrary
//! socket paths or server names is not a way to gain one.
//!
//! A live session on a server outside that set is **absent by epistemic limit**
//! — not stopped, not unknown, not there at all — and becomes visible later when
//! an ambient selection or a durable record supplies the pointer.
//!
//! # What this module must be TOLD
//!
//! * **The ambient server.** SC-017j consumes the selection this invocation's
//!   transport already made; it does not ratify the environment control that
//!   makes it (`AE_TMUX_SERVER` is SC-1410c, still unclassified).
//! * **How to enumerate a server.** [`Discovery`] is the seam: this crate has no
//!   tmux transport yet, and phase 2 owns the one it grows. It is a CALLED port
//!   rather than injected data on purpose — entitlement that filters a list
//!   somebody else gathered is satisfied by a sweeper that queries everything
//!   and discards the surplus, and the trace is where that shows. Here the only
//!   servers ever contacted are the ones [`entitled_servers`] returned, and a
//!   recording double can prove it.
//!
//!   The port is enumeration-shaped — server in, sessions out. There is no
//!   `has_session(server, name)`, so phase 1 CANNOT ask an existence question
//!   about a durable candidate even by accident; that shape is what turned a
//!   prefix match into a deletion in #105.
//! * **The state root.** SC-404 derives the roots from `AE_HOME`; which value
//!   `AE_HOME` has is SC-1410a, also unclassified, so [`Roots`] is handed the
//!   home rather than reading the environment.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::meta::{Selector, ServerSelector};
use crate::session::RecordSnapshot;

/// The nested state directory inside a worktree — SC-400d's legacy layout.
const WORKTREE_STATE_DIR: &str = ".ae";

/// The ae state roots for one invocation, derived from `AE_HOME` (SC-404).
///
/// **The archive is deliberately not reachable from this type.** SC-017j says
/// archives never enter the inventory, and the cheapest way to hold that is to
/// give the inventory reader no way to name the archive root at all — a rule
/// that cannot be expressed in the code that must obey it is a rule that gets
/// obeyed until someone is in a hurry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    sessions: PathBuf,
    worktrees: PathBuf,
}

impl Roots {
    /// The roots under `ae_home` — SC-404's default derivation, both of
    /// SC-400d's layouts.
    #[must_use]
    pub fn under<P: Into<PathBuf>>(ae_home: P) -> Self {
        let home = ae_home.into();
        Self {
            sessions: home.join("sessions"),
            worktrees: home.join("worktrees"),
        }
    }

    /// The canonical root, `<AE_HOME>/sessions`.
    #[must_use]
    pub fn sessions(&self) -> &Path {
        &self.sessions
    }

    /// The legacy worktree root, `<AE_HOME>/worktrees`.
    #[must_use]
    pub fn worktrees(&self) -> &Path {
        &self.worktrees
    }
}

/// Which SC-400d layout a durable candidate was found in.
///
/// Part of its provenance rather than a formatting detail: the two layouts spell
/// a session's name in different places, and a candidate that forgot which one
/// it came from could not say why its name is what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// `<AE_HOME>/sessions/<session-name>/`.
    Canonical,
    /// `<AE_HOME>/worktrees/<worktree-name>/.ae/<session-name>/`, where the
    /// outer worktree name and the inner session name may differ. The INNER
    /// leaf is the inventory name.
    WorktreeNested,
}

/// What happened when the record was read.
///
/// Re-exported from the module that PERFORMS the read: the outcome and the read
/// must not be able to disagree, and the only way to guarantee that is for the
/// same call to produce both.
pub use crate::session::MetaRead;

/// Durable session state found under one of SC-400d's roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableRecord {
    /// The state directory. THE identity of this candidate, root-qualified, so
    /// two paths sharing a leaf are two candidates.
    pub path: PathBuf,
    /// The `<session-name>` leaf — the inventory name (SC-400d).
    ///
    /// Lossy for a non-UTF-8 name; the bytes survive in `path`, which is what
    /// any later read must use. A name ae cannot spell is still a candidate:
    /// dropping it would be exactly the disappearance this module forbids.
    pub name: String,
    /// Which layout it was found in.
    pub layout: Layout,
    /// The SC-405l normalized server selector.
    pub server: ServerSelector,
    /// What reading its `meta` did — see [`MetaRead`].
    pub meta_read: MetaRead,
    /// Everything the record said, read ONCE, at discovery.
    ///
    /// **The phase-2 gate's criterion 14 binds the whole phase, not just
    /// classification**, and this field is what makes that possible: the digest
    /// is assembled from these bytes rather than from a second read at emission
    /// time. A second observation would let a record that was unreadable here
    /// become readable before rendering and repair its own loss fact, and would
    /// print record facts beside a liveness answer that never held at the same
    /// moment.
    ///
    /// This costs no extra I/O. The selector above already required opening the
    /// `meta`; carrying what that read produced is strictly cheaper than
    /// reading it again later.
    pub snapshot: RecordSnapshot,
}

/// A tmux server ae holds a pointer to.
///
/// Two ways to hold one, and they are never assumed equivalent: SC-017j rules
/// that "unproved equivalence between selector spellings never authorizes a
/// merge", so an ambient server and a recorded `name`/`socket` selector are
/// distinct identities here even when a human can see they address the same
/// tmux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerId {
    /// The server this invocation's ordinary transport already selected, with
    /// no explicit selector of its own. SC-1410c owns how it was selected; this
    /// phase consumes the selection.
    Ambient,
    /// A server named by a positive, unambiguous durable selector (SC-405l).
    Selected(Selector),
}

/// One session an entitled server reported, before ae decides whether it is its
/// own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSession {
    /// The exact session name the server reported.
    pub name: String,
    /// The `AE_SESSION` marker the server holds for it, if any.
    ///
    /// Presence is the ownership evidence SC-017j names ("whose ownership
    /// marker is missing"), and it is what the incumbent tests. Whether a marker
    /// that is PRESENT but names a different session is still positive evidence
    /// is SC-017l's "missing/mismatched" wording — a phase-2 question, and one
    /// this phase deliberately does not answer: guessing it here could only
    /// REMOVE a live session from the inventory, which is the direction this
    /// whole ruling exists to forbid.
    pub marker: Option<String>,
}

/// A server query that did not answer.
///
/// Carries nothing yet. The REASON matters to SC-017l, which turns it into
/// `unknown` one phase later; inventing a reason taxonomy here would be writing
/// that row's content early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryFailed;

/// The one tmux question phase 1 is allowed to ask.
///
/// Enumeration only, and only of a server [`entitled_servers`] returned. There
/// is deliberately no existence check: SC-017j lets ae enumerate an entitled
/// server and read ownership for the names that come back, and nothing else.
pub trait Discovery {
    /// Every session `server` reports, with its ownership marker.
    ///
    /// # Errors
    ///
    /// [`QueryFailed`] when the server did not answer — unreachable, no such
    /// server, transport error. A failure removes nothing: it is the absence of
    /// new candidates, never the absence of existing ones.
    fn enumerate(&self, server: &ServerId) -> Result<Vec<DiscoveredSession>, QueryFailed>;
}

/// A live session ae positively identified as its own, on a server it was
/// entitled to ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSighting {
    /// The server that answered. Not "the ambient server" — the one that
    /// answered, which is what makes the entitlement trace checkable.
    pub server: ServerId,
    /// Its exact session name.
    pub name: String,
    /// The `AE_SESSION` value the server reported.
    pub marker: String,
}

/// Which sources established a candidate.
///
/// **SC-509b needs this and it is not the same question as damage.** Source
/// membership says whether a durable record EXISTS; [`MetaRead`] says whether it
/// could be read. Collapsing them would let the digest report a destroyed record
/// as a session that never had one.
///
/// Derived from the two source fields, never stored beside them: a provenance
/// that can disagree with the sources it describes is one that eventually will.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// Durable state only — no entitled server reported it.
    Durable,
    /// A live session only — no durable record was found for it.
    Live,
    /// Both, positively matched. The union coalesced exactly one candidate.
    Both,
}

/// One inventory candidate: durable state, a live sighting, or both.
///
/// Deliberately carries no status. Liveness is SC-017k/SC-017l's question, one
/// phase later, and a status field here would be a place to answer it early.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The session name this candidate is known by.
    pub name: String,
    /// The durable record, when there is one.
    pub durable: Option<DurableRecord>,
    /// The live sighting, when one was positively joined to this candidate.
    pub live: Option<LiveSighting>,
}

impl Candidate {
    /// Which sources established this candidate.
    #[must_use]
    pub const fn provenance(&self) -> Provenance {
        match (self.durable.is_some(), self.live.is_some()) {
            (true, true) => Provenance::Both,
            (true, false) => Provenance::Durable,
            (false, _) => Provenance::Live,
        }
    }

    /// A candidate that exists only because a durable record does.
    #[must_use]
    pub fn durable(record: DurableRecord) -> Self {
        Self {
            name: record.name.clone(),
            durable: Some(record),
            live: None,
        }
    }

    /// A candidate that exists only because a live session was seen.
    ///
    /// SC-017j: it "remains visible"; the absence of a durable record is the
    /// separate SC-509b `degraded` fact, not a reason to drop it.
    #[must_use]
    pub fn tmux_only(sighting: LiveSighting) -> Self {
        Self {
            name: sighting.name.clone(),
            durable: None,
            live: Some(sighting),
        }
    }

    /// Whether this candidate has no durable record behind it.
    #[must_use]
    pub const fn is_tmux_only(&self) -> bool {
        self.durable.is_none()
    }
}

/// A logical source whose terminal enumeration failed — SC-017o.
///
/// The row names three classes, and each is a place where ae asked a question
/// and got no answer. What they have in common is the fact worth recording:
/// **absence in this snapshot is not proof**. A listing that silently omits an
/// unknowable number of sessions asserts a completeness it did not establish,
/// which is the confident-empty shape #105 exists to remove.
///
/// What is NOT here matters as much. A missing durable root or an absent `.ae`
/// subtree is an AUTHORITATIVE EMPTY SOURCE — it answered, and the answer was
/// "nothing". Archives and servers outside the entitled set were never required.
/// And a candidate directory that WAS discovered, whose `meta` will not read,
/// stays that candidate's own SC-405i/SC-509b record-loss fact: it never becomes
/// enumeration incompleteness, because nothing about the enumeration failed.
///
/// A loss fact names the SOURCE, never a session: the useful fact is not which
/// sessions were lost — nobody can know that — but that some may have been.
/// Guessing an identity here would be the fabrication SC-017o forbids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailedSource {
    /// `<AE_HOME>/sessions` exists and would not enumerate.
    CanonicalRoot(PathBuf),
    /// `<AE_HOME>/worktrees` exists and would not enumerate, hiding every
    /// worktree beneath it.
    WorktreeRoot(PathBuf),
    /// A discovered `<worktree>/.ae` exists and would not enumerate.
    WorktreeState(PathBuf),
    /// An entitled tmux server did not answer.
    Server(ServerId),
}

/// What the durable scan found, and which sources failed to answer.
///
/// The loss list is the third member of a family this build has now separated
/// four times: **never-asked is not unreachable**, **record-absent is not
/// record-unreadable**, **unlistable is not absent**, and now **a source that
/// failed is not a source that answered "nothing"**. Each pair is two epistemic
/// states that a tidier design would render identically.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DurableScan {
    /// Every durable candidate found.
    pub records: Vec<DurableRecord>,
    /// Durable sources whose enumeration failed (SC-017o).
    ///
    /// Never candidates and never a source of one — nothing can be named inside
    /// a directory that will not enumerate.
    pub incomplete: Vec<FailedSource>,
}

impl From<Vec<DurableRecord>> for DurableScan {
    /// A scan that found these records and enumerated every source it needed.
    fn from(records: Vec<DurableRecord>) -> Self {
        Self {
            records,
            incomplete: Vec::new(),
        }
    }
}

/// Every durable candidate under `roots`, both SC-400d layouts, path order.
///
/// A candidate is a state DIRECTORY: `<sessions>/<name>/`, or
/// `<worktrees>/<worktree>/.ae/<name>/` where the two names may differ and the
/// inner one is the inventory name. A bare worktree directory with no nested
/// state directory is not a candidate.
///
/// The `meta` inside each is read for its SC-405l selector ONLY, and a read that
/// fails costs the selector rather than the candidate: the record is built from
/// the directory first, and the read fills fields into it.
///
/// **No source failure ends the scan** (SC-017o: "discovery continues and every
/// candidate found from other sources survives"). This function is therefore
/// infallible by construction — there is no `Result` left to return, because
/// every way it can fail is a fact it records and carries on from. An earlier
/// shape propagated the canonical root's error with `?`, which meant an
/// unlistable `<AE_HOME>/sessions` also cost every candidate in every worktree:
/// the exact "one bad subtree must not cost every candidate elsewhere" property
/// that the nested arm was already written to hold.
///
/// The order is by path, not by traversal: `read_dir` order is a filesystem fact
/// that differs between platforms and between runs. This is internal determinism
/// only — the ORDER a listing shows is SC-017n's, applied later.
#[must_use]
pub fn durable_records(roots: &Roots) -> DurableScan {
    let mut scan = DurableScan::default();

    match child_dirs(roots.sessions()) {
        Ok(paths) => scan
            .records
            .extend(paths.into_iter().map(|p| record_at(p, Layout::Canonical))),
        // An ABSENT root never reaches here: `child_dirs` answers it with an
        // empty list, because a machine that never ran ae has no sessions and
        // that is an answer, not a failure.
        Err(_) => scan
            .incomplete
            .push(FailedSource::CanonicalRoot(roots.sessions().to_path_buf())),
    }

    match child_dirs(roots.worktrees()) {
        Ok(worktrees) => {
            for worktree in worktrees {
                // SC-400d: the candidate is the NESTED state directory. A bare
                // worktree is a checkout, not a session — and an absent `.ae` is
                // likewise an authoritative empty answer, not a loss.
                let state_root = worktree.join(WORKTREE_STATE_DIR);
                match child_dirs(&state_root) {
                    Ok(states) => scan.records.extend(
                        states
                            .into_iter()
                            .map(|p| record_at(p, Layout::WorktreeNested)),
                    ),
                    Err(_) => scan
                        .incomplete
                        .push(FailedSource::WorktreeState(state_root)),
                }
            }
        }
        Err(_) => scan
            .incomplete
            .push(FailedSource::WorktreeRoot(roots.worktrees().to_path_buf())),
    }

    scan.records
        .sort_by(|left, right| left.path.cmp(&right.path));
    scan
}

/// The direct child DIRECTORIES of `dir`, or nothing at all when `dir` does not
/// exist.
fn child_dirs(dir: &Path) -> io::Result<Vec<PathBuf>> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: SC-400d root enumeration — see clippy.toml"
    )]
    let listing = fs::read_dir(dir);
    let entries = match listing {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        // A file here is not a session (the lifecycle locks live beside the
        // session dirs). An entry whose TYPE cannot be read is KEPT: inability
        // to verify is not absence, and the cost of being wrong is a spurious
        // row rather than a vanished session.
        if entry.file_type().is_ok_and(|kind| !kind.is_dir()) {
            continue;
        }
        found.push(entry.path());
    }
    Ok(found)
}

/// The durable record for the state directory at `path`.
///
/// The record exists before the `meta` is opened, which is what makes an
/// unreadable one cost facts instead of the candidate.
fn record_at(path: PathBuf, layout: Layout) -> DurableRecord {
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned();
    let mut record = DurableRecord {
        path,
        name,
        layout,
        server: ServerSelector::Missing,
        meta_read: MetaRead::Absent,
        snapshot: RecordSnapshot::default(),
    };
    // ONE read of this record, here, feeding both the selector and every SC-509
    // field the digest will need. The meta had to be opened for the selector
    // anyway; what changes is that nothing downstream opens it again.
    record.snapshot = RecordSnapshot::read(&record.path);
    // BOTH facts from the ONE read. Absent and unreadable are different facts
    // and stay different (SC-405l as amended: both normalize the SELECTOR to
    // `missing`, and only the record-read fact tells them apart) — and telling
    // them apart is the READ's job, because it is the only thing that saw the
    // error. Asking the filesystem again here answered "absent" for a directory
    // the process may not traverse, inventing a fact about bytes that exist.
    record.meta_read = record.snapshot.meta_read;
    if let Some(meta) = &record.snapshot.meta {
        // No selector is derived from bytes nobody could read — the alternative
        // is querying a server on a guess.
        record.server = meta.server_selector();
    }
    record
}

/// The servers ae is entitled to enumerate, most-ambient first.
///
/// SC-017j's finite, pointer-derived set: the ambient server this invocation's
/// transport selected, plus every distinct positive, unambiguous selector a
/// durable candidate recorded. Missing and ambiguous selectors contribute
/// nothing. There is no third source — an arbitrary socket path or server name
/// is not a pointer ae holds.
#[must_use]
pub fn entitled_servers(ambient: Option<&ServerId>, durable: &[DurableRecord]) -> Vec<ServerId> {
    let mut entitled: Vec<ServerId> = Vec::new();
    if let Some(ambient) = ambient {
        entitled.push(ambient.clone());
    }
    for record in durable {
        if let Some(selector) = record.server.entitles() {
            let server = ServerId::Selected(selector.clone());
            if !entitled.contains(&server) {
                entitled.push(server);
            }
        }
    }
    entitled
}

/// What phase 1 established: which sessions EXIST, and nothing about whether
/// they run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inventory {
    /// Every candidate, durable and live-only.
    pub candidates: Vec<Candidate>,
    /// Every logical source whose enumeration failed — SC-017o.
    ///
    /// ONE list for all three classes, deliberately. The row asks whether ALL
    /// SC-017j enumerations completed, and a completeness answer derived from
    /// several lists is one that goes wrong the day a fourth source class is
    /// added to only some of them. [`Inventory::complete`] reads this and
    /// nothing else.
    ///
    /// Nothing renders it yet — the stderr diagnostic and the digest's
    /// `inventory_complete` are SC-017o's phase-2/3 surfaces. It is never a
    /// candidate source: a failed source names no identity, and inventing one is
    /// what the row forbids.
    pub incomplete: Vec<FailedSource>,
}

impl Inventory {
    /// Whether every SC-017j enumeration completed — SC-017o's snapshot fact.
    ///
    /// Derived, never stored: a boolean that can disagree with the loss facts
    /// beside it is one that eventually will. Answerable for an EMPTY inventory,
    /// which is the case the row calls out — "nothing found" and "nothing found,
    /// and I could not look everywhere" are different snapshots, and only one of
    /// them is evidence of absence.
    #[must_use]
    pub const fn complete(&self) -> bool {
        self.incomplete.is_empty()
    }

    /// The entitled servers that did not answer.
    ///
    /// A fact about the QUERY, never about a session: SC-017l turns it into
    /// `unknown` one phase later, and nothing here may read it as a status. Kept
    /// reachable because discarding it would make phase 2 ask the same dead
    /// server again to learn what this pass already knows. A server ae was never
    /// entitled to ask is NOT in here — never asked is not unreachable.
    pub fn unreachable(&self) -> impl Iterator<Item = &ServerId> {
        self.incomplete.iter().filter_map(|source| match source {
            FailedSource::Server(server) => Some(server),
            FailedSource::CanonicalRoot(_)
            | FailedSource::WorktreeRoot(_)
            | FailedSource::WorktreeState(_) => None,
        })
    }
}

/// Take the SC-017j inventory: durable records unioned with what the entitled
/// servers report.
///
/// Every durable record becomes a candidate and stays one — this function starts
/// as all of them and only ever pushes. `discovery` is called for the entitled
/// servers and for no others, so "ae does not gain entitlement by sweeping" is a
/// property of the call sequence rather than of a filter applied afterwards.
///
/// **Discovery completes before reconciliation** (SC-017j). Every sighting is
/// gathered first, so no join can depend on which server answered first.
///
/// A sighting joins a durable candidate only on SC-017j's join witness: that
/// candidate's selector is positive, the sighting came from that very server,
/// its name exactly equals the candidate's inventory name, and **exactly one**
/// durable candidate matches that tuple. Zero matches leave a live-only
/// candidate; more than one and NONE merges — every durable candidate and the
/// sighting all remain. Server plus exact name is a join witness, not identity.
///
/// ```
/// use ae::inventory::{Discovery, DiscoveredSession, Inventory, QueryFailed, ServerId, take};
///
/// struct OneSession;
/// impl Discovery for OneSession {
///     fn enumerate(&self, _: &ServerId) -> Result<Vec<DiscoveredSession>, QueryFailed> {
///         Ok(vec![DiscoveredSession {
///             name: "my-feature".to_owned(),
///             marker: Some("my-feature".to_owned()),
///         }])
///     }
/// }
///
/// let scan = ae::inventory::DurableScan::default();
/// let Inventory { candidates, .. } = take(scan, Some(&ServerId::Ambient), &OneSession);
/// assert_eq!(candidates.len(), 1);
/// assert!(candidates[0].is_tmux_only());
/// ```
pub fn take<D: Discovery + ?Sized>(
    durable: DurableScan,
    ambient: Option<&ServerId>,
    discovery: &D,
) -> Inventory {
    let entitled = entitled_servers(ambient, &durable.records);
    let mut inventory = Inventory {
        candidates: durable
            .records
            .into_iter()
            .map(Candidate::durable)
            .collect(),
        // Carried, not consulted: an incompleteness the caller inherits.
        incomplete: durable.incomplete,
    };

    // Discovery, complete, before any reconciliation.
    let mut sighted: Vec<LiveSighting> = Vec::new();
    for server in entitled {
        let Ok(sessions) = discovery.enumerate(&server) else {
            // SC-017o's third source class. The snapshot is incomplete; every
            // candidate from every other source survives untouched.
            inventory.incomplete.push(FailedSource::Server(server));
            continue;
        };
        for session in sessions {
            // No ownership evidence: not ae's, so not a candidate — and still
            // not a reason to touch a durable candidate that happens to share
            // its name. That substitution is #105.
            let Some(marker) = session.marker else {
                continue;
            };
            sighted.push(LiveSighting {
                server: server.clone(),
                name: session.name,
                marker,
            });
        }
    }

    // Reconciliation, over the complete picture.
    for sighting in sighted {
        match join_witness(&inventory.candidates, &sighting) {
            Some(at) => inventory.candidates[at].live = Some(sighting),
            None => inventory.candidates.push(Candidate::tmux_only(sighting)),
        }
    }
    inventory
}

/// The single durable candidate `sighting` positively joins, if exactly one
/// does.
///
/// SC-017j's witness is the (recorded server, exact inventory name) tuple, and
/// its cardinality rule is explicit: with more than one match, NONE merges.
/// Picking one would invent an identity out of a witness the row says is not
/// identity.
fn join_witness(candidates: &[Candidate], sighting: &LiveSighting) -> Option<usize> {
    let mut found = None;
    for (at, candidate) in candidates.iter().enumerate() {
        let matches = candidate.live.is_none()
            && candidate.durable.as_ref().is_some_and(|record| {
                record.name == sighting.name
                    && record.server.entitles().is_some_and(|selector| {
                        ServerId::Selected(selector.clone()) == sighting.server
                    })
            });
        if matches {
            if found.is_some() {
                return None;
            }
            found = Some(at);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "fixtures build and inspect real directories; the boundary is about \
                  what PRODUCT code may reach"
    )]

    //! Each test names the pre-registered criterion of
    //! `docs/migration/p1-phase1-gate.md` it answers. The gate was authored
    //! against the ROWS, without reading this module, so a criterion it cannot
    //! run is a contract gap and is reported as one rather than reinterpreted
    //! into something passable.

    use super::{
        Candidate, DiscoveredSession, Discovery, DurableRecord, DurableScan, FailedSource,
        Inventory, Layout, LiveSighting, MetaRead, Provenance, QueryFailed, Roots, Selector,
        ServerId, ServerSelector, durable_records, entitled_servers, take,
    };
    use crate::session::RecordSnapshot;
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn named(server: &str) -> ServerId {
        ServerId::Selected(Selector::Name(server.to_owned()))
    }

    fn positive(server: &str) -> ServerSelector {
        ServerSelector::Positive(Selector::Name(server.to_owned()))
    }

    /// A tmux world that RECORDS every server it is asked about.
    ///
    /// Criterion 13: a candidate-absence assertion cannot tell a sweeper from a
    /// non-sweeper, because a sweeper can query and then discard. The trace is
    /// the only observable that can.
    struct Servers {
        worlds: Vec<(ServerId, Result<Vec<DiscoveredSession>, QueryFailed>)>,
        trace: RefCell<Vec<ServerId>>,
    }

    impl Servers {
        fn new() -> Self {
            Self {
                worlds: Vec::new(),
                trace: RefCell::new(Vec::new()),
            }
        }

        /// A server that answers with these `(name, marker)` sessions.
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

        /// A server that exists but does not answer.
        fn down(mut self, server: ServerId) -> Self {
            self.worlds.push((server, Err(QueryFailed)));
            self
        }

        /// The SET of servers contacted — criterion 3 and 10 ask for the set,
        /// and criterion 18 forbids pinning query order.
        fn contacted(&self) -> Vec<String> {
            let mut seen: Vec<String> = self
                .trace
                .borrow()
                .iter()
                .map(|server| match server {
                    ServerId::Ambient => "ambient".to_owned(),
                    ServerId::Selected(Selector::Name(name)) => format!("name:{name}"),
                    ServerId::Selected(Selector::Socket(path)) => {
                        format!("socket:{}", path.display())
                    }
                })
                .collect();
            seen.sort();
            seen
        }
    }

    impl Discovery for Servers {
        fn enumerate(&self, server: &ServerId) -> Result<Vec<DiscoveredSession>, QueryFailed> {
            self.trace.borrow_mut().push(server.clone());
            self.worlds
                .iter()
                .find(|(known, _)| known == server)
                .map_or(Ok(Vec::new()), |(_, answer)| answer.clone())
        }
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("ae-inventory-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("a scratch dir");
            Self(dir)
        }

        /// A canonical candidate: `<home>/sessions/<name>/`.
        fn session(&self, name: &str) -> PathBuf {
            let dir = self.0.join("sessions").join(name);
            fs::create_dir_all(&dir).expect("a session dir");
            dir
        }

        /// A legacy candidate: `<home>/worktrees/<worktree>/.ae/<session>/`.
        fn nested(&self, worktree: &str, session: &str) -> PathBuf {
            let dir = self
                .0
                .join("worktrees")
                .join(worktree)
                .join(".ae")
                .join(session);
            fs::create_dir_all(&dir).expect("a nested state dir");
            dir
        }

        /// A worktree checkout with NO nested state directory.
        fn bare_worktree(&self, worktree: &str) -> PathBuf {
            let dir = self.0.join("worktrees").join(worktree);
            fs::create_dir_all(dir.join("src")).expect("a bare worktree");
            dir
        }

        fn roots(&self) -> Roots {
            Roots::under(&self.0)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn identity(candidate: &Candidate) -> String {
        match (&candidate.durable, &candidate.live) {
            (Some(record), _) => format!("durable:{}", record.path.display()),
            (None, Some(live)) => match &live.server {
                ServerId::Ambient => format!("live:{}@ambient", live.name),
                ServerId::Selected(Selector::Name(name)) => {
                    format!("live:{}@name:{name}", live.name)
                }
                ServerId::Selected(Selector::Socket(path)) => {
                    format!("live:{}@socket:{}", live.name, path.display())
                }
            },
            (None, None) => unreachable!("a candidate is durable, live, or both"),
        }
    }

    /// Candidate identities as a SET.
    ///
    /// Criterion 18: collection order is an open choice, so no test here pins
    /// it. Durable candidates are compared by PATH — criterion 6's "two
    /// independently addressable candidates" is exactly the distinction a
    /// name-keyed comparison would erase.
    fn identities(inventory: &Inventory) -> Vec<String> {
        let mut seen: Vec<String> = inventory.candidates.iter().map(identity).collect();
        seen.sort();
        seen
    }

    fn durable_identities(inventory: &Inventory) -> Vec<String> {
        identities(inventory)
            .into_iter()
            .filter(|id| id.starts_with("durable:"))
            .collect()
    }

    /// Provenance per candidate, in identity order — never iteration order.
    fn provenances(inventory: &Inventory) -> Vec<Provenance> {
        let mut ordered: Vec<(String, Provenance)> = inventory
            .candidates
            .iter()
            .map(|candidate| (identity(candidate), candidate.provenance()))
            .collect();
        ordered.sort_by(|left, right| left.0.cmp(&right.0));
        ordered
            .into_iter()
            .map(|(_, provenance)| provenance)
            .collect()
    }

    fn attached(inventory: &Inventory, path: &Path) -> Option<LiveSighting> {
        inventory
            .candidates
            .iter()
            .find(|candidate| {
                candidate
                    .durable
                    .as_ref()
                    .is_some_and(|record| record.path == path)
            })
            .and_then(|candidate| candidate.live.clone())
    }

    fn found(inventory: &Inventory, path: &Path) -> DurableRecord {
        inventory
            .candidates
            .iter()
            .find_map(|candidate| {
                candidate
                    .durable
                    .as_ref()
                    .filter(|record| record.path == path)
            })
            .cloned()
            .unwrap_or_else(|| panic!("no candidate at {}", path.display()))
    }

    /// This module's own source, comments stripped, TESTS EXCLUDED.
    ///
    /// The structural guards below ask the source a question the runtime cannot
    /// answer — non-access has no signal. Excluding the test half is
    /// load-bearing: every needle they forbid appears in the tests that forbid
    /// it, so a whole-file scan would report the guard itself and pass for the
    /// wrong reason forever after someone "fixed" it. The split is asserted, so
    /// a guard that scanned nothing fails instead of passing.
    fn module_source() -> String {
        let source = include_str!("inventory.rs");
        let code: String = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let (module, tests) = code
            .split_once("#[cfg(test)]")
            .expect("this module has a test module");
        assert!(
            tests.contains("fn module_source"),
            "the split landed where it was meant to"
        );
        assert!(
            module.contains("pub fn take"),
            "the scan reached the real code"
        );
        module.to_owned()
    }

    fn record(path: &str, server: ServerSelector) -> DurableRecord {
        let path = PathBuf::from(path);
        DurableRecord {
            name: path
                .file_name()
                .expect("a last component")
                .to_string_lossy()
                .into_owned(),
            path,
            layout: Layout::Canonical,
            server,
            meta_read: MetaRead::Parsed,
            snapshot: RecordSnapshot::default(),
        }
    }

    fn scan(records: Vec<DurableRecord>) -> DurableScan {
        DurableScan::from(records)
    }

    fn plain(path: &str) -> DurableRecord {
        record(path, ServerSelector::Missing)
    }

    // ---- criterion 1: the phase observation seam ---------------------------

    #[test]
    fn criterion_1_the_candidate_collection_is_observable_before_any_classification() {
        let servers = Servers::new().live(ServerId::Ambient, &[("ghost", Some("ghost"))]);
        let inventory = take(
            scan(vec![plain("/s/kept")]),
            Some(&ServerId::Ambient),
            &servers,
        );
        assert_eq!(
            identities(&inventory),
            ["durable:/s/kept", "live:ghost@ambient"]
        );
        assert_eq!(
            inventory
                .candidates
                .iter()
                .map(Candidate::is_tmux_only)
                .collect::<Vec<_>>(),
            [false, true],
            "and each candidate knows which source established it"
        );
    }

    // ---- criterion 2: both SC-400d durable layouts -------------------------

    #[test]
    fn criterion_2_both_durable_layouts_are_discovered_and_the_inner_leaf_is_the_name() {
        let scratch = Scratch::new("two-layouts");
        let canonical = scratch.session("canonical-one");
        // The outer worktree name DIFFERS from the inner session leaf — SC-400d
        // says the two may differ and the inner one is the inventory name.
        let nested = scratch.nested("feature-checkout", "nested-session");
        let bare = scratch.bare_worktree("no-state-here");

        let records = durable_records(&scratch.roots()).records;
        let by_path: Vec<(&Path, &str, Layout)> = records
            .iter()
            .map(|record| (record.path.as_path(), record.name.as_str(), record.layout))
            .collect();
        assert_eq!(
            by_path,
            [
                (canonical.as_path(), "canonical-one", Layout::Canonical),
                (nested.as_path(), "nested-session", Layout::WorktreeNested),
            ],
            "the outer worktree name is never the inventory name"
        );
        assert!(
            !records.iter().any(|record| record.path == bare),
            "a bare worktree with no nested state directory is not a candidate"
        );
    }

    #[test]
    fn a_worktree_state_root_that_will_not_list_is_recorded_rather_than_silently_skipped() {
        // The third instance of one family tonight: never-asked is not
        // unreachable, record-absent is not record-unreadable, and a state
        // directory ae could not LIST is not one that is not there. A session
        // living in here is invisible, so the INCOMPLETENESS is the fact — the
        // alternative is a confident listing that omits it and says nothing.
        let scratch = Scratch::new("unlistable");
        let kept = scratch.session("elsewhere");
        let good = scratch.nested("readable-checkout", "seen");
        // `.ae` as a FILE: read_dir fails with NotADirectory for every uid,
        // where a chmod would depend on who is running the suite.
        let blocked = scratch.0.join("worktrees").join("opaque-checkout");
        fs::create_dir_all(&blocked).expect("a worktree");
        fs::write(blocked.join(".ae"), "not a directory").expect("an unlistable state root");
        assert!(
            fs::read_dir(blocked.join(".ae")).is_err(),
            "the fixture must genuinely fail to list"
        );

        let scan = durable_records(&scratch.roots());
        assert_eq!(
            scan.incomplete,
            [FailedSource::WorktreeState(blocked.join(".ae"))],
            "the incompleteness is recorded"
        );
        assert_eq!(
            scan.records
                .iter()
                .map(|record| record.path.as_path())
                .collect::<Vec<_>>(),
            [kept.as_path(), good.as_path()],
            "and one unlistable subtree costs no candidate anywhere else"
        );

        let inventory = take(scan, None, &Servers::new());
        assert_eq!(
            inventory.incomplete,
            [FailedSource::WorktreeState(blocked.join(".ae"))],
            "carried through to the boundary"
        );
        assert!(!inventory.complete(), "and the SNAPSHOT says so");
        // `identities` sorts, and `sessions/` sorts before `worktrees/`.
        assert_eq!(
            identities(&inventory),
            [
                format!("durable:{}", kept.display()),
                format!("durable:{}", good.display()),
            ],
            "and it is never a candidate source"
        );
    }

    #[test]
    fn a_bare_worktree_is_absence_rather_than_loss() {
        // SC-400d: a worktree with no nested state directory is simply not a
        // candidate. Recording it as an incompleteness would make the loss list
        // fire on the NORMAL case and stop meaning anything.
        let scratch = Scratch::new("bare-not-loss");
        scratch.bare_worktree("just-a-checkout");
        let scan = durable_records(&scratch.roots());
        assert!(scan.records.is_empty());
        assert!(
            scan.incomplete.is_empty(),
            "nothing was lost — there was nothing there"
        );
    }

    #[test]
    fn criterion_2_a_worktree_root_that_does_not_exist_is_not_a_failure() {
        let scratch = Scratch::new("no-worktrees");
        scratch.session("only-canonical");
        let records = durable_records(&scratch.roots()).records;
        assert_eq!(records.len(), 1);
    }

    // ---- criterion 3: archives are zero input ------------------------------

    #[test]
    fn criterion_3_archives_change_neither_the_candidates_nor_the_set_of_queried_servers() {
        let scratch = Scratch::new("archive");
        scratch.session("live-one");
        let baseline_servers = Servers::new().live(ServerId::Ambient, &[]);
        let baseline = take(
            durable_records(&scratch.roots()),
            Some(&ServerId::Ambient),
            &baseline_servers,
        );

        // (a) an archive-only identity, (b) one whose basename collides with a
        // durable candidate, (c) one whose meta names an unentitled server.
        for name in ["archived-only", "live-one"] {
            let dir = scratch.0.join("archive").join(name);
            fs::create_dir_all(&dir).expect("an archive fixture");
            fs::write(
                dir.join("meta"),
                "mode=local\ntmux_server_kind=socket\ntmux_server=/tmp/unentitled.sock\n",
            )
            .expect("an archived meta");
        }

        let after_servers = Servers::new().live(ServerId::Ambient, &[]);
        let after = take(
            durable_records(&scratch.roots()),
            Some(&ServerId::Ambient),
            &after_servers,
        );

        assert_eq!(identities(&after), identities(&baseline), "same candidates");
        assert_eq!(
            after_servers.contacted(),
            baseline_servers.contacted(),
            "and archive bytes conferred no entitlement"
        );
        assert_eq!(after_servers.contacted(), ["ambient"]);
    }

    // ---- criterion 4: an unreadable meta deletes nothing, in EACH layout ---

    #[test]
    fn criterion_4_an_unreadable_meta_in_either_layout_keeps_its_candidate() {
        let scratch = Scratch::new("unreadable-meta");
        let canonical = scratch.session("damaged-canonical");
        let nested = scratch.nested("checkout", "damaged-nested");
        for dir in [&canonical, &nested] {
            // A DIRECTORY named `meta`. chmod is not enough on its own — a run
            // as root reads a 0000 file happily, and the test would then pass
            // without ever creating the condition it names. EISDIR holds for
            // every uid, so the condition is real wherever this runs, and it is
            // ASSERTED before anything is concluded from it.
            fs::create_dir_all(dir.join("meta")).expect("a meta that cannot be read as a file");
            let failure = fs::read(dir.join("meta")).expect_err("the read must genuinely fail");
            assert!(
                !matches!(failure.kind(), std::io::ErrorKind::NotFound),
                "the fixture must fail on READING, not on absence: {failure:?}"
            );
        }

        let servers = Servers::new().live(ServerId::Ambient, &[]);
        let inventory = take(
            durable_records(&scratch.roots()),
            Some(&ServerId::Ambient),
            &servers,
        );

        for dir in [&canonical, &nested] {
            let record = found(&inventory, dir);
            assert_eq!(
                record.meta_read,
                MetaRead::Unreadable,
                "the read outcome is carried, not inferred later"
            );
            assert_eq!(
                record.server,
                ServerSelector::Missing,
                "no selector is derived from bytes nobody could read"
            );
        }
        assert_eq!(
            servers.contacted(),
            ["ambient"],
            "and no server query was sourced from either unreadable record"
        );
    }

    #[test]
    fn a_meta_behind_a_directory_the_process_cannot_traverse_is_unreadable_not_absent() {
        // THE CELL A SECOND OBSERVATION GETS WRONG. The bytes exist; the
        // process may not reach them. Anything that answers "is there a meta?"
        // by asking the filesystem a second time gets `false` here and reports
        // ABSENT — inventing the loss of a record that was never lost, and
        // collapsing phase-1 criteria 21 and 23's two states into one.
        //
        // Deliberately NOT the EISDIR fixture: that one leaves the path
        // observable, so a second look still says "something is there" and the
        // bug hides. The failure has to be in the TRAVERSAL for the two
        // observations to disagree.
        let scratch = Scratch::new("unsearchable");
        let dir = scratch.session("locked");
        fs::write(dir.join("meta"), "mode=local\n").expect("real bytes on disk");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let denied = fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).is_ok();
            assert!(denied, "the fixture must be able to deny traversal");
        }
        // The precondition, ASSERTED rather than assumed: running as root would
        // traverse anyway and the cell would not exist. Stating it means a
        // privileged runner fails loudly instead of passing vacuously.
        let unreachable = fs::read(dir.join("meta")).is_err();
        let observed_absent = !dir.join("meta").exists();
        assert!(
            unreachable,
            "this proof needs a uid that cannot traverse the directory; \
             running as root defeats the fixture"
        );
        assert!(
            observed_absent,
            "and the second observation must DISAGREE with the read — that \
             disagreement is the whole finding"
        );

        let scan = durable_records(&scratch.roots());
        let record = scan
            .records
            .iter()
            .find(|record| record.path == dir)
            .expect("the candidate survives");
        assert_eq!(
            record.meta_read,
            MetaRead::Unreadable,
            "the read said EACCES; nothing downstream may downgrade that to absence"
        );
        assert_eq!(record.server, ServerSelector::Missing);

        // Restore before the fixture is dropped, or the tree cannot be removed.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o755));
        }
    }

    #[test]
    fn an_absent_meta_is_a_different_read_outcome_from_an_unreadable_one() {
        let scratch = Scratch::new("absent-meta");
        let bare = scratch.session("no-meta");
        let inventory = take(durable_records(&scratch.roots()), None, &Servers::new());
        assert_eq!(found(&inventory, &bare).meta_read, MetaRead::Absent);
    }

    // ---- criterion 5: live-only discovery, with an ownership control -------

    #[test]
    fn criterion_5_only_the_marked_tmux_only_session_enters_the_inventory() {
        let servers = Servers::new().live(
            ServerId::Ambient,
            &[("marked", Some("marked")), ("unmarked", None)],
        );
        let inventory = take(DurableScan::default(), Some(&ServerId::Ambient), &servers);
        assert_eq!(identities(&inventory), ["live:marked@ambient"]);
        assert!(inventory.candidates[0].is_tmux_only());
    }

    // ---- criterion 6: distinct identities under one basename ---------------

    #[test]
    fn criterion_6_one_basename_in_two_layouts_is_two_addressable_candidates() {
        let scratch = Scratch::new("same-leaf");
        let canonical = scratch.session("mdk");
        let nested = scratch.nested("other-checkout", "mdk");
        let inventory = take(durable_records(&scratch.roots()), None, &Servers::new());
        assert_eq!(
            durable_identities(&inventory),
            [
                format!("durable:{}", canonical.display()),
                format!("durable:{}", nested.display()),
            ],
            "equal leaves across paths never deduplicate"
        );
        assert_eq!(found(&inventory, &canonical).name, "mdk");
        assert_eq!(found(&inventory, &nested).name, "mdk");
        assert_ne!(
            found(&inventory, &canonical).layout,
            found(&inventory, &nested).layout,
            "and each knows which root qualified it"
        );
    }

    // ---- criterion 7: a failed server query removes nothing ----------------

    #[test]
    fn criterion_7_a_backend_failure_removes_no_durable_candidate_and_no_other_server() {
        let durable = scan(vec![
            plain("/s/no-pointer"),
            record("/s/points-down", positive("sock-down")),
            record("/s/points-up", positive("sock-up")),
        ]);
        let servers = Servers::new()
            .down(named("sock-down"))
            .live(named("sock-up"), &[("elsewhere", Some("elsewhere"))]);

        let inventory = take(durable, Some(&ServerId::Ambient), &servers);

        assert_eq!(
            durable_identities(&inventory),
            [
                "durable:/s/no-pointer",
                "durable:/s/points-down",
                "durable:/s/points-up",
            ],
            "a failed query is not a reason to lose a durable candidate"
        );
        assert!(
            identities(&inventory).contains(&"live:elsewhere@name:sock-up".to_owned()),
            "the server that DID answer still contributes"
        );
        assert_eq!(
            inventory.unreachable().collect::<Vec<_>>(),
            [&named("sock-down")],
            "the failure is recorded as a fact about the QUERY"
        );
    }

    // ---- criterion 8: the prefix orientation, in the firing direction ------

    #[test]
    fn criterion_8_a_long_live_sibling_never_absorbs_the_short_dead_durable_candidate() {
        // The orientation is the test: durable `mdk` with NO live `mdk`, and a
        // separately live `mdk-app`. Co-occurrence in the other direction is
        // the substitution that proves nothing.
        let servers = Servers::new().live(ServerId::Ambient, &[("mdk-app", Some("mdk-app"))]);
        let inventory = take(
            scan(vec![record("/s/mdk", positive("sock-a"))]),
            Some(&ServerId::Ambient),
            &servers,
        );
        assert_eq!(
            identities(&inventory),
            ["durable:/s/mdk", "live:mdk-app@ambient"]
        );
        assert_eq!(attached(&inventory, Path::new("/s/mdk")), None);
    }

    // ---- criterion 9: an exact live name without the marker ---------------

    #[test]
    fn criterion_9a_an_unmarked_exact_live_name_leaves_the_durable_candidate_alone() {
        let servers = Servers::new().live(ServerId::Ambient, &[("mdk", None)]);
        let inventory = take(
            scan(vec![plain("/s/mdk")]),
            Some(&ServerId::Ambient),
            &servers,
        );
        assert_eq!(identities(&inventory), ["durable:/s/mdk"]);
        assert_eq!(attached(&inventory, Path::new("/s/mdk")), None);
    }

    #[test]
    fn criterion_9b_the_same_unmarked_session_with_no_durable_record_is_no_candidate() {
        let servers = Servers::new().live(ServerId::Ambient, &[("mdk", None)]);
        let inventory = take(DurableScan::default(), Some(&ServerId::Ambient), &servers);
        assert!(
            inventory.candidates.is_empty(),
            "positive ownership is what admits a live-only candidate, and there was none"
        );
    }

    // ---- criteria 10/11/12: entitlement, end to end ------------------------

    #[test]
    fn criterion_10_only_the_ambient_server_and_recorded_pointers_are_contacted() {
        // A = ambient, B = named by a durable candidate's own meta on disk,
        // C = live and reachable to the harness but named by nobody.
        let scratch = Scratch::new("entitlement");
        let pointer = scratch.session("pointer");
        fs::write(
            pointer.join("meta"),
            "tmux_server_kind=name\ntmux_server=B-pointed-at\n",
        )
        .expect("a selector on disk");

        let servers = Servers::new()
            .live(ServerId::Ambient, &[("on-a", Some("on-a"))])
            .live(named("B-pointed-at"), &[("on-b", Some("on-b"))])
            .live(named("C-unnamed"), &[("on-c", Some("on-c"))]);

        let inventory = take(
            durable_records(&scratch.roots()),
            Some(&ServerId::Ambient),
            &servers,
        );

        assert_eq!(
            servers.contacted(),
            ["ambient", "name:B-pointed-at"],
            "C is never contacted — the trace, not the result, is what shows a sweep"
        );
        assert_eq!(
            identities(&inventory),
            [
                format!("durable:{}", pointer.display()),
                "live:on-a@ambient".to_owned(),
                "live:on-b@name:B-pointed-at".to_owned(),
            ]
        );
    }

    #[test]
    fn criterion_11_missing_and_ambiguous_selectors_confer_nothing_and_delete_nothing() {
        // The raw bytes an implementation might be tempted to use as a server
        // are live and would answer if asked. They are not asked.
        let scratch = Scratch::new("no-entitlement");
        let no_selector = scratch.session("no-selector");
        let ambiguous = scratch.session("ambiguous");
        fs::write(
            ambiguous.join("meta"),
            "tmux_server_kind=ambiguous\ntmux_server=tempting\n",
        )
        .expect("an ambiguous selector on disk");

        let servers = Servers::new()
            .live(ServerId::Ambient, &[])
            .live(named("tempting"), &[("tempting-session", Some("x"))]);

        let inventory = take(
            durable_records(&scratch.roots()),
            Some(&ServerId::Ambient),
            &servers,
        );

        assert_eq!(
            durable_identities(&inventory),
            [
                format!("durable:{}", ambiguous.display()),
                format!("durable:{}", no_selector.display()),
            ],
            "neither candidate was lost for having no usable pointer"
        );
        assert_eq!(
            found(&inventory, &ambiguous).server,
            ServerSelector::Ambiguous
        );
        assert_eq!(servers.contacted(), ["ambient"], "no guessed selector");
        assert_eq!(
            identities(&inventory).len(),
            2,
            "and the session on those bytes stayed out"
        );
    }

    #[test]
    fn criterion_12_a_session_outside_the_entitled_set_is_absent_rather_than_classified() {
        let servers = Servers::new()
            .live(ServerId::Ambient, &[])
            .live(named("C-unnamed"), &[("on-c", Some("on-c"))]);
        let inventory = take(DurableScan::default(), Some(&ServerId::Ambient), &servers);
        assert!(
            inventory.candidates.is_empty(),
            "no candidate, and no placeholder standing in for one"
        );
        assert!(
            inventory.unreachable().next().is_none(),
            "C is not unreachable — it was never ae's to ask, which is a different fact"
        );
    }

    // ---- criterion 13: no sweep, proven by the trace and by the source -----

    #[test]
    fn criterion_13_unentitled_servers_are_never_contacted_even_to_be_discarded() {
        let scratch = Scratch::new("no-sweep");
        scratch.session("real");
        // Plausible scan bait, on disk beside the real roots.
        for bait in ["tmux-1000", "sockets", ".ae-sock"] {
            fs::create_dir_all(scratch.0.join(bait)).expect("bait");
            fs::write(scratch.0.join(bait).join("default"), "socket").expect("bait socket");
        }
        let servers = Servers::new()
            .live(ServerId::Ambient, &[("real", Some("real"))])
            .live(
                ServerId::Selected(Selector::Socket(PathBuf::from("/tmp/tmux-1000/default"))),
                &[("swept", Some("swept"))],
            );

        let inventory = take(
            durable_records(&scratch.roots()),
            Some(&ServerId::Ambient),
            &servers,
        );

        assert_eq!(
            servers.contacted(),
            ["ambient"],
            "a sweeper that queries and then discards would show up HERE and nowhere else"
        );
        assert!(
            !identities(&inventory).iter().any(|id| id.contains("swept")),
            "and of course it is not in the result either"
        );
    }

    #[test]
    fn criterion_13_the_only_filesystem_reach_is_the_two_ratified_roots() {
        // The trace above covers the tmux half. The filesystem half is not
        // observable from behavior — non-access has no signal — so it is asked
        // of the SOURCE. Weaker than the compiler probe in the parity self-test
        // (this one reads text), and it is here because the alternative is no
        // check at all on the half of criterion 13 that names socket
        // directories.
        let module = module_source();
        assert_eq!(
            module.matches("fs::").count(),
            1,
            "exactly one filesystem call outside the tests"
        );
        assert!(module.contains("fs::read_dir(dir)"));
        assert_eq!(
            module.matches("child_dirs(").count(),
            4,
            "one definition and three ratified roots: sessions, worktrees, and the nested .ae"
        );
        for sweep_bait in [
            concat!("tmux", "-"),
            concat!("/t", "mp"),
            concat!("soc", "ket("),
            "glob",
        ] {
            assert!(
                !module.contains(sweep_bait),
                "no socket-path knowledge belongs in this module: {sweep_bait}"
            );
        }
    }

    // ---- criterion 14: the durable subset survives every tmux world --------

    #[test]
    fn criterion_14_the_durable_projection_is_identical_across_four_tmux_worlds() {
        let fixture = || {
            scan(vec![
                plain("/s/mdk"),
                plain("/s/quiet"),
                record("/s/pointed", positive("sock-b")),
            ])
        };
        let worlds: Vec<(&str, Servers)> = vec![
            (
                "server failure",
                Servers::new().down(ServerId::Ambient).down(named("sock-b")),
            ),
            (
                "short-dead/long-live prefix sibling",
                Servers::new().live(ServerId::Ambient, &[("mdk-app", Some("mdk-app"))]),
            ),
            (
                "exact live without marker",
                Servers::new().live(ServerId::Ambient, &[("mdk", None), ("quiet", None)]),
            ),
            ("no live server", Servers::new()),
        ];

        let expected = [
            "durable:/s/mdk".to_owned(),
            "durable:/s/pointed".to_owned(),
            "durable:/s/quiet".to_owned(),
        ];
        for (world, servers) in worlds {
            let inventory = take(fixture(), Some(&ServerId::Ambient), &servers);
            assert_eq!(
                durable_identities(&inventory),
                expected,
                "the durable projection moved under: {world}"
            );
        }
    }

    // ---- criterion 15: the discovery call boundary -------------------------

    #[test]
    fn criterion_15_durable_inclusion_is_never_gated_on_a_query_and_ownership_never_filters_it() {
        let servers = Servers::new().down(ServerId::Ambient);
        let inventory = take(
            scan(vec![plain("/s/a"), plain("/s/b")]),
            Some(&ServerId::Ambient),
            &servers,
        );
        assert_eq!(
            durable_identities(&inventory),
            ["durable:/s/a", "durable:/s/b"]
        );
    }

    #[test]
    fn criterion_15_the_port_can_only_enumerate_a_server_never_test_one_name() {
        let module = module_source();
        let trait_body = module
            .split_once("pub trait Discovery {")
            .expect("the port")
            .1
            .split_once('}')
            .expect("its body")
            .0
            .to_owned();
        assert!(trait_body.contains("fn enumerate(&self, server: &ServerId)"));
        assert_eq!(
            trait_body.matches("fn ").count(),
            1,
            "one question, not two"
        );
        for existence_check in [
            concat!("has_", "session"),
            concat!("has-", "session"),
            concat!("session_", "exists"),
        ] {
            assert!(
                !module.contains(existence_check),
                "{existence_check} is an existence question, and this phase asks none"
            );
        }
    }

    // ---- criterion 16: no status is constructed here -----------------------

    #[test]
    fn criterion_16_nothing_in_this_module_names_a_status() {
        let module = module_source();
        for status in [
            concat!("Sta", "tus"),
            concat!("Run", "ning"),
            concat!("Stop", "ped"),
            concat!("Unkn", "own"),
            concat!("Session", "Entry"),
        ] {
            assert!(
                !module.contains(status),
                "phase 1 must not be able to say {status}"
            );
        }
    }

    // ---- criterion 20: exactly-one join, and refused ambiguity -------------

    #[test]
    fn criterion_20a_one_match_coalesces_into_a_single_candidate_carrying_both_provenances() {
        let durable = scan(vec![record("/s/api", positive("sock-a"))]);
        let servers = Servers::new().live(named("sock-a"), &[("api", Some("api"))]);
        let inventory = take(durable, None, &servers);

        assert_eq!(identities(&inventory), ["durable:/s/api"], "exactly one");
        assert_eq!(provenances(&inventory), [Provenance::Both]);
        let candidate = &inventory.candidates[0];
        assert_eq!(
            candidate.durable.as_ref().map(|r| r.path.as_path()),
            Some(Path::new("/s/api")),
            "the durable provenance survived the coalesce"
        );
        assert_eq!(
            candidate.live.as_ref().map(|l| l.name.as_str()),
            Some("api"),
            "and so did the live one"
        );
    }

    #[test]
    fn criterion_20a_control_the_same_name_on_another_entitled_server_stays_distinct() {
        let durable = scan(vec![record("/s/api", positive("sock-a"))]);
        let servers = Servers::new()
            .live(ServerId::Ambient, &[("api", Some("api"))])
            .live(named("sock-a"), &[]);
        let inventory = take(durable, Some(&ServerId::Ambient), &servers);
        assert_eq!(
            identities(&inventory),
            ["durable:/s/api", "live:api@ambient"],
            "same name, different server: the witness did not hold"
        );
        assert_eq!(
            provenances(&inventory),
            [Provenance::Durable, Provenance::Live]
        );
    }

    #[test]
    fn criterion_20b_two_root_distinct_twins_and_a_sighting_leave_all_three_and_join_neither() {
        let scratch = Scratch::new("twins");
        let canonical = scratch.session("twin");
        let nested = scratch.nested("checkout", "twin");
        for dir in [&canonical, &nested] {
            fs::write(
                dir.join("meta"),
                "tmux_server_kind=name\ntmux_server=sock-a\n",
            )
            .expect("the same positive selector in both");
        }
        let servers = Servers::new().live(named("sock-a"), &[("twin", Some("twin"))]);
        let inventory = take(durable_records(&scratch.roots()), None, &servers);

        assert_eq!(
            identities(&inventory),
            [
                format!("durable:{}", canonical.display()),
                format!("durable:{}", nested.display()),
                "live:twin@name:sock-a".to_owned(),
            ],
            "all three remain — with more than one match, NONE merges"
        );
        assert_eq!(attached(&inventory, &canonical), None);
        assert_eq!(attached(&inventory, &nested), None);
        assert_eq!(
            provenances(&inventory),
            [Provenance::Durable, Provenance::Durable, Provenance::Live],
            "ambiguous cardinality refuses to coalesce — it does not pick"
        );
    }

    #[test]
    fn a_missing_or_ambiguous_selector_never_authorizes_a_join() {
        for selector in [ServerSelector::Missing, ServerSelector::Ambiguous] {
            let servers = Servers::new().live(ServerId::Ambient, &[("api", Some("api"))]);
            let inventory = take(
                scan(vec![record("/s/api", selector.clone())]),
                Some(&ServerId::Ambient),
                &servers,
            );
            assert_eq!(
                identities(&inventory),
                ["durable:/s/api", "live:api@ambient"],
                "name equality alone is never a join witness: {selector:?}"
            );
        }
    }

    // ---- criterion 21: source membership vs record-read loss ---------------

    #[test]
    fn criterion_21_source_membership_and_record_read_loss_are_independent_facts() {
        // Four fixtures, and the point is which pairs must AGREE and which must
        // DIFFER. (b), (c) and (d) all normalize the selector to `missing` —
        // that is SC-405l as amended, and criterion 23 requires it. What keeps
        // them apart is the read-loss axis alone. Satisfy this by making their
        // SELECTORS differ and criterion 23 breaks; collapse the read facts and
        // this one breaks. Both at once is the only correct answer.
        let scratch = Scratch::new("four-facts");
        let readable = scratch.session("b-readable-no-selector");
        fs::write(readable.join("meta"), "mode=local\n").expect("a readable meta");
        let absent = scratch.session("c-absent-meta");
        let unreadable = scratch.session("d-unreadable-meta");
        fs::create_dir_all(unreadable.join("meta")).expect("a meta that cannot be read as a file");
        assert!(
            fs::read(unreadable.join("meta")).is_err(),
            "the fixture must genuinely fail to read"
        );

        let servers = Servers::new().live(ServerId::Ambient, &[("a-ghost", Some("a-ghost"))]);
        let inventory = take(
            durable_records(&scratch.roots()),
            Some(&ServerId::Ambient),
            &servers,
        );

        let ghost = inventory
            .candidates
            .iter()
            .find(|candidate| candidate.is_tmux_only())
            .expect("the tmux-only candidate");
        assert_eq!(ghost.provenance(), Provenance::Live);
        assert!(ghost.durable.is_none(), "(a) has no record to have lost");

        // The selector axis: identical for all three durable fixtures.
        for dir in [&readable, &absent, &unreadable] {
            assert_eq!(
                found(&inventory, dir).server,
                ServerSelector::Missing,
                "no selector fact is available, however that came to be"
            );
            assert_eq!(
                Candidate::provenance(
                    inventory
                        .candidates
                        .iter()
                        .find(|c| c.durable.as_ref().is_some_and(|r| r.path == *dir))
                        .expect("a durable candidate")
                ),
                Provenance::Durable
            );
        }

        // The read-loss axis: three different answers, none inferred from the
        // selector or from the absence of live evidence.
        assert_eq!(found(&inventory, &readable).meta_read, MetaRead::Parsed);
        assert_eq!(found(&inventory, &absent).meta_read, MetaRead::Absent);
        assert_eq!(
            found(&inventory, &unreadable).meta_read,
            MetaRead::Unreadable
        );
    }

    // ---- criterion 24: the combined arm, and the opposed controls ---------

    #[test]
    fn criterion_24c_an_entitled_server_that_will_not_enumerate_is_one_loss_beside_a_live_candidate()
     {
        // The third terminal failure, with a healthy source contributing.
        let servers = Servers::new()
            .down(named("sock-down"))
            .live(ServerId::Ambient, &[("healthy", Some("healthy"))]);
        let inventory = take(
            scan(vec![record("/s/points-down", positive("sock-down"))]),
            Some(&ServerId::Ambient),
            &servers,
        );
        assert!(!inventory.complete());
        assert_eq!(
            inventory.incomplete,
            [FailedSource::Server(named("sock-down"))],
            "one logical loss fact for the one failed source"
        );
        assert_eq!(
            identities(&inventory),
            ["durable:/s/points-down", "live:healthy@ambient"],
            "the healthy source still contributes, and nothing is fabricated for the dead one"
        );
    }

    #[test]
    fn criterion_24_the_next_depth_is_covered_when_the_parent_enumeration_succeeds() {
        // The discovered-subtree arm, kept separately from the combined one: the
        // worktrees root enumerated FINE and the failure is one level below it,
        // which is the only way a `.ae` subtree can be discovered-then-lost.
        let scratch = Scratch::new("depth-control");
        fs::write(scratch.0.join("sessions"), "not a directory").expect("a hostile fixture");
        let blocked = scratch.0.join("worktrees").join("opaque-checkout");
        fs::create_dir_all(&blocked).expect("a worktree");
        fs::write(blocked.join(".ae"), "not a directory").expect("an unlistable state root");
        let healthy = scratch.nested("good-checkout", "survivor");
        for enumeration in [scratch.0.join("sessions"), blocked.join(".ae")] {
            assert!(
                fs::read_dir(&enumeration).is_err(),
                "the enumeration operation itself must fail: {}",
                enumeration.display()
            );
        }

        let inventory = take(durable_records(&scratch.roots()), None, &Servers::new());

        assert!(!inventory.complete());
        assert_eq!(
            inventory.incomplete,
            [
                FailedSource::CanonicalRoot(scratch.0.join("sessions")),
                FailedSource::WorktreeState(blocked.join(".ae")),
            ],
            "a failure at each depth, each named for the source that failed"
        );
        assert_eq!(
            identities(&inventory),
            [format!("durable:{}", healthy.display())],
            "and the sibling worktree the parent DID enumerate still contributes"
        );
    }

    #[test]
    fn criterion_24_the_combined_arm_is_both_durable_roots_with_an_ambient_healthy_source() {
        // THE ARM A COUNT PASSES AND A FLAG DOES NOT. Every single-failure arm
        // is satisfied by an implementation reporting a constant 1, or keeping
        // only the first loss. Two failures at once is what tells those apart.
        //
        // FIXTURE VALIDITY, and it is not incidental: with BOTH durable roots
        // unlistable there are no durable candidates, hence no recorded
        // selectors, hence no entitlement except the ambient server. A healthy
        // source planted on any other server would be one THE PRODUCT COULD NOT
        // REACH — the fixture would build, every assertion would pass, and it
        // would prove nothing. The trace is asserted below so that stays true.
        let scratch = Scratch::new("both-roots");
        fs::write(scratch.0.join("sessions"), "not a directory").expect("a hostile fixture");
        fs::write(scratch.0.join("worktrees"), "not a directory").expect("a hostile fixture");
        for enumeration in [scratch.0.join("sessions"), scratch.0.join("worktrees")] {
            assert!(
                fs::read_dir(&enumeration).is_err(),
                "the enumeration operation itself must fail: {}",
                enumeration.display()
            );
        }

        let servers = Servers::new().live(ServerId::Ambient, &[("healthy", Some("healthy"))]);
        let inventory = take(
            durable_records(&scratch.roots()),
            Some(&ServerId::Ambient),
            &servers,
        );

        assert!(!inventory.complete());
        assert_eq!(
            inventory.incomplete.len(),
            2,
            "a count, not a flag: two sources failed and two facts are kept"
        );
        assert_eq!(
            inventory.incomplete,
            [
                FailedSource::CanonicalRoot(scratch.0.join("sessions")),
                FailedSource::WorktreeRoot(scratch.0.join("worktrees")),
            ],
            "and the two are distinguishable — different class AND different path"
        );
        assert!(
            !inventory
                .incomplete
                .iter()
                .any(|source| matches!(source, FailedSource::WorktreeState(_))),
            "nothing invented for subtrees under a root that never enumerated"
        );
        assert_eq!(
            servers.contacted(),
            ["ambient"],
            "the fixture is REACHABLE: ambient is the only entitlement derivable here"
        );
        assert_eq!(
            identities(&inventory),
            ["live:healthy@ambient"],
            "the healthy third source contributes, and no identity is invented for the lost roots"
        );
    }

    #[test]
    fn criterion_24_the_five_opposed_controls_stay_complete_and_add_no_loss() {
        // A loss signal that fires on the normal case has stopped meaning
        // anything. Each of these ANSWERED — the answer was "nothing" — or was
        // never ae's to ask.
        //
        // The two missing-root controls are SEPARATE on purpose: a single
        // fixture with neither root present cannot tell "absent canonical is
        // handled" from "absent worktrees is handled", so an implementation that
        // called one of them incomplete would pass a combined control.
        let no_canonical = Scratch::new("control-missing-canonical");
        no_canonical.bare_worktree("a-checkout");
        assert!(
            take(
                durable_records(&no_canonical.roots()),
                None,
                &Servers::new()
            )
            .complete(),
            "a missing CANONICAL root is an authoritative empty source"
        );

        let no_worktrees = Scratch::new("control-missing-worktrees");
        no_worktrees.session("present");
        assert!(
            !no_worktrees.0.join("worktrees").exists(),
            "the fixture must actually lack the worktrees root"
        );
        let scanned = take(
            durable_records(&no_worktrees.roots()),
            None,
            &Servers::new(),
        );
        assert!(
            scanned.complete(),
            "a missing WORKTREES root is an authoritative empty source too"
        );
        assert_eq!(
            scanned.candidates.len(),
            1,
            "and the other root still answered"
        );

        let bare = Scratch::new("control-bare-ae");
        bare.bare_worktree("just-a-checkout");
        assert!(
            take(durable_records(&bare.roots()), None, &Servers::new()).complete(),
            "an absent worktree .ae subtree is an authoritative empty source"
        );

        let empty = Scratch::new("control-readable-empty");
        fs::create_dir_all(empty.0.join("sessions")).expect("a readable, empty root");
        let read = take(durable_records(&empty.roots()), None, &Servers::new());
        assert!(read.candidates.is_empty());
        assert!(read.complete(), "a readable empty source answered");

        let outside = Servers::new()
            .live(ServerId::Ambient, &[])
            .live(named("never-ours"), &[("theirs", Some("theirs"))]);
        let epistemic = take(DurableScan::default(), Some(&ServerId::Ambient), &outside);
        assert!(
            epistemic.complete(),
            "a server outside the entitled set was never required — not asking is not losing"
        );
        assert!(epistemic.candidates.is_empty());
    }

    // ---- criterion 22: the union crosses the boundary standalone -----------

    #[test]
    fn criterion_22_inventory_is_a_standalone_operation_over_raw_discovery_facts() {
        let module = module_source();
        let signature = module
            .split_once("pub fn take")
            .expect("the operation")
            .1
            .split_once('{')
            .expect("its signature")
            .0
            .to_owned();
        assert!(signature.contains("durable: DurableScan"));
        assert!(signature.contains("ambient: Option<&ServerId>"));
        assert!(signature.contains("discovery: &D"));
        assert!(
            signature.contains("-> Inventory"),
            "the union is the return value, not a side effect on someone else's state"
        );

        let servers = Servers::new().live(ServerId::Ambient, &[("live-one", Some("live-one"))]);
        let inventory = take(
            scan(vec![plain("/s/durable-one")]),
            Some(&ServerId::Ambient),
            &servers,
        );
        assert_eq!(
            identities(&inventory),
            ["durable:/s/durable-one", "live:live-one@ambient"]
        );
    }

    #[test]
    fn criterion_22_raw_server_discovery_may_have_run_before_inventory_did() {
        // The explicit NON-requirement: nothing here imposes an order between
        // durable and server discovery. A `Discovery` that answers from facts
        // gathered earlier is as legal as one that shells out on the spot, and
        // must produce the same inventory.
        let gathered_earlier = Servers::new().live(ServerId::Ambient, &[("early", Some("early"))]);
        let _ = gathered_earlier.enumerate(&ServerId::Ambient);
        assert_eq!(
            gathered_earlier.contacted(),
            ["ambient"],
            "the facts were gathered up front"
        );

        let live_now = Servers::new().live(ServerId::Ambient, &[("early", Some("early"))]);
        assert_eq!(
            identities(&take(
                scan(vec![plain("/s/d")]),
                Some(&ServerId::Ambient),
                &gathered_earlier
            )),
            identities(&take(
                scan(vec![plain("/s/d")]),
                Some(&ServerId::Ambient),
                &live_now
            )),
            "when the facts were collected is not inventory's business"
        );
    }

    // ---- criterion 23: the entitlement half of SC-405l ---------------------

    #[test]
    fn criterion_23_an_absent_or_unreadable_meta_normalizes_to_missing_not_to_a_fifth_state() {
        // The trap this pairs with criterion 21: `missing` is what the reader
        // says when no selector fact is available to it, so a record it could
        // not read at all is `missing` — never `ambiguous`, which is reserved
        // for bytes that WERE readable and admit no single positive mapping,
        // and never something outside the four-value domain.
        let scratch = Scratch::new("loss-is-missing");
        let absent = scratch.session("absent-meta");
        let unreadable = scratch.session("unreadable-meta");
        fs::create_dir_all(unreadable.join("meta")).expect("an unreadable meta");
        assert!(
            fs::read(unreadable.join("meta")).is_err(),
            "genuinely unreadable"
        );

        let servers = Servers::new();
        let inventory = take(durable_records(&scratch.roots()), None, &servers);
        for dir in [&absent, &unreadable] {
            assert_eq!(found(&inventory, dir).server, ServerSelector::Missing);
        }
        assert!(
            servers.contacted().is_empty(),
            "and a missing selector never reaches the queried-server set"
        );
        assert_eq!(durable_identities(&inventory).len(), 2, "both still here");
    }

    #[test]
    fn criterion_23_only_a_positive_selector_reaches_the_queried_server_set() {
        // The discriminator matrix itself is a meta-reading unit test, beside
        // the normalizer it tests. What belongs HERE is the consequence: no
        // missing or ambiguous form may enter the queried-server set.
        let durable = scan(vec![
            record("/s/name", positive("by-name")),
            record(
                "/s/socket",
                ServerSelector::Positive(Selector::Socket(PathBuf::from("/tmp/ae.sock"))),
            ),
            record("/s/missing", ServerSelector::Missing),
            record("/s/ambiguous", ServerSelector::Ambiguous),
        ]);
        let servers = Servers::new();
        let inventory = take(durable, None, &servers);

        assert_eq!(
            servers.contacted(),
            ["name:by-name", "socket:/tmp/ae.sock"],
            "both positive TYPES are queried, and nothing else is"
        );
        assert_eq!(
            durable_identities(&inventory).len(),
            4,
            "and no candidate disappeared for having an unusable selector"
        );
    }

    #[test]
    fn criterion_23_a_name_and_a_socket_of_the_same_spelling_are_two_servers() {
        // SC-017j: unproved equivalence between selector spellings never
        // authorizes a merge. These two are not proven equivalent, so they are
        // two entitlements and two queries.
        let durable = scan(vec![
            record("/s/one", positive("/tmp/ae.sock")),
            record(
                "/s/two",
                ServerSelector::Positive(Selector::Socket(PathBuf::from("/tmp/ae.sock"))),
            ),
        ]);
        let servers = Servers::new();
        let _ = take(durable, None, &servers);
        assert_eq!(
            servers.contacted(),
            ["name:/tmp/ae.sock", "socket:/tmp/ae.sock"],
            "the type is part of the identity"
        );
    }

    // ---- the durable reader, and the invariant it holds by shape -----------

    #[test]
    fn sc_404_the_two_state_roots_are_derived_and_the_archive_is_not_reachable() {
        let roots = Roots::under("/home/x/.ae");
        assert_eq!(roots.sessions(), Path::new("/home/x/.ae/sessions"));
        assert_eq!(roots.worktrees(), Path::new("/home/x/.ae/worktrees"));
    }

    #[test]
    fn a_file_under_a_state_root_is_not_a_session() {
        let scratch = Scratch::new("lock-file");
        scratch.session("real");
        fs::write(
            scratch.0.join("sessions").join(".lifecycle.real.lock"),
            "held",
        )
        .expect("a lock fixture");
        let records = durable_records(&scratch.roots()).records;
        assert_eq!(
            records.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["real"]
        );
    }

    #[test]
    fn sc_017o_a_canonical_root_that_will_not_enumerate_still_leaves_the_worktrees_scanned() {
        // The `?` this replaced aborted the WHOLE scan, so an unlistable
        // `<AE_HOME>/sessions` also cost every candidate in every worktree —
        // the same defect the nested arm was already written to avoid, in the
        // one place the early-return had been there all along.
        let scratch = Scratch::new("root-is-a-file");
        fs::write(scratch.0.join("sessions"), "not a directory").expect("a hostile fixture");
        let nested = scratch.nested("checkout", "survivor");
        // The fixture must prove the ENUMERATION returned an error, not merely
        // that something looks odd. A file where a directory belongs fails
        // read_dir with NotADirectory for every uid; a chmod would depend on who
        // is running the suite, and would pass vacuously as root.
        assert!(
            fs::read_dir(scratch.0.join("sessions")).is_err(),
            "the enumeration operation itself must fail"
        );

        let scan = durable_records(&scratch.roots());
        assert_eq!(
            scan.incomplete,
            [FailedSource::CanonicalRoot(scratch.0.join("sessions"))],
            "\"I could not look\" is recorded, never rendered as \"nothing is there\""
        );
        assert_eq!(
            scan.records
                .iter()
                .map(|record| record.path.as_path())
                .collect::<Vec<_>>(),
            [nested.as_path()],
            "and discovery continued: the other source's candidates survive"
        );
        assert!(!take(scan, None, &Servers::new()).complete());
    }

    #[test]
    fn criterion_24d_a_failed_worktrees_root_is_one_loss_and_invents_no_child_losses() {
        let scratch = Scratch::new("worktrees-is-a-file");
        let canonical = scratch.session("survivor");
        fs::write(scratch.0.join("worktrees"), "not a directory").expect("a hostile fixture");
        assert!(
            fs::read_dir(scratch.0.join("worktrees")).is_err(),
            "the enumeration operation itself must fail"
        );

        let inventory = take(durable_records(&scratch.roots()), None, &Servers::new());

        assert!(!inventory.complete());
        assert_eq!(
            inventory.incomplete,
            [FailedSource::WorktreeRoot(scratch.0.join("worktrees"))],
            "exactly one loss, shaped as the root that failed"
        );
        // THE POINT OF THIS ARM: you cannot count what you could not see. The
        // `.ae` subtrees under an unlistable worktrees root were never
        // discovered, so inventing a loss fact per undiscovered subtree would be
        // reporting a number nobody established — the same fabrication SC-017o
        // forbids for identities, one level up.
        assert!(
            !inventory
                .incomplete
                .iter()
                .any(|source| matches!(source, FailedSource::WorktreeState(_))),
            "no guessed child-subtree losses for subtrees that were never discovered"
        );
        assert_eq!(
            identities(&inventory),
            [format!("durable:{}", canonical.display())],
            "and the healthy canonical candidate survives"
        );
    }

    #[test]
    fn sc_017o_the_snapshot_records_all_three_source_classes_at_once() {
        // One snapshot, one completeness answer, three different ways to have
        // lost a source. `complete()` reads the one list, so a class that stops
        // being recorded cannot leave the boolean saying everything was fine.
        let scratch = Scratch::new("all-three");
        fs::write(scratch.0.join("sessions"), "not a directory").expect("a hostile fixture");
        let blocked = scratch.0.join("worktrees").join("opaque");
        fs::create_dir_all(&blocked).expect("a worktree");
        fs::write(blocked.join(".ae"), "not a directory").expect("an unlistable state root");

        let scan = durable_records(&scratch.roots());
        let servers = Servers::new().down(ServerId::Ambient);
        let inventory = take(scan, Some(&ServerId::Ambient), &servers);

        assert_eq!(
            inventory.incomplete,
            [
                FailedSource::CanonicalRoot(scratch.0.join("sessions")),
                FailedSource::WorktreeState(blocked.join(".ae")),
                FailedSource::Server(ServerId::Ambient),
            ],
            "every failed logical source keeps its own loss fact"
        );
        assert!(!inventory.complete());
        assert!(
            inventory.candidates.is_empty(),
            "and NOTHING is fabricated for the identities those sources might hold"
        );
    }

    #[test]
    fn sc_017o_an_empty_but_complete_snapshot_is_distinguishable_from_an_empty_broken_one() {
        // The row's own point: "nothing found" and "nothing found, and I could
        // not look everywhere" are different snapshots, and only one of them is
        // evidence of absence.
        let scratch = Scratch::new("empty-complete");
        let whole = take(durable_records(&scratch.roots()), None, &Servers::new());
        assert!(whole.candidates.is_empty());
        assert!(whole.complete(), "a fresh machine looked everywhere");

        let broken = Scratch::new("empty-broken");
        fs::write(broken.0.join("sessions"), "not a directory").expect("a hostile fixture");
        let partial = take(durable_records(&broken.roots()), None, &Servers::new());
        assert!(partial.candidates.is_empty());
        assert!(!partial.complete(), "same emptiness, different evidence");
    }

    #[test]
    fn sc_017o_a_discovered_candidate_with_an_unreadable_meta_is_not_enumeration_loss() {
        // The row draws this line explicitly: once the directory was
        // discovered, its meta is SC-405i/SC-509b's record-loss fact and never
        // becomes snapshot incompleteness. The enumeration succeeded — it found
        // exactly the thing whose contents are damaged.
        let scratch = Scratch::new("meta-not-enumeration");
        let damaged = scratch.session("damaged");
        fs::create_dir_all(damaged.join("meta")).expect("an unreadable meta");
        assert!(
            fs::read(damaged.join("meta")).is_err(),
            "genuinely unreadable"
        );

        let absent = scratch.session("no-meta-at-all");

        let inventory = take(durable_records(&scratch.roots()), None, &Servers::new());
        assert!(inventory.complete(), "nothing about the ENUMERATION failed");
        assert_eq!(found(&inventory, &damaged).meta_read, MetaRead::Unreadable);
        assert_eq!(found(&inventory, &absent).meta_read, MetaRead::Absent);
    }

    #[test]
    fn a_missing_state_root_is_an_empty_inventory_not_an_error() {
        let scratch = Scratch::new("no-root");
        assert_eq!(durable_records(&scratch.roots()), DurableScan::default());
    }

    #[test]
    fn a_name_ae_cannot_spell_is_still_a_candidate() {
        let scratch = Scratch::new("odd-name");
        scratch.session("plain");
        // APFS enforces UTF-8 filenames and refuses this one with EILSEQ, so on
        // macOS the arm below cannot be built at all. The skip is STATED rather
        // than silent: the assertion runs on the Linux leg, where ext4 takes any
        // byte sequence, and a test that quietly proves nothing on half the
        // matrix is worse than one that says which half.
        #[cfg(unix)]
        let unspellable = {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;
            let raw = OsStr::from_bytes(b"broken-\xff-name");
            fs::create_dir(scratch.0.join("sessions").join(raw)).is_ok()
        };
        #[cfg(not(unix))]
        let unspellable = false;

        let records = durable_records(&scratch.roots()).records;
        assert_eq!(records.len(), usize::from(unspellable) + 1);
        if unspellable {
            assert!(
                records
                    .iter()
                    .any(|record| record.name.contains('\u{FFFD}')),
                "the name is unspellable, so it is lossy — and still inventory"
            );
        }
    }

    #[test]
    fn the_durable_reader_guarantees_sorted_output_rather_than_traversal_order() {
        // The property is DETERMINISM, and the reader guarantees it by sorting.
        // Asserting a remembered order would be the trap criterion 18 warns
        // about — it fails a correct change. Asserting sortedness cannot.
        //
        // No case-only pair in the fixture: APFS folds `Alpha` into `alpha` and
        // the two would be ONE directory, which is a filesystem fact rather than
        // anything this reader decides.
        let scratch = Scratch::new("order");
        for name in ["zulu", "alpha", "mike", "alpha-2"] {
            scratch.session(name);
        }
        scratch.nested("checkout", "nested-one");
        let records = durable_records(&scratch.roots()).records;
        let paths: Vec<&Path> = records.iter().map(|record| record.path.as_path()).collect();
        let mut sorted = paths.clone();
        sorted.sort_unstable();
        assert_eq!(paths, sorted, "read_dir order must not reach the answer");

        let mut names: Vec<&str> = records.iter().map(|record| record.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            ["alpha", "alpha-2", "mike", "nested-one", "zulu"],
            "both layouts, one sorted answer"
        );
    }

    #[test]
    fn sc_017j_the_entitled_set_is_the_ambient_server_plus_distinct_recorded_pointers() {
        let durable = scan(vec![
            record("/s/one", positive("sock-a")),
            plain("/s/two"),
            record("/s/three", ServerSelector::Ambiguous),
            record("/s/four", positive("sock-a")),
        ]);
        assert_eq!(
            entitled_servers(Some(&ServerId::Ambient), &durable.records),
            [ServerId::Ambient, named("sock-a")],
            "distinct pointers only: a repeat is not a second entitlement"
        );
    }
}
