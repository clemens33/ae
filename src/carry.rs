//! Carrying ONE conversation between two accounts of the SAME tool.
//!
//! `ae reseat` was built for a TOOL change: the successor arrives on another
//! harness, which cannot read the predecessor's store, so it is handed ae's own
//! seed pack and starts fresh. When the two profiles run the same binary and
//! differ only in their config home — the case a dead vendor quota forces, one
//! login to another — that is a loss with no cause: the conversation is a set
//! of ordinary files in a directory ae already knows the path of.
//!
//! So it is COPIED, never moved. The source home is the rollback: a carry that
//! fails leaves the predecessor's account exactly as it found it, and the move
//! falls back to the seed path it always took.
//!
//! Four rules hold the whole module up:
//!
//! 1. **Bytes, never structure.** Nothing here parses a transcript, a task file
//!    or a checkpoint. The store is hostile persisted state and stays opaque,
//!    which is why this adds no parser and owes no fuzz target.
//! 2. **No link is followed.** Every node is classified with `symlink_metadata`
//!    before it is read, written or descended, the way
//!    `lifecycle/end.rs::safe_entries` classifies a store it is about to delete
//!    from.
//! 3. **Nothing in the target is overwritten.** A node that is already there is
//!    either byte-identical — ae's own interrupted attempt, so it is left
//!    exactly as it is, mtime included — or it belongs to something else, and
//!    then the whole carry refuses.
//! 4. **The copy set is BINDING.** The transcript and every sidecar the source
//!    actually has. A source that is simply not there is no failure — most
//!    conversations never wrote a checkpoint — but a read ae cannot make, a
//!    write it cannot make, and a target node it cannot explain all abandon the
//!    carry rather than delivering half a conversation.
//!
//! **THE TRANSCRIPT IS THE COMMIT MARKER.** Sidecars and project memory are
//! copied first and the transcript last, so a crash anywhere in the middle
//! leaves the target with NO conversation — nothing the tool or ae will find,
//! and nothing that makes the next attempt refuse. That next attempt re-copies,
//! finds its own earlier files byte-identical, no-ops over them, and commits.
//! Every file is published by temp-write and rename for the same reason: a
//! half-written sidecar would be neither identical nor explicable, and would
//! turn one interrupted carry into a permanent seeded fallback.
//!
//! The paths are COMPUTED, never searched for — see [`project_key`], which owns
//! that spelling and the reason the carry refuses when it cannot compute one.

use std::path::{Path, PathBuf};

use crate::tool::{CarrySpec, ToolKind};

/// How deep a sidecar tree is copied. Measured shapes need two
/// (`<uuid>/tool-results/<file>`); the rest is headroom that still bounds a
/// pathological store, because this runs under the session's lifecycle lock.
const MAX_DEPTH: usize = 4;

/// How many nodes one carry may visit. The same bound, from the other side.
const MAX_NODES: usize = 4096;

/// The store directories a conversation's uuid-keyed sidecars live in, beside
/// the project directory that holds the transcript itself. Every one of them is
/// optional: a conversation that never wrote a checkpoint has no
/// `file-history/<uuid>/`.
const SIDECAR_ROOTS: [&str; 3] = ["file-history", "session-env", "tasks"];

/// A move whose conversation can travel, with both ends resolved.
pub(crate) struct Plan {
    /// The account the conversation lives in now.
    from: PathBuf,
    /// The account it is copied into.
    to: PathBuf,
    /// The project directory name both ends use — [`project_key`] of the seat's
    /// recorded working copy.
    key: String,
    /// The conversation, grammar-proven before this was built.
    id: String,
}

impl Plan {
    /// The two accounts this crossing names, for the one line ae prints.
    pub(crate) fn homes(&self) -> (&Path, &Path) {
        (&self.from, &self.to)
    }

    /// The conversation being carried.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
}

/// What a finished carry has to say for itself.
pub(crate) struct Crossing {
    /// The target already had a project memory of its own, so it was LEFT — ae
    /// never merges two accounts' memories, and a human is told which one the
    /// successor is reading.
    pub(crate) memory_kept: bool,
}

/// One seat's move, as the caller resolved it.
pub(crate) struct Move<'a> {
    /// The binary the seat records running now.
    pub(crate) from_binary: &'a str,
    /// The binary the new profile lexes to.
    pub(crate) to_binary: &'a str,
    /// That binary's tool, for the adapter row this asks about.
    pub(crate) tool: ToolKind,
    /// The conversation the seat records.
    pub(crate) id: &'a str,
    /// The seat's recorded working copy: what the tool keys its store by, and
    /// what the pane is respawned into.
    pub(crate) work_dir: &'a Path,
    /// The store the seat's conversation lives in, from its RECORDED row —
    /// `None` when that row names no usable path.
    pub(crate) from: Option<&'a Path>,
    /// The store the new profile resolves to — `None` when it names none.
    pub(crate) to: Option<&'a Path>,
}

