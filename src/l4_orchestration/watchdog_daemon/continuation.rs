//! The launch-bound continuation of a carried or kept automatic reseat.

use super::cycle_apply::DeskClaim;
use super::{ACTOR, Accounting, Acting, Cycle, Effect, Event, Journal, Observation, PaneState};
use crate::autoreseat::{Fate, NOTICE_ACTION, leg};
use crate::meta::{
    self, ContinuationFate, ContinuationPhase, ContinuationStep, ReseatContinuation,
};
use crate::watchdog::MAIN_SLOT;
use std::io::Write;

/// The normal sweep still decides if an overview is due. The continuation
/// owns its own delivery gate and uses the same turn when both are due.
#[cfg(test)]
pub(super) fn book(
    booked: &mut Accounting,
    seen: &Observation,
    owed: ReseatContinuation,
    settling: bool,
) {
    book_from(booked, seen, (owed, false), settling);
}

/// [`book`], with the obligation's origin read from the same meta bytes
/// ([`Cycle::read_reseat_obligation`]): it rides on the booked effect.
pub(super) fn book_from(
    booked: &mut Accounting,
    seen: &Observation,
    (owed, requested): (ReseatContinuation, bool),
    settling: bool,
) {
    let mut overview = false;
    if owed.phase == ContinuationPhase::Owed {
        // Only a changed overview rides in the continuation. A desk-stale
        // tick waits for a later free cycle, its change still unconsumed.
        overview = booked.effects.contains(&Effect::SweepNudge)
            && seen
                .sweep
                .as_ref()
                .is_some_and(|sweep| sweep.overview_changed);
        booked
            .effects
            .retain(|effect| *effect != Effect::SweepNudge);
        if settling
            || !seen.capture_ok
            || seen.human_prompt.is_some()
            || seen.is_dead
            || booked.next.dead_latched
            || seen.harness.frame.is_busy()
            || seen.harness.human_draft
        {
            return;
        }
        // When the sweep cadence is disabled, the ordinary ladder can book
        // another turn. This cycle's eligible continuation owns the paste.
        booked
            .effects
            .retain(|effect| !effect.held_while_settling());
    }
    booked.effects.push(Effect::ReseatContinuation {
        owed,
        overview,
        requested,
    });
}

