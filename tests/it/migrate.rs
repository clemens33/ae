//! The migration chain, and the upgrade sweep that runs it over every session.
//!
//! Three arms, because the contract has three halves that cannot be proven the
//! same way:
//!
//! * the VERSION-DIRECTORY sweep and the chain's refusal are library facts over
//!   real directories — no process and no tmux needed;
//! * the STOPPED-session sweep is black-box through `ae _install --from`,
//!   because what is being proven is that a real publish repoints real sessions
//!   before it repoints the command link;
//! * the RUNNING-session daemon restart needs a real tmux server, for the same
//!   reason [`super::daemons`] does: what can go wrong is what the guards make
//!   of a live server's answers.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::parity::Invocation;
use super::parity::capture::raw;
use super::phase2::{run_tmux, tmux_present};

/// A fixture `$HOME`: `<home>/.ae` is the state root, `<home>/.local/bin/ae`
/// the command link — the two paths a publish derives.
struct Rig {
    scratch: PathBuf,
    home: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let scratch = PathBuf::from(format!("/tmp/aemig.{}.{tag}", std::process::id()));
        let _ = remove(&scratch);
        let home = scratch.join("home");
        assert!(fs::create_dir_all(&home).is_ok(), "a fixture home");
        Self { scratch, home }
    }

    fn root(&self) -> PathBuf {
        self.home.join(".ae")
    }

    fn versions(&self) -> PathBuf {
        self.root().join("versions")
    }

    fn link(&self) -> PathBuf {
        self.home.join(".local").join("bin").join("ae")
    }

    /// A bundle whose core reports `version` — the three members `just bundle`
    /// packages, with the manifest both `sha256sum` spellings accept.
    fn bundle(&self, version: &str) -> PathBuf {
        let dir = self.scratch.join(format!("ae-{version}-fixture"));
        assert!(fs::create_dir_all(&dir).is_ok(), "a bundle root");
        write_exec(
            &dir.join("ae-core"),
            &format!("#!/bin/sh\necho \"ae {version}\"\n"),
        );
        write_exec(&dir.join("install"), "#!/bin/sh\necho bootstrap\n");
        let mut manifest = String::new();
        for name in ["ae-core", "install"] {
            let bytes = fs::read(dir.join(name))
                .unwrap_or_else(|why| panic!("{name} should be readable: {why}"));
            let _ = writeln!(manifest, "{}  {name}", ae::install::sha256_hex(&bytes));
        }
        let path = dir.join("SHA256SUMS");
        let _ = fs::remove_file(&path);
        assert!(fs::write(&path, manifest).is_ok(), "a manifest");
        dir
    }

    /// A stopped session recording `core` and declaring `version_row`.
    fn session(&self, name: &str, core: &str, version_row: Option<u32>) -> PathBuf {
        let dir = self.root().join("sessions").join(name);
        assert!(fs::create_dir_all(&dir).is_ok(), "a session dir");
        let mut meta = String::new();
        if let Some(version) = version_row {
            let _ = writeln!(meta, "{}={version}", ae::migrate::KEY);
        }
        let _ = write!(
            meta,
            "mode=local\nsession={name}\nwork_dir=/w\nae_version=0.0.1\n\
             ae_core={core}\nae_core_version=0.0.1\n\
             schema=2\nseat.main=lead\nprofile.main=cl\n"
        );
        assert!(fs::write(dir.join("meta"), meta).is_ok(), "a meta");
        dir
    }

    /// A session in the v1 shape: no `meta_version`, no `schema=2`,
    /// and the `agent.<slot>` rows v2 replaced. This is the only meta the chain
    /// cannot place, and the rig has to be able to build one because
    /// `session()` above writes `schema=2` — as every real session does.
    fn legacy_session(&self, name: &str) -> PathBuf {
        let dir = self.root().join("sessions").join(name);
        assert!(fs::create_dir_all(&dir).is_ok(), "a session dir");
        let mut meta = String::new();
        let _ = write!(
            meta,
            "mode=local\nsession={name}\nwork_dir=/w\nae_version=0.0.1\n\
             ae_core=/nowhere/ae-core\nagent.main=lead:lead:\n"
        );
        assert!(fs::write(dir.join("meta"), meta).is_ok(), "a v1 meta");
        dir
    }

    /// A stopped session whose declared version row cannot be parsed.
    fn unreadable_session(&self, name: &str) -> PathBuf {
        let dir = self.session(name, "/nowhere/ae-core", Some(ae::migrate::CURRENT));
        let text = meta_of(&dir).replacen(
            &format!("{}={}\n", ae::migrate::KEY, ae::migrate::CURRENT),
            &format!("{}=v{}\n", ae::migrate::KEY, ae::migrate::CURRENT),
            1,
        );
        assert!(
            fs::write(dir.join("meta"), text).is_ok(),
            "an unreadable meta"
        );
        dir
    }

    /// A session whose `meta` node cannot be read as a file.
    fn unreadable_meta_session(&self, name: &str) -> PathBuf {
        let dir = self.root().join("sessions").join(name);
        assert!(
            fs::create_dir_all(dir.join("meta")).is_ok(),
            "an unreadable meta node"
        );
        dir
    }

    /// A version directory with one member, standing in for a published one.
    fn plant_version(&self, version: &str) -> PathBuf {
        let dir = self.versions().join(version);
        assert!(fs::create_dir_all(&dir).is_ok(), "a version dir");
        assert!(fs::write(dir.join("ae-core"), "old\n").is_ok(), "a core");
        dir
    }

    fn install(&self, from: &Path) -> (Option<i32>, String, String) {
        self.install_inner(from, None, None)
    }

    fn install_with_tmux_tmpdir(
        &self,
        from: &Path,
        tmux_tmpdir: Option<&Path>,
    ) -> (Option<i32>, String, String) {
        self.install_inner(from, tmux_tmpdir, None)
    }

    fn install_with_caller(
        &self,
        from: &Path,
        tmux_tmpdir: &Path,
        marker: &str,
        pane: &str,
    ) -> (Option<i32>, String, String) {
        self.install_inner(from, Some(tmux_tmpdir), Some((marker, pane)))
    }

    fn install_inner(
        &self,
        from: &Path,
        tmux_tmpdir: Option<&Path>,
        caller: Option<(&str, &str)>,
    ) -> (Option<i32>, String, String) {
        #[allow(
            clippy::disallowed_types,
            reason = "the black-box door: a publish is what a real process does to a real HOME"
        )]
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_ae"));
        command
            .env_remove("AE_HOME")
            .env_remove("CONFIG_FILE")
            .env_remove("AE_VERSION")
            .env("AE_NO_AUTOSTART", "1")
            .env("HOME", &self.home)
            .args(["_install", "--from", &from.to_string_lossy()]);
        if let Some((marker, pane)) = caller {
            command.env("TMUX", marker).env("TMUX_PANE", pane);
        } else {
            command.env_remove("TMUX").env_remove("TMUX_PANE");
        }
        if let Some(tmux_tmpdir) = tmux_tmpdir {
            command.env("TMUX_TMPDIR", tmux_tmpdir);
        }
        let out = command
            .output()
            .unwrap_or_else(|why| panic!("the product binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = remove(&self.scratch);
    }
}

fn write_exec(path: &Path, text: &str) {
    let _ = fs::remove_file(path);
    assert!(fs::write(path, text).is_ok(), "a script");
    assert!(
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).is_ok(),
        "an executable script"
    );
}

/// Remove a tree whose members may be 0555 — the mode a publish leaves.
fn remove(path: &Path) -> std::io::Result<()> {
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let _ = fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o755));
            if entry.path().is_dir() {
                let _ = remove(&entry.path());
            }
        }
    }
    fs::remove_dir_all(path)
}

fn meta_of(dir: &Path) -> String {
    fs::read_to_string(dir.join("meta"))
        .unwrap_or_else(|why| panic!("{}: {why}", dir.join("meta").display()))
}

fn value_of(meta: &str, key: &str) -> Option<String> {
    meta.lines()
        .filter_map(|line| line.split_once('='))
        .find(|(found, _)| *found == key)
        .map(|(_, value)| value.to_owned())
}

