//! Frozen race-guard acceptance for `retarget_one` in
//! `src/lifecycle/aliases.rs`: a captured `prior` alone never replaces an
//! entry that is no longer our alias. Included as a `cfg(test)` child
//! module of `aliases`; it compiles with the extraction, not on main.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "independent scratch fixtures for the retarget race guard"
)]

use super::retarget_one;
use std::path::PathBuf;

fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(format!(
        "/tmp/ae-regsid-retarget-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directories");
    dir
}

#[test]
fn plain_file_replacing_the_alias_stays_untouched_with_no_creation() {
    let dir = scratch("plain-race");
    let new_dir = dir.join("new");
    std::fs::create_dir_all(&new_dir).expect("target directory");
    let old_dir = dir.join("old");
    let alias = dir.join("stale");
    std::fs::write(&alias, b"existing-content").expect("plain file");
    let mut err = Vec::new();
    retarget_one(&alias, &old_dir, &old_dir, &new_dir, &mut err).expect("race guard returns ok");
    assert_eq!(
        std::fs::read(&alias).expect("file still reads"),
        b"existing-content",
        "a plain file is never removed"
    );
    assert!(
        !std::fs::symlink_metadata(&alias)
            .expect("file metadata")
            .file_type()
            .is_symlink(),
        "no alias was created over the file"
    );
    assert!(err.is_empty(), "a skipped race warns nothing: {err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn foreign_sibling_alias_stays_pointing_outside_with_no_retarget() {
    let dir = scratch("foreign-race");
    let new_dir = dir.join("new");
    std::fs::create_dir_all(&new_dir).expect("target directory");
    let old_dir = dir.join("old");
    let foreign = dir.join("foreign");
    std::fs::create_dir_all(&foreign).expect("foreign directory");
    std::fs::write(foreign.join("keep"), b"keep").expect("foreign keep");
    let alias = dir.join("stale");
    std::os::unix::fs::symlink(&foreign, &alias).expect("foreign symlink");
    let mut err = Vec::new();
    retarget_one(&alias, &old_dir, &old_dir, &new_dir, &mut err).expect("race guard returns ok");
    assert_eq!(
        std::fs::read_link(&alias).expect("still a symlink"),
        foreign,
        "the foreign sibling link still names its own target"
    );
    assert_eq!(
        std::fs::read(foreign.join("keep")).expect("foreign keep reads"),
        b"keep",
        "nothing was written through the foreign link"
    );
    assert!(err.is_empty(), "a skipped race warns nothing: {err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unchanged_alias_retargets_directly_to_the_new_directory() {
    let dir = scratch("changed-race");
    let new_dir = dir.join("new");
    std::fs::create_dir_all(&new_dir).expect("target directory");
    let old_dir = dir.join("old");
    let alias = dir.join("stale");
    std::os::unix::fs::symlink(&old_dir, &alias).expect("alias symlink");
    let mut err = Vec::new();
    retarget_one(&alias, &old_dir, &old_dir, &new_dir, &mut err)
        .expect("unchanged alias retargets");
    assert_eq!(
        std::fs::read_link(&alias).expect("readlink of alias"),
        new_dir,
        "the unchanged alias retargets directly"
    );
    assert!(err.is_empty(), "a clean retarget warns nothing: {err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wrong_prior_never_retargets_an_unchanged_alias() {
    let dir = scratch("wrong-prior");
    let new_dir = dir.join("new");
    std::fs::create_dir_all(&new_dir).expect("target directory");
    let old_dir = dir.join("old");
    let alias = dir.join("stale");
    std::os::unix::fs::symlink(&old_dir, &alias).expect("alias symlink");
    let mut err = Vec::new();
    retarget_one(&alias, &dir.join("other"), &old_dir, &new_dir, &mut err)
        .expect("wrong prior returns ok");
    assert_eq!(
        std::fs::read_link(&alias).expect("readlink of alias"),
        old_dir,
        "a mismatched prior leaves the alias pointing at old"
    );
    assert!(err.is_empty(), "a skipped entry warns nothing: {err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
