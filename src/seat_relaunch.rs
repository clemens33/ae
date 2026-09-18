//! `relaunch <agent>`: bring ONE provably dead seat back, in place.
//!
//! The pain this answers, measured 2026-09-18: a fixed seat's tool exited and
//! its pane sat at its shell. Nothing brought that ONE seat back. A fixed seat
//! cannot be retired, so its name stayed bound to a dead pane and the only
//! recovery was a human re-running the pane's launch line by hand.
//!
//! This is the single-slot half of a launch, by REUSE and never by fork: the
//! same `.launch-attempt` stamp, the same capture-floor rule, the same
//! `_run` line, the same launch-turn delivery. What it adds is PROOF. A launch
//! creates the pane it pastes into and so knows it is idle; this pastes into a
//! pane that already existed, so it must first prove the seat dead — and it
//! REFUSES, by name, whenever it cannot.
//!
//! The order is the contract:
//!
//! 1. the target resolves, and it must be a seat of the CALLER'S OWN session;
//! 2. under the session's lifecycle lock, the pane is proven to be a seat's
//!    pane sitting at an IDLE shell with the recorded tool gone;
//! 3. the shell's input line is cleared and the launch line pasted;
//! 4. the seat's OWN TOOL is observed running — not merely "something is",
//!    because `_run` itself in the foreground is not a started agent;
//! 5. only then does the launch turn go out, past the lock.
//!
//! Every refusal exits 1 with the reason and the next step, and nothing is
//! rolled back: a relaunch that pasted but never saw the tool leaves the pane
//! showing why, and a second relaunch refuses `running` if it came up after all.

use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use crate::deliver;
use crate::inventory::ServerId;
use crate::procs::{self, Descendancy};
use crate::session_launch::TurnOutcome;
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;
use crate::tmux::ObservedPaneProbe;
use crate::tracked::{self, EventFields};
use crate::transport;

/// The usage line.
pub const RELAUNCH_USAGE: &str = "Usage: relaunch <agent>";

/// The event action a relaunch records, whatever its outcome.
const RELAUNCH_ACTION: &str = "relaunch";

/// How many polls the tool-identity observation takes after the paste.
const IDENTITY_POLLS: u32 = 75;

/// The pause between those polls — 75 x 200ms is the 15s bound the docs state.
const IDENTITY_POLL: Duration = Duration::from_millis(200);

/// How often the identity poll spends a process-table snapshot. The pane's
/// foreground is a cheap tmux read; `ps` is not, and a tool whose launcher
/// keeps the foreground is only found by the walk.
const WALK_EVERY: u32 = 5;

// ---- the pure decisions ---------------------------------------------------

/// Is the SEAT'S OWN TOOL running in this pane?
///
/// Two arms, because one is not enough for every tool class: a tool that takes
/// the foreground is seen there, and a tool behind a launcher (node, bun) is
/// seen only as a descendant — which is exactly what the watchdog's latch
/// release already measures. `_run` in the foreground satisfies NEITHER, and
/// that is the point: the core composing the command, or refusing and
/// printing, is not a started agent.
#[must_use]
pub(crate) fn identity_proven(foreground: &str, agent_bin: &str, walk: Descendancy) -> bool {
    !agent_bin.is_empty()
        && (procs::name_matches(foreground, agent_bin) || matches!(walk, Descendancy::Present))
}

/// What a launch turn's outcome means for the caller: whether the relaunch
/// SUCCEEDED, and the word the line prints.
///
/// `Unconfirmed` is `send`'s own wording and reads as exit 0: the turn was
/// pasted and Enter pressed, and ae will not claim more than it proved. Only
/// text that never landed fails the relaunch.
#[must_use]
pub(crate) const fn turn_verdict(outcome: TurnOutcome) -> (bool, &'static str) {
    match outcome {
        TurnOutcome::Submitted => (true, "submitted"),
        TurnOutcome::Unconfirmed => (true, "unconfirmed"),
        TurnOutcome::Undelivered => (false, "undelivered"),
    }
}

/// Which conversation the seat came back on, read from the meta AFTER `_run`
/// decided: the recorded id survived its store probe, or the seat took the
/// fresh fallback.
#[must_use]
pub(crate) fn conversation_word(before: &str, after: &str) -> &'static str {
    if crate::launch::id_probeable(before) && before == after {
        "exact"
    } else {
        "fresh"
    }
}

