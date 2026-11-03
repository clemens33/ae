//! Frozen acceptance spec (slice limitwake): when auto reseat waits for a
//! near short-window reset instead of moving the seat. Oracle: brief
//! `.local/brief-limitwake.md` AMENDMENT 1 — under 60 min to a known short
//! reset waits; 60 min or more, unknown, or not proven short moves as
//! today; a wait that outlives reset + fallback moves as today.
//!
//! Installed byte-identical at `src/autoreseat/limitwake_spec.rs`, wired
//! from `src/autoreseat.rs` via
//! `#[cfg(test)] #[path = "autoreseat/limitwake_spec.rs"] mod limitwake_spec;`.

use super::{LIMIT_WAIT_MAX_SECS, LIMIT_WAKE_FALLBACK_SECS, LimitReset, reset_wait};

const AT: i64 = 100_000;

const SHORT: Option<LimitReset> = Some(LimitReset {
    at: AT,
    short: true,
});

#[test]
fn the_wait_bounds_are_the_ruled_constants() {
    assert_eq!(LIMIT_WAIT_MAX_SECS, 3600, "under 60 minutes waits");
    assert_eq!(LIMIT_WAKE_FALLBACK_SECS, 300, "five default cycles");
}

#[test]
fn an_unknown_or_unproven_short_reset_never_waits() {
    for now in [AT - 100_000, AT - 60, AT, AT + 60] {
        assert_eq!(reset_wait(None, now), None, "unknown: {now}");
    }
    let unproven = Some(LimitReset {
        at: AT,
        short: false,
    });
    for now in [AT - 3599, AT - 60, AT, AT + 299] {
        assert_eq!(
            reset_wait(unproven, now),
            None,
            "weekly/long or unstated length moves as today: {now}"
        );
    }
}

#[test]
fn a_short_reset_under_sixty_minutes_away_waits_until_the_fallback() {
    // The boundary is the ruling's own words: UNDER 60 min waits, 60 min
    // or more moves. Judged at decision time, each cycle.
    assert_eq!(reset_wait(SHORT, AT - 3601), None, "far: moves");
    assert_eq!(reset_wait(SHORT, AT - 3600), None, "exactly 60 min: moves");
    assert_eq!(
        reset_wait(SHORT, AT - 3599),
        Some(AT + 300),
        "inside 60 min: waits"
    );
    assert_eq!(reset_wait(SHORT, AT - 1), Some(AT + 300));
    assert_eq!(reset_wait(SHORT, AT), Some(AT + 300), "at the reset");
    assert_eq!(
        reset_wait(SHORT, AT + 299),
        Some(AT + 300),
        "the woken turn still has its cycles to land and scroll"
    );
}

#[test]
fn a_wait_that_outlives_the_fallback_moves_as_today() {
    assert_eq!(
        reset_wait(SHORT, AT + 300),
        None,
        "at reset + fallback the move path applies again — no seat stuck"
    );
    assert_eq!(reset_wait(SHORT, AT + 10_000), None);
}
