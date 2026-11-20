//! Outcome 1 applies to every drawable frame, including the gap between
//! the loader's Fleet answer and its following orchestrator proof.

#![allow(
    clippy::disallowed_methods,
    reason = "tests prepare only their private seat record and overlay"
)]

use super::*;

const ORCH: &str = crate::orchestrator::ORCHESTRATOR_SESSION;
const OID: &str = "0199c0de-2222-4890-abcd-ef0123456789";
const TURN: &str = "LAST_LIVE_SEAT_TURN";

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

fn panel_rows(app: &mut App) -> Vec<String> {
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    app.frame(&mut buf);
    let cells = app.layout.panel.expect("the open side panel stays drawn");
    (cells.y..cells.y + cells.height)
        .map(|y| {
            (cells.x..cells.x + cells.width)
                .map(|x| buf[(x, y)].symbol())
                .collect()
        })
        .collect()
}

#[test]
fn a_stopped_fleet_never_draws_the_seat_while_its_proof_is_still_in_flight() {
    let root = Root::new("o8-transition");
    seat_record(&root);
    let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
    let mut app = app(Some("api"));
    refold(
        &mut app,
        &reader,
        &root,
        &[("api", Status::Running), (ORCH, Status::Running)],
    );
    app.answer(Answer::Orchestrator(loader::Proof::Proven(OID.to_owned())));
    app.open_panel(false);
    app.answer(view(ORCH, OID, 1, TURN));
    let before = panel_rows(&mut app).join("\n");
    assert!(
        before.contains("active") && before.contains(TURN) && before.contains("Enter writes"),
        "the real live seat and its chat are visible before it stops: {before}"
    );

    let now = Timestamp::now();
    let read = reader.fold(
        ["api", ORCH]
            .into_iter()
            .map(|name| (name.to_owned(), root.0.join("sessions").join(name)))
            .collect(),
        World::new(
            now,
            vec![
                entry("api", Status::Running, None),
                entry(ORCH, Status::Stopped, None),
            ],
        ),
        None,
        &crate::theme::FleetOrder::EMPTY,
        now,
    );
    // The real loader sends Fleet before Orchestrator as separate wakes.
    // It can be descheduled between sends. A resize can draw at this point
    // even if drain requests no repaint. Keeping the prior live snapshot
    // until both answers land is also valid, so no reducer choice is pinned.
    let (wake, wakes) = std::sync::mpsc::channel();
    wake.send(loader::Wake::Answer(Box::new(Answer::Fleet(read))))
        .expect("the stopped fleet wake is queued");
    let mut keys = crate::console::input::Keys::app();
    assert!(
        run::drain(&mut app, &mut keys, &wakes, &mut None).is_some(),
        "the fleet wake does not close the app while its proof has not arrived"
    );
    let between = panel_rows(&mut app);
    let header = between.first().expect("the panel has a header");
    assert!(
        header.contains(ORCH) && !header.to_ascii_lowercase().contains("stopped"),
        "a stopped fleet never presents the seat as a stopped session before its proof: {header}"
    );
    let body = between.join("\n");
    if !header.contains("active") {
        assert!(
            !body.contains(TURN) && !body.contains("Enter writes") && body.contains("read-only"),
            "a status column has no old chat or writable composer before its proof: {body}"
        );
    }

    wake.send(loader::Wake::Answer(Box::new(Answer::Orchestrator(
        loader::Proof::Absent,
    ))))
    .expect("the following absent proof is queued");
    // Only the first step leaves repaint open. This following proof changes
    // the displayed status, so its drain owes a repaint.
    assert_eq!(
        run::drain(&mut app, &mut keys, &wakes, &mut None),
        Some(true)
    );
    let after = panel_rows(&mut app).join("\n");
    assert!(
        after.contains("orchestrator stopped; O starts it") && !after.contains(TURN),
        "the following proof keeps the plain recovery line and no old chat: {after}"
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the two-wake scenario keeps its live guards and both rendered transitions together"
)]
fn a_stopped_fleet_never_leaves_a_status_column_with_an_active_panel_writer() {
    const DRAFT: &str = "PRIVATE_O8_DRAFT";
    const LOST: &str = "orchestrator stopped; O starts it";

    let root = Root::new("o8-writing-transition");
    seat_record(&root);
    let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
    let mut app = app(Some("api"));
    refold(
        &mut app,
        &reader,
        &root,
        &[("api", Status::Running), (ORCH, Status::Running)],
    );
    app.answer(Answer::Orchestrator(loader::Proof::Proven(OID.to_owned())));
    app.open_panel(false);
    app.answer(view(ORCH, OID, 1, TURN));
    let live = panel_rows(&mut app).join("\n");
    assert!(
        live.contains("active") && live.contains(TURN) && live.contains("Enter writes"),
        "the live panel is drawn before its writer receives keys: {live}"
    );
    // Enter writing and type a private draft through real keys. No key sends it.
    assert!(take_keys(&mut app, vec![(Key::Enter, Instant::now())]).is_some());
    assert!(
        take_keys(
            &mut app,
            vec![(Key::Text(DRAFT.as_bytes().to_vec()), Instant::now())],
        )
        .is_some()
    );
    let before = panel_rows(&mut app).join("\n");
    assert!(
        before.contains("active")
            && before.contains(TURN)
            && before.contains(DRAFT)
            && before.contains("Enter sends")
            && app.writer_name() == Some(ORCH)
            && app.composing()
            && app.layout.cursor.is_some(),
        "the real live panel writer has an unsent draft before the seat stops: {before}"
    );

    let now = Timestamp::now();
    let read = reader.fold(
        ["api", ORCH]
            .into_iter()
            .map(|name| (name.to_owned(), root.0.join("sessions").join(name)))
            .collect(),
        World::new(
            now,
            vec![
                entry("api", Status::Running, None),
                entry(ORCH, Status::Stopped, None),
            ],
        ),
        None,
        &crate::theme::FleetOrder::EMPTY,
        now,
    );
    let (wake, wakes) = std::sync::mpsc::channel();
    wake.send(loader::Wake::Answer(Box::new(Answer::Fleet(read))))
        .expect("the stopped fleet wake is queued before its proof");
    let mut keys = crate::console::input::Keys::app();
    assert!(
        run::drain(&mut app, &mut keys, &wakes, &mut None).is_some(),
        "the stopped fleet wake leaves the app open"
    );
    let between = panel_rows(&mut app);
    let header = between.first().expect("the panel has a header");
    assert!(
        header.contains(ORCH) && !header.to_ascii_lowercase().contains("stopped"),
        "a writing transition never shows a stopped seat header: {header}"
    );
    let body = between.join("\n");
    // As in the browsing case, an atomic prior-live snapshot remains valid.
    // Once the frame becomes a status column, its composer must be read-only.
    if !header.contains("active") {
        assert!(
            body.contains(LOST)
                && body.contains("read-only")
                && !body.contains(TURN)
                && !body.contains(DRAFT)
                && !body.contains("Enter sends")
                && !body.contains("Enter writes"),
            "a stopped seat status column ends its active composer and shows recovery before the proof: {body}"
        );
        assert!(
            app.writer_name().is_none()
                && !app.composing()
                && app.held.is_none()
                && app.layout.cursor.is_none(),
            "a stopped seat status column leaves no writer, held reason or cursor"
        );
    }
    let key = (ORCH.to_owned(), OID.to_owned());
    assert_eq!(
        app.inputs.get(&key).map(Input::draft).as_deref(),
        Some(DRAFT),
        "the stopped fleet preserves the unsent seat draft"
    );

    wake.send(loader::Wake::Answer(Box::new(Answer::Orchestrator(
        loader::Proof::Absent,
    ))))
    .expect("the following absent proof is queued");
    assert_eq!(
        run::drain(&mut app, &mut keys, &wakes, &mut None),
        Some(true)
    );
    let after = panel_rows(&mut app).join("\n");
    assert!(
        after.contains(LOST)
            && after.contains("read-only")
            && !after.contains(TURN)
            && !after.contains(DRAFT)
            && !after.contains("Enter sends")
            && !after.contains("Enter writes"),
        "the absent proof leaves only recovery and a read-only composer: {after}"
    );
    assert!(
        app.writer_name().is_none()
            && !app.composing()
            && app.held.is_none()
            && app.layout.cursor.is_none(),
        "the absent proof leaves no writer, held reason or cursor"
    );
    assert_eq!(
        app.inputs.get(&key).map(Input::draft).as_deref(),
        Some(DRAFT),
        "the absent proof preserves the unsent seat draft"
    );
}
