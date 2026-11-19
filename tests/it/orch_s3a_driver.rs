//! S3a driver regressions the frozen acceptance module does not pin: the
//! native fences a bound stop's tmux mutations carry, run against a private
//! tmux server that is replaced or changed under the proof. Each mutation is
//! ae's own argv; the fixture only arranges the server and reads liveness.

#![allow(
    clippy::expect_used,
    reason = "a fixture that cannot arrange its server fails the proof"
)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ae::inventory::ServerId;
use ae::meta::Selector;
use ae::tmux::{
    SessionStamp, fenced_kill_args, fenced_view_kill_args, interpret_session_stamp,
    session_stamp_args,
};

use super::cli::OwnedScratch;
use super::phase2::{capture_tmux, run_tmux};

const UUID: &str = "11111111-2222-4333-8444-555555555555";
const OTHER: &str = "11111111-2222-4333-8444-666666666666";

/// One private tmux server at its own socket under an owned scratch root.
struct Server {
    scratch: OwnedScratch,
    sock: PathBuf,
}

impl Server {
    fn new(tag: &str) -> Self {
        let mut scratch = OwnedScratch::root("s3afence", tag);
        let sock = scratch.path().join("srv");
        scratch.add_tmux_server(sock.clone());
        Self { scratch, sock }
    }

    fn id(&self) -> ServerId {
        ServerId::Selected(Selector::Socket(self.sock.clone()))
    }

    /// Run ae-built argv, which already names the socket.
    fn run(&self, args: &[String]) -> (bool, String) {
        run_tmux(args, self.scratch.path())
    }

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.sock.display().to_string()];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        let (ok, out) = self.run(&args);
        assert!(ok, "private tmux {tail:?}: {out}");
        out
    }

    fn alive(&self, name: &str) -> bool {
        let sock = self.sock.display().to_string();
        let target = format!("={name}");
        let args = ["-S", sock.as_str(), "has-session", "-t", target.as_str()];
        self.run(&args.map(ToOwned::to_owned)).0
    }

    /// A detached session `name`, stamped with `uuid` as ae stamps one.
    fn session(&self, name: &str, uuid: &str) {
        self.tmux(&["new-session", "-d", "-s", name]);
        self.stamp_with(name, uuid);
    }

    /// A session `name` on a FRESH server at the same socket. The killed
    /// server may still be exiting, and a client that reaches it fails, so
    /// the start is retried until a new server answers.
    fn session_after_kill(&self, name: &str, uuid: &str) {
        let mut args = vec!["-S".to_owned(), self.sock.display().to_string()];
        args.extend(["new-session", "-d", "-s", name].map(ToOwned::to_owned));
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let out = capture_tmux(&args, self.scratch.path());
            if out.succeeded {
                break;
            }
            assert!(
                Instant::now() < until,
                "a fresh server never started: {out:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        self.stamp_with(name, uuid);
    }

    fn stamp_with(&self, name: &str, uuid: &str) {
        let scope = format!("={name}:");
        self.tmux(&["set-option", "-t", &scope, "@ae_session_uuid", uuid]);
    }

    /// The bound stop's own isolated read of `name`.
    fn stamp(&self, name: &str) -> SessionStamp {
        let (ok, out) = self.run(&session_stamp_args(&self.id(), name));
        interpret_session_stamp(ok, &out, name).expect("one whole stamp record")
    }

    fn id_of(&self, name: &str) -> String {
        let scope = format!("={name}:");
        self.tmux(&["display-message", "-p", "-t", &scope, "#{session_id}"])
            .trim()
            .to_owned()
    }

    /// A session `name` that claims to be a pane view through its options.
    fn claimed_view(&self, name: &str, grouped_with: Option<&str>, options: &[(&str, &str)]) {
        match grouped_with {
            Some(real) => {
                let target = format!("={real}");
                self.tmux(&["new-session", "-d", "-s", name, "-t", &target]);
            }
            None => {
                self.tmux(&["new-session", "-d", "-s", name]);
            }
        }
        let scope = format!("={name}:");
        for (option, value) in options {
            self.tmux(&["set-option", "-t", &scope, option, value]);
        }
    }
}

