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

mod sidecar;
mod warming;

const ORCH: &str = "orchestrator";
const IDLE: &str = include_str!("../../fixtures/harness-state/claude-idle-167x40.txt");
const WARMUP: &str = "keep holding…";
const HOLD: &str =
    include_str!("../../fixtures/harness-state/claude-voice-holding-2.1.332-120x41.esc");
const INTERIM: &str =
    include_str!("../../fixtures/harness-state/claude-voice-interim-2.1.332-120x41.esc");
const TRANSCRIBING: &str =
    include_str!("../../fixtures/harness-state/claude-voice-transcribing-2.1.332-120x41.txt");
const WORDS: &str = "Review that note in Wien and say Grüße.";
const PRIOR_DRAFT: &str = "Keep my note.";
const UNSENT_NOTICE: &str = "dictation: the text is in the orchestrator input, not sent";

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

fn redraw_voice_frame(rig: &Rig, frame: &str) {
    fs::write(rig.scratch.join("control"), frame).expect("private Claude redraw");
    rig.frame_barrier();
}

fn arm_voice_start(rig: &Rig, after_burst: &str) {
    rig.arm_frame("onwarm", &with_footer(WARMUP));
    rig.arm_frame("onhold", after_burst);
}

fn voice_wire(rig: &Rig) -> Vec<u8> {
    fs::read(rig.received.with_extension("wire")).expect("private pane wire")
}

fn wait_for_voice_spaces(rig: &Rig, pane: &str, min: usize) {
    assert!(
        await_evidence(PANE_WAIT, || {
            (voice_wire(rig).len() >= min).then_some(())
        })
        .is_some(),
        "the mic did not send {min} Spaces to the proved pane, wire {:?}:\n{}",
        voice_wire(rig),
        pane_frame(rig, pane)
    );
    assert_eq!(&voice_wire(rig)[..min], vec![b' '; min]);
}

fn voice_report(rig: &Rig, pane: &str, before: &str, why: &str, message: &str) -> String {
    until_screen(rig, pane, why, |screen| {
        screen.lines().skip(1).any(|row| {
            !before.lines().any(|old| old == row) && row.to_lowercase().contains(message)
        })
    })
}

#[test]
fn the_bound_hold_mic_leaves_the_unsent_transcript_in_claude() {
    let idle = composer_cursor(IDLE, 3);
    let rig = orchestrator("voice-hold-start", "claude", &idle);
    record_hold_launch(&rig);
    let project = rig.scratch.join(".claude");
    fs::create_dir(&project).expect("private project settings");
    fs::write(
        project.join("settings.local.json"),
        r#"{"voice":{"enabled":true,"mode":"tap","autoSubmit":true}}"#,
    )
    .expect("private tap settings below the recorded CLI layer");
    let sibling = active_sibling(&rig);
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    checked(&rig, &["send-keys", "-t", &pane, "Enter"]);
    until_screen(&rig, &pane, "the owner is editing the panel draft", |s| {
        s.contains("Enter sends")
    });
    input(&rig, &pane, PRIOR_DRAFT);
    let shown = until_screen(&rig, &pane, "the idle control is visible", |s| {
        status(s, "unknown") && s.contains(PRIOR_DRAFT)
    });
    arm_voice_start(&rig, HOLD);
    rig.arm_frame("onrepeat", INTERIM);
    press_mic(&rig, &pane, &shown);
    wait_for_voice_spaces(&rig, &pane, 6);
    until_screen(
        &rig,
        &rig.pane,
        "the measured interim waveform is drawn",
        |s| s.contains("Term.▅"),
    );
    let after_interim = voice_wire(&rig).len();
    wait_for_voice_spaces(&rig, &pane, after_interim + 1);
    let recording = until_screen(&rig, &pane, "the measured hold appears as recording", |s| {
        status(s, "recording")
    });
    press_mic(&rig, &pane, &recording);
    let stopped = voice_report(
        &rig,
        &pane,
        &recording,
        "stop releases the hold while the frame still shows Holding",
        "dictation: stopping",
    );
    let at_stop = rig.wire();
    assert!(at_stop.len() >= 6 && at_stop.iter().all(|key| *key == b' '));

    // Keep the frame positive through stop, then reproduce the two observed
    // post-release states. A stale visible waveform must not re-arm the hold.
    redraw_voice_frame(&rig, TRANSCRIBING);
    let processing = voice_report(
        &rig,
        &pane,
        &stopped,
        "the controller reads Transcribing before the insert",
        "dictation: transcribing",
    );
    assert!(!processing.contains(UNSENT_NOTICE));
    let cells = ratatui_core::text::Span::raw(WORDS).width();
    redraw_voice_frame(&rig, &composer_cursor(&with_draft(WORDS), cells + 3));
    let result = voice_report(
        &rig,
        &pane,
        &processing,
        "the notice explains where the text is",
        UNSENT_NOTICE,
    );
    assert!(result.contains("Enter sends"));
    let draft_before = shown
        .lines()
        .rev()
        .take(3)
        .find(|row| row.contains(PRIOR_DRAFT))
        .expect("the panel draft before the click");
    let draft = result.lines().rev().take(3);
    assert!(draft.clone().any(|row| row == draft_before));
    assert!(draft.into_iter().all(|row| !row.contains(WORDS)));
    assert_eq!(
        rig.wire(),
        at_stop,
        "no key, clear, paste or Enter after the stop"
    );
    let source = pane_frame(&rig, &rig.pane);
    assert!(
        source
            .lines()
            .any(|line| line.strip_prefix("❯\u{a0}") == Some(WORDS)),
        "the exact transcript remains in Claude: {source}"
    );
    assert!(
        rig.submitted().is_empty(),
        "the transcript was never submitted"
    );
    assert_eq!(rig.enter_count(), 0, "no Enter was sent to Claude");
    let history = rig.transitions();
    assert!(history.starts_with("onwarm\napp-key\n"), "{history}");
    assert!(history.contains("onhold\n"), "{history}");
    untouched(&rig, &sibling);
    untouched(&rig, &api);
}

