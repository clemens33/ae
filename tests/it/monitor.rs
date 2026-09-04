//! The orchestrator sweep, black-box, against a real tmux server.
//!
//! The sweep's whole value is its DEDUP, and a dedup can only be observed
//! across invocations: one run's answer means nothing without the next run's.
//! So every arm here drives the shipped binary two or three times over one
//! state file and asserts on the sequence — which is also the shape the defect
//! takes when it comes back (a change reported twice, or an undelivered one
//! reported never).
//!
//! A real server, not a double: `status == "running"` is what the sweep filters
//! on, and a fixture that faked it would prove the diff and nothing about which
//! sessions reach it.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::fs;
use std::path::{Path, PathBuf};

use super::cli::ae;
use super::parity::Invocation;
use super::parity::capture::raw;
use super::phase2::{run_tmux, tmux_present};

/// A scratch dir short enough to hold a socket path — `sun_path` is 104 bytes
/// on macOS and the usual temp dir eats most of it.
fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(format!("/tmp/ae-mon-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    assert!(fs::create_dir_all(&dir).is_ok(), "a short scratch dir");
    dir
}

/// Kill the arm's server and remove its scratch, WHATEVER ended the arm — a
/// failed assertion included, so one real failure does not leave a tmux server
/// behind to confuse the next test.
struct Cleanup {
    socket: PathBuf,
    scratch: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let out = self.scratch.join("cleanup-out");
        let err = self.scratch.join("cleanup-err");
        let invocation = Invocation::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .arg("kill-server");
        let _ = raw::run(&invocation, &self.scratch, &out, &err);
        let _ = fs::remove_dir_all(&self.scratch);
    }
}

fn tmux(socket: &Path, scratch: &Path, words: &[&str]) -> (bool, String) {
    let mut args = vec!["-S".to_owned(), socket.display().to_string()];
    args.extend(words.iter().map(|word| (*word).to_owned()));
    run_tmux(&args, scratch)
}

fn require_tmux(scratch: &Path) {
    if !tmux_present(scratch) {
        let _ = fs::remove_dir_all(scratch);
        panic!(
            "tmux is not runnable here, so the sweep's running-session filter cannot be \
             proven; install tmux or run this suite where one exists"
        );
    }
}

/// Write `body` at `path` and make it executable.
///
/// `fs::write` TRUNCATES through a symlink, and a session helper is a symlink
/// to the core — so this only ever runs against a fixture directory that has
/// none, and it creates the file rather than replacing one.
fn plant_script(path: &Path, body: &str) {
    let _ = fs::remove_file(path);
    assert!(fs::write(path, body).is_ok(), "the script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert!(
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).is_ok(),
            "the script must be executable"
        );
    }
}

/// A `say` that records every report it is handed and exits `code`.
///
/// The recording is what makes "delivered" a fact rather than an inference: the
/// sweep's own `delivered` field says what it BELIEVES, and the log says what
/// actually reached the operator's channel.
fn plant_say(dir: &Path, log: &Path, code: u8) {
    plant_script(
        &dir.join("say"),
        &format!(
            "#!/bin/sh\nprintf '%s\\n--\\n' \"$1\" >> {}\nexit {code}\n",
            log.display()
        ),
    );
}

fn said(log: &Path) -> String {
    fs::read_to_string(log).unwrap_or_default()
}

/// A session directory the product will discover, with one roster seat and the
/// event stream its attention is derived from.
fn plant_session(root: &Path, name: &str, socket: &Path, events: &[&str]) -> PathBuf {
    let dir = root.join("sessions").join(name);
    assert!(fs::create_dir_all(&dir).is_ok(), "a session meta dir");
    let meta = format!(
        "mode=local\nsession={name}\ntmux_server_kind=socket\ntmux_server={}\n\
         seat.main=lead\nprofile.main=cl\nagent_bin.main=claude\n",
        socket.display()
    );
    assert!(fs::write(dir.join("meta"), meta).is_ok(), "the record");
    let mut body = String::new();
    for line in events {
        body.push_str(line);
        body.push('\n');
    }
    assert!(
        fs::write(dir.join("events.jsonl"), body).is_ok(),
        "the event container"
    );
    dir
}

/// Bring `name` up on `socket` as a session ae will own.
///
/// The `AE_SESSION` marker is NOT decoration: a session tmux reports without it
/// classifies as `unknown`, never `running`, and the sweep only looks at running
/// ones. A fixture that skipped it would prove the sweep silent for the wrong
/// reason.
fn start_session(socket: &Path, scratch: &Path, name: &str) {
    assert!(
        tmux(
            socket,
            scratch,
            &["new-session", "-d", "-s", name, "sleep", "60"]
        )
        .0,
        "the session the sweep must see"
    );
    assert!(
        tmux(
            socket,
            scratch,
            &["set-environment", "-t", name, "AE_SESSION", name]
        )
        .0,
        "the ownership marker, without which the session reads as unknown"
    );
}

