//! `ae archive preview` — the read-only tracer, black-box (P3.1).
//!
//! Byte-parity against the frozen bash on ordinary, empty and
//! malformed-but-readable sessions, plus the two guarantees a read tracer must
//! keep: it writes NOTHING, and it never blocks or follows a non-regular file
//! in place of a source. The expected outputs under
//! `tests/fixtures/archive-preview/<case>/` were captured from the frozen
//! `ae archive preview`; `<DIR>` in the expected stderr stands for the session
//! directory this run built.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "fixtures build and inspect real directories with expect on the fixture \
              I/O; the capability boundary is about what PRODUCT code may reach"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn cases_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/archive-preview")
}

/// A temp dir removed on drop. Its own module so a panicking test still cleans
/// up (Drop runs while unwinding).
struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae-arprev-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Recursively copy a case's `session/` tree into `dest`.
fn copy_tree(src: &Path, dest: &Path) {
    std::fs::create_dir_all(dest).expect("mkdir dest");
    for entry in std::fs::read_dir(src).expect("read_dir src").flatten() {
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type().expect("file_type").is_dir() {
            copy_tree(&from, &to);
        } else {
            std::fs::copy(&from, &to).expect("copy file");
        }
    }
}

/// The session NAME the banner reports is the session directory's basename, so
/// each case is installed into a dir named after its own `meta`'s `session=` —
/// the name the frozen capture ran under (demo / fresh / odd).
fn session_name(case: &str) -> String {
    let meta = std::fs::read_to_string(cases_root().join(case).join("session/meta"))
        .expect("fixture meta");
    meta.lines()
        .find_map(|l| l.strip_prefix("session="))
        .expect("a session= in the fixture meta")
        .to_owned()
}

fn install(case: &str, scratch: &Scratch) -> PathBuf {
    let dir = scratch.path().join(session_name(case));
    copy_tree(&cases_root().join(case).join("session"), &dir);
    dir
}

fn read_fixture(case: &str, name: &str) -> Vec<u8> {
    std::fs::read(cases_root().join(case).join(name))
        .unwrap_or_else(|e| panic!("fixture {case}/{name}: {e}"))
}

/// Every regular file under `root` → its size, for the no-write proof.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, u64> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let meta = entry.metadata().expect("metadata");
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                out.insert(path.strip_prefix(root).unwrap().to_owned(), meta.len());
            }
        }
    }
    out
}

