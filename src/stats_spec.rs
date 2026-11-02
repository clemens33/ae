//! #287 slice2: byte fixtures, hand totals, configured evidence only.
//! Oracle: issue #287, plan v2 Slice2 and the lead flag-state ruling.

use crate::stats::{self, Cell, Reason, Row, Scoped, Source, SourceGap, ThisSession, View};
use crate::stats_fixture as f;
use std::collections::BTreeMap;

fn source(label: &str, session: &str, bytes: &[u8]) -> Source {
    Source::live(
        label,
        format!("session_id={session}\n").as_bytes(),
        Ok(bytes.to_vec()),
    )
}

fn one(session: &str, bytes: &[u8]) -> View {
    stats::fold(vec![source("fixture", session, bytes)], Some(session))
}

fn current(view: &View) -> &Scoped {
    let ThisSession::Ready(current) = &view.this else {
        panic!("current UUID resolved")
    };
    current
}

fn expected_cell(word: &str) -> Cell {
    match word {
        "unknown" => Cell::Unknown,
        "none set" => Cell::NoneSet,
        "direct" => Cell::Direct,
        "unproved" => Cell::Unproved,
        value => Cell::Value(value.to_owned()),
    }
}

fn rows(actual: &[Row], expected: Vec<(Vec<&str>, u64)>) {
    let got: BTreeMap<Vec<Cell>, u64> = actual
        .iter()
        .map(|row| (row.cells.clone(), row.count))
        .collect();
    let want: BTreeMap<Vec<Cell>, u64> = expected
        .into_iter()
        .map(|(cells, count)| (cells.into_iter().map(expected_cell).collect(), count))
        .collect();
    assert_eq!(actual.len(), got.len(), "one row per tuple");
    assert_eq!(got, want);
    assert!(
        actual.windows(2).all(|pair| pair[0].count >= pair[1].count),
        "count descending"
    );
}

fn assert_harness(view: &Scoped, expected: &[(&str, u64)]) {
    rows(
        &view.harness,
        expected.iter().map(|(a, n)| (vec![*a], *n)).collect(),
    );
}
fn assert_clients(view: &Scoped, expected: &[(&str, u64)]) {
    rows(
        &view.client,
        expected.iter().map(|(a, n)| (vec![*a], *n)).collect(),
    );
}
fn assert_model_effort(view: &Scoped, expected: &[(&str, &str, u64)]) {
    rows(
        &view.model_effort,
        expected
            .iter()
            .map(|(a, b, n)| (vec![*a, *b], *n))
            .collect(),
    );
}
fn assert_joint(view: &Scoped, expected: &[(&str, &str, &str, &str, u64)]) {
    rows(
        &view.joint,
        expected
            .iter()
            .map(|(harness, client, model, effort, count)| {
                (vec![*harness, *client, *model, *effort], *count)
            })
            .collect(),
    );
}
fn assert_sums(view: &Scoped) {
    for table in [&view.harness, &view.client, &view.model_effort, &view.joint] {
        assert_eq!(table.iter().map(|row| row.count).sum::<u64>(), view.total);
    }
    assert!(view.balanced());
}
fn assert_gap(view: &Scoped, word: &str) {
    assert!(
        view.gaps
            .iter()
            .any(|gap| gap.line().to_lowercase().contains(word)),
        "missing {word}: {:?}",
        view.gaps
    );
}

