//! #287 slice2: real scratch files, bounded stepped scan, generation cancellation.
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use super::stats::{LIMITS, Limits, Scan};
use super::{Answer, Job, Reader, Request};
use crate::stats::{Reason, SourceGap, ThisSession, View};
use crate::stats_fixture as f;

static SERIAL: AtomicU64 = AtomicU64::new(0);
fn archive_content(session: &str) -> (String, String) {
    (
        format!(
            "archive_id={session}\narchive_version=1\nsource_session=old-name\nhandover_count=0\nmemo_topic_count=0\npending_request_count=0\n"
        ),
        format!(
            "- Archive ID: {session}\n## Handover (0)\n## Memo topics (0)\n## Unresolved requests (0)\n"
        ),
    )
}
struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "ae-stats-s2-{}-{tag}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("owned fresh fixture directory");
        Self(path)
    }
    fn live(&self, name: &str, session: &str, bytes: &[u8]) -> PathBuf {
        let dir = self.0.join("sessions").join(name);
        std::fs::create_dir_all(&dir).expect("live fixture dir");
        std::fs::write(dir.join("meta"), format!("session_id={session}\n")).expect("meta");
        std::fs::write(dir.join("events.jsonl"), bytes).expect("journal");
        dir
    }
    fn archived(&self, session: &str, bytes: &[u8]) -> PathBuf {
        let dir = self.0.join("archive").join(session);
        std::fs::create_dir_all(dir.join("messages")).expect("archive fixture dirs");
        for directory in [&dir, &dir.join("messages")] {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
                .expect("archive directory mode");
        }
        let (meta, digest) = archive_content(session);
        for (name, body) in [
            ("meta", meta.as_bytes()),
            ("digest.md", digest.as_bytes()),
            ("memo.tsv", b"".as_slice()),
            ("events.jsonl", bytes),
        ] {
            std::fs::write(dir.join(name), body).expect("archive member");
            std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o600))
                .expect("archive file mode");
        }
        dir
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cert(n: u32) -> Vec<u8> {
    f::birth(
        f::S,
        n,
        &f::fields("codex", None, Some("sol"), Some("high")),
    )
    .into_bytes()
}
fn drain(scan: &mut Scan) -> View {
    for _ in 0..2048 {
        if let Some(view) = scan.step() {
            return view;
        }
    }
    panic!("finite fixture scan must terminate");
}
fn gap(view: &View, reason: SourceGap) {
    assert!(
        view.all
            .gaps
            .iter()
            .any(|gap| gap.reason == Reason::Source(reason)),
        "missing {reason:?}: {:?}",
        view.all.gaps
    );
}
#[allow(
    clippy::disallowed_methods,
    reason = "test-only tree snapshot of owned scratch inputs proves Statistics writes nothing"
)]
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(base: &Path, path: &Path, rows: &mut Vec<(PathBuf, Vec<u8>)>) {
        let meta = std::fs::symlink_metadata(path).expect("snapshot node");
        if meta.is_dir() {
            rows.push((
                path.strip_prefix(base).expect("contained").to_owned(),
                b"directory".to_vec(),
            ));
            for entry in std::fs::read_dir(path).expect("snapshot listing") {
                walk(base, &entry.expect("snapshot entry").path(), rows);
            }
        } else {
            let bytes = if meta.file_type().is_symlink() {
                b"symlink".to_vec()
            } else {
                std::fs::read(path).expect("snapshot bytes")
            };
            rows.push((
                path.strip_prefix(base).expect("contained").to_owned(),
                bytes,
            ));
        }
    }
    let mut rows = Vec::new();
    walk(root, root, &mut rows);
    rows.sort();
    rows
}

