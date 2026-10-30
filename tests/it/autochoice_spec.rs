//! #269: real automatic candidate/leg/lifecycle/persistence path; external tools are fakes.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures own scratch stores, processes and private tmux"
)]

use std::time::{Duration, Instant};

use ae::autoreseat::{ATTEMPT_ACTION, DONE_ACTION, FAILED_ACTION, HELD_ACTION, REFUSED_ACTION};
use ae::events::Event;
use ae::time::Timestamp;

use super::seat_relaunch::Rig;

const SLOT: &str = "spawned.0";
const ID: &str = "12345678-1234-1234-1234-123456789abc";
const WAIT: Duration = Duration::from_secs(90);

fn until(rig: &Rig, done: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(done(), "bounded wait: {}", rig.events());
}

fn row(rig: &Rig, key: &str, value: &str) {
    let prefix = format!("{key}=");
    let text = rig
        .meta()
        .lines()
        .filter(|line| !line.starts_with(&prefix))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(rig.dir.join("meta"), format!("{text}\n{key}={value}\n")).expect("fixture meta");
}

fn observation(rig: &Rig, model: &str, effort: bool) {
    row(rig, "observed_model.spawned.0", model);
    row(rig, "observed_model_pin.spawned.0", "opus");
    if effort {
        row(rig, "observed_effort.spawned.0", "high");
        row(rig, "observed_effort_pin.spawned.0", "xhigh");
    }
}

fn events(rig: &Rig) -> Vec<Event> {
    rig.events()
        .lines()
        .map(|line| Event::parse_line(line).expect("journal"))
        .collect()
}

fn booked(rig: &Rig, action: &str) -> Vec<Event> {
    events(rig)
        .into_iter()
        .filter(|e| {
            e.action == action
                && ae::watchdog::event_is_addressed_to(e, &rig.session, SLOT, "scout")
        })
        .collect()
}

fn journal(rig: &Rig, action: &str, key: Timestamp, reference: bool) {
    let reference = if reference {
        format!(",\"ref\":\"{key}\"")
    } else {
        String::new()
    };
    let summary = if action == "auto-reseat-headroom" {
        ",\"summary\":\"headroom 95% weekly_all 7d\""
    } else {
        ""
    };
    let line = format!(
        "{}{{\"ts\":\"{key}\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"scout\",\"target_slot\":\"{SLOT}\",\"target_session\":\"{}\"{reference}{summary}}}\n",
        rig.events(),
        rig.session
    );
    std::fs::write(rig.dir.join("events.jsonl"), line).expect("episode");
}

fn core(rig: &Rig) -> super::cli::Runner {
    let mut runner = super::cli::ae();
    runner
        .env("HOME", rig.scratch.join("home"))
        .env("AE_HOME", &rig.scratch)
        .env("CONFIG_FILE", rig.scratch.join("config"))
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", &rig.main_pane)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("AE_SENDER_OVERRIDE");
    runner
}

