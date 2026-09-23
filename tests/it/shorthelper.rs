//! `ae @<session> <helper> …`, black-box: the short spelling of a helper link.
//!
//! The subject is a route, so every test here runs the shipped binary. What is
//! being proven is that the marked spelling reaches the SAME helper, against
//! the session it names and no other, while carrying none of the things a
//! session directory cannot grant: not a launch, not a path out of the
//! sessions root, and above all not an identity.
//!
//! No tmux: the runner is pinned to a socket in a directory that does not
//! exist, so a route that fell through to a launch could not hide it.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "fixtures build and inspect real directories; the capability boundary is \
              about what PRODUCT code may reach"
)]

use std::fs;
use std::path::{Path, PathBuf};

use super::cli::{ae, helper};

/// A scratch state root with its `sessions/` directory, per test.
fn scratch(tag: &str) -> PathBuf {
    let dir = super::cli::OwnedScratch::root("sh", tag).keep();
    assert!(
        fs::create_dir_all(dir.join("sessions")).is_ok(),
        "a scratch state root"
    );
    dir
}

/// One session directory, the way a launch leaves it on disk.
fn plant(root: &Path, name: &str) -> PathBuf {
    let dir = root.join("sessions").join(name);
    assert!(fs::create_dir_all(&dir).is_ok(), "the session dir");
    dir
}

/// One helper LINK in `dir`, the way a launch publishes it.
fn link(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let _ = fs::remove_file(&path);
    assert!(
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), &path).is_ok(),
        "a {name} link"
    );
    path
}

