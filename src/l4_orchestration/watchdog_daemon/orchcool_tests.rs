//! The orchestrator's policy cooldown delays the next ADMISSION and nothing else:
//! the verdict of a persistent block is told on the next observation, so a
//! quota episode that re-arms during the wait never loses its notice.

use super::{AutoAct, AutoSeat, auto_acts};
use crate::autoreseat::{
    ATTEMPT_ACTION, CauseClass, Frame, HEADROOM_ACTION, HELD_ACTION, POLICY_HOLD_WAIT_SECS, Pane,
    Room, Settings, StartRead, Switch, Window,
};
use crate::events::Event;
use crate::time::Timestamp;

const ACTOR: &str = "watchdog";
const CAUSE: &str = "headroom 96% weekly_all 7d";

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

fn seat(stays: Option<&str>) -> AutoSeat {
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

#[test]
fn the_cooldown_delays_the_admission_but_never_the_notice() {
    let key = Timestamp::from_epoch(100);
    let held = crate::autoreseat::leg::tagged(
        &CauseClass::OrchestratorStays.lead("late: the policy turned off"),
        CAUSE,
    );
    let journal = [
        record(100, HEADROOM_ACTION, &format!(r#","summary":"{CAUSE}""#)),
        record(200, ATTEMPT_ACTION, &format!(r#","ref":"{key}""#)),
        record(
            210,
            HELD_ACTION,
            &format!(r#","ref":"{key}","summary":"{held}""#),
        ),
    ];
    let until = 210 + POLICY_HOLD_WAIT_SECS;
    // The policy still produces no move: the verdict is told while the key cools.
    assert_eq!(
        acts(&seat(Some("the stays notice")), &journal, until - 1),
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
        acts(&seat(None), &journal, until - 1).is_empty(),
        "no admission while the key cools"
    );
    assert_eq!(
        acts(&seat(None), &journal, until),
        [AutoAct::Trigger {
            agent: "one".to_owned()
        }],
        "and the next one is not used up however many holds came before"
    );
}
