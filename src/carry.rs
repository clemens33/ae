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
//!    byte-identical — ae's own interrupted attempt, so it is left exactly as
//!    it is, mtime included — or it is ARCHIVED, or the whole carry refuses.
//!    Archived means RENAMED ASIDE, inside the same account, into a directory
//!    this move created for itself: never deleted, never written over. Only a
//!    regular file the Check could explain is archived — a transcript whose
//!    bytes are a STRICT prefix of the source (the older copy an earlier round
//!    trip left behind) or a sidecar holding other bytes. A transcript that
//!    forked or runs longer, a link, a directory where a file belongs and a
//!    node ae cannot read all refuse.
//! 4. **The copy set is BINDING.** The transcript and every sidecar the source
//!    actually has. A source that is simply not there is no failure — most
//!    conversations never wrote a checkpoint — but a read ae cannot make, a
//!    write it cannot make, and a target node it cannot explain all abandon the
//!    carry rather than delivering half a conversation.
//!
//! **THE TRANSCRIPT IS THE COMMIT MARKER.** Sidecars and project memory are
//! copied first and the transcript last, so a crash anywhere in the middle
//! never leaves the NEW conversation half there. What it does leave depends on
//! what the target held: a transcript it lacked stays absent, and one archived
//! right before its publication leaves its name empty — in both, nothing the
//! tool or ae will find; an older copy a round trip left behind is still there
//! under its name with its OLD bytes, beside sidecars that may already be
//! archived or refreshed, until its own archive moves it; an identical one stays
//! exactly as it was. That is not a transaction. It is what makes the next
//! attempt CONVERGE: it re-copies, finds its own earlier files byte-identical,
//! no-ops over them, archives what is still stale, and commits.
//! Every file is published EXCLUSIVELY for the same reason — a temp only that
//! call opened, then a `hard_link` onto the final name, never a rename: a
//! half-written sidecar would be neither identical nor explicable, and would
//! turn one interrupted carry into a permanent seeded fallback.
//!
//! **THE CHECK DECIDES WHAT IS ARCHIVED; THE COPY ONLY RE-PROVES IT.** The
//! pre-stop [`Mode::Check`] fingerprints every node it would archive — identity
//! and a content digest, fixed size per node — and the [`Mode::Copy`] after the
//! stop archives EXACTLY that set: each node proven unchanged before the first
//! write and again immediately before it is moved. A frozen node that is gone,
//! changed or became equal, and a difference the Check never saw, abandon the
//! copy to the seed path instead, because the target account is outside ae's
//! lifecycle lock and the Check is what the human's move was decided on.
//!
//! The paths are COMPUTED, never searched for — see [`project_key`], which owns
//! that spelling and the reason the carry refuses when it cannot compute one.

use std::ffi::OsStr;
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

/// Where, inside the TARGET account, a move archives the files it replaces:
/// `<account>/ae-carry-archive/<conversation>/<YYYYMMDDTHHMMSSZ>-<pid>.<n>/`,
/// each file under its own account-relative path. The same account, so the move
/// is a rename and the bytes never travel; one fresh directory per move, so no
/// archive is ever written into twice. The stamp is the UTC second with no `-`
/// or `:`, so the leaf copied as a bare name cannot look like scp's `host:path`.
const ARCHIVE: &str = "ae-carry-archive";

/// How many names one move tries for its fresh archive directory. Each try is
/// an exclusive creation, so a collision costs one syscall; the bound only
/// stops a directory something else keeps filling.
const ARCHIVE_TRIES: u32 = 64;

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
#[derive(Debug, Default)]
pub(crate) struct Crossing {
    /// The target already had a project memory of its own, so it was LEFT — ae
    /// never merges two accounts' memories, and a human is told which one the
    /// successor is reading.
    pub(crate) memory_kept: bool,
    /// Out of a [`Mode::Check`]: what the [`Mode::Copy`] after it may archive.
    proof: Proof,
    /// Out of a [`Mode::Copy`]: how many target files it archived, and the one
    /// directory it moved them into. `None` when it archived nothing.
    archived: Option<(usize, PathBuf)>,
}

impl Crossing {
    /// The decisions a Check made, which the Copy acts on and never adds to.
    pub(crate) fn proof(&self) -> &Proof {
        &self.proof
    }

