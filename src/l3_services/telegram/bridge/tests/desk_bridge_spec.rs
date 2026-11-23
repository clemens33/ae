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
    pass(Mode::Orchestrator);
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
    pass(Mode::Orchestrator);
    assert_eq!(
        fake.requests().len(),
        3,
        "an unreadable board preserves the old text"
    );
    assert!(String::from_utf8_lossy(&err).contains("the pin keeps its last text"));
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one pinned message tests all three scope cases in order"
)]
fn a_pinned_board_exports_only_live_rows_allowed_by_the_existing_scope() {
    use crate::telegram::scope::Selector as ScopeSelector;
    use crate::time::Timestamp;

    let fixture = Fixture::new();
    let paths = Paths::under(&fixture.root);
    fs::create_dir_all(paths.state()).expect("bridge state");
    let sessions = fixture.root.join("sessions");
    let dispatcher = sessions.join(SEAT);
    let other = sessions.join(ORDINARY);
    let private = sessions.join("private-probe");
    let now = Timestamp::now().epoch();
    for (name, dir, id, role, reason) in [
        (
            SEAT,
            &dispatcher,
            "0199c0de-5555-4890-abcd-ef0123456789",
            true,
            "dispatcher reason",
        ),
        (
            ORDINARY,
            &other,
            "0199c0de-4444-4890-abcd-ef0123456789",
            false,
            "included reason",
        ),
        (
            "private-probe",
            &private,
            "0199c0de-7777-4890-abcd-ef0123456789",
            false,
            "private diagnostic reason",
        ),
    ] {
        fixture.start(name);
        fixture.record(dir, id, "s", role, "Manual choice remains");
        for (key, value) in [
            ("AE_SESSION", name),
            ("AE_HOME", fixture.root.to_str().expect("fixture root")),
        ] {
            let (ok, output) = run_tmux_op(&argv(
                &fixture.server,
                &Op::SetEnv {
                    session: name,
                    key,
                    value,
                },
            ));
            assert!(ok, "{output}");
        }
        fs::write(dir.join(".launch-attempt"), (now - 600).to_string())
            .expect("fixture launch attempt");
        fs::write(
            dir.join("events.jsonl"),
            format!(
                "{{\"ts\":\"{}\",\"actor\":\"lead\",\"action\":\"state\",\
                 \"ref\":\"blocked\",\"summary\":\"{reason}\"}}\n",
                Timestamp::from_epoch(now - 100)
            ),
        )
        .expect("fixture declaration");
    }
    let original = fs::read(dispatcher.join("desk-record")).expect("manual input bytes");
    let fake = Fake::start(vec![Reply::ok(), Reply::json(200, PIN_OK)]);
    let api = fake.api();
    let pass = |mode, also| {
        let relay = Relay::new(
            i64::MIN,
            paths.config.clone(),
            Scope { mode, also },
            crate::settings_menu::role_holder,
        );
        let mut err = Vec::new();
        let code = pump(
            &api,
            &paths,
            Knobs {
                once: true,
                ..Knobs::default()
            },
            &relay,
            None,
            &AtomicBool::new(false),
            &mut err,
        )
        .expect("real outbound pass");
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
    };
    let text = |index: usize| {
        let requests = fake.requests();
        let body = json::parse(&requests[index].body).expect("Telegram request JSON");
        let text = body.get_str("text").expect("pinned board text");
        assert!(text.contains("Manual choice remains"), "{text}");
        assert!(text.contains("LIVE") && text.contains("as of"), "{text}");
        assert!(text.encode_utf16().count() <= 3900, "{text}");
        text.to_owned()
    };

    pass(Mode::Orchestrator, Vec::new());
    assert_eq!(fake.requests().len(), 2, "one board and its pin");
    let default = text(0);
    assert!(!default.contains("partial"), "{default}");
    assert!(
        default.contains(SEAT) && default.contains("dispatcher reason"),
        "{default}"
    );
    assert!(
        !default.contains(ORDINARY) && !default.contains("included reason"),
        "{default}"
    );
    assert!(
        !default.contains("private-probe") && !default.contains("private diagnostic reason"),
        "{default}"
    );

    let broken = fixture.root.join("worktrees/broken");
    fs::create_dir_all(&broken).expect("unrelated worktree path");
    fs::write(broken.join(".ae"), b"cannot enumerate a file").expect("unreadable state root");
    let private_events = private.join("events.jsonl");
    let saved_events = fs::read(&private_events).expect("excluded journal");
    fs::remove_file(&private_events).expect("remove excluded journal");
    fs::create_dir(&private_events).expect("make excluded journal unreadable");
    pass(Mode::Orchestrator, Vec::new());
    assert_eq!(
        fake.requests().len(),
        2,
        "losses outside the scope cannot change or edit the restricted pin"
    );
    fs::remove_dir(&private_events).expect("remove unreadable journal node");
    fs::write(&private_events, saved_events).expect("restore excluded journal");

    fake.script(vec![Reply::ok()]);
    pass(
        Mode::Orchestrator,
        vec![ScopeSelector::Name(ORDINARY.to_owned())],
    );
    assert_eq!(
        fake.requests().len(),
        3,
        "one edit for the admitted session"
    );
    let additional = text(2);
    assert!(
        additional.contains(ORDINARY) && additional.contains("included reason"),
        "{additional}"
    );
    assert!(
        !additional.contains("private-probe") && !additional.contains("private diagnostic reason"),
        "{additional}"
    );

    fake.script(vec![Reply::ok()]);
    pass(Mode::All, Vec::new());
    assert_eq!(
        fake.requests().len(),
        4,
        "one edit to include the remaining session"
    );
    let all = text(3);
    assert!(
        all.contains("private-probe") && all.contains("private diagnostic reason"),
        "{all}"
    );
    assert!(
        all.contains("partial"),
        "all includes the failed worktree: {all}"
    );

    fs::remove_file(&private_events).expect("remove admitted journal");
    fs::create_dir(&private_events).expect("make admitted journal unreadable");
    fake.script(vec![Reply::ok()]);
    pass(
        Mode::Orchestrator,
        vec![ScopeSelector::Name("private-probe".to_owned())],
    );
    let unavailable = text(4);
    assert!(unavailable.contains("partial"), "{unavailable}");
    assert!(
        unavailable
            .lines()
            .any(|row| row.contains("private-probe") && row.contains("unavailable")),
        "{unavailable}"
    );
    assert!(
        !unavailable.contains("private diagnostic reason"),
        "{unavailable}"
    );
    assert!(!unavailable.contains("included reason"), "{unavailable}");
    assert_eq!(
        fs::read(dispatcher.join("desk-record")).expect("manual output bytes"),
        original,
    );
}

