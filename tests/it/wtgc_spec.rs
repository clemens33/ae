//! Independently authored #275 acceptance tests: real git, real ae, scratch state only.
//! Oracles: the issue, lead ruling R1-R6/R2a, and the approved plan. No product seam is mocked.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures create and inspect owned scratch repos and fail on setup errors"
)]

use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::cli::{OwnedScratch, Runner, ae, bounded, git_in};
use super::renametell_routing_spec::Rig;

type Result = (Option<i32>, String, String);

struct World {
    scratch: OwnedScratch,
    repo: PathBuf,
    state: PathBuf,
    config: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let scratch = OwnedScratch::root("wtgc", tag);
        let repo = scratch.join("repo");
        let state = scratch.join("state");
        let config = scratch.join("config");
        fs::create_dir_all(&repo).expect("repo directory");
        fs::create_dir_all(state.join("sessions")).expect("state directory");
        fs::write(&config, "[workspace]\nquota = off\nchat = off\n").expect("config");
        init(&repo);
        let repo = fs::canonicalize(repo).expect("canonical repository root");
        let state = fs::canonicalize(state).expect("canonical state root");
        let config = fs::canonicalize(config).expect("canonical config file");
        Self {
            scratch,
            repo,
            state,
            config,
        }
    }

    fn tree(&self, name: &str) -> PathBuf {
        let tree = self.scratch.join(name);
        git_in(
            &self.repo,
            &["worktree", "add", "-q", "-b", name, text(&tree)],
        );
        let tree = fs::canonicalize(tree).expect("canonical worktree root");
        age(&tree, 3);
        tree
    }

    fn command(&self, cwd: &Path) -> Runner {
        let mut command = ae();
        command
            .current_dir(cwd)
            .env("PWD", cwd)
            .env("HOME", &self.scratch)
            .env("AE_HOME", &self.state)
            .env("CONFIG_FILE", &self.config);
        command
    }

    fn gc(&self, args: &[&str]) -> Result {
        self.gc_at(&self.repo, args)
    }

    fn gc_at(&self, cwd: &Path, args: &[&str]) -> Result {
        let mut command = self.command(cwd);
        command.args(["worktree", "gc"]).args(args);
        run(command)
    }

    fn list(&self) -> String {
        let result = self.gc(&[]);
        assert_eq!(result.0, Some(0), "gc listing: {result:?}");
        result.1
    }

    fn session(&self, name: &str, seat: &str, seat_dir: Option<&Path>) -> PathBuf {
        let dir = self.state.join("sessions").join(name);
        self.session_at(&dir, name, &self.repo, seat, seat_dir);
        dir
    }

    fn session_at(&self, dir: &Path, name: &str, work: &Path, seat: &str, seat_dir: Option<&Path>) {
        fs::create_dir_all(dir).expect("session directory");
        let extra = seat_dir.map_or_else(String::new, |path| {
            format!("work_dir.spawned.0={}\n", path.display())
        });
        fs::write(dir.join("meta"), format!(
            "schema=2\nsession={name}\nsession_id=27500000-1111-2222-3333-444455556666\nmode=local\nwork_dir={}\norigin={}\nseat.main=lead\nprofile.main=fake\nagent_bin.main=claude\nseat.spawned.0={seat}\nprofile.spawned.0=fake\nagent_bin.spawned.0=claude\n{extra}",
            work.display(), self.repo.display()
        )).expect("session metadata");
    }
}

fn text(path: &Path) -> &str {
    path.to_str().expect("UTF-8 fixture path")
}

fn init(repo: &Path) {
    git_in(repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join("a.txt"), "constant").expect("tracked file");
    fs::write(
        repo.join(".gitignore"),
        "target/\n.venv/\nnode_modules/\n.local/\n*.bak\n",
    )
    .expect("ignore file");
    git_in(repo, &["add", "."]);
    git_in(repo, &["commit", "-q", "-m", "base"]);
}

fn run(mut command: Runner) -> Result {
    let child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("ae process");
    let output =
        bounded(child, Duration::from_secs(45)).expect("ae returned within the fixture bound");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn reflog(tree: &Path) -> PathBuf {
    let path = PathBuf::from(git_in(tree, &["rev-parse", "--git-path", "logs/HEAD"]));
    if path.is_absolute() {
        path
    } else {
        tree.join(path)
    }
}

fn age(tree: &Path, days: u64) {
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current epoch")
        .as_secs()
        - days * 86_400
        - 60;
    let path = reflog(tree);
    let original = fs::read_to_string(&path).expect("worktree's own HEAD reflog");
    let mut rewritten = String::new();
    for line in original.lines() {
        let (fields, message) = line
            .split_once('\t')
            .map_or((line, None), |(fields, message)| (fields, Some(message)));
        let zone = fields.rfind(' ').expect("reflog zone");
        let timestamp = fields[..zone].rfind(' ').expect("reflog timestamp");
        write!(
            rewritten,
            "{} {epoch}{}",
            &fields[..timestamp],
            &fields[zone..]
        )
        .expect("format reflog metadata");
        if let Some(message) = message {
            write!(rewritten, "\t{message}").expect("format reflog message");
        }
        rewritten.push('\n');
    }
    fs::write(path, rewritten).expect("backdated valid reflog");
}

fn row<'a>(listing: &'a str, tree: &Path) -> &'a str {
    let path = tree.display().to_string();
    let physical = tree
        .parent()
        .and_then(|parent| fs::canonicalize(parent).ok())
        .zip(tree.file_name())
        .map(|(parent, name)| parent.join(name).display().to_string());
    listing
        .lines()
        .find(|line| {
            line.trim_end().ends_with(&path)
                || physical
                    .as_ref()
                    .is_some_and(|value| line.trim_end().ends_with(value))
        })
        .unwrap_or_else(|| panic!("no row for {path}:\n{listing}"))
}

fn removable(listing: &str, tree: &Path) -> bool {
    row(listing, tree).split_whitespace().next() == Some("remove")
}

fn token(listing: &str) -> String {
    let token = listing
        .lines()
        .find_map(|line| line.split_once("--remove ").map(|(_, rest)| rest))
        .and_then(|rest| rest.split_whitespace().next())
        .expect("confirmation token in dry run");
    assert_eq!(token.len(), 16, "token width: {listing}");
    assert!(
        token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "token alphabet: {listing}"
    );
    token.to_owned()
}

