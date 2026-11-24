//! The public self-reseat request and the real watchdog that consumes it.

use super::*;

const REQUEST: &str = "reseat-request";
const DESTINATION: &str = "fake-claude-b";
const REFUSED: &str = "self-reseat-refused";

fn fixture(tag: &str, orchestrator: bool) -> SeatRig {
    let rig = Rig::new(tag);
    let path = rig.scratch.join("config");
    let config = fs::read_to_string(&path).expect("fixture config");
    let mut profiles = String::new();
    for line in config.lines() {
        if let Some(command) = line.strip_prefix("fake-claude-a = ") {
            let command = command.trim_matches('"');
            writeln!(profiles, "fake-claude-a = \"{command} --model opus\"")
                .expect("source profile");
        } else if let Some(command) = line.strip_prefix("fake-claude-b = ") {
            let command = command.trim_matches('"');
            writeln!(profiles, "fake-claude-b = \"{command} --model sonnet\"")
                .expect("destination profile");
            writeln!(profiles, "fake-claude-alt = \"{command} --model haiku\"")
                .expect("alternate profile");
        } else {
            writeln!(profiles, "{line}").expect("fixture config line");
        }
    }
    fs::write(&path, profiles).expect("explicitly different model pins");
    let seat = SeatRig::with_rig(rig, orchestrator, DESTINATION);
    // An explicit request is independent of the automatic quota policy and map.
    configure_from(&seat.rig, "off", "fake-claude-a", "fake-claude");
    seat
}

fn command(seat: &SeatRig, tail: &[&str]) -> (Option<i32>, String, String) {
    let rig = &seat.rig;
    let mut words = vec!["reseat", &rig.session, seat.name];
    words.extend_from_slice(tail);
    rig.run_top(&rig.main_pane, &words)
}

fn queue(seat: &SeatRig, profile: &str) {
    let rig = &seat.rig;
    let before = (
        rig.capture(&rig.main_pane),
        rig.meta_row("launch_id.main"),
        rig.launched(),
    );
    let receipt = command(seat, &["--using", profile]);
    assert_eq!(receipt.0, Some(0), "{receipt:?}");
    assert_eq!(
        receipt.1.trim(),
        format!("reseat queued: {profile} after this turn")
    );
    assert!(
        fs::metadata(rig.dir.join(REQUEST)).is_ok_and(|meta| meta.is_file() && meta.len() > 0),
        "success must publish one durable request"
    );
    assert_eq!(rig.capture(&rig.main_pane), before.0);
    assert_eq!(rig.meta_row("launch_id.main"), before.1);
    assert_eq!(rig.launched(), before.2);
    still_source(seat);
}

fn still_source(seat: &SeatRig) {
    let rig = &seat.rig;
    assert_eq!(rig.tool_pid(&rig.main_pane, "claude"), Some(seat.pid));
    assert_eq!(rig.meta_row("profile.main"), "fake-claude-a");
    assert_eq!(rig.meta_row("harness_session.main"), CARRIED);
    assert!(seat.records("reseat").is_empty(), "{}", rig.events());
    assert!(!seat.other_store.exists(), "no copy before a move");
}

fn watch(seat: &SeatRig, done: impl Fn() -> bool) {
    let rig = &seat.rig;
    let (ok, pane) = rig.tmux(&[
        "new-window",
        "-d",
        "-t",
        &rig.session,
        "-P",
        "-F",
        "#{pane_id}",
        "sleep",
        "120",
    ]);
    assert!(ok, "{pane}");
    let pane = pane.trim();
    assert!(
        rig.tmux(&["set-option", "-p", "-t", pane, "@ae_agent", "_watchdog"])
            .0
    );
    assert!(
        watch_until_from(rig, Some(pane), done),
        "watchdog did not produce the expected result\n{}\n{}",
        rig.events(),
        daemon_err(rig)
    );
}

/// Observe advances in the live daemon heartbeat, including a completed cycle
/// between two writes. A test for absence must first prove the daemon ran.
fn cycles(rig: &Rig, count: usize) -> impl Fn() -> bool {
    let path = rig.dir.join(".watchdog-beat");
    let last = Cell::new(fs::metadata(&path).and_then(|meta| meta.modified()).ok());
    let seen = Cell::new(0);
    move || {
        let time = fs::metadata(&path).and_then(|meta| meta.modified()).ok();
        if time.is_some() && time != last.replace(time) {
            seen.set(seen.get() + 1);
        }
        seen.get() >= count
    }
}