/// Run the preview, bounded: a subject that could hang must fail with a red
/// that ARRIVES, not one that stalls the lane. `None` if it had to be killed.
fn bounded_preview(dir: &Path) -> Option<std::process::Output> {
    let child = crate::cli::ae()
        .arg("_archive-preview")
        .arg(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    crate::cli::bounded(child, Duration::from_secs(10))
}

#[test]
fn the_preview_is_byte_identical_to_the_frozen_digest_on_every_readable_shape() {
    let scratch = Scratch::new("parity");
    for case in ["ordinary", "empty", "malformed"] {
        let dir = install(case, &scratch);
        let out = bounded_preview(&dir).expect("preview returned");
        let want_rc: i32 = String::from_utf8_lossy(&read_fixture(case, "rc"))
            .trim()
            .parse()
            .expect("rc");
        assert_eq!(out.status.code(), Some(want_rc), "{case} rc");
        assert_eq!(
            out.stdout,
            read_fixture(case, "expected.stdout"),
            "{case} stdout"
        );
        let stderr =
            String::from_utf8_lossy(&out.stderr).replace(&dir.display().to_string(), "<DIR>");
        assert_eq!(
            stderr.as_bytes(),
            read_fixture(case, "expected.stderr"),
            "{case} stderr"
        );
    }
}

#[test]
fn the_preview_writes_nothing() {
    let scratch = Scratch::new("nowrite");
    for case in ["ordinary", "empty", "malformed"] {
        let dir = install(case, &scratch);
        let before = snapshot(&dir);
        let out = bounded_preview(&dir).expect("preview returned");
        assert!(matches!(out.status.code(), Some(0 | 1)), "{case}");
        assert_eq!(
            snapshot(&dir),
            before,
            "{case}: the tracer wrote into the session"
        );
    }
}

#[test]
fn a_non_regular_event_container_is_a_named_refusal_and_never_blocks() {
    // A preview must not leave its session directory to render linked or
    // special-node bytes (colead ruling, P3.1). events.jsonl replaced by, in
    // turn: a FIFO (an ungated open would block on it forever), a directory, a
    // symlink to a REGULAR file (the escape the follow variants would render), a
    // symlink to a directory, and a symlink to a FIFO. Each must return promptly
    // (never block), refuse by name at rc=1 with NO digest on stdout, and write
    // nothing. This is the intentional divergence from the frozen `[[ -f ]]`,
    // which follows the symlink-to-regular and treats the FIFO/dir as absent.
    let scratch = Scratch::new("hostile");
    for kind in [
        "fifo",
        "dir",
        "symlink-regular",
        "symlink-dir",
        "symlink-fifo",
    ] {
        let dir = scratch
            .path()
            .join(format!("{}-{kind}", session_name("ordinary")));
        copy_tree(&cases_root().join("ordinary").join("session"), &dir);
        let events = dir.join("events.jsonl");
        std::fs::remove_file(&events).expect("remove events");
        match kind {
            "fifo" => crate::cli::mkfifo(&events),
            "dir" => std::fs::create_dir(&events).expect("mkdir events"),
            "symlink-regular" => {
                let target = dir.join("a-regular.jsonl");
                std::fs::write(&target, b"{\"action\":\"state\"}\n").expect("write target");
                std::os::unix::fs::symlink(&target, &events).expect("symlink");
            }
            "symlink-dir" => {
                let target = dir.join("a-dir");
                std::fs::create_dir(&target).expect("mkdir target");
                std::os::unix::fs::symlink(&target, &events).expect("symlink");
            }
            "symlink-fifo" => {
                let target = dir.join("a-fifo");
                crate::cli::mkfifo(&target);
                std::os::unix::fs::symlink(&target, &events).expect("symlink");
            }
            _ => unreachable!(),
        }
        let before = snapshot(&dir);
        let out = bounded_preview(&dir)
            .unwrap_or_else(|| panic!("{kind}: the tracer BLOCKED on a non-regular source"));
        assert_eq!(out.status.code(), Some(1), "{kind}: refuses at 1");
        assert!(out.stdout.is_empty(), "{kind}: a refusal renders no digest");
        assert!(
            String::from_utf8_lossy(&out.stderr)
                .contains("has a non-regular events.jsonl — it cannot be archived."),
            "{kind}: {:?}",
            out.stderr
        );
        assert_eq!(snapshot(&dir), before, "{kind}: wrote into the session");
    }
}

#[test]
fn a_non_regular_meta_is_a_named_refusal_without_blocking() {
    // meta as a FIFO: a non-regular meta is refused BY NAME before the id read
    // would follow it (not misreported as a missing UUID), and the refusal must
    // not hang on the FIFO. `nonregular_existing` lstats, so it never opens it.
    let scratch = Scratch::new("metafifo");
    let dir = scratch.path().join(session_name("ordinary"));
    copy_tree(&cases_root().join("ordinary").join("session"), &dir);
    std::fs::remove_file(dir.join("meta")).expect("remove meta");
    crate::cli::mkfifo(&dir.join("meta"));
    // Snapshot AFTER the FIFO is in place — the tracer must add nothing to that.
    let before = snapshot(&dir);
    let out = bounded_preview(&dir).expect("the tracer BLOCKED on a non-regular meta");
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "a refusal renders no digest");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("has a non-regular meta — it cannot be archived."),
        "{:?}",
        out.stderr
    );
    assert_eq!(snapshot(&dir), before, "the tracer wrote into the session");
}

#[test]
fn a_roster_ae_cannot_parse_refuses_the_whole_preview() {
    // `_ar_build_meta` REFUSES a roster it cannot parse — a bad slot or a ref
    // that is not `alias:name[:session-id]` fails the whole preview (rc=1) with
    // the frozen line, then the composer's own refusal, and writes nothing. A
    // read tracer that rendered a plausible-but-wrong roster would be worse than
    // one that refuses. `<name>` in the second line is the session directory's
    // basename.
    let scratch = Scratch::new("refuse");
    let cases = [
        (
            "colon-less-ref",
            "agent.main=cl:lead:AE3AA692-E177-4798-9BA0-D14E0D084061",
            "agent.main=noColon",
            "archive: roster entry 'agent.main=noColon' is not alias:name[:session-id].",
        ),
        (
            "unrecognised-slot",
            "agent_bin.spawned.0=grok",
            "agent_bin.spawned.0=grok\nagent.bogus=x:y",
            "archive: meta carries an unrecognised roster slot 'agent.bogus'.",
        ),
        // A BARE `agent.main` record (no `=`) is not dropped: the frozen
        // `_ar_roster_slots` names the slot `main`, its ref reads empty, and
        // `_ar_build_meta` refuses it as `agent.main=`.
        (
            "bare-recognised-slot",
            "agent.main=cl:lead:AE3AA692-E177-4798-9BA0-D14E0D084061",
            "agent.main",
            "archive: roster entry 'agent.main=' is not alias:name[:session-id].",
        ),
        // A BARE record for an UNrecognised slot refuses on the slot grammar
        // first, before its (empty) ref is ever consulted.
        (
            "bare-unrecognised-slot",
            "agent_bin.spawned.0=grok",
            "agent_bin.spawned.0=grok\nagent.bogus",
            "archive: meta carries an unrecognised roster slot 'agent.bogus'.",
        ),
    ];
    for (tag, from, to, first_line) in cases {
        let dir = scratch.path().join(tag);
        copy_tree(&cases_root().join("ordinary").join("session"), &dir);
        let meta_path = dir.join("meta");
        let meta = std::fs::read_to_string(&meta_path).expect("read meta");
        assert!(
            meta.contains(from),
            "{tag}: fixture anchor '{from}' not found"
        );
        std::fs::write(&meta_path, meta.replace(from, to)).expect("rewrite meta");

        let before = snapshot(&dir);
        let out = bounded_preview(&dir)
            .unwrap_or_else(|| panic!("{tag}: the tracer BLOCKED on an unparsable roster"));
        assert_eq!(out.status.code(), Some(1), "{tag}: refuses at 1");
        assert!(out.stdout.is_empty(), "{tag}: a refusal renders no digest");
        let want = format!("{first_line}\nae: could not render a preview for '{tag}'.\n");
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            want,
            "{tag}: exact two-line refusal"
        );
        assert_eq!(snapshot(&dir), before, "{tag}: wrote into the session");
    }
}