#[test]
fn identity_matrix_keeps_routes_aliases_and_three_flag_states_separate() {
    let (s, t) = f::matrix();
    let view = stats::fold(
        vec![source("live/S", f::S, &s), source("archive/T", f::T, &t)],
        Some(f::S),
    );
    let all = &view.all;
    assert_eq!(all.total, 12);
    assert_eq!((all.model_known, all.effort_known), (7, 7));
    assert_harness(
        all,
        &[("claude", 5), ("codex", 3), ("muse", 1), ("unknown", 3)],
    );
    assert_clients(
        all,
        &[
            ("claude-p", 4),
            ("claude-mic", 1),
            ("codex", 2),
            ("direct", 2),
            ("unknown", 3),
        ],
    );
    assert_model_effort(
        all,
        &[
            ("sonnet", "high", 2),
            ("opus", "max", 1),
            ("sol", "high", 1),
            ("unproved", "unproved", 1),
            ("unproved", "high", 1),
            ("sol", "unproved", 1),
            ("claude-sonnet-5-5", "high", 1),
            ("none set", "none set", 1),
            ("unknown", "unknown", 3),
        ],
    );
    assert_joint(
        all,
        &[
            ("claude", "claude-p", "sonnet", "high", 1),
            ("claude", "claude-mic", "sonnet", "high", 1),
            ("claude", "claude-p", "opus", "max", 1),
            ("codex", "codex", "sol", "high", 1),
            ("muse", "direct", "unproved", "unproved", 1),
            ("claude", "claude-p", "unproved", "high", 1),
            ("codex", "codex", "sol", "unproved", 1),
            ("claude", "claude-p", "claude-sonnet-5-5", "high", 1),
            ("codex", "direct", "none set", "none set", 1),
            ("unknown", "unknown", "unknown", "unknown", 3),
        ],
    );
    let ThisSession::Ready(current) = &view.this else {
        panic!("S UUID resolved")
    };
    assert_eq!(current.total, 10);
    assert_eq!((current.model_known, current.effort_known), (5, 5));
    assert_sums(all);
    assert_sums(current);
    let debug = format!("{view:?}");
    assert!(
        !debug.contains(f::PRIVATE),
        "prompt canary must not reach projection"
    );
    assert_eq!(all.legacy, 2, "normal legacy is a labelled lower bound");
    assert_gap(all, "certificate lost");
    assert_gap(all, "ambiguous-model");
    assert_gap(all, "unknown-effort");
    assert_gap(all, "unsupported");
}

#[test]
fn restart_noise_never_counts_and_retire_then_new_same_name_does() {
    let fields = f::fields("claude", Some("claude-p"), Some("sonnet"), Some("high"));
    let first = f::birth(f::S, 1, &fields);
    let mut bytes = first.clone();
    bytes.push_str(&f::link(1));
    for action in [
        "resume",
        "relaunch",
        "reseat",
        "auto-reseat",
        "effort-applied",
        "effort-restored",
        "auto-reseat-done",
        "conversation-new",
        "conversation-resumed",
        "spawn-failed",
        "retire",
    ] {
        bytes.push_str(&f::event(
            action,
            &format!(",\"summary\":\"{}\"", f::PRIVATE),
        ));
    }
    assert_eq!(one(f::S, bytes.as_bytes()).all.total, 1);
    bytes.push_str(&f::birth(f::S, 2, &fields));
    bytes.push_str(&f::link(2));
    let view = one(f::S, bytes.as_bytes());
    assert_eq!(
        view.all.total, 2,
        "fresh birth, same display name and second"
    );
    assert_harness(&view.all, &[("claude", 2)]);
    assert_sums(&view.all);
}

#[test]
fn modern_overlap_rename_replay_and_session_uuid_are_independent() {
    let cells = f::fields("codex", None, Some("sol"), Some("high"));
    let s = format!("{}{}", f::birth(f::S, 1, &cells), f::link(1));
    let renamed = s.replace("\"target\":\"scout\"", "\"target\":\"renamed\"");
    let t = format!("{}{}", f::birth(f::T, 1, &cells), f::link(1));
    let sources = || {
        vec![
            source("live/new-name", f::S, s.as_bytes()),
            source("archive/old-name", f::S, renamed.as_bytes()),
            source("T", f::T, t.as_bytes()),
            source("replay", f::S, s.as_bytes()),
        ]
    };
    let view = stats::fold(sources(), Some(f::S));
    assert_eq!(
        view.all.total, 2,
        "same birth UUID in distinct session UUIDs is distinct"
    );
    assert_eq!(current(&view).total, 1);
    assert_model_effort(&view.all, &[("sol", "high", 2)]);
    let reversed = stats::fold(sources().into_iter().rev().collect(), Some(f::S));
    assert_eq!(view, reversed, "enumeration order is not evidence");
}

