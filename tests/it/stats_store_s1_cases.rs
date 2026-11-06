//! Frozen S1 fixture/oracle lane. No `stats_store` implementation is used here.
#![allow(
    dead_code,
    reason = "shared oracle fixtures serve tests and the fuzz bin"
)]
#![allow(clippy::expect_used, reason = "fixture construction is infallible")]

use ae::stats::{self, Cell, Gap, Reason, Row, Scoped, Source, SourceGap, ThisSession, View};
use std::collections::BTreeSet;
use std::fmt::Write as _;

pub const S: &str = "11111111-1111-4111-8111-111111111111";
pub const T: &str = "22222222-2222-4222-8222-222222222222";
pub const U: &str = "33333333-3333-4333-8333-333333333333";
pub const PRIVATE: &str = "PRIVATE-PROMPT-BRIEF-COMMAND-TOKEN-https://private.invalid";
pub const SCOPES: [Option<&str>; 4] = [None, Some(S), Some(T), Some(U)];

pub fn id(n: u32) -> String {
    format!("aaaaaaaa-aaaa-4aaa-8aaa-{n:012x}")
}

pub fn event(action: &str, extra: &str) -> String {
    format!(
        "{{\"ts\":\"2026-10-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"{action}\",\"target\":\"scout\"{extra}}}\n"
    )
}

pub fn birth(session: &str, n: u32, fields: &str) -> String {
    event(
        "worker-birth",
        &format!(
            ",\"target_slot\":\"spawned.0\",\"target_session_id\":\"{session}\",\"birth_v\":1,\"birth\":\"{}\"{fields}",
            id(n)
        ),
    )
}

pub fn fields(
    tool: &str,
    client: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
) -> String {
    let mut fields = format!(",\"birth_tool\":\"{tool}\",\"birth_profile\":\"preset\"");
    for (key, value) in [
        ("birth_client", client),
        ("birth_model", model),
        ("birth_effort", effort),
    ] {
        if let Some(value) = value {
            write!(fields, ",\"{key}\":\"{value}\"").expect("write to String");
        }
    }
    fields
}

pub fn link(n: u32) -> String {
    event(
        "spawn",
        &format!(",\"birth\":\"{}\",\"summary\":\"{PRIVATE}\"", id(n)),
    )
}

pub fn legacy(summary: &str) -> String {
    event("spawn", &format!(",\"summary\":\"{summary}\""))
}

pub fn fact(session: &str, n: u32, kind: &str) -> String {
    event(
        "worker-fact",
        &format!(
            ",\"target_session_id\":\"{session}\",\"birth_v\":1,\"birth\":\"{}\",\"fact\":\"{kind}\",\"start\":\"spawned.0:{}:41\",\"observed_source\":\"frame\",\"observed_model\":\"runtime-only\",\"observed_effort\":\"max\"",
            id(n),
            id(n)
        ),
    )
}

#[derive(Clone)]
pub enum Kind {
    Live(Vec<u8>),
    Archive(String),
    Failed(Option<String>, SourceGap),
}

#[derive(Clone)]
pub struct Feed {
    pub id: String,
    pub label: String,
    pub kind: Kind,
    pub bytes: Vec<u8>,
    /// Original journal positions; LF is excluded from the line contents.
    pub records: Vec<(usize, usize, bool)>,
    pub cursor: usize,
    pub inserted: BTreeSet<usize>,
    pub active: bool,
}

impl Feed {
    pub fn raw(id: &str, label: &str, kind: Kind, bytes: Vec<u8>) -> Self {
        let mut feed = Self::new(id, label, kind, vec![]);
        let mut start = 0;
        for line in bytes.split(|byte| *byte == b'\n') {
            let insertable = std::str::from_utf8(line)
                .ok()
                .and_then(|text| ae::events::Event::parse_line(text).ok())
                .and_then(|event| ae::birth::decode(&event))
                .is_some_and(|record| {
                    matches!(
                        record,
                        Ok(ae::birth::Record::Birth(_) | ae::birth::Record::SpawnLink { .. })
                    )
                });
            feed.records.push((start, start + line.len(), insertable));
            start += line.len() + 1;
        }
        feed.bytes = bytes;
        feed
    }

    pub fn live(id: &str, label: &str, session: Option<&str>, rows: Vec<(String, bool)>) -> Self {
        let meta = session.map_or_else(Vec::new, |s| format!("session_id={s}\n").into_bytes());
        Self::new(id, label, Kind::Live(meta), rows)
    }

    pub fn archive(id: &str, session: &str, rows: Vec<(String, bool)>) -> Self {
        Self::new(id, session, Kind::Archive(session.to_owned()), rows)
    }

