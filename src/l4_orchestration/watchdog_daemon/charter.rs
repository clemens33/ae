//! The orchestrator seat's charter update (D5): one turn that hands a running
//! seat the current instructions when its launch holds an older charter.

use super::{Cycle, Event, Journal, SeatLaunch, Write, tmux};
use crate::deliver::{Delivered, DeliveryVerification, Failure, Stage};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The first words of the turn, which the seat and its owner read.
const HEAD: &str = "CHARTER UPDATE (ae ";

/// The slot the orchestrator seat runs in.
const SLOT: &str = "main";

/// How long the turn waits for a busy seat before it gives up for this cycle.
const DEFER: Duration = Duration::from_secs(10);

/// What the charter step did this cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Charter {
    /// Nothing to send, or the seat is not proven.
    Idle,
    /// A turn was attempted: the cycle sends the seat no overview beside it.
    Attempted,
    /// The current launch holds a claim from an earlier cycle whose turn is
    /// unconfirmed: the overview carries the owner's remedy.
    Unresolved,
}

/// How one attempt ended, from the delivery's own typed receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Receipt {
    /// Submitted and the input box was seen clear after Enter.
    Landed,
    /// Refused before anything was pasted: a later cycle may try again.
    BeforeSubmit,
    /// Pasted, or not provably unpasted: never sent again to this launch.
    Unconfirmed,
}

impl Receipt {
    fn of(delivery: Option<&Result<Delivered, Failure>>) -> Self {
        match delivery {
            Some(Ok(delivered)) if delivered.verification == DeliveryVerification::Verified => {
                Self::Landed
            }
            // The composed leg returns nothing when the pane was dead, its
            // lock was held or it stayed busy: nothing was pasted.
            None
            | Some(Err(
                Failure::DeadPane
                | Failure::Unproven { .. }
                | Failure::Storage
                | Failure::Lock
                | Failure::NoticeRefused { .. }
                | Failure::Abandoned { .. }
                | Failure::NotComposed { .. }
                | Failure::Refused {
                    stage: Stage::BeforeInput,
                    ..
                },
            )) => Self::BeforeSubmit,
            // An unverifiable submit, a failed paste, a notice whose pointer
            // was pasted and a refusal after input all may have left text.
            Some(_) => Self::Unconfirmed,
        }
    }

    const fn summary(self) -> &'static str {
        match self {
            Self::Landed => "charter update delivered",
            Self::BeforeSubmit => "charter update refused before submit; a later cycle retries",
            Self::Unconfirmed => "charter update unconfirmed; not sent again to this launch",
        }
    }
}

/// Settle the claim of launch `launch` by `receipt`: a landed turn becomes
/// the stamp, a refusal before submit drops the claim, and an unconfirmed
/// turn keeps it, so no cycle sends the turn to that launch again.
fn settle(
    meta_dir: &Path,
    launch: &str,
    receipt: Receipt,
) -> Result<(), crate::meta::RewriteError> {
    match receipt {
        Receipt::Landed => {
            crate::meta::set_charter(meta_dir, SLOT, launch, crate::config::SEAT_CHARTER_VERSION)
                .map(|_| ())
        }
        Receipt::BeforeSubmit => crate::meta::release_charter(meta_dir, SLOT, launch).map(|_| ()),
        Receipt::Unconfirmed => Ok(()),
    }
}

/// The proved seat a charter turn may reach, from one meta snapshot and this
/// cycle's pane observation.
struct Seat {
    /// The main agent's name.
    agent: String,
    /// The recorded main pane, which this cycle observed in the main slot.
    pane: String,
    /// The launch token this cycle's snapshot holds, carried through the
    /// claim, the admission and the stamp.
    launch: String,
    /// The charter the launch recorded, 1 when it recorded none for itself.
    running: u32,
    /// Whether this launch already holds a claim.
    claimed: bool,
    /// The seat overlay the snapshot records as the local config, proven
    /// by [`crate::meta::dispatcher_proof`]: the one file the charter text
    /// comes from.
    overlay: PathBuf,
    /// The session id the snapshot records, which keys the seat's declarations.
    session_id: String,
}