#[test]
fn repeated_birth_gap_labels_are_order_independent() {
    let first = f::birth(
        f::S,
        1,
        &f::fields("codex", None, Some("sol"), Some("high")),
    );
    let second = f::birth(
        f::S,
        1,
        &f::fields("codex", None, Some("other"), Some("high")),
    );
    for (reason, journals) in [
        (
            Reason::CertificateLost,
            [f::link(1), f::link(1), f::link(1)],
        ),
        (Reason::Conflict, [first.clone(), second, first]),
    ] {
        let sources = || {
            ["zeta", "middle", "alpha"]
                .into_iter()
                .zip(&journals)
                .map(|(label, bytes)| source(label, f::S, bytes.as_bytes()))
                .collect::<Vec<_>>()
        };
        let forward = stats::fold(sources(), Some(f::S));
        let reverse = stats::fold(sources().into_iter().rev().collect(), Some(f::S));
        assert_eq!(forward, reverse, "gap labels are evidence too: {reason:?}");
        assert_eq!(forward.all.total, 1);
        assert_eq!(forward.all.gaps.len(), 1);
        let gap = &forward.all.gaps[0];
        assert_eq!(gap.reason, reason);
        assert_eq!(gap.count, 1);
        assert!(
            gap.source
                .as_deref()
                .is_some_and(|label| ["alpha", "middle", "zeta"].contains(&label)),
            "canonical gap label must name a source that actually supplied it"
        );
        assert_eq!(current(&forward).gaps, forward.all.gaps);
    }
}

#[test]
fn balanced_rejects_each_wrong_table_and_each_part_above_its_whole() {
    let row = |cells| vec![Row { cells, count: 2 }];
    let valid = Scoped {
        total: 2,
        legacy: 1,
        model_known: 1,
        effort_known: 1,
        range: None,
        sources_read: 1,
        sources_total: 2,
        harness: row(vec![Cell::Unknown]),
        client: row(vec![Cell::Unknown]),
        model_effort: row(vec![Cell::Unknown; 2]),
        joint: row(vec![Cell::Unknown; 4]),
        gaps: Vec::new(),
    };
    assert!(valid.balanced(), "hand-built tables each sum to two");
    for index in 0..4 {
        for count in [0, 1, 3] {
            let mut wrong = valid.clone();
            let table = match index {
                0 => &mut wrong.harness,
                1 => &mut wrong.client,
                2 => &mut wrong.model_effort,
                _ => &mut wrong.joint,
            };
            table[0].count = count;
            assert!(
                !wrong.balanced(),
                "table {index} sums to {count}, total two"
            );
        }
        let mut wrong = valid.clone();
        let part = match index {
            0 => &mut wrong.legacy,
            1 => &mut wrong.model_known,
            2 => &mut wrong.effort_known,
            _ => &mut wrong.sources_read,
        };
        *part = 3;
        assert!(!wrong.balanced(), "part {index} is three, whole two");
    }
    let mut boundary = valid;
    boundary.legacy = 2;
    boundary.model_known = 2;
    boundary.effort_known = 2;
    boundary.sources_read = 2;
    assert!(boundary.balanced(), "equality is valid at every part bound");
}

#[test]
fn legacy_multiplicity_subsequence_and_whole_record_divergence_are_honest() {
    let a = f::legacy("prompt-A-private");
    let b = f::legacy("prompt-B-private");
    let c = f::legacy("prompt-C-private");
    let full = format!("{a}{a}{b}");
    let trim = format!("{a}{b}");
    let view = stats::fold(
        vec![
            source("archive/S", f::S, full.as_bytes()),
            source("live/S", f::S, trim.as_bytes()),
        ],
        Some(f::S),
    );
    assert_eq!(
        view.all.total, 3,
        "subsequence retains multiplicity, not a set"
    );
    assert_harness(&view.all, &[("unknown", 3)]);
    let reversed = stats::fold(
        vec![
            source("live/S", f::S, trim.as_bytes()),
            source("archive/S", f::S, full.as_bytes()),
        ],
        Some(f::S),
    );
    assert_eq!(view, reversed, "legacy subsequence works in both orders");
    let opposed = format!("{a}{c}");
    let excluded = stats::fold(
        vec![
            source("archive/S", f::S, full.as_bytes()),
            source("live/S", f::S, opposed.as_bytes()),
        ],
        Some(f::S),
    );
    assert_eq!(
        excluded.all.total, 0,
        "neither divergent legacy list is safely attributable"
    );
    assert_gap(&excluded.all, "legacy excluded");
    assert!(excluded.all.gaps.iter().any(|gap| gap.reason
        == Reason::LegacyExcluded {
            sessions: 1,
            a: 2,
            b: 3
        }));
    let reversed = stats::fold(
        vec![
            source("live/S", f::S, opposed.as_bytes()),
            source("archive/S", f::S, full.as_bytes()),
        ],
        Some(f::S),
    );
    assert_eq!(excluded, reversed, "legacy divergence works in both orders");
    let debug = format!("{excluded:?}");
    for private in ["prompt-A-private", "prompt-B-private", "prompt-C-private"] {
        assert!(
            !debug.contains(private),
            "legacy comparison must not retain prompt text"
        );
    }
}

