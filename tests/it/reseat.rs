//! `ae reseat`: moving ONE seat to another tool, against a REAL tmux server.
//!
//! The whole operation runs, on [`super::seat_relaunch::Rig`] — the same
//! fixture the relaunch pins use, because this verb reuses that operation's
//! dead proof and its paste, and a second rig would let the two drift.
//!
//! The seat is made dead the way the field case is (tool gone, pane at its
//! shell), then moved to a DIFFERENT fake, and what the successor is handed is
//! read out of the fake's own receipt.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::fmt::Write as _;

use super::seat_relaunch::Rig;

/// The conversation the predecessor is carrying when it is moved off.
const OLD_ID: &str = "11111111-1111-4111-8111-111111111111";

/// Append rows a launch would have recorded for a seat that has been running:
/// a conversation, a post-exec launch stamp, and a positively observed model.
/// Every one of them belongs to the tool that is LEAVING.
fn record_history(rig: &Rig, slot: &str) {
    let meta = rig.dir.join("meta");
    let mut text = std::fs::read_to_string(&meta).unwrap_or_default();
    let _ = writeln!(
        text,
        "harness_session.{slot}={OLD_ID}\nlaunch_time.{slot}=111\ncapture_floor.{slot}=100\n\
         observed_model.{slot}=Grok 4.6\nobserved_model_pin.{slot}=grok-4-6"
    );
    assert!(std::fs::write(&meta, text).is_ok(), "the seat's history");
}

#[test]
fn a_dead_seat_moves_to_another_tool_in_its_own_pane_and_is_handed_its_seed() {
    let rig = Rig::new("move");
    rig.seat_rows("spawned.0", "scout", "grok", "grok");
    record_history(&rig, "spawned.0");
    let pane = rig.new_pane("spawned.0", "scout");
    rig.start(&pane, "spawned.0", "grok");
    rig.kill_tools(&pane);

    let (code, out, err) = rig.run_top(
        &rig.main_pane.clone(),
        &["reseat", &rig.session, "scout", "--using", "fake-opencode"],
    );

    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(
        out.contains("reseated scout")
            && out.contains(&pane)
            && out.contains("spawned.0")
            && out.contains("to fake-opencode"),
        "the line names the agent, its pane, its slot and where it went: {out}"
    );
    // THE SAME PANE, on the SAME SLOT — a move, not a second seat beside it.
    assert!(
        rig.tool_pid(&pane, "opencode").is_some(),
        "the NEW tool runs beneath the original pane"
    );
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    assert_eq!(rig.meta_row("agent_bin.spawned.0"), "opencode");
    assert_eq!(rig.meta_row("seat.spawned.0"), "scout");
    // The predecessor is kept ADDRESSABLE and tagged with the tool that owns
    // it: the successor's reader looks in a different store entirely.
    assert_eq!(
        rig.meta_row("harness_session_prior.spawned.0"),
        format!("grok:{OLD_ID}")
    );
    assert_ne!(
        rig.meta_row("harness_session.spawned.0"),
        OLD_ID,
        "the current row is the SUCCESSOR's conversation"
    );
    // Everything that described the tool that left is gone, and the stamps
    // that describe a launch are this launch's.
    for gone in ["observed_model.spawned.0=", "observed_model_pin.spawned.0="] {
        assert!(
            !rig.meta().contains(gone),
            "{gone} survived:\n{}",
            rig.meta()
        );
    }
    assert_ne!(rig.meta_row("launch_time.spawned.0"), "111");
    assert_ne!(rig.meta_row("capture_floor.spawned.0"), "100");
    // The seed is KEPT, so a human can re-send it by hand, and it is ae's own
    // setup turn — the marker rule's `ctx`, put on by the one owner.
    let seed = std::fs::read_to_string(rig.dir.join("seed.scout.md")).unwrap_or_default();
    assert!(seed.starts_with("⟦ae:ctx⟧\n"), "{seed:?}");
    assert!(
        seed.contains("## 10. successor instructions"),
        "the seed IS the seat pack: {seed}"
    );
    // What the SUCCESSOR actually received, read off the fake's own receipt.
    let received = rig.received();
    assert!(
        received.contains("⟦ae:ctx⟧") && received.contains("## 10. successor instructions"),
        "the successor was handed the seed: {received}"
    );
    assert!(
        rig.events().contains("\"action\":\"reseat\""),
        "the move is recorded: {}",
        rig.events()
    );
    // The successor came up in the seat's RECORDED working copy: the pasted
    // line carries the `cd`, because `pane_line`'s own directory is the state
    // dir and a shell that wandered would resume the seat somewhere else.
    assert!(
        rig.launched()
            .contains(&format!("cwd={}", rig.scratch.join("work").display())),
        "the successor started in the seat's working copy: {}",
        rig.launched()
    );
}

