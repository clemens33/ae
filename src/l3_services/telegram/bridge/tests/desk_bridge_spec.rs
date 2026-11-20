//! Board source selection through the real outbound pump and HTTP boundary.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the acceptance test owns its files and private tmux socket"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::inventory::ServerId;
use crate::json::{self, Value};
use crate::meta::Selector;
use crate::session_tmux::{Op, argv};
use crate::telegram::routing::World as _;
use crate::telegram::scope::{Mode, Scope};
use crate::telegram::tests::{FAKE_TOKEN, Fake, Reply};
use crate::transport::{fixture_tmux, run_tmux_op};

use super::super::{ForwardingMachine, Machine, Relay, pump};
use super::{Knobs, Paths};

const SEAT: &str = "review-seat";
const ORDINARY: &str = "review-other";
const PIN_OK: &str = r#"{"ok":true,"result":true}"#;

struct Fixture {
    root: PathBuf,
    server: ServerId,
}

impl Fixture {
    fn new() -> Self {
        let name = format!(
            "ae-desk-pump-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        );
        let root = Path::new("/tmp").join(name);
        fs::create_dir(&root).expect("private fixture state");
        let server = ServerId::Selected(Selector::Socket(root.join("s")));
        Self { root, server }
    }

    fn start(&self, name: &str) {
        let (ok, output) = run_tmux_op(&argv(
            &self.server,
            &Op::NewSession {
                name,
                work_dir: self.root.to_str().expect("UTF-8 fixture path"),
            },
        ));
        assert!(ok, "start private {name} tmux session: {output}");
    }

    fn record(&self, dir: &Path, id: &str, socket: &str, role: bool, text: &str) {
        fs::create_dir_all(dir).expect("record directory");
        let role = if role {
            format!(
                "meta_agent=true\nlocal_config={}\n",
                self.root.join("orchestrator.config").display()
            )
        } else {
            String::new()
        };
        fs::write(
            dir.join("meta"),
            format!(
                "schema=2\nsession_id={id}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\ntmux_server_kind=socket\ntmux_server={}\n{role}",
                self.root.join(socket).display()
            ),
        )
        .expect("durable metadata");
        write_board(dir, text);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fixture_tmux::end(&self.server);
        let mut last = fixture_tmux::probe(&self.server);
        for _ in 0..20 {
            if last.proven() {
                let _ = fs::remove_dir_all(&self.root);
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
            last = fixture_tmux::probe(&self.server);
        }
        assert!(
            std::thread::panicking(),
            "fixture socket cleanup unproven: {last:?}"
        );
    }
}

fn write_board(dir: &Path, text: &str) {
    fs::write(
        dir.join("desk-record"),
        format!("ae-desk 1\nupdated 2026-11-20T08:40:12Z\nD7\t{text}\n"),
    )
    .expect("fixture board");
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one pump scenario checks source changes and the resulting request order"
)]
fn the_pump_reads_only_the_running_canonical_seat_board_and_rereads_its_replacement() {
    let fixture = Fixture::new();
    let paths = Paths::under(&fixture.root);
    fs::create_dir_all(paths.state()).expect("bridge state");
    let ordinary = fixture.root.join("sessions").join(ORDINARY);
    let seat = fixture.root.join("sessions").join(SEAT);
    let nested = fixture.root.join("worktrees/topic/.ae").join(SEAT);
    let ordinary_id = "0199c0de-4444-4890-abcd-ef0123456789";
    let seat_id = "0199c0de-5555-4890-abcd-ef0123456789";
    let nested_id = "0199c0de-6666-4890-abcd-ef0123456789";
    fixture.start(ORDINARY);
    fixture.start(SEAT);
    fixture.record(&ordinary, ordinary_id, "s", false, "Ordinary board decoy");
    let fake = Fake::start(vec![Reply::ok(), Reply::json(200, PIN_OK)]);
    let api = fake.api();
    let mut err = Vec::new();
    let mut pass = |mode| {
        let relay = Relay::new(
            i64::MIN,
            paths.config.clone(),
            Scope {
                mode,
                also: Vec::new(),
            },
            crate::settings_menu::role_holder,
        );
        let knobs = Knobs {
            once: true,
            ..Knobs::default()
        };
        assert_eq!(
            pump(
                &api,
                &paths,
                knobs,
                &relay,
                None,
                &AtomicBool::new(false),
                &mut err
            )
            .expect("real outbound pass"),
            0
        );
    };

    let world = ForwardingMachine(
        Machine::under(paths.roots()).with_census(crate::settings_menu::role_holder),
    );
    assert!(
        world
            .running()
            .iter()
            .any(|session| session.dir == ordinary)
    );
    pass(Mode::All);
    assert!(
        fake.requests().is_empty(),
        "a running ordinary session is no source"
    );

    fixture.record(&seat, seat_id, "stopped", true, "Canonical board");
    fixture.record(&nested, nested_id, "s", true, "Nested namesake decoy");
    assert!(matches!(world.seat(), super::Seat::Held { dir, .. } if dir == seat));
    let running = world.running();
    assert!(
        running
            .iter()
            .any(|session| session.dir == nested && session.meta_agent)
    );
    assert!(running.iter().all(|session| session.dir != seat));
    pass(Mode::All);
    assert!(
        fake.requests().is_empty(),
        "the living namesake does not prove the canonical seat"
    );
    assert!(!paths.state().join("desk-pin").exists());

    fixture.record(&seat, seat_id, "s", true, "Canonical board");
    pass(Mode::Orchestrator);
    pass(Mode::Orchestrator);
    let requests = fake.requests();
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert_eq!(requests[0].path, format!("/bot{FAKE_TOKEN}/sendMessage"));
    let body = json::parse(&requests[0].body).expect("sent board JSON");
    let text = body.get_str("text").expect("sent board");
    assert!(text.contains("Canonical board") && !text.contains("decoy"));
    assert_eq!(requests[1].path, format!("/bot{FAKE_TOKEN}/pinChatMessage"));
    assert_eq!(
        json::parse(&requests[1].body)
            .expect("pin JSON")
            .get("message_id"),
        Some(&Value::Num(42))
    );

    write_board(&seat, "Canonical replacement");
    fake.script(vec![Reply::ok()]);
    pass(Mode::All);
    pass(Mode::Orchestrator);
    let requests = fake.requests();
    assert_eq!(requests.len(), 3, "{requests:?}");
    assert_eq!(
        requests[2].path,
        format!("/bot{FAKE_TOKEN}/editMessageText")
    );
    let body = json::parse(&requests[2].body).expect("edited board JSON");
    assert_eq!(body.get("message_id"), Some(&Value::Num(42)));
    assert!(
        body.get_str("text")
            .expect("edited board")
            .contains("Canonical replacement")
    );

    fs::write(seat.join("desk-record"), "broken record").expect("damaged board");
    pass(Mode::All);
    assert_eq!(
        fake.requests().len(),
        3,
        "an unreadable board preserves the old text"
    );
    assert!(String::from_utf8_lossy(&err).contains("the pin keeps its last text"));
}
