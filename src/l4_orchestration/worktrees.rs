//! Worktree cleanup: the ONE classifier `ae worktree gc` and the retire guard
//! share, so the two surfaces cannot disagree about whether a tree may go.
//!
//! Git facts come through [`crate::git`], filesystem facts through existing
//! doors: [`crate::teardown::classify_dir`] (an lstat, because `git worktree
//! remove` resolves a symlink), [`crate::doors::canonical_strict_dir`],
//! [`crate::doors::cwd`] and the durable scan. The only removal is
//! [`crate::git::worktree_remove_plain`] (no `--force`). gc reaches it only for
//! a candidate whose FRESH verdict is `remove` and whose identity equals the
//! one the confirmed token bound. Nothing here prunes, touches a branch or
//! forces anything; a removed worktree's branch stays.
//!
//! Every rendered path, branch, owner and git-derived word passes
//! [`crate::event_text::sanitize_menu_text`]; the token is computed over the
//! raw values.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Component, Path, PathBuf};

use crate::doors::StrictDirError;
use crate::git::{Cleanliness, WorktreeEntry};
use crate::inventory::{DurableRecord, FailedSource, MetaRead};
use crate::meta::{Anomaly, Meta, RecordedWorkDir};
use crate::teardown::DirKind;

/// The public usage line.
pub const USAGE: &str = "Usage: ae worktree gc [--idle <days>] [--remove <token>]";

/// Whole idle days after which a clean, merged, unowned worktree may go.
pub const DEFAULT_IDLE_DAYS: u64 = 2;

const DAY_SECS: i64 = 86_400;

/// The token payload's version tag: a change of payload changes the tag.
const TOKEN_TAG: &[u8] = b"ae-worktree-gc-v1";

/// A parsed `ae worktree gc` command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcArgs {
    /// `--idle <days>`: at least 1.
    pub idle: u64,
    /// `--remove <token>`: the confirmation, or `None` for the dry run.
    pub remove: Option<String>,
}

/// Parse the words after `ae worktree`.
///
/// # Errors
///
/// The usage complaint for anything but `gc` with valid flags: a missing or
/// unknown sub-command, an unknown or repeated flag, a flag without its
/// value, or an `--idle` that is not a whole number of at least 1.
pub fn parse_args(tail: &[String]) -> Result<GcArgs, String> {
    let Some((verb, flags)) = tail.split_first() else {
        return Err("ae worktree: missing sub-command".to_owned());
    };
    if verb != "gc" {
        return Err(format!(
            "ae worktree: unknown sub-command '{}'",
            clean(verb)
        ));
    }
    let mut idle = None;
    let mut remove = None;
    let mut words = flags.iter();
    while let Some(flag) = words.next() {
        match (flag.as_str(), words.next()) {
            ("--idle", Some(days)) if idle.is_none() => idle = Some(parse_days(days)?),
            ("--remove", Some(token)) if remove.is_none() && !token.is_empty() => {
                remove = Some(token.clone());
            }
            ("--idle" | "--remove", None) => {
                return Err(format!("ae worktree gc: {flag} needs a value"));
            }
            ("--idle" | "--remove", Some(_)) => {
                return Err(format!("ae worktree gc: {flag} is repeated or empty"));
            }
            _ => {
                return Err(format!(
                    "ae worktree gc: unknown argument '{}'",
                    clean(flag)
                ));
            }
        }
    }
    Ok(GcArgs {
        idle: idle.unwrap_or(DEFAULT_IDLE_DAYS),
        remove,
    })
}

/// A whole number of days, at least 1.
fn parse_days(value: &str) -> Result<u64, String> {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| value.parse::<u64>().ok())
        .flatten()
        .filter(|days| *days >= 1)
        .ok_or_else(|| {
            format!(
                "ae worktree gc: --idle needs a whole number of days, at least 1, not '{}'",
                clean(value)
            )
        })
}

/// The porcelain-only keep reason, in verdict precedence: `bare`, `missing`
/// (git's `prunable`), `locked`, `unknown` — or `None` when the listing alone
/// does not keep the tree. An `unknown` entry never passes.
#[must_use]
pub fn porcelain_keep(entry: &WorktreeEntry) -> Option<&'static str> {
    if entry.bare {
        Some("bare")
    } else if entry.prunable {
        Some("missing")
    } else if entry.locked {
        Some("locked")
    } else if entry.unknown {
        Some("unknown")
    } else {
        None
    }
}

/// `ae worktree gc`: the dry-run listing, or the confirmed removal of exactly
/// the set the token bound.
///
/// # Errors
///
/// Only a failure to write `out` or `err`.
pub fn run(
    root: &Path,
    args: &GcArgs,
    now: i64,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let context = match Context::load(root, args.idle, now) {
        Ok(context) => context,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(crate::entry::EXIT_FAILED);
        }
    };
    let rows = context.rows();
    let token = context.token(&rows);
    let Some(given) = args.remove.as_deref() else {
        context.render(&rows, &token, out)?;
        return Ok(0);
    };
    if given != token {
        writeln!(
            err,
            "Error: the removable set changed since that listing; nothing was removed."
        )?;
        context.render(&rows, &token, out)?;
        return Ok(crate::entry::EXIT_FAILED);
    }
    let mut all_removed = true;
    for bound in context.candidates(&rows) {
        all_removed &= remove_one(root, args.idle, now, &bound, out)?;
    }
    Ok(if all_removed {
        0
    } else {
        crate::entry::EXIT_FAILED
    })
}