#[test]
fn scan_is_lazy_and_each_step_reads_only_one_source() {
    let root = Scratch::new("stepped");
    let a = root.live("a", f::S, &cert(1));
    let b = root.live("b", f::S, &cert(2));
    let live = BTreeMap::from([("a".into(), a), ("b".into(), b.clone())]);
    let mut scan = Scan::new(root.0.clone(), live, true, 7, Some(f::S.into()));
    assert_eq!(scan.generation(), 7);
    assert_eq!(
        scan.remaining(),
        None,
        "constructor performs no discovery or journal read"
    );
    assert!(
        scan.step().is_none(),
        "first step discovers, no partial answer"
    );
    assert_eq!(scan.remaining(), Some(2));
    assert!(
        scan.step().is_none(),
        "first source is not the final answer"
    );
    assert_eq!(scan.remaining(), Some(1));
    let mut changed = cert(2);
    changed.extend(cert(3));
    std::fs::write(b.join("events.jsonl"), changed)
        .expect("concurrent committed spawn before second read");
    let view = drain(&mut scan);
    assert_eq!(
        view.all.total, 3,
        "second source was deferred, not read ahead"
    );
    assert!(view.all.balanced());
}

#[test]
fn journal_exactly_at_its_cap_is_readable_or_a_budget_gap_never_oversized() {
    let root = Scratch::new("exact-cap");
    let body = cert(1);
    let dir = root.live("edge", f::S, &body);
    let meta_size = u64::try_from(format!("session_id={}\n", f::S).len()).expect("small meta");
    let body_size = u64::try_from(body.len()).expect("small journal");
    let live = BTreeMap::from([("edge".to_owned(), dir)]);
    for shortage in [0, 1] {
        let mut scan = Scan::limited(
            root.0.clone(),
            live.clone(),
            true,
            1,
            Some(f::S.into()),
            Limits {
                bytes: meta_size + body_size - shortage,
                journal: body_size,
                ..LIMITS
            },
        );
        let view = drain(&mut scan);
        assert_eq!(view.all.sources_total, 1);
        if shortage == 0 {
            assert_eq!(view.all.total, 1, "exact reservation and cap both admit");
            assert_eq!(view.all.sources_read, 1);
            assert!(view.all.gaps.is_empty());
        } else {
            assert_eq!(view.all.total, 0, "budget refused the only certificate");
            assert_eq!(view.all.sources_read, 0);
            assert_eq!(
                view.all.gaps,
                vec![crate::stats::Gap {
                    source: Some("edge".to_owned()),
                    reason: Reason::Source(SourceGap::Budget),
                    count: 1,
                }],
                "a file at the cap is not oversized"
            );
        }
    }
}

#[test]
fn production_caps_and_each_small_limit_are_enforced() {
    assert_eq!(
        LIMITS,
        Limits {
            sources: 512,
            bytes: 256 * 1024 * 1024,
            journal: 4 * 1024 * 1024
        }
    );
    let root = Scratch::new("limits");
    let body = cert(1);
    let meta_size = u64::try_from(format!("session_id={}\n", f::S).len()).expect("small");
    let mut live = BTreeMap::new();
    for (name, n) in [("a", 1), ("b", 2), ("c", 3)] {
        live.insert(name.to_owned(), root.live(name, f::S, &cert(n)));
    }
    let mut normal = Scan::new(root.0.clone(), live.clone(), true, 1, None);
    assert_eq!(normal.limits(), LIMITS);
    assert_eq!(
        drain(&mut normal).all.total,
        3,
        "fixture is healthy without limits"
    );
    let mut sources = Scan::limited(
        root.0.clone(),
        live.clone(),
        true,
        1,
        None,
        Limits {
            sources: 2,
            ..LIMITS
        },
    );
    let capped = drain(&mut sources);
    assert_eq!(capped.all.total, 2);
    gap(&capped, SourceGap::Budget);
    let body_size = u64::try_from(body.len()).expect("small");
    let mut bytes = Scan::limited(
        root.0.clone(),
        live.clone(),
        true,
        1,
        None,
        Limits {
            bytes: meta_size + body_size,
            ..LIMITS
        },
    );
    let capped = drain(&mut bytes);
    assert_eq!(
        capped.all.total, 1,
        "budget is shared across source steps, meta included"
    );
    gap(&capped, SourceGap::Budget);
    let mut journal = Scan::limited(
        root.0.clone(),
        live,
        true,
        1,
        None,
        Limits {
            journal: body_size - 1,
            ..LIMITS
        },
    );
    let capped = drain(&mut journal);
    assert_eq!(capped.all.total, 0);
    gap(&capped, SourceGap::Oversized);
}

