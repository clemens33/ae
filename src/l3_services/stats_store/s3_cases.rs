//! Shared frozen S3 lifecycle oracle for unit acceptance and cargo-fuzz.
#![allow(dead_code, reason = "shared unit/fuzz scenario helpers")]
#![allow(clippy::expect_used, reason = "contract fixture assertions")]

#[cfg(test)]
use crate as ae;
use ae::stats::{self, ThisSession};
use ae::stats_store::disk::{
    self, Decision, Dir, Life, Limits, MemoryJournals, Next, Op, Plan, Prepared, Shown, SourceSpec,
    Standing,
};

#[path = "s2_cases.rs"]
mod base;
pub use base::{Feed, S, T, birth};

pub const LIMITS: Limits = Limits {
    chunk: 4096,
    line: 4096,
};

pub fn complete(dir: &mut dyn Dir, plan: &Plan) {
    plan.apply(dir, plan.ops.len())
        .expect("whole lifecycle plan");
}

pub fn plan(decision: Decision) -> Plan {
    match decision {
        Decision::Publish(plan) => plan,
        other => panic!("expected lifecycle plan: {other:?}"),
    }
}

pub fn work(
    dir: &mut dyn Dir,
    journals: &mut MemoryJournals,
    spec: &SourceSpec,
    life: &Life,
) -> Prepared {
    let observed = Prepared::read_life(dir, journals, spec, LIMITS, None, None, life)
        .expect("prepare lifecycle work");
    match observed.next {
        Next::Work(work) => work,
        other => panic!("expected work, got {other:?}"),
    }
}

pub fn check(dir: &mut dyn Dir, feeds: &[Feed]) {
    for scope in [None, Some(S), Some(T)] {
        let expected = stats::fold(
            feeds
                .iter()
                .filter(|f| f.active)
                .map(Feed::source)
                .collect(),
            scope,
        );
        let Shown::Ready { view, .. } = disk::shown(dir, scope) else {
            panic!("materialized lifecycle view for {scope:?}");
        };
        assert_eq!(view, expected, "all fields, scope {scope:?}");
        assert!(view.all.balanced());
        if let ThisSession::Ready(this) = view.this {
            assert!(this.balanced());
        }
    }
    let Shown::Ready { view, .. } = disk::resum(dir).expect("independent root sum") else {
        panic!("materialized root sum");
    };
    assert_eq!(
        view.all,
        stats::fold(
            feeds
                .iter()
                .filter(|f| f.active)
                .map(Feed::source)
                .collect(),
            None
        )
        .all
    );
}

pub fn recover(dir: &mut dyn Dir) {
    let redo = disk::recovery(dir).expect("lifecycle recovery");
    complete(dir, &redo);
    assert!(
        disk::recovery(dir).expect("second recovery").ops.is_empty(),
        "recovery twice equals once"
    );
}

pub fn seed(dir: &mut dyn Dir, journals: &mut MemoryJournals) -> Vec<Feed> {
    let mut feeds = vec![
        Feed::live("live/a", Some(S), [birth(S, 1), birth(T, 2)].concat()),
        Feed::live("live/b", Some(T), birth(T, 3)),
        Feed::live("live/idless", None, b"bad\n".to_vec()),
    ];
    for (index, feed) in feeds.iter_mut().enumerate() {
        feed.ino = 20 + index as u64;
        journals.set(feed.id(), 1, feed.ino, &feed.bytes);
        let prepared = work(dir, journals, &feed.spec, &Life::default());
        let publication = plan(disk::publish(dir, journals, &prepared).expect("seed publication"));
        complete(dir, &publication);
        feed.active = true;
        feed.cursor = feed.bytes.len() as u64;
    }
    check(dir, &feeds);
    feeds
}

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Remove,
    Replace,
    Reorigin,
    Rebind,
}

