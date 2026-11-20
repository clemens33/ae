//! Frozen characterisation of the one-pass episode fold, before extraction.
//! Oracle: `episodes`' contract and docs/internals/reseat.md, automatic reseat
//! and headroom episodes. Explicit BASE PINs preserve unspecified base output.
//! No scratch state or external process: every input is a parsed journal record.

use super::{
    ATTEMPT_ACTION, CLEARED_ACTION, DONE_ACTION, Episode, FAILED_ACTION, HEADROOM_ACTION,
    HELD_ACTION, LIMIT_ACTION, Outcome, REARMED_ACTION, REFUSED_ACTION, SPAWN_ACTION, Trigger,
    episodes, left_on_headroom,
};
use crate::events::Event;
use crate::time::Timestamp;

const SESSION: &str = "spec-session";
const SLOT: &str = "spawned.7";
const AGENT: &str = "scout";
// Independent contract pins: 30-minute retry, three refused/failed rounds.
const COOLDOWN: i64 = 1_800;

fn stamp(second: i64) -> Timestamp {
    Timestamp::from_epoch(1_700_000_000 + second)
}

fn event(second: i64, action: &str, key: Option<i64>) -> Event {
    let reference = key
        .map(|key| format!(r#", "ref":"{}""#, stamp(key)))
        .unwrap_or_default();
    Event::parse_line(&format!(
        r#"{{"ts":"{}","actor":"watchdog","action":"{action}","target":"{AGENT}"{reference}}}"#,
        stamp(second)
    ))
    .expect("valid synthetic journal record")
}

// Expected states come from the written rules, never Episode::opened/absorb.
fn wanted(key: i64, trigger: Trigger) -> Episode {
    Episode {
        key: stamp(key),
        reference: stamp(key).to_string(),
        trigger,
        cause: String::new(),
        attempts: 0,
        rounds: 0,
        closed_at: None,
        open: None,
        terminal: None,
        held: false,
        last_hold: None,
        choice_held: false,
    }
}

fn folded(events: &[Event]) -> (Option<Episode>, Option<Episode>) {
    episodes(events, SESSION, SLOT, AGENT)
}

fn both() -> Vec<Event> {
    vec![
        event(0, LIMIT_ACTION, None),
        event(10, HEADROOM_ACTION, None),
    ]
}

#[test]
fn address_and_actor_precede_every_action_but_addressed_spawn_precedes_actor() {
    let actions = [
        SPAWN_ACTION,
        LIMIT_ACTION,
        CLEARED_ACTION,
        HEADROOM_ACTION,
        REARMED_ACTION,
        DONE_ACTION,
        ATTEMPT_ACTION,
        HELD_ACTION,
        REFUSED_ACTION,
        FAILED_ACTION,
    ];
    for action in actions {
        // Every guarded action must otherwise change the chosen seed. An
        // opener beside an existing episode would make its guard test vacuous.
        let (base, expected) = if matches!(action, LIMIT_ACTION | HEADROOM_ACTION) {
            (Vec::new(), (None, None))
        } else {
            (
                both(),
                (
                    Some(wanted(0, Trigger::Limit)),
                    Some(wanted(10, Trigger::Headroom)),
                ),
            )
        };
        for target in ["elsewhere", "@elsewhere:scout"] {
            let mut record = event(30, action, Some(10));
            record.target = Some(target.to_owned());
            let mut journal = base.clone();
            journal.push(record);
            assert_eq!(
                folded(&journal),
                expected,
                "foreign target {action} {target}"
            );
        }
        // A complete wrong route wins over a matching display name; a partial
        // route is also unassociated, never a display fallback.
        for route in [
            r#", "target_slot":"spawned.8","target_session":"spec-session""#,
            r#", "target_slot":"spawned.7","target_session":"elsewhere""#,
            r#", "target_slot":"spawned.7""#,
        ] {
            let record = Event::parse_line(&format!(
                r#"{{"ts":"{}","actor":"watchdog","action":"{action}","target":"{AGENT}","ref":"{}"{route}}}"#,
                stamp(30), stamp(10)
            )).expect("valid foreign route");
            let mut journal = base.clone();
            journal.push(record);
            assert_eq!(folded(&journal), expected, "foreign route {action} {route}");
        }
        for actor in ["lead", "human", AGENT] {
            let mut record = event(30, action, Some(10));
            record.actor = actor.to_owned();
            let mut journal = base.clone();
            journal.push(record);
            let result = if action == SPAWN_ACTION {
                (None, None)
            } else {
                expected.clone()
            };
            assert_eq!(folded(&journal), result, "actor {actor} {action}");
        }
    }
    // Correct routing keeps addressing the seat after its display was renamed.
    let text = format!(
        r#"{{"ts":"{}","actor":"lead","action":"spawn","target":"old-name","target_slot":"{SLOT}","target_session":"{SESSION}"}}"#,
        stamp(30)
    );
    let mut journal = both();
    journal.push(Event::parse_line(&text).expect("routed spawn"));
    assert_eq!(folded(&journal), (None, None), "routed spawn before actor");
}

#[test]
fn openers_preserve_first_key_and_cause_even_after_a_terminal() {
    for closing in [
        None,
        Some(REFUSED_ACTION),
        Some(FAILED_ACTION),
        Some(DONE_ACTION),
    ] {
        for headroom_first in [false, true] {
            let mut first = event(10, HEADROOM_ACTION, None);
            first.summary = Some("headroom 97% weekly".to_owned());
            let limit = event(0, LIMIT_ACTION, None);
            let mut journal = if headroom_first {
                vec![first, limit]
            } else {
                vec![limit, first]
            };
            let mut expected_limit = wanted(0, Trigger::Limit);
            let mut expected_room = wanted(10, Trigger::Headroom);
            expected_room.cause = "headroom 97% weekly".to_owned();
            if let Some(action) = closing {
                journal.push(event(20, action, Some(0)));
                let outcome = match action {
                    REFUSED_ACTION => Outcome::Refused,
                    FAILED_ACTION => Outcome::Failed,
                    _ => Outcome::Done,
                };
                expected_limit.terminal = Some(outcome);
                expected_room.terminal = Some(outcome);
                if outcome == Outcome::Done {
                    // The move removes headroom, so the next crossing opens it.
                    expected_room = wanted(40, Trigger::Headroom);
                } else {
                    expected_limit.rounds = 1;
                    expected_limit.closed_at = Some(stamp(20));
                    expected_room.rounds = 1;
                    expected_room.closed_at = Some(stamp(20));
                }
            }
            journal.extend([
                event(30, LIMIT_ACTION, None),
                event(40, HEADROOM_ACTION, None),
            ]);
            assert_eq!(
                folded(&journal),
                (Some(expected_limit), Some(expected_room)),
                "{closing:?} headroom_first={headroom_first}"
            );
        }
    }
}

#[test]
fn clear_and_rearm_are_independent_with_distinct_or_shared_keys() {
    for room_key in [0, 10] {
        for terminal in [false, true] {
            let mut base = vec![
                event(0, LIMIT_ACTION, None),
                event(room_key, HEADROOM_ACTION, None),
            ];
            let mut limit = wanted(0, Trigger::Limit);
            let mut room = wanted(room_key, Trigger::Headroom);
            if terminal {
                base.push(event(20, REFUSED_ACTION, Some(0)));
                for expected in [&mut limit, &mut room] {
                    expected.terminal = Some(Outcome::Refused);
                    expected.rounds = 1;
                    expected.closed_at = Some(stamp(20));
                }
            }
            for key in [None, Some(-10), Some(room_key)] {
                let mut journal = base.clone();
                journal.push(event(30, REARMED_ACTION, key));
                let expected_room = if key == Some(room_key) {
                    None
                } else {
                    Some(room.clone())
                };
                assert_eq!(
                    folded(&journal),
                    (Some(limit.clone()), expected_room),
                    "rearm key={key:?} terminal={terminal}"
                );
                let mut journal = base.clone();
                journal.push(event(30, CLEARED_ACTION, key));
                assert_eq!(
                    folded(&journal),
                    (None, Some(room.clone())),
                    "clear key={key:?} terminal={terminal}"
                );
            }
        }
    }
}

#[test]
fn cooldown_reopens_before_cross_key_closing_only_at_the_boundary() {
    for first_action in [REFUSED_ACTION, FAILED_ACTION] {
        for next_action in [ATTEMPT_ACTION, REFUSED_ACTION, FAILED_ACTION] {
            for after in [COOLDOWN - 1, COOLDOWN] {
                let now = 20 + after;
                let journal = [
                    event(0, LIMIT_ACTION, None),
                    event(5, ATTEMPT_ACTION, Some(0)),
                    event(20, first_action, Some(0)),
                    event(30, HEADROOM_ACTION, None),
                    event(now, next_action, Some(0)),
                ];
                let mut limit = wanted(0, Trigger::Limit);
                limit.attempts = 1;
                limit.rounds = 1;
                limit.closed_at = Some(stamp(20));
                limit.terminal = Some(if first_action == REFUSED_ACTION {
                    Outcome::Refused
                } else {
                    Outcome::Failed
                });
                let mut room = wanted(30, Trigger::Headroom);
                if next_action == ATTEMPT_ACTION {
                    // BASE PIN: an early attempt still counts/opens on a closed
                    // episode; it does not erase the terminal before cooldown.
                    limit.attempts = if after == COOLDOWN { 1 } else { 2 };
                    limit.open = Some(stamp(now));
                    if after == COOLDOWN {
                        limit.terminal = None;
                    }
                } else if after == COOLDOWN {
                    let outcome = if next_action == REFUSED_ACTION {
                        Outcome::Refused
                    } else {
                        Outcome::Failed
                    };
                    limit.attempts = 0;
                    limit.rounds = 2;
                    limit.closed_at = Some(stamp(now));
                    limit.terminal = Some(outcome);
                    room.rounds = 1;
                    room.closed_at = Some(stamp(now));
                    room.terminal = Some(outcome);
                }
                assert_eq!(
                    folded(&journal),
                    (Some(limit), Some(room)),
                    "{first_action} {next_action} after={after}"
                );
            }
        }
    }
}

#[test]
fn live_peer_closing_never_recounts_an_already_terminal_episode() {
    for terminal_kind in [Trigger::Limit, Trigger::Headroom] {
        let (old_action, fresh_action) = if terminal_kind == Trigger::Limit {
            (LIMIT_ACTION, HEADROOM_ACTION)
        } else {
            (HEADROOM_ACTION, LIMIT_ACTION)
        };
        let journal = [
            event(0, old_action, None),
            event(5, ATTEMPT_ACTION, Some(0)),
            event(10, REFUSED_ACTION, Some(0)),
            event(20, fresh_action, None),
            event(30, FAILED_ACTION, Some(20)),
        ];
        let mut old = wanted(0, terminal_kind);
        old.attempts = 1;
        old.rounds = 1;
        old.terminal = Some(Outcome::Refused);
        old.closed_at = Some(stamp(10));
        let mut fresh = wanted(
            20,
            if terminal_kind == Trigger::Limit {
                Trigger::Headroom
            } else {
                Trigger::Limit
            },
        );
        fresh.rounds = 1;
        fresh.terminal = Some(Outcome::Failed);
        fresh.closed_at = Some(stamp(30));
        let expected = if terminal_kind == Trigger::Limit {
            (Some(old), Some(fresh))
        } else {
            (Some(fresh), Some(old))
        };
        assert_eq!(folded(&journal), expected, "terminal {terminal_kind:?}");
    }
}

#[test]
fn cooldown_does_not_reopen_for_other_actions_refs_headroom_or_spent_rounds() {
    for action in [HELD_ACTION, LIMIT_ACTION, HEADROOM_ACTION, "unrelated"] {
        let mut journal = vec![
            event(0, LIMIT_ACTION, None),
            event(10, REFUSED_ACTION, Some(0)),
            event(20, HEADROOM_ACTION, None),
        ];
        journal.push(event(10 + COOLDOWN, action, Some(0)));
        let limit = folded(&journal).0.expect("limit retained");
        assert_eq!(
            (limit.terminal, limit.rounds, limit.closed_at),
            (Some(Outcome::Refused), 1, Some(stamp(10))),
            "{action}"
        );
    }
    for key in [None, Some(-1)] {
        for action in [ATTEMPT_ACTION, REFUSED_ACTION, FAILED_ACTION] {
            let journal = [
                event(0, LIMIT_ACTION, None),
                event(10, REFUSED_ACTION, Some(0)),
                event(10 + COOLDOWN, action, key),
            ];
            let mut expected = wanted(0, Trigger::Limit);
            expected.rounds = 1;
            expected.closed_at = Some(stamp(10));
            expected.terminal = Some(Outcome::Refused);
            assert_eq!(
                folded(&journal),
                (Some(expected), None),
                "wrong ref {key:?} {action}"
            );
        }
    }
    let mut journal = vec![event(0, LIMIT_ACTION, None)];
    for second in [10, 10 + COOLDOWN, 10 + 2 * COOLDOWN] {
        journal.push(event(second, REFUSED_ACTION, Some(0)));
    }
    journal.push(event(10 + 3 * COOLDOWN, ATTEMPT_ACTION, Some(0)));
    let mut expected = wanted(0, Trigger::Limit);
    expected.rounds = 3;
    expected.closed_at = Some(stamp(10 + 2 * COOLDOWN));
    expected.terminal = Some(Outcome::Refused);
    // BASE PIN: spent/closed attempts absorb metadata but cannot reopen.
    expected.attempts = 1;
    expected.open = Some(stamp(10 + 3 * COOLDOWN));
    assert_eq!(folded(&journal), (Some(expected), None), "spent limit");
    let journal = [
        event(0, HEADROOM_ACTION, None),
        event(10, REFUSED_ACTION, Some(0)),
        event(10 + COOLDOWN, ATTEMPT_ACTION, Some(0)),
    ];
    let mut expected = wanted(0, Trigger::Headroom);
    expected.rounds = 1;
    expected.closed_at = Some(stamp(10));
    expected.terminal = Some(Outcome::Refused);
    expected.attempts = 1;
    expected.open = Some(stamp(10 + COOLDOWN));
    assert_eq!(
        folded(&journal),
        (None, Some(expected)),
        "headroom stays terminal"
    );
}

#[test]
fn move_is_reference_agnostic_and_final_from_every_terminal_state() {
    for closing in [
        None,
        Some(REFUSED_ACTION),
        Some(FAILED_ACTION),
        Some(DONE_ACTION),
    ] {
        for key in [None, Some(-100), Some(0), Some(10)] {
            let mut journal = both();
            journal.push(event(15, ATTEMPT_ACTION, Some(0)));
            let mut expected = wanted(0, Trigger::Limit);
            expected.attempts = 1;
            if let Some(action) = closing {
                journal.push(event(20, action, Some(0)));
                if action != DONE_ACTION {
                    expected.rounds = 1;
                    expected.closed_at = Some(stamp(20));
                }
            }
            journal.extend([
                event(30, DONE_ACTION, key),
                event(30 + COOLDOWN, FAILED_ACTION, Some(0)),
            ]);
            expected.terminal = Some(Outcome::Done);
            assert_eq!(
                folded(&journal),
                (Some(expected), None),
                "closing={closing:?} ref={key:?}"
            );
        }
    }
    assert_eq!(
        folded(&[
            event(0, HEADROOM_ACTION, None),
            event(20, DONE_ACTION, None)
        ]),
        (None, None)
    );
}

#[test]
fn closed_hold_metadata_is_preserved_and_observed_choice_is_sticky() {
    // BASE PIN: held/attempt can change closed metadata without reopening.
    for opener in [LIMIT_ACTION, HEADROOM_ACTION] {
        let trigger = if opener == LIMIT_ACTION {
            Trigger::Limit
        } else {
            Trigger::Headroom
        };
        let mut hold = event(20, HELD_ACTION, Some(0));
        hold.summary = Some("observed-choice: operator model".to_owned());
        let mut journal = vec![
            event(0, opener, None),
            event(10, FAILED_ACTION, Some(0)),
            hold,
        ];
        let mut expected = wanted(0, trigger);
        expected.rounds = 1;
        expected.closed_at = Some(stamp(10));
        expected.terminal = Some(Outcome::Failed);
        expected.held = true;
        expected.last_hold = Some("observed-choice: operator model".to_owned());
        expected.choice_held = true;
        for step in 0..3 {
            if step == 1 {
                journal.push(event(21, HELD_ACTION, Some(0)));
                expected.last_hold = None;
            }
            if step == 2 {
                journal.push(event(22, ATTEMPT_ACTION, Some(0)));
                expected.attempts = 1;
                expected.open = Some(stamp(22));
                expected.held = false;
            }
            let pair = if trigger == Trigger::Limit {
                (Some(expected.clone()), None)
            } else {
                (None, Some(expected.clone()))
            };
            assert_eq!(folded(&journal), pair, "{opener} step={step}");
        }
    }
}

#[test]
fn absorb_requires_own_key_and_unknown_actions_do_nothing() {
    for action in [
        ATTEMPT_ACTION,
        HELD_ACTION,
        REFUSED_ACTION,
        FAILED_ACTION,
        "unrelated",
        "reseat",
    ] {
        for key in [None, Some(-10)] {
            let mut journal = both();
            journal.push(event(30, action, key));
            assert_eq!(
                folded(&journal),
                (
                    Some(wanted(0, Trigger::Limit)),
                    Some(wanted(10, Trigger::Headroom))
                ),
                "{action} {key:?}"
            );
        }
    }
    for key in [0, 10] {
        let mut journal = both();
        journal.push(event(30, ATTEMPT_ACTION, Some(key)));
        let mut limit = wanted(0, Trigger::Limit);
        let mut room = wanted(10, Trigger::Headroom);
        let expected = if key == 0 { &mut limit } else { &mut room };
        expected.attempts = 1;
        expected.open = Some(stamp(30));
        assert_eq!(
            folded(&journal),
            (Some(limit), Some(room)),
            "own attempt {key}"
        );
    }
}

#[test]
fn append_order_and_prefix_leave_kind_survive_backward_or_equal_timestamps() {
    // Journal order is authoritative: an earlier timestamp is not a sort key.
    let journal = [
        event(100, LIMIT_ACTION, None),
        event(20, CLEARED_ACTION, None),
        event(10, LIMIT_ACTION, None),
    ];
    assert_eq!(folded(&journal), (Some(wanted(10, Trigger::Limit)), None));
    for limit_before_attempt in [false, true] {
        let mut journal = vec![event(0, HEADROOM_ACTION, None)];
        if limit_before_attempt {
            journal.push(event(0, LIMIT_ACTION, None));
        }
        journal.push(event(0, ATTEMPT_ACTION, Some(0)));
        if !limit_before_attempt {
            journal.push(event(0, LIMIT_ACTION, None));
        }
        let mut moved = event(30, DONE_ACTION, None);
        moved.reference = Some("previous-profile".to_owned());
        journal.push(moved);
        let expected = if limit_before_attempt {
            Vec::<String>::new()
        } else {
            vec!["previous-profile".to_owned()]
        };
        assert_eq!(
            left_on_headroom(&journal, SESSION, SLOT, AGENT),
            expected,
            "limit_before_attempt={limit_before_attempt}"
        );
    }
}
