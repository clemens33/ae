//! D54 delivery gates on the measured Claude Code 2.1.332 warmup frame.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use super::{
    IDLE, PANE_WAIT, Rig, WARMUP, app, await_evidence, checked, input, orchestrator,
    record_hold_launch, rig_world, until_screen, untouched, with_footer,
};

const INBOUND: &str = "Owner request must wait for dictation.";

fn redrawn(rig: &Rig, frame: &str) {
    fs::write(rig.scratch.join("control"), frame).expect("fixture frame");
    rig.frame_barrier();
}

fn enter(rig: &Rig, pane: &str) {
    checked(rig, &["send-keys", "-t", pane, "Enter"]);
}

#[test]
fn an_admitted_owner_ask_to_the_warming_orchestrator_keeps_the_draft_and_sends_no_input() {
    let rig = orchestrator("voice-owner", "claude", &with_footer(WARMUP));
    let api = rig_world(&rig).session("api", "tui", "");
    let pane = app(&rig, "api");
    enter(&rig, &pane);
    until_screen(
        &rig,
        &pane,
        "setup: owner can compose in the orchestrator panel",
        |s| s.contains("Enter sends"),
    );
    input(&rig, &pane, INBOUND);
    until_screen(&rig, &pane, "setup: owner draft is visible", |s| {
        s.contains(INBOUND)
    });
    enter(&rig, &pane);
    let journal = await_evidence(PANE_WAIT, || {
        let events = rig.events();
        (events.contains("\"action\":\"delivery-abandoned\"")
            || events.contains("\"action\":\"ask\""))
        .then_some(events)
    })
    .expect("the owner request reaches actual dispatch and gets a delivery outcome");
    assert!(
        journal.contains("\"action\":\"delivery-abandoned\""),
        "{journal}"
    );
    assert!(
        journal.to_lowercase().contains("dictat"),
        "dictation-specific hold: {journal}"
    );
    assert!(!journal.contains("\"action\":\"ask\""), "{journal}");
    assert!(
        rig.wire().is_empty(),
        "the orchestrator receives no paste or Enter"
    );
    until_screen(
        &rig,
        &pane,
        "the input stays as a draft after refusal",
        |s| s.lines().rev().take(5).any(|row| row.contains(INBOUND)),
    );
    untouched(&rig, &api);
}

