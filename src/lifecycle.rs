//! `_end`, `_stop` and `_compact`: the three destructive lifecycle operations,
//! whole, in the core.
//!
//! Ported from `ae`'s `cmd_end`/`end_session`/`_end_session_locked`/
//! `cleanup_session`/`_end_archive_step`, `cmd_stop`/`_stop_dispatch`/
//! `_stop_one_session`/`_stop_session_locked`, and `cmd_compact`. The core
//! already owned every STEP (`_end-local-teardown`, `_archive-publish`,
//! `_compact-freeze` and the rest); what lived in bash was the ORDER, and the
//! order is the whole contract:
//!
//! 1. the per-session **lifecycle lock** is taken before the target is
//!    classified and held through the last removal, so a start or resume can
//!    never land inside an end;
//! 2. the session is **positively identified on its own recorded server** and
//!    its kill is **verified** — an unverifiable kill returns before anything
//!    is snapshotted, let alone deleted;
//! 3. git has its say (commit, then push) on a tree nothing is writing to any
//!    more;
//! 4. the **archive is published** — mandatory on a keep, after the verified
//!    stop and after git, and BEFORE any live state is removed, so a failed
//!    archive fails the end with the whole session still on disk;
//! 5. only then is the live state removed.
//!
//! # What this module may NOT do
//!
//! Guess a server. Every refusal here is fail-closed: a session with no
//! positive server record is refused rather than swept for, an unreachable
//! server is never read as "stopped", and `--assume-stopped` is the only
//! acknowledgement that crosses that line — per target, never for `all`.

pub(crate) mod compaction;
pub(crate) mod end;

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::inventory::ServerId;
use crate::meta::{self, ServerSelector};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::tmux::StopProbe;
use crate::transport;

/// How long a lifecycle operation waits for the per-session lock — the frozen
/// `flock -w 15`.
const LIFECYCLE_WAIT: Duration = Duration::from_secs(15);

/// `<AE_HOME>/sessions`.
pub(crate) fn sessions_dir(root: &Path) -> PathBuf {
    crate::inventory::Roots::under(root).sessions().to_owned()
}

/// `<AE_HOME>/worktrees`.
pub(crate) fn worktrees_dir(root: &Path) -> PathBuf {
    crate::inventory::Roots::under(root).worktrees().to_owned()
}

/// Take the per-session lifecycle lock — `<sessions>/.lifecycle.<name>.lock`.
///
/// The returned handle IS the lock: dropping it releases, so every caller binds
/// it for exactly the region the frozen `{ … } 9>"$lock"` block covered. The
/// frozen path degraded loudly when `flock` was absent; the core has the
/// capability unconditionally, so there is no degraded mode to announce.
pub(crate) fn lock(root: &Path, name: &str) -> io::Result<fs::File> {
    let sessions = sessions_dir(root);
    fs::create_dir_all(&sessions)?;
    crate::state::acquire(
        &sessions.join(format!(".lifecycle.{name}.lock")),
        LIFECYCLE_WAIT,
    )
}

/// The frozen session-name grammar — `^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$`.
///
/// One definition, quoted verbatim in every refusal that raises it. A name is
/// simultaneously a tmux session, a directory under `<AE_HOME>/sessions`, part
/// of a lock filename and the target of a removal, which is why it is
/// allowlisted rather than merely screened.
pub(crate) fn name_is_valid(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    name.len() <= 128 && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Whether `name` may be used to reach an EXISTING session — the frozen
/// `_session_name_usable`. The grammar, OR a legacy name that is already a real
/// direct child of the sessions root, so a pre-grammar session is never made
/// un-endable. A name carrying a path separator or a `.`/`..` component is
/// refused whatever the directory holds.
pub(crate) fn name_is_usable(root: &Path, name: &str) -> bool {
    if name_is_valid(name) {
        return true;
    }
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return false;
    }
    // The legacy arm. `symlink_metadata` classifies the LINK, so a grammar-
    // invalid symlink pointing outside the root can never qualify.
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the frozen legacy-name arm — an existing direct-child session directory is usable even when its name predates the grammar"
    )]
    let meta = fs::symlink_metadata(sessions_dir(root).join(name));
    meta.is_ok_and(|m| m.is_dir())
}

