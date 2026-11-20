//! Frozen live-door standing and malformed source-id refusals.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
#![allow(clippy::disallowed_methods, reason = "private real filesystem fixture")]
use super::disk::{FileJournals, Journal, Standing};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-s3-standing-live-{}-{}",
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
fn s3_file_live_standing_refuses_path_shaped_ids_and_distinguishes_meta_states() {
    let root = Root::new();
    let sessions = root.0.join("sessions");
    let nested = sessions.join("a/b");
    fs::create_dir_all(&nested).expect("real nested fixture");
    fs::write(sessions.join("a/meta"), b"fixture").expect("live meta");
    fs::write(nested.join("meta"), b"fixture").expect("nested meta");
    let mut journals = FileJournals::at(&root.0);
    assert_eq!(journals.standing("live/a"), Standing::Present);
    for source in [
        "live/a/b",
        "live/a/../a",
        "live/",
        "live/.",
        "live/..",
        "archive/not-a-uuid",
        "alien/a",
    ] {
        assert_eq!(
            journals.standing(source),
            Standing::Unknown,
            "a malformed source id never derives standing from a joined path: {source}"
        );
    }
    fs::remove_file(sessions.join("a/meta")).expect("meta disappeared");
    assert_eq!(journals.standing("live/a"), Standing::Gone);
    fs::create_dir(sessions.join("a/meta")).expect("irregular meta node");
    assert_eq!(journals.standing("live/a"), Standing::Unknown);
}
