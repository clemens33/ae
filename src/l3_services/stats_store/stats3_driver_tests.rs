//! #287 slice 3 driver tests: a view no fold could make is refused.

use crate::stats::{self, Source, ThisSession};
use crate::stats_fixture as f;

/// One failed start for birth `n` of `session`, its certificate first.
fn failed(session: &str, n: u32) -> String {
    let start = format!(",\"start\":\"spawned.0:{}:7\"", f::id(n));
    f::birth(
        session,
        n,
        &f::fields("claude", None, Some("sonnet"), Some("high")),
    ) + &f::fact(session, n, "start-attempted", &start)
        + &f::fact(session, n, "start-failed", &start)
        + &f::fact(session, n, "brief-failed", "")
}

#[test]
fn a_session_scope_cannot_drop_outcomes_its_complement_has_no_birth_for() {
    let source = |name: &str, session: &str, n| {
        Source::live(
            name,
            format!("session_id={session}\n").as_bytes(),
            Ok(failed(session, n).into_bytes()),
        )
    };
    let view = stats::fold(vec![source("S", f::S, 1), source("T", f::T, 2)], Some(f::S));
    assert_eq!(
        (view.all.total, view.all.start_failed, view.all.brief_failed),
        (2, 2, 2)
    );
    let wire = super::render_view(&view);
    assert_eq!(
        super::parse_view(&wire).ok(),
        Some(view.clone()),
        "the fold's own view"
    );
    for brief in [false, true] {
        let mut forged = view.clone();
        let ThisSession::Ready(this) = &mut forged.this else {
            panic!("S has a session id");
        };
        // One birth outside this session cannot hold both of all's failures.
        if brief {
            this.brief_failed = 0;
        } else {
            this.start_failed = 0;
        }
        assert!(this.balanced());
        assert!(
            super::parse_view(&super::render_view(&forged)).is_err(),
            "brief={brief}: a view no fold makes"
        );
    }
}

/// The tracked nonzero view seed is a view the codec accepts.
#[test]
fn the_nonzero_outcome_view_seed_parses() {
    let seed = include_str!("../../../fuzz/seeds/stats_store/view-hand-outcomes-nonzero");
    let view = super::parse_view(seed).expect("a valid nonzero view");
    assert_eq!((view.all.brief_failed, view.all.start_failed), (1, 1));
    assert_eq!(
        super::render_view(&view),
        seed,
        "renders back byte for byte"
    );
}
