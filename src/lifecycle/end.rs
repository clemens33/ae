//! `_end`: the whole end operation.
//!
//! CAPTURE THEN DELETE, and the order is the contract. The archive is published
//! AFTER the session is verifiably stopped and after git has had its say, and
//! BEFORE any live state is removed — so a failed archive returns non-zero with
//! the whole session still on disk. ae never deletes a session it could not
//! capture.

use std::io::{self, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use crate::archive;
use crate::config::{Workspace, read_workspace};
use crate::inventory::ServerId;
use crate::meta::{self, Selector, ServerSelector};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;
use crate::tmux::StopProbe;
use crate::tool::ToolKind;
use crate::transport;

use super::{
    ClientPrompt, DetachedArgv, all_sessions, announce_to_clients, claims, confirm_on_client,
    dir_exists, handoff_clients_before_kill, kill_verified, lock, meta_value, name_is_usable,
    path_exists, recorded_server, server_from_words, server_kind_word, server_of,
    server_value_word, sessions_dir, views_gone, worktrees_dir,
};

/// The usage line.
const USAGE: &str =
    "Usage: _end [-f] [--purge-history|--keep-history] [--assume-stopped] <session-name|all>";

#[cfg(test)]
mod end_expect_spec;
pub(crate) mod transaction;

const END_REQUEST_ACTION: &str = "end-request";
const END_RESULT_ACTION: &str = "end-result";

/// What `ae end` will do with a session's memory, in five answers.
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

impl Action {
    fn word(&self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Purge => "purge",
            Self::Nothing => "nothing",
            Self::Unavailable => "unavailable",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        match word {
            "keep" => Some(Self::Keep),
            "purge" => Some(Self::Purge),
            "nothing" => Some(Self::Nothing),
            "unavailable" => Some(Self::Unavailable),
            _ => None,
        }
    }
}

/// One target's resolved plan.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Plan {
    action: Action,
    detail: String,
    purge: bool,
}

/// One prompt-time plan carried across the tmux and detached-process handoffs.
#[derive(Debug, Clone)]
struct ConfirmedPlan {
    name: String,
    plan: Plan,
    /// The history choice came from config, so config remains part of the
    /// changed-plan check under the lifecycle lock.
    from_default: bool,
    /// The tmux incarnation observed in the same resolution that rendered the
    /// prompt, re-proven under the lifecycle lock.
    tmux: FrozenTmux,
}

#[derive(Default)]
struct ConfirmedParts {
    name: String,
    action: Option<Action>,
    detail: Option<String>,
    purge: Option<bool>,
    from_default: Option<bool>,
    tmux: Option<FrozenTmux>,
}

/// One tmux look at a positively recorded server — the ONE reading helper the
/// prompt resolution and the under-lock re-proof share, in today's read shape:
/// a listing hit IS presence; only a miss pays for the strict probe.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TmuxSight {
    Live { id: String, created: String },
    Absent,
    Unknown,
}

fn sight(server: &ServerId, name: &str) -> TmuxSight {
    if let Some(identity) = transport::observe_session_identity(server, name) {
        return TmuxSight::Live {
            id: identity.id,
            created: identity.created,
        };
    }
    if transport::verify_session_absent(server, name) == StopProbe::Absent {
        TmuxSight::Absent
    } else {
        TmuxSight::Unknown
    }
}

/// Whether ending a session of `mode` commits and pushes its work, as
/// [`end_locked`] branches on it. `None` is a mode that would reach its git
/// branch without being one: no question can say what ending it does.
fn pushes_of(mode: &str) -> Option<bool> {
    match mode {
        "local" | "" => Some(false),
        "git" | "full" => Some(true),
        _ => None,
    }
}

/// The prompt-time tmux reading, frozen with the plan and re-proven under the
/// lifecycle lock. `Unobserved` promises nothing: no positive server record at
/// the prompt, or `-f`, which freezes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FrozenTmux {
    Live { id: String, created: String },
    Absent,
    Unknown,
    Unobserved,
}

impl FrozenTmux {
    /// The `Confirmed:` line of a refusal.
    fn describe(&self) -> String {
        match self {
            Self::Live { id, created } => describe_live(id, created),
            Self::Absent => "no tmux session (observed absent)".to_owned(),
            Self::Unknown => "tmux unobservable (server unreachable)".to_owned(),
            Self::Unobserved => "nothing (no tmux promise)".to_owned(),
        }
    }

    /// Parse the `--confirmed-tmux=` word: `absent`, `unknown`,
    /// `unobserved`, or `$<n>|<created>`.
    fn from_word(word: &str) -> Option<Self> {
        match word {
            "absent" => Some(Self::Absent),
            "unknown" => Some(Self::Unknown),
            "unobserved" => Some(Self::Unobserved),
            live => {
                let (id, created) = live.split_once('|')?;
                if !crate::tmux::session_id_is_valid(id) || !crate::tmux::is_decimal(created) {
                    return None;
                }
                Some(Self::Live {
                    id: id.to_owned(),
                    created: created.to_owned(),
                })
            }
        }
    }

    /// The `--confirmed-tmux=` word.
    fn word(&self) -> String {
        match self {
            Self::Absent => "absent".to_owned(),
            Self::Unknown => "unknown".to_owned(),
            Self::Unobserved => "unobserved".to_owned(),
            Self::Live { id, created } => format!("{id}|{created}"),
        }
    }
}

impl TmuxSight {
    /// The `Now:` line of a refusal.
    fn describe(&self, name: &str) -> String {
        match self {
            Self::Live { id, created } => describe_live(id, created),
            Self::Absent => format!("no session '{name}' on its recorded server"),
            Self::Unknown => "recorded tmux server unreachable".to_owned(),
        }
    }
}

fn describe_live(id: &str, created: &str) -> String {
    format!("tmux {id} (created {created})")
}

/// What the re-proof found — PURE, table-tested: every frozen × seen cell.
enum Reproof {
    /// Proceed exactly as today: a match, nothing promised, or an unreachable
    /// server the existing assume-stopped rule still governs.
    Proceed,
    /// Refuse; the first line of the refusal, naming the mismatch.
    Refuse(String),
}

/// Re-prove the frozen incarnation against the under-lock sight. A target
/// that no longer matches is refused with both versions; nothing is touched.
fn reprove(name: &str, frozen: &FrozenTmux, seen: &TmuxSight) -> Reproof {
    match (frozen, seen) {
        (
            FrozenTmux::Live { id, created },
            TmuxSight::Live {
                id: now,
                created: born,
            },
        ) if id == now && created == born => Reproof::Proceed,
        (FrozenTmux::Live { .. }, TmuxSight::Live { .. }) => Reproof::Refuse(format!(
            "'{name}' is a different tmux session than the one confirmed."
        )),
        (FrozenTmux::Live { .. }, TmuxSight::Absent) => {
            Reproof::Refuse(format!("the tmux session confirmed for '{name}' is gone."))
        }
        (FrozenTmux::Absent, TmuxSight::Live { .. }) => Reproof::Refuse(format!(
            "'{name}' is live again, but it was absent when confirmed."
        )),
        (FrozenTmux::Unknown, TmuxSight::Live { .. }) => Reproof::Refuse(format!(
            "'{name}' holds a tmux session ae could not see when it was confirmed."
        )),
        (FrozenTmux::Unknown, TmuxSight::Absent) => Reproof::Refuse(format!(
            "'{name}' is absent, but ae could not see its tmux server when it was confirmed."
        )),
        (FrozenTmux::Absent, TmuxSight::Absent)
        | (FrozenTmux::Unobserved, _)
        | (_, TmuxSight::Unknown) => Reproof::Proceed,
    }
}

/// One target's frozen question: the plan the prompt rendered plus the tmux
/// incarnation observed in that same resolution.
struct FrozenTarget {
    name: String,
    plan: Plan,
    tmux: FrozenTmux,
}

/// One target's frozen question: the carried plan when one rode in, else the
/// freshly resolved plan plus the tmux reading — unless `-f`, which freezes
/// nothing because nothing was promised.
fn freeze_target(root: &Path, args: &Args, name: &str) -> FrozenTarget {
    if let Some(confirmed) = confirmed_for(args, name) {
        FrozenTarget {
            name: name.to_owned(),
            plan: confirmed.plan.clone(),
            tmux: confirmed.tmux.clone(),
        }
    } else {
        let plan = resolve_plan(root, name, args.purge_cli);
        let tmux = if args.force {
            FrozenTmux::Unobserved
        } else {
            freeze_tmux(root, name)
        };
        FrozenTarget {
            name: name.to_owned(),
            plan,
            tmux,
        }
    }
}

/// The prompt-time tmux reading for `name` on its positively recorded server,
/// or `Unobserved` when there is no record to observe on.
fn freeze_tmux(root: &Path, name: &str) -> FrozenTmux {
    let bytes = meta::read_bytes(&sessions_dir(root).join(name)).unwrap_or_default();
    let ServerSelector::Positive(selector) = server_of(&bytes) else {
        return FrozenTmux::Unobserved;
    };
    match sight(&ServerId::Selected(selector), name) {
        TmuxSight::Live { id, created } => FrozenTmux::Live { id, created },
        TmuxSight::Absent => FrozenTmux::Absent,
        TmuxSight::Unknown => FrozenTmux::Unknown,
    }
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

/// What the human confirmed in `ae app`'s End… question, for the one literal
/// session that question named.
///
/// The app's modal IS the confirmation, inside tmux or not, so no client is
/// named or needed. Nothing here is believed when it is captured: it travels
/// with the operation to the one process that holds the target's lifecycle
/// lock, which proves all of it before its first write.
#[derive(Debug, Clone)]
pub(crate) struct EndExpectation {
    /// ae's own identity for the session's state directory.
    uuid: String,
    /// The server the record named when the question was asked.
    server: ServerId,
    /// That server's process, proven for a live target only: a stopped
    /// session's server may be another process, or none, and that is no
    /// change to what was confirmed.
    identity: Option<crate::tmux::ServerIdentity>,
    /// The epoch second after which the answer is stale. Carried, never
    /// renewed: a supervisor that starts late is as stale as its answer.
    deadline: i64,
    /// The plan and the tmux reading the question rendered.
    confirmed: ConfirmedPlan,
    /// The server is gone and the boot-time proof held when the question
    /// asked, so the question said so: the flag of invariant (b). The proof
    /// itself is never carried, only re-proven.
    assume_stopped: bool,
    /// The session's mode commits and pushes its work on the way out: what
    /// the question said, carried, and proven again against the record.
    pushes: bool,
}

impl EndExpectation {
    /// What `name`'s End… question freezes from one resolution of its plan
    /// and tmux reading, or why it may not be asked. `stopped` is what the
    /// card drew: only a stopped card may rest on the boot-time proof.
    pub(crate) fn unattended(
        root: &Path,
        name: &str,
        uuid: &str,
        server: ServerId,
        stopped: bool,
        deadline: i64,
    ) -> Result<Self, String> {
        let dir = sessions_dir(root).join(name);
        let plan = resolve_plan(root, name, None);
        if plan.action == Action::Unavailable {
            return Err(format!("it cannot be archived: {}", plan.detail));
        }
        let bytes = meta::read_bytes(&dir).unwrap_or_default();
        let pushes = pushes_of(&meta_value(&bytes, "mode"))
            .ok_or("its record names a mode ae does not know")?;
        let (tmux, identity, assume_stopped) = match sight(&server, name) {
            TmuxSight::Live { id, created } => {
                let identity = transport::observe_server_identity(&server)
                    .ok_or("the tmux server its record names did not answer")?;
                (FrozenTmux::Live { id, created }, Some(identity), false)
            }
            TmuxSight::Absent => (FrozenTmux::Absent, None, false),
            TmuxSight::Unknown if stopped && boot_proved_stopped(&dir, &server, name) => {
                (FrozenTmux::Unknown, None, true)
            }
            TmuxSight::Unknown => {
                return Err(format!(
                    "its tmux server did not answer; if it is stopped: ae end --assume-stopped {name}"
                ));
            }
        };
        Ok(Self {
            uuid: uuid.to_owned(),
            server,
            identity,
            deadline,
            confirmed: ConfirmedPlan {
                name: name.to_owned(),
                plan,
                from_default: true,
                tmux,
            },
            assume_stopped,
            pushes,
        })
    }

    /// The `session_id` the question was asked about.
    pub(crate) fn uuid(&self) -> &str {
        &self.uuid
    }

    /// The server the question was asked about.
    pub(crate) fn server(&self) -> &ServerId {
        &self.server
    }

    /// The epoch second the answer goes stale.
    pub(crate) fn deadline(&self) -> i64 {
        self.deadline
    }

    /// The tmux session id the answer may kill, when one is live.
    pub(crate) fn live_id(&self) -> Option<&str> {
        match &self.confirmed.tmux {
            FrozenTmux::Live { id, .. } => Some(id),
            _ => None,
        }
    }

    /// What ending it will do, as the question says it, from the frozen plan.
    pub(crate) fn lines(&self) -> Vec<String> {
        let plan = &self.confirmed.plan;
        let mut lines = vec![
            match plan.action {
                Action::Purge => "NO archive, and any archive of it is DELETED.",
                Action::Nothing => "Nothing to archive; its state is deleted.",
                Action::Keep | Action::Unavailable => "Archives it, then deletes its state.",
            }
            .to_owned(),
        ];
        lines.push(
            if plan.purge {
                "Its agents' conversation files are DELETED."
            } else {
                "Its agents' conversation files are kept."
            }
            .to_owned(),
        );
        if self.pushes {
            lines.push("Commits and pushes its work first.".to_owned());
        }
        if self.assume_stopped {
            lines.push("Server gone; proven stopped by the host reboot.".to_owned());
        }
        lines
    }

    /// The detached owner's exact argv, on `core`.
    fn argv(&self, core: String) -> DetachedArgv {
        let name = &self.confirmed.name;
        let mut command = vec![
            core,
            crate::cli::END.to_owned(),
            "--supervise".to_owned(),
            name.clone(),
        ];
        push_confirmed(&mut command, &self.confirmed);
        if self.assume_stopped {
            command.push("--assume-stopped".to_owned());
        }
        let (pid, start) = self
            .identity
            .as_ref()
            .map_or(("", ""), |found| (found.pid.as_str(), found.start.as_str()));
        command.extend([
            format!("--expect-uuid={}", self.uuid),
            format!("--expect-server-kind={}", server_kind_word(&self.server)),
            format!("--expect-server={}", server_value_word(&self.server)),
            format!("--expect-server-pid={pid}"),
            format!("--expect-server-start={start}"),
            format!("--expect-deadline={}", self.deadline),
            format!("--expect-pushes={}", if self.pushes { "on" } else { "off" }),
        ]);
        DetachedArgv(command)
    }

    /// Prove the expectation against the world, under the caller's lock.
    ///
    /// Reads only: a target this cannot prove comes out of the operation byte
    /// for byte as it went in. The ordinary end re-proves its tmux reading and
    /// plan after the migration; a confirmed one proves both here, first.
    fn check(&self, root: &Path, dir: &Path, name: &str, now: i64) -> Result<(), String> {
        if now > self.deadline {
            return Err(format!(
                "the confirmation for '{name}' expired before it could be applied"
            ));
        }
        if now < self.deadline - crate::session_menu::CONFIRM_WINDOW_SECS {
            return Err(format!(
                "the confirmation for '{name}' is stamped in the future"
            ));
        }
        // THE RAW META, never a migrated one.
        let Ok(bytes) = meta::read_bytes(dir) else {
            return Err(format!("ae cannot read the metadata of '{name}'"));
        };
        // EXACTLY ONE row: a doubled identity does not say which was confirmed.
        let uuid = meta::sole_value(&bytes, "session_id")
            .map(|value| archive::canonical_uuid(&String::from_utf8_lossy(value)));
        if uuid.as_deref() != Some(self.uuid.as_str()) {
            return Err(format!(
                "the state directory of '{name}' was replaced since it was confirmed"
            ));
        }
        if pushes_of(&meta_value(&bytes, "mode")) != Some(self.pushes) {
            return Err(format!(
                "whether ending '{name}' commits and pushes changed between the confirmation and now"
            ));
        }
        let ServerSelector::Positive(selector) = server_of(&bytes) else {
            return Err(format!(
                "'{name}' has no positive server record, so ae cannot prove what was confirmed"
            ));
        };
        // A server that is gone answers nothing, so its spelling is all that
        // can match; a live one may be spelled two ways.
        let recorded = ServerId::Selected(selector);
        if recorded != self.server
            && !crate::fleet_read::SocketPaths::live().proven_same(&self.server, &recorded)
        {
            return Err(format!(
                "'{name}' is recorded on a different tmux server than the one it was confirmed on"
            ));
        }
        if let Some(identity) = &self.identity
            && transport::observe_server_identity(&self.server).as_ref() != Some(identity)
        {
            return Err(format!(
                "the tmux server was replaced since '{name}' was confirmed"
            ));
        }
        self.check_tmux(dir, name)?;
        let purge = (!self.confirmed.from_default).then_some(self.confirmed.plan.purge);
        if resolve_plan(root, name, purge) != self.confirmed.plan {
            return Err(format!(
                "what '{name}' would do changed between the confirmation and now"
            ));
        }
        Ok(())
    }

    /// The tmux half of [`Self::check`]. An unreachable server earns nothing
    /// by itself, so it passes only what the question asked on that same
    /// footing, with the flag, and with the boot-time proof holding NOW.
    fn check_tmux(&self, dir: &Path, name: &str) -> Result<(), String> {
        let seen = sight(&self.server, name);
        if seen == TmuxSight::Unknown {
            let proved = self.confirmed.tmux == FrozenTmux::Unknown
                && self.assume_stopped
                && boot_proved_stopped(dir, &self.server, name);
            if proved {
                return Ok(());
            }
            return Err(format!(
                "the tmux server of '{name}' did not answer, so ae cannot prove what was confirmed"
            ));
        }
        match reprove(name, &self.confirmed.tmux, &seen) {
            Reproof::Proceed => Ok(()),
            Reproof::Refuse(first) => Err(first),
        }
    }
}

/// The `--expect-*=` words of one `_end` argv, as typed: all seven, or none.
#[derive(Default)]
struct ExpectWords {
    uuid: Option<String>,
    kind: Option<String>,
    server: Option<String>,
    pid: Option<String>,
    start: Option<String>,
    deadline: Option<String>,
    pushes: Option<String>,
}

impl ExpectWords {
    /// Lift one `--expect-*=value` word.
    fn take(&mut self, flag: &str) -> Result<(), String> {
        let (key, value) = flag.split_once('=').unwrap_or((flag, ""));
        let slot = match key {
            "--expect-uuid" => &mut self.uuid,
            "--expect-server-kind" => &mut self.kind,
            "--expect-server" => &mut self.server,
            "--expect-server-pid" => &mut self.pid,
            "--expect-server-start" => &mut self.start,
            "--expect-deadline" => &mut self.deadline,
            "--expect-pushes" => &mut self.pushes,
            _ => return Err(format!("Error: unknown flag '{flag}'.")),
        };
        if !flag.contains('=') {
            return Err(format!("Error: {key} requires a value."));
        }
        if slot.replace(value.to_owned()).is_some() {
            return Err(format!("Error: {key} may be given only once."));
        }
        Ok(())
    }

    /// Whether any word was given.
    fn any(&self) -> bool {
        [
            &self.uuid,
            &self.kind,
            &self.server,
            &self.pid,
            &self.start,
            &self.deadline,
            &self.pushes,
        ]
        .iter()
        .any(|slot| slot.is_some())
    }

    /// The expectation these words and `args` name, proven well formed: it
    /// belongs to a supervisor with exactly one carried plan, for its operand.
    fn build(self, args: &Args) -> Result<EndExpectation, String> {
        let incomplete =
            || "Error: an --expect-* identity is incomplete; all seven travel together.";
        let uuid = self.uuid.ok_or_else(incomplete)?;
        let kind = self.kind.ok_or_else(incomplete)?;
        let server = self.server.ok_or_else(incomplete)?;
        let pid = self.pid.ok_or_else(incomplete)?;
        let start = self.start.ok_or_else(incomplete)?;
        let deadline = self.deadline.ok_or_else(incomplete)?;
        let pushes = self.pushes.ok_or_else(incomplete)?;
        if args.mode != RunMode::Supervise {
            return Err("Error: an --expect-* identity needs --supervise.".to_owned());
        }
        let [confirmed] = args.confirmed.as_slice() else {
            return Err("Error: an --expect-* identity needs one carried plan.".to_owned());
        };
        if confirmed.name != args.target {
            return Err(format!(
                "Error: the carried end plan does not match '{}'.",
                args.target
            ));
        }
        if confirmed.plan.action == Action::Unavailable || confirmed.tmux == FrozenTmux::Unobserved
        {
            return Err("Error: a confirmed end needs a plan it can carry out.".to_owned());
        }
        let server = server_from_words(&kind, &server).ok_or(
            "Error: --expect-server-kind is 'name' with a name, or 'socket' with an absolute path.",
        )?;
        let uuid = archive::canonical_uuid(&uuid);
        if uuid.is_empty() {
            return Err("Error: --expect-uuid is not a session uuid.".to_owned());
        }
        let deadline = deadline
            .parse::<i64>()
            .map_err(|_| "Error: --expect-deadline is not an epoch second.".to_owned())?;
        let pushes = match pushes.as_str() {
            "on" => true,
            "off" => false,
            _ => return Err("Error: --expect-pushes is on or off.".to_owned()),
        };
        let live = matches!(confirmed.tmux, FrozenTmux::Live { .. });
        let identity = match (pid.as_str(), start.as_str(), live) {
            ("", "", false) => None,
            (pid, start, true)
                if crate::tmux::is_decimal(pid) && crate::tmux::is_decimal(start) =>
            {
                Some(crate::tmux::ServerIdentity {
                    pid: pid.to_owned(),
                    start: start.to_owned(),
                })
            }
            _ => {
                return Err(
                    "Error: --expect-server-pid and --expect-server-start are decimals for a live target and empty otherwise."
                        .to_owned(),
                );
            }
        };
        Ok(EndExpectation {
            uuid,
            server,
            identity,
            deadline,
            confirmed: confirmed.clone(),
            assume_stopped: args.assume_stopped,
            pushes,
        })
    }
}

/// What the argv said.
#[derive(Clone)]
struct Args {
    target: String,
    force: bool,
    assume_stopped: bool,
    mode: RunMode,
    pane: Option<String>,
    confirmed: Vec<ConfirmedPlan>,
    /// `Some(true)` for `--purge-history`, `Some(false)` for `--keep-history`,
    /// `None` when the caller passed neither and each session's OWN config
    /// decides.
    purge_cli: Option<bool>,
    /// What a supervisor must prove under the lock before it touches `target`,
    /// which it then treats as one literal session name.
    expect: Option<EndExpectation>,
}

/// Which process owns the destructive sequence.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RunMode {
    Direct,
    Handoff,
    Supervise,
}