#[test]
fn default_is_a_dry_run_and_confirmation_removes_only_clean_merged_trees_and_keeps_branches() {
    let world = World::new("default");
    let clean = world.tree("candidate");
    let unmerged = world.tree("unmerged");
    let dirty = world.tree("dirty");
    git_in(
        &unmerged,
        &["commit", "-q", "--allow-empty", "-m", "not landed"],
    );
    age(&unmerged, 3);
    fs::write(dirty.join("a.txt"), "valuable edit").expect("dirty fixture");
    for cache in ["target", ".venv", "node_modules"] {
        fs::create_dir_all(clean.join(cache)).expect("cache directory");
        fs::write(clean.join(cache).join("payload"), "cache").expect("cache file");
    }
    let before = git_in(&world.repo, &["worktree", "list", "--porcelain"]);
    let listing = world.list();
    assert!(removable(&listing, &clean), "{listing}");
    assert!(
        row(&listing, &clean)
            .split_whitespace()
            .any(|word| word == "clean"),
        "{listing}"
    );
    assert!(!removable(&listing, &unmerged), "{listing}");
    assert!(!removable(&listing, &dirty), "{listing}");
    assert_eq!(
        git_in(&world.repo, &["worktree", "list", "--porcelain"]),
        before,
        "dry run changed registration"
    );
    assert!(
        clean.join("target/payload").exists(),
        "dry run deleted a cache"
    );
    assert!(
        !listing
            .lines()
            .any(|line| line.split_whitespace().next() == Some("remove")
                && line.trim_end().ends_with(text(&world.repo))),
        "main became a candidate: {listing}"
    );
    let result = world.gc(&["--remove", &token(&listing)]);
    assert_eq!(result.0, Some(0), "{result:?}");
    assert!(!clean.exists(), "confirmed worktree remained");
    assert!(
        unmerged.exists() && dirty.exists(),
        "ineligible tree was removed"
    );
    assert_eq!(
        fs::read_to_string(dirty.join("a.txt")).expect("dirty work retained"),
        "valuable edit"
    );
    assert!(
        !git_in(
            &world.repo,
            &["show-ref", "--verify", "refs/heads/candidate"]
        )
        .is_empty(),
        "gc deleted a branch"
    );
}

#[test]
fn cheap_display_facts_are_present_even_for_an_owned_dirty_tree() {
    let world = World::new("display");
    let tree = world.tree("subject");
    fs::write(tree.join("a.txt"), "uncommitted work").expect("dirty file");
    world.session("owner", "worker", Some(&tree));
    let listing = world.list();
    let line = row(&listing, &tree);
    assert!(
        line.contains("owner")
            && line.contains("yes")
            && line.contains("3d")
            && line.contains("dirty"),
        "cheap facts missing: {listing}"
    );
}

#[test]
fn cheap_only_rows_say_clean_question_and_only_a_proven_eligible_row_says_clean() {
    let world = World::new("proof-state");
    let eligible = world.tree("eligible");
    let owned = world.tree("owned");
    let unmerged = world.tree("unmerged");
    world.session("owner", "worker", Some(&owned));
    git_in(
        &unmerged,
        &["commit", "-q", "--allow-empty", "-m", "not landed"],
    );
    age(&unmerged, 3);
    let listing = world.list();
    assert!(removable(&listing, &eligible), "{listing}");
    assert!(
        row(&listing, &eligible)
            .split_whitespace()
            .any(|word| word == "clean"),
        "eligible row lacks the full-proof state: {listing}"
    );
    for tree in [&owned, &unmerged] {
        let line = row(&listing, tree);
        assert!(!removable(&listing, tree), "{listing}");
        assert!(
            line.split_whitespace().any(|word| word == "clean?")
                && !line.split_whitespace().any(|word| word == "clean"),
            "cheap checks were presented as a full proof: {listing}"
        );
    }
}

#[test]
fn staged_untracked_masked_and_noncache_ignored_data_are_preserved() {
    let world = World::new("cleanliness");
    let control = world.tree("control");
    for (name, kind) in [
        ("staged", 0),
        ("untracked", 1),
        ("assumed", 2),
        ("skipped", 3),
        ("evidence", 4),
        ("backup", 5),
    ] {
        let tree = world.tree(name);
        match kind {
            0 => {
                fs::write(tree.join("a.txt"), "staged work").expect("write");
                git_in(&tree, &["add", "a.txt"]);
            }
            1 => fs::write(tree.join("notes"), "untracked work").expect("write"),
            2 => {
                git_in(&tree, &["update-index", "--assume-unchanged", "a.txt"]);
                fs::write(tree.join("a.txt"), "masked work").expect("write");
            }
            3 => {
                git_in(&tree, &["update-index", "--skip-worktree", "a.txt"]);
                fs::write(tree.join("a.txt"), "sparse work").expect("write");
            }
            4 => {
                fs::create_dir_all(tree.join(".local")).expect("mkdir");
                fs::write(tree.join(".local/evidence"), "review evidence").expect("write");
            }
            5 => fs::write(tree.join("a.bak"), "backup").expect("write"),
            _ => unreachable!(),
        }
    }
    let listing = world.list();
    assert!(removable(&listing, &control), "positive control: {listing}");
    for name in [
        "staged",
        "untracked",
        "assumed",
        "skipped",
        "evidence",
        "backup",
    ] {
        assert!(
            !removable(&listing, &world.scratch.join(name)),
            "{name}: {listing}"
        );
    }
    let result = world.gc(&["--remove", &token(&listing)]);
    assert_eq!(result.0, Some(0), "{result:?}");
    assert!(!control.exists(), "control was not removed");
    for name in [
        "staged",
        "untracked",
        "assumed",
        "skipped",
        "evidence",
        "backup",
    ] {
        assert!(world.scratch.join(name).exists(), "{name} was deleted");
    }
}

#[test]
fn a_clean_filter_cannot_make_changed_raw_bytes_removable() {
    let world = World::new("filter");
    fs::write(world.repo.join(".gitattributes"), "*.txt filter=hide\n").expect("attributes");
    git_in(
        &world.repo,
        &["config", "filter.hide.clean", "printf constant"],
    );
    git_in(&world.repo, &["add", ".gitattributes"]);
    git_in(&world.repo, &["commit", "-q", "-m", "attributes"]);
    let control = world.tree("control");
    let tree = world.tree("filtered");
    fs::write(tree.join("a.txt"), "valuable changed bytes").expect("changed raw bytes");
    let listing = world.list();
    assert!(
        removable(&listing, &control),
        "positive filter control: {listing}"
    );
    assert!(
        !removable(&listing, &tree),
        "raw bytes were called removable: {listing}"
    );
    let result = world.gc(&["--remove", &token(&listing)]);
    assert_eq!(result.0, Some(0), "{result:?}");
    assert_eq!(
        fs::read_to_string(tree.join("a.txt")).expect("raw bytes kept"),
        "valuable changed bytes"
    );
}

