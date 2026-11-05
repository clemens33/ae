use super::*;
use crate::stats::{self, Source};

const BUSY_BYTES: usize = 8_600_000;

fn busy_journal() -> Vec<u8> {
    let mut bytes = cert(1);
    bytes.extend(f::link(1).bytes());
    let ordinary = f::event("send", &format!(",\"summary\":\"{}\"", "x".repeat(4096)));
    while bytes.len() < BUSY_BYTES {
        bytes.extend(ordinary.bytes());
    }
    bytes.extend(f::legacy(f::PRIVATE).bytes());
    bytes.extend(cert(2));
    bytes.extend(f::link(2).trim_end_matches('\n').bytes());
    bytes
}

fn finish_with_bound(scan: &mut Scan, bound: usize) -> View {
    for _ in 0..bound {
        if let Some(view) = scan.step() {
            return view;
        }
    }
    panic!("Statistics did not finish within {bound} source steps");
}

#[test]
fn busy_live_counts_spawns_before_and_after_old_cap_and_finishes() {
    let root = Scratch::new("307-busy-live");
    let bytes = busy_journal();
    assert!(bytes.len() >= BUSY_BYTES);
    let dir = root.live("busy", f::S, &bytes);
    let mut scan = Scan::new(
        root.0.clone(),
        BTreeMap::from([("busy".into(), dir)]),
        true,
        1,
        Some(f::S.into()),
    );
    let started = Instant::now();
    let view = finish_with_bound(&mut scan, 5);
    eprintln!(
        "#307 synthetic busy journal: {} bytes, {:?}",
        bytes.len(),
        started.elapsed()
    );
    assert_eq!(
        view.all.total, 3,
        "two modern births plus one legacy announcement, including records past 4 MiB"
    );
    assert_eq!(view.all.legacy, 1);
    assert_eq!((view.all.sources_read, view.all.sources_total), (1, 1));
    assert!(
        view.all.gaps.is_empty(),
        "large total size alone loses no source: {:?}",
        view.all.gaps
    );
    assert!(!view.all.partial());
    assert!(view.all.balanced());
    let ThisSession::Ready(current) = &view.this else {
        panic!("known session")
    };
    assert_eq!(current.total, 3);
    assert!(
        !format!("{view:?}").contains(f::PRIVATE),
        "prompt canary never enters View"
    );
}

#[test]
fn busy_archive_counts_and_overlap_with_live_deduplicates() {
    let root = Scratch::new("307-busy-archive");
    let bytes = busy_journal();
    root.archived(f::S, &bytes);
    let mut archive = Scan::new(root.0.clone(), BTreeMap::new(), true, 1, Some(f::S.into()));
    let view = finish_with_bound(&mut archive, 5);
    assert_eq!(
        view.all.total, 3,
        "large validated archive retains all births and legacy spawns"
    );
    assert!(view.all.gaps.is_empty(), "{:?}", view.all.gaps);
    let dir = root.live("busy", f::S, &bytes);
    let mut both = Scan::new(
        root.0.clone(),
        BTreeMap::from([("busy".into(), dir)]),
        true,
        2,
        Some(f::S.into()),
    );
    let view = finish_with_bound(&mut both, 8);
    assert_eq!(
        view.all.total, 3,
        "live/archive copies are one history, including legacy"
    );
    assert_eq!((view.all.sources_read, view.all.sources_total), (2, 2));
    assert!(view.all.gaps.is_empty());
    assert!(view.all.balanced());
}

#[test]
fn small_full_view_keeps_noise_dates_malformed_utf8_and_escaped_actions() {
    let root = Scratch::new("307-small-parity");
    let (mut bytes, _) = f::matrix();
    bytes.extend(
        f::event("send", ",\"summary\":\"ordinary\"")
            .replace("2026-10-01", "2026-09-01")
            .bytes(),
    );
    bytes.extend(
        f::event("retire", "")
            .replace("2026-10-01", "2026-11-03")
            .bytes(),
    );
    bytes.extend(b"\n \t\r\nnot json\n\xff\n");
    bytes.extend(
        f::birth(
            f::S,
            20,
            &f::fields("codex", None, Some("sol"), Some("high")),
        )
        .replace("worker-birth", "worker-\\u0062irth")
        .trim_end_matches('\n')
        .bytes(),
    );
    let meta = format!("session_id={}\n", f::S);
    let expected = stats::fold(
        vec![Source::live("small", meta.as_bytes(), Ok(bytes.clone()))],
        Some(f::S),
    );
    assert_eq!(
        expected.all.total, 11,
        "seven births + two legacy + one lost link + escaped birth"
    );
    assert_eq!(
        expected.all.range,
        Some(("2026-09-01".into(), "2026-11-03".into()))
    );
    assert!(
        expected
            .all
            .gaps
            .iter()
            .any(|gap| gap.reason == Reason::Malformed && gap.count == 2)
    );
    let dir = root.live("small", f::S, &bytes);
    let actual = drain(&mut Scan::new(
        root.0.clone(),
        BTreeMap::from([("small".into(), dir)]),
        true,
        1,
        Some(f::S.into()),
    ));
    assert_eq!(
        actual, expected,
        "the whole pre-existing projection remains unchanged"
    );
    assert!(!format!("{actual:?}").contains(f::PRIVATE));
}

