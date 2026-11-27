//! #225 acceptance, authored from the brief and issue before implementation.
//! The lead ruled an eight-cadence cap, nine-cadence ceiling, and three-cadence
//! motion bound. Expected values below are requirements, not product outputs.
//! Pure production seams exercise journal provenance, direct target lookup,
//! accounting, and durable witnesses. Daemon observation/writer wiring needs
//! an independent read or live private daemon verification in addition.

#![allow(clippy::expect_used, reason = "acceptance fixture setup")]

use ae::attention::Reason;
use ae::digest::Status;
use ae::events::{Event, RefMeaning};
use ae::harness_state::HarnessState;
use ae::meta::Meta;
use ae::session::{AgentRuntime, RecordSnapshot, SessionRead, SessionRuntime};
use ae::time::Timestamp;
use ae::wait_on::{self, Facts};
use ae::watchdog::{
    self, DoneProgress, HoldReason, HoldWitness, ProofQuiet, QuietKind, WaitProgress, WaitState,
};
use ae::watchdog_daemon::{
    Activity, Effect, HarnessObservation, Knobs, Observation, PaneState, TargetMotion, WaitTarget,
    account_with_activity, target_held, target_motion,
};

const CADENCE: u64 = 300;
const CAP_CADENCES: u64 = 8;
const SESSION: &str = "holdspec";
const PROOF: i64 = 10_000;

fn at(seconds: i64) -> Timestamp {
    Timestamp::from_epoch(seconds)
}

fn event(seconds: i64, actor: &str, slot: &str, action: &str, extra: &str) -> Event {
    Event::parse_line(&format!(
        r#"{{"ts":"{}","actor":"{actor}","actor_slot":"{slot}","actor_session":"{SESSION}","action":"{action}"{extra}}}"#,
        at(seconds)
    ))
    .expect("valid acceptance event")
}

fn declaration(seconds: i64, actor: &str, slot: &str, state: &str, on: Option<&str>) -> Event {
    let on = on.map_or_else(String::new, |on| format!(r#", "wait_on":"{on}""#));
    event(
        seconds,
        actor,
        slot,
        "state",
        &format!(r#", "ref":"{state}"{on}"#),
    )
}

fn journal(on: Option<&str>, target_state: &str) -> Vec<Event> {
    vec![
        declaration(
            PROOF - 1,
            "worker",
            "spawned.1",
            target_state,
            Some("waiter"),
        ),
        declaration(PROOF, "waiter", "main", "waiting-agent", on),
    ]
}

fn lookup(events: &[Event], worker_present: bool) -> Option<String> {
    let mut seats = vec![("main", "waiter", "waiter")];
    if worker_present {
        seats.push(("spawned.1", "worker", "worker"));
    }
    let facts = Facts::read(events, SESSION, &seats);
    wait_on::held_on(&Facts::links(&facts), "waiter").map(str::to_owned)
}

fn due(age: u64) -> WaitProgress {
    WaitProgress::ChallengeDue {
        confirmations: 0,
        required: 2,
        wait_age_secs: age,
        attempts: 0,
    }
}

fn target(motion: TargetMotion) -> WaitTarget {
    WaitTarget {
        agent: "worker".to_owned(),
        motion,
    }
}

fn observed(age: u64, kind: QuietKind) -> Observation {
    Observation {
        now_epoch: PROOF + i64::try_from(age).expect("bounded age"),
        hash: 7,
        harness: HarnessObservation {
            frame: HarnessState::Idle,
            human_draft: false,
            durable_stale: false,
            declaration: None,
        },
        identity: 1,
        is_dead: false,
        throttle: None,
        limit_notice: None,
        limit_reset: None,
        limit_woke_journal: None,
        auto_continue: false,
        capture_ok: true,
        human_prompt: None,
        throttle_quota: None,
        quiet: Some(kind),
        ended_wait: None,
        done_progress: DoneProgress::None,
        wait_progress: due(age),
        wait_rung: 0,
        wait_ends_elsewhere: false,
        target_moved: None,
        descendancy: ae::procs::Descendancy::Present,
        launch_attempt: ae::tmux::Evidence::Silent,
        outstanding_alert: None,
        unreachable: false,
        failed_elsewhere: false,
        prompt_named: false,
        start: None,
        last_actor_event_age_secs: age,
        declared_age_secs: age,
        suspended: watchdog::Suspensions::default(),
        sweep: None,
        own_work: ae::session::OwnWork::default(),
        auto_in_flight: None,
        auto_deadline: None,
    }
}

fn activity(motion: Option<TargetMotion>, held: Option<HoldWitness>) -> Activity {
    Activity {
        proof: Some(ProofQuiet {
            proof: Some(at(PROOF)),
            unchanged: true,
            settle: Some(at(PROOF)),
        }),
        wait_target: motion.map(target),
        wait_witness: held,
        ..Activity::default()
    }
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "fixture helper matches the optional witness inputs and assertions"
)]
fn witness(reason: HoldReason) -> Option<HoldWitness> {
    Some(HoldWitness { reason, open: true })
}

fn knobs() -> Knobs {
    Knobs {
        idle_nudge_secs: CADENCE,
        done_confirmations: 2,
        ..Knobs::default()
    }
}

fn challenged(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::WaitChallenge { .. }))
}