#[test]
fn mic_refuses_an_unread_voice_frame_even_with_a_proved_launch() {
    use ae::harness_state::{Dictation, dictation};
    let (composer, _) = IDLE.rsplit_once("  ⏵⏵").expect("measured idle mode row");
    let unread = format!("{composer}unknown local footer\n");
    assert_eq!(
        dictation(&unread, ae::tool::ToolKind::Claude),
        Dictation::Unread
    );
    let rig = orchestrator("voice-unread-mic", "claude", &composer_cursor(&unread, 3));
    record_hold_launch(&rig);
    let sibling = active_sibling(&rig);
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    let shown = until_screen(&rig, &pane, "the unread voice frame is unknown", |s| {
        status(s, "unknown")
    });
    press_mic(&rig, &pane, &shown);
    voice_report(
        &rig,
        &pane,
        &shown,
        "the mic explains the unread voice frame",
        "refused: dictation",
    );
    rig.frame_barrier();
    assert!(
        rig.wire().is_empty(),
        "the refused click has no key or paste effect"
    );
    untouched(&rig, &sibling);
    untouched(&rig, &api);
}

#[test]
fn mic_refuses_an_unrecognized_pane_state_with_a_clear_voice_footer() {
    use ae::harness_state::{Dictation, HarnessState, classify, dictation};
    use ae::tool::ToolKind;

    const REFUSAL: &str = "the orchestrator pane state is not recognized; no key was sent";
    let unfamiliar = IDLE.replacen(
        "✻ Crunched for 16s · done 4:01 PM",
        "Unrecognized vendor state",
        1,
    );
    assert_ne!(unfamiliar, IDLE);
    assert_eq!(dictation(&unfamiliar, ToolKind::Claude), Dictation::Clear);
    assert_eq!(
        classify(&unfamiliar, ToolKind::Claude),
        HarnessState::Unknown
    );
    let rig = orchestrator(
        "voice-unknown-pane",
        "claude",
        &composer_cursor(&unfamiliar, 3),
    );
    record_hold_launch(&rig);
    let sibling = active_sibling(&rig);
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    let shown = until_screen(&rig, &pane, "the mic is visible for the bound pane", |s| {
        mic(panel_header(s)).is_some()
    });
    assert!(!shown.contains("the orchestrator pane state is not recognized"));
    press_mic(&rig, &pane, &shown);
    until_screen(
        &rig,
        &pane,
        "the click explains the unrecognized pane state",
        |s| {
            let panel = s
                .lines()
                .filter_map(|row| row.rsplit_once(['│', '┃']))
                .map(|(_, row)| row.trim())
                .collect::<Vec<_>>()
                .join(" ");
            panel.contains(REFUSAL)
        },
    );
    assert!(
        rig.wire().is_empty(),
        "an unrecognized state receives no input"
    );
    untouched(&rig, &sibling);
    untouched(&rig, &api);
}

