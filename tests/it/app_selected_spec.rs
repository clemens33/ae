//! Frozen B2 acceptance: ruling D1-D4/P1-P3 and inherited B1/C1 contracts.
//! Private real apps/chat, target journals, delivered bytes, terminal cells
//! and independent kernel leases. No mocked admission or writing proof.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use crate::app_mode::writing;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ae::events::Event;

const S_ID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const T_ID: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";
const NEW_ID: &str = "0199c0de-dddd-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);
const WRITER: &str = ".console-writer.lock";

struct Hold {
    path: PathBuf,
    trace: PathBuf,
    before: usize,
    release: Option<mpsc::Sender<()>>,
    watcher: Option<std::thread::JoinHandle<()>>,
}

impl Hold {
    fn new(dir: &Path, key: &str) -> Self {
        let path = dir.join(key);
        let trace = dir.join("trace");
        let before = fs::read_to_string(&trace)
            .unwrap_or_default()
            .lines()
            .count();
        fs::write(&path, "hold").expect("private read gate");
        let (release, wait) = mpsc::channel();
        let fallback = path.clone();
        let watcher = std::thread::spawn(move || {
            let _ = wait.recv_timeout(Duration::from_secs(45));
            let _ = fs::remove_file(fallback);
        });
        Self {
            path,
            trace,
            before,
            release: Some(release),
            watcher: Some(watcher),
        }
    }

