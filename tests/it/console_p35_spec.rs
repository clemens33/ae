//! Independent P3.5 acceptance: a kept draft is literal until fresh Enter;
//! ownership promotion restores it, while another console only reads. Speaker
//! choice sticks in process memory and a fresh process starts at the main seat.
//! Oracle: P3.5 backlog items 1-2, plus lead's no-parse/all-seats banner ruling.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures own private sockets and inspect real console effects"
)]

use std::fs;
use std::io::Read as _;
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ae::console::input::{Effect, Input, Reading};
use ae::events::{Event, RoutingMember};

const UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const LIMIT: Duration = Duration::from_secs(20);
const OBSERVER: &str = r"use strict;
use warnings;
use IO::Socket::UNIX;
my $s = IO::Socket::UNIX->new(Peer => $ARGV[0], Type => SOCK_STREAM) or die $!;
binmode(STDIN); binmode($s); $s->autoflush(1);
while (sysread(STDIN, my $b, 4096)) { print {$s} $b or last; }
";

fn quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

struct Output {
    bytes: Vec<u8>,
    chunks: Receiver<Vec<u8>>,
    stream: UnixStream,
    reader: Option<JoinHandle<()>>,
}

impl Output {
    fn attach(rig: &Rig, pane: &str, tag: &str) -> Self {
        let socket = rig.root.join(tag);
        let listener = UnixListener::bind(&socket).expect("private observer socket");
        let (send, chunks) = mpsc::channel();
        let (connected, connection) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("observer connects");
            connected
                .send(stream.try_clone().expect("shutdown handle"))
                .expect("live observer");
            let mut buffer = [0; 4096];
            while let Ok(n) = stream.read(&mut buffer) {
                if n == 0 || send.send(buffer[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let script = rig.root.join("observer.pl");
        fs::write(&script, OBSERVER).expect("external output observer");
        let command = format!("exec perl {} {}", quote(&script), quote(&socket));
        rig.tmux(&["pipe-pane", "-O", "-t", pane, &command]);
        let stream = connection.recv_timeout(LIMIT).expect("observer ready");
        let bytes = rig
            .tmux(&["capture-pane", "-p", "-J", "-t", pane])
            .into_bytes();
        Self {
            bytes,
            chunks,
            stream,
            reader: Some(reader),
        }
    }

    fn wait(&mut self, why: &str, met: impl Fn(&str) -> bool) {
        let deadline = Instant::now() + LIMIT;
        loop {
            let text = String::from_utf8_lossy(&self.bytes);
            if met(&text) {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let chunk = self
                .chunks
                .recv_timeout(remaining)
                .unwrap_or_else(|err| panic!("{why}: {err}; terminal output:\n{text}"));
            self.bytes.extend(chunk);
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
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
        let root = tool
            .dir
            .parent()
            .expect("sessions")
            .parent()
            .expect("root")
            .to_owned();
        let name = tool
            .dir
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
        let meta = format!(
            "session={}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=codex\nlaunch_id.main=tok-rig\nseat.worker.0=colead\nagent_bin.worker.0=codex\nlaunch_id.worker.0=tok-colead\n",
            rig.name,
            rig.socket.display()
        );
        fs::write(rig.tool.dir.join("meta"), meta).expect("lead pair fixture");
        fs::write(rig.root.join("config"), "").expect("isolated config");
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        fs::write(rig.tool.dir.join(".launch-attempt"), epoch.to_string()).expect("launch stamp");
        rig.tmux(&["set-option", "-t", &rig.name, "@ae_session_uuid", UUID]);
        rig.tmux(&["set-option", "-t", &rig.name, "@ae_look", "off"]);
        rig.tmux(&["set-option", "-t", &rig.name, "@ae_motion", "off"]);
        rig.tmux(&[
            "set-option",
            "-t",
            &rig.name,
            "@ae_main_pane",
            &rig.tool.pane,
        ]);
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
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &self.root);
        assert!(ok, "private tmux {tail:?}: {out}");
        out
    }

    fn open(&self) -> String {
        let out = super::cli::ae()
            .env("AE_HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .env("TMUX", format!("{},1,0", self.socket.display()))
            .env("TMUX_PANE", &self.tool.pane)
            .arg("_console")
            .output()
            .expect("real console opens");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
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
            let (pane, stamp) = row.split_once('|')?;
            (stamp == UUID).then(|| pane.to_owned())
        })
        .expect("stamped chat pane")
    }

    fn second(&self) -> String {
        // Stamp before starting input, as the real toggle does. The first
        // console remains owner throughout. Use argv: tmux's printed
        // pane_start_command is not a replayable shell command.
        let target = format!("={}:", self.name);
        let pane = self.tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &target,
            "-n",
            "second",
            "/bin/sh",
        ]);
        let pane = pane.trim().to_owned();
        self.tmux(&["set-option", "-p", "-t", &pane, "@ae_console", UUID]);
        let home = format!("AE_HOME={}", self.root.display());
        let config = format!("CONFIG_FILE={}", self.root.join("config").display());
        let server = format!("AE_TMUX_SERVER={}", self.socket.display());
        self.tmux(&[
            "respawn-pane",
            "-k",
            "-t",
            &pane,
            "/bin/sh",
            "-c",
            "stty -icanon -echo -ixon -iexten min 1 time 0 && exec \"$0\" \"$@\"",
            "env",
            &home,
            &config,
            &server,
            "AE_TMUX_SERVER_KIND=socket",
            env!("CARGO_BIN_EXE_ae"),
            "console",
            &self.name,
            "--follow",
            "--input",
        ]);
        pane
    }

    fn asks(&self) -> Vec<Event> {
        self.tool
            .events()
            .lines()
            .filter_map(|row| Event::parse_line(row).ok())
            .filter(|event| event.actor == "console:local" && event.action == "ask")
            .collect()
    }

    fn draft(&self, bytes: &[u8]) {
        fs::write(self.tool.dir.join("console.draft"), bytes).expect("independent kept draft");
    }

    fn colead(&self) -> PathBuf {
        let received = self.root.join("colead-received");
        let enters = self.root.join("colead-enters");
        fs::write(&received, "").expect("colead receipt");
        fs::write(&enters, "").expect("colead Enter receipt");
        let script = self.root.join("faketui.pl");
        let control = self.root.join("colead-control");
        let target = format!("={}:", self.name);
        let pane = self.tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &target,
            "-n",
            "colead",
            "perl",
            script.to_str().expect("script path"),
            received.to_str().expect("receipt path"),
            enters.to_str().expect("Enter path"),
            "400",
            "codex",
            "0",
            "",
            control.to_str().expect("control path"),
        ]);
        let pane = pane.trim();
        self.tmux(&["set-option", "-p", "-t", pane, "@ae_slot", "worker.0"]);
        self.tmux(&["set-option", "-p", "-t", pane, "@ae_agent", "colead"]);
        received
    }

    fn enter(&self, pane: &str, raw: &str) {
        if !raw.is_empty() {
            self.tmux(&["send-keys", "-t", pane, "-l", "--", raw]);
        }
        self.tmux(&["send-keys", "-t", pane, "Enter"]);
    }

    fn asked(&self, out: &mut Output, body: &str) -> Event {
        out.wait("real console records and confirms the ask", |text| {
            self.asks().iter().any(|ask| {
                ask.summary.as_deref() == Some(body)
                    && ask
                        .reference
                        .as_deref()
                        .is_some_and(|id| text.contains(&format!("sent {id}")))
            })
        });
        self.asks()
            .into_iter()
            .find(|ask| ask.summary.as_deref() == Some(body))
            .expect("observed ask")
    }
}