/// Re-read everything, revalidate ONE bound candidate, remove it with plain
/// `git worktree remove`, and report what the filesystem and git show
/// afterwards. True only for a proven removal.
fn remove_one(
    root: &Path,
    idle: u64,
    now: i64,
    bound: &Identity,
    out: &mut impl Write,
) -> io::Result<bool> {
    let shown = clean(&bound.path);
    let Ok(fresh) = Context::load(root, idle, now) else {
        writeln!(
            out,
            "skipped {shown}: the worktrees could not be listed again"
        )?;
        return Ok(false);
    };
    let Some(entry) = fresh
        .linked
        .iter()
        .find(|entry| entry.path == bound.path && fresh.identity(entry) == *bound)
    else {
        writeln!(out, "skipped {shown}: identity changed")?;
        return Ok(false);
    };
    let row = fresh.classify(entry);
    if let Some(reason) = &row.keep {
        writeln!(out, "skipped {shown}: keep: {}", clean(reason))?;
        return Ok(false);
    }
    let path = Path::new(&bound.path);
    if !matches!(crate::teardown::classify_dir(path), DirKind::RealDir) {
        writeln!(out, "skipped {shown}: keep: not a real dir")?;
        return Ok(false);
    }
    if !still_bound(bound) {
        writeln!(out, "skipped {shown}: identity changed")?;
        return Ok(false);
    }
    let exited = crate::git::worktree_remove_plain(bound.root.as_bytes(), bound.path.as_bytes());
    let kind = crate::teardown::classify_dir(path);
    let registered = registration(&bound.root, &bound.path);
    match (exited, &kind, registered) {
        (true, DirKind::Absent, Some(false)) => {
            let branch = match bound.branch.as_str() {
                "-" => "detached HEAD, no branch".to_owned(),
                branch => format!("branch {} kept", clean(short_branch(branch))),
            };
            writeln!(out, "removed {shown} ({branch})")?;
            Ok(true)
        }
        (false, DirKind::RealDir, Some(true)) => {
            writeln!(out, "git refused {shown}; it is kept")?;
            Ok(false)
        }
        _ => {
            let registered = match registered {
                Some(true) => "yes",
                Some(false) => "no",
                None => "unknown",
            };
            writeln!(
                out,
                "{shown}: state unknown after git worktree remove (dir {}, registered {registered}); inspect it by hand",
                kind_word(&kind)
            )?;
            Ok(false)
        }
    }
}

/// Whether git, asked from `root`, still registers `path`: `None` unless the
/// listing is whole — every record terminated, the main worktree at `root`
/// first, no entry `unknown` — so an empty, cut or damaged listing never
/// proves the registration gone.
fn registration(root: &str, path: &str) -> Option<bool> {
    let listed = crate::git::worktree_list(root.as_bytes())?;
    let entries = crate::git::parse_worktree_porcelain(&listed);
    let whole = listed.ends_with("\n\n")
        && entries.first().is_some_and(|main| main.path == root)
        && entries.iter().all(|entry| !entry.unknown);
    whole.then(|| entries.iter().any(|entry| entry.path == path))
}

/// The lstat answer in words.
const fn kind_word(kind: &DirKind) -> &'static str {
    match kind {
        DirKind::RealDir => "present",
        DirKind::Symlink => "a symlink",
        DirKind::NotDir => "not a dir",
        DirKind::Absent => "absent",
        DirKind::StatErr(_) => "unreadable",
    }
}

/// The branch the main worktree has checked out: what every candidate must be
/// merged into.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    /// The full ref name.
    reference: String,
    /// Its 40-hex commit.
    sha: String,
}

/// The target of a listing, or `None` when the main worktree is detached,
/// bare or unknown.
fn target_of(main: &WorktreeEntry) -> Option<Target> {
    if main.unknown || main.bare || main.detached {
        return None;
    }
    Some(Target {
        reference: main.branch.clone()?,
        sha: main.head.clone()?,
    })
}

/// `entry`'s identity under the main worktree `main` and its `target`.
fn identity_of(main: &WorktreeEntry, target: Option<&Target>, entry: &WorktreeEntry) -> Identity {
    Identity {
        root: main.path.clone(),
        target_ref: target
            .map(|target| target.reference.clone())
            .unwrap_or_default(),
        target_sha: target.map(|target| target.sha.clone()).unwrap_or_default(),
        path: entry.path.clone(),
        head: entry.head.clone().unwrap_or_default(),
        branch: entry.branch.clone().unwrap_or_else(|| "-".to_owned()),
    }
}

/// Whether git, asked once more from the bound repo root, still lists the
/// bound candidate with the bound identity: path, HEAD, branch, and the main
/// worktree's target ref and sha. The last read before the removal.
fn still_bound(bound: &Identity) -> bool {
    let Some(listed) = crate::git::worktree_list(bound.root.as_bytes()) else {
        return false;
    };
    let mut entries = crate::git::parse_worktree_porcelain(&listed);
    if entries.is_empty() {
        return false;
    }
    let main = entries.remove(0);
    let target = target_of(&main);
    let mut matching = entries.iter().filter(|entry| entry.path == bound.path);
    match (matching.next(), matching.next()) {
        (Some(entry), None) => {
            !entry.unknown && identity_of(&main, target.as_ref(), entry) == *bound
        }
        _ => false,
    }
}

