//! ae's git facts, derived by running `git` — the ONLY product caller of
//! [`crate::transport::run_git`], the fixed-program git leg of the one process
//! door.
//!
//! Six consumers, one door: the archive preview's range and HEAD facts, the
//! `ae end` path's commit and push, the watchdog's branch-and-dirty segment,
//! `ae brief`'s dirty marker — which is the watchdog's [`work_tree_dirty`], read
//! a second time rather than spelled a second way, so the `*` on a card and the
//! `*` on a status line can never disagree — the stopped rename's worktree
//! registration and admin dir, and the worktree cleanup facts that
//! `ae worktree gc` and the retire guard share ([`crate::worktrees`]).
//!
//! A non-local (`worktree`/`copy`) preview runs these in the session's work dir.
//! Two properties are structural, not incidental:
//!
//! * **No shell, so nothing to inject.** Every invocation is built as OS-native
//!   argv ([`argv`]): the work-tree path is one `OsString` element after `-C`,
//!   and the base/tip shas are their own elements. A hostile `work_dir` or a sha
//!   with metacharacters is data to `git`, never a command line. The path is
//!   taken as RAW bytes from the meta value (not a lossy `String`), so a valid
//!   non-UTF-8 work dir survives to `git` intact.
//! * **The interpreters are strict, and git answers the filesystem.** A HEAD is
//!   a value only if it is exactly 40 lowercase hex; a count only if it is all
//!   ASCII digits — anything else (a bare repo's `HEAD`, an unborn branch's
//!   error echo, a `fatal:` line) falls to `-`. Missing, non-directory and
//!   non-repository paths are not pre-checked with a filesystem `stat`; `git`
//!   itself fails on them and the failure becomes `-`, so this module reads
//!   nothing of the world but a process's exit and stdout.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt as _;

