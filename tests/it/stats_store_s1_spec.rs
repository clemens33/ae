//! Frozen S1 public-contract tests. Author: statsstore-nav (`OpenAI`).
//! Oracle: accepted human requirement + unchanged `stats::fold` on consumed bytes.
#![allow(
    dead_code,
    reason = "the fuzz bin shares this harness; test-only helpers vanish there"
)]
#![allow(
    clippy::expect_used,
    reason = "frozen test fixtures and asserted successful operations"
)]

#[path = "stats_store_s1_cases.rs"]
mod cases;

#[cfg(test)]
use ae::stats::{self, Reason, SourceGap, ThisSession};
use ae::stats::{Fold, Reading, Source, View};
use ae::stats_store::{Store, parse_view, render_view};
use cases::{Feed, Kind, PRIVATE, S, SCOPES, T};

struct Harness {
    store: Store,
    feeds: Vec<Feed>,
    line_limit: Option<usize>,
    privacy_canary: bool,
    steps: usize,
}

impl Harness {
    fn new(feeds: Vec<Feed>) -> Self {
        Self {
            store: Store::default(),
            feeds,
            line_limit: None,
            privacy_canary: true,
            steps: 0,
        }
    }

    fn check(&mut self) {
        self.steps += 1;
        for scope in SCOPES {
            let expected = self.line_limit.map_or_else(
                || cases::oracle(&self.feeds, scope),
                |cap| limited_oracle(&self.feeds, scope, cap),
            );
            let actual = self.store.view(scope);
            assert_eq!(actual, expected, "operation {} scope {scope:?}", self.steps);
            cases::assert_balanced(&actual);
            if self.privacy_canary {
                assert!(
                    !format!("{actual:?}").contains(PRIVATE),
                    "no prompt in view"
                );
            }
        }
    }

    fn register(&mut self, index: usize) {
        let feed = &mut self.feeds[index];
        match &feed.kind {
            Kind::Live(meta) => self.store.live_source(&feed.id, &feed.label, meta),
            Kind::Archive(session) => self.store.archive_source(&feed.id, session),
            Kind::Failed(session, gap) => {
                self.store
                    .failed_source(&feed.id, &feed.label, session.as_deref(), *gap)
            }
        }
        .expect("register or replace source");
        feed.reset();
        self.check();
    }

    fn insert(&mut self, source: usize, record: usize) {
        let feed = &mut self.feeds[source];
        let (start, end, allowed) = feed.records[record];
        assert!(allowed);
        self.store
            .event(
                &feed.id,
                u64::try_from(start).expect("offset"),
                &feed.bytes[start..end],
            )
            .expect("certificate or linked spawn at its journal offset");
        feed.inserted.insert(record);
        self.check();
    }

    fn chunk(&mut self, index: usize, end: usize, eof: bool) {
        let feed = &mut self.feeds[index];
        let start = feed.cursor;
        assert!(start <= end && end <= feed.bytes.len());
        let offered = &feed.bytes[start..end];
        let expected = if eof {
            end
        } else {
            start
                + offered
                    .iter()
                    .rposition(|byte| *byte == b'\n')
                    .map_or(0, |pos| pos + 1)
        };
        let cursor = self
            .store
            .chunk(
                &feed.id,
                u64::try_from(start).expect("offset"),
                offered,
                eof,
            )
            .expect("sequential prefix at current cursor");
        assert_eq!(
            cursor,
            u64::try_from(expected).expect("offset"),
            "committed byte cursor"
        );
        feed.cursor = expected;
        self.check();
    }

    fn finish(&mut self, index: usize) {
        if !matches!(self.feeds[index].kind, Kind::Failed(..)) {
            self.chunk(index, self.feeds[index].bytes.len(), true);
        }
    }

    fn remove(&mut self, index: usize) {
        self.store
            .remove(&self.feeds[index].id)
            .expect("idempotent removal");
        self.feeds[index].active = false;
        self.check();
    }

    fn reload(&mut self) {
        let text = self.store.render().expect("bounded fixture renders");
        if self.privacy_canary {
            assert!(
                !text.contains(PRIVATE),
                "persisted evidence must discard prompt/brief/command text"
            );
        }
        self.store = Store::parse(&text).expect("persisted evidence parses");
        self.check();
    }

    fn snapshots(&self) -> Vec<View> {
        SCOPES
            .into_iter()
            .map(|scope| self.store.view(scope))
            .collect()
    }
}