#[test]
fn missing_certificate_is_one_unknown_and_orphan_facts_are_inert() {
    let linked = format!("{}{}", f::link(1), f::link(1));
    let orphan = f::fact(
        f::S,
        2,
        "observed",
        &format!(
            ",\"start\":\"spawned.0:{}:41\",\"observed_model\":\"opus\",\"observed_effort\":\"max\"",
            f::id(2)
        ),
    );
    let view = one(f::S, format!("{linked}{orphan}").as_bytes());
    assert_eq!(view.all.total, 1);
    assert_harness(&view.all, &[("unknown", 1)]);
    assert_model_effort(&view.all, &[("unknown", "unknown", 1)]);
    assert_gap(&view.all, "certificate lost");
    assert_eq!(one(f::S, orphan.as_bytes()).all.total, 0);
    assert!(
        one(f::S, orphan.as_bytes()).all.gaps.is_empty(),
        "orphan reseat facts are routine, not gaps"
    );
}

#[test]
fn certificate_conflicts_poison_only_differing_cells_not_the_whole_tuple() {
    let a = f::birth(
        f::S,
        1,
        &f::fields("claude", Some("claude-p"), Some("sonnet"), Some("high")),
    );
    let b = a.replace("\"birth_model\":\"sonnet\"", "\"birth_model\":\"opus\"");
    let view = stats::fold(
        vec![
            source("A", f::S, a.as_bytes()),
            source("B", f::S, b.as_bytes()),
        ],
        Some(f::S),
    );
    assert_eq!(view.all.total, 1);
    assert_harness(&view.all, &[("claude", 1)]);
    assert_clients(&view.all, &[("claude-p", 1)]);
    assert_model_effort(&view.all, &[("unknown", "high", 1)]);
    assert_eq!((view.all.model_known, view.all.effort_known), (0, 1));
    assert_gap(&view.all, "conflict");
    assert_sums(&view.all);
    let only_profile = a.replace(
        "\"birth_profile\":\"preset\"",
        "\"birth_profile\":\"renamed-profile\"",
    );
    let profile = stats::fold(
        vec![
            source("A", f::S, a.as_bytes()),
            source("B", f::S, only_profile.as_bytes()),
        ],
        Some(f::S),
    );
    assert_eq!(profile.all.total, 1);
    assert_model_effort(&profile.all, &[("sonnet", "high", 1)]);
    assert_eq!((profile.all.model_known, profile.all.effort_known), (1, 1));
}

#[test]
fn bad_and_newer_certificates_report_gaps_without_inventing_defaults() {
    let original = f::birth(
        f::S,
        1,
        &f::fields("claude", Some("claude-p"), Some("sonnet"), Some("high")),
    );
    let damaged = original.replace("\"birth_effort\":\"high\"", "\"birth_effort\":7");
    let unsupported = f::birth(f::S, 2, &f::fields("codex", None, Some("sol"), None))
        .replace("\"birth_v\":1", "\"birth_v\":2");
    let invalid_id = original.replace(&f::id(1), "not-an-id");
    let bytes = format!("{damaged}{unsupported}{invalid_id}");
    let view = one(f::S, bytes.as_bytes());
    assert_eq!(view.all.total, 2);
    assert_harness(&view.all, &[("unknown", 2)]);
    assert_clients(&view.all, &[("unknown", 2)]);
    assert_model_effort(&view.all, &[("unknown", "unknown", 2)]);
    assert_eq!((view.all.model_known, view.all.effort_known), (0, 0));
    assert_gap(&view.all, "damaged");
    assert_gap(&view.all, "unsupported");
    assert_sums(&view.all);
}

