//! Session-state teardown on the pinned core.
//!
//! Two entries share ONE authority and ONE commit primitive:
//! * `_end-local-teardown <session-dir>` (P3.5) removes the canonical state of a
//!   `mode=local` session.
//! * `_end-nonlocal-teardown <session-dir> [--preserve]` (P3.6) removes the managed
//!   workdir (a `full` copy, or a `git` worktree) AND then the canonical state of a
//!   nonlocal session; `--preserve` keeps the workdir byte-for-byte and removes the
//!   canonical state only.
//!
//! Bash still owns confirmation/plan, the lifecycle lock, the verified tmux stop,
//! Git commit/fetch/push, provider-history purge, `kill_heartbeat` and the archive;
//! the core begins only after a successful archive/purge. `AE_CORE=` and every
//! legacy/unsupported shape fall back to the frozen bash `rm`/worktree path.
//!
//! ROOT AUTHORITY (B1). The sessions and worktrees roots are derived from the
//! ENVIRONMENT — `state_root()` (`AE_HOME` nonempty else `HOME/.ae`) → the
//! `sessions/` and `worktrees/` siblings — NEVER from the operand path. The operand
//! must be the configured `sessions/<name>` direct child (lexical equality) or the
//! core refuses: a request cannot point the core at an arbitrary directory to
//! delete. Only the FINAL root components are lstat-classified, so a symlinked
//! ancestor (a deployment's `/tmp`, a symlinked `HOME`) stays supported.
//!
//! Deletion is not an in-place `rm -rf`. The commit boundary is an atomic RENAME of
//! the resource into a sibling tombstone `.ending.<name>` under its own root: it
//! clears the canonical NAME first, so any partial-delete debris lives under a dead,
//! un-launchable name (the grammar forbids a leading dot) and cannot masquerade as a
//! live session or a resumable worktree. After the rename is fsynced the resource is
//! DURABLY gone; a crash leaves the tombstone as an inspectable recovery marker, and
//! the launch guard refuses to reuse the name silently. A git worktree is removed by
//! `git worktree remove --force` instead — git owns that lifecycle and its own
//! atomicity — and on a git refusal there is NO `rm -rf` fallback.
//!
//! The rename dest is keyed by pathname, so it shares the same-UID hostile
//! substitution residual documented at
//! [`archive::store::make_claim`](crate::archive::store): the lifecycle lock
//! serialises cooperating teardown/launch, and closing the hostile case needs
//! descriptor-relative `renameat` machinery outside std. Out of scope here.

use std::fs;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use crate::meta;
use crate::state::EXIT_FAILED;