/// WHICH gap left the seat unproven — never a bare "cannot tell".
///
/// The liveness owner answers Alive/Dead/Unproven and deliberately carries no
/// reason; the reason is these same readings, said out loud, so a human is told
/// what to fix rather than that ae gave up.
#[must_use]
pub(crate) fn unproven_gap(pid: Option<u32>, agent_bin: &str, table_read: bool) -> String {
    if pid.is_none() {
        return "tmux reported no pid for the pane".to_owned();
    }
    if !table_read {
        return "the process table could not be read".to_owned();
    }
    format!(
        "the recorded tool is '{agent_bin}', which ae reads as a shell — it cannot be told apart \
         from the pane's own shell"
    )
}

// ---- the operation --------------------------------------------------------

/// WHICH verb a refusal names.
///
/// The dead proof and the paste are ONE owner serving two commands: `relaunch`
/// brings a seat back on its own profile, `reseat` moves it to another. Their
/// refusals are the same refusals and must stay so — what differs is the word
/// a human is told to re-run, so the word is a parameter and not a second copy
/// of the ladder.
#[derive(Clone, Copy)]
pub(crate) struct Verb {
    /// The imperative, as a human types it: `relaunch`, `reseat`.
    imperative: &'static str,
    /// Its past participle, for a sentence about what did not happen.
    past: &'static str,
}

/// `relaunch`'s own word.
pub(crate) const RELAUNCH_VERB: Verb = Verb {
    imperative: "relaunch",
    past: "relaunched",
};

/// `reseat`'s.
pub(crate) const RESEAT_VERB: Verb = Verb {
    imperative: "reseat",
    past: "reseated",
};

/// Who asked and when — the two facts every record carries, together so the
/// operation's own signatures stay about the SEAT.
#[derive(Clone, Copy)]
struct Actor<'a> {
    caller: &'a str,
    now: Timestamp,
}

/// The facts one relaunch works from, gathered before the lock.
pub(crate) struct Target {
    pub(crate) agent: String,
    pub(crate) slot: String,
    pub(crate) pane: String,
    pub(crate) server: ServerId,
}

/// `_relaunch <meta-dir> <agent>`.
///
/// # Errors
///
/// Only a failure to write `out` or `err`. Every refusal is an exit code.
#[allow(clippy::too_many_lines, reason = "the frozen order, kept in one place")]
#[allow(
    clippy::too_many_arguments,
    reason = "the frozen helper's inputs, spelled out rather than bundled"
)]
pub(crate) fn run(
    root: &Path,
    dir: &Path,
    tail: &[String],
    caller: &str,
    own_session: &str,
    now: Timestamp,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let [name] = tail else {
        writeln!(err, "{RELAUNCH_USAGE}")?;
        writeln!(err, "  Example: relaunch colead")?;
        return Ok(EXIT_USAGE);
    };
    let actor = Actor { caller, now };
    let target = match resolve(dir, name, own_session) {
        Ok(target) => target,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(EXIT_FAILED);
        }
    };
    // THE LOCK, taken the way every lifecycle writer takes it: the dead proof,
    // the stamp, the paste and the tool-start observation are one transaction,
    // so a second relaunch, a stop or a compact cannot interleave with them.
    let Ok(lifecycle) = crate::lifecycle::lock(root, own_session) else {
        writeln!(
            err,
            "Error: another lifecycle operation holds '{own_session}' — it waited and gave up. Try again once that finishes."
        )?;
        return Ok(EXIT_FAILED);
    };
    let Some(seat) = prove_dead(dir, &target, RELAUNCH_VERB, err)? else {
        return Ok(EXIT_FAILED);
    };
    let outcome = start(dir, &target, &seat, now, RELAUNCH_VERB, err)?;
    // PAST THE LOCK before any readiness wait: a gated turn blocks up to 45s,
    // and nothing else may be held out of the session's lifecycle for that.
    drop(lifecycle);
    match outcome {
        // Nothing reached the pane and the refusing site already printed its
        // reason: no second line contradicting it, and no event claiming a
        // paste. A record here would tell a later reader the seat was touched.
        Started::NotPasted => Ok(EXIT_FAILED),
        Started::NotSeen => {
            record(dir, actor, &target, "pasted, tool not seen");
            writeln!(
                err,
                "Error: '{}' line pasted, tool not seen yet: look at the pane; a second relaunch refuses `running` if it came up.",
                target.agent
            )?;
            Ok(EXIT_FAILED)
        }
        Started::Running { resuming_seat } => {
            finish(dir, &target, &seat, resuming_seat, actor, out, err)
        }
    }
}

