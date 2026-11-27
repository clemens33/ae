//! D54 acceptance from the owner rulings. Drive the real app and send helper
//! against private tmux. The fake Claude only draws frames and records input.

#![allow(
    clippy::expect_used,
    reason = "acceptance fixtures fail loudly when external setup fails"
)]

use std::fs;
use std::mem::ManuallyDrop;

use super::{PANE_WAIT, Rig, await_evidence};
use crate::o2guard_p2_spec::{Seat, World, uuid_of};

mod warming;

const ORCH: &str = "orchestrator";
const IDLE: &str = include_str!("../../fixtures/harness-state/claude-idle-167x40.txt");
const WARMUP: &str = "keep holding…";
fn checked(rig: &Rig, args: &[&str]) -> String {
    let (ok, text) = rig.tmux(args);
    assert!(ok, "private tmux {args:?}: {text}");
    text
}

fn seal(rig: &Rig, uuid: &str) {
    checked(
        rig,
        &["set-option", "-t", &rig.pane, "@ae_session_uuid", uuid],
    );
    checked(
        rig,
        &["set-option", "-t", &rig.pane, "@ae_main_pane", &rig.pane],
    );
    checked(rig, &["set-option", "-t", &rig.pane, "@ae_attn_rank", "0"]);
    let overlay = rig.scratch.join("orchestrator.config");
    fs::write(&overlay, "[workspace]\norchestrator = true\n").expect("role overlay");
    let path = rig.dir.join("meta");
    let old = fs::read_to_string(&path).expect("seat meta");
    let fields = [
        "schema",
        "session_id",
        "mode",
        "layout",
        "main_pane",
        "meta_agent",
        "local_config",
        "config_home.main",
        "work_dir",
    ];
    let old: Vec<_> = old
        .lines()
        .filter(|row| {
            row.split_once('=')
                .is_none_or(|(name, _)| !fields.contains(&name))
        })
        .collect();
    fs::write(
        path,
        format!(
            "{}\nschema=2\nsession_id={uuid}\nmode=local\nlayout=lead-pair\nmain_pane={}\nmeta_agent=true\nlocal_config={}\nconfig_home.main={}\nwork_dir={}\n",
            old.join("\n"), rig.pane, overlay.display(), rig.scratch.join("claude-home").display(), rig.scratch.display()
        ),
    )
    .expect("private role meta");
    ae::watchdog_glue::touch_beat(&rig.dir).expect("fresh private watchdog beat");
}

fn orchestrator(tag: &str, tool: &str, frame: &str) -> Rig {
    let mut rig = Rig::with_frame(tag, tool, frame);
    checked(&rig, &["rename-session", "-t", &rig.session, ORCH]);
    checked(
        &rig,
        &[
            "set-environment",
            "-t",
            "=orchestrator:",
            "AE_SESSION",
            ORCH,
        ],
    );
    let next = rig.dir.with_file_name(ORCH);
    fs::rename(&rig.dir, &next).expect("private role record");
    rig.dir = next;
    let path = rig.dir.join("meta");
    let old = fs::read_to_string(&path).expect("seat meta");
    fs::write(
        path,
        old.replacen(
            &format!("session={}\n", rig.session),
            "session=orchestrator\n",
            1,
        ),
    )
    .expect("canonical role name");
    ORCH.clone_into(&mut rig.session);
    seal(&rig, &uuid_of(ORCH));
    let home = rig.scratch.join("claude-home");
    fs::create_dir(&home).expect("private Claude config home");
    fs::write(
        home.join("settings.json"),
        r#"{"voice":{"enabled":true,"mode":"hold","autoSubmit":false}}"#,
    )
    .expect("private hold settings");
    rig
}

fn record_hold_launch(rig: &Rig) {
    use std::io::Write as _;

    fs::OpenOptions::new()
        .append(true)
        .open(rig.dir.join("meta"))
        .expect("private role metadata")
        .write_all(b"voice.main=tok-rig\n")
        .expect("bound CLI hold override");
}

fn with_draft(words: &str) -> String {
    IDLE.replace("❯\u{a0}", &format!("❯\u{a0}{words}"))
        .replace(" · ← for agents", "")
}

fn composer_cursor(frame: &str, column: usize) -> String {
    let row = frame
        .lines()
        .position(|line| line.starts_with('❯'))
        .expect("fixture composer row")
        + 1;
    format!("{frame}\x1b[{row};{column}H")
}