#[test]
fn kept_archives_overlap_live_then_survive_live_removal_and_disappear_on_purge() {
    let root = Scratch::new("kept");
    let live_dir = root.live("renamed", f::S, &cert(1));
    let archive = root.archived(f::S, &cert(1));
    let before = snapshot(&root.0);
    let live = BTreeMap::from([("renamed".to_owned(), live_dir.clone())]);
    let view = drain(&mut Scan::new(
        root.0.clone(),
        live,
        true,
        1,
        Some(f::S.into()),
    ));
    assert_eq!(
        view.all.total, 1,
        "live/archive overlap counts a modern birth once"
    );
    assert_eq!(
        snapshot(&root.0),
        before,
        "no store, lock, temp, cache, or source mutation"
    );
    std::fs::remove_dir_all(live_dir).expect("simulate kept end removing live registration");
    let kept = drain(&mut Scan::new(
        root.0.clone(),
        BTreeMap::new(),
        true,
        2,
        Some(f::S.into()),
    ));
    assert_eq!(kept.all.total, 1, "validated retained archive remains");
    std::fs::remove_dir_all(archive).expect("simulate authorized purge of owned fixture");
    let purged = drain(&mut Scan::new(
        root.0.clone(),
        BTreeMap::new(),
        true,
        3,
        Some(f::S.into()),
    ));
    assert_eq!(
        purged.all.total, 0,
        "new scans reflect retained sources, no hidden all-time store"
    );
}

#[test]
fn roots_never_mix_and_claimed_invalid_missing_unreadable_sources_are_gaps() {
    let a = Scratch::new("root-a");
    let b = Scratch::new("root-b");
    a.archived(f::S, &cert(1));
    b.archived(
        f::S,
        &format!(
            "{}{}",
            String::from_utf8(cert(2)).expect("UTF8"),
            String::from_utf8(cert(3)).expect("UTF8")
        )
        .into_bytes(),
    );
    let one = drain(&mut Scan::new(a.0.clone(), BTreeMap::new(), true, 1, None));
    let two = drain(&mut Scan::new(b.0.clone(), BTreeMap::new(), true, 1, None));
    assert_eq!((one.all.total, two.all.total), (1, 2));
    std::fs::create_dir(a.0.join("archive").join(format!(".publishing.{}", f::S)))
        .expect("own synthetic archive claim");
    let claimed = drain(&mut Scan::new(a.0.clone(), BTreeMap::new(), true, 2, None));
    assert_eq!(claimed.all.total, 0);
    gap(&claimed, SourceGap::Claimed);
    let bad = b.archived(f::T, &cert(4));
    std::fs::write(bad.join("unexpected"), b"invalid member").expect("invalid archive");
    let missing = b.live("missing", f::S, b"");
    std::fs::remove_file(missing.join("events.jsonl")).expect("missing journal");
    let unreadable = b.live("unreadable", f::S, b"");
    std::fs::remove_file(unreadable.join("events.jsonl")).expect("replace journal");
    std::fs::create_dir(unreadable.join("events.jsonl"))
        .expect("directory is deterministically not a regular file, even as root");
    let live = BTreeMap::from([
        ("missing".into(), missing),
        ("unreadable".into(), unreadable),
    ]);
    let view = drain(&mut Scan::new(b.0.clone(), live, true, 2, None));
    assert_eq!(view.all.total, 2);
    gap(&view, SourceGap::Invalid);
    gap(&view, SourceGap::Missing);
    gap(&view, SourceGap::Unreadable);
}