#[test]
fn orchself_replaces_while_busy_and_carries_the_selected_model_after_the_turn() {
    let seat = fixture("osmove", true);
    let rig = &seat.rig;
    assert!(
        rig.launched().contains("--model opus"),
        "{}",
        rig.launched()
    );
    rig.mark_busy(&rig.main_pane);
    let before = rig.meta_row("launch_id.main");
    queue(&seat, "fake-claude-alt");
    queue(&seat, DESTINATION);

    let elapsed = cycles(rig, 2);
    let released = Cell::new(false);
    watch(&seat, || {
        if !released.get() && elapsed() {
            still_source(&seat);
            assert!(
                rig.dir.join(REQUEST).is_file(),
                "busy preserves the request"
            );
            assert!(seat.records(REFUSED).is_empty(), "busy is not a refusal");
            fs::remove_file(rig.scratch.join("__BUSY__")).expect("the turn ends");
            released.set(true);
        }
        rig.meta_row("profile.main") == DESTINATION
            && rig
                .tool_pid(&rig.main_pane, "claude")
                .is_some_and(|pid| pid != seat.pid)
            && rig.launched().lines().any(|line| {
                line.contains("--model sonnet") && line.contains(&format!("--resume {CARRIED}"))
            })
            && seat.records("self-reseat-done").len() == 1
    });
    assert!(released.get(), "a move must wait until the turn ends");
    assert_eq!(rig.meta_row("harness_session.main"), CARRIED);
    assert_ne!(rig.meta_row("launch_id.main"), before);
    assert!(
        !rig.launched().contains("--model haiku"),
        "the replaced request never executes"
    );
    assert_eq!(
        fs::read_to_string(&seat.other_store).expect("carried transcript"),
        FACT
    );
}

#[test]
fn orchself_refuses_cross_adapter_up_front() {
    let seat = fixture("oscross", true);
    let receipt = command(&seat, &["--using", "fake-opencode"]);
    assert_ne!(receipt.0, Some(0), "{receipt:?}");
    let reason = receipt.2.to_ascii_lowercase();
    assert!(
        reason.contains("conversation") || reason.contains("adapter") || reason.contains("binary"),
        "the refusal names the carry boundary: {receipt:?}"
    );
    assert!(!seat.rig.dir.join(REQUEST).exists());
    still_source(&seat);
}

#[test]
fn orchself_late_missing_transcript_refuses_once_and_reaches_the_seat() {
    let seat = fixture("oslate", true);
    let rig = &seat.rig;
    queue(&seat, DESTINATION);
    let launched = rig.launched();
    fs::remove_file(&seat.transcript).expect("the source transcript disappears after queueing");
    let after_notice = std::cell::OnceCell::new();
    watch(&seat, || {
        let refusals = seat.records(REFUSED);
        let Some(refusal) = refusals.first() else {
            return false;
        };
        let reason = refusal
            .summary
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        assert!(
            reason.contains("transcript") || reason.contains("conversation"),
            "the journal names the carry failure: {refusal:?}"
        );
        let delivered = rig.received().to_ascii_lowercase();
        let heard = delivered.contains("reseat")
            && delivered.contains("refus")
            && (delivered.contains("transcript") || delivered.contains("conversation"));
        heard && after_notice.get_or_init(|| cycles(rig, 2))()
    });
    assert_eq!(seat.records(REFUSED).len(), 1, "{}", rig.events());
    assert_eq!(rig.launched(), launched, "no fresh start");
    still_source(&seat);
}

#[test]
fn orchself_cancelled_request_does_not_move_an_idle_seat() {
    let seat = fixture("oscancel", true);
    queue(&seat, DESTINATION);
    let receipt = command(&seat, &["--cancel"]);
    assert_eq!(receipt.0, Some(0), "{receipt:?}");
    watch(&seat, cycles(&seat.rig, 3));
    assert!(
        seat.records("self-reseat-attempt").is_empty(),
        "{}",
        seat.rig.events()
    );
    still_source(&seat);
}

