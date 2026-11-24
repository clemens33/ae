//! The automatic reseat decisions: the seats the step reads, the acts it can choose and the return and headroom routes.

use super::{Event, Throttle, Timestamp};

/// The daemon's read of the orchestrator's due move: its notice when the rules produce
/// no move, and the tail (scope and manual move) every notice of the seat ends with.
#[derive(Debug, Clone)]
pub(super) struct StaysRead {
    pub(super) stays: Option<String>,
    pub(super) tail: String,
}

/// One seat after this cycle's accounting, as the auto reseat step reads it.
#[derive(Debug, Clone)]
pub(super) struct AutoSeat {
    pub(super) slot: String,
    pub(super) agent: String,
    pub(super) profile: String,
    pub(super) pane: crate::autoreseat::Pane,
    /// Its usage-limit latch stands: the seat is the limit path's alone.
    pub(super) limited: bool,
    /// Its own account against the headroom threshold, from this cycle's held
    /// quota observation.
    pub(super) room: crate::autoreseat::Room,
    /// Its `[auto_reseat]` row names nothing, but its own family does.
    pub(super) derives: bool,
    /// ORCHESTRATOR MODEL GUARD: this cycle's due move of the orchestrator's main
    /// seat yields no move whatever the cause; the words of the one owner notice.
    pub(super) stays: Option<String>,
    /// The scope and the manual move of that notice, when a move is due: what a hold
    /// only the leg finds, and keeps finding, is told with ([`StaysRead`]).
    pub(super) held_tail: Option<String>,
    /// This cycle's read of it against the session's resume.
    pub(super) start: crate::autoreseat::StartRead,
    /// Its latched episode's carried binding reset: a near short one waits
    /// for the wake instead of moving ([`crate::autoreseat::reset_wait`]).
    pub(super) reset: Option<crate::autoreseat::LimitReset>,
    /// The reset epoch of its newest wake the journal proves LANDED: the seat
    /// resumed on that reset, which is no reason to move it.
    pub(super) wake_landed: Option<i64>,
    /// The reset its carry saw the tool's own resume armed for: the vendor
    /// resumes it, which plays the landed role.
    pub(super) armed: Option<i64>,
}

/// What the auto reseat step does this cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AutoAct {
    /// Hand `agent` to the trigger.
    Trigger { agent: String },
    /// Say, once per episode, that the limited seat has no candidate at all.
    Unmapped {
        slot: String,
        agent: String,
        key: Timestamp,
        profile: String,
    },
    /// Say, once per episode, that the due move of the orchestrator stays
    /// undone and why: no attempt opens, so nothing is spent.
    Stays {
        slot: String,
        agent: String,
        key: Timestamp,
        body: String,
    },
    /// Say, once per episode, that the last round was refused or failed and
    /// no attempt follows.
    GaveUp {
        slot: String,
        agent: String,
        key: Timestamp,
        profile: String,
    },
    /// Name why the due seat is not moved, once per episode and reason.
    Held {
        slot: String,
        agent: String,
        key: Timestamp,
        reason: crate::autoreseat::HoldReason,
        trigger: crate::autoreseat::Trigger,
    },
    /// Close an attempt that passed its bound with no outcome.
    Overdue {
        slot: String,
        agent: String,
        key: Timestamp,
        trigger: crate::autoreseat::Trigger,
    },
    /// Open a headroom episode: `cause` names the crossing.
    Open {
        slot: String,
        agent: String,
        cause: String,
    },
    /// End the headroom episode keyed `key`: the account reads relieved.
    Rearm {
        slot: String,
        agent: String,
        key: Timestamp,
        summary: String,
    },
    /// Open a return episode: `cause` names the origin and its room (#305).
    OpenReturn {
        slot: String,
        agent: String,
        cause: String,
    },
    /// Say, once per chain keyed by its newest move `key`, that the seat went
    /// home too often to go home again by itself.
    Flapped {
        slot: String,
        agent: String,
        key: Timestamp,
        origin: String,
    },
}

