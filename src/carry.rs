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
//! THE THREAT MODEL, because a guard that oversells itself is the defect it
//! exists to prevent: BOTH config homes belong to the SAME OS user, and
//! someone who can write into them already owns the account. The bar this
//! module holds is therefore NO SILENT CLOBBER, NO FOLLOWED LINK and NO SILENT
//! SKIP — a target file is never replaced, a link is never traversed, and a
//! node ae cannot read is never quietly left out of a carry it then reports
//! complete. It is NOT a guarantee against a hostile writer sharing the
//! account: the residual window between a classification and the read or write
//! that follows it needs `openat`/`O_NOFOLLOW`, which has no safe-Rust
//! spelling without a libc dependency this crate forbids, so it is NAMED here
//! rather than closed.
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
//! Every file is published EXCLUSIVELY for the same reason — a temp only that
//! call opened, then a `hard_link` onto the final name, never a rename: a
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

/// The per-project directory a working copy's memory lives in. Not uuid-keyed,
/// so it is the one part of the copy set that belongs to the account rather
/// than to the conversation.
const MEMORY: &str = "memory";

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

/// Whether a walk over the copy set WRITES. ONE walk, two uses: `Check`
/// classifies, reads and compares every node exactly as `Copy` does — the
/// transcript, the sidecar trees, project memory, depth and node budget — and
/// skips only the creation in [`make_dir`] and [`publish`]. It proves the set
/// as it stands at that moment and no longer: the target account is outside
/// ae's lifecycle lock, so what appears between a `Check` and its `Copy` is met
/// by the copy's own refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Check,
    Copy,
}

/// CHECK or COPY the conversation. A `Copy` `Err` is the loud fallback: the
/// seat still moves, on a fresh conversation with the seed pack, and the
/// reason is what ae prints.
pub(crate) fn run(plan: &Plan, mode: Mode) -> Result<Crossing, String> {
    let name = format!("{}.jsonl", plan.id);
    let project: [&str; 2] = ["projects", &plan.key];
    // THE CONVERSATION ITSELF, read before anything is written: a carry that
    // cannot read the thing it exists to move has nothing to do.
    let (source_file, node) = under(&plan.from, &[project[0], project[1], &name])?;
    let bytes = read_regular(&source_file, node)?;
    // THE TARGET PROJECT DIRECTORY, before anything is put in it. `under`
    // refuses a link at every level, so this also proves the two components
    // above it.
    let (target, _) = under(&plan.to, &project)?;
    // ...and whether the conversation is already there, decided BEFORE the
    // sidecars are copied, so a target holding a different conversation costs
    // nothing but the reading.
    let (target_file, node) = under(&plan.to, &[project[0], project[1], &name])?;
    let committed = match node {
        Node::Missing => false,
        // AE'S OWN INTERRUPTED ATTEMPT — or a carry that already finished.
        Node::File if read_regular(&target_file, node)? == bytes => true,
        Node::File => {
            return Err(format!(
                "{} already holds a different conversation file",
                target_file.display()
            ));
        }
        other => return Err(format!("{} {}", target_file.display(), other.word())),
    };

    let mut budget = MAX_NODES;
    // SIDECARS FIRST. Each is optional in the source and binding once present.
    copy_tree(plan, &[project[0], project[1], &plan.id], &mut budget, mode)?;
    for root in SIDECAR_ROOTS {
        copy_tree(plan, &[root, &plan.id], &mut budget, mode)?;
    }
    // PROJECT MEMORY is not uuid-keyed: it belongs to the working copy and is
    // shared by every conversation in that account. ae copies it only into an
    // account that has none, and NEVER merges two — the human's ruling, and the
    // only safe one, since a merge cannot be undone by hand. Only a real
    // directory is that account's own memory; anything else there is a target
    // ae cannot explain, and it says which.
    let (target_memory, node) = under(&plan.to, &[project[0], project[1], MEMORY])?;
    let memory_kept = match node {
        Node::Missing => false,
        Node::Dir => true,
        other => return Err(format!("{} {}", target_memory.display(), other.word())),
    };
    if !memory_kept {
        copy_tree(plan, &[project[0], project[1], MEMORY], &mut budget, mode)?;
    }
    // THE COMMIT. Last, so everything above is already in place when the
    // conversation becomes findable.
    if committed {
        // PROVEN AGAIN, at the boundary. The reading above happened before the
        // sidecars were copied; this one is what the caller's `Ok` rests on,
        // because the target account is not under ae's lifecycle lock and the
        // successor is about to resume exactly these bytes.
        let (_, node) = under(&plan.to, &[project[0], project[1], &name])?;
        still_holds(&target_file, node, &bytes)?;
    } else if mode == Mode::Copy {
        make_dir(&target, mode)?;
        publish(&target_file, &bytes)?;
    }
    Ok(Crossing { memory_kept })
}

