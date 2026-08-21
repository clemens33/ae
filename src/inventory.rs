//! Which sessions EXIST, before anything asks whether they are running.
//!
//! **SC-017j** — the inventory is the union of (a) durable session state under
//! the canonical sessions root and (b) positively identified ae-owned live tmux
//! sessions on a server ae is already entitled to query. Archives are inert and
//! never enter it. Every durable candidate survives into classification: a
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
//! all, and there is no code path from this module to [`crate::digest::Status`].
//!
//! Two structural consequences, both deliberate:
//!
//! * **This module opens no file inside a candidate directory.** A candidate is
//!   a directory entry, so an unreadable, absent or malformed `meta` cannot
//!   remove one — the damage is SC-405i/SC-509b's `degraded` fact, decided a
//!   layer up, on a candidate that already exists. When a row finally names the
//!   durable server selector (see [`RecordedServer`]), reading it must preserve
//!   this: a failed read yields [`RecordedServer::Missing`], never a drop.
//! * **Identity is the PATH, never the last component.** SC-017j does not
//!   authorize basename-only deduplication of distinct identities: two paths
//!   whose last component matches are two candidates.
//!
//! # Entitlement — a finite, pointer-derived set
//!
//! ae may enumerate a tmux server only when it already holds a pointer to it:
//! the ambient server this invocation's ordinary transport selected, or a
//! positive, unambiguous selector recorded by a durable candidate. A missing or
//! ambiguous selector confers no entitlement. Sweeping arbitrary socket paths or
//! server names is not a way to gain one.
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
}

impl Roots {
    /// The roots under `ae_home` — SC-404's default derivation.
    #[must_use]
    pub fn under<P: Into<PathBuf>>(ae_home: P) -> Self {
        Self {
            sessions: ae_home.into().join("sessions"),
        }
    }

    /// The sessions root, `<AE_HOME>/sessions`.
    #[must_use]
    pub fn sessions(&self) -> &Path {
        &self.sessions
    }
}

/// A tmux server ae already holds a pointer to.
///
/// Opaque on purpose. What a selector LOOKS like — socket path, server name,
/// the `tmux_server` / `tmux_server_kind` pair the bash era wrote — is not
/// ratified by any row this module can cite, and bash is evidence rather than an
/// oracle. So this type neither parses nor constructs a selector: it carries the
/// one the caller's transport already resolved, and compares two of them for
/// exact equality, which is all SC-017j's "every DISTINCT positive, unambiguous
/// server selector" needs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServerId(String);

impl ServerId {
    /// The server this selector names.
    pub fn new<S: Into<String>>(selector: S) -> Self {
        Self(selector.into())
    }

    /// The selector, as the caller supplied it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a durable record says about the server its session lives on.
///
/// SC-017j splits the world exactly three ways, because only one of them
/// confers entitlement: a *positive, unambiguous* selector. The other two are
/// not failures to be retried — they are the ratified reason a durable candidate
/// stays in inventory with its liveness unresolved.
///
/// **Nothing produces [`RecordedServer::Positive`] yet, and that is a contract
/// gap rather than an omission here.** SC-405b enumerates the session-context
/// meta keys as `mode`, `origin`, `work_dir`, `goal`; SC-405c the roster keys;
/// SC-405d rules that every other key is tolerated and *never interpreted*. No
/// row names a durable server-selector key, so there is nothing this reader may
/// legally consume — reading the bash era's `tmux_server` would promote evidence
/// to contract. [`durable_records`] therefore records `Missing` for every
/// candidate until that row exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedServer {
    /// A positive, unambiguous selector: this candidate points at a server, and
    /// that pointer entitles ae to query it.
    Positive(ServerId),
    /// No selector was recorded. Confers no entitlement.
    Missing,
    /// A selector was recorded but does not identify one server. Confers no
    /// entitlement — an ambiguous pointer is not a pointer.
    Ambiguous,
}

impl RecordedServer {
    /// The server this record entitles ae to query, if any.
    #[must_use]
    pub const fn entitles(&self) -> Option<&ServerId> {
        match self {
            Self::Positive(server) => Some(server),
            Self::Missing | Self::Ambiguous => None,
        }
    }
}

