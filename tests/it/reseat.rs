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
    // The seat's recorded first message, which `run::clear_slot` DELETES. The
    // pack must therefore be built before the cleanup, and this is the pin on
    // that ordering: the text is in the seed and the file is gone afterwards.
    let prompt = rig.dir.join("launch.spawned.0.prompt");
    assert!(
        std::fs::write(&prompt, "the brief this seat was opened with").is_ok(),
        "a recorded first message"
    );
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
    // EXACTLY `pending`: opencode's id can only be captured after its tool
    // starts, so a reseat onto it must not invent one — `!= OLD_ID` would pass
    // for a stale uuid just as happily.
    assert_eq!(rig.meta_row("harness_session.spawned.0"), "pending");
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
    let seed_path = rig.dir.join("seed.scout.md");
    let seed = std::fs::read_to_string(&seed_path).unwrap_or_default();
    assert!(seed.starts_with("⟦ae:ctx⟧\n"), "{seed:?}");
    assert_eq!(mode_of(&seed_path), 0o600, "the seed is published 0600");
    // THE ORDER, pinned: the pack carries the first message, and the file it
    // was read from is gone — so the build happened before the cleanup.
    assert!(
        seed.contains("## 9. first message")
            && seed.contains("the brief this seat was opened with"),
        "the seed carries the recorded first message: {seed}"
    );
    assert!(
        !prompt.exists(),
        "the slot's launch files are cleared: {} survived",
        prompt.display()
    );
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

    // CRASH WINDOW 2, in the shape a human meets it: the meta names the new
    // profile and the pane is back at its shell. `relaunch` is what the
    // refusal on that path tells them to run, so it has to finish the move
    // rather than resurrect the tool the seat left.
    rig.kill_tools(&pane);
    let (code, out, err) = rig.run("_relaunch", &["scout"]);
    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(out.contains("relaunched scout"), "{out}");
    assert!(
        rig.tool_pid(&pane, "opencode").is_some(),
        "back on the NEW profile's tool, not the one it was moved off"
    );
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
}

#[test]
fn a_running_seat_is_stopped_in_place_and_moved() {
    // THE FULL ROUND. The seat's tool is running and its frame is provably
    // idle, so `reseat` stops it where it stands and moves the seat on. What
    // the stop must not cost: the pane, its ae stamps, its directory.
    let rig = Rig::new("fullround");
    rig.seat_rows("spawned.0", "scout", "claude", "claude");
    record_history(&rig, "spawned.0");
    let pane = rig.new_pane("spawned.0", "scout");
    rig.start(&pane, "spawned.0", "claude");
    let work_dir = rig.meta_row("work_dir");

    let (code, out, err) = rig.run_top(
        &rig.main_pane.clone(),
        &["reseat", &rig.session, "scout", "--using", "fake-opencode"],
    );

    assert_eq!(
        code,
        Some(0),
        "out={out} err={err}\nframe was:\n{}",
        rig.capture(&pane)
    );
    assert!(out.contains("reseated scout"), "{out}");
    assert!(
        rig.tool_pid(&pane, "claude").is_none(),
        "the old tool is gone"
    );
    assert!(
        rig.tool_pid(&pane, "opencode").is_some(),
        "the successor holds the SAME pane"
    );
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    // The predecessor is handed on TAGGED with the tool that owns its store,
    // which is the one that was just stopped.
    assert_eq!(
        rig.meta_row("harness_session_prior.spawned.0"),
        format!("claude:{OLD_ID}")
    );
    // The pane is the same pane, and it is still the seat's: a stop that lost
    // the stamps would leave a pane ae can no longer place.
    assert_eq!(rig.pane_fact(&pane, "#{pane_id}"), pane);
    assert_eq!(rig.pane_fact(&pane, "#{@ae_agent}"), "scout");
    assert_eq!(rig.pane_fact(&pane, "#{@ae_slot}"), "spawned.0");
    // Canonicalised on both sides: macOS answers `/private/tmp` where the
    // record says `/tmp`, and the claim is that they are the SAME directory.
    let canonical = |path: &str| {
        std::fs::canonicalize(path)
            .map_or_else(|_| path.to_owned(), |path| path.display().to_string())
    };
    assert_eq!(
        canonical(&rig.pane_fact(&pane, "#{pane_current_path}")),
        canonical(&work_dir)
    );
    // TWO records: ae stopped the tool, then moved the seat. The stop names
    // the binary it stopped, because the meta keeps only the current one.
    let events = rig.events();
    assert!(events.contains("stopped claude in place"), "{events}");
    assert!(events.contains("reseated scout"), "{events}");
}