/// The public `ae` over `root`, with NO pane identity at all.
fn run(root: &Path, argv: &[&str]) -> (Option<i32>, String, String) {
    let out = ae()
        .env("HOME", root)
        .env("AE_HOME", root)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .args(argv)
        .output()
        .expect("the ae binary should run");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The same tail through the session's own LINK — the spelling the short form
/// is a second spelling of.
fn through_link(root: &Path, path: &Path, argv: &[&str]) -> (Option<i32>, String, String) {
    let out = helper(path)
        .env("HOME", root)
        .env("AE_HOME", root)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .args(argv)
        .output()
        .expect("the helper link should run");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The names under `<root>/sessions`, sorted — the sweep every refusal owes.
fn sessions(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root.join("sessions"))
        .expect("the sessions root")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn the_short_form_runs_the_named_sessions_own_helper() {
    let root = scratch("round-trip");
    let demo = plant(&root, "demo");
    let other = plant(&root, "other");

    let (code, _, stderr) = run(&root, &["@demo", "memo", "add", "--topic", "t", "hello"]);
    assert_eq!(code, Some(0), "{stderr}");
    let (code, stdout, stderr) = run(&root, &["@demo", "memo", "read"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("hello"), "the memo reads back: {stdout}");

    // The TRANSLATED directory is the one that was named, and only it.
    assert!(demo.join("memo.tsv").exists(), "the memo landed in 'demo'");
    assert!(
        !other.join("memo.tsv").exists(),
        "a second session was written to"
    );
    // And the tail reached the helper untouched: the topic is a helper flag,
    // never a word the route ate.
    let (_, stdout, _) = run(&root, &["@demo", "memo", "read", "--topic", "t"]);
    assert!(stdout.contains("hello"), "the tail survived: {stdout}");
}

#[test]
fn the_short_form_and_the_link_are_one_helper() {
    let root = scratch("equivalence");
    let demo = plant(&root, "demo");
    let send = link(&demo, "send");
    let memo = link(&demo, "memo");

    // A read, and a write whose target does not resolve: both spellings answer
    // byte for byte, because there is one helper behind them.
    assert_eq!(
        run(&root, &["@demo", "memo", "read"]),
        through_link(&root, &memo, &["read"]),
        "memo read"
    );
    assert_eq!(
        run(&root, &["@demo", "send", "nobody", "hi"]),
        through_link(&root, &send, &["nobody", "hi"]),
        "send to an unresolvable target"
    );
}

#[test]
fn the_short_form_shares_the_one_translation_table() {
    let root = scratch("aliases");
    plant(&root, "demo");

    // `mark-done` is `state done <reason>` — a FIXED PREFIX, and the only way
    // the short form has it is by reading the one table. Bare `state` is the
    // read and exits 0; `mark-done` with no reason is a DECLARATION, which
    // needs a caller identity this shell does not have.
    let (read, _, _) = run(&root, &["@demo", "state"]);
    assert_eq!(read, Some(0), "bare state is the read");
    let (declared, _, stderr) = run(&root, &["@demo", "mark-done"]);
    assert_eq!(
        declared,
        Some(1),
        "mark-done carries its 'done' prefix into a declaration: {stderr}"
    );
    assert!(stderr.contains(ae::state::NO_IDENTITY), "{stderr}");

    // And an alias answers exactly as the name it aliases.
    assert_eq!(
        run(&root, &["@demo", "peak", "lead", "5"]),
        run(&root, &["@demo", "peek", "lead", "5"]),
        "peak IS peek"
    );
}

#[test]
fn every_helper_name_is_reachable_through_the_marker() {
    let root = scratch("catalog");
    plant(&root, "demo");
    // The whole table is pinned purely in `shim`; this is the black-box
    // sample that proves the pure pin is about the shipped route.
    //
    // A helper's OWN usage error and the marker's are both exit 2, so the code
    // discriminates nothing: the text does. A known name must be answered by
    // the helper it names, never by the route.
    for name in ["memo", "state", "requests", "mark-done", "peak"] {
        let (_, _, stderr) = run(&root, &["@demo", name, "--help-is-not-a-thing"]);
        assert!(
            !stderr.contains("ae @<session> <helper>"),
            "'{name}' is a helper, so the marker never refuses it: {stderr}"
        );
    }
    let (code, stdout, stderr) = run(&root, &["@demo", "nosuch", "x"]);
    assert_eq!(code, Some(2), "an unknown helper is a usage error");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("ae @<session> <helper>"), "{stderr}");
}

#[test]
fn a_malformed_marked_argv_is_a_usage_error_that_reads_nothing() {
    let root = scratch("usage");
    plant(&root, "demo");
    for argv in [
        vec!["@"],
        vec!["@", "memo", "read"],
        vec!["@demo"],
        vec!["@demo", "nosuch"],
        vec!["@..", "memo", "read"],
        vec!["@../escape", "memo", "read"],
        vec!["@demo/../other", "memo", "read"],
        vec!["@-lead", "memo", "read"],
        vec!["@.hidden", "memo", "read"],
    ] {
        let (code, stdout, stderr) = run(&root, &argv);
        assert_eq!(code, Some(2), "{argv:?}: {stdout}{stderr}");
        assert!(stdout.is_empty(), "{argv:?} wrote to stdout: {stdout}");
        assert!(
            stderr.contains("Usage: ae @<session> <helper>"),
            "{argv:?} must name the accepted spelling: {stderr}"
        );
    }
    assert_eq!(sessions(&root), ["demo"], "a usage error created a session");
}

#[test]
fn a_target_that_is_not_a_plain_session_directory_refuses_before_the_helper() {
    let root = scratch("targets");
    let demo = plant(&root, "demo");
    let elsewhere = root.join("elsewhere");
    assert!(
        fs::create_dir_all(&elsewhere).is_ok(),
        "an outside directory"
    );

    assert!(
        fs::write(root.join("sessions").join("afile"), "not a session").is_ok(),
        "a regular file where a session would be"
    );
    assert!(
        std::os::unix::fs::symlink(&elsewhere, root.join("sessions").join("alink")).is_ok(),
        "a symlink to a real directory"
    );
    assert!(
        std::os::unix::fs::symlink(root.join("nothing"), root.join("sessions").join("adangle"))
            .is_ok(),
        "a dangling symlink"
    );

    // ABSENT: exit 1, and it says where it looked.
    let (code, stdout, stderr) = run(&root, &["@missing", "memo", "read"]);
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("no session 'missing'"), "{stderr}");

    // PRESENT BUT NOT A PLAIN DIRECTORY: exit 1, unfollowed, every kind.
    for name in ["afile", "alink", "adangle"] {
        let (code, stdout, stderr) = run(&root, &[&format!("@{name}"), "memo", "add", "leak"]);
        assert_eq!(code, Some(1), "{name}: {stdout}{stderr}");
        assert!(stdout.is_empty(), "{name}: {stdout}");
        assert!(
            stderr.contains("is not a plain directory"),
            "{name}: {stderr}"
        );
    }
    // Nothing was followed and nothing was created on the way.
    assert!(
        !elsewhere.join("memo.tsv").exists(),
        "the symlink was followed"
    );
    assert!(!root.join("nothing").exists(), "the dangle was created");
    assert!(
        !demo.join("memo.tsv").exists(),
        "an unrelated session wrote"
    );
    assert_eq!(
        fs::read_to_string(root.join("sessions").join("afile")).unwrap_or_default(),
        "not a session",
        "the regular file was written through"
    );
}

/// A session RETAINED from before the current name grammar is still addressed
/// by every other consumer (`lib.rs::session_name_usable`), but the marker asks
/// the grammar alone. So it is reachable by its LINK and refused by the short
/// form — and the refusal has to hand over the spelling that works, or the
/// session reads as unreachable.
#[test]
fn a_pre_grammar_session_is_link_only_and_the_refusal_says_so() {
    let root = scratch("legacy");
    // A name the grammar refuses, on disk as a real direct-child directory:
    // exactly what the migration rule keeps working.
    let legacy = plant(&root, "old.session");
    let memo = link(&legacy, "memo");

    let (code, stdout, stderr) = run(&root, &["@old.session", "memo", "add", "kept"]);
    assert_eq!(code, Some(2), "the marker takes canonical names only");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(
        stderr.contains("'old.session' is not a session name"),
        "the refusal names the cause: {stderr}"
    );
    assert!(
        stderr.contains("the session's own link stays valid"),
        "the refusal must hand over the spelling that works: {stderr}"
    );
    assert!(
        !legacy.join("memo.tsv").exists(),
        "a refused call wrote to the legacy session"
    );

    // And that spelling really is the one that works.
    let (code, _, stderr) = through_link(&root, &memo, &["add", "kept"]);
    assert_eq!(code, Some(0), "the link still serves it: {stderr}");
    assert!(legacy.join("memo.tsv").exists(), "the link wrote");
}

#[test]
fn the_marker_grants_no_caller_identity() {
    let root = scratch("identity");
    let demo = plant(&root, "demo");
    let state = link(&demo, "state");

    // `state` is a WRITER that needs to know who is declaring. Naming the
    // session in the argv answers a different question, so it refuses — and it
    // refuses exactly as the link does.
    let short = run(&root, &["@demo", "state", "working", "nothing should land"]);
    let path = through_link(&root, &state, &["working", "nothing should land"]);
    assert_eq!(short.0, Some(1), "{short:?}");
    assert_eq!(
        short.2,
        format!("{}\n", ae::state::NO_IDENTITY),
        "{short:?}"
    );
    assert_eq!(short, path, "the typed session is not an identity");
    assert!(
        !demo.join("events.jsonl").exists(),
        "a refused declaration opened a ledger"
    );

    // A FOREIGN pane is still foreign: a pane id on a server that cannot
    // answer proves no identity, and the marker does not supply one.
    let foreign = |argv: &[&str]| {
        let out = ae()
            .env("HOME", &root)
            .env("AE_HOME", &root)
            .env(
                "TMUX",
                format!("{},0,0", root.join("no-server.sock").display()),
            )
            .env("TMUX_PANE", "%99")
            .args(argv)
            .output()
            .expect("the ae binary should run");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let short = foreign(&["@demo", "state", "working", "still nothing"]);
    assert_eq!(short.0, Some(1), "{short:?}");
    assert!(
        !demo.join("events.jsonl").exists(),
        "a foreign pane declared through the marker"
    );
}

#[test]
fn a_helper_word_without_the_marker_is_not_the_short_form() {
    let root = scratch("unmarked");
    let demo = plant(&root, "demo");
    assert!(
        fs::write(
            demo.join("memo.tsv"),
            "1970-01-01T00:00:00Z\thuman\tt\tsecret\n"
        )
        .is_ok(),
        "a memo to read"
    );
    // Marked, it reads.
    let (code, stdout, stderr) = run(&root, &["@demo", "memo", "read"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("secret"), "{stdout}");

    // Unmarked, `memo` is a word for the ordinary route — never this session's
    // helper, and never a new session either.
    for argv in [vec!["memo", "read"], vec!["demo", "memo", "read"]] {
        let (code, stdout, stderr) = run(&root, &argv);
        assert_ne!(code, Some(0), "{argv:?}: {stdout}{stderr}");
        assert!(
            !stdout.contains("secret"),
            "{argv:?} entered helper dispatch: {stdout}"
        );
    }
    assert_eq!(
        sessions(&root),
        ["demo"],
        "the unmarked route created state"
    );
}

#[test]
fn a_marked_word_never_becomes_a_launch() {
    let root = scratch("never-launch");
    plant(&root, "demo");
    // Every refusal class, over a name no session holds: none of them may fall
    // through to the route that would create one.
    for argv in [
        vec!["@fresh"],
        vec!["@fresh", "nosuch"],
        vec!["@fresh", "memo", "read"],
        vec!["@fresh", "spawn", "worker", "--using", "cl"],
    ] {
        let (code, stdout, stderr) = run(&root, &argv);
        assert!(
            matches!(code, Some(1 | 2)),
            "{argv:?}: {code:?} {stdout}{stderr}"
        );
        assert!(
            !root.join("sessions").join("fresh").exists(),
            "{argv:?} created a session"
        );
    }
    assert_eq!(sessions(&root), ["demo"]);
    for residue in [".lifecycle.fresh.lock", "fresh"] {
        assert!(
            !root.join(residue).exists() && !root.join("sessions").join(residue).exists(),
            "{residue} was left behind"
        );
    }
}
