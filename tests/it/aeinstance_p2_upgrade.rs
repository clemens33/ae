//! Offline P2 floor oracle; LAST literal is released default-only v2026.10.59.
#![allow(
    clippy::disallowed_methods,
    reason = "offline upgrade tests own their temporary HOME; only network fetch is injected"
)]

use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::PathBuf;

use super::{Pin, upgrade_with};

struct Home(PathBuf);
impl Home {
    fn new(tag: &str) -> Self {
        let dir = super::home_fixture(&format!("p2-{tag}"), None);
        fs::create_dir_all(dir.join(".ae")).expect("sandbox HOME");
        Self(dir)
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::set_permissions(self.0.join(".ae"), fs::Permissions::from_mode(0o755));
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn entries(path: &std::path::Path) -> Vec<std::ffi::OsString> {
    let mut names: Vec<_> = fs::read_dir(path)
        .expect("owned directory")
        .map(|e| e.expect("entry").file_name())
        .collect();
    names.sort();
    names
}

fn refuses(shape: &str) {
    for version in ["2026.10.58", "2026.10.59"] {
        let home = Home::new(&format!("{shape}-{version}"));
        let base = home.0.join(".ae");
        let instances = base.join("instances");
        match shape {
            "dir" | "denied" => fs::create_dir(&instances).expect("empty instances still count"),
            "file" => fs::write(&instances, "blocked").expect("non-directory instances"),
            "link" => symlink(home.0.join("missing"), &instances).expect("dangling instances"),
            _ => panic!("unknown floor fixture"),
        }
        let before = entries(&base);
        if shape == "denied" {
            fs::set_permissions(&base, fs::Permissions::from_mode(0o000)).expect("lstat EACCES");
        }
        let (mut calls, mut out) = (0, Vec::new());
        let why = upgrade_with(
            &home.0,
            &Pin::Exact(version.to_owned()),
            "darwin-arm64",
            &mut out,
            &mut |_, _, _| {
                calls += 1;
                Err("spec fetch reached".to_owned())
            },
        )
        .expect_err("old pin must refuse");
        fs::set_permissions(&base, fs::Permissions::from_mode(0o755)).expect("restore HOME");
        assert!(
            why.contains("instance") && why.contains("2026.10.59") && why.contains("AE_VERSION"),
            "wrong refusal: {why}"
        );
        assert_eq!(calls, 0, "floor checked after fetch");
        assert!(out.is_empty(), "floor printed download progress");
        assert_eq!(entries(&base), before, "floor created locks/state");
    }
}

#[test]
fn older_and_boundary_pin_refuse_an_empty_instances_directory() {
    refuses("dir");
}
#[test]
fn older_and_boundary_pin_refuse_a_non_directory_instances_entry() {
    refuses("file");
}
#[test]
fn older_and_boundary_pin_refuse_a_dangling_instances_symlink() {
    refuses("link");
}
#[test]
fn older_and_boundary_pin_refuse_unknown_instances_after_lstat_error() {
    refuses("denied");
}

#[test]
fn absence_newer_pin_and_latest_still_reach_fetch() {
    for (tag, pin, present) in [
        ("absent", Pin::Exact("2026.10.59".to_owned()), false),
        ("aware", Pin::Exact("2026.10.60".to_owned()), true),
        ("latest", Pin::Latest, true),
    ] {
        let home = Home::new(tag);
        if present {
            fs::create_dir(home.0.join(".ae/instances")).expect("instances");
        }
        let mut calls = 0;
        let why = upgrade_with(
            &home.0,
            &pin,
            "darwin-arm64",
            &mut Vec::new(),
            &mut |_, _, _| {
                calls += 1;
                Err("spec fetch reached".to_owned())
            },
        )
        .expect_err("injected fetch refusal");
        assert_eq!(calls, 1, "valid upgrade refused: {why}");
        assert_eq!(why, "spec fetch reached");
    }
}

#[test]
fn an_exact_equal_instance_aware_version_reaches_archive_for_repair() {
    let home = Home::new("equal");
    fs::create_dir(home.0.join(".ae/instances")).expect("instances");
    fs::create_dir_all(home.0.join(".local/bin")).expect("bin");
    symlink(
        home.0.join(".ae/versions/2099.1.1/ae-core"),
        home.0.join(".local/bin/ae"),
    )
    .expect("current link");
    let mut urls = Vec::new();
    let why = upgrade_with(
        &home.0,
        &Pin::Exact("2099.1.1".to_owned()),
        "darwin-arm64",
        &mut Vec::new(),
        &mut |url, _, _| {
            urls.push(url.to_owned());
            if url.ends_with("/SHA256SUMS") {
                Ok(format!("{}  ae-2099.1.1-darwin-arm64.tar.gz\n", "a".repeat(64)).into_bytes())
            } else {
                Err("spec archive reached".to_owned())
            }
        },
    )
    .expect_err("injected archive refusal");
    assert_eq!(urls.len(), 2, "same-version repair was a no-op: {why}");
    assert!(urls[1].ends_with("/ae-2099.1.1-darwin-arm64.tar.gz"));
    assert_eq!(why, "spec archive reached");
}