#[test]
fn facts_before_parent_extensions_divergence_and_late_values_never_rewrite_configured() {
    let cert = f::birth(
        f::S,
        1,
        &f::fields("claude", Some("claude-p"), Some("sonnet"), None),
    );
    let start = format!(",\"start\":\"spawned.0:{}:41\"", f::id(1));
    let attempt = f::fact(f::S, 1, "start-attempted", &start);
    let observed = f::fact(
        f::S,
        1,
        "observed",
        &format!(
            "{start},\"observed_source\":\"frame\",\"observed_model\":\"opus\",\"observed_effort\":\"max\""
        ),
    );
    let first = format!("{attempt}{cert}");
    let extended = format!("{attempt}{observed}{cert}");
    let joined = stats::fold(
        vec![
            source("short", f::S, first.as_bytes()),
            source("long", f::S, extended.as_bytes()),
        ],
        Some(f::S),
    );
    assert_eq!(joined.all.total, 1);
    assert_model_effort(&joined.all, &[("sonnet", "none set", 1)]);
    assert_eq!((joined.all.model_known, joined.all.effort_known), (1, 1));
    let after = format!("{cert}{attempt}{observed}");
    let ordered = one(f::S, after.as_bytes());
    assert_model_effort(&ordered.all, &[("sonnet", "none set", 1)]);
    let divergent = format!(
        "{cert}{attempt}{}",
        f::fact(f::S, 1, "start-failed", &start)
    );
    let conflict = stats::fold(
        vec![
            source("A", f::S, extended.as_bytes()),
            source("B", f::S, divergent.as_bytes()),
        ],
        Some(f::S),
    );
    assert_eq!(conflict.all.total, 1);
    assert_model_effort(&conflict.all, &[("sonnet", "none set", 1)]);
    assert_gap(&conflict.all, "conflict");
}

#[test]
fn malformed_journal_lines_are_a_visible_partial_read_and_empty_is_supported_zero() {
    let empty = one(f::S, b"");
    assert_eq!(empty.all.total, 0);
    assert!(empty.all.gaps.is_empty());
    let cert = f::birth(
        f::S,
        1,
        &f::fields("codex", None, Some("sol"), Some("high")),
    );
    let bytes = format!("{cert}{{partial private-canary\n");
    let view = one(f::S, bytes.as_bytes());
    assert_eq!(view.all.total, 1);
    assert_gap(&view.all, "malformed");
    assert!(!format!("{view:?}").contains("private-canary"));
    let non_utf8 = one(f::S, &[0xff, b'\n']);
    assert_eq!(non_utf8.all.total, 0);
    assert!(
        !non_utf8.all.gaps.is_empty(),
        "invalid encoding is a gap, never healthy empty"
    );
}

#[test]
fn missing_current_uuid_does_not_turn_this_session_into_all_retained() {
    let bytes = f::birth(f::S, 1, &f::fields("codex", None, None, None));
    let view = stats::fold(vec![source("S", f::S, bytes.as_bytes())], None);
    assert_eq!(view.all.total, 1);
    assert!(matches!(view.this, ThisSession::NoUuid));
    assert_model_effort(&view.all, &[("none set", "none set", 1)]);
}

#[test]
fn readable_source_and_certificate_without_session_uuid_count_once_without_current_scope() {
    let cert = f::birth(
        f::S,
        1,
        &f::fields("codex", None, Some("sol"), Some("high")),
    )
    .replace(&format!(",\"target_session_id\":\"{}\"", f::S), "");
    let bytes = format!("{cert}{}{}", f::link(1), f::link(1));
    let view = stats::fold(
        vec![Source::live(
            "no-uuid",
            b"seat.main=lead\n",
            Ok(bytes.into_bytes()),
        )],
        None,
    );
    assert_eq!(view.all.total, 1, "own-source links resolve without a UUID");
    assert_eq!((view.all.sources_read, view.all.sources_total), (1, 1));
    assert!(matches!(view.this, ThisSession::NoUuid));
    assert!(
        view.all
            .gaps
            .iter()
            .any(|gap| gap.reason == Reason::NoSessionId)
    );
    assert_harness(&view.all, &[("codex", 1)]);
    assert_model_effort(&view.all, &[("sol", "high", 1)]);
    assert_sums(&view.all);
}