#[test]
fn a_pane_that_does_not_come_back_to_a_shell_is_refused_in_the_stops_own_words() {
    // THE BOUNDED WAIT, PINNED. `respawn-pane` returns as soon as tmux has
    // STARTED the pane's command, not when that command is a shell a human
    // could type into — and the command it starts is the pane's OWN, which
    // this pane arranges to be a sleep the second time it runs. So the pane
    // never comes back inside the bound, and the stop must wait the bound out
    // and refuse IN ITS OWN WORDS, with the meta untouched. A stop that did
    // not wait would hand that pane straight to the dead proof and refuse in
    // the dead proof's words instead.
    let rig = Rig::new("slowshell");
    rig.seat_rows("spawned.0", "scout", "claude", "claude");
    record_history(&rig, "spawned.0");
    let script = rig.scratch.join("slowshell");
    assert!(
        std::fs::write(
            &script,
            "[ -f \"$0.seen\" ] && exec sleep 60\n: > \"$0.seen\"\nexec /bin/sh\n",
        )
        .is_ok(),
        "a start command that is a shell once and a sleep every time after"
    );
    let pane = rig.new_pane_running(
        "spawned.0",
        "scout",
        &format!("/bin/sh {}", script.display()),
    );
    rig.start(&pane, "spawned.0", "claude");

    let (code, out, err) = rig.run_top(
        &rig.main_pane.clone(),
        &["reseat", &rig.session, "scout", "--using", "fake-opencode"],
    );

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(
        err.contains("is not back at an idle shell after 10s"),
        "the stop's own timeout, not the dead proof's refusal: {err}"
    );
    // Nothing moved: the seat still records the profile it had, so `relaunch`
    // brings it back and a second reseat is free to try again.
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude");
}

#[test]
fn a_busy_seat_is_refused_and_the_flag_does_not_lift_it() {
    // NEVER A SILENT MID-TURN KILL. The frame says a turn is running, and no
    // flag makes that a reason to stop: `--stop-unknown` lifts an UNREADABLE
    // frame, never a read one.
    let rig = Rig::new("busymove");
    rig.seat_rows("worker.1", "w1", "claude", "claude");
    let pane = rig.new_pane("worker.1", "w1");
    rig.start(&pane, "worker.1", "claude");
    rig.mark_busy(&pane);

    for tail in [
        &["w1", "--using", "fake-grok"][..],
        &["w1", "--using", "fake-grok", "--stop-unknown"][..],
    ] {
        let mut args = vec!["reseat", &rig.session];
        args.extend(tail);
        let (code, out, err) = rig.run_top(&rig.main_pane.clone(), &args);
        assert_eq!(code, Some(1), "{tail:?} out={out} err={err}");
        assert!(
            err.contains("is BUSY") && err.contains(&pane) && err.contains("interrupt w1"),
            "{tail:?}: the refusal names the state, the pane and the next step: {err}"
        );
        assert!(
            rig.tool_pid(&pane, "claude").is_some(),
            "{tail:?}: the busy tool is still running"
        );
        assert_eq!(rig.meta_row("profile.worker.1"), "fake-claude");
        assert!(!rig.dir.join("seed.w1.md").exists(), "{tail:?}: no seed");
        // NOTHING WAS WRITTEN, and the audit is where that would show first.
        assert!(
            !rig.events().contains("stopped"),
            "{tail:?}: a refusal recorded a stop: {}",
            rig.events()
        );
    }
}

#[test]
fn a_frame_ae_cannot_read_is_refused_until_the_flag_says_otherwise() {
    // An UNMODELLED harness has no grammar ae can read a running turn from.
    // That is not permission to kill it — it is a refusal that names the flag.
    let rig = Rig::new("blindmove");
    let pane = rig.seat("worker.1", "w1", "grok");

    let (code, out, err) = rig.run_top(
        &rig.main_pane.clone(),
        &["reseat", &rig.session, "w1", "--using", "fake-opencode"],
    );

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(
        err.contains("cannot read") && err.contains(&pane) && err.contains("--stop-unknown"),
        "the refusal names the gap, the pane and the way past it: {err}"
    );
    assert_eq!(rig.meta_row("profile.worker.1"), "fake-grok");
    assert!(rig.tool_pid(&pane, "grok").is_some(), "still running");
    assert!(
        !rig.events().contains("stopped"),
        "a refusal recorded a stop: {}",
        rig.events()
    );
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

    // With the flag, the same seat stops and moves — same pane, new tool.
    let (code, out, err) = rig.run_top(
        &rig.main_pane.clone(),
        &[
            "reseat",
            &rig.session,
            "w1",
            "--using",
            "fake-opencode",
            "--stop-unknown",
        ],
    );
    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(rig.tool_pid(&pane, "opencode").is_some(), "moved in place");
    assert_eq!(rig.meta_row("profile.worker.1"), "fake-opencode");
    assert!(rig.events().contains("stopped grok in place"), "audited");
}

