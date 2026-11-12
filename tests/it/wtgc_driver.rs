//! Driver regressions for issue 275 beside the frozen `wtgc_spec`: real git, real ae, scratch
//! state only. Oracles: lead ruling ae-20261112T163456Z-40c1f3f6 on R2a — only an exactly canonical
//! absolute recorded `work_dir` row takes part in gc ownership; any other row makes the owner facts
//! unknown, whether its directory exists or not, and the footer names its session. The mutation
//! regressions further down pin the documented proof rules (docs/reference/contract.md, key
//! invariants): the clean proof holds only for the listed HEAD, an index that did not move under
//! it and queries that succeeded; a merge or clean fact git cannot establish never blocks
//! retirement; only a whole post-remove listing proves the registration gone; and the owner facts
//! of a canonical meta stay whole when the state root is reached through a symlink.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "driver fixtures create and inspect owned scratch repos and fail on setup errors"
)]

use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::cli::{OwnedScratch, Runner, ae, bounded, git_in};
use super::renametell_routing_spec::Rig;

type Outcome = (Option<i32>, String, String);

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

    /// `ae worktree gc <args>` from the repo, not yet run.
    fn command(&self, args: &[&str]) -> Runner {
        let mut command = ae();
        command
            .current_dir(&self.repo)
            .env("PWD", &self.repo)
            .env("HOME", &self.scratch)
            .env("AE_HOME", self.scratch.join("state"))
            .env("CONFIG_FILE", self.scratch.join("config"))
            .args(["worktree", "gc"])
            .args(args);
        command
    }

    /// `ae worktree gc` (dry run) from the repo: exit code and stdout.
    fn list(&self) -> (Option<i32>, String) {
        let (code, stdout, _) = run(self.command(&[]));
        (code, stdout)
    }

    /// The listing row for the subject worktree.
    fn row<'a>(&self, listing: &'a str) -> &'a str {
        let path = self.tree.display().to_string();
        listing
            .lines()
            .find(|line| line.trim_end().ends_with(&path))
            .unwrap_or_else(|| panic!("no row for {path}:\n{listing}"))
    }

    /// Whether the subject row's verdict is `remove`.
    fn removable(&self, listing: &str) -> bool {
        self.row(listing).split_whitespace().next() == Some("remove")
    }

    /// Make the subject a removal candidate: a three-day-old HEAD reflog.
    fn age_subject(&self) {
        age(&self.tree, 3);
        let (code, listing) = self.list();
        assert_eq!(code, Some(0), "{listing}");
        assert!(
            self.removable(&listing),
            "positive control: the aged subject must be removable before any injection: {listing}"
        );
    }
}