fn with_footer(footer: &str) -> String {
    let (frame, mode) = IDLE
        .rsplit_once(" · ← for agents")
        .expect("fixture idle footer");
    assert_eq!(mode.trim(), "", "fixture footer is the final row");
    format!("{frame} · {footer}\n")
}

fn rig_world(rig: &Rig) -> ManuallyDrop<World> {
    // The delivery rig already owns teardown of the socket and state root.
    ManuallyDrop::new(World {
        root: rig.scratch.clone(),
        sock: rig.sock.clone(),
    })
}

fn input(rig: &Rig, pane: &str, bytes: &str) {
    checked(rig, &["send-keys", "-t", pane, "-l", "--", bytes]);
}

fn pane_frame(rig: &Rig, pane: &str) -> String {
    checked(rig, &["capture-pane", "-p", "-t", pane])
}

fn until_screen(rig: &Rig, pane: &str, why: &str, test: impl Fn(&str) -> bool) -> String {
    await_evidence(PANE_WAIT, || {
        let screen = pane_frame(rig, pane);
        test(&screen).then_some(screen)
    })
    .unwrap_or_else(|| panic!("{why}:\n{}", pane_frame(rig, pane)))
}

fn app(rig: &Rig, home: &str) -> String {
    checked(
        rig,
        &[
            "set-environment",
            "-t",
            &format!("={home}:"),
            "AE_SESSION",
            home,
        ],
    );
    let root = rig.scratch.display();
    let config = rig.scratch.join("config");
    let gates = rig.scratch.join("gates");
    fs::create_dir(&gates).expect("private app read gates");
    fs::write(&config, "[workspace]\nchat = app\n").expect("private config");
    let pane = checked(
        rig,
        &[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &format!("={home}:"),
            "env",
            "-u",
            "CLAUDE_CONFIG_DIR",
            "-u",
            "CODEX_HOME",
            "TMUX=",
            "TMUX_PANE=",
            "TERM=xterm-256color",
            "AE_NO_AUTOSTART=1",
            &format!("HOME={root}"),
            &format!("TMUX_TMPDIR={root}"),
            &format!("AE_HOME={root}"),
            &format!("CONFIG_FILE={}", config.display()),
            "AE_TMUX_SERVER_KIND=socket",
            &format!("AE_TMUX_SERVER={}", rig.sock.display()),
            "AE_SEND_DEFER_SEC=0",
            &format!("AE_TEST_APP_READ_GATE={}", gates.display()),
            env!("CARGO_BIN_EXE_ae"),
            "app",
            home,
        ],
    );
    let pane = pane.trim().to_owned();
    until_screen(rig, &pane, "setup: the app reads its home", |screen| {
        screen.contains(&format!("to {home}")) && screen.contains("Enter writes")
    });
    input(rig, &pane, "O");
    until_screen(
        rig,
        &pane,
        "setup: the real panel proved its main pane",
        |screen| {
            screen
                .lines()
                .next()
                .is_some_and(|line| line.contains("pane") && line.contains("F full"))
        },
    );
    pane
}

fn mic(header: &str) -> Option<usize> {
    header
        .find(['🎙', '🎤'])
        .map(|byte| ratatui_core::text::Span::raw(&header[..byte]).width() + 1)
}

fn panel_header(screen: &str) -> &str {
    screen.lines().next().expect("panel header row")
}

fn status(screen: &str, label: &str) -> bool {
    let header = panel_header(screen);
    let Some((_, status)) = header.split_once(['🎙', '🎤']) else {
        return false;
    };
    let shown = status
        .trim_start_matches('\u{fe0f}')
        .split_whitespace()
        .next();
    matches!(
        (label, shown),
        ("unknown", Some("?")) | ("recording", Some("rec"))
    ) || shown.is_some_and(|word| word.eq_ignore_ascii_case(label))
}

fn press_mic(rig: &Rig, pane: &str, shown: &str) {
    let col = mic(panel_header(shown)).expect("the mic is drawn in the panel header beside pane");
    input(rig, pane, &format!("\x1b[<0;{col};1M"));
}

