//! Exercise current human-client readers with a real, marked control client.

use ae::inventory::ServerId;
use ae::meta::Selector;
use ae::transport::{
    observe_client_place, observe_clients, observe_menu_client, observe_picker_client_session,
    observe_picker_read, observe_session_stamp, observe_stamped_clients, observe_view_census,
    untargeted_attach_choice,
};

use super::{IDLE, ORCH, PANE_WAIT, Rig, await_evidence, checked, orchestrator};
use crate::cli::{OwnedChild, tmux_control_client_with_term};

const DICTATION_TERM: &str = "ae-dictation";
const VIEWER_TERM: &str = "ae-d54-ordinary-control";

fn server(rig: &Rig) -> ServerId {
    ServerId::Selected(Selector::Socket(rig.sock.clone()))
}

fn await_attach(rig: &Rig, child: &OwnedChild, session: &str, term: &str) -> String {
    let name = format!("client-{}", child.id());
    let expected = format!("{name}|{session}|1|{term}");
    await_evidence(PANE_WAIT, || {
        let roster = checked(
            rig,
            &[
                "list-clients",
                "-F",
                "#{client_name}|#{session_name}|#{client_control_mode}|#{client_termname}",
            ],
        );
        roster.lines().any(|row| row == expected).then_some(())
    })
    .expect("the raw tmux roster admits the exact control client on this session");
    name
}

fn attach(rig: &Rig, session: &str, term: &str) -> (OwnedChild, String) {
    let child = tmux_control_client_with_term(&rig.sock, &format!("={session}"), term)
        .expect("private control client");
    let name = await_attach(rig, &child, session, term);
    (child, name)
}

#[test]
fn the_sidecar_is_absent_from_the_attention_roster() {
    let rig = orchestrator("sidecar-attention", "claude", IDLE);
    let marked = tmux_control_client_with_term(&rig.sock, "=orchestrator", DICTATION_TERM)
        .expect("private dictation client");
    let name = await_attach(&rig, &marked, ORCH, DICTATION_TERM);
    let (_viewer, viewer) = attach(&rig, ORCH, VIEWER_TERM);

    let roster = observe_clients(&server(&rig)).expect("complete private attention roster");
    assert!(
        roster
            .iter()
            .any(|row| row.name == viewer && row.pane == rig.pane),
        "ordinary control client sees the pane: {roster:?}"
    );
    assert!(
        roster.iter().all(|row| row.name != name),
        "the attached dictation client must not receive a human attention vote: {roster:?}"
    );
}

#[test]
fn the_sidecar_cannot_receive_a_bound_stop_confirmation() {
    let rig = orchestrator("sidecar-confirm", "claude", IDLE);
    let (_marked, name) = attach(&rig, ORCH, DICTATION_TERM);
    let (_viewer, viewer) = attach(&rig, ORCH, VIEWER_TERM);
    let server = server(&rig);
    let stamp = observe_session_stamp(&server, ORCH).expect("exact orchestrator stamp");
    let clients = observe_stamped_clients(&server, &stamp).expect("whole stamped client listing");
    assert!(
        clients.contains(&viewer),
        "ordinary client remains: {clients:?}"
    );
    assert!(
        !clients.contains(&name),
        "bound stop must never choose the dictation client: {clients:?}"
    );
}

#[test]
fn the_sidecar_alone_does_not_make_an_orchestrator_view_present() {
    let rig = orchestrator("sidecar-presence", "claude", IDLE);
    let (_marked, _) = attach(&rig, ORCH, DICTATION_TERM);
    checked(&rig, &["new-session", "-d", "-s", "a-viewer", "sleep 60"]);
    let (_viewer, _) = attach(&rig, "a-viewer", VIEWER_TERM);
    let census = observe_view_census(&server(&rig)).expect("whole view census");
    assert!(
        census
            .iter()
            .any(|row| row.name == "a-viewer" && row.attached),
        "a different session retains its viewer: {census:?}"
    );
    assert!(
        census.iter().any(|row| row.name == ORCH && !row.attached),
        "the dictation-only session is not viewed: {census:?}"
    );
}

#[test]
fn the_sidecar_does_not_change_which_detached_session_ae_picks() {
    let rig = orchestrator("sidecar-choice", "claude", IDLE);
    let (_marked, _) = attach(&rig, ORCH, DICTATION_TERM);
    checked(&rig, &["new-session", "-d", "-s", "a-viewer", "sleep 60"]);
    let (_viewer, _) = attach(&rig, "a-viewer", VIEWER_TERM);

    assert_eq!(
        untargeted_attach_choice(&server(&rig), "absent").as_deref(),
        Some(ORCH),
        "the orchestrator is the only session without a human viewer"
    );
}

#[test]
fn a_human_viewer_keeps_the_orchestrator_attached_beside_the_sidecar() {
    let rig = orchestrator("sidecar-viewer", "claude", IDLE);
    let (_marked, _) = attach(&rig, ORCH, DICTATION_TERM);
    checked(&rig, &["new-session", "-d", "-s", "z-detached", "sleep 60"]);
    checked(&rig, &["new-session", "-d", "-s", "a-viewer", "sleep 60"]);
    let (_other, _) = attach(&rig, "a-viewer", VIEWER_TERM);
    let (_viewer, _) = attach(&rig, ORCH, VIEWER_TERM);

    let census = observe_view_census(&server(&rig)).expect("whole view census");
    assert!(
        census.iter().any(|row| row.name == ORCH && row.attached),
        "the ordinary viewer keeps the orchestrator attached: {census:?}"
    );
    assert_eq!(
        untargeted_attach_choice(&server(&rig), "absent").as_deref(),
        Some("z-detached"),
        "only the third session has no human viewer"
    );
}

#[test]
fn the_sidecar_has_no_client_place_or_picker_look() {
    let rig = orchestrator("sidecar-look", "claude", IDLE);
    let (_marked, name) = attach(&rig, ORCH, DICTATION_TERM);
    let (_viewer, viewer) = attach(&rig, ORCH, VIEWER_TERM);
    let server = server(&rig);
    assert!(
        observe_client_place(&server, &viewer).is_some(),
        "ordinary control client retains a place"
    );
    assert!(
        observe_picker_read(&server, &viewer).is_some_and(|row| row.look.is_some()),
        "ordinary control client retains its look"
    );

    let marked_read = observe_picker_read(&server, &name).expect("whole picker read");
    assert!(
        marked_read.client.is_none(),
        "sidecar cannot invoke the picker"
    );
    assert!(observe_menu_client(&server, &name).is_none());
    assert!(observe_picker_client_session(&server, &name).is_none());
    let marked_place = observe_client_place(&server, &name);
    assert!(
        marked_place.is_none() && marked_read.look.is_none(),
        "sidecar leaked a place {marked_place:?} or a look {:?}",
        marked_read.look
    );
}
