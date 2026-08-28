//! The shared archive store: the ONE implementation of the archive root
//! classifier, the `.publishing.<uuid>` claim primitive, the tree validator, and
//! the capability doors — used by [`super::publish`], [`super::from`] and
//! [`super::purge`] so the trust rules are proved in one place, never cloned.
//!
//! Reads of the world go through the doors here, each named at its site —
//! `archive.rs` is an inventoried reader (clippy.toml's disallowed-methods
//! boundary). Writes, renames and fsyncs are not reads and need no door.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use super::meta_get;

/// The archive root for a session directory — `<AE_HOME>/archive`, from the
/// session path `<AE_HOME>/sessions/<name>` — so no `env::var` door is needed.
/// `None` only for a path with no grandparent, which a real session never is.
pub(super) fn archive_root_of(session_dir: &Path) -> Option<PathBuf> {
    session_dir
        .parent()
        .and_then(Path::parent)
        .map(|home| home.join("archive"))
}

/// Whether the archive root is present as a real directory, or simply not there.
/// A symlinked root is never either — it is a hard refusal ([`require_real_root`]
/// returns `Err`), because a link is how a pointer would reach outside the
/// archive ae owns.
pub(super) enum RootState {
    /// A real directory (created when `create`).
    Present,
    /// Not a directory, and `create` was false — the caller decides whether that
    /// is a refusal (inherit) or nothing-to-do (purge).
    Absent,
}

/// Classify — and, with `create`, materialise — the archive root at `root`.
///
/// A symlinked root is always a hard refusal. With `create` (publish): make it
/// `0700` and prove its own directory entry durable by an unconditional parent
/// fsync, then `Present`. Without `create` (inherit/purge): `Present` only for an
/// existing real directory, else `Absent` — never created, never fsynced.
pub(super) fn require_real_root(root: &Path, create: bool) -> Result<RootState, String> {
    if let Ok(meta) = symlink_meta(root)
        && meta.file_type().is_symlink()
    {
        return Err(format!(
            "archive: '{}' is a symlink, not ae's archive root — refusing to act through it.",
            root.display()
        ));
    }
    if !create {
        return match symlink_meta(root) {
            Ok(meta) if meta.is_dir() => Ok(RootState::Present),
            _ => Ok(RootState::Absent),
        };
    }
    if !exists(root) {
        fs::create_dir_all(root)
            .map_err(|why| format!("archive: could not create '{}': {why}", root.display()))?;
    }
    let _ = fs::set_permissions(root, fs::Permissions::from_mode(0o700));
    // Persist the archive/ directory's OWN entry in its PARENT — unconditionally,
    // before any claim. The later root fsync persists entries INSIDE archive/, not
    // archive/'s own entry, and `ae end` deletes the live session once publication
    // reports success; so an unconfirmed root must fail the publish here rather
    // than after the source is gone.
    if let Some(parent) = root.parent() {
        fsync_dir(parent).map_err(|why| {
            format!(
                "archive: could not confirm '{}' is durable (fsync parent: {why}); refusing before publishing so nothing is deleted.",
                root.display()
            )
        })?;
    }
    Ok(RootState::Present)
}

/// The `.publishing.<aid>` claim path under `root` — the mutual-exclusion token
/// publication and purge share.
pub(super) fn claim_path(root: &Path, aid: &str) -> PathBuf {
    root.join(format!(".publishing.{aid}"))
}

/// Atomically create the claim directory at exactly `0700`. `Ok(())` means this
/// process now owns the id; an `AlreadyExists` error means another publisher or
/// purger holds it (or a crash left it standing) — the caller refuses and NEVER
/// guess-cleans a claim it did not create.
///
/// RESIDUAL (documented, ruled out of this delivery): the claim is a mutual-
/// exclusion token by PATHNAME. ae's threat model is cooperating/confused
/// same-UID agents, all of which respect this mkdir claim. A DELIBERATELY hostile
/// same-UID process could, after we win the mkdir, unlink our claim directory and
/// substitute a symlink at `.publishing.<aid>`, so that a later `fs::rename` or
/// removal keyed on that path resolves the middle component through the link and
/// acts outside the archive root. Closing that honestly needs descriptor-relative
/// `renameat`/`unlinkat`/`openat` machinery absent from std (or a `rustix`
/// dependency); a pathname re-check would only move the same race, not close it,
/// so ae adds neither now. REVISIT this and the purge/publish rename boundaries if
/// ae ever treats peer processes as hostile, or adopts rustix/openat capabilities.
pub(super) fn make_claim(claim: &Path) -> io::Result<()> {
    fs::DirBuilder::new().mode(0o700).create(claim)
}