/// Resolve `name` to a seat of the CALLER'S OWN session.
fn resolve(dir: &Path, name: &str, own_session: &str) -> Result<Target, String> {
    let (resolved, server) = tracked::resolve_on(name, own_session, dir)
        .map_err(|why| tracked::ResolveError::message(&why))?;
    // An EMPTY session is not another session: a raw `%pane` of a pane that is
    // gone, or of one ae never stamped, resolves with no stamps at all. Those
    // are the ladder's to name, and naming them "cross-session" would be false.
    if !resolved.session.is_empty() && resolved.session != own_session {
        return Err(format!(
            "Error: '{name}' is on session '{}', not '{own_session}'. relaunch works on the caller's own session only — run it from that session.",
            resolved.session
        ));
    }
    Ok(Target {
        agent: if resolved.agent.is_empty() {
            name.to_owned()
        } else {
            resolved.agent
        },
        slot: resolved.slot,
        pane: resolved.pane,
        server,
    })
}

/// What the dead proof yields when it passes: everything the paste needs.
pub(crate) struct Proven {
    pub(crate) seat: crate::run::Seat,
    pub(crate) agent_bin: String,
    pub(crate) work_dir: String,
    pub(crate) id_before: String,
}

/// THE PROOF, under the lock. `Ok(None)` means a refusal was printed.
#[allow(
    clippy::too_many_lines,
    reason = "the refusal ladder, kept in one place"
)]
pub(crate) fn prove_dead(
    dir: &Path,
    target: &Target,
    verb: Verb,
    err: &mut impl Write,
) -> io::Result<Option<Proven>> {
    // DOES THE PANE EXIST? This read answers that and nothing else answers it:
    // measured 2026-09-18, `display-message -p -t %<missing>` EXITS 0 and
    // renders every field EMPTY, so the pane probe below cannot tell a pane
    // that is gone from one it read badly. `pane_dead` is `0` or `1` for a
    // pane that exists and unreadable for one that does not, so an unreadable
    // answer here IS the missing pane — named as such, before anything else.
    let Some(pane_is_dead) = transport::observe_pane_dead(&target.server, &target.pane) else {
        writeln!(
            err,
            "Error: pane {} of '{}' is gone — ae does not recreate a seat's pane. Stop and resume the session instead.",
            target.pane, target.agent
        )?;
        return Ok(None);
    };
    if pane_is_dead {
        writeln!(
            err,
            "Error: pane {} of '{}' is DEAD — its shell exited and tmux is holding the pane. Stop and resume the session.",
            target.pane, target.agent
        )?;
        return Ok(None);
    }
    // ONE probe and ONE process-table snapshot feed every step below, so no two
    // steps can disagree about the same instant.
    let Some(probe) = transport::observe_pane_probe(&target.server, &target.pane) else {
        writeln!(
            err,
            "Error: pane {} of '{}' could not be read. Nothing was {}.",
            target.pane, target.agent, verb.past
        )?;
        return Ok(None);
    };
    if target.slot.is_empty() {
        writeln!(
            err,
            "Error: pane {} carries no ae slot — it is not a seat's pane (a monitor pane, or one ae never stamped).",
            target.pane
        )?;
        return Ok(None);
    }
    // The recorded binary is the seat's IDENTITY, read once and used by both
    // the refusal below and the liveness decision.
    let agent_bin = deliver::recorded_binary(dir, &target.slot);
    if agent_bin.is_empty() {
        writeln!(
            err,
            "Error: no recorded tool for '{}' (agent_bin.{} is missing) — re-run the pane line by hand, or stop and resume the session.",
            target.agent, target.slot
        )?;
        return Ok(None);
    }
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let work_dir = crate::lifecycle::meta_value(&bytes, "work_dir");
    if work_dir.is_empty() || !crate::lifecycle::dir_exists(Path::new(&work_dir)) {
        writeln!(
            err,
            "Error: '{}' records its working copy at {} but it is gone — restore it, or end the session.",
            crate::lifecycle::meta_value(&bytes, "session"),
            if work_dir.is_empty() {
                "nothing"
            } else {
                &work_dir
            }
        )?;
        return Ok(None);
    }
    // The ONE seat-command resolution, so `_run` is handed the same command the
    // launch would have composed and its own snapshot check can refuse a tool
    // that changed underneath the seat.
    let seat = match crate::run::read_seat(dir, &target.slot, None) {
        Ok(seat) => seat,
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was {}.", verb.past)?;
            return Ok(None);
        }
    };
    let table = procs::snapshot();
    let walk = match probe.pid {
        Some(pid) => procs::descendancy(table.as_deref(), pid, &agent_bin),
        None => Descendancy::Unknown,
    };
    if identity_proven(&probe.command, &agent_bin, walk) {
        writeln!(
            err,
            "Error: '{}' is running (pane {}) — nothing to {}.",
            target.agent, target.pane, verb.imperative
        )?;
        return Ok(None);
    }
    if let Some(line) = busy_refusal(&probe, table.as_deref(), target, verb) {
        writeln!(err, "{line}")?;
        return Ok(None);
    }
    // THE ONE DEAD OWNER, fed the readings taken above.
    let binary_known = !crate::watchdog::command_is_shell(&agent_bin);
    let shell_in_foreground = crate::watchdog::command_is_shell(&probe.command);
    if deliver::observed_liveness(shell_in_foreground, binary_known, probe.pid, walk)
        != deliver::PaneLiveness::Dead
    {
        writeln!(
            err,
            "Error: cannot prove '{}' dead (pane {}): {}. Nothing was {}.",
            target.agent,
            target.pane,
            unproven_gap(probe.pid, &agent_bin, table.is_some()),
            verb.past
        )?;
        return Ok(None);
    }
    let id_before =
        crate::lifecycle::meta_value(&bytes, &format!("harness_session.{}", target.slot));
    Ok(Some(Proven {
        seat,
        agent_bin,
        work_dir,
        id_before,
    }))
}

