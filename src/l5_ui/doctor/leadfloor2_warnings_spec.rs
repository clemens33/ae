//! #309 diagnostic oracle: real floor parse problems reach the doctor rows.

use super::*;

#[test]
fn leadfloor2_doctor_reports_real_floor_and_tier_problems_without_other_notes() {
    let config = "[workspace]\nauto_reseat = all\nauto_reseat_at = bad\nauto_reseat_lead_floor = bad\n[model_tiers]\none = sonnet-5-5, 1\ntwo = sonnet-5-5, 3\nbad = haiku, +3\n";
    let settings = crate::autoreseat::settings_in(Path::new("/spec/leadfloor2"), config);
    assert!(
        settings
            .notes
            .iter()
            .any(|note| note.starts_with("auto_reseat_at")),
        "the parser must produce an unrelated warning for the filter control"
    );
    let mut found = Report::default();
    reseat_floor_rows(&reseat_floor_facts(&settings), &mut found);
    let rows = found.rows();
    assert_eq!(rows.len(), 3, "{}", found.render());
    assert!(rows.iter().all(|row| row.level == Level::Warn));
    assert!(rows.iter().all(|row| row.label == "reseat-floor"));
    for fragments in [
        &["auto_reseat_lead_floor", "bad", "0 to 255"][..],
        &["[model_tiers]", "bad", "+3", "ignored"][..],
        &["[model_tiers]", "one = 1", "two = 3", "different tiers"][..],
    ] {
        assert!(
            rows.iter()
                .any(|row| fragments.iter().all(|part| row.detail.contains(part))),
            "missing real diagnostic {fragments:?}: {}",
            found.render()
        );
    }
    assert!(
        rows.iter()
            .all(|row| !row.detail.starts_with("auto_reseat_at"))
    );
    assert_eq!(found.failures(), 0, "warnings never fail doctor");
    assert_eq!(found.exit_code(), 0);
}

#[test]
fn leadfloor2_doctor_valid_floor_and_off_switch_have_no_phantom_warnings() {
    for (config, detail) in [
        (
            "[workspace]\nauto_reseat = all\nauto_reseat_lead_floor = 3\n[model_tiers]\nlarge = opus, 4\n",
            "lead floor 3, 1 model tier row(s)",
        ),
        ("[workspace]\nauto_reseat = all\n", "no lead floor set"),
        (
            "[workspace]\nauto_reseat = off\nauto_reseat_lead_floor = bad\n[model_tiers]\nbad = haiku, +3\n",
            "skipped (auto_reseat off)",
        ),
    ] {
        let settings = crate::autoreseat::settings_in(Path::new("/spec/leadfloor2"), config);
        let mut found = Report::default();
        reseat_floor_rows(&reseat_floor_facts(&settings), &mut found);
        assert_eq!(
            found.rows(),
            &[Row {
                level: Level::Ok,
                label: "reseat-floor".to_owned(),
                detail: detail.to_owned()
            }]
        );
        assert_eq!(found.warnings(), 0);
        assert_eq!(found.exit_code(), 0);
    }
}
