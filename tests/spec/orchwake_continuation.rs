//! Product-path checks for the navigator's C1–C9 continuation contract.

#![allow(
    clippy::disallowed_methods,
    reason = "the fixture reads its own scratch files and helper observations"
)]

use super::super::{Activity, account_settling};
use super::*;
use crate::autoreseat::{Fate, NOTICE_ACTION, Nudge, leg};
use crate::meta::{self, ContinuationPhase, ContinuationStep, Conversation, ReseatContinuation};
use crate::time::Timestamp;
use std::os::unix::fs::PermissionsExt;

const LAUNCH: &str = "11111111-1111-4111-8111-111111111111";
const SUCCESSOR: &str = "22222222-2222-4222-8222-222222222222";
const INITIAL: &str = "33333333-3333-4333-8333-333333333333";
const FLEET: &str = "WORKING\n  alpha lead ship the slice";
const HASH: &str = "0123456789abcdef";

struct Fixture {
    scratch: Scratch,
    helper: SendHelper,
    server: ServerId,
    orchestrator: bool,
}

impl Fixture {
    fn new(orchestrator: bool) -> Self {
        let scratch = Scratch::new("orchwake-continuation");
        let dir = &scratch.0;
        let role = if orchestrator {
            "meta_agent=true\n"
        } else {
            ""
        };
        std::fs::write(
            dir.join("meta"),
            format!(
                "session=demo\nseat.main=orchestrator\nagent_bin.main=claude\n\
                 launch_id.main={INITIAL}\nharness_session.main={INITIAL}\n\
                 charter.main=1\ncharter_launch.main={INITIAL}\n{role}"
            ),
        )
        .expect("initial seat");
        let helper = SendHelper::for_session(dir);
        let send = dir.join("send");
        std::fs::write(&send, format!(r#"#!/bin/sh
set -e
cd "$(dirname "$0")"
printf '%s|%s|%s\n' "$_AE_EXPECT_SLOT" "$_AE_EXPECT_LAUNCH" "$1" >> calls
printf '%s' "$2" > body
cp meta at-call
summary=$_AE_EVENT_SUMMARY
extra=
case "$(cat mode)" in
    refused)
        printf '{{"ts":"2026-11-23T06:00:00Z","actor":"watchdog","action":"delivery-abandoned","target":"%s","ref":"%s","summary":"refused: target stayed busy (human-only prompt on the pane); %s"}}\n' "$1" "$_AE_EVENT_REF" "$summary" >> events.jsonl
        printf '%s\n' '{}'
        exit 1 ;;
    absent) exit 0 ;;
    pasted) exit 1 ;;
    pending) summary="{}$summary" ;;
    unverifiable) extra=',"unverifiable":"frame missing"' ;;
    verified) ;;
    *) exit 2 ;;
