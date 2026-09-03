//! Pane DELIVERY, in the core (B move 1).
//!
//! What the frozen `helper_send_body` and `ae_submit_pasted_message` did, done
//! here: the dead-pane refusal, the provenance envelope, the recovery-body
//! store, the per-target lock, the busy/human-presence deferral, the bracketed
//! paste, the oversize notice with its on-screen proof, and the submit
//! verification. The callback into bash (`_send-deliver`) is no longer on this
//! path; the glue's copy stays until it is cut, and is not read by anything
//! here.
//!
//! # The order is the frozen one
//!
//! Refuse → frame → STORE → lock → prepare → defer → cancel → paste → prove →
//! Enter → verify → unlock. Two of those are load-bearing in an order that
//! looks arbitrary:
//!
//! * the recovery body is published BEFORE the paste, so a message that does
//!   not survive delivery is still recoverable by its recipient. A storage
//!   failure is terminal — no pane text, no paste and no event may follow one.
//! * the busy check happens BEFORE the copy-mode cancel, because the cancel
//!   keystroke ITSELF would clobber staged or human-typed text. Inspect first,
//!   then cancel.
//!
//! # No focus change, ever
//!
//! `paste-buffer -t` and `send-keys -t` write to the NAMED pane, and focus is
//! not part of any TUI's submission contract. A `select-pane` here changed the
//! window's active pane and routed the human's in-flight keystrokes into the
//! target mid-send — acute under a lead pair, where two agents share window 0.
//! Only the explicit `focus` helper may select panes.
//!
//! # No durable outbox
//!
//! ae is not a queue. Every failure below is LOUD and non-zero, the body stays
//! on disk where the recipient can read it, and the LLM sender re-sends when it
//! sees the failure.

pub mod notice;
pub mod region;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::inventory::ServerId;
use crate::state;
use crate::tmux::{Key, Styling};
use crate::transport;
use region::{Occupancy, Tool};

/// How long a send waits for a busy target before abandoning —
/// `AE_SEND_DEFER_SEC`'s default. A human mid-sentence outlasts two seconds.
pub const DEFAULT_DEFER: Duration = Duration::from_secs(30);

/// How often the deferral loop re-reads the target.
const DEFER_POLL: Duration = Duration::from_millis(400);

/// A client's quiet period, past which it is no longer evidence of a human at
/// the pane.
const VIEW_GRACE: i64 = 4;

/// The pause between the paste and the Enter, for claude.
///
/// Claude only: a 24-sample sweep found sends to IDLE claude panes losing the
/// Enter while sends to BUSY panes queued and delivered. The root cause is
/// unproven and version-dependent, so this is cheap insurance for the
/// flow-control hypothesis, NOT the guarantee — the verify-then-retry loop
/// below is. Codex keeps the short settle: no evidence of the drop there, and
/// every send would pay the latency.
const SETTLE_CLAUDE: Duration = Duration::from_millis(300);

/// The same pause for every other tool.
const SETTLE_OTHER: Duration = Duration::from_millis(100);

/// The pause before each staged re-read after Enter.
const VERIFY_POLL: Duration = Duration::from_millis(300);

/// How many extra Enters the verification will send before giving up.
const VERIFY_RETRIES: usize = 2;

/// The pause before each notice proof attempt.
const NOTICE_POLL: Duration = Duration::from_millis(100);

/// The pause after an interrupt's cancel keys, before its message is pasted.
const INTERRUPT_SETTLE: Duration = Duration::from_millis(500);

/// How long the per-target lock is waited for.
///
/// The frozen `ae_lock_target` blocked forever. This is BOUNDED and loud
/// instead: a helper that hangs on a lock hangs the agent that ran it, with no
/// output to say why, and a send that cannot start is a re-send rather than a
/// lost message. Generous enough that a full deferral plus a paste ahead of us
/// in the queue does not trip it.
const LOCK_WAIT: Duration = Duration::from_mins(2);

/// Which entry point is delivering, and therefore what its refusals say and
/// what it does to the pane before pasting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `send`, and the tracked requests behind it. Frames the body in the
    /// provenance envelope and waits for a clear input box.
    Send,
    /// `interrupt`. Cancels first and pastes into whatever is there — the
    /// whole point is to reach a target that is mid-generation, so the
    /// deferral that protects a send would defeat it. Not framed: an
    /// interrupt is a control action, not transcript chat.
    Interrupt,
    /// A spawn's BRIEF, pasted into a freshly launched TUI. Not framed — the
    /// task is the agent's own first instruction, not a message from a peer —
    /// and not deferred, because the caller has already proven the input box
    /// idle with [`input_ready`]; a second wait here would only re-ask a
    /// question that was just answered, on a pane nobody else is writing to.
    Launch,
}

