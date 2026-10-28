//! #250 O3 phase 4 R4/R6/R7/R8/R11/R13. Recovery is an ID-keyed read,
//! never a new dispatch or a target-ledger repair. Snapshot cuts start with
//! real writer output; expected outcomes come from the approved crash contract.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns persisted crash snapshots and reads real effects"
)]
use super::cli::byte_tree as tree;
use super::o2guard_p2_spec::ledger;
use super::relay_chat_spec::{Out, Rig, echo_count, events, receipt, rows, words};
use ae::json::Value;
use std::fs;
use std::path::Path;

fn status(r: &Rig, id: &str) -> Out {
    r.run(&["--status", id])
}
fn write_rows(dir: &Path, rows: &[Value]) {
    let text = if rows.is_empty() {
        String::new()
    } else {
        rows.iter()
            .map(Value::render)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    };
    fs::write(dir.join("events.jsonl"), text).expect("frozen crash/corruption snapshot");
}
fn set(row: &mut Value, key: &str, value: Value) {
    let Value::Obj(fields) = row else {
        panic!("object event")
    };
    fields.retain(|(k, _)| k != key);
    fields.push((key.to_owned(), value));
}
fn read_only(r: &Rig, id: &str) -> Out {
    let state = || {
        (
            tree(&r.owner.tool.dir),
            r.keys(),
            r.sent(),
            r.owner.screen(&r.owner.tool.pane),
        )
    };
    let before = state();
    let out = status(r, id);
    assert_eq!(state(), before, "status writes/types nothing in target");
    out
}
fn has(out: &Out, expected: &[&str]) {
    let text = words(out).to_lowercase();
    for word in expected {
        assert!(
            text.contains(&word.to_lowercase()),
            "missing {word}: {text}"
        );
    }
}

#[test]
fn r4_claim_only_crash_gap_is_uncertain_without_invented_pending_or_resend() {
    let r = Rig::new("o3sclaim");
    let id = r.ask("delivered before opening was recorded");
    let emitter: Vec<_> = rows(&r.caller.dir)
        .into_iter()
        .filter(|e| e.get_str("action") == Some("relay-attempt"))
        .collect();
    write_rows(&r.caller.dir, &emitter);
    write_rows(&r.owner.tool.dir, &[]);
    let out = read_only(&r, &id);
    assert_eq!(
        out.0,
        Some(0),
        "successful evidence read even for uncertainty"
    );
    receipt(&out, &id, "uncertain", "absent");
    let count = echo_count(&r, &id);
    read_only(&r, &id);
    assert_eq!(echo_count(&r, &id), count, "one recovery echo");
}

#[test]
fn r4_missing_result_remains_uncertain_even_with_a_reply() {
    let r = Rig::new("o3smissing");
    let id = r.ask("receipt lost after claim");
    r.reply(&id, "reply does not prove submit verification");
    let emitter: Vec<_> = rows(&r.caller.dir)
        .into_iter()
        .filter(|e| ![Some("relay-result"), Some("chat")].contains(&e.get_str("action")))
        .collect();
    write_rows(&r.caller.dir, &emitter);
    let out = read_only(&r, &id);
    assert_eq!(out.0, Some(0));
    receipt(&out, &id, "uncertain", "replied");
    has(
        &out,
        &[
            "uncertain",
            "replied",
            "reply does not prove submit verification",
        ],
    );
}

#[test]
fn r5_not_delivered_receipt_status_is_a_successful_read() {
    let r = Rig::new("o3srefused");
    let id = r.refused(false);
    let out = read_only(&r, &id);
    assert_eq!(out.0, Some(0));
    receipt(&out, &id, "not-delivered", "absent");
}

