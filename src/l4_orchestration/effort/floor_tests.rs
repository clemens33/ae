//! Unit tests of the floored step (#318 S2): what `twin_from` and `deeper`
//! decide without a running seat.

use super::*;
use crate::config::parse_identity;

/// Codex profiles of one model at every tier from low to xhigh, floored at
/// `floor`.
fn floored(floor: &str) -> IdentityConfig {
    parse_identity(&format!(
        "[profiles]\nx=codex -m gpt -c model_reasoning_effort=xhigh\nh=codex -m gpt -c model_reasoning_effort=high\nm=codex -m gpt -c model_reasoning_effort=medium\nl=codex -m gpt -c model_reasoning_effort=low\n[effort_floor]\n\"gpt\" = \"{floor}\"\n"
    ))
    .expect("floored config fixture")
}

fn step(cfg: &IdentityConfig, first: Option<Level>) -> Result<(Level, Level), TwinRefusal> {
    twin_from(
        cfg,
        None,
        "x",
        &RecordedClient::Missing,
        ToolKind::Codex,
        first,
    )
    .map(|twin| (twin.baseline, twin.target))
}

#[test]
fn a_floored_step_reaches_the_floor_and_never_passes_it() {
    let cfg = floored("medium");
    assert_eq!(step(&cfg, None), Ok((Level::Xhigh, Level::High)));
    assert_eq!(
        step(&cfg, Some(Level::High)),
        Ok((Level::High, Level::Medium)),
        "a step may land on the floor itself"
    );
    assert_eq!(
        step(&cfg, Some(Level::Medium)),
        Err(TwinRefusal::Floored(Level::Medium)),
        "no step goes below the floor"
    );
    let low = floored("low");
    assert_eq!(
        step(&low, Some(Level::Medium)),
        Ok((Level::Medium, Level::Low))
    );
    let high = floored("high");
    assert_eq!(step(&high, None), Ok((Level::Xhigh, Level::High)));
    assert_eq!(
        step(&high, Some(Level::High)),
        Err(TwinRefusal::Floored(Level::High))
    );
}

const OBS: i64 = 1_700_000_000;
const NOW: i64 = OBS + 10;
const RESET: i64 = OBS + 86_400;

fn key(bucket: &str) -> WindowKey {
    WindowKey {
        bucket: bucket.into(),
        qualifier: Some("pro".into()),
        minutes: Some(10_080),
    }
}

fn pressed(bucket: &str, observed_at: i64) -> Pressured {
    Pressured {
        key: key(bucket),
        observed_at,
        resets_at: Some(RESET),
    }
}

fn reading(bucket: &str, judged: f64) -> Window {
    Window {
        key: key(bucket),
        judged,
        usable: true,
        observed_at: NOW,
        resets_at: Some(RESET),
    }
}

#[test]
fn merged_windows_keep_the_first_step_add_the_second_and_take_its_reading() {
    let first = [pressed("b", OBS), pressed("a", OBS)];
    let second = vec![pressed("c", NOW), pressed("b", NOW)];
    assert_eq!(
        merged(&first, second),
        Some(vec![
            pressed("a", OBS),
            pressed("b", NOW),
            pressed("c", NOW)
        ]),
        "a kept, b replaced by the second step's reading, c added, sorted by key"
    );
    let names = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];
    let five: Vec<Pressured> = names[..5].iter().map(|b| pressed(b, OBS)).collect();
    let three = names[5..8].iter().map(|b| pressed(b, NOW)).collect();
    assert_eq!(
        merged(&five, three).map(|windows| windows.len()),
        Some(MAX_WINDOWS),
        "exactly the cap is recordable"
    );
    let four = names[5..].iter().map(|b| pressed(b, NOW)).collect();
    assert_eq!(merged(&five, four), None, "one past the cap is not");
}

