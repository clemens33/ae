//! Frozen appfleet contracts. The baseline uses the existing full readers;
//! driver adapters invoke these same functions through Journals and the REAL
//! Reader. Byte counters judge consumption, fresh reads judge answers.

use std::collections::BTreeSet;
use std::fs::{self, FileTimes, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::loader::{Answer, FleetRead, Reader, ViewRead};
use super::tests::{ID, Root};
use crate::brief::{self, Filed};
use crate::console::Console;
use crate::inventory::{FailedSource, ServerId};
use crate::session::RecordSnapshot;
use crate::time::Timestamp;

pub(super) trait Files {
    fn snapshot(&mut self, dir: &Path) -> RecordSnapshot;
    fn memo(&mut self, dir: &Path) -> io::Result<Vec<Filed>>;
    /// Cumulative bytes handed to the journal and memo parsers.
    fn costs(&self) -> (u64, u64);
    fn retain(&mut self, dirs: &[PathBuf]);
    fn cached_dirs(&self) -> BTreeSet<PathBuf>;
}

pub(super) trait ReaderFiles {
    fn fleet(&mut self) -> FleetRead;
    fn view(&mut self, name: &str) -> Option<ViewRead>;
    fn costs(&self) -> (u64, u64);
    fn cached_dirs(&self) -> BTreeSet<PathBuf>;
}

fn event(words: &str) -> Vec<u8> {
    format!(
        "{{\"ts\":\"2026-01-01T00:00:00Z\",\"actor\":\"lead\",\"action\":\"goal\",\"summary\":\"{words}\"}}\n"
    )
    .into_bytes()
}

fn memo(words: &str) -> Vec<u8> {
    format!("2026-01-01T00:00:00Z\tlead\tgoal\t{words}\n").into_bytes()
}

fn seed(root: &Root, name: &str) -> PathBuf {
    let dir = root.0.join("sessions").join(name);
    fs::create_dir_all(&dir).expect("fixture session");
    fs::write(
        dir.join("meta"),
        format!(
            "schema=2\nsession_id={ID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\n"
        ),
    )
    .expect("fixture meta");
    fs::write(dir.join("events.jsonl"), event("first")).expect("fixture journal");
    fs::write(dir.join("memo.tsv"), memo("first")).expect("fixture memo");
    dir
}

fn append(path: &Path, bytes: &[u8]) {
    OpenOptions::new()
        .append(true)
        .open(path)
        .expect("append fixture")
        .write_all(bytes)
        .expect("append bytes");
}

fn length(path: &Path) -> u64 {
    OpenOptions::new()
        .read(true)
        .open(path)
        .and_then(|file| file.metadata())
        .map_or(0, |meta| meta.len())
}

fn same_files<F: Files>(subject: &mut F, dir: &Path) {
    assert_eq!(
        subject.snapshot(dir),
        RecordSnapshot::read(dir),
        "snapshot {dir:?}"
    );
    let actual = subject.memo(dir).map_err(|why| why.kind());
    let expected = crate::store::open(dir)
        .memo_bytes()
        .map(|bytes| brief::filed(&bytes))
        .map_err(|why| why.kind());
    assert_eq!(actual, expected, "memo {dir:?}");
}

/// A sequence, not isolated cold reads: every mutation follows a held read.
pub(super) fn file_changes<F: Files>(make: impl Fn() -> F) {
    let root = Root::new("fleet-spec-files");
    let dir = seed(&root, "api");
    let (journal, notes) = (dir.join("events.jsonl"), dir.join("memo.tsv"));
    let mut subject = make();
    same_files(&mut subject, &dir);
    for words in ["second", "third"] {
        append(&journal, &event(words));
        append(&notes, &memo(words));
        same_files(&mut subject, &dir);
    }
    let partial = event("partial");
    append(&journal, &partial[..partial.len() / 2]);
    same_files(&mut subject, &dir);
    append(&journal, &partial[partial.len() / 2..]);
    same_files(&mut subject, &dir);
    append(&journal, b"\r\nnot-json\n\xff\xfe\n");
    append(&journal, &event("after-damage"));
    same_files(&mut subject, &dir);
    let damaged = subject.snapshot(&dir).events.expect("readable damage");
    assert_eq!(damaged.skipped.len(), 2, "invalid records survive cache");
    assert_eq!(
        damaged.cursor.offset,
        length(&journal),
        "absolute byte cursor"
    );
    for bytes in [
        event("rewrite-longer-than-first"),
        event("tiny"),
        Vec::new(),
    ] {
        fs::write(&journal, bytes).expect("same inode rewrite/truncate");
        fs::write(&notes, memo("rewritten")).expect("memo rewrite");
        same_files(&mut subject, &dir);
    }
    fs::write(&journal, event("again")).expect("grow after truncate");
    same_files(&mut subject, &dir);
    let replacement = dir.join("replacement");
    for (path, bytes) in [(&journal, event("new inode")), (&notes, memo("new inode"))] {
        fs::write(&replacement, bytes).expect("replacement file");
        fs::rename(&replacement, path).expect("replace inode");
    }
    same_files(&mut subject, &dir);
    fs::write(
        dir.join("meta"),
        format!(
            "schema=2\nsession_id={}\nseat.main=other\n",
            ID.replace("1234", "bbbb")
        ),
    )
    .expect("new identity, unchanged journal");
    same_files(&mut subject, &dir);
    fs::remove_file(dir.join("meta")).expect("absent meta");
    same_files(&mut subject, &dir);
}

/// Different nanosecond mtimes distinguish same-second, same-length rewrites.
pub(super) fn file_identity<F: Files>(make: impl Fn() -> F) {
    let root = Root::new("fleet-spec-identity");
    let dir = seed(&root, "api");
    let path = dir.join("events.jsonl");
    let notes = dir.join("memo.tsv");
    let base = SystemTime::UNIX_EPOCH + Duration::from_hours(500_000);
    let stamp = |path: &Path, nanos| {
        OpenOptions::new()
            .write(true)
            .open(path)
            .expect("mtime fixture")
            .set_times(FileTimes::new().set_modified(base + Duration::from_nanos(nanos)))
            .expect("nanosecond mtime");
    };
    stamp(&path, 100);
    stamp(&notes, 100);
    let mut subject = make();
    same_files(&mut subject, &dir);
    fs::write(&path, event("other")).expect("same length rewrite");
    fs::write(&notes, memo("other")).expect("same length memo rewrite");
    stamp(&path, 200);
    stamp(&notes, 200);
    same_files(&mut subject, &dir);
    let original = subject.snapshot(&dir).events.expect("journal");
    assert_eq!(original.events[0].summary.as_deref(), Some("other"));
}

/// Preserve each reader's existing missing/nonregular/error meanings.
pub(super) fn file_shapes<F: Files>(make: impl Fn() -> F) {
    let root = Root::new("fleet-spec-shapes");
    let dir = seed(&root, "api");
    let (journal, notes) = (dir.join("events.jsonl"), dir.join("memo.tsv"));
    let mut subject = make();
    same_files(&mut subject, &dir);
    for path in [&journal, &notes] {
        fs::remove_file(path).expect("delete held file");
    }
    same_files(&mut subject, &dir);
    assert!(
        subject
            .snapshot(&dir)
            .events
            .expect("absent is empty")
            .events
            .is_empty()
    );
    for path in [&journal, &notes] {
        fs::create_dir(path).expect("directory replaces file");
    }
    same_files(&mut subject, &dir);
    assert!(
        subject.snapshot(&dir).events.is_none(),
        "journal directory is unreadable"
    );
    assert!(
        subject
            .memo(&dir)
            .expect("memo directory is empty")
            .is_empty()
    );
    for path in [&journal, &notes] {
        fs::remove_dir(path).expect("remove nonregular");
    }
    fs::write(&journal, event("recovered")).expect("recreate journal");
    fs::write(&notes, memo("recovered")).expect("recreate memo");
    same_files(&mut subject, &dir);
    for path in [&journal, &notes] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o0)).expect("unreadable fixture");
    }
    if matches!(
        crate::store::read_source(&journal),
        crate::store::SourceRead::Unreadable(_)
    ) {
        same_files(&mut subject, &dir);
        assert!(subject.snapshot(&dir).events.is_none());
        assert_eq!(
            subject.memo(&dir).expect_err("unreadable memo").kind(),
            io::ErrorKind::PermissionDenied
        );
    }
    for path in [&journal, &notes] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("restore permissions");
    }
    same_files(&mut subject, &dir);
    fs::remove_file(&notes).expect("replace memo by dangling link");
    symlink(dir.join("missing"), &notes).expect("dangling memo");
    same_files(&mut subject, &dir);
    let target = dir.join("target");
    fs::write(&target, memo("target one")).expect("link target appears");
    fs::remove_file(&notes).expect("remove dangling link");
    symlink(&target, &notes).expect("live memo link");
    same_files(&mut subject, &dir);
    fs::write(&target, memo("target two!")).expect("target length changes");
    same_files(&mut subject, &dir);
}

