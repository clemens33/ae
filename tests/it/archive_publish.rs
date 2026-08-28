//! `ae _archive-publish` — the archive publisher and its `.publishing.<uuid>`
//! claim (P3.3), black-box against the built binary.
//!
//! These drive the core the way `_end_archive_step` does — `_archive-publish
//! <session-dir> <push-outcome> <push-ref> <preserved> <workdir> <archived-at>`,
//! the session dir under an `<AE_HOME>/sessions/<name>` shape so the core derives
//! `<AE_HOME>/archive` from it — and assert the properties the assignment
//! requires: an exact payload with no executable bit, refusal of an existing
//! target and a standing claim, classified-source refusal, a validated timestamp,
//! a handled failure that cleans its own claim while leaving the source
//! untouched, and the `target\tfiles\tbytes` line a bash consumer reads back.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "fixtures build real sessions and read the published tree on disk; the \
              capability boundary is about what PRODUCT code may reach"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

const AID: &str = "e795c9e9-1111-2222-3333-444455556666";
const AT: &str = "2026-08-28T10:00:00Z";

/// A temp dir removed on drop, even while a test panics.
struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae-pub-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }
    /// `<AE_HOME>` — the sessions live under it, the archive root beside them.
    fn home(&self) -> &Path {
        &self.0
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build a session `<home>/sessions/<name>` with a roster, a memo, one pending
/// request event and its message body. Returns the session dir.
fn session(home: &Path, name: &str) -> PathBuf {
    let dir = home.join("sessions").join(name);
    std::fs::create_dir_all(dir.join("messages")).expect("mkdir session");
    std::fs::create_dir_all(home.join("archive")).expect("mkdir archive root");
    std::fs::write(
        dir.join("meta"),
        format!(
            "session={name}\n\
             session_id={AID}\n\
             session_id_origin=session\n\
             mode=worktree\n\
             origin=/some/origin\n\
             layout=vertical\n\
             goal=ship the archive\n\
             ae_version=0.2.11\n\
             parent_archive_id=-\n\
             agent.main=cl:lead:sid-xyz\n\
             agent_bin.main=claude\n"
        ),
    )
    .expect("write meta");
    std::fs::write(dir.join("memo.tsv"), "ts1\tcl:lead\thandover\tpicking up\n").expect("memo");
    std::fs::write(
        dir.join("events.jsonl"),
        "{\"ts\":\"2026-08-01T00:00:00Z\",\"actor\":\"cl:lead\",\"action\":\"ask\",\"ref\":\"r1\",\"summary\":\"a question\",\"body_file\":\"messages/r1.ask.txt\"}\n",
    )
    .expect("events");
    std::fs::write(dir.join("messages").join("r1.ask.txt"), "the body\n").expect("message");
    dir
}

/// Run `_archive-publish` against `dir` with the given operation facts.
fn publish(
    dir: &Path,
    push_outcome: &str,
    push_ref: &str,
    preserved: &str,
    workdir: &str,
    archived_at: &str,
) -> std::process::Output {
    let child = crate::cli::ae()
        .arg("_archive-publish")
        .arg(dir)
        .arg(push_outcome)
        .arg(push_ref)
        .arg(preserved)
        .arg(workdir)
        .arg(archived_at)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    crate::cli::bounded(child, Duration::from_secs(10)).expect("publish returned")
}

/// The common not-managed publish: no git story, `-` for every operation fact.
fn publish_unmanaged(dir: &Path) -> std::process::Output {
    publish(dir, "not-managed", "-", "-", "-", AT)
}

fn mode_of(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

/// A digest of the session dir's content — every file's relative path and bytes
/// (or `None` when unreadable) — to prove a failed publish left the source
/// byte-for-byte untouched. `.lock` files are excluded: acquiring a lock may
/// create one, which the assignment classes as lock infrastructure, not source.
fn source_snapshot(dir: &Path) -> Vec<(String, Option<Vec<u8>>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(p) = stack.pop() {
        let meta = std::fs::symlink_metadata(&p).unwrap();
        if meta.is_dir() {
            for e in std::fs::read_dir(&p).unwrap() {
                stack.push(e.unwrap().path());
            }
        } else if meta.is_file() {
            let rel = p.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
            if Path::new(&rel).extension().is_some_and(|e| e == "lock") {
                continue;
            }
            out.push((rel, std::fs::read(&p).ok()));
        }
    }
    out.sort();
    out
}

#[test]
fn publishes_a_complete_archive_and_prints_the_bash_diagnostic() {
    let scratch = Scratch::new("ok");
    let dir = session(scratch.home(), "demo");
    let out = publish_unmanaged(&dir);
    assert_eq!(out.status.code(), Some(0), "publish rc");

    let target = scratch.home().join("archive").join(AID);
    // The tab-separated line a bash consumer reads: <target>\t<files>\t<bytes>.
    let stdout = String::from_utf8(out.stdout).unwrap();
    let line = stdout.trim_end_matches('\n');
    let fields: Vec<&str> = line.split('\t').collect();
    assert_eq!(fields.len(), 3, "three tab-separated fields: {line:?}");
    assert_eq!(fields[0], target.to_string_lossy(), "field 1 is the target");
    assert_eq!(fields[1], "5", "five files");
    assert!(fields[2].parse::<u64>().is_ok(), "byte count parses");

    // The payload set and its modes.
    for (name, want) in [
        ("meta", 0o600),
        ("digest.md", 0o600),
        ("memo.tsv", 0o600),
        ("events.jsonl", 0o600),
        ("messages", 0o700),
        ("messages/r1.ask.txt", 0o600),
    ] {
        let p = target.join(name);
        assert!(p.exists(), "{name} present");
        assert_eq!(mode_of(&p), want, "{name} mode");
    }
    assert_eq!(mode_of(&target), 0o700, "archive dir mode");
    // No executable bit on any file.
    assert_eq!(mode_of(&target.join("meta")) & 0o111, 0, "no exec bit");
    // The composed meta keys the archive and names the source.
    let meta = std::fs::read_to_string(target.join("meta")).unwrap();
    assert!(meta.contains(&format!("archive_id={AID}")));
    assert!(meta.contains("archive_version=1"));
    assert!(meta.contains("source_session=demo"));
    assert!(meta.contains(&format!("archived_at={AT}")));
    // No claim was left behind.
    assert!(
        !scratch
            .home()
            .join("archive")
            .join(format!(".publishing.{AID}"))
            .exists(),
        "the claim was removed after a clean publish"
    );
}

#[test]
fn an_existing_target_is_refused_and_left_untouched() {
    let scratch = Scratch::new("exists");
    let dir = session(scratch.home(), "demo");
    let target = scratch.home().join("archive").join(AID);
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("sentinel"), b"do not touch").unwrap();

    let out = publish_unmanaged(&dir);
    assert_eq!(out.status.code(), Some(1), "refuses an existing target");
    assert!(String::from_utf8_lossy(&out.stderr).contains("already exists"));
    assert_eq!(
        std::fs::read(target.join("sentinel")).unwrap(),
        b"do not touch",
        "the existing archive was not merged into or overwritten"
    );
}

#[test]
fn a_standing_claim_is_refused_and_never_guess_cleaned() {
    let scratch = Scratch::new("claim");
    let dir = session(scratch.home(), "demo");
    // A crash left the claim standing: the next run must refuse and leave it.
    let claim = scratch
        .home()
        .join("archive")
        .join(format!(".publishing.{AID}"));
    std::fs::create_dir_all(claim.join("payload")).unwrap();
    std::fs::write(claim.join("payload").join("half"), b"partial").unwrap();

    let out = publish_unmanaged(&dir);
    assert_eq!(out.status.code(), Some(1), "refuses a standing claim");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("another publisher holds") || err.contains("crashed holding it"));
    assert!(
        claim.join("payload").join("half").exists(),
        "the standing claim was left exactly as found, not guess-cleaned"
    );
    assert!(
        !scratch.home().join("archive").join(AID).exists(),
        "nothing was published"
    );
}

