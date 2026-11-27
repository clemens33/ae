//! The cycle's observation, quota and identity steps: the methods of Cycle that read the world and book what it said.

use super::acts::StaysRead;
use super::{
    Acting, Cycle, Effect, Event, GO_USAGE_FLYING, HOLD_MAX_CYCLES, HarnessObservation,
    IdentityHold, Ordering, OverviewReading, PaneState, Path, PathBuf, QuotaAction, QuotaCarry,
    QuotaInputs, ResolveIdentity, RosterEntry, SeatIdentity, Status, SweepObservation, UNIX_EPOCH,
    Verdict, Write, ask_receipt, declaration_key, delivered_receipts, history_clock, history_same,
    is_sweep_target, last_done_event_at, last_working_declaration_at, latest_relevant_event,
    launch_id_for, live_model, quiet_hash, quota_ask_candidates, quota_delivery,
    quota_observation_due, quota_recipients, quota_samples_at, start_pass_due,
    system_time_from_epoch, throttle_quota_line,
};

impl Cycle<'_> {
    /// Apply one pane's booked effects, collecting the cycle-level quota
    /// request: the recovery pass belongs to the sweep as a whole.
    pub(super) fn apply_booked(
        &self,
        effects: &[Effect],
        on: &Acting<'_>,
        state: &mut PaneState,
        quota_refresh: &mut bool,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        for effect in effects {
            if matches!(effect, Effect::QuotaRefresh) {
                *quota_refresh = true;
                continue;
            }
            self.apply(effect, on, state, err)?;
        }
        Ok(())
    }

    /// The ONE recovery pass a release requests: the same refresh the due
    /// path calls, invoked directly — never through the due counter — so the
    /// cadence keeps its own schedule. `quota = off` runs none.
    pub(super) fn refresh_after_limit_release(
        &self,
        requested: bool,
        carry: &mut QuotaCarry,
        inputs: &QuotaInputs<'_>,
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        if requested && self.knobs.quota_aware {
            self.refresh_quota(carry, inputs, now, err)?;
        }
        Ok(())
    }

    /// The state root, the global config and the home the session-relative
    /// quota inputs are read from.
    fn quota_paths(&self) -> (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>) {
        let root = crate::doors::state_root(crate::shape::current()).or_else(|| {
            self.meta_dir
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        });
        let global = root
            .as_deref()
            .map(|root| crate::doors::config_file(crate::shape::current(), root));
        (root, global, crate::doors::home())
    }

    /// The same session-relative quota inputs as the generated `quota` helper.
    fn quota_observation(
        &self,
        now: i64,
    ) -> Result<crate::quota::Observation, crate::config::ConfigError> {
        let (root, global, home) = self.quota_paths();
        let roots = root.as_deref().map(crate::inventory::Roots::under);
        crate::quota::observe(&crate::quota::Inputs {
            home: home.as_deref(),
            global: global.as_deref(),
            local: self.local_config.as_deref(),
            sessions: roots.as_ref().map(crate::inventory::Roots::sessions),
            now,
        })
    }

    pub(super) fn apply_quota_actions(
        &self,
        carry: &mut QuotaCarry,
        actions: Vec<QuotaAction>,
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        for action in actions {
            match action {
                QuotaAction::Dropped { recipient, summary } => {
                    self.emit(
                        crate::quota::action::ADVISORY_DROPPED,
                        &recipient,
                        &summary,
                        err,
                    )?;
                }
                QuotaAction::AskDropped { recipient, summary } => {
                    self.emit(
                        crate::quota::action::CHECKPOINT_DROPPED,
                        &recipient,
                        &summary,
                        err,
                    )?;
                }
                QuotaAction::Cancelled(summary) => {
                    // NO target: news to nobody, so no fold reads it.
                    self.emit(
                        crate::quota::action::CHECKPOINT_CANCELLED,
                        "",
                        &summary,
                        err,
                    )?;
                }
                QuotaAction::Deliver(pending) => {
                    let text = pending.advisory.render(self.meta_dir, now);
                    let delivery = self.deliver(
                        &pending.recipient.agent,
                        &text,
                        crate::quota::action::ADVISORY,
                        &text,
                        None,
                        Some(&pending.recipient.slot),
                    );
                    if let Some(QuotaAction::Dropped { recipient, summary }) = carry
                        .record_delivery(&pending, quota_delivery(&delivery), self.meta_dir, now)
                    {
                        self.emit(
                            crate::quota::action::ADVISORY_DROPPED,
                            &recipient,
                            &summary,
                            err,
                        )?;
                    }
                }
                QuotaAction::Ask(ask) => {
                    // The SAME sender as the advisory — one delivery owner, so
                    // the seat reads one marker, written by `provenance`. The
                    // receipt rides the event the helper writes on delivery.
                    let text = ask.advisory.checkpoint_ask(self.meta_dir, now);
                    let receipt = ask_receipt(&ask.key, ask.advisory.resets_at(), &ask.recipient);
                    let delivery = self.deliver(
                        &ask.recipient.agent,
                        &text,
                        crate::quota::action::CHECKPOINT,
                        &text,
                        receipt.as_deref(),
                        Some(&ask.recipient.slot),
                    );
                    if let Some(QuotaAction::Dropped { recipient, summary }) = carry
                        .record_ask_delivery(&ask, quota_delivery(&delivery), self.meta_dir, now)
                    {
                        self.emit(
                            crate::quota::action::CHECKPOINT_DROPPED,
                            &recipient,
                            &summary,
                            err,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    fn history_seat(
        &self,
        entry: &RosterEntry,
        inputs: &QuotaInputs<'_>,
        now: i64,
    ) -> (crate::quota::history::Seat, Option<i64>) {
        let launch = launch_id_for(&self.launch_ids, &entry.slot);
        let held = inputs
            .panes
            .iter()
            .find(|pane| {
                pane.slot.as_deref() == Some(entry.slot.as_str())
                    && pane.agent.as_deref() == Some(entry.name.as_str())
            })
            .and_then(|pane| {
                inputs
                    .held
                    .iter()
                    .find(|(id, _)| id == &pane.pane_id)
                    .map(|(_, state)| state)
            });
        let capture = held
            .and_then(|state| state.history_capture.as_ref())
            .filter(|(_, _, id)| id.as_deref() == launch);
        let clock = history_clock(capture.map(|(at, _, _)| *at), now);
        let identity = held
            .and_then(|state| state.held_identity.as_ref())
            .filter(|hold| Some(hold.launch.as_str()) == launch && clock.is_some());
        let declared = latest_relevant_event(inputs.events, self.session, &entry.slot, &entry.name)
            .filter(crate::watchdog::declaration_current)
            .and_then(|relevant| relevant.event.declared_state())
            .map(str::to_owned);
        (
            crate::quota::history::Seat {
                slot: entry.slot.clone(),
                agent: entry.name.clone(),
                launch: launch.map(str::to_owned),
                observed: clock
                    .and(capture)
                    .map_or("unknown", |(_, state, _)| state.as_str())
                    .to_owned(),
                declared,
                model: identity.and_then(|hold| hold.identity.model.clone()),
                effort: identity.and_then(|hold| hold.identity.effort.clone()),
            },
            clock,
        )
    }

    fn write_quota_history(
        &self,
        carry: &mut QuotaCarry,
        observation: &crate::quota::Observation,
        inputs: &QuotaInputs<'_>,
        now: i64,
        err: &mut impl Write,
    ) {
        use crate::quota::history::{self, Row, Workload};
        if !self.knobs.quota_aware || self.knobs.quota_every_secs == 0 {
            return;
        }
        let incarnation = crate::meta::session_id_in(self.meta_dir);
        let mut current: Vec<_> = quota_samples_at(observation, now)
            .iter()
            .filter_map(|s| {
                crate::quota::history_sample(s.group, &s.reading, self.session, &incarnation, now)
            })
            .collect();
        let mut rows: Vec<_> = current
            .iter()
            .filter(|s| !carry.history_seen.iter().any(|held| history_same(held, s)))
            .cloned()
            .map(Row::Sample)
            .collect();
        let scopes: std::collections::BTreeSet<_> = observation
            .groups
            .iter()
            .filter_map(crate::quota::history_scope)
            .collect();
        let identities: Vec<_> = self
            .roster
            .iter()
            .map(|entry| (entry, crate::quota::recorded_identity(entry)))
            .collect();
        let unscoped = identities
            .iter()
            .filter(|(_, identity)| identity.is_none())
            .count();
        for scope in scopes {
            let mut seats = Vec::new();
            let mut observed_at: Option<i64> = None;
            for (entry, _) in identities.iter().filter(|(_, identity)| {
                identity.as_ref().is_some_and(|id| {
                    id.tool.as_str() == scope.tool
                        && id.source.to_str() == Some(scope.source.as_str())
                })
            }) {
                let (seat, clock) = self.history_seat(entry, inputs, now);
                if let Some(at) = clock {
                    observed_at = Some(observed_at.map_or(at, |old| old.min(at)));
                }
                seats.push(seat);
            }
            rows.push(Row::Workload(Workload {
                producer: self.session.to_owned(),
                incarnation: incarnation.clone(),
                collected_at: now,
                scope,
                observed_at,
                seats,
                unscoped,
            }));
        }
        match history::append(self.meta_dir, &rows, now) {
            Ok(()) => {
                current.truncate(256);
                carry.history_seen = current;
            }
            Err(why) => {
                let day = now.div_euclid(86_400);
                if carry.history_note_day != Some(day) {
                    carry.history_note_day = Some(day);
                    let _ = writeln!(err, "ae: watchdog: quota history skipped: {why}");
                }
            }
        }
    }

    fn refresh_quota(
        &self,
        carry: &mut QuotaCarry,
        inputs: &QuotaInputs<'_>,
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        match self.quota_observation(now) {
            Ok(observation) => {
                let mut recipients = quota_recipients(&self.roster, self.lead_pair);
                // TWO sets, never one: the advisory's lead pair above, and
                // every seat proven to sit on a readable scope below.
                let mut candidates = quota_ask_candidates(&self.roster, inputs.panes);
                // A seat this cycle's auto step moves hears neither: a turn
                // on its account could only fail.
                let urgent = self.urgent_slots(&observation, inputs.panes, now);
                recipients.retain(|seat| !urgent.contains(&seat.slot));
                candidates.retain(|seat| !urgent.contains(&seat.recipient.slot));
                for candidate in &mut candidates {
                    candidate.live = live_model(
                        &candidate.recipient,
                        inputs.panes,
                        inputs.held,
                        &self.launch_ids,
                    );
                }
                let actions = carry.reconcile_pass(
                    &observation,
                    &recipients,
                    &candidates,
                    &delivered_receipts(inputs.events),
                    self.meta_dir,
                );
                let summary = Self::observed_summary(&observation, actions.len());
                // LAST: the trace attests a COMPLETED refresh — observation,
                // booking, and delivery all done — so awaiting the line
                // proves everything the test subsequently reads has already
                // happened. Tracing before delivery would attest booking
                // while the receipt was still absent.
                self.apply_quota_actions(carry, actions, now, err)?;
                self.tell_reset_expiries(&observation, inputs);
                self.write_quota_history(carry, &observation, inputs, now, err);
                Self::trace_quota(&summary);
                Ok(())
            }
            Err(why) => {
                writeln!(
                    err,
                    "ae: watchdog: quota observation failed — skipped: {why}"
                )?;
                Ok(())
            }
        }
    }

    /// Read the fleet through [`crate::fleet_read::world_read`] with a positioned
    /// journal reader, enrich it through the same card collector as
    /// `ae brief --all`, then touch the watchdog-owned checkpoint without
    /// advancing the last-delivered hash.
    ///
    /// The same read decides the desk-stale tick: each journal it holds is
    /// read once with its records' source positions. `ack` is the
    /// orchestrator main's agent, whose `done` acknowledges an overview.
    pub(super) fn overview(
        &self,
        now: i64,
        ack: Option<&str>,
        err: &mut impl Write,
    ) -> Option<(OverviewReading, DeskSight)> {
        let Some(root) = crate::doors::state_root(crate::shape::current()) else {
            let _ = writeln!(
                err,
                "ae: watchdog: no state root for fleet overview — skipped"
            );
            return None;
        };
        let (read, positions) = positioned_world(&root);
        let world = &read.world;
        let sessions = crate::inventory::Roots::under(&root);
        let cards: Vec<crate::brief::Card> = world
            .sessions
            .iter()
            .filter(|entry| entry.status == Status::Running && entry.name != self.session)
            .map(|entry| {
                crate::brief::card_for(
                    entry,
                    &sessions.sessions().join(&entry.name),
                    None,
                    false,
                    world.now,
                    None,
                )
            })
            .collect();
        let rendered = crate::overview::render(&cards, self.session);
        let hash = crate::overview::semantic_hash(&cards, self.session);
        let checkpoint = match crate::watchdog_state::overview_heartbeat(self.meta_dir, now) {
            Ok(checkpoint) => checkpoint,
            Err(why) => {
                let _ = writeln!(
                    err,
                    "ae: watchdog: fleet overview checkpoint failed — skipped: {why}"
                );
                return None;
            }
        };
        let prior = crate::watchdog_state::desk_tick(self.meta_dir);
        let last_at = prior.last_at;
        let booking = crate::desktick::read(&read, &positions, (self.meta_dir, ack), &prior, now)
            .map(|stale| crate::desktick::Booking {
                stale,
                prior,
                overview_last: checkpoint.last_delivered_at,
            });
        let sight = DeskSight { booking, last_at };
        let reading = OverviewReading {
            rendered,
            hash,
            checkpoint,
        };
        Some((reading, sight))
    }

    /// The orchestrator-main-only observation, including the durable hash and
    /// spacing gate recovered with this cycle's overview.
    pub(super) fn sweep_observation(
        &self,
        slot: &str,
        agent: &str,
        events: &[Event],
        (overview, desk): (Option<&OverviewReading>, Option<&DeskSight>),
        now: i64,
    ) -> Option<SweepObservation> {
        is_sweep_target(self.meta_agent, slot).then(|| {
            SweepObservation::new(
                system_time_from_epoch(now).unwrap_or(UNIX_EPOCH),
                last_done_event_at(events, self.session, agent),
            )
            .with_working_since(last_working_declaration_at(events, self.session, agent))
            .with_overview(
                overview.is_some_and(OverviewReading::changed),
                overview.and_then(OverviewReading::persisted_outstanding_since),
                overview.and_then(OverviewReading::persisted_last_delivery),
            )
            .with_desk(
                desk.is_some_and(|sight| sight.booking.is_some()),
                desk.and_then(|sight| sight.last_at)
                    .and_then(system_time_from_epoch),
            )
        })
    }

    /// The worst exact-match quota row for this slot — `None` unless the pane
    /// is ACTUALLY throttled, because that is the only reading it explains.
    /// Unaware sessions never hold an observation (the cycle skips the quota
    /// read entirely), so no quota line can render there either.
    pub(super) fn throttle_quota(
        &self,
        quota: &QuotaCarry,
        slot: &str,
        now: i64,
        throttled: bool,
    ) -> Option<String> {
        if !throttled {
            return None;
        }
        self.roster
            .iter()
            .find(|entry| entry.slot == slot)
            .and_then(|entry| {
                quota.last_observation.as_ref().and_then(|observation| {
                    throttle_quota_line(observation, &quota.tracked, entry, self.meta_dir, now)
                })
            })
    }

    pub(super) fn harness_observation(
        &self,
        capture: &str,
        tool: crate::tool::ToolKind,
        events: &[Event],
        slot: &str,
        agent: &str,
    ) -> HarnessObservation {
        let declaration = crate::session::latest_declaration_in(events, self.session, slot, agent)
            .map(|event| quiet_hash(&declaration_key(event)));
        HarnessObservation {
            frame: crate::harness_state::classify(capture, tool),
            human_draft: crate::harness_state::has_human_draft(capture, tool),
            durable_stale: crate::session::alert_reason_in(events, self.session, slot, agent)
                == Some(crate::attention::Reason::Stale),
            declaration,
        }
    }

    /// Record this pane's observed model, when the tool's live model is
    /// readable. Best-effort: a refused write (a stale guard, a failed read) is
    /// the next cycle's problem, never this cycle's.
    ///
    /// `pin` is [`Self::seat_pin`]'s answer for THIS cycle, read fresh there
    /// and handed to both consumers so one config read serves both; an
    /// operator edit while the session runs is visible on the next cycle.
    pub(super) fn note_model(
        &self,
        capture: &str,
        tool: crate::tool::ToolKind,
        slot: &str,
        pins: &crate::model_drift::Pins,
    ) {
        if !tool.adapter().model.observes() {
            return;
        }
        let Some(entry) = self.roster.iter().find(|entry| entry.slot == slot) else {
            return;
        };
        // A seat with no profile has no pin to disagree with, and the durable
        // row is about that disagreement: it writes nothing here, as it never
        // has. `pin` cannot stand in — it is `None` for an unpinned profile too.
        if entry.profile.is_none() {
            return;
        }
        let Some(launch_id) = launch_id_for(&self.launch_ids, slot) else {
            return;
        };
        let _ = crate::model_drift::observe(
            self.meta_dir,
            (slot, &entry.name),
            tool,
            capture,
            launch_id,
            pins,
        );
    }

    /// This seat's profile model pin for this cycle, or `None` when the tool's
    /// live model is not one ae observes at all.
    ///
    /// Read ONCE per pane per cycle: the durable drift row and the picker's
    /// drift mark are two questions about the same pin.
    pub(super) fn seat_pin(
        &self,
        slot: &str,
        tool: crate::tool::ToolKind,
    ) -> crate::model_drift::Pins {
        let profile = self
            .roster
            .iter()
            .find(|entry| entry.slot == slot)
            .and_then(|entry| entry.profile.as_deref());
        match profile {
            Some(profile) if tool.adapter().model.observes() => self.profile_pins(profile, tool),
            _ => crate::model_drift::Pins::default(),
        }
    }

    /// What this cycle will SHOW for one seat's model, holding the last proven
    /// answer while the frame cannot be read.
    ///
    /// The read is the GATED one: a picker cell is a claim about what the seat
    /// is running now, so a frame whose composer is absent proves nothing here
    /// even when its grammar would parse. The hold then covers the ordinary
    /// gaps — a turn in flight, a draft in the box, one failed capture —
    /// bounded by [`HOLD_MAX_CYCLES`] and by the seat's own launch id. A DEAD
    /// seat clears it outright: whatever it was running, it is not now.
    pub(super) fn resolve_identity(
        &self,
        carried: &mut PaneState,
        seen: &ResolveIdentity<'_>,
    ) -> SeatIdentity {
        if seen.verdict == Verdict::Dead {
            carried.held_identity = None;
            return SeatIdentity::default();
        }
        let launch = launch_id_for(&self.launch_ids, seen.slot).map(str::to_owned);
        let observed = crate::harness_state::observed_identity(seen.capture, seen.tool);
        if let Some(model) = observed.model {
            let identity = SeatIdentity {
                drift: matches!(
                    crate::model_drift::decide(
                        &crate::harness_state::HarnessIdentity {
                            model: Some(model.clone()),
                            effort: None,
                        },
                        seen.pin,
                        seen.tool.adapter().pin_match,
                    ),
                    // A profile that pins NO model has nothing to drift from;
                    // that is a report, not a disagreement.
                    crate::model_drift::Decision::Record { pin: Some(_), .. }
                ),
                model: Some(model),
                effort: observed.effort,
            };
            carried.held_identity = launch.map(|launch| IdentityHold {
                launch,
                age: 0,
                identity: identity.clone(),
            });
            return identity;
        }
        match (&mut carried.held_identity, launch) {
            (Some(hold), Some(launch)) if hold.launch == launch && hold.age < HOLD_MAX_CYCLES => {
                hold.age = hold.age.saturating_add(1);
                hold.identity.clone()
            }
            (held, _) => {
                *held = None;
                SeatIdentity::default()
            }
        }
    }

    /// The profile's model flag value, from the same config files `_run`
    /// reads. An unreadable config is `None`, never a guess.
    fn profile_pins(&self, profile: &str, tool: crate::tool::ToolKind) -> crate::model_drift::Pins {
        let root = crate::doors::state_root(crate::shape::current());
        let global = root
            .as_deref()
            .map(|root| crate::doors::config_file(crate::shape::current(), root));
        crate::launch_cmd::profile_pins(
            global.as_deref(),
            self.local_config.as_deref(),
            profile,
            tool,
        )
    }

    /// ORCHESTRATOR MODEL GUARD: the words of the one owner notice when the
    /// orchestrator's main seat has a move due (its usage limit is reached, or
    /// its account reads at the threshold) and the rules produce none, for any
    /// reason: the policy is `off`, nothing is configured, the running model is
    /// not proven, only other models are listed, a same-model candidate is
    /// refused, no candidate keeps the conversation, or a request is in flight.
    /// `None` when no move is due or one is permitted; the human's own observed
    /// choice that holds it is a reason like the rest, and its `observed-choice`
    /// hold is journaled by the act that tells it. Decided
    /// BEFORE any attempt opens, with the leg's own stance and chooser, so no
    /// retry budget is spent and a candidate that becomes usable still moves the
    /// seat. The policy `on` keeps its movement and gets the notice all the same.
    #[allow(
        clippy::too_many_lines,
        reason = "one ordered verdict, read top to bottom"
    )]
    pub(super) fn orchestrator_stays(
        &self,
        quota: &QuotaCarry,
        events: &[Event],
        (slot, agent): (&str, &str),
        (capture, due): (
            &str,
            (
                &crate::autoreseat::Room,
                Option<crate::autoreseat::LimitReset>,
                bool,
            ),
        ),
        now: i64,
    ) -> Option<StaysRead> {
        use crate::autoreseat::{Ineligible, Room, Stance};
        let (room, reset, limited) = due;
        if !self.meta_agent
            || slot != "main"
            || self.auto.switch != crate::autoreseat::Switch::All
            || !(limited || matches!(room, Room::Crossed(_)))
        {
            return None;
        }
        let profile = self.seat_profile(slot);
        let seat = crate::autoreseat::Seat {
            session: self.session,
            slot,
            agent,
            profile: &profile,
            orchestrator: true,
        };
        let declared = match crate::autoreseat::eligible(&self.auto, &seat) {
            Ok(list) => list,
            Err(Ineligible::Unmapped) => &[],
            Err(_) => return None,
        };
        let row = self.roster.iter().find(|row| row.slot == slot)?;
        // The human's observed choice holds the move: the notice says so, and the
        // `observed-choice` hold is journaled by the act that tells it.
        let observed = crate::run::observed_standing(self.meta_dir, slot);
        let bytes = crate::meta::read_bytes(self.meta_dir).unwrap_or_default();
        let auto = self.auto.seat_view(self.lead_pair, slot);
        let stance = crate::autoreseat::leg::orchestrator_stance(&auto, &bytes, row, Some(capture));
        // A running model that nothing proves is part of why the seat stays: the owner reads the
        // proof's own words, and the pin the profile held when the observer first saw the pane
        // leave it. The observer drops the launch record then, so that pin is the only
        // durable word of what the seat was set to run; it is named as a pin, never as proof.
        let unproven = match &stance {
            Some(Stance::Keep(crate::autoreseat::Origin::Unproven(why))) => Some(why.clone()),
            _ => None,
        };
        let pinned = match crate::meta::observed_pair(&bytes, slot) {
            crate::meta::Rows::Valid(pair) => pair.model_pin,
            _ => None,
        };
        let off = stance == Some(Stance::Off);
        let auto = auto.staged(stance);
        let list = crate::autoreseat::leg::listed(self.meta_dir, &bytes, row, declared);
        // The profile `ae reseat` can read, the first listed: a row naming a
        // deleted profile is no manual move.
        let by_hand = list
            .iter()
            .map(|(name, _)| name)
            .find(|name| crate::reseat::resolved(self.meta_dir, name).is_some())
            .cloned();
        // The stage guard refuses every automatic move of a seat that records no
        // launch or conversation, whatever the policy and the candidates say.
        let guarded = crate::autoreseat::leg::records_launch(&bytes, slot)
            && !crate::lifecycle::meta_value(&bytes, &format!("harness_session.{slot}")).is_empty();
        // `None` when a move is permitted: only the tail is then kept, for a hold the
        // leg alone finds.
        let reason: Option<String> = if let Some(hold) = &observed {
            Some(with_unproven(
                crate::autoreseat::observed_summary(hold),
                unproven.as_deref(),
                pinned.as_deref(),
            ))
        } else if list.is_empty() {
            Some(format!(
                "nothing is configured to take it ({profile} has no candidate)"
            ))
        } else if off {
            Some("auto_reseat_orchestrator = off".to_owned())
        } else if !guarded {
            Some("it records no launch or conversation to guard".to_owned())
        } else {
            let left = crate::autoreseat::left_profiles(events, self.session, slot, agent);
            let left_on_headroom =
                crate::autoreseat::left_on_headroom(events, self.session, slot, agent);
            let latched =
                crate::autoreseat::latched_identities(&self.roster, events, self.session, slot);
            let pair = crate::autoreseat::leg::pair_of(
                self.meta_dir,
                &bytes,
                &self.roster,
                self.lead_pair,
                slot,
            );
            let route = crate::autoreseat::Route::Move {
                seat_profile: &profile,
            };
            let bar = auto.headroom_at.filter(|_| !limited);
            let choose = |among: &[(String, crate::autoreseat::Provenance)]| {
                let judged = crate::autoreseat::leg::candidates(
                    among,
                    |to| crate::reseat::resolved(self.meta_dir, to),
                    (&left, &left_on_headroom),
                    quota.last_observation.as_ref(),
                    &latched,
                    now,
                );
                crate::autoreseat::choose_seat(&auto, route, &judged, now, bar, pair.as_ref())
            };
            match crate::autoreseat::leg::orchestrator_list(
                self.meta_dir,
                &bytes,
                (self.session, slot, agent),
                list.clone(),
            ) {
                Ok(kept) => {
                    let choice = choose(&kept);
                    // A move the chooser allows has no reason of its own.
                    choice
                        .pick
                        .is_none()
                        .then(|| crate::autoreseat::leg::passed_over_words(&choice))
                }
                // No candidate keeps the conversation: the model refusal of the
                // whole list, where there is one, says more than the travel one.
                Err((class, why)) => {
                    let choice = choose(&list);
                    Some(
                        if class == crate::autoreseat::CauseClass::NoCandidate
                            && crate::autoreseat::leg::guard_refused(&choice)
                        {
                            crate::autoreseat::leg::passed_over_words(&choice)
                        } else {
                            class.details(&why).trim_start_matches("held: ").to_owned()
                        },
                    )
                }
            }
        };
        let when = |at: i64| format!(", resets {}", crate::time::Timestamp::from_epoch(at));
        // The scope that triggered the episode: a limit the pane draws outranks
        // a threshold also read.
        let scope = match room {
            Room::Crossed(window) if !limited => format!(
                "{} reads {:.0}%{}",
                window.label,
                window.judged,
                window.resets_at.map(when).unwrap_or_default()
            ),
            _ => {
                let notice = crate::watchdog::limit_notice(
                    capture,
                    row.binary.as_deref().unwrap_or_default(),
                );
                format!(
                    "its usage limit is reached{}{}",
                    notice.map(|line| format!(" ({line})")).unwrap_or_default(),
                    reset.map(|reset| when(reset.at)).unwrap_or_default()
                )
            }
        };
        let action = match by_hand {
            Some(name) => format!(
                "Move it by hand: ae reseat {} {agent} --using {name}",
                self.session
            ),
            None => format!("No candidate is configured: add an [auto_reseat] row for {profile}."),
        };
        let tail = format!(". {scope}. {action}");
        Some(StaysRead {
            stays: reason.map(|reason| format!("{reason}{tail}")),
            tail,
        })
    }

    /// The line the first-sight notice of `agent`'s limit gains when auto
    /// reseat may move it, ranked over this cycle's held quota observation.
    pub(super) fn auto_deadline(
        &self,
        quota: &QuotaCarry,
        events: &[Event],
        (slot, agent): (&str, &str),
        now: i64,
    ) -> Option<String> {
        let profile = self.seat_profile(slot);
        let seat = crate::autoreseat::Seat {
            session: self.session,
            slot,
            agent,
            profile: &profile,
            orchestrator: self.meta_agent,
        };
        let declared = match crate::autoreseat::eligible(&self.auto, &seat) {
            Ok(list) => list,
            Err(crate::autoreseat::Ineligible::Unmapped) => &[],
            Err(_) => return None,
        };
        let row = self.roster.iter().find(|row| row.slot == slot)?;
        // A move the human's observed choice holds is never forecast (#269).
        if crate::run::observed_standing(self.meta_dir, slot).is_some() {
            return None;
        }
        let bytes = crate::meta::read_bytes(self.meta_dir).unwrap_or_default();
        let list = crate::autoreseat::leg::listed(self.meta_dir, &bytes, row, declared);
        if list.is_empty() {
            return None;
        }
        let left = crate::autoreseat::left_profiles(events, self.session, slot, agent);
        let left_on_headroom =
            crate::autoreseat::left_on_headroom(events, self.session, slot, agent);
        let latched =
            crate::autoreseat::latched_identities(&self.roster, events, self.session, slot);
        // `quota = off` holds no observation here: `QuotaCarry::clear_held`.
        let judged = crate::autoreseat::leg::candidates(
            &list,
            |to| crate::reseat::resolved(self.meta_dir, to),
            (&left, &left_on_headroom),
            quota.last_observation.as_ref(),
            &latched,
            now,
        );
        // The pair guard forecasts what the legs will choose (#305).
        let pair = crate::autoreseat::leg::pair_of(
            self.meta_dir,
            &bytes,
            &self.roster,
            self.lead_pair,
            slot,
        );
        // The seat's own view carries the lead floor, so the forecast never
        // promises a move the leg would refuse (#309).
        let auto = self.auto.seat_view(self.lead_pair, slot);
        // The orchestrator's model guard: the forecast never promises a move
        // the leg would refuse for the model (`auto_reseat_orchestrator`).
        let stance = (self.meta_agent && slot == "main")
            .then(|| {
                let capture =
                    crate::autoreseat::leg::live_capture(self.meta_dir, self.session, agent);
                crate::autoreseat::leg::orchestrator_stance(&auto, &bytes, row, capture.as_deref())
            })
            .flatten();
        let auto = auto.staged(stance);
        let route = crate::autoreseat::Route::Move {
            seat_profile: &profile,
        };
        let choice =
            crate::autoreseat::choose_seat(&auto, route, &judged, now, None, pair.as_ref());
        Some(crate::autoreseat::leg::deadline(
            agent,
            &choice,
            self.auto.grace_secs,
        ))
    }

    /// Whether the seat at `slot`, eligible but for an `[auto_reseat]` row that
    /// names nothing, has a candidate in its own family. Asked only while the
    /// seat could act — `active` (on its limit, or near or past its threshold),
    /// or inside an episode — so a quiet or filtered seat costs no config read.
    pub(super) fn derives(
        &self,
        events: &[Event],
        (slot, agent): (&str, &str),
        active: bool,
    ) -> bool {
        use crate::autoreseat::{Ineligible, Seat, eligible, episode, headroom_episode};
        let profile = self.seat_profile(slot);
        let seat = Seat {
            session: self.session,
            slot,
            agent,
            profile: &profile,
            orchestrator: self.meta_agent,
        };
        if eligible(&self.auto, &seat) != Err(Ineligible::Unmapped) {
            return false;
        }
        let live = active
            || headroom_episode(events, self.session, slot, agent).is_some()
            || episode(events, self.session, slot, agent).is_some_and(|found| found.open.is_some());
        live && self
            .roster
            .iter()
            .find(|row| row.slot == slot)
            .is_some_and(|row| {
                let bytes = crate::meta::read_bytes(self.meta_dir).unwrap_or_default();
                crate::autoreseat::leg::derivable(self.meta_dir, &bytes, row)
            })
    }

    /// One fresh read of the seat at `slot` against this session's resume.
    pub(super) fn start_read(
        &self,
        quota: Option<&crate::quota::Observation>,
        (slot, pane): (&str, &str),
        pin: Option<&str>,
        now: i64,
    ) -> crate::autoreseat::StartRead {
        let Some(row) = self.roster.iter().find(|row| row.slot == slot) else {
            return crate::autoreseat::StartRead::default();
        };
        let profile = self.seat_profile(slot);
        let at = crate::autoreseat::leg::StartSeat {
            dir: self.meta_dir,
            row,
            seat: crate::autoreseat::Seat {
                session: self.session,
                slot,
                agent: &row.name,
                profile: &profile,
                orchestrator: self.meta_agent,
            },
            settled: self.settled,
            pane: Some(pane),
            pin,
        };
        crate::autoreseat::leg::start_read(&self.auto, &at, quota, now)
    }

    /// Whether auto reseat may move any seat of this session.
    fn any_seat_reachable(&self) -> bool {
        self.roster.iter().any(|row| {
            let profile = self.seat_profile(&row.slot);
            let seat = crate::autoreseat::Seat {
                session: self.session,
                slot: &row.slot,
                agent: &row.name,
                profile: &profile,
                orchestrator: self.meta_agent,
            };
            let bytes = crate::meta::read_bytes(self.meta_dir).unwrap_or_default();
            crate::autoreseat::leg::reachable(&self.auto, &seat, (self.meta_dir, &bytes), row)
        })
    }

    /// The slots a resume's exhausted reading moves now, by this observation.
    fn urgent_slots(
        &self,
        observation: &crate::quota::Observation,
        panes: &[crate::tmux::WatchPane],
        now: i64,
    ) -> Vec<String> {
        if self.settled.is_none() {
            return Vec::new();
        }
        panes
            .iter()
            .filter_map(|pane| {
                pane.slot
                    .as_deref()
                    .map(|slot| (slot, pane.pane_id.as_str()))
            })
            .filter(|(slot, pane)| {
                let bin = self.agent_bin(slot).unwrap_or_default();
                let tool = crate::tool::ToolKind::from_binary_name(&bin);
                let pin = self.seat_pin(slot, tool).model;
                self.start_read(Some(observation), (slot, pane), pin.as_deref(), now)
                    .urgent(now)
            })
            .map(|(slot, _)| slot.to_owned())
            .collect()
    }

    /// One quota-cadence pass: the vendor-quota observation and advisory
    /// booking, and only when aware. `quota = off` WINS over
    /// `quota_every_secs`: no quota read is even attempted unaware, while the
    /// cadence's due counter still advances.
    pub(super) fn run_quota_cadence(
        &self,
        carry: &mut QuotaCarry,
        inputs: &QuotaInputs<'_>,
        now: i64,
        err: &mut impl Write,
    ) -> crate::error::Result<()> {
        let regular = quota_observation_due(carry, &self.knobs);
        // Zero disables the periodic pass, and this one with it; a session with
        // no seat auto reseat may move reads exactly as it did.
        let follow_up = self.knobs.quota_every_secs > 0
            && start_pass_due(carry.first_pass_at, carry.start_pass, self.settled, now)
            && self.any_seat_reachable();
        if follow_up {
            carry.start_pass = self.settled;
        }
        if regular || follow_up {
            carry.first_pass_at.get_or_insert(now);
            self.spawn_go_usage(now);
            if self.knobs.quota_aware {
                self.refresh_quota(carry, inputs, now, err)?;
            } else {
                Self::trace_quota("skipped");
            }
        }
        Ok(())
    }

    /// Refresh the shared `OpenCode` Go usage cache on a detached thread, at
    /// most one in flight. The thread's answer reaches every reader through
    /// that cache alone; this pass observes whatever the cache held before it.
    fn spawn_go_usage(&self, now: i64) {
        let (Some(root), global, home) = self.quota_paths() else {
            return;
        };
        let local = self.local_config.as_deref();
        let roots = crate::inventory::Roots::under(&root);
        let key = crate::quota::go_key(&crate::quota::Inputs {
            home: home.as_deref(),
            global: global.as_deref(),
            local,
            sessions: Some(roots.sessions()),
            now,
        });
        let due = crate::go_usage::watchdog_due(
            crate::quota::configured_quota_on(global.as_deref(), local),
            self.knobs.quota_aware,
            key.is_some(),
            GO_USAGE_FLYING.load(Ordering::SeqCst),
        );
        let Some(key) = key.filter(|_| due) else {
            return;
        };
        let flying = crate::go_usage::Flying::arm(&GO_USAGE_FLYING);
        // A refused spawn drops the closure, and with it the guard.
        let _ = std::thread::Builder::new()
            .name("ae-go-usage".to_owned())
            .spawn(move || {
                let _flying = flying;
                let _ = crate::go_usage::refresh(&root, &key, now);
            });
    }

    /// The attestation one completed refresh reports to the test trace: the
    /// maximum window percentage it observed (`none` when no row stated one)
    /// and how many quota actions that observation booked — advisories,
    /// checkpoint asks and cancellations alike. The count is a progress signal
    /// for a test awaiting a completed pass, never a per-kind assertion.
    fn observed_summary(observation: &crate::quota::Observation, booked: usize) -> String {
        let mut max: Option<f64> = None;
        for group in &observation.groups {
            for row in &group.rows {
                if let Some(pct) = row
                    .used_percent
                    .as_deref()
                    .and_then(|text| text.parse::<f64>().ok())
                {
                    max = Some(max.map_or(pct, |held: f64| held.max(pct)));
                }
            }
        }
        format!(
            "observed max={} booked={booked}",
            max.map_or_else(|| "none".to_owned(), |value| format!("{value}"))
        )
    }
}

/// An observed choice's hold, with the proof that the running model is not proven
/// and the pin the profile held when the observer first saw the pane leave it: the
/// owner reads the model the pane draws and the model the seat was set to run.
pub(super) fn with_unproven(said: String, unproven: Option<&str>, pinned: Option<&str>) -> String {
    let Some(why) = unproven else {
        return said;
    };
    let pin = pinned.map_or_else(String::new, |pin| {
        format!("; its profile pinned {pin} when the choice was seen")
    });
    format!("{said}; its running model is not proven ({why}){pin}")
}

/// What the cycle read of the desk-stale tick: the change booked for the
/// delivery, if one is due, and the stored claim's spacing stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DeskSight {
    pub(super) booking: Option<crate::desktick::Booking>,
    last_at: Option<i64>,
}

/// One classified fleet read whose reader records each journal's source
/// positions from the very bytes it parses.
fn positioned_world(root: &Path) -> (crate::fleet_read::WorldRead, crate::desktick::Positions) {
    let mut positions = crate::desktick::Positions::new();
    let read = crate::fleet_read::world_read(root, &mut |dir| {
        crate::session::RecordSnapshot::read_with(dir, || {
            let (read, ends) = crate::session::SessionRead::open_positioned(dir)?;
            positions.insert(dir.to_owned(), ends);
            Ok(read)
        })
    });
    (read, positions)
}
