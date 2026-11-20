//! Baseline-compatible S5 acceptance. Install as a child of `app::loader`.
//! Oracle: the published store is authoritative for the tab; journal changes await upkeep.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned acceptance fixture IO"
)]

use super::{Reader, Wake};
use crate::app::{App, take_keys};
use crate::console::input;
use crate::stats::{self, Source};
use crate::stats_fixture as f;
use crate::stats_store::disk::{self, Files, Limits, Shown};
use crate::stats_store::maint::{Maintainer, Outcome};
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-s5-red-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("exclusive fixture root");
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn birth(n: u32) -> Vec<u8> {
    f::birth(
        f::S,
        n,
        &f::fields("codex", None, Some("sol"), Some("high")),
    )
    .into_bytes()
}
fn frame(app: &mut App) -> String {
    let mut buf = Buffer::empty(Rect::new(0, 0, 180, 60));
    app.frame(&mut buf);
    (0..60)
        .map(|y| (0..180).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
fn key(app: &mut App, text: &str) {
    let _ = take_keys(
        app,
        vec![(input::Key::Text(text.as_bytes().to_vec()), Instant::now())],
    );
}

#[test]
fn s5_red_real_loader_uses_published_counts_despite_readable_opposed_journal() {
    let root = Fixture::new();
    let dir = root.0.join("sessions/alpha");
    fs::create_dir_all(&dir).expect("fixture live source");
    fs::create_dir(root.0.join("archive")).expect("complete archive namespace");
    let meta = format!("session=alpha\nsession_id={}\n", f::S);
    let original = birth(1);
    fs::write(dir.join("meta"), &meta).expect("meta");
    fs::write(dir.join("events.jsonl"), &original).expect("journal");
    let oracle = stats::fold(
        vec![Source::live("alpha", meta.as_bytes(), Ok(original.clone()))],
        None,
    );
    assert_eq!(oracle.all.total, 1, "independent known fixture");
    let mut upkeep = Maintainer::new(
        &root.0,
        Limits {
            chunk: 4 << 20,
            line: 4 << 20,
        },
        Duration::from_millis(100),
    );
    let mut idle = false;
    for _ in 0..128 {
        let step = upkeep.step();
        assert!(!matches!(step.outcome, Outcome::Refused(_)), "{step:?}");
        if step.outcome == Outcome::Idle {
            idle = true;
            break;
        }
    }
    assert!(idle, "fixture must finish building before the subject runs");
    drop(upkeep);
    let Shown::Ready { view, pending, .. } = disk::shown(&mut Files::at(&root.0), None) else {
        panic!("published fixture");
    };
    assert_eq!(pending, 0);
    assert_eq!(
        view, oracle,
        "publish calibrated against the independent fold"
    );
    let mut opposed = original;
    opposed.extend(birth(2));
    assert_eq!(
        stats::fold(
            vec![Source::live("alpha", meta.as_bytes(), Ok(opposed.clone()))],
            None
        )
        .all
        .total,
        2
    );
    fs::write(dir.join("events.jsonl"), opposed)
        .expect("readable opposed value, no upkeep follows");

    let mut reader = Reader::new(root.0.clone(), None, None);
    reader.fleet_at = Some(Instant::now());
    reader.dirs.insert("alpha".to_owned(), dir);
    reader.ids.insert("alpha".to_owned(), f::S.to_owned());
    let (wake, wakes) = mpsc::channel();
    let (ask, handle) = super::spawn(reader, wake).expect("reader thread");
    let mut app = App::new(None, None, None, Some(ask));
    app.fleeted = true;
    app.open_settings();
    key(&mut app, "7");
    let mut rendered = frame(&mut app);
    let end = Instant::now() + Duration::from_secs(3);
    while !rendered.contains("Recorded worker spawns:") && Instant::now() < end {
        if let Ok(Wake::Answer(answer)) = wakes.recv_timeout(Duration::from_millis(20)) {
            app.answer(*answer);
        }
        rendered = frame(&mut app);
    }
    drop(app);
    handle
        .join()
        .expect("reader exits after its sender is dropped");
    assert!(
        rendered.contains("Recorded worker spawns: 1"),
        "tab must show the last published count, not the readable opposed journal:\n{rendered}"
    );
    assert!(
        !rendered.contains("Recorded worker spawns: 2"),
        "tab must not perform its old per-open fold"
    );
}