fn held(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, Effect::WaitHeld { .. }))
        .count()
}

#[test]
fn only_a_direct_live_same_session_working_target_is_eligible() {
    for on in ["worker", "holdspec:worker"] {
        assert_eq!(
            lookup(&journal(Some(on), "working"), true).as_deref(),
            Some("worker")
        );
    }
    for state in [
        "done",
        "blocked",
        "waiting-agent",
        "waiting-user",
        "unknown",
    ] {
        assert_eq!(
            lookup(&journal(Some("worker"), state), true),
            None,
            "{state}"
        );
    }
    for on in [
        None,
        Some("elsewhere:worker"),
        Some("@elsewhere"),
        Some("@holdspec"),
        Some("waiter"),
        Some("absent"),
    ] {
        assert_eq!(lookup(&journal(on, "working"), true), None, "{on:?}");
    }
    assert_eq!(
        lookup(&journal(Some("worker"), "working"), false),
        None,
        "dead, retired, or absent seats are excluded from the live roster"
    );
    let mut chain = journal(Some("worker"), "waiting-agent");
    chain[0] = declaration(
        PROOF - 1,
        "worker",
        "spawned.1",
        "waiting-agent",
        Some("missing"),
    );
    assert_eq!(
        lookup(&chain, true),
        None,
        "a wait chain cannot inherit working"
    );
    chain.push(declaration(PROOF + 1, "waiter", "main", "working", None));
    assert_eq!(
        lookup(&chain, true),
        None,
        "the waiting seat must still be waiting-agent"
    );
}

#[test]
fn target_journal_motion_uses_own_routing_and_ignores_watchdog_and_inbound_records() {
    let mut events = journal(Some("worker"), "working");
    events.push(event(
        PROOF + 50,
        "worker",
        "spawned.1",
        "send",
        r#", "target":"waiter""#,
    ));
    events.push(event(
        PROOF + 90,
        "other",
        "spawned.2",
        "send",
        r#", "target":"worker","target_slot":"spawned.1","target_session":"holdspec""#,
    ));
    events.push(event(
        PROOF + 95,
        "watchdog",
        "monitor",
        "nudge",
        r#", "target":"worker","target_slot":"spawned.1","target_session":"holdspec""#,
    ));
    events.push(event(PROOF + 99, "worker", "spawned.9", "memo", ""));
    let motion = target_motion(
        &events,
        SESSION,
        "spawned.1",
        "worker",
        Some(PROOF + 40),
        false,
    );
    assert_eq!(motion.journal, Some(PROOF + 50));
    assert_eq!(motion.pane, Some(PROOF + 40));
    assert!(!motion.job);
    assert_eq!(motion.newest(PROOF + 100), Some(PROOF + 50));
    assert_eq!(
        target_motion(&events, "elsewhere", "spawned.1", "worker", None, false).journal,
        None
    );
}

#[test]
fn journal_pane_and_live_job_each_independently_hold_beyond_the_old_bound() {
    let now = PROOF + 1_200;
    for motion in [
        TargetMotion {
            journal: Some(now - 1),
            pane: None,
            job: false,
        },
        TargetMotion {
            journal: None,
            pane: Some(now - 1),
            job: false,
        },
        TargetMotion {
            journal: None,
            pane: None,
            job: true,
        },
    ] {
        assert!(target_held(due(1_200), Some(&target(motion)), now, CADENCE));
        let result = account_with_activity(
            &PaneState::default(),
            &observed(1_200, QuietKind::WaitingAgent),
            &knobs(),
            None,
            &activity(Some(motion), None),
        );
        assert!(!challenged(&result.effects), "{motion:?}");
        assert!(result.effects.contains(&Effect::WaitHeld {
            reason: HoldReason::Target,
            on: Some("worker".to_owned())
        }));
    }
}

