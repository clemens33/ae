//! #309 part A: independent acceptance, human brief + lead F1-F5 are the oracle.
//! Exercise existing APIs so the unmodified base compiles and fails behaviorally.

#![allow(
    clippy::disallowed_methods,
    reason = "independent unit fixtures own and remove only their private scratch directories"
)]

use super::*;
use crate::autoreseat::{self as ar, Choice, Kin, PairGuard, Step};
use crate::tool::Provider;
use std::sync::atomic::{AtomicU64, Ordering};

const NOW: i64 = 1_800_000_000;
const REASON: &str = "a lead seat never changes model unasked";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    dir: PathBuf,
    bytes: Vec<u8>,
    slot: String,
}

impl Fixture {
    fn new(layout: &str, slot: &str, twin: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "ae-leadfloor-unit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("sessions/check")).expect("private session");
        for account in ["own", "away", "twin"] {
            std::fs::create_dir_all(root.join(account)).expect("private accounts");
        }
        let root = root.canonicalize().expect("canonical private root");
        let dir = root.join("sessions/check");
        let command = |account: &str, model: &str| {
            format!(
                "CLAUDE_CONFIG_DIR={} claude --model {model}",
                root.join(account).display()
            )
        };
        let mut text = format!(
            "[profiles]\nsource = \"{}\"\nsame = \"{}\"\naway = \"{}\"\n",
            command("own", "opus"),
            command("own", "haiku"),
            command("away", "sonnet")
        );
        if twin {
            text.push_str(&format!("twin = \"{}\"\n", command("twin", "opus")));
        }
        text.push_str("[workspace]\nauto_reseat = all\n");
        let config = root.join("config");
        std::fs::write(&config, text).expect("private identity config");
        let bytes = format!(
            "schema=2\nsession=check\nconfig={}\norigin={}\nwork_dir={}\nlayout={layout}\nseat.{slot}=subject\nprofile.{slot}=source\nagent_bin.{slot}=claude\nconfig_home.{slot}={}\n",
            config.display(), root.display(), root.display(), root.join("own").display()
        ).into_bytes();
        std::fs::write(dir.join("meta"), &bytes).expect("private meta");
        Self {
            root,
            dir,
            bytes,
            slot: slot.to_owned(),
        }
    }

    fn judged(&self, declared: &[String]) -> Vec<Candidate> {
        let text = std::str::from_utf8(&self.bytes).expect("fixture utf8");
        let meta = crate::meta::Meta::parse(text);
        let row = meta
            .roster()
            .iter()
            .find(|row| row.slot == self.slot)
            .expect("recorded subject");
        let list = listed(&self.dir, &self.bytes, row, declared);
        candidates(
            &list,
            |profile| crate::reseat::resolved(&self.dir, profile),
            (&[], &[]),
            None,
            &[],
            NOW,
        )
    }

