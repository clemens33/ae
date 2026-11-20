//! #287 slice 3 driver tests: outcome rules the frozen acceptance leaves open.

use crate::stats::{self, Reason, Source};
use crate::stats_fixture as f;

fn source(label: &str, session: &str, bytes: &[u8]) -> Source {
    Source::live(
        label,
        format!("session_id={session}\n").as_bytes(),
        Ok(bytes.to_vec()),
    )
}

/// #287 slice 3, Q3: a birth whose certificates conflict proves no outcome,
/// even when every source agrees on its facts.
#[test]
fn a_certificate_conflict_counts_no_outcome_from_agreed_facts() {
    let cert = |model| {
        f::birth(
            f::S,
            1,
            &f::fields("claude", None, Some(model), Some("high")),
        )
    };
    let start = format!(",\"start\":\"spawned.0:{}:7\"", f::id(1));
    let facts = f::fact(f::S, 1, "start-attempted", &start)
        + &f::fact(f::S, 1, "start-failed", &start)
        + &f::fact(f::S, 1, "brief-failed", "");
    let view = stats::fold(
        vec![
            source("one", f::S, (cert("sonnet") + &facts).as_bytes()),
            source("two", f::S, (cert("opus") + &facts).as_bytes()),
        ],
        None,
    );
    assert_eq!(view.all.total, 1);
    assert!(
        view.all
            .gaps
            .iter()
            .any(|gap| gap.reason == Reason::Conflict),
        "{view:?}"
    );
    assert_eq!(view.all.brief_failed, 0, "a conflicted birth is unproved");
    assert_eq!(view.all.start_failed, 0, "a conflicted birth is unproved");
    assert_eq!(view.all.start_unproved(), 1);
}

/// The tracked fuzz seed holds every outcome rule: a failed start with an
/// undelivered brief, a bare attempt, and a failure retried by an attempt.
#[test]
fn the_outcome_fuzz_seed_folds_to_one_failed_start_of_three() {
    let journal = include_bytes!("../../fuzz/seeds/stats_fold/outcomes");
    let view = stats::fold(vec![source("a", f::S, journal)], Some(f::S));
    let all = &view.all;
    assert_eq!((all.total, all.legacy), (3, 0));
    assert_eq!((all.brief_failed, all.start_failed), (1, 1));
    assert_eq!(all.start_unproved(), 2);
    let store = include_bytes!("../../fuzz/seeds/stats_store/mode0-outcomes");
    assert_eq!(
        store.first().map(|mode| mode % 16),
        Some(0),
        "the journal mode"
    );
    assert_eq!(store.get(2..), Some(&journal[..]), "the same journal");
}