mod live_scope_identity {
    use std::cell::Cell;

    use crate::desklive::{Cache, Live, LiveSeat, LiveSession, SessionBody};
    use crate::telegram::desk::Machine as Board;
    use crate::telegram::routing::{RunningSession, Seat, World};
    use crate::telegram::scope::Selector;
    use crate::time::Timestamp;

    use super::super::super::{Fleet, board_pass_with};
    use super::{Fake, Fixture, Mode, ORDINARY, PIN_OK, Paths, Reply, SEAT, Scope, json};

    const START: i64 = 1_795_164_012;
    const SEAT_ID: &str = "0199c0de-5555-4890-abcd-ef0123456789";
    const OLD_ID: &str = "0199c0de-4444-4890-abcd-ef0123456789";
    const NEW_ID: &str = "0199c0de-7777-4890-abcd-ef0123456789";

    struct Census {
        seat: RunningSession,
        peers: Vec<RunningSession>,
        calls: Cell<usize>,
        changes_after_first_read: bool,
    }

    impl World for Census {
        fn running(&self) -> Vec<RunningSession> {
            let mut sessions = vec![self.seat.clone()];
            sessions.extend(self.peers.iter().cloned());
            sessions
        }

        fn seat(&self) -> Seat {
            let calls = self.calls.replace(self.calls.get() + 1);
            if self.changes_after_first_read && calls > 0 {
                Seat::Unproven("the role census changed".to_owned())
            } else {
                Seat::Held {
                    dir: self.seat.dir.clone(),
                    name: self.seat.name.clone(),
                }
            }
        }
    }

    fn running(dir: &std::path::Path, name: &str, id: &str, meta_agent: bool) -> RunningSession {
        RunningSession {
            name: name.to_owned(),
            dir: dir.to_owned(),
            session_id: id.to_owned(),
            meta_agent,
            main: None,
            agents: Vec::new(),
            pair: Vec::new(),
            last_active: None,
        }
    }

    fn census(
        seat: &std::path::Path,
        peers: Vec<RunningSession>,
        changes_after_first_read: bool,
    ) -> Census {
        Census {
            seat: running(seat, SEAT, SEAT_ID, true),
            peers,
            calls: Cell::new(0),
            changes_after_first_read,
        }
    }

    fn sampled(session: &RunningSession, reason: &str) -> LiveSession {
        LiveSession {
            name: session.name.clone(),
            dir: Some(session.dir.clone()),
            uuid: Some(session.session_id.clone()),
            body: Ok(SessionBody {
                seats: vec![LiveSeat {
                    name: "lead".to_owned(),
                    main: true,
                    state: "blocked".to_owned(),
                    since: Some(Timestamp::from_epoch(START - 60)),
                    reason: reason.to_owned(),
                }],
                asks: Vec::new(),
            }),
        }
    }

