// B5 accounting acceptance. Real helper/World fixtures; no product outcome mock.
use super::*;

const KEYS: [&str; 3] = ["_AE_EXPECT_SLOT", "_AE_EXPECT_LAUNCH", "_AE_EXPECT_LEGACY"];
const ROLE: &str = "orchestrator role refuses unproven/outside initiation; use its pane, ae chat, ae app or Telegram";
const OTHER: &str = "0199c0de-2222-4890-abcd-ef0123456789";

#[derive(Clone, Copy)]
enum Case {
    Stale,
    Malformed,
    Conflict,
    Slot,
    RoleAllowed,
    RoleEarly,
    RoleMatching,
    RoleAbsent,
    Guard,
    GuardAbsent,
    SeatMatching,
    SeatAbsent,
    RawExpected,
    RawAbsent,
    RawPhysicalExpected,
    RawPhysicalAbsent,
    PastedExpected,
    PastedAbsent,
    EnteredExpected,
    EnteredAbsent,
}

fn command(w: &World, caller: &str, target: &str, action: &str, expected: bool) -> Runner {
    let mut cmd = w.rig.command(Some(caller));
    cmd.args(["@s", "send"]);
    if caller != w.main {
        cmd.arg("--cross-session");
    }
    cmd.args([target, BODY])
        .env_remove("AE_SENDER_OVERRIDE")
        .env("AE_SEND_DEFER_SEC", "15")
        .env("_AE_EVENT_ACTION", action)
        .env("_AE_EVENT_REF", "b5-reference")
        .env("_AE_EVENT_SUMMARY", BODY)
        .env("_AE_EVENT_ACTOR_SLOT", "main")
        .env(
            "_AE_EVENT_ACTOR_SESSION",
            if caller == w.main { "s" } else { "other" },
        )
        .env(
            "_AE_EVENT_TARGET_SLOT",
            if target == w.pane { "" } else { SLOT },
        )
        .env("_AE_EVENT_TARGET_SESSION", "s")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in KEYS {
        cmd.env_remove(key);
    }
    if expected {
        cmd.env("_AE_EXPECT_SLOT", SLOT)
            .env("_AE_EXPECT_LAUNCH", "tok-kid");
    }
    cmd
}

fn rows(w: &World, session: &str, action: &str) -> Vec<String> {
    read(w.rig.dir(session).join("events.jsonl"))
        .lines()
        .filter(|line| line.contains(&format!("\"action\":\"{action}\"")))
        .map(str::to_owned)
        .collect()
}

fn audit(w: &World, ledger: &str) {
    let here = rows(w, ledger, "refused");
    assert_eq!(
        here.len(),
        1,
        "{}",
        read(w.rig.dir(ledger).join("events.jsonl"))
    );
    assert!(
        !here[0].contains("\"target"),
        "refusal must be unaddressed: {}",
        here[0]
    );
    let other = if ledger == "s" { "other" } else { "s" };
    assert!(rows(w, other, "refused").is_empty(), "wrong refusal ledger");
}