fn overview(rig: &Rig, body: &str, env: &[(&str, &str)]) -> (Option<i32>, String) {
    let watchdog = checked(
        rig,
        &[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            "=orchestrator:",
            "sleep",
            "120",
        ],
    );
    let output = super::super::ae()
        .env("AE_HOME", &rig.scratch)
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", watchdog.trim())
        .env("_AE_EVENT_ACTION", "sweep-nudge")
        .env("_AE_EVENT_SUMMARY", "fleet overview changed")
        .env("AE_SENDER_OVERRIDE", "watchdog")
        .env("_AE_EXPECT_SLOT", "main")
        .env("_AE_EXPECT_LAUNCH", "tok-rig")
        .env("AE_SEND_DEFER_SEC", "0")
        .envs(env.iter().copied())
        .arg(ae::cli::SEND)
        .arg(&rig.dir)
        .args(["tui", body])
        .output()
        .expect("watchdog delivery helper");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn overview_holds_for_live_claude_warming_and_delivers_when_clear() {
    let rig = orchestrator("voice-overview", "claude", &with_footer(WARMUP));
    let (code, why) = overview(&rig, "overview marker", &[]);
    assert_ne!(code, Some(0), "a warming pane refuses overview: {why}");
    rig.frame_barrier();
    assert!(rig.wire().is_empty(), "no overview input while warming");

    // The same text printed by the transcript has no bearing on a clear box.
    redrawn(&rig, &format!("A colleague quoted {WARMUP}\n{IDLE}"));
    let (code, why) = overview(&rig, "overview marker", &[]);
    assert_eq!(
        code,
        Some(0),
        "same admitted route after voice holds ends: {why}"
    );
    assert!(rig.submitted().contains("overview marker"));
    assert_eq!(rig.enter_count(), 1, "exactly one send after recovery");
}

#[test]
fn overview_requires_a_readable_voice_footer_only_on_the_proved_claude_seat() {
    let (composer, _) = IDLE.rsplit_once("  ⏵⏵").expect("measured mode row");
    let frame = format!("{composer}unknown local footer\n");
    let unproved = orchestrator("voice-unread-control", "claude", &frame);
    let (code, why) = overview(&unproved, "unproved unread marker", &[]);
    assert_eq!(
        code,
        Some(0),
        "the same legacy composer is deliverable: {why}"
    );
    assert!(unproved.submitted().contains("unproved unread marker"));
    assert_eq!(unproved.enter_count(), 1);

    let rig = orchestrator("voice-unread", "claude", &frame);
    record_hold_launch(&rig);
    let (code, why) = overview(&rig, "proved unread marker", &[]);
    rig.frame_barrier();
    assert_eq!(
        rig.wire(),
        b"",
        "no input to the proved seat with an unreadable voice frame: {why}"
    );
    assert_ne!(code, Some(0), "the proved seat refused the overview");
}

#[test]
fn claude_warming_is_observed_as_busy_and_scoped_to_the_current_footer() {
    use ae::harness_state::{HarnessState, classify};
    use ae::tool::ToolKind;

    assert_eq!(classify(IDLE, ToolKind::Claude), HarnessState::Idle);
    let in_transcript = format!("Earlier output quoted {WARMUP}\n{IDLE}");
    assert_eq!(
        classify(&in_transcript, ToolKind::Claude),
        HarnessState::Idle
    );
    for warming in [
        with_footer(WARMUP),
        with_footer(&format!("\x1b[38;5;246m{WARMUP}\x1b[0m")),
    ] {
        assert_eq!(classify(&warming, ToolKind::Claude), HarnessState::Busy);
        let muse = classify(&warming, ToolKind::Muse);
        assert_ne!(
            muse,
            HarnessState::Busy,
            "a Claude-only footer is not Muse warming"
        );
    }
}

fn retry(rig: &Rig, env: &[(&str, &str)]) -> (Option<i32>, String) {
    let output = super::super::ae()
        .env("AE_HOME", &rig.scratch)
        .env("_AE_EVENT_ACTION", "brief-retry")
        .env("AE_SEND_DEFER_SEC", "0")
        .envs(env.iter().copied())
        .arg(ae::cli::SEND)
        .arg(&rig.dir)
        .args(["tui", "the retry leg takes its body from the record"])
        .output()
        .expect("private brief retry leg");
    (
        output.status.code(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ),
    )
}

fn brief_record(rig: &Rig) -> std::path::PathBuf {
    let record = rig.dir.join("brief-retry.main.rec");
    fs::write(
        &record,
        format!(
            "brief-retry 1\nslot=main\nreference=spawn-main\npane={}\nlaunch_id=tok-rig\n\
             actor=tui\nattempts=0\ncreated={}\nphase=armed\nbody\nvoice brief marker",
            rig.pane,
            ae::time::Timestamp::now().epoch() - 30,
        ),
    )
    .expect("a normal retry record");
    fs::write(rig.dir.join("undelivered.tui.txt"), "voice brief marker").expect("preserved brief");
    record
}

#[test]
fn a_retry_that_already_sees_warming_does_not_spend_an_attempt() {
    let rig = orchestrator("voice-retry-visible", "claude", &with_footer(WARMUP));
    let record = brief_record(&rig);
    let original = fs::read(&record).expect("armed retry");
    let (code, why) = retry(&rig, &[]);
    assert_ne!(code, Some(0), "warming holds brief retry: {why}");
    assert_eq!(
        fs::read(&record).expect("brief stays on record"),
        original,
        "{why}"
    );
    rig.frame_barrier();
    assert!(
        rig.wire().is_empty(),
        "a visible warming receives no brief input"
    );
}

// The subprocess forwards all actual tmux operations. Immediately after a
// successful buffer load it publishes the warming frame on the real pane
// and waits for the fake Claude to redraw it. Overview and brief retry
// also require the kernel send lock at this point.
// The baseline's earlier idle read cannot accidentally observe the new frame.
const PRE_PASTE: &str = r#"#!/usr/bin/perl
use strict;
use warnings;
use Fcntl qw(:flock);
use Errno qw(EAGAIN EWOULDBLOCK);
my @real = ($ENV{D54_REAL_TMUX}, '-S', $ENV{D54_SOCKET});
my $socket = 0;
for (my $i = 0; $i + 1 < @ARGV; ++$i) {
    $socket = 1 if $ARGV[$i] eq '-S' && $ARGV[$i + 1] eq $ENV{D54_SOCKET};
}
my $buffer = $ENV{D54_BUFFER};
if ($socket && (grep { $_ eq 'load-buffer' } @ARGV)
    && (!defined($buffer) || grep { $_ eq $buffer } @ARGV)) {
    system { $real[0] } $real[0], @ARGV;
    die "private load-buffer failed: $?" if $?;
    if (defined $ENV{D54_LOCK}) {
        open my $lock, '>>', $ENV{D54_LOCK} or die "send lock: $!";
        die 'load-buffer must hold the target send lock' if flock($lock, LOCK_EX | LOCK_NB);
        die "send lock probe: $!" unless $! == EAGAIN || $! == EWOULDBLOCK;
        close $lock or die "send lock close: $!";
    }
    rename $ENV{D54_NEXT}, $ENV{D54_CONTROL} or die "publish voice frame: $!";
    open my $drain, '>', "$ENV{D54_CONTROL}.drain" or die "drain: $!";
    close $drain or die "drain close: $!";
    system { $real[0] } @real, 'wait-for', "$ENV{D54_CONTROL}.ack";
    die "redraw failed: $?" if $?;
    open my $trace, '>', $ENV{D54_TRACE} or die "trace: $!";
    print $trace "loaded", (defined $ENV{D54_LOCK} ? " under lock" : ""),
        ", then warming drawn before paste\n";
    close $trace or die "trace close: $!";
    exit 0;
}
exec { $real[0] } $real[0], @ARGV;
die "forward real tmux: $!";
"#;

fn before_paste_env(scratch: &Path, socket: &Path) -> Vec<(&'static str, String)> {
    let path = std::env::var_os("PATH").expect("fixture PATH");
    let real = std::env::split_paths(&path)
        .map(|dir| dir.join("tmux"))
        .find(|file| file.is_file())
        .expect("real tmux executable");
    let bin = scratch.join("before-paste");
    fs::create_dir(&bin).expect("private tmux wrapper directory");
    let wrapper = bin.join("tmux");
    fs::write(&wrapper, PRE_PASTE).expect("before-paste wrapper");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).expect("wrapper executable");
    let next = scratch.join("next-voice-frame");
    fs::write(&next, with_footer(WARMUP)).expect("next frame");
    vec![
        (
            "PATH",
            format!("{}:{}", bin.display(), path.to_string_lossy()),
        ),
        ("D54_REAL_TMUX", real.display().to_string()),
        ("D54_SOCKET", socket.display().to_string()),
        ("D54_NEXT", next.display().to_string()),
        ("D54_CONTROL", scratch.join("control").display().to_string()),
        ("D54_TRACE", scratch.join("d54-trace").display().to_string()),
    ]
}

