//! The helper's words for the end of a set: the stream, the exit status and
//! the text of every result, and nothing on stdout for an error.

use super::{ReadError, Rendered, SetError, SetOutcome, render_set};
use crate::desk::{Delta, Id, Section};

fn changed(total: usize) -> SetOutcome {
    SetOutcome::Changed {
        delta: Delta {
            added: vec![Id {
                section: Section::Decide,
                number: 7,
            }],
            removed: vec![],
            changed: vec![Id {
                section: Section::Running,
                number: 2,
            }],
        },
        total,
    }
}

fn refused(why: &str) -> Rendered {
    Rendered {
        code: 1,
        stdout: String::new(),
        stderr: format!("desk set REFUSED: {why}\n"),
    }
}

#[test]
fn a_change_prints_its_count_and_ids_to_stdout_at_zero() {
    let done = render_set(&Ok(changed(2)), 2);
    assert_eq!(done.code, 0);
    assert_eq!(done.stdout, "desk set: 2 items written (+D7 ~R2)\n");
    assert!(done.stderr.is_empty());
    assert_eq!(
        render_set(&Ok(changed(1)), 1).stdout,
        "desk set: 1 item written (+D7 ~R2)\n"
    );
}

#[test]
fn an_unchanged_board_says_so_at_zero_with_its_count() {
    let same = render_set(&Ok(SetOutcome::Unchanged), 3);
    assert_eq!(same.code, 0);
    assert_eq!(same.stdout, "desk set: unchanged (3 items)\n");
    assert!(same.stderr.is_empty());
    assert_eq!(
        render_set(&Ok(SetOutcome::Unchanged), 1).stdout,
        "desk set: unchanged (1 item)\n"
    );
}

#[test]
fn a_set_that_changed_nothing_is_refused_with_the_reason_at_one() {
    for (error, why) in [
        (
            SetError::Admission("this session is not an orchestrator".to_owned()),
            "this session is not an orchestrator",
        ),
        (SetError::Lock("held".to_owned()), "held"),
        (
            SetError::Blocked(ReadError::NotRegular),
            "not a regular file",
        ),
        (SetError::Write("disk full".to_owned()), "disk full"),
    ] {
        let rendered = render_set(&Err(error.clone()), 2);
        assert_eq!(rendered.code, 1, "{error:?}");
        assert!(rendered.stdout.is_empty(), "{error:?}");
        assert!(
            rendered.stderr.starts_with("desk set REFUSED: ") && rendered.stderr.contains(why),
            "{error:?}: {}",
            rendered.stderr
        );
        assert!(rendered.stderr.ends_with('\n'));
    }
    assert_eq!(
        render_set(&Err(SetError::Admission("x".to_owned())), 1),
        refused("x")
    );
}

#[test]
fn a_replaced_board_without_its_event_says_failed_after_write_at_two() {
    let rendered = render_set(&Err(SetError::Event("journal blocked".to_owned())), 3);
    assert_eq!(rendered.code, 2);
    assert!(rendered.stdout.is_empty());
    assert_eq!(
        rendered.stderr,
        "desk set FAILED AFTER WRITE: the board was replaced (3 items) and its journal event was not recorded (the detail says when the journal's state is unknown): journal blocked\n"
    );
    assert!(!rendered.stderr.contains("REFUSED"));
}

#[test]
fn a_replaced_board_whose_entry_was_not_synced_says_unconfirmed_at_two_with_no_event() {
    let rendered = render_set(&Err(SetError::Unsynced("fsync failed".to_owned())), 1);
    assert_eq!(rendered.code, 2);
    assert!(rendered.stdout.is_empty());
    assert_eq!(
        rendered.stderr,
        "desk set UNCONFIRMED: the board was replaced (1 item) but its directory entry could not be synced, so it may not survive a crash, and no journal event was written: fsync failed\n"
    );
    assert!(!rendered.stderr.contains("REFUSED"));
}
