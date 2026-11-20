//! O8 identity is the conjunction of the role and the canonical overlay.
//! A role claim without that overlay refuses launch and remains ordinary.

#![allow(
    clippy::disallowed_methods,
    reason = "tests prepare and read only their private session fixture"
)]

use super::*;
use crate::inventory::ServerId;
use crate::meta::Selector;
use crate::settings_menu::{self, Control};
use std::fmt::Write as _;

const NAME: &str = crate::orchestrator::ORCHESTRATOR_SESSION;

/// A live private server makes an absent target provable. The cleanup owner
/// registers before creation and retains the root if its end is unproven.
struct Socket {
    server: ServerId,
    retained: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Socket {
    fn new(root: &Root) -> Self {
        let socket = Self {
            server: ServerId::Selected(Selector::Socket(root.0.join("s"))),
            retained: root.retention(),
        };
        let work_dir = root.0.to_str().expect("the private UTF-8 root");
        let (ok, _) = crate::transport::run_tmux_op(&crate::session_tmux::argv(
            &socket.server,
            &crate::session_tmux::Op::NewSession {
                name: "keeper",
                work_dir,
            },
        ));
        assert!(ok, "the private server starts without the target session");
        assert_eq!(
            crate::transport::verify_session_absent(&socket.server, NAME),
            crate::tmux::StopProbe::Absent,
            "the live private server proves the canonical target absent"
        );
        socket
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        use crate::transport::fixture_tmux as wire;
        let _ = wire::end(&self.server);
        let mut last = wire::probe(&self.server);
        for _ in 0..20 {
            if last.proven() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
            last = wire::probe(&self.server);
        }
        self.retained
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(
            std::thread::panicking(),
            "private socket cleanup unproven: {last:?}"
        );
    }
}

fn record(root: &Root, name: &str, role: bool, overlay: Option<&std::path::Path>) {
    let dir = root.0.join("sessions").join(name);
    std::fs::create_dir_all(&dir).expect("the private record directory");
    let mut meta = format!(
        "schema=2\nsession_id={ID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\n\
         tmux_server_kind=socket\ntmux_server={}\nmain_pane=%1\n",
        root.0.join("s").display()
    );
    if role {
        meta.push_str("meta_agent=true\n");
    }
    if let Some(overlay) = overlay {
        writeln!(meta, "local_config={}", overlay.display())
            .expect("write the private overlay fact");
    }
    std::fs::write(dir.join("meta"), meta).expect("the private identity facts");
}

fn control(root: &Root) -> Control {
    let server = ServerId::Selected(Selector::Socket(root.0.join("s")));
    settings_menu::discover(&root.0, &server, &server)
}

fn listed(root: &Root, name: &str) -> bool {
    let reader = Reader::new(root.0.clone(), None, None);
    let mut app = app(None);
    refold(
        &mut app,
        &reader,
        root,
        &[("api", Status::Stopped), (name, Status::Stopped)],
    );
    // A stopped group is normally folded. Open it before observing its rows.
    app.toggle_stopped();
    app.fleet.rows.iter().any(|row| row.name == name)
}

#[test]
fn a_role_without_the_canonical_overlay_is_unavailable_and_names_why() {
    for name in [NAME, "router"] {
        let root = Root::new("o8-wrong-overlay");
        let _socket = Socket::new(&root);
        let wrong = root.0.join("ordinary.config");
        for overlay in [None, Some(wrong.as_path())] {
            record(&root, name, true, overlay);
            let found = control(&root);
            assert!(
                matches!(&found, Control::Unavailable(why) if why.contains(name) && why.contains("overlay")),
                "a role without the seat overlay refuses launch and names the record and reason: {found:?}"
            );
        }
    }
}

#[test]
fn changing_only_the_overlay_returns_the_unproved_role_to_the_ordinary_list() {
    let root = Root::new("o8-overlay-list");
    let seat = root.0.join("orchestrator.config");
    record(&root, NAME, true, Some(&seat));
    assert!(!listed(&root, NAME), "the proved seat is hidden");
    let wrong = root.0.join("ordinary.config");
    record(&root, NAME, true, Some(&wrong));
    assert!(
        listed(&root, NAME),
        "the role claim without the seat overlay is an ordinary listed session"
    );
}

#[test]
fn the_overlay_without_the_role_keeps_its_ordinary_namesake_row() {
    let root = Root::new("o8-overlay-no-role");
    let seat = root.0.join("orchestrator.config");
    record(&root, NAME, false, Some(&seat));
    assert!(
        listed(&root, NAME),
        "the overlay alone never hides an ordinary namesake"
    );
}

/// GREEN at base: the two matching facts still select the existing owner.
#[test]
fn the_matching_role_and_overlay_keep_the_stopped_seat_resume() {
    let root = Root::new("o8-overlay-control");
    let _socket = Socket::new(&root);
    let seat = root.0.join("orchestrator.config");
    record(&root, NAME, true, Some(&seat));
    let found = control(&root);
    assert!(
        matches!(&found, Control::Resume(target) if target.name == NAME && target.uuid == ID),
        "the canonical stopped seat keeps its resume identity: {found:?}"
    );
}
