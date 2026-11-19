//! Frozen outside-open acceptance. The real app, terminal, pane proof and
//! view preparation run. Only the interactive tmux attach is replaced.

use super::*;
use std::os::unix::fs::PermissionsExt as _;

const OLD: &str = "ae app is not running inside tmux";
const ATTACH_OUTPUT: &str = "OPENOUT-FAKE-ATTACH";
const GONE: &str = "openout fixture: target server gone";

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

struct Outside {
    rig: Rig,
    app: String,
    attach: PathBuf,
    trace: PathBuf,
}

impl Outside {
    fn new(rig: Rig, fails: bool) -> Self {
        let root = rig.scratch.path().display().to_string();
        let bin = rig.scratch.join("bin");
        let attach = rig.scratch.join("attach");
        let trace = rig.scratch.join("terminal");
        let path = std::env::var_os("PATH").expect("fixture PATH");
        let real = std::env::split_paths(&path)
            .map(|dir| dir.join("tmux"))
            .find(|candidate| candidate.is_file())
            .expect("real tmux for proof commands");
        let path =
            std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(&path)))
                .expect("fixture PATH with attach shim");
        fs::create_dir(&bin).expect("private executable directory");
        let wrapper = bin.join("tmux");
        let action = if fails {
            format!(
                "printf '%s\\n' {} 'SECOND-ATTACH-DIAGNOSTIC' >&2; exit 73",
                quote(GONE)
            )
        } else {
            "exit 0".to_owned()
        };
        fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\ncase \" $* \" in\n*' attach-session '*|*' attach '*)\n\
                 [ -t 0 ] && [ -t 1 ] || exit 72\n\
                 stty icanon echo || exit 72\n\
                 {{ printf '<%s>:<%s>\\n' \"${{TMUX+x}}\" \"${{TMUX_PANE+x}}\"; \
                 printf '%s\\n' \"$@\"; }} > {attach}.part && mv {attach}.part {attach}\n\
                 printf '%s\\n' {output}\n{action}\n;;\nesac\nexec {real} \"$@\"\n",
                attach = quote(&attach.display().to_string()),
                output = quote(ATTACH_OUTPUT),
                real = quote(&real.display().to_string()),
            ),
        )
        .expect("only the owned attach is faked");
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).expect("executable shim");
        // The holder supplies a pty. Empty markers classify the app as outside
        // and let the attach receipt distinguish removal from inheritance.
        let app = rig
            .tmux(&[
                "new-window",
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                "-t",
                HOME,
                "env",
                "-u",
                "CLAUDE_CONFIG_DIR",
                "-u",
                "CODEX_HOME",
                "TMUX=",
                "TMUX_PANE=",
                &format!("HOME={root}"),
                &format!("AE_HOME={root}"),
                &format!("TMUX_TMPDIR={root}"),
                &format!("CONFIG_FILE={root}/config"),
                &format!("AE_TEST_APP_READ_GATE={}", rig.gates.display()),
                "AE_TMUX_SERVER_KIND=socket",
                &format!("AE_TMUX_SERVER={}", rig.socket.display()),
                &format!("PATH={}", path.to_string_lossy()),
                env!("CARGO_BIN_EXE_ae"),
                "app",
                HOME,
            ])
            .trim()
            .to_owned();
        rig.wait_screen(&app, "GUARD outside app painted home", |screen| {
            screen.contains(HOME) && screen.contains("Enter writes")
        });
        rig.tmux(&[
            "pipe-pane",
            "-O",
            "-t",
            &app,
            &format!("cat > {}", quote(&trace.display().to_string())),
        ]);
        Self {
            rig,
            app,
            attach,
            trace,
        }
    }

    fn returned(&self, message: &str) -> String {
        let screen = self
            .rig
            .wait_screen(&self.app, "app returns from outside attach", |s| {
                s.contains(message) || s.contains(OLD)
            });
        assert!(screen.contains(message), "expected {message:?}:\n{screen}");
        assert!(
            !screen.contains(ATTACH_OUTPUT),
            "whole frame replaces the child's output"
        );
        screen
    }

    fn terminal_restored(&self, server: &Path) {
        let receipt =
            fs::read_to_string(&self.attach).expect("the terminal reached the fake attach");
        let mut lines = receipt.lines();
        assert_eq!(lines.next(), Some("<>:<>"), "attach inherited tmux markers");
        assert!(
            lines.any(|line| Path::new(line) == server),
            "attach did not use {}: {receipt}",
            server.display()
        );
        Rig::wait("raw output capture includes the resumed screen", || {
            fs::read_to_string(&self.trace).is_ok_and(|text| {
                text.split_once(ATTACH_OUTPUT)
                    .is_some_and(|(_, after)| after.contains("\x1b[?1006h"))
            })
        });
        let capture = fs::read_to_string(&self.trace).expect("terminal byte capture");
        let (before, after) = capture
            .split_once(ATTACH_OUTPUT)
            .expect("child's draw boundary");
        for mode in ["1049", "1002", "1006"] {
            assert!(
                before.contains(&format!("\x1b[?{mode}l")),
                "mode {mode} left before attach"
            );
            assert!(
                after.contains(&format!("\x1b[?{mode}h")),
                "mode {mode} restored after attach"
            );
        }
    }
}