    fn cache(as_of: i64, sessions: Vec<LiveSession>) -> Cache {
        Cache::holding(
            as_of,
            Live {
                as_of: Timestamp::from_epoch(as_of),
                sessions,
                losses: 0,
            },
        )
    }

    fn pin_text(fake: &Fake, request: usize) -> String {
        let requests = fake.requests();
        let body = json::parse(&requests[request].body).expect("pin HTTP request");
        body.get_str("text").expect("pin text").to_owned()
    }

    #[test]
    fn a_default_pin_keeps_the_first_proven_seat_scope_for_the_whole_pass() {
        let fixture = Fixture::new();
        fixture.start(SEAT);
        let paths = Paths::under(&fixture.root);
        std::fs::create_dir_all(paths.state()).expect("bridge state");
        let seat = fixture.root.join("sessions").join(SEAT);
        fixture.record(&seat, SEAT_ID, "s", true, "Manual decision");
        let private = running(
            &fixture.root.join("sessions").join(ORDINARY),
            ORDINARY,
            OLD_ID,
            false,
        );
        let world = census(&seat, vec![private.clone()], true);
        let mut sample = cache(
            START,
            vec![
                sampled(&world.seat, "held seat reason"),
                sampled(&private, "private reason"),
            ],
        );
        let mut board = Board::new(paths.state());
        let fake = Fake::start(vec![Reply::ok(), Reply::json(200, PIN_OK)]);
        board_pass_with(
            &fake.api(),
            &world,
            Some(Fleet {
                root: &fixture.root,
                scope: &Scope::default(),
                cache: &mut sample,
            }),
            &mut board,
            START,
            &mut Vec::new(),
        );

        assert_eq!(fake.requests().len(), 2, "one board and one pin");
        let text = pin_text(&fake, 0);
        assert!(
            text.contains("Manual decision") && text.contains("held seat reason"),
            "{text}"
        );
        assert!(
            !text.contains(ORDINARY) && !text.contains("private reason"),
            "{text}"
        );
    }

    #[test]
    fn a_newly_admitted_uuid_cannot_export_a_cached_namesake() {
        let fixture = Fixture::new();
        fixture.start(SEAT);
        let paths = Paths::under(&fixture.root);
        std::fs::create_dir_all(paths.state()).expect("bridge state");
        let seat = fixture.root.join("sessions").join(SEAT);
        fixture.record(&seat, SEAT_ID, "s", true, "Manual decision");
        let peer_path = fixture.root.join("sessions").join(ORDINARY);
        let wrong_path = fixture.root.join("worktrees/other/.ae").join(ORDINARY);
        let old = running(&peer_path, ORDINARY, OLD_ID, false);
        let current = running(&peer_path, ORDINARY, NEW_ID, false);
        let namesake = running(&wrong_path, ORDINARY, NEW_ID, false);
        let before = census(&seat, vec![old.clone()], false);
        let after = census(&seat, vec![current.clone()], false);
        let mut sample = cache(
            START,
            vec![
                sampled(&before.seat, "held seat reason"),
                sampled(&old, "excluded previous UUID"),
                sampled(&namesake, "excluded other directory"),
            ],
        );
        let scope = Scope {
            mode: Mode::Orchestrator,
            also: vec![Selector::Id(NEW_ID.to_owned())],
        };
        let mut board = Board::new(paths.state());
        let fake = Fake::start(vec![Reply::ok(), Reply::json(200, PIN_OK)]);
        let mut pass = |world: &Census, cache: &mut Cache, now: i64| {
            board_pass_with(
                &fake.api(),
                world,
                Some(Fleet {
                    root: &fixture.root,
                    scope: &scope,
                    cache,
                }),
                &mut board,
                now,
                &mut Vec::new(),
            );
        };

        pass(&before, &mut sample, START);
        assert!(pin_text(&fake, 0).contains("held seat reason"));
        pass(&after, &mut sample, START + 1);
        assert_eq!(
            fake.requests().len(),
            2,
            "a recent sample with no matching identity cannot add either namesake"
        );

        let mut refreshed = cache(
            START + 60,
            vec![
                sampled(&after.seat, "held seat reason"),
                sampled(&current, "admitted current UUID"),
                sampled(&namesake, "excluded other directory"),
            ],
        );
        fake.script(vec![Reply::ok()]);
        pass(&after, &mut refreshed, START + 60);
        assert_eq!(
            fake.requests().len(),
            3,
            "one edit for the admitted identity"
        );
        let text = pin_text(&fake, 2);
        assert!(text.contains("admitted current UUID"), "{text}");
        assert!(!text.contains("excluded other directory"), "{text}");
    }
}