/// One delivery, fully specified.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    /// The session meta directory: where `messages/` and the reply helper live.
    pub dir: &'a Path,
    /// The server the TARGET is on, not the caller's ambient one.
    pub server: &'a ServerId,
    /// The target pane id.
    pub pane: &'a str,
    /// How the target is named in every diagnostic and in the event.
    pub logged_target: &'a str,
    /// The target's session — the notice's path grammar, and which session's
    /// meta the dead-pane guard reads.
    pub target_session: &'a str,
    /// The target pane's `@ae_slot`, or empty. Already read by the resolver,
    /// so nothing here asks tmux for it a second time.
    pub pane_slot: &'a str,
    /// This session's name.
    pub own_session: &'a str,
    /// The event action the body store names the recovery file after.
    pub action: &'a str,
    /// The request id, or empty.
    pub reference: &'a str,
    /// The VERIFIED sender for the envelope. Empty is `unverified` — never
    /// bare, because bare is the human's signature and only direct typing
    /// earns it.
    pub actor: &'a str,
    /// The message as composed, before framing.
    pub body: &'a str,
    /// Which entry point this is.
    pub shape: Shape,
    /// How long to wait for a busy target.
    pub defer: Duration,
}

/// A delivery that landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivered {
    /// The published recovery body — the event's `body_file`.
    pub body_file: String,
    /// Exactly what was framed and stored, which is what a pane send's event
    /// summary is of.
    pub framed: String,
}

/// Why a delivery did not land. Every arm has already printed its own loud
/// line; this is what the caller needs to decide the exit code and whether a
/// `delivery-failed` event is owed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The target pane is a shell, not a running agent. Nothing was stored.
    DeadPane,
    /// The recovery body could not be published. Nothing was pasted.
    Storage,
    /// The per-target lock was not acquired.
    Lock,
    /// The oversize notice could not be composed as a small pointer.
    NoticeRefused {
        /// The published recovery body, still readable.
        body_file: String,
    },
    /// The target stayed busy for the whole deferral.
    Abandoned,
    /// The paste itself failed. Nothing reached the pane.
    Paste {
        /// The published recovery body, still readable.
        body_file: String,
    },
    /// The submit was never confirmed. It may be staged, unsent.
    Unconfirmed {
        /// The published recovery body, still readable.
        body_file: String,
        /// Whether the failure was the notice's on-screen proof, which is the
        /// arm the frozen body records a `delivery-failed` event for.
        notice: bool,
    },
}

impl Failure {
    /// The recovery body, where one was published before the failure.
    #[must_use]
    pub fn body_file(&self) -> &str {
        match self {
            Self::DeadPane | Self::Storage | Self::Lock | Self::Abandoned => "",
            Self::NoticeRefused { body_file }
            | Self::Paste { body_file }
            | Self::Unconfirmed { body_file, .. } => body_file,
        }
    }
}

/// Deliver `request` to its pane.
///
/// # Errors
///
/// Only a failure to write `err`. A refused or unconfirmed delivery is the
/// `Ok(Err(_))` arm, because it is an outcome rather than an I/O fault.
pub fn deliver(
    request: &Request<'_>,
    err: &mut impl Write,
) -> io::Result<Result<Delivered, Failure>> {
    let probe = transport::observe_pane_probe(request.server, request.pane).unwrap_or_default();
    let tool = target_tool(request, &probe.command);
    // Interpreted-sink guard: refuse to paste into a pane whose agent has DIED
    // and dropped to a shell — a stray Enter would EXECUTE the message as a
    // shell command. No retry and no deferral: a dead agent will not recover on
    // its own, so this is placed BEFORE the lock and the deferral rather than
    // after a full wait.
    if pane_agent_is_dead(request, &probe) {
        writeln!(err, "{}", dead_pane_line(request))?;
        return Ok(Err(Failure::DeadPane));
    }
    let framed = frame(request);
    // Publish the exact recoverable pane text BEFORE locking or submitting it.
    // A storage failure is terminal for this delivery.
    let body_file = match store_body(request.dir, request.reference, request.action, &framed) {
        Ok(path) => path.display().to_string(),
        Err(why) => {
            writeln!(err, "ae: message body storage failed: {why}")?;
            return Ok(Err(Failure::Storage));
        }
    };
    let Some(_held) = lock_target(request.dir, request.pane) else {
        writeln!(
            err,
            "ae: {} to {} ABANDONED — another delivery held the target lock for {}s. Re-send.",
            request.action,
            request.logged_target,
            LOCK_WAIT.as_secs()
        )?;
        return Ok(Err(Failure::Lock));
    };
    let prepared = notice::prepare(
        tool,
        request.action,
        request.reference,
        envelope_actor(request),
        request.target_session,
        request.own_session,
        Path::new(&body_file),
        framed.len() as u64,
        request.dir,
    );
    let Ok(mode) = prepared else {
        writeln!(
            err,
            "ae: oversized {} notice could not be composed; body preserved at {body_file}. Nothing was submitted.",
            request.action
        )?;
        return Ok(Err(Failure::NoticeRefused { body_file }));
    };
    if request.shape == Shape::Send && !wait_for_quiet(request, tool) {
        writeln!(
            err,
            "ae: send to {} ABANDONED — target stayed busy / human input or attention (not clear within {}s; AE_SEND_DEFER_SEC overrides). Re-send.",
            request.logged_target,
            request.defer.as_secs()
        )?;
        return Ok(Err(Failure::Abandoned));
    }
    // Safe now: cancel, then paste — all by `-t` target, never by selection.
    let _ = transport::send_key(request.server, request.pane, Key::CancelCopyMode);
    if request.shape == Shape::Interrupt {
        let _ = transport::send_key(request.server, request.pane, Key::Escape);
        std::thread::sleep(INTERRUPT_SETTLE);
    }
    let payload = match &mode {
        notice::Mode::Direct => framed.as_str(),
        notice::Mode::Notice(pointer) => pointer.as_str(),
    };
    match submit(request, tool, payload, &mode, &body_file, err)? {
        Ok(()) => Ok(Ok(Delivered { body_file, framed })),
        Err(failure) => {
            // The submit's own line said WHICH step failed; this one names the
            // delivery and where its body is. The frozen body printed both,
            // for every arm of the submit — a staged-but-unsent paste and a
            // paste that never landed are equally worth re-sending, and only
            // this line says where the text went.
            writeln!(
                err,
                "ae: {} to {} UNCONFIRMED — submit not verified; body preserved at {body_file}. Re-send.",
                match request.shape {
                    Shape::Send => "send",
                    Shape::Interrupt => "interrupt message",
                    Shape::Launch => "spawn brief",
                },
                request.logged_target
            )?;
            Ok(Err(failure))
        }
    }
}