esac
printf '{{"ts":"2026-11-23T06:00:00Z","actor":"watchdog","action":"%s","target":"%s","ref":"%s","summary":"%s"%s}}\n' "$_AE_EVENT_ACTION" "$1" "$_AE_EVENT_REF" "$summary" "$extra" >> events.jsonl
"#, crate::send::RETRYABLE_MARKER, crate::tracked::UNCONFIRMED_SUMMARY_PREFIX)).expect("send fixture");
        std::fs::set_permissions(&send, std::fs::Permissions::from_mode(0o755)).expect("exec");
        std::fs::write(dir.join("mode"), "verified").expect("default receipt");
        let server = ServerId::Selected(Selector::Socket(dir.join("nobody.sock")));
        Self {
            scratch,
            helper,
            server,
            orchestrator,
        }
    }

    fn dir(&self) -> &Path {
        &self.scratch.0
    }

    fn publish(&self, launch: &str, conversation: Conversation<'_>) {
        meta::publish_seat_move(
            self.dir(),
            "main",
            "orchestrator",
            &meta::SeatMove {
                profile: "successor",
                binary: "claude",
                launch_id: launch,
                conversation,
                orchestrator_continuation: self.orchestrator,
                pair: None,
            },
        )
        .expect("publish through the reseat's guarded writer");
    }

    fn leg_nudge(&self, fate: Fate) -> Option<Nudge> {
        leg::post_move_nudge(
            &leg::Argv {
                session: "demo".to_owned(),
                dir: self.dir().to_path_buf(),
                slot: "main".to_owned(),
                key: Timestamp::from_epoch(0),
            },
            "orchestrator",
            fate,
            &crate::reseat::Reported {
                minted: Some(LAUNCH.to_owned()),
                orchestrator_continuation: self.orchestrator,
                ..Default::default()
            },
        )
    }

    fn cycle(&self, launch: &str) -> super::super::Cycle<'_> {
        let mut cycle = demo_cycle(self.dir(), &self.helper, &self.server);
        cycle.meta_agent = self.orchestrator;
        cycle.launch_ids = vec![(
            "main".to_owned(),
            super::super::SeatLaunch::Id(launch.to_owned()),
        )];
        cycle
    }

    fn tick(
        &self,
        prior: &PaneState,
        observed: &Observation,
        overview: Option<&OverviewReading>,
        settling: bool,
    ) -> PaneState {
        let cycle = self.cycle(LAUNCH);
        let owed = cycle.read_reseat_continuation();
        self.apply(&cycle, owed, prior, observed, overview, settling)
    }

    fn apply(
        &self,
        cycle: &super::super::Cycle<'_>,
        owed: Option<ReseatContinuation>,
        prior: &PaneState,
        observed: &Observation,
        overview: Option<&OverviewReading>,
        settling: bool,
    ) -> PaneState {
        let mut booked = account_settling(
            prior,
            observed,
            &cycle.knobs,
            None,
            &Activity::default(),
            settling,
        );
        if observed.sweep.is_some()
            && let Some(owed) = owed
        {
            super::super::continuation::book(&mut booked, observed, owed, settling);
        }
        let events = read_events(self.dir());
        let on = super::super::Acting {
            agent: "orchestrator",
            slot: "main",
            pane: "%0",
            seen: observed,
            events: &events,
            overview,
        };
        let mut err = Vec::new();
        cycle
            .apply_booked(&booked.effects, &on, &mut booked.next, &mut false, &mut err)
            .expect("apply the watchdog's booking");
        assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
        booked.next
    }

    fn obligation(&self) -> Option<ReseatContinuation> {
        meta::reseat_continuation(&meta::read_bytes(self.dir()).expect("seat meta"))
    }

    fn mode(&self, mode: &str) {
        std::fs::write(self.dir().join("mode"), mode).expect("receipt mode");
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir().join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn body(&self) -> String {
        std::fs::read_to_string(self.dir().join("body")).expect("a sent turn")
    }

    fn phase(&self) -> Option<ContinuationPhase> {
        self.obligation().map(|row| row.phase)
    }
}

fn carried() -> Conversation<'static> {
    Conversation::Carried {
        config_home: "/account-b",
        config_home_base: None,
    }
}

fn reading() -> OverviewReading {
    OverviewReading {
        rendered: FLEET.to_owned(),
        hash: HASH.to_owned(),
        checkpoint: crate::monitor::OverviewCheckpoint {
            hash: None,
            outstanding_since: None,
            last_delivered_at: None,
        },
    }
}

fn no_overview() -> Observation {
    let mut observed = orchwake_seat(None);
    observed.sweep = Some(SweepObservation::new(at(1000), None).with_overview(false, None, None));
    observed
}

fn expected_call() -> [String; 1] {
    [format!("main|{LAUNCH}|orchestrator")]
}

#[test]
fn orchwake_c1_prompt_holds_a_published_carried_or_kept_turn() {
    for (conversation, fate) in [
        (carried(), meta::ContinuationFate::Carried),
        (Conversation::Kept, meta::ContinuationFate::Kept),
    ] {
        let f = Fixture::new(true);
        f.publish(LAUNCH, conversation);
        let observed = orchwake_seat(Some(orchwake_report_only()));
        let prior = f.tick(&PaneState::default(), &observed, Some(&reading()), false);
        let after = f.tick(&prior, &observed, Some(&reading()), false);
        assert_eq!(after.human_prompt_streak, 2);
        assert!(f.calls().is_empty());
        assert_eq!(
            f.obligation(),
            Some(ReseatContinuation {
                launch: LAUNCH.to_owned(),
                fate,
                phase: ContinuationPhase::Owed,
            })
        );
    }
}

