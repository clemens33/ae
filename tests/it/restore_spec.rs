//! Independent acceptance tests for the restore contract. Fixtures use real
//! launches and private tmux servers; only durable input facts are planted.

#![allow(
    clippy::disallowed_methods,
    reason = "acceptance fixtures create and inspect their own isolated session state"
)]

use std::io::{BufRead as _, Read as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use super::cli::{OwnedChild, OwnedScratch, Runner, ae, bounded};
use super::phase2::run_tmux;

const CONFIG: &str = "[profiles]\nidle = \"sleep 600\"\n[roster]\nlead = idle\n\
    [workspace]\nmain = lead\nlayout = vertical\n";

struct Fleet {
    scratch: OwnedScratch,
    home: PathBuf,
    project: PathBuf,
    socket: PathBuf,
    epoch: i64,
}

impl Fleet {
    fn new(tag: &str) -> Self {
        let mut scratch = OwnedScratch::root("rs", tag);
        let home = scratch.join("state");
        let project = scratch.join("project");
        let socket = scratch.join("sock");
        scratch.add_tmux_server(socket.clone());
        assert!(
            std::fs::create_dir_all(&home).is_ok(),
            "isolated state root"
        );
        assert!(
            std::fs::create_dir_all(&project).is_ok(),
            "isolated project"
        );
        assert!(
            std::fs::write(home.join("config"), CONFIG).is_ok(),
            "harmless idle configuration"
        );
        let epoch = ae::time::Timestamp::now().epoch();
        let fleet = Self {
            scratch,
            home,
            project,
            socket,
            epoch,
        };
        assert!(
            fleet
                .tmux(&["-f", "/dev/null", "new-session", "-d", "-s", "keeper"])
                .0
        );
        fleet
    }

    fn command(&self, args: &[&str]) -> Runner {
        let mut command = ae();
        command
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("HOME", &self.scratch)
            .env("AE_HOME", &self.home)
            .env("CONFIG_FILE", self.home.join("config"))
            .env("TMUX_TMPDIR", &self.scratch)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .env("AE_NO_AUTOSTART", "1")
            .current_dir(&self.project)
            .args(args);
        command
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        self.command(args)
            .output()
            .unwrap_or_else(|why| panic!("real ae entry: {why}"))
    }

    fn tmux(&self, words: &[&str]) -> (bool, String) {
        let mut argv = vec!["-S".to_owned(), self.socket.display().to_string()];
        argv.extend(words.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&argv, &self.scratch)
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.home.join("sessions").join(name)
    }

    fn launch(&self, name: &str) {
        success(&self.run(&["--local", name, "--no-attach"]));
    }

    fn missing(&self, name: &str, launch_age: i64, beat_age: Option<i64>) {
        self.launch(name);
        assert!(self.tmux(&["kill-session", "-t", &format!("={name}")]).0);
        self.stamp(name, self.epoch - launch_age);
        if let Some(age) = beat_age {
            self.beat(name, self.epoch - age);
        }
    }

    fn crash(&self) {
        let (ok, pid) = self.tmux(&["display-message", "-p", "#{pid}"]);
        assert!(ok, "private server pid");
        let pid = pid
            .trim()
            .parse::<u32>()
            .unwrap_or_else(|why| panic!("numeric private server pid: {why}"));
        assert!(pid > 1);
        // This command runs on the fixture's private -S server only. SIGKILL
        // leaves a stale socket; a clean exit can leave one too.
        let _ = self.tmux(&["run-shell", &format!("kill -KILL {pid}")]);
        assert!(!self.tmux(&["list-sessions"]).0, "server must be gone");
        // phase2::run_tmux writes command stderr here (phase2.rs:1022).
        let diagnostic = std::fs::read_to_string(self.scratch.join("stderr"))
            .unwrap_or_else(|why| panic!("private crash diagnostic: {why}"));
        assert!(
            diagnostic.starts_with("no server running on "),
            "stale socket must prove absence: {diagnostic}"
        );
    }

    fn stamp(&self, name: &str, epoch: i64) {
        assert!(
            std::fs::write(self.dir(name).join(".launch-attempt"), format!("{epoch}\n")).is_ok(),
            "controlled launch epoch"
        );
    }

    fn beat(&self, name: &str, epoch: i64) {
        let beat = std::fs::File::create(self.dir(name).join(".watchdog-beat"))
            .unwrap_or_else(|why| panic!("controlled mtime-only beat: {why}"));
        let seconds =
            u64::try_from(epoch).unwrap_or_else(|why| panic!("positive fixture epoch: {why}"));
        assert!(
            beat.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
                .is_ok(),
            "controlled beat mtime"
        );
    }

    fn event(&self, name: &str, age: i64, action: &str, summary: &str) {
        ae::store::open(&self.dir(name))
            .append_event(&line(self.epoch - age, name, action, summary))
            .unwrap_or_else(|why| panic!("fixture ledger append: {why}"));
    }

    fn identity(&self, name: &str) -> String {
        let (ok, identity) = self.tmux(&[
            "list-panes",
            "-s",
            "-t",
            &format!("={name}"),
            "-F",
            "#{session_id}|#{@ae_session_uuid}|#{pane_id}",
        ]);
        assert!(
            ok && identity
                .lines()
                .next()
                .is_some_and(|row| row.starts_with('$')),
            "live identity of {name}: {identity}"
        );
        identity
    }

    fn names(&self) -> Vec<String> {
        let (ok, names) = self.tmux(&["list-sessions", "-F", "#{session_name}"]);
        assert!(ok, "private server must answer");
        let mut names = names.lines().map(str::to_owned).collect::<Vec<_>>();
        names.sort();
        names
    }
}

fn line(epoch: i64, name: &str, action: &str, summary: &str) -> String {
    // Stop-result spellings: src/lifecycle.rs:683,688,689 at 17afa2af.
    format!(
        "{{\"ts\":\"{}\",\"actor\":\"human\",\"action\":\"{action}\",\"target\":\"{name}\",\"summary\":\"{summary}\"}}\n",
        ae::time::Timestamp::from_epoch(epoch)
    )
}

fn success(out: &std::process::Output) {
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn unchanged(path: &Path, before: &[u8]) {
    assert_eq!(
        std::fs::read(path).unwrap_or_else(|why| panic!("retained state: {why}")),
        before
    );
}

fn skip_reason(stderr: &str, name: &str) -> String {
    // Per-session explanations precede summaries that may name several sessions.
    stderr
        .lines()
        .find(|line| {
            line.split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
                .any(|token| token == name)
        })
        .unwrap_or_else(|| panic!("skipped {name} unnamed on stderr: {stderr}"))
        .replacen(name, "", 1)
        .to_ascii_lowercase()
}

fn waiting_restore(
    fleet: &Fleet,
    name: &str,
) -> (std::fs::File, OwnedChild, std::thread::JoinHandle<String>) {
    let held = ae::store::lock(
        &fleet
            .home
            .join("sessions")
            .join(format!(".lifecycle.{name}.lock")),
        Duration::from_secs(1),
    )
    .unwrap_or_else(|why| panic!("hold fixture lifecycle lock: {why}"));
    let mut command = fleet.command(&["--no-attach"]);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|why| panic!("restore waiting on lifecycle lock: {why}"));
    let stdout = child
        .stdout
        .take()
        .unwrap_or_else(|| panic!("restore stdout pipe"));
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        let mut transcript = String::new();
        let _ = reader.read_line(&mut transcript);
        let _ = sender.send(transcript.clone());
        reader
            .read_to_string(&mut transcript)
            .unwrap_or_else(|why| panic!("restore progress stream: {why}"));
        transcript
    });
    // Public progress follows the advisory scan and precedes the lock. The
    // pipe/channel barrier makes the following ledger/beat change a real race.
    let first = receiver.recv_timeout(Duration::from_secs(10));
    let expected = format!("ae: restoring {name}");
    let started = first.as_ref().is_ok_and(|line| {
        line.trim_end()
            .strip_prefix(&expected)
            .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with(' '))
    });
    if !started {
        let _ = child.kill();
        let failed = bounded(child, Duration::from_secs(5));
        let transcript = reader.join().unwrap_or_default();
        panic!("restore progress missing: first={first:?}, out={failed:?}, stdout={transcript}");
    }
    (held, child, reader)
}