/// Prove the target STILL holds exactly the bytes ae carried.
///
/// The early reading in [`run`] is what lets a target holding another
/// conversation cost nothing but one read; THIS one is what an `Ok` rests on.
/// The target account is not under ae's lifecycle lock — only the session is —
/// so a conversation ae decided was already there is proven again at the
/// boundary where the successor is about to resume it.
fn still_holds(path: &Path, node: Node, bytes: &[u8]) -> Result<(), String> {
    if node != Node::File || read_regular(path, node)? != bytes {
        return Err(format!("{} changed while ae was copying", path.display()));
    }
    Ok(())
}

/// Copy one optional tree, named by its components under each account root. A
/// source that is not there is `Ok`; anything present is binding.
fn copy_tree(plan: &Plan, parts: &[&str], budget: &mut usize, mode: Mode) -> Result<(), String> {
    let (source, node) = under(&plan.from, parts)?;
    match node {
        Node::Missing => Ok(()),
        Node::Dir => {
            let (target, node) = under(&plan.to, parts)?;
            match node {
                Node::Missing | Node::Dir => copy_dir(&source, &target, MAX_DEPTH, budget, mode),
                other => Err(format!("{} {}", target.display(), other.word())),
            }
        }
        other => Err(format!("{} {}", source.display(), other.word())),
    }
}

/// Copy one CLASSIFIED source directory into a target ae has classified too.
/// Every child is classified before it is read, written or descended, so the
/// no-link walk [`under`] makes over the roots holds all the way down.
fn copy_dir(
    source: &Path,
    target: &Path,
    depth: usize,
    budget: &mut usize,
    mode: Mode,
) -> Result<(), String> {
    if depth == 0 {
        return Err(format!(
            "{} is nested deeper than ae will copy",
            source.display()
        ));
    }
    make_dir(target, mode)?;
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
        match classify(&entry)? {
            Node::Dir => copy_dir(&entry, &into, depth - 1, budget, mode)?,
            Node::File => copy_file(&entry, &into, mode)?,
            // A LISTED ENTRY THAT IS GONE is a source ae cannot promise it
            // copied, exactly like one it cannot read. The copy set is binding,
            // so this abandons the carry rather than delivering it short.
            other => return Err(format!("{} {}", entry.display(), other.word())),
        }
    }
    Ok(())
}

/// One file into a target that must not already hold a different one.
fn copy_file(source: &Path, target: &Path, mode: Mode) -> Result<(), String> {
    let bytes = read_regular(source, Node::File)?;
    match classify(target)? {
        Node::Missing if mode == Mode::Check => Ok(()),
        Node::Missing => publish(target, &bytes),
        Node::File if read_regular(target, Node::File)? == bytes => Ok(()),
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
    /// A socket, a device or a fifo: not something to copy, and reading one
    /// could block forever. It earns a link's refusal and its OWN word, so the
    /// loud reason never calls a device a symbolic link.
    NonRegular,
}

impl Node {
    /// How a refusal names this node.
    fn word(self) -> &'static str {
        match self {
            Node::Missing => "is not there",
            Node::Link => "is a symbolic link",
            Node::Dir => "is a directory",
            Node::File => "is a file",
            Node::NonRegular => "is not a regular file",
        }
    }
}

