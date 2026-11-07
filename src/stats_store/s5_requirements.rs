//! Requirements retained when the per-open Scan tests disappear.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned acceptance fixture IO"
)]
use super::disk::{self, Files, Glance};
use super::s5_fixture::{Fixture, S, T, birth};
use crate::stats::{self, Reason, Source, SourceGap};
use crate::stats_fixture as f;
use std::fs;

fn published(root: &Fixture, this: Option<&str>) -> stats::View {
    match disk::glance(&mut Files::at(&root.0), this) {
        Glance::Ready { view, .. } => view,
        other => panic!("expected maintained answer: {other:?}"),
    }
}

#[test]
fn s5_small_full_view_keeps_noise_dates_malformed_utf8_and_escaped_actions() {
    let root = Fixture::new();
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
        f::birth(S, 20, &f::fields("codex", None, Some("sol"), Some("high")))
            .replace("worker-birth", "worker-\\u0062irth")
            .bytes(),
    );
    let meta = format!("session_id={S}\n");
    let expected = stats::fold(
        vec![Source::live("small", meta.as_bytes(), Ok(bytes.clone()))],
        Some(S),
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
    root.live("small", S, &bytes);
    root.build();
    let actual = published(&root, Some(S));
    assert_eq!(
        actual, expected,
        "the whole pre-existing projection remains unchanged"
    );
    assert!(!format!("{actual:?}").contains(f::PRIVATE));
}

#[test]
fn s5_archive_unterminated_escaped_final_birth_is_counted() {
    let root = Fixture::new();
    let bytes = f::birth(S, 20, &f::fields("codex", None, Some("sol"), Some("high")))
        .replace("worker-birth", "worker-\\u0062irth")
        .trim_end_matches('\n')
        .as_bytes()
        .to_vec();
    assert!(
        !bytes.ends_with(b"\n"),
        "finite archive fixture exercises final EOF"
    );
    let expected = stats::fold(vec![Source::archive(S, Ok(bytes.clone()))], Some(S));
    assert_eq!(expected.all.total, 1);
    root.archive(S, &bytes);
    root.build();
    assert_eq!(
        published(&root, Some(S)),
        expected,
        "a retained archive has a final EOF, unlike a live growing journal"
    );
}

#[test]
fn s5_archive_meta_and_digest_accept_exact_member_cap_and_refuse_one_byte_more() {
    const CAP: usize = 4 << 20;
    for member in ["meta", "digest.md"] {
        let root = Fixture::new();
        let bytes = birth(S, 1);
        let dir = root.archive(S, &bytes);
        let mut padded = fs::read(dir.join(member)).expect("valid member headers");
        padded.resize(CAP, b' ');
        fs::write(dir.join(member), &padded).expect("exact cap padding");
        crate::archive::validated_journal(&dir, S, &mut |node| fs::read(node))
            .expect("exact-cap tree remains valid independent of store");
        root.build();
        let expected = stats::fold(vec![Source::archive(S, Ok(bytes))], Some(S));
        assert_eq!(
            published(&root, Some(S)),
            expected,
            "{member}: exact cap remains readable"
        );
        padded.push(b' ');
        fs::write(dir.join(member), padded).expect("one byte over cap");
        root.build();
        let actual = published(&root, Some(S));
        assert_eq!(
            actual.all.total, 0,
            "{member}: refused validation cannot count its journal"
        );
        assert_eq!((actual.all.sources_read, actual.all.sources_total), (0, 1));
        assert!(
            actual
                .all
                .gaps
                .iter()
                .any(|gap| gap.source.as_deref() == Some(S)
                    && gap.reason == Reason::Source(SourceGap::Oversized))
        );
        assert!(actual.all.partial());
        assert!(actual.all.balanced());
    }
}

#[test]
fn s5_absent_and_empty_archive_namespaces_are_healthy_empty_history() {
    for present in [false, true] {
        let root = Fixture::new();
        if !present {
            fs::remove_dir(root.0.join("archive")).expect("absent archive namespace");
        }
        let dir = root.live("empty", S, b"");
        root.build();
        let actual = published(&root, None);
        assert_eq!(
            actual,
            stats::fold(
                vec![Source::live(
                    "empty",
                    &fs::read(dir.join("meta")).expect("oracle meta"),
                    Ok(Vec::new())
                )],
                None
            )
        );
        assert_eq!(actual.all.total, 0);
        assert!(!actual.all.partial());
        assert_eq!(
            root.0.join("archive").exists(),
            present,
            "upkeep does not create missing archive history"
        );
    }
}