/// Does this move carry its conversation? PURE, and `None` is SILENT: an
/// ordinary tool change, a move inside one account, a seat with no conversation
/// and a tool whose store ae has not measured all take the path they took
/// before this module existed, and say nothing new.
pub(crate) fn plan(moving: &Move<'_>) -> Option<Plan> {
    // THE SAME BINARY, exactly. Both sides are already the lexed, path-stripped
    // harness word, so two profiles of one tool match and a tool change never
    // does. Asked on the string rather than on the tool class deliberately: the
    // claim being made is that the successor reads the predecessor's own files.
    if moving.from_binary.is_empty() || moving.from_binary != moving.to_binary {
        return None;
    }
    // Whether that tool's conversation IS a portable file set. A per-tool fact,
    // so the adapter row owns it.
    if moving.tool.adapter().carry == CarrySpec::NotPortable {
        return None;
    }
    // A conversation ae cannot name is one it cannot copy. The same grammar
    // every other id gate asks, so a `pending` slot, a hand-edited row and a
    // tagged predecessor element are all refused here.
    if !crate::session_launch::capture::is_lowercase_uuid(moving.id) {
        return None;
    }
    let (from, to) = (moving.from?, moving.to?);
    // SAME ACCOUNT: the conversation is already where the successor will look.
    // Nothing to copy, and nothing this module should claim.
    if from == to {
        return None;
    }
    Some(Plan {
        from: from.to_owned(),
        to: to.to_owned(),
        key: project_key(moving.work_dir),
        id: moving.id.to_owned(),
    })
}

/// The project directory one working copy keys: its path with every `/` as `-`.
///
/// THE ONE OWNER of that spelling for code that may move. It deliberately
/// matches AE'S OWN PROBE (`run::resumable`'s `StoreProbe::ProjectTranscript`)
/// rather than claude's internal rule, which resolves symbolic links before it
/// builds the same string: the copy exists to be found by the probe, so a
/// divergence between the two would put the file where nothing looks. When they
/// disagree — a working copy reached through a link — the source transcript is
/// simply not at this key and the carry refuses, which is honest: that seat
/// could not be exact-resumed in its OLD account either.
#[must_use]
pub fn project_key(work_dir: &Path) -> String {
    work_dir
        .display()
        .to_string()
        .chars()
        .map(|ch| if ch == '/' { '-' } else { ch })
        .collect()
}

/// COPY the conversation. `Err` is the loud fallback: the seat still moves, on
/// a fresh conversation with the seed pack, and the reason is what ae prints.
pub(crate) fn run(plan: &Plan) -> Result<Crossing, String> {
    let source = plan.from.join("projects").join(&plan.key);
    let target = plan.to.join("projects").join(&plan.key);
    let name = format!("{}.jsonl", plan.id);
    // THE CONVERSATION ITSELF, read before anything is written: a carry that
    // cannot read the thing it exists to move has nothing to do.
    let bytes = read_regular(&source.join(&name))?;
    // THE TARGET PROJECT DIRECTORY, before anything is put in it. A link here
    // would file another account's conversation somewhere neither home names.
    for directory in [plan.to.join("projects"), target.clone()] {
        if matches!(classify(&directory), Node::Link | Node::File) {
            return Err(format!("{} is not a directory", directory.display()));
        }
    }
    // ...and whether the conversation is already there, decided BEFORE the
    // sidecars are copied, so a target holding a different conversation costs
    // nothing but the reading.
    let committed = match classify(&target.join(&name)) {
        Node::Missing => false,
        // AE'S OWN INTERRUPTED ATTEMPT — or a carry that already finished. The
        // bytes are proven identical, so nothing is written and nothing is
        // touched, mtime included.
        Node::File if read_regular(&target.join(&name)).as_deref() == Ok(bytes.as_slice()) => true,
        _ => {
            return Err(format!(
                "{} already holds a different conversation file",
                target.join(&name).display()
            ));
        }
    };

    let mut budget = MAX_NODES;
    // SIDECARS FIRST. Each is optional in the source and binding once present.
    copy_tree(&source.join(&plan.id), &target.join(&plan.id), &mut budget)?;
    for root in SIDECAR_ROOTS {
        copy_tree(
            &plan.from.join(root).join(&plan.id),
            &plan.to.join(root).join(&plan.id),
            &mut budget,
        )?;
    }
    // PROJECT MEMORY is not uuid-keyed: it belongs to the working copy and is
    // shared by every conversation in that account. ae copies it only into an
    // account that has none, and NEVER merges two — the human's ruling, and the
    // only safe one, since a merge cannot be undone by hand.
    let memory_kept = classify(&target.join("memory")) != Node::Missing;
    if !memory_kept {
        copy_tree(&source.join("memory"), &target.join("memory"), &mut budget)?;
    }
    // THE COMMIT. Last, so everything above is already in place when the
    // conversation becomes findable.
    if !committed {
        make_dir(&target)?;
        publish(&target.join(&name), &bytes)?;
    }
    Ok(Crossing { memory_kept })
}

