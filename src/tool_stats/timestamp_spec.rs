//! Per-stream timestamp extrema (issue #322): a stream keeps at most its
//! minimum and maximum stamps while counts, range, legacy decoding and
//! warm-open stability stay exact. Frozen acceptance tests; RED on main.
#![allow(
    clippy::expect_used,
    reason = "independent frozen acceptance assertions"
)]

use super::incr::{self, Conv, StreamKey};
use super::{Adapter, Caps, Event, Ident, Key, Load, StreamRole, doc};
use crate::tool_stats::{Ask, Filter, Page, Scope};

fn stream(role: StreamRole, copy: u8) -> StreamKey {
    StreamKey {
        role,
        copy: [copy; 16],
    }
}

fn parent() -> StreamKey {
    stream(StreamRole::Parent, 0)
}

fn stamps(values: &[i64]) -> Vec<Event> {
    values.iter().map(|at| Event::Stamp(*at)).collect()
}

fn attempt(id: u8, name: &str) -> Event {
    Event::Attempt {
        id: Key([id; 16]),
        tool: Ident::Name(name.into()),
        mcp: None,
        model: Some(Ident::Name("model-a".into())),
        turn: None,
        message: None,
    }
}

fn result(id: u8, error: bool) -> Event {
    Event::Result {
        id: Key([id; 16]),
        error,
        load: Some(Load::Unknown),
    }
}

fn stamp_count(conv: &Conv, key: &StreamKey) -> usize {
    conv.resume
        .streams
        .get(key)
        .map(|set| {
            set.iter()
                .filter(|event| matches!(event, Event::Stamp(_)))
                .count()
        })
        .unwrap_or(0)
}

fn stamp_values(conv: &Conv, key: &StreamKey) -> Vec<i64> {
    let mut out: Vec<i64> = conv.resume.streams.get(key).map_or_else(Vec::new, |set| {
        set.iter()
            .filter_map(|event| match event {
                Event::Stamp(at) => Some(*at),
                _ => None,
            })
            .collect()
    });
    out.sort_unstable();
    out
}

fn attempt_count(conv: &Conv, key: &StreamKey) -> usize {
    conv.resume.streams.get(key).map_or(0, |set| {
        set.iter()
            .filter(|event| matches!(event, Event::Attempt { .. }))
            .count()
    })
}

/// Bounded retention: 64 distinct stamps collapse to the two extrema while
/// attempts and results survive.
#[test]
fn timestamp_extrema_bounded_per_stream_after_many_stamps() {
    let key = parent();
    let values: Vec<i64> = (0..64).map(|n| 1000 + n * 10).collect();
    let mut events = stamps(&values);
    events.extend([attempt(1, "Read"), result(1, false)]);
    let mut conv = Conv::new(Adapter::Claude, "conv-bound");
    conv.apply(&key, &events, &Caps::SLICE);
    let kept = stamp_values(&conv, &key);
    assert_eq!(
        kept,
        vec![1000, 1000 + 63 * 10],
        "one stream keeps only its minimum and maximum stamps"
    );
    assert_eq!(stamp_count(&conv, &key), 2);
    assert_eq!(conv.summary.range, Some((1000, 1630)));
    assert_eq!(
        attempt_count(&conv, &key),
        1,
        "non-timestamp events are kept"
    );
}

/// Exact extrema under append, replay and out-of-order delivery.
#[test]
fn timestamp_extrema_exact_under_append_replay_order() {
    let key = parent();
    // Start in the middle, then extend both ends.
    let mut conv = Conv::new(Adapter::Claude, "conv-order");
    conv.apply(&key, &stamps(&[500, 600, 700]), &Caps::SLICE);
    conv.apply(&key, &stamps(&[450, 750, 550]), &Caps::SLICE);
    // Replay already-held evidence changes nothing.
    let before = conv.clone();
    conv.apply(&key, &stamps(&[450, 500, 600, 700, 750]), &Caps::SLICE);
    assert_eq!(conv, before, "replay of held stamps is a no-op");
    // Out-of-order batch still yields the same extrema.
    let mut other = Conv::new(Adapter::Claude, "conv-order");
    other.apply(&key, &stamps(&[750, 600, 450, 700, 500, 550]), &Caps::SLICE);
    assert_eq!(conv.summary.range, Some((450, 750)));
    assert_eq!(other.summary.range, Some((450, 750)));
    assert_eq!(
        stamp_values(&conv, &key),
        vec![450, 750],
        "append and replay keep only extrema"
    );
    assert_eq!(stamp_values(&other, &key), vec![450, 750]);
    // Same extrema as the cold fold over the full stamp set.
    let view = incr::combine(
        &incr::Published {
            generation: 1,
            convs: vec![incr::publish(&conv.summary, None)],
            over_cap: 0,
        },
        &Ask {
            page: Page::Tools,
            scope: Scope::All,
            filter: Filter::All,
        },
        &Caps::SLICE,
    );
    assert_eq!(view.range, Some((450, 750)));
}