#[test]
fn orchwake_c2_first_clear_admitted_cycle_combines_the_checked_turn() {
    let f = Fixture::new(true);
    f.publish(LAUNCH, carried());
    let fleet = reading();
    let modal = orchwake_seat(Some(orchwake_report_only()));
    let state = f.tick(&PaneState::default(), &modal, Some(&fleet), false);
    let clear = orchwake_seat(None);
    let state = f.tick(&state, &clear, Some(&fleet), true);
    assert!(f.calls().is_empty(), "wake settling admits no turn");
    let state = f.tick(&state, &clear, Some(&fleet), false);
    assert_eq!(f.calls(), expected_call());
    let body = f.body();
    assert!(
        body.find("Restore your state first").expect("continuation")
            < body.find(FLEET).expect("overview"),
        "{body}"
    );
    let at_call = std::fs::read(f.dir().join("at-call")).expect("at helper entry");
    assert_eq!(
        meta::reseat_continuation(&at_call)
            .expect("pre-paste claim")
            .phase,
        ContinuationPhase::Claimed
    );
    assert_eq!(f.phase(), None);
    assert_eq!(state.sweep.unacknowledged_deliveries, 1);
    let checkpoint = crate::monitor::overview_heartbeat(f.dir(), 10_001).expect("overview state");
    assert_eq!(checkpoint.hash.as_deref(), Some(HASH));
    let events = read_events(f.dir());
    assert_eq!(
        crate::watchdog::last_watchdog_delivery(&events, "demo", "main", "orchestrator"),
        Timestamp::parse("2026-11-23T06:00:00Z")
    );
}

#[test]
fn orchwake_c3_hash_cadence_and_recent_work_do_not_hold_the_continuation() {
    for sweep in [
        SweepObservation::new(at(1000), None).with_overview(false, None, None),
        SweepObservation::new(at(1000), None).with_overview(true, None, Some(at(999))),
        SweepObservation::new(at(1000), None).with_working_since(Some(at(999))),
    ] {
        let f = Fixture::new(true);
        f.publish(LAUNCH, Conversation::Kept);
        let mut observed = no_overview();
        observed.sweep = Some(sweep);
        let state = f.tick(&PaneState::default(), &observed, Some(&reading()), false);
        assert_eq!(f.calls(), expected_call());
        assert!(f.body().contains("another model or effort"));
        assert!(!f.body().contains(FLEET));
        assert!(state.sweep.last_sweep.is_none());
        assert_eq!(f.phase(), None);
    }
}

#[test]
fn orchwake_readiness_holds_each_ineligible_pane_until_the_next_clean_cycle() {
    for (name, observed, prior) in [
        (
            "dead shell",
            Observation {
                is_dead: true,
                ..no_overview()
            },
            PaneState::default(),
        ),
        (
            "latched death with no proof of life",
            Observation {
                descendancy: Descendancy::Unknown,
                ..no_overview()
            },
            PaneState {
                dead_latched: true,
                ..PaneState::default()
            },
        ),
        (
            "busy",
            Observation {
                harness: HarnessObservation {
                    frame: crate::harness_state::HarnessState::Busy,
                    ..no_overview().harness
                },
                ..no_overview()
            },
            PaneState::default(),
        ),
        (
            "human draft",
            Observation {
                harness: HarnessObservation {
                    human_draft: true,
                    ..no_overview().harness
                },
                ..no_overview()
            },
            PaneState::default(),
        ),
        (
            "unread capture",
            Observation {
                capture_ok: false,
                ..no_overview()
            },
            PaneState::default(),
        ),
    ] {
        let f = Fixture::new(true);
        f.publish(LAUNCH, carried());
        let state = f.tick(&prior, &observed, None, false);
        assert!(f.calls().is_empty(), "{name}");
        assert_eq!(f.phase(), Some(ContinuationPhase::Owed), "{name}");
        f.tick(&state, &no_overview(), None, false);
        assert_eq!(f.calls(), expected_call(), "{name}");
        assert_eq!(f.phase(), None, "{name}");
    }
}

