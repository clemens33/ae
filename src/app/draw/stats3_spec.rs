//! Frozen #287 slice 3: counts and wording from journal bytes, never from the
//! fold's own values. Every expectation below is a hand-counted birth outcome.
#![allow(clippy::expect_used, reason = "acceptance fixture contracts")]

use super::{Paint, settings_body};
use crate::app::model::SettingsTab;
use crate::app::settings::{SettingsBodies, StatsScope};
use crate::stats::{self, Source, View};
use crate::stats_fixture as f;
use crate::stats_store::{self, Store};

fn certificate(session: &str, n: u32) -> String {
    f::birth(
        session,
        n,
        &f::fields("claude", Some("route"), Some("sonnet"), Some("high")),
    )
}

fn fact(session: &str, n: u32, kind: &str, pid: u32) -> String {
    let fields = if kind == "brief-failed" {
        String::new()
    } else {
        format!(",\"start\":\"spawned.0:{}:{pid}\"", f::id(n))
    };
    f::fact(session, n, kind, &fields)
}

fn view(journals: Vec<(&str, &str, String)>, this: Option<&str>) -> View {
    stats::fold(
        journals
            .into_iter()
            .map(|(name, session, text)| {
                Source::live(
                    name,
                    format!("session_id={session}\n").as_bytes(),
                    Ok(text.into_bytes()),
                )
            })
            .collect(),
        this,
    )
}

fn rendered(view: &View, scope: StatsScope, width: u16) -> String {
    let mut bodies = SettingsBodies::default();
    bodies.stats.view = Some(Box::new(view.clone()));
    bodies.stats.scope = scope;
    settings_body(
        SettingsTab::Statistics,
        &bodies,
        Paint::of(None),
        width,
        false,
    )
    .into_iter()
    .map(|(text, _)| text)
    .collect::<Vec<_>>()
    .join("\n")
}

fn outcomes(view: &View, scope: StatsScope, total: u64, brief: u64, failed: u64, unproved: u64) {
    let text = rendered(view, scope, 240);
    for expected in [
        format!("Recorded worker spawns: {total}"),
        format!("Brief undelivered: {brief}"),
        format!("Start failed: {failed}"),
        format!("Start unproved: {unproved}"),
    ] {
        assert!(text.contains(&expected), "missing {expected}: {text}");
    }
    assert!(view.all.balanced());
    assert!(
        !text
            .split(|c: char| !c.is_alphabetic())
            .any(|word| word.eq_ignore_ascii_case("started")),
        "an attempt never becomes a successful start: {text}"
    );
    assert!(!text.contains(f::PRIVATE));
}

#[test]
fn stats3_outcomes_are_per_birth_and_brief_failure_is_independent_of_start_failure() {
    // S: birth 1 has a failed brief only; 2 has exec failure only; 3 has
    // both; 4 has neither; 5 is legacy. T adds one attempted start only.
    let mut s = String::new();
    for n in 1..=4 {
        s.push_str(&certificate(f::S, n));
        s.push_str(&f::link(n));
    }
    for n in [1, 3] {
        s.push_str(&fact(f::S, n, "brief-failed", 0));
    }
    for n in [2, 3] {
        s.push_str(&fact(f::S, n, "start-attempted", n));
        s.push_str(&fact(f::S, n, "start-failed", n));
    }
    s.push_str(&f::legacy(f::PRIVATE));
    let t = certificate(f::T, 6) + &fact(f::T, 6, "start-attempted", 6);
    let result = view(vec![("S", f::S, s), ("T", f::T, t)], Some(f::S));
    outcomes(&result, StatsScope::All, 6, 2, 2, 4);
    outcomes(&result, StatsScope::ThisSession, 5, 2, 2, 3);
    // Narrow rows wrap the complete outcome sentence instead of clipping counts.
    let narrow = rendered(&result, StatsScope::All, 40);
    let words = narrow.split_whitespace().collect::<Vec<_>>().join(" ");
    for label in [
        "Brief undelivered: 2",
        "Start failed: 2",
        "Start unproved: 4",
    ] {
        assert!(words.contains(label), "narrow view lost {label}: {narrow}");
    }
}

#[test]
fn stats3_child_facts_before_the_parent_certificate_join_without_creating_an_extra_birth() {
    let before = fact(f::S, 1, "start-attempted", 10) + &fact(f::S, 1, "start-failed", 10);
    let first = view(vec![("S", f::S, before.clone())], Some(f::S));
    outcomes(&first, StatsScope::All, 0, 0, 0, 0);
    let after = view(
        vec![("S", f::S, before + &certificate(f::S, 1))],
        Some(f::S),
    );
    outcomes(&after, StatsScope::All, 1, 0, 1, 0);
}

