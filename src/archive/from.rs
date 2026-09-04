//! `_archive-from-preflight <archive-root> <raw-uuid>` — the read-only `--from`
//! preflight (P3.4).
//!
//! Bash routes the frozen `_ar_from_preflight` through this before any launch side
//! effect: it proves an archive is a real, validated, not-mid-flight ae archive
//! and hands back only the two counts Bash already consumes — `aid\thandover\t
//! pending` — as ONE observation, frozen against the id they were proved with. A
//! caller re-reading the counts afterwards would be a second observation of a
//! file another process may be deleting. `AE_CORE=` falls back to the frozen body.
//!
//! It reuses the one shared archive [`store`](super::store): the same real-root
//! classifier, the same `.publishing.<aid>` claim path, and the same
//! [`validate_tree`](super::store::validate_tree) that gates publication — trust
//! is proved in one place, never cloned.

use std::io::{self, Write};
use std::path::Path;

use super::store::{
    RootState, claim_path, exists, is_direct_child, read_file, require_real_root, symlink_meta,
    validate_tree,
};
use super::{canonical_uuid, meta_get};
use crate::state::EXIT_FAILED;

/// `_archive-from-preflight` core entry.
pub(crate) fn run(
    root: &Path,
    raw_uuid: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let aid = canonical_uuid(raw_uuid);
    if aid.is_empty() {
        writeln!(
            err,
            "Error: --from takes an archive UUID; '{raw_uuid}' is not one."
        )?;
        return Ok(EXIT_FAILED);
    }
    match require_real_root(root, false) {
        Ok(RootState::Present) => {}
        Ok(RootState::Absent) => {
            writeln!(err, "Error: no archive root at {}.", root.display())?;
            return Ok(EXIT_FAILED);
        }
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(EXIT_FAILED);
        }
    }

    let path = root.join(&aid);
    // A live claim means this exact id is being published or purged right now.
    let claim = claim_path(root, &aid);
    if exists(&claim) {
        writeln!(
            err,
            "Error: archive {aid} is being published or purged right now ({}).",
            claim.display()
        )?;
        return Ok(EXIT_FAILED);
    }

    // A real directory, never a link — which is how a lineage pointer would
    // otherwise reach outside the archive — and a direct child of the root.
    let is_real_dir = symlink_meta(&path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink());
    if !is_real_dir {
        writeln!(err, "Error: no archive {aid} in {}.", root.display())?;
        return Ok(EXIT_FAILED);
    }
    if !is_direct_child(root, &path) {
        writeln!(
            err,
            "Error: {} is not a direct child of {}.",
            path.display(),
            root.display()
        )?;
        return Ok(EXIT_FAILED);
    }

    // meta and digest must be present, regular, non-symlink and readable.
    for f in ["meta", "digest.md"] {
        let fp = path.join(f);
        let regular = symlink_meta(&fp).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink());
        if !regular || read_file(&fp).is_err() {
            writeln!(err, "Error: archive {aid} has no readable {f}.")?;
            return Ok(EXIT_FAILED);
        }
    }

    if validate_tree(&path, &aid).is_err() {
        writeln!(
            err,
            "Error: archive {aid} did not validate — refusing to inherit from it."
        )?;
        return Ok(EXIT_FAILED);
    }

    // The counts leave with the id they were proved against — read once, here.
    let meta_bytes = read_file(&path.join("meta")).unwrap_or_default();
    let handover = meta_get(&meta_bytes, "handover_count");
    let pending = meta_get(&meta_bytes, "pending_request_count");
    if !is_count(&handover) || !is_count(&pending) {
        writeln!(
            err,
            "Error: archive {aid} has incomplete counts (handover='{handover}', pending='{pending}')."
        )?;
        return Ok(EXIT_FAILED);
    }

    // ONE observation, frozen.
    write!(out, "{aid}\t{handover}\t{pending}")?;
    Ok(0)
}

/// A non-empty run of ASCII digits — the shape a count must have to be trusted.
fn is_count(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}
