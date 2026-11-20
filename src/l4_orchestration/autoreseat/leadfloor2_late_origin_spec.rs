//! #309 ruling 3: only a proven chain's exact origin exempts a late return.

#![allow(
    clippy::disallowed_methods,
    reason = "acceptance owns and removes only its private journal fixture"
)]

use super::*;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn chain_journal(slot: &str) -> String {
    const NOW: i64 = 1_800_000_000;
    let key = Timestamp::from_epoch(NOW - 100).to_string();
    let mut journal = String::new();
    for (offset, action, reference, summary) in [
        (-100, "limit", "", String::new()),
        (
            -99,
            super::super::ATTEMPT_ACTION,
            key.as_str(),
            String::new(),
        ),
        (
            -98,
            "reseat",
            "",
            format!("reseated subject (pane %1, slot {slot}) to source, seed submitted"),
        ),
        (-97, super::super::DONE_ACTION, "origin", "moved".to_owned()),
    ] {
        writeln!(
            journal,
            "{{\"ts\":\"{}\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"subject\",\"target_slot\":\"{slot}\",\"target_session\":\"check\",\"ref\":\"{reference}\",\"summary\":\"{summary}\"}}",
            Timestamp::from_epoch(NOW + offset)
        )
        .expect("string journal");
    }
    journal
}

#[test]
fn leadfloor2_late_return_exempts_exact_proven_chain_origin_only() {
    let owned = Root(std::env::temp_dir().join(format!(
        "ae-lf2-late-origin-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir_all(&owned.0).expect("private root");
    let settings = crate::autoreseat::settings_in(
        &owned.0.join("config"),
        "[workspace]\nauto_reseat = all\nauto_reseat_lead_floor = 3\n",
    );
    let destination = model_of_command("claude --model haiku");
    for slot in ["main", "worker.0"] {
        let locked = format!("layout=lead-pair\nseat.{slot}=subject\nprofile.{slot}=source\n");
        for (journal, returning, to, allowed) in [
            (chain_journal(slot), true, "origin", true),
            (chain_journal(slot), true, "other", false),
            (String::new(), true, "origin", false),
            (chain_journal(slot), false, "origin", false),
        ] {
            std::fs::write(owned.0.join("events.jsonl"), journal).expect("private history");
            let refusal = floor_changed(
                &settings,
                (&owned.0, locked.as_bytes()),
                ("check", slot, "subject"),
                (Some("claude --model opus"), to, &destination),
                returning,
            );
            assert_eq!(
                refusal.is_none(),
                allowed,
                "{slot}/{returning}/{to}: {refusal:?}"
            );
            if let Some(why) = refusal {
                assert!(why.contains("no tier for its model"), "{why}");
                assert!(why.contains("nothing was stopped"), "{why}");
            }
        }
    }
}
