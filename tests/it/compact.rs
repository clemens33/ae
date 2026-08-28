//! `ae _compact-freeze <session-dir> [--keep-history]` — compact's freeze/resolve step
//! on the built binary, black-box. Pure read-only: it emits the frozen tuple or a
//! clear refusal, and mutates nothing.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "fixtures build real session dirs and config on disk; the capability boundary is about what PRODUCT code may reach"
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

/// A temp dir removed on drop, whose root doubles as `AE_HOME`.
struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae-compact-it-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        Self(dir)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const UUID: &str = "22222222-2222-2222-2222-222222222222";

fn freeze(ae_home: &Path, dir: &Path, keep_history: bool) -> std::process::Output {
    let mut cmd = crate::cli::ae();
    cmd.env("AE_HOME", ae_home);
    cmd.arg("_compact-freeze").arg(dir);
    if keep_history {
        cmd.arg("--keep-history");
    }
    crate::cli::bounded(
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn compact-freeze"),
        Duration::from_secs(10),
    )
    .expect("compact-freeze returned")
}

/// Build `<AE_HOME>/sessions/<name>` with a local meta and a `[workspace]` config.
/// `origin` is `AE_HOME` itself (a real, canonicalizable dir).
fn local_session(s: &Scratch, name: &str, mode: &str, config_body: &str) -> PathBuf {
    let sessions = s.0.join("sessions");
    let dir = sessions.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let config = s.0.join("config");
    std::fs::write(&config, config_body).unwrap();
    let meta = format!(
        "session_id={UUID}\nmode={mode}\norigin={}\nagent.main=cl:main:{UUID}\nconfig={}\n",
        s.0.display(),
        config.display()
    );
    std::fs::write(dir.join("meta"), meta).unwrap();
    dir
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_local_session_emits_the_frozen_tuple() {
    let s = Scratch::new("ok");
    let dir = local_session(
        &s,
        "sess",
        "local",
        "[workspace]\nmain = cl\nworkers = a, b\n",
    );
    let out = freeze(s.0.as_path(), &dir, false);
    assert_eq!(
        out.status.code(),
        Some(0),
        "freeze succeeds: {}",
        stderr(&out)
    );
    let line = stdout(&out);
    let fields: Vec<&str> = line.trim_end().split('\u{1f}').collect();
    assert_eq!(fields.len(), 10, "ten fields: {line:?}");
    assert_eq!(fields[0], "sess", "name");
    assert_eq!(fields[1], UUID, "uuid");
    assert_eq!(fields[3], "local", "mode");
    assert_eq!(fields[6], "false", "purge");
    assert_eq!(fields[8], "cl:main", "main_ref");
    assert_eq!(fields[9], "main=cl workers=a, b", "roster");
    // Read-only: the session dir and its meta are untouched.
    assert!(dir.join("meta").exists());
}

#[test]
fn a_fifo_global_config_is_refused_without_hanging() {
    // A recorded global config that is a FIFO must be refused by CLASSIFICATION, never
    // opened: an ungated `read_to_string` on a writerless FIFO blocks forever. The
    // bounded runner (10s) turns any such hang into a test failure, so a clean code-1
    // refusal is positive proof the open never happened.
    let s = Scratch::new("fifo");
    let dir = s.0.join("sessions").join("sess");
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = s.0.join("fifo-config");
    crate::cli::mkfifo(&cfg);
    let meta = format!(
        "session_id={UUID}\nmode=local\norigin={}\nagent.main=cl:main:{UUID}\nconfig={}\n",
        s.0.display(),
        cfg.display()
    );
    std::fs::write(dir.join("meta"), meta).unwrap();
    let out = freeze(s.0.as_path(), &dir, false);
    assert_eq!(out.status.code(), Some(1), "FIFO refused: {}", stderr(&out));
    assert!(
        stderr(&out).contains("not a readable regular file"),
        "clear refusal: {}",
        stderr(&out)
    );
    assert!(stdout(&out).is_empty(), "no tuple on refusal");
}

#[test]
fn a_managed_mode_is_refused_clearly() {
    let s = Scratch::new("git");
    let dir = local_session(&s, "sess", "git", "[workspace]\nmain = cl\n");
    let out = freeze(s.0.as_path(), &dir, false);
    assert_eq!(out.status.code(), Some(1), "managed mode refuses");
    assert!(
        stderr(&out).contains("local-mode only"),
        "clear refusal: {}",
        stderr(&out)
    );
    assert!(stdout(&out).is_empty(), "no tuple on refusal");
}