fn attempt(w: &World, action: &str, wanted: usize, foreign: bool) {
    let found = rows(w, "s", action);
    assert_eq!(found.len(), wanted, "{}", w.events());
    assert!(
        rows(w, "other", action).is_empty(),
        "attempt duplicated to caller ledger"
    );
    if wanted == 1 {
        let line = &found[0];
        for field in [
            "\"target\":\"kid\"",
            "\"target_slot\":\"spawned.0\"",
            "\"target_session\":\"s\"",
            "\"actor_slot\":\"main\"",
            "\"ref\":\"b5-reference\"",
        ] {
            assert!(line.contains(field), "missing {field}: {line}");
        }
        assert!(
            line.contains("[unconfirmed]") && line.contains("refused pre-paste: seat not current"),
            "{line}"
        );
        let session = if foreign { "other" } else { "s" };
        assert!(
            line.contains(&format!("\"actor_session\":\"{session}\"")),
            "{line}"
        );
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "Keep each frozen race schedule and its setup and accounting oracles together."
)]
fn run(tag: &str, action: &str, case: Case) {
    let w = World::new(tag);
    let foreign = matches!(
        case,
        Case::RoleEarly | Case::RoleMatching | Case::RoleAbsent
    );
    let caller = if foreign {
        let [main, peer] = w.rig.create("other", OTHER);
        w.rig.world218_capture(tag, [&main, &peer, &w.main]);
        main
    } else {
        w.main.clone()
    };
    let raw = matches!(
        case,
        Case::RawExpected | Case::RawAbsent | Case::RawPhysicalExpected | Case::RawPhysicalAbsent
    );
    if raw {
        for key in ["@ae_slot", "@ae_agent"] {
            w.stamp(key, None);
        }
    }
    let expected = !matches!(
        case,
        Case::GuardAbsent
            | Case::RoleAbsent
            | Case::SeatAbsent
            | Case::RawAbsent
            | Case::RawPhysicalAbsent
            | Case::PastedAbsent
            | Case::EnteredAbsent
    );
    let target = if raw { w.pane.as_str() } else { "kid" };
    let mut cmd = command(&w, &caller, target, action, expected);
    match case {
        Case::Stale => w.change("replace", SLOT),
        Case::Malformed => {
            cmd.env_remove("_AE_EXPECT_LAUNCH");
        }
        Case::Conflict => {
            cmd.env("_AE_EXPECT_LEGACY", "1");
        }
        Case::Slot => {
            cmd.env("_AE_EXPECT_SLOT", "spawned.1");
        }
        Case::RoleAllowed => {
            w.row("meta_agent", Some("true"));
            w.change("replace", SLOT);
        }
        _ => {}
    }
    let physical = matches!(case, Case::RawPhysicalExpected | Case::RawPhysicalAbsent);
    if physical {
        w.barrier(&mut cmd);
        w.resume();
        cmd.env("SEATPROOF_FAULT", "physical")
            .env("SEATPROOF_PANE", &w.pane);
    }
    let stage = match case {
        Case::PastedExpected | Case::PastedAbsent => Some("paste"),
        Case::EnteredExpected | Case::EnteredAbsent => Some("enter"),
        _ => None,
    };
    if let Some(event) = stage {
        w.arm_barrier(&mut cmd, event, SLOT, &w.out);
    }
    let before = w.screen();
    let lock_case = matches!(
        case,
        Case::GuardAbsent
            | Case::Guard
            | Case::RoleEarly
            | Case::RoleMatching
            | Case::RoleAbsent
            | Case::SeatMatching
            | Case::SeatAbsent
    );
    let output = if lock_case {
        let held = w.lock();
        let before_bodies = w.bodies();
        let child = cmd.spawn().expect("SETUP: real helper spawn");
        wait("SETUP: admitted body while target lock held", || {
            w.bodies() > before_bodies
        });
        if matches!(
            case,
            Case::RoleEarly | Case::RoleMatching | Case::RoleAbsent
        ) {
            w.row("meta_agent", Some("true"));
        }
        if matches!(case, Case::Guard | Case::GuardAbsent | Case::RoleEarly) {
            w.change("replace", SLOT);
        }
        if matches!(case, Case::SeatMatching | Case::SeatAbsent) {
            w.change("remove", SLOT);
        }
        drop(held);
        finish(child)
    } else {
        cmd.output().expect("SETUP: real helper completion")
    };
    eprintln!(
        "B5 BEGIN tag={tag} action={action} native={:?}\nstdout={}\nstderr={}\ns={}\nother={}\nB5 END tag={tag}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        w.events(),
        read(w.rig.dir("other").join("events.jsonl"))
    );
    if physical {
        assert!(
            w.rig.scratch.join("barrier/physical-failed").is_file(),
            "SETUP: physical fault missed"
        );
    }
    if stage.is_some() {
        assert!(
            w.out.with_extension("changed").is_file(),
            "SETUP: after-input mutation missed"
        );
        assert!(
            w.rig.scratch.join("barrier/revocation-seen").is_file(),
            "SETUP: after-input acknowledgement missed"
        );
        w.acknowledged();
    }
    if matches!(case, Case::RawAbsent) {
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        wait("SETUP: raw target consumed body", || {
            read(&w.out).contains(BODY)
        });
        assert!(rows(&w, "s", "refused").is_empty());
        assert_eq!(rows(&w, "s", action).len(), 1, "{}", w.events());
        return;
    }
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("REFUSED"), "{stderr}");
    match case {
        Case::RoleEarly | Case::RoleMatching | Case::RoleAbsent => {
            assert!(
                stderr.contains(ROLE),
                "early Role policy diagnostic changed: {stderr}"
            );
            assert!(
                !stderr.contains("was replaced"),
                "seat comparison bypassed Role policy: {stderr}"
            );
        }
        Case::RoleAllowed => {
            assert!(
                stderr.contains("orchestrator role") && stderr.contains("was replaced"),
                "{stderr}"
            );
            assert!(
                !stderr.contains(ROLE),
                "allowed caller was denied: {stderr}"
            );
        }
        Case::Malformed
        | Case::Conflict
        | Case::RawExpected
        | Case::RawPhysicalExpected
        | Case::RawPhysicalAbsent => {
            assert!(
                stderr.contains("could not be proven"),
                "wrong refusal class: {stderr}"
            );
        }
        Case::Stale | Case::Slot | Case::Guard | Case::GuardAbsent => {
            assert!(
                stderr.contains("was replaced"),
                "wrong refusal class: {stderr}"
            );
        }
        Case::SeatMatching | Case::SeatAbsent => {
            assert!(
                stderr.contains("is retired"),
                "wrong refusal class: {stderr}"
            );
        }
        _ => {}
    }
    if stage.is_none() {
        w.untouched(&before);
    }
    audit(&w, if foreign { "other" } else { "s" });
    let ordinary_attempt = matches!(
        case,
        Case::GuardAbsent
            | Case::RoleMatching
            | Case::RoleAbsent
            | Case::SeatMatching
            | Case::SeatAbsent
    );
    attempt(&w, action, usize::from(ordinary_attempt), foreign);
    if let Some(event) = stage {
        assert_eq!(
            read(w.out.with_extension("enters")).lines().count(),
            usize::from(event == "enter")
        );
        assert!(
            stderr.contains(if event == "paste" {
                "no Enter was sent"
            } else {
                "submission unconfirmed"
            }),
            "{stderr}"
        );
        assert!(
            fs::read_dir(w.dir().join("messages"))
                .expect("SETUP: stored bodies")
                .any(|entry| read(entry.expect("SETUP: body entry").path()).contains(BODY)),
            "recoverable body missing"
        );
    }
}

