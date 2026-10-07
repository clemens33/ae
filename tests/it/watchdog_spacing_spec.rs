//! Frozen #225 S1 acceptance, from lead ruling f1647223 and plan rev3.
//!
//! Oracle: first/proactive/news waits use one cadence; an answered, delivered
//! same-state challenge earns two cadences until news or a proactive declaration.
//! The declaration-age ceiling, retry bound, and done timeline stay unchanged.
//! Times/counts below are requirements, never obtained from the fold or cap function.
//! Each fixture is a real JSONL journal read through `RecordSnapshot`, then consumed
//! by `wait_progress`, the list entry, and daemon account. No live state or harness.

#![allow(
    clippy::expect_used,
    reason = "frozen acceptance asserts that private fixture setup succeeds"
)]

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use ae::attention::Reason;
use ae::digest::{SessionEntry, Status};
use ae::harness_state::HarnessState;
use ae::meta::Meta;
use ae::session::{AgentRuntime, RecordSnapshot, SessionRuntime, entry_from};
use ae::time::Timestamp;
use ae::watchdog::{self, DoneProgress, QuietKind, WaitProgress, WaitState};
use ae::watchdog_daemon::{
    Accounting, Effect, HarnessObservation, Knobs, Observation, PaneState, Verdict, account,
};

use super::cli::OwnedScratch;

const SESSION: &str = "spacing";
const AGENT: &str = "agent";
const SLOT: &str = "main";
const TARGET: &str = r#","target":"agent","target_slot":"main","target_session":"spacing""#;
const ACTOR: &str = r#","actor_slot":"main","actor_session":"spacing""#;

fn at(seconds: i64) -> Timestamp {
    Timestamp::from_epoch(
        Timestamp::parse("2026-10-07T12:00:00Z")
            .expect("fixture origin")
            .epoch()
            + seconds,
    )
}

fn row(seconds: i64, actor: &str, action: &str, extra: &str) -> String {
    format!(
        r#"{{"ts":"{}","actor":"{actor}","action":"{action}"{extra}}}"#,
        at(seconds)
    )
}

struct Journal {
    root: OwnedScratch,
    lines: Vec<String>,
    cadence: u64,
    required: u8,
    launch: Option<&'static str>,
}

