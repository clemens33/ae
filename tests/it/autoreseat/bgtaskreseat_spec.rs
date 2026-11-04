//! #304 B acceptance: requirements from the issue, brief and lead D1–D6.
//! Frozen by the independent navigator before implementation. Only external
//! harnesses are replaced; the real stop leg, quota reader and journal run.
use super::*;

use ae::autoreseat::{CauseClass, Ending, Fate, Move, notice};

const WATCHING: &str = include_str!("../../fixtures/harness-state/muse-limit-watching.txt");
const TITLE: &str = "background evidence watch";

// Measured wording/wrap: .local/bgtaskreseat/incident-pane-tail.txt. Combining
// the vendor error with WATCHING and varying its reset are DERIVED fixtures.
fn iso_frame(reset: i64, wrapped: bool) -> String {
    let reset = Timestamp::from_epoch(reset);
    let gap = if wrapped { "\n  " } else { " " };
    format!(
        "◆ model failed: API error 429 [request_id=00000000-0000-0000-0000-000000000000]:{gap}Subscription quota exhausted. Your usage window resets at{gap}{reset}. (rate_limit_error)\n\n{WATCHING}"
    )
}

fn setup(tag: &str, frame: &str, age: i64) -> (Rig, String, String, i64) {
    let rig = Rig::new(tag);
    assert!(
        std::fs::copy(
            rig.scratch.join("tools/claude"),
            rig.scratch.join("tools/muse")
        )
        .is_ok()
    );
    let home = rig.scratch.join("home");
    assert!(std::fs::create_dir_all(home.join(".config/muse")).is_ok());
    assert!(
        rig.tmux(&["set-environment", "HOME", &home.display().to_string()])
            .0
    );
    let config = rig.scratch.join("config");
    let text =
        std::fs::read_to_string(&config).unwrap_or_else(|why| panic!("fixture config: {why}"));
    let profile = format!(
        "fake-muse = \"{} {}\"\n[workspace]\nmuse_quota = on\n",
        rig.scratch.join("tools/muse").display(),
        rig.scratch.join("muse.pl").display()
    );
    assert!(std::fs::write(&config, text.replace("[workspace]", &profile)).is_ok());
    configure_from(&rig, ON_NOW, "fake-muse", "fake-opencode");
    let pane = limitstop_playback(&rig, "muse", frame);
    let now = Timestamp::now().epoch();
    let key = Timestamp::from_epoch(now - age).to_string();
    journal_at(&rig, &key, "limit", None);
    journal_at(
        &rig,
        &Timestamp::from_epoch(now).to_string(),
        ATTEMPT_ACTION,
        Some(&key),
    );
    (rig, pane, key, now)
}

/// Cache bytes follow Muse's existing v1 persisted contract, independently of
/// any new B helper. The vendor observation age and episode age are separate.
fn held(rig: &Rig, now: i64, age: i64, reset: Option<i64>, used: u8, foreign: bool) {
    let home = rig.scratch.join(if foreign {
        "foreign/.config/muse"
    } else {
        "home/.config/muse"
    });
    assert!(std::fs::create_dir_all(&home).is_ok());
    let home = std::fs::canonicalize(home)
        .unwrap_or_else(|why| panic!("canonical fixture account: {why}"));
    let reset = reset.map_or_else(String::new, |at| format!(",\"resets_at_ms\":{}", at * 1000));
    let cache = format!(
        "{{\"v\":1,\"home\":\"{}\",\"attempt_at\":{now},\"outcome\":\"done\",\"allowance\":{{\"weekly\":{{\"used_percent\":{used}{reset}}},\"as_of_ms\":{}}}}}",
        home.display(),
        (now - age) * 1000
    );
    assert!(std::fs::create_dir_all(rig.scratch.join("quota")).is_ok());
    assert!(std::fs::write(rig.scratch.join("quota/muse.json"), cache).is_ok());
}

fn preserved(rig: &Rig, pane: &str, key: &str) -> Event {
    let pid = rig
        .tool_pid(pane, "muse")
        .unwrap_or_else(|| panic!("predecessor alive"));
    let (code, out, err) = leg(rig, "spawned.0", key);
    assert_eq!(code, Some(1), "out={out} err={err}");
    assert_eq!(
        rig.tool_pid(pane, "muse"),
        Some(pid),
        "same process survives"
    );
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-muse");
    assert!(with_action(&records(rig), "reseat").is_empty());
    assert!(!rig.dir.join("seed.scout.md").exists());
    assert_eq!(booked(rig, DONE_ACTION), 0);
    one_outcome(rig, REFUSED_ACTION)
}