/// `_end-local-teardown` core entry.
pub(crate) fn run(dir: &Path, out: &mut impl Write, err: &mut impl Write) -> io::Result<u8> {
    let _ = out; // this teardown speaks only on failure; success is silent (rc 0).

    let (name, roots) = match prelude(dir, err)? {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let bytes = match prove_identified_session_dir(dir, &name, err)? {
        Ok(b) => b,
        Err(code) => return Ok(code),
    };
    // Mode must be exactly local — nonlocal shapes route to `run_nonlocal`.
    if meta::first_value(&bytes, "mode") != Some(b"local".as_slice()) {
        let shown = meta_value(&bytes, "mode");
        writeln!(
            err,
            "teardown: '{name}' does not prove mode 'local' (meta records '{shown}', compared byte-exact) — refusing (only local teardown is this entry's)."
        )?;
        return Ok(EXIT_FAILED);
    }
    commit_teardown(dir, &roots.sessions, &name, "session", RECOVER_MANUAL, err)
}

/// `_end-nonlocal-teardown` core entry for `mode=full` (copy) and `mode=git`
/// (worktree).
pub(crate) fn run_nonlocal(
    dir: &Path,
    preserve: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let _ = out;

    let (name, roots) = match prelude(dir, err)? {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let bytes = match prove_identified_session_dir(dir, &name, err)? {
        Ok(b) => b,
        Err(code) => return Ok(code),
    };
    // Mode must be exactly full or git.
    let is_git = match meta::first_value(&bytes, "mode") {
        Some(b"git") => true,
        Some(b"full") => false,
        _ => {
            let shown = meta_value(&bytes, "mode");
            writeln!(
                err,
                "teardown: '{name}' does not prove mode 'full' or 'git' (meta records '{shown}', compared byte-exact) — refusing (nonlocal teardown is only for a managed copy or worktree)."
            )?;
            return Ok(EXIT_FAILED);
        }
    };

    // Worktrees ROOT authority (B1): the configured worktrees/ root must be a real
    // non-symlink dir, and the recorded work_dir must be EXACTLY its <name> child.
    if !matches!(classify_dir(&roots.worktrees), DirKind::RealDir) {
        writeln!(
            err,
            "teardown: the configured worktrees root '{}' is not a real directory — refusing.",
            roots.worktrees.display()
        )?;
        return Ok(EXIT_FAILED);
    }
    let managed = roots.worktrees.join(&name);
    if meta::first_value(&bytes, "work_dir") != Some(managed.as_os_str().as_bytes()) {
        let shown = meta_value(&bytes, "work_dir");
        writeln!(
            err,
            "teardown: '{name}' does not prove work_dir '{}' (meta records '{shown}', compared byte-exact) — refusing to act on a workdir outside the configured worktrees root.",
            managed.display()
        )?;
        return Ok(EXIT_FAILED);
    }

    if preserve {
        // Q2: before promising the workdir was preserved, prove it is a real
        // non-symlink dir; then leave it byte-for-byte and remove canonical only.
        if !matches!(classify_dir(&managed), DirKind::RealDir) {
            writeln!(
                err,
                "teardown: the managed working directory '{}' to preserve is not a real directory — refusing (cannot promise it was preserved).",
                managed.display()
            )?;
            return Ok(EXIT_FAILED);
        }
        return commit_teardown(dir, &roots.sessions, &name, "session", RECOVER_MANUAL, err);
    }

    // WORKDIR FIRST.
    let removed = if is_git {
        remove_git_worktree(&bytes, &managed, &name, err)?
    } else {
        remove_copy_workdir(&managed, &roots.worktrees, &name, err)?
    };
    if let Err(code) = removed {
        return Ok(code);
    }

    // The workdir is durably gone.
    commit_teardown(dir, &roots.sessions, &name, "session", RECOVER_ARCHIVE, err)
}

/// Shared front matter for both entries: derive the name, reject a bad grammar
/// or a `..` traversal, resolve the configured roots from the environment, and
/// prove the operand is the configured `sessions/<name>` direct child.
fn prelude(dir: &Path, err: &mut impl Write) -> io::Result<Result<(String, ConfiguredRoots), u8>> {
    let Some(name) = dir.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
        writeln!(
            err,
            "teardown: '{}' has no session-name component.",
            dir.display()
        )?;
        return Ok(Err(EXIT_FAILED));
    };
    // Re-validate the fresh session-name grammar.
    if !is_valid_session_name(&name) {
        writeln!(
            err,
            "teardown: '{name}' is not a grammar-valid session name — refusing (legacy names use the bash path)."
        )?;
        return Ok(Err(EXIT_FAILED));
    }
    // Reject any `..` traversal component: a `..` could let the basename grammar pass
    // while the path resolves elsewhere; bash passes no traversal.
    if dir
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        writeln!(
            err,
            "teardown: '{}' contains a '..' traversal component — refusing.",
            dir.display()
        )?;
        return Ok(Err(EXIT_FAILED));
    }
    let roots = match configured_roots(err)? {
        Ok(r) => r,
        Err(code) => return Ok(Err(code)),
    };
    if let Some(code) = prove_configured_sessions_child(dir, &name, &roots, err)? {
        return Ok(Err(code));
    }
    Ok(Ok((name, roots)))
}

/// The configured `sessions/` and `worktrees/` roots — the INDEPENDENT authority for
/// what a teardown may touch, derived from the environment, never from the operand.
struct ConfiguredRoots {
    sessions: PathBuf,
    worktrees: PathBuf,
}

