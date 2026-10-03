//! Independent acceptance contract for console input. Only the external tool
//! is simulated: the console, tty, tracked delivery and journal are real.
//! Completed acts print their result after the journal write. Demotion disables
//! bracketed paste before printing its read-only reason; promotion enables it.
//! The visible owner prompt is `to lead> ` in this fixture, after paste enablement.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures own private sockets and inspect their recorded effects"
)]

use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ae::events::{Event, RoutingMember};

const UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const LIMIT: Duration = Duration::from_secs(20);
const OPEN_IDS: [&str; 5] = [
    "ae-20260930T000000Z-00000001",
    "ae-20260930T000000Z-00000002",
    "ae-20260930T000000Z-00000003",
    "ae-20260930T000000Z-00000004",
    "ae-20260930T000000Z-00000005",
];

// Forward terminal output to an in-process bounded wait, rather than polling
// capture-pane or sleeping until an assertion happens to become true.
const OBSERVER: &str = r"use strict;
use warnings;
use IO::Socket::UNIX;
my $socket = IO::Socket::UNIX->new(Peer => $ARGV[0], Type => SOCK_STREAM) or die $!;
binmode(STDIN); binmode($socket); $socket->autoflush(1);
while (sysread(STDIN, my $bytes, 4096)) { print {$socket} $bytes or last; }
";