#[test]
fn a_symlinked_regular_source_is_refused_not_followed() {
    // The safety core of the P3.1 ruling: a symlink to a REGULAR file — the one
    // shape the frozen `[[ -f ]]` would follow and render — is refused for meta
    // and for memo.tsv (events is covered by the hostile-container test). The
    // preview must not read a byte through the link, so: rc=1, a named refusal on
    // stderr, no digest on stdout, and nothing written into the session.
    let scratch = Scratch::new("symlink-refuse");
    for (file, needle) in [
        ("meta", "has a non-regular meta"),
        ("memo.tsv", "has a non-regular memo.tsv"),
    ] {
        let dir = scratch.path().join(file.replace('.', "-"));
        copy_tree(&cases_root().join("ordinary").join("session"), &dir);
        let node = dir.join(file);
        let _ = std::fs::remove_file(&node);
        let target = dir.join(format!("real-{}", file.replace('.', "-")));
        std::fs::write(&target, b"session=demo\n").expect("write link target");
        std::os::unix::fs::symlink(&target, &node).expect("symlink source");
        let before = snapshot(&dir);
        let out = bounded_preview(&dir).unwrap_or_else(|| panic!("{file}: BLOCKED on a symlink"));
        assert_eq!(out.status.code(), Some(1), "{file}: refuses at 1");
        assert!(out.stdout.is_empty(), "{file}: a refusal renders no digest");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(needle),
            "{file}: {:?}",
            out.stderr
        );
        assert_eq!(snapshot(&dir), before, "{file}: wrote into the session");
    }
}

/// A minimal local-mode session whose `meta` is written verbatim (so a test can
/// control the final newline). Events is an empty regular file.
fn minimal_session(scratch: &Scratch, tag: &str, meta: &str) -> PathBuf {
    let dir = scratch.path().join(tag);
    std::fs::create_dir_all(&dir).expect("mkdir session");
    std::fs::write(dir.join("meta"), meta).expect("write meta");
    std::fs::write(dir.join("events.jsonl"), b"").expect("write events");
    dir
}

#[test]
fn a_roster_record_on_the_final_unterminated_line_is_not_dropped() {
    // The frozen `_ar_roster_slots` is awk: it processes a final record with no
    // trailing newline. A meta whose LAST line is a valid `agent.main=…` (no LF)
    // must still render that slot; one whose last line is a bare `agent.main`
    // must refuse, not silently succeed with an empty roster. Expected outputs
    // captured from the frozen `ae archive preview`.
    const HEAD: &str = "session=nolf\nsession_id=e795c9e9-1111-2222-3333-444455556666\n\
                        session_id_origin=session\nmode=local\n";
    let scratch = Scratch::new("nolf");

    // Valid roster record on the final unterminated line -> rendered, rc 0.
    let valid = minimal_session(&scratch, "valid", &format!("{HEAD}agent.main=cl:lead"));
    let out = bounded_preview(&valid).expect("valid nolf preview");
    assert_eq!(out.status.code(), Some(0), "valid nolf rc");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("- main — cl:lead ("),
        "the final unterminated roster record was dropped: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    // Bare (keyless) record on the final unterminated line -> refuse, rc 1.
    let bare = minimal_session(&scratch, "bare", &format!("{HEAD}agent.main"));
    let out = bounded_preview(&bare).expect("bare nolf preview");
    assert_eq!(out.status.code(), Some(1), "bare nolf rc");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "archive: roster entry 'agent.main=' is not alias:name[:session-id].\n\
         ae: could not render a preview for 'bare'.\n",
        "bare nolf refusal"
    );
}
