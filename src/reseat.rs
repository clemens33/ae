//! `ae reseat <session> <agent> --using <profile> [--stop-unknown]`: move ONE
//! seat to another profile, in place.
//!
//! The pain, measured across this fleet: a seat whose vendor quota dies is lost
//! WITH ITS CONTEXT. The only way on was a fresh spawn under a new name plus a
//! handover written by hand, and the seat's slot, pane, name and history stayed
//! bound to a tool that can no longer answer.
//!
//! This moves the SAME seat: same slot, same pane, same name, same records. The
//! successor arrives on another tool and is handed ae's own account of the seat
//! — the seed pack `ae brief --seat` renders, the predecessor's last turns
//! included — as its first turn.
//!
//! IT DOES THE FULL ROUND. A seat whose tool is still running is STOPPED IN
//! PLACE first — same pane id, same pane options, same scrollback, the shell
//! left idle in the session's recorded working copy — and only then moved. A
//! seat that is already gone takes exactly the path it took before.
//!
//! The stop is never silent and never guessed: [`stop_running_tool`] owns its
//! rule. Past it the seat must still pass
//! [`crate::seat_relaunch::prove_dead`] — the same owner, asked with this
//! verb's word, so the two commands refuse for the same reasons in the same
//! order.
//!
//! The order is the contract, and everything before the stop is undone by
//! doing nothing:
//!
//! 1. the argv, the session, the caller, the seat and the profile resolve —
//!    no state is written;
//! 2. under the session's lifecycle lock, a running tool is stopped and the
//!    stop is recorded; then the seat is proven dead;
//! 3. the seed pack is built (it reads the seat's recorded first message,
//!    which step 4 removes) and published as `seed.<agent>.md`;
//! 4. every per-slot launch file goes, so the successor's `_run` has no stale
//!    first message, no stale sid file and no start marker to resume from;
//! 5. the meta rows move in ONE guarded replacement;
//! 6. the pane line is pasted and the new tool observed;
//! 7. past the lock: the tool's own launch turn, then the seed.
//!
//! Three crash windows, all named and all recoverable by hand: between the
//! stop and 4 the seat still records the OLD profile and its pane is at a
//! shell, so `relaunch` brings it back on the tool it had; between 4 and 5 the
//! same is true with the launch files gone; between 5 and 6 the meta names the
//! NEW profile and the pane sits at its shell, which is exactly what
//! `relaunch` finishes.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::harness_state::HarnessState;
use crate::inventory::ServerId;
use crate::procs::{self, Descendancy};
use crate::seat_relaunch::{Proven, RESEAT_VERB, Started, Target};
use crate::session_tmux::Op;
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;
use crate::tmux::ObservedPaneProbe;
use crate::tool::ToolKind;
use crate::tracked::{self, EventFields};
use crate::transport;

/// The usage line.
pub const RESEAT_USAGE: &str =
    "Usage: ae reseat <session> <agent> --using <profile> [--stop-unknown]";

/// The event action a reseat records, whatever its outcome.
const RESEAT_ACTION: &str = "reseat";

/// How long the stop waits for the pane to come back to an idle shell.
const STOP_POLLS: u32 = 50;

/// The pause between those polls — 50 x 200ms is the 10s bound the docs state.
const STOP_POLL: Duration = Duration::from_millis(200);

/// The gap between the two frame readings the stop takes.
///
/// ONE reading is not enough: between a tool receiving Enter and drawing its
/// spinner the box is empty and no turn is drawn yet, and a single frame in
/// that gap reads IDLE for a seat that has just been given work.
const FRAME_GAP: Duration = Duration::from_secs(1);

/// How long the stop waits for the pane's send-lock.
///
/// Short, and its own refusal, because this is asked while the session's
/// lifecycle lock is already held: a delivery in flight is a reason to come
/// back, never a reason to hold a session out of its own lifecycle.
const SEND_LOCK_WAIT: Duration = Duration::from_secs(5);

/// The argv, validated.
struct Parsed {
    session: String,
    agent: String,
    profile: String,
    /// `--stop-unknown`: stop a tool whose frame ae cannot read. It lifts the
    /// UNKNOWN refusal and nothing else — a frame that reads BUSY is refused
    /// with or without it.
    stop_unknown: bool,
}

/// Parse `<session> <agent> --using <profile>`.
///
/// The flag may sit anywhere, as `spawn`'s does, because that is where a hand
/// puts it. Anything else beginning with `-` is refused rather than swallowed
/// as a name.
fn parse(tail: &[String]) -> Result<Parsed, String> {
    let mut names: Vec<&str> = Vec::new();
    let mut profile = String::new();
    let mut stop_unknown = false;
    let mut words = tail.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            // Anywhere, like `--using`, because that is where a hand puts it.
            "--stop-unknown" => stop_unknown = true,
            "--using" => {
                let Some(value) = words.next() else {
                    return Err("Error: --using needs a profile name.".to_owned());
                };
                profile.clone_from(value);
            }
            flag if flag.starts_with("--using=") => {
                flag["--using=".len()..].clone_into(&mut profile);
            }
            flag if flag.starts_with('-') => {
                return Err(format!("Error: unknown flag '{flag}'."));
            }
            name => names.push(name),
        }
    }
    let [session, agent] = names.as_slice() else {
        return Err(RESEAT_USAGE.to_owned());
    };
    if profile.is_empty() {
        return Err("Error: reseat needs --using <profile>.".to_owned());
    }
    // The same line a launch draws around its own override: `profile@client` is
    // a LAUNCH grammar, and this verb writes no client row — it removes the one
    // the launch left, because that override belongs to a profile the seat no
    // longer runs.
    if profile.contains('@') {
        return Err(format!(
            "Error: --using '{profile}' names a client override — profile@client is launch-only. \
             reseat takes a bare profile."
        ));
    }
    Ok(Parsed {
        session: (*session).to_owned(),
        agent: (*agent).to_owned(),
        profile,
        stop_unknown,
    })
}

/// What the carry question answered, and what the audit line says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Carried {
    /// Not a carry question at all — a tool change, a move inside one account,
    /// a seat with no conversation, or a tool whose store ae has not measured.
    /// SILENT: this move behaves exactly as it did before carrying existed, and
    /// says nothing new on either stream or in its record.
    No,
    /// The conversation's files are in the new account.
    Yes,
    /// The carry was live and REFUSED. The move still happens — a fresh
    /// conversation and the seed pack — and this is what it could not do.
    Seeded(String),
}

impl Carried {
    /// The word the reseat record carries, empty when there was no question.
    fn word(&self) -> String {
        match self {
            Self::No => String::new(),
            Self::Yes => "carried".to_owned(),
            Self::Seeded(why) => format!("seeded ({why})"),
        }
    }
}

/// Does this move's conversation travel? `None` is every silent arm.
///
/// Reads the seat's recorded account rather than re-resolving the profile it is
/// leaving: for a retained conversation the RECORD is what names the store, and
/// re-deriving it here could send the copy looking in a home this seat never
/// used.
fn carry_plan(
    locked: &[u8],
    slot: &str,
    moving: &Moving,
    id: &str,
    work_dir: &str,
) -> Option<crate::carry::Plan> {
    let recorded = crate::meta::Meta::parse(&String::from_utf8_lossy(locked))
        .roster()
        .iter()
        .find(|row| row.slot == slot)
        .map(|row| row.config_home.clone())?;
    let (crate::meta::RecordedConfigHome::Path(from)
    | crate::meta::RecordedConfigHome::Implicit(from)) = recorded
    else {
        return None;
    };
    let account = moving.account.as_ref()?;
    let from_binary = crate::lifecycle::meta_value(locked, &format!("agent_bin.{slot}"));
    crate::carry::plan(&crate::carry::Move {
        from_binary: &from_binary,
        to_binary: &moving.binary,
        tool: moving.tool,
        id,
        work_dir: Path::new(work_dir),
        from: Some(&from),
        to: Some(&account.path),
    })
}