pub(super) fn file_costs<F: Files>(make: impl Fn() -> F) {
    let root = Root::new("fleet-spec-cost");
    let dir = seed(&root, "api");
    let mut subject = make();
    same_files(&mut subject, &dir);
    let warm = subject.costs();
    assert!(warm.0 > 0 && warm.1 > 0, "cold parses both fixture files");
    same_files(&mut subject, &dir);
    assert_eq!(
        subject.costs(),
        warm,
        "unchanged journal AND memo parse zero bytes"
    );
    append(&dir.join("events.jsonl"), &event("append"));
    same_files(&mut subject, &dir);
    assert_eq!(
        subject.costs(),
        (warm.0 + length(&dir.join("events.jsonl")), warm.1),
        "change reparses the WHOLE journal, memo remains cached"
    );
    fs::write(dir.join("memo.tsv"), memo("change")).expect("changed memo");
    let before = subject.costs();
    same_files(&mut subject, &dir);
    assert_eq!(
        subject.costs(),
        (before.0, before.1 + length(&dir.join("memo.tsv"))),
        "memo change leaves journal cached"
    );
}

pub(super) fn file_eviction<F: Files>(make: impl Fn() -> F) {
    let root = Root::new("fleet-spec-retain");
    let (one, two) = (seed(&root, "api"), seed(&root, "web"));
    let mut subject = make();
    same_files(&mut subject, &one);
    same_files(&mut subject, &two);
    assert_eq!(
        subject.cached_dirs(),
        BTreeSet::from([one.clone(), two.clone()])
    );
    subject.retain(std::slice::from_ref(&two));
    assert_eq!(subject.cached_dirs(), BTreeSet::from([two]));
    let before = subject.costs();
    same_files(&mut subject, &one);
    assert_eq!(
        subject.costs(),
        (
            before.0 + length(&one.join("events.jsonl")),
            before.1 + length(&one.join("memo.tsv"))
        ),
        "evicted source re-enters cold"
    );
    subject.retain(&[]);
    assert!(subject.cached_dirs().is_empty(), "no removed source held");
}

