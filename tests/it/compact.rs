//! `ae _compact-freeze <session-dir> [--keep-history]` — reboot's freeze/resolve step
//! on the built binary, black-box. Pure read-only: it emits the frozen tuple or a
//! clear refusal, and mutates nothing. Plus the public surface: the `ae compact`
//! tripwire + verb usage, and `ae reboot` answering the destructive argv.

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
        let dir = super::cli::OwnedScratch::root("compact", tag).keep();
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

/// Build `<AE_HOME>/sessions/<name>` with a local meta and a `[workspace]`
/// config.
fn local_session(s: &Scratch, name: &str, mode: &str, config_body: &str) -> PathBuf {
    let sessions = s.0.join("sessions");
    let dir = sessions.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let config = s.0.join("config");
    std::fs::write(&config, config_body).unwrap();
    let meta = format!(
        "session_id={UUID}\nmode={mode}\norigin={}\nseat.main=main\nprofile.main=cl\nharness_session.main={UUID}\nconfig={}\n",
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

/// Run any core subcommand under `AE_HOME`, bounded.
fn core(ae_home: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = crate::cli::ae();
    cmd.env("AE_HOME", ae_home);
    for a in args {
        cmd.arg(a);
    }
    crate::cli::bounded(
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn core"),
        Duration::from_secs(10),
    )
    .expect("core returned")
}

/// The frozen tuple `_compact-freeze` emits for `dir` (trailing newline trimmed).
fn tuple_of(ae_home: &Path, dir: &Path) -> String {
    let out = freeze(ae_home, dir, false);
    assert_eq!(out.status.code(), Some(0), "freeze: {}", stderr(&out));
    stdout(&out).trim_end().to_owned()
}

#[test]
fn archive_step_refuses_a_malformed_tuple() {
    let s = Scratch::new("arch-malformed");
    let dir = local_session(&s, "sess", "local", "[workspace]\nmain = cl\n");
    let out = core(
        s.0.as_path(),
        &[
            "_compact-archive",
            dir.to_str().unwrap(),
            "not\u{1f}enough\u{1f}fields",
            "2026-08-01T00:00:00Z",
            "-",
            "-",
            "-",
            "-",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("did not parse"), "{}", stderr(&out));
}

#[test]
fn archive_step_refuses_a_replacement_session() {
    let s = Scratch::new("arch-replace");
    let dir = local_session(&s, "sess", "local", "[workspace]\nmain = cl\n");
    let tuple = tuple_of(s.0.as_path(), &dir);
    // The session is replaced under the same name: a fresh session_id.
    let meta = format!(
        "session_id=99999999-9999-9999-9999-999999999999\nmode=local\norigin={}\nseat.main=main\nprofile.main=cl\nharness_session.main=99999999-9999-9999-9999-999999999999\nconfig={}\n",
        s.0.display(),
        s.0.join("config").display()
    );
    std::fs::write(dir.join("meta"), meta).unwrap();
    let out = core(
        s.0.as_path(),
        &[
            "_compact-archive",
            dir.to_str().unwrap(),
            &tuple,
            "2026-08-01T00:00:00Z",
            "-",
            "-",
            "-",
            "-",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("not the session that was authorized"),
        "{}",
        stderr(&out)
    );
    assert!(stdout(&out).is_empty(), "no recovery line on refusal");
}

#[test]
fn archive_step_refuses_a_tuple_whose_name_was_altered() {
    // The altered-name attack: the real live session is `sess`, but the
    // authorization tuple's name field is rewritten to an absent `ghost`.
    let s = Scratch::new("arch-altered-name");
    let dir = local_session(&s, "sess", "local", "[workspace]\nmain = cl\n");
    let tuple = tuple_of(s.0.as_path(), &dir);
    let mut fields: Vec<&str> = tuple.split('\u{1f}').collect();
    fields[0] = "ghost";
    let altered = fields.join("\u{1f}");
    let out = core(
        s.0.as_path(),
        &[
            "_compact-archive",
            dir.to_str().unwrap(),
            &altered,
            "2026-08-01T00:00:00Z",
            "-",
            "-",
            "-",
            "-",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("does not point at this session"),
        "{}",
        stderr(&out)
    );
    assert!(stdout(&out).is_empty(), "no recovery line on refusal");
    assert!(!s.0.join("archive").join(UUID).exists(), "nothing archived");
}

#[test]
fn archive_step_refuses_when_the_stop_is_unprovable() {
    // A local_session records NO tmux_server → a Missing selector → verify_stopped is
    // Unknown → archive refuses (fail closed), never touching tmux or the archive.
    let s = Scratch::new("arch-unprovable");
    let dir = local_session(&s, "sess", "local", "[workspace]\nmain = cl\n");
    let tuple = tuple_of(s.0.as_path(), &dir);
    let out = core(
        s.0.as_path(),
        &[
            "_compact-archive",
            dir.to_str().unwrap(),
            &tuple,
            "2026-08-01T00:00:00Z",
            "-",
            "-",
            "-",
            "-",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("could not PROVE") && stderr(&out).contains("stopped"),
        "{}",
        stderr(&out)
    );
    assert!(
        stdout(&out).is_empty(),
        "nothing archived, no recovery line"
    );
    // Read-only: the archive root was never created.
    assert!(!s.0.join("archive").join(UUID).exists());
}

#[test]
fn teardown_step_refuses_when_the_stop_is_unprovable() {
    let s = Scratch::new("teardown-unprovable");
    let dir = local_session(&s, "sess", "local", "[workspace]\nmain = cl\n");
    let tuple = tuple_of(s.0.as_path(), &dir);
    let out = core(
        s.0.as_path(),
        &["_compact-teardown", dir.to_str().unwrap(), &tuple],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("could not PROVE"), "{}", stderr(&out));
    // The live session is untouched.
    assert!(
        dir.join("meta").exists(),
        "teardown refused, session intact"
    );
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
    assert_eq!(fields[8], "main", "main_ref");
    assert_eq!(fields[9], "main=cl workers=a, b", "roster");
    // Read-only: the session dir and its meta are untouched.
    assert!(dir.join("meta").exists());
}

#[test]
fn a_fifo_global_config_is_refused_without_hanging() {
    // A recorded global config that is a FIFO must be refused by
    // CLASSIFICATION, never opened: an ungated `read_to_string` on a writerless
    // FIFO blocks forever.
    let s = Scratch::new("fifo");
    let dir = s.0.join("sessions").join("sess");
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = s.0.join("fifo-config");
    crate::cli::mkfifo(&cfg);
    let meta = format!(
        "session_id={UUID}\nmode=local\norigin={}\nseat.main=main\nprofile.main=cl\nharness_session.main={UUID}\nconfig={}\n",
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

/// Seed a LEGACY handover ask (slotless pre-rename `ae:compact:` actor → main)
/// into `<dir>/events.jsonl`, with a stored body carrying memo baseline 0.
fn seed_handover(dir: &Path) -> String {
    let reference = "ae-20260829T000000Z-abcd1234";
    std::fs::create_dir_all(dir.join("messages")).unwrap();
    let body = dir.join("messages").join("handover.ask.txt");
    std::fs::write(&body, "COMPACT HANDOVER\nAE-COMPACT-MEMO-BASELINE=0\n").unwrap();
    let ask = format!(
        "{{\"ts\":\"2026-08-29T00:00:00Z\",\"actor\":\"ae:compact:{UUID}\",\"action\":\"ask\",\"target\":\"cl:main\",\"ref\":\"{reference}\",\"body_file\":\"{}\",\"actor_session\":\"sess\",\"target_slot\":\"main\",\"target_session\":\"sess\"}}\n",
        body.display()
    );
    std::fs::write(dir.join("events.jsonl"), ask).unwrap();
    reference.to_owned()
}

#[test]
fn compact_wait_succeeds_end_to_end_when_both_facts_are_present() {
    // The full CLI path: parse → dispatch → wait_step.
    let s = Scratch::new("wait-e2e");
    let dir = s.0.join("sessions").join("sess");
    std::fs::create_dir_all(&dir).unwrap();
    let reference = seed_handover(&dir);
    // The reply from main, and a new handover memo row.
    let reply = format!(
        "{{\"ts\":\"2026-08-29T00:01:00Z\",\"actor\":\"cl:main\",\"action\":\"reply\",\"ref\":\"{reference}\",\"actor_slot\":\"main\",\"actor_session\":\"sess\"}}\n"
    );
    std::fs::write(
        dir.join("events.jsonl"),
        format!(
            "{{\"ts\":\"2026-08-29T00:00:00Z\",\"actor\":\"ae:compact:{UUID}\",\"action\":\"ask\",\"target\":\"cl:main\",\"ref\":\"{reference}\",\"body_file\":\"{}\",\"actor_session\":\"sess\",\"target_slot\":\"main\",\"target_session\":\"sess\"}}\n{reply}",
            dir.join("messages").join("handover.ask.txt").display()
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("memo.tsv"),
        "2026-08-29T00:01:00Z\tcl:main\thandover\tpicking up\n",
    )
    .unwrap();
    let out = core(
        s.0.as_path(),
        &[
            "_compact-wait",
            dir.to_str().unwrap(),
            &reference,
            "--timeout",
            "0",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("handover complete"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn compact_cancel_withdraws_end_to_end() {
    // The full CLI path: parse → dispatch → cancel_step, which appends a slotless cancel.
    let s = Scratch::new("cancel-e2e");
    let dir = s.0.join("sessions").join("sess");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("meta"), format!("session_id={UUID}\nmode=local\n")).unwrap();
    let reference = seed_handover(&dir);
    let out = core(
        s.0.as_path(),
        &["_compact-cancel", dir.to_str().unwrap(), &reference],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    // A cancel event for the ref was appended to the ledger.
    let ledger = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
    assert!(
        ledger.contains("\"action\":\"cancel\"") && ledger.contains(&reference),
        "cancel event recorded: {ledger}"
    );
}

/// Run a PUBLIC verb (`compact`, `reboot`) with the hermetic env the entry
/// rig uses: a scratch `HOME`/`AE_HOME`/config, no tmux inheritance.
/// `rootless` drops every state-root door instead, for the pre-preamble pin.
fn public_with(s: &Scratch, args: &[&str], rootless: bool) -> std::process::Output {
    let mut cmd = crate::cli::ae();
    if rootless {
        cmd.env_remove("HOME")
            .env_remove("AE_HOME")
            .env_remove("CONFIG_FILE");
    } else {
        cmd.env("HOME", &s.0)
            .env("AE_HOME", &s.0)
            .env("CONFIG_FILE", s.0.join("config"));
    }
    cmd.env("AE_NO_AUTOSTART", "1")
        .env("TMUX_TMPDIR", &s.0)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("AE_TMUX_SERVER_KIND")
        .env_remove("AE_TMUX_SERVER");
    for a in args {
        cmd.arg(a);
    }
    crate::cli::bounded(
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn public verb"),
        Duration::from_secs(10),
    )
    .expect("public verb returned")
}

#[test]
fn compact_tripwire_fires_the_exact_line_for_every_destructive_flag() {
    let s = Scratch::new("tripwire");
    for (args, flag) in [
        (vec!["compact", "-f", "sess"], "-f"),
        (vec!["compact", "--force", "sess"], "--force"),
        (vec!["compact", "--keep-history", "sess"], "--keep-history"),
        (vec!["compact", "--digest-only", "sess"], "--digest-only"),
        (
            vec!["compact", "--exec-plan", "/tmp/p", "sess"],
            "--exec-plan",
        ),
        (vec!["compact", "--exec-plan"], "--exec-plan"),
    ] {
        let out = public_with(&s, &args, false);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
        assert_eq!(
            stderr(&out),
            format!(
                "ae: '{flag}' belongs to the destructive verb, which is now 'ae reboot'. Run: ae reboot {flag} [name]\n"
            ),
            "{args:?}"
        );
        assert!(stdout(&out).is_empty(), "{args:?}");
    }
    // Named-but-refused and unknown flags get the generic error.
    for flag in ["--purge-history", "--bogus"] {
        let out = public_with(&s, &["compact", flag, "sess"], false);
        assert_eq!(out.status.code(), Some(2), "{flag}");
        assert_eq!(stderr(&out), format!("Error: unknown flag '{flag}'.\n"));
    }
}

#[test]
fn compact_tripwire_fires_without_any_state_root() {
    // Routed before the preamble: with neither HOME nor AE_HOME set there is
    // no exit-1 state-root refusal, just the exit-2 tripwire, alone on stderr.
    let s = Scratch::new("noroot");
    let out = public_with(&s, &["compact", "--force", "sess"], true);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert_eq!(
        stderr(&out),
        "ae: '--force' belongs to the destructive verb, which is now 'ae reboot'. Run: ae reboot --force [name]\n"
    );
    assert!(stdout(&out).is_empty());
}

#[test]
fn bare_compact_needs_a_name_outside_a_session() {
    let s = Scratch::new("compact-usage");
    let out = public_with(&s, &["compact"], false);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert_eq!(stderr(&out), ae::entry::COMPACT_USAGE);
    assert!(stdout(&out).is_empty());
    let out = public_with(&s, &["compact", "sess"], false);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(stderr(&out), "ae: no session state for 'sess'.\n");
}

#[test]
fn reboot_accepts_every_tripwire_flag_past_its_parser() {
    // The parser side of the list-equals-parser pin: each tripwire spelling
    // reaches the missing-name usage (never unknown-flag), so the tripwire
    // fires exactly for what the destructive verb accepts.
    let s = Scratch::new("reboot-flags");
    for (args, accepted) in [
        (vec!["reboot", "-f"], true),
        (vec!["reboot", "--force"], true),
        (vec!["reboot", "--keep-history"], true),
        (vec!["reboot", "--digest-only"], true),
        (vec!["reboot", "--exec-plan", "/tmp/p"], true),
        (vec!["reboot", "--bogus"], false),
    ] {
        let out = public_with(&s, &args, false);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        if accepted {
            assert!(
                stderr(&out).starts_with("Usage: _compact "),
                "{args:?}: {}",
                stderr(&out)
            );
        } else {
            assert!(
                stderr(&out).contains("unknown flag"),
                "{args:?}: {}",
                stderr(&out)
            );
        }
        assert!(stdout(&out).is_empty(), "{args:?}");
    }
}

#[test]
fn a_legacy_compact_handover_is_found_and_withdrawn_with_its_bytes() {
    // Seed OLD, resume (find-outstanding), cancel: the withdrawal lands and
    // keeps the legacy opener bytes, never rewritten to the new namespace.
    let s = Scratch::new("legacy-e2e");
    let dir = local_session(&s, "sess", "local", "[workspace]\nmain = cl\n");
    let reference = seed_handover(&dir);
    let out = core(
        s.0.as_path(),
        &["_compact-find-outstanding", dir.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), reference, "the resume path finds it");
    let out = core(
        s.0.as_path(),
        &["_compact-cancel", dir.to_str().unwrap(), &reference],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let ledger = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
    let cancel = ledger
        .lines()
        .find(|line| line.contains("\"action\":\"cancel\""))
        .expect("a cancel record");
    assert!(
        cancel.contains(&format!("\"actor\":\"ae:compact:{UUID}\"")),
        "{cancel}"
    );
}

/// The public verb's whole-store behaviour the internals cannot show: the R1
/// run lock refuses a second run BEFORE any state is touched.
#[test]
fn a_held_run_lock_refuses_the_second_compact_before_any_state_is_touched() {
    let s = Scratch::new("lock-held");
    let dir = s.0.join("sessions").join("sess");
    std::fs::create_dir_all(&dir).unwrap();
    let lock = dir.join("seatcompact.lock");
    let held = ae::store::lock(&lock, Duration::ZERO).expect("the fixture holds the run lock");

    let out = core(s.0.as_path(), &["compact", "sess"]);

    let expected = format!(
        "another ae compact holds {}\n",
        ae::seatcompact::cell(&lock.to_string_lossy())
    );
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(stderr(&out), expected, "the projected path, alone");
    assert!(stdout(&out).is_empty(), "no report");
    assert!(
        !dir.join("events.jsonl").exists(),
        "a refused run records nothing"
    );
    drop(held);
}

/// The OTHER reader of one ledger: `SessionRead::pending` is what
/// `entry_from` turns into the `unanswered` attention
/// (`session.rs::entry_from`), so an answered seat checkpoint must be closed
/// here too. The view's `requests::states` is pinned beside its sensor.
#[test]
fn the_watchdog_ledger_reader_reads_an_answered_seat_checkpoint_as_closed() {
    let uuid = "22222222-2222-2222-2222-222222222222";
    let reference = "ae-20260917T090000Z-abcdef01";
    let lines = [
        format!(
            "{{\"ts\":\"2026-09-17T09:00:00Z\",\"actor\":\"ae:seats:{uuid}\",\"action\":\"ask\",\"target\":\"cl:main\",\"ref\":\"{reference}\",\"actor_session\":\"sess\",\"target_slot\":\"main\",\"target_session\":\"sess\",\"summary\":\"checkpoint\"}}"
        ),
        format!(
            "{{\"ts\":\"2026-09-17T09:01:00Z\",\"actor\":\"cl:main\",\"action\":\"reply\",\"target\":\"ae:seats:{uuid}\",\"ref\":\"{reference}\",\"actor_slot\":\"main\",\"actor_session\":\"sess\",\"target_session\":\"sess\",\"summary\":\"saved\"}}"
        ),
    ];
    let events: Vec<ae::events::Event> = lines
        .iter()
        .map(|line| ae::events::Event::parse_line(line).expect("a fixture event"))
        .collect();
    let read = ae::session::SessionRead::from_drain(&ae::events::Drain {
        events,
        cursor: ae::events::Cursor::default(),
        skipped: Vec::new(),
        drained: true,
    });
    let now = ae::time::Timestamp::parse("2026-09-17T10:00:00Z").expect("a fixture clock");
    assert!(
        read.unanswered(now, ae::session::DEFAULT_UNANSWERED_SECS)
            .is_empty(),
        "still unanswered: {:?}",
        read.pending
    );
}

/// D1: the stamp hold refuses a writerless FIFO swapped in between its `lstat`
/// and its open, PROMPTLY: `O_NONBLOCK` keeps the open from waiting for a
/// writer. A lost bit hangs here, and `.config/nextest.toml` ends that.
#[test]
fn a_writerless_fifo_swapped_in_after_lstat_is_refused_without_blocking() {
    let s = Scratch::new("stamp-fifo");
    let store = ae::store::open(&s.0);
    store.stamp_launch_attempt(1_789_105_855).unwrap();
    let node = store.stamp_node().expect("a regular stamp");
    let fifo = s.0.join("fifo");
    crate::cli::mkfifo(&fifo);
    std::fs::rename(&fifo, store.launch_attempt_path()).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(node.open().err(), Some(ae::store::StampGap::Unreadable));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "refused promptly"
    );
}