#[test]
fn s5_file_or_symlink_archive_namespace_is_a_gap_and_never_contributes_target_history() {
    for linked in [false, true] {
        let root = Fixture::new();
        let target = Fixture::new();
        let archived = target.archive(T, &birth(T, 2));
        let archive_before = fs::read(archived.join("events.jsonl")).expect("target journal");
        let live = birth(S, 1);
        let dir = root.live("alpha", S, &live);
        let meta = fs::read(dir.join("meta")).expect("oracle meta");
        fs::remove_dir(root.0.join("archive")).expect("replace namespace");
        if linked {
            std::os::unix::fs::symlink(target.0.join("archive"), root.0.join("archive"))
                .expect("archive root link");
        } else {
            fs::write(root.0.join("archive"), b"not a directory").expect("archive root file");
        }
        root.build();
        let expected = stats::fold(
            vec![
                Source::live("alpha", &meta, Ok(live)),
                Source::failed("archive", None, SourceGap::Unreadable),
            ],
            Some(S),
        );
        let actual = published(&root, Some(S));
        assert_eq!(
            actual, expected,
            "invalid archive enumeration must stay visible in both scopes"
        );
        assert_eq!(actual.all.total, 1);
        assert!(actual.all.partial());
        assert!(actual.all.balanced());
        assert_eq!(
            fs::read(archived.join("events.jsonl")).expect("target unchanged"),
            archive_before
        );
        let kind = fs::symlink_metadata(root.0.join("archive"))
            .expect("namespace unchanged")
            .file_type();
        assert_eq!(kind.is_symlink(), linked);
        assert_eq!(kind.is_file(), !linked);
    }
}

#[test]
fn s5_journal_larger_than_thirty_two_mib_keeps_births_at_both_ends() {
    let root = Fixture::new();
    let mut bytes = birth(S, 1);
    for _ in 0..4097 {
        bytes.extend(vec![b' '; 8191]);
        bytes.push(b'\n');
    }
    bytes.extend(birth(S, 2));
    assert!(
        bytes.len() > 32 << 20,
        "fixture crosses the removed historical whole-source cap"
    );
    let dir = root.live("large", S, &bytes);
    let expected = stats::fold(
        vec![Source::live(
            "large",
            &fs::read(dir.join("meta")).expect("oracle meta"),
            Ok([birth(S, 1), birth(S, 2)].concat()),
        )],
        Some(S),
    );
    assert_eq!(expected.all.total, 2);
    root.build();
    assert_eq!(
        published(&root, Some(S)),
        expected,
        "all chunks contribute without a whole-journal size cap"
    );
}

#[test]
fn s5_missing_and_unreadable_live_journals_remain_named_gaps_beside_healthy_counts() {
    let root = Fixture::new();
    let bytes = birth(S, 1);
    let healthy = root.live("healthy", S, &bytes);
    let missing = root.live("missing", S, b"");
    let unreadable = root.live("unreadable", S, b"");
    fs::remove_file(missing.join("events.jsonl")).expect("missing journal");
    fs::remove_file(unreadable.join("events.jsonl")).expect("replace journal");
    fs::create_dir(unreadable.join("events.jsonl"))
        .expect("directory cannot be read as journal even as root");
    let expected = stats::fold(
        vec![
            Source::live(
                "healthy",
                &fs::read(healthy.join("meta")).expect("healthy meta"),
                Ok(bytes),
            ),
            Source::live(
                "missing",
                &fs::read(missing.join("meta")).expect("missing source meta"),
                Err(SourceGap::Missing),
            ),
            Source::live(
                "unreadable",
                &fs::read(unreadable.join("meta")).expect("unreadable source meta"),
                Err(SourceGap::Unreadable),
            ),
        ],
        Some(S),
    );
    root.build();
    assert_eq!(published(&root, Some(S)), expected);
    assert_eq!(expected.all.total, 1);
    assert_eq!(
        (expected.all.sources_read, expected.all.sources_total),
        (1, 3)
    );
}