fn untouched(rig: &Rig, decoy: &Seat) {
    // The witness arrives AFTER the app action. Allowing no preceding bytes
    // tests that earlier input could not have gone to the selected seat.
    const BARRIER: &str = "d54 fixture barrier";
    input(rig, &decoy.pane, BARRIER);
    let saw = await_evidence(PANE_WAIT, || {
        let bytes = fs::read(&decoy.receipt).expect("decoy receipt");
        bytes.ends_with(BARRIER.as_bytes()).then_some(bytes)
    });
    assert_eq!(
        saw.as_deref(),
        Some(BARRIER.as_bytes()),
        "the decoy got only its test barrier"
    );
}

fn active_sibling(rig: &Rig) -> Seat {
    let receipt = rig.scratch.join("sibling-receipt");
    fs::write(&receipt, "").expect("private sibling receipt");
    let command = format!("stty raw -echo; exec cat -u > {}", receipt.display());
    let sibling = checked(
        rig,
        &[
            "split-window",
            "-d",
            "-h",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &rig.pane,
            &command,
        ],
    );
    let pane = sibling.trim().to_owned();
    super::pane_command(
        |args| rig.tmux(args),
        &pane,
        std::time::Instant::now() + PANE_WAIT,
        |command| command == "cat",
    );
    checked(rig, &["select-pane", "-t", &pane]);
    let active = checked(
        rig,
        &[
            "display-message",
            "-p",
            "-t",
            "=orchestrator:",
            "#{pane_id}",
        ],
    );
    assert_eq!(active.trim(), pane, "the decoy is the active pane");
    Seat {
        pane,
        dir: rig.dir.clone(),
        receipt,
    }
}

#[test]
fn an_idle_frame_without_a_voice_hint_is_unknown() {
    let rig = orchestrator("voice-unknown", "claude", IDLE);
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    until_screen(&rig, &pane, "idle has no positive voice hint", |s| {
        status(s, "unknown")
    });
    assert!(rig.wire().is_empty(), "passive reads send no bytes");
    untouched(&rig, &api);
}

fn version_refusal(rig: &Rig, pane: &str, before: &str) {
    let after = until_screen(
        rig,
        pane,
        "the click explains that this installed version cannot dictate",
        |screen| {
            screen.lines().skip(1).any(|row| {
                !before.lines().any(|prior| prior == row)
                    && row.contains("dictation")
                    && row.contains("not available in this version")
            })
        },
    );
    assert!(
        after.lines().skip(1).all(|row| {
            before.lines().any(|prior| prior == row) || !row.to_lowercase().contains("relaunch")
        }),
        "an inactive version does not promise a relaunch helps"
    );
}

#[test]
fn before_measured_recording_states_the_mic_refuses_without_a_key() {
    let rig = orchestrator("voice-pending", "claude", &composer_cursor(IDLE, 3));
    record_hold_launch(&rig);
    let sibling = active_sibling(&rig);
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    let shown = until_screen(&rig, &pane, "the idle control is visible", |s| {
        status(s, "unknown")
    });
    press_mic(&rig, &pane, &shown);
    version_refusal(&rig, &pane, &shown);
    rig.frame_barrier();
    assert!(
        rig.wire().is_empty(),
        "the refused click has no key or paste effect"
    );
    untouched(&rig, &sibling);
    untouched(&rig, &api);
}

#[test]
fn the_mic_explains_an_older_launch_without_voice_proof() {
    let rig = orchestrator("voice-old-launch", "claude", &composer_cursor(IDLE, 3));
    let meta = fs::read_to_string(rig.dir.join("meta")).expect("private launch metadata");
    assert!(meta.lines().any(|row| row.starts_with("launch_id.main=")));
    assert!(!meta.lines().any(|row| row.starts_with("voice.main=")));
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    let shown = until_screen(&rig, &pane, "the older launch has a mic", |s| {
        status(s, "unknown")
    });
    press_mic(&rig, &pane, &shown);
    version_refusal(&rig, &pane, &shown);
    rig.frame_barrier();
    assert!(rig.wire().is_empty(), "the older launch receives no bytes");
    untouched(&rig, &api);
}

#[test]
fn a_same_name_replacement_does_not_inherit_the_drawn_control() {
    let rig = orchestrator("voice-replaced", "claude", IDLE);
    record_hold_launch(&rig);
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    let off = until_screen(&rig, &pane, "idle is unknown before any voice input", |s| {
        status(s, "unknown")
    });
    let gate = crate::app_writer_spec::Hold::new(&rig.scratch.join("gates"), "@app-keys");
    press_mic(&rig, &pane, &off);
    gate.held();
    seal(&rig, "99999999-1111-4111-8111-111111111111");
    drop(gate);
    version_refusal(&rig, &pane, &off);
    assert!(rig.wire().is_empty(), "the replacement receives no bytes");
    untouched(&rig, &api);
}