impl Journal {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self {
            root: OwnedScratch::root("ws", &NEXT.fetch_add(1, Ordering::Relaxed).to_string()),
            lines: Vec::new(),
            cadence: 300,
            required: 2,
            launch: Some("L1"),
        }
    }

    fn declare(&mut self, seconds: i64, state: &str) {
        self.lines.push(row(
            seconds,
            AGENT,
            "state",
            &format!(r#"{ACTOR},"ref":"{state}","summary":"still waiting""#),
        ));
    }

    fn footprint(&mut self, seconds: i64, action: &str, summary: &str) {
        let reference = self
            .launch
            .map_or_else(String::new, |id| format!(r#","ref":"{id}""#));
        self.lines.push(row(
            seconds,
            "watchdog",
            action,
            &format!(r#"{TARGET}{reference},"summary":"{summary}""#),
        ));
    }

    fn challenge(&mut self, seconds: i64, state: WaitState, number: u8) {
        self.footprint(
            seconds,
            "wait-challenge",
            &format!("{} confirmation {number}/{}", state.as_str(), self.required),
        );
    }

    fn proof(state: WaitState) -> Self {
        let mut journal = Self::new();
        journal.declare(0, state.as_str());
        journal.challenge(300, state, 1);
        journal.declare(330, state.as_str());
        journal
    }

    fn peer_news(&mut self, seconds: i64) {
        self.lines.push(row(seconds, "peer", "send", TARGET));
    }

    fn own_news(&mut self, seconds: i64) {
        self.lines.push(row(seconds, AGENT, "send", ACTOR));
    }

    fn snapshot(&self) -> RecordSnapshot {
        let dir = self.root.path().join("sessions").join(SESSION);
        fs::create_dir_all(&dir).expect("private session directory");
        let launch = self
            .launch
            .map_or_else(String::new, |id| format!("launch_id.main={id}\n"));
        let meta = format!(
            "schema=2\nmode=local\nsession={SESSION}\nseat.main={AGENT}\n\
             profile.main=spec\nagent_bin.main=codex\n{launch}\
             idle_nudge_secs={}\ndone_confirmations={}\n",
            self.cadence, self.required
        );
        fs::write(dir.join("meta"), &meta).expect("fixture meta");
        fs::write(dir.join("events.jsonl"), self.lines.join("\n") + "\n").expect("fixture journal");
        let snapshot = RecordSnapshot::read(&dir);
        let read = snapshot.events.as_ref().expect("journal read");
        assert!(read.skipped.is_empty(), "fixture parse: {:?}", read.skipped);
        assert_eq!(read.events.len(), self.lines.len(), "all records parsed");
        assert_eq!(snapshot.meta, Some(Meta::parse(&meta)));
        snapshot
    }

    fn progress(&self, now: Timestamp, state: WaitState) -> WaitProgress {
        let snapshot = self.snapshot();
        watchdog::wait_progress(
            &snapshot.events.expect("parsed journal").events,
            SESSION,
            SLOT,
            AGENT,
            self.launch,
            now,
            self.cadence,
            self.required,
            state,
        )
    }

    /// Production readers select the raw declaration, not an effective blocked
    /// verdict. Feed their shared parsed records to all three consumers.
    fn observe(&self, now: Timestamp, state: WaitState, frame: HarnessState) -> Seen {
        let snapshot = self.snapshot();
        let events = &snapshot.events.as_ref().expect("parsed journal").events;
        let progress = watchdog::wait_progress(
            events,
            SESSION,
            SLOT,
            AGENT,
            self.launch,
            now,
            self.cadence,
            self.required,
            state,
        );
        let done_progress = watchdog::done_progress(
            events,
            SESSION,
            SLOT,
            AGENT,
            self.launch,
            now,
            self.cadence,
            self.required,
        );
        let quiet = watchdog::latest_relevant_event(events, SESSION, SLOT, AGENT)
            .and_then(|event| watchdog::quiet_reason(&event));
        let runtime = SessionRuntime {
            agents: vec![AgentRuntime {
                slot: SLOT.to_owned(),
                alive: Some(true),
                alert: None,
                observed: frame,
            }],
            ..SessionRuntime::new(Status::Running)
        };
        let entry = entry_from(&snapshot, SESSION, &runtime, now, 3600);
        let observation = Observation {
            now_epoch: now.epoch(),
            hash: 7,
            harness: HarnessObservation {
                frame,
                human_draft: false,
                durable_stale: false,
                declaration: None,
            },
            identity: 1,
            is_dead: false,
            throttle: None,
            limit_notice: None,
            capture_ok: true,
            human_prompt: None,
            throttle_quota: None,
            quiet,
            ended_wait: None,
            done_progress,
            wait_progress: progress,
            descendancy: ae::procs::Descendancy::Present,
            launch_attempt: ae::tmux::Evidence::Silent,
            outstanding_alert: None,
            last_actor_event_age_secs: ae::watchdog_daemon::last_actor_event_age(
                events,
                SESSION,
                SLOT,
                AGENT,
                now.epoch(),
            ),
            declared_age_secs: ae::watchdog_daemon::declaration_age(
                events,
                SESSION,
                SLOT,
                AGENT,
                now.epoch(),
            ),
            sweep: None,
            own_work: entry.agents[0].own_work.unwrap_or_default(),
            auto_in_flight: None,
            auto_deadline: None,
        };
        let knobs = Knobs {
            idle_nudge_secs: self.cadence,
            done_confirmations: self.required,
            undelivered_max: 3,
            ..Knobs::default()
        };
        Seen {
            progress,
            entry,
            accounting: account(&PaneState::default(), &observation, &knobs),
        }
    }

    fn check(&self, seconds: i64, state: WaitState, want: WaitProgress) -> Seen {
        self.check_at(at(seconds), state, want)
    }

    fn check_at(&self, now: Timestamp, state: WaitState, want: WaitProgress) -> Seen {
        let seen = self.observe(now, state, HarnessState::Unknown);
        assert_eq!(seen.progress, want, "fold at {now}");
        let want_cell = (want != WaitProgress::None).then_some(want);
        assert_eq!(
            seen.entry.agents[0].wait_progress, want_cell,
            "list at {now}"
        );
        let should_push = matches!(
            want,
            WaitProgress::ChallengeDue {
                attempts: 0..=2,
                ..
            }
        );
        assert_eq!(
            seen.accounting
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::WaitChallenge { .. })),
            should_push,
            "daemon at {now}: {:?}",
            seen.accounting.effects
        );
        seen
    }
}

struct Seen {
    progress: WaitProgress,
    entry: SessionEntry,
    accounting: Accounting,
}

fn provisional(confirmations: u8, required: u8) -> WaitProgress {
    WaitProgress::Provisional {
        confirmations,
        required,
    }
}

fn due(confirmations: u8, required: u8, age: u64, attempts: u32) -> WaitProgress {
    WaitProgress::ChallengeDue {
        confirmations,
        required,
        wait_age_secs: age,
        attempts,
    }
}

#[test]
fn unchanged_waiting_agent_earns_600_seconds_after_verified_proof() {
    let state = WaitState::WaitingAgent;
    let journal = Journal::proof(state);
    journal.check(630, state, provisional(1, 2));
    journal.check(929, state, provisional(1, 2));
    journal.check(930, state, due(1, 2, 600, 0));
}

#[test]
fn unchanged_blocked_earns_the_same_spacing() {
    let state = WaitState::Blocked;
    let journal = Journal::proof(state);
    journal.check(630, state, provisional(1, 2));
    journal.check(929, state, provisional(1, 2));
    journal.check(930, state, due(1, 2, 600, 0));
}

#[test]
fn first_challenge_keeps_the_base_deadline_in_both_wait_states() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        let mut journal = Journal::new();
        journal.declare(0, state.as_str());
        journal.check(299, state, provisional(0, 2));
        journal.check(300, state, due(0, 2, 300, 0));
    }
}

