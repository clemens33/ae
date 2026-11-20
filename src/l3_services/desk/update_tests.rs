//! The time a board change is stamped with: later than the record it replaces
//! at the first change in a second, and never earlier than the one before.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "this unit test owns and inspects its scratch files"
)]

use std::fs;
use std::path::PathBuf;

use super::{next_update, read, set};
use crate::desk::parse_input;
use crate::time::Timestamp;

const NOW: i64 = 1_795_164_012;

fn stamp(now: i64, replaced: Option<i64>) -> i64 {
    next_update(
        Timestamp::from_epoch(now),
        replaced.map(Timestamp::from_epoch),
    )
    .epoch()
}

#[test]
fn a_stamp_is_now_unless_the_replaced_time_is_this_second_or_the_next() {
    assert_eq!(stamp(NOW, None), NOW, "the first record");
    assert_eq!(stamp(NOW, Some(NOW - 5)), NOW, "an older record");
    assert_eq!(stamp(NOW, Some(NOW)), NOW + 1, "the same second ticks");
    assert_eq!(
        stamp(NOW, Some(NOW + 1)),
        NOW + 1,
        "a record already one second ahead keeps its second"
    );
    assert_eq!(
        stamp(NOW, Some(NOW + 2)),
        NOW,
        "two seconds ahead is not trusted"
    );
    assert_eq!(
        stamp(NOW, Some(253_402_300_799)),
        NOW,
        "a hostile future date is not carried"
    );
}

#[test]
fn a_tick_past_the_last_canonical_time_stays_on_now() {
    let last = 253_402_300_799;
    assert_eq!(stamp(last, Some(last)), last);
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn three_changes_in_one_second_never_show_an_earlier_time() {
    let dir = Scratch(std::env::temp_dir().join(format!(
        "ae-desk-update-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    )));
    fs::create_dir(&dir.0).expect("private state directory");
    let now = Timestamp::from_epoch(NOW);
    let mut shown = Vec::new();
    for text in ["first", "second", "third"] {
        let items = parse_input(format!("DECIDE\nD1 | {text}\n").as_bytes()).expect("input");
        set(&dir.0, items, now, "boss").expect("set");
        shown.push(read(&dir.0).ok().flatten().expect("record").updated.epoch());
    }
    assert_eq!(shown, [NOW, NOW + 1, NOW + 1]);
}