#[test]
fn target_quiet_and_cap_boundaries_release_without_renewal_from_fresh_motion() {
    let now = PROOF + 1_200;
    for age in [899, 900, 901] {
        let moving = target(TargetMotion {
            journal: Some(now - age),
            ..TargetMotion::default()
        });
        assert_eq!(
            target_held(due(1_200), Some(&moving), now, CADENCE),
            age < 900,
            "motion age {age}"
        );
    }
    assert!(!target_held(
        due(1_200),
        Some(&target(TargetMotion::default())),
        now,
        CADENCE
    ));
    assert!(!target_held(due(1_200), None, now, CADENCE));
    assert_eq!(watchdog::wait_hold_cap(CADENCE), CAP_CADENCES * CADENCE);
    for age in [2_399, 2_400, 2_401] {
        let now = PROOF + age;
        let moving = target(TargetMotion {
            journal: Some(now),
            pane: Some(now),
            job: true,
        });
        assert_eq!(
            target_held(
                due(u64::try_from(age).expect("positive age")),
                Some(&moving),
                now,
                CADENCE
            ),
            age < 2_400,
            "proof age {age}"
        );
    }
    assert!(!target_held(
        due(300),
        Some(&target(TargetMotion {
            job: true,
            ..TargetMotion::default()
        })),
        now,
        0
    ));
    assert!(!target_held(
        WaitProgress::None,
        Some(&target(TargetMotion {
            job: true,
            ..TargetMotion::default()
        })),
        now,
        CADENCE
    ));
}

#[test]
fn target_change_quiet_and_cap_make_account_challenge_again() {
    for (age, motion) in [
        (1_200, None),
        (
            1_200,
            Some(TargetMotion {
                journal: Some(PROOF + 300),
                ..TargetMotion::default()
            }),
        ),
        (
            2_400,
            Some(TargetMotion {
                job: true,
                ..TargetMotion::default()
            }),
        ),
    ] {
        let result = account_with_activity(
            &PaneState::default(),
            &observed(age, QuietKind::WaitingAgent),
            &knobs(),
            None,
            &activity(motion, witness(HoldReason::Target)),
        );
        assert!(challenged(&result.effects), "age {age}, motion {motion:?}");
        assert_eq!(held(&result.effects), 0);
    }
    for kind in [QuietKind::Blocked, QuietKind::WaitingUser] {
        let result = account_with_activity(
            &PaneState::default(),
            &observed(1_200, kind),
            &knobs(),
            None,
            &activity(
                Some(TargetMotion {
                    job: true,
                    ..TargetMotion::default()
                }),
                None,
            ),
        );
        assert_eq!(
            held(&result.effects),
            0,
            "{kind:?} cannot gain a target hold"
        );
        if kind == QuietKind::Blocked {
            assert!(challenged(&result.effects));
        }
    }
}

#[test]
fn one_wait_held_effect_per_proof_episode_even_when_target_motion_continues() {
    let motion = TargetMotion {
        job: true,
        ..TargetMotion::default()
    };
    let first = account_with_activity(
        &PaneState::default(),
        &observed(1_200, QuietKind::WaitingAgent),
        &knobs(),
        None,
        &activity(Some(motion), None),
    );
    assert_eq!(held(&first.effects), 1);
    let second = account_with_activity(
        &first.next,
        &observed(1_201, QuietKind::WaitingAgent),
        &knobs(),
        None,
        &activity(Some(motion), witness(HoldReason::Target)),
    );
    assert!(!challenged(&second.effects));
    assert_eq!(held(&second.effects), 0);
    let restarted = account_with_activity(
        &PaneState::default(),
        &observed(1_202, QuietKind::WaitingAgent),
        &knobs(),
        None,
        &activity(Some(motion), witness(HoldReason::Target)),
    );
    assert_eq!(
        held(&restarted.effects),
        0,
        "journal witness survives loss of memory carry"
    );
}

fn reading(events: &[Event]) -> watchdog::WaitReading {
    watchdog::wait_reading(
        events,
        SESSION,
        "main",
        "waiter",
        Some("L1"),
        at(PROOF + 1_200),
        CADENCE,
        2,
        WaitState::WaitingAgent,
    )
}