/// Resolve the configured roots from `state_root()` (`AE_HOME` nonempty else
/// `HOME/.ae`).
fn configured_roots(err: &mut impl Write) -> io::Result<Result<ConfiguredRoots, u8>> {
    let Some(state_root) = crate::state_root() else {
        writeln!(err, "teardown: {} — refusing.", crate::NO_STATE_ROOT)?;
        return Ok(Err(EXIT_FAILED));
    };
    let roots = crate::inventory::Roots::under(state_root);
    Ok(Ok(ConfiguredRoots {
        sessions: roots.sessions().to_owned(),
        worktrees: roots.worktrees().to_owned(),
    }))
}

/// Prove the operand is the configured `sessions/<name>` direct child (B1): the
/// sessions ROOT (from the environment) is a real non-symlink dir, and the
/// operand is LEXICALLY that root's `<name>` child.
fn prove_configured_sessions_child(
    dir: &Path,
    name: &str,
    roots: &ConfiguredRoots,
    err: &mut impl Write,
) -> io::Result<Option<u8>> {
    if !matches!(classify_dir(&roots.sessions), DirKind::RealDir) {
        writeln!(
            err,
            "teardown: the configured sessions root '{}' is not a real directory — refusing.",
            roots.sessions.display()
        )?;
        return Ok(Some(EXIT_FAILED));
    }
    let expected = roots.sessions.join(name);
    if !same_path_lexically(dir, &expected) {
        writeln!(
            err,
            "teardown: '{}' is not the configured session directory '{}' — refusing to act outside the configured sessions root.",
            dir.display(),
            expected.display()
        )?;
        return Ok(Some(EXIT_FAILED));
    }
    Ok(None)
}

/// Lexical path equality by components — normalises `//`, a trailing `/` and a
/// `.` component, and never resolves a symlink.
fn same_path_lexically(a: &Path, b: &Path) -> bool {
    a.components().eq(b.components())
}

/// lstat the session dir as a real non-symlink directory, read its meta as a
/// real plain file, and prove `meta.session == name` byte-exact.
fn prove_identified_session_dir(
    dir: &Path,
    name: &str,
    err: &mut impl Write,
) -> io::Result<Result<Vec<u8>, u8>> {
    match classify_dir(dir) {
        DirKind::RealDir => {}
        DirKind::Absent => {
            writeln!(
                err,
                "teardown: '{}' is already absent — refusing (the caller expected a live session to remove).",
                dir.display()
            )?;
            return Ok(Err(EXIT_FAILED));
        }
        DirKind::Symlink | DirKind::NotDir => {
            writeln!(
                err,
                "teardown: '{}' is not a real session directory — refusing to follow or delete it.",
                dir.display()
            )?;
            return Ok(Err(EXIT_FAILED));
        }
        DirKind::StatErr(why) => {
            writeln!(
                err,
                "teardown: cannot stat '{}' ({why}) — refusing.",
                dir.display()
            )?;
            return Ok(Err(EXIT_FAILED));
        }
    }
    let bytes = match read_identity_meta(dir, err)? {
        Ok(b) => b,
        Err(code) => return Ok(Err(code)),
    };
    // Identity is proved on the RAW meta bytes, byte-exact.
    if meta::first_value(&bytes, "session") != Some(name.as_bytes()) {
        let shown = meta_value(&bytes, "session");
        writeln!(
            err,
            "teardown: '{}' does not prove session '{name}' (meta records '{shown}', compared byte-exact) — refusing to delete an unproven directory.",
            dir.display()
        )?;
        return Ok(Err(EXIT_FAILED));
    }
    Ok(Ok(bytes))
}

/// Read `<dir>/meta` ONLY if it is a real plain file.
fn read_identity_meta(dir: &Path, err: &mut impl Write) -> io::Result<Result<Vec<u8>, u8>> {
    let meta_path = dir.join("meta");
    match symlink_meta(&meta_path) {
        Ok(m) if m.file_type().is_file() => {}
        Ok(_) => {
            writeln!(
                err,
                "teardown: '{}' is not a plain file — refusing to prove identity through a link, FIFO or special node.",
                meta_path.display()
            )?;
            return Ok(Err(EXIT_FAILED));
        }
        Err(why) if why.kind() == io::ErrorKind::NotFound => {
            writeln!(
                err,
                "teardown: '{}' has no readable meta ({why}) — refusing to delete a session it cannot identify.",
                dir.display()
            )?;
            return Ok(Err(EXIT_FAILED));
        }
        Err(why) => {
            writeln!(
                err,
                "teardown: cannot stat '{}' ({why}) — refusing.",
                meta_path.display()
            )?;
            return Ok(Err(EXIT_FAILED));
        }
    }
    // Now a proven plain file — no link followed, no FIFO to block.
    match meta::read_bytes(dir) {
        Ok(b) => Ok(Ok(b)),
        Err(why) => {
            writeln!(
                err,
                "teardown: '{}' has no readable meta ({why}) — refusing to delete a session it cannot identify.",
                dir.display()
            )?;
            Ok(Err(EXIT_FAILED))
        }
    }
}

