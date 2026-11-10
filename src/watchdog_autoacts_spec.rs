//! Characterisation spec for the pure watchdog auto-reseat step.
//!
//! Oracle: docs/internals/watchdog.md (Automatic reseat),
//! docs/internals/reseat.md (Automatic reseat, Headroom episodes, A resume on
//! an exhausted seat), and `auto_acts`' doc comment. Exact ordering and naming
//! memory assertions marked BEHAVIOUR PIN preserve f2b775d6 rather than assert
//! a new policy. No pane, filesystem, process or current decision oracle.

use super::{ACTOR, AutoAct, AutoSeat, auto_acts};
use crate::autoreseat::{
    Frame, HoldReason, LimitReset, Pane, Room, Settings, StartRead, Switch, Trigger, Window,
};
use crate::events::Event;
use crate::time::Timestamp;

fn stamp(at: i64) -> Timestamp {
    Timestamp::from_epoch(at)
}

fn row(at: i64, action: &str, agent: &str, key: Option<i64>, summary: &str) -> Event {
    let reference = key.map_or_else(String::new, |at| format!(r#","ref":"{}""#, stamp(at)));
    Event::parse_line(&format!(
        r#"{{"ts":"{}","actor":"{ACTOR}","action":"{action}","target":"{agent}"{reference},"summary":"{summary}"}}"#,
        stamp(at)
    ))
    .expect("valid fixture journal row")
}

fn settings() -> Settings {
    Settings {
        switch: Switch::On,
        sessions: None,
        grace_secs: 0,
        map: vec![("source".to_owned(), vec!["target".to_owned()])],
        headroom_at: Some(95),
        headroom_note: None,
        lead: crate::autoreseat::Lead::default(),
        floor: crate::autoreseat::FloorSetting::None,
        floor_note: None,
        tiers: Vec::new(),
        lead_seat: false,
        notes: Vec::new(),
    }
}

fn window(judged: f64) -> Window {
    Window {
        judged,
        critical: judged >= 95.0,
        long: true,
        observed_at: 200,
        resets_at: None,
        status: crate::quota::Status::Fresh,
        label: "weekly_all 7d".to_owned(),
    }
}

fn seat(slot: &str, agent: &str) -> AutoSeat {
    AutoSeat {
        slot: slot.to_owned(),
        agent: agent.to_owned(),
        profile: "source".to_owned(),
        pane: Pane {
            frame: Frame::Clear,
            human_prompt: false,
            client_input: None,
        },
        limited: true,
        room: Room::Unread,
        derives: false,
        start: StartRead::default(),
        reset: None,
        wake_landed: None,
        armed: None,
    }
}

fn acts(settings: &Settings, seats: &[AutoSeat], events: &[Event], now: i64) -> Vec<AutoAct> {
    auto_acts(
        settings,
        seats,
        events,
        ("dev", false),
        now,
        &mut Vec::new(),
    )
}

fn trigger(agent: &str) -> AutoAct {
    AutoAct::Trigger {
        agent: agent.to_owned(),
    }
}

fn held(slot: &str, agent: &str, key: i64, reason: HoldReason, trigger: Trigger) -> AutoAct {
    AutoAct::Held {
        slot: slot.to_owned(),
        agent: agent.to_owned(),
        key: stamp(key),
        reason,
        trigger,
    }
}

fn headroom(at: i64, agent: &str) -> Event {
    row(
        at,
        "auto-reseat-headroom",
        agent,
        None,
        "headroom 96% weekly_all 7d",
    )
}

#[test]
fn derived_candidates_bypass_only_the_missing_map() {
    let mut on = settings();
    on.map.clear();
    let mut one = seat("spawned.1", "one");
    let limits = [row(100, "limit", "one", None, "")];
    let unmapped = AutoAct::Unmapped {
        slot: one.slot.clone(),
        agent: one.agent.clone(),
        key: stamp(100),
        profile: one.profile.clone(),
    };
    assert_eq!(acts(&on, &[one.clone()], &limits, 300), [unmapped]);
    one.derives = true;
    assert_eq!(
        acts(&on, &[one.clone()], &limits, 300),
        [trigger("one")],
        "derived limit admitted"
    );
    one.limited = false;
    one.room = Room::Crossed(window(96.0));
    assert_eq!(
        acts(&on, &[one.clone()], &[], 300),
        [AutoAct::Open {
            slot: one.slot.clone(),
            agent: one.agent.clone(),
            cause: "headroom 96% weekly_all 7d".to_owned(),
        }],
        "derived headroom admitted"
    );
    one.derives = false;
    assert!(
        acts(&on, &[one.clone()], &[], 300).is_empty(),
        "unmapped headroom silent"
    );
    one.derives = true;
    one.limited = true;
    for (switch, sessions, slot, orchestrator) in [
        (Switch::Off, None, "spawned.1", false),
        (
            Switch::On,
            Some(vec!["elsewhere".to_owned()]),
            "spawned.1",
            false,
        ),
        (Switch::On, None, "main", false),
        (Switch::On, None, "invalid", false),
        (Switch::All, None, "spawned.1", true),
    ] {
        let settings = Settings {
            switch,
            sessions,
            ..on.clone()
        };
        one.slot = slot.to_owned();
        assert!(
            auto_acts(
                &settings,
                &[one.clone()],
                &limits,
                ("dev", orchestrator),
                300,
                &mut Vec::new()
            )
            .is_empty(),
            "derived never overrides switch/session/class/slot/orchestrator: {settings:?}, {slot}, {orchestrator}"
        );
    }
}

/// BEHAVIOUR PIN: the latch selects the limit path. A residual journal limit
/// alone must not name an unmapped seat whose current latch does not stand.
#[test]
fn an_unlatched_unmapped_seat_does_not_name_a_residual_limit_episode() {
    let mut on = settings();
    on.map.clear();
    let mut one = seat("spawned.1", "one");
    one.limited = false;
    let events = [row(100, "limit", "one", None, "")];
    for room in [Room::Unread, Room::Crossed(window(96.0))] {
        one.room = room;
        assert!(
            acts(&on, std::slice::from_ref(&one), &events, 300).is_empty(),
            "unmapped unlatched seat is silent despite residual limit: {:?}",
            one.room
        );
    }
    one.limited = true;
    assert_eq!(
        acts(&on, std::slice::from_ref(&one), &events, 300),
        [AutoAct::Unmapped {
            slot: "spawned.1".to_owned(),
            agent: "one".to_owned(),
            key: stamp(100),
            profile: "source".to_owned(),
        }],
        "latched control names its missing candidates"
    );
    one.derives = true;
    assert_eq!(
        acts(&on, &[one], &events, 300),
        [trigger("one")],
        "derived admission still outranks missing-map naming"
    );
}

#[test]
fn unmapped_naming_obeys_its_episode_filters_and_plain_grace() {
    let mut on = settings();
    on.map.clear();
    on.grace_secs = 600;
    let mut one = seat("spawned.1", "one");
    // BEHAVIOUR PIN B1: an unmapped seat uses plain grace even when its
    // exhausted resume would waive grace for an admitted seat.
    one.start = StartRead {
        settled: Some(100),
        exhausted: Some(window(100.0)),
        ..StartRead::default()
    };
    let limit = row(100, "limit", "one", None, "");
    let expected = AutoAct::Unmapped {
        slot: one.slot.clone(),
        agent: one.agent.clone(),
        key: stamp(100),
        profile: one.profile.clone(),
    };
    for (name, events, now, emits) in [
        ("missing", vec![], 700, false),
        ("plain grace", vec![limit.clone()], 300, false),
        ("due", vec![limit.clone()], 700, true),
        (
            "in flight",
            vec![limit.clone(), row(650, "auto-reseat", "one", Some(100), "")],
            700,
            false,
        ),
        (
            "overdue is not unmapped",
            vec![limit.clone(), row(110, "auto-reseat", "one", Some(100), "")],
            700,
            false,
        ),
        (
            "terminal",
            vec![
                limit.clone(),
                row(120, "auto-reseat-refused", "one", Some(100), ""),
            ],
            3000,
            false,
        ),
        (
            "already said",
            vec![
                limit.clone(),
                row(
                    200,
                    "auto-reseat-held",
                    "one",
                    Some(100),
                    "no-candidate: held: no declared or derived candidate for this profile",
                ),
            ],
            700,
            false,
        ),
    ] {
        let expected = if emits {
            vec![expected.clone()]
        } else {
            vec![]
        };
        assert_eq!(acts(&on, &[one.clone()], &events, now), expected, "{name}");
    }
    one.pane.frame = Frame::Busy;
    assert_eq!(
        acts(&on, &[one.clone()], std::slice::from_ref(&limit), 700),
        [expected],
        "busy names missing candidate"
    );
    one.pane.frame = Frame::Clear;
    one.derives = true;
    assert_eq!(
        acts(&on, &[one], &[limit], 300),
        [trigger("one")],
        "admitted urgent control"
    );
}

fn urgent_seat() -> AutoSeat {
    let mut one = seat("spawned.1", "one");
    one.limited = false;
    // Resume exhaustion may come from a short window while the weekly
    // headroom reading is relieved. Both reads can stand in the same cycle.
    let exhausted = Window {
        long: false,
        label: "session_all 5h".to_owned(),
        ..window(100.0)
    };
    one.start = StartRead {
        settled: Some(200),
        exhausted: Some(exhausted),
        ..StartRead::default()
    };
    one
}

#[test]
fn urgent_opening_and_earlier_episode_rearm_precede_room_readings() {
    let mut on = settings();
    on.headroom_at = None;
    let one = urgent_seat();
    let opening = AutoAct::Open {
        slot: one.slot.clone(),
        agent: one.agent.clone(),
        cause: "headroom 100% session_all 5h".to_owned(),
    };
    for room in [
        Room::Unread,
        Room::Crossed(window(96.0)),
        Room::Relieved(window(10.0)),
    ] {
        let mut one = one.clone();
        one.room = room;
        assert_eq!(
            acts(&on, &[one.clone()], &[], 300),
            std::slice::from_ref(&opening),
            "urgent crossing precedence"
        );
        assert_eq!(
            acts(&on, &[one], &[headroom(199, "one")], 300),
            [AutoAct::Rearm {
                slot: "spawned.1".to_owned(),
                agent: "one".to_owned(),
                key: stamp(199),
                summary: "headroom cleared: a new start reads its own account".to_owned(),
            }],
            "older episode closes before relief/threshold-off"
        );
    }
    for key in [200, 201] {
        let mut relieved = one.clone();
        relieved.room = Room::Relieved(window(10.0));
        assert_eq!(
            acts(&on, &[relieved], &[headroom(key, "one")], 300),
            [trigger("one")],
            "same/new start's episode remains urgent even with threshold off and room relieved"
        );
    }
    let mut ordinary = one;
    ordinary.start.exhausted = None;
    assert!(
        acts(&on, &[ordinary.clone()], &[], 300).is_empty(),
        "no exhaustion cannot open urgently"
    );
    assert!(
        acts(&on, &[ordinary], &[headroom(200, "one")], 300).is_empty(),
        "no threshold and nonurgent episode silent"
    );
}

#[test]
fn urgent_frame_proof_waives_only_the_allowed_holds() {
    let mut on = settings();
    on.grace_secs = 600;
    on.headroom_at = None;
    let events = [headroom(200, "one")];
    for (frame, quiet, prompting, input, reason) in [
        (Frame::Unproven, true, false, None, None),
        (
            Frame::Unproven,
            false,
            false,
            None,
            Some(HoldReason::Unproven),
        ),
        (Frame::Busy, true, false, None, Some(HoldReason::Busy)),
        (Frame::Draft, true, false, None, Some(HoldReason::Draft)),
        (Frame::Unread, true, false, None, Some(HoldReason::Unread)),
        (
            Frame::Unproven,
            true,
            true,
            None,
            Some(HoldReason::HumanPrompt),
        ),
        (
            Frame::Clear,
            true,
            false,
            Some(299),
            Some(HoldReason::ClientInput),
        ),
    ] {
        let mut one = urgent_seat();
        one.start.quiet = quiet;
        one.pane = Pane {
            frame,
            human_prompt: prompting,
            client_input: input,
        };
        let expected = reason.map_or_else(
            || trigger("one"),
            |why| held("spawned.1", "one", 200, why, Trigger::Headroom),
        );
        assert_eq!(
            acts(&on, &[one], &events, 300),
            [expected],
            "{frame:?}, quiet={quiet}, prompt={prompting}, input={input:?}"
        );
    }
    let mut ordinary = urgent_seat();
    ordinary.start.brief_armed = true;
    ordinary.start.quiet = true;
    ordinary.pane.frame = Frame::Unproven;
    assert!(
        acts(&on, &[ordinary], &events, 300).is_empty(),
        "armed brief blocks urgency; threshold off skips"
    );
}

#[test]
fn hold_memory_drops_removed_slots_and_old_keys_even_for_a_journaled_hold() {
    let on = settings();
    let mut one = seat("spawned.1", "one");
    one.pane.frame = Frame::Busy;
    let events = [row(100, "limit", "one", None, "")];
    let mut named = vec![("spawned.2".to_owned(), stamp(90), HoldReason::Draft)];
    assert_eq!(
        auto_acts(
            &on,
            &[one.clone()],
            &events,
            ("dev", false),
            300,
            &mut named
        ),
        [held(
            "spawned.1",
            "one",
            100,
            HoldReason::Busy,
            Trigger::Limit
        )]
    );
    assert_eq!(
        named,
        [("spawned.1".to_owned(), stamp(100), HoldReason::Busy)],
        "removed slot pruned"
    );
    assert!(auto_acts(&on, &[], &events, ("dev", false), 300, &mut named).is_empty());
    assert!(named.is_empty(), "empty roster forgets every slot");
    assert_eq!(
        auto_acts(
            &on,
            &[one.clone()],
            &events,
            ("dev", false),
            300,
            &mut named
        ),
        [held(
            "spawned.1",
            "one",
            100,
            HoldReason::Busy,
            Trigger::Limit
        )],
        "re-added slot names again"
    );
    one.limited = false;
    one.room = Room::Crossed(window(96.0));
    let events = [
        headroom(200, "one"),
        row(
            250,
            "auto-reseat-held",
            "one",
            Some(200),
            "busy: held: the seat is busy (headroom 96% weekly_all 7d)",
        ),
    ];
    named.push(("spawned.1".to_owned(), stamp(200), HoldReason::Draft));
    assert!(
        auto_acts(&on, &[one], &events, ("dev", false), 300, &mut named).is_empty(),
        "journal already names busy"
    );
    // BEHAVIOUR PIN: pruning happens even when the journal suppresses the act;
    // same-key OTHER reasons remain, while the old episode is forgotten.
    assert_eq!(
        named,
        [("spawned.1".to_owned(), stamp(200), HoldReason::Draft)]
    );
}

#[test]
fn mixed_limit_and_headroom_arbitration_is_independent_of_seat_order() {
    let on = settings();
    let one = seat("spawned.1", "one");
    let mut two = seat("spawned.2", "two");
    two.limited = false;
    two.room = Room::Crossed(window(96.0));
    for reversed in [false, true] {
        let seats = if reversed {
            vec![two.clone(), one.clone()]
        } else {
            vec![one.clone(), two.clone()]
        };
        for (limit_key, headroom_key, winner) in
            [(100, 90, "two"), (90, 100, "one"), (100, 100, "one")]
        {
            let mut events = [
                row(limit_key, "limit", "one", None, ""),
                headroom(headroom_key, "two"),
            ];
            events.sort_by_key(|event| event.ts);
            assert_eq!(
                acts(&on, &seats, &events, 300),
                [trigger(winner)],
                "reverse={reversed}, limit={limit_key}, headroom={headroom_key}"
            );
        }
    }
}

fn spent(agent: &str, key: i64) -> Vec<Event> {
    vec![
        row(key, "limit", agent, None, ""),
        row(key + 10, "auto-reseat-refused", agent, Some(key), ""),
        row(key + 1810, "auto-reseat-failed", agent, Some(key), ""),
        row(key + 3610, "auto-reseat-refused", agent, Some(key), ""),
    ]
}

#[test]
fn mixed_actions_keep_roster_order_and_append_the_one_trigger_last() {
    let mut busy = seat("spawned.2", "busy");
    busy.pane.frame = Frame::Busy;
    let mut missing = seat("spawned.3", "missing");
    missing.profile = "unmapped".to_owned();
    let mut opening = seat("spawned.4", "opening");
    opening.limited = false;
    opening.room = Room::Crossed(window(96.0));
    let mut rearm = seat("spawned.5", "relieved");
    rearm.limited = false;
    rearm.room = Room::Relieved(window(10.0));
    let mut events = vec![
        row(100, "limit", "due", None, ""),
        row(110, "limit", "busy", None, ""),
        row(120, "limit", "missing", None, ""),
        headroom(130, "relieved"),
    ];
    events.extend(spent("spent", 140));
    events.extend([
        row(150, "limit", "overdue", None, ""),
        row(160, "auto-reseat", "overdue", Some(150), ""),
    ]);
    events.sort_by_key(|event| event.ts);
    let mut exhausted = seat("spawned.6", "spent");
    exhausted.reset = Some(LimitReset {
        at: 10_100,
        short: true,
    });
    let seats = [
        seat("spawned.1", "due"),
        busy,
        missing,
        opening,
        rearm,
        exhausted,
        seat("spawned.7", "overdue"),
    ];
    // BEHAVIOUR PIN: roster-order records precede the trigger even when the
    // selected seat is first. Give-up is named before a near-reset silent wait.
    assert_eq!(
        acts(&settings(), &seats, &events, 10_000),
        [
            held("spawned.2", "busy", 110, HoldReason::Busy, Trigger::Limit),
            AutoAct::Unmapped {
                slot: "spawned.3".to_owned(),
                agent: "missing".to_owned(),
                key: stamp(120),
                profile: "unmapped".to_owned()
            },
            AutoAct::Open {
                slot: "spawned.4".to_owned(),
                agent: "opening".to_owned(),
                cause: "headroom 96% weekly_all 7d".to_owned()
            },
            AutoAct::Rearm {
                slot: "spawned.5".to_owned(),
                agent: "relieved".to_owned(),
                key: stamp(130),
                summary: "headroom cleared: 10% weekly_all 7d".to_owned()
            },
            AutoAct::GaveUp {
                slot: "spawned.6".to_owned(),
                agent: "spent".to_owned(),
                key: stamp(140),
                profile: "source".to_owned()
            },
            AutoAct::Overdue {
                slot: "spawned.7".to_owned(),
                agent: "overdue".to_owned(),
                key: stamp(150),
                trigger: Trigger::Limit
            },
            trigger("due"),
        ]
    );
}
