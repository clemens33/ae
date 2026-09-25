//! The two daemons' LIFECYCLE, against a real tmux server.
//!
//! `_watchdog` and `_telegram` start and stop long-lived processes, and the
//! whole risk is in what the guards do with a real
//! server's answers: a `start` that cannot see a running daemon spawns a second
//! one, and a `stop` that cannot see a pane reports a kill it did not perform.
//! So both arms drive the product entries against a live server rather than a
//! double.
//!
//! No bridge is ever spawned here. The Telegram arm plants a tmux session under
//! the bridge's own name, which is exactly what the liveness check looks for —
//! proving `start`'s idempotence and `stop`'s kill without a process that would
//! long-poll a real API with a fake token.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::fs;
use std::path::{Path, PathBuf};

use ae::telegram::bridge::Paths;

use super::parity::Invocation;
use super::parity::capture::raw;
use super::phase2::{run_tmux, tmux_present};

/// A scratch dir short enough to hold a socket path — `sun_path` is 104 bytes
/// on macOS and the usual temp dir eats most of it.
fn scratch(tag: &str) -> PathBuf {
    super::cli::OwnedScratch::root("dmn", tag).keep()
}

fn socket_of(scratch: &Path) -> PathBuf {
    scratch.join("s")
}

/// Kill the arm's server and remove its scratch, WHATEVER ended the arm.
struct Cleanup {
    socket: PathBuf,
    scratch: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        // NOT the panicking `tmux` helper: this runs while a panic may already
        // be unwinding, and a second panic there aborts the whole process —
        // taking the failure report with it.
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
            "tmux is not runnable here, so the daemon lifecycle's real-server arms cannot be \
             proven; install tmux or run this suite where one exists"
        );
    }
}

