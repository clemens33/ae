//! S6b acceptance: #230, plan-r2 S6b, and lead rulings R1-R4 in plan-s6b.md.
//! The app's modal is the human confirmation inside/outside tmux; no client
//! is required. A confirmed End owns one literal target, proves the frozen
//! identity under the lifecycle lock before any target write, and archives
//! before removing state. R1 retains stop -> archive -> removal: publication
//! failure preserves stopped, resumable state; preflight failure keeps it live.
//!
//! The guarded argv is the pair's agreed serialization (plan-s6b.md), not a
//! second product API: `_end --supervise <literal-name> --confirmed-*=...`
//! with all six `--expect-*=...` words. No force flag or real session is used.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "acceptance fixtures inspect only owned scratch state through existing process doors"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::cli::OwnedScratch;
use super::parity::Invocation;
use super::parity::capture::ExitOutcome;
use super::parity::capture::raw::{kill_servers_under, run as run_raw};

const UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const OTHER: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";
const ARCHIVE_ID: &str = UUID;
static CALL: AtomicU64 = AtomicU64::new(0);

struct Rig {
    root: PathBuf,
    state: PathBuf,
    sock: PathBuf,
    config: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let root = OwnedScratch::root("s6b", tag).keep();
        let state = root.join("ae");
        fs::create_dir_all(state.join("sessions")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        let config = root.join("config");
        fs::write(
            &config,
            concat!(
                "[profiles]\ncl = \"sh\"\n[roster]\nlead = cl\n",
                "[workspace]\nmain = lead\nchat = off\nicons = on\npalette = darcula\n",
            ),
        )
        .unwrap();
        let rig = Self {
            state,
            sock: root.join("ae.sock"),
            config,
            root,
        };
        rig.must_tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "host",
            "-x",
            "160",
            "-y",
            "50",
            "sh",
            "-c",
            "while :; do sleep 1; done",
        ]);
        rig
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.state.join("sessions").join(name)
    }

    fn archive(&self) -> PathBuf {
        self.state.join("archive").join(ARCHIVE_ID)
    }

    fn command(
        &self,
        program: &str,
        args: &[String],
        extra: &[(&str, String)],
    ) -> (i32, String, String) {
        let id = CALL.fetch_add(1, Ordering::Relaxed);
        let out = self.root.join(format!("call-{id}.out"));
        let err = self.root.join(format!("call-{id}.err"));
        // The existing raw door registers every real tmux start for crash
        // cleanup and strips both inherited TMUX markers itself.
        let invocation = if program == "tmux" {
            Invocation::new("tmux")
        } else {
            Invocation::new("env")
                .arg("-u")
                .arg("TMUX")
                .arg("-u")
                .arg("TMUX_PANE")
                .arg(program)
        };
        let mut invocation = invocation
            .env("HOME", self.root.join("home"))
            .env("AE_HOME", &self.state)
            .env("CONFIG_FILE", &self.config)
            .env("TMUX_TMPDIR", &self.root)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.sock)
            .env("AE_NO_AUTOSTART", "1");
        for arg in args {
            invocation = invocation.arg(arg);
        }
        for (key, value) in extra {
            invocation = invocation.env(key, value);
        }
        let status = run_raw(&invocation, &self.root, &out, &err).unwrap();
        let code = match status.outcome() {
            ExitOutcome::Code(code) => code,
            ExitOutcome::Signalled => -1,
        };
        let stdout = fs::read_to_string(&out).unwrap_or_default();
        let stderr = fs::read_to_string(&err).unwrap_or_default();
        let _ = fs::remove_file(out);
        let _ = fs::remove_file(err);
        (code, stdout, stderr)
    }

    fn tmux(&self, args: &[&str]) -> (i32, String, String) {
        let mut words = vec![
            "-u".to_owned(),
            "-S".to_owned(),
            self.sock.display().to_string(),
        ];
        words.extend(args.iter().map(|word| (*word).to_owned()));
        self.command("tmux", &words, &[])
    }

    fn must_tmux(&self, args: &[&str]) -> String {
        let (code, out, err) = self.tmux(args);
        assert_eq!(code, 0, "tmux {args:?}: {err}");
        out.trim().to_owned()
    }

    fn plant(&self, name: &str, uuid: &str, live: bool) {
        let dir = self.dir(name);
        fs::create_dir_all(&dir).unwrap();
        let mut meta = format!(
            "session_id={uuid}\nmode=local\nwork_dir={}\nseat.main=lead\nprofile.main=cl\n\
             agent_bin.main=sh\ntmux_server_kind=socket\ntmux_server={}\n",
            self.root.display(),
            self.sock.display(),
        );
        if live {
            let pane = self.must_tmux(&[
                "new-session",
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                "-s",
                name,
                "-n",
                "main",
                "sh",
                "-c",
                "while :; do sleep 1; done",
            ]);
            self.must_tmux(&["set-option", "-t", &format!("={name}:"), "@ae_session", "1"]);
            self.must_tmux(&[
                "set-option",
                "-t",
                &format!("={name}:"),
                "@ae_session_uuid",
                uuid,
            ]);
            self.must_tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", "lead"]);
            self.must_tmux(&["set-option", "-p", "-t", &pane, "@ae_slot", "main"]);
            self.must_tmux(&[
                "set-option",
                "-t",
                &format!("={name}:"),
                "@ae_picker_agents",
                &format!("v2;4102444800;60;lead:cl:working:{pane}:claude:Opus 4.7:high:"),
            ]);
            let pid = self.must_tmux(&["display-message", "-p", "-t", &pane, "#{pane_pid}"]);
            meta.push_str(&format!("pane.main={pane}\npid.main={pid}\n"));
        }
        fs::write(dir.join("meta"), meta).unwrap();
        fs::write(
            dir.join("events.jsonl"),
            concat!(
                "{\"ts\":\"2026-01-01T00:00:00Z\",\"actor\":\"lead\",\"action\":\"goal\",",
                "\"target\":\"\",\"summary\":\"S6B_HISTORY_SENTINEL\"}\n",
            ),
        )
        .unwrap();
        fs::write(
            dir.join("memo.tsv"),
            "2026-01-01T00:00:00Z\tlead\tkeep\tS6B_MEMO_SENTINEL\n",
        )
        .unwrap();
    }

    fn live(&self, name: &str) -> bool {
        let (code, _, _) = self.tmux(&["has-session", "-t", &format!("={name}")]);
        code == 0
    }

    fn app(&self, name: &str, inside: bool) -> String {
        self.app_boot(name, inside, None)
    }

    fn app_boot(&self, name: &str, inside: bool, boot: Option<i64>) -> String {
        self.app_at(name, inside, boot, "host")
    }

    fn app_at(&self, name: &str, inside: bool, boot: Option<i64>, home: &str) -> String {
        let prelude = if inside { "" } else { "unset TMUX TMUX_PANE; " };
        let boot = boot.map_or_else(String::new, |at| format!("AE_TEST_BOOT_TIME={at} "));
        let command = format!(
            "{prelude}HOME='{}' AE_HOME='{}' CONFIG_FILE='{}' TMUX_TMPDIR='{}' \
             AE_TMUX_SERVER_KIND=socket AE_TMUX_SERVER='{}' AE_NO_AUTOSTART=1 {boot}\
             exec '{}' app '{}'",
            self.root.join("home").display(),
            self.state.display(),
            self.config.display(),
            self.root.display(),
            self.sock.display(),
            env!("CARGO_BIN_EXE_ae"),
            name,
        );
        let pane = self.must_tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &format!("={home}"),
            "-n",
            "app",
            "sh",
            "-c",
            &command,
        ]);
        self.wait(&pane, "app loaded", |screen| {
            screen.contains("Enter writes")
        });
        assert!(
            self.must_tmux(&["list-clients", "-F", "#{client_name}"])
                .is_empty(),
            "the proof requires zero attached tmux clients"
        );
        pane
    }

    fn capture(&self, pane: &str) -> String {
        self.must_tmux(&["capture-pane", "-p", "-t", pane])
    }

    fn key(&self, pane: &str, key: &str) {
        thread::sleep(Duration::from_millis(30));
        self.must_tmux(&["send-keys", "-t", pane, key]);
    }

    fn wait(&self, pane: &str, why: &str, mut predicate: impl FnMut(&str) -> bool) -> String {
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            let screen = self.capture(pane);
            if predicate(&screen) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}:\n{screen}");
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn question(&self, pane: &str, name: &str) -> String {
        self.key(pane, "m");
        self.wait(pane, "End offered in session menu", |screen| {
            screen.contains("End…")
        });
        self.key(pane, "e");
        self.wait(pane, "End confirmation shown", |screen| {
            screen.contains("[ Cancel ]") && screen.contains("[ End ]") && screen.contains(name)
        })
    }

    fn run_end(&self, words: &[String], extra: &[(&str, String)]) -> (i32, String, String) {
        self.command(env!("CARGO_BIN_EXE_ae"), words, extra)
    }

    fn frozen(&self, name: &str, live: bool) -> Vec<String> {
        let (tmux, pid, start) = if live {
            let tmux = self.must_tmux(&[
                "display-message",
                "-p",
                "-t",
                &format!("={name}:"),
                "#{session_id}|#{session_created}",
            ]);
            let identity = self.must_tmux(&["display-message", "-p", "#{pid}|#{start_time}"]);
            let (pid, start) = identity.split_once('|').expect("server identity");
            (tmux, pid.to_owned(), start.to_owned())
        } else {
            ("absent".to_owned(), String::new(), String::new())
        };
        vec![
            "_end".into(),
            "--supervise".into(),
            name.into(),
            format!("--confirmed-target={name}"),
            "--confirmed-action=keep".into(),
            format!("--confirmed-detail={}", self.archive().display()),
            "--confirmed-purge=off".into(),
            "--confirmed-source=explicit".into(),
            format!("--confirmed-tmux={tmux}"),
            "--keep-history".into(),
            format!("--expect-uuid={UUID}"),
            "--expect-server-kind=socket".into(),
            format!("--expect-server={}", self.sock.display()),
            format!("--expect-server-pid={pid}"),
            format!("--expect-server-start={start}"),
            format!("--expect-deadline={}", epoch() + 60),
        ]
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = kill_servers_under(&self.root, std::slice::from_ref(&self.sock));
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn epoch() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}

