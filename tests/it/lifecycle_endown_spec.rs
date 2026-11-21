//! An ordinary agent seat's plain end of its own session: the operand it
//! resolves is judged even when no name was typed, and the record it was
//! admitted on is proven again under the session's lifecycle lock.
//!
//! Oracle: nav ruling on ae-20261120T233017Z-4d721e07 — an inferred target
//! from another session's pane refuses like a typed one, and a record
//! replaced before the effect lock is never stopped, migrated or written.

use super::*;

const OTHER: &str = "55555555-5555-4555-8555-555555555555";

fn words(tail: &[&str]) -> Vec<String> {
    tail.iter().map(|word| (*word).to_owned()).collect()
}

/// The rig's own record, now naming another session uuid: what a replacement
/// of the same name on the same server leaves behind.
fn replace_record(rig: &Rig) {
    let meta = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
    assert!(meta.contains(UUID), "the rig records {UUID}");
    std::fs::write(rig.dir.join("meta"), meta.replace(UUID, OTHER)).expect("replacement");
}

fn nothing_ended(rig: &Rig) {
    assert!(rig.dir.is_dir(), "the record stays");
    let meta = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
    assert!(meta.contains(OTHER), "{meta}");
    assert!(rig.session_is_live(), "the session stays live");
    assert!(!rig.archive().exists(), "no archive of the admitted uuid");
    assert!(
        !rig.home.join("archive").join(OTHER).exists(),
        "no archive of the replacement"
    );
}

/// No name typed: the pane names the operand. A pane of another session
/// resolves to that session, and the seat's end of it refuses as the typed
/// name does, with exit 2, and that session stays.
#[test]
fn a_seat_end_resolved_from_another_sessions_pane_refuses() {
    let rig = Rig::new("eopane");
    let target = "eovictim";
    let dir = rig.home.join("sessions").join(target);
    std::fs::create_dir(&dir).expect("target dir");
    assert!(rig.tmux(&["new-session", "-d", "-s", target, "sh"]).0);
    let pane = first_pane(
        |args| rig.tmux(args),
        &["-s", "-t", &format!("={target}")],
        Instant::now() + SETUP_WAIT,
    );
    let meta = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
    let meta = meta
        .replace(
            &format!("session={}\n", rig.name),
            &format!("session={target}\n"),
        )
        .replace(UUID, OTHER)
        .replace(
            &format!("main_pane={}\n", rig.pane),
            &format!("main_pane={pane}\n"),
        );
    std::fs::write(dir.join("meta"), meta).expect("target registration");
    let (code, out, err) = rig.run_inside(&["_end", "-f", &format!("--pane={pane}")]);
    assert_eq!(code, Some(2), "{out}{err}");
    assert!(out.is_empty(), "{out}");
    assert!(
        err.contains("ae: end REFUSED") && err.contains(&format!("'{target}' is not its own")),
        "{err}"
    );
    assert!(dir.is_dir(), "the other session's record stays");
    let (_, listed) = rig.tmux(&["list-sessions", "-F", "#{session_name}"]);
    assert!(listed.lines().any(|line| line == target), "{listed}");
    assert!(rig.session_is_live(), "the caller's session stays");
    assert!(!rig.home.join("archive").join(OTHER).exists());
}

/// The carried expectation is proven under the lock, on the supervised and on
/// the direct route: a record whose uuid is not the admitted one refuses
/// before anything is stopped or written, the refusal included.
#[test]
fn a_carried_own_expectation_refuses_a_replaced_record_on_both_routes() {
    let rig = Rig::new("eostale");
    replace_record(&rig);
    let before = guard_spec::tree(&rig.dir);
    let refusal = format!(
        "Error: '{}' is no longer the session {UUID} this seat asked to end: its record was replaced since the end was asked — nothing was stopped and nothing was deleted.\n",
        rig.name
    );
    let mut supervised = words(&["_end", "--supervise", &rig.name]);
    supervised.extend(words(&[
        &format!("--confirmed-target={}", rig.name),
        "--confirmed-action=keep",
        "--confirmed-detail=/frozen/archive",
        "--confirmed-purge=off",
        "--confirmed-source=explicit",
        "--confirmed-tmux=unknown",
        &format!("--expect-own={UUID}"),
    ]));
    let argv: Vec<&str> = supervised.iter().map(String::as_str).collect();
    assert_eq!(rig.run(&argv), (Some(1), String::new(), refusal.clone()));
    assert_eq!(guard_spec::tree(&rig.dir), before);
    nothing_ended(&rig);
    let direct = format!("--expect-own={UUID}");
    assert_eq!(
        rig.run(&["_end", "-f", &rig.name, &direct]),
        (
            Some(1),
            String::new(),
            format!("{refusal}1 session(s) failed to end. See errors above.\n")
        )
    );
    assert_eq!(guard_spec::tree(&rig.dir), before);
    nothing_ended(&rig);
}