fn moved(rig: &Rig, pane: &str, key: &str) {
    link_send(rig);
    let (code, out, err) = leg(rig, "spawned.0", key);
    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(
        rig.tool_pid(pane, "muse").is_none(),
        "background predecessor ended"
    );
    assert!(rig.tool_pid(pane, "opencode").is_some(), "replacement runs");
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    let done = one_outcome(rig, DONE_ACTION);
    assert_eq!(done.reference.as_deref(), Some("fake-muse"));
    assert!(
        done.summary
            .as_deref()
            .is_some_and(|s| s.starts_with("to fake-opencode,")),
        "{done:?}"
    );
    let chat = said(rig);
    assert_eq!(chat.len(), 1, "one human notice: {chat:?}");
    let told = chat[0].summary.as_deref().unwrap_or_default();
    assert!(
        told.contains(TITLE) && told.contains("ended"),
        "ended task named: {told}"
    );
    assert_eq!(told.matches(" Next: ").count(), 1, "{told}");
}

#[test]
fn bgtask_cause_names_the_read_background_task_without_reset_proof() {
    let (rig, pane, key, _) = setup("bgcause", WATCHING, 1200);
    let event = preserved(&rig, &pane, &key);
    let summary = event.summary.as_deref().unwrap_or_default();
    assert!(summary.starts_with("background-task:"), "{summary}");
    assert!(summary.contains(TITLE), "{summary}");
}

#[test]
fn bgtask_long_full_limit_after_grace_moves_and_names_ended_task() {
    let frame = iso_frame(Timestamp::now().epoch() + 43_260, true);
    let (rig, pane, key, _) = setup("bglong", &frame, 1200);
    moved(&rig, &pane, &key);
}

#[test]
fn bgtask_unwrapped_absolute_reset_moves_without_a_quota_identity_join() {
    let frame = iso_frame(Timestamp::now().epoch() + 43_260, false);
    let (rig, pane, key, _) = setup("bgunwrapped", &frame, 1200);
    moved(&rig, &pane, &key);
}

#[test]
fn bgtask_stale_foreign_cache_does_not_subtract_valid_pane_reset_proof() {
    let frame = iso_frame(Timestamp::now().epoch() + 43_260, true);
    let (rig, pane, key, now) = setup("bgstalecache", &frame, 1200);
    held(&rig, now, 3000, Some(now + 60), 79, true);
    moved(&rig, &pane, &key);
}

#[test]
fn bgtask_status_is_not_a_watching_only_exception() {
    let frame = iso_frame(Timestamp::now().epoch() + 43_260, true).replace("watching", "running");
    let (rig, pane, key, _) = setup("bgrunning", &frame, 1200);
    moved(&rig, &pane, &key);
}

#[test]
fn bgtask_multiple_task_rows_with_different_statuses_are_still_background_only() {
    let frame = iso_frame(Timestamp::now().epoch() + 43_260, true).replace(
        "  muse-wonder-full",
        "└ ◆ second background task  done 1s\n  muse-wonder-full",
    );
    let (rig, pane, key, _) = setup("bgmultiple", &frame, 1200);
    moved(&rig, &pane, &key);
}

#[test]
fn bgtask_shorter_than_twelve_hours_preserves_process() {
    let frame = iso_frame(Timestamp::now().epoch() + 43_140, true);
    let (rig, pane, key, _) = setup("bgshort", &frame, 1200);
    preserved(&rig, &pane, &key);
}

#[test]
fn bgtask_before_ten_minute_grace_preserves_process() {
    let frame = iso_frame(Timestamp::now().epoch() + 43_260, true);
    let (rig, pane, key, _) = setup("bggrace", &frame, 60);
    preserved(&rig, &pane, &key);
}

#[test]
fn bgtask_unknown_reset_preserves_process() {
    let (rig, pane, key, now) = setup("bgnoreset", WATCHING, 1200);
    held(&rig, now, 0, None, 100, false);
    preserved(&rig, &pane, &key);
}

#[test]
fn bgtask_stale_observation_before_episode_correlation_bound_preserves_process() {
    let (rig, pane, key, now) = setup("bgold", WATCHING, 1200);
    held(&rig, now, 3000, Some(now + 43_260), 100, false);
    preserved(&rig, &pane, &key);
}

#[test]
fn bgtask_stale_healthy_observation_never_proves_exhaustion() {
    let (rig, pane, key, now) = setup("bghealthy", WATCHING, 1200);
    held(&rig, now, 1500, Some(now + 43_260), 79, false);
    preserved(&rig, &pane, &key);
}

#[test]
fn bgtask_foreign_account_observation_preserves_process() {
    let (rig, pane, key, now) = setup("bgforeign", WATCHING, 1200);
    held(&rig, now, 0, Some(now + 43_260), 100, true);
    preserved(&rig, &pane, &key);
}

#[test]
fn bgtask_future_observation_preserves_process() {
    let (rig, pane, key, now) = setup("bgfuture", WATCHING, 1200);
    held(&rig, now, -3600, Some(now + 43_260), 100, false);
    preserved(&rig, &pane, &key);
}