fn ready(output: &mut Output) {
    output.wait("owner prompt", |text| text.contains("to lead>"));
}

fn banner(text: &str) -> bool {
    text.split(['\r', '\n']).any(|line| {
        line.contains("already")
            && line.contains("sent")
            && line
                .split(|ch: char| !ch.is_alphanumeric())
                .any(|word| word == "lead")
            && line.contains("colead")
            && line.contains("prefix H")
            && line.contains("Enter")
    })
}

#[test]
fn a_kept_draft_is_restored_literally_with_an_all_seats_uncertainty_banner() {
    let rig = Rig::new("p35literal");
    let receipt = rig.colead();
    let raw = b"@colead /close x";
    rig.draft(raw);
    let pane = rig.open();
    let mut out = Output::attach(&rig, &pane, "observe");
    ready(&mut out);
    out.wait("complete restored literal line", |text| {
        text.contains("to lead> @colead /close x")
    });
    let text = out.text();
    assert!(
        text.contains("to lead> @colead /close x"),
        "literal kept line: {text}"
    );
    assert!(
        banner(&text),
        "both seats + uncertainty + check before Enter: {text}"
    );
    assert!(rig.asks().is_empty(), "restore never submits");
    assert_eq!(
        fs::read(rig.tool.dir.join("console.draft")).expect("draft stays"),
        raw
    );
    assert!(
        !rig.tool.events().contains("\"action\":\"cancel\""),
        "restore never closes"
    );
    rig.enter(&pane, "");
    let ask = rig.asked(&mut out, "/close x");
    assert_eq!(ask.target.as_deref(), Some("colead"));
    assert_eq!(ask.target_slot, RoutingMember::Value("worker.0".to_owned()));
    assert!(
        fs::read_to_string(receipt)
            .expect("colead received Enter")
            .contains("/close x")
    );
    assert!(
        !rig.tool.events().contains("\"action\":\"cancel\""),
        "a prefixed close stays a literal ask after Enter"
    );
    assert!(
        !rig.tool.dir.join("console.draft").exists(),
        "confirmed submit clears the restored draft"
    );
}

