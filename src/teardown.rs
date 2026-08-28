//! `_end-local-teardown <session-dir>` — removes the canonical session state of a
//! LOCAL-mode session from the sessions root (P3.5).
//!
//! Bash routes here from `cleanup_session` for a `mode=local` session whose name
//! is grammar-valid; it keeps confirmation/plan, the lifecycle lock, tmux stop,
//! Git, provider-history purge, `kill_heartbeat`, legacy-path cleanup, and the
//! local/nonlocal branch. Legacy grammar-invalid-but-usable local names (an
//! existing-session consumer accepts them) STAY on the bash `rm` fallback so they
//! are not made un-endable; the core re-validates the fresh grammar for every
//! request it receives. `AE_CORE=` falls back to the frozen `rm -rf`.
//!
//! Deletion is not an in-place `rm -rf`. The commit boundary is an atomic RENAME
//! of the session dir into a sibling tombstone `.ending.<name>`: it clears the
//! canonical NAME first, so any partial-delete debris lives under a dead,
//! un-launchable name (the grammar forbids a leading dot) and can never masquerade
//! as a session. After the rename is fsynced the session is DURABLY gone and
//! cannot resurrect; a crash then leaves the tombstone as an inspectable recovery
//! marker, and the launch guard refuses to reuse the name silently.
//!
//! The rename dest is keyed by pathname, so it shares the same-UID hostile
//! substitution residual documented at
//! [`archive::store::make_claim`](crate::archive::store): the lifecycle lock
//! serialises cooperating teardown/launch, and closing the hostile case needs
//! descriptor-relative `renameat` machinery outside std. Out of scope here.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use crate::meta;
use crate::state::EXIT_FAILED;

/// `_end-local-teardown` core entry. `0` once the canonical session state is gone
/// AND the removal is durable; [`EXIT_FAILED`] with a named refusal, or an
/// explicit recoverable state, otherwise. Never follows a link, never deletes a
/// path it cannot prove is this local session's, and never touches origin or
/// `work_dir`.
pub(crate) fn run(dir: &Path, out: &mut impl Write, err: &mut impl Write) -> io::Result<u8> {
    let _ = out; // this teardown speaks only on failure; success is silent (rc 0).

    let Some(name) = dir.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
        writeln!(
            err,
            "teardown: '{}' has no session-name component.",
            dir.display()
        )?;
        return Ok(EXIT_FAILED);
    };
    let Some(root) = dir.parent() else {
        writeln!(err, "teardown: '{}' has no sessions root.", dir.display())?;
        return Ok(EXIT_FAILED);
    };

    // C1: re-validate the FRESH session-name grammar for every request. Bash routes
    // only grammar-valid local names here; a legacy name reaching this point is a
    // routing error, refused rather than acted on. The grammar forbids '/', '.',
    // '..' and a leading dot, so the name is a safe direct-child basename.
    if !is_valid_session_name(&name) {
        writeln!(
            err,
            "teardown: '{name}' is not a grammar-valid session name — refusing (legacy names use the bash path)."
        )?;
        return Ok(EXIT_FAILED);
    }

    // Reject any `..` traversal component. A `..` can let the basename grammar check
    // pass while the path resolves to a different directory; bash passes
    // `${SESSIONS_DIR}/${session_name}` with none, so a request carrying one is
    // malformed — refused before any read or delete.
    if dir
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        writeln!(
            err,
            "teardown: '{}' contains a '..' traversal component — refusing.",
            dir.display()
        )?;
        return Ok(EXIT_FAILED);
    }

    if let Some(code) = prove_real_local_dir(dir, root, &name, err)? {
        return Ok(code);
    }

    commit_teardown(dir, root, &name, err)
}