fn finish_restore(
    child: OwnedChild,
    reader: std::thread::JoinHandle<String>,
) -> std::process::Output {
    let output = bounded(child, Duration::from_mins(2));
    let stdout = reader
        .join()
        .unwrap_or_else(|why| panic!("restore reader: {why:?}"));
    let mut output = output.unwrap_or_else(|| panic!("restore must terminate"));
    output.stdout = stdout.into_bytes();
    output
}

#[test]
fn resume_retains_latest_clean_stop_even_behind_a_later_failed_stop() {
    let fleet = Fleet::new("trim");
    fleet.missing("retained", 600, Some(60));
    let old = line(
        fleet.epoch - 1000,
        "retained",
        "stop-result",
        "already stopped",
    );
    let latest = line(
        fleet.epoch - 900,
        "retained",
        "stop-result",
        "stopped: verified gone on its recorded server",
    );
    let failed = line(
        fleet.epoch - 800,
        "retained",
        "stop-result",
        "FAILED: unavailable",
    );
    let ordinary = line(fleet.epoch - 700, "retained", "state", "done");
    let ledger = format!("{old}{latest}{failed}{}", ordinary.repeat(1005));
    let path = fleet.dir("retained").join("events.jsonl");
    std::fs::write(&path, ledger).expect("ledger beyond resume retention window");
    success(&fleet.run(&["retained", "--no-attach"]));
    let retained = std::fs::read_to_string(path).expect("trimmed ledger");
    assert!(
        retained.contains(latest.trim()),
        "resume dropped the last clean stop"
    );
    assert!(
        !retained.contains(old.trim()),
        "only newest clean stop is pinned"
    );
    assert!(
        !retained.contains(failed.trim()),
        "FAILED is not a clean stop pin"
    );
}