#[test]
fn worker_fact_divergence_is_not_erased_by_stream_projection() {
    let root = Scratch::new("307-fact-conflict");
    let birth = String::from_utf8(cert(1)).expect("fixture UTF8");
    let start = format!(",\"start\":\"spawned.0:{}:41\"", f::id(1));
    let attempt = f::fact(f::S, 1, "start-attempted", &start);
    let live_bytes = format!(
        "{birth}{attempt}{}",
        f::fact(f::S, 1, "start-failed", &start)
    );
    let archive_bytes = format!(
        "{birth}{attempt}{}",
        f::fact(
            f::S,
            1,
            "observed",
            &format!("{start},\"observed_source\":\"frame\",\"observed_model\":\"sol\"")
        )
    );
    let dir = root.live("fact", f::S, live_bytes.as_bytes());
    root.archived(f::S, archive_bytes.as_bytes());
    let view = drain(&mut Scan::new(
        root.0.clone(),
        BTreeMap::from([("fact".into(), dir)]),
        true,
        1,
        Some(f::S.into()),
    ));
    assert_eq!(view.all.total, 1);
    assert!(
        view.all
            .gaps
            .iter()
            .any(|gap| gap.reason == Reason::Conflict),
        "worker-fact divergence remains visible: {:?}",
        view.all.gaps
    );
    assert!(view.all.partial());
}

#[test]
fn scan_finishes_for_many_sources_and_explains_source_and_byte_bounds() {
    let root = Scratch::new("307-bounds");
    let mut live = BTreeMap::new();
    for n in 1..=7 {
        let name = format!("source-{n}");
        live.insert(name.clone(), root.live(&name, f::S, &cert(n)));
    }
    let mut healthy = Scan::new(root.0.clone(), live.clone(), true, 1, None);
    assert_eq!(finish_with_bound(&mut healthy, 8).all.total, 7);
    let mut source_limited = Scan::limited(
        root.0.clone(),
        live.clone(),
        true,
        1,
        None,
        Limits {
            sources: 3,
            ..LIMITS
        },
    );
    let limited = finish_with_bound(&mut source_limited, 4);
    assert_eq!(limited.all.total, 3);
    assert_eq!(
        (limited.all.sources_read, limited.all.sources_total),
        (3, 7)
    );
    assert_eq!(
        limited
            .all
            .gaps
            .iter()
            .filter(|gap| gap.reason == Reason::Source(SourceGap::Budget))
            .count(),
        4
    );
    let mut byte_limited = Scan::limited(
        root.0.clone(),
        live,
        true,
        1,
        None,
        Limits { bytes: 1, ..LIMITS },
    );
    let limited = finish_with_bound(&mut byte_limited, 8);
    assert_eq!(limited.all.total, 0);
    assert_eq!(
        (limited.all.sources_read, limited.all.sources_total),
        (0, 7)
    );
    gap(&limited, SourceGap::Budget);
    assert!(limited.all.partial());
}

#[test]
fn journal_above_32_mib_still_counts_when_scan_budget_has_room() {
    let root = Scratch::new("307-no-journal-cutoff");
    let mut bytes = cert(1);
    let ordinary = f::event("send", &format!(",\"summary\":\"{}\"", "x".repeat(4096)));
    while bytes.len() <= 33 * 1024 * 1024 {
        bytes.extend(ordinary.bytes());
    }
    bytes.extend(cert(2));
    let dir = root.live("large", f::S, &bytes);
    let mut scan = Scan::new(
        root.0.clone(),
        BTreeMap::from([("large".into(), dir)]),
        true,
        1,
        None,
    );
    let view = finish_with_bound(&mut scan, 12);
    assert_eq!(
        view.all.total, 2,
        "no independent journal-size cutoff inside scan budget"
    );
    assert_eq!((view.all.sources_read, view.all.sources_total), (1, 1));
    assert!(view.all.gaps.is_empty(), "{:?}", view.all.gaps);
}