#[test]
fn idle_uses_the_worktrees_own_reflog_and_unknown_or_recent_trees_stay() {
    let world = World::new("idle");
    let old = world.tree("old");
    let recent = world.tree("recent");
    let unknown = world.tree("unknown");
    age(&recent, 0);
    fs::remove_file(reflog(&unknown)).expect("remove only fixture reflog");
    let listing = world.list();
    assert!(removable(&listing, &old), "{listing}");
    assert!(
        !removable(&listing, &recent),
        "old commit did not make the new checkout idle: {listing}"
    );
    assert!(
        !removable(&listing, &unknown),
        "absent reflog did not fail closed: {listing}"
    );
    assert!(
        row(&listing, &unknown).contains("idle unknown")
            && row(&listing, &unknown)
                .split_whitespace()
                .any(|word| word == "?"),
        "branch reflog fallback hid the missing worktree reflog: {listing}"
    );
    assert!(row(&listing, &recent).contains("recent"), "{listing}");
    let longer = world.gc(&["--idle", "4"]);
    assert_eq!(longer.0, Some(0), "{longer:?}");
    assert!(
        !removable(&longer.1, &old),
        "idle threshold ignored: {longer:?}"
    );
    let minimum = world.gc(&["--idle", "1"]);
    assert_eq!(minimum.0, Some(0), "{minimum:?}");
    assert!(
        removable(&minimum.1, &old),
        "valid minimum rejected: {minimum:?}"
    );
}

#[test]
fn locked_missing_detached_and_no_target_are_conservative() {
    let world = World::new("special");
    let locked = world.tree("locked");
    let missing = world.tree("missing");
    let detached = world.tree("detached");
    git_in(&world.repo, &["worktree", "lock", text(&locked)]);
    fs::remove_dir_all(&missing).expect("fixture worktree manually vanished");
    git_in(&detached, &["checkout", "-q", "--detach"]);
    age(&detached, 3);
    let listing = world.list();
    assert!(!removable(&listing, &locked), "{listing}");
    assert!(!removable(&listing, &missing), "{listing}");
    assert!(
        removable(&listing, &detached),
        "merged detached tree should be removable: {listing}"
    );
    git_in(&world.repo, &["checkout", "-q", "--detach"]);
    let no_target = world.list();
    assert!(
        !removable(&no_target, &detached),
        "detached main was invented as a target: {no_target}"
    );
}

#[test]
fn caller_worktree_root_and_nested_directory_are_protected() {
    let world = World::new("cwd");
    let tree = world.tree("subject");
    let nested = tree.join("empty-nested");
    fs::create_dir_all(&nested).expect("nested current directory");
    for cwd in [&tree, &nested] {
        let result = world.gc_at(cwd, &[]);
        assert_eq!(result.0, Some(0), "{result:?}");
        assert!(
            !removable(&result.1, &tree),
            "caller worktree removable: {result:?}"
        );
        assert!(row(&result.1, &tree).contains("current dir"), "{result:?}");
    }
    assert!(
        removable(&world.list(), &tree),
        "caller guard leaked to unrelated cwd"
    );
}

#[test]
fn owners_by_exact_nested_dir_branch_and_leaf_keep_only_the_owned_tree() {
    for kind in 0..4 {
        let world = World::new(&format!("owner-{kind}"));
        let tree = world.tree("subject");
        let control = world.tree("control");
        let nested = tree.join("empty-subdir");
        fs::create_dir_all(&nested).expect("nested seat directory");
        match kind {
            0 => {
                world.session("owner", "worker", Some(&tree));
            }
            1 => {
                world.session("owner", "worker", Some(&nested));
            }
            2 => {
                git_in(&world.repo, &["branch", "-m", "subject", "worker"]);
                world.session("owner", "worker", None);
            }
            3 => {
                world.session("owner", "subject", None);
            }
            _ => unreachable!(),
        }
        let listing = world.list();
        assert!(
            row(&listing, &tree).contains("owner"),
            "owner kind {kind}: {listing}"
        );
        assert!(!removable(&listing, &tree), "owner kind {kind}: {listing}");
        assert!(
            removable(&listing, &control),
            "unrelated control kind {kind}: {listing}"
        );
    }
}

#[test]
fn vanished_recorded_dirs_protect_lexical_descendants_without_blocking_unrelated_trees() {
    for inside in [false, true] {
        let world = World::new(if inside {
            "vanished-in"
        } else {
            "vanished-out"
        });
        let tree = world.tree("subject");
        let control = world.tree("control");
        let recorded = if inside {
            tree.join("vanished-seat-dir")
        } else {
            world.scratch.join("vanished-seat-dir")
        };
        fs::create_dir_all(&recorded).expect("once-existing recorded seat directory");
        let recorded = fs::canonicalize(&recorded).expect("canonical recording");
        world.session("owner", "worker", Some(&recorded));
        fs::remove_dir(&recorded).expect("recorded directory vanished");
        let listing = world.list();
        assert!(
            !listing.contains("owners unknown"),
            "absence was treated as unreadability: {listing}"
        );
        assert_eq!(
            removable(&listing, &tree),
            !inside,
            "lexical owner case inside={inside}: {listing}"
        );
        assert!(
            removable(&listing, &control),
            "vanished row blocked unrelated tree: {listing}"
        );
    }
}

#[test]
fn unresolved_owner_records_fail_closed_and_name_the_offending_session() {
    for kind in 0..6 {
        let world = World::new(&format!("unknown-owner-{kind}"));
        let tree = world.tree("subject");
        let dir = world.session("broken-owner", "worker", None);
        let meta = dir.join("meta");
        let original = fs::read_to_string(&meta).expect("meta");
        let revised = match kind {
            0 => format!("{original}work_dir.spawned.0=relative\n"),
            1 => format!(
                "{original}work_dir.spawned.0={}\nwork_dir.spawned.0={}\n",
                tree.display(),
                world.repo.display()
            ),
            2 => format!("{original}work_dir.spawned.0=\n"),
            3 => format!("{original}work_dir={}\n", world.repo.display()),
            4 => original.replace("seat.spawned.0=worker\n", "seat.spawned.0=\n"),
            5 => original,
            _ => unreachable!(),
        };
        fs::write(&meta, revised).expect("damaged fixture meta");
        if kind == 5 {
            fs::remove_file(&meta).expect("remove meta");
            fs::create_dir(&meta).expect("unread meta node");
        }
        let listing = world.list();
        assert!(
            !removable(&listing, &tree),
            "unknown owner kind {kind}: {listing}"
        );
        assert!(
            listing.contains("owners unknown") && listing.contains("broken-owner"),
            "unexplained gap kind {kind}: {listing}"
        );
    }
}