/// `refs/heads/<name>` as `<name>`; any other ref as written.
fn short_branch(reference: &str) -> &str {
    reference.strip_prefix("refs/heads/").unwrap_or(reference)
}

/// Whether `path` is the worktree at `raw` (or at its canonical spelling) or
/// lies below it, compared component-wise.
fn at_or_below(path: &Path, raw: &Path, canonical: Option<&Path>) -> bool {
    path.starts_with(raw) || canonical.is_some_and(|root| path.starts_with(root))
}

/// Whether `path` is spelled the way a canonical path is: absolute, with
/// only plain names after the root — no `.`, no `..`, no doubled or trailing
/// `/`. Only such a spelling may stand in for a directory that is gone.
fn canonical_spelling(path: &Path) -> bool {
    let mut rebuilt = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Normal(_) => rebuilt.push(component),
            Component::Prefix(_) | Component::CurDir | Component::ParentDir => return false,
        }
    }
    path.is_absolute() && rebuilt.as_os_str() == path.as_os_str()
}

/// A recorded path's spellings: as recorded when that is spelled
/// canonically, plus its canonical form while it still resolves. Empty for a
/// non-canonical spelling of a directory that is gone.
fn spellings(recorded: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if canonical_spelling(recorded) {
        found.push(recorded.to_path_buf());
    }
    if let Ok(canonical) = crate::doors::canonical_strict_dir(recorded) {
        found.push(canonical);
    }
    found
}

/// Who may still need a worktree: every durable session record, live or
/// stopped.
#[derive(Debug, Default)]
struct Owners {
    /// Paths that keep any worktree holding them, each with its owner label.
    dirs: Vec<(PathBuf, String)>,
    /// Seat names that keep a worktree whose branch or leaf they match.
    names: Vec<(String, String)>,
    /// The first reason the owner facts are incomplete: then nothing goes.
    gap: Option<String>,
}

impl Owners {
    /// Read both durable layouts under `root`.
    fn scan(root: &Path) -> Self {
        let scan = crate::inventory::durable_meta_records(&crate::inventory::Roots::under(root));
        let mut owners = Self::default();
        for failed in &scan.incomplete {
            owners.note_gap(format!("{} could not be listed", failed_source(failed)));
        }
        for record in &scan.records {
            owners.add_record(record);
        }
        owners
    }

    fn note_gap(&mut self, why: String) {
        if self.gap.is_none() {
            self.gap = Some(why);
        }
    }

    fn session_gap(&mut self, session: &str, why: &str) {
        self.note_gap(format!("session '{session}': {why}"));
    }

    /// One record: its state dir, its session work dir, every seat's name and
    /// recorded work dir.
    fn add_record(&mut self, record: &DurableRecord) {
        let session = record.name.as_str();
        self.protect(&record.path, session, session, true);
        match record.meta_read {
            MetaRead::Parsed => {}
            MetaRead::Absent => {
                self.session_gap(session, "its meta is missing");
                return;
            }
            MetaRead::Unreadable => {
                self.session_gap(session, "its meta could not be read");
                return;
            }
        }
        let Some(meta) = record.snapshot.meta.as_ref() else {
            return;
        };
        if !crate::session::roster_complete(meta) {
            self.session_gap(session, "its roster is incomplete");
        }
        self.add_session_dir(meta, session);
        for seat in meta.roster() {
            let label = format!("{session}:{}", seat.name);
            self.names.push((seat.name.clone(), label.clone()));
            match &seat.work_dir {
                RecordedWorkDir::Missing => {}
                RecordedWorkDir::Path(path) => self.protect(path, &label, session, false),
                RecordedWorkDir::Invalid(why) => self.session_gap(
                    session,
                    &format!(
                        "the work_dir row of seat '{}' is unusable ({why})",
                        seat.name
                    ),
                ),
            }
        }
    }

    /// The session `work_dir` row: one canonical absolute path, or a gap.
    fn add_session_dir(&mut self, meta: &Meta, session: &str) {
        let duplicated = meta.anomalies().iter().any(
            |anomaly| matches!(anomaly, Anomaly::DuplicateKey { key, .. } if key == "work_dir"),
        );
        if duplicated {
            self.session_gap(session, "its work_dir row is named more than once");
            return;
        }
        match meta.work_dir().map(RecordedWorkDir::parse) {
            None | Some(RecordedWorkDir::Missing) => {}
            Some(RecordedWorkDir::Path(path)) => self.protect(&path, session, session, false),
            Some(RecordedWorkDir::Invalid(why)) => {
                self.session_gap(session, &format!("its work_dir row is unusable ({why})"));
            }
        }
    }

