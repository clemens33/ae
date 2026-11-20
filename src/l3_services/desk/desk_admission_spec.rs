//! The write admission re-observes the same real pane under the record lock.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the acceptance test owns and observes a private tmux socket and scratch files"
)]

use std::cell::Cell;
use std::fs;
use std::io::{self, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{SetError, SetOutcome, admit_set, admit_set_with_sync, render_set};
use crate::desk::parse_input;
use crate::inventory::ServerId;
use crate::meta::Selector;
use crate::relay::Observed;
use crate::session_tmux::{Op, argv};
use crate::store;
use crate::time::Timestamp;
use crate::transport::{self, fixture_tmux};

const SESSION: &str = "desk-race";
const UUID: &str = "0199c0de-5555-4890-abcd-ef0123456789";
const BOARD: &[u8] = b"ae-desk 1\nupdated 2026-11-20T08:40:12Z\nD1\tFirst board\n";
const JOURNAL: &[u8] = b"{\"ts\":\"2026-11-20T08:40:12Z\",\"actor\":\"boss\",\"action\":\"desk\",\"summary\":\"+D1\"}\n";

struct Fixture {
    root: PathBuf,
    dir: PathBuf,
    server: ServerId,
    pane: String,
}

impl Fixture {
    fn new() -> Self {
        let name = format!(
            "ae-desk-admit-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        );
        let root = Path::new("/tmp").join(name);
        let dir = root.join("sessions").join(SESSION);
        fs::create_dir_all(&dir).expect("private dispatcher directory");
        fs::write(
            root.join("orchestrator.config"),
            crate::config::SEAT_TEMPLATE,
        )
        .expect("the private dispatcher overlay");
        let server = ServerId::Selected(Selector::Socket(root.join("s")));
        let mut fixture = Self {
            root,
            dir,
            server,
            pane: String::new(),
        };
        let (ok, output) = transport::run_tmux_op(&argv(
            &fixture.server,
            &Op::NewSession {
                name: SESSION,
                work_dir: fixture.root.to_str().expect("private directory"),
            },
        ));
        assert!(ok, "start owned tmux: {output}");
        fixture.pane = transport::observe_window_panes(&fixture.server, SESSION)
            .expect("owned session answers")
            .into_iter()
            .next()
            .expect("initial pane")
            .pane_id;
        fixture.pane_option("@ae_agent", "boss");
        fixture.pane_option("@ae_slot", "main");
        let command = format!("set-option -t ={SESSION}: @ae_session_uuid {UUID}");
        assert!(transport::run_view_command(
            &fixture.server,
            SESSION,
            &command
        ));
        fixture.meta("launch-before");
        fs::write(fixture.dir.join("desk-record"), BOARD).expect("existing board");
        fs::write(fixture.dir.join("events.jsonl"), JOURNAL).expect("existing audit");
        fixture
    }

    fn pane_option(&self, key: &str, value: &str) {
        let command = format!("set-option -p -t {} {key} {value}", self.pane);
        assert!(transport::run_view_command(&self.server, SESSION, &command));
    }

    fn meta(&self, launch: &str) {
        fs::write(
            self.dir.join("meta"),
            format!(
                "session={SESSION}\nsession_id={UUID}\nmeta_agent=true\nlocal_config={}\nseat.main=boss\nlaunch_id.main={launch}\nmain_pane={}\ntmux_server_kind=socket\ntmux_server={}\n",
                self.root.join("orchestrator.config").display(),
                self.pane,
                self.root.join("s").display(),
            ),
        )
        .expect("dispatcher proof metadata");
    }

    fn observe(&self, pane: &str) -> Option<Observed> {
        transport::observe_viewer(&self.server, pane).map(|viewer| Observed {
            viewer,
            server: self.server.clone(),
            pane: Some(pane.to_owned()),
        })
    }

    fn assert_preserved(&self) {
        assert_eq!(
            fs::read(self.dir.join("desk-record")).expect("prior board"),
            BOARD
        );
        assert_eq!(
            fs::read(self.dir.join("events.jsonl")).expect("prior audit"),
            JOURNAL
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fixture_tmux::end(&self.server);
        for _ in 0..20 {
            if fixture_tmux::probe(&self.server).proven() {
                let _ = fs::remove_dir_all(&self.root);
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(std::thread::panicking(), "owned socket cleanup unproven");
    }
}

#[test]
fn another_launch_missing_pane_or_retagged_slot_under_the_lock_cannot_publish() {
    let fixture = Fixture::new();
    let lock = store::lock_path(&fixture.dir.join("desk-record"));
    let items = parse_input(b"DECIDE\nD7 | Became a new board\n").expect("valid changed input");
    let now = Timestamp::from_epoch(1_795_164_800);
    for case in ["launch changed", "pane unavailable", "slot changed"] {
        let mut seen = 0;
        let mut observe = || {
            seen += 1;
            if seen == 1 {
                assert!(
                    store::lock(&lock, Duration::ZERO).is_ok(),
                    "proof one precedes the lock"
                );
                return fixture.observe(&fixture.pane);
            }
            assert_eq!(seen, 2);
            let held = store::lock(&lock, Duration::ZERO).expect_err("proof two holds the lock");
            assert_eq!(held.kind(), ErrorKind::WouldBlock);
            match case {
                "launch changed" => fixture.meta("launch-after"),
                "pane unavailable" => return fixture.observe("%99999"),
                "slot changed" => fixture.pane_option("@ae_slot", "worker.0"),
                _ => unreachable!("fixture cases are closed"),
            }
            fixture.observe(&fixture.pane)
        };
        let why = admit_set(&fixture.dir, &mut observe, items.clone(), now)
            .expect_err("second observation must refuse")
            .to_string();
        assert_eq!(seen, 2);
        match case {
            "launch changed" => assert!(
                why.contains("the seat changed while the board was set"),
                "{why}"
            ),
            "pane unavailable" => assert!(why.contains("the caller is unproven now"), "{why}"),
            _ => assert!(
                why.contains("the caller is unproven now") && why.contains("main seat"),
                "{why}"
            ),
        }
        fixture.assert_preserved();
        fixture.meta("launch-before");
        fixture.pane_option("@ae_slot", "main");
    }

    let mut observations = 0;
    let mut same = || {
        observations += 1;
        fixture.observe(&fixture.pane)
    };
    assert!(matches!(
        admit_set(&fixture.dir, &mut same, items, now),
        Ok(SetOutcome::Changed { total: 1, .. })
    ));
    assert_eq!(observations, 2);
    let published = fs::read_to_string(fixture.dir.join("desk-record")).expect("replacement board");
    assert_eq!(
        published.lines().skip(2).collect::<Vec<_>>(),
        ["D7\tBecame a new board"]
    );
    let journal = fs::read_to_string(fixture.dir.join("events.jsonl")).expect("new audit");
    let mut events = journal.lines();
    assert_eq!(
        events.next().expect("prior event").as_bytes(),
        &JOURNAL[..JOURNAL.len() - 1]
    );
    let event = crate::events::Event::parse_line(events.next().expect("new event"))
        .expect("parse new audit");
    assert_eq!(
        (event.action.as_str(), event.actor.as_str()),
        ("desk", "boss")
    );
    assert!(events.next().is_none());
}

#[test]
fn the_admitted_writer_reports_a_failed_directory_sync_and_leaves_the_audit_intact() {
    let fixture = Fixture::new();
    let record = fixture.dir.join("desk-record");
    let journal = fixture.dir.join("events.jsonl");
    let lock = store::lock_path(&record);
    let items = parse_input(b"RUNNING\nR8 | Ready for review\n").expect("valid replacement");
    let now = Timestamp::parse("2026-11-20T08:41:12Z").expect("different update time");
    let expected = b"ae-desk 1\nupdated 2026-11-20T08:41:12Z\nR8\tReady for review\n";
    let observations = Cell::new(0);
    let mut observe = || {
        observations.set(observations.get() + 1);
        fixture.observe(&fixture.pane)
    };
    let mut did_sync = false;
    let result = admit_set_with_sync(&fixture.dir, &mut observe, items, now, |dir| {
        did_sync = true;
        assert_eq!(observations.get(), 2, "both pane observations precede sync");
        assert_eq!(dir, fixture.dir.as_path());
        let held = store::lock(&lock, Duration::ZERO).expect_err("admission holds the writer lock");
        assert_eq!(held.kind(), ErrorKind::WouldBlock);
        assert_eq!(fs::read(&record).expect("visible replacement"), expected);
        assert_eq!(fs::read(&journal).expect("prior audit"), JOURNAL);
        Err(io::Error::other("fixture admitted directory sync failed"))
    });
    assert!(did_sync, "the admitted writer reaches directory sync");
    assert_eq!(observations.get(), 2);
    assert!(matches!(&result, Err(SetError::Unsynced(_))), "{result:?}");
    assert_eq!(fs::read(&record).expect("published replacement"), expected);
    assert_eq!(fs::read(&journal).expect("unchanged journal"), JOURNAL);

    let outcome = render_set(&result, 1);
    assert_eq!(outcome.code, 2);
    assert!(outcome.stdout.is_empty());
    assert!(
        outcome.stderr.starts_with("desk set UNCONFIRMED:")
            && outcome.stderr.contains("the board was replaced (1 item)")
            && outcome.stderr.contains("no journal event was written")
            && outcome
                .stderr
                .contains("fixture admitted directory sync failed"),
        "{}",
        outcome.stderr
    );
}

struct InputAcrossLaunch<'a> {
    fixture: &'a Fixture,
    bytes: &'static [u8],
    changed: bool,
}

impl Read for InputAcrossLaunch<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.changed {
            self.fixture.meta("launch-after-input-started");
            self.changed = true;
        }
        self.bytes.read(buf)
    }
}

#[test]
fn the_helper_cannot_complete_a_request_after_the_launch_changes_during_input() {
    let fixture = Fixture::new();
    let mut input = InputAcrossLaunch {
        fixture: &fixture,
        bytes: b"RUNNING\nR8 | Belongs to the original launch\n",
        changed: false,
    };
    let mut observations = 0;
    let mut observe = || {
        observations += 1;
        fixture.observe(&fixture.pane)
    };
    let now = Timestamp::parse("2026-11-20T08:41:12Z").expect("different update time");
    let argv = [String::from("set"), String::from("-")];
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = crate::deskcmd::run(
        &fixture.dir,
        &argv,
        &mut observe,
        now,
        &mut input,
        &mut out,
        &mut err,
    )
    .expect("capture the real helper output");
    assert!(
        input.changed,
        "the original dispatcher was allowed to supply input"
    );
    assert_eq!(code, 1, "a new launch cannot complete the original request");
    assert!(out.is_empty());
    let refusal = String::from_utf8(err).expect("helper stderr text");
    assert!(
        refusal.starts_with("desk set REFUSED:")
            && refusal.contains("the seat changed while the board was set"),
        "{refusal}"
    );
    assert_eq!(observations, 2, "the first proof covers the input read");
    fixture.assert_preserved();
}
