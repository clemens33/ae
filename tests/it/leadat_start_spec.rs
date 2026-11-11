//! #305 part 3: lead headroom override never masks urgent exhausted-start moves.
use super::*;

#[test]
fn leadat_control_urgent_lead_pair_moves_with_lead_threshold_off_or_numeric() {
    for (session, value) in [("latstartoff", "off"), ("latstart96", "96")] {
        let rig = fixture(
            session,
            &format!("auto_reseat_lead_at = {value}\n"),
            "destination",
        );
        let config = std::fs::read_to_string(&rig.config).expect("config");
        std::fs::write(
            &rig.config,
            config
                .replace("lead = source\n", "lead = source\ncolead = source\n")
                .replace("layout = vertical", "layout = lead-pair\nworkers = colead"),
        )
        .expect("paired roster");
        launch(&rig, session);
        retained_slot(&rig, session, "main");
        retained_slot(&rig, session, "worker.0");
        quota(&rig, "source", "session", 100, 0, None);
        stop(&rig, session);
        launch(&rig, session);
        let mut daemon = watch(&rig, session);
        until_stepped(&rig, session, &mut daemon, || {
            [("main", "lead"), ("worker.0", "colead")]
                .iter()
                .all(|(slot, name)| booked_slot(&rig, session, DONE_ACTION, slot, name).len() == 1)
        });
        for (slot, name) in [("main", "lead"), ("worker.0", "colead")] {
            assert_eq!(
                row(&rig.meta(session), &format!("profile.{slot}")),
                "destination"
            );
            assert_eq!(
                booked_slot(&rig, session, ATTEMPT_ACTION, slot, name).len(),
                1
            );
        }
    }
}
