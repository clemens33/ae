//! S4 real product spawn acceptance. Frozen S4 v1; navigator revision and lead gate required for changes.
//! Install as a child module of tests/it/spawn.rs to reuse the existing private-server Rig.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned private-server fixture IO"
)]

use super::Rig;
use ae::json::Value;
use ae::stats::{self, Source, ThisSession};
use ae::stats_store::disk::{self, Files, Shown};
use std::fmt::Write as _;

const SID: &str = "11111111-1111-4111-8111-111111111111";
const PRIVATE: &str = "PRIVATE_S4_SPAWN_PAYLOAD";

fn rig(tag: &str) -> Rig {
    let rig = Rig::new(tag);
    let mut meta = rig.meta();
    writeln!(meta, "session_id={SID}").expect("fixture session UUID");
    std::fs::write(rig.dir.join("meta"), meta).expect("canonical UUID meta");
    assert!(
        rig.tmux(&[
            "set-option",
            "-t",
            &format!("={}:", rig.session),
            "@ae_session_uuid",
            SID
        ])
        .0
    );
    rig
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    match value.get(key) {
        Some(Value::Str(value)) => Some(value),
        _ => None,
    }
}

#[test]
fn s4_real_spawn_survives_all_processes_with_both_counted_sites_and_no_journal_available() {
    let rig = rig("s4durable");
    let (code, stdout, stderr) =
        rig.run(ae::cli::SPAWN, &["scout", "--using", "fake", "--", PRIVATE]);
    assert_eq!(code, Some(0), "{stdout}\n{stderr}");
    let journal = rig.events();
    let meta = rig.meta();
    let birth = journal
        .lines()
        .filter_map(|line| ae::json::parse(line).ok())
        .find(|record| text(record, "action") == Some("worker-birth"))
        .expect("the product committed a worker birth");
    let birth_id = text(&birth, "birth").expect("worker birth key");
    assert!(
        rig.tmux(&["kill-server"]).0,
        "stop all processes on the private test server"
    );
    std::fs::rename(
        rig.dir.join("events.jsonl"),
        rig.scratch.join("unavailable-journal"),
    )
    .expect("journal unavailable to any later reader");

    // A new reader owns no pre-spawn cache and no maintainer. The writer process has exited.
    for scope in [
        None,
        Some(SID),
        Some("22222222-2222-4222-8222-222222222222"),
    ] {
        let expected = stats::fold(
            vec![Source::live(
                &rig.session,
                meta.as_bytes(),
                Ok(journal.as_bytes().to_vec()),
            )],
            scope,
        );
        let Shown::Ready { view, updating, .. } = disk::shown(&mut Files::at(&rig.scratch), scope)
        else {
            panic!("a counted spawn must already have durable views after process exit");
        };
        assert!(!updating, "the spawn returned after its publication");
        assert_eq!(
            view, expected,
            "all fields of the original fold in scope {scope:?}"
        );
        assert_eq!(view.all.total, 1, "one committed worker");
        assert!(view.all.balanced());
        if let ThisSession::Ready(this) = &view.this {
            assert!(this.balanced());
        }
    }

    let evidence = std::fs::read_to_string(
        rig.scratch
            .join("stats/session")
            .join(format!("{SID}.evidence")),
    )
    .expect("durable evidence for the canonical session");
    let records: Vec<Value> = evidence
        .lines()
        .filter_map(|line| ae::json::parse(line).ok())
        .filter_map(|record| text(&record, "line").and_then(|line| ae::json::parse(line).ok()))
        .collect();
    for action in ["worker-birth", "spawn"] {
        assert!(
            records
                .iter()
                .any(|record| text(record, "action") == Some(action)
                    && text(record, "birth") == Some(birth_id)),
            "the actual {action} append site must publish its own record without catch-up"
        );
    }
    assert!(
        !evidence.contains(PRIVATE),
        "reduced evidence never stores the prompt"
    );
}

#[test]
fn s4_held_stats_lock_keeps_the_real_spawn_successful_and_names_the_counting_gap() {
    let rig = rig("s4busy");
    let held = ae::store::lock(&rig.scratch.join("stats.lock"), std::time::Duration::ZERO)
        .expect("test holds the root lock");
    let (code, stdout, stderr) =
        rig.run(ae::cli::SPAWN, &["scout", "--using", "fake", "--", PRIVATE]);
    assert_eq!(code, Some(0), "{stdout}\n{stderr}");
    assert!(stdout.starts_with("Spawned scout in pane %"), "{stdout}");
    assert!(
        stderr.contains("birth not counted in statistics yet")
            && stderr.contains("ae app counts it from the journal"),
        "site 1 must say that journal catch-up is needed: {stderr}"
    );
    let warnings = stderr
        .lines()
        .filter(|line| line.contains("birth not counted in statistics yet"))
        .count();
    assert_eq!(warnings, 1, "the linked-spawn site stays silent");
    assert!(
        rig.events()
            .lines()
            .filter_map(|line| ae::json::parse(line).ok())
            .any(|record| text(&record, "action") == Some("worker-birth")),
        "the journal remains authoritative"
    );
    drop(held);
}

#[test]
fn s4_failed_stats_lock_keeps_the_real_spawn_successful_and_names_the_counting_gap() {
    let rig = rig("s4badlock");
    std::fs::create_dir(rig.scratch.join("stats.lock")).expect("a nonregular lock node");
    let (code, stdout, stderr) =
        rig.run(ae::cli::SPAWN, &["scout", "--using", "fake", "--", PRIVATE]);
    assert_eq!(code, Some(0), "{stdout}\n{stderr}");
    assert!(stdout.starts_with("Spawned scout in pane %"), "{stdout}");
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.contains("birth not counted in statistics yet"))
            .count(),
        1,
        "one certificate warning, linked-spawn silent: {stderr}"
    );
    assert!(
        stderr.contains("stats.lock"),
        "the failure names the refused lock: {stderr}"
    );
}
