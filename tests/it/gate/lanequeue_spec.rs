//! Acceptance for the timed lane-lock queue. Every claim uses the real recipe in a scratch git
//! checkout. The linked checkout must share its main checkout's queue.
#![allow(clippy::expect_used, reason = "fixture failures name the prerequisite")]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use super::lanetemp_spec::fake_sysfs;
use super::{lane_lock, lane_lock_checkouts, lane_lock_env, read};
use crate::cli::{OwnedChild, OwnedScratch, bounded, helper_by_name};

struct Fixture {
    scratch: OwnedScratch,
    main: PathBuf,
    linked: PathBuf,
    locks: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let (scratch, main, linked) = lane_lock_checkouts(tag);
        let locks = main.join(".local/locks");
        fs::create_dir_all(&locks).expect("scratch locks directory");
        Self {
            scratch,
            main,
            linked,
            locks,
        }
    }

    fn lock(&self, args: &[&str]) -> (i32, String, String) {
        lane_lock(&self.scratch, &self.main, args)
    }

    fn success(&self, args: &[&str]) -> String {
        let (code, out, err) = self.lock(args);
        assert_eq!(code, 0, "{args:?}: {out}{err}");
        out
    }

    fn status(&self, held: bool) -> String {
        let (code, out, err) = self.lock(&["status"]);
        assert_eq!(code, i32::from(held), "{out}{err}");
        out
    }

    fn next(&self) -> String {
        let path = self.locks.join("next");
        assert!(
            fs::symlink_metadata(&path).is_ok_and(|info| info.is_file()),
            "next remains a plain file for old readers"
        );
        read(&path)
    }

    fn set_next(&self, text: &str) {
        fs::write(self.locks.join("next"), text).expect("legacy next record");
    }

    fn waiter(&self, name: &str, secs: &str, linked: bool) -> Waiter {
        let out = self.scratch.join(format!("{name}.out"));
        let err = self.scratch.join(format!("{name}.err"));
        let pid = self.scratch.join(format!("{name}.pid"));
        let startup = self.scratch.join("bash-startup");
        // Just spawns the Bash recipe. Record that process's PID so signals reach the waiter,
        // and just can reap it. Nothing in the recipe is substituted by this fixture.
        fs::write(
            &startup,
            "if [ \"$0\" = lane-lock ]; then printf '%s\\n' \"$$\" >\"$AE_SPEC_RECIPE_PID\"; fi\n",
        )
        .expect("Bash startup file");
        let mut cmd = helper_by_name("just");
        cmd.process_group(0);
        let child = cmd
            .current_dir(if linked { &self.linked } else { &self.main })
            .env("HOME", self.scratch.path())
            .env("GIT_CONFIG_GLOBAL", self.scratch.join("gitconfig"))
            .env("AE_LANE_MIN_FREE_GIB", "0")
            .env("AE_LANE_MAX_TEMP_C", "0")
            .env("BASH_ENV", startup)
            .env("AE_SPEC_RECIPE_PID", &pid)
            .args(["lane-lock", "claim", name, secs])
            .stdin(Stdio::null())
            .stdout(fs::File::create(&out).expect("claim stdout"))
            .stderr(fs::File::create(&err).expect("claim stderr"))
            .spawn()
            .expect("start the private claim");
        Waiter {
            child: Some(child),
            out,
            err,
            pid,
        }
    }

    fn ticket(&self, name: &str) -> Option<PathBuf> {
        fs::read_dir(self.locks.join("queue"))
            .ok()?
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .and_then(|file| file.split('+').nth(1))
                    == Some(name)
            })
            .map(|entry| entry.path())
    }
}

struct Waiter {
    child: Option<OwnedChild>,
    out: PathBuf,
    err: PathBuf,
    pid: PathBuf,
}