fn held_event(seconds: i64, launch: &str) -> Event {
    let summary =
        watchdog::wait_held_summary(WaitState::WaitingAgent, HoldReason::Target, Some("worker"));
    event(
        seconds,
        "watchdog",
        "monitor",
        watchdog::WAIT_HELD_ACTION,
        &format!(
            r#", "target":"waiter","target_slot":"main","target_session":"holdspec","ref":"{launch}","summary":"{summary}""#
        ),
    )
}

#[test]
fn parsed_wait_held_is_a_durable_witness_but_never_a_proof_attempt_news_or_paint() {
    let mut events = journal(Some("worker"), "working");
    let before = reading(&events);
    assert_eq!(before.held, None);
    let hold = held_event(PROOF + 900, "L1");
    assert_eq!(hold.ref_meaning(), RefMeaning::LaunchId("L1"));
    events.push(hold);
    let after = reading(&events);
    assert_eq!(after.held, witness(HoldReason::Target));
    assert_eq!(after.progress, before.progress);
    assert_eq!(after.quiet, before.quiet);
    assert_eq!(after.rung, before.rung);
    assert_eq!(
        watchdog::last_watchdog_delivery(&events, SESSION, "main", "waiter"),
        None
    );
    assert_eq!(lookup(&events, true).as_deref(), Some("worker"));
    events.push(declaration(
        PROOF + 901,
        "waiter",
        "main",
        "waiting-agent",
        Some("worker"),
    ));
    assert_eq!(
        reading(&events).held,
        None,
        "a newer proof begins a new episode"
    );
}

#[test]
fn wrong_incarnation_or_an_old_wait_held_cannot_suppress_episode_telemetry() {
    let mut events = vec![held_event(PROOF - 2, "L1")];
    events.extend(journal(Some("worker"), "working"));
    assert_eq!(reading(&events).held, None);
    events.push(held_event(PROOF + 900, "L0"));
    assert_eq!(reading(&events).held, None, "previous incarnation");
}

#[test]
fn ordinary_unchanged_waits_become_measurable_and_keep_the_three_cadence_bound() {
    // Observe a real pre-proof cycle first. A daemon restart alone cannot
    // establish that a pane has remained unchanged since an old proof.
    let mut initial = observed(0, QuietKind::WaitingAgent);
    initial.wait_progress = WaitProgress::None;
    let mut carry = account_with_activity(
        &PaneState::default(),
        &initial,
        &knobs(),
        None,
        &Activity {
            proof: Some(ProofQuiet::default()),
            ..Activity::default()
        },
    )
    .next;
    let mut seen_witness = None;
    let mut count = 0;
    for age in [300, 301, 302, 303] {
        let result = account_with_activity(
            &carry,
            &observed(age, QuietKind::WaitingAgent),
            &knobs(),
            None,
            &activity(None, seen_witness),
        );
        assert!(!challenged(&result.effects), "ordinary quiet hold at {age}");
        count += held(&result.effects);
        if held(&result.effects) != 0 {
            seen_witness = witness(HoldReason::Unchanged);
        }
        if held(&result.effects) != 0 {
            assert!(result.effects.contains(&Effect::WaitHeld {
                reason: HoldReason::Unchanged,
                on: None
            }));
        }
        carry = result.next;
    }
    assert_eq!(count, 1);
    let due = account_with_activity(
        &carry,
        &observed(900, QuietKind::WaitingAgent),
        &knobs(),
        None,
        &activity(None, seen_witness),
    );
    assert!(challenged(&due.effects), "old hold ends at three cadences");
}

#[test]
fn another_seats_motion_does_not_defer_an_ordinary_idle_nudge() {
    let mut initial = observed(0, QuietKind::WaitingAgent);
    initial.quiet = None;
    initial.wait_progress = WaitProgress::None;
    let prior = account_with_activity(
        &PaneState::default(),
        &initial,
        &knobs(),
        None,
        &Activity::default(),
    )
    .next;
    let mut idle = observed(300, QuietKind::WaitingAgent);
    idle.quiet = None;
    idle.wait_progress = WaitProgress::None;
    let baseline = account_with_activity(&prior, &idle, &knobs(), None, &Activity::default());
    let unrelated = account_with_activity(
        &prior,
        &idle,
        &knobs(),
        None,
        &Activity {
            wait_target: Some(target(TargetMotion {
                job: true,
                ..TargetMotion::default()
            })),
            ..Activity::default()
        },
    );
    assert!(
        baseline.effects.contains(&Effect::Nudge),
        "control must reach idle nudge"
    );
    assert_eq!(unrelated.effects, baseline.effects);
    assert_eq!(unrelated.verdict, baseline.verdict);
}

