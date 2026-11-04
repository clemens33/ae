//! Driver-owned unit tests of the stop's long-limit pieces (#304): which
//! refusal names `background-task`, and the last word taken under every lock
//! right before the write — the third reading, the fresh clock and the
//! keyboard.
use super::{HarnessState, LongLimit, ToolKind, background_only, long_limit_last_word};
use crate::tmux::ObservedClient;

const PLAIN: &str = include_str!("../../tests/fixtures/harness-state/muse-limit-plain.txt");
const WATCHING: &str = include_str!("../../tests/fixtures/harness-state/muse-limit-watching.txt");
const TITLE: &str = "background evidence watch";
/// `date -u -j -f %Y-%m-%dT%H:%M:%SZ 2027-01-15T08:00:00Z +%s`.
const RESET: i64 = 1_800_000_000;
const NOW: i64 = RESET - 86_400;
const ASK: LongLimit = LongLimit {
    key: NOW - 3600,
    opened: NOW - 30,
};
const PANE: &str = "%7";
const NO_CLIENTS: &[ObservedClient] = &[];

/// The watching stop under its own wrapped 429 row, resetting at [`RESET`].
fn long() -> String {
    format!(
        "◆ model failed: API error 429 [request_id=00000000-0000-0000-0000-000000000000]:\n  \
         Subscription quota exhausted. Your usage window resets at\n  2027-01-15T08:00:00Z. \
         (rate_limit_error)\n\n{WATCHING}"
    )
}

fn client(pane: &str, activity: Option<u64>) -> ObservedClient {
    ObservedClient {
        name: "/dev/ttys001".to_owned(),
        session: "aedev".to_owned(),
        pane: pane.to_owned(),
        activity,
    }
}

fn last_word(
    third: Option<&str>,
    clients: Option<&[ObservedClient]>,
) -> Result<Vec<String>, &'static str> {
    let frame = long();
    long_limit_last_word(
        [Some(frame.as_str()), Some(frame.as_str()), third],
        ASK,
        NOW,
        (clients, PANE),
    )
}

#[test]
fn three_readings_alike_with_no_new_input_end_the_named_task() {
    let frame = long();
    assert_eq!(
        last_word(Some(frame.as_str()), Some(NO_CLIENTS)),
        Ok(vec![TITLE.to_owned()])
    );
    // Input before the attempt opened was the attempt's own judgement, and a
    // client elsewhere, whatever its input, is no input here.
    let opened = u64::try_from(ASK.opened).unwrap_or_default();
    for clients in [
        vec![client(PANE, Some(opened - 1))],
        vec![client("%8", Some(opened + 60))],
        vec![client("%8", None), client(PANE, Some(opened - 1))],
    ] {
        assert_eq!(
            last_word(Some(frame.as_str()), Some(clients.as_slice())),
            Ok(vec![TITLE.to_owned()]),
            "{clients:?}"
        );
    }
}

#[test]
fn unread_clients_or_input_since_the_attempt_opened_refuses() {
    let frame = long();
    assert_eq!(
        last_word(Some(frame.as_str()), None),
        Err("ae cannot read the pane's clients")
    );
    let opened = u64::try_from(ASK.opened).unwrap_or_default();
    for at in [opened, opened + 1, u64::MAX] {
        assert_eq!(
            last_word(
                Some(frame.as_str()),
                Some(&[client("%8", None), client(PANE, Some(at))][..])
            ),
            Err("a client gave the pane input since the move began"),
            "{at}"
        );
    }
    // A client viewing the pane whose last input ae cannot read may have just
    // typed: unknown input refuses at the stop, whatever the others say.
    for clients in [
        vec![client(PANE, None)],
        vec![client(PANE, Some(opened - 1)), client(PANE, None)],
    ] {
        assert_eq!(
            last_word(Some(frame.as_str()), Some(clients.as_slice())),
            Err("ae cannot read the input of a client viewing the pane"),
            "{clients:?}"
        );
    }
}

#[test]
fn a_missing_or_changed_third_reading_refuses() {
    assert_eq!(
        last_word(None, Some(NO_CLIENTS)),
        Err("ae could not read the frame again")
    );
    let frame = long();
    for (third, why) in [
        (
            frame.replace("\n❯\n", "\n❯ a human draft\n"),
            "the frame changed between readings",
        ),
        (
            frame.replace(TITLE, "another task"),
            "its tasks changed between readings",
        ),
        (
            frame.replace("2027-01-15", "2027-01-16"),
            "the vendor reset changed between readings",
        ),
        (
            WATCHING.to_owned(),
            "no absolute vendor reset proves the limit 12 h away",
        ),
    ] {
        assert_eq!(
            last_word(Some(third.as_str()), Some(NO_CLIENTS)),
            Err(why),
            "{third}"
        );
    }
}

#[test]
fn the_last_word_reads_the_clock_it_is_handed() {
    let frame = long();
    let late = RESET - 43_199;
    assert_eq!(
        long_limit_last_word(
            [Some(frame.as_str()); 3],
            ASK,
            late,
            (Some(NO_CLIENTS), PANE)
        ),
        Err("its limit resets in under 12 h")
    );
    let early = ASK.key + 599;
    assert_eq!(
        long_limit_last_word(
            [Some(frame.as_str()); 3],
            ASK,
            early,
            (Some(NO_CLIENTS), PANE)
        ),
        Err("its limit is under 10 min old")
    );
}

#[test]
fn only_a_decisive_background_only_muse_reading_names_the_task() {
    let unknown = |capture| (HarnessState::Unknown, false, capture);
    let watching = unknown(Some(WATCHING));
    let draft = WATCHING.replace("\n❯\n", "\n❯ draft\n");
    let stray = WATCHING.replace("main · ↓", "human prompt\nmain · ↓");
    assert!(background_only(watching, watching, ToolKind::Muse));
    assert!(background_only(
        watching,
        unknown(Some(&draft)),
        ToolKind::Muse
    ));
    // The FIRST reading that proves nothing decides, as the refusal's words do.
    assert!(background_only(
        (HarnessState::Unknown, true, Some(PLAIN)),
        watching,
        ToolKind::Muse
    ));
    assert!(background_only(
        (HarnessState::Idle, false, Some(PLAIN)),
        watching,
        ToolKind::Muse
    ));
    assert!(!background_only(
        unknown(Some(&draft)),
        watching,
        ToolKind::Muse
    ));
    assert!(!background_only(
        unknown(Some(&stray)),
        watching,
        ToolKind::Muse
    ));
    assert!(!background_only(unknown(None), watching, ToolKind::Muse));
    // Two proving readings decide nothing, and no other tool's rows are read.
    let proven = (HarnessState::Unknown, true, Some(WATCHING));
    assert!(!background_only(proven, proven, ToolKind::Muse));
    assert!(!background_only(watching, watching, ToolKind::Claude));
}