fn present(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SnapshotEntry {
    Directory(PathBuf),
    File(PathBuf, Vec<u8>),
    Symlink(PathBuf, PathBuf),
    Other(PathBuf),
}

/// Every entry below `root`, including file bytes and symlink targets.
fn snapshot(root: &Path) -> Vec<SnapshotEntry> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .unwrap_or_else(|why| {
                    panic!("{} is below {}: {why}", path.display(), root.display())
                })
                .to_owned();
            let meta = fs::symlink_metadata(&path)
                .unwrap_or_else(|why| panic!("{}: {why}", path.display()));
            if meta.file_type().is_symlink() {
                out.push(SnapshotEntry::Symlink(
                    relative,
                    fs::read_link(&path).unwrap_or_else(|why| panic!("{}: {why}", path.display())),
                ));
            } else if meta.is_dir() {
                out.push(SnapshotEntry::Directory(relative));
                stack.push(path);
            } else if meta.is_file() {
                out.push(SnapshotEntry::File(
                    relative,
                    fs::read(&path).unwrap_or_else(|why| panic!("{}: {why}", path.display())),
                ));
            } else {
                out.push(SnapshotEntry::Other(relative));
            }
        }
    }
    out.sort();
    out
}

// ─── the chain itself ────────────────────────────────────────────────────

/// IMPORTANT (r2-8): the publish sweep reports a pending rename's sessions
/// and skips them untouched — no migration, repoint, or helper write — while
/// placeable sessions still move. Smallest defeating mutation: drop the
/// pending skip from either sweep pass.
#[test]
fn a_pending_rename_is_reported_and_skipped_by_a_publish() {
    let rig = Rig::new("skip-pending-rename");
    let stale = rig.plant_version("2026.1.1");
    let healthy = rig.session(
        "healthy",
        &stale.join("ae-core").to_string_lossy(),
        Some(ae::migrate::CURRENT),
    );
    let pending = rig.session(
        "pendold",
        &stale.join("ae-core").to_string_lossy(),
        Some(ae::migrate::CURRENT),
    );
    // A well-formed prepared carrier over the pending session: hostile-shaped
    // but valid input for the admission owner under test.
    let carrier = "rename_intent=1\nsession_id=e795c9e9-1234-4890-abcd-ef0123456789\nold=pendold\nnew=pendnew\nmode=local\nold_work=/w\nnew_work=/w\norigin=/w\nserver_kind=ambient\nserver_value=\nphase=prepared\nwork_dev=0\nwork_ino=0\nadmin_dev=0\nadmin_ino=0\n";
    assert!(
        fs::write(
            rig.root()
                .join("sessions")
                .join(".rename.pendold.pendnew.intent"),
            carrier
        )
        .is_ok(),
        "a carrier"
    );
    let before = snapshot(&pending);

    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.9"));
    assert_eq!(code, Some(0), "the publish failed: {stdout}{stderr}");
    assert!(
        stdout.contains("skipped pendold: rename 'pendold' → 'pendnew' is in progress"),
        "the skip was reported: {stdout}"
    );
    assert_eq!(
        snapshot(&pending),
        before,
        "the pending session changed under the sweep"
    );
    let published = rig.versions().join("2026.9.9").join("ae-core");
    assert_eq!(
        value_of(&meta_of(&healthy), "ae_core").as_deref(),
        Some(published.to_string_lossy().as_ref()),
        "the placeable session was still repointed"
    );
}

#[test]
fn a_session_at_the_current_version_is_left_byte_for_byte_alone() {
    let rig = Rig::new("noop");
    let dir = rig.session("cur", "/nowhere/ae-core", Some(ae::migrate::CURRENT));
    let before = meta_of(&dir);
    assert_eq!(ae::migrate::session(&dir), Ok(None));
    assert_eq!(meta_of(&dir), before, "a no-op chain rewrote the meta");
}

#[test]
fn a_session_with_no_version_row_is_refused_by_name_with_the_fresh_start_line() {
    let rig = Rig::new("preversion");
    let dir = rig.legacy_session("old");
    let refused = ae::migrate::session(&dir).expect_err("the pre-version past");
    assert_eq!(refused, ae::migrate::Refusal::Absent);
    let line = refused.line("old");
    assert!(line.contains("ae end old"), "{line}");
    assert!(line.contains(ae::migrate::KEY), "{line}");

    // A stop or an end must still be possible: the note is written, and it is
    // a note, not a refusal.
    let noted = ae::migrate::session_noted(&dir, "old").expect("a reported refusal");
    assert!(noted.starts_with("note: "), "{noted}");
    assert!(noted.contains("ae end old"), "{noted}");
}

#[test]
fn a_session_that_carries_only_schema_2_is_stamped_and_otherwise_untouched() {
    // THE INSTALLED BASE. `meta_version` is younger than the sessions, so every
    // session that existed before the chain has none — 28 of 28 on the machine
    // this was written for. They all say `schema=2`, which is the same shape in
    // the older word, so they are PLACED at 2 rather than refused, and the row
    // is written in on first touch.
    let rig = Rig::new("stamp");
    let dir = rig.session("prechain", "/nowhere/ae-core", None);
    let before = meta_of(&dir);
    assert!(
        !before.contains(ae::migrate::KEY),
        "the fixture is not pre-chain"
    );

    assert_eq!(
        ae::migrate::session(&dir),
        Ok(Some(ae::migrate::Stepped::Stamped))
    );
    let after = meta_of(&dir);
    assert!(
        after.contains(&format!("{}={}", ae::migrate::KEY, ae::migrate::CURRENT)),
        "the row was not stamped in: {after}"
    );
    // OTHERWISE BYTE-IDENTICAL: a stamp adds one row and touches nothing else.
    assert_eq!(
        after,
        format!("{before}{}={}\n", ae::migrate::KEY, ae::migrate::CURRENT),
        "the stamp rewrote more than the row it came to add"
    );
    // And it is idempotent: the second touch has nothing to do.
    assert_eq!(ae::migrate::session(&dir), Ok(None));
    assert_eq!(meta_of(&dir), after);
    // Nothing is reported to an operator about a no-op.
    assert_eq!(ae::migrate::session_noted(&dir, "prechain"), None);
}

#[test]
fn a_session_with_neither_key_is_the_one_the_chain_cannot_place() {
    // The v1 roster: it says nothing about its shape in either word.
    let rig = Rig::new("neither");
    let dir = rig.legacy_session("v1");
    let refused = ae::migrate::session(&dir).expect_err("the pre-version past");
    assert_eq!(refused, ae::migrate::Refusal::Absent);
    assert!(refused.line("v1").contains("ae end v1"), "{refused:?}");
    assert!(
        !meta_of(&dir).contains(ae::migrate::KEY),
        "a refused meta was written to"
    );
}

#[test]
fn a_directory_under_sessions_with_no_meta_is_nothing_to_migrate() {
    let rig = Rig::new("nometa");
    let dir = rig.root().join("sessions").join("hollow");
    assert!(fs::create_dir_all(&dir).is_ok(), "a hollow session dir");
    assert_eq!(
        ae::migrate::session(&dir),
        Err(ae::migrate::Refusal::Missing)
    );
    assert_eq!(ae::migrate::session_noted(&dir, "hollow"), None);
}

// ─── the version-directory sweep ─────────────────────────────────────────

#[test]
fn the_version_sweep_keeps_the_published_one_and_every_one_a_session_records() {
    let rig = Rig::new("prune");
    for version in ["2026.1.1", "2026.2.2", "2026.3.3"] {
        rig.plant_version(version);
    }
    // One session still names 2026.2.2 — the case the sweep exists to be safe
    // about, because a session left behind must not lose its core.
    let kept = rig.versions().join("2026.2.2").join("ae-core");
    rig.session("holds", &kept.to_string_lossy(), Some(ae::migrate::CURRENT));
    // A session whose core lives somewhere else entirely protects nothing here.
    rig.session("foreign", "/usr/local/bin/ae", Some(ae::migrate::CURRENT));

    let notes = ae::migrate::prune_versions(&rig.root(), &rig.link(), "2026.3.3");
    assert!(
        present(&rig.versions().join("2026.3.3")),
        "the published version was pruned"
    );
    assert!(present(&kept), "a version a session records was pruned");
    assert!(
        !present(&rig.versions().join("2026.1.1")),
        "an unreferenced version survived"
    );
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("2026.1.1"), "{notes:?}");
}