#[test]
fn session_workdirs_and_older_layout_state_are_not_collected() {
    let world = World::new("session-protection");
    let session_tree = world.tree("session-tree");
    let nested_state = world.state.join("worktrees/old-tree");
    git_in(
        &world.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "old-tree",
            text(&nested_state),
        ],
    );
    age(&nested_state, 3);
    let control = world.tree("control");
    world.session_at(
        &world.state.join("sessions/session-owner"),
        "session-owner",
        &session_tree,
        "worker",
        None,
    );
    world.session_at(
        &nested_state.join(".ae/old-session"),
        "old-session",
        &world.repo,
        "oldworker",
        None,
    );
    let listing = world.list();
    assert!(
        !removable(&listing, &session_tree),
        "session workdir not protected: {listing}"
    );
    assert!(
        !removable(&listing, &nested_state),
        "older layout state not protected: {listing}"
    );
    assert!(row(&listing, &nested_state).contains("owner"), "{listing}");
    assert!(removable(&listing, &control), "{listing}");
}

#[test]
fn stale_tokens_bind_target_branch_sha_idle_and_the_selected_set() {
    for kind in 0..4 {
        let world = World::new(&format!("token-{kind}"));
        let tree = world.tree("subject");
        let old_token = token(&world.list());
        match kind {
            0 => {
                git_in(
                    &world.repo,
                    &["commit", "-q", "--allow-empty", "-m", "target moved"],
                );
            }
            1 => {
                git_in(&world.repo, &["branch", "-m", "main", "renamed-target"]);
            }
            2 => {
                world.tree("new-candidate");
            }
            3 => {
                git_in(
                    &world.repo,
                    &["branch", "-m", "subject", "renamed-candidate"],
                );
            }
            _ => unreachable!(),
        }
        let result = world.gc(&["--remove", &old_token]);
        assert_eq!(result.0, Some(1), "stale token kind {kind}: {result:?}");
        assert!(
            tree.exists(),
            "stale confirmation removed a tree kind {kind}"
        );
        assert!(
            format!("{}{}", result.1, result.2).contains("changed"),
            "no stale-set explanation: {result:?}"
        );
    }
    let world = World::new("token-idle");
    let tree = world.tree("subject");
    let result = world.gc(&["--idle", "1", "--remove", &token(&world.list())]);
    assert_eq!(result.0, Some(1), "idle value not bound: {result:?}");
    assert!(tree.exists());
}

#[test]
fn invalid_worktree_syntax_is_usage_and_does_not_launch_a_session() {
    let world = World::new("syntax");
    for args in [
        vec!["worktree"],
        vec!["worktree", "other"],
        vec!["worktree", "gc", "--idle", "0"],
        vec!["worktree", "gc", "--idle", "-1"],
        vec!["worktree", "gc", "--idle", "x"],
        vec!["worktree", "gc", "--idle"],
        vec!["worktree", "gc", "--remove"],
        vec!["worktree", "gc", "--force"],
    ] {
        let mut command = world.command(&world.repo);
        command.args(&args);
        let result = run(command);
        assert_eq!(result.0, Some(2), "{args:?}: {result:?}");
    }
    assert_eq!(
        fs::read_dir(world.state.join("sessions"))
            .expect("sessions")
            .count(),
        0,
        "usage created a session"
    );
}

// A git interposer is an external dependency fixture. All reads and ordinary removals reach real
// git. A marker proves each injected response or between-item mutation actually happened.
const GIT_INTERPOSER: &str = r#"#!/usr/bin/perl
use strict;
use warnings;
my $real = $ENV{WTGC_REAL_GIT} or die 'no real git';
open(my $trace, '>>', $ENV{WTGC_TRACE}) or die $!;
print $trace join(' ', @ARGV), "\n";
close $trace;
my $mode = $ENV{WTGC_MODE} // '';
sub mark {
    open(my $file, '>', $ENV{WTGC_MARKER}) or die $!;
    print $file "applied\n";
    close $file;
}
if ($mode eq 'fail-all') { mark(); exit 128; }
if ($mode eq 'fail-merge' && grep { $_ eq 'merge-base' || $_ eq 'rev-list' } @ARGV) { mark(); exit 128; }
if ($mode eq 'fail-diff' && grep { /\Adiff(?:-|\z)/ || $_ eq 'status' } @ARGV) { mark(); exit 128; }
my $listing = join(' ', @ARGV) =~ /worktree list --porcelain/;
if ($listing && $mode =~ /^proof-/) {
    my $count = 0;
    if (open(my $old, '<', $ENV{WTGC_COUNT})) { $count = <$old>; close $old; }
    open(my $new, '>', $ENV{WTGC_COUNT}) or die $!;
    print $new $count + 1;
    close $new;
}
if ($listing && -e $ENV{WTGC_MARKER} && $mode =~ /^post-/) {
    print "worktree /unreadable\nHEAD broken\n" if $mode eq 'post-malformed';
    exit 0;
}
if ($mode =~ /^proof-/ && !-e $ENV{WTGC_MARKER}
    && join(' ', @ARGV) eq "-C $ENV{WTGC_OTHER} rev-parse HEAD") {
    open(my $count_file, '<', $ENV{WTGC_COUNT}) or die $!;
    my $count = <$count_file>;
    close $count_file;
    if ($count >= 2) {
        if ($mode eq 'proof-head') {
            system($real, '-C', $ENV{WTGC_REPO}, 'update-ref', 'refs/heads/subject', $ENV{WTGC_REPLACEMENT}) == 0 or die 'change proof HEAD';
        } elsif ($mode eq 'proof-branch') {
            system($real, '-C', $ENV{WTGC_OTHER}, 'symbolic-ref', 'HEAD', 'refs/heads/replacement') == 0 or die 'change proof branch';
        } elsif ($mode eq 'proof-target') {
            system($real, '-C', $ENV{WTGC_REPO}, 'symbolic-ref', 'HEAD', 'refs/heads/replacement') == 0 or die 'change proof target';
        }
        mark();
    }
}
for my $arg (@ARGV) {
    die 'forbidden destructive argv' if $arg eq '--force' || $arg eq '-D' || $arg eq 'prune' || $arg eq 'branch';
}
my $remove = 0;
for (my $i = 0; $i + 1 < @ARGV; $i++) {
    $remove = 1 if $ARGV[$i] eq 'worktree' && $ARGV[$i + 1] eq 'remove';
}
if ($remove) {
    if ($mode =~ /^post-/) {
        system($real, @ARGV) == 0 or die 'real remove failed';
        mark(); exit 0;
    }
    if ($mode eq 'fail-kept') { mark(); exit 1; }
    if ($mode eq 'success-kept') { mark(); exit 0; }
    if ($mode eq 'fail-after') {
        system($real, @ARGV) == 0 or die 'real remove failed';
        mark(); exit 1;
    }
    if (!-e $ENV{WTGC_MARKER}) {
        if ($mode eq 'dirty') {
            open(my $file, '>', "$ENV{WTGC_OTHER}/a.txt") or die $!;
            print $file 'arrived after the first classification';
            close $file;
            mark();
        } elsif ($mode eq 'branch' || $mode eq 'head') {
            if ($mode eq 'head') {
                system($real, '-C', $ENV{WTGC_OTHER}, 'update-ref', '--no-deref', 'HEAD', $ENV{WTGC_REPLACEMENT}) == 0 or die 'change detached HEAD';
            } else {
                system($real, '-C', $ENV{WTGC_OTHER}, 'symbolic-ref', 'HEAD', 'refs/heads/replacement') == 0 or die 'change branch';
            }
            open(my $old_log, '<', $ENV{WTGC_REFLOG}) or die $!;
            my @rows = <$old_log>;
            close $old_log;
            open(my $new_log, '>', $ENV{WTGC_REFLOG}) or die $!;
            for my $row (@rows) {
                $row =~ s/(> )\d+( [+-]\d{4})(\t|$)/$1 . $ENV{WTGC_OLD_EPOCH} . $2 . $3/e;
                print $new_log $row;
            }
            close $new_log;
            mark();
        } elsif ($mode eq 'target') {
            system($real, '-C', $ENV{WTGC_REPO}, 'symbolic-ref', 'HEAD', 'refs/heads/replacement') == 0 or die 'change target';
            mark();
        }
    }
}
exec $real, @ARGV;
die $!;
"#;

