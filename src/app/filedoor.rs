//! The file reader's one door onto the file system: a file a lane links,
//! read for the reader panel — and only from the loader thread.
//!
//! A path in a lane is hostile text (an agent wrote it), and the link scanner
//! links more than the reader opens: any absolute path, `..` kept as written.
//! So the door judges the CANONICAL target against the CANONICAL roots the
//! link base names, the session's work dir and the viewer's home: a path
//! outside both, by `..` or by a symlink, is refused by name (the terminal's
//! own OSC 8 open still reaches it), while a link that stays inside is
//! followed. A target must be a regular file, checked BEFORE it is opened (a
//! FIFO or a device opened blindly would block the loader for good) and again
//! on the opened handle; it is read once, bounded.
//!
//! NAMED RESIDUAL: between the canonicalisation and the gate and the open a
//! same-user process can swap the file (to a FIFO, say). The re-check on the
//! opened handle narrows that and does not close it.

use std::io::Read as _;
use std::path::{Path, PathBuf};

/// The roots a file may be read under: the session's recorded work dir and the
/// viewer's home. A root that does not canonicalise is no root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roots {
    pub dir: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

/// What a read came to: the file's bytes and its length on disk, or the named
/// reason it was refused.
#[derive(Debug)]
pub enum Read {
    Bytes { bytes: Vec<u8>, size: u64 },
    Refused(String),
}

/// `why`, as a refusal.
fn refuse(why: impl Into<String>) -> Read {
    Read::Refused(why.into())
}

/// An I/O error as the reason a read stopped: a missing file is `not found`,
/// anything else `unreadable` and its kind.
fn failed(error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        "not found".to_owned()
    } else {
        format!("unreadable: {}", error.kind())
    }
}

/// `path` read, at most `max` bytes, if it is a regular file under a root.
/// The refusals come in this order: no usable root, the target missing or
/// unreadable, outside every root, not a regular file, longer than `max`.
#[must_use]
pub fn read(path: &Path, roots: &Roots, max: usize) -> Read {
    let held: Vec<PathBuf> = [roots.dir.as_deref(), roots.home.as_deref()]
        .into_iter()
        .flatten()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .collect();
    if held.is_empty() {
        return refuse("no base to read against");
    }
    let canonical = match std::fs::canonicalize(path) {
        Ok(canonical) => canonical,
        Err(error) => return refuse(failed(&error)),
    };
    if !held.iter().any(|root| canonical.starts_with(root)) {
        return refuse("outside the session dir and home");
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the reader's regular-file gate, before the target is opened — see clippy.toml"
    )]
    let gate = std::fs::metadata(&canonical);
    match gate {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return refuse("not a regular file"),
        Err(error) => return refuse(failed(&error)),
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the reader's one open of a gated regular file, read once and bounded — see clippy.toml"
    )]
    let opened = std::fs::File::open(&canonical);
    let file = match opened {
        Ok(file) => file,
        Err(error) => return refuse(failed(&error)),
    };
    let size = match file.metadata() {
        Ok(meta) if meta.is_file() => meta.len(),
        Ok(_) => return refuse("not a regular file"),
        Err(error) => return refuse(failed(&error)),
    };
    let limit = u64::try_from(max).unwrap_or(u64::MAX);
    if size > limit {
        return refuse(format!("too large: {size} bytes (limit {max})"));
    }
    let mut bytes = Vec::new();
    if let Err(error) = file.take(limit.saturating_add(1)).read_to_end(&mut bytes) {
        return refuse(failed(&error));
    }
    // The file grew past `max` since it was measured.
    let taken = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if taken > limit {
        return refuse(format!(
            "too large: {} bytes (limit {max})",
            size.max(taken)
        ));
    }
    Read::Bytes { bytes, size }
}