    fn held(&self) {
        let key = self.path.file_name().expect("gate key").to_string_lossy();
        let until = Instant::now() + WAIT;
        loop {
            let trace = fs::read_to_string(&self.trace).unwrap_or_default();
            if trace
                .lines()
                .skip(self.before)
                .any(|line| line == format!("held {key}"))
            {
                assert!(self.path.exists(), "GUARD attested gate remains held");
                return;
            }
            assert!(
                Instant::now() < until,
                "GUARD {key} did not attest: {trace}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
    }
}

struct Target {
    tool: super::deliver::Rig,
    dir: PathBuf,
    name: String,
    socket: PathBuf,
    pane: String,
}

struct Rig {
    s: Target,
    t: Target,
    root: PathBuf,
}

impl Rig {
    fn new(tag: &str, other_server: bool) -> Self {
        let home = super::deliver::Rig::new(&format!("{tag}s"), "codex", 0);
        let other = super::deliver::Rig::new(&format!("{tag}t"), "codex", 0);
        let root_of = |dir: &Path| {
            dir.parent()
                .expect("sessions")
                .parent()
                .expect("root")
                .to_owned()
        };
        let root = root_of(&home.dir);
        let other_root = root_of(&other.dir);
        let name_of = |dir: &Path| {
            dir.file_name()
                .expect("session")
                .to_string_lossy()
                .into_owned()
        };
        let s_name = name_of(&home.dir);
        let t_name = name_of(&other.dir);
        let socket = root.join("sock");
        let t_socket = if other_server {
            other_root.join("sock")
        } else {
            socket.clone()
        };
        let t_pane = if other_server {
            other.pane.clone()
        } else {
            let command = format!(
                "exec perl {} {} {} 400 codex 0 '' {}",
                other_root.join("faketui.pl").display(),
                other_root.join("received").display(),
                other_root.join("enters").display(),
                other_root.join("control").display()
            );
            let args = [
                "-S",
                socket.to_str().expect("socket"),
                "new-session",
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                "-x",
                "400",
                "-y",
                "40",
                "-s",
                &t_name,
                &command,
            ];
            let (ok, pane) = super::phase2::run_tmux(
                &args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
                &root,
            );
            assert!(ok, "GUARD second same-server fake: {pane}");
            pane.trim().to_owned()
        };
        let t_dir = root.join("sessions").join(&t_name);
        fs::create_dir(&t_dir).expect("second session in same state root");
        let s = Target {
            dir: home.dir.clone(),
            name: s_name,
            socket,
            pane: home.pane.clone(),
            tool: home,
        };
        let t = Target {
            dir: t_dir,
            name: t_name,
            socket: t_socket,
            pane: t_pane,
            tool: other,
        };
        let rig = Self { s, t, root };
        fs::write(
            rig.root.join("config"),
            "[workspace]\nchat = app\nicons = off\n",
        )
        .expect("private config");
        for (target, uuid) in [(&rig.s, S_ID), (&rig.t, T_ID)] {
            rig.prepare(target, uuid);
        }
        rig
    }

    fn prepare(&self, target: &Target, uuid: &str) {
        self.meta(target, uuid, "colead");
        fs::write(target.dir.join("events.jsonl"), "").expect("empty target journal");
        fs::write(
            target.dir.join(".launch-attempt"),
            ae::time::Timestamp::now().epoch().to_string(),
        )
        .expect("launch stamp");
        for (key, value) in [
            ("@ae_session_uuid", uuid),
            ("@ae_look", "off"),
            ("@ae_motion", "off"),
            ("@ae_main_pane", target.pane.as_str()),
            ("default-size", "160x45"),
        ] {
            self.tmux(target, &["set-option", "-t", &target.name, key, value]);
        }
        for (key, value) in [("@ae_slot", "main"), ("@ae_agent", "lead")] {
            self.tmux(
                target,
                &["set-option", "-p", "-t", &target.pane, key, value],
            );
        }
        self.wait(target, &target.pane, "GUARD fake tool is ready", |screen| {
            screen.contains('›')
        });
    }

    fn meta(&self, target: &Target, uuid: &str, peer: &str) {
        assert_eq!(target.dir, self.root.join("sessions").join(&target.name));
        fs::write(target.dir.join("meta"), format!(
            "session={}\nmode=local\nsession_id={uuid}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=codex\nlaunch_id.main=tok-rig\nseat.worker.0={peer}\nagent_bin.worker.0=codex\nlaunch_id.worker.0=tok-peer\n",
            target.name, target.socket.display())).expect("recorded target pair and server");
    }

    fn tmux(&self, host: &Target, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), host.socket.display().to_string()];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &self.root);
        assert!(ok, "private tmux {tail:?}: {out}");
        out
    }

    fn frame(&self, host: &Target, pane: &str) -> String {
        self.tmux(host, &["capture-pane", "-p", "-t", pane])
    }

    /// Two equal complete snapshots after the witness; no assertion in a predicate.
    fn wait(&self, host: &Target, pane: &str, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + WAIT;
        let mut previous = None;
        loop {
            let screen = self.frame(host, pane);
            if met(&screen) {
                if previous.as_ref() == Some(&screen) {
                    return screen;
                }
                previous = Some(screen.clone());
            } else {
                previous = None;
            }
            assert!(Instant::now() < until, "{why}; screen:\n{screen}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn app(&self, label: &str, home: bool) -> (String, PathBuf) {
        self.app_count(label, home, 2)
    }

    fn app_count(&self, label: &str, home: bool, count: usize) -> (String, PathBuf) {
        let gates = self.root.join(format!("gates-{label}"));
        fs::create_dir(&gates).expect("app read gates");
        let mut args = vec![
            "new-window".to_owned(),
            "-d".into(),
            "-P".into(),
            "-F".into(),
            "#{pane_id}".into(),
            "-t".into(),
            self.s.name.clone(),
            "-n".into(),
            label.into(),
            "env".into(),
        ];
        for key in ["TMUX", "TMUX_PANE", "CLAUDE_CONFIG_DIR", "CODEX_HOME"] {
            args.extend(["-u".into(), key.into()]);
        }
        args.extend([
            format!("AE_HOME={}", self.root.display()),
            format!("HOME={}", self.root.display()),
            format!("CONFIG_FILE={}", self.root.join("config").display()),
            "AE_TMUX_SERVER_KIND=socket".into(),
            format!("AE_TMUX_SERVER={}", self.s.socket.display()),
            format!("AE_TEST_APP_READ_GATE={}", gates.display()),
            env!("CARGO_BIN_EXE_ae").into(),
            "app".into(),
        ]);
        if home {
            args.push(self.s.name.clone());
        }
        let pane = self
            .tmux(
                &self.s,
                &args.iter().map(String::as_str).collect::<Vec<_>>(),
            )
            .trim()
            .to_owned();
        assert!(pane.starts_with('%'), "GUARD actual app PTY: {pane}");
        self.tmux(
            &self.s,
            &["resize-window", "-t", &pane, "-x", "160", "-y", "45"],
        );
        let sessions = format!("Sessions {count}");
        self.wait(&self.s, &pane, "GUARD target rows loaded", |screen| {
            screen.contains(&sessions)
                && (count == 0 || (screen.contains("Overview") && screen.contains(&self.t.name)))
        });
        (pane, gates)
    }

    fn raw(&self, host: &Target, pane: &str, text: &str) {
        self.tmux(host, &["send-keys", "-t", pane, "-l", "--", text]);
    }

    fn address(&self, target: &Target, home: bool, speaker: &str) -> String {
        assert!(
            !home || target.name == self.s.name,
            "only S is this app's home"
        );
        let suffix = if home { "" } else { " (not home)" };
        format!("to {}{suffix} › {speaker}", target.name)
    }

    fn select(&self, pane: &str, target: &Target) -> String {
        let screen = self.wait(&self.s, pane, "GUARD complete target card", |screen| {
            screen.lines().any(|row| {
                row.chars()
                    .take(44)
                    .collect::<String>()
                    .contains(&target.name)
            })
        });
        let (x, y) = card(&screen, &target.name);
        self.raw(&self.s, pane, &format!("\x1b[<0;{};{}M", x + 1, y + 1));
        self.wait(&self.s, pane, "GUARD selected target header", |screen| {
            screen
                .lines()
                .nth(1)
                .is_some_and(|row| row.contains(&target.name))
        })
    }

    fn enter(&self, pane: &str, target: &Target, home: bool) -> String {
        let address = self.address(target, home, "lead");
        let screen = self.wait(&self.s, pane, "B2 selected target is eligible", |s| {
            s.contains(&address) && s.contains("Enter writes")
        });
        assert_eq!(cursor(self, &self.s, pane).0, 0, "browse cursor hidden");
        assert!(
            screen
                .lines()
                .nth(1)
                .is_some_and(|row| row.contains(&target.name)),
            "target header"
        );
        self.raw(&self.s, pane, "i");
        let screen = self.wait(&self.s, pane, "B2 selected write lease acquired", |s| {
            writing(s) && s.contains(&address)
        });
        held(&target.dir);
        assert_eq!(cursor(self, &self.s, pane).0, 1, "C1 writing cursor shown");
        screen
    }

    fn typed(&self, pane: &str, text: &str) -> String {
        self.raw(&self.s, pane, text);
        self.wait(
            &self.s,
            pane,
            &format!("GUARD fresh draft drawn: {text}"),
            |s| composer_has(s, text) && writing(s),
        )
    }

    fn escape(&self, pane: &str, target: &Target) {
        self.raw(&self.s, pane, "\x1b");
        self.wait(&self.s, pane, "GUARD Esc compacts browse", |s| {
            s.contains("Enter writes") && !writing(s)
        });
        free(&target.dir);
    }

    fn events(&self, target: &Target) -> Vec<Event> {
        assert_eq!(target.dir, self.root.join("sessions").join(&target.name));
        fs::read_to_string(target.dir.join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|row| Event::parse_line(row).expect("fixture event"))
            .collect()
    }

    fn asks(&self, target: &Target) -> Vec<Event> {
        self.events(target)
            .into_iter()
            .filter(|e| e.actor == "console:local" && e.action == "ask")
            .collect()
    }

    /// Poll a journal that may still have a partial tail; assertions use `events()`.
    fn event_count(target: &Target, action: &str) -> usize {
        fs::read_to_string(target.dir.join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|row| Event::parse_line(row).ok())
            .filter(|e| e.actor == "console:local" && e.action == action)
            .count()
    }

    fn send(&self, pane: &str, target: &Target, text: &str, count: usize) -> String {
        self.typed(pane, text);
        self.raw(&self.s, pane, "\r");
        self.wait(
            &self.s,
            pane,
            "B2 literal ask submitted in selected journal",
            |s| s.contains("sent") && Self::event_count(target, "ask") == count,
        );
        let asks = self.asks(target);
        let ask = asks.last().expect("new target ask");
        assert_eq!(ask.actor, "console:local");
        assert_eq!(ask.target.as_deref(), Some("lead"));
        assert_eq!(ask.summary.as_deref(), Some(text));
        let id = ask.reference.as_ref().expect("request id").clone();
        let body = fs::read_to_string(ask.body_file.as_deref().expect("stored body path"))
            .expect("stored tracked body");
        assert!(
            body.contains(&format!("from human:chat: {text}\n\nREQUIRED:")),
            "literal human body"
        );
        let until = Instant::now() + WAIT;
        while !target.tool.submitted().contains(text) {
            assert!(
                Instant::now() < until,
                "actual target fake received and submitted: {text}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        id
    }

    fn command(&self, pane: &str, command: &str, witness: &str, finished: impl Fn(&str) -> bool) {
        self.raw(&self.s, pane, "\x15");
        self.wait(&self.s, pane, "GUARD previous draft cleared", |s| {
            writing(s)
                && self
                    .tmux(
                        &self.s,
                        &["display-message", "-p", "-t", pane, "#{cursor_y}"],
                    )
                    .trim()
                    .parse::<usize>()
                    .ok()
                    == Some(41)
        });
        self.typed(pane, command);
        self.raw(&self.s, pane, "\r");
        self.wait(&self.s, pane, "target command outcome", |s| {
            s.contains(witness) && !composer_has(s, command) && finished(s)
        });
    }

    fn chat(&self, target: &Target) -> String {
        fs::write(
            self.root.join("config"),
            "[workspace]\nchat = chat\nicons = off\n",
        )
        .expect("owner chat config");
        for (key, value) in [
            ("HOME", self.root.display().to_string()),
            ("AE_HOME", self.root.display().to_string()),
            (
                "CONFIG_FILE",
                self.root.join("config").display().to_string(),
            ),
        ] {
            self.tmux(
                target,
                &["set-environment", "-t", &target.name, key, &value],
            );
        }
        for key in ["CLAUDE_CONFIG_DIR", "CODEX_HOME"] {
            self.tmux(target, &["set-environment", "-r", "-t", &target.name, key]);
        }
        let out = super::cli::ae()
            .env("AE_HOME", &self.root)
            .env("HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &target.socket)
            .env("TMUX", format!("{},1,0", target.socket.display()))
            .env("TMUX_PANE", &target.pane)
            .arg("_console")
            .output()
            .expect("private owner chat");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let uuid = if target.name == self.s.name {
            S_ID
        } else {
            T_ID
        };
        let pane = self
            .tmux(
                target,
                &[
                    "list-panes",
                    "-s",
                    "-t",
                    &target.name,
                    "-F",
                    "#{pane_id}|#{@ae_console}",
                ],
            )
            .lines()
            .find_map(|line| {
                let (pane, stamp) = line.split_once('|')?;
                (stamp == uuid).then(|| pane.to_owned())
            })
            .expect("stamped owner chat");
        self.wait(target, &pane, "GUARD owner chat accepts input", |s| {
            s.contains("to lead>")
        });
        pane
    }

    fn no_writes(&self, target: &Target) {
        assert!(self.asks(target).is_empty(), "no target ask");
        assert!(
            !target.dir.join("console.draft").exists(),
            "no disk draft published"
        );
        assert!(
            !target.dir.join("messages").exists(),
            "no body directory written"
        );
    }
}

fn card(screen: &str, name: &str) -> (usize, usize) {
    screen
        .lines()
        .enumerate()
        .find_map(|(y, row)| {
            let side = row.chars().take(44).collect::<String>();
            side.find(name)
                .map(|byte| (side[..byte].chars().count(), y))
        })
        .expect("drawn target card")
}

fn composer_has(screen: &str, text: &str) -> bool {
    let Some(start) = screen.lines().enumerate().find_map(|(row, line)| {
        (row >= 32 && line.chars().skip(47).collect::<String>().starts_with("to ")).then_some(row)
    }) else {
        return false;
    };
    screen
        .lines()
        .skip(start)
        .take(42 - start)
        .any(|row| row.chars().skip(47).collect::<String>().contains(text))
}

fn cancelled(rig: &Rig, target: &Target) -> Vec<Event> {
    rig.events(target)
        .into_iter()
        .filter(|e| e.actor == "console:local" && e.action == "cancel")
        .collect()
}

fn cursor(rig: &Rig, host: &Target, pane: &str) -> (usize, usize, usize) {
    let raw = rig.tmux(
        host,
        &[
            "display-message",
            "-p",
            "-t",
            pane,
            "#{cursor_flag},#{cursor_x},#{cursor_y}",
        ],
    );
    let fields = raw
        .trim()
        .split(',')
        .map(|n| n.parse::<usize>().expect("terminal cursor number"))
        .collect::<Vec<_>>();
    assert_eq!(fields.len(), 3);
    (fields[0], fields[1], fields[2])
}

fn held(dir: &Path) {
    let file = OpenOptions::new()
        .append(true)
        .open(dir.join(WRITER))
        .expect("writer created regular lease node");
    assert!(
        matches!(file.try_lock(), Err(TryLockError::WouldBlock)),
        "actual target lease held"
    );
}

fn lease(dir: &Path) -> File {
    let file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(dir.join(WRITER))
        .expect("regular fixture witness");
    file.try_lock().expect("kernel lease independently free");
    file
}

fn free(dir: &Path) {
    drop(lease(dir));
}

#[test]
fn home_control_preserves_the_original_address_journal_and_terminal_cursor() {
    let rig = Rig::new("bscontrol", false);
    let (pane, _) = rig.app("home-control", true);
    let frame = rig.wait(&rig.s, &pane, "home control address", |s| {
        s.contains(&rig.address(&rig.s, true, "lead")) && s.contains("Enter writes")
    });
    assert!(!frame.contains("(not home)"));
    rig.raw(&rig.s, &pane, "i");
    rig.wait(&rig.s, &pane, "home control lease", writing);
    held(&rig.s.dir);
    rig.send(&pane, &rig.s, "unchanged-home-control", 1);
    rig.no_writes(&rig.t);
    rig.escape(&pane, &rig.s);
    assert_eq!(cursor(&rig, &rig.s, &pane).0, 0);
}

#[test]
fn non_home_same_server_writes_only_selected_journal_and_real_target() {
    let rig = Rig::new("bsselect", false);
    let (pane, _) = rig.app("select", true);
    rig.select(&pane, &rig.t);
    rig.enter(&pane, &rig.t, false);
    rig.send(&pane, &rig.t, "literal-target-only", 1);
    rig.no_writes(&rig.s);
    assert!(fs::symlink_metadata(rig.t.dir.join(WRITER)).is_ok_and(|m| m.file_type().is_file()));
    rig.escape(&pane, &rig.t);
    rig.select(&pane, &rig.s);
    let frame = rig.frame(&rig.s, &pane);
    assert!(
        frame.contains(&rig.address(&rig.s, true, "lead")),
        "home address unchanged"
    );
    assert!(
        !frame.contains("literal-target-only"),
        "never another target lane"
    );
}

#[test]
fn selected_recorded_second_server_receives_literal_console_local_ask() {
    let rig = Rig::new("bsserver", true);
    let (pane, _) = rig.app("second-server", true);
    rig.select(&pane, &rig.t);
    rig.enter(&pane, &rig.t, false);
    rig.send(&pane, &rig.t, "on-the-recorded-second-socket", 1);
    rig.no_writes(&rig.s);
    assert_ne!(
        rig.s.socket, rig.t.socket,
        "GUARD genuinely distinct servers"
    );
}

#[test]
fn outside_tmux_without_home_writes_the_selected_running_session() {
    let rig = Rig::new("bsoutside", false);
    let (pane, _) = rig.app("no-home", false);
    let frame = rig.wait(&rig.s, &pane, "B2 no-home first selected address", |s| {
        s.contains(&rig.address(&rig.s, false, "lead")) && s.contains("Enter writes")
    });
    assert!(
        frame
            .lines()
            .nth(1)
            .is_some_and(|row| row.contains(&rig.s.name))
    );
    rig.enter(&pane, &rig.s, false);
    rig.send(&pane, &rig.s, "no-home-is-needed", 1);
    rig.no_writes(&rig.t);
}

#[test]
fn card_switch_releases_old_target_before_new_entry_and_keeps_both_drafts_and_speakers() {
    let rig = Rig::new("bspark", false);
    let peer_command = format!(
        "exec perl {} {} {} 400 codex 0 '' {}",
        rig.root.join("faketui.pl").display(),
        rig.root.join("peer-received").display(),
        rig.root.join("peer-enters").display(),
        rig.root.join("peer-control").display()
    );
    let peer = rig
        .tmux(
            &rig.s,
            &[
                "new-window",
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                "-t",
                &rig.s.name,
                &peer_command,
            ],
        )
        .trim()
        .to_owned();
    for (key, value) in [("@ae_slot", "worker.0"), ("@ae_agent", "colead")] {
        rig.tmux(&rig.s, &["set-option", "-p", "-t", &peer, key, value]);
    }
    rig.wait(&rig.s, &peer, "GUARD peer fake ready", |s| s.contains('›'));
    let (pane, _) = rig.app("parked", true);
    rig.enter(&pane, &rig.s, true);
    rig.typed(&pane, "@colead choose-speaker");
    rig.raw(&rig.s, &pane, "\r");
    rig.wait(
        &rig.s,
        &pane,
        "GUARD S addressed ask selects its speaker",
        |s| s.contains("› colead") && s.contains("sent") && Rig::event_count(&rig.s, "ask") == 1,
    );
    assert_eq!(rig.asks(&rig.s)[0].target.as_deref(), Some("colead"));
    assert_eq!(
        rig.asks(&rig.s)[0].summary.as_deref(),
        Some("choose-speaker")
    );
    assert!(
        fs::read_to_string(rig.root.join("peer-received"))
            .expect("peer receipt")
            .contains("choose-speaker")
    );
    rig.typed(&pane, "s-private-prose");
    let t = rig.select(&pane, &rig.t);
    free(&rig.s.dir);
    assert!(
        !t.contains("s-private-prose"),
        "S draft not re-addressed into T"
    );
    rig.enter(&pane, &rig.t, false);
    let s_witness = lease(&rig.s.dir);
    rig.typed(&pane, "t-private-prose");
    let s = rig.select(&pane, &rig.s);
    free(&rig.t.dir);
    assert!(
        s.contains(&rig.address(&rig.s, true, "colead")) && s.contains("s-private-prose"),
        "S parked speaker and draft restored"
    );
    assert!(!s.contains("t-private-prose"));
    drop(s_witness);
    rig.raw(&rig.s, &pane, "i");
    rig.wait(&rig.s, &pane, "B2 parked S re-entry", |s| {
        writing(s) && s.contains("› colead") && s.contains("s-private-prose")
    });
    held(&rig.s.dir);
    let t = rig.select(&pane, &rig.t);
    free(&rig.s.dir);
    assert!(t.contains(&rig.address(&rig.t, false, "lead")) && t.contains("t-private-prose"));
    assert_eq!(
        rig.asks(&rig.s).len(),
        1,
        "parking never submits either draft"
    );
    rig.no_writes(&rig.t);
}

#[test]
fn writing_keys_and_list_wheel_cannot_readdress_a_target() {
    let rig = Rig::new("bskeys", false);
    let (pane, _) = rig.app("keys", true);
    rig.select(&pane, &rig.t);
    rig.enter(&pane, &rig.t, false);
    rig.typed(&pane, "j!1-q");
    let screen = rig.frame(&rig.s, &pane);
    let (x, y) = card(&screen, &rig.s.name);
    rig.raw(&rig.s, &pane, "\x1b[B");
    for code in [64, 65] {
        rig.raw(&rig.s, &pane, &format!("\x1b[<{code};{};{}M", x + 1, y + 1));
    }
    rig.typed(&pane, "-after-wheel");
    let frame = rig.frame(&rig.s, &pane);
    assert!(
        frame
            .lines()
            .nth(1)
            .is_some_and(|row| row.contains(&rig.t.name))
    );
    assert!(frame.contains("j!1-q-after-wheel") && writing(&frame));
    held(&rig.t.dir);
    free(&rig.s.dir);
    rig.no_writes(&rig.s);
    rig.no_writes(&rig.t);
}

#[test]
fn acquisition_restores_each_targets_own_disk_only_into_its_empty_memory() {
    let rig = Rig::new("bsrestore", false);
    fs::write(rig.s.dir.join("console.draft"), "s-on-disk").expect("S kept disk");
    fs::write(rig.t.dir.join("console.draft"), "t-on-disk").expect("T kept disk");
    let (pane, _) = rig.app("restore", true);
    rig.select(&pane, &rig.t);
    let frame = rig.enter(&pane, &rig.t, false);
    assert!(frame.contains("t-on-disk") && !frame.contains("s-on-disk"));
    assert!(frame.contains("Kept line, maybe already sent"));
    rig.typed(&pane, "-in-memory");
    rig.select(&pane, &rig.s);
    free(&rig.t.dir);
    let frame = rig.enter(&pane, &rig.s, true);
    assert!(frame.contains("s-on-disk") && !frame.contains("t-on-disk"));
    rig.select(&pane, &rig.t);
    free(&rig.s.dir);
    fs::write(rig.t.dir.join("console.draft"), "new-t-disk").expect("new external disk bytes");
    let frame = rig.enter(&pane, &rig.t, false);
    assert!(
        frame.contains("t-on-disk-in-memory") && !frame.contains("new-t-disk"),
        "memory wins over target disk"
    );
    rig.raw(&rig.s, &pane, "\x15");
    rig.wait(&rig.s, &pane, "GUARD target memory cleared", |s| {
        !s.contains("t-on-disk-in-memory") && writing(s)
    });
    rig.escape(&pane, &rig.t);
    let frame = rig.enter(&pane, &rig.t, false);
    assert!(
        frame.contains("new-t-disk"),
        "fresh empty acquisition restores T only"
    );
    assert!(rig.asks(&rig.s).is_empty() && rig.asks(&rig.t).is_empty());
}

#[test]
fn per_target_five_open_cap_and_both_close_forms_touch_only_that_journal() {
    let rig = Rig::new("bscap", false);
    let (pane, _) = rig.app("cap", true);
    rig.enter(&pane, &rig.s, true);
    let s_id = rig.send(&pane, &rig.s, "s-open", 1);
    rig.select(&pane, &rig.t);
    free(&rig.s.dir);
    rig.enter(&pane, &rig.t, false);
    let mut t_ids = Vec::new();
    for n in 0..5 {
        t_ids.push(rig.send(&pane, &rig.t, &format!("t-open-{n}"), n + 1));
    }
    rig.typed(&pane, "t-over-cap");
    rig.raw(&rig.s, &pane, "\r");
    rig.wait(&rig.s, &pane, "B2 T sixth ask refused", |s| {
        s.contains("refused") && s.contains("5 requests open")
    });
    assert_eq!(rig.asks(&rig.t).len(), 5, "S count does not enter T cap");
    assert_eq!(rig.asks(&rig.s).len(), 1);
    rig.select(&pane, &rig.s);
    free(&rig.t.dir);
    rig.enter(&pane, &rig.s, true);
    rig.send(&pane, &rig.s, "s-still-admits-while-t-is-full", 2);
    assert_eq!(rig.asks(&rig.t).len(), 5);
    rig.select(&pane, &rig.t);
    free(&rig.s.dir);
    rig.enter(&pane, &rig.t, false);
    rig.command(&pane, "/close", "closed an ask", |_| {
        Rig::event_count(&rig.t, "cancel") == 1
    });
    let closed = cancelled(&rig, &rig.t);
    assert_eq!(closed.len(), 1);
    assert_eq!(
        closed[0].reference.as_deref(),
        t_ids.last().map(String::as_str),
        "newest T only"
    );
    assert!(rig.events(&rig.s).iter().all(|e| e.action != "cancel"));
    rig.command(
        &pane,
        &format!("/close {}", t_ids[0]),
        "closed an ask",
        |_| Rig::event_count(&rig.t, "cancel") == 2,
    );
    let closed = cancelled(&rig, &rig.t);
    assert_eq!(closed.len(), 2);
    assert_eq!(closed[1].reference.as_deref(), Some(t_ids[0].as_str()));
    let before = rig.wait(&rig.s, &pane, "GUARD T close completed", |s| {
        s.contains("closed an ask") && writing(s)
    });
    let refused_before = before.matches("refused:").count();
    rig.command(&pane, &format!("/close {s_id}"), "refused", |s| {
        s.matches("refused:").count() > refused_before && Rig::event_count(&rig.t, "cancel") == 2
    });
    assert_eq!(
        rig.events(&rig.t)
            .iter()
            .filter(|e| e.action == "cancel")
            .count(),
        2,
        "cannot close S id from T"
    );
    assert!(rig.events(&rig.s).iter().all(|e| e.action != "cancel"));
    assert_eq!(rig.asks(&rig.t).len(), 5);
}

#[test]
fn apps_on_distinct_targets_hold_distinct_leases_and_only_target_chat_is_excluded() {
    let rig = Rig::new("bsparallel", false);
    let s_chat = rig.chat(&rig.s);
    let t_chat = rig.chat(&rig.t);
    let (p, _) = rig.app("app-s", true);
    let (q, _) = rig.app("app-t", true);
    rig.select(&q, &rig.t);
    rig.enter(&q, &rig.t, false);
    rig.raw(&rig.t, &t_chat, "t-chat-busy\r");
    rig.wait(
        &rig.t,
        &t_chat,
        "B2 T owner chat refused without waiting",
        |s| s.contains("an ae app is writing to"),
    );
    assert!(rig.asks(&rig.t).is_empty());
    assert!(!rig.t.dir.join("console.draft").exists());
    rig.raw(&rig.s, &s_chat, "s-chat-free\r");
    rig.wait(&rig.s, &s_chat, "B2 unrelated S chat still submits", |s| {
        s.contains("sent") && Rig::event_count(&rig.s, "ask") == 1
    });
    assert!(rig.s.tool.submitted().contains("s-chat-free"));
    rig.enter(&p, &rig.s, true);
    held(&rig.s.dir);
    held(&rig.t.dir);
    rig.send(&p, &rig.s, "s-app", 2);
    rig.send(&q, &rig.t, "t-app", 1);
    assert_eq!(
        rig.asks(&rig.s)
            .iter()
            .map(|e| e.summary.as_deref().expect("summary"))
            .collect::<Vec<_>>(),
        ["s-chat-free", "s-app"]
    );
    assert_eq!(rig.asks(&rig.t)[0].summary.as_deref(), Some("t-app"));
    rig.escape(&q, &rig.t);
    held(&rig.s.dir);
    rig.raw(&rig.s, &s_chat, "s-chat-busy\r");
    rig.wait(
        &rig.s,
        &s_chat,
        "B2 S owner chat refuses while T is free",
        |s| s.contains("an ae app is writing to"),
    );
    assert_eq!(rig.asks(&rig.s).len(), 2, "busy S chat adds no ask");
    assert!(!rig.s.dir.join("console.draft").exists());
    rig.raw(&rig.t, &t_chat, "\x15/close\r");
    rig.wait(&rig.t, &t_chat, "T chat closes after T app releases", |s| {
        s.contains("closed an ask")
    });
    assert!(rig.events(&rig.s).iter().all(|e| e.action != "cancel"));
}

#[test]
fn changed_pair_refuses_retry_and_reentry_and_original_pair_recovers_its_kept_prose() {
    let rig = Rig::new("bspair", false);
    let (pane, gates) = rig.app("pair", true);
    rig.select(&pane, &rig.t);
    rig.enter(&pane, &rig.t, false);
    rig.typed(&pane, "parked-original-pair");
    rig.escape(&pane, &rig.t);
    let world = Hold::new(&gates, "@world");
    world.held();
    rig.meta(&rig.t, T_ID, "different-peer");
    rig.raw(&rig.s, &pane, "i");
    let refused = rig.wait(&rig.s, &pane, "P1 changed pair refuses fresh entry", |s| {
        s.contains("not writing:") && s.contains("pair") && s.contains("restart")
    });
    assert!(!writing(&refused));
    free(&rig.t.dir);
    rig.raw(&rig.s, &pane, "q\r");
    rig.wait(
        &rig.s,
        &pane,
        "P1 held retry still refuses original pair",
        |s| s.contains("not writing:") && s.contains("restart"),
    );
    rig.raw(&rig.s, &pane, "\x1b");
    drop(world);
    let browse = rig.wait(&rig.s, &pane, "P1 changed pair remains read-only", |s| {
        s.contains("read-only ·") && s.contains("restart")
    });
    assert!(browse.contains("pair"));
    rig.select(&pane, &rig.s);
    let parked = rig.select(&pane, &rig.t);
    assert!(parked.contains("read-only ·") && parked.contains("restart"));
    rig.raw(&rig.s, &pane, "i");
    rig.raw(&rig.s, &pane, "\x1b[<0;160;45M");
    rig.wait(
        &rig.s,
        &pane,
        "GUARD re-entry keys processed before gear",
        |s| s.contains("Settings") && s.contains("Quota") && s.contains("About"),
    );
    free(&rig.t.dir);
    assert_eq!(cursor(&rig, &rig.s, &pane).0, 0);
    rig.raw(&rig.s, &pane, "\x1b");
    rig.wait(&rig.s, &pane, "P1 re-entry cannot rebind", |s| {
        s.contains("read-only ·") && s.contains("restart")
    });
    rig.no_writes(&rig.s);
    rig.no_writes(&rig.t);
    // Restore the original pair: its Input must still hold the original prose.
    rig.meta(&rig.t, T_ID, "colead");
    rig.wait(&rig.s, &pane, "P1 original pair readable again", |s| {
        s.contains("Enter writes")
    });
    rig.raw(&rig.s, &pane, "\r");
    let restored = rig.wait(&rig.s, &pane, "P1 original pair remains bound", |s| {
        writing(s) && s.contains("parked-original-pair")
    });
    assert!(restored.contains(&rig.address(&rig.t, false, "lead")));
    assert!(
        !restored.contains("parked-original-pairq"),
        "held q stayed swallowed"
    );
}

#[test]
fn home_and_non_home_uuid_replacement_drop_old_memory_once_after_positive_fleet_read() {
    for home in [true, false] {
        let rig = Rig::new(
            if home {
                "bsreplacehome"
            } else {
                "bsreplacetarget"
            },
            false,
        );
        let (pane, gates) = rig.app("replace", true);
        let target = if home { &rig.s } else { &rig.t };
        if !home {
            rig.select(&pane, target);
        }
        rig.enter(&pane, target, home);
        rig.typed(&pane, "old-incarnation-prose");
        let world = Hold::new(&gates, "@world");
        world.held();
        rig.meta(target, NEW_ID, "colead");
        rig.tmux(
            target,
            &["set-option", "-t", &target.name, "@ae_session_uuid", NEW_ID],
        );
        // Admission has fresh UUID proof but no fleet absorption yet.
        let notice = format!("draft for {} dropped", target.name);
        rig.raw(&rig.s, &pane, "\r");
        rig.wait(&rig.s, &pane, "GUARD UUID admission refused", |s| {
            s.contains("not writing:") && s.contains("refused") && !s.contains(&notice)
        });
        free(&target.dir);
        rig.no_writes(target);
        drop(world);
        rig.wait(
            &rig.s,
            &pane,
            "D4 positive fleet read drops old incarnation",
            |s| s.contains(&notice),
        );
        rig.raw(&rig.s, &pane, "\x1b");
        rig.wait(&rig.s, &pane, "D4 new incarnation browses eligible", |s| {
            s.contains("Enter writes")
        });
        let entered = rig.enter(&pane, target, home);
        assert!(
            !entered.contains("old-incarnation-prose"),
            "old draft never inherited"
        );
        rig.send(&pane, target, "only-new-incarnation", 1);
        let frame = rig.wait(&rig.s, &pane, "D4 one retained target notice", |s| {
            s.contains("sent") && s.matches(&notice).count() == 1
        });
        assert_eq!(
            frame.matches(&notice).count(),
            1,
            "one notice per dropped identity"
        );
        assert_eq!(
            rig.asks(target)[0].summary.as_deref(),
            Some("only-new-incarnation")
        );
        let other = if home { &rig.t } else { &rig.s };
        rig.select(&pane, other);
        let elsewhere = rig.wait(&rig.s, &pane, "D4 other target lane loaded", |s| {
            s.contains(&rig.address(other, !home, "lead")) && s.contains("Enter writes")
        });
        assert!(
            !elsewhere.contains(&notice),
            "a listed target's notice belongs only to its own lane"
        );
        rig.select(&pane, target);
        let returned = rig.wait(&rig.s, &pane, "D4 target notice returns in its lane", |s| {
            s.contains(&rig.address(target, home, "lead")) && s.matches(&notice).count() == 1
        });
        assert_eq!(
            returned.matches(&notice).count(),
            1,
            "name-keyed notice retained once"
        );
        rig.no_writes(other);
    }
}

#[test]
fn transient_unreadable_uuid_preserves_a_parked_incarnation_draft() {
    let rig = Rig::new("bsunknown", false);
    let (pane, _) = rig.app("unknown", true);
    rig.select(&pane, &rig.t);
    rig.enter(&pane, &rig.t, false);
    rig.typed(&pane, "not-lost-on-unknown");
    rig.escape(&pane, &rig.t);
    let meta = fs::read_to_string(rig.t.dir.join("meta")).expect("original meta");
    fs::write(
        rig.t.dir.join("meta"),
        meta.replace(&format!("session_id={T_ID}"), "session_id=unreadable"),
    )
    .expect("transient unknown id");
    let refused = rig.wait(
        &rig.s,
        &pane,
        "GUARD unreadable target positively seen",
        |s| s.contains("read-only ·") && s.contains("session_id"),
    );
    assert!(
        !refused.contains("draft for"),
        "Unknown is not positive replacement"
    );
    fs::write(rig.t.dir.join("meta"), meta).expect("original identity readable again");
    rig.wait(&rig.s, &pane, "GUARD original UUID eligible again", |s| {
        s.contains("Enter writes")
    });
    let entered = rig.enter(&pane, &rig.t, false);
    assert!(entered.contains("not-lost-on-unknown"));
    rig.no_writes(&rig.s);
    rig.no_writes(&rig.t);
}

#[test]
fn stopped_missing_id_and_unreadable_pair_refuse_selected_writes_without_any_disk_effect() {
    for case in 0..3 {
        let rig = Rig::new(&format!("bsrefuse{case}"), false);
        match case {
            0 => {
                rig.tmux(&rig.t, &["kill-session", "-t", &rig.t.name]);
            }
            1 => {
                let meta = fs::read_to_string(rig.t.dir.join("meta")).expect("meta");
                fs::write(
                    rig.t.dir.join("meta"),
                    meta.replace(&format!("session_id={T_ID}\n"), ""),
                )
                .expect("no recorded id");
            }
            _ => {
                let meta = fs::read_to_string(rig.t.dir.join("meta")).expect("meta");
                fs::write(
                    rig.t.dir.join("meta"),
                    meta.replace("seat.main=lead", "seat.main=%3"),
                )
                .expect("unreadable pair");
            }
        }
        let (pane, _) = rig.app("refused", true);
        let screen = rig.select(&pane, &rig.t);
        assert!(screen.contains("read-only ·"));
        let stopped = format!("{} is stopped", rig.t.name);
        let reason = match case {
            0 => stopped.as_str(),
            1 => "session_id",
            _ => "main seat",
        };
        assert!(
            screen.contains(reason),
            "D3 reason names target failure: {screen}"
        );
        rig.raw(&rig.s, &pane, "iBAD\r\x1b[<0;160;45M");
        rig.wait(
            &rig.s,
            &pane,
            "GUARD refused-entry keys processed before gear",
            |s| s.contains("Settings") && s.contains("Quota") && s.contains("About"),
        );
        free(&rig.t.dir);
        assert_eq!(cursor(&rig, &rig.s, &pane).0, 0);
        rig.raw(&rig.s, &pane, "\x1b");
        rig.wait(&rig.s, &pane, "D3 named refusal remains read-only", |s| {
            s.contains("read-only ·") && s.contains(reason) && !writing(s)
        });
        rig.no_writes(&rig.s);
        rig.no_writes(&rig.t);
    }
}

#[test]
fn outside_tmux_without_home_names_a_stopped_selected_target_and_never_writes() {
    let rig = Rig::new("bsnohomestopped", false);
    rig.tmux(&rig.t, &["kill-session", "-t", &rig.t.name]);
    let (pane, _) = rig.app("no-home-stopped", false);
    let screen = rig.select(&pane, &rig.t);
    let reason = format!("{} is stopped", rig.t.name);
    assert!(screen.contains("read-only ·"));
    assert!(
        screen.contains(&reason),
        "P3/D3 names the stopped target: {screen}"
    );
    assert!(
        !screen.contains("no home session"),
        "selection owns the refusal"
    );
    assert!(!screen.contains("Enter writes"));
    rig.raw(&rig.s, &pane, "iBAD\r\x1b[<0;160;45M");
    rig.wait(
        &rig.s,
        &pane,
        "GUARD no-home refusal keys processed before gear",
        |s| s.contains("Settings") && s.contains("Quota") && s.contains("About"),
    );
    free(&rig.t.dir);
    assert_eq!(cursor(&rig, &rig.s, &pane).0, 0);
    rig.raw(&rig.s, &pane, "\x1b");
    rig.wait(
        &rig.s,
        &pane,
        "P3/D3 stopped selection remains read-only",
        |s| s.contains("read-only ·") && s.contains(&reason) && !writing(s),
    );
    rig.no_writes(&rig.s);
    rig.no_writes(&rig.t);
}

#[test]
fn unresolvable_recorded_server_refuses_delivery_without_rerouting_to_home() {
    let rig = Rig::new("bsunresolved", true);
    let (pane, gates) = rig.app("unresolved", true);
    rig.select(&pane, &rig.t);
    rig.enter(&pane, &rig.t, false);
    rig.typed(&pane, "never-goes-home");
    let world = Hold::new(&gates, "@world");
    world.held();
    let original = fs::read_to_string(rig.t.dir.join("meta")).expect("target meta");
    fs::write(
        rig.t.dir.join("meta"),
        original.replace(
            &format!("tmux_server={}\n", rig.t.socket.display()),
            "tmux_server=\n",
        ),
    )
    .expect("unresolvable selector");
    rig.raw(&rig.s, &pane, "\r");
    rig.wait(&rig.s, &pane, "D2 named selector refusal", |s| {
        s.contains("refused")
            && (s.contains("server") || s.contains("socket") || s.contains("selector"))
    });
    assert!(rig.asks(&rig.t).is_empty());
    rig.no_writes(&rig.s);
    assert!(!rig.s.tool.submitted().contains("never-goes-home"));
    drop(world);
}

/// Every byte read by Q precedes its fresh T lease, even when the reader
/// splits i/X/Enter. P submits the copied stream and releases T first.
#[test]
fn old_entry_text_and_enter_never_duplicate_a_stream_in_the_selected_target() {
    const STREAM: &str = "iX\r";
    for split in 1..=STREAM.len() {
        let rig = Rig::new(&format!("bsstale{split}"), false);
        let (p, _) = rig.app("first", true);
        let (q, gates) = rig.app("lagging", true);
        rig.select(&p, &rig.t);
        rig.select(&q, &rig.t);
        rig.enter(&p, &rig.t, false);
        rig.typed(&p, "X");
        let keys = Hold::new(&gates, "@app-keys");
        rig.raw(&rig.s, &q, &STREAM[..split]);
        keys.held();
        let read = (split < STREAM.len()).then(|| {
            let read = Hold::new(&gates, "@app-read");
            rig.raw(&rig.s, &q, &STREAM[split..]);
            read.held();
            read
        });
        rig.raw(&rig.s, &p, "\r");
        rig.wait(&rig.s, &p, "GUARD P submits before releasing T", |s| {
            s.contains("sent") && Rig::event_count(&rig.t, "ask") == 1
        });
        rig.escape(&p, &rig.t);
        drop(read);
        drop(keys);
        let acquired = rig.wait(&rig.s, &q, "B2 Q acquires a fresh selected lease", |s| {
            writing(s) && s.contains(&rig.address(&rig.t, false, "lead"))
        });
        held(&rig.t.dir);
        assert!(
            !composer_has(&acquired, "X"),
            "split={split}: old body never enters the fresh T draft"
        );
        rig.send(&q, &rig.t, "Y", 2);
        assert_eq!(
            rig.asks(&rig.t)
                .iter()
                .map(|e| e.summary.as_deref().expect("body"))
                .collect::<Vec<_>>(),
            ["X", "Y"],
            "split={split}: old Enter submitted nothing"
        );
        rig.no_writes(&rig.s);
    }
}

/// A real S revocation is captured by the reader before the app leaves S.
/// The UI and reader gates delay that answer until a different target or a
/// new acquisition of the same target is writing. The answer is then stale.
#[test]
fn a_late_proof_cannot_revoke_another_target() {
    late_proof(true);
}

#[test]
fn a_late_proof_cannot_revoke_a_new_entry_of_the_same_target() {
    late_proof(false);
}

fn late_proof(another: bool) {
    let rig = Rig::new(
        if another {
            "bslateother"
        } else {
            "bslateentry"
        },
        false,
    );
    let (pane, gates) = rig.app("late-proof", true);
    rig.select(&pane, &rig.t);
    let coverage = format!("coverage incomplete: {}:lead", rig.t.name);
    rig.wait(
        &rig.s,
        &pane,
        "GUARD T view loaded before blocking its reader",
        |s| s.contains(&coverage),
    );
    rig.select(&pane, &rig.s);
    rig.enter(&pane, &rig.s, true);
    rig.typed(&pane, "s-parked");
    let first_world = Hold::new(&gates, "@world");
    first_world.held();
    let first_journal = Hold::new(&gates, &rig.s.name);
    let keys = Hold::new(&gates, "@app-keys");
    let target = if another { &rig.t } else { &rig.s };
    let frame = rig.frame(&rig.s, &pane);
    let (x, y) = card(&frame, &target.name);
    // Two ESC bytes separate the explicit exit from the SGR click.
    rig.raw(&rig.s, &pane, &format!("\x1b\x1b[<0;{};{}M", x + 1, y + 1));
    keys.held();
    drop(first_world);
    first_journal.held();
    // The first fleet and the view's pair were read as the original pair;
    // only the following Owned proof must observe the temporary change.
    rig.meta(&rig.s, S_ID, "changed-peer");
    let late_world = Hold::new(&gates, "@world");
    drop(first_journal);
    late_world.held();
    // owned() has already read the different pair; restore the actual
    // pair before the fresh admission so its refusal is truly stale.
    rig.meta(&rig.s, S_ID, "colead");
    drop(keys);
    rig.wait(
        &rig.s,
        &pane,
        "GUARD switched before fresh entry read",
        |s| s.contains("Enter writes") && s.contains(&rig.address(target, !another, "lead")),
    );
    free(&rig.s.dir);
    // The entry key must be read after the previous lease was released;
    // an i in the exit chunk is correctly refused by B1's release cut.
    rig.raw(&rig.s, &pane, "i");
    rig.wait(&rig.s, &pane, "GUARD new writer before late S proof", |s| {
        writing(s) && s.lines().nth(1).is_some_and(|r| r.contains(&target.name))
    });
    held(&target.dir);
    rig.typed(&pane, "-new-entry");
    let delivered = Hold::new(&gates, &target.name);
    drop(late_world);
    delivered.held();
    // The journal gate attests that the reader has dispatched the late
    // Owned answer before this new key read supplies a UI witness.
    let survived = rig.typed(&pane, "-after-old-proof");
    assert!(writing(&survived));
    assert!(
        survived
            .lines()
            .nth(1)
            .is_some_and(|r| r.contains(&target.name))
    );
    assert!(!survived.contains("not writing:"));
    assert!(composer_has(&survived, "-new-entry-after-old-proof"));
    if !another {
        assert!(composer_has(
            &survived,
            "s-parked-new-entry-after-old-proof"
        ));
    }
    held(&target.dir);
    if another {
        free(&rig.s.dir);
    }
    rig.no_writes(&rig.s);
    rig.no_writes(&rig.t);
    drop(delivered);
}

#[test]
fn non_home_address_uses_its_drawn_width_for_whole_draft_and_terminal_cursor() {
    let rig = Rig::new("bswrap", false);
    let (pane, _) = rig.app("wrap", true);
    rig.select(&pane, &rig.t);
    rig.enter(&pane, &rig.t, false);
    // Accepted 160x45 geometry: left=47, room=111. Every address character
    // here is one drawn cell, including ›; three spaces precede the draft.
    let address = format!("{}   ", rig.address(&rig.t, false, "lead"));
    let indent = address.chars().count();
    let width = 111 - indent;
    let raw = format!("{}end", "a".repeat(width * 2));
    rig.raw(&rig.s, &pane, &format!("\x1b[200~{raw}\x1b[201~"));
    let frame = rig.wait(
        &rig.s,
        &pane,
        "B2 three complete rows with selected address",
        |s| {
            s.lines()
                .nth(39)
                .is_some_and(|r| r.contains(&format!("{address}{}", "a".repeat(width))))
                && s.lines()
                    .nth(40)
                    .is_some_and(|r| r.trim_end().ends_with(&"a".repeat(width)))
                && s.lines()
                    .nth(41)
                    .is_some_and(|r| r.trim_end().ends_with("end"))
        },
    );
    for (row, suffix) in [
        (39, "a".repeat(width)),
        (40, "a".repeat(width)),
        (41, "end".to_owned()),
    ] {
        let line = frame.lines().nth(row).expect("composer row");
        let chat = line.chars().skip(47).collect::<String>();
        let prefix = if row == 39 {
            address.clone()
        } else {
            " ".repeat(indent)
        };
        assert_eq!(chat.trim_end(), format!("{prefix}{suffix}"));
    }
    assert_eq!(cursor(&rig, &rig.s, &pane), (1, 47 + indent + 3, 41));
    rig.raw(&rig.s, &pane, "\r");
    rig.wait(&rig.s, &pane, "B2 wrap changes display only", |s| {
        s.contains("sent") && Rig::event_count(&rig.t, "ask") == 1
    });
    assert_eq!(rig.asks(&rig.t)[0].summary.as_deref(), Some(raw.as_str()));
    assert!(rig.t.tool.submitted().contains(&raw));
    rig.no_writes(&rig.s);
}

#[test]
fn cached_target_outside_this_state_root_refuses_without_recreating_its_directory() {
    let rig = Rig::new("bsoutsideroot", false);
    let (pane, gates) = rig.app("outside-root", true);
    rig.select(&pane, &rig.t);
    rig.wait(&rig.s, &pane, "GUARD cached T was eligible", |s| {
        s.contains(&rig.address(&rig.t, false, "lead")) && s.contains("Enter writes")
    });
    let world = Hold::new(&gates, "@world");
    world.held();
    let outside = rig.root.join("outside-state-root");
    fs::rename(&rig.t.dir, &outside).expect("fixture T is no longer under either state root");
    rig.raw(&rig.s, &pane, "i");
    let refused = rig.wait(
        &rig.s,
        &pane,
        "D3 cached out-of-root target refuses entry",
        |s| s.contains("not writing:") || s.contains("read-only ·"),
    );
    assert!(!writing(&refused));
    assert!(
        refused
            .lines()
            .nth(1)
            .is_some_and(|r| r.contains(&rig.t.name))
    );
    let reason = refused
        .lines()
        .find(|r| r.contains("not writing:") || r.contains("read-only ·"))
        .expect("named target refusal");
    assert!(
        reason.contains("meta")
            || reason.contains("root")
            || reason.contains("session")
            || reason.contains("record")
    );
    assert!(
        !rig.t.dir.exists(),
        "a refused entry must not recreate the missing session"
    );
    assert_eq!(
        fs::read_to_string(outside.join("events.jsonl")).expect("outside journal"),
        ""
    );
    assert!(!outside.join("console.draft").exists());
    assert!(!outside.join("messages").exists());
    assert!(
        !outside.join(WRITER).exists(),
        "entry refused before publishing any lease node"
    );
    rig.no_writes(&rig.s);
    drop(world);
}

#[test]
fn a_positive_absent_name_drops_its_draft_once_and_a_new_incarnation_never_inherits_it() {
    let rig = Rig::new("bsabsent", false);
    let (pane, gates) = rig.app("absent", true);
    rig.select(&pane, &rig.t);
    rig.enter(&pane, &rig.t, false);
    rig.typed(&pane, "drop-on-positive-absence");
    let world = Hold::new(&gates, "@world");
    world.held();
    let outside = rig.root.join("absent-target");
    fs::rename(&rig.t.dir, &outside).expect("positive directory absence");
    drop(world);
    let notice = format!("draft for {} dropped", rig.t.name);
    rig.wait(&rig.s, &pane, "D4 positive absent name drops memory", |s| {
        s.contains(&notice)
    });
    free(&outside);
    assert_eq!(
        fs::read_to_string(outside.join("events.jsonl")).expect("old journal"),
        ""
    );
    assert!(!outside.join("console.draft").exists());
    assert!(!outside.join("messages").exists());
    fs::rename(&outside, &rig.t.dir).expect("same name returns with a new identity");
    rig.meta(&rig.t, NEW_ID, "colead");
    rig.tmux(
        &rig.t,
        &["set-option", "-t", &rig.t.name, "@ae_session_uuid", NEW_ID],
    );
    rig.raw(&rig.s, &pane, "\x1b");
    rig.wait(&rig.s, &pane, "GUARD returned T is listed", |s| {
        s.contains("Sessions 2") && s.contains(&rig.t.name)
    });
    rig.select(&pane, &rig.t);
    let frame = rig.enter(&pane, &rig.t, false);
    assert!(!composer_has(&frame, "drop-on-positive-absence"));
    rig.send(&pane, &rig.t, "new-after-absence", 1);
    let frame = rig.frame(&rig.s, &pane);
    assert_eq!(frame.matches(&notice).count(), 1);
    rig.no_writes(&rig.s);
}

#[test]
fn no_home_and_no_selected_session_remain_read_only_with_a_hidden_cursor() {
    let rig = Rig::new("bsempty", false);
    for (target, label) in [(&rig.s, "outside-s"), (&rig.t, "outside-t")] {
        fs::rename(&target.dir, rig.root.join(label)).expect("empty app state root");
    }
    let (pane, _) = rig.app_count("empty", false, 0);
    let frame = rig.wait(&rig.s, &pane, "B2 no selected target is read-only", |s| {
        s.contains("Sessions 0") && s.contains("read-only ·") && !s.contains("Enter writes")
    });
    assert!(!frame.contains("(not home)"));
    assert_eq!(cursor(&rig, &rig.s, &pane).0, 0);
    rig.raw(&rig.s, &pane, "iEMPTY\r\x1b[<0;160;45M");
    rig.wait(
        &rig.s,
        &pane,
        "GUARD empty-root keys processed before gear",
        |s| s.contains("Settings") && s.contains("Quota") && s.contains("About"),
    );
    for label in ["outside-s", "outside-t"] {
        let dir = rig.root.join(label);
        assert_eq!(
            fs::read_to_string(dir.join("events.jsonl")).expect("outside journal"),
            ""
        );
        assert!(!dir.join("console.draft").exists());
        assert!(!dir.join("messages").exists());
        assert!(!dir.join(WRITER).exists());
    }
    assert!(!rig.s.dir.exists() && !rig.t.dir.exists());
}