/// Where this seat's seed is published — beside the meta, 0600, and KEPT after
/// a successful move: it is what a human re-sends by hand when the turn did not
/// land, and what tells a later reader what the successor was told.
///
/// The name goes through the same sanitiser a slot does, for the same reason:
/// this is a FILENAME COMPONENT, and the agent it is built from is read back
/// out of a pane option and a hand-editable meta rather than re-validated
/// against the agent grammar here. ONE owner, so the path this publishes and
/// the path a refusal tells a human to `cat` cannot disagree.
fn seed_file(dir: &Path, agent: &str) -> PathBuf {
    dir.join(format!("seed.{}.md", crate::launch::safe_slot(agent)))
}

/// Who ran this, read ONCE: the caller rule, the self-reseat refusal and the
/// event's actor are three questions about the same pane.
#[derive(Default)]
struct Caller {
    /// `$TMUX_PANE`, when this ran inside tmux at all.
    pane: Option<String>,
    /// `@ae_agent` — the display ref a record names. Empty for a plain shell,
    /// which the record renders as `human`, exactly as `relaunch` does.
    display: String,
}

/// THE CALLER RULE. `Err` is the refusal, `Ok` the caller.
///
/// A pane ae STAMPED is a seat, and a seat may reseat only inside its own
/// session — the boundary `relaunch` draws, for the same reason. A plain shell
/// carries no stamp and may reseat any session, because the human is who this
/// verb is for: the moment a lead's own quota dies, no agent of that session
/// can run anything.
fn caller_of(session: &str) -> Result<Caller, String> {
    let pane = crate::doors::calling_pane_id();
    let Some((viewer, _)) = crate::actual_calling_pane() else {
        return Ok(Caller {
            pane,
            display: String::new(),
        });
    };
    let display = viewer.agent.clone().unwrap_or_default();
    let Some(slot) = viewer.slot.clone().filter(|slot| !slot.is_empty()) else {
        return Ok(Caller { pane, display });
    };
    match viewer.session.as_deref() {
        Some(name) if name == session => Ok(Caller { pane, display }),
        Some(name) => Err(format!(
            "Error: this pane is slot {slot} of '{name}', and reseat works on the caller's own \
             session only — run it from '{session}', or from a shell outside ae."
        )),
        None => Err(format!(
            "Error: this pane carries ae slot {slot} but tmux did not say which session it is in \
             — ae will not reseat '{session}' for a caller it cannot place."
        )),
    }
}

/// Resolve `agent` to a seat of `session`.
///
/// Deliberately NOT `relaunch`'s resolver: that one refuses every session but
/// the caller's, and this verb's caller may be a shell in none. The refusal
/// that matters here is [`caller_refusal`], already made.
fn resolve(dir: &Path, agent: &str, session: &str) -> Result<Target, String> {
    let (resolved, server): (tracked::Resolved, ServerId) =
        tracked::resolve_on(agent, session, dir)
            .map_err(|why| tracked::ResolveError::message(&why))?;
    if !resolved.session.is_empty() && resolved.session != session {
        return Err(format!(
            "Error: '{agent}' is on session '{}', not '{session}'.",
            resolved.session
        ));
    }
    Ok(Target {
        agent: if resolved.agent.is_empty() {
            agent.to_owned()
        } else {
            resolved.agent
        },
        slot: resolved.slot,
        pane: resolved.pane,
        server,
    })
}

/// The new profile resolved to the two facts the meta records.
///
/// The SAME resolution `crate::run::read_seat` makes for a seat that records no
/// client override, so a profile this verb accepts is one `_run` can launch —
/// asked BEFORE anything is written, because a profile that does not lex is a
/// refusal and not a half-moved seat.
fn resolve_profile(dir: &Path, bytes: &[u8], profile: &str, agent: &str) -> Result<Moving, String> {
    let value = |key: &str| crate::lifecycle::meta_value(bytes, key);
    let global = value("config");
    let local = crate::config::local_overlay(dir, &value("origin"));
    // The orchestrator seat's own overlay is not a profile source, exactly as
    // `read_seat` has it: reading it here would offer a profile the launch
    // would then refuse.
    let orchestrator_seat = local.as_deref().is_some_and(|path| {
        dir.parent()
            .and_then(Path::parent)
            .is_some_and(|home| crate::orchestrator::is_seat_overlay(path, home))
    });
    let cfg = crate::config::read_identity(
        (!global.is_empty()).then(|| Path::new(&global)),
        (!orchestrator_seat).then_some(local.as_deref()).flatten(),
    )
    .map_err(|why| why.to_string())?;
    let home = crate::doors::home();
    let command = cfg
        .command(profile, home.as_deref())
        .map_err(|why| why.to_string())?;
    let Some(command) = command.filter(|command| !command.as_str().trim().is_empty()) else {
        return Err(format!(
            "profile '{profile}' is not configured on this machine — '{agent}' cannot be moved to it"
        ));
    };
    let parsed = crate::launch_cmd::lex_simple_command(command.as_str())
        .map_err(|why| format!("profile '{profile}' is not one simple command — {why}"))?;
    // NO tool-class judgement here, deliberately: a seat may be moved to
    // exactly the profiles a LAUNCH accepts, and the launch asks the adapter
    // rows for its behaviour rather than refusing a binary it does not know.
    Ok(Moving {
        account: account_of(&command, parsed.tool(), home.as_deref()),
        tool: parsed.tool(),
        binary: parsed.binary,
    })
}

/// The new profile's recorded facts.
struct Moving {
    tool: ToolKind,
    binary: String,
    /// Where this profile's conversations live, when ae can say.
    account: Option<Account>,
}

/// The account a profile selects, resolved once and kept whole: the path a
/// carry copies INTO, and the two rows that record it.
///
/// Held together because they are one identity — an implicit store and an
/// explicit one over the same path are different accounts, and a row published
/// without its base would say the wrong one.
struct Account {
    /// The canonical directory itself.
    path: PathBuf,
    /// `config_home.<slot>`, in [`crate::meta::RecordedConfigHome`]'s grammar.
    row: String,
    /// `config_home_base.<slot>` — the canonical effective `HOME` an implicit
    /// store was selected against, and `None` for an explicit one.
    base: Option<String>,
}