/// The BUSY refusals — a pane whose shell is not idle, in the two shapes that
/// are not the seat's own tool. A human's editor or a running script is not a
/// dead seat, and ae never clears a line it did not put there.
fn busy_refusal(
    probe: &ObservedPaneProbe,
    table: Option<&[procs::Proc]>,
    target: &Target,
    verb: Verb,
) -> Option<String> {
    let (agent, pane) = (&target.agent, &target.pane);
    let word = verb.imperative;
    if !crate::watchdog::command_is_shell(&probe.command) {
        return Some(format!(
            "Error: pane {pane} of '{agent}' is BUSY — '{}' holds its foreground, not a shell. Let it finish, then {word}.",
            probe.command
        ));
    }
    if table.is_some_and(|table| procs::has_any_descendant(table, probe.pid)) {
        return Some(format!(
            "Error: pane {pane} of '{agent}' is BUSY — a process runs under its shell. Let it finish, then {word}."
        ));
    }
    None
}

/// What the paste-and-observe half concluded.
pub(crate) enum Started {
    /// The seat's own tool is running. `resuming_seat` was read BEFORE the
    /// paste, because `_run` publishes the start marker pre-exec on a create
    /// and a later read would call a fresh seat a resumed one.
    Running { resuming_seat: bool },
    /// The line went in and the tool never appeared within the bound.
    NotSeen,
    /// Refused BEFORE the paste. The pane was never written to, the refusing
    /// site has already said why, and nothing past here may claim otherwise.
    NotPasted,
}

