//! Frozen real-door standing classification, independent of journal witnesses.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
#![allow(clippy::disallowed_methods, reason = "private real filesystem fixture")]
use super::disk::{FileJournals, Journal, Standing};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const S: &str = "11111111-1111-4111-8111-111111111111";

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-s3-standing-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("private fixture root");
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn s3_file_archive_standing_distinguishes_absence_claim_and_metadata_error() {
    let root = Root::new();
    let archive = root.0.join("archive");
    let kept = archive.join(S);
    let claim = archive.join(format!(".publishing.{S}"));
    let source = format!("archive/{S}");
    let mut journals = FileJournals::at(&root.0);

    assert_eq!(
        journals.standing(&source),
        Standing::Gone,
        "absent namespace"
    );
    fs::create_dir(&archive).expect("archive namespace");
    assert_eq!(journals.standing(&source), Standing::Gone, "absent archive");
    fs::create_dir(&kept).expect("archive directory");
    assert_eq!(journals.standing(&source), Standing::Present);
    fs::create_dir(&claim).expect("publisher or purge claim");
    assert_eq!(journals.standing(&source), Standing::Claimed);
    fs::remove_dir(&kept).expect("claimed directory removed");
    assert_eq!(journals.standing(&source), Standing::Claimed, "claim wins");
    fs::remove_dir(&claim).expect("claim cleared");
    assert_eq!(journals.standing(&source), Standing::Gone);
    fs::remove_dir(&archive).expect("empty namespace removed");
    fs::write(&archive, b"not a directory").expect("ENOTDIR fixture");
    assert_eq!(
        journals.standing(&source),
        Standing::Unknown,
        "a metadata error other than NotFound does not prove absence"
    );
}