/// The auto reseat step's decisions, PURE: every latched seat the switch
/// admits is decided by [`crate::autoreseat::decide`], each hold is named once
/// per episode and reason (`named` is that memory), and at most ONE seat is
/// triggered — the oldest episode, the slot breaking a tie.
///
/// A seat whose latch does not stand is the HEADROOM path's: its episode opens
/// on a crossing, re-arms on relief, and is decided like a limit episode while
/// its account reads at or past the CURRENT threshold; within the gap below it
/// the episode stays open and moves nothing (#311). A headroom hold the
/// journal already ends on is not named again, a restart included.
///
/// A seat no limit or headroom path holds goes home by [`return_route`] when
/// `homes` names its slot (#305); a seat with no such entry decides as it
/// always did.
#[allow(clippy::too_many_lines, reason = "one ordered decision per seat")]
pub(super) fn auto_acts_with(
    settings: &crate::autoreseat::Settings,
    (seats, homes): (&[AutoSeat], &[(String, crate::autoreseat::leg::HomeRead)]),
    events: &[Event],
    (session, orchestrator, lead_pair): (&str, bool, bool),
    now: i64,
    named: &mut Vec<(String, Timestamp, crate::autoreseat::HoldReason)>,
) -> Vec<AutoAct> {
    use crate::autoreseat::{Decision, Ineligible, Room, Trigger};
    named.retain(|(slot, _, _)| seats.iter().any(|seat| seat.slot == *slot));
    let home = |seat: &AutoSeat| {
        homes
            .iter()
            .find(|(slot, _)| *slot == seat.slot)
            .map(|(_, found)| found)
    };
    let mut acts = Vec::new();
    let mut due: Option<(Timestamp, &AutoSeat)> = None;
    for seat in seats {
        // Every decision below is the seat's own: the lead pair reads its own
        // headroom threshold when the row is set.
        let view = settings.seat_view(lead_pair, &seat.slot);
        let settings = &*view;
        let asked = crate::autoreseat::Seat {
            session,
            slot: &seat.slot,
            agent: &seat.agent,
            profile: &seat.profile,
            orchestrator,
        };
        let back = crate::autoreseat::return_episode(events, session, &seat.slot, &seat.agent);
        // The origin is a return's one candidate, so a seat with no row of its
        // own may still go home: only the return path decides it then.
        let homeward_only = match crate::autoreseat::eligible(settings, &asked) {
            Err(Ineligible::Unmapped) if seat.derives || seat.stays.is_some() => false,
            Err(Ineligible::Unmapped) if seat.limited => {
                acts.extend(unmapped_act(settings, seat, events, session, now));
                continue;
            }
            Err(Ineligible::Unmapped) if home(seat).is_some() || back.is_some() => true,
            Err(_) => continue,
            Ok(_) => false,
        };
        let limit = crate::autoreseat::episode(events, session, &seat.slot, &seat.agent);
        let headroom =
            crate::autoreseat::headroom_episode(events, session, &seat.slot, &seat.agent);
        // An attempt in flight, of either kind, decides the seat until it ends.
        let flying =
            crate::autoreseat::flying_any(&[limit.as_ref(), headroom.as_ref(), back.as_ref()]);
        // A seat its own account presses is the headroom path's: it goes home
        // only with no headroom episode at all (#305).
        let pressed = headroom.is_some();
        let found = if homeward_only {
            match flying {
                Some(open) => (open.trigger == Trigger::Return).then_some(open),
                None if pressed => None,
                None => match return_route(seat, home(seat), back, (events, session)) {
                    Homeward::Act(act) => {
                        acts.push(act);
                        continue;
                    }
                    Homeward::Decide(found) => found,
                },
            }
        } else if flying.is_some() {
            flying
        } else if seat.limited {
            limit
        } else {
            match headroom_route(settings, seat, headroom, now) {
                Headroom::Act(act) => {
                    acts.push(act);
                    continue;
                }
                Headroom::Decide(None) if !pressed => {
                    match return_route(seat, home(seat), back, (events, session)) {
                        Homeward::Act(act) => {
                            acts.push(act);
                            continue;
                        }
                        Homeward::Decide(found) => found,
                    }
                }
                Headroom::Decide(found) => found,
            }
        };
        let Some(found) = found else {
            continue;
        };
        if found.gave_up() {
            if !found.holds_with(crate::autoreseat::GAVE_UP_SUMMARY) {
                let (slot, agent) = (seat.slot.clone(), seat.agent.clone());
                let (key, profile) = (found.key, seat.profile.clone());
                acts.push(AutoAct::GaveUp {
                    slot,
                    agent,
                    key,
                    profile,
                });
            }
            continue;
        }
        if waits_for_reset(seat, &found, now) {
            continue;
        }
        let (slot, agent, key, trigger) = (
            seat.slot.clone(),
            seat.agent.clone(),
            found.key,
            found.trigger,
        );
        // A headroom episode moves only while its account reads at or past the
        // CURRENT threshold; within the gap below it the episode stays open and
        // waits (#311). An attempt already open is judged whatever it reads.
        let urgent = seat.start.urgent(key.epoch());
        let near = urgent || matches!(seat.room, Room::Crossed(_));
        // A return takes the plain grace and the plain frame: a resume's
        // waivers are for an exhausted start, never for going home.
        let homeward = trigger == Trigger::Return;
        // A hold only the leg can find, and keeps finding, is told once by its
        // durable count, on the key this cycle actually selects. Its notice and the
        // key's retry are independent: the admission below still waits on the
        // cooldown alone, so telling it never blocks recovery.
        if !homeward
            && seat.stays.is_none()
            && let (Some(tail), Some((count, why))) =
                (seat.held_tail.as_deref(), found.persistent_hold())
        {
            acts.push(AutoAct::Stays {
                slot: slot.clone(),
                agent: agent.clone(),
                key,
                body: format!("the move was held {count} times: {why}{tail}"),
            });
        }
        let waits = if homeward {
            crate::autoreseat::Waits::plain(settings.grace_secs)
        } else {
            seat.start.waits(key.epoch(), settings.grace_secs)
        };
        let mut pane = seat.pane;
        if !homeward {
            pane.frame = seat.start.frame(key.epoch(), pane.frame);
        }
        match crate::autoreseat::decide_with_waits(Some(&found), waits, &pane, now) {
            Decision::Overdue => acts.push(AutoAct::Overdue {
                slot,
                agent,
                key,
                trigger,
            }),
            Decision::Hold(_) | Decision::Attempt if trigger == Trigger::Headroom && !near => {}
            // ORCHESTRATOR MODEL GUARD: a due move the policy cannot make is said
            // to the owner once, and no attempt opens: nothing is spent, so a
            // candidate that becomes usable moves the seat next cycle.
            Decision::Hold(_) | Decision::Attempt if seat.stays.is_some() && !homeward => {
                acts.extend(seat.stays.clone().map(|body| AutoAct::Stays {
                    slot,
                    agent,
                    key,
                    body,
                }));
            }
            // A return's holds are silent: a working seat is no news (M6).
            Decision::Hold(_) if homeward => {}
            Decision::Hold(reason) => {
                let said = trigger == Trigger::Headroom
                    && found
                        .holds_with(&crate::autoreseat::leg::held_summary(reason, found.cause()));
                if name_hold(named, &slot, key, reason, said) {
                    acts.push(AutoAct::Held {
                        slot,
                        agent,
                        key,
                        reason,
                        trigger,
                    });
                }
            }
            // A hold of the orchestrator's own rules delays the next admission
            // only; the verdict above is told whatever the cooldown says.
            Decision::Attempt if found.cooling_until(now).is_some() => {}
            Decision::Attempt => {
                if due.is_none_or(|(at, first)| (key, &seat.slot) < (at, &first.slot)) {
                    due = Some((key, seat));
                }
            }
            Decision::Rest | Decision::Wait { .. } | Decision::InFlight => {}
        }
    }
    acts.extend(due.map(|(_, seat)| AutoAct::Trigger {
        agent: seat.agent.clone(),
    }));
    acts
}