/// The tmux server a session's own meta records, or the refusal to guess.
pub(crate) fn server_of(bytes: &[u8]) -> ServerSelector {
    meta::Meta::parse(&String::from_utf8_lossy(bytes)).server_selector()
}

/// A meta value as an owned lossy string, empty when the key is absent.
pub(crate) fn meta_value(bytes: &[u8], key: &str) -> String {
    meta::first_value(bytes, key).map_or_else(String::new, |value| {
        String::from_utf8_lossy(value).into_owned()
    })
}

/// The EXACT live session id for `name` on `server`, or `None` when it is not
/// live there — the frozen `_end_live_id`.
///
/// An id, never a name: `kill-session -t proj` PREFIX-MATCHES and kills a live
/// `project` while reporting success. Every kill in this module is addressed by
/// what this returns.
pub(crate) fn live_id(server: &ServerId, name: &str) -> Option<String> {
    transport::observe_session_id(server, name)
}

/// Kill an exactly-identified session on its recorded server and VERIFY it
/// died — the frozen `_lifecycle_kill_verified`, and the ONE answer to "is it
/// gone" that `stop`, `end` and `compact` share so the three cannot drift into
/// disagreeing.
///
/// `verb` is the caller's own word, so the retry line names the command the
/// human actually ran. The kill's own exit status is deliberately ignored: the
/// proof is the verification that follows it.
pub(crate) fn kill_verified(
    server: &ServerId,
    name: &str,
    verb: &str,
    session_id: &str,
    err: &mut impl Write,
) -> io::Result<bool> {
    let _ = transport::kill_session(server, session_id);
    match transport::verify_session_absent(server, name) {
        StopProbe::Absent => Ok(true),
        StopProbe::Present => {
            writeln!(
                err,
                "Error: could not kill session '{name}' — still alive; state preserved. Retry 'ae {verb} {name}'."
            )?;
            Ok(false)
        }
        StopProbe::Unknown => {
            writeln!(
                err,
                "Error: cannot verify '{name}' was killed (its tmux server is unreachable) — state preserved."
            )?;
            Ok(false)
        }
    }
}

/// Every ae session the state root knows about, live or stopped — the frozen
/// `list_ae_sessions` + `iter_stopped_sessions` union that `end all` and
/// `stop all` enumerate, in directory order made deterministic by a sort.
///
/// Read from DURABLE state only. A tmux-only session (a name ae has no record
/// of) is deliberately not a target: `end` would have nothing to archive and
/// `stop` would be killing something it cannot prove is its own.
pub(crate) fn all_sessions(root: &Path) -> Vec<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: `end all` / `stop all` enumerate the sessions root — the frozen list_ae_sessions + iter_stopped_sessions union"
    )]
    let entries = fs::read_dir(sessions_dir(root));
    let Ok(entries) = entries else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            entry
                .file_type()
                .ok()
                .filter(std::fs::FileType::is_dir)
                .map(|_| name)
        })
        .collect();
    names.sort();
    names
}

/// Whether `path` is a directory, following symlinks.
pub(crate) fn dir_exists(path: &Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the lifecycle paths must know whether a recorded work dir, origin or session dir is there before acting on it"
    )]
    let meta = fs::metadata(path);
    meta.is_ok_and(|m| m.is_dir())
}

/// Whether `path` exists at all, following symlinks.
pub(crate) fn path_exists(path: &Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the archive plan asks whether a session's memory files are present before calling a target unarchivable"
    )]
    let meta = fs::metadata(path);
    meta.is_ok()
}

// ---- `_stop` ---------------------------------------------------------------

/// The frozen `ae stop` usage.
const STOP_USAGE: &str = "Usage: _stop <session-name|all> [-y] [--self]";