macro_rules! accounting {
    ($name:ident, $tag:literal, $action:literal, $case:ident) => {
        #[test]
        fn $name() {
            run($tag, $action, Case::$case);
        }
    };
}
accounting!(b5_stale_done, "r218-b5-stale-done", "done-challenge", Stale);
accounting!(b5_stale_wait, "r218-b5-stale-wait", "wait-challenge", Stale);
accounting!(b5_stale_moved, "r218-b5-stale-moved", "target-moved", Stale);
accounting!(
    b5_malformed_done,
    "r218-b5-malformed-done",
    "done-challenge",
    Malformed
);
accounting!(
    b5_malformed_wait,
    "r218-b5-malformed-wait",
    "wait-challenge",
    Malformed
);
accounting!(
    b5_malformed_moved,
    "r218-b5-malformed-moved",
    "target-moved",
    Malformed
);
accounting!(
    b5_conflict_done,
    "r218-b5-conflict-done",
    "done-challenge",
    Conflict
);
accounting!(
    b5_conflict_wait,
    "r218-b5-conflict-wait",
    "wait-challenge",
    Conflict
);
accounting!(
    b5_conflict_moved,
    "r218-b5-conflict-moved",
    "target-moved",
    Conflict
);
accounting!(b5_slot_done, "r218-b5-slot-done", "done-challenge", Slot);
accounting!(b5_slot_wait, "r218-b5-slot-wait", "wait-challenge", Slot);
accounting!(b5_slot_moved, "r218-b5-slot-moved", "target-moved", Slot);
accounting!(
    b5_role_allowed_done,
    "r218-b5-role-allowed-done",
    "done-challenge",
    RoleAllowed
);
accounting!(
    b5_role_allowed_wait,
    "r218-b5-role-allowed-wait",
    "wait-challenge",
    RoleAllowed
);
accounting!(
    b5_role_allowed_moved,
    "r218-b5-role-allowed-moved",
    "target-moved",
    RoleAllowed
);
accounting!(
    b5_role_early_done,
    "r218-b5-role-early-done",
    "done-challenge",
    RoleEarly
);
accounting!(
    b5_role_early_wait,
    "r218-b5-role-early-wait",
    "wait-challenge",
    RoleEarly
);
accounting!(
    b5_role_early_moved,
    "r218-b5-role-early-moved",
    "target-moved",
    RoleEarly
);
accounting!(
    b5_role_matching_done,
    "r218-b5-role-matching-done",
    "done-challenge",
    RoleMatching
);
accounting!(
    b5_role_matching_wait,
    "r218-b5-role-matching-wait",
    "wait-challenge",
    RoleMatching
);
accounting!(
    b5_role_matching_moved,
    "r218-b5-role-matching-moved",
    "target-moved",
    RoleMatching
);
accounting!(
    b5_role_absent_done,
    "r218-b5-role-absent-done",
    "done-challenge",
    RoleAbsent
);
accounting!(
    b5_role_absent_wait,
    "r218-b5-role-absent-wait",
    "wait-challenge",
    RoleAbsent
);
accounting!(
    b5_role_absent_moved,
    "r218-b5-role-absent-moved",
    "target-moved",
    RoleAbsent
);
accounting!(b5_guard_done, "r218-b5-guard-done", "done-challenge", Guard);
accounting!(b5_guard_wait, "r218-b5-guard-wait", "wait-challenge", Guard);
accounting!(b5_guard_moved, "r218-b5-guard-moved", "target-moved", Guard);
accounting!(
    b5_seat_matching_done,
    "r218-b5-seat-matching-done",
    "done-challenge",
    SeatMatching
);
accounting!(
    b5_seat_matching_wait,
    "r218-b5-seat-matching-wait",
    "wait-challenge",
    SeatMatching
);
accounting!(
    b5_seat_matching_moved,
    "r218-b5-seat-matching-moved",
    "target-moved",
    SeatMatching
);
accounting!(
    b5_seat_absent_done,
    "r218-b5-seat-absent-done",
    "done-challenge",
    SeatAbsent
);
accounting!(
    b5_seat_absent_wait,
    "r218-b5-seat-absent-wait",
    "wait-challenge",
    SeatAbsent
);
accounting!(
    b5_seat_absent_moved,
    "r218-b5-seat-absent-moved",
    "target-moved",
    SeatAbsent
);
accounting!(
    b5_raw_expected_done,
    "r218-b5-raw-expected-done",
    "done-challenge",
    RawExpected
);
accounting!(
    b5_raw_expected_wait,
    "r218-b5-raw-expected-wait",
    "wait-challenge",
    RawExpected
);
accounting!(
    b5_raw_expected_moved,
    "r218-b5-raw-expected-moved",
    "target-moved",
    RawExpected
);
accounting!(
    b5_raw_absent_done,
    "r218-b5-raw-absent-done",
    "done-challenge",
    RawAbsent
);
accounting!(
    b5_raw_absent_wait,
    "r218-b5-raw-absent-wait",
    "wait-challenge",
    RawAbsent
);
accounting!(
    b5_raw_absent_moved,
    "r218-b5-raw-absent-moved",
    "target-moved",
    RawAbsent
);
accounting!(
    b5_raw_physical_expected_done,
    "r218-b5-raw-physical-expected-done",
    "done-challenge",
    RawPhysicalExpected
);
accounting!(
    b5_raw_physical_expected_wait,
    "r218-b5-raw-physical-expected-wait",
    "wait-challenge",
    RawPhysicalExpected
);
accounting!(
    b5_raw_physical_expected_moved,
    "r218-b5-raw-physical-expected-moved",
    "target-moved",
    RawPhysicalExpected
);
accounting!(
    b5_raw_physical_absent_done,
    "r218-b5-raw-physical-absent-done",
    "done-challenge",
    RawPhysicalAbsent
);
accounting!(
    b5_raw_physical_absent_wait,
    "r218-b5-raw-physical-absent-wait",
    "wait-challenge",
    RawPhysicalAbsent
);
accounting!(
    b5_raw_physical_absent_moved,
    "r218-b5-raw-physical-absent-moved",
    "target-moved",
    RawPhysicalAbsent
);
accounting!(
    b5_pasted_expected_done,
    "r218-b5-pasted-expected-done",
    "done-challenge",
    PastedExpected
);
accounting!(
    b5_pasted_expected_wait,
    "r218-b5-pasted-expected-wait",
    "wait-challenge",
    PastedExpected
);
accounting!(
    b5_pasted_expected_moved,
    "r218-b5-pasted-expected-moved",
    "target-moved",
    PastedExpected
);
accounting!(
    b5_pasted_absent_done,
    "r218-b5-pasted-absent-done",
    "done-challenge",
    PastedAbsent
);
accounting!(
    b5_pasted_absent_wait,
    "r218-b5-pasted-absent-wait",
    "wait-challenge",
    PastedAbsent
);
accounting!(
    b5_pasted_absent_moved,
    "r218-b5-pasted-absent-moved",
    "target-moved",
    PastedAbsent
);
accounting!(
    b5_entered_expected_done,
    "r218-b5-entered-expected-done",
    "done-challenge",
    EnteredExpected
);
accounting!(
    b5_entered_expected_wait,
    "r218-b5-entered-expected-wait",
    "wait-challenge",
    EnteredExpected
);
accounting!(
    b5_entered_expected_moved,
    "r218-b5-entered-expected-moved",
    "target-moved",
    EnteredExpected
);
accounting!(
    b5_entered_absent_done,
    "r218-b5-entered-absent-done",
    "done-challenge",
    EnteredAbsent
);
accounting!(
    b5_entered_absent_wait,
    "r218-b5-entered-absent-wait",
    "wait-challenge",
    EnteredAbsent
);
accounting!(
    b5_entered_absent_moved,
    "r218-b5-entered-absent-moved",
    "target-moved",
    EnteredAbsent
);
accounting!(
    b5_guard_absent_done,
    "r218-b5-guard-absent-done",
    "done-challenge",
    GuardAbsent
);
accounting!(
    b5_guard_absent_wait,
    "r218-b5-guard-absent-wait",
    "wait-challenge",
    GuardAbsent
);
accounting!(
    b5_guard_absent_moved,
    "r218-b5-guard-absent-moved",
    "target-moved",
    GuardAbsent
);