/// A direct, non-dotted child: `cand`'s parent is exactly `root` and its final
/// component is a real name. Defence in depth — the id is a validated UUID, so
/// the join is a direct child by construction, but a link or a crafted path never
/// reaches outside the root ae owns.
pub(super) fn is_direct_child(root: &Path, cand: &Path) -> bool {
    cand.parent() == Some(root)
        && cand
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|b| !b.is_empty() && b != "." && b != "..")
}

/// Validate that the tree at `path` IS an ae archive for `aid`: a real `0700`
/// directory; only the known entries, each a non-symlink regular file at `0600`
/// with no executable bit; the required files present; `messages/` a `0700`
/// directory of `0600` `.txt` files; and meta and digest agreeing on the
/// archive id, the version, a named source session, and the three counts a
/// human-edited pair would disagree on. The one gate publish, inherit and purge
/// all pass before they trust a tree.
/// Reject any DIRECT child of `path` outside the exact archive set — the
/// whole-tree rejection the frozen `_ar_validate_tree` made. A destructive
/// consumer (purge) deletes this tree recursively and `--from` trusts it as
/// lineage, so an unrecognised top-level file or directory is unvalidated bytes
/// we must never bless. Enumeration failure REFUSES; it never defaults to an
/// empty (vacuously-passing) listing.
fn reject_unknown_top_level(path: &Path) -> Result<(), String> {
    let allowed = ["meta", "digest.md", "memo.tsv", "events.jsonl", "messages"];
    let top = read_dir(path).map_err(|why| {
        format!(
            "archive: cannot enumerate '{}' ({why}) — refusing an incompletely-read tree.",
            path.display()
        )
    })?;
    for entry in top {
        let name = entry
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !allowed.contains(&name.as_str()) {
            return Err(format!(
                "archive: unexpected top-level entry '{name}' — an ae archive holds only {allowed:?}."
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_tree(path: &Path, aid: &str) -> Result<(), String> {
    let dir_mode =
        symlink_meta(path).map_err(|why| format!("archive: staged tree unreadable: {why}"))?;
    if dir_mode.file_type().is_symlink() || !dir_mode.is_dir() {
        return Err(format!(
            "archive: '{}' is not a real directory.",
            path.display()
        ));
    }
    check_mode(&dir_mode, 0o700)
        .map_err(|m| format!("archive: payload has mode {m}, expected 700."))?;

    reject_unknown_top_level(path)?;

    for name in ["meta", "digest.md", "memo.tsv", "events.jsonl"] {
        let meta =
            symlink_meta(&path.join(name)).map_err(|_| format!("archive: '{name}' is missing."))?;
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(format!("archive: '{name}' is not a regular file."));
        }
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o111 != 0 {
            return Err(format!(
                "archive: '{name}' has mode {mode:o} — an archived file must carry no executable bit."
            ));
        }
        if mode != 0o600 {
            return Err(format!(
                "archive: '{name}' has mode {mode:o}, expected 600."
            ));
        }
    }
    let msgs = path.join("messages");
    let msgs_meta =
        symlink_meta(&msgs).map_err(|_| "archive: 'messages/' is missing.".to_owned())?;
    if msgs_meta.file_type().is_symlink() || !msgs_meta.is_dir() {
        return Err("archive: 'messages/' must be a directory.".to_owned());
    }
    check_mode(&msgs_meta, 0o700)
        .map_err(|m| format!("archive: 'messages/' has mode {m}, expected 700."))?;
    // Propagate an enumeration failure as a validation failure — never treat an
    // unreadable or partially-read messages/ as empty (which would bless a tree
    // whose bodies were never examined, right before purge removes them).
    let msg_entries = read_dir(&msgs).map_err(|why| {
        format!(
            "archive: cannot enumerate 'messages/' ({why}) — refusing an incompletely-read tree."
        )
    })?;
    for entry in msg_entries {
        let base = entry
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let meta = symlink_meta(&entry)
            .map_err(|why| format!("archive: 'messages/{base}' unreadable: {why}"))?;
        if base.rsplit('.').next() != Some("txt") {
            return Err(format!("archive: unexpected entry 'messages/{base}'."));
        }
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(format!("archive: 'messages/{base}' is not a regular file."));
        }
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o111 != 0 || mode != 0o600 {
            return Err(format!(
                "archive: 'messages/{base}' has mode {mode:o}, expected 600."
            ));
        }
    }

    // Meta/digest consistency: the same checks the frozen validator makes, cheap
    // insurance against a future divergence in the two renderers.
    let meta_bytes = read_file(&path.join("meta"))
        .map_err(|why| format!("archive: staged meta unreadable: {why}"))?;
    let digest = read_file(&path.join("digest.md"))
        .map_err(|why| format!("archive: staged digest unreadable: {why}"))?;
    let digest = String::from_utf8_lossy(&digest);
    if meta_get(&meta_bytes, "archive_id") != aid {
        return Err(format!("archive: meta archive_id does not match '{aid}'."));
    }
    if meta_get(&meta_bytes, "archive_version") != "1" {
        return Err("archive: archive_version is not 1.".to_owned());
    }
    if meta_get(&meta_bytes, "source_session").is_empty() {
        return Err("archive: meta names no source_session.".to_owned());
    }
    for (key, section) in [
        ("handover_count", "## Handover ("),
        ("memo_topic_count", "## Memo topics ("),
        ("pending_request_count", "## Unresolved requests ("),
    ] {
        let want = meta_get(&meta_bytes, key);
        let have = digest
            .split_once(section)
            .and_then(|(_, rest)| rest.split_once(')'))
            .map(|(n, _)| n.to_owned())
            .unwrap_or_default();
        if want != have {
            return Err(format!(
                "archive: digest says {section}{have}) but meta says {want}."
            ));
        }
    }
    if !digest.contains(&format!("- Archive ID: {aid}")) {
        return Err(format!("archive: digest does not name archive ID {aid}."));
    }
    Ok(())
}

/// The `(files, bytes)` totals of a published tree — the diagnostic a bash
/// consumer reads back after a publish.
pub(super) fn count_tree(target: &Path) -> (u64, u64) {
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut stack = vec![target.to_owned()];
    while let Some(path) = stack.pop() {
        let Ok(meta) = symlink_meta(&path) else {
            continue;
        };
        if meta.is_dir() {
            if let Ok(entries) = read_dir(&path) {
                stack.extend(entries);
            }
        } else if meta.is_file() {
            files += 1;
            bytes += meta.len();
        }
    }
    (files, bytes)
}

// ── capability doors ────────────────────────────────────────────────────────

/// `fs::read`, the one place the archive modules read a file's bytes.
pub(super) fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: reads a source, staged or archived file — see clippy.toml"
    )]
    let bytes = std::fs::read(path);
    bytes
}