#[test]
fn own_and_addressed_news_restore_base_spacing_without_ending_a_wait() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        for own in [false, true] {
            let mut journal = Journal::proof(state);
            if own {
                journal.own_news(400);
            } else {
                journal.peer_news(400);
            }
            journal.check(629, state, provisional(1, 2));
            journal.check(630, state, due(1, 2, 300, 0));
        }
    }
}

#[test]
fn a_later_proof_clears_old_news_but_not_news_after_that_proof() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        let mut journal = Journal::proof(state);
        journal.peer_news(400);
        journal.challenge(630, state, 2);
        journal.declare(660, state.as_str());
        journal.check(960, state, provisional(0, 2));
        journal.check(1260, state, due(0, 2, 600, 0));
        journal.own_news(700);
        journal.check(960, state, due(0, 2, 300, 0));
    }
}

#[test]
fn nth_proof_rearms_with_extended_spacing_including_required_one() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        let mut journal = Journal::proof(state);
        journal.challenge(930, state, 2);
        journal.declare(960, state.as_str());
        journal.check(1260, state, provisional(0, 2));
        journal.check(1559, state, provisional(0, 2));
        journal.check(1560, state, due(0, 2, 600, 0));

        let mut one = Journal::new();
        one.required = 1;
        one.declare(0, state.as_str());
        one.challenge(300, state, 1);
        one.declare(330, state.as_str());
        one.check(630, state, provisional(0, 1));
        one.check(930, state, due(0, 1, 600, 0));
    }
}

