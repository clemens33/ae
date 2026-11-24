#![allow(
    clippy::expect_used,
    reason = "fixture failures identify the missing prerequisite"
)]

use super::*;
use std::fs;
use std::path::{Path, PathBuf};

const SESSION: &str = "d04bc042-f51e-4fea-a814-b8bcac07557e";
const OTHER: &str = "b72543cb-c102-45c6-a4f6-81950a066560";
const OWNER_ASK: &str = "ae-20260925T100000Z-deadbeef";
const RELAY: &str = "ae-20260925T100001Z-cafebabe";
const ON: &str = "all\nauto_reseat_at = 95\nauto_reseat_grace_secs = 0";
const FACT: &str = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"The compass word is olive.\"}}\n";
const CHARTER: &str = include_str!("../../../contrib/aeorchestrator/orchestrator.config");

struct SeatRig {
    rig: Rig,
    name: &'static str,
    transcript: PathBuf,
    other_store: PathBuf,
    pid: u32,
}

impl SeatRig {
    fn new(tag: &str, orchestrator: bool, candidates: &str) -> Self {
        let rig = Rig::new(tag);
        pin_profile(&rig, "fake-claude-a", "claude-opus-5");
        pin_profile(&rig, "fake-claude-b", "claude-opus-5");
        Self::with_rig(rig, orchestrator, candidates)
    }

    fn with_rig(rig: Rig, orchestrator: bool, candidates: &str) -> Self {
        configure_from(&rig, ON, "fake-claude-a", candidates);
        let scratch = fs::canonicalize(&rig.scratch).expect("canonical scratch");
        let work = scratch.join("work");
        let from = scratch.join("home-a");
        let to = scratch.join("home-b");
        let home = scratch.join("home");
        for path in [&from, &to, &home] {
            fs::create_dir_all(path).expect("account directory");
        }
        let project = ae::carry::project_key(&work);
        let transcript = from
            .join("projects")
            .join(&project)
            .join(format!("{CARRIED}.jsonl"));
        let other_store = to
            .join("projects")
            .join(&project)
            .join(format!("{CARRIED}.jsonl"));
        fs::create_dir_all(transcript.parent().expect("project directory"))
            .expect("source Claude project");
        fs::write(&transcript, FACT).expect("source transcript");
        quota(&from, 96);
        quota(&to, 2);

        let name = if orchestrator { "orchestrator" } else { "lead" };
        rebind(&rig, "seat.main", name);
        rebind(&rig, "profile.main", "fake-claude-a");
        rebind(&rig, "agent_bin.main", "claude");
        rebind(&rig, "work_dir", work.to_str().expect("plain fixture path"));
        let mut rows = format!(
            "session_id={SESSION}\nlaunch_id.main={CARRIED}\nharness_session.main={CARRIED}\n"
        );
        if orchestrator {
            let overlay = rig.scratch.join("orchestrator.config");
            fs::write(&overlay, CHARTER).expect("seat charter");
            writeln!(rows, "meta_agent=true\nlocal_config={}", overlay.display())
                .expect("role and overlay");
        }
        fs::write(rig.dir.join("meta"), rig.meta() + &rows).expect("main seat metadata");
        assert!(
            rig.tmux(&[
                "set-option",
                "-t",
                &rig.session,
                "@ae_session_uuid",
                SESSION
            ])
            .0
        );
        // A production launch starts its shell in work_dir with this state root.
        // Rig's generic initial shell is elsewhere and has no AE_HOME.
        let shell = format!(
            "exec env AE_HOME={} HOME={} /bin/sh",
            rig.scratch.display(),
            home.display()
        );
        assert!(
            rig.tmux(&[
                "respawn-pane",
                "-k",
                "-t",
                &rig.main_pane,
                "-c",
                work.to_str().expect("plain fixture path"),
                &shell,
            ])
            .0
        );
        assert!(
            rig.tmux(&["set-option", "-p", "-t", &rig.main_pane, "@ae_agent", name])
                .0
        );
        rig.start(&rig.main_pane, "main", "claude");
        let pid = rig
            .tool_pid(&rig.main_pane, "claude")
            .expect("the initial tool");
        assert_eq!(rig.meta_row("harness_session.main"), CARRIED);
        assert!(
            rig.capture(&rig.main_pane).contains("done (0s)"),
            "the source shows a proven idle frame"
        );
        Self {
            rig,
            name,
            transcript,
            other_store,
            pid,
        }
    }