#[test]
fn a_read_only_console_restores_only_when_it_becomes_the_owner() {
    let rig = Rig::new("p35promote");
    let first = rig.open();
    let mut first_out = Output::attach(&rig, &first, "first");
    ready(&mut first_out);
    rig.draft(b"saved before promotion");
    let second = rig.second();
    let mut out = Output::attach(&rig, &second, "second-out");
    out.wait("another console only reads", |text| {
        text.contains("read-only")
    });
    assert!(
        !out.text().contains("saved before promotion"),
        "read-only never restores"
    );
    rig.tmux(&["kill-pane", "-t", &first]);
    ready(&mut out);
    out.wait("complete restored line after promotion", |text| {
        text.contains("to lead> saved before promotion")
    });
    assert!(
        out.text().contains("to lead> saved before promotion"),
        "promotion restores: {}",
        out.text()
    );
    assert!(
        banner(&out.text()),
        "promotion shows uncertainty banner: {}",
        out.text()
    );
    assert!(rig.asks().is_empty(), "promotion never submits");
}

#[test]
fn an_oversized_kept_draft_is_refused_by_name_and_never_truncated_into_input() {
    let rig = Rig::new("p35oversized");
    rig.draft(&vec![b'x'; 65_537]);
    let pane = rig.open();
    let mut out = Output::attach(&rig, &pane, "observe");
    ready(&mut out);
    let text = out.text();
    assert!(
        text.lines().any(|line| line.contains("draft")
            && line.contains("65536")
            && (line.contains("refused") || line.contains("too"))),
        "named oversized refusal: {text}"
    );
    assert!(!text.contains("to lead> x"), "never truncate draft");
    assert!(rig.asks().is_empty());
}

#[test]
fn a_fifo_kept_draft_is_refused_by_name_without_opening_or_blocking() {
    let rig = Rig::new("p35fifo");
    let path = rig.tool.dir.join("console.draft");
    // Existing child-process door; this FIFO has no writer.
    super::cli::mkfifo(&path);
    let pane = rig.open();
    let mut out = Output::attach(&rig, &pane, "observe");
    ready(&mut out);
    let text = out.text();
    assert!(
        text.lines().any(|line| line.contains("draft")
            && (line.contains("FIFO") || line.contains("fifo") || line.contains("regular file"))
            && (line.contains("refused") || line.contains("not restored"))),
        "named FIFO refusal: {text}"
    );
    assert!(rig.asks().is_empty());
}

