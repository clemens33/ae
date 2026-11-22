//! Deterministic listener-local credential lifetime and capacity pins.

use std::time::Duration;

use ae::web::auth::Auth;
use ae::web::clock::{Clock, ManualClock};

fn redeem(auth: &mut Auth, clock: &ManualClock, sequence: u8) -> String {
    let nonce = auth.mint_code([sequence; 16], clock.now());
    auth.redeem(&nonce, [sequence; 32], clock.now())
        .unwrap_or_else(|_| panic!("a newly minted code can open session {sequence}"))
}

#[test]
fn a_code_just_under_ten_minutes_works_once_and_an_older_code_refuses() {
    let mut auth = Auth::new([7; 32]);
    let clock = ManualClock::new(1_795_000_000);
    let unspent = auth.mint_code([1; 16], clock.now());
    clock.advance(Duration::from_secs(599));
    assert!(auth.redeem(&unspent, [2; 32], clock.now()).is_ok());
    assert!(auth.redeem(&unspent, [3; 32], clock.now()).is_err());

    let expiring = auth.mint_code([4; 16], clock.now());
    clock.advance(Duration::from_secs(601));
    assert!(auth.redeem(&expiring, [5; 32], clock.now()).is_err());
    let _ = redeem(&mut auth, &clock, 6);
}

#[test]
fn authenticated_use_extends_only_the_used_sessions_idle_lifetime() {
    let mut auth = Auth::new([7; 32]);
    let clock = ManualClock::new(1_795_000_000);
    let active = redeem(&mut auth, &clock, 1);
    let silent = redeem(&mut auth, &clock, 2);
    clock.advance(Duration::from_secs(43_199));
    assert!(auth.check(&active, clock.now()));
    clock.advance(Duration::from_secs(2));
    assert!(auth.check(&active, clock.now()));
    assert!(!auth.check(&silent, clock.now()));
    clock.advance(Duration::from_secs(43_201));
    assert!(!auth.check(&active, clock.now()));
}

#[test]
fn the_ninth_live_session_evicts_the_least_recently_used_session() {
    let mut auth = Auth::new([7; 32]);
    let clock = ManualClock::new(1_795_000_000);
    let mut sessions = Vec::new();
    for sequence in 1..=8 {
        sessions.push(redeem(&mut auth, &clock, sequence));
        clock.advance(Duration::from_secs(1));
    }
    for session in &sessions[1..] {
        assert!(auth.check(session, clock.now()));
        clock.advance(Duration::from_secs(1));
    }
    assert!(auth.check(&sessions[0], clock.now()));
    clock.advance(Duration::from_secs(1));
    let ninth = redeem(&mut auth, &clock, 9);
    assert!(!auth.check(&sessions[1], clock.now()));
    for session in sessions.iter().take(1).chain(sessions.iter().skip(2)) {
        assert!(auth.check(session, clock.now()));
    }
    assert!(auth.check(&ninth, clock.now()));
}