#[test]
fn loader_cancels_within_one_source_step_and_never_leaks_a_generation() {
    let root = Scratch::new("cancel");
    let a = root.live("a", f::S, &cert(1));
    let b = root.live("b", f::S, &cert(2));
    let mut reader = Reader::new(root.0.clone(), None, None);
    reader.fleet_at = Some(Instant::now());
    reader.scanned = true;
    reader.dirs = BTreeMap::from([("a".into(), a), ("b".into(), b)]);
    reader.ids.insert("a".into(), f::S.into());
    reader.selected = Some("a".into());
    reader.take(Request::Stats { generation: 1 });
    assert!(reader.stats.is_none(), "closed overlay cannot start a scan");
    reader.take(Request::Settings {
        open: true,
        generation: 1,
    });
    reader.take(Request::Stats { generation: 1 });
    assert!(reader.stats.is_some());
    assert!(
        reader.stats_step().is_none(),
        "listing is no partial answer"
    );
    assert!(
        reader.stats_step().is_none(),
        "first source leaves work pending"
    );
    reader.take(Request::Settings {
        open: false,
        generation: 1,
    });
    assert!(
        reader.stats.is_none(),
        "close discards pending sources at the next job boundary"
    );
    assert!(!matches!(reader.next(Instant::now()), Job::Stats));
    reader.take(Request::Settings {
        open: true,
        generation: 2,
    });
    reader.take(Request::Stats { generation: 1 });
    assert!(reader.stats.is_none(), "stale request cannot start work");
    reader.take(Request::Stats { generation: 2 });
    let answer = (0..20)
        .find_map(|_| reader.stats_step())
        .expect("new generation scan completes");
    let Answer::Stats { generation, view } = answer else {
        panic!("Stats answer")
    };
    assert_eq!(generation, 2);
    assert_eq!(view.all.total, 2);
    assert!(matches!(view.this, ThisSession::Ready(_)));
    let now = Instant::now();
    reader.settings = Some((
        2,
        Some(
            now.checked_sub(super::super::REFRESH)
                .expect("fixture clock exceeds refresh interval"),
        ),
    ));
    assert!(reader.stats.is_none(), "completed scan removed");
    assert!(
        matches!(reader.next(now), Job::Settings),
        "quota refresh is due"
    );
    assert!(
        !matches!(reader.next(now), Job::Stats),
        "quota cadence never restarts stats"
    );
}

#[test]
fn a_pending_stats_scan_yields_to_existing_view_look_and_fleet_work() {
    let root = Scratch::new("priority");
    let dir = root.live("a", f::S, &cert(1));
    let now = Instant::now();
    let mut reader = Reader::new(root.0.clone(), None, None);
    reader.scanned = true;
    reader.fleet_at = Some(now);
    reader.dirs.insert("a".into(), dir);
    reader.take(Request::Settings {
        open: true,
        generation: 1,
    });
    reader.settings = Some((1, Some(now)));
    reader.take(Request::Stats { generation: 1 });
    assert!(
        matches!(reader.next(now), Job::Stats),
        "pending scan is runnable"
    );
    reader.looks.insert("a".into());
    assert!(matches!(reader.next(now),Job::Look(name) if name == "a"));
    reader.looks.clear();
    reader.rereads.insert("a".into());
    assert!(matches!(reader.next(now),Job::View(name) if name == "a"));
    reader.rereads.clear();
    reader.settings = None;
    assert!(
        matches!(reader.next(now + super::super::REFRESH), Job::Fleet),
        "statistics never starves a due fleet read"
    );
}

