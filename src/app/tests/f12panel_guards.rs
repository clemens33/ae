use super::*;
use crate::app::{handover, panel::Opening};

const ORCH: &str = crate::orchestrator::ORCHESTRATOR_SESSION;
const OID: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";
const REPLACEMENT: &str = "0199c0de-cccc-4890-abcd-ef0123456789";

fn kept(root: &Root) -> (Reader, handover::Handover) {
    let (mut sender, reader) = housed(root);
    sender.proved(
        loader::Proof::Proven(OID.to_owned()),
        Timestamp::now().epoch(),
    );
    sender.open_panel(false);
    let written = sender
        .handover("%1", crate::VERSION, Timestamp::now().epoch())
        .expect("sender's main session has an id");
    (reader, written)
}

fn staged(root: &Root, home: Option<&str>) -> (App, i64) {
    let (reader, carried) = kept(root);
    let mut app = app(home);
    let at = carried.at;
    app.answer(Answer::Handover {
        view: carried.view,
        at,
    });
    refold(&mut app, &reader, root, &[("api", Status::Running)]);
    assert!(matches!(app.orch.opening, Opening::Restore(..)));
    (app, at)
}

#[test]
fn a_fresh_view_expires_while_waiting_for_orchestrator_proof() {
    let root = Root::new("f12-late-proof");
    let (mut app, at) = staged(&root, Some("api"));
    app.proved(
        loader::Proof::Proven(OID.to_owned()),
        at + handover::MAX_AGE_SECS + 1,
    );
    assert!(
        app.orch.panel.is_none(),
        "the delayed proof cannot open a stale panel"
    );
}

#[test]
fn a_key_before_the_proof_cancels_the_carried_opening() {
    let root = Root::new("f12-key-before-proof");
    let (mut app, at) = staged(&root, Some("api"));
    assert!(take_keys(&mut app, vec![(Key::Escape, Instant::now())]).is_some());
    app.proved(loader::Proof::Proven(OID.to_owned()), at);
    assert!(app.orch.panel.is_none(), "an arrival cannot overtake a key");
}

#[test]
fn the_orchestrator_home_opens_full_when_its_carried_incarnation_is_gone() {
    let root = Root::new("f12-orch-home");
    let (mut app, at) = staged(&root, Some(ORCH));
    app.proved(loader::Proof::Proven(REPLACEMENT.to_owned()), at);
    let home = app.orch.panel.as_ref().expect("the usual home opens");
    assert!(home.full && app.panel_focused());
    assert_eq!(
        home.pin(),
        Some(REPLACEMENT),
        "the default opens on the new proof"
    );
}

#[test]
fn a_v2_option_does_not_open_a_legacy_handed_panel_on_a_non_row() {
    let root = Root::new("f12-option-non-row");
    let (reader, mut carried) = kept(&root);
    let mut app = app(Some("api"));
    carried.view.session = ORCH.to_owned();
    carried.view.id = OID.to_owned();
    app.answer(Answer::Handover {
        view: carried.view,
        at: carried.at,
    });
    refold(&mut app, &reader, &root, &[("api", Status::Running)]);
    app.proved(loader::Proof::Proven(OID.to_owned()), carried.at);
    assert!(
        app.orch.panel.is_none(),
        "the role holder is not an adoptable row"
    );
}