#[test]
fn mic_refuses_a_tap_mode_pane_without_a_bound_hold_override_before_any_key() {
    let rig = orchestrator("voice-tap-mode", "claude", &composer_cursor(IDLE, 3));
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    let shown = until_screen(&rig, &pane, "idle has no positive voice hint", |s| {
        status(s, "unknown")
    });

    // The user file permits hold mode. A later local settings file has higher
    // precedence and denies it, even while the drawn app frame is unchanged.
    let workspace = rig.scratch.join(".claude");
    fs::create_dir(&workspace).expect("private project settings");
    fs::write(
        workspace.join("settings.local.json"),
        r#"{"voice":{"enabled":true,"mode":"tap","autoSubmit":false}}"#,
    )
    .expect("higher precedence tap override");
    let mut meta = fs::read_to_string(rig.dir.join("meta")).expect("role meta");
    meta.push_str("voice.main=tok-before-relaunch\n");
    fs::write(rig.dir.join("meta"), meta).expect("stale launch proof");
    press_mic(&rig, &pane, &shown);
    version_refusal(&rig, &pane, &shown);
    rig.frame_barrier();
    assert!(rig.wire().is_empty(), "a tap-mode seat receives no key");
    untouched(&rig, &api);
}

#[test]
fn mic_leaves_an_existing_composer_draft_intact() {
    let rig = orchestrator(
        "voice-occupied",
        "claude",
        &with_draft("My existing draft."),
    );
    record_hold_launch(&rig);
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    let shown = until_screen(&rig, &pane, "mic is shown next to the proved pane", |s| {
        mic(panel_header(s)).is_some()
    });
    press_mic(&rig, &pane, &shown);
    version_refusal(&rig, &pane, &shown);
    rig.frame_barrier();
    assert!(
        rig.wire().is_empty(),
        "no voice probe or cleanup touches owner text"
    );
    assert!(pane_frame(&rig, &rig.pane).contains("My existing draft."));
    untouched(&rig, &api);
}

#[test]
fn an_empty_composer_in_a_busy_frame_does_not_start_hold() {
    let busy = include_str!("../../fixtures/harness-state/claude-busy-167x40.txt");
    let rig = orchestrator("voice-busy", "claude", &composer_cursor(busy, 3));
    record_hold_launch(&rig);
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    let shown = until_screen(&rig, &pane, "mic is shown next to the proved pane", |s| {
        mic(panel_header(s)).is_some()
    });
    press_mic(&rig, &pane, &shown);
    version_refusal(&rig, &pane, &shown);
    rig.frame_barrier();
    assert!(rig.wire().is_empty(), "a busy Claude receives no voice key");
    untouched(&rig, &api);
}

fn disabled_mic(tool: &str, frame: &str) {
    let rig = orchestrator(&format!("voice-{tool}"), tool, frame);
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    let disabled = until_screen(
        &rig,
        &pane,
        "the unsupported tool still has a mic symbol",
        |s| mic(panel_header(s)).is_some(),
    );
    assert!(
        !status(&disabled, "unknown"),
        "the disabled header is distinct from the usable unknown state"
    );
    press_mic(&rig, &pane, &disabled);
    version_refusal(&rig, &pane, &disabled);
    assert!(
        rig.wire().is_empty(),
        "the disabled control sends no {tool} key"
    );
    untouched(&rig, &api);
}

#[test]
fn codex_mic_is_drawn_disabled_with_a_reason() {
    disabled_mic(
        "codex",
        include_str!("../../fixtures/codex-composer/codex-idle-0.155.1-200x40.esc"),
    );
}

#[test]
fn opencode_mic_is_drawn_disabled_with_a_reason() {
    disabled_mic(
        "opencode",
        include_str!("../../fixtures/runtime-identity/opencode-composed-plain-80x24.txt"),
    );
}

#[test]
fn grok_mic_is_drawn_disabled_with_a_reason() {
    disabled_mic(
        "grok",
        include_str!("../../fixtures/runtime-identity/grok-transcript-only.txt"),
    );
}
