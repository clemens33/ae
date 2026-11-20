//! Seat start and readiness: the pane command, the wait for the agent, the gated launch-turn delivery.

use super::{
    FORK_POLLS, LAUNCH_FAILED_ACTION, LAUNCH_READY_POLLS, Launching, Path, READY_CHUNK,
    READY_CHUNKS, START_POLL, START_POLLS, ServerId, Session, ToolKind, Write, config, deliver, io,
    launch, meta, tmux, transport, write_private,
};

/// Phase 1 of a seat launch: hand one agent's pane the command that BECOMES
/// its agent, and wait for the tool to take the pane over.
///
/// Pastes the pane command, waits for the tool's process, and stamps
/// `launch_time`. The gated launch turn is NOT delivered here — it is
/// returned, so the caller delivers it after the monitors start. The paste
/// gate is evaluated at this exec-time moment for the reason the caller
/// states.
pub(super) fn start_agent(
    shape: &Session,
    dir: &Path,
    core: &Path,
    agent: &Launching,
    server: &ServerId,
    err: &mut impl Write,
) -> io::Result<Result<Option<String>, String>> {
    // THE MARKER IS THE CREATE-VS-RESUME DISCRIMINATOR, so a fresh seat must
    // not inherit one.
    if !shape.resuming
        && let Err(why) = crate::run::clear_slot(dir, &agent.slot)
    {
        writeln!(
            err,
            "ae: could not clear the start marker for '{}' ({why}) — agent not started",
            agent.slot
        )?;
        return Ok(Err(format!("stale start marker for {}", agent.slot)));
    }
    let resuming_seat =
        crate::lifecycle::path_exists(&crate::run::started_marker(dir, &agent.slot));
    // Fire and forget: the reader here IS a shell, and an unconfirmed submit
    // must not abort a launch that may well have taken.
    // One head for line and wait: a link moving between two reads cannot disagree.
    let head = crate::run::pane_head(core);
    let snapshot = agent
        .command_snapshot
        .as_ref()
        .map(config::ResolvedCommand::as_str);
    let command = crate::run::pane_line(&head, dir, &agent.slot, snapshot);
    let _ = deliver::submit_shell_text(server, &agent.pane, &command);
    let launched = head
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    wait_for_agent_start(server, &agent.pane, agent.tool, &launched);
    if agent.tool.adapter().capture.is_needed() {
        // Kept as the legacy/lifecycle launch stamp. Capture safety uses the
        // separate `capture_floor` already published before this exec.
        let _ = meta::rewrite(
            dir,
            &format!("launch_time.{}", agent.slot),
            Some(&crate::time::Timestamp::now().epoch().to_string()),
        );
    }
    let prompt = launch::initial_prompt_for(agent.tool, dir, &agent.slot);
    if prompt.is_empty() || !launch_turn_is_pasted(agent.tool, resuming_seat) {
        return Ok(Ok(None));
    }
    Ok(Ok(Some(prompt)))
}

/// The launch ID this seat launches with: the one it already has, else a fresh
/// one. It has two jobs: disambiguating post-launch capture in a tool's own
/// store, and guarding observed-model writes against a re-created seat.
///
/// Every tool needs the second job. The first and marker injection remain
/// gated by the adapter's capture and marker capabilities. Measured 2026-09-04:
/// two codex seats resumed while both were still `pending` then raced onto ONE
/// rollout and recorded the SAME id twice.
pub(crate) fn launch_token(_tool: ToolKind, stored: Option<String>) -> String {
    stored
        .filter(|id| !id.is_empty())
        .unwrap_or_else(launch::generate_uuid)
}

