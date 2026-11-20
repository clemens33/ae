//! Shared S5 acceptance fixture. Install as cfg(test) pub(crate) mod in `stats_store`.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned acceptance fixture IO"
)]
use super::disk::Limits;
use super::maint::{Maintainer, Outcome};
use crate::stats::{self, Source, View};
use crate::stats_fixture as f;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub(crate) const S: &str = f::S;
pub(crate) const T: &str = f::T;
pub(crate) struct Fixture(pub(crate) PathBuf);
impl Fixture {
    pub(crate) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-s5-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("exclusive fixture root");
        fs::create_dir(path.join("sessions")).expect("complete sessions namespace");
        fs::create_dir(path.join("archive")).expect("complete archive namespace");
        Self(path)
    }
    pub(crate) fn live(&self, name: &str, uuid: &str, bytes: &[u8]) -> PathBuf {
        let dir = self.0.join("sessions").join(name);
        fs::create_dir(&dir).expect("fixture live source");
        fs::write(
            dir.join("meta"),
            format!("session={name}\nsession_id={uuid}\n"),
        )
        .expect("meta");
        fs::write(dir.join("events.jsonl"), bytes).expect("journal");
        dir
    }
    pub(crate) fn maintainer(&self) -> Maintainer {
        Maintainer::new(
            &self.0,
            Limits {
                chunk: 4 << 20,
                line: 4 << 20,
            },
            Duration::from_millis(100),
        )
    }
    pub(crate) fn build(&self) {
        drain(&mut self.maintainer());
    }
    pub(crate) fn archive(&self, uuid: &str, bytes: &[u8]) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = self.0.join("archive").join(uuid);
        fs::create_dir_all(dir.join("messages")).expect("archive dirs");
        for node in [&self.0.join("archive"), &dir, &dir.join("messages")] {
            fs::set_permissions(node, fs::Permissions::from_mode(0o700))
                .expect("private archive directory");
        }
        let meta = format!(
            "archive_id={uuid}\narchive_version=1\nsource_session=old\nsource_session_id={uuid}\nhandover_count=0\nmemo_topic_count=0\npending_request_count=0\n"
        );
        let digest = format!(
            "- Archive ID: {uuid}\n## Handover (0)\n## Memo topics (0)\n## Unresolved requests (0)\n"
        );
        for (name, body) in [
            ("meta", meta.as_bytes()),
            ("digest.md", digest.as_bytes()),
            ("memo.tsv", b"".as_slice()),
            ("events.jsonl", bytes),
        ] {
            fs::write(dir.join(name), body).expect("archive member");
            fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o600))
                .expect("private archive member");
        }
        crate::archive::validated_journal(&dir, uuid, &mut |node| fs::read(node))
            .expect("calibrated valid archive");
        dir
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub(crate) fn birth(uuid: &str, n: u32) -> Vec<u8> {
    f::birth(
        uuid,
        n,
        &f::fields("codex", None, Some("sol"), Some("high")),
    )
    .into_bytes()
}
pub(crate) fn drain(upkeep: &mut Maintainer) {
    for _ in 0..512 {
        let turn = upkeep.step();
        assert!(
            !matches!(turn.outcome, Outcome::Refused(_)),
            "fixture upkeep refused: {turn:?}"
        );
        if turn.outcome == Outcome::Idle {
            return;
        }
    }
    panic!("finite fixture never caught up");
}
pub(crate) fn oracle(dir: &Path, name: &str, scope: Option<&str>) -> View {
    stats::fold(
        vec![Source::live(
            name,
            &fs::read(dir.join("meta")).expect("oracle meta"),
            Ok(fs::read(dir.join("events.jsonl")).expect("oracle journal")),
        )],
        scope,
    )
}
