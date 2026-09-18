//! Pane DELIVERY, in the core.
//!
//! The dead-pane refusal, the provenance envelope, the recovery-body store, the
//! per-target lock, the busy/human-presence deferral, the bracketed paste, the
//! oversize notice with its on-screen proof, and the submit verification.

pub mod notice;
pub mod region;

use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::inventory::ServerId;
use crate::tmux::{Key, Styling};
use crate::tool::{Composed, InputModel, ToolKind};
use crate::transport;
use region::Occupancy;

/// How long a send waits for a busy target before abandoning —
/// `AE_SEND_DEFER_SEC`'s default.
pub const DEFAULT_DEFER: Duration = Duration::from_secs(30);

/// How often the deferral loop re-reads the target.
const DEFER_POLL: Duration = Duration::from_millis(400);

/// A client's quiet period, past which it is no longer evidence of a human at
/// the pane.
const VIEW_GRACE: i64 = 4;

/// The pause between the paste and the Enter for a STYLE-delimited composer —
/// codex: no Enter-drop evidence there, and every send would pay the latency.
const SETTLE_STYLE_DELIMITED: Duration = Duration::from_millis(100);

/// The conservative pause: claude's border-delimited box (a 24-sample sweep
/// caught sends to IDLE panes losing the Enter) and EVERY unmodelled tool —
/// which takes the whole framed body at any size, the largest paste in the
/// product, and gets no input-box submit verification, so it takes the same
/// conservative pause.
const SETTLE_CONSERVATIVE: Duration = Duration::from_millis(300);

/// The pause between a shell line's paste and its Enter.
const SETTLE_SHELL_TEXT: Duration = Duration::from_millis(100);

/// The pause before each staged re-read after Enter.
const VERIFY_POLL: Duration = Duration::from_millis(300);

/// How many extra Enters the verification will send before giving up.
const VERIFY_RETRIES: usize = 2;

/// The pause before each notice proof attempt.
const NOTICE_POLL: Duration = Duration::from_millis(100);

/// The pause after an interrupt's cancel keys, before its message is pasted.
const INTERRUPT_SETTLE: Duration = Duration::from_millis(500);

/// How long the per-target lock is waited for.
const LOCK_WAIT: Duration = Duration::from_mins(2);

/// Which entry point is delivering, and therefore what its refusals say and
/// what it does to the pane before pasting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `send`, and the tracked requests behind it.
    Send,
    /// The orchestrator seat's privileged, unenveloped human-authority relay.
    Relay,
    /// `interrupt`.
    Interrupt,
    /// A spawn's BRIEF, pasted into a freshly launched TUI.
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
    /// The target pane's `@ae_slot`, or empty.
    pub pane_slot: &'a str,
    /// This session's name.
    pub own_session: &'a str,
    /// The event action the body store names the recovery file after.
    pub action: &'a str,
    /// The request id, or empty.
    pub reference: &'a str,
    /// The VERIFIED sender for the envelope.
    pub actor: &'a str,
    /// The message as composed, before framing.
    pub body: &'a str,
    /// Which entry point this is.
    pub shape: Shape,
    /// How long to wait for a busy target.
    pub defer: Duration,
    /// The COMPOSED-UI signal of the tool this delivery expects, read only by
    /// the Launch recheck under the target lock of an UNMODELLED pane.
    /// Callers of every other shape pass [`Composed::NONE`]; a modelled pane
    /// answers through its [`InputModel`] instead.
    pub composed: Composed,
}

/// A delivery that landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivered {
    /// The published recovery body — the event's `body_file`.
    pub body_file: String,
    /// Exactly what was framed and stored, which is what a pane send's event
    /// summary is of.
    pub framed: String,
    /// Whether ae verified the Enter reached the harness's input model.
    pub verification: DeliveryVerification,
}

/// How certain ae is that a pasted message submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryVerification {
    /// The input box was positively observed clear after Enter.
    Verified,
    /// Enter was sent, but ae could not prove what the pane did with it.
    Unverifiable(Unverifiable),
}

impl DeliveryVerification {
    /// The reason an auditable event record carries for an unverifiable submit.
    #[must_use]
    pub const fn unverifiable_marker(self) -> Option<&'static str> {
        match self {
            Self::Verified => None,
            Self::Unverifiable(reason) => Some(reason.event_marker()),
        }
    }
}

/// What observing a pasted message after Enter proved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitState {
    /// The input box was positively observed clear after Enter.
    Submitted,
    /// The input box was positively observed to still contain the paste.
    StillStaged,
    /// Enter was sent, but ae could not prove what the pane did with it.
    Unknown(Unverifiable),
}

/// Why a submit observation could not be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unverifiable {
    /// ae has no grammar for this tool's input box.
    Unmodelled,
    /// tmux did not return a screen capture.
    CaptureUnreadable,
    /// A capture arrived, but it did not contain a readable live input box.
    PaneUnparseable,
}

impl Unverifiable {
    /// Stable marker spelling for the event ledger.
    #[must_use]
    pub const fn event_marker(self) -> &'static str {
        match self {
            Self::Unmodelled => "unmodelled-input",
            Self::CaptureUnreadable => "unreadable-capture",
            Self::PaneUnparseable => "unparseable-input",
        }
    }
}

/// Why a delivery did not land.
#[derive(Clone, PartialEq, Eq)]
pub enum Failure {
    /// The target pane is a shell, not a running agent.
    DeadPane,
    /// The under-lock pane probe could not prove a live agent (absent, or
    /// naming no pid), so NOTHING was pasted — fail closed.
    Unproven {
        /// The published recovery body, still readable.
        body_file: String,
    },
    /// The recovery body could not be published.
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
    /// The paste itself failed.
    Paste {
        /// The published recovery body, still readable.
        body_file: String,
    },
    /// The pane left its composed input box between the readiness proof and the
    /// target lock; NOTHING was pasted. Death is a different failure
    /// ([`Failure::DeadPane`]) and is checked first.
    NotComposed {
        /// The published recovery body, still readable.
        body_file: String,
    },
    /// The submit was never confirmed.
    Unconfirmed {
        /// The published recovery body, still readable.
        body_file: String,
        /// Exactly what was framed before submit verification failed. For a
        /// notice arm, the pointer is the payload pasted on screen; this is
        /// the original framed body preserved by the recovery record.
        framed: String,
        /// Whether the failure was the notice's on-screen proof — the arm that
        /// records a `delivery-failed` event.
        notice: bool,
    },
}

impl fmt::Debug for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeadPane => formatter.write_str("DeadPane"),
            Self::Unproven { body_file } => formatter
                .debug_struct("Unproven")
                .field("body_file", body_file)
                .finish(),
            Self::Storage => formatter.write_str("Storage"),
            Self::Lock => formatter.write_str("Lock"),
            Self::NoticeRefused { body_file } => formatter
                .debug_struct("NoticeRefused")
                .field("body_file", body_file)
                .finish(),
            Self::Abandoned => formatter.write_str("Abandoned"),
            Self::Paste { body_file } => formatter
                .debug_struct("Paste")
                .field("body_file", body_file)
                .finish(),
            Self::NotComposed { body_file } => formatter
                .debug_struct("NotComposed")
                .field("body_file", body_file)
                .finish(),
            Self::Unconfirmed {
                body_file, notice, ..
            } => formatter
                .debug_struct("Unconfirmed")
                .field("body_file", body_file)
                .field("notice", notice)
                .finish(),
        }
    }
}

