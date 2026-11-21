//! The fleet feed's rename rows: one when a live rename completes, none while
//! a checked publication still refuses it, and one for a session with no meta.

use super::*;

fn live(tag: &str) -> Rig {
    let rig = Rig::new(tag);
    ae::meta::rewrite(&rig.dir, "watchdog", Some("false")).expect("watchdog off");
    write_exec(&rig.dir.join("events-tail"), RENAME_MONITOR_CORE);
    rig
}

fn renamed(root: &Path) -> Vec<(String, String, String)> {
    ae::fleetfeed::read(root)
        .rows
        .into_iter()
        .filter(|row| row.what == ae::fleetfeed::What::Renamed)
        .map(|row| (row.session, row.note, row.actor))
        .collect()
}

#[test]
fn a_completed_live_rename_writes_one_fleet_row() {
    let rig = live("lfrenok");
    let old = rig.name.clone();
    let result = rig.run(&["rename", &old, "fresh"]);
    assert_eq!(result.0, Some(0), "{result:?}");
    assert_eq!(
        renamed(&rig.home),
        [("fresh".to_owned(), old, "owner".to_owned())]
    );
}

#[test]
fn a_rename_a_checked_publication_refuses_writes_no_fleet_row() {
    let rig = live("lfrenrefused");
    std::fs::create_dir(rig.dir.join("workspace.md")).expect("manifest obstruction");
    let result = rig.run(&["rename", &rig.name, "fresh"]);
    assert_eq!(result.0, Some(1), "manifest refusal: {result:?}");
    assert!(
        renamed(&rig.home).is_empty(),
        "a refused rename is no rename event: {:?}",
        renamed(&rig.home)
    );
}

#[test]
fn a_live_rename_of_a_session_with_no_meta_writes_one_fleet_row() {
    let rig = AmbientRig::new("lfrenbare");
    let old = rig.plant("old", "watchdog=false\n");
    std::fs::remove_file(old.join("meta")).expect("no meta");
    let result = rig.run(&["rename", "old", "new"]);
    assert_eq!(result.0, Some(0), "{result:?}");
    assert_eq!(
        renamed(&rig.home),
        [("new".to_owned(), "old".to_owned(), "owner".to_owned())]
    );
}
