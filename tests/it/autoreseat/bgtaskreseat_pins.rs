//! #304 B survivor pins, authored by the navigator after the frozen spec.
//! Oracle: the brief and tool.rs's capability (only a harness whose
//! background task rows ae reads may have them ended on a long limit);
//! never the code under test.
use super::*;

const WATCHING: &str = include_str!("../../fixtures/harness-state/muse-limit-watching.txt");

/// The measured Muse long-limit frame (wrapped ISO 429 row over the
/// background-only stop), resetting at `reset`. Derived as in the spec.
fn muse_long_frame(reset: i64) -> String {
    let reset = Timestamp::from_epoch(reset);
    format!(
        "◆ model failed: API error 429 [request_id=00000000-0000-0000-0000-000000000000]:\n  Subscription quota exhausted. Your usage window resets at\n  {reset}. (rate_limit_error)\n\n{WATCHING}"
    )
}

/// mutant src/reseat.rs:1154:29 (`&&` -> `||`): a NON-Muse seat on a limit
/// episode whose pane happens to draw a Muse-shaped background-only frame,
/// reset 12 h+ ahead and past the 10 min grace, is not a harness whose
/// background tasks ae reads: the stop refuses its unknown frame and the
/// process survives. Unknown never proves idleness.
#[test]
fn bgtask_pin_non_muse_seat_with_a_muse_shaped_long_limit_frame_is_not_stopped() {
    let rig = Rig::new("bgpinclaude");
    configure(&rig, ON_NOW, "fake-opencode");
    let frame = muse_long_frame(Timestamp::now().epoch() + 43_260);
    let pane = limitstop_playback(&rig, "claude", &frame);
    let now = Timestamp::now().epoch();
    let key = Timestamp::from_epoch(now - 1200).to_string();
    journal_at(&rig, &key, "limit", None);
    journal_at(
        &rig,
        &Timestamp::from_epoch(now).to_string(),
        ATTEMPT_ACTION,
        Some(&key),
    );
    link_send(&rig);
    let pid = rig
        .tool_pid(&pane, "claude")
        .unwrap_or_else(|| panic!("predecessor alive"));

    let (code, out, err) = leg(&rig, "spawned.0", &key);

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert_eq!(rig.tool_pid(&pane, "claude"), Some(pid), "same process");
    untouched(&rig, &pane);
    assert_eq!(booked(&rig, DONE_ACTION), 0, "{}", rig.events());
    let refused = one_outcome(&rig, REFUSED_ACTION);
    let summary = refused.summary.as_deref().unwrap_or_default();
    assert!(summary.starts_with("unknown-frame:"), "{summary}");
}

/// mutant src/watchdog.rs:948:49 (`&&` -> `||`): the vendor's wrapped 429
/// block ends at the first BLANK row, and a row of only spaces is blank — a
/// terminal draws it as nothing (tmux keeps trailing cells it painted). The
/// reset it states is still proven; joining the blank row into the sentence
/// would lose it.
#[test]
fn bgtask_pin_a_spaces_only_row_after_the_429_block_still_ends_it() {
    // `date -u -j -f %Y-%m-%dT%H:%M:%SZ 2027-01-15T08:00:00Z +%s`.
    const RESET: i64 = 1_800_000_000;
    let frame = muse_long_frame(RESET);
    assert_eq!(
        ae::watchdog::muse_limit_reset(&frame),
        Some(RESET),
        "control"
    );
    let spaced = frame.replacen("(rate_limit_error)\n\n", "(rate_limit_error)\n    \n", 1);
    assert_ne!(spaced, frame, "the blank row was replaced");
    assert_eq!(ae::watchdog::muse_limit_reset(&spaced), Some(RESET));
}