#[test]
fn proactive_declarations_clear_earned_spacing_before_and_after_news() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        for news_first in [false, true] {
            let mut journal = Journal::proof(state);
            if news_first {
                journal.peer_news(380);
            }
            journal.declare(400, state.as_str());
            if !news_first {
                journal.peer_news(420);
            }
            journal.check(699, state, provisional(1, 2));
            journal.check(700, state, due(1, 2, 300, 0));
        }
        let mut no_news = Journal::proof(state);
        no_news.declare(400, state.as_str());
        no_news.check(700, state, due(1, 2, 300, 0));
        no_news.declare(720, state.as_str());
        no_news.check(1019, state, provisional(1, 2));
        no_news.check(1020, state, due(1, 2, 300, 0));
    }
}

#[test]
fn proactive_declarations_without_a_delivered_challenge_never_earn_spacing() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        let mut journal = Journal::new();
        for seconds in [0, 100, 200, 330] {
            journal.declare(seconds, state.as_str());
        }
        journal.check(629, state, provisional(0, 2));
        journal.check(630, state, due(0, 2, 300, 0));
    }
}

#[test]
fn delivered_challenge_lapses_on_base_cadence_and_late_proof_earns_spacing() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        let mut journal = Journal::new();
        journal.declare(0, state.as_str());
        journal.challenge(300, state, 1);
        journal.check(
            599,
            state,
            WaitProgress::Challenged {
                confirmations: 0,
                required: 2,
            },
        );
        let lapsed = journal.check(
            600,
            state,
            WaitProgress::Lapsed {
                confirmations: 0,
                required: 2,
            },
        );
        assert!(lapsed.accounting.effects.contains(&Effect::Nudge));
        journal.footprint(610, "nudge", "ordinary reminder after lapsed proof");
        journal.declare(900, state.as_str());
        journal.check(1200, state, provisional(1, 2));
        journal.check(1500, state, due(1, 2, 600, 0));
    }
}

#[test]
fn same_second_append_order_decides_whether_news_is_since_the_proof() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        for news_after in [false, true] {
            let mut journal = Journal::new();
            journal.declare(0, state.as_str());
            journal.challenge(300, state, 1);
            if !news_after {
                journal.peer_news(330);
            }
            journal.declare(330, state.as_str());
            if news_after {
                journal.peer_news(330);
            }
            journal.check(
                630,
                state,
                if news_after {
                    due(1, 2, 300, 0)
                } else {
                    provisional(1, 2)
                },
            );
        }
    }
}

#[test]
fn same_second_challenge_and_identical_proof_still_count_by_append_order() {
    let state = WaitState::Blocked;
    let mut journal = Journal::new();
    journal.declare(0, state.as_str());
    journal.challenge(0, state, 1);
    journal.declare(0, state.as_str());
    journal.check(300, state, provisional(1, 2));
    journal.check(600, state, due(1, 2, 600, 0));
}

#[test]
fn news_after_an_extended_deadline_does_not_hide_an_overdue_challenge() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        let mut journal = Journal::proof(state);
        journal.peer_news(940);
        journal.check(950, state, due(1, 2, 620, 0));
    }
}

#[test]
fn routing_owners_select_news_instead_of_stale_display_names() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        for extra in [
            r#","target":"old-name","target_slot":"main","target_session":"spacing""#,
            r#","actor_slot":"main","actor_session":"spacing""#,
        ] {
            let mut journal = Journal::proof(state);
            journal.lines.push(row(400, "old-name", "send", extra));
            journal.check(630, state, due(1, 2, 300, 0));
        }
        for (actor, extra) in [
            (
                "peer",
                r#","target":"agent","target_slot":"worker.9","target_session":"spacing""#,
            ),
            (
                "peer",
                r#","target":"agent","target_slot":"main","target_session":"elsewhere""#,
            ),
            ("peer", r#","target":"agent","target_slot":"""#),
            (
                AGENT,
                r#","actor_slot":"worker.9","actor_session":"spacing""#,
            ),
        ] {
            let mut journal = Journal::proof(state);
            journal.lines.push(row(400, actor, "send", extra));
            journal.check(630, state, provisional(1, 2));
        }
    }
}

