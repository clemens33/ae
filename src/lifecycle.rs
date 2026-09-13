//! `_end`, `_stop` and `_compact`: the three destructive lifecycle operations,
//! whole, in the core.
//!
//! Each STEP is its own core entry (`_end-local-teardown`, `_archive-publish`,
//! `_compact-freeze` and the rest); this module owns the ORDER, and the order
//! is the whole contract:
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

pub(crate) mod compaction;
pub(crate) mod end;

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::inventory::ServerId;
use crate::meta::{self, Selector, ServerSelector};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::tmux::StopProbe;
use crate::transport;

/// How long a lifecycle operation waits for the per-session lock.
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
pub(crate) fn lock(root: &Path, name: &str) -> io::Result<fs::File> {
    let sessions = sessions_dir(root);
    fs::create_dir_all(&sessions)?;
    crate::store::lock(
        &sessions.join(format!(".lifecycle.{name}.lock")),
        LIFECYCLE_WAIT,
    )
}

/// The session-name grammar — `^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$`.
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

/// Whether `name` may be used to reach an EXISTING session.
pub(crate) fn name_is_usable(root: &Path, name: &str) -> bool {
    if name_is_valid(name) {
        return true;
    }
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return false;
    }
    // The pre-grammar arm.
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
/// live there.
pub(crate) fn live_id(server: &ServerId, name: &str) -> Option<String> {
    transport::observe_session_id(server, name)
}

/// Kill an exactly-identified session on its recorded server and VERIFY it
/// died — the ONE answer to "is it gone" that `stop`, `end` and `compact`
/// share so the three cannot drift into
/// disagreeing.
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

/// Every ae session the state root knows about, live or stopped — the union
/// `end all` and `stop all` enumerate, in directory order made deterministic by
/// a sort.
pub(crate) fn all_sessions(root: &Path) -> Vec<String> {
    census(root).ok().flatten().unwrap_or_default()
}

/// [`all_sessions`], but a census that could not be TAKEN says so.
///
/// The difference is not cosmetic, and it is why this exists. `end all` and
/// `stop all` may treat an unreadable sessions root as nothing to do: they act
/// on what they found, and finding nothing means doing nothing. The upgrade
/// sweep may NOT. It decides which version directories no session records any
/// more, and an empty answer there authorises DELETING the core a session is
/// running on — measured: with the sessions root at mode 000 the publish
/// returned success, removed the old version, and left every meta naming a file
/// that was gone. A census that failed and a census that found nothing must
/// therefore be different values.
///
/// An entry whose `file_type` cannot be read is a failure too, for the same
/// reason: skipping it silently drops a session from the keep-set. That is why
/// a missing sessions ROOT is `Ok(None)` rather than an empty list: it is the
/// one reading where "found nothing" and "could not look" genuinely coincide,
/// and it must not be spelled the same way as a `NotFound` raised by an entry
/// that vanished mid-walk — that one is a partial reading like any other.
///
/// # Errors
///
/// The underlying [`io::Error`] — the root could not be enumerated for any
/// reason but its absence, or one of its entries could not be classified.
pub(crate) fn census(root: &Path) -> io::Result<Option<Vec<String>>> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: `end all` / `stop all` enumerate the sessions root — the frozen list_ae_sessions + iter_stopped_sessions union"
    )]
    let entries = match fs::read_dir(sessions_dir(root)) {
        Ok(entries) => entries,
        // A state root that has never had a session. Not a failure to look.
        Err(why) if why.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(why) => return Err(why),
    };
    let mut names: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if entry.file_type()?.is_dir() {
            names.push(name);
        }
    }
    names.sort();
    Ok(Some(names))
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

/// The `ae stop` usage.
const STOP_USAGE: &str = "Usage: _stop <session-name|all> [-y] [--self]";