#[test]
fn r6_r7_local_reply_full_body_and_followups_echo_once_each() {
    let r = Rig::new("o3sreply");
    let id = r.ask("please answer here");
    let answer = "reply α line\n".repeat(300) + "FULL-BODY-END";
    r.reply(&id, &answer);
    let out = read_only(&r, &id);
    assert_eq!(out.0, Some(0));
    receipt(&out, &id, "sent", "replied");
    assert!(
        out.1.contains(&answer),
        "whole answer on stdout, beyond summary cap"
    );
    assert_eq!(echo_count(&r, &id), 2, "receipt + reply");
    let lane = r.chat("orch");
    assert!(lane.contains(&id));
    assert!(
        !lane.contains("FULL-BODY-END"),
        "emitter lane is an explicit excerpt"
    );
    for _ in 0..2 {
        assert_eq!(read_only(&r, &id).0, Some(0));
    }
    assert_eq!(echo_count(&r, &id), 2);
    r.reply(&id, "follow-up body");
    has(&read_only(&r, &id), &["follow-up body"]);
    assert_eq!(
        echo_count(&r, &id),
        3,
        "follow-up has distinct record identity"
    );
}

#[test]
fn r6_local_close_and_orchestrator_restart_use_one_target_lifetime() {
    let r = Rig::new("o3slife");
    let id = r.ask("take this on");
    ae::console::submit::close_owned(&r.owner.tool.dir, &r.owner.home, Some(&id), || Ok(()))
        .expect("owner local close");
    receipt(&read_only(&r, &id), &id, "sent", "cancelled");
    assert!(ae::console::lane::open_asks(&events(&r.owner.tool.dir), &r.owner.home).is_empty());
    // Each helper call is a fresh process. No emitter pending copy exists to
    // close or replay when that process restarts.
    assert!(!events(&r.caller.dir).iter().any(|e| e.action == "ask"));
}

#[test]
fn r8_foreign_origin_torn_target_and_damaged_result_are_visible_gaps() {
    let r = Rig::new("o3sgaps");
    let id = r.ask("strict evidence only");
    receipt(&read_only(&r, &id), &id, "sent", "pending");
    let original = rows(&r.owner.tool.dir);
    let mut pane_only = original.clone();
    assert_eq!(pane_only[0].get_str("action"), Some("ask"));
    let stamp = pane_only[0].get_str("target_session_uuid");
    assert!(stamp.is_some_and(|u| !u.is_empty()));
    set(&mut pane_only[0], "target_session_id", Value::str(""));
    write_rows(&r.owner.tool.dir, &pane_only);
    receipt(&read_only(&r, &id), &id, "sent", "pending");
    for key in [
        "relay_origin",
        "target_slot",
        "target_session_uuid",
        "target_session_id",
    ] {
        let mut altered = original.clone();
        let ask = altered
            .iter_mut()
            .find(|e| e.get_str("action") == Some("ask"))
            .expect("ask");
        set(ask, key, Value::str("0199c0de-aaaa-4890-abcd-ef0123456789"));
        write_rows(&r.owner.tool.dir, &altered);
        let out = read_only(&r, &id);
        assert_eq!(out.0, Some(1), "{key} binding gap: {}", words(&out));
        receipt(&out, &id, "sent", "unknown");
    }
    write_rows(&r.owner.tool.dir, &original);
    let good = ledger(&r.owner.tool.dir);
    fs::write(r.owner.tool.dir.join("events.jsonl"), good + "{torn").expect("torn tail");
    let out = read_only(&r, &id);
    assert_eq!(out.0, Some(1), "strict snapshot refuses torn tail");
    receipt(&out, &id, "sent", "unknown");
    write_rows(&r.owner.tool.dir, &original);
    let mut emitter = rows(&r.caller.dir);
    let result = emitter
        .iter_mut()
        .find(|e| e.get_str("action") == Some("relay-result"))
        .expect("result");
    set(result, "outcome", Value::str("invented-success"));
    write_rows(&r.caller.dir, &emitter);
    assert_ne!(
        read_only(&r, &id).0,
        Some(0),
        "unknown outcome is not Fresh/Sent"
    );
}

#[test]
fn r3_r11_status_selects_id_not_equal_body_and_never_redispatches() {
    let r = Rig::new("o3sids");
    let a = r.ask("identical text");
    let b = r.ask("identical text");
    r.reply(&a, "answer-a-only");
    r.reply(&b, "answer-b-only");
    let out = read_only(&r, &a);
    assert!(out.1.contains("answer-a-only") && !out.1.contains("answer-b-only"));
    let out = read_only(&r, &b);
    assert!(out.1.contains("answer-b-only") && !out.1.contains("answer-a-only"));
    assert_ne!(read_only(&r, "missing-id").0, Some(0));
    assert_eq!(
        r.run(&["--id", &a, &r.owner.home, "must never replay"]).0,
        Some(2)
    );
    assert_eq!(r.run(&["--status"]).0, Some(2));
    assert_eq!(r.asks().len(), 2);
}