    pub fn new(id: &str, label: &str, kind: Kind, rows: Vec<(String, bool)>) -> Self {
        let mut bytes = Vec::new();
        let mut records = Vec::new();
        for (line, insertable) in rows {
            let start = bytes.len();
            bytes.extend_from_slice(line.as_bytes());
            let end = bytes.len() - usize::from(bytes.last() == Some(&b'\n'));
            records.push((start, end, insertable));
        }
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
            kind,
            bytes,
            records,
            cursor: 0,
            inserted: BTreeSet::new(),
            active: false,
        }
    }

    pub fn reset(&mut self) {
        self.cursor = 0;
        self.inserted.clear();
        self.active = true;
    }

    pub fn selected(&self) -> Vec<u8> {
        let mut selected = Vec::new();
        for (index, &(start, end, _)) in self.records.iter().enumerate() {
            let after = end + usize::from(self.bytes.get(end) == Some(&b'\n'));
            if after <= self.cursor || self.inserted.contains(&index) {
                selected.extend_from_slice(&self.bytes[start..end]);
                selected.push(b'\n');
            }
        }
        selected
    }

    pub fn source(&self) -> Source {
        let bytes = self.selected();
        match &self.kind {
            Kind::Live(meta) => Source::live(&self.label, meta, Ok(bytes)),
            Kind::Archive(session) => Source::archive(session, Ok(bytes)),
            Kind::Failed(session, gap) => Source::failed(&self.label, session.as_deref(), *gap),
        }
    }
}

pub fn oracle(feeds: &[Feed], scope: Option<&str>) -> View {
    stats::fold(
        feeds
            .iter()
            .filter(|feed| feed.active)
            .map(Feed::source)
            .collect(),
        scope,
    )
}

pub fn assert_balanced(view: &View) {
    assert!(view.all.balanced(), "ALL: {:?}", view.all);
    if let ThisSession::Ready(this) = &view.this {
        assert!(this.balanced(), "THIS: {this:?}");
        assert!(this.total <= view.all.total);
    }
}

/// Deterministic generated families, including fields that must survive reduction.
#[allow(
    clippy::too_many_lines,
    reason = "one fixture matrix makes all interacting evidence visible"
)]
pub fn fleet(seed: u64) -> Vec<Feed> {
    let model = ["sonnet", "opus", "sol", "escaped-\\\"-model"]
        [usize::try_from(seed % 4).expect("bounded index")];
    let a = fields("claude", Some("claude-p"), Some(model), Some("high"));
    let b = fields("codex", None, Some("sol"), None);
    let original = birth(S, 1, &a);
    let duplicate = original.replace("\"target\":\"scout\"", "\"target\":\"renamed\"");
    let damaged = birth(S, 3, &a).replace("\"birth_effort\":\"high\"", "\"birth_effort\":7");
    let unsupported = birth(S, 2, &b).replace("\"birth_v\":1", "\"birth_v\":2");
    let unproved = format!(
        "{},\"birth_model_unproved\":\"ambiguous-model\",\"birth_effort_unproved\":\"unsupported\"",
        fields("muse", None, None, None)
    );
    let mut live = Feed::live(
        "a",
        "zz-live",
        Some(S),
        vec![
            (
                event("memo", &format!(",\"summary\":\"{PRIVATE}\""))
                    .replace("2026-10-01", "2025-01-01"),
                false,
            ),
            (fact(S, 1, "start-attempted"), false),
            (original, true),
            (link(1), true),
            (legacy(PRIVATE), false),
            (legacy(PRIVATE), false),
            (unsupported, false),
            (damaged, false),
            (link(10), true),
            ("not-json\n".to_owned(), false),
            (birth(T, 8, &b), true),
            (link(8), true),
            (fact(T, 8, "observed"), false),
            (birth(S, 4, &a), true),
            (fact(S, 4, "start-attempted"), false),
            (
                event("resume", "").replace("2026-10-01", "2027-12-31"),
                false,
            ),
        ],
    );
    if let Kind::Live(meta) = &mut live.kind {
        meta.extend_from_slice(b"seat.spawned.0=scout\nseat.spawned.1=silent\n");
    }
    let mut feeds = vec![
        live,
        Feed::archive(
            "archive-s",
            S,
            vec![(duplicate, true), (link(1), true), (legacy(PRIVATE), false)],
        ),
        Feed::live(
            "conflict",
            "00-conflict",
            Some(S),
            vec![
                (birth(S, 1, &a.replace(model, "other-model")), true),
                (birth(S, 4, &a), true),
                (fact(S, 4, "start-failed"), false),
            ],
        ),
        Feed::live(
            "t",
            "t-live",
            Some(T),
            vec![
                (birth(T, 8, &b), true),
                (link(8), true),
                (birth(T, 9, &unproved), true),
                (legacy("T-left"), false),
            ],
        ),
        Feed::archive(
            "archive-t",
            T,
            vec![(birth(T, 8, &b), true), (legacy("T-right"), false)],
        ),
        Feed::live(
            "idless",
            "no-id",
            None,
            vec![
                (birth(T, 8, &b), true),
                (link(8), true),
                (fact(T, 8, "observed"), false),
                (legacy(PRIVATE), false),
            ],
        ),
        Feed::live(
            "idless-range",
            "no-id-range",
            None,
            vec![(event("memo", "").replace("2026-10-01", "2024-03-02"), false)],
        ),
    ];
    for (n, gap) in [
        SourceGap::Missing,
        SourceGap::Unreadable,
        SourceGap::Oversized,
        SourceGap::Budget,
        SourceGap::Invalid,
        SourceGap::Claimed,
        SourceGap::Retained,
    ]
    .into_iter()
    .enumerate()
    {
        let session = [None, Some(S), Some(T)][n % 3].map(ToOwned::to_owned);
        feeds.push(Feed::new(
            &format!("failed-{n}"),
            &format!("gap-{n}"),
            Kind::Failed(session, gap),
            vec![],
        ));
    }
    feeds
}