#[test]
fn bare_restore_resumes_only_the_recent_unstopped_watched_cohort() {
    let fleet = Fleet::new("cohort");
    for name in ["alpha", "beta", "failed"] {
        fleet.missing(name, 600, Some(60));
    }
    fleet.event("failed", 500, "stop-result", "FAILED: unavailable");
    fleet.missing("edge", 1800, Some(960)); // W=900 inclusive.
    fleet.missing("s-old", 1800, Some(961)); // W+1 excluded.
    fleet.missing("s-clean", 600, Some(60));
    fleet.event("s-clean", 500, "stop-result", "already stopped");
    fleet.missing("s-equal", 600, Some(60));
    fleet.event(
        "s-equal",
        600,
        "stop-result",
        "stopped: verified gone on its recorded server",
    );
    fleet.missing("s-nobeat", 600, None);
    // Lead ruling 2026-09-30: a pre-launch beat belongs to the prior incarnation.
    fleet.missing("s-prior", 30, Some(60));
    let frozen = ["s-old", "s-clean", "s-equal", "s-nobeat", "s-prior"].map(|name| {
        (
            name,
            std::fs::read(fleet.dir(name).join("meta")).expect("saved meta"),
        )
    });
    let out = fleet.run(&["--no-attach"]);
    success(&out);
    assert_eq!(fleet.names(), ["alpha", "beta", "edge", "failed", "keeper"]);
    let progress = String::from_utf8_lossy(&out.stdout);
    for name in ["alpha", "beta", "edge", "failed"] {
        assert!(progress.contains(name), "{progress}");
    }
    let skipped = String::from_utf8_lossy(&out.stderr);
    for (name, before) in frozen {
        unchanged(&fleet.dir(name).join("meta"), &before);
        let explanation = skip_reason(&skipped, name);
        let causes: &[&str] = match name {
            "s-old" => &["beat", "window", "old", "abandoned"],
            "s-clean" | "s-equal" => &["stop", "shutdown"],
            _ => &["beat", "watch", "incarnation", "launch"],
        };
        assert!(
            causes.iter().any(|cause| explanation.contains(cause)),
            "skipped {name} lacks its eligibility reason: {skipped}"
        );
    }
}