    /// Keep any worktree holding a recorded work dir row only when the row is
    /// exactly canonical (ruling R2a): an existing dir must resolve to its own
    /// spelling, and a dir that is gone (absent, or no longer a directory) must
    /// be spelled canonically and then compares lexically. Any other row — a
    /// `..`, a `.`, a doubled or trailing `/`, a symlink hop — or any other
    /// failure is a gap. The state dir (`required`) protects by both spellings
    /// and must resolve.
    fn protect(&mut self, recorded: &Path, label: &str, session: &str, required: bool) {
        let lexical = canonical_spelling(recorded);
        match crate::doors::canonical_strict_dir(recorded) {
            Ok(canonical) if required => {
                if lexical {
                    self.dirs.push((recorded.to_path_buf(), label.to_owned()));
                }
                self.dirs.push((canonical, label.to_owned()));
            }
            // Byte equality: `Path` equality compares components, which drops
            // a `.`, doubled and trailing separators.
            Ok(canonical) if canonical.as_os_str() == recorded.as_os_str() => {
                self.dirs.push((canonical, label.to_owned()));
            }
            Ok(canonical) => self.session_gap(
                session,
                &format!(
                    "its recorded dir {} is not canonical (it resolves to {})",
                    recorded.display(),
                    canonical.display()
                ),
            ),
            Err(StrictDirError::Absent { .. } | StrictDirError::NotDirectory { .. })
                if !required && lexical =>
            {
                self.dirs.push((recorded.to_path_buf(), label.to_owned()));
            }
            Err(StrictDirError::Absent { .. } | StrictDirError::NotDirectory { .. })
                if !required =>
            {
                self.session_gap(
                    session,
                    &format!(
                        "its recorded dir {} is gone and not spelled canonically",
                        recorded.display()
                    ),
                );
            }
            Err(_) => self.session_gap(
                session,
                &format!("its recorded dir {} could not be read", recorded.display()),
            ),
        }
    }

    /// The owner label keeping the worktree at `raw`, if any: by dir first,
    /// then by seat name against the branch or the path leaf.
    fn owner_of(&self, raw: &Path, canonical: Option<&Path>, branch: Option<&str>) -> Option<&str> {
        if let Some((_, label)) = self
            .dirs
            .iter()
            .find(|(path, _)| at_or_below(path, raw, canonical))
        {
            return Some(label);
        }
        let leaf = raw.file_name().and_then(OsStr::to_str);
        self.names
            .iter()
            .find(|(name, _)| Some(name.as_str()) == branch || Some(name.as_str()) == leaf)
            .map(|(_, label)| label.as_str())
    }
}

/// The durable source a failed enumeration names.
fn failed_source(failed: &FailedSource) -> String {
    match failed {
        FailedSource::CanonicalRoot(path)
        | FailedSource::WorktreeRoot(path)
        | FailedSource::WorktreeState(path) => path.display().to_string(),
        FailedSource::Server(_) => "a tmux server".to_owned(),
    }
}

/// How long ago a worktree's HEAD last moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Idle {
    /// Whole days since the newest entry of its own HEAD reflog.
    Days(i64),
    /// No reflog entry to read.
    Unknown,
    /// Not asked: the directory is not observable.
    NotObserved,
}

/// One candidate's identity: what the token binds and what the per-item check
/// compares, byte for byte.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Identity {
    root: String,
    target_ref: String,
    target_sha: String,
    path: String,
    head: String,
    branch: String,
}

/// One listed worktree, judged.
#[derive(Debug)]
struct Row {
    entry: WorktreeEntry,
    merged: Option<bool>,
    idle: Idle,
    state: Option<Cleanliness>,
    owner: Option<String>,
    /// Why the tree is kept, or `None` for `remove`.
    keep: Option<String>,
}

/// Everything one listing judges against, read once.
struct Context {
    /// The caller's working directory as spelled.
    cwd: PathBuf,
    /// Its canonical form, or `None` when it does not resolve.
    caller: Option<PathBuf>,
    main: WorktreeEntry,
    linked: Vec<WorktreeEntry>,
    target: Option<Target>,
    owners: Owners,
    idle: u64,
    now: i64,
}

impl Context {
    /// The repo holding the caller's cwd, its target and every owner fact.
    fn load(root: &Path, idle: u64, now: i64) -> Result<Self, String> {
        let cwd = crate::doors::cwd();
        let mut entries = crate::git::worktree_list(cwd.as_os_str().as_bytes())
            .map(|listed| crate::git::parse_worktree_porcelain(&listed))
            .unwrap_or_default();
        if entries.is_empty() {
            return Err(format!(
                "Error: git listed no worktrees for {} (not a work tree, or git failed)",
                clean(&cwd.display().to_string())
            ));
        }
        let main = entries.remove(0);
        Ok(Self {
            caller: crate::doors::canonical_strict_dir(&cwd).ok(),
            cwd,
            target: target_of(&main),
            main,
            linked: entries,
            owners: Owners::scan(root),
            idle,
            now,
        })
    }

    fn rows(&self) -> Vec<Row> {
        self.linked
            .iter()
            .map(|entry| self.classify(entry))
            .collect()
    }

    /// The cheap facts for every observable row, the verdict, and the full
    /// proof only for a row every earlier step let through.
    fn classify(&self, entry: &WorktreeEntry) -> Row {
        let raw = Path::new(&entry.path);
        let kind = crate::teardown::classify_dir(raw);
        let canonical = matches!(kind, DirKind::RealDir)
            .then(|| crate::doors::canonical_strict_dir(raw).ok())
            .flatten();
        let observable = canonical.is_some() && !entry.unknown && !entry.bare;
        let bytes = entry.path.as_bytes();
        let merged = match (&self.target, entry.head.as_deref()) {
            (Some(target), Some(head)) if observable => Some(crate::git::is_ancestor(
                self.main.path.as_bytes(),
                head,
                &target.sha,
            )),
            _ => None,
        };
        let idle = if observable {
            crate::git::head_reflog_newest(bytes).map_or(Idle::Unknown, |at| {
                Idle::Days(self.now.saturating_sub(at).max(0) / DAY_SECS)
            })
        } else {
            Idle::NotObserved
        };
        let owner = self
            .owners
            .owner_of(
                raw,
                canonical.as_deref(),
                entry.branch.as_deref().map(short_branch),
            )
            .map(str::to_owned);
        let mut row = Row {
            entry: entry.clone(),
            merged,
            idle,
            state: observable.then(|| crate::git::cheap_cleanliness(bytes)),
            owner,
            keep: None,
        };
        row.keep = self.keep_reason(&row, &kind, canonical.as_deref());
        if row.keep.is_none() {
            let proof =
                crate::git::full_cleanliness(bytes, entry.head.as_deref().unwrap_or_default());
            if proof != (Cleanliness::Clean { proven: true }) {
                row.keep = Some(state_word(&proof));
            }
            row.state = Some(proof);
        }
        row
    }