#[test]
fn orchself_damaged_request_does_not_move_an_idle_seat() {
    let seat = fixture("osdamage", true);
    queue(&seat, DESTINATION);
    let path = seat.rig.dir.join(REQUEST);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("queued file");
    file.write_all(b"\x00unexpected\n")
        .expect("damage the genuine request");
    drop(file);
    watch(&seat, cycles(&seat.rig, 3));
    let refusals = seat.records(REFUSED);
    assert_eq!(refusals.len(), 1, "{}", seat.rig.events());
    let reason = refusals[0]
        .summary
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        reason.contains("damag") || reason.contains("malform") || reason.contains("invalid"),
        "{reason}"
    );
    assert!(
        seat.records("self-reseat-attempt").is_empty(),
        "{}",
        seat.rig.events()
    );
    still_source(&seat);
}

#[test]
fn orchself_ordinary_seat_cannot_queue_its_own_reseat() {
    let seat = fixture("osordinary", false);
    let receipt = command(&seat, &["--using", DESTINATION]);
    assert_ne!(receipt.0, Some(0), "{receipt:?}");
    assert!(receipt.2.contains("cannot reseat itself"), "{receipt:?}");
    assert!(!seat.rig.dir.join(REQUEST).exists());
    still_source(&seat);
}

#[test]
fn orchself_keeps_own_session_lifecycle_refusals() {
    let seat = fixture("oslifecycle", true);
    let rig = &seat.rig;
    for words in [
        vec!["stop", &rig.session],
        vec!["end", &rig.session, "--keep-history"],
        vec!["rename", &rig.session, "unwanted-name"],
    ] {
        let receipt = rig.run_top(&rig.main_pane, &words);
        assert_ne!(receipt.0, Some(0), "{words:?}: {receipt:?}");
        assert!(
            receipt.2.contains("own session") && receipt.2.contains("REFUSED"),
            "{words:?}: {receipt:?}"
        );
    }
    still_source(&seat);
    assert!(rig.dir.is_dir());
    assert!(
        rig.tmux(&["has-session", "-t", &format!("={}", rig.session)])
            .0
    );
}

#[test]
fn orchself_checks_argv_roster_profile_and_pair_before_writing() {
    let seat = fixture("ospreflight", true);
    let rig = &seat.rig;
    for (tail, reason) in [
        (
            vec!["--using", "fake-claude-b@"],
            "not <profile> or <profile>@<client>",
        ),
        (vec!["--using", "not-a-profile"], "not-a-profile"),
        (vec!["--using", "fake-claude-a"], "already runs"),
        (
            vec!["--using", "fake-claude-b@not-approved"],
            "not an approved pair",
        ),
    ] {
        let receipt = command(&seat, &tail);
        assert_ne!(receipt.0, Some(0), "{tail:?}: {receipt:?}");
        assert!(receipt.2.contains(reason), "{tail:?}: {receipt:?}");
        assert!(!rig.dir.join(REQUEST).exists(), "{tail:?}");
    }
    // This change grants the orchestrator authority over its own seat only.
    let other = rig.seat("worker.1", "w1", "opencode");
    let pid = rig
        .tool_pid(&other, "opencode")
        .expect("the other seat runs");
    let receipt = rig.run_top(
        &rig.main_pane,
        &["reseat", &rig.session, "w1", "--using", DESTINATION],
    );
    assert_ne!(receipt.0, Some(0), "{receipt:?}");
    assert!(!rig.dir.join(REQUEST).exists());
    assert_eq!(rig.tool_pid(&other, "opencode"), Some(pid));
    still_source(&seat);
}

#[test]
fn orchself_charter_explains_the_queued_self_reseat() {
    assert!(
        CHARTER.split("\n\n").any(|paragraph| {
            paragraph.contains("ae reseat ")
                && paragraph.contains("--using")
                && paragraph.contains("--cancel")
                && paragraph.contains("queu")
                && (paragraph.contains("turn") || paragraph.contains("idle"))
        }),
        "the shipped charter should explain both request forms and the deferred move"
    );
    assert!(
        !CHARTER.contains("retire, reseat, relaunch"),
        "the old blanket refusal conflicts with the self-reseat instruction"
    );
}