#[test]
fn an_unreadable_sessions_root_stops_the_version_sweep_rather_than_emptying_it() {
    // BLOCKER: a census that FAILED is not a census that found nothing. With
    // the sessions root unreadable, an inventory that swallowed the error
    // returned no sessions, so nothing "recorded" the old version and the
    // publish deleted the core every session was running on.
    let rig = Rig::new("blindprune");
    let stale = rig.plant_version("2026.1.1");
    rig.session(
        "held",
        &stale.join("ae-core").to_string_lossy(),
        Some(ae::migrate::CURRENT),
    );
    let sessions = rig.root().join("sessions");
    assert!(
        fs::set_permissions(&sessions, fs::Permissions::from_mode(0o000)).is_ok(),
        "an unreadable sessions root"
    );

    let notes = ae::migrate::prune_versions(&rig.root(), &rig.link(), "2026.9.9");
    let _ = fs::set_permissions(&sessions, fs::Permissions::from_mode(0o755));

    assert!(
        present(&stale),
        "a pinned version was deleted blind: {notes:?}"
    );
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].starts_with("WARNING:"), "{notes:?}");
    assert!(notes[0].contains("could not be enumerated"), "{notes:?}");
}

#[test]
fn a_meta_the_sweep_cannot_read_stops_it_too_rather_than_being_skipped() {
    // The same rule one level down: a session whose pin is unreadable has not
    // been ruled OUT of use, and pruning past it is guessing.
    let rig = Rig::new("blindmeta");
    let stale = rig.plant_version("2026.1.1");
    let dir = rig.session(
        "opaque",
        &stale.join("ae-core").to_string_lossy(),
        Some(ae::migrate::CURRENT),
    );
    let meta = dir.join("meta");
    assert!(
        fs::set_permissions(&meta, fs::Permissions::from_mode(0o000)).is_ok(),
        "an unreadable meta"
    );

    let notes = ae::migrate::prune_versions(&rig.root(), &rig.link(), "2026.9.9");
    let _ = fs::set_permissions(&meta, fs::Permissions::from_mode(0o644));

    assert!(
        present(&stale),
        "a version was pruned past an unreadable pin"
    );
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("opaque"),
        "the warning names no session: {notes:?}"
    );
}

#[test]
fn the_version_the_command_link_names_is_never_swept_even_if_no_meta_records_it() {
    // The floor under the publisher lock. A core older than that lock takes no
    // lock, so during the rollout that adds it one could repoint the command
    // between this keep-set being built and the sweep running. Everything else
    // a stale keep-set gets wrong costs disk space; deleting the directory the
    // live `ae` resolves into costs the command itself.
    let rig = Rig::new("linkfloor");
    let live = rig.plant_version("2026.1.1");
    let orphan = rig.plant_version("2026.1.2");
    assert!(
        fs::create_dir_all(rig.link().parent().unwrap_or(&rig.home)).is_ok(),
        "a bin dir"
    );
    assert!(
        std::os::unix::fs::symlink(live.join("ae-core"), rig.link()).is_ok(),
        "a command link"
    );

    let notes = ae::migrate::prune_versions(&rig.root(), &rig.link(), "2026.9.9");
    assert!(
        present(&live),
        "the sweep deleted what the command link names: {notes:?}"
    );
    assert!(
        !present(&orphan),
        "the floor kept a version nothing names: {notes:?}"
    );
}

#[test]
fn a_state_root_with_no_sessions_directory_still_sweeps_old_versions() {
    // The other side of the fallible census: "could not look" must refuse, but
    // a state root that has never had a session is not that. It is a first
    // install, and refusing there would leave every fresh machine accumulating
    // version directories forever.
    let rig = Rig::new("firstinstall");
    let stale = rig.plant_version("2026.1.1");
    assert!(
        !rig.root().join("sessions").exists(),
        "the rig planted a sessions root"
    );

    let notes = ae::migrate::prune_versions(&rig.root(), &rig.link(), "2026.9.9");
    assert!(!present(&stale), "nothing was swept: {notes:?}");
    assert!(
        notes.iter().all(|note| !note.starts_with("WARNING:")),
        "a missing sessions root was read as unreadable: {notes:?}"
    );
}

#[test]
fn the_version_sweep_never_re_modes_a_symlink_it_removes() {
    // `set_permissions` FOLLOWS a link, so re-moding a version's members before
    // unlinking them reached OUT of the directory being removed: an external
    // 0600 file came back 0644, and a link to another installed core would have
    // lost its published mode.
    let rig = Rig::new("symmode");
    let outside = rig.scratch.join("private");
    assert!(
        fs::write(
            &outside, "secret
"
        )
        .is_ok(),
        "an external file"
    );
    assert!(
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o600)).is_ok(),
        "its own mode"
    );
    let stale = rig.plant_version("2026.1.1");
    assert!(
        std::os::unix::fs::symlink(&outside, stale.join("pointer")).is_ok(),
        "a member that is a link out"
    );

    let notes = ae::migrate::prune_versions(&rig.root(), &rig.link(), "2026.9.9");
    assert!(!present(&stale), "the version was not removed: {notes:?}");
    assert!(
        present(&outside),
        "the sweep followed the link and deleted through it"
    );
    assert_eq!(
        fs::symlink_metadata(&outside)
            .expect("the external file")
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "the sweep chmodded through a symlink"
    );
}

// ─── the publish, black-box ──────────────────────────────────────────────

#[test]
fn a_publish_repoints_every_stopped_session_before_it_repoints_the_command_link() {
    let rig = Rig::new("stopped");
    let stale = rig.plant_version("2026.1.1");
    let one = rig.session(
        "alpha",
        &stale.join("ae-core").to_string_lossy(),
        Some(ae::migrate::CURRENT),
    );
    let two = rig.session(
        "beta",
        &stale.join("ae-core").to_string_lossy(),
        Some(ae::migrate::CURRENT),
    );

    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.9"));
    assert_eq!(code, Some(0), "install failed: {stdout}{stderr}");

    let published = rig.versions().join("2026.9.9").join("ae-core");
    for dir in [&one, &two] {
        let meta = meta_of(dir);
        assert_eq!(
            value_of(&meta, "ae_core").as_deref(),
            Some(published.to_string_lossy().as_ref()),
            "the session still names the old core: {meta}"
        );
        assert_eq!(
            value_of(&meta, "ae_core_version").as_deref(),
            Some("2026.9.9")
        );
        assert_eq!(value_of(&meta, "ae_version").as_deref(), Some("2026.9.9"));
        // EVERY helper, enumerated from the product's own list rather than
        // sampled: a name left on the old core is a helper an agent calls and
        // gets the wrong binary from, and a five-name sample cannot see it.
        for helper in ae::shim::HELPERS {
            assert_eq!(
                fs::read_link(dir.join(helper.name)).ok(),
                Some(published.clone()),
                "{} does not name the published core",
                helper.name
            );
        }
    }
    assert_eq!(
        fs::read_link(rig.link()).ok(),
        Some(published),
        "the command link does not name the published core"
    );
    // The version nothing records any more is gone, and the publish said so.
    assert!(
        !present(&stale),
        "the superseded version directory survived"
    );
    assert!(stdout.contains("2026.1.1"), "unreported prune: {stdout}");
}