/// Prove the dir is a REAL session directory (never a symlink, never followed)
/// whose recorded identity is this local session (`meta.session == name` AND
/// `meta.mode == local`). `Ok(None)` to proceed; `Ok(Some(code))` after writing a
/// named refusal — nothing has been mutated at this point.
fn prove_real_local_dir(
    dir: &Path,
    root: &Path,
    name: &str,
    err: &mut impl Write,
) -> io::Result<Option<u8>> {
    // The sessions ROOT must be a real directory, never a symlink: a symlinked root
    // would let the rename+remove act THROUGH the link on a directory outside the
    // intended sessions tree (`lstat` on the target's final component alone does not
    // see it). lstat classifies the root without following it. Symlinks ABOVE the
    // sessions root — a deployment's /tmp, a symlinked HOME — are the operator's, not
    // an attack surface, so the check stops at the root, the one component this core
    // owns the meaning of.
    match symlink_meta(root) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
            writeln!(
                err,
                "teardown: the sessions root '{}' is not a real directory — refusing to act through a link.",
                root.display()
            )?;
            return Ok(Some(EXIT_FAILED));
        }
        Ok(_) => {}
        Err(why) => {
            writeln!(
                err,
                "teardown: cannot stat the sessions root '{}' ({why}) — refusing.",
                root.display()
            )?;
            return Ok(Some(EXIT_FAILED));
        }
    }

    match symlink_meta(dir) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
            writeln!(
                err,
                "teardown: '{}' is not a real session directory — refusing to follow or delete it.",
                dir.display()
            )?;
            return Ok(Some(EXIT_FAILED));
        }
        Ok(_) => {}
        Err(why) if why.kind() == io::ErrorKind::NotFound => {
            writeln!(
                err,
                "teardown: '{}' is already absent — refusing (the caller expected a live session to remove).",
                dir.display()
            )?;
            return Ok(Some(EXIT_FAILED));
        }
        Err(why) => {
            writeln!(
                err,
                "teardown: cannot stat '{}' ({why}) — refusing.",
                dir.display()
            )?;
            return Ok(Some(EXIT_FAILED));
        }
    }

    let bytes = match read_identity_meta(dir, err)? {
        Ok(b) => b,
        Err(code) => return Ok(Some(code)),
    };
    // Identity is proved on the RAW meta bytes, byte-exact. `meta::first_value`
    // returns the value WITHOUT its line terminator but WITH any trailing spaces,
    // tabs or CR (the frozen `read_session_meta` preserves them). A destructive op
    // must never normalize an unproven directory into a match: `session=demo ` is NOT
    // `demo`, and `mode=local\t` is NOT `local`. `meta_value` is used only to DISPLAY
    // what was recorded, never to decide.
    if meta::first_value(&bytes, "session") != Some(name.as_bytes()) {
        let shown = meta_value(&bytes, "session");
        writeln!(
            err,
            "teardown: '{}' does not prove session '{name}' (meta records '{shown}', compared byte-exact) — refusing to delete an unproven directory.",
            dir.display()
        )?;
        return Ok(Some(EXIT_FAILED));
    }
    if meta::first_value(&bytes, "mode") != Some(b"local".as_slice()) {
        let shown = meta_value(&bytes, "mode");
        writeln!(
            err,
            "teardown: '{name}' does not prove mode 'local' (meta records '{shown}', compared byte-exact) — refusing (only local teardown is the core's)."
        )?;
        return Ok(Some(EXIT_FAILED));
    }
    Ok(None)
}

/// Read `<dir>/meta` ONLY if it is a real plain file. Proving the DIR is real does
/// not prove `<dir>/meta` is: `meta::read_bytes` is `fs::read`, which FOLLOWS a
/// symlink and BLOCKS on a FIFO. A symlinked meta pointing at an attacker-controlled
/// file holding `session=demo`/`mode=local` would forge the identity proof from
/// OUTSIDE the session dir; a FIFO would hang `ae end` under the lifecycle lock. So
/// lstat and require a plain file, refusing every other existing node. The read that
/// follows is of a proven regular file — the residual swap-after-lstat is the same-UID
/// race documented at [`archive::store::make_claim`](crate::archive::store), out of
/// scope here. `Ok(Ok(bytes))` on success; `Ok(Err(code))` after a named refusal.
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

/// Under a proven identity: rename the session dir to its sibling tombstone (the
/// commit boundary), then durably remove it. No `rc 0` while any session byte
/// could remain at the canonical name or under an unconfirmed removal.
fn commit_teardown(dir: &Path, root: &Path, name: &str, err: &mut impl Write) -> io::Result<u8> {
    let tombstone = root.join(format!(".ending.{name}"));
    // A standing tombstone is a previous teardown that did not complete: refuse,
    // never overwrite it. The human recovers it (there is no end-resume path).
    if symlink_meta(&tombstone).is_ok() {
        writeln!(
            err,
            "teardown: {} is standing — a previous teardown of '{name}' did not complete.",
            tombstone.display()
        )?;
        writeln!(err, "  Inspect it, then remove it by hand before retrying.")?;
        return Ok(EXIT_FAILED);
    }
    // COMMIT: the atomic rename clears the canonical name. Before it, nothing is
    // deleted; a failure here leaves the session intact.
    if let Err(why) = fs::rename(dir, &tombstone) {
        writeln!(
            err,
            "teardown: could not remove '{}' (rename: {why}) — nothing was deleted.",
            dir.display()
        )?;
        return Ok(EXIT_FAILED);
    }
    // Make the rename durable BEFORE removing anything: once the sessions root is
    // synced the session is DURABLY gone from its name and cannot resurrect.
    if let Err(why) = fsync_dir(root) {
        return incomplete_retained(
            err,
            &tombstone,
            name,
            &format!("its durability is unconfirmed (fsync root: {why})"),
        );
    }
    // Past here the session is durably gone. Remove the tombstone, then sync.
    if let Err(why) = fs::remove_dir_all(&tombstone) {
        return incomplete_retained(
            err,
            &tombstone,
            name,
            &format!("the state could not be removed (remove: {why})"),
        );
    }
    if let Err(why) = fsync_dir(root) {
        return removed_unsynced(err, &tombstone, name, &format!("fsync root: {why}"));
    }

    // The canonical session state is gone and the removal is durable. Silent success.
    Ok(0)
}