/// `_stop <name|all> [-y]` — the whole stop operation.
///
/// Stop DESTROYS NOTHING, so it carries none of `end`'s `--assume-stopped`
/// machinery: with no positive ownership record there is nothing to
/// acknowledge, only a guess to refuse.
pub(crate) fn run_stop(
    root: &Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let mut target = String::new();
    let mut yes = false;
    let mut is_self = false;
    let mut supervise = false;
    // `--pane <id>`: the caller's own pane (the shim passes `$TMUX_PANE`, or the
    // operator's explicit `--pane=<id>` from a run-shell child, where the
    // inherited $TMUX_PANE names a FOREIGN pane). The core resolves it to a
    // session itself: a target equal to it is a self-stop, and `all` with the
    // caller inside any target is handed to the detached supervisor whole, so
    // nothing is abandoned when the caller's pane dies mid-fleet.
    let (pane, words) = split_pane_flag(tail);
    for arg in &words {
        match arg.as_str() {
            "-y" | "--yes" => yes = true,
            // The caller asserts it is running INSIDE the target. Identity is
            // the shim's to prove (it passes the pane's own `$AE_SESSION`); this
            // flag is how that proof arrives, and it is why the operation cannot
            // run in this process.
            "--self" => is_self = true,
            // The detached worker `--self` starts. Never human-typed, and never
            // reachable from `--self` itself: this arm runs the locked stop
            // directly, so no re-detach is possible by shape.
            "--supervise" => supervise = true,
            flag if flag.starts_with('-') => {
                writeln!(err, "Error: unknown flag '{flag}'. Use -y/--yes or --self.")?;
                return Ok(EXIT_USAGE);
            }
            name if target.is_empty() => name.clone_into(&mut target),
            extra => {
                writeln!(
                    err,
                    "Error: unexpected extra argument '{extra}' — _stop takes one session name (or 'all')."
                )?;
                return Ok(EXIT_USAGE);
            }
        }
    }
    let caller_session = if pane.is_empty() {
        None
    } else {
        crate::transport::observe_pane_owner(&ServerId::Ambient, &pane).map(|owner| owner.session)
    };
    if target.is_empty() && is_self {
        let Some(own) = self_target(caller_session.as_deref(), err)? else {
            return Ok(EXIT_FAILED);
        };
        target = own;
    }
    if target.is_empty() {
        writeln!(err, "{STOP_USAGE}")?;
        return Ok(EXIT_USAGE);
    }
    // `--self` asserts "the session I am in", so it is a claim about ONE
    // session. With `all` it is a claim about everyone else, which is how
    // `ae stop all -y --self` classified every candidate as self, kept the last
    // and left the rest live with rc 0.
    if is_self && target == "all" {
        writeln!(
            err,
            "Error: --self names the session you are in — it cannot be combined with 'all'."
        )?;
        return Ok(EXIT_USAGE);
    }
    if supervise {
        return run_supervisor(root, &target, out, err);
    }
    if let Some(own) = &caller_session {
        if target == "all" && all_sessions(root).contains(own) {
            return fleet_supervised(root, own, yes, out, err);
        }
        if target == *own {
            is_self = true;
        }
    }
    if is_self {
        return self_supervised(root, &target, yes, out, err);
    }
    if target == "all" {
        let names = all_sessions(root);
        if names.is_empty() {
            writeln!(out, "No running ae sessions.")?;
            return Ok(0);
        }
        // THE FLEET FORM CONFIRMS FROM EVERY CALLER. Singular stop destroys
        // nothing and needs no prompt; `stop all` takes down every session a
        // typo away, so without -y it asks, and with no terminal it refuses —
        // the bash contract from the first day (glue cut 2 finding).
        if !yes && !confirm_fleet_stop(names.len(), out, err)? {
            return Ok(EXIT_FAILED);
        }
        let mut failures = 0_u32;
        for name in names {
            // A stopped session in the roster is not a failure of `stop all`:
            // the fleet form's job is that nothing is left running, and one
            // already down satisfies it. Only a session that could not be
            // stopped counts.
            match stop_recorded(root, &name, out, err)? {
                StopOutcome::Stopped | StopOutcome::AlreadyStopped => {}
                StopOutcome::Failed => failures += 1,
            }
        }
        if failures > 0 {
            writeln!(
                err,
                "{failures} session(s) failed to stop. See errors above."
            )?;
            return Ok(EXIT_FAILED);
        }
        return Ok(0);
    }
    if !name_is_usable(root, &target) {
        writeln!(err, "ae: '{target}' is not a usable session name.")?;
        return Ok(EXIT_FAILED);
    }
    match stop_recorded(root, &target, out, err)? {
        StopOutcome::Stopped => Ok(0),
        StopOutcome::AlreadyStopped | StopOutcome::Failed => Ok(EXIT_FAILED),
    }
}

