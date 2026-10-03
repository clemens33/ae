//! Phase 4 acceptance: a fresh/resumed session opens its human chat first,
//! while its lead remains the recorded pane. Oracle: chat-window0 R1-R7;
//! frozen off fixtures record the pre-feature layouts on f74f1877.

#![allow(
    clippy::disallowed_methods,
    reason = "acceptance rigs build and inspect their own private filesystem"
)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::cli::{OwnedScratch, Runner, ae, tmux_attached_client, tmux_signalled};
use super::phase2::run_tmux;

#[derive(Clone, Copy)]
enum Shape {
    Solo,
    Workers,
    Pair,
}

impl Shape {
    fn layout(self) -> &'static str {
        match self {
            Self::Solo => "vertical",
            Self::Workers => "lead-solo",
            Self::Pair => "lead-pair",
        }
    }

    fn workers(self) -> &'static str {
        match self {
            Self::Solo => "",
            Self::Workers => "builder, reviewer",
            Self::Pair => "colead, builder",
        }
    }

    fn off(self) -> &'static str {
        match self {
            Self::Solo => include_str!("../fixtures/chat-window0/off-solo.txt"),
            Self::Workers => include_str!("../fixtures/chat-window0/off-workers.txt"),
            Self::Pair => include_str!("../fixtures/chat-window0/off-pair.txt"),
        }
    }
}

struct Rig {
    scratch: OwnedScratch,
    sock: PathBuf,
    home: PathBuf,
    project: PathBuf,
    config: PathBuf,
}

impl Rig {
    fn new(tag: &str, shape: Shape, chat: Option<&str>) -> Self {
        let mut scratch = OwnedScratch::root("cw0", tag);
        let sock = scratch.join("sock");
        scratch.add_tmux_server(sock.clone());
        let rig = Self {
            sock,
            home: scratch.join("state"),
            project: scratch.join("project"),
            config: scratch.join("config"),
            scratch,
        };
        assert!(
            std::fs::create_dir_all(&rig.project).is_ok(),
            "private project"
        );
        let chat = chat.map_or_else(String::new, |value| format!("chat = {value}\n"));
        std::fs::write(
            &rig.config,
            format!(
                "[profiles]\nidle = \"sleep 600\"\n\n\
                 [roster]\nlead = idle\ncolead = idle\nbuilder = idle\nreviewer = idle\n\n\
                 [workspace]\nmain = lead\nworkers = {}\nlayout = {}\n\
                 watchdog = false\ntheme = off\n{chat}",
                shape.workers(),
                shape.layout(),
            ),
        )
        .unwrap_or_else(|why| panic!("private config: {why}"));
        rig
    }

