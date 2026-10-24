//! Frozen acceptance (#248 P1b-2 TRANSACT, Batch A GUARDS): every successful
//! `store::lock` still names the inode its path names; the console's draft
//! mutations and the Telegram cursor's checkpoint each take one short exact
//! guard (a 5 s writer wait); a recovery record never writes into a same-name
//! replacement of its session; and the two new guard names are
//! registration-only while every lookalike stays retained.
//!
//! Oracle: `plan-p1b2.md` r3 lines 9-12 and 24, `p1b2-locks-r3.md` with its
//! frozen r2 table, lead ACK 0d071dfb (I3 identity rule, I4 view narrowing).
//! Real kernel locks, real `_end` on a private tmux socket. API binding: the
//! instance pins call `ae::store::DraftRecord::{open, publish, clear,
//! recover}`, today `pub(crate)`.
//!
//! Shared fixture helpers live here, `pub(super)`, for the Batch B spec.

use std::collections::BTreeMap;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::time::Instant;

use super::*;

/// Relative path -> (kind, permission bits, bytes or link text).
pub(super) type Tree = BTreeMap<PathBuf, (char, u32, Vec<u8>)>;

/// Every node under `root`, by lstat; a link is never followed.
pub(super) fn tree(root: &Path) -> Tree {
    let mut out = Tree::new();
    let mut todo = vec![PathBuf::new()];
    while let Some(rel) = todo.pop() {
        let here = root.join(&rel);
        let entries =
            std::fs::read_dir(&here).unwrap_or_else(|why| panic!("{}: {why}", here.display()));
        for entry in entries {
            let path = rel.join(entry.expect("an entry").file_name());
            let full = root.join(&path);
            let meta = std::fs::symlink_metadata(&full).expect("lstat");
            let mode = meta.permissions().mode() & 0o7777;
            let row = if meta.file_type().is_symlink() {
                let text = std::fs::read_link(&full).expect("readlink");
                ('l', mode, text.into_os_string().into_vec())
            } else if meta.is_dir() {
                todo.push(path.clone());
                ('d', mode, Vec::new())
            } else if meta.is_file() {
                let bytes =
                    std::fs::read(&full).unwrap_or_else(|why| panic!("{}: {why}", full.display()));
                ('f', mode, bytes)
            } else {
                ('o', mode, Vec::new())
            };
            out.insert(path, row);
        }
    }
    out
}

pub(super) fn put(dir: &Path, rel: &str, bytes: &[u8]) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("parent dirs");
    std::fs::write(&path, bytes).unwrap_or_else(|why| panic!("{rel}: {why}"));
}

pub(super) fn end(rig: &Rig) -> (Option<i32>, String, String) {
    rig.run_with_env(
        &[
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ],
        &["_end", "-f", &rig.name],
    )
}

/// A second session keeps the isolated server answering once the target stops.
pub(super) fn keepalive(rig: &Rig) {
    assert!(
        rig.tmux(&["new-session", "-d", "-s", "keepalive", "sh"]).0,
        "a second session keeps the server answering"
    );
}

/// A standing tombstone makes the teardown refuse AFTER the archive is published.
pub(super) fn obstruct_teardown(rig: &Rig) -> PathBuf {
    let tomb = rig
        .home
        .join("sessions")
        .join(format!(".ending.{}", rig.name));
    std::fs::write(&tomb, b"obstruction").expect("a standing tombstone");
    tomb
}

/// Publish the archive, let the teardown refuse, and return the archive bytes.
pub(super) fn half_ended(rig: &Rig) -> Tree {
    keepalive(rig);
    put(&rig.dir, "notes/a.txt", b"retained once");
    put(&rig.dir, "memo.tsv", b"x\tremember\n");
    put(&rig.dir, "messages/a.txt", b"a body");
    let tomb = obstruct_teardown(rig);
    let (code, out, err) = end(rig);
    assert_eq!(code, Some(1), "the teardown refused: {out}{err}");
    assert!(exists(&rig.dir.join("meta")), "the registration is kept");
    let first = tree(&rig.archive());
    std::fs::remove_file(&tomb).expect("the obstruction cleared");
    // A later archived_at is a comparison substitution, never drift.
    std::thread::sleep(Duration::from_millis(1100));
    first
}

