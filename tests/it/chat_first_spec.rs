//! Slice chat-first acceptance: `prefix h` puts the chat at the session's
//! first index in running sessions too. Oracle: brief chat-first B1-B4 plus
//! the settled Q1 (missing main: chat selected, loud note, hook preserved)
//! and Q2 (restamp every successful chat selection). Solo shape: the move is
//! index arithmetic, seat count adds no oracle.

#![allow(
    clippy::disallowed_methods,
    reason = "acceptance rigs build and inspect their own private filesystem"
)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::cli::{OwnedScratch, Runner, ae, tmux_attached_client, tmux_signalled};
use super::phase2::run_tmux;

/// A session id no launch mints, for the foreign-stamp rigs.
const FOREIGN_UUID: &str = "00000000-0000-4000-8000-000000000001";

struct Rig {
    scratch: OwnedScratch,
    sock: PathBuf,
    home: PathBuf,
    project: PathBuf,
    config: PathBuf,
}

impl Rig {
    fn new(tag: &str, chat: Option<&str>) -> Self {
        let mut scratch = OwnedScratch::root("cf", tag);
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
                 [roster]\nlead = idle\n\n\
                 [workspace]\nmain = lead\nworkers = \nlayout = vertical\n\
                 watchdog = false\ntheme = off\n{chat}",
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

    fn launch(&self, session: &str) {
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
        assert!(
            output.status.success(),
            "launch {session}: {}\n{}",
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

    fn main_option(&self, session: &str) -> String {
        self.tmux(&["display-message", "-p", "-t", session, "#{@ae_main_pane}"])
            .trim()
            .to_owned()
    }

    fn hook(&self, session: &str) -> String {
        self.tmux(&["show-hooks", "-t", session, "client-session-changed"])
    }

    /// Overwrite the session hook with the pre-chat lead-only shape and
    /// return the seeded command: without a restamp an attach lands on main.
    fn seed_lead_only_hook(&self, session: &str, main: &str) -> String {
        let id = self.tmux(&["display-message", "-p", "-t", session, "#{session_id}"]);
        let command = format!(
            "if-shell -F -t \"{main}\" \"#{{==:#{{session_id}},{}}}\" \
             \"select-window -t {main} ; select-pane -t {main}\"",
            id.trim()
        );
        self.tmux(&[
            "set-hook",
            "-t",
            session,
            "client-session-changed",
            &command,
        ]);
        command
    }

    fn assert_lead_only_hook(&self, session: &str, command: &str) {
        let hooks = self.hook(session);
        let expected = format!("client-session-changed[0] {command}");
        assert_eq!(
            hooks.lines().filter(|line| *line == expected).count(),
            1,
            "lead-only legacy hook: {hooks}"
        );
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
        let uuid = self.meta_uuid(session);
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

    fn key_output(&self, pane: &str) -> std::process::Output {
        self.command()
            .env("TMUX", format!("{},1,0", self.sock.display()))
            .env("TMUX_PANE", pane)
            .arg("_console")
            .output()
            .unwrap_or_else(|why| panic!("chat shortcut runs: {why}"))
    }

    fn key(&self, pane: &str) {
        let output = self.key_output(pane);
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

    fn kill_chat(&self, pane: &str) {
        self.tmux(&["respawn-pane", "-k", "-t", pane, "sh", "-c", "exit 3"]);
        let start = Instant::now();
        loop {
            let dead = self.tmux(&["display-message", "-p", "-t", pane, "#{pane_dead}"]);
            if dead.trim() == "1" {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "chat pane never dies"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn meta_uuid(&self, session: &str) -> String {
        let meta = std::fs::read_to_string(self.home.join("sessions").join(session).join("meta"))
            .unwrap_or_else(|why| panic!("launch meta: {why}"));
        meta.lines()
            .find_map(|line| line.strip_prefix("session_id="))
            .unwrap_or_else(|| panic!("recorded session uuid: {meta}"))
            .to_owned()
    }

    fn set_meta_uuid(&self, session: &str, uuid: &str) {
        let path = self.home.join("sessions").join(session).join("meta");
        let meta = std::fs::read_to_string(&path).unwrap_or_else(|why| panic!("meta: {why}"));
        let rewritten: Vec<_> = meta
            .lines()
            .map(|line| {
                if line.starts_with("session_id=") {
                    format!("session_id={uuid}")
                } else {
                    line.to_owned()
                }
            })
            .collect();
        std::fs::write(&path, rewritten.join("\n") + "\n")
            .unwrap_or_else(|why| panic!("rewritten meta: {why}"));
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
            "wait-for -S cf-attached",
        ]);
        let mut client = tmux_attached_client(&self.sock, session)
            .unwrap_or_else(|why| panic!("private attached client: {why}"));
        assert!(
            tmux_signalled(&self.sock, "cf-attached", Duration::from_secs(5)),
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

/// B1: after the key the chat is the first window, seats shifted, lead intact.
fn assert_opened_first(rig: &Rig, session: &str, main: &str) -> String {
    rig.key(main);
    assert_eq!(
        rig.windows(session),
        "0|chat|1\n1|lead|1\n99|ae-monitor|1\n",
        "chat opens first, seats shift"
    );
    let chat = rig.chat(session);
    assert_eq!(rig.main(session), main, "lead pane id unchanged");
    assert_eq!(rig.current(session), chat, "chat selected");
    rig.live_chat(&chat);
    rig.key(&chat);
    assert_eq!(rig.current(session), main, "toggle-back returns to lead");
    rig.attach_lands(session, &chat);
    chat
}

#[test]
fn chat_first_off_launch_opens_chat_at_first_index() {
    let rig = Rig::new("open", Some("off"));
    rig.launch("cfOpen");
    let main = rig.main("cfOpen");
    assert_eq!(
        rig.windows("cfOpen"),
        "0|lead|1\n99|ae-monitor|1\n",
        "pre-feature layout"
    );
    assert_opened_first(&rig, "cfOpen", &main);
}

#[test]
fn chat_first_workspace_off_opens_chat_at_first_index() {
    let rig = Rig::new("wsoff", None);
    assert!(
        std::fs::create_dir_all(rig.project.join(".ae")).is_ok(),
        "workspace config dir"
    );
    std::fs::write(rig.project.join(".ae/config"), "[workspace]\nchat = off\n")
        .unwrap_or_else(|why| panic!("workspace chat override: {why}"));
    rig.launch("cfWsOff");
    let main = rig.main("cfWsOff");
    assert_opened_first(&rig, "cfWsOff", &main);
}

#[test]
fn chat_first_legacy_chat_moves_to_first_index() {
    let rig = Rig::new("moveback", Some("on"));
    rig.launch("cfMove");
    let chat = rig.chat("cfMove");
    let main = rig.main("cfMove");
    rig.live_chat(&chat);
    rig.tmux(&["swap-window", "-s", "cfMove:0", "-t", "cfMove:1"]);
    assert_eq!(
        rig.windows("cfMove"),
        "0|lead|1\n1|chat|1\n99|ae-monitor|1\n",
        "legacy 1:chat setup"
    );
    let seeded = rig.seed_lead_only_hook("cfMove", &main);
    rig.assert_lead_only_hook("cfMove", &seeded);
    rig.key(&main);
    assert_eq!(
        rig.windows("cfMove"),
        "0|chat|1\n1|lead|1\n99|ae-monitor|1\n",
        "chat moved first, no second chat"
    );
    assert_eq!(rig.chat("cfMove"), chat, "same chat pane after move");
    assert_eq!(rig.main("cfMove"), main, "lead pane id unchanged");
    assert_eq!(rig.current("cfMove"), chat, "chat selected");
    rig.live_chat(&chat);
    rig.key(&chat);
    assert_eq!(
        rig.current("cfMove"),
        main,
        "toggle-back after move returns to lead"
    );
    rig.attach_lands("cfMove", &chat);
}

#[test]
fn chat_first_already_first_select_restamps_stale_hook() {
    let rig = Rig::new("alreadyfirst", Some("on"));
    rig.launch("cfFirst");
    let chat = rig.chat("cfFirst");
    let main = rig.main("cfFirst");
    rig.live_chat(&chat);
    let seeded = rig.seed_lead_only_hook("cfFirst", &main);
    rig.assert_lead_only_hook("cfFirst", &seeded);
    rig.tmux(&["select-window", "-t", &main]);
    rig.tmux(&["select-pane", "-t", &main]);
    rig.key(&main);
    assert_eq!(
        rig.windows("cfFirst"),
        "0|chat|1\n1|lead|1\n99|ae-monitor|1\n",
        "no move, no duplicate chat"
    );
    assert_eq!(rig.current("cfFirst"), chat, "chat selected");
    rig.key(&chat);
    assert_eq!(rig.current("cfFirst"), main, "toggle-back returns to lead");
    rig.attach_lands("cfFirst", &chat);
}

#[test]
fn chat_first_dead_chat_respawns_at_first_index() {
    let rig = Rig::new("respawn", Some("on"));
    rig.launch("cfRespawn");
    let chat = rig.chat("cfRespawn");
    let main = rig.main("cfRespawn");
    rig.live_chat(&chat);
    rig.tmux(&["swap-window", "-s", "cfRespawn:0", "-t", "cfRespawn:1"]);
    rig.kill_chat(&chat);
    let seeded = rig.seed_lead_only_hook("cfRespawn", &main);
    rig.assert_lead_only_hook("cfRespawn", &seeded);
    rig.key(&main);
    assert_eq!(
        rig.windows("cfRespawn"),
        "0|chat|1\n1|lead|1\n99|ae-monitor|1\n",
        "respawned chat moved first"
    );
    assert_eq!(rig.chat("cfRespawn"), chat, "same chat pane after respawn");
    assert_eq!(rig.current("cfRespawn"), chat, "chat selected");
    rig.live_chat(&chat);
    rig.attach_lands("cfRespawn", &chat);
}

#[test]
fn chat_first_unbound_meta_opens_nothing() {
    let rig = Rig::new("unbound", Some("off"));
    rig.launch("cfUnbound");
    let main = rig.main("cfUnbound");
    let before = rig.windows("cfUnbound");
    rig.set_meta_uuid("cfUnbound", FOREIGN_UUID);
    let output = rig.key_output(&main);
    assert!(!output.status.success(), "unbound toggle fails");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not the one tmux carries"),
        "refusal names the mismatch: {stderr}"
    );
    assert_eq!(rig.windows("cfUnbound"), before, "no window opened");
}

#[test]
fn chat_first_foreign_lookalike_stays_put() {
    let rig = Rig::new("lookalike", Some("off"));
    rig.launch("cfLook");
    let main = rig.main("cfLook");
    let uuid = rig.meta_uuid("cfLook");
    assert_ne!(uuid, FOREIGN_UUID, "foreign stamp is actually foreign");
    let created = rig.tmux(&[
        "new-window",
        "-d",
        "-t",
        "cfLook",
        "-n",
        "chat",
        "-P",
        "-F",
        "#{pane_id}|#{window_id}",
    ]);
    let (look_pane, look_window) = created
        .trim()
        .split_once('|')
        .unwrap_or_else(|| panic!("lookalike ids: {created}"));
    rig.tmux(&[
        "set-option",
        "-p",
        "-t",
        look_pane,
        "@ae_console",
        FOREIGN_UUID,
    ]);
    rig.tmux(&["set-window-option", "-t", look_pane, "remain-on-exit", "on"]);
    assert_eq!(
        rig.windows("cfLook"),
        "0|lead|1\n1|chat|1\n99|ae-monitor|1\n",
        "foreign lookalike setup"
    );
    rig.key(&main);
    assert_eq!(
        rig.windows("cfLook"),
        "0|chat|1\n1|lead|1\n2|chat|1\n99|ae-monitor|1\n",
        "correct chat first, lookalike shifted"
    );
    let first = rig.tmux(&[
        "display-message",
        "-p",
        "-t",
        "cfLook:0",
        "#{pane_id}|#{@ae_console}",
    ]);
    let (chat, stamp) = first
        .trim()
        .split_once('|')
        .unwrap_or_else(|| panic!("first window stamps: {first}"));
    assert_eq!(stamp, uuid, "first chat carries the session uuid");
    let panes = rig.tmux(&[
        "list-panes",
        "-s",
        "-t",
        "cfLook",
        "-F",
        "#{pane_id}|#{@ae_console}",
    ]);
    assert_eq!(
        panes.lines().filter(|line| line.ends_with(&uuid)).count(),
        1,
        "one correctly stamped chat pane: {panes}"
    );
    assert_eq!(
        rig.tmux(&["display-message", "-p", "-t", look_window, "#{@ae_console}"])
            .trim(),
        FOREIGN_UUID,
        "lookalike window keeps its foreign stamp"
    );
    rig.live_chat(chat);
    rig.attach_lands("cfLook", chat);
}

#[test]
fn chat_first_missing_main_selects_chat_and_leaves_hook() {
    let rig = Rig::new("nomain", Some("on"));
    rig.launch("cfNoMain");
    let chat = rig.chat("cfNoMain");
    let main = rig.main("cfNoMain");
    let hook_before = rig.hook("cfNoMain");
    assert!(
        hook_before.contains("client-session-changed"),
        "a hook exists to preserve: {hook_before}"
    );
    rig.tmux(&["set-option", "-u", "-t", "cfNoMain", "@ae_main_pane"]);
    assert_eq!(
        rig.main_option("cfNoMain"),
        "",
        "main option actually unset"
    );
    rig.tmux(&["select-window", "-t", &main]);
    rig.tmux(&["select-pane", "-t", &main]);
    assert_eq!(rig.current("cfNoMain"), main, "lead focused before key");
    let output = rig.key_output(&main);
    assert!(!output.status.success(), "missing main is loud");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("attach hook was left unchanged"),
        "visible note names the preserved hook: {stderr}"
    );
    assert_eq!(rig.current("cfNoMain"), chat, "chat selected first");
    assert_eq!(rig.main_option("cfNoMain"), "", "@ae_main_pane untouched");
    assert_eq!(rig.hook("cfNoMain"), hook_before, "hook preserved");
}
