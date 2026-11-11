//! Frozen acceptance spec (slice limitwake): a latched seat whose
//! vendor-limit reset has passed gets ONE continuation turn from the
//! watchdog, without a human or peer. Oracle: brief
//! `.local/brief-limitwake.md` (invariant + AMENDMENT 1), never current
//! code. Everything else about the latch stays: 5b is the only
//! release, a failed capture is no clearing, an auto reseat in flight
//! holds it, nudges stay suppressed while it stands. A wake that
//! LANDED for the carried reset also holds the auto move for that
//! reset — past the fallback too, until a fresh reading names a new
//! binding reset (the re-hit proof): the seat resumed, and the old
//! limit cell still inside the notice window is not a move reason.
//! A vendor that arms its OWN resume (brief AMENDMENT 2: claude's
//! below-box `Continuing automatically at …` footer) owns that reset:
//! no ae wake ever for it while the arming stands — a cancel read
//! BEFORE the reset hands the wake back — and its resume holds the
//! auto move exactly like a landed ae wake.
//!
//! Installed byte-identical at `src/watchdog_limitwake_spec.rs`, wired
//! from `src/watchdog_daemon.rs` via
//! `#[cfg(test)] #[path = "watchdog_limitwake_spec.rs"] mod limitwake_spec;`.

use super::{
    ACTOR, Acting, AutoAct, AutoSeat, Cycle, Effect, HELPER_NAME, HarnessObservation, Knobs,
    LIMIT_WAKE_GRACE_SECS, Observation, PaneState, SendHelper, Verdict, account, auto_acts,
    limit_wake_text,
};
use crate::autoreseat::{
    Frame, HEADROOM_ACTION, IN_FLIGHT_SECS, LimitReset, Pane, Room, Settings, Switch, Trigger,
    Window,
};
use crate::events::Event;
use crate::harness_state::HarnessState;
use crate::inventory::ServerId;
use crate::procs::Descendancy;
use crate::watchdog::{
    DoneProgress, HumanPrompt, QuietKind, Throttle, WaitProgress, quiet_filter, quiet_hash,
};
use std::path::{Path, PathBuf};

/// The episode's first sight, its known reset, and the ruled grace.
const SINCE: i64 = 9_000;
const RESET: i64 = 9_500;
const GRACE: i64 = 60;

fn seen() -> Observation {
    Observation {
        now_epoch: 10_000,
        hash: 7,
        harness: HarnessObservation {
            frame: HarnessState::Unknown,
            human_draft: false,
            durable_stale: false,
            declaration: None,
        },
        identity: 1,
        is_dead: false,
        throttle: None,
        limit_notice: None,
        auto_continue: false,
        limit_reset: None,
        limit_woke_journal: None,
        human_prompt: None,
        capture_ok: true,
        throttle_quota: None,
        quiet: None,
        ended_wait: None,
        done_progress: DoneProgress::None,
        wait_progress: WaitProgress::None,
        wait_rung: 0,
        wait_ends_elsewhere: false,
        target_moved: None,
        descendancy: Descendancy::Present,
        launch_attempt: crate::tmux::Evidence::Silent,
        outstanding_alert: Some(crate::attention::Reason::Dead),
        unreachable: false,
        failed_elsewhere: false,
        prompt_named: false,
        start: None,
        last_actor_event_age_secs: 0,
        declared_age_secs: 0,
        suspended: crate::watchdog::Suspensions::default(),
        sweep: None,
        own_work: crate::session::OwnWork::default(),
        auto_in_flight: None,
        auto_deadline: None,
    }
}

fn reset(at: i64) -> LimitReset {
    LimitReset { at, short: true }
}

/// A latched seat carrying the known reset, as the cycles before the
/// reset left it.
fn latched() -> PaneState {
    PaneState {
        identity: Some(seen().identity),
        limit_since: Some(SINCE),
        limit_reset: Some(reset(RESET)),
        ..PaneState::default()
    }
}

/// The limit still drawn at `now`; `reading` is this cycle's fresh
/// reset reading (gone once the reset passes — the row reads Unknown).
fn limited(now: i64, reading: Option<LimitReset>) -> Observation {
    Observation {
        now_epoch: now,
        throttle: Some(Throttle::LimitReached),
        limit_reset: reading,
        ..seen()
    }
}

/// The vendor's own resume is armed on this cycle's frame.
fn armed(now: i64, reading: Option<LimitReset>) -> Observation {
    Observation {
        auto_continue: true,
        ..limited(now, reading)
    }
}

fn wakes(effects: &[Effect]) -> Vec<i64> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::LimitWake { reset } => Some(*reset),
            _ => None,
        })
        .collect()
}

fn actions(effects: &[Effect]) -> Vec<&str> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { action, .. } => Some(*action),
            _ => None,
        })
        .collect()
}