/// What an unbindable caller's envelope says.
///
/// EVERY helper-delivered message is marked, with a verified sender or with
/// this — never with a guess and never left bare. Bare is the human's
/// signature, and only direct typing earns it, so an unbindable caller is
/// marked unverified rather than falling through to look like the human.
pub const UNVERIFIED: &str = "unverified";

/// The message as it reaches the pane.
///
/// The origin envelope is one terse line, emitted HERE from the pane's own
/// stamp — never composed from the sender's prose, which is the whole point: a
/// sender cannot mark its own message as coming from someone else by writing a
/// header, because the header it would have to forge is added after it.
fn frame(request: &Request<'_>) -> String {
    if matches!(request.shape, Shape::Interrupt | Shape::Launch) {
        return request.body.to_owned();
    }
    format!(
        "⟦ae:msg from {}⟧\n{}",
        envelope_actor(request),
        request.body
    )
}

/// The name the envelope and the notice head carry — the verified sender, or
/// [`UNVERIFIED`].
fn envelope_actor<'a>(request: &'a Request<'_>) -> &'a str {
    if request.actor.is_empty() {
        UNVERIFIED
    } else {
        request.actor
    }
}

/// The refusal line for this entry point.
fn dead_pane_line(request: &Request<'_>) -> String {
    match request.shape {
        Shape::Send => format!(
            "ae: send to {} REFUSED — target pane is a shell, not a running agent (the agent process is gone). Nothing pasted; a stray Enter would EXECUTE the message as a shell command. Re-launch the agent, then re-send.",
            request.logged_target
        ),
        Shape::Interrupt => format!(
            "ae: interrupt of {} REFUSED — target pane is a shell, not a running agent; a stray Enter would EXECUTE the message as a shell command. Re-launch the agent, then re-send.",
            request.logged_target
        ),
        Shape::Launch => format!(
            "ae: brief for {} REFUSED — the pane is a shell, not a running agent (the launch did not take). Nothing pasted; a stray Enter would EXECUTE the brief as a shell command.",
            request.logged_target
        ),
    }
}

/// The tool whose input box this pane draws — `ae_target_tool`.
///
/// The authoritative fact is the seat's `agent_bin.<slot>` row in this
/// session's meta; the pane's live command is the fallback for a pane outside
/// it. Identity v2: the agent NAME says nothing about the harness.
fn target_tool(request: &Request<'_>, command: &str) -> Tool {
    let recorded = recorded_binary(request.dir, request.pane_slot);
    match Tool::from_name(&recorded) {
        Tool::Other => Tool::from_name(command),
        known => known,
    }
}

/// `agent_bin.<slot>` out of the meta in `dir`, or empty.
///
/// Two callers read DIFFERENT metas with it, and the frozen helpers did the
/// same: `ae_target_tool` reads THIS session's meta (the tool model is a local
/// question — which TUI is ae talking to), while `_agent_bin_for_pane`, whose
/// answer decides whether an agent is dead, reads the PANE'S OWN session's.
/// A cross-session target's seat is recorded where that session records it,
/// and asking the caller's meta for it would compare against whatever agent
/// happens to hold the same slot here.
fn recorded_binary(dir: &Path, slot: &str) -> String {
    if slot.is_empty() {
        return String::new();
    }
    let Ok(bytes) = crate::meta::read_bytes(dir) else {
        return String::new();
    };
    crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes))
        .roster()
        .iter()
        .find(|entry| entry.slot == slot)
        .and_then(|entry| entry.binary.clone())
        .unwrap_or_default()
}