fn limited_oracle(feeds: &[Feed], scope: Option<&str>, cap: usize) -> View {
    let mut fold = Fold::default();
    for feed in feeds.iter().filter(|feed| feed.active) {
        let opened = match &feed.kind {
            Kind::Live(meta) => Reading::live(&feed.label, meta, u64::MAX),
            Kind::Archive(session) => Reading::archive(session, u64::MAX),
            Kind::Failed(session, gap) => {
                Err(Source::failed(&feed.label, session.as_deref(), *gap))
            }
        };
        match opened {
            Ok(mut reading) => {
                for line in feed.selected().split(|byte| *byte == b'\n') {
                    if line.len() > cap {
                        reading.over_long();
                    } else {
                        reading.line(line).expect("uncapped retained memory");
                    }
                }
                fold.add_reading(reading);
            }
            Err(failed) => fold.add(failed),
        }
    }
    fold.finish(scope)
}

#[test]
fn maintained_view_matches_hand_counts_and_survives_reload() {
    let mut h = Harness::new(cases::hand_feeds());
    h.check();
    for index in 0..h.feeds.len() {
        h.register(index);
        h.finish(index);
    }
    assert_eq!(h.store.view(Some(S)), cases::hand_view());
    h.reload();
    for scope in SCOPES {
        let view = h.store.view(scope);
        assert_eq!(
            parse_view(&render_view(&view)).expect("view round trip"),
            view
        );
    }
    h.remove(0);
    assert_eq!(h.store.view(Some(S)).all.total, 0);
    h.remove(0);
}

#[test]
fn event_ahead_of_cursor_then_chunks_is_exactly_once_and_has_no_history_debt() {
    let rows = vec![
        (cases::event("memo", ""), false),
        (
            cases::birth(
                S,
                1,
                &cases::fields("codex", None, Some("sol"), Some("high")),
            ),
            true,
        ),
        (cases::link(1), true),
        (cases::legacy(PRIVATE), false),
    ];
    let mut h = Harness::new(vec![Feed::live("a", "live", Some(S), rows)]);
    h.register(0);
    h.insert(0, 2); // Lost link first, then a certificate refines the same birth.
    assert_eq!(h.store.view(Some(S)).all.total, 1);
    assert!(
        h.store
            .view(Some(S))
            .all
            .gaps
            .iter()
            .any(|g| g.reason == Reason::CertificateLost)
    );
    h.reload();
    h.insert(0, 1);
    let once = h.store.render().expect("once-inserted evidence");
    h.insert(0, 1);
    assert_eq!(
        h.store.render().expect("duplicate evidence"),
        once,
        "duplicate event must not grow or change evidence"
    );
    assert_eq!(h.store.view(Some(S)).all.model_known, 1);
    h.reload();
    let end = h.feeds[0].records[0].1 + 1;
    h.chunk(0, end, false);
    h.finish(0);
    let caught_up = h.snapshots();
    let caught_up_evidence = h.store.render().expect("caught-up evidence");
    h.insert(0, 2);
    assert_eq!(
        h.store.render().expect("behind-cursor link"),
        caught_up_evidence
    );
    h.insert(0, 1);
    assert_eq!(
        h.store.render().expect("behind-cursor birth"),
        caught_up_evidence
    );
    assert_eq!(
        h.snapshots(),
        caught_up,
        "event after catch-up is also idempotent"
    );
    h.reload();
}

#[test]
fn persisted_size_is_independent_of_summary_length_in_legacy_link_and_memo() {
    fn build(summary: &str, changed: &str) -> Harness {
        let selected = |kind| if kind == changed { summary } else { PRIVATE };
        // Keep every cursor/offset in the same five-decimal-digit range in both
        // fixtures, so legitimate byte-position persistence cannot change size.
        let padding = cases::event("memo", &format!(",\"summary\":\"{}\"", "P".repeat(20_000)));
        let feed = Feed::live(
            "privacy",
            "privacy",
            Some(S),
            vec![
                (padding, false),
                (
                    cases::birth(S, 1, &cases::fields("codex", None, Some("sol"), None)),
                    true,
                ),
                (cases::link(1).replace(PRIVATE, selected("link")), true),
                (cases::legacy(selected("legacy")), false),
                (
                    cases::event("memo", &format!(",\"summary\":\"{}\"", selected("memo"))),
                    false,
                ),
            ],
        );
        let mut h = Harness::new(vec![feed]);
        h.register(0);
        h.insert(0, 2); // Judge the event path as well as sequential reduction.
        h.finish(0);
        h.reload();
        h
    }
    for kind in ["legacy", "link", "memo"] {
        let short = build(PRIVATE, kind);
        let long = build(&PRIVATE.repeat(64), kind);
        assert_eq!(
            short.snapshots(),
            long.snapshots(),
            "summary is not statistics: {kind}"
        );
        assert_eq!(
            short.store.render().expect("short prompt").len(),
            long.store.render().expect("long prompt").len(),
            "persisted {kind} evidence must not retain encoded or literal prompt bytes"
        );
    }
}