#[test]
fn a_limit_past_its_reset_plus_grace_books_one_wake_and_keeps_the_latch() {
    assert_eq!(LIMIT_WAKE_GRACE_SECS, GRACE, "the ruled vendor-clock grace");
    let knobs = Knobs::default();
    // The reading is gone by now (freshness drops the row at its
    // reset); the CARRIED reset is what the wake judges.
    let booked = account(&latched(), &limited(RESET + GRACE, None), &knobs);
    assert_eq!(booked.verdict, Verdict::Limit, "a wake is not a release");
    assert_eq!(wakes(&booked.effects), [RESET], "exactly one wake, for R");
    assert!(
        !booked.effects.contains(&Effect::Nudge),
        "the wake spends no nudge"
    );
    assert!(
        !booked.effects.contains(&Effect::QuotaRefresh),
        "recovery is 5b's, on the clear capture that follows"
    );
    assert!(actions(&booked.effects).is_empty(), "nothing is named");
    assert_eq!(booked.next.limit_since, Some(SINCE), "the latch stands");
    assert_eq!(
        booked.next.limit_reset,
        Some(reset(RESET)),
        "a reading gone quiet never clears the carry"
    );
    assert_eq!(
        booked.next.limit_woken, None,
        "booking does not spend R — delivery does"
    );
    assert_eq!(booked.next.nudge_count, 0);
    // Undelivered, it books again next cycle: the attempt bound and
    // delivery are what stop it, not the booking.
    let again = account(&booked.next, &limited(RESET + GRACE + 60, None), &knobs);
    assert_eq!(wakes(&again.effects), [RESET]);
}

#[test]
fn no_wake_before_the_reset_or_inside_the_grace() {
    let knobs = Knobs::default();
    for now in [RESET - 1, RESET, RESET + GRACE - 1] {
        let cycle = account(&latched(), &limited(now, Some(reset(RESET))), &knobs);
        assert_eq!(cycle.verdict, Verdict::Limit, "{now}");
        assert!(wakes(&cycle.effects).is_empty(), "now {now}: too early");
    }
    let due = account(&latched(), &limited(RESET + GRACE, None), &knobs);
    assert_eq!(wakes(&due.effects), [RESET], "at reset + grace exactly");
}

#[test]
fn no_wake_when_the_reset_is_unknown() {
    let knobs = Knobs::default();
    let unknowing = PaneState {
        identity: Some(seen().identity),
        limit_since: Some(SINCE),
        ..PaneState::default()
    };
    let cycle = account(&unknowing, &limited(1_000_000, None), &knobs);
    assert_eq!(cycle.verdict, Verdict::Limit);
    assert!(
        wakes(&cycle.effects).is_empty(),
        "never guess a time, however long the seat has waited"
    );
    // The cycle a reset first reads adopts and judges it.
    let late = account(
        &unknowing,
        &limited(99_000 + GRACE, Some(reset(99_000))),
        &knobs,
    );
    assert_eq!(late.next.limit_reset, Some(reset(99_000)));
    assert_eq!(wakes(&late.effects), [99_000]);
}

#[test]
fn a_spent_reset_never_wakes_again_from_carry_or_journal() {
    let knobs = Knobs::default();
    let spent_carry = PaneState {
        limit_woken: Some(RESET),
        ..latched()
    };
    let cycle = account(&spent_carry, &limited(RESET + GRACE, None), &knobs);
    assert!(wakes(&cycle.effects).is_empty(), "woken this daemon run");
    // A restarted daemon holds no carry; the journal's delivered (or
    // unconfirmed) wake for the same R still spends it — never resend.
    let journaled = Observation {
        limit_woke_journal: Some(RESET),
        ..limited(RESET + GRACE, None)
    };
    let restarted = account(&latched(), &journaled, &knobs);
    assert!(
        wakes(&restarted.effects).is_empty(),
        "the journal already holds this R's wake"
    );
    // A journal spent on an OLDER reset spends nothing about this one.
    let older = Observation {
        limit_woke_journal: Some(RESET - 100),
        ..limited(RESET + GRACE, None)
    };
    let fresh = account(&latched(), &older, &knobs);
    assert_eq!(wakes(&fresh.effects), [RESET]);
}