/// The meta directory the TARGET pane's own session keeps — a sibling of this
/// one under the same sessions root. This session's own directory when the
/// target is here, or when its session could not be read.
fn target_meta_dir(request: &Request<'_>) -> PathBuf {
    if request.target_session.is_empty() || request.target_session == request.own_session {
        return request.dir.to_path_buf();
    }
    match request.dir.parent() {
        Some(root) => root.join(request.target_session),
        None => request.dir.to_path_buf(),
    }
}

/// Is this pane a DEAD-agent shell — `_pane_agent_is_dead`?
///
/// A real agent that exits drops its pane back to a login shell, where a paste
/// plus Enter EXECUTES the message as a shell command. Detect: the foreground
/// is a shell AND the agent expected there is not itself a shell AND the agent
/// binary is not alive as a DESCENDANT.
///
/// The descendant walk is not optional. A LIVE agent running a Bash tool shows
/// a shell foreground while its binary is alive underneath, and the version of
/// this check without the walk false-refused sends to live-but-busy agents.
///
/// Fails OPEN throughout — a shell-based agent (a test dummy), an unstamped
/// pane, an unreadable meta and an unreadable process table all deliver.
fn pane_agent_is_dead(request: &Request<'_>, probe: &crate::tmux::ObservedPaneProbe) -> bool {
    if !crate::watchdog::command_is_shell(&probe.command) {
        return false; // a real process is in the foreground -> alive
    }
    let binary = recorded_binary(&target_meta_dir(request), request.pane_slot);
    if binary.is_empty() || crate::watchdog::command_is_shell(&binary) {
        return false; // shell-based agent or undeterminable -> deliver
    }
    let Some(pid) = probe.pid else {
        return false;
    };
    matches!(
        crate::procs::descendancy(crate::procs::snapshot().as_deref(), pid, &binary),
        crate::procs::Descendancy::Absent
    )
}

/// Wait until the target's input box is safe to paste into, or give up.
///
/// Two predicates, either of which defers. The input sensor is primary and
/// fails CLOSED within a modelled tool: OCCUPIED and INDETERMINATE are both
/// unsafe, and only a positive idle read is safe. The client-presence check is
/// supplemental and tool-agnostic — typing into an idle codex is undetectable
/// in its capture, but a human plausibly AT the pane is cheap to see.
///
/// The target lock is HELD while waiting, so competing deliveries queue behind
/// this one in order.
fn wait_for_quiet(request: &Request<'_>, tool: Tool) -> bool {
    let started = Instant::now();
    loop {
        if !input_busy(request.server, request.pane, tool)
            && !recently_viewed(request.server, request.pane)
        {
            return true;
        }
        if started.elapsed() >= request.defer {
            return false;
        }
        std::thread::sleep(DEFER_POLL);
    }
}

/// Is it UNSAFE to paste into this pane right now — `_paste_input_busy`?
///
/// An unmodelled tool is always safe: it has no reliable predicate, and failing
/// closed there would break every send to gemini, opencode and plain shells.
/// Fail-closed applies WITHIN a modelled tool.
#[must_use]
pub fn input_busy(server: &ServerId, pane: &str, tool: Tool) -> bool {
    if !tool.is_modelled() {
        return false;
    }
    read_occupancy(server, pane, tool) != Occupancy::Idle
}

/// Is our pasted message STILL STAGED — `_paste_still_staged`?
///
/// Only a positive OCCUPIED read counts. Idle and indeterminate both degrade to
/// "submitted", which is non-regressive: a false loud alarm here makes the
/// sender DUPLICATE an already-delivered message.
#[must_use]
pub fn still_staged(server: &ServerId, pane: &str, tool: Tool) -> bool {
    tool.is_modelled() && read_occupancy(server, pane, tool) == Occupancy::Occupied
}

/// Capture and read the pane's input box.
fn read_occupancy(server: &ServerId, pane: &str, tool: Tool) -> Occupancy {
    match transport::capture_screen(server, pane, Styling::Escapes) {
        Some(region) => region::occupancy(&region, tool),
        None => Occupancy::Unreadable,
    }
}