#[test]
fn non_news_watchdog_footprints_and_unrelated_records_preserve_spacing() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        let mut journal = Journal::proof(state);
        journal.footprint(400, "nudge", "ordinary watchdog reminder");
        journal.footprint(410, "quota-advisory", "quota changed");
        journal.footprint(420, "sweep-nudge", "overview reminder");
        journal
            .lines
            .push(row(430, "other", "send", r#","target":"peer""#));
        journal.check(630, state, provisional(1, 2));
        journal.check(930, state, due(1, 2, 600, 0));
    }
}

#[test]
fn own_memo_chase_and_inbound_task_are_news_without_becoming_wait_answers() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        for (actor, action, extra) in [
            (AGENT, "memo", ACTOR.to_owned()),
            (
                AGENT,
                "ask",
                format!(r#"{ACTOR},"target":"peer","ref":"chase""#),
            ),
            ("peer", "ask", format!(r#"{TARGET},"ref":"task""#)),
        ] {
            let mut journal = Journal::proof(state);
            journal.lines.push(row(400, actor, action, &extra));
            journal.check(630, state, due(1, 2, 300, 0));
        }
    }
}

#[test]
fn records_that_end_waits_still_supersede_the_episode() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        for ending_state in ["working", "waiting-user", "done"] {
            let mut journal = Journal::proof(state);
            journal.declare(400, ending_state);
            journal.check(930, state, WaitProgress::None);
        }
        let mut human = Journal::proof(state);
        human
            .lines
            .push(row(400, "telegram:fixture", "send", TARGET));
        human.check(930, state, WaitProgress::None);
    }
    let state = WaitState::WaitingAgent;
    let mut answered = Journal::new();
    answered.lines.push(row(
        0,
        AGENT,
        "ask",
        &format!(r#"{ACTOR},"target":"peer","ref":"request""#),
    ));
    answered.declare(1, state.as_str());
    answered.challenge(301, state, 1);
    answered.declare(331, state.as_str());
    answered.lines.push(row(
        400,
        "peer",
        "reply",
        &format!(r#"{TARGET},"ref":"request""#),
    ));
    answered.check(931, state, WaitProgress::None);
}

#[test]
fn launch_mismatch_and_crossed_challenges_keep_existing_resets() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        let mut mismatch = Journal::proof(state);
        mismatch.lines.push(row(
            400,
            "watchdog",
            "wait-challenge",
            &format!(
                r#"{TARGET},"ref":"old-launch","summary":"{} confirmation 2/2""#,
                state.as_str()
            ),
        ));
        mismatch.check(930, state, WaitProgress::None);

        let mut crossed = Journal::proof(state);
        crossed.footprint(400, "done-challenge", "confirmation 1/2");
        crossed.check(930, state, WaitProgress::None);

        let other = match state {
            WaitState::WaitingAgent => WaitState::Blocked,
            WaitState::Blocked => WaitState::WaitingAgent,
        };
        let mut wrong_state = Journal::proof(state);
        wrong_state.challenge(400, other, 1);
        wrong_state.check(630, state, provisional(1, 2));
        wrong_state.check(930, state, due(1, 2, 600, 0));
    }
}

#[test]
fn switching_wait_state_starts_fresh_instead_of_inheriting_a_proof() {
    let mut journal = Journal::proof(WaitState::WaitingAgent);
    journal.declare(400, "blocked");
    assert_eq!(
        journal.progress(at(700), WaitState::WaitingAgent),
        WaitProgress::None
    );
    journal.check(699, WaitState::Blocked, provisional(0, 2));
    journal.check(700, WaitState::Blocked, due(0, 2, 300, 0));
}