/// Resolve the account a profile's command would give its tool.
///
/// The lookup is CONTROLLED, not ambient — `HOME` is the launch home and every
/// other variable is unset — because this runs in the CALLER's process and the
/// caller's environment is not the pane's. It is the reading
/// `session_launch::refuse_store_conflict` takes for exactly the same reason,
/// and it maps to a recorded row by `run::config_home_identity`'s rule for a
/// seat with nothing recorded yet.
///
/// `None` means ae cannot name this profile's account: a `$VAR` only the pane
/// can see, no effective `HOME`, or a path that will not canonicalize. Nothing
/// is refused on it — a move whose account ae cannot name simply cannot carry.
fn account_of(
    command: &crate::config::ResolvedCommand,
    tool: ToolKind,
    home: Option<&Path>,
) -> Option<Account> {
    let home_value = home.map(|path| path.display().to_string());
    let resolution = crate::launch_cmd::config_home_resolution(command, tool, &|name| {
        (name == "HOME").then(|| home_value.clone()).flatten()
    });
    let crate::launch_cmd::Resolved::Path(path) =
        crate::run::canonical_config_home(&resolution.home).ok()?
    else {
        return None;
    };
    if resolution.explicit {
        return Some(Account {
            row: path.display().to_string(),
            path,
            base: None,
        });
    }
    // An IMPLICIT store is only an identity together with the HOME that
    // selected it, so a base ae cannot canonicalize is not half an answer.
    let crate::launch_cmd::Resolved::Path(base) =
        crate::run::canonical_config_home(&resolution.base).ok()?
    else {
        return None;
    };
    Some(Account {
        row: format!("implicit:{}", path.display()),
        path,
        base: Some(base.display().to_string()),
    })
}

/// What the stop step concluded.
enum Stop {
    /// The seat's tool was not running. NOTHING was read beyond the pane, and
    /// every refusal a dead or unreadable seat deserves belongs to
    /// [`crate::seat_relaunch::prove_dead`], which runs next — this arm is how
    /// an already-dead seat stays byte-identical to what it was before the
    /// stop existed.
    NotRunning,
    /// The tool was stopped and the pane is back at an idle shell, carrying
    /// the binary that was stopped for the record.
    Stopped(String),
    /// A refusal was printed. Nothing was killed: every arm that reaches this
    /// is decided BEFORE the respawn, except the bounded wait, which says so.
    Refused,
}

/// Where the caller stands relative to the seat whose tool is about to be
/// ended — PURE, from the readings the stop already took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallerStanding {
    /// The caller's process tree does not place it under the seat's pane.
    Clear,
    /// The caller runs BENEATH that pane: ending the tool would kill it.
    Under,
    /// A reading is missing, so the two cannot be told apart.
    Unprovable,
}

/// FAIL-CLOSED BY CONSTRUCTION: a pane with no readable pid, or a process
/// table ae could not take, is [`CallerStanding::Unprovable`] and never
/// permission to kill. `Clear` is the only answer the stop may act on.
fn caller_standing(
    pane_pid: Option<u32>,
    table: Option<&[procs::Proc]>,
    me: u32,
) -> CallerStanding {
    let (Some(pane_pid), Some(rows)) = (pane_pid, table) else {
        return CallerStanding::Unprovable;
    };
    if procs::is_descendant_of(rows, pane_pid, me) {
        CallerStanding::Under
    } else {
        CallerStanding::Clear
    }
}

/// PURE: is `probe` a pane back at an IDLE shell?
///
/// The same two questions [`crate::seat_relaunch::prove_dead`] asks, asked of
/// the readings the wait already has — a shell in the foreground and nothing
/// running under it. An unreadable pid or an unusable process table is FALSE,
/// so the wait keeps waiting and then refuses rather than declaring a pane
/// idle it could not read.
fn pane_back_at_shell(probe: &ObservedPaneProbe, table: Option<&[procs::Proc]>) -> bool {
    probe.pid.is_some()
        && crate::watchdog::command_is_shell(&probe.command)
        && table.is_some_and(|table| !procs::has_any_descendant(table, probe.pid))
}

/// What a timed-out stop SAYS, from the last reading it could take.
///
/// The two cases need different advice and conflating them wastes a human's
/// time. A SHELL in the foreground means the respawn took and something runs
/// under it, which usually finishes — re-running the reseat is the right next
/// step. A NON-shell foreground means the pane came back running something
/// else, which for an ae seat pane cannot happen and for a pane made by hand
/// means the respawn restarted that pane's own start command: no number of
/// retries changes it, so the advice is to look rather than try again.
fn stop_timeout_advice(probe: Option<&ObservedPaneProbe>) -> (String, &'static str) {
    let Some(probe) = probe else {
        return (
            "the pane stopped answering".to_owned(),
            "look at the pane before trying again.",
        );
    };
    if crate::watchdog::command_is_shell(&probe.command) {
        return (
            "its shell is back but something still runs under it".to_owned(),
            "re-run the reseat once that finishes.",
        );
    }
    (
        format!("'{}' holds its foreground", probe.command),
        "look at the pane: the respawn did not leave a shell, so re-running will not help.",
    )
}

/// The seat's current harness frame, fail-closed.
///
/// A capture ae could not take is [`HarnessState::Unknown`], never idle: the
/// absence of a reading is not evidence that a turn is not running.
fn frame(target: &Target, tool: ToolKind) -> HarnessState {
    transport::capture_pane(&target.server, &target.pane).map_or(HarnessState::Unknown, |capture| {
        crate::harness_state::classify(&capture, tool)
    })
}