impl Failure {
    /// The recovery body, where one was published before the failure.
    #[must_use]
    pub fn body_file(&self) -> &str {
        match self {
            Self::DeadPane | Self::Storage | Self::Lock | Self::Abandoned => "",
            Self::NoticeRefused { body_file }
            | Self::Paste { body_file }
            | Self::NotComposed { body_file }
            | Self::Unproven { body_file }
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
    let input = target_input(request, &probe.command);
    // Interpreted-sink guard: refuse to paste into a pane whose agent has DIED
    // and dropped to a shell — a stray Enter would EXECUTE the message as a
    // shell command.
    if refuses_as_dead(pane_liveness_at(
        &target_meta_dir(request),
        request.pane_slot,
        &probe,
    )) {
        writeln!(err, "{}", dead_pane_line(request))?;
        return Ok(Err(Failure::DeadPane));
    }
    let framed = frame(request);
    if relay_oversize_refused(request, &framed, err)? {
        return Ok(Err(Failure::NoticeRefused {
            body_file: String::new(),
        }));
    }
    // Publish the exact recoverable pane text BEFORE locking or submitting it.
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
        input.model,
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
    if let Err(failure) = quiet_or_abandoned(request, input.model, err)? {
        return Ok(Err(failure));
    }
    if let Err(failure) = launch_recheck(request, input, &body_file, err)? {
        return Ok(Err(failure));
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
    match submit(request, input, payload, &mode, &body_file, &framed, err)? {
        Ok(verification) => Ok(Ok(Delivered {
            body_file,
            framed,
            verification,
        })),
        Err(failure) => {
            // The submit's own line said WHICH step failed; this one names the
            // delivery and where its body is.
            match request.shape {
                Shape::Send => writeln!(
                    err,
                    "ae: send to {} UNCONFIRMED — submit not verified; body preserved at {body_file}.",
                    request.logged_target
                )?,
                Shape::Relay => writeln!(
                    err,
                    "ae: relay to {} UNCONFIRMED — submit not verified; body preserved at {body_file}.",
                    request.logged_target
                )?,
                Shape::Interrupt => writeln!(
                    err,
                    "ae: interrupt message to {} UNCONFIRMED — submit not verified; body preserved at {body_file}. Re-send.",
                    request.logged_target
                )?,
                Shape::Launch => writeln!(
                    err,
                    "ae: spawn brief to {} UNCONFIRMED — submit not verified; body preserved at {body_file}. Re-send.",
                    request.logged_target
                )?,
            }
            Ok(Err(failure))
        }
    }
}

/// Refuse an oversize relay before its body reaches the recovery-notice path.
fn relay_oversize_refused(
    request: &Request<'_>,
    framed: &str,
    err: &mut impl Write,
) -> io::Result<bool> {
    // A relay promises exact bare text. A pointer would change the text and
    // expose the orchestrator to the target.
    if request.shape != Shape::Relay || framed.len() as u64 <= notice::LIMIT {
        return Ok(false);
    }
    writeln!(
        err,
        "ae: relay to {} REFUSED — message is {} B; verbatim relay limit is {} B. Nothing was sent.",
        request.logged_target,
        framed.len(),
        notice::LIMIT
    )?;
    Ok(true)
}

/// What an unbindable caller's envelope says.
pub const UNVERIFIED: &str = "unverified";

/// The message as it reaches the pane. Every ae-originated turn is marked on
/// its first line by [`crate::provenance`], with the verb that names its own
/// authority; `relay` alone stays bare because it carries human authority.
fn frame(request: &Request<'_>) -> String {
    let actor = envelope_actor(request);
    match request.shape {
        Shape::Send => crate::provenance::first_line(&crate::provenance::peer(actor), request.body),
        Shape::Relay => request.body.to_owned(),
        Shape::Interrupt => {
            crate::provenance::first_line(&crate::provenance::interrupt(actor), request.body)
        }
        Shape::Launch => {
            crate::provenance::first_line(&crate::provenance::brief(actor), request.body)
        }
    }
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
        Shape::Relay => format!(
            "ae: relay to {} REFUSED — target pane is a shell, not a running agent. Nothing was sent.",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TargetInput {
    model: InputModel,
    diagnostic: &'static str,
}

/// The input-box grammar this pane draws — `ae_target_tool`.
fn target_input(request: &Request<'_>, command: &str) -> TargetInput {
    let recorded = recorded_binary(&target_meta_dir(request), request.pane_slot);
    choose_input(&recorded, command)
}

fn choose_input(recorded: &str, command: &str) -> TargetInput {
    let recorded = ToolKind::from_binary_name(recorded).adapter();
    let adapter = if recorded.input.model.is_modelled() {
        recorded
    } else {
        ToolKind::from_binary_name(command).adapter()
    };
    TargetInput {
        model: adapter.input.model,
        diagnostic: if adapter.input.model.is_modelled() {
            adapter.name
        } else {
            "other"
        },
    }
}

/// `agent_bin.<slot>` out of the meta in `dir`, or empty.
pub(crate) fn recorded_binary(dir: &Path, slot: &str) -> String {
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
/// one under the same sessions root.
fn target_meta_dir(request: &Request<'_>) -> PathBuf {
    if request.target_session.is_empty() || request.target_session == request.own_session {
        return request.dir.to_path_buf();
    }
    match request.dir.parent() {
        Some(root) => root.join(request.target_session),
        None => request.dir.to_path_buf(),
    }
}

/// What the pane-level liveness observation says.
///
/// THREE states, because a boolean collapsed "cannot tell" into "alive": a
/// shell in the foreground with an unusable snapshot, an unreadable recorded
/// binary or no pid is UNPROVEN, and each caller decides policy explicitly
/// ([`refuses_as_dead`] for ordinary delivery, [`under_lock_refusal`] for the
/// unmodelled Launch re-proof).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaneLiveness {
    /// A real process owns the foreground, or the agent is a proven
    /// descendant of the pane's pid.
    Alive,
    /// A shell owns the foreground and the recorded agent is PROVEN gone.
    Dead,
    /// A shell owns the foreground and no proof either way can be made.
    Unproven,
}

/// What a refusal a liveness answer maps to, when it maps to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LivenessRefusal {
    Dead,
    Unproven,
}

/// The liveness decision, PURE: a pid-less probe is UNPROVEN whatever the
/// foreground says — a probe ae cannot fully read proves nothing — and only a
/// NAMED pid lets a non-shell foreground count as Alive. A shell foreground is
/// then proven only by a usable recorded binary and a process walk that is not
/// Unknown.
pub(crate) const fn observed_liveness(
    shell_in_foreground: bool,
    binary_known: bool,
    pid: Option<u32>,
    walk: crate::procs::Descendancy,
) -> PaneLiveness {
    if pid.is_none() {
        return PaneLiveness::Unproven;
    }
    if !shell_in_foreground {
        return PaneLiveness::Alive;
    }
    if !binary_known {
        return PaneLiveness::Unproven;
    }
    match walk {
        crate::procs::Descendancy::Absent => PaneLiveness::Dead,
        crate::procs::Descendancy::Present => PaneLiveness::Alive,
        crate::procs::Descendancy::Unknown => PaneLiveness::Unproven,
    }
}

/// The pane-level liveness observation — the one owner `_pane_agent_is_dead`
/// grew three states out of. Computes the raw readings and hands them to the
/// pure [`observed_liveness`], so every branch of the decision is pinnable.
fn pane_liveness_at(
    meta_dir: &Path,
    slot: &str,
    probe: &crate::tmux::ObservedPaneProbe,
) -> PaneLiveness {
    let shell_in_foreground = crate::watchdog::command_is_shell(&probe.command);
    let binary = recorded_binary(meta_dir, slot);
    let binary_known = !binary.is_empty() && !crate::watchdog::command_is_shell(&binary);
    let walk = match probe.pid {
        Some(pid) => crate::procs::descendancy(crate::procs::snapshot().as_deref(), pid, &binary),
        None => crate::procs::Descendancy::Unknown,
    };
    observed_liveness(shell_in_foreground, binary_known, probe.pid, walk)
}

/// Observe a pane's liveness OUTSIDE a delivery, for a caller that must not
/// advise a send without proof (the spawn's failure recovery). An
/// unobservable pane is Unproven, never Alive.
pub(crate) fn observe_pane_liveness(
    server: &ServerId,
    meta_dir: &Path,
    pane: &str,
    slot: &str,
) -> PaneLiveness {
    match transport::observe_pane_probe(server, pane) {
        Some(probe) => pane_liveness_at(meta_dir, slot, &probe),
        None => PaneLiveness::Unproven,
    }
}

/// ORDINARY delivery's policy (the pre-lock check, Send, Relay, Interrupt):
/// only a PROVEN dead pane refuses. `Unproven` keeps the standing fail-open,
/// because a transient process-snapshot failure must not refuse every
/// delivery.
const fn refuses_as_dead(liveness: PaneLiveness) -> bool {
    matches!(liveness, PaneLiveness::Dead)
}

/// The UNDER-LOCK unmodelled Launch policy: the pane must be PROVEN alive.
/// `Dead` refuses as a dead pane, `Unproven` refuses as unproven; only
/// `Alive` may take the paste.
const fn under_lock_refusal(liveness: PaneLiveness) -> Option<LivenessRefusal> {
    match liveness {
        PaneLiveness::Alive => None,
        PaneLiveness::Dead => Some(LivenessRefusal::Dead),
        PaneLiveness::Unproven => Some(LivenessRefusal::Unproven),
    }
}

/// Wait until the target's input box is safe to paste into, or give up.
///
/// A box busy ONLY because of an ae-staged paste chip is not a human draft:
/// once, and bounded, the wait drains it ([`flush_staged_chip`]) instead of
/// deferring the whole budget; a chip plus other text still defers.
fn wait_for_quiet(request: &Request<'_>, model: InputModel) -> bool {
    let started = Instant::now();
    let mut flushed = false;
    loop {
        let busy = input_busy(request.server, request.pane, model);
        if !busy && !recently_viewed(request.server, request.pane) {
            return true;
        }
        if busy && !flushed && !recently_viewed(request.server, request.pane) {
            flushed = true;
            let budget = request.defer.saturating_sub(started.elapsed());
            let _ = flush_staged_chip(request.server, request.pane, model, budget);
        }
        if started.elapsed() >= request.defer {
            return false;
        }
        std::thread::sleep(DEFER_POLL);
    }
}

/// Is the composer holding NOTHING but a staged paste chip ae can own?
fn staged_chip_present(server: &ServerId, pane: &str, model: InputModel) -> bool {
    transport::capture_screen(server, pane, Styling::Escapes)
        .is_some_and(|region| region::staged_paste(&region, model))
}

/// Submit a staged chip ONCE, after the pane has settled.
///
/// Waiting for the pane to stop redrawing is the recognizable "the tool is no
/// longer answering" state; then one Enter drains the chip — never a second
/// paste over it. `None` when the settle wait ran out, so the write is bounded.
/// Residue: a human's own untouched chip would drain too, which is why no human
/// attention may be recent at the call sites.
fn flush_staged_chip(
    server: &ServerId,
    pane: &str,
    model: InputModel,
    budget: Duration,
) -> Option<SubmitState> {
    let started = Instant::now();
    let mut previous = transport::capture_screen(server, pane, Styling::Plain);
    loop {
        std::thread::sleep(VERIFY_POLL);
        let current = transport::capture_screen(server, pane, Styling::Plain);
        if pane_settled(previous.as_deref(), current.as_deref()) {
            break;
        }
        if started.elapsed() >= budget {
            return None;
        }
        previous = current;
    }
    match still_staged(server, pane, model) {
        // Re-read the SHAPE, not just the verdict: a draft typed under the chip
        // is not ours to submit, and the chip may have drained on its own.
        SubmitState::StillStaged if staged_chip_present(server, pane, model) => {
            let _ = transport::send_key(server, pane, Key::Enter);
            std::thread::sleep(VERIFY_POLL);
            Some(still_staged(server, pane, model))
        }
        observed => Some(observed),
    }
}

/// Is it UNSAFE to paste into this pane right now — `_paste_input_busy`?
#[must_use]
pub fn input_busy(server: &ServerId, pane: &str, model: InputModel) -> bool {
    if !model.is_modelled() {
        return false;
    }
    read_occupancy(server, pane, model) != Occupancy::Idle
}

/// Observe whether our pasted message left the input box —
/// `_paste_still_staged`.
#[must_use]
pub fn still_staged(server: &ServerId, pane: &str, model: InputModel) -> SubmitState {
    if !model.is_modelled() {
        return SubmitState::Unknown(Unverifiable::Unmodelled);
    }
    match transport::capture_screen(server, pane, Styling::Escapes) {
        Some(region) if region::queued_submission(&region, model) => SubmitState::Submitted,
        Some(region) => match region::occupancy(&region, model) {
            Occupancy::Idle => SubmitState::Submitted,
            Occupancy::Occupied => SubmitState::StillStaged,
            Occupancy::Unreadable => SubmitState::Unknown(Unverifiable::PaneUnparseable),
        },
        None => SubmitState::Unknown(Unverifiable::CaptureUnreadable),
    }
}

/// Capture and read the pane's input box.
fn read_occupancy(server: &ServerId, pane: &str, model: InputModel) -> Occupancy {
    match transport::capture_screen(server, pane, Styling::Escapes) {
        Some(region) => region::occupancy(&region, model),
        None => Occupancy::Unreadable,
    }
}

/// Is an attached client LOOKING AT this pane with recent input —
/// `_pane_recently_viewed`?
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
#[must_use]
pub fn tool_initializing(server: &ServerId, pane: &str, model: InputModel) -> bool {
    match transport::capture_screen(server, pane, Styling::Plain) {
        Some(capture) => region::initializing(&capture, model),
        None => false,
    }
}

/// Whether `pane` is ready to be pasted into at launch or spawn time —
/// `_spawn_input_ready`.
///
/// This is a ONE-OBSERVATION predicate, so it answers only for a MODELLED
/// composer. An unmodelled tool has no grammar to recognise readiness from;
/// for it readiness is COMPOSED *and* SETTLED — positive evidence its box is
/// drawn, over two byte-identical captures — observed by [`wait_input_ready`].
/// Called with an unmodelled model this fails closed.
#[must_use]
pub fn input_ready(server: &ServerId, pane: &str, model: InputModel) -> bool {
    if !model.is_modelled() {
        return false;
    }
    if tool_initializing(server, pane, model) {
        return false;
    }
    !input_busy(server, pane, model)
}

/// How often the launch readiness wait re-reads the pane.
const READY_POLL: Duration = Duration::from_millis(500);

/// Wait, bounded, until `pane` will accept a paste; `polls` counts
/// [`READY_POLL`] periods.
///
/// A modelled pane is asked [`input_ready`] on each poll. An UNMODELLED one is
/// asked whether it is composed *and* settled ([`wait_until_settled`]) against
/// the tool's own composed markers; with no markers it can never be ready and
/// the caller REFUSES visibly, as it did before the markers were withdrawn.
#[must_use]
pub fn wait_input_ready(
    server: &ServerId,
    pane: &str,
    model: InputModel,
    composed: Composed,
    polls: u32,
) -> bool {
    if !model.is_modelled() {
        return wait_until_settled(server, pane, composed, polls);
    }
    for _ in 0..polls {
        if input_ready(server, pane, model) {
            return true;
        }
        std::thread::sleep(READY_POLL);
    }
    false
}

/// Wait, bounded, until an UNMODELLED pane is COMPOSED and SETTLED.
///
/// Stability alone is never readiness: a blank or splash frame can be
/// byte-identical for seconds while the tool is still initialising, and a
/// paste into it is silently lost (measured on opencode 1.18.31, 2026-09-15 —
/// blank until ~+3.0 s, and a paste at +1 s vanished with the tool reporting
/// success). One capture seeds the comparison; every following [`READY_POLL`]
/// takes another, and readiness is granted only when that pair is
/// byte-identical AND the seeded capture carries one of the tool's composed
/// markers ([`unmodelled_ready`]).
///
/// A tool with NO signal ([`Composed::NONE`]) can never be ready here —
/// the wait refuses at once and the caller refuses visibly. That is the
/// standing behaviour for gemini and unknown: they are unmodelled and carry
/// no usable composed signal. agy and grok carry a measured signal
/// (see the `Composed` rows in `src/tool.rs`); the wait grants readiness
/// only on their drawn structure, stable over two captures.
#[must_use]
fn wait_until_settled(server: &ServerId, pane: &str, composed: Composed, polls: u32) -> bool {
    if composed.is_empty() {
        // No composed signal can ever pass here: refuse NOW rather than burn
        // the whole budget on an answer that is already known.
        return false;
    }
    let mut previous = transport::capture_screen(server, pane, Styling::Plain);
    for _ in 1..polls {
        std::thread::sleep(READY_POLL);
        let current = transport::capture_screen(server, pane, Styling::Plain);
        if unmodelled_ready(previous.as_deref(), current.as_deref(), composed) {
            return true;
        }
        previous = current;
    }
    false
}

/// The whole readiness verdict for one unmodelled capture pair: the seeded
/// frame carries a COMPOSED marker, and the two frames are byte-identical and
/// non-empty ([`pane_settled`]). Pure, so both halves are pinnable without
/// tmux — and the composed half is the one `e737b6b3` dropped, which let a
/// blank boot frame pass as ready.
fn unmodelled_ready(before: Option<&str>, after: Option<&str>, composed: Composed) -> bool {
    before.is_some_and(|capture| region::composed_ui(capture, composed))
        && pane_settled(before, after)
}

/// Re-read the pane's SCREEN once, under the target lock, and answer whether
/// its composed box is still there. A failed capture and a marker-less screen
/// answer false. This reads the screen ONLY: an agent that died leaves its
/// composer DRAWN above the returning shell prompt and still matches, so it
/// is never the only guard — [`launch_recheck`] runs the pane-level liveness
/// owner ([`pane_liveness_at`]) first, exactly as the pre-lock path does.
fn reconfirm_composed(server: &ServerId, pane: &str, composed: Composed) -> bool {
    transport::capture_screen(server, pane, Styling::Plain)
        .is_some_and(|capture| region::composed_ui(&capture, composed))
}

/// The Send/Relay quiet gate: a busy target (or a human's attention on it)
/// abandons after the deferral, with nothing pasted. Every other shape
/// proceeds — and so does an unmodelled target, whose `input_busy` is false.
fn quiet_or_abandoned(
    request: &Request<'_>,
    model: InputModel,
    err: &mut impl Write,
) -> io::Result<Result<(), Failure>> {
    if !matches!(request.shape, Shape::Send | Shape::Relay) || wait_for_quiet(request, model) {
        return Ok(Ok(()));
    }
    writeln!(
        err,
        "ae: {} to {} ABANDONED — target stayed busy / human input or attention (not clear within {}s; AE_SEND_DEFER_SEC overrides). Re-send.",
        request.action,
        request.logged_target,
        request.defer.as_secs()
    )?;
    Ok(Err(Failure::Abandoned))
}

/// The under-lock unproven refusal: an absent probe, a pid-less probe, an
/// unreadable recorded binary or an unusable process walk all land here, and
/// NOTHING is pasted.
fn unproven_refusal(
    request: &Request<'_>,
    body_file: &str,
    err: &mut impl Write,
) -> io::Result<Failure> {
    writeln!(
        err,
        "ae: brief for {} REFUSED — the pane could not be proven a live agent under the target lock; NOTHING was pasted. Body preserved at {body_file}; re-send once the pane is observable.",
        request.logged_target
    )?;
    Ok(Failure::Unproven {
        body_file: body_file.to_owned(),
    })
}

/// The under-lock Launch re-proof: `Ok(Ok(()))` lets the paste proceed, and
/// the `Err` arm is a visible refusal with nothing pasted. Only an UNMODELLED
/// Launch is re-proven — every other shape and every modelled pane keeps the
/// behaviour it had.
///
/// Two questions, in this order: is the PANE still a live agent — the one
/// [`pane_liveness_at`] owner, whose `Dead` and `Unproven` both refuse here
/// ([`under_lock_refusal`]), because a dead or unknowable agent's stale
/// composer would still satisfy the screen read below — and does its SCREEN
/// still show the composed box ([`reconfirm_composed`]).
fn launch_recheck(
    request: &Request<'_>,
    input: TargetInput,
    body_file: &str,
    err: &mut impl Write,
) -> io::Result<Result<(), Failure>> {
    if request.shape != Shape::Launch || input.model.is_modelled() {
        return Ok(Ok(()));
    }
    let Some(probe) = transport::observe_pane_probe(request.server, request.pane) else {
        return Ok(Err(unproven_refusal(request, body_file, err)?));
    };
    match under_lock_refusal(pane_liveness_at(
        &target_meta_dir(request),
        request.pane_slot,
        &probe,
    )) {
        None => {}
        Some(LivenessRefusal::Dead) => {
            writeln!(err, "{}", dead_pane_line(request))?;
            writeln!(err, "ae: the recovery body is preserved at {body_file}.")?;
            return Ok(Err(Failure::DeadPane));
        }
        Some(LivenessRefusal::Unproven) => {
            return Ok(Err(unproven_refusal(request, body_file, err)?));
        }
    }
    if !reconfirm_composed(request.server, request.pane, request.composed) {
        writeln!(
            err,
            "ae: brief for {} REFUSED — the pane left its composed input box while the target lock was held; NOTHING was pasted. Body preserved at {body_file}; re-send once the pane is composed.",
            request.logged_target
        )?;
        return Ok(Err(Failure::NotComposed {
            body_file: body_file.to_owned(),
        }));
    }
    Ok(Ok(()))
}

/// Paste `text` into `pane` and press Enter, verifying the submit.
#[must_use]
pub fn submit_shell_text(server: &ServerId, pane: &str, text: &str) -> bool {
    // Through the same door the message path uses, so a paste that fails after
    // the load does not leave the staged bytes readable by `save-buffer` from
    // any client on the server.
    if stage_and_paste(server, &buffer_name(pane), text.as_bytes(), pane).is_err() {
        return false;
    }
    std::thread::sleep(SETTLE_SHELL_TEXT);
    transport::send_key(server, pane, Key::Enter)
}

/// Paste and CONFIRM the submit — `ae_submit_pasted_message`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageFailure {
    /// `load-buffer` refused: nothing is staged.
    Load,
    /// `paste-buffer` refused after the load: the staged buffer has been deleted.
    Paste,
}

/// Stage `bytes` and paste them into `pane`, and NEVER leave the bytes behind.
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
    input: TargetInput,
    payload: &str,
    mode: &notice::Mode,
    body_file: &str,
    framed: &str,
    err: &mut impl Write,
) -> io::Result<Result<DeliveryVerification, Failure>> {
    let model = input.model;
    let diagnostic = input.diagnostic;
    let buffer = buffer_name(request.pane);
    let server = request.server;
    let pane = request.pane;
    match stage_and_paste(server, &buffer, payload.as_bytes(), pane) {
        Ok(()) => {}
        Err(StageFailure::Load) => {
            writeln!(
                err,
                "ae: paste transport FAILED for pane {pane} ({diagnostic}) — could not stage {} bytes. Nothing was sent.",
                payload.chars().count()
            )?;
            return Ok(Err(Failure::Paste {
                body_file: body_file.to_owned(),
            }));
        }
        Err(StageFailure::Paste) => {
            writeln!(
                err,
                "ae: paste FAILED into pane {pane} ({diagnostic}) — nothing was sent."
            )?;
            return Ok(Err(Failure::Paste {
                body_file: body_file.to_owned(),
            }));
        }
    }
    if let notice::Mode::Notice(pointer) = mode
        && !prove_notice(server, pane, model, pointer, &buffer)
    {
        writeln!(
            err,
            "ae: notice UNCONFIRMED to pane {pane} ({diagnostic}) — recovery body preserved at {body_file}. Nothing was submitted; re-send."
        )?;
        return Ok(Err(Failure::Unconfirmed {
            body_file: body_file.to_owned(),
            framed: framed.to_owned(),
            notice: true,
        }));
    }
    match submit_staged(server, pane, model) {
        SubmitState::Submitted => return Ok(Ok(DeliveryVerification::Verified)),
        SubmitState::Unknown(reason) => {
            return Ok(Ok(DeliveryVerification::Unverifiable(reason)));
        }
        SubmitState::StillStaged => {}
    }
    // The box took the paste. If it still holds exactly an ae-staged chip, the
    // harness refused the TURN (Muse's "turn-submit backlog full"): the same
    // bounded retry the quiet gate uses, so a spawn's brief lands without an
    // interrupt.
    if staged_chip_present(server, pane, model) {
        match flush_staged_chip(server, pane, model, request.defer) {
            Some(SubmitState::Submitted) => return Ok(Ok(DeliveryVerification::Verified)),
            Some(SubmitState::Unknown(reason)) => {
                return Ok(Ok(DeliveryVerification::Unverifiable(reason)));
            }
            Some(SubmitState::StillStaged) | None => {}
        }
    }
    if matches!(request.shape, Shape::Send | Shape::Relay) {
        writeln!(
            err,
            "ae: submit UNCONFIRMED to pane {pane} ({diagnostic}) — message may not have sent."
        )?;
    } else {
        writeln!(
            err,
            "ae: submit UNCONFIRMED to pane {pane} ({diagnostic}) — message may not have sent. Re-send."
        )?;
    }
    Ok(Err(Failure::Unconfirmed {
        body_file: body_file.to_owned(),
        framed: framed.to_owned(),
        notice: false,
    }))
}

/// The settle an input model takes between the paste and the Enter.
fn settle_for(model: InputModel) -> Duration {
    if model == InputModel::StyleDelimited {
        SETTLE_STYLE_DELIMITED
    } else {
        SETTLE_CONSERVATIVE
    }
}

/// Press Enter, then observe whether the paste left the input box.
///
/// A booting TUI swallows the Enter often enough that a single bare press is
/// not a submit — it is a hope. Measured live on 2026-09-04: two codex seats
/// resumed at once, one Enter took and the other left its turn sitting in the
/// box. Every first-message delivery presses through here so the retry and the
/// verdict have ONE owner.
#[must_use]
pub fn submit_staged(server: &ServerId, pane: &str, model: InputModel) -> SubmitState {
    std::thread::sleep(settle_for(model));
    let _ = transport::send_key(server, pane, Key::Enter);
    for retry in 0..=VERIFY_RETRIES {
        std::thread::sleep(VERIFY_POLL);
        let observed = still_staged(server, pane, model);
        match observed {
            SubmitState::Submitted | SubmitState::Unknown(_) => return observed,
            SubmitState::StillStaged if retry == VERIFY_RETRIES => return SubmitState::StillStaged,
            SubmitState::StillStaged => {
                let _ = transport::send_key(server, pane, Key::Enter);
            }
        }
    }
    SubmitState::StillStaged
}

// ---- the guarded operation ---------------------------------------------------

/// One guarded paste: everything [`deliver()`] waits for, then the caller's
/// identity proof under the lifecycle lock, then an instant re-proof, the
/// paste and a bounded submit. R10's one owner — the caller duplicates none.
#[derive(Debug, Clone)]
pub struct GuardedRequest<'a> {
    /// The session meta directory: the send-lock root and the meta the
    /// pre-lock dead-pane guard reads.
    pub dir: &'a Path,
    /// The server the TARGET is on, not the caller's ambient one.
    pub server: &'a ServerId,
    /// The target pane id.
    pub pane: &'a str,
    /// The target pane's `@ae_slot`, or empty.
    pub pane_slot: &'a str,
    /// The target's session — which session's meta the dead-pane guard reads.
    pub target_session: &'a str,
    /// This session's name.
    pub own_session: &'a str,
    /// The target's input-box grammar.
    pub model: InputModel,
    /// The exact bytes to paste — verbatim, no envelope, no notice arm.
    pub text: &'a str,
    /// How long to wait for a busy target before abandoning.
    pub defer: Duration,
}

/// Why the guarded operation pasted nothing. Every leg is a constant, never
/// the offending value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leg {
    /// A shell owns the foreground: pre-lock PROVEN gone through meta+`ps`
    /// (the [`deliver()`] policy), under the lock read from the probe alone —
    /// and an unreadable probe fails closed to this same leg. Nothing typed.
    Dead,
    /// The target stayed busy, or human attention stayed on it: the full
    /// deferral pre-lock, one snapshot under it. Nothing typed.
    Busy,
    /// Another delivery held the pane's send-lock for the whole wait.
    TargetLocked,
    /// The lifecycle lock could not be taken — returned by the caller's
    /// `prove`, never by the operation itself.
    LifecycleLocked,
    /// Staging or pasting failed under the lock; a staged-then-refused buffer
    /// was already deleted, so no byte reached the pane.
    PasteFailed,
    /// Identity legs — constructed ONLY by the caller's `prove` closure,
    /// never by the operation itself. The live identity read could not be
    /// made.
    Unreadable,
    /// The seat's live identity is vacant where the carried facts name it.
    Vacant,
    /// The live identity differs from the carried facts.
    Mismatch,
    /// A coherent same-name replacement holds the seat: the live occupant is
    /// not ours, and is left untouched.
    Live,
}