fn change(words: &mut [String], flag: &str, value: &str) {
    let prefix = format!("{flag}=");
    let word = words
        .iter_mut()
        .find(|word| word.starts_with(&prefix))
        .expect("flag present");
    *word = format!("{prefix}{value}");
}

fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, map: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                map.insert(path.strip_prefix(base).unwrap().to_owned(), b"DIR".to_vec());
                walk(base, &path, map);
            } else if kind.is_symlink() {
                map.insert(
                    path.strip_prefix(base).unwrap().to_owned(),
                    fs::read_link(&path)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                );
            } else {
                map.insert(
                    path.strip_prefix(base).unwrap().to_owned(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut map = BTreeMap::new();
    walk(dir, dir, &mut map);
    map
}

fn assert_archive(rig: &Rig) {
    let archive = rig.archive();
    assert!(
        archive.is_dir(),
        "successful End must publish the session's archive"
    );
    let bytes = fs::read(archive.join("events.jsonl")).expect("archived events");
    assert!(String::from_utf8_lossy(&bytes).contains("S6B_HISTORY_SENTINEL"));
    let memos = fs::read(archive.join("memo.tsv")).expect("archived memo");
    assert!(String::from_utf8_lossy(&memos).contains("S6B_MEMO_SENTINEL"));
}

fn successful_app_end(inside: bool) {
    let rig = Rig::new(if inside { "inside" } else { "outside" });
    rig.plant("alpha", UUID, true);
    rig.plant("sibling", OTHER, true);
    let sibling = snapshot(&rig.dir("sibling"));
    let pane = rig.app("alpha", inside);
    let card_row = rig
        .capture(&pane)
        .lines()
        .position(|line| {
            line.chars()
                .take(35)
                .collect::<String>()
                .split_whitespace()
                .any(|word| word == "alpha")
        })
        .expect("the selected card was painted");
    let question = rig.question(&pane, "alpha");
    assert!(
        question.to_lowercase().contains("archive") && !question.contains("NO archive"),
        "question must explain preservation: {question}"
    );
    assert!(rig.live("alpha"));
    assert!(
        !rig.archive().exists(),
        "opening a question must not archive or end"
    );
    rig.key(&pane, "y");
    rig.wait(&pane, "End result visible", |screen| {
        screen.contains("Ended alpha") && !rig.dir("alpha").exists()
    });
    assert!(!rig.live("alpha"));
    assert_archive(&rig);
    assert!(rig.live("sibling"));
    assert_eq!(snapshot(&rig.dir("sibling")), sibling);
    let screen = rig.capture(&pane);
    assert!(
        !screen
            .lines()
            .nth(card_row)
            .unwrap_or_default()
            .chars()
            .take(35)
            .collect::<String>()
            .split_whitespace()
            .any(|word| word == "alpha"),
        "ended card remains: {screen}"
    );
}

#[test]
fn app_end_inside_tmux_needs_no_attached_client() {
    successful_app_end(true);
}

#[test]
fn app_end_outside_tmux_needs_no_attached_client() {
    successful_app_end(false);
}

#[test]
fn end_confirmation_cancels_by_default_and_by_escape_without_writes() {
    let rig = Rig::new("cancel");
    rig.plant("alpha", UUID, true);
    let pane = rig.app("alpha", false);
    for key in ["Enter", "Escape", "q", "n"] {
        rig.question(&pane, "alpha");
        let before = snapshot(&rig.dir("alpha"));
        rig.key(&pane, key);
        rig.wait(&pane, "cancel returns to app", |s| !s.contains("[ End ]"));
        assert_eq!(
            snapshot(&rig.dir("alpha")),
            before,
            "cancel {key} wrote state"
        );
        assert!(rig.live("alpha"));
        assert!(!rig.archive().exists());
    }
}

#[test]
fn stopped_card_can_be_ended_from_the_app() {
    let rig = Rig::new("stopped");
    rig.plant("alpha", UUID, false);
    rig.plant("running", OTHER, true);
    let pane = rig.app("running", false);
    rig.key(&pane, "z");
    rig.wait(&pane, "stopped card revealed", |s| s.contains("alpha"));
    rig.key(&pane, "j");
    rig.question(&pane, "alpha");
    rig.key(&pane, "y");
    rig.wait(&pane, "stopped End completed", |s| {
        s.contains("Ended alpha") && !rig.dir("alpha").exists()
    });
    assert_archive(&rig);
    assert!(rig.live("running"));
}

#[test]
fn unsupported_archive_source_refuses_before_killing_the_session() {
    let rig = Rig::new("preflight");
    rig.plant("alpha", UUID, true);
    let pane = rig.app("alpha", false);
    rig.question(&pane, "alpha");
    // A recognised retained source is replaced, so the publisher's actual
    // source preflight must refuse; an unrelated symlink is not our oracle.
    fs::remove_file(rig.dir("alpha").join("memo.tsv")).unwrap();
    std::os::unix::fs::symlink(rig.root.join("missing"), rig.dir("alpha").join("memo.tsv"))
        .unwrap();
    let meta = fs::read(rig.dir("alpha").join("meta")).unwrap();
    rig.key(&pane, "y");
    rig.wait(&pane, "preflight error visible", |s| {
        !s.contains("[ End ]") && s.contains("FAILED")
    });
    assert!(
        rig.live("alpha"),
        "pre-kill detectable archive refusal must keep session RUNNING"
    );
    assert_eq!(fs::read(rig.dir("alpha").join("meta")).unwrap(), meta);
    assert!(!rig.archive().exists());
}

#[test]
fn publication_failure_keeps_stopped_resumable_state_and_names_the_outcome() {
    let rig = Rig::new("publish");
    rig.plant("alpha", UUID, true);
    let pane = rig.app("alpha", false);
    rig.question(&pane, "alpha");
    let meta = fs::read(rig.dir("alpha").join("meta")).unwrap();
    let memo = fs::read(rig.dir("alpha").join("memo.tsv")).unwrap();
    // The target is absent, so transaction::inspect passes. Publication then
    // refuses archive's symlink root after the verified kill. A regular-file
    // parent would produce ENOTDIR during inspect and fail BEFORE the kill.
    let archive_storage = rig.root.join("archive-storage");
    fs::create_dir(&archive_storage).unwrap();
    std::os::unix::fs::symlink(&archive_storage, rig.state.join("archive")).unwrap();
    rig.key(&pane, "y");
    let screen = rig.wait(&pane, "stopped-not-ended failure visible", |s| {
        s.contains("STOPPED") && s.contains("not ended") && !s.contains("[ End ]")
    });
    assert!(
        !screen.contains("Ended alpha"),
        "publication failure claimed completion: {screen}"
    );
    assert!(!rig.live("alpha"));
    assert!(rig.dir("alpha").is_dir());
    assert_eq!(fs::read(rig.dir("alpha").join("memo.tsv")).unwrap(), memo);
    let after = fs::read_to_string(rig.dir("alpha").join("meta")).unwrap();
    assert!(
        after.contains(UUID),
        "the preserved session must keep its identity"
    );
    assert!(String::from_utf8_lossy(&meta).contains(UUID));
    assert!(
        after.contains("work_dir="),
        "the stopped record remains resumable"
    );
    assert!(
        fs::read_to_string(rig.dir("alpha").join("events.jsonl"))
            .unwrap()
            .contains("S6B_HISTORY_SENTINEL")
    );
    fs::remove_file(rig.state.join("archive")).unwrap();
    let (code, out, err) = rig.run_end(&["alpha".into(), "--no-attach".into()], &[]);
    assert_eq!(
        code, 0,
        "preserved state must resume through public entry: {out}\n{err}"
    );
    assert!(
        rig.live("alpha"),
        "publication failure left an unresumable session"
    );
}

#[test]
fn swapping_meta_identity_while_the_question_is_open_writes_nothing() {
    let rig = Rig::new("modal-swap");
    rig.plant("alpha", UUID, true);
    let pane = rig.app("alpha", false);
    rig.question(&pane, "alpha");
    let path = rig.dir("alpha").join("meta");
    let meta = fs::read_to_string(&path).unwrap().replace(UUID, OTHER);
    fs::write(&path, meta).unwrap();
    let before = snapshot(&rig.dir("alpha"));
    rig.key(&pane, "y");
    rig.wait(&pane, "stale modal refused", |s| {
        !s.contains("[ End ]") && s.to_lowercase().contains("refused")
    });
    assert_eq!(snapshot(&rig.dir("alpha")), before);
    assert!(rig.live("alpha"));
    assert!(!rig.archive().exists());
}

#[test]
fn guarded_end_reproves_identity_after_waiting_for_the_removal_lock() {
    let rig = Rig::new("lock-swap");
    rig.plant("alpha", UUID, true);
    let words = rig.frozen("alpha", true);
    let lock = fs::File::create(rig.state.join("sessions/.lifecycle.alpha.lock")).unwrap();
    lock.lock().unwrap();
    thread::scope(|scope| {
        let result = scope.spawn(|| rig.run_end(&words, &[]));
        thread::sleep(Duration::from_millis(300));
        assert!(
            !result.is_finished(),
            "End did not wait for its lifecycle lock"
        );
        let path = rig.dir("alpha").join("meta");
        let meta = fs::read_to_string(&path).unwrap().replace(UUID, OTHER);
        fs::write(&path, meta).unwrap();
        let before = snapshot(&rig.dir("alpha"));
        lock.unlock().unwrap();
        let (code, out, err) = result.join().unwrap();
        assert_eq!(
            code, 1,
            "expected under-lock identity refusal: {out}\n{err}"
        );
        assert!(
            err.contains("state directory") || err.contains("replaced"),
            "{err}"
        );
        assert_eq!(
            snapshot(&rig.dir("alpha")),
            before,
            "refusal wrote before proving identity"
        );
    });
    assert!(rig.live("alpha"));
    assert!(!rig.archive().exists());
}

#[test]
fn every_frozen_live_identity_and_deadline_is_checked_before_any_write() {
    for (tag, flag, value, cause) in [
        ("expired", "--expect-deadline", "1", "expired"),
        ("future", "--expect-deadline", "4102444800", "future"),
        ("uuid", "--expect-uuid", OTHER, "state directory"),
        ("pid", "--expect-server-pid", "1", "server"),
        ("start", "--expect-server-start", "1", "server"),
        (
            "selector",
            "--expect-server",
            "/nonexistent/s6b.sock",
            "server",
        ),
        ("tmux", "--confirmed-tmux", "$999999|1", "tmux session"),
    ] {
        let rig = Rig::new(tag);
        rig.plant("alpha", UUID, true);
        let mut words = rig.frozen("alpha", true);
        change(&mut words, flag, value);
        let before = snapshot(&rig.dir("alpha"));
        let (code, out, err) = rig.run_end(&words, &[]);
        assert_eq!(
            code, 1,
            "{tag}: runtime refusal, not parse failure: {out}\n{err}"
        );
        assert!(err.to_lowercase().contains(cause), "{tag}: {err}");
        assert_eq!(
            snapshot(&rig.dir("alpha")),
            before,
            "{tag}: target write before proof"
        );
        assert!(rig.live("alpha"), "{tag}: killed the unproven target");
        assert!(!rig.archive().exists());
    }
}

#[test]
fn missing_duplicate_or_invalid_raw_uuid_never_gets_migrated_by_confirmation() {
    for tag in ["missing", "duplicate", "invalid"] {
        let rig = Rig::new(tag);
        rig.plant("alpha", UUID, true);
        let words = rig.frozen("alpha", true);
        let path = rig.dir("alpha").join("meta");
        let before_meta = fs::read_to_string(&path).unwrap();
        let meta = match tag {
            "missing" => before_meta.replace(&format!("session_id={UUID}\n"), ""),
            "duplicate" => format!("{before_meta}session_id={UUID}\n"),
            _ => before_meta.replace(UUID, "not-a-uuid"),
        };
        fs::write(path, meta).unwrap();
        let before = snapshot(&rig.dir("alpha"));
        let (code, out, err) = rig.run_end(&words, &[]);
        assert_eq!(code, 1, "{tag}: {out}\n{err}");
        assert_eq!(
            snapshot(&rig.dir("alpha")),
            before,
            "{tag}: migrated or recorded before proof"
        );
        assert!(rig.live("alpha"));
    }
}

#[test]
fn a_recreated_tmux_session_is_not_the_session_the_human_confirmed() {
    let rig = Rig::new("incarnation");
    rig.plant("alpha", UUID, true);
    let words = rig.frozen("alpha", true);
    rig.must_tmux(&["kill-session", "-t", "=alpha"]);
    rig.plant("alpha", UUID, true);
    let before = snapshot(&rig.dir("alpha"));
    let (code, out, err) = rig.run_end(&words, &[]);
    assert_eq!(code, 1, "{out}\n{err}");
    assert!(err.contains("different tmux session"), "{err}");
    assert_eq!(snapshot(&rig.dir("alpha")), before);
    assert!(rig.live("alpha"));
}

#[test]
fn confirmed_literal_all_ends_only_that_one_session_in_the_app() {
    let rig = Rig::new("literal-all-app");
    rig.plant("all", UUID, true);
    rig.plant("sibling", OTHER, true);
    let pane = rig.app("all", false);
    rig.question(&pane, "all");
    // This session was never even present when the human saw the question.
    rig.plant("late", "0199c0de-cccc-4890-abcd-ef0123456789", true);
    let sibling = snapshot(&rig.dir("sibling"));
    let late = snapshot(&rig.dir("late"));
    rig.key(&pane, "y");
    rig.wait(&pane, "literal all completed", |s| {
        s.contains("Ended all") && !rig.dir("all").exists()
    });
    assert_archive(&rig);
    for (name, before) in [("sibling", sibling), ("late", late)] {
        assert!(rig.live(name), "literal all widened to {name}");
        assert_eq!(snapshot(&rig.dir(name)), before);
    }
}

#[test]
fn guarded_supervisor_preserves_literal_all_instead_of_reentering_fleet_end() {
    let rig = Rig::new("literal-all-owner");
    rig.plant("all", UUID, true);
    let words = rig.frozen("all", true);
    rig.plant("late", OTHER, true);
    let late = snapshot(&rig.dir("late"));
    let (code, out, err) = rig.run_end(&words, &[]);
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(!rig.dir("all").exists());
    assert!(!rig.live("all"));
    assert_archive(&rig);
    assert!(rig.live("late"), "confirmed single target widened to fleet");
    assert_eq!(snapshot(&rig.dir("late")), late);
}

#[test]
fn absent_session_appearing_before_confirm_is_refused_without_target_writes() {
    let rig = Rig::new("absent-live");
    rig.plant("alpha", UUID, false);
    let words = rig.frozen("alpha", false);
    rig.plant("alpha", UUID, true);
    let before = snapshot(&rig.dir("alpha"));
    let (code, out, err) = rig.run_end(&words, &[]);
    assert_eq!(code, 1, "{out}\n{err}");
    assert_eq!(snapshot(&rig.dir("alpha")), before);
    assert!(rig.live("alpha"));
    assert!(!rig.archive().exists());
}

#[test]
fn guarded_end_rejects_incomplete_duplicate_or_unattended_identity_grammar() {
    for tag in [
        "missing",
        "duplicate",
        "half-process",
        "ambient",
        "not-supervised",
    ] {
        let rig = Rig::new(tag);
        rig.plant("alpha", UUID, true);
        let mut words = rig.frozen("alpha", true);
        match tag {
            "missing" => words.retain(|word| !word.starts_with("--expect-uuid=")),
            "duplicate" => words.push(format!("--expect-uuid={UUID}")),
            "half-process" => change(&mut words, "--expect-server-start", ""),
            "ambient" => change(&mut words, "--expect-server-kind", "ambient"),
            _ => words.retain(|word| word != "--supervise"),
        }
        let before = snapshot(&rig.dir("alpha"));
        let (code, out, err) = rig.run_end(&words, &[]);
        assert_eq!(
            code, 2,
            "{tag}: malformed expectation must be usage error: {out}\n{err}"
        );
        assert_eq!(snapshot(&rig.dir("alpha")), before);
        assert!(rig.live("alpha"));
    }
}

#[test]
fn changed_history_plan_is_refused_before_migration_or_request_event() {
    let rig = Rig::new("changed-plan");
    rig.plant("alpha", UUID, true);
    let mut words = rig.frozen("alpha", true);
    change(&mut words, "--confirmed-detail", "/a/different/archive");
    let before = snapshot(&rig.dir("alpha"));
    let (code, out, err) = rig.run_end(&words, &[]);
    assert_eq!(code, 1, "{out}\n{err}");
    assert!(
        err.contains("changed") || err.contains("different"),
        "{err}"
    );
    assert_eq!(snapshot(&rig.dir("alpha")), before);
    assert!(rig.live("alpha"));
}

#[test]
fn losing_a_previously_reachable_stopped_server_refuses_before_any_write() {
    let rig = Rig::new("absent-unknown");
    rig.plant("alpha", UUID, false);
    let words = rig.frozen("alpha", false);
    // Keep the actual server private and reachable at the new socket for
    // teardown; the confirmed selector now answers Unknown rather than Absent.
    let moved = rig.root.join("moved.sock");
    fs::rename(&rig.sock, &moved).unwrap();
    let before = snapshot(&rig.dir("alpha"));
    let (code, out, err) = rig.run_end(&words, &[]);
    assert_eq!(code, 1, "{out}\n{err}");
    assert_eq!(
        snapshot(&rig.dir("alpha")),
        before,
        "unknown server earned a write"
    );
    assert!(!rig.archive().exists());
    fs::rename(moved, &rig.sock).unwrap();
}

fn missing_server(rig: &Rig) -> String {
    let missing = rig.root.join("never.sock").display().to_string();
    let path = rig.dir("alpha").join("meta");
    let meta = fs::read_to_string(&path)
        .unwrap()
        .replace(&rig.sock.display().to_string(), &missing);
    fs::write(path, format!("{meta}created=1700000000\n")).unwrap();
    missing
}

#[test]
fn reboot_proof_is_rechecked_under_lock_before_a_stopped_end_writes() {
    let rig = Rig::new("boot-reproof");
    rig.plant("alpha", UUID, false);
    let mut words = rig.frozen("alpha", false);
    let missing = missing_server(&rig);
    change(&mut words, "--confirmed-tmux", "unknown");
    change(&mut words, "--expect-server", &missing);
    words.push("--assume-stopped".into());
    let lock = fs::File::create(rig.state.join("sessions/.lifecycle.alpha.lock")).unwrap();
    lock.lock().unwrap();
    thread::scope(|scope| {
        let result =
            scope.spawn(|| rig.run_end(&words, &[("AE_TEST_BOOT_TIME", "1700003600".into())]));
        thread::sleep(Duration::from_millis(300));
        assert!(!result.is_finished(), "End did not wait for removal lock");
        // The newest launch now postdates the captured reboot, while the
        // selector, uuid and frozen Unknown remain unchanged.
        let path = rig.dir("alpha").join("meta");
        let meta = fs::read_to_string(&path)
            .unwrap()
            .replace("created=1700000000", "created=1800000000");
        fs::write(path, meta).unwrap();
        let before = snapshot(&rig.dir("alpha"));
        lock.unlock().unwrap();
        let (code, out, err) = result.join().unwrap();
        assert_eq!(code, 1, "{out}\n{err}");
        assert_eq!(
            snapshot(&rig.dir("alpha")),
            before,
            "boot reproof came after a write"
        );
    });
    assert!(!rig.archive().exists());
}

#[test]
fn reboot_proven_stopped_session_can_end_with_flag_and_proof_but_neither_alone() {
    for tag in ["proved", "no-flag", "no-proof"] {
        let rig = Rig::new(tag);
        rig.plant("alpha", UUID, false);
        let mut words = rig.frozen("alpha", false);
        let missing = missing_server(&rig);
        change(&mut words, "--confirmed-tmux", "unknown");
        change(&mut words, "--expect-server", &missing);
        if tag != "no-flag" {
            words.push("--assume-stopped".into());
        }
        let boot = if tag == "no-proof" {
            "1699990000"
        } else {
            "1700003600"
        };
        let before = snapshot(&rig.dir("alpha"));
        let (code, out, err) = rig.run_end(&words, &[("AE_TEST_BOOT_TIME", boot.into())]);
        if tag == "proved" {
            assert_eq!(code, 0, "{out}\n{err}");
            assert_archive(&rig);
            assert!(!rig.dir("alpha").exists());
        } else {
            assert_eq!(code, 1, "{tag}: {out}\n{err}");
            assert_eq!(snapshot(&rig.dir("alpha")), before);
            assert!(!rig.archive().exists());
        }
    }
}

#[test]
fn reboot_end_question_explains_the_proof_and_keeps_cancel_default() {
    let rig = Rig::new("boot-modal");
    rig.plant("alpha", UUID, false);
    missing_server(&rig);
    rig.plant("running", OTHER, true);
    let pane = rig.app_boot("running", false, Some(1700003600));
    rig.key(&pane, "z");
    rig.wait(&pane, "reboot stopped card revealed", |s| {
        s.contains("alpha")
    });
    rig.key(&pane, "j");
    let question = rig.question(&pane, "alpha");
    let lower = question.to_lowercase();
    assert!(
        lower.contains("server") && lower.contains("gone") && lower.contains("reboot"),
        "reboot acknowledgment must be explicit: {question}"
    );
    let before = snapshot(&rig.dir("alpha"));
    rig.key(&pane, "Enter");
    rig.wait(
        &pane,
        "Cancel remains default after reboot acknowledgment",
        |s| !s.contains("[ End ]"),
    );
    assert_eq!(snapshot(&rig.dir("alpha")), before);
    assert!(!rig.archive().exists());
    rig.question(&pane, "alpha");
    rig.key(&pane, "y");
    rig.wait(&pane, "reboot End completed", |s| {
        s.contains("Ended alpha") && !rig.dir("alpha").exists()
    });
    assert_archive(&rig);
    assert!(rig.live("running"));
}

#[test]
fn ending_the_apps_own_session_survives_its_pane_and_finishes_removal() {
    let rig = Rig::new("own-home");
    rig.plant("alpha", UUID, true);
    rig.plant("sibling", OTHER, true);
    let sibling = snapshot(&rig.dir("sibling"));
    let pane = rig.app_at("alpha", true, None, "alpha");
    let question = rig.question(&pane, "alpha");
    assert!(
        question.to_lowercase().contains("this app closes"),
        "own-home confirmation must name the app closing: {question}"
    );
    assert!(rig.live("alpha"));
    rig.key(&pane, "y");
    // The observer pane is intentionally killed. Observe through the private
    // server's unrelated host, never by requiring the dead app to draw a result.
    let until = Instant::now() + Duration::from_secs(30);
    while rig.live("alpha") || rig.dir("alpha").exists() {
        assert!(
            Instant::now() < until,
            "supervisor died or stalled when its app pane closed"
        );
        thread::sleep(Duration::from_millis(25));
    }
    assert_archive(&rig);
    let panes = rig.must_tmux(&["list-panes", "-a", "-F", "#{pane_id}"]);
    assert!(
        !panes.lines().any(|line| line == pane),
        "the ended app pane survived"
    );
    assert!(rig.live("sibling"));
    assert_eq!(snapshot(&rig.dir("sibling")), sibling);
    assert!(
        rig.must_tmux(&["list-clients", "-F", "#{client_name}"])
            .is_empty()
    );
}

#[test]
fn effective_purge_question_names_no_archive_cancel_preserves_and_confirm_purges() {
    let rig = Rig::new("purge");
    rig.plant("alpha", UUID, true);
    rig.plant("sibling", OTHER, true);
    let policy = rig.root.join("purge-config");
    fs::write(&policy, "[workspace]\npurge_agent_history = on\n").unwrap();
    let path = rig.dir("alpha").join("meta");
    let meta = fs::read_to_string(&path).unwrap();
    fs::write(path, format!("{meta}config={}\n", policy.display())).unwrap();
    let sibling = snapshot(&rig.dir("sibling"));
    let pane = rig.app("alpha", false);
    let question = rig.question(&pane, "alpha");
    assert!(
        question.contains("NO archive"),
        "purge consequence must be explicit: {question}"
    );
    let before = snapshot(&rig.dir("alpha"));
    rig.key(&pane, "Enter");
    rig.wait(&pane, "purge Cancel is default", |s| !s.contains("[ End ]"));
    assert_eq!(snapshot(&rig.dir("alpha")), before);
    assert!(rig.live("alpha"));
    assert!(!rig.archive().exists());
    rig.question(&pane, "alpha");
    rig.key(&pane, "y");
    rig.wait(&pane, "purge result visible", |s| {
        s.contains("Ended alpha") && !rig.dir("alpha").exists()
    });
    assert!(!rig.live("alpha"));
    assert!(
        !rig.archive().exists(),
        "effective purge silently used Keep"
    );
    assert!(rig.live("sibling"));
    assert_eq!(snapshot(&rig.dir("sibling")), sibling);
}

#[test]
fn unavailable_corrupt_state_has_no_affirmable_end_and_preserves_the_tree() {
    let rig = Rig::new("unavailable");
    rig.plant("alpha", UUID, true);
    let pane = rig.app("alpha", false);
    // Preserve the already drawn card but corrupt its raw archive identity
    // before selecting End. Neither stale card nor a fresh Unknown permits y.
    let path = rig.dir("alpha").join("meta");
    let meta = fs::read_to_string(&path)
        .unwrap()
        .replace(UUID, "not-a-uuid");
    fs::write(path, meta).unwrap();
    let before = snapshot(&rig.dir("alpha"));
    rig.key(&pane, "m");
    rig.wait(&pane, "End remains visible on corrupt card", |s| {
        s.contains("End…")
    });
    rig.key(&pane, "e");
    let screen = rig.wait(&pane, "unavailable End visibly refused", |s| {
        !s.contains("[ End ]") && s.to_lowercase().contains("refused")
    });
    assert!(
        !screen.contains("[ End ]"),
        "unavailable target offered affirmative action"
    );
    assert_eq!(snapshot(&rig.dir("alpha")), before);
    assert!(rig.live("alpha"));
    assert!(!rig.archive().exists());
}