#[test]
fn outside_composer_open_hands_over_its_terminal_and_resumes_input() {
    let out = Outside::new(Rig::new("outside-open"), false);
    let rig = &out.rig;
    rig.composer(&out.app);
    let journal = rig.scratch.join("sessions").join(HOME).join("events.jsonl");
    let before = fs::read(&journal).expect("journal before navigation");
    rig.submit_open(&out.app, "scout");
    out.returned("back from scout");
    out.terminal_restored(&rig.socket);
    rig.raw(&out.app, "RAW-INPUT-AFTER-OPEN");
    rig.notice(&out.app, "RAW-INPUT-AFTER-OPEN");
    assert_eq!(fs::read(journal).expect("journal after navigation"), before);
}

#[test]
fn outside_attach_failure_restores_raw_screen_and_mouse_with_one_reason() {
    let out = Outside::new(Rig::new("outside-error"), true);
    let rig = &out.rig;
    rig.compose_open(&out.app, "scout");
    let screen = out.returned(GONE);
    let refusals: Vec<_> = screen
        .lines()
        .filter(|line| line.contains("refused: /open scout"))
        .collect();
    assert_eq!(refusals.len(), 1, "one visible refusal:\n{screen}");
    assert!(
        refusals[0].contains(GONE),
        "the refusal names the failure:\n{screen}"
    );
    assert!(!screen.contains("SECOND-ATTACH-DIAGNOSTIC"));
    out.terminal_restored(&rig.socket);
    rig.raw(&out.app, "RAW-INPUT-AFTER-FAILURE");
    rig.notice(&out.app, "RAW-INPUT-AFTER-FAILURE");
}

#[test]
fn outside_open_refuses_an_agent_restamp_between_input_and_the_live_proof() {
    let out = Outside::new(Rig::new("outside-proof"), false);
    let rig = &out.rig;
    rig.composer(&out.app);
    rig.raw(&out.app, "/open scout");
    rig.notice(&out.app, "/open scout");
    let gate = rig.gates.join("@app-keys");
    fs::write(&gate, "hold Enter").expect("private Enter gate");
    let held = Hold(gate);
    rig.key(&out.app, "Enter");
    Rig::wait("GUARD Enter reached the app input gate", || {
        fs::read_to_string(rig.gates.join("trace"))
            .is_ok_and(|text| text.lines().any(|line| line == "held @app-keys"))
    });
    rig.stamp(&rig.scout, "spawned.0", "replacement");
    drop(held);
    let expected = "refused: /open scout: its pane carries another agent; nothing selected";
    let screen = rig.wait_screen(&out.app, "open refuses its restamped pane", |s| {
        s.contains(expected) || s.contains(OLD)
    });
    assert!(
        screen.contains(expected),
        "expected {expected:?}:\n{screen}"
    );
    assert!(
        !out.attach.exists(),
        "a stale proof cannot reach the terminal child"
    );
}