/// `_stop <name|all> [-y] [--self|--handoff|--supervise]` — the whole stop
/// operation.
#[allow(
    clippy::too_many_lines,
    reason = "the public, handoff and supervisor modes share one ordered parse and target resolution"
)]
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
    let mut handoff = false;
    // `--pane <id>`: the caller's own pane (the shim passes `$TMUX_PANE`, or
    // the operator's explicit `--pane=<id>` from a run-shell child, where the
    // inherited $TMUX_PANE names a FOREIGN pane).
    let (pane, words) = split_pane_flag(tail);
    // `--expect-*`: the identity a menu confirmation proved, which only the
    // detached supervisor re-proves under the target's lifecycle lock.
    let (expect, words) = match split_expectation(&words) {
        Ok(split) => split,
        Err(reason) => {
            writeln!(err, "Error: {reason}")?;
            return Ok(EXIT_USAGE);
        }
    };
    for arg in &words {
        match arg.as_str() {
            "-y" | "--yes" => yes = true,
            // The caller asserts it is running INSIDE the target.
            "--self" => is_self = true,
            // The detached worker `--self` starts.
            "--supervise" => supervise = true,
            // A tmux `run-shell` job starts this, then exits before the
            // detached supervisor kills the job's target session.
            "--handoff" => handoff = true,
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
    let caller_server = crate::doors::caller_server();
    let caller_session = recorded_caller_session(root, caller_server.as_ref(), &pane);
    if target.is_empty() {
        if let Some(own) = &caller_session {
            target.clone_from(own);
            is_self = true;
        } else if is_self {
            let Some(own) = self_target(caller_session.as_deref(), err)? else {
                return Ok(EXIT_FAILED);
            };
            target = own;
        }
    }
    if target.is_empty() {
        writeln!(err, "{STOP_USAGE}")?;
        return Ok(EXIT_USAGE);
    }
    // `--self` asserts "the session I am in", so it is a claim about ONE
    // session.
    if is_self && target == "all" {
        writeln!(
            err,
            "Error: --self names the session you are in — it cannot be combined with 'all'."
        )?;
        return Ok(EXIT_USAGE);
    }
    if supervise {
        return run_supervisor(root, &target, expect.as_ref(), out, err);
    }
    if expect.is_some() {
        writeln!(
            err,
            "Error: --expect-* describes one supervised stop and belongs with --supervise."
        )?;
        return Ok(EXIT_USAGE);
    }
    if handoff {
        return start_stop_supervisor(root, &target, out, err);
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
        // THE FLEET FORM CONFIRMS FROM EVERY CALLER.
        if !yes && !confirm_fleet_stop(names.len(), out, err)? {
            return Ok(EXIT_FAILED);
        }
        let mut failures = 0_u32;
        for name in names {
            // A stopped session in the roster is not a failure of `stop all`:
            // the fleet form's job is that nothing is left running, and one
            // already down satisfies it.
            match stop_recorded(root, &name, None, out, err)? {
                StopOutcome::Stopped | StopOutcome::AlreadyStopped => {}
                // `Refused` belongs to a confirmed stop, which the fleet form
                // never is; it is counted as a failure rather than ignored.
                StopOutcome::Failed | StopOutcome::Refused(_) => failures += 1,
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
    match stop_recorded(root, &target, None, out, err)? {
        StopOutcome::Stopped => Ok(0),
        StopOutcome::AlreadyStopped | StopOutcome::Failed | StopOutcome::Refused(_) => {
            Ok(EXIT_FAILED)
        }
    }
}

/// What a CONFIRMED stop expects to find when it finally runs.
///
/// The session menu proves these when it builds the confirmation and again
/// when the human answers, but neither moment holds the target's lifecycle
/// lock. They travel with the operation so the one place that does hold that
/// lock can prove them a last time, immediately before the kill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StopExpectation {
    /// The tmux session id the human was shown.
    pub(crate) session_id: String,
    /// ae's own identity for that session's state directory.
    pub(crate) uuid: String,
    /// The tmux server process the click happened on.
    pub(crate) server_pid: String,
    /// The epoch second that server started.
    pub(crate) server_start: String,
    /// The epoch second after which the confirmation is stale. Carried, never
    /// renewed: a supervisor that starts late is as stale as its answer.
    pub(crate) deadline: i64,
    /// The client that asked, for the one-line outcome.
    pub(crate) client: String,
    /// That client's process. A tty path is reused, so the name alone cannot
    /// tell the attachment that answered from the next one to take its place.
    pub(crate) client_pid: String,
    /// The server the click happened on, carried explicitly.
    ///
    /// NOT read back from the target's metadata: that file is mutable, can
    /// lose its selector and can disappear, and the human waiting for an
    /// answer is on this server whatever the target's directory says.
    pub(crate) server: ServerId,
    /// Settings Pause requires the raw target role to remain exact under lock.
    pub(crate) require_meta_agent: bool,
}

impl StopExpectation {
    /// The `--expect-*` words a supervisor argv carries.
    fn argv_words(&self) -> Vec<String> {
        vec![
            "--expect-session-id".to_owned(),
            self.session_id.clone(),
            "--expect-uuid".to_owned(),
            self.uuid.clone(),
            "--expect-server-pid".to_owned(),
            self.server_pid.clone(),
            "--expect-server-start".to_owned(),
            self.server_start.clone(),
            "--expect-deadline".to_owned(),
            self.deadline.to_string(),
            "--expect-client".to_owned(),
            self.client.clone(),
            "--expect-client-pid".to_owned(),
            self.client_pid.clone(),
            "--expect-server-kind".to_owned(),
            server_kind_word(&self.server).to_owned(),
            "--expect-server".to_owned(),
            server_value_word(&self.server),
            "--expect-meta-agent".to_owned(),
            self.require_meta_agent.to_string(),
        ]
    }

    /// Whether the captured server is still the one that was clicked on.
    fn server_survives(&self) -> bool {
        transport::observe_server_identity(&self.server)
            .is_some_and(|found| found.pid == self.server_pid && found.start == self.server_start)
    }

    /// Whether the attachment that answered is still the one attached.
    fn clicker_survives(&self) -> bool {
        transport::observe_menu_client(&self.server, &self.client)
            .is_some_and(|client| client.pid == self.client_pid)
    }

    /// Say `text` to the clicker, and only to it.
    ///
    /// The route is the CAPTURED server and the CAPTURED attachment, both
    /// re-proven here. Nothing falls back to another client, and nothing is
    /// said at all when the human who asked is no longer there to hear it.
    fn tell_clicker(&self, text: &str) {
        if self.server_survives() && self.clicker_survives() {
            let _ = transport::display_client_message(&self.server, &self.client, text);
        }
    }

    /// Prove the expectation against the world, under the caller's lock.
    ///
    /// Nothing here writes, reads a migrated meta or trusts a second lookup:
    /// a target this cannot prove must come out of the operation byte for byte
    /// as it went in, and the id it returns is the ONLY one authorised to die.
    fn check(&self, dir: &Path, name: &str, now: i64) -> ExpectCheck {
        if now > self.deadline {
            return ExpectCheck::Refused(format!(
                "the confirmation for '{name}' expired before it could be applied"
            ));
        }
        if now < self.deadline - crate::session_menu::CONFIRM_WINDOW_SECS {
            return ExpectCheck::Refused(format!(
                "the confirmation for '{name}' is stamped in the future"
            ));
        }
        // THE RAW META, never a migrated one: proving the identity is what
        // earns the right to rewrite this directory, so it cannot come after.
        let Ok(bytes) = meta::read_bytes(dir) else {
            return ExpectCheck::Refused(format!("ae cannot read the metadata of '{name}'"));
        };
        if self.require_meta_agent && meta::meta_agent_role(&bytes) != meta::MetaAgentRole::Role {
            return ExpectCheck::Refused(format!(
                "'{name}' no longer proves the orchestrator role"
            ));
        }
        let uuid = crate::archive::canonical_uuid(&meta_value(&bytes, "session_id"));
        if uuid != self.uuid {
            return ExpectCheck::Refused(format!(
                "the state directory of '{name}' was replaced since it was confirmed"
            ));
        }
        // ae's OWN record must still name the server the click happened on.
        // The captured one is the authority; this only refuses a target that
        // has since been re-recorded somewhere else.
        let ServerSelector::Positive(selector) = server_of(&bytes) else {
            return ExpectCheck::Refused(format!(
                "'{name}' has no positive server record, so ae cannot prove what was confirmed"
            ));
        };
        let recorded = ServerId::Selected(selector);
        let mut sockets = crate::SocketPaths::asking(transport::observe_socket_path);
        if !sockets.proven_same(&self.server, &recorded) {
            return ExpectCheck::Refused(format!(
                "'{name}' is recorded on a different tmux server than the one it was confirmed on"
            ));
        }
        if !self.server_survives() {
            return ExpectCheck::Refused(format!(
                "the tmux server was replaced since '{name}' was confirmed"
            ));
        }
        // THE HUMAN WHO ANSWERED must still be there to be told what happened,
        // and must be the same attachment: a tty path outlives its client.
        if !self.clicker_survives() {
            return ExpectCheck::Refused(format!(
                "the client that confirmed '{name}' is gone, so ae will not apply its answer"
            ));
        }
        match live_id(&self.server, name) {
            // The ONE id this operation may kill. Nothing downstream looks the
            // name up again: tmux is not under ae's lifecycle lock, so a second
            // answer could name a session nobody confirmed.
            Some(live) if live == self.session_id => ExpectCheck::Proven {
                server: self.server.clone(),
                id: self.session_id.clone(),
            },
            Some(_) => ExpectCheck::Refused(format!(
                "'{name}' is a different tmux session than the one confirmed"
            )),
            // An absent session is not an unmet expectation: it is what a
            // duplicate application of one confirmation must be.
            None => ExpectCheck::Absent,
        }
    }
}

/// The `--expect-server-kind` word for one server.
fn server_kind_word(server: &ServerId) -> &'static str {
    match server {
        ServerId::Selected(Selector::Name(_)) => "name",
        ServerId::Selected(Selector::Socket(_)) => "socket",
        ServerId::Ambient => "ambient",
    }
}

/// The `--expect-server` word for one server.
fn server_value_word(server: &ServerId) -> String {
    match server {
        ServerId::Selected(Selector::Name(name)) => name.clone(),
        ServerId::Selected(Selector::Socket(path)) => path.display().to_string(),
        ServerId::Ambient => String::new(),
    }
}

/// What proving a [`StopExpectation`] under the lifecycle lock found.
enum ExpectCheck {
    /// The target is the confirmed one, and this is the id it answers to.
    Proven { server: ServerId, id: String },
    /// The target is provably ae's and provably already gone.
    Absent,
    /// Something did not match. NOTHING may be written to the target.
    Refused(String),
}

/// `_session-menu apply` — the human confirmed; detach the ONE existing stop
/// supervisor, carrying the identity it must prove under the lock.
///
/// This writes NOTHING to the target. It has not held the target's lifecycle
/// lock, so it cannot yet know the directory is the one that was confirmed,
/// and an audit line in a stranger's directory is exactly the unauthorised
/// write the identity proof exists to prevent. The supervisor records the
/// human's answer once that proof has passed.
pub(crate) fn stop_confirmed(
    root: &Path,
    expect: &StopExpectation,
    name: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    if !name_is_usable(root, name) {
        writeln!(err, "ae: '{name}' is not a usable session name.")?;
        return Ok(EXIT_FAILED);
    }
    let Some(argv) = supervisor_argv_expecting(name, Some(expect)) else {
        writeln!(
            err,
            "Error: ae cannot name its own executable, so it cannot hand '{name}' to a supervisor — nothing was stopped."
        )?;
        return Ok(EXIT_FAILED);
    };
    if !crate::transport::run_detached(&argv) {
        writeln!(
            err,
            "Error: could not start the supervisor for '{name}' — nothing was stopped."
        )?;
        return Ok(EXIT_FAILED);
    }
    writeln!(out, "Stopping '{name}'.")?;
    Ok(0)
}

/// `stop all` without `-y`: ask on a terminal, refuse without one.
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
/// caller's streams were: the request before, the outcome after.
fn stop_recorded(
    root: &Path,
    name: &str,
    expect: Option<&StopExpectation>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<StopOutcome> {
    let dir = sessions_dir(root).join(name);
    // An ORDINARY stop owns its target by construction, so it brackets the
    // operation from out here. A CONFIRMED one does not own anything yet:
    // BOTH of its lines are written by `stop_one`, inside the lock it proved
    // the identity under, so no window exists in which this process has
    // written to a directory it never proved was the confirmed one.
    if expect.is_some() {
        return stop_one(root, name, expect, out, err);
    }
    emit_stop_event(&dir, name, STOP_REQUEST_ACTION, "stop requested");
    let mut captured_out = Vec::new();
    let mut captured_err = Vec::new();
    let outcome = stop_one(root, name, expect, &mut captured_out, &mut captured_err)?;
    emit_stop_event(
        &dir,
        name,
        STOP_RESULT_ACTION,
        &stop_summary(&outcome, &captured_err),
    );
    out.write_all(&captured_out)?;
    err.write_all(&captured_err)?;
    Ok(outcome)
}

/// The durable one-line result of one stop.
fn stop_summary(outcome: &StopOutcome, captured_err: &[u8]) -> String {
    match outcome {
        StopOutcome::Stopped => {
            // A successful stop may still carry migration or client-handoff
            // warnings; retain every captured warning in the durable result.
            let warning = String::from_utf8_lossy(captured_err).trim().to_owned();
            if warning.is_empty() {
                "stopped: verified gone on its recorded server".to_owned()
            } else {
                format!("stopped: verified gone on its recorded server; {warning}")
            }
        }
        StopOutcome::AlreadyStopped => "already stopped".to_owned(),
        StopOutcome::Failed => format!("FAILED: {}", String::from_utf8_lossy(captured_err).trim()),
        // NOT OURS TO WRITE TO. A refused identity leaves the directory alone,
        // and this summary is never emitted for it.
        StopOutcome::Refused(_) => String::new(),
    }
}

/// What one target's stop did — kept distinct so the fleet form can treat an
/// already-stopped session as satisfied while the singular form still reports
/// it as the `Session 'x' is not running.` failure.
enum StopOutcome {
    Stopped,
    AlreadyStopped,
    Failed,
    /// A CONFIRMED stop whose identity did not hold under the lock. Distinct
    /// from `Failed` because the target is not provably ae's to write to: a
    /// refusal leaves the directory byte for byte as it found it.
    Refused(String),
}

// ---- `--self`: the stop that cannot run in the process asking for it -------

/// The event a self-stop records before it hands over, and the one the
/// supervisor records when it is done.
const STOP_REQUEST_ACTION: &str = "stop-request";
const STOP_RESULT_ACTION: &str = "stop-result";

/// A `nohup` argv minted ONLY by [`supervisor_argv`].
pub(crate) struct DetachedArgv(Vec<String>);

impl DetachedArgv {
    /// The argv for the transport door to spawn.
    pub(crate) fn as_args(&self) -> &[String] {
        &self.0
    }
}

/// What asking an attached tmux client did.
pub(super) enum ClientPrompt {
    /// Tmux displayed the question and retained the continuation.
    Shown,
    /// No client is attached to the caller's session.
    Nobody,
    /// A client existed, but tmux refused the prompt.
    Failed,
}

/// Ask the most recently active client viewing `session`.
///
/// One prompt has one continuation. Prompting every attached client would let
/// two `y` answers start the destructive operation twice, so the active client
/// is the deterministic owner of this confirmation.
pub(super) fn confirm_on_client(
    server: &ServerId,
    session: &str,
    prompt: &str,
    argv: &DetachedArgv,
) -> ClientPrompt {
    let Some(clients) = transport::observe_clients(server) else {
        return ClientPrompt::Nobody;
    };
    let Some(client) = clients
        .iter()
        .filter(|client| client.session == session && !client.name.is_empty())
        .max_by(|one, other| {
            one.activity
                .unwrap_or_default()
                .cmp(&other.activity.unwrap_or_default())
                .then_with(|| one.name.cmp(&other.name))
        })
    else {
        return ClientPrompt::Nobody;
    };
    let continuation = crate::tmux::run_shell_background_command(argv.as_args());
    if transport::confirm_before(server, &client.name, prompt, &continuation) {
        ClientPrompt::Shown
    } else {
        ClientPrompt::Failed
    }
}

/// Display one outcome on every client still attached to `server`.
pub(super) fn announce_to_clients(server: &ServerId, text: &str) {
    let Some(clients) = transport::observe_clients(server) else {
        return;
    };
    for client in clients {
        if !client.name.is_empty() {
            let _ = transport::display_client_message(server, &client.name, text);
        }
    }
}

/// Move clients watching `name` before its session is killed. The destination
/// follows the same creation/pinned ordering the fleet strip draws.
pub(super) fn handoff_clients_before_kill(server: &ServerId, name: &str) -> Option<String> {
    let sessions = transport::observe_fleet_sessions(server)?;
    let rows: Vec<crate::theme::FleetRow> = sessions
        .iter()
        .map(|session| crate::theme::FleetRow {
            name: session.name.clone(),
            id: session.id.clone(),
            mark: crate::theme::Mark::from_rank(&session.rank),
            current: session.name == name,
        })
        .collect();
    let next = crate::theme::next_fleet_session(&rows, name)?;
    let clients = transport::observe_clients(server)?;
    let attached = clients
        .into_iter()
        .filter(|client| client.session == name)
        .collect::<Vec<_>>();
    if attached.is_empty() {
        return None;
    }
    let failed = attached
        .iter()
        .filter(|client| !transport::switch_client(server, &client.name, &next))
        .count();
    (failed > 0).then(|| {
        format!(
            "client handoff failed: {failed} client(s) could not switch from '{name}' to '{next}'"
        )
    })
}

/// The positive server record of one session.
pub(super) fn recorded_server(root: &Path, name: &str) -> Option<ServerId> {
    let bytes = meta::read_bytes(&sessions_dir(root).join(name)).ok()?;
    match server_of(&bytes) {
        ServerSelector::Positive(selector) => Some(ServerId::Selected(selector)),
        ServerSelector::Missing | ServerSelector::Ambiguous => None,
    }
}

/// The caller's session only when its recorded server is the actual caller.
///
/// Pane ids and session names repeat across tmux servers. Observing a pane on
/// the server `$TMUX` names establishes its name, but that name becomes an ae
/// self-target only after the session's durable selector answers with the same
/// socket identity.
pub(crate) fn recorded_caller_session(
    root: &Path,
    caller_server: Option<&ServerId>,
    pane: &str,
) -> Option<String> {
    let caller_server = caller_server?;
    if pane.is_empty() {
        return None;
    }
    let owner = transport::observe_pane_owner(caller_server, pane)?;
    if !name_is_usable(root, &owner.session) {
        return None;
    }
    let recorded = recorded_server(root, &owner.session)?;
    let mut sockets = crate::SocketPaths::asking(transport::observe_socket_path);
    sockets
        .proven_same(caller_server, &recorded)
        .then_some(owner.session)
}

/// `nohup <this binary> _stop --supervise <name>` — the ONE shape this module
/// can mint, with the session name as its own argv element (no shell, so
/// nothing to inject) and nothing else settable by a caller.
fn supervisor_argv(name: &str) -> Option<DetachedArgv> {
    supervisor_argv_expecting(name, None)
}

/// The same argv, plus the identity a CONFIRMED stop must still find.
fn supervisor_argv_expecting(name: &str, expect: Option<&StopExpectation>) -> Option<DetachedArgv> {
    let own = crate::shape::resolved_exe()?;
    let mut argv = vec![
        own.to_string_lossy().into_owned(),
        crate::cli::STOP.to_owned(),
        "--supervise".to_owned(),
        name.to_owned(),
    ];
    if let Some(expect) = expect {
        argv.extend(expect.argv_words());
    }
    Some(DetachedArgv(argv))
}

/// `<core> _stop --handoff <name>` — the short-lived tmux job which starts
/// [`supervisor_argv`] and exits before the target session is killed.
fn handoff_argv(name: &str) -> Option<DetachedArgv> {
    let own = crate::shape::resolved_exe()?;
    Some(DetachedArgv(vec![
        own.to_string_lossy().into_owned(),
        crate::cli::STOP.to_owned(),
        "--handoff".to_owned(),
        name.to_owned(),
    ]))
}

/// Append one line to the target's event log, best-effort.
pub(super) fn emit_lifecycle_event(dir: &Path, name: &str, action: &str, summary: &str) {
    let _ = crate::store::open(dir).append_event(&crate::tracked::event_line(
        &crate::tracked::EventFields {
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
        },
    ));
}

/// Append one stop event to the target's log, best-effort.
fn emit_stop_event(dir: &Path, name: &str, action: &str, summary: &str) {
    emit_lifecycle_event(dir, name, action, summary);
}

fn stop_prompt(name: &str) -> String {
    format!("Stop '{name}'? Kills the session you are in. (y/n)")
}

/// `_stop --self <name>`: hand the whole stop to a detached supervisor and
/// return.
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
        // ASK WHETHER WE CAN ASK, BEFORE ASKING.
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            let Some(argv) = handoff_argv(name) else {
                writeln!(
                    err,
                    "Error: ae cannot name its own executable, so it cannot hand '{name}' to a supervisor — nothing was stopped."
                )?;
                return Ok(EXIT_FAILED);
            };
            let Some(caller_server) = crate::doors::caller_server() else {
                writeln!(err, "Error: the calling tmux server is unknown; pass -y.")?;
                return Ok(EXIT_FAILED);
            };
            return match confirm_on_client(&caller_server, name, &stop_prompt(name), &argv) {
                ClientPrompt::Shown => Ok(0),
                ClientPrompt::Nobody => {
                    writeln!(err, "Error: nobody attached to confirm; pass -y.")?;
                    Ok(EXIT_FAILED)
                }
                ClientPrompt::Failed => {
                    writeln!(err, "Error: tmux refused the confirmation prompt; pass -y.")?;
                    Ok(EXIT_FAILED)
                }
            };
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
        // THE REQUEST IS ALREADY IN THE LOG, so the outcome has to be too.
        emit_stop_event(
            &dir,
            name,
            STOP_RESULT_ACTION,
            "FAILED: supervisor did not start",
        );
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
/// order.
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

/// Lift the `--expect-*` pairs out of a stop tail; the rest stays in order.
///
/// All ten or none: a partial expectation is a caller that lost a field, and
/// proving only part of the captured identity before a kill is not the contract.
#[allow(
    clippy::too_many_lines,
    reason = "one grammar: ten fields lifted, then each proven, in the order a caller reads them"
)]
fn split_expectation(
    tail: &[String],
) -> std::result::Result<(Option<StopExpectation>, Vec<String>), String> {
    let mut session_id = None;
    let mut uuid = None;
    let mut server_pid = None;
    let mut server_start = None;
    let mut deadline = None;
    let mut client = None;
    let mut client_pid = None;
    let mut server_kind = None;
    let mut server_value = None;
    let mut require_meta_agent = None;
    let mut words: Vec<String> = Vec::with_capacity(tail.len());
    let mut rest = tail.iter();
    while let Some(arg) = rest.next() {
        let slot = match arg.as_str() {
            "--expect-session-id" => &mut session_id,
            "--expect-uuid" => &mut uuid,
            "--expect-server-pid" => &mut server_pid,
            "--expect-server-start" => &mut server_start,
            "--expect-deadline" => &mut deadline,
            "--expect-client" => &mut client,
            "--expect-client-pid" => &mut client_pid,
            "--expect-server-kind" => &mut server_kind,
            "--expect-server" => &mut server_value,
            "--expect-meta-agent" => &mut require_meta_agent,
            _ => {
                words.push(arg.clone());
                continue;
            }
        };
        if slot.is_some() {
            return Err(format!("{arg} may be given only once."));
        }
        let Some(value) = rest.next() else {
            return Err(format!("{arg} requires a value."));
        };
        *slot = Some(value.clone());
    }
    let given = [
        &session_id,
        &uuid,
        &server_pid,
        &server_start,
        &deadline,
        &client,
        &client_pid,
        &server_kind,
        &server_value,
        &require_meta_agent,
    ]
    .iter()
    .filter(|slot| slot.is_some())
    .count();
    if given == 0 {
        return Ok((None, words));
    }
    let (
        Some(session_id),
        Some(uuid),
        Some(server_pid),
        Some(server_start),
        Some(deadline),
        Some(client),
        Some(client_pid),
        Some(server_kind),
        Some(server_value),
        Some(require_meta_agent),
    ) = (
        session_id,
        uuid,
        server_pid,
        server_start,
        deadline,
        client,
        client_pid,
        server_kind,
        server_value,
        require_meta_agent,
    )
    else {
        return Err(
            "an --expect-* identity is incomplete; all ten fields travel together.".to_owned(),
        );
    };
    let server = match (server_kind.as_str(), server_value.as_str()) {
        ("name", name) if !name.is_empty() => ServerId::Selected(Selector::Name(name.to_owned())),
        ("socket", path) if Path::new(path).is_absolute() => {
            ServerId::Selected(Selector::Socket(PathBuf::from(path)))
        }
        // An ambient server is whichever one the caller happened to be in, so
        // it names nothing a later process can prove.
        _ => {
            return Err(
                "--expect-server-kind is 'name' with a name, or 'socket' with an absolute path."
                    .to_owned(),
            );
        }
    };
    let Ok(deadline) = deadline.parse::<i64>() else {
        return Err("--expect-deadline is not an epoch second.".to_owned());
    };
    if !crate::tmux::session_id_is_valid(&session_id) {
        return Err("--expect-session-id is not a tmux session id.".to_owned());
    }
    let uuid = crate::archive::canonical_uuid(&uuid);
    if uuid.is_empty() {
        return Err("--expect-uuid is not a session uuid.".to_owned());
    }
    if !crate::tmux::is_decimal(&server_pid)
        || !crate::tmux::is_decimal(&server_start)
        || !crate::tmux::is_decimal(&client_pid)
    {
        return Err(
            "--expect-server-pid, --expect-server-start and --expect-client-pid are decimals."
                .to_owned(),
        );
    }
    let require_meta_agent = match require_meta_agent.as_str() {
        "true" => true,
        "false" => false,
        _ => return Err("--expect-meta-agent is not true or false.".to_owned()),
    };
    Ok((
        Some(StopExpectation {
            session_id,
            uuid,
            server_pid,
            server_start,
            deadline,
            client,
            client_pid,
            server,
            require_meta_agent,
        }),
        words,
    ))
}

/// `_stop --supervise all`: the whole fleet, one session at a time, from a
/// process no target pane owns.
fn run_supervisor(
    root: &Path,
    name: &str,
    expect: Option<&StopExpectation>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    // `all` IS a legal session name, and a menu can capture a session called
    // that. An expectation names ONE captured session, so it never widens into
    // the fleet form, whatever that session happens to be called. The public
    // fleet syntax below is untouched.
    if name != "all" || expect.is_some() {
        return supervise_one(root, name, expect, out, err);
    }
    // A fleet stop may move a client to the next session that this same sweep
    // will end; once no session remains, tmux detaches it as usual.
    let mut failures = 0_u32;
    for session in all_sessions(root) {
        if supervise_one(root, &session, None, out, err)? != 0 {
            failures += 1;
        }
    }
    Ok(u8::from(failures != 0))
}

/// `_stop --handoff`: detach the real supervisor, then let the tmux
/// `run-shell` job end while the target still exists.
fn start_stop_supervisor(
    root: &Path,
    name: &str,
    _out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let Some(argv) = supervisor_argv(name) else {
        writeln!(
            err,
            "Error: ae cannot name its own executable, so it cannot hand '{name}' to a supervisor — nothing was stopped."
        )?;
        return Ok(EXIT_FAILED);
    };
    if !transport::run_detached(&argv) {
        let dir = sessions_dir(root).join(name);
        emit_stop_event(&dir, name, STOP_REQUEST_ACTION, "stop requested from tmux");
        emit_stop_event(
            &dir,
            name,
            STOP_RESULT_ACTION,
            "FAILED: supervisor did not start",
        );
        writeln!(
            err,
            "Error: could not start the supervisor for '{name}' — nothing was stopped."
        )?;
        return Ok(EXIT_FAILED);
    }
    Ok(0)
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
        // THE SUPERVISOR CANNOT PROMPT; THIS PROCESS STILL CAN.
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            let Some(argv) = handoff_argv("all") else {
                writeln!(
                    err,
                    "Error: could not locate this binary to detach the fleet stop."
                )?;
                return Ok(EXIT_FAILED);
            };
            let prompt =
                format!("Stop all ae sessions? Kills '{own}', the session you are in. (y/n)");
            let Some(caller_server) = crate::doors::caller_server() else {
                writeln!(err, "Error: the calling tmux server is unknown; pass -y.")?;
                return Ok(EXIT_FAILED);
            };
            return match confirm_on_client(&caller_server, own, &prompt, &argv) {
                ClientPrompt::Shown => Ok(0),
                ClientPrompt::Nobody => {
                    writeln!(err, "Error: nobody attached to confirm; pass -y.")?;
                    Ok(EXIT_FAILED)
                }
                ClientPrompt::Failed => {
                    writeln!(err, "Error: tmux refused the confirmation prompt; pass -y.")?;
                    Ok(EXIT_FAILED)
                }
            };
        }
        if !confirm_fleet_stop(all_sessions(root).len(), out, err)? {
            return Ok(EXIT_FAILED);
        }
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
        // Same reason as the single self-stop: a request already recorded and
        // no result is a stop that reads as still running.
        emit_stop_event(
            &dir,
            own,
            STOP_RESULT_ACTION,
            "FAILED: supervisor did not start",
        );
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
    expect: Option<&StopExpectation>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    // A CONFIRMED stop has its own route home: the server the click happened
    // on, carried with it. Every early refusal below reaches the human that
    // way, including the ones where the target's own directory is gone and
    // `recorded_server` could answer nothing at all.
    let refuse = |reason: &str, err: &mut dyn Write| -> io::Result<u8> {
        if let Some(expect) = expect {
            expect.tell_clicker(&format!("Not stopped: {reason}."));
        }
        writeln!(
            err,
            "Error: {reason} — nothing was stopped, state preserved."
        )?;
        Ok(EXIT_FAILED)
    };
    if !name_is_usable(root, name) {
        return refuse(&format!("'{name}' is not a usable session name"), err);
    }
    if !dir_exists(&sessions_dir(root).join(name)) {
        return refuse(&format!("ae has no state for '{name}' any more"), err);
    }
    // This process has no streams a human can read; the record written by the
    // stop itself is the durable outcome. A menu-origin stop answers the ONE
    // client that asked for it; only an ordinary supervisor announces to every
    // client, because only it has no particular human to answer.
    match stop_recorded(root, name, expect, out, err)? {
        StopOutcome::Stopped => {
            let line = format!("Stopped {name}");
            match expect {
                Some(expect) => expect.tell_clicker(&line),
                None => {
                    if let Some(server) = recorded_server(root, name) {
                        announce_to_clients(&server, &line);
                    }
                }
            }
            Ok(0)
        }
        // The target was NOT proven to be the confirmed one, so nothing was
        // written to it. The answer goes to the human who asked, and nowhere.
        StopOutcome::Refused(reason) => refuse(&reason, err),
        StopOutcome::AlreadyStopped | StopOutcome::Failed => {
            if let Some(expect) = expect {
                expect.tell_clicker(&format!("'{name}' was not stopped."));
            }
            Ok(EXIT_FAILED)
        }
    }
}

