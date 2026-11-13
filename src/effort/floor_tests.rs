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