#[test]
fn a_warming_frame_after_the_overview_buffer_load_prevents_paste() {
    let rig = orchestrator("voice-before-paste", "claude", IDLE);
    let mut setup = before_paste_env(&rig.scratch, &rig.sock);
    setup.push(("D54_LOCK", rig.send_lock_path().display().to_string()));
    let env: Vec<_> = setup
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let (code, why) = overview(&rig, "overview marker", &env);
    let trace = fs::read_to_string(rig.scratch.join("d54-trace"))
        .expect("the admitted overview reached the staged-buffer boundary");
    assert_eq!(
        trace,
        "loaded under lock, then warming drawn before paste\n"
    );
    rig.frame_barrier();
    assert!(rig.wire().is_empty(), "no paste or Enter: {code:?}: {why}");
}

#[test]
fn an_admitted_brief_retry_rechecks_warming_before_paste_and_recovers_once() {
    let rig = orchestrator("voice-retry-late", "claude", IDLE);
    let record = brief_record(&rig);
    let mut setup = before_paste_env(&rig.scratch, &rig.sock);
    setup.push(("D54_LOCK", rig.send_lock_path().display().to_string()));
    let env: Vec<_> = setup
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let (code, why) = retry(&rig, &env);
    let trace = fs::read_to_string(rig.scratch.join("d54-trace"))
        .expect("the admitted Launch reached the staged-buffer boundary");
    assert_eq!(
        trace,
        "loaded under lock, then warming drawn before paste\n"
    );
    assert_ne!(code, Some(0), "late voice holds Launch: {why}");
    assert!(
        rig.wire().is_empty(),
        "late warming receives no paste or Enter"
    );
    assert!(
        record.is_file(),
        "the brief stays armed because no text reached the pane: {why}"
    );

    redrawn(&rig, IDLE);
    let (code, why) = retry(&rig, &[]);
    assert_eq!(code, Some(0), "brief recovers once warming stops: {why}");
    assert!(!record.exists(), "the delivered brief is removed");
    assert_eq!(rig.submitted().matches("voice brief marker").count(), 1);
    assert_eq!(rig.enter_count(), 1);
}