impl Cycle<'_> {
    pub(super) fn read_reseat_continuation(&self) -> Option<ReseatContinuation> {
        self.read_reseat_obligation().map(|(owed, _)| owed)
    }

    /// The obligation and its origin, from ONE read of the meta: whether the
    /// move that started its launch was the orchestrator's own request (the
    /// launch-bound row the same publication wrote). The origin travels with
    /// the obligation through Claim and Confirm; it is never read again.
    pub(super) fn read_reseat_obligation(&self) -> Option<(ReseatContinuation, bool)> {
        if !self.meta_agent {
            return None;
        }
        let launch = super::launch_id_for(&self.launch_ids, MAIN_SLOT)?;
        let bytes = meta::read_bytes(self.meta_dir).ok()?;
        let owed = meta::reseat_continuation(&bytes).filter(|row| row.launch == launch)?;
        let requested = meta::requested_continuation(&bytes, &owed.launch);
        Some((owed, requested))
    }

    pub(super) fn reseat_continuation(
        &self,
        owed: &ReseatContinuation,
        overview: bool,
        requested: bool,
        on: &Acting<'_>,
        state: &mut PaneState,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let fate = match owed.fate {
            ContinuationFate::Carried => Fate::Carried,
            ContinuationFate::Kept => Fate::Kept,
        };
        // A requested move's continuation says it was the seat's own request.
        let (mut body, summary) = if requested {
            crate::reseat::request::continuation(fate)
        } else {
            leg::continuation(fate)
        };
        if owed.phase == ContinuationPhase::Claimed {
            if confirmed(on.events, on.agent, &owed.launch, &summary) {
                self.advance_continuation(owed, ContinuationStep::Confirm, err)?;
            } else {
                self.tell_unknown_continuation(owed, on, err)?;
            }
            return Ok(());
        }
        let mut overview = on.overview.filter(|_| overview);
        if !self.advance_continuation(owed, ContinuationStep::Claim, err)? {
            return Ok(());
        }
        // A due desk-stale line rides only with a changed overview, claimed
        // after the continuation's own claim and settled by the same outcome.
        // A tick or overview landed since the observation holds the joined
        // overview; the continuation still goes with its own text.
        let desk = match (overview, state.desk.take()) {
            (Some(_), Some(booking)) => match self.claim_desk(booking, on.slot, err)? {
                DeskClaim::Claimed(previous, claim) => Some((previous, claim)),
                DeskClaim::Skipped => None,
                DeskClaim::Held => {
                    overview = None;
                    None
                }
            },
            _ => None,
        };
        // A joined overview without a desk claim rechecks the shared spacing
        // it was booked under; the continuation goes either way.
        if overview.is_some() && desk.is_none() && !self.spacing_unmoved(on, err)? {
            overview = None;
        }
        if let Some(overview) = overview {
            body.push_str("\n\n");
            body.push_str(&if desk.is_some() {
                overview.body_with(crate::desktick::LINE)
            } else {
                overview.body()
            });
        }
        let mut claimed = owed.clone();
        claimed.phase = ContinuationPhase::Claimed;
        let delivery = self.deliver(
            on.agent,
            &body,
            NOTICE_ACTION,
            &summary,
            Some(&claimed.launch),
            Some(on.slot),
        );
        if let Some((previous, claim)) = &desk {
            self.settle_desk(previous, claim, &delivery, err)?;
        }
        let events = super::read_events(self.meta_dir);
        let new_events = events.get(on.events.len()..).unwrap_or_default();
        let delivered =
            delivery.code == Some(0) && confirmed(new_events, on.agent, &claimed.launch, &summary);
        if delivered {
            self.advance_continuation(&claimed, ContinuationStep::Confirm, err)?;
        } else if delivery.code.is_some_and(|code| code != 0)
            && delivery
                .stdout
                .lines()
                .any(|line| line == crate::send::RETRYABLE_MARKER)
        {
            if self.advance_continuation(&claimed, ContinuationStep::Reopen, err)? {
                let previous = state.continuation_refused.replace(owed.launch.clone());
                if previous.as_ref() == Some(&owed.launch) {
                    let reason = refusal_reason(new_events, on.agent, &owed.launch, &summary);
                    self.tell_continuation(
                        owed,
                        on,
                        &format!("reseat-continuation-held:{}", owed.launch),
                        &format!(
                            "{} is still waiting for its automatic reseat continuation. The send helper reports {reason}. Check the pane. The watchdog will retry.",
                            on.agent
                        ),
                        err,
                    )?;
                }
            }
        } else {
            self.tell_unknown_continuation(&claimed, on, err)?;
        }
        if let Some(overview) = overview {
            self.record_overview(on, state, overview, delivered, err)?;
        }
        Ok(())
    }

    fn advance_continuation(
        &self,
        expected: &ReseatContinuation,
        step: ContinuationStep,
        err: &mut impl Write,
    ) -> crate::error::Result<bool> {
        match meta::advance_reseat_continuation(self.meta_dir, expected, step) {
            Ok(changed) => Ok(changed),
            Err(why) => {
                writeln!(
                    err,
                    "ae: watchdog: automatic reseat continuation: {}",
                    why.cause()
                )?;
                Ok(false)
            }
        }
    }

    fn tell_unknown_continuation(
        &self,
        owed: &ReseatContinuation,
        on: &Acting<'_>,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        self.tell_continuation(
            owed,
            on,
            &format!("reseat-continuation:{}", owed.launch),
            &format!(
                "{} may have received its automatic reseat continuation. Check the pane before sending another turn.",
                on.agent
            ),
            err,
        )
    }

    fn tell_continuation(
        &self,
        owed: &ReseatContinuation,
        on: &Acting<'_>,
        reference: &str,
        message: &str,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let told = on.events.iter().any(|event| {
            event.actor == ACTOR
                && event.action == "chat"
                && event.target.is_none()
                && event.reference.as_deref() == Some(reference)
        });
        if !told && self.read_reseat_continuation().as_ref() == Some(owed) {
            Journal {
                meta_dir: self.meta_dir,
                session: self.session,
            }
            .record_referring("chat", "", reference, message, err)?;
        }
        Ok(())
    }
}

fn confirmed(events: &[Event], agent: &str, launch: &str, summary: &str) -> bool {
    events.iter().any(|event| {
        event.actor == ACTOR
            && event.action == NOTICE_ACTION
            && event.target.as_deref() == Some(agent)
            && event.reference.as_deref() == Some(launch)
            && event.summary.as_deref() == Some(summary)
            && event.unverifiable.is_none()
    })
}

/// An abandoned helper turn records what held it. The typed refusal marker
/// also covers failures that write no reason to the journal.
fn refusal_reason(events: &[Event], agent: &str, launch: &str, summary: &str) -> String {
    let reason = events
        .iter()
        .rev()
        .filter(|event| {
            event.actor == ACTOR
                && event.action == crate::tracked::ABANDONED_ACTION
                && event.target.as_deref() == Some(agent)
                && event.reference.as_deref() == Some(launch)
        })
        .find_map(|event| {
            let text = event.summary.as_deref()?.strip_prefix("refused: ")?;
            let (reason, sent) = text.rsplit_once("; ")?;
            (sent == summary).then_some(reason)
        })
        .unwrap_or("a refusal before paste");
    crate::board::terminal_text(reason)
}