/// The armed proof is the LIVE footer BELOW claude's box, as measured:
/// transcript rows above the box are history and prove nothing — the
/// cancelled fixture carries an armed history row and the cancelled
/// row, and no footer.
#[test]
fn the_vendor_armed_row_counts_only_below_the_box() {
    use crate::harness_state::auto_continue_armed;
    use crate::tool::ToolKind;
    const ARMED_REAL: &str =
        include_str!("../tests/fixtures/harness-state/claude-limit-2.1.296-180x50.txt");
    const CANCELLED_REAL: &str =
        include_str!("../tests/fixtures/harness-state/claude-limit-cancelled-2.1.296-180x50.txt");
    assert!(
        auto_continue_armed(ARMED_REAL, ToolKind::Claude),
        "the measured armed frame"
    );
    assert!(
        !auto_continue_armed(CANCELLED_REAL, ToolKind::Claude),
        "the measured cancelled frame: armed history, no footer"
    );
    assert!(
        !auto_continue_armed(ARMED_REAL, ToolKind::Codex),
        "another tool's grammar never reads claude's footer"
    );
    let frame = |above: &str, below: &str| {
        format!("{above}───\n❯\n───\n{below}🧠 Opus 5.5 (low)  📁 work\n⏵⏵ bypass permissions on\n")
    };
    assert!(
        auto_continue_armed(
            &frame("", "Continuing automatically at 1am · Esc to cancel\n"),
            ToolKind::Claude
        ),
        "the footer row below the box"
    );
    assert!(
        !auto_continue_armed(
            &frame("Continuing automatically at 1am · esc to cancel\n", ""),
            ToolKind::Claude
        ),
        "the same words quoted above the box are transcript"
    );
    assert!(!auto_continue_armed(&frame("", ""), ToolKind::Claude));
    assert!(!auto_continue_armed("hello\n", ToolKind::Claude));
    assert!(!auto_continue_armed("", ToolKind::Claude));
}

#[test]
fn an_armed_vendor_resume_is_never_woken_for_that_reset() {
    let knobs = Knobs::default();
    let booked = account(&latched(), &armed(RESET + GRACE, None), &knobs);
    assert_eq!(booked.verdict, Verdict::Limit);
    assert!(
        wakes(&booked.effects).is_empty(),
        "the vendor owns this reset"
    );
    assert_eq!(
        booked.next.limit_armed,
        Some(RESET),
        "the arming is carried against its reset"
    );
    // The vendor fires and the footer goes: still no ae wake for R,
    // however much later — a second continue is never ae's to type.
    for now in [RESET + GRACE + 120, RESET + 100_000] {
        let cycle = account(&booked.next, &limited(now, None), &knobs);
        assert!(
            wakes(&cycle.effects).is_empty(),
            "now {now}: R is spent on the vendor"
        );
    }
    // Armed on the very cycle its reset first reads: adopted, armed,
    // and judged in one accounting.
    let unknowing = PaneState {
        identity: Some(seen().identity),
        limit_since: Some(SINCE),
        ..PaneState::default()
    };
    let late = account(
        &unknowing,
        &armed(99_000 + GRACE, Some(reset(99_000))),
        &knobs,
    );
    assert!(wakes(&late.effects).is_empty());
    assert_eq!(late.next.limit_armed, Some(99_000));
    // The 5b release clears the arming with the episode.
    let released = account(&booked.next, &seen(), &knobs);
    assert_eq!(actions(&released.effects), ["alert-cleared"]);
    assert_eq!(released.next.limit_armed, None);
    // A re-hit names a new binding reset; the old arming is inert
    // against it and that reset wakes on the ordinary rule.
    let named = account(&booked.next, &limited(9_800, Some(reset(20_000))), &knobs);
    assert!(wakes(&named.effects).is_empty(), "not yet");
    let due = account(&named.next, &limited(20_000 + GRACE, None), &knobs);
    assert_eq!(wakes(&due.effects), [20_000]);
}

#[test]
fn a_cancel_read_before_the_reset_hands_the_wake_back() {
    let knobs = Knobs::default();
    let arming = account(&latched(), &armed(RESET - 600, Some(reset(RESET))), &knobs);
    assert_eq!(arming.next.limit_armed, Some(RESET));
    // A blind cycle proves nothing either way.
    let blind = Observation {
        now_epoch: RESET - 120,
        capture_ok: false,
        ..seen()
    };
    let held = account(&arming.next, &blind, &knobs);
    assert_eq!(
        held.next.limit_armed,
        Some(RESET),
        "a failed capture neither arms nor disarms"
    );
    // Read again, footer gone, still before the reset: the vendor was
    // cancelled (esc) and will not fire — the reset is ae's again.
    let cancelled = account(&held.next, &limited(RESET - 60, None), &knobs);
    assert_eq!(
        cancelled.next.limit_armed, None,
        "cancelled before the reset"
    );
    let due = account(&cancelled.next, &limited(RESET + GRACE, None), &knobs);
    assert_eq!(wakes(&due.effects), [RESET], "and the wake fires as ever");
    // The wire race, named: a footer first seen gone at or after the
    // reset cannot be told from the vendor having fired, so the
    // arming stands and no wake follows.
    let at_the_wire = account(&arming.next, &limited(RESET + GRACE, None), &knobs);
    assert_eq!(at_the_wire.next.limit_armed, Some(RESET));
    assert!(wakes(&at_the_wire.effects).is_empty());
}

