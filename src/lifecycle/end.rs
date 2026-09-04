//! `_end`: the whole end operation.
//!
//! CAPTURE THEN DELETE, and the order is the contract. The archive is published
//! AFTER the session is verifiably stopped and after git has had its say, and
//! BEFORE any live state is removed — so a failed archive returns non-zero with
//! the whole session still on disk. ae never deletes a session it could not
//! capture.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::archive;
use crate::config::{Workspace, read_workspace};
use crate::inventory::ServerId;
use crate::meta::{self, Selector, ServerSelector};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;
use crate::tmux::StopProbe;
use crate::transport;

use super::{
    all_sessions, dir_exists, kill_verified, live_id, lock, meta_value, name_is_usable,
    path_exists, server_of, sessions_dir, worktrees_dir,
};

/// The frozen usage line.
const USAGE: &str =
    "Usage: _end [-f] [--purge-history|--keep-history] [--assume-stopped] <session-name|all>";

/// What `ae end` will do with a session's memory — the frozen
/// `_end_archive_plan`'s five answers.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    /// The archive is published, keyed by the session's recorded id.
    Keep,
    /// No archive is created, and one already there is deleted.
    Purge,
    /// Nothing to archive — a leftover directory with no session memory.
    Nothing,
    /// There IS state, but it cannot be archived: end must refuse.
    Unavailable,
}

/// One target's resolved plan. The FROZEN form of this is what the human is
/// held to: a plan that changed between the confirmation and the act is an
/// answer given to a different question, and the end refuses rather than carry
/// out the other one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Plan {
    action: Action,
    detail: String,
    purge: bool,
}

impl Plan {
    /// The target's line in the confirmation list — BOTH actions, per target,
    /// because "unless a session's own config says" is a sentence about no
    /// particular session and this prompt is the last thing between a human and
    /// a delete.
    fn line(&self, name: &str) -> String {
        let history = if self.purge {
            "conversation files DELETED"
        } else {
            "conversation files KEPT"
        };
        let detail = &self.detail;
        match self.action {
            Action::Keep => format!("  - {name}: archive -> {detail}/ · {history}"),
            Action::Purge => {
                format!("  - {name}: NO archive, and {detail}/ is DELETED if it exists · {history}")
            }
            Action::Nothing => format!("  - {name}: nothing to archive ({detail}) · {history}"),
            Action::Unavailable => {
                format!("  - {name}: CANNOT be archived ({detail}) — end will refuse it")
            }
        }
    }
}

/// What the argv said.
struct Args {
    target: String,
    force: bool,
    assume_stopped: bool,
    /// `Some(true)` for `--purge-history`, `Some(false)` for `--keep-history`,
    /// `None` when the caller passed neither and each session's OWN config
    /// decides.
    purge_cli: Option<bool>,
}

/// `_end [-f] [--purge-history|--keep-history] [--assume-stopped] <name|all>`.
pub(crate) fn run(
    root: &Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let args = match parse(tail) {
        Ok(args) => args,
        Err(message) => {
            writeln!(err, "{message}")?;
            return Ok(EXIT_USAGE);
        }
    };
    // The stopped acknowledgement is PER-TARGET destructive intent — never
    // valid for 'all'.
    if args.target == "all" && args.assume_stopped {
        writeln!(
            err,
            "Error: --assume-stopped is per-target only — not valid with 'all'."
        )?;
        return Ok(EXIT_USAGE);
    }

    // RESOLVE BEFORE PROMPTING. A destructive confirm must never fire on an
    // unresolved target: `ae end <bogus>` must error here, before any prompt.
    let targets: Vec<String> = if args.target == "all" {
        all_sessions(root)
    } else {
        if !name_is_usable(root, &args.target) {
            writeln!(err, "Session '{}' not found.", args.target)?;
            return Ok(EXIT_FAILED);
        }
        let dir = sessions_dir(root).join(&args.target);
        if !dir_exists(&dir) && !dir_exists(&worktrees_dir(root).join(&args.target)) {
            writeln!(err, "Session '{}' not found.", args.target)?;
            return Ok(EXIT_FAILED);
        }
        vec![args.target.clone()]
    };

    // ONE resolution: the prompt renders from these fields and the frozen
    // contract is built from the same ones. Resolving a second time to populate
    // the contract made the human's answer and the end's contract two different
    // observations, and every later gate then compared against the wrong one.
    let frozen: Vec<(String, Plan)> = targets
        .iter()
        .map(|name| {
            let plan = resolve_plan(root, name, args.purge_cli);
            (name.clone(), plan)
        })
        .collect();

    if !args.force {
        // Set REGARDLESS of how many targets there were: the human was asked,
        // and what they were asked about is now the whole of what may be ended.
        confirm_body(root, &args, &frozen, err)?;
        write!(err, "Continue? [y/N] ")?;
        err.flush()?;
        match read_reply() {
            Some(reply) if reply.starts_with('y') || reply.starts_with('Y') => {}
            Some(_) => {
                writeln!(err, "Aborted.")?;
                return Ok(0);
            }
            None => {
                writeln!(
                    err,
                    "Error: could not obtain confirmation — no input on stdin."
                )?;
                writeln!(
                    err,
                    "  Nothing was stopped and nothing was deleted. Run it from a terminal, or"
                )?;
                writeln!(err, "  pass -f if you mean to proceed without being asked.")?;
                return Ok(EXIT_FAILED);
            }
        }
    }

    if frozen.is_empty() {
        writeln!(out, "No ae sessions.")?;
        return Ok(0);
    }

    // EXACTLY the list the human was shown. Re-enumerating here would let a
    // session that appeared between the prompt and the answer be ended without
    // ever having been named.
    let mut failures = 0_u32;
    for (name, plan) in &frozen {
        // `-f` freezes nothing, because nothing was promised.
        let contract = if args.force { None } else { Some(plan) };
        if !end_one(root, name, &args, contract, out, err)? {
            failures += 1;
        }
    }
    if failures > 0 {
        writeln!(
            err,
            "{failures} session(s) failed to end. See errors above."
        )?;
        return Ok(EXIT_FAILED);
    }
    Ok(0)
}