/// The applied first step xhigh -> high on windows `a` and `b`.
fn first_step() -> (Fold, Stored) {
    let record = Record::parse(&format!(
        "v1 applied slot:main base:xhigh low:high at:80 ep:{OBS} sid:11111111-1111-4111-8111-111111111111 agent:lead tool:codex launch:launch-1 pid:42 conv:22222222-2222-4222-8222-222222222222 profile:x client:- model:gpt win:a/pro/10080/{OBS}/{RESET},b/pro/10080/{OBS}/{RESET}"
    ))
    .expect("first-step record fixture");
    let episode = Episode {
        key: OBS,
        threshold: 80,
        down: Leg::Done(Transition {
            direction: Direction::Down,
            from: Level::Xhigh,
            to: Level::High,
        }),
        up: Leg::Unspent,
        named: Vec::new(),
        yielded: false,
    };
    (Fold::Open(episode), Stored::Present(record))
}

fn second_twin() -> Twin {
    Twin {
        baseline: Level::High,
        target: Level::Medium,
        aliases: vec!["m".into()],
    }
}

fn decided(
    fold: &Fold,
    stored: &Stored,
    twin: &Result<Twin, TwinRefusal>,
    windows: Vec<Window>,
    new_attempts: bool,
) -> Decision {
    let scope = Scope {
        windows,
        spend_capped: false,
    };
    decide(&Inputs {
        at: 80,
        new_attempts,
        scope: Some(&scope),
        limited: false,
        fold,
        stored,
        guard: Some(Ok(())),
        twin,
        live: Some(Level::High),
        now: NOW,
    })
}

#[test]
fn the_second_step_needs_ninety_and_records_every_moving_window() {
    let (fold, stored) = first_step();
    let twin = Ok(second_twin());
    let at = |a: f64, b: f64, c: f64| vec![reading("a", a), reading("b", b), reading("c", c)];
    assert_eq!(
        decided(&fold, &stored, &twin, at(74.9, 89.9, 85.0), true),
        Decision::Hold(Hold::Pressure),
        "under ninety the first step holds"
    );
    let Decision::Deeper {
        twin: chosen,
        windows,
    } = decided(&fold, &stored, &twin, at(74.9, 90.0, 90.0), true)
    else {
        panic!("ninety takes the second step");
    };
    assert_eq!(Ok(chosen), twin);
    assert_eq!(
        windows,
        vec![pressed("a", OBS), pressed("b", NOW), pressed("c", NOW)]
    );
    assert_eq!(
        decided(&fold, &stored, &twin, at(74.9, 90.0, 90.0), false),
        Decision::Hold(Hold::Disabled)
    );
}

#[test]
fn a_refused_second_twin_holds_silently_or_names_its_reason_once() {
    let (fold, stored) = first_step();
    let windows = || vec![reading("a", 90.0), reading("b", 90.0)];
    for silent in [
        Err(TwinRefusal::Floored(Level::High)),
        Err(TwinRefusal::Floor(Level::High)),
        Ok(Twin {
            baseline: Level::Xhigh,
            target: Level::High,
            aliases: vec!["h".into()],
        }),
    ] {
        assert_eq!(
            decided(&fold, &stored, &silent, windows(), true),
            Decision::Hold(Hold::Pressure),
            "{silent:?}"
        );
    }
    let refused = Err(TwinRefusal::NoTwin(Level::Medium));
    assert_eq!(
        decided(&fold, &stored, &refused, windows(), true),
        Decision::Name("no-twin")
    );
    let Fold::Open(mut episode) = fold else {
        panic!("open fixture");
    };
    episode.named.push("no-twin".into());
    assert_eq!(
        decided(&Fold::Open(episode), &stored, &refused, windows(), true),
        Decision::Hold(Hold::Skipped)
    );
}

#[test]
fn second_step_windows_past_the_cap_are_named_once_then_held() {
    let (fold, stored) = first_step();
    let twin = Ok(second_twin());
    let names = ["c", "d", "e", "f", "g", "h", "i"];
    let windows = || names.iter().map(|b| reading(b, 90.0)).collect::<Vec<_>>();
    assert_eq!(
        decided(&fold, &stored, &twin, windows(), true),
        Decision::Name("unrecordable"),
        "two kept and seven new windows are nine"
    );
    let Fold::Open(mut episode) = fold else {
        panic!("open fixture");
    };
    episode.named.push("unrecordable".into());
    assert_eq!(
        decided(&Fold::Open(episode), &stored, &twin, windows(), true),
        Decision::Hold(Hold::Skipped)
    );
}