fn interpose(command: &mut Runner, root: &Path, mode: &str, repo: &Path, other: &Path) -> PathBuf {
    let bin = root.join("git-bin");
    fs::create_dir_all(&bin).expect("git shim directory");
    fs::write(bin.join("git"), GIT_INTERPOSER).expect("git external fixture");
    fs::set_permissions(bin.join("git"), fs::Permissions::from_mode(0o755))
        .expect("git shim executable");
    let real = ae::doctor::resolve_on_path("git").expect("installed git");
    let original = command
        .get_envs()
        .find_map(|(key, value)| {
            if key == "PATH" {
                value.map(ToOwned::to_owned)
            } else {
                None
            }
        })
        .or_else(|| std::env::var_os("PATH"))
        .expect("runner PATH");
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&original)))
        .expect("fixture PATH");
    let marker = root.join("git-mutation-applied");
    let replacement = if matches!(mode, "head" | "proof-head") {
        git_in(repo, &["rev-parse", "refs/heads/replacement"])
    } else {
        String::new()
    };
    command
        .env("PATH", path)
        .env("WTGC_REAL_GIT", real)
        .env("WTGC_TRACE", root.join("git-trace"))
        .env("WTGC_MODE", mode)
        .env("WTGC_MARKER", &marker)
        .env("WTGC_COUNT", root.join("git-list-count"))
        .env("WTGC_REPO", repo)
        .env("WTGC_OTHER", other)
        .env("WTGC_REPLACEMENT", replacement)
        .env("WTGC_REFLOG", reflog(other))
        .env(
            "WTGC_OLD_EPOCH",
            (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("current epoch")
                .as_secs()
                - 3 * 86_400
                - 60)
                .to_string(),
        );
    marker
}

#[test]
fn each_removal_rechecks_dirty_facts_and_the_exact_candidate_and_target_identity() {
    // Approved plan [RO] removes candidates in token order, sorted by raw path bytes.
    for mode in ["dirty", "branch", "head", "target"] {
        let world = World::new(&format!("recheck-{mode}"));
        let first_commit = git_in(&world.repo, &["rev-parse", "HEAD"]);
        if mode == "head" {
            git_in(
                &world.repo,
                &["commit", "-q", "--allow-empty", "-m", "second"],
            );
        }
        let first = world.tree("a-first");
        let second = world.tree("b-second");
        if mode == "head" {
            git_in(&second, &["checkout", "-q", "--detach"]);
            age(&second, 3);
        }
        let replacement = if mode == "head" {
            first_commit
        } else {
            git_in(&world.repo, &["rev-parse", "HEAD"])
        };
        git_in(&world.repo, &["branch", "replacement", &replacement]);
        let confirm = token(&world.list());
        let mut command = world.command(&world.repo);
        command.args(["worktree", "gc", "--remove", &confirm]);
        let marker = interpose(&mut command, &world.scratch, mode, &world.repo, &second);
        let result = run(command);
        assert_eq!(
            fs::read_to_string(marker).expect("mutation must apply"),
            "applied\n"
        );
        assert_eq!(result.0, Some(1), "changed facts mode {mode}: {result:?}");
        assert!(
            !first.exists(),
            "positive first removal did not happen mode {mode}: {result:?}"
        );
        assert!(
            second.exists(),
            "stale second observation authorized deletion mode {mode}: {result:?}"
        );
        if mode == "dirty" {
            assert_eq!(
                fs::read_to_string(second.join("a.txt")).expect("new work kept"),
                "arrived after the first classification"
            );
        } else {
            assert!(
                removable(&world.list(), &second),
                "identity test only proved an ordinary disqualifier, mode {mode}"
            );
        }
    }
}

#[test]
fn git_failure_reports_kept_only_when_both_the_directory_and_registration_remain() {
    for mode in ["fail-kept", "fail-after", "success-kept"] {
        let world = World::new(&format!("remove-result-{mode}"));
        let tree = world.tree("subject");
        let confirm = token(&world.list());
        let mut command = world.command(&world.repo);
        command.args(["worktree", "gc", "--remove", &confirm]);
        let marker = interpose(&mut command, &world.scratch, mode, &world.repo, &tree);
        let result = run(command);
        assert!(
            marker.exists(),
            "git result injection not applied: {result:?}"
        );
        assert_eq!(
            result.0,
            Some(1),
            "unproven remove reported success mode {mode}: {result:?}"
        );
        let output = format!("{}{}", result.1, result.2);
        if mode == "fail-kept" {
            assert!(
                tree.exists()
                    && git_in(&world.repo, &["worktree", "list", "--porcelain"])
                        .contains(text(&tree))
            );
            assert!(
                output.contains("kept"),
                "proven kept state missing: {output}"
            );
        } else {
            assert!(
                output.contains("state unknown"),
                "partial/unobserved result overclaimed: {output}"
            );
            assert!(!output.contains("it is kept"), "false kept claim: {output}");
            assert_eq!(tree.exists(), mode == "success-kept");
        }
    }
}