    /// How many target files the copy moved aside, and where. `None` means
    /// none were, and the move says nothing it did not say before archiving
    /// existed.
    pub(crate) fn archived(&self) -> Option<(usize, &Path)> {
        self.archived
            .as_ref()
            .map(|(count, dir)| (*count, dir.as_path()))
    }
}

/// Every archive decision one Check made: the ONLY ones a Copy may act on.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Proof {
    /// The target transcript, a strict byte prefix of the source when checked.
    stale: Option<Print>,
    /// Every target sidecar file that held other bytes than the source's.
    conflicts: Vec<Print>,
}

impl Proof {
    fn is_empty(&self) -> bool {
        self.stale.is_none() && self.conflicts.is_empty()
    }
}

/// One target file as the Check saw it, FIXED IN SIZE whatever the file's: a
/// proof that must survive the stop may not hold the bytes of every file it
/// covers, so it holds who the file was and a digest of what it held.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Print {
    /// Where it is, relative to the target account.
    relative: PathBuf,
    /// Which node it was.
    stamp: Stamp,
    /// What it held: SHA-256 of its bytes, through the one digest ae has.
    digest: String,
}

/// The identity half of a [`Print`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    dev: u64,
    ino: u64,
    len: u64,
    modified: Option<std::time::SystemTime>,
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

/// The project directory one working copy keys, by claude's own rule: every
/// character that is not an ASCII letter or digit becomes `-`, one dash per
/// UTF-16 code unit (claude 2.1.299: `path.replace(/[^a-zA-Z0-9]/g, "-")`, so a
/// character outside the BMP is a surrogate pair and two dashes).
///
/// THE ONE OWNER of that spelling: `run::resumable`'s probe, the rename
/// preflight and this account carry all call it, so a copy lands exactly where
/// claude wrote the transcript and the probe looks for it. The carry keys the
/// RECORDED work dir while the probe keys the pane's physical cwd (`getcwd`),
/// so a working copy reached through a link can key differently in the two:
/// the carry then finds no source transcript at its key and refuses, though
/// the probe may still resume that seat in its OLD account.
///
/// RESIDUAL: claude cuts a key longer than 200 characters at 200 and appends a
/// hash of the path. That is read in its bundle and unmeasured against a real
/// long-path folder, so it is NOT implemented here.
#[must_use]
pub fn project_key(work_dir: &Path) -> String {
    let mut key = String::new();
    for ch in work_dir.display().to_string().chars() {
        if ch.is_ascii_alphanumeric() {
            key.push(ch);
        } else {
            key.extend(std::iter::repeat_n('-', ch.len_utf16()));
        }
    }
    key
}

/// Whether a walk over the copy set WRITES. ONE walk, two uses: `Check`
/// classifies, reads and compares every node exactly as `Copy` does — the
/// transcript, the sidecar trees, project memory, depth and node budget — and
/// skips only the creation in [`make_dir`] and [`publish`] and the archiving in
/// [`set_aside`]. It proves the set as it stands at that moment and no longer:
/// the target account is outside ae's lifecycle lock, so a `Copy` carries the
/// Check's [`Proof`], walks the set once more WITHOUT writing, and acts only
/// when that walk proves the very same decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode<'a> {
    Check,
    /// Write, archiving exactly what this Check proved — or nothing at all.
    Copy(&'a Proof),
}

/// CHECK or COPY the conversation. A `Copy` `Err` is the loud fallback: the
/// seat still moves, on a fresh conversation with the seed pack, and the
/// reason is what ae prints.
pub(crate) fn run(plan: &Plan, mode: Mode<'_>) -> Result<Crossing, String> {
    let mut aside = Aside::default();
    match carry(plan, mode, &mut aside) {
        Ok(mut crossing) => {
            crossing.archived = aside
                .dir
                .filter(|_| aside.count > 0)
                .map(|dir| (aside.count, dir));
            Ok(crossing)
        }
        // A COPY THAT ARCHIVED AND THEN FAILED has still moved those files, so
        // the fallback after it must not make that a silent archive: the
        // reason says how many and where. A directory it created and then
        // could not fill is named too, never removed behind the reader's back.
        Err(why) => Err(match aside.dir {
            Some(dir) => format!(
                "{why}; {} already archived in {}",
                files(aside.count),
                dir.display()
            ),
            None => why,
        }),
    }
}

/// `1 target file`, `2 target files` — the one spelling of an archive count.
pub(crate) fn files(count: usize) -> String {
    format!("{count} target file{}", if count == 1 { "" } else { "s" })
}

