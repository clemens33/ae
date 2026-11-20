//! Bound rename acceptance through the public CLI and private tmux.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures read their private session records and lifecycles"
)]

use super::o2guard_p2_spec::uuid_of;
use super::relay_chat_spec::{Out, events, words};
use super::renametell_routing_spec::{A, B, Rig};
use ae::telegram::{
    Echoes,
    bridge::SharedChat,
    inbound::{Origin, Submit, Submitted},
    routing::Target,
};
use std::fs;
use std::process::Stdio;
use std::time::{Duration, Instant};

const DISPATCHER: &str = "orchestrator";
const OLD: &str = "rename-target";
const NEW: &str = "rename-topic";
const OTHER: &str = "rename-other";

pub(super) fn dispatcher(rig: &Rig) -> String {
    let [pane, _] = rig.create(DISPATCHER, A);
    let overlay = rig.root.join("orchestrator.config");
    fs::write(
        &overlay,
        include_str!("../../contrib/aeorchestrator/orchestrator.config"),
    )
    .expect("current overlay in the private ae root");
    let path = rig.dir(DISPATCHER).join("meta");
    let meta = fs::read_to_string(&path).expect("dispatcher fixture meta");
    fs::write(
        path,
        format!(
            "{meta}meta_agent=true\nlocal_config={}\n",
            overlay.display()
        ),
    )
    .expect("dispatcher role and local overlay");
    pane
}

pub(super) fn succeeds(out: &Out) {
    assert_eq!(out.0, Some(0), "{}", words(out));
}

pub(super) fn refuses(out: &Out) {
    assert!(matches!(out.0, Some(1 | 2)), "{}", words(out));
}

pub(super) fn live(rig: &Rig, name: &str) -> bool {
    rig.tmux(&["list-sessions", "-F", "#{session_name}"])
        .lines()
        .any(|row| row == name)
}

pub(super) fn declare(rig: &Rig, name: &str, pane: &str, state: &str) {
    succeeds(&rig.helper(
        name,
        "state",
        &[
            state,
            "The acceptance fixture records the real seat declaration before the lifecycle action.",
        ],
        Some(pane),
    ));
}

pub(super) fn owner_input(rig: &Rig, body: &str, update_id: i64) -> String {
    let chat = SharedChat {
        root: rig.root.clone(),
        echoes: Echoes::default(),
    };
    let target = Target {
        session: DISPATCHER.to_owned(),
        dir: rig.dir(DISPATCHER),
        session_id: A.to_owned(),
        pair: vec!["lead".to_owned(), "colead".to_owned()],
        seat: Some("lead".to_owned()),
    };
    let origin = Origin {
        update_id,
        date: ae::time::Timestamp::now().epoch(),
        from_id: "42".to_owned(),
    };
    let result = chat.submit(&target, body, &origin);
    assert!(
        matches!(
            result,
            Submitted::Sent { .. } | Submitted::Unconfirmed { .. }
        ),
        "{result:?}"
    );
    let id = origin.request_id();
    assert!(
        events(&rig.dir(DISPATCHER))
            .iter()
            .any(|row| row.action == "ask"
                && row.actor == "console:local"
                && row.reference.as_deref() == Some(id.as_str()))
    );
    id
}

fn rename(
    rig: &Rig,
    old: &str,
    new: &str,
    id: &str,
    pane: Option<&str>,
    input: Option<&str>,
) -> Out {
    let mut args = vec!["rename", old, new, "--expect-uuid", id];
    if let Some(input) = input {
        args.extend(["--owner-input", input]);
    }
    rig.run(&args, pane)
}

fn assert_renamed(rig: &Rig, old: &str, new: &str, id: &str, running: bool) {
    assert!(!live(rig, old), "old live name {old} did not move");
    assert_eq!(live(rig, new), running, "new live name {new}");
    let meta = fs::read_to_string(rig.dir(new).join("meta")).expect("renamed record");
    assert!(meta.lines().any(|row| row == format!("session={new}")));
    assert!(meta.lines().any(|row| row == format!("session_id={id}")));
    if running {
        assert_eq!(
            rig.tmux(&["show-options", "-v", "-t", new, "@ae_session_uuid"])
                .trim(),
            id
        );
        assert!(
            events(&rig.dir(new))
                .iter()
                .any(|row| row.action == "rename"
                    && row.summary.as_deref() == Some(format!("{old} -> {new}").as_str()))
        );
    }
    assert!(
        rig.root
            .join("sessions")
            .join(format!(".lifecycle.{new}.lock"))
            .is_file(),
        "the new name's lifecycle lock was acquired"
    );
}