/// Oracle uses today's FULL world route and today's fresh fold, never cache.
fn full_fleet(root: &Path) -> FleetRead {
    let (snapshot, world) = crate::current_world(root);
    let dirs = snapshot
        .sessions
        .iter()
        .filter_map(|session| {
            let durable = session.candidate.durable.as_ref()?;
            Some((session.candidate.name.clone(), durable.path.clone()))
        })
        .collect();
    let reader = Reader::new(root.to_path_buf(), None, None);
    let mut read = reader.fold(dirs, world, None, &crate::fleet_order(), Timestamp::now());
    read.scanned = !snapshot
        .incomplete
        .iter()
        .any(|source| !matches!(source, FailedSource::Server(_)));
    read
}

fn equal_fleet(actual: &FleetRead, expected: &FleetRead) {
    assert_eq!(actual.dirs, expected.dirs, "dirs");
    assert_eq!(actual.ids, expected.ids, "ids");
    assert_eq!(actual.world, expected.world, "world/verdicts");
    assert_eq!(actual.facts, expected.facts, "facts");
    assert_eq!(actual.needs, expected.needs, "needs");
    assert_eq!(actual.fleet, expected.fleet, "rows");
    assert_eq!(actual.pairs, expected.pairs, "pairs");
    assert_eq!(actual.scanned, expected.scanned, "scanned");
    assert_eq!(actual.memos, expected.memos, "memos");
}

fn same_fleet<R: ReaderFiles>(subject: &mut R, root: &Path) -> FleetRead {
    for _ in 0..12 {
        let start = Timestamp::now();
        let actual = subject.fleet();
        let expected = full_fleet(root);
        if Timestamp::now() == start {
            equal_fleet(&actual, &expected);
            return actual;
        }
    }
    panic!("same-second fleet observation could not settle");
}