/// Clear the shell's input line, paste the launch line, and watch for the
/// seat's own tool. Runs UNDER the lock.
pub(crate) fn start(
    dir: &Path,
    target: &Target,
    proven: &Proven,
    now: Timestamp,
    verb: Verb,
    err: &mut impl Write,
) -> io::Result<Started> {
    // The launch-attempt stamp, before the write it is about, exactly as a
    // launch, a resume and a spawn owe it — a reboot's boot-time proof reads
    // this session as untouched since the boot without it. CHECKED.
    if let Err(why) = crate::store::open(dir).stamp_launch_attempt(now.epoch()) {
        writeln!(
            err,
            "Error: '{}' launch attempt could not be recorded ({why}) — nothing was {}.",
            target.agent, verb.past
        )?;
        return Ok(Started::NotPasted);
    }
    // THE CAPTURE FLOOR, by the same rule the launch's meta document applies:
    // a retained exact conversation keeps the floor it was born under, and a
    // seat whose id cannot be probed gets a fresh one BEFORE the tool starts.
    if proven.seat.tool.adapter().capture.is_needed()
        && !crate::launch::id_probeable(&proven.id_before)
    {
        let _ = crate::meta::rewrite(
            dir,
            &format!("capture_floor.{}", target.slot),
            Some(&now.epoch().to_string()),
        );
    }
    let resuming_seat =
        crate::lifecycle::path_exists(&crate::run::started_marker(dir, &target.slot));
    // A shell prompt is not a modelled composer: nothing here can be verified,
    // and a half-typed line the paste would CONCATENATE with the launch line
    // is the harm worth spending an unverifiable key on. `C-u` alone kills only
    // to the line start, so the cursor goes to the end first.
    for key in [crate::tmux::Key::LineEnd, crate::tmux::Key::ClearLine] {
        if !transport::send_key(&target.server, &target.pane, key) {
            writeln!(
                err,
                "Error: could not clear the shell input line of pane {} — nothing was pasted.",
                target.pane
            )?;
            return Ok(Started::NotPasted);
        }
    }
    // `pane_line`'s directory is the STATE dir and `_run` never chdirs, so the
    // seat's recorded working copy is restored here or a shell that wandered
    // would resume the agent somewhere else entirely.
    let Some(core) = crate::shape::resolved_exe() else {
        writeln!(
            err,
            "Error: the core could not name its own binary — nothing was {}.",
            verb.past
        )?;
        return Ok(Started::NotPasted);
    };
    let line = format!(
        "cd {} && {}",
        crate::launch::shell_quote(&proven.work_dir),
        crate::run::pane_line(
            &crate::run::pane_head(&core),
            dir,
            &target.slot,
            Some(proven.seat.command.as_str()),
        )
    );
    // Fire and forget, as a launch does: the reader IS a shell, and the tool
    // observation below is the truth about whether it took.
    let _ = deliver::submit_shell_text(&target.server, &target.pane, &line);
    for poll in 0..IDENTITY_POLLS {
        std::thread::sleep(IDENTITY_POLL);
        if observe_identity(target, &proven.agent_bin, poll % WALK_EVERY == 0) {
            // The POST-EXEC lifecycle stamp, and only once the exec is proven:
            // writing it for a tool that never started would date a launch that
            // did not happen.
            if proven.seat.tool.adapter().capture.is_needed() {
                let _ = crate::meta::rewrite(
                    dir,
                    &format!("launch_time.{}", target.slot),
                    Some(&Timestamp::now().epoch().to_string()),
                );
            }
            return Ok(Started::Running { resuming_seat });
        }
    }
    Ok(Started::NotSeen)
}

/// One identity observation: the pane's foreground always, the process walk
/// only when the caller is spending a snapshot on this round.
pub(crate) fn observe_identity(target: &Target, agent_bin: &str, walk_now: bool) -> bool {
    let Some(probe) = transport::observe_pane_probe(&target.server, &target.pane) else {
        return false;
    };
    let walk = match (walk_now, probe.pid) {
        (true, Some(pid)) => procs::descendancy(procs::snapshot().as_deref(), pid, agent_bin),
        _ => Descendancy::Unknown,
    };
    identity_proven(&probe.command, agent_bin, walk)
}

/// Everything past the lock: the launch turn, the conversation the seat came
/// back on, the post-launch capture, and the last word.
fn finish(
    dir: &Path,
    target: &Target,
    proven: &Proven,
    resuming_seat: bool,
    actor: Actor<'_>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let tool = proven.seat.tool;
    let prompt = crate::launch::initial_prompt_for(tool, dir, &target.slot);
    let turn = if prompt.is_empty()
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
    // Re-read, because `_run` decided: a recorded id that failed its store
    // probe has been moved to the predecessor chain and the current row cleared.
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let id_after =
        crate::lifecycle::meta_value(&bytes, &format!("harness_session.{}", target.slot));
    let conversation = conversation_word(&proven.id_before, &id_after);
    // Only a slot that reads `pending` NOW needs capturing: `_run`'s own
    // in-pane fallback may have cleared it after the pre-exec read.
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
    // LAST: the tool may have died again while the turn was being delivered,
    // and exit 0 means the seat is up NOW.
    if !observe_identity(target, &proven.agent_bin, true) {
        record(dir, actor, target, "tool stopped after the relaunch");
        writeln!(
            err,
            "Error: '{}' started and is gone again (pane {}) — look at the pane.",
            target.agent, target.pane
        )?;
        return Ok(EXIT_FAILED);
    }
    let (ok, word) = turn.map_or((true, ""), turn_verdict);
    let turn_note = if word.is_empty() {
        String::new()
    } else {
        format!(", turn {word}")
    };
    let line = format!(
        "relaunched {} (pane {}, slot {}): {conversation}{turn_note}",
        target.agent, target.pane, target.slot
    );
    record(dir, actor, target, &line);
    if !ok {
        writeln!(err, "Error: {line} — the turn never landed; re-send it.")?;
        return Ok(EXIT_FAILED);
    }
    writeln!(out, "{line}")?;
    // The brief belongs to the spawner (rule 9), and ae does not re-deliver it:
    // a seat that came back on a FRESH conversation has never seen it.
    if conversation == "fresh"
        && crate::lifecycle::path_exists(&crate::run::prompt_file(dir, &target.slot))
    {
        writeln!(
            out,
            "  conversation gone — brief NOT re-delivered, its spawner re-sends it"
        )?;
    }
    Ok(0)
}