#[test]
fn a_rehit_rearms_on_the_next_reset_without_a_loop() {
    let knobs = Knobs::default();
    // The wake for R lands; apply's marking is written here as the
    // state it leaves (its own pin is the apply spec below).
    let woken = account(&latched(), &limited(RESET + GRACE, None), &knobs);
    let delivered = PaneState {
        limit_woken: Some(RESET),
        // Refusals spent against R must not tax the next reset.
        limit_wake_attempts: 2,
        ..woken.next
    };
    // The woken turn hits the limit again: phrase still drawn, no new
    // reading, and even a stale row naming the same R changes nothing.
    for reading in [None, Some(reset(RESET))] {
        let rehit = account(&delivered, &limited(RESET + GRACE + 60, reading), &knobs);
        assert_eq!(rehit.verdict, Verdict::Limit);
        assert!(
            wakes(&rehit.effects).is_empty(),
            "R is spent: {reading:?} starts no loop"
        );
    }
    // A NEXT known reset re-arms the one wake, attempts from zero.
    let next_reset = 20_000;
    let named = account(
        &delivered,
        &limited(RESET + GRACE + 120, Some(reset(next_reset))),
        &knobs,
    );
    assert!(wakes(&named.effects).is_empty(), "not yet");
    assert_eq!(named.next.limit_reset, Some(reset(next_reset)));
    assert_eq!(
        named.next.limit_wake_attempts, 0,
        "a new reset starts its own attempts"
    );
    let due = account(&named.next, &limited(next_reset + GRACE, None), &knobs);
    assert_eq!(wakes(&due.effects), [next_reset], "one wake for the new R");
}

#[test]
fn the_wake_is_bounded_by_refused_attempts() {
    let knobs = Knobs::default();
    let spent_tries = PaneState {
        limit_wake_attempts: knobs.undelivered_max,
        ..latched()
    };
    let cycle = account(&spent_tries, &limited(RESET + GRACE, None), &knobs);
    assert!(
        wakes(&cycle.effects).is_empty(),
        "refusals at the bound book no further wake for this R"
    );
    let one_left = PaneState {
        limit_wake_attempts: knobs.undelivered_max - 1,
        ..latched()
    };
    let cycle = account(&one_left, &limited(RESET + GRACE, None), &knobs);
    assert_eq!(wakes(&cycle.effects), [RESET], "one below the bound books");
}

#[test]
fn a_busy_draft_prompt_or_moving_seat_is_not_woken() {
    let knobs = Knobs::default();
    let due = limited(RESET + GRACE, None);
    let prompt = HumanPrompt {
        question: "Do you trust the contents of this project?".to_owned(),
        keys: "↑/↓ Navigate · enter Confirm".to_owned(),
        trust: None,
        dismiss: false,
    };
    let guarded = [
        (
            "busy: a turn is already running",
            Observation {
                harness: HarnessObservation {
                    frame: HarnessState::Busy,
                    ..due.harness
                },
                ..due.clone()
            },
        ),
        (
            "a human draft sits in the box",
            Observation {
                harness: HarnessObservation {
                    frame: HarnessState::Idle,
                    human_draft: true,
                    ..due.harness
                },
                ..due.clone()
            },
        ),
        (
            "a prompt only the human may answer",
            Observation {
                human_prompt: Some(prompt),
                ..due.clone()
            },
        ),
        (
            "an auto reseat is moving the seat",
            Observation {
                auto_in_flight: Some(Trigger::Limit),
                ..due.clone()
            },
        ),
    ];
    for (why, seen_guarded) in guarded {
        let cycle = account(&latched(), &seen_guarded, &knobs);
        assert_eq!(cycle.verdict, Verdict::Limit, "{why}");
        assert!(wakes(&cycle.effects).is_empty(), "not typed into: {why}");
        assert_eq!(cycle.next.limit_since, Some(SINCE), "{why}: latch holds");
    }
    // The controls: an idle composer and an unknown frame (the night
    // panes read unknown; the guarded delivery refuses for itself).
    let idle = Observation {
        harness: HarnessObservation {
            frame: HarnessState::Idle,
            ..due.harness
        },
        ..due.clone()
    };
    assert_eq!(wakes(&account(&latched(), &idle, &knobs).effects), [RESET]);
    assert_eq!(wakes(&account(&latched(), &due, &knobs).effects), [RESET]);
}

#[test]
fn a_failed_capture_neither_clears_nor_wakes() {
    let knobs = Knobs::default();
    let blind = Observation {
        now_epoch: RESET + GRACE,
        capture_ok: false,
        ..seen()
    };
    let held = account(&latched(), &blind, &knobs);
    assert_eq!(held.verdict, Verdict::Limit, "no reading is not a clearing");
    assert_eq!(held.next.limit_since, Some(SINCE));
    assert_eq!(held.next.limit_reset, Some(reset(RESET)), "carry untouched");
    assert!(
        wakes(&held.effects).is_empty(),
        "and no wake into a blind pane"
    );
    assert!(actions(&held.effects).is_empty(), "no retraction");
    assert!(!held.effects.contains(&Effect::QuotaRefresh), "no pass");
    // The next good capture, still limited, still owes the wake: one
    // blind cycle loses nothing.
    let owed = account(&held.next, &limited(RESET + GRACE + 60, None), &knobs);
    assert_eq!(wakes(&owed.effects), [RESET]);
}