fn same_view<R: ReaderFiles>(subject: &mut R, dir: &Path, name: &str) -> u64 {
    for _ in 0..12 {
        let start = Timestamp::now();
        let actual = subject.view(name).expect("listed session view");
        let mut console = Console::open_standing(name.to_owned(), dir.to_path_buf());
        let mut fresh = console.read().expect("fresh console");
        let needs = match fresh.needs {
            Ok(section) => Some(section),
            Err(why) => {
                fresh.lane.coverage.push(format!("needs you unread: {why}"));
                None
            }
        };
        if Timestamp::now() != start {
            continue;
        }
        assert_eq!(actual.name, name);
        assert_eq!(actual.id, console.uuid());
        assert_eq!(actual.lane, fresh.lane, "lane");
        assert_eq!(actual.needs, needs, "view needs");
        assert_eq!(actual.roster.as_deref(), console.roster(), "view roster");
        return actual.seq;
    }
    panic!("same-second view observation could not settle");
}

pub(super) fn reader_changes<R: ReaderFiles>(make: impl Fn(&Path) -> R) {
    let root = Root::new("fleet-spec-reader");
    let api = seed(&root, "api");
    let mut subject = make(&root.0);
    same_fleet(&mut subject, &root.0);
    let seq = same_view(&mut subject, &api, "api");
    append(&api.join("events.jsonl"), &event("appended"));
    append(&api.join("events.jsonl"), b"{\"ts\":\"2026-01-01T00:00:00Z\",\"actor\":\"lead\",\"action\":\"ask\",\"target\":\"colead\",\"ref\":\"open-spec\"}\n");
    append(&api.join("memo.tsv"), &memo("appended"));
    same_fleet(&mut subject, &root.0);
    assert!(
        same_view(&mut subject, &api, "api") > seq,
        "view sequence advances"
    );
    fs::write(api.join("events.jsonl"), event("rewritten")).expect("rewrite after warm view");
    same_fleet(&mut subject, &root.0);
    same_view(&mut subject, &api, "api");
    let web = seed(&root, "web");
    let added = same_fleet(&mut subject, &root.0);
    assert!(added.ids.contains_key("web"));
    same_view(&mut subject, &web, "web");
    fs::rename(&web, root.0.join("sessions/renamed")).expect("rename session");
    let renamed = same_fleet(&mut subject, &root.0);
    assert!(!renamed.ids.contains_key("web") && renamed.ids.contains_key("renamed"));
    fs::remove_dir_all(root.0.join("sessions/renamed")).expect("remove session");
    assert!(
        !same_fleet(&mut subject, &root.0)
            .ids
            .contains_key("renamed")
    );
    fs::write(
        api.join("meta"),
        format!(
            "schema=2\nsession_id={}\nseat.main=new-lead\n",
            ID.replace("1234", "bbbb")
        ),
    )
    .expect("replace identity/pair");
    let replaced = same_fleet(&mut subject, &root.0);
    assert_eq!(replaced.ids["api"], ID.replace("1234", "bbbb"));
    same_view(&mut subject, &api, "api");
    fs::remove_dir_all(root.0.join("sessions")).expect("remove inventory");
    fs::write(root.0.join("sessions"), b"not a directory").expect("unreadable inventory");
    assert!(!same_fleet(&mut subject, &root.0).scanned);
    fs::remove_file(root.0.join("sessions")).expect("restore inventory");
    seed(&root, "api");
    assert!(same_fleet(&mut subject, &root.0).scanned);
}