#[test]
fn a_nonregular_core_source_is_refused() {
    let scratch = Scratch::new("nonreg");
    let dir = session(scratch.home(), "demo");
    // events.jsonl becomes a symlink: a classified source the publisher refuses.
    std::fs::remove_file(dir.join("events.jsonl")).unwrap();
    std::os::unix::fs::symlink("/etc/hosts", dir.join("events.jsonl")).unwrap();

    let out = publish_unmanaged(&dir);
    assert_eq!(out.status.code(), Some(1), "refuses a non-regular source");
    assert!(String::from_utf8_lossy(&out.stderr).contains("non-regular events.jsonl"));
    assert!(
        !scratch.home().join("archive").join(AID).exists(),
        "nothing published"
    );
}

#[test]
fn a_malformed_timestamp_is_refused_before_any_claim() {
    let scratch = Scratch::new("ts");
    let dir = session(scratch.home(), "demo");
    for bad in [
        "2026-08-28 10:00:00Z",
        "2026-8-28T10:00:00Z",
        "not-a-time",
        "2026-08-28T10:00:00Z\n",
    ] {
        let out = publish(&dir, "not-managed", "-", "-", "-", bad);
        assert_eq!(out.status.code(), Some(1), "refuses {bad:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("is not an ISO-8601 UTC instant"),
            "names the timestamp for {bad:?}"
        );
    }
    // No archive and no claim was created by any of the refusals.
    let archive = scratch.home().join("archive");
    let leftovers: Vec<_> = std::fs::read_dir(&archive).unwrap().flatten().collect();
    assert!(
        leftovers.is_empty(),
        "no target or claim from a rejected timestamp"
    );
}