/// `stop all` without `-y`: ask on a terminal, refuse without one. `true`
/// means go ahead.
fn confirm_fleet_stop(
    count: usize,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<bool> {
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        writeln!(
            err,
            "Error: 'stop all' stops every running ae session ({count}), and there is no terminal to confirm on."
        )?;
        writeln!(err, "  Re-run with -y: ae stop all -y")?;
        writeln!(err, "  Nothing was stopped.")?;
        return Ok(false);
    }
    write!(out, "Stop all {count} running ae session(s)? [y/N] ")?;
    out.flush()?;
    let mut reply = String::new();
    let answered = std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut reply)
        .is_ok_and(|read| read > 0);
    let reply = reply.trim_start();
    if !answered || !(reply.starts_with('y') || reply.starts_with('Y')) {
        writeln!(out, "Nothing was stopped.")?;
        return Ok(false);
    }
    Ok(true)
}

/// One target's stop, RECORDED in that target's own events log whatever the
/// caller's streams were: the request before, the outcome after. The log is
/// the only witness that survives the caller — an agent that stops a session
/// from a pane about to close, a script whose stdout nobody reads — so every
/// stop path goes through here, in-process or supervised. The diagnostics are
/// captured so the failure reason can travel into the record, then replayed
/// to the caller unchanged.
fn stop_recorded(
    root: &Path,
    name: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<StopOutcome> {
    let dir = sessions_dir(root).join(name);
    emit_stop_event(&dir, name, STOP_REQUEST_ACTION, "stop requested");
    let mut captured_out = Vec::new();
    let mut captured_err = Vec::new();
    let outcome = stop_one(root, name, &mut captured_out, &mut captured_err)?;
    let summary = match outcome {
        StopOutcome::Stopped => "stopped: verified gone on its recorded server".to_owned(),
        StopOutcome::AlreadyStopped => "already stopped".to_owned(),
        StopOutcome::Failed => format!("FAILED: {}", String::from_utf8_lossy(&captured_err).trim()),
    };
    emit_stop_event(&dir, name, STOP_RESULT_ACTION, &summary);
    out.write_all(&captured_out)?;
    err.write_all(&captured_err)?;
    Ok(outcome)
}

/// What one target's stop did — kept distinct so the fleet form can treat an
/// already-stopped session as satisfied while the singular form still reports
/// it as the frozen `Session 'x' is not running.` failure.
enum StopOutcome {
    Stopped,
    AlreadyStopped,
    Failed,
}

// ---- `--self`: the stop that cannot run in the process asking for it -------

/// The event a self-stop records before it hands over, and the one the
/// supervisor records when it is done. Both go in the TARGET's own log, because
/// after a self-stop the pane that asked is gone and this file is the only
/// account a human has.
const STOP_REQUEST_ACTION: &str = "stop-request";
const STOP_RESULT_ACTION: &str = "stop-result";

/// A `nohup` argv minted ONLY by [`supervisor_argv`]. Its inner vector is
/// private, so no other module can fabricate a command line and hand it to
/// [`crate::transport::run_detached`] — the door runs a `DetachedArgv`, and only
/// this module can construct one. Same seal, and the same reasoning, as
/// [`crate::git::GitArgv`].
pub(crate) struct DetachedArgv(Vec<String>);

impl DetachedArgv {
    /// The argv for the transport door to spawn. Reading is harmless;
    /// construction is what is sealed.
    pub(crate) fn as_args(&self) -> &[String] {
        &self.0
    }
}

/// `nohup <this binary> _stop --supervise <name>` — the ONE shape this module
/// can mint, with the session name as its own argv element (no shell, so
/// nothing to inject) and nothing else settable by a caller.
///
/// `None` when this process cannot name its own executable: without that the
/// supervisor could only be started through a `PATH` lookup, which may resolve
/// to a different `ae` than the one holding the contract.
fn supervisor_argv(name: &str) -> Option<DetachedArgv> {
    let own = std::env::current_exe().ok()?;
    Some(DetachedArgv(vec![
        own.to_string_lossy().into_owned(),
        crate::cli::STOP.to_owned(),
        "--supervise".to_owned(),
        name.to_owned(),
    ]))
}

/// `nohup env AE_NO_AUTOSTART=1 <glue> <session>` — the launch's orchestrator
/// companion (the frozen ae:8055), and the second shape this module can mint.
///
/// `env` carries the recursion guard the core cannot set for itself: the child
/// is the GLUE, which reads `AE_NO_AUTOSTART` to decide whether to start
/// companions of its own. Every word is fixed but the glue's path and the
/// scaffold's session name, and there is no shell, so a path with a space stays
/// one argument.
pub(crate) fn orchestrator_argv(glue: &Path, session: &str) -> DetachedArgv {
    DetachedArgv(vec![
        "env".to_owned(),
        "AE_NO_AUTOSTART=1".to_owned(),
        glue.display().to_string(),
        session.to_owned(),
    ])
}

/// Append one line to the target's event log, best-effort.
///
/// Best-effort deliberately: a self-stop whose audit line could not be written
/// must still stop the session. The line is the account, not the operation.
fn emit_stop_event(dir: &Path, name: &str, action: &str, summary: &str) {
    let _ = crate::state::emit(
        dir,
        &crate::tracked::event_line(&crate::tracked::EventFields {
            ts: crate::time::Timestamp::now(),
            actor: "human",
            action,
            target: name,
            reference: "",
            actor_slot: "",
            actor_session: "",
            target_slot: "",
            target_session: "",
            summary,
            body_file: "",
        }),
    );
}

/// `_stop --self <name>`: hand the whole stop to a detached supervisor and
/// return.
///
/// THE CALLER IS INSIDE THE TARGET. Nothing running in that pane can own this
/// operation, because the kill destroys the pane mid-loop — the lock would be
/// released by a dead process, the verification would never run, and the result
/// would never be recorded. Earlier attempts to identify the caller among the
/// targets and stop it last cannot work in general, so nothing here iterates:
/// a detached child owns the lock, the identity lookup, the kill and the
/// verification, and no session it kills is running it.
///
/// This function takes NO lock. It must not: the child needs the same
/// per-session lock, and a lock held by a process about to be killed is the
/// exact hazard being avoided.
fn self_supervised(
    root: &Path,
    name: &str,
    yes: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    if !name_is_usable(root, name) {
        writeln!(err, "ae: '{name}' is not a usable session name.")?;
        return Ok(EXIT_FAILED);
    }
    let dir = sessions_dir(root).join(name);
    if !dir_exists(&dir) {
        writeln!(
            err,
            "Error: no session state for '{name}' — refusing to self-stop something ae does not own."
        )?;
        return Ok(EXIT_FAILED);
    }
    if !yes {
        // ASK WHETHER WE CAN ASK, BEFORE ASKING. A caller with no terminal gets
        // one clean error naming the flag, not a prompt it cannot answer.
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            writeln!(
                err,
                "Error: '{name}' is the session you are in, and there is no terminal to confirm on."
            )?;
            writeln!(
                err,
                "  Re-run with -y to stop it non-interactively: ae stop {name} -y"
            )?;
            return Ok(EXIT_FAILED);
        }
        writeln!(
            out,
            "Stop '{name}'? This kills the session you are working in."
        )?;
        writeln!(
            out,
            "  Agents may be mid-turn: active writes and partial turns can be interrupted."
        )?;
        writeln!(
            out,
            "  Your ae state, working tree and provider conversation files are PRESERVED —"
        )?;
        writeln!(
            out,
            "  the guarantee is recoverability (resume from the provider's own checkpoint),"
        )?;
        writeln!(out, "  not mid-write atomicity.")?;
        write!(out, "Continue? [y/N] ")?;
        out.flush()?;
        let mut reply = String::new();
        let answered = std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut reply)
            .is_ok_and(|read| read > 0);
        let reply = reply.trim_start();
        if !answered || !(reply.starts_with('y') || reply.starts_with('Y')) {
            writeln!(out, "Not stopped.")?;
            return Ok(EXIT_FAILED);
        }
    }
    let Some(argv) = supervisor_argv(name) else {
        writeln!(
            err,
            "Error: ae cannot name its own executable, so it cannot hand '{name}' to a supervisor — nothing was stopped."
        )?;
        return Ok(EXIT_FAILED);
    };
    // The INTENT is recorded before the handover, so a human whose pane vanished
    // can tell "ae was asked and something went wrong" from "ae was never asked".
    emit_stop_event(
        &dir,
        name,
        STOP_REQUEST_ACTION,
        "self-stop requested from inside the session",
    );
    if !crate::transport::run_detached(&argv) {
        writeln!(
            err,
            "Error: could not start the supervisor for '{name}' — nothing was stopped."
        )?;
        return Ok(EXIT_FAILED);
    }
    writeln!(out, "Stopping '{name}' out of pane; this pane will close.")?;
    writeln!(
        out,
        "  The outcome is recorded durably in {}/events.jsonl (action: {STOP_RESULT_ACTION}).",
        dir.display()
    )?;
    Ok(0)
}