/// The whole route: the seat ends its own session, the supervisor waits on
/// the held lifecycle lock, and the record is replaced while it waits. The
/// supervisor proves its expectation under the lock, so the replacement is
/// neither stopped, migrated, archived nor written to. A supervisor without
/// that check ends it or records a failure in it. The supervisor starts
/// before the replacement, so this route cannot tell a carried expectation
/// from one the supervisor derived itself: the carrier unit test pins that.
#[test]
fn a_seat_end_of_its_own_session_refuses_a_record_replaced_before_the_lock() {
    let rig = Rig::new("eoreplace");
    let held = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(
            rig.home
                .join(format!("sessions/.lifecycle.{}.lock", rig.name)),
        )
        .expect("the canonical lifecycle lock");
    held.try_lock().expect("hold the lifecycle lock");
    assert_eq!(
        rig.run_inside(&["_end", "-f"]),
        (
            Some(0),
            format!("Ending '{}' out of pane; this pane will close.\n", rig.name),
            String::new()
        )
    );
    std::thread::sleep(Duration::from_millis(500));
    replace_record(&rig);
    let replaced = guard_spec::tree(&rig.dir);
    drop(held);
    // The supervisor waits at most its lock wait; an unproven end of the
    // replacement takes well under this.
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(guard_spec::tree(&rig.dir), replaced);
    nothing_ended(&rig);
}

/// A seat's own end as its handoff job or its supervisor receives it, with the
/// carried expectation and a frozen keep plan.
fn own_argv(rig: &Rig, mode: &str) -> Vec<String> {
    let mut argv = words(&["_end", mode, &rig.name]);
    argv.extend(words(&[
        &format!("--confirmed-target={}", rig.name),
        "--confirmed-action=keep",
        "--confirmed-detail=/frozen/archive",
        "--confirmed-purge=off",
        "--confirmed-source=explicit",
        "--confirmed-tmux=unknown",
        &format!("--expect-own={UUID}"),
    ]));
    argv
}

/// The handoff job of a seat's own end (the job the client's yes runs),
/// started when the record by that name is already a replacement: it writes
/// nothing to it, and its supervisor proves the carried uuid under the lock
/// and writes nothing either.
#[test]
fn a_seat_handoff_writes_nothing_to_a_record_already_replaced() {
    let rig = Rig::new("eohandoff");
    replace_record(&rig);
    let before = guard_spec::tree(&rig.dir);
    let argv = own_argv(&rig, "--handoff");
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    assert_eq!(rig.run(&argv), (Some(0), String::new(), String::new()));
    assert_eq!(
        guard_spec::tree(&rig.dir),
        before,
        "the handoff writes nothing"
    );
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(
        guard_spec::tree(&rig.dir),
        before,
        "nor does its supervisor"
    );
    nothing_ended(&rig);
}

/// A link in the record's place is no own record, even when its target
/// names the carried uuid: the supervisor refuses and writes nothing.
#[test]
fn a_link_to_matching_metadata_is_no_own_record() {
    let rig = Rig::new("eolink");
    let moved = rig.home.join("moved-record");
    std::fs::rename(&rig.dir, &moved).expect("move the record");
    std::os::unix::fs::symlink(&moved, &rig.dir).expect("link in its place");
    let before = guard_spec::tree(&moved);
    let argv = own_argv(&rig, "--supervise");
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    let (code, out, err) = rig.run(&argv);
    assert_eq!(code, Some(1), "{out}{err}");
    assert!(out.is_empty(), "{out}");
    assert!(!err.is_empty());
    assert_eq!(guard_spec::tree(&moved), before);
    assert!(
        std::fs::symlink_metadata(&rig.dir)
            .expect("the link")
            .file_type()
            .is_symlink()
    );
    assert!(rig.session_is_live());
    assert!(!rig.archive().exists());
}
