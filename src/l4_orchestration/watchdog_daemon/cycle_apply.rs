//! The cycle's applying steps: what the daemon does with a pane's booked effects, the auto reseat, the trust answers and the notices.

use super::{
    ACTOR, AUTO_PLACEHOLDER, Acting, AutoAct, AutoSeat, Carry, Cycle, DoneProgress, Effect, Event,
    HarnessState, Journal, OVERDUE_SUMMARY, Observation, PaneState, QuietKind, QuietQuery,
    QuietResolution, QuotaCarry, RETRY_PLACEHOLDER, SeatLaunch, SeatVerdict, SweepAlert, Timestamp,
    UNIX_EPOCH, Verdict, WaitCheck, WaitEnded, Write, age_secs, announced, auto_acts_with,
    declared_clause, done_challenge_text, entry_mut, epoch_second, latest_relevant_event,
    launch_id_for, limit_wake_text, naming, nudge_words, quiet_reason, read_events, record_nudge,
    record_sweep, slot_latched, stale_display, store, sweep_effects, system_time_from_epoch,
    target_moved_text, tmux, tracked, transport, wait_challenge_text,
};
use super::{DeathEpisode, deadnotice};

impl Cycle<'_> {
    /// The RESOLVED quiet suppression for one pane: `done` while its episode
    /// is live, a wait until the human's own input in the pane ends it. A wait
    /// that ended THIS cycle is returned with the suppression's absence, so
    /// the nudge that follows can name it.
    pub(super) fn resolve_quiet(
        &self,
        query: &QuietQuery<'_>,
        done_progress: DoneProgress,
    ) -> QuietResolution {
        let Some(relevant) =
            latest_relevant_event(query.events, self.session, query.slot, query.agent)
        else {
            return QuietResolution::NONE;
        };
        let Some(kind) = quiet_reason(&relevant) else {
            return QuietResolution::NONE;
        };
        if kind == QuietKind::Done {
            return QuietResolution {
                quiet: (!matches!(
                    done_progress,
                    DoneProgress::None | DoneProgress::Lapsed { .. }
                ))
                .then_some(kind),
                ended: None,
            };
        }
        let delivered = crate::watchdog::last_watchdog_delivery(
            query.events,
            self.session,
            query.slot,
            query.agent,
        );
        if crate::watchdog::human_input_ends_wait(query.activity, relevant.event.ts, delivered) {
            let input_age_secs = query
                .activity
                .and_then(|epoch| i64::try_from(epoch).ok())
                .map_or(0, |epoch| age_secs(query.now, epoch));
            return QuietResolution {
                quiet: None,
                ended: Some(WaitEnded {
                    kind,
                    input_age_secs,
                }),
            };
        }
        QuietResolution {
            quiet: Some(kind),
            ended: None,
        }
    }

    /// Perform one effect.
    #[allow(
        clippy::too_many_lines,
        reason = "one arm per effect, each the effect's whole delivery and record"
    )]
    pub(super) fn apply(
        &self,
        effect: &Effect,
        on: &Acting<'_>,
        state: &mut PaneState,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let agent = on.agent;
        match effect {
            // A panel's naming and an unreachable failure are keyed to their
            // tool start: what the quiet gate ages from, what a new start ends.
            Effect::Emit { action, summary }
                if crate::autotrust::panel_named(summary)
                    || (*action == "alert" && summary.starts_with(crate::events::UNREACHABLE)) =>
            {
                Journal {
                    meta_dir: self.meta_dir,
                    session: self.session,
                }
                .record_referring(
                    action,
                    agent,
                    on.seen.start.as_deref().unwrap_or_default(),
                    summary,
                    err,
                )
            }
            // The dead latch's records are routed to the seat's slot.
            Effect::Emit { action, summary } if deadnotice::routes_to_slot(action, summary) => {
                self.emit_at(action, (agent, on.slot), summary, err)
            }
            Effect::Emit { action, summary } => self.emit(action, agent, summary, err),
            Effect::Notify(text) => {
                self.notify(agent, text);
                Ok(())
            }
            Effect::Note(text) => {
                writeln!(err, "ae: watchdog: {agent}: {text}")?;
                Ok(())
            }
            Effect::AnswerTrust { name } => self.answer_trust(*name, on, state, err),
            Effect::SweepNudge => self.sweep_nudge(on, state, err),
            // Cycle-level by design: `run` collects it and performs the ONE
            // pass after the sweep, outside every per-pane path.
            Effect::QuotaRefresh => Ok(()),
            Effect::ReconcileWedge => {
                // The DURABLE half of the wedge clear.
                if crate::session::alert_reason_in(on.events, self.session, on.slot, agent)
                    .is_some()
                {
                    let alert = SweepAlert::ClearWedge;
                    self.emit(alert.action(), agent, &alert.summary(), err)?;
                }
                Ok(())
            }
            Effect::Nudge => {
                let idle_age = (on.seen.harness.frame == HarnessState::Idle)
                    .then(|| {
                        state
                            .idle_since_epoch
                            .map(|at| age_secs(on.seen.now_epoch, at))
                    })
                    .flatten();
                let display = stale_display(idle_age.unwrap_or(on.seen.last_actor_event_age_secs));
                let waiting = on.seen.own_work.reason();
                let (text, summary) = nudge_words(
                    self.goal.as_deref(),
                    self.meta_dir,
                    idle_age,
                    &display,
                    waiting.as_deref(),
                    declared_clause(on.events, self.session, on.slot, agent).as_deref(),
                    on.seen.ended_wait,
                );
                let code = self
                    .deliver(agent, &text, "nudge", &summary, None, Some(on.slot))
                    .code;
                self.reachable_again(on, "nudge", code, err)?;
                let delivered = code == Some(0);
                for effect in record_nudge(state, delivered, &self.knobs, &display) {
                    self.apply(&effect, on, state, err)?;
                }
                Ok(())
            }
            Effect::LimitWake { reset } => {
                let text = limit_wake_text(self.goal.as_deref(), self.meta_dir, *reset);
                let summary = format!(
                    "usage limit reset at {} — continuation turn",
                    Timestamp::from_epoch(*reset)
                );
                let reference = reset.to_string();
                let code = self
                    .deliver(
                        agent,
                        &text,
                        crate::watchdog::LIMIT_WAKE_ACTION,
                        &summary,
                        Some(&reference),
                        Some(on.slot),
                    )
                    .code;
                self.reachable_again(on, crate::watchdog::LIMIT_WAKE_ACTION, code, err)?;
                // The helper's success covers a delivered AND an unconfirmed
                // paste: either may have reached the seat, so the reset is
                // spent. A refusal pasted nothing and counts one attempt. The
                // nudge budget is not this path's: neither touches it.
                if code == Some(0) {
                    state.limit_woken = Some(*reset);
                } else {
                    state.limit_wake_attempts = state.limit_wake_attempts.saturating_add(1);
                }
                Ok(())
            }
            Effect::DoneChallenge {
                confirmations,
                required,
                done_age_secs,
            } => {
                let text = done_challenge_text(
                    self.goal.as_deref(),
                    self.meta_dir,
                    *done_age_secs,
                    *confirmations,
                    *required,
                );
                let summary = crate::watchdog::challenge_summary(
                    crate::watchdog::Challenge::Done,
                    confirmations.saturating_add(1),
                    *required,
                );
                let launch = launch_id_for(&self.launch_ids, on.slot);
                let code = self
                    .deliver(
                        agent,
                        &text,
                        "done-challenge",
                        &summary,
                        launch,
                        Some(on.slot),
                    )
                    .code;
                self.reachable_again(on, "done-challenge", code, err)
            }
            Effect::DoneHeld {
                confirmations,
                required,
            } => {
                // Journal only: the record IS the credit the fold admits, so
                // it carries the launch witness exactly as a challenge does.
                let summary = crate::watchdog::challenge_summary(
                    crate::watchdog::Challenge::Done,
                    confirmations.saturating_add(1),
                    *required,
                );
                Journal {
                    meta_dir: self.meta_dir,
                    session: self.session,
                }
                .record_referring(
                    crate::watchdog::DONE_HELD_ACTION,
                    agent,
                    launch_id_for(&self.launch_ids, on.slot).unwrap_or_default(),
                    &summary,
                    err,
                )
            }
            Effect::WaitHeld { reason, on: target } => {
                // Journal only, keyed to the launch witness as a challenge
                // is; the wait fold reads it back through
                // `Event::wait_held` (#225).
                let held = match on.seen.quiet {
                    Some(QuietKind::Blocked) => crate::watchdog::WaitState::Blocked,
                    _ => crate::watchdog::WaitState::WaitingAgent,
                };
                let summary = crate::watchdog::wait_held_summary(held, *reason, target.as_deref());
                Journal {
                    meta_dir: self.meta_dir,
                    session: self.session,
                }
                .record_referring(
                    crate::watchdog::WAIT_HELD_ACTION,
                    agent,
                    launch_id_for(&self.launch_ids, on.slot).unwrap_or_default(),
                    &summary,
                    err,
                )
            }
            Effect::TargetMoved { moved, age_secs } => {
                let target =
                    crate::session::declaration_of(on.events, self.session, on.slot, agent)
                        .and_then(|(declared, _)| declared.wait_on.as_deref())
                        .unwrap_or("its target");
                let (text, summary) = target_moved_text(
                    self.goal.as_deref(),
                    self.meta_dir,
                    *age_secs,
                    target,
                    moved,
                );
                let action = crate::watchdog::TARGET_MOVED_ACTION;
                let number = moved.ordinal.to_string();
                let code = self
                    .deliver(agent, &text, action, &summary, Some(&number), Some(on.slot))
                    .code;
                self.reachable_again(on, action, code, err)
            }
            Effect::WaitChallenge {
                confirmations,
                required,
                wait_age_secs,
                state,
                escalated,
            } => {
                let launch = launch_id_for(&self.launch_ids, on.slot);
                let owed = crate::watchdog::wait_goal_owed(
                    on.events,
                    self.session,
                    on.slot,
                    agent,
                    launch,
                );
                let target =
                    crate::session::declaration_of(on.events, self.session, on.slot, agent)
                        .and_then(|(declared, _)| declared.wait_on.as_deref());
                let text = wait_challenge_text(
                    self.goal.as_deref().filter(|_| owed),
                    self.meta_dir,
                    &WaitCheck {
                        age: *wait_age_secs,
                        confirmations: *confirmations,
                        required: *required,
                        state: *state,
                        escalated: *escalated,
                        target,
                    },
                );
                let num = confirmations.saturating_add(1);
                let summary = crate::watchdog::challenge_summary(
                    crate::watchdog::Challenge::Wait(*state),
                    num,
                    *required,
                );
                let code = self
                    .deliver(
                        agent,
                        &text,
                        "wait-challenge",
                        &summary,
                        launch,
                        Some(on.slot),
                    )
                    .code;
                self.reachable_again(on, "wait-challenge", code, err)
            }
        }
    }

    /// After a delivery to a seat whose unreachable failure stands: ONE
    /// `alert-cleared` once the helper's own record of `action` says it landed.
    fn reachable_again(
        &self,
        on: &Acting<'_>,
        action: &str,
        code: Option<i32>,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        if !on.seen.unreachable || code != Some(0) {
            return Ok(());
        }
        let events = read_events(self.meta_dir);
        let landed = events.iter().rev().find(|event| {
            event.action == action
                && crate::watchdog::event_is_addressed_to(event, self.session, on.slot, on.agent)
        });
        if landed.is_some_and(|event| !tracked::summary_is_unconfirmed(event.summary.as_deref())) {
            let summary = format!("{} — {action} landed", crate::events::REACHABLE_AGAIN);
            self.emit("alert-cleared", on.agent, &summary, err)?;
        }
        Ok(())
    }

    /// Deliver one sweep prompt and book what happened.
    pub(super) fn sweep_nudge(
        &self,
        on: &Acting<'_>,
        state: &mut PaneState,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let Some(_observed) = on.seen.sweep.as_ref() else {
            // Unreachable by construction: only the sweep branch emits this
            // effect, and it runs only where the observation exists.
            writeln!(
                err,
                "ae: watchdog: sweep prompt for {} had no sweep reading — skipped",
                on.agent
            )?;
            return Ok(());
        };
        let Some(overview) = on.overview else {
            // A failed fleet read suppresses the change gate, so this is also
            // unreachable by construction. Keep the refusal loud if those two
            // paths ever drift.
            writeln!(
                err,
                "ae: watchdog: sweep nudge for {} had no fleet overview — skipped",
                on.agent
            )?;
            return Ok(());
        };
        // Delivery is CHECKED.
        let body = overview.body();
        let delivered = self
            .deliver(
                on.agent,
                &body,
                crate::watchdog::SWEEP_NUDGE_ACTION,
                "fleet overview changed",
                None,
                Some(on.slot),
            )
            .code
            == Some(0);
        // This is intentionally AFTER the checked delivery. A deferred submit
        // cannot be acknowledged by a `done` event that preceded the paste,
        // and the minimum spacing begins when the paste actually landed.
        let settled_epoch = Timestamp::now().epoch();
        let settled_now = system_time_from_epoch(settled_epoch).unwrap_or(UNIX_EPOCH);
        let booked = record_sweep(&mut state.sweep, delivered, settled_now, &self.knobs.sweep);
        if delivered {
            let outstanding_since = state
                .sweep
                .outstanding_since
                .map_or_else(|| epoch_second(settled_now), epoch_second);
            if let Err(why) = crate::monitor::record_overview_delivery(
                self.meta_dir,
                &overview.hash,
                outstanding_since,
                settled_epoch,
            ) {
                writeln!(
                    err,
                    "ae: watchdog: delivered fleet overview but could not persist its hash: {why}"
                )?;
            }
        }
        for effect in sweep_effects(booked) {
            self.apply(&effect, on, state, err)?;
        }
        Ok(())
    }

    /// THE ONE PLACE THIS DAEMON EXECUTES ANYTHING, and the reason both nudges
    /// route through it rather than each spawning for itself: a second delivery
    /// site is a second thing to audit, and a unit guard in this file holds the
    /// count at one.
    ///
    /// `seat` is the slot the delivery was decided for: the helper then proves
    /// the incarnation this cycle's snapshot read there (#218 2c), and a row
    /// that snapshot could not prove is refused here, before anything runs.
    /// `None` is a leg that binds its own incarnation (the brief retry) or
    /// types nothing (the auto reseat trigger).
    pub(super) fn deliver(
        &self,
        agent: &str,
        text: &str,
        action: &str,
        summary: &str,
        reference: Option<&str>,
        seat: Option<&str>,
    ) -> transport::Delivery {
        let mut vars = vec![
            ("AE_SENDER_OVERRIDE", ACTOR),
            ("_AE_EVENT_ACTION", action),
            ("_AE_EVENT_SUMMARY", summary),
        ];
        vars.extend(reference.map(|value| ("_AE_EVENT_REF", value)));
        if let Some(slot) = seat {
            let row = match self.launch_ids.iter().find(|(at, _)| at == slot) {
                None => tracked::LaunchRow::Legacy,
                Some((_, SeatLaunch::Id(id))) => tracked::LaunchRow::Token(id),
                Some((_, SeatLaunch::Unproven(why))) => tracked::LaunchRow::Damaged(why),
            };
            match tracked::expected_members(slot, row) {
                Ok(members) => vars.extend(members),
                Err(why) => {
                    let seat = (agent, slot);
                    let mut err = std::io::stderr();
                    let _ =
                        tracked::refuse_unproven_seat(self.meta_dir, ACTOR, seat, why, &mut err);
                    return transport::Delivery {
                        code: Some(1),
                        stdout: String::new(),
                    };
                }
            }
        }
        transport::deliver(self.helper.path(), agent, text, false, &vars)
    }

    /// The auto reseat step: journal each hold and overdue failure the pure
    /// core decides, then hand its ONE due seat to the trigger through the one
    /// exec site.
    pub(super) fn auto_reseat(
        &self,
        named: &mut Vec<(String, Timestamp, crate::autoreseat::HoldReason)>,
        seats: (&[AutoSeat], &[(String, crate::autoreseat::leg::HomeRead)]),
        events: &[Event],
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let context = (self.session, self.meta_agent, self.lead_pair);
        for act in auto_acts_with(&self.auto, seats, events, context, now, named) {
            let AutoAct::Trigger { agent } = &act else {
                self.auto_record(&act, now, err)?;
                continue;
            };
            let attempt = crate::autoreseat::ATTEMPT_ACTION;
            let delivery = self.deliver(agent, AUTO_PLACEHOLDER, attempt, "", None, None);
            if delivery.code != Some(0) {
                writeln!(
                    err,
                    "ae: watchdog: auto reseat of {agent} not started this cycle"
                )?;
            }
        }
        Ok(())
    }

    /// Journal one hold, overdue failure, headroom opening, re-arm or give-up
    /// under the seat's lock, and only while the journal as it reads NOW still
    /// owes it: a trigger may have opened an attempt, or the leg ended one,
    /// since this cycle's read.
    #[allow(
        clippy::too_many_lines,
        reason = "one ordered writer per act, each re-reading the journal under the lock"
    )]
    pub(super) fn auto_record(
        &self,
        act: &AutoAct,
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        use crate::autoreseat::Trigger;
        let (slot, agent) = match act {
            AutoAct::Held { slot, agent, .. }
            | AutoAct::Overdue { slot, agent, .. }
            | AutoAct::Open { slot, agent, .. }
            | AutoAct::Rearm { slot, agent, .. }
            | AutoAct::GaveUp { slot, agent, .. }
            | AutoAct::OpenReturn { slot, agent, .. }
            | AutoAct::Flapped { slot, agent, .. }
            | AutoAct::Unmapped { slot, agent, .. } => (slot, agent),
            AutoAct::Trigger { .. } => return Ok(()),
        };
        let lock = crate::autoreseat::lock_path(self.meta_dir, slot);
        let Ok(_held) = store::lock(&lock, std::time::Duration::ZERO) else {
            return Ok(());
        };
        let events = read_events(self.meta_dir);
        let journal = Journal {
            meta_dir: self.meta_dir,
            session: self.session,
        };
        let headroom = crate::autoreseat::headroom_episode(&events, self.session, slot, agent);
        match act {
            AutoAct::Open { cause, .. } => {
                if headroom.is_none() {
                    journal.record(crate::autoreseat::HEADROOM_ACTION, agent, cause, err)?;
                }
                return Ok(());
            }
            AutoAct::Rearm { key, summary, .. } => {
                if headroom.is_some_and(|open| open.key == *key && open.open.is_none()) {
                    let action = crate::autoreseat::REARMED_ACTION;
                    journal.record_referring(action, agent, &key.to_string(), summary, err)?;
                }
                return Ok(());
            }
            AutoAct::OpenReturn { cause, .. } => {
                let back = crate::autoreseat::return_episode(&events, self.session, slot, agent);
                let chain = crate::autoreseat::chain(&events, self.session, slot, agent);
                if back.is_none() && chain.is_some() {
                    journal.record(crate::autoreseat::RETURN_ACTION, agent, cause, err)?;
                }
                return Ok(());
            }
            AutoAct::Flapped { key, origin, .. } => {
                let said = crate::autoreseat::flapped_summary(self.session, agent, origin);
                let reference = key.to_string();
                let held = events.iter().any(|event| {
                    event.action == crate::autoreseat::HELD_ACTION
                        && event.actor == crate::watchdog::WATCHDOG_ACTOR
                        && event.reference.as_deref() == Some(reference.as_str())
                        && event.summary.as_deref() == Some(said.as_str())
                        && crate::watchdog::event_is_addressed_to(event, self.session, slot, agent)
                });
                if !held {
                    let action = crate::autoreseat::HELD_ACTION;
                    journal.record_referring(action, agent, &reference, &said, err)?;
                    crate::autoreseat::leg::tell_held(
                        self.meta_dir,
                        (self.session, slot, agent),
                        *key,
                        Timestamp::from_epoch(now),
                        &said,
                    );
                }
                return Ok(());
            }
            AutoAct::GaveUp { key, .. } => {
                let said = crate::autoreseat::GAVE_UP_SUMMARY;
                let found = crate::autoreseat::episode(&events, self.session, slot, agent)
                    .filter(|open| open.key == *key)
                    .or_else(|| {
                        crate::autoreseat::return_episode(&events, self.session, slot, agent)
                    });
                if found.is_some_and(|open| {
                    open.key == *key && open.gave_up() && !open.holds_with(said)
                }) {
                    let action = crate::autoreseat::HELD_ACTION;
                    journal.record_referring(action, agent, &key.to_string(), said, err)?;
                    crate::autoreseat::leg::tell_gave_up(
                        self.meta_dir,
                        (self.session, slot, agent),
                        *key,
                        Timestamp::from_epoch(now),
                    );
                }
                return Ok(());
            }
            AutoAct::Unmapped { key, profile, .. } => {
                let summary = crate::autoreseat::leg::ineligible_record(
                    crate::autoreseat::Ineligible::Unmapped,
                );
                let found = crate::autoreseat::episode(&events, self.session, slot, agent);
                if crate::autoreseat::owed(found.as_ref(), *key, false, now)
                    && !found.as_ref().is_some_and(|open| open.holds_with(&summary))
                {
                    journal.record_referring(
                        crate::autoreseat::HELD_ACTION,
                        agent,
                        &key.to_string(),
                        &summary,
                        err,
                    )?;
                    crate::autoreseat::leg::tell_unmapped(
                        self.meta_dir,
                        (self.session, slot, agent),
                        profile,
                        *key,
                        Timestamp::from_epoch(now),
                    );
                }
                return Ok(());
            }
            AutoAct::Held { .. } | AutoAct::Overdue { .. } | AutoAct::Trigger { .. } => {}
        }
        let (key, overdue, action, summary, trigger) = match act {
            AutoAct::Held {
                key,
                reason,
                trigger,
                ..
            } => (
                *key,
                false,
                crate::autoreseat::HELD_ACTION,
                reason.record(),
                *trigger,
            ),
            AutoAct::Overdue { key, trigger, .. } => (
                *key,
                true,
                crate::autoreseat::FAILED_ACTION,
                crate::autoreseat::CauseClass::Other.lead(OVERDUE_SUMMARY),
                *trigger,
            ),
            _ => return Ok(()),
        };
        let found = match trigger {
            Trigger::Limit => crate::autoreseat::episode(&events, self.session, slot, agent),
            Trigger::Headroom => headroom,
            Trigger::Return => {
                crate::autoreseat::return_episode(&events, self.session, slot, agent)
            }
        };
        let cause = found
            .as_ref()
            .map(|open| open.cause().to_owned())
            .unwrap_or_default();
        let summary = crate::autoreseat::leg::tagged(&summary, &cause);
        let said = trigger == Trigger::Headroom
            && !overdue
            && found.as_ref().is_some_and(|open| open.holds_with(&summary));
        if !said && crate::autoreseat::owed(found.as_ref(), key, overdue, now) {
            journal.record_referring(action, agent, &key.to_string(), &summary, err)?;
        }
        Ok(())
    }

    /// Journal the note an unusable `auto_reseat_at`, `auto_reseat_lead_at` or
    /// `auto_reseat_lead_floor` left, target-less, once per daemon start and
    /// note: the operator sees the threshold ae fell back to, or the floor that
    /// holds every lead seat, without a move having to say it.
    pub(super) fn auto_note(
        &self,
        noted: &mut Vec<String>,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let journal = Journal {
            meta_dir: self.meta_dir,
            session: self.session,
        };
        for note in [
            &self.auto.headroom_note,
            &self.auto.lead.note,
            &self.auto.floor_note,
        ]
        .into_iter()
        .flatten()
        {
            if noted.contains(note) {
                continue;
            }
            noted.push(note.clone());
            journal.record(crate::autoreseat::NOTICE_ACTION, "", note, err)?;
        }
        Ok(())
    }

    /// Name an unusable `auto_trust` value once per daemon start: it answers
    /// nothing, exactly like `off`.
    pub(super) fn trust_note(
        &self,
        noted: &mut Vec<String>,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let crate::config::AutoTrust::Unusable(note) = &self.auto_trust else {
            return Ok(());
        };
        if noted.contains(note) {
            return Ok(());
        }
        noted.push(note.clone());
        writeln!(err, "ae: watchdog: auto_trust {note}; it answers nothing")?;
        Ok(())
    }

    /// Whether this cycle's prompt is a trust block that is this seat's to
    /// answer — asked only with `auto_trust` on and a measured block drawn.
    pub(super) fn trust_due(
        &self,
        seen: &Observation,
        (slot, pane): (&str, &str),
        tool: Option<&str>,
        input: Option<i64>,
    ) -> Option<crate::autotrust::Due> {
        let prompt = seen
            .human_prompt
            .as_ref()
            .filter(|_| self.auto_trust.answers())?;
        let seat = self.trust_seat(slot, pane, tool?);
        if prompt.dismiss {
            return Some(crate::autotrust::dismiss_due(&seat, input));
        }
        Some(crate::autotrust::due(&seat, prompt.trust.as_ref()?))
    }

    fn trust_seat<'s>(
        &'s self,
        slot: &'s str,
        pane: &'s str,
        tool: &'s str,
    ) -> crate::autotrust::Seat<'s> {
        crate::autotrust::Seat {
            dir: self.meta_dir,
            server: self.server,
            session: self.session,
            slot,
            pane,
            tool,
        }
    }

    /// Answer the trust block on `on`'s pane; an answer that does not land
    /// names the prompt exactly as today when it is still to be named.
    fn answer_trust(
        &self,
        name: bool,
        on: &Acting<'_>,
        state: &mut PaneState,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let tool = self.agent_bin(on.slot).unwrap_or_default();
        let seat = self.trust_seat(on.slot, on.pane, &tool);
        // A panel is named at the bound already (`name` is false): the clear
        // that follows a landed Escape is 5c's, so the streak is left to it.
        let (ask, word) = if on.seen.human_prompt.as_ref().is_some_and(|it| it.dismiss) {
            let input = state
                .client_activity
                .and_then(|(_, at)| i64::try_from(at).ok());
            (crate::deliver::Ask::Panel(input), "auto-dismiss")
        } else {
            (crate::deliver::Ask::Trust, "auto-trust")
        };
        match crate::deliver::answer_trust(&seat, ask, err) {
            crate::deliver::TrustAnswer::Trusted(summary) => {
                self.notify(on.agent, &format!("{word}: {summary}"));
                // Never named, so nothing is to clear: a modal still drawn is
                // counted again and, its start now spent, named at the bound.
                if name {
                    state.human_prompt_streak = 0;
                }
                return Ok(());
            }
            crate::deliver::TrustAnswer::Declined(why) if name => writeln!(
                err,
                "ae: watchdog: {}: auto_trust did not answer the trust prompt: {why}",
                on.agent
            )?,
            _ => {}
        }
        if let Some(prompt) = on.seen.human_prompt.as_ref().filter(|_| name) {
            for effect in naming(prompt) {
                self.apply(&effect, on, state, err)?;
            }
        }
        Ok(())
    }

    /// The seat at `slot`'s own binding reset, from the held quota
    /// observation; `None` when no roster row or reading names one.
    pub(super) fn own_reset(
        &self,
        quota: &QuotaCarry,
        slot: &str,
        pin: Option<&str>,
        now: i64,
    ) -> Option<crate::autoreseat::LimitReset> {
        let entry = self.roster.iter().find(|entry| entry.slot == slot)?;
        crate::autoreseat::leg::own_reset(quota.last_observation.as_ref(), entry, pin, now)
    }

    /// The return the seat at `slot` may make this cycle, from the held quota
    /// observation: none while auto reseat is off or the seat is latched.
    pub(super) fn home(
        &self,
        events: &[Event],
        slot: &str,
        quota: &QuotaCarry,
        unlatched: bool,
        now: i64,
    ) -> Option<crate::autoreseat::leg::HomeRead> {
        if self.auto.switch == crate::autoreseat::Switch::Off || !unlatched {
            return None;
        }
        let row = self.roster.iter().find(|entry| entry.slot == slot)?;
        let latched =
            crate::autoreseat::latched_identities(&self.roster, events, self.session, slot);
        let read = (quota.last_observation.as_ref(), latched.as_slice());
        let auto = self.auto.seat_view(self.lead_pair, slot);
        crate::autoreseat::leg::home(self.meta_dir, events, (self.session, row), &auto, read, now)
    }

    /// The seat at `slot`'s own account against the headroom threshold, from
    /// the held quota observation; unread whenever headroom may not act.
    pub(super) fn own_room(
        &self,
        quota: &QuotaCarry,
        slot: &str,
        pin: Option<&str>,
        now: i64,
    ) -> crate::autoreseat::Room {
        let observed = quota.last_observation.as_ref();
        let on = self.auto.switch != crate::autoreseat::Switch::Off;
        let auto = self.auto.seat_view(self.lead_pair, slot);
        match (on, self.roster.iter().find(|entry| entry.slot == slot)) {
            (true, Some(entry)) => match auto.headroom_at {
                Some(at) => crate::autoreseat::leg::own_room(observed, entry, pin, at, now),
                None => crate::autoreseat::leg::own_relief(observed, entry, pin, now),
            },
            _ => crate::autoreseat::Room::Unread,
        }
    }

    /// One brief-retry pass: set aside what is permanently damaged, then spend
    /// this cycle's ONE delivery attempt on the next record in the rotation.
    ///
    /// Records are found by NAMING each roster slot's own path — never by
    /// listing the directory — so a legacy `undelivered.*.txt`, which carries
    /// no record, stays inert forever, and a file planted at any other name is
    /// never read.
    ///
    /// Damage classification is deliberately OUTSIDE the one-per-cycle budget.
    /// A single unreadable record that happens to be the oldest would otherwise
    /// eat the cycle's only attempt every cycle and starve every deliverable
    /// brief behind it for the whole half hour.
    ///
    /// THE ROTATION, and why oldest-first alone is not enough. Oldest-created
    /// goes first, because the brief that has waited longest should. But the
    /// leg answers `Skip` for a seat that is not ready, and a Skip mutates
    /// NOTHING — no attempt, no mark, no timestamp — so a stuck oldest record
    /// would be recomputed and re-picked every cycle, and every newer
    /// deliverable brief behind it would wait out the whole 30-minute bound.
    /// One stuck seat in a batch spawn is enough to do it. So `cursor` holds
    /// the SLOT tried last and the scan starts after it, wrapping: still one
    /// delivery per cycle, and every record gets its turn within roster-size
    /// cycles. The slot rather than an index, because the roster grows and
    /// shrinks between cycles; a slot that is gone starts the scan at the
    /// front.
    pub(super) fn retry_briefs(
        &self,
        cursor: &mut Option<String>,
        verdicts: &[(String, SeatVerdict)],
        events: &[Event],
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let mut ready: Vec<(i64, String, String)> = Vec::new();
        for entry in &self.roster {
            // CHANNEL TWO of two. This cycle already judged the seat, so a
            // latched human-only prompt costs no capture to honour here. The
            // leg asks the pane again for itself — that is channel one, and it
            // is what covers a trigger that never came through this daemon.
            if slot_latched(verdicts, &entry.slot) {
                continue;
            }
            let Some(reading) = crate::brief_retry::read(self.meta_dir, &entry.slot) else {
                continue;
            };
            match reading {
                Ok(record) => ready.push((record.created, entry.slot.clone(), entry.name.clone())),
                Err(damaged) => {
                    self.set_damaged_aside(&entry.name, &entry.slot, damaged, events, now, err)?;
                }
            }
        }
        // The slot breaks a tie, so the order is TOTAL: two records written in
        // the same second must not swap places between cycles, or the rotation
        // could step over one of them forever.
        ready.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        if ready.is_empty() {
            return Ok(());
        }
        let after = cursor
            .as_deref()
            .and_then(|slot| ready.iter().position(|(_, at, _)| at == slot))
            .map_or(0, |at| at + 1);
        let (_, slot, name) = &ready[after % ready.len()];
        *cursor = Some(slot.clone());
        // The argv carries nothing that reaches the pane: the helper reads the
        // text AND the actor from the record. It is here only because the
        // helper's own grammar needs a message word.
        let delivery = self.deliver(
            name,
            RETRY_PLACEHOLDER,
            crate::brief_retry::RETRY_ACTION,
            "",
            None,
            None,
        );
        if delivery.code != Some(0) {
            // Every outcome the helper acts on, it has already recorded and
            // said out loud. A non-zero exit is the ordinary skip.
            writeln!(
                err,
                "ae: watchdog: brief retry for {name} did not deliver this cycle"
            )?;
        }
        Ok(())
    }

    /// The session-notice step: trigger the leg for every live, unlatched seat
    /// the fold finds a notice due for. The leg folds again under the target
    /// lock and journals every outcome, so a skip costs nothing here.
    pub(super) fn tell_seats(
        &self,
        verdicts: &[(String, SeatVerdict)],
        events: &[Event],
        now: i64,
    ) {
        let bytes = crate::meta::read_bytes(self.meta_dir).unwrap_or_default();
        let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
        let interval = self.knobs.interval_secs;
        let bound = i64::try_from(interval).map_or(i64::MAX, |secs| secs.saturating_mul(2));
        for entry in &self.roster {
            let live = verdicts.iter().any(|(slot, seat)| {
                *slot == entry.slot && !matches!(seat.verdict, Verdict::Dead | Verdict::HumanPrompt)
            });
            let seat = crate::session_notice::Seat::read(&meta, &bytes, &entry.slot, self.session);
            if live && crate::session_notice::due(events, &seat, now, bound).is_some() {
                let action = crate::session_notice::TRIGGER_ACTION;
                let interval = interval.to_string();
                let seat = Some(entry.slot.as_str());
                let _ = self.deliver(&entry.name, action, action, &interval, None, seat);
            }
        }
    }

    /// Move a record ae will never read aside, once, and say so in the ledger.
    ///
    /// Only PERMANENT damage is destroyed. A read that merely failed may be a
    /// passing `EMFILE`, and the next cycle may read it perfectly well, so it
    /// is left exactly where it is until its own mtime proves it was never
    /// going to be read.
    ///
    /// THE ORDER IS THE PROOF: the give-up is durable BEFORE the rename. A
    /// daemon killed between the two leaves the record in place WITH its
    /// event — the next cycle's dedupe finds the ref, skips the append and
    /// only renames, so nothing is silent and nothing is repeated. A failed
    /// append likewise leaves the record where it is, said on stderr, for the
    /// next cycle to retry.
    fn set_damaged_aside(
        &self,
        name: &str,
        slot: &str,
        damaged: crate::brief_retry::Damaged,
        events: &[Event],
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        if !crate::brief_retry::should_destroy(&damaged, now) {
            writeln!(
                err,
                "ae: watchdog: {name}'s brief record could not be read ({}) — left alone, it may read next cycle",
                damaged.kind
            )?;
            return Ok(());
        }
        // THE DESTINATION IS STABLE: the record's own mtime dates the fallback
        // name, so the ref the event carries names a destination every later
        // cycle recomputes exactly — the dedupe below cannot drift into a
        // second event. No usable stamp fails closed to plain-only.
        let destination =
            match crate::brief_retry::destination(self.meta_dir, slot, damaged.modified) {
                Ok(destination) => destination,
                // Never a loop: it says so once per cycle and changes nothing.
                Err(why) => {
                    writeln!(err, "ae: watchdog: {why}")?;
                    return Ok(());
                }
            };
        let destination_ref = destination.display().to_string();
        if !announced(events, &destination_ref) {
            // THE REF IS THE IDEMPOTENCE KEY: it names the destination, is
            // written uncapped, and the next cycle matches it exactly.
            let appended = Journal {
                meta_dir: self.meta_dir,
                session: self.session,
            }
            .append_referring(
                crate::brief_retry::GAVE_UP_ACTION,
                (name, ""),
                &destination_ref,
                &format!(
                    "brief record damaged ({}) — given up; the record goes to {}; the brief is \
                     preserved at undelivered.{name}.txt",
                    damaged.kind, destination_ref
                ),
            );
            if let Err(why) = appended {
                writeln!(
                    err,
                    "ae: watchdog: brief-gave-up for {name} not recorded: {why} — the record \
                     stays in place",
                )?;
                return Ok(());
            }
        }
        if let Err(why) = crate::brief_retry::move_aside(self.meta_dir, slot, &destination) {
            writeln!(err, "ae: watchdog: {why}")?;
        }
        Ok(())
    }

    /// Append one watchdog event for `agent`.
    pub(super) fn emit(
        &self,
        action: &str,
        agent: &str,
        summary: &str,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        Journal {
            meta_dir: self.meta_dir,
            session: self.session,
        }
        .record(action, agent, summary, err)
    }

    /// Append one watchdog event for the seat at `slot`, routed by slot and
    /// session: the dead latch's own records, so a display name that another
    /// slot takes over later (a rename) never inherits this seat's death.
    pub(super) fn emit_at(
        &self,
        action: &str,
        seat: (&str, &str),
        summary: &str,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        Journal {
            meta_dir: self.meta_dir,
            session: self.session,
        }
        .record_routed(action, seat, "", summary, err)
    }

    /// Roster agents with no live pane. The carry holds each seat's one
    /// spawner-notice standing (#211), shared with the per-pane step, and the
    /// wake settle that holds the paste.
    pub(super) fn sweep_missing(
        &self,
        live: &[String],
        carry: &mut Carry,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let settling = carry.settle > 0;
        let Carry {
            missing, notices, ..
        } = carry;
        // Read only when a seat is missing, and again after an alert this
        // sweep wrote, so every later fold holds it.
        let mut journal: Option<Vec<Event>> = None;
        let mut session_id: Option<String> = None;
        for entry in &self.roster {
            let reference = entry.reference();
            if live.contains(&reference) {
                if let Some((_, state)) = missing.iter_mut().find(|(key, _)| *key == entry.slot) {
                    state.streak = 0;
                }
                continue;
            }
            // The GLYPH debounce and the ALERT are both keyed by SLOT: the alert
            // routes by slot and session, and only an older keyless alert falls
            // back to the display name.
            let state = entry_mut(missing, &entry.slot);
            state.streak = state.streak.saturating_add(1);
            // One human alert per death episode, whichever leg it showed
            // first, and the journal says whether it was raised: a restart
            // never repeats it, and a closer since then makes a new death.
            let alerted = {
                let events = journal.get_or_insert_with(|| read_events(self.meta_dir));
                let id =
                    session_id.get_or_insert_with(|| crate::meta::session_id_in(self.meta_dir));
                let key = crate::events::SessionKey::new(self.session, id);
                let seat = (entry.slot.as_str(), entry.name.as_str());
                matches!(
                    deadnotice::death_episode(events, key, seat),
                    DeathEpisode::Open(_)
                )
            };
            if !alerted {
                let seat = (reference.as_str(), entry.slot.as_str());
                self.emit_at("alert", seat, deadnotice::MISSING_ALERT, err)?;
                self.notify(&reference, "pane is MISSING");
                journal = None;
            }
            // The spawner's notice is a paste: the wake settle holds it, and
            // the standing keeps it owed for the first free cycle (#238).
            if !settling {
                let events = journal.get_or_insert_with(|| read_events(self.meta_dir));
                self.tell_missing(entry, entry_mut(notices, &entry.slot), events, err)?;
            }
        }
        Ok(())
    }

    /// The transient alert shown beside every watchdog event
    /// (`display-message -d 10000`).
    fn notify(&self, agent: &str, text: &str) {
        let Some(session_id) = transport::observe_session_id(self.server, self.session) else {
            return;
        };
        let message = tmux::format_literal(&format!("[ae watchdog] {agent} {text}"));
        let _ = transport::display_message(self.server, &session_id, &message);
    }
}
