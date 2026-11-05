//! The daemon's return route (#305): which seats on an automatic chain the
//! auto reseat step opens, decides or leaves alone.

use super::{AutoAct, AutoSeat, auto_acts_with};
use crate::autoreseat::leg::Home;
use crate::autoreseat::{
    ATTEMPT_ACTION, Frame, Pane, RETURN_ACTION, Room, Settings, StartRead, Switch, Window,
};
use crate::events::Event;
use crate::time::Timestamp;

const SESSION: &str = "aedev";
const SLOT: &str = "spawned.1";
const AGENT: &str = "one";
const NOW: i64 = 1_800_000_000;

/// Auto reseat on, no grace, and `map` as the `[auto_reseat]` rows.
fn settings(switch: Switch, map: &[(&str, &[&str])]) -> Settings {
    Settings {
        switch,
        sessions: None,
        grace_secs: 0,
        map: map
            .iter()
            .map(|(from, to)| {
                (
                    (*from).to_owned(),
                    to.iter().map(|name| (*name).to_owned()).collect(),
                )
            })
            .collect(),
        headroom_at: Some(95),
        headroom_note: None,
        notes: Vec::new(),
    }
}

/// An idle seat on `substitute` that no family derives and no limit holds.
fn seat(room: Room) -> AutoSeat {
    AutoSeat {
        slot: SLOT.to_owned(),
        agent: AGENT.to_owned(),
        profile: "substitute".to_owned(),
        pane: Pane {
            frame: Frame::Clear,
            human_prompt: false,
            client_input: None,
        },
        limited: false,
        room,
        derives: false,
        start: StartRead::default(),
        reset: None,
        wake_landed: None,
        armed: None,
    }
}

fn ready() -> Window {
    Window {
        judged: 20.0,
        critical: false,
        long: true,
        observed_at: NOW - 30,
        resets_at: None,
        status: crate::quota::Status::Fresh,
        label: "weekly_all 7d".to_owned(),
    }
}

fn home(ready: Option<Window>) -> Vec<(String, Home)> {
    vec![(
        SLOT.to_owned(),
        Home {
            origin: "origin".to_owned(),
            newest: Timestamp::from_epoch(NOW - 600),
            ready,
            flapped: false,
        },
    )]
}

fn record(at: i64, action: &str, rest: &str) -> Event {
    Event::parse_line(&format!(
        r#"{{"ts":"{}","actor":"watchdog","action":"{action}","target":"{AGENT}","target_slot":"{SLOT}","target_session":"{SESSION}"{rest},"summary":"return to origin: 20% weekly_all 7d"}}"#,
        Timestamp::from_epoch(at)
    ))
    .expect("a journal line")
}

fn acts(
    settings: &Settings,
    seat: &AutoSeat,
    homes: &[(String, Home)],
    events: &[Event],
) -> Vec<AutoAct> {
    auto_acts_with(
        settings,
        (std::slice::from_ref(seat), homes),
        events,
        (SESSION, false),
        NOW,
        &mut Vec::new(),
    )
}

fn opened() -> AutoAct {
    AutoAct::OpenReturn {
        slot: SLOT.to_owned(),
        agent: AGENT.to_owned(),
        cause: crate::autoreseat::return_words("origin", &ready()),
    }
}

/// The origin is the return's one candidate: a seat whose current profile has
/// no `[auto_reseat]` row, and no family to derive one, still goes home.
#[test]
fn an_unmapped_seat_on_a_chain_opens_and_takes_its_return() {
    let unmapped = settings(Switch::On, &[]);
    let idle = seat(Room::Unread);
    assert_eq!(
        acts(&unmapped, &idle, &home(Some(ready())), &[]),
        [opened()]
    );
    let key = Timestamp::from_epoch(NOW - 60);
    let open = [record(key.epoch(), RETURN_ACTION, "")];
    assert_eq!(
        acts(&unmapped, &idle, &home(Some(ready())), &open),
        [AutoAct::Trigger {
            agent: AGENT.to_owned()
        }]
    );
    // Its own account's pressure opens no headroom move it has no row for.
    let pressed = seat(Room::Crossed(ready()));
    assert_eq!(
        acts(&unmapped, &pressed, &home(Some(ready())), &[]),
        [opened()]
    );
}

/// Unmapped and not on a chain, unproven, or with auto reseat off, the seat is
/// left alone exactly as before.
#[test]
fn an_unmapped_seat_with_no_proven_return_or_the_switch_off_is_left_alone() {
    let idle = seat(Room::Unread);
    assert!(acts(&settings(Switch::On, &[]), &idle, &[], &[]).is_empty());
    assert!(acts(&settings(Switch::On, &[]), &idle, &home(None), &[]).is_empty());
    assert!(
        acts(
            &settings(Switch::Off, &[]),
            &idle,
            &home(Some(ready())),
            &[]
        )
        .is_empty()
    );
    // A return attempt in flight decides the seat until it ends.
    let key = Timestamp::from_epoch(NOW - 60);
    let flying = [
        record(key.epoch(), RETURN_ACTION, ""),
        record(
            key.epoch() + 1,
            ATTEMPT_ACTION,
            &format!(r#","ref":"{key}""#),
        ),
    ];
    assert!(
        acts(
            &settings(Switch::On, &[]),
            &idle,
            &home(Some(ready())),
            &flying
        )
        .is_empty()
    );
}

/// A mapped seat goes home the same way: the row is never what decides it.
#[test]
fn a_mapped_seat_on_a_chain_opens_its_return() {
    let mapped = settings(Switch::On, &[("substitute", &["manual"])]);
    assert_eq!(
        acts(&mapped, &seat(Room::Unread), &home(Some(ready())), &[]),
        [opened()]
    );
}
