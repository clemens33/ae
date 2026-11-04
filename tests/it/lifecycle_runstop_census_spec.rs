//! Supplemental run-stop characterisation on unrefactored c1f4549f.
//!
//! BASELINE BEHAVIOUR B5: caller resolution follows a valid session record
//! through a symlink, but the fleet census skips that directory entry.
//! Preserve this mismatch during the refactor; this pin does not fix it.

use super::*;

/// A resolved caller that IS in the fleet must delegate confirmation to its
/// attached client, even when this command itself has no terminal.
#[test]
fn runstop_resolved_caller_in_census_keeps_fleet_confirmation_route() {
    let rig = Rig::new("rsinfleet");
    let before = guard_spec::tree(&rig.dir);
    assert_eq!(
        rig.run_inside(&["_stop", "all", "--pane", &rig.pane]),
        (
            Some(1),
            String::new(),
            "Error: nobody attached to confirm; pass -y.\n".to_owned(),
        ),
        "the resolved fleet caller delegates confirmation to its client"
    );
    assert_eq!(guard_spec::tree(&rig.dir), before, "no state writes");
    assert!(rig.session_is_live(), "unconfirmed fleet is still running");
}

#[test]
fn runstop_resolved_caller_absent_from_census_keeps_public_all_route() {
    let rig = Rig::new("rscensus");
    let record = rig.home.join("linked-record");
    std::fs::rename(&rig.dir, &record).expect("move this fixture's record");
    std::os::unix::fs::symlink(&record, &rig.dir).expect("link the session record");
    let before = guard_spec::tree(&record);

    // Prove that this same linked record still resolves the caller. A public
    // single-target stop would proceed rather than demand self confirmation.
    assert_eq!(
        rig.run_inside(&["_stop", &rig.name, "--pane", &rig.pane]),
        (
            Some(1),
            String::new(),
            "Error: nobody attached to confirm; pass -y.\n".to_owned(),
        ),
        "the linked session is the positively resolved caller"
    );
    assert_eq!(guard_spec::tree(&record), before);
    assert!(rig.session_is_live());

    assert_eq!(
        rig.run_inside(&["_stop", "all", "-y", "--pane", &rig.pane]),
        (
            Some(0),
            "No running ae sessions.\n".to_owned(),
            String::new(),
        ),
        "a resolved caller missing from the census is not the fleet route"
    );
    assert_eq!(guard_spec::tree(&record), before, "no state writes");
    assert_eq!(
        std::fs::read_link(&rig.dir).expect("the record link survives"),
        record
    );
    assert!(
        rig.session_is_live(),
        "the census did not select this session"
    );
}
