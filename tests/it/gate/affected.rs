//! The merge-gate selector behind `just test-affected` (M1, one hop): the integration tests a
//! change can reach directly, or why the whole suite must run.
//!
//! The recipe writes the change as framed records ([`decode`]); the selector answers in the line
//! protocol of [`render`]. Every unit test and the guard owners `doors`, `floor` and `gate` always
//! run. A changed integration file selects every module that owns it. A changed library module
//! selects the integration modules whose own file names it: an `ae::` path or `use` of it, or a
//! name that `src/lib.rs` or its parent module re-exports from it. Every changed path is also
//! data: an `include*!`, a `#[path]` or a string literal that names it selects the reader, and a
//! library module that includes it counts as changed. What the rules cannot place, and every
//! build input, config, crate root, module-map, shared-rig, deletion, rename and macro change,
//! runs the whole suite and names why.
//!
//! The accepted M1 gaps: a test that reaches a changed module only through another module
//! (A -> B -> C), a test that drives only the binary, and run-time or macro-built paths. Every
//! subset run prints their size as the counter line; the release's whole hp run is the backstop.
//!
//! Pure, std only: `examples/affected.rs` mounts this file and owns stdin and stdout.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use super::modtree::{self, Crate, Files, Kind, Lexed, ModPath, join_path, normalize};

/// Where the stream's change set comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The committed range `<merge-base>..HEAD` of a clean checkout.
    Gate,
    /// The working tree against the merge-base, untracked files included (`print=yes`).
    Preview,
}

/// One changed path's git status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Added,
    Modified,
    Deleted,
    TypeChanged,
    Renamed {
        from: String,
    },
    Copied {
        from: String,
    },
    /// An untracked file; only a preview stream carries one.
    Untracked,
}

/// One changed path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) status: Status,
    pub(crate) path: String,
}

/// The decoded stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Input {
    pub(crate) mode: Mode,
    /// The base ref as the recipe received it.
    pub(crate) base: String,
    pub(crate) merge_base: String,
    pub(crate) head: String,
    pub(crate) changes: Vec<Change>,
    /// Every repository path of the new side.
    pub(crate) tracked: BTreeSet<String>,
    /// The text of every `.rs` file under `src/` and `tests/` on the new side.
    pub(crate) new: Files,
    /// The merge-base text of every modified (`M`, `T`) `.rs` file under `src/` and `tests/`.
    pub(crate) old: Files,
}

/// The decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Whole,
    Subset,
}

/// Why the whole suite runs: a stable identifier and the path or fact behind it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Reason {
    pub(crate) id: &'static str,
    pub(crate) detail: String,
}

/// Why an owner is selected, or a data form the rules could not resolve (`unresolved`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Why {
    pub(crate) owner: String,
    pub(crate) file: String,
    pub(crate) line: usize,
    /// `own`, `namespace`, `reexport`, `include`, `path-attr`, `literal`, `basename` or `unresolved`.
    pub(crate) rule: &'static str,
    pub(crate) target: String,
}

/// The selection the protocol prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub(crate) verdict: Verdict,
    pub(crate) reasons: Vec<Reason>,
    /// The selected integration owners, the guards included.
    pub(crate) owners: BTreeSet<String>,
    pub(crate) why: Vec<Why>,
    /// A `.rs` file under `src/` changed: the subset also runs the doctests.
    pub(crate) doctests: bool,
    /// `install` changed: the run adds `just check`.
    pub(crate) check: bool,
    /// Statically enumerated integration tests outside every selected owner.
    pub(crate) not_selected: usize,
    /// Statically enumerated integration tests.
    pub(crate) total: usize,
    pub(crate) filterset: String,
}

/// The integration owners every run selects.
const GUARDS: [&str; 3] = ["doors", "floor", "gate"];
/// The selector's own sources: a change to them is a change to the gate itself.
const SELECTOR_SOURCES: [&str; 3] = [
    "examples/affected.rs",
    "tests/it/gate/affected.rs",
    "tests/it/gate/modtree.rs",
];
const BUILD_INPUTS: [&str; 4] = [
    "Cargo.toml",
    "Cargo.lock",
    "build.rs",
    "rust-toolchain.toml",
];
const LIB_ROOT: &str = "src/lib.rs";
const BIN_ROOT: &str = "src/main.rs";
const IT_ROOT: &str = "tests/it/main.rs";
const MAC_MANIFEST: &str = ".config/macos-subset";

/// The extension of a repository path's last component.
fn extension(path: &str) -> Option<&str> {
    std::path::Path::new(path)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
}

/// A Rust source path.
fn is_rust(path: &str) -> bool {
    extension(path) == Some("rs")
}

/// A `.rs` file under `src/` or `tests/`: the stream carries its text.
fn is_source(path: &str) -> bool {
    is_rust(path) && (path.starts_with("src/") || path.starts_with("tests/"))
}

// ---------------------------------------------------------------------------
// The stream
// ---------------------------------------------------------------------------

/// The record tags, in stream order.
const TAGS: [&str; 10] = [
    "version",
    "mode",
    "base",
    "merge-base",
    "head",
    "change",
    "tracked",
    "new",
    "old",
    "end",
];
/// The first repeatable record class (`change`); `tracked`, `new` and `old` follow it.
const FIRST_CLASS: usize = 5;
/// The last repeatable record class (`old`).
const LAST_CLASS: usize = 8;