/// Classify `root` joined with `parts`, walking EVERY component from the
/// account root down and refusing a link at any level.
///
/// The leaf lstat alone is not enough: a read, a listing or a write re-opens
/// the WHOLE pathname, so an ancestor link would redirect the copy out of the
/// account the plan names. Each component is proven to be a plain file name
/// too, which is what makes the joined path unable to climb out of `root` — a
/// cheaper and more honest proof than canonicalizing, which would itself
/// follow the links this refuses. An ancestor that is simply absent makes the
/// leaf absent; anything else there is an error.
///
/// RESIDUAL, named rather than closed: between this walk and the read or write
/// that follows it, a component could be replaced. Closing that needs
/// `openat`/`O_NOFOLLOW`, which has no safe-Rust spelling without a libc
/// dependency this crate forbids. Both accounts belong to the same OS user, so
/// the bar here is no silent clobber, no followed link and no silent skip — not
/// a guarantee against someone who already owns the account.
fn under(root: &Path, parts: &[&str]) -> Result<(PathBuf, Node), String> {
    let mut full = root.to_path_buf();
    for part in parts {
        let mut components = Path::new(part).components();
        if !matches!(components.next(), Some(std::path::Component::Normal(_)))
            || components.next().is_some()
        {
            return Err(format!("'{part}' is not a plain file name"));
        }
        full.push(part);
    }
    let mut walk = root.to_path_buf();
    let mut node = classify(&walk)?;
    for part in parts {
        match node {
            Node::Dir => {}
            Node::Missing => return Ok((full, Node::Missing)),
            other => return Err(format!("{} {}", walk.display(), other.word())),
        }
        walk.push(part);
        node = classify(&walk)?;
    }
    Ok((full, node))
}

/// What `path` is, without following it. `Missing` is a PROVEN absence and
/// nothing else: a node ae cannot even stat is a node it cannot promise it
/// copied, so every other failure is an error and the carry is abandoned
/// loudly rather than reported complete without it.
fn classify(path: &Path) -> Result<Node, String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the carry classifies every store node without following it, so a link can never redirect a copy out of the account it names"
    )]
    let meta = std::fs::symlink_metadata(path);
    let meta = match meta {
        Ok(meta) => meta,
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Ok(Node::Missing),
        Err(why) => return Err(format!("could not read {} ({why})", path.display())),
    };
    let kind = meta.file_type();
    Ok(if kind.is_symlink() {
        Node::Link
    } else if kind.is_dir() {
        Node::Dir
    } else if kind.is_file() {
        Node::File
    } else {
        Node::NonRegular
    })
}