#[test]
fn the_real_console_routes_follow_ups_to_the_speaker_and_restarts_at_main() {
    let rig = Rig::new("p35speaker");
    let receipt = rig.colead();
    let first = rig.open();
    let mut out = Output::attach(&rig, &first, "first");
    ready(&mut out);
    rig.enter(&first, "@colead first request");
    rig.asked(&mut out, "first request");
    rig.enter(&first, "sticky follow up");
    let ask = rig.asked(&mut out, "sticky follow up");
    assert_eq!(ask.target.as_deref(), Some("colead"));
    assert_eq!(ask.target_slot, RoutingMember::Value("worker.0".to_owned()));
    assert!(
        fs::read_to_string(receipt)
            .expect("colead receipt")
            .contains("sticky follow up"),
        "sticky route reaches the actual colead pane"
    );
    assert!(
        out.text().contains("to colead>"),
        "sticky prompt: {}",
        out.text()
    );
    rig.tmux(&["kill-pane", "-t", &first]);
    let restarted = rig.open();
    let mut restarted_out = Output::attach(&rig, &restarted, "restarted");
    ready(&mut restarted_out);
    rig.enter(&restarted, "fresh process request");
    let ask = rig.asked(&mut restarted_out, "fresh process request");
    assert_eq!(ask.target.as_deref(), Some("lead"));
    assert_eq!(ask.target_slot, RoutingMember::Value("main".to_owned()));
}

fn pair() -> Vec<String> {
    vec!["lead".to_owned(), "colead".to_owned()]
}

fn owner(now: Instant) -> Input {
    let mut input = Input::new(pair());
    let _ = input.tick(Reading::Owner, now);
    input
}

fn expect_ask(input: &mut Input, bytes: &[u8], now: Instant, seat: &str, body: &str) {
    let got = input.chunk(bytes, now);
    assert!(
        matches!(got.as_slice(), [Effect::Ask { seat: actual, body: text, .. }] if actual == seat && text == body),
        "{got:?}"
    );
}

#[test]
fn speaker_selection_sticks_for_prompt_routing_and_close_until_main_is_addressed() {
    let now = Instant::now();
    let mut input = owner(now);
    expect_ask(&mut input, b"@colead first\r", now, "colead", "first");
    assert_eq!(input.line().as_deref(), Some("to colead> "));
    expect_ask(&mut input, b"follow up\r", now, "colead", "follow up");
    assert_eq!(
        input.chunk(b"/close x\r", now),
        vec![Effect::Close("x".to_owned())]
    );
    assert_eq!(input.line().as_deref(), Some("to colead> "));
    expect_ask(&mut input, b"@lead back\r", now, "lead", "back");
    assert_eq!(input.line().as_deref(), Some("to lead> "));
    expect_ask(
        &mut input,
        b"main follow up\r",
        now,
        "lead",
        "main follow up",
    );
}

#[test]
fn refused_routes_preserve_the_speaker_and_only_a_restart_resets_it() {
    let now = Instant::now();
    let mut input = owner(now);
    expect_ask(&mut input, b"@colead first\r", now, "colead", "first");
    for raw in [
        b"@lead\r".as_slice(),
        b"@missing no\r",
        b"@elsewhere:lead no\r",
        b"/unknown\r",
    ] {
        assert!(matches!(
            input.chunk(raw, now).as_slice(),
            [Effect::Print(_)]
        ));
        assert_eq!(input.line().as_deref(), Some("to colead> "));
    }
    let _ = input.tick(Reading::NotOwner("another console".to_owned()), now);
    let _ = input.tick(Reading::Owner, now);
    assert_eq!(input.line().as_deref(), Some("to colead> "));
    let mut restarted = owner(now);
    assert_eq!(restarted.line().as_deref(), Some("to lead> "));
    expect_ask(
        &mut restarted,
        b"after restart\r",
        now,
        "lead",
        "after restart",
    );
}