fn code(out: &std::process::Output, expected: i32) {
    assert_eq!(
        out.status.code(),
        Some(expected),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn trigger(rig: &Rig) {
    let out = core(rig)
        .args([&format!("@{}", rig.session), "send", "scout", "auto reseat"])
        .env("AE_SENDER_OVERRIDE", "watchdog")
        .env("_AE_EVENT_ACTION", ATTEMPT_ACTION)
        .output()
        .expect("real automatic trigger");
    code(&out, 0);
}

fn leg(rig: &Rig, key: Timestamp) {
    let out = core(rig)
        .arg("_auto-reseat")
        .arg(&rig.dir)
        .args([SLOT, &key.to_string()])
        .output()
        .expect("real automatic leg");
    code(&out, 1);
}

fn setup(tag: &str, limit: bool, declared: bool) -> (Rig, String, Timestamp) {
    let rig = Rig::new(tag);
    let command = |home: &str| {
        format!(
            "CLAUDE_CONFIG_DIR={} {} {} --model opus --effort xhigh",
            rig.scratch.join(home).display(),
            rig.scratch.join("tools/claude").display(),
            rig.scratch.join("claude.pl").display()
        )
    };
    let mapping = if declared {
        "[auto_reseat]\nfake-source = fake-target\n"
    } else {
        ""
    };
    std::fs::write(rig.scratch.join("config"), format!("[profiles]\nfake-source = \"{}\"\nfake-target = \"{}\"\n[workspace]\nmain = lead\nlayout = vertical\nauto_reseat = on\nauto_reseat_grace_secs = 0\n{mapping}", command("home-a"), command("home-b"))).expect("config");
    ae::config::read_identity(Some(&rig.scratch.join("config")), None).expect("valid profiles");
    for (home, used) in [("home-a", 95), ("home-b", 10)] {
        let dir = rig.scratch.join(home);
        std::fs::create_dir_all(&dir).expect("account");
        let now = Timestamp::now().epoch();
        std::fs::write(dir.join(".claude.json"), format!("{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"weekly_all\",\"group\":\"weekly_all\",\"percent\":{used},\"resets_at\":\"{}\",\"scope\":null}}]}}}}}}\n", now * 1000, Timestamp::from_epoch(now + 86_400))).expect("quota fixture");
    }
    rig.seat_rows(SLOT, "scout", "source", "claude");
    let pane = rig.new_pane(SLOT, "scout");
    let home = rig.scratch.join("home");
    std::fs::create_dir_all(&home).expect("isolated home");
    assert!(
        rig.tmux(&[
            "respawn-pane",
            "-k",
            "-t",
            &pane,
            "env",
            "-u",
            "CLAUDE_CONFIG_DIR",
            "-u",
            "CODEX_HOME",
            &format!("HOME={}", home.display()),
            &format!("AE_HOME={}", rig.scratch.display()),
            &format!("CONFIG_FILE={}", rig.scratch.join("config").display()),
            "/bin/sh"
        ])
        .0
    );
    rig.start(&pane, SLOT, "claude");
    let root = std::fs::canonicalize(&rig.scratch).expect("canonical scratch");
    let work = root.join("work");
    row(&rig, "work_dir", &work.display().to_string());
    row(&rig, "harness_session.spawned.0", ID);
    row(
        &rig,
        "config_home.spawned.0",
        &root.join("home-a").display().to_string(),
    );
    let transcript = root
        .join("home-a/projects")
        .join(ae::carry::project_key(&work))
        .join(format!("{ID}.jsonl"));
    std::fs::create_dir_all(transcript.parent().expect("transcript parent")).expect("store");
    std::fs::write(
        transcript,
        "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n",
    )
    .expect("conversation");
    if limit {
        rig.mark_limited(&pane);
    }
    let key = Timestamp::from_epoch(Timestamp::now().epoch() - 10);
    journal(
        &rig,
        if limit {
            "limit"
        } else {
            "auto-reseat-headroom"
        },
        key,
        false,
    );
    (rig, pane, key)
}

/// Shared only with the independently migrated old headroom contract.
pub(super) fn trigger_and_assert_held(rig: &Rig, pane: &str, key: Timestamp) {
    let before = rig.meta();
    let pid = rig.tool_pid(pane, "claude").expect("original process");
    let launches = rig.launched();
    trigger(rig);
    until(rig, || {
        [HELD_ACTION, DONE_ACTION, REFUSED_ACTION, FAILED_ACTION]
            .iter()
            .any(|a| !booked(rig, a).is_empty())
    });
    assert_eq!(
        rig.meta(),
        before,
        "hold must preserve all seat/observation/store rows"
    );
    assert_eq!(
        rig.tool_pid(pane, "claude"),
        Some(pid),
        "hold never stops/restarts"
    );
    assert_eq!(rig.launched(), launches, "no successor launch");
    let held = booked(rig, HELD_ACTION);
    assert_eq!(held.len(), 1, "{}", rig.events());
    assert_eq!(held[0].reference.as_deref(), Some(key.to_string().as_str()));
    assert!(
        held[0]
            .summary
            .as_deref()
            .is_some_and(|s| s.starts_with("observed-choice: ")),
        "{held:?}"
    );
    for action in [DONE_ACTION, REFUSED_ACTION, FAILED_ACTION, "reseat"] {
        assert!(booked(rig, action).is_empty(), "{action}: {}", rig.events());
    }
    until(rig, || {
        rig.events()
            .contains(&format!("ae reseat {} scout --using", rig.session))
    });
    assert!(!rig.dir.join("seed.scout.md").exists(), "no context seed");
    // home-b is the twin only in the new fixtures. The migrated family uses
    // home-c; its no-move proof is the full meta/PID/launch assertions above.
    assert!(
        !rig.scratch.join("home-b/projects").exists(),
        "no home-b conversation copy"
    );
}

#[test]
fn automatic_headroom_and_limit_hold_model_effort_and_combined_choices() {
    for (tag, limit, declared, model, effort) in [
        ("achmodel", false, false, "Sonnet 5.5", false),
        ("aclimmodel", true, true, "Sonnet 5.5", false),
        ("acheffort", false, true, "Opus 5", true),
        ("aclimboth", true, false, "Sonnet 5.5", true),
    ] {
        let (rig, pane, key) = setup(tag, limit, declared);
        observation(&rig, model, effort);
        trigger_and_assert_held(&rig, &pane, key);
    }
}

#[test]
fn automatic_damage_holds_truthfully() {
    let (rig, pane, key) = setup("acdamage", false, false);
    row(&rig, "observed_effort.spawned.0", "high");
    row(&rig, "observed_effort_pin.spawned.0", "xhigh");
    trigger_and_assert_held(&rig, &pane, key);
    assert!(
        booked(&rig, HELD_ACTION)[0]
            .summary
            .as_deref()
            .is_some_and(|s| s.contains("damaged"))
    );
}

#[test]
fn no_observation_and_stale_observation_keep_existing_automatic_move() {
    for (tag, stale) in [("acabsent", false), ("acstale", true)] {
        let (rig, pane, _) = setup(tag, false, false);
        if stale {
            observation(&rig, "Sonnet 5.5", false);
            row(&rig, "observed_model_pin.spawned.0", "haiku");
        }
        trigger(&rig);
        until(&rig, || !booked(&rig, DONE_ACTION).is_empty());
        assert_eq!(rig.meta_row("profile.spawned.0"), "fake-target");
        assert_eq!(rig.meta_row("harness_session.spawned.0"), ID);
        assert!(rig.tool_pid(&pane, "claude").is_some());
        assert!(rig.meta_row("observed_model.spawned.0").is_empty());
        assert!(
            rig.launched().contains("--model opus"),
            "{}",
            rig.launched()
        );
    }
    let (rig, pane, _) = setup("acmanual", false, false);
    observation(&rig, "Sonnet 5.5", true);
    let out = core(&rig)
        .args(["reseat", &rig.session, "scout", "--using", "fake-target"])
        .output()
        .expect("manual choice");
    code(&out, 0);
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-target");
    assert_eq!(rig.meta_row("harness_session.spawned.0"), ID);
    for key in [
        "observed_model",
        "observed_model_pin",
        "observed_effort",
        "observed_effort_pin",
    ] {
        assert!(
            rig.meta_row(&format!("{key}.{SLOT}")).is_empty(),
            "manual choice resets {key}"
        );
    }
    assert!(rig.tool_pid(&pane, "claude").is_some());
    assert!(
        rig.launched().contains("--model opus"),
        "{}",
        rig.launched()
    );
}

#[test]
fn held_key_has_bounded_retries_and_no_duplicate_notice_or_destructive_work() {
    let (rig, pane, key) = setup("acretry", true, false);
    observation(&rig, "Opus 5", true);
    trigger_and_assert_held(&rig, &pane, key);
    let notices = || {
        events(&rig)
            .into_iter()
            .filter(|e| {
                e.summary.as_deref().is_some_and(|s| {
                    s.contains(&format!("ae reseat {} scout --using", rig.session))
                })
            })
            .count()
    };
    let before = notices();
    assert!(before > 0, "a recovery notice really landed");
    leg(&rig, key);
    assert_eq!(
        booked(&rig, HELD_ACTION).len(),
        1,
        "closed attempt is inert"
    );
    let pid = rig.tool_pid(&pane, "claude");
    let busy = "busy: held: the seat is busy";
    let draft = "busy: held: a human draft sits in the input box";
    for why in [busy, draft] {
        let session = &rig.session;
        let hold = format!(
            "{{\"ts\":\"{key}\",\"actor\":\"watchdog\",\"action\":\"{HELD_ACTION}\",\"target\":\"scout\",\"target_slot\":\"{SLOT}\",\"target_session\":\"{session}\",\"ref\":\"{key}\",\"summary\":\"{why}\"}}\n"
        );
        let path = rig.dir.join("events.jsonl");
        std::fs::write(path, format!("{}{hold}", rig.events())).expect("intervening daemon hold");
    }
    journal(&rig, ATTEMPT_ACTION, key, true);
    leg(&rig, key);
    assert_eq!(booked(&rig, HELD_ACTION).len(), 4);
    assert_eq!(notices(), before, "same key does not tell again");
    let found =
        ae::autoreseat::episode(&events(&rig), &rig.session, SLOT, "scout").expect("episode");
    assert!(
        found.terminal.is_none(),
        "hold consumes no terminal limit round"
    );
    let clear = ae::autoreseat::Pane {
        frame: ae::autoreseat::Frame::Clear,
        human_prompt: false,
        client_input: None,
    };
    assert_eq!(
        ae::autoreseat::decide(Some(&found), 0, &clear, Timestamp::now().epoch()),
        ae::autoreseat::Decision::Rest
    );
    assert_eq!(rig.tool_pid(&pane, "claude"), pid);
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-source");
    assert!(booked(&rig, DONE_ACTION).is_empty());
}

#[test]
fn choice_published_after_booking_holds_on_the_lifecycle_guarded_path() {
    let (rig, pane, key) = setup("aclate", false, false);
    let held = ae::store::lock(
        &rig.scratch
            .join("sessions")
            .join(format!(".lifecycle.{}.lock", rig.session)),
        Duration::ZERO,
    )
    .expect("test owns lifecycle barrier");
    let leg_lock = ae::autoreseat::lock_path(&rig.dir, SLOT);
    journal(&rig, ATTEMPT_ACTION, key, true);
    let child = core(&rig)
        .arg("_auto-reseat")
        .arg(&rig.dir)
        .args([SLOT, &key.to_string()])
        .spawn()
        .expect("real booked leg");
    // Settling assumption, not lifecycle-arrival proof: the auto lock precedes
    // all leg reads. A single probe avoids the old rapid acquiring poll.
    // WouldBlock proves ownership only; final placement needs source review.
    std::thread::sleep(Duration::from_secs(2));
    let probe = ae::store::lock(&leg_lock, Duration::from_millis(200));
    assert!(
        matches!(&probe, Err(why) if why.kind() == std::io::ErrorKind::WouldBlock),
        "seam precondition: leg must already own its auto lock: {probe:?}"
    );
    std::thread::sleep(Duration::from_secs(1));
    observation(&rig, "Opus 5", true);
    let before = rig.meta();
    let pid = rig.tool_pid(&pane, "claude");
    let launches = rig.launched();
    drop(held);
    let out = child
        .wait_with_output()
        .expect("leg exits after barrier release");
    code(&out, 1);
    assert_eq!(rig.meta(), before, "latest persisted pair preserved");
    assert_eq!(rig.tool_pid(&pane, "claude"), pid);
    assert_eq!(rig.launched(), launches);
    assert_eq!(booked(&rig, HELD_ACTION).len(), 1, "{}", rig.events());
    assert!(
        booked(&rig, HELD_ACTION)[0]
            .summary
            .as_deref()
            .is_some_and(|s| s.starts_with("observed-choice: "))
    );
    assert!(booked(&rig, "reseat").is_empty());
    assert!(!rig.scratch.join("home-b/projects").exists());
}

#[test]
fn daemon_names_the_hold_even_without_a_family_and_never_forecasts_a_forbidden_move() {
    for (no_family, tag) in [(false, "acforecast"), (true, "acnofamily")] {
        let (rig, pane, _) = setup(tag, true, false);
        if no_family {
            let path = rig.scratch.join("config");
            let text = std::fs::read_to_string(&path).expect("profiles");
            std::fs::write(
                &path,
                text.lines()
                    .filter(|s| !s.starts_with("fake-target ="))
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
            .expect("source-only family");
        }
        observation(&rig, "Opus 5", true);
        std::fs::write(rig.dir.join("events.jsonl"), "").expect("real first sight");
        let attach = format!(
            "env -u TMUX tmux -S '{}' attach-session -t '={}'",
            rig.sock.display(),
            rig.session
        );
        assert!(
            rig.tmux(&[
                "new-session",
                "-d",
                "-s",
                "viewer",
                "-x",
                "200",
                "-y",
                "40",
                &attach
            ])
            .0
        );
        until(&rig, || {
            !rig.tmux(&[
                "list-clients",
                "-t",
                &format!("={}", rig.session),
                "-F",
                "#{client_tty}",
            ])
            .1
            .trim()
            .is_empty()
        });
        let pid = rig.tool_pid(&pane, "claude");
        let _daemon = super::reseat_headroom_spec::watch(&rig);
        until(&rig, || {
            [HELD_ACTION, REFUSED_ACTION, DONE_ACTION]
                .iter()
                .any(|a| !booked(&rig, a).is_empty())
        });
        super::reseat_headroom_spec::sweeps(&rig);
        let messages = rig.tmux(&["show-messages"]).1;
        let notices: Vec<_> = messages
            .lines()
            .filter(|s| {
                s.contains(" message: [ae watchdog] ") && s.contains("hit its vendor usage limit")
            })
            .collect();
        assert!(!notices.is_empty(), "a real first-sight notice: {messages}");
        assert!(
            notices
                .iter()
                .all(|s| !s.contains("ae will move") && !s.contains("ae cannot move")),
            "{notices:?}"
        );
        assert!(
            booked(&rig, HELD_ACTION).iter().any(|e| e
                .summary
                .as_deref()
                .is_some_and(|s| s.starts_with("observed-choice: "))),
            "{}",
            rig.events()
        );
        assert!(booked(&rig, REFUSED_ACTION).is_empty());
        assert!(booked(&rig, DONE_ACTION).is_empty());
        assert_eq!(rig.tool_pid(&pane, "claude"), pid);
        assert_eq!(rig.meta_row("profile.spawned.0"), "fake-source");
    }
}