#[test]
fn every_byte_boundary_and_final_unterminated_line_preserve_cursor_and_prefix() {
    let first = cases::birth(S, 1, &cases::fields("codex", None, Some("sol"), None));
    let tail = cases::event("memo", "").trim_end_matches('\n').to_owned();
    let feed = Feed::live("a", "live", Some(S), vec![(first, true), (tail, false)]);
    let mut h = Harness::new(vec![feed]);
    h.register(0);
    h.insert(0, 0);
    for end in 0..=h.feeds[0].bytes.len() {
        if end >= h.feeds[0].cursor {
            h.chunk(0, end, false);
        }
        if end % 17 == 0 {
            h.reload();
        }
    }
    let before = h.feeds[0].cursor;
    assert!(
        before < h.feeds[0].bytes.len(),
        "unterminated tail remains for EOF"
    );
    h.finish(0);
    h.reload();
}

#[test]
fn all_foreign_idless_source_gap_range_and_conflict_fields_equal_full_fold() {
    let mut h = Harness::new(cases::fleet(3));
    for index in (0..h.feeds.len()).rev() {
        h.register(index);
        h.finish(index);
        h.reload();
    }
    let all = h.store.view(Some(S)).all;
    for reason in [
        Reason::Malformed,
        Reason::Damaged,
        Reason::Unsupported,
        Reason::Conflict,
        Reason::CertificateLost,
        Reason::SessionMismatch,
        Reason::NoSessionId,
        Reason::LiveNoBirth,
    ] {
        assert!(
            all.gaps.iter().any(|gap| gap.reason == reason),
            "fixture witness for {reason:?}"
        );
    }
    assert!(
        all.gaps
            .iter()
            .any(|g| matches!(g.reason, Reason::Unproved { .. }))
    );
    assert!(
        all.gaps
            .iter()
            .any(|g| matches!(g.reason, Reason::LegacyExcluded { .. }))
    );
    assert_eq!(
        all.range,
        Some(("2024-03-02".to_owned(), "2027-12-31".to_owned()))
    );
    for scope in SCOPES {
        let view = h.store.view(scope);
        assert_eq!(
            parse_view(&render_view(&view)).expect("all reasons/cells/scopes round trip"),
            view
        );
    }
    for index in [6, 0, 5, 2, 4, 3, 1] {
        h.remove(index);
        h.reload();
    }
    for index in 7..h.feeds.len() {
        h.remove(index);
    }
    assert_eq!(h.store.view(Some(S)).all, stats::Scoped::default());
}

#[test]
fn foreign_claim_context_controls_own_link_suppression_and_live_born_proof() {
    let cells = cases::fields("codex", None, Some("sol"), None);
    let mut origin = Feed::live(
        "foreign",
        "origin-A",
        Some(S),
        vec![
            (cases::birth(T, 1, &cells), true),
            (cases::link(1), true),
            (cases::fact(T, 1, "observed"), false),
        ],
    );
    if let Kind::Live(meta) = &mut origin.kind {
        meta.extend_from_slice(b"seat.spawned.0=scout\n");
    }
    let mut h = Harness::new(vec![
        origin,
        Feed::live(
            "b",
            "origin-B",
            Some(T),
            vec![(cases::birth(T, 1, &cells), true)],
        ),
        Feed::live(
            "none",
            "no-id",
            None,
            vec![(cases::birth(T, 1, &cells), true), (cases::link(1), true)],
        ),
    ]);
    for index in 0..3 {
        h.register(index);
        h.finish(index);
    }
    assert_eq!(
        h.store.view(Some(S)).all.total,
        1,
        "one key across all origin groups"
    );
    let ThisSession::Ready(this) = h.store.view(Some(S)).this else {
        panic!("given UUID")
    };
    assert_eq!(
        this.total, 0,
        "own link suppressed by same-source foreign claim"
    );
    assert!(!this.gaps.iter().any(|g| g.reason == Reason::LiveNoBirth));
    h.remove(0);
    assert_eq!(
        h.store.view(Some(T)).all.model_known,
        1,
        "removing foreign unknown restores proved cell"
    );
    h.reload();
}