/// Must this seat's first turn be PASTED rather than baked into its argv?
///
/// `_run` appends the inline first message on the CREATE path alone; EVERY
/// resume composes with an empty prompt — the exact one, and equally the fresh
/// fallback a seat whose recorded id fails its store probe has to take. So the start
/// marker decides this, and the id must not: a codex seat resumed while its
/// `harness_session` is still `pending` takes that fallback, and gating on a
/// probeable id would leave it with NO user turn at all. It would then write no
/// rollout (see `launch::initial_prompt_for` for the measurement), so the
/// re-capture that `launching_capture` deliberately schedules for exactly those
/// pending slots would find nothing, and the seat would stay unresumable for
/// the rest of its life.
pub(crate) const fn launch_turn_is_pasted(tool: ToolKind, resuming_seat: bool) -> bool {
    resuming_seat && tool.adapter().input.paste_initial_on_resume
}

/// What one seat's launch turn DID — the fact a per-slot caller needs and the
/// roster-wide launch deliberately ignores.
///
/// A launch delivers N turns and fails none of them loudly enough to stop:
/// each failure is already durable on its own (the preserved file, the event,
/// the stderr line). A SINGLE-slot caller answers for one seat and must map
/// the turn onto its own exit code, so the same delivery reports what it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnOutcome {
    /// The turn was pasted and its submit PROVEN.
    Submitted,
    /// The turn was pasted and Enter pressed, but the submit could not be
    /// proven either way. `send`'s own wording, and never "confirmed".
    Unconfirmed,
    /// The turn never landed. The text is preserved and the failure recorded.
    Undelivered,
    /// The turn never landed because the seat shows a prompt only the HUMAN
    /// may answer, named in the recorded failure; the text is preserved.
    Blocked,
}

/// What a launch turn does when its seat shows a prompt only the human may
/// answer — the CALLER's choice, because only the caller knows whether a human
/// is there to answer it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnHumanPrompt {
    /// Keep the whole readiness wait: a human at a fresh launch answers the
    /// prompt, and the turn must still land by itself.
    Wait,
    /// Give the pane back once the SAME prompt holds on two consecutive reads:
    /// a single-seat verb's caller is often an agent, which cannot answer it
    /// and needs control back to tell the human.
    FailFast,
}

/// One readiness chunk: the input became ready, or it did not and the pane
/// showed this human-only prompt, if any.
pub(super) enum ReadyRead {
    Ready,
    Unready(Option<crate::watchdog::HumanPrompt>),
}

/// Where the launch turn's readiness wait ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Readiness {
    Ready,
    Blocked(crate::watchdog::HumanPrompt),
    NotReady,
}

/// Fold the readiness chunks, in order, into where the wait ends — pure, so
/// the caller's choice is pinnable without a pane. A prompt is STABLE when the
/// same question held on two consecutive reads, never one, and readiness wins
/// the moment it comes, a prompt the human answered included. `FailFast` stops
/// at the first stable prompt; `Wait` reads every chunk and names the prompt
/// only when it is still stable at the end.
pub(super) fn settle(reads: impl IntoIterator<Item = ReadyRead>, on: OnHumanPrompt) -> Readiness {
    let mut previous: Option<crate::watchdog::HumanPrompt> = None;
    let mut stable = None;
    for read in reads {
        let ReadyRead::Unready(current) = read else {
            return Readiness::Ready;
        };
        stable = match (previous.take(), &current) {
            (Some(before), Some(now)) if before.question == now.question => Some(now.clone()),
            _ => None,
        };
        if on == OnHumanPrompt::FailFast
            && let Some(prompt) = &stable
        {
            return Readiness::Blocked(prompt.clone());
        }
        previous = current;
    }
    stable.map_or(Readiness::NotReady, Readiness::Blocked)
}

/// How many polls one chunk takes to wait `READY_CHUNK` poll intervals, for
/// either input model. A modelled wait sleeps once per poll; an unmodelled one
/// spends its first poll on the capture it compares against
/// (`deliver::wait_input_ready`), so its chunk takes one poll more to wait as
/// long, and the chunked wait keeps the whole launch budget for both.
pub(super) const fn chunk_polls(model: crate::tool::InputModel) -> u32 {
    if model.is_modelled() {
        READY_CHUNK
    } else {
        READY_CHUNK + 1
    }
}