/// The `full` (copy) workdir removal: the P3.5 rename-to-tombstone primitive
/// applied to the managed copy under the worktrees root.
fn remove_copy_workdir(
    managed: &Path,
    worktrees_root: &Path,
    name: &str,
    err: &mut impl Write,
) -> io::Result<Result<(), u8>> {
    // The managed copy must be a real non-symlink dir before we rename it.
    match classify_dir(managed) {
        DirKind::RealDir => {}
        DirKind::Absent => {
            writeln!(
                err,
                "teardown: the managed working copy '{}' is already absent — refusing (the session state is RETAINED; retry ae end once the workdir is restored, or remove the session by hand).",
                managed.display()
            )?;
            return Ok(Err(EXIT_FAILED));
        }
        _ => {
            writeln!(
                err,
                "teardown: the managed working copy '{}' is not a real directory — refusing (the session state is RETAINED).",
                managed.display()
            )?;
            return Ok(Err(EXIT_FAILED));
        }
    }
    match commit_teardown(
        managed,
        worktrees_root,
        name,
        "working copy",
        RECOVER_MANUAL,
        err,
    )? {
        0 => Ok(Ok(())),
        code => Ok(Err(code)),
    }
}

/// The `git` (worktree) workdir removal: a sealed `git worktree remove --force`
/// through the one process door (no `prune` — that is global housekeeping).
fn remove_git_worktree(
    bytes: &[u8],
    managed: &Path,
    name: &str,
    err: &mut impl Write,
) -> io::Result<Result<(), u8>> {
    let Some(origin) = meta::first_value(bytes, "origin").filter(|o| !o.is_empty()) else {
        writeln!(
            err,
            "teardown: '{name}' records no origin for its git worktree — refusing (the session state is RETAINED)."
        )?;
        return Ok(Err(EXIT_FAILED));
    };
    // lstat the exact managed child BEFORE handing it to git: `git worktree
    // remove` resolves a symlink and would delete an EXTERNALLY registered
    // worktree at the link target.
    match classify_dir(managed) {
        DirKind::RealDir => {}
        DirKind::Absent => {
            writeln!(
                err,
                "teardown: the git worktree '{}' for '{name}' is already absent — refusing (the session state is RETAINED).",
                managed.display()
            )?;
            return Ok(Err(EXIT_FAILED));
        }
        _ => {
            writeln!(
                err,
                "teardown: the git worktree path '{}' for '{name}' is a link or special node, not a real directory — refusing to hand it to git (the session state is RETAINED).",
                managed.display()
            )?;
            return Ok(Err(EXIT_FAILED));
        }
    }
    if crate::git::worktree_remove(origin, managed.as_os_str().as_bytes()) {
        return Ok(Ok(()));
    }
    // git refused.
    if matches!(classify_dir(managed), DirKind::RealDir) {
        writeln!(
            err,
            "teardown: git refused to remove the worktree '{}' for '{name}' — BOTH the working tree and the session state are RETAINED.",
            managed.display()
        )?;
        writeln!(
            err,
            "  The worktree may be locked or dirty; resolve it there, then retry ae end."
        )?;
    } else {
        writeln!(
            err,
            "teardown: git worktree remove for '{name}' failed and '{}' is no longer a normal directory (PARTIAL/UNKNOWN) — the session state is RETAINED.",
            managed.display()
        )?;
        writeln!(
            err,
            "  Inspect the worktree and the durable archive, then remove the session by hand; do not run ae end again."
        )?;
    }
    Ok(Err(EXIT_FAILED))
}