/// `_stop --supervise <name>`: the detached worker.
///
/// It — not the dying caller — owns the lock, the identity lookup, the kill and
/// the verification, and it records the OUTCOME so a human whose pane vanished
/// can still find out what happened. Its streams are `/dev/null`, so the event
/// log is the only report it can make; that is why the failure arm records the
/// diagnostic rather than merely returning a code.
///
/// Calls [`stop_one`] directly, never `run_stop`: this process inherits `TMUX`
/// from the pane that spawned it, so routing through the self path again would
/// recurse. Calling the locked path directly makes that impossible by shape.
/// `--self` with no name IS a name: the session the caller's pane resolves to.
fn self_target(caller: Option<&str>, err: &mut impl Write) -> io::Result<Option<String>> {
    if let Some(own) = caller {
        return Ok(Some(own.to_owned()));
    }
    writeln!(
        err,
        "Error: --self with no session name needs a pane ae can resolve (--pane <id>); this one is not an ae agent pane."
    )?;
    Ok(None)
}

/// Lift `--pane <id>` / `--pane=<id>` out of a stop tail; the rest stays in
/// order. An empty value is no pane.
fn split_pane_flag(tail: &[String]) -> (String, Vec<String>) {
    let mut pane: Option<String> = None;
    let mut words: Vec<String> = Vec::with_capacity(tail.len());
    let mut it = tail.iter();
    while let Some(arg) = it.next() {
        if arg == "--pane" {
            pane = it.next().cloned();
        } else if let Some(value) = arg.strip_prefix("--pane=") {
            pane = Some(value.to_owned());
        } else {
            words.push(arg.clone());
        }
    }
    (pane.unwrap_or_default(), words)
}