fn assert_audit(rig: &Rig, pane: &str, old: &str, new: &str, id: &str, input: Option<&str>) {
    let rows = events(&rig.dir(DISPATCHER));
    let matches = |row: &&ae::events::Event| {
        row.actor == "ae/orchestrator"
            && row.caller_pane.as_deref() == Some(pane)
            && row.actor_session_id.value() == Some(A)
            && row.caller_session_uuid.as_deref() == Some(A)
            && row.target_session.value() == Some(old)
            && row.target_session_uuid.as_deref() == Some(id)
            && row.owner_input.as_deref() == input
            && row.summary.as_deref().is_some_and(|text| {
                text.starts_with("rename ") && text.contains(old) && text.contains(new)
            })
    };
    let request = rows
        .iter()
        .filter(matches)
        .find(|row| row.action == "dispatch-request")
        .expect("rename request in the dispatcher journal");
    assert!(request.reference.is_some());
    assert!(rows.iter().filter(matches).any(|row| {
        row.action == "dispatch-result"
            && row.reference == request.reference
            && row
                .summary
                .as_deref()
                .is_some_and(|text| text.ends_with(": ok"))
    }));
}

#[test]
fn a_live_bound_rename_keeps_the_uuid_memo_open_ask_history_and_journal() {
    let rig = Rig::new("orn-live");
    let caller = dispatcher(&rig);
    let [lead, peer] = rig.create(OLD, B);
    let note = "The session memo predates the change of name.";
    succeeds(&rig.helper(
        OLD,
        "memo",
        &["add", "--topic", "continuity", note],
        Some(&lead),
    ));
    let question = "The open request predates the change of name.";
    let ask = rig.helper(
        OLD,
        "ask",
        &["--cross-session", "colead", question],
        Some(&lead),
    );
    succeeds(&ask);
    let id = ask.1.trim();
    assert!(!id.is_empty(), "{}", words(&ask));
    let journal = events(&rig.dir(OLD));
    let opening = journal
        .iter()
        .find(|row| {
            row.action == "ask"
                && row.reference.as_deref() == Some(id)
                && row.actor_session_id.value() == Some(B)
                && row.target_session_id.value() == Some(B)
        })
        .expect("real routed ask opening");
    let body_path = std::path::Path::new(opening.body_file.as_deref().expect("saved ask body"));
    let body_name = body_path.file_name().expect("saved body name");
    let saved_body = fs::read(rig.dir(OLD).join("messages").join(body_name))
        .expect("original saved conversation body");
    let old_place = rig.tmux(&["display-message", "-p", "-t", OLD, "#{session_id}"]);

    succeeds(&rename(&rig, OLD, NEW, B, Some(&caller), None));
    assert_renamed(&rig, OLD, NEW, B, true);
    assert_eq!(
        rig.tmux(&["display-message", "-p", "-t", NEW, "#{session_id}"]),
        old_place
    );
    assert_audit(&rig, &caller, OLD, NEW, B, None);
    assert_eq!(
        fs::read(rig.dir(NEW).join("messages").join(body_name))
            .expect("retained conversation body"),
        saved_body
    );

    let memo = rig.helper(NEW, "memo", &["read", "--topic", "continuity"], Some(&lead));
    succeeds(&memo);
    assert!(memo.1.contains(note), "{}", words(&memo));
    for (pane, view) in [(&lead, "mine"), (&peer, "inbox")] {
        let requests = rig.helper(NEW, "requests", &[view], Some(pane));
        succeeds(&requests);
        assert!(
            requests
                .1
                .lines()
                .any(|line| line.starts_with("pending") && line.contains(id)),
            "{}",
            words(&requests)
        );
    }
    let answer = "The original asker receives this answer under the new name.";
    let reply = rig.helper(NEW, "reply", &[id, answer], Some(&peer));
    succeeds(&reply);
    assert!(
        rig.received(B, "lead")
            .iter()
            .any(|text| text.contains(answer))
    );
    assert!(
        rig.received(B, "colead")
            .iter()
            .any(|text| text.contains(question))
    );
    let after = events(&rig.dir(NEW));
    assert!(
        after.starts_with(&journal),
        "pre-rename journal rows remain available"
    );
}