#[test]
fn a_publish_stamps_every_pre_chain_session_and_counts_them_in_one_line() {
    // THE RELEASE THAT ADDS THE CHAIN, as it will actually be met: every
    // session on the machine carries `schema=2` and no version row. All of them
    // must come through stamped, none refused, and the operator must be told
    // once with a number rather than once per session.
    let rig = Rig::new("stampsweep");
    let sessions: Vec<PathBuf> = ["one", "two", "three"]
        .iter()
        .map(|name| rig.session(name, "/nowhere/ae-core", None))
        .collect();
    // One session already carries the row, so the count is of the stamped ones
    // and not simply of every session.
    let already = rig.session("four", "/nowhere/ae-core", Some(ae::migrate::CURRENT));

    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.9"));
    assert_eq!(code, Some(0), "the publish failed: {stdout}{stderr}");

    let published = rig.versions().join("2026.9.9").join("ae-core");
    for dir in sessions.iter().chain(std::iter::once(&already)) {
        let meta = meta_of(dir);
        assert!(
            meta.contains(&format!("{}={}", ae::migrate::KEY, ae::migrate::CURRENT)),
            "a session came through the publish unstamped: {meta}"
        );
        assert_eq!(
            value_of(&meta, "ae_core").as_deref(),
            Some(published.to_string_lossy().as_ref()),
            "a stamped session was not repointed: {meta}"
        );
    }
    // ONE line, carrying the number.
    let stamped: Vec<&str> = stdout
        .lines()
        .filter(|line| line.contains("stamped"))
        .collect();
    assert_eq!(stamped.len(), 1, "{stdout}");
    assert!(stamped[0].contains('3'), "the count is wrong: {stamped:?}");
    assert!(stamped[0].contains("schema=2"), "{stamped:?}");
}

#[test]
fn a_stopped_pre_chain_session_is_reported_and_skipped_by_a_publish() {
    let rig = Rig::new("skip-stopped-pre-chain");
    let stale = rig.plant_version("2026.1.1");
    let healthy = rig.session(
        "healthy",
        &stale.join("ae-core").to_string_lossy(),
        Some(ae::migrate::CURRENT),
    );
    let skipped = rig.legacy_session("legacy");
    let before = snapshot(&skipped);
    assert!(
        !present(&rig.link()),
        "the fixture already has a command link"
    );

    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.9"));
    assert_eq!(code, Some(0), "the publish failed: {stdout}{stderr}");

    let published = rig.versions().join("2026.9.9").join("ae-core");
    assert_eq!(
        fs::read_link(rig.link()).ok(),
        Some(published.clone()),
        "the command link did not move past the stopped legacy session"
    );
    assert_eq!(
        value_of(&meta_of(&healthy), "ae_core").as_deref(),
        Some(published.to_string_lossy().as_ref()),
        "the placeable session was not repointed"
    );
    assert!(
        stdout.contains(
            "ae: skipped legacy: it pre-dates the migration chain and is not running — end it (ae end legacy)"
        ),
        "the skip was not reported: {stdout}"
    );
    assert_eq!(
        snapshot(&skipped),
        before,
        "the skipped session directory changed"
    );
    assert!(
        !meta_of(&skipped).contains("ae_core_version="),
        "the skipped meta started naming a version"
    );
    assert!(!present(&stale), "the superseded version was not swept");
}

#[test]
fn stopped_sessions_are_skipped_for_every_version_refusal() {
    let rig = Rig::new("skip-version-refusals");
    let sessions = [
        ("unreadable", rig.unreadable_session("unreadable")),
        ("no-step", rig.session("no-step", "/old/core", Some(1))),
        (
            "ahead",
            rig.session("ahead", "/old/core", Some(ae::migrate::CURRENT + 1)),
        ),
    ];
    let before: Vec<_> = sessions.iter().map(|(_, dir)| snapshot(dir)).collect();

    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.9"));
    assert_eq!(code, Some(0), "the publish failed: {stdout}{stderr}");
    assert!(
        fs::read_link(rig.link()).is_ok_and(|target| target.ends_with("2026.9.9/ae-core")),
        "the command link did not move"
    );
    assert!(
        stdout.lines().any(|line| {
            line == "ae: skipped ahead: meta newer than this ae, left untouched — upgrade ae to resume it"
        }),
        "the newer session was given unsafe recovery advice: {stdout}"
    );
    for ((name, dir), before) in sessions.iter().zip(before) {
        assert!(
            stdout.contains(&format!("ae: skipped {name}:")),
            "{name} was not reported: {stdout}"
        );
        assert_eq!(snapshot(dir), before, "the skipped {name} session changed");
        for helper in ae::shim::HELPERS {
            assert!(
                !present(&dir.join(helper.name)),
                "{} was rendered into skipped session {name}",
                helper.name
            );
        }
    }
}

#[test]
fn a_session_whose_helpers_cannot_be_relinked_is_reported_as_partially_relinked() {
    // The meta is rewritten BEFORE the helpers are, so a helper failure leaves
    // that session repointed. Reporting it as untouched — which is what
    // recording the repoint after the helpers did — sends the operator looking
    // in the wrong place, and the sessions that DID move go unnamed with it.
    let rig = Rig::new("partial");
    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.9"));
    assert_eq!(code, Some(0), "the first install failed: {stdout}{stderr}");
    let first = rig.versions().join("2026.9.9").join("ae-core");

    rig.session("aaa", &first.to_string_lossy(), Some(ae::migrate::CURRENT));
    let blocked = rig.session("zzz", &first.to_string_lossy(), Some(ae::migrate::CURRENT));
    // A helper name occupied by a directory that is not empty: `send` cannot be
    // unlinked and cannot be replaced by a symlink.
    let occupied = blocked.join("send");
    assert!(
        fs::create_dir_all(&occupied).is_ok(),
        "an occupied helper name"
    );
    assert!(fs::write(occupied.join("keep"), "x").is_ok(), "its content");

    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.10"));
    assert_eq!(code, Some(1), "the publish did not fail: {stdout}{stderr}");
    assert!(
        stderr.contains("PARTIALLY relinked"),
        "the abort does not say the session moved half way: {stderr}"
    );
    // The COUNT is the finding: the half-moved session has to appear in the
    // list of sessions that already name the new version, alongside the one
    // that moved whole. Recording the repoint after the helpers rather than
    // before them leaves it out, and an operator reads "aaa" and stops there.
    assert!(
        stderr.contains("2 session(s) already name 2026.9.10: aaa, zzz"),
        "the abort does not count the half-moved session among the moved: {stderr}"
    );
    assert_eq!(
        value_of(&meta_of(&blocked), "ae_core_version").as_deref(),
        Some("2026.9.10"),
        "the fixture did not reach the helper step"
    );
    assert!(
        std::fs::read_link(rig.link())
            .is_ok_and(|target| target.starts_with(rig.versions().join("2026.9.9"))),
        "the command link moved despite the abort"
    );
}

// ─── the namespace a publish would actually write to ─────────────────────

#[test]
fn a_checkout_run_whose_state_root_is_elsewhere_refuses_to_upgrade() {
    // A publish is `$HOME`-pinned: it writes `$HOME/.ae` and repoints
    // `$HOME/.local/bin/ae`, whatever `AE_HOME` says. So a checkout build
    // pointed at a second namespace and asked to upgrade would have migrated,
    // repointed and PRUNED the sessions of the other root. It refuses instead,
    // naming both, and it does so before anything is downloaded.
    let (mut command, dir) = super::cli::ae_hermetic();
    let elsewhere = dir.join("elsewhere");
    let out = command
        .env("AE_HOME", &elsewhere)
        .arg("upgrade")
        .output()
        .unwrap_or_else(|why| panic!("the product binary should run: {why}"));

    assert_eq!(out.status.code(), Some(2), "{:?}", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&elsewhere.to_string_lossy().into_owned()),
        "the refusal does not name the state root in use: {stderr}"
    );
    assert!(
        stderr.contains(&dir.join(".ae").to_string_lossy().into_owned()),
        "the refusal does not name the root a publish would write: {stderr}"
    );
    assert!(
        !present(&elsewhere) && !present(&dir.join(".ae")),
        "the refusal wrote to one of the roots it named"
    );
}

// ─── the running session, against a real tmux ────────────────────────────

/// A scratch dir short enough to hold a socket path — `sun_path` is 104 bytes
/// on macOS and the usual temp dir eats most of it.
fn tmux_scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(format!("/tmp/ae-mg-{tag}-{}", std::process::id()));
    let _ = remove(&dir);
    assert!(fs::create_dir_all(&dir).is_ok(), "a short scratch dir");
    dir
}