#[test]
fn legacy_display_and_refless_challenges_are_still_readable() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        let mut journal = Journal::new();
        journal.launch = None;
        journal.lines.push(row(
            0,
            AGENT,
            "state",
            &format!(r#","ref":"{}""#, state.as_str()),
        ));
        journal.lines.push(row(
            300,
            "watchdog",
            "wait-challenge",
            &format!(
                r#","target":"agent","summary":"{} confirmation 1/2""#,
                state.as_str()
            ),
        ));
        journal.lines.push(row(
            330,
            AGENT,
            "state",
            &format!(r#","ref":"{}""#, state.as_str()),
        ));
        journal.check(630, state, provisional(1, 2));
        journal.check(930, state, due(1, 2, 600, 0));

        journal.launch = Some("L1");
        journal.check(629, state, provisional(0, 2));
        journal.check(630, state, due(0, 2, 300, 0));
    }
}

#[test]
fn failed_attempts_never_earn_credit_and_keep_the_unreachable_bound() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        let mut journal = Journal::new();
        journal.declare(0, state.as_str());
        for (index, action, prefix, suffix) in [
            (0, "wait-challenge", "[unconfirmed] ", ""),
            (
                1,
                "wait-challenge",
                "[unconfirmed] ",
                "; refused pre-paste: dead pane",
            ),
            (2, "delivery-abandoned", "abandoned: busy; ", ""),
        ] {
            journal.footprint(
                300 + index,
                action,
                &format!("{prefix}{} confirmation 1/2{suffix}", state.as_str()),
            );
            let attempts = u32::try_from(index + 1).expect("three attempts");
            let seen = journal.check(310, state, due(0, 2, 310, attempts));
            assert_eq!(
                seen.accounting.effects.iter().any(|effect| matches!(effect, Effect::Emit { action: "alert", summary } if summary.contains("unreachable"))),
                attempts == 3,
            );
        }
        journal.declare(330, state.as_str());
        journal.check(629, state, provisional(0, 2));
        journal.check(630, state, due(0, 2, 300, 3));
        journal.footprint(
            640,
            "delivery-abandoned",
            &format!("abandoned: busy; {} confirmation 1/2", state.as_str()),
        );
        let past_bound = journal.check(650, state, due(0, 2, 320, 4));
        assert!(
            past_bound.accounting.effects.is_empty(),
            "no repeated unreachable alert"
        );
    }
}

#[test]
fn failed_footprints_do_not_strip_previously_earned_spacing() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        let mut journal = Journal::proof(state);
        // Compatible pre-upgrade attempts from the old one-cadence schedule
        // count toward the bound but cannot revoke an already credited proof.
        journal.footprint(
            630,
            "wait-challenge",
            &format!(
                "[unconfirmed] {} confirmation 2/2; refused pre-paste: no composer",
                state.as_str()
            ),
        );
        journal.footprint(
            640,
            "delivery-abandoned",
            &format!("abandoned: busy; {} confirmation 2/2", state.as_str()),
        );
        journal.check(650, state, provisional(1, 2));
        journal.check(950, state, due(1, 2, 620, 2));
    }
}

#[test]
fn cadence_one_and_non_default_cadence_use_exact_two_periods() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        for (cadence, proof, before, deadline) in [(1, 1, 2, 3), (420, 450, 1289, 1290)] {
            let mut journal = Journal::new();
            journal.cadence = cadence;
            journal.declare(0, state.as_str());
            journal.challenge(i64::try_from(cadence).expect("small cadence"), state, 1);
            journal.declare(proof, state.as_str());
            journal.check(before, state, provisional(1, 2));
            journal.check(
                deadline,
                state,
                due(
                    1,
                    2,
                    u64::try_from(deadline - proof).expect("positive age"),
                    0,
                ),
            );
        }
    }
}

