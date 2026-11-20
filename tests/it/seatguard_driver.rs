//! Driver regressions for the seat guard and the start journal at the public
//! binary entry on private tmux: `ae orchestrator` from an agent seat and from
//! the owner's terminal, bare `ae` from a stamped pane and the owner's restore,
//! and the child of `ae reboot`.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixture setup and independent before/after state checks"
)]

use super::super::o2guard_p2_spec::{Seat, World};
use super::super::relay_chat_spec::{Out, events, words};
use super::*;
use ae::events::Event;
use std::fs;
use std::mem::ManuallyDrop;
use std::time::{Duration, SystemTime};

const SOURCE: &str = "assistant";

/// The idle identity, with the orchestrator seat bound to the idle profile.
const CONFIG: &str = "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\
    orchestrator = idle\n\n[workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n\
    chat = off\nauto_upgrade = off\nquota = off\n";

fn fixture(tag: &str) -> (Rig, Seat) {
    let rig = Rig::idle(tag);
    fs::write(&rig.config, CONFIG).expect("private inert configuration");
    let world = ManuallyDrop::new(World {
        root: rig.home.clone(),
        sock: rig.sock.clone(),
    });
    let caller = world.session(SOURCE, "lead", "");
    (rig, caller)
}

fn run(rig: &Rig, pane: Option<&str>, argv: &[&str]) -> Out {
    let mut cmd = ae();
    cmd.env("HOME", &rig.scratch)
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", &rig.config)
        .env("TMUX_TMPDIR", &rig.scratch)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &rig.sock)
        .env("AE_NO_AUTOSTART", "1")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("AE_SENDER_OVERRIDE")
        .current_dir(&rig.project)
        .args(argv);
    if let Some(pane) = pane {
        cmd.env("TMUX", format!("{},0,0", rig.sock.display()))
            .env("TMUX_PANE", pane);
    }
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .expect("private executable returned");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn succeeds(out: &Out) {
    assert_eq!(out.0, Some(0), "{}", words(out));
}

fn is_live(rig: &Rig, name: &str) -> bool {
    rig.sessions().iter().any(|live| live == name)
}

fn starts(rig: &Rig, name: &str) -> Vec<Event> {
    events(&rig.dir(name))
        .into_iter()
        .filter(|row| row.action == "session-start")
        .collect()
}

fn recorded_uuid(rig: &Rig, name: &str) -> String {
    rig.meta(name)
        .lines()
        .find_map(|line| line.strip_prefix("session_id="))
        .expect("a recorded session uuid")
        .to_owned()
}

/// The owner's own terminal: no caller facts, no gap.
fn owner_row(row: &Event, summary: &str, uuid: &str) {
    assert_eq!(row.actor, "human", "{row:?}");
    assert_eq!(row.summary.as_deref(), Some(summary), "{row:?}");
    assert_eq!(row.target_session_uuid.as_deref(), Some(uuid), "{row:?}");
    assert_eq!(row.caller_pane, None, "{row:?}");
    assert_eq!(row.caller_session_uuid, None, "{row:?}");
    assert_eq!(row.identity_gap, None, "{row:?}");
}

#[test]
fn a_seat_reattaches_its_own_running_session_without_a_start_row() {
    let rig = Rig::idle("sgd-own");
    fs::write(&rig.config, CONFIG).expect("private inert configuration");
    let world = ManuallyDrop::new(World {
        root: rig.home.clone(),
        sock: rig.sock.clone(),
    });
    let current = format!("{}={}\n", ae::migrate::KEY, ae::migrate::CURRENT);
    let caller = world.session(SOURCE, "lead", &current);
    let journal = events(&caller.dir);
    let own = run(&rig, Some(&caller.pane), &[SOURCE, "--no-attach"]);
    succeeds(&own);
    assert!(own.1.contains("is running"), "{}", words(&own));
    assert!(is_live(&rig, SOURCE), "{}", words(&own));
    assert_eq!(events(&caller.dir), journal, "a reattach journals nothing");
}