/// `_end [-f] [--purge-history|--keep-history] [--assume-stopped] <name|all>`.
///
/// One ordered preflight serves the confirmation, handoff and direct paths:
/// argv, caller, target, mode, targets, rename, freeze, who is asked, then
/// the end itself. Each step below is named, in that order.
pub(crate) fn run(
    root: &Path,
    tail: &[String],
    ambient_pane: Option<&str>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let mut args = match parse(tail) {
        Ok(args) => args,
        Err(message) => {
            writeln!(err, "{message}")?;
            return Ok(EXIT_USAGE);
        }
    };
    if args.pane.is_none() {
        args.pane = ambient_pane.map(ToOwned::to_owned);
    }
    let caller_server = crate::doors::caller_server();
    let caller_session = args
        .pane
        .as_deref()
        .and_then(|pane| super::recorded_caller_session(root, caller_server.as_ref(), pane));
    if args.target.is_empty() {
        let Some(own) = &caller_session else {
            writeln!(err, "{USAGE}")?;
            return Ok(EXIT_USAGE);
        };
        args.target.clone_from(own);
    }
    if args.mode == RunMode::Supervise {
        return run_supervised(root, &args, out, err);
    }
    // The stopped acknowledgement is PER-TARGET destructive intent — never
    // valid for 'all'.
    if args.target == "all" && args.assume_stopped {
        writeln!(
            err,
            "Error: --assume-stopped is per-target only — not valid with 'all'."
        )?;
        return Ok(EXIT_USAGE);
    }

    // RESOLVE BEFORE PROMPTING.
    let targets = match targets_of(root, &args, err)? {
        ControlFlow::Continue(targets) => targets,
        ControlFlow::Break(code) => return Ok(code),
    };
    if let ControlFlow::Break(code) = rename_pending(root, &targets, err)? {
        return Ok(code);
    }

    // ONE resolution: the prompt renders from these fields and the frozen
    // contract is built from the same ones.
    let frozen: Vec<FrozenTarget> = targets
        .iter()
        .map(|name| freeze_target(root, &args, name))
        .collect();

    match prompt_route(args.mode, args.force, caller_session.as_deref(), || {
        std::io::IsTerminal::is_terminal(&std::io::stdin())
    }) {
        Prompt::Handoff => {
            if frozen.is_empty() {
                writeln!(out, "No ae sessions.")?;
                return Ok(0);
            }
            return handoff(
                root,
                &args,
                &frozen,
                caller_session.as_deref(),
                false,
                out,
                err,
            );
        }
        Prompt::Client(caller) => {
            return confirm_on_caller_client(caller_server.as_ref(), caller, &args, &frozen, err);
        }
        Prompt::Terminal => {
            if let ControlFlow::Break(code) = confirm_in_terminal(root, &args, &frozen, err)? {
                return Ok(code);
            }
        }
        Prompt::Forced => {}
    }

    if frozen.is_empty() {
        writeln!(out, "No ae sessions.")?;
        return Ok(0);
    }

    if caller_session
        .as_ref()
        .is_some_and(|caller| targets.iter().any(|target| target == caller))
    {
        return handoff(
            root,
            &args,
            &frozen,
            caller_session.as_deref(),
            true,
            out,
            err,
        );
    }

    end_each(root, &args, &frozen, out, err)
}

