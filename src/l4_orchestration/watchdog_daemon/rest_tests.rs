use super::{
    Carry, Knobs, Look, Pace as _, STEP_BACKSTOP, StepPace, TickerMode, rest, ticker_mode,
};
use crate::transport::quietwd_spec::Rig;
use std::time::{Duration, Instant};

#[test]
fn a_stepped_rest_lasts_its_duration_without_a_step_and_a_step_ends_it_one_backstop_ahead() {
    let rig = Rig::new("steprest");
    let channel = "qw-steprest";
    let mut pace = StepPace::new(rig.server.clone(), channel);
    // No step: the rest lasts its whole duration, and the clock gains no
    // skew over the wall clock.
    let began = Instant::now();
    pace.sleep(Duration::from_millis(300));
    assert!(
        began.elapsed() >= Duration::from_millis(300),
        "with no step the rest lasts its duration"
    );
    assert_eq!(pace.skew, Duration::ZERO, "with no step no skew");
    // A step ends the next rest at once and moves the clock one backstop
    // ahead, past any interval deadline.
    assert!(
        crate::transport::signal_channel(&rig.server, channel),
        "the step is sent"
    );
    let began = Instant::now();
    pace.sleep(Duration::from_mins(1));
    assert!(
        began.elapsed() < Duration::from_secs(30),
        "a step ends the rest"
    );
    assert_eq!(pace.skew, STEP_BACKSTOP, "a step adds one backstop");
    assert!(
        pace.now() + Duration::from_secs(1) >= Instant::now() + STEP_BACKSTOP,
        "the effective clock runs one backstop ahead"
    );
}

#[test]
fn an_unstepped_retry_rests_one_plain_interval_and_asks_tmux_nothing() {
    let rig = Rig::new("retryrest");
    let mut carry = Carry::new();
    carry.look = Some(Look::DEFAULT);
    assert_eq!(
        ticker_mode(carry.look.as_ref()),
        TickerMode::Animated,
        "a look whose interval would tick"
    );
    let knobs = Knobs {
        interval_secs: 1,
        ..Knobs::default()
    };
    let began = Instant::now();
    let ((), calls) = crate::transport::spawns_during(|| {
        rest(
            &rig.server,
            &rig.session,
            &mut carry,
            knobs,
            false,
            &mut None,
        );
    });
    assert!(
        began.elapsed() >= Duration::from_secs(1),
        "the retry rest lasts the interval"
    );
    assert!(
        calls.is_empty(),
        "a retry rest asks tmux nothing: {calls:?}"
    );
}