#[test]
fn orchwake_c4_pre_paste_refusal_reopens_the_turn_without_charging_a_solo_sweep() {
    let f = Fixture::new(true);
    f.publish(LAUNCH, carried());
    f.mode("refused");
    let observed = no_overview();
    let prior = PaneState {
        sweep: crate::watchdog::SweepState {
            fails: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    let refused = f.tick(&prior, &observed, None, false);
    assert_eq!(refused.sweep.fails, 2);
    assert_eq!(f.phase(), Some(ContinuationPhase::Owed));
    assert_eq!(f.calls(), expected_call());
    assert!(
        orchwake_owner_rows(f.dir()).is_empty(),
        "one refusal books no owner alert"
    );
    let mut refused = refused;
    let retry_max = Knobs::default().sweep.retry_max;
    for retry in 0..retry_max {
        refused = f.tick(&refused, &observed, None, false);
        assert_eq!(refused.sweep.fails, 2);
        assert_eq!(f.phase(), Some(ContinuationPhase::Owed));
        if retry == 0 {
            assert_eq!(orchwake_owner_rows(f.dir()).len(), 1);
        }
    }
    let notices = orchwake_owner_rows(f.dir());
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].contains("orchestrator"), "{notices:?}");
    assert!(
        notices[0].contains("human-only prompt on the pane"),
        "{notices:?}"
    );
    let restarted = f.tick(&PaneState::default(), &observed, None, false);
    f.tick(&restarted, &observed, None, false);
    assert_eq!(orchwake_owner_rows(f.dir()), notices);
    f.mode("verified");
    let landed = f.tick(&refused, &observed, None, false);
    assert_eq!(landed.sweep.fails, 2);
    assert_eq!(
        f.calls(),
        vec![expected_call()[0].clone(); retry_max as usize + 4]
    );
    assert_eq!(f.phase(), None);
}

#[test]
fn orchwake_a_new_dialog_is_named_without_treating_warning_redraws_as_new() {
    let f = Fixture::new(true);
    let login = orchwake_seat(Some(orchwake_report_only()));
    let first = f.tick(&PaneState::default(), &login, None, false);
    let named_login = f.tick(&first, &login, None, false);
    let notices = orchwake_owner_rows(f.dir());
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].contains("Select login method"));

    let mut trust_prompt = prompt();
    trust_prompt.warnings = vec!["The first read shows this warning.".to_owned()];
    let trust = orchwake_seat(Some(trust_prompt.clone()));
    let first = f.tick(&named_login, &trust, None, false);
    assert_eq!(orchwake_owner_rows(f.dir()), notices);

    trust_prompt.warnings = vec!["The second read wraps the warning differently.".to_owned()];
    let trust = orchwake_seat(Some(trust_prompt.clone()));
    let named_trust = f.tick(&first, &trust, None, false);
    let notices = orchwake_owner_rows(f.dir());
    assert_eq!(notices.len(), 2, "{notices:?}");
    assert!(notices[1].contains("Do you trust the contents of this project?"));

    trust_prompt.warnings = vec!["A later redraw shows a different warning.".to_owned()];
    let redraw = orchwake_seat(Some(trust_prompt));
    let _ = f.tick(&named_trust, &redraw, None, false);
    assert_eq!(
        read_events(f.dir())
            .iter()
            .filter(|event| event.action == "human-prompt")
            .count(),
        2
    );
    let restarted = f.tick(&PaneState::default(), &redraw, None, false);
    f.tick(&restarted, &redraw, None, false);
    let named = read_events(f.dir())
        .into_iter()
        .filter(|event| event.action == "human-prompt")
        .count();
    assert_eq!(named, 3, "the restart may rebook the naming");
    assert_eq!(orchwake_owner_rows(f.dir()), notices);
}

#[test]
fn orchwake_c5_ambiguous_turns_are_spent_and_tell_the_owner_once() {
    for mode in ["absent", "pending", "unverifiable", "pasted", "crash"] {
        let f = Fixture::new(true);
        f.publish(LAUNCH, carried());
        let observed = no_overview();
        if mode == "crash" {
            assert!(
                meta::advance_reseat_continuation(
                    f.dir(),
                    &f.obligation().expect("owed"),
                    ContinuationStep::Claim,
                )
                .expect("interrupted writer took the claim")
            );
        } else {
            f.mode(mode);
            f.tick(&PaneState::default(), &observed, None, false);
        }
        f.mode("verified");
        for _ in 0..3 {
            f.tick(&PaneState::default(), &observed, None, false);
        }
        assert_eq!(f.phase(), Some(ContinuationPhase::Claimed), "{mode}");
        assert_eq!(f.calls().len(), usize::from(mode != "crash"), "{mode}");
        let owner = orchwake_owner_rows(f.dir());
        assert_eq!(owner.len(), 1, "{mode}: {owner:?}");
        assert!(owner[0].contains("Check the pane"), "{owner:?}");
    }
}

