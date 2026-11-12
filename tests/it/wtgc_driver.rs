//! Driver regressions for issue 275 beside the frozen `wtgc_spec`: real git, real ae, scratch
//! state only. Oracle: lead ruling ae-20261112T163456Z-40c1f3f6 on R2a — only an exactly canonical
//! absolute recorded `work_dir` row takes part in gc ownership; any other row makes the owner facts
//! unknown, whether its directory exists or not, and the footer names its session.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "driver fixtures create and inspect owned scratch repos and fail on setup errors"
)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::cli::{OwnedScratch, ae, bounded, git_in};

/// A scratch repo with one linked worktree `subject` and an empty state root.
struct World {
    scratch: PathBuf,
    repo: PathBuf,
    tree: PathBuf,
    _owned: OwnedScratch,
}

impl World {
    fn new(tag: &str) -> Self {
        let owned = OwnedScratch::root("wtgcdrv", tag);
        let scratch = fs::canonicalize(owned.path()).expect("canonical scratch root");
        let repo = scratch.join("repo");
        fs::create_dir_all(&repo).expect("repo directory");
        fs::create_dir_all(scratch.join("state").join("sessions")).expect("state directory");
        fs::write(
            scratch.join("config"),
            "[workspace]\nquota = off\nchat = off\n",
        )
        .expect("config");
        git_in(&repo, &["init", "-q", "-b", "main"]);
        fs::write(repo.join("a.txt"), "constant").expect("tracked file");
        git_in(&repo, &["add", "."]);
        git_in(&repo, &["commit", "-q", "-m", "base"]);
        let tree = scratch.join("subject");
        git_in(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "subject",
                tree.to_str().expect("UTF-8 fixture path"),
            ],
        );
        Self {
            scratch,
            repo,
            tree,
            _owned: owned,
        }
    }

    /// A stopped session whose spawned seat records `seat_dir` as its work dir.
    fn session(&self, name: &str, seat_dir: &Path) {
        let dir = self.scratch.join("state").join("sessions").join(name);
        fs::create_dir_all(&dir).expect("session directory");
        fs::write(
            dir.join("meta"),
            format!(
                "schema=2\nsession={name}\nsession_id=27500000-1111-2222-3333-444455556677\nmode=local\nwork_dir={repo}\norigin={repo}\nseat.main=lead\nprofile.main=fake\nagent_bin.main=claude\nseat.spawned.0=worker\nprofile.spawned.0=fake\nagent_bin.spawned.0=claude\nwork_dir.spawned.0={}\n",
                seat_dir.display(),
                repo = self.repo.display()
            ),
        )
        .expect("session metadata");
    }

    /// `ae worktree gc` (dry run) from the repo: exit code and stdout.
    fn list(&self) -> (Option<i32>, String) {
        let mut command = ae();
        command
            .current_dir(&self.repo)
            .env("PWD", &self.repo)
            .env("HOME", &self.scratch)
            .env("AE_HOME", self.scratch.join("state"))
            .env("CONFIG_FILE", self.scratch.join("config"))
            .args(["worktree", "gc"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let child = command.spawn().expect("ae process");
        let output =
            bounded(child, Duration::from_secs(45)).expect("ae returned within the fixture bound");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    }

    /// The listing row for the subject worktree.
    fn row<'a>(&self, listing: &'a str) -> &'a str {
        let path = self.tree.display().to_string();
        listing
            .lines()
            .find(|line| line.trim_end().ends_with(&path))
            .unwrap_or_else(|| panic!("no row for {path}:\n{listing}"))
    }
}

/// Run gc with one session whose seat row is `seat_dir`, which resolves to the subject worktree,
/// and require the owners gap that names that session instead of a resolved owner.
fn existing_noncanonical_row_is_a_gap(world: &World, seat_dir: &Path) {
    assert_eq!(
        fs::canonicalize(seat_dir).expect("the recorded dir exists"),
        fs::canonicalize(&world.tree).expect("subject exists"),
        "fixture row must resolve to the subject worktree"
    );
    // Raw bytes: `Path` equality would treat `/subject/.` and `/subject/` as canonical.
    assert_ne!(
        seat_dir.as_os_str(),
        fs::canonicalize(seat_dir)
            .expect("the recorded dir exists")
            .as_os_str(),
        "fixture row must not be canonical"
    );
    world.session("damaged", seat_dir);
    let (code, listing) = world.list();
    assert_eq!(code, Some(0), "{listing}");
    assert!(
        world.row(&listing).contains("owners unknown"),
        "a non-canonical existing row took part in ownership: {listing}"
    );
    let footer = listing
        .lines()
        .find(|line| line.starts_with("Owners unknown:"))
        .unwrap_or_else(|| panic!("no owners gap footer: {listing}"));
    assert!(
        footer.contains("'damaged'"),
        "gap does not name the session: {listing}"
    );
}

#[test]
fn an_existing_dot_dot_owner_row_makes_the_owner_facts_unknown() {
    let world = World::new("dotdot");
    let outside = world.scratch.join("outside");
    fs::create_dir(&outside).expect("outside directory");
    let recorded = outside.join("..").join("subject");
    existing_noncanonical_row_is_a_gap(&world, &recorded);
}

#[test]
fn an_existing_trailing_dot_owner_row_makes_the_owner_facts_unknown() {
    let world = World::new("dot");
    let recorded = world.tree.join(".");
    existing_noncanonical_row_is_a_gap(&world, &recorded);
}

#[test]
fn an_existing_trailing_slash_owner_row_makes_the_owner_facts_unknown() {
    let world = World::new("slash");
    let recorded = PathBuf::from(format!("{}/", world.tree.display()));
    existing_noncanonical_row_is_a_gap(&world, &recorded);
}

#[test]
fn an_existing_symlinked_owner_row_makes_the_owner_facts_unknown() {
    let world = World::new("symlink");
    let link = world.scratch.join("link-to-subject");
    symlink(&world.tree, &link).expect("symlink to the subject worktree");
    existing_noncanonical_row_is_a_gap(&world, &link);
}

#[test]
fn an_exactly_canonical_owner_row_still_names_its_owner() {
    // Control: the same fixture with the canonical row keeps the tree by owner, with no gap.
    let world = World::new("canonical");
    let recorded = fs::canonicalize(&world.tree).expect("subject exists");
    world.session("damaged", &recorded);
    let (code, listing) = world.list();
    assert_eq!(code, Some(0), "{listing}");
    assert!(
        world.row(&listing).contains("owner") && !world.row(&listing).contains("owners unknown"),
        "canonical row lost its owner: {listing}"
    );
    assert!(
        world.row(&listing).contains("damaged:worker"),
        "owner label missing: {listing}"
    );
    assert!(!listing.contains("Owners unknown:"), "{listing}");
}