#[test]
fn no_warmup_leaves_the_two_probe_spaces_untouched() {
    let idle = composer_cursor(IDLE, 3);
    let rig = orchestrator("voice-no-warmup", "claude", &idle);
    record_hold_launch(&rig);
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    let shown = until_screen(&rig, &pane, "the idle control is visible", |s| {
        status(s, "unknown")
    });
    // With voice off, the two Spaces look blank in capture-pane -p. The
    // cursor is two cells later. Even this positive cursor witness may
    // not authorize a destructive key in the notice-only controller.
    rig.arm_frame("onwarm", &composer_cursor(&with_draft("  "), 5));
    press_mic(&rig, &pane, &shown);
    voice_report(
        &rig,
        &pane,
        &shown,
        "start reports no warmup",
        "claude showed no hold warmup; two spaces may stay in the orchestrator input",
    );
    assert!(rig.transitions().contains("onwarm\n"));
    assert_eq!(rig.wire(), b"  ", "only the probe Spaces");
    assert!(
        rig.submitted().is_empty(),
        "nothing reaches Claude as a turn"
    );
    untouched(&rig, &api);
}

#[test]
fn a_changed_composer_during_the_probe_receives_no_cleanup_or_more_space() {
    let idle = composer_cursor(IDLE, 3);
    let rig = orchestrator("voice-probe-edit", "claude", &idle);
    record_hold_launch(&rig);
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    let shown = until_screen(&rig, &pane, "the idle control is visible", |s| {
        status(s, "unknown")
    });
    let foreign = "Owner edited during the probe.";
    rig.arm_frame(
        "onwarm",
        &composer_cursor(&with_draft(foreign), foreign.len() + 3),
    );
    press_mic(&rig, &pane, &shown);
    voice_report(
        &rig,
        &pane,
        &shown,
        "start reports the changed composer",
        "refused: dictation did not start",
    );
    assert_eq!(rig.wire(), b"  ", "only two Spaces reached the pane");
    assert!(pane_frame(&rig, &rig.pane).contains(foreign));
    untouched(&rig, &api);
}

#[test]
fn a_probe_with_no_cursor_advance_never_clears_an_apparently_empty_box() {
    let idle = composer_cursor(IDLE, 3);
    let rig = orchestrator("voice-probe-swallowed", "claude", &idle);
    record_hold_launch(&rig);
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    let shown = until_screen(&rig, &pane, "the idle control is visible", |s| {
        status(s, "unknown")
    });
    rig.arm_frame("onwarm", &idle);
    press_mic(&rig, &pane, &shown);
    voice_report(
        &rig,
        &pane,
        &shown,
        "start reports an unproved probe",
        "refused: dictation did not start",
    );
    assert_eq!(rig.wire(), b"  ", "no proof of new Spaces at the cursor");
    untouched(&rig, &api);
}

#[test]
fn a_lapsed_hold_leaves_a_wrapped_insert_unsent_in_claude() {
    let idle = composer_cursor(IDLE, 3);
    let rig = orchestrator("voice-wrapped", "claude", &idle);
    record_hold_launch(&rig);
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    checked(&rig, &["send-keys", "-t", &pane, "Enter"]);
    until_screen(&rig, &pane, "the owner is editing the panel draft", |s| {
        s.contains("Enter sends")
    });
    input(&rig, &pane, PRIOR_DRAFT);
    let shown = until_screen(&rig, &pane, "the idle control is visible", |s| {
        status(s, "unknown") && s.contains(PRIOR_DRAFT)
    });
    // The first repeat draws a label without a waveform. That frame still
    // blocks delivery and cannot authorize another Space.
    let label_only = HOLD.replacen('▁', "", 1);
    arm_voice_start(&rig, HOLD);
    rig.arm_frame("onrepeat", &label_only);
    press_mic(&rig, &pane, &shown);
    wait_for_voice_spaces(&rig, &pane, 6);
    until_screen(&rig, &rig.pane, "label-only frame after a repeat", |s| {
        s.contains("listening…") && !s.contains('▁')
    });
    redraw_voice_frame(&rig, TRANSCRIBING);
    let processing = voice_report(
        &rig,
        &pane,
        &shown,
        "the controller sees the actual Transcribing label before the insert",
        "dictation: transcribing",
    );
    assert!(!processing.contains(UNSENT_NOTICE));
    // This row is short of the border, yet the next word does not fit.
    // Display rows cannot distinguish a dropped separator from a newline.
    let first = "x".repeat(145);
    let second = "Donaudampfschifffahrtsgesellschaft";
    let wrapped = with_draft(&format!("{first}\n  {second}"));
    redraw_voice_frame(&rig, &wrapped);
    let reported = voice_report(
        &rig,
        &pane,
        &processing,
        "the notice explains where the text is",
        UNSENT_NOTICE,
    );
    let draft_before = shown
        .lines()
        .rev()
        .take(3)
        .find(|row| row.contains(PRIOR_DRAFT))
        .expect("the panel draft before the click");
    let draft = reported.lines().rev().take(3);
    assert!(draft.clone().any(|row| row == draft_before));
    assert!(draft.into_iter().all(|row| !row.contains(second)));
    assert_eq!(rig.wire(), b"      ", "no repeat, clear, paste or Enter");
    let source = pane_frame(&rig, &rig.pane);
    assert!(source.contains(&first) && source.contains(second));
    assert!(rig.submitted().is_empty(), "the remote draft stays unsent");
    untouched(&rig, &api);
}