#[test]
fn a_reseat_to_claude_keeps_the_seed_when_warming_starts_after_buffer_load() {
    let rig = crate::seat_relaunch::Rig::new("voice-seed");
    rig.seat_rows("spawned.0", "scout", "grok", "grok");
    let pane = rig.new_pane("spawned.0", "scout");
    rig.start(&pane, "spawned.0", "grok");
    rig.kill_tools(&pane);
    let control = rig.scratch.join("control");
    let receipt = rig.scratch.join("successor");
    let enters = rig.scratch.join("successor-enters");
    fs::write(&control, IDLE).expect("successor's initial Claude frame");
    // Use the existing frame fake under the reseat rig's Claude-named
    // interpreter. It draws the fixture and records input from the real pane.
    let args = format!(
        "unshift @ARGV, qw({} {} 200 claude 0 frame {});\n",
        receipt.display(),
        enters.display(),
        control.display()
    );
    let script =
        super::super::FAKE_TUI.replacen("use warnings;\n", &format!("use warnings;\n{args}"), 1);
    fs::write(rig.scratch.join("claude.pl"), script).expect("Claude successor fake");

    let mut env = before_paste_env(&rig.scratch, &rig.sock);
    env.push(("D54_BUFFER", "ae-launch-spawned.0".to_owned()));
    let output = super::super::ae()
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", &rig.main_pane)
        .env("AE_HOME", &rig.scratch)
        .envs(env)
        .args(["reseat", &rig.session, "scout", "--using", "fake-claude"])
        .output()
        .expect("the actual reseat command");
    let why = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        fs::read_to_string(rig.scratch.join("d54-trace")).expect("seed reaches the paste boundary"),
        "loaded, then warming drawn before paste\n"
    );
    assert!(
        rig.tool_pid(&pane, "claude").is_some(),
        "Claude is in the reseated pane"
    );
    let wire = fs::read(receipt.with_extension("wire")).expect("successor's input log");
    assert!(wire.is_empty(), "no seed paste or Enter: {wire:?}, {why}");
    assert_eq!(output.status.code(), Some(1), "{why}");
    assert!(why.contains("seed undelivered"), "{why}");
    assert!(rig.dir.join("seed.scout.md").is_file(), "seed is kept");
}

#[test]
fn a_warming_frame_after_paste_prevents_the_first_enter() {
    let rig = orchestrator("voice-after-paste", "claude", IDLE);
    rig.arm_frame("onpaste", &with_footer(WARMUP));
    let (code, why) = overview(&rig, "overview marker", &[]);
    rig.frame_barrier();
    let wire = rig.wire();
    assert!(
        wire.ends_with(b"\x1b[201~"),
        "the paste reached the pane, nothing followed: {wire:?}"
    );
    assert_eq!(
        rig.transitions(),
        "onpaste\n",
        "fixture switched after the real paste"
    );
    assert_eq!(
        rig.enter_count(),
        0,
        "the first Enter requires a fresh read: {code:?}: {why}"
    );
}

#[test]
fn a_warming_frame_after_the_first_enter_prevents_a_retry_enter() {
    let rig = orchestrator("voice-after-enter", "claude", IDLE);
    let staged = IDLE.replace("❯\u{a0}", "❯\u{a0}overview marker");
    rig.arm_frame("onpaste", &staged);
    let warming = with_footer(WARMUP).replace("❯\u{a0}", "❯\u{a0}overview marker");
    rig.arm_frame("onenter", &warming);
    let (code, why) = overview(&rig, "overview marker", &[]);
    rig.frame_barrier();
    assert!(
        rig.transitions().contains("onenter\n"),
        "the first Enter really started warming"
    );
    assert_eq!(
        rig.enter_count(),
        1,
        "no retry Enter while warming: {code:?}: {why}"
    );
}

const MEASURED_HOLD: &str =
    include_str!("../../../fixtures/harness-state/claude-voice-holding-2.1.332-120x41.txt");
