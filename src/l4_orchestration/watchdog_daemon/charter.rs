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
    /// the seat is proved, its newest declaration is `done`, the charter its
    /// launch recorded is older than [`crate::config::SEAT_CHARTER_VERSION`],
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
        events: &[Event],
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
        let target = crate::config::SEAT_CHARTER_VERSION;
        if settling || seat.running >= target || !self.declared_done(events, &seat) {
            return Ok(Charter::Idle);
        }
        let Some(prompt) = charter_prompt(&seat.overlay) else {
            return Ok(Charter::Idle);
        };
        match crate::meta::claim_charter(self.meta_dir, SLOT, &seat.launch, target) {
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
        let receipt = self.charter_deliver(&seat, &reference, &text, err)?;
        if let Err(why) = settle(self.meta_dir, &seat.launch, receipt) {
            writeln!(
                err,
                "ae: watchdog: {}, but its claim could not be settled ({}); it is not sent again",
                receipt.summary(),
                why.cause()
            )?;
        }
        Journal {
            meta_dir: self.meta_dir,
            session: self.session,
        }
        .record_referring(
            crate::watchdog::CHARTER_UPDATE_ACTION,
            &seat.agent,
            &reference,
            receipt.summary(),
            err,
        )?;
        Ok(Charter::Attempted)
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

    /// Whether the seat's newest declaration in this session's main slot is
    /// `done`.
    fn declared_done(&self, events: &[Event], seat: &Seat) -> bool {
        let key = crate::events::SessionKey::new(self.session, &seat.session_id);
        events
            .iter()
            .rev()
            .find(|event| {
                event.declared_state().is_some()
                    && crate::watchdog::event_is_actor_in(event, key, SLOT, &seat.agent)
            })
            .and_then(Event::declared_state)
            == Some("done")
    }

    /// One composed turn to the proved pane on this cycle's server, admitted
    /// in process as ae's own leg against the launch `seat` carries, and its
    /// typed receipt.
    fn charter_deliver(
        &self,
        seat: &Seat,
        reference: &str,
        text: &str,
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
        let delivery = crate::deliver::deliver_composed_checked(
            &request,
            Some(&guard),
            || Some(text.to_owned()),
            err,
        )?;
        Ok(Receipt::of(delivery.as_ref()))
    }
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