fn stop_on_the_holding_frame(rig: &Rig, pane: &str) -> String {
    let idle = until_screen(rig, pane, "the proved idle mic is visible", |screen| {
        status(screen, "unknown")
    });
    arm_voice_start(rig, HOLD);
    press_mic(rig, pane, &idle);
    wait_for_voice_spaces(rig, pane, 6);
    let recording = until_screen(rig, pane, "the mic shows the live hold", |screen| {
        status(screen, "recording")
    });
    press_mic(rig, pane, &recording);
    voice_report(
        rig,
        pane,
        &recording,
        "the second click acknowledges stop while the frame still shows Holding",
        "dictation: stopping",
    )
}

fn assert_no_unsent_claim(screen: &str) {
    assert!(!screen.contains(UNSENT_NOTICE), "{screen}");
    assert!(
        !screen.to_lowercase().contains("nothing was sent"),
        "the result only describes what the app observed: {screen}"
    );
}

#[test]
fn an_empty_result_reports_only_that_no_text_is_visible() {
    let idle = composer_cursor(IDLE, 3);
    let rig = orchestrator("voice-empty-result", "claude", &idle);
    record_hold_launch(&rig);
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    let stopped = stop_on_the_holding_frame(&rig, &pane);
    let wire = rig.wire();
    redraw_voice_frame(&rig, TRANSCRIBING);
    let processing = voice_report(
        &rig,
        &pane,
        &stopped,
        "the controller observes Transcribing before the empty result",
        "dictation: transcribing",
    );
    redraw_voice_frame(&rig, &idle);
    let result = voice_report(
        &rig,
        &pane,
        &processing,
        "the empty result only reports the observed input",
        "no text is visible in the orchestrator input",
    );
    assert_no_unsent_claim(&result);
    assert_eq!(
        rig.wire(),
        wire,
        "the empty-result observation sends no key"
    );
    assert!(rig.submitted().is_empty());
    untouched(&rig, &api);
}

