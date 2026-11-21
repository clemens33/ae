//! Frozen acceptance for f12panel: an `ae app` restart into another core
//! returns the human to the view they left, the orchestrator panel included.
//!
//! Oracle: the human's report (2026-11-19) "when i am in the pane of the
//! orchestrator and go back with f12 - the orchestrator side panel or full is
//! closed - we should go back to the same basically", the lead's rulings in
//! .local/brief-f12panel.md (same panel, same side/full mode, same focus,
//! scroll where cheap; a restart with no panel open keeps today's closed wire
//! byte for byte) and the existing contracts of `app::handover` (hostile,
//! canonical, bounded, fresh for `MAX_AGE_SECS`, never inherited by another
//! incarnation). Never derived from the fix.
//!
//! The writer is the real `run::step`; only the terminal and the exec are
//! recorded. The exec receiver is mirrored as `run.rs` builds it today: the
//! carrier passes `upgrade::carried` and lands in `App.keep` before the first
//! fleet read. The option receiver is `Answer::Handover`. Both must restore.
//! No test names a field, a token or a type the fix introduces.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the spec writes only its private state root and records the exec"
)]

use super::*;
use crate::app::handover::{self, CAP};
use crate::app::upgrade::{self, Carried, Upgrade, Via};
use crate::app::{Keep, Outside, Step, maintainer, step};
use std::collections::BTreeSet;
use std::io;

const ORCH: &str = crate::orchestrator::ORCHESTRATOR_SESSION;
const OID: &str = "0199c0de-2222-4890-abcd-ef0123456789";
const NEW: &str = "0199c0de-3333-4890-abcd-ef0123456789";
/// A core newer than any this crate is: `exec_argv` hands a view only to a
/// newer core.
const NEXT: &str = "2099.1.1";
const TURNS: usize = 200;

/// The panel the human left open, or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Left {
    Closed,
    /// Beside the main column, holding the keys.
    Side,
    /// Beside the main column, the main column holding the keys.
    SideMain,
    /// Over the main column.
    Full,
}