#[test]
fn a_floored_twin_opens_no_episode() {
    let windows = || vec![reading("a", 90.0)];
    let floored = Err(TwinRefusal::Floored(Level::Xhigh));
    assert_eq!(
        decided(&Fold::Armed, &Stored::Absent, &floored, windows(), true),
        Decision::Idle,
        "a floor at the baseline leaves nothing to step"
    );
    assert_eq!(
        decided(
            &Fold::Armed,
            &Stored::Absent,
            &Ok(second_twin()),
            windows(),
            true
        ),
        Decision::Open { threshold: 80 },
        "the same reading with a twin opens"
    );
}

/// The seat's watchdog records, folded; at `OBS`, so the episode is fresh.
fn history(records: &[(&str, &str, &str)]) -> Fold {
    let events: Vec<Event> = records
        .iter()
        .map(|(action, reference, summary)| {
            Event::parse_line(&format!(
                r#"{{"ts":"2023-11-14T22:13:20Z","actor":"watchdog","action":"{action}","target":"lead","target_slot":"main","target_session":"fixture","target_session_id":"11111111-1111-4111-8111-111111111111","ref":"{reference}","summary":"{summary}"}}"#
            ))
            .expect("literal effort history")
        })
        .collect();
    fold(
        &events,
        SessionKey::new("fixture", "11111111-1111-4111-8111-111111111111"),
        "main",
        "lead",
    )
}

const FIRST: [(&str, &str, &str); 3] = [
    ("effort-episode", "1700000000", "threshold 80"),
    ("effort-reserved", "1700000000-down", "xhigh->high"),
    ("effort-applied", "1700000000-down", "xhigh->high"),
];

#[test]
fn a_step_down_moves_one_adjacent_tier() {
    let Fold::Open(episode) = history(&FIRST) else {
        panic!("an adjacent first step folds open");
    };
    assert_eq!(
        episode.down,
        Leg::Done(Transition {
            direction: Direction::Down,
            from: Level::Xhigh,
            to: Level::High,
        })
    );
    assert!(
        matches!(
            history(&[
                FIRST[0],
                ("effort-reserved", "1700000000-down", "xhigh->medium")
            ]),
            Fold::Doubt(_)
        ),
        "a step down skips no tier"
    );
}

#[test]
fn a_second_step_starts_where_the_first_step_landed() {
    let mut records = FIRST.to_vec();
    records.push(("effort-reserved", "1700000000-down2", "high->medium"));
    assert!(matches!(history(&records), Fold::Open(_)));
    records.pop();
    records.push(("effort-reserved", "1700000000-down2", "medium->low"));
    assert!(
        matches!(history(&records), Fold::Doubt(_)),
        "a second step from a tier the first step did not reach"
    );
}

#[test]
fn a_step_up_climbs_one_or_two_tiers_and_never_down() {
    let up = |summary| pair(summary, Direction::Up).map(|t| (t.from, t.to));
    assert_eq!(up("high->xhigh"), Some((Level::High, Level::Xhigh)));
    assert_eq!(up("medium->xhigh"), Some((Level::Medium, Level::Xhigh)));
    for wrong in ["low->xhigh", "xhigh->high", "high->high"] {
        assert_eq!(up(wrong), None, "{wrong}");
    }
}

#[test]
fn a_fresh_conversation_fails_only_a_first_step_and_drops_a_second() {
    let tail = format!(
        "at:80 ep:{OBS} sid:11111111-1111-4111-8111-111111111111 agent:lead tool:codex launch:launch-1 pid:42 conv:22222222-2222-4222-8222-222222222222 profile:x client:- model:gpt win:a/pro/10080/{OBS}/{RESET}"
    );
    let resumed = |text: String| {
        let record = Record::parse(&text).expect("restart record fixture");
        on_resume(&Stored::Present(record), Ok(()), None, Switch::On, false)
    };
    assert_eq!(
        resumed(format!(
            "v1 restart-down slot:main base:xhigh low:high {tail}"
        )),
        Resume::Failed { episode: OBS },
        "the first step was authorized and spent"
    );
    assert_eq!(
        resumed(format!(
            "v2 restart-down slot:main base:xhigh low:medium step:2 {tail}"
        )),
        Resume::Discard {
            episode: Some(OBS),
            why: Discarded::Fresh,
        },
        "the profile's own effort ends a second step like an applied one"
    );
}