/// Is an attached client LOOKING AT this pane with recent input —
/// `_pane_recently_viewed`?
///
/// A client's `#{pane_id}` is the pane IT is viewing, which is stronger than
/// `#{pane_active}`; `#{client_activity}` is the epoch of its last INPUT, and
/// navigation counts, so a false positive is a safe deferral. This cannot prove
/// the input box is empty and cannot close the check-to-paste race — it narrows
/// the codex idle-typing gap, nothing more.
fn recently_viewed(server: &ServerId, pane: &str) -> bool {
    let Some(clients) = transport::observe_clients(server) else {
        return false;
    };
    let now = match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        Err(_) => return false,
    };
    clients.iter().any(|client| {
        client.pane == pane
            && client
                .activity
                .and_then(|epoch| i64::try_from(epoch).ok())
                .is_some_and(|epoch| now - epoch < VIEW_GRACE)
    })
}

/// Is this tool provably still starting up — `_spawn_input_ready`'s first
/// question, asked for every tool.
///
/// A tool that is provably still initializing is not ready however idle its
/// input box looks. The markers are NEGATIVE: their absence is not readiness,
/// and the input predicate still governs.
#[must_use]
pub fn tool_initializing(server: &ServerId, pane: &str, tool: Tool) -> bool {
    match transport::capture_screen(server, pane, Styling::Plain) {
        Some(capture) => region::initializing(&capture, tool),
        None => false,
    }
}

/// Whether `pane` is ready to be pasted into at launch or spawn time —
/// `_spawn_input_ready`.
///
/// AN IDLE INPUT BOX IS NOT AN INITIALIZED APPLICATION: the start-up question
/// is asked FIRST and for every tool, because a tool that is provably still
/// initializing is not ready however idle its box looks.
///
/// For a MODELLED tool the second question is the same structural predicate
/// the send path uses, so spawn cannot disagree with it: a booting TUI has no
/// live prompt (indeterminate, so unsafe, so not ready) and a modal is
/// occupied. An UNMODELLED tool has no predicate at all — [`input_busy`]
/// reports it safe unconditionally, which would mean "ready" the instant the
/// pane exists, i.e. the boot-gap paste this question exists to prevent — so
/// it keeps the composed-UI marker instead. That is weaker, and it is what it
/// had before.
#[must_use]
pub fn input_ready(server: &ServerId, pane: &str, tool: Tool) -> bool {
    if tool_initializing(server, pane, tool) {
        return false;
    }
    if tool.is_modelled() {
        return !input_busy(server, pane, tool);
    }
    transport::capture_screen(server, pane, Styling::Plain)
        .is_some_and(|screen| region::composed_ui(&screen))
}

/// How often the launch readiness wait re-reads the pane — the frozen
/// `_wait_input_ready`'s `sleep 0.5`.
const READY_POLL: Duration = Duration::from_millis(500);

/// Wait, bounded, until `pane` will accept a paste — the frozen
/// `_wait_input_ready`, whose `polls` this counts in the same units.
///
/// ONE bounded readiness wait, for both delivery moments. The spawn path had it
/// inline and the launch path had nothing at all; the same question deserves
/// the same answer wherever it is asked. A timeout is `false`, and the CALLER
/// must then leave the pane untouched: a brief is re-sendable, a clobbered
/// modal is not.
#[must_use]
pub fn wait_input_ready(server: &ServerId, pane: &str, tool: Tool, polls: u32) -> bool {
    for _ in 0..polls {
        if input_ready(server, pane, tool) {
            return true;
        }
        std::thread::sleep(READY_POLL);
    }
    false
}

/// Paste `text` into `pane` and press Enter, verifying the submit.
///
/// The LAUNCH-COMMAND paste, which is the one delivery in ae that legitimately
/// targets a SHELL: the pane has not started its agent yet, and the text is the
/// path of the launch script it must run. So none of [`deliver`]'s guards apply
/// — the dead-pane refusal exists precisely to keep a message out of a shell,
/// and here the shell is the intended reader.
///
/// Best effort by design, exactly as the frozen `tmux_paste_submit || true`
/// call site is: a shell is unmodelled, so [`still_staged`] cannot see anything
/// and the single Enter is all there is. The return value says whether the
/// staging and the Enter were accepted, never that the shell ran it.
#[must_use]
pub fn submit_shell_text(server: &ServerId, pane: &str, text: &str) -> bool {
    let buffer = buffer_name(pane);
    if !transport::load_buffer(server, &buffer, text.as_bytes())
        || !transport::paste_buffer(server, &buffer, pane)
    {
        return false;
    }
    std::thread::sleep(SETTLE_OTHER);
    transport::send_key(server, pane, Key::Enter)
}

/// Paste and CONFIRM the submit — `ae_submit_pasted_message`.
///
/// On a busy TUI a single Enter can be swallowed and the staged text later
/// clears WITHOUT sending — the founding exhibit is a review paste that never
/// reached a busy codex. So the Enter is verified, retried a bounded number of
/// times, and then FAILS LOUD.
/// Why a staged paste did not reach the pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageFailure {
    /// `load-buffer` refused: nothing is staged.
    Load,
    /// `paste-buffer` refused after the load: the staged buffer has been deleted.
    Paste,
}