/// One `relaunch` record for every attempt that REACHED the pane — the seat's
/// history is the only place a later reader can see that its conversation was
/// restarted, and the only place that must not claim a paste that never was.
fn record(dir: &Path, actor: Actor<'_>, target: &Target, summary: &str) {
    let _ = crate::store::open(dir).append_event(&tracked::event_line(&EventFields {
        ts: actor.now,
        actor: if actor.caller.is_empty() {
            "human"
        } else {
            actor.caller
        },
        action: RELAUNCH_ACTION,
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
        caller_pane: "",
        caller_session_uuid: "",
        identity_gap: "",
        summary,
        body_file: "",
    }));
}

#[cfg(test)]
mod tests {
    use super::{
        Descendancy, TurnOutcome, conversation_word, identity_proven, turn_verdict, unproven_gap,
    };

    #[test]
    fn the_seats_own_tool_is_proven_by_the_foreground_or_by_the_walk_and_by_nothing_else() {
        // The launcher case: a tool behind node/bun keeps its launcher in the
        // foreground, and only the walk finds it.
        assert!(identity_proven("node", "claude", Descendancy::Present));
        // The plain case: the tool IS the foreground.
        assert!(identity_proven("claude", "claude", Descendancy::Absent));
        assert!(
            identity_proven("/opt/tools/claude", "claude", Descendancy::Unknown),
            "the foreground is compared by BASENAME, as the recorded binary is"
        );
        // THE MUTATION THIS PIN EXISTS FOR. `_run` is the core composing the
        // seat's command: a real process, not a shell, and not a started agent.
        // A check that asked only "is something alive here" would pass it.
        assert!(
            !identity_proven("ae", "claude", Descendancy::Absent),
            "the core in the foreground is NOT the seat's tool"
        );
        assert!(
            !identity_proven("sh", "claude", Descendancy::Absent),
            "a bare shell is not a started agent"
        );
        assert!(
            !identity_proven("claude", "", Descendancy::Present),
            "with no recorded binary there is no identity to prove"
        );
        assert!(
            !identity_proven("node", "claude", Descendancy::Unknown),
            "an unreadable process table proves nothing"
        );
    }

    #[test]
    fn only_a_turn_that_never_landed_fails_the_relaunch_and_each_prints_its_own_word() {
        assert_eq!(turn_verdict(TurnOutcome::Submitted), (true, "submitted"));
        // THE MUTATION THIS PIN EXISTS FOR: an unproven submit must not be
        // reported as a proven one. It is exit 0 — the turn was pasted and
        // Enter pressed — but the WORD is never "submitted".
        assert_eq!(
            turn_verdict(TurnOutcome::Unconfirmed),
            (true, "unconfirmed")
        );
        assert_eq!(
            turn_verdict(TurnOutcome::Undelivered),
            (false, "undelivered")
        );
    }

    #[test]
    fn the_conversation_is_exact_only_when_a_probeable_id_survived_the_run() {
        assert_eq!(conversation_word("abc-123", "abc-123"), "exact");
        assert_eq!(
            conversation_word("abc-123", "pending"),
            "fresh",
            "a recorded id that failed its store probe took the fallback"
        );
        assert_eq!(conversation_word("abc-123", "def-456"), "fresh");
        assert_eq!(
            conversation_word("pending", "pending"),
            "fresh",
            "an id that was never probeable cannot have been resumed exactly"
        );
        assert_eq!(conversation_word("", ""), "fresh");
    }

    #[test]
    fn an_unproven_seat_is_told_which_gap_left_it_unproven() {
        assert_eq!(
            unproven_gap(None, "claude", true),
            "tmux reported no pid for the pane"
        );
        assert_eq!(
            unproven_gap(Some(7), "claude", false),
            "the process table could not be read"
        );
        assert!(
            unproven_gap(Some(7), "bash", true).contains("'bash'"),
            "the gap NAMES the recorded tool it could not tell from a shell"
        );
    }
}