#[test]
fn each_source_failure_names_its_source_and_never_reads_as_healthy_zero() {
    for gap in [
        SourceGap::Missing,
        SourceGap::Unreadable,
        SourceGap::Oversized,
        SourceGap::Budget,
        SourceGap::Invalid,
        SourceGap::Claimed,
    ] {
        let view = stats::fold(
            vec![Source::failed("failed-source", Some(f::S), gap)],
            Some(f::S),
        );
        assert_eq!(view.all.total, 0);
        assert!(
            view.all
                .gaps
                .iter()
                .any(|found| found.source.as_deref() == Some("failed-source")
                    && found.reason == Reason::Source(gap))
        );
        assert_eq!(view.all.sources_read, 0);
        assert_eq!(view.all.sources_total, 1);
        assert!(!current(&view).gaps.is_empty());
    }
    let supported = source("empty-source", f::S, b"");
    let zero = stats::fold(vec![supported], Some(f::S));
    assert_eq!(
        (
            zero.all.total,
            zero.all.sources_read,
            zero.all.sources_total
        ),
        (0, 1, 1)
    );
    assert!(zero.all.gaps.is_empty());
}

#[test]
fn partial_range_roster_notes_and_unreadable_meta_never_supply_counts() {
    let bytes = format!(
        "{}{}",
        f::birth(f::S, 1, &f::fields("codex", None, None, None)),
        f::event("said", ",\"summary\":\"private-date-range-canary\"")
            .replace("2026-10-01", "2026-10-30")
    );
    let meta = format!(
        "session_id={}\nseat.main=lead\nseat.worker.0=fixed\nseat.spawned.7=unproved\n",
        f::S
    );
    let view = stats::fold(
        vec![
            Source::live("live-S", meta.as_bytes(), Ok(bytes.into_bytes())),
            Source::failed("missing-T", Some(f::T), SourceGap::Missing),
        ],
        Some(f::S),
    );
    assert_eq!(view.all.total, 1);
    assert_eq!(
        view.all.range,
        Some(("2026-10-01".to_owned(), "2026-10-30".to_owned()))
    );
    assert_eq!((view.all.sources_read, view.all.sources_total), (1, 2));
    assert_gap(&view.all, "missing-t");
    assert!(
        view.all
            .gaps
            .iter()
            .any(|gap| gap.reason == Reason::LiveNoBirth && gap.count == 1)
    );
    assert!(!format!("{view:?}").contains("private-date-range-canary"));
    let unreadable = stats::fold(
        vec![Source::live(
            "bad-meta",
            &[0xff],
            Ok(f::link(1).into_bytes()),
        )],
        None,
    );
    assert_eq!(
        unreadable.all.total, 0,
        "unreadable metadata does not authorize journal attribution"
    );
    assert!(!unreadable.all.gaps.is_empty());
}

#[test]
fn source_session_mismatch_cannot_borrow_trust_or_current_scope() {
    let cert = f::birth(
        f::T,
        1,
        &f::fields("claude", Some("claude-p"), Some("sonnet"), Some("high")),
    );
    let view = stats::fold(vec![source("claims-S", f::S, cert.as_bytes())], Some(f::S));
    assert_eq!(view.all.total, 1);
    assert_eq!(
        current(&view).total,
        0,
        "source S must not acquire a certificate claiming T"
    );
    assert_harness(&view.all, &[("unknown", 1)]);
    assert_model_effort(&view.all, &[("unknown", "unknown", 1)]);
    assert!(
        view.all
            .gaps
            .iter()
            .any(|gap| gap.reason == Reason::SessionMismatch)
    );
    let in_t = stats::fold(vec![source("claims-S", f::S, cert.as_bytes())], Some(f::T));
    assert_eq!(
        current(&in_t).total,
        1,
        "salvaged explicit identity is T, still unknown"
    );
}