/// The entry paths directly under `dir` (no recursion here; the caller classifies
/// each). FALLIBLE: `Err` if `dir` cannot be opened, AND if any single directory
/// entry cannot be read — both propagate rather than yielding a short or empty
/// list, so a destructive/validation caller refuses an incompletely-read
/// directory instead of blessing it. Never reintroduce an empty fallback here.
pub(super) fn read_dir(dir: &Path) -> io::Result<Vec<PathBuf>> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: lists messages/ or a tree for staging/validation — see clippy.toml"
    )]
    let read = std::fs::read_dir(dir)?;
    // Propagate EVERY DirEntry error — a destructive/validation consumer must
    // refuse on incomplete enumeration, never silently drop entries. NO `flatten`.
    read.map(|entry| entry.map(|e| e.path())).collect()
}

/// `symlink_metadata` (lstat) — classifies a node without following it.
pub(super) fn symlink_meta(path: &Path) -> io::Result<fs::Metadata> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: lstat to classify a node and read its mode — see clippy.toml"
    )]
    let meta = std::fs::symlink_metadata(path);
    meta
}

/// Whether `path` exists (an lstat that answers, symlink included) — the door
/// standing in for the disallowed `Path::exists`.
pub(super) fn exists(path: &Path) -> bool {
    symlink_meta(path).is_ok()
}

/// `File::open` of a directory, to `fsync` it — how a rename or a new entry is
/// made durable on Unix.
pub(super) fn fsync_dir(dir: &Path) -> io::Result<()> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: opens a directory to fsync it for durable publication — see clippy.toml"
    )]
    let handle = std::fs::File::open(dir)?;
    handle.sync_all()
}

/// A directory created at exactly `0700`, whatever the umask.
pub(super) fn mkdir_0700(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new().mode(0o700).create(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

/// A file written at exactly `0600` and `fsync`ed — whatever the umask, the mode
/// the validator demands and the durability the assignment requires.
pub(super) fn write_file_0600(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    file.sync_all()
}

/// `Ok(())` if `meta`'s mode is exactly `want`, else `Err(mode)` for the message.
pub(super) fn check_mode(meta: &fs::Metadata, want: u32) -> Result<(), String> {
    let mode = meta.permissions().mode() & 0o777;
    if mode == want {
        Ok(())
    } else {
        Err(format!("{mode:o}"))
    }
}

/// A stable fingerprint of a directory — sorted `name:size:mtime` for every entry
/// — so a change during a copy is caught rather than mixed.
pub(super) fn messages_fingerprint(dir: &Path) -> String {
    let Ok(mut entries) = read_dir(dir) else {
        return String::new();
    };
    entries.sort();
    let mut out = String::new();
    for path in entries {
        let base = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Ok(meta) = symlink_meta(&path) {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            let _ = write!(out, "{base}:{}:{mtime};", meta.len());
        }
    }
    out
}