#[test]
fn symlinked_worktree_paths_and_unsupported_tracked_modes_are_kept() {
    let world = World::new("symlink");
    let tree = world.tree("subject");
    let moved = world.scratch.join("actual-tree");
    fs::rename(&tree, &moved).expect("move fixture without repairing registration");
    symlink(&moved, &tree).expect("registered path becomes a symlink");
    let listing = world.list();
    assert!(
        !removable(&listing, &tree),
        "symlink path admitted: {listing}"
    );
    assert!(moved.join("a.txt").exists(), "symlink target was touched");
    let world = World::new("tracked-link");
    symlink("a.txt", world.repo.join("link")).expect("tracked symlink");
    git_in(&world.repo, &["add", "link"]);
    git_in(&world.repo, &["commit", "-q", "-m", "link"]);
    let tree = world.tree("subject");
    assert!(
        !removable(&world.list(), &tree),
        "unsupported tracked symlink was removable"
    );
}

const SESSION: &str = "wtgcsession";
const SESSION_ID: &str = "27500000-aaaa-bbbb-cccc-000000000001";

fn retire_rig(tag: &str) -> (Rig, PathBuf, String) {
    let rig = Rig::new(tag);
    let repo = rig.scratch.join("repo");
    fs::create_dir_all(&repo).expect("retire repo");
    init(&repo);
    let repo = fs::canonicalize(repo).expect("canonical repo");
    let [main, _] = rig.create(SESSION, SESSION_ID);
    let meta = rig.dir(SESSION).join("meta");
    let original = fs::read_to_string(&meta).expect("session meta");
    let revised = original
        .lines()
        .map(|line| {
            if line.starts_with("work_dir=") || line.starts_with("origin=") {
                format!(
                    "{}={}",
                    line.split_once('=').expect("row").0,
                    repo.display()
                )
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(meta, revised).expect("session repo");
    (rig, repo, main)
}

// This is the emitted spawned-seat shape: a private pane, its stamps, an incarnation and the
// atomic seat/target rows. It avoids invoking a harness while testing retirement itself.
fn seat(rig: &Rig, repo: &Path, name: &str, index: usize, work: Option<&Path>) -> String {
    let slot = format!("spawned.{index}");
    let pane = rig
        .tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &format!("{SESSION}:"),
            "-n",
            name,
            "-c",
            text(repo),
            "sleep 120",
        ])
        .trim()
        .to_owned();
    rig.tmux(&["set-option", "-p", "-t", &pane, "@ae_slot", &slot]);
    rig.tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", name]);
    let meta = rig.dir(SESSION).join("meta");
    let original = fs::read_to_string(&meta).expect("meta");
    let target = work.map_or_else(String::new, |dir| {
        format!("work_dir.{slot}={}\n", dir.display())
    });
    fs::write(meta, format!("{original}seat.{slot}={name}\nprofile.{slot}=fake\nagent_bin.{slot}=claude\nharness_session.{slot}=pending\nlaunch_id.{slot}=wtgc-{index}\npane.{slot}={pane}\n{target}")).expect("spawned seat shape");
    pane
}

fn retire_tree(rig: &Rig, repo: &Path, name: &str) -> PathBuf {
    let tree = rig.scratch.join(name);
    git_in(repo, &["worktree", "add", "-q", "-b", name, text(&tree)]);
    fs::canonicalize(tree).expect("canonical tree")
}

fn refused_untouched(rig: &Rig, main: &str, target: &str, slot: &str) {
    let meta = rig.dir(SESSION).join("meta");
    let journal = rig.dir(SESSION).join("events.jsonl");
    let before_meta = fs::read(&meta).expect("meta before");
    let before_events = fs::read(&journal).unwrap_or_default();
    let before_panes = rig.tmux(&[
        "list-panes",
        "-s",
        "-t",
        SESSION,
        "-F",
        "#{pane_id}:#{@ae_agent}:#{@ae_slot}",
    ]);
    let artifact = rig.dir(SESSION).join(format!("launch.{slot}.started"));
    fs::write(&artifact, "preserve marker").expect("launch artifact");
    let result = rig.helper(SESSION, "retire", &[target], Some(main));
    assert_eq!(
        result.0,
        Some(1),
        "retire should refuse before mutation: {result:?}"
    );
    assert!(
        result.2.contains("worktree"),
        "missing ownership refusal: {result:?}"
    );
    assert_eq!(fs::read(meta).expect("meta retained"), before_meta);
    assert_eq!(fs::read(journal).unwrap_or_default(), before_events);
    assert_eq!(
        rig.tmux(&[
            "list-panes",
            "-s",
            "-t",
            SESSION,
            "-F",
            "#{pane_id}:#{@ae_agent}:#{@ae_slot}"
        ]),
        before_panes
    );
    assert_eq!(
        fs::read_to_string(artifact).expect("artifact retained"),
        "preserve marker"
    );
}

#[test]
fn retire_refuses_dirty_or_unmerged_owned_trees_before_any_mutation_for_name_and_pane_targets() {
    for kind in ["dirty", "unmerged", "by-name"] {
        let (rig, repo, main) = retire_rig(&format!("retire-{kind}"));
        let name = if kind == "by-name" { "worker" } else { "tree" };
        let tree = retire_tree(&rig, &repo, name);
        let pane = seat(
            &rig,
            &repo,
            "worker",
            0,
            if kind == "by-name" { None } else { Some(&tree) },
        );
        if kind == "unmerged" {
            git_in(
                &tree,
                &["commit", "-q", "--allow-empty", "-m", "not landed"],
            );
        } else {
            fs::write(tree.join("a.txt"), "uncommitted work").expect("dirty work");
        }
        refused_untouched(&rig, &main, "worker", "spawned.0");
        refused_untouched(&rig, &main, &pane, "spawned.0");
        assert!(tree.exists());
    }
}

#[test]
fn each_coowner_of_a_dirty_worktree_must_resolve_cleanup_before_retirement() {
    let (rig, repo, main) = retire_rig("coowners");
    let tree = retire_tree(&rig, &repo, "tree");
    seat(&rig, &repo, "alpha", 0, Some(&tree));
    seat(&rig, &repo, "beta", 1, Some(&tree));
    fs::write(tree.join("a.txt"), "shared uncommitted work").expect("dirty work");
    refused_untouched(&rig, &main, "alpha", "spawned.0");
    refused_untouched(&rig, &main, "beta", "spawned.1");
}

#[test]
fn a_sessions_own_dirty_linked_worktree_is_never_owned_by_an_inheriting_seat() {
    for mode in ["git", "worktree"] {
        let (rig, repo, main) = retire_rig(&format!("own-dir-{mode}"));
        // The leaf and branch also match the seat, exercising the session-dir exclusion
        // ahead of both ownership heuristics. The seat has no work_dir.<slot> row.
        let tree = retire_tree(&rig, &repo, "worker");
        let meta = rig.dir(SESSION).join("meta");
        let original = fs::read_to_string(&meta).expect("session meta");
        let revised = original
            .replace("mode=local\n", &format!("mode={mode}\n"))
            .replace(
                &format!("work_dir={}\n", repo.display()),
                &format!("work_dir={}\n", tree.display()),
            );
        fs::write(&meta, revised).expect("linked session work dir");
        seat(&rig, &tree, "worker", 0, None);
        fs::write(tree.join("a.txt"), "session work retained").expect("dirty session work");
        let result = rig.helper(SESSION, "retire", &["worker"], Some(&main));
        assert_eq!(
            result.0,
            Some(0),
            "session work dir trapped retire mode {mode}: {result:?}"
        );
        assert!(
            !fs::read_to_string(meta)
                .expect("retired seat metadata")
                .contains("seat.spawned.0=worker"),
            "seat remained mode {mode}"
        );
        assert_eq!(
            fs::read_to_string(tree.join("a.txt")).expect("session work retained"),
            "session work retained"
        );
    }
}

#[test]
fn clean_merged_retirement_warns_and_the_override_preserves_cleanup_actor_and_audit_note() {
    for override_dirty in [false, true] {
        let (rig, repo, main) = retire_rig(if override_dirty {
            "override"
        } else {
            "clean-retire"
        });
        let tree = retire_tree(&rig, &repo, "tree");
        seat(&rig, &repo, "worker", 0, Some(&tree));
        if override_dirty {
            fs::write(tree.join("a.txt"), "keep this edit").expect("dirty work");
        }
        let args = if override_dirty {
            vec!["worker", "--keep-worktree", "lead assumes cleanup"]
        } else {
            vec!["worker"]
        };
        let result = rig.helper(SESSION, "retire", &args, Some(&main));
        assert_eq!(result.0, Some(0), "{result:?}");
        assert!(tree.exists(), "retire removed a worktree");
        let meta = fs::read_to_string(rig.dir(SESSION).join("meta")).expect("meta");
        assert!(!meta.contains("seat.spawned.0=worker"), "seat remained");
        let journal =
            fs::read_to_string(rig.dir(SESSION).join("events.jsonl")).expect("retire journal");
        let line = journal
            .lines()
            .find(|line| line.contains("\"action\":\"retire\""))
            .expect("retire event");
        let event = ae::events::Event::parse_line(line).expect("valid retire event");
        assert_eq!(event.actor, "lead", "cleanup actor changed");
        if override_dirty {
            assert!(
                line.contains("\"kept_worktree\":") && line.contains(text(&tree)),
                "cleanup audit missing: {line}"
            );
            assert!(
                line.contains("lead assumes cleanup"),
                "reason missing: {line}"
            );
            assert_eq!(
                fs::read_to_string(tree.join("a.txt")).expect("work kept"),
                "keep this edit"
            );
        } else {
            assert!(
                result.2.contains("note:") && result.2.contains(text(&tree)),
                "silent retirement: {result:?}"
            );
        }
    }
}

#[test]
fn keep_worktree_after_a_reason_word_is_text_and_cannot_bypass_the_guard() {
    let (rig, repo, main) = retire_rig("reason-text");
    let tree = retire_tree(&rig, &repo, "tree");
    seat(&rig, &repo, "worker", 0, Some(&tree));
    fs::write(tree.join("a.txt"), "keep edit").expect("dirty work");
    let result = rig.helper(
        SESSION,
        "retire",
        &["worker", "reason", "--keep-worktree"],
        Some(&main),
    );
    assert_eq!(
        result.0,
        Some(1),
        "a reason word became an override: {result:?}"
    );
    assert!(
        fs::read_to_string(rig.dir(SESSION).join("meta"))
            .expect("meta")
            .contains("seat.spawned.0=worker")
    );
}

#[test]
fn unknown_git_facts_allow_recovery_retirement_but_require_a_visible_note() {
    let (rig, repo, main) = retire_rig("retire-unknown");
    let tree = retire_tree(&rig, &repo, "tree");
    seat(&rig, &repo, "worker", 0, Some(&tree));
    let mut command = rig.command(Some(&main));
    command.args(["@wtgcsession", "retire", "worker"]);
    let marker = interpose(&mut command, &rig.scratch, "fail-all", &repo, &tree);
    let result = run(command);
    assert!(marker.exists(), "git failure fixture did not apply");
    assert_eq!(result.0, Some(0), "recovery was trapped: {result:?}");
    assert!(
        result.2.contains("note:") && result.2.contains("not checked"),
        "silent unknown worktree facts: {result:?}"
    );
    assert!(tree.exists(), "retire deleted worktree data");
}

#[test]
fn paths_with_spaces_remain_single_operands_and_terminal_controls_are_inert() {
    let world = World::new("space");
    let tree = world.scratch.join("tree with spaces");
    git_in(
        &world.repo,
        &["worktree", "add", "-q", "-b", "spaced", text(&tree)],
    );
    let tree = fs::canonicalize(tree).expect("physical path");
    age(&tree, 3);
    let listing = world.list();
    assert!(removable(&listing, &tree), "space path lost: {listing}");
    let result = world.gc(&["--remove", &token(&listing)]);
    assert_eq!(result.0, Some(0), "{result:?}");
    assert!(!tree.exists(), "space path was split or quoted incorrectly");

    let world = World::new("terminal");
    let control = world.tree("control");
    let tree = world.tree("subject");
    fs::write(tree.join("backup\u{1b}[31m.bak"), "evidence").expect("hostile ignored filename");
    let listing = world.list();
    assert!(removable(&listing, &control), "{listing}");
    assert!(
        !removable(&listing, &tree),
        "ignored data admitted: {listing}"
    );
    assert!(
        !listing
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t'),
        "terminal control reached output"
    );
}

#[test]
fn a_gitlink_index_entry_cannot_pass_the_clean_proof() {
    let world = World::new("gitlink");
    let commit = git_in(&world.repo, &["rev-parse", "HEAD"]);
    git_in(
        &world.repo,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{commit},vendor"),
        ],
    );
    git_in(&world.repo, &["commit", "-q", "-m", "gitlink"]);
    let tree = world.tree("subject");
    assert!(
        !removable(&world.list(), &tree),
        "unsupported gitlink was removable"
    );
}