#[test]
fn an_orchestrator_start_is_refused_to_a_seat_and_journals_the_owners_create() {
    let (rig, caller) = fixture("sgd-orch");
    let refused = run(&rig, Some(&caller.pane), &["orchestrator", "--no-attach"]);
    let shown = words(&refused);
    assert_eq!(refused.0, Some(2), "{shown}");
    assert!(
        refused
            .2
            .contains("runs no launch form on another session, and 'orchestrator' is not its own")
            && refused.2.contains("ae orchestrator"),
        "{shown}"
    );
    assert!(!is_live(&rig, "orchestrator"), "{shown}");
    assert!(
        !rig.dir("orchestrator").join("meta").exists(),
        "nothing was written for the seat: {shown}"
    );
    let rows: Vec<Event> = events(&caller.dir)
        .into_iter()
        .filter(|row| row.action == "launch-refused")
        .collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].target.as_deref(), Some("orchestrator"), "{rows:?}");
    assert_eq!(
        rows[0].summary.as_deref(),
        Some(
            "launch refused via orchestrator-start: an agent seat runs no launch form on another session"
        ),
        "{rows:?}"
    );

    succeeds(&run(&rig, None, &["orchestrator", "--no-attach"]));
    assert!(is_live(&rig, "orchestrator"), "the owner started the seat");
    let rows = starts(&rig, "orchestrator");
    assert_eq!(rows.len(), 1, "{rows:?}");
    let uuid = recorded_uuid(&rig, "orchestrator");
    owner_row(&rows[0], "created via orchestrator-start", &uuid);
}

#[test]
fn a_seat_cannot_start_the_orchestrator_through_the_unattended_settings_apply() {
    let (rig, caller) = fixture("sgd-settings");
    let deadline = (ae::time::Timestamp::now().epoch() + 30).to_string();
    let applied = run(
        &rig,
        Some(&caller.pane),
        &[
            "orchestrator",
            "--settings-apply",
            "start",
            "--target",
            "orchestrator",
            "--uuid",
            "",
            "--unattended",
            "--deadline",
            &deadline,
        ],
    );
    let shown = words(&applied);
    assert_eq!(applied.0, Some(2), "{shown}");
    assert!(
        applied
            .2
            .contains("runs no launch form on another session, and 'orchestrator' is not its own"),
        "{shown}"
    );
    assert!(!is_live(&rig, "orchestrator"), "{shown}");
    assert!(!rig.dir("orchestrator").join("meta").exists(), "{shown}");
    let rows: Vec<Event> = events(&caller.dir)
        .into_iter()
        .filter(|row| row.action == "launch-refused")
        .collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        rows[0].summary.as_deref(),
        Some("launch refused via settings: an agent seat runs no launch form on another session"),
        "{rows:?}"
    );
}

/// `alpha`, launched by the owner and then gone from its server without a
/// stop, inside the restore window and watched: what bare `ae` restores.
fn crashed_alpha(rig: &Rig) -> String {
    succeeds(&run(rig, None, &["alpha", "--local", "--no-attach"]));
    rig.wait_for_pane_command("alpha", "main", "sleep");
    let uuid = recorded_uuid(rig, "alpha");
    assert!(rig.tmux(&["kill-session", "-t", "=alpha"]).0, "alpha gone");
    let now = ae::time::Timestamp::now().epoch();
    fs::write(
        rig.dir("alpha").join(".launch-attempt"),
        format!("{}\n", now - 600),
    )
    .expect("a launch inside the window");
    let beat = fs::File::create(rig.dir("alpha").join(".watchdog-beat")).expect("beat");
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(u64::try_from(now - 60).expect("epoch"));
    beat.set_modified(at).expect("a recent beat");
    uuid
}

#[test]
fn bare_ae_from_a_stamped_pane_restores_nothing_and_the_owners_restore_is_journaled() {
    let (rig, caller) = fixture("sgd-restore");
    let uuid = crashed_alpha(&rig);
    assert_eq!(starts(&rig, "alpha").len(), 1, "the owner's create");

    let listed = run(&rig, Some(&caller.pane), &[]);
    assert!(!is_live(&rig, "alpha"), "{}", words(&listed));
    assert_eq!(starts(&rig, "alpha").len(), 1, "{}", words(&listed));

    let restored = run(&rig, None, &["--no-attach"]);
    succeeds(&restored);
    assert!(is_live(&rig, "alpha"), "{}", words(&restored));
    let rows = starts(&rig, "alpha");
    assert_eq!(rows.len(), 2, "{rows:?}");
    owner_row(&rows[1], "resumed via restore", &uuid);
}