/// STOP THE SEAT'S TOOL, in place. Runs UNDER the lifecycle lock, BEFORE the
/// dead proof and before anything durable is written.
///
/// The mechanism is `respawn-pane -k`, the same tmux verb ae already uses to
/// replace a monitor process: measured on tmux 3.7b it keeps the pane id, the
/// pane's `@ae_*` options, its index and its scrollback, kills the pane's
/// process tree, and leaves tmux's default shell in the directory `-c` names.
/// No new process door, and no signal ae would need `unsafe` to send.
#[allow(
    clippy::too_many_lines,
    reason = "the stop ladder, kept in one place beside the order it enforces"
)]
fn stop_running_tool(
    dir: &Path,
    target: &Target,
    bytes: &[u8],
    stop_unknown: bool,
    err: &mut impl Write,
) -> io::Result<Stop> {
    let agent_bin = crate::deliver::recorded_binary(dir, &target.slot);
    // IS IT RUNNING? Read the way the dead proof reads it, from ONE probe and
    // ONE process-table snapshot. A seat with no recorded tool, or a pane that
    // will not answer, is not something this step may kill: the dead proof
    // names both, in its own words, a moment from now.
    let Some(probe) = transport::observe_pane_probe(&target.server, &target.pane) else {
        return Ok(Stop::NotRunning);
    };
    let table = procs::snapshot();
    let walk = probe.pid.map_or(Descendancy::Unknown, |pid| {
        procs::descendancy(table.as_deref(), pid, &agent_bin)
    });
    if !crate::seat_relaunch::identity_proven(&probe.command, &agent_bin, walk) {
        return Ok(Stop::NotRunning);
    }

    // FROM HERE THE TOOL IS RUNNING, and every arm below refuses before the
    // first write. The working copy first, because it is a DURABLE fact the
    // records already carry and the shell is respawned into it: refusing on it
    // after the kill would be the worst order this verb could take.
    let work_dir = match crate::seat_relaunch::usable_work_dir(bytes) {
        Ok(work_dir) => work_dir,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(Stop::Refused);
        }
    };
    // NOT FROM UNDER THE SEAT ITSELF. The pane check upstream catches a caller
    // ae stamped; this catches the other half — a process the target's own
    // tool started, which carries no `$TMUX_PANE` of its own to compare. Both
    // halves need the pid and the table, so a gap in either REFUSES: ae will
    // not kill a tool it cannot prove is not its own parent.
    match caller_standing(probe.pid, table.as_deref(), std::process::id()) {
        CallerStanding::Clear => {}
        CallerStanding::Under => {
            writeln!(
                err,
                "Error: this command runs UNDER '{}' (pane {}) — stopping that tool would kill \
                 the process asking for the reseat. Run it from another seat, or from a plain \
                 shell.",
                target.agent, target.pane
            )?;
            return Ok(Stop::Refused);
        }
        CallerStanding::Unprovable => {
            writeln!(
                err,
                "Error: '{}' is running in pane {} and ae cannot prove this command is not \
                 running under it ({}) — nothing was stopped.",
                target.agent,
                target.pane,
                crate::seat_relaunch::unproven_gap(probe.pid, &agent_bin, table.is_some())
            )?;
            return Ok(Stop::Refused);
        }
    }
    // THE PANE'S SEND-LOCK, so no delivery can land a turn between the frame
    // ae reads and the kill it decides from. Held across the reading, the
    // respawn and the wait, and dropped with this scope — the seed turn later
    // takes no such lock and cannot deadlock against it.
    let Some(_send_lock) = crate::deliver::lock_target(dir, &target.pane, SEND_LOCK_WAIT) else {
        writeln!(
            err,
            "Error: a delivery holds pane {} of '{}' — nothing was stopped. Try again once it \
             finishes.",
            target.pane, target.agent
        )?;
        return Ok(Stop::Refused);
    };
    // THE FRAME. Only a POSITIVELY proven idle box may be stopped, and it is
    // proven TWICE: `Busy` short-circuits on the first reading, and a seat
    // that was handed work between the two readings is caught by the second.
    let tool = ToolKind::from_binary_name(&agent_bin);
    let busy = |err: &mut dyn Write| -> io::Result<()> {
        writeln!(
            err,
            "Error: '{}' is BUSY — a turn is running in pane {}. Wait for it, or \
             `{}/interrupt {}` first, then re-run the reseat.",
            target.agent,
            target.pane,
            dir.display(),
            target.agent
        )
    };
    let first = frame(target, tool);
    if first == HarnessState::Busy {
        busy(err)?;
        return Ok(Stop::Refused);
    }
    std::thread::sleep(FRAME_GAP);
    let second = frame(target, tool);
    if second == HarnessState::Busy {
        busy(err)?;
        return Ok(Stop::Refused);
    }
    let proven_idle = first == HarnessState::Idle && second == HarnessState::Idle;
    if !proven_idle && !stop_unknown {
        writeln!(
            err,
            "Error: ae cannot read '{}'s frame in pane {} of '{}', so it cannot tell a running \
             turn from an idle box — pass --stop-unknown to stop it anyway. Nothing was reseated.",
            agent_bin, target.pane, target.agent
        )?;
        return Ok(Stop::Refused);
    }

    // THE ONLY WRITE. A tmux that refuses the argv leaves the tool running,
    // and says so in its own words rather than the timeout's.
    let (respawned, _) = transport::run_tmux_op(&crate::session_tmux::argv(
        &target.server,
        &Op::RespawnPane {
            pane: &target.pane,
            work_dir: &work_dir,
            command: &[],
        },
    ));
    if !respawned {
        writeln!(
            err,
            "Error: tmux would not respawn pane {} of '{}' — the tool is still running and \
             nothing was reseated.",
            target.pane, target.agent
        )?;
        return Ok(Stop::Refused);
    }
    for _ in 0..STOP_POLLS {
        std::thread::sleep(STOP_POLL);
        let Some(probe) = transport::observe_pane_probe(&target.server, &target.pane) else {
            continue;
        };
        if pane_back_at_shell(&probe, procs::snapshot().as_deref()) {
            return Ok(Stop::Stopped(agent_bin));
        }
    }
    // PAST THE KILL and still not idle: say what holds the pane, because this
    // is the one refusal where the seat's tool has already been stopped. The
    // meta is untouched, so `relaunch` brings the seat back on the profile it
    // still records, and a second reseat is free to try again.
    let (holding, next) =
        stop_timeout_advice(transport::observe_pane_probe(&target.server, &target.pane).as_ref());
    writeln!(
        err,
        "Error: '{}' was stopped but pane {} is not back at an idle shell after {}s — {holding}. \
         Nothing was moved: '{}' still records its old profile, so {next}",
        target.agent,
        target.pane,
        (STOP_POLL * STOP_POLLS).as_secs(),
        target.agent
    )?;
    Ok(Stop::Refused)
}

