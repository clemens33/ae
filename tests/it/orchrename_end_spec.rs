//! Bound end acceptance for the proved dispatcher and ordinary seats.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures own the session records, retained archive and private tmux"
)]

use super::o2guard_p2_spec::uuid_of;
use super::orchrename_rename_spec::{declare, dispatcher, live, owner_input, refuses, succeeds};
use super::relay_chat_spec::{Out, events, words};
use super::renametell_routing_spec::{A, B, Rig};
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

const DISPATCHER: &str = "orchestrator";
const TARGET: &str = "end-target";
const RENAMED: &str = "end-topic";
const OTHER: &str = "end-other";

fn end(rig: &Rig, target: &str, id: &str, pane: Option<&str>, approval: Option<&str>) -> Out {
    let mut args = vec!["end", target, "--keep-history", "--expect-uuid", id];
    if let Some(approval) = approval {
        args.extend(["--owner-input", approval]);
    }
    rig.run(&args, pane)
}

fn record_id(rig: &Rig, name: &str) -> String {
    let meta = fs::read_to_string(rig.dir(name).join("meta")).expect("session record");
    meta.lines()
        .find_map(|line| line.strip_prefix("session_id="))
        .expect("the recorded canonical UUID")
        .to_owned()
}

fn archive(rig: &Rig, id: &str) -> PathBuf {
    rig.root.join("archive").join(id)
}

fn assert_ended(rig: &Rig, name: &str, id: &str) {
    assert!(!live(rig, name), "the ended session {name} is still live");
    assert!(
        !rig.dir(name).exists(),
        "the ended session {name} is still registered"
    );
    let kept = archive(rig, id);
    assert!(kept.is_dir(), "no retained archive for {id}");
    let meta = fs::read_to_string(kept.join("meta")).expect("kept session metadata");
    assert!(
        meta.lines()
            .any(|row| row == format!("source_session_id={id}")),
        "{meta}"
    );
    assert!(
        meta.lines()
            .any(|row| row == format!("source_session={name}")),
        "{meta}"
    );
    assert!(meta.lines().any(|row| row == "reason=end"), "{meta}");
}

fn assert_audit(rig: &Rig, caller: &str, target: &str, id: &str, input: Option<&str>) {
    let rows = events(&rig.dir(DISPATCHER));
    let matches = |row: &&ae::events::Event| {
        row.actor == "ae/orchestrator"
            && row.actor_session_id.value() == Some(A)
            && row.caller_session_uuid.as_deref() == Some(A)
            && row.caller_pane.as_deref() == Some(caller)
            && row.target_session.value() == Some(target)
            && row.target_session_uuid.as_deref() == Some(id)
            && row.owner_input.as_deref() == input
    };
    let request = rows
        .iter()
        .filter(matches)
        .find(|row| {
            row.action == "dispatch-request"
                && row.summary.as_deref() == Some(format!("end {target}").as_str())
        })
        .expect("end request in the dispatcher journal");
    assert!(request.reference.is_some());
    assert!(
        rows.iter()
            .filter(matches)
            .any(|row| row.action == "dispatch-result"
                && row.reference == request.reference
                && row.summary.as_deref() == Some(format!("end {target}: ok").as_str()))
    );
}

fn all_done(rig: &Rig, target: &str, panes: &[String; 2]) {
    for pane in panes {
        declare(rig, target, pane, "done");
    }
}