#[test]
fn the_child_of_a_reboot_names_the_reboot_and_its_archive() {
    let (rig, _) = fixture("sgd-reboot");
    succeeds(&run(&rig, None, &["rb", "--local", "--no-attach"]));
    rig.wait_for_pane_command("rb", "main", "sleep");
    let archive = recorded_uuid(&rig, "rb");
    let rebooted = run(&rig, None, &["_compact", "rb", "-f", "--digest-only"]);
    succeeds(&rebooted);
    assert!(is_live(&rig, "rb"), "{}", words(&rebooted));
    let fresh = recorded_uuid(&rig, "rb");
    assert_ne!(fresh, archive, "a fresh incarnation");
    let rows: Vec<Event> = starts(&rig, "rb")
        .into_iter()
        .filter(|row| row.actor == "ae:reboot")
        .collect();
    assert_eq!(rows.len(), 1, "{}", words(&rebooted));
    let row = &rows[0];
    assert_eq!(
        row.summary.as_deref(),
        Some("created via reboot"),
        "{row:?}"
    );
    assert_eq!(row.reference.as_deref(), Some(archive.as_str()), "{row:?}");
    assert_eq!(
        row.target_session_uuid.as_deref(),
        Some(fresh.as_str()),
        "{row:?}"
    );
    assert_eq!(
        row.identity_gap.as_deref(),
        Some("caller pane ended with the rebooted session"),
        "{row:?}"
    );
    assert_eq!(row.caller_pane, None, "{row:?}");
}

fn refused_rows(dir: &std::path::Path) -> Vec<Event> {
    events(dir)
        .into_iter()
        .filter(|row| row.action == "launch-refused")
        .collect()
}

fn refused_unjournaled(out: &Out) {
    let shown = words(out);
    assert_eq!(out.0, Some(2), "{shown}");
    assert!(
        out.2.contains(&format!(
            "the refusal could not be journaled in session '{SOURCE}'"
        )),
        "{shown}"
    );
}

#[test]
fn a_seat_record_that_is_a_foreign_link_receives_no_refusal_row() {
    let (rig, caller) = fixture("sgd-link");
    let elsewhere = rig.scratch.join("elsewhere");
    fs::create_dir(&elsewhere).expect("a directory outside the sessions root");
    let moved = elsewhere.join(SOURCE);
    fs::rename(&caller.dir, &moved).expect("the caller record leaves the sessions root");
    std::os::unix::fs::symlink(&moved, &caller.dir).expect("a link that is no rename alias");
    let journal = events(&moved);

    let refused = run(&rig, Some(&caller.pane), &["orchestrator", "--no-attach"]);
    refused_unjournaled(&refused);
    assert_eq!(events(&moved), journal, "no row through the link");
    assert!(!is_live(&rig, "orchestrator"), "{}", words(&refused));
}

#[test]
fn a_seat_record_in_another_lifecycle_operation_receives_no_refusal_row() {
    let (rig, caller) = fixture("sgd-hold");
    let lock = rig
        .home
        .join("sessions")
        .join(format!(".lifecycle.{SOURCE}.lock"));
    let hold = ae::store::lock(&lock, Duration::ZERO)
        .expect("an owner rename holds the caller's lifecycle lock");
    let journal = events(&caller.dir);

    let refused = run(&rig, Some(&caller.pane), &["orchestrator", "--no-attach"]);
    refused_unjournaled(&refused);
    assert_eq!(
        events(&caller.dir),
        journal,
        "no row while the record is held"
    );

    drop(hold);
    let again = run(&rig, Some(&caller.pane), &["orchestrator", "--no-attach"]);
    assert_eq!(again.0, Some(2), "{}", words(&again));
    assert_eq!(refused_rows(&caller.dir).len(), 1, "{}", words(&again));
    assert!(!is_live(&rig, "orchestrator"), "{}", words(&again));
}