fn parse(tail: &[String]) -> Result<Args, String> {
    let mut args = Args {
        target: String::new(),
        force: false,
        assume_stopped: false,
        purge_cli: None,
    };
    for arg in tail {
        match arg.as_str() {
            "-f" | "--force" => args.force = true,
            "--assume-stopped" => args.assume_stopped = true,
            "--purge-history" => args.purge_cli = Some(true),
            "--keep-history" => args.purge_cli = Some(false),
            flag if flag.starts_with('-') => {
                return Err(format!(
                    "Error: unknown flag '{flag}'. Use -f, --purge-history, --keep-history, --assume-stopped."
                ));
            }
            name if args.target.is_empty() => name.clone_into(&mut args.target),
            // Destructive command — never silently drop a stray arg.
            extra => {
                return Err(format!(
                    "Error: unexpected extra argument '{extra}' — _end takes one session name (or 'all')."
                ));
            }
        }
    }
    if args.target.is_empty() {
        return Err(USAGE.to_owned());
    }
    Ok(args)
}

/// Read the confirmation. A LINE, not a raw single keystroke: the frozen
/// `read -r -n 1` needs a terminal in raw mode, which this crate cannot enter
/// (`unsafe_code = "forbid"` closes the termios route). `None` is EOF — a
/// caller with no stdin, which is never consent.
fn read_reply() -> Option<String> {
    let mut buffer = String::new();
    // A LINE, not to EOF: a human at a terminal answers and presses Enter, and
    // reading to EOF there would block forever waiting for a ^D they were never
    // asked for. An empty read and a failed read are the same answer — nobody
    // replied.
    match std::io::stdin().read_line(&mut buffer) {
        Ok(read) if read > 0 => Some(buffer.trim_start().to_owned()),
        _ => None,
    }
}

/// The confirmation body — on STDERR, so `_end`'s stdout carries only what
/// actually happened.
fn confirm_body(
    root: &Path,
    args: &Args,
    frozen: &[(String, Plan)],
    err: &mut impl Write,
) -> io::Result<()> {
    if args.target == "all" {
        writeln!(err, "This will END every ae session:")?;
    } else {
        writeln!(err, "This will END the session:")?;
    }
    for (name, plan) in frozen {
        writeln!(err, "{}", plan.line(name))?;
    }
    if frozen.is_empty() {
        writeln!(err, "  (none)")?;
    }
    writeln!(
        err,
        "Work in a managed workspace is committed and pushed to ae/<session> first; the session is then removed from {}.",
        sessions_dir(root).display()
    )?;
    Ok(())
}

/// What `ae end` will do with this session's memory, resolved BEFORE the prompt
/// so the confirmation can name the exact path — the frozen `_end_archive_plan`.
///
/// A QUERY: it never writes.
fn resolve_plan(root: &Path, name: &str, purge_cli: Option<bool>) -> Plan {
    let dir = sessions_dir(root).join(name);
    let bytes = meta::read_bytes(&dir).unwrap_or_default();
    let purge = effective_purge(&bytes, purge_cli);
    let archive_root = root.join("archive");

    if meta::read_bytes(&dir).is_err() {
        // A missing meta is not the same as nothing to lose. `Nothing` is
        // reserved for a target that HAS no session memory. A directory still
        // carrying memo, events or request payloads but no meta cannot be
        // identified, so it is UNAVAILABLE and the end refuses: otherwise the
        // cleanup deletes that memory unread, which is the one outcome this
        // whole path exists to prevent.
        let has_memory = has_nonempty(&dir.join("memo.tsv"))
            || has_nonempty(&dir.join("events.jsonl"))
            || has_message_payload(&dir.join("messages"));
        return if has_memory {
            Plan {
                action: Action::Unavailable,
                detail: "its meta is missing, so ae cannot identify the session — but its memo, events or request payloads are still there".to_owned(),
                purge,
            }
        } else {
            Plan {
                action: Action::Nothing,
                detail: "no session memory to archive".to_owned(),
                purge,
            }
        };
    }

    let raw = meta_value(&bytes, "session_id");
    let aid = archive::canonical_uuid(&raw);
    // ABSENT vs CORRUPT, decided BEFORE purge gets a say. A value ae cannot
    // parse is the only evidence of what went wrong with that session, and it
    // makes the session unidentifiable — a refusal whichever way the history
    // flag points.
    if aid.is_empty() && !raw.is_empty() {
        return Plan {
            action: Action::Unavailable,
            detail: format!(
                "its session_id ({raw}) is not a UUID, and ae will not overwrite a value it cannot parse"
            ),
            purge,
        };
    }
    if aid.is_empty() {
        // CLEAN CUT, as `_compact-freeze` already rules for the same state: a
        // session with no valid id is unsupported old state, refused with a
        // refresh/migrate instruction rather than minted an id on the way to
        // an immutable archive.
        return Plan {
            action: Action::Unavailable,
            detail: "it records no valid session id — refresh or migrate the session, then retry"
                .to_owned(),
            purge,
        };
    }
    if purge {
        return Plan {
            action: Action::Purge,
            detail: archive_root.join(&aid).display().to_string(),
            purge,
        };
    }
    Plan {
        action: Action::Keep,
        detail: archive_root.join(&aid).display().to_string(),
        purge,
    }
}