#[test]
fn outside_orchestrator_pane_button_uses_its_recorded_server_and_keeps_its_draft() {
    let mut rig = Rig::new("outside-panel");
    fs::write(
        rig.scratch.join("config"),
        "[workspace]\nchat = app\nicons = off\n",
    )
    .expect("plain app header for button coordinates");
    let other = rig.scratch.join("orch.sock");
    rig.scratch.add_tmux_server(other.clone());
    let pane = rig.at(
        &other,
        &[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-s",
            "orchestrator",
            "-e",
            "AE_SESSION=orchestrator",
            "sleep",
            "600",
        ],
    );
    let pane = pane.trim();
    for (scope, target, option, value) in [
        ("-t", "=orchestrator:", "@ae_session_uuid", OTHER_UUID),
        ("-t", "=orchestrator:", "@ae_main_pane", pane),
        ("-pt", pane, "@ae_slot", "main"),
        ("-pt", pane, "@ae_agent", "orchestrator"),
    ] {
        rig.at(&other, &["set-option", scope, target, option, value]);
    }
    let overlay = rig.scratch.join("orchestrator.config");
    fs::write(&overlay, "[workspace]\norchestrator = true\n").expect("canonical role overlay");
    let dir = rig.scratch.join("sessions").join("orchestrator");
    fs::create_dir(&dir).expect("role record directory");
    fs::write(dir.join("meta"), format!(
        "schema=2\nsession=orchestrator\nmode=local\nsession_id={OTHER_UUID}\nlayout=vertical\nseat.main=orchestrator\nagent_bin.main=codex\nmain_pane={pane}\nmeta_agent=true\nlocal_config={}\ntmux_server_kind=socket\ntmux_server={}\n",
        overlay.display(), other.display()
    )).expect("live role record");
    ae::watchdog_glue::touch_beat(&dir).expect("fresh role watchdog beat");
    let mut history = String::new();
    for n in 0..80 {
        writeln!(history, "{{\"ts\":\"2026-10-03T06:00:00Z\",\"actor\":\"orchestrator\",\"action\":\"chat\",\"summary\":\"ORCH-TURN-{n:03}\"}}")
            .expect("role history line");
    }
    fs::write(dir.join("events.jsonl"), history).expect("role history");

    let out = Outside::new(rig, false);
    let rig = &out.rig;
    rig.raw(&out.app, "O");
    rig.wait_screen(
        &out.app,
        "GUARD live panel with a pane button and transcript",
        |s| {
            s.lines().next().is_some_and(|row| row.contains("pane  F "))
                && s.contains("ORCH-TURN-079")
        },
    );
    rig.key(&out.app, "Enter");
    rig.wait_screen(&out.app, "GUARD panel composer has keys", |s| {
        s.contains("to orchestrator") && writing(s)
    });
    rig.raw(&out.app, "OUTSIDE-PANEL-DRAFT");
    rig.notice(&out.app, "OUTSIDE-PANEL-DRAFT");
    // Wheel while composing scrolls the lane and leaves the draft alone.
    let screen = rig.screen(&out.app);
    let header = screen.lines().next().expect("drawn header");
    let idx = header.rfind("pane  F ").expect("live pane button");
    let column = header[..idx].chars().count() + 1;
    for _ in 0..12 {
        rig.raw(&out.app, &format!("\x1b[<64;{column};12M"));
    }
    let below = "↓ 12 newer turns · End";
    let scrolled = rig.wait_screen(
        &out.app,
        "GUARD panel painted all wheel events while drafting",
        |s| s.contains("OUTSIDE-PANEL-DRAFT") && s.contains(below),
    );
    let first_turn = |screen: &str| {
        screen.lines().find_map(|line| {
            line.split_once("ORCH-TURN-")
                .map(|(_, id)| id.trim().to_owned())
        })
    };
    let top = first_turn(&scrolled).expect("history landmark");
    rig.raw(&out.app, &format!("\x1b[<0;{column};1M"));
    let after = out.returned("back from orchestrator");
    out.terminal_restored(&other);
    assert!(
        after.contains("OUTSIDE-PANEL-DRAFT"),
        "draft survives:\n{after}"
    );
    assert!(
        first_turn(&after).as_deref() == Some(top.as_str()) && after.contains(below),
        "scroll survives at ORCH-TURN-{top}:\n{after}"
    );
}