#[test]
fn a_fresh_live_sibling_does_not_evict_or_replace_the_crashed_cohort() {
    let fleet = Fleet::new("live");
    fleet.launch("live");
    fleet.beat("live", fleet.epoch);
    fleet.missing("lost", 7200, Some(3600));
    let identity = fleet.identity("live");
    let meta = std::fs::read(fleet.dir("live").join("meta")).expect("live meta");
    success(&fleet.run(&["--no-attach"]));
    assert_eq!(fleet.names(), ["keeper", "live", "lost"]);
    assert_eq!(fleet.identity("live"), identity, "live session was rebuilt");
    unchanged(&fleet.dir("live").join("meta"), &meta);
}

#[test]
fn a_present_non_ae_session_cannot_evict_the_crashed_cohort() {
    let fleet = Fleet::new("occupied");
    fleet.missing("occupied", 7200, Some(0));
    assert!(
        fleet
            .tmux(&["new-session", "-d", "-s", "occupied", "sleep 600"])
            .0
    );
    // Lead ruling: every Present name is outside the cohort, including a
    // same-name replacement without ae's ownership marker (tmux.rs:33).
    assert!(
        fleet
            .tmux(&["set-environment", "-u", "-t", "=occupied", "AE_SESSION"])
            .0
    );
    let (ok, _) = fleet.tmux(&["show-environment", "-t", "=occupied", "AE_SESSION"]);
    assert!(!ok, "non-ae fixture retains AE_SESSION ownership marker");
    let identity = fleet.identity("occupied");
    fleet.missing("lost", 7200, Some(3600));
    let path = fleet.dir("occupied").join("meta");
    let before = std::fs::read(&path).expect("saved occupied meta");
    success(&fleet.run(&["--no-attach"]));
    assert_eq!(fleet.names(), ["keeper", "lost", "occupied"]);
    assert_eq!(fleet.identity("occupied"), identity);
    unchanged(&path, &before);
}

#[test]
fn two_concurrent_bare_restores_and_a_later_repeat_keep_one_incarnation() {
    let fleet = Fleet::new("double");
    for name in ["alpha", "beta", "gamma"] {
        fleet.missing(name, 600, Some(60));
    }
    let mut first = fleet.command(&["--no-attach"]);
    let mut second = fleet.command(&["--no-attach"]);
    let first = first
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("first restore");
    let second = second
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("second restore");
    let first = bounded(first, Duration::from_mins(2)).expect("first restore terminates");
    let second = bounded(second, Duration::from_mins(2)).expect("second restore terminates");
    success(&first);
    success(&second);
    assert_eq!(fleet.names(), ["alpha", "beta", "gamma", "keeper"]);
    let identities = ["alpha", "beta", "gamma"].map(|name| fleet.identity(name));
    let attempts = ["alpha", "beta", "gamma"]
        .map(|name| std::fs::read(fleet.dir(name).join(".launch-attempt")).expect("launch stamp"));
    success(&fleet.run(&["--no-attach"]));
    for (index, name) in ["alpha", "beta", "gamma"].iter().enumerate() {
        assert_eq!(fleet.identity(name), identities[index]);
        unchanged(&fleet.dir(name).join(".launch-attempt"), &attempts[index]);
    }
}

#[test]
fn a_session_stopped_through_ae_stays_stopped_beside_a_restored_sibling() {
    let fleet = Fleet::new("cleanstop");
    fleet.missing("lost", 600, Some(60));
    fleet.launch("stopped");
    fleet.stamp("stopped", fleet.epoch - 600);
    fleet.beat("stopped", fleet.epoch - 60);
    success(&fleet.run(&["stop", "stopped"]));
    let meta = std::fs::read(fleet.dir("stopped").join("meta")).expect("stopped meta");
    let events =
        std::fs::read(fleet.dir("stopped").join("events.jsonl")).expect("real stop ledger");
    success(&fleet.run(&["--no-attach"]));
    assert_eq!(fleet.names(), ["keeper", "lost"]);
    unchanged(&fleet.dir("stopped").join("meta"), &meta);
    unchanged(&fleet.dir("stopped").join("events.jsonl"), &events);
}

