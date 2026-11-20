use super::{WatchRead, Watched, before, classify, trust_deadline, watched, watches};
use crate::autotrust::InWait;
use crate::tool::ToolKind;
use std::time::{Duration, Instant};

/// PINS (contract.md:337, "a FRESH session launch watches each seat that
/// gets no launch turn and whose tool measured a trust block"; `auto_trust`
/// off answers nothing): resuming skips, `auto_trust` off skips, and a
/// non-matching seat (no pane, no measured trust block, its own launch turn)
/// is ignored. Each case flips ONE condition of a watched seat.
#[test]
fn a_launch_watches_only_fresh_turnless_trust_capable_seats_with_auto_trust_on() {
    let fresh = (false, true);
    assert!(watches(fresh, "%1", ToolKind::Claude, false));
    assert!(watches(fresh, "%1", ToolKind::Codex, false));
    assert!(watches(fresh, "%1", ToolKind::Agy, false));
    for (launch, pane, tool, turn, why) in [
        (
            (true, true),
            "%1",
            ToolKind::Claude,
            false,
            "a resumed launch",
        ),
        (
            (false, false),
            "%1",
            ToolKind::Claude,
            false,
            "auto_trust off",
        ),
        (
            (true, false),
            "%1",
            ToolKind::Claude,
            false,
            "resumed and off",
        ),
        (
            fresh,
            "",
            ToolKind::Claude,
            false,
            "a preserved row with no pane",
        ),
        (
            fresh,
            "%1",
            ToolKind::Gemini,
            false,
            "a tool with no trust block",
        ),
        (
            fresh,
            "%1",
            ToolKind::Claude,
            true,
            "a seat with its own launch turn",
        ),
    ] {
        assert!(!watches(launch, pane, tool, turn), "{why}");
    }
}

/// PINS (contract.md:337, "ONE launch-wide 15 s deadline that starts no
/// read past it"): the deadline 15 s ends the watch. Fixed instants, no
/// sleep: a read starts only strictly before the deadline.
#[test]
fn the_watch_deadline_is_the_budget_after_its_start_and_ends_reads_at_it() {
    let now = Instant::now();
    let deadline = trust_deadline(now);
    assert_eq!(deadline.duration_since(now), Duration::from_secs(15));
    assert!(before(now, deadline));
    assert!(
        !before(deadline, deadline),
        "AT the deadline no read starts"
    );
    assert!(!before(deadline + Duration::from_millis(1), deadline));
}

/// PINS (contract.md:337, "a MEASURED folder-trust block ... matched whole"
/// is the one exception; "Any other frame ... the prompt is named as
/// today"): the trust guard refuses an unproven block. Only a frame read as
/// a whole measured block reaches the in-wait answerer, which then proves
/// the folder itself (`autotrust::prove`); any other prompt is left to be
/// named and is never answered; no prompt is nothing yet.
#[test]
fn only_a_trust_block_reaches_the_in_wait_answer() {
    let prompt = |trust: bool| crate::watchdog::HumanPrompt {
        question: "Trust this folder?".to_owned(),
        keys: "enter continue · esc back".to_owned(),
        trust: trust.then(|| crate::watchdog::TrustSighting {
            folder: "/tmp/x".to_owned(),
            toward_yes: None,
        }),
        dismiss: false,
    };
    let asked = std::cell::Cell::new(0);
    let answer = || {
        asked.set(asked.get() + 1);
        InWait::Trusted
    };
    assert_eq!(
        classify(Some(prompt(true)), answer),
        WatchRead::Trust(InWait::Trusted)
    );
    assert_eq!(asked.get(), 1);
    assert_eq!(
        classify(Some(prompt(false)), answer),
        WatchRead::Other("Trust this folder?".to_owned())
    );
    assert_eq!(classify(None, answer), WatchRead::Nothing);
    assert_eq!(asked.get(), 1, "nothing but a trust block is answered");
}

/// Run `reads` under a budget of `opens` reads; how it ended and how many
/// reads it took.
fn run(reads: &[WatchRead], opens: usize) -> (Watched, usize) {
    let mut queue = reads.iter().cloned();
    let taken = std::cell::Cell::new(0);
    let mut left = opens;
    let end = watched(
        || {
            taken.set(taken.get() + 1);
            queue.next().unwrap_or(WatchRead::Nothing)
        },
        || {
            let open = left > 0;
            left = left.saturating_sub(1);
            open
        },
    );
    (end, taken.get())
}

/// PINS (the brief: the watch ends at the first ready composer; contract.md
/// :337: an answer tried and stopped tries no more): the first ready
/// composer ends it, as do a landed or stopped answer and another prompt
/// held on two reads. Each ending at the read that decides it.
#[test]
fn a_watch_ends_at_ready_trusted_stop_or_a_held_other_prompt() {
    use WatchRead::{Nothing, Other, Ready, Trust};
    let other = || Other("Pick one?".to_owned());
    for (reads, want) in [
        (vec![Nothing, Ready], (Watched::Ready, 2)),
        (
            vec![Nothing, Trust(InWait::Again), Trust(InWait::Trusted)],
            (Watched::Trusted, 3),
        ),
        (vec![Trust(InWait::Stop), Ready], (Watched::Stopped, 1)),
        (vec![other(), other()], (Watched::Other, 2)),
        (vec![other(), Nothing, other(), Ready], (Watched::Ready, 4)),
        (
            vec![
                other(),
                Other("Another?".to_owned()),
                Other("Another?".to_owned()),
            ],
            (Watched::Other, 3),
        ),
    ] {
        assert_eq!(run(&reads, 10), want, "{reads:?}");
    }
}

/// PINS (contract.md:337, "starts no read past it and never cuts a started
/// answer short"): the budget is asked BEFORE each read, never during one;
/// a watch that never settles stops at it, a not-yet answer retries only
/// inside it, and a spent budget starts no read at all.
#[test]
fn a_watch_never_starts_a_read_past_its_budget() {
    assert_eq!(run(&[], 5), (Watched::Deadline, 5));
    let again = vec![WatchRead::Trust(InWait::Again); 20];
    assert_eq!(run(&again, 3), (Watched::Deadline, 3));
    assert_eq!(run(&[WatchRead::Ready], 0), (Watched::Deadline, 0));
}