#[test]
fn damaged_and_unsupported_facts_are_inert_but_visible_on_a_known_birth() {
    let cert = f::birth(
        f::S,
        1,
        &f::fields("codex", None, Some("sol"), Some("high")),
    );
    let fact = f::fact(f::S, 1, "brief-failed", "");
    let bad = fact.replace("\"fact\":\"brief-failed\"", "\"fact\":7");
    let newer = fact.replace("\"birth_v\":1", "\"birth_v\":2");
    let view = one(f::S, format!("{cert}{bad}{newer}").as_bytes());
    assert_eq!(view.all.total, 1);
    assert_model_effort(&view.all, &[("sol", "high", 1)]);
    assert_gap(&view.all, "damaged");
    assert_gap(&view.all, "unsupported");
}

#[test]
fn current_metadata_and_profile_edits_never_backfill_history() {
    let cert = f::birth(
        f::S,
        1,
        &f::fields("claude", Some("claude-p"), Some("sonnet"), Some("high")),
    );
    let bytes = format!("{cert}{}", f::legacy(f::PRIVATE)).into_bytes();
    let meta = format!(
        "session_id={}\nseat.spawned.0=scout\nprofile.spawned.0={}\nclient.spawned.0=claude-mic\nmodel.spawned.0=opus\nobserved_model.spawned.0=opus\nobserved_effort.spawned.0=max\nagent_cmd.spawned.0={}\n",
        f::S,
        f::PRIVATE,
        f::PRIVATE
    );
    let view = stats::fold(
        vec![Source::live("S", meta.as_bytes(), Ok(bytes))],
        Some(f::S),
    );
    assert_eq!(view.all.total, 2);
    assert_clients(&view.all, &[("claude-p", 1), ("unknown", 1)]);
    assert_model_effort(
        &view.all,
        &[("sonnet", "high", 1), ("unknown", "unknown", 1)],
    );
    assert!(!format!("{view:?}").contains(f::PRIVATE));
}

#[test]
fn replaying_the_entire_matrix_preserves_legacy_multiplicity_without_multiplication() {
    let (s, t) = f::matrix();
    let view = stats::fold(
        vec![
            source("live-S", f::S, &s),
            source("replay-S", f::S, &s),
            Source::archive(f::T, Ok(t.clone())),
            Source::archive(f::T, Ok(t)),
        ],
        Some(f::S),
    );
    assert_eq!((view.all.total, view.all.legacy), (12, 2));
    assert_eq!((current(&view).total, current(&view).legacy), (10, 2));
    assert_eq!((view.all.model_known, view.all.effort_known), (7, 7));
    assert_sums(&view.all);
}

#[test]
fn model_and_effort_numerators_follow_their_own_flag_state() {
    for (fields, model, effort, cells) in [
        (
            f::fields("codex", None, None, None),
            1,
            1,
            ("none set", "none set"),
        ),
        (
            format!(
                "{},\"birth_model_unproved\":\"ambiguous-model\"",
                f::fields("codex", None, None, Some("high"))
            ),
            0,
            1,
            ("unproved", "high"),
        ),
        (
            format!(
                "{},\"birth_effort_unproved\":\"unknown-effort\"",
                f::fields("codex", None, Some("sol"), None)
            ),
            1,
            0,
            ("sol", "unproved"),
        ),
        (
            format!(
                "{},\"birth_model_unproved\":\"ambiguous-model\",\"birth_effort_unproved\":\"unknown-effort\"",
                f::fields("codex", None, None, None)
            ),
            0,
            0,
            ("unproved", "unproved"),
        ),
    ] {
        let view = one(f::S, f::birth(f::S, 1, &fields).as_bytes());
        assert_eq!(view.all.total, 1);
        assert_eq!(
            (view.all.model_known, view.all.effort_known),
            (model, effort)
        );
        assert_model_effort(&view.all, &[(cells.0, cells.1, 1)]);
        assert_sums(&view.all);
    }
}

fn live_notes(view: &View, source: &str) -> u64 {
    view.all
        .gaps
        .iter()
        .filter(|gap| gap.reason == Reason::LiveNoBirth && gap.source.as_deref() == Some(source))
        .map(|gap| gap.count)
        .sum()
}