/// Hand totals: one configured birth + one legacy spawn, plus one id-less failed
/// source. The failed source covers THIS once but contributes no spawn or range.
pub fn hand_feeds() -> Vec<Feed> {
    vec![
        Feed::live(
            "known",
            "known",
            Some(S),
            vec![
                (birth(S, 1, &fields("codex", None, Some("sol"), None)), true),
                (link(1), true),
                (legacy(PRIVATE), false),
            ],
        ),
        Feed::new(
            "unread",
            "global-gap",
            Kind::Failed(None, SourceGap::Missing),
            vec![],
        ),
    ]
}

pub fn hand_view() -> View {
    let row = |cells, count| Row { cells, count };
    let v = |s: &str| Cell::Value(s.to_owned());
    let scoped = Scoped {
        total: 2,
        legacy: 1,
        model_known: 1,
        effort_known: 1,
        range: Some(("2026-10-01".to_owned(), "2026-10-01".to_owned())),
        sources_read: 1,
        sources_total: 2,
        harness: vec![row(vec![v("codex")], 1), row(vec![Cell::Unknown], 1)],
        client: vec![row(vec![Cell::Direct], 1), row(vec![Cell::Unknown], 1)],
        model_effort: vec![
            row(vec![v("sol"), Cell::NoneSet], 1),
            row(vec![Cell::Unknown; 2], 1),
        ],
        joint: vec![
            row(vec![v("codex"), Cell::Direct, v("sol"), Cell::NoneSet], 1),
            row(vec![Cell::Unknown; 4], 1),
        ],
        gaps: vec![Gap {
            source: Some("global-gap".to_owned()),
            reason: Reason::Source(SourceGap::Missing),
            count: 1,
        }],
    };
    View {
        all: scoped.clone(),
        this: ThisSession::Ready(Box::new(scoped)),
    }
}

#[test]
fn independent_oracle_matches_hand_written_all_fields() {
    let mut feeds = hand_feeds();
    for feed in &mut feeds {
        feed.reset();
        feed.cursor = feed.bytes.len();
    }
    let expected = hand_view();
    assert_balanced(&expected);
    assert_eq!(oracle(&feeds, Some(S)), expected);
    let absent = oracle(&feeds, Some(U));
    let ThisSession::Ready(absent) = absent.this else {
        panic!("given UUID")
    };
    assert_eq!(
        (absent.total, absent.sources_read, absent.sources_total),
        (0, 0, 1)
    );
    assert_eq!(absent.gaps, expected.all.gaps);
    assert_eq!(absent.range, None);
}

#[test]
fn generated_oracle_is_balanced_order_independent_and_private() {
    for seed in 0..4 {
        let mut feeds = fleet(seed);
        for feed in &mut feeds {
            feed.reset();
            feed.cursor = feed.bytes.len();
        }
        for scope in SCOPES {
            let view = oracle(&feeds, scope);
            assert_balanced(&view);
            assert!(!format!("{view:?}").contains(PRIVATE));
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
                    view.all.gaps.iter().any(|g| g.reason == reason),
                    "fixture witness for {reason:?}"
                );
            }
            assert!(
                view.all
                    .gaps
                    .iter()
                    .any(|g| matches!(g.reason, Reason::Unproved { .. }))
            );
            assert!(
                view.all
                    .gaps
                    .iter()
                    .any(|g| matches!(g.reason, Reason::LegacyExcluded { .. }))
            );
            assert_eq!(
                view.all.range,
                Some(("2024-03-02".to_owned(), "2027-12-31".to_owned()))
            );
            feeds.reverse();
            assert_eq!(oracle(&feeds, scope), view);
        }
    }
}