/// The launch turn's readiness wait. A tool whose adapter row names a
/// human-only prompt is waited on in chunks, its pane read for that prompt
/// between them through the same capture door readiness uses; every other
/// tool waits exactly as before, in one piece. A measured trust block between
/// chunks is handed to `trusted`, and one it answered reads as no prompt, so
/// the wait goes on toward a ready box.
fn wait_ready(
    server: &ServerId,
    pane: &str,
    tool: ToolKind,
    on: OnHumanPrompt,
    mut trusted: impl FnMut() -> bool,
) -> Readiness {
    let input = tool.adapter().input;
    let polled =
        |polls| deliver::wait_input_ready(server, pane, input.model, input.composed, polls);
    if tool.adapter().prompt.is_none() {
        return if polled(LAUNCH_READY_POLLS) {
            Readiness::Ready
        } else {
            Readiness::NotReady
        };
    }
    let reads = (0..READY_CHUNKS).map(|_| {
        if polled(chunk_polls(input.model)) {
            return ReadyRead::Ready;
        }
        let prompt = transport::capture_screen(server, pane, tmux::Styling::Plain)
            .and_then(|frame| crate::watchdog::human_prompt_class(&frame, tool.adapter().name));
        if prompt.as_ref().is_some_and(|asked| asked.trust.is_some()) && trusted() {
            return ReadyRead::Unready(None);
        }
        ReadyRead::Unready(prompt)
    });
    settle(reads, on)
}

/// The gated, loud, DURABLE launch-turn delivery, for ONE seat.
///
/// The side effects are unchanged and unconditional: a failure preserves the
/// text, records the event and says so on stderr. The RETURN is the addition —
/// the launch throws it away, a single-slot caller reads it.
#[expect(
    clippy::too_many_arguments,
    reason = "one seat's delivery facts plus the caller's prompt choice, spelled out"
)]
pub(crate) fn deliver_launch_turn(
    dir: &Path,
    server: &ServerId,
    slot: &str,
    pane: &str,
    tool: ToolKind,
    prompt: &str,
    on_prompt: OnHumanPrompt,
    err: &mut impl Write,
) -> io::Result<TurnOutcome> {
    let model = tool.adapter().input.model;
    let mut outcome = TurnOutcome::Undelivered;
    // `auto_trust` answers a trust block inside this wait until an answer is
    // tried that is not a "not yet".
    let mut tried = crate::autotrust::InWait::Again;
    let trusted = || {
        if tried == crate::autotrust::InWait::Again {
            tried = crate::autotrust::in_wait(dir, server, slot, pane, &mut *err);
            return tried == crate::autotrust::InWait::Trusted;
        }
        false
    };
    let reason = match wait_ready(server, pane, tool, on_prompt, trusted) {
        Readiness::Ready => {
        // NO select-pane: `paste-buffer -t` writes to the NAMED pane, and
        // selecting mid-send routes the human's in-flight keystrokes into the
        // target — acute under lead-pair, where two agents share window 0.
        match deliver::stage_and_paste(
            server,
            &format!("ae-launch-{slot}"),
            prompt.as_bytes(),
            pane,
        ) {
            // A bare Enter is not a submit: a booting TUI swallows it, and for
            // a seat resumed while its id is still `pending` this turn is the
            // ONLY thing that will ever create a rollout to capture. So the
            // press is PROVEN, and a turn left in the box falls through to the
            // durable failure below rather than passing as delivered.
            Ok(()) => match deliver::submit_staged(server, pane, model) {
                deliver::SubmitState::Submitted => return Ok(TurnOutcome::Submitted),
                deliver::SubmitState::Unknown(_) => return Ok(TurnOutcome::Unconfirmed),
                deliver::SubmitState::StillStaged => {
                    "submit UNCONFIRMED — the turn is staged unsent in the input box".to_owned()
                }
            },
            Err(failure) => format!("submit UNCONFIRMED ({failure:?}) — it may be staged unsent"),
        }
        }
        Readiness::Blocked(asked) => {
            outcome = TurnOutcome::Blocked;
            format!(
                "a prompt only the human may answer is up: \"{}\" — press: {}; ae never answers it",
                asked.question, asked.keys
            )
        }
        Readiness::NotReady => "input never reached a confirmed-ready state within 45s (still initializing, busy, modal, or unreadable)".to_owned(),
    };
    let file = dir.join(format!("undelivered.launch-{slot}.txt"));
    let preserved = write_private(&file, prompt).is_ok();
    let _ = crate::store::open(dir).append_event(&crate::tracked::event_line(
        &crate::tracked::EventFields {
            ts: crate::time::Timestamp::now(),
            actor: "ae",
            action: LAUNCH_FAILED_ACTION,
            target: slot,
            reference: "",
            actor_slot: "",
            actor_session: "",
            target_slot: slot,
            target_session: "",
            actor_session_id: "",
            target_session_id: "",
            target_server: "",
            target_pane: "",
            target_session_uuid: "",
            caller_server: "",
            caller_pane: "",
            caller_session_uuid: "",
            identity_gap: "",
            summary: &format!(
                "launch prompt NOT delivered to {slot} ({}, pane {pane}): {reason}",
                tool.as_str()
            ),
            body_file: "",
        },
    ));
    writeln!(
        err,
        "ae: LAUNCH PROMPT NOT DELIVERED to {slot} (pane {pane}): {reason}"
    )?;
    if preserved {
        writeln!(err, "ae: the text is preserved at {}", file.display())?;
    }
    Ok(outcome)
}