/// A POST-rename failure with the tombstone STILL present: the session was removed
/// from its name but its state is not yet durably gone. NO success; RETAIN the
/// tombstone as an inspectable recovery marker; tell the human the ONLY recovery
/// that works — inspect and remove it by hand, then retry — never "re-run end",
/// which cannot find a session whose canonical name is already gone. rc 1.
fn incomplete_retained(
    err: &mut impl Write,
    tombstone: &Path,
    name: &str,
    reason: &str,
) -> io::Result<u8> {
    writeln!(
        err,
        "teardown: INCOMPLETE for '{name}' — the session was removed from its name but {reason}."
    )?;
    writeln!(
        err,
        "  The tombstone {} is RETAINED and may still hold session state. Inspect and remove it by hand, then retry the intended operation.",
        tombstone.display()
    )?;
    Ok(EXIT_FAILED)
}

/// A POST-removal failure: the tombstone is already gone and only the FINAL sync
/// failed. The session is DURABLY removed from its name (synced at the rename) and
/// cannot resurrect as the old session; only the tombstone's removal durability is
/// unconfirmed, so it may transiently reappear. Distinct no-retained-marker state.
/// rc 1.
fn removed_unsynced(
    err: &mut impl Write,
    tombstone: &Path,
    name: &str,
    reason: &str,
) -> io::Result<u8> {
    writeln!(
        err,
        "teardown: DURABILITY UNCONFIRMED for '{name}' — the session is durably removed from its name and cannot resurrect, but the tombstone removal is not confirmed on disk ({reason})."
    )?;
    writeln!(
        err,
        "  If {} is present, remove it by hand; no session can resurrect from it.",
        tombstone.display()
    )?;
    Ok(EXIT_FAILED)
}

/// The fresh session-name grammar the bash `_validate_session_name` enforces:
/// `^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$` — a leading alphanumeric, then up to 127
/// more of alphanumeric / `_` / `-`. Forbids empty, over-long, a leading `.`,
/// `_` or `-`, and every traversal or separator byte.
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

/// The FIRST `<key>=` value from meta bytes, trimmed of surrounding whitespace
/// (a trailing CR from a hand-edited meta included) — the identity strings this
/// compares are ASCII with no meaningful edge whitespace.
fn meta_value(bytes: &[u8], key: &str) -> String {
    meta::first_value(bytes, key)
        .map(|v| String::from_utf8_lossy(v).trim().to_owned())
        .unwrap_or_default()
}

/// `symlink_metadata` (lstat) — classifies a node without following it.
fn symlink_meta(path: &Path) -> io::Result<fs::Metadata> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: lstat to classify the session dir / tombstone without following a link — see clippy.toml"
    )]
    let meta = std::fs::symlink_metadata(path);
    meta
}

/// `File::open` of a directory, to `fsync` it — how a rename or a removal is made
/// durable on Unix.
fn fsync_dir(dir: &Path) -> io::Result<()> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: opens the sessions root to fsync it for durable teardown — see clippy.toml"
    )]
    let handle = std::fs::File::open(dir)?;
    handle.sync_all()
}

#[cfg(test)]
mod tests {
    use super::{incomplete_retained, is_valid_session_name, removed_unsynced};
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
        let rc_r = incomplete_retained(&mut retained, tomb, "demo", "reason").unwrap();
        let retained = String::from_utf8(retained).unwrap();

        let mut removed = Vec::new();
        let rc_m = removed_unsynced(&mut removed, tomb, "demo", "fsync root: x").unwrap();
        let removed = String::from_utf8(removed).unwrap();

        assert_eq!(rc_r, EXIT_FAILED);
        assert_eq!(rc_m, EXIT_FAILED);
        // The retained state keeps a marker and says so; the removed state says the
        // session is durably gone and never claims a retained marker.
        assert!(
            retained.contains("is RETAINED"),
            "retained names the marker"
        );
        assert!(
            removed.contains("durably removed") && removed.contains("cannot resurrect"),
            "removed-unsynced states the session is gone"
        );
        assert!(
            !removed.contains("RETAINED"),
            "removed-unsynced never claims a retained marker"
        );
        // Neither recovery line tells the human to re-run end (it cannot work).
        assert!(!retained.contains("re-run") && !removed.contains("re-run"));
    }
}