fn quoted(path: &std::path::Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

fn refusal_names(text: &str, named: &str) -> bool {
    text.split(['\r', '\n'])
        .any(|line| line.contains("refused:") && line.contains(named))
}

struct Output {
    bytes: Vec<u8>,
    chunks: Receiver<Vec<u8>>,
    stream: Option<UnixStream>,
    socket: PathBuf,
    reader: Option<JoinHandle<()>>,
}

impl Output {
    fn attach(rig: &ConsoleRig, pane: &str, name: &str) -> Self {
        let socket = rig.root.join(name);
        let listener = UnixListener::bind(&socket).expect("private output socket");
        let (send, chunks) = mpsc::channel();
        let (connected, connection) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("observer connects");
            stream
                .set_read_timeout(Some(LIMIT))
                .expect("bounded reader");
            if connected
                .send(stream.try_clone().expect("shutdown handle"))
                .is_err()
            {
                return;
            }
            let mut bytes = [0; 4096];
            loop {
                match stream.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => {
                        if send.send(bytes[..count].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(why)
                        if matches!(
                            why.kind(),
                            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                        ) => {}
                    Err(_) => break,
                }
            }
        });
        let mut output = Self {
            bytes: Vec::new(),
            chunks,
            stream: None,
            socket,
            reader: Some(reader),
        };
        let script = rig.root.join("observer.pl");
        fs::write(&script, OBSERVER).expect("external output observer");
        let command = format!("exec perl {} {}", quoted(&script), quoted(&output.socket));
        rig.tmux(&["pipe-pane", "-O", "-t", pane, &command]);
        output.stream = Some(connection.recv_timeout(LIMIT).expect("observer ready"));
        output.bytes = rig.tmux(&["capture-pane", "-p", "-t", pane]).into_bytes();
        output
    }

    fn wait(&mut self, description: &str, mut met: impl FnMut(&str) -> bool) {
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
                .unwrap_or_else(|why| panic!("{description}: {why}; terminal output:\n{text}"));
            self.bytes.extend(chunk);
        }
    }

    fn clear(&mut self) {
        self.bytes.clear();
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ = stream.shutdown(Shutdown::Both);
        } else {
            // Unblock accept even when starting the observer itself failed.
            let _ = UnixStream::connect(&self.socket);
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

struct ConsoleRig {
    tool: super::deliver::Rig,
    root: PathBuf,
    socket: PathBuf,
    name: String,
}

impl ConsoleRig {
    fn new(tag: &str) -> Self {
        let tool = super::deliver::Rig::new(tag, "codex", 0);
        let root = tool
            .dir
            .parent()
            .expect("sessions")
            .parent()
            .expect("root")
            .to_owned();
        let socket = root.join("sock");
        let name = tool
            .dir
            .file_name()
            .expect("session name")
            .to_string_lossy()
            .into_owned();
        let rig = Self {
            tool,
            root,
            socket,
            name,
        };
        let meta = format!(
            "session={}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=codex\nlaunch_id.main=tok-rig\nseat.worker.0=colead\nagent_bin.worker.0=codex\nlaunch_id.worker.0=tok-colead\nseat.spawned.0=scout\n",
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
        args.extend(tail.iter().map(|word| (*word).to_owned()));
        let (ok, output) = super::phase2::run_tmux(&args, &self.root);
        assert!(ok, "private tmux {tail:?}: {output}");
        output
    }

    fn toggle(&self) -> String {
        let output = super::cli::ae()
            .env("AE_HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .env("TMUX", format!("{},1,0", self.socket.display()))
            .env("TMUX_PANE", &self.tool.pane)
            .arg("_console")
            .output()
            .expect("console toggle runs");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let panes = self.tmux(&[
            "list-panes",
            "-s",
            "-t",
            &self.name,
            "-F",
            "#{pane_id}|#{@ae_console}",
        ]);
        panes
            .lines()
            .find_map(|line| {
                let (pane, stamp) = line.split_once('|')?;
                (stamp == UUID).then(|| pane.to_owned())
            })
            .expect("a stamped console")
    }

    fn ready(&self, pane: &str, output: &mut Output) {
        output.wait(
            "console shows its current owner prompt after enabling paste",
            |_| {
                // Read the current screen, not an old prompt in Output's history.
                // tmux 3.4 has no bracket_paste_flag format field.
                self.tmux(&["capture-pane", "-p", "-t", pane])
                    .lines()
                    .rfind(|line| !line.trim().is_empty())
                    // capture-pane may omit the idle prompt's trailing space.
                    .is_some_and(|line| line.trim_start().starts_with("to lead>"))
            },
        );
    }

    fn type_line(&self, pane: &str, line: &str) {
        self.tmux(&["send-keys", "-t", pane, "-l", "--", line]);
        self.tmux(&["send-keys", "-t", pane, "Enter"]);
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

    fn asked(&self, output: &mut Output, body: &str) -> Event {
        output.wait("typed text records a real chat ask", |_| {
            self.asks()
                .iter()
                .any(|event| event.summary.as_deref() == Some(body))
        });
        self.asks()
            .into_iter()
            .find(|event| event.summary.as_deref() == Some(body))
            .expect("observed ask")
    }

    fn seed_five(&self) {
        let mut records = String::new();
        for id in OPEN_IDS {
            writeln!(
                records,
                "{{\"ts\":\"2026-09-30T00:00:00Z\",\"actor\":\"console:local\",\"action\":\"ask\",\"target\":\"lead\",\"ref\":\"{id}\",\"target_slot\":\"main\",\"target_session\":\"{}\",\"summary\":\"open fixture\"}}", self.name
            )
            .expect("fixture record");
        }
        fs::write(self.tool.dir.join("events.jsonl"), records)
            .expect("five independently specified open asks");
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
}

#[test]
fn typed_text_uses_the_real_console_ask_and_marker_path() {
    let rig = ConsoleRig::new("specask");
    let pane = rig.toggle();
    assert_eq!(
        rig.tmux(&["display-message", "-p", "-t", &pane, "#{window_name}"])
            .trim(),
        "chat"
    );
    rig.tmux(&["rename-window", "-t", &pane, "console"]);
    assert_eq!(rig.toggle(), pane, "an old window is found by its stamp");
    assert_eq!(
        rig.tmux(&["display-message", "-p", "-t", &pane, "#{window_name}"])
            .trim(),
        "console",
        "the toggle does not rename an existing window"
    );
    let mut output = Output::attach(&rig, &pane, "w1");
    rig.ready(&pane, &mut output);
    rig.type_line(&pane, "independent typed request");
    let ask = rig.asked(&mut output, "independent typed request");
    assert_eq!(ask.target.as_deref(), Some("lead"));
    assert_eq!(ask.target_slot, RoutingMember::Value("main".to_owned()));
    assert!(
        ask.actor_slot == RoutingMember::Absent,
        "console never impersonates an agent seat"
    );
    let received = rig.tool.submitted();
    assert!(
        received.starts_with("⟦ae:msg from human:chat⟧"),
        "{received}"
    );
    assert!(received.contains("independent typed request"), "{received}");
    assert!(
        received.contains("REQUIRED:") && received.contains(" reply "),
        "{received}"
    );
    let id = ask.reference.as_deref().expect("request id");
    assert!(received.contains(id), "{received}");
    assert!(
        received.contains(&format!("REQUEST {id} from human:chat:")),
        "{received}"
    );
    output.wait("the chat reports the ask outcome after its body", |text| {
        text.find("independent typed request")
            .zip(text.rfind("sent"))
            .is_some_and(|(body, outcome)| body < outcome)
    });
    assert_eq!(rig.asks().len(), 1);
}

#[test]
fn an_explicit_colead_prefix_routes_the_literal_body_to_the_colead_seat() {
    let rig = ConsoleRig::new("speccolead");
    let received = rig.colead();
    let pane = rig.toggle();
    let mut output = Output::attach(&rig, &pane, "w1");
    rig.ready(&pane, &mut output);
    rig.type_line(&pane, "@colead @literal body");
    let ask = rig.asked(&mut output, "@literal body");
    assert_eq!(ask.target.as_deref(), Some("colead"));
    assert_eq!(ask.target_slot, RoutingMember::Value("worker.0".to_owned()));
    let body = fs::read_to_string(received).expect("what colead received");
    assert!(body.starts_with("⟦ae:msg from human:chat⟧"), "{body}");
    assert!(body.contains("@literal body"), "{body}");
    assert!(
        fs::read(rig.root.join("received"))
            .expect("lead receipt")
            .is_empty(),
        "lead received no colead request"
    );
    assert_eq!(rig.asks().len(), 1);
}

#[test]
fn a_prefixed_slash_is_literal_and_only_a_bare_close_withdraws_the_ask() {
    let rig = ConsoleRig::new("specslash");
    let pane = rig.toggle();
    let mut output = Output::attach(&rig, &pane, "w1");
    rig.ready(&pane, &mut output);
    rig.type_line(&pane, "@lead /close literal");
    let ask = rig.asked(&mut output, "/close literal");
    let id = ask.reference.expect("request id");
    assert_eq!(ask.target.as_deref(), Some("lead"));
    assert!(rig.tool.submitted().contains("/close literal"));
    output.clear();
    rig.type_line(&pane, &format!("/close {id}"));
    output.wait("bare close records the chat's scoped withdrawal", |_| {
        rig.tool
            .events()
            .lines()
            .filter_map(|line| Event::parse_line(line).ok())
            .any(|event| {
                event.actor == "console:local"
                    && event.action == "cancel"
                    && event.reference.as_deref() == Some(id.as_str())
            })
    });
    assert_eq!(rig.asks().len(), 1, "close never becomes another ask");
}

#[test]
fn unsupported_routes_and_commands_refuse_before_any_journal_or_draft_write() {
    let rig = ConsoleRig::new("specrefuse");
    let pane = rig.toggle();
    let mut output = Output::attach(&rig, &pane, "w1");
    rig.ready(&pane, &mut output);
    for (line, named) in [
        ("@scout no", "scout"),
        ("@missing no", "missing"),
        ("@elsewhere:lead no", "elsewhere"),
        ("/help", "unknown command"),
        ("/close", "no open ask"),
    ] {
        let before = rig.tool.events();
        output.clear();
        rig.type_line(&pane, line);
        output.wait("refusal names the unusable route or command", |text| {
            refusal_names(text, named)
        });
        assert_eq!(rig.tool.events(), before, "{line}");
        assert!(!rig.tool.dir.join("console.draft").exists(), "{line}");
    }
}

#[test]
fn the_external_tool_fixtures_receive_console_marked_tracked_asks() {
    // Calibration: prove the simulated tool, route and receipt are usable on
    // the baseline even though the new console input path is still RED.
    let rig = ConsoleRig::new("speccontrol");
    let colead = rig.colead();
    let sender = ae::tracked::Sender {
        display: "console:local".to_owned(),
        slot: String::new(),
        session: rig.name.clone(),
    };
    for (target, body, entropy, receipt, slot) in [
        (
            "lead",
            "independent fixture control",
            7,
            rig.root.join("received"),
            "main",
        ),
        (
            "colead",
            "independent colead control",
            8,
            colead,
            "worker.0",
        ),
    ] {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = ae::tracked::run(
            ae::tracked::Kind::Ask,
            &rig.tool.dir,
            &[target.to_owned(), body.to_owned()],
            Some(&sender),
            &rig.name,
            ae::time::Timestamp::now(),
            entropy,
            Duration::ZERO,
            &mut out,
            &mut err,
        )
        .expect("tracked control runs");
        assert_eq!(code, 0, "{target}: {}", String::from_utf8_lossy(&err));
        let received = fs::read_to_string(receipt).expect("submitted control receipt");
        assert!(
            received.starts_with("⟦ae:msg from human:chat⟧"),
            "{received}"
        );
        assert!(received.contains(body), "{received}");
        assert!(rig.asks().iter().any(|ask| {
            ask.target.as_deref() == Some(target)
                && ask.summary.as_deref() == Some(body)
                && ask.target_slot == RoutingMember::Value(slot.to_owned())
        }));
    }
    assert_eq!(rig.asks().len(), 2);
}

#[test]
fn a_sixth_open_request_refuses_and_names_all_five_without_appending() {
    let rig = ConsoleRig::new("speccap");
    rig.seed_five();
    let before = rig.tool.events();
    let pane = rig.toggle();
    let mut output = Output::attach(&rig, &pane, "w1");
    rig.ready(&pane, &mut output);
    output.clear();
    rig.type_line(&pane, "sixth independent request");
    output.wait("cap refusal counts open asks without showing ids", |text| {
        text.contains("5 requests open") && OPEN_IDS.iter().all(|id| !text.contains(id))
    });
    assert_eq!(rig.tool.events(), before);
    assert_eq!(
        fs::read(rig.tool.dir.join("console.draft")).expect("refused draft retained"),
        b"sixth independent request"
    );
}

#[test]
fn a_second_console_is_read_only_and_its_enter_never_opens_a_request() {
    let rig = ConsoleRig::new("specreadonly");
    let first = rig.toggle();
    let mut first_output = Output::attach(&rig, &first, "w1");
    rig.ready(&first, &mut first_output);
    rig.tmux(&["set-option", "-p", "-t", &first, "@ae_console", "foreign"]);
    let second = rig.toggle();
    let mut second_output = Output::attach(&rig, &second, "w2");
    rig.ready(&second, &mut second_output);
    rig.tmux(&["set-option", "-p", "-t", &first, "@ae_console", UUID]);
    second_output.wait("second console names its input owner", |text| {
        text.contains("read-only") && text.contains("input owned by window")
    });
    assert!(
        second_output
            .bytes
            .windows(b"\x1b[?2004l".len())
            .any(|bytes| bytes == b"\x1b[?2004l"),
        "demotion disables bracketed paste before its read-only line"
    );
    rig.type_line(&second, "read-only text must not become an ask");
    rig.ready(&first, &mut first_output);
    rig.type_line(&first, "owner remains usable");
    rig.asked(&mut first_output, "owner remains usable");
    let asks = rig.asks();
    assert_eq!(asks.len(), 1);
    assert_eq!(asks[0].summary.as_deref(), Some("owner remains usable"));
    second_output.clear();
    rig.tmux(&["kill-pane", "-t", &first]);
    rig.ready(&second, &mut second_output);
    second_output.wait("promotion enables bracketed paste again", |text| {
        text.contains("\x1b[?2004h")
    });
    rig.type_line(&second, "fresh request after promotion");
    rig.asked(&mut second_output, "fresh request after promotion");
    let asks = rig.asks();
    assert_eq!(
        asks.len(),
        2,
        "read-only text is never replayed on promotion"
    );
    assert_eq!(
        asks[1].summary.as_deref(),
        Some("fresh request after promotion")
    );
}

#[test]
fn an_overflowing_paste_cannot_turn_its_control_u_and_newlines_into_commands() {
    let rig = ConsoleRig::new("specpaste");
    let pane = rig.toggle();
    let mut output = Output::attach(&rig, &pane, "w1");
    rig.ready(&pane, &mut output);
    let mut paste = vec![b'a'; 65_537];
    paste.extend_from_slice(b"\x15pasted request\nsecond pasted request\n");
    let file = rig.root.join("paste");
    fs::write(&file, paste).expect("oversized paste fixture");
    rig.tmux(&[
        "load-buffer",
        "-b",
        "spec",
        file.to_str().expect("fixture path"),
    ]);
    rig.tmux(&["paste-buffer", "-p", "-r", "-b", "spec", "-t", &pane]);
    output.wait("oversized paste stays a refused draft", |text| {
        text.contains("over 65536")
    });
    assert!(rig.asks().is_empty(), "pasted newlines never submit");
    output.clear();
    rig.tmux(&["send-keys", "-t", &pane, "Enter"]);
    output.wait("outside Enter refuses overflow", |text| {
        refusal_names(text, "65536")
    });
    assert!(rig.asks().is_empty(), "never submit a truncated draft");
    rig.tmux(&["send-keys", "-t", &pane, "C-u"]);
    rig.type_line(&pane, "fresh request after clearing overflow");
    rig.asked(&mut output, "fresh request after clearing overflow");
    assert_eq!(rig.asks().len(), 1);
}

fn shell_literal_sites(dir: &std::path::Path) -> Vec<(PathBuf, usize)> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).expect("source directory") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            found.extend(shell_literal_sites(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let text = fs::read_to_string(&path).expect("source text");
            let production = text.split("\n#[cfg(test)]\n").next().expect("source head");
            let count = production.matches("stty -icanon").count();
            if count > 0 {
                found.push((path, count));
            }
        }
    }
    found
}

#[test]
fn the_fixed_shell_literal_is_present_and_the_opened_pane_uses_it() {
    // The exact literal is a contract pin; the real pane argv below separately
    // pins that the consumer actually uses the authorized wrapper.
    let source = include_str!("../../src/console/toggle.rs");
    let source = source
        .split_once("\n#[cfg(test)]\n")
        .expect("terminal test module")
        .0;
    let literal = r#"stty -icanon -echo -ixon -iexten min 1 time 0 && exec \"$0\" \"$@\""#;
    assert_eq!(
        source.matches(literal).count(),
        1,
        "one exact fixed Rust string literal"
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    assert_eq!(
        shell_literal_sites(&root),
        vec![(root.join("console/toggle.rs"), 1)],
        "one shell-literal site across all production sources"
    );
    let rig = ConsoleRig::new("specwrapper");
    let pane = rig.toggle();
    let command = rig.tmux(&[
        "display-message",
        "-p",
        "-t",
        &pane,
        "#{pane_start_command}",
    ]);
    for part in [
        "/bin/sh",
        "stty -icanon -echo -ixon -iexten min 1 time 0",
        " chat ",
        &rig.name,
        "--follow",
        "--input",
    ] {
        assert!(command.contains(part), "missing {part}: {command}");
    }
}