#[test]
fn the_next_clear_capture_releases_as_today_and_the_next_episode_starts_clean() {
    let knobs = Knobs::default();
    let delivered = PaneState {
        limit_woken: Some(RESET),
        ..latched()
    };
    let released = account(&delivered, &seen(), &knobs);
    assert_eq!(released.verdict, Verdict::Active);
    assert_eq!(
        actions(&released.effects),
        ["alert-cleared"],
        "5b unchanged"
    );
    assert!(released.effects.contains(&Effect::QuotaRefresh), "one pass");
    assert_eq!(released.next.limit_since, None);
    assert_eq!(
        released.next.limit_reset, None,
        "the spent episode's reset leaves with it"
    );
    assert_eq!(released.next.limit_wake_attempts, 0);
    assert_eq!(
        released.next.limit_woken,
        Some(RESET),
        "the spent mark survives the release"
    );
    // A new episode with no fresh reading carries no old reset into it.
    let again = account(&released.next, &limited(30_000, None), &knobs);
    assert_eq!(again.verdict, Verdict::Limit);
    assert_eq!(
        actions(&again.effects),
        ["limit"],
        "a new episode, named once"
    );
    assert!(
        wakes(&again.effects).is_empty(),
        "the old R wakes nothing here"
    );
}

#[test]
fn a_quiet_seat_the_limit_outranks_is_woken_on_the_same_rule() {
    let knobs = Knobs::default();
    // First seen 60 s ago, declared 180 s ago: the limit outranks the
    // declaration and ends nothing (existing rule), so it wakes too.
    let declared = |now: i64| Observation {
        quiet: Some(QuietKind::Done),
        declared_age_secs: 180,
        ..limited(now, None)
    };
    let prior = PaneState {
        limit_since: Some(RESET + GRACE - 60),
        ..latched()
    };
    let early = account(&prior, &declared(RESET + GRACE - 1), &knobs);
    assert_eq!(early.verdict, Verdict::Limit);
    assert!(wakes(&early.effects).is_empty(), "inside the grace");
    let due = account(&prior, &declared(RESET + GRACE), &knobs);
    assert_eq!(due.verdict, Verdict::Limit);
    assert_eq!(wakes(&due.effects), [RESET]);
    assert!(
        !due.effects.contains(&Effect::Nudge),
        "the declaration is not nudged around the wake"
    );
}