impl Cycle<'_> {
    /// Hand the orchestrator seat the current charter once per launch: when
    /// the seat is proved, the charter its launch recorded is older than
    /// [`crate::config::SEAT_CHARTER_VERSION`], the seat [`admits`] the turn
    /// and the prompt of the seat overlay teaches the current one. A
    /// launch-bound claim is taken under the meta lock before the first key;
    /// only a refusal the delivery proves came before any paste drops it, a
    /// landed turn turns it into the stamp, and anything else keeps it, so the
    /// turn is never sent twice to one launch. A `settling` cycle attempts
    /// nothing.
    ///
    /// # Errors
    ///
    /// Only a failure to write `err`.
    pub(super) fn charter_update(
        &self,
        observed: &[tmux::WatchPane],
        settling: bool,
        err: &mut impl Write,
    ) -> crate::error::Result<Charter> {
        let Some(seat) = self.charter_seat(observed) else {
            return Ok(Charter::Idle);
        };
        if seat.claimed {
            return Ok(Charter::Unresolved);
        }
        if settling || seat.running >= crate::config::SEAT_CHARTER_VERSION {
            return Ok(Charter::Idle);
        }
        let sight = Live {
            meta_dir: self.meta_dir,
            session: self.session,
            server: self.server,
            seat: &seat,
            tool: crate::tool::ToolKind::from_binary_name(
                self.agent_bin(SLOT).as_deref().unwrap_or_default(),
            ),
        };
        let journal = Journal {
            meta_dir: self.meta_dir,
            session: self.session,
        };
        attempt(
            &journal,
            &seat,
            &sight,
            |reference, compose, err| self.charter_deliver(&seat, reference, compose, err),
            err,
        )
    }

    /// The seat a charter turn may reach: the pane this cycle observed in the
    /// main slot passes [`crate::meta::dispatcher_proof`] against a fresh meta
    /// read, the proof the seat's own commands pass, and the cycle's launch
    /// snapshot names the launch that read records.
    fn charter_seat(&self, observed: &[tmux::WatchPane]) -> Option<Seat> {
        if !self.meta_agent {
            return None;
        }
        let root = crate::doors::state_root(crate::shape::current())?;
        let bytes = crate::meta::read_bytes(self.meta_dir).ok()?;
        let mut mains = observed
            .iter()
            .filter(|pane| pane.slot.as_deref() == Some(SLOT));
        let (Some(pane), None) = (mains.next(), mains.next()) else {
            return None;
        };
        let agent = pane.agent.as_deref()?;
        crate::meta::dispatcher_proof(&bytes, &root, SLOT, agent, Some(&pane.pane_id)).ok()?;
        let text = |key: &str| {
            crate::meta::sole_value(&bytes, key)
                .and_then(|value| std::str::from_utf8(value).ok())
                .map(str::to_owned)
        };
        let row = |key: &str| text(&format!("{key}.{SLOT}")).unwrap_or_default();
        let launch = match self.launch_ids.iter().find(|(slot, _)| slot == SLOT) {
            Some((_, SeatLaunch::Id(id))) if !id.is_empty() && *id == row("launch_id") => {
                id.clone()
            }
            _ => return None,
        };
        let running = if row("charter_launch") == launch {
            row("charter").parse::<u32>().unwrap_or(1)
        } else {
            1
        };
        Some(Seat {
            agent: agent.to_owned(),
            pane: pane.pane_id.clone(),
            claimed: row("charter_pending") == launch,
            launch,
            running,
            overlay: PathBuf::from(text(crate::config::LOCAL_CONFIG_KEY)?),
            session_id: text("session_id").unwrap_or_default(),
        })
    }

    /// One composed turn to the proved pane on this cycle's server, admitted
    /// in process as ae's own leg against the launch `seat` carries, and its
    /// typed receipt. `compose` runs after the delivery's quiet gate, just
    /// before the paste; its `None` pastes nothing.
    fn charter_deliver(
        &self,
        seat: &Seat,
        reference: &str,
        compose: &dyn Fn() -> Option<String>,
        err: &mut impl Write,
    ) -> crate::error::Result<Receipt> {
        let request = crate::deliver::Request {
            dir: self.meta_dir,
            server: self.server,
            pane: &seat.pane,
            logged_target: &seat.agent,
            target_session: self.session,
            pane_slot: SLOT,
            own_session: self.session,
            action: crate::watchdog::CHARTER_UPDATE_ACTION,
            reference,
            actor: crate::watchdog::WATCHDOG_ACTOR,
            body: "",
            shape: crate::deliver::Shape::Context,
            defer: DEFER,
            composed: crate::tool::Composed::NONE,
        };
        let Some(guard) = crate::tracked::admit_leg(&request, (SLOT, &seat.launch), err)? else {
            return Ok(Receipt::BeforeSubmit);
        };
        let delivery =
            crate::deliver::deliver_composed_checked(&request, Some(&guard), compose, err)?;
        Ok(Receipt::of(delivery.as_ref()))
    }
}

