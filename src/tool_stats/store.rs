//! #288 persistence (P2): publication and recovery plans for the tools
//! subtree `<root>/stats/tools/` of the S5 payload, pure over
//! [`disk::Dir`]. This module reads through a `Dir` and BUILDS plans; it
//! never opens, renames or removes a file itself: [`disk::Plan::apply`],
//! run by the caller under `stats.lock`, stays the one executor of writes
//! under `<root>/stats/`.
//!
//! One publication stages the conversation document and the view, writes
//! the intent, renames the conversation document into place (the commit
//! point: its counters and cursors never split), renames the view and drops
//! the intent. [`recovery`] finishes a committed publication or undoes one
//! that did not commit, so every crash prefix reads as the old or the new
//! generation whole.

use super::Adapter;
use super::Key;
use super::doc::{self, ASSOC_CAP, Assoc, CONV_CAP, DocError, Intent};
use super::incr::{Conv, Published};
use crate::stats_store::disk::{self, Op, Read};

/// The tools subtree.
const TOOLS: &str = "stats/tools";
/// The conversation documents' directory.
const CONVS: &str = "stats/tools/conv";
const VIEW: &str = "stats/tools/view";
const ASSOC: &str = "stats/tools/assoc";
const INTENT: &str = "stats/tools/intent";
const STAGED_CONV: &str = "stats/tools/conv.tmp";
const STAGED_VIEW: &str = "stats/tools/view.tmp";
const STAGED_ASSOC: &str = "stats/tools/assoc.tmp";

/// One document as a load found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded<T> {
    Absent,
    Ok(T),
    /// Damaged, oversized or of another format: rebuild from the sources.
    Rebuild(DocError),
    /// An IO error: never rebuilt over.
    Unreadable(std::io::ErrorKind),
}

/// Why no plan was made.
#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    /// The generation moved, or an interrupted publication awaits its
    /// recovery: the work is dropped.
    Stale,
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        out.push(char::from(b"0123456789abcdef"[usize::from(byte & 15)]));
    }
    out
}

/// The payload name of one conversation document: a digest of its adapter
/// and id, never the id itself.
#[must_use]
pub fn conv_name(adapter: Adapter, id: &str) -> String {
    let key = Key::of(
        b"tools-conv",
        [adapter.word().as_bytes(), b"\0", id.as_bytes()]
            .concat()
            .as_slice(),
    );
    format!("{CONVS}/{}", hex(&key.0))
}

fn load<T>(
    dir: &mut dyn disk::Dir,
    name: &str,
    cap: u64,
    parse: impl Fn(&[u8]) -> Result<T, DocError>,
) -> Loaded<T> {
    match dir.read(name, cap) {
        Read::Absent => Loaded::Absent,
        Read::Bytes(bytes) => parse(&bytes).map_or_else(Loaded::Rebuild, Loaded::Ok),
        Read::Oversized => Loaded::Rebuild(DocError::Oversized),
        Read::Unreadable(_) | Read::Invalid(_) => Loaded::Unreadable(std::io::ErrorKind::Other),
    }
}

pub fn load_view(dir: &mut dyn disk::Dir) -> Loaded<Published> {
    load(dir, VIEW, doc::VIEW_CAP, doc::parse_view)
}

/// The conversation document of `(adapter, id)`; one that names another
/// conversation is damage.
pub fn load_conv(dir: &mut dyn disk::Dir, adapter: Adapter, id: &str) -> Loaded<Conv> {
    match load(dir, &conv_name(adapter, id), CONV_CAP, doc::parse_conv) {
        Loaded::Ok(conv) if conv.summary.adapter != adapter || conv.summary.id != id => {
            Loaded::Rebuild(DocError::Damaged("identity"))
        }
        other => other,
    }
}

pub fn load_assoc(dir: &mut dyn disk::Dir) -> Loaded<Assoc> {
    load(dir, ASSOC, ASSOC_CAP, doc::parse_assoc)
}

/// The names under the tools subtree, `stats/` required: Tools never makes
/// the S5 payload.
fn listing(dir: &mut dyn disk::Dir) -> Result<(Vec<String>, Vec<Op>), StoreError> {
    let root = dir.list("").map_err(StoreError::Io)?;
    if !root.iter().any(|name| name == "stats") {
        return Err(StoreError::Io(std::io::Error::from(
            std::io::ErrorKind::NotFound,
        )));
    }
    let stats = dir.list("stats").map_err(StoreError::Io)?;
    let mut make = Vec::new();
    if !stats.iter().any(|name| name == "tools") {
        make.push(Op::MakeDir(TOOLS.to_owned()));
        make.push(Op::MakeDir(CONVS.to_owned()));
        return Ok((Vec::new(), make));
    }
    let names = dir.list(TOOLS).map_err(StoreError::Io)?;
    if !names.iter().any(|name| name == "conv") {
        make.push(Op::MakeDir(CONVS.to_owned()));
    }
    Ok((names, make))
}

