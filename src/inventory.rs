//! Which sessions EXIST, before anything asks whether they are running.
//!
//! The inventory is the union of (a) durable session state under
//! two readable layouts and (b) positively identified ae-owned live
//! tmux sessions on a server ae is already entitled to query. Archives are inert
//! and never enter it. Every durable candidate survives into classification: a
//! failed liveness query, a prefix-only name match, or a live exact-name session
//! whose ownership marker is missing cannot delete the candidate.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::meta::{Selector, ServerSelector};
use crate::session::RecordSnapshot;

/// The nested state directory inside a worktree — legacy layout.
const WORKTREE_STATE_DIR: &str = ".ae";

/// The ae state roots for one invocation, derived from `AE_HOME`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    sessions: PathBuf,
    worktrees: PathBuf,
}

impl Roots {
    /// The roots under `ae_home` — default derivation, both of
    /// layouts.
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

/// Which durable layout a candidate was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// `<AE_HOME>/sessions/<session-name>/`.
    Canonical,
    /// `<AE_HOME>/worktrees/<worktree-name>/.ae/<session-name>/`, where the
    /// outer worktree name and the inner session name may differ.
    WorktreeNested,
}

/// What happened when the record was read.
pub use crate::session::MetaRead;

/// Durable session state found under one of roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableRecord {
    /// The state directory.
    pub path: PathBuf,
    /// The `<session-name>` leaf — the inventory name.
    pub name: String,
    /// Which layout it was found in.
    pub layout: Layout,
    /// The normalized server selector.
    pub server: ServerSelector,
    /// What reading its `meta` did — see [`MetaRead`].
    pub meta_read: MetaRead,
    /// Everything the record said, read ONCE, at discovery.
    pub snapshot: RecordSnapshot,
}

/// A tmux server ae holds a pointer to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerId {
    /// The server this invocation's ordinary transport already selected, with
    /// no explicit selector of its own.
    Ambient,
    /// A server named by a positive, unambiguous durable selector.
    Selected(Selector),
}

impl ServerId {
    /// The server a `--server-kind <kind> --server <value>` pair names, or the
    /// refusal to guess one.
    ///
    /// # Errors
    ///
    /// The operator-facing line, naming what it could not use.
    pub fn from_typed_flags(kind: &str, value: &str) -> Result<Self, String> {
        match kind {
            "" if value.is_empty() => Ok(Self::Ambient),
            "socket" if !value.is_empty() => {
                Ok(Self::Selected(Selector::Socket(PathBuf::from(value))))
            }
            "name" if !value.is_empty() => Ok(Self::Selected(Selector::Name(value.to_owned()))),
            "" => Err(
                "Error: --server was given without a --server-kind, so ae cannot tell a socket path from a server name; ae will not fall back to the ambient server."
                    .to_owned(),
            ),
            "socket" | "name" => Err(format!(
                "Error: --server-kind {kind} needs a --server value; ae will not fall back to the ambient server."
            )),
            other => Err(format!(
                "Error: '{other}' is not a tmux server kind ae can resolve (expected 'socket' or 'name'); ae will not fall back to the ambient server."
            )),
        }
    }
}

/// One session an entitled server reported, before ae decides whether it is its
/// own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSession {
    /// The exact session name the server reported.
    pub name: String,
    /// The `AE_SESSION` marker the server holds for it, if any.
    pub marker: Option<String>,
}

/// A server query that did not answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryFailed;

/// The one tmux question phase 1 is allowed to ask.
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
    /// The server that answered.
    pub server: ServerId,
    /// Its exact session name.
    pub name: String,
    /// The `AE_SESSION` value the server reported.
    pub marker: String,
}

/// Which sources established a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// Durable state only — no entitled server reported it.
    Durable,
    /// A live session only — no durable record was found for it.
    Live,
    /// Both, positively matched.
    Both,
}

/// One inventory candidate: durable state, a live sighting, or both.
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

/// A logical source whose terminal enumeration failed.
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DurableScan {
    /// Every durable candidate found.
    pub records: Vec<DurableRecord>,
    /// Durable sources whose enumeration failed.
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

/// Every durable candidate under `roots`, both layouts, path order.
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
                // The candidate is the NESTED state directory.
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
        reason = "a door: the durable-root enumeration — see clippy.toml"
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
        // session dirs).
        if entry.file_type().is_ok_and(|kind| !kind.is_dir()) {
            continue;
        }
        // A dot-directory is ae's own infrastructure, never a session: `.locks`
        // holds the lifecycle locks and sits right beside the session dirs.
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with('.'))
        {
            continue;
        }
        found.push(entry.path());
    }
    Ok(found)
}

