//! Frozen rule-6 unit acceptance for the planned seam in
//! `src/lifecycle/aliases.rs`: `create_alias` links only when absent and
//! verifies, `keep_alias` maps a create failure to one warning line.
//! Included as a `cfg(test)` child module of `aliases`; it does not compile
//! on main before the seam exists. The ten public integration tests in
//! `regsid_spec.rs` compile on main and are RED there.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "independent scratch fixtures for the alias seam"
)]

use super::{create_alias, keep_alias};
use std::path::PathBuf;

fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(format!(
        "/tmp/ae-regsid-aliases-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directories");
    dir
}

fn target_dir(dir: &std::path::Path) -> PathBuf {
    let target = dir.join("new");
    std::fs::create_dir_all(&target).expect("target directory");
    target
}

#[test]
fn create_alias_links_when_absent_and_verifies() {
    let dir = scratch("create-ok");
    let target = target_dir(&dir);
    let alias = dir.join("old");
    create_alias(&alias, &target).expect("absent alias links");
    assert_eq!(
        std::fs::read_link(&alias).expect("readlink"),
        target,
        "the new link verifies to its target"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn create_alias_refuses_an_existing_file_without_touching_it() {
    let dir = scratch("create-file");
    let target = target_dir(&dir);
    let alias = dir.join("old");
    std::fs::write(&alias, b"existing-content").expect("existing file");
    let err = create_alias(&alias, &target).expect_err("file blocks creation");
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(
        std::fs::read(&alias).expect("file still reads"),
        b"existing-content",
        "an existing file is never replaced"
    );
    assert!(
        !std::fs::symlink_metadata(&alias)
            .expect("file metadata")
            .file_type()
            .is_symlink(),
        "no link was written over the file"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn create_alias_refuses_an_existing_directory_without_touching_it() {
    let dir = scratch("create-dir");
    let target = target_dir(&dir);
    let alias = dir.join("old");
    std::fs::create_dir_all(&alias).expect("existing directory");
    std::fs::write(alias.join("keep"), b"keep").expect("kept file");
    let err = create_alias(&alias, &target).expect_err("directory blocks creation");
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(
        std::fs::read(alias.join("keep")).expect("keep still reads"),
        b"keep",
        "an existing directory is never emptied"
    );
    assert!(
        std::fs::symlink_metadata(&alias)
            .expect("directory metadata")
            .is_dir(),
        "the directory was not turned into a link"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn create_alias_refuses_a_foreign_symlink_without_following_it() {
    let dir = scratch("create-link");
    let target = target_dir(&dir);
    let foreign = dir.join("foreign");
    std::fs::create_dir_all(&foreign).expect("foreign directory");
    std::fs::write(foreign.join("keep"), b"keep").expect("foreign keep");
    let alias = dir.join("old");
    std::os::unix::fs::symlink(&foreign, &alias).expect("foreign symlink");
    let err = create_alias(&alias, &target).expect_err("foreign symlink blocks creation");
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(
        std::fs::read_link(&alias).expect("still a symlink"),
        foreign,
        "the foreign link still names its own target"
    );
    assert_eq!(
        std::fs::read(foreign.join("keep")).expect("foreign keep reads"),
        b"keep",
        "nothing was written through the foreign link"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keep_alias_is_silent_on_success_and_warns_once_on_failure() {
    let dir = scratch("keep-alias");
    let target = target_dir(&dir);
    let alias = dir.join("old");
    let mut err = Vec::new();
    keep_alias(&alias, &target, &mut err).expect("success returns ok");
    assert!(err.is_empty(), "a kept alias warns nothing: {err:?}");
    assert_eq!(
        std::fs::read_link(&alias).expect("kept readlink"),
        target,
        "the success alias verifies"
    );

    // A second keep over the now-existing alias is a creation failure: it
    // returns Ok, writes one notice line, and leaves the entry untouched.
    let mut notice = Vec::new();
    keep_alias(&alias, &target, &mut notice).expect("failure still returns ok");
    let text = String::from_utf8(notice).expect("notice is utf-8");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1, "exactly one warning line: {text:?}");
    assert!(
        text.contains("old helper path") && text.contains("not kept"),
        "the line says the old helper path is not kept: {text:?}"
    );
    assert_eq!(
        std::fs::read_link(&alias).expect("untouched readlink"),
        target,
        "the failed keep did not replace the entry"
    );

    // The same notice contract holds for a foreign file.
    let blocked = dir.join("blocked");
    std::fs::write(&blocked, b"raced").expect("blocked file");
    let mut foreign = Vec::new();
    keep_alias(&blocked, &target, &mut foreign).expect("foreign failure still returns ok");
    let text = String::from_utf8(foreign).expect("foreign notice is utf-8");
    assert_eq!(
        text.lines().count(),
        1,
        "one line for a foreign block: {text:?}"
    );
    assert!(
        text.contains("old helper path") && text.contains("not kept"),
        "a foreign block warns the same line: {text:?}"
    );
    assert_eq!(
        std::fs::read(&blocked).expect("foreign still reads"),
        b"raced",
        "the foreign file was never replaced"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