pub(super) const MUTATION: &str = ".console-draft-mutation.lock";
pub(super) const CURSOR_LOCK: &str = "telegram-outbound.cursor.lock";
pub(super) const INSTANCE: &str = "0199c0de-dddd-4890-abcd-ef0123456789";
const OTHER: &str = "0199c0de-eeee-4890-abcd-ef0123456789";

/// Take `path` now; a thread drops it after `held`.
pub(super) fn hold_for(path: &Path, held: Duration) -> std::thread::JoinHandle<()> {
    let lock = ae::store::lock(path, Duration::ZERO).expect("the fixture takes the lock");
    std::thread::spawn(move || {
        std::thread::sleep(held);
        drop(lock);
    })
}

fn timed<T>(act: impl FnOnce() -> T) -> (T, Duration) {
    let started = Instant::now();
    let done = act();
    (done, started.elapsed())
}

/// A guard held throughout: the writer waited its bounded wait, then failed.
fn waited_then_refused<T: std::fmt::Debug>(what: &str, got: (std::io::Result<T>, Duration)) {
    let (got, took) = got;
    assert!(got.is_err(), "{what}: a held guard refuses: {got:?}");
    assert!(
        took >= Duration::from_secs(4) && took < Duration::from_secs(30),
        "{what}: the writer waits ~{:?}, never zero or forever: {took:?}",
        ae::store::LOCK_WAIT
    );
}

/// How many of this process's open descriptors name inode `ino`.
fn open_on(ino: u64) -> usize {
    std::fs::read_dir("/dev/fd")
        .expect("/dev/fd")
        .filter_map(Result::ok)
        .filter(|entry| std::fs::metadata(entry.path()).is_ok_and(|m| m.ino() == ino))
        .count()
}