/// What the guarded operation did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing was pasted, for the named leg.
    Skipped(Leg),
    /// The bytes were pasted and Enter was sent: the RAW submit verdict.
    Sent(SubmitState),
}

/// The KNOWN submit failure: `send_key` refused an Enter. Matched at the one
/// call site into `not dispatched (enter failed)` — never `?`-propagated,
/// never `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnterFailed;

/// Paste `request`'s bytes through the lifecycle lock the caller's `prove`
/// takes — R10's ONE guarded operation.
///
/// Steps: (1) every long wait first — the [`deliver()`] dead-pane refusal
/// (meta+`ps`), the pane's send-lock, the busy/human-input deferral; (2) the
/// caller's `prove`, which takes the lifecycle lock and re-proves identity,
/// returning the held lock or a leg; (3) under that lock, the operation's OWN
/// instant delivery-safety variants — one pane probe, one busy snapshot, no
/// loop, no file, no process; (4) the paste ([`stage_and_paste`]) and the
/// BOUNDED submit ([`submit_bounded`]); (5) the lifecycle lock drops, (6) then
/// the send-lock.
///
/// What step (1) deliberately OMITS from [`deliver()`]: the envelope (the
/// bytes paste verbatim), the recovery-body store (the caller's checkpoint is
/// the durability), the notice arm (the caller composes exact bytes) and the
/// Launch-only re-proof. No `wait_input_ready`: the seat proved itself
/// interactive before the call, and the deferral below owns busy.
///
/// # Errors
/// Returns `Err(EnterFailed)` when `send_key` refuses an Enter during the
/// bounded submit — the paste is already staged, so this is `not dispatched`,
/// never a skip and never `Unknown`.
pub fn deliver_guarded(
    request: &GuardedRequest<'_>,
    prove: impl FnOnce() -> Result<std::fs::File, Leg>,
) -> Result<Outcome, EnterFailed> {
    // The `deliver()`-shaped view the guard owners below take. They read
    // `dir`, `server`, `pane`, `defer` and the meta-routing triple only; the
    // envelope fields stay empty because this operation frames nothing.
    let view = Request {
        dir: request.dir,
        server: request.server,
        pane: request.pane,
        logged_target: "",
        target_session: request.target_session,
        pane_slot: request.pane_slot,
        own_session: request.own_session,
        action: "",
        reference: "",
        actor: "",
        body: "",
        shape: Shape::Send,
        defer: request.defer,
        composed: Composed::NONE,
    };
    // (1a) The `deliver()`-identical dead-pane refusal — the meta+`ps` owner —
    // BEFORE any lock, so a dead pane refuses fast and lock-free.
    let liveness = pane_liveness_at(
        &target_meta_dir(&view),
        request.pane_slot,
        &transport::observe_pane_probe(request.server, request.pane).unwrap_or_default(),
    );
    if refuses_as_dead(liveness) {
        return Ok(Outcome::Skipped(Leg::Dead));
    }
    // (1b) The pane's send-lock. Declared BEFORE the lifecycle guard so every
    // exit — explicit or by scope — releases the lifecycle lock first.
    let Some(send_lock) = lock_target(request.dir, request.pane) else {
        return Ok(Outcome::Skipped(Leg::TargetLocked));
    };
    // (1c) The full busy/human-input deferral — the `wait_for_quiet` owner,
    // OUTSIDE the lifecycle lock.
    if !wait_for_quiet(&view, request.model) {
        return Ok(Outcome::Skipped(Leg::Busy));
    }
    // (2) The caller's proof: the lifecycle lock, then identity. A leg skips
    // with the send-lock released by the scope.
    let lifecycle_lock = match prove() {
        Ok(guard) => guard,
        Err(leg) => return Ok(Outcome::Skipped(leg)),
    };
    // (3)+(4) under the held lock; (5) the lifecycle lock drops first, (6)
    // then the send-lock.
    let outcome = under_lock(request);
    drop(lifecycle_lock);
    drop(send_lock);
    outcome
}