#[test]
fn stats3_later_create_after_reseat_matches_no_birth_and_cannot_fill_the_old_gap() {
    let bytes = certificate(f::S, 1)
        + &fact(f::S, 2, "start-attempted", 10)
        + &fact(f::S, 2, "start-failed", 10)
        + &fact(f::S, 2, "brief-failed", 0);
    let result = view(vec![("S", f::S, bytes)], Some(f::S));
    outcomes(&result, StatsScope::All, 1, 0, 0, 1);
    let text = rendered(&result, StatsScope::All, 240).to_lowercase();
    for word in ["observed", "unknown", "backfill"] {
        assert!(
            text.contains(word),
            "explain pre-slice observed gaps ({word}): {text}"
        );
    }
    assert!(
        !result.all.partial(),
        "ordinary reseat orphans do not create a coverage gap"
    );
}

#[test]
fn stats3_a_retry_is_unproved_instead_of_proving_a_successful_execution() {
    let base = certificate(f::S, 1)
        + &fact(f::S, 1, "start-attempted", 10)
        + &fact(f::S, 1, "start-failed", 10);
    outcomes(
        &view(vec![("S", f::S, base.clone())], None),
        StatsScope::All,
        1,
        0,
        1,
        0,
    );
    let retry = base + &fact(f::S, 1, "start-attempted", 11);
    outcomes(
        &view(vec![("S", f::S, retry)], None),
        StatsScope::All,
        1,
        0,
        0,
        1,
    );
}

#[test]
fn stats3_overlap_is_idempotent_and_divergent_fact_sequences_are_unknown() {
    let base = certificate(f::S, 1) + &fact(f::S, 1, "start-attempted", 10);
    let longer =
        base.clone() + &fact(f::S, 1, "start-failed", 10) + &fact(f::S, 1, "brief-failed", 0);
    for reverse in [false, true] {
        let mut sources = vec![
            ("live", f::S, base.clone()),
            ("archive", f::S, longer.clone()),
            ("repeat", f::S, longer.clone()),
        ];
        if reverse {
            sources.reverse();
        }
        outcomes(&view(sources, None), StatsScope::All, 1, 1, 1, 0);
    }
    let divergent = base + &fact(f::S, 1, "start-attempted", 11);
    let result = view(vec![("one", f::S, longer), ("two", f::S, divergent)], None);
    outcomes(&result, StatsScope::All, 1, 0, 0, 1);
    assert!(result.all.partial(), "conflict is an explicit coverage gap");
}

#[test]
fn stats3_event_loss_is_partial_and_unknown_instead_of_a_successful_execution() {
    let bytes =
        f::link(1) + &fact(f::S, 1, "start-attempted", 10) + &fact(f::S, 1, "start-failed", 10);
    let result = view(vec![("S", f::S, bytes)], None);
    outcomes(&result, StatsScope::All, 1, 0, 0, 1);
    assert!(result.all.partial());
    assert!(rendered(&result, StatsScope::All, 240).contains("certificate lost"));
}

#[test]
fn stats3_outcomes_survive_evidence_and_materialized_view_reload() {
    let bytes = fact(f::S, 1, "start-attempted", 10)
        + &certificate(f::S, 1)
        + &fact(f::S, 1, "brief-failed", 0)
        + &fact(f::S, 1, "start-failed", 10);
    let mut store = Store::default();
    store
        .live_source("S", "S", format!("session_id={}\n", f::S).as_bytes())
        .expect("source");
    store
        .chunk("S", 0, bytes.as_bytes(), true)
        .expect("journal read");
    outcomes(&store.view(Some(f::S)), StatsScope::All, 1, 1, 1, 0);
    let encoded = store.render().expect("evidence persists");
    assert!(!encoded.contains(f::PRIVATE));
    let store = Store::parse(&encoded).expect("evidence reloads");
    let view = store.view(Some(f::S));
    outcomes(&view, StatsScope::All, 1, 1, 1, 0);
    outcomes(&view, StatsScope::ThisSession, 1, 1, 1, 0);
    let view = stats_store::parse_view(&stats_store::render_view(&view)).expect("view reloads");
    outcomes(&view, StatsScope::All, 1, 1, 1, 0);
    outcomes(&view, StatsScope::ThisSession, 1, 1, 1, 0);
}