#[test]
fn a_foreign_draft_without_transcribing_reports_an_unproved_result() {
    const FOREIGN: &str = "Text typed directly in Claude.";
    let rig = orchestrator("voice-unproved-result", "claude", &composer_cursor(IDLE, 3));
    record_hold_launch(&rig);
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    let stopped = stop_on_the_holding_frame(&rig, &pane);
    let wire = rig.wire();
    redraw_voice_frame(&rig, &with_draft(FOREIGN));
    let result = voice_report(
        &rig,
        &pane,
        &stopped,
        "the foreign draft without Transcribing receives an unproved result",
        "ae could not verify the result",
    );
    assert!(result.contains("check the orchestrator input"));
    assert_no_unsent_claim(&result);
    assert_eq!(rig.wire(), wire, "the unproved result sends no key");
    assert!(pane_frame(&rig, &rig.pane).contains(FOREIGN));
    assert!(rig.submitted().is_empty());
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
    let reported = voice_report(
        &rig,
        &pane,
        &shown,
        "the click reports the missing hold proof",
        "refused: this launch lacks ae's hold-mode dictation proof (docs/app.md)",
    );
    assert!(
        reported.lines().skip(1).all(|row| {
            shown.lines().any(|prior| prior == row) || !row.to_lowercase().contains("relaunch")
        }),
        "the click does not claim a relaunch fixes an unproved launch"
    );
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
    until_screen(&rig, &pane, "the held panel sees the replacement", |s| {
        s.to_lowercase().contains("replaced")
    });
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
    let reported = voice_report(
        &rig,
        &pane,
        &shown,
        "the click reports the missing hold proof",
        "refused: this launch lacks ae's hold-mode dictation proof (docs/app.md)",
    );
    assert!(
        reported.lines().skip(1).all(|row| {
            shown.lines().any(|prior| prior == row) || !row.to_lowercase().contains("relaunch")
        }),
        "the click does not claim a relaunch fixes an unproved launch"
    );
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
    until_screen(
        &rig,
        &pane,
        "start explains why a pre-existing draft blocks it",
        |s| {
            s.lines().skip(1).any(|row| {
                let row = row.to_lowercase();
                row.contains("composer") || row.contains("draft")
            })
        },
    );
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
    until_screen(&rig, &pane, "the mic reports that Claude is busy", |s| {
        s.lines().skip(1).any(|row| {
            let row = row.to_lowercase();
            row.contains("busy") || row.contains("idle") || row.contains("ready")
        })
    });
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
    until_screen(
        &rig,
        &pane,
        "the tool exposes its no-dictation reason",
        |s| {
            s.lines().any(|line| {
                let line = line.to_lowercase();
                line.contains("dictation")
                    && (line.contains(tool)
                        || line.contains("unsupported")
                        || line.contains("unavailable"))
            })
        },
    );
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

fn dictation_children(rig: &Rig) -> Vec<u32> {
    checked(
        rig,
        &[
            "list-clients",
            "-F",
            "#{client_control_mode}|#{client_termname}|#{session_name}|#{client_pid}",
        ],
    )
    .lines()
    .filter_map(|row| row.strip_prefix("1|ae-dictation|orchestrator|"))
    .map(|pid| pid.parse().expect("private control-client PID"))
    .collect()
}

fn start_owned_dictation(rig: &Rig) -> (String, String, u32) {
    record_hold_launch(rig);
    let _api = rig_world(rig).session("api", "tui", "");
    let pane = app(rig, "api");
    let idle = until_screen(
        rig,
        &pane,
        "the panel renders its passive idle mic",
        |screen| status(screen, "unknown"),
    );
    assert!(
        dictation_children(rig).is_empty(),
        "idle panel reads must not attach a control client"
    );
    arm_voice_start(rig, HOLD);
    press_mic(rig, &pane, &idle);
    wait_for_voice_spaces(rig, &pane, 6);
    let recording = until_screen(rig, &pane, "the panel renders a live recording", |screen| {
        status(screen, "recording")
    });
    let pids = dictation_children(rig);
    assert_eq!(pids.len(), 1, "one owned child while holding: {pids:?}");
    (pane, recording, pids[0])
}

#[test]
fn the_panel_starts_one_child_and_reaps_it_after_read_only_settle() {
    let rig = orchestrator("sidecar-panel-stop", "claude", &composer_cursor(IDLE, 3));
    let (pane, recording, pid) = start_owned_dictation(&rig);
    press_mic(&rig, &pane, &recording);
    let stopped = voice_report(
        &rig,
        &pane,
        &recording,
        "stop acknowledges the write fence",
        "dictation: stopping",
    );
    let wire = rig.wire();
    redraw_voice_frame(&rig, TRANSCRIBING);
    let processing = voice_report(
        &rig,
        &pane,
        &stopped,
        "the worker enters read-only settle",
        "dictation: transcribing",
    );
    assert_eq!(
        dictation_children(&rig),
        [pid],
        "settle uses the same owned child"
    );
    let cells = ratatui_core::text::Span::raw(WORDS).width();
    redraw_voice_frame(&rig, &composer_cursor(&with_draft(WORDS), cells + 3));
    voice_report(
        &rig,
        &pane,
        &processing,
        "settle leaves the result in Claude",
        UNSENT_NOTICE,
    );
    assert!(
        await_evidence(PANE_WAIT, || dictation_children(&rig)
            .is_empty()
            .then_some(()))
        .is_some(),
        "the finished controller must leave no attached client"
    );
    assert_eq!(rig.wire(), wire, "the read-only settle sent no key");
    assert!(rig.submitted().is_empty());
}

#[test]
fn normal_app_exit_during_hold_removes_the_owned_client() {
    let rig = orchestrator("sidecar-panel-quit", "claude", &composer_cursor(IDLE, 3));
    let (pane, _, _) = start_owned_dictation(&rig);
    input(&rig, &pane, "O");
    input(&rig, &pane, "qq");
    assert!(
        await_evidence(PANE_WAIT, || {
            let app_closed = !rig.tmux(&["capture-pane", "-p", "-t", &pane]).0;
            (app_closed && dictation_children(&rig).is_empty()).then_some(())
        })
        .is_some(),
        "the app must exit and leave no sidecar attached to the proved pane"
    );
    assert!(rig.wire().iter().all(|key| *key == b' '));
    assert!(rig.submitted().is_empty());
}