/// The exec a due restart attempts, recorded and refused.
struct Recorded(Vec<String>);
impl Outside for Recorded {
    fn suspend(&mut self) {}
    fn exec(&mut self, argv: &[String]) -> io::Error {
        self.0 = argv.to_vec();
        io::Error::other("f12panel spec records the exec")
    }
    fn resume(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn clear(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The orchestrator's seat record, incarnation `OID`.
fn seat_record(root: &Root) {
    let dir = root.0.join("sessions").join(ORCH);
    std::fs::create_dir_all(&dir).expect("private seat directory");
    let overlay = root.0.join("orchestrator.config");
    std::fs::write(&overlay, "[workspace]\norchestrator = true\n").expect("seat overlay");
    let meta = format!(
        "schema=2\nsession_id={OID}\nlayout=lead-pair\nseat.main=lead\nmeta_agent=true\nlocal_config={}\n",
        overlay.display()
    );
    std::fs::write(dir.join("meta"), meta).expect("seat metadata");
}

/// The orchestrator's lane of `TURNS` one-line turns, read as `OID`.
fn lane() -> Answer {
    let items = (0..TURNS)
        .map(|n| {
            said(
                &format!("F12-TURN-{n:03}"),
                i64::try_from(n).expect("small"),
            )
        })
        .collect();
    Answer::View(ViewRead {
        name: ORCH.to_owned(),
        id: OID.to_owned(),
        seq: 1,
        lane: Lane {
            items,
            coverage: Vec::new(),
        },
        needs: None,
        desk: None,
        roster: None,
    })
}

/// A fresh app homed on `api`, before any read.
fn fresh() -> App {
    app(Some("api"))
}

/// The first fleet read of `root`: `api` and the orchestrator, both running.
fn first_read(app: &mut App, root: &Root) {
    let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
    refold(
        app,
        &reader,
        root,
        &[("api", Status::Running), (ORCH, Status::Running)],
    );
}

/// An app homed on `api` that left the panel as `left`, its chat scrolled
/// back from the newest turn.
fn sender(root: &Root, left: Left) -> App {
    seat_record(root);
    let mut app = fresh();
    first_read(&mut app, root);
    if left == Left::Closed {
        return app;
    }
    app.answer(Answer::Orchestrator(loader::Proof::Proven(OID.to_owned())));
    app.open_panel(left == Left::Full);
    if left == Left::SideMain {
        app.focus_column(false);
    }
    app.answer(lane());
    let _ = framed(&mut app);
    app.in_panel(|app| app.model.reveal(0, 30, 10));
    app
}

/// The text of every row of the panel the last frame drew.
fn panel_rows(app: &mut App) -> Vec<String> {
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    app.frame(&mut buf);
    let Some(cells) = app.layout.panel else {
        return Vec::new();
    };
    (cells.y..cells.y + cells.height)
        .map(|y| {
            (cells.x..cells.x + cells.width)
                .map(|x| buf[(x, y)].symbol())
                .collect()
        })
        .collect()
}

/// The orchestrator turns the panel shows.
fn turns(rows: &[String]) -> BTreeSet<String> {
    let joined = rows.join("\n");
    joined
        .match_indices("F12-TURN-")
        .filter_map(|(at, _)| joined.get(at..at + 12))
        .map(str::to_owned)
        .collect()
}

/// Run the real restart decision on `app` into [`NEXT`]: the carrier the
/// recorded exec hands on, and what the exec receiver's own gate makes of it.
fn restart(app: &mut App, root: &Root) -> (String, Keep) {
    app.answer(Answer::Core(Some(Upgrade {
        version: NEXT.to_owned(),
        restart: Ok(Via::Exec {
            link: "/home/me/.local/bin/ae".into(),
        }),
    })));
    let (tx, _rx) = std::sync::mpsc::channel();
    let mut upkeep = maintainer::Upkeep::start(root.0.clone(), tx);
    let mut outside = Recorded(Vec::new());
    let mut err = Vec::new();
    let result = step(app, &mut outside, &mut upkeep, &[], &mut err).expect("restart step");
    upkeep.stop(maintainer::STOP_BOUND);
    assert!(
        matches!(result, Step::Stay(true)),
        "the recorded exec fails and the app stays"
    );
    let flag = (outside.0.iter())
        .position(|arg| arg == handover::FLAG)
        .expect("the due restart hands the view on");
    let text = outside.0.get(flag + 1).expect("the carrier text").clone();
    assert_eq!(
        flag + 2,
        outside.0.len(),
        "the carrier is the last argument"
    );
    let now = Timestamp::now().epoch();
    let Carried::View(view, at) = upgrade::carried(&text, std::process::id(), NEXT, now) else {
        panic!("the exec receiver must accept the carrier: {text}");
    };
    (text, Keep { view, at })
}

/// How the view reaches the app that replaces the sender.
#[derive(Debug, Clone, Copy)]
enum Carrier {
    /// `ae app --app-view`: `run.rs` sets `App.keep` before the first read.
    Exec,
    /// The `@ae_app_view` option of a respawned chat window.
    Option,
}

/// The app that replaces the sender, after the loader's first fleet read and
/// its orchestrator proof `proof`, one frame drawn while the panel's lane is
/// still loading.
fn receiver(root: &Root, keep: Keep, carrier: Carrier, proof: loader::Proof) -> App {
    let mut app = fresh();
    match carrier {
        Carrier::Exec => app.keep = Some(keep),
        Carrier::Option => app.answer(Answer::Handover {
            view: keep.view,
            at: keep.at,
        }),
    }
    first_read(&mut app, root);
    app.answer(Answer::Orchestrator(proof));
    let _ = framed(&mut app);
    app
}

/// The mode control the panel header offers: the other mode.
fn offered(rows: &[String]) -> &'static str {
    let header = rows.first().map(String::as_str).unwrap_or_default();
    match (header.contains("F full"), header.contains("F side")) {
        (true, false) => "side",
        (false, true) => "full",
        _ => panic!("the panel header shows exactly one mode control: {header:?}"),
    }
}

/// The human's report: after the restart the same panel stands open on the
/// same incarnation, in the same mode, with the same column holding the
/// keys and the same orchestrator turns on screen, through either carrier.
#[test]
fn a_restart_returns_to_the_panel_mode_focus_and_scroll_it_left() {
    for left in [Left::Side, Left::SideMain, Left::Full] {
        for carrier in [Carrier::Exec, Carrier::Option] {
            let root = Root::new("f12panel-return");
            let mut before = sender(&root, left);
            let shown = panel_rows(&mut before);
            let seen = turns(&shown);
            assert!(
                !seen.is_empty() && !seen.contains(&format!("F12-TURN-{:03}", TURNS - 1)),
                "{left:?}: the sender's panel is scrolled back from the newest turn: {seen:?}"
            );
            let focused = before.panel_focused();
            let (text, keep) = restart(&mut before, &root);

            let mut after = receiver(&root, keep, carrier, loader::Proof::Proven(OID.to_owned()));
            let what = format!("{left:?} via {carrier:?}, carrier {text}");
            let panel = after.orch.panel.as_ref();
            assert!(panel.is_some(), "{what}: the panel is open again");
            assert_eq!(
                panel.and_then(|panel| panel.pin()),
                Some(OID),
                "{what}: bound to the incarnation it showed"
            );
            assert_eq!(
                panel.map(|panel| panel.full),
                Some(left == Left::Full),
                "{what}: the same mode"
            );
            assert_eq!(after.panel_focused(), focused, "{what}: the same focus");
            assert_eq!(
                after.model.selected(),
                Some("api"),
                "{what}: same selection"
            );

            after.answer(lane());
            let rows = panel_rows(&mut after);
            assert_eq!(
                offered(&rows),
                offered(&shown),
                "{what}: the header offers the same mode switch"
            );
            assert_eq!(
                turns(&rows),
                seen,
                "{what}: the same orchestrator turns on screen"
            );
        }
    }
}

/// The closed control: a restart with no panel open hands on today's line,
/// byte for byte, and the app it starts opens no panel on the proof.
#[test]
fn a_restart_with_the_panel_closed_hands_on_todays_line_and_opens_none() {
    for carrier in [Carrier::Exec, Carrier::Option] {
        let root = Root::new("f12panel-closed");
        let mut before = sender(&root, Left::Closed);
        let (text, keep) = restart(&mut before, &root);
        let pid = std::process::id();
        let at = keep.at;
        assert_eq!(
            text,
            format!("v1|%{pid}|{NEXT}|{at}|api|{ID}|agents||||0|0||0"),
            "the closed wire is today's"
        );
        let after = receiver(&root, keep, carrier, loader::Proof::Proven(OID.to_owned()));
        assert!(after.orch.panel.is_none(), "{carrier:?}: nothing opens");
        assert_eq!(after.model.selected(), Some("api"));
    }
}

/// The closed wire stays byte-identical for an older core's lines, with and
/// without the dragged panel width, and a panel opened and closed again
/// before the restart leaves nothing behind.
#[test]
fn a_closed_panel_keeps_the_v1_line_byte_for_byte() {
    let legacy = format!(
        "v1|%1|{}|100|web|{ID}|agents|lead|31|18|0|9||0",
        crate::VERSION
    );
    for text in [legacy.clone(), format!("{legacy}|56")] {
        let carried = handover::parse(&text).expect("an older core's line parses");
        for (opened, full) in [(false, false), (true, false), (true, true)] {
            let mut app = app(None);
            two(&mut app);
            app.model.adopt(&app.fleet, &carried.view);
            if opened {
                app.orch.proof = loader::Proof::Proven(OID.to_owned());
                app.open_panel(full);
                app.close_panel();
                assert!(app.orch.panel.is_none());
            }
            let written = app.handover("%1", crate::VERSION, 100).expect("web view");
            assert_eq!(
                written.render(),
                text,
                "opened {opened} full {full}: the closed wire is byte-identical"
            );
        }
    }
}

/// The carried panel is preference, never authority: another incarnation,
/// no orchestrator, or a carrier past its age opens nothing.
#[test]
fn a_carried_panel_opens_only_on_its_incarnation_while_fresh() {
    let proofs = [
        (loader::Proof::Proven(NEW.to_owned()), "replaced"),
        (loader::Proof::Absent, "absent"),
    ];
    for carrier in [Carrier::Exec, Carrier::Option] {
        for (proof, why) in proofs.clone() {
            let root = Root::new("f12panel-incarnation");
            let mut before = sender(&root, Left::Full);
            let (_, keep) = restart(&mut before, &root);
            let after = receiver(&root, keep, carrier, proof);
            assert!(
                after.orch.panel.is_none(),
                "{carrier:?}: an orchestrator {why} opens no panel"
            );
        }
        let root = Root::new("f12panel-stale");
        let mut before = sender(&root, Left::Side);
        let (_, mut keep) = restart(&mut before, &root);
        keep.at -= handover::MAX_AGE_SECS + 1;
        let after = receiver(&root, keep, carrier, loader::Proof::Proven(OID.to_owned()));
        assert!(
            after.orch.panel.is_none(),
            "{carrier:?}: a carrier past its age opens no panel"
        );
    }
}

/// The restore happens once: the loader proves the orchestrator every
/// cycle, and a panel the human closed after the restart stays closed.
#[test]
fn a_restored_panel_closed_by_the_human_stays_closed() {
    for carrier in [Carrier::Exec, Carrier::Option] {
        let root = Root::new("f12panel-once");
        let mut before = sender(&root, Left::Side);
        let (_, keep) = restart(&mut before, &root);
        let mut after = receiver(&root, keep, carrier, loader::Proof::Proven(OID.to_owned()));
        assert!(after.orch.panel.is_some(), "{carrier:?}: restored first");
        after.close_panel();
        after.answer(Answer::Orchestrator(loader::Proof::Proven(OID.to_owned())));
        first_read(&mut after, &root);
        after.answer(Answer::Orchestrator(loader::Proof::Proven(OID.to_owned())));
        assert!(
            after.orch.panel.is_none(),
            "{carrier:?}: a later proof never reopens it"
        );
    }
}

/// With no session selected the handover carries nothing, panel or not: the
/// selection fields never stand for the orchestrator.
#[test]
fn no_selection_hands_on_no_view_even_with_the_panel_open() {
    let mut app = app(None);
    app.orch.proof = loader::Proof::Proven(OID.to_owned());
    app.open_panel(true);
    assert!(app.model.selected().is_none());
    assert!(app.handover("%1", crate::VERSION, 100).is_none());
}

/// The carrier with a panel is hostile text like every other: canonical,
/// within [`CAP`] at every bound, and refused when any field is replaced by
/// text no grammar admits or a field is appended.
#[test]
fn a_carrier_with_a_panel_is_canonical_bounded_and_refuses_garbage() {
    let name = "a".repeat(128);
    let seat = "s".repeat(64);
    let version = "9".repeat(32);
    let mut app = app(None);
    app.fleet = Fleet {
        rows: vec![row(&name, 1, false)],
        home: None,
    };
    app.dirs.insert(name.clone(), PathBuf::from("long"));
    app.ids.insert(name.clone(), ID.to_owned());
    app.model = Model::new(&app.fleet);
    let widest = format!(
        "v1|%9999999999|{version}|99999999999|{name}|{ID}|agents|{seat}|65535|65535|1000000|65535|65535|1000000|65535"
    );
    let carried = handover::parse(&widest).expect("the widest older line parses");
    app.model.adopt(&app.fleet, &carried.view);
    app.orch.proof = loader::Proof::Proven(OID.to_owned());
    app.open_panel(false);
    app.focus_column(false);
    app.in_panel(|app| app.model.reveal(0, usize::MAX, 65_536));
    let text = app
        .handover("%9999999999", &version, 99_999_999_999)
        .expect("the long row's view")
        .render();
    assert!(
        text.len() <= CAP,
        "{} bytes over the cap: {text}",
        text.len()
    );
    let read = handover::parse(&text).expect("the widest carrier parses");
    assert_eq!(read.render(), text, "the widest carrier is canonical");

    let root = Root::new("f12panel-hostile");
    for left in [Left::Side, Left::SideMain, Left::Full] {
        let mut before = sender(&root, left);
        let (line, _) = restart(&mut before, &root);
        for text in [line.as_str(), text.as_str()] {
            let parsed = handover::parse(text).expect("the carrier parses");
            assert_eq!(parsed.render(), text, "{left:?}: canonical");
            assert!(
                handover::parse(&format!("{text}|x")).is_none(),
                "{left:?}: |x"
            );
            let fields: Vec<&str> = text.split('|').collect();
            for at in 0..fields.len() {
                for junk in ["x y", "-1", "\u{1b}[2J"] {
                    let mut bad = fields.clone();
                    bad[at] = junk;
                    let bad = bad.join("|");
                    assert!(
                        handover::parse(&bad).is_none(),
                        "{left:?}: field {at} as {junk:?} must be refused: {bad}"
                    );
                }
            }
        }
    }
}

#[path = "appzoom_spec.rs"]
mod appzoom_spec;

#[path = "appzoom_drive_spec.rs"]
mod appzoom_drive_spec;

#[path = "panellinks_spec.rs"]
mod panellinks_spec;