/// Kill the arm's server and remove its scratch, WHATEVER ended the arm.
struct Cleanup {
    socket: PathBuf,
    scratch: PathBuf,
}

/// A named or ambient server rooted in a private `TMUX_TMPDIR`.
struct ServerCleanup {
    selector: Vec<String>,
    scratch: PathBuf,
}

struct MultiServerCleanup {
    selectors: Vec<Vec<String>>,
    scratch: PathBuf,
}

impl Drop for MultiServerCleanup {
    fn drop(&mut self) {
        for selector in &self.selectors {
            let mut args = selector.clone();
            args.push("kill-server".to_owned());
            let _ = run_tmux(&args, &self.scratch);
        }
        let _ = remove(&self.scratch);
    }
}

impl Drop for ServerCleanup {
    fn drop(&mut self) {
        let mut invocation = Invocation::new("tmux");
        for word in &self.selector {
            invocation = invocation.arg(word);
        }
        invocation = invocation.arg("kill-server");
        let _ = raw::run(
            &invocation,
            &self.scratch,
            &self.scratch.join("cleanup-out"),
            &self.scratch.join("cleanup-err"),
        );
        let _ = remove(&self.scratch);
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        // NOT the panicking helper: this runs while a panic may already be
        // unwinding, and a second panic there takes the failure report with it.
        let out = self.scratch.join("cleanup-out");
        let err = self.scratch.join("cleanup-err");
        let invocation = Invocation::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .arg("kill-server");
        let _ = raw::run(&invocation, &self.scratch, &out, &err);
        let _ = remove(&self.scratch);
    }
}

/// The id of the session's one AGENT pane — the pane whose `@ae_agent` stamp is
/// a seat name rather than one of the two `_`-prefixed monitors.
fn agent_pane_of(socket: &Path, scratch: &Path, session: &str) -> String {
    let (_, listed) = tmux(
        socket,
        scratch,
        &[
            "list-panes",
            "-s",
            "-t",
            session,
            "-F",
            "#{@ae_agent} #{pane_id}",
        ],
    );
    listed
        .lines()
        .find(|line| !line.starts_with('_'))
        .and_then(|line| line.split_whitespace().next_back())
        .unwrap_or_else(|| panic!("no agent pane in {session}: {listed:?}"))
        .to_owned()
}

fn tmux(socket: &Path, scratch: &Path, words: &[&str]) -> (bool, String) {
    let mut args = vec!["-S".to_owned(), socket.display().to_string()];
    args.extend(words.iter().map(|word| (*word).to_owned()));
    run_tmux(&args, scratch)
}

fn start_session(scratch: &Path, selector: &[&str], session: &str) {
    let mut args: Vec<String> = selector.iter().map(|word| (*word).to_owned()).collect();
    args.extend(
        [
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            session,
            "sleep",
            "60",
        ]
        .iter()
        .map(|word| (*word).to_owned()),
    );
    assert!(run_tmux(&args, scratch).0, "the {session} server");
}

fn start_server(scratch: &Path, selector: &[&str], session: &str) -> ServerCleanup {
    start_session(scratch, selector, session);
    ServerCleanup {
        selector: selector.iter().map(|word| (*word).to_owned()).collect(),
        scratch: scratch.to_owned(),
    }
}

fn assert_equal_widths(socket: &Path, scratch: &Path, target: &str) {
    let (_, listed) = tmux(
        socket,
        scratch,
        &["list-panes", "-t", target, "-F", "#{pane_width}"],
    );
    let widths = listed
        .lines()
        .filter_map(|width| width.parse::<usize>().ok())
        .collect::<Vec<_>>();
    assert_eq!(widths.len(), 2, "two lead-pair panes: {listed}");
    assert!(
        widths[0].abs_diff(widths[1]) <= 1,
        "lead panes differ by more than one cell: {listed}"
    );
}

fn root_status_binding(socket: &Path, scratch: &Path, key: &str) -> String {
    let (_, keys) = tmux(socket, scratch, &["list-keys", "-T", "root"]);
    let prefix = format!("bind-key  -T root {key} ");
    keys.lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("one {key} binding: {keys}"))
        .to_owned()
}

fn assert_tmux_default_mouse_binding(socket: &Path, scratch: &Path) {
    let binding = root_status_binding(socket, scratch, "MouseDown1Status");
    assert!(
        binding.contains("switch-client -t =") && !binding.contains("if-shell"),
        "the pre-release server begins with tmux's default: {binding}"
    );
}

fn assert_ae_status_bindings(socket: &Path, scratch: &Path) {
    let server = ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(socket.to_owned()));
    let menu_mouse = ae::transport::observe_tmux_floor(&server).menu_mouse();
    let (_, keys) = tmux(socket, scratch, &["list-keys", "-T", "root"]);
    for key in [
        "MouseDown3Pane",
        "M-MouseDown3Pane",
        "MouseDown3StatusLeft",
        "M-MouseDown3Status",
        "M-MouseDown3StatusLeft",
    ] {
        assert!(
            !keys
                .lines()
                .any(|line| line.starts_with(&format!("bind-key  -T root {key} "))),
            "the upgraded ae-owned server removes tmux's stock right-click menu for {key}: {keys}"
        );
    }
    let down = root_status_binding(socket, scratch, "MouseDown1Status");
    assert!(
        down.contains("#{||:#{==:#{mouse_status_range},ae}")
            && down.contains("#{==:#{mouse_status_range},ae-more}")
            && !down.contains("@ae_orchestrator_id")
            && down.contains("mouse_status_range},window")
            && down.contains("mouse_status_range},session")
            && down.contains("select-window -t #{window_id}")
            && down.contains("switch-client -c #{q:client_name} -t #{session_id}"),
        "the upgrade reasserts press navigation: {down}"
    );
    let up_picker = if menu_mouse {
        let (_, keys) = tmux(socket, scratch, &["list-keys", "-T", "root"]);
        assert!(
            !keys.contains("bind-key  -T root MouseUp1Status ")
                && !keys.contains("bind-key  -T root MouseUp3Status "),
            "mouse-aware upgrade clears stale Up bindings: {keys}"
        );
        None
    } else {
        assert!(
            !down.contains("orchestrator"),
            "tmux 3.4 must not open twice: {down}"
        );
        Some(root_status_binding(socket, scratch, "MouseUp1Status"))
    };
    let picker = up_picker.as_deref().unwrap_or(&down);
    for needle in [
        "orchestrator",
        "--client",
        "#{q:client_name}",
        "AE_HOME=",
        "CONFIG_FILE=",
        "AE_TMUX_SERVER=",
        "AE_TMUX_SERVER_KIND=socket",
    ] {
        assert!(picker.contains(needle), "missing {needle:?}: {picker}");
    }
    let down_menu = root_status_binding(socket, scratch, "MouseDown3Status");
    let menu = if menu_mouse {
        down_menu
    } else {
        assert!(
            !down_menu.contains("orchestrator") && !down_menu.contains("_session-menu"),
            "tmux 3.4 press context path must be a no-op: {down_menu}"
        );
        root_status_binding(socket, scratch, "MouseUp3Status")
    };
    for needle in [
        "#{||:#{==:#{mouse_status_range},ae}",
        "mouse_status_range},ae-more",
        "orchestrator",
        "--client",
        "'_session-menu' 'show'",
        "--session-id",
        "--server-start",
        "{mouse}",
    ] {
        assert!(menu.contains(needle), "missing {needle:?}: {menu}");
    }
    let (prefix_ok, prefix) = tmux(socket, scratch, &["list-keys", "-T", "prefix"]);
    let hotkey = prefix
        .lines()
        .find(|line| line.contains("-T prefix a "))
        .unwrap_or_default();
    assert!(
        prefix_ok
            && hotkey.contains("run-shell -b")
            && hotkey.contains("orchestrator")
            && hotkey.contains("--client")
            && hotkey.contains("#{q:client_name}"),
        "the upgrade reasserts prefix a (query ok={prefix_ok}): {hotkey}"
    );
}