/// Every question ae asks git, each a fixed argv shape.
enum Query<'a> {
    /// `rev-parse --is-inside-work-tree` — the guard, judged by exit status.
    IsWorkTree,
    /// `rev-parse HEAD` — the final commit, judged by its printed value.
    Head,
    /// `merge-base --is-ancestor <base> <tip>` — the range guard (exit status).
    IsAncestor { base: &'a str, tip: &'a str },
    /// `rev-list --count <base>..<tip>` — the range size (printed value).
    CountRange { base: &'a str, tip: &'a str },
    /// `symbolic-ref --quiet --short HEAD` — the branch NAME, judged by its
    /// printed value.
    Branch,
    /// `rev-parse --short HEAD` — the detached-HEAD fallback the branch segment
    /// reaches for when `symbolic-ref` names nothing.
    ShortHead,
    /// `status --porcelain --untracked-files=no` — the dirty marker.
    PorcelainStatus,
    /// `worktree list --porcelain` — the managed-worktree registration the
    /// stopped rename proves ownership and destination absence against.
    WorktreeList,
    /// `rev-parse --absolute-git-dir` — the worktree's OWN git dir, which the
    /// stopped rename asks per attempt instead of spelling the admin id from
    /// the leaf git suffixes on collision (#196).
    AbsoluteGitDir,
    /// `worktree move <old> <new>` — the stopped rename's managed git move,
    /// run with `-C <origin>`. Git itself refuses a main worktree, a locked
    /// one and a populated-submodule one before moving anything.
    WorktreeMove {
        worktree: &'a OsStr,
        new_path: &'a OsStr,
    },
    /// `worktree remove --force <worktree>` — the git-mode teardown's workdir
    /// commit, judged by exit status.
    WorktreeRemove { worktree: &'a OsStr },
    /// `worktree remove <worktree>` — `ae worktree gc`'s removal, WITHOUT
    /// `--force`, so git itself still refuses a dirty, untracked or locked
    /// tree after ae's own proof.
    WorktreeRemovePlain { worktree: &'a OsStr },
    /// `reflog exists HEAD` — whether THIS worktree keeps its own HEAD reflog,
    /// judged by exit status. `reflog -n 1 HEAD` alone would answer from the
    /// checked-out branch's reflog when the worktree's own is gone.
    ReflogExists,
    /// `reflog -n 1 --date=unix --format=%gd HEAD` — the newest HEAD reflog
    /// entry, printed as `HEAD@{<unix seconds>}`.
    ReflogNewest,
    /// `ls-files --others --ignored --exclude-standard --directory
    /// --no-empty-directory -z`, cache and fsmonitor off — the ignored paths a
    /// plain `worktree remove` would delete with the tree.
    IgnoredOthers,
    /// `worktree prune` — the end path's housekeeping after a managed
    /// worktree is removed.
    WorktreePrune,
    /// `worktree add --detach <worktree> HEAD` — the `--worktree` launch's
    /// working copy, run with `-C <origin>`.
    WorktreeAdd { worktree: &'a OsStr },
    /// `add -A` — the end path's stage-everything before the session commit.
    AddAll,
    /// `commit -m <subject> -m <body>` — the two-`-m` session commit.
    Commit { subject: &'a str, body: &'a str },
    /// `diff --quiet` — unstaged changes, judged by exit status (non-zero = dirty).
    DiffQuiet,
    /// `diff --cached --quiet` — staged changes, same judgement.
    DiffCachedQuiet,
    /// `ls-files --others --exclude-standard` — untracked, judged by output.
    LsFilesOthers,
    /// End's lossless-removal proof, independent of dirt-hiding settings.
    LosslessDiff { cached: bool },
    /// The same forced-settings diff with `--name-only -z` instead of
    /// `--quiet`: asked only after a `LosslessDiff` failed, so a named path
    /// proves a change and a failure stays unknown.
    DiffNames { cached: bool },
    /// `ls-files --others --directory -z` — include ignored and empty paths.
    AllOthers,
    /// `ls-files --stage --full-name -v -z` — full index entries and flags.
    IndexEntries,
    /// `hash-object --no-filters -- <paths>` — hashes of actual workdir bytes.
    HashPaths { paths: &'a [&'a str] },
    /// `remote get-url origin` — whether a push target exists at all.
    RemoteGetUrl,
    /// `fetch origin --quiet` — the silent refresh before the
    /// reachability test.
    FetchOrigin,
    /// `branch -r --contains HEAD` — whether HEAD is already on a remote branch.
    BranchRemoteContains,
    /// `merge-base HEAD origin/HEAD` — the base the pushed-file count is taken from.
    MergeBaseOriginHead,
    /// `diff --name-only <base> HEAD` — the files the push carries.
    DiffNameOnly { base: &'a str },
    /// `push -u origin HEAD:refs/heads/<branch>` — the end path's push, judged
    /// by exit status.
    PushHead { branch: &'a str },
    /// `log -n 5 --format=%s` — the seed pack's recent-commit subjects. The
    /// count is BAKED so the shape stays fixed: a caller-chosen number would
    /// make this the one query whose argv is not visible in this file.
    RecentSubjects,
    /// `describe --tags --abbrev=0` — the seed pack's latest reachable tag.
    LatestTag,
}

/// A git argv minted ONLY by this module's [`argv`] builder.
pub(crate) struct GitArgv(Vec<OsString>);

impl GitArgv {
    /// The OS-native argv for the transport door to spawn.
    pub(crate) fn as_os_args(&self) -> &[OsString] {
        &self.0
    }
}

/// Build the OS-native argv for one query, always `-C <wdir>` first so git runs
/// in the work dir without the process ever changing directory.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per fixed query shape; the whole point is that every git command line this crate can build is visible in one place"
)]
fn argv(wdir: &OsStr, query: &Query) -> GitArgv {
    let mut args = vec![OsString::from("-C"), wdir.to_owned()];
    match *query {
        Query::IsWorkTree => {
            args.push("rev-parse".into());
            args.push("--is-inside-work-tree".into());
        }
        Query::Head => {
            args.push("rev-parse".into());
            args.push("HEAD".into());
        }
        Query::IsAncestor { base, tip } => {
            args.push("merge-base".into());
            args.push("--is-ancestor".into());
            args.push(base.into());
            args.push(tip.into());
        }
        Query::CountRange { base, tip } => {
            args.push("rev-list".into());
            args.push("--count".into());
            args.push(format!("{base}..{tip}").into());
        }
        Query::Branch => {
            args.push("symbolic-ref".into());
            args.push("--quiet".into());
            args.push("--short".into());
            args.push("HEAD".into());
        }
        Query::ShortHead => {
            args.push("rev-parse".into());
            args.push("--short".into());
            args.push("HEAD".into());
        }
        Query::PorcelainStatus => {
            args.push("status".into());
            args.push("--porcelain".into());
            args.push("--untracked-files=no".into());
        }
        Query::WorktreeList => {
            args.push("worktree".into());
            args.push("list".into());
            args.push("--porcelain".into());
        }
        Query::AbsoluteGitDir => {
            args.push("rev-parse".into());
            args.push("--absolute-git-dir".into());
        }
        Query::WorktreeMove { worktree, new_path } => {
            args.push("worktree".into());
            args.push("move".into());
            args.push(worktree.to_owned());
            args.push(new_path.to_owned());
        }
        Query::WorktreeRemove { worktree } => {
            args.push("worktree".into());
            args.push("remove".into());
            args.push("--force".into());
            args.push(worktree.to_owned());
        }
        Query::WorktreeRemovePlain { worktree } => {
            args.push("worktree".into());
            args.push("remove".into());
            args.push(worktree.to_owned());
        }
        Query::ReflogExists => {
            args.extend(["reflog", "exists", "HEAD"].map(OsString::from));
        }
        Query::ReflogNewest => {
            args.extend(
                ["reflog", "-n", "1", "--date=unix", "--format=%gd", "HEAD"].map(OsString::from),
            );
        }
        Query::IgnoredOthers => {
            args.extend(
                [
                    "-c",
                    "core.untrackedCache=false",
                    "-c",
                    "core.fsmonitor=false",
                    "ls-files",
                    "--others",
                    "--ignored",
                    "--exclude-standard",
                    "--directory",
                    "--no-empty-directory",
                    "-z",
                ]
                .map(OsString::from),
            );
        }
        Query::WorktreePrune => {
            args.push("worktree".into());
            args.push("prune".into());
        }
        Query::WorktreeAdd { worktree } => {
            args.push("worktree".into());
            args.push("add".into());
            args.push("--detach".into());
            args.push(worktree.to_owned());
            args.push("HEAD".into());
        }
        Query::AddAll => {
            args.push("add".into());
            args.push("-A".into());
        }
        Query::Commit { subject, body } => {
            args.push("commit".into());
            args.push("-m".into());
            args.push(subject.into());
            args.push("-m".into());
            args.push(body.into());
        }
        Query::DiffQuiet => {
            args.push("diff".into());
            args.push("--quiet".into());
        }
        Query::DiffCachedQuiet => {
            args.push("diff".into());
            args.push("--cached".into());
            args.push("--quiet".into());
        }
        Query::LsFilesOthers => {
            args.push("ls-files".into());
            args.push("--others".into());
            args.push("--exclude-standard".into());
        }
        Query::LosslessDiff { cached } | Query::DiffNames { cached } => {
            for setting in [
                "core.fileMode=true",
                "core.ignoreStat=false",
                "core.fsmonitor=false",
                "core.trustctime=true",
                "core.checkStat=default",
                "core.symlinks=true",
            ] {
                args.extend(["-c".into(), setting.into()]);
            }
            args.extend(
                [
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--ignore-submodules=none",
                ]
                .map(OsString::from),
            );
            if matches!(*query, Query::DiffNames { .. }) {
                args.extend(["--name-only".into(), "-z".into()]);
            } else {
                args.push("--quiet".into());
            }
            if cached {
                args.push("--cached".into());
            }
        }
        Query::AllOthers => {
            args.extend(
                [
                    "-c",
                    "core.untrackedCache=false",
                    "-c",
                    "core.fsmonitor=false",
                ]
                .map(OsString::from),
            );
            args.extend(["ls-files", "--others", "--directory", "-z"].map(OsString::from));
        }
        Query::IndexEntries => {
            args.extend(
                [
                    "-c",
                    "core.untrackedCache=false",
                    "-c",
                    "core.fsmonitor=false",
                ]
                .map(OsString::from),
            );
            args.extend(["ls-files", "--stage", "--full-name", "-v", "-z"].map(OsString::from));
        }
        Query::HashPaths { paths } => {
            args.extend(["hash-object", "--no-filters", "--"].map(OsString::from));
            args.extend(paths.iter().map(OsString::from));
        }
        Query::RemoteGetUrl => {
            args.push("remote".into());
            args.push("get-url".into());
            args.push("origin".into());
        }
        Query::FetchOrigin => {
            args.push("fetch".into());
            args.push("origin".into());
            args.push("--quiet".into());
        }
        Query::BranchRemoteContains => {
            args.push("branch".into());
            args.push("-r".into());
            args.push("--contains".into());
            args.push("HEAD".into());
        }
        Query::MergeBaseOriginHead => {
            args.push("merge-base".into());
            args.push("HEAD".into());
            args.push("origin/HEAD".into());
        }
        Query::DiffNameOnly { base } => {
            args.push("diff".into());
            args.push("--name-only".into());
            args.push(base.into());
            args.push("HEAD".into());
        }
        Query::PushHead { branch } => {
            args.push("push".into());
            args.push("-u".into());
            args.push("origin".into());
            args.push(format!("HEAD:refs/heads/{branch}").into());
        }
        Query::RecentSubjects => {
            args.push("log".into());
            args.push("-n".into());
            args.push(RECENT_SUBJECTS.to_string().into());
            args.push("--format=%s".into());
        }
        Query::LatestTag => {
            args.push("describe".into());
            args.push("--tags".into());
            args.push("--abbrev=0".into());
        }
    }
    GitArgv(args)
}

/// A commit is a value only as exactly 40 lowercase-hex.
fn is_hex40(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A count is a value only as one-or-more ASCII digits.
fn is_count(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `_ar_git_head <wdir>`: the 40-hex HEAD of the work tree at `wdir`, or `-`.
pub(crate) fn head(wdir: &[u8]) -> String {
    if wdir.is_empty() {
        return "-".to_owned();
    }
    let wdir = OsStr::from_bytes(wdir);
    if !crate::transport::run_git(&argv(wdir, &Query::IsWorkTree)).0 {
        return "-".to_owned();
    }
    let out = crate::transport::run_git(&argv(wdir, &Query::Head)).1;
    let head = out.trim();
    if is_hex40(head) {
        head.to_owned()
    } else {
        "-".to_owned()
    }
}

/// `_ar_git_range <wdir> <base> <tip>`: `(range, count)` for `base..tip`, or
/// `("-", "-")`.
pub(crate) fn range(wdir: &[u8], base: &str, tip: &str) -> (String, String) {
    let dash = || ("-".to_owned(), "-".to_owned());
    if !is_hex40(base) || !is_hex40(tip) || wdir.is_empty() {
        return dash();
    }
    let wdir = OsStr::from_bytes(wdir);
    if !crate::transport::run_git(&argv(wdir, &Query::IsAncestor { base, tip })).0 {
        return dash();
    }
    let out = crate::transport::run_git(&argv(wdir, &Query::CountRange { base, tip })).1;
    let count = out.trim();
    if is_count(count) {
        (format!("{base}..{tip}"), count.to_owned())
    } else {
        dash()
    }
}

/// `git -C <origin> worktree remove --force <worktree>` — remove the exact
/// managed git worktree of a `mode=git` teardown.
pub(crate) fn worktree_remove(origin: &[u8], worktree: &[u8]) -> bool {
    let origin = OsStr::from_bytes(origin);
    let worktree = OsStr::from_bytes(worktree);
    crate::transport::run_git(&argv(origin, &Query::WorktreeRemove { worktree })).0
}

/// `git -C <origin> worktree list --porcelain` — the registration text the
/// stopped rename proves worktree ownership and destination absence against,
/// or `None` when git itself fails there.
pub(crate) fn worktree_list(origin: &[u8]) -> Option<String> {
    let origin = OsStr::from_bytes(origin);
    let (succeeded, listed) = crate::transport::run_git(&argv(origin, &Query::WorktreeList));
    succeeded.then_some(listed)
}

/// One `git worktree list --porcelain` entry, with every attribute git printed.
#[allow(
    clippy::struct_excessive_bools,
    reason = "one flag per bare porcelain attribute line git prints, plus the parser's own verdict"
)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorktreeEntry {
    /// The `worktree <path>` opener's path, trailing whitespace trimmed and a
    /// C-quoted spelling undone — the reading the stopped rename relies on.
    pub path: String,
    /// The `HEAD <sha>` value, only as exactly 40 lowercase hex.
    pub head: Option<String>,
    /// The `branch <ref>` value: the FULL ref name git printed.
    pub branch: Option<String>,
    /// A bare `detached` line.
    pub detached: bool,
    /// A bare `bare` line.
    pub bare: bool,
    /// A `locked` line, with or without a reason.
    pub locked: bool,
    /// A `prunable` line, with or without a reason: git's own "the directory
    /// is gone" fact.
    pub prunable: bool,
    /// Whether any fact of this entry is not what git prints for a sound
    /// worktree: a path that is not absolute, carries a U+FFFD from a lossy
    /// read, or needed trimming or unquoting; a missing or non-40-hex HEAD; a
    /// non-bare entry without exactly one of `branch` and `detached`; a
    /// repeated or unknown attribute line; or a path another entry also
    /// names. An unknown entry keeps its tree whatever else it says.
    pub unknown: bool,
}

/// Parse `git worktree list --porcelain`: blank-line-separated entries, each
/// opened by a `worktree <path>` line. Attribute lines before any opener are
/// ignored. Never panics, on any input (fuzz target `worktree_porcelain`).
#[must_use]
pub fn parse_worktree_porcelain(text: &str) -> Vec<WorktreeEntry> {
    let mut entries = Vec::new();
    let mut current: Option<(WorktreeEntry, Vec<&str>)> = None;
    for raw in text.lines() {
        let line = raw.trim_end();
        if line.is_empty() {
            if let Some(pending) = current.take() {
                entries.push(finish_entry(pending.0));
            }
            continue;
        }
        if let Some(spelled) = line.strip_prefix("worktree ") {
            if let Some(pending) = current.take() {
                entries.push(finish_entry(pending.0));
            }
            let path = unquote_porcelain(spelled);
            let exact = raw.strip_prefix("worktree ") == Some(path.as_str());
            let unknown = !exact || !path.starts_with('/') || path.contains('\u{fffd}');
            current = Some((
                WorktreeEntry {
                    path,
                    unknown,
                    ..WorktreeEntry::default()
                },
                Vec::new(),
            ));
            continue;
        }
        let Some((entry, seen)) = current.as_mut() else {
            continue;
        };
        let (word, value) = line
            .split_once(' ')
            .map_or((line, None), |(word, value)| (word, Some(value)));
        if seen.contains(&word) {
            entry.unknown = true;
        } else {
            seen.push(word);
        }
        match (word, value) {
            ("HEAD", Some(sha)) if is_hex40(sha) => entry.head = Some(sha.to_owned()),
            ("branch", Some(reference)) if !reference.is_empty() => {
                entry.branch = Some(reference.to_owned());
            }
            ("detached", None) => entry.detached = true,
            ("bare", None) => entry.bare = true,
            ("locked", _) => entry.locked = true,
            ("prunable", _) => entry.prunable = true,
            _ => entry.unknown = true,
        }
    }
    if let Some(pending) = current.take() {
        entries.push(finish_entry(pending.0));
    }
    // A path listed twice cannot say which entry's facts are its own.
    let paths: Vec<String> = entries.iter().map(|entry| entry.path.clone()).collect();
    for entry in &mut entries {
        if paths.iter().filter(|path| **path == entry.path).count() > 1 {
            entry.unknown = true;
        }
    }
    entries
}

/// Close one entry: a non-bare worktree needs a HEAD and exactly one of
/// `branch` and `detached`.
fn finish_entry(mut entry: WorktreeEntry) -> WorktreeEntry {
    if !entry.bare && (entry.head.is_none() || entry.detached == entry.branch.is_some()) {
        entry.unknown = true;
    }
    entry
}

/// Unquote one porcelain path: plain, or C-quoted with the escapes git
/// emits for spaces and quotes. Anything else rides through literally rather
/// than failing the whole listing.
fn unquote_porcelain(path: &str) -> String {
    if path.len() < 2 || !path.starts_with('"') || !path.ends_with('"') {
        return path.to_owned();
    }
    let inner = &path[1..path.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') | None => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

/// `git -C <origin> worktree remove <worktree>` with NO `--force` — the gc
/// removal, judged by exit status. Git refuses a dirty, untracked or locked
/// tree itself; it deletes ignored files with the tree and keeps the branch.
pub(crate) fn worktree_remove_plain(origin: &[u8], worktree: &[u8]) -> bool {
    let origin = OsStr::from_bytes(origin);
    let worktree = OsStr::from_bytes(worktree);
    crate::transport::run_git(&argv(origin, &Query::WorktreeRemovePlain { worktree })).0
}

/// Whether `base` is an ancestor of `tip`, asked in `wdir`. Both must be
/// 40-hex; any non-zero exit (not an ancestor, or a git failure) is `false`.
pub(crate) fn is_ancestor(wdir: &[u8], base: &str, tip: &str) -> bool {
    !wdir.is_empty()
        && is_hex40(base)
        && is_hex40(tip)
        && crate::transport::run_git(&argv(
            OsStr::from_bytes(wdir),
            &Query::IsAncestor { base, tip },
        ))
        .0
}

/// Whether `head` is merged into `target`, asked in `wdir`: `Some(true)` when
/// git confirms the ancestry, `Some(false)` only when git then counts commits
/// in `head` that `target` lacks, `None` when neither is proven (a failed
/// call, or a zero count after a failed ancestry check).
pub(crate) fn merged_into(wdir: &[u8], head: &str, target: &str) -> Option<bool> {
    if is_ancestor(wdir, head, target) {
        return Some(true);
    }
    if wdir.is_empty() || !is_hex40(head) || !is_hex40(target) {
        return None;
    }
    let (ok, out) = ask(
        OsStr::from_bytes(wdir),
        &Query::CountRange {
            base: target,
            tip: head,
        },
    );
    let count = out.trim();
    (ok && is_count(count) && count.bytes().any(|digit| digit != b'0')).then_some(false)
}

/// The unix time of the newest entry in the worktree's OWN HEAD reflog, or
/// `None` when that reflog is absent, empty, or answers anything but one
/// strict `HEAD@{<digits>}` line.
pub(crate) fn head_reflog_newest(wdir: &[u8]) -> Option<i64> {
    if wdir.is_empty() {
        return None;
    }
    let wdir = OsStr::from_bytes(wdir);
    if !crate::transport::run_git(&argv(wdir, &Query::ReflogExists)).0 {
        return None;
    }
    let (ok, out) = crate::transport::run_git(&argv(wdir, &Query::ReflogNewest));
    if !ok {
        return None;
    }
    parse_reflog_selector(&out)
}

/// `HEAD@{<digits>}` plus at most one trailing newline, as seconds.
fn parse_reflog_selector(out: &str) -> Option<i64> {
    let line = out.strip_suffix('\n').unwrap_or(out);
    let digits = line.strip_prefix("HEAD@{")?.strip_suffix('}')?;
    if !is_count(digits) {
        return None;
    }
    digits.parse().ok()
}

/// The ignored directories a plain `worktree remove` may delete with the
/// tree: build caches, which a rebuild recreates. Every other ignored path is
/// data and keeps the tree.
pub(crate) const CACHE_DIRS: [&str; 3] = ["target", ".venv", "node_modules"];

/// Whether one `IgnoredOthers` entry is a whole cache directory: it ends in
/// `/` and its last component is one of [`CACHE_DIRS`].
fn is_cache_entry(entry: &str) -> bool {
    entry
        .strip_suffix('/')
        .and_then(|dir| dir.rsplit('/').next())
        .is_some_and(|leaf| CACHE_DIRS.contains(&leaf))
}

/// What the worktree clean checks found: the gc STATE column and the retire
/// guard's cleanliness verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cleanliness {
    /// Every check held. `proven` is the full proof (HEAD and index pinned,
    /// raw bytes hashed, both re-read); without it only the cheap checks ran.
    Clean { proven: bool },
    /// A tracked or staged change git's diff reports.
    Dirty,
    /// Raw bytes differ from their index hash although the diff is quiet.
    DirtyBytes,
    /// An index entry with a flag (assume-unchanged, skip-worktree, other
    /// tags) or a conflict stage.
    IndexFlags,
    /// An index entry that is not a plain or executable file: a symlink, a
    /// gitlink or any other mode.
    UnsupportedMode,
    /// An untracked path that is not ignored.
    Untracked,
    /// An ignored path that is not a whole build cache; the first one git
    /// listed.
    IgnoredData(String),
    /// A git call failed or answered lossily, or a pin changed mid-proof.
    Unknown,
}

/// Why the complete index cannot be read as plain file rows.
enum IndexDefect {
    /// The read failed, was lossy, or was cut short.
    Unreadable,
    /// A flag tag or a conflict stage.
    Flags,
    /// A mode other than `100644` or `100755`.
    Mode,
    /// A row whose shape is not `<tag> <mode> <hash> <stage>\t<path>`.
    Malformed,
}

impl IndexDefect {
    const fn cleanliness(&self) -> Cleanliness {
        match self {
            Self::Unreadable | Self::Malformed => Cleanliness::Unknown,
            Self::Flags => Cleanliness::IndexFlags,
            Self::Mode => Cleanliness::UnsupportedMode,
        }
    }
}

/// What hashing the tracked paths' actual bytes showed.
enum TrackedBytes {
    Equal,
    Differ,
    Unreadable,
}

/// One git question in `path`.
fn ask(path: &OsStr, query: &Query<'_>) -> (bool, String) {
    crate::transport::run_git(&argv(path, query))
}

/// The complete `IndexEntries` answer as `(hash, path)` rows, or the first
/// reason it cannot be.
fn index_rows(ok: bool, index: &str) -> Result<Vec<(&str, &str)>, IndexDefect> {
    if !ok || index.contains('\u{fffd}') || !index.is_empty() && !index.ends_with('\0') {
        return Err(IndexDefect::Unreadable);
    }
    index.split_terminator('\0').map(index_row).collect()
}

/// Hash every tracked path's workdir bytes with no filters, in chunks of 128,
/// and compare each with its index hash.
fn tracked_bytes(path: &OsStr, entries: &[(&str, &str)]) -> TrackedBytes {
    for chunk in entries.chunks(128) {
        let paths: Vec<_> = chunk.iter().map(|(_, name)| *name).collect();
        let (ok, hashes) = ask(path, &Query::HashPaths { paths: &paths });
        if !ok {
            return TrackedBytes::Unreadable;
        }
        if hashes.lines().ne(chunk.iter().map(|(hash, _)| *hash)) {
            return TrackedBytes::Differ;
        }
    }
    TrackedBytes::Equal
}

/// Both forced-settings diffs quiet: no tracked and no staged change.
fn diffs_quiet(path: &OsStr) -> bool {
    [false, true]
        .into_iter()
        .all(|cached| ask(path, &Query::LosslessDiff { cached }).0)
}

/// What both forced-settings diffs showed for the worktree clean checks.
enum Diffs {
    /// Both quiet.
    Quiet,
    /// A diff failed and git, asked again, named a changed path.
    Changed,
    /// A diff failed and no change was proven: a failure, not dirt.
    Unproven,
}

/// Both diffs, judged so a failed git call never reads as dirt: a failed
/// `--quiet` diff is asked again with `--name-only`, and only a named path is
/// `Changed`. A proven change on either side wins over an unproven one.
fn diffs_state(path: &OsStr) -> Diffs {
    let mut unproven = false;
    for cached in [false, true] {
        if ask(path, &Query::LosslessDiff { cached }).0 {
            continue;
        }
        let (ok, names) = ask(path, &Query::DiffNames { cached });
        if ok && !names.is_empty() {
            return Diffs::Changed;
        }
        unproven = true;
    }
    if unproven {
        Diffs::Unproven
    } else {
        Diffs::Quiet
    }
}

/// The untracked and ignored halves every clean check ends with.
fn others_clean(path: &OsStr) -> Cleanliness {
    let (ok, others) = ask(path, &Query::LsFilesOthers);
    if !ok {
        return Cleanliness::Unknown;
    }
    if !others.is_empty() {
        return Cleanliness::Untracked;
    }
    let (ok, ignored) = ask(path, &Query::IgnoredOthers);
    if !ok || ignored.contains('\u{fffd}') || !ignored.is_empty() && !ignored.ends_with('\0') {
        return Cleanliness::Unknown;
    }
    match ignored
        .split_terminator('\0')
        .find(|entry| !is_cache_entry(entry))
    {
        Some(data) => Cleanliness::IgnoredData(data.to_owned()),
        None => Cleanliness::Clean { proven: false },
    }
}

/// The CHEAP clean checks at `wdir`: the index row rule, both diffs, the
/// untracked list and the ignored list. `Clean { proven: false }` at best.
pub(crate) fn cheap_cleanliness(wdir: &[u8]) -> Cleanliness {
    if wdir.is_empty() {
        return Cleanliness::Unknown;
    }
    let path = OsStr::from_bytes(wdir);
    let (ok, index) = ask(path, &Query::IndexEntries);
    if let Err(defect) = index_rows(ok, &index) {
        return defect.cleanliness();
    }
    match diffs_state(path) {
        Diffs::Quiet => {}
        Diffs::Changed => return Cleanliness::Dirty,
        Diffs::Unproven => return Cleanliness::Unknown,
    }
    others_clean(path)
}

/// The FULL clean proof at `wdir` for the commit `head` the caller judged:
/// HEAD pinned and equal to `head`, the index pinned, the row rule, both
/// diffs, every tracked path's raw bytes hashed, HEAD and the index re-read
/// unchanged, then the untracked and ignored lists. A HEAD other than `head`
/// is `Unknown`: the proof would cover a commit nobody judged merged.
pub(crate) fn full_cleanliness(wdir: &[u8], head: &str) -> Cleanliness {
    if wdir.is_empty() || !is_hex40(head) {
        return Cleanliness::Unknown;
    }
    let path = OsStr::from_bytes(wdir);
    let (ok, head_before) = ask(path, &Query::Head);
    if !ok || head_before.trim() != head {
        return Cleanliness::Unknown;
    }
    let (ok, index) = ask(path, &Query::IndexEntries);
    let entries = match index_rows(ok, &index) {
        Ok(entries) => entries,
        Err(defect) => return defect.cleanliness(),
    };
    match diffs_state(path) {
        Diffs::Quiet => {}
        Diffs::Changed => return Cleanliness::Dirty,
        Diffs::Unproven => return Cleanliness::Unknown,
    }
    match tracked_bytes(path, &entries) {
        TrackedBytes::Equal => {}
        TrackedBytes::Differ => return Cleanliness::DirtyBytes,
        TrackedBytes::Unreadable => return Cleanliness::Unknown,
    }
    let (ok, index_after) = ask(path, &Query::IndexEntries);
    let (head_ok, head_after) = ask(path, &Query::Head);
    if !ok || !head_ok || index_after != index || head_after != head_before {
        return Cleanliness::Unknown;
    }
    match others_clean(path) {
        Cleanliness::Clean { .. } => Cleanliness::Clean { proven: true },
        other => other,
    }
}

/// `git -C <work> rev-parse --absolute-git-dir` — the worktree's OWN git dir
/// as git spells it, or `None` when git itself fails there. The stopped
/// rename's admin leg asks this per attempt; the caller containment-checks
/// the answer before trusting it.
pub(crate) fn absolute_git_dir(work: &[u8]) -> Option<String> {
    if work.is_empty() {
        return None;
    }
    let work = OsStr::from_bytes(work);
    let (succeeded, dir) = crate::transport::run_git(&argv(work, &Query::AbsoluteGitDir));
    succeeded.then_some(dir)
}

/// `git -C <origin> worktree move <old> <new>` — move one managed worktree's
/// registration and directory together. Judged by exit status; git refuses a
/// main, locked or submodule-populated worktree before moving anything.
pub(crate) fn worktree_move(origin: &[u8], old: &[u8], new: &[u8]) -> bool {
    let origin = OsStr::from_bytes(origin);
    let old = OsStr::from_bytes(old);
    let new = OsStr::from_bytes(new);
    crate::transport::run_git(&argv(
        origin,
        &Query::WorktreeMove {
            worktree: old,
            new_path: new,
        },
    ))
    .0
}

/// The watchdog's branch segment: the branch NAME at `wdir`, or its short HEAD
/// when HEAD is detached, or `None`.
pub(crate) fn branch_head(wdir: &[u8]) -> Option<String> {
    if wdir.is_empty() {
        return None;
    }
    let wdir = OsStr::from_bytes(wdir);
    if !crate::transport::run_git(&argv(wdir, &Query::IsWorkTree)).0 {
        return None;
    }
    let (named, out) = crate::transport::run_git(&argv(wdir, &Query::Branch));
    let branch = out.trim();
    if named && !branch.is_empty() {
        return Some(branch.to_owned());
    }
    let (resolved, out) = crate::transport::run_git(&argv(wdir, &Query::ShortHead));
    let sha = out.trim();
    if resolved && !sha.is_empty() {
        Some(sha.to_owned())
    } else {
        None
    }
}

/// How many commit subjects a seed pack carries.
const RECENT_SUBJECTS: usize = 5;

/// The last [`RECENT_SUBJECTS`] commit subjects at `wdir`, newest first.
///
/// A subject is free text a human wrote, so nothing here interprets it beyond
/// dropping an empty line; the pack neutralises what it renders. A failed run,
/// a path that is no work tree, and a repository with no commits all answer the
/// same way — an empty list, which the pack prints as `none recorded`.
pub(crate) fn recent_subjects(wdir: &[u8]) -> Vec<String> {
    if wdir.is_empty() {
        return Vec::new();
    }
    let wdir = OsStr::from_bytes(wdir);
    if !crate::transport::run_git(&argv(wdir, &Query::IsWorkTree)).0 {
        return Vec::new();
    }
    let (succeeded, out) = crate::transport::run_git(&argv(wdir, &Query::RecentSubjects));
    if !succeeded {
        return Vec::new();
    }
    out.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(RECENT_SUBJECTS)
        .map(str::to_owned)
        .collect()
}

/// The nearest tag reachable from HEAD at `wdir`, or `None`.
///
/// STRICT, like every other interpreter here: a value only when the output is
/// one non-empty token with no whitespace and no control byte. `describe`
/// prints a `fatal:` line to stderr and nothing to stdout when no tag exists,
/// and an unannotated-tag repository can print surprising things; neither may
/// become a tag name the successor trusts.
pub(crate) fn latest_tag(wdir: &[u8]) -> Option<String> {
    if wdir.is_empty() {
        return None;
    }
    let wdir = OsStr::from_bytes(wdir);
    if !crate::transport::run_git(&argv(wdir, &Query::IsWorkTree)).0 {
        return None;
    }
    let (succeeded, out) = crate::transport::run_git(&argv(wdir, &Query::LatestTag));
    let tag = out.trim();
    (succeeded && !tag.is_empty() && !tag.chars().any(|ch| ch.is_whitespace() || ch.is_control()))
        .then(|| tag.to_owned())
}

/// Whether the work tree at `wdir` has TRACKED modifications — the `*` the
/// watchdog appends to the displayed branch.
pub(crate) fn work_tree_dirty(wdir: &[u8]) -> bool {
    if wdir.is_empty() {
        return false;
    }
    let wdir = OsStr::from_bytes(wdir);
    let (succeeded, out) = crate::transport::run_git(&argv(wdir, &Query::PorcelainStatus));
    succeeded && out.lines().any(|line| !line.is_empty())
}

/// The TRACKED paths one `git status --porcelain --untracked-files=no` output
/// names, each as git wrote it past its two status columns.
pub(crate) fn changed_paths(porcelain: &str) -> Vec<String> {
    porcelain
        .lines()
        .filter_map(|line| line.get(3..))
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// The tracked paths the work tree at `wdir` has changed, read through the
/// query [`work_tree_dirty`] asks.
pub(crate) fn tracked_changes(wdir: &[u8]) -> Vec<String> {
    if wdir.is_empty() {
        return Vec::new();
    }
    let wdir = OsStr::from_bytes(wdir);
    let (succeeded, out) = crate::transport::run_git(&argv(wdir, &Query::PorcelainStatus));
    if succeeded {
        changed_paths(&out)
    } else {
        Vec::new()
    }
}

// ---- the end path's git leg ------------------------------------------------
//
// `ae end` commits and pushes a managed session's work before anything is
// deleted. Every call below is one fixed [`Query`] through the same sealed
// [`GitArgv`] door, so the whole leg adds argv SHAPES and no new capability:
// the work-tree path stays one OS-native element after `-C`, and a branch or
// commit message rides as its own element with no shell anywhere.

/// Whether `wdir` is inside a git work tree — the end path's repo
/// precondition, judged by the exit status of `git -C … rev-parse
/// --is-inside-work-tree`.
pub(crate) fn is_work_tree(wdir: &[u8]) -> bool {
    !wdir.is_empty()
        && crate::transport::run_git(&argv(OsStr::from_bytes(wdir), &Query::IsWorkTree)).0
}

/// Whether the work tree has anything to commit: unstaged changes, staged
/// changes, or untracked files that are not ignored.
pub(crate) fn has_pending_work(wdir: &[u8]) -> bool {
    let path = OsStr::from_bytes(wdir);
    if !crate::transport::run_git(&argv(path, &Query::DiffQuiet)).0 {
        return true;
    }
    if !crate::transport::run_git(&argv(path, &Query::DiffCachedQuiet)).0 {
        return true;
    }
    let (_, listed) = crate::transport::run_git(&argv(path, &Query::LsFilesOthers));
    !listed.trim().is_empty()
}

/// Positive, conservative proof before managed-workdir removal. Failures,
/// ignored/untracked data (including empty dirs), masked index entries and
/// gitlinks preserve the directory. Hash actual bytes rather than trusting stat.
pub(crate) fn lossless_removal(wdir: &[u8]) -> Result<(), &'static str> {
    let path = OsStr::from_bytes(wdir);
    let (ok, before) = ask(path, &Query::Head);
    if !ok || !is_hex40(before.trim()) || !head_is_on_a_remote(wdir) {
        return Err("the current commit is not proven on a remote");
    }
    if !diffs_quiet(path) {
        return Err("tracked or staged data is not proven clean");
    }
    let (ok, others) = ask(path, &Query::AllOthers);
    if !ok || !others.is_empty() {
        return Err("untracked or ignored data remains, or its absence is unproven");
    }
    let (ok, index) = ask(path, &Query::IndexEntries);
    let entries = index_rows(ok, &index).map_err(|defect| match defect {
        IndexDefect::Unreadable => "the complete index could not be proven",
        IndexDefect::Flags | IndexDefect::Mode | IndexDefect::Malformed => {
            "the index has a masked, unsupported or gitlink entry"
        }
    })?;
    if !matches!(tracked_bytes(path, &entries), TrackedBytes::Equal) {
        return Err("tracked bytes differ from their committed index, or cannot be read");
    }
    let (ok, after_index) = ask(path, &Query::IndexEntries);
    let (head_ok, after_head) = ask(path, &Query::Head);
    if !ok || !head_ok || after_index != index || after_head != before {
        return Err("git facts changed during the removal proof");
    }
    Ok(())
}

/// One `ls-files --stage -v` row as `(hash, path)`: tag `H`, mode `100644`
/// or `100755`, a 40-hex hash, stage `0` and a non-empty path — or why not.
fn index_row(row: &str) -> Result<(&str, &str), IndexDefect> {
    let (tag, rest) = row.split_once(' ').ok_or(IndexDefect::Malformed)?;
    let (header, path) = rest.split_once('\t').ok_or(IndexDefect::Malformed)?;
    let mut fields = header.split(' ');
    let (Some(mode), Some(hash), Some(stage), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return Err(IndexDefect::Malformed);
    };
    if !is_hex40(hash) || path.is_empty() {
        return Err(IndexDefect::Malformed);
    }
    if tag != "H" || stage != "0" {
        return Err(IndexDefect::Flags);
    }
    if !matches!(mode, "100644" | "100755") {
        return Err(IndexDefect::Mode);
    }
    Ok((hash, path))
}

/// `git -C <wdir> add -A` then `commit -m <subject> -m <body>`.
pub(crate) fn commit_all(wdir: &[u8], subject: &str, body: &str) -> bool {
    let path = OsStr::from_bytes(wdir);
    if !crate::transport::run_git(&argv(path, &Query::AddAll)).0 {
        return false;
    }
    crate::transport::run_git(&argv(path, &Query::Commit { subject, body })).0
}

/// Whether a remote named `origin` is configured, by the exit status of
/// `git remote get-url origin`.
pub(crate) fn has_origin(wdir: &[u8]) -> bool {
    crate::transport::run_git(&argv(OsStr::from_bytes(wdir), &Query::RemoteGetUrl)).0
}

/// `git fetch origin --quiet`, best-effort.
pub(crate) fn fetch_origin(wdir: &[u8]) {
    let _ = crate::transport::run_git(&argv(OsStr::from_bytes(wdir), &Query::FetchOrigin));
}

/// Whether HEAD is already contained in some remote-tracking branch — the
/// frozen `branch -r --contains HEAD | grep -q .`, judged by OUTPUT, because
/// git exits zero with an empty list.
pub(crate) fn head_is_on_a_remote(wdir: &[u8]) -> bool {
    let (succeeded, listed) =
        crate::transport::run_git(&argv(OsStr::from_bytes(wdir), &Query::BranchRemoteContains));
    succeeded && !listed.trim().is_empty()
}

/// How many files the unpushed commits touch: the count of
/// `diff --name-only <merge-base HEAD origin/HEAD> HEAD`, with `HEAD~1`
/// standing in when no merge base resolves.
pub(crate) fn pushed_file_count(wdir: &[u8]) -> String {
    let path = OsStr::from_bytes(wdir);
    let (based, out) = crate::transport::run_git(&argv(path, &Query::MergeBaseOriginHead));
    let base = out.trim();
    let base = if based && !base.is_empty() {
        base.to_owned()
    } else {
        "HEAD~1".to_owned()
    };
    let (listed, names) =
        crate::transport::run_git(&argv(path, &Query::DiffNameOnly { base: &base }));
    if listed {
        names.lines().filter(|l| !l.is_empty()).count().to_string()
    } else {
        "?".to_owned()
    }
}

/// `git -C <wdir> push -u origin HEAD:refs/heads/<branch>` — judged by exit
/// status.
pub(crate) fn push_head(wdir: &[u8], branch: &str) -> bool {
    crate::transport::run_git(&argv(OsStr::from_bytes(wdir), &Query::PushHead { branch })).0
}

/// `git -C <origin> worktree prune`, best-effort — the housekeeping
/// after a managed worktree goes.
pub(crate) fn worktree_prune(origin: &[u8]) {
    let _ = crate::transport::run_git(&argv(OsStr::from_bytes(origin), &Query::WorktreePrune));
}

/// `git -C <origin> worktree add --detach <worktree> HEAD` — the `--worktree`
/// launch's working copy.
pub(crate) fn worktree_add_detached(origin: &[u8], worktree: &[u8]) -> bool {
    let origin = OsStr::from_bytes(origin);
    let worktree = OsStr::from_bytes(worktree);
    if crate::transport::run_git(&argv(origin, &Query::WorktreeAdd { worktree })).0 {
        return true;
    }
    let _ = crate::transport::run_git(&argv(origin, &Query::WorktreePrune));
    crate::transport::run_git(&argv(origin, &Query::WorktreeAdd { worktree })).0
}

#[cfg(test)]
mod tests {
    use super::{GitArgv, Query, argv, changed_paths, is_count, is_hex40};
    use std::ffi::OsString;

    fn strs(args: &GitArgv) -> Vec<String> {
        args.as_os_args()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_porcelain_row_names_its_path_verbatim_and_a_short_row_names_none() {
        let porcelain = concat!(
            " M src/a.rs\n",
            "M  b\n",
            "R  old -> new\n",
            "M  \"sp ace\"\n",
            "\n",
            "x\n",
            "M \n",
            "abé\n",
            "MM é\n",
        );
        assert_eq!(
            changed_paths(porcelain),
            ["src/a.rs", "b", "old -> new", "\"sp ace\"", "é"]
        );
        assert!(changed_paths("").is_empty());
    }

    #[test]
    fn hex40_accepts_only_exactly_forty_lowercase_hex() {
        assert!(is_hex40("0123456789abcdef0123456789abcdef01234567"));
        assert!(!is_hex40("0123456789ABCDEF0123456789abcdef01234567")); // uppercase
        assert!(!is_hex40("0123456789abcdef0123456789abcdef0123456")); // 39
        assert!(!is_hex40("0123456789abcdef0123456789abcdef012345678")); // 41
        assert!(!is_hex40("g123456789abcdef0123456789abcdef01234567")); // non-hex
        assert!(!is_hex40("")); // empty
        assert!(!is_hex40("HEAD")); // a bare repo's echo
    }

    #[test]
    fn count_accepts_only_ascii_digits() {
        assert!(is_count("0"));
        assert!(is_count("42"));
        assert!(!is_count("")); // empty
        assert!(!is_count("-1")); // sign
        assert!(!is_count("1 2")); // space
        assert!(!is_count("fatal")); // an error word
    }

    #[test]
    fn argv_is_c_first_then_the_fixed_subcommand() {
        let w = OsString::from("/w d"); // a space proves argv, not a command line
        assert_eq!(
            strs(&argv(&w, &Query::IsWorkTree)),
            ["-C", "/w d", "rev-parse", "--is-inside-work-tree"]
        );
        assert_eq!(
            strs(&argv(&w, &Query::Head)),
            ["-C", "/w d", "rev-parse", "HEAD"]
        );
        let base = "0123456789abcdef0123456789abcdef01234567";
        let tip = "89abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(
            strs(&argv(&w, &Query::IsAncestor { base, tip })),
            ["-C", "/w d", "merge-base", "--is-ancestor", base, tip]
        );
        assert_eq!(
            strs(&argv(&w, &Query::CountRange { base, tip })),
            [
                "-C",
                "/w d",
                "rev-list",
                "--count",
                &format!("{base}..{tip}")
            ]
        );
    }

    #[test]
    fn the_rename_worktree_shapes_are_c_first_then_fixed_words() {
        let w = OsString::from("/origin");
        assert_eq!(
            strs(&argv(&w, &Query::WorktreeList)),
            ["-C", "/origin", "worktree", "list", "--porcelain"]
        );
        let old = OsString::from("/wt/old");
        let new = OsString::from("/wt/new");
        assert_eq!(
            strs(&argv(
                &w,
                &Query::WorktreeMove {
                    worktree: &old,
                    new_path: &new,
                }
            )),
            ["-C", "/origin", "worktree", "move", "/wt/old", "/wt/new"]
        );
        let wt = OsString::from("/wt/old");
        assert_eq!(
            strs(&argv(&wt, &Query::AbsoluteGitDir)),
            ["-C", "/wt/old", "rev-parse", "--absolute-git-dir"]
        );
    }

    #[test]
    fn absolute_git_dir_refuses_an_empty_work_dir() {
        assert_eq!(super::absolute_git_dir(b""), None);
    }

    #[test]
    fn the_worktree_cleanup_shapes_are_c_first_then_fixed_words_and_never_force() {
        let w = OsString::from("/origin");
        let tree = OsString::from("/wt/a b");
        assert_eq!(
            strs(&argv(&w, &Query::WorktreeRemovePlain { worktree: &tree })),
            ["-C", "/origin", "worktree", "remove", "/wt/a b"]
        );
        assert_eq!(
            strs(&argv(&w, &Query::WorktreeRemove { worktree: &tree })),
            ["-C", "/origin", "worktree", "remove", "--force", "/wt/a b"]
        );
        assert_eq!(
            strs(&argv(&w, &Query::ReflogExists)),
            ["-C", "/origin", "reflog", "exists", "HEAD"]
        );
        assert_eq!(
            strs(&argv(&w, &Query::ReflogNewest)),
            [
                "-C",
                "/origin",
                "reflog",
                "-n",
                "1",
                "--date=unix",
                "--format=%gd",
                "HEAD"
            ]
        );
        assert_eq!(
            strs(&argv(&w, &Query::IgnoredOthers)),
            [
                "-C",
                "/origin",
                "-c",
                "core.untrackedCache=false",
                "-c",
                "core.fsmonitor=false",
                "ls-files",
                "--others",
                "--ignored",
                "--exclude-standard",
                "--directory",
                "--no-empty-directory",
                "-z"
            ]
        );
        let forced = [
            "-c",
            "core.fileMode=true",
            "-c",
            "core.ignoreStat=false",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.trustctime=true",
            "-c",
            "core.checkStat=default",
            "-c",
            "core.symlinks=true",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=none",
            "--name-only",
            "-z",
        ];
        for cached in [false, true] {
            let mut expected = vec!["-C", "/origin"];
            expected.extend(forced);
            if cached {
                expected.push("--cached");
            }
            assert_eq!(strs(&argv(&w, &Query::DiffNames { cached })), expected);
        }
    }

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn the_porcelain_parser_reads_every_attribute_of_a_sound_listing() {
        let listed = format!(
            "worktree /r\nHEAD {SHA}\nbranch refs/heads/main\n\n\
             worktree /w/a b\nHEAD {SHA}\ndetached\nlocked reason \"x\"\n\n\
             worktree /w/gone\nHEAD {SHA}\nbranch refs/heads/gone\nprunable gitdir file points to non-existent location\n\n\
             worktree /bare\nbare\n"
        );
        let entries = super::parse_worktree_porcelain(&listed);
        assert_eq!(
            entries,
            vec![
                super::WorktreeEntry {
                    path: "/r".to_owned(),
                    head: Some(SHA.to_owned()),
                    branch: Some("refs/heads/main".to_owned()),
                    ..super::WorktreeEntry::default()
                },
                super::WorktreeEntry {
                    path: "/w/a b".to_owned(),
                    head: Some(SHA.to_owned()),
                    detached: true,
                    locked: true,
                    ..super::WorktreeEntry::default()
                },
                super::WorktreeEntry {
                    path: "/w/gone".to_owned(),
                    head: Some(SHA.to_owned()),
                    branch: Some("refs/heads/gone".to_owned()),
                    prunable: true,
                    ..super::WorktreeEntry::default()
                },
                super::WorktreeEntry {
                    path: "/bare".to_owned(),
                    bare: true,
                    ..super::WorktreeEntry::default()
                },
            ]
        );
        assert!(super::parse_worktree_porcelain("").is_empty());
        assert!(super::parse_worktree_porcelain("HEAD x\nlocked\n").is_empty());
    }

    #[test]
    fn every_unsound_porcelain_fact_makes_its_entry_unknown_and_only_that_entry() {
        let good = format!("worktree /ok\nHEAD {SHA}\nbranch refs/heads/ok\n");
        for (why, bad) in [
            (
                "short head",
                "worktree /x\nHEAD abc\nbranch refs/heads/x\n".to_owned(),
            ),
            ("no head", "worktree /x\nbranch refs/heads/x\n".to_owned()),
            (
                "no branch, not detached",
                format!("worktree /x\nHEAD {SHA}\n"),
            ),
            (
                "branch and detached",
                format!("worktree /x\nHEAD {SHA}\nbranch refs/heads/x\ndetached\n"),
            ),
            (
                "repeated attribute",
                format!("worktree /x\nHEAD {SHA}\nHEAD {SHA}\nbranch refs/heads/x\n"),
            ),
            (
                "repeated lock",
                format!("worktree /x\nHEAD {SHA}\ndetached\nlocked\nlocked a\n"),
            ),
            (
                "unknown attribute",
                format!("worktree /x\nHEAD {SHA}\ndetached\nfrobbed\n"),
            ),
            (
                "empty branch",
                format!("worktree /x\nHEAD {SHA}\nbranch \n"),
            ),
            (
                "relative path",
                format!("worktree x\nHEAD {SHA}\ndetached\n"),
            ),
            (
                "quoted path",
                format!("worktree \"/x y\"\nHEAD {SHA}\ndetached\n"),
            ),
            (
                "trailing space",
                format!("worktree /x \nHEAD {SHA}\ndetached\n"),
            ),
            (
                "lossy path",
                format!("worktree /x\u{fffd}\nHEAD {SHA}\ndetached\n"),
            ),
        ] {
            let entries = super::parse_worktree_porcelain(&format!("{good}\n{bad}"));
            assert_eq!(entries.len(), 2, "{why}");
            assert!(!entries[0].unknown, "{why}: the sound entry stays known");
            assert!(entries[1].unknown, "{why}");
        }
        let twice = format!("{good}\n{good}");
        let entries = super::parse_worktree_porcelain(&twice);
        assert!(
            entries.iter().all(|entry| entry.unknown),
            "a path listed twice"
        );
    }

    #[test]
    fn the_reflog_selector_is_strict() {
        assert_eq!(
            super::parse_reflog_selector("HEAD@{1700000000}\n"),
            Some(1_700_000_000)
        );
        assert_eq!(super::parse_reflog_selector("HEAD@{0}"), Some(0));
        for bad in [
            "",
            "\n",
            "HEAD@{}\n",
            "HEAD@{-1}\n",
            "HEAD@{1 }\n",
            "HEAD@{1}\n\n",
            "HEAD@{1}\nHEAD@{2}\n",
            "main@{1}\n",
            "HEAD@{2026-01-01}\n",
            "HEAD@{99999999999999999999999}\n",
        ] {
            assert_eq!(super::parse_reflog_selector(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_whole_cache_directories_are_cache_entries() {
        for cache in [
            "target/",
            ".venv/",
            "node_modules/",
            "fuzz/target/",
            "a/b/.venv/",
        ] {
            assert!(super::is_cache_entry(cache), "{cache}");
        }
        for data in [
            "target",
            ".local/",
            "fuzz/",
            "target/x",
            "a.bak",
            "mutants.out/",
            "/",
            "",
            "targets/",
            "target/../",
        ] {
            assert!(!super::is_cache_entry(data), "{data}");
        }
    }

    #[test]
    fn an_index_row_names_its_defect() {
        let ok = format!("H 100644 {SHA} 0\ta b");
        assert!(matches!(super::index_row(&ok), Ok((SHA, "a b"))));
        assert!(super::index_row(&format!("H 100755 {SHA} 0\tx")).is_ok());
        for (row, flags) in [
            (format!("h 100644 {SHA} 0\tx"), true),
            (format!("S 100644 {SHA} 0\tx"), true),
            (format!("H 100644 {SHA} 1\tx"), true),
            (format!("H 120000 {SHA} 0\tx"), false),
            (format!("H 160000 {SHA} 0\tx"), false),
        ] {
            let defect = super::index_row(&row).err();
            assert!(
                matches!(
                    (defect, flags),
                    (Some(super::IndexDefect::Flags), true)
                        | (Some(super::IndexDefect::Mode), false)
                ),
                "{row}"
            );
        }
        for row in [
            String::new(),
            "H".to_owned(),
            format!("H 100644 {SHA} 0 x\ty"),
            format!("H 100644 {SHA}\tx"),
            "H 100644 abc 0\tx".to_owned(),
            format!("H 100644 {SHA} 0\t"),
            format!("H 100644 {SHA} 0 x"),
        ] {
            assert!(
                matches!(super::index_row(&row), Err(super::IndexDefect::Malformed)),
                "{row:?}"
            );
        }
    }
}