#[test]
fn archive_meta_and_digest_each_obey_the_content_cap_before_the_journal_counts() {
    for member in ["meta", "digest.md"] {
        let root = Scratch::new("archive-content-cap");
        let dir = root.archived(f::S, &cert(1));
        let (meta, digest) = archive_content(f::S);
        let mut padded = if member == "meta" { meta } else { digest };
        padded.push_str(&" ".repeat(1024 - padded.len()));
        assert_eq!(
            padded.len(),
            1024,
            "valid matching headers plus inert padding"
        );
        std::fs::write(dir.join(member), padded).expect("own fixture member, mode preserved");
        let normal = drain(&mut Scan::new(
            root.0.clone(),
            BTreeMap::new(),
            true,
            1,
            None,
        ));
        assert_eq!(
            normal.all.total, 1,
            "{member}: padded archive is otherwise valid"
        );
        assert!(normal.all.gaps.is_empty(), "{member}: {normal:?}");
        let capped = drain(&mut Scan::limited(
            root.0.clone(),
            BTreeMap::new(),
            true,
            2,
            None,
            Limits {
                journal: 512,
                ..LIMITS
            },
        ));
        assert_eq!(
            capped.all.total, 0,
            "{member}: refused validation cannot count its journal"
        );
        assert_eq!((capped.all.sources_read, capped.all.sources_total), (0, 1));
        assert!(
            capped
                .all
                .gaps
                .iter()
                .any(|gap| gap.source.as_deref() == Some(f::S)
                    && gap.reason == Reason::Source(SourceGap::Oversized))
        );
        assert!(capped.all.partial());
        assert!(capped.all.balanced());
    }
}

#[test]
fn archive_validation_meta_digest_and_journal_draw_on_one_byte_budget() {
    let root = Scratch::new("archive-content-budget");
    let journal = cert(1);
    root.archived(f::S, &journal);
    let (meta, digest) = archive_content(f::S);
    let meta_size = u64::try_from(meta.len()).expect("small fixture");
    let digest_size = u64::try_from(digest.len()).expect("small fixture");
    let journal_size = u64::try_from(journal.len()).expect("small fixture");
    let entire = meta_size + digest_size + journal_size;
    for (bytes, total) in [
        (meta_size - 1, 0),
        (meta_size, 0),
        (meta_size + digest_size, 0),
        (entire - 1, 0),
        (entire, 1),
    ] {
        let view = drain(&mut Scan::limited(
            root.0.clone(),
            BTreeMap::new(),
            true,
            1,
            None,
            Limits { bytes, ..LIMITS },
        ));
        assert_eq!(view.all.total, total, "budget {bytes} of {entire}");
        assert_eq!((view.all.sources_read, view.all.sources_total), (total, 1));
        if total == 0 {
            assert!(
                view.all
                    .gaps
                    .iter()
                    .any(|gap| gap.source.as_deref() == Some(f::S)
                        && gap.reason == Reason::Source(SourceGap::Budget)),
                "{view:?}"
            );
            assert!(view.all.partial());
        } else {
            assert!(
                view.all.gaps.is_empty(),
                "exact byte reserve is accepted: {view:?}"
            );
            assert!(!view.all.partial());
        }
        assert!(view.all.balanced());
    }
}

#[test]
fn archive_validation_content_charges_survive_the_next_source_step() {
    let root = Scratch::new("archive-shared-budget");
    let first = cert(1);
    let second = f::birth(
        f::T,
        2,
        &f::fields("codex", None, Some("sol"), Some("high")),
    );
    root.archived(f::S, &first);
    root.archived(f::T, second.as_bytes());
    let (meta_s, digest_s) = archive_content(f::S);
    let (meta_t, digest_t) = archive_content(f::T);
    let entire = u64::try_from(
        meta_s.len() + digest_s.len() + first.len() + meta_t.len() + digest_t.len() + second.len(),
    )
    .expect("small fixture");
    for (bytes, total) in [(entire - 1, 1), (entire, 2)] {
        let view = drain(&mut Scan::limited(
            root.0.clone(),
            BTreeMap::new(),
            true,
            1,
            None,
            Limits { bytes, ..LIMITS },
        ));
        assert_eq!(view.all.total, total, "byte budget spans archive steps");
        assert_eq!((view.all.sources_read, view.all.sources_total), (total, 2));
        if total == 1 {
            assert!(
                view.all
                    .gaps
                    .iter()
                    .any(|gap| gap.source.as_deref() == Some(f::T)
                        && gap.reason == Reason::Source(SourceGap::Budget)),
                "second source refused: {view:?}"
            );
        } else {
            assert!(view.all.gaps.is_empty(), "{view:?}");
        }
        assert!(view.all.balanced());
    }
}