#[test]
fn two_divergent_legacy_groups_aggregate_fields_and_removal_restores_each_longest_list() {
    let mut h = Harness::new(vec![
        Feed::live(
            "s1",
            "S-one",
            Some(S),
            vec![(cases::legacy("A"), false), (cases::legacy("A"), false)],
        ),
        Feed::live("s2", "S-two", Some(S), vec![(cases::legacy("B"), false)]),
        Feed::live("t1", "T-one", Some(T), vec![(cases::legacy("C"), false)]),
        Feed::live(
            "t2",
            "T-two",
            Some(T),
            vec![
                (cases::legacy("D"), false),
                (cases::legacy("D"), false),
                (cases::legacy("D"), false),
            ],
        ),
    ]);
    for index in 0..4 {
        h.register(index);
        h.finish(index);
    }
    let gaps = h.store.view(None).all.gaps;
    let excluded: Vec<_> = gaps
        .iter()
        .filter(|g| matches!(g.reason, Reason::LegacyExcluded { .. }))
        .collect();
    assert_eq!(excluded.len(), 1, "one aggregate, not one row per group");
    assert_eq!(
        excluded[0].reason,
        Reason::LegacyExcluded {
            sessions: 2,
            a: 2,
            b: 5
        }
    );
    h.reload();
    h.remove(1);
    assert_eq!(h.store.view(None).all.total, 2);
    h.remove(2);
    assert_eq!(h.store.view(None).all.total, 5);
}

#[test]
fn line_limit_invalid_utf8_malformed_and_empty_lines_have_exact_gap_counts() {
    let mut bytes = vec![
        ("bad-json\n".to_owned(), false),
        ("x\n".to_owned(), false),
        (" \t\r\n".to_owned(), false),
        (format!("{}\n", "z".repeat(96)), false),
        (format!("{}\n", "z".repeat(97)), false),
        (
            cases::event("memo", &format!(",\"summary\":\"{PRIVATE}\""))
                .replace("2026-10-01", "2023-02-01"),
            false,
        ),
    ];
    // The ordinary event above is also over 96 bytes; both are line-limit gaps.
    let mut feed = Feed::live("a", "hostile", Some(S), std::mem::take(&mut bytes));
    let start = feed.records[1].0;
    feed.bytes[start] = 0xff;
    let mut h = Harness::new(vec![feed]);
    h.store = Store::with_line_limit(96);
    h.line_limit = Some(96);
    h.register(0);
    h.finish(0);
    let gaps = h.store.view(Some(S)).all.gaps;
    assert_eq!(
        gaps.iter()
            .find(|g| g.reason == Reason::Malformed)
            .expect("malformed")
            .count,
        3
    );
    assert_eq!(
        gaps.iter()
            .find(|g| g.reason == Reason::LineLimit)
            .expect("over limit")
            .count,
        2
    );
    let view = h.store.view(Some(S));
    assert_eq!(
        parse_view(&render_view(&view)).expect("LineLimit gap round trip"),
        view
    );
    h.reload();
    h.register(0);
    h.finish(0); // The custom line cap must survive persistence too.
}

#[test]
fn default_line_limit_is_enforced_by_the_chunk_consumer() {
    let cap = 4 * 1024 * 1024;
    let mut bytes = vec![b'x'; cap + 1];
    bytes.push(b'\n');
    let feed = Feed::raw(
        "a",
        "large-line",
        Kind::Live(format!("session_id={S}\n").into_bytes()),
        bytes,
    );
    let mut h = Harness::new(vec![feed]);
    h.line_limit = Some(cap); // Independent oracle cap; Store stays Default.
    h.register(0);
    h.finish(0);
    assert_eq!(
        h.store
            .view(Some(S))
            .all
            .gaps
            .iter()
            .find(|g| g.reason == Reason::LineLimit)
            .expect("default cap consumed")
            .count,
        1
    );
    h.reload();
}