/// What one Copy has archived so far.
#[derive(Default)]
struct Aside {
    /// The move's fresh directory, created right before its FIRST archive.
    dir: Option<PathBuf>,
    count: usize,
}

fn carry(plan: &Plan, mode: Mode<'_>, aside: &mut Aside) -> Result<Crossing, String> {
    let name = format!("{}.jsonl", plan.id);
    let project: [&str; 2] = ["projects", &plan.key];
    let transcript: [&str; 3] = [project[0], project[1], &name];
    // THE CONVERSATION ITSELF, read before anything is written: a carry that
    // cannot read the thing it exists to move has nothing to do.
    let (source_file, node) = under(&plan.from, &transcript)?;
    let bytes = read_regular(&source_file, node)?;
    // THE TARGET PROJECT DIRECTORY, before anything is put in it. `under`
    // refuses a link at every level, so this also proves the two components
    // above it.
    let (target, _) = under(&plan.to, &project)?;
    // ...and what the target already holds under the conversation's name,
    // decided BEFORE the sidecars are walked, so a target holding a different
    // conversation costs nothing but the reading.
    let (target_file, node) = under(&plan.to, &transcript)?;
    let held = held(&target_file, node, &bytes)?;
    let mut proof = Proof {
        stale: match &held {
            Held::Stale(print) => Some(print.clone().within(&plan.to)?),
            Held::Missing | Held::Same => None,
        },
        conflicts: Vec::new(),
    };
    // THE SURVEY: every sidecar walked WITHOUT writing, in both modes. For a
    // Check it IS the proof; for a Copy it is the claim-before-write — the
    // whole set re-read after the stop, so a frozen file that changed or
    // became equal and a difference the Check never saw all abandon the copy
    // with nothing written.
    let mut budget = MAX_NODES;
    for parts in sidecars(&project, &plan.id) {
        proof
            .conflicts
            .extend(copy_tree(plan, &parts, &mut budget, Mode::Check)?);
    }
    proof.conflicts.sort_by(|a, b| a.relative.cmp(&b.relative));
    // PROJECT MEMORY is not uuid-keyed: it belongs to the working copy and is
    // shared by every conversation in that account. ae copies it only into an
    // account that has none, and NEVER merges two — the human's ruling, and the
    // only safe one, since a merge cannot be undone by hand. Only a real
    // directory is that account's own memory; anything else there is a target
    // ae cannot explain, and it says which.
    let memory: [&str; 3] = [project[0], project[1], MEMORY];
    let (target_memory, node) = under(&plan.to, &memory)?;
    let memory_kept = match node {
        Node::Missing => false,
        Node::Dir => true,
        other => return Err(format!("{} {}", target_memory.display(), other.word())),
    };
    if !memory_kept {
        copy_tree(plan, &memory, &mut budget, Mode::Check)?;
    }
    let Mode::Copy(checked) = mode else {
        // WHERE THE COPY WILL ARCHIVE, proven before anything is stopped.
        archivable(plan, &proof)?;
        return Ok(Crossing {
            memory_kept,
            proof,
            archived: None,
        });
    };
    // THE CHECK DECIDED, so the Copy acts on exactly that decision or on
    // nothing: the human's move was decided on what the Check saw.
    if let Some(path) = drift(&proof, checked) {
        return Err(changed(&plan.to.join(path)));
    }
    // Each frozen sidecar goes aside now, proven AGAIN at the moment it moves;
    // the walk below then publishes into the names they left.
    for print in &checked.conflicts {
        set_aside(plan, print, aside, mode)?;
    }
    let mut budget = MAX_NODES;
    for parts in sidecars(&project, &plan.id) {
        copy_tree(plan, &parts, &mut budget, mode)?;
    }
    if !memory_kept {
        copy_tree(plan, &memory, &mut budget, mode)?;
    }
    // THE COMMIT. Last, so everything above is already in place when the
    // conversation becomes findable.
    match (held, &checked.stale) {
        // PROVEN AGAIN, at the boundary. The reading above happened before the
        // sidecars were copied; this one is what the caller's `Ok` rests on,
        // because the target account is not under ae's lifecycle lock and the
        // successor is about to resume exactly these bytes.
        (Held::Same, _) => {
            let (_, node) = under(&plan.to, &transcript)?;
            still_holds(&target_file, node, &bytes)?;
        }
        // THE OLDER COPY GOES ASIDE ONLY NOW, right before the conversation
        // takes its name, so the commit marker is still the last name written.
        (Held::Stale(_), Some(print)) => {
            set_aside(plan, print, aside, mode)?;
            publish(&target_file, &bytes)?;
        }
        // Unreachable past the drift proof, and spelled rather than assumed.
        (Held::Stale(_), None) => return Err(changed(&target_file)),
        (Held::Missing, _) => {
            make_dir(&target, mode)?;
            publish(&target_file, &bytes)?;
        }
    }
    Ok(Crossing {
        memory_kept,
        proof: Proof::default(),
        archived: None,
    })
}