/// Whether the hold of `slot`'s episode `key` for `reason` is named this
/// cycle, remembering it in `named` if so. The slot's memory of any other
/// episode is dropped first, whatever is said; a hold the journal already
/// ends on (`said`) or one named before is not named again.
pub(super) fn name_hold(
    named: &mut Vec<(String, Timestamp, crate::autoreseat::HoldReason)>,
    slot: &str,
    key: Timestamp,
    reason: crate::autoreseat::HoldReason,
    said: bool,
) -> bool {
    named.retain(|(at, held, _)| at != slot || *held == key);
    let fresh = !said && !named.contains(&(slot.to_owned(), key, reason));
    if fresh {
        named.push((slot.to_owned(), key, reason));
    }
    fresh
}

/// What the auto reseat step says this cycle of a limited seat with no
/// candidate at all: named once per episode, while that limit episode is
/// still owed a move — a terminal episode, one the journal already ends on
/// that hold, or one left to its reset's wake says nothing, and neither does
/// one inside the grace or with an attempt open.
pub(super) fn unmapped_act(
    settings: &crate::autoreseat::Settings,
    seat: &AutoSeat,
    events: &[Event],
    session: &str,
    now: i64,
) -> Option<AutoAct> {
    use crate::autoreseat::{Decision, Ineligible};
    let said = crate::autoreseat::leg::ineligible_record(Ineligible::Unmapped);
    let open =
        crate::autoreseat::episode(events, session, &seat.slot, &seat.agent).filter(|open| {
            open.terminal.is_none() && !open.holds_with(&said) && !waits_for_reset(seat, open, now)
        })?;
    let decision = crate::autoreseat::decide(Some(&open), settings.grace_secs, &seat.pane, now);
    matches!(decision, Decision::Attempt | Decision::Hold(_)).then(|| AutoAct::Unmapped {
        slot: seat.slot.clone(),
        agent: seat.agent.clone(),
        key: open.key,
        profile: seat.profile.clone(),
    })
}

