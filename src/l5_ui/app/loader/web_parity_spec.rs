//! S1a parity between the app reader's retained panel lane and the real web
//! source, including a stored reply, relay-echo policy and a partial journal.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::{Answer, Reader, Request, SettingsPaths};
use crate::app::tests::Root;
use crate::console::Reach;
use crate::console::lane::Kind;
use crate::inventory::ServerId;
use crate::meta::Selector;
use crate::transport::{self, fixture_tmux};
use crate::web::state::{LiveSource, Read, Source, Standing};

const ORCH: &str = "orchestrator";
const UUID: &str = "0199c0de-1234-4890-abcd-ef0123456788";
const ASK: &str = "ae-20260930T060000Z-0000abcd";
const BODY: &str = "full-answer-marker\nsecond line of the stored answer";
const SHORT: &str = "short fallback is not the stored answer";
const ECHO: &str = "raw-relay-marker";
const TAIL: &str = "completed-tail-marker";

struct PrivateTmux {
    server: ServerId,
    retained: Arc<AtomicBool>,
}

impl PrivateTmux {
    fn new(root: &Root) -> Self {
        let server = ServerId::Selected(Selector::Socket(root.0.join("s")));
        let rig = Self {
            server,
            retained: root.retention(),
        };
        let argv = crate::session_tmux::argv(
            &rig.server,
            &crate::session_tmux::Op::NewSession {
                name: ORCH,
                work_dir: root.0.to_str().unwrap_or_default(),
            },
        );
        let (ok, output) = transport::run_tmux_op(&argv);
        assert!(ok, "private tmux start: {output}");
        assert!(transport::run_view_command(
            &rig.server,
            ORCH,
            &format!("set-option -t ={ORCH}: @ae_session_uuid {UUID}")
        ));
        rig
    }
}