    fn command(&self) -> Runner {
        let mut command = ae();
        command
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("HOME", &self.scratch)
            .env("TMUX_TMPDIR", &self.scratch)
            .env("AE_HOME", &self.home)
            .env("CONFIG_FILE", &self.config)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.sock)
            .current_dir(&self.project);
        command
    }

    fn launch(&self, session: &str) -> String {
        let mut command = self.command();
        command
            .arg(ae::cli::LAUNCH)
            .args(["--home"])
            .arg(&self.home)
            .arg("--cwd")
            .arg(&self.project)
            .arg("--global")
            .arg(&self.config)
            .args(["--server-kind", "socket", "--server"])
            .arg(&self.sock)
            .args(["--no-attach", "--no-autostart"]);
        let local = self.project.join(".ae/config");
        if local.is_file() {
            command.arg("--local-config").arg(&local);
        }
        let output = command
            .args(["--", "--local", session])
            .output()
            .unwrap_or_else(|why| panic!("private launch runs: {why}"));
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(
            output.status.success(),
            "launch {session}: {}\n{stderr}",
            String::from_utf8_lossy(&output.stdout)
        );
        stderr
    }

    fn public(&self, tail: &[&str]) {
        let output = self
            .command()
            .args(tail)
            .output()
            .unwrap_or_else(|why| panic!("private ae runs: {why}"));
        assert!(
            output.status.success(),
            "{tail:?}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.sock.display().to_string()];
        args.extend(tail.iter().map(|value| (*value).to_owned()));
        let (success, output) = run_tmux(&args, &self.scratch);
        assert!(success, "private tmux {tail:?}: {output}");
        output
    }

    fn windows(&self, session: &str) -> String {
        self.tmux(&[
            "list-windows",
            "-t",
            session,
            "-F",
            "#{window_index}|#{window_name}|#{window_panes}",
        ])
    }

    fn main(&self, session: &str) -> String {
        let panes = self.tmux(&[
            "list-panes",
            "-s",
            "-t",
            session,
            "-F",
            "#{pane_id}|#{@ae_slot}|#{@ae_agent}",
        ]);
        let main = panes
            .lines()
            .find_map(|line| line.strip_suffix("|main|lead"))
            .unwrap_or_else(|| panic!("one stamped lead pane: {panes}"));
        let recorded = self.tmux(&["show-options", "-v", "-t", session, "@ae_main_pane"]);
        assert_eq!(recorded.trim(), main, "main remains the lead pane id");
        main.to_owned()
    }

    fn chat(&self, session: &str) -> String {
        let windows = self.windows(session);
        assert!(
            windows.starts_with("0|chat|1\n"),
            "the first window is the human chat: {windows}"
        );
        assert_eq!(windows.matches("|chat|1\n").count(), 1, "one chat window");
        let pane = self.tmux(&[
            "display-message",
            "-p",
            "-t",
            &format!("{session}:0"),
            "#{pane_id}|#{@ae_console}|#{@ae_agent}|#{@ae_slot}",
        ]);
        let fields: Vec<_> = pane.trim().split('|').collect();
        assert_eq!(fields.len(), 4, "chat pane stamps: {pane}");
        let meta = std::fs::read_to_string(self.home.join("sessions").join(session).join("meta"))
            .unwrap_or_else(|why| panic!("launch meta: {why}"));
        let uuid = meta
            .lines()
            .find_map(|line| line.strip_prefix("session_id="))
            .unwrap_or_else(|| panic!("recorded session uuid: {meta}"));
        assert_eq!(fields[1], uuid, "chat bound to published meta");
        assert_eq!(fields[2], "", "watchdog sees no chat agent");
        assert_eq!(fields[3], "", "chat has no agent slot");
        let host_uuid = self.tmux(&["show-options", "-v", "-t", session, "@ae_session_uuid"]);
        assert_eq!(host_uuid.trim(), uuid, "chat bound to host session");
        let remain = self.tmux(&[
            "show-window-options",
            "-v",
            "-t",
            fields[0],
            "remain-on-exit",
        ]);
        assert_eq!(remain.trim(), "on", "a failed chat keeps its reopen hint");
        let command = self.tmux(&[
            "display-message",
            "-p",
            "-t",
            fields[0],
            "#{pane_start_command}",
        ]);
        for argument in ["chat", session, "--follow", "--input"] {
            assert!(
                command.contains(argument),
                "chat argv lacks {argument}: {command}"
            );
        }
        assert_ne!(
            fields[0],
            self.main(session),
            "chat is separate from the lead"
        );
        fields[0].to_owned()
    }

    fn current(&self, session: &str) -> String {
        self.tmux(&["display-message", "-p", "-t", session, "#{pane_id}"])
            .trim()
            .to_owned()
    }

    fn key(&self, pane: &str, jump: bool) {
        let mut command = self.command();
        command
            .env("TMUX", format!("{},1,0", self.sock.display()))
            .env("TMUX_PANE", pane)
            .arg("_console");
        if jump {
            command.arg("--jump");
        }
        let output = command
            .output()
            .unwrap_or_else(|why| panic!("chat shortcut runs: {why}"));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn live_chat(&self, pane: &str) {
        let start = Instant::now();
        loop {
            let state = self.tmux(&[
                "display-message",
                "-p",
                "-t",
                pane,
                "#{pane_dead}|#{pane_current_command}",
            ]);
            if state.trim() == "0|ae" || state.trim() == "0|ae-core" {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "chat never runs: {state}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn attach_lands(&self, session: &str, expected: &str) {
        let monitor = self.tmux(&[
            "display-message",
            "-p",
            "-t",
            &format!("{session}:99"),
            "#{pane_id}",
        ]);
        self.tmux(&["select-window", "-t", monitor.trim()]);
        self.tmux(&["select-pane", "-t", monitor.trim()]);
        self.tmux(&[
            "set-hook",
            "-t",
            session,
            "client-session-changed[1]",
            "wait-for -S cw0-attached",
        ]);
        let mut client = tmux_attached_client(&self.sock, session)
            .unwrap_or_else(|why| panic!("private attached client: {why}"));
        assert!(
            tmux_signalled(&self.sock, "cw0-attached", Duration::from_secs(5)),
            "attach hook completed"
        );
        assert_eq!(
            self.current(session),
            expected,
            "actual attached client lands correctly"
        );
        let _ = client.kill();
        let _ = client.wait();
    }
}

fn matrix(tag: &str, shape: Shape) {
    let off = Rig::new(&format!("{tag}off"), shape, Some("off"));
    off.launch("cwOff");
    assert_eq!(
        off.windows("cwOff"),
        shape.off(),
        "off preserves the full old layout"
    );
    assert_eq!(off.current("cwOff"), off.main("cwOff"));

    let on = Rig::new(&format!("{tag}on"), shape, Some("on"));
    on.launch("cwOn");
    let chat = on.chat("cwOn");
    assert_eq!(on.current("cwOn"), chat, "attach destination is chat");
    on.live_chat(&chat);
    let old_seats: Vec<_> = shape
        .off()
        .lines()
        .filter(|line| !line.contains("ae-monitor"))
        .map(|line| {
            line.split_once('|')
                .unwrap_or_else(|| panic!("fixture fields: {line}"))
                .1
        })
        .collect();
    let new_windows = on.windows("cwOn");
    let new_seats: Vec<_> = new_windows
        .lines()
        .filter(|line| !line.contains("|chat|") && !line.contains("ae-monitor"))
        .map(|line| {
            line.split_once('|')
                .unwrap_or_else(|| panic!("tmux fields: {line}"))
                .1
        })
        .collect();
    assert_eq!(
        new_seats, old_seats,
        "chat preserves all agent window names and pane counts"
    );
}

#[test]
fn chat_window0_solo_on_off() {
    matrix("solo", Shape::Solo);
}

#[test]
fn chat_window0_workers_on_off() {
    matrix("work", Shape::Workers);
}

#[test]
fn chat_window0_lead_pair_on_off() {
    matrix("pair", Shape::Pair);
}

#[test]
fn chat_window0_defaults_on_and_workspace_off_overrides_global_on() {
    let rig = Rig::new("default", Shape::Solo, None);
    rig.launch("cwDefault");
    let chat = rig.chat("cwDefault");
    assert_eq!(rig.current("cwDefault"), chat);
    assert!(
        std::fs::create_dir_all(rig.project.join(".ae")).is_ok(),
        "workspace config dir"
    );
    std::fs::write(rig.project.join(".ae/config"), "[workspace]\nchat = off\n")
        .unwrap_or_else(|why| panic!("workspace chat override: {why}"));
    rig.launch("cwOverride");
    assert_eq!(
        rig.windows("cwOverride"),
        Shape::Solo.off(),
        "workspace overrides default on"
    );
}

#[test]
fn chat_window0_unusable_config_defaults_on_with_one_note() {
    for (tag, value) in [("bad", "broken"), ("empty", "")] {
        let rig = Rig::new(tag, Shape::Solo, Some(value));
        let stderr = rig.launch("cwBad");
        rig.chat("cwBad");
        let notes: Vec<_> = stderr
            .lines()
            .filter(|line| line.contains("chat"))
            .collect();
        assert_eq!(notes.len(), 1, "one visible unusable-chat note: {stderr}");
        assert!(
            notes[0].contains(value),
            "note names unusable value: {stderr}"
        );
    }
}

#[test]
fn chat_window0_uuid_mismatch_keeps_seats_and_names_chat_refusal_once() {
    let rig = Rig::new("uuid-mismatch", Shape::Solo, Some("on"));
    let foreign_uuid = "00000000-0000-4000-8000-000000000001";
    // A real inherited option makes the vacant-only UUID seed hold. No
    // production fault seam: this private server owns every session here.
    rig.tmux(&["new-session", "-d", "-s", "keepUuid", "sleep 600"]);
    rig.tmux(&["set-option", "-g", "@ae_session_uuid", foreign_uuid]);
    let stderr = rig.launch("cwUuidMismatch");
    assert_eq!(
        rig.windows("cwUuidMismatch"),
        Shape::Solo.off(),
        "a host/meta UUID mismatch must not open a chat"
    );
    let main = rig.main("cwUuidMismatch");
    assert_eq!(rig.current("cwUuidMismatch"), main);
    assert_eq!(
        rig.tmux(&[
            "display-message",
            "-p",
            "-t",
            &main,
            "#{pane_dead}|#{@ae_agent}|#{@ae_slot}",
        ])
        .trim(),
        "0|lead|main",
        "the launch remains up after its chat is refused"
    );
    let host = rig.tmux(&[
        "display-message",
        "-p",
        "-t",
        "cwUuidMismatch",
        "#{@ae_session_uuid}",
    ]);
    assert_eq!(host.trim(), foreign_uuid, "foreign UUID is not overwritten");
    let dir = rig.home.join("sessions/cwUuidMismatch");
    let meta = std::fs::read_to_string(dir.join("meta")).expect("launch meta");
    let recorded = meta
        .lines()
        .find_map(|line| line.strip_prefix("session_id="))
        .expect("recorded session UUID");
    assert_ne!(recorded, foreign_uuid, "mismatch was actually exercised");
    let notes: Vec<_> = stderr
        .lines()
        .filter(|line| line.contains("chat"))
        .collect();
    assert_eq!(notes.len(), 1, "one visible chat refusal: {stderr}");
    assert!(notes[0].contains("session id in its meta"), "{stderr}");
    assert!(notes[0].contains("prefix h opens the chat"), "{stderr}");
    let journal = std::fs::read_to_string(dir.join("events.jsonl")).expect("launch journal");
    let failed: Vec<_> = journal
        .lines()
        .map(|line| ae::events::Event::parse_line(line).expect("journal event"))
        .filter(|event| event.action == "chat-window-failed")
        .collect();
    assert_eq!(failed.len(), 1, "one durable chat refusal: {journal}");
    assert!(
        failed[0]
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains("session id in its meta")),
        "journal names the UUID mismatch: {journal}"
    );
}

#[test]
fn chat_window0_resume_rebuilds_chat_before_selecting_it() {
    let rig = Rig::new("resume", Shape::Pair, Some("on"));
    rig.launch("cwResume");
    rig.chat("cwResume");
    rig.public(&["stop", "cwResume"]);
    rig.launch("cwResume");
    let rebuilt = rig.chat("cwResume");
    assert_eq!(rig.current("cwResume"), rebuilt);
    rig.live_chat(&rebuilt);
}

#[test]
fn chat_window0_rename_respawns_for_new_name_and_keeps_lead_identity() {
    let rig = Rig::new("rename", Shape::Pair, Some("on"));
    rig.launch("cwOld");
    let chat = rig.chat("cwOld");
    let main = rig.main("cwOld");
    rig.public(&["rename", "cwOld", "cwNew"]);
    let renamed = rig.chat("cwNew");
    assert_eq!(renamed, chat, "rename retains the chat window and pane");
    assert_eq!(
        rig.main("cwNew"),
        main,
        "rename preserves main pane identity"
    );
    let command = rig.tmux(&[
        "display-message",
        "-p",
        "-t",
        &chat,
        "#{pane_start_command}",
    ]);
    assert!(
        !command.contains("cwOld"),
        "old-name argv must be respawned: {command}"
    );
    rig.live_chat(&chat);
}

#[test]
fn chat_window0_shortcuts_toggle_existing_chat_and_jump_to_recorded_lead() {
    let rig = Rig::new("keys", Shape::Pair, Some("on"));
    rig.launch("cwKeys");
    let chat = rig.chat("cwKeys");
    let main = rig.main("cwKeys");
    rig.tmux(&["select-window", "-t", &main]);
    rig.tmux(&["select-pane", "-t", &main]);
    rig.key(&main, false);
    assert_eq!(
        rig.current("cwKeys"),
        chat,
        "prefix h selects existing window-0 chat"
    );
    rig.key(&chat, false);
    assert_eq!(
        rig.current("cwKeys"),
        main,
        "prefix h returns to lead window"
    );
    rig.key(&main, false);
    rig.key(&chat, true);
    assert_eq!(
        rig.current("cwKeys"),
        main,
        "prefix H targets recorded lead pane"
    );
    assert_eq!(
        rig.windows("cwKeys").matches("|chat|1\n").count(),
        1,
        "keys never duplicate chat"
    );
}

#[test]
fn chat_window0_actual_attach_lands_on_live_chat() {
    let rig = Rig::new("attach", Shape::Pair, Some("on"));
    rig.launch("cwAttach");
    let chat = rig.chat("cwAttach");
    rig.live_chat(&chat);
    rig.attach_lands("cwAttach", &chat);
    rig.launch("cwAttach");
    assert_eq!(
        rig.chat("cwAttach"),
        chat,
        "running reattach preserves layout"
    );
    rig.attach_lands("cwAttach", &chat);
}

#[test]
fn chat_window0_dead_or_absent_chat_attach_falls_back_to_lead() {
    for dead in [true, false] {
        let rig = Rig::new(
            if dead { "dead" } else { "absent" },
            Shape::Solo,
            Some("on"),
        );
        rig.launch("cwFallback");
        let chat = rig.chat("cwFallback");
        rig.live_chat(&chat);
        if dead {
            rig.tmux(&["respawn-pane", "-k", "-t", &chat, "false"]);
            let start = Instant::now();
            loop {
                let state = rig.tmux(&["display-message", "-p", "-t", &chat, "#{pane_dead}"]);
                if state.trim() == "1" {
                    break;
                }
                assert!(
                    start.elapsed() < Duration::from_secs(5),
                    "chat must be dead"
                );
                std::thread::sleep(Duration::from_millis(25));
            }
        } else {
            rig.tmux(&["kill-window", "-t", &chat]);
        }
        let main = rig.main("cwFallback");
        rig.attach_lands("cwFallback", &main);
    }
}

#[test]
fn chat_window0_off_focus_hook_keeps_pre_feature_lead_command() {
    let rig = Rig::new("offhook", Shape::Pair, Some("off"));
    rig.launch("cwOffHook");
    let main = rig.main("cwOffHook");
    let hooks = rig.tmux(&["show-hooks", "-t", "cwOffHook"]);
    let id = rig.tmux(&["display-message", "-p", "-t", "cwOffHook", "#{session_id}"]);
    let expected = format!(
        "client-session-changed[0] if-shell -F -t \"{main}\" \"#{{==:#{{session_id}},{}}}\" \"select-window -t {main} ; select-pane -t {main}\"",
        id.trim()
    );
    assert_eq!(
        hooks.lines().filter(|line| *line == expected).count(),
        1,
        "off keeps old focus hook: {hooks}"
    );
    rig.attach_lands("cwOffHook", &main);
}

#[test]
fn chat_window0_workspace_on_overrides_global_off() {
    let rig = Rig::new("onlayer", Shape::Solo, Some("off"));
    assert!(std::fs::create_dir_all(rig.project.join(".ae")).is_ok());
    assert!(std::fs::write(rig.project.join(".ae/config"), "[workspace]\nchat = on\n").is_ok());
    rig.launch("cwLayer");
    let chat = rig.chat("cwLayer");
    assert_eq!(
        rig.current("cwLayer"),
        chat,
        "workspace on overrides global off"
    );
}

#[test]
fn chat_window0_spawning_keeps_chat_and_lead_in_their_windows() {
    use std::os::unix::fs::PermissionsExt as _;
    let rig = Rig::new("spawn", Shape::Pair, Some("on"));
    // Mock only the external harness. A sleeper has no composer and cannot
    // receive the mandatory spawn brief; the real spawn/tmux path stays live.
    let fake = rig.scratch.join("claude");
    assert!(std::fs::write(&fake, include_str!("../fixtures/chat-window0/claude.pl")).is_ok());
    assert!(std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).is_ok());
    let config =
        std::fs::read_to_string(&rig.config).unwrap_or_else(|why| panic!("spawn config: {why}"));
    assert!(
        std::fs::write(
            &rig.config,
            config.replacen(
                "[profiles]\n",
                &format!("[profiles]\nspawnspec = \"{}\"\n", fake.display()),
                1
            )
        )
        .is_ok()
    );
    rig.launch("cwSpawn");
    let chat = rig.chat("cwSpawn");
    let main = rig.main("cwSpawn");
    let output = rig
        .command()
        .env("TMUX", format!("{},1,0", rig.sock.display()))
        .env("TMUX_PANE", &main)
        .args(["@cwSpawn", "spawn", "extra", "--using", "spawnspec"])
        .output()
        .unwrap_or_else(|why| panic!("spawn helper runs: {why}"));
    assert!(
        output.status.success(),
        "spawn: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        rig.chat("cwSpawn"),
        chat,
        "spawn leaves first chat pane intact"
    );
    assert_eq!(
        rig.main("cwSpawn"),
        main,
        "spawn preserves lead pane identity"
    );
    let panes = rig.tmux(&[
        "list-panes",
        "-s",
        "-t",
        "cwSpawn",
        "-F",
        "#{window_name}|#{window_index}|#{@ae_agent}|#{@ae_slot}",
    ]);
    assert!(
        panes
            .lines()
            .any(|line| line.starts_with("extra|") && line.ends_with("|extra|spawned.0")),
        "spawn owns a separate agent window: {panes}"
    );
}