/// Decode the recipe's stream, or name the first way it is malformed (reason `stream`).
///
/// A record is `<tag> <length>\n`, exactly `length` payload bytes and `\n`. The records are
/// `version` (`1`), `mode`, `base`, `merge-base` and `head` once each, in that order; then any
/// number of `change`, `tracked`, `new` and `old` records, in that class order; then `end`,
/// whose payload counts the records before it, and nothing after it. A changed `.rs` file under
/// `src/` or `tests/` needs its `new` record unless it is deleted, and exactly the modified
/// (`M`, `T`) ones need an `old` record.
pub(crate) fn decode(bytes: &[u8]) -> Result<Input, Reason> {
    let mut records = frame(bytes)?.into_iter();
    if expect(&mut records, 0)? != b"1" {
        return Err(stream("version is not 1"));
    }
    let mode = match expect(&mut records, 1)? {
        b"gate" => Mode::Gate,
        b"preview" => Mode::Preview,
        _ => return Err(stream("mode is neither gate nor preview")),
    };
    let base = text(expect(&mut records, 2)?, "base")?;
    if base.is_empty() || base.contains(char::is_control) {
        return Err(stream("base is empty or holds a control character"));
    }
    let merge_base = sha(expect(&mut records, 3)?, "merge-base")?;
    let head = sha(expect(&mut records, 4)?, "head")?;
    let mut input = Input {
        mode,
        base,
        merge_base,
        head,
        changes: Vec::new(),
        tracked: BTreeSet::new(),
        new: Files::new(),
        old: Files::new(),
    };
    let mut changed = BTreeSet::new();
    let mut last = FIRST_CLASS;
    for (tag, payload) in records {
        if !(FIRST_CLASS..=LAST_CLASS).contains(&tag) || tag < last {
            return Err(stream(format!("{} is out of order", TAGS[tag])));
        }
        last = tag;
        let duplicate = match TAGS[tag] {
            "change" => {
                let change = change(payload, mode)?;
                let fresh = changed.insert(change.path.clone());
                input.changes.push(change);
                !fresh
            }
            "tracked" => !input.tracked.insert(path(payload, "tracked")?),
            "new" => {
                let (path, content) = content(payload, "new")?;
                input.new.insert(path, content).is_some()
            }
            _ => {
                let (path, content) = content(payload, "old")?;
                input.old.insert(path, content).is_some()
            }
        };
        if duplicate {
            return Err(stream(format!("{} repeats a path", TAGS[tag])));
        }
    }
    for change in &input.changes {
        if change.status != Status::Deleted
            && is_source(&change.path)
            && !input.new.contains_key(&change.path)
        {
            return Err(stream(format!("change {} has no new record", change.path)));
        }
    }
    let wants_old: BTreeSet<&str> = input
        .changes
        .iter()
        .filter(|change| {
            matches!(change.status, Status::Modified | Status::TypeChanged)
                && is_source(&change.path)
        })
        .map(|change| change.path.as_str())
        .collect();
    if let Some(missing) = wants_old
        .iter()
        .find(|path| !input.old.contains_key(**path))
    {
        return Err(stream(format!("modified {missing} has no old record")));
    }
    if let Some(spurious) = input
        .old
        .keys()
        .find(|path| !wants_old.contains(path.as_str()))
    {
        return Err(stream(format!(
            "old record for {spurious}, which is not a modified source"
        )));
    }
    Ok(input)
}

/// The protocol answer to the bytes the recipe sent. A malformed stream runs the whole suite
/// with reason `stream`.
pub(crate) fn respond(stream: &[u8]) -> String {
    render(&match decode(stream) {
        Ok(input) => select(&input),
        Err(reason) => unreadable(reason),
    })
}

/// The whole-suite selection for a stream that could not be decoded. Nothing says `install` did
/// not change, so the install checks run too.
fn unreadable(reason: Reason) -> Selection {
    Selection {
        verdict: Verdict::Whole,
        reasons: vec![reason],
        owners: BTreeSet::new(),
        why: Vec::new(),
        doctests: false,
        check: true,
        not_selected: 0,
        total: 0,
        filterset: String::new(),
    }
}

fn stream(detail: impl Into<String>) -> Reason {
    Reason {
        id: "stream",
        detail: detail.into(),
    }
}

/// Split the stream into (tag index, payload) records up to `end`, checking every header and
/// length, the count `end` carries, and that nothing follows it.
fn frame(bytes: &[u8]) -> Result<Vec<(usize, &[u8])>, Reason> {
    let mut records = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let newline = bytes[at..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| at + offset)
            .ok_or_else(|| stream("a record header has no newline"))?;
        let header = std::str::from_utf8(&bytes[at..newline])
            .map_err(|_| stream("a record header is not UTF-8"))?;
        let (tag, length) = header
            .split_once(' ')
            .ok_or_else(|| stream(format!("header {header:?} has no length")))?;
        let index = TAGS
            .iter()
            .position(|known| *known == tag)
            .ok_or_else(|| stream(format!("unknown tag {tag:?}")))?;
        let length = count(length)
            .ok_or_else(|| stream(format!("{tag} length {length:?} is not a plain decimal")))?;
        let start = newline + 1;
        let end = start
            .checked_add(length)
            .filter(|end| *end < bytes.len())
            .ok_or_else(|| stream(format!("{tag} payload is truncated")))?;
        if bytes[end] != b'\n' {
            return Err(stream(format!("{tag} payload does not end at its length")));
        }
        let payload = &bytes[start..end];
        at = end + 1;
        if tag == "end" {
            if std::str::from_utf8(payload).ok().and_then(count) != Some(records.len()) {
                return Err(stream(format!(
                    "end does not count {} records",
                    records.len()
                )));
            }
            if at != bytes.len() {
                return Err(stream("bytes follow end"));
            }
            return Ok(records);
        }
        records.push((index, payload));
    }
    Err(stream("missing end"))
}

/// The next record's payload when its tag is `TAGS[want]`.
fn expect<'a>(
    records: &mut std::vec::IntoIter<(usize, &'a [u8])>,
    want: usize,
) -> Result<&'a [u8], Reason> {
    match records.next() {
        Some((tag, payload)) if tag == want => Ok(payload),
        Some((tag, _)) => Err(stream(format!(
            "{} where {} belongs",
            TAGS[tag], TAGS[want]
        ))),
        None => Err(stream(format!("no {} record", TAGS[want]))),
    }
}

/// A decimal count: digits only, no sign, no leading zero except `0` itself.
fn count(text: &str) -> Option<usize> {
    let plain = !text.is_empty()
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    plain.then(|| text.parse().ok()).flatten()
}

fn text(payload: &[u8], tag: &str) -> Result<String, Reason> {
    String::from_utf8(payload.to_vec()).map_err(|_| stream(format!("{tag} is not UTF-8")))
}

fn sha(payload: &[u8], tag: &str) -> Result<String, Reason> {
    let hex = payload.len() == 40
        && payload
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte));
    if !hex {
        return Err(stream(format!("{tag} is not a 40-digit hex object name")));
    }
    text(payload, tag)
}

/// A repository path: non-empty UTF-8 without control characters.
fn path(payload: &[u8], tag: &str) -> Result<String, Reason> {
    let path = text(payload, tag)?;
    if path.is_empty() || path.contains(char::is_control) {
        return Err(stream(format!(
            "{tag} path {path:?} is empty or holds a control character"
        )));
    }
    Ok(path)
}

/// A `new` or `old` payload: `<path>\0<text>`.
fn content(payload: &[u8], tag: &str) -> Result<(String, String), Reason> {
    let split = payload
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| stream(format!("{tag} has no NUL after its path")))?;
    Ok((
        path(&payload[..split], tag)?,
        text(&payload[split + 1..], tag)?,
    ))
}