    /// The first keep reason in verdict precedence, or `None` when only the
    /// full clean proof stands between the row and `remove`.
    fn keep_reason(&self, row: &Row, kind: &DirKind, canonical: Option<&Path>) -> Option<String> {
        let keep = |reason: &str| Some(reason.to_owned());
        match porcelain_keep(&row.entry) {
            Some(reason @ "bare") => return keep(reason),
            _ if matches!(kind, DirKind::Absent) => return keep("missing"),
            Some(reason) => return keep(reason),
            None => {}
        }
        let Some(canonical) = canonical else {
            return keep("not a real dir");
        };
        let raw = Path::new(&row.entry.path);
        let Some(caller) = &self.caller else {
            return keep("caller unknown");
        };
        if at_or_below(&self.cwd, raw, Some(canonical)) || at_or_below(caller, raw, Some(canonical))
        {
            return keep("current dir");
        }
        if row.owner.is_some() {
            return keep("owner");
        }
        if self.owners.gap.is_some() {
            return keep("owners unknown");
        }
        if self.target.is_none() {
            return keep("no target");
        }
        if row.merged != Some(true) {
            return keep("unmerged");
        }
        match row.idle {
            Idle::Days(days) if u64::try_from(days).is_ok_and(|days| days >= self.idle) => {}
            Idle::Days(_) => return keep("recent"),
            Idle::Unknown | Idle::NotObserved => return keep("idle unknown"),
        }
        match &row.state {
            Some(Cleanliness::Clean { proven: false }) => None,
            Some(other) => Some(state_word(other)),
            None => keep("clean unknown"),
        }
    }

    fn identity(&self, entry: &WorktreeEntry) -> Identity {
        identity_of(&self.main, self.target.as_ref(), entry)
    }

    /// The removable rows' identities, sorted by raw path bytes: the token's
    /// order and the removal order.
    fn candidates(&self, rows: &[Row]) -> Vec<Identity> {
        let mut found: Vec<Identity> = rows
            .iter()
            .filter(|row| row.keep.is_none())
            .map(|row| self.identity(&row.entry))
            .collect();
        found.sort_by(|left, right| left.path.cmp(&right.path));
        found
    }

    /// 16 lowercase hex of FNV-1a 64 over the version tag, the repo root, the
    /// target ref and sha, the idle value, then each candidate's path, HEAD
    /// and branch — every field length-prefixed, raw bytes.
    fn token(&self, rows: &[Row]) -> String {
        let mut payload = TOKEN_TAG.to_vec();
        let idle = self.idle.to_string();
        let target = self.identity(&self.main);
        for field in [
            target.root.as_str(),
            target.target_ref.as_str(),
            target.target_sha.as_str(),
            idle.as_str(),
        ] {
            push_field(&mut payload, field.as_bytes());
        }
        for candidate in self.candidates(rows) {
            for field in [&candidate.path, &candidate.head, &candidate.branch] {
                push_field(&mut payload, field.as_bytes());
            }
        }
        format!("{:016x}", fnv1a(&payload))
    }

    /// The listing: header, one row per linked worktree with the path last,
    /// then the footer with the count and the confirmation command.
    fn render(&self, rows: &[Row], token: &str, out: &mut impl Write) -> io::Result<()> {
        let unit = if self.idle == 1 { "day" } else { "days" };
        match &self.target {
            Some(target) => writeln!(
                out,
                "Worktrees of {}: target {} @ {}, removable after {} idle {unit}.",
                clean(&self.main.path),
                clean(short_branch(&target.reference)),
                target.sha.get(..8).unwrap_or(&target.sha),
                self.idle
            )?,
            None => writeln!(
                out,
                "Worktrees of {}: no target (the main worktree is detached, bare or unreadable), so nothing can go.",
                clean(&self.main.path)
            )?,
        }
        let mut table = vec![
            [
                "VERDICT", "MERGED", "IDLE", "STATE", "BRANCH", "OWNER", "PATH",
            ]
            .map(str::to_owned),
        ];
        table.extend(rows.iter().map(|row| self.cells(row)));
        let mut widths = [0_usize; 6];
        for cells in &table {
            for (width, cell) in widths.iter_mut().zip(cells.iter()) {
                *width = (*width).max(cell.chars().count());
            }
        }
        for cells in &table {
            let mut line = String::new();
            for (cell, width) in cells.iter().zip(widths) {
                let _ = write!(line, "{cell:<width$}  ");
            }
            line.push_str(&cells[6]);
            writeln!(out, "{line}")?;
        }
        if let Some(gap) = &self.owners.gap {
            writeln!(
                out,
                "Owners unknown: {}; every worktree is kept until that record is repaired.",
                clean(gap)
            )?;
        }
        if rows
            .iter()
            .any(|row| row.keep.as_deref() == Some("missing"))
        {
            writeln!(
                out,
                "A missing worktree stays registered until 'git worktree prune'; ae never runs it."
            )?;
        }
        let can_go = rows.iter().filter(|row| row.keep.is_none()).count();
        if can_go == 0 {
            writeln!(out, "0 of {} can go.", rows.len())
        } else {
            writeln!(
                out,
                "{can_go} of {} can go. To remove exactly these: ae worktree gc --idle {} --remove {token}",
                rows.len(),
                self.idle
            )
        }
    }