#[test]
fn bgtask_full_cache_without_absolute_pane_reset_preserves_process() {
    let (rig, pane, key, now) = setup("bgcacheonly", WATCHING, 1200);
    held(&rig, now, 0, Some(now + 43_260), 100, false);
    preserved(&rig, &pane, &key);
}

#[test]
fn bgtask_queued_user_echo_between_error_and_newest_limit_is_allowed() {
    let frame = iso_frame(Timestamp::now().epoch() + 43_260, true).replace(
        "\n\n◆ Usage limit reached",
        "\n\n❯ ⟦ae:msg from watchdog⟧\n  Queued message body\n\n◆ Usage limit reached",
    );
    let (rig, pane, key, _) = setup("bgecho", &frame, 1200);
    moved(&rig, &pane, &key);
}

#[test]
fn bgtask_old_error_above_a_complete_composer_and_footer_cannot_prove_current_incarnation() {
    let bottom = WATCHING
        .split_once("── Voice input")
        .unwrap_or_else(|| panic!("measured composer"))
        .1;
    let boundary = format!("── Voice input{bottom}\n");
    let frame = iso_frame(Timestamp::now().epoch() + 43_260, true).replace(
        "\n\n◆ Usage limit reached",
        &format!("\n\n{boundary}\n◆ Usage limit reached"),
    );
    let (rig, pane, key, _) = setup("bgincarnation", &frame, 1200);
    preserved(&rig, &pane, &key);
}

#[test]
fn bgtask_other_transcript_or_shell_between_error_and_limit_preserves_process() {
    let base = iso_frame(Timestamp::now().epoch() + 43_260, true);
    for (tag, row) in [
        ("bgpriorcmd", "◆ Ran command · new work"),
        ("bgpriorshell", "$ muse"),
    ] {
        let frame = base.replace(
            "\n\n◆ Usage limit reached",
            &format!("\n\n{row}\n\n◆ Usage limit reached"),
        );
        let (rig, pane, key, _) = setup(tag, &frame, 1200);
        preserved(&rig, &pane, &key);
    }
}

#[test]
fn bgtask_quoted_or_malformed_error_never_proves_a_reset() {
    let base = iso_frame(Timestamp::now().epoch() + 43_260, true);
    for (tag, frame) in [
        (
            "bgquoted",
            base.replacen("◆ model failed", "  ◆ model failed", 1),
        ),
        (
            "bgbaduuid",
            base.replace("00000000-0000-0000-0000-000000000000", "not-a-uuid"),
        ),
        (
            "bgwrongerror",
            base.replace("Subscription quota exhausted.", "Temporary server failure."),
        ),
    ] {
        let (rig, pane, key, _) = setup(tag, &frame, 1200);
        preserved(&rig, &pane, &key);
    }
}

#[test]
fn bgtask_absolute_reset_in_the_past_preserves_process() {
    let frame = iso_frame(Timestamp::now().epoch() - 60, true);
    let (rig, pane, key, _) = setup("bgexpired", &frame, 1200);
    preserved(&rig, &pane, &key);
}

#[test]
fn bgtask_missing_or_duplicate_model_footer_preserves_process() {
    let base = iso_frame(Timestamp::now().epoch() + 43_260, true);
    let footer = "  muse-wonder-full-1.0 · max · ~/project · YOLO\n";
    for (tag, frame) in [
        ("bgnofurniture", base.replace(footer, "")),
        (
            "bgdoublefurniture",
            base.replace(footer, &format!("{footer}{footer}")),
        ),
    ] {
        let (rig, pane, key, _) = setup(tag, &frame, 1200);
        preserved(&rig, &pane, &key);
    }
}

#[test]
fn bgtask_manual_reseat_without_override_still_preserves_background_work() {
    let frame = iso_frame(Timestamp::now().epoch() + 43_260, true);
    let (rig, pane, _, _) = setup("bgmanual", &frame, 1200);
    let pid = rig.tool_pid(&pane, "muse");
    let output = super::super::cli::ae()
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", &rig.main_pane)
        .env("AE_HOME", &rig.scratch)
        .env("HOME", rig.scratch.join("home"))
        .env_remove("AE_SENDER_OVERRIDE")
        .args(["reseat", &rig.session, "scout", "--using", "fake-opencode"])
        .output()
        .unwrap_or_else(|why| panic!("manual reseat fixture: {why}"));
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(rig.tool_pid(&pane, "muse"), pid);
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-muse");
    assert!(with_action(&records(&rig), "reseat").is_empty());
}

#[test]
fn bgtask_draft_even_with_long_full_limit_preserves_process() {
    let frame =
        iso_frame(Timestamp::now().epoch() + 43_260, true).replace("\n❯\n", "\n❯ human draft\n");
    let (rig, pane, key, _) = setup("bgdraft", &frame, 1200);
    preserved(&rig, &pane, &key);
}