/// The durable record for the state directory at `path`.
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
    // ONE read of this record, here, feeding both the selector and every field
    // the digest will need.
    record.snapshot = RecordSnapshot::read(&record.path);
    // BOTH facts from the ONE read.
    record.meta_read = record.snapshot.meta_read;
    if let Some(meta) = &record.snapshot.meta {
        // No selector is derived from bytes nobody could read — the alternative
        // is querying a server on a guess.
        record.server = meta.server_selector();
    }
    record
}

/// The servers ae is entitled to enumerate, most-ambient first.
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
    /// Every logical source whose enumeration failed.
    pub incomplete: Vec<FailedSource>,
}

impl Inventory {
    /// Whether every enumeration completed — snapshot fact.
    #[must_use]
    pub const fn complete(&self) -> bool {
        self.incomplete.is_empty()
    }

    /// The entitled servers that did not answer.
    pub fn unreachable(&self) -> impl Iterator<Item = &ServerId> {
        self.incomplete.iter().filter_map(|source| match source {
            FailedSource::Server(server) => Some(server),
            FailedSource::CanonicalRoot(_)
            | FailedSource::WorktreeRoot(_)
            | FailedSource::WorktreeState(_) => None,
        })
    }
}

/// Take the inventory: durable records unioned with what the entitled
/// servers report.
///
/// Every durable record becomes a candidate and stays one — this function starts
/// as all of them and only ever pushes. `discovery` is called for the entitled
/// servers and for no others, so "ae does not gain entitlement by sweeping" is a
/// property of the call sequence rather than of a filter applied afterwards.
///
/// **Discovery completes before reconciliation**. Every sighting is
/// gathered first, so no join can depend on which server answered first.
///
/// A sighting joins a durable candidate only on join witness: that
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
            // Third source class.
            inventory.incomplete.push(FailedSource::Server(server));
            continue;
        };
        for session in sessions {
            // No ownership evidence: not ae's, so not a candidate — and still
            // not a reason to touch a durable candidate that happens to share
            // its name.
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

    //! Each test names the pre-registered criterion of the retired phase-1
    //! gate document it answers. That gate was authored
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

    /// `.locks` sits beside the session directories and is not a session.
    #[test]
    fn a_dot_directory_beside_the_sessions_is_not_a_candidate() {
        let scratch = Scratch::new("dot-dirs");
        scratch.session("real");
        let sessions = scratch.0.join("sessions");
        for internal in [".locks", ".cache"] {
            fs::create_dir_all(sessions.join(internal)).expect("an internal dir");
        }

        let found = super::child_dirs(&sessions).expect("the scan succeeds");
        let names: Vec<String> = found
            .iter()
            .filter_map(|path| path.file_name()?.to_str().map(ToOwned::to_owned))
            .collect();

        assert_eq!(names, ["real"], "only the session survives the scan");
        assert!(
            !names.iter().any(|name| name.starts_with('.')),
            "no dot-entry is ever a candidate: {names:?}"
        );
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

    // ---- criterion 2: both durable layouts ---------------------------------

    #[test]
    fn criterion_2_both_durable_layouts_are_discovered_and_the_inner_leaf_is_the_name() {
        let scratch = Scratch::new("two-layouts");
        let canonical = scratch.session("canonical-one");
        // The outer worktree name DIFFERS from the inner session leaf
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
        // directory ae could not LIST is not one that is not there.
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
        // A worktree with no nested state directory is simply not a candidate.
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
            // without ever creating the condition it names.
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
        // THE CELL A SECOND OBSERVATION GETS WRONG.
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
        // traverse anyway and the cell would not exist.
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
        // separately live `mdk-app`.
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
        // are live and would answer if asked.
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
        // The trace above covers the tmux half.
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
        // DIFFER.
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
        // THE ARM A COUNT PASSES AND A FLAG DOES NOT.
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
        // anything.
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
        // durable and server discovery.
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

    // ---- criterion 23: the entitlement half of the selector ----------------

    #[test]
    fn criterion_23_an_absent_or_unreadable_meta_normalizes_to_missing_not_to_a_fifth_state() {
        // The trap this pairs with criterion 21: `missing` is what the reader
        // says when no selector fact is available to it, so a record it could
        // not read at all is `missing` — never `ambiguous`, which is reserved
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
        // the normalizer it tests.
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
        // Unproved equivalence between selector spellings never authorizes a
        // merge.
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
        let scratch = Scratch::new("root-is-a-file");
        fs::write(scratch.0.join("sessions"), "not a directory").expect("a hostile fixture");
        let nested = scratch.nested("checkout", "survivor");
        // The fixture must prove the ENUMERATION returned an error, not merely
        // that something looks odd.
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
        // THE POINT OF THIS ARM: you cannot count what you could not see.
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
        // lost a source.
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
        // discovered, its meta is record-loss fact and never becomes snapshot
        // incompleteness.
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
        // macOS the arm below cannot be built at all.
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