/// Where the return path leaves a seat no limit or headroom path holds.
#[derive(Debug, PartialEq)]
enum Homeward {
    /// Record this act: the seat is done this cycle.
    Act(AutoAct),
    /// Decide this return episode like a limit one; none leaves the seat be.
    Decide(Option<crate::autoreseat::Episode>),
}

/// The return path of one seat, given its return episode `back` (#305): a
/// chain whose origin proves room opens one where none is — or, past
/// [`crate::autoreseat::MAX_RETURNS`], holds once instead — and an open one is
/// decided only while that proof stands. Anything less is silent, except a
/// root the pair gate refuses, which is decided until its refusal is said.
fn return_route(
    seat: &AutoSeat,
    read: Option<&crate::autoreseat::leg::HomeRead>,
    back: Option<crate::autoreseat::Episode>,
    (events, session): (&[Event], &str),
) -> Homeward {
    let Some(read) = read else {
        return Homeward::Decide(None);
    };
    let home = &read.home;
    let (slot, agent) = (seat.slot.clone(), seat.agent.clone());
    // A root that carried a client the pair gate refuses goes to the trigger
    // until the episode holds its ONE refusal of that root, whatever other
    // rounds failed or were refused before (their cooldowns still apply);
    // then the seat holds on its substitute until a human moves it or the
    // chain ends (#296).
    if read.refusal.is_some() {
        return match back {
            None => Homeward::Act(AutoAct::OpenReturn {
                slot,
                agent,
                cause: crate::autoreseat::refused_return_words(&home.origin),
            }),
            Some(open)
                if crate::autoreseat::leg::root_refusal_told(
                    events,
                    (session, &seat.slot, &seat.agent),
                    open.key,
                    &home.origin,
                ) =>
            {
                Homeward::Decide(None)
            }
            Some(open) => Homeward::Decide(Some(open)),
        };
    }
    match (back, home.ready.as_ref()) {
        (None, Some(_)) if home.flapped => Homeward::Act(AutoAct::Flapped {
            slot,
            agent,
            key: home.newest,
            origin: home.origin.clone(),
        }),
        (None, Some(window)) => Homeward::Act(AutoAct::OpenReturn {
            slot,
            agent,
            cause: crate::autoreseat::return_words(&home.origin, window),
        }),
        (Some(open), Some(_)) => Homeward::Decide(Some(open)),
        (Some(open), None) if open.gave_up() => Homeward::Decide(Some(open)),
        _ => Homeward::Decide(None),
    }
}

/// Where the headroom path leaves a seat whose latch does not stand.
#[derive(Debug, PartialEq)]
pub(super) enum Headroom {
    /// Record this act: the seat is done this cycle.
    Act(AutoAct),
    /// Decide this episode like a limit one; none leaves the seat be.
    Decide(Option<crate::autoreseat::Episode>),
}