/// The recovery guidance a canonical-state failure prints, tracking commit
/// state.
const RECOVER_MANUAL: &str = "Inspect and remove it by hand to recover; the name is already cleared, so it cannot resurrect.";
/// `RECOVER_ARCHIVE`: the workdir was already committed away, so `ae end` cannot be
/// retried — its repo precondition fails with the workdir gone.
const RECOVER_ARCHIVE: &str = "The workdir is already gone, so ae end cannot be retried; inspect the durable archive, then remove the tombstone by hand.";

/// Rename a resource dir into its sibling tombstone (the commit boundary), then
/// durably remove it.
fn commit_teardown(
    dir: &Path,
    root: &Path,
    name: &str,
    resource: &str,
    recovery: &str,
    err: &mut impl Write,
) -> io::Result<u8> {
    let tombstone = root.join(format!(".ending.{name}"));
    // A standing tombstone is a previous teardown that did not complete:
    // refuse, never overwrite it.
    if symlink_meta(&tombstone).is_ok() {
        writeln!(
            err,
            "teardown: {} is standing — a previous teardown of the {resource} of '{name}' did not complete.",
            tombstone.display()
        )?;
        writeln!(err, "  Inspect it, then remove it by hand before retrying.")?;
        return Ok(EXIT_FAILED);
    }
    // COMMIT: the atomic rename clears the canonical name.
    if let Err(why) = fs::rename(dir, &tombstone) {
        writeln!(
            err,
            "teardown: could not remove the {resource} '{}' (rename: {why}) — nothing was deleted.",
            dir.display()
        )?;
        return Ok(EXIT_FAILED);
    }
    // Make the rename durable BEFORE removing anything: once the root is synced the
    // resource is DURABLY gone from its name and cannot resurrect.
    if let Err(why) = fsync_dir(root) {
        return incomplete_retained(
            err,
            &tombstone,
            name,
            resource,
            recovery,
            &format!("its durability is unconfirmed (fsync root: {why})"),
        );
    }
    if let Err(why) = fs::remove_dir_all(&tombstone) {
        return incomplete_retained(
            err,
            &tombstone,
            name,
            resource,
            recovery,
            &format!("the state could not be removed (remove: {why})"),
        );
    }
    if let Err(why) = fsync_dir(root) {
        return removed_unsynced(
            err,
            &tombstone,
            name,
            resource,
            &format!("fsync root: {why}"),
        );
    }
    // The resource is gone and the removal is durable.
    Ok(0)
}

/// A POST-rename failure with the tombstone STILL present: the resource was
/// removed from its name but is not yet durably gone.
fn incomplete_retained(
    err: &mut impl Write,
    tombstone: &Path,
    name: &str,
    resource: &str,
    recovery: &str,
    reason: &str,
) -> io::Result<u8> {
    writeln!(
        err,
        "teardown: INCOMPLETE for the {resource} of '{name}' — it was removed from its name but {reason}."
    )?;
    writeln!(
        err,
        "  The tombstone {} is RETAINED and may still hold {resource} state. {recovery}",
        tombstone.display()
    )?;
    Ok(EXIT_FAILED)
}

/// A POST-removal failure: the tombstone is already gone and only the FINAL
/// sync failed.
fn removed_unsynced(
    err: &mut impl Write,
    tombstone: &Path,
    name: &str,
    resource: &str,
    reason: &str,
) -> io::Result<u8> {
    writeln!(
        err,
        "teardown: DURABILITY UNCONFIRMED for the {resource} of '{name}' — durably removed from its name and cannot resurrect, but the tombstone removal is not confirmed on disk ({reason})."
    )?;
    writeln!(
        err,
        "  If {} is present, remove it by hand; nothing can resurrect from it.",
        tombstone.display()
    )?;
    Ok(EXIT_FAILED)
}

/// lstat classification of a path's final component, never following a link.
enum DirKind {
    RealDir,
    Symlink,
    NotDir,
    Absent,
    StatErr(io::Error),
}