// Review regressions: requirements R1/R2/R2a/R4/R5/R6; the existing frozen v2 cases stay intact.
#[test]
fn an_absent_meta_in_a_durable_session_directory_makes_all_owners_unknown() {
    let world = World::new("absent-meta");
    let tree = world.tree("subject");
    let dir = world.session("metaless", "worker", Some(&tree));
    fs::remove_file(dir.join("meta")).expect("remove only fixture meta");
    let listing = world.list();
    assert!(
        !removable(&listing, &tree),
        "owner facts were lost: {listing}"
    );
    assert!(row(&listing, &tree).contains("owners unknown"), "{listing}");
    let footer = listing
        .lines()
        .find(|line| line.starts_with("Owners unknown:"))
        .expect("owners gap footer");
    assert!(
        footer.contains("'metaless'"),
        "offending session not named: {listing}"
    );
}

#[test]
fn a_noncanonical_vanished_owner_row_cannot_use_the_lexical_absence_exception() {
    let world = World::new("parent-owner");
    let tree = world.tree("subject");
    let outside = world.scratch.join("outside");
    fs::create_dir(&outside).expect("outside directory");
    let recorded = outside.join("..").join("subject").join("vanished");
    assert!(!recorded.exists(), "vanished fixture exists");
    world.session("damaged", "worker", Some(&recorded));
    let listing = world.list();
    assert!(
        !removable(&listing, &tree),
        "noncanonical owner disappeared: {listing}"
    );
    assert!(row(&listing, &tree).contains("owners unknown"), "{listing}");
    assert!(
        listing.contains("damaged"),
        "offending session not named: {listing}"
    );
}