#[test]
fn a_seat_back_from_the_dead_carries_no_stale_reset_into_its_new_episode() {
    let knobs = Knobs::default();
    let dead = PaneState {
        identity: Some(seen().identity),
        dead_latched: true,
        limit_since: Some(SINCE),
        limit_reset: Some(reset(RESET)),
        limit_armed: Some(RESET),
        ..PaneState::default()
    };
    let back = account(&dead, &limited(RESET + GRACE, None), &knobs);
    assert_eq!(back.verdict, Verdict::Limit);
    assert_eq!(actions(&back.effects), ["dead-cleared", "limit"]);
    assert_eq!(
        back.next.limit_reset, None,
        "the resurrection resets the carry, reset included"
    );
    assert_eq!(back.next.limit_armed, None, "and its arming");
    assert!(
        wakes(&back.effects).is_empty(),
        "no wake off a dead episode's reset"
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one table of the amendment's cases over one scaffolding"
)]
fn auto_reseat_waits_for_a_near_short_reset_and_moves_otherwise() {
    use crate::autoreseat::ATTEMPT_ACTION;
    let record = |at: i64, action: &str, agent: &str, rest: &str| {
        Event::parse_line(&format!(
            r#"{{"ts":"{}","actor":"{ACTOR}","action":"{action}","target":"{agent}"{rest}}}"#,
            crate::time::Timestamp::from_epoch(at)
        ))
        .expect("a journal line")
    };
    let key = crate::time::Timestamp::from_epoch(100);
    let seat = |profile: &str,
                limited: bool,
                room: Room,
                seat_reset: Option<LimitReset>,
                wake_landed: Option<i64>,
                armed_reset: Option<i64>| AutoSeat {
        slot: "spawned.1".to_owned(),
        agent: "one".to_owned(),
        profile: profile.to_owned(),
        pane: Pane {
            frame: Frame::Clear,
            human_prompt: false,
            client_input: None,
        },
        limited,
        room,
        derives: false,
        start: crate::autoreseat::StartRead::default(),
        reset: seat_reset,
        wake_landed,
        armed: armed_reset,
    };
    let on = Settings {
        switch: Switch::On,
        sessions: Some(vec!["dev".to_owned()]),
        grace_secs: 0,
        map: vec![("sol6x".to_owned(), vec!["opus55x".to_owned()])],
        headroom_at: None,
        headroom_note: None,
        lead: crate::autoreseat::Lead::default(),
        floor: crate::autoreseat::FloorSetting::None,
        floor_note: None,
        tiers: Vec::new(),
        lead_seat: false,
        notes: Vec::new(),
    };
    let acts = |settings: &Settings, seats: &[AutoSeat], events: &[Event], now: i64| {
        auto_acts(
            settings,
            seats,
            events,
            ("dev", false),
            now,
            &mut Vec::new(),
        )
    };
    let trigger = vec![AutoAct::Trigger {
        agent: "one".to_owned(),
    }];
    let now = 1000;
    let episode = [record(100, "limit", "one", "")];
    let near = |at: i64| Some(LimitReset { at, short: true });
    let latched_seat =
        |seat_reset: Option<LimitReset>| seat("sol6x", true, Room::Unread, seat_reset, None, None);
    // The same seat with a wake the journal proves landed (a confirmed
    // `limit-wake` row for that epoch — never an unconfirmed one).
    let landed_seat = |seat_reset: Option<LimitReset>, landed: i64| {
        seat("sol6x", true, Room::Unread, seat_reset, Some(landed), None)
    };
    // The same seat with the vendor's own resume armed for an epoch
    // (the daemon's carry of the below-box footer proof).
    let armed_seat = |seat_reset: Option<LimitReset>, armed: i64| {
        seat("sol6x", true, Room::Unread, seat_reset, None, Some(armed))
    };
    // The control: unknown reset moves exactly as today.
    assert_eq!(
        acts(&on, &[latched_seat(None)], &episode, now),
        trigger,
        "unknown reset moves"
    );
    // Near short reset: silent wait — no trigger, no naming.
    for at in [now + 1800, now + 3599, now - 1] {
        assert!(
            acts(&on, &[latched_seat(near(at))], &episode, now).is_empty(),
            "reset at {at}: waits"
        );
    }
    // 60 min or more away at decision time: moves.
    assert_eq!(
        acts(&on, &[latched_seat(near(now + 3600))], &episode, now),
        trigger,
        "exactly 60 min away moves"
    );
    // Not proven short (weekly/long or unstated): moves even when near.
    let long = Some(LimitReset {
        at: now + 600,
        short: false,
    });
    assert_eq!(
        acts(&on, &[latched_seat(long)], &episode, now),
        trigger,
        "a long window's near reset still moves"
    );
    // Wait expired (now at reset + fallback, still latched, clear frame).
    assert_eq!(
        acts(&on, &[latched_seat(near(now - 300))], &episode, now),
        trigger,
        "past reset + fallback the move applies"
    );
    assert!(
        acts(&on, &[latched_seat(near(now - 299))], &episode, now).is_empty(),
        "one second of fallback left still waits"
    );
    // A wake that LANDED for the carried reset holds the move for that
    // reset past the fallback: the seat resumed, and the old limit
    // cell still inside the notice window keeps the latch (and this
    // path) standing — it is not a move reason. Short or long alike.
    for landed_reset in [near(now - 300), near(now - 100_000)] {
        let at = landed_reset.map_or(0, |held| held.at);
        assert!(
            acts(&on, &[landed_seat(landed_reset, at)], &episode, now).is_empty(),
            "landed for {at}: no trigger however far past the fallback"
        );
    }
    let long_past = Some(LimitReset {
        at: now - 300,
        short: false,
    });
    assert!(
        acts(&on, &[landed_seat(long_past, now - 300)], &episode, now).is_empty(),
        "a landed long-window wake holds its reset too"
    );
    // A landing for a DIFFERENT reset holds nothing about this one,
    // and a fresh reading naming a new binding reset (the re-hit
    // proof) hands the seat back to the ordinary rules.
    assert_eq!(
        acts(
            &on,
            &[landed_seat(near(now - 300), now - 9999)],
            &episode,
            now
        ),
        trigger,
        "landed for an older reset, this one past its fallback: moves"
    );
    assert_eq!(
        acts(
            &on,
            &[landed_seat(near(now + 7200), now - 300)],
            &episode,
            now
        ),
        trigger,
        "a new far binding reset re-arms the move path"
    );
    // The vendor's own armed resume plays the landed role — and
    // nothing more: inside the wait it changes nothing, past the
    // fallback it holds that reset, and it never widens the wait to
    // a far reset the amendment says to move on.
    assert!(
        acts(
            &on,
            &[armed_seat(near(now + 1800), now + 1800)],
            &episode,
            now
        )
        .is_empty(),
        "armed and near: waits, the vendor resumes it"
    );
    assert!(
        acts(
            &on,
            &[armed_seat(near(now - 300), now - 300)],
            &episode,
            now
        )
        .is_empty(),
        "armed for the carried reset: no trigger past the fallback"
    );
    assert!(
        acts(
            &on,
            &[armed_seat(near(now - 100_000), now - 100_000)],
            &episode,
            now
        )
        .is_empty(),
        "however far past, while that reset is the carried one"
    );
    assert_eq!(
        acts(
            &on,
            &[armed_seat(near(now + 3600), now + 3600)],
            &episode,
            now
        ),
        trigger,
        "armed does not widen the sixty-minute wait"
    );
    assert_eq!(
        acts(
            &on,
            &[armed_seat(near(now + 7200), now - 300)],
            &episode,
            now
        ),
        trigger,
        "a new far binding reset releases an old arming"
    );
    // An attempt already open is never masked by the wait: past its
    // bound it closes overdue, near reset or not.
    let mut flying = episode.to_vec();
    flying.push(record(
        900,
        ATTEMPT_ACTION,
        "one",
        &format!(r#","ref":"{key}""#),
    ));
    let overdue_at = 900 + IN_FLIGHT_SECS;
    assert_eq!(
        acts(
            &on,
            &[latched_seat(near(overdue_at + 600))],
            &flying,
            overdue_at
        ),
        [AutoAct::Overdue {
            slot: "spawned.1".to_owned(),
            agent: "one".to_owned(),
            key,
            trigger: Trigger::Limit,
        }]
    );
    // Nor is it masked by a landed wake: the hold applies only while
    // no attempt is open.
    assert_eq!(
        acts(
            &on,
            &[landed_seat(near(overdue_at - 300), overdue_at - 300)],
            &flying,
            overdue_at
        ),
        [AutoAct::Overdue {
            slot: "spawned.1".to_owned(),
            agent: "one".to_owned(),
            key,
            trigger: Trigger::Limit,
        }],
        "an open attempt still closes overdue"
    );
    assert_eq!(
        acts(
            &on,
            &[armed_seat(near(overdue_at - 300), overdue_at - 300)],
            &flying,
            overdue_at
        ),
        [AutoAct::Overdue {
            slot: "spawned.1".to_owned(),
            agent: "one".to_owned(),
            key,
            trigger: Trigger::Limit,
        }],
        "nor masked by an armed vendor resume"
    );
    // The unmapped naming waits too: it is deferred to the fallback,
    // not lost.
    let unmapped = |seat_reset: Option<LimitReset>, wake_landed: Option<i64>| {
        seat("ghostx", true, Room::Unread, seat_reset, wake_landed, None)
    };
    let unmapped_armed = |seat_reset: Option<LimitReset>, armed: i64| {
        seat("ghostx", true, Room::Unread, seat_reset, None, Some(armed))
    };
    let naming = [AutoAct::Unmapped {
        slot: "spawned.1".to_owned(),
        agent: "one".to_owned(),
        key,
        profile: "ghostx".to_owned(),
    }];
    assert_eq!(
        acts(&on, &[unmapped(None, None)], &episode, now),
        naming,
        "control: named today"
    );
    assert!(
        acts(&on, &[unmapped(near(now + 1800), None)], &episode, now).is_empty(),
        "a seat that will wake is not told it has nowhere to move"
    );
    assert_eq!(
        acts(&on, &[unmapped(near(now - 300), None)], &episode, now),
        naming,
        "the deferred naming lands at the fallback"
    );
    assert!(
        acts(
            &on,
            &[unmapped(near(now - 300), Some(now - 300))],
            &episode,
            now
        )
        .is_empty(),
        "but never for a reset whose wake landed: the seat resumed"
    );
    assert!(
        acts(
            &on,
            &[unmapped_armed(near(now - 300), now - 300)],
            &episode,
            now
        )
        .is_empty(),
        "nor for a reset the vendor was armed for"
    );
    // Headroom episodes are untouched by the reset wait.
    let headroom_on = Settings {
        headroom_at: Some(95),
        ..on.clone()
    };
    let window = Window {
        judged: 96.0,
        critical: true,
        long: true,
        observed_at: 90,
        resets_at: None,
        status: crate::quota::Status::Fresh,
        label: "weekly_all 7d".to_owned(),
    };
    let opened = [record(
        100,
        HEADROOM_ACTION,
        "one",
        r#","summary":"headroom 96% weekly_all 7d""#,
    )];
    let crossing = seat(
        "sol6x",
        false,
        Room::Crossed(window),
        near(now + 1800),
        Some(now + 1800),
        Some(now + 1800),
    );
    assert_eq!(
        acts(&headroom_on, &[crossing], &opened, now),
        trigger,
        "only the limit path waits"
    );
}

/// One scratch session dir, removed with the test.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae-limitwake-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A send helper with `script` as its body, in the staged name the
/// cycle's `SendHelper` reads.
fn helper_with(dir: &Path, script: &str) -> SendHelper {
    use std::os::unix::fs::PermissionsExt;
    let send = dir.join(HELPER_NAME);
    std::fs::write(&send, script).expect("the fake helper");
    std::fs::set_permissions(&send, std::fs::Permissions::from_mode(0o755)).expect("exec");
    SendHelper::for_session(dir)
}

/// What the journaling helper booked: action, ref, summary per row.
fn booked(dir: &Path) -> Vec<(String, Option<String>, Option<String>)> {
    let journal = crate::session::SessionRead::open(dir).expect("the helper ran");
    journal
        .events
        .into_iter()
        .map(|event| (event.action, event.reference, event.summary))
        .collect()
}

fn spec_cycle<'a>(dir: &'a Path, helper: &'a SendHelper, server: &'a ServerId) -> Cycle<'a> {
    Cycle {
        knobs: Knobs::default(),
        meta_dir: dir,
        helper,
        server,
        session: "demo",
        goal: None,
        roster: Vec::new(),
        local_config: None,
        lead_pair: false,
        fleet_order: crate::theme::FleetOrder::EMPTY,
        auto: crate::autoreseat::settings(None),
        auto_trust: crate::config::AutoTrust::Off,
        meta_agent: false,
        launch_ids: Vec::new(),
        started: None,
        settled: None,
    }
}

