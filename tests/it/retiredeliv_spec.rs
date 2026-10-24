//! Frozen #218 slice 2a acceptance: a historical reply never crosses a retired seat — neither to
//! the seat that replaced a retired asker nor from the seat that replaced a retired target. Real
//! `ask`, `retire` and `reply` helpers, private tmux, fixture-only agents.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixture setup reads scratch state and fails on setup errors"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::time::{Duration, Instant};

use super::renametell_routing_spec::{A, B, CEILING, Rig};

// The lowest free spawned slot, which a successor reuses.
const SLOT: &str = "spawned.0";
// Every spawned seat's receipt lives under this tag, whatever its session is called now.
const TAG: &str = "spawned";

/// The one refusal a reply prints when the request's `end` (`asker` or `target`) retired.
fn refused(id: &str, end: &str) -> String {
    format!("ae: reply {id} REFUSED — the request's {end} retired; NOTHING was typed.")
}

/// A spawned seat in its own window of `session`: the fake composer, the pane stamps and the
/// roster rows `run_retire` proves. Its receipt is `<agent>-<launch>` under [`TAG`], so a
/// same-name successor keeps its own.
fn seat(rig: &Rig, session: &str, agent: &str, launch: &str) -> String {
    seat_at(rig, session, SLOT, agent, launch)
}

/// [`seat`] at another spawned `slot`.
fn seat_at(rig: &Rig, session: &str, slot: &str, agent: &str, launch: &str) -> String {
    let command = composer_command(rig, &format!("{agent}-{launch}"));
    let target = format!("{session}:");
    let pane = rig
        .tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &target,
            "-n",
            agent,
            &command,
        ])
        .trim()
        .to_owned();
    rig.tmux(&["set-option", "-p", "-t", &pane, "@ae_slot", slot]);
    rig.tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", agent]);
    let meta = rig.dir(session).join("meta");
    let text = format!(
        "{}seat.{slot}={agent}\nprofile.{slot}=fake\nagent_bin.{slot}=claude\nharness_session.{slot}=pending\nlaunch_id.{slot}={launch}\npane.{slot}={pane}\n",
        fs::read_to_string(&meta).expect("session meta")
    );
    fs::write(&meta, text).expect("seat rows");
    composer(rig, &pane);
    pane
}

/// The fake composer's command line, writing its receipt `label` under [`TAG`].
fn composer_command(rig: &Rig, label: &str) -> String {
    let receipt = rig.receipt(TAG, label);
    let claude = rig.scratch.join("claude");
    format!("exec perl {} {}", claude.display(), receipt.display())
}

/// What a reseat does to the pane: the same pane and stamps, a new tool process (receipt `label`).
fn respawn(rig: &Rig, pane: &str, label: &str) {
    rig.tmux(&[
        "respawn-pane",
        "-k",
        "-t",
        pane,
        &composer_command(rig, label),
    ]);
    composer(rig, pane);
}