/// What one reading of a pane says about the tool typed into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Start {
    /// tmux's own forked child: the pane's command has not run yet.
    Forked,
    /// A shell, no answer, or ae's own binary on its way to the tool.
    Waiting,
    /// The tool, or a command that is none of the above.
    Started,
}

/// The reading of `current`, the pane's command, for `tool` typed into it.
pub(super) fn pane_start(current: &str, tool: &str, launched: &str) -> Start {
    // opencode's process reports as `opencode.exe` — its bun-built launcher.
    if current.strip_suffix(".exe").unwrap_or(current) == tool {
        return Start::Started;
    }
    if current == "tmux" {
        return Start::Forked;
    }
    if crate::watchdog::command_is_shell(current) || (!launched.is_empty() && current == launched) {
        return Start::Waiting;
    }
    Start::Started
}

/// Read until a reading is [`Start::Started`] or either budget is spent.
pub(super) fn await_start(
    mut read: impl FnMut() -> String,
    mut nap: impl FnMut(),
    tool: &str,
    launched: &str,
) {
    let (mut waiting, mut forked) = (0, 0);
    while waiting < START_POLLS && forked < FORK_POLLS {
        match pane_start(&read(), tool, launched) {
            Start::Started => return,
            Start::Waiting => waiting += 1,
            Start::Forked => forked += 1,
        }
        nap();
    }
}

/// Wait, briefly, for the tool's process to replace the pane's shell. `launched`
/// is the file name the pasted line launched under, from the same owner; empty
/// when the caller takes ae's own binary in the pane as the tool's start.
pub(crate) fn wait_for_agent_start(server: &ServerId, pane: &str, tool: ToolKind, launched: &str) {
    if !tool.adapter().input.wait_for_process {
        return;
    }
    // The pane runs ae's own binary before it runs the tool: `_run` composes
    // the command and `exec`s it, so for a moment `pane_current_command` is
    // the launched name rather than the tool.
    await_start(
        || {
            transport::observe_pane_probe(server, pane)
                .map(|probe| probe.command)
                .unwrap_or_default()
        },
        || std::thread::sleep(START_POLL),
        tool.as_str(),
        launched,
    );
}
