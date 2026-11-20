//! Unit pins for the board machine's rules that the acceptance specs reach
//! only through one scenario each: the checkpoint grammar, the pin spacing,
//! the hold over memory, the told write failure and the namesake line.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "these unit tests own their scratch files and label failed fixtures"
)]

use std::fs;
use std::path::{Path, PathBuf};

use super::{Machine, PIN_FILE, Pin, parse_pin, pin_delay};
use crate::desk::{Desk, parse_input};
use crate::telegram::tests::{CHAT, Fake, Reply, with_unwritable_dir};
use crate::time::Timestamp;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "ae-desk-unit-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).expect("scratch directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn board(text: &str, epoch: i64) -> Desk {
    Desk {
        updated: Timestamp::from_epoch(epoch),
        items: parse_input(format!("DECIDE\nD1 | {text}\n").as_bytes()).expect("fixture items"),
    }
}

fn by_id(id: i64) -> Reply {
    Reply::json(
        200,
        &format!(r#"{{"ok":true,"result":{{"message_id":{id}}}}}"#),
    )
}

fn pins(fake: &Fake) -> usize {
    fake.requests()
        .iter()
        .filter(|seen| seen.path.ends_with("/pinChatMessage"))
        .count()
}

#[test]
fn a_checkpoint_carries_every_positive_i64_message_id() {
    let max = "ae-desk-pin 1\nchat -1001234567890\nmessage 9223372036854775807\npinned 1\n";
    let pin = parse_pin(max.as_bytes()).expect("the largest id");
    assert_eq!(pin.encode(), max);
    assert_eq!(pin.placed.map(|placed| placed.message), Some(i64::MAX));
    for message in [
        "9223372036854775808",
        "0",
        "042",
        "00",
        "-1",
        "+1",
        "",
        "10000000000000000000",
    ] {
        let text = format!("ae-desk-pin 1\nchat -1001234567890\nmessage {message}\npinned 0\n");
        assert!(
            parse_pin(text.as_bytes()).is_err(),
            "accepted id {message:?}"
        );
    }
}

#[test]
fn a_checkpoint_chat_is_letters_digits_at_underscore_and_hyphen() {
    for chat in ["-1001234567890", "@board_chat", "a-B_9", &"x".repeat(64)] {
        let text = format!("ae-desk-pin 1\nchat {chat}\n");
        assert_eq!(parse_pin(text.as_bytes()).expect("a valid chat").chat, chat);
    }
    for chat in [
        "",
        "has space",
        "tab\t",
        "caf\u{e9}",
        "a/b",
        "a.b",
        "!",
        &"x".repeat(65),
    ] {
        let text = format!("ae-desk-pin 1\nchat {chat}\n");
        assert!(
            parse_pin(text.as_bytes()).is_err(),
            "accepted chat {chat:?}"
        );
    }
}

#[test]
fn a_pin_waits_forty_seconds_doubling_to_ten_minutes() {
    let waits: Vec<u64> = (0..8).map(pin_delay).collect();
    assert_eq!(waits, [0, 40, 80, 160, 320, 600, 600, 600]);
}

#[test]
fn an_unreadable_checkpoint_holds_even_when_a_message_waits_in_memory() {
    let dir = Scratch::new("hold-memory");
    let fake = Fake::start(vec![by_id(42)]);
    let api = fake.api();
    let mut machine = Machine::new(dir.path().to_owned());
    let mut err = Vec::new();
    let first = board("kept in memory", 1000);
    with_unwritable_dir(dir.path(), || {
        let _ = machine.pass(&api, CHAT, Some(&first), 1000, &mut err);
    });
    assert_eq!(fake.requests().len(), 1);
    fs::write(dir.path().join(PIN_FILE), "damaged\n").expect("a damaged checkpoint");
    let held = machine.pass(&api, CHAT, Some(&first), 2000, &mut err);
    assert!(held.held.is_some());
    assert_eq!(fake.requests().len(), 1, "a hold makes no call");
    assert_eq!(
        fs::read_to_string(dir.path().join(PIN_FILE)).expect("the checkpoint"),
        "damaged\n",
        "nothing writes over an unreadable checkpoint"
    );
    let missing = machine.pass(&api, CHAT, None, 2001, &mut err);
    assert!(
        missing.held.is_some(),
        "a missing board still reports the hold"
    );
    machine.unreadable(CHAT, "no good", &mut err);
    let told = String::from_utf8_lossy(&err).into_owned();
    assert!(!told.contains("the board is unreadable"), "{told}");
}

#[test]
fn a_checkpoint_the_disk_refuses_is_told_once_and_the_pin_waits() {
    let dir = Scratch::new("write-told");
    let fake = Fake::start(vec![by_id(42)]);
    let api = fake.api();
    let mut machine = Machine::new(dir.path().to_owned());
    let mut err = Vec::new();
    let first = board("accepted and not saved", 1000);
    with_unwritable_dir(dir.path(), || {
        for now in [1000, 1001, 1002] {
            let _ = machine.pass(&api, CHAT, Some(&first), now, &mut err);
        }
    });
    let told = String::from_utf8_lossy(&err).into_owned();
    assert_eq!(
        told.matches("the checkpoint of board message 42 was not written")
            .count(),
        1,
        "{told}"
    );
    assert!(told.contains("the pin waits until it is"), "{told}");
    assert_eq!(pins(&fake), 0);
}

#[test]
fn a_pin_that_is_due_is_made_while_an_edit_is_due_on_every_pass() {
    let dir = Scratch::new("pin-starved");
    let refused = Reply::json(
        403,
        r#"{"ok":false,"error_code":403,"description":"Forbidden"}"#,
    );
    let fake = Fake::start(vec![by_id(42), refused, by_id(42)]);
    let api = fake.api();
    let mut machine = Machine::new(dir.path().to_owned());
    let mut err = Vec::new();
    let _ = machine.pass(&api, CHAT, Some(&board("first", 1000)), 1000, &mut err);
    assert_eq!(pins(&fake), 1, "the first pin is refused");
    for now in 1001..=1039 {
        let text = format!("change {now}");
        let _ = machine.pass(&api, CHAT, Some(&board(&text, now)), now, &mut err);
    }
    assert_eq!(pins(&fake), 1, "the pin waits forty seconds");
    let _ = machine.pass(
        &api,
        CHAT,
        Some(&board("change 1040", 1040)),
        1040,
        &mut err,
    );
    assert_eq!(
        pins(&fake),
        2,
        "a due pin goes with the edit of the same pass"
    );
    let stored = fs::read_to_string(dir.path().join(PIN_FILE)).expect("checkpoint");
    assert_eq!(
        stored,
        Pin {
            chat: CHAT.to_owned(),
            placed: Some(super::Placed {
                message: 42,
                pinned: true
            })
        }
        .encode()
    );
}

#[test]
fn a_namesake_of_the_seat_is_told_once_per_episode() {
    let dir = Scratch::new("namesake");
    let mut machine = Machine::new(dir.path().to_owned());
    let mut err = Vec::new();
    let held = Path::new("/state/sessions/seat");
    let other = Path::new("/state/worktrees/topic/.ae/seat");
    machine.namesake("seat", other, held, &mut err);
    machine.namesake("seat", other, held, &mut err);
    machine.namesake_over();
    machine.namesake("seat", other, held, &mut err);
    let told = String::from_utf8_lossy(&err).into_owned();
    let line = "ae: telegram: desk skipped: running session seat is at /state/worktrees/topic/.ae/seat, not the census-held /state/sessions/seat";
    assert_eq!(told.matches(line).count(), 2, "{told}");
}