#[test]
fn an_unreadable_stopped_meta_is_reported_and_skipped() {
    let rig = Rig::new("skip-unreadable-meta");
    let held = rig.plant_version("2026.1.1");
    let dir = rig.unreadable_meta_session("unreadable");
    assert!(
        std::os::unix::fs::symlink(held.join("ae-core"), dir.join("send")).is_ok(),
        "a helper whose version cannot be read from meta"
    );
    let before = snapshot(&dir);
    let tmux_tmpdir = tmux_scratch("unreadable-stopped");

    let (code, stdout, stderr) =
        rig.install_with_tmux_tmpdir(&rig.bundle("2026.9.9"), Some(&tmux_tmpdir));
    let _ = remove(&tmux_tmpdir);
    assert_eq!(code, Some(0), "the publish failed: {stdout}{stderr}");
    assert!(
        stdout.contains("ae: skipped unreadable: meta unreadable (")
            && stdout.contains("), not running"),
        "the unreadable meta was not reported: {stdout}"
    );
    assert!(
        stdout.contains("WARNING: no version was removed"),
        "the unsafe prune was not reported: {stdout}"
    );
    assert_eq!(
        snapshot(&dir),
        before,
        "the unreadable stopped session changed"
    );
    assert!(
        fs::read_link(rig.link()).is_ok_and(|target| target.ends_with("2026.9.9/ae-core")),
        "the command link did not move"
    );
    assert!(present(&held), "the unreadable session's core was pruned");
    assert!(
        fs::canonicalize(dir.join("send")).is_ok(),
        "the unreadable session's helper became dangling"
    );
}

#[test]
fn an_unreadable_live_meta_aborts_on_both_legacy_servers() {
    for (tag, selector) in [("owned", &["-L", "ae"][..]), ("historical", &[][..])] {
        let rig = Rig::new(&format!("refuse-unreadable-{tag}"));
        let tmux_tmpdir = tmux_scratch(&format!("unreadable-{tag}"));
        let (code, stdout, stderr) =
            rig.install_with_tmux_tmpdir(&rig.bundle("2026.9.9"), Some(&tmux_tmpdir));
        assert_eq!(code, Some(0), "the first install failed: {stdout}{stderr}");
        let first = rig.versions().join("2026.9.9").join("ae-core");
        let name = format!("unreadable-{tag}");
        let _server = start_server(&tmux_tmpdir, selector, &name);
        let dir = rig.unreadable_meta_session(&name);
        assert!(
            std::os::unix::fs::symlink(&first, dir.join("send")).is_ok(),
            "a live helper"
        );
        let before = snapshot(&dir);

        let (code, stdout, stderr) =
            rig.install_with_tmux_tmpdir(&rig.bundle("2026.9.10"), Some(&tmux_tmpdir));
        assert_eq!(
            code,
            Some(1),
            "the live meta did not abort: {stdout}{stderr}"
        );
        assert!(
            stderr.contains(&format!("session {name:?}: meta could not be read")),
            "the live refusal does not name its session: {stderr}"
        );
        assert_eq!(
            fs::read_link(rig.link()).ok(),
            Some(first),
            "the command link moved past the live unreadable meta"
        );
        assert_eq!(
            snapshot(&dir),
            before,
            "the live unreadable session changed"
        );
    }
}

