//! S1a driver regressions the frozen acceptance module does not pin. Real
//! binary and private tmux through the same rig as `orch_s1a_spec`; the stored
//! relay copy is edited directly only to model a damaged or transferred record.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private files, panes and process invocations"
)]

use ae::telegram::{
    Echoes,
    bridge::SharedChat,
    inbound::{Origin, Submit, Submitted},
    routing::Target,
};
use std::fs;
use std::path::{Path, PathBuf};

use super::o2guard_p2_spec::uuid_of;
use super::relay_chat_spec::{Out, Rig, events, receipt, words};

const OWNER: &str = "I approve this exact fixture action for this session.";
const TASK: &str = "Take the fixture action.";

fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("fixture file")
}

fn append(dir: &Path, add: &str) {
    let path = dir.join("meta");
    fs::write(&path, format!("{}{add}", read(&path))).expect("fixture meta");
}

/// A dispatcher rig as the acceptance module builds it: the real overlay, the
/// recorded main pane, and the target's current context stamp.
fn rig(tag: &str) -> Rig {
    let r = Rig::new(tag);
    let overlay = r.world.root.join("orchestrator.config");
    fs::write(
        &overlay,
        include_str!("../../contrib/aeorchestrator/orchestrator.config"),
    )
    .expect("real dispatcher overlay");
    append(
        &r.caller.dir,
        &format!(
            "main_pane={}\nlocal_config={}\n",
            r.caller.pane,
            overlay.display()
        ),
    );
    append(&r.owner.tool.dir, "context.main=1\n");
    r
}

/// The owner's Telegram ask in the dispatcher's own ledger, through the real
/// shared submission.
fn owner_ask(r: &Rig) -> String {
    let chat = SharedChat {
        root: r.world.root.clone(),
        echoes: Echoes::default(),
    };
    let target = Target {
        session: "orch".to_owned(),
        dir: r.caller.dir.clone(),
        session_id: uuid_of("orch"),
        pair: vec!["boss".to_owned()],
        seat: Some("boss".to_owned()),
    };
    let origin = Origin {
        update_id: 91,
        date: 1_795_000_000,
        from_id: "42".to_owned(),
    };
    let result = chat.submit(&target, OWNER, &origin);
    assert!(
        matches!(
            &result,
            Submitted::Sent { .. } | Submitted::Unconfirmed { .. }
        ),
        "setup: real Telegram shared submission: {result:?}"
    );
    origin.request_id()
}

fn verify(r: &Rig, id: &str) -> Out {
    r.world.run(
        Some(&r.owner.tool.pane),
        &[&format!("@{}", r.owner.home), "requests", "--verify", id],
        &[],
    )
}

#[test]
fn a_bound_relay_whose_stored_outer_envelope_is_not_the_dispatchers_does_not_verify() {
    let r = rig("s1adrvenv");
    let input = owner_ask(&r);
    let out = r.run(&[
        r.owner.home.as_str(),
        "--owner-input",
        &input,
        "--task",
        TASK,
    ]);
    assert_eq!(out.0, Some(0), "relay delivery: {}", words(&out));
    let id = out.1.lines().next().expect("receipt id").to_owned();
    receipt(&out, &id, "sent", "pending");
    let opening = events(&r.owner.tool.dir)
        .into_iter()
        .find(|event| event.reference.as_deref() == Some(id.as_str()) && event.action == "ask")
        .expect("target-ledger opening");
    let path = PathBuf::from(opening.body_file.as_ref().expect("stored relay"));
    let stored = read(&path);

    let proved = verify(&r, &id);
    assert_eq!(
        proved.0,
        Some(0),
        "the delivered frame verifies: {}",
        words(&proved)
    );

    // The same inner proof under the chat's own envelope, as a recovery copy
    // of an older core would carry it: both sender markers, and the REQUEST
    // delimiter the legacy parser needs (colon and space, not colon and LF).
    // The display reads it whole; verify must not.
    let modern = format!("⟦ae:msg from ae/orchestrator⟧\nREQUEST {id} from ae/orchestrator:\n");
    let chat_head = format!("⟦ae:msg from human:chat⟧\nREQUEST {id} from human:chat: ");
    let legacy = stored.replacen(&modern, &chat_head, 1);
    assert_ne!(legacy, stored, "the fixture must change the outer envelope");
    let inner = ae::tracked::uncompose_relay(&stored, &id).expect("the delivered relay frame");
    assert_eq!(
        ae::tracked::uncompose_ask(&legacy, &id),
        Some(inner),
        "the legacy decoder returns the identical inner bytes"
    );
    assert!(
        ae::tracked::uncompose_relay(&legacy, &id).is_none(),
        "the relay decoder refuses the chat's envelope"
    );
    fs::write(&path, &legacy).expect("legacy outer envelope");
    let refused = verify(&r, &id);
    assert_eq!(refused.0, Some(1), "{}", words(&refused));
    assert_eq!(
        refused.1.lines().next(),
        Some("not owner ingress"),
        "{}",
        words(&refused)
    );
    // The owner's quoted words are in the stored body only, never in the
    // one-line summary the chat falls back to: reading them is reading it whole.
    assert!(
        r.chat(&r.owner.home).contains(OWNER),
        "the chat reads the whole legacy copy"
    );

    fs::write(&path, stored).expect("restore");
    assert_eq!(verify(&r, &id).0, Some(0));
}

fn snapshot(dir: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, into: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).expect("fixture directory") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, into);
            } else if let Ok(bytes) = fs::read(&path) {
                into.insert(path, bytes);
            }
        }
    }
    let mut files = std::collections::BTreeMap::new();
    walk(dir, &mut files);
    files
}

/// The served underscore entry of every launch form is a lower writer door:
/// the dispatcher's pane is refused there too, and nothing under the state
/// root or in tmux changes.
#[test]
fn the_served_launch_entry_is_refused_to_the_dispatcher_before_any_effect() {
    let r = rig("s1alaunchentry");
    let sessions = r.world.root.join("sessions");
    for argv in [
        vec![ae::cli::LAUNCH, "--no-attach"],
        vec![ae::cli::LAUNCH, "fixture-new", "--local", "--no-attach"],
    ] {
        let before = snapshot(&sessions);
        let out = r.world.run(Some(&r.caller.pane), &argv, &[]);
        let said = words(&out);
        assert_eq!(out.0, Some(2), "{argv:?}: {said}");
        assert!(
            said.contains("launch REFUSED") && said.contains("orchestrator"),
            "{argv:?}: {said}"
        );
        assert_eq!(
            snapshot(&sessions),
            before,
            "{argv:?}: changed session files"
        );
        assert!(
            !r.world.tmux("has-session -t", &["fixture-new"]).0,
            "{argv:?}: created a session"
        );
    }
}
