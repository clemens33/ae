//! The archive preview's git facts, derived by running `git` — the ONLY product
//! caller of [`crate::transport::run_git`], the fixed-program git leg of the one
//! process door.
//!
//! This is a faithful port of the frozen `_ar_git_head` and `_ar_git_range`,
//! which a non-local (`worktree`/`copy`) preview runs in the session's work dir.
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

/// The four questions the preview asks git, each a fixed argv shape. A typed
/// query rather than free-form strings so a caller cannot assemble an arbitrary
/// git command line, and so [`argv`] is the one place the wire form is decided.
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
    /// printed value. `--quiet` so a detached HEAD is a silent failure rather
    /// than a diagnostic on stderr the watchdog would have to filter.
    Branch,
    /// `rev-parse --short HEAD` — the detached-HEAD fallback the frozen branch
    /// segment reaches for when `symbolic-ref` names nothing (ae:13866).
    ShortHead,
    /// `status --porcelain --untracked-files=no` — the dirty marker. Untracked
    /// files are deliberately EXCLUDED: a build artifact nobody committed is not
    /// a modified work tree, and the bar would otherwise read `*` forever.
    PorcelainStatus,
    /// `worktree remove --force <worktree>` — the git-mode teardown's workdir
    /// commit, judged by exit status. Run with `-C <origin>`. A successful remove
    /// deletes that worktree's working directory AND its admin record in origin;
    /// `prune` is deliberately NOT part of this path (it is global housekeeping
    /// over unrelated stale entries and must never make a removed workdir look
    /// retained).
    WorktreeRemove { worktree: &'a OsStr },
    /// `worktree prune` — the frozen end path's housekeeping after a managed
    /// worktree is removed. Best-effort: its exit status is deliberately ignored.
    WorktreePrune,
    /// `worktree add --detach <worktree> HEAD` — the `--worktree` launch's
    /// working copy, run with `-C <origin>`. Detached deliberately: ae manages
    /// no branch at creation, and a named branch would collide the second time
    /// the same origin launches a session.
    WorktreeAdd { worktree: &'a OsStr },
    /// `add -A` — the end path's stage-everything before the session commit.
    AddAll,
    /// `commit -m <subject> -m <body>` — the frozen two-`-m` session commit.
    Commit { subject: &'a str, body: &'a str },
    /// `diff --quiet` — unstaged changes, judged by exit status (non-zero = dirty).
    DiffQuiet,
    /// `diff --cached --quiet` — staged changes, same judgement.
    DiffCachedQuiet,
    /// `ls-files --others --exclude-standard` — untracked, judged by output.
    LsFilesOthers,
    /// `remote get-url origin` — whether a push target exists at all.
    RemoteGetUrl,
    /// `fetch origin --quiet` — the frozen silent refresh before the reachability
    /// test. Best-effort: the frozen path swallows its failure.
    FetchOrigin,
    /// `branch -r --contains HEAD` — whether HEAD is already on a remote branch.
    BranchRemoteContains,
    /// `merge-base HEAD origin/HEAD` — the base the pushed-file count is taken from.
    MergeBaseOriginHead,
    /// `diff --name-only <base> HEAD` — the files the push carries.
    DiffNameOnly { base: &'a str },
    /// `push -u origin HEAD:refs/heads/<branch>` — the end path's push, judged by
    /// exit status. The branch rides as its own argv element.
    PushHead { branch: &'a str },
}

/// A git argv minted ONLY by this module's [`argv`] builder. Its inner vector
/// is private, so no other module can fabricate an arbitrary git command line
/// and hand it to [`crate::transport::run_git`] — the transport door runs a
/// `GitArgv`, but only `src/git.rs` can construct one from a typed [`Query`].
/// This is the boundary a grep guard cannot give: an alias-import of `run_git`
/// is useless without a `GitArgv`, and a `GitArgv` cannot be built from raw argv
/// anywhere but here.
pub(crate) struct GitArgv(Vec<OsString>);

impl GitArgv {
    /// The OS-native argv for the transport door to spawn. Reading is harmless;
    /// construction is what is sealed.
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
        Query::WorktreeRemove { worktree } => {
            args.push("worktree".into());
            args.push("remove".into());
            args.push("--force".into());
            args.push(worktree.to_owned());
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
    }
    GitArgv(args)
}

/// A commit is a value only as exactly 40 lowercase-hex, the frozen
/// `^[0-9a-f]{40}$`.
fn is_hex40(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A count is a value only as one-or-more ASCII digits, the frozen `^[0-9]+$`.
fn is_count(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `_ar_git_head <wdir>`: the 40-hex HEAD of the work tree at `wdir`, or `-`.
///
/// An empty path is `-` without running git (the frozen `[[ -n "$wdir" ]]`). The
/// work-tree guard is judged by exit status, so a bare repository (which answers
/// `false` at exit zero) proceeds and is then rejected by the 40-hex interpreter
/// on its literal `HEAD` output — the same two-step the frozen reader takes.
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
///
/// Both endpoints must be 40-hex and `base` must be an ancestor of `tip`
/// (`merge-base --is-ancestor`, judged by exit status — a rewritten or unrelated
/// base fails it), then the count must parse. Any miss is `("-", "-")`.
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

/// `git -C <origin> worktree remove --force <worktree>` — remove the exact managed
/// git worktree of a `mode=git` teardown. Judged by exit status: `true` iff git
/// exited zero, at which point that worktree's working directory AND its admin
/// record in `origin` are gone. `origin` and `worktree` ride as RAW bytes → one
/// `OsStr` argv element each, so a hostile path is data to git, never a command
/// line, and a non-UTF-8 path survives intact. A git that refuses (a locked or
/// dirty worktree, a path that is not a registered worktree) returns `false` with
/// NO `rm -rf` fallback — the caller reclassifies the managed child and retains the
/// session state. Git mutates only `origin`'s worktree ADMIN metadata; origin's
/// checked-out content is never touched and origin is never a deletion target.
pub(crate) fn worktree_remove(origin: &[u8], worktree: &[u8]) -> bool {
    let origin = OsStr::from_bytes(origin);
    let worktree = OsStr::from_bytes(worktree);
    crate::transport::run_git(&argv(origin, &Query::WorktreeRemove { worktree })).0
}

/// The watchdog's branch segment: the branch NAME at `wdir`, or its short HEAD
/// when HEAD is detached, or `None`.
///
/// `None` for an empty path, a path that is not inside a work tree (judged by
/// exit status, as the frozen `_watchdog_branch_segment` does at ae:13859), and
/// a HEAD that names nothing. The value is trimmed of surrounding whitespace and
/// otherwise passed through verbatim — the DISPLAY trim belongs to the caller,
/// because `@ae_branch_name` is the machine value and must not carry one.
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

/// Whether the work tree at `wdir` has TRACKED modifications — the `*` the
/// watchdog appends to the displayed branch.
///
/// A failed run is `false`: an unreadable work tree is not a dirty one, and a
/// bar that claims uncommitted work on the strength of a git that did not answer
/// is worse than a bar that says nothing.
pub(crate) fn work_tree_dirty(wdir: &[u8]) -> bool {
    if wdir.is_empty() {
        return false;
    }
    let wdir = OsStr::from_bytes(wdir);
    let (succeeded, out) = crate::transport::run_git(&argv(wdir, &Query::PorcelainStatus));
    succeeded && out.lines().any(|line| !line.is_empty())
}

// ---- the end path's git leg ------------------------------------------------
//
// `ae end` commits and pushes a managed session's work before anything is
// deleted. Every call below is one fixed [`Query`] through the same sealed
// [`GitArgv`] door, so the whole leg adds argv SHAPES and no new capability:
// the work-tree path stays one OS-native element after `-C`, and a branch or
// commit message rides as its own element with no shell anywhere.

/// Whether `wdir` is inside a git work tree — the frozen end path's repo
/// precondition, judged by exit status exactly as `git -C … rev-parse
/// --is-inside-work-tree >/dev/null 2>&1` is.
pub(crate) fn is_work_tree(wdir: &[u8]) -> bool {
    !wdir.is_empty()
        && crate::transport::run_git(&argv(OsStr::from_bytes(wdir), &Query::IsWorkTree)).0
}

/// Whether the work tree has anything to commit: unstaged changes, staged
/// changes, or untracked files that are not ignored.
///
/// The frozen test is three commands OR-ed, and the untracked leg is judged by
/// OUTPUT rather than status — `ls-files` exits zero either way.
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

/// `git -C <wdir> add -A` then `commit -m <subject> -m <body>`. `true` only if
/// BOTH exited zero — a failed stage must never look like a successful commit.
pub(crate) fn commit_all(wdir: &[u8], subject: &str, body: &str) -> bool {
    let path = OsStr::from_bytes(wdir);
    if !crate::transport::run_git(&argv(path, &Query::AddAll)).0 {
        return false;
    }
    crate::transport::run_git(&argv(path, &Query::Commit { subject, body })).0
}

/// Whether a remote named `origin` is configured — the frozen
/// `git remote get-url origin >/dev/null 2>&1`.
pub(crate) fn has_origin(wdir: &[u8]) -> bool {
    crate::transport::run_git(&argv(OsStr::from_bytes(wdir), &Query::RemoteGetUrl)).0
}

/// `git fetch origin --quiet`, best-effort. The frozen path swallows its
/// failure (`|| true`): an offline end still commits and still archives.
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

/// How many files the unpushed commits touch, as the frozen line prints it: the
/// count of `diff --name-only <merge-base HEAD origin/HEAD> HEAD`, with `HEAD~1`
/// standing in when no merge base resolves.
///
/// A count, never an error: the frozen path prints `?` when git cannot answer,
/// and this is a cosmetic line above a push that runs either way.
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
/// status. `false` is the frozen refusal: the session is STOPPED, the commit is
/// safe locally, and nothing is deleted.
pub(crate) fn push_head(wdir: &[u8], branch: &str) -> bool {
    crate::transport::run_git(&argv(OsStr::from_bytes(wdir), &Query::PushHead { branch })).0
}

/// `git -C <origin> worktree prune`, best-effort — the frozen housekeeping after
/// a managed worktree goes. Its status is ignored on purpose: pruning unrelated
/// stale entries must never fail an end whose own removal succeeded.
pub(crate) fn worktree_prune(origin: &[u8]) {
    let _ = crate::transport::run_git(&argv(OsStr::from_bytes(origin), &Query::WorktreePrune));
}

/// `git -C <origin> worktree add --detach <worktree> HEAD` — the `--worktree`
/// launch's working copy. Judged by exit status.
///
/// A first refusal is retried ONCE behind [`worktree_prune`], which is the
/// frozen recovery (`ae:13310`): a previous session whose directory was removed
/// without `worktree remove` leaves a stale admin record, and `add` refuses the
/// path until it is pruned. The prune is global housekeeping over unrelated
/// stale entries, which is why it runs only on the retry and never on the
/// happy path.
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
    use super::{GitArgv, Query, argv, is_count, is_hex40};
    use std::ffi::OsString;

    fn strs(args: &GitArgv) -> Vec<String> {
        args.as_os_args()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
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
}