pub(super) fn reader_costs<R: ReaderFiles>(make: impl Fn(&Path) -> R) {
    let root = Root::new("fleet-spec-reader-cost");
    let api = seed(&root, "api");
    for index in 0..24 {
        seed(&root, &format!("session{index:02}"));
    }
    let mut servers = Servers(Vec::new());
    servers.start(&root, "cost.sock", "api", true);
    fs::write(api.join("meta"), format!("schema=2\nsession_id={ID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\ntmux_server_kind=socket\ntmux_server={}\n", root.0.join("cost.sock").display())).expect("live cost fixture");
    append(&api.join("events.jsonl"), b"{\"ts\":\"2026-01-01T00:00:00Z\",\"actor\":\"lead\",\"action\":\"state\",\"ref\":\"blocked\"}\n");
    let mut subject = make(&root.0);
    assert!(
        same_fleet(&mut subject, &root.0).needs.contains_key("api"),
        "live attention fixture exercises needs reuse"
    );
    same_view(&mut subject, &api, "api");
    let warm = subject.costs();
    assert!(warm.0 > 0 && warm.1 > 0, "actual reader cold parses");
    same_fleet(&mut subject, &root.0);
    same_view(&mut subject, &api, "api");
    assert_eq!(
        subject.costs(),
        warm,
        "idle REAL fleet + view + memo paths parse zero bytes"
    );
    append(&api.join("events.jsonl"), &event("changed live journal"));
    same_fleet(&mut subject, &root.0);
    same_view(&mut subject, &api, "api");
    assert_eq!(
        subject.costs(),
        (warm.0 + length(&api.join("events.jsonl")), warm.1),
        "changed REAL fleet + needs + view parse journal once"
    );
    let removed = root.0.join("sessions/session00");
    assert!(subject.cached_dirs().contains(&removed));
    fs::remove_dir_all(&removed).expect("remove cached session");
    same_fleet(&mut subject, &root.0);
    assert!(
        !subject.cached_dirs().contains(&removed),
        "fleet removal evicts cache"
    );
    let before = subject.costs();
    seed(&root, "session00");
    same_fleet(&mut subject, &root.0);
    assert!(
        subject.costs().0 > before.0 && subject.costs().1 > before.1,
        "re-entry reads cold"
    );
}

struct Servers(Vec<(ServerId, String)>);
impl Servers {
    fn start(&mut self, root: &Root, socket: &str, name: &str, marked: bool) -> ServerId {
        let server = ServerId::Selected(crate::meta::Selector::Socket(root.0.join(socket)));
        let (ok, _) = crate::transport::run_tmux_op(&crate::session_tmux::argv(
            &server,
            &crate::session_tmux::Op::NewSession {
                name,
                work_dir: root.0.to_str().expect("fixture path"),
            },
        ));
        assert!(ok, "private server/session {name}");
        self.0.push((
            server.clone(),
            crate::transport::observe_session_id(&server, name).expect("private session id"),
        ));
        if marked {
            let (ok, _) = crate::transport::run_tmux_op(&crate::session_tmux::argv(
                &server,
                &crate::session_tmux::Op::SetEnv {
                    session: name,
                    key: "AE_SESSION",
                    value: name,
                },
            ));
            assert!(ok, "fixture ownership marker");
        }
        server
    }
}
impl Drop for Servers {
    fn drop(&mut self) {
        for (server, id) in &self.0 {
            let _ = crate::transport::kill_session(server, id);
        }
    }
}

pub(super) fn reader_servers<R: ReaderFiles>(make: impl Fn(&Path) -> R) {
    let root = Root::new("fleet-spec-servers");
    let api = seed(&root, "api");
    let mut servers = Servers(Vec::new());
    servers.start(&root, "one.sock", "keeper", false);
    let one = servers.start(&root, "one.sock", "api", true);
    let two = servers.start(&root, "second.sock", "keeper", false);
    append(&api.join("events.jsonl"), b"{\"ts\":\"2026-01-01T00:00:00Z\",\"actor\":\"lead\",\"action\":\"state\",\"ref\":\"blocked\"}\n");
    let meta_on = |socket: &str| {
        fs::write(api.join("meta"), format!("schema=2\nsession_id={ID}\nseat.main=lead\ntmux_server_kind=socket\ntmux_server={}\n", root.0.join(socket).display())).expect("server meta");
    };
    meta_on("one.sock");
    let mut subject = make(&root.0);
    let running = same_fleet(&mut subject, &root.0);
    assert!(
        running.needs.contains_key("api"),
        "live verdict supplies needs"
    );
    assert_eq!(
        running
            .world
            .sessions
            .iter()
            .find(|entry| entry.name == "api")
            .expect("api")
            .status,
        crate::digest::Status::Running
    );
    meta_on("second.sock");
    let stopped = same_fleet(&mut subject, &root.0);
    assert_eq!(
        stopped
            .world
            .sessions
            .iter()
            .find(|entry| entry.name == "api")
            .expect("api")
            .status,
        crate::digest::Status::Stopped,
        "changed recorded server is observed now"
    );
    servers.start(&root, "second.sock", "api", true);
    let live = same_fleet(&mut subject, &root.0);
    assert_eq!(
        live.world
            .sessions
            .iter()
            .find(|entry| entry.name == "api")
            .expect("api")
            .status,
        crate::digest::Status::Running,
        "server changes are never cached across reads"
    );
    assert_ne!(one, two);
}

