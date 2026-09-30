//! Independent acceptance tests for the restore contract. Fixtures use real
//! launches and private tmux servers; only durable input facts are planted.

#![allow(
    clippy::disallowed_methods,
    reason = "acceptance fixtures create and inspect their own isolated session state"
)]

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use super::cli::{OwnedScratch, Runner, ae, bounded};
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
        // This command runs on the fixture's private -S server only. A crash
        // keeps its socket, unlike a clean kill-server or last-session exit.
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
            "display-message",
            "-p",
            "-t",
            &format!("={name}"),
            "#{session_id}|#{@ae_session_uuid}|#{pane_id}",
        ]);
        assert!(ok, "live identity of {name}");
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
#[ignore = "restore phase 2"]
fn bare_restore_resumes_only_the_recent_unstopped_watched_cohort() {
    let fleet = Fleet::new("cohort");
    for name in ["alpha", "beta", "failed"] {
        fleet.missing(name, 600, Some(60));
    }
    fleet.event("failed", 500, "stop-result", "FAILED: unavailable");
    fleet.missing("edge", 1800, Some(960)); // W=900 inclusive.
    fleet.missing("abandoned", 1800, Some(961)); // W+1 excluded.
    fleet.missing("stopped", 600, Some(60));
    fleet.event("stopped", 500, "stop-result", "already stopped");
    fleet.missing("equal", 600, Some(60));
    fleet.event(
        "equal",
        600,
        "stop-result",
        "stopped: verified gone on its recorded server",
    );
    fleet.missing("unwatched", 600, None);
    // Lead ruling 2026-09-30: a pre-launch beat belongs to the prior incarnation.
    fleet.missing("priorbeat", 30, Some(60));
    let frozen = ["abandoned", "stopped", "equal", "unwatched", "priorbeat"].map(|name| {
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
    for (name, before) in frozen {
        unchanged(&fleet.dir(name).join("meta"), &before);
    }
}

#[test]
#[ignore = "restore phase 2"]
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
#[ignore = "restore phase 2"]
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
#[ignore = "restore phase 2"]
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
#[ignore = "restore phase 2"]
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
#[ignore = "restore phase 2"]
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
#[ignore = "restore phase 2"]
fn a_failed_stop_request_allows_a_later_bare_restore() {
    let fleet = Fleet::new("failstop");
    fleet.missing("retry", 600, Some(60));
    fleet.event("retry", 500, "stop-request", "stop requested");
    fleet.event("retry", 400, "stop-result", "FAILED: unavailable");
    success(&fleet.run(&["--no-attach"]));
    assert_eq!(fleet.names(), ["keeper", "retry"]);
}

#[test]
#[ignore = "restore phase 2"]
fn unproven_recorded_server_is_reported_and_skipped_beside_a_success() {
    let fleet = Fleet::new("unknown");
    fleet.missing("proven", 600, Some(60));
    fleet.missing("unproven", 600, Some(60));
    let path = fleet.dir("unproven").join("meta");
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
        stderr.contains("unproven"),
        "skipped session unnamed: {stderr}"
    );
    assert!(
        stderr.contains("missing.sock"),
        "absence reason missing: {stderr}"
    );
    unchanged(&path, edited.as_bytes());
}

#[test]
#[ignore = "restore phase 2"]
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
#[ignore = "restore phase 3"]
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