#[test]
fn disabled_knobs_keep_wait_progress_absent_and_never_deliver_challenges() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        for (cadence, required) in [(0, 2), (300, 0)] {
            let mut journal = Journal::proof(state);
            journal.cadence = cadence;
            journal.required = required;
            journal.check(930, state, WaitProgress::None);
        }
    }
}

#[test]
fn overflowing_cap_arithmetic_falls_back_to_base_never_zero_or_shorter() {
    // Largest cadence with an exact 4x ceiling, and the first overflowing
    // value. These literal boundaries come from u64 range and the ruled 4x cap.
    const LAST_EXACT: u64 = 4_611_686_018_427_387_903;
    const FIRST_OVERFLOW: u64 = 4_611_686_018_427_387_904;
    let state = WaitState::Blocked;
    for (cadence, expected) in [
        (LAST_EXACT, provisional(1, 2)),
        (FIRST_OVERFLOW, due(1, 2, FIRST_OVERFLOW, 0)),
    ] {
        let mut journal = Journal::proof(state);
        journal.cadence = cadence;
        journal.check(330, state, provisional(1, 2));
        let now = Timestamp::from_epoch(
            at(330).epoch() + i64::try_from(cadence).expect("cadence fits i64"),
        );
        journal.check_at(now, state, expected);
    }
    for cadence in [u64::MAX / 2, u64::MAX - 1, u64::MAX] {
        let mut journal = Journal::proof(state);
        journal.cadence = cadence;
        journal.check(330, state, provisional(1, 2));
        journal.check(630, state, provisional(1, 2));
        journal.check_at(Timestamp::from_epoch(i64::MAX), state, provisional(1, 2));
    }
}

#[test]
fn list_and_daemon_escalate_from_declaration_age_despite_late_delivery() {
    let state = WaitState::WaitingAgent;
    let mut journal = Journal::new();
    journal.declare(0, state.as_str());
    // A delayed poll delivers near the declaration's unchanged 1200s ceiling.
    journal.challenge(1100, state, 1);
    let waiting = WaitProgress::Challenged {
        confirmations: 0,
        required: 2,
    };
    let before = journal.check(1199, state, waiting);
    assert_eq!(before.entry.agents[0].reason, None);
    assert_eq!(
        before.accounting.verdict,
        Verdict::Quiet(QuietKind::WaitingAgent)
    );
    let ceiling = journal.check(1200, state, waiting);
    assert_eq!(
        ceiling.entry.agents[0].state.as_deref(),
        Some("waiting-agent")
    );
    assert_eq!(ceiling.entry.agents[0].reason, Some(Reason::Blocked));
    assert_eq!(
        ceiling.accounting.verdict,
        Verdict::Quiet(QuietKind::Blocked)
    );
    assert!(!ceiling.accounting.effects.contains(&Effect::Nudge));
    journal.declare(1230, state.as_str());
    let renewed = journal.check(1530, state, provisional(1, 2));
    assert_eq!(renewed.entry.agents[0].reason, None);
    assert_eq!(
        renewed.accounting.verdict,
        Verdict::Quiet(QuietKind::WaitingAgent)
    );
    journal.check(1830, state, due(1, 2, 600, 0));
    let lapse = journal.check(2429, state, due(1, 2, 1199, 0));
    assert_eq!(lapse.entry.agents[0].reason, None);
    let old = journal.check(2430, state, due(1, 2, 1200, 0));
    assert_eq!(old.entry.agents[0].reason, Some(Reason::Blocked));
    assert_eq!(old.accounting.verdict, Verdict::Quiet(QuietKind::Blocked));
    assert!(old.accounting.effects.contains(&Effect::WaitChallenge {
        confirmations: 1,
        required: 2,
        wait_age_secs: 1200,
        state,
        escalated: true,
    }));
}

