//! #309 ruling 3: a real RETURN keeps the proven human origin exemption.
//! The same tierless destination is refused on the ordinary automatic move route.

use super::*;

const LEAD_SLOT: &str = "worker.0";

fn return_fixture(tag: &str, returning: bool) -> (Rig, String, Timestamp) {
    let rig = Rig::new(tag);
    let config = format!(
        "[profiles]\norigin = \"{}\"\nsubstitute = \"{}\"\n[workspace]\nmain = lead\nlayout = lead-pair\nauto_reseat = all\nauto_reseat_grace_secs = 0\nauto_reseat_lead_floor = 3\n[model_tiers]\ncurrent = claude-opus-5, 4\n[auto_reseat]\nsubstitute = origin\n",
        command(&rig, "home-a").replace("--model claude-opus-5", "--model haiku"),
        command(&rig, "home-b")
    );
    std::fs::write(rig.scratch.join("config"), config).expect("two-profile floor fixture");
    let away = Timestamp::now().epoch() - 60;
    append(&rig, "lead", "spawn", away - 20, "", "spawned scout");
    hop(&rig, "origin", "substitute", "auto-reseat-headroom", away);
    low(&rig, "home-a", 20.0);
    low(&rig, "home-b", 10.0);
    let pane = start(&rig, "substitute");
    let key = if returning {
        open_return(&rig)
    } else {
        let key = Timestamp::from_epoch(Timestamp::now().epoch() - 10);
        append(&rig, "watchdog", "limit", key.epoch(), "", "usage limit");
        append(
            &rig,
            "watchdog",
            ATTEMPT_ACTION,
            key.epoch() + 1,
            &key.to_string(),
            "from substitute to origin (usage limit)",
        );
        rig.mark_limited(&pane);
        key
    };
    // Use the existing moveback fixture's real process and journal as a lead seat.
    std::fs::write(
        rig.dir.join("meta"),
        rig.meta().replace(".spawned.0=", ".worker.0="),
    )
    .expect("remap current seat to colead");
    row(&rig, "layout", "lead-pair");
    // An existing collapsed pair may stay on its proven provider (#305 R7).
    row(&rig, "profile.main", "substitute");
    row(&rig, "agent_bin.main", "claude");
    std::fs::write(
        rig.dir.join("events.jsonl"),
        rig.events().replace("spawned.0", LEAD_SLOT),
    )
    .expect("remap historical seat routing");
    assert!(
        rig.tmux(&["set-option", "-p", "-t", &pane, "@ae_slot", LEAD_SLOT])
            .0
    );
    (rig, pane, key)
}

fn lead_events(rig: &Rig, action: &str) -> Vec<Event> {
    records(rig)
        .into_iter()
        .filter(|event| {
            event.action == action
                && ae::watchdog::event_is_addressed_to(event, &rig.session, LEAD_SLOT, NAME)
        })
        .collect()
}

fn run_lead_leg(rig: &Rig, key: Timestamp) -> Output {
    core(rig)
        .arg("_auto-reseat")
        .arg(&rig.dir)
        .args([LEAD_SLOT, &key.to_string()])
        .output()
        .expect("real automatic lead leg")
}

#[test]
fn leadfloor2_real_return_leg_restores_tierless_proven_origin() {
    let (rig, _pane, key) = return_fixture("lf2returnroute", true);
    let result = run_lead_leg(&rig, key);
    assert!(
        result.status.success(),
        "return: {}\n{}",
        String::from_utf8_lossy(&result.stderr),
        rig.events()
    );
    assert_eq!(
        rig.meta_row("profile.worker.0"),
        "origin",
        "{}",
        rig.events()
    );
    let done = lead_events(&rig, DONE_ACTION);
    assert_eq!(done.len(), 2, "one historical departure, one real return");
    assert_eq!(done[1].reference.as_deref(), Some("substitute"));
    assert!(
        lead_events(&rig, REFUSED_ACTION).is_empty(),
        "{}",
        rig.events()
    );
}

#[test]
fn leadfloor2_real_plain_move_cannot_borrow_the_origin_return_exemption() {
    let (rig, pane, key) = return_fixture("lf2moveroute", false);
    let pid = rig.tool_pid(&pane, "claude").expect("source process");
    let result = run_lead_leg(&rig, key);
    let refused = lead_events(&rig, REFUSED_ACTION);
    assert_eq!(refused.len(), 1, "{result:?}\n{}", rig.events());
    let why = refused[0].summary.as_deref().expect("refusal reason");
    for required in [
        "origin",
        "declared in [auto_reseat] substitute",
        "haiku",
        "no tier for its model",
    ] {
        assert!(why.contains(required), "missing {required}: {why}");
    }
    assert_eq!(rig.meta_row("profile.worker.0"), "substitute");
    assert_eq!(rig.tool_pid(&pane, "claude"), Some(pid));
    assert_eq!(lead_events(&rig, DONE_ACTION).len(), 1, "no new move");
    assert_eq!(lead_events(&rig, "reseat").len(), 1, "no new reseat");
    assert!(lead_events(&rig, "stop").is_empty(), "{}", rig.events());
}