/// Stage `bytes` and paste them into `pane`, and NEVER leave the bytes behind.
///
/// `paste-buffer -d` consumes the buffer only when it succeeds; a paste that
/// fails after a successful load (the pane died in between, the server refused)
/// left the body sitting in the server's buffer stack, readable by `save-buffer`
/// from any client — one leaked body per failed or raced delivery (colead gate
/// 0b570acd, deterministic repro). Every post-load exit deletes the buffer.
///
/// # Errors
///
/// [`StageFailure::Load`] when the bytes could not be staged (nothing to clean
/// up); [`StageFailure::Paste`] when the paste refused after the load — the
/// staged buffer has already been deleted.
pub fn stage_and_paste(
    server: &ServerId,
    buffer: &str,
    bytes: &[u8],
    pane: &str,
) -> Result<(), StageFailure> {
    if !transport::load_buffer(server, buffer, bytes) {
        return Err(StageFailure::Load);
    }
    if transport::paste_buffer(server, buffer, pane) {
        return Ok(());
    }
    let _ = transport::delete_buffer(server, buffer);
    Err(StageFailure::Paste)
}

fn submit(
    request: &Request<'_>,
    tool: Tool,
    payload: &str,
    mode: &notice::Mode,
    body_file: &str,
    err: &mut impl Write,
) -> io::Result<Result<(), Failure>> {
    let buffer = buffer_name(request.pane);
    let server = request.server;
    let pane = request.pane;
    match stage_and_paste(server, &buffer, payload.as_bytes(), pane) {
        Ok(()) => {}
        Err(StageFailure::Load) => {
            writeln!(
                err,
                "ae: paste transport FAILED for pane {pane} ({}) — could not stage {} bytes. Nothing was sent.",
                tool.as_str(),
                payload.chars().count()
            )?;
            return Ok(Err(Failure::Paste {
                body_file: body_file.to_owned(),
            }));
        }
        Err(StageFailure::Paste) => {
            writeln!(
                err,
                "ae: paste FAILED into pane {pane} ({}) — nothing was sent.",
                tool.as_str()
            )?;
            return Ok(Err(Failure::Paste {
                body_file: body_file.to_owned(),
            }));
        }
    }
    if let notice::Mode::Notice(pointer) = mode
        && !prove_notice(server, pane, tool, pointer, &buffer)
    {
        writeln!(
            err,
            "ae: notice UNCONFIRMED to pane {pane} ({}) — recovery body preserved at {body_file}. Nothing was submitted; re-send.",
            tool.as_str()
        )?;
        return Ok(Err(Failure::Unconfirmed {
            body_file: body_file.to_owned(),
            notice: true,
        }));
    }
    let settle = if tool == Tool::Claude {
        SETTLE_CLAUDE
    } else {
        SETTLE_OTHER
    };
    std::thread::sleep(settle);
    let _ = transport::send_key(server, pane, Key::Enter);
    for _ in 0..VERIFY_RETRIES {
        std::thread::sleep(VERIFY_POLL);
        if !still_staged(server, pane, tool) {
            return Ok(Ok(()));
        }
        let _ = transport::send_key(server, pane, Key::Enter);
    }
    std::thread::sleep(VERIFY_POLL);
    if !still_staged(server, pane, tool) {
        return Ok(Ok(()));
    }
    writeln!(
        err,
        "ae: submit UNCONFIRMED to pane {pane} ({}) — message may not have sent. Re-send.",
        tool.as_str()
    )?;
    Ok(Err(Failure::Unconfirmed {
        body_file: body_file.to_owned(),
        notice: false,
    }))
}

/// Prove the staged notice on screen before any Enter.
///
/// A notice is a delivery POINTER, so Enter is forbidden until the visible
/// input rows show the exact staged bytes. ONE clear-and-repaste is allowed,
/// and only when the clear itself is MEASURABLE: a changed capture is not
/// proof, because a footer redraw changes bytes while the old pointer stays
/// staged. Anything short of a positive empty read leaves the pane untouched —
/// in particular, no Enter.
fn prove_notice(server: &ServerId, pane: &str, tool: Tool, pointer: &str, buffer: &str) -> bool {
    for attempt in 0..2 {
        std::thread::sleep(NOTICE_POLL);
        let region = transport::capture_screen(server, pane, Styling::Escapes).unwrap_or_default();
        if notice::prove(tool, &region, pointer) {
            return true;
        }
        if attempt != 0 {
            return false;
        }
        if !clear_is_measurable(server, pane, tool) {
            return false;
        }
        if stage_and_paste(server, buffer, pointer.as_bytes(), pane).is_err() {
            return false;
        }
    }
    false
}