#[test]
fn zero_cadence_keeps_the_attention_marker_at_1200_without_nudges() {
    let state = WaitState::WaitingAgent;
    let mut journal = Journal::new();
    journal.cadence = 0;
    journal.declare(0, state.as_str());
    let before = journal.check(1199, state, WaitProgress::None);
    assert_eq!(before.entry.agents[0].reason, None);
    let old = journal.check(1200, state, WaitProgress::None);
    assert_eq!(old.entry.agents[0].reason, Some(Reason::Blocked));
    assert_eq!(old.accounting.verdict, Verdict::Quiet(QuietKind::Blocked));
    assert!(old.accounting.effects.is_empty());
}

#[test]
fn daemon_delivers_once_due_with_unchanged_payload_and_budget() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        let journal = Journal::proof(state);
        // Changing only the pane frame must not create journal news or remove
        // quiet suppression. Idle/busy/unknown all use the same wait fold.
        for frame in [
            HarnessState::Unknown,
            HarnessState::Idle,
            HarnessState::Busy,
        ] {
            let early = journal.observe(at(630), state, frame);
            assert_eq!(early.progress, provisional(1, 2));
            assert!(early.accounting.effects.is_empty());
            let ready = journal.observe(at(930), state, frame);
            assert_eq!(
                ready.accounting.effects,
                vec![Effect::WaitChallenge {
                    confirmations: 1,
                    required: 2,
                    wait_age_secs: 600,
                    state,
                    escalated: false,
                }]
            );
            assert_eq!(ready.accounting.next.nudge_count, 0);
        }
    }
}

#[test]
fn list_keeps_existing_unconfirmed_cell_words_for_both_wait_states() {
    for state in [WaitState::WaitingAgent, WaitState::Blocked] {
        let journal = Journal::proof(state);
        for seconds in [630, 930] {
            let seen = journal.observe(at(seconds), state, HarnessState::Unknown);
            let table = ae::listing::table_at(&[&seen.entry], at(seconds));
            assert!(
                table.contains(&format!("{} (unconfirmed 1/2)", state.as_str())),
                "{table}"
            );
            assert_eq!(seen.entry.agents[0].to_json().get("wait_progress"), None);
        }
    }
}

#[test]
fn done_progress_retains_base_challenges_and_terminal_confirmation() {
    let mut journal = Journal::new();
    journal.declare(0, "done");
    journal.footprint(300, "done-challenge", "confirmation 1/2");
    journal.declare(330, "done");
    for (seconds, want, delivery) in [
        (
            629,
            DoneProgress::Provisional {
                confirmations: 1,
                required: 2,
            },
            false,
        ),
        (
            630,
            DoneProgress::ChallengeDue {
                confirmations: 1,
                required: 2,
                done_age_secs: 300,
                attempts: 0,
            },
            true,
        ),
    ] {
        let seen = journal.observe(at(seconds), WaitState::WaitingAgent, HarnessState::Unknown);
        let events = journal.snapshot().events.expect("parsed journal").events;
        assert_eq!(
            watchdog::done_progress(
                &events,
                SESSION,
                SLOT,
                AGENT,
                Some("L1"),
                at(seconds),
                300,
                2
            ),
            want
        );
        assert_eq!(seen.entry.agents[0].done_progress, Some(want));
        assert_eq!(
            seen.accounting
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::DoneChallenge { .. })),
            delivery
        );
    }
    journal.footprint(630, "done-challenge", "confirmation 2/2");
    journal.declare(660, "done");
    let confirmed = journal.observe(at(1260), WaitState::WaitingAgent, HarnessState::Unknown);
    let events = journal.snapshot().events.expect("parsed journal").events;
    assert_eq!(
        watchdog::done_progress(&events, SESSION, SLOT, AGENT, Some("L1"), at(1260), 300, 2),
        DoneProgress::Confirmed
    );
    assert_eq!(confirmed.entry.agents[0].done_progress, None);
    assert!(confirmed.accounting.effects.is_empty());
}
