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

/// An unmapped seat its own account presses into a headroom episode is that
/// episode's, never the return's, even with the origin ready.
#[test]
fn an_unmapped_seat_with_a_headroom_episode_never_opens_a_return() {
    let pressed = [record(NOW - 60, crate::autoreseat::HEADROOM_ACTION, "")];
    assert!(
        acts(
            &settings(Switch::On, &[]),
            &seat(Room::Unread),
            &home(Some(ready())),
            &pressed
        )
        .is_empty()
    );
}

/// An unmapped seat's return attempt past its bound is booked overdue like
/// any other; a limit attempt in flight on it is not the return's to decide.
#[test]
fn an_unmapped_seats_return_attempt_past_its_bound_is_overdue() {
    let key = Timestamp::from_epoch(NOW - 600);
    let reference = format!(r#","ref":"{key}""#);
    let overdue = [
        record(key.epoch(), RETURN_ACTION, ""),
        record(key.epoch() + 1, ATTEMPT_ACTION, &reference),
    ];
    assert_eq!(
        acts(
            &settings(Switch::On, &[]),
            &seat(Room::Unread),
            &home(Some(ready())),
            &overdue
        ),
        [AutoAct::Overdue {
            slot: SLOT.to_owned(),
            agent: AGENT.to_owned(),
            key,
            trigger: crate::autoreseat::Trigger::Return,
        }]
    );
    let limit = [
        record(key.epoch(), "limit", ""),
        record(key.epoch() + 1, ATTEMPT_ACTION, &reference),
    ];
    assert!(
        acts(
            &settings(Switch::On, &[]),
            &seat(Room::Unread),
            &home(Some(ready())),
            &limit
        )
        .is_empty()
    );
}

/// A mapped seat with a headroom episode still standing goes home only once
/// that episode is gone, even when the threshold is off and its own route
/// therefore decides nothing (M5: limit, then headroom, then return).
#[test]
fn a_standing_headroom_episode_holds_a_mapped_seats_return() {
    let mut off = settings(Switch::On, &[("substitute", &["manual"])]);
    off.headroom_at = None;
    let standing = [record(NOW - 60, crate::autoreseat::HEADROOM_ACTION, "")];
    assert!(acts(&off, &seat(Room::Unread), &home(Some(ready())), &standing).is_empty());
    assert_eq!(
        acts(&off, &seat(Room::Unread), &home(Some(ready())), &[]),
        [opened()],
        "with no episode the same seat goes home"
    );
}

/// Past the flap limit a ready origin is told once, never opened.
#[test]
fn a_flapped_seat_is_held_not_returned() {
    let mut flapped = home(Some(ready()));
    flapped[0].1.flapped = true;
    assert_eq!(
        acts(
            &settings(Switch::On, &[]),
            &seat(Room::Unread),
            &flapped,
            &[]
        ),
        [AutoAct::Flapped {
            slot: SLOT.to_owned(),
            agent: AGENT.to_owned(),
            key: Timestamp::from_epoch(NOW - 600),
            origin: "origin".to_owned(),
        }]
    );
}

/// An open return is decided only while its proof stands; once the origin no
/// longer proves room it waits silently, unless its rounds are spent, which is
/// told as every spent path is.
#[test]
fn an_open_return_moves_only_on_its_proof_and_a_spent_one_gives_up() {
    use crate::autoreseat::{MAX_ROUNDS, REFUSED_ACTION, RETRY_COOLDOWN_SECS};
    let mapped = settings(Switch::On, &[("substitute", &["manual"])]);
    let key = Timestamp::from_epoch(NOW - 20_000);
    let reference = format!(r#","ref":"{key}""#);
    let open = [record(key.epoch(), RETURN_ACTION, "")];
    assert!(
        acts(&mapped, &seat(Room::Unread), &home(None), &open).is_empty(),
        "unproven: no attempt"
    );
    let mut spent = open.to_vec();
    let mut at = key.epoch();
    for _ in 0..MAX_ROUNDS {
        spent.push(record(at + 1, ATTEMPT_ACTION, &reference));
        spent.push(record(at + 2, REFUSED_ACTION, &reference));
        at += 2 + RETRY_COOLDOWN_SECS;
    }
    assert_eq!(
        acts(&mapped, &seat(Room::Unread), &home(None), &spent),
        [AutoAct::GaveUp {
            slot: SLOT.to_owned(),
            agent: AGENT.to_owned(),
            key,
            profile: "substitute".to_owned(),
        }]
    );
}
