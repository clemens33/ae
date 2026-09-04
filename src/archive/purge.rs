//! `_archive-purge <session-dir> <aid> <source-session> <parent-id>` — the
//! `--purge-history` archive deletion (P3.4).
//!
//! Bash routes the frozen `_ar_purge_archive` here; it keeps stop/git, live-state
//! cleanup and compact. The core proves the archive is provably THIS session's,
//! deletes it under the shared `.publishing.<aid>` claim, and — because purge is a
//! PRIVACY promise — reports success ONLY after the bytes are gone AND the removal
//! is durable. `AE_CORE=` falls back to the frozen body.
//!
//! Deletion is not an in-place `rm -rf`. The commit boundary is an atomic RENAME
//! of the target into the claim (`claim/doomed`), symmetric with publication's
//! rename: after it the archive no longer exists at its canonical id path, and
//! that path is NEVER left half-deleted (which a mid-way `rm -rf` risks, and which
//! would then fail [`validate_tree`](super::store::validate_tree) and be
//! un-repurgeable). A failure BEFORE the rename deletes nothing and clears our own
//! claim; a failure AFTER it never reports success and names its TRUE state —
//! while the doomed archive is still under the claim, that claim is RETAINED as a
//! recovery marker; once the claim and its bytes are already gone and only the
//! final durability is unconfirmed, it reports that distinct no-marker state
//! instead ([`postcommit_retained`] vs [`postcommit_removed_unsynced`]).

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use super::store::{
    RootState, archive_root_of, claim_path, exists, fsync_dir, make_claim, read_file,
    require_real_root, symlink_meta, validate_tree,
};
use super::{canonical_uuid, meta_get};
use crate::state::EXIT_FAILED;

/// `_archive-purge` core entry.
pub(crate) fn run(
    dir: &Path,
    aid_arg: &str,
    source_session: &str,
    parent_id: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    // The id builds a path that ends at `rm`; it is never trusted from argv.
    let aid = canonical_uuid(aid_arg);
    if aid.is_empty() {
        writeln!(
            err,
            "archive: '{aid_arg}' is not an archive UUID — refusing to purge."
        )?;
        return Ok(EXIT_FAILED);
    }
    // Never purge the parent this session launched from.
    let parent = canonical_uuid(parent_id);
    if !parent.is_empty() && parent == aid {
        writeln!(
            err,
            "archive: refusing to purge {aid} — it is the parent archive this session was launched from."
        )?;
        return Ok(EXIT_FAILED);
    }

    let Some(root) = archive_root_of(dir) else {
        writeln!(err, "archive: '{}' has no archive root.", dir.display())?;
        return Ok(EXIT_FAILED);
    };
    match require_real_root(&root, false) {
        // A real root: proceed.
        Ok(RootState::Present) => {}
        Ok(RootState::Absent) => return Ok(0),
        // A symlinked root has already been refused, loudly, by the proof itself.
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(EXIT_FAILED);
        }
    }

    // Serialize with publication on the primitive it uses: whoever wins the
    // mkdir owns this id.
    let claim = claim_path(&root, &aid);
    if make_claim(&claim).is_err() {
        writeln!(
            err,
            "archive: {} is standing — another publisher may be mid-publication of {aid}.",
            claim.display()
        )?;
        writeln!(
            err,
            "  Refusing to purge behind it. Inspect that claim, then remove it by hand if it is stale."
        )?;
        return Ok(EXIT_FAILED);
    }
    // Make the claim entry durable (P3.3's root fsync) before anything destructive.
    if let Err(why) = fsync_dir(&root) {
        return precommit_fail(
            &root,
            &claim,
            err,
            &format!("archive: could not make the purge claim durable (fsync root: {why})."),
        );
    }

    purge_owned(&root, &claim, &aid, source_session, out, err)
}