/// A `change` payload: `<status>\0<path>`, or `<status>\0<from>\0<path>` for `R` and `C`.
fn change(payload: &[u8], mode: Mode) -> Result<Change, Reason> {
    let record = text(payload, "change")?;
    let fields: Vec<&str> = record.split('\0').collect();
    let scored = |status: &str, letter: char| {
        status.strip_prefix(letter).is_some_and(|score| {
            (1..=3).contains(&score.len()) && score.bytes().all(|byte| byte.is_ascii_digit())
        })
    };
    let (status, changed) = match fields.as_slice() {
        ["A", changed] => (Status::Added, *changed),
        ["M", changed] => (Status::Modified, *changed),
        ["D", changed] => (Status::Deleted, *changed),
        ["T", changed] => (Status::TypeChanged, *changed),
        ["U", changed] if mode == Mode::Preview => (Status::Untracked, *changed),
        [status, from, changed] if scored(status, 'R') => (
            Status::Renamed {
                from: path(from.as_bytes(), "change")?,
            },
            *changed,
        ),
        [status, from, changed] if scored(status, 'C') => (
            Status::Copied {
                from: path(from.as_bytes(), "change")?,
            },
            *changed,
        ),
        _ => return Err(stream(format!("change record {record:?} is malformed"))),
    };
    Ok(Change {
        status,
        path: path(changed.as_bytes(), "change")?,
    })
}

// ---------------------------------------------------------------------------
// The selection
// ---------------------------------------------------------------------------

/// Select the integration tests `input` reaches in one hop, or the whole suite with its reasons.
pub(crate) fn select(input: &Input) -> Selection {
    let mut run = Run::new(input);
    run.triggers();
    run.structure();
    run.data();
    run.namespace();
    run.unplaced();
    run.finish()
}

/// The whole-suite trigger a path is by name alone.
fn trigger(path: &str) -> Option<&'static str> {
    if SELECTOR_SOURCES.contains(&path) {
        Some("selector-source")
    } else if BUILD_INPUTS.contains(&path) || path.starts_with(".cargo/") {
        Some("build-input")
    } else if path == "justfile" {
        Some("justfile")
    } else if path.starts_with(".config/") && path != MAC_MANIFEST {
        Some("config")
    } else if path == BIN_ROOT {
        Some("crate-root")
    } else {
        None
    }
}

/// A path that adds nothing when nobody names it: docs, top-level Markdown, fuzz inputs and the
/// Mac subset manifest.
fn exempt(path: &str) -> bool {
    path.starts_with("docs/")
        || path.starts_with("fuzz/")
        || path == MAC_MANIFEST
        || (!path.contains('/') && extension(path) == Some("md"))
}

/// `path` is `owner` or inside it. The crate root `""` contains nothing here: it is never an owner.
fn within(path: &str, owner: &str) -> bool {
    !owner.is_empty()
        && path
            .strip_prefix(owner)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
}

/// A test name strictly under `owner`, the form the filterset's `^owner::` matches.
fn under(name: &str, owner: &str) -> bool {
    name.strip_prefix(owner)
        .is_some_and(|rest| rest.starts_with("::"))
}

/// Text safe for one protocol line.
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

fn tree_error(error: &modtree::TreeError) -> String {
    match error {
        modtree::TreeError::Unresolved { file, line, module } => {
            format!("{file}:{line} declares {module}, which loads no file")
        }
        modtree::TreeError::Root(root) => format!("{root} is missing"),
    }
}

/// The new side, rolled back as far as the stream carries the merge-base: added files removed,
/// modified files at their merge-base text.
fn old_side(input: &Input) -> Files {
    let mut files = input.new.clone();
    for change in &input.changes {
        match change.status {
            Status::Added | Status::Untracked | Status::Renamed { .. } | Status::Copied { .. } => {
                files.remove(&change.path);
            }
            Status::Modified | Status::TypeChanged => {
                if let Some(text) = input.old.get(&change.path) {
                    files.insert(change.path.clone(), text.clone());
                }
            }
            Status::Deleted => {}
        }
    }
    files
}

/// Each changed path, and a renamed path's old name.
fn named_paths(change: &Change) -> impl Iterator<Item = &str> {
    let from = match &change.status {
        Status::Renamed { from } => Some(from.as_str()),
        _ => None,
    };
    std::iter::once(change.path.as_str()).chain(from)
}

/// Every integration file: its owners and the module paths its text references.
type Refs = BTreeMap<String, (Vec<String>, BTreeSet<String>)>;

/// One selection in progress.
struct Run<'a> {
    input: &'a Input,
    /// The new side's module tree, once it walked.
    tree: Option<modtree::Tree>,
    /// Every module path and the files it loads, from `tree`.
    files_of: BTreeMap<ModPath, BTreeSet<String>>,
    /// Every new-side source file, lexed once.
    lexed: BTreeMap<&'a str, Lexed<'a>>,
    /// Every integration file: its owners and the module paths its text references.
    refs: Option<Refs>,
    reasons: BTreeSet<Reason>,
    owners: BTreeSet<String>,
    /// (owner, file, rule, target) to the first line that gave it.
    why: BTreeMap<(String, String, &'static str, String), usize>,
    /// Library modules counted as changed for the namespace rule.
    changed_src: BTreeSet<String>,
    /// Changed paths a rule placed: owned by a module or named by a reader.
    placed: BTreeSet<String>,
    /// Changed paths that are a whole trigger by name.
    triggered: BTreeSet<String>,
    doctests: bool,
    check: bool,
}

impl<'a> Run<'a> {
    fn new(input: &'a Input) -> Self {
        Run {
            input,
            tree: None,
            files_of: BTreeMap::new(),
            lexed: input
                .new
                .iter()
                .map(|(path, text)| (path.as_str(), modtree::lex(text)))
                .collect(),
            refs: None,
            reasons: BTreeSet::new(),
            owners: BTreeSet::new(),
            why: BTreeMap::new(),
            changed_src: BTreeSet::new(),
            placed: BTreeSet::new(),
            triggered: BTreeSet::new(),
            doctests: false,
            check: false,
        }
    }

    fn whole(&mut self, id: &'static str, detail: impl Into<String>) {
        self.reasons.insert(Reason {
            id,
            detail: clean(&detail.into()),
        });
    }

    /// Select `owner` and record why. The crate root `""` is never selected.
    fn pick(&mut self, owner: &str, file: &str, line: usize, rule: &'static str, target: &str) {
        if owner.is_empty() {
            return;
        }
        self.owners.insert(owner.to_owned());
        self.explain(owner, file, line, rule, target);
    }

    fn explain(&mut self, owner: &str, file: &str, line: usize, rule: &'static str, target: &str) {
        let first = self
            .why
            .entry((owner.to_owned(), file.to_owned(), rule, clean(target)))
            .or_insert(line);
        *first = (*first).min(line);
    }