/// Durable session state found under a sessions root.
///
/// The `path` is the identity; `name` is its last component, kept for display
/// and for the exact-name matching SC-017k will do a phase later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableRecord {
    /// The session directory. THE identity of this candidate.
    pub path: PathBuf,
    /// The directory's last component.
    ///
    /// Lossy for a non-UTF-8 name — the bytes survive in `path`, which is what
    /// any later read must use. A name ae cannot spell is still a candidate:
    /// dropping it would be exactly the disappearance this module forbids.
    pub name: String,
    /// The server this record points at, per SC-017j's three-way split.
    pub server: RecordedServer,
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

/// One inventory candidate: durable state, a live sighting, or both.
///
/// Deliberately carries no status. Liveness is SC-017k/SC-017l's question, one
/// phase later, and a `Status` field here would be a place to answer it early.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The session name this candidate is known by.
    pub name: String,
    /// The durable record, when there is one.
    pub durable: Option<DurableRecord>,
    /// The live sighting, when one was positively attached to this candidate.
    pub live: Option<LiveSighting>,
}

impl Candidate {
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

/// Every durable candidate under `roots`, path order.
///
/// One candidate per direct child DIRECTORY of the sessions root. Nothing
/// inside a candidate is opened, so no `meta` damage can remove one.
///
/// The order is by path, not by traversal: `read_dir` order is a filesystem
/// fact that differs between platforms and between runs. This is internal
/// determinism only — the ORDER a listing shows is SC-017n's, applied later.
///
/// # Errors
///
/// The sessions root not existing is not an error: a machine that never ran ae
/// has no sessions, which is an empty inventory rather than a failure. A root
/// that EXISTS and will not read is returned as the [`io::Error`] it is —
/// inventing an empty answer there would report "no sessions" for "I could not
/// look", which is the shape of #105 one level up.
pub fn durable_records(roots: &Roots) -> io::Result<Vec<DurableRecord>> {
    let entries = match fs::read_dir(roots.sessions()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut records = Vec::new();
    for entry in entries {
        let entry = entry?;
        // A file under the sessions root is not a session (the lifecycle locks
        // live there). An entry whose TYPE cannot be read is kept: inability to
        // verify is not absence, and the cost of being wrong here is a spurious
        // row rather than a vanished session.
        if entry.file_type().is_ok_and(|kind| !kind.is_dir()) {
            continue;
        }
        let path = entry.path();
        records.push(DurableRecord {
            name: entry.file_name().to_string_lossy().into_owned(),
            path,
            // See RecordedServer: no ratified row names a durable selector key.
            server: RecordedServer::Missing,
        });
    }
    records.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(records)
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
    let mut add = |server: &ServerId| {
        if !entitled.iter().any(|known| known == server) {
            entitled.push(server.clone());
        }
    };
    if let Some(ambient) = ambient {
        add(ambient);
    }
    for record in durable {
        if let Some(server) = record.server.entitles() {
            add(server);
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
    /// Entitled servers that did not answer.
    ///
    /// A fact about the QUERY, never about a session: SC-017l turns it into
    /// `unknown` one phase later, and nothing here may read it as a status. It
    /// is kept because discarding it would make phase 2 ask the same dead server
    /// again to learn what this pass already knows.
    pub unreachable: Vec<ServerId>,
}

/// Take the SC-017j inventory: durable records unioned with what the entitled
/// servers report.
///
/// Every durable record becomes a candidate and stays one — this function starts
/// as all of them and only ever pushes. `discovery` is called for the entitled
/// servers and for no others, so "ae does not gain entitlement by sweeping" is a
/// property of the call sequence rather than of a filter applied afterwards.
///
/// A discovered session joins the inventory only when it carries an ownership
/// marker. It ATTACHES to a durable candidate only on positive evidence that
/// they are the same session: that candidate records this exact server, and the
/// names match exactly. Anything weaker attaches nothing and leaves a tmux-only
/// candidate beside the durable one — a visible duplicate is recoverable, a
/// wrong merge is a fabricated identity.
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
/// let ambient = ServerId::new("default");
/// let Inventory { candidates, .. } = take(Vec::new(), Some(&ambient), &OneSession);
/// assert_eq!(candidates.len(), 1);
/// assert!(candidates[0].is_tmux_only());
/// ```
pub fn take<D: Discovery + ?Sized>(
    durable: Vec<DurableRecord>,
    ambient: Option<&ServerId>,
    discovery: &D,
) -> Inventory {
    let entitled = entitled_servers(ambient, &durable);
    let mut inventory = Inventory {
        candidates: durable.into_iter().map(Candidate::durable).collect(),
        unreachable: Vec::new(),
    };
    for server in entitled {
        let Ok(sessions) = discovery.enumerate(&server) else {
            // The query failed. That takes nothing away: every durable candidate
            // is already in `candidates`, and this loop cannot remove one.
            inventory.unreachable.push(server);
            continue;
        };
        for session in sessions {
            let Some(marker) = session.marker else {
                // No ownership evidence: not ae's, so not a candidate — and
                // still not a reason to touch a durable candidate that happens
                // to share its name. That substitution is #105.
                continue;
            };
            let sighting = LiveSighting {
                server: server.clone(),
                name: session.name,
                marker,
            };
            match attachment(&inventory.candidates, &sighting) {
                Some(at) => inventory.candidates[at].live = Some(sighting),
                None => inventory.candidates.push(Candidate::tmux_only(sighting)),
            }
        }
    }
    inventory
}

/// The single durable candidate `sighting` positively belongs to, if exactly one
/// does.
///
/// Exact name AND recorded server, and UNIQUE: two candidates recording the same
/// server under the same name are two identities claiming one tmux session, and
/// picking either would invent the answer. Ambiguity attaches nothing, exactly
/// as an ambiguous selector entitles nothing.
fn attachment(candidates: &[Candidate], sighting: &LiveSighting) -> Option<usize> {
    let mut found = None;
    for (at, candidate) in candidates.iter().enumerate() {
        let matches = candidate.live.is_none()
            && candidate.durable.as_ref().is_some_and(|record| {
                record.name == sighting.name && record.server.entitles() == Some(&sighting.server)
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
    //! Each test names the pre-registered criterion of
    //! `docs/migration/p1-phase1-gate.md` it answers. The gate was authored
    //! against the ROW, without reading this module, so a criterion it cannot
    //! run is a contract gap and is reported as one rather than reinterpreted
    //! into something passable.

    use super::{
        Candidate, DiscoveredSession, Discovery, DurableRecord, Inventory, LiveSighting,
        QueryFailed, RecordedServer, Roots, ServerId, durable_records, entitled_servers, take,
    };
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};

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

        /// A server that answers with these sessions.
        fn live(mut self, server: &str, sessions: &[(&str, Option<&str>)]) -> Self {
            self.worlds.push((
                ServerId::new(server),
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
        fn down(mut self, server: &str) -> Self {
            self.worlds.push((ServerId::new(server), Err(QueryFailed)));
            self
        }

        fn contacted(&self) -> Vec<String> {
            self.trace
                .borrow()
                .iter()
                .map(|server| server.as_str().to_owned())
                .collect()
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

        fn session(&self, name: &str) -> PathBuf {
            let dir = self.0.join("sessions").join(name);
            fs::create_dir_all(&dir).expect("a session dir");
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

    /// Candidate identities as a SET.
    ///
    /// Criterion 18: collection order is an open choice, so no test here may
    /// pin it. Durable candidates are compared by PATH — criterion 6's "two
    /// independently addressable candidates" is exactly the distinction a
    /// name-keyed comparison would erase.
    fn identities(inventory: &Inventory) -> Vec<String> {
        let mut seen: Vec<String> = inventory
            .candidates
            .iter()
            .map(|candidate| match &candidate.durable {
                Some(record) => format!("durable:{}", record.path.display()),
                None => match &candidate.live {
                    Some(live) => format!("live:{}@{}", live.name, live.server.as_str()),
                    None => unreachable!("a candidate is durable, live, or both"),
                },
            })
            .collect();
        seen.sort();
        seen
    }

    fn durable_identities(inventory: &Inventory) -> Vec<String> {
        let mut kept: Vec<String> = identities(inventory)
            .into_iter()
            .filter(|id| id.starts_with("durable:"))
            .collect();
        kept.sort();
        kept
    }

    fn attached(inventory: &Inventory, path: &str) -> Option<LiveSighting> {
        inventory
            .candidates
            .iter()
            .find(|candidate| {
                candidate
                    .durable
                    .as_ref()
                    .is_some_and(|record| record.path == Path::new(path))
            })
            .and_then(|candidate| candidate.live.clone())
    }

    /// This module's own source, comments stripped, TESTS EXCLUDED.
    ///
    /// The three structural guards below ask the source a question the runtime
    /// cannot answer — non-access has no signal. Excluding the test half is
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

    fn record(path: &str, server: RecordedServer) -> DurableRecord {
        let path = PathBuf::from(path);
        DurableRecord {
            name: path
                .file_name()
                .expect("a last component")
                .to_string_lossy()
                .into_owned(),
            path,
            server,
        }
    }

    fn plain(path: &str) -> DurableRecord {
        record(path, RecordedServer::Missing)
    }

    // ---- criterion 1: the phase observation seam ---------------------------

    #[test]
    fn criterion_1_the_candidate_collection_is_observable_before_any_classification() {
        // The seam is `Inventory` itself: a candidate carries no status, so
        // nothing downstream had to run for these assertions to mean something.
        let servers = Servers::new().live("ambient", &[("ghost", Some("ghost"))]);
        let inventory = take(
            vec![plain("/s/kept")],
            Some(&ServerId::new("ambient")),
            &servers,
        );
        assert_eq!(
            identities(&inventory),
            ["durable:/s/kept", "live:ghost@ambient"]
        );
        assert!(
            inventory
                .candidates
                .iter()
                .all(|c| c.durable.is_some() || c.live.is_some()),
            "every candidate is one of the two sources, never a fabricated third"
        );
        let tmux_only: Vec<bool> = inventory
            .candidates
            .iter()
            .map(Candidate::is_tmux_only)
            .collect();
        assert_eq!(
            tmux_only,
            [false, true],
            "and each one knows which source it came from"
        );
    }

    // ---- criterion 3: archives are zero input ------------------------------

    #[test]
    fn criterion_3_archives_change_neither_the_candidates_nor_the_query_trace() {
        let scratch = Scratch::new("archive");
        scratch.session("live-one");
        let baseline_records = durable_records(&scratch.roots()).expect("a readable root");
        let baseline_servers = Servers::new().live("ambient", &[]);
        let baseline = take(
            baseline_records,
            Some(&ServerId::new("ambient")),
            &baseline_servers,
        );

        // (a) an archive-only identity, (b) one whose basename collides with a
        // durable candidate, (c) one whose meta names an unentitled server.
        for name in ["archived-only", "live-one"] {
            let dir = scratch.0.join("archive").join(name);
            fs::create_dir_all(&dir).expect("an archive fixture");
            fs::write(
                dir.join("meta"),
                "mode=local\ntmux_server=/tmp/unentitled.sock\n",
            )
            .expect("an archived meta");
        }

        let after_servers = Servers::new().live("ambient", &[]);
        let after = take(
            durable_records(&scratch.roots()).expect("a readable root"),
            Some(&ServerId::new("ambient")),
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

    // ---- criterion 4: an unreadable meta deletes nothing -------------------

    #[test]
    fn criterion_4_a_durable_directory_whose_meta_cannot_be_read_is_still_a_candidate() {
        let scratch = Scratch::new("unreadable-meta");
        let dir = scratch.session("damaged");
        // A DIRECTORY named `meta`. chmod is not enough on its own — a run as
        // root reads a 0000 file happily, and the test would then pass without
        // ever creating the condition it names. EISDIR holds for every uid, so
        // the condition is real wherever this runs, and it is ASSERTED before
        // anything is concluded from it.
        fs::create_dir_all(dir.join("meta")).expect("a meta that cannot be read as a file");
        let failure = fs::read(dir.join("meta")).expect_err("the meta read must genuinely fail");
        assert!(
            !matches!(failure.kind(), std::io::ErrorKind::NotFound),
            "the fixture must fail on READING, not on absence: {failure:?}"
        );

        let servers = Servers::new().live("ambient", &[]);
        let inventory = take(
            durable_records(&scratch.roots()).expect("a readable root"),
            Some(&ServerId::new("ambient")),
            &servers,
        );
        assert_eq!(
            durable_identities(&inventory),
            [format!("durable:{}", dir.display())]
        );
        assert_eq!(
            servers.contacted(),
            ["ambient"],
            "and no entitlement was derived from bytes nobody could read"
        );
    }

    // ---- criterion 5: live-only discovery, with an ownership control -------

    #[test]
    fn criterion_5_only_the_marked_tmux_only_session_enters_the_inventory() {
        let servers =
            Servers::new().live("ambient", &[("marked", Some("marked")), ("unmarked", None)]);
        let inventory = take(Vec::new(), Some(&ServerId::new("ambient")), &servers);
        assert_eq!(identities(&inventory), ["live:marked@ambient"]);
        assert!(
            inventory.candidates[0].is_tmux_only(),
            "it is live-only, which is the fact SC-509b's degraded reads later"
        );
    }

    // ---- criterion 7: a failed server query removes nothing ----------------

    #[test]
    fn criterion_7_a_backend_failure_removes_no_durable_candidate_and_no_other_server() {
        let reachable = ServerId::new("sock-up");
        let durable = vec![
            plain("/s/no-pointer"),
            record(
                "/s/points-down",
                RecordedServer::Positive(ServerId::new("sock-down")),
            ),
            record("/s/points-up", RecordedServer::Positive(reachable)),
        ];
        let servers = Servers::new()
            .down("sock-down")
            .live("sock-up", &[("elsewhere", Some("elsewhere"))]);

        let inventory = take(durable, Some(&ServerId::new("ambient")), &servers);

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
            identities(&inventory).contains(&"live:elsewhere@sock-up".to_owned()),
            "the server that DID answer still contributes"
        );
        assert_eq!(
            inventory.unreachable,
            [ServerId::new("sock-down")],
            "the failure is recorded as a fact about the QUERY"
        );
    }

    // ---- criterion 8: the prefix orientation, in the firing direction ------

    #[test]
    fn criterion_8_a_long_live_sibling_never_absorbs_the_short_dead_durable_candidate() {
        // The orientation is the test: durable `mdk` with NO live `mdk`, and a
        // separately live `mdk-app`. Co-occurrence in the other direction is
        // the substitution that proves nothing.
        let servers = Servers::new().live("ambient", &[("mdk-app", Some("mdk-app"))]);
        let inventory = take(
            vec![plain("/s/mdk")],
            Some(&ServerId::new("ambient")),
            &servers,
        );
        assert_eq!(
            identities(&inventory),
            ["durable:/s/mdk", "live:mdk-app@ambient"]
        );
        assert_eq!(
            attached(&inventory, "/s/mdk"),
            None,
            "the sibling did not attach to it either"
        );
    }

    // ---- criterion 9: an exact live name without the marker ---------------

    #[test]
    fn criterion_9a_an_unmarked_exact_live_name_leaves_the_durable_candidate_alone() {
        let servers = Servers::new().live("ambient", &[("mdk", None)]);
        let inventory = take(
            vec![plain("/s/mdk")],
            Some(&ServerId::new("ambient")),
            &servers,
        );
        assert_eq!(identities(&inventory), ["durable:/s/mdk"]);
        assert_eq!(attached(&inventory, "/s/mdk"), None);
    }

    #[test]
    fn criterion_9b_the_same_unmarked_session_with_no_durable_record_is_no_candidate() {
        let servers = Servers::new().live("ambient", &[("mdk", None)]);
        let inventory = take(Vec::new(), Some(&ServerId::new("ambient")), &servers);
        assert!(
            inventory.candidates.is_empty(),
            "positive ownership is what admits a live-only candidate, and there was none"
        );
    }

    // ---- criteria 10/11/12: entitlement, as far as it can be run today -----

    #[test]
    fn criterion_10_only_the_ambient_server_and_recorded_pointers_are_contacted() {
        // A = ambient, B = named by a durable candidate, C = live and reachable
        // to the harness but named by nobody.
        let servers = Servers::new()
            .live("A-ambient", &[("on-a", Some("on-a"))])
            .live("B-pointed-at", &[("on-b", Some("on-b"))])
            .live("C-unnamed", &[("on-c", Some("on-c"))]);
        let durable = vec![record(
            "/s/pointer",
            RecordedServer::Positive(ServerId::new("B-pointed-at")),
        )];

        let inventory = take(durable, Some(&ServerId::new("A-ambient")), &servers);

        let mut contacted = servers.contacted();
        contacted.sort();
        assert_eq!(
            contacted,
            ["A-ambient", "B-pointed-at"],
            "C is never contacted — the trace, not the result, is what shows a sweep"
        );
        assert_eq!(
            identities(&inventory),
            [
                "durable:/s/pointer",
                "live:on-a@A-ambient",
                "live:on-b@B-pointed-at",
            ]
        );
    }

    #[test]
    fn criterion_11_missing_and_ambiguous_selectors_confer_nothing_and_delete_nothing() {
        // The raw bytes an implementation might be tempted to use as a server
        // are live and would answer if asked. They are not asked.
        let servers = Servers::new()
            .live("ambient", &[])
            .live("/tmp/tempting.sock", &[("tempting", Some("tempting"))])
            .live(
                "ambiguous-bytes",
                &[("also-tempting", Some("also-tempting"))],
            );
        let durable = vec![
            plain("/s/no-selector"),
            record("/s/ambiguous", RecordedServer::Ambiguous),
        ];

        let inventory = take(durable, Some(&ServerId::new("ambient")), &servers);

        assert_eq!(
            durable_identities(&inventory),
            ["durable:/s/ambiguous", "durable:/s/no-selector"]
        );
        assert_eq!(servers.contacted(), ["ambient"], "no guessed selector");
        assert_eq!(
            identities(&inventory),
            ["durable:/s/ambiguous", "durable:/s/no-selector"],
            "and the sessions on those servers stayed out"
        );
    }

    #[test]
    fn criterion_12_a_session_outside_the_entitled_set_is_absent_rather_than_classified() {
        let servers = Servers::new()
            .live("ambient", &[])
            .live("C-unnamed", &[("on-c", Some("on-c"))]);
        let inventory = take(Vec::new(), Some(&ServerId::new("ambient")), &servers);
        assert!(
            inventory.candidates.is_empty(),
            "no candidate, and no placeholder standing in for one"
        );
        assert!(
            inventory.unreachable.is_empty(),
            "C is not unreachable — it was never ae's to ask, which is a different fact"
        );
    }

    // ---- criterion 13: no sweep, proven by the trace and by the source -----

    #[test]
    fn criterion_13_unentitled_servers_are_never_contacted_even_to_be_discarded() {
        let scratch = Scratch::new("no-sweep");
        scratch.session("real");
        // Plausible scan bait, on disk beside the real root.
        for bait in ["tmux-1000", "sockets", ".ae-sock"] {
            fs::create_dir_all(scratch.0.join(bait)).expect("bait");
            fs::write(scratch.0.join(bait).join("default"), "socket").expect("bait socket");
        }
        let servers = Servers::new()
            .live("ambient", &[("real", Some("real"))])
            .live("/tmp/tmux-1000/default", &[("swept", Some("swept"))]);

        let inventory = take(
            durable_records(&scratch.roots()).expect("a readable root"),
            Some(&ServerId::new("ambient")),
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
    fn criterion_13_the_only_filesystem_reach_in_this_module_is_the_sessions_root() {
        // The trace above covers the tmux half. The filesystem half is not
        // observable from behavior — non-access has no signal — so it is asked
        // of the SOURCE. Weaker than the compiler probe in the parity self-test
        // (this one reads text), and it is here because the alternative is no
        // check at all on the half of criterion 13 that names socket
        // directories.
        let module = module_source();
        let fs_calls = module.matches("fs::").count();
        assert_eq!(
            fs_calls, 1,
            "exactly one filesystem call outside the tests, and it is read_dir of the sessions root"
        );
        assert!(module.contains("fs::read_dir(roots.sessions())"));
        for sweep_bait in ["tmux-", "/tmp", "socket", "glob", "read_dir(\""] {
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
            vec![
                plain("/s/mdk"),
                plain("/s/quiet"),
                record(
                    "/s/pointed",
                    RecordedServer::Positive(ServerId::new("sock-b")),
                ),
            ]
        };
        let ambient = ServerId::new("ambient");
        let worlds: Vec<(&str, Servers)> = vec![
            (
                "server failure",
                Servers::new().down("ambient").down("sock-b"),
            ),
            (
                "short-dead/long-live prefix sibling",
                Servers::new().live("ambient", &[("mdk-app", Some("mdk-app"))]),
            ),
            (
                "exact live without marker",
                Servers::new().live("ambient", &[("mdk", None), ("quiet", None)]),
            ),
            ("no live server", Servers::new()),
        ];

        let expected = [
            "durable:/s/mdk".to_owned(),
            "durable:/s/pointed".to_owned(),
            "durable:/s/quiet".to_owned(),
        ];
        for (world, servers) in worlds {
            let inventory = take(fixture(), Some(&ambient), &servers);
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
        // Ownership filtering applies to tmux-only discovery ONLY. Here every
        // server fails, so there is no query result at all — and every durable
        // candidate is still present, unmarked ones included.
        let durable = vec![plain("/s/a"), plain("/s/b")];
        let servers = Servers::new().down("ambient");
        let inventory = take(durable, Some(&ServerId::new("ambient")), &servers);
        assert_eq!(
            durable_identities(&inventory),
            ["durable:/s/a", "durable:/s/b"]
        );
    }

    #[test]
    fn criterion_15_the_port_can_only_enumerate_a_server_never_test_one_name() {
        // By construction: `Discovery` takes a server and returns a list. There
        // is no name parameter anywhere in the trait, so a per-candidate
        // existence check is not a thing this phase can express — which is what
        // turned a prefix match into a deletion in the incumbent.
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
        // Needles assembled from halves: this file must not be a place the
        // token it forbids can hide, and scanning only the non-test half is
        // already load-bearing enough without the needle sitting in the other.
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

    // ---- the durable reader, and the invariant it holds by shape -----------

    #[test]
    fn sc_404_the_sessions_root_is_derived_and_the_archive_is_not_reachable() {
        let roots = Roots::under("/home/x/.ae");
        assert_eq!(roots.sessions(), Path::new("/home/x/.ae/sessions"));
    }

    #[test]
    fn sc_017j_a_candidate_with_no_meta_at_all_still_appears() {
        let scratch = Scratch::new("no-meta");
        scratch.session("bare");
        let found = durable_records(&scratch.roots()).expect("a readable root");
        assert_eq!(
            found.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["bare"]
        );
    }

    #[test]
    fn a_file_under_the_sessions_root_is_not_a_session() {
        let scratch = Scratch::new("lock-file");
        scratch.session("real");
        fs::write(
            scratch.0.join("sessions").join(".lifecycle.real.lock"),
            "held",
        )
        .expect("a lock fixture");
        let found = durable_records(&scratch.roots()).expect("a readable root");
        assert_eq!(
            found.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["real"]
        );
    }

    #[test]
    fn a_selector_is_carried_verbatim_because_this_crate_does_not_parse_one() {
        for selector in ["default", "/tmp/ae-1000/socket", ""] {
            assert_eq!(ServerId::new(selector).as_str(), selector);
        }
    }

    #[test]
    fn a_sessions_root_that_exists_and_will_not_read_is_an_error() {
        // "I could not look" must not render as "there is nothing there".
        let scratch = Scratch::new("root-is-a-file");
        fs::write(scratch.0.join("sessions"), "not a directory").expect("a hostile fixture");
        assert!(durable_records(&scratch.roots()).is_err());
    }

    #[test]
    fn a_missing_sessions_root_is_an_empty_inventory_not_an_error() {
        let scratch = Scratch::new("no-root");
        assert_eq!(
            durable_records(&scratch.roots()).expect("a fresh machine is not a failure"),
            Vec::new()
        );
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

        let found = durable_records(&scratch.roots()).expect("a readable root");
        assert_eq!(found.len(), usize::from(unspellable) + 1);
        if unspellable {
            assert!(
                found.iter().any(|record| record.name.contains('\u{FFFD}')),
                "the name is unspellable, so it is lossy — and still inventory"
            );
        }
    }

    #[test]
    fn the_durable_reader_orders_by_path_rather_than_by_traversal() {
        // A property of the READER, not of the candidate collection: criterion
        // 18 leaves collection order open, and no test here pins it.
        let scratch = Scratch::new("order");
        for name in ["zulu", "alpha", "mike"] {
            scratch.session(name);
        }
        let found = durable_records(&scratch.roots()).expect("a readable root");
        assert_eq!(
            found.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["alpha", "mike", "zulu"],
            "read_dir order is a filesystem fact and must not reach the answer"
        );
    }

    #[test]
    fn sc_017j_the_entitled_set_is_the_ambient_server_plus_distinct_recorded_pointers() {
        let ambient = ServerId::new("ambient");
        let durable = vec![
            record("/s/one", RecordedServer::Positive(ServerId::new("sock-a"))),
            plain("/s/two"),
            record("/s/three", RecordedServer::Ambiguous),
            record("/s/four", RecordedServer::Positive(ServerId::new("sock-a"))),
        ];
        assert_eq!(
            entitled_servers(Some(&ambient), &durable),
            [ambient, ServerId::new("sock-a")],
            "distinct pointers only: a repeat is not a second entitlement"
        );
    }

    #[test]
    fn criterion_6_two_paths_sharing_a_last_component_are_two_addressable_candidates() {
        // Blocked in its full form (one candidate per durable ROOT — see the
        // gate's blocked table), so this runs the half that does not depend on
        // the second root: distinct paths, one basename, both addressable.
        let durable = vec![plain("/roots/a/my-feature"), plain("/roots/b/my-feature")];
        let inventory = take(durable, None, &Servers::new());
        assert_eq!(
            identities(&inventory),
            ["durable:/roots/a/my-feature", "durable:/roots/b/my-feature"],
            "the basename is not the key; the path is"
        );
    }

    #[test]
    fn a_sighting_attaches_only_on_the_exact_name_and_that_candidate_s_own_server() {
        let durable = vec![record(
            "/s/api",
            RecordedServer::Positive(ServerId::new("sock-a")),
        )];
        let servers = Servers::new().live("sock-a", &[("api", Some("api"))]);
        let inventory = take(durable, None, &servers);
        assert_eq!(identities(&inventory), ["durable:/s/api"], "one, not two");
        assert_eq!(
            attached(&inventory, "/s/api").map(|live| live.name),
            Some("api".to_owned())
        );
    }

    #[test]
    fn a_sighting_from_a_different_server_never_attaches_by_name_alone() {
        let durable = vec![record(
            "/s/api",
            RecordedServer::Positive(ServerId::new("sock-a")),
        )];
        let servers = Servers::new()
            .live("ambient", &[("api", Some("api"))])
            .live("sock-a", &[]);
        let inventory = take(durable, Some(&ServerId::new("ambient")), &servers);
        assert_eq!(
            identities(&inventory),
            ["durable:/s/api", "live:api@ambient"],
            "same name, different server: two identities until a row says otherwise"
        );
    }

    #[test]
    fn an_ambiguous_attachment_attaches_nothing() {
        let server = ServerId::new("sock-a");
        let durable = vec![
            record("/roots/a/twin", RecordedServer::Positive(server.clone())),
            record("/roots/b/twin", RecordedServer::Positive(server)),
        ];
        let servers = Servers::new().live("sock-a", &[("twin", Some("twin"))]);
        let inventory = take(durable, None, &servers);
        assert_eq!(
            identities(&inventory),
            [
                "durable:/roots/a/twin",
                "durable:/roots/b/twin",
                "live:twin@sock-a",
            ],
            "picking either durable twin would invent the answer"
        );
    }
}