/// Steps (3)+(4) under the caller-held lifecycle lock: the instant re-proof —
/// tmux reads ONLY, no file, no process, no wait — and the paste with its
/// bounded submit.
fn under_lock(request: &GuardedRequest<'_>) -> Result<Outcome, EnterFailed> {
    // (3) One probe: a shell in the foreground — or no readable probe at all,
    // which fails closed — refuses as Dead. Meta and `ps` are NOT consulted.
    let alive = match transport::observe_pane_probe(request.server, request.pane) {
        Some(probe) => instant_alive(probe.pid, crate::watchdog::command_is_shell(&probe.command)),
        None => false,
    };
    if !alive {
        return Ok(Outcome::Skipped(Leg::Dead));
    }
    // (3) One busy snapshot, no loop.
    if input_busy(request.server, request.pane, request.model)
        || recently_viewed(request.server, request.pane)
    {
        return Ok(Outcome::Skipped(Leg::Busy));
    }
    // (4) The paste, then the bounded submit under the same lock.
    if stage_and_paste(
        request.server,
        &buffer_name(request.pane),
        request.text.as_bytes(),
        request.pane,
    )
    .is_err()
    {
        return Ok(Outcome::Skipped(Leg::PasteFailed));
    }
    submit_bounded(request.server, request.pane, request.model).map(Outcome::Sent)
}

