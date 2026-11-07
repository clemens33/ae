//! Frozen purge recovery: a retired payload is present even without stats/.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
#![allow(clippy::duplicate_mod, reason = "shared frozen fold oracle")]
#[path = "s3_cases.rs"]
mod cases;
use super::disk::{self, Dir, Files, Life, MemoryJournals, Op, Purged, Shown, SourceSpec};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-s3-purge-retired-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("private purge fixture");
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn no_archive_uuid(path: &Path) {
    for entry in fs::read_dir(path).expect("entire state root") {
        let entry = entry.expect("store entry");
        assert!(
            !entry.file_name().to_string_lossy().contains(cases::S),
            "purge leaves no archive UUID filename: {}",
            entry.path().display()
        );
        if entry.file_type().expect("entry type").is_dir() {
            no_archive_uuid(&entry.path());
        } else {
            let bytes = fs::read(entry.path()).expect("store bytes");
            assert!(
                !bytes
                    .windows(cases::S.len())
                    .any(|part| part == cases::S.as_bytes()),
                "purge leaves no archive UUID bytes: {}",
                entry.path().display()
            );
        }
    }
}

#[test]
fn s3_purge_recovers_a_retired_only_payload_before_removing_archive_counts() {
    let root = Root::new();
    let mut files = Files::at(&root.0);
    let mut journals = MemoryJournals::new();
    let mut archive = cases::Feed::live(
        "unused",
        Some(cases::S),
        [cases::birth(cases::S, 1), cases::birth(cases::T, 2)].concat(),
    );
    archive.spec = SourceSpec::Archive {
        id: format!("archive/{}", cases::S),
        uuid: cases::S.to_owned(),
    };
    let survivor = cases::Feed::live("live/survivor", Some(cases::T), cases::birth(cases::T, 3));
    let mut feeds = vec![archive, survivor];
    for (index, feed) in feeds.iter_mut().enumerate() {
        feed.ino = 30 + u64::try_from(index).expect("fixture index");
        journals.set(feed.id(), 1, feed.ino, &feed.bytes);
        let work = cases::work(&mut files, &mut journals, &feed.spec, &Life::default());
        let publication = cases::plan(
            disk::publish(&mut files, &mut journals, &work).expect("source publication"),
        );
        cases::complete(&mut files, &publication);
        feed.active = true;
        feed.cursor = u64::try_from(feed.bytes.len()).expect("fixture cursor");
    }
    cases::check(&mut files, &feeds);
    let Shown::Ready { epoch, .. } = disk::shown(&mut files, None) else {
        panic!("materialized epoch");
    };
    // A recoverable swap state: the only surviving payload is retired and
    // proves the epoch in its own directory name. No active/staged payload.
    let retired = format!("stats.old.{epoch}");
    files
        .apply(&Op::Rename {
            from: "stats".to_owned(),
            to: retired.clone(),
        })
        .expect("retire payload before purge");
    let listing = files.list("").expect("crash-state listing");
    assert!(listing.contains(&retired));
    assert!(
        !listing
            .iter()
            .any(|name| name == "stats" || name == "stats.new")
    );

    assert_eq!(
        disk::purged(&root.0, cases::S, Duration::from_millis(100))
            .expect("purge through swap recovery"),
        Purged::Removed,
        "a recoverable retired payload is not an absent store"
    );
    feeds[0].active = false;
    cases::check(&mut files, &feeds);
    no_archive_uuid(&root.0);
    let listing = files.list("").expect("post-purge listing");
    assert!(listing.iter().any(|name| name == "stats"));
    assert!(!listing.iter().any(|name| name.starts_with("stats.old.")));
    let later = disk::recovery(&mut files).expect("later writer recovery");
    cases::complete(&mut files, &later);
    assert!(
        later.ops.is_empty(),
        "no retired payload can later restore archive statistics"
    );
    cases::check(&mut files, &feeds);
    no_archive_uuid(&root.0);
}