    /// The module paths of `krate` that own `file`; the crate root is `""`.
    fn owners_of(&self, file: &str, krate: Crate) -> Vec<String> {
        self.tree
            .as_ref()
            .and_then(|tree| tree.owners.get(file))
            .map(|owners| {
                owners
                    .iter()
                    .filter(|owner| owner.krate == krate)
                    .map(|owner| owner.path.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn is_added(&self, file: &str) -> bool {
        self.input.changes.iter().any(|change| {
            change.path == file && matches!(change.status, Status::Added | Status::Untracked)
        })
    }

    /// Whole triggers by path, deletions and renames of Rust files, and the two flags.
    fn triggers(&mut self) {
        let input = self.input;
        for change in &input.changes {
            let from = match &change.status {
                Status::Renamed { from } | Status::Copied { from } => Some(from.as_str()),
                _ => None,
            };
            for path in std::iter::once(change.path.as_str()).chain(from) {
                if let Some(id) = trigger(path) {
                    self.whole(id, path);
                    self.triggered.insert(path.to_owned());
                }
                self.doctests |= path.starts_with("src/") && is_rust(path);
                self.check |= path == "install";
            }
            let rust = is_rust(&change.path) || from.is_some_and(is_rust);
            match &change.status {
                Status::Deleted if rust => self.whole("deleted", change.path.as_str()),
                Status::Renamed { from } | Status::Copied { from } if rust => {
                    self.whole("renamed", format!("{from} -> {}", change.path));
                }
                _ => {}
            }
        }
    }

    /// Both module trees, the module-map comparison, and each changed Rust file.
    fn structure(&mut self) {
        let input = self.input;
        let tree = match modtree::tree(&input.new) {
            Ok(tree) => tree,
            Err(error) => {
                self.whole(
                    "module-map",
                    format!("the new tree: {}", tree_error(&error)),
                );
                return;
            }
        };
        let old_files = old_side(input);
        match modtree::tree(&old_files) {
            Ok(old) => {
                for file in input
                    .new
                    .keys()
                    .filter(|file| old_files.contains_key(*file))
                {
                    let before = old.owners.get(file).map(Vec::as_slice).unwrap_or_default();
                    let after = tree.owners.get(file).map(Vec::as_slice).unwrap_or_default();
                    if before != after {
                        self.whole("module-map", format!("{file} changes its owners"));
                    }
                }
            }
            Err(error) => self.whole(
                "module-map",
                format!("the old tree: {}", tree_error(&error)),
            ),
        }
        for (file, owners) in &tree.owners {
            for owner in owners {
                self.files_of
                    .entry(owner.clone())
                    .or_default()
                    .insert(file.clone());
            }
        }
        self.tree = Some(tree);
        for change in &input.changes {
            if change.status == Status::Deleted || !is_rust(&change.path) {
                continue;
            }
            let Some(after) = input.new.get(&change.path) else {
                continue;
            };
            let before = input.old.get(&change.path).map_or("", String::as_str);
            self.changed_source(change, before, after);
        }
    }

    /// A changed `.rs` file: macros, crate roots, owners and the shared-rig check.
    fn changed_source(&mut self, change: &Change, before: &str, after: &str) {
        let path = change.path.as_str();
        if path == BIN_ROOT {
            return;
        }
        let (removed, added) = item_diff(before, after);
        if removed
            .iter()
            .chain(&added)
            .any(|item| item.kind == "macro_rules")
        {
            self.whole("macro", path);
        }
        if path == LIB_ROOT || path == IT_ROOT {
            self.placed.insert(path.to_owned());
            let krate = if path == LIB_ROOT {
                Crate::Lib
            } else {
                Crate::It
            };
            let root = [String::new()];
            let cheap = matches!(change.status, Status::Modified | Status::TypeChanged)
                && removed.is_empty()
                && added
                    .iter()
                    .all(|item| self.declares_added(item, krate, &root));
            if !cheap {
                self.whole("crate-root", path);
            }
            return;
        }
        let it = self.owners_of(path, Crate::It);
        let lib = self.owners_of(path, Crate::Lib);
        if !(it.is_empty() && lib.is_empty() && self.owners_of(path, Crate::Bin).is_empty()) {
            self.placed.insert(path.to_owned());
        }
        for owner in lib.into_iter().filter(|owner| !owner.is_empty()) {
            self.changed_src.insert(owner);
        }
        for owner in &it {
            self.pick(owner, path, 1, "own", path);
        }
        // Only a `#[test]` fn changes nothing another module can reach. Any other changed item
        // of a file that outside modules reference is a shared rig: an import, a helper, a
        // constant or a module declaration can change what they compute.
        if removed.iter().chain(&added).any(|item| !item.test)
            && !it.is_empty()
            && let Some(user) = self.outside_user(path, &it)
        {
            self.whole("shared-rig", format!("{path} is used by {user}"));
        }
    }

    /// Whether `item` is a `mod name;` whose module, under each of `owners`, loads only files
    /// this change adds.
    fn declares_added(&self, item: &Item, krate: Crate, owners: &[String]) -> bool {
        let Some(module) = &item.module else {
            return false;
        };
        !owners.is_empty()
            && owners.iter().all(|owner| {
                let declared = ModPath {
                    krate,
                    path: join_path(owner, module),
                };
                self.files_of.get(&declared).is_some_and(|files| {
                    !files.is_empty() && files.iter().all(|file| self.is_added(file))
                })
            })
    }

    /// An integration file outside every one of `owners`' subtrees whose own text references
    /// one of them.
    fn outside_user(&mut self, file: &str, owners: &[String]) -> Option<String> {
        if self.refs.is_none() {
            self.refs = Some(self.module_refs());
        }
        self.refs
            .as_ref()?
            .iter()
            .find(|(other, (contexts, targets))| {
                other.as_str() != file
                    && !contexts
                        .iter()
                        .all(|context| owners.iter().any(|owner| within(context, owner)))
                    && targets
                        .iter()
                        .any(|target| owners.iter().any(|owner| within(target, owner)))
            })
            .map(|(other, _)| other.clone())
    }

    /// Every integration file's owners and the module paths its text references.
    fn module_refs(&self) -> Refs {
        let mut all = BTreeMap::new();
        for (file, lexed) in &self.lexed {
            let contexts = self.owners_of(file, Crate::It);
            if contexts.is_empty() {
                continue;
            }
            let targets = module_refs(lexed, &contexts);
            all.insert((*file).to_owned(), (contexts, targets));
        }
        all
    }

    /// Rule 3.4: every changed path is data to whoever names it.
    fn data(&mut self) {
        let input = self.input;
        let wanted: Vec<&str> = input.changes.iter().flat_map(named_paths).collect();
        let mut picks = Vec::new();
        let mut unresolved = Vec::new();
        let mut placed = Vec::new();
        let mut roots = Vec::new();
        let mut modules = Vec::new();
        for (file, lexed) in &self.lexed {
            let it = self.owners_of(file, Crate::It);
            let lib = self.owners_of(file, Crate::Lib);
            if it.is_empty() && lib.is_empty() && self.owners_of(file, Crate::Bin).is_empty() {
                continue;
            }
            let refs = data_refs(lexed, file);
            for reference in &refs {
                if let DataRef::Unresolved { line, text } = reference {
                    for owner in &it {
                        unresolved.push((owner.clone(), *file, *line, text.clone()));
                    }
                }
            }
            for wanted_path in &wanted {
                let wanted_path = *wanted_path;
                let base = wanted_path.rsplit('/').next().unwrap_or(wanted_path);
                for reference in &refs {
                    let (line, rule) = match reference {
                        DataRef::Include { line, target } if target == wanted_path => {
                            (*line, "include")
                        }
                        DataRef::PathAttr { line, target } if target == wanted_path => {
                            (*line, "path-attr")
                        }
                        DataRef::Literal { line, value }
                            if value == wanted_path
                                || value
                                    .strip_suffix(wanted_path)
                                    .is_some_and(|head| head.ends_with('/')) =>
                        {
                            (*line, "literal")
                        }
                        DataRef::Literal { line, value } if value == base => (*line, "basename"),
                        _ => continue,
                    };
                    if !it.is_empty() {
                        placed.push(wanted_path);
                        for owner in &it {
                            picks.push((owner.clone(), *file, line, rule, wanted_path));
                        }
                    }
                    if rule == "include" && !(lib.is_empty() && file.starts_with("tests/")) {
                        placed.push(wanted_path);
                        if *file == LIB_ROOT || *file == BIN_ROOT {
                            roots.push((*file, wanted_path));
                        }
                        modules.extend(lib.iter().filter(|owner| !owner.is_empty()).cloned());
                    }
                }
            }
        }
        for (owner, file, line, rule, target) in picks {
            self.pick(&owner, file, line, rule, target);
        }
        for (owner, file, line, text) in unresolved {
            self.explain(&owner, file, line, "unresolved", &text);
        }
        for (file, target) in roots {
            self.whole("crate-root", format!("{file} includes {target}"));
        }
        self.placed.extend(placed.into_iter().map(str::to_owned));
        self.changed_src.extend(modules);
    }

    /// Rule 3.3: a changed library module selects every integration file that names it.
    fn namespace(&mut self) {
        let mut targets: Vec<(Vec<String>, &'static str, String)> = Vec::new();
        for module in &self.changed_src {
            for (segments, rule) in self.targets(module) {
                targets.push((segments, rule, module.clone()));
            }
        }
        if targets.is_empty() {
            return;
        }
        let mut picks = Vec::new();
        for (file, lexed) in &self.lexed {
            let owners = self.owners_of(file, Crate::It);
            if owners.is_empty() {
                continue;
            }
            for (line, reference) in ae_refs(lexed) {
                for (segments, rule, module) in &targets {
                    if reference.starts_with(segments) {
                        for owner in &owners {
                            picks.push((owner.clone(), *file, line, *rule, module.clone()));
                        }
                    }
                }
            }
        }
        for (owner, file, line, rule, target) in picks {
            self.pick(&owner, file, line, rule, &target);
        }
    }

    /// The `ae::…` paths that name library module `module`: the module itself, and every name
    /// `src/lib.rs` or its parent module re-exports from it with a plain `pub use`.
    fn targets(&self, module: &str) -> Vec<(Vec<String>, &'static str)> {
        let segments: Vec<String> = module.split("::").map(str::to_owned).collect();
        let mut found = vec![(ae_path(&[], &segments), "namespace")];
        let parent = module.rsplit_once("::").map_or("", |(parent, _)| parent);
        for scope in BTreeSet::from(["", parent]) {
            let scope_segments: Vec<String> = scope
                .split("::")
                .filter(|segment| !segment.is_empty())
                .map(str::to_owned)
                .collect();
            let declared = ModPath {
                krate: Crate::Lib,
                path: scope.to_owned(),
            };
            for file in self.files_of.get(&declared).into_iter().flatten() {
                let Some(lexed) = self.lexed.get(file.as_str()) else {
                    continue;
                };
                for (_, public, leaves) in uses(lexed) {
                    if !public {
                        continue;
                    }
                    for (path, alias, glob) in leaves {
                        if !resolve_lib(&scope_segments, &path).starts_with(&segments) {
                            continue;
                        }
                        let exported = if glob { Vec::new() } else { vec![alias] };
                        found.push((ae_path(&scope_segments, &exported), "reexport"));
                    }
                }
            }
        }
        found
    }

    /// A changed path no rule placed runs the whole suite, unless it is exempt.
    fn unplaced(&mut self) {
        let input = self.input;
        for change in &input.changes {
            for path in named_paths(change) {
                if self.triggered.contains(path) || self.placed.contains(path) || exempt(path) {
                    continue;
                }
                let gone = matches!(
                    change.status,
                    Status::Deleted | Status::Renamed { .. } | Status::Copied { .. }
                );
                if gone && is_rust(path) {
                    continue;
                }
                self.whole("unknown-path", format!("{path} has no owner and no reader"));
            }
        }
    }

    fn finish(self) -> Selection {
        let why: Vec<Why> = self
            .why
            .into_iter()
            .map(|((owner, file, rule, target), line)| Why {
                owner,
                file,
                line,
                rule,
                target,
            })
            .collect();
        if !self.reasons.is_empty() {
            return Selection {
                verdict: Verdict::Whole,
                reasons: self.reasons.into_iter().collect(),
                owners: BTreeSet::new(),
                why,
                doctests: self.doctests,
                check: self.check,
                not_selected: 0,
                total: 0,
                filterset: String::new(),
            };
        }
        let mut owners = self.owners;
        owners.extend(GUARDS.map(str::to_owned));
        let names = self
            .tree
            .as_ref()
            .map(|tree| modtree::test_names(&self.input.new, &tree.owners))
            .unwrap_or_default();
        let generated = self
            .tree
            .as_ref()
            .map(|tree| modtree::generated_roots(&self.input.new, &tree.owners))
            .unwrap_or_default();
        let not_selected = names
            .iter()
            .filter(|name| !owners.iter().any(|owner| under(name, owner)))
            .count();
        // An owner stays a filter term when a test name is under it, or when a macro body
        // writes tests in it or below it: only an owner with neither is a dead prefix.
        let listed: Vec<&str> = owners
            .iter()
            .filter(|owner| {
                GUARDS.contains(&owner.as_str())
                    || names.iter().any(|name| under(name, owner))
                    || generated
                        .iter()
                        .any(|root| root == *owner || under(root, owner))
            })
            .map(String::as_str)
            .collect();
        let kept: Vec<&str> = listed
            .iter()
            .filter(|owner| !listed.iter().any(|outer| under(owner, outer)))
            .copied()
            .collect();
        Selection {
            verdict: Verdict::Subset,
            reasons: Vec::new(),
            filterset: format!(
                "kind(lib) | kind(bin) | (binary_id(ae::it) & test(/^({})::/))",
                kept.join("|")
            ),
            owners,
            why,
            doctests: self.doctests,
            check: self.check,
            not_selected,
            total: names.len(),
        }
    }
}

/// `ae::<scope>::<rest>` as segments.
fn ae_path(scope: &[String], rest: &[String]) -> Vec<String> {
    std::iter::once("ae".to_owned())
        .chain(scope.iter().cloned())
        .chain(rest.iter().cloned())
        .collect()
}

/// A `use` path written in library module `scope`, as segments from the crate root.
fn resolve_lib(scope: &[String], path: &[String]) -> Vec<String> {
    let mut base = scope.to_vec();
    let mut rest = path;
    match path.first().map(String::as_str) {
        Some("crate") => {
            base.clear();
            rest = &path[1..];
        }
        Some("self" | "super") => {
            while let Some(first) = rest.first() {
                match first.as_str() {
                    "self" => {}
                    "super" => {
                        base.pop();
                    }
                    _ => break,
                }
                rest = &rest[1..];
            }
        }
        _ => {}
    }
    base.extend(rest.iter().cloned());
    base
}

// ---------------------------------------------------------------------------
// Reading a file
// ---------------------------------------------------------------------------

/// What one file names as data.
enum DataRef {
    /// `include!`, `include_str!` or `include_bytes!` of a literal path, resolved from the
    /// file's directory, or of `concat!(env!("CARGO_MANIFEST_DIR"), "/x")`, resolved to `x`.
    Include { line: usize, target: String },
    /// An `include*!` whose argument is neither form.
    Unresolved { line: usize, text: String },
    /// `path = "x"` in an attribute, resolved from the file's directory.
    PathAttr { line: usize, target: String },
    /// Any string literal.
    Literal { line: usize, value: String },
}

fn data_refs(lexed: &Lexed, file: &str) -> Vec<DataRef> {
    let dir = file.rsplit_once('/').map_or("", |(dir, _)| dir);
    let beside = |relative: &str| {
        normalize(&if dir.is_empty() {
            relative.to_owned()
        } else {
            format!("{dir}/{relative}")
        })
    };
    let mut found = Vec::new();
    for at in 0..lexed.tokens.len() {
        let line = lexed.line(lexed.tokens[at].start);
        if let Some(value) = lexed.literal(at) {
            found.push(DataRef::Literal {
                line,
                value: value.to_owned(),
            });
        }
        if matches!(
            lexed.ident(at),
            Some("include" | "include_str" | "include_bytes")
        ) && lexed.is_punct(at + 1, b'!')
            && lexed.is_punct(at + 2, b'(')
            && let Some(close) = lexed.matching(at + 2)
        {
            found.push(match include_argument(lexed, at + 3, close) {
                Some(Argument::Relative(path)) => DataRef::Include {
                    line,
                    target: beside(path),
                },
                Some(Argument::Manifest(path)) => DataRef::Include {
                    line,
                    target: normalize(path),
                },
                None => DataRef::Unresolved {
                    line,
                    text: (at + 3..close)
                        .map(|index| {
                            let token = lexed.tokens[index];
                            &lexed.text[token.start..token.end]
                        })
                        .collect(),
                },
            });
        }
        if lexed.is_ident(at, "path")
            && lexed.is_punct(at + 1, b'=')
            && at > 0
            && (lexed.is_punct(at - 1, b'[') || lexed.is_punct(at - 1, b','))
            && let Some(value) = lexed.literal(at + 2)
        {
            found.push(DataRef::PathAttr {
                line,
                target: beside(value),
            });
        }
    }
    found
}

enum Argument<'t> {
    Relative(&'t str),
    Manifest(&'t str),
}

/// The tokens `start..close` as `"x"` or `concat!(env!("CARGO_MANIFEST_DIR"), "/x")`.
fn include_argument<'t>(lexed: &'t Lexed, start: usize, close: usize) -> Option<Argument<'t>> {
    if start + 1 == close {
        return lexed.literal(start).map(Argument::Relative);
    }
    let shape = lexed.is_ident(start, "concat")
        && lexed.is_punct(start + 1, b'!')
        && lexed.is_punct(start + 2, b'(')
        && lexed.is_ident(start + 3, "env")
        && lexed.is_punct(start + 4, b'!')
        && lexed.is_punct(start + 5, b'(')
        && lexed.literal(start + 6) == Some("CARGO_MANIFEST_DIR")
        && lexed.is_punct(start + 7, b')')
        && lexed.is_punct(start + 8, b',')
        && lexed.matching(start + 2) == Some(close - 1)
        && (start + 10 == close - 1
            || (start + 11 == close - 1 && lexed.is_punct(start + 10, b',')));
    if !shape {
        return None;
    }
    lexed
        .literal(start + 9)
        .map(|path| Argument::Manifest(path.trim_start_matches('/')))
}

/// Every multi-segment path `a::b…` that does not continue another path: its first token and
/// its segments.
fn paths(lexed: &Lexed) -> Vec<(usize, Vec<String>)> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < lexed.tokens.len() {
        if let Some(first) = lexed.ident(at)
            && !(at >= 2 && lexed.is_path_sep(at - 2))
        {
            let mut segments = vec![first.to_owned()];
            let mut next = at + 1;
            while lexed.is_path_sep(next) {
                let Some(segment) = lexed.ident(next + 2) else {
                    break;
                };
                segments.push(segment.to_owned());
                next += 3;
            }
            if segments.len() > 1 {
                found.push((at, segments));
                at = next;
                continue;
            }
        }
        at += 1;
    }
    found
}

/// A `use` statement's leaves: (path, alias, glob).
type Leaves = Vec<(Vec<String>, String, bool)>;

/// Every `use` statement: its `use` token, whether it is a plain `pub use`, and its leaves.
fn uses(lexed: &Lexed) -> Vec<(usize, bool, Leaves)> {
    let mut found = Vec::new();
    for at in 0..lexed.tokens.len() {
        if !lexed.is_ident(at, "use") {
            continue;
        }
        let Some(end) = (at + 1..lexed.tokens.len()).find(|end| lexed.is_punct(*end, b';')) else {
            continue;
        };
        let mut leaves = Vec::new();
        modtree::use_tree(lexed, at + 1, end, &[], &mut leaves);
        found.push((at, at > 0 && lexed.is_ident(at - 1, "pub"), leaves));
    }
    found
}

/// Every `ae::…` path and `use ae::…` leaf: its line and its segments.
fn ae_refs(lexed: &Lexed) -> Vec<(usize, Vec<String>)> {
    let line = |at: usize| lexed.line(lexed.tokens[at].start);
    let is_ae = |segments: &[String]| segments.first().is_some_and(|first| first == "ae");
    let mut found: Vec<(usize, Vec<String>)> = paths(lexed)
        .into_iter()
        .filter(|(_, segments)| is_ae(segments))
        .map(|(at, segments)| (line(at), segments))
        .collect();
    for (at, _, leaves) in uses(lexed) {
        for (path, _, _) in leaves {
            if is_ae(&path) {
                found.push((line(at), path));
            }
        }
    }
    found
}

/// The integration module paths a file's text references from each of its owners `contexts`:
/// `crate::…`, `self::…` and `super::…` exactly, and any other path from the module and from
/// each of its ancestors, which a `use super::*` reaches. Paths and `use` leaves alike.
fn module_refs(lexed: &Lexed, contexts: &[String]) -> BTreeSet<String> {
    let spans = lexed.inline_modules();
    let mut found = BTreeSet::new();
    let leaves: Vec<(usize, Vec<String>)> = uses(lexed)
        .into_iter()
        .flat_map(|(at, _, leaves)| leaves.into_iter().map(move |(path, _, _)| (at, path)))
        .collect();
    for (at, segments) in paths(lexed).into_iter().chain(leaves) {
        let inline = Lexed::inline_path(&spans, at);
        for context in contexts {
            found.extend(resolve(&join_path(context, &inline), &segments));
        }
    }
    found
}

/// The module paths `segments` can name from integration module `here`.
fn resolve(here: &str, segments: &[String]) -> Vec<String> {
    let mut base: Vec<&str> = here.split("::").filter(|part| !part.is_empty()).collect();
    let joined = |base: &[&str], rest: &[String]| {
        base.iter()
            .copied()
            .chain(rest.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("::")
    };
    match segments.first().map(String::as_str) {
        None | Some("ae" | "std" | "core" | "alloc" | "Self") => Vec::new(),
        Some("crate") => vec![joined(&[], &segments[1..])],
        Some("self" | "super") => {
            let mut rest = segments;
            while let Some(first) = rest.first() {
                match first.as_str() {
                    "self" => {}
                    "super" => {
                        base.pop();
                    }
                    _ => break,
                }
                rest = &rest[1..];
            }
            vec![joined(&base, rest)]
        }
        Some(_) => (0..=base.len())
            .map(|depth| joined(&base[..depth], segments))
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

/// One item of a file, for the macro, crate-root and shared-rig comparisons.
#[derive(Clone, Debug)]
struct Item {
    /// The inline-module prefix and the token text: the item's identity in a comparison.
    text: String,
    /// `fn`, `use`, `mod`, `macro_rules`, `struct`, …: the first word after the visibility.
    kind: String,
    /// A fn with an exact `#[test]` attribute.
    test: bool,
    /// For `mod name;`: the declared module under the item's inline modules.
    module: Option<String>,
}

/// The items only `before` has and the items only `after` has, compared by text as multisets.
fn item_diff(before: &str, after: &str) -> (Vec<Item>, Vec<Item>) {
    let mut balance: BTreeMap<String, (isize, Item)> = BTreeMap::new();
    for item in items(before) {
        balance.entry(item.text.clone()).or_insert((0, item)).0 -= 1;
    }
    for item in items(after) {
        balance.entry(item.text.clone()).or_insert((0, item)).0 += 1;
    }
    let mut removed = Vec::new();
    let mut added = Vec::new();
    for (count, item) in balance.into_values() {
        match count.signum() {
            -1 => removed.push(item),
            1 => added.push(item),
            _ => {}
        }
    }
    (removed, added)
}

/// The items of a file: the top-level ones and those of inline `mod { }` blocks, whose header
/// is an item of its own.
fn items(text: &str) -> Vec<Item> {
    let lexed = modtree::lex(text);
    let mut found = Vec::new();
    split_items(&lexed, 0, lexed.tokens.len(), "", &mut found);
    found
}

fn split_items(lexed: &Lexed, start: usize, end: usize, prefix: &str, found: &mut Vec<Item>) {
    let mut at = start;
    while at < end {
        let first = at;
        if lexed.is_punct(at, b'#') && lexed.is_punct(at + 1, b'!') {
            let last = lexed
                .attribute_end(at)
                .filter(|close| *close < end)
                .unwrap_or(end - 1);
            found.push(Item {
                text: format!("{prefix}|{}", token_text(lexed, first, last)),
                kind: "#!".to_owned(),
                test: false,
                module: None,
            });
            at = last + 1;
            continue;
        }
        let mut test = false;
        while lexed.is_punct(at, b'#') && lexed.is_punct(at + 1, b'[') {
            let Some(close) = lexed.attribute_end(at).filter(|close| *close < end) else {
                break;
            };
            test |= close == at + 3 && lexed.is_ident(at + 2, "test");
            at = close + 1;
        }
        let head = if lexed.is_ident(at, "pub") {
            lexed.skip_visibility(at)
        } else {
            at
        };
        let kind = item_kind(lexed, head);
        if kind == "mod"
            && let Some(name) = lexed.ident(head + 1)
            && lexed.is_punct(head + 2, b'{')
            && let Some(close) = lexed.matching(head + 2).filter(|close| *close < end)
        {
            found.push(Item {
                text: format!("{prefix}|{}", token_text(lexed, first, head + 2)),
                kind: "mod-header".to_owned(),
                test: false,
                module: None,
            });
            split_items(lexed, head + 3, close, &join_path(prefix, name), found);
            at = close + 1;
            continue;
        }
        let last = item_last(lexed, head, end, &kind).max(first);
        let module = (kind == "mod" && lexed.is_punct(head + 2, b';'))
            .then(|| lexed.ident(head + 1))
            .flatten()
            .map(|name| join_path(prefix, name));
        found.push(Item {
            text: format!("{prefix}|{}", token_text(lexed, first, last)),
            test: test && kind == "fn",
            kind,
            module,
        });
        at = last + 1;
    }
}

/// The item's kind word at `head`, past `async`, `const`, `unsafe`, `extern "…"` and `default`
/// when a `fn` follows them.
fn item_kind(lexed: &Lexed, head: usize) -> String {
    let mut probe = head;
    while matches!(
        lexed.ident(probe),
        Some("async" | "const" | "unsafe" | "extern" | "default")
    ) || (probe > head && lexed.literal(probe).is_some())
    {
        probe += 1;
    }
    if lexed.is_ident(probe, "fn") {
        return "fn".to_owned();
    }
    lexed.ident(head).unwrap_or_default().to_owned()
}

/// The last token of the item whose kind word is at `head`: its `;` at depth zero, or the `}`
/// that closes its body (never for `use`, `const`, `static` and `type`, which end at `;`).
fn item_last(lexed: &Lexed, head: usize, end: usize, kind: &str) -> usize {
    let to_semicolon = matches!(kind, "use" | "const" | "static" | "type");
    let mut depth = 0usize;
    for at in head..end {
        match lexed.tokens[at].kind {
            Kind::Punct(b'(' | b'[' | b'{') => depth += 1,
            Kind::Punct(b')' | b']') => depth = depth.saturating_sub(1),
            Kind::Punct(b'}') => {
                depth = depth.saturating_sub(1);
                if depth == 0 && !to_semicolon {
                    return at;
                }
            }
            Kind::Punct(b';') if depth == 0 => return at,
            _ => {}
        }
    }
    end.saturating_sub(1)
}

/// The tokens `first..=last` joined by single spaces: whitespace and comments do not count.
fn token_text(lexed: &Lexed, first: usize, last: usize) -> String {
    lexed.tokens[first..=last]
        .iter()
        .map(|token| &lexed.text[token.start..token.end])
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// The protocol
// ---------------------------------------------------------------------------

/// The selection as the recipe reads it, one fact per line:
///
/// ```text
/// version 1
/// verdict whole|subset
/// reason <id> <detail>      zero or more, sorted; at least one when whole
/// owner <path>              subset only, sorted
/// why <owner> <file>:<line> <rule> <target>    sorted
/// doctests yes|no
/// check yes|no
/// counter <N> <M>           subset only: N of M statically enumerated integration tests not selected
/// filterset <filterset>     subset only
/// end
/// ```
pub(crate) fn render(selection: &Selection) -> String {
    let subset = selection.verdict == Verdict::Subset;
    let yes = |flag: bool| if flag { "yes" } else { "no" };
    let mut out = String::from("version 1\n");
    out.push_str(if subset {
        "verdict subset\n"
    } else {
        "verdict whole\n"
    });
    let mut reasons = selection.reasons.clone();
    reasons.sort();
    reasons.dedup();
    for reason in &reasons {
        let _ = writeln!(out, "reason {} {}", reason.id, clean(&reason.detail));
    }
    if subset {
        for owner in &selection.owners {
            let _ = writeln!(out, "owner {}", clean(owner));
        }
    }
    let mut why = selection.why.clone();
    why.sort();
    why.dedup();
    for entry in &why {
        let _ = writeln!(
            out,
            "why {} {}:{} {} {}",
            clean(&entry.owner),
            clean(&entry.file),
            entry.line,
            entry.rule,
            clean(&entry.target)
        );
    }
    let _ = writeln!(out, "doctests {}", yes(selection.doctests));
    let _ = writeln!(out, "check {}", yes(selection.check));
    if subset {
        let _ = writeln!(
            out,
            "counter {} {}",
            selection.not_selected, selection.total
        );
        let _ = writeln!(out, "filterset {}", clean(&selection.filterset));
    }
    out.push_str("end\n");
    out
}

#[cfg(test)]
#[path = "affected/affected_spec.rs"]
mod affected_spec;

#[cfg(test)]
mod respond {
    use super::respond;

    /// A stream the selector cannot decode still answers: the whole suite, reason `stream`, and
    /// the install checks, because nothing says `install` did not change.
    #[test]
    fn a_malformed_stream_runs_the_whole_suite() {
        for stream in [&b""[..], b"version 1\n1\n", b"version 1\n2\nend 1\n1\n"] {
            let answer = respond(stream);
            let lines: Vec<&str> = answer.lines().collect();
            assert_eq!(lines[..2], ["version 1", "verdict whole"], "{answer}");
            assert!(lines[2].starts_with("reason stream "), "{answer}");
            assert_eq!(lines[3..], ["doctests no", "check yes", "end"], "{answer}");
        }
    }
}

#[cfg(test)]
mod generated {
    use super::{Change, Files, Input, Mode, Status, Verdict, select};

    const GENERATED: &str = "macro_rules! both { ($name:ident) => { #[test] fn $name() {} }; } \
                             both!(first); const VALUE: u8 = 1;";

    /// A module whose every test a `macro_rules!` body writes (`fn $name`) has no test name the
    /// walk can read, yet it has tests: a change to it keeps it in the filterset.
    #[test]
    fn an_owner_whose_tests_a_macro_writes_stays_in_the_filterset() {
        let new: Files = [
            ("src/lib.rs", "pub mod changed;"),
            ("src/main.rs", "fn main() {}"),
            ("src/changed.rs", "pub fn value() -> u8 { 1 }"),
            (
                "tests/it/main.rs",
                "mod doors; mod floor; mod gate; mod generated;",
            ),
            ("tests/it/doors.rs", "#[test] fn door_guard() {}"),
            ("tests/it/floor.rs", "#[test] fn floor_guard() {}"),
            ("tests/it/gate.rs", "#[test] fn gate_guard() {}"),
            ("tests/it/generated.rs", GENERATED),
        ]
        .into_iter()
        .map(|(path, text)| (path.to_owned(), text.to_owned()))
        .collect();
        let path = "tests/it/generated.rs".to_owned();
        let input = Input {
            mode: Mode::Gate,
            base: "main".to_owned(),
            merge_base: "a".repeat(40),
            head: "b".repeat(40),
            changes: vec![Change {
                status: Status::Modified,
                path: path.clone(),
            }],
            tracked: new.keys().cloned().collect(),
            new,
            old: Files::from([(path, GENERATED.replace("u8 = 1", "u8 = 0"))]),
        };
        let got = select(&input);
        assert!(matches!(got.verdict, Verdict::Subset), "expected a subset");
        assert!(got.owners.contains("generated"));
        assert_eq!(
            got.filterset,
            "kind(lib) | kind(bin) | (binary_id(ae::it) & test(/^(doors|floor|gate|generated)::/))"
        );
    }
}