/// Write `body` at `path` and make it executable.
fn plant_script(path: &Path, body: &str) {
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

/// A session meta dir naming `socket` as its server, with the two helpers the
/// start path runs: a `watchdog` that publishes a pidfile and then stays alive,
/// and an `events-tail` for the monitor window.
fn plant_session(root: &Path, session: &str, socket: &Path) -> PathBuf {
    let meta_dir = root.join("sessions").join(session);
    assert!(fs::create_dir_all(&meta_dir).is_ok(), "a session meta dir");
    let meta = format!(
        "mode=local\nsession={session}\ntmux_server_kind=socket\ntmux_server={}\n\
         seat.main=lead\nprofile.main=cl\nagent_bin.main=claude\n",
        socket.display()
    );
    assert!(fs::write(meta_dir.join("meta"), meta).is_ok(), "the record");
    plant_script(
        &meta_dir.join("watchdog"),
        "#!/bin/sh\n\
         d=$(cd \"$(dirname \"$0\")\" && pwd)\n\
         printf '%s\\n' \"$$\" > \"$d/.watchdog.pid.staged\"\n\
         mv \"$d/.watchdog.pid.staged\" \"$d/.watchdog.pid\"\n\
         exec sleep 60\n",
    );
    plant_script(&meta_dir.join("events-tail"), "#!/bin/sh\nexec sleep 60\n");
    meta_dir
}

/// Run one product entry and return `(code, stdout, stderr)`.
fn watchdog(root: &Path, words: &[&str]) -> (u8, String, String) {
    let tail: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = match ae::watchdog_lifecycle::run(root, &tail, &mut out, &mut err) {
        Ok(code) => code,
        Err(why) => panic!("the entry writes to in-memory buffers: {why}"),
    };
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

fn telegram(ae_home: &Path, words: &[&str]) -> (u8, String, String) {
    let tail: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = match ae::telegram_lifecycle::run(ae_home, &tail, &mut out, &mut err) {
        Ok(code) => code,
        Err(why) => panic!("the entry writes to in-memory buffers: {why}"),
    };
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

#[test]
fn a_watchdog_starts_once_reports_its_pid_and_stops_with_its_pane() {
    let scratch = scratch("wd");
    require_tmux(&scratch);
    let socket = socket_of(&scratch);
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let meta_dir = plant_session(&root, "wdlife", &socket);
    assert!(
        tmux(
            &socket,
            &scratch,
            &["new-session", "-d", "-s", "wdlife", "sleep", "60"]
        )
        .0,
        "the session the watchdog watches"
    );

    // BEFORE: nothing published, nothing running.
    let (code, out, _) = watchdog(&root, &["status", "wdlife"]);
    assert_eq!((code, out.trim()), (0, "Watchdog is not running."));

    let (code, out, err) = watchdog(&root, &["start", "wdlife"]);
    assert_eq!(code, 0, "the start failed: {err}");
    assert!(
        out.contains("Watchdog started in hidden ae-monitor window"),
        "unexpected start output: {out}"
    );
    let pid = ae::watchdog_glue::read_pid(&meta_dir).expect("the daemon published a pidfile");

    // The pane carries the stamp every later decision keys on.
    let (_, panes) = tmux(
        &socket,
        &scratch,
        &["list-panes", "-s", "-t", "wdlife", "-F", "#{@ae_agent}"],
    );
    assert!(
        panes.lines().any(|line| line == "_watchdog"),
        "no stamped watchdog pane: {panes:?}"
    );

    let (code, out, _) = watchdog(&root, &["status", "wdlife"]);
    assert_eq!(
        (code, out.trim()),
        (0, format!("Watchdog is running (pid {pid}).").as_str())
    );

    // IDEMPOTENCE: a second start does not spawn a second daemon.
    let (code, out, _) = watchdog(&root, &["start", "wdlife"]);
    assert_eq!(code, 0);
    assert!(
        out.contains(&format!("Watchdog is already running (pid {pid}).")),
        "a second start did not defer: {out}"
    );
    assert_eq!(ae::watchdog_glue::read_pid(&meta_dir), Some(pid));

    let (code, out, err) = watchdog(&root, &["stop", "wdlife"]);
    assert_eq!(code, 0, "the stop failed: {err}");
    assert_eq!(out.trim(), "Watchdog stopped.");
    assert_eq!(
        ae::watchdog_glue::read_pid(&meta_dir),
        None,
        "the registration outlived the daemon"
    );
    let (_, panes) = tmux(
        &socket,
        &scratch,
        &["list-panes", "-s", "-t", "wdlife", "-F", "#{@ae_agent}"],
    );
    assert!(
        !panes.lines().any(|line| line == "_watchdog"),
        "the watchdog pane outlived the stop: {panes:?}"
    );

    // AFTER: a second stop is not an error, and status agrees with it.
    let (code, out, _) = watchdog(&root, &["stop", "wdlife"]);
    assert_eq!((code, out.trim()), (0, "Watchdog is not running."));
    let (code, out, _) = watchdog(&root, &["status", "wdlife"]);
    assert_eq!((code, out.trim()), (0, "Watchdog is not running."));
}

/// A stop typed in a plain shell whose locale is not UTF-8 still kills the
/// pane (#187). Outside tmux, with no UTF-8 locale, a tmux client is not UTF-8,
/// and the server sanitizes what it prints to that client; the ownership probe
/// used to come back unreadable and the kill was refused.
#[test]
fn a_stop_from_a_shell_without_a_utf8_locale_still_kills_the_watchdog_pane() {
    let scratch = scratch("wdposix");
    require_tmux(&scratch);
    let socket = socket_of(&scratch);
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let meta_dir = plant_session(&root, "wdposix", &socket);
    let (ok, _) = tmux(
        &socket,
        &scratch,
        &["new-session", "-d", "-s", "wdposix", "sleep", "60"],
    );
    assert!(ok, "the session the watchdog watches");
    let (code, _, err) = watchdog(&root, &["start", "wdposix"]);
    assert_eq!(code, 0, "the start failed: {err}");

    // The binary, not the in-process entry: the locale must reach tmux's own
    // environment, and only a child process can carry one. `ae()` strips
    // `$TMUX`, which would otherwise make the client UTF-8 by itself.
    let mut stop = super::cli::ae();
    stop.env("AE_HOME", &root)
        .env("LC_ALL", "C")
        .env_remove("LC_CTYPE")
        .env_remove("LANG")
        .args(["watchdog", "stop", "wdposix"]);
    let out = super::cli::bounded(
        stop.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the ae binary runs"),
        std::time::Duration::from_secs(30),
    )
    .expect("the stop returned");
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(stdout.trim(), "Watchdog stopped.");
    assert!(stderr.is_empty(), "the stop refused something: {stderr}");
    assert_eq!(ae::watchdog_glue::read_pid(&meta_dir), None);
    let (_, panes) = tmux(
        &socket,
        &scratch,
        &["list-panes", "-s", "-t", "wdposix", "-F", "#{@ae_agent}"],
    );
    assert!(
        !panes.lines().any(|line| line == "_watchdog"),
        "the watchdog pane outlived the stop: {panes:?}"
    );
}

/// A stop over a refused kill keeps the pidfile and exits 1 (#194). The
/// linked pane reads as ours in the listing (presence is Running) but as
/// theirs in the ownership probe, so the kill refuses over a live daemon.
#[test]
fn a_stop_over_a_refused_kill_keeps_its_pidfile_and_exits_1() {
    let scratch = scratch("wdref");
    require_tmux(&scratch);
    let socket = socket_of(&scratch);
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let meta_dir = plant_session(&root, "ours", &socket);
    let run = |words: &[&str]| tmux(&socket, &scratch, words);
    let pane = super::refusal_rig::linked(&run, "ours", "theirs", "ae-monitor", "_watchdog");
    super::refusal_rig::own(&run, "ours", &root);
    // The pane's own live pid makes presence Running.
    let pid = super::refusal_rig::pid_of(&run, &pane);
    assert!(
        fs::write(meta_dir.join(".watchdog.pid"), format!("{pid}\n")).is_ok(),
        "a live pidfile"
    );

    let (code, out, err) = watchdog(&root, &["stop", "ours"]);

    let panes = tmux(&socket, &scratch, &["list-panes", "-a", "-F", "#{pane_id}"]).1;
    let events = events_of(&meta_dir);
    let meta = fs::read_to_string(meta_dir.join("meta")).unwrap_or_default();
    let (shown, _) = tmux(
        &socket,
        &scratch,
        &["show-options", "-v", "-t", "ours", "@ae_watchdog_status"],
    );
    assert_eq!(code, 1, "a refused stop exits 1: {out} {err}");
    assert!(
        !out.contains("Watchdog stopped."),
        "no success over a live daemon: {out}"
    );
    assert!(
        err.contains(&format!("refusing to kill pane {pane}"))
            && err.contains(&format!("pane {pane} could not be killed")),
        "both lines name the pane: {err}"
    );
    assert!(panes.contains(&pane), "the refused pane is alive: {panes}");
    assert_eq!(
        fs::read_to_string(meta_dir.join(".watchdog.pid")).unwrap_or_default(),
        format!("{pid}\n"),
        "the pidfile still names the live daemon"
    );
    assert!(
        events
            .iter()
            .any(|line| line.contains("watchdog-stop") && line.contains("refused:")),
        "one refused audit: {events:?}"
    );
    assert!(
        !meta.contains("watchdog=false"),
        "meta keeps no false flag: {meta}"
    );
    assert!(!shown, "no watchdog-off seed over a running daemon");
}

/// A legacy watchdog the reap found but could not kill counts as refused,
/// never as stopped (#194). The refused audit AND err name the pane id and
/// its short reason.
#[test]
fn a_stop_with_a_legacy_pane_it_could_not_kill_refuses() {
    let scratch = scratch("wdlegref");
    require_tmux(&scratch);
    let socket = socket_of(&scratch);
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let meta_dir = plant_session(&root, "ours", &socket);
    let run = |words: &[&str]| tmux(&socket, &scratch, words);
    let pane = super::refusal_rig::linked(&run, "ours", "theirs", "ae-monitor", "_shepherd");
    // No pidfile: no running main daemon.

    let (code, out, err) = watchdog(&root, &["stop", "ours"]);

    let panes = tmux(&socket, &scratch, &["list-panes", "-a", "-F", "#{pane_id}"]).1;
    let events = events_of(&meta_dir);
    assert_eq!(code, 1, "a refused legacy reap exits 1: {out} {err}");
    assert!(
        !out.contains("Watchdog stopped."),
        "a found-but-live legacy is not stopped: {out}"
    );
    assert!(
        err.contains(&pane) && err.contains("it belongs to session"),
        "err names the pane id and its short reason: {err}"
    );
    assert!(
        events.iter().any(|line| line.contains("watchdog-stop")
            && line.contains("refused:")
            && line.contains(&pane)
            && line.contains("it belongs to session")),
        "the refused audit names the pane id and its short reason: {events:?}"
    );
    assert!(panes.contains(&pane), "the legacy pane is alive: {panes}");
}

/// A start that found a legacy watchdog it could not take aborts before it
/// spawns anything: exit 1, a `refused:` audit, and no `_watchdog` pane
/// beside the live legacy one.
#[test]
fn a_start_with_a_legacy_pane_it_could_not_kill_aborts() {
    let scratch = scratch("wdlegstart");
    require_tmux(&scratch);
    let socket = socket_of(&scratch);
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let meta_dir = plant_session(&root, "ours", &socket);
    let run = |words: &[&str]| tmux(&socket, &scratch, words);
    let pane = super::refusal_rig::linked(&run, "ours", "theirs", "ae-monitor", "_shepherd");

    let (code, out, err) = watchdog(&root, &["start", "ours"]);

    let stamps = tmux(
        &socket,
        &scratch,
        &["list-panes", "-s", "-t", "ours", "-F", "#{@ae_agent}"],
    )
    .1;
    let events = events_of(&meta_dir);
    assert_eq!(
        code, 1,
        "a refused legacy reap aborts the start: {out} {err}"
    );
    assert!(
        err.contains("a legacy watchdog of")
            && err.contains(&pane)
            && err.contains("it belongs to session"),
        "err names the pane and its short reason: {err}"
    );
    assert!(
        events.iter().any(|line| line.contains("watchdog-start")
            && line.contains("refused:")
            && line.contains(&pane)),
        "one refused audit naming the legacy pane: {events:?}"
    );
    assert!(
        !stamps.lines().any(|line| line == "_watchdog"),
        "no watchdog pane spawned beside the live legacy: {stamps:?}"
    );
}

/// I2: a refused legacy kill does not lock out the main stop. Each
/// registration follows its own verdict (the main pane dies, its pidfile is
/// cleared), while the session facts wait for zero refusals: exit 1, no
/// "Watchdog stopped.", no off seed.
#[test]
fn a_stop_with_a_refused_legacy_kill_still_stops_a_killable_main() {
    let scratch = scratch("wdlegmain");
    require_tmux(&scratch);
    let socket = socket_of(&scratch);
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let meta_dir = plant_session(&root, "ours", &socket);
    let run = |words: &[&str]| tmux(&socket, &scratch, words);
    let legacy = super::refusal_rig::linked(&run, "ours", "theirs", "ae-monitor", "_shepherd");
    super::refusal_rig::own(&run, "ours", &root);
    // A live main watchdog, built by hand (a lifecycle start would abort on
    // the refused legacy reap).
    let (main, pid) = super::refusal_rig::stamped(&run, "ours", "_watchdog");
    assert!(
        fs::write(meta_dir.join(".watchdog.pid"), format!("{pid}\n")).is_ok(),
        "a live pidfile"
    );

    let (code, out, err) = watchdog(&root, &["stop", "ours"]);

    let panes = tmux(&socket, &scratch, &["list-panes", "-a", "-F", "#{pane_id}"]).1;
    let events = events_of(&meta_dir);
    let meta = fs::read_to_string(meta_dir.join("meta")).unwrap_or_default();
    let (shown, _) = tmux(
        &socket,
        &scratch,
        &["show-options", "-v", "-t", "ours", "@ae_watchdog_status"],
    );
    assert_eq!(code, 1, "any refusal exits 1: {out} {err}");
    assert!(
        !out.contains("Watchdog stopped."),
        "no stopped line with a refusal outstanding: {out}"
    );
    assert!(
        !panes.contains(&main),
        "the killable main pane is gone: {panes}"
    );
    assert!(panes.contains(&legacy), "the legacy pane lives: {panes}");
    assert!(
        fs::read_to_string(meta_dir.join(".watchdog.pid")).is_err(),
        "the main pidfile is cleared with its daemon"
    );
    assert!(
        events.iter().any(|line| line.contains("watchdog-stop")
            && line.contains("refused:")
            && line.contains(&legacy)),
        "one refused audit naming the legacy pane: {events:?}"
    );
    assert!(
        !meta.contains("watchdog=false"),
        "meta keeps no false flag: {meta}"
    );
    assert!(!shown, "no watchdog-off seed with a refusal outstanding");
}

/// Positive control for the no-seed asserts above: in this same rig shape, a
/// stop that settles DOES publish the watchdog-off seed, so those asserts
/// would fail on the buggy path.
#[test]
fn a_settling_stop_seeds_the_session_it_watched() {
    let scratch = scratch("wdseedctl");
    require_tmux(&scratch);
    let socket = socket_of(&scratch);
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let meta_dir = plant_session(&root, "ours", &socket);
    assert!(
        tmux(
            &socket,
            &scratch,
            &["new-session", "-d", "-s", "ours", "sleep", "60"]
        )
        .0,
        "the session the watchdog watches"
    );
    let run = |words: &[&str]| tmux(&socket, &scratch, words);
    super::refusal_rig::own(&run, "ours", &root);
    let (_, pid) = super::refusal_rig::stamped(&run, "ours", "_watchdog");
    assert!(
        fs::write(meta_dir.join(".watchdog.pid"), format!("{pid}\n")).is_ok(),
        "a live pidfile"
    );

    let (code, out, err) = watchdog(&root, &["stop", "ours"]);
    assert_eq!(code, 0, "the settling stop: {out} {err}");
    let (shown, value) = tmux(
        &socket,
        &scratch,
        &[
            "show-options",
            "-v",
            "-t",
            "ours",
            ae::tmux::WATCHDOG_STATUS_OPTION,
        ],
    );
    assert!(
        shown && value.contains("watchdog off"),
        "the seed is visible: rc={shown} {value:?}"
    );
}

/// The audit ledger's lines, oldest first.
fn events_of(meta_dir: &Path) -> Vec<String> {
    match fs::read_to_string(meta_dir.join("events.jsonl")) {
        Ok(body) => body.lines().map(str::to_owned).collect(),
        Err(why) => panic!("the audit ledger exists: {why}"),
    }
}

/// Every start and stop leaves exactly one audit record — through the one
/// writer, with no target, and a read-only `status` leaves none.
#[test]
fn a_start_and_a_stop_each_leave_exactly_one_audit_record() {
    let scratch = scratch("wdaudit");
    require_tmux(&scratch);
    let socket = socket_of(&scratch);
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let meta_dir = plant_session(&root, "wdaudit", &socket);
    let (ok, _) = tmux(
        &socket,
        &scratch,
        &["new-session", "-d", "-s", "wdaudit", "sleep", "60"],
    );
    assert!(ok, "the session the watchdog watches");

    let (code, _, err) = watchdog(&root, &["start", "wdaudit"]);
    assert_eq!(code, 0, "the start failed: {err}");
    let pid = ae::watchdog_glue::read_pid(&meta_dir).expect("the published pidfile");
    let (code, _, _) = watchdog(&root, &["start", "wdaudit"]);
    assert_eq!(code, 0);
    let (code, _, err) = watchdog(&root, &["stop", "wdaudit"]);
    assert_eq!(code, 0, "the stop failed: {err}");
    let (code, _, _) = watchdog(&root, &["stop", "wdaudit"]);
    assert_eq!(code, 0);
    let (code, _, _) = watchdog(&root, &["status", "wdaudit"]);
    assert_eq!(code, 0);

    let lines = events_of(&meta_dir);
    assert_eq!(lines.len(), 4, "one record per start/stop, none for status");
    let already = format!("already running (pid {pid})");
    for (line, action, summary) in [
        (&lines[0], "watchdog-start", "started"),
        (&lines[1], "watchdog-start", already.as_str()),
        (&lines[2], "watchdog-stop", "stopped"),
        (&lines[3], "watchdog-stop", "not running"),
    ] {
        assert!(
            line.contains(&format!(r#""action":"{action}""#)),
            "missing action {action}: {line}"
        );
        // No `--pane` on these argv, so no calling pane is known.
        assert!(line.contains(r#""actor":"human""#), "{line}");
        assert!(
            line.contains(&format!(r#""summary":"{summary}""#)),
            "missing summary {summary}: {line}"
        );
        assert!(
            !line.contains(r#""target""#),
            "a target would un-quiet the named agent: {line}"
        );
        let event = ae::events::Event::parse_line(line).expect("the writer's JSON parses");
        assert_eq!(event.alert_meaning(), ae::events::AlertMeaning::Undefined);
    }
}

/// A refused start or stop is recorded too, and the refusal's exit code is
/// unchanged by the audit.
#[test]
fn a_refused_start_and_stop_are_recorded_without_changing_the_outcome() {
    // An append failure warns once and changes neither the outcome nor the
    // exit code: the ledger is a directory here, so the audit cannot land.
    let fail_scratch = scratch("wdfail");
    let fail_root = fail_scratch.join("home");
    let fail_dir = plant_session(&fail_root, "wdfail", &fail_scratch.join("no-such-socket"));
    assert!(fs::create_dir_all(fail_dir.join("events.jsonl")).is_ok());
    let (code, out, err) = watchdog(&fail_root, &["stop", "wdfail"]);
    assert_eq!((code, out.trim()), (0, "Watchdog is not running."));
    assert!(err.contains("ae: watchdog audit failed"), "{err}");
    let _ = fs::remove_dir_all(&fail_scratch);

    let scratch = scratch("wdref");
    let _cleanup = Cleanup {
        socket: socket_of(&scratch),
        scratch: scratch.clone(),
    };
    // A server that answers nothing, with a pidfile claiming a daemon: every
    // presence probe reads Unknown.
    let root = scratch.join("home");
    let meta_dir = plant_session(&root, "wdref", &scratch.join("no-such-socket"));
    assert!(
        fs::write(meta_dir.join(".watchdog.pid"), "424242\n").is_ok(),
        "a pidfile no server speaks for"
    );

    let (code, _, err) = watchdog(&root, &["stop", "wdref"]);
    assert_eq!(code, 1, "the refused stop keeps its exit code");
    assert!(err.contains("tmux did not answer"), "{err}");
    let (code, _, err) = watchdog(&root, &["start", "wdref"]);
    assert_eq!(code, 0, "the skipped start keeps its exit code");
    assert!(err.contains("start skipped"), "{err}");

    let lines = events_of(&meta_dir);
    assert_eq!(lines.len(), 2, "one refusal record per verb");
    for (line, action) in [(&lines[0], "watchdog-stop"), (&lines[1], "watchdog-start")] {
        assert!(
            line.contains(&format!(r#""action":"{action}""#)),
            "missing action {action}: {line}"
        );
        assert!(
            line.contains("refused: tmux did not answer"),
            "the outcome says refused: {line}"
        );
        assert!(!line.contains(r#""target""#), "{line}");
    }
}

/// The `wdseed` session as a LAUNCH and a live daemon would have left it.
///
/// The ownership pair the seed is proven against, the look the seed is rendered
/// in, and the published facts a stop has to take back — including an attention
/// verdict that is NOT the seed, so a stop that merely left the last one
/// standing fails the arm too.
fn plant_watched_session(socket: &Path, scratch: &Path, root: &Path, look: &ae::theme::Look) {
    let set = |flag: &str, name: &str, value: &str| {
        assert!(
            tmux(socket, scratch, &[flag, "-t", "wdseed", name, value]).0,
            "{name} must be plantable"
        );
    };
    set("set-environment", "AE_SESSION", "1");
    set("set-environment", "AE_HOME", &root.display().to_string());
    for (option, value) in ae::theme::fact_options(look, "/work") {
        set("set-option", &option, &value);
    }
    let needs_you = ae::theme::Mark::NeedsYou;
    for (option, value) in [
        (
            ae::theme::ATTENTION_RANK_OPTION,
            needs_you.rank().to_string(),
        ),
        (
            ae::theme::ATTENTION_GLYPH_OPTION,
            needs_you.glyph(true).to_owned(),
        ),
        (ae::theme::ATTENTION_STYLE_OPTION, "fg=red".to_owned()),
        (
            ae::theme::AGENTS_OPTION,
            "v1;2000;60;lead:cl:working:%1".to_owned(),
        ),
        (ae::theme::FLEET_STRIP_OPTION, "strip".to_owned()),
        (ae::theme::GOAL_OPTION, "ship".to_owned()),
        // A DUMMY version, deliberately not a CalVer this repo will ever carry:
        // the arm asserts the option is cleared, never what it held.
        (ae::theme::VERSION_OPTION, "ae 0.0.0-dummy".to_owned()),
        (
            ae::tmux::WATCHDOG_STATUS_OPTION,
            "#[fg=green]watching".to_owned(),
        ),
    ] {
        set("set-option", option, &value);
    }
}

/// A STOPPED watchdog leaves its session listed, and says why.
///
/// The regression: the retraction unset the attention rank along with
/// everything else, and every fleet reader drops a rankless row — so a session
/// that was still running vanished from every other session's strip and from
/// the picker while `ae list` went on calling it running, with nothing on its
/// own bar to say what had happened.
#[test]
fn a_stopped_watchdog_leaves_the_running_session_seeded_and_listed() {
    let scratch = scratch("wdseed");
    require_tmux(&scratch);
    let socket = socket_of(&scratch);
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let _meta_dir = plant_session(&root, "wdseed", &socket);
    assert!(
        tmux(
            &socket,
            &scratch,
            &["new-session", "-d", "-s", "wdseed", "sleep", "60"]
        )
        .0,
        "the session the watchdog watches"
    );

    let look = ae::theme::Look::DEFAULT;
    plant_watched_session(&socket, &scratch, &root, &look);

    let (code, _, err) = watchdog(&root, &["start", "wdseed"]);
    assert_eq!(code, 0, "the start failed: {err}");
    let (code, out, err) = watchdog(&root, &["stop", "wdseed"]);
    assert_eq!(code, 0, "the stop failed: {err}");
    assert_eq!(out.trim(), "Watchdog stopped.");

    let read = |name: &str| {
        tmux(
            &socket,
            &scratch,
            &["show-options", "-v", "-t", "wdseed", name],
        )
        .1
        .trim()
        .to_owned()
    };
    // THE SEED, exactly — the same three values a launch writes.
    for (option, value) in ae::theme::seed_options(&look) {
        assert_eq!(
            read(&option),
            value,
            "{option} must be the launch seed once nothing is measuring the session"
        );
    }
    // And nothing a live daemon vouched for survives it.
    for option in [
        ae::theme::AGENTS_OPTION,
        ae::theme::FLEET_STRIP_OPTION,
        ae::theme::GOAL_OPTION,
        ae::theme::VERSION_OPTION,
    ] {
        assert!(
            read(option).is_empty(),
            "{option} outlived the daemon that published it"
        );
    }
    // The session's own bar says why it is not being measured.
    assert_eq!(
        read(ae::tmux::WATCHDOG_STATUS_OPTION),
        ae::theme::watchdog_off_segment(&look),
        "a stopped watchdog leaves a truthful health segment, not a stale one"
    );

    // THE POINT: a peer reading the server still finds the row.
    let (listed, listing) = tmux(
        &socket,
        &scratch,
        &["list-sessions", "-F", ae::tmux::FLEET_SESSION_FORMAT],
    );
    let rows = ae::tmux::interpret_fleet_sessions(listed, &listing).unwrap_or_default();
    assert!(
        rows.iter().any(
            |row| row.name == "wdseed" && row.rank == ae::theme::Mark::Stale.rank().to_string()
        ),
        "a running session whose watchdog stopped must stay on every peer's strip: {rows:?}"
    );

    // A watchdog that had ALREADY died leaves the same truthful bar. Clobber
    // the seed with a frozen verdict, as a dead daemon's last cycle would have,
    // and stop again: this stop finds nothing to kill, so a seed scoped to the
    // arm that kills one would leave the lie standing.
    let set = |name: &str, value: &str| {
        assert!(
            tmux(
                &socket,
                &scratch,
                &["set-option", "-t", "wdseed", name, value]
            )
            .0
        );
    };
    set(ae::theme::ATTENTION_RANK_OPTION, "4");
    set(ae::tmux::WATCHDOG_STATUS_OPTION, "#[fg=green]watching");
    let (code, out, err) = watchdog(&root, &["stop", "wdseed"]);
    assert_eq!((code, out.trim()), (0, "Watchdog is not running."), "{err}");
    assert_eq!(
        read(ae::theme::ATTENTION_RANK_OPTION),
        ae::theme::Mark::Stale.rank().to_string(),
        "a stop that finds no daemon still hands a frozen verdict back to the seed"
    );
    assert_eq!(
        read(ae::tmux::WATCHDOG_STATUS_OPTION),
        ae::theme::watchdog_off_segment(&look),
        "and still says nothing is measuring the session"
    );
}

#[test]
fn the_watchdog_entry_refuses_a_session_it_cannot_name_or_find() {
    let scratch = scratch("wdref");
    let root = scratch.join("home");
    assert!(fs::create_dir_all(root.join("sessions")).is_ok());

    // No name, and no pane to derive one from.
    let (code, _, err) = watchdog(&root, &["status"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("no session name given and not inside an ae tmux session"),
        "unexpected refusal: {err}"
    );

    // A name that is not a session.
    let (code, _, err) = watchdog(&root, &["status", "nope"]);
    assert_eq!(code, 1);
    assert!(err.contains("session 'nope' not found"), "{err}");

    // A name that is not a NAME.
    let (code, _, err) = watchdog(&root, &["status", "../../etc"]);
    assert_eq!(code, 1);
    assert!(err.contains("not found"), "{err}");

    // An action that is not one.
    let (code, _, err) = watchdog(&root, &["restart", "nope"]);
    assert_eq!(code, 2, "a usage error is 2, not 1");
    assert!(err.contains("start|stop|status"), "{err}");

    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn telegram_start_is_idempotent_status_reports_it_and_stop_kills_the_bridge() {
    let scratch = scratch("tg");
    require_tmux(&scratch);
    let socket = socket_of(&scratch);
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let ae_home = scratch.join("home");
    assert!(fs::create_dir_all(&ae_home).is_ok());
    let token = ae_home.join("token");
    assert!(fs::write(&token, "123456:fake-token\n").is_ok());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert!(fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).is_ok());
    }
    let config = ae_home.join("config");
    assert!(
        fs::write(
            &config,
            format!(
                "[workspace]\nmain = lead\n\n[telegram]\ntoken_file = {}\nchat_id = 7\ninclude = state\n",
                token.display()
            ),
        )
        .is_ok(),
        "the config"
    );
    let server = [
        "--server-kind",
        "socket",
        "--server",
        &socket.display().to_string(),
    ]
    .map(ToOwned::to_owned);

    // The bridge's own session, planted rather than spawned: the liveness check
    // is an exact name match over `list-sessions`, so this is what a running
    // bridge looks like to every one of these commands.
    assert!(
        tmux(
            &socket,
            &scratch,
            &["new-session", "-d", "-s", "ae-telegram", "sleep", "60"]
        )
        .0,
        "the stand-in bridge session"
    );

    let mut words: Vec<&str> = vec!["start"];
    words.extend(server.iter().map(String::as_str));
    let (code, out, err) = telegram(&ae_home, &words);
    assert_eq!(code, 0, "the start failed: {err}");
    assert!(
        out.contains("already running (tmux session ae-telegram)"),
        "a start beside a live bridge did not defer: {out}"
    );
    // It still recorded the INTENT, which is what makes a later launch revive it.
    let written = fs::read_to_string(&config).expect("the config survived");
    assert!(
        ae::telegram_lifecycle::enabled_in(&written),
        "the intent was not persisted: {written}"
    );
    assert!(
        written.contains("[workspace]\nmain = lead\n"),
        "the rewrite disturbed another section: {written}"
    );

    let mut words: Vec<&str> = vec!["status"];
    words.extend(server.iter().map(String::as_str));
    let (code, out, _) = telegram(&ae_home, &words);
    assert_eq!(code, 0);
    for expected in [
        "intent:  enabled=true",
        "runtime: daemon running (tmux session ae-telegram)",
        "token:   OK",
        "include: state",
        "WARN: 'chat' not in include",
    ] {
        assert!(out.contains(expected), "status missed {expected:?}: {out}");
    }

    let mut words: Vec<&str> = vec!["stop"];
    words.extend(server.iter().map(String::as_str));
    let (code, out, err) = telegram(&ae_home, &words);
    assert_eq!(code, 0, "the stop failed: {err}");
    assert!(out.contains("ae telegram: stopped"), "{out}");
    let (_, sessions) = tmux(
        &socket,
        &scratch,
        &["list-sessions", "-F", "#{session_name}"],
    );
    assert!(
        !sessions.lines().any(|line| line == "ae-telegram"),
        "the bridge session outlived the stop: {sessions:?}"
    );
    let written = fs::read_to_string(&config).expect("the config survived");
    assert!(
        !ae::telegram_lifecycle::enabled_in(&written),
        "a stop that leaves the intent enabled is undone by the next launch: {written}"
    );

    let mut words: Vec<&str> = vec!["stop"];
    words.extend(server.iter().map(String::as_str));
    let (code, out, _) = telegram(&ae_home, &words);
    assert_eq!((code, out.trim()), (0, "ae telegram: was not running"));
}

#[test]
fn a_telegram_start_without_credentials_refuses_before_it_enables_anything() {
    let scratch = scratch("tgcred");
    let ae_home = scratch.join("home");
    assert!(fs::create_dir_all(&ae_home).is_ok());
    let config = ae_home.join("config");
    assert!(fs::write(&config, "[workspace]\nmain = lead\n").is_ok());

    let (code, _, err) = telegram(&ae_home, &["start"]);
    assert_eq!(code, 1, "a start with no credentials must fail");
    assert!(err.contains("token_file is not set"), "{err}");
    let written = fs::read_to_string(&config).expect("the config survived");
    assert_eq!(
        written, "[workspace]\nmain = lead\n",
        "a refused start must not leave enabled=true for a later autostart to act on"
    );

    // The autostart makes the same judgement, and says nothing at all when the
    // config never asked for a bridge.
    let mut err = Vec::new();
    let started = ae::telegram_lifecycle::autostart(
        &Paths::under(&ae_home),
        &ae::inventory::ServerId::Ambient,
        "some-session",
        &ae_home,
        &mut err,
    )
    .expect("the autostart writes to an in-memory buffer");
    assert!(
        !started,
        "nothing may start from a config that says nothing"
    );
    assert!(
        err.is_empty(),
        "a disabled bridge is not a warning: {}",
        String::from_utf8_lossy(&err)
    );

    let _ = fs::remove_dir_all(&scratch);
}