/// THE purge sensor — one definition, consulted by the confirmation prompt
/// (before the human answers) and by the cleanup (when it acts).
///
/// A CLI flag overrides globally; otherwise the answer comes from THIS
/// session's OWN config, hydrated from its meta `config` plus its origin's
/// local config, so a cross-repo end or `ae end all` honors each session's
/// policy rather than the caller's cwd. An unreadable config is `false`: the
/// frozen sensor's `get_config` miss is a keep, and a keep is the safe answer.
fn effective_purge(bytes: &[u8], purge_cli: Option<bool>) -> bool {
    if let Some(explicit) = purge_cli {
        return explicit;
    }
    workspace_of(bytes).is_some_and(|w| w.purge_agent_history)
}

/// The `[workspace]` block a session's own config resolves to — its recorded
/// global config layered under its origin's local `.ae/config`.
///
/// No usable stored config is NOT a fall back to the caller's cwd or global
/// (that would let an unrelated `purge = true` bleed in): only the session's
/// OWN origin overlay can opt into anything.
fn workspace_of(bytes: &[u8]) -> Option<Workspace> {
    let config = meta_value(bytes, "config");
    let origin = meta_value(bytes, "origin");
    let global = (!config.is_empty() && config != "/dev/null")
        .then(|| PathBuf::from(&config))
        .filter(|path| archive::regular_file(path));
    let local = (!origin.is_empty())
        .then(|| Path::new(&origin).join(".ae").join("config"))
        .filter(|path| archive::regular_file(path));
    read_workspace(global.as_deref(), local.as_deref()).ok()
}

fn has_nonempty(path: &Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the archive plan must know whether a meta-less session directory still holds memory before calling it unarchivable"
    )]
    let meta = std::fs::metadata(path);
    meta.is_ok_and(|m| m.is_file() && m.len() > 0)
}

fn has_message_payload(dir: &Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the same question as `has_nonempty`, over the request-payload directory"
    )]
    let entries = std::fs::read_dir(dir);
    entries.is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".txt"))
    })
}

// ---- one session -----------------------------------------------------------