impl Drop for PrivateTmux {
    fn drop(&mut self) {
        let _ = fixture_tmux::end(&self.server);
        for _ in 0..20 {
            if fixture_tmux::probe(&self.server).proven() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.retained.store(true, Ordering::SeqCst);
        assert!(std::thread::panicking(), "private tmux cleanup unproven");
    }
}

fn save(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
}

fn journal(dir: &Path, complete_tail: bool) {
    let messages = dir.join("messages");
    std::fs::create_dir_all(&messages).unwrap_or_else(|error| panic!("fixture messages: {error}"));
    let stored = messages.join(format!("{ASK}.reply.0000aa.txt"));
    save(&stored, BODY);
    let opening = format!(
        r#"{{"ts":"2026-09-30T06:00:00Z","actor":"console:local","action":"ask","target":"lead","ref":"{ASK}","target_slot":"main","target_session":"{ORCH}","target_server":"/fixture","target_pane":"%1","target_session_uuid":"{UUID}","summary":"the fixture question"}}"#
    );
    let answer = format!(
        r#"{{"ts":"2026-09-30T06:01:00Z","actor":"lead","action":"reply","target":"console:local","ref":"{ASK}","actor_slot":"main","actor_session":"{ORCH}","caller_server":"/fixture","caller_pane":"%1","caller_session_uuid":"{UUID}","body_file":"{}","summary":"{SHORT}"}}"#,
        stored.display()
    );
    let echo = format!(
        r#"{{"ts":"2026-09-30T06:02:00Z","actor":"ae:relay","action":"chat","ref":"{ASK}","summary":"reply fd5bf0 from lead: {ECHO}"}}"#
    );
    let tail = format!(
        r#"{{"ts":"2026-09-30T06:03:00Z","actor":"ae:compact","action":"chat","summary":"{TAIL}"}}"#
    );
    let end = if complete_tail { "\n" } else { "" };
    save(
        &dir.join(crate::store::EVENTS),
        &format!("{opening}\n{answer}\n{echo}\n{tail}{end}"),
    );
}

fn panel(reader: &mut Reader) -> super::ViewRead {
    let Some(Answer::View(mut view)) = reader.stage(ORCH) else {
        panic!("the panel reader returns a lane");
    };
    while reader.staging(ORCH, Reach::Every) {
        let Some(Answer::View(current)) = reader.stage(ORCH) else {
            panic!("the panel reader completes all standing lanes");
        };
        view = current;
    }
    assert_eq!(view.name, ORCH);
    assert_eq!(view.id, UUID);
    view
}

fn compare(panel: &super::ViewRead, web: &Read, hidden: bool, complete: bool) {
    assert_eq!(web.orchestrator.state, Standing::Proven);
    assert_eq!(web.orchestrator.name.as_deref(), Some(ORCH));

    let expected: Vec<_> = panel
        .lane
        .items
        .iter()
        .map(|item| item.body.as_str())
        .collect();
    let actual: Vec<_> = web.chat.rows.iter().map(|row| row.body.as_str()).collect();
    assert!(!expected.is_empty());
    assert_eq!(actual, expected);
    for (item, row) in panel.lane.items.iter().zip(&web.chat.rows) {
        assert_eq!(row.micros, item.micros);
        assert_eq!(row.key, crate::app::lane::key(item));
    }
    assert_eq!(web.chat.coverage, panel.lane.coverage);
    assert_eq!(web.chat.cut, 0);

    assert!(
        panel
            .lane
            .items
            .iter()
            .any(|item| { matches!(item.kind, Kind::Answer { .. }) && item.body == BODY })
    );
    assert!(web.chat.rows.iter().any(|row| {
        row.kind == "answer"
            && row.body == BODY
            && row.voice == "lead"
            && row.who == "lead"
            && row.what == "answers"
    }));
    let has = |marker: &str| actual.iter().any(|body| body.contains(marker));
    assert!(!has(SHORT));
    assert_eq!(has(ECHO), !hidden);
    assert_eq!(has(TAIL), complete);
    assert_eq!(
        web.chat
            .coverage
            .iter()
            .any(|line| line.starts_with("relay — answer echoes hidden")),
        hidden
    );
}

#[test]
fn web_source_agrees_with_the_panel_on_a_reply_relay_echo_and_partial_journal() {
    let root = Root::new("web-parity");
    let socket = PrivateTmux::new(&root);
    let overlay = root.0.join("orchestrator.config");
    save(&overlay, "[workspace]\norchestrator = true\n");
    let dir = root.0.join("sessions").join(ORCH);
    std::fs::create_dir(&dir).unwrap_or_else(|error| panic!("fixture session: {error}"));
    save(
        &dir.join("meta"),
        &format!(
            "schema=2\nsession_id={UUID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\n\
             session={ORCH}\nmeta_agent=true\nlocal_config={}\ntmux_server_kind=socket\ntmux_server={}\n",
            overlay.display(),
            root.0.join("s").display()
        ),
    );
    let global = root.0.join("config");
    save(&global, "[workspace]\nrelay_replies = off\n");
    journal(&dir, false);

    let mut reader = Reader::new(root.0.clone(), None, Some(socket.server.clone()));
    reader.fleet_at = Some(Instant::now());
    reader.dirs.insert(ORCH.to_owned(), dir.clone());
    reader.ids.insert(ORCH.to_owned(), UUID.to_owned());
    reader.take(Request::Panel(Some(ORCH.to_owned())));
    reader.settings_paths(SettingsPaths {
        home: Some(root.0.join("home")),
        global: global.clone(),
        local: None,
    });
    let web = LiveSource::new(root.0.clone(), global.clone());
    compare(&panel(&mut reader), &web.read(), true, false);

    save(&global, "[workspace]\nrelay_replies = on\n");
    compare(&panel(&mut reader), &web.read(), false, false);
    journal(&dir, true);
    compare(&panel(&mut reader), &web.read(), false, true);
}