#[test]
fn absent_archive_root_and_real_empty_archive_directory_are_healthy_empty() {
    for exists in [false, true] {
        let root = Scratch::new("archive-root-empty");
        if exists {
            std::fs::create_dir(root.0.join("archive")).expect("own empty archive root");
        }
        let before = snapshot(&root.0);
        let view = drain(&mut Scan::new(
            root.0.clone(),
            BTreeMap::new(),
            true,
            1,
            None,
        ));
        assert_eq!(view.all.total, 0);
        assert_eq!((view.all.sources_read, view.all.sources_total), (0, 0));
        assert!(view.all.gaps.is_empty(), "proved empty history: {view:?}");
        assert!(!view.all.partial());
        assert!(view.all.balanced());
        assert_eq!(snapshot(&root.0), before, "no store or root is created");
    }
}

#[test]
fn regular_file_archive_root_is_unreadable_history_not_healthy_empty() {
    let root = Scratch::new("archive-root-file");
    let live = root.live("live", f::S, &cert(1));
    std::fs::write(root.0.join("archive"), b"this is not an archive directory")
        .expect("own regular-file root fixture");
    let before = snapshot(&root.0);
    let view = drain(&mut Scan::new(
        root.0.clone(),
        BTreeMap::from([("live".into(), live)]),
        true,
        1,
        Some(f::S.into()),
    ));
    assert_eq!(view.all.total, 1, "live history still counts");
    assert_eq!((view.all.sources_read, view.all.sources_total), (1, 2));
    assert_eq!(view.all.gaps.len(), 1);
    let refusal = &view.all.gaps[0];
    assert_eq!(refusal.source.as_deref(), Some("archive"));
    assert_eq!(refusal.reason, Reason::Source(SourceGap::Unreadable));
    assert_eq!(refusal.count, 1);
    assert_eq!(refusal.line(), "archive: unreadable");
    assert!(view.all.partial());
    assert!(view.all.balanced());
    let ThisSession::Ready(current) = &view.this else {
        panic!("live session UUID is known")
    };
    assert_eq!(current.total, 1);
    assert!(
        current.partial(),
        "unattributed missing history affects either scope"
    );
    assert_eq!(snapshot(&root.0), before, "invalid root is never repaired");
}

#[test]
fn symlink_archive_root_is_refused_even_when_target_has_valid_history() {
    let root = Scratch::new("archive-root-link");
    let target = Scratch::new("archive-root-link-target");
    target.archived(f::S, &cert(1));
    let readable = drain(&mut Scan::new(
        target.0.clone(),
        BTreeMap::new(),
        true,
        1,
        None,
    ));
    assert_eq!(
        readable.all.total, 1,
        "target is independently valid retained history"
    );
    assert!(readable.all.gaps.is_empty());
    let live = root.live("live", f::S, &cert(2));
    std::os::unix::fs::symlink(target.0.join("archive"), root.0.join("archive"))
        .expect("own archive-root link");
    let before = snapshot(&root.0);
    let target_before = snapshot(&target.0);
    let view = drain(&mut Scan::new(
        root.0.clone(),
        BTreeMap::from([("live".into(), live)]),
        true,
        2,
        None,
    ));
    assert_eq!(
        view.all.total, 1,
        "link target must not contribute its birth"
    );
    assert_eq!((view.all.sources_read, view.all.sources_total), (1, 2));
    assert_eq!(view.all.gaps.len(), 1);
    let refusal = &view.all.gaps[0];
    assert_eq!(refusal.source.as_deref(), Some("archive"));
    assert_eq!(refusal.reason, Reason::Source(SourceGap::Unreadable));
    assert_eq!(refusal.count, 1);
    assert_eq!(refusal.line(), "archive: unreadable");
    assert!(view.all.partial());
    assert!(view.all.balanced());
    assert_eq!(snapshot(&root.0), before, "link is never replaced");
    assert_eq!(snapshot(&target.0), target_before, "target bytes unchanged");
}