/// The under-lock liveness verdict, PURE: only a NAMED pid with a non-shell
/// foreground counts as alive. Meta and `ps` are not consulted, and an
/// unreadable probe fails closed to false.
const fn instant_alive(pid: Option<u32>, shell_in_foreground: bool) -> bool {
    pid.is_some() && !shell_in_foreground
}

/// Press Enter, then observe whether the paste left the input box —
/// [`submit_staged`]'s logic with `send_key`'s refusal propagated as the
/// KNOWN failure [`EnterFailed`] instead of swallowed.
///
/// The sleep budget is one settle (at most 300 ms) plus up to three 300 ms
/// verification reads — ≤ 1.2 s. Wall time past that is tmux responsiveness,
/// unbounded by ae like every lifecycle-lock holder.
///
/// # Errors
/// Returns `Err(EnterFailed)` when `send_key` refuses any Enter.
pub fn submit_bounded(
    server: &ServerId,
    pane: &str,
    model: InputModel,
) -> Result<SubmitState, EnterFailed> {
    std::thread::sleep(settle_for(model));
    if !transport::send_key(server, pane, Key::Enter) {
        return Err(EnterFailed);
    }
    for retry in 0..=VERIFY_RETRIES {
        std::thread::sleep(VERIFY_POLL);
        let observed = still_staged(server, pane, model);
        match observed {
            SubmitState::Submitted | SubmitState::Unknown(_) => return Ok(observed),
            SubmitState::StillStaged if retry == VERIFY_RETRIES => {
                return Ok(SubmitState::StillStaged);
            }
            SubmitState::StillStaged => {
                if !transport::send_key(server, pane, Key::Enter) {
                    return Err(EnterFailed);
                }
            }
        }
    }
    Ok(SubmitState::StillStaged)
}

