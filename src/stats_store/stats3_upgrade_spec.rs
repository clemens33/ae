//! Frozen #287 slice 3, Q1: genuine old bytes, one rebuild, hand-counted totals.
//! The fixture was captured from ea6def36245d44b97627a3a3bc1cfa00645243cf
//! on the hp before the outcome implementation. It is never regenerated here.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures copy and inspect only their own store payload"
)]

use super::disk::{self, Files, Glance};
use super::maint::Outcome;
use super::s5_fixture::{Fixture, S, birth, drain};
use crate::json::{self, Value};
use std::fs;

const OLD: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/stats3-v1");

fn install_old(root: &Fixture) {
    let payload = root.0.join("stats");
    fs::create_dir_all(payload.join("session")).expect("fixture store directories");
    for name in [
        "epoch",
        "view",
        "session/11111111-1111-4111-8111-111111111111.evidence",
        "session/11111111-1111-4111-8111-111111111111.view",
    ] {
        fs::copy(format!("{OLD}/stats/{name}"), payload.join(name)).expect("frozen format-1 bytes");
    }
}

fn counter(scope: &crate::stats::Scoped, name: &str, expected: i64) -> i64 {
    let projected = crate::stats::View {
        all: scope.clone(),
        this: crate::stats::ThisSession::NoUuid,
    };
    let wire = super::render_view(&projected);
    let head = json::parse(wire.lines().nth(1).expect("scope header")).expect("view JSON");
    assert_eq!(
        head.get(name),
        Some(&Value::Num(expected)),
        "hand total {name}: {wire}"
    );
    let Some(Value::Num(actual)) = head.get(name) else {
        panic!("numeric outcome counter {name}");
    };
    *actual
}

#[test]
fn stats3_fresh_process_rebuilds_a_real_format_one_store_once_without_losing_births() {
    let root = Fixture::new();
    let bytes = fs::read(format!("{OLD}/events.jsonl")).expect("frozen original journal");
    let dir = root.live("old", S, &bytes);
    install_old(&root);
    let before = fs::read_to_string(root.0.join("stats/epoch")).expect("old epoch");
    assert_eq!(before, "0\n");
    let mut upkeep = root.maintainer();
    let mut repairs = 0;
    let mut settled = false;
    for _ in 0..128 {
        let step = upkeep.step();
        assert!(
            !matches!(step.outcome, Outcome::Refused(_)),
            "upgrade refused: {step:?}"
        );
        if matches!(step.outcome, Outcome::Repaired { .. }) {
            repairs += 1;
        }
        if step.outcome == Outcome::Idle {
            settled = true;
            break;
        }
    }
    assert!(settled, "bounded fixture rebuild completes");
    assert_eq!(
        repairs, 1,
        "new-format process repairs the old payload once"
    );
    assert!(!upkeep.stopped());
    let Glance::Ready {
        view, all, epoch, ..
    } = disk::glance(&mut Files::at(&root.0), Some(S))
    else {
        panic!("rebuilt format-1 store must publish a readable view");
    };
    assert!(epoch > 0);
    assert_eq!(all.pending, 0);
    for scope in [
        &view.all,
        match &view.this {
            crate::stats::ThisSession::Ready(scope) => scope,
            crate::stats::ThisSession::NoUuid => panic!("known session must remain available"),
        },
    ] {
        assert_eq!(
            scope.total, 2,
            "two original certificates, independent of outcomes"
        );
        counter(scope, "brief_failed", 1);
        let failed = counter(scope, "start_failed", 1);
        assert_eq!(
            scope.total - u64::try_from(failed).expect("nonnegative failed count"),
            1,
            "birth 2 has one unproved start after the rebuild"
        );
        assert!(scope.balanced());
    }
    // Further upkeep must stay usable after the repair episode closes.
    let mut appended = bytes;
    appended.extend(birth(S, 3));
    fs::write(dir.join("events.jsonl"), appended).expect("one later real birth");
    drain(&mut upkeep);
    assert!(!upkeep.stopped());
    let Glance::Ready {
        view, epoch: later, ..
    } = disk::glance(&mut Files::at(&root.0), Some(S))
    else {
        panic!("upkeep still publishes after upgrading");
    };
    assert_eq!(
        later, epoch,
        "ordinary append does not trigger another rebuild"
    );
    assert_eq!(view.all.total, 3);
    counter(&view.all, "brief_failed", 1);
    counter(&view.all, "start_failed", 1);
    let mut next_process = root.maintainer();
    for _ in 0..8 {
        let step = next_process.step();
        assert!(
            !matches!(step.outcome, Outcome::Repaired { .. } | Outcome::Refused(_)),
            "{step:?}"
        );
    }
    assert!(!next_process.stopped());
}