/// The sidecar trees of one conversation, in the order they are walked: the
/// uuid directory beside the transcript, then each uuid-keyed store root.
fn sidecars<'a>(project: &[&'a str; 2], id: &'a str) -> Vec<Vec<&'a str>> {
    let mut trees = vec![vec![project[0], project[1], id]];
    trees.extend(SIDECAR_ROOTS.iter().map(|root| vec![*root, id]));
    trees
}

/// The first account-relative path two proofs disagree on, or `None` when
/// they are the same decisions about the same nodes.
fn drift<'a>(now: &'a Proof, checked: &'a Proof) -> Option<&'a Path> {
    if now.stale != checked.stale {
        return now
            .stale
            .as_ref()
            .or(checked.stale.as_ref())
            .map(|print| print.relative.as_path());
    }
    let mut ours = now.conflicts.iter();
    let mut theirs = checked.conflicts.iter();
    loop {
        match (ours.next(), theirs.next()) {
            (None, None) => return None,
            (Some(a), Some(b)) if a == b => {}
            (Some(a), Some(b)) => return Some(a.relative.as_path().min(b.relative.as_path())),
            (Some(print), None) | (None, Some(print)) => return Some(&print.relative),
        }
    }
}

/// What the target already holds under the conversation's name.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Held {
    Missing,
    /// The very bytes ae is carrying: an interrupted attempt, or a carry that
    /// already finished.
    Same,
    /// A STRICT byte prefix of them: the older copy a round trip left behind.
    Stale(Print),
}

/// Judge the target transcript against the source's bytes. Anything that is
/// neither the same conversation nor an older copy of it — a fork, a longer
/// file — belongs to something else, and the carry refuses.
fn held(path: &Path, node: Node, source: &[u8]) -> Result<Held, String> {
    match node {
        Node::Missing => return Ok(Held::Missing),
        Node::File => {}
        other => return Err(format!("{} {}", path.display(), other.word())),
    }
    let different = || {
        format!(
            "{} already holds a different conversation file",
            path.display()
        )
    };
    // THE LENGTH FIRST: a target longer than the source is refused unread, so
    // the read below is never larger than the source ae already holds.
    let before = stamp(path)?;
    if usize::try_from(before.len).map_or(true, |len| len > source.len()) {
        return Err(different());
    }
    let bytes = read_regular(path, node)?;
    if bytes == source {
        return Ok(Held::Same);
    }
    if bytes.len() < source.len() && source.starts_with(&bytes) {
        return Ok(Held::Stale(print(path, before, &bytes)?));
    }
    Err(different())
}

/// A [`Print`] of the bytes just read from `path`, whose identity `before` was
/// taken before the read. A node that moved WHILE it was read is not the node
/// the bytes came from, so it proves nothing. The path is the absolute one;
/// [`Print::within`] makes it account-relative.
fn print(path: &Path, before: Stamp, bytes: &[u8]) -> Result<Print, String> {
    if stamp(path)? != before || u64::try_from(bytes.len()).ok() != Some(before.len) {
        return Err(changed(path));
    }
    Ok(Print {
        relative: path.to_path_buf(),
        stamp: before,
        digest: crate::install::sha256_hex(bytes),
    })
}

impl Print {
    /// The same print, its path made relative to `account`.
    fn within(self, account: &Path) -> Result<Self, String> {
        let relative = self
            .relative
            .strip_prefix(account)
            .map_err(|_| {
                format!(
                    "{} is outside {}",
                    self.relative.display(),
                    account.display()
                )
            })?
            .to_path_buf();
        Ok(Self { relative, ..self })
    }
}