const MEASURED_HOLD_STYLED: &str =
    include_str!("../../../fixtures/harness-state/claude-voice-holding-2.1.332-120x41.esc");
const MEASURED_SPEECH: &str =
    include_str!("../../../fixtures/harness-state/claude-voice-speaking-2.1.332-120x41.txt");

#[test]
fn measured_hold_is_the_right_aligned_label_and_waveform_at_the_current_box() {
    use ae::harness_state::{Dictation, HarnessState, classify, dictation};
    use ae::tool::ToolKind;

    let label = MEASURED_HOLD.lines().next().expect("the measured label");
    let wave = "❯\u{a0}▁";
    assert!(label.starts_with(' ') && label.ends_with("listening…"));
    assert!(MEASURED_HOLD.contains(wave));

    let no_wave = MEASURED_HOLD.replacen(wave, "❯\u{a0}", 1);
    let no_label = MEASURED_HOLD.replacen(label, "", 1);
    let left_aligned = MEASURED_HOLD.replacen(label, label.trim_start(), 1);
    let separated = MEASURED_HOLD.replacen(label, &format!("{label}\nOlder output"), 1);
    let blank_gap = MEASURED_HOLD.replacen(label, &format!("{label}\n"), 1);
    let quoted = format!("{label}\nOlder output\n{IDLE}");
    let draft = no_label.replacen(wave, "❯\u{a0}A draft quotes listening…", 1);
    for frame in [
        &no_label,
        &left_aligned,
        &separated,
        &blank_gap,
        &quoted,
        &draft,
    ] {
        assert!(
            !dictation(frame, ToolKind::Claude).active(),
            "a draft or old output cannot supply the active label: {frame:?}"
        );
    }
    let reduced_motion = dictation(&no_wave, ToolKind::Claude);
    assert_ne!(reduced_motion, Dictation::Holding);
    assert_eq!(classify(&quoted, ToolKind::Claude), HarnessState::Idle);
    assert_ne!(dictation(MEASURED_HOLD, ToolKind::Muse), Dictation::Holding);

    assert_eq!(
        dictation(MEASURED_HOLD, ToolKind::Claude),
        Dictation::Holding
    );
    assert_eq!(
        classify(MEASURED_HOLD, ToolKind::Claude),
        HarnessState::Busy
    );
    assert!(MEASURED_SPEECH.contains("❯\u{a0}▇"));
    assert_eq!(
        dictation(MEASURED_SPEECH, ToolKind::Claude),
        Dictation::Holding
    );

    // SYNTHETIC, from the vendor renderer's audio-level alphabet in 2.1.332.
    // The real frame above supplies the position, shape and lowest bar.
    for glyph in '▂'..='█' {
        let spoken = MEASURED_HOLD.replacen('▁', &glyph.to_string(), 1);
        assert_eq!(dictation(&spoken, ToolKind::Claude), Dictation::Holding);
    }
}

#[test]
fn measured_hold_label_alone_blocks_delivery_without_proving_a_waveform() {
    use ae::harness_state::{Dictation, HarnessState, classify, dictation};
    use ae::tool::ToolKind;

    // SYNTHETIC: 2.1.332 disables the waveform under prefersReducedMotion,
    // and its recording-label renderer still emits listening… independently.
    let reduced_motion = MEASURED_HOLD.replacen('▁', "", 1);
    let state = dictation(&reduced_motion, ToolKind::Claude);
    assert_ne!(state, Dictation::Holding, "no waveform proof for a key");
    assert!(state.active(), "label alone must stop delivery: {state:?}");
    assert_eq!(
        classify(&reduced_motion, ToolKind::Claude),
        HarnessState::Busy
    );
}

#[test]
fn measured_hold_label_blocks_delivery_when_the_composer_is_occupied() {
    use ae::harness_state::{Dictation, HarnessState, classify, dictation};
    use ae::tool::ToolKind;

    // SYNTHETIC: exercising the label independently of draft contents. The
    // waveform position with an existing draft has not been measured.
    for input in ["A draft", "A draft ▁"] {
        let draft = MEASURED_HOLD.replacen('▁', input, 1);
        let state = dictation(&draft, ToolKind::Claude);
        assert_ne!(state, Dictation::Holding, "no waveform proof for a key");
        assert!(state.active(), "label above occupied composer: {state:?}");
        assert_eq!(classify(&draft, ToolKind::Claude), HarnessState::Busy);
    }
}