fn down(from: Level, to: Level) -> Transition {
    Transition {
        direction: Direction::Down,
        from,
        to,
    }
}

/// The open episode at `OBS` whose down leg is `down`, never stepped up.
fn ledger(down: Leg) -> Fold {
    stepped(down, Leg::Unspent)
}

/// The open episode at `OBS` with these legs.
fn stepped(down: Leg, up: Leg) -> Fold {
    Fold::Open(Episode {
        key: OBS,
        threshold: 80,
        down,
        up,
        named: Vec::new(),
        yielded: false,
    })
}

/// A resume of the recorded conversation, its guard passed, against `fold`.
fn resumed(head: &str, fold: &Fold) -> Resume {
    let record = Record::parse(&format!(
        "{head} at:80 ep:{OBS} sid:11111111-1111-4111-8111-111111111111 agent:lead tool:codex launch:launch-1 pid:42 conv:22222222-2222-4222-8222-222222222222 profile:x client:- model:gpt win:a/pro/10080/{OBS}/{RESET}"
    ))
    .expect("resume record fixture");
    on_resume(
        &Stored::Present(record),
        Ok(()),
        Some(fold),
        Switch::On,
        true,
    )
}

#[test]
fn a_restart_replays_only_the_step_the_journal_reserved() {
    let first = down(Level::Xhigh, Level::High);
    let restart = "v1 restart-down slot:main base:xhigh low:high";
    assert_eq!(
        resumed(restart, &ledger(Leg::Reserved(first))),
        Resume::Replay(Level::High)
    );
    assert_eq!(
        resumed(
            "v1 restart-down slot:main base:max low:xhigh",
            &ledger(Leg::Reserved(first))
        ),
        Resume::Refuse(State::RestartDown),
        "the journal reserved another step from another baseline"
    );
    let second = "v2 restart-down slot:main base:xhigh low:medium step:2";
    let deeper = |to| ledger(Leg::Deeper(first, Second::Reserved(down(Level::High, to))));
    assert_eq!(
        resumed(second, &deeper(Level::Medium)),
        Resume::Replay(Level::Medium)
    );
    assert_eq!(
        resumed(second, &deeper(Level::Low)),
        Resume::Refuse(State::RestartDown),
        "the journal reserved another second step from the same tier"
    );
}

#[test]
fn an_applied_record_replays_the_tier_the_journal_left_the_seat_on() {
    let first = down(Level::Xhigh, Level::High);
    let second = down(Level::High, Level::Medium);
    let one = "v1 applied slot:main base:xhigh low:high";
    assert_eq!(
        resumed(one, &ledger(Leg::Deeper(first, Second::Reserved(second)))),
        Resume::Replay(Level::High),
        "a reserved second step leaves the seat on the first step's tier"
    );
    assert_eq!(
        resumed(one, &ledger(Leg::Deeper(first, Second::Done(second)))),
        Resume::Keep(Kept::Ledger),
        "the journal moved the seat past the record"
    );
    assert_eq!(
        resumed(
            "v2 applied slot:main base:xhigh low:medium step:2",
            &ledger(Leg::Deeper(first, Second::Done(second)))
        ),
        Resume::Replay(Level::Medium)
    );
}

#[test]
fn a_restart_up_replays_only_from_the_tier_both_steps_reached() {
    let first = down(Level::Xhigh, Level::High);
    let second = down(Level::High, Level::Medium);
    let up = Leg::Reserved(Transition {
        direction: Direction::Up,
        from: Level::Medium,
        to: Level::Xhigh,
    });
    let restart = "v2 restart-up slot:main base:xhigh low:medium step:2";
    let both = Leg::Deeper(first, Second::Done(second));
    assert_eq!(
        resumed(restart, &stepped(both, up)),
        Resume::Replay(Level::Xhigh)
    );
    assert_eq!(
        resumed(restart, &stepped(both, Leg::Unspent)),
        Resume::Refuse(State::RestartUp),
        "the journal reserved no step up"
    );
    assert_eq!(
        resumed(restart, &stepped(Leg::Done(first), up)),
        Resume::Refuse(State::RestartUp),
        "the journal left the seat on the first step's tier"
    );
}