#[test]
fn a_delivered_wake_marks_the_reset_spent_and_journals_it_under_its_ref() {
    assert_eq!(crate::watchdog::LIMIT_WAKE_ACTION, "limit-wake");
    let scratch = Scratch::new("delivered");
    let script = "#!/bin/sh\nprintf '{\"ts\":\"2026-09-23T00:00:00Z\",\"actor\":\"watchdog\",\"action\":\"%s\",\"ref\":\"%s\",\"summary\":\"%s\"}\\n' \"$_AE_EVENT_ACTION\" \"$_AE_EVENT_REF\" \"$_AE_EVENT_SUMMARY\" >> \"$(dirname \"$0\")/events.jsonl\"\n";
    let helper = helper_with(&scratch.0, script);
    let server = ServerId::Ambient;
    let observed = limited(RESET + GRACE, None);
    let on = Acting {
        agent: "lead",
        slot: "main",
        pane: "%0",
        seen: &observed,
        events: &[],
        overview: None,
    };
    let mut state = PaneState {
        // A sentinel: the wake is not a nudge and spends no budget.
        nudge_count: 1,
        ..latched()
    };
    spec_cycle(&scratch.0, &helper, &server)
        .apply(
            &Effect::LimitWake { reset: RESET },
            &on,
            &mut state,
            &mut Vec::new(),
        )
        .expect("the wake is delivered");
    assert_eq!(state.limit_woken, Some(RESET), "R is spent on delivery");
    assert_eq!(state.limit_wake_attempts, 0);
    assert_eq!(state.nudge_count, 1, "untouched");
    let rows = booked(&scratch.0);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].0, "limit-wake", "its own action, never the nudge's");
    assert_eq!(
        rows[0].1.as_deref(),
        Some(RESET.to_string().as_str()),
        "the ref is the reset epoch — the restart half of the spent proof"
    );
    // A confirmed wake is a landed paste: it ends an unreachable
    // failure like a nudge does. An unconfirmed one does not read
    // landed, but still spends R through the journal's presence.
    let landed = Event::parse_line(&format!(
        r#"{{"ts":"2026-09-23T00:00:00Z","actor":"{ACTOR}","action":"limit-wake","ref":"{RESET}","summary":"woke"}}"#
    ))
    .expect("a wake record");
    assert!(crate::events::landed(&landed));
    let unconfirmed = Event::parse_line(&format!(
        r#"{{"ts":"2026-09-23T00:00:00Z","actor":"{ACTOR}","action":"limit-wake","ref":"{RESET}","summary":"[unconfirmed] woke"}}"#
    ))
    .expect("an unconfirmed wake record");
    assert!(!crate::events::landed(&unconfirmed));
}