#[test]
fn a_stopped_bound_rename_keeps_its_memo_and_never_starts_it() {
    let rig = Rig::new("orn-stopped");
    let caller = dispatcher(&rig);
    let [lead, _] = rig.create(OLD, B);
    let note = "The stopped session remembers this note.";
    succeeds(&rig.helper(
        OLD,
        "memo",
        &["add", "--topic", "memory", note],
        Some(&lead),
    ));
    succeeds(&rig.run(&["stop", OLD], None));
    assert!(!live(&rig, OLD));
    succeeds(&rename(&rig, OLD, NEW, B, Some(&caller), None));
    assert_renamed(&rig, OLD, NEW, B, false);
    assert_audit(&rig, &caller, OLD, NEW, B, None);
    let memo = rig.helper(NEW, "memo", &["read", "--topic", "memory"], None);
    succeeds(&memo);
    assert!(memo.1.contains(note), "{}", words(&memo));
}

#[test]
fn own_name_and_own_uuid_refuse_and_the_plain_seat_form_names_the_bound_form() {
    let rig = Rig::new("orn-own");
    let caller = dispatcher(&rig);
    rig.create(OLD, B);
    rig.create(OTHER, A);
    let before = events(&rig.dir(DISPATCHER));
    for name in [DISPATCHER, OTHER] {
        let blocked = rename(&rig, name, NEW, A, Some(&caller), None);
        refuses(&blocked);
        assert!(words(&blocked).contains("own"), "{}", words(&blocked));
        assert!(
            words(&blocked).contains(&format!("ae rename {DISPATCHER} {NEW}")),
            "{}",
            words(&blocked)
        );
        assert!(live(&rig, name));
    }
    let plain = rig.run(&["rename", OLD, NEW], Some(&caller));
    refuses(&plain);
    assert!(words(&plain).contains("--expect-uuid"), "{}", words(&plain));
    assert_eq!(events(&rig.dir(DISPATCHER)), before);
    assert!(!live(&rig, NEW));
    assert!(!rig.dir(NEW).exists());
}

#[test]
fn the_target_uuid_must_match_and_neither_name_may_be_an_alias() {
    let rig = Rig::new("orn-names");
    let caller = dispatcher(&rig);
    rig.create(OLD, B);
    let other_id = uuid_of(OTHER);
    rig.create(OTHER, &other_id);
    let alias = "rename-vacated";
    // A real owner rename leaves the same alias a normal rename could reclaim.
    succeeds(&rig.run(&["rename", OTHER, alias], None));
    let link = fs::read_link(rig.dir(OTHER)).expect("old name is a real retained alias");
    let meta = fs::read(rig.dir(OLD).join("meta")).expect("target preimage");
    let before = events(&rig.dir(DISPATCHER));
    for (source, dest, id) in [
        (OLD, NEW, other_id.as_str()),
        (OLD, OTHER, B),
        (OLD, alias, B),
        (OLD, "bad name", B),
        (OLD, "stop", B),
        (OTHER, NEW, other_id.as_str()),
    ] {
        refuses(&rename(&rig, source, dest, id, Some(&caller), None));
        assert!(live(&rig, OLD));
        assert!(live(&rig, alias));
        assert_eq!(fs::read_link(rig.dir(OTHER)).expect("alias survives"), link);
        assert_eq!(
            fs::read(rig.dir(OLD).join("meta")).expect("target preimage"),
            meta
        );
        assert!(!live(&rig, NEW));
    }
    assert!(!rig.dir(NEW).exists());
    assert_eq!(events(&rig.dir(DISPATCHER)), before);
}