#[test]
fn a_resume_whose_record_holds_no_canonical_uuid_rolls_back_without_a_start_row() {
    let (rig, _) = fixture("sgd-nouuid");
    succeeds(&run(&rig, None, &["beta", "--local", "--no-attach"]));
    rig.wait_for_pane_command("beta", "main", "sleep");
    let created = starts(&rig, "beta");
    assert_eq!(created.len(), 1, "{created:?}");
    succeeds(&run(&rig, None, &["stop", "beta"]));
    assert!(!is_live(&rig, "beta"), "the owner stopped beta");
    let broken: String = rig
        .meta("beta")
        .lines()
        .map(|line| {
            if line.starts_with("session_id=") {
                "session_id=not-a-uuid\n".to_owned()
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    fs::write(rig.dir("beta").join("meta"), &broken).expect("a record without a canonical uuid");

    let resumed = run(&rig, None, &["beta", "--resume", "--no-attach"]);
    let shown = words(&resumed);
    assert_ne!(resumed.0, Some(0), "{shown}");
    assert!(!is_live(&rig, "beta"), "{shown}");
    assert_eq!(
        starts(&rig, "beta"),
        created,
        "no start row without a uuid: {shown}"
    );
    assert!(
        rig.meta("beta").contains("session_id=not-a-uuid"),
        "the stopped record stays: {shown}"
    );
}

/// A seat in `SOURCE` whose session carries no `@ae_session_uuid`.
fn bare_fixture(tag: &str) -> (Rig, Seat) {
    let rig = Rig::idle(tag);
    fs::write(&rig.config, CONFIG).expect("private inert configuration");
    let world = ManuallyDrop::new(World {
        root: rig.home.clone(),
        sock: rig.sock.clone(),
    });
    let current = format!("{}={}\n", ae::migrate::KEY, ae::migrate::CURRENT);
    let caller = world.session(SOURCE, "lead", &current);
    let (ok, _) = rig.tmux(&["set-option", "-u", "-t", &caller.pane, "@ae_session_uuid"]);
    assert!(ok, "the session carries no uuid");
    (rig, caller)
}

#[test]
fn a_seat_whose_session_carries_no_uuid_still_reattaches_its_own_session() {
    let (rig, caller) = bare_fixture("sgd-bare");
    let journal = events(&caller.dir);
    let own = run(&rig, Some(&caller.pane), &[SOURCE, "--no-attach"]);
    succeeds(&own);
    assert!(own.1.contains("is running"), "{}", words(&own));
    assert!(is_live(&rig, SOURCE), "{}", words(&own));
    assert_eq!(events(&caller.dir), journal, "a reattach journals nothing");

    // A stamp that is present but no canonical uuid is no vacancy: it must
    // prove the recorded uuid, and it cannot.
    let (ok, _) = rig.tmux(&[
        "set-option",
        "-t",
        &caller.pane,
        "@ae_session_uuid",
        "not-a-uuid",
    ]);
    assert!(ok, "a malformed live stamp");
    let meta = rig.meta(SOURCE);
    let malformed = run(&rig, Some(&caller.pane), &[SOURCE, "--no-attach"]);
    let shown = words(&malformed);
    assert_eq!(malformed.0, Some(2), "{shown}");
    assert!(
        malformed.2.contains(&format!(
            "runs no launch form on another session, and '{SOURCE}' is not its own"
        )),
        "{shown}"
    );
    assert_eq!(events(&caller.dir), journal, "no journal change: {shown}");
    assert_eq!(rig.meta(SOURCE), meta, "the record stayed intact: {shown}");
    assert!(is_live(&rig, SOURCE), "{shown}");
}

#[test]
fn a_seat_without_a_uuid_owns_no_record_of_its_name_on_another_server() {
    let (mut rig, caller) = bare_fixture("sgd-elsewhere");
    let other = rig.scratch.join("other.sock");
    rig.scratch.add_tmux_server(other.clone());
    let (started, _) = rig.keeper_on(
        &other,
        &[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "keeper",
            "sleep 600",
        ],
    );
    assert!(
        started,
        "a second private server, where '{SOURCE}' is absent"
    );
    let meta = rig.meta(SOURCE).replace(
        &format!("tmux_server={}\n", rig.sock.display()),
        &format!("tmux_server={}\n", other.display()),
    );
    assert!(meta.contains(&format!("tmux_server={}\n", other.display())));
    fs::write(rig.dir(SOURCE).join("meta"), &meta)
        .expect("a record of the same name on another server");

    let out = run(&rig, Some(&caller.pane), &[SOURCE, "--no-attach"]);
    let shown = words(&out);
    assert_eq!(out.0, Some(2), "{shown}");
    assert!(
        out.2.contains(&format!(
            "runs no launch form on another session, and '{SOURCE}' is not its own"
        )),
        "{shown}"
    );
    assert_eq!(rig.meta(SOURCE), meta, "the record stayed intact: {shown}");
    assert!(
        is_live(&rig, SOURCE),
        "the caller's session stayed live: {shown}"
    );
}