/// Several streams plus a gone source stay independent.
#[test]
fn timestamp_extrema_independent_across_streams_and_gone() {
    let a = stream(StreamRole::Parent, 0);
    let b = stream(StreamRole::Child("agent-one".into()), 1);
    let mut conv = Conv::new(Adapter::Claude, "conv-multi");
    conv.apply(
        &a,
        &stamps(&(0..32).map(|n| 10_000 + n).collect::<Vec<_>>()),
        &Caps::SLICE,
    );
    conv.apply(
        &b,
        &stamps(&(0..32).map(|n| 20_000 + n).collect::<Vec<_>>()),
        &Caps::SLICE,
    );
    conv.source_gone(&b);
    // Extend only stream a; stream b extrema stay its own.
    conv.apply(&a, &stamps(&[9_999, 10_045]), &Caps::SLICE);
    assert_eq!(stamp_values(&conv, &a), vec![9_999, 10_045]);
    assert_eq!(stamp_values(&conv, &b), vec![20_000, 20_031]);
    assert_eq!(conv.summary.range, Some((9_999, 20_031)));
    assert_eq!(
        conv.summary.files.get(&b),
        Some(&incr::Source::Gone),
        "a gone source keeps its extrema and counts"
    );
}

fn legacy_conv(id: &str, key: &StreamKey, events: Vec<Event>) -> Conv {
    let mut conv = Conv::new(Adapter::Claude, id);
    conv.resume
        .streams
        .insert(key.clone(), events.into_iter().collect());
    conv.summarize();
    conv
}

/// Legacy many-stamp documents round-trip unchanged and compact only
/// after first new evidence; format stays 1.
#[test]
fn timestamp_extrema_legacy_compacts_only_after_new_evidence() {
    let key = parent();
    let values: Vec<i64> = (0..48).map(|n| 5_000 + n).collect();
    let mut events = stamps(&values);
    events.extend([attempt(9, "Bash"), result(9, false)]);
    // Direct insert: the pre-fix byte form, independent of apply.
    let legacy = legacy_conv("conv-legacy", &key, events);
    assert_eq!(
        stamp_count(&legacy, &key),
        48,
        "pre-decode: legacy stream holds all 48 stamps"
    );
    let bytes = doc::render_conv(&legacy);
    // Decode alone never compacts and round-trips byte-identically.
    let decoded = doc::parse_conv(&bytes).expect("legacy document parses");
    assert_eq!(decoded, legacy, "legacy parse keeps every stamp");
    assert_eq!(
        doc::render_conv(&decoded),
        bytes,
        "unchanged warm open rewrites nothing"
    );
    assert_eq!(doc::TOOLS_FORMAT, 1, "format version is preserved");
    // First new evidence compacts only this stream to extrema.
    let mut updated = decoded;
    updated.apply(&key, &stamps(&[4_999, 5_010]), &Caps::SLICE);
    assert_eq!(
        stamp_values(&updated, &key),
        vec![4_999, 5_047],
        "legacy stream compacts to extrema on first update"
    );
    assert_eq!(updated.summary.range, Some((4_999, 5_047)));
    assert_eq!(attempt_count(&updated, &key), 1);
    let fresh = doc::render_conv(&updated);
    assert!(fresh.len() < bytes.len(), "updated document is smaller");
    let reparsed = doc::parse_conv(&fresh).expect("updated document parses");
    assert_eq!(reparsed, updated);
}

/// Guard: decode without new evidence never compacts (green on main). The
/// Reader-level zero conversation bytes / zero Publish units for an
/// unchanged warm open are pinned by `calls_lazy_spec` warm-open tests.
#[test]
fn timestamp_extrema_decode_without_update_is_stable() {
    let key = parent();
    // Direct insert: decode-stable for legacy bytes, independent of apply.
    let conv = legacy_conv("conv-stable", &key, stamps(&[10, 20, 30, 40]));
    assert_eq!(
        stamp_count(&conv, &key),
        4,
        "pre-decode: legacy stream holds all 4 stamps"
    );
    let bytes = doc::render_conv(&conv);
    let decoded = doc::parse_conv(&bytes).expect("document parses");
    assert_eq!(doc::render_conv(&decoded), bytes);
    assert_eq!(
        stamp_count(&decoded, &key),
        4,
        "decode alone keeps all stamps"
    );
}

/// Idempotence guard: an apply that adds no evidence keeps a legacy
/// many-stamp document byte-identical. Compaction requires added evidence.
#[test]
fn timestamp_extrema_noop_apply_keeps_legacy_bytes() {
    let key = parent();
    let values: Vec<i64> = (0..32).map(|n| 7_000 + n).collect();
    let legacy = legacy_conv("conv-noop", &key, stamps(&values));
    assert_eq!(
        stamp_count(&legacy, &key),
        32,
        "pre-decode: legacy stream holds all 32 stamps"
    );
    let bytes = doc::render_conv(&legacy);
    let mut empty = legacy.clone();
    empty.apply(&key, &[], &Caps::SLICE);
    assert_eq!(
        doc::render_conv(&empty),
        bytes,
        "empty apply keeps legacy bytes"
    );
    assert_eq!(stamp_count(&empty, &key), 32);
    let mut held = legacy.clone();
    held.apply(&key, &stamps(&[7_000, 7_010, 7_031]), &Caps::SLICE);
    assert_eq!(
        doc::render_conv(&held),
        bytes,
        "held-only apply keeps legacy bytes"
    );
    assert_eq!(stamp_count(&held, &key), 32);
}
