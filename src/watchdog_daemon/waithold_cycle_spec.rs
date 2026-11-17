//! Driver-owned #225 wiring proof through the REAL cycle and a private tmux
//! socket (`transport::quietwd_spec::Rig`): the daemon reads the waited-on
//! seat's motion and journals the target hold itself. The frozen acceptance
//! (`tests/it/waithold_spec.rs`) covers the pure seams; this covers the
//! observation and writer links it cannot reach.

use super::{Carry, Cycle, Knobs, SeatLaunch, SendHelper, read_events};
use crate::time::Timestamp;

/// Waits until `pane` runs `sleep` itself, the command a held seat shows.
fn settle(rig: &crate::transport::quietwd_spec::Rig, pane: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let command = rig.must(&[
            "display-message",
            "-p",
            "-t",
            pane,
            "#{pane_current_command}",
        ]);
        if command.trim() == "sleep" {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "pane {pane} never settled on sleep: {command}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// One private session: `lead` (slot `main`) waits on `worker` (slot
/// `spawned.1`), both panes held by a live `sleep`.
struct Seats {
    rig: crate::transport::quietwd_spec::Rig,
    meta: std::path::PathBuf,
    /// The waited-on seat's pane.
    worker: String,
}

impl Seats {
    fn new(tag: &str) -> Self {
        let rig = crate::transport::quietwd_spec::Rig::new(tag);
        // tmux starts a pane command through the default shell, which need
        // not exec it (fish stays the pane's command), and a pane whose
        // command is a shell holds no seat (`held_seats`). Both seats
        // therefore exec `sleep` and settle on it before any cycle runs.
        rig.must(&["respawn-pane", "-k", "-t", &rig.pane, "exec sleep 600"]);
        let worker = rig.must(&[
            "split-window",
            "-d",
            "-t",
            &rig.pane,
            "-P",
            "-F",
            "#{pane_id}",
            "exec sleep 600",
        ]);
        let worker = worker.trim().to_owned();
        for (pane, agent, slot) in [
            (rig.pane.as_str(), "lead", "main"),
            (worker.as_str(), "worker", "spawned.1"),
        ] {
            rig.must(&["set-option", "-p", "-t", pane, "@ae_agent", agent]);
            rig.must(&["set-option", "-p", "-t", pane, "@ae_slot", slot]);
            settle(&rig, pane);
        }
        // The precondition both cases need: the cycle's enumeration holds
        // both seats, so the wait link to `worker` exists. A loaded host once
        // showed one settled pane missing from a single enumeration, so the
        // fixture waits on the cycle's own predicate and names every miss.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut misses = Vec::new();
        loop {
            let panes = crate::transport::observe_watch_panes(&rig.server, "current")
                .expect("private panes");
            if super::held_seats(&panes, None, &|_| None) == ["lead", "worker"] {
                break;
            }
            misses.push(format!("{panes:?}"));
            assert!(
                std::time::Instant::now() < deadline,
                "both seats never held: {misses:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if !misses.is_empty() {
            eprintln!("waithold-settle-miss {}: {misses:?}", misses.len());
        }
        let meta = rig.path("meta");
        std::fs::create_dir(&meta).expect("owned journal directory");
        Self { rig, meta, worker }
    }

    fn append(&self, line: &str) {
        assert!(
            crate::events::Event::parse_line(line).is_ok(),
            "fixture line: {line}"
        );
        crate::store::open(&self.meta)
            .append_event(&format!("{line}\n"))
            .expect("owned journal append");
    }

    fn record(&self, at: i64, actor: &str, slot: &str, rest: &str) {
        self.append(&format!(
            r#"{{"ts":"{}","actor":"{actor}","actor_slot":"{slot}","actor_session":"current",{rest}}}"#,
            Timestamp::from_epoch(at)
        ));
    }

    /// The worker declared `working` and the lead `waiting-agent --on worker`
    /// twenty minutes ago: a due wait past the unchanged hold's bound.
    fn waiting(&self, now: i64) {
        self.record(
            now - 1_200,
            "worker",
            "spawned.1",
            r#""action":"state","ref":"working","summary":"building""#,
        );
        self.record(
            now - 1_200,
            "lead",
            "main",
            r#""action":"state","ref":"waiting-agent","summary":"waits on worker","wait_on":"worker""#,
        );
    }

    /// One real cycle. The launch row of `main` is unproven on purpose: a
    /// delivery to the lead is then refused before anything runs and leaves
    /// a `refused` record, the journal trace of an attempted challenge.
    fn cycle(&self, carry: &mut Carry) -> String {
        let helper = SendHelper::for_session(&self.meta);
        let cycle = Cycle {
            knobs: Knobs {
                quota_aware: false,
                ..Knobs::default()
            },
            meta_dir: &self.meta,
            helper: &helper,
            server: &self.rig.server,
            session: "current",
            goal: None,
            roster: Vec::new(),
            local_config: None,
            lead_pair: false,
            fleet_order: crate::theme::FleetOrder::EMPTY,
            auto: crate::autoreseat::settings(None),
            auto_trust: crate::config::AutoTrust::Off,
            meta_agent: false,
            launch_ids: vec![("main".to_owned(), SeatLaunch::Unproven("fixture"))],
            started: None,
            settled: None,
        };
        let mut err = Vec::new();
        cycle.run(carry, &mut err).expect("private real cycle");
        String::from_utf8_lossy(&err).into_owned()
    }

    fn held(&self) -> Vec<String> {
        read_events(&self.meta)
            .into_iter()
            .filter(|event| event.action == crate::watchdog::WAIT_HELD_ACTION)
            .map(|event| {
                format!(
                    "{} {}",
                    event.target.unwrap_or_default(),
                    event.summary.unwrap_or_default()
                )
            })
            .collect()
    }

    fn challenged(&self) -> usize {
        read_events(&self.meta)
            .iter()
            .filter(|event| {
                event.action == "refused"
                    && event
                        .summary
                        .as_deref()
                        .is_some_and(|summary| summary.contains("seat lead (main)"))
            })
            .count()
    }
}

#[test]
fn a_real_cycle_holds_a_wait_on_a_moving_working_target_with_one_record() {
    let seats = Seats::new("waithold-moving");
    let now = Timestamp::now().epoch();
    seats.waiting(now);
    // The worker's own fresh record: the journal half of its motion.
    seats.record(
        now - 10,
        "worker",
        "spawned.1",
        r#""action":"memo","ref":"progress","summary":"half done""#,
    );
    let mut carry = Carry::new();
    for pass in 0..2 {
        let err = seats.cycle(&mut carry);
        assert_eq!(
            seats.held(),
            ["lead waiting-agent held: target on worker"],
            "pass {pass}: {err}"
        );
        assert_eq!(seats.challenged(), 0, "pass {pass}: {err}");
    }
}

#[test]
fn a_real_cycle_challenges_a_wait_on_a_quiet_working_target_without_a_hold() {
    let seats = Seats::new("waithold-quiet");
    let now = Timestamp::now().epoch();
    // Working, but no own record, pane motion or job since twenty minutes.
    seats.waiting(now);
    let mut carry = Carry::new();
    let err = seats.cycle(&mut carry);
    assert_eq!(seats.held(), Vec::<String>::new(), "{err}");
    assert_eq!(seats.challenged(), 1, "{err}");
}

/// The worker's carried pane state: a frame that moved five seconds ago.
fn moved_frame(identity: u64, now: i64) -> super::PaneState {
    super::PaneState {
        identity: Some(identity),
        last_hash_change: Some(now - 5),
        ..super::PaneState::default()
    }
}

#[test]
fn a_real_cycle_holds_on_the_carried_pane_motion_of_the_same_incarnation() {
    let seats = Seats::new("waithold-pane");
    let now = Timestamp::now().epoch();
    // No fresh record and no job: the pane half alone moves.
    seats.waiting(now);
    let mut carry = Carry::new();
    let identity = crate::watchdog::quiet_hash("spawned.1\nworker");
    carry
        .panes
        .push((seats.worker.clone(), moved_frame(identity, now)));
    let err = seats.cycle(&mut carry);
    assert_eq!(
        seats.held(),
        ["lead waiting-agent held: target on worker"],
        "{err}"
    );
    assert_eq!(seats.challenged(), 0, "{err}");
}

#[test]
fn carried_pane_motion_of_another_pane_or_incarnation_never_holds() {
    let now = Timestamp::now().epoch();
    let own = crate::watchdog::quiet_hash("spawned.1\nworker");
    let older = crate::watchdog::quiet_hash("spawned.1\nolder");
    // The same pane under another seat identity, and the worker's identity
    // carried for another pane: neither is this target's motion.
    for (case, other_pane, identity) in [("incarnation", false, older), ("pane", true, own)] {
        let seats = Seats::new(&format!("waithold-{case}"));
        seats.waiting(now);
        let pane = if other_pane {
            "%99".to_owned()
        } else {
            seats.worker.clone()
        };
        let mut carry = Carry::new();
        carry.panes.push((pane, moved_frame(identity, now)));
        let err = seats.cycle(&mut carry);
        assert_eq!(seats.held(), Vec::<String>::new(), "{case}: {err}");
        assert_eq!(seats.challenged(), 1, "{case}: {err}");
    }
}

#[test]
fn inbound_news_keeps_the_open_target_witness() {
    use crate::events::Event;
    let line = |at: i64, rest: &str| {
        Event::parse_line(&format!(
            r#"{{"ts":"{}",{rest}}}"#,
            Timestamp::from_epoch(at)
        ))
        .expect("fixture line")
    };
    let proof = 10_000;
    let summary = crate::watchdog::wait_held_summary(
        crate::watchdog::WaitState::WaitingAgent,
        crate::watchdog::HoldReason::Target,
        Some("worker"),
    );
    let mut events = vec![
        line(
            proof,
            r#""actor":"lead","actor_slot":"main","actor_session":"current","action":"state","ref":"waiting-agent","wait_on":"worker""#,
        ),
        line(
            proof + 900,
            &format!(
                r#""actor":"watchdog","actor_slot":"monitor","actor_session":"current","action":"wait-held","target":"lead","target_slot":"main","target_session":"current","summary":"{summary}""#
            ),
        ),
    ];
    let held = |events: &[Event]| {
        crate::watchdog::wait_reading(
            events,
            "current",
            "main",
            "lead",
            None,
            Timestamp::from_epoch(proof + 1_200),
            300,
            2,
            crate::watchdog::WaitState::WaitingAgent,
        )
        .held
    };
    let open = Some(crate::watchdog::HoldWitness {
        reason: crate::watchdog::HoldReason::Target,
        open: true,
    });
    assert_eq!(held(&events), open);
    // A peer's ask is news the wait outlives, not the end of the episode: the
    // witness stays, so the daemon owes no second record.
    events.push(line(
        proof + 950,
        r#""actor":"peer","actor_slot":"spawned.2","actor_session":"current","action":"ask","target":"lead","target_slot":"main","target_session":"current","ref":"ae-1","summary":"status?""#,
    ));
    assert_eq!(held(&events), open);
}

/// The proof epoch of the pure accounting cases below.
const PROOF: i64 = 10_000;

/// A quiet `waiting-agent` seat whose wait is due at `age`, declared then.
fn due_wait(age: u64) -> super::Observation {
    use crate::watchdog::{QuietKind, WaitProgress};
    super::Observation {
        now_epoch: PROOF + i64::try_from(age).unwrap_or(i64::MAX),
        hash: 7,
        harness: super::HarnessObservation {
            frame: crate::harness_state::HarnessState::Idle,
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
        quiet: Some(QuietKind::WaitingAgent),
        ended_wait: None,
        done_progress: crate::watchdog::DoneProgress::None,
        wait_progress: WaitProgress::ChallengeDue {
            confirmations: 0,
            required: 2,
            wait_age_secs: age,
            attempts: 0,
        },
        wait_rung: 0,
        wait_ends_elsewhere: false,
        target_moved: None,
        descendancy: crate::procs::Descendancy::Present,
        launch_attempt: crate::tmux::Evidence::Silent,
        outstanding_alert: None,
        unreachable: false,
        failed_elsewhere: false,
        prompt_named: false,
        start: None,
        last_actor_event_age_secs: age,
        declared_age_secs: age,
        suspended: crate::watchdog::Suspensions::default(),
        sweep: None,
        own_work: crate::session::OwnWork::default(),
        auto_in_flight: None,
        auto_deadline: None,
    }
}

/// The proof reading of [`PROOF`] with a target whose newest motion is at
/// `moved`, and the episode's `witness`.
fn target_activity(moved: i64, witness: Option<crate::watchdog::HoldWitness>) -> super::Activity {
    super::Activity {
        proof: Some(crate::watchdog::ProofQuiet {
            proof: Some(Timestamp::from_epoch(PROOF)),
            unchanged: true,
            settle: Some(Timestamp::from_epoch(PROOF)),
        }),
        wait_target: Some(super::WaitTarget {
            agent: "worker".to_owned(),
            motion: super::TargetMotion {
                journal: Some(moved),
                ..super::TargetMotion::default()
            },
        }),
        wait_witness: witness,
        ..super::Activity::default()
    }
}

fn cadence_knobs() -> Knobs {
    Knobs {
        idle_nudge_secs: 300,
        done_confirmations: 2,
        ..Knobs::default()
    }
}

#[test]
fn the_cycle_that_owes_the_target_record_already_reads_the_top_rung() {
    use crate::watchdog::{HoldReason, QuietKind};
    // Past the own rung's ceiling (four cadences) and not yet witnessed: the
    // record this cycle writes is what promotes, so the seat does not read
    // `blocked` in the cycle that starts its hold.
    let seen = due_wait(1_200);
    let activity = target_activity(seen.now_epoch - 10, None);
    let result = super::account_with_activity(
        &super::PaneState::default(),
        &seen,
        &cadence_knobs(),
        None,
        &activity,
    );
    assert!(result.effects.contains(&super::Effect::WaitHeld {
        reason: HoldReason::Target,
        on: Some("worker".to_owned()),
    }));
    assert_eq!(
        result.verdict,
        super::Verdict::Quiet(QuietKind::WaitingAgent)
    );
}

#[test]
fn a_released_hold_past_the_own_ceiling_reads_blocked_with_an_escalated_challenge() {
    use crate::watchdog::{HoldReason, HoldWitness, QuietKind, WaitState};
    // The target went quiet (its newest motion is older than three
    // cadences): the open hold ends, the own rung is back, and the wait is
    // past its ceiling, so the challenge is escalated and the seat reads
    // `blocked` in the same cycle.
    let seen = due_wait(1_200);
    let witness = Some(HoldWitness {
        reason: HoldReason::Target,
        open: true,
    });
    let activity = target_activity(seen.now_epoch - 900, witness);
    let result = super::account_with_activity(
        &super::PaneState::default(),
        &seen,
        &cadence_knobs(),
        None,
        &activity,
    );
    assert!(result.effects.contains(&super::Effect::WaitChallenge {
        confirmations: 0,
        required: 2,
        wait_age_secs: 1_200,
        state: WaitState::WaitingAgent,
        escalated: true,
    }));
    assert_eq!(result.verdict, super::Verdict::Quiet(QuietKind::Blocked));
}

#[test]
fn held_on_reads_only_the_queried_seats_own_wait() {
    use crate::events::Event;
    let line = |actor: &str, slot: &str, state: &str, on: &str| {
        Event::parse_line(&format!(
            r#"{{"ts":"{}","actor":"{actor}","actor_slot":"{slot}","actor_session":"current","action":"state","ref":"{state}"{on}}}"#,
            Timestamp::from_epoch(PROOF)
        ))
        .expect("fixture line")
    };
    // `other` waits on the working `worker`; `lead` is working itself.
    let events = [
        line(
            "other",
            "spawned.2",
            "waiting-agent",
            r#","wait_on":"worker""#,
        ),
        line("lead", "main", "working", ""),
        line("worker", "spawned.1", "working", ""),
    ];
    let seats = [
        ("spawned.2", "other", "other"),
        ("main", "lead", "lead"),
        ("spawned.1", "worker", "worker"),
    ];
    let facts = crate::wait_on::Facts::read(&events, "current", &seats);
    let links = crate::wait_on::Facts::links(&facts);
    assert_eq!(crate::wait_on::held_on(&links, "other"), Some("worker"));
    assert_eq!(crate::wait_on::held_on(&links, "lead"), None);
}

#[test]
fn a_wait_held_record_from_another_actor_is_ordinary_news() {
    use crate::events::Event;
    let summary = crate::watchdog::wait_held_summary(
        crate::watchdog::WaitState::WaitingAgent,
        crate::watchdog::HoldReason::Target,
        Some("worker"),
    );
    let line = |at: i64, rest: &str| {
        Event::parse_line(&format!(
            r#"{{"ts":"{}",{rest}}}"#,
            Timestamp::from_epoch(at)
        ))
        .expect("fixture line")
    };
    let events = [
        line(
            PROOF,
            r#""actor":"lead","actor_slot":"main","actor_session":"current","action":"state","ref":"waiting-agent","wait_on":"worker""#,
        ),
        // A peer writes the watchdog's action name to the waiter: not the
        // watchdog's telemetry, so it is news like any unknown record.
        line(
            PROOF + 900,
            &format!(
                r#""actor":"peer","actor_slot":"spawned.2","actor_session":"current","action":"wait-held","target":"lead","target_slot":"main","target_session":"current","summary":"{summary}""#
            ),
        ),
    ];
    let reading = crate::watchdog::wait_reading(
        &events,
        "current",
        "main",
        "lead",
        None,
        Timestamp::from_epoch(PROOF + 1_200),
        300,
        2,
        crate::watchdog::WaitState::WaitingAgent,
    );
    assert_eq!(reading.held, None);
    assert!(!reading.quiet.unchanged, "news after the proof");
}

#[test]
fn the_wake_settle_holds_wait_held_but_never_a_journal_only_effect() {
    use super::Effect;
    assert!(
        Effect::WaitHeld {
            reason: crate::watchdog::HoldReason::Target,
            on: Some("worker".to_owned()),
        }
        .held_while_settling()
    );
    // Effects that never reach the seat still apply in the settle.
    for kept in [
        Effect::Emit {
            action: "alert",
            summary: "kept".to_owned(),
        },
        Effect::Notify("kept".to_owned()),
        Effect::Note("kept".to_owned()),
        Effect::QuotaRefresh,
        Effect::ReconcileWedge,
    ] {
        assert!(!kept.held_while_settling(), "{kept:?}");
    }
}

#[test]
fn a_real_cycle_journals_a_blocked_unchanged_hold_under_its_own_state() {
    let seats = Seats::new("waithold-blocked");
    let now = Timestamp::now().epoch();
    // Blocked four hundred seconds ago: the challenge is due and the proof
    // is still inside the unchanged hold's three cadences.
    seats.record(
        now - 400,
        "lead",
        "main",
        r#""action":"state","ref":"blocked","summary":"needs a ruling""#,
    );
    // The lead pane was already observed without a proof, so this proof's
    // frame starts unmoved and its first settled capture is its baseline.
    let mut carry = Carry::new();
    carry.panes.push((
        seats.rig.pane.clone(),
        super::PaneState {
            identity: Some(crate::watchdog::quiet_hash("main\nlead")),
            proof_frame: Some(super::ProofFrame::default()),
            ..super::PaneState::default()
        },
    ));
    let err = seats.cycle(&mut carry);
    assert_eq!(seats.held(), ["lead blocked held: unchanged"], "{err}");
    assert_eq!(seats.challenged(), 0, "{err}");
}