/// Wait until the fake composer in `pane` has drawn.
fn composer(rig: &Rig, pane: &str) {
    let deadline = Instant::now() + CEILING;
    while !rig
        .tmux(&["capture-pane", "-p", "-t", pane])
        .contains("fake-model")
    {
        assert!(Instant::now() < deadline, "fixture composer did not start");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// A real tracked `ask` from `pane` to `target`; the printed request id.
fn ask(rig: &Rig, session: &str, pane: &str, target: &str) -> String {
    let mut words = vec!["--cross-session", target, "question"];
    if !target.starts_with('@') {
        words.remove(0);
    }
    let (code, out, err) = rig.helper(session, "ask", &words, Some(pane));
    assert_eq!(code, Some(0), "ask setup: {out}{err}");
    out.trim().to_owned()
}

/// The real `retire` helper, run from the session's main pane.
fn retire(rig: &Rig, session: &str, main: &str, agent: &str) {
    let result = rig.helper(session, "retire", &[agent], Some(main));
    assert_eq!(result.0, Some(0), "retire setup: {result:?}");
}

/// The `reply` events carrying `id` in `session`'s ledger.
fn replies(rig: &Rig, session: &str, id: &str) -> usize {
    fs::read_to_string(rig.dir(session).join("events.jsonl"))
        .expect("session ledger")
        .lines()
        .filter(|line| {
            line.contains(r#""action":"reply""#) && line.contains(&format!(r#""ref":"{id}""#))
        })
        .count()
}

/// The pane's screen as tmux draws it now.
fn screen(rig: &Rig, pane: &str) -> String {
    rig.tmux(&["capture-pane", "-p", "-t", pane])
}

/// Nothing typed: no receipt file `<tag>`/`<label>` at all (not even an empty Enter) and the
/// pane's screen byte-equal to `before`, captured just ahead of the reply.
fn untouched(rig: &Rig, tag: &str, label: &str, pane: &str, before: &str) -> bool {
    !rig.receipt(tag, label).exists() && screen(rig, pane) == before
}

/// Delivered exactly once to the receipt `<tag>`/`<label>`, and recorded once in `session`'s ledger.
fn delivered(
    rig: &Rig,
    session: &str,
    (tag, label): (&str, &str),
    id: &str,
    reply: &(Option<i32>, String, String),
) {
    assert_eq!(reply.0, Some(0), "reply refused live seats: {reply:?}");
    let want = format!("[{id}] answer");
    let got = rig.received(tag, label);
    assert_eq!(
        got.iter().filter(|text| text.contains(&want)).count(),
        1,
        "the asker {label} did not receive the reply once: {got:?}"
    );
    assert_eq!(
        replies(rig, session, id),
        1,
        "reply event not recorded once"
    );
}

/// Exit 1 with the one `end`-retired refusal, nothing typed into any of `seats`
/// (`(tag, label, pane, screen before)`), no reply event in `ledgers`.
fn refused_retired(
    rig: &Rig,
    reply: &(Option<i32>, String, String),
    (id, end): (&str, &str),
    seats: &[(&str, &str, &str, &str)],
    ledgers: &[&str],
) {
    assert_eq!(
        reply.0,
        Some(1),
        "reply across a retired {end} was not refused: {reply:?}"
    );
    assert!(
        reply.2.lines().any(|line| line == refused(id, end)),
        "missing the {end}-retired refusal line: {reply:?}"
    );
    for (tag, label, pane, before) in seats {
        assert!(
            untouched(rig, tag, label, pane, before),
            "the historical reply reached {label}: {:?}",
            rig.received(tag, label)
        );
    }
    for ledger in ledgers {
        assert_eq!(
            replies(rig, ledger, id),
            0,
            "a reply-success record landed in {ledger}"
        );
    }
}

#[test]
fn a_reply_to_a_retired_asker_never_reaches_a_differently_named_successor() {
    let rig = Rig::new("rdnew");
    let panes = rig.create("s", A);
    let old = seat(&rig, "s", "old-worker", "tok-old");
    let id = ask(&rig, "s", &old, "lead");
    retire(&rig, "s", &panes[0], "old-worker");
    let new = seat(&rig, "s", "new-worker", "tok-new");
    let before = screen(&rig, &new);
    let reply = rig.helper("s", "reply", &[&id, "answer"], Some(&panes[0]));
    refused_retired(
        &rig,
        &reply,
        (&id, "asker"),
        &[(TAG, "new-worker-tok-new", &new, &before)],
        &["s"],
    );
}

#[test]
fn a_cross_session_reply_never_reaches_a_same_named_successor_retired_only_in_the_askers_ledger() {
    let rig = Rig::new("rdcross");
    let req = rig.create("req", A);
    let resp = rig.create("resp", B);
    let old = seat(&rig, "req", "worker", "tok-old");
    let id = ask(&rig, "req", &old, "@resp:lead");
    retire(&rig, "req", &req[0], "worker");
    let new = seat(&rig, "req", "worker", "tok-new");
    let theirs = fs::read_to_string(rig.dir("resp").join("events.jsonl")).expect("resp ledger");
    assert!(
        theirs.contains(&id) && !theirs.contains(r#""action":"retire""#),
        "fixture: the retire must live in the asker's ledger only"
    );
    let before = screen(&rig, &new);
    let reply = rig.helper("resp", "reply", &[&id, "answer"], Some(&resp[0]));
    refused_retired(
        &rig,
        &reply,
        (&id, "asker"),
        &[(TAG, "worker-tok-new", &new, &before)],
        &["req", "resp"],
    );
}

#[test]
fn a_retire_after_a_rename_still_refuses_the_reply() {
    let rig = Rig::new("rdrename");
    let panes = rig.create("old", A);
    let old = seat(&rig, "old", "worker", "tok-old");
    let id = ask(&rig, "old", &old, "lead");
    let renamed = rig.run(&["rename", "old", "new"], None);
    assert_eq!(renamed.0, Some(0), "live rename setup: {renamed:?}");
    retire(&rig, "new", &panes[0], "worker");
    let new = seat(&rig, "new", "worker", "tok-new");
    let before = screen(&rig, &new);
    let reply = rig.helper("new", "reply", &[&id, "answer"], Some(&panes[0]));
    refused_retired(
        &rig,
        &reply,
        (&id, "asker"),
        &[(TAG, "worker-tok-new", &new, &before)],
        &["new"],
    );
}

#[test]
fn a_successor_at_a_retired_targets_slot_cannot_answer_its_predecessors_request() {
    let rig = Rig::new("rdtarget");
    let panes = rig.create("s", A);
    seat(&rig, "s", "old-worker", "tok-old");
    let id = ask(&rig, "s", &panes[0], "old-worker");
    retire(&rig, "s", &panes[0], "old-worker");
    let new = seat(&rig, "s", "new-worker", "tok-new");
    let before = screen(&rig, &panes[0]);
    let reply = rig.helper("s", "reply", &[&id, "answer"], Some(&new));
    refused_retired(
        &rig,
        &reply,
        (&id, "target"),
        &[(A, "lead", &panes[0], &before)],
        &["s"],
    );
}

#[test]
fn an_unreadable_asker_ledger_refuses_as_unproven_never_as_retired() {
    let rig = Rig::new("rdunread");
    rig.create("req", A);
    let resp = rig.create("resp", B);
    let old = seat(&rig, "req", "worker", "tok-old");
    let id = ask(&rig, "req", &old, "@resp:lead");
    let before = screen(&rig, &old);
    let ledger = rig.dir("req").join("events.jsonl");
    fs::set_permissions(&ledger, fs::Permissions::from_mode(0o000)).expect("hide asker ledger");
    let reply = rig.helper("resp", "reply", &[&id, "answer"], Some(&resp[0]));
    fs::set_permissions(&ledger, fs::Permissions::from_mode(0o600)).expect("restore asker ledger");
    assert!(
        untouched(&rig, TAG, "worker-tok-old", &old, &before),
        "an unproven asker received the reply: {reply:?}"
    );
    assert_eq!(
        reply.0,
        Some(1),
        "unprovable asker was not refused: {reply:?}"
    );
    let head = format!("ae: reply {id} REFUSED — ");
    assert!(
        reply.2.lines().any(|line| line.starts_with(&head)
            && line.contains("could not be re-proven")
            && line.contains("NOTHING was typed")
            && !line.contains("retired")),
        "missing the unproven refusal line: {reply:?}"
    );
    assert_eq!(
        replies(&rig, "resp", &id),
        0,
        "a reply-success record landed"
    );
}

#[test]
fn an_ordinary_reply_to_a_live_spawned_asker_still_lands() {
    let rig = Rig::new("rdlive");
    let panes = rig.create("s", A);
    let old = seat(&rig, "s", "worker", "tok-old");
    let id = ask(&rig, "s", &old, "lead");
    let reply = rig.helper("s", "reply", &[&id, "answer"], Some(&panes[0]));
    delivered(&rig, "s", (TAG, "worker-tok-old"), &id, &reply);
}

#[test]
fn a_reseated_asker_keeps_receiving_its_reply() {
    const KEPT: &str = "0199c0de-4444-4890-abcd-ef0123456789";
    const FRESH: &str = "0199c0de-5555-4890-abcd-ef0123456789";
    // A kept move resumes the same conversation; a seeded one starts a new conversation and
    // records the old one as its predecessor. Both are the same seat.
    for (arm, conversation, prior) in [
        ("kept", KEPT, String::new()),
        (
            "seeded",
            FRESH,
            format!("harness_session_prior.{SLOT}=claude:{KEPT}\n"),
        ),
    ] {
        let rig = Rig::new(&format!("rdreseat{arm}"));
        let panes = rig.create("s", A);
        let pane = seat(&rig, "s", "worker", "tok-old");
        let meta = rig.dir("s").join("meta");
        let from = format!("harness_session.{SLOT}=pending\n");
        let text = fs::read_to_string(&meta).expect("session meta");
        assert!(text.contains(&from), "fixture: no pending conversation row");
        let text = text.replace(&from, &format!("harness_session.{SLOT}={KEPT}\n"));
        fs::write(&meta, text).expect("conversation row");
        let id = ask(&rig, "s", &pane, "lead");
        // What a reseat leaves: the same pane and stamps, a new tool process, new launch token,
        // profile and conversation rows, and its own `reseat` record naming the slot.
        respawn(&rig, &pane, "worker-tok-new");
        let text = fs::read_to_string(&meta)
            .expect("session meta")
            .replace(
                &format!("launch_id.{SLOT}=tok-old"),
                &format!("launch_id.{SLOT}=tok-new"),
            )
            .replace(
                &format!("profile.{SLOT}=fake\n"),
                &format!("profile.{SLOT}=fake-other\n"),
            )
            .replace(
                &format!("harness_session.{SLOT}={KEPT}\n"),
                &format!("harness_session.{SLOT}={conversation}\n{prior}"),
            );
        assert!(
            text.contains(&format!("launch_id.{SLOT}=tok-new\n"))
                && text.contains(&format!("profile.{SLOT}=fake-other\n"))
                && text.contains(&format!("harness_session.{SLOT}={conversation}\n{prior}")),
            "fixture: the reseat rows did not land"
        );
        fs::write(&meta, text).expect("reseated rows");
        rig.append("s", &format!(r#"{{"ts":"2026-10-24T10:01:00Z","actor":"lead","action":"reseat","target":"worker","target_slot":"{SLOT}","summary":"stopped claude in place (pane {pane}); fake -> fake-other, {arm}"}}"#));
        let reply = rig.helper("s", "reply", &[&id, "answer"], Some(&panes[0]));
        delivered(&rig, "s", (TAG, "worker-tok-new"), &id, &reply);
    }
}

#[test]
fn a_renamed_session_still_delivers_to_its_spawned_asker() {
    let rig = Rig::new("rdmoved");
    let panes = rig.create("old", A);
    let old = seat(&rig, "old", "worker", "tok-old");
    let id = ask(&rig, "old", &old, "lead");
    let renamed = rig.run(&["rename", "old", "new"], None);
    assert_eq!(renamed.0, Some(0), "live rename setup: {renamed:?}");
    let reply = rig.helper("new", "reply", &[&id, "answer"], Some(&panes[0]));
    delivered(&rig, "new", (TAG, "worker-tok-old"), &id, &reply);
}

#[test]
fn an_earlier_occupants_retire_never_blocks_the_current_askers_reply() {
    let rig = Rig::new("rdearly");
    let panes = rig.create("s", A);
    seat(&rig, "s", "first", "tok-first");
    retire(&rig, "s", &panes[0], "first");
    let now = seat(&rig, "s", "second", "tok-second");
    let id = ask(&rig, "s", &now, "lead");
    let reply = rig.helper("s", "reply", &[&id, "answer"], Some(&panes[0]));
    delivered(&rig, "s", (TAG, "second-tok-second"), &id, &reply);
}

#[test]
fn an_unrelated_seat_retired_after_the_ask_never_blocks_the_askers_reply() {
    let rig = Rig::new("rdother");
    let panes = rig.create("s", A);
    let asker = seat(&rig, "s", "worker", "tok-old");
    seat_at(&rig, "s", "spawned.1", "other", "tok-other");
    let id = ask(&rig, "s", &asker, "lead");
    retire(&rig, "s", &panes[0], "other");
    let reply = rig.helper("s", "reply", &[&id, "answer"], Some(&panes[0]));
    delivered(&rig, "s", (TAG, "worker-tok-old"), &id, &reply);
}

#[test]
fn a_reseated_target_whose_slot_had_an_earlier_retire_still_answers() {
    let rig = Rig::new("rdtargetok");
    let panes = rig.create("s", A);
    seat(&rig, "s", "first", "tok-first");
    retire(&rig, "s", &panes[0], "first");
    let target = seat(&rig, "s", "worker", "tok-old");
    let id = ask(&rig, "s", &panes[0], "worker");
    // The target's own reseat after the ask: same pane and stamps, new process, launch and profile.
    respawn(&rig, &target, "worker-tok-new");
    let meta = rig.dir("s").join("meta");
    let text = fs::read_to_string(&meta).expect("session meta");
    let text = text
        .replace(
            &format!("launch_id.{SLOT}=tok-old\n"),
            &format!("launch_id.{SLOT}=tok-new\n"),
        )
        .replace(
            &format!("profile.{SLOT}=fake\n"),
            &format!("profile.{SLOT}=fake-other\n"),
        );
    assert!(
        text.contains(&format!("launch_id.{SLOT}=tok-new\n"))
            && text.contains(&format!("profile.{SLOT}=fake-other\n")),
        "fixture: the reseat rows did not land"
    );
    fs::write(&meta, text).expect("reseated rows");
    rig.append("s", &format!(r#"{{"ts":"2026-10-24T10:01:00Z","actor":"lead","action":"reseat","target":"worker","target_slot":"{SLOT}","summary":"stopped claude in place (pane {target}); fake -> fake-other, kept"}}"#));
    let reply = rig.helper("s", "reply", &[&id, "answer"], Some(&target));
    delivered(&rig, "s", (A, "lead"), &id, &reply);
}