fn run(mut command: Runner) -> Outcome {
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

/// Backdate every entry of the worktree's own HEAD reflog by `days`.
fn age(tree: &Path, days: u64) {
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current epoch")
        .as_secs()
        - days * 86_400
        - 60;
    let path = PathBuf::from(git_in(tree, &["rev-parse", "--git-path", "logs/HEAD"]));
    let path = if path.is_absolute() {
        path
    } else {
        tree.join(path)
    };
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

/// The confirmation token a dry run prints.
fn token(listing: &str) -> String {
    listing
        .lines()
        .find_map(|line| line.split_once("--remove ").map(|(_, rest)| rest))
        .and_then(|rest| rest.split_whitespace().next())
        .expect("confirmation token in dry run")
        .to_owned()
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

// A git interposer is an external dependency fixture: every query reaches real git, and one mode
// changes the world or the answer at one named point. A marker proves the injection happened.
const DRIVER_GIT: &str = r#"#!/usr/bin/perl
use strict;
use warnings;
my $real = $ENV{WTGCD_REAL_GIT} or die 'no real git';
my $mode = $ENV{WTGCD_MODE} // '';
my $subject = $ENV{WTGCD_SUBJECT} // '';
my $marker = $ENV{WTGCD_MARKER} or die 'no marker';
my $args = join(' ', @ARGV);
sub mark {
    open(my $file, '>', $marker) or die $!;
    print $file "applied\n";
    close $file;
}
for my $arg (@ARGV) {
    die 'forbidden destructive argv' if $arg eq '--force' || $arg eq '-D' || $arg eq 'prune' || $arg eq 'branch';
}
if ($mode eq 'fail-ancestry' && grep { $_ eq 'merge-base' } @ARGV) { mark(); exit 128; }
if ($mode eq 'fail-quiet-diff' && grep({ $_ eq 'diff' } @ARGV) && grep({ $_ eq '--quiet' } @ARGV)) {
    mark();
    exit 128;
}
if ($mode eq 'fail-final-head' && $args eq "-C $subject rev-parse HEAD") {
    my $count = 0;
    if (open(my $old, '<', "$marker.count")) { $count = <$old>; close $old; }
    $count += 1;
    open(my $new, '>', "$marker.count") or die $!;
    print $new $count;
    close $new;
    if ($count == 2) {
        open(my $pipe, '-|', $real, @ARGV) or die $!;
        my $out = do { local $/; <$pipe> };
        close $pipe or die 'real rev-parse failed';
        print $out;
        mark();
        exit 128;
    }
}
if ($mode eq 'move-head' && !-e $marker && $args eq "-C $subject rev-parse HEAD") {
    system($real, '-C', $ENV{WTGCD_REPO}, 'update-ref', 'refs/heads/subject', $ENV{WTGCD_REPLACEMENT}) == 0 or die 'move HEAD';
    mark();
}
if ($mode eq 'stage-late' && !-e $marker && $args =~ /\A-C \Q$subject\E .*\bhash-object --no-filters --/) {
    open(my $file, '>', "$subject/late.txt") or die $!;
    print $file "staged while the proof ran\n";
    close $file;
    system($real, '-C', $subject, 'update-index', '--add', 'late.txt') == 0 or die 'stage late file';
    mark();
}
if ($mode =~ /\Apost-/) {
    if ($args =~ /\bworktree remove\b/) {
        system($real, @ARGV) == 0 or die 'real remove failed';
        mark();
        exit 0;
    }
    if (-e $marker && $args =~ /\bworktree list --porcelain\b/) {
        open(my $pipe, '-|', $real, @ARGV) or die $!;
        my $listing = do { local $/; <$pipe> };
        close $pipe or die 'real listing failed';
        if ($mode eq 'post-cut') {
            $listing =~ s/\n\z// or die 'listing was not terminated';
        } elsif ($mode eq 'post-foreign-root') {
            $listing =~ s/\Aworktree [^\n]*/worktree \/elsewhere/ or die 'no main record';
        } elsif ($mode eq 'post-unknown') {
            $listing .= "worktree /elsewhere\nHEAD broken\n\n";
        } else {
            die "unknown mode $mode";
        }
        print $listing;
        exit 0;
    }
}
exec $real, @ARGV;
die $!;
"#;

/// Put the interposer first on `command`'s PATH in `mode`; `subject` is the worktree the mode acts
/// on, `repo` the main worktree. Returns the marker path.
fn interpose(
    command: &mut Runner,
    root: &Path,
    mode: &str,
    subject: &Path,
    repo: &Path,
    replacement: &str,
) -> PathBuf {
    let bin = root.join("git-bin");
    fs::create_dir_all(&bin).expect("git shim directory");
    fs::write(bin.join("git"), DRIVER_GIT).expect("git external fixture");
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
    command
        .env("PATH", path)
        .env("WTGCD_REAL_GIT", real)
        .env("WTGCD_MODE", mode)
        .env("WTGCD_MARKER", &marker)
        .env("WTGCD_SUBJECT", subject)
        .env("WTGCD_REPO", repo)
        .env("WTGCD_REPLACEMENT", replacement);
    marker
}

#[test]
fn a_head_that_moves_before_the_proof_reads_it_keeps_the_tree() {
    // The listed HEAD is merged and clean; a commit with the same tree replaces it after the
    // listing. A proof of the new commit says nothing about the listed one.
    let world = World::new("move-head");
    let listed = git_in(&world.tree, &["rev-parse", "HEAD"]);
    git_in(
        &world.tree,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "same tree, not listed",
        ],
    );
    let moved = git_in(&world.tree, &["rev-parse", "HEAD"]);
    git_in(&world.repo, &["update-ref", "refs/heads/subject", &listed]);
    world.age_subject();
    let mut command = world.command(&[]);
    let marker = interpose(
        &mut command,
        &world.scratch,
        "move-head",
        &world.tree,
        &world.repo,
        &moved,
    );
    let (code, listing, stderr) = run(command);
    assert!(marker.exists(), "HEAD was not moved: {listing}{stderr}");
    assert_eq!(git_in(&world.tree, &["rev-parse", "HEAD"]), moved);
    assert_eq!(code, Some(0), "{listing}{stderr}");
    assert!(
        !world.removable(&listing),
        "a proof of another commit made the listed HEAD removable: {listing}"
    );
}

#[test]
fn a_final_head_query_that_fails_keeps_the_tree_whatever_it_printed() {
    // The proof's second HEAD read prints the listed HEAD and then fails: a failed query proves
    // nothing, so the tree is kept even though its output matches.
    let world = World::new("fail-final-head");
    world.age_subject();
    let mut command = world.command(&[]);
    let marker = interpose(
        &mut command,
        &world.scratch,
        "fail-final-head",
        &world.tree,
        &world.repo,
        "",
    );
    let (code, listing, stderr) = run(command);
    assert!(
        marker.exists(),
        "the final HEAD read did not fail: {listing}{stderr}"
    );
    assert_eq!(code, Some(0), "{listing}{stderr}");
    assert!(
        !world.removable(&listing),
        "a failed final HEAD read counted as proof: {listing}"
    );
}

#[test]
fn an_aliased_state_root_keeps_canonical_owner_facts_whole() {
    // AE_HOME reached through a symlink: the scan reports the state dirs under the alias, which
    // resolve elsewhere. A canonical meta there must not turn into an owners gap.
    let world = World::new("alias-root");
    world.age_subject();
    world.session("alias", &world.repo);
    let (code, listing) = world.list();
    assert_eq!(code, Some(0), "{listing}");
    assert!(
        world.removable(&listing) && !listing.contains("Owners unknown:"),
        "positive control: a canonical meta under the real root keeps the facts whole: {listing}"
    );
    let alias = world.scratch.join("state-alias");
    symlink(world.scratch.join("state"), &alias).expect("aliased state root");
    let mut command = world.command(&[]);
    command.env("AE_HOME", &alias);
    let (code, listing, stderr) = run(command);
    assert_eq!(code, Some(0), "{listing}{stderr}");
    assert!(
        !listing.contains("Owners unknown:"),
        "an aliased state root made the owner facts unknown: {listing}"
    );
    assert!(
        world.removable(&listing),
        "an aliased state root kept an unowned eligible tree: {listing}"
    );
}

#[test]
fn an_index_that_changes_while_the_proof_runs_keeps_the_tree() {
    // A file staged between the proof's two index reads is work the proof never covered.
    let world = World::new("stage-late");
    world.age_subject();
    let mut command = world.command(&[]);
    let marker = interpose(
        &mut command,
        &world.scratch,
        "stage-late",
        &world.tree,
        &world.repo,
        "",
    );
    let (code, listing, stderr) = run(command);
    assert!(marker.exists(), "nothing was staged: {listing}{stderr}");
    assert_eq!(
        git_in(&world.tree, &["diff", "--cached", "--name-only"]),
        "late.txt",
        "the late file must be staged and nothing else"
    );
    assert_eq!(code, Some(0), "{listing}{stderr}");
    assert!(
        !world.removable(&listing),
        "an index change during the proof was ignored: {listing}"
    );
}

/// Remove the aged subject with its token while `mode` rewrites the one listing git returns after
/// the removal; that listing is not whole, so the result must be `state unknown`.
fn post_remove_listing_is_not_proof(mode: &str) {
    let world = World::new(mode);
    world.age_subject();
    let (_, listing) = world.list();
    let confirm = token(&listing);
    let mut command = world.command(&["--remove", &confirm]);
    let marker = interpose(
        &mut command,
        &world.scratch,
        mode,
        &world.tree,
        &world.repo,
        "",
    );
    let (code, stdout, stderr) = run(command);
    assert!(
        marker.exists(),
        "the real removal did not run mode {mode}: {stdout}{stderr}"
    );
    assert!(
        !world.tree.exists(),
        "the real removal did not apply mode {mode}"
    );
    assert_eq!(
        code,
        Some(1),
        "a listing that is not whole proved the registration gone, mode {mode}: {stdout}{stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("state unknown"),
        "missing unknown-state report mode {mode}: {stdout}{stderr}"
    );
}

#[test]
fn a_post_remove_listing_cut_before_its_last_blank_line_is_not_proof() {
    post_remove_listing_is_not_proof("post-cut");
}

#[test]
fn a_terminated_post_remove_listing_under_another_root_is_not_proof() {
    post_remove_listing_is_not_proof("post-foreign-root");
}

#[test]
fn a_terminated_post_remove_listing_with_an_unknown_entry_is_not_proof() {
    post_remove_listing_is_not_proof("post-unknown");
}

const SESSION: &str = "wtgcdriver";
const SESSION_ID: &str = "27500000-aaaa-bbbb-cccc-000000000002";

#[test]
fn a_failed_ancestry_check_over_a_merged_head_does_not_trap_retire() {
    // `merge-base --is-ancestor` fails; the count of commits the target lacks is a successful 0.
    // Nothing proves the worktree unmerged, so the guard notes the gap and retires.
    retire_notes_an_unproven_fact("ancestry", "fail-ancestry");
}

#[test]
fn a_failed_quiet_diff_without_a_named_change_does_not_trap_retire() {
    // Both `diff --quiet` calls fail; asked again with `--name-only`, git names no path. Nothing
    // proves the worktree dirty, so the guard notes the gap and retires.
    retire_notes_an_unproven_fact("quiet-diff", "fail-quiet-diff");
}

/// Retire the seat `worker`, whose worktree is clean and merged into `main`, while `mode` makes one
/// git fact unprovable, and require a retirement with a visible note instead of a refusal.
fn retire_notes_an_unproven_fact(tag: &str, mode: &str) {
    let rig = Rig::new(&format!("wtgcdrv-{tag}"));
    let repo = rig.scratch.join("repo");
    fs::create_dir_all(&repo).expect("retire repo");
    git_in(&repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join("a.txt"), "constant").expect("tracked file");
    git_in(&repo, &["add", "."]);
    git_in(&repo, &["commit", "-q", "-m", "base"]);
    let repo = fs::canonicalize(repo).expect("canonical repo");
    let [main, _] = rig.create(SESSION, SESSION_ID);
    let meta = rig.dir(SESSION).join("meta");
    let rows = fs::read_to_string(&meta).expect("session meta");
    let rows: String = rows
        .lines()
        .map(|line| match line.split_once('=') {
            Some((key @ ("work_dir" | "origin"), _)) => format!("{key}={}\n", repo.display()),
            _ => format!("{line}\n"),
        })
        .collect();
    let tree = rig.scratch.join("tree");
    git_in(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "tree",
            tree.to_str().expect("UTF-8 fixture path"),
        ],
    );
    let tree = fs::canonicalize(tree).expect("canonical tree");
    assert_eq!(
        git_in(&tree, &["rev-list", "--count", "main..HEAD"]),
        "0",
        "the worktree HEAD must be merged"
    );
    let slot = "spawned.0";
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
            "worker",
            "-c",
            repo.to_str().expect("UTF-8 fixture path"),
            "sleep 120",
        ])
        .trim()
        .to_owned();
    rig.tmux(&["set-option", "-p", "-t", &pane, "@ae_slot", slot]);
    rig.tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", "worker"]);
    fs::write(
        &meta,
        format!(
            "{rows}seat.{slot}=worker\nprofile.{slot}=fake\nagent_bin.{slot}=claude\nharness_session.{slot}=pending\nlaunch_id.{slot}=wtgcdrv-0\npane.{slot}={pane}\nwork_dir.{slot}={}\n",
            tree.display()
        ),
    )
    .expect("spawned seat shape");
    let target = format!("@{SESSION}");
    let mut command = rig.command(Some(&main));
    command.args([target.as_str(), "retire", "worker"]);
    let marker = interpose(&mut command, &rig.scratch, mode, &tree, &repo, "");
    let (code, stdout, stderr) = run(command);
    assert!(
        marker.exists(),
        "the injection did not apply mode {mode}: {stdout}{stderr}"
    );
    assert_eq!(
        code,
        Some(0),
        "an unestablished git fact trapped retirement mode {mode}: {stdout}{stderr}"
    );
    assert!(
        !stderr.contains("still owns worktree"),
        "the guard refused on an unproven fact mode {mode}: {stderr}"
    );
    assert!(
        stderr.contains("note:") && stderr.contains("not checked"),
        "the unproven fact was silent mode {mode}: {stderr}"
    );
    assert!(
        tree.join("a.txt").exists(),
        "retire removed the worktree mode {mode}"
    );
}