#[derive(Default)]
struct BaselineFiles {
    journal: u64,
    memo: u64,
}
impl Files for BaselineFiles {
    fn snapshot(&mut self, dir: &Path) -> RecordSnapshot {
        let read = RecordSnapshot::read(dir);
        if read.events.is_some() {
            self.journal = self
                .journal
                .saturating_add(length(&dir.join("events.jsonl")));
        }
        read
    }
    fn memo(&mut self, dir: &Path) -> io::Result<Vec<Filed>> {
        let bytes = crate::store::open(dir).memo_bytes()?;
        self.memo = self.memo.saturating_add(bytes.len() as u64);
        Ok(brief::filed(&bytes))
    }
    fn costs(&self) -> (u64, u64) {
        (self.journal, self.memo)
    }
    fn retain(&mut self, _: &[PathBuf]) {}
    fn cached_dirs(&self) -> BTreeSet<PathBuf> {
        BTreeSet::new()
    }
}

struct BaselineReader {
    root: PathBuf,
    reader: Reader,
    journal: u64,
    memo: u64,
}
impl BaselineReader {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            reader: Reader::new(root.to_path_buf(), None, None),
            journal: 0,
            memo: 0,
        }
    }
}
impl ReaderFiles for BaselineReader {
    fn fleet(&mut self) -> FleetRead {
        let read = self
            .reader
            .fleet()
            .into_iter()
            .find_map(|answer| {
                if let Answer::Fleet(read) = answer {
                    Some(read)
                } else {
                    None
                }
            })
            .expect("real fleet answer");
        // Baseline fleet's full scan parses every journal, and needs_of parses
        // each attention session again. This adapter has no caching behaviour.
        for (name, dir) in &read.dirs {
            let bytes = length(&dir.join("events.jsonl"));
            self.journal = self.journal.saturating_add(bytes);
            if read.needs.contains_key(name) {
                self.journal = self.journal.saturating_add(bytes);
            }
            self.memo = self.memo.saturating_add(length(&dir.join("memo.tsv")));
        }
        read
    }
    fn view(&mut self, name: &str) -> Option<ViewRead> {
        let answer = self.reader.view(name)?;
        let Answer::View(read) = answer else {
            panic!("view answer")
        };
        self.journal = self
            .journal
            .saturating_add(length(&self.reader_root_dir(name).join("events.jsonl")));
        Some(read)
    }
    fn costs(&self) -> (u64, u64) {
        (self.journal, self.memo)
    }
    fn cached_dirs(&self) -> BTreeSet<PathBuf> {
        BTreeSet::new()
    }
}
impl BaselineReader {
    fn reader_root_dir(&self, name: &str) -> PathBuf {
        self.root.join("sessions").join(name)
    }
}

#[test]
fn baseline_file_changes_match_full_reads() {
    file_changes(BaselineFiles::default);
}
#[test]
fn baseline_file_identity_matches_full_reads() {
    file_identity(BaselineFiles::default);
}
#[test]
fn baseline_file_shapes_match_full_reads() {
    file_shapes(BaselineFiles::default);
}
#[test]
fn baseline_reader_changes_match_full_reads() {
    reader_changes(BaselineReader::new);
}
#[test]
fn baseline_reader_server_changes_match_full_reads() {
    reader_servers(BaselineReader::new);
}

// Deliberately ignored baseline witnesses: the uncached reader must FAIL.
// Run with nextest --run-ignored only; optimized adapters run these normally.
#[test]
#[ignore = "RED witness: baseline reparses unchanged files"]
fn baseline_cost_witness() {
    file_costs(BaselineFiles::default);
}
#[test]
#[ignore = "RED witness: baseline holds no cache"]
fn baseline_eviction_witness() {
    file_eviction(BaselineFiles::default);
}
#[test]
#[ignore = "RED witness: actual baseline Reader reparses idle fleet/view"]
fn baseline_reader_cost_witness() {
    reader_costs(BaselineReader::new);
}