/// One agent declaring a work state, at a fixed stamp.
fn declared(agent: &str, state: &str) -> String {
    format!(r#"{{"ts":"2026-09-01T10:00:00Z","actor":"{agent}","action":"state","ref":"{state}"}}"#)
}

/// The moment every arm sweeps as of — fixed, so nothing here is a race.
const NOW: &str = "1788000000";

/// Run one sweep and return `(code, stdout, stderr)`.
fn sweep(root: &Path, dir: &Path, flags: &[&str]) -> (Option<i32>, String, String) {
    let out = ae()
        .env("AE_HOME", root)
        .arg(ae::cli::MONITOR)
        .arg(ae::monitor::SWEEP)
        .arg(dir)
        .args(["--now", NOW])
        .args(flags)
        .output();
    // Not `expect`: clippy's allow-*-in-tests covers `#[test]` bodies, and this
    // is a helper beside them.
    let Ok(out) = out else {
        panic!("the ae binary should run")
    };
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The one JSON document a `--format json` sweep prints.
fn document(stdout: &str) -> ae::json::Value {
    match ae::json::parse(stdout.trim()) {
        Ok(doc) => doc,
        Err(why) => panic!("the sweep prints one JSON document: {why}: {stdout}"),
    }
}

/// The report lines a `--format json` sweep produced.
fn report(stdout: &str) -> Vec<String> {
    let doc = document(stdout);
    let Some(ae::json::Value::Arr(lines)) = doc.get("report") else {
        panic!("report must be an array: {stdout}");
    };
    lines
        .iter()
        .map(|line| line.as_str().unwrap_or_default().to_owned())
        .collect()
}

/// Whether that sweep believes its report was delivered.
fn delivered(stdout: &str) -> bool {
    document(stdout).get("delivered") == Some(&ae::json::Value::Bool(true))
}

#[test]
fn a_repeated_attention_state_is_reported_once_and_its_change_is_reported_again() {
    // THE DEDUP CONTRACT. An agent that is blocked is worth one message; an
    // agent that is STILL blocked on the next sweep is worth none. The failure
    // this pins is the one the orchestrator had before the helper existed:
    // every five minutes, the same alert.
    let scratch = scratch("dedup");
    require_tmux(&scratch);
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let log = scratch.join("said");
    let dir = plant_session(&root, "mondd", &socket, &[&declared("lead", "blocked")]);
    plant_say(&dir, &log, 0);
    start_session(&socket, &scratch, "mondd");

    // A first run reports the attention it finds — that half is NOT suppressed,
    // only the fleet inventory is.
    let (code, out, err) = sweep(&root, &dir, &["--format", "json"]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(
        report(&out),
        vec!["⚠ mondd · lead needs you: blocked".to_owned()],
        "the first sweep's report"
    );
    assert!(delivered(&out), "say exited zero: {err}");
    assert_eq!(said(&log), "⚠ mondd · lead needs you: blocked\n--\n");

    // Nothing changed. Nothing is said.
    let (code, out, err) = sweep(&root, &dir, &["--format", "json"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        report(&out).is_empty(),
        "an unchanged attention must not be reported twice: {out}"
    );
    assert!(!delivered(&out), "there was nothing to deliver: {out}");
    assert_eq!(
        said(&log),
        "⚠ mondd · lead needs you: blocked\n--\n",
        "say must not have been run a second time"
    );

    // The reason CHANGES — that is a new fact, and it names the old one.
    assert!(
        fs::write(
            dir.join("events.jsonl"),
            format!("{}\n", declared("lead", "waiting-user"))
        )
        .is_ok(),
        "the changed declaration"
    );
    let (code, out, err) = sweep(&root, &dir, &["--format", "json"]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(
        report(&out),
        vec!["⚠ mondd · lead now: waiting-user (was blocked)".to_owned()],
        "a changed reason is a new report, and it carries the previous one"
    );

    // The attention GOES AWAY while the session keeps running — an all-clear,
    // exactly once.
    assert!(
        fs::write(
            dir.join("events.jsonl"),
            format!("{}\n", declared("lead", "working"))
        )
        .is_ok(),
        "the cleared declaration"
    );
    let (_, out, err) = sweep(&root, &dir, &["--format", "json"]);
    assert_eq!(
        report(&out),
        vec!["✓ mondd · lead cleared".to_owned()],
        "{err}"
    );
    let (_, out, _) = sweep(&root, &dir, &["--format", "json"]);
    assert!(
        report(&out).is_empty(),
        "a delivered all-clear is said once: {out}"
    );
}

#[test]
fn a_report_that_was_not_delivered_is_reported_again_until_it_lands() {
    // THE GUARANTEE THE DEDUP IS ONLY SAFE UNDER. `last_seen` advances every
    // sweep; `notified` advances only after `say` exits zero. Without that
    // asymmetry the dedup would swallow the one alert whose delivery failed —
    // silently, and forever.
    let scratch = scratch("retry");
    require_tmux(&scratch);
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let log = scratch.join("said");
    let dir = plant_session(&root, "monrt", &socket, &[&declared("lead", "blocked")]);
    plant_say(&dir, &log, 1);
    start_session(&socket, &scratch, "monrt");

    let expected = vec!["⚠ monrt · lead needs you: blocked".to_owned()];
    for attempt in 1..=2 {
        let (code, out, err) = sweep(&root, &dir, &["--format", "json"]);
        assert_eq!(code, Some(0), "attempt {attempt}: {err}");
        assert_eq!(report(&out), expected, "attempt {attempt}: {out}");
        assert!(!delivered(&out), "attempt {attempt}: say exited non-zero");
        assert!(
            err.contains("say failed"),
            "attempt {attempt}: the failure must be loud: {err}"
        );
    }

    // The channel comes back. It lands, and then it stops.
    plant_say(&dir, &log, 0);
    let (_, out, err) = sweep(&root, &dir, &["--format", "json"]);
    assert_eq!(report(&out), expected, "{err}");
    assert!(delivered(&out), "{err}");
    let (_, out, _) = sweep(&root, &dir, &["--format", "json"]);
    assert!(
        report(&out).is_empty(),
        "once delivered, the same attention is silent: {out}"
    );
}

#[test]
fn the_state_round_trips_through_the_file_the_watchdog_reads_as_a_heartbeat() {
    // The state file is not private bookkeeping: the watchdog stats it to tell
    // an orchestrator that is LIVE from one that is still SWEEPING. So the name
    // is a two-party contract, and a sweep that wrote anywhere else would leave
    // the wedge check watching a file nobody writes.
    let scratch = scratch("state");
    require_tmux(&scratch);
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let log = scratch.join("said");
    let dir = plant_session(&root, "monst", &socket, &[&declared("lead", "blocked")]);
    plant_say(&dir, &log, 0);
    start_session(&socket, &scratch, "monst");

    let state = dir.join("meta-agent-state.json");
    assert!(
        !state.exists(),
        "the fixture must start without one, or the first run is not one"
    );
    let (code, _, err) = sweep(&root, &dir, &["--format", "json"]);
    assert_eq!(code, Some(0), "{err}");

    let text = fs::read_to_string(&state).expect("the sweep wrote its state");
    let parsed = ae::monitor::State::parse(&text).expect("its own reader takes it back");
    assert_eq!(parsed.last_sweep_at, 1_788_000_000, "the sweep's own `now`");
    assert_eq!(
        parsed.sessions.get("monst").copied(),
        Some(1),
        "the fleet baseline holds the running session and its agent count"
    );
    let key = format!("monst{}lead", '\u{1f}');
    let attn = parsed
        .attention
        .get(&key)
        .expect("attention is keyed per agent");
    assert_eq!(attn.reason, "blocked");
    assert_eq!(attn.rank, ae::attention::Reason::Blocked.rank());
    assert_eq!(
        (attn.first_seen, attn.last_seen),
        (1_788_000_000, 1_788_000_000)
    );
    assert!(attn.notified, "it was delivered, so it is marked delivered");
    assert!(!attn.cleared);

    // Rendering what was read back reproduces the document — the round trip
    // that makes the next sweep's diff a comparison of like with like.
    assert_eq!(parsed.render(), text, "render(parse(x)) == x");

    // It is the operator's business and nobody else's.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(&state)
            .expect("the state file")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the state file names the whole fleet");
    }

    // A CORRUPT state file is a first run, not a crash and not a misread: the
    // sweep reports its attention again rather than trusting half a document.
    assert!(
        fs::write(&state, "{not json at all").is_ok(),
        "the corruption"
    );
    let (code, out, err) = sweep(&root, &dir, &["--format", "json"]);
    assert_eq!(
        code,
        Some(0),
        "a corrupt state must not fail the sweep: {err}"
    );
    assert_eq!(
        report(&out),
        vec!["⚠ monst · lead needs you: blocked".to_owned()],
        "a corrupt state starts clean"
    );
}

#[test]
fn a_dry_run_previews_the_report_and_changes_nothing() {
    let scratch = scratch("dry");
    require_tmux(&scratch);
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let log = scratch.join("said");
    let dir = plant_session(&root, "mondr", &socket, &[&declared("lead", "blocked")]);
    plant_say(&dir, &log, 0);
    start_session(&socket, &scratch, "mondr");

    // Text format, because that is what a human previewing runs.
    let (code, out, err) = sweep(&root, &dir, &["--dry-run"]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(out, "⚠ mondr · lead needs you: blocked\n");
    assert!(
        !dir.join("meta-agent-state.json").exists(),
        "no state written"
    );
    assert_eq!(said(&log), "", "a dry run delivers nothing");

    // `--init` seeds the same snapshot SILENTLY — the first-install path, so a
    // fresh orchestrator does not announce a fleet already running.
    let (code, out, err) = sweep(&root, &dir, &["--init"]);
    assert_eq!((code, out.as_str()), (Some(0), ""), "{err}");
    assert_eq!(said(&log), "", "--init says nothing");
    let (_, out, _) = sweep(&root, &dir, &["--format", "json"]);
    assert!(
        report(&out).is_empty(),
        "a seeded attention is already known: {out}"
    );
}

#[test]
fn no_notify_prints_without_delivering_and_without_marking_anything_notified() {
    let scratch = scratch("quietly");
    require_tmux(&scratch);
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let log = scratch.join("said");
    let dir = plant_session(&root, "monnn", &socket, &[&declared("lead", "blocked")]);
    plant_say(&dir, &log, 0);
    start_session(&socket, &scratch, "monnn");

    let (code, out, err) = sweep(&root, &dir, &["--no-notify"]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(out, "⚠ monnn · lead needs you: blocked\n");
    assert_eq!(said(&log), "", "--no-notify runs no helper at all");

    // An UNCONFIRMED report is not a delivered one: the next sweep says it
    // again. This is the same rule as a failed send, reached the other way.
    let (_, out, _) = sweep(&root, &dir, &["--no-notify"]);
    assert_eq!(out, "⚠ monnn · lead needs you: blocked\n");
}

#[test]
fn the_sweep_command_the_charter_prints_is_the_one_the_binary_accepts() {
    // The charter is INSTRUCTION, not documentation: the orchestrator runs the
    // command in its fence verbatim. A rename here that did not reach that
    // fence would leave every installed orchestrator running a word the binary
    // refuses — silently, because nobody reads a charter after installing it.
    let charter = Path::new(env!("CARGO_MANIFEST_DIR")).join("contrib/aeorchestrator/CHARTER.md");
    let text = fs::read_to_string(&charter).expect("the charter ships with the repo");
    let quoted = text
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("ae ") && line.contains(ae::cli::MONITOR))
        .unwrap_or_else(|| panic!("the charter must print the sweep command"));
    assert_eq!(
        quoted,
        format!(
            "ae {} {} __HELPERS_DIR__",
            ae::cli::MONITOR,
            ae::monitor::SWEEP
        ),
        "the charter's fence and the product's argv are one contract"
    );

    // And it RUNS: the same argv, with the placeholder resolved.
    let scratch = scratch("charter");
    require_tmux(&scratch);
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let log = scratch.join("said");
    let dir = plant_session(&root, "moncc", &socket, &[&declared("lead", "blocked")]);
    plant_say(&dir, &log, 0);
    start_session(&socket, &scratch, "moncc");

    let words: Vec<&str> = quoted.split_whitespace().skip(1).collect();
    let out = ae()
        .env("AE_HOME", &root)
        .args(&words[..words.len() - 1])
        .arg(&dir)
        .output()
        .expect("the ae binary should run");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "⚠ moncc · lead needs you: blocked\n",
        "the charter's own command, run: {stderr}"
    );
    assert_eq!(said(&log), "⚠ moncc · lead needs you: blocked\n--\n");
}

#[test]
fn a_subcommand_or_flag_the_sweep_does_not_know_is_a_usage_error() {
    // `2` and not `1`: "you asked wrong" is a different answer from "it went
    // wrong", and a cadence that silently did nothing on a typo is the failure.
    for tail in [
        vec![ae::cli::MONITOR, "sweeep", "/nowhere"],
        vec![
            ae::cli::MONITOR,
            ae::monitor::SWEEP,
            "/nowhere",
            "--frobnicate",
        ],
        vec![
            ae::cli::MONITOR,
            ae::monitor::SWEEP,
            "/nowhere",
            "--format",
            "yaml",
        ],
        vec![ae::cli::MONITOR, ae::monitor::SWEEP, "/nowhere", "--now"],
        vec![ae::cli::MONITOR, ae::monitor::SWEEP],
    ] {
        let out = ae().args(&tail).output().expect("the ae binary should run");
        assert_eq!(out.status.code(), Some(2), "{tail:?}: {:?}", out.status);
        assert!(out.stdout.is_empty(), "{tail:?}: stdout must stay empty");
    }
}