/// End one session, under its lifecycle lock. `false` means the end did NOT
/// complete — and in every such case the session is still there.
///
/// THE LIFECYCLE LOCK is taken here, before the target is classified, and held
/// through the last removal: the classification decays otherwise, because the
/// window between the proof and the cleanup spans commit/fetch/push, and a
/// start or resume in that window made cleanup delete state under a freshly
/// LIVE session. A final recheck alone would still race.
#[allow(
    clippy::too_many_lines,
    reason = "the frozen order IS the contract; splitting it would put the sequence in two places"
)]
fn end_one(
    root: &Path,
    name: &str,
    args: &Args,
    contract: Option<&Plan>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<bool> {
    let Ok(_guard) = lock(root, name) else {
        writeln!(
            err,
            "Error: another lifecycle operation (start/resume/end) is in progress for '{name}' — retry shortly. State preserved."
        )?;
        return Ok(false);
    };
    let dir = sessions_dir(root).join(name);
    let bytes = meta::read_bytes(&dir).unwrap_or_default();

    // ══ THE INVARIANT: ae NEVER deletes session state unless
    //    (a) the target was positively identified on ITS OWN recorded server
    //        and its kill was verified, or
    //    (b) the human passed --assume-stopped for THIS single target, and the
    //        full enumerable sweep found nothing AND left no unverifiable
    //        socket standing, or
    //    (c) the target's POSITIVE record names a server that verifiably lacks
    //        the session.
    // Classification comes FIRST — before ANY server query — so a blank or
    // ambiguous record can never aim a lookup (or a kill) at a server it does
    // not own.
    let selector = server_of(&bytes);
    let mut session_id: Option<String> = None;
    let mut server = None;
    match selector {
        ServerSelector::Positive(sel) => {
            let id = ServerId::Selected(sel);
            session_id = live_id(&id, name);
            if session_id.is_none()
                && transport::verify_session_absent(&id, name) == StopProbe::Unknown
            {
                writeln!(
                    err,
                    "Error: cannot verify session '{name}' state (its recorded tmux server is unreachable) — state preserved."
                )?;
                return Ok(false);
            }
            server = Some(id);
        }
        ServerSelector::Missing | ServerSelector::Ambiguous => {
            // No positive ownership record. The ONLY teardown path is the
            // explicit per-target acknowledgement — and even that refuses when
            // the name is live anywhere enumerable, because the assumption
            // would be provably false.
            match sweep(root, name) {
                Sweep::Found(where_) => {
                    writeln!(
                        err,
                        "Error: session '{name}' has no positive server record AND is LIVE on enumerable server '{where_}'."
                    )?;
                    writeln!(
                        err,
                        "  Establish ownership first: 'ae doctor --refresh {name}'."
                    )?;
                    writeln!(err, "  State preserved.")?;
                    return Ok(false);
                }
                Sweep::Unsure(where_) => {
                    writeln!(
                        err,
                        "Error: cannot verify enumerable tmux server '{where_}' (socket exists but did not answer) — session '{name}' could be hiding behind it."
                    )?;
                    writeln!(
                        err,
                        "  Fix that server (permissions/staleness), then retry. State preserved."
                    )?;
                    return Ok(false);
                }
                Sweep::Clear => {}
            }
            if !args.assume_stopped {
                writeln!(
                    err,
                    "Error: session '{name}' has no positive server record (pre-fix or ambiguous meta) — it may be live somewhere ae cannot enumerate."
                )?;
                writeln!(
                    err,
                    "  Resolve: 'ae doctor --refresh {name}', or acknowledge it is stopped: ae end --assume-stopped {name}"
                )?;
                writeln!(err, "  State preserved.")?;
                return Ok(false);
            }
            writeln!(
                out,
                "Proceeding on explicit --assume-stopped acknowledgement (not live on any enumerable server, default included)."
            )?;
        }
    }

    let mode = meta_value(&bytes, "mode");
    let origin = meta_value(&bytes, "origin");
    let work_dir = resolve_workdir(root, &bytes, name);

    // A session ae cannot IDENTIFY is refused HERE, before anything is stopped:
    // the archive step would refuse it at the end anyway, and there is no
    // reason to stop a session for a failure already visible.
    let plan = resolve_plan(root, name, args.purge_cli);
    if plan.action == Action::Unavailable {
        writeln!(
            err,
            "Error: session '{name}' cannot be archived — {}.",
            plan.detail
        )?;
        writeln!(
            err,
            "  Nothing was stopped and NOTHING was deleted; ae does not delete a session it could not capture."
        )?;
        return Ok(false);
    }
    // A contract that no longer matches the one confirmed is caught here too,
    // not only at the archive step: the step is the backstop under the lock,
    // but by then the session has been stopped.
    if let Some(frozen) = contract
        && *frozen != plan
    {
        writeln!(
            err,
            "Error: what '{name}' would do changed between the confirmation and now — refusing to act on an answer given to a different question."
        )?;
        writeln!(err, "  Confirmed: {}", frozen.line(name))?;
        writeln!(err, "  Now:       {}", plan.line(name))?;
        writeln!(
            err,
            "  Nothing was stopped and nothing was deleted. Re-run 'ae end {name}' to see the current plan."
        )?;
        return Ok(false);
    }

    // Local mode manages no branch of its own, so the archive records the git
    // outcome honestly as not-managed rather than inventing a range from the
    // human's own checkout.
    if mode == "local" || mode.is_empty() {
        if let (Some(server), Some(id)) = (server.as_ref(), session_id.as_ref())
            && !kill_verified(server, name, "end", id, err)?
        {
            return Ok(false);
        }
        // CAPTURE BEFORE DELETE.
        if !archive_step(root, name, &plan, &Git::none(), out, err)? {
            return Ok(false);
        }
        if !cleanup(root, name, &bytes, &plan, false, out, err)? {
            return Ok(false);
        }
        writeln!(out, "Ended local session {name}")?;
        return Ok(true);
    }

    // git/full mode. THE ORDER IS THE FIX: verified stop, THEN snapshot.
    //
    // end used to add/commit/fetch/push while the agents were STILL LIVE and
    // only then kill. A write landing after the snapshot and before the kill
    // was neither committed nor left on disk — cleanup deleted the directory it
    // lived in.
    //
    // The repo precondition is checked BEFORE the stop, deliberately: refusing
    // here leaves the session running, and there is no reason to stop a session
    // for a failure we can see coming.
    let wdir_bytes = work_dir.as_os_str().as_encoded_bytes().to_vec();
    if !dir_exists(&work_dir) || !crate::git::is_work_tree(&wdir_bytes) {
        writeln!(
            err,
            "Error: working directory is not a git repo — cannot preserve work."
        )?;
        writeln!(
            err,
            "  Nothing was stopped and nothing was deleted; the session is still running."
        )?;
        writeln!(err, "  Working directory: {}", work_dir.display())?;
        return Ok(false);
    }
    // STOP FIRST — and verify it. Everything below this line snapshots a tree
    // nothing is writing to any more.
    if let (Some(server), Some(id)) = (server.as_ref(), session_id.as_ref())
        && !kill_verified(server, name, "end", id, err)?
    {
        return Ok(false);
    }

    if crate::git::has_pending_work(&wdir_bytes) {
        writeln!(out, "Committing changes in {name}...")?;
        let stamp = Timestamp::now().to_string();
        let human = stamp.replace('T', " ").replace('Z', " UTC");
        if !crate::git::commit_all(
            &wdir_bytes,
            &format!("ae: end session {name}"),
            &format!("Ended: {human}"),
        ) {
            writeln!(
                err,
                "Error: commit failed. The session is STOPPED; nothing was deleted."
            )?;
            writeln!(
                err,
                "  Your work, ae state and agent conversation files are all preserved."
            )?;
            writeln!(err, "  Working directory: {}", work_dir.display())?;
            writeln!(err, "  Fix the commit there, then re-run: ae end {name}")?;
            writeln!(err, "  Or pick the session back up: ae {name}")?;
            return Ok(false);
        }
    }

    // The push outcome is recorded by the branch that ACTUALLY runs and handed
    // to the archive — never rediscovered afterwards, when the remote may have
    // moved and the worktree may already be gone.
    let has_origin = crate::git::has_origin(&wdir_bytes);
    let mut git = Git::none();
    let mut preserve_workdir = false;
    if has_origin {
        crate::git::fetch_origin(&wdir_bytes);
    }
    if has_origin && !crate::git::head_is_on_a_remote(&wdir_bytes) {
        let branch = sanitize_branch_name(name);
        writeln!(out, "Pushing to origin/{branch}...")?;
        writeln!(
            out,
            "  {} file(s) changed",
            crate::git::pushed_file_count(&wdir_bytes)
        )?;
        if !crate::git::push_head(&wdir_bytes, &branch) {
            writeln!(
                err,
                "Error: push failed. The session is STOPPED; nothing was deleted."
            )?;
            writeln!(
                err,
                "  The commit is safe locally — ae state and the working tree are preserved."
            )?;
            writeln!(err, "  Working directory: {}", work_dir.display())?;
            writeln!(
                err,
                "  To retry the push: cd {} && git push -u origin HEAD:refs/heads/{branch}",
                work_dir.display()
            )?;
            writeln!(err, "  Then finish up:    ae end {name}")?;
            writeln!(err, "  Or pick the session back up: ae {name}")?;
            return Ok(false);
        }
        writeln!(out, "Pushed to origin/{branch}")?;
        "pushed".clone_into(&mut git.outcome);
        git.push_ref = format!("origin/{branch}");
        git.workdir = work_dir.display().to_string();
    } else if has_origin {
        writeln!(out, "No new commits to push.")?;
        "already-reachable".clone_into(&mut git.outcome);
        git.workdir = work_dir.display().to_string();
    } else {
        // B3 durability: a no-remote git target just COMMITTED work that exists
        // ONLY in this directory. Removing it would silently destroy the work
        // the prompt promised to keep — preserve the directory, remove only
        // ae's session state.
        writeln!(out, "No remote 'origin' — skipping push.")?;
        preserve_workdir = true;
        "no-origin".clone_into(&mut git.outcome);
        git.preserved = work_dir.display().to_string();
        git.workdir = work_dir.display().to_string();
    }

    // The last thing before the session stops existing: capture it, or fail and
    // keep it.
    if !archive_step(root, name, &plan, &git, out, err)? {
        return Ok(false);
    }
    if !cleanup(root, name, &bytes, &plan, preserve_workdir, out, err)? {
        return Ok(false);
    }
    if preserve_workdir {
        writeln!(
            out,
            "Ended {name} — work committed locally (no origin remote to push to)."
        )?;
        writeln!(out, "  Directory preserved: {}", work_dir.display())?;
    } else {
        writeln!(out, "Ended {name}")?;
    }
    let _ = (origin, mode);
    Ok(true)
}

/// The operation facts the archive publisher needs and the core does not derive
/// itself.
struct Git {
    outcome: String,
    push_ref: String,
    preserved: String,
    workdir: String,
}

impl Git {
    /// Local mode manages no branch of its own.
    fn none() -> Self {
        Self {
            outcome: "not-managed".to_owned(),
            push_ref: "-".to_owned(),
            preserved: "-".to_owned(),
            workdir: String::new(),
        }
    }
}

/// Publish the archive (or perform the purge) for a session that is verifiably
/// stopped, BEFORE any live state is removed.
///
/// `false` means the end must stop with everything still on disk.
fn archive_step(
    root: &Path,
    name: &str,
    plan: &Plan,
    git: &Git,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<bool> {
    let dir = sessions_dir(root).join(name);
    match plan.action {
        Action::Nothing => Ok(true),
        Action::Unavailable => {
            writeln!(
                err,
                "Error: session '{name}' cannot be archived — {}.",
                plan.detail
            )?;
            writeln!(
                err,
                "  The session is STOPPED and NOTHING was deleted; ae does not delete a session it could not capture."
            )?;
            Ok(false)
        }
        Action::Purge => {
            let bytes = meta::read_bytes(&dir).unwrap_or_default();
            let aid = archive::canonical_uuid(&meta_value(&bytes, "session_id"));
            if aid.is_empty() {
                writeln!(
                    out,
                    "No archive written (--purge-history); this session never had an id to key one."
                )?;
                return Ok(true);
            }
            let parent = meta_value(&bytes, "parent_archive_id");
            let mut captured = Vec::new();
            let code = archive::purge::run(&dir, &aid, name, &parent, &mut captured, err)?;
            if code != 0 {
                // The purge already emitted the precise state — pre-commit:
                // nothing deleted; post-commit: PURGE INCOMPLETE. Do NOT
                // re-assert "nothing was deleted" here: that is false for an
                // incomplete purge.
                writeln!(
                    err,
                    "Error: purging the archive for '{name}' failed (see above) — the session is STOPPED."
                )?;
                return Ok(false);
            }
            let removed = String::from_utf8_lossy(&captured).trim().to_owned();
            if removed.is_empty() {
                writeln!(
                    out,
                    "No archive written (--purge-history); none existed for {aid}."
                )?;
            } else {
                writeln!(out, "Purged archive {aid}")?;
                writeln!(out, "  removed {removed}")?;
            }
            Ok(true)
        }
        Action::Keep => {
            let bytes = meta::read_bytes(&dir).unwrap_or_default();
            let aid = archive::canonical_uuid(&meta_value(&bytes, "session_id"));
            let archived_at = Timestamp::now().to_string();
            let ops = archive::publish::Ops {
                push_outcome: &git.outcome,
                push_ref: if git.push_ref.is_empty() {
                    "-"
                } else {
                    &git.push_ref
                },
                preserved: if git.preserved.is_empty() {
                    "-"
                } else {
                    &git.preserved
                },
                workdir: &git.workdir,
                archived_at: &archived_at,
            };
            let mut captured = Vec::new();
            let code = archive::publish::run(&dir, &ops, &mut captured, err)?;
            if code != 0 {
                writeln!(
                    err,
                    "Error: archiving session '{name}' failed — the session is STOPPED and NOTHING was deleted."
                )?;
                writeln!(
                    err,
                    "  Fix the cause reported above, then re-run: ae end {name}"
                )?;
                return Ok(false);
            }
            let line = String::from_utf8_lossy(&captured);
            let mut fields = line.trim_end_matches('\n').split('\t');
            let path = fields.next().unwrap_or_default();
            let files = fields.next().unwrap_or_default();
            let count = fields.next().unwrap_or_default();
            writeln!(out, "Archived {aid}")?;
            writeln!(out, "  {path}")?;
            writeln!(out, "  {files} file(s), {count} byte(s)")?;
            Ok(true)
        }
    }
}

/// Remove the live session — the frozen `cleanup_session`.
///
/// The FROZEN answer wins when a human confirmed one: re-resolving the purge
/// policy here would let a config edit made during the prompt delete
/// conversation files the human was told would be kept.
fn cleanup(
    root: &Path,
    name: &str,
    bytes: &[u8],
    plan: &Plan,
    preserve: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<bool> {
    let dir = sessions_dir(root).join(name);
    let mode = meta_value(bytes, "mode");
    let origin = meta_value(bytes, "origin");

    // Read meta BEFORE the removal below.
    if plan.purge {
        purge_conversation_files(root, bytes, out, err)?;
    } else {
        writeln!(
            out,
            "  kept agent conversation files (claude/codex/agy token history; purge: ae end --purge-history)"
        )?;
    }

    // The core owns the removal — a rename-to-tombstone commit boundary that
    // clears the canonical name atomically and reports success only after the
    // removal is durable. A teardown that RAN and FAILED has left the session
    // recoverable under its tombstone and printed the state: fail loudly, do
    // NOT delete more and do NOT print an end line.
    let valid = super::name_is_valid(name);
    if (mode == "local" || mode.is_empty()) && valid {
        if crate::teardown::run(&dir, out, err)? != 0 {
            writeln!(
                err,
                "Error: the session state for '{name}' was not fully removed — see the teardown diagnostic above; the session is NOT ended."
            )?;
            if plan.purge {
                writeln!(
                    err,
                    "  The agent conversation history WAS purged as requested; only the session-state removal did not complete."
                )?;
            }
            return Ok(false);
        }
        // Legacy cleanup: the old worktree-nested and origin-nested paths.
        remove_legacy(root, name, &origin);
        if preserve {
            writeln!(
                out,
                "Removed ae session state for {name} (working directory preserved)"
            )?;
        } else {
            writeln!(out, "Cleaned up local session {name}")?;
        }
        return Ok(true);
    }
    if (mode == "full" || mode == "git") && valid {
        // The core owns BOTH the managed workdir AND the canonical state
        // (workdir first, canonical last).
        if crate::teardown::run_nonlocal(&dir, preserve, out, err)? != 0 {
            writeln!(
                err,
                "Error: the teardown of '{name}' did not complete — see the diagnostic above; the session is NOT ended."
            )?;
            if plan.purge {
                writeln!(
                    err,
                    "  The agent conversation history WAS purged as requested; only the teardown did not complete."
                )?;
            }
            return Ok(false);
        }
        // The core already removed the managed workdir together with the
        // canonical state; it owns BOTH resources. Return BEFORE the legacy
        // cleanup so those removals can never delete a subtree of a preserved
        // workdir or touch `origin/.ae/<name>`.
        if mode == "git" && !origin.is_empty() {
            crate::git::worktree_prune(origin.as_bytes());
        }
        if !preserve {
            writeln!(out, "Removed worktree for {name}")?;
        }
        return Ok(true);
    }
    // A grammar-invalid legacy name: removed here rather than by the core, so
    // it is never made un-endable.
    let _ = std::fs::remove_dir_all(&dir);
    remove_legacy(root, name, &origin);
    if preserve {
        writeln!(
            out,
            "Removed ae session state for {name} (working directory preserved)"
        )?;
    } else {
        writeln!(out, "Cleaned up local session {name}")?;
    }
    Ok(true)
}

/// The pre-canonical state locations, removed best-effort.
fn remove_legacy(root: &Path, name: &str, origin: &str) {
    let _ = std::fs::remove_dir_all(worktrees_dir(root).join(name).join(".ae").join(name));
    if !origin.is_empty() {
        let nested = Path::new(origin).join(".ae").join(name);
        if dir_exists(&nested) {
            let _ = std::fs::remove_dir_all(nested);
        }
    }
}

/// Delete the agent CLI conversation files this session's captured harness ids
/// name — the frozen `_cleanup_agent_session_files`.
///
/// Opt-in by construction: these are the only local per-session record of token
/// usage, so they are KEPT unless the resolved policy says otherwise.
fn purge_conversation_files(
    root: &Path,
    bytes: &[u8],
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<()> {
    let text = String::from_utf8_lossy(bytes);
    let parsed = meta::Meta::parse(&text);
    let home = root.parent().map(Path::to_path_buf);
    for entry in parsed.roster() {
        let uuid = entry.harness_session.as_deref().unwrap_or_default();
        if uuid.is_empty() || uuid == "pending" {
            continue;
        }
        let tool = entry.binary.as_deref().unwrap_or_default();
        let Some(home) = home.as_ref() else { continue };
        match tool {
            "claude" => {
                if let Some(file) = find_claude_file(home, uuid)
                    && std::fs::remove_file(&file).is_ok()
                {
                    writeln!(out, "  removed claude conversation: {}", file.display())?;
                }
            }
            "codex" => {
                if let Some(file) = find_codex_file(home, uuid)
                    && std::fs::remove_file(&file).is_ok()
                {
                    writeln!(out, "  removed codex rollout: {}", file.display())?;
                }
            }
            // agy names its conversation EXACTLY: one file per id, in one flat
            // directory, so there is no lookup to implement and no ambiguity to
            // refuse. The `-wal`/`-shm` siblings go with it — left behind they
            // are an orphaned write-ahead log for a database that no longer
            // exists, which is the same retention leak as the database itself.
            "agy" => {
                for file in agy_conversation_files(home, uuid) {
                    if std::fs::remove_file(&file).is_ok() {
                        writeln!(out, "  removed agy conversation: {}", file.display())?;
                    }
                }
            }
            "gemini" | "opencode" => {
                writeln!(
                    err,
                    "  note: {tool} conversation file for slot {} left in place (lookup not yet implemented)",
                    entry.slot
                )?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// agy's conversation database for `uuid`, and the `SQLite` sidecars beside it.
///
/// No search and no ambiguity check, unlike the claude and codex finders: agy
/// keeps one file per conversation NAMED for the id in one flat directory, so
/// the path is computed rather than looked for, and only paths that exist are
/// returned. `-wal` and `-shm` are appended to the FULL file name (`<id>.db-wal`
/// is what `SQLite` writes), which is why they are built from the database's own
/// name rather than from a second extension.
fn agy_conversation_files(home: &Path, uuid: &str) -> Vec<PathBuf> {
    let store = home.join(crate::session_launch::capture::AGY_CONVERSATIONS);
    [
        format!("{uuid}.db"),
        format!("{uuid}.db-wal"),
        format!("{uuid}.db-shm"),
    ]
    .into_iter()
    .map(|name| store.join(name))
    .filter(|path| path_exists(path))
    .collect()
}

/// `~/.claude/projects/*/<uuid>.jsonl`, and only when exactly one matches.
fn find_claude_file(home: &Path, uuid: &str) -> Option<PathBuf> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the frozen conversation-file purge enumerates ~/.claude/projects"
    )]
    let entries = std::fs::read_dir(home.join(".claude").join("projects")).ok()?;
    let mut hits: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path().join(format!("{uuid}.jsonl")))
        .filter(|path| path_exists(path))
        .collect();
    if hits.len() == 1 { hits.pop() } else { None }
}

/// `~/.codex/sessions/Y/M/D/*<uuid>*.jsonl`, and only when exactly one matches.
fn find_codex_file(home: &Path, uuid: &str) -> Option<PathBuf> {
    fn children(path: &Path) -> Vec<PathBuf> {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the frozen conversation-file purge walks ~/.codex/sessions/Y/M/D"
        )]
        let entries = std::fs::read_dir(path);
        entries.map_or_else(
            |_| Vec::new(),
            |entries| entries.flatten().map(|entry| entry.path()).collect(),
        )
    }
    let mut level = vec![home.join(".codex").join("sessions")];
    for _ in 0..3 {
        level = level.iter().flat_map(|path| children(path)).collect();
    }
    let mut hits: Vec<PathBuf> = level
        .into_iter()
        .filter(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            name.contains(uuid) && name.ends_with(".jsonl")
        })
        .collect();
    if hits.len() == 1 { hits.pop() } else { None }
}

/// Where a session's work is — its recorded `work_dir`, else the standard
/// worktree path.
fn resolve_workdir(root: &Path, bytes: &[u8], name: &str) -> PathBuf {
    let recorded = meta_value(bytes, "work_dir");
    if !recorded.is_empty() {
        let path = PathBuf::from(&recorded);
        if dir_exists(&path) {
            return path;
        }
    }
    worktrees_dir(root).join(name)
}

/// `sanitize_branch_name`: strip an `ae-` prefix, replace everything outside
/// `[A-Za-z0-9._-]` with `-`, collapse repeats, trim edges, prefix `ae/`.
fn sanitize_branch_name(name: &str) -> String {
    let stripped = name.strip_prefix("ae-").unwrap_or(name);
    let mut out = String::with_capacity(stripped.len());
    for ch in stripped.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    format!("ae/{}", out.trim_matches('-'))
}

// ---- the enumerable sweep --------------------------------------------------

/// What the enumerable sweep found — a TRI-state, because "no answer" and "not
/// there" are different facts and only one of them may authorise a deletion.
enum Sweep {
    /// The name is LIVE on this enumerable server.
    Found(String),
    /// This socket exists but did not answer, so the target could be hiding
    /// behind it.
    Unsure(String),
    /// Every enumerable socket answered, and none of them has the name.
    Clear,
}

/// Ask every enumerable tmux socket whether `name` is live there.
///
/// Only `--assume-stopped` consults this, and it is the reason that
/// acknowledgement is safe to honour: an assumption that is provably false is
/// refused rather than trusted.
fn sweep(root: &Path, name: &str) -> Sweep {
    let Some(dir) = socket_dir(root) else {
        // Nowhere to enumerate is not proof of absence, but it is also not a
        // sighting; the acknowledgement stands on its own, as it does for a
        // server ae cannot enumerate at all.
        return Sweep::Clear;
    };
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the frozen _end_sweep_servers enumerates the tmux socket directory before an --assume-stopped deletion"
    )]
    let entries = std::fs::read_dir(&dir);
    let Ok(entries) = entries else {
        return Sweep::Clear;
    };
    let mut unsure = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !crate::tmux::is_addressable_socket(&path) {
            continue;
        }
        let label = entry.file_name().to_string_lossy().into_owned();
        let server = ServerId::Selected(Selector::Socket(path));
        match transport::verify_session_absent(&server, name) {
            StopProbe::Present => return Sweep::Found(label),
            StopProbe::Absent => {}
            StopProbe::Unknown => {
                if unsure.is_none() {
                    unsure = Some(label);
                }
            }
        }
    }
    unsure.map_or(Sweep::Clear, Sweep::Unsure)
}