#[test]
fn r11_replacement_target_refuses_even_when_old_name_still_exists() {
    let r = Rig::new("o3sreplace");
    let id = r.ask("bound to old target UUID");
    r.owner
        .meta("0199c0de-aaaa-4890-abcd-ef0123456789", "colead");
    let out = read_only(&r, &id);
    assert_ne!(
        out.0,
        Some(0),
        "replacement is a gap, not old request state"
    );
}

#[test]
fn r13_bad_origin_grammar_keeps_ask_cap_and_shows_a_badge_gap() {
    let r = Rig::new("o3sorigin");
    let id = r.ask("still pending with damaged origin");
    let mut target = rows(&r.owner.tool.dir);
    let ask = target
        .iter_mut()
        .find(|e| e.get_str("action") == Some("ask"))
        .expect("ask");
    set(ask, "relay_origin", Value::str("not-a-uuid"));
    write_rows(&r.owner.tool.dir, &target);
    assert_eq!(
        ae::console::lane::open_asks(&events(&r.owner.tool.dir), &r.owner.home).len(),
        1
    );
    assert_ne!(
        read_only(&r, &id).0,
        Some(0),
        "status names malformed origin"
    );
}

#[test]
fn r11_same_origin_uuid_owns_the_audit_after_orchestrator_rename() {
    let mut r = Rig::new("o3srename");
    let id = r.ask("origin and target stable identity");
    assert!(r.world.tmux("rename-session -t", &["=orch", "renamed"]).0);
    let renamed = r.caller.dir.with_file_name("renamed");
    fs::rename(&r.caller.dir, &renamed).expect("ordinary session-dir rename");
    r.caller.dir = renamed;
    let meta = fs::read_to_string(r.caller.dir.join("meta")).expect("meta");
    fs::write(
        r.caller.dir.join("meta"),
        meta.replace("session=orch\n", "session=renamed\n"),
    )
    .expect("renamed meta");
    assert_eq!(
        read_only(&r, &id).0,
        Some(0),
        "same origin UUID owns audit after rename"
    );
}

#[test]
fn r11_target_uuid_follows_rename_despite_reused_name() {
    let mut r = Rig::new("o3sfollow");
    let id = r.ask("follow the UUID holding this ask");
    r.reply(&id, "answer remains with renamed UUID");
    let old = r.owner.home.clone();
    r.owner.tmux(&["rename-session", "-t", &old, "moved"]);
    let moved = r.owner.tool.dir.with_file_name("moved");
    fs::rename(&r.owner.tool.dir, &moved).expect("rename target directory");
    r.owner.tool.dir = moved;
    r.owner.home = "moved".to_owned();
    r.owner
        .meta("0199c0de-cccc-4890-abcd-ef0123456789", "colead");
    let replacement = r.world.session(&old, "lead", "");
    let before = tree(&replacement.dir);
    let out = read_only(&r, &id);
    assert_eq!(
        out.0,
        Some(0),
        "follow UUID after name reuse: {}",
        words(&out)
    );
    receipt(&out, &id, "sent", "replied");
    assert!(out.1.contains("answer remains with renamed UUID"));
    assert_eq!(tree(&replacement.dir), before, "replacement untouched");
}

#[test]
fn r7_missing_reply_body_is_a_gap_without_a_reply_echo() {
    let r = Rig::new("o3sbodygap");
    let id = r.ask("whole admitted answer required");
    r.reply(&id, "body about to become unreadable");
    let target = rows(&r.owner.tool.dir);
    let answer = target
        .iter()
        .find(|e| e.get_str("action") == Some("reply"))
        .expect("reply");
    fs::remove_file(answer.get_str("body_file").expect("body path")).expect("remove body");
    let out = read_only(&r, &id);
    assert_eq!(out.0, Some(1), "current body gap");
    receipt(&out, &id, "sent", "replied");
    assert!(
        out.2.contains(&id) && out.2.contains("body"),
        "named body gap: {}",
        words(&out)
    );
    assert_eq!(echo_count(&r, &id), 1, "no fabricated reply echo");
}