/// `ae reseat <session> <agent> --using <profile> [--stop-unknown]`.
///
/// # Errors
///
/// Only a failure to write `out` or `err`. Every refusal is an exit code.
#[allow(clippy::too_many_lines, reason = "the frozen order, kept in one place")]
pub(crate) fn run(
    root: &Path,
    world: Option<&crate::listing::World>,
    tail: &[String],
    now: Timestamp,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let parsed = match parse(tail) {
        Ok(parsed) => parsed,
        Err(line) => {
            writeln!(err, "{line}")?;
            if line != RESEAT_USAGE {
                writeln!(err, "{RESEAT_USAGE}")?;
            }
            return Ok(EXIT_USAGE);
        }
    };
    // The seed is built from the world the caller already enumerated, so the
    // pack a successor is handed is the one `ae brief --seat` prints this
    // instant — and an incomplete enumeration refuses rather than packs a
    // session it only half saw.
    let Some(entry) = world.and_then(|world| {
        world
            .sessions
            .iter()
            .find(|entry| entry.name == parsed.session)
    }) else {
        writeln!(
            err,
            "Error: no session named '{}' that ae could read.",
            parsed.session
        )?;
        return Ok(EXIT_FAILED);
    };
    let dir = crate::inventory::Roots::under(root)
        .sessions()
        .join(&parsed.session);
    let caller = match caller_of(&parsed.session) {
        Ok(caller) => caller,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(EXIT_FAILED);
        }
    };
    // THE ROSTER FIRST, and tmux only after it. Everything a typo gets wrong —
    // the seat's name, the profile — is a DURABLE fact, and answering it from
    // the meta means a stopped session diagnoses the argv exactly as a running
    // one does instead of refusing for want of a pane.
    let bytes = crate::meta::read_bytes(&dir).unwrap_or_default();
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let Some(seated) = meta.roster().iter().find(|row| row.name == parsed.agent) else {
        let roster: Vec<&str> = meta.roster().iter().map(|row| row.name.as_str()).collect();
        writeln!(
            err,
            "Error: '{}' has no seat named '{}'; its roster is {}.",
            parsed.session,
            parsed.agent,
            if roster.is_empty() {
                "empty".to_owned()
            } else {
                roster.join(", ")
            }
        )?;
        return Ok(EXIT_FAILED);
    };
    let (slot, recorded) = (
        seated.slot.clone(),
        seated.profile.clone().unwrap_or_default(),
    );
    // What the seat recorded BEFORE the move, for the two messages that must
    // say what a half-finished move left behind. A seat with no profile row is
    // legal input: the move is what gives it one, and an empty string never
    // equals the non-empty profile the parse guarantees.
    let was = if recorded.is_empty() {
        "no profile".to_owned()
    } else {
        format!("profile '{recorded}'")
    };
    if recorded == parsed.profile {
        writeln!(
            err,
            "Error: '{}' already runs profile '{}' — there is nothing to move. `relaunch {}` \
             brings it back on the same profile.",
            parsed.agent, parsed.profile, parsed.agent
        )?;
        return Ok(EXIT_FAILED);
    }
    let moving = match resolve_profile(&dir, &bytes, &parsed.profile, &parsed.agent) {
        Ok(moving) => moving,
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was reseated.")?;
            return Ok(EXIT_FAILED);
        }
    };
    let target = match resolve(&dir, &parsed.agent, &parsed.session) {
        Ok(target) => target,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(EXIT_FAILED);
        }
    };
    // NOT YOUR OWN SEAT. The dead proof would refuse a live caller anyway, so
    // this closes no hole today — it states the rule instead of leaving it to
    // an accident of ordering, and it says the useful thing: the seat running
    // this command cannot be the seat whose tool is replaced under it.
    if caller.pane.as_deref() == Some(target.pane.as_str()) {
        writeln!(
            err,
            "Error: '{}' is THIS pane — a seat cannot reseat itself, because the tool running the \
             command is the one that would be replaced. Ask another seat, or run it from a shell.",
            target.agent
        )?;
        return Ok(EXIT_FAILED);
    }
    // The pane's own stamp against the roster. They disagree only when a stamp
    // is stale or a meta was hand-edited, and the move would then be published
    // for one seat and pasted into another's pane.
    if target.slot != slot {
        writeln!(
            err,
            "Error: pane {} is stamped slot {} but '{}' seats '{}' at {slot} — fix the meta, or \
             stop and resume the session.",
            target.pane, target.slot, parsed.session, parsed.agent
        )?;
        return Ok(EXIT_FAILED);
    }

    // THE LOCK, taken the way every lifecycle writer takes it: the dead proof,
    // the seed, the slot cleanup, the meta move and the paste are ONE
    // transaction, so no stop, compact, relaunch or second reseat interleaves.
    let Ok(lifecycle) = crate::lifecycle::lock(root, &parsed.session) else {
        writeln!(
            err,
            "Error: another lifecycle operation holds '{}' — it waited and gave up. Try again \
             once that finishes.",
            parsed.session
        )?;
        return Ok(EXIT_FAILED);
    };
    // What the records say NOW, under the lock: the stop reads the working copy
    // from it, and the audit line reads the conversation the predecessor holds.
    let locked = crate::meta::read_bytes(&dir).unwrap_or_default();
    let prior = crate::lifecycle::meta_value(&locked, &format!("harness_session.{}", target.slot));
    let mut at = Record {
        caller: &caller,
        now,
        from: &recorded,
        to: &parsed.profile,
        prior: &prior,
        // Filled in once the carry question is answered, below. The STOP record
        // is written before that and carries no word, which is right: a stop
        // says nothing about where the conversation went.
        carry: String::new(),
    };
    // THE STOP, before the dead proof and before anything durable is written.
    match stop_running_tool(&dir, &target, &locked, parsed.stop_unknown, err)? {
        Stop::Refused => return Ok(EXIT_FAILED),
        // The record goes in only once the pane is PROVEN back at its shell:
        // an audit line claiming a stop that did not finish would outlive
        // every refusal above it.
        Stop::Stopped(binary) => record(
            &dir,
            &at,
            &target,
            &format!("stopped {binary} in place (pane {})", target.pane),
        ),
        Stop::NotRunning => {}
    }
    let Some(before) = crate::seat_relaunch::prove_dead(&dir, &target, RESEAT_VERB, err)? else {
        return Ok(EXIT_FAILED);
    };
    // DOES THE CONVERSATION TRAVEL? Asked here, past the dead proof and before
    // anything is removed, so a refusal costs only the reading. Every arm that
    // does not carry leaves the rest of this function exactly as it was.
    let plan = carry_plan(&locked, &target.slot, &moving, &prior, &before.work_dir);
    let marker = crate::run::started_marker(&dir, &target.slot);
    // THE RESUME MARKER, PROVEN WRITABLE BEFORE ANYTHING MOVES. `clear_slot`
    // below removes it and a carried seat has to have it back, or `_run` would
    // open a NEW conversation beside the store this is about to copy. The two
    // files cannot be one write, so the failure is taken HERE instead: nothing
    // has been removed, nothing copied, and the seat is exactly what it was.
    // Past this point only a crash can separate the removal from the rewrite,
    // and re-running the same reseat converges — the source store is untouched
    // and the seat still records the conversation.
    if plan.is_some()
        && let Err(why) = crate::launch::publish_data(&marker, b"")
    {
        writeln!(
            err,
            "Error: {why} — nothing was reseated and '{}' still records {was}.",
            target.agent
        )?;
        return Ok(EXIT_FAILED);
    }
    let planned = plan.is_some();
    let carried = match plan {
        None => Carried::No,
        Some(plan) => {
            let (from, to) = plan.homes();
            match crate::carry::run(&plan) {
                Ok(crossing) => {
                    // RULING 1: typing the reseat with the other account's
                    // profile IS the consent, so there is no prompt — but the
                    // crossing is never silent. Said HERE rather than with the
                    // final line because it is true from this moment: the
                    // bytes are in the other account whatever the steps below
                    // decide, and the source is untouched either way.
                    writeln!(
                        out,
                        "carried conversation {} from {} to {}{}",
                        plan.id(),
                        from.display(),
                        to.display(),
                        if crossing.memory_kept {
                            " (the target already had a project memory; it was kept, never merged)"
                        } else {
                            ""
                        }
                    )?;
                    Carried::Yes
                }
                Err(why) => {
                    // RULING 4: the move still happens, on a fresh conversation
                    // with the seed pack, and what failed is named.
                    writeln!(
                        err,
                        "note: '{}' could not carry conversation {} from {} to {} ({why}) — \
                         moving it on a fresh conversation with its seed pack instead.",
                        target.agent,
                        plan.id(),
                        from.display(),
                        to.display()
                    )?;
                    Carried::Seeded(why)
                }
            }
        }
    };
    at.carry = carried.word();
    // BUILT FIRST, because the pack reads the seat's recorded first message and
    // the cleanup below removes that file. A CARRIED seat gets neither: the
    // conversation itself travelled, so there is nothing for ae to tell the
    // successor and nothing for a human to re-send by hand.
    let seed = if carried == Carried::Yes {
        None
    } else {
        let pack = match crate::seat_pack(
            entry,
            &dir,
            crate::doors::home().as_deref(),
            now,
            &target.agent,
        ) {
            Ok(pack) => crate::provenance::first_line(&crate::provenance::ctx(), &pack),
            Err(why) => {
                writeln!(err, "Error: {why}. Nothing was reseated.")?;
                return Ok(EXIT_FAILED);
            }
        };
        if let Err(why) =
            crate::launch::publish_data(&seed_file(&dir, &target.agent), pack.as_bytes())
        {
            writeln!(err, "Error: {why}. Nothing was reseated.")?;
            return Ok(EXIT_FAILED);
        }
        Some(pack)
    };
    if let Err(why) = crate::run::clear_slot(&dir, &target.slot) {
        // THE MARKER, BACK. `clear_slot` removes it FIRST and can fail on a
        // later file, and the meta has NOT moved — so the seat is still its old
        // self and still needs the marker its old self had. Only where a carry
        // was planned, because that is the one path where ae has just PROVEN
        // the marker was there; a seat that never started must not be handed
        // one it never had.
        let stranded = planned && crate::launch::publish_data(&marker, b"").is_err();
        let note = match (stranded, carried == Carried::Yes) {
            (true, _) => {
                " Its start marker could not be put back either, so its next start would open a \
                 NEW conversation beside the one it records."
            }
            // SAID, because a carry has already written in the other account
            // and "nothing was touched" would not be true of it.
            (false, true) => {
                " Its start marker is back; the copy already in the other account is inert, since \
                 no record names it."
            }
            (false, false) => "",
        };
        writeln!(
            err,
            "Error: the launch files of slot {} could not be cleared ({why}) — the seat was not \
             moved and '{}' still records {was}.{note}",
            target.slot, target.agent
        )?;
        return Ok(EXIT_FAILED);
    }
    // THE RESUME MARKER, PUT BACK. `clear_slot` removes it with the rest of the
    // slot's launch files, and `_run` reads exactly that file to decide between
    // creating a conversation and resuming one: without this the store would be
    // copied and the successor would then open a BRAND NEW conversation beside
    // it, which is the whole failure this slice exists to prevent. The write was
    // already proven possible before anything moved, so a failure here is a
    // transient one and is tried once more; a second failure REFUSES with the
    // meta unmoved, which is the seat's old self — it still records its old
    // profile, both stores are untouched, and re-running the same reseat puts
    // the marker back and carries again.
    if carried == Carried::Yes {
        let mut back = crate::launch::publish_data(&marker, b"");
        if back.is_err() {
            back = crate::launch::publish_data(&marker, b"");
        }
        if let Err(why) = back {
            writeln!(
                err,
                "Error: {why} — the conversation was copied but slot {} could not be marked as \
                 resuming, so the seat was not moved and '{}' still records {was}. Re-run the \
                 same reseat once {} can be written; until then relaunching '{}' would start a \
                 NEW conversation on its old account. The source account still holds the \
                 conversation; the copy in the other account is inert, since no record names it.",
                target.slot,
                target.agent,
                marker.display(),
                target.agent
            )?;
            return Ok(EXIT_FAILED);
        }
    }
    let conversation = if crate::launch::takes_launch_session_id(moving.tool) {
        crate::launch::generate_uuid()
    } else {
        crate::launch::PENDING.to_owned()
    };
    let moved = crate::meta::publish_seat_move(
        &dir,
        &target.slot,
        &target.agent,
        &crate::meta::SeatMove {
            profile: &parsed.profile,
            binary: &moving.binary,
            launch_id: &crate::session_launch::launch_token(moving.tool, None),
            conversation: match (carried == Carried::Yes, moving.account.as_ref()) {
                // The account rows and the kept conversation are published in
                // the SAME replacement: a conversation whose store no row names
                // is the window a later reader would resolve to the home the
                // seat just left.
                (true, Some(account)) => crate::meta::Conversation::Carried {
                    config_home: &account.row,
                    config_home_base: account.base.as_deref(),
                },
                // A carry proves an account, so the second arm is unreachable;
                // it is spelled rather than unwrapped, and it is today's move.
                _ => crate::meta::Conversation::Fresh {
                    id: &conversation,
                    capture_floor: now.epoch(),
                },
            },
        },
    );
    if let Err(why) = moved {
        writeln!(
            err,
            "Error: the seat move could not be recorded ({}) — slot {} has no launch files now \
             and still records {was}. Re-run the reseat.",
            why.cause(),
            target.slot
        )?;
        return Ok(EXIT_FAILED);
    }
    // Re-read: the command, the tool and the conversation are the NEW profile's
    // now, and `start` pastes what `read_seat` composes from the moved rows.
    let after = match crate::run::read_seat(&dir, &target.slot, None) {
        Ok(seat) => Proven {
            seat,
            agent_bin: moving.binary.clone(),
            work_dir: before.work_dir,
            id_before: if carried == Carried::Yes {
                prior.clone()
            } else {
                conversation
            },
        },
        Err(why) => {
            writeln!(
                err,
                "Error: {why}. '{}' already records profile '{}' and its pane is at a shell, so \
                 `relaunch {}` finishes the move once the profile reads.",
                target.agent, parsed.profile, target.agent
            )?;
            return Ok(EXIT_FAILED);
        }
    };
    let started = crate::seat_relaunch::start(&dir, &target, &after, now, RESEAT_VERB, err)?;
    // PAST THE LOCK before any readiness wait: a gated turn blocks up to 45s,
    // and nothing else may be held out of the session's lifecycle for that.
    drop(lifecycle);
    match started {
        // Nothing reached the pane and the refusing site already said why. The
        // meta HAS moved, so the next step is named rather than left implicit.
        Started::NotPasted => {
            writeln!(
                err,
                "  '{}' records profile '{}' now — `relaunch {}` starts it.",
                target.agent, parsed.profile, target.agent
            )?;
            Ok(EXIT_FAILED)
        }
        Started::NotSeen => {
            record(&dir, &at, &target, "pasted, tool not seen");
            writeln!(
                err,
                "Error: '{}' moved to '{}' and its tool was not seen: look at the pane; \
                 `relaunch {}` finishes the move once it is back at a shell.",
                target.agent, parsed.profile, target.agent
            )?;
            Ok(EXIT_FAILED)
        }
        Started::Running { resuming_seat } => finish(
            &dir,
            &target,
            &after,
            resuming_seat,
            &parsed,
            seed.as_deref(),
            &at,
            out,
            err,
        ),
    }
}

