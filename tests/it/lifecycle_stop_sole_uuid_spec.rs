//! Independent acceptance for the shared confirmed Stop owner.
//! Child of tests/it/lifecycle.rs; real supervisor, lock and private tmux.

use super::*;

#[test]
fn duplicate_raw_uuid_refuses_before_migration_journal_and_kill() {
    for (tag, suffix) in [
        ("same", format!("session_id={UUID}\n")),
        (
            "other",
            "session_id=44444444-4444-4444-4444-444444444444\n".to_owned(),
        ),
        ("bad", "session_id=not-a-uuid\n".to_owned()),
    ] {
        let rig = Rig::new(&format!("menu-sole-{tag}"));
        let client = attach_client(&rig, &format!("vsole-{tag}"));
        let raw = std::fs::read_to_string(rig.dir.join("meta")).expect("raw meta");
        // Legacy schema exposes any migration before proof as a byte change.
        let raw = raw.replace("schema=2\n", "");
        std::fs::write(rig.dir.join("meta"), format!("{raw}{suffix}")).expect("duplicate UUID");
        for leaf in [
            "events.jsonl",
            "memory.md",
            "conversations",
            ".launch-attempt",
        ] {
            std::fs::write(rig.dir.join(leaf), "independent sentinel\n").expect("state sentinel");
        }
        let leaves = [
            "meta",
            "events.jsonl",
            "memory.md",
            "conversations",
            ".launch-attempt",
        ];
        let before: Vec<_> = leaves
            .iter()
            .map(|leaf| std::fs::read(rig.dir.join(leaf)).expect("before"))
            .collect();
        let mut argv = vec![
            "_stop".to_owned(),
            "--supervise".to_owned(),
            rig.name.clone(),
        ];
        argv.extend(expectation(&rig, now() + 60, (&client.0, &client.1)));
        let borrowed: Vec<_> = argv.iter().map(String::as_str).collect();
        let (code, out, err) = rig.run(&borrowed);
        assert_ne!(
            code,
            Some(0),
            "{tag}: duplicate identity earned Stop: {out} {err}"
        );
        assert!(
            err.contains("state directory") || err.contains("session_id"),
            "{tag}: identity refusal required: {err}"
        );
        assert!(
            rig.session_is_live(),
            "{tag}: duplicate identity killed the target"
        );
        for (leaf, before) in leaves.iter().zip(before) {
            assert_eq!(
                std::fs::read(rig.dir.join(leaf)).expect("after"),
                before,
                "{tag}: refused target wrote {leaf}"
            );
        }
    }
}

#[test]
fn sole_uuid_valid_control_still_completes_shared_owner() {
    let rig = Rig::new("menu-sole-control");
    let client = attach_client(&rig, "vsole-control");
    let mut argv = vec![
        "_stop".to_owned(),
        "--supervise".to_owned(),
        rig.name.clone(),
    ];
    argv.extend(expectation(&rig, now() + 60, (&client.0, &client.1)));
    let borrowed: Vec<_> = argv.iter().map(String::as_str).collect();
    let (code, out, err) = rig.run(&borrowed);
    assert_eq!(code, Some(0), "valid sole UUID: {out} {err}");
    assert!(!rig.session_is_live());
    assert!(
        rig.dir.join("meta").is_file(),
        "Stop preserves the state directory"
    );
    let events = std::fs::read_to_string(rig.dir.join("events.jsonl")).expect("stop journal");
    assert!(
        events.contains("stop-request") && events.contains("stop-result"),
        "shared owner did not finish: {events}"
    );
}

#[test]
fn another_directory_with_same_uuid_does_not_disable_row_owner() {
    let rig = Rig::new("menu-shared-uuid");
    let other = rig.home.join("sessions/rename-other");
    std::fs::create_dir_all(&other).expect("other row");
    std::fs::write(other.join("meta"), format!("schema=2\nsession_id={UUID}\n"))
        .expect("same UUID in other row");
    let before = std::fs::read(other.join("meta")).expect("other bytes");
    let client = attach_client(&rig, "vshared-uuid");
    let mut argv = vec![
        "_stop".to_owned(),
        "--supervise".to_owned(),
        rig.name.clone(),
    ];
    argv.extend(expectation(&rig, now() + 60, (&client.0, &client.1)));
    let borrowed: Vec<_> = argv.iter().map(String::as_str).collect();
    let (code, out, err) = rig.run(&borrowed);
    assert_eq!(
        code,
        Some(0),
        "fleet-wide uniqueness is not Stop authority: {out} {err}"
    );
    assert!(!rig.session_is_live());
    assert_eq!(
        std::fs::read(other.join("meta")).expect("other after"),
        before
    );
}
