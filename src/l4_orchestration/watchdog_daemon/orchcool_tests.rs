//! The orchestrator's policy cooldown delays the next ADMISSION and nothing else:
//! the verdict of a persistent block is told on the next observation, so a
//! quota episode that re-arms during the wait never loses its notice. A hold the
//! leg alone keeps finding is told by its durable count, on the selected key, and
//! never blocks the retry.

use super::{AutoAct, AutoSeat, auto_acts};
use crate::autoreseat::{
    ATTEMPT_ACTION, CauseClass, Frame, HEADROOM_ACTION, HELD_ACTION, POLICY_HOLD_WAIT_SECS, Pane,
    Room, Settings, StartRead, Switch, Window,
};
use crate::events::Event;
use crate::time::Timestamp;

const ACTOR: &str = "watchdog";
const CAUSE: &str = "headroom 96% weekly_all 7d";
const TAIL: &str =
    ". its usage limit is reached. Move it by hand: ae reseat dev one --using opus55x";

fn record(at: i64, action: &str, rest: &str) -> Event {
    Event::parse_line(&format!(
        r#"{{"ts":"{}","actor":"{ACTOR}","action":"{action}","target":"one"{rest}}}"#,
        Timestamp::from_epoch(at)
    ))
    .expect("a journal line")
}

fn crossed() -> Room {
    Room::Crossed(Window {
        judged: 96.0,
        critical: true,
        long: true,
        observed_at: 90,
        resets_at: None,
        status: crate::quota::Status::Fresh,
        label: "weekly_all 7d".to_owned(),
    })
}

fn seat(stays: Option<&str>, tail: Option<&str>) -> AutoSeat {
    AutoSeat {
        slot: "spawned.1".to_owned(),
        agent: "one".to_owned(),
        profile: "sol6x".to_owned(),
        pane: Pane {
            frame: Frame::Clear,
            human_prompt: false,
            client_input: None,
        },
        limited: false,
        room: crossed(),
        derives: false,
        stays: stays.map(str::to_owned),
        held_tail: tail.map(str::to_owned),
        start: StartRead::default(),
        reset: None,
        wake_landed: None,
        armed: None,
    }
}

fn settings() -> Settings {
    Settings {
        switch: Switch::On,
        sessions: None,
        grace_secs: 0,
        map: vec![("sol6x".to_owned(), vec!["opus55x".to_owned()])],
        headroom_at: Some(95),
        headroom_note: None,
        lead: crate::autoreseat::Lead::default(),
        floor: crate::autoreseat::FloorSetting::None,
        floor_note: None,
        tiers: Vec::new(),
        lead_seat: false,
        orchestrator: crate::autoreseat::OrchestratorPolicy::default(),
        notes: Vec::new(),
    }
}

fn acts(seat: &AutoSeat, events: &[Event], now: i64) -> Vec<AutoAct> {
    auto_acts(
        &settings(),
        std::slice::from_ref(seat),
        events,
        ("dev", false),
        now,
        &mut Vec::new(),
    )
}

/// The episode opened at 100 and `holds` attempts of it held, each 400 s apart.
fn held_journal(holds: i64, why: &str) -> Vec<Event> {
    let key = Timestamp::from_epoch(100);
    let held = crate::autoreseat::leg::tagged(&CauseClass::OrchestratorStays.lead(why), CAUSE);
    let mut journal = vec![record(
        100,
        HEADROOM_ACTION,
        &format!(r#","summary":"{CAUSE}""#),
    )];
    for turn in 0..holds {
        journal.push(record(
            200 + turn * 400,
            ATTEMPT_ACTION,
            &format!(r#","ref":"{key}""#),
        ));
        journal.push(record(
            210 + turn * 400,
            HELD_ACTION,
            &format!(r#","ref":"{key}","summary":"{held}""#),
        ));
    }
    journal
}

#[test]
fn the_cooldown_delays_the_admission_but_never_the_notice() {
    let key = Timestamp::from_epoch(100);
    let journal = held_journal(1, "late: the policy turned off");
    let until = 210 + POLICY_HOLD_WAIT_SECS;
    // The policy still produces no move: the verdict is told while the key cools.
    assert_eq!(
        acts(&seat(Some("the stays notice"), None), &journal, until - 1),
        [AutoAct::Stays {
            slot: "spawned.1".to_owned(),
            agent: "one".to_owned(),
            key,
            body: "the stays notice".to_owned(),
        }],
        "the cooldown never swallows the verdict"
    );
    // The policy now permits a move: it waits out the cooldown, then it opens.
    assert!(
        acts(&seat(None, Some(TAIL)), &journal, until - 1).is_empty(),
        "no admission while the key cools, and one hold is no news"
    );
    assert_eq!(
        acts(&seat(None, Some(TAIL)), &journal, until),
        [AutoAct::Trigger {
            agent: "one".to_owned()
        }],
        "and the next one is not used up however many holds came before"
    );
}

#[test]
fn a_hold_the_leg_keeps_finding_is_told_once_by_count_and_never_blocks_the_retry() {
    let key = Timestamp::from_epoch(100);
    let journal = held_journal(3, "Error: the carry source is missing");
    let last = 210 + 2 * 400;
    let told = AutoAct::Stays {
        slot: "spawned.1".to_owned(),
        agent: "one".to_owned(),
        key,
        body: format!(
            "the move was held 3 times: Error: the carry source is missing ({CAUSE}){TAIL}"
        ),
    };
    // While the key cools: the count is told, no admission.
    assert_eq!(
        acts(&seat(None, Some(TAIL)), &journal, last + 1),
        std::slice::from_ref(&told)
    );
    // Past the cooldown: still told (the journal dedupes it), and the retry opens.
    assert_eq!(
        acts(
            &seat(None, Some(TAIL)),
            &journal,
            last + POLICY_HOLD_WAIT_SECS
        ),
        [
            told,
            AutoAct::Trigger {
                agent: "one".to_owned()
            }
        ]
    );
    // No tail means no move is due: nothing is told.
    assert!(acts(&seat(None, None), &journal, last + 1).is_empty());
    // The verdict of the policy takes precedence and is the only notice.
    assert_eq!(
        acts(
            &seat(Some("the stays notice"), Some(TAIL)),
            &journal,
            last + 1
        ),
        [AutoAct::Stays {
            slot: "spawned.1".to_owned(),
            agent: "one".to_owned(),
            key,
            body: "the stays notice".to_owned(),
        }]
    );
}

#[test]
fn an_observed_hold_carries_the_proof_that_the_launch_and_the_pane_contradict() {
    let said = "held: observed choice Opus 5 stands — an automatic move would drop it";
    assert_eq!(
        super::cycle_observe::with_unproven(
            said.to_owned(),
            Some("the pane draws Opus 5, not claude-sonnet-5")
        ),
        format!(
            "{said}; its running model is not proven (the pane draws Opus 5, not claude-sonnet-5)"
        ),
        "both models are named"
    );
    assert_eq!(
        super::cycle_observe::with_unproven(said.to_owned(), None),
        said,
        "a proven model adds nothing"
    );
}
