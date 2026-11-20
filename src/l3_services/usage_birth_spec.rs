//! Frozen stats287 s1 test 6: birth certificates never supply usage lifetimes.
//! Child of usage.rs solely to exercise its existing private retired fold.
#![allow(
    clippy::expect_used,
    reason = "independently authored retired-journal fixture"
)]

#[test]
fn statistics_family_records_do_not_change_retired_usage_candidates_or_coverage() {
    for slot in ["", ",\"target_slot\":\"spawned.0\""] {
        let spawn = format!(
            "{{\"ts\":\"2026-11-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"spawn\",\"target\":\"scout\",\"summary\":\"task\"{slot}}}\n"
        );
        let retire = "{\"ts\":\"2026-11-01T10:00:10Z\",\"actor\":\"lead\",\"action\":\"retire\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"summary\":\"tool=claude profile=p config_home= config_home_base=\"}\n";
        let birth = "{\"ts\":\"2026-11-01T10:00:01Z\",\"actor\":\"lead\",\"action\":\"worker-birth\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session_id\":\"11111111-1111-4111-8111-111111111111\",\"birth_v\":1,\"birth\":\"22222222-2222-4222-8222-222222222222\",\"birth_tool\":\"claude\",\"birth_profile\":\"p\"}\n";
        let fact = "{\"ts\":\"2026-11-01T10:00:02Z\",\"actor\":\"watchdog\",\"action\":\"worker-fact\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session_id\":\"11111111-1111-4111-8111-111111111111\",\"birth_v\":1,\"birth\":\"22222222-2222-4222-8222-222222222222\",\"fact\":\"start-attempted\",\"start\":\"spawned.0:22222222-2222-4222-8222-222222222222:41\"}\n";
        let mut before = Vec::new();
        let mut before_audit = crate::profile_view::Audit::default();
        super::profile_retired(
            format!("{spawn}{retire}").as_bytes(),
            "s",
            &mut before,
            &mut before_audit,
        );
        assert_eq!(before.len(), 1, "a real retirement is reached");
        assert_eq!(before[0].0.name, "scout");
        assert_eq!(before[0].1, if slot.is_empty() { None } else { Some(10) });
        assert!(before[0].3);
        let mut after = Vec::new();
        let mut after_audit = crate::profile_view::Audit::default();
        super::profile_retired(
            format!("{birth}{spawn}{fact}{retire}{birth}{fact}").as_bytes(),
            "s",
            &mut after,
            &mut after_audit,
        );
        assert_eq!(format!("{after:?}"), format!("{before:?}"));
        assert_eq!(after_audit.gaps, before_audit.gaps);
        assert_eq!(after_audit.notes, before_audit.notes);
    }
}
