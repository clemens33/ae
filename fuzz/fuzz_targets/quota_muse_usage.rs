#![no_main]

use libfuzzer_sys::fuzz_target;

// Muse's `exec --json` stream, the failed run's stdout and the held cache
// record: vendor bytes ae does not control, plus a hand-editable file under the
// state root. First byte selects the parser; the rest is hostile input. The
// stream is parsed at a fixed clock and at a clock the next eight bytes choose,
// so window arithmetic sees hostile times as well as hostile bytes, and a cache
// record that parses must render back to a record that parses to the same
// value. A failed run's stdout is read for its usage limit (`2`): a reset it
// yields must be a believable clock, and a limit held from it must survive the
// cache round trip and show its one row. A pane muse drew (`3`) is read for the
// reset its own 429 row states and the background tasks under its stop: a reset
// must be a believable clock written in the pane, from a stop that stands up
// to its box, and the tasks must be whole titles of a frame that names one.
const NOW: i64 = 1_792_416_000; // 2026-10-19T12:00:00Z

fuzz_target!(|data: &[u8]| {
    let Some((selector, rest)) = data.split_first() else {
        return;
    };
    let mut clock = [0u8; 8];
    for (slot, byte) in clock.iter_mut().zip(rest.iter()) {
        *slot = *byte;
    }
    let chosen = i64::from_le_bytes(clock);
    if *selector == 0 {
        let _ = std::hint::black_box(ae::quota::muse::parse(rest, NOW));
        let _ = std::hint::black_box(ae::quota::muse::parse(rest, chosen));
        if let Ok(allowance) = ae::quota::muse::extract(rest) {
            let _ = std::hint::black_box(allowance.rows(chosen));
        }
    } else if *selector == 2 {
        if let Some(reset) = ae::quota::muse::limit(rest) {
            assert!((1..=253_402_300_799).contains(&reset));
            let held = ae::quota::muse::Cached {
                home: "/h/.config/muse".to_owned(),
                attempt_at: chosen,
                outcome: ae::quota::muse::Outcome::Limit,
                resets_at: Some(reset),
                allowance: None,
            };
            let again = ae::quota::muse::parse_cache(held.render().as_bytes());
            assert_eq!(again.as_ref(), Some(&held));
            assert_eq!(held.rows(NOW).len(), 1);
        }
    } else if *selector == 3 {
        let pane = String::from_utf8_lossy(rest);
        let standing = ae::watchdog::limit_standing(&pane, "muse");
        if let Some(reset) = ae::watchdog::muse_limit_reset(&pane) {
            assert!((1..=253_402_300_799).contains(&reset));
            assert!(pane.contains(&ae::time::Timestamp::from_epoch(reset).to_string()));
            assert!(!matches!(standing, ae::watchdog::LimitStanding::NoCell));
        }
        if let Some(tasks) = ae::watchdog::muse_background_tasks(&pane) {
            assert!(!tasks.is_empty());
            for task in &tasks {
                assert!(!task.is_empty() && task.trim() == task && !task.contains("  "));
            }
            assert!(matches!(
                standing,
                ae::watchdog::LimitStanding::BackgroundTask(_)
            ));
        }
    } else if let Some(cached) = ae::quota::muse::parse_cache(rest) {
        let again = ae::quota::muse::parse_cache(cached.render().as_bytes());
        assert_eq!(again.as_ref(), Some(&cached));
        let _ = std::hint::black_box(cached.due("/h/.config/muse", chosen));
        let _ = std::hint::black_box(cached.rows(chosen));
    }
});