#[test]
fn only_one_genuine_owner_input_for_rename_can_waive_a_busy_declaration() {
    let rig = Rig::new("orn-busy");
    let caller = dispatcher(&rig);
    let [_, peer] = rig.create(OLD, B);
    declare(&rig, OLD, &peer, "working");
    let blocked = rename(&rig, OLD, NEW, B, Some(&caller), None);
    refuses(&blocked);
    assert!(words(&blocked).contains("working"), "{}", words(&blocked));

    for (update, text) in [
        (720, format!("end {OLD}")),
        (721, format!("rename {OLD}-other")),
    ] {
        let wrong = owner_input(&rig, &text, update);
        refuses(&rename(&rig, OLD, NEW, B, Some(&caller), Some(&wrong)));
        assert!(live(&rig, OLD));
        assert!(!rig.dir(NEW).exists());
    }
    let approved = owner_input(&rig, &format!("rename {OLD}"), 722);
    succeeds(&rename(&rig, OLD, NEW, B, Some(&caller), Some(&approved)));
    assert_audit(&rig, &caller, OLD, NEW, B, Some(&approved));
    // The ordinary owner can put the UUID and its still-busy seat back under
    // the old name, so replay would have the same verb, name and target.
    succeeds(&rig.run(&["rename", NEW, OLD], None));
    assert!(live(&rig, OLD));
    declare(&rig, OLD, &peer, "working");
    // The return move leaves NEW as an alias. An unused destination makes
    // the replay reach the owner-input check instead of the name check.
    let fresh = "rename-replay";
    let before = events(&rig.dir(DISPATCHER));
    let replay = rename(&rig, OLD, fresh, B, Some(&caller), Some(&approved));
    refuses(&replay);
    assert!(
        words(&replay).contains(&format!("owner input {approved} already authorized")),
        "{}",
        words(&replay)
    );
    assert_eq!(events(&rig.dir(DISPATCHER)), before);
    assert!(live(&rig, OLD));
    assert!(!live(&rig, fresh));
    assert!(!rig.dir(fresh).exists());
}

#[test]
fn a_pending_legacy_request_still_blocks_a_dispatcher_rename() {
    let rig = Rig::new("orn-strand");
    let caller = dispatcher(&rig);
    rig.create(OLD, B);
    rig.append(
        OLD,
        &format!(
            r#"{{"ts":"2026-10-10T10:00:00Z","actor":"lead","action":"ask","target":"colead","ref":"legacy-open","actor_slot":"main","actor_session":"{OLD}","target_slot":"worker.0","target_session":"{OLD}","summary":"question"}}"#
        ),
    );
    let rows = events(&rig.dir(OLD));
    let blocked = rename(&rig, OLD, NEW, B, Some(&caller), None);
    refuses(&blocked);
    assert!(
        words(&blocked).contains("legacy-open"),
        "{}",
        words(&blocked)
    );
    assert!(live(&rig, OLD));
    assert!(!live(&rig, NEW));
    assert_eq!(events(&rig.dir(OLD)), rows);
}

#[test]
fn an_ordinary_seat_cannot_rename_another_session_in_any_form() {
    let rig = Rig::new("orn-ordinary");
    let [caller, _] = rig.create(OTHER, A);
    rig.create(OLD, B);
    let target_rows = events(&rig.dir(OLD));
    for (index, args) in [
        vec!["rename", OLD, NEW],
        vec!["rename", OLD, NEW, "--expect-uuid", B],
    ]
    .iter()
    .enumerate()
    {
        let out = rig.run(args, Some(&caller));
        assert_eq!(out.0, Some(2), "{}", words(&out));
        assert!(
            words(&out).contains(&format!("ae rename {OLD} {NEW}")),
            "{}",
            words(&out)
        );
        assert!(live(&rig, OLD));
        assert!(!live(&rig, NEW));
        assert!(!rig.dir(NEW).exists());
        assert_eq!(events(&rig.dir(OLD)), target_rows);
        let rows = events(&rig.dir(OTHER));
        let blocked: Vec<_> = rows
            .iter()
            .filter(|row| row.action == "lifecycle-refused")
            .collect();
        assert_eq!(blocked.len(), index + 1, "{rows:?}");
        let row = blocked[index];
        assert_eq!(row.actor, "lead");
        assert_eq!(row.actor_slot.value(), Some("main"));
        assert_eq!(row.actor_session.value(), Some(OTHER));
        assert_eq!(row.caller_session_uuid.as_deref(), Some(A));
        assert_eq!(row.caller_pane.as_deref(), Some(caller.as_str()));
        assert_eq!(row.target.as_deref(), Some(OLD));
        assert_eq!(row.target_session_uuid.as_deref(), Some(B));
        assert_eq!(
            row.summary.as_deref(),
            Some("rename refused: an agent seat runs no lifecycle form on another session")
        );
    }
    succeeds(&rig.run(&["rename", "rename-own-topic"], Some(&caller)));
    assert_renamed(&rig, OTHER, "rename-own-topic", A, true);
    succeeds(&rename(&rig, OLD, NEW, B, None, None));
    assert_renamed(&rig, OLD, NEW, B, true);
}