pub fn scenario(
    dir: &mut dyn Dir,
    journals: &mut MemoryJournals,
    kind: Kind,
) -> (Plan, Vec<Feed>, Vec<Feed>) {
    let before = seed(dir, journals);
    let mut after = before.clone();
    let publication = match kind {
        Kind::Remove => {
            after[0].active = false;
            plan(disk::remove(dir, Some(S), "live/a", None).expect("source removal"))
        }
        Kind::Replace => {
            after[0].spec = SourceSpec::Failed {
                id: "live/a".to_owned(),
                label: "live/a".to_owned(),
                session: Some(S.to_owned()),
                gap: stats::SourceGap::Missing,
            };
            after[0].cursor = 0;
            let prepared = work(dir, journals, &after[0].spec, &Life::default());
            plan(disk::publish(dir, journals, &prepared).expect("replace with failure"))
        }
        Kind::Reorigin => {
            after[0].spec = SourceSpec::Live {
                id: "live/a".to_owned(),
                label: "renamed".to_owned(),
                meta: format!("session_id={S}\nseat.spawned.9=unborn\n").into_bytes(),
            };
            let prepared = work(dir, journals, &after[0].spec, &Life::default());
            plan(disk::publish(dir, journals, &prepared).expect("refresh origin"))
        }
        Kind::Rebind => {
            let mut copy = before[0].clone();
            copy.spec = SourceSpec::Archive {
                id: format!("archive/{S}"),
                uuid: S.to_owned(),
            };
            copy.ino = 99;
            journals.set(copy.id(), 1, copy.ino, &copy.bytes);
            let life = Life {
                fingerprint: Some("a".repeat(64)),
                donors: vec!["live/a".to_owned()],
            };
            let prepared = work(dir, journals, &copy.spec, &life);
            assert_eq!(
                prepared.donor(),
                Some("live/a"),
                "a matching donor must actually be used"
            );
            assert_eq!(
                prepared.span().1,
                before[0].cursor,
                "copy the consumed prefix, not an arbitrary re-read"
            );
            after.push(copy);
            plan(disk::publish(dir, journals, &prepared).expect("rebind publication"))
        }
    };
    (publication, before, after)
}

pub fn committed(publication: &Plan, cut: usize) -> bool {
    let target = format!("stats/session/{S}.evidence");
    let boundary = publication
        .ops
        .iter()
        .position(|op| matches!(op, Op::Rename { to, .. } if to == &target))
        .expect("primary evidence commit boundary")
        + 1;
    cut >= boundary
}

pub fn crash(
    dir: &mut dyn Dir,
    journals: &mut MemoryJournals,
    kind: Kind,
    selector: usize,
    recut: Option<usize>,
) {
    let (publication, before, after) = scenario(dir, journals, kind);
    let cut = selector % (publication.ops.len() + 1);
    publication.apply(dir, cut).expect("lifecycle crash prefix");
    if let Some(recut) = recut {
        let recovery = disk::recovery(dir).expect("interrupted lifecycle recovery");
        recovery
            .apply(dir, recut % (recovery.ops.len() + 1))
            .expect("recovery prefix");
    }
    recover(dir);
    check(
        dir,
        if committed(&publication, cut) {
            &after
        } else {
            &before
        },
    );
    let pruning = disk::prune(dir, Some(S)).expect("prune crash leftovers");
    complete(dir, &pruning);
    recover(dir);
    check(
        dir,
        if committed(&publication, cut) {
            &after
        } else {
            &before
        },
    );
}

pub fn fuzz_case(data: &[u8]) {
    let selected = data.first().copied().unwrap_or_default();
    let index = usize::from(data.get(1).copied().unwrap_or_default());
    let mut dir = disk::Memory::new();
    let mut journals = MemoryJournals::new();
    match selected % 6 {
        0..=3 => {
            let kind = [Kind::Remove, Kind::Replace, Kind::Reorigin, Kind::Rebind]
                [usize::from(selected % 4)];
            crash(
                &mut dir,
                &mut journals,
                kind,
                index,
                data.get(2).copied().map(usize::from),
            );
        }
        4 => {
            let before = seed(&mut dir, &mut journals);
            let spec = SourceSpec::Archive {
                id: format!("archive/{S}"),
                uuid: S.to_owned(),
            };
            journals.set(spec.id(), 1, 91, &birth(S, 90));
            let prepared = work(&mut dir, &mut journals, &spec, &Life::default());
            journals.stand(
                spec.id(),
                if index & 1 == 0 {
                    Standing::Gone
                } else {
                    Standing::Claimed
                },
            );
            assert!(matches!(
                disk::publish(&mut dir, &mut journals, &prepared).expect("standing proof"),
                Decision::Discard(disk::Why::Gone)
            ));
            check(&mut dir, &before);
        }
        _ => {
            let before = seed(&mut dir, &mut journals);
            let at = before[0].cursor;
            assert!(matches!(
                disk::remove(&mut dir, Some(S), "live/a", Some((u64::MAX, at)))
                    .expect("stamp proof"),
                Decision::Discard(disk::Why::Stamp)
            ));
            check(&mut dir, &before);
        }
    }
}
