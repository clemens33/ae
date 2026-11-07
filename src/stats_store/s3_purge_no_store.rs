//! Frozen no-store purge: unrelated state-root entries do not constitute a payload.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
use super::disk::{self, Purged};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-s3-purge-no-store-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("exclusive private no-store fixture");
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn s3_purge_absent_payload_ignores_other_names_without_creating_statistics_files() {
    let root = Root::new();
    fs::create_dir(root.0.join("sessions")).expect("live namespace");
    fs::create_dir(root.0.join("archive")).expect("archive namespace");
    for _ in 0..2 {
        assert_eq!(
            disk::purged(
                &root.0,
                "11111111-1111-4111-8111-111111111111",
                Duration::ZERO
            )
            .expect("nothing to update when no payload exists"),
            Purged::Absent
        );
        let mut names = fs::read_dir(&root.0)
            .expect("fixture entries")
            .map(|entry| entry.expect("directory entry").file_name())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            vec![
                std::ffi::OsString::from("archive"),
                std::ffi::OsString::from("sessions")
            ],
            "an absent statistics payload needs no stats.lock or materialization"
        );
    }
}