#[test]
fn measured_hold_with_the_label_row_clipped_cannot_prove_the_pane_clear() {
    use ae::harness_state::{Dictation, dictation};
    use ae::tool::ToolKind;

    let (_, clipped) = MEASURED_HOLD.split_once('\n').expect("the captured label");
    let state = dictation(clipped, ToolKind::Claude);
    assert_ne!(state, Dictation::Holding, "no label proof for a key");
    assert!(state.active() || state == Dictation::Unread, "{state:?}");
}

#[test]
fn measured_hold_after_buffer_load_refuses_overview_before_paste() {
    assert_measured_hold_pre_paste_refusal("voice-hold-paste", MEASURED_HOLD_STYLED, true);
}

#[test]
fn measured_hold_label_alone_after_buffer_load_refuses_overview_before_paste() {
    // SYNTHETIC reduced-motion variant of the styled owner frame.
    let reduced_motion = MEASURED_HOLD_STYLED.replacen('▁', "", 1);
    assert_measured_hold_pre_paste_refusal("voice-label-paste", &reduced_motion, false);
}

fn assert_measured_hold_pre_paste_refusal(tag: &str, frame: &str, has_wave: bool) {
    let idle = include_str!("../../../fixtures/harness-state/claude-voice-idle-2.1.332-120x40.txt");
    let rig = orchestrator(tag, "claude", idle);
    checked(
        &rig,
        &[
            "resize-window",
            "-t",
            "=orchestrator:0",
            "-x",
            "120",
            "-y",
            "41",
        ],
    );
    let mut setup = before_paste_env(&rig.scratch, &rig.sock);
    fs::write(rig.scratch.join("next-voice-frame"), frame).expect("the next voice frame");
    setup.push(("D54_LOCK", rig.send_lock_path().display().to_string()));
    let env: Vec<_> = setup
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let (code, why) = overview(&rig, "owner capture gate marker", &env);
    let trace = fs::read_to_string(rig.scratch.join("d54-trace"))
        .expect("the admitted overview reached the staged-buffer boundary");
    assert!(trace.starts_with("loaded under lock"), "{trace}");
    let recorded = super::pane_frame(&rig, &rig.pane);
    assert!(recorded.contains("listening…"));
    assert_eq!(recorded.contains('▁'), has_wave);
    assert!(rig.wire().is_empty(), "no input: {tag}: {code:?}: {why}");
    assert_ne!(code, Some(0), "the measured hold refuses overview");
    assert!(why.contains("Claude dictation"), "{why}");
}

const MEASURED_TRANSCRIBING: &str =
    include_str!("../../../fixtures/harness-state/claude-voice-transcribing-2.1.332-120x41.txt");
const MEASURED_TRANSCRIBING_STYLED: &str =
    include_str!("../../../fixtures/harness-state/claude-voice-transcribing-2.1.332-120x41.esc");
const MEASURED_TAP: &str =
    include_str!("../../../fixtures/harness-state/claude-voice-tap-2.1.332-120x50.txt");
const MEASURED_TAP_STYLED: &str =
    include_str!("../../../fixtures/harness-state/claude-voice-tap-2.1.332-120x50.esc");
const MEASURED_INTERIM: &str =
    include_str!("../../../fixtures/harness-state/claude-voice-interim-2.1.332-120x41.txt");
const MEASURED_INTERIM_STYLED: &str =
    include_str!("../../../fixtures/harness-state/claude-voice-interim-2.1.332-120x41.esc");

#[test]
fn measured_interim_text_and_its_separate_waveform_read_holding() {
    use ae::harness_state::{Dictation, HarnessState, classify, dictation};
    use ae::tool::ToolKind;

    assert!(MEASURED_INTERIM.contains("❯\u{a0}Term.▅"));
    assert!(MEASURED_INTERIM_STYLED.contains("❯\u{a0}\x1b[38;5;246mTerm.\x1b[38;5;110m▅"));
    let plain = dictation(MEASURED_INTERIM, ToolKind::Claude);
    assert!(plain.active(), "the upper label still blocks delivery");
    assert_ne!(plain, Dictation::Holding, "interim styling is required");

    assert_eq!(
        dictation(MEASURED_INTERIM_STYLED, ToolKind::Claude),
        Dictation::Holding
    );
    assert_eq!(
        classify(MEASURED_INTERIM_STYLED, ToolKind::Claude),
        HarnessState::Busy
    );
    // BINARY-DERIVED. The waveform changes colour as it animates and
    // returns to gray (indexed 244 in the silent capture) during a pause.
    for bar in ["\x1b[38;5;146m▇", "\x1b[38;5;244m▁"] {
        let other_color = MEASURED_INTERIM_STYLED.replace("\x1b[38;5;110m▅", bar);
        assert_eq!(
            dictation(&other_color, ToolKind::Claude),
            Dictation::Holding
        );
    }
}