fn append_pending(rig: &Rig, target: &str, id: &str) {
    rig.append(
        target,
        &format!(r#"{{"ts":"2026-11-20T16:00:00Z","actor":"lead","action":"ask","target":"colead","ref":"end-fixture-pending","actor_slot":"main","actor_session":"{target}","actor_session_id":"{id}","target_slot":"worker.0","target_session":"{target}","target_session_id":"{id}","summary":"Still awaiting the answer"}}"#),
    );
}

fn assert_pending(rig: &Rig, target: &str) {
    let out = rig.helper(target, "requests", &["all"], None);
    succeeds(&out);
    assert!(
        out.1
            .lines()
            .any(|row| row.starts_with("pending") && row.contains("end-fixture-pending")),
        "{}",
        words(&out)
    );
}

#[test]
fn the_dispatcher_can_keep_the_session_it_created_even_after_a_rename() {
    let rig = Rig::new("ore-create");
    let caller = dispatcher(&rig);
    let config = fs::read_to_string(&rig.config).expect("private config");
    fs::write(&rig.config, format!("{config}purge_agent_history = true\n"))
        .expect("a destructive default must not override the bound Keep form");
    let work = rig.scratch.join("project");
    fs::create_dir(&work).expect("private local project");
    succeeds(&rig.run(
        &[
            TARGET,
            "--dir",
            work.to_str().expect("project path"),
            "--local",
            "--no-attach",
        ],
        Some(&caller),
    ));
    let id = record_id(&rig, TARGET);
    let created = events(&rig.dir(DISPATCHER));
    let create_row = created
        .iter()
        .find(|row| {
            row.action == "dispatch-result"
                && row.summary.as_deref() == Some(format!("create {TARGET}: ok").as_str())
                && row.target_session_uuid.as_deref() == Some(id.as_str())
        })
        .expect("real dispatcher create completes in its audit");
    assert!(
        created
            .iter()
            .any(|row| row.action == "dispatch-request" && row.reference == create_row.reference)
    );
    succeeds(&rig.run(
        &["rename", TARGET, RENAMED, "--expect-uuid", &id],
        Some(&caller),
    ));

    let note = "An operator can recover this memo from the kept archive.";
    succeeds(&rig.helper(RENAMED, "memo", &["add", "--topic", "keep", note], None));
    let sessions = rig.dir(RENAMED);
    let kept = sessions.join("messages");
    fs::create_dir_all(&kept).expect("retained message directory");
    fs::write(kept.join("turn.txt"), b"prior conversation bytes").expect("kept message");
    rig.append(
        RENAMED,
        r#"{"ts":"2026-11-20T15:59:00Z","actor":"lead","action":"fixture-note","summary":"retained event"}"#,
    );
    assert!(
        events(&sessions)
            .iter()
            .all(|row| row.declared_state() != Some("done"))
    );

    succeeds(&end(&rig, RENAMED, &id, Some(&caller), None));
    assert_ended(&rig, RENAMED, &id);
    assert_audit(&rig, &caller, RENAMED, &id, None);
    let kept = archive(&rig, &id);
    assert!(
        fs::read_to_string(kept.join("memo.tsv"))
            .expect("kept memo")
            .contains(note)
    );
    assert!(
        events(&kept)
            .iter()
            .any(|row| row.summary.as_deref() == Some("retained event"))
    );
    assert_eq!(
        fs::read(kept.join("messages/turn.txt")).expect("kept conversation"),
        b"prior conversation bytes"
    );
    assert!(work.is_dir(), "ending a local session keeps the workspace");
}

#[test]
fn a_real_dispatcher_start_also_grants_end_authority() {
    let rig = Rig::new("ore-start");
    let caller = dispatcher(&rig);
    let work = rig.scratch.join("project");
    fs::create_dir(&work).expect("private local project");
    succeeds(&rig.run(
        &[
            TARGET,
            "--dir",
            work.to_str().expect("project path"),
            "--local",
            "--no-attach",
        ],
        None,
    ));
    let id = record_id(&rig, TARGET);
    succeeds(&rig.run(&["stop", TARGET], None));
    succeeds(&rig.run(&[TARGET, "--no-attach"], Some(&caller)));
    assert!(live(&rig, TARGET));
    let starts = events(&rig.dir(DISPATCHER));
    let started = starts
        .iter()
        .find(|row| {
            row.action == "dispatch-result"
                && row.summary.as_deref() == Some(format!("start {TARGET}: ok").as_str())
                && row.target_session_uuid.as_deref() == Some(id.as_str())
        })
        .expect("real dispatcher start completes in its audit");
    assert!(
        starts
            .iter()
            .any(|row| row.action == "dispatch-request" && row.reference == started.reference)
    );
    assert!(
        events(&rig.dir(TARGET))
            .iter()
            .all(|row| row.declared_state() != Some("done"))
    );
    succeeds(&end(&rig, TARGET, &id, Some(&caller), None));
    assert_ended(&rig, TARGET, &id);
    assert_audit(&rig, &caller, TARGET, &id, None);
}

#[test]
fn the_unowned_end_requires_all_seats_done_including_stopped_targets() {
    let rig = Rig::new("ore-done");
    let caller = dispatcher(&rig);
    let panes = rig.create(TARGET, B);
    declare(&rig, TARGET, &panes[0], "done");
    refuses(&end(&rig, TARGET, B, Some(&caller), None));
    assert!(live(&rig, TARGET));
    assert!(!archive(&rig, B).exists());

    succeeds(&rig.helper(
        TARGET,
        "state",
        &[
            "waiting-user",
            "Should the fixture keep waiting? 1. Keep this session open | \
             2. Finish the work (recommend 1 to exercise the all-done rule).",
        ],
        Some(&panes[1]),
    ));
    refuses(&end(&rig, TARGET, B, Some(&caller), None));
    assert!(live(&rig, TARGET));
    assert!(!archive(&rig, B).exists());

    declare(&rig, TARGET, &panes[1], "done");
    succeeds(&rig.run(&["stop", TARGET], None));
    assert!(!live(&rig, TARGET));
    succeeds(&end(&rig, TARGET, B, Some(&caller), None));
    assert_ended(&rig, TARGET, B);
    assert_audit(&rig, &caller, TARGET, B, None);
}

#[test]
fn a_live_unowned_target_with_every_seat_done_and_no_ask_can_end() {
    let rig = Rig::new("ore-live");
    let caller = dispatcher(&rig);
    let panes = rig.create(TARGET, B);
    all_done(&rig, TARGET, &panes);
    succeeds(&end(&rig, TARGET, B, Some(&caller), None));
    assert_ended(&rig, TARGET, B);
    assert_audit(&rig, &caller, TARGET, B, None);
}

#[test]
fn authenticated_owner_words_waive_the_end_state_gate_once() {
    let rig = Rig::new("ore-input");
    let caller = dispatcher(&rig);
    let panes = rig.create(TARGET, B);
    declare(&rig, TARGET, &panes[0], "done");
    declare(&rig, TARGET, &panes[1], "working");
    refuses(&end(&rig, TARGET, B, Some(&caller), None));
    for (update, text) in [
        (731, format!("rename {TARGET}")),
        (732, format!("end {TARGET}-shadow")),
    ] {
        let wrong = owner_input(&rig, &text, update);
        refuses(&end(&rig, TARGET, B, Some(&caller), Some(&wrong)));
        assert!(live(&rig, TARGET));
    }
    let approved = owner_input(&rig, &format!("end {TARGET}"), 733);
    succeeds(&end(&rig, TARGET, B, Some(&caller), Some(&approved)));
    assert_ended(&rig, TARGET, B);
    assert_audit(&rig, &caller, TARGET, B, Some(&approved));

    let new_id = uuid_of(OTHER);
    let replacement = rig.create(TARGET, &new_id);
    declare(&rig, TARGET, &replacement[1], "working");
    let replay = end(&rig, TARGET, &new_id, Some(&caller), Some(&approved));
    refuses(&replay);
    assert!(
        words(&replay).contains("already authorized"),
        "{}",
        words(&replay)
    );
    assert!(live(&rig, TARGET));
    assert!(!archive(&rig, &new_id).exists());
}

#[test]
fn a_pending_ask_keeps_a_done_target_until_the_owner_approves_end() {
    let rig = Rig::new("ore-open");
    let caller = dispatcher(&rig);
    let panes = rig.create(TARGET, B);
    all_done(&rig, TARGET, &panes);
    append_pending(&rig, TARGET, B);
    assert_pending(&rig, TARGET);
    let rows = events(&rig.dir(TARGET));
    refuses(&end(&rig, TARGET, B, Some(&caller), None));
    assert!(live(&rig, TARGET));
    assert_eq!(events(&rig.dir(TARGET)), rows);
    assert!(!archive(&rig, B).exists());
    let approval = owner_input(&rig, &format!("end {TARGET}"), 734);
    succeeds(&end(&rig, TARGET, B, Some(&caller), Some(&approval)));
    assert_ended(&rig, TARGET, B);
    assert_audit(&rig, &caller, TARGET, B, Some(&approval));
    assert!(
        events(&archive(&rig, B))
            .iter()
            .any(|row| row.reference.as_deref() == Some("end-fixture-pending"))
    );
}

#[test]
fn end_never_uses_rm_force_fleet_or_purge_as_a_dispatcher_route() {
    let rig = Rig::new("ore-routes");
    let caller = dispatcher(&rig);
    let panes = rig.create(TARGET, B);
    let duplicate = rig.create(OTHER, A);
    all_done(&rig, TARGET, &panes);
    all_done(&rig, OTHER, &duplicate);
    let invalid = uuid_of("absent-end-target");
    let target_rows = events(&rig.dir(TARGET));
    let dispatcher_rows = events(&rig.dir(DISPATCHER));
    for args in [
        vec!["end", TARGET, "--purge-history", "--expect-uuid", B],
        vec!["end", TARGET, "--keep-history", "--expect-uuid", B, "-f"],
        vec!["end", TARGET, "--expect-uuid", B],
        vec!["end", "all", "--keep-history", "--expect-uuid", B],
        vec!["end", TARGET, "--keep-history", "--expect-uuid", &invalid],
        vec!["rm", TARGET, "--keep-history", "--expect-uuid", B],
        vec![
            "_end",
            "--bound",
            TARGET,
            "--keep-history",
            "--expect-uuid",
            B,
            "--supervise",
        ],
    ] {
        refuses(&rig.run(&args, Some(&caller)));
    }
    for name in [DISPATCHER, OTHER] {
        let blocked = end(&rig, name, A, Some(&caller), None);
        refuses(&blocked);
        assert!(words(&blocked).contains("own"), "{}", words(&blocked));
        assert!(live(&rig, name));
    }
    let plain = rig.run(&["end", TARGET, "-f", "--keep-history"], Some(&caller));
    refuses(&plain);
    assert!(words(&plain).contains("--expect-uuid"), "{}", words(&plain));
    assert_eq!(events(&rig.dir(DISPATCHER)), dispatcher_rows);
    assert_eq!(events(&rig.dir(TARGET)), target_rows);
    assert!(live(&rig, TARGET));
    assert!(!archive(&rig, B).exists());
}

fn claimed_create_row(
    rig: &Rig,
    caller: &str,
    verb: &str,
    reference: &str,
    actor: &str,
    reported: &str,
    result: Option<&str>,
) {
    let (action, detail) = result.map_or_else(
        || ("dispatch-request", format!("{verb} {TARGET}")),
        |outcome| ("dispatch-result", format!("{verb} {TARGET}: {outcome}")),
    );
    let server = rig.socket.display();
    rig.append(
        DISPATCHER,
        &format!(
            r#"{{"ts":"2026-11-20T15:59:00Z","actor":"ae/orchestrator","action":"{action}","target":"{TARGET}","ref":"{reference}","actor_slot":"main","actor_session":"{DISPATCHER}","actor_session_id":"{actor}","caller_pane":"{caller}","caller_server":"{server}","caller_session_uuid":"{actor}","target_session":"{TARGET}","target_session_id":"{reported}","target_session_uuid":"{reported}","summary":"{detail}"}}"#
        ),
    );
}

#[test]
fn a_lone_failed_resume_foreign_or_wrong_uuid_result_grants_no_authority() {
    let rig = Rig::new("ore-ledger");
    let caller = dispatcher(&rig);
    let panes = rig.create(TARGET, B);
    declare(&rig, TARGET, &panes[1], "working");
    let other = uuid_of(OTHER);
    claimed_create_row(&rig, &caller, "create", "dispatch-lone", A, B, Some("ok"));
    refuses(&end(&rig, TARGET, B, Some(&caller), None));
    assert!(live(&rig, TARGET));
    assert!(!archive(&rig, B).exists());
    for (verb, reference, actor, reported, outcome) in [
        ("create", "dispatch-failed", A, B, "failed (exit 1)"),
        ("resume", "dispatch-resume", A, B, "ok"),
        ("start", "dispatch-foreign", other.as_str(), B, "ok"),
        ("start", "dispatch-wrong-target", A, other.as_str(), "ok"),
    ] {
        claimed_create_row(&rig, &caller, verb, reference, actor, reported, None);
        claimed_create_row(
            &rig,
            &caller,
            verb,
            reference,
            actor,
            reported,
            Some(outcome),
        );
        refuses(&end(&rig, TARGET, B, Some(&caller), None));
        assert!(live(&rig, TARGET));
        assert!(!archive(&rig, B).exists());
    }
}

#[test]
fn a_pending_ask_written_after_the_stop_keeps_the_target_registered() {
    let rig = Rig::new("ore-recheck");
    let caller = dispatcher(&rig);
    let panes = rig.create(TARGET, B);
    all_done(&rig, TARGET, &panes);
    let memo_hold = ae::store::lock(&rig.dir(TARGET).join("memo.tsv.lock"), Duration::ZERO)
        .expect("hold the archive memo read until the new ask lands");
    let child = rig
        .command(Some(&caller))
        .args(["end", TARGET, "--keep-history", "--expect-uuid", B])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the bound end");
    let until = Instant::now() + Duration::from_secs(30);
    while live(&rig, TARGET) {
        assert!(
            Instant::now() < until,
            "the bound end never reached its stop"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    append_pending(&rig, TARGET, B);
    drop(memo_hold);
    let out = child.wait_with_output().expect("bound end result");
    assert!(!out.status.success(), "{out:?}");
    assert!(
        rig.dir(TARGET).is_dir(),
        "the stopped record must stay resumable"
    );
    assert_eq!(record_id(&rig, TARGET), B);
    assert!(
        !archive(&rig, B).exists(),
        "the veto must precede archive publication"
    );
    assert_pending(&rig, TARGET);
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(
        text.contains("stopped") && text.contains("pending"),
        "{text}"
    );
}

#[test]
fn an_admitted_end_will_not_stop_a_changed_incarnation() {
    let rig = Rig::new("ore-fence");
    let caller = dispatcher(&rig);
    let panes = rig.create(TARGET, B);
    all_done(&rig, TARGET, &panes);
    let audit_hold = ae::store::lock(
        &rig.dir(DISPATCHER).join("events.jsonl.lock"),
        Duration::ZERO,
    )
    .expect("hold the dispatcher audit");
    let child = rig
        .command(Some(&caller))
        .args(["end", TARGET, "--keep-history", "--expect-uuid", B])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("bound end process");
    let target_lock = rig.dir(TARGET).join("events.jsonl.lock");
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let Ok(probe) = ae::store::lock(&target_lock, Duration::ZERO) else {
            break;
        };
        drop(probe);
        assert!(
            Instant::now() < until,
            "end never held target events before audit"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    rig.tmux(&[
        "set-option",
        "-t",
        TARGET,
        "@ae_session_uuid",
        &uuid_of(OTHER),
    ]);
    drop(audit_hold);
    let out = child.wait_with_output().expect("end process result");
    assert!(!out.status.success(), "{out:?}");
    assert!(live(&rig, TARGET));
    assert_eq!(record_id(&rig, TARGET), B);
    assert!(!archive(&rig, B).exists());
    let rows = events(&rig.dir(DISPATCHER));
    let requests: Vec<_> = rows
        .iter()
        .filter(|row| row.action == "dispatch-request")
        .collect();
    let [request] = requests.as_slice() else {
        panic!("one admitted end request before the fence: {rows:?}");
    };
    assert!(request.reference.is_some());
    assert_eq!(request.target_session_uuid.as_deref(), Some(B));
    assert!(
        rows.iter().any(|row| row.action == "dispatch-result"
            && row.reference == request.reference
            && row
                .summary
                .as_deref()
                .is_some_and(|value| !value.ends_with(": ok"))),
        "the fence refusal needs a result for the original request: {rows:?}"
    );
}

#[test]
fn an_unavailable_dispatcher_audit_leaves_an_admissible_target_live() {
    let rig = Rig::new("ore-audit");
    let caller = dispatcher(&rig);
    let panes = rig.create(TARGET, B);
    all_done(&rig, TARGET, &panes);
    fs::create_dir(rig.dir(DISPATCHER).join("events.jsonl"))
        .expect("the audit source cannot accept a row");
    let before = events(&rig.dir(TARGET));
    refuses(&end(&rig, TARGET, B, Some(&caller), None));
    assert!(live(&rig, TARGET));
    assert_eq!(record_id(&rig, TARGET), B);
    assert_eq!(events(&rig.dir(TARGET)), before);
    assert!(!archive(&rig, B).exists());
}

fn assert_cross_refusal(rig: &Rig, caller: &str, args: &[&str], expected_count: usize) {
    let before = events(&rig.dir(TARGET));
    let out = rig.run(args, Some(caller));
    assert_eq!(out.0, Some(2), "{}", words(&out));
    let verb = args[0];
    assert!(
        words(&out).contains(&format!("ae {verb} {TARGET}")),
        "{}",
        words(&out)
    );
    assert_eq!(events(&rig.dir(TARGET)), before);
    assert!(live(rig, TARGET));
    assert!(!archive(rig, B).exists());
    let rows = events(&rig.dir(OTHER));
    let refused: Vec<_> = rows
        .iter()
        .filter(|row| row.action == "lifecycle-refused")
        .collect();
    assert_eq!(refused.len(), expected_count, "{rows:?}");
    let row = refused[expected_count - 1];
    assert_eq!(row.actor, "lead");
    assert_eq!(row.actor_slot.value(), Some("main"));
    assert_eq!(row.actor_session.value(), Some(OTHER));
    assert_eq!(row.actor_session_id.value(), Some(A));
    assert_eq!(row.target.as_deref(), Some(TARGET));
    assert_eq!(row.caller_pane.as_deref(), Some(caller));
    assert_eq!(row.caller_session_uuid.as_deref(), Some(A));
    assert_eq!(row.target_session_uuid.as_deref(), Some(B));
    assert_eq!(
        row.summary.as_deref(),
        Some(
            format!("{verb} refused: an agent seat runs no lifecycle form on another session")
                .as_str()
        )
    );
}

#[test]
fn ordinary_seats_refuse_foreign_end_and_rm_while_the_owners_forms_still_work() {
    let rig = Rig::new("ore-seat");
    let [caller, _] = rig.create(OTHER, A);
    rig.create("unaffected-session", &uuid_of("unaffected-session"));
    rig.create(TARGET, B);
    for (index, args) in [
        vec!["end", TARGET, "-f", "--keep-history"],
        vec!["rm", TARGET, "-f", "--keep-history"],
        vec!["end", TARGET, "--keep-history", "--expect-uuid", B],
    ]
    .iter()
    .enumerate()
    {
        assert_cross_refusal(&rig, &caller, args, index + 1);
    }
    succeeds(&rig.run(&["rm", OTHER, "-f", "--keep-history"], Some(&caller)));
    let until = Instant::now() + Duration::from_secs(30);
    while rig.dir(OTHER).exists() {
        assert!(
            Instant::now() < until,
            "the own-session rm did not finish its existing handoff"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_ended(&rig, OTHER, A);
    succeeds(&end(&rig, TARGET, B, None, None));
    assert_ended(&rig, TARGET, B);

    let purge_id = uuid_of("owner-purge-control");
    rig.create(TARGET, &purge_id);
    succeeds(&rig.run(&["rm", TARGET, "-f", "--purge-history"], None));
    assert!(!live(&rig, TARGET));
    assert!(!rig.dir(TARGET).exists());
    assert!(!archive(&rig, &purge_id).exists());
}

#[test]
fn the_charter_teaches_the_admitted_forms_and_checks_the_lane_before_end() {
    let charter = include_str!("../../contrib/aeorchestrator/CHARTER.md");
    let config = include_str!("../../contrib/aeorchestrator/orchestrator.config");
    let (_, prompt) = config.split_once("[prompt]\n").expect("seeded prompt");
    let readme = include_str!("../../contrib/aeorchestrator/README.md");
    let rename = "ae rename <old> <new> --expect-uuid <uuid>";
    let end = "ae end <name> --keep-history --expect-uuid <uuid>";
    let create = "ae <name> --dir <path> --local";
    for source in [charter, prompt, readme] {
        for form in [rename, end, create] {
            assert!(source.contains(form), "missing admitted form {form:?}");
        }
        assert!(
            !source
                .to_ascii_lowercase()
                .contains("runs no rename command")
        );
        assert!(!source.contains("Run no rename command."));
    }
    for source in [charter, prompt] {
        assert!(source.contains(
            "never end a session whose seats hold or wait for a build or test lane claim"
        ));
    }
}