#[test]
fn replacing_same_source_id_drops_old_evidence_and_absent_remove_is_idempotent() {
    let mut h = Harness::new(cases::hand_feeds());
    for index in 0..h.feeds.len() {
        h.register(index);
        h.finish(index);
    }
    h.feeds[0] = Feed::live(
        "known",
        "renamed",
        Some(T),
        vec![(
            cases::birth(
                T,
                9,
                &cases::fields("claude", Some("claude-mic"), None, Some("max")),
            ),
            true,
        )],
    );
    h.register(0);
    assert_eq!(h.store.view(Some(S)).all.total, 0);
    h.finish(0);
    h.reload();
    h.feeds[0] = Feed::new(
        "known",
        "failed-replacement",
        Kind::Failed(Some(T.to_owned()), SourceGap::Invalid),
        vec![],
    );
    h.register(0);
    h.reload();
    h.remove(0);
    h.remove(0);
}

#[test]
fn rejected_cursor_unknown_source_and_event_kinds_are_transactional() {
    fn refuse(h: &mut Harness) {
        let before = h.store.render().expect("render");
        assert!(h.store.chunk("unknown", 0, b"\n", true).is_err());
        assert!(h.store.event("unknown", 0, b"{}").is_err());
        let wrong = u64::try_from(h.feeds[0].cursor).expect("cursor") + 1;
        assert!(h.store.chunk("known", wrong, b"\n", false).is_err());
        for row in [
            cases::legacy(PRIVATE),
            cases::fact(S, 1, "observed"),
            cases::event("memo", ""),
            "not-json\n".to_owned(),
        ] {
            assert!(
                h.store
                    .event("known", 0, row.trim_end_matches('\n').as_bytes())
                    .is_err()
            );
        }
        let valid = cases::birth(S, 2, &cases::fields("codex", None, None, None));
        assert!(
            h.store
                .event("unread", 0, valid.trim_end_matches('\n').as_bytes())
                .is_err()
        );
        assert!(h.store.chunk("unread", 0, valid.as_bytes(), true).is_err());
        assert_eq!(
            h.store.render().expect("render"),
            before,
            "no failed operation changes evidence/cursor/views"
        );
        h.check();
    }
    let mut h = Harness::new(cases::hand_feeds());
    h.register(0);
    h.register(1);
    refuse(&mut h);
    let first = h.feeds[0].records[0].1 + 1;
    h.chunk(0, first, false);
    h.insert(0, 1);
    refuse(&mut h); // Nonempty prefix, advanced cursor and an inserted event.
    h.finish(0);
    h.reload();
}