#[test]
fn wrapped_interim_reads_holding_when_the_waveform_follows_the_final_character() {
    use ae::harness_state::{Dictation, dictation};
    use ae::tool::ToolKind;

    // BINARY-DERIVED. The vendor overlays interim text in the composer.
    // Its waveform sits at the cursor, after the interim range.
    let first = "word ".repeat(23);
    for prefix in ["  ", "  \x1b[38;5;246m"] {
        let wrapped = MEASURED_INTERIM_STYLED.replacen(
            "Term.",
            &format!("{}\n{prefix}Term.", first.trim_end()),
            1,
        );
        assert_ne!(wrapped, MEASURED_INTERIM_STYLED);
        assert_eq!(dictation(&wrapped, ToolKind::Claude), Dictation::Holding);
        let wave_before_end = wrapped.replace(
            "Term.\x1b[38;5;110m▅\x1b[39m",
            "\x1b[38;5;110m▅\x1b[38;5;246mTerm.\x1b[39m",
        );
        assert_ne!(wave_before_end, wrapped);
        assert_ne!(
            dictation(&wave_before_end, ToolKind::Claude),
            Dictation::Holding
        );
    }
    let wrong_first_row = MEASURED_INTERIM_STYLED.replacen(
        "\x1b[38;5;246mTerm.",
        &format!("{}\n  \x1b[38;5;246mTerm.", first.trim_end()),
        1,
    );
    assert_ne!(wrong_first_row, MEASURED_INTERIM_STYLED);
    assert_ne!(
        dictation(&wrong_first_row, ToolKind::Claude),
        Dictation::Holding,
        "a dim continuation cannot lend its foreground to a default first row"
    );
}

#[test]
fn a_finished_insert_or_a_block_in_a_draft_never_proves_holding() {
    use ae::harness_state::{Dictation, dictation};
    use ae::tool::ToolKind;

    let (label, box_rows) = MEASURED_INTERIM_STYLED
        .split_once('\n')
        .expect("the measured upper label");
    let content = "\x1b[38;5;246mTerm.\x1b[38;5;110m▅\x1b[39m";
    assert!(box_rows.contains(content));
    let finished = box_rows.replacen(content, "Term.", 1);
    assert!(!dictation(&finished, ToolKind::Claude).active());
    let draft = box_rows.replacen(content, "▅", 1);
    assert!(!dictation(&draft, ToolKind::Claude).active());

    for replacement in [
        "▅",
        "Term.",
        "Term.▅",
        "Term.\x1b[38;5;110m▅\x1b[39m",
        "\x1b[38;5;250mTerm.\x1b[38;5;110m▅\x1b[39m",
        "\x1b[38;5;246mTerm.▅\x1b[39m",
        "\x1b[38;5;246mTerm.\x1b[39m",
        "\x1b[38;5;246mOne\x1b[39m \x1b[38;5;246mtwo\x1b[38;5;110m▅\x1b[39m",
        "\x1b[38;5;246mTerm. \x1b[38;5;110m▅\x1b[39m",
        "\x1b[38;5;246mTerm.\x1b[38;5;110m▅\x1b[39mtext",
    ] {
        let frame = format!("{label}\n{}", box_rows.replacen(content, replacement, 1));
        let state = dictation(&frame, ToolKind::Claude);
        assert!(state.active(), "the attached label blocks delivery");
        assert_ne!(state, Dictation::Holding, "{replacement:?}");
    }
    let displaced = format!("{label}\nOlder output\n{box_rows}");
    assert_ne!(dictation(&displaced, ToolKind::Claude), Dictation::Holding);
}

#[test]
fn measured_transcribing_reads_busy_before_the_insert() {
    assert_measured_upper_voice_state(
        MEASURED_TRANSCRIBING,
        "Transcribing…",
        ae::harness_state::Dictation::Transcribing,
    );
}