/// Everything past the lock: the tool's own launch turn, the seed turn, the
/// post-launch capture, the last identity check and the exit rule.
#[allow(
    clippy::too_many_arguments,
    reason = "the frozen helper's inputs, spelled out rather than bundled"
)]
fn finish(
    dir: &Path,
    target: &Target,
    proven: &Proven,
    resuming_seat: bool,
    parsed: &Parsed,
    seed: Option<&str>,
    at: &Record<'_>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let tool = proven.seat.tool;
    // The tool's OWN launch turn first, where its adapter has one: codex's
    // rollout does not exist until a user turn, and the registration handshake
    // it carries must not be buried under a 24 KB seed.
    let prompt = crate::launch::initial_prompt_for(tool, dir, &target.slot);
    let launch_turn = if prompt.is_empty()
        || !crate::session_launch::launch_turn_is_pasted(tool, resuming_seat)
    {
        None
    } else {
        Some(crate::session_launch::deliver_launch_turn(
            dir,
            &target.server,
            &target.slot,
            &target.pane,
            tool,
            &prompt,
            err,
        )?)
    };
    // THE SEED, as ae's own setup turn: `deliver_launch_turn` pastes its text
    // verbatim, so the marker is put on here by the ONE owner that spells it.
    // A CARRIED seat is handed none: it resumed the conversation itself, and a
    // pack recounting what it already remembers would be noise on its first
    // turn.
    let seed_turn = match seed {
        Some(seed) => Some(crate::session_launch::deliver_launch_turn(
            dir,
            &target.server,
            &target.slot,
            &target.pane,
            tool,
            seed,
            err,
        )?),
        None => None,
    };
    // Only a slot that reads `pending` NOW needs capturing: `_run`'s own
    // in-pane fallback may have cleared it after the move.
    let id_after = crate::lifecycle::meta_value(
        &crate::meta::read_bytes(dir).unwrap_or_default(),
        &format!("harness_session.{}", target.slot),
    );
    if tool.adapter().capture.is_needed() && id_after == crate::launch::PENDING {
        crate::session_launch::capture::start(
            dir,
            &[crate::session_launch::capture::Target {
                slot: target.slot.clone(),
                tool,
                pane: target.pane.clone(),
            }],
        );
    }
    // LAST: the new tool may have died while the turns were delivered, and
    // exit 0 means the seat is up NOW.
    if !crate::seat_relaunch::observe_identity(target, &proven.agent_bin, true) {
        record(dir, at, target, "tool stopped after the reseat");
        writeln!(
            err,
            "Error: '{}' started on '{}' and is gone again (pane {}) — look at the pane.",
            target.agent, parsed.profile, target.pane
        )?;
        return Ok(EXIT_FAILED);
    }
    let (launch_ok, launch_word) =
        launch_turn.map_or((true, ""), crate::seat_relaunch::turn_verdict);
    // A seat that was handed no seed has no seed verdict to report, and says
    // what it DID instead: the conversation came with it.
    let (seed_ok, seed_note) = seed_turn.map_or((true, ", carried".to_owned()), |turn| {
        let (ok, word) = crate::seat_relaunch::turn_verdict(turn);
        (ok, format!(", seed {word}"))
    });
    let launch_note = if launch_word.is_empty() {
        String::new()
    } else {
        format!(", launch turn {launch_word}")
    };
    let line = format!(
        "reseated {} (pane {}, slot {}) to {}{launch_note}{seed_note}",
        target.agent, target.pane, target.slot, parsed.profile
    );
    record(dir, at, target, &line);
    if !launch_ok || !seed_ok {
        writeln!(err, "Error: {line} — a turn never landed.")?;
        if seed.is_some() {
            writeln!(
                err,
                "  the seat is up; send its seed by hand: {}/send {} \"$(cat {})\"",
                dir.display(),
                target.agent,
                seed_file(dir, &target.agent).display()
            )?;
        }
        return Ok(EXIT_FAILED);
    }
    writeln!(out, "{line}")?;
    Ok(0)
}