fn shuffled(mut x: u64) -> u64 {
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn choose(value: u64, len: usize) -> usize {
    usize::try_from(value % u64::try_from(len).expect("nonempty bounded collection"))
        .expect("index")
}

fn interleave(seed: u64, controls: &[u8], rounds: usize) {
    let mut h = Harness::new(cases::fleet(seed));
    let mut random = seed + 1;
    for step in 0..rounds {
        random = shuffled(random);
        let control = controls
            .get(step % controls.len().max(1))
            .copied()
            .map_or(random, u64::from);
        let index = choose(random >> 8, h.feeds.len());
        if !h.feeds[index].active {
            h.register(index);
        }
        match control % 7 {
            0 => h.reload(),
            1 => h.remove(index),
            2 => h.register(index),
            3 | 4 => {
                let inserts: Vec<_> = h.feeds[index]
                    .records
                    .iter()
                    .enumerate()
                    .filter_map(|(n, record)| record.2.then_some(n))
                    .collect();
                if !inserts.is_empty() {
                    let record = inserts[choose(random >> 16, inserts.len())];
                    h.insert(index, record);
                }
            }
            _ => {
                if !matches!(h.feeds[index].kind, Kind::Failed(..)) {
                    let cursor = h.feeds[index].cursor;
                    let next = h.feeds[index].bytes[cursor..]
                        .iter()
                        .position(|b| *b == b'\n')
                        .map_or(h.feeds[index].bytes.len(), |n| cursor + n + 1);
                    h.chunk(index, next, next == h.feeds[index].bytes.len());
                }
            }
        }
    }
    // Every run ends with whole-source equality, then subtraction after catch-up.
    for index in 0..h.feeds.len() {
        if !h.feeds[index].active {
            h.register(index);
        }
        h.finish(index);
    }
    h.reload();
    for index in (0..h.feeds.len()).rev() {
        h.remove(index);
    }
}

#[test]
fn generated_operation_interleavings_equal_fold_after_every_transition() {
    for seed in 0..24 {
        interleave(seed, &[], 70);
    }
}

#[test]
fn all_360_legal_small_interleavings_cover_events_chunks_removal_and_reload() {
    fn permutations(prefix: &mut Vec<u8>, used: u8, out: &mut Vec<Vec<u8>>) {
        if prefix.len() == 6 {
            // Chunks are sequential; the two event inserts, remove and reload
            // can occur at every position and in either order.
            if prefix.iter().position(|n| *n == 2) < prefix.iter().position(|n| *n == 3) {
                out.push(prefix.clone());
            }
            return;
        }
        for n in 0..6 {
            if used & (1 << n) == 0 {
                prefix.push(n);
                permutations(prefix, used | (1 << n), out);
                prefix.pop();
            }
        }
    }
    let mut orders = Vec::new();
    permutations(&mut Vec::new(), 0, &mut orders);
    assert_eq!(orders.len(), 360);
    for order in orders {
        let mut h = Harness::new(vec![Feed::live(
            "a",
            "live",
            Some(S),
            vec![
                (
                    cases::birth(S, 1, &cases::fields("codex", None, Some("sol"), None)),
                    true,
                ),
                (cases::link(1), true),
            ],
        )]);
        h.register(0);
        for op in order {
            match op {
                0 | 1 if h.feeds[0].active => h.insert(0, usize::from(op)),
                2 if h.feeds[0].active => {
                    let end = h.feeds[0].records[0].1 + 1;
                    h.chunk(0, end, false);
                }
                3 if h.feeds[0].active => h.finish(0),
                4 => h.reload(),
                5 => h.remove(0),
                _ => {
                    assert!(!h.feeds[0].active);
                    if op < 2 {
                        assert!(h.store.event("a", 0, b"{}").is_err());
                    } else {
                        assert!(h.store.chunk("a", 0, b"", true).is_err());
                    }
                    h.check();
                }
            }
        }
        assert_eq!(h.store.view(Some(S)).all.total, 0);
    }
}

/// Shared by the compiled `stats_store` cargo-fuzz target. Raw file parsing is
/// separate from generated valid journals: hostile byte mutations exercise both.
fn exercise_parsers(data: &[u8]) {
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(store) = Store::parse(text) {
            let rendered = store.render().expect("accepted bounded state renders");
            let restored = Store::parse(&rendered).expect("accepted state round trips");
            for scope in SCOPES {
                let view = store.view(scope);
                cases::assert_balanced(&view);
                assert_eq!(restored.view(scope), view);
            }
        }
        if let Ok(view) = parse_view(text) {
            cases::assert_balanced(&view);
            assert_eq!(
                parse_view(&render_view(&view)).expect("accepted view round trips"),
                view
            );
        }
    }
}

pub fn fuzz_case(data: &[u8]) {
    exercise_parsers(data);
    // Most executions spend their budget entirely on the hostile file parsers.
    // One mode in sixteen also checks the heavier journal/update oracle.
    if data.first().is_none_or(|mode| mode % 16 != 0) {
        return;
    }
    let seed = data
        .iter()
        .take(8)
        .fold(0_u64, |n, byte| (n << 8) | u64::from(*byte));
    interleave(seed % 64, data, 4 + data.len().min(20));
    // Arbitrary journal bytes, independently attributed to a live UUID, an
    // archive UUID, and no UUID. This also fuzzes reduction of unknown members,
    // repeated keys, bad UTF-8 and family/envelope grammar, not just controls.
    let bounded = &data[..data.len().min(4096)];
    let mut feeds = Vec::new();
    for (n, part) in bounded.split(|byte| *byte == 0).take(3).enumerate() {
        let kind = match n {
            0 => Kind::Live(format!("session_id={S}\n").into_bytes()),
            1 => Kind::Archive(T.to_owned()),
            _ => Kind::Live(Vec::new()),
        };
        let label = if n == 1 {
            T.to_owned()
        } else {
            format!("raw-{n}")
        };
        feeds.push(Feed::raw(&format!("raw-{n}"), &label, kind, part.to_vec()));
    }
    let mut h = Harness::new(feeds);
    // An arbitrary model/label may legitimately equal the fixture prompt marker.
    h.privacy_canary = false;
    for n in 0..h.feeds.len() {
        h.register(n);
        let inserts: Vec<_> = h.feeds[n]
            .records
            .iter()
            .enumerate()
            .filter_map(|(index, row)| row.2.then_some(index))
            .take(4)
            .collect();
        for index in inserts.into_iter().rev() {
            h.insert(n, index);
        }
        let size = h.feeds[n].bytes.len();
        for part in 1..=4 {
            h.chunk(n, size * part / 4, part == 4);
        }
        h.reload();
    }
    if !h.feeds.is_empty() {
        h.remove(0);
        h.reload();
    }
}

