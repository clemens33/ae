//! The undelivered-brief retry, black-box: the WIRING, not the grammar.
//!
//! The record's own grammar, its damage classes and the gate that weighs one
//! against a cycle are unit-tested beside the code, where a hostile byte string
//! needs no pane. What cannot be proven there is the part that spans processes:
//! that the session's own watchdog FINDS a record, that it reaches the leg
//! through the one helper it already shells to, that it spends exactly one
//! attempt per cycle, and that the text it delivers comes from the record
//! rather than from whoever triggered it.
//!
//! So the arms here run the real daemon against a real tmux server, with the
//! `send` helper replaced by a script that records what it was asked to do.
//! That script is the assertion surface: a brief ae never tried to deliver
//! leaves no line in it.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "fixtures build and inspect real directories with expect on the fixture \
              I/O; the capability boundary is about what PRODUCT code may reach"
)]

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::phase2::{run_tmux, tmux_present};

/// How long an arm waits for a real daemon cycle before it fails rather than
/// hangs. The daemon runs at `--interval 1`, so this is many cycles.
const BUDGET: Duration = Duration::from_secs(20);

/// The brief every fixture plants, with the newline a real brief carries.
const BRIEF: &str = "⟦ae:brief from lead⟧\nbuild the thing\n\nreply when done";

struct Rig {
    scratch: PathBuf,
    root: PathBuf,
    dir: PathBuf,
    socket: PathBuf,
    config: PathBuf,
    home: PathBuf,
}