#[test]
fn a_running_pre_chain_session_still_aborts_the_publish() {
    let rig = Rig::new("refuse-running-pre-chain");
    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.9"));
    assert_eq!(code, Some(0), "the first install failed: {stdout}{stderr}");
    let first = rig.versions().join("2026.9.9").join("ae-core");
    let early = rig.session("aaa", &first.to_string_lossy(), Some(ae::migrate::CURRENT));
    let early_before = snapshot(&early);

    let scratch = tmux_scratch("running-pre-chain");
    if !tmux_present(&scratch) {
        let _ = remove(&scratch);
        panic!(
            "tmux is not runnable here, so the running pre-chain refusal cannot be proven; \
             install tmux or run this suite where one exists"
        );
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let session = "zzz";
    let dir = rig.legacy_session(session);
    let mut meta = meta_of(&dir);
    let _ = write!(
        meta,
        "tmux_server_kind=socket\ntmux_server={}\n",
        socket.display()
    );
    assert!(fs::write(dir.join("meta"), meta).is_ok(), "the server rows");
    assert!(
        tmux(
            &socket,
            &scratch,
            &["new-session", "-d", "-s", session, "sleep", "60"]
        )
        .0,
        "the running legacy session"
    );
    let before = snapshot(&dir);

    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.10"));
    assert_eq!(code, Some(1), "the publish did not fail: {stdout}{stderr}");
    assert!(
        stderr.contains("session \"zzz\" records no meta_version"),
        "the refusal changed: {stderr}"
    );
    assert_eq!(
        fs::read_link(rig.link()).ok(),
        Some(first),
        "the command link moved despite the running legacy session"
    );
    assert_eq!(snapshot(&dir), before, "the running legacy session changed");
    assert_eq!(
        snapshot(&early),
        early_before,
        "pass one repointed a placeable session before the live refusal"
    );
}

/// Historical default-server liveness must not depend on caller `$TMUX`.
#[test]
fn a_legacy_default_session_blocks_publish_from_outside_and_foreign_caller() {
    let rig = Rig::new("legacy-default-foreign-caller");
    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.9"));
    assert_eq!(code, Some(0), "the first install failed: {stdout}{stderr}");
    let old_core = rig.versions().join("2026.9.9").join("ae-core");
    let victim = rig.legacy_session("victim");
    assert!(std::os::unix::fs::symlink(&old_core, victim.join("send")).is_ok());
    let before = snapshot(&victim);
    let scratch = tmux_scratch("legacy-default-foreign-caller");
    if !tmux_present(&scratch) {
        let _ = remove(&scratch);
        panic!("tmux is not runnable here");
    }
    let _servers = MultiServerCleanup {
        selectors: vec![Vec::new(), vec!["-L".to_owned(), "foreign".to_owned()]],
        scratch: scratch.clone(),
    };
    start_session(&scratch, &[], "victim");

    let (code, stdout, stderr) =
        rig.install_with_tmux_tmpdir(&rig.bundle("2026.9.10"), Some(&scratch));
    assert_eq!(
        code,
        Some(1),
        "outside-TMUX publish did not refuse: {stdout}{stderr}"
    );
    assert!(
        stderr.contains("session \"victim\" records no meta_version"),
        "{stderr}"
    );
    assert_eq!(fs::read_link(rig.link()).ok(), Some(old_core.clone()));
    assert_eq!(snapshot(&victim), before);

    start_session(&scratch, &["-L", "foreign"], "caller");
    let (_, marker) = run_tmux(
        &[
            "-L",
            "foreign",
            "list-panes",
            "-a",
            "-F",
            "#{socket_path}|#{pane_id}",
        ]
        .iter()
        .map(|word| (*word).to_owned())
        .collect::<Vec<_>>(),
        &scratch,
    );
    let (socket, pane) = marker.trim().split_once('|').expect("caller marker");
    let (code, stdout, stderr) = rig.install_with_caller(
        &rig.bundle("2026.9.10"),
        &scratch,
        &format!("{socket},0,0"),
        pane,
    );
    assert_eq!(
        code,
        Some(1),
        "foreign-caller publish did not refuse: {stdout}{stderr}"
    );
    assert!(
        stderr.contains("session \"victim\" records no meta_version"),
        "{stderr}"
    );
    assert_eq!(fs::read_link(rig.link()).ok(), Some(old_core));
    assert_eq!(snapshot(&victim), before);
}

#[test]
fn an_ahead_meta_without_ae_core_keeps_old_helper_resolvable() {
    let rig = Rig::new("legacy-version-only");
    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.9"));
    assert_eq!(code, Some(0), "the first install failed: {stdout}{stderr}");
    let old_core = rig.versions().join("2026.9.9").join("ae-core");
    let dir = rig.root().join("sessions").join("version-only");
    assert!(fs::create_dir_all(&dir).is_ok());
    assert!(
        fs::write(
            dir.join("meta"),
            "meta_version=3\nmode=local\nsession=version-only\nae_core_version=2026.9.9\n",
        )
        .is_ok()
    );
    assert!(std::os::unix::fs::symlink(&old_core, dir.join("send")).is_ok());
    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.10"));
    assert_eq!(code, Some(0), "publish failed: {stdout}{stderr}");
    assert!(
        fs::canonicalize(dir.join("send")).is_ok(),
        "old helper became dangling"
    );
}

/// A core that behaves like the two things a session runs it as: the watchdog
/// publishes a pidfile, everything else just stays alive.
const FAKE_CORE: &str = "#!/bin/sh\n\
     d=$(cd \"$(dirname \"$0\")\" && pwd)\n\
     case \"$(basename \"$0\")\" in\n\
     watchdog) printf '%s\\n' \"$$\" > \"$d/.watchdog.pid.staged\"\n\
     mv \"$d/.watchdog.pid.staged\" \"$d/.watchdog.pid\" ;;\n\
     esac\n\
     exec sleep 60\n";

/// A LIVE session on `socket`: a meta at the current version pinned to
/// `old_core`, the two helpers the start path runs, its tmux session, a
/// stand-in bridge, and a started watchdog.
fn plant_running(
    scratch: &Path,
    socket: &Path,
    root: &Path,
    session: &str,
    old_core: &Path,
) -> PathBuf {
    let dir = root.join("sessions").join(session);
    assert!(fs::create_dir_all(&dir).is_ok(), "a session dir");
    let meta = format!(
        "{}={}\nmode=local\nsession={session}\nwork_dir=/w\nae_version=0.0.1\n\
         ae_core={}\nae_core_version=0.0.1\n\
         tmux_server_kind=socket\ntmux_server={}\n\
         schema=2\nseat.main=lead\nprofile.main=cl\nagent_bin.main=claude\n",
        ae::migrate::KEY,
        ae::migrate::CURRENT,
        old_core.display(),
        socket.display()
    );
    assert!(fs::write(dir.join("meta"), meta).is_ok(), "a meta");
    // Linked at the OLD core, by hand — so the test asserts against the
    // product's own re-render rather than against itself.
    for helper in ["watchdog", "events-tail"] {
        assert!(
            std::os::unix::fs::symlink(old_core, dir.join(helper)).is_ok(),
            "a helper link"
        );
    }
    assert!(
        tmux(
            socket,
            scratch,
            &["new-session", "-d", "-s", session, "sleep", "60"]
        )
        .0,
        "the session the watchdog watches"
    );
    // The bridge is machine-global: a tmux session under its own name is
    // exactly what the liveness check looks for, and no real bridge is ever
    // spawned here.
    assert!(
        tmux(
            socket,
            scratch,
            &["new-session", "-d", "-s", "ae-telegram", "sleep", "60"]
        )
        .0,
        "a stand-in bridge"
    );
    // The start is RETRIED once. `watchdog start` waits a bounded number of
    // polls for the daemon to publish a pidfile, and this fixture's daemon is a
    // shell script in a freshly split tmux pane: under a full suite run, that
    // pane can lose the race to a loaded machine. Retrying is not weakening the
    // proof — the test still requires a pid here and a DIFFERENT pid after the
    // sweep — it just refuses to report a scheduling delay as a product defect.
    let mut last = String::new();
    for _ in 0..2 {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = ae::watchdog_lifecycle::run(
            root,
            &["start".to_owned(), session.to_owned()],
            &mut out,
            &mut err,
        )
        .unwrap_or_else(|why| panic!("the entry writes to in-memory buffers: {why}"));
        last = String::from_utf8_lossy(&err).into_owned();
        if code == 0 && ae::watchdog_glue::read_pid(&dir).is_some() {
            return dir;
        }
    }
    panic!("the watchdog did not start after two tries: {last}");
}

/// Rewrite a planted running session into the unhooked, equal-width
/// lead-pair shape that predates the pair policy.
fn make_unhooked_lead_pair(socket: &Path, scratch: &Path, dir: &Path, session: &str) -> String {
    let main_pane = agent_pane_of(socket, scratch, session);
    let mut meta = meta_of(dir);
    let _ = write!(
        meta,
        "layout=lead-pair\nmain_pane={main_pane}\nseat.worker.0=colead\nprofile.worker.0=cl\nagent_bin.worker.0=claude\n"
    );
    assert!(fs::write(dir.join("meta"), meta).is_ok(), "lead-pair meta");
    assert!(
        tmux(
            socket,
            scratch,
            &["split-window", "-h", "-t", &main_pane, "sleep", "60"]
        )
        .0,
        "the colead pane"
    );
    assert!(
        tmux(
            socket,
            scratch,
            &[
                "set-window-option",
                "-t",
                &main_pane,
                "main-pane-width",
                "50%"
            ]
        )
        .0,
        "the old equal width"
    );
    assert!(
        tmux(
            socket,
            scratch,
            &["select-layout", "-t", &main_pane, "even-horizontal"]
        )
        .0,
        "the old equal layout"
    );
    main_pane
}

fn assert_lead_pair_policy(socket: &Path, scratch: &Path, main_pane: &str, session: &str) {
    let (_, main_width) = tmux(
        socket,
        scratch,
        &[
            "show-window-options",
            "-v",
            "-t",
            main_pane,
            "main-pane-width",
        ],
    );
    assert_eq!(main_width.trim(), "50%", "upgrade restores pair width");
    let (_, resize_hook) = tmux(
        socket,
        scratch,
        &["show-hooks", "-w", "-t", main_pane, "window-resized"],
    );
    assert!(
        resize_hook.contains("window_zoomed_flag") && resize_hook.contains("main-vertical"),
        "upgrade restores the guarded resize hook: {resize_hook}"
    );
    assert!(
        tmux(
            socket,
            scratch,
            &["resize-window", "-t", session, "-x", "150", "-y", "40"]
        )
        .0,
        "resize the upgraded lead-pair window"
    );
    assert_equal_widths(socket, scratch, session);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one running-session upgrade and daemon restart contract"
)]
fn a_running_sessions_daemons_are_restarted_on_the_new_core() {
    let scratch = tmux_scratch("run");
    if !tmux_present(&scratch) {
        let _ = remove(&scratch);
        panic!(
            "tmux is not runnable here, so the running-session half of the upgrade sweep cannot \
             be proven; install tmux or run this suite where one exists"
        );
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let session = "wdmig";

    let old_core = scratch.join("old-core");
    let new_core = scratch.join("new-core");
    write_exec(&old_core, FAKE_CORE);
    write_exec(&new_core, FAKE_CORE);

    let dir = plant_running(&scratch, &socket, &root, session, &old_core);
    let before = ae::watchdog_glue::read_pid(&dir).expect("a pidfile");
    let agent_pane = make_unhooked_lead_pair(&socket, &scratch, &dir, session);
    let (_, session_id) = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-t", session, "#{session_id}"],
    );
    let session_id = session_id.trim().to_owned();
    let old_hook = format!("select-window -t {agent_pane} ; select-pane -t {agent_pane}");
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "set-hook",
                "-t",
                &agent_pane,
                "client-session-changed",
                &old_hook,
            ],
        )
        .0,
        "plant the unguarded pre-upgrade focus hook"
    );
    assert_tmux_default_mouse_binding(&socket, &scratch);

    let notes = ae::migrate::onto(&root, &new_core, "2026.9.9").expect("the sweep");

    assert_ae_status_bindings(&socket, &scratch);
    assert_lead_pair_policy(&socket, &scratch, &agent_pane, session);
    let (_, focus_hook) = tmux(
        &socket,
        &scratch,
        &["show-hooks", "-t", session, "client-session-changed"],
    );
    assert!(
        focus_hook.contains(&format!("#{{==:#{{session_id}},{session_id}}}"))
            && focus_hook.contains(&old_hook),
        "upgrade guards the focus hook with its captured session id: {focus_hook}"
    );
    let (_, published) = tmux(
        &socket,
        &scratch,
        &[
            "show-options",
            "-v",
            "-t",
            "=wdmig:",
            ae::theme::MAIN_PANE_OPTION,
        ],
    );
    assert_eq!(
        published.trim(),
        agent_pane,
        "the upgrade publishes the main pane only after the live membership proof"
    );

    // The meta and every helper now name the new core.
    let text = meta_of(&dir);
    assert_eq!(
        value_of(&text, "ae_core").as_deref(),
        Some(new_core.to_string_lossy().as_ref()),
        "{text}"
    );
    assert_eq!(
        fs::read_link(dir.join("watchdog")).ok(),
        Some(new_core.clone()),
        "the watchdog helper still names the old core"
    );

    // The watchdog is a NEW process, in a live stamped pane.
    let after = ae::watchdog_glue::read_pid(&dir).expect("a republished pidfile");
    assert_ne!(after, before, "the watchdog was not restarted");
    let (_, panes) = tmux(
        &socket,
        &scratch,
        &["list-panes", "-s", "-t", session, "-F", "#{@ae_agent}"],
    );
    assert!(
        panes.lines().any(|line| line == "_watchdog"),
        "no stamped watchdog pane after the restart: {panes:?}"
    );
    // Its command word is the new core, reached through the re-rendered helper.
    let (_, started) = tmux(
        &socket,
        &scratch,
        &[
            "list-panes",
            "-s",
            "-t",
            session,
            "-F",
            "#{@ae_agent} #{pane_start_command}",
        ],
    );
    assert!(
        started
            .lines()
            .any(|line| line.starts_with("_watchdog") && line.contains("/watchdog")),
        "the watchdog pane does not run the session helper: {started:?}"
    );

    // The bridge was replaced, on the new core.
    let (_, bridge) = tmux(
        &socket,
        &scratch,
        &[
            "list-panes",
            "-t",
            "ae-telegram",
            "-F",
            "#{pane_start_command}",
        ],
    );
    assert!(
        bridge.contains(&new_core.display().to_string()),
        "the bridge was not restarted on the new core: {bridge:?}"
    );
    assert!(
        notes
            .iter()
            .any(|note| note.starts_with("restarted the watchdog of wdmig on the new core")),
        "the sweep did not report the watchdog restart: {notes:?}"
    );
    assert!(
        notes.iter().any(|note| note.contains("telegram")),
        "the sweep did not report the bridge restart: {notes:?}"
    );

    // Agent panes are NEVER touched — the ORIGINAL pane, by id. A count would
    // be satisfied by a sweep that killed the agent and left the two monitor
    // panes standing, which is the failure this line exists to catch.
    let (_, after_panes) = tmux(
        &socket,
        &scratch,
        &["list-panes", "-s", "-t", session, "-F", "#{pane_id}"],
    );
    assert!(
        after_panes.lines().any(|line| line == agent_pane),
        "the agent pane {agent_pane} did not survive the sweep: {after_panes:?}"
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one real running-session migration from planted old layout through watchdog rewrite"
)]
fn upgrading_a_running_session_without_an_orchestrator_rewrites_the_menu_range() {
    let scratch = tmux_scratch("running-look");
    if !tmux_present(&scratch) {
        let _ = remove(&scratch);
        panic!("tmux is not runnable here, so running look migration cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let session = "lookmig";
    let old_core = scratch.join("old-core");
    write_exec(&old_core, FAKE_CORE);
    let dir = plant_running(&scratch, &socket, &root, session, &old_core);
    assert!(
        tmux(&socket, &scratch, &["kill-session", "-t", "ae-telegram"]).0,
        "this arm needs no bridge"
    );
    for (name, value) in [
        (ae::theme::LOOK_OPTION, "on"),
        (ae::theme::PALETTE_OPTION, "darcula"),
        (ae::theme::ICONS_OPTION, "on"),
        (ae::theme::MOTION_OPTION, "on"),
        (ae::theme::LOOK_STAMP_OPTION, "10:darcula:on:on"),
        ("status-format[1]", "old version layout"),
    ] {
        assert!(
            tmux(
                &socket,
                &scratch,
                &["set-option", "-t", session, name, value]
            )
            .0,
            "plant {name}"
        );
    }
    let (_, orchestrator) = tmux(
        &socket,
        &scratch,
        &[
            "show-option",
            "-v",
            "-t",
            session,
            ae::theme::ORCHESTRATOR_ID_OPTION,
        ],
    );
    assert!(
        orchestrator.trim().is_empty(),
        "no orchestrator: {orchestrator}"
    );

    let actual_core = Path::new(env!("CARGO_BIN_EXE_ae"));
    let notes = ae::migrate::onto(&root, actual_core, ae::VERSION).expect("the sweep");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut line = String::new();
    let mut stamp = String::new();
    let fleet_prefix = "#[align=left fg=#808080 bg=#313335]#[range=user|ae]#{?@ae_menu_open,#[bg=#214283 fg=#A9B7C6],} ≡#[norange]";
    let migrated = |line: &str, stamp: &str| {
        line.split_once("#[align=right")
            .is_some_and(|(fleet, right)| {
                fleet.starts_with(fleet_prefix)
                    && !fleet.contains(ae::theme::VERSION_OPTION)
                    && right.ends_with(
                        "#[range=user|ae-settings]#{?@ae_settings_open,#[bg=#214283 fg=#A9B7C6],} ⚙ #[norange]\n",
                    )
                    && !right.contains(ae::theme::VERSION_OPTION)
                    && stamp.trim() == "19:darcula:on:on"
            })
    };
    while Instant::now() < deadline {
        line = tmux(
            &socket,
            &scratch,
            &["show-option", "-v", "-t", session, "status-format[1]"],
        )
        .1;
        stamp = tmux(
            &socket,
            &scratch,
            &[
                "show-option",
                "-v",
                "-t",
                session,
                ae::theme::LOOK_STAMP_OPTION,
            ],
        )
        .1;
        if migrated(&line, &stamp) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        migrated(&line, &stamp),
        "running session kept its pre-upgrade menu layout or received a partial menu contract: {line:?}; stamp={stamp:?}; notes={notes:?}"
    );
    assert_eq!(
        stamp.trim(),
        "19:darcula:on:on",
        "the new format stamp did not land"
    );
    assert_ae_status_bindings(&socket, &scratch);
    assert_eq!(
        fs::read_link(dir.join("watchdog")).ok(),
        Some(actual_core.to_path_buf())
    );
}

#[test]
fn a_publish_starts_a_missing_watchdog_on_the_new_core() {
    let scratch = tmux_scratch("missing-watchdog");
    if !tmux_present(&scratch) {
        let _ = remove(&scratch);
        panic!(
            "tmux is not runnable here, so missing-watchdog recovery cannot be proven; install \
             tmux or run this suite where one exists"
        );
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let session = "wdmissing";
    let old_core = scratch.join("old-core");
    let new_core = scratch.join("new-core");
    write_exec(&old_core, FAKE_CORE);
    write_exec(&new_core, FAKE_CORE);
    let dir = plant_running(&scratch, &socket, &root, session, &old_core);
    let (_, watchdog) = tmux(
        &socket,
        &scratch,
        &[
            "list-panes",
            "-s",
            "-t",
            session,
            "-F",
            "#{@ae_agent}|#{pane_id}",
        ],
    );
    let watchdog_pane = watchdog
        .lines()
        .find_map(|line| line.strip_prefix("_watchdog|"))
        .unwrap_or_else(|| panic!("the planted watchdog pane: {watchdog:?}"));
    assert!(
        tmux(&socket, &scratch, &["kill-pane", "-t", watchdog_pane]).0,
        "the watchdog is removed before publish"
    );

    let notes = ae::migrate::onto(&root, &new_core, "2026.9.9").expect("the sweep");

    let pid = ae::watchdog_glue::read_pid(&dir).expect("publish started a watchdog");
    assert!(matches!(
        ae::watchdog_lifecycle::presence(
            &ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(socket.clone())),
            session,
            &dir,
        ),
        ae::watchdog_lifecycle::Presence::Running(seen) if seen == pid
    ));
    assert!(
        notes.iter().any(|note| {
            note.starts_with("started the watchdog of wdmissing on the new core (pid ")
        }),
        "the recovery note is missing: {notes:?}"
    );
}