/// The headroom path of one seat, given its headroom episode `found`: a
/// start's urgent read or a crossing opens an episode where none is; an
/// earlier start's episode ends when a new start reads its own account, and
/// relief ends one no attempt moves while no start is urgent. Otherwise the
/// episode is decided, but with no threshold only a start's own episode is.
pub(super) fn headroom_route(
    settings: &crate::autoreseat::Settings,
    seat: &AutoSeat,
    found: Option<crate::autoreseat::Episode>,
    now: i64,
) -> Headroom {
    use crate::autoreseat::Room;
    let (slot, agent) = (seat.slot.clone(), seat.agent.clone());
    let opening = seat.start.urgent(now);
    let earlier = |open: &crate::autoreseat::Episode| {
        opening && seat.start.settled.is_some_and(|at| open.key.epoch() < at)
    };
    match (&found, &seat.room) {
        (None, _) if opening => {
            let cause = seat
                .start
                .exhausted
                .as_ref()
                .map(crate::autoreseat::headroom_words)
                .unwrap_or_default();
            return Headroom::Act(AutoAct::Open { slot, agent, cause });
        }
        (None, Room::Crossed(window)) if settings.headroom_at.is_some() => {
            let cause = crate::autoreseat::headroom_words(window);
            return Headroom::Act(AutoAct::Open { slot, agent, cause });
        }
        // Each start answers for itself: an earlier start's episode
        // ends, and the next cycle opens this one's.
        (Some(open), _) if open.open.is_none() && earlier(open) => {
            let summary = crate::autoreseat::RESTARTED_SUMMARY.to_owned();
            let key = open.key;
            return Headroom::Act(AutoAct::Rearm {
                slot,
                agent,
                key,
                summary,
            });
        }
        (Some(open), Room::Relieved(window)) if open.open.is_none() && !opening => {
            let summary = crate::autoreseat::relieved_words(window);
            let key = open.key;
            return Headroom::Act(AutoAct::Rearm {
                slot,
                agent,
                key,
                summary,
            });
        }
        _ => {}
    }
    // With no threshold, only a start's own episode moves.
    if settings.headroom_at.is_none()
        && !found
            .as_ref()
            .is_some_and(|open| seat.start.urgent(open.key.epoch()))
    {
        return Headroom::Decide(None);
    }
    Headroom::Decide(found)
}

/// Whether a limit episode with no attempt open is left to the seat's wake
/// this cycle, silently — no trigger and no naming: its known short reset is
/// near or not yet past its fallback ([`crate::autoreseat::reset_wait`]), or
/// the carried reset has passed and the seat resumed on it — the journal
/// proves its wake LANDED, or the tool's own resume was armed for it. That
/// second hold never widens the wait to a reset still ahead, and a fresh
/// reading naming a new binding reset ends it. An open attempt is never
/// masked, and headroom is not this path's.
fn waits_for_reset(seat: &AutoSeat, episode: &crate::autoreseat::Episode, now: i64) -> bool {
    let resumed = |reset: crate::autoreseat::LimitReset| {
        now >= reset.at && (seat.wake_landed == Some(reset.at) || seat.armed == Some(reset.at))
    };
    episode.trigger == crate::autoreseat::Trigger::Limit
        && episode.open.is_none()
        && (crate::autoreseat::reset_wait(seat.reset, now).is_some()
            || seat.reset.is_some_and(resumed))
}

/// Whether a cycle reads the seat's binding reset: only on its limit — the
/// notice drawn, or the latch standing — the one window a wake judges.
pub(super) fn reads_reset(throttle: Option<Throttle>, latched: bool) -> bool {
    throttle == Some(Throttle::LimitReached) || latched
}

/// The tool's own resume as this cycle READ it: a failed capture reads
/// nothing armed, and the latch never takes it for a cancel either.
pub(super) fn armed_reading(capture_ok: bool, capture: &str, tool: crate::tool::ToolKind) -> bool {
    capture_ok && crate::harness_state::auto_continue_armed(capture, tool)
}

/// The first-sight forecast of a seat auto reseat leaves to its reset's wake
/// ([`crate::autoreseat::reset_wait`]), or `None` when the move applies.
pub(super) fn waiting_forecast(
    agent: &str,
    reset: Option<crate::autoreseat::LimitReset>,
    now: i64,
) -> Option<String> {
    let held = reset?;
    crate::autoreseat::reset_wait(reset, now)?;
    Some(format!(
        "ae waits for {agent}'s limit to reset at {} and wakes it then; it moves only if the wake does not land",
        Timestamp::from_epoch(held.at)
    ))
}