/// One session's stop, under its own lifecycle lock.
/// Capture each measurable seat's live model and publish it, so a manual
/// choice survives this stop.
///
/// Runs under the caller's lifecycle lock and BEFORE any kill: the panes are
/// still alive, which is exactly what the watchdog's cadence cannot promise.
/// Deliberately best-effort — a session whose meta or panes cannot be read
/// still stops, with one warning from the caller.
fn observe_models_before_stop(dir: &Path, server: &ServerId, name: &str) -> Result<(), String> {
    let bytes = meta::read_bytes(dir).map_err(|why| why.to_string())?;
    let parsed = meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let panes = transport::observe_watch_panes(server, name)
        .ok_or_else(|| "pane enumeration failed".to_owned())?;
    let global = meta_value(&bytes, "config");
    let local = crate::config::local_overlay(dir, &meta_value(&bytes, "origin"));
    let mut failure: Option<String> = None;
    for entry in parsed.roster() {
        let tool =
            crate::tool::ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or_default());
        if tool.adapter().model_flags.is_empty() {
            continue;
        }
        let Some(pane) = panes
            .iter()
            .find(|pane| pane.slot.as_deref() == Some(entry.slot.as_str()))
        else {
            continue;
        };
        let launch_key = format!("launch_id.{}", entry.slot);
        let launch_id = meta::sole_value(&bytes, &launch_key)
            .map(|value| String::from_utf8_lossy(value).into_owned())
            .unwrap_or_default();
        if launch_id.is_empty() {
            continue;
        }
        let pin = entry.profile.as_deref().and_then(|profile| {
            crate::launch_cmd::profile_model_pin(
                (!global.is_empty()).then(|| Path::new(&global)),
                local.as_deref(),
                profile,
                tool,
            )
        });
        let capture = transport::capture_pane(server, &pane.pane_id).unwrap_or_default();
        if let Err(why) = crate::model_drift::observe(
            dir,
            &entry.slot,
            &entry.name,
            tool,
            &capture,
            &launch_id,
            pin.as_deref(),
        ) {
            failure.get_or_insert(why);
        }
    }
    failure.map_or(Ok(()), Err)
}