impl Waiter {
    fn waiting(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if read(&self.err).contains("lane-lock: waiting") {
                return;
            }
            let ended = self.child.as_mut().expect("private claim").try_wait();
            assert!(
                Instant::now() < deadline && matches!(ended, Ok(None)),
                "claim did not enter WAITING: {}{} ({ended:?})",
                read(&self.out),
                read(&self.err)
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn signal(&self, signal: &str) {
        let pid = self.pid();
        let output = helper_by_name("kill")
            .args(["-s", signal, &pid.to_string()])
            .output()
            .expect("signal the waiter");
        assert!(
            output.status.success(),
            "{signal} for {pid}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn pid(&self) -> u32 {
        let pid = read(&self.pid).trim().parse::<u32>().expect("recipe pid");
        assert!(pid > 1, "the waiter owns a private process");
        pid
    }

    fn finish(mut self, expected_success: bool, seconds: u64) -> (String, String) {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let child = self.child.as_mut().expect("private claim");
        while child.try_wait().expect("claim status").is_none() {
            assert!(
                Instant::now() < deadline,
                "claim failed to end: {}",
                read(&self.err)
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        let child = self.child.take().expect("private claim");
        let finished = bounded(child, Duration::from_secs(1)).expect("reap the claim");
        let (out, err) = (read(&self.out), read(&self.err));
        assert_eq!(finished.status.success(), expected_success, "{out}{err}");
        (out, err)
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut()
            && matches!(child.try_wait(), Ok(None))
        {
            // Also reap descendants when an assertion fails while a recipe is stopped.
            let _ = helper_by_name("kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .output();
        }
    }
}

fn status_row<'a>(status: &'a str, name: &str, state: &str) -> &'a str {
    status
        .lines()
        .filter(|line| line.starts_with("queue"))
        .find(|line| {
            let words = || line.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-');
            words().any(|word| word == name) && words().any(|word| word == state)
        })
        .unwrap_or_else(|| panic!("missing {state} queue row for {name}:\n{status}"))
}

fn assert_age(row: &str) {
    let age = row
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|word| {
            word.starts_with(|ch: char| ch.is_ascii_digit()) && word.ends_with(['s', 'm', 'h'])
        });
    assert!(age, "queue row must include its age: {row}");
}

fn assert_bind(out: &str, name: &str) {
    let expected = format!("bind runs to this claim: export AE_LANE_CLAIM={name}");
    assert_eq!(
        out.lines().filter(|line| *line == expected).count(),
        1,
        "{out}"
    );
}

fn assert_stale_report(out: &str, err: &str, name: &str) {
    let combined = format!("{out}\n{err}");
    assert_eq!(
        combined
            .lines()
            .filter(|line| line.contains(name) && line.contains("stale") && line.contains("ticket"))
            .count(),
        1,
        "{combined}"
    );
}

#[test]
fn oldest_live_waiter_claims_before_a_later_waiter_from_another_worktree() {
    let fixture = Fixture::new("queue-fifo");
    fixture.success(&["claim", "holder"]);
    assert_eq!(fixture.lock(&["claim", "now", "0"]).0, 1);
    assert!(
        fixture.ticket("holder").is_none() && fixture.ticket("now").is_none(),
        "wait 0 writes no ticket"
    );
    let mut first = fixture.waiter("z-first", "5", false);
    first.waiting();
    first.signal("STOP");
    fixture.success(&["release", "holder"]);
    // An old dispatcher can empty next. The live ticket must still prevent overtaking.
    fixture.set_next("");
    let mut second = fixture.waiter("a-second", "12", true);
    second.waiting();
    let status = fixture.status(false);
    let first_row = status_row(&status, "z-first", "live");
    let second_row = status_row(&status, "a-second", "live");
    assert!(status.starts_with("gate: free\n"), "{status}");
    assert!(status.find(first_row) < status.find(second_row), "{status}");
    assert_age(first_row);
    assert_age(second_row);
    first.signal("CONT");
    first.finish(true, 9);
    assert!(fixture.ticket("z-first").is_none());
    fixture.success(&["release", "z-first"]);
    second.finish(true, 14);
    assert!(fixture.ticket("a-second").is_none());
    assert!(fixture.next().is_empty());
    fixture.success(&["release", "a-second"]);
}

#[test]
fn the_waiting_head_reserves_empty_next_and_matching_success_clears_next() {
    let fixture = Fixture::new("queue-next");
    fixture.success(&["claim", "holder"]);
    let mut head = fixture.waiter("head", "5", false);
    head.waiting();
    assert_eq!(fixture.next().trim(), "head");
    fixture.success(&["release", "holder"]);
    let (out, _) = head.finish(true, 9);
    assert!(fixture.next().is_empty());
    assert_bind(&out, "head");
    fixture.success(&["release", "head"]);
}

#[test]
fn a_manually_named_next_still_blocks_other_claims_with_a_free_gate() {
    let fixture = Fixture::new("queue-legacy-next");
    fixture.set_next("legacy\n");
    let (code, out, err) = fixture.lock(&["claim", "timed", "2"]);
    assert_eq!(code, 1, "{out}{err}");
    assert_eq!(fixture.next(), "legacy\n");
    assert!(
        fixture.ticket("timed").is_none(),
        "timeout removed the ticket"
    );
    assert!(fixture.status(false).contains("next: legacy"));
    let (code, out, err) = fixture.lock(&["claim", "legacy", "0"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(fixture.next().is_empty());
    assert_bind(&out, "legacy");
    assert!(fixture.ticket("legacy").is_none());
    fixture.success(&["release", "legacy"]);
}

#[test]
fn cancel_wait_removes_only_the_named_ticket() {
    let fixture = Fixture::new("queue-cancel");
    fixture.success(&["claim", "holder"]);
    let owner = read(&fixture.locks.join("gate/owner"));
    let mut first = fixture.waiter("cancel-me", "30", false);
    first.waiting();
    let first_ticket = fixture
        .ticket("cancel-me")
        .expect("first waiter has a ticket");
    let mut second = fixture.waiter("keep-me", "30", true);
    second.waiting();
    let second_ticket = fixture
        .ticket("keep-me")
        .expect("second waiter has a ticket");
    fixture.success(&["cancel-wait", "cancel-me"]);
    first.finish(false, 4);
    assert!(!first_ticket.exists());
    assert!(second_ticket.exists());
    let status = fixture.status(true);
    status_row(&status, "keep-me", "live");
    assert!(
        status
            .lines()
            .next()
            .is_some_and(|line| line.starts_with("gate: held by holder")),
        "{status}"
    );
    assert_eq!(read(&fixture.locks.join("gate/owner")), owner);
    fixture.success(&["cancel-wait", "keep-me"]);
    second.finish(false, 4);
    assert!(!second_ticket.exists());
    fixture.success(&["release", "holder"]);
    fixture.success(&["claim", "later", "0"]);
    fixture.success(&["release", "later"]);
}

#[test]
fn a_waiting_claim_cleans_its_ticket_and_reservation_when_it_ends() {
    for (name, secs, signal) in [("timeout", "3", None), ("terminated", "30", Some("TERM"))] {
        let fixture = Fixture::new(name);
        fixture.success(&["claim", "holder"]);
        let mut waiter = fixture.waiter(name, secs, false);
        waiter.waiting();
        let ticket = fixture
            .ticket(name)
            .expect("the waiting claim has a ticket");
        if let Some(signal) = signal {
            waiter.signal(signal);
        }
        waiter.finish(false, 8);
        assert!(!ticket.exists());
        assert!(fixture.next().is_empty());
        fixture.success(&["release", "holder"]);
    }
}

#[test]
fn a_dead_head_is_reported_and_removed_and_its_own_next_goes_with_it() {
    let fixture = Fixture::new("queue-dead-head");
    fixture.success(&["claim", "holder"]);
    let mut head = fixture.waiter("dead-head", "30", false);
    head.waiting();
    let ticket = fixture
        .ticket("dead-head")
        .expect("head published its ticket");
    assert_eq!(fixture.next().trim(), "dead-head");
    head.signal("KILL");
    head.finish(false, 5);
    let status = fixture.status(true);
    assert_age(status_row(&status, "dead-head", "stale"));
    assert!(
        ticket.exists(),
        "status leaves stale tickets for a claimant"
    );
    fixture.success(&["release", "holder"]);

    let (code, out, err) = fixture.lock(&["claim", "after-death", "2"]);
    assert_eq!(code, 0, "{out}{err}");
    assert_stale_report(&out, &err, "dead-head");
    assert_bind(&out, "after-death");
    assert!(!ticket.exists());
    assert!(fixture.next().is_empty());
    fixture.success(&["release", "after-death"]);
}

#[test]
fn stale_cleanup_preserves_a_manual_next_even_when_it_names_the_dead_waiter() {
    for case in ["prewritten", "replacement", "in-place"] {
        let fixture = Fixture::new(case);
        fixture.success(&["claim", "holder"]);
        if case == "prewritten" {
            fixture.set_next("dead-manual\n");
        }
        let mut head = fixture.waiter("dead-manual", "30", false);
        head.waiting();
        let ticket = fixture
            .ticket("dead-manual")
            .expect("head published its ticket");
        assert_eq!(fixture.next(), "dead-manual\n");
        head.signal("KILL");
        head.finish(false, 5);
        if case == "replacement" {
            // An external writer replaces the old reservation by a manual assignment of the same name.
            let manual = fixture.locks.join("manual-next");
            fs::write(&manual, "dead-manual\n").expect("new manual assignment");
            fs::rename(manual, fixture.locks.join("next")).expect("replace the reservation");
        }
        if case == "in-place" {
            fixture.set_next("");
            fixture.set_next("dead-manual\n");
        }
        fixture.success(&["release", "holder"]);

        let (code, out, err) = fixture.lock(&["claim", "after-death", "2"]);
        assert_eq!(code, 1, "{case}: {out}{err}");
        assert!(!ticket.exists());
        assert_eq!(fixture.next(), "dead-manual\n");
    }
}

#[test]
fn a_live_pid_with_a_different_start_identity_is_stale() {
    let fixture = Fixture::new("queue-reused-pid");
    fixture.set_next("legacy\n");
    let mut head = fixture.waiter("old-process", "30", false);
    head.waiting();
    head.signal("STOP");
    let path = fixture
        .ticket("old-process")
        .expect("head published its ticket");
    let basename = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("ASCII ticket name");
    let mut fields: Vec<_> = basename.split('+').collect();
    assert_eq!(fields[2], head.pid().to_string());
    assert_ne!(fields[3], "ObsoleteStartIdentity");
    fields[3] = "ObsoleteStartIdentity";
    let reused = path.with_file_name(fields.join("+"));
    fs::rename(&path, &reused).expect("change only the recorded start identity");
    status_row(&fixture.status(false), "old-process", "stale");

    fixture.set_next("");
    let (code, out, err) = fixture.lock(&["claim", "after-reuse", "2"]);
    assert_eq!(code, 0, "{out}{err}");
    assert_stale_report(&out, &err, "old-process");
    assert!(!reused.exists());
    fixture.success(&["release", "after-reuse"]);
    head.signal("KILL");
    head.finish(false, 5);
}

#[test]
fn an_unreadable_start_identity_still_counts_as_live() {
    let fixture = Fixture::new("queue-unreadable");
    fixture.set_next("legacy\n");
    let mut head = fixture.waiter("unreadable-head", "30", false);
    head.waiting();
    head.signal("STOP");
    let ticket = fixture
        .ticket("unreadable-head")
        .expect("head published its ticket");
    let pid = head.pid().to_string();
    let ps = fixture.scratch.join("ps");
    let observed = fixture.scratch.join("ps-observed");
    // Only the target PID's external identity read fails. All other ps calls stay real.
    fs::write(
        &ps,
        "#!/bin/sh\nfor arg do\n  if [ \"$arg\" = \"$AE_SPEC_NO_IDENTITY\" ]; then\n    : >\"$AE_SPEC_PS_OBSERVED\"\n    exit 1\n  fi\ndone\nexec /bin/ps \"$@\"\n",
    )
    .expect("fixture ps shim");
    fs::set_permissions(ps, fs::Permissions::from_mode(0o755)).expect("executable ps shim");
    let path = format!(
        "{}:{}",
        fixture.scratch.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let run = |args: &[&str]| {
        lane_lock_env(
            &fixture.scratch,
            &fixture.main,
            args,
            &[
                ("PATH", &path),
                ("AE_SPEC_NO_IDENTITY", &pid),
                (
                    "AE_SPEC_PS_OBSERVED",
                    observed.to_str().expect("fixture path"),
                ),
            ],
        )
    };
    fixture.set_next("");
    let (code, status, err) = run(&["status"]);
    assert_eq!(code, 0, "{status}{err}");
    assert!(
        observed.exists(),
        "the external identity observation failed as arranged"
    );
    let row = status_row(&status, "unreadable-head", "live");
    assert!(
        row.contains("unsure") || row.contains("unreadable"),
        "{row}"
    );
    fs::remove_file(&observed).expect("reset the observation marker");
    let (code, out, err) = run(&["claim", "behind-unreadable", "2"]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(observed.exists() && ticket.exists());
    head.signal("KILL");
    head.finish(false, 5);
}

#[test]
fn damaged_tickets_are_reported_and_block_until_timeout_without_being_executed() {
    for case in ["bad-name", "oversized"] {
        let fixture = Fixture::new(case);
        fixture.set_next("legacy\n");
        let mut head = fixture.waiter("damaged-head", "30", false);
        head.waiting();
        let ticket = fixture
            .ticket("damaged-head")
            .expect("a product-generated ticket");
        head.signal("KILL");
        head.finish(false, 5);
        let damaged = if case == "bad-name" {
            let dest = ticket.with_file_name("$(touch EXECUTED)");
            fs::rename(&ticket, &dest).expect("damage the ticket name only");
            dest
        } else {
            let content = format!("$(touch EXECUTED)\n#{}\n", "x".repeat(64 * 1024));
            fs::write(&ticket, content).expect("oversized ticket content");
            ticket
        };
        fixture.set_next("");
        let status = fixture.status(false);
        let reported = |text: &str| {
            text.split_ascii_whitespace()
                .any(|word| word.trim_matches(|ch: char| !ch.is_ascii_alphabetic()) == "damaged")
        };
        assert!(reported(&status), "{case}: {status}");
        let (code, out, err) = fixture.lock(&["claim", "behind-damage", "2"]);
        assert_eq!(code, 1, "{case}: {out}{err}");
        assert!(
            err.contains("waiting") && reported(&err) && err.contains("not claimed"),
            "{case}: {out}{err}"
        );
        assert!(
            damaged.exists(),
            "{case}: damaged state must stay for inspection"
        );
        for directory in [
            &fixture.main,
            &fixture.linked,
            &fixture.locks,
            &fixture.locks.join("queue"),
        ] {
            assert!(
                !directory.join("EXECUTED").exists(),
                "{case}: no ticket text ran in {}",
                directory.display()
            );
        }
        assert!(fixture.ticket("behind-damage").is_none());
    }
}

#[test]
fn damage_with_a_later_key_does_not_move_ahead_of_an_older_live_ticket() {
    let fixture = Fixture::new("queue-later-damage");
    fixture.success(&["claim", "holder"]);
    let mut first = fixture.waiter("older-live", "5", false);
    first.waiting();
    first.signal("STOP");
    let mut later = fixture.waiter("later-damaged", "30", true);
    later.waiting();
    let ticket = fixture
        .ticket("later-damaged")
        .expect("the later waiter has a ticket");
    later.signal("KILL");
    later.finish(false, 5);
    let basename = ticket
        .file_name()
        .and_then(|file| file.to_str())
        .expect("ASCII ticket name");
    let (key, _) = basename.split_once('+').expect("sortable ticket name");
    let damaged = ticket.with_file_name(format!("{key}+broken"));
    fs::rename(ticket, &damaged).expect("preserve the order key and damage later fields");
    fixture.set_next("");
    let status = fixture.status(true);
    let older = status
        .find(status_row(&status, "older-live", "live"))
        .expect("queue row");
    let broken = status
        .find("broken")
        .expect("status names the damaged entry");
    assert!(older < broken, "{status}");
    fixture.success(&["release", "holder"]);
    first.signal("CONT");
    first.finish(true, 12);
    assert!(damaged.exists());
    fixture.success(&["release", "older-live"]);
}

#[test]
fn cooling_waits_for_the_ticket_to_reach_the_head() {
    let fixture = Fixture::new("queue-cooling");
    let sysroot = fake_sysfs(&fixture.scratch, "92000", "0");
    fixture.success(&["claim", "holder"]);
    let mut head = fixture.waiter("first", "30", false);
    head.waiting();
    head.signal("STOP");
    fixture.success(&["release", "holder"]);
    fixture.set_next("");

    let (code, out, err) = lane_lock_env(
        &fixture.scratch,
        &fixture.linked,
        &["claim", "behind-hot-head", "2"],
        &[
            (
                "AE_LANE_SYSFS_ROOT",
                sysroot.to_str().expect("fixture path"),
            ),
            ("AE_LANE_MAX_TEMP_C", "75"),
            ("AE_LANE_COOL_STEP_SECS", "1"),
            ("AE_LANE_COOL_CAP_SECS", "1"),
        ],
    );
    assert_eq!(code, 1, "{out}{err}");
    assert!(!err.contains("lane-lock: cooling"), "{out}{err}");
    head.signal("KILL");
    head.finish(false, 5);
}

#[test]
fn a_non_directory_queue_cannot_be_used_for_tickets() {
    use std::os::unix::fs::symlink;

    for case in ["queue-file", "queue-link"] {
        let fixture = Fixture::new(case);
        let queue = fixture.locks.join("queue");
        let outside = fixture.scratch.join("decoy");
        fs::create_dir(&outside).expect("decoy directory");
        if case == "queue-file" {
            fs::write(&queue, "untouched").expect("regular file at queue path");
        } else {
            symlink(&outside, &queue).expect("queue symlink");
        }
        assert!(
            fixture
                .status(false)
                .lines()
                .any(|line| line.starts_with("queue") && line.contains("directory"))
        );
        let (code, out, err) = fixture.lock(&["claim", "cannot-queue", "2"]);
        assert_ne!(code, 0, "{case}: {out}{err}");
        assert!(err.contains("queue"), "{case}: {out}{err}");
        assert!(
            fs::read_dir(&outside)
                .expect("decoy directory")
                .next()
                .is_none()
        );
        assert!(!fixture.locks.join("gate").exists());
    }
}
