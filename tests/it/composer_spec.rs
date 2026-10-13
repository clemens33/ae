//! Frozen composer acceptance: brief R1-R4 + S1. Oracles are public input,
//! captured screens and the journal. Stable API only: RED on base, GREEN final.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures"
)]

use ae::console::input::{self, Effect, Input, Reading, Size};
use ae::events::Event;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const UUID: &str = "0199c0de-dddd-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);
const WORD: &str = "fixture words";

fn owner() -> (Input, Instant) {
    let mut input = Input::new(vec!["lead".to_owned(), "colead".to_owned()]);
    let at = Instant::now();
    let _ = input.tick(Reading::Owner, at);
    (input, at)
}

fn rows(input: &Input) -> Vec<String> {
    input
        .bare_view(Size {
            width: 40,
            height: 24,
        })
        .rows
}

fn asks(effects: &[Effect]) -> Vec<Vec<u8>> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Ask { raw, .. } => Some(raw.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn ctrl_j_inserts_a_newline_it_never_sends() {
    let (mut input, at) = owner();
    let effects = input.chunk(b"a\x0ab", at + Duration::from_millis(1));
    assert!(effects.is_empty(), "Ctrl+J sends nothing: {effects:?}");
    assert_eq!(rows(&input), ["a", "b"]);
    let effects = input.chunk(b"\r", at + Duration::from_millis(2));
    assert_eq!(asks(&effects), [b"a\nb".as_slice()]);
}

#[test]
fn alt_enter_inserts_a_newline() {
    for seq in [b"\x1b\r".as_slice(), b"\x1b\n".as_slice()] {
        let (mut input, at) = owner();
        let stream = [b"a".as_slice(), seq, b"b"].concat();
        let effects = input.chunk(&stream, at + Duration::from_millis(1));
        assert!(effects.is_empty(), "{seq:?} sends nothing: {effects:?}");
        assert_eq!(rows(&input), ["a", "b"], "{seq:?}");
    }
}

#[test]
fn paste_folds_cr_and_crlf_to_lf() {
    let (mut input, at) = owner();
    let effects = input.chunk(
        b"\x1b[200~a\rb\r\nc\nd\x1b[201~\r",
        at + Duration::from_millis(1),
    );
    assert_eq!(asks(&effects), [b"a\nb\nc\nd".as_slice()]);
    let (mut input, at) = owner();
    let mut keys = input::Keys::app();
    let fed = keys.feed(
        b"\x1b[200~a\rb\r\nc\x1b[201~\r",
        at + Duration::from_millis(1),
    );
    assert_eq!(asks(&input.keyed(fed)), [b"a\nb\nc".as_slice()]);
}

#[test]
fn refused_command_keeps_its_bytes_in_the_composer() {
    for (line, want) in [(b"@lead \r".as_slice(), "@lead "), (b"/nope\r", "/nope")] {
        let (mut input, at) = owner();
        let effects = input.chunk(line, at + Duration::from_millis(1));
        assert!(asks(&effects).is_empty(), "{effects:?}");
        assert_eq!(rows(&input), [want]);
    }
}

#[test]
fn read_only_refusal_keeps_its_bytes() {
    let mut input = Input::new(vec!["lead".to_owned(), "colead".to_owned()]);
    let at = Instant::now();
    let _ = input.tick(Reading::NotOwner("another chat owns input".to_owned()), at);
    let effects = input.chunk(b"hello\r", at + Duration::from_millis(1));
    assert!(asks(&effects).is_empty(), "{effects:?}");
    assert_eq!(rows(&input), ["hello"]);
}

struct Rig {
    tool: super::deliver::Rig,
    root: PathBuf,
    socket: PathBuf,
    name: String,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let tool = super::deliver::Rig::new(tag, "codex", 0);
        let dir = tool.dir.clone();
        let root = dir
            .parent()
            .expect("sessions")
            .parent()
            .expect("root")
            .to_owned();
        let name = dir
            .file_name()
            .expect("session")
            .to_string_lossy()
            .into_owned();
        let socket = root.join("sock");
        let rig = Self {
            tool,
            root,
            socket,
            name,
        };
        fs::write(rig.tool.dir.join("meta"), format!(
            "session={}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=codex\nlaunch_id.main=tok-composer\nseat.worker.0=colead\nagent_bin.worker.0=codex\nlaunch_id.worker.0=tok-colead\n",
            rig.name, rig.socket.display())).expect("lead pair meta");
        fs::write(rig.root.join("config"), "").expect("isolated config");
        fs::write(rig.tool.dir.join("events.jsonl"), "").expect("empty journal");
        for (option, value) in [
            ("@ae_look", "off"),
            ("@ae_icons", "off"),
            ("@ae_session_uuid", UUID),
            ("@ae_motion", "off"),
            ("@ae_main_pane", rig.tool.pane.as_str()),
            ("default-size", "160x45"),
        ] {
            rig.tmux(&["set-option", "-t", &rig.name, option, value]);
        }
        rig.tmux(&[
            "set-option",
            "-p",
            "-t",
            &rig.tool.pane,
            "@ae_agent",
            "lead",
        ]);
        rig
    }

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|word| (*word).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &self.root);
        assert!(ok, "private tmux {tail:?}: {out}");
        out
    }

    fn open(&self, app: bool) -> String {
        if app {
            fs::write(self.root.join("config"), "[workspace]\nchat = app\n").expect("chat = app");
        }
        let out = super::cli::ae()
            .env("AE_HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .env("TMUX", format!("{},1,0", self.socket.display()))
            .env("TMUX_PANE", &self.tool.pane)
            .arg("_console")
            .output()
            .expect("chat window opens");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{err}");
        self.tmux(&[
            "list-panes",
            "-s",
            "-t",
            &self.name,
            "-F",
            "#{pane_id}|#{@ae_console}",
        ])
        .lines()
        .find_map(|row| {
            row.split_once('|')
                .and_then(|(p, s)| (s == UUID).then(|| p.to_owned()))
        })
        .expect("stamped console")
    }

    fn wait(&self, pane: &str, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + WAIT;
        loop {
            let screen = self.tmux(&["capture-pane", "-p", "-t", pane]);
            if met(&screen) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}; terminal screen:\n{screen}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn keys(&self, pane: &str, key: &str) {
        self.tmux(&["send-keys", "-t", pane, key]);
    }

    fn write(&self, pane: &str) {
        let until = Instant::now() + WAIT;
        while self
            .tmux(&["capture-pane", "-p", "-t", pane])
            .contains("? keys")
        {
            assert!(Instant::now() < until, "app never starts writing");
            self.keys(pane, "i");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn literal(&self, pane: &str, text: &str) {
        self.tmux(&["send-keys", "-t", pane, "-l", "--", text]);
    }

    fn type_line(&self, pane: &str, line: &str) {
        self.literal(pane, line);
        self.keys(pane, "Enter");
    }

    fn seed_five(&self) -> Vec<String> {
        use std::fmt::Write as _;
        let ids: Vec<String> = (11..=15)
            .map(|n| format!("ae-20260930T000000Z-000000{n}"))
            .collect();
        let mut records = String::new();
        for id in &ids {
            writeln!(records, "{{\"ts\":\"2026-09-30T00:00:00Z\",\"actor\":\"console:local\",\"action\":\"ask\",\"target\":\"lead\",\"ref\":\"{id}\",\"target_slot\":\"main\",\"target_session\":\"{}\",\"summary\":\"{WORD}\"}}", self.name).expect("fixture record");
        }
        fs::write(self.tool.dir.join("events.jsonl"), records).expect("five open asks");
        ids
    }

    fn asks(&self) -> Vec<Event> {
        self.tool
            .events()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| Event::parse_line(line).expect("every recorded event parses"))
            .filter(|event| event.actor == "console:local" && event.action == "ask")
            .collect()
    }

    fn ready(&self, pane: &str) {
        self.wait(pane, "owner prompt", |s| s.contains("to lead>"));
    }
}

#[test]
fn chat_ctrl_j_inserts_and_enter_sends_one_multiline_ask() {
    let rig = Rig::new("composer-r1");
    let pane = rig.open(false);
    rig.ready(&pane);
    rig.literal(&pane, "a");
    rig.keys(&pane, "C-j");
    rig.literal(&pane, "b");
    rig.keys(&pane, "Enter");
    let until = Instant::now() + WAIT;
    while rig.asks().is_empty() {
        assert!(Instant::now() < until, "no ask recorded");
        std::thread::sleep(Duration::from_millis(25));
    }
    let asks = rig.asks();
    assert_eq!(asks.len(), 1, "one ask, not one per line");
    let body = fs::read_to_string(asks[0].body_file.as_deref().expect("stored body path"))
        .expect("stored tracked body");
    assert!(body.contains("a\nb"), "body keeps the line break: {body:?}");
}

#[test]
fn chat_cap_refusal_keeps_the_draft_visible() {
    let rig = Rig::new("composer-r4chat");
    rig.seed_five();
    let pane = rig.open(false);
    rig.ready(&pane);
    rig.type_line(&pane, "sixth stays visible");
    let screen = rig.wait(&pane, "cap refusal", |s| s.contains("5 requests open"));
    assert!(
        screen.contains("sixth stays visible"),
        "draft kept: {screen}"
    );
}

#[test]
fn close_names_the_ask_it_closed() {
    let rig = Rig::new("composer-close");
    let ids = rig.seed_five();
    let pane = rig.open(false);
    rig.ready(&pane);
    rig.type_line(&pane, "/close");
    let screen = rig.wait(&pane, "close is answered", |screen| {
        screen.contains("closed ")
    });
    let named = screen.contains(&ids[4]) && screen.contains(WORD);
    assert!(named, "close names id + words: {screen}");
}

#[test]
fn app_writing_shows_its_keys() {
    let rig = Rig::new("composer-r3");
    let pane = rig.open(true);
    rig.wait(&pane, "browse is drawn", |s| s.contains("? keys"));
    rig.write(&pane);
    rig.literal(&pane, "probe-keys");
    let screen = rig.wait(&pane, "app is writing", |s| s.contains("probe-keys"));
    let footer = screen.lines().last().unwrap_or_default();
    assert!(
        footer.contains("Enter sends · Alt+Enter ^J newline · Esc browses"),
        "writing keys row: {footer:?}"
    );
}

#[test]
fn app_cap_refusal_keeps_the_draft_visible() {
    let rig = Rig::new("composer-r4app");
    rig.seed_five();
    let pane = rig.open(true);
    rig.wait(&pane, "lane drawn", |s| s.contains("fixture words"));
    rig.write(&pane);
    rig.type_line(&pane, "app sixth stays");
    let screen = rig.wait(&pane, "cap refusal", |s| s.contains("5 requests open"));
    assert!(screen.contains("app sixth stays"), "draft kept: {screen}");
}
