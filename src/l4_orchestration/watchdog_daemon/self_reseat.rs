//! The orchestrator's queued reseat, as the watchdog's cycle sees its pane:
//! ONE turn at an idle frame, the owed refusal notice first, else the claim
//! of a pending request ([`crate::reseat::request`]).

use super::accounting::Accounting;
use super::{Acting, Cycle, Effect, Event, Observation};
use crate::harness_state::HarnessState;
use crate::reseat::request;
use std::io::Write;
use std::path::Path;

/// The message text the claim's private send carries; the trigger reads none.
const PLACEHOLDER: &str = "self-reseat";

/// Book the seat's one self-reseat turn this cycle, or nothing: only at a
/// positively idle frame with no draft, no prompt and no death, never while
/// the wake settles, and never while a continuation is owed to this launch.
/// The booked turn owns the cycle's paste: nothing else is pasted with it.
pub(super) fn book(
    booked: &mut Accounting,
    seen: &Observation,
    events: &[Event],
    (dir, seat): (&Path, (&str, &str)),
    (settling, continuation): (bool, bool),
) {
    if settling
        || continuation
        || !seen.capture_ok
        || seen.human_prompt.is_some()
        || seen.is_dead
        || booked.next.dead_latched
        || seen.harness.frame != HarnessState::Idle
        || seen.harness.human_draft
    {
        return;
    }
    let effect = if let Some(ending) = request::owed_notice(events, seat) {
        Effect::SelfReseatNotice {
            reference: ending.reference.clone().unwrap_or_default(),
            text: request::notice_text(&ending, seat),
        }
    } else if request::pending(dir) && !request::auto_in_flight(events, seat) {
        Effect::SelfReseatClaim
    } else {
        return;
    };
    booked
        .effects
        .retain(|effect| !effect.held_while_settling());
    booked.effects.push(effect);
}

impl Cycle<'_> {
    /// Hand the pending request to the trigger through the one exec site.
    pub(super) fn self_reseat_claim(
        &self,
        on: &Acting<'_>,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let delivery = self.deliver(
            on.agent,
            PLACEHOLDER,
            request::ATTEMPT_ACTION,
            "",
            None,
            None,
        );
        if delivery.code != Some(0) {
            writeln!(
                err,
                "ae: watchdog: self reseat of {} not started this cycle",
                on.agent
            )?;
        }
        Ok(())
    }

    /// Deliver the owed refusal or failure as the seat's turn, bound to its
    /// launch. The helper's own record under [`request::NOTICE_ACTION`] and
    /// `reference` is the receipt; a refused delivery stays owed.
    pub(super) fn self_reseat_notice(
        &self,
        on: &Acting<'_>,
        (reference, text): (&str, &str),
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let summary = format!("told {} how its requested reseat ended", on.agent);
        let delivery = self.deliver(
            on.agent,
            text,
            request::NOTICE_ACTION,
            &summary,
            Some(reference),
            Some(on.slot),
        );
        if delivery.code != Some(0) {
            writeln!(
                err,
                "ae: watchdog: the self reseat notice to {} is still owed",
                on.agent
            )?;
        }
        Ok(())
    }
}