/// `${TMUX_TMPDIR:-/tmp}/tmux-<uid>` — where tmux keeps its default sockets.
///
/// The uid comes from the state root's own owner rather than a syscall:
/// `unsafe_code = "forbid"` closes the libc route, and the directory ae keeps
/// its state in is owned by the user whose tmux sockets are being enumerated.
fn socket_dir(root: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt as _;
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: TMUX_TMPDIR is where tmux itself looks, and the state root's owner is the uid tmux names its socket directory after"
    )]
    let base = std::env::var_os("TMUX_TMPDIR")
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: see above — the uid is read from the state root rather than from libc"
    )]
    let owner = std::fs::metadata(root).ok()?;
    Some(base.join(format!("tmux-{}", owner.uid())))
}

#[cfg(test)]
mod tests {
    use super::{Action, Plan, path_exists, purge_conversation_files, sanitize_branch_name};
    use std::path::{Path, PathBuf};

    /// `--purge-history` deletes the conversations ae can name by EXACT id, and
    /// nothing else. agy can be named exactly — one file per id — so it belongs
    /// in that set; it was silently skipped, which is a retention leak rather
    /// than a conservative default, because the operator asked for a purge.
    #[test]
    fn a_purge_removes_the_agy_conversation_and_its_sidecars_and_no_one_else_s() {
        let scratch = std::env::temp_dir().join(format!("ae-purge-agy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let home = scratch.join("home");
        // `purge_conversation_files` derives the tool homes from the state
        // root's PARENT, which is what makes a fake home possible at all.
        let root = home.join(".ae");
        let store = home.join(".gemini/antigravity-cli/conversations");
        std::fs::create_dir_all(&store).expect("a conversation store");
        std::fs::create_dir_all(&root).expect("a state root");

        let mine = "643393ad-eb92-4b9e-ab7a-0fe7b1221fa1";
        let sibling = "aaaaaaaa-1111-4111-8111-111111111111";
        let write = |path: &Path| std::fs::write(path, "x").expect("a fixture conversation");
        for suffix in ["db", "db-wal", "db-shm"] {
            write(&store.join(format!("{mine}.{suffix}")));
            write(&store.join(format!("{sibling}.{suffix}")));
        }
        // A seat whose id was never captured names nothing, so it must delete
        // nothing — the guard that stops a pending seat purging a stranger.
        let meta = format!(
            "session=x\nschema=2\nseat.main=lead\nagent_bin.main=agy\n\
             harness_session.main={mine}\nseat.worker.1=hand\nagent_bin.worker.1=agy\n\
             harness_session.worker.1=pending\n"
        );
        let mut out = Vec::new();
        let mut err = Vec::new();
        purge_conversation_files(&root, meta.as_bytes(), &mut out, &mut err)
            .expect("the purge writes its report");
        let said = String::from_utf8_lossy(&out).into_owned();

        for suffix in ["db", "db-wal", "db-shm"] {
            let gone = store.join(format!("{mine}.{suffix}"));
            assert!(
                !path_exists(&gone),
                "the seat's own {suffix} must be removed: {said}"
            );
            assert!(
                said.contains(&format!("removed agy conversation: {}", gone.display())),
                "and the removal must be reported: {said}"
            );
            assert!(
                path_exists(&store.join(format!("{sibling}.{suffix}"))),
                "another conversation's {suffix} must be untouched"
            );
        }
        assert!(
            !said.contains("pending"),
            "a seat with no captured id names nothing: {said}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// The paths are computed, never searched, and only existing ones are named.
    #[test]
    fn the_agy_purge_names_only_files_that_are_there() {
        let scratch = std::env::temp_dir().join(format!("ae-purge-agy2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let store = scratch.join(".gemini/antigravity-cli/conversations");
        std::fs::create_dir_all(&store).expect("a conversation store");
        let id = "643393ad-eb92-4b9e-ab7a-0fe7b1221fa1";
        assert_eq!(
            super::agy_conversation_files(&scratch, id),
            Vec::<PathBuf>::new(),
            "an id with nothing on disk names nothing"
        );
        std::fs::write(store.join(format!("{id}.db")), "x").expect("a conversation");
        assert_eq!(
            super::agy_conversation_files(&scratch, id),
            vec![store.join(format!("{id}.db"))],
            "a database with no sidecars is one file, not three"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn the_branch_name_is_the_frozen_sanitisation() {
        assert_eq!(sanitize_branch_name("proj"), "ae/proj");
        assert_eq!(sanitize_branch_name("ae-proj"), "ae/proj");
        assert_eq!(sanitize_branch_name("a/b c"), "ae/a-b-c");
        assert_eq!(sanitize_branch_name("--x--"), "ae/x");
        assert_eq!(
            sanitize_branch_name("keep.dots_and-dashes"),
            "ae/keep.dots_and-dashes"
        );
    }

    #[test]
    fn a_plan_line_states_both_actions() {
        let keep = Plan {
            action: Action::Keep,
            detail: "/a/archive/u".to_owned(),
            purge: false,
        };
        assert_eq!(
            keep.line("s"),
            "  - s: archive -> /a/archive/u/ · conversation files KEPT"
        );
        let purge = Plan {
            action: Action::Purge,
            detail: "/a/archive/u".to_owned(),
            purge: true,
        };
        assert_eq!(
            purge.line("s"),
            "  - s: NO archive, and /a/archive/u/ is DELETED if it exists · conversation files DELETED"
        );
    }
}