#[test]
fn the_final_proof_is_bound_to_the_listed_head() {
    final_proof_identity("proof-head");
}

#[test]
fn the_final_proof_is_bound_to_the_listed_branch() {
    final_proof_identity("proof-branch");
}

#[test]
fn the_final_proof_is_bound_to_the_listed_target() {
    final_proof_identity("proof-target");
}

fn final_proof_identity(mode: &str) {
    // The approved plan pins HEAD with the existing git leg's Head shape,
    // `-C <tree> rev-parse HEAD`. The marker proves this injection reached it.
    let world = World::new(mode);
    let tree = world.tree("subject");
    let original = git_in(&world.repo, &["rev-parse", "HEAD"]);
    let replacement = if mode == "proof-head" {
        git_in(&tree, &["commit", "-q", "--allow-empty", "-m", "unmerged"]);
        let changed = git_in(&tree, &["rev-parse", "HEAD"]);
        git_in(
            &world.repo,
            &["update-ref", "refs/heads/subject", &original],
        );
        age(&tree, 3);
        changed
    } else {
        original.clone()
    };
    git_in(&world.repo, &["branch", "replacement", &replacement]);
    let listing = world.list();
    assert!(
        removable(&listing, &tree),
        "positive pre-mutation control: {listing}"
    );
    let mut command = world.command(&world.repo);
    command.args(["worktree", "gc", "--remove", &token(&listing)]);
    let marker = interpose(&mut command, &world.scratch, mode, &world.repo, &tree);
    let result = run(command);
    assert!(
        marker.exists(),
        "proof mutation did not apply mode {mode}: {result:?}"
    );
    assert_eq!(
        result.0,
        Some(1),
        "proof used stale identity mode {mode}: {result:?}"
    );
    assert!(
        tree.exists(),
        "changed identity deleted mode {mode}: {result:?}"
    );
    if mode == "proof-head" {
        assert_eq!(
            git_in(&tree, &["rev-list", "--count", "main..HEAD"]),
            "1",
            "HEAD did not become unmerged"
        );
    } else {
        assert_eq!(
            git_in(&tree, &["rev-parse", "HEAD"]),
            original,
            "branch/ref control also moved HEAD"
        );
    }
}

#[test]
fn a_selective_merge_failure_is_unknown_and_does_not_trap_retire() {
    selective_failure("fail-merge");
}

#[test]
fn a_selective_diff_failure_is_unknown_and_does_not_trap_retire() {
    selective_failure("fail-diff");
}

fn selective_failure(mode: &str) {
    let (rig, repo, main) = retire_rig(mode);
    let tree = retire_tree(&rig, &repo, "tree");
    seat(&rig, &repo, "worker", 0, Some(&tree));
    let mut command = rig.command(Some(&main));
    command.args(["@wtgcsession", "retire", "worker"]);
    let marker = interpose(&mut command, &rig.scratch, mode, &repo, &tree);
    let result = run(command);
    assert!(
        marker.exists(),
        "selective failure did not apply mode {mode}: {result:?}"
    );
    assert_eq!(
        result.0,
        Some(0),
        "unknown facts trapped retirement mode {mode}: {result:?}"
    );
    assert!(
        result.2.contains("note:") && result.2.contains("not checked"),
        "unknown facts were silent mode {mode}: {result:?}"
    );
    assert!(tree.join("a.txt").exists(), "retire removed the worktree");
}

#[test]
fn a_trailing_space_worktree_cannot_disappear_from_the_retire_guard_silently() {
    let (rig, repo, main) = retire_rig("space-retire");
    let tree = rig.scratch.join("tree ");
    git_in(&repo, &["worktree", "add", "-q", "-b", "tree", text(&tree)]);
    let tree = fs::canonicalize(tree).expect("trailing-space worktree");
    seat(&rig, &repo, "worker", 0, Some(&tree));
    fs::write(tree.join("a.txt"), "work retained").expect("dirty owned tree");
    let result = rig.helper(SESSION, "retire", &["worker"], Some(&main));
    // Either preserve the exact path and refuse on proven dirt, or declare the listing
    // unproven and permit recovery with a visible R6 note. Silent retirement is forbidden.
    match result.0 {
        Some(0) => assert!(
            result.2.contains("note:") && result.2.contains("not checked"),
            "unknown worktree silently dropped: {result:?}"
        ),
        Some(1) => assert!(
            result.2.contains("still owns worktree") && result.2.contains(text(&tree)),
            "unrelated refusal did not test the guard: {result:?}"
        ),
        _ => panic!("unexpected retire result: {result:?}"),
    }
    assert_eq!(
        fs::read_to_string(tree.join("a.txt")).expect("work kept"),
        "work retained"
    );
}

#[test]
fn an_empty_post_remove_listing_cannot_prove_registration_absence() {
    post_remove_listing("post-empty");
}

#[test]
fn an_unreadable_post_remove_listing_cannot_prove_registration_absence() {
    post_remove_listing("post-malformed");
}

fn post_remove_listing(mode: &str) {
    let world = World::new(mode);
    let tree = world.tree("subject");
    let confirm = token(&world.list());
    let mut command = world.command(&world.repo);
    command.args(["worktree", "gc", "--remove", &confirm]);
    let marker = interpose(&mut command, &world.scratch, mode, &world.repo, &tree);
    let result = run(command);
    assert!(
        marker.exists(),
        "post-remove injection did not apply mode {mode}: {result:?}"
    );
    assert!(
        !tree.exists(),
        "real removal did not apply mode {mode}: {result:?}"
    );
    assert_eq!(
        result.0,
        Some(1),
        "unproven registration absence reported success mode {mode}: {result:?}"
    );
    assert!(
        format!("{}{}", result.1, result.2).contains("state unknown"),
        "missing unknown-state report mode {mode}: {result:?}"
    );
}