/// What every `reseat` record carries besides its outcome: WHO asked, and the
/// move itself.
///
/// The move is on the record because the meta only keeps the CURRENT profile:
/// a later reader asking what this seat used to run has the predecessor's id
/// in the roster but nothing that names the profile it ran under, and the
/// event is the only place that pairing can live.
struct Record<'a> {
    caller: &'a Caller,
    now: Timestamp,
    from: &'a str,
    to: &'a str,
    prior: &'a str,
    /// What became of the conversation — EMPTY when the move was never a carry
    /// question, so every record a tool change writes stays byte-identical to
    /// what it wrote before this existed. OWNED, because the verdict is only
    /// reached after the stop has already written its own record through this.
    carry: String,
}

/// One `reseat` record for every attempt that REACHED the pane — the seat's
/// history is the only place a later reader can see that its TOOL changed, and
/// the only place that must not claim a paste that never was.
/// The event's own sentence. A row ae cannot prove reads `none`, and that
/// includes [`crate::launch::PENDING`]: it is the word for a conversation that
/// never resolved, `reseated` hands on no prior row for it, and an audit line
/// must not read as if one were being left behind.
fn summary(outcome: &str, from: &str, to: &str, prior: &str, carry: &str) -> String {
    let named = |value: &str| {
        if value.is_empty() || value == crate::launch::PENDING {
            "none".to_owned()
        } else {
            value.to_owned()
        }
    };
    format!(
        "{outcome} [from {} to {to}, prior {}{}]",
        named(from),
        named(prior),
        if carry.is_empty() {
            String::new()
        } else {
            format!(", {carry}")
        }
    )
}