    fn choose(&self, declared: &[String], room: Option<u8>) -> Choice {
        ar::choose_with(&self.judged(declared), NOW, room)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn leadfloor_both_lead_slots_pass_over_every_sibling_step_and_name_the_refusal() {
    for slot in ["main", "worker.0"] {
        let fixture = Fixture::new("lead-pair", slot, false);
        for room in [None, Some(95)] {
            let choice = fixture.choose(&[], room);
            assert!(choice.pick.is_none(), "{slot} {room:?}: {choice:?}");
            assert_eq!(choice.skipped.len(), 2, "siblings stay visible: {choice:?}");
            let (class, why) = refusal(&choice, None);
            assert_eq!(class, CauseClass::NoCandidate);
            assert!(why.contains("no usable candidate"), "{why}");
            for name in ["same", "away"] {
                assert!(
                    why.contains(&format!("{name}, derived sibling ({REASON})")),
                    "{why}"
                );
            }
            let forecast = deadline("subject", &choice, 0);
            assert!(forecast.contains(REASON), "{forecast}");
            assert!(!forecast.contains("will move"), "{forecast}");
        }
    }
}

#[test]
fn leadfloor_a_declared_model_change_and_an_off_account_twin_remain_usable() {
    for slot in ["main", "worker.0"] {
        let fixture = Fixture::new("lead-pair", slot, true);
        let twin = fixture.choose(&[], None);
        assert_eq!(
            twin.pick.as_ref().map(|(name, _)| name.as_str()),
            Some("twin")
        );
        assert_eq!(
            twin.provenance,
            Provenance::Derived(Kin::Twin, Step::Account)
        );
        assert_eq!(
            twin.skipped.len(),
            2,
            "the kept sibling cannot outrank a twin"
        );
        let note = attempt_note("headroom 95% weekly_all 7d", &twin.skipped);
        assert!(note.contains(REASON), "{note}");
        let explicit = fixture.choose(&["same".to_owned()], None);
        assert_eq!(
            explicit.pick.as_ref().map(|(name, _)| name.as_str()),
            Some("same")
        );
        assert_eq!(explicit.provenance, Provenance::Declared);
    }
}

#[test]
fn leadfloor_other_layouts_and_nonlead_slots_keep_the_existing_sibling_choice() {
    for (layout, slot) in [
        ("vertical", "main"),
        ("vertical", "worker.0"),
        ("lead-pair", "worker.1"),
        ("lead-pair", "spawned.0"),
    ] {
        let fixture = Fixture::new(layout, slot, true);
        let choice = fixture.choose(&[], None);
        assert_eq!(
            choice.pick.as_ref().map(|(name, _)| name.as_str()),
            Some("same"),
            "{layout} {slot}: {choice:?}"
        );
        assert_eq!(
            choice.provenance,
            Provenance::Derived(Kin::Sibling, Step::Kept)
        );
        assert!(choice.skipped.is_empty(), "{choice:?}");
    }
}

#[test]
fn leadfloor_an_already_collapsed_pair_fallback_cannot_restore_a_barred_sibling() {
    for twin in [false, true] {
        let fixture = Fixture::new("lead-pair", "main", twin);
        let candidates = fixture.judged(&[]);
        let pair = PairGuard {
            partner: Some("peer".to_owned()),
            partner_serves: Some(Provider::Anthropic),
            seat_serves: Some(Provider::Anthropic),
            destinations: candidates
                .iter()
                .map(|candidate| (candidate.profile.clone(), Some(Provider::Anthropic)))
                .collect(),
        };
        let choice = ar::choose_paired(&candidates, NOW, None, Some(&pair));
        assert_eq!(
            choice.pick.as_ref().map(|(name, _)| name.as_str()),
            twin.then_some("twin"),
            "{choice:?}"
        );
        for (_, skip) in choice.skipped {
            assert_eq!(skip_word(skip), REASON);
        }
    }
}

#[test]
fn leadfloor_urgent_start_waives_waits_but_never_the_sibling_restriction() {
    let fixture = Fixture::new("lead-pair", "main", false);
    let settings = ar::settings_in(
        &fixture.root.join("config"),
        "[workspace]\nauto_reseat = all\nauto_reseat_at = off\nauto_reseat_grace_secs = 600\n",
    );
    let seat = ar::Seat {
        session: "check",
        slot: "main",
        agent: "subject",
        profile: "source",
        orchestrator: false,
    };
    let now = Timestamp::from_epoch(NOW);
    let event = crate::events::Event::parse_line(&format!(
        "{{\"ts\":\"{now}\",\"actor\":\"watchdog\",\"action\":\"auto-reseat-headroom\",\"target\":\"subject\",\"target_slot\":\"main\",\"target_session\":\"check\",\"summary\":\"exhausted start\"}}"
    )).expect("urgent episode");
    let start = ar::StartRead {
        settled: Some(NOW),
        exhausted: Some(ar::Window {
            judged: 100.0,
            critical: true,
            long: false,
            observed_at: NOW,
            resets_at: Some(NOW + 3600),
            status: crate::quota::Status::Fresh,
            label: "session 5h".to_owned(),
        }),
        brief_armed: false,
        quiet: true,
    };
    let sight = Sight {
        pane: ar::Pane {
            frame: ar::Frame::Unproven,
            human_prompt: false,
            client_input: Some(NOW - 60),
        },
        limited: false,
        near: true,
    };
    let plan = plan_with(
        &settings,
        &seat,
        &sight,
        &start,
        &[event],
        |declared, _| fixture.judged(declared),
        now,
    );
    assert!(
        matches!(plan,
            Plan::Refuse { class: CauseClass::NoCandidate, ref why, .. }
                if why.contains(REASON)
        ),
        "urgent path must reach named refusal: {plan:?}"
    );
}