#[test]
fn a_fenced_kill_spares_the_session_a_replacement_server_gave_the_same_id() {
    let server = Server::new("reuse");
    server.session("target", UUID);
    let proved = server.stamp("target");
    server.tmux(&["kill-server"]);
    // The replacement carries the same uuid and, being a fresh server, the
    // same first id: only the server incarnation tells them apart.
    server.session_after_kill("other", UUID);
    let current = server.stamp("other");
    assert_eq!(
        current.session.id, proved.session.id,
        "the fixture must model a reused id"
    );
    assert_ne!(current.server, proved.server, "a replacement incarnation");
    // Every session fact is the current one; only the server pair is the old
    // proof's, so only the server check can spare the session.
    let stale = SessionStamp {
        server: proved.server.clone(),
        ..current.clone()
    };
    let _ = server.run(&fenced_kill_args(&server.id(), &stale).expect("a spelled fence"));
    assert!(
        server.alive("other"),
        "a kill fenced to the old incarnation reached the replacement's session"
    );
    let _ = server.run(&fenced_kill_args(&server.id(), &current).expect("a spelled fence"));
    assert!(
        !server.alive("other"),
        "a kill fenced to the current incarnation kills its session"
    );
}

#[test]
fn a_fenced_kill_spares_a_session_whose_stamp_changed_under_the_proof() {
    let server = Server::new("restamp");
    server.session("keeper", OTHER);
    server.session("target", UUID);
    let proved = server.stamp("target");
    server.stamp_with("target", OTHER);
    let _ = server.run(&fenced_kill_args(&server.id(), &proved).expect("a spelled fence"));
    assert!(
        server.alive("target"),
        "a kill fenced to the proved stamp reached a session with another stamp"
    );
    server.stamp_with("target", UUID);
    let _ = server.run(&fenced_kill_args(&server.id(), &proved).expect("a spelled fence"));
    assert!(!server.alive("target"), "the proved session dies");
    assert!(server.alive("keeper"), "only the proved session dies");
}

#[test]
fn a_fenced_view_kill_needs_the_server_to_see_a_view_of_the_target() {
    let server = Server::new("views");
    server.session("target", UUID);
    let proved = server.stamp("target");
    let of = proved.session.id.clone();
    // Claims a view of the target by option, but its own name is no view's.
    server.claimed_view("plain", None, &[("@ae_view_of", of.as_str())]);
    // A view's name, but a view of some other session.
    server.claimed_view(
        "_aeview-s3a-other",
        None,
        &[("@ae_view_of", "$999"), ("@ae_view_uuid", OTHER)],
    );
    // Real views of the target, by its id and by its uuid.
    server.claimed_view(
        "_aeview-s3a-byid",
        Some("target"),
        &[("@ae_view_of", of.as_str())],
    );
    server.claimed_view(
        "_aeview-s3a-byuuid",
        Some("target"),
        &[("@ae_view_of", "$999"), ("@ae_view_uuid", UUID)],
    );
    for name in [
        "plain",
        "_aeview-s3a-other",
        "_aeview-s3a-byid",
        "_aeview-s3a-byuuid",
    ] {
        let view = server.id_of(name);
        let args = fenced_view_kill_args(&server.id(), &proved, &view).expect("a spelled fence");
        let _ = server.run(&args);
    }
    assert!(server.alive("plain"), "a session with no view name died");
    assert!(
        server.alive("_aeview-s3a-other"),
        "a view of another session died"
    );
    assert!(!server.alive("_aeview-s3a-byid"), "the view by id survives");
    assert!(
        !server.alive("_aeview-s3a-byuuid"),
        "the view by uuid survives"
    );
    assert!(server.alive("target"), "a view kill reached the target");
}