/// Copy one optional directory tree. A source that is not there is `Ok`;
/// anything present is binding.
fn copy_tree(source: &Path, target: &Path, budget: &mut usize) -> Result<(), String> {
    match classify(source) {
        Node::Missing => Ok(()),
        Node::Dir => copy_dir(source, target, MAX_DEPTH, budget),
        Node::Link | Node::File => Err(format!("{} is not a directory", source.display())),
    }
}

fn copy_dir(source: &Path, target: &Path, depth: usize, budget: &mut usize) -> Result<(), String> {
    if depth == 0 {
        return Err(format!(
            "{} is nested deeper than ae will copy",
            source.display()
        ));
    }
    make_dir(target)?;
    for entry in children(source)? {
        if *budget == 0 {
            return Err(format!(
                "{} holds more files than ae will copy in one move",
                source.display()
            ));
        }
        *budget -= 1;
        let Some(name) = entry.file_name() else {
            return Err(format!("{} has no name", entry.display()));
        };
        let into = target.join(name);
        match classify(&entry) {
            Node::Dir => copy_dir(&entry, &into, depth - 1, budget)?,
            Node::File => copy_file(&entry, &into)?,
            Node::Link => return Err(format!("{} is a symbolic link", entry.display())),
            // Raced away between the listing and the classification.
            Node::Missing => {}
        }
    }
    Ok(())
}

/// One file into a target that must not already hold a different one.
fn copy_file(source: &Path, target: &Path) -> Result<(), String> {
    let bytes = read_regular(source)?;
    match classify(target) {
        Node::Missing => publish(target, &bytes),
        Node::File if read_regular(target).as_deref() == Ok(bytes.as_slice()) => Ok(()),
        _ => Err(format!("{} already exists", target.display())),
    }
}

/// What one path IS, without following a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Node {
    Missing,
    Link,
    Dir,
    File,
}

fn classify(path: &Path) -> Node {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the carry classifies every store node without following it, so a link can never redirect a copy out of the account it names"
    )]
    let meta = std::fs::symlink_metadata(path);
    let Ok(meta) = meta else {
        return Node::Missing;
    };
    let kind = meta.file_type();
    if kind.is_symlink() {
        Node::Link
    } else if kind.is_dir() {
        Node::Dir
    } else if kind.is_file() {
        Node::File
    } else {
        // A socket or a device in a conversation store is not something to
        // copy, and reading one could block forever. Classified as a link
        // because the refusal it earns is the same one.
        Node::Link
    }
}

/// The bytes of `path`, but only when it is a REGULAR FILE — the lstat first,
/// so a link is never opened.
fn read_regular(path: &Path) -> Result<Vec<u8>, String> {
    if classify(path) != Node::File {
        return Err(format!("{} is not a readable file", path.display()));
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the carry reads the conversation's own bytes, which it copies opaquely and never parses"
    )]
    let bytes = std::fs::read(path);
    bytes.map_err(|why| format!("could not read {} ({why})", path.display()))
}

/// The children of a directory ae has already classified.
fn children(path: &Path) -> Result<Vec<PathBuf>, String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the carry enumerates one classified sidecar directory of the conversation it is copying"
    )]
    let entries = std::fs::read_dir(path);
    let entries = entries.map_err(|why| format!("could not list {} ({why})", path.display()))?;
    entries
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|why| format!("could not list {} ({why})", path.display()))
        })
        .collect()
}