/// `_stop --supervise all`: the whole fleet, one session at a time, from a
/// process no target pane owns. Used when the caller sits inside a target.
fn run_supervisor(
    root: &Path,
    name: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    if name != "all" {
        return supervise_one(root, name, out, err);
    }
    let mut failures = 0_u32;
    for session in all_sessions(root) {
        if supervise_one(root, &session, out, err)? != 0 {
            failures += 1;
        }
    }
    Ok(u8::from(failures != 0))
}

/// Hand `stop all` to the detached supervisor because the caller is inside
/// `own`, one of the targets; print the same two lines the single self-stop
/// prints and return.
fn fleet_supervised(
    root: &Path,
    own: &str,
    yes: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    if !yes {
        writeln!(
            err,
            "Error: 'stop all' from inside session '{own}' needs -y: the stop is handed to a detached supervisor and cannot prompt."
        )?;
        return Ok(EXIT_FAILED);
    }
    let Some(argv) = supervisor_argv("all") else {
        writeln!(
            err,
            "Error: could not locate this binary to detach the fleet stop."
        )?;
        return Ok(EXIT_FAILED);
    };
    let dir = sessions_dir(root).join(own);
    emit_stop_event(
        &dir,
        own,
        STOP_REQUEST_ACTION,
        "stop all requested from inside",
    );
    if !crate::transport::run_detached(&argv) {
        writeln!(
            err,
            "Error: could not start the detached supervisor for 'stop all'."
        )?;
        return Ok(EXIT_FAILED);
    }
    writeln!(
        out,
        "Stopping all ae sessions out of pane (this one included)."
    )?;
    writeln!(out, "  outcome: {}/events.jsonl", dir.display())?;
    Ok(0)
}

