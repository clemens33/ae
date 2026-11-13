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