/// `path` as a directory at `0700`, created with its parents if it is not
/// there. An existing DIRECTORY is success; anything else is not, because the
/// caller is about to write into it.
fn make_dir(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt as _;

    match classify(path) {
        Node::Dir => return Ok(()),
        Node::Missing => {}
        Node::Link | Node::File => return Err(format!("{} is not a directory", path.display())),
    }
    if let Some(parent) = path.parent() {
        make_dir(parent)?;
    }
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(path)
        .map_err(|why| format!("could not create {} ({why})", path.display()))
}

/// Publish `bytes` at `path`, `0600`, ATOMICALLY: a temp beside it, created
/// with `create_new` so it can never be an existing file, then one rename.
///
/// The rename is what makes a crash survivable — a reader sees the whole file
/// or none of it, never a prefix — and the caller has already proven `path` is
/// absent, under the session's lifecycle lock.
fn publish(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let temp = PathBuf::from(format!(
        "{}.ae-carry.{}",
        path.display(),
        std::process::id()
    ));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        });
    if let Err(why) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("could not write {} ({why})", temp.display()));
    }
    if let Err(why) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("could not publish {} ({why})", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::tool::ToolKind;

    const ID: &str = "22222222-2222-4222-8222-222222222222";

    fn moving<'a>(
        from_binary: &'a str,
        to_binary: &'a str,
        id: &'a str,
        from: Option<&'a Path>,
        to: Option<&'a Path>,
    ) -> super::Move<'a> {
        super::Move {
            from_binary,
            to_binary,
            tool: ToolKind::Claude,
            id,
            work_dir: Path::new("/w"),
            from,
            to,
        }
    }

    #[test]
    fn the_project_key_is_the_working_copy_with_every_separator_flattened() {
        // The spelling `run::resumable` reads. Not claude's own rule, which
        // resolves links first — the copy exists to be found by THAT probe.
        assert_eq!(
            super::project_key(Path::new("/Users/x/projects/ae")),
            "-Users-x-projects-ae"
        );
        // A relative path keeps its shape, and the empty one is not special:
        // both are answers a caller has to be able to compare, not panic on.
        assert_eq!(super::project_key(Path::new("a/b")), "a-b");
        assert_eq!(super::project_key(Path::new("")), "");
    }

    #[test]
    fn a_carry_needs_the_same_binary_a_portable_store_a_real_id_and_two_accounts() {
        let (a, b) = (Path::new("/h/a"), Path::new("/h/b"));
        let plan = super::plan(&moving("claude", "claude", ID, Some(a), Some(b)))
            .expect("two accounts of one tool carry");
        assert_eq!(plan.homes(), (a, b));
        assert_eq!(plan.id(), ID);
        assert_eq!(plan.key, "-w");

        // ANOTHER TOOL. The successor cannot read those files, so there is
        // nothing to carry and nothing to say.
        assert!(super::plan(&moving("claude", "codex", ID, Some(a), Some(b))).is_none());
        // A seat with no recorded binary is not proof of sameness.
        assert!(super::plan(&moving("", "", ID, Some(a), Some(b))).is_none());
        // THE SAME ACCOUNT: the conversation is already where the successor
        // will look.
        assert!(super::plan(&moving("claude", "claude", ID, Some(a), Some(a))).is_none());
        // An account ae cannot name, either end.
        assert!(super::plan(&moving("claude", "claude", ID, None, Some(b))).is_none());
        assert!(super::plan(&moving("claude", "claude", ID, Some(a), None)).is_none());
        // A conversation ae cannot name. `pending` is the word for one that
        // never resolved; the others are what a hand-edited row looks like.
        for id in [
            crate::launch::PENDING,
            "",
            "22222222-2222-4222-8222-22222222222",
            "22222222-2222-4222-8222-22222222222Z",
            "claude:22222222-2222-4222-8222-222222222222",
            "../../etc/passwd",
        ] {
            assert!(
                super::plan(&moving("claude", "claude", id, Some(a), Some(b))).is_none(),
                "{id} is not a conversation ae may build a path from"
            );
        }
    }

    #[test]
    fn a_tool_whose_store_ae_has_not_measured_never_carries() {
        // The adapter row is the whole rule, and every row but claude's says
        // NotPortable — a statement about EVIDENCE, not about the tools.
        for tool in [
            ToolKind::Codex,
            ToolKind::Muse,
            ToolKind::Agy,
            ToolKind::Gemini,
            ToolKind::Grok,
            ToolKind::OpenCode,
            ToolKind::Unknown,
        ] {
            let mut moving = moving(
                "same",
                "same",
                ID,
                Some(Path::new("/h/a")),
                Some(Path::new("/h/b")),
            );
            moving.tool = tool;
            assert!(super::plan(&moving).is_none(), "{tool:?}");
        }
    }
}