#[test]
fn bgtask_another_footer_row_before_or_after_task_preserves_process() {
    let frame = iso_frame(Timestamp::now().epoch() + 43_260, true);
    for (tag, frame) in [
        (
            "bgstraybefore",
            frame.replace("main · ↓", "human prompt\nmain · ↓"),
        ),
        (
            "bgstrayafter",
            frame.replace("  muse-wonder", "human prompt\n  muse-wonder"),
        ),
    ] {
        let (rig, pane, key, _) = setup(tag, &frame, 1200);
        preserved(&rig, &pane, &key);
    }
}

#[test]
fn bgtask_new_transcript_turn_after_limit_preserves_process() {
    let frame = iso_frame(Timestamp::now().epoch() + 43_260, true)
        .replace("── Voice input", "◆ Running a newer turn\n── Voice input");
    let (rig, pane, key, _) = setup("bgnewturn", &frame, 1200);
    preserved(&rig, &pane, &key);
}

#[test]
fn bgtask_auto_reseat_off_preserves_process() {
    let frame = iso_frame(Timestamp::now().epoch() + 43_260, true);
    let (rig, pane, key, _) = setup("bgoff", &frame, 1200);
    configure_from(&rig, "off", "fake-muse", "fake-opencode");
    let pid = rig.tool_pid(&pane, "muse");
    let (code, _, _) = leg(&rig, "spawned.0", &key);
    assert_eq!(code, Some(1));
    assert_eq!(rig.tool_pid(&pane, "muse"), pid);
    assert_eq!(booked(&rig, DONE_ACTION), 0);
    assert!(with_action(&records(&rig), "reseat").is_empty());
}

#[test]
fn bgtask_headroom_episode_never_uses_the_long_limit_exception() {
    let frame = iso_frame(Timestamp::now().epoch() + 43_260, true);
    let (rig, pane, key, _) = setup("bgheadroom", &frame, 1200);
    let path = rig.dir.join("events.jsonl");
    let events = std::fs::read_to_string(&path).unwrap_or_else(|why| panic!("events: {why}"));
    let events = events.replace(
        "\"action\":\"limit\"",
        "\"action\":\"auto-reseat-headroom\",\"summary\":\"headroom 95% weekly_all 7d\"",
    );
    assert!(std::fs::write(path, events).is_ok());
    let pid = rig.tool_pid(&pane, "muse");
    let (code, _, _) = leg(&rig, "spawned.0", &key);
    assert_eq!(code, Some(1));
    assert_eq!(rig.tool_pid(&pane, "muse"), pid);
    assert_eq!(booked(&rig, DONE_ACTION), 0);
    assert!(with_action(&records(&rig), "reseat").is_empty());
}

#[test]
fn bgtask_refusal_notice_has_one_complete_next_command() {
    let command = "ae reseat aedev scout --using sol61x --stop-unknown";
    let why = format!(
        "unknown-frame: Error: frame not proven (background task: {TITLE}) — once its box is empty and the limit line is its newest row, run: {command} — ae cannot read the frame. Nothing was reseated."
    );
    let told = notice(
        "aedev",
        "scout",
        &Ending::Refused(CauseClass::UnknownFrame, &why),
    );
    assert_eq!(told.matches(command).count(), 1, "{told}");
    assert_eq!(told.matches("once its box is empty").count(), 1, "{told}");
    assert!(told.contains(" Next: ") && told.ends_with('.'), "{told}");
    assert!(told.chars().count() <= 400, "{told}");
}

#[test]
fn bgtask_all_notice_endings_cut_only_at_a_word_boundary() {
    let words = "boundaryword ".repeat(150);
    let why = format!("Error: {words}");
    let moved = Move {
        from: "old",
        to: "next",
        tool: ("muse", "codex"),
        model: ("a", "b"),
        fate: Fate::Seeded,
        nudge: None,
        critical: false,
        dirty: false,
        cause: &words,
    };
    for ending in [
        Ending::Held(CauseClass::Other, &why),
        Ending::Refused(CauseClass::UnknownFrame, &why),
        Ending::Failed(CauseClass::Other, &why),
        Ending::Unmapped(&words),
        Ending::Moved(moved),
    ] {
        let told = notice("s", "a", &ending);
        assert!(told.chars().count() <= 400, "{told}");
        let (body, _) = told
            .rsplit_once(" Next: ")
            .unwrap_or_else(|| panic!("whole next step: {told}"));
        assert!(body.ends_with('…'), "cut is explicit: {told}");
        assert!(
            body.trim_end_matches('…')
                .trim_end()
                .ends_with("boundaryword"),
            "no partial word: {told}"
        );
    }
}