/// One charter attempt for `seat`, whose launch holds an older charter. The
/// seat must [`admits`] the turn at this early read. Then the claim is taken,
/// and `deliver` composes the turn through a closure that reads `sight` again
/// (the late gate): a seat that turned busy or changed its declaration in
/// between gets nothing pasted, and the refusal before submit releases its
/// claim for a later cycle. The receipt settles the claim and is journaled.
fn attempt<W: Write>(
    journal: &Journal<'_>,
    seat: &Seat,
    sight: &impl Sight,
    deliver: impl FnOnce(&str, &dyn Fn() -> Option<String>, &mut W) -> crate::error::Result<Receipt>,
    err: &mut W,
) -> crate::error::Result<Charter> {
    if !admits(sight) {
        return Ok(Charter::Idle);
    }
    let Some(prompt) = charter_prompt(&seat.overlay) else {
        return Ok(Charter::Idle);
    };
    let target = crate::config::SEAT_CHARTER_VERSION;
    match crate::meta::claim_charter(journal.meta_dir, SLOT, &seat.launch, target) {
        Ok(true) => {}
        Ok(false) => return Ok(Charter::Idle),
        Err(why) => {
            writeln!(
                err,
                "ae: watchdog: could not claim the orchestrator charter update ({}) — not sent",
                why.cause()
            )?;
            return Ok(Charter::Idle);
        }
    }
    let reference = format!(
        "charter-{}-{}",
        crate::time::Timestamp::now().epoch(),
        seat.launch
    );
    let text = format!(
        "{HEAD}{}): your instructions changed. These replace your earlier instructions:\n\n{prompt}",
        crate::shape::VERSION
    );
    let compose = || late_compose(sight, &text);
    let receipt = deliver(&reference, &compose, err)?;
    if let Err(why) = settle(journal.meta_dir, &seat.launch, receipt) {
        writeln!(
            err,
            "ae: watchdog: {}, but its claim could not be settled ({}); it is not sent again",
            receipt.summary(),
            why.cause()
        )?;
    }
    journal.record_referring(
        crate::watchdog::CHARTER_UPDATE_ACTION,
        &seat.agent,
        &reference,
        receipt.summary(),
        err,
    )?;
    Ok(Charter::Attempted)
}

/// What the charter step reads of the seat, fresh at each call: once at the
/// early gate and once more at the late gate, just before the turn is
/// composed.
trait Sight {
    /// The seat's newest declaration in this session's main slot.
    fn declared(&self) -> Option<String>;
    /// The seat's frame.
    fn frame(&self) -> Frame;
}

/// The seat's frame as one fresh capture of its main pane shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frame {
    /// The tool's input model reads its input box empty, its grammar reads no
    /// running turn, and the same capture shows no human-only prompt or
    /// service wait.
    Idle,
    /// Anything else, or no capture: a running turn, a draft, a drawn prompt.
    Held,
    /// A tool whose frame ae cannot read.
    Unread,
}

/// The late gate: `text` when the seat still [`admits`] the turn at this read,
/// just before the paste, else nothing, so nothing is pasted.
fn late_compose(sight: &impl Sight, text: &str) -> Option<String> {
    admits(sight).then(|| text.to_owned())
}

/// Whether the seat takes the turn now, from one fresh declaration read and,
/// when the declaration allows a turn at all, one fresh frame read.
fn admits(sight: &impl Sight) -> bool {
    let declared = sight.declared();
    matches!(declared.as_deref(), Some("done" | "waiting-user"))
        && eligible(declared.as_deref(), sight.frame())
}