/// Under our durable claim: prove ownership, then commit the deletion by
/// renaming the target into the claim and durably removing it.
fn purge_owned(
    root: &Path,
    claim: &Path,
    aid: &str,
    source_session: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let target = root.join(aid);
    if !exists(&target) {
        // Absent target after OUR claim: nothing to purge — clear the claim
        // durably and succeed with no output.
        return match clear_claim(root, claim) {
            Ok(()) => Ok(0),
            Err(diag) => {
                writeln!(err, "{diag}")?;
                Ok(EXIT_FAILED)
            }
        };
    }
    // A real archive directory, never a link or other node.
    if !symlink_meta(&target).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        return precommit_fail(
            root,
            claim,
            err,
            &format!(
                "archive: '{}' is not a real archive directory — refusing to remove it.",
                target.display()
            ),
        );
    }
    // Prove the tree IS an archive before deleting it; rm -rf is not the way to
    // find out what a thing is.
    if validate_tree(&target, aid).is_err() {
        return precommit_fail(
            root,
            claim,
            err,
            &format!(
                "archive: '{}' does not validate as an ae archive — refusing to delete it.\n  Inspect it and remove it by hand if that is what you want.",
                target.display()
            ),
        );
    }
    let meta_bytes = read_file(&target.join("meta")).unwrap_or_default();
    let meta_id = meta_get(&meta_bytes, "archive_id");
    if meta_id != aid {
        return precommit_fail(
            root,
            claim,
            err,
            &format!(
                "archive: '{}' records archive_id '{meta_id}' — refusing to purge an archive that is not what its path claims.",
                target.display()
            ),
        );
    }
    // A NONEMPTY owner that MATCHES, or nothing happens: an empty source_session
    // is never a wildcard.
    let meta_name = meta_get(&meta_bytes, "source_session");
    if source_session.is_empty() || meta_name.is_empty() || meta_name != source_session {
        return precommit_fail(
            root,
            claim,
            err,
            &format!(
                "archive: '{}' records source_session '{meta_name}' and this end is '{source_session}' — refusing to purge an archive that is not provably this session's.",
                target.display()
            ),
        );
    }

    // COMMIT: the atomic rename.
    let doomed = claim.join("doomed");
    if let Err(why) = fs::rename(&target, &doomed) {
        return precommit_fail(
            root,
            claim,
            err,
            &format!(
                "archive: could not remove '{}' (rename: {why}).",
                target.display()
            ),
        );
    }
    // Past the commit: the archive no longer exists at its id path.
    if let Err(why) = fsync_dir(claim).and_then(|()| fsync_dir(root)) {
        return postcommit_retained(
            err,
            claim,
            &target,
            &format!("its durability is unconfirmed (fsync: {why})"),
        );
    }
    if let Err(why) = fs::remove_dir_all(claim) {
        return postcommit_retained(
            err,
            claim,
            &target,
            &format!("the payload could not be removed (remove: {why})"),
        );
    }
    // The claim AND the bytes under it are now GONE; only the DURABILITY of
    // that removal is unconfirmed.
    if let Err(why) = fsync_dir(root) {
        return postcommit_removed_unsynced(err, &target, &format!("fsync root: {why}"));
    }

    // The archive is gone and the removal is durable.
    write!(out, "{}", target.display())?;
    Ok(0)
}

/// Remove our own claim and fsync the root.
fn clear_claim(root: &Path, claim: &Path) -> Result<(), String> {
    fs::remove_dir_all(claim).map_err(|why| {
        format!(
            "archive: could not remove the purge claim {} ({why}); remove it by hand.",
            claim.display()
        )
    })?;
    fsync_dir(root).map_err(|why| {
        format!("archive: the purge claim was removed but not durably (fsync root: {why}).")
    })?;
    Ok(())
}

/// A PRE-rename refusal: nothing was deleted.
fn precommit_fail(root: &Path, claim: &Path, err: &mut impl Write, diag: &str) -> io::Result<u8> {
    writeln!(err, "{diag}")?;
    if let Err(cleanup) = clear_claim(root, claim) {
        writeln!(err, "{cleanup}")?;
    }
    Ok(EXIT_FAILED)
}

/// A POST-rename failure with the doomed archive STILL under our claim: the
/// canonical id path was removed but the payload is not yet durably gone.
fn postcommit_retained(
    err: &mut impl Write,
    claim: &Path,
    target: &Path,
    reason: &str,
) -> io::Result<u8> {
    writeln!(
        err,
        "archive: PURGE INCOMPLETE for {} — the archive was removed from its id path but {reason}.",
        target.display()
    )?;
    writeln!(
        err,
        "  The claim {} is RETAINED and may still hold payload bytes; inspect and remove it by hand. The session is STOPPED and nothing else was deleted.",
        claim.display()
    )?;
    Ok(EXIT_FAILED)
}

/// A POST-removal failure: the archive AND our claim are already GONE, but the
/// DURABILITY of that final removal is unconfirmed (a crash here could
/// resurrect the emptied claim directory, never the archive at its id path).
fn postcommit_removed_unsynced(
    err: &mut impl Write,
    target: &Path,
    reason: &str,
) -> io::Result<u8> {
    writeln!(
        err,
        "archive: PURGE DURABILITY UNCONFIRMED for {} — the archive and the purge claim were both removed, but the removal is not confirmed on disk ({reason}).",
        target.display()
    )?;
    writeln!(
        err,
        "  No claim remains to inspect and the bytes are gone from their id path. The session is STOPPED and nothing else was deleted."
    )?;
    Ok(EXIT_FAILED)
}

#[cfg(test)]
mod tests {
    use super::{postcommit_removed_unsynced, postcommit_retained};
    use crate::state::EXIT_FAILED;
    use std::path::Path;

    // The two post-commit states report DISTINCTLY: one retains a claim to
    // inspect, the other has already removed it.
    #[test]
    fn retained_and_removed_states_are_reported_distinctly() {
        let claim = Path::new("/x/.publishing.abc");
        let target = Path::new("/x/abc");

        let mut retained = Vec::new();
        let rc_r = postcommit_retained(&mut retained, claim, target, "reason").unwrap();
        let retained = String::from_utf8(retained).unwrap();

        let mut removed = Vec::new();
        let rc_m = postcommit_removed_unsynced(&mut removed, target, "fsync root: x").unwrap();
        let removed = String::from_utf8(removed).unwrap();

        assert_eq!(rc_r, EXIT_FAILED, "retained is a failure");
        assert_eq!(rc_m, EXIT_FAILED, "removed-unsynced is a failure");
        // The retained state names a claim that is kept; the removed state must
        // NOT say the claim is retained (that would be false — it is gone).
        assert!(
            retained.contains("is RETAINED"),
            "retained names the marker"
        );
        assert!(
            removed.contains("No claim remains"),
            "removed-unsynced says there is nothing to inspect"
        );
        assert!(
            !removed.contains("RETAINED"),
            "removed-unsynced never claims a retained marker"
        );
    }
}
