//! The o8 identity ruling applies to chat and writing as well as list rows:
//! an ordinary session named `orchestrator` keeps every ordinary behavior.

#![allow(
    clippy::disallowed_methods,
    reason = "tests prepare and read only their private session fixture"
)]

use super::*;

const NAME: &str = crate::orchestrator::ORCHESTRATOR_SESSION;
const BODY: &str = "ORDINARY_NAMESAKE_CHAT";

/// A listed ordinary namesake with a real record and no orchestrator role.
fn ordinary(root: &Root) -> (App, Reader) {
    std::fs::rename(
        root.0.join("sessions/api"),
        root.0.join("sessions").join(NAME),
    )
    .expect("rename the private ordinary fixture directory");
    let reader = Reader::new(root.0.clone(), None, None);
    let mut app = app(None);
    let mut read = reader.fold(
        [(NAME.to_owned(), root.0.join("sessions").join(NAME))]
            .into_iter()
            .collect(),
        World::new(Timestamp::now(), vec![entry(NAME, Status::Running, None)]),
        None,
        &crate::theme::FleetOrder::EMPTY,
        Timestamp::now(),
    );
    // List identity is pinned separately. Supply the ordinary row at this
    // reducer boundary so the old name filter cannot make setup fail first.
    read.fleet = Fleet {
        rows: vec![row(NAME, 1, false)],
        home: None,
    };
    app.answer(Answer::Fleet(read));
    assert_eq!(app.model.selected(), Some(NAME), "the listed fixture");
    assert!(app.orch.panel.is_none(), "no seat panel opens by name");
    (app, reader)
}

#[test]
fn a_non_home_ordinary_namesake_loads_its_selected_chat() {
    let root = Root::new("o8-namesake-chat");
    let (mut app, _reader) = ordinary(&root);
    assert!(app.home.is_none(), "the namesake is not home");
    app.answer(view(NAME, ID, 1, BODY));
    let shown = framed(&mut app);
    assert!(
        shown.contains(BODY),
        "a selected ordinary namesake shows its read chat: {shown}"
    );
    assert!(!app.loading, "the ordinary chat read ends loading");
    assert!(app.orch.panel.is_none(), "reading opens no seat panel");
}

#[test]
fn a_stopped_ordinary_namesake_ends_writing_held_and_keeps_its_draft() {
    let root = Root::new("o8-namesake-stop");
    let (mut app, reader) = ordinary(&root);
    let at = writing(&mut app);
    let _ = app.compose(Key::Text(b"KEPT_ORDINARY_DRAFT".to_vec()), at);
    refold(&mut app, &reader, &root, &[(NAME, Status::Stopped)]);
    assert!(
        !app.composing(),
        "a stopped ordinary namesake ends its writer like any ordinary session"
    );
    assert_eq!(app.held.as_deref(), Some("orchestrator is stopped"));
    assert_eq!(drafted(&app), "KEPT_ORDINARY_DRAFT");
}

#[test]
fn a_replaced_ordinary_namesake_ends_writing_held() {
    let root = Root::new("o8-namesake-replaced");
    let (mut app, _reader) = ordinary(&root);
    writing(&mut app);
    let next = "0199c0de-3333-4890-abcd-ef0123456789";
    app.answer(Answer::Fleet(read_of(&app, &[(NAME, next)])));
    assert!(!app.composing(), "replacement ends the ordinary writer");
    assert_eq!(
        app.held.as_deref(),
        Some("the session was replaced"),
        "a replaced ordinary namesake keeps the ordinary held reason"
    );
}

/// Open the inert seat panel, then write the ordinary namesake in the main
/// column. The panel and the namesake act on different identities.
fn beside_panel(root: &Root) -> App {
    let (mut app, _reader) = ordinary(root);
    app.open_panel(false);
    app.focus_column(false);
    let at = writing(&mut app);
    let _ = app.compose(Key::Text(b"MAIN_NAMESAKE_DRAFT".to_vec()), at);
    assert_eq!(app.writer_name(), Some(NAME), "the main writer started");
    assert!(!app.panel_focused(), "the main column holds the keys");
    app
}

#[test]
fn closing_the_seat_panel_keeps_the_ordinary_namesake_writer() {
    let root = Root::new("o8-namesake-close");
    let mut app = beside_panel(&root);
    app.close_panel();
    assert!(app.orch.panel.is_none(), "the seat panel closed");
    assert!(
        app.composing(),
        "closing the seat panel leaves the ordinary namesake writer writing"
    );
    assert_eq!(drafted(&app), "MAIN_NAMESAKE_DRAFT");
}

#[test]
fn an_absent_seat_proof_keeps_the_ordinary_namesake_writer() {
    let root = Root::new("o8-namesake-proof");
    let mut app = beside_panel(&root);
    app.answer(Answer::Orchestrator(loader::Proof::Absent));
    assert!(
        app.composing(),
        "an absent seat proof leaves the ordinary namesake writer writing"
    );
    assert_eq!(drafted(&app), "MAIN_NAMESAKE_DRAFT");
}