/// Whether a seat that declared `declared` and shows `frame` may get the turn:
/// `done` or `waiting-user` on an idle frame. For a tool whose frame ae cannot
/// read, only `done`, as before: its checked delivery still refuses a draft or
/// a busy composer before any paste.
fn eligible(declared: Option<&str>, frame: Frame) -> bool {
    match frame {
        Frame::Idle => matches!(declared, Some("done" | "waiting-user")),
        Frame::Unread => declared == Some("done"),
        Frame::Held => false,
    }
}

/// The frame one styled capture shows. `read` is the delivery's own prompt
/// read of that capture. Only a capture whose input box the tool's input model
/// reads empty, with no known prompt drawn, and whose printable rows the tool's
/// grammar classifies idle, is [`Frame::Idle`]: the printable rows alone cannot
/// tell a dim placeholder from the same text typed as a draft. No capture is
/// held.
fn frame_of(
    tool: crate::tool::ToolKind,
    capture: Option<&str>,
    read: &crate::deliver::PromptRead,
) -> Frame {
    use crate::deliver::region;
    let model = tool.input_model();
    let idle = model.is_modelled()
        && matches!(read, crate::deliver::PromptRead::NoKnownPrompt)
        && capture.is_some_and(|styled| {
            region::occupancy(styled, model) == region::Occupancy::Idle
                && crate::harness_state::classify(&region::plain_text(styled), tool)
                    == crate::harness_state::HarnessState::Idle
        });
    if idle { Frame::Idle } else { Frame::Held }
}

/// The live seat: its journal on disk and one capture of its main pane.
struct Live<'a> {
    meta_dir: &'a Path,
    session: &'a str,
    server: &'a crate::inventory::ServerId,
    seat: &'a Seat,
    tool: crate::tool::ToolKind,
}

impl Sight for Live<'_> {
    fn declared(&self) -> Option<String> {
        newest_declaration(&super::read_events(self.meta_dir), self.session, self.seat)
    }

    /// One styled capture, taken with the tool's own input model, serves every
    /// half: the delivery's prompt read takes it, the input model reads its box,
    /// and the tool's grammar classifies its printable rows.
    fn frame(&self) -> Frame {
        if !crate::harness_state::observable(self.tool) {
            return Frame::Unread;
        }
        let (capture, read) = crate::deliver::prompt_hold(
            self.server,
            &self.seat.pane,
            self.tool.input_model(),
            Some(self.tool),
        );
        frame_of(self.tool, capture.as_deref(), &read)
    }
}

/// The newest declaration in `events` of the seat in `session`'s main slot.
fn newest_declaration(events: &[Event], session: &str, seat: &Seat) -> Option<String> {
    let key = crate::events::SessionKey::new(session, &seat.session_id);
    events
        .iter()
        .rev()
        .find(|event| {
            event.declared_state().is_some()
                && crate::watchdog::event_is_actor_in(event, key, SLOT, &seat.agent)
        })
        .and_then(Event::declared_state)
        .map(str::to_owned)
}