/// Did C-u demonstrably empty the input box — `_notice_clear_measurable`?
///
/// Requires the modelled sensor to report a POSITIVE EMPTY region. Indeterminate
/// and occupied both mean no.
fn clear_is_measurable(server: &ServerId, pane: &str, tool: Tool) -> bool {
    if !transport::send_key(server, pane, Key::ClearLine) {
        return false;
    }
    std::thread::sleep(NOTICE_POLL);
    match transport::capture_screen(server, pane, Styling::Escapes) {
        Some(region) if !region.is_empty() => region::occupancy(&region, tool) == Occupancy::Idle,
        _ => false,
    }
}

/// The tmux buffer this delivery stages in — the frozen `ae-send-$$`, made
/// per-pane as well as per-process so two concurrent deliveries from one
/// process cannot share one.
fn buffer_name(pane: &str) -> String {
    let sanitized: String = pane
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    format!("ae-send-{}-{sanitized}", std::process::id())
}

// ---- the recovery body ----------------------------------------------------

/// Publish the exact delivered text beside the session —
/// `ae_store_message_body`.
///
/// RECOVERABLE BY THE RECIPIENT, not just by the sender's memory. Event
/// summaries are capped at 200 characters by design, so a message that does not
/// survive delivery cannot be reconstructed from the log — which is what turned
/// two incidents into "please resend verbatim". The event points at this file.
///
/// ONE ARTIFACT PER DELIVERY, never per REF. An ask and its reply share a
/// request id BY DESIGN, so a ref-named file meant the reply TRUNCATED the ask's
/// body and the ask event's pointer silently resolved to the reply's text. A
/// record that can be overwritten is not a record. The ref stays in the NAME so
/// a request and its answer still correlate; uniqueness comes from an exclusive
/// create rather than from a timestamp two deliveries can share.
///
/// Mode 0600: this is the same material as the pane content, not world-readable
/// metadata.
///
/// # Errors
///
/// The directory, the temporary, its mode or its publication — each named.
pub fn store_body(
    dir: &Path,
    reference: &str,
    action: &str,
    body: &str,
) -> Result<PathBuf, String> {
    let messages = dir.join("messages");
    std::fs::create_dir_all(&messages)
        .map_err(|why| format!("could not create {} ({why})", messages.display()))?;
    let stem = if is_name_safe(reference) {
        reference.to_owned()
    } else {
        // The frozen fallback is a UTC stamp. Rendered from the same clock,
        // with the separators dropped so the stem stays inside the name
        // grammar above: `msg-20260903T101500Z`.
        let stamp = crate::time::Timestamp::now().to_string();
        format!(
            "msg-{}",
            stamp
                .chars()
                .filter(|ch| *ch != '-' && *ch != ':')
                .collect::<String>()
        )
    };
    let action = if is_name_safe(action) { action } else { "send" };
    let mut last = String::new();
    for attempt in 0..8u32 {
        let temp = messages.join(format!(
            "{stem}.{action}.{:06x}",
            unique_suffix().wrapping_add(u64::from(attempt)) & 0xff_ffff
        ));
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => file,
            Err(why) => {
                last = format!("could not allocate in {} ({why})", messages.display());
                continue;
            }
        };
        let written = file
            .write_all(body.as_bytes())
            .and_then(|()| file.set_permissions(mode_600()));
        drop(file);
        if let Err(why) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("could not write {} ({why})", temp.display()));
        }
        // Publish by hard-linking then unlinking the temporary name: `link`
        // fails when the final name already exists, so no delivery can clobber
        // an earlier body.
        let mut published = temp.clone().into_os_string();
        published.push(".txt");
        let final_path = PathBuf::from(published);
        if std::fs::hard_link(&temp, &final_path).is_ok() {
            let _ = std::fs::remove_file(&temp);
            return Ok(final_path);
        }
        let _ = std::fs::remove_file(&temp);
        last = format!("could not publish a unique path in {}", messages.display());
    }
    Err(last)
}

/// The name grammar the frozen store screens a ref and an action against.
fn is_name_safe(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

/// Mode 0600, as a `Permissions`.
fn mode_600() -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(0o600)
}

/// A per-call suffix: the pid mixed with the monotonic clock, which is what
/// `mktemp` bought and nothing here needs to be unpredictable for.
fn unique_suffix() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    u64::from(nanos) ^ (u64::from(std::process::id()) << 13)
}

