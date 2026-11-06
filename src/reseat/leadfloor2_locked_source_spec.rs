//! #309 late-gate oracle: the source under the lifecycle lock owns exemptions.
//! A queued move must not use the model of a predecessor that another reseat replaced.

#![allow(
    clippy::disallowed_methods,
    reason = "acceptance fixtures own and remove only private files"
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
#[allow(
    clippy::too_many_lines,
    reason = "the admitted and locked snapshots stay together in one acceptance transaction"
)]
fn leadfloor2_late_floor_uses_locked_source_not_admitted_source() {
    let owned = Root(std::env::temp_dir().join(format!(
        "ae-lf2-locked-source-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir_all(&owned.0).expect("private root");
    let root = owned.0.canonicalize().expect("canonical root");
    assert_eq!(
        crate::doors::config_file(crate::shape::current(), &root),
        root.join("config"),
        "fixture requires CONFIG_FILE unset; never write through an ambient config override"
    );
    let dir = root.join("sessions/check");
    for path in [&dir, &root.join("own"), &root.join("away")] {
        std::fs::create_dir_all(path).expect("private fixture directory");
    }
    for slot in ["main", "worker.0"] {
        for (floor, locked_profile, refused) in [
            ("3", "high", true),
            ("3", "unreadable", true),
            ("3", "low", false),
            ("", "high", false),
        ] {
            let floor = if floor.is_empty() {
                String::new()
            } else {
                format!("auto_reseat_lead_floor = {floor}\n")
            };
            let config = format!(
                "[profiles]\nlow = \"CLAUDE_CONFIG_DIR={} claude --model haiku\"\nhigh = \"CLAUDE_CONFIG_DIR={} claude --model opus\"\ndestination = \"CLAUDE_CONFIG_DIR={} claude --model haiku\"\npeer = \"codex -m gpt-6\"\n[workspace]\nauto_reseat = all\n{floor}[model_tiers]\nsmall = haiku, 1\nlarge = opus, 4\n",
                root.join("own").display(),
                root.join("own").display(),
                root.join("away").display()
            );
            let file = root.join("config");
            std::fs::write(&file, config).expect("private global config");
            let partner = if slot == "main" { "worker.0" } else { "main" };
            let meta = |profile: &str| {
                format!(
                    "schema=2\nsession=check\nconfig={}\norigin={}\nlayout=lead-pair\nseat.{slot}=subject\nprofile.{slot}={profile}\nagent_bin.{slot}=claude\nseat.{partner}=partner\nprofile.{partner}=peer\nagent_bin.{partner}=codex\n",
                    file.display(),
                    root.display()
                )
            };
            let admitted = meta("low");
            let old = crate::meta::Meta::parse(&admitted);
            let row = old
                .roster()
                .iter()
                .find(|row| row.slot == slot)
                .expect("admitted seat");
            let from_command = recorded_command(&dir, admitted.as_bytes(), row);
            assert!(
                from_command
                    .as_deref()
                    .is_some_and(|command| command.contains("--model haiku"))
            );
            let moving = resolve_profile(&dir, admitted.as_bytes(), "destination", "subject")
                .expect("real destination resolution");
            let entry = crate::digest::SessionEntry::new("check", crate::digest::Status::Running);
            let seat = Admitted {
                entry: &entry,
                dir: dir.clone(),
                caller: Caller::default(),
                was: "profile 'low'".to_owned(),
                from: "low".to_owned(),
                to: "destination".to_owned(),
                moving,
                from_command,
                target: Target {
                    agent: "subject".to_owned(),
                    slot: slot.to_owned(),
                    pane: "%fixture".to_owned(),
                    server: ServerId::from_typed_flags(
                        "socket",
                        &root.join("unused.sock").to_string_lossy(),
                    )
                    .expect("private selector"),
                },
            };
            let parsed = Parsed {
                session: "check".to_owned(),
                agent: "subject".to_owned(),
                profile: "destination".to_owned(),
                client: None,
                stop: StopPolicy::Proven,
                automatic: true,
                long_limit: None,
                expect: None,
                ended: std::cell::RefCell::default(),
            };
            let ask = Ask {
                root: &root,
                world: None,
                parsed: &parsed,
                now: Timestamp::from_epoch(1_800_000_000),
            };
            // The lifecycle predecessor has now published its different source.
            let locked = meta(locked_profile);
            std::fs::write(dir.join("meta"), &locked).expect("current locked metadata");
            let mut err = Vec::new();
            let result = prepare(&ask, &seat, locked.as_bytes(), &mut err);
            let err = String::from_utf8(err).expect("utf8 diagnostic");
            if refused {
                assert!(
                    matches!(
                        result,
                        Err(Halt::End(Ended::Refused(CauseClass::FloorChanged)))
                    ),
                    "{slot}: locked source {locked_profile} must remove the stale same-model exemption: {err}"
                );
                assert!(err.contains("tier 1 is below the lead floor 3"), "{err}");
                assert!(err.contains("nothing was stopped"), "{err}");
            } else {
                assert!(
                    result.is_ok(),
                    "{slot}: no floor or a proven same-model source remains allowed: {err}"
                );
            }
            assert_eq!(
                std::fs::read_to_string(dir.join("meta")).expect("meta after prepare"),
                locked
            );
        }
    }
}