#[test]
fn the_argv_and_the_roster_are_answered_before_any_pane_is_read() {
    // Each of these is a DURABLE fact, so each must refuse on its own terms.
    // The seat is deliberately LIVE: a ladder that read the pane first would
    // answer every one of them with "is running" instead.
    let rig = Rig::new("ladder");
    let pane = rig.seat("worker.1", "w1", "opencode");

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
    // THE ORDER PROOF, now that this verb can end a tool: a typo must never
    // reach the stop, so the seat's tool is still the one that was running.
    assert!(rig.tool_pid(&pane, "opencode").is_some(), "never stopped");
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

/// The published mode of one file, for the 0600 assertions.
fn mode_of(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .map(|meta| meta.permissions().mode() & 0o777)
        .unwrap_or_default()
}

#[test]
fn a_move_onto_a_tool_that_takes_its_id_at_launch_gets_a_fresh_one_and_a_lost_seed_fails_loudly() {
    // TWO rules on one move, because the fixture gives both for free.
    //
    // The other half of the conversation rule: opencode's id is captured after
    // the fact and reads `pending`, grok's is passed on the launch line and
    // must be a FRESH uuid — never the predecessor's, and never `pending`.
    //
    // And the seat is up while its SEED never landed. This fake draws no input
    // box, so the delivery cannot be proven and ae will not claim it: the move
    // is real and recorded, the exit is 1, and the refusal hands over the
    // command that re-sends the kept seed by hand.
    let rig = Rig::new("freshid");
    rig.seat_rows("spawned.0", "scout", "opencode", "opencode");
    record_history(&rig, "spawned.0");
    let pane = rig.new_pane("spawned.0", "scout");
    rig.start(&pane, "spawned.0", "opencode");
    rig.kill_tools(&pane);

    let (code, out, err) = rig.run_top(
        &rig.main_pane.clone(),
        &["reseat", &rig.session, "scout", "--using", "fake-grok"],
    );

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(
        err.contains("seed undelivered") && err.contains("a turn never landed"),
        "the seat is up and the turn is not claimed: {err}"
    );
    assert!(
        err.contains("send scout") && err.contains("seed.scout.md"),
        "the hand-send command names the kept seed: {err}"
    );
    assert!(
        rig.tool_pid(&pane, "grok").is_some(),
        "the seat IS up — only the turn failed"
    );
    let id = rig.meta_row("harness_session.spawned.0");
    assert_ne!(id, OLD_ID, "not the predecessor's");
    assert_ne!(id, "pending", "grok takes one at launch");
    // The canonical shape, spelled here rather than reached for through a
    // private module: 8-4-4-4-12 lowercase hex with the dashes where they go.
    let groups: Vec<usize> = id.split('-').map(str::len).collect();
    assert_eq!(groups, vec![8, 4, 4, 4, 12], "a canonical uuid: {id:?}");
    assert!(
        id.chars()
            .all(|ch| ch == '-' || ch.is_ascii_digit() || ch.is_ascii_lowercase()),
        "lowercase hex only: {id:?}"
    );
    // And the predecessor is tagged with the tool that owned it, which is the
    // OLD one here — the tag follows the conversation, not the arrival.
    assert_eq!(
        rig.meta_row("harness_session_prior.spawned.0"),
        format!("opencode:{OLD_ID}")
    );
    // The launch line carried it: grok is the class that takes `--session-id`.
    assert!(
        rig.launched().contains(&format!("--session-id {id}")),
        "the new conversation reached the launch line: {}",
        rig.launched()
    );
}

#[test]
fn a_reseat_records_who_asked_and_which_way_the_seat_moved() {
    // The audit trail's own pin. The meta keeps only the CURRENT profile, so
    // the event is the one place that pairs the predecessor's conversation
    // with the profile it ran under — and the actor must be the SEAT that ran
    // the command, never a blanket `human`.
    let rig = Rig::new("record");
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

    let events = rig.events();
    // THE MOVE's record, not the stop's: this seat was running, so the stop
    // wrote a `reseat` record of its own first, and the conversation is a fact
    // about the move. A stop names no conversation at all.
    let line = events
        .lines()
        .find(|line| line.contains("\"action\":\"reseat\"") && line.contains("reseated scout"))
        .unwrap_or_default();
    assert!(
        line.contains("\"actor\":\"lead\""),
        "the calling SEAT, not `human`: {line}"
    );
    assert!(
        line.contains("from fake-grok to fake-opencode"),
        "the move, both ends: {line}"
    );
    assert!(
        line.contains(&format!("prior {OLD_ID}")),
        "the conversation the seat is leaving: {line}"
    );
    assert!(
        line.contains("\"target\":\"scout\"") && line.contains("\"target_slot\":\"spawned.0\""),
        "the seat it names: {line}"
    );
}

#[test]
fn a_seat_cannot_reseat_itself() {
    // The tool running the command is the one that would be replaced under it.
    // The dead proof would refuse a live caller anyway; this states the rule
    // rather than leaving it to an accident of ordering.
    let rig = Rig::new("selfmove");
    let pane = rig.seat("worker.1", "w1", "opencode");

    let (code, out, err) = rig.run_top(
        &pane,
        &["reseat", &rig.session, "w1", "--using", "fake-grok"],
    );

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(
        err.contains("is THIS pane") && err.contains("cannot reseat itself"),
        "the refusal says which pane and why: {err}"
    );
    assert!(
        !err.contains("is running"),
        "refused on its own terms: {err}"
    );
    assert_eq!(rig.meta_row("profile.worker.1"), "fake-opencode");
    // The refusal is decided BEFORE the lock and before any write, so the
    // caller's own tool — the one that would have been ended — is untouched.
    assert!(rig.tool_pid(&pane, "opencode").is_some(), "still running");
    assert!(rig.received().is_empty(), "{:?}", rig.received());
}
