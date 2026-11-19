//! O8 mutation follow-up: a launch accepts only its captured validity window,
//! and raw human input takes precedence over a late seat handover.

#![allow(
    clippy::disallowed_methods,
    reason = "tests create only their private seat record and overlay"
)]

use super::*;
use crate::session_launch::{ExpectedLaunch, ExpectedState};

const ORCH: &str = crate::orchestrator::ORCHESTRATOR_SESSION;
const OID: &str = "0199c0de-2222-4890-abcd-ef0123456789";

#[test]
fn an_unattended_expectation_accepts_only_its_captured_window() {
    // The captured menu validity window is 120 seconds, inclusive at both
    // ends. Unattended Start and Resume keep that same deadline contract.
    const DEADLINE: i64 = 1_789_109_780;
    for state in [
        ExpectedState::AbsentCanonical,
        ExpectedState::StoppedRole {
            uuid: OID.to_owned(),
        },
    ] {
        let expected = ExpectedLaunch::unattended(state, DEADLINE);
        for now in [DEADLINE - 120, DEADLINE - 60, DEADLINE] {
            assert_eq!(
                expected.check_action(now),
                Ok(()),
                "an unattended request accepts every instant in its captured window: {now}"
            );
        }
        let future = expected.check_action(DEADLINE - 121);
        assert!(
            future.is_err_and(|why| why.contains("stamped in the future")),
            "a future-stamped unattended request is refused"
        );
        let expired = expected.check_action(DEADLINE + 1);
        assert!(
            expired.is_err_and(|why| why.contains("expired")),
            "an expired unattended request is refused"
        );
    }
}

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

fn handed_seat() -> Answer {
    Answer::Handover {
        view: crate::app::handover::View {
            session: ORCH.to_owned(),
            id: OID.to_owned(),
            focus: None,
            sidebar: None,
            list: None,
            pages: 0,
            rows: 0,
            list_top: None,
            body_top: 0,
            panel: None,
            orchestrator: None,
        },
        at: Timestamp::now().epoch(),
    }
}

#[test]
fn raw_human_input_prevents_a_late_seat_handover_from_opening_the_panel() {
    for touched in [false, true] {
        let root = Root::new("o8-late-seat-handover");
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
        let before = framed(&mut app);
        assert!(
            app.model.selected() == Some("api") && app.layout.panel.is_none(),
            "the ordinary main session is drawn before any handover: {before}"
        );

        let (wake, wakes) = std::sync::mpsc::channel();
        if touched {
            // A raw Esc need not decode to a key to end handover authority.
            // src/app/run.rs:443 marks touched before decoding those bytes.
            wake.send(loader::Wake::Keys(Instant::now(), b"\x1b".to_vec()))
                .expect("the raw human input precedes the late handover");
        }
        wake.send(loader::Wake::Answer(Box::new(handed_seat())))
            .expect("the seat handover follows any human input");
        wake.send(loader::Wake::Answer(Box::new(Answer::Orchestrator(
            loader::Proof::Proven(OID.to_owned()),
        ))))
        .expect("the next matching proof can settle the handover");
        let mut keys = crate::console::input::Keys::app();
        assert!(run::drain(&mut app, &mut keys, &wakes, &mut None).is_some());
        assert_eq!(app.touched, touched, "the raw input control was consumed");
        let after = framed(&mut app);
        if touched {
            assert!(
                app.model.selected() == Some("api")
                    && app.layout.panel.is_none()
                    && !app.panel_focused(),
                "raw human input prevents a late seat handover from opening its panel: {after}"
            );
        } else {
            assert!(
                app.panel_focused()
                    && app.layout.panel.is_some()
                    && after.lines().next().is_some_and(|line| line.contains(ORCH)),
                "the untouched control admits and draws the same seat handover: {after}"
            );
        }
    }
}