#[test]
fn typed_hold_reader_accepts_only_the_ruled_grammar() {
    for (state, reason, on) in [
        (WaitState::WaitingAgent, HoldReason::Unchanged, None),
        (WaitState::Blocked, HoldReason::Unchanged, None),
        (WaitState::WaitingAgent, HoldReason::Target, Some("worker")),
    ] {
        let summary = watchdog::wait_held_summary(state, reason, on);
        let row = event(
            PROOF + 300,
            "watchdog",
            "monitor",
            "wait-held",
            &format!(r#", "summary":"{summary}""#),
        );
        assert_eq!(row.wait_held(), Some((state, reason, on)));
    }
    for summary in [
        "waiting-agent held: target",
        "blocked held: target on worker",
        "waiting-agent held: target on @other",
        "waiting-agent held: target on other:worker",
        "waiting-agent held: target on worker extra",
        "waiting-agent held: unknown",
        "waiting-agent held: unchanged on worker",
        "done held: unchanged",
    ] {
        let row = event(
            PROOF + 300,
            "watchdog",
            "monitor",
            "wait-held",
            &format!(r#", "summary":"{summary}""#),
        );
        assert_eq!(row.wait_held(), None, "{summary}");
    }
    let mut fake = held_event(PROOF + 900, "L1");
    fake.actor = "worker".to_owned();
    assert_eq!(fake.wait_held(), None);
    fake.actor = "watchdog".to_owned();
    fake.action = "send".to_owned();
    assert_eq!(fake.wait_held(), None);
}

#[test]
fn an_unchanged_hold_can_upgrade_once_to_target_and_never_records_a_downgrade() {
    let motion = TargetMotion {
        job: true,
        ..TargetMotion::default()
    };
    let mut initial = observed(0, QuietKind::WaitingAgent);
    initial.wait_progress = WaitProgress::None;
    let carry = account_with_activity(
        &PaneState::default(),
        &initial,
        &knobs(),
        None,
        &Activity {
            proof: Some(ProofQuiet::default()),
            ..Activity::default()
        },
    )
    .next;
    let ordinary = account_with_activity(
        &carry,
        &observed(300, QuietKind::WaitingAgent),
        &knobs(),
        None,
        &activity(None, None),
    );
    assert!(ordinary.effects.contains(&Effect::WaitHeld {
        reason: HoldReason::Unchanged,
        on: None
    }));
    let upgraded = account_with_activity(
        &ordinary.next,
        &observed(301, QuietKind::WaitingAgent),
        &knobs(),
        None,
        &activity(Some(motion), witness(HoldReason::Unchanged)),
    );
    assert!(upgraded.effects.contains(&Effect::WaitHeld {
        reason: HoldReason::Target,
        on: Some("worker".to_owned())
    }));
    let again = account_with_activity(
        &upgraded.next,
        &observed(302, QuietKind::WaitingAgent),
        &knobs(),
        None,
        &activity(Some(motion), witness(HoldReason::Target)),
    );
    assert_eq!(held(&again.effects), 0);
    let downgrade = account_with_activity(
        &again.next,
        &observed(303, QuietKind::WaitingAgent),
        &knobs(),
        None,
        &activity(None, witness(HoldReason::Target)),
    );
    assert_eq!(held(&downgrade.effects), 0);
    assert!(
        challenged(&downgrade.effects),
        "target release must journal a challenge even inside the ordinary hold bound"
    );
}

#[test]
fn a_challenge_state_retire_or_cap_closes_the_open_hold_episode() {
    for ending in [
        declaration(
            PROOF + 901,
            "waiter",
            "main",
            "waiting-agent",
            Some("worker"),
        ),
        event(
            PROOF + 901,
            "lead",
            "worker.1",
            "retire",
            r#", "target":"waiter","target_slot":"main","target_session":"holdspec""#,
        ),
    ] {
        let mut events = journal(Some("worker"), "working");
        events.push(held_event(PROOF + 900, "L1"));
        assert_eq!(reading(&events).held, witness(HoldReason::Target));
        events.push(ending);
        assert_eq!(reading(&events).held, None, "ending {:?}", events.last());
    }
    for summary in [
        "waiting-agent confirmation 1/2",
        "[unconfirmed] waiting-agent confirmation 1/2; refused pre-paste: dead pane",
    ] {
        let mut events = journal(Some("worker"), "working");
        events.push(working_superseded());
        events.push(held_event(PROOF + 900, "L1"));
        events.push(event(PROOF + 901, "watchdog", "monitor", "wait-challenge", &format!(r#", "target":"waiter","target_slot":"main","target_session":"holdspec","ref":"L1","summary":"{summary}""#)));
        assert_eq!(
            reading(&events).held,
            Some(HoldWitness {
                reason: HoldReason::Target,
                open: false
            })
        );
        assert_eq!(
            list_reason(&events, 1_200),
            Some(Reason::Blocked),
            "challenge closes promotion: {summary}"
        );
    }
    let mut events = journal(Some("worker"), "working");
    events.push(held_event(PROOF + 900, "L1"));
    let expired = watchdog::wait_reading(
        &events,
        SESSION,
        "main",
        "waiter",
        Some("L1"),
        at(PROOF + 2_400),
        CADENCE,
        2,
        WaitState::WaitingAgent,
    );
    assert_eq!(
        watchdog::promoted_rung(expired.progress, expired.rung, expired.held, true, CADENCE),
        expired.rung,
        "the cap expires promotion even without a closing challenge"
    );
}

#[test]
fn a_closed_target_hold_never_reopens_and_spent_attempts_never_start_a_hold() {
    let moving = Some(TargetMotion {
        job: true,
        ..TargetMotion::default()
    });
    let closed = Some(HoldWitness {
        reason: HoldReason::Target,
        open: false,
    });
    let result = account_with_activity(
        &PaneState::default(),
        &observed(1_200, QuietKind::WaitingAgent),
        &knobs(),
        None,
        &activity(moving, closed),
    );
    assert!(challenged(&result.effects));
    assert_eq!(held(&result.effects), 0);
    let mut spent = observed(1_200, QuietKind::WaitingAgent);
    spent.wait_progress = WaitProgress::ChallengeDue {
        confirmations: 0,
        required: 2,
        wait_age_secs: 1_200,
        attempts: 3,
    };
    let result = account_with_activity(
        &PaneState::default(),
        &spent,
        &knobs(),
        None,
        &activity(moving, None),
    );
    assert_eq!(
        held(&result.effects),
        0,
        "an exhausted delivery budget must not open an uncloseable promotion"
    );
}

fn list_reason(events: &[Event], age: i64) -> Option<Reason> {
    list_waiter(events, PROOF + age, CADENCE, true).reason
}

fn list_waiter(
    events: &[Event],
    now: i64,
    cadence: u64,
    worker_alive: bool,
) -> ae::digest::AgentEntry {
    let snapshot = RecordSnapshot {
        meta: Some(Meta::parse(&format!(
            "schema=2\nmode=local\nsession=holdspec\nseat.main=waiter\nseat.spawned.1=worker\nseat.spawned.2=relay\nprofile.main=spec\nprofile.spawned.1=spec\nprofile.spawned.2=spec\nlaunch_id.main=L1\nlaunch_id.spawned.1=W1\nlaunch_id.spawned.2=R1\nidle_nudge_secs={cadence}\ndone_confirmations=2\n",
        ))),
        events: Some(SessionRead::from_drain(&ae::events::Drain {
            events: events.to_vec(),
            cursor: ae::events::Cursor::default(),
            skipped: Vec::new(),
            drained: true,
        })),
        ..RecordSnapshot::default()
    };
    let runtime = SessionRuntime {
        agents: ["main", "spawned.1", "spawned.2"]
            .into_iter()
            .map(|slot| AgentRuntime {
                pane_id: None,
                slot: slot.to_owned(),
                alive: Some(slot != "spawned.1" || worker_alive),
                alert: None,
                observed: HarnessState::Idle,
            })
            .collect(),
        ..SessionRuntime::new(Status::Running)
    };
    ae::session::entry_from(&snapshot, SESSION, &runtime, at(now), 3600)
        .agents
        .into_iter()
        .find(|agent| agent.name == "waiter")
        .expect("waiting list entry")
}

#[test]
fn list_requires_the_daemon_witness_and_agrees_with_account_on_a_fresh_target_hold() {
    let mut events = journal(Some("worker"), "working");
    events.push(working_superseded());
    assert_eq!(
        list_reason(&events, 1_200),
        Some(Reason::Blocked),
        "without current working, promotion requires the daemon witness"
    );
    events.push(held_event(PROOF + 900, "L1"));
    assert_eq!(list_reason(&events, 1_200), None);
    let result = account_with_activity(
        &PaneState::default(),
        &observed(1_200, QuietKind::WaitingAgent),
        &knobs(),
        None,
        &activity(
            Some(TargetMotion {
                job: true,
                ..TargetMotion::default()
            }),
            witness(HoldReason::Target),
        ),
    );
    assert_eq!(
        result.verdict,
        ae::watchdog_daemon::Verdict::Quiet(QuietKind::WaitingAgent)
    );
    assert!(!challenged(&result.effects));
    assert_eq!(list_reason(&events, 2_399), None);
    assert_eq!(
        list_reason(&events, 2_400),
        Some(Reason::Blocked),
        "an expired witness cannot extend list suppression"
    );
    for state in ["done", "blocked", "waiting-agent", "waiting-user"] {
        let mut changed = events.clone();
        changed.push(declaration(
            PROOF + 901,
            "worker",
            "spawned.1",
            state,
            Some("waiter"),
        ));
        assert_eq!(
            list_reason(&changed, 1_200),
            Some(Reason::Blocked),
            "target {state} drops promotion"
        );
    }
    for rung in 0..=3 {
        assert_eq!(
            watchdog::promoted_rung(due(1_200), rung, None, true, CADENCE),
            rung
        );
        assert_eq!(
            watchdog::promoted_rung(
                due(1_200),
                rung,
                witness(HoldReason::Unchanged),
                true,
                CADENCE
            ),
            rung
        );
        assert_eq!(
            watchdog::promoted_rung(due(1_200), rung, witness(HoldReason::Target), true, CADENCE),
            3
        );
        assert_eq!(
            watchdog::promoted_rung(
                due(1_200),
                rung,
                witness(HoldReason::Target),
                false,
                CADENCE
            ),
            rung
        );
        assert_eq!(
            watchdog::promoted_rung(due(2_400), rung, witness(HoldReason::Target), true, CADENCE),
            rung
        );
    }
}

#[test]
fn local_working_target_ceiling_uses_awake_seconds_and_preserves_large_bases() {
    for (target, cadence, boundary, sleep) in [
        ("worker", 300, 3_600, 0),
        ("holdspec:worker", 300, 3_600, 7_200),
        ("worker", 0, 3_600, 0),
        ("worker", 900, 7_200, 0),
        ("worker", 1_800, 7_200, 0),
        ("worker", 3_600, 14_400, 0),
    ] {
        let mut events = journal(Some(target), "working");
        if sleep != 0 {
            let summary = watchdog::suspended_summary(watchdog::Suspended {
                secs: u64::try_from(sleep).expect("nonnegative fixture sleep"),
                wall_secs: 7_260,
                mono_secs: 60,
            });
            events.push(event(
                PROOF + 100 + sleep,
                "watchdog",
                "monitor",
                watchdog::HOST_SUSPENDED_ACTION,
                &format!(r#", "summary":"{summary}""#),
            ));
        }
        for (age, reason) in [(boundary - 1, None), (boundary, Some(Reason::Blocked))] {
            let waiter = list_waiter(&events, PROOF + sleep + age, cadence, true);
            assert_eq!(waiter.state.as_deref(), Some("waiting-agent"));
            assert_eq!(
                waiter.reason, reason,
                "{target}, {cadence}s cadence, {age}s awake"
            );
        }
    }
}

#[test]
fn local_working_target_ceiling_uses_the_credited_rung() {
    let mut events = journal(Some("worker"), "working");
    for (rung, challenged_at, confirmation, boundary) in [
        (1, PROOF + 300, 1, 3_600),
        (2, PROOF + 910, 2, 4_500),
        (3, PROOF + 2_120, 1, 7_200),
    ] {
        events.push(event(
            challenged_at,
            "watchdog",
            "monitor",
            "wait-challenge",
            &format!(
                r#", "target":"waiter","target_slot":"main","target_session":"holdspec","ref":"L1","summary":"waiting-agent confirmation {confirmation}/2""#
            ),
        ));
        let proof = challenged_at + 10;
        events.push(declaration(
            proof,
            "waiter",
            "main",
            "waiting-agent",
            Some("worker"),
        ));
        let reading = watchdog::wait_reading(
            &events,
            SESSION,
            "main",
            "waiter",
            Some("L1"),
            at(proof + 1),
            CADENCE,
            2,
            WaitState::WaitingAgent,
        );
        assert_eq!(reading.rung, rung, "fixture's credited rung");
        assert_eq!(
            list_waiter(&events, proof + boundary - 1, CADENCE, true).reason,
            None,
            "before the extended rung-{rung} ceiling"
        );
        assert_eq!(
            list_waiter(&events, proof + boundary, CADENCE, true).reason,
            Some(Reason::Blocked),
            "at the extended rung-{rung} ceiling"
        );
    }
}

#[test]
fn only_a_current_working_direct_local_target_extends_the_wait() {
    for (on, target_state, superseded, worker_alive) in [
        (None, "working", false, true),
        (Some("@holdspec"), "working", false, true),
        (Some("waiter"), "working", false, true),
        (Some("missing"), "working", false, true),
        (Some("worker"), "waiting-agent", false, true),
        (Some("worker"), "waiting-user", false, true),
        (Some("worker"), "blocked", false, true),
        (Some("worker"), "done", false, true),
        (Some("worker"), "working", true, true),
        (Some("worker"), "working", false, false),
    ] {
        let mut events = journal(on, target_state);
        if superseded {
            events.push(working_superseded());
        }
        for cadence in [0, CADENCE] {
            assert_eq!(
                list_waiter(&events, PROOF + 1_199, cadence, worker_alive).reason,
                None
            );
            assert_eq!(
                list_waiter(&events, PROOF + 1_200, cadence, worker_alive).reason,
                Some(Reason::Blocked),
                "{on:?}, {cadence}s cadence, target state {target_state}, superseded {superseded}, alive {worker_alive}"
            );
        }
    }
    for on in ["@elsewhere", "elsewhere:worker"] {
        let events = journal(Some(on), "working");
        assert_eq!(
            list_reason(&events, 7_200),
            None,
            "{on} keeps the external exemption"
        );
    }
}

fn working_superseded() -> Event {
    event(
        PROOF + 1,
        "worker",
        "spawned.1",
        "memo",
        r#", "ref":"progress","summary":"newer than the declaration""#,
    )
}

#[test]
fn a_working_end_of_a_chain_does_not_extend_the_direct_wait() {
    for (target, age, reason) in [
        ("worker", 1_200, Some(Reason::Blocked)),
        ("@elsewhere", 7_200, None),
    ] {
        let events = [
            declaration(PROOF - 2, "worker", "spawned.1", "working", None),
            declaration(
                PROOF - 1,
                "relay",
                "spawned.2",
                "waiting-agent",
                Some(target),
            ),
            declaration(PROOF, "waiter", "main", "waiting-agent", Some("relay")),
        ];
        assert_eq!(list_reason(&events, age), reason, "relay waits on {target}");
    }
}

#[test]
fn the_working_declaration_must_match_the_target_slot_and_session() {
    let wrong_slot = declaration(PROOF - 1, "worker", "spawned.9", "working", None);
    let mut wrong_session = declaration(PROOF - 1, "worker", "spawned.1", "working", None);
    wrong_session.actor_session = ae::events::RoutingMember::Value("elsewhere".to_owned());
    for working in [wrong_slot, wrong_session] {
        let events = [
            working,
            declaration(PROOF, "waiter", "main", "waiting-agent", Some("worker")),
        ];
        assert_eq!(
            list_reason(&events, 1_200),
            Some(Reason::Blocked),
            "the target has no routed working declaration"
        );
    }
}

#[test]
fn inbound_news_ends_working_currency_and_a_watchdog_nudge_keeps_it() {
    for (actor, slot, action, reason) in [
        ("relay", "spawned.2", "send", Some(Reason::Blocked)),
        ("watchdog", "monitor", "nudge", None),
    ] {
        let mut events = journal(Some("worker"), "working");
        events.push(event(
            PROOF + 1,
            actor,
            slot,
            action,
            r#", "target":"worker","target_slot":"spawned.1","target_session":"holdspec","summary":"status check""#,
        ));
        assert_eq!(list_reason(&events, 1_200), reason, "inbound {action}");
    }
}