#[test]
fn a_failed_restore_is_reported_without_preventing_the_next_session() {
    let fleet = Fleet::new("partial");
    fleet.missing("a-invalid", 600, Some(60));
    fleet.missing("z-valid", 600, Some(60));
    let path = fleet.dir("a-invalid").join("meta");
    let meta = std::fs::read_to_string(&path).expect("saved profile");
    assert!(
        meta.contains("profile.main=idle\n"),
        "fixture profile missing: {meta}"
    );
    let edited = meta.replace("profile.main=idle\n", "profile.main=removed\n");
    std::fs::write(&path, &edited).expect("unavailable saved profile");
    let out = fleet.run(&["--no-attach"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fleet.names(), ["keeper", "z-valid"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("z-valid"));
    assert!(String::from_utf8_lossy(&out.stderr).contains("a-invalid"));
    unchanged(&path, edited.as_bytes());
}

#[test]
fn a_pending_stop_request_is_not_overridden_by_bare_restore() {
    let fleet = Fleet::new("stopping");
    fleet.missing("stopping", 600, Some(60));
    fleet.event("stopping", 500, "stop-request", "stop requested");
    let before = std::fs::read(fleet.dir("stopping").join("meta")).expect("saved state");
    let out = fleet.run(&["--no-attach"]);
    assert_ne!(out.status.code(), Some(2), "restore flag is valid");
    assert_eq!(fleet.names(), ["keeper"]);
    unchanged(&fleet.dir("stopping").join("meta"), &before);
}

#[test]
fn a_failed_stop_request_allows_a_later_bare_restore() {
    let fleet = Fleet::new("failstop");
    fleet.missing("retry", 600, Some(60));
    fleet.event("retry", 500, "stop-request", "stop requested");
    fleet.event("retry", 400, "stop-result", "FAILED: unavailable");
    success(&fleet.run(&["--no-attach"]));
    assert_eq!(fleet.names(), ["keeper", "retry"]);
}

#[test]
fn unproven_recorded_server_is_reported_and_skipped_beside_a_success() {
    let fleet = Fleet::new("unknown");
    fleet.missing("proven", 600, Some(60));
    fleet.missing("u-saved", 600, Some(60));
    let path = fleet.dir("u-saved").join("meta");
    let before = std::fs::read_to_string(&path).expect("saved server record");
    let missing = fleet.scratch.join("missing.sock");
    let edited = before
        .lines()
        .map(|line| {
            if line.starts_with("tmux_server=") {
                format!("tmux_server={}", missing.display())
            } else if line.starts_with("tmux_server_kind=") {
                "tmux_server_kind=socket".to_owned()
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&path, &edited).expect("unknown server fixture");
    let out = fleet.run(&["--no-attach"]);
    success(&out); // A deliberate skip is not a failed restore attempt.
    assert_eq!(fleet.names(), ["keeper", "proven"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        skip_reason(&stderr, "u-saved").contains("missing.sock"),
        "absence reason missing: {stderr}"
    );
    unchanged(&path, edited.as_bytes());
}

#[test]
fn a_clean_stop_landing_after_the_scan_is_rechecked_under_the_lifecycle_lock() {
    let fleet = Fleet::new("lock-stop");
    fleet.missing("held", 600, Some(60));
    let path = fleet.dir("held").join("meta");
    let before = std::fs::read(&path).expect("saved meta");
    let (held, child, reader) = waiting_restore(&fleet, "held");
    // Fixture epoch may precede scan wall time. This stop follows the advisory
    // scan in ledger order and is newer than the controlled launch stamp.
    fleet.event("held", 0, "stop-result", "already stopped");
    drop(held);
    let out = finish_restore(child, reader);
    success(&out); // Keeper remains live; only this restore was skipped.
    assert_eq!(fleet.names(), ["keeper"]);
    assert!(skip_reason(&String::from_utf8_lossy(&out.stderr), "held").contains("stop"));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("ae: restored held"));
    unchanged(&path, &before);
}

#[test]
fn a_beat_moved_outside_the_carried_window_is_rechecked_under_the_lifecycle_lock() {
    let fleet = Fleet::new("lock-beat");
    fleet.missing("held", 1800, Some(60));
    let path = fleet.dir("held").join("meta");
    let before = std::fs::read(&path).expect("saved meta");
    let (held, child, reader) = waiting_restore(&fleet, "held");
    // Still newer than the launch, but older than the pre-scan cutoff at -960.
    fleet.beat("held", fleet.epoch - 1200);
    drop(held);
    let out = finish_restore(child, reader);
    success(&out);
    assert_eq!(fleet.names(), ["keeper"]);
    let reason = skip_reason(&String::from_utf8_lossy(&out.stderr), "held");
    assert!(reason.contains("beat") || reason.contains("window"));
    unchanged(&path, &before);
}

#[test]
fn a_recorded_server_changed_after_the_scan_cannot_reuse_the_old_cohort_cutoff() {
    let fleet = Fleet::new("lock-server");
    let destination = Fleet::new("lock-server-target");
    fleet.missing("held", 600, Some(60));
    let path = fleet.dir("held").join("meta");
    let before = std::fs::read_to_string(&path).expect("saved recorded server");
    let edited = before
        .lines()
        .map(|line| {
            if line.starts_with("tmux_server=") {
                format!("tmux_server={}", destination.socket.display())
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_ne!(edited, before, "recorded server fixture key");
    let (held, child, reader) = waiting_restore(&fleet, "held");
    // Both private servers prove this name absent, so either preflight view
    // succeeds. The locked recheck must bind the cutoff to the scanned server.
    std::fs::write(&path, &edited).expect("change only the recorded server");
    drop(held);
    let out = finish_restore(child, reader);
    success(&out);
    assert_eq!(fleet.names(), ["keeper"]);
    assert_eq!(destination.names(), ["keeper"]);
    let reason = skip_reason(&String::from_utf8_lossy(&out.stderr), "held");
    assert!(reason.contains("server") || reason.contains("record") || reason.contains("changed"));
    unchanged(&path, edited.as_bytes());
}

#[test]
fn a_manual_resume_and_bare_restore_create_one_incarnation() {
    let fleet = Fleet::new("manual-race");
    fleet.launch("joined");
    let previous = fleet.identity("joined");
    let previous = previous
        .lines()
        .next()
        .expect("first identity row")
        .split('|')
        .next()
        .expect("session id")
        .trim_start_matches('$')
        .parse::<u64>()
        .expect("numeric session id");
    assert!(fleet.tmux(&["kill-session", "-t", "=joined"]).0);
    fleet.stamp("joined", fleet.epoch - 600);
    fleet.beat("joined", fleet.epoch - 60);
    let mut manual = fleet.command(&["joined", "--no-attach"]);
    let manual = manual
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("manual resume");
    let mut bare = fleet.command(&["--no-attach"]);
    let bare = bare
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("bare restore beside manual resume");
    success(&bounded(manual, Duration::from_mins(2)).expect("manual resume terminates"));
    success(&bounded(bare, Duration::from_mins(2)).expect("bare restore terminates"));
    assert_eq!(fleet.names(), ["joined", "keeper"]);
    let identity = fleet.identity("joined");
    // Same private server stayed alive: one new tmux session advances its id once.
    assert_eq!(
        identity
            .lines()
            .next()
            .and_then(|row| row.split('|').next()),
        Some(format!("${}", previous + 1).as_str())
    );
    success(&fleet.run(&["--no-attach"]));
    assert_eq!(fleet.identity("joined"), identity);
}

#[test]
fn only_unknown_candidates_on_an_empty_target_keep_the_exit_one_hint() {
    let fleet = Fleet::new("unknown-empty");
    fleet.missing("saved", 600, Some(60));
    assert!(fleet.tmux(&["kill-session", "-t", "=keeper"]).0);
    // A clean exit can leave a stale socket and therefore prove absence.
    // This fixture deliberately tests SocketMissing, whose boot proof is Unknown.
    if let Err(why) = std::fs::remove_file(&fleet.socket) {
        assert_eq!(why.kind(), std::io::ErrorKind::NotFound, "{why}");
    }
    assert!(
        std::fs::symlink_metadata(&fleet.socket)
            .is_err_and(|why| why.kind() == std::io::ErrorKind::NotFound),
        "fixture socket must be explicitly missing"
    );
    let path = fleet.dir("saved").join("meta");
    let before = std::fs::read(&path).expect("saved meta");
    let out = fleet.run(&["--no-attach"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let reason = skip_reason(&stderr, "saved");
    assert!(
        ["unproven", "unknown", "not before boot", "socket missing"]
            .iter()
            .any(|cause| reason.contains(cause)),
        "missing absence explanation: {stderr}"
    );
    assert!(stderr.contains("ae: no running ae session. Start one with: ae <name>\n"));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("ae: restored"));
    unchanged(&path, &before);
}

#[test]
fn a_private_server_crash_restores_the_previously_running_session() {
    let fleet = Fleet::new("crash-on");
    fleet.launch("saved");
    fleet.stamp("saved", fleet.epoch - 600);
    fleet.beat("saved", fleet.epoch - 60);
    fleet.crash();
    success(&fleet.run(&["--no-attach"]));
    assert_eq!(fleet.names(), ["saved"]);
}

#[test]
fn a_project_restore_off_cannot_override_the_global_restore_policy() {
    let fleet = Fleet::new("local-off");
    std::fs::write(fleet.home.join("config"), format!("{CONFIG}restore = on\n"))
        .expect("global restore policy");
    let local = fleet.project.join(".ae");
    std::fs::create_dir_all(&local).expect("project config directory");
    std::fs::write(local.join("config"), "[workspace]\nrestore = off\n")
        .expect("project opt-out must not vote");
    fleet.missing("saved", 600, Some(60));
    let out = fleet.run(&["--no-attach"]);
    success(&out);
    assert!(String::from_utf8_lossy(&out.stdout).contains("ae: restored saved"));
    assert_eq!(fleet.names(), ["keeper", "saved"]);
}

#[test]
fn unusable_restore_values_stay_on_and_are_named_once_on_stderr() {
    // An unquoted empty value fails the shared reader; its reason names "invalid".
    for (value, seen) in [
        ("OFF", "OFF"),
        ("ascii", "ascii"),
        ("no", "no"),
        ("", "invalid"),
    ] {
        let fleet = Fleet::new("bad-restore");
        fleet.missing("saved", 600, Some(60));
        std::fs::write(
            fleet.home.join("config"),
            format!("{CONFIG}restore = {value}\n"),
        )
        .expect("unusable global declaration");
        let out = fleet.run(&["--no-attach"]);
        success(&out);
        assert!(String::from_utf8_lossy(&out.stdout).contains("ae: restored saved"));
        assert_eq!(fleet.names(), ["keeper", "saved"]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            stderr
                .lines()
                .filter(|line| {
                    line.contains("restore")
                        && line
                            .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-')
                            .any(|token| token == seen)
                })
                .count(),
            1,
            "one note must name unusable declaration {value:?}: {stderr}"
        );
    }
}

#[test]
fn restore_off_preserves_the_empty_server_hint_and_saved_state_byte_for_byte() {
    let fleet = Fleet::new("off");
    fleet.launch("saved");
    fleet.stamp("saved", fleet.epoch - 600);
    fleet.beat("saved", fleet.epoch - 60);
    std::fs::write(
        fleet.home.join("config"),
        format!("{CONFIG}restore = off\n"),
    )
    .expect("global opt-out");
    fleet.crash();
    let before = std::fs::read(fleet.dir("saved").join("meta")).expect("saved meta");
    let out = fleet.run(&[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    // Baseline listing keeps a crashed recorded server Unknown (lib.rs:1373),
    // even though ordinary resume proves absence from the stale socket.
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "ae: no running ae session. Start one with: ae <name>\n"
    );
    unchanged(&fleet.dir("saved").join("meta"), &before);
    assert!(!fleet.tmux(&["has-session", "-t", "=saved"]).0);
}