#[test]
fn an_unreadable_message_cleans_its_claim_and_leaves_the_source_untouched() {
    let scratch = Scratch::new("unread");
    let dir = session(scratch.home(), "demo");
    // An eligible regular message we cannot read is an injected failure AFTER the
    // claim is taken: the publish must refuse, remove its OWN claim, and never
    // touch the live session.
    let msg = dir.join("messages").join("secret.txt");
    std::fs::write(&msg, b"top secret").unwrap();
    std::fs::set_permissions(&msg, std::fs::Permissions::from_mode(0o000)).unwrap();
    let before = source_snapshot(&dir);

    let out = publish_unmanaged(&dir);
    // Snapshot while the message is still 0o000 (unreadable → None in both), so
    // before == after proves the source was untouched, then restore for Drop.
    let after = source_snapshot(&dir);
    std::fs::set_permissions(&msg, std::fs::Permissions::from_mode(0o600)).unwrap();

    assert_eq!(out.status.code(), Some(1), "an unreadable message refuses");
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read messages/secret.txt"));
    assert!(
        !scratch.home().join("archive").join(AID).exists(),
        "nothing published"
    );
    assert!(
        !scratch
            .home()
            .join("archive")
            .join(format!(".publishing.{AID}"))
            .exists(),
        "the publisher removed its OWN claim on a handled failure"
    );
    assert_eq!(before, after, "the live session bytes were not touched");
}

#[test]
fn a_symlink_message_is_skipped_and_the_digest_reads_unavailable() {
    let scratch = Scratch::new("symmsg");
    let dir = session(scratch.home(), "demo");
    // The pending request's own body is a symlink: it must be skipped (never
    // followed) and its digest entry must read 'Payload: unavailable' from the
    // STAGED set — no dangling messages/<base> link is published.
    std::fs::remove_file(dir.join("messages").join("r1.ask.txt")).unwrap();
    std::os::unix::fs::symlink("/etc/hosts", dir.join("messages").join("r1.ask.txt")).unwrap();

    let out = publish_unmanaged(&dir);
    assert_eq!(out.status.code(), Some(0), "the archive still publishes");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("skipping messages/r1.ask.txt"),
        "the skip is loud"
    );
    let target = scratch.home().join("archive").join(AID);
    assert!(
        !target.join("messages").join("r1.ask.txt").exists(),
        "the symlink body was never followed into the archive"
    );
    let digest = std::fs::read_to_string(target.join("digest.md")).unwrap();
    assert!(
        digest.contains("Payload: unavailable"),
        "the digest names the skipped body as unavailable, not a dangling link"
    );
}