#[test]
fn hostile_persisted_text_is_refused_and_accepted_state_round_trips() {
    for bad in [
        "",
        "null",
        "{}",
        "[]",
        "true",
        "0",
        "{",
        "\0",
        "NaN",
        "{\"stats_store\":999999}",
    ] {
        assert!(
            Store::parse(bad).is_err(),
            "not an evidence document: {bad:?}"
        );
        assert!(parse_view(bad).is_err(), "not a view document: {bad:?}");
    }
    let mut h = Harness::new(cases::hand_feeds());
    for n in 0..h.feeds.len() {
        h.register(n);
        h.finish(n);
    }
    let evidence = h.store.render().expect("evidence");
    let view = render_view(&h.store.view(Some(S)));
    for text in [&evidence, &view] {
        for end in 0..text.len() {
            if text.is_char_boundary(end) {
                exercise_parsers(&text.as_bytes()[..end]);
            }
        }
    }
    for raw in [vec![0xff, 0, 0xc0, 0x80], vec![b'['; 512], vec![b'9'; 256]] {
        fuzz_case(&raw);
    }
}

#[test]
fn view_parser_rejects_inconsistent_counters_rows_dates_and_scope_relationships() {
    let valid = cases::hand_view();
    assert_eq!(
        parse_view(&render_view(&valid)).expect("valid hand view"),
        valid
    );
    let mut invalid = Vec::new();
    let mut v = valid.clone();
    v.all.total += 1;
    invalid.push(v);
    let mut v = valid.clone();
    v.all.sources_read = v.all.sources_total + 1;
    invalid.push(v);
    let mut v = valid.clone();
    v.all.model_known = v.all.total + 1;
    invalid.push(v);
    let mut v = valid.clone();
    v.all.effort_known = v.all.total + 1;
    invalid.push(v);
    let mut v = valid.clone();
    v.all.legacy = v.all.total + 1;
    invalid.push(v);
    let mut v = valid.clone();
    v.all.harness[0].cells.push(stats::Cell::Unknown);
    invalid.push(v);
    let mut v = valid.clone();
    v.all.client[0].count = u64::MAX;
    invalid.push(v);
    let mut v = valid.clone();
    v.all.range = Some(("2026-12-31".to_owned(), "2026-01-01".to_owned()));
    invalid.push(v);
    let mut v = valid.clone();
    v.all.gaps[0].count = 0;
    invalid.push(v);
    let mut v = valid.clone();
    v.all.joint[1].cells = v.all.joint[0].cells.clone();
    invalid.push(v);
    let mut v = valid.clone();
    v.all.harness.reverse();
    invalid.push(v);
    let mut v = valid.clone();
    v.all.gaps.push(v.all.gaps[0].clone());
    invalid.push(v);
    for field in [
        "legacy",
        "model_known",
        "effort_known",
        "sources_read",
        "sources_total",
    ] {
        let mut v = valid.clone();
        let ThisSession::Ready(this) = &mut v.this else {
            panic!("hand fixture UUID")
        };
        match field {
            "legacy" => this.legacy = v.all.legacy + 1,
            "model_known" => this.model_known = v.all.model_known + 1,
            "effort_known" => this.effort_known = v.all.effort_known + 1,
            "sources_read" => this.sources_read = v.all.sources_read + 1,
            _ => this.sources_total = v.all.sources_total + 1,
        }
        assert!(this.balanced(), "isolate cross-scope {field} validation");
        invalid.push(v);
    }
    for date in ["not-a-date", "2026-1-01", "2026-10-xx"] {
        let mut v = valid.clone();
        v.all.range = Some((date.to_owned(), date.to_owned()));
        invalid.push(v);
    }
    let mut v = valid;
    v.all = stats::Scoped::default();
    invalid.push(v);
    for (case, view) in invalid.iter().enumerate() {
        assert!(
            parse_view(&render_view(view)).is_err(),
            "corrupt materialized view case {case}"
        );
    }
}