fn supervise_one(
    root: &Path,
    name: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    if !name_is_usable(root, name) {
        return Ok(EXIT_FAILED);
    }
    if !dir_exists(&sessions_dir(root).join(name)) {
        return Ok(EXIT_FAILED);
    }
    // This process has no streams a human can read; the record written by
    // `stop_recorded` is the only place the outcome survives.
    match stop_recorded(root, name, out, err)? {
        StopOutcome::Stopped => Ok(0),
        StopOutcome::AlreadyStopped | StopOutcome::Failed => Ok(EXIT_FAILED),
    }
}

/// One session's stop, under its own lifecycle lock.
fn stop_one(
    root: &Path,
    name: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<StopOutcome> {
    let Ok(_guard) = lock(root, name) else {
        writeln!(
            err,
            "Error: another lifecycle operation (start/resume/end) is in progress for '{name}' — retry shortly. Nothing was stopped."
        )?;
        return Ok(StopOutcome::Failed);
    };
    let dir = sessions_dir(root).join(name);
    let Ok(bytes) = meta::read_bytes(&dir) else {
        writeln!(err, "Session '{name}' not found.")?;
        return Ok(StopOutcome::Failed);
    };
    let ServerSelector::Positive(selector) = server_of(&bytes) else {
        writeln!(
            err,
            "Error: session '{name}' has no positive server record — ae cannot tell which tmux server owns it, and will not guess."
        )?;
        writeln!(err, "  Resolve: 'ae doctor --refresh {name}'.")?;
        writeln!(err, "  Nothing was stopped; state preserved.")?;
        return Ok(StopOutcome::Failed);
    };
    let server = ServerId::Selected(selector);
    let Some(session_id) = live_id(&server, name) else {
        // "Empty answer" and "server unreachable" look identical from here, and
        // only one of them means stopped.
        if transport::verify_session_absent(&server, name) == StopProbe::Unknown {
            writeln!(
                err,
                "Error: cannot verify session '{name}' (its recorded tmux server is unreachable) — nothing was stopped, state preserved."
            )?;
            return Ok(StopOutcome::Failed);
        }
        writeln!(err, "Session '{name}' is not running.")?;
        return Ok(StopOutcome::AlreadyStopped);
    };
    if !kill_verified(&server, name, "stop", &session_id, err)? {
        return Ok(StopOutcome::Failed);
    }
    writeln!(out, "Stopped {name}")?;
    Ok(StopOutcome::Stopped)
}

#[cfg(test)]
mod tests {
    use super::{name_is_valid, sessions_dir, worktrees_dir};
    use std::path::Path;

    #[test]
    fn the_session_name_grammar_is_the_frozen_one() {
        assert!(name_is_valid("proj"));
        assert!(name_is_valid("a"));
        assert!(name_is_valid("A1_b-c"));
        assert!(!name_is_valid(""));
        assert!(!name_is_valid("_leading"));
        assert!(!name_is_valid("-leading"));
        assert!(!name_is_valid("has/slash"));
        assert!(!name_is_valid(".."));
        assert!(!name_is_valid("has space"));
        assert!(name_is_valid(&format!("a{}", "b".repeat(127))));
        assert!(!name_is_valid(&format!("a{}", "b".repeat(128))));
    }

    #[test]
    fn the_roots_are_the_inventory_roots() {
        assert_eq!(sessions_dir(Path::new("/x")), Path::new("/x/sessions"));
        assert_eq!(worktrees_dir(Path::new("/x")), Path::new("/x/worktrees"));
    }
}