#[test]
fn a_refused_wake_counts_one_attempt_and_spends_nothing() {
    let scratch = Scratch::new("refused");
    let helper = helper_with(&scratch.0, "#!/bin/sh\nexit 3\n");
    let server = ServerId::Ambient;
    let observed = limited(RESET + GRACE, None);
    let on = Acting {
        agent: "lead",
        slot: "main",
        pane: "%0",
        seen: &observed,
        events: &[],
        overview: None,
    };
    let mut state = PaneState {
        nudge_count: 1,
        ..latched()
    };
    spec_cycle(&scratch.0, &helper, &server)
        .apply(
            &Effect::LimitWake { reset: RESET },
            &on,
            &mut state,
            &mut Vec::new(),
        )
        .expect("a refusal is booked, not raised");
    assert_eq!(state.limit_woken, None, "nothing landed, nothing spent");
    assert_eq!(state.limit_wake_attempts, 1);
    assert_eq!(state.nudge_count, 1, "untouched");
}

#[test]
fn the_wake_text_is_a_nudge_footprint_and_names_the_reset() {
    let meta = Path::new("/home/x/.ae/sessions/demo");
    for goal in [None, Some("ship the slice")] {
        let text = limit_wake_text(goal, meta, RESET);
        assert_eq!(
            quiet_filter(&text),
            "",
            "the wake body is a footprint, not agent output: {text}"
        );
        assert_eq!(
            quiet_hash("live output\n"),
            quiet_hash(&format!("live output\n{text}\n")),
            "a delivered wake must not move the pane hash"
        );
        assert!(
            text.contains(&crate::time::Timestamp::from_epoch(RESET).to_string()),
            "the seat is told which reset passed: {text}"
        );
    }
}
