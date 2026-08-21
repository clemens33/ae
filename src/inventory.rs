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
//! * **The live sightings.** Enumerating a tmux server is I/O this crate has no
//!   transport for yet, so sightings arrive as data — tagged with the server
//!   they were seen on, which is what makes the entitlement filter checkable
//!   rather than assumed.
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

/// Whether a live tmux session carries positive ae-ownership evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    /// Positively identified as ae's.
    AeOwned,
    /// No ownership evidence. SC-017j admits only positively identified
    /// sessions, so this one does not become a candidate — and, critically,
    /// does not delete a durable candidate that shares its name either.
    Unmarked,
}

/// One live tmux session, as an enumeration of a server reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSession {
    /// The server it was seen on. Not "the ambient server" — the one that
    /// answered, which is what makes the entitlement filter checkable.
    pub server: ServerId,
    /// Its exact session name.
    pub name: String,
    /// Whether it is positively ae's.
    pub ownership: Ownership,
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
    pub live: Option<LiveSession>,
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
    pub fn tmux_only(sighting: LiveSession) -> Self {
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

/// The SC-017j inventory: durable records unioned with entitled live sightings.
///
/// Every durable record becomes a candidate and stays one. A sighting joins the
/// inventory only when it is positively ae-owned AND was seen on an entitled
/// server; otherwise it is absent by epistemic limit, which is not the same as
/// being classified stopped.
///
/// A sighting ATTACHES to a durable candidate only on positive evidence that
/// they are the same session: the candidate records that exact server, and the
/// names match exactly. Anything weaker attaches nothing and leaves a tmux-only
/// candidate beside the durable one — a visible duplicate is recoverable, a
/// wrong merge is a fabricated identity.
///
/// ```
/// use ae::inventory::{LiveSession, Ownership, ServerId, candidates};
///
/// let ambient = ServerId::new("default");
/// let sighting = LiveSession {
///     server: ambient.clone(),
///     name: "my-feature".to_owned(),
///     ownership: Ownership::AeOwned,
/// };
/// let inventory = candidates(Vec::new(), Some(&ambient), &[sighting]);
/// assert_eq!(inventory.len(), 1);
/// assert!(inventory[0].is_tmux_only());
/// ```
#[must_use]
pub fn candidates(
    durable: Vec<DurableRecord>,
    ambient: Option<&ServerId>,
    sighted: &[LiveSession],
) -> Vec<Candidate> {
    let entitled = entitled_servers(ambient, &durable);
    let mut inventory: Vec<Candidate> = durable.into_iter().map(Candidate::durable).collect();
    let mut seen: Vec<(&ServerId, &str)> = Vec::new();
    for sighting in sighted {
        if sighting.ownership != Ownership::AeOwned {
            continue;
        }
        if !entitled.contains(&sighting.server) {
            continue;
        }
        // Enumerating the same server twice is one sighting, not two.
        let key = (&sighting.server, sighting.name.as_str());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        match attachment(&inventory, sighting) {
            Some(at) => inventory[at].live = Some(sighting.clone()),
            None => inventory.push(Candidate::tmux_only(sighting.clone())),
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
fn attachment(inventory: &[Candidate], sighting: &LiveSession) -> Option<usize> {
    let mut found = None;
    for (at, candidate) in inventory.iter().enumerate() {
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
    use super::{
        Candidate, DurableRecord, LiveSession, Ownership, RecordedServer, Roots, ServerId,
        candidates, durable_records, entitled_servers,
    };
    use std::fs;
    use std::path::{Path, PathBuf};

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

    fn names(inventory: &[Candidate]) -> Vec<&str> {
        inventory.iter().map(|c| c.name.as_str()).collect()
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

    fn sighting(server: &str, name: &str) -> LiveSession {
        LiveSession {
            server: ServerId::new(server),
            name: name.to_owned(),
            ownership: Ownership::AeOwned,
        }
    }

    #[test]
    fn sc_404_the_sessions_root_is_derived_and_the_archive_is_not_reachable() {
        let roots = Roots::under("/home/x/.ae");
        assert_eq!(roots.sessions(), Path::new("/home/x/.ae/sessions"));
    }

    #[test]
    fn sc_017j_a_candidate_with_no_readable_meta_still_appears() {
        // The reader opens nothing inside the directory, so there is no parse
        // that could fail and remove it. A directory with no meta at all is the
        // strongest form of that: it is still a candidate.
        let scratch = Scratch::new("no-meta");
        scratch.session("bare");
        let with_junk = scratch.session("damaged");
        fs::write(with_junk.join("meta"), "\u{0}not=a=meta\n").expect("a broken fixture");

        let found = durable_records(&scratch.roots()).expect("a readable root");
        assert_eq!(
            found.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["bare", "damaged"]
        );
    }

    #[test]
    fn sc_017j_the_archive_contributes_nothing() {
        // Archives are inert. The reader cannot even name the archive root:
        // `Roots` derives the sessions root and nothing else.
        let scratch = Scratch::new("archive");
        scratch.session("live-one");
        let archive = scratch.0.join("archive").join("9f0c-uuid");
        fs::create_dir_all(&archive).expect("an archive fixture");
        fs::write(archive.join("meta"), "mode=local\n").expect("an archived meta");

        let found = durable_records(&scratch.roots()).expect("a readable root");
        assert_eq!(
            found.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["live-one"]
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
        // Phase 2 hands this to a tmux transport. Whatever a selector means, it
        // means it there — round-tripping it unchanged is this type's whole job.
        for selector in ["default", "/tmp/ae-1000/socket", ""] {
            assert_eq!(ServerId::new(selector).as_str(), selector);
        }
    }

    #[test]
    fn a_sessions_root_that_exists_and_will_not_read_is_an_error() {
        // The other side of the NotFound exception: "I could not look" must not
        // render as "there is nothing there", which is #105's shape one level
        // up. A regular file where the root belongs fails with NotADirectory on
        // every platform, without depending on permissions or on who is running.
        let scratch = Scratch::new("root-is-a-file");
        fs::write(scratch.0.join("sessions"), "not a directory").expect("a hostile fixture");
        assert!(
            durable_records(&scratch.roots()).is_err(),
            "an unreadable root is reported, never answered as empty"
        );
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
        // Dropping an unspellable name is the disappearance this module exists
        // to forbid; the bytes survive in the path, which is the identity.
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
    fn the_reader_orders_by_path_rather_than_by_traversal() {
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
    fn sc_017j_the_entitled_set_is_the_ambient_server_plus_recorded_pointers() {
        let ambient = ServerId::new("ambient");
        let durable = vec![
            record("/s/one", RecordedServer::Positive(ServerId::new("sock-a"))),
            record("/s/two", RecordedServer::Missing),
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
    fn sc_017j_a_missing_or_ambiguous_selector_confers_no_entitlement() {
        let durable = vec![
            record("/s/one", RecordedServer::Missing),
            record("/s/two", RecordedServer::Ambiguous),
        ];
        assert!(
            entitled_servers(None, &durable).is_empty(),
            "an ambiguous pointer is not a pointer"
        );
        // ...and the candidates themselves are untouched by having none.
        let inventory = candidates(durable, None, &[]);
        assert_eq!(names(&inventory), ["one", "two"]);
    }

    #[test]
    fn sc_017j_a_live_session_with_no_durable_record_still_appears() {
        let ambient = ServerId::new("ambient");
        let inventory = candidates(Vec::new(), Some(&ambient), &[sighting("ambient", "ghost")]);
        assert_eq!(names(&inventory), ["ghost"]);
        assert!(inventory[0].is_tmux_only());
    }

    #[test]
    fn sc_017j_a_live_session_on_an_unentitled_server_is_absent_by_epistemic_limit() {
        // "not classified stopped or unknown" — it is not in the inventory at
        // all, and it takes nothing with it.
        let durable = vec![record("/s/kept", RecordedServer::Missing)];
        let inventory = candidates(
            durable,
            Some(&ServerId::new("ambient")),
            &[sighting("some-other-socket", "elsewhere")],
        );
        assert_eq!(names(&inventory), ["kept"]);
    }

    #[test]
    fn sc_017j_an_unmarked_live_session_is_not_a_candidate_and_deletes_nothing() {
        // The #105 shape: a live session missing its ownership marker used to
        // remove the durable directory that shared its name.
        let durable = vec![record("/s/mine", RecordedServer::Missing)];
        let unmarked = LiveSession {
            ownership: Ownership::Unmarked,
            ..sighting("ambient", "mine")
        };
        let inventory = candidates(durable, Some(&ServerId::new("ambient")), &[unmarked]);
        assert_eq!(names(&inventory), ["mine"]);
        assert!(
            inventory[0].live.is_none(),
            "unmarked evidence attaches nothing"
        );
        assert!(
            !inventory[0].is_tmux_only(),
            "and the durable record stands"
        );
    }

    #[test]
    fn sc_017j_two_paths_sharing_a_last_component_are_two_candidates() {
        // The row does NOT authorize basename-only deduplication of distinct
        // identities. Identity is the path.
        let durable = vec![
            record("/roots/a/my-feature", RecordedServer::Missing),
            record("/roots/b/my-feature", RecordedServer::Missing),
        ];
        let inventory = candidates(durable, None, &[]);
        assert_eq!(names(&inventory), ["my-feature", "my-feature"]);
        assert_ne!(
            inventory[0].durable.as_ref().map(|r| &r.path),
            inventory[1].durable.as_ref().map(|r| &r.path)
        );
    }

    #[test]
    fn a_sighting_attaches_only_on_the_exact_name_and_the_recorded_server() {
        let server = ServerId::new("sock-a");
        let durable = vec![record("/s/api", RecordedServer::Positive(server.clone()))];
        let inventory = candidates(durable, None, &[sighting("sock-a", "api")]);
        assert_eq!(names(&inventory), ["api"], "one candidate, not two");
        assert_eq!(
            inventory[0].live.as_ref().map(|l| l.name.as_str()),
            Some("api")
        );
    }

    #[test]
    fn a_prefix_sibling_never_attaches_in_either_direction() {
        // SC-017k's exact-name hazard, arriving one phase early: `tmux
        // has-session -t api` matches `api-v2`, which is how a sibling used to
        // answer for a session that was not there.
        let server = ServerId::new("sock-a");
        let durable = vec![
            record("/s/api", RecordedServer::Positive(server.clone())),
            record("/s/api-v2", RecordedServer::Positive(server.clone())),
        ];
        let inventory = candidates(durable, None, &[sighting("sock-a", "api-v2-extra")]);
        assert_eq!(
            names(&inventory),
            ["api", "api-v2", "api-v2-extra"],
            "the sighting is its own candidate; neither durable record absorbed it"
        );
        assert!(inventory[0].live.is_none());
        assert!(inventory[1].live.is_none());
    }

    #[test]
    fn a_sighting_from_a_different_server_never_attaches_by_name_alone() {
        let durable = vec![record(
            "/s/api",
            RecordedServer::Positive(ServerId::new("sock-a")),
        )];
        let inventory = candidates(
            durable,
            Some(&ServerId::new("ambient")),
            &[sighting("ambient", "api")],
        );
        assert_eq!(
            names(&inventory),
            ["api", "api"],
            "same name, different server: two identities until a row says otherwise"
        );
    }

    #[test]
    fn an_ambiguous_attachment_attaches_nothing() {
        // Two identities recording the same server under the same name. Picking
        // either is inventing the answer, so the sighting stands alone.
        let server = ServerId::new("sock-a");
        let durable = vec![
            record("/roots/a/twin", RecordedServer::Positive(server.clone())),
            record("/roots/b/twin", RecordedServer::Positive(server.clone())),
        ];
        let inventory = candidates(durable, None, &[sighting("sock-a", "twin")]);
        assert_eq!(names(&inventory), ["twin", "twin", "twin"]);
        assert_eq!(inventory.iter().filter(|c| c.live.is_some()).count(), 1);
        assert!(inventory[2].is_tmux_only());
    }

    #[test]
    fn enumerating_the_same_server_twice_adds_one_candidate() {
        let ambient = ServerId::new("ambient");
        let inventory = candidates(
            Vec::new(),
            Some(&ambient),
            &[sighting("ambient", "once"), sighting("ambient", "once")],
        );
        assert_eq!(names(&inventory), ["once"]);
    }

    #[test]
    fn the_same_name_on_two_entitled_servers_is_two_candidates() {
        let durable = vec![record(
            "/s/pointer",
            RecordedServer::Positive(ServerId::new("sock-a")),
        )];
        let inventory = candidates(
            durable,
            Some(&ServerId::new("ambient")),
            &[sighting("ambient", "shared"), sighting("sock-a", "shared")],
        );
        assert_eq!(names(&inventory), ["pointer", "shared", "shared"]);
    }

    #[test]
    fn a_durable_record_is_never_removed_by_anything_the_live_side_reports() {
        // The invariant, stated as one test: every hostile live report at once.
        let durable = vec![
            record("/s/keep-me", RecordedServer::Missing),
            record("/s/keep-me-too", RecordedServer::Ambiguous),
        ];
        let hostile = [
            sighting("ambient", "keep-me"),      // exact name, no recorded server
            sighting("ambient", "keep-me-too2"), // prefix sibling
            LiveSession {
                ownership: Ownership::Unmarked,
                ..sighting("ambient", "keep-me-too")
            },
            sighting("unentitled-socket", "keep-me"),
        ];
        let inventory = candidates(durable, Some(&ServerId::new("ambient")), &hostile);
        let kept: Vec<&str> = inventory
            .iter()
            .filter(|c| !c.is_tmux_only())
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(kept, ["keep-me", "keep-me-too"]);
    }
}
