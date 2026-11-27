//! Deliver dated reset notices to this session's main lead.

use std::io::Write as _;

use super::{Cycle, Journal, QuotaInputs, Timestamp};
use crate::quota::{Observation, ResetExpiryNotice, action::RESET_EXPIRY, reset_expiry_notices};
use crate::store;
use crate::watchdog::MAIN_SLOT;

impl Cycle<'_> {
    pub(super) fn tell_reset_expiries(&self, observation: &Observation, inputs: &QuotaInputs<'_>) {
        let Some(lead) = self.roster.iter().find(|seat| seat.slot == MAIN_SLOT) else {
            return;
        };
        // Most cycles have no due dates or already carry their receipts.
        if reset_expiry_notices(observation, inputs.events).is_empty() {
            return;
        }
        let lock = store::lock_path(&self.meta_dir.join(RESET_EXPIRY));
        let Ok(_held) = store::lock(&lock, store::LOCK_WAIT) else {
            return;
        };
        let Some(events) = self.notice_events() else {
            return;
        };
        for notice in reset_expiry_notices(observation, &events) {
            self.tell_reset_expiry(lead, inputs.panes, &notice);
        }
    }

    fn tell_reset_expiry(
        &self,
        lead: &crate::meta::RosterEntry,
        panes: &[crate::tmux::WatchPane],
        notice: &ResetExpiryNotice,
    ) {
        let Some((mut request, launch)) = self.lead_notice_request(
            (lead, panes),
            (RESET_EXPIRY, &notice.reference, &notice.text),
        ) else {
            return;
        };
        request.defer = std::time::Duration::ZERO;
        let mut err = std::io::stderr();
        let Ok(Some(guard)) = crate::tracked::admit_leg(&request, (&lead.slot, launch), &mut err)
        else {
            return;
        };
        let journal = Journal {
            meta_dir: self.meta_dir,
            session: self.session,
        };
        // Claim under the target lock after the quiet check, before typing.
        // The journal carries the full notice if the process stops before
        // submit. Its durable receipt also prevents a duplicate after submit.
        let compose = || {
            if Timestamp::now().epoch() >= notice.expiry {
                return None;
            }
            journal
                .append_referring(
                    RESET_EXPIRY,
                    (&lead.name, &lead.slot),
                    &notice.reference,
                    &notice.text,
                )
                .inspect_err(|why| {
                    let _ = writeln!(
                        std::io::stderr(),
                        "ae: watchdog: {RESET_EXPIRY} for {} not recorded: {why}",
                        lead.name
                    );
                })
                .ok()?;
            Some(notice.text.clone())
        };
        let _ = crate::deliver::deliver_composed_checked(&request, Some(&guard), compose, &mut err);
    }
}
