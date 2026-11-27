//! A caller's live dispatcher proof, and only that proof, permits cross-session
//! delivery without an explicit cross-session flag.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the acceptance fixture must establish and read its tmux state"
)]

use super::o2guard_p2_spec::{received, uuid_of};
use super::relay_chat_spec::{Rig, events, words};
use std::fs;

const TEMPLATE: &str = include_str!("../../contrib/aeorchestrator/orchestrator.config");
/// The charter sentences of versions 2 to 5: an overlay without them carries
/// the bound owner words and nothing newer.
const CHARTER_MARKERS: [&str; 4] = [
    "You are the owner's chief of staff across the ae fleet",
    "never end a session whose seats hold or wait for a build or test lane claim",
    "queues a move of this seat to another model or account of the same tool",
    "Begin every reply about one topic with that topic's `#<slug>`",
];

#[test]
fn a_charter_1_dispatcher_overlay_with_the_owner_words_can_still_relay() {
    let rig = Rig::dispatcher("execlegacy");
    let config = rig.world.root.join("orchestrator.config");
    // Synthetic: the current template without every later charter sentence.
    let older = CHARTER_MARKERS
        .iter()
        .fold(TEMPLATE.to_owned(), |text, marker| {
            assert!(text.contains(marker), "the template carries {marker:?}");
            text.replace(marker, "an older sentence")
        });
    fs::write(&config, &older).expect("a charter 1 overlay");
    let state = rig.world.run(
        Some(&rig.caller.pane),
        &[
            "@orch",
            "state",
            "working",
            "The old dispatcher is working while the new core serves its relay.",
        ],
        &[],
    );
    assert_eq!(state.0, Some(0), "{}", words(&state));
    let id = rig.ask("The unchanged owner quote can still enter the target chat.");
    assert!(
        rig.asks()
            .iter()
            .any(|row| row.reference.as_deref() == Some(id.as_str()))
    );
    assert_eq!(
        fs::read_to_string(config).expect("legacy overlay remains readable"),
        older
    );
}

#[test]
fn cross_session_short_helpers_use_the_proved_dispatcher_as_sender() {
    let rig = Rig::dispatcher("execdeliver");
    let regular = rig.world.session("regular", "ordinary", "");
    let address = format!("{}:lead", rig.owner.home);
    let id = rig.owner.tmux(&[
        "display-message",
        "-p",
        "-t",
        &rig.owner.home,
        "#{@ae_session_uuid}",
    ]);
    let id = id.trim();
    assert_eq!(id.len(), 36, "private target is UUID-stamped");

    for verb in ["send", "ask", "review", "interrupt"] {
        let body = format!("dispatcher's cross-session {verb} fixture body");
        let delivered = rig.world.run(
            Some(&rig.caller.pane),
            &["@orch", verb, &address, &body],
            &[],
        );
        assert_eq!(delivered.0, Some(0), "{verb}: {}", words(&delivered));
        assert!(
            rig.sent().contains(&body),
            "{verb} did not reach the target pane"
        );
        assert_audit(&rig, verb, &address, id);
        let ordinary_body = format!("ordinary cross-session {verb} fixture body");
        let refused = rig.world.run(
            Some(&regular.pane),
            &["@regular", verb, &address, &ordinary_body],
            &[],
        );
        assert_ne!(refused.0, Some(0), "{verb}: {}", words(&refused));
        assert!(!rig.sent().contains(&ordinary_body));
    }

    let trusted = rig.world.run(
        Some(&regular.pane),
        &[
            "@regular",
            "send",
            "--cross-session",
            &address,
            "ordinary explicit cross-session send",
        ],
        &[],
    );
    assert_eq!(trusted.0, Some(0), "{}", words(&trusted));
    assert!(rig.sent().contains("ordinary explicit cross-session send"));

    for verb in ["ask", "review"] {
        let opening = events(&rig.owner.tool.dir)
            .into_iter()
            .find(|row| row.action == verb && row.target_session_id.value() == Some(id))
            .expect("target request names the target UUID");
        assert_eq!(opening.actor, "ae/orchestrator");
        assert_eq!(
            opening.actor_session_id.value(),
            Some(uuid_of("orch").as_str())
        );
        if verb == "ask" {
            let scope = format!("@{}", rig.owner.home);
            let ask = opening.reference.as_deref().expect("ask id");
            let answer = "The target answered the dispatcher using its tracked ask id.";
            let reply = rig.world.run(
                Some(&rig.owner.tool.pane),
                &[&scope, "reply", ask, answer],
                &[],
            );
            assert_eq!(reply.0, Some(0), "{}", words(&reply));
            assert!(received(&rig.caller).contains(answer));
        }
    }
}

fn assert_audit(rig: &Rig, verb: &str, address: &str, uuid: &str) {
    let rows = events(&rig.caller.dir);
    let summary = format!("{verb} {address}");
    let named = |row: &ae::events::Event| {
        row.actor == "ae/orchestrator"
            && row.target.as_deref() == Some(address)
            && row.target_session.value() == Some(rig.owner.home.as_str())
            && row.target_session_id.value() == Some(uuid)
            && row.target_session_uuid.as_deref() == Some(uuid)
            && row.owner_input.is_none()
    };
    let request = rows
        .iter()
        .find(|row| {
            row.action == "dispatch-request"
                && named(row)
                && row.summary.as_deref() == Some(summary.as_str())
        })
        .expect("helper request audit");
    assert!(request.reference.is_some());
    assert!(
        rows.iter().any(|row| row.action == "dispatch-result"
            && named(row)
            && row.reference == request.reference
            && row.summary.as_deref() == Some(format!("{summary}: ok").as_str())),
        "helper result audit for {summary}"
    );
}

#[test]
fn a_dispatcher_role_without_its_unique_launch_id_has_no_implicit_cross_session_power() {
    let rig = Rig::dispatcher("execnoprf");
    let target = rig.world.session("ordinary", "agent", "");
    let path = rig.caller.dir.join("meta");
    let complete = fs::read_to_string(&path).expect("proof-bearing dispatcher record");
    let missing = complete.replace("launch_id.main=tok-orch\n", "");
    assert_ne!(missing, complete, "fixture removed the sole launch proof");
    fs::write(&path, missing).expect("role and pane stamps remain");
    let send = |body| {
        rig.world.run(
            Some(&rig.caller.pane),
            &["@orch", "send", "ordinary:agent", body],
            &[],
        )
    };
    let refused = send("missing launch id must not deliver");
    assert_ne!(refused.0, Some(0), "{}", words(&refused));
    assert!(received(&target).is_empty());

    fs::write(path, complete).expect("restore the sole launch proof");
    let delivered = send("restored dispatcher proof delivers");
    assert_eq!(delivered.0, Some(0), "{}", words(&delivered));
    assert!(received(&target).contains("restored dispatcher proof delivers"));
}