fn classify_dir(path: &Path) -> DirKind {
    match symlink_meta(path) {
        Ok(m) if m.file_type().is_symlink() => DirKind::Symlink,
        Ok(m) if !m.is_dir() => DirKind::NotDir,
        Ok(_) => DirKind::RealDir,
        Err(e) if e.kind() == io::ErrorKind::NotFound => DirKind::Absent,
        Err(e) => DirKind::StatErr(e),
    }
}

/// The fresh session-name grammar the bash `_validate_session_name` enforces:
/// `^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$` — a leading alphanumeric, then up to 127
/// more of alphanumeric / `_` / `-`.
fn is_valid_session_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    match bytes.split_first() {
        Some((first, rest)) if bytes.len() <= 128 => {
            first.is_ascii_alphanumeric()
                && rest
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
        }
        _ => false,
    }
}

/// The FIRST `<key>=` value from meta bytes, trimmed of surrounding whitespace (a
/// trailing CR from a hand-edited meta included) — used ONLY to DISPLAY what was
/// recorded in a refusal, never to decide (decisions compare raw bytes).
fn meta_value(bytes: &[u8], key: &str) -> String {
    meta::first_value(bytes, key)
        .map(|v| String::from_utf8_lossy(v).trim().to_owned())
        .unwrap_or_default()
}

/// `symlink_metadata` (lstat) — classifies a node without following it.
fn symlink_meta(path: &Path) -> io::Result<fs::Metadata> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: lstat to classify a root / session dir / workdir / tombstone without following a link — see clippy.toml"
    )]
    let meta = std::fs::symlink_metadata(path);
    meta
}

/// `File::open` of a directory, to `fsync` it — how a rename or a removal is made
/// durable on Unix.
fn fsync_dir(dir: &Path) -> io::Result<()> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: opens a root to fsync it for durable teardown — see clippy.toml"
    )]
    let handle = std::fs::File::open(dir)?;
    handle.sync_all()
}

#[cfg(test)]
mod tests {
    use super::{
        RECOVER_ARCHIVE, RECOVER_MANUAL, incomplete_retained, is_valid_session_name,
        removed_unsynced,
    };
    use crate::state::EXIT_FAILED;
    use std::path::Path;

    #[test]
    fn the_fresh_name_grammar_is_enforced() {
        for ok in ["a", "A9", "sess-1_2", "x".repeat(128).as_str()] {
            assert!(is_valid_session_name(ok), "accepts {ok:?}");
        }
        for bad in [
            "",
            "_x",
            "-x",
            ".x",
            ".ending.x",
            "a/b",
            "a.b",
            "..",
            ".",
            "a b",
            &"y".repeat(129),
        ] {
            assert!(!is_valid_session_name(bad), "rejects {bad:?}");
        }
    }

    #[test]
    fn the_two_post_rename_states_are_reported_distinctly() {
        let tomb = Path::new("/s/.ending.demo");

        let mut retained = Vec::new();
        let rc_r = incomplete_retained(
            &mut retained,
            tomb,
            "demo",
            "session",
            RECOVER_MANUAL,
            "reason",
        )
        .unwrap();
        let retained = String::from_utf8(retained).unwrap();

        let mut removed = Vec::new();
        let rc_m =
            removed_unsynced(&mut removed, tomb, "demo", "session", "fsync root: x").unwrap();
        let removed = String::from_utf8(removed).unwrap();

        assert_eq!(rc_r, EXIT_FAILED);
        assert_eq!(rc_m, EXIT_FAILED);
        // The retained state keeps a marker and says so; the removed state says the
        // resource is durably gone and never claims a retained marker.
        assert!(
            retained.contains("is RETAINED"),
            "retained names the marker"
        );
        assert!(
            removed.contains("durably removed") && removed.contains("cannot resurrect"),
            "removed-unsynced states the resource is gone"
        );
        assert!(
            !removed.contains("RETAINED"),
            "removed-unsynced never claims a retained marker"
        );
    }

    #[test]
    fn the_recovery_guidance_tracks_commit_state() {
        // Manual recovery never claims ae end can be retried; archive recovery says
        // ae end cannot be retried and points at the durable archive.
        assert!(!RECOVER_MANUAL.contains("ae end"));
        assert!(RECOVER_ARCHIVE.contains("ae end cannot be retried"));
        assert!(RECOVER_ARCHIVE.contains("archive"));
    }
}