    /// One row's seven sanitized cells.
    fn cells(&self, row: &Row) -> [String; 7] {
        let verdict = row
            .keep
            .as_ref()
            .map_or_else(|| "remove".to_owned(), |reason| format!("keep: {reason}"));
        let merged = match row.merged {
            Some(true) => "yes",
            Some(false) => "no",
            None => "-",
        };
        let idle = match row.idle {
            Idle::Days(days) => format!("{days}d"),
            Idle::Unknown => "?".to_owned(),
            Idle::NotObserved => "-".to_owned(),
        };
        let state = row
            .state
            .as_ref()
            .map_or_else(|| "-".to_owned(), state_word);
        let branch = match (&row.entry.branch, row.entry.detached) {
            (Some(branch), _) => short_branch(branch).to_owned(),
            (None, true) => "detached".to_owned(),
            (None, false) => "-".to_owned(),
        };
        let owner = match (&row.owner, &self.owners.gap) {
            (Some(owner), _) => owner.clone(),
            (None, Some(_)) => "?".to_owned(),
            (None, None) => "-".to_owned(),
        };
        [
            verdict,
            merged.to_owned(),
            idle,
            state,
            branch,
            owner,
            row.entry.path.clone(),
        ]
        .map(|cell| clean(&cell))
    }
}

/// The STATE word for one clean-check finding.
fn state_word(state: &Cleanliness) -> String {
    match state {
        Cleanliness::Clean { proven: true } => "clean".to_owned(),
        Cleanliness::Clean { proven: false } => "clean?".to_owned(),
        Cleanliness::Dirty => "dirty".to_owned(),
        Cleanliness::DirtyBytes => "dirty (bytes)".to_owned(),
        Cleanliness::IndexFlags => "index-flags".to_owned(),
        Cleanliness::UnsupportedMode => "unsupported mode".to_owned(),
        Cleanliness::Untracked => "untracked".to_owned(),
        Cleanliness::IgnoredData(path) => format!("ignored-data {path}"),
        Cleanliness::Unknown => "clean unknown".to_owned(),
    }
}

/// `<byte length>:<bytes>`.
fn push_field(payload: &mut Vec<u8>, bytes: &[u8]) {
    payload.extend_from_slice(bytes.len().to_string().as_bytes());
    payload.push(b':');
    payload.extend_from_slice(bytes);
}

/// FNV-1a, 64 bit. Not a secret: it binds the confirmation to the exact list
/// the human read.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Terminal-safe text: printable ASCII only.
fn clean(text: &str) -> String {
    crate::event_text::sanitize_menu_text(text)
}

// ---- the retire guard --------------------------------------------------------

/// One owned worktree that blocks a retire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Blocking {
    /// The worktree path as git listed it.
    pub(crate) path: String,
    /// Its branch short name, or `detached`.
    pub(crate) branch: String,
    /// Every failed condition, joined.
    pub(crate) why: String,
}

/// What the retire guard found for one seat.
#[derive(Debug, Default)]
pub(crate) struct RetireCheck {
    /// Owned worktrees that are not proven clean and merged.
    pub(crate) blocking: Vec<Blocking>,
    /// Lines for stderr when the retire goes ahead.
    pub(crate) notes: Vec<String>,
}

impl RetireCheck {
    /// One refusal line per blocking worktree.
    pub(crate) fn refusals(&self, agent: &str) -> Vec<String> {
        let agent = clean(agent);
        self.blocking
            .iter()
            .map(|blocking| {
                format!(
                    "Error: '{agent}' still owns worktree {} (branch {}, {}); nothing was retired. Have '{agent}' merge or clean it, or retire with --keep-worktree to take over its cleanup: retire {agent} --keep-worktree [reason]",
                    clean(&blocking.path),
                    clean(&blocking.branch),
                    clean(&blocking.why),
                )
            })
            .collect()
    }