/// The names this end is about: the carried plans of a handoff, the census
/// for `all`, or the one named session, which must exist.
fn targets_of(
    root: &Path,
    args: &Args,
    err: &mut impl Write,
) -> io::Result<ControlFlow<u8, Vec<String>>> {
    if args.mode == RunMode::Handoff && !args.confirmed.is_empty() {
        let Some(carried) = carried_targets(&args.target, &args.confirmed) else {
            writeln!(
                err,
                "Error: carried end plans do not match '{}'.",
                args.target
            )?;
            return Ok(ControlFlow::Break(EXIT_USAGE));
        };
        return Ok(ControlFlow::Continue(carried));
    }
    if args.target == "all" {
        return Ok(ControlFlow::Continue(all_sessions(root)));
    }
    if !name_is_usable(root, &args.target) {
        writeln!(err, "Session '{}' not found.", args.target)?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    let dir = sessions_dir(root).join(&args.target);
    if !dir_exists(&dir) && !dir_exists(&worktrees_dir(root).join(&args.target)) {
        writeln!(err, "Session '{}' not found.", args.target)?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    Ok(ControlFlow::Continue(vec![args.target.clone()]))
}

/// A pending rename transaction owns its names until its proved retry
/// converges: an end must not tear down a mixed generation as ordinary state.
/// The first blocked target, in order, refuses the whole end.
fn rename_pending(
    root: &Path,
    targets: &[String],
    err: &mut impl Write,
) -> io::Result<ControlFlow<u8>> {
    for target in targets {
        if let Some(blocked) = crate::rename::intent_blocks(root, target) {
            writeln!(err, "Error: {blocked}. Nothing was ended.")?;
            return Ok(ControlFlow::Break(crate::state::EXIT_FAILED));
        }
    }
    Ok(ControlFlow::Continue(()))
}

/// No terminal, inside a session: the caller's attached tmux client is asked,
/// and its `y` runs the handoff continuation. This process ends nothing.
fn confirm_on_caller_client(
    caller_server: Option<&ServerId>,
    caller: &str,
    args: &Args,
    frozen: &[FrozenTarget],
    err: &mut impl Write,
) -> io::Result<u8> {
    let Some(continuation) = handoff_argv(args, frozen) else {
        writeln!(
            err,
            "Error: ae cannot name its own executable, so it cannot hand '{}' to a supervisor — nothing was ended.",
            args.target
        )?;
        return Ok(EXIT_FAILED);
    };
    let Some(caller_server) = caller_server else {
        writeln!(err, "Error: the calling tmux server is unknown; pass -f.")?;
        return Ok(EXIT_FAILED);
    };
    match confirm_on_client(
        caller_server,
        caller,
        &end_prompt(&args.target, frozen),
        &continuation,
    ) {
        ClientPrompt::Shown => Ok(0),
        ClientPrompt::Nobody => {
            writeln!(err, "Error: nobody attached to confirm; pass -f.")?;
            Ok(EXIT_FAILED)
        }
        ClientPrompt::Failed => {
            writeln!(err, "Error: tmux refused the confirmation prompt; pass -f.")?;
            Ok(EXIT_FAILED)
        }
    }
}

/// The terminal question. Only a yes continues to the end.
fn confirm_in_terminal(
    root: &Path,
    args: &Args,
    frozen: &[FrozenTarget],
    err: &mut impl Write,
) -> io::Result<ControlFlow<u8>> {
    // Set REGARDLESS of how many targets there were: the human was asked,
    // and what they were asked about is now the whole of what may be ended.
    confirm_body(root, args, frozen, err)?;
    write!(err, "Continue? [y/N] ")?;
    err.flush()?;
    match answer(read_reply().as_deref()) {
        Answer::Yes => Ok(ControlFlow::Continue(())),
        Answer::No => {
            writeln!(err, "Aborted.")?;
            Ok(ControlFlow::Break(0))
        }
        Answer::NoInput => {
            writeln!(
                err,
                "Error: could not obtain confirmation — no input on stdin."
            )?;
            writeln!(
                err,
                "  Nothing was stopped and nothing was deleted. Run it from a terminal, or"
            )?;
            writeln!(err, "  pass -f if you mean to proceed without being asked.")?;
            Ok(ControlFlow::Break(EXIT_FAILED))
        }
    }
}

/// End EXACTLY the list the human was shown, in that order, and count what
/// failed.
fn end_each(
    root: &Path,
    args: &Args,
    frozen: &[FrozenTarget],
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let mut failures = 0_u32;
    for target in frozen {
        let contract = contract_for(args, target);
        if !end_one(root, &target.name, args, contract, out, err)? {
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

/// The carried names a handoff ends, in carried order, when they answer the
/// operand — `all`, or exactly one plan for the named target — and no name
/// rides twice.
fn carried_targets(target: &str, confirmed: &[ConfirmedPlan]) -> Option<Vec<String>> {
    let carried: Vec<String> = confirmed
        .iter()
        .map(|confirmed| confirmed.name.clone())
        .collect();
    let matches_operand = target == "all"
        || (carried.len() == 1 && carried.first().map(String::as_str) == Some(target));
    let unique = carried
        .iter()
        .enumerate()
        .all(|(index, name)| !carried[..index].contains(name));
    (matches_operand && unique).then_some(carried)
}

/// Who answers for an end that is not supervised.
#[derive(Debug, PartialEq, Eq)]
enum Prompt<'a> {
    /// `--handoff`: the human already answered; nobody is asked again.
    Handoff,
    /// No terminal on stdin, inside a session: the caller's attached client.
    Client(&'a str),
    /// The terminal on stdin, or nothing at all.
    Terminal,
    /// `-f`: nobody is asked.
    Forced,
}

/// Who answers. `stdin_tty` is asked only for an unforced direct end, before
/// the caller is consulted, so stdin is looked at exactly when a prompt is due.
fn prompt_route(
    mode: RunMode,
    force: bool,
    caller: Option<&str>,
    stdin_tty: impl FnOnce() -> bool,
) -> Prompt<'_> {
    if mode == RunMode::Handoff {
        return Prompt::Handoff;
    }
    if force {
        return Prompt::Forced;
    }
    let tty = stdin_tty();
    match caller {
        Some(caller) if !tty => Prompt::Client(caller),
        _ => Prompt::Terminal,
    }
}

/// What one line typed at the terminal question means.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Yes,
    No,
    NoInput,
}

/// `y` or `Y` first is yes; any other line is no; no line at all is no input.
fn answer(reply: Option<&str>) -> Answer {
    match reply {
        Some(reply) if reply.starts_with('y') || reply.starts_with('Y') => Answer::Yes,
        Some(_) => Answer::No,
        None => Answer::NoInput,
    }
}

/// What `target` is ended under: the plan carried for it, else the plan the
/// human was shown — unless `-f`, which freezes nothing because nothing was
/// promised.
fn contract_for<'a>(
    args: &'a Args,
    target: &'a FrozenTarget,
) -> Option<(&'a Plan, &'a FrozenTmux)> {
    confirmed_for(args, &target.name)
        .map(|confirmed| (&confirmed.plan, &confirmed.tmux))
        .or_else(|| (!args.force).then_some((&target.plan, &target.tmux)))
}

#[allow(
    clippy::too_many_lines,
    reason = "the six confirmed-* flags share one argv shape; splitting the match would put the wire grammar in two places"
)]
fn parse(tail: &[String]) -> Result<Args, String> {
    let mut args = Args {
        target: String::new(),
        force: false,
        assume_stopped: false,
        mode: RunMode::Direct,
        pane: None,
        confirmed: Vec::new(),
        purge_cli: None,
        expect: None,
    };
    let mut confirmed = Vec::<ConfirmedParts>::new();
    let mut expect = ExpectWords::default();
    for arg in tail {
        match arg.as_str() {
            "-f" | "--force" => args.force = true,
            "--supervise" => args.mode = RunMode::Supervise,
            "--handoff" => args.mode = RunMode::Handoff,
            "--assume-stopped" => args.assume_stopped = true,
            "--purge-history" => args.purge_cli = Some(true),
            "--keep-history" => args.purge_cli = Some(false),
            "--pane" => {
                return Err("Error: --pane needs a pane id as its next argument.".to_owned());
            }
            flag if flag.starts_with("--pane=") => {
                args.pane = flag.strip_prefix("--pane=").map(ToOwned::to_owned);
            }
            flag if flag.starts_with("--confirmed-target=") => {
                let name = flag.strip_prefix("--confirmed-target=").unwrap_or_default();
                if name.is_empty() {
                    return Err("Error: a carried end plan has no target.".to_owned());
                }
                confirmed.push(ConfirmedParts {
                    name: name.to_owned(),
                    ..ConfirmedParts::default()
                });
            }
            flag if flag.starts_with("--confirmed-action=") => {
                let word = flag.strip_prefix("--confirmed-action=").unwrap_or_default();
                let Some(action) = Action::from_word(word) else {
                    return Err(format!("Error: invalid carried end action '{word}'."));
                };
                last_confirmed(&mut confirmed, flag)?.action = Some(action);
            }
            flag if flag.starts_with("--confirmed-detail=") => {
                let detail = flag.strip_prefix("--confirmed-detail=").unwrap_or_default();
                last_confirmed(&mut confirmed, flag)?.detail = Some(detail.to_owned());
            }
            flag if flag.starts_with("--confirmed-purge=") => {
                let word = flag.strip_prefix("--confirmed-purge=").unwrap_or_default();
                let purge = match word {
                    "on" => true,
                    "off" => false,
                    _ => return Err(format!("Error: invalid carried purge decision '{word}'.")),
                };
                last_confirmed(&mut confirmed, flag)?.purge = Some(purge);
            }
            flag if flag.starts_with("--confirmed-source=") => {
                let word = flag.strip_prefix("--confirmed-source=").unwrap_or_default();
                let from_default = match word {
                    "default" => true,
                    "explicit" => false,
                    _ => return Err(format!("Error: invalid carried end source '{word}'.")),
                };
                last_confirmed(&mut confirmed, flag)?.from_default = Some(from_default);
            }
            flag if flag.starts_with("--confirmed-tmux=") => {
                let word = flag.strip_prefix("--confirmed-tmux=").unwrap_or_default();
                let Some(tmux) = FrozenTmux::from_word(word) else {
                    return Err(format!("Error: invalid carried tmux identity '{word}'."));
                };
                last_confirmed(&mut confirmed, flag)?.tmux = Some(tmux);
            }
            flag if flag.starts_with("--expect-") => expect.take(flag)?,
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
    args.confirmed = confirmed
        .into_iter()
        .map(|parts| {
            let (Some(action), Some(detail), Some(purge), Some(from_default), Some(tmux)) = (
                parts.action,
                parts.detail,
                parts.purge,
                parts.from_default,
                parts.tmux,
            ) else {
                return Err(format!(
                    "Error: carried end plan for '{}' is incomplete.",
                    parts.name
                ));
            };
            Ok(ConfirmedPlan {
                name: parts.name,
                plan: Plan {
                    action,
                    detail,
                    purge,
                },
                from_default,
                tmux,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if expect.any() {
        args.expect = Some(expect.build(&args)?);
    }
    Ok(args)
}

fn last_confirmed<'a>(
    confirmed: &'a mut [ConfirmedParts],
    flag: &str,
) -> Result<&'a mut ConfirmedParts, String> {
    confirmed
        .last_mut()
        .ok_or_else(|| format!("Error: '{flag}' has no carried end target."))
}

fn confirmed_for<'a>(args: &'a Args, name: &str) -> Option<&'a ConfirmedPlan> {
    args.confirmed
        .iter()
        .find(|confirmed| confirmed.name == name)
}

fn confirmed_plans(args: &Args, frozen: &[FrozenTarget]) -> Vec<ConfirmedPlan> {
    if !args.confirmed.is_empty() {
        return args.confirmed.clone();
    }
    frozen
        .iter()
        .map(|target| ConfirmedPlan {
            name: target.name.clone(),
            plan: target.plan.clone(),
            from_default: args.purge_cli.is_none(),
            tmux: target.tmux.clone(),
        })
        .collect()
}

fn end_prompt(name: &str, frozen: &[FrozenTarget]) -> String {
    let has_purge = frozen.iter().any(|target| target.plan.purge);
    let has_keep = frozen.iter().any(|target| !target.plan.purge);
    let consequence = match (has_keep, has_purge) {
        (true, false) => "Archives, then deletes its state.",
        (false, true) => "Deletes its state and purges the agent history.",
        (true, true) => "Archives kept histories, purges configured histories, then deletes state.",
        (false, false) => "Deletes its state.",
    };
    format!("End '{name}'? {consequence} (y/n)")
}

fn push_confirmed(command: &mut Vec<String>, confirmed: &ConfirmedPlan) {
    command.push(format!("--confirmed-target={}", confirmed.name));
    command.push(format!(
        "--confirmed-action={}",
        confirmed.plan.action.word()
    ));
    command.push(format!("--confirmed-detail={}", confirmed.plan.detail));
    command.push(format!(
        "--confirmed-purge={}",
        if confirmed.plan.purge { "on" } else { "off" }
    ));
    command.push(format!(
        "--confirmed-source={}",
        if confirmed.from_default {
            "default"
        } else {
            "explicit"
        }
    ));
    command.push(format!("--confirmed-tmux={}", confirmed.tmux.word()));
    command.push(
        if confirmed.plan.purge {
            "--purge-history"
        } else {
            "--keep-history"
        }
        .to_owned(),
    );
}

/// The detached worker's exact argv.
fn supervisor_argv(args: &Args, confirmed: &ConfirmedPlan) -> Option<DetachedArgv> {
    let own = crate::shape::resolved_exe()?;
    Some(supervisor_args(
        own.to_string_lossy().into_owned(),
        args,
        confirmed,
    ))
}

fn supervisor_args(core: String, args: &Args, confirmed: &ConfirmedPlan) -> DetachedArgv {
    let mut command = vec![
        core,
        crate::cli::END.to_owned(),
        "--supervise".to_owned(),
        confirmed.name.clone(),
    ];
    push_confirmed(&mut command, confirmed);
    if args.assume_stopped {
        command.push("--assume-stopped".to_owned());
    }
    if let Some(pane) = args.pane.as_deref().filter(|pane| !pane.is_empty()) {
        command.push(format!("--pane={pane}"));
    }
    DetachedArgv(command)
}

/// The short-lived tmux job's exact argv. It records intent, starts the
/// detached worker and exits before the target session is killed.
fn handoff_argv(args: &Args, frozen: &[FrozenTarget]) -> Option<DetachedArgv> {
    let own = crate::shape::resolved_exe()?;
    let mut command = vec![
        own.to_string_lossy().into_owned(),
        crate::cli::END.to_owned(),
        "--handoff".to_owned(),
        args.target.clone(),
    ];
    for confirmed in confirmed_plans(args, frozen) {
        push_confirmed(&mut command, &confirmed);
    }
    if args.assume_stopped {
        command.push("--assume-stopped".to_owned());
    }
    if let Some(pane) = args.pane.as_deref().filter(|pane| !pane.is_empty()) {
        command.push(format!("--pane={pane}"));
    }
    Some(DetachedArgv(command))
}

/// Facts which survive the successful removal of a session's live state.
struct Notice {
    name: String,
    dir: PathBuf,
    server: Option<ServerId>,
    archive_id: String,
    purge: bool,
}

fn notices(root: &Path, args: &Args) -> Vec<Notice> {
    args.confirmed
        .iter()
        .map(|confirmed| {
            let name = confirmed.name.clone();
            let dir = sessions_dir(root).join(&name);
            let bytes = meta::read_bytes(&dir).unwrap_or_default();
            Notice {
                server: recorded_server(root, &name),
                archive_id: archive::canonical_uuid(&meta_value(&bytes, "session_id")),
                purge: confirmed.plan.purge,
                name,
                dir,
            }
        })
        .collect()
}

fn request_summary(pane: Option<&str>) -> String {
    let Some(pane) = pane.filter(|pane| !pane.is_empty()) else {
        return "end requested by supervisor".to_owned();
    };
    let agent = crate::doors::caller_server()
        .as_ref()
        .and_then(|server| transport::observe_viewer(server, pane))
        .and_then(|viewer| viewer.agent)
        .unwrap_or_else(|| "unknown".to_owned());
    format!("end requested from inside by {agent}/{pane}")
}

/// `_end --supervise`: run the whole end where killing a target pane cannot
/// kill this process. The handoff's request is captured by the archive; only
/// failures get a result event because only failures retain a live directory.
fn run_supervised(
    root: &Path,
    args: &Args,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    if args.confirmed.len() != 1
        || args.confirmed.first().map(|plan| &plan.name) != Some(&args.target)
    {
        writeln!(
            err,
            "Error: end supervisor received no matching confirmed plan."
        )?;
        return Ok(EXIT_USAGE);
    }
    if let Some(expect) = &args.expect {
        return run_expected(root, args, expect, out, err);
    }
    let notices = notices(root, args);
    let mut tail = vec!["-f".to_owned(), args.target.clone()];
    for confirmed in &args.confirmed {
        push_confirmed(&mut tail, confirmed);
    }
    if args.assume_stopped {
        tail.push("--assume-stopped".to_owned());
    }
    let mut captured_out = Vec::new();
    let mut captured_err = Vec::new();
    let code = match run(root, &tail, None, &mut captured_out, &mut captured_err) {
        Ok(code) => code,
        Err(why) => return supervisor_io_failed(root, &notices, &why, err),
    };
    settle(
        root,
        &notices,
        &Supervised {
            code,
            out: captured_out,
            err: captured_err,
            stopped: false,
        },
        out,
        err,
    )
}

/// What a supervised end left behind, to report.
struct Supervised {
    code: u8,
    out: Vec<u8>,
    err: Vec<u8>,
    /// The session is stopped and was not ended: the failure line says so
    /// first, whatever the owner's own first error line was.
    stopped: bool,
}

/// The owner itself failed on I/O: every target still there records it.
fn supervisor_io_failed(
    root: &Path,
    notices: &[Notice],
    why: &io::Error,
    err: &mut impl Write,
) -> io::Result<u8> {
    record_failure(root, notices, &format!("FAILED: end supervisor I/O: {why}"));
    writeln!(err, "Error: end supervisor I/O failed: {why}")?;
    Ok(EXIT_FAILED)
}

/// Record `summary` in every target still on disk, and tell its clients.
fn record_failure(root: &Path, notices: &[Notice], summary: &str) {
    for notice in notices {
        if dir_exists(&notice.dir) {
            transaction::emit(
                &notice.dir,
                &notice.name,
                if notice.purge { "" } else { &notice.archive_id },
                END_RESULT_ACTION,
                summary,
            );
            transaction::notice(root, notice, summary);
        }
    }
}

/// Report a finished supervised end: announced when it worked, recorded when
/// it failed, its captured streams passed on either way.
fn settle(
    root: &Path,
    notices: &[Notice],
    ran: &Supervised,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    if ran.code == 0 {
        for notice in notices {
            let line = if !notice.purge && !notice.archive_id.is_empty() {
                format!("Ended {} — archived {}", notice.name, notice.archive_id)
            } else {
                format!("Ended {}", notice.name)
            };
            if let Some(server) = &notice.server {
                announce_to_clients(server, &line);
            }
        }
    } else {
        let reason = String::from_utf8_lossy(&ran.err);
        let reason = reason
            .lines()
            .find(|line| line.starts_with("Error:"))
            .or_else(|| reason.lines().find(|line| !line.is_empty()))
            .unwrap_or("end failed");
        let summary = if ran.stopped {
            format!("FAILED: session STOPPED, not ended: {reason}")
        } else {
            format!("FAILED: {reason}")
        };
        record_failure(root, notices, &summary);
    }
    let output = crate::error::with_status(ran.code, out.write_all(&ran.out));
    err.write_all(&ran.err)?;
    output?;
    Ok(ran.code)
}

/// `_end --supervise` carrying an [`EndExpectation`]: the whole end of ONE
/// literal session. `run` is not re-entered, because it reads the operand
/// `all` as the fleet and a session may be named that. Under the lock the
/// expectation is proven first; until it holds nothing is written to the
/// target, not even the record of this refusal.
fn run_expected(
    root: &Path,
    args: &Args,
    expect: &EndExpectation,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let name = args.target.as_str();
    if !name_is_usable(root, name) {
        writeln!(err, "Error: '{name}' is not a usable session name.")?;
        return Ok(EXIT_FAILED);
    }
    if let Some(blocked) = crate::rename::intent_blocks(root, name) {
        writeln!(err, "Error: {blocked}. Nothing was ended.")?;
        return Ok(EXIT_FAILED);
    }
    let Ok(_guard) = lock(root, name) else {
        writeln!(err, "{}", locked_out(name))?;
        return Ok(EXIT_FAILED);
    };
    let dir = sessions_dir(root).join(name);
    if let Err(why) = expect.check(root, &dir, name, Timestamp::now().epoch()) {
        writeln!(
            err,
            "Error: {why} — nothing was stopped and nothing was deleted."
        )?;
        return Ok(EXIT_FAILED);
    }
    let notices = notices(root, args);
    let plan = &expect.confirmed.plan;
    let id = if plan.action == Action::Keep {
        expect.uuid.as_str()
    } else {
        ""
    };
    transaction::emit(
        &dir,
        name,
        id,
        END_REQUEST_ACTION,
        "end requested from ae app",
    );
    let contract = (plan, &expect.confirmed.tmux);
    let (mut captured_out, mut captured_err) = (Vec::new(), Vec::new());
    let ended = end_locked(
        root,
        name,
        args,
        Some(contract),
        &mut captured_out,
        &mut captured_err,
    );
    let code = match ended {
        Ok(true) => 0,
        Ok(false) => EXIT_FAILED,
        Err(why) => return supervisor_io_failed(root, &notices, &why, err),
    };
    let stopped = code != 0
        && (expect.confirmed.tmux == FrozenTmux::Unknown
            || sight(&expect.server, name) == TmuxSight::Absent);
    settle(
        root,
        &notices,
        &Supervised {
            code,
            out: captured_out,
            err: captured_err,
            stopped,
        },
        out,
        err,
    )
}

/// Hand a confirmed End to its detached owner. This writes NOTHING to the
/// target: it does not hold the target's lock, so it cannot yet know the
/// directory is the one that was confirmed. The owner records the human's
/// answer once its proof has passed.
pub(crate) fn end_confirmed_via(
    root: &Path,
    expect: &EndExpectation,
    name: &str,
    detach: impl FnOnce(&DetachedArgv) -> bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    if !name_is_usable(root, name) {
        writeln!(err, "'{name}' is not a usable session name")?;
        return Ok(EXIT_FAILED);
    }
    let Some(own) = crate::shape::resolved_exe() else {
        writeln!(err, "ae cannot name its own executable")?;
        return Ok(EXIT_FAILED);
    };
    if !detach(&expect.argv(own.to_string_lossy().into_owned())) {
        writeln!(err, "ae could not start the End job")?;
        return Ok(EXIT_FAILED);
    }
    writeln!(out, "Ending '{name}'.")?;
    Ok(0)
}

/// Hand an already-authorized end to a detached process.
fn handoff(
    root: &Path,
    args: &Args,
    frozen: &[FrozenTarget],
    caller_session: Option<&str>,
    report_start: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let carried = confirmed_plans(args, frozen);
    let mut ordered: Vec<&String> = frozen.iter().map(|target| &target.name).collect();
    ordered.sort_by_key(|name| u8::from(caller_session == Some(name.as_str())));
    let Some(supervisors) = ordered
        .iter()
        .map(|name| {
            carried
                .iter()
                .find(|confirmed| confirmed.name == name.as_str())
                .and_then(|confirmed| supervisor_argv(args, confirmed))
                .map(|argv| (*name, argv))
        })
        .collect::<Option<Vec<_>>>()
    else {
        writeln!(
            err,
            "Error: ae cannot name its own executable, so it cannot hand '{}' to a supervisor — nothing was ended.",
            args.target
        )?;
        return Ok(EXIT_FAILED);
    };
    let requested = request_summary(args.pane.as_deref());
    let captured: Vec<_> = frozen
        .iter()
        .map(|target| {
            let dir = sessions_dir(root).join(&target.name);
            let id = if target.plan.action == Action::Keep {
                archive::canonical_uuid(&meta::session_id_in(&dir))
            } else {
                String::new()
            };
            (target.name.as_str(), dir, id)
        })
        .collect();
    for (name, dir, id) in &captured {
        transaction::emit(dir, name, id, END_REQUEST_ACTION, &requested);
    }
    let mut failures = 0_u32;
    for (name, argv) in supervisors {
        if !transport::run_detached(&argv) {
            failures += 1;
            let dir = sessions_dir(root).join(name);
            if let Some((_, _, id)) = captured.iter().find(|(n, _, _)| *n == name.as_str()) {
                transaction::emit(
                    &dir,
                    name,
                    id,
                    END_RESULT_ACTION,
                    "FAILED: supervisor did not start",
                );
                let notice = Notice {
                    name: name.clone(),
                    dir,
                    server: recorded_server(root, name),
                    archive_id: id.clone(),
                    purge: false,
                };
                transaction::notice(root, &notice, "supervisor did not start; retry ae end");
            }
        }
    }
    if failures > 0 {
        writeln!(
            err,
            "Error: could not start {failures} end supervisor(s) for '{}' — affected state was preserved.",
            args.target,
        )?;
        return Ok(EXIT_FAILED);
    }
    if !report_start {
        return Ok(0);
    }
    if args.target == "all" {
        writeln!(out, "Ending all ae sessions out of pane.")?;
    } else {
        writeln!(
            out,
            "Ending '{}' out of pane; this pane will close.",
            args.target
        )?;
    }
    Ok(0)
}

/// Read the confirmation.
fn read_reply() -> Option<String> {
    let mut buffer = String::new();
    // A LINE, not to EOF: a human at a terminal answers and presses Enter, and
    // reading to EOF there would block forever waiting for a ^D they were never
    // asked for.
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
    frozen: &[FrozenTarget],
    err: &mut impl Write,
) -> io::Result<()> {
    if args.target == "all" {
        writeln!(err, "This will END every ae session:")?;
    } else {
        writeln!(err, "This will END the session:")?;
    }
    for target in frozen {
        writeln!(err, "{}", target.plan.line(&target.name))?;
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
/// so the confirmation can name the exact path.
fn resolve_plan(root: &Path, name: &str, purge_cli: Option<bool>) -> Plan {
    let dir = sessions_dir(root).join(name);
    let bytes = meta::read_bytes(&dir).unwrap_or_default();
    let purge = effective_purge(&bytes, purge_cli);
    let archive_root = root.join("archive");

    if meta::read_bytes(&dir).is_err() {
        // A missing meta is not the same as nothing to lose.
        let store = crate::store::open(&dir);
        let has_memory = has_nonempty(&store.memo_path())
            || has_nonempty(&store.events_path())
            || has_message_payload(&dir.join("messages"))
            || archive::retained::has_evidence(&dir);
        return if has_memory {
            Plan {
                action: Action::Unavailable,
                detail: format!(
                    "its meta is missing or unreadable, so ae cannot identify {} — session memory or retained evidence is still there",
                    dir.display()
                ),
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
    // ABSENT vs CORRUPT, decided BEFORE purge gets a say.
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
fn effective_purge(bytes: &[u8], purge_cli: Option<bool>) -> bool {
    if let Some(explicit) = purge_cli {
        return explicit;
    }
    workspace_of(bytes).is_some_and(|w| w.purge_agent_history)
}

/// The `[workspace]` block a session's own config resolves to — its recorded
/// global config layered under its origin's local `.ae/config`.
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

/// The boot-time proof, composed once for the two verbs that cross it: `end`
/// (only behind `--assume-stopped`) and `rename` (which deletes nothing and so
/// needs no acknowledgement).
///
/// `dir` is the session's own state directory and `server` the POSITIVE
/// recorded server that just failed the strict probe. [`crate::inventory::last_live`]
/// supplies the session's own last sign of life and
/// [`crate::tmux::classify_absence`] is the one rule that weighs it against
/// [`crate::doors::boot_time`] — the same proof a resume and the fleet listing
/// already cross.
///
/// A positive `true` means the session's own last activity predates the host
/// boot, so no process can still hold it. Every gap — no boot time, a boot in
/// the future, no recorded activity, activity at or after the boot, an
/// unreadable source, or a failure that was not a missing socket — answers
/// `false`, and the caller's refusal stands.
///
/// The caller has ALREADY asked [`transport::verify_session_absent`] and read
/// `Unknown`; this function re-probes, so a server that revives between the two
/// calls is seen as `Present` rather than assumed missing. That window is
/// fail-CLOSED: a revived or foreign session makes the verdict anything but
/// `Absent`, and the strict refusal stands.
pub(crate) fn boot_proved_stopped(dir: &Path, server: &ServerId, name: &str) -> bool {
    let probe = transport::probe_absence(server, name);
    let evidence = crate::inventory::last_live(dir);
    let boot = crate::doors::boot_time(crate::shape::current());
    let now = crate::time::Timestamp::now().epoch();
    crate::tmux::classify_absence(probe, evidence, boot, now) == StopProbe::Absent
}

/// The refusal of a target whose lifecycle lock is held elsewhere.
fn locked_out(name: &str) -> String {
    format!(
        "Error: another lifecycle operation (start/resume/end) is in progress for '{name}' — retry shortly. State preserved."
    )
}

/// End one session, under its lifecycle lock.
fn end_one(
    root: &Path,
    name: &str,
    args: &Args,
    contract: Option<(&Plan, &FrozenTmux)>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<bool> {
    let Ok(_guard) = lock(root, name) else {
        writeln!(err, "{}", locked_out(name))?;
        return Ok(false);
    };
    end_locked(root, name, args, contract, out, err)
}

/// End one session whose lifecycle lock the caller holds.
#[allow(
    clippy::too_many_lines,
    reason = "the frozen order IS the contract: every step is named here, in order, so the sequence stays in one place"
)]
fn end_locked(
    root: &Path,
    name: &str,
    args: &Args,
    contract: Option<(&Plan, &FrozenTmux)>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<bool> {
    let dir = sessions_dir(root).join(name);
    let current_purge = end_purge(confirmed_for(args, name), args.purge_cli);
    // Refuse unsupported/unreadable snapshot nodes before migration writes
    // meta. This is advisory; the publisher repeats capture under source holds.
    let preflight_plan = resolve_plan(root, name, current_purge);
    if preflight_refused(root, &dir, &preflight_plan, err)? {
        return Ok(false);
    }
    // The chain, under the lifecycle lock this end already holds. A refusal is
    // REPORTED, never fatal — the whole point of an end is that a session can
    // always be torn down, whatever shape its meta is in.
    if let Some(note) = crate::migrate::session_noted(&dir, name) {
        writeln!(err, "{note}")?;
    }
    let bytes = meta::read_bytes(&dir).unwrap_or_default();

    // ══ THE INVARIANT: ae NEVER deletes session state unless
    //    (a) the target was positively identified on ITS OWN recorded server
    //        and its kill was verified, or
    //    (b) the human passed --assume-stopped for THIS single target, and
    //        EITHER the full enumerable sweep found nothing AND left no
    //        unverifiable socket standing, OR the target's POSITIVE record
    //        names an unreachable server AND the boot-time proof positively
    //        places the session's own last sign of life before the host boot
    //        (no process survives a reboot) — the flag AND the independent
    //        proof, both or refuse, because a missing socket alone proves
    //        nothing, or
    //    (c) the target's POSITIVE record names a server that verifiably lacks
    //        the session.
    let proof = match server_of(&bytes) {
        ServerSelector::Positive(sel) => {
            positive_sight(&dir, name, args, ServerId::Selected(sel), out, err)?
        }
        ServerSelector::Missing | ServerSelector::Ambiguous => {
            let frozen_tmux = contract.map(|(_, tmux)| tmux);
            unrecorded(root, name, args, frozen_tmux, out, err)?.then(Proof::default)
        }
    };
    let Some(proof) = proof else {
        return Ok(false);
    };

    let mode = meta_value(&bytes, "mode");
    let work_dir = resolve_workdir(root, &bytes, name);

    let plan = resolve_plan(root, name, current_purge);
    if let Some(lines) = refusal_before_stop(name, &plan, contract, proof.seen.as_ref()) {
        for line in lines {
            writeln!(err, "{line}")?;
        }
        return Ok(false);
    }

    // A session already gone may still have a pane view running its agents;
    // ending it would archive and delete the state they run under.
    if proof.session_id.is_none()
        && matches!(proof.seen, Some(TmuxSight::Absent))
        && let Some(server) = proof.server.as_ref()
        && !views_gone(server, &dir, name, "end", err)?
    {
        writeln!(err, "  Nothing was stopped and nothing was deleted.")?;
        return Ok(false);
    }

    let retry = if plan.action == Action::Keep {
        match transaction::inspect(&dir, &bytes) {
            Ok(retry) => retry,
            Err(why) => {
                writeln!(err, "Error: {why}")?;
                return Ok(false);
            }
        }
    } else {
        None
    };

    // Local mode, or a retry of an end whose git step already ran: stop, then
    // capture, with no git of its own.
    if let Some((git, ended)) = early_finish(&mode, retry) {
        if !stop_target(&dir, name, &bytes, &plan, &proof, err)? {
            return Ok(false);
        }
        // CAPTURE BEFORE DELETE.
        if !transaction::finish(root, name, &bytes, &plan, &git, out, err)? {
            return Ok(false);
        }
        writeln!(out, "{ended} {name}")?;
        return Ok(true);
    }

    // The git path: git and full mode, and by exclusion any mode not local.
    let wdir_bytes = work_dir.as_os_str().as_encoded_bytes().to_vec();
    if workdir_refused(&work_dir, &wdir_bytes, err)? {
        return Ok(false);
    }
    // STOP FIRST — and verify it.
    if !stop_target(&dir, name, &bytes, &plan, &proof, err)? {
        return Ok(false);
    }
    if !commit_pending(&wdir_bytes, &work_dir, name, out, err)? {
        return Ok(false);
    }
    // The push outcome is recorded by the branch that ACTUALLY runs and handed
    // to the archive — never rediscovered afterwards, when the remote may have
    // moved and the worktree may already be gone.
    let Some((mut git, mut preserve_workdir)) = push_work(&wdir_bytes, &work_dir, name, out, err)?
    else {
        return Ok(false);
    };
    if !preserve_workdir && nonlossless_workdir(&work_dir, name, out)? {
        preserve_workdir = true;
        git.preserved = work_dir.display().to_string();
    }

    // The last thing before the session stops existing: capture it, or fail and
    // keep it.
    if !transaction::finish(root, name, &bytes, &plan, &git, out, err)? {
        return Ok(false);
    }
    if preserve_workdir {
        writeln!(out, "Ended {name} — working directory preserved.")?;
        writeln!(out, "  Directory preserved: {}", work_dir.display())?;
    } else {
        writeln!(out, "Ended {name}")?;
    }
    Ok(true)
}

/// Whether the git path cannot preserve the work because the working
/// directory is missing or not a git work tree, saying so on `err` before
/// anything is stopped. A missing directory asks git nothing.
fn workdir_refused(work_dir: &Path, wdir: &[u8], err: &mut impl Write) -> io::Result<bool> {
    if dir_exists(work_dir) && crate::git::is_work_tree(wdir) {
        return Ok(false);
    }
    writeln!(
        err,
        "Error: working directory is not a git repo — cannot preserve work."
    )?;
    writeln!(
        err,
        "  Nothing was stopped and nothing was deleted; the session is still running."
    )?;
    writeln!(err, "  Working directory: {}", work_dir.display())?;
    Ok(true)
}

/// Commit whatever the stopped session left uncommitted; a clean tree commits
/// nothing and stamps nothing. Whether the end may go on: a failed commit is
/// reported on `err` with the way back.
fn commit_pending(
    wdir: &[u8],
    work_dir: &Path,
    name: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<bool> {
    if !crate::git::has_pending_work(wdir) {
        return Ok(true);
    }
    writeln!(out, "Committing changes in {name}...")?;
    let stamp = Timestamp::now().to_string();
    let human = stamp.replace('T', " ").replace('Z', " UTC");
    if crate::git::commit_all(
        wdir,
        &format!("ae: end session {name}"),
        &format!("Ended: {human}"),
    ) {
        return Ok(true);
    }
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
    Ok(false)
}

/// Where the committed work went.
enum Pushed {
    /// Pushed to this `ae/…` branch on origin.
    Branch(String),
    /// Already on a remote branch; nothing to push.
    Reachable,
    /// No origin remote: the work exists only in the directory.
    NoOrigin,
}

/// Push the committed work if origin lacks it, saying what happened on `out`:
/// the archive's git facts and whether the directory must be preserved, or
/// `None` for a failed push reported on `err` with the way back. The remote is
/// fetched, and HEAD placed against it, only when there is an origin.
fn push_work(
    wdir: &[u8],
    work_dir: &Path,
    name: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<Option<(Git, bool)>> {
    let has_origin = crate::git::has_origin(wdir);
    if has_origin {
        crate::git::fetch_origin(wdir);
    }
    let pushed = if has_origin && !crate::git::head_is_on_a_remote(wdir) {
        let branch = sanitize_branch_name(name);
        writeln!(out, "Pushing to origin/{branch}...")?;
        writeln!(
            out,
            "  {} file(s) changed",
            crate::git::pushed_file_count(wdir)
        )?;
        if !crate::git::push_head(wdir, &branch) {
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
            return Ok(None);
        }
        writeln!(out, "Pushed to origin/{branch}")?;
        Pushed::Branch(branch)
    } else if has_origin {
        writeln!(out, "No new commits to push.")?;
        Pushed::Reachable
    } else {
        // B3 durability: a no-remote git target just COMMITTED work that exists
        // ONLY in this directory.
        writeln!(
            out,
            "Working directory preserved: {} (no origin remote to push to).",
            work_dir.display()
        )?;
        Pushed::NoOrigin
    };
    Ok(Some(push_facts(pushed, work_dir)))
}

/// The archive's git facts for where the work went, and whether the directory
/// is preserved — PURE. Every field a kind does not name keeps the
/// [`Git::none`] value; the `"-"` of `preserved` is load-bearing, because the
/// finish preserves the directory whenever it is anything else.
fn push_facts(pushed: Pushed, work_dir: &Path) -> (Git, bool) {
    let mut git = Git::none();
    git.workdir = work_dir.display().to_string();
    match pushed {
        Pushed::Branch(branch) => {
            "pushed".clone_into(&mut git.outcome);
            git.push_ref = format!("origin/{branch}");
            (git, false)
        }
        Pushed::Reachable => {
            "already-reachable".clone_into(&mut git.outcome);
            (git, false)
        }
        Pushed::NoOrigin => {
            "no-origin".clone_into(&mut git.outcome);
            git.preserved.clone_from(&git.workdir);
            (git, true)
        }
    }
}

/// Which end finishes without a git step of its own, and its final word —
/// PURE. Local mode (or no mode) manages no branch, so its archive records the
/// git outcome as not-managed, or a retry's recorded facts; any other mode
/// skips git only on a retry, whose git step already ran. `None` takes the git
/// path, whatever the mode is called.
fn early_finish(mode: &str, retry: Option<Git>) -> Option<(Git, &'static str)> {
    if mode == "local" || mode.is_empty() {
        return Some((retry.unwrap_or_else(Git::none), "Ended local session"));
    }
    retry.map(|git| (git, "Ended"))
}

/// Stop the proven target: hand its clients off, then kill it and verify the
/// kill. A failed handoff is journaled and warned about only once the kill
/// held; a refused kill (reported on `err`) discards it. A target with no live
/// tmux session to kill is already stopped.
fn stop_target(
    dir: &Path,
    name: &str,
    bytes: &[u8],
    plan: &Plan,
    proof: &Proof,
    err: &mut impl Write,
) -> io::Result<bool> {
    let (Some(server), Some(id)) = (proof.server.as_ref(), proof.session_id.as_ref()) else {
        return Ok(true);
    };
    let handoff_note = handoff_clients_before_kill(server, name);
    if !kill_verified(server, name, "end", id, err)? {
        return Ok(false);
    }
    if let Some(note) = handoff_note {
        // A retry (an end whose archive may already be complete) is always
        // Keep, so it names the same id the git and local paths name; emit
        // itself decides whether that id still takes the row.
        let archive_id = if plan.action == Action::Keep {
            archive::canonical_uuid(&meta_value(bytes, "session_id"))
        } else {
            String::new()
        };
        transaction::emit(dir, name, &archive_id, END_RESULT_ACTION, &note);
        writeln!(err, "Warning: {note}")?;
    }
    Ok(true)
}

/// The history choice an end resolves its plan with — PURE. A target the
/// prompt confirmed carries its plan's choice, unless that choice came from
/// config, which then decides again under the lock (`None`); a target nothing
/// was confirmed for takes the command line's.
fn end_purge(confirmed: Option<&ConfirmedPlan>, purge_cli: Option<bool>) -> Option<bool> {
    confirmed.map_or(purge_cli, |confirmed| {
        if confirmed.from_default {
            None
        } else {
            Some(confirmed.plan.purge)
        }
    })
}

/// Whether a kept history's retained snapshot nodes are unsupported or
/// unreadable, saying why on `err`. Only a [`Action::Keep`] plan asks; the
/// archive target is named from a FRESH read of the recorded id.
fn preflight_refused(
    root: &Path,
    dir: &Path,
    plan: &Plan,
    err: &mut impl Write,
) -> io::Result<bool> {
    if plan.action != Action::Keep {
        return Ok(false);
    }
    let Err(why) = crate::archive::retained::preflight(dir) else {
        return Ok(false);
    };
    let target = root
        .join("archive")
        .join(archive::canonical_uuid(&meta::session_id_in(dir)));
    if archive::retained::node_exists(&target) {
        writeln!(
            err,
            "Error: {}",
            archive::publish::mismatch(dir, &target, &why)
        )?;
    } else {
        writeln!(err, "Error: {why}")?;
    }
    Ok(true)
}

/// What the server proof established (the invariant in [`end_locked`]): the
/// recorded server, the exact tmux session to kill on it, and what the lock
/// saw there. All three are `None` for a target with no positive record that
/// the acknowledgement let through.
#[derive(Default)]
struct Proof {
    server: Option<ServerId>,
    session_id: Option<String>,
    seen: Option<TmuxSight>,
}

/// The target's own recorded server: what it shows now. An unreachable server
/// is refused unless the human acknowledged the stop AND the boot-time proof
/// holds.
fn positive_sight(
    dir: &Path,
    name: &str,
    args: &Args,
    id: ServerId,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<Option<Proof>> {
    let reading = sight(&id, name);
    if reading == TmuxSight::Unknown {
        if args.assume_stopped && boot_proved_stopped(dir, &id, name) {
            writeln!(
                out,
                "Proceeding on explicit --assume-stopped acknowledgement and the boot-time proof (this session's own last sign of life predates the host boot, so nothing can still hold it)."
            )?;
        } else {
            writeln!(
                err,
                "Error: cannot verify session '{name}' state (its recorded tmux server is unreachable) — state preserved."
            )?;
            return Ok(None);
        }
    }
    let session_id = match &reading {
        TmuxSight::Live { id: live, .. } => Some(live.clone()),
        TmuxSight::Absent | TmuxSight::Unknown => None,
    };
    Ok(Some(Proof {
        server: Some(id),
        session_id,
        seen: Some(reading),
    }))
}

/// A target with no positive server record: refused when the prompt saw one
/// (a lost record is a refusal, never a sweep), else swept and judged by
/// [`unrecorded_verdict`]. Whether it may proceed.
fn unrecorded(
    root: &Path,
    name: &str,
    args: &Args,
    frozen_tmux: Option<&FrozenTmux>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<bool> {
    if let Some(tmux) = frozen_tmux
        && !matches!(tmux, FrozenTmux::Unobserved)
    {
        writeln!(
            err,
            "Error: '{name}' has no positive server record, so ae cannot prove what was confirmed."
        )?;
        writeln!(err, "  Confirmed: {}", tmux.describe())?;
        writeln!(err, "  Now:       no positive server record")?;
        writeln!(
            err,
            "  Nothing was stopped and nothing was deleted. Re-run 'ae end {name}' to see the current plan."
        )?;
        return Ok(false);
    }
    match unrecorded_verdict(name, &sweep(root, name), args.assume_stopped) {
        Verdict::Refuse(lines) => {
            for line in lines {
                writeln!(err, "{line}")?;
            }
            Ok(false)
        }
        Verdict::Proceed(line) => {
            writeln!(out, "{line}")?;
            Ok(true)
        }
    }
}

/// What a recordless target's sweep allows.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Refuse, with these `err` lines.
    Refuse(Vec<String>),
    /// Proceed, announcing it with this `out` line.
    Proceed(&'static str),
}

/// Judge the enumerable sweep of a recordless target — PURE. Only every
/// enumerable server answering without it AND the human's acknowledgement
/// let it through.
fn unrecorded_verdict(name: &str, sweep: &Sweep, assume_stopped: bool) -> Verdict {
    match sweep {
        Sweep::Found(where_) => Verdict::Refuse(vec![
            format!(
                "Error: session '{name}' has no positive server record AND is LIVE on enumerable server '{where_}'."
            ),
            format!("  Establish ownership first: 'ae doctor --refresh {name}'."),
            "  State preserved.".to_owned(),
        ]),
        Sweep::Unsure(where_) => Verdict::Refuse(vec![
            format!(
                "Error: cannot verify enumerable tmux server '{where_}' (socket exists but did not answer) — session '{name}' could be hiding behind it."
            ),
            "  Fix that server (permissions/staleness), then retry. State preserved.".to_owned(),
        ]),
        Sweep::Clear if !assume_stopped => Verdict::Refuse(vec![
            format!(
                "Error: session '{name}' has no positive server record (pre-fix or ambiguous meta) — it may be live somewhere ae cannot enumerate."
            ),
            format!(
                "  Resolve: 'ae doctor --refresh {name}', or acknowledge it is stopped: ae end --assume-stopped {name}"
            ),
            "  State preserved.".to_owned(),
        ]),
        Sweep::Clear => Verdict::Proceed(
            "Proceeding on explicit --assume-stopped acknowledgement (not live on any enumerable server, default included).",
        ),
    }
}

/// The refusals due before anything is stopped, in order — PURE: a session ae
/// cannot identify (the archive step would refuse it anyway), a confirmed
/// tmux incarnation the lock no longer sees, and a confirmed plan that is no
/// longer the plan. The `err` lines of the first that applies.
fn refusal_before_stop(
    name: &str,
    plan: &Plan,
    contract: Option<(&Plan, &FrozenTmux)>,
    seen: Option<&TmuxSight>,
) -> Option<Vec<String>> {
    if plan.action == Action::Unavailable {
        return Some(vec![
            format!(
                "Error: session '{name}' cannot be archived — {}.",
                plan.detail
            ),
            "  Nothing was stopped and NOTHING was deleted; ae does not delete a session it could not capture."
                .to_owned(),
        ]);
    }
    let rerun = || {
        format!(
            "  Nothing was stopped and nothing was deleted. Re-run 'ae end {name}' to see the current plan."
        )
    };
    let (frozen_plan, tmux) = contract?;
    if let Some(reading) = seen
        && let Reproof::Refuse(first) = reprove(name, tmux, reading)
    {
        return Some(vec![
            format!("Error: {first}"),
            format!("  Confirmed: {}", tmux.describe()),
            format!("  Now:       {}", reading.describe(name)),
            rerun(),
        ]);
    }
    (*frozen_plan != *plan).then(|| {
        vec![
            format!(
                "Error: what '{name}' would do changed between the confirmation and now — refusing to act on an answer given to a different question."
            ),
            format!("  Confirmed: {}", frozen_plan.line(name)),
            format!("  Now:       {}", plan.line(name)),
            rerun(),
        ]
    })
}

/// The operation facts the archive publisher needs and the core does not derive
/// itself.
struct Git {
    outcome: String,
    push_ref: String,
    preserved: String,
    workdir: String,
    retry_id: Option<String>,
}

impl Git {
    /// Local mode manages no branch of its own.
    fn none() -> Self {
        Self {
            outcome: "not-managed".to_owned(),
            push_ref: "-".to_owned(),
            preserved: "-".to_owned(),
            workdir: String::new(),
            retry_id: None,
        }
    }
}

/// Publish the archive (or perform the purge) for a session that is verifiably
/// stopped, BEFORE any live state is removed.
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
                // nothing deleted; post-commit: PURGE INCOMPLETE.
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
            let held = match archive::publish::hold_sources(&dir) {
                Ok(held) => held,
                Err(why) => {
                    writeln!(err, "{why}")?;
                    return Ok(false);
                }
            };
            archive_keep(&dir, name, git, &held, out, err)
        }
    }
}

fn archive_keep(
    dir: &Path,
    name: &str,
    git: &Git,
    held: &archive::publish::SourceGuards,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<bool> {
    let bytes = meta::read_bytes(dir).unwrap_or_default();
    let aid = archive::canonical_uuid(&meta_value(&bytes, "session_id"));
    let archived_at = Timestamp::now().to_string();
    let ops = archive::publish::Ops {
        reason: archive::publish::Reason::End,
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
    let code = archive::publish::end_snapshot(dir, &ops, held, &mut captured, err)?;
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

/// Remove the live session.
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

    keep_legacy(root, name, &origin, out)?;

    // Read meta BEFORE the removal below.
    if plan.purge {
        purge_conversation_files(root, name, bytes, out, err)?;
    } else {
        writeln!(
            out,
            "  kept agent conversation files (claude/codex/agy token history; purge: ae end --purge-history)"
        )?;
    }

    // The core owns the removal — a rename-to-tombstone commit boundary that
    // clears the canonical name atomically and reports success only after the
    // removal is durable.
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
        // Repeat the positive proof immediately before managed removal.
        let workdir = resolve_workdir(root, bytes, name);
        let preserve = preserve || nonlossless_workdir(&workdir, name, out)?;
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
        // canonical state; it owns BOTH resources.
        if mode == "git" && !origin.is_empty() {
            crate::git::worktree_prune(origin.as_bytes());
        }
        if !preserve {
            writeln!(out, "Removed worktree for {name}")?;
        }
        return Ok(true);
    }
    // A grammar-invalid name: removed here, so it is never made un-endable.
    let _ = std::fs::remove_dir_all(&dir);
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

/// A failed/unknown proof preserves the managed resource and explains why.
fn nonlossless_workdir(workdir: &Path, name: &str, out: &mut impl Write) -> io::Result<bool> {
    let proof = if archive::retained::legacy_state(workdir, name).is_some() {
        Err("legacy session state or a non-directory/unreadable .ae path remains")
    } else {
        crate::git::lossless_removal(workdir.as_os_str().as_encoded_bytes())
    };
    if let Err(why) = proof {
        writeln!(
            out,
            "  Working directory preserved: {} ({why}).",
            workdir.display()
        )?;
        return Ok(true);
    }
    Ok(false)
}

/// Legacy history stays where it is; inspect no link target and delete nothing.
fn keep_legacy(root: &Path, name: &str, origin: &str, out: &mut impl Write) -> io::Result<()> {
    let mut bases = vec![worktrees_dir(root).join(name)];
    if !origin.is_empty() {
        bases.push(PathBuf::from(origin));
    }
    for base in bases {
        if let Some(path) = archive::retained::legacy_state(&base, name) {
            writeln!(out, "  Legacy session state preserved: {}", path.display())?;
        }
    }
    Ok(())
}

/// Delete the agent CLI conversation files this session's captured harness ids
/// name, except any id another session under `root` also names or might name
/// (`claims`). `name` is the session being ended and never counts against
/// itself.
fn purge_conversation_files(
    root: &Path,
    name: &str,
    bytes: &[u8],
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<()> {
    let text = String::from_utf8_lossy(bytes);
    let parsed = meta::Meta::parse(&text);
    let home = crate::instance::user_home(root);
    // Every conversation this session can NAME: each seat's current id and the
    // predecessors it abandoned — same proof, same body, same promise. A
    // predecessor carries the TOOL that owns it, and that tool decides which
    // arm below removes it: a seat reseated from codex to claude has ids in both
    // stores, and looking either up in the other's would name a file belonging
    // to nobody. A legacy untagged id takes the slot's current binary, which is
    // the chain's own rule for one.
    let mut conversations: Vec<(&meta::RosterEntry, &str, Option<&str>)> = Vec::new();
    for entry in parsed.roster() {
        let recorded = entry.harness_session.as_deref().unwrap_or_default();
        if !recorded.is_empty() && recorded != "pending" {
            conversations.push((entry, recorded, None));
        }
        conversations.extend(
            parsed
                .harness_session_prior(&entry.slot)
                .into_iter()
                .filter(|prior| !prior.is_empty() && *prior != "pending")
                .map(|prior| match meta::prior_parts(prior) {
                    Some(parsed) => (entry, parsed.id, parsed.tool),
                    None => (entry, prior, None),
                }),
        );
    }
    if conversations.is_empty() {
        return Ok(());
    }
    // An id is a NAME this session recorded, not proof that it owns the file:
    // a foreign or hand-edited row would otherwise delete the conversation a
    // different session still resumes. One read of the other sessions' metas
    // answers every id below, for every tool.
    let others = claims::Others::scan(root, name);
    for (entry, recorded, tagged) in conversations {
        // A NAME, NEVER A PATH. `harness_session.<slot>` is metadata — a
        // hand-editable file, and `set-harness-session` screens only for control
        // bytes — and every arm below interpolates it into a filename that is
        // then joined onto a tool's store and REMOVED. `Path::join` with an
        // absolute operand DISCARDS the store entirely, so a row reading
        // `/etc/passwd` is not a conversation id, it is a deletion target; and
        // `../../x` walks out of the store just as well. So the value is proven
        // to be a UUID first, with the grammar the archive already uses, and an
        // id that is not one names NOTHING — reported as a loss, because a
        // conversation ae cannot safely name is a conversation the purge did not
        // remove, and the operator asked for it to be gone. A predecessor id is
        // the same hand-editable class and crosses the same proof.
        let uuid = crate::archive::canonical_uuid(recorded);
        if uuid.is_empty() {
            writeln!(
                err,
                "  note: conversation file for slot {} left in place (recorded id is not a UUID)",
                entry.slot
            )?;
            continue;
        }
        let uuid = uuid.as_str();
        if others.verdict(uuid).keeps(&entry.slot, err)? {
            continue;
        }
        let current = entry.binary.as_deref().unwrap_or_default();
        let tool = tagged.unwrap_or(current);
        // The CONFIG HOME below is the slot's current one, so a predecessor
        // that ran under a DIFFERENT tool also ran under a store this meta no
        // longer records. That conversation is reported as left in place rather
        // than hunted for in the wrong store: the purge names every loss and
        // guesses at none.
        if tagged.is_some_and(|tag| tag != current) {
            writeln!(
                err,
                "  note: {tool} conversation file for slot {} left in place (it predates a reseat and its config home is not recorded)",
                entry.slot
            )?;
            continue;
        }
        match tool {
            "claude" => {
                let Some(config_home) = purge_config_home(root, entry, "claude", err)? else {
                    continue;
                };
                if let Some(file) = find_claude_file(&config_home, uuid, &entry.slot, err)?
                    && remove_contained(&config_home, &file, &entry.slot, err)?
                {
                    writeln!(out, "  removed claude conversation: {}", file.display())?;
                }
            }
            "codex" => {
                let Some(config_home) = purge_config_home(root, entry, "codex", err)? else {
                    continue;
                };
                if let Some(file) = find_codex_file(&config_home, uuid, &entry.slot, err)?
                    && remove_contained(&config_home, &file, &entry.slot, err)?
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
                let Some(home) = home.as_ref() else { continue };
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

/// Resolve and validate the config-store root a destructive walk may enter.
fn purge_config_home(
    state_root: &Path,
    entry: &meta::RosterEntry,
    tool: &str,
    err: &mut impl Write,
) -> io::Result<Option<PathBuf>> {
    let (candidate, recorded) = match &entry.config_home {
        meta::RecordedConfigHome::Path(path) | meta::RecordedConfigHome::Implicit(path) => {
            (path.clone(), true)
        }
        meta::RecordedConfigHome::Missing => {
            let Some(home) = crate::instance::user_home(state_root) else {
                warn_config_home(entry, tool, "legacy HOME is unavailable", err)?;
                return Ok(None);
            };
            let Some(default) = ToolKind::from_binary_name(tool)
                .adapter()
                .config_home_default
            else {
                return Ok(None);
            };
            (home.join(default), false)
        }
        meta::RecordedConfigHome::Absent => {
            warn_config_home(entry, tool, "recorded config home is absent", err)?;
            return Ok(None);
        }
        meta::RecordedConfigHome::Unknown => {
            warn_config_home(entry, tool, "recorded config home is unknown", err)?;
            return Ok(None);
        }
        meta::RecordedConfigHome::Invalid => {
            warn_config_home(
                entry,
                tool,
                "recorded config home is malformed or duplicated",
                err,
            )?;
            return Ok(None);
        }
    };
    if !candidate.is_absolute() {
        warn_config_home(entry, tool, "config home is not absolute", err)?;
        return Ok(None);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: purge validates the recorded tool-store root before entering it"
    )]
    let metadata = std::fs::symlink_metadata(&candidate);
    let Ok(metadata) = metadata else {
        warn_config_home(entry, tool, "config home is not an existing directory", err)?;
        return Ok(None);
    };
    if recorded && metadata.file_type().is_symlink() {
        warn_config_home(entry, tool, "recorded config home is a symbolic link", err)?;
        return Ok(None);
    }
    if !metadata.is_dir() {
        warn_config_home(entry, tool, "config home is not an existing directory", err)?;
        return Ok(None);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: purge pins every destructive walk to the config store's canonical root"
    )]
    let canonical = std::fs::canonicalize(&candidate);
    let Ok(canonical) = canonical else {
        warn_config_home(entry, tool, "config home cannot be canonicalized", err)?;
        return Ok(None);
    };
    if recorded && canonical != candidate {
        warn_config_home(entry, tool, "recorded config home is not canonical", err)?;
        return Ok(None);
    }
    Ok(Some(canonical))
}

fn warn_config_home(
    entry: &meta::RosterEntry,
    tool: &str,
    reason: &str,
    err: &mut impl Write,
) -> io::Result<()> {
    writeln!(
        err,
        "  note: {tool} conversation file for slot {} left in place ({reason})",
        entry.slot
    )
}

/// agy's conversation database for `uuid`, and the `SQLite` sidecars beside it.
///
/// No search and no ambiguity check, unlike the claude and codex finders: agy
/// keeps one file per conversation NAMED for the id in one flat directory, so
/// the path is computed rather than looked for, and only paths that exist are
/// returned. `-wal` and `-shm` are appended to the FULL file name (`<id>.db-wal`
/// is what `SQLite` writes), which is why they are built from the database's own
/// name rather than from a second extension.
///
/// `uuid` is the caller's proven one. Computing a path from an unvalidated id is
/// how a metadata row becomes a deletion target, so the check lives at the
/// boundary where the row is READ rather than here — one guard covering every
/// arm, instead of one per tool that a fourth tool would be added without.
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

/// `<config-home>/projects/*/<uuid>.jsonl`, and only when exactly one matches.
fn find_claude_file(
    root: &Path,
    uuid: &str,
    slot: &str,
    err: &mut impl Write,
) -> io::Result<Option<PathBuf>> {
    let mut hits = Vec::new();
    for project in safe_directories(root, &root.join("projects"), slot, err)? {
        let candidate = project.join(format!("{uuid}.jsonl"));
        if safe_file(root, &candidate, slot, err)? {
            hits.push(candidate);
        }
    }
    Ok(if hits.len() == 1 { hits.pop() } else { None })
}

/// `<config-home>/sessions/Y/M/D/*<uuid>*.jsonl`, exactly one match.
fn find_codex_file(
    root: &Path,
    uuid: &str,
    slot: &str,
    err: &mut impl Write,
) -> io::Result<Option<PathBuf>> {
    let mut level = vec![root.join("sessions")];
    for _ in 0..3 {
        let mut next = Vec::new();
        for directory in level {
            next.extend(safe_directories(root, &directory, slot, err)?);
        }
        level = next;
    }
    let mut hits = Vec::new();
    for directory in level {
        for file in safe_files(root, &directory, slot, err)? {
            let name = file.file_name().unwrap_or_default().to_string_lossy();
            if name.contains(uuid) && name.ends_with(".jsonl") {
                hits.push(file);
            }
        }
    }
    Ok(if hits.len() == 1 { hits.pop() } else { None })
}

/// Child directories of `directory`, without following symbolic links.
fn safe_directories(
    root: &Path,
    directory: &Path,
    slot: &str,
    err: &mut impl Write,
) -> io::Result<Vec<PathBuf>> {
    Ok(safe_entries(root, directory, slot, err)?
        .into_iter()
        .filter_map(|(path, directory)| directory.then_some(path))
        .collect())
}

/// Child files of `directory`, without following symbolic links.
fn safe_files(
    root: &Path,
    directory: &Path,
    slot: &str,
    err: &mut impl Write,
) -> io::Result<Vec<PathBuf>> {
    Ok(safe_entries(root, directory, slot, err)?
        .into_iter()
        .filter_map(|(path, directory)| (!directory).then_some(path))
        .collect())
}

fn safe_entries(
    root: &Path,
    directory: &Path,
    slot: &str,
    err: &mut impl Write,
) -> io::Result<Vec<(PathBuf, bool)>> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: purge inspects each store node without following symbolic links"
    )]
    let metadata = std::fs::symlink_metadata(directory);
    let Ok(metadata) = metadata else {
        return Ok(Vec::new());
    };
    if metadata.file_type().is_symlink() {
        warn_symlink(directory, slot, err)?;
        return Ok(Vec::new());
    }
    if !metadata.is_dir() || !canonical_parented(root, directory) {
        return Ok(Vec::new());
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the contained conversation-file purge enumerates a validated store directory"
    )]
    let entries = std::fs::read_dir(directory);
    let Ok(entries) = entries else {
        return Ok(Vec::new());
    };
    let mut safe = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: purge rejects descendant links before considering a node"
        )]
        let child = std::fs::symlink_metadata(&path);
        let Ok(child) = child else { continue };
        if child.file_type().is_symlink() {
            warn_symlink(&path, slot, err)?;
        } else if child.is_dir() || child.is_file() {
            safe.push((path, child.is_dir()));
        }
    }
    Ok(safe)
}

fn safe_file(root: &Path, path: &Path, slot: &str, err: &mut impl Write) -> io::Result<bool> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: purge rejects a conversation-file link rather than following it"
    )]
    let metadata = std::fs::symlink_metadata(path);
    let Ok(metadata) = metadata else {
        return Ok(false);
    };
    if metadata.file_type().is_symlink() {
        warn_symlink(path, slot, err)?;
        return Ok(false);
    }
    Ok(metadata.is_file() && canonical_parented(root, path.parent().unwrap_or(path)))
}

fn canonical_parented(root: &Path, path: &Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: every destructive traversal node is canonicalized under its pinned root"
    )]
    let canonical = std::fs::canonicalize(path);
    canonical.is_ok_and(|canonical| canonical.starts_with(root))
}

fn remove_contained(
    root: &Path,
    path: &Path,
    slot: &str,
    err: &mut impl Write,
) -> io::Result<bool> {
    let Some(parent) = path.parent() else {
        return Ok(false);
    };
    if !canonical_parented(root, parent) {
        writeln!(
            err,
            "  note: conversation file for slot {slot} left in place (parent escapes recorded config home): {}",
            path.display()
        )?;
        return Ok(false);
    }
    if !safe_file(root, path, slot, err)? {
        return Ok(false);
    }
    Ok(std::fs::remove_file(path).is_ok())
}

fn warn_symlink(path: &Path, slot: &str, err: &mut impl Write) -> io::Result<()> {
    writeln!(
        err,
        "  note: conversation path for slot {slot} left in place (symbolic link skipped): {}",
        path.display()
    )
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
    use super::{
        Action, Answer, ConfirmedPlan, EndExpectation, ExpectWords, FrozenTarget, FrozenTmux, Git,
        Plan, Prompt, Pushed, Reproof, RunMode, Sweep, TmuxSight, Verdict, answer, archive_step,
        carried_targets, contract_for, early_finish, end_prompt, end_purge, freeze_target, parse,
        path_exists, prompt_route, purge_conversation_files, push_facts, refusal_before_stop,
        reprove, sanitize_branch_name, server_from_words, supervisor_args, unrecorded_verdict,
    };
    use std::path::{Path, PathBuf};

    const UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
    const NOW: i64 = 1_700_000_000;
    const WINDOW: i64 = crate::session_menu::CONFIRM_WINDOW_SECS;

    /// A fresh private state root holding the record of session `alpha`.
    fn planted(tag: &str, meta: Option<&str>) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("ae-end-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("sessions").join("alpha");
        std::fs::create_dir_all(&dir).expect("a session dir");
        if let Some(meta) = meta {
            std::fs::write(dir.join("meta"), meta).expect("a meta");
        }
        (root, dir)
    }

    fn expectation(plan: Plan, deadline: i64) -> EndExpectation {
        EndExpectation {
            uuid: UUID.to_owned(),
            server: server_from_words("socket", "/nowhere/ae.sock").expect("a socket"),
            identity: None,
            deadline,
            confirmed: ConfirmedPlan {
                name: "alpha".to_owned(),
                plan,
                from_default: true,
                tmux: FrozenTmux::Unknown,
            },
            assume_stopped: false,
            pushes: false,
        }
    }

    fn keep() -> Plan {
        Plan {
            action: Action::Keep,
            detail: "/archive/id".to_owned(),
            purge: false,
        }
    }

    #[test]
    fn a_confirmation_is_refused_on_its_stamp_and_record_before_tmux_is_asked() {
        let record = |rows: &str| {
            Some(format!(
                "{rows}tmux_server_kind=socket\ntmux_server=/nowhere/ae.sock\n"
            ))
        };
        let sole = format!("session_id={UUID}\n");
        let table: [(&str, i64, Option<String>, &str); 11] = [
            ("expired", NOW - 1, record(&sole), "expired"),
            ("deadline", NOW, None, "cannot read the metadata"),
            ("future", NOW + WINDOW + 1, record(&sole), "in the future"),
            ("window", NOW + WINDOW, None, "cannot read the metadata"),
            ("no-uuid", NOW, record(""), "was replaced"),
            (
                "double",
                NOW,
                record(&format!("{sole}{sole}")),
                "was replaced",
            ),
            (
                "other",
                NOW,
                record("session_id=0199c0de-bbbb-4890-abcd-ef0123456789\n"),
                "was replaced",
            ),
            (
                "pushes",
                NOW,
                record(&format!("{sole}mode=git\n")),
                "changed",
            ),
            (
                "mode",
                NOW,
                record(&format!("{sole}mode=other\n")),
                "changed",
            ),
            (
                "no-server",
                NOW,
                Some(sole.clone()),
                "no positive server record",
            ),
            (
                "elsewhere",
                NOW,
                Some(format!(
                    "{sole}tmux_server_kind=socket\ntmux_server=/nowhere/other.sock\n"
                )),
                "different tmux server",
            ),
        ];
        for (tag, deadline, meta, want) in table {
            let (root, dir) = planted(&format!("check-{tag}"), meta.as_deref());
            let refused = expectation(keep(), deadline)
                .check(&root, &dir, "alpha", NOW)
                .expect_err(tag);
            assert!(refused.contains(want), "{tag}: {refused}");
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn a_question_says_what_its_frozen_plan_will_do() {
        let purge = Plan {
            action: Action::Purge,
            detail: "/archive/id".to_owned(),
            purge: true,
        };
        let nothing = Plan {
            action: Action::Nothing,
            detail: String::new(),
            purge: false,
        };
        let archive = "Archives it, then deletes its state.";
        let kept = "Its agents' conversation files are kept.";
        let gone = "Its agents' conversation files are DELETED.";
        let table: [(Plan, bool, bool, Vec<&str>); 5] = [
            (keep(), false, false, vec![archive, kept]),
            (
                keep(),
                true,
                false,
                vec![archive, kept, "Commits and pushes its work first."],
            ),
            (
                keep(),
                false,
                true,
                vec![
                    archive,
                    kept,
                    "Server gone; proven stopped by the host reboot.",
                ],
            ),
            (
                purge,
                false,
                false,
                vec!["NO archive, and any archive of it is DELETED.", gone],
            ),
            (
                nothing,
                false,
                false,
                vec!["Nothing to archive; its state is deleted.", kept],
            ),
        ];
        for (plan, pushes, assume_stopped, want) in table {
            let mut expect = expectation(plan, NOW);
            (expect.pushes, expect.assume_stopped) = (pushes, assume_stopped);
            assert_eq!(expect.lines(), want);
        }
    }

    #[test]
    fn unattended_derives_pushes_from_real_metadata_for_every_mode() {
        let push = "Commits and pushes its work first.";
        for (tag, mode, shown) in [
            ("local", "mode=local\n", Some(false)),
            ("git", "mode=git\n", Some(true)),
            ("full", "mode=full\n", Some(true)),
            ("legacy", "", Some(false)),
            ("unknown", "mode=other\n", None),
        ] {
            // A stopped card on a server that never existed, last alive long
            // before this host booted: the capture needs no tmux server.
            let (root, dir) = planted(&format!("derive-{tag}"), None);
            let socket = root.join("never.sock").display().to_string();
            let meta = format!(
                "session=alpha\nsession_id={UUID}\n{mode}started=1\n\
                 tmux_server_kind=socket\ntmux_server={socket}\n"
            );
            std::fs::write(dir.join("meta"), meta).expect("a meta");
            std::fs::write(dir.join("memory.md"), "history\n").expect("a memory");
            let server = server_from_words("socket", &socket).expect("a socket");
            let asked = EndExpectation::unattended(&root, "alpha", UUID, server, true, NOW);
            if let Some(pushes) = shown {
                let lines = asked.expect(tag).lines();
                let said = lines.iter().any(|line| line == push);
                assert_eq!(said, pushes, "{tag}: {lines:?}");
            } else {
                let refused = asked.expect_err(tag);
                assert!(refused.contains("mode"), "{tag}: {refused}");
            }
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn a_question_on_the_default_is_refused_when_the_default_changed_under_it() {
        let (root, dir) = planted("default-moved", None);
        let socket = root.join("never.sock").display().to_string();
        let policy = root.join("policy");
        std::fs::write(&policy, "[workspace]\npurge_agent_history = off\n").expect("a policy");
        let meta = format!(
            "session=alpha\nsession_id={UUID}\nmode=local\nstarted=1\n\
             tmux_server_kind=socket\ntmux_server={socket}\nconfig={}\n",
            policy.display()
        );
        std::fs::write(dir.join("meta"), meta).expect("a meta");
        std::fs::write(dir.join("memory.md"), "history\n").expect("a memory");
        let server = server_from_words("socket", &socket).expect("a socket");
        let expect = EndExpectation::unattended(&root, "alpha", UUID, server, true, NOW)
            .expect("a question on the default");
        (expect.check(&root, &dir, "alpha", NOW)).expect("nothing changed yet");
        std::fs::write(&policy, "[workspace]\npurge_agent_history = on\n").expect("a policy");
        let refused = (expect.check(&root, &dir, "alpha", NOW)).expect_err("the default moved");
        assert!(refused.contains("changed"), "{refused}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_expect_word_is_taken_once_with_a_value_or_refused() {
        let mut words = ExpectWords::default();
        assert!(!words.any());
        words.take("--expect-uuid=u").expect("a word");
        assert!(words.any());
        for (word, want) in [
            ("--expect-uuid=v", "may be given only once"),
            ("--expect-deadline", "requires a value"),
            ("--expect-other=1", "unknown flag"),
        ] {
            let refused = words.take(word).expect_err(word);
            assert!(refused.contains(want), "{word}: {refused}");
        }
    }

    #[test]
    fn a_purge_deletes_the_archive_the_session_already_had() {
        let meta = format!("session={0}\nsession_id={UUID}\nmode=local\n", "alpha");
        let (root, dir) = planted("purge-prior", Some(&meta));
        std::fs::write(dir.join("memory.md"), "kept history\n").expect("a memory");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let keep = Plan {
            detail: root.join("archive").join(UUID).display().to_string(),
            ..keep()
        };
        let archived = archive_step(&root, "alpha", &keep, &Git::none(), &mut out, &mut err);
        assert!(
            archived.expect("the keep ran"),
            "{}",
            String::from_utf8_lossy(&err)
        );
        let prior = root.join("archive").join(UUID);
        assert!(
            path_exists(&prior),
            "the fixture needs a real prior archive"
        );
        let purge = Plan {
            action: Action::Purge,
            detail: keep.detail.clone(),
            purge: true,
        };
        out.clear();
        let purged = archive_step(&root, "alpha", &purge, &Git::none(), &mut out, &mut err);
        assert!(
            purged.expect("the purge ran"),
            "{}",
            String::from_utf8_lossy(&err)
        );
        assert!(!path_exists(&prior), "the prior archive outlived the purge");
        let told = String::from_utf8_lossy(&out);
        assert!(told.contains(&format!("Purged archive {UUID}")), "{told}");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn frozen(name: &str, purge: bool) -> FrozenTarget {
        FrozenTarget {
            name: name.to_owned(),
            plan: Plan {
                action: if purge { Action::Purge } else { Action::Keep },
                detail: "/archive/id".to_owned(),
                purge,
            },
            tmux: FrozenTmux::Unobserved,
        }
    }

    #[test]
    fn the_in_pane_end_prompt_names_the_session_and_the_consequence() {
        let keep = vec![frozen("inside", false)];
        assert_eq!(
            end_prompt("inside", &keep),
            "End 'inside'? Archives, then deletes its state. (y/n)"
        );
        let purge = vec![frozen("inside", true)];
        assert_eq!(
            end_prompt("inside", &purge),
            "End 'inside'? Deletes its state and purges the agent history. (y/n)"
        );
    }

    #[test]
    fn the_supervisor_argv_carries_the_confirmed_plan_and_history_flag() {
        let args =
            parse(&["inside".to_owned(), "--assume-stopped".to_owned()]).expect("valid end args");
        let confirmed = ConfirmedPlan {
            name: "inside".to_owned(),
            plan: Plan {
                action: Action::Keep,
                detail: "/archive/id".to_owned(),
                purge: false,
            },
            from_default: true,
            tmux: FrozenTmux::Live {
                id: "$3".to_owned(),
                created: "1789100001".to_owned(),
            },
        };
        assert_eq!(
            supervisor_args("/core".to_owned(), &args, &confirmed).as_args(),
            [
                "/core",
                "_end",
                "--supervise",
                "inside",
                "--confirmed-target=inside",
                "--confirmed-action=keep",
                "--confirmed-detail=/archive/id",
                "--confirmed-purge=off",
                "--confirmed-source=default",
                "--confirmed-tmux=$3|1789100001",
                "--keep-history",
                "--assume-stopped",
            ]
        );
    }

    #[test]
    fn a_forced_freeze_promises_no_tmux_reading() {
        let root = std::env::temp_dir().join(format!("ae-end-force-freeze-{}", std::process::id()));
        let dir = root.join("sessions").join("inside");
        std::fs::create_dir_all(&dir).expect("a session dir");
        // A positive record on a socket that cannot exist: the unforced
        // freeze reads Unknown here, so `-f` reading Unobserved proves the
        // flag skips the observation rather than the record missing it.
        std::fs::write(
            dir.join("meta"),
            "schema=2\ntmux_server_kind=socket\ntmux_server=/nonexistent-ae-socket\n",
        )
        .expect("a meta with an unreachable record");
        let forced = parse(&["-f".to_owned(), "inside".to_owned()]).expect("valid end args");
        assert_eq!(
            freeze_target(&root, &forced, "inside").tmux,
            FrozenTmux::Unobserved
        );
        let asked = parse(&["inside".to_owned()]).expect("valid end args");
        assert_eq!(
            freeze_target(&root, &asked, "inside").tmux,
            FrozenTmux::Unknown
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_carried_tmux_identity_round_trips_and_rejects_garbage() {
        for (word, expected) in [
            ("absent", FrozenTmux::Absent),
            ("unknown", FrozenTmux::Unknown),
            ("unobserved", FrozenTmux::Unobserved),
            (
                "$3|1789100001",
                FrozenTmux::Live {
                    id: "$3".to_owned(),
                    created: "1789100001".to_owned(),
                },
            ),
        ] {
            let parsed = FrozenTmux::from_word(word).expect("a valid word");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.word(), word);
        }
        for word in [
            "",
            "live",
            "$3",
            "1789100001",
            "3|1789100001",
            "$3|yesterday",
            "$3|1|2",
            "absent|x",
        ] {
            assert_eq!(FrozenTmux::from_word(word), None, "{word}");
        }
    }

    #[test]
    fn a_carried_plan_without_tmux_is_incomplete_not_a_weaker_proof() {
        let tail = [
            "inside",
            "--confirmed-target=inside",
            "--confirmed-action=keep",
            "--confirmed-detail=/archive/id",
            "--confirmed-purge=off",
            "--confirmed-source=explicit",
        ]
        .map(ToOwned::to_owned);
        let err = parse(&tail)
            .err()
            .expect("a plan without tmux is incomplete");
        assert!(err.contains("incomplete"), "{err}");
        let mut tail = tail.to_vec();
        tail.push("--confirmed-tmux=bogus".to_owned());
        let err = parse(&tail).err().expect("a bogus tmux word is refused");
        assert!(err.contains("invalid carried tmux identity"), "{err}");
    }

    #[test]
    fn the_reproof_table_covers_every_frozen_by_seen_cell() {
        let live = |id: &str, created: &str| FrozenTmux::Live {
            id: id.to_owned(),
            created: created.to_owned(),
        };
        let seen = |id: &str, created: &str| TmuxSight::Live {
            id: id.to_owned(),
            created: created.to_owned(),
        };
        // (frozen, seen, proceeds?)
        let rows = [
            (live("$3", "1"), seen("$3", "1"), true),
            (live("$3", "1"), seen("$3", "2"), false),
            (live("$3", "1"), seen("$4", "1"), false),
            (live("$3", "1"), TmuxSight::Absent, false),
            (live("$3", "1"), TmuxSight::Unknown, true),
            (FrozenTmux::Absent, seen("$3", "1"), false),
            (FrozenTmux::Absent, TmuxSight::Absent, true),
            (FrozenTmux::Absent, TmuxSight::Unknown, true),
            (FrozenTmux::Unknown, seen("$3", "1"), false),
            (FrozenTmux::Unknown, TmuxSight::Absent, false),
            (FrozenTmux::Unknown, TmuxSight::Unknown, true),
            (FrozenTmux::Unobserved, seen("$3", "1"), true),
            (FrozenTmux::Unobserved, TmuxSight::Absent, true),
            (FrozenTmux::Unobserved, TmuxSight::Unknown, true),
        ];
        for (frozen, seen, proceeds) in rows {
            match reprove("zzq9", &frozen, &seen) {
                Reproof::Proceed => assert!(proceeds, "{frozen:?} x {seen:?}"),
                Reproof::Refuse(first) => {
                    assert!(!proceeds, "{frozen:?} x {seen:?}");
                    assert!(
                        first.contains("'zzq9'"),
                        "the refusal names the target: {first}"
                    );
                }
            }
        }
    }

    #[test]
    fn purge_uses_each_recorded_config_home_and_legacy_still_uses_the_default() {
        let scratch =
            std::env::temp_dir().join(format!("ae-purge-config-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let home = scratch.join("home");
        let root = home.join(".ae");
        let claude = scratch.join("claude-account");
        let codex = scratch.join("codex-account");
        let legacy = home.join(".claude");
        let claude_id = "11111111-1111-4111-8111-111111111111";
        let codex_id = "22222222-2222-4222-8222-222222222222";
        let legacy_id = "33333333-3333-4333-8333-333333333333";
        let claude_file = claude
            .join("projects/work")
            .join(format!("{claude_id}.jsonl"));
        let codex_file = codex
            .join("sessions/2026/09/08")
            .join(format!("rollout-{codex_id}.jsonl"));
        let legacy_file = legacy
            .join("projects/work")
            .join(format!("{legacy_id}.jsonl"));
        for file in [&claude_file, &codex_file, &legacy_file] {
            std::fs::create_dir_all(file.parent().expect("a store directory")).expect("dirs");
            std::fs::write(file, "conversation").expect("conversation");
        }
        std::fs::create_dir_all(&root).expect("state root");
        let claude = std::fs::canonicalize(&claude).expect("canonical claude root");
        let codex = std::fs::canonicalize(&codex).expect("canonical codex root");
        let meta = format!(
            "schema=2\nseat.main=lead\nagent_bin.main=claude\nconfig_home.main={}\n\
             harness_session.main={claude_id}\nseat.worker.1=pair\nagent_bin.worker.1=codex\n\
             config_home.worker.1={}\nharness_session.worker.1={codex_id}\n\
             seat.worker.2=legacy\nagent_bin.worker.2=claude\nharness_session.worker.2={legacy_id}\n",
            claude.display(),
            codex.display()
        );
        let mut out = Vec::new();
        let mut err = Vec::new();
        purge_conversation_files(&root, "ended", meta.as_bytes(), &mut out, &mut err)
            .expect("purge");
        assert!(!path_exists(&claude_file), "recorded Claude root");
        assert!(!path_exists(&codex_file), "recorded Codex root");
        assert!(!path_exists(&legacy_file), "legacy default root");
        assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn purge_skips_and_reports_descendant_symlinks_that_escape_the_store() {
        use std::os::unix::fs::symlink;

        let scratch = std::env::temp_dir().join(format!("ae-purge-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let home = scratch.join("home");
        let root = home.join(".ae");
        let claude = scratch.join("claude");
        let codex = scratch.join("codex");
        let outside_claude = scratch.join("outside-claude");
        let outside_codex = scratch.join("outside-codex");
        let claude_id = "44444444-4444-4444-8444-444444444444";
        let codex_id = "55555555-5555-4555-8555-555555555555";
        let claude_file = outside_claude.join(format!("{claude_id}.jsonl"));
        let codex_file = outside_codex
            .join("09/08")
            .join(format!("rollout-{codex_id}.jsonl"));
        for file in [&claude_file, &codex_file] {
            std::fs::create_dir_all(file.parent().expect("an outside directory")).expect("dirs");
            std::fs::write(file, "keep").expect("outside conversation");
        }
        std::fs::create_dir_all(claude.join("projects")).expect("claude store");
        std::fs::create_dir_all(codex.join("sessions")).expect("codex store");
        std::fs::create_dir_all(&root).expect("state root");
        symlink(&outside_claude, claude.join("projects/escape")).expect("project link");
        symlink(&outside_codex, codex.join("sessions/2026")).expect("date link");
        let claude = std::fs::canonicalize(&claude).expect("canonical claude root");
        let codex = std::fs::canonicalize(&codex).expect("canonical codex root");
        let meta = format!(
            "schema=2\nseat.main=lead\nagent_bin.main=claude\nconfig_home.main={}\n\
             harness_session.main={claude_id}\nseat.worker.1=pair\nagent_bin.worker.1=codex\n\
             config_home.worker.1={}\nharness_session.worker.1={codex_id}\n",
            claude.display(),
            codex.display()
        );
        let mut out = Vec::new();
        let mut err = Vec::new();
        purge_conversation_files(&root, "ended", meta.as_bytes(), &mut out, &mut err)
            .expect("purge");
        assert!(path_exists(&claude_file) && path_exists(&codex_file));
        let warning = String::from_utf8_lossy(&err);
        assert!(warning.contains("symbolic link skipped"), "{warning}");
        assert!(warning.contains("projects/escape"), "{warning}");
        assert!(warning.contains("sessions/2026"), "{warning}");
        assert!(out.is_empty(), "{}", String::from_utf8_lossy(&out));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_recorded_canonical_root_survives_the_original_symlink_being_retargeted() {
        use std::os::unix::fs::symlink;

        let scratch =
            std::env::temp_dir().join(format!("ae-purge-retarget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let home = scratch.join("home");
        let root = home.join(".ae");
        let first = scratch.join("account-a");
        let second = scratch.join("account-b");
        let link = scratch.join("client");
        let id = "66666666-6666-4666-8666-666666666666";
        let first_file = first.join("projects/w").join(format!("{id}.jsonl"));
        let second_file = second.join("projects/w").join(format!("{id}.jsonl"));
        for file in [&first_file, &second_file] {
            std::fs::create_dir_all(file.parent().expect("a store directory")).expect("dirs");
            std::fs::write(file, "conversation").expect("conversation");
        }
        std::fs::create_dir_all(&root).expect("state root");
        symlink(&first, &link).expect("first target");
        let recorded = std::fs::canonicalize(&link).expect("first-start canonical root");
        std::fs::remove_file(&link).expect("remove old link");
        symlink(&second, &link).expect("retargeted link");
        let meta = format!(
            "schema=2\nseat.main=lead\nagent_bin.main=claude\nconfig_home.main={}\n\
             harness_session.main={id}\n",
            recorded.display()
        );
        let mut out = Vec::new();
        let mut err = Vec::new();
        purge_conversation_files(&root, "ended", meta.as_bytes(), &mut out, &mut err)
            .expect("purge");
        assert!(!path_exists(&first_file), "recorded target is purged");
        assert!(
            path_exists(&second_file),
            "retargeted destination is untouched"
        );
        assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn purge_leaves_unknown_absent_and_malformed_config_homes_intact() {
        let scratch = std::env::temp_dir().join(format!("ae-purge-unknown-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let home = scratch.join("home");
        let root = home.join(".ae");
        let store = home.join(".claude/projects/w");
        let id = "77777777-7777-4777-8777-777777777777";
        let file = store.join(format!("{id}.jsonl"));
        std::fs::create_dir_all(&store).expect("default store");
        std::fs::create_dir_all(&root).expect("state root");
        std::fs::write(&file, "keep").expect("conversation");
        for row in [
            "config_home.main\n",
            "config_home.main=unknown\n",
            "config_home.main=absent\n",
            "config_home.main=relative\n",
            "config_home.main=implicit:/recorded/store\n",
            "config_home.main=/one\nconfig_home.main=/two\n",
        ] {
            let meta = format!(
                "schema=2\nseat.main=lead\nagent_bin.main=claude\n{row}\
                 harness_session.main={id}\n"
            );
            let mut out = Vec::new();
            let mut err = Vec::new();
            purge_conversation_files(&root, "ended", meta.as_bytes(), &mut out, &mut err)
                .expect("purge");
            assert!(path_exists(&file), "{row:?}");
            assert!(out.is_empty(), "{row:?}");
            assert!(
                String::from_utf8_lossy(&err).contains("left in place"),
                "{row:?}: {}",
                String::from_utf8_lossy(&err)
            );
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }

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
        purge_conversation_files(&root, "ended", meta.as_bytes(), &mut out, &mut err)
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

    /// A recorded id is a NAME, not a path — and the purge is `remove_file`.
    ///
    /// `harness_session.<slot>` is hand-editable metadata that only ever gets
    /// screened for control bytes, so an absolute value makes `Path::join`
    /// discard the store and a dot-dot value walks out of it. Either one turns a
    /// roster row into a deletion target anywhere the user can write. The guard
    /// is at the row, so this covers claude and codex too.
    #[test]
    fn a_recorded_id_that_is_not_a_uuid_names_nothing_and_deletes_nothing() {
        let scratch = std::env::temp_dir().join(format!("ae-purge-esc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let home = scratch.join("home");
        let root = home.join(".ae");
        let store = home.join(".gemini/antigravity-cli/conversations");
        std::fs::create_dir_all(&store).expect("a conversation store");
        std::fs::create_dir_all(&root).expect("a state root");

        // Two bystanders OUTSIDE the store, each named by one of the escapes.
        let outside = scratch.join("outside");
        std::fs::create_dir_all(&outside).expect("a bystander dir");
        let absolute = outside.join("absolute");
        let dotdot = home.join(".gemini/antigravity-cli/escaped.db");
        std::fs::write(&absolute, "keep me").expect("a bystander");
        std::fs::write(&dotdot, "keep me").expect("a bystander");

        for (tool, id) in [
            ("agy", absolute.display().to_string()),
            ("agy", "../escaped".to_owned()),
            ("claude", absolute.display().to_string()),
            ("codex", "../escaped".to_owned()),
            // Shapes that are close to a UUID but are not one.
            ("agy", "643393ad-eb92-4b9e-ab7a".to_owned()),
            (
                "agy",
                "643393ad-eb92-4b9e-ab7a-0fe7b1221fa1/../../x".to_owned(),
            ),
        ] {
            let meta = format!(
                "session=x\nschema=2\nseat.main=lead\nagent_bin.main={tool}\n\
                 harness_session.main={id}\n"
            );
            let mut out = Vec::new();
            let mut err = Vec::new();
            purge_conversation_files(&root, "ended", meta.as_bytes(), &mut out, &mut err)
                .expect("the purge reports rather than fails");
            let said = String::from_utf8_lossy(&out).into_owned();
            let noted = String::from_utf8_lossy(&err).into_owned();
            assert!(
                said.is_empty(),
                "nothing may be reported removed for {tool} id {id}: {said}"
            );
            assert!(
                noted.contains("is not a UUID"),
                "and the loss must be reported for {tool} id {id}: {noted}"
            );
            assert!(
                path_exists(&absolute) && path_exists(&dotdot),
                "a file outside the store was removed via {tool} id {id}"
            );
        }
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

    fn confirmed(purge: bool, from_default: bool) -> ConfirmedPlan {
        ConfirmedPlan {
            name: "alpha".to_owned(),
            plan: Plan {
                action: Action::Keep,
                detail: "/archive/id".to_owned(),
                purge,
            },
            from_default,
            tmux: FrozenTmux::Unobserved,
        }
    }

    #[test]
    fn the_end_purge_takes_the_confirmed_choice_unless_config_made_it() {
        for cli in [None, Some(true), Some(false)] {
            assert_eq!(
                end_purge(None, cli),
                cli,
                "nothing confirmed: the command line"
            );
            for purge in [true, false] {
                assert_eq!(
                    end_purge(Some(&confirmed(purge, true)), cli),
                    None,
                    "config made it: config decides again under the lock"
                );
                assert_eq!(
                    end_purge(Some(&confirmed(purge, false)), cli),
                    Some(purge),
                    "the confirmed choice, never the command line"
                );
            }
        }
    }

    #[test]
    fn a_recordless_target_proceeds_only_on_a_clear_sweep_and_the_flag() {
        let found = Sweep::Found("default".to_owned());
        let unsure = Sweep::Unsure("other".to_owned());
        for flag in [false, true] {
            assert_eq!(
                unrecorded_verdict("s", &found, flag),
                Verdict::Refuse(vec![
                    "Error: session 's' has no positive server record AND is LIVE on enumerable server 'default'.".to_owned(),
                    "  Establish ownership first: 'ae doctor --refresh s'.".to_owned(),
                    "  State preserved.".to_owned(),
                ])
            );
            assert_eq!(
                unrecorded_verdict("s", &unsure, flag),
                Verdict::Refuse(vec![
                    "Error: cannot verify enumerable tmux server 'other' (socket exists but did not answer) — session 's' could be hiding behind it.".to_owned(),
                    "  Fix that server (permissions/staleness), then retry. State preserved.".to_owned(),
                ])
            );
        }
        assert_eq!(
            unrecorded_verdict("s", &Sweep::Clear, false),
            Verdict::Refuse(vec![
                "Error: session 's' has no positive server record (pre-fix or ambiguous meta) — it may be live somewhere ae cannot enumerate.".to_owned(),
                "  Resolve: 'ae doctor --refresh s', or acknowledge it is stopped: ae end --assume-stopped s".to_owned(),
                "  State preserved.".to_owned(),
            ])
        );
        assert_eq!(
            unrecorded_verdict("s", &Sweep::Clear, true),
            Verdict::Proceed(
                "Proceeding on explicit --assume-stopped acknowledgement (not live on any enumerable server, default included)."
            )
        );
    }

    #[test]
    fn the_refusals_before_the_stop_apply_in_order() {
        let rerun = "  Nothing was stopped and nothing was deleted. Re-run 'ae end s' to see the current plan.";
        let unavailable = Plan {
            action: Action::Unavailable,
            detail: "why".to_owned(),
            purge: false,
        };
        let other = Plan {
            purge: true,
            ..keep()
        };
        let live = TmuxSight::Live {
            id: "$2".to_owned(),
            created: "9".to_owned(),
        };
        let frozen = FrozenTmux::Live {
            id: "$1".to_owned(),
            created: "9".to_owned(),
        };
        // Unidentifiable comes first, contract or not.
        let first = refusal_before_stop("s", &unavailable, Some((&other, &frozen)), Some(&live));
        assert_eq!(
            first,
            Some(vec![
                "Error: session 's' cannot be archived — why.".to_owned(),
                "  Nothing was stopped and NOTHING was deleted; ae does not delete a session it could not capture.".to_owned(),
            ])
        );
        // Nothing confirmed: nothing to re-prove.
        assert_eq!(refusal_before_stop("s", &keep(), None, Some(&live)), None);
        // A different incarnation outranks a changed plan.
        let mismatch = refusal_before_stop("s", &keep(), Some((&other, &frozen)), Some(&live));
        assert_eq!(
            mismatch,
            Some(vec![
                "Error: 's' is a different tmux session than the one confirmed.".to_owned(),
                "  Confirmed: tmux $1 (created 9)".to_owned(),
                "  Now:       tmux $2 (created 9)".to_owned(),
                rerun.to_owned(),
            ])
        );
        // No sight (a recordless target): only the plan is compared.
        let changed = refusal_before_stop("s", &keep(), Some((&other, &frozen)), None);
        assert_eq!(
            changed,
            Some(vec![
                "Error: what 's' would do changed between the confirmation and now — refusing to act on an answer given to a different question.".to_owned(),
                format!("  Confirmed: {}", other.line("s")),
                format!("  Now:       {}", keep().line("s")),
                rerun.to_owned(),
            ])
        );
        let same = FrozenTmux::Live {
            id: "$2".to_owned(),
            created: "9".to_owned(),
        };
        assert_eq!(
            refusal_before_stop("s", &keep(), Some((&keep(), &same)), Some(&live)),
            None
        );
    }

    #[test]
    fn only_local_mode_or_a_retry_finishes_without_git() {
        let retried = || Git {
            outcome: "pushed".to_owned(),
            ..Git::none()
        };
        for mode in ["local", ""] {
            let (git, ended) = early_finish(mode, None).expect("local finishes early");
            assert_eq!(
                (git.outcome.as_str(), ended),
                ("not-managed", "Ended local session")
            );
            let (git, ended) = early_finish(mode, Some(retried())).expect("local retry");
            assert_eq!(
                (git.outcome.as_str(), ended),
                ("pushed", "Ended local session")
            );
        }
        for mode in ["git", "full", "weird"] {
            assert!(
                early_finish(mode, None).is_none(),
                "{mode} takes the git path"
            );
            let (git, ended) = early_finish(mode, Some(retried())).expect("a retry skips git");
            assert_eq!((git.outcome.as_str(), ended), ("pushed", "Ended"));
        }
    }

    #[test]
    fn the_push_facts_keep_every_unnamed_field_at_its_none_value() {
        let facts = |pushed| {
            let (git, preserve) = push_facts(pushed, Path::new("/w d"));
            (
                [git.outcome, git.push_ref, git.preserved, git.workdir],
                git.retry_id,
                preserve,
            )
        };
        let row = |cells: [&str; 4]| cells.map(str::to_owned);
        assert_eq!(
            facts(Pushed::Branch("ae/s".to_owned())),
            (row(["pushed", "origin/ae/s", "-", "/w d"]), None, false)
        );
        assert_eq!(
            facts(Pushed::Reachable),
            (row(["already-reachable", "-", "-", "/w d"]), None, false)
        );
        assert_eq!(
            facts(Pushed::NoOrigin),
            (row(["no-origin", "-", "/w d", "/w d"]), None, true)
        );
    }

    fn carry(name: &str) -> ConfirmedPlan {
        ConfirmedPlan {
            name: name.to_owned(),
            ..confirmed(false, false)
        }
    }

    #[test]
    fn a_handoff_ends_the_carried_names_only_when_they_answer_the_operand_once_each() {
        let names = |plans: &[ConfirmedPlan], target: &str| carried_targets(target, plans);
        let owned = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();
        assert_eq!(names(&[carry("a")], "a"), Some(owned(&["a"])));
        assert_eq!(names(&[carry("a")], "b"), None, "one plan for another name");
        assert_eq!(
            names(&[carry("a"), carry("b")], "a"),
            None,
            "a named target takes exactly one plan, even when one of them is its own"
        );
        assert_eq!(
            names(&[carry("b"), carry("a")], "all"),
            Some(owned(&["b", "a"])),
            "all takes every distinct plan, in carried order"
        );
        assert_eq!(
            names(&[carry("a"), carry("b"), carry("a")], "all"),
            None,
            "a name carried twice is refused even for all"
        );
        assert_eq!(names(&[carry("a"), carry("a")], "a"), None);
    }

    #[test]
    fn the_terminal_is_looked_at_exactly_when_an_unforced_direct_end_must_ask() {
        let route = |mode, force, caller, tty| {
            let reads = std::cell::Cell::new(0_u32);
            let prompt = prompt_route(mode, force, caller, || {
                reads.set(reads.get() + 1);
                tty
            });
            (prompt, reads.get())
        };
        for force in [false, true] {
            for caller in [None, Some("own")] {
                for tty in [false, true] {
                    assert_eq!(
                        route(RunMode::Handoff, force, caller, tty),
                        (Prompt::Handoff, 0),
                        "handoff never asks: force={force} caller={caller:?} tty={tty}"
                    );
                    assert_eq!(
                        route(RunMode::Direct, true, caller, tty),
                        (Prompt::Forced, 0),
                        "-f never asks: caller={caller:?} tty={tty}"
                    );
                }
            }
        }
        assert_eq!(
            route(RunMode::Direct, false, Some("own"), false),
            (Prompt::Client("own"), 1),
            "no terminal inside a session asks the caller's client"
        );
        assert_eq!(
            route(RunMode::Direct, false, Some("own"), true),
            (Prompt::Terminal, 1),
            "a terminal inside a session is asked directly"
        );
        for tty in [false, true] {
            assert_eq!(
                route(RunMode::Direct, false, None, tty),
                (Prompt::Terminal, 1),
                "outside a session the terminal is asked, and looked at once: tty={tty}"
            );
        }
    }

    #[test]
    fn only_a_line_starting_with_y_confirms_and_no_line_is_no_input() {
        for yes in ["y", "Y", "yes\n", "Yes please\n", "y\n"] {
            assert_eq!(answer(Some(yes)), Answer::Yes, "{yes:?}");
        }
        for no in ["", "\n", "n\n", "no\n", "N", " y", "ja\n"] {
            assert_eq!(answer(Some(no)), Answer::No, "{no:?}");
        }
        assert_eq!(answer(None), Answer::NoInput);
    }

    #[test]
    fn a_target_ends_under_its_carried_plan_else_the_shown_plan_unless_forced() {
        let carried = [
            "--confirmed-target=alpha",
            "--confirmed-action=keep",
            "--confirmed-detail=/carried",
            "--confirmed-purge=off",
            "--confirmed-source=explicit",
            "--confirmed-tmux=unknown",
        ];
        let args = |words: &[&str]| {
            let words: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
            parse(&words).expect("well-formed words")
        };
        let shown = frozen("alpha", true);
        let carried_plan = Plan {
            action: Action::Keep,
            detail: "/carried".to_owned(),
            purge: false,
        };
        for force in [&[][..], &["-f"][..]] {
            let mut words = vec!["alpha"];
            words.extend_from_slice(force);
            words.extend_from_slice(&carried);
            assert_eq!(
                contract_for(&args(&words), &shown),
                Some((&carried_plan, &FrozenTmux::Unknown)),
                "a carried plan wins, with or without -f: {force:?}"
            );
            assert_eq!(
                contract_for(&args(&words), &frozen("beta", true)),
                force
                    .is_empty()
                    .then_some((&frozen("beta", true).plan, &FrozenTmux::Unobserved)),
                "another name's carried plan is not this target's: {force:?}"
            );
        }
        assert_eq!(
            contract_for(&args(&["alpha"]), &shown),
            Some((&shown.plan, &shown.tmux)),
            "unforced, the shown plan is the contract"
        );
        assert_eq!(
            contract_for(&args(&["-f", "alpha"]), &shown),
            None,
            "-f promised nothing"
        );
    }
}