#[test]
fn a_fifo_core_source_is_refused_without_blocking() {
    let scratch = Scratch::new("fifo");
    let dir = session(scratch.home(), "demo");
    // meta is replaced by a FIFO. Classification must refuse it BEFORE any read,
    // so the publisher never opens it — a read would block indefinitely (no
    // writer) while all three source locks are held. The bounded runner proves
    // the process returns promptly instead of hanging.
    std::fs::remove_file(dir.join("meta")).unwrap();
    crate::cli::mkfifo(&dir.join("meta"));

    let out = publish_unmanaged(&dir);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a FIFO meta is refused, not read (a read would block)"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("non-regular meta"));
    assert!(
        !scratch.home().join("archive").join(AID).exists(),
        "nothing published"
    );
}

#[test]
fn a_symlinked_messages_dir_is_skipped_and_never_followed() {
    let scratch = Scratch::new("symdir");
    let dir = session(scratch.home(), "demo");
    // messages/ ITSELF is a symlink to an external directory holding a payload
    // that must NEVER be staged. `read_dir` would follow the link and copy the
    // external *.txt in; classification of the root refuses to follow it.
    let external = scratch.home().join("external-msgs");
    std::fs::create_dir_all(&external).unwrap();
    std::fs::write(external.join("leak.txt"), b"must not be archived").unwrap();
    std::fs::remove_dir_all(dir.join("messages")).unwrap();
    std::os::unix::fs::symlink(&external, dir.join("messages")).unwrap();

    let out = publish_unmanaged(&dir);
    assert_eq!(out.status.code(), Some(0), "the archive still publishes");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("skipping messages/"),
        "the skip of the symlinked dir is loud"
    );
    let target = scratch.home().join("archive").join(AID);
    assert!(
        !target.join("messages").join("leak.txt").exists(),
        "the external file behind the symlinked dir was never staged"
    );
    assert_eq!(
        std::fs::read(external.join("leak.txt")).unwrap(),
        b"must not be archived",
        "the external source was not touched"
    );
    let digest = std::fs::read_to_string(target.join("digest.md")).unwrap();
    assert!(
        digest.contains("Payload: unavailable"),
        "the referenced body reads unavailable from the empty staged set"
    );
}

#[test]
fn an_unreadable_core_ledger_refuses_rather_than_publish_empty() {
    let scratch = Scratch::new("unreadledger");
    let dir = session(scratch.home(), "demo");
    // memo.tsv is a REGULAR file that cannot be read: it passes the non-regular
    // gate but must REFUSE — an immutable archive must never publish with a
    // ledger silently emptied by an unwrap_or_default.
    std::fs::set_permissions(dir.join("memo.tsv"), std::fs::Permissions::from_mode(0o000)).unwrap();
    let before = source_snapshot(&dir);

    let out = publish_unmanaged(&dir);
    let after = source_snapshot(&dir);
    std::fs::set_permissions(dir.join("memo.tsv"), std::fs::Permissions::from_mode(0o600)).unwrap();

    assert_eq!(out.status.code(), Some(1), "an unreadable ledger refuses");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("cannot read") && err.contains("evidence dropped"),
        "names the refusal and its reason: {err}"
    );
    assert!(
        !scratch.home().join("archive").join(AID).exists(),
        "nothing published"
    );
    assert_eq!(before, after, "the live session bytes were not touched");
}

#[test]
fn an_unreadable_messages_dir_refuses_as_unknown_loss() {
    let scratch = Scratch::new("unreadmsgdir");
    let dir = session(scratch.home(), "demo");
    // messages/ is a REAL directory that cannot be enumerated (0o000): unknown
    // loss, not a classified absence. It must REFUSE rather than silently publish
    // an archive with no message bodies — the sharper boundary a symlinked/absent
    // root does not cross.
    let msgs = dir.join("messages");
    std::fs::set_permissions(&msgs, std::fs::Permissions::from_mode(0o000)).unwrap();

    let out = publish_unmanaged(&dir);
    std::fs::set_permissions(&msgs, std::fs::Permissions::from_mode(0o700)).unwrap();

    assert_eq!(
        out.status.code(),
        Some(1),
        "an unenumerable real dir refuses"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cannot enumerate messages/"),
        "names the enumeration failure"
    );
    assert!(
        !scratch.home().join("archive").join(AID).exists(),
        "nothing published"
    );
}