    /// The audit member a `--keep-worktree` retire records: the blocking
    /// paths, joined by `, `, flattened and capped like a reason.
    pub(crate) fn kept(&self) -> String {
        let joined = self
            .blocking
            .iter()
            .map(|blocking| blocking.path.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        crate::state::summary_of(&joined)
    }
}

/// The directories the retire guard reads for one seat.
struct SeatDirs {
    /// The session work dir's spellings: trees holding it are never the seat's.
    session: Vec<PathBuf>,
    /// The seat's recorded `work_dir.<slot>` spellings: trees holding them are.
    recorded: Vec<PathBuf>,
    /// Where git is asked: the seat's dir while it resolves, else the
    /// session's.
    listing: Option<PathBuf>,
    /// Why the seat's own row cannot be used, when it cannot.
    unusable: Option<String>,
}

impl SeatDirs {
    fn of(meta: &Meta, slot: &str) -> Self {
        let session_dir = match meta.work_dir().map(RecordedWorkDir::parse) {
            Some(RecordedWorkDir::Path(path)) => Some(path),
            _ => None,
        };
        let mut dirs = Self {
            session: session_dir.as_deref().map(spellings).unwrap_or_default(),
            recorded: Vec::new(),
            listing: session_dir,
            unusable: None,
        };
        match meta
            .roster()
            .iter()
            .find(|seat| seat.slot == slot)
            .map(|seat| &seat.work_dir)
        {
            Some(RecordedWorkDir::Path(path)) => {
                dirs.recorded = spellings(path);
                if dirs.recorded.is_empty() {
                    dirs.unusable = Some(format!(
                        "its work_dir row {} is gone and not spelled canonically",
                        path.display()
                    ));
                }
                if let Ok(canonical) = crate::doors::canonical_strict_dir(path) {
                    dirs.listing = Some(canonical);
                }
            }
            Some(RecordedWorkDir::Invalid(why)) => {
                dirs.unusable = Some(format!("its work_dir row is unusable: {why}"));
            }
            Some(RecordedWorkDir::Missing) | None => {}
        }
        dirs
    }
}

/// The retire guard for the seat in `slot` named `agent`: every linked
/// worktree that seat owns — by its recorded `work_dir.<slot>`, or by a
/// branch or path leaf equal to its name — minus the session's own work dir
/// and missing trees, judged independently of any co-owner (ruling R3). Clean and
/// merged leaves a note; anything else blocks, a worktree path that is not a
/// real dir included. Git facts that cannot be established never block: they
/// leave a note instead (ruling R6).
pub(crate) fn retire_check(meta: Option<&Meta>, slot: &str, agent: &str) -> RetireCheck {
    let mut check = RetireCheck::default();
    let shown = clean(agent);
    let unchecked = |what: &str| {
        format!(
            "note: worktrees of '{shown}' were not checked ({}); check them by hand.",
            clean(what)
        )
    };
    let Some(meta) = meta else {
        check
            .notes
            .push(unchecked("the session meta could not be read"));
        return check;
    };
    let SeatDirs {
        session: session_paths,
        recorded,
        listing,
        unusable,
    } = SeatDirs::of(meta, slot);
    if let Some(what) = unusable {
        check.notes.push(unchecked(&format!(
            "{what}; only name ownership was checked"
        )));
    }
    let Some(dir) = listing else {
        check
            .notes
            .push(unchecked("the session has no usable work_dir"));
        return check;
    };
    let mut entries = crate::git::worktree_list(dir.as_os_str().as_bytes())
        .map(|listed| crate::git::parse_worktree_porcelain(&listed))
        .unwrap_or_default();
    if entries.is_empty() {
        check.notes.push(unchecked(&format!(
            "git listed no worktrees for {}",
            dir.display()
        )));
        return check;
    }
    let main = entries.remove(0);
    if main.unknown {
        check.notes.push(unchecked(&format!(
            "git's listing of the main worktree {} is unreadable",
            main.path
        )));
        return check;
    }
    let target = target_of(&main);
    for entry in &entries {
        if entry.unknown {
            check.notes.push(unchecked(&format!(
                "git's listing of {} is unreadable",
                entry.path
            )));
            continue;
        }
        let raw = Path::new(&entry.path);
        let kind = crate::teardown::classify_dir(raw);
        let canonical = matches!(kind, DirKind::RealDir)
            .then(|| crate::doors::canonical_strict_dir(raw).ok())
            .flatten();
        let inside = |path: &PathBuf| at_or_below(path, raw, canonical.as_deref());
        if session_paths.iter().any(inside)
            || entry.bare
            || entry.prunable
            || matches!(kind, DirKind::Absent)
        {
            continue;
        }
        let branch = entry.branch.as_deref().map(short_branch);
        let leaf = raw.file_name().and_then(OsStr::to_str);
        if !(recorded.iter().any(inside) || branch == Some(agent) || leaf == Some(agent)) {
            continue;
        }
        match judge_owned(&main, target.as_ref(), entry, canonical.is_some()) {
            Owned::Blocks(why) => check.blocking.push(Blocking {
                path: entry.path.clone(),
                branch: branch.map_or_else(|| "detached".to_owned(), str::to_owned),
                why,
            }),
            Owned::Unproven(facts) => check.notes.push(unchecked(&format!(
                "git could not prove {} {facts}",
                entry.path
            ))),
            Owned::CleanMerged => check.notes.push(format!(
                "note: '{shown}' left worktree {} (clean, merged); 'ae worktree gc' removes it.",
                clean(&entry.path)
            )),
        }
    }
    check
}

/// The retire guard's judgement of one owned worktree.
enum Owned {
    /// Not clean and merged: every failed condition, joined.
    Blocks(String),
    /// Nothing failed, but git could not prove these facts (`merged`,
    /// `clean`): the R6 note, never a block.
    Unproven(&'static str),
    /// The full clean proof and the merge test both held.
    CleanMerged,
}

/// Judge one owned worktree: locked, a real dir, merged into the target, and
/// the full clean proof. Idle and owners are not the guard's business.
fn judge_owned(
    main: &WorktreeEntry,
    target: Option<&Target>,
    entry: &WorktreeEntry,
    real_dir: bool,
) -> Owned {
    let mut why = Vec::new();
    let (mut merged, mut clean) = (true, true);
    if entry.locked {
        why.push("locked".to_owned());
    }
    if real_dir {
        match (target, entry.head.as_deref()) {
            (Some(target), Some(head)) => {
                match crate::git::merged_into(main.path.as_bytes(), head, &target.sha) {
                    Some(true) => {}
                    Some(false) => why.push("unmerged".to_owned()),
                    None => merged = false,
                }
            }
            _ => why.push("no target".to_owned()),
        }
        match crate::git::full_cleanliness(
            entry.path.as_bytes(),
            entry.head.as_deref().unwrap_or_default(),
        ) {
            Cleanliness::Clean { proven: true } => {}
            Cleanliness::Unknown => clean = false,
            other => why.push(state_word(&other)),
        }
    } else {
        why.push("not a real dir".to_owned());
    }
    if !why.is_empty() {
        return Owned::Blocks(why.join(", "));
    }
    match (merged, clean) {
        (true, true) => Owned::CleanMerged,
        (false, true) => Owned::Unproven("merged"),
        (true, false) => Owned::Unproven("clean"),
        (false, false) => Owned::Unproven("merged or clean"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_IDLE_DAYS, GcArgs, at_or_below, canonical_spelling, fnv1a, parse_args,
        porcelain_keep, push_field, short_branch,
    };
    use crate::git::WorktreeEntry;
    use std::path::Path;

    fn words(args: &[&str]) -> Vec<String> {
        args.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn gc_arguments_take_only_a_positive_idle_and_one_token() {
        assert_eq!(
            parse_args(&words(&["gc"])),
            Ok(GcArgs {
                idle: DEFAULT_IDLE_DAYS,
                remove: None
            })
        );
        assert_eq!(
            parse_args(&words(&["gc", "--remove", "00ff", "--idle", "1"])),
            Ok(GcArgs {
                idle: 1,
                remove: Some("00ff".to_owned())
            })
        );
        for bad in [
            &[][..],
            &["other"],
            &["gc", "--idle"],
            &["gc", "--idle", "0"],
            &["gc", "--idle", "-1"],
            &["gc", "--idle", "+1"],
            &["gc", "--idle", "1.5"],
            &["gc", "--idle", ""],
            &["gc", "--idle", "99999999999999999999999"],
            &["gc", "--idle", "2", "--idle", "3"],
            &["gc", "--remove"],
            &["gc", "--remove", ""],
            &["gc", "--remove", "a", "--remove", "b"],
            &["gc", "--force"],
            &["gc", "--idle=3"],
            &["gc", "extra"],
        ] {
            assert!(parse_args(&words(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_porcelain_keep_follows_the_verdict_precedence_and_never_passes_unknown() {
        let sound = WorktreeEntry {
            path: "/w".to_owned(),
            ..WorktreeEntry::default()
        };
        assert_eq!(porcelain_keep(&sound), None);
        let with = |edit: fn(&mut WorktreeEntry)| {
            let mut entry = sound.clone();
            edit(&mut entry);
            porcelain_keep(&entry)
        };
        assert_eq!(with(|e| e.unknown = true), Some("unknown"));
        assert_eq!(
            with(|e| {
                e.unknown = true;
                e.locked = true;
            }),
            Some("locked")
        );
        assert_eq!(
            with(|e| {
                e.locked = true;
                e.prunable = true;
            }),
            Some("missing")
        );
        assert_eq!(
            with(|e| {
                e.prunable = true;
                e.bare = true;
            }),
            Some("bare")
        );
    }

    #[test]
    fn containment_is_component_wise_on_either_spelling() {
        let raw = Path::new("/r/wt");
        assert!(at_or_below(Path::new("/r/wt"), raw, None));
        assert!(at_or_below(Path::new("/r/wt/a/b"), raw, None));
        assert!(!at_or_below(Path::new("/r/wt2"), raw, None));
        assert!(!at_or_below(Path::new("/r"), raw, None));
        assert!(at_or_below(
            Path::new("/real/wt/x"),
            raw,
            Some(Path::new("/real/wt"))
        ));
    }

    #[test]
    fn only_a_canonical_spelling_may_stand_in_for_a_vanished_dir() {
        for sound in ["/", "/r/wt", "/r/wt /x", "/r/.hidden/x"] {
            assert!(canonical_spelling(Path::new(sound)), "{sound}");
        }
        for damaged in [
            "r/wt", "/r/../wt", "/r/./wt", "/r//wt", "/r/wt/", "/r/wt/.", "/r/wt/..", "",
        ] {
            assert!(!canonical_spelling(Path::new(damaged)), "{damaged}");
        }
    }

    #[test]
    fn short_branch_strips_only_the_heads_prefix() {
        assert_eq!(short_branch("refs/heads/a/b"), "a/b");
        assert_eq!(short_branch("refs/remotes/o/a"), "refs/remotes/o/a");
    }

    #[test]
    fn the_token_hash_is_fnv1a_64_over_length_prefixed_fields() {
        // Published FNV-1a 64 vectors.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
        let mut payload = Vec::new();
        push_field(&mut payload, b"ab");
        push_field(&mut payload, b"");
        assert_eq!(payload, b"2:ab0:");
        let mut split = Vec::new();
        push_field(&mut split, b"a");
        push_field(&mut split, b"b");
        assert_ne!(payload, split, "length prefixes keep field borders");
    }
}
