//! The o7app incarnation pin also governs an orchestrator home: a late
//! lane for another UUID never enters the already-open seat panel.

#![allow(
    clippy::disallowed_methods,
    reason = "the test creates only its private seat record and overlay"
)]

use super::*;

const ORCH: &str = crate::orchestrator::ORCHESTRATOR_SESSION;
const OID: &str = "0199c0de-2222-4890-abcd-ef0123456789";
const NEXT: &str = "0199c0de-3333-4890-abcd-ef0123456789";
const OLD: &str = "ADMITTED_SEAT_LANE";
const NEW: &str = "UNPROVEN_REPLACEMENT_LANE";

fn seat_record(root: &Root) {
    let dir = root.0.join("sessions").join(ORCH);
    std::fs::create_dir_all(&dir).expect("private seat directory");
    let overlay = root.0.join("orchestrator.config");
    std::fs::write(&overlay, "[workspace]\norchestrator = true\n")
        .expect("private canonical seat overlay");
    let meta = format!(
        "schema=2\nsession_id={OID}\nlayout=lead-pair\nseat.main=lead\nmeta_agent=true\nlocal_config={}\n",
        overlay.display()
    );
    std::fs::write(dir.join("meta"), meta).expect("private seat metadata");
}

fn panel_text(app: &mut App) -> String {
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    app.frame(&mut buf);
    let cells = app.layout.panel.expect("the side panel is drawn");
    (cells.y..cells.y + cells.height)
        .map(|y| {
            (cells.x..cells.x + cells.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn an_orchestrator_home_never_draws_another_uuid_in_its_pinned_panel() {
    let root = Root::new("o8-home-panel-uuid");
    seat_record(&root);
    let reader = Reader::new(root.0.clone(), Some(ORCH.to_owned()), None);
    let mut app = app(Some(ORCH));
    refold(
        &mut app,
        &reader,
        &root,
        &[("api", Status::Running), (ORCH, Status::Running)],
    );
    app.answer(Answer::Orchestrator(loader::Proof::Proven(OID.to_owned())));
    app.open_panel(false);
    app.answer(view(ORCH, OID, 1, OLD));
    let before = panel_text(&mut app);
    assert_eq!(app.home.as_deref(), Some(ORCH), "the seat is home");
    assert!(
        before.contains("active") && before.contains(OLD) && app.panel_line().is_none(),
        "the proved seat and its admitted lane are drawn before the late view: {before}"
    );

    // A console answer can reach home before the fleet and proof observe
    // its replacement. The open panel remains admitted to the prior UUID.
    app.answer(view(ORCH, NEXT, 2, NEW));
    let after = panel_text(&mut app);
    assert!(
        !after.contains(NEW),
        "an open seat panel never draws another UUID before its proof: {after}"
    );
    assert!(
        app.panel_entry(&(ORCH.to_owned(), OID.to_owned())),
        "a late view never changes the panel's admitted incarnation"
    );
}

#[test]
fn a_panel_never_draws_another_uuid_cached_by_home_before_it_binds() {
    let root = Root::new("o8-cached-home-panel-uuid");
    seat_record(&root);
    let reader = Reader::new(root.0.clone(), Some(ORCH.to_owned()), None);
    let mut app = app(Some(ORCH));
    refold(
        &mut app,
        &reader,
        &root,
        &[("api", Status::Running), (ORCH, Status::Running)],
    );

    // Home can answer before the separate proof wake opens its panel.
    app.answer(view(ORCH, NEXT, 1, NEW));
    assert!(app.orch.panel.is_none(), "the proof has not opened a panel");
    assert_eq!(
        app.shown.get(ORCH).map(|shown| shown.id.as_str()),
        Some(NEXT),
        "the other incarnation is cached before the panel binds"
    );

    app.answer(Answer::Orchestrator(loader::Proof::Proven(OID.to_owned())));
    let first = panel_text(&mut app);
    assert!(
        first.contains("active") && app.panel_entry(&(ORCH.to_owned(), OID.to_owned())),
        "the proof opens a panel on its own incarnation: {first}"
    );
    assert!(
        !first.contains(NEW),
        "a newly bound seat panel never draws another UUID cached before its proof: {first}"
    );

    app.answer(view(ORCH, OID, 2, OLD));
    let current = panel_text(&mut app);
    assert!(
        current.contains("active") && current.contains(OLD),
        "the admitted incarnation appears when its own lane arrives: {current}"
    );
}