fn record(dir: &Path, at: &Record<'_>, target: &Target, outcome: &str) {
    let summary = summary(outcome, at.from, at.to, at.prior, &at.carry);
    let _ = crate::store::open(dir).append_event(&tracked::event_line(&EventFields {
        ts: at.now,
        // The caller's own display ref, exactly as `relaunch` records it: an
        // agent that moves a peer's seat must not appear in the audit trail as
        // the human, and a plain shell IS the human.
        actor: if at.caller.display.is_empty() {
            "human"
        } else {
            &at.caller.display
        },
        action: RESEAT_ACTION,
        target: &target.agent,
        reference: "",
        actor_slot: "",
        actor_session: "",
        target_slot: &target.slot,
        target_session: "",
        target_server: "",
        target_pane: &target.pane,
        target_session_uuid: "",
        caller_server: "",
        caller_pane: at.caller.pane.as_deref().unwrap_or_default(),
        caller_session_uuid: "",
        identity_gap: "",
        summary: &summary,
        body_file: "",
    }));
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    fn parsed(words: &[&str]) -> Result<(String, String, String), String> {
        super::parse(&argv(words)).map(|parsed| (parsed.session, parsed.agent, parsed.profile))
    }

    #[test]
    fn the_stop_flag_sits_anywhere_and_is_off_unless_it_is_typed() {
        // `--using`'s own rule, for the same reason: this is where a hand puts
        // a flag, and a seat must never be stopped by a spelling accident.
        for words in [
            &["work", "lead", "--using", "lunam", "--stop-unknown"][..],
            &["work", "--stop-unknown", "lead", "--using", "lunam"][..],
            &["--stop-unknown", "work", "lead", "--using", "lunam"][..],
        ] {
            let parsed = super::parse(&argv(words)).expect("a valid argv");
            assert!(parsed.stop_unknown, "{words:?}");
            assert_eq!(parsed.agent, "lead", "{words:?}");
        }
        let plain = super::parse(&argv(&["work", "lead", "--using", "lunam"])).expect("valid");
        assert!(!plain.stop_unknown, "the flag is opt-in");
        // And it is not a name: a seat called `--stop-unknown` cannot exist,
        // and the ladder must not swallow the word as one.
        assert_eq!(
            super::parse(&argv(&["work", "--stop-unknown", "--using", "lunam"]))
                .err()
                .as_deref(),
            Some(super::RESEAT_USAGE)
        );
    }

    #[test]
    fn a_caller_the_stop_cannot_place_is_never_cleared_to_kill() {
        use super::CallerStanding;
        use crate::procs::Proc;
        // Parentage is the whole question, so one helper builds the rows.
        let row = |pid, ppid| Proc {
            pid,
            ppid,
            comm: "x".to_owned(),
        };
        let rows = vec![row(10, 1), row(20, 10), row(30, 20), row(40, 1)];
        // The command runs under the seat's own tool: ending it would kill the
        // process asking.
        assert_eq!(
            super::caller_standing(Some(10), Some(&rows), 30),
            CallerStanding::Under
        );
        assert_eq!(
            super::caller_standing(Some(10), Some(&rows), 40),
            CallerStanding::Clear
        );
        // Either gap answers the same way, because ae may not end a tool it
        // cannot prove is not its own parent.
        assert_eq!(
            super::caller_standing(None, Some(&rows), 40),
            CallerStanding::Unprovable
        );
        assert_eq!(
            super::caller_standing(Some(10), None, 40),
            CallerStanding::Unprovable
        );
    }

    #[test]
    fn a_timed_out_stop_tells_a_human_which_of_the_two_states_the_pane_is_in() {
        use crate::tmux::ObservedPaneProbe;
        let probe = |command: &str| ObservedPaneProbe {
            command: command.to_owned(),
            pid: Some(10),
        };
        // The respawn took and the shell is busy: waiting is what helps.
        let (holding, next) = super::stop_timeout_advice(Some(&probe("zsh")));
        assert!(holding.contains("under it"), "{holding}");
        assert!(next.contains("re-run"), "{next}");
        // Something else came back in the pane. Re-running cannot change that,
        // so the advice must not send a human round the same loop.
        let (holding, next) = super::stop_timeout_advice(Some(&probe("sleep")));
        assert!(holding.contains("'sleep'"), "{holding}");
        assert!(next.contains("will not help"), "{next}");
        let (holding, next) = super::stop_timeout_advice(None);
        assert!(holding.contains("stopped answering"), "{holding}");
        assert!(next.contains("look at the pane"), "{next}");
    }

    #[test]
    fn a_pane_is_back_at_a_shell_only_when_both_readings_prove_it() {
        use crate::procs::Proc;
        use crate::tmux::ObservedPaneProbe;
        let shell = |pid| ObservedPaneProbe {
            command: "zsh".to_owned(),
            pid,
        };
        let empty: Vec<Proc> = Vec::new();
        assert!(super::pane_back_at_shell(&shell(Some(10)), Some(&empty)));
        // A process under the shell is not an idle shell: the seat's tool may
        // be dying, or a human may have started something.
        let busy = vec![Proc {
            pid: 20,
            ppid: 10,
            comm: "vim".to_owned(),
        }];
        assert!(!super::pane_back_at_shell(&shell(Some(10)), Some(&busy)));
        // The tool still holds the foreground.
        assert!(!super::pane_back_at_shell(
            &ObservedPaneProbe {
                command: "codex".to_owned(),
                pid: Some(10),
            },
            Some(&empty)
        ));
        // EVERY gap is false, so the bounded wait times out and refuses rather
        // than calling a pane it could not read idle.
        assert!(!super::pane_back_at_shell(&shell(None), Some(&empty)));
        assert!(!super::pane_back_at_shell(&shell(Some(10)), None));
    }

    #[test]
    fn a_frame_grammar_the_classifier_does_not_own_is_never_read_as_idle() {
        use crate::harness_state::{HarnessState, classify};
        use crate::tool::ToolKind;
        // Muse's adapter declares the BorderDelimited input model, which
        // routes it to CLAUDE's frame grammar — a grammar muse does not draw.
        // The stop therefore may never take a muse frame for an idle claude
        // one. Measured muse capture, plain, 80x24, its composer genuinely
        // idle: the row above the prompt is muse's own `── Voice input …`
        // rule, which is not a claude border, so the frame fails closed.
        let idle_muse =
            include_str!("../tests/fixtures/runtime-identity/muse-idle-plain-80x24.txt");
        assert_eq!(classify(idle_muse, ToolKind::Muse), HarnessState::Unknown);
        // An unmodelled tool has no grammar at all, and an empty capture is
        // the shape a failed read takes.
        assert_eq!(
            classify(idle_muse, ToolKind::OpenCode),
            HarnessState::Unknown
        );
        assert_eq!(classify("", ToolKind::Claude), HarnessState::Unknown);
    }

    #[test]
    fn a_conversation_that_never_resolved_is_nothing_to_hand_on() {
        // The seat moved, and what it leaves behind is a word rather than an
        // id. The meta is already right — `reseated` writes no prior row it
        // cannot prove — and the audit line has to say the same thing.
        assert_eq!(
            super::summary("reseated", "a", "b", crate::launch::PENDING, ""),
            "reseated [from a to b, prior none]"
        );
        assert_eq!(
            super::summary(
                "reseated",
                "",
                "b",
                "11111111-1111-4111-8111-111111111111",
                ""
            ),
            "reseated [from none to b, prior 11111111-1111-4111-8111-111111111111]"
        );
        // The carry word is the ONLY thing that changes, and only when the move
        // was a carry question at all: every tool change records what it always
        // recorded, byte for byte.
        assert_eq!(
            super::summary("reseated", "a", "b", crate::launch::PENDING, "carried"),
            "reseated [from a to b, prior none, carried]"
        );
        assert_eq!(
            super::Carried::Seeded("no transcript".to_owned()).word(),
            "seeded (no transcript)"
        );
        assert!(super::Carried::No.word().is_empty());
    }

    #[test]
    fn the_flag_sits_anywhere_and_every_other_dash_word_is_refused() {
        let want = Ok(("work".to_owned(), "lead".to_owned(), "lunam".to_owned()));
        // Where a hand puts it: after the names, between them, in front, and
        // in the `=` spelling a shell completion produces.
        for words in [
            &["work", "lead", "--using", "lunam"][..],
            &["work", "--using", "lunam", "lead"][..],
            &["--using", "lunam", "work", "lead"][..],
            &["work", "lead", "--using=lunam"][..],
        ] {
            assert_eq!(parsed(words), want, "{words:?}");
        }
        // A dash word is never a name: swallowing one would reseat a seat the
        // caller did not type.
        assert_eq!(
            parsed(&["work", "-f", "lead", "--using", "lunam"]),
            Err("Error: unknown flag '-f'.".to_owned())
        );
    }

    #[test]
    fn an_incomplete_argv_names_what_is_missing() {
        assert_eq!(parsed(&[]), Err(super::RESEAT_USAGE.to_owned()));
        assert_eq!(parsed(&["work"]), Err(super::RESEAT_USAGE.to_owned()));
        assert_eq!(
            parsed(&["work", "lead", "extra", "--using", "lunam"]),
            Err(super::RESEAT_USAGE.to_owned())
        );
        assert_eq!(
            parsed(&["work", "lead"]),
            Err("Error: reseat needs --using <profile>.".to_owned())
        );
        assert_eq!(
            parsed(&["work", "lead", "--using"]),
            Err("Error: --using needs a profile name.".to_owned())
        );
    }

    #[test]
    fn a_client_override_is_refused_because_this_verb_writes_no_client_row() {
        // The launch grammar `profile@client` pins a seat to one [clients]
        // label. A reseat REMOVES that row, so accepting the spelling here
        // would silently drop the half the caller typed.
        let refusal = parsed(&["work", "lead", "--using", "lunam@cc-mic"])
            .expect_err("a client override is refused");
        assert!(refusal.contains("launch-only"), "{refusal}");
        assert!(refusal.contains("lunam@cc-mic"), "{refusal}");
    }

    #[test]
    fn the_seed_is_published_beside_the_meta_under_the_seats_own_name() {
        // Named for the SEAT, not the slot: it is what a human `cat`s when a
        // turn did not land, and the refusal that says so prints this path.
        assert_eq!(
            super::seed_file(Path::new("/s/work"), "colead"),
            Path::new("/s/work/seed.colead.md")
        );
        // A name that is not a filename component cannot escape the session
        // directory, whatever put it in the meta or the pane option. The dot
        // SURVIVES, because a slot is `spawned.0` and the sanitiser is shared;
        // what cannot survive is the separator, so `..` is inert here.
        let escaped = super::seed_file(Path::new("/s/work"), "../../etc/x");
        assert_eq!(escaped, Path::new("/s/work/seed..._.._etc_x.md"));
        assert_eq!(escaped.parent(), Some(Path::new("/s/work")));
    }
}