impl Rig {
    /// A state root with one session, a live tmux server, and a roster seat
    /// whose pane really exists on it.
    fn new(tag: &str) -> Self {
        let scratch = PathBuf::from(format!("/tmp/ae-briefretry.{}.{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        let root = scratch.join("state");
        let dir = root.join("sessions").join("fixture");
        let home = scratch.join("home");
        for path in [&dir, &home] {
            assert!(fs::create_dir_all(path).is_ok(), "a fixture dir");
        }
        if !tmux_present(&scratch) {
            let _ = fs::remove_dir_all(&scratch);
            panic!(
                "tmux is not runnable here, so the retry's real-server arms cannot be proven; \
                 install tmux or run this suite where one exists"
            );
        }
        let socket = scratch.join("sock");
        let config = scratch.join("config");
        assert!(
            fs::write(&config, "[profiles]\ncl = \"claude\"\n").is_ok(),
            "a fixture config"
        );
        let rig = Self {
            scratch,
            root,
            dir,
            socket,
            config,
            home,
        };
        rig.tmux(&["new-session", "-d", "-s", "fixture", "sleep", "300"]);
        rig
    }

    /// One tmux call on this rig's OWN private socket, never a real server.
    fn tmux(&self, args: &[&str]) -> String {
        let mut all = vec!["-S".to_owned(), self.socket.display().to_string()];
        all.extend(args.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&all, &self.scratch).1
    }

    /// The pane id tmux really gave the seat's window.
    fn pane(&self) -> String {
        self.tmux(&["list-panes", "-t", "=fixture", "-F", "#{pane_id}"])
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned()
    }

    /// Publish a meta naming this server and one spawned seat at `pane`.
    fn seat(&self, pane: &str, launch_id: &str) {
        self.seats(pane, &[("spawned.1", "scribe")], launch_id);
    }

    /// The same, for as many spawned seats as an arm needs.
    fn seats(&self, pane: &str, roster: &[(&str, &str)], launch_id: &str) {
        use std::fmt::Write as _;
        let mut meta = format!(
            "mode=local\nmeta_version=2\nsession=fixture\ntmux_server_kind=socket\ntmux_server={}\n\
             seat.main=lead\nprofile.main=cl\nagent_bin.main=claude\nlaunch_id.main=tok-main\n",
            self.socket.display()
        );
        for (slot, name) in roster {
            let _ = write!(
                meta,
                "seat.{slot}={name}\nprofile.{slot}=cl\nagent_bin.{slot}=claude\n\
                 launch_id.{slot}={launch_id}\npane.{slot}={pane}\n"
            );
        }
        assert!(fs::write(self.dir.join("meta"), meta).is_ok(), "a meta");
    }

    /// The `send` helper the daemon shells to, replaced by a script that
    /// records every delivery it was asked to make and succeeds.
    fn recording_send(&self) {
        use std::os::unix::fs::PermissionsExt as _;
        let send = self.dir.join("send");
        assert!(
            fs::write(
                &send,
                "#!/bin/sh\nprintf '%s|%s|%s\\n' \"${_AE_EVENT_ACTION:-none}\" \"$1\" \"$2\" \
                 >> \"$(dirname \"$0\")/delivered\"\nexit 0\n",
            )
            .is_ok(),
            "the send helper"
        );
        assert!(
            fs::set_permissions(&send, fs::Permissions::from_mode(0o755)).is_ok(),
            "an executable send helper"
        );
    }

    /// Plant one record for `slot`, exactly as the grammar spells it.
    fn record(&self, slot: &str, pane: &str, launch_id: &str, attempts: u32, created: i64) {
        let body = format!(
            "brief-retry 1\nslot={slot}\nreference=spawn-{slot}\npane={pane}\n\
             launch_id={launch_id}\nactor=lead\nattempts={attempts}\ncreated={created}\n\
             phase=armed\nbody\n{BRIEF}"
        );
        assert!(
            fs::write(self.dir.join(format!("brief-retry.{slot}.rec")), body).is_ok(),
            "a planted record"
        );
    }

    fn record_path(&self, slot: &str) -> PathBuf {
        self.dir.join(format!("brief-retry.{slot}.rec"))
    }

    /// Everything the recording `send` was asked to deliver, so far.
    fn delivered(&self) -> String {
        fs::read_to_string(self.dir.join("delivered")).unwrap_or_default()
    }

    fn events(&self) -> String {
        fs::read_to_string(self.dir.join("events.jsonl")).unwrap_or_default()
    }

    /// Run the real daemon until `done` holds, or fail rather than hang.
    fn watch_until(&self, done: impl Fn() -> bool) -> bool {
        let out = fs::File::create(self.scratch.join("daemon-out")).expect("daemon stdout");
        let err = fs::File::create(self.scratch.join("daemon-err")).expect("daemon stderr");
        let mut child = super::cli::ae()
            .arg("_watchdog-run")
            .arg(&self.dir)
            .args([
                "--interval",
                "1",
                "--stale-secs",
                "999999",
                "--tg-supervise-secs",
                "0",
            ])
            .env("HOME", &self.home)
            .env("AE_HOME", &self.root)
            .env("CONFIG_FILE", &self.config)
            .stdout(out)
            .stderr(err)
            .spawn()
            .expect("the ae binary should spawn");
        let deadline = Instant::now() + BUDGET;
        let mut held = false;
        while Instant::now() < deadline {
            if done() {
                held = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        held
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
        let _ = fs::remove_dir_all(&self.scratch);
    }
}

fn now() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or_default(),
    )
    .unwrap_or_default()
}

/// THE WIRING. A record the gate would deliver reaches the leg through the one
/// helper the daemon already shells to, named by the retry action and by the
/// SEAT — never by the record's own contents, which the helper reads itself.
#[test]
fn a_deliverable_record_reaches_the_leg_through_the_watchdogs_one_helper() {
    let rig = Rig::new("wiring");
    let pane = rig.pane();
    rig.seat(&pane, "tok-1");
    rig.recording_send();
    rig.record("spawned.1", &pane, "tok-1", 0, now());

    let called = rig.watch_until(|| rig.delivered().contains("brief-retry|"));
    assert!(
        called,
        "the daemon never asked the helper to retry the brief; it delivered: {:?}",
        rig.delivered()
    );
    let line = rig
        .delivered()
        .lines()
        .find(|line| line.starts_with("brief-retry|"))
        .unwrap_or_default()
        .to_owned();
    // The seat is named; the message word carries NOTHING that reaches a pane.
    assert!(
        line.starts_with("brief-retry|scribe|"),
        "the retry names the seat: {line}"
    );
    assert!(
        !line.contains("build the thing"),
        "the brief's text must never ride the argv: {line}"
    );
}

/// A legacy `undelivered.<name>.txt` carries no record, and no glob may ever
/// qualify one into a brief. Planted beside a session with no record at all,
/// it must leave the helper untouched.
#[test]
fn a_recordless_undelivered_file_is_never_delivered() {
    let rig = Rig::new("inert");
    let pane = rig.pane();
    rig.seat(&pane, "tok-1");
    rig.recording_send();
    assert!(
        fs::write(rig.dir.join("undelivered.scribe.txt"), BRIEF).is_ok(),
        "a legacy undelivered file"
    );

    // Give the daemon real cycles to be wrong in.
    let delivered_something = rig.watch_until(|| rig.delivered().contains("brief-retry|"));
    assert!(
        !delivered_something,
        "a recordless file was delivered as a brief: {:?}",
        rig.delivered()
    );
}

/// Permanent damage is set aside ONCE, so the same bytes are never read as a
/// brief again, and the ledger says the record ended.
#[test]
fn a_permanently_damaged_record_is_set_aside_and_named_in_the_ledger() {
    let rig = Rig::new("damaged");
    let pane = rig.pane();
    rig.seat(&pane, "tok-1");
    rig.recording_send();
    assert!(
        fs::write(rig.record_path("spawned.1"), "not a record at all\n").is_ok(),
        "a damaged record"
    );

    let aside = rig.dir.join("brief-retry.spawned.1.rec.damaged");
    let moved = rig.watch_until(|| aside.exists());
    assert!(moved, "the damaged record was never set aside");
    assert!(
        !rig.record_path("spawned.1").exists(),
        "the damaged record is gone from the name a reader would use"
    );
    assert!(
        rig.events().contains("brief-gave-up"),
        "the give-up was never recorded: {}",
        rig.events()
    );
    assert!(
        !rig.delivered().contains("brief-retry|"),
        "damaged bytes were delivered as a brief: {:?}",
        rig.delivered()
    );
}

/// THE ROTATION. Oldest-created goes first, but the leg answers `Skip` for a
/// seat that is not ready and a Skip mutates NOTHING — so without a cursor the
/// same oldest record is recomputed and re-picked every cycle, and every newer
/// brief behind it waits out the whole 30-minute bound. One stuck seat in a
/// batch spawn is enough.
///
/// The fixture is that shape exactly: three records, and a `send` that succeeds
/// without consuming any of them, so every cycle faces the same three. A daemon
/// that picks by age alone reaches only the oldest, forever. This arm requires
/// all three to have been reached.
#[test]
fn a_stuck_oldest_record_cannot_starve_the_briefs_behind_it() {
    let rig = Rig::new("rotation");
    let pane = rig.pane();
    rig.seats(
        &pane,
        &[
            ("spawned.1", "alpha"),
            ("spawned.2", "beta"),
            ("spawned.3", "gamma"),
        ],
        "tok-1",
    );
    rig.recording_send();
    // ALPHA IS THE OLDEST, and is the one a by-age daemon would never leave.
    let base = now();
    rig.record("spawned.1", &pane, "tok-1", 0, base - 300);
    rig.record("spawned.2", &pane, "tok-1", 0, base - 200);
    rig.record("spawned.3", &pane, "tok-1", 0, base - 100);

    let all_three = rig.watch_until(|| {
        let delivered = rig.delivered();
        ["alpha", "beta", "gamma"]
            .iter()
            .all(|name| delivered.contains(&format!("brief-retry|{name}|")))
    });
    assert!(
        all_three,
        "the rotation never reached every record — a stuck oldest starves the rest. \
         delivered: {:?}",
        rig.delivered()
    );
    // AND THE ORDER IS STILL OLDEST-FIRST: the longest wait went first.
    let first = rig
        .delivered()
        .lines()
        .find(|line| line.starts_with("brief-retry|"))
        .unwrap_or_default()
        .to_owned();
    assert!(
        first.starts_with("brief-retry|alpha|"),
        "the oldest brief must still go first: {first}"
    );
}

/// Damage classification is OUTSIDE the one-per-cycle delivery budget: a
/// damaged record and a deliverable one are both handled in the SAME cycle, so
/// one unreadable file cannot eat the attempt that belonged to a live brief.
#[test]
fn damage_is_classified_outside_the_one_delivery_a_cycle_budget() {
    let rig = Rig::new("budget");
    let pane = rig.pane();
    rig.seats(
        &pane,
        &[("spawned.1", "broken"), ("spawned.2", "waiting")],
        "tok-1",
    );
    rig.recording_send();
    let base = now();
    // The DAMAGED one is the older, so a budget that spent itself on damage
    // would reach the live brief only after the damaged file was gone.
    assert!(
        fs::write(rig.record_path("spawned.1"), "not a record\n").is_ok(),
        "a damaged record"
    );
    rig.record("spawned.2", &pane, "tok-1", 0, base - 100);

    let both = rig.watch_until(|| {
        rig.dir.join("brief-retry.spawned.1.rec.damaged").exists()
            && rig.delivered().contains("brief-retry|waiting|")
    });
    assert!(
        both,
        "damage and delivery did not both happen; aside={} delivered={:?}",
        rig.dir.join("brief-retry.spawned.1.rec.damaged").exists(),
        rig.delivered()
    );
}