/// A writer queued on a lock whose path is unregistered (and, with
/// `replace`, recreated under the same name) before the holder lets go.
fn queued(tag: &str, replace: bool) {
    let scratch = OwnedScratch::root("lcg", tag);
    let path = scratch.path().join("meta.lock");
    let holder = ae::store::lock(&path, Duration::ZERO).expect("the first writer");
    let old = std::fs::metadata(&path).expect("stat").ino();
    let wanted = path.clone();
    let waiter = std::thread::spawn(move || ae::store::lock(&wanted, ae::store::LOCK_WAIT));
    let mut opened = false;
    for _ in 0..400 {
        if open_on(old) >= 2 {
            opened = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(opened, "the waiter opened the old inode and is queued");
    std::fs::remove_file(&path).expect("unregistered");
    if replace {
        std::fs::write(&path, b"").expect("a same-name replacement");
    }
    let new = replace.then(|| std::fs::metadata(&path).expect("stat").ino());
    drop(holder);
    let got = waiter.join().expect("the waiter");
    let why = got.expect_err("a lock whose path no longer names its inode is refused");
    assert!(
        why.to_string().contains(&path.display().to_string()),
        "the refusal names the path: {why}"
    );
    if let Some(new) = new {
        let meta = std::fs::symlink_metadata(&path).expect("the replacement stays");
        assert_eq!((meta.ino(), meta.len()), (new, 0), "nothing written there");
        ae::store::lock(&path, Duration::ZERO).expect("the replacement itself locks");
    } else {
        assert!(!exists(&path), "never reopened into a new incarnation");
    }
}

#[test]
fn guard_an_uncontended_or_briefly_contended_lock_is_unchanged() {
    let scratch = OwnedScratch::root("lcg", "a1");
    let path = scratch.path().join("memo.tsv.lock");
    let held = ae::store::lock(&path, Duration::ZERO).expect("an uncontended lock");
    assert!(std::fs::symlink_metadata(&path).expect("created").is_file());
    drop(held);
    let release = hold_for(&path, Duration::from_millis(800));
    let (got, took) = timed(|| ae::store::lock(&path, ae::store::LOCK_WAIT));
    got.expect("a contended lock is granted once its holder lets go");
    assert!(took >= Duration::from_millis(500), "it waited: {took:?}");
    release.join().expect("the holder");
}

#[test]
fn guard_a_queued_lock_across_a_same_name_replacement_refuses_naming_the_path() {
    queued("a2", true);
}

#[test]
fn guard_a_queued_lock_whose_path_vanished_refuses_and_recreates_nothing() {
    queued("a3", false);
}

#[test]
fn guard_the_two_new_guard_names_are_registration_only_and_lookalikes_are_retained() {
    let rig = Rig::new("gd1");
    put(&rig.dir, MUTATION, b"");
    put(&rig.dir, CURSOR_LOCK, b"");
    let lookalikes: &[(&str, &[u8])] = &[
        ("console-draft-mutation.lock", b""),
        (".console-draft-mutation.locks", b"m"),
        ("telegram-outbound.cursor.lock.bak", b""),
        ("other-outbound.cursor.lock", b"c"),
    ];
    let mut want = Tree::new();
    for (rel, bytes) in lookalikes {
        put(&rig.dir, rel, bytes);
        want.insert(PathBuf::from(rel), ('f', 0o600, bytes.to_vec()));
    }
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(!exists(&rig.dir), "the live state is gone");
    assert_eq!(
        tree(&rig.archive().join("retained")),
        want,
        "exactly the two exact guard names are dropped; no suffix broadening"
    );
}

#[test]
fn guard_an_older_archive_retaining_a_reserved_name_refuses_the_retry_and_keeps_its_bytes() {
    let rig = Rig::new("gd2");
    half_ended(&rig);
    let retained = rig.archive().join("retained");
    for name in [MUTATION, CURSOR_LOCK] {
        // What a P1b-1 core captured: these names were user data then.
        put(&retained, name, b"older bytes");
        std::fs::set_permissions(retained.join(name), std::fs::Permissions::from_mode(0o600))
            .expect("inert");
        put(&rig.dir, name, b"older bytes");
    }
    let first = tree(&rig.archive());
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(
        err.contains(&rig.archive().display().to_string())
            && err.contains(&rig.dir.display().to_string()),
        "the refusal names the archive and the live dir: {err}"
    );
    assert_eq!(tree(&rig.archive()), first, "the archive is exact");
    for name in [MUTATION, CURSOR_LOCK] {
        assert_eq!(
            std::fs::read(rig.dir.join(name)).expect("the live bytes"),
            b"older bytes",
            "{name}: never discarded"
        );
    }
}

#[test]
fn guard_a_held_mutation_guard_stops_legacy_draft_publish_and_clear() {
    let scratch = OwnedScratch::root("lcg", "a4");
    let dir = scratch.path();
    let store = ae::store::open(dir);
    store.publish_console_draft(b"before").expect("free guard");
    let guard = ae::store::lock(&dir.join(MUTATION), Duration::ZERO).expect("held");
    waited_then_refused("publish", timed(|| store.publish_console_draft(b"during")));
    waited_then_refused("clear", timed(|| store.clear_console_draft()));
    let legacy = dir.join("console.draft");
    assert_eq!(std::fs::read(&legacy).expect("kept"), b"before");
    drop(guard);
    let release = hold_for(&dir.join(MUTATION), Duration::from_secs(1));
    store
        .publish_console_draft(b"after")
        .expect("a guard released within the wait is waited for");
    release.join().expect("the holder");
    assert_eq!(std::fs::read(&legacy).expect("published"), b"after");
}

#[test]
fn guard_a_held_mutation_guard_stops_instance_open_publish_clear_and_recovery() {
    let scratch = OwnedScratch::root("lcg", "a5");
    let dir = scratch.path();
    let mine = ae::store::DraftRecord::open(dir, INSTANCE).expect("an instance");
    mine.publish(b"mine").expect("free guard");
    let idle = ae::store::DraftRecord::open(dir, OTHER).expect("a second, idle instance");
    put(dir, "console.draft", b"an orphan");
    let own = dir.join(format!("console.draft.{INSTANCE}"));
    let guard = ae::store::lock(&dir.join(MUTATION), Duration::ZERO).expect("held");
    std::thread::scope(|scope| {
        let publish = scope.spawn(|| timed(|| mine.publish(b"later")));
        let clear = scope.spawn(|| timed(|| mine.clear()));
        let recover = scope.spawn(|| idle.recover());
        let third = "0199c0de-ffff-4890-abcd-ef0123456789";
        let open = scope.spawn(|| timed(|| ae::store::DraftRecord::open(dir, third)));
        waited_then_refused("publish", publish.join().expect("publish"));
        waited_then_refused("clear", clear.join().expect("clear"));
        waited_then_refused("open", open.join().expect("open"));
        let _ = recover.join().expect("recover");
    });
    assert_eq!(std::fs::read(&own).expect("own kept"), b"mine");
    assert_eq!(
        std::fs::read(dir.join("console.draft")).expect("the orphan stays"),
        b"an orphan",
        "recovery moves nothing while the guard is held"
    );
    assert!(!exists(&dir.join(format!("console.draft.{OTHER}"))));
    drop(guard);
    mine.clear().expect("a free guard clears");
    assert!(!exists(&own));
}

/// One record bound to its session, then `replace` swaps one identity half:
/// every guarded mutation must refuse, and a fresh record works there.
fn stale_record(tag: &str, replace: impl FnOnce(&Path, &Path)) {
    let scratch = OwnedScratch::root("lcg", tag);
    let dir = scratch.path().join("sessions").join("lcstale");
    let meta = |uuid: &str| format!("session=lcstale\nsession_id={uuid}\n");
    put(&dir, "meta", meta(UUID).as_bytes());
    let own = dir.join(format!("console.draft.{INSTANCE}"));
    let record = ae::store::DraftRecord::open(&dir, INSTANCE).expect("an instance");
    record
        .publish(b"first")
        .expect("the bound record publishes");
    assert_eq!(std::fs::read(&own).expect("published"), b"first");
    replace(&dir, scratch.path());
    let _ = std::fs::remove_file(&own);
    put(&dir, "console.draft", b"theirs");
    assert!(
        record.publish(b"stale").is_err(),
        "{tag}: a stale record refuses"
    );
    assert!(
        !exists(&own),
        "{tag}: nothing published into the replacement"
    );
    let _ = record.recover();
    assert!(
        !exists(&own),
        "{tag}: nothing recovered into the replacement"
    );
    assert_eq!(
        std::fs::read(dir.join("console.draft")).expect("kept"),
        b"theirs",
        "{tag}: the orphan stays"
    );
    put(&dir, &format!("console.draft.{INSTANCE}"), b"theirs too");
    let _ = record.clear();
    assert_eq!(
        std::fs::read(&own).expect("never cleared"),
        b"theirs too",
        "{tag}: a stale record clears nothing in the replacement"
    );
    // The stale record still owns its lifetime; only its release frees it.
    drop(record);
    let fresh = ae::store::DraftRecord::open(&dir, INSTANCE).expect("a fresh record");
    fresh
        .publish(b"fresh")
        .expect("a record bound to the replacement publishes");
    assert_eq!(std::fs::read(&own).expect("fresh"), b"fresh");
}

#[test]
fn guard_a_record_refuses_a_new_directory_carrying_its_session_uuid() {
    stale_record("a6u", |dir, root| {
        std::fs::rename(dir, root.join("gone")).expect("unregistered");
        put(
            dir,
            "meta",
            format!("session=lcstale\nsession_id={UUID}\n").as_bytes(),
        );
    });
}

#[test]
fn guard_a_record_refuses_its_own_directory_once_the_uuid_is_rewritten() {
    stale_record("a6d", |dir, _| {
        let before = std::fs::metadata(dir).expect("the dir").ino();
        let replacement = "44444444-4444-4444-4444-444444444444";
        std::fs::write(
            dir.join("meta"),
            format!("session=lcstale\nsession_id={replacement}\n"),
        )
        .expect("rewritten in place");
        assert_eq!(std::fs::metadata(dir).expect("the dir").ino(), before);
    });
}

#[test]
fn guard_a_held_cursor_guard_stops_the_cursor_checkpoint() {
    let scratch = OwnedScratch::root("lcg", "a7");
    let path = scratch.path().join(ae::telegram::CURSOR_FILE);
    let before = ae::telegram::Cursor {
        inode: 7,
        offset: 1,
    };
    ae::telegram::store_cursor(&path, &before).expect("free guard");
    let lock = scratch.path().join(CURSOR_LOCK);
    let guard = ae::store::lock(&lock, Duration::ZERO).expect("held");
    let later = ae::telegram::Cursor {
        inode: 7,
        offset: 2,
    };
    let (got, took) = timed(|| ae::telegram::store_cursor(&path, &later));
    assert!(got.is_err(), "a held cursor guard refuses: {got:?}");
    assert!(
        took >= Duration::from_secs(4) && took < Duration::from_secs(30),
        "bounded wait: {took:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("kept"),
        before.render()
    );
    drop(guard);
    let release = hold_for(&lock, Duration::from_secs(1));
    ae::telegram::store_cursor(&path, &later).expect("waited for");
    release.join().expect("the holder");
    assert_eq!(
        std::fs::read_to_string(&path).expect("stored"),
        later.render()
    );
}
