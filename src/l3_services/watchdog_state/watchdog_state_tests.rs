//! The checkpoint's locked operations: the spacing stamps, and the desk
//! claim's compare-and-swap against the stored claim and overview delivery.

use super::{
    DeskTick, Stamps, claim_desk_tick, desk_tick, record_overview_delivery, restore_desk_tick,
    stamps,
};
use crate::events::Cursor;
use std::path::PathBuf;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae-wdstate-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn claim(at: i64, offset: u64) -> DeskTick {
    DeskTick {
        last_at: Some(at),
        cursor: [(
            "0199c0de-5555-4890-abcd-ef0123456789".to_owned(),
            Cursor {
                generation: 0,
                offset,
            },
        )]
        .into(),
    }
}

#[test]
fn the_stamps_read_both_halves_of_the_shared_spacing() {
    let scratch = Scratch::new("stamps");
    assert_eq!(stamps(&scratch.0), Stamps::default());
    record_overview_delivery(&scratch.0, "0123456789abcdef", 90, 100).expect("overview");
    assert!(
        claim_desk_tick(&scratch.0, &DeskTick::default(), Some(100), &claim(200, 64))
            .expect("claim")
    );
    assert_eq!(
        stamps(&scratch.0),
        Stamps {
            overview_last: Some(100),
            desk_last: Some(200),
        }
    );
}

#[test]
fn a_claim_stands_down_when_the_overview_or_the_stored_claim_moved() {
    let scratch = Scratch::new("claim");
    record_overview_delivery(&scratch.0, "0123456789abcdef", 90, 100).expect("overview");
    assert!(
        !claim_desk_tick(&scratch.0, &DeskTick::default(), Some(99), &claim(200, 64))
            .expect("an overview delivered since the observation")
    );
    assert_eq!(desk_tick(&scratch.0), DeskTick::default());
    assert!(
        !claim_desk_tick(&scratch.0, &claim(150, 32), Some(100), &claim(200, 64))
            .expect("a stored claim other than the one read")
    );
    assert!(
        claim_desk_tick(&scratch.0, &DeskTick::default(), Some(100), &claim(200, 64))
            .expect("both as read")
    );
    assert_eq!(desk_tick(&scratch.0), claim(200, 64));
    assert_eq!(
        stamps(&scratch.0).overview_last,
        Some(100),
        "overview fields stay"
    );
}

#[test]
fn a_restore_puts_the_previous_claim_back_only_over_its_own_claim() {
    let scratch = Scratch::new("restore");
    let previous = claim(100, 32);
    assert!(claim_desk_tick(&scratch.0, &DeskTick::default(), None, &previous).expect("first"));
    assert!(claim_desk_tick(&scratch.0, &previous, None, &claim(200, 64)).expect("second"));
    assert!(
        !restore_desk_tick(&scratch.0, &claim(300, 96), &previous)
            .expect("a newer claim is never overwritten")
    );
    assert_eq!(desk_tick(&scratch.0), claim(200, 64));
    assert!(restore_desk_tick(&scratch.0, &claim(200, 64), &previous).expect("its own claim"));
    assert_eq!(desk_tick(&scratch.0), previous);
}