#[test]
fn routine_addressed_events_and_orphan_facts_do_not_prove_a_live_birth() {
    let meta = format!("session_id={}\nseat.spawned.0=scout\n", f::S);
    for (label, bytes) in [
        ("empty", String::new()),
        (
            "said",
            f::event(
                "said",
                ",\"target_slot\":\"spawned.0\",\"summary\":\"hello\"",
            ),
        ),
        (
            "resume",
            f::event("resume", ",\"target_slot\":\"spawned.0\""),
        ),
        (
            "retire",
            f::event("retire", ",\"target_slot\":\"spawned.0\""),
        ),
        (
            "orphan fact",
            f::fact(f::S, 1, "brief-failed", ",\"target_slot\":\"spawned.0\""),
        ),
    ] {
        let view = stats::fold(
            vec![Source::live("own", meta.as_bytes(), Ok(bytes.into_bytes()))],
            Some(f::S),
        );
        assert_eq!(view.all.total, 0, "{label}");
        assert_eq!(live_notes(&view, "own"), 1, "{label} proves no birth");
        assert!(!view.all.partial(), "a live-seat note costs no coverage");
        assert_sums(&view.all);
    }
}

#[test]
fn only_own_birth_or_spawn_with_matching_name_and_optional_slot_proves_a_seat() {
    let meta = format!("session_id={}\nseat.spawned.0=scout\n", f::S);
    let cert = f::birth(
        f::S,
        1,
        &f::fields("codex", None, Some("sol"), Some("high")),
    );
    let damaged = cert.replace("\"birth_effort\":\"high\"", "\"birth_effort\":7");
    let newer = cert.replace("\"birth_v\":1", "\"birth_v\":2");
    for (shape, record) in [
        ("certificate", cert),
        ("damaged certificate", damaged),
        ("newer certificate", newer),
        ("linked spawn", f::link(1)),
        ("legacy spawn", f::legacy(f::PRIVATE)),
    ] {
        let no_slot = record.replace(",\"target_slot\":\"spawned.0\"", "");
        let same_slot = no_slot.replace(
            "\"target\":\"scout\"",
            "\"target\":\"scout\",\"target_slot\":\"spawned.0\"",
        );
        let wrong_slot = same_slot.replace(
            "\"target_slot\":\"spawned.0\"",
            "\"target_slot\":\"spawned.7\"",
        );
        let old_name = same_slot.replace("\"target\":\"scout\"", "\"target\":\"retired-worker\"");
        let no_name = same_slot.replace(",\"target\":\"scout\"", "");
        for (identity, bytes, notes) in [
            ("name and slot", same_slot, 0),
            ("name without slot", no_slot, 0),
            ("same name, wrong slot", wrong_slot, 1),
            ("reused slot, old name", old_name, 1),
            ("slot without name", no_name, 1),
        ] {
            let view = stats::fold(
                vec![Source::live("own", meta.as_bytes(), Ok(bytes.into_bytes()))],
                Some(f::S),
            );
            assert_eq!(view.all.total, 1, "{shape}, {identity}");
            assert_eq!(live_notes(&view, "own"), notes, "{shape}, {identity}");
            assert_sums(&view.all);
            assert!(!format!("{view:?}").contains(f::PRIVATE));
        }
    }
}

#[test]
fn another_live_source_or_archive_cannot_prove_the_current_live_seat() {
    let meta = format!("session_id={}\nseat.spawned.0=scout\n", f::S);
    let cert = f::birth(
        f::S,
        1,
        &f::fields("codex", None, Some("sol"), Some("high")),
    );
    for archive in [false, true] {
        let sources = || {
            let other = if archive {
                Source::archive(f::S, Ok(cert.as_bytes().to_vec()))
            } else {
                source("other-live", f::S, cert.as_bytes())
            };
            vec![Source::live("own", meta.as_bytes(), Ok(Vec::new())), other]
        };
        let view = stats::fold(sources(), Some(f::S));
        assert_eq!(view.all.total, 1, "historical count is still retained");
        assert_eq!(
            live_notes(&view, "own"),
            1,
            "own journal has no birth evidence"
        );
        assert!(!view.all.partial());
        let reversed = stats::fold(sources().into_iter().rev().collect(), Some(f::S));
        assert_eq!(
            view, reversed,
            "source-local proof is enumeration independent"
        );
    }
}