#[test]
fn measured_tap_reads_busy_from_the_record_symbol_and_send_hint() {
    assert_measured_upper_voice_state(
        MEASURED_TAP,
        "⏺ REC · tap to send",
        ae::harness_state::Dictation::Tapping,
    );
}

fn assert_measured_upper_voice_state(frame: &str, name: &str, want: ae::harness_state::Dictation) {
    use ae::harness_state::dictation;
    use ae::tool::ToolKind;

    let (label, box_rows) = frame.split_once('\n').expect("label above the composer");
    assert!(label.starts_with(' ') && label.trim() == name);
    let scrollback = format!("{label}\nOlder output\n{box_rows}");
    let left_aligned = format!("{name}\n{box_rows}");
    let draft = format!(
        "Older output\n{}",
        box_rows.replace("❯\u{a0}", &format!("❯\u{a0}{name} "))
    );
    for negative in [&scrollback, &left_aligned, &draft] {
        assert!(!dictation(negative, ToolKind::Claude).active());
    }
    assert!(!dictation(frame, ToolKind::Muse).active());
    assert_dictation_busy(frame, want);
}

#[test]
fn measured_transcribing_blocks_a_pane_with_a_draft() {
    // SYNTHETIC: the attached label must still hold delivery during insert.
    let draft = MEASURED_TRANSCRIBING.replacen("❯\u{a0}\n", "❯\u{a0}A draft\n", 1);
    assert_ne!(draft, MEASURED_TRANSCRIBING);
    assert_dictation_busy(&draft, ae::harness_state::Dictation::Transcribing);
}

#[test]
fn measured_tap_blocks_a_pane_with_a_draft() {
    // SYNTHETIC: the complete tap label is evidence independent of the box.
    let draft = MEASURED_TAP.replacen("❯\u{a0}▁\n", "❯\u{a0}A draft\n", 1);
    assert_ne!(draft, MEASURED_TAP);
    assert_dictation_busy(&draft, ae::harness_state::Dictation::Tapping);
}

fn assert_dictation_busy(frame: &str, want: ae::harness_state::Dictation) {
    use ae::harness_state::{HarnessState, classify, dictation};
    use ae::tool::ToolKind;

    let state = dictation(frame, ToolKind::Claude);
    assert_eq!(state, want, "measured upper label: {frame:?}");
    assert!(state.active(), "must block delivery: {frame:?}");
    assert_eq!(classify(frame, ToolKind::Claude), HarnessState::Busy);
}

#[test]
fn measured_transcribing_after_load_blocks_overview_paste() {
    assert_measured_upper_voice_pre_paste_refusal(
        "transcribing-paste",
        MEASURED_TRANSCRIBING_STYLED,
        "41",
        "Transcribing…",
    );
}

#[test]
fn measured_tap_after_load_blocks_overview_paste() {
    assert_measured_upper_voice_pre_paste_refusal(
        "tap-rec-paste",
        MEASURED_TAP_STYLED,
        "50",
        "⏺ REC · tap to send",
    );
}

fn assert_measured_upper_voice_pre_paste_refusal(
    tag: &str,
    frame: &str,
    height: &str,
    label: &str,
) {
    let idle = include_str!("../../../fixtures/harness-state/claude-voice-idle-2.1.332-120x40.txt");
    let rig = orchestrator(tag, "claude", idle);
    checked(
        &rig,
        &[
            "resize-window",
            "-t",
            "=orchestrator:0",
            "-x",
            "120",
            "-y",
            height,
        ],
    );
    let mut setup = before_paste_env(&rig.scratch, &rig.sock);
    fs::write(rig.scratch.join("next-voice-frame"), frame).expect("the next voice frame");
    setup.push(("D54_LOCK", rig.send_lock_path().display().to_string()));
    let env: Vec<_> = setup
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let (code, why) = overview(&rig, "owner capture gate marker", &env);
    let trace = fs::read_to_string(rig.scratch.join("d54-trace"))
        .expect("the admitted overview reached the staged-buffer boundary");
    assert!(trace.starts_with("loaded under lock"), "{trace}");
    assert!(super::pane_frame(&rig, &rig.pane).contains(label));
    rig.frame_barrier();
    assert!(rig.wire().is_empty(), "no input: {tag}: {code:?}: {why}");
    assert_ne!(code, Some(0), "measured dictation refuses overview");
    assert!(why.contains("Claude dictation"), "{why}");
}