#[test]
fn a_live_seat_is_refused_and_nothing_about_it_moves() {
    // reseat KILLS NOTHING. A seat whose tool still answers is a refusal, not
    // a tool ae ends to make room for another.
    let rig = Rig::new("livemove");
    let pane = rig.seat("worker.1", "w1", "opencode");

    let (code, out, err) = rig.run_top(
        &rig.main_pane.clone(),
        &["reseat", &rig.session, "w1", "--using", "fake-grok"],
    );

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(
        err.contains("is running") && err.contains(&pane) && err.contains("nothing to reseat"),
        "the refusal names the state, the pane and this verb: {err}"
    );
    assert_eq!(rig.meta_row("profile.worker.1"), "fake-opencode");
    // PANE BYTES ARE NOT EVIDENCE: the fake logs everything it is sent, so an
    // EMPTY receipt is the proof that nothing was typed at the live agent.
    assert!(
        rig.received().is_empty(),
        "the live agent received: {:?}",
        rig.received()
    );
    assert!(
        !rig.dir.join("seed.w1.md").exists(),
        "no seed was published"
    );
}

#[test]
fn the_argv_and_the_roster_are_answered_before_any_pane_is_read() {
    // Each of these is a DURABLE fact, so each must refuse on its own terms.
    // The seat is deliberately LIVE: a ladder that read the pane first would
    // answer every one of them with "is running" instead.
    let rig = Rig::new("ladder");
    rig.seat("worker.1", "w1", "opencode");

    for (tail, want) in [
        (
            ["ghost", "--using", "fake-grok"],
            "has no seat named 'ghost'",
        ),
        (
            ["w1", "--using", "fake-opencode"],
            "already runs profile 'fake-opencode'",
        ),
        (
            ["w1", "--using", "no-such-profile"],
            "is not configured on this machine",
        ),
    ] {
        let mut args = vec!["reseat", &rig.session];
        args.extend(tail);
        let (code, out, err) = rig.run_top(&rig.main_pane.clone(), &args);
        assert_eq!(code, Some(1), "{tail:?} out={out} err={err}");
        assert!(err.contains(want), "{tail:?}: {err}");
        assert!(!err.contains("is running"), "{tail:?}: {err}");
    }
    // The roster refusal lists what IS there, so a typo is fixed in one step.
    let (_, _, err) = rig.run_top(
        &rig.main_pane.clone(),
        &["reseat", &rig.session, "ghost", "--using", "fake-grok"],
    );
    assert!(err.contains("lead") && err.contains("w1"), "{err}");
    assert!(rig.received().is_empty(), "{:?}", rig.received());
    assert_eq!(rig.meta_row("profile.worker.1"), "fake-opencode");
}

#[test]
fn a_seat_of_another_session_is_refused_for_a_caller_ae_stamped() {
    // A stamped pane IS a seat, and rule 8c's boundary holds here as it does
    // for every other agent-to-agent verb. A human's own shell carries no
    // stamp and is not refused — that caller is the point of the verb.
    let rig = Rig::new("cross");
    let other = rig.scratch.join("sessions").join("other");
    assert!(std::fs::create_dir_all(&other).is_ok(), "a second session");
    assert!(
        std::fs::write(
            other.join("meta"),
            format!(
                "session=other\nmeta_version=2\nmode=local\nwork_dir={}\n\
                 seat.main=peer\nprofile.main=fake-grok\nagent_bin.main=grok\n\
                 tmux_server_kind=socket\ntmux_server={}\n",
                rig.scratch.display(),
                rig.scratch.join("no-server.sock").display(),
            ),
        )
        .is_ok(),
        "the second session's meta"
    );

    let (code, out, err) = rig.run_top(
        &rig.main_pane.clone(),
        &["reseat", "other", "peer", "--using", "fake-codex"],
    );

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(
        err.contains("caller's own session only") && err.contains(&rig.session),
        "the refusal names the boundary and where the caller is: {err}"
    );
    assert!(
        std::fs::read_to_string(other.join("meta"))
            .unwrap_or_default()
            .contains("profile.main=fake-grok"),
        "the other session's meta is untouched"
    );
}

#[test]
fn a_pane_stamped_for_another_slot_is_refused_before_the_lock() {
    // The roster and the pane's own stamp are two records, and they disagree
    // only when a stamp is stale or a meta was hand-edited. The move would
    // then be PUBLISHED for one seat and PASTED into another's pane, so it is
    // refused with both readings named.
    let rig = Rig::new("slotskew");
    rig.seat_rows("spawned.0", "scout", "grok", "grok");
    let pane = rig.new_pane("spawned.4", "scout");
    rig.start(&pane, "spawned.0", "grok");
    rig.kill_tools(&pane);

    let (code, out, err) = rig.run_top(
        &rig.main_pane.clone(),
        &["reseat", &rig.session, "scout", "--using", "fake-opencode"],
    );

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(
        err.contains("spawned.4") && err.contains("spawned.0") && err.contains(&pane),
        "the refusal names both readings and the pane: {err}"
    );
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-grok");
    assert!(
        !rig.dir.join("seed.scout.md").exists(),
        "no seed was published"
    );
}