/// The prompt of the seat overlay at `overlay`, when it teaches the current
/// charter. Only the overlay counts: no other config layer speaks for the seat.
fn charter_prompt(overlay: &Path) -> Option<String> {
    let (_, prompt) = crate::render::prompt_instructions(&[overlay.to_path_buf()])?;
    (crate::config::charter_version(&prompt) >= crate::config::SEAT_CHARTER_VERSION)
        .then_some(prompt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deliver::Unverifiable;

    fn scratch(tag: &str) -> PathBuf {
        let dir = PathBuf::from(format!("/tmp/ae-charter-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the scratch dir");
        std::fs::write(dir.join("meta"), "session=orch\nlaunch_id.main=tok\n")
            .expect("a seat meta");
        dir
    }

    fn meta(dir: &Path) -> String {
        String::from_utf8(crate::meta::read_bytes(dir).expect("the seat meta"))
            .expect("a UTF-8 seat meta")
    }

    fn delivered(verification: DeliveryVerification) -> Delivered {
        Delivered {
            body_file: "/b".to_owned(),
            framed: "f".to_owned(),
            verification,
        }
    }

    #[test]
    fn only_a_refusal_before_any_paste_lets_a_later_cycle_retry() {
        assert_eq!(Receipt::of(None), Receipt::BeforeSubmit);
        assert_eq!(
            Receipt::of(Some(&Ok(delivered(DeliveryVerification::Verified)))),
            Receipt::Landed
        );
        assert_eq!(
            Receipt::of(Some(&Ok(delivered(DeliveryVerification::Unverifiable(
                Unverifiable::PaneUnparseable
            ))))),
            Receipt::Unconfirmed
        );
        let body_file = "/b".to_owned();
        for before in [
            Failure::DeadPane,
            Failure::Storage,
            Failure::Lock,
            Failure::NotComposed {
                body_file: body_file.clone(),
                blocker: None,
            },
            crate::deliver::refused(&body_file, Stage::BeforeInput),
        ] {
            assert_eq!(Receipt::of(Some(&Err(before))), Receipt::BeforeSubmit);
        }
        for after in [
            Failure::Unconfirmed {
                body_file: body_file.clone(),
                framed: "f".to_owned(),
                notice: true,
            },
            Failure::Unconfirmed {
                body_file: body_file.clone(),
                framed: "f".to_owned(),
                notice: false,
            },
            Failure::Paste {
                body_file: body_file.clone(),
            },
            crate::deliver::refused(&body_file, Stage::Pasted),
            crate::deliver::refused(&body_file, Stage::Entered),
        ] {
            assert_eq!(Receipt::of(Some(&Err(after))), Receipt::Unconfirmed);
        }
    }

    #[test]
    fn an_unverifiable_submit_keeps_the_claim_and_stamps_no_version() {
        let dir = scratch("unverifiable");
        let version = crate::config::SEAT_CHARTER_VERSION;
        assert!(crate::meta::claim_charter(&dir, SLOT, "tok", version).expect("claim"));
        let receipt = Receipt::of(Some(&Ok(delivered(DeliveryVerification::Unverifiable(
            Unverifiable::PaneUnparseable,
        )))));
        settle(&dir, "tok", receipt).expect("settled");
        let after = meta(&dir);
        assert!(after.contains("charter_pending.main=tok\n"), "{after}");
        assert!(!after.contains("charter.main="), "{after}");
        assert!(!after.contains("charter_launch.main="), "{after}");
        assert!(
            !crate::meta::claim_charter(&dir, SLOT, "tok", version).expect("second claim"),
            "the kept claim refuses a second turn to the launch"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    const TEMPLATE: &str = include_str!("../../../contrib/aeorchestrator/orchestrator.config");
    /// The measured styled Codex idle frame: a bold `›` and the DIM
    /// placeholder.
    const CODEX_IDLE: &str =
        include_str!("../../../tests/fixtures/codex-composer/codex-idle-0.155.1-200x40.esc");
    /// The placeholder's dim run in [`CODEX_IDLE`].
    const DIM_PLACEHOLDER: &str = "\u{1b}[2mAsk Codex to do anything\u{1b}[0m";
    /// The row above the composer in [`CODEX_IDLE`].
    const CODEX_DONE_ROW: &str = "\u{1b}[2m  done 12:53 PM\u{1b}[0m";
    /// The running turn's row of the measured plain busy frame
    /// `tests/fixtures/harness-state/codex-busy-280x40.txt`.
    const CODEX_WORKING_ROW: &str = "• Working (5m 03s • esc to interrupt)";
    const CLAUDE_IDLE: &str =
        include_str!("../../../tests/fixtures/harness-state/claude-idle-167x40.txt");

    /// [`CODEX_IDLE`] with `from` replaced by `to`, once; the replacement must
    /// hit.
    fn codex_with(from: &str, to: &str) -> String {
        assert!(
            CODEX_IDLE.contains(from),
            "{from:?} is in the measured frame"
        );
        CODEX_IDLE.replacen(from, to, 1)
    }

    /// The frame [`Live`] reads from `capture`: the delivery's prompt read of
    /// the same capture, then [`frame_of`].
    fn frame_read(tool: crate::tool::ToolKind, capture: &str) -> Frame {
        frame_of(
            tool,
            Some(capture),
            &crate::deliver::prompt_read(Some(tool), Some(capture)),
        )
    }

    /// A sight that declares `declared` at every read and whose frames are real
    /// captures, read as [`Live`] reads one: the first from `early`, every
    /// later one from `late`.
    struct Captured {
        declared: &'static str,
        early: String,
        late: String,
        frames: std::cell::Cell<u32>,
    }

    impl Sight for Captured {
        fn declared(&self) -> Option<String> {
            Some(self.declared.to_owned())
        }

        fn frame(&self) -> Frame {
            let read = self.frames.replace(self.frames.get() + 1);
            let capture = if read == 0 { &self.early } else { &self.late };
            frame_read(crate::tool::ToolKind::Codex, capture)
        }
    }

    /// A sight that answers its first read of each half from `early` and every
    /// later read from `late`.
    struct Scripted {
        early: (Option<&'static str>, Frame),
        late: (Option<&'static str>, Frame),
        declared: std::cell::Cell<u32>,
        frames: std::cell::Cell<u32>,
    }

    impl Scripted {
        fn new(early: (Option<&'static str>, Frame), late: (Option<&'static str>, Frame)) -> Self {
            Self {
                early,
                late,
                declared: std::cell::Cell::new(0),
                frames: std::cell::Cell::new(0),
            }
        }
    }

    impl Sight for Scripted {
        fn declared(&self) -> Option<String> {
            let read = self.declared.replace(self.declared.get() + 1);
            let (declared, _) = if read == 0 { self.early } else { self.late };
            declared.map(str::to_owned)
        }

        fn frame(&self) -> Frame {
            let read = self.frames.replace(self.frames.get() + 1);
            if read == 0 { self.early.1 } else { self.late.1 }
        }
    }

    /// A seat in `dir` whose launch `tok` recorded charter 1, with the
    /// released template as its overlay.
    fn seat_in(dir: &Path) -> Seat {
        let overlay = dir.join("orchestrator.config");
        std::fs::write(&overlay, TEMPLATE).expect("the seat overlay");
        Seat {
            agent: "lead".to_owned(),
            pane: "%1".to_owned(),
            launch: "tok".to_owned(),
            running: 1,
            claimed: false,
            overlay,
            session_id: String::new(),
        }
    }

    /// One attempt whose delivery pastes what the late compose returns, as the
    /// checked delivery does past its quiet gate: the text lands, and no text
    /// is a refusal before any paste.
    fn attempt_with(dir: &Path, seat: &Seat, sight: &impl Sight) -> (Charter, Option<String>) {
        let journal = Journal {
            meta_dir: dir,
            session: "orch",
        };
        let mut pasted = None;
        let charter = attempt(
            &journal,
            seat,
            sight,
            |_, compose, _| {
                pasted = compose();
                Ok(if pasted.is_some() {
                    Receipt::Landed
                } else {
                    Receipt::of(None)
                })
            },
            &mut Vec::new(),
        )
        .expect("the attempt writes its diagnostics");
        (charter, pasted)
    }

    fn summaries(dir: &Path) -> Vec<String> {
        super::super::read_events(dir)
            .into_iter()
            .filter(|event| event.action == crate::watchdog::CHARTER_UPDATE_ACTION)
            .filter_map(|event| event.summary)
            .collect()
    }

    #[test]
    fn a_seat_that_turns_busy_or_redeclares_after_the_early_read_gets_nothing_pasted() {
        let idle = (Some("waiting-user"), Frame::Idle);
        for (tag, late) in [
            ("late-frame", (Some("waiting-user"), Frame::Held)),
            ("late-declaration", (Some("working"), Frame::Idle)),
        ] {
            let dir = scratch(tag);
            let seat = seat_in(&dir);
            let (charter, pasted) = attempt_with(&dir, &seat, &Scripted::new(idle, late));
            assert_eq!(charter, Charter::Attempted, "{tag}");
            assert_eq!(pasted, None, "{tag}: the late gate composes nothing");
            let after = meta(&dir);
            assert!(!after.contains("charter_pending.main="), "{tag}: {after}");
            assert!(!after.contains("charter.main="), "{tag}: {after}");
            assert_eq!(summaries(&dir), [Receipt::BeforeSubmit.summary()], "{tag}");

            let (charter, pasted) = attempt_with(&dir, &seat, &Scripted::new(idle, idle));
            assert_eq!(charter, Charter::Attempted, "{tag}");
            assert!(
                pasted.is_some_and(|text| text.starts_with(HEAD)),
                "{tag}: a later idle cycle delivers"
            );
            let after = meta(&dir);
            let version = crate::config::SEAT_CHARTER_VERSION;
            assert!(
                after.contains(&format!("charter.main={version}\n")),
                "{tag}: {after}"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_seat_the_early_read_does_not_admit_is_never_claimed() {
        for early in [
            (Some("waiting-user"), Frame::Held),
            (Some("done"), Frame::Held),
            (Some("waiting-user"), Frame::Unread),
            (Some("working"), Frame::Idle),
            (None, Frame::Idle),
        ] {
            let dir = scratch("early");
            let seat = seat_in(&dir);
            let journal = Journal {
                meta_dir: &dir,
                session: "orch",
            };
            let charter = attempt(
                &journal,
                &seat,
                &Scripted::new(early, (Some("done"), Frame::Idle)),
                |_, _, _| panic!("{early:?} reached the delivery"),
                &mut Vec::new(),
            )
            .expect("no diagnostics");
            assert_eq!(charter, Charter::Idle, "{early:?}");
            let after = meta(&dir);
            assert!(
                !after.contains("charter_pending.main="),
                "{early:?}: {after}"
            );
            assert!(summaries(&dir).is_empty(), "{early:?}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn done_or_waiting_user_needs_an_idle_frame_and_an_unread_tool_keeps_done_only() {
        for (declared, frame, admitted) in [
            (Some("done"), Frame::Idle, true),
            (Some("waiting-user"), Frame::Idle, true),
            (Some("done"), Frame::Held, false),
            (Some("waiting-user"), Frame::Held, false),
            (Some("done"), Frame::Unread, true),
            (Some("waiting-user"), Frame::Unread, false),
            (Some("working"), Frame::Idle, false),
            (Some("waiting-agent"), Frame::Idle, false),
            (Some("blocked"), Frame::Idle, false),
            (Some("working"), Frame::Unread, false),
            (None, Frame::Idle, false),
            (None, Frame::Unread, false),
        ] {
            assert_eq!(
                eligible(declared, frame),
                admitted,
                "{declared:?} {frame:?}"
            );
            let sight = Scripted::new((declared, frame), (declared, frame));
            assert_eq!(admits(&sight), admitted, "{declared:?} {frame:?}");
        }
    }

    #[test]
    fn only_an_idle_capture_with_no_drawn_prompt_is_an_idle_frame() {
        use crate::deliver::{PromptRead, region};
        use crate::tool::{InputModel, ToolKind};
        let drawn = PromptRead::Drawn(crate::watchdog::HumanPrompt {
            kind: crate::watchdog::DialogKind::FolderTrust,
            warnings: Vec::new(),
            question: "Allow this command?".to_owned(),
            keys: "Enter to confirm".to_owned(),
            trust: None,
            dismiss: false,
        });
        // COMPOSITE: the measured styled idle frame with the measured running
        // turn's row in place of the row above its still empty composer.
        let busy = codex_with(CODEX_DONE_ROW, CODEX_WORKING_ROW);
        assert_eq!(
            region::occupancy(&busy, InputModel::StyleDelimited),
            region::Occupancy::Idle,
            "the running turn's composer is empty"
        );
        let claude_draft = CLAUDE_IDLE.replacen("❯\u{a0}\n", "❯\u{a0}fix the build\n", 1);
        assert_ne!(claude_draft, CLAUDE_IDLE, "the draft is typed into the box");
        for (tool, capture, frame) in [
            (ToolKind::Codex, CODEX_IDLE, Frame::Idle),
            (ToolKind::Claude, CLAUDE_IDLE, Frame::Idle),
            (ToolKind::Codex, busy.as_str(), Frame::Held),
            (ToolKind::Claude, claude_draft.as_str(), Frame::Held),
        ] {
            assert_eq!(frame_read(tool, capture), frame, "{tool:?} {capture:?}");
        }
        // The printable rows alone carry no style for the input model to read.
        assert_eq!(
            frame_read(ToolKind::Codex, &region::plain_text(CODEX_IDLE)),
            Frame::Held
        );
        for read in [drawn, PromptRead::CodexServiceWait, PromptRead::Unreadable] {
            assert_eq!(
                frame_of(ToolKind::Codex, Some(CODEX_IDLE), &read),
                Frame::Held,
                "{read:?}"
            );
        }
        assert_eq!(
            frame_of(ToolKind::Codex, None, &PromptRead::NoKnownPrompt),
            Frame::Held
        );
    }

    #[test]
    fn a_draft_typed_like_the_dim_placeholder_is_held_at_the_late_gate() {
        use crate::deliver::{PromptRead, region};
        use crate::tool::{InputModel, ToolKind};
        let tool = ToolKind::Codex;
        // The placeholder's own cells, typed: its dim run without the dim.
        let draft = codex_with(DIM_PLACEHOLDER, "Ask Codex to do anything");
        let plain = region::plain_text(CODEX_IDLE);
        assert_eq!(
            region::plain_text(&draft),
            plain,
            "identical printable rows"
        );
        assert_eq!(
            crate::harness_state::classify(&plain, tool),
            crate::harness_state::HarnessState::Idle
        );
        for capture in [CODEX_IDLE, draft.as_str()] {
            assert_eq!(
                crate::deliver::prompt_read(Some(tool), Some(capture)),
                PromptRead::NoKnownPrompt
            );
        }
        let model = InputModel::StyleDelimited;
        assert_eq!(tool.input_model(), model);
        assert_eq!(
            region::occupancy(CODEX_IDLE, model),
            region::Occupancy::Idle
        );
        assert_eq!(
            region::occupancy(&draft, model),
            region::Occupancy::Occupied
        );
        assert_eq!(frame_read(tool, CODEX_IDLE), Frame::Idle);
        assert_eq!(frame_read(tool, &draft), Frame::Held);

        let dir = scratch("draft");
        let seat = seat_in(&dir);
        let typed = Captured {
            declared: "waiting-user",
            early: CODEX_IDLE.to_owned(),
            late: draft,
            frames: std::cell::Cell::new(0),
        };
        let (charter, pasted) = attempt_with(&dir, &seat, &typed);
        assert_eq!(charter, Charter::Attempted);
        assert_eq!(
            pasted, None,
            "the late gate composes nothing over the draft"
        );
        assert_eq!(typed.frames.get(), 2, "each gate read one frame");
        let after = meta(&dir);
        assert!(!after.contains("charter_pending.main="), "{after}");
        assert!(!after.contains("charter.main="), "{after}");
        assert_eq!(summaries(&dir), [Receipt::BeforeSubmit.summary()]);

        let idle = Captured {
            declared: "waiting-user",
            early: CODEX_IDLE.to_owned(),
            late: CODEX_IDLE.to_owned(),
            frames: std::cell::Cell::new(0),
        };
        let (_, pasted) = attempt_with(&dir, &seat, &idle);
        assert!(
            pasted.is_some_and(|text| text.starts_with(HEAD)),
            "the placeholder at both gates gets the turn"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tool_whose_frame_ae_cannot_read_is_unread_without_a_capture() {
        let tool = crate::tool::ToolKind::Muse;
        assert!(!crate::harness_state::observable(tool));
        let dir = scratch("unread");
        let seat = seat_in(&dir);
        let server = crate::inventory::ServerId::Selected(crate::meta::Selector::Name(
            "ae-charter-no-such-server".to_owned(),
        ));
        let live = Live {
            meta_dir: &dir,
            session: "orch",
            server: &server,
            seat: &seat,
            tool,
        };
        assert_eq!(live.frame(), Frame::Unread);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_landed_turn_stamps_and_a_refused_one_frees_the_claim() {
        let dir = scratch("settle");
        let version = crate::config::SEAT_CHARTER_VERSION;
        assert!(crate::meta::claim_charter(&dir, SLOT, "tok", version).expect("claim"));
        settle(&dir, "tok", Receipt::BeforeSubmit).expect("released");
        assert!(!meta(&dir).contains("charter_pending.main="));
        assert!(crate::meta::claim_charter(&dir, SLOT, "tok", version).expect("reclaim"));
        settle(&dir, "tok", Receipt::Landed).expect("stamped");
        let after = meta(&dir);
        assert!(
            after.contains(&format!("charter.main={version}\n")),
            "{after}"
        );
        assert!(after.contains("charter_launch.main=tok\n"), "{after}");
        assert!(!after.contains("charter_pending.main="), "{after}");
        assert!(
            !crate::meta::claim_charter(&dir, SLOT, "tok", version).expect("stamped claim"),
            "a launch that holds the version is never claimed again"
        );
        assert!(
            !crate::meta::claim_charter(&dir, SLOT, "other", version).expect("stale launch"),
            "a launch the meta no longer records is never claimed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