/// Prove the target file a [`Print`] names is STILL that node holding those
/// bytes, walking every component from the account root without following a
/// link. Asked right before the file is moved aside.
///
/// The identity is taken again AFTER the read, by [`print`]: a node replaced
/// or written while it was read would otherwise pass on the old bytes.
fn still_printed(account: &Path, frozen: &Print) -> Result<PathBuf, String> {
    let parts: Vec<&OsStr> = frozen.relative.iter().collect();
    let (path, node) = under(account, &parts)?;
    if node != Node::File || stamp(&path)? != frozen.stamp {
        return Err(changed(&path));
    }
    let bytes = read_regular(&path, node)?;
    if print(&path, frozen.stamp, &bytes)?.digest != frozen.digest {
        return Err(changed(&path));
    }
    Ok(path)
}

/// The words for a target node that is not what the Check saw.
fn changed(path: &Path) -> String {
    format!("{} changed after ae checked it", path.display())
}

/// Move one proven file ASIDE: renamed, inside the same account, into this
/// move's own fresh directory under its account-relative path. Never a
/// delete, and never over a name that is there.
///
/// The destination is prepared FIRST — the directory, its parents — and only
/// then is the file proven against its [`Print`] and the destination proven
/// absent by the same no-link walk from the account root, so nothing ae does
/// stands between that proof and the rename. A file that changed leaves its
/// name where it was, and the fallback names the directory, empty or not.
/// RESIDUAL, named rather than closed: a writer of the same OS user changing
/// the file, or planting that destination inside ae's own fresh directory,
/// between those proofs and the rename — `RENAME_NOREPLACE` has no safe-Rust
/// spelling without libc.
fn set_aside(plan: &Plan, print: &Print, aside: &mut Aside, mode: Mode<'_>) -> Result<(), String> {
    let dir = if let Some(dir) = &aside.dir {
        dir.clone()
    } else {
        let dir = fresh_archive(plan, mode)?;
        aside.dir = Some(dir.clone());
        dir
    };
    let into = dir.join(&print.relative);
    if let Some(parent) = into.parent() {
        make_dir(parent, mode)?;
    }
    let relative = into
        .strip_prefix(&plan.to)
        .map_err(|_| format!("{} is outside {}", into.display(), plan.to.display()))?;
    let parts: Vec<&OsStr> = relative.iter().collect();
    let path = still_printed(&plan.to, print)?;
    match under(&plan.to, &parts)? {
        (_, Node::Missing) => {}
        (into, other) => return Err(format!("{} {}", into.display(), other.word())),
    }
    std::fs::rename(&path, &into)
        .map_err(|why| format!("could not archive {} ({why})", path.display()))?;
    aside.count += 1;
    Ok(())
}

/// This move's own archive directory, `0700`, created EXCLUSIVELY: a name that
/// is already there — another move in the same second, or a crashed one — is
/// never reused; the next name is tried instead.
fn fresh_archive(plan: &Plan, mode: Mode<'_>) -> Result<PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt as _;

    let (base, _) = under(&plan.to, &[ARCHIVE, plan.id.as_str()])?;
    make_dir(&base, mode)?;
    let stamp: String = crate::time::Timestamp::now()
        .to_string()
        .chars()
        .filter(|ch| *ch != '-' && *ch != ':')
        .collect();
    for n in 0..ARCHIVE_TRIES {
        let dir = base.join(format!("{stamp}-{}.{n}", std::process::id()));
        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => return Ok(dir),
            Err(why) if why.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(why) => return Err(format!("could not create {} ({why})", dir.display())),
        }
    }
    Err(format!(
        "{} has no free name for this move's archive",
        base.display()
    ))
}

/// Could a Copy archive what this Check found? Proven WITHOUT writing: every
/// directory the archive writes in — the deepest one that already exists on
/// its own path, and the directory each archived file leaves — is a real
/// directory its owner may write in and search, on the device the archived
/// file is on, so the rename is neither refused nor asked to cross a mount.
///
/// A PROXY, said as one: the mode bits are what a Check can read without a
/// write and without libc. An ACL, a read-only mount, a file flag or a
/// directory another user owns can still refuse the Copy, and that is met by
/// the Copy's own failure — the loud fallback to the seed path.
fn archivable(plan: &Plan, proof: &Proof) -> Result<(), String> {
    if proof.is_empty() {
        return Ok(());
    }
    let path = [ARCHIVE, plan.id.as_str()];
    let mut deepest = plan.to.clone();
    for depth in 1..=path.len() {
        match under(&plan.to, &path[..depth])? {
            (dir, Node::Dir) => deepest = dir,
            (_, Node::Missing) => break,
            (other, node) => return Err(format!("{} {}", other.display(), node.word())),
        }
    }
    let device = writable(&deepest)?;
    for print in proof.stale.iter().chain(&proof.conflicts) {
        let node = plan.to.join(&print.relative);
        let parent = node.parent().unwrap_or(&plan.to);
        if writable(parent)? != device || print.stamp.dev != device {
            return Err(format!(
                "{} is on another file system than {}, so ae could not archive it there",
                node.display(),
                deepest.display()
            ));
        }
    }
    Ok(())
}

