//! Independent #309 edge oracle: lead ruling ae-20261106T150440Z-21546d25.
//! An unpinned Twin must fail closed under a floor; the no-floor path is unchanged.

#![allow(
    clippy::disallowed_methods,
    reason = "acceptance fixture owns and removes only its private scratch directory"
)]

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn leadfloor2_unpinned_twin_is_not_a_proven_same_model_under_a_floor() {
    const NOW: i64 = 1_800_000_000;
    for slot in ["main", "worker.0"] {
        for floor in [
            "",
            "auto_reseat_lead_floor = 3\n",
            "auto_reseat_lead_floor = bad\n",
        ] {
            let root = Root(std::env::temp_dir().join(format!(
                "ae-lf2-unpinned-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )));
            for suffix in ["sessions/check", "own", "other"] {
                std::fs::create_dir_all(root.0.join(suffix)).expect("private fixture");
            }
            let base = root.0.canonicalize().expect("canonical private root");
            let dir = base.join("sessions/check");
            let file = base.join("config");
            let config = format!(
                "[profiles]\nsource = \"CLAUDE_CONFIG_DIR={} claude\"\ntwin = \"CLAUDE_CONFIG_DIR={} claude\"\n[workspace]\nauto_reseat = all\n{floor}",
                base.join("own").display(),
                base.join("other").display()
            );
            std::fs::write(&file, &config).expect("private config");
            let bytes = format!(
                "schema=2\nsession=check\nconfig={}\norigin={}\nwork_dir={}\nlayout=lead-pair\nseat.{slot}=subject\nprofile.{slot}=source\nagent_bin.{slot}=claude\nconfig_home.{slot}={}\n",
                file.display(), base.display(), base.display(), base.join("own").display()
            ).into_bytes();
            std::fs::write(dir.join("meta"), &bytes).expect("private meta");
            let meta = crate::meta::Meta::parse(std::str::from_utf8(&bytes).expect("utf8"));
            let row = meta
                .roster()
                .iter()
                .find(|row| row.slot == slot)
                .expect("subject");
            let list = listed(&dir, &bytes, row, &[]);
            let judged = candidates(
                &list,
                |profile| crate::reseat::resolved(&dir, profile),
                (&[], &[]),
                None,
                &[],
                NOW,
            );
            assert_eq!(judged.len(), 1, "one derived off-account Twin");
            let raw = crate::autoreseat::settings_in(&file, &config);
            let settings = raw.seat_view(true, slot);
            let choice = crate::autoreseat::choose_seat(
                &settings,
                crate::autoreseat::Route::Move {
                    seat_profile: "source",
                },
                &judged,
                NOW,
                None,
                None,
            );
            if floor.is_empty() {
                assert_eq!(
                    choice.pick.as_ref().map(|(name, _)| name.as_str()),
                    Some("twin"),
                    "no-floor behavior must remain unchanged: {choice:?}"
                );
            } else {
                assert!(
                    choice.pick.is_none(),
                    "an unreadable model cannot prove a Twin exemption: {choice:?}"
                );
                let (_, why) = refusal(&choice, None);
                assert!(why.contains("twin, derived twin, model unknown"), "{why}");
                if floor.contains(" = 3") {
                    assert!(why.contains("no tier"), "{why}");
                } else {
                    assert!(why.contains("unusable"), "{why}");
                }
                let forecast = deadline("subject", &choice, 0);
                assert!(!forecast.contains("will move"), "{forecast}");
            }
        }
    }
}