fn stop_one(
    root: &Path,
    name: &str,
    expect: Option<&StopExpectation>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<StopOutcome> {
    let busy =
        format!("another lifecycle operation (start/resume/end) is in progress for '{name}'");
    let Ok(_guard) = lock(root, name) else {
        // WITHOUT THE LOCK THERE IS NO PROOF. A confirmed stop that cannot
        // reach the critical section has learnt nothing about this directory,
        // so it must not write its failure into it either.
        if expect.is_some() {
            return Ok(StopOutcome::Refused(busy));
        }
        writeln!(err, "Error: {busy} — retry shortly. Nothing was stopped.")?;
        return Ok(StopOutcome::Failed);
    };
    let dir = sessions_dir(root).join(name);
    // THE AUTHORITATIVE POINT, and it comes FIRST. A confirmed stop proves the
    // target is the one the human was shown before this operation reads a
    // migrated meta, rewrites a legacy one or appends a single line to the
    // directory: proving the identity is what earns the right to touch it.
    let confirmed = match expect {
        Some(expect) => match expect.check(&dir, name, crate::time::Timestamp::now().epoch()) {
            ExpectCheck::Refused(reason) => return Ok(StopOutcome::Refused(reason)),
            ExpectCheck::Absent => {
                // Proven ae's and proven already gone: this directory IS the
                // confirmed one, so the durable result belongs in it.
                emit_stop_event(&dir, name, STOP_RESULT_ACTION, "already stopped");
                writeln!(err, "Session '{name}' is not running.")?;
                return Ok(StopOutcome::AlreadyStopped);
            }
            ExpectCheck::Proven { server, id } => {
                // The human's answer, recorded with its provenance, now that
                // the directory has proven it is the one that was confirmed.
                emit_stop_event(
                    &dir,
                    name,
                    STOP_REQUEST_ACTION,
                    &format!(
                        "stop confirmed on the session menu by client {} ({id})",
                        expect.client
                    ),
                );
                Some((server, id))
            }
        },
        None => None,
    };
    // The chain, under the lifecycle lock this stop already holds. A refusal is
    // REPORTED, never fatal: a session whose shape ae cannot place is exactly
    // the one an operator needs to be able to stop.
    if let Some(note) = crate::migrate::session_noted(&dir, name) {
        writeln!(err, "{note}")?;
    }
    // The id a confirmed stop proved is the ONLY one it may kill. Looking the
    // name up a second time would hand a rename or a recreate between the two
    // answers the authority the human never gave: ae's lifecycle lock does not
    // lock tmux.
    let (server, session_id) = if let Some(proven) = confirmed {
        proven
    } else {
        {
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
                // "Empty answer" and "server unreachable" look identical from
                // here, and only one of them means stopped.
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
            (server, session_id)
        }
    };
    // THE DURABLE CUT. The watchdog observes on its cadence; a stop can arrive
    // between the human's model change and the next cycle, so the final
    // observation happens HERE, with the panes still alive and the lifecycle
    // lock held. Best-effort: a failure warns and the stop proceeds.
    if let Err(why) = observe_models_before_stop(&dir, &server, name) {
        writeln!(
            err,
            "Warning: could not observe the seat models before stopping '{name}': {why}"
        )?;
    }
    if expect.is_none() {
        return kill_under_lock(&server, name, &session_id, out, err);
    }
    // The confirmed path keeps its own streams so it can write the durable
    // result BEFORE `_guard` drops: the request line, the kill and the result
    // are ONE critical section over ONE proven identity.
    let mut confirmed_out = Vec::new();
    let mut confirmed_err = Vec::new();
    let outcome = kill_under_lock(
        &server,
        name,
        &session_id,
        &mut confirmed_out,
        &mut confirmed_err,
    )?;
    emit_stop_event(
        &dir,
        name,
        STOP_RESULT_ACTION,
        &stop_summary(&outcome, &confirmed_err),
    );
    out.write_all(&confirmed_out)?;
    err.write_all(&confirmed_err)?;
    Ok(outcome)
}

/// Hand the clients over and kill one exactly-identified session. The caller
/// holds the lifecycle lock and has already decided which id may die.
fn kill_under_lock(
    server: &ServerId,
    name: &str,
    session_id: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<StopOutcome> {
    if let Some(note) = handoff_clients_before_kill(server, name) {
        writeln!(err, "Warning: {note}")?;
    }
    if !kill_verified(server, name, "stop", session_id, err)? {
        return Ok(StopOutcome::Failed);
    }
    writeln!(out, "Stopped {name}")?;
    Ok(StopOutcome::Stopped)
}

#[cfg(test)]
mod tests {
    use super::{name_is_valid, sessions_dir, stop_prompt, worktrees_dir};
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
    fn the_in_pane_stop_prompt_names_the_session_and_the_consequence() {
        assert_eq!(
            stop_prompt("inside"),
            "Stop 'inside'? Kills the session you are in. (y/n)"
        );
    }

    #[test]
    fn the_roots_are_the_inventory_roots() {
        assert_eq!(sessions_dir(Path::new("/x")), Path::new("/x/sessions"));
        assert_eq!(worktrees_dir(Path::new("/x")), Path::new("/x/worktrees"));
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the fixture builds and re-modes a real root; the boundary is on product code"
    )]
    fn an_absent_sessions_root_is_a_different_answer_from_an_unreadable_one() {
        use std::os::unix::fs::PermissionsExt as _;

        // The distinction the version sweep spends: `None` is "this root has
        // never had a session" and is safe to treat as empty, while an error is
        // a reading that FAILED. They must not both arrive as a `NotFound`
        // error kind either — an entry that vanishes mid-walk raises that too,
        // and it is a partial reading, not an absent root.
        let root =
            std::path::PathBuf::from(format!("/tmp/ae-census-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(matches!(super::census(&root), Ok(None)), "an absent root");

        let sessions = sessions_dir(&root);
        assert!(std::fs::create_dir_all(sessions.join("one")).is_ok());
        assert_eq!(
            super::census(&root).ok().flatten(),
            Some(vec!["one".to_owned()]),
            "a root with one session"
        );

        assert!(
            std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o000)).is_ok()
        );
        let unreadable = super::census(&root);
        let _ = std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o755));
        assert!(
            unreadable.is_err(),
            "an unreadable root answered {unreadable:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