/// The device of `dir`, when it is a real directory its owner may write in and
/// search.
fn writable(dir: &Path) -> Result<u64, String> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    match lstat(dir)? {
        Some(meta) if meta.file_type().is_dir() => {
            if meta.permissions().mode() & 0o300 == 0o300 {
                Ok(meta.dev())
            } else {
                Err(format!(
                    "{} is not writable, so ae could not archive the target's own files there",
                    dir.display()
                ))
            }
        }
        Some(_) | None => Err(changed(dir)),
    }
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
/// source that is not there is `Ok`; anything present is binding. What a Check
/// would archive comes back, account-relative.
fn copy_tree(
    plan: &Plan,
    parts: &[&str],
    budget: &mut usize,
    mode: Mode<'_>,
) -> Result<Vec<Print>, String> {
    let (source, node) = under(&plan.from, parts)?;
    match node {
        Node::Missing => Ok(Vec::new()),
        Node::Dir => {
            let (target, node) = under(&plan.to, parts)?;
            match node {
                Node::Missing | Node::Dir => copy_dir(&source, &target, MAX_DEPTH, budget, mode)?
                    .into_iter()
                    .map(|print| print.within(&plan.to))
                    .collect(),
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
    mode: Mode<'_>,
) -> Result<Vec<Print>, String> {
    if depth == 0 {
        return Err(format!(
            "{} is nested deeper than ae will copy",
            source.display()
        ));
    }
    make_dir(target, mode)?;
    let mut conflicts = Vec::new();
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
            Node::Dir => conflicts.extend(copy_dir(&entry, &into, depth - 1, budget, mode)?),
            Node::File => conflicts.extend(copy_file(&entry, &into, mode)?),
            // A LISTED ENTRY THAT IS GONE is a source ae cannot promise it
            // copied, exactly like one it cannot read. The copy set is binding,
            // so this abandons the carry rather than delivering it short.
            other => return Err(format!("{} {}", entry.display(), other.word())),
        }
    }
    Ok(conflicts)
}

/// One file into a target that must not already hold a different one. A Check
/// that meets a different REGULAR file prints it for archiving instead — one
/// file's bytes at a time, so the proof stays fixed-size per file; a Copy
/// meets none, because the files the Check printed are already aside.
fn copy_file(source: &Path, target: &Path, mode: Mode<'_>) -> Result<Option<Print>, String> {
    let bytes = read_regular(source, Node::File)?;
    match classify(target)? {
        Node::Missing if mode == Mode::Check => Ok(None),
        Node::Missing => publish(target, &bytes).map(|()| None),
        Node::File => {
            let before = stamp(target)?;
            let held = read_regular(target, Node::File)?;
            if held == bytes {
                Ok(None)
            } else if mode == Mode::Check {
                print(target, before, &held).map(Some)
            } else {
                Err(format!("{} already exists", target.display()))
            }
        }
        other => Err(format!("{} {}", target.display(), other.word())),
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
fn under<S: AsRef<OsStr>>(root: &Path, parts: &[S]) -> Result<(PathBuf, Node), String> {
    let mut full = root.to_path_buf();
    for part in parts {
        let part = Path::new(part.as_ref());
        let mut components = part.components();
        if !matches!(components.next(), Some(std::path::Component::Normal(_)))
            || components.next().is_some()
        {
            return Err(format!("'{}' is not a plain file name", part.display()));
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
        walk.push(part.as_ref());
        node = classify(&walk)?;
    }
    Ok((full, node))
}

/// The metadata of `path`, without following it. `None` is a PROVEN absence
/// and nothing else: a node ae cannot even stat is a node it cannot promise it
/// copied, so every other failure is an error and the carry is abandoned
/// loudly rather than reported complete without it.
fn lstat(path: &Path) -> Result<Option<std::fs::Metadata>, String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the carry classifies every store node without following it, so a link can never redirect a copy out of the account it names"
    )]
    let meta = std::fs::symlink_metadata(path);
    match meta {
        Ok(meta) => Ok(Some(meta)),
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(why) => Err(format!("could not read {} ({why})", path.display())),
    }
}

/// What `path` is, without following it.
fn classify(path: &Path) -> Result<Node, String> {
    let Some(meta) = lstat(path)? else {
        return Ok(Node::Missing);
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

/// Which regular file `path` is right now. Anything else there is a node that
/// changed under the reading that classified it.
fn stamp(path: &Path) -> Result<Stamp, String> {
    use std::os::unix::fs::MetadataExt as _;

    match lstat(path)? {
        Some(meta) if meta.file_type().is_file() => Ok(Stamp {
            dev: meta.dev(),
            ino: meta.ino(),
            len: meta.len(),
            modified: meta.modified().ok(),
        }),
        Some(_) | None => Err(changed(path)),
    }
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
    fn the_project_key_is_claudes_rule_every_non_ascii_alphanumeric_unit_flattened() {
        // The ORACLE is claude's own rule, not this function: claude 2.1.299
        // keys a project with `path.replace(/[^a-zA-Z0-9]/g, "-")`, and every
        // entry under `~/.claude*/projects` on the measuring host matches
        // `^[A-Za-z0-9-]+$`. That regex has no `u` flag, so it works per UTF-16
        // code unit: a BMP character (`é`) is one dash and a non-BMP one
        // (`😀`, a surrogate pair) is two — measured with node on 2026-10-14.
        for (path, key) in [
            ("/Users/x/projects/ae", "-Users-x-projects-ae"),
            ("/Users/u/.ae/worktrees/571", "-Users-u--ae-worktrees-571"),
            ("/a/b_c d.e", "-a-b-c-d-e"),
            ("/a/é.b", "-a---b"),
            ("/a/é😀.b", "-a-----b"),
        ] {
            assert_eq!(super::project_key(Path::new(path)), key, "{path}");
        }
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
                    &plan.to.join("projects/-w").join(format!("{ID}.jsonl")),
                    b"forked transcript",
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

    // ADD-only v1.1 proof pins; no product seam or implementation edits.

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the fixture reads its own regular file before a real same-length inode replacement"
    )]
    fn v11_a_same_length_replacement_cannot_freeze_the_previous_inode() {
        let dir = scratch("rr11inode");
        let path = dir.join("held");
        put(&path, b"old");
        let before = super::stamp(&path).expect("fixture regular file");
        let bytes = std::fs::read(&path).expect("fixture old bytes");
        std::fs::rename(&path, dir.join("previous-inode")).expect("fixture old inode retained");
        put(&path, b"new");
        let after = super::stamp(&path).expect("fixture replacement");
        assert_eq!(after.len, before.len, "fixture same-length replacement");
        assert_ne!(after.ino, before.ino, "fixture distinct live inodes");
        let frozen = super::print(&path, before, &bytes);
        std::fs::remove_dir_all(&dir).expect("fixture cleanup");
        assert!(
            frozen.is_err(),
            "R4 a read from another inode is no frozen proof: {frozen:?}"
        );
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the fixture reads a pre-planted archive marker in its own scratch account"
    )]
    fn v11_a_preplanted_archive_collision_uses_the_next_nonce_without_reuse() {
        let plan = planted("rr11collision");
        let base = plan.to.join("ae-carry-archive").join(ID);
        let start = crate::time::Timestamp::now().epoch();
        // Cover every second in a bounded interval, including a clock boundary;
        // a successful retry never depends on two calls sharing one second.
        for second in start - 2..=start + 120 {
            let utc = crate::time::Timestamp::from_epoch(second).to_string();
            let stamp = format!(
                "{}{}{}T{}{}{}Z",
                &utc[..4],
                &utc[5..7],
                &utc[8..10],
                &utc[11..13],
                &utc[14..16],
                &utc[17..19]
            );
            let taken = base.join(format!("{stamp}-{}.0", std::process::id()));
            put(&taken.join("keep"), b"earlier archive bytes");
        }
        let now = crate::time::Timestamp::now().epoch();
        assert!(
            (start - 2..=start + 120).contains(&now),
            "fixture current second has a planted .0"
        );
        let fresh = super::fresh_archive(&plan, super::Mode::Copy(&super::Proof::default()))
            .expect("R3 taken .0 must retry a fresh nonce");
        let name = fresh
            .file_name()
            .expect("nonce name")
            .to_str()
            .expect("ASCII nonce");
        let (prefix, n) = name.rsplit_once('.').expect("documented counter");
        assert_eq!(n, "1", "R3 .0 is taken; next nonce is .1");
        let taken = base.join(format!("{prefix}.0"));
        assert_eq!(
            std::fs::read(taken.join("keep")).expect("earlier archive kept"),
            b"earlier archive bytes"
        );
        assert!(
            fresh.is_dir() && fresh != taken,
            "R3 exclusively fresh directory"
        );
        assert!(
            std::fs::read_dir(&fresh)
                .expect("new directory readable")
                .next()
                .is_none(),
            "R3 no reuse of old archive"
        );
        std::fs::remove_dir_all(plan.from.parent().expect("fixture root"))
            .expect("fixture cleanup");
    }

    #[test]
    fn v11_a_readonly_archive_parent_reports_the_actual_permission_failure() {
        use std::os::unix::fs::PermissionsExt as _;
        let plan = planted("rr11permission");
        let base = plan.to.join("ae-carry-archive").join(ID);
        std::fs::create_dir_all(&base).expect("fixture archive parent");
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o500))
            .expect("fixture readonly parent");
        let fixture = std::fs::create_dir(base.join("permission-probe"));
        let result = super::fresh_archive(&plan, super::Mode::Copy(&super::Proof::default()));
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700))
            .expect("fixture permissions restored");
        std::fs::remove_dir_all(plan.from.parent().expect("fixture root"))
            .expect("fixture cleanup");
        assert_eq!(
            fixture
                .expect_err("fixture requires an unprivileged runner")
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        let why = result.expect_err("R4 archive creation is denied");
        assert!(
            why.contains(&base.display().to_string())
                && why.to_ascii_lowercase().contains("permission denied"),
            "R4 actual IO cause and archive path are retained: {why}"
        );
    }

    #[test]
    fn v11_the_file_device_must_match_even_when_its_parent_device_matches() {
        let plan = planted("rr11device");
        put(
            &plan.to.join("projects/-w").join(format!("{ID}.jsonl")),
            b"trans",
        );
        let checked = super::run(&plan, super::Mode::Check).expect("fixture local prefix proof");
        let mut proof = checked.proof.clone();
        assert!(
            super::archivable(&plan, &proof).is_ok(),
            "fixture local devices match"
        );
        // A mounted file can have a different device from its containing
        // directory. Exercise this proof value without privileged mounts;
        // the real-directory reads and BOTH-device comparison stay unmocked.
        proof.stale.as_mut().expect("fixture stale file").stamp.dev ^= 1;
        let result = super::archivable(&plan, &proof);
        std::fs::remove_dir_all(plan.from.parent().expect("fixture root"))
            .expect("fixture cleanup");
        assert!(
            result.is_err_and(|why| why.contains("another file system")),
            "R4 known cross-device file must refuse during Check"
        );
    }

    #[test]
    fn v11_owner_write_and_search_bits_do_not_make_a_regular_file_a_directory() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch("rr11directory");
        let path = dir.join("file");
        put(&path, b"regular file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("fixture owner mode bits");
        let result = super::writable(&path);
        std::fs::remove_dir_all(&dir).expect("fixture cleanup");
        assert!(
            result.is_err(),
            "R4 archive proxy proves a real directory, not mode bits alone"
        );
    }

    #[test]
    fn v11_a_regular_file_stamp_refuses_a_real_directory_and_a_link() {
        let dir = scratch("rr11regular");
        let path = dir.join("file");
        put(&path, b"regular file");
        let link = dir.join("link");
        std::os::unix::fs::symlink(&path, &link).expect("fixture link");
        assert!(super::stamp(&path).is_ok(), "fixture positive regular file");
        let directory = super::stamp(&dir);
        let linked = super::stamp(&link);
        std::fs::remove_dir_all(&dir).expect("fixture cleanup");
        assert!(
            directory.is_err(),
            "R4 a directory cannot become a regular-file fingerprint"
        );
        assert!(
            linked.is_err(),
            "R4 a link cannot become a regular-file fingerprint"
        );
    }
}