/// Take the per-target lock — `ae_lock_target`.
///
/// Per PANE and global across sessions, so `send` and `interrupt` to one target
/// exclude each other wherever they were run from. `flock(2)` locks belong to
/// the open file description, so a bash helper holding this same path and this
/// one exclude each other too — which is what keeps the glue's copy safe while
/// it still exists.
fn lock_target(dir: &Path, pane: &str) -> Option<std::fs::File> {
    let root = dir.parent()?.join(".locks");
    std::fs::create_dir_all(&root).ok()?;
    let sanitized: String = pane
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    state::acquire(&root.join(format!("send-lock-{sanitized}")), LOCK_WAIT).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        Failure, Request, Shape, UNVERIFIED, buffer_name, frame, is_name_safe, store_body,
    };
    use crate::inventory::ServerId;

    fn request<'a>(actor: &'a str, body: &'a str, shape: Shape) -> Request<'a> {
        Request {
            dir: std::path::Path::new("/m/sessions/s"),
            server: &ServerId::Ambient,
            pane: "%3",
            logged_target: "worker",
            target_session: "s",
            pane_slot: "worker.0",
            own_session: "s",
            action: "send",
            reference: "",
            actor,
            body,
            shape,
            defer: super::DEFAULT_DEFER,
        }
    }

    #[test]
    fn the_envelope_is_added_here_so_a_sender_cannot_forge_its_own() {
        assert_eq!(
            frame(&request("cl:lead", "hello", Shape::Send)),
            "⟦ae:msg from cl:lead⟧\nhello"
        );
        assert_eq!(
            frame(&request("", "hello", Shape::Send)),
            format!("⟦ae:msg from {UNVERIFIED}⟧\nhello"),
            "an unbindable caller is MARKED, never left bare — bare is the human's signature"
        );
        // A sender's own header is inside the body, under the one this adds.
        assert_eq!(
            frame(&request(
                "cl:lead",
                "⟦ae:msg from someone-else⟧\nx",
                Shape::Send
            )),
            "⟦ae:msg from cl:lead⟧\n⟦ae:msg from someone-else⟧\nx"
        );
        assert_eq!(
            frame(&request("cl:lead", "stop", Shape::Interrupt)),
            "stop",
            "an interrupt is a control action, not transcript chat"
        );
    }

    #[test]
    fn a_failure_says_where_the_body_went_only_when_one_was_published() {
        assert_eq!(Failure::DeadPane.body_file(), "");
        assert_eq!(Failure::Storage.body_file(), "");
        assert_eq!(Failure::Lock.body_file(), "");
        assert_eq!(Failure::Abandoned.body_file(), "");
        assert_eq!(
            Failure::Unconfirmed {
                body_file: "/m/x.txt".into(),
                notice: true
            }
            .body_file(),
            "/m/x.txt"
        );
        assert_eq!(
            Failure::Paste {
                body_file: "/m/y.txt".into()
            }
            .body_file(),
            "/m/y.txt"
        );
        assert_eq!(
            Failure::NoticeRefused {
                body_file: "/m/z.txt".into()
            }
            .body_file(),
            "/m/z.txt"
        );
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a TEST reading back what the store wrote — the deny enumerates the PRODUCT's read doors"
    )]
    fn the_record_is_one_artifact_per_delivery_and_never_clobbers_an_earlier_one() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("ae-deliver-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // An ask and its reply SHARE a request id by design. A ref-named file
        // meant the reply truncated the ask's body, and the ask event's
        // pointer then resolved to the reply's text.
        let ask = store_body(&dir, "ae-1", "ask", "the question").unwrap();
        let reply = store_body(&dir, "ae-1", "reply", "the answer").unwrap();
        let second_ask = store_body(&dir, "ae-1", "ask", "again").unwrap();
        assert_ne!(ask, reply);
        assert_ne!(ask, second_ask, "two deliveries, two records");
        assert_eq!(std::fs::read_to_string(&ask).unwrap(), "the question");
        assert_eq!(std::fs::read_to_string(&reply).unwrap(), "the answer");
        for path in [&ask, &reply, &second_ask] {
            let name = path.file_name().and_then(std::ffi::OsStr::to_str).unwrap();
            assert!(
                name.starts_with("ae-1."),
                "the ref stays in the NAME: {name}"
            );
            assert!(
                std::path::Path::new(name)
                    .extension()
                    .is_some_and(|ext| ext == "txt"),
                "{name}"
            );
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600,
                "the same material as the pane content, not world-readable metadata"
            );
        }
        // A ref or action outside the name grammar does not reach the path.
        let hostile = store_body(&dir, "../../etc/x", "a/b", "body").unwrap();
        let name = hostile
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap();
        assert!(
            name.starts_with("msg-") && name.contains(".send."),
            "a ref that is not a name is stamped instead, and the action falls back: {name}"
        );
        assert_eq!(hostile.parent(), Some(dir.join("messages").as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_name_grammar_is_the_frozen_one() {
        assert!(is_name_safe("ae-20260903T093208Z-ab12cd34"));
        assert!(is_name_safe("review-1.2_3"));
        assert!(!is_name_safe(""));
        assert!(!is_name_safe("../escape"));
        assert!(!is_name_safe("has space"));
        assert!(!is_name_safe("colon:in:name"));
    }

    #[test]
    fn the_paste_buffer_is_named_per_process_and_per_pane() {
        let name = buffer_name("%12");
        assert!(
            name.starts_with("ae-send-") && name.ends_with("-_12"),
            "{name}"
        );
        assert_ne!(buffer_name("%12"), buffer_name("%13"));
    }
}
