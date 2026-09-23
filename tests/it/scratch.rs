//! Where the suite's scratch roots live, and the one rule that says a path is
//! one of them. `cli::OwnedScratch::root` allocates them; the parity reaper and
//! the `justfile` lane sweep (`_tmux-isolated`, `owned_root`) delete only what
//! [`owned_root`] accepts, and the two spellings must agree.

#![allow(
    clippy::disallowed_methods,
    reason = "test scratch lives on the real filesystem; the boundary is about PRODUCT code"
)]

use std::path::{Path, PathBuf};

/// The longest socket path tmux can bind: `sun_path` is 104 bytes on macOS,
/// NUL included.
pub(crate) const SOCKET_PATH_MAX: usize = 103;

/// `AE_TEST_TMPDIR`, else `/tmp` — never `$TMPDIR`, which macOS spells 49 bytes
/// long: a `-L` socket two levels beneath it outgrows [`SOCKET_PATH_MAX`].
pub(crate) fn base() -> PathBuf {
    std::env::var_os("AE_TEST_TMPDIR")
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from)
}

/// The directory a reaper may delete for `target`, which `owner` registered:
/// `Some(<base>/ae-it-<owner>)` only when `target` is exactly one name beneath
/// it, the parent a real directory and the target one too or gone (its owner
/// removed it). Any other registered path is some process's cwd, and its bytes
/// are not the reaper's to take.
pub(crate) fn owned_root(owner: u32, target: &Path) -> Option<PathBuf> {
    let parent = target.parent()?;
    target.file_name()?;
    if parent.file_name()? != format!("ae-it-{owner}").as_str() {
        return None;
    }
    let at = parent.parent()?;
    let base = base();
    if at != base && std::fs::canonicalize(&base).ok().as_deref() != Some(at) {
        return None;
    }
    let plain = |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir());
    let gone = std::fs::symlink_metadata(target)
        .is_err_and(|why| why.kind() == std::io::ErrorKind::NotFound);
    (plain(parent) && (gone || plain(target))).then(|| parent.to_owned())
}
