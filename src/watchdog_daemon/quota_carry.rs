//! The quota carry: the keys, samples and asks the daemon keeps between cycles, the reconcile pass and the readout line.

#[cfg(doc)]
use super::Cycle;
use super::{
    ACTOR, AtomicBool, Event, HOLD_MAX_CYCLES, Knobs, PaneState, Path, QuotaLevel, RosterEntry,
    launch_id_for, quiet_hash,
};

/// Labels are not identity: the canonical source and vendor dimensions are.
///
/// A window belongs to the CLIENT SCOPE, not to the conversation that happened
/// to observe it, so the rollout is deliberately absent: several rollouts under
/// one config home report one fact and must produce one advisory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct QuotaKey {
    pub(super) source: std::path::PathBuf,
    pub(super) bucket: String,
    pub(super) qualifier: Option<String>,
    pub(super) window_minutes: Option<u32>,
}

#[derive(Debug, Clone)]
pub(super) struct QuotaSample<'a> {
    key: QuotaKey,
    /// The scope this observation was read from. Its LABELS are all this
    /// daemon takes from it; the numbers and the policy live on the reading.
    pub(super) group: &'a crate::quota::Group,
    /// The observation and the policy it was judged under, as one value.
    pub(super) reading: crate::quota::Reading,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct QuotaTracked {
    key: QuotaKey,
    /// The classified observation: the level, the row it was decided from, the
    /// policy it was judged under, and the provenance of both.
    ///
    /// ONE value, because a level and the numbers behind it must not be able to
    /// drift apart. Every consumer — the next classification, the notice it
    /// books, the throttle line a seat reads — takes all of them from here, and
    /// `Classified::adopt` is the only thing that moves any of them.
    pub(super) classified: crate::quota::Classified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QuotaRecipient {
    pub(super) slot: String,
    pub(super) agent: String,
    pub(super) harness_session: Option<String>,
    pub(super) config_home: crate::meta::RecordedConfigHome,
    pub(super) config_home_base: crate::meta::RecordedConfigHomeBase,
    pub(super) binary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingAdvisory {
    key: QuotaKey,
    pub(super) observed_at: i64,
    pub(super) level: QuotaLevel,
    pub(super) recipient: QuotaRecipient,
    pub(super) advisory: crate::quota::Advisory,
    attempts: u8,
}

/// One booked CHECKPOINT ASK, waiting for its seat.
///
/// Keyed by scope key AND seat slot, because the fan-out is per seat: two seats
/// on one scope hold two of these, and each retries on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingAsk {
    pub(super) key: QuotaKey,
    /// The stamp of the HELD observation the level was decided from.
    pub(super) observed_at: i64,
    pub(super) recipient: QuotaRecipient,
    /// Built by [`crate::quota::Observation::transition`] from the classified
    /// reading that decided the level, so the text cannot quote another
    /// sample's numbers.
    pub(super) advisory: crate::quota::Advisory,
    attempts: u8,
}

/// A roster seat PROVEN to sit on one client scope, with a live pane to speak
/// into.
///
/// The identity is [`crate::quota::recorded_identity`]'s — the one owner of the
/// tool-kind-plus-canonical-config-home join, which `throttle_quota_line`
/// already matches a seat by. Nothing here re-derives a source path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QuotaAskCandidate {
    pub(super) recipient: QuotaRecipient,
    identity: crate::quota::RecordedIdentity,
    /// The model the seat's own frame last proved, from [`live_model`];
    /// `None` when nothing current proves one.
    pub(super) live: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum QuotaAction {
    Deliver(Box<PendingAdvisory>),
    /// Deliver one advisory-only checkpoint ask to one seat on the scope.
    Ask(Box<PendingAsk>),
    Dropped {
        recipient: String,
        summary: String,
    },
    /// A booked checkpoint ask cancelled before it was delivered, journaled as
    /// the ask's own drop rather than the advisory's.
    AskDropped {
        recipient: String,
        summary: String,
    },
    /// A booked ask withdrawn because its seat now runs another model family,
    /// and the summary that names it. Journaled with NO target: see
    /// [`crate::quota::action::CHECKPOINT_CANCELLED`].
    Cancelled(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QuotaDelivery {
    Delivered,
    Retryable,
    Uncertain,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct QuotaCarry {
    pub(super) sweeps_until_observe: u64,
    pub(super) tracked: Vec<QuotaTracked>,
    pub(super) pending: Vec<PendingAdvisory>,
    /// Booked checkpoint asks, one per scope key and seat slot. Separate from
    /// `pending` on purpose: the advisory recipients are the lead pair, the ask
    /// recipients are every seat on the scope, and neither set may widen the
    /// other.
    pub(super) asks: Vec<PendingAsk>,
    /// The seats already asked about a window in its CURRENT low episode,
    /// booked or delivered. A real entry into the band clears the window's
    /// marks, and so does every cancellation before delivery: a booking is
    /// not a delivery.
    pub(super) settled: Vec<(QuotaKey, QuotaRecipient)>,
    pub(super) last_observation: Option<crate::quota::Observation>,
    /// When this daemon's first pass ran.
    pub(super) first_pass_at: Option<i64>,
    /// The resume whose follow-up pass has run.
    pub(super) start_pass: Option<i64>,
    /// Only successfully appended samples, bounded to one current pass (256 keys).
    pub(super) history_seen: Vec<crate::quota::history::Sample>,
    pub(super) history_note_day: Option<i64>,
}

/// Whether this classification ENTERS the checkpoint band.
///
/// `before` is `None` at a window's first sight, which counts as entering: a
/// daemon that starts with a scope already Low would otherwise stay silent
/// until the next transition, which is exactly when the seat can no longer
/// speak. `Low -> Critical` and `Critical -> Low` are both already inside the
/// band and ask nothing more; the band is left, and can be entered again, only
/// when [`crate::quota::classify`]'s own hysteresis says so.
const fn entered_low(before: Option<QuotaLevel>, after: QuotaLevel) -> bool {
    matches!(after, QuotaLevel::Low | QuotaLevel::Critical)
        && matches!(before, None | Some(QuotaLevel::Headroom))
}

pub(super) fn quota_samples_at(
    observation: &crate::quota::Observation,
    now: i64,
) -> Vec<QuotaSample<'_>> {
    let mut samples: Vec<QuotaSample<'_>> = Vec::new();
    for group in &observation.groups {
        let Some(source) = group.source.clone() else {
            continue;
        };
        for row in &group.rows {
            if !matches!(
                row.status,
                crate::quota::Status::Fresh | crate::quota::Status::Stale
            ) {
                continue;
            }
            if !matches!(
                crate::quota::freshness(row.observed_at, row.resets_at, now),
                crate::quota::Status::Fresh | crate::quota::Status::Stale
            ) {
                continue;
            }
            let Some(reading) = crate::quota::Reading::of(group.policy.clone(), row.clone()) else {
                continue;
            };
            if now.saturating_sub(reading.observed_at()) > 60 * 60 {
                continue;
            }
            let key = QuotaKey {
                source: source.clone(),
                bucket: row.bucket.clone(),
                qualifier: row.qualifier.clone(),
                window_minutes: row.window_minutes,
            };
            let sample = QuotaSample {
                key: key.clone(),
                group,
                reading,
            };
            if let Some(existing) = samples.iter_mut().find(|sample| sample.key == key) {
                // One scope can be read through several rollouts in one scan,
                // and they report ONE account. So the same rules apply here as
                // across cycles: the newest row wins, every account fact wins
                // on its own stamp, and an accountless rollout takes nothing
                // away from a sibling that reported a cap.
                if existing.reading.adopt(&sample.reading) == crate::quota::Adopted::Observation {
                    existing.group = sample.group;
                }
            } else {
                samples.push(sample);
            }
        }
    }
    samples.sort_by(|left, right| left.key.cmp(&right.key));
    samples
}

pub(super) fn quota_samples(observation: &crate::quota::Observation) -> Vec<QuotaSample<'_>> {
    quota_samples_at(observation, observation.now)
}

pub(super) fn history_clock(at: Option<i64>, now: i64) -> Option<i64> {
    at.filter(|at| {
        *at > 0 && *at < now && now.saturating_sub(*at) <= crate::quota::history::ALIGN_SECS
    })
}

pub(super) fn history_same(
    a: &crate::quota::history::Sample,
    b: &crate::quota::history::Sample,
) -> bool {
    a.scope == b.scope
        && a.bucket == b.bucket
        && a.qualifier == b.qualifier
        && a.window_minutes == b.window_minutes
        && a.resets_at == b.resets_at
        && a.observed_at == b.observed_at
        && a.policy == b.policy
}

/// Whether the seat at `slot` is one of the lead pair: the main seat, and
/// worker.0 in a lead-pair layout.
pub(crate) fn in_lead_pair(slot: &str, lead_pair: bool) -> bool {
    slot == "main" || (lead_pair && slot == "worker.0")
}

pub(super) fn quota_recipients(roster: &[RosterEntry], lead_pair: bool) -> Vec<QuotaRecipient> {
    roster
        .iter()
        .filter(|entry| in_lead_pair(&entry.slot, lead_pair))
        .map(|entry| QuotaRecipient {
            slot: entry.slot.clone(),
            agent: entry.name.clone(),
            harness_session: entry.harness_session.clone(),
            config_home: entry.config_home.clone(),
            config_home_base: entry.config_home_base.clone(),
            binary: entry.binary.clone(),
        })
        .collect()
}

/// Every roster seat the checkpoint ask may reach: one that sits on a quota
/// scope ae can read, and has a live pane to speak into.
///
/// This is deliberately NOT [`quota_recipients`]. The advisory goes to the lead
/// pair, who choose profiles; the ask goes to whoever is about to lose its
/// voice, fixed seats and spawned seats alike.
///
/// Three reasons a seat is skipped, all fail-closed:
///
/// * [`crate::quota::recorded_identity`] answers `None` — the tool has no quota
///   parser (muse, opencode, gemini), or the seat's recorded config home is
///   incomplete, or a Codex seat has no recorded conversation. An unproven
///   identity is never matched to a scope, exactly as for the throttle line.
/// * No pane carries the seat's slot. A seat ae cannot see is a seat ae cannot
///   prove is listening, and the delivery would fail anyway.
/// * The pane sits at a shell. That covers the DEAD pane (a shell with the tool
///   gone) and the pane whose tool has merely been quit; neither can read.
///
/// RESIDUAL, named rather than fixed: on the tools whose TUI ae does not model,
/// the guarded send cannot see a human's half-typed draft, so an ask may land
/// mid-input there. The modelled tools defer on that draft through the existing
/// readiness check. Closing it needs a frame model per tool, which is a
/// different slice.
pub(super) fn quota_ask_candidates(
    roster: &[RosterEntry],
    panes: &[crate::tmux::WatchPane],
) -> Vec<QuotaAskCandidate> {
    roster
        .iter()
        .filter(|entry| {
            panes.iter().any(|pane| {
                pane.slot.as_deref() == Some(entry.slot.as_str())
                    && !crate::watchdog::command_is_shell(&pane.current_command)
            })
        })
        .filter_map(|entry| {
            Some(QuotaAskCandidate {
                recipient: QuotaRecipient {
                    slot: entry.slot.clone(),
                    agent: entry.name.clone(),
                    harness_session: entry.harness_session.clone(),
                    config_home: entry.config_home.clone(),
                    config_home_base: entry.config_home_base.clone(),
                    binary: entry.binary.clone(),
                },
                identity: crate::quota::recorded_identity(entry)?,
                live: None,
            })
        })
        .collect()
}

/// The model this seat's own frame last proved, as [`Cycle::resolve_identity`]
/// holds it for the picker: launch-bound, composer-gated and bounded by
/// [`HOLD_MAX_CYCLES`]. Never the profile's pin.
///
/// The quota pass runs before the pane loop, so this is the PREVIOUS cycle's
/// hold. A carry that belongs to another seat generation, a hold from another
/// launch, or a seat with no launch id at all answers `None`, which asks.
pub(super) fn live_model(
    recipient: &QuotaRecipient,
    panes: &[crate::tmux::WatchPane],
    carried: &[(String, PaneState)],
    launch_ids: &[(String, String)],
) -> Option<String> {
    let launch = launch_id_for(launch_ids, &recipient.slot)?;
    let generation = quiet_hash(&format!("{}\n{}", recipient.slot, recipient.agent));
    panes
        .iter()
        .filter(|pane| pane.slot.as_deref() == Some(recipient.slot.as_str()))
        .find_map(|pane| {
            let (_, state) = carried.iter().find(|(id, _)| *id == pane.pane_id)?;
            let hold = state.held_identity.as_ref()?;
            (state.identity == Some(generation)
                && hold.launch == launch
                && hold.age < HOLD_MAX_CYCLES)
                .then(|| hold.identity.model.clone())
                .flatten()
        })
}

/// The RECEIPT a delivered checkpoint ask leaves in the journal: the `ref` of
/// its `quota-checkpoint` event, which the send helper writes only when the
/// paste was delivered or left unconfirmed.
///
/// An IDENTITY, compared by exact equality and never decoded: FNV-1a 64 over a
/// version tag and the window, its reset and the seat's conversation, each
/// field length-prefixed so no two tuples share bytes. `None` — no receipt, so
/// a recovered first sight asks again — when the vendor stated no reset or the
/// seat has no recorded conversation.
pub(super) fn ask_receipt(
    key: &QuotaKey,
    resets_at: Option<i64>,
    recipient: &QuotaRecipient,
) -> Option<String> {
    let resets = resets_at?.to_le_bytes();
    let conversation = crate::meta::prior_parts(recipient.harness_session.as_deref()?)?.id;
    let window = key.window_minutes.map(u32::to_le_bytes);
    let mut bytes = b"ae-quota-ask-v1".to_vec();
    for field in [
        Some(key.source.as_os_str().as_encoded_bytes()),
        Some(key.bucket.as_bytes()),
        key.qualifier.as_deref().map(str::as_bytes),
        window.as_ref().map(<[u8; 4]>::as_slice),
        Some(resets.as_slice()),
        Some(conversation.as_bytes()),
    ] {
        match field {
            None => bytes.push(0),
            Some(value) => {
                bytes.push(1);
                bytes.extend(u64::try_from(value.len()).unwrap_or(u64::MAX).to_le_bytes());
                bytes.extend_from_slice(value);
            }
        }
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    Some(format!("quota-ask-{hash:016x}"))
}

/// Every checkpoint-ask receipt the journal holds: the `ref` of a
/// `quota-checkpoint` event this daemon's actor wrote. A `delivery-abandoned`
/// record carries the same ref and is NOT one — nothing reached the seat.
pub(super) fn delivered_receipts(events: &[Event]) -> Vec<&str> {
    events
        .iter()
        .filter(|event| event.action == crate::quota::action::CHECKPOINT && event.actor == ACTOR)
        .filter_map(|event| event.reference.as_deref())
        .collect()
}

/// The reset epoch of the seat's newest wake the journal holds: a
/// `limit-wake` row this daemon's actor wrote to the seat, whose `ref` parses
/// as an epoch. With `landed`, only one [`crate::events::landed`] reads as a
/// confirmed paste; without, an unconfirmed one counts too, because it may
/// have reached the seat. The send helper writes neither for a refusal.
pub(super) fn limit_wake_in(
    events: &[Event],
    (session, slot, agent): (&str, &str, &str),
    landed: bool,
) -> Option<i64> {
    events
        .iter()
        .rev()
        .filter(|event| {
            event.action == crate::watchdog::LIMIT_WAKE_ACTION
                && event.actor == ACTOR
                && crate::watchdog::event_is_addressed_to(event, session, slot, agent)
                && (!landed || crate::events::landed(event))
        })
        .find_map(|event| event.reference.as_deref()?.parse::<i64>().ok())
}

/// What one quota pass reads beside its own carry.
pub(super) struct QuotaInputs<'a> {
    pub(super) panes: &'a [crate::tmux::WatchPane],
    /// The pane carry, for each seat's held live model.
    pub(super) held: &'a [(String, PaneState)],
    /// This cycle's journal, for the checkpoint asks' receipts.
    pub(super) events: &'a [Event],
}

/// Whether this daemon's `OpenCode` Go usage refresh is in flight.
pub(super) static GO_USAGE_FLYING: AtomicBool = AtomicBool::new(false);

fn quota_sweep_count(knobs: &Knobs) -> Option<u64> {
    (knobs.quota_every_secs > 0).then(|| {
        knobs
            .quota_every_secs
            .div_ceil(knobs.interval_secs.max(1))
            .max(1)
    })
}

pub(super) fn quota_observation_due(carry: &mut QuotaCarry, knobs: &Knobs) -> bool {
    let Some(sweeps) = quota_sweep_count(knobs) else {
        return false;
    };
    if carry.sweeps_until_observe > 0 {
        carry.sweeps_until_observe -= 1;
        return false;
    }
    carry.sweeps_until_observe = sweeps.saturating_sub(1);
    true
}

/// How long after a resume settles its one follow-up pass waits: a seat's
/// statusline feeds its first reading 2-3 s into its start (measured).
const START_PASS_DELAY_SECS: i64 = 5;

/// Whether this cycle owes the resume settled at `settled` its one follow-up
/// quota pass: the first pass may have run before the seat's first reading,
/// and the regular cadence may not come back inside the start window.
/// Separate from [`quota_observation_due`], whose counter this never advances.
pub(super) fn start_pass_due(
    first_pass_at: Option<i64>,
    taken: Option<i64>,
    settled: Option<i64>,
    now: i64,
) -> bool {
    settled.is_some_and(|settled| {
        taken != Some(settled)
            && first_pass_at.is_some_and(|at| at < now)
            && settled.saturating_add(START_PASS_DELAY_SECS) <= now
            && now <= settled.saturating_add(crate::autoreseat::START_WINDOW_SECS)
    })
}

impl QuotaCarry {
    /// Drop everything a quota observation ever taught: tracked windows,
    /// pending notices and the last observation — but NOT the shared due
    /// counter, which paces the advisory passes. Called every cycle while
    /// unaware, so the property holds: while unaware, nothing holds a quota
    /// observation for any consumer (throttle line included) to inject.
    /// Losing hysteresis across an OFF/ON cycle is correct and
    /// direction-symmetric with reset declarations clearing a critical.
    pub(super) fn clear_held(&mut self) {
        self.tracked.clear();
        self.pending.clear();
        // An ask is held quota knowledge too: while unaware nothing may stay
        // booked to fire the moment awareness returns. So is a mark: a later
        // first sight must consult the journal, not a memory from before.
        self.asks.clear();
        self.settled.clear();
        self.last_observation = None;
        self.history_seen.clear();
    }

    fn cancel_where(
        &mut self,
        predicate: impl Fn(&PendingAdvisory) -> bool,
        reason: &str,
        meta_dir: &Path,
        now: i64,
        actions: &mut Vec<QuotaAction>,
    ) {
        let mut retained = Vec::new();
        for pending in self.pending.drain(..) {
            if predicate(&pending) {
                actions.push(QuotaAction::Dropped {
                    recipient: pending.recipient.agent,
                    summary: format!("{reason}: {}", pending.advisory.render(meta_dir, now)),
                });
            } else {
                retained.push(pending);
            }
        }
        self.pending = retained;
    }

    /// The ask's own cancellation, kept apart from the advisory's so that
    /// neither recipient set can reach the other's bookings. A cancelled
    /// booking leaves no mark: it never reached the seat.
    fn cancel_asks_where(
        &mut self,
        predicate: impl Fn(&PendingAsk) -> bool,
        reason: &str,
        meta_dir: &Path,
        now: i64,
        actions: &mut Vec<QuotaAction>,
    ) {
        let mut retained = Vec::new();
        for ask in self.asks.drain(..) {
            if predicate(&ask) {
                self.settled
                    .retain(|(key, recipient)| !(*key == ask.key && *recipient == ask.recipient));
                actions.push(QuotaAction::AskDropped {
                    recipient: ask.recipient.agent,
                    summary: format!("{reason}: {}", ask.advisory.checkpoint_ask(meta_dir, now)),
                });
            } else {
                retained.push(ask);
            }
        }
        self.asks = retained;
    }

    /// The account-wide weekly reading of `sample`'s scope — the same tool and
    /// the same canonical source — as the cell its notice names beside its
    /// reset. Each reading is the one HELD, offered this pass's observation:
    /// a weekly window sorted after `sample` is still seen, and a row the raw
    /// clock refused never replaces a held one.
    fn held_weekly(
        &self,
        samples: &[QuotaSample<'_>],
        sample: &QuotaSample<'_>,
        trigger: &crate::quota::Classified,
    ) -> Option<String> {
        let held: Vec<crate::quota::Classified> = samples
            .iter()
            .filter(|other| {
                other.key != sample.key
                    && other.group.tool == sample.group.tool
                    && other.key.source == sample.key.source
            })
            .map(|other| {
                let Some(tracked) = self.tracked.iter().find(|held| held.key == other.key) else {
                    return crate::quota::Classified::first(other.reading.clone());
                };
                let mut classified = tracked.classified.clone();
                let _ = classified.adopt(&other.reading);
                classified
            })
            .collect();
        crate::quota::weekly_context(sample.group.tool, trigger, &held)
    }

    /// Book ONE checkpoint ask for every seat on a scope whose window sits in
    /// the band, unless that seat is already settled for this episode or its
    /// live model is proven to be of another family than the window's.
    ///
    /// Run every pass, not only at an entry, so a seat that switches INTO the
    /// window's family mid-episode is asked too. A seat that already holds an
    /// undelivered ask for the key has it REPLACED rather than doubled: a band
    /// that cleared and was entered again while the first ask was still
    /// deferred must still produce exactly one ask, carrying the newer facts.
    fn book_asks(
        &mut self,
        observation: &crate::quota::Observation,
        samples: &[QuotaSample<'_>],
        candidates: &[QuotaAskCandidate],
    ) {
        for sample in samples {
            let Some(held) = self
                .tracked
                .iter()
                .find(|tracked| tracked.key == sample.key)
                .map(|tracked| tracked.classified.clone())
                .filter(|held| matches!(held.level(), QuotaLevel::Low | QuotaLevel::Critical))
            else {
                continue;
            };
            let key = &sample.key;
            let unasked: Vec<&QuotaAskCandidate> = candidates
                .iter()
                .filter(|candidate| {
                    candidate.identity.tool == sample.group.tool
                        && candidate.identity.source == key.source
                        && !self
                            .settled
                            .contains(&(key.clone(), candidate.recipient.clone()))
                        && !crate::quota::scoped_elsewhere(
                            candidate.identity.tool,
                            key.qualifier.as_deref(),
                            candidate.live.as_deref(),
                        )
                })
                .collect();
            for candidate in unasked {
                self.asks
                    .retain(|ask| !(ask.key == *key && ask.recipient == candidate.recipient));
                self.asks.push(PendingAsk {
                    key: key.clone(),
                    observed_at: held.observed_at(),
                    recipient: candidate.recipient.clone(),
                    advisory: observation
                        .transition(sample.group, &held)
                        .with_weekly(self.held_weekly(samples, sample, &held)),
                    attempts: 0,
                });
                self.settled
                    .push((key.clone(), candidate.recipient.clone()));
            }
        }
    }

    /// Withdraw every booked ask whose seat's live model is now PROVEN to be
    /// of another family than its window, clearing the mark so a switch back
    /// is asked again. Named, and journaled with no target.
    ///
    /// The live model is the previous cycle's hold, so a switch inside one
    /// pass cannot be seen here: this reaches a DEFERRED ask, one pass later.
    fn withdraw_switched(
        &mut self,
        candidates: &[QuotaAskCandidate],
        meta_dir: &Path,
        now: i64,
        actions: &mut Vec<QuotaAction>,
    ) {
        let mut retained = Vec::new();
        for ask in self.asks.drain(..) {
            let switched = candidates.iter().find(|candidate| {
                candidate.recipient == ask.recipient
                    && crate::quota::scoped_elsewhere(
                        candidate.identity.tool,
                        ask.key.qualifier.as_deref(),
                        candidate.live.as_deref(),
                    )
            });
            let Some(live) = switched.and_then(|candidate| candidate.live.as_deref()) else {
                retained.push(ask);
                continue;
            };
            self.settled
                .retain(|(key, recipient)| !(*key == ask.key && *recipient == ask.recipient));
            actions.push(QuotaAction::Cancelled(format!(
                "{}: checkpoint seat now runs {live}, not {}: {}",
                ask.recipient.agent,
                ask.key.qualifier.as_deref().unwrap_or_default(),
                ask.advisory.checkpoint_ask(meta_dir, now)
            )));
        }
        self.asks = retained;
    }

    /// Mark every seat on a window's scope whose receipt the journal already
    /// holds, at a FIRST sight inside the band: a restart or a recovery from
    /// silence inside an episode that seat was already asked about.
    fn recover_settled(
        &mut self,
        key: &QuotaKey,
        advisory: &crate::quota::Advisory,
        candidates: &[QuotaAskCandidate],
        receipts: &[&str],
    ) {
        for candidate in candidates.iter().filter(|candidate| {
            ask_receipt(key, advisory.resets_at(), &candidate.recipient)
                .is_some_and(|receipt| receipts.contains(&receipt.as_str()))
        }) {
            self.settled
                .push((key.clone(), candidate.recipient.clone()));
        }
    }

    /// Drop the booked asks this pass may no longer deliver: one whose seat is
    /// gone from the candidate set, and one whose facts have aged out.
    ///
    /// Deliberately NOT keyed on the level: an ask booked at an entry survives
    /// every later move inside the band, because cancelling it there would mean
    /// the seat is never asked at all.
    fn cancel_stale_asks(
        &mut self,
        candidates: &[QuotaAskCandidate],
        now: i64,
        meta_dir: &Path,
        actions: &mut Vec<QuotaAction>,
    ) {
        self.cancel_asks_where(
            |ask| {
                !candidates
                    .iter()
                    .any(|candidate| candidate.recipient == ask.recipient)
            },
            "checkpoint recipient is gone",
            meta_dir,
            now,
            actions,
        );
        self.cancel_asks_where(
            |ask| !ask.advisory.current_at(now),
            "checkpoint ask expired",
            meta_dir,
            now,
            actions,
        );
    }

    /// One quota pass. `receipts` are the journal's [`delivered_receipts`],
    /// consulted ONLY at a window's first sight inside the band.
    ///
    /// The order is load-bearing: stale asks are cancelled first, so a seat
    /// that is eligible again is booked afresh by the same pass; the sample
    /// loop then decides entries and recoveries; a switched seat's deferred
    /// ask is withdrawn; and only then are the unasked seats booked.
    pub(super) fn reconcile_pass(
        &mut self,
        observation: &crate::quota::Observation,
        recipients: &[QuotaRecipient],
        candidates: &[QuotaAskCandidate],
        receipts: &[&str],
        meta_dir: &Path,
    ) -> Vec<QuotaAction> {
        let mut actions = Vec::new();
        self.cancel_stale_asks(candidates, observation.now, meta_dir, &mut actions);
        self.cancel_where(
            |pending| !recipients.contains(&pending.recipient),
            "recipient identity changed",
            meta_dir,
            observation.now,
            &mut actions,
        );
        self.cancel_where(
            |pending| !pending.advisory.current_at(observation.now),
            "quota advisory expired",
            meta_dir,
            observation.now,
            &mut actions,
        );

        let samples = quota_samples(observation);
        self.drop_silent_keys(&samples, meta_dir, observation.now, &mut actions);

        for sample in &samples {
            let previous = self
                .tracked
                .iter()
                .position(|tracked| tracked.key == sample.key);
            let Some(index) = previous else {
                // The first sight of a window is classified and SILENT: there
                // is no transition to report yet.
                let classified = crate::quota::Classified::first(sample.reading.clone());
                // The ASK is not silent here, and that is the whole point: a
                // daemon that starts with the scope already Low has seats which
                // may never get another transition to be warned by. The
                // advisory's own first-sight silence above is untouched. A
                // first sight may also be a RECOVERY — a restart, or a window
                // back from silence — inside an episode a seat was already
                // asked about; its receipt in the journal says so.
                if entered_low(None, classified.level()) {
                    let advisory = observation.transition(sample.group, &classified);
                    self.recover_settled(&sample.key, &advisory, candidates, receipts);
                }
                self.tracked.push(QuotaTracked {
                    key: sample.key.clone(),
                    classified,
                });
                continue;
            };
            // Two clocks, one classification, and the classified reading keeps
            // both: the RAW observation is replaced only by a strictly newer
            // stamp, whatever the policy says, while the declaration and the
            // account facts merge on their own provenance. Either way the level
            // that comes back was decided from the observation now HELD.
            let before = self.tracked[index].classified.level();
            let adopted = self.tracked[index].classified.adopt(&sample.reading);
            if adopted == crate::quota::Adopted::Nothing {
                continue;
            }
            let after = self.tracked[index].classified.level();
            if before == after {
                continue;
            }
            self.cancel_where(
                |pending| pending.key == sample.key,
                if adopted == crate::quota::Adopted::Observation {
                    "superseded by newer quota transition"
                } else {
                    "superseded by a quota policy change"
                },
                meta_dir,
                observation.now,
                &mut actions,
            );
            // The notice describes the HELD observation, which is also what
            // decided the level, so its numbers and the provenance it is booked
            // under cannot be another sample's.
            let held = self.tracked[index].classified.clone();
            let advisory = observation
                .transition(sample.group, &held)
                .with_weekly(self.held_weekly(&samples, sample, &held));
            // ENTERING the band opens a new episode: every seat on the scope is
            // asked once more. Moving around inside it does not, and neither
            // does leaving it; only a re-entry after `classify`'s hysteresis
            // let the window clear asks again. A declaration change re-derives
            // the held row and reaches here the same way, so a withdrawn reset
            // re-arms the ask exactly as it re-arms the advisory. A real entry
            // never consults the journal.
            if entered_low(Some(before), after) {
                self.settled.retain(|(key, _)| *key != sample.key);
            }
            for recipient in recipients {
                self.pending.push(PendingAdvisory {
                    key: sample.key.clone(),
                    observed_at: held.observed_at(),
                    level: after,
                    recipient: recipient.clone(),
                    advisory: advisory.clone(),
                    attempts: 0,
                });
            }
        }
        self.withdraw_switched(candidates, meta_dir, observation.now, &mut actions);
        self.book_asks(observation, &samples, candidates);

        actions.extend(
            self.pending
                .iter()
                .cloned()
                .map(Box::new)
                .map(QuotaAction::Deliver),
        );
        actions.extend(
            self.asks
                .iter()
                .cloned()
                .map(Box::new)
                .map(QuotaAction::Ask),
        );
        self.last_observation = Some(observation.clone());
        actions
    }

    /// Forget every key this observation no longer reports, and cancel the
    /// notices that were still waiting on them.
    fn drop_silent_keys(
        &mut self,
        samples: &[QuotaSample<'_>],
        meta_dir: &Path,
        now: i64,
        actions: &mut Vec<QuotaAction>,
    ) {
        let live_keys: Vec<QuotaKey> = samples.iter().map(|sample| sample.key.clone()).collect();
        let silent_keys: Vec<QuotaKey> = self
            .tracked
            .iter()
            .filter(|tracked| !live_keys.contains(&tracked.key))
            .map(|tracked| tracked.key.clone())
            .collect();
        self.tracked
            .retain(|tracked| live_keys.contains(&tracked.key));
        // Its marks go with it: the window's return is a first sight, and a
        // first sight answers to the journal alone.
        self.settled.retain(|(key, _)| live_keys.contains(key));
        self.cancel_where(
            |pending| silent_keys.contains(&pending.key),
            "quota observation went silent",
            meta_dir,
            now,
            actions,
        );
        self.cancel_asks_where(
            |ask| silent_keys.contains(&ask.key),
            "quota observation went silent",
            meta_dir,
            now,
            actions,
        );
    }

    pub(super) fn record_delivery(
        &mut self,
        delivered: &PendingAdvisory,
        result: QuotaDelivery,
        meta_dir: &Path,
        now: i64,
    ) -> Option<QuotaAction> {
        let index = self.pending.iter().position(|pending| {
            pending.key == delivered.key
                && pending.observed_at == delivered.observed_at
                && pending.level == delivered.level
                && pending.recipient == delivered.recipient
        })?;
        match result {
            QuotaDelivery::Delivered => {
                self.pending.remove(index);
                return None;
            }
            QuotaDelivery::Uncertain => {
                let pending = self.pending.remove(index);
                return Some(QuotaAction::Dropped {
                    recipient: pending.recipient.agent,
                    summary: format!(
                        "delivery result uncertain; not retrying: {}",
                        pending.advisory.render(meta_dir, now)
                    ),
                });
            }
            QuotaDelivery::Retryable => {}
        }
        self.pending[index].attempts = self.pending[index].attempts.saturating_add(1);
        if self.pending[index].attempts < 2 {
            return None;
        }
        let pending = self.pending.remove(index);
        Some(QuotaAction::Dropped {
            recipient: pending.recipient.agent,
            summary: format!(
                "delivery failed twice: {}",
                pending.advisory.render(meta_dir, now)
            ),
        })
    }

    /// The ask's delivery bookkeeping, mirroring [`Self::record_delivery`]: a
    /// delivered ask is FORGOTTEN, so the deferral that got it through cannot
    /// be followed by a duplicate; a proven pre-submit refusal is retried once;
    /// anything ambiguous is dropped rather than risk pasting it twice.
    pub(super) fn record_ask_delivery(
        &mut self,
        delivered: &PendingAsk,
        result: QuotaDelivery,
        meta_dir: &Path,
        now: i64,
    ) -> Option<QuotaAction> {
        let index = self.asks.iter().position(|ask| {
            ask.key == delivered.key
                && ask.observed_at == delivered.observed_at
                && ask.recipient == delivered.recipient
        })?;
        match result {
            QuotaDelivery::Delivered => {
                self.asks.remove(index);
                return None;
            }
            QuotaDelivery::Uncertain => {
                let ask = self.asks.remove(index);
                return Some(QuotaAction::Dropped {
                    recipient: ask.recipient.agent,
                    summary: format!(
                        "delivery result uncertain; not retrying: {}",
                        ask.advisory.checkpoint_ask(meta_dir, now)
                    ),
                });
            }
            QuotaDelivery::Retryable => {}
        }
        self.asks[index].attempts = self.asks[index].attempts.saturating_add(1);
        if self.asks[index].attempts < 2 {
            return None;
        }
        let ask = self.asks.remove(index);
        Some(QuotaAction::Dropped {
            recipient: ask.recipient.agent,
            summary: format!(
                "delivery failed twice: {}",
                ask.advisory.checkpoint_ask(meta_dir, now)
            ),
        })
    }
}

/// One candidate for the throttle line: the classified observation itself, so
/// ranking and rendering cannot read different ones.
#[derive(Debug, Clone)]
struct QuotaReadout<'a> {
    key: QuotaKey,
    group: &'a crate::quota::Group,
    classified: crate::quota::Classified,
}

pub(super) fn throttle_quota_line(
    observation: &crate::quota::Observation,
    tracked: &[QuotaTracked],
    entry: &RosterEntry,
    meta_dir: &Path,
    now: i64,
) -> Option<String> {
    let identity = crate::quota::recorded_identity(entry)?;
    // Whatever provenance decides the LEVEL supplies the numbers rendered with
    // it. For a scope this daemon already tracks that is the HELD reading, so a
    // sample the raw clock refused is never what a throttled seat is shown.
    let mut matches: Vec<QuotaReadout<'_>> = quota_samples_at(observation, now)
        .into_iter()
        .filter(|sample| sample.group.tool == identity.tool && sample.key.source == identity.source)
        .map(|sample| {
            let classified = match tracked.iter().find(|held| held.key == sample.key) {
                Some(held) => held.classified.clone(),
                None => crate::quota::Classified::first(sample.reading),
            };
            QuotaReadout {
                key: sample.key,
                group: sample.group,
                classified,
            }
        })
        .collect();
    matches.sort_by(|left, right| {
        right
            .classified
            .level()
            .cmp(&left.classified.level())
            .then_with(|| {
                right
                    .classified
                    .judged()
                    .total_cmp(&left.classified.judged())
            })
            .then_with(|| left.key.cmp(&right.key))
    });
    let worst = matches.first()?;
    Some(observation.state_line_at(worst.group, &worst.classified, meta_dir, now))
}