#[test]
fn a_live_target_stamp_changed_after_admission_is_never_renamed() {
    let rig = Rig::new("orn-fence");
    let caller = dispatcher(&rig);
    rig.create(OLD, B);
    let target_record = fs::read(rig.dir(OLD).join("meta")).expect("fixture target record");
    let audit_path = rig.dir(DISPATCHER).join("events.jsonl.lock");
    let audit_hold = ae::store::lock(&audit_path, Duration::ZERO).expect("hold dispatcher audit");
    let child = rig
        .command(Some(&caller))
        .args(["rename", OLD, NEW, "--expect-uuid", B])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("bound rename process");
    let target_path = rig.dir(OLD).join("events.jsonl.lock");
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let Ok(probe) = ae::store::lock(&target_path, Duration::ZERO) else {
            break;
        };
        drop(probe);
        assert!(
            Instant::now() < until,
            "rename never held its target events before audit"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // The already-admitted tmux $id does not change. A plain exact-id rename
    // would still act on it, and only the bound stamp fence refuses the swap.
    rig.tmux(&["set-option", "-t", OLD, "@ae_session_uuid", &uuid_of(OTHER)]);
    drop(audit_hold);
    let out = child.wait_with_output().expect("rename process result");
    assert!(!out.status.success(), "{out:?}");
    let rows = events(&rig.dir(DISPATCHER));
    let requests: Vec<_> = rows
        .iter()
        .filter(|row| row.action == "dispatch-request")
        .collect();
    let [request] = requests.as_slice() else {
        panic!("one admitted rename request before the fence: {rows:?}");
    };
    assert_eq!(request.target_session_uuid.as_deref(), Some(B));
    assert!(request.reference.is_some());
    assert!(
        rows.iter().any(|row| row.action == "dispatch-result"
            && row.reference == request.reference
            && row
                .summary
                .as_deref()
                .is_some_and(|value| !value.ends_with(": ok"))),
        "the fence refusal needs a result for the original request: {rows:?}"
    );
    assert!(live(&rig, OLD));
    assert!(!live(&rig, NEW));
    assert!(!rig.dir(NEW).exists());
    assert_eq!(
        fs::read(rig.dir(OLD).join("meta")).expect("target record"),
        target_record
    );
}

#[test]
fn a_dispatcher_with_an_unavailable_audit_cannot_rename_the_target() {
    let rig = Rig::new("orn-audit");
    let caller = dispatcher(&rig);
    rig.create(OLD, B);
    let hold = ae::store::lock(
        &rig.dir(DISPATCHER).join("events.jsonl.lock"),
        Duration::ZERO,
    )
    .expect("dispatcher audit lock cannot be acquired by the child");
    let before = fs::read(rig.dir(OLD).join("meta")).expect("target meta");
    let blocked = rename(&rig, OLD, NEW, B, Some(&caller), None);
    drop(hold);
    refuses(&blocked);
    assert!(live(&rig, OLD));
    assert!(!live(&rig, NEW));
    assert!(!rig.dir(NEW).exists());
    assert_eq!(
        fs::read(rig.dir(OLD).join("meta")).expect("target meta"),
        before
    );
}