    fn append(&self, line: &str) {
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.rig.dir.join("events.jsonl"))
            .and_then(|mut f| writeln!(f, "{line}"))
            .expect("append fixture journal row");
    }

    fn records(&self, action: &str) -> Vec<Event> {
        records(&self.rig)
            .into_iter()
            .filter(|event| {
                event.action == action
                    && ae::watchdog::event_is_addressed_to(
                        event,
                        &self.rig.session,
                        "main",
                        self.name,
                    )
            })
            .collect()
    }

    // The daemon leaves these records when it dispatches a leg. Supplying
    // them lets the refusal cases isolate the leg's last admission check.
    fn attempt(&self) -> (Option<i32>, String, String) {
        self.attempt_with_summary("headroom 96% weekly_all 7d")
    }

    fn attempt_with_summary(&self, summary: &str) -> (Option<i32>, String, String) {
        self.attempt_for("auto-reseat-headroom", summary)
    }

    fn attempt_for(&self, trigger: &str, summary: &str) -> (Option<i32>, String, String) {
        let key = self.open_attempt(trigger, summary);
        self.attempt_command(key)
            .output()
            .map(|out| {
                (
                    out.status.code(),
                    String::from_utf8_lossy(&out.stdout).into_owned(),
                    String::from_utf8_lossy(&out.stderr).into_owned(),
                )
            })
            .expect("the detached leg runs")
    }

    fn open_attempt(&self, trigger: &str, summary: &str) -> Timestamp {
        let key = Timestamp::from_epoch(Timestamp::now().epoch() - 1);
        for (action, extra) in [
            (trigger, format!(r#","summary":"{summary}""#)),
            (ATTEMPT_ACTION, format!(r#","ref":"{key}""#)),
        ] {
            self.append(&format!(
                r#"{{"ts":"{key}","actor":"watchdog","action":"{action}","target":"{}","target_slot":"main","target_session":"{}"{extra}}}"#,
                self.name, self.rig.session,
            ));
        }
        key
    }

    fn attempt_command(&self, key: Timestamp) -> super::super::cli::Runner {
        let mut command = super::super::cli::ae();
        command
            .env("TMUX", format!("{},0,0", self.rig.sock.display()))
            .env_remove("TMUX_PANE")
            .env("AE_HOME", &self.rig.scratch)
            .env("HOME", self.rig.scratch.join("home"))
            .env("CONFIG_FILE", self.rig.scratch.join("config"))
            .arg("_auto-reseat")
            .arg(&self.rig.dir)
            .args(["main", &key.to_string()]);
        command
    }

    fn held(&self, reason: &str) {
        self.assert_held(&self.attempt(), reason);
    }

    fn assert_held(&self, receipt: &(Option<i32>, String, String), reason: &str) {
        assert_ne!(receipt.0, Some(0), "{receipt:?}");
        self.assert_reason(&[HELD_ACTION], reason);
        self.assert_unmoved();
    }

    fn assert_denied(&self, receipt: &(Option<i32>, String, String), reason: &str) {
        assert_ne!(receipt.0, Some(0), "{receipt:?}");
        self.assert_reason(&[HELD_ACTION, REFUSED_ACTION], reason);
        self.assert_unmoved();
    }

    fn assert_reason(&self, actions: &[&str], reason: &str) {
        assert!(
            actions
                .iter()
                .any(|action| self.records(action).iter().any(|event| {
                    event
                        .summary
                        .as_deref()
                        .is_some_and(|text| text.contains(reason))
                })),
            "expected {reason:?} in {actions:?}:\n{}",
            self.rig.events()
        );
    }

    fn assert_unmoved(&self) {
        assert_eq!(
            self.rig.tool_pid(&self.rig.main_pane, "claude"),
            Some(self.pid)
        );
        assert_eq!(self.rig.meta_row("profile.main"), "fake-claude-a");
        assert_eq!(self.rig.meta_row("harness_session.main"), CARRIED);
        assert!(self.records("reseat").is_empty(), "no stop recorded");
        assert!(!self.other_store.exists(), "no carry published");
    }
}

fn pin_profile(rig: &Rig, profile: &str, model: &str) {
    let path = rig.scratch.join("config");
    let text = fs::read_to_string(&path).expect("fixture config");
    let row = text
        .lines()
        .find(|line| line.starts_with(&format!("{profile} = \"")))
        .expect("fixture profile row");
    let command = row.strip_suffix('"').expect("quoted command");
    let command = command
        .split_once(" --model ")
        .map_or(command, |(base, _)| base);
    fs::write(
        &path,
        text.replacen(row, &format!("{command} --model {model}\""), 1),
    )
    .expect("fixture profile pin");
}

fn model_seat(tag: &str, model: Option<&str>, orchestrator: bool) -> SeatRig {
    let rig = Rig::new(tag);
    pin_profile(&rig, "fake-claude-a", "claude-opus-5");
    if let Some(model) = model {
        pin_profile(&rig, "fake-claude-b", model);
    }
    let seat = SeatRig::with_rig(rig, orchestrator, "fake-claude-b");
    remove_unused_profiles(&seat.rig, &["fake-claude"]);
    seat
}

fn set_policy(seat: &SeatRig, policy: &str) {
    configure_from(
        &seat.rig,
        &format!("{ON}\nauto_reseat_orchestrator = {policy}"),
        "fake-claude-a",
        "fake-claude-b",
    );
}

#[test]
fn orchpin_headroom_applies_the_global_policy_only_to_the_orchestrator() {
    for (tag, role, policy, model, moves) in [
        ("opdefaultok", true, None, "claude-opus-5", true),
        ("opon", true, Some("on"), "claude-sonnet-5", true),
        ("opordinary", false, Some("off"), "claude-sonnet-5", true),
        ("opdefault", true, None, "claude-sonnet-5", false),
        (
            "opexplicit",
            true,
            Some("same-model"),
            "claude-sonnet-5",
            false,
        ),
        ("opoff", true, Some("off"), "claude-opus-5", false),
        ("opbad", true, Some("wrong"), "claude-sonnet-5", false),
    ] {
        let seat = model_seat(tag, Some(model), role);
        let rig = &seat.rig;
        if let Some(policy) = policy {
            set_policy(&seat, policy);
        }
        let receipt = seat.attempt();
        if moves {
            assert_eq!(receipt.0, Some(0), "{tag}: {receipt:?}\n{}", rig.events());
            assert_eq!(rig.meta_row("profile.main"), "fake-claude-b");
            assert_eq!(rig.meta_row("harness_session.main"), CARRIED);
            assert_ne!(rig.tool_pid(&rig.main_pane, "claude"), Some(seat.pid));
        } else {
            let expected = if policy == Some("off") { "off" } else { model };
            seat.assert_denied(&receipt, expected);
        }
    }
}

#[test]
fn orchpin_skips_a_declared_different_model_and_takes_the_matching_login() {
    let seat = model_seat("opchoice", Some("claude-opus-5"), true);
    let rig = &seat.rig;
    let path = rig.scratch.join("config");
    let config = fs::read_to_string(&path).expect("fixture config");
    let matching = config
        .lines()
        .find(|line| line.starts_with("fake-claude-b = "))
        .expect("matching login profile");
    let other = matching
        .replacen("fake-claude-b =", "fake-claude-other =", 1)
        .replace("claude-opus-5", "claude-sonnet-5");
    fs::write(
        &path,
        config.replacen(matching, &format!("{matching}\n{other}"), 1),
    )
    .expect("a different model on the other login");
    configure_from(rig, ON, "fake-claude-a", "fake-claude-other, fake-claude-b");
    let receipt = seat.attempt();
    assert_eq!(receipt.0, Some(0), "{receipt:?}\n{}", rig.events());
    assert_eq!(rig.meta_row("profile.main"), "fake-claude-b");
    assert_eq!(rig.meta_row("harness_session.main"), CARRIED);
}

#[test]
fn orchpin_unknown_or_family_only_candidate_does_not_prove_the_exact_model() {
    for (tag, model) in [("opunknown", None), ("opalias", Some("opus"))] {
        let seat = model_seat(tag, model, true);
        seat.assert_denied(&seat.attempt(), "fake-claude-b");
    }
}

#[test]
fn orchpin_a_profile_edit_does_not_change_the_running_model() {
    let seat = model_seat("oplive", Some("claude-sonnet-5"), true);
    let rig = &seat.rig;
    pin_profile(rig, "fake-claude-a", "claude-sonnet-5");
    let frame = rig.capture(&rig.main_pane);
    assert!(frame.contains("Opus 5"), "{frame}");
    seat.assert_denied(&seat.attempt(), "claude-sonnet-5");
}

#[test]
fn orchpin_vendor_limit_can_only_take_the_same_model() {
    for (tag, model, moves) in [
        ("oplimitsame", "claude-opus-5", true),
        ("oplimitwrong", "claude-sonnet-5", false),
    ] {
        let seat = model_seat(tag, Some(model), true);
        let rig = &seat.rig;
        // The vendor frame is the authority for this trigger. Keep the quota
        // cache below the headroom bar so only the limit can cause a move.
        quota(&rig.scratch.join("home-a"), 2);
        rig.mark_limited(&rig.main_pane);
        let receipt = seat.attempt_for("limit", "usage limit");
        if moves {
            assert_eq!(receipt.0, Some(0), "{tag}: {receipt:?}\n{}", rig.events());
            assert_eq!(rig.meta_row("profile.main"), "fake-claude-b");
            assert_eq!(rig.meta_row("harness_session.main"), CARRIED);
            assert_ne!(rig.tool_pid(&rig.main_pane, "claude"), Some(seat.pid));
        } else {
            seat.assert_denied(&receipt, model);
        }
    }
}

#[test]
fn orchpin_global_on_still_excludes_main_even_with_the_policy_on() {
    let seat = model_seat("opmainon", Some("claude-opus-5"), true);
    configure_from(
        &seat.rig,
        "on\nauto_reseat_at = 95\nauto_reseat_grace_secs = 0\nauto_reseat_orchestrator = on",
        "fake-claude-a",
        "fake-claude-b",
    );
    seat.held("main");
}

#[test]
fn orchpin_return_must_keep_the_running_model() {
    for (tag, origin, moves) in [
        ("opreturnsame", "claude-opus-5", true),
        ("opreturnwrong", "claude-sonnet-5", false),
    ] {
        let seat = model_seat(tag, Some(origin), true);
        let rig = &seat.rig;
        // A pre-policy automatic carry from B left the running conversation
        // on A. The origin's fresh room is newer than that historical hop.
        let away = Timestamp::now().epoch() - 20;
        let first = Timestamp::from_epoch(away - 3);
        for (at, action, extra) in [
            (first, "auto-reseat-headroom", r#","summary":"headroom 96% weekly_all 7d""#.to_owned()),
            (Timestamp::from_epoch(away - 2), ATTEMPT_ACTION, format!(r#","ref":"{first}""#)),
            (Timestamp::from_epoch(away - 1), "reseat", r#","summary":"reseated orchestrator (pane %1, slot main) to fake-claude-a, resumed""#.to_owned()),
            (Timestamp::from_epoch(away), DONE_ACTION, r#","ref":"fake-claude-b","summary":"to fake-claude-a, carried (headroom 96% weekly_all 7d)""#.to_owned()),
        ] {
            seat.append(&format!(
                r#"{{"ts":"{at}","actor":"watchdog","action":"{action}","target":"{}","target_slot":"main","target_session":"{}"{extra}}}"#,
                seat.name, rig.session
            ));
        }
        let chain = ae::autoreseat::chain(&records(rig), &rig.session, "main", seat.name)
            .expect("a returnable pre-policy automatic chain");
        assert_eq!(chain.origin, "fake-claude-b");
        assert!(Timestamp::now().epoch() > chain.away_at.epoch());
        quota(&rig.scratch.join("home-a"), 2);
        let receipt = seat.attempt_for("auto-reseat-return", "return to origin: 2% weekly_all 7d");
        if moves {
            assert_eq!(receipt.0, Some(0), "{tag}: {receipt:?}\n{}", rig.events());
            assert_eq!(seat.records(DONE_ACTION).len(), 2, "{}", rig.events());
            assert_eq!(rig.meta_row("profile.main"), "fake-claude-b");
        } else {
            assert_ne!(receipt.0, Some(0), "{receipt:?}");
            seat.assert_reason(&[HELD_ACTION, REFUSED_ACTION], origin);
            assert_eq!(seat.records(DONE_ACTION).len(), 1, "{}", rig.events());
            assert_eq!(seat.records("reseat").len(), 1, "{}", rig.events());
            assert_eq!(rig.tool_pid(&rig.main_pane, "claude"), Some(seat.pid));
            assert_eq!(rig.meta_row("profile.main"), "fake-claude-a");
        }
        assert_eq!(rig.meta_row("harness_session.main"), CARRIED);
    }
}

#[test]
fn orchpin_late_policy_recheck_precedes_the_stop() {
    const BARRIER: &str = "@auto-reseat-pair";
    for (tag, policy, model) in [
        ("oplatesame", "same-model", "claude-sonnet-5"),
        ("oplateoff", "off", "claude-opus-5"),
    ] {
        let seat = model_seat(tag, Some(model), true);
        let rig = &seat.rig;
        set_policy(&seat, "on");
        let gate = rig.scratch.join("orchpin-read-gate");
        fs::create_dir(&gate).expect("private read gate");
        fs::write(gate.join(BARRIER), "").expect("arm the existing before-move seam");
        let key = seat.open_attempt("auto-reseat-headroom", "headroom 96% weekly_all 7d");
        let mut child = seat
            .attempt_command(key)
            .env("AE_TEST_APP_READ_GATE", &gate)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("start the real detached leg");
        let deadline = Instant::now() + BUDGET;
        let arrived = || {
            fs::read_to_string(gate.join("trace"))
                .is_ok_and(|trace| trace.lines().any(|line| line == "held @auto-reseat-pair"))
        };
        while !arrived() && Instant::now() < deadline {
            assert!(
                child.try_wait().expect("leg status").is_none(),
                "{}",
                rig.events()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            arrived(),
            "the leg chose its candidate before the config flip"
        );
        set_policy(&seat, policy);
        fs::remove_file(gate.join(BARRIER)).expect("release the existing seam");
        let out = child.wait_with_output().expect("late gate receipt");
        assert!(!out.status.success(), "{tag}: {out:?}\n{}", rig.events());
        assert!(seat.records(DONE_ACTION).is_empty(), "{}", rig.events());
        seat.assert_reason(&[HELD_ACTION, REFUSED_ACTION], "orchestrator-changed");
        seat.assert_unmoved();
    }
}

fn owner_quota_notices(seat: &SeatRig) -> Vec<String> {
    records(&seat.rig)
        .into_iter()
        .filter(|event| {
            event.action == "chat" && event.actor == "watchdog" && event.target.is_none()
        })
        .filter_map(|event| event.summary)
        .filter(|summary| summary.contains("weekly_all"))
        .collect()
}

fn wait_for_quota_notice(seat: &SeatRig, reset: Timestamp, manual: bool) {
    let rig = &seat.rig;
    assert!(
        watch_until(rig, || {
            !owner_quota_notices(seat).is_empty() || !seat.records(DONE_ACTION).is_empty()
        }),
        "{}\n{}",
        rig.events(),
        daemon_err(rig)
    );
    let notices = owner_quota_notices(seat);
    assert_eq!(notices.len(), 1, "{}\n{}", rig.events(), daemon_err(rig));
    let notice = &notices[0];
    assert!(notice.contains("reset"), "{notice}");
    assert!(notice.contains(&reset.to_string()[11..16]), "{notice}");
    if manual {
        let command = format!("ae reseat {} main --using fake-claude-b", rig.session);
        assert!(notice.contains(&command), "{notice}");
    } else {
        assert!(
            notice.to_ascii_lowercase().contains("no candidate") && notice.contains("configured"),
            "{notice}"
        );
        assert!(!notice.contains("ae reseat"), "{notice}");
    }
    seat.assert_unmoved();
}

#[test]
fn orchpin_quota_notice_dedupes_across_restart_and_does_not_prevent_recovery() {
    let seat = model_seat("opnotice", Some("claude-opus-5"), true);
    let rig = &seat.rig;
    let reset = quota(&rig.scratch.join("home-a"), 96);
    // The candidate is below hard exhaustion but past the headroom bar.
    quota(&rig.scratch.join("home-b"), 96);
    wait_for_quota_notice(&seat, reset, true);

    let restart = Instant::now();
    assert!(watch_until(rig, || restart.elapsed() >= Duration::from_secs(4)));
    assert_eq!(owner_quota_notices(&seat).len(), 1, "{}", rig.events());

    quota(&rig.scratch.join("home-b"), 2);
    assert!(
        watch_until(rig, || !seat.records(DONE_ACTION).is_empty()),
        "candidate recovery did not move:\n{}\n{}",
        rig.events(),
        daemon_err(rig)
    );
    assert_eq!(rig.meta_row("profile.main"), "fake-claude-b");
    assert_eq!(rig.meta_row("harness_session.main"), CARRIED);
}

#[test]
fn orchpin_quota_notice_also_reaches_the_owner_with_policy_off() {
    let seat = model_seat("opnoticeoff", Some("claude-opus-5"), true);
    let reset = quota(&seat.rig.scratch.join("home-a"), 96);
    set_policy(&seat, "off");
    wait_for_quota_notice(&seat, reset, true);
}

#[test]
fn orchpin_quota_notice_reaches_the_owner_without_a_configured_candidate() {
    let seat = model_seat("opnoticeempty", None, true);
    let rig = &seat.rig;
    let reset = quota(&rig.scratch.join("home-a"), 96);
    remove_unused_profiles(
        rig,
        &[
            "fake-claude-b",
            "fake-opencode",
            "fake-grok",
            "fake-codex",
            "fake-agy",
        ],
    );
    configure_from(rig, ON, "unrelated", "fake-claude-a");
    wait_for_quota_notice(&seat, reset, false);
}

#[test]
fn orchpin_unusable_policy_has_one_note_per_watchdog_start() {
    let seat = model_seat("opinvalid", Some("claude-sonnet-5"), true);
    let rig = &seat.rig;
    set_policy(&seat, "bad-setting");
    quota(&rig.scratch.join("home-a"), 2);
    let notes = || {
        records(rig)
            .into_iter()
            .filter(|event| {
                event.actor == "watchdog"
                    && event.target.is_none()
                    && event.summary.as_deref().is_some_and(|text| {
                        text.contains("auto_reseat_orchestrator")
                            && text.contains("bad-setting")
                            && text.contains("same-model")
                    })
            })
            .count()
    };
    for expected in 1..=2 {
        let start = Instant::now();
        assert!(
            watch_until(rig, || {
                notes() >= expected && start.elapsed() >= Duration::from_secs(4)
            }),
            "{}\n{}",
            rig.events(),
            daemon_err(rig)
        );
        assert_eq!(notes(), expected, "{}", rig.events());
    }
}

fn quota(home: &Path, percent: u8) -> Timestamp {
    let now = Timestamp::now().epoch();
    let reset = Timestamp::from_epoch(now + 86_400);
    fs::write(
        home.join(".claude.json"),
        format!(
            r#"{{"cachedUsageUtilization":{{"fetchedAtMs":{},"utilization":{{"limits":[{{"kind":"weekly_all","group":"weekly_all","percent":{percent},"resets_at":"{}","scope":null}}]}}}}}}"#,
            now * 1_000,
            reset,
        ),
    )
    .expect("fresh fixture weekly quota");
    reset
}

fn await_carried_continuation(seat: &SeatRig) {
    let rig = &seat.rig;
    let (created, pane) = rig.tmux(&[
        "new-window",
        "-d",
        "-t",
        &rig.session,
        "-P",
        "-F",
        "#{pane_id}",
        "sleep",
        "60",
    ]);
    assert!(created, "{pane}");
    let pane = pane.trim();
    assert!(
        rig.tmux(&["set-option", "-p", "-t", pane, "@ae_agent", "_watchdog"])
            .0
    );
    assert!(
        watch_until_from(rig, Some(pane), || {
            !seat.records(DONE_ACTION).is_empty()
                && !seat.records(ae::autoreseat::NOTICE_ACTION).is_empty()
                && rig.meta_row("reseat_continuation.main").is_empty()
        }),
        "no move:\n{}\n{}",
        rig.events(),
        daemon_err(rig)
    );
    let continuation = seat.records(ae::autoreseat::NOTICE_ACTION);
    assert_eq!(continuation.len(), 1, "the leg and daemon share one turn");
    assert_eq!(
        continuation[0].reference.as_deref(),
        Some(rig.meta_row("launch_id.main").as_str()),
        "the daemon's checked continuation names the carried launch"
    );
    assert_eq!(
        continuation[0].summary.as_deref(),
        Some("continuation turn after an automatic carried reseat")
    );
}

#[test]
fn orchreseat_watchdog_skips_a_fresh_candidate_and_carries_the_main_conversation() {
    let seat = SeatRig::new("orcarry", true, "fake-opencode, fake-claude-b");
    let rig = &seat.rig;
    let desk = ae::desk::parse_input(b"DECIDE\nD7 | first draft\n").expect("fixture desk");
    ae::deskstore::set(&rig.dir, desk, Timestamp::now(), seat.name).expect("saved desk");
    let memo = format!(
        "{}\torchestrator\tgeneral\tKeep the decision.\n",
        Timestamp::now()
    );
    fs::write(rig.dir.join("memo.tsv"), &memo).expect("fixture memo");
    let board = fs::read(rig.dir.join("desk-record")).expect("desk bytes");
    let initial_launch = rig.meta_row("launch_id.main");
    let charter = rig.meta_row("charter.main");
    assert!(
        !charter.is_empty(),
        "the original start stamped its charter"
    );

    await_carried_continuation(&seat);
    assert_eq!(rig.meta_row("profile.main"), "fake-claude-b");
    assert_eq!(rig.meta_row("harness_session.main"), CARRIED);
    assert_ne!(rig.tool_pid(&rig.main_pane, "claude"), Some(seat.pid));
    assert!(rig.tool_pid(&rig.main_pane, "claude").is_some());
    assert!(
        rig.launched().contains(&format!("--resume {CARRIED}")),
        "{}",
        rig.launched()
    );
    assert_eq!(
        fs::read_to_string(&seat.transcript).expect("source transcript intact"),
        FACT
    );
    assert_eq!(
        fs::read_to_string(&seat.other_store).expect("destination transcript"),
        FACT
    );
    assert!(
        !rig.dir.join("seed.orchestrator.md").exists(),
        "no fresh seed"
    );
    assert_eq!(rig.meta_row("meta_agent"), "true");
    assert_eq!(rig.meta_row("session_id"), SESSION);
    assert_eq!(rig.meta_row("main_pane"), rig.main_pane);
    assert_ne!(rig.meta_row("launch_id.main"), initial_launch);
    assert!(!rig.meta_row("launch_id.main").is_empty());
    assert_eq!(rig.meta_row("charter.main"), charter);
    assert_eq!(
        rig.meta_row("charter_launch.main"),
        rig.meta_row("launch_id.main")
    );
    assert!(
        rig.meta_row("context.main")
            .parse::<u32>()
            .is_ok_and(|stamp| stamp > 0)
    );
    assert_eq!(
        fs::read(rig.dir.join("desk-record")).expect("desk bytes"),
        board
    );
    assert_eq!(
        fs::read_to_string(rig.dir.join("memo.tsv")).expect("memo bytes"),
        memo
    );
    assert_eq!(
        fs::read_to_string(rig.meta_row("local_config")).expect("charter overlay"),
        CHARTER
    );

    // Exercise the guard left by the move itself. The separate _run refusal
    // fixture supplies a record and cannot catch a move that never writes it.
    rig.kill_tools(&rig.main_pane);
    fs::remove_file(&seat.other_store).expect("lose the destination transcript before restart");
    let restart = super::super::cli::ae()
        .env("AE_HOME", &rig.scratch)
        .env("HOME", rig.scratch.join("home"))
        .env("CONFIG_FILE", rig.scratch.join("config"))
        .current_dir(rig.meta_row("work_dir"))
        .args([ae::cli::RUN, "--print"])
        .arg(&rig.dir)
        .arg("main")
        .output()
        .expect("check the successor's restart plan");
    assert!(!restart.status.success(), "{restart:?}");
    let refusal = String::from_utf8_lossy(&restart.stderr);
    assert!(
        refusal.contains("refusing a fresh start") && refusal.contains(CARRIED),
        "{refusal}"
    );
    assert_eq!(rig.meta_row("harness_session.main"), CARRIED);
}

#[test]
fn orchreseat_same_account_move_requires_the_recorded_conversation() {
    for (tag, present) in [("orkeep", true), ("orkeepgap", false)] {
        let rig = Rig::new(tag);
        let path = rig.scratch.join("config");
        let config = fs::read_to_string(&path).expect("fixture config");
        let source = config
            .lines()
            .find(|line| line.starts_with("fake-claude-a = "))
            .expect("source profile");
        let command = source
            .strip_prefix("fake-claude-a = \"")
            .and_then(|text| text.strip_suffix('"'))
            .expect("quoted source command");
        let profiles = format!(
            "fake-claude-a = \"{command} --model opus\"\nfake-claude-keep = \"{command} --model sonnet\""
        );
        fs::write(&path, config.replacen(source, &profiles, 1)).expect("same-account profiles");
        let seat = SeatRig::with_rig(rig, true, "fake-claude-keep");
        let rig = &seat.rig;
        remove_unused_profiles(rig, &["fake-claude", "fake-claude-b"]);
        configure_from(
            rig,
            &format!("{ON}\nauto_reseat_orchestrator = on"),
            "fake-claude-a",
            "fake-claude-keep",
        );

        // The source hits the Opus window. Sonnet uses the same account but
        // only the account-wide window applies to that model.
        let now = Timestamp::now().epoch();
        let reset = Timestamp::from_epoch(now + 86_400);
        fs::write(
            rig.scratch.join("home-a/.claude.json"),
            format!(
                r#"{{"cachedUsageUtilization":{{"fetchedAtMs":{},"utilization":{{"limits":[{{"kind":"weekly_scoped","group":"weekly_scoped","percent":96,"resets_at":"{reset}","scope":{{"model":{{"display_name":"Opus"}}}}}},{{"kind":"weekly_all","group":"weekly_all","percent":10,"resets_at":"{reset}","scope":null}}]}}}}}}"#,
                now * 1_000
            ),
        )
        .expect("model-scoped quota");
        if !present {
            fs::remove_file(&seat.transcript).expect("remove the recorded conversation");
        }
        let home = rig.meta_row("config_home.main");
        assert!(!home.is_empty(), "the initial launch recorded its account");
        let receipt = seat.attempt_with_summary("headroom 96% weekly_scoped 7d");
        if present {
            assert_eq!(receipt.0, Some(0), "{receipt:?}\n{}", rig.events());
            assert!(
                seat.records(DONE_ACTION).iter().any(|done| done
                    .summary
                    .as_deref()
                    .is_some_and(|summary| summary.starts_with("to fake-claude-keep, kept"))),
                "{}",
                rig.events()
            );
            assert_eq!(rig.meta_row("profile.main"), "fake-claude-keep");
            assert_eq!(rig.meta_row("harness_session.main"), CARRIED);
            assert_eq!(rig.meta_row("config_home.main"), home);
            assert!(
                rig.launched().lines().any(|line| {
                    line.contains("--model sonnet") && line.contains(&format!("--resume {CARRIED}"))
                }),
                "{}",
                rig.launched()
            );
            assert_eq!(
                fs::read_to_string(&seat.transcript).expect("retained transcript"),
                FACT
            );
        } else {
            seat.assert_held(
                &receipt,
                "no candidate keeps the orchestrator's conversation",
            );
            assert!(seat.records(REFUSED_ACTION).is_empty(), "{}", rig.events());
        }
    }
}

fn record_pending_owner_request(seat: &SeatRig) {
    let rig = &seat.rig;
    seat.append(&format!(
        r#"{{"ts":"{}","actor":"console:local","action":"ask","target":"{}","target_slot":"main","target_session":"{}","target_session_id":"{SESSION}","target_session_uuid":"{SESSION}","target_server":"{}","target_pane":"{}","ref":"{OWNER_ASK}","summary":"Wait for this answer"}}"#,
        Timestamp::now(), seat.name, rig.session, rig.sock.display(), rig.main_pane,
    ));
    assert!(
        ae::requests::states(rig.events().as_bytes())
            .iter()
            .any(|request| {
                request.id == OWNER_ASK.as_bytes()
                    && request.status == ae::requests::Status::Pending
            }),
        "fixture is a pending request"
    );
}

#[test]
fn orchreseat_holds_a_pending_owner_chat_request() {
    let seat = SeatRig::new("orask", true, "fake-claude-b");
    record_pending_owner_request(&seat);
    seat.held("in flight");
}

fn record_relay_claim(seat: &SeatRig) {
    seat.append(&format!(
        r#"{{"ts":"{}","actor":"orchestrator","actor_slot":"main","actor_session":"{}","actor_session_id":"{SESSION}","actor_launch_id":"{CARRIED}","action":"relay-attempt","target":"project","target_slot":"main","target_session":"project","target_session_id":"{OTHER}","target_session_uuid":"{OTHER}","ref":"{RELAY}","relay_mode":"interim","summary":"Relay the owner's request"}}"#,
        Timestamp::now(), seat.rig.session,
    ));
}

#[test]
fn orchreseat_holds_a_claimed_relay_without_a_result() {
    let seat = SeatRig::new("orrelay", true, "fake-claude-b");
    record_relay_claim(&seat);
    assert_eq!(
        ae::relay::claimed(&records(&seat.rig), RELAY, SESSION)
            .expect("fixture relay claim")
            .outcome,
        None
    );
    seat.held("in flight");
}

#[test]
fn orchreseat_moves_after_a_claimed_relay_has_a_result() {
    let seat = SeatRig::new("orrelaydone", true, "fake-claude-b");
    record_relay_claim(&seat);
    seat.append(&format!(
        r#"{{"ts":"{}","actor":"orchestrator","actor_slot":"main","actor_session":"{}","actor_session_id":"{SESSION}","actor_launch_id":"{CARRIED}","action":"relay-result","target":"project","target_slot":"main","target_session":"project","target_session_id":"{OTHER}","target_session_uuid":"{OTHER}","ref":"{RELAY}","outcome":"sent"}}"#,
        Timestamp::now(), seat.rig.session,
    ));
    let rig = &seat.rig;
    assert_eq!(
        ae::relay::claimed(&records(rig), RELAY, SESSION)
            .expect("fixture relay claim")
            .outcome,
        Some("sent")
    );
    let receipt = seat.attempt();
    assert_eq!(receipt.0, Some(0), "{receipt:?}\n{}", rig.events());
    assert_eq!(seat.records(DONE_ACTION).len(), 1, "{}", rig.events());
    assert_eq!(rig.meta_row("profile.main"), "fake-claude-b");
    assert_eq!(rig.meta_row("harness_session.main"), CARRIED);
}

#[test]
fn orchreseat_holds_while_the_console_has_admitted_a_send() {
    let seat = SeatRig::new("orcompose", true, "fake-claude-b");
    let _admission = ae::store::open(&seat.rig.dir)
        .console_admission_now()
        .expect("console owns admission");
    seat.held("in flight");
}

#[test]
fn orchreseat_holds_if_every_candidate_would_start_a_new_conversation() {
    let seat = SeatRig::new("orfresh", true, "fake-opencode");
    remove_unused_profiles(&seat.rig, &["fake-claude", "fake-claude-b"]);
    seat.held("conversation");
}

#[test]
fn orchreseat_holds_an_open_attempt_when_switched_off() {
    let seat = SeatRig::new("oroff", true, "fake-claude-b");
    configure_from(&seat.rig, "off", "fake-claude-a", "fake-claude-b");
    seat.held("auto reseat is off");
}

#[test]
fn orchreseat_does_not_change_the_normal_main_seats_fresh_move() {
    let seat = SeatRig::new("orordinary", false, "fake-opencode");
    record_pending_owner_request(&seat);
    let receipt = seat.attempt();
    assert_eq!(receipt.0, Some(0), "{receipt:?}\n{}", seat.rig.events());
    assert_eq!(seat.rig.meta_row("profile.main"), "fake-opencode");
    assert!(seat.rig.tool_pid(&seat.rig.main_pane, "opencode").is_some());
    assert_eq!(seat.records(DONE_ACTION).len(), 1, "{}", seat.rig.events());
}

#[test]
fn orchreseat_holds_an_owner_request_opened_by_its_main_seat() {
    const OWNER: &str = "telegram:42";
    let seat = SeatRig::new("orownreply", true, "fake-claude-b");
    let asks = seat
        .rig
        .run(ae::cli::ASK, &[OWNER, "Please choose a direction."]);
    assert_eq!(asks.0, Some(0), "{asks:?}");
    let id = asks.1.trim();
    assert!(
        ae::requests::states(seat.rig.events().as_bytes())
            .iter()
            .any(|request| {
                request.id == id.as_bytes()
                    && request.from == ae::tracked::ORCHESTRATOR_DISPLAY.as_bytes()
                    && request.to == OWNER.as_bytes()
                    && request.status == ae::requests::Status::Pending
            }),
        "fixture opened a pending request to the owner: {}",
        seat.rig.events()
    );
    assert!(
        records(&seat.rig).iter().any(|event| {
            event.action == "ask"
                && event.reference.as_deref() == Some(id)
                && event.actor_slot.value() == Some("main")
                && event.actor_session_id.value() == Some(SESSION)
                && event.actor_launch_id.value() == Some(CARRIED)
        }),
        "the opening binds the orchestrator's current launch"
    );
    seat.held("in flight");
}

#[test]
fn orchreseat_refuses_a_move_without_a_launch_to_guard() {
    let seat = SeatRig::new("ornolaunch", true, "fake-claude-b");
    let rig = &seat.rig;
    rebind(rig, "launch_id.main", "");
    let receipt = seat.attempt();
    assert_ne!(receipt.0, Some(0), "{receipt:?}");
    assert!(
        receipt
            .2
            .contains("records no launch or conversation to guard"),
        "{receipt:?}\n{}",
        rig.events()
    );
    assert_eq!(rig.tool_pid(&rig.main_pane, "claude"), Some(seat.pid));
    assert_eq!(rig.meta_row("profile.main"), "fake-claude-a");
    assert_eq!(rig.meta_row("harness_session.main"), CARRIED);
    assert!(seat.records("reseat").is_empty(), "no stop recorded");
    assert!(!seat.other_store.exists(), "no carry published");
}

#[test]
fn orchreseat_a_copy_write_failure_after_the_stop_cannot_seed() {
    use std::os::unix::fs::PermissionsExt as _;

    let seat = SeatRig::new("orcopyfail", true, "fake-claude-b");
    let rig = &seat.rig;
    let project = seat.other_store.parent().expect("destination project");
    fs::create_dir_all(project).expect("destination account has its project");
    fs::write(&seat.other_store, FACT).expect("the target already has the transcript");
    let source_sidecar = seat
        .transcript
        .parent()
        .expect("source project")
        .join(CARRIED);
    fs::create_dir(&source_sidecar).expect("source conversation sidecar");
    fs::write(source_sidecar.join("note"), "a sidecar the carry must copy")
        .expect("source sidecar content");
    // The survey can read the matching transcript and the missing sidecar.
    // Copy cannot create the sidecar even though the resume probe finds the
    // target transcript. A failed carry still requires the original profile.
    fs::set_permissions(project, fs::Permissions::from_mode(0o500))
        .expect("deny publication to the target");
    let receipt = seat.attempt();
    fs::set_permissions(project, fs::Permissions::from_mode(0o700))
        .expect("allow the fixture cleanup");
    let events = records(rig);
    assert!(
        with_action(&events, "reseat").iter().any(|event| {
            event
                .summary
                .as_deref()
                .is_some_and(|summary| summary.starts_with("stopped claude"))
        }),
        "the failure is past the stop: {receipt:?}\n{}",
        rig.events()
    );
    assert_ne!(receipt.0, Some(0), "{receipt:?}");
    assert_eq!(rig.meta_row("profile.main"), "fake-claude-a");
    assert_eq!(rig.meta_row("harness_session.main"), CARRIED);
    assert_eq!(
        fs::read_to_string(&seat.transcript).expect("source transcript"),
        FACT
    );
    assert_eq!(
        fs::read_to_string(&seat.other_store).expect("target transcript still present"),
        FACT
    );
    assert!(
        !project.join(CARRIED).exists(),
        "the sidecar was not copied"
    );
    assert!(
        !rig.dir.join("seed.orchestrator.md").exists(),
        "no seeded successor"
    );
    assert!(seat.records(DONE_ACTION).is_empty(), "no successful move");
    assert_eq!(
        rig.launched().matches("--session-id ").count(),
        1,
        "{}",
        rig.launched()
    );
}

#[test]
fn orchreseat_strict_resume_cannot_exec_a_fresh_tool_after_a_store_gap() {
    for (tag, started) in [("orstrictresume", true), ("orstrictstart", false)] {
        let seat = SeatRig::new(tag, true, "fake-claude-b");
        let rig = &seat.rig;
        rig.kill_tools(&rig.main_pane);
        let launch = rig.meta_row("launch_id.main");
        fs::write(
            rig.dir.join("must_resume.main"),
            format!("{CARRIED} {launch}\n"),
        )
        .expect("the record a strict move leaves");
        fs::remove_file(&seat.transcript).expect("lose the conversation before _run");
        if !started {
            fs::remove_file(ae::run::started_marker(&rig.dir, "main"))
                .expect("model an interrupted start-marker write");
        }

        // If _run falls back, the fake records the exec and returns without
        // needing a terminal. A failure from the fake cannot pass this check.
        let executed = rig.scratch.join("unexpected-exec");
        fs::write(
            rig.scratch.join("claude.pl"),
            format!(
                "open my $f, '>', '{}' or die; exit 0;\n",
                executed.display()
            ),
        )
        .expect("record a forbidden exec");
        let out = super::super::cli::ae()
            .env("AE_HOME", &rig.scratch)
            .env("HOME", rig.scratch.join("home"))
            .env("CONFIG_FILE", rig.scratch.join("config"))
            .current_dir(rig.meta_row("work_dir"))
            .arg("_run")
            .arg(&rig.dir)
            .arg("main")
            .output()
            .expect("_run returns");
        assert!(!out.status.success(), "{tag}: {out:?}");
        assert!(!executed.exists(), "{tag}: _run entered the harness");
        assert_eq!(rig.meta_row("harness_session.main"), CARRIED, "{tag}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("resum") && stderr.contains(CARRIED),
            "{tag}: {stderr}"
        );
    }
}