/// Positive proof an unmodelled pane has DRAWN something and STOPPED CHANGING.
///
/// True only when both captures succeeded, are non-empty and byte-identical; a
/// missing, empty or changing screen is never ready. Pure, so the settle
/// decision is pinnable without tmux.
fn pane_settled(before: Option<&str>, after: Option<&str>) -> bool {
    matches!(
        (before, after),
        (Some(before), Some(after)) if !before.is_empty() && before == after
    )
}

/// Prove the staged notice on screen before any Enter.
fn prove_notice(
    server: &ServerId,
    pane: &str,
    model: InputModel,
    pointer: &str,
    buffer: &str,
) -> bool {
    for attempt in 0..2 {
        std::thread::sleep(NOTICE_POLL);
        let region = transport::capture_screen(server, pane, Styling::Escapes).unwrap_or_default();
        if notice::prove(model, &region, pointer) {
            return true;
        }
        if attempt != 0 {
            return false;
        }
        if !clear_is_measurable(server, pane, model) {
            return false;
        }
        if stage_and_paste(server, buffer, pointer.as_bytes(), pane).is_err() {
            return false;
        }
    }
    false
}

/// Did C-u demonstrably empty the input box — `_notice_clear_measurable`?
fn clear_is_measurable(server: &ServerId, pane: &str, model: InputModel) -> bool {
    if !transport::send_key(server, pane, Key::ClearLine) {
        return false;
    }
    std::thread::sleep(NOTICE_POLL);
    match transport::capture_screen(server, pane, Styling::Escapes) {
        Some(region) if !region.is_empty() => region::occupancy(&region, model) == Occupancy::Idle,
        _ => false,
    }
}