/// The bytes of `path`, which the caller has already classified as a REGULAR
/// FILE — so a link is never opened.
fn read_regular(path: &Path, node: Node) -> Result<Vec<u8>, String> {
    if node != Node::File {
        return Err(format!("{} {}", path.display(), node.word()));
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
/// caller is about to write into it. A [`Mode::Check`] classifies and creates
/// nothing.
fn make_dir(path: &Path, mode: Mode) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt as _;

    match classify(path)? {
        Node::Dir => return Ok(()),
        Node::Missing if mode == Mode::Check => return Ok(()),
        Node::Missing => {}
        other => return Err(format!("{} {}", path.display(), other.word())),
    }
    if let Some(parent) = path.parent() {
        make_dir(parent, mode)?;
    }
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(path)
        .map_err(|why| format!("could not create {} ({why})", path.display()))
}

/// Publish `bytes` at `path`, `0600`, WITHOUT EVER REPLACING ANYTHING.
///
/// [`crate::init::create_exclusive`] IS that operation and ae already owns it:
/// a temp under a nonce THIS call opened — never a name a crashed attempt or
/// another user could be holding, and so never one whose cleanup would delete
/// someone else's bytes — then a `hard_link` onto the final name, which fails
/// `AlreadyExists` in the same syscall that would have created it.
///
/// NOT a rename. `rename` REPLACES a destination that appeared between the
/// caller's classification and this write, and the target account is outside
/// ae's lifecycle lock — only the session is under it. The temp sits beside the
/// final name, so the same-volume requirement holds by construction.
fn publish(path: &Path, bytes: &[u8]) -> Result<(), String> {
    crate::init::create_exclusive(path, bytes, 0o600)
        .map_err(|why| format!("could not publish {} ({why})", path.display()))
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

    /// A scratch directory of this test's own, under the system temp dir.
    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("aecarry.{}.{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a scratch dir");
        dir
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the fixture reads its own scratch dir; the boundary is about what PRODUCT code may reach, and tests/it/phase3.rs cuts at the test module"
    )]
    fn a_publication_never_replaces_a_file_that_is_already_there() {
        // NOT a rename: the target account is outside ae's lifecycle lock, so
        // the destination can appear between the caller's classification and
        // this write, and a rename would silently take another conversation's
        // place.
        let dir = scratch("clobber");
        let path = dir.join("held.jsonl");
        assert!(std::fs::write(&path, b"someone else's\n").is_ok());

        let refused = super::publish(&path, b"ae's own\n");

        assert!(refused.is_err(), "an existing name is never replaced");
        assert_eq!(
            std::fs::read(&path).ok(),
            Some(b"someone else's\n".to_vec()),
            "and the bytes that were there are still there"
        );
        // The temp the attempt wrote is not left in the account either.
        let left: Vec<_> = std::fs::read_dir(&dir)
            .map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).collect())
            .unwrap_or_default();
        assert_eq!(left, vec![path], "no scratch file is left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn put(path: &Path, bytes: &[u8]) {
        assert!(
            path.parent()
                .is_some_and(|p| std::fs::create_dir_all(p).is_ok())
        );
        assert!(std::fs::write(path, bytes).is_ok(), "{}", path.display());
    }

    /// A whole source set in `a`, an empty account `b`, and the plan between.
    fn planted(tag: &str) -> super::Plan {
        let dir = scratch(tag);
        let (from, to) = (dir.join("a"), dir.join("b"));
        let project = from.join("projects").join("-w");
        put(&project.join(format!("{ID}.jsonl")), b"transcript");
        put(&project.join(ID).join("tool-results/t1.txt"), b"tool");
        put(&project.join("memory/notes.md"), b"remembered");
        put(&from.join("file-history").join(ID).join("h@v1"), b"check");
        put(&from.join("tasks").join(ID).join("1.json"), b"{}");
        assert!(std::fs::create_dir_all(&to).is_ok());
        let (key, id) = ("-w".to_owned(), ID.to_owned());
        super::Plan { from, to, key, id }
    }

    /// Every node under `root`, sorted — what a Check must leave as it was.
    #[allow(
        clippy::disallowed_methods,
        reason = "the fixture lists its own scratch dir; the boundary is about what PRODUCT code may reach"
    )]
    fn listing(root: &Path) -> Vec<std::path::PathBuf> {
        let (mut found, mut stack) = (Vec::new(), vec![root.to_path_buf()]);
        while let Some(dir) = stack.pop() {
            for path in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = path.path();
                if path.is_dir() && !path.is_symlink() {
                    stack.push(path.clone());
                }
                found.push(path);
            }
        }
        found.sort();
        found
    }

    #[test]
    fn a_check_proves_a_clean_set_and_creates_nothing() {
        let plan = planted("check");
        assert!(super::run(&plan, super::Mode::Check).is_ok(), "a clean set");
        assert_eq!(listing(&plan.to), Vec::<std::path::PathBuf>::new());
        let _ = std::fs::remove_dir_all(plan.from.parent().unwrap_or(&plan.from));
    }

    #[test]
    fn every_failure_a_check_meets_leaves_the_target_as_it_found_it() {
        use std::os::unix::fs::PermissionsExt as _;
        // Run as root, `closed` stops being a refusal and this fails loudly.
        type Broken = fn(&super::Plan);
        let cases: [(&str, Broken); 5] = [
            ("link", |plan| {
                let leaf = plan.from.join("file-history").join(ID).join("h@v1");
                assert!(std::fs::remove_file(&leaf).is_ok());
                assert!(std::os::unix::fs::symlink("/etc/hosts", &leaf).is_ok());
            }),
            ("clash", |plan| {
                put(
                    &plan.to.join("file-history").join(ID).join("h@v1"),
                    b"theirs",
                );
            }),
            ("closed", |plan| {
                let file = plan.from.join("tasks").join(ID).join("1.json");
                let closed = std::fs::Permissions::from_mode(0o000);
                assert!(std::fs::set_permissions(file, closed).is_ok());
            }),
            ("deep", |plan| {
                put(
                    &plan.from.join("file-history").join(ID).join("a/b/c/d/x"),
                    b"deep",
                );
            }),
            ("memory", |plan| {
                put(&plan.to.join("projects/-w/memory"), b"not a directory");
            }),
        ];
        for (tag, broken) in cases {
            let plan = planted(tag);
            broken(&plan);
            let before = listing(&plan.to);
            let checked = super::run(&plan, super::Mode::Check).err();
            assert!(
                checked.is_some(),
                "{tag}: a Check refuses what a Copy would"
            );
            assert_eq!(
                listing(&plan.to),
                before,
                "{tag}: {checked:?} created something"
            );
            let _ = std::fs::remove_dir_all(plan.from.parent().unwrap_or(&plan.from));
        }
    }

    #[test]
    fn a_check_meets_the_node_budget_a_copy_would() {
        let dir = scratch("budget");
        put(&dir.join("src/one"), b"1");
        put(&dir.join("src/two"), b"2");
        let (source, target, mut budget) = (dir.join("src"), dir.join("dst"), 1);
        let checked = super::copy_dir(&source, &target, 4, &mut budget, super::Mode::Check);
        assert!(checked.is_err_and(|why| why.contains("more files than ae will copy")));
        assert_eq!(
            super::classify(&target),
            Ok(super::Node::Missing),
            "creates nothing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_conversation_that_changed_under_a_finished_carry_is_not_reported_carried() {
        let dir = scratch("changed");
        let path = dir.join("held.jsonl");
        assert!(std::fs::write(&path, b"first\n").is_ok());

        assert!(
            super::still_holds(&path, super::Node::File, b"first\n").is_ok(),
            "the bytes ae copied are the bytes that are there"
        );
        let changed = super::still_holds(&path, super::Node::File, b"second\n");
        assert!(
            changed.is_err_and(|why| why.contains("changed while ae was copying")),
            "a target that moved under the carry is a loud fallback, not an Ok"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the fixture reads its own scratch dir; the boundary is about what PRODUCT code may reach, and tests/it/phase3.rs cuts at the test module"
    )]
    fn a_publication_never_removes_a_temp_it_did_not_open() {
        // A temp named after the process ALONE is a name another attempt — or,
        // after a crash and a pid reuse, another program — can already hold,
        // and cleaning up a name like that deletes bytes ae never wrote.
        let dir = scratch("nonce");
        let path = dir.join("held.jsonl");
        let squatter = dir.join(format!("held.jsonl.ae-carry.{}", std::process::id()));
        assert!(std::fs::write(&squatter, b"not ae's\n").is_ok());

        let published = super::publish(&path, b"ae's own\n");

        assert!(
            published.is_ok(),
            "a name ae does not own cannot decide whether it can publish: {published:?}"
        );
        assert_eq!(
            std::fs::read(&squatter).ok(),
            Some(b"not ae's\n".to_vec()),
            "and it is never the file ae cleans up"
        );
        assert_eq!(std::fs::read(&path).ok(), Some(b"ae's own\n".to_vec()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