#[test]
fn orchwake_c6_restart_recovers_owed_and_remembers_closed() {
    let f = Fixture::new(true);
    f.publish(LAUNCH, Conversation::Kept);
    f.tick(
        &PaneState::default(),
        &orchwake_seat(Some(orchwake_report_only())),
        None,
        false,
    );
    assert_eq!(f.phase(), Some(ContinuationPhase::Owed));
    f.tick(&PaneState::default(), &no_overview(), None, false);
    assert_eq!(f.phase(), None);
    f.tick(&PaneState::default(), &no_overview(), None, false);
    assert_eq!(f.calls(), expected_call());
}

#[test]
fn orchwake_c7_successor_rejects_an_old_send_and_preserves_its_own_row() {
    for phase in [ContinuationPhase::Owed, ContinuationPhase::Claimed] {
        let f = Fixture::new(true);
        f.publish(LAUNCH, carried());
        if phase == ContinuationPhase::Claimed {
            assert!(
                meta::advance_reseat_continuation(
                    f.dir(),
                    &f.obligation().expect("owed"),
                    ContinuationStep::Claim,
                )
                .expect("claim old launch")
            );
        }
        let cycle = f.cycle(LAUNCH);
        let stale = cycle.read_reseat_continuation().expect("old snapshot");
        f.publish(SUCCESSOR, Conversation::Kept);
        let successor = f.obligation();
        assert_eq!(successor.as_ref().expect("new turn").launch, SUCCESSOR);
        assert!(cycle.read_reseat_continuation().is_none());
        f.apply(
            &cycle,
            Some(stale),
            &PaneState::default(),
            &no_overview(),
            None,
            false,
        );
        assert!(f.calls().is_empty());
        assert!(orchwake_owner_rows(f.dir()).is_empty());
        assert_eq!(f.obligation(), successor);

        meta::rewrite(f.dir(), "launch_id.main", Some(INITIAL))
            .expect("a new launch with no automatic move may leave the old row");
        assert!(f.obligation().is_none(), "a superseded row is inert");
        assert!(f.cycle(INITIAL).read_reseat_continuation().is_none());
    }
}

#[test]
fn orchwake_c8_seeded_moves_skip_and_ordinary_moves_keep_the_leg_attempt() {
    let seeded = Fixture::new(true);
    seeded.publish(
        LAUNCH,
        Conversation::Fresh {
            id: SUCCESSOR,
            capture_floor: 0,
        },
    );
    assert_eq!(seeded.leg_nudge(Fate::Seeded), None);
    seeded.tick(&PaneState::default(), &no_overview(), None, false);
    assert_eq!(seeded.phase(), None);
    assert!(seeded.calls().is_empty());

    let ordinary = Fixture::new(false);
    ordinary.publish(LAUNCH, carried());
    assert_eq!(ordinary.leg_nudge(Fate::Carried), Some(Nudge::Submitted));
    ordinary.tick(&PaneState::default(), &no_overview(), None, false);
    assert_eq!(ordinary.phase(), None);
    assert_eq!(ordinary.calls(), expected_call());
    let events = read_events(ordinary.dir());
    assert_eq!(events[0].action, NOTICE_ACTION);
}

#[test]
fn orchwake_c9_the_clean_move_leaves_its_one_turn_to_the_watchdog() {
    let f = Fixture::new(true);
    f.publish(LAUNCH, carried());
    assert_eq!(f.leg_nudge(Fate::Carried), Some(Nudge::Owed));
    assert!(f.calls().is_empty());
    let state = f.tick(&PaneState::default(), &no_overview(), None, false);
    f.tick(&state, &no_overview(), None, false);
    assert_eq!(f.calls(), expected_call());
    assert_eq!(f.phase(), None);
    // A charter still owed to this launch remains claimable after its turn.
    assert!(
        meta::claim_charter(f.dir(), "main", LAUNCH, crate::config::SEAT_CHARTER_VERSION)
            .expect("continuation did not consume the charter")
    );
}