/// The tmux buffer this delivery stages in, named per-pane as well as
/// per-process so two concurrent deliveries from one
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
        // The fallback is a UTC stamp.
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

/// The name grammar the store screens a ref and an action against.
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
fn lock_target(dir: &Path, pane: &str) -> Option<std::fs::File> {
    let root = dir.parent()?.join(".locks");
    std::fs::create_dir_all(&root).ok()?;
    let sanitized: String = pane
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    crate::store::lock(&root.join(format!("send-lock-{sanitized}")), LOCK_WAIT).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        Failure, LivenessRefusal, PaneLiveness, Request, Shape, TargetInput, UNVERIFIED,
        VERIFY_POLL, buffer_name, choose_input, frame, instant_alive, is_name_safe,
        observed_liveness, pane_settled, refuses_as_dead, settle_for, store_body,
        under_lock_refusal, unmodelled_ready,
    };
    use crate::inventory::ServerId;
    use crate::tool::{Composed, InputModel, ToolKind};
    use std::time::Duration;

    /// The REAL opencode boot frame: blank, stable for ~2.7 s. This is the
    /// frame `e737b6b3` accepted as ready.
    const OPENCODE_BOOT: &str =
        include_str!("../tests/fixtures/opencode-composer/opencode-boot-frame.esc");
    /// The REAL opencode composed frame: the welcome screen with its composer.
    const OPENCODE_COMPOSED: &str =
        include_str!("../tests/fixtures/opencode-composer/opencode-composed-frame.esc");
    /// The REAL grok boot/composed frames and the REAL agy boot/composed/
    /// trust-modal frames (provenance beside them).
    const GROK_COMPOSED: &str =
        include_str!("../tests/fixtures/grok-composer/grok-composed-frame.txt");
    const GROK_BOOT: &str = include_str!("../tests/fixtures/grok-composer/grok-boot-frame.txt");
    const AGY_COMPOSED: &str =
        include_str!("../tests/fixtures/agy-composer/agy-composed-frame.txt");
    const AGY_BOOT: &str = include_str!("../tests/fixtures/agy-composer/agy-boot-frame.txt");
    const AGY_MODAL: &str =
        include_str!("../tests/fixtures/agy-composer/agy-trust-modal-frame.txt");

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
            composed: Composed::NONE,
        }
    }

    #[test]
    fn input_selection_prefers_a_modelled_record_and_falls_back_with_exact_diagnostics() {
        assert_eq!(
            choose_input("claude", "codex"),
            TargetInput {
                model: InputModel::BorderDelimited,
                diagnostic: "claude",
            }
        );
        assert_eq!(
            choose_input("gemini", "codex"),
            TargetInput {
                model: InputModel::StyleDelimited,
                diagnostic: "codex",
            }
        );
        assert_eq!(
            choose_input("gemini", "gemini"),
            TargetInput {
                model: InputModel::Unmodelled,
                diagnostic: "other",
            }
        );
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
            "⟦ae:interrupt from cl:lead⟧\nstop",
            "an interrupt is marked as the control action it is"
        );
        assert_eq!(
            frame(&request("cl:lead", "task", Shape::Launch)),
            "⟦ae:brief from cl:lead⟧\ntask",
            "a spawn brief is marked as the task contract it is"
        );
        assert_eq!(
            frame(&request("orchestrator", "human words", Shape::Relay)),
            "human words",
            "relay is the one privileged unenveloped sender"
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
                framed: "framed".into(),
                notice: true
            }
            .body_file(),
            "/m/x.txt"
        );
        assert_eq!(
            format!(
                "{:?}",
                Failure::Unconfirmed {
                    body_file: "/m/x.txt".into(),
                    framed: "framed body".into(),
                    notice: false
                }
            ),
            "Unconfirmed { body_file: \"/m/x.txt\", notice: false }",
            "the recovery body is not leaked into diagnostics"
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
    fn a_legacy_v1_meta_supplies_no_recorded_binary_and_is_not_an_empty_v2_session() {
        // The delivery path reads `agent_bin.<slot>` THROUGH the roster, so a
        // meta this ae does not serve must answer nothing rather than panic
        // or invent a tool. The three arms are the ones that can be confused.
        let dir = std::env::temp_dir().join(format!("ae-deliver-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meta"),
            "mode=local\nagent.main=claude:lead\nagent_bin.main=claude\n",
        )
        .unwrap();
        assert_eq!(
            super::recorded_binary(&dir, "main"),
            "",
            "a v1 row names no seat, so its agent_bin belongs to nobody"
        );
        std::fs::write(
            dir.join("meta"),
            "mode=local\nseat.main=lead\nagent_bin.main=claude\n",
        )
        .unwrap();
        assert_eq!(
            super::recorded_binary(&dir, "main"),
            "claude",
            "the CONTROL: a v2 seat still answers"
        );
        std::fs::write(dir.join("meta"), "mode=local\n").unwrap();
        assert_eq!(super::recorded_binary(&dir, "main"), "", "and so does none");
        let _ = std::fs::remove_dir_all(&dir);
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
        // An ask and its reply SHARE a request id by design.
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

    #[test]
    fn the_settle_gives_the_unverified_composers_the_conservative_pause() {
        assert_eq!(
            settle_for(InputModel::StyleDelimited),
            Duration::from_millis(100),
            "codex is the measured, retried composer"
        );
        assert_eq!(
            settle_for(InputModel::BorderDelimited),
            Duration::from_millis(300)
        );
        assert_eq!(
            settle_for(InputModel::Unmodelled),
            Duration::from_millis(300),
            "the largest paste in the product takes the conservative pause"
        );
    }

    /// The signal as the tool table hands it to `wait_input_ready`.
    fn opencode_markers() -> Composed {
        ToolKind::OpenCode.adapter().input.composed
    }

    /// The grok/agy signals as the tool table hands them over.
    fn grok_markers() -> Composed {
        ToolKind::Grok.adapter().input.composed
    }

    fn agy_markers() -> Composed {
        ToolKind::Agy.adapter().input.composed
    }

    #[test]
    fn an_opencode_boot_frame_is_stable_but_not_composed_and_never_ready() {
        // The regression, pinned: this frame is byte-identical and non-empty,
        // so the OLD settle accepted it — but it carries no composed marker,
        // and a paste into it is lost.
        assert!(
            pane_settled(Some(OPENCODE_BOOT), Some(OPENCODE_BOOT)),
            "the boot frame IS settled: stability alone would call it ready"
        );
        assert!(
            !super::region::composed_ui(OPENCODE_BOOT, opencode_markers()),
            "a blank boot frame carries neither marker"
        );
        assert!(
            !unmodelled_ready(Some(OPENCODE_BOOT), Some(OPENCODE_BOOT), opencode_markers()),
            "composed AND settled, both: a stable blank frame is never ready"
        );
    }

    #[test]
    fn an_opencode_composed_frame_is_composed_and_ready() {
        assert!(
            OPENCODE_COMPOSED.contains('╹'),
            "the composer's structural corner is load-bearing in the fixture"
        );
        assert!(
            OPENCODE_COMPOSED.contains("Ask anything…"),
            "the composer's placeholder is load-bearing in the fixture"
        );
        assert!(super::region::composed_ui(
            OPENCODE_COMPOSED,
            opencode_markers()
        ));
        assert!(
            unmodelled_ready(
                Some(OPENCODE_COMPOSED),
                Some(OPENCODE_COMPOSED),
                opencode_markers()
            ),
            "the real composed frame is ready"
        );
        assert!(
            !unmodelled_ready(
                Some(OPENCODE_COMPOSED),
                Some(OPENCODE_BOOT),
                opencode_markers()
            ),
            "and a frame that keeps changing is not, however composed its seed"
        );
    }

    #[test]
    fn a_grok_boot_frame_is_stable_but_never_ready() {
        assert!(
            pane_settled(Some(GROK_BOOT), Some(GROK_BOOT)),
            "the boot frame IS settled: stability alone would call it ready"
        );
        assert!(!super::region::composed_ui(GROK_BOOT, grok_markers()));
        assert!(!unmodelled_ready(
            Some(GROK_BOOT),
            Some(GROK_BOOT),
            grok_markers()
        ));
    }

    #[test]
    fn a_grok_composed_frame_is_composed_and_ready() {
        assert!(unmodelled_ready(
            Some(GROK_COMPOSED),
            Some(GROK_COMPOSED),
            grok_markers()
        ));
        assert!(
            !unmodelled_ready(Some(GROK_COMPOSED), Some(GROK_BOOT), grok_markers()),
            "a frame that keeps changing is not, however composed its seed"
        );
    }

    #[test]
    fn an_agy_composed_frame_is_composed_and_ready() {
        assert!(unmodelled_ready(
            Some(AGY_COMPOSED),
            Some(AGY_COMPOSED),
            agy_markers()
        ));
        assert!(
            !unmodelled_ready(Some(AGY_COMPOSED), Some(AGY_BOOT), agy_markers()),
            "a frame that keeps changing is not, however composed its seed"
        );
    }

    #[test]
    fn an_agy_trust_modal_is_stable_but_never_ready() {
        assert!(
            pane_settled(Some(AGY_MODAL), Some(AGY_MODAL)),
            "the modal IS settled: stability alone would call it ready"
        );
        assert!(!super::region::composed_ui(AGY_MODAL, agy_markers()));
        assert!(!unmodelled_ready(
            Some(AGY_MODAL),
            Some(AGY_MODAL),
            agy_markers()
        ));
    }

    #[test]
    fn an_agy_boot_frame_is_never_ready() {
        assert!(!unmodelled_ready(
            Some(AGY_BOOT),
            Some(AGY_BOOT),
            agy_markers()
        ));
    }

    #[test]
    fn tools_without_a_composed_signal_refuse_even_a_composed_frame() {
        for kind in [ToolKind::Gemini, ToolKind::Unknown] {
            let spec = kind.adapter().input.composed;
            assert!(spec.is_empty());
            assert!(
                !unmodelled_ready(Some(GROK_COMPOSED), Some(GROK_COMPOSED), spec),
                "no signal refuses at once, whatever the frame"
            );
        }
        assert!(!grok_markers().is_empty());
        assert!(!agy_markers().is_empty());
    }

    #[test]
    fn a_pidless_probe_never_proves_a_live_agent() {
        use crate::procs::Descendancy;
        // A pid-less probe proves NOTHING, whatever the foreground says —
        // including a non-shell command, which is otherwise the Alive shortcut.
        assert_eq!(
            observed_liveness(false, true, None, Descendancy::Present),
            PaneLiveness::Unproven,
            "a non-shell foreground with no pid is unproven"
        );
        assert_eq!(
            observed_liveness(true, true, None, Descendancy::Present),
            PaneLiveness::Unproven,
            "a shell foreground with no pid is unproven"
        );
        // The one Alive shortcut: a NAMED pid and a non-shell foreground.
        assert_eq!(
            observed_liveness(false, false, Some(4242), Descendancy::Unknown),
            PaneLiveness::Alive
        );
        // A shell foreground needs a usable binary and a walk that is not
        // Unknown: everything else is UNPROVEN.
        assert_eq!(
            observed_liveness(true, false, Some(4242), Descendancy::Present),
            PaneLiveness::Unproven,
            "an unreadable meta (or a shell-based profile) proves nothing"
        );
        assert_eq!(
            observed_liveness(true, true, Some(4242), Descendancy::Unknown),
            PaneLiveness::Unproven,
            "an unusable process snapshot proves nothing"
        );
        assert_eq!(
            observed_liveness(true, true, Some(4242), Descendancy::Absent),
            PaneLiveness::Dead
        );
        assert_eq!(
            observed_liveness(true, true, Some(4242), Descendancy::Present),
            PaneLiveness::Alive
        );
    }

    #[test]
    fn each_delivery_path_picks_its_own_unproven_policy() {
        // The under-lock unmodelled Launch requires a PROVEN live agent.
        assert_eq!(under_lock_refusal(PaneLiveness::Alive), None);
        assert_eq!(
            under_lock_refusal(PaneLiveness::Dead),
            Some(LivenessRefusal::Dead)
        );
        assert_eq!(
            under_lock_refusal(PaneLiveness::Unproven),
            Some(LivenessRefusal::Unproven),
            "an unprovable pane must never take the Launch paste"
        );
        // Ordinary delivery keeps the standing fail-open: only Dead refuses,
        // so a transient snapshot failure cannot refuse every send.
        assert!(refuses_as_dead(PaneLiveness::Dead));
        assert!(
            !refuses_as_dead(PaneLiveness::Unproven),
            "ordinary delivery must NOT start refusing on a transient ps failure"
        );
        assert!(!refuses_as_dead(PaneLiveness::Alive));
    }

    #[test]
    fn a_tool_with_no_composed_signal_is_never_ready_however_settled() {
        // gemini and the unknown fallback carry NO composed signal: for
        // them readiness can only refuse, visibly, exactly as it did before
        // the settle arm was introduced.
        let stable = "a settled frame of some tool\n";
        assert!(pane_settled(Some(stable), Some(stable)));
        assert!(
            !unmodelled_ready(Some(stable), Some(stable), Composed::NONE),
            "an empty marker list is a refusal, not a fallback to settled-only"
        );
    }

    #[test]
    fn a_missing_or_empty_capture_is_never_settled() {
        assert!(!pane_settled(None, None));
        assert!(!pane_settled(None, Some("box")));
        assert!(!pane_settled(Some("box"), None));
        assert!(!pane_settled(Some(""), Some("")));
    }

    #[test]
    fn a_pane_that_keeps_changing_is_never_settled() {
        assert!(!pane_settled(Some("booting 1"), Some("booting 2")));
        assert!(!pane_settled(Some(""), Some("drawn")));
        assert!(!pane_settled(Some("drawn"), Some("")));
    }

    #[test]
    fn the_guarded_submit_sleep_budget_fits_1_2s_on_every_input_model() {
        // R10 pin: one settle (worst arm) plus up to three verification
        // reads. The retry bound itself is pinned by the counted tmux calls
        // in tests/it/deliver.rs — never by wall time.
        for model in [
            InputModel::BorderDelimited,
            InputModel::StyleDelimited,
            InputModel::Unmodelled,
        ] {
            let budget = settle_for(model) + VERIFY_POLL * 3;
            assert!(
                budget <= Duration::from_millis(1200),
                "{model:?}: {budget:?}"
            );
        }
    }

    #[test]
    fn the_under_lock_liveness_verdict_fails_closed() {
        // R10 pin: only a NAMED pid with a non-shell foreground is alive.
        assert!(instant_alive(Some(4242), false));
        assert!(!instant_alive(Some(4242), true));
        assert!(!instant_alive(None, false));
        assert!(!instant_alive(None, true));
    }
}