fn leaf(name: &str) -> &str {
    name.rsplit_once('/').map_or(name, |(_, leaf)| leaf)
}

/// The plan that publishes `conv` and `view`.
///
/// # Errors
///
/// [`StoreError::Stale`] when the published view is not older than `view`
/// or an interrupted publication awaits [`recovery`]; [`StoreError::Io`]
/// when the payload cannot be listed or does not exist.
pub fn publish(
    dir: &mut dyn disk::Dir,
    conv: &Conv,
    view: &Published,
) -> Result<disk::Plan, StoreError> {
    let (names, mut ops) = listing(dir)?;
    let held = |name: &str| names.iter().any(|one| one == leaf(name));
    if held(INTENT) {
        return Err(StoreError::Stale);
    }
    match load_view(dir) {
        Loaded::Ok(current) if current.generation >= view.generation => {
            return Err(StoreError::Stale);
        }
        // Never rebuilt over: an IO error stops the publication.
        Loaded::Unreadable(kind) => return Err(StoreError::Io(kind.into())),
        Loaded::Ok(_) | Loaded::Absent | Loaded::Rebuild(_) => {}
    }
    for staged in [STAGED_CONV, STAGED_VIEW, STAGED_ASSOC] {
        if held(staged) {
            ops.push(Op::Remove(staged.to_owned()));
        }
    }
    let name = conv_name(conv.summary.adapter, &conv.summary.id);
    ops.push(Op::Write {
        name: STAGED_CONV.to_owned(),
        bytes: doc::render_conv(conv),
    });
    ops.push(Op::Write {
        name: STAGED_VIEW.to_owned(),
        bytes: doc::render_view(view),
    });
    ops.push(Op::Write {
        name: INTENT.to_owned(),
        bytes: doc::render_intent(&Intent {
            conv: name.clone(),
            generation: view.generation,
        }),
    });
    ops.push(Op::Rename {
        from: STAGED_CONV.to_owned(),
        to: name,
    });
    ops.push(Op::Rename {
        from: STAGED_VIEW.to_owned(),
        to: VIEW.to_owned(),
    });
    ops.push(Op::Remove(INTENT.to_owned()));
    Ok(disk::Plan::of(ops))
}

/// The plan that writes the association document whole: it carries no
/// counts, so a crash leaves the old or the new one.
///
/// # Errors
///
/// As [`publish`].
pub fn publish_assoc(dir: &mut dyn disk::Dir, assoc: &Assoc) -> Result<disk::Plan, StoreError> {
    let (names, mut ops) = listing(dir)?;
    if names.iter().any(|one| one == leaf(STAGED_ASSOC)) {
        ops.push(Op::Remove(STAGED_ASSOC.to_owned()));
    }
    ops.push(Op::Write {
        name: STAGED_ASSOC.to_owned(),
        bytes: doc::render_assoc(assoc),
    });
    ops.push(Op::Rename {
        from: STAGED_ASSOC.to_owned(),
        to: ASSOC.to_owned(),
    });
    Ok(disk::Plan::of(ops))
}

/// The plan that finishes or undoes an interrupted publication: with its
/// intent and the conversation document already renamed it is committed
/// and the view is renamed too; otherwise the staged files go, the staged
/// conversation document last, so a crash inside the undo is undone again
/// and never read as committed. Nothing to do is an empty plan.
///
/// # Errors
///
/// [`StoreError::Io`] when the subtree cannot be listed.
pub fn recovery(dir: &mut dyn disk::Dir) -> Result<disk::Plan, StoreError> {
    let names = dir.list(TOOLS).map_err(StoreError::Io)?;
    let held = |name: &str| names.iter().any(|one| one == leaf(name));
    let mut ops = Vec::new();
    if held(INTENT) {
        if held(STAGED_CONV) {
            // Undo: the staged view goes first, so the staged conversation
            // still marks the publication uncommitted until it goes too.
            if held(STAGED_VIEW) {
                ops.push(Op::Remove(STAGED_VIEW.to_owned()));
            }
            ops.push(Op::Remove(STAGED_CONV.to_owned()));
        } else if held(STAGED_VIEW) {
            ops.push(Op::Rename {
                from: STAGED_VIEW.to_owned(),
                to: VIEW.to_owned(),
            });
        }
        ops.push(Op::Remove(INTENT.to_owned()));
    } else {
        for staged in [STAGED_CONV, STAGED_VIEW] {
            if held(staged) {
                ops.push(Op::Remove(staged.to_owned()));
            }
        }
    }
    Ok(disk::Plan::of(ops))
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
