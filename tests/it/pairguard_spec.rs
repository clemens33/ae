//! Independent acceptance for #305 part 2, brief + lead R1-R7 as oracle.
//! Authored before implementation; only the existing CLI and private Rig are driven.
//! Candidate/Move literals are deliberately absent: no new API is needed for base RED.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures inspect only their existing private Rig stores and server"
)]

use std::fmt::Write as _;
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use ae::autoreseat::{ATTEMPT_ACTION, DONE_ACTION, HELD_ACTION, REFUSED_ACTION};
use ae::events::Event;
use ae::time::Timestamp;

use super::seat_relaunch::Rig;

const SLOT: &str = "spawned.0";
const NAME: &str = "slice-nav";

fn row(rig: &Rig, key: &str, value: &str) {
    let before = rig.meta();
    let prefix = format!("{key}=");
    let mut after = String::new();
    let mut seen = false;
    for line in before.lines() {
        if line.starts_with(&prefix) {
            assert!(!seen, "fixture never rewrites duplicate rows");
            seen = true;
            writeln!(after, "{key}={value}").expect("string");
        } else {
            writeln!(after, "{line}").expect("string");
        }
    }
    if !seen {
        writeln!(after, "{key}={value}").expect("string");
    }
    std::fs::write(rig.dir.join("meta"), after).expect("private meta");
}

fn command(rig: &Rig, tool: &str, account: &str, extra: &str) -> String {
    let account = if tool == "claude" {
        format!("CLAUDE_CONFIG_DIR={} ", rig.scratch.join(account).display())
    } else {
        String::new()
    };
    format!(
        "{account}{} {} {extra}",
        rig.scratch.join("tools").join(tool).display(),
        rig.scratch.join(format!("{tool}.pl")).display()
    )
}

fn configure(rig: &Rig, profiles: &[(&str, String)], candidates: &str, knobs: &str) {
    let path = rig.scratch.join("config");
    let base = std::fs::read_to_string(&path).expect("Rig config");
    let mut text = base
        .split("[workspace]")
        .next()
        .expect("profiles")
        .to_owned();
    for (name, command) in profiles {
        writeln!(text, "{name} = \"{command}\"").expect("string");
    }
    writeln!(
        text,
        "[workspace]\nmain = lead\nlayout = vertical\nauto_reseat = all\nauto_reseat_grace_secs = 0\n{knobs}\n[auto_reseat]\nsource = {candidates}"
    )
    .expect("string");
    std::fs::write(path, text).expect("private config");
}

fn profiles(rig: &Rig, peer_tool: &str, first_tool: &str) -> Vec<(&'static str, String)> {
    vec![
        (
            "source",
            command(rig, "claude", "home-source", "--pg source"),
        ),
        ("peer", command(rig, peer_tool, "home-peer", "--pg peer")),
        (
            "first",
            command(rig, first_tool, "home-first", "--pg first"),
        ),
        ("good", command(rig, "claude", "home-good", "--pg good")),
    ]
}

fn seat(rig: &Rig, slot: &str, name: &str, profile: &str, tool: &str, running: bool) -> String {
    for (key, value) in [("seat", name), ("profile", profile), ("agent_bin", tool)] {
        row(rig, &format!("{key}.{slot}"), value);
    }
    let pane = rig.new_pane(slot, name);
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
            &format!("HOME={}", rig.scratch.join("home").display()),
            &format!("AE_HOME={}", rig.scratch.display()),
            &format!("CONFIG_FILE={}", rig.scratch.join("config").display()),
            "/bin/sh",
        ])
        .0,
        "scratch shell environment before any harness starts"
    );
    if running {
        rig.start(&pane, slot, tool);
    }
    pane
}

fn peer(rig: &Rig, tool: &str) {
    seat(rig, "spawned.1", "slice", "peer", tool, false);
}

fn pressure(rig: &Rig, account: &str, used: f64) {
    let now = Timestamp::now();
    let dir = rig.scratch.join(account);
    std::fs::create_dir_all(&dir).expect("private quota dir");
    std::fs::write(
        dir.join(".claude.json"),
        format!(
            "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"weekly_all\",\"group\":\"weekly_all\",\"percent\":{used},\"resets_at\":\"{}\",\"scope\":null}}]}}}}}}",
            now.epoch() * 1000,
            Timestamp::from_epoch(now.epoch() + 86400)
        ),
    )
    .expect("private quota reading");
}

fn append(
    rig: &Rig,
    slot: &str,
    name: &str,
    action: &str,
    ts: Timestamp,
    reference: &str,
    summary: &str,
) {
    let mut text = rig.events();
    let reference = if reference.is_empty() {
        String::new()
    } else {
        format!(",\"ref\":\"{reference}\"")
    };
    let summary = if summary.is_empty() {
        String::new()
    } else {
        format!(",\"summary\":\"{summary}\"")
    };
    writeln!(
        text,
        "{{\"ts\":\"{ts}\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"{name}\",\"target_slot\":\"{slot}\",\"target_session\":\"{}\"{reference}{summary}}}",
        rig.session
    ).expect("string");
    std::fs::write(rig.dir.join("events.jsonl"), text).expect("private journal");
}

fn episode(rig: &Rig, slot: &str, name: &str, headroom: bool, attempt: bool) -> Timestamp {
    let key = Timestamp::from_epoch(Timestamp::now().epoch() - 60);
    append(
        rig,
        slot,
        name,
        if headroom {
            "auto-reseat-headroom"
        } else {
            "limit"
        },
        key,
        "",
        if headroom {
            "headroom 95% weekly_all 7d"
        } else {
            ""
        },
    );
    if attempt {
        append(
            rig,
            slot,
            name,
            ATTEMPT_ACTION,
            Timestamp::now(),
            &key.to_string(),
            "",
        );
    }
    key
}

fn core(rig: &Rig) -> super::cli::Runner {
    let mut cmd = super::cli::ae();
    cmd.env("HOME", rig.scratch.join("home"))
        .env("AE_HOME", &rig.scratch)
        .env("CONFIG_FILE", rig.scratch.join("config"))
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", &rig.main_pane)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("AE_SENDER_OVERRIDE");
    cmd
}

fn leg_command(rig: &Rig, slot: &str, key: Timestamp) -> super::cli::Runner {
    let mut cmd = core(rig);
    cmd.arg("_auto-reseat")
        .arg(&rig.dir)
        .args([slot, &key.to_string()]);
    cmd
}

fn leg(rig: &Rig, slot: &str, key: Timestamp) -> Output {
    leg_command(rig, slot, key)
        .output()
        .expect("real leg entry")
}

// Reject a published wrong choice immediately; a fake adapter that has no
// composer must not turn a wrong-choice assertion into a readiness timeout.
fn moved(rig: &Rig, slot: &str, key: Timestamp, expected: &str) -> Output {
    let mut child = leg_command(rig, slot, key)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("real leg child");
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let published = rig.meta_row(&format!("profile.{slot}"));
        if published != "source" {
            assert_eq!(
                published,
                expected,
                "published destination: {}",
                rig.events()
            );
            break;
        }
        if child.try_wait().expect("child status").is_some() {
            break;
        }
        assert!(
            Instant::now() < until,
            "destination not published: {}",
            rig.events()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    let output = child.wait_with_output().expect("leg ending");
    assert_eq!(
        rig.meta_row(&format!("profile.{slot}")),
        expected,
        "{}",
        rig.events()
    );
    output
}

fn records(rig: &Rig) -> Vec<Event> {
    rig.events()
        .lines()
        .map(|line| Event::parse_line(line).expect("valid event"))
        .collect()
}

fn of(rig: &Rig, action: &str) -> Vec<Event> {
    records(rig)
        .into_iter()
        .filter(|e| e.action == action)
        .collect()
}

fn chats(rig: &Rig) -> Vec<String> {
    of(rig, "chat")
        .into_iter()
        .filter_map(|e| e.summary)
        .collect()
}

fn pair_refusal(rig: &Rig, before: &str, seat_name: &str, peer_name: &str, unproven: bool) {
    assert_eq!(rig.meta(), before, "a held pair publishes no seat change");
    assert!(of(rig, DONE_ACTION).is_empty());
    assert!(
        of(rig, "reseat").is_empty(),
        "hold must precede any stop or move record"
    );
    let refusal = of(rig, REFUSED_ACTION);
    assert_eq!(refusal.len(), 1, "{}", rig.events());
    let why = refusal[0].summary.as_deref().expect("refusal reason");
    assert!(why.starts_with("pair-provider:"), "{why}");
    let said = chats(rig);
    assert_eq!(said.len(), 1, "one told/chat line per outcome: {said:?}");
    assert!(
        said[0].contains(seat_name) && said[0].contains(peer_name),
        "{said:?}"
    );
    assert!(
        said[0].contains("Anthropic"),
        "seat provider must be named: {said:?}"
    );
    if unproven {
        assert!(
            said[0].contains("unproven") || said[0].contains("ambiguous"),
            "{said:?}"
        );
    }
    assert!(said[0].chars().count() <= ae::autoreseat::NOTICE_CHARS);
}

#[test]
fn headroom_skips_the_partners_provider_for_a_usable_diverse_destination() {
    let rig = Rig::new("pgprefer");
    configure(&rig, &profiles(&rig, "codex", "codex"), "first, good", "");
    pressure(&rig, "home-source", 95.0);
    seat(&rig, SLOT, NAME, "source", "claude", true);
    peer(&rig, "codex");
    let key = episode(&rig, SLOT, NAME, true, true);
    let result = moved(&rig, SLOT, key, "good");
    assert_eq!(
        rig.meta_row("profile.spawned.0"),
        "good",
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(of(&rig, DONE_ACTION).len(), 1, "{}", rig.events());
}

#[test]
fn headroom_holds_the_running_seat_and_names_both_providers() {
    let rig = Rig::new("pghold");
    configure(&rig, &profiles(&rig, "codex", "codex"), "first", "");
    pressure(&rig, "home-source", 95.0);
    let pane = seat(&rig, SLOT, NAME, "source", "claude", true);
    peer(&rig, "codex");
    let key = episode(&rig, SLOT, NAME, true, true);
    let before = rig.meta();
    let pid = rig.tool_pid(&pane, "claude").expect("running source");
    leg(&rig, SLOT, key);
    pair_refusal(&rig, &before, NAME, "slice", false);
    assert_eq!(
        rig.tool_pid(&pane, "claude"),
        Some(pid),
        "same running process survives hold"
    );
}

#[test]
fn drawn_vendor_limit_does_not_waive_pair_diversity() {
    let rig = Rig::new("pglimit");
    configure(&rig, &profiles(&rig, "codex", "codex"), "first", "");
    let pane = seat(&rig, SLOT, NAME, "source", "claude", true);
    peer(&rig, "codex");
    rig.mark_limited(&pane);
    let key = episode(&rig, SLOT, NAME, false, true);
    let before = rig.meta();
    leg(&rig, SLOT, key);
    pair_refusal(&rig, &before, NAME, "slice", false);
    let prior = rig.events();
    leg(&rig, SLOT, key);
    assert_eq!(
        rig.events(),
        prior,
        "closed attempt cannot retell or reclose"
    );
    // Backdate the REAL completed round, preserving ordering and reference identity.
    let mut old = String::new();
    for line in prior.lines() {
        let event = Event::parse_line(line).expect("first-round event");
        writeln!(
            old,
            "{}",
            line.replace(
                &format!("\"ts\":\"{}\"", event.ts),
                &format!(
                    "\"ts\":\"{}\"",
                    Timestamp::from_epoch(event.ts.epoch() - 2000)
                ),
            )
        )
        .expect("string");
    }
    old = old.replace(
        &key.to_string(),
        &Timestamp::from_epoch(key.epoch() - 2000).to_string(),
    );
    std::fs::write(rig.dir.join("events.jsonl"), old).expect("prior completed round");
    core(&rig)
        .args([&format!("@{}", rig.session), "send", NAME, "auto reseat"])
        .env("AE_SENDER_OVERRIDE", "watchdog")
        .env("_AE_EVENT_ACTION", ATTEMPT_ACTION)
        .output()
        .expect("retry trigger after the cooldown");
    assert_eq!(
        of(&rig, REFUSED_ACTION).len(),
        2,
        "one later refusal round: {}",
        rig.events()
    );
    assert_eq!(
        chats(&rig).len(),
        1,
        "same key and reason must not be told twice: {}",
        rig.events()
    );
}

#[test]
fn urgent_exhausted_resume_does_not_waive_pair_diversity() {
    let rig = Rig::new("pgurgent");
    configure(
        &rig,
        &profiles(&rig, "codex", "codex"),
        "first",
        "auto_reseat_at = off",
    );
    pressure(&rig, "home-source", 100.0);
    seat(&rig, SLOT, NAME, "source", "claude", true);
    peer(&rig, "codex");
    let key = episode(&rig, SLOT, NAME, true, true);
    row(&rig, "started", &(key.epoch() - 1).to_string());
    row(&rig, "launch_settled", &(key.epoch() - 1).to_string());
    let before = rig.meta();
    leg(&rig, SLOT, key);
    pair_refusal(&rig, &before, NAME, "slice", false);
}

#[test]
fn mutable_unknown_or_adapter_disagreeing_partners_never_prove_diversity() {
    for (tag, profile_tool, recorded_tool) in [
        ("pgoc", "opencode", "opencode"),
        ("pgagy", "agy", "agy"),
        ("pgdrift", "codex", "claude"),
    ] {
        let rig = Rig::new(tag);
        let mut entries = profiles(&rig, profile_tool, "claude");
        if profile_tool == "agy" {
            entries[1].1.push_str(" --model gemini-4-argon");
        }
        configure(&rig, &entries, "first", "");
        seat(&rig, SLOT, NAME, "source", "claude", false);
        peer(&rig, profile_tool);
        row(&rig, "agent_bin.spawned.1", recorded_tool);
        let key = episode(&rig, SLOT, NAME, false, true);
        let before = rig.meta();
        leg(&rig, SLOT, key);
        pair_refusal(&rig, &before, NAME, "slice", true);
    }
}

#[test]
fn an_unknown_destination_is_skipped_even_before_a_declared_known_destination() {
    for (tag, unknown) in [("pgdestoc", "opencode"), ("pgdestagy", "agy")] {
        let rig = Rig::new(tag);
        configure(&rig, &profiles(&rig, "codex", unknown), "first, good", "");
        seat(&rig, SLOT, NAME, "source", "claude", false);
        peer(&rig, "codex");
        let key = episode(&rig, SLOT, NAME, false, true);
        moved(&rig, SLOT, key, "good");
        assert_eq!(
            rig.meta_row("profile.spawned.0"),
            "good",
            "{}",
            rig.events()
        );
    }
}

#[test]
fn all_three_members_of_a_suffix_chain_hold_as_ambiguous() {
    for moving in 0..3 {
        let rig = Rig::new(&format!("pgchain{moving}"));
        configure(&rig, &profiles(&rig, "codex", "claude"), "first", "");
        let names = ["x", "x-nav", "x-nav-nav"];
        for (index, name) in names.iter().enumerate() {
            seat(
                &rig,
                &format!("spawned.{index}"),
                name,
                "source",
                "claude",
                false,
            );
        }
        let slot = format!("spawned.{moving}");
        let key = episode(&rig, &slot, names[moving], false, true);
        let before = rig.meta();
        leg(&rig, &slot, key);
        pair_refusal(&rig, &before, names[moving], "ambiguous", true);
    }
}

#[test]
fn lead_pair_slots_override_a_name_that_would_suggest_another_partner() {
    let rig = Rig::new("pglead");
    configure(&rig, &profiles(&rig, "codex", "codex"), "first", "");
    row(&rig, "layout", "lead-pair");
    row(&rig, "profile.main", "peer");
    row(&rig, "agent_bin.main", "codex");
    seat(&rig, "worker.0", NAME, "source", "claude", false);
    seat(&rig, "spawned.1", "slice", "fake-codex", "codex", false);
    let key = episode(&rig, "worker.0", NAME, false, true);
    let before = rig.meta();
    leg(&rig, "worker.0", key);
    pair_refusal(&rig, &before, NAME, "lead", false);
}

#[test]
fn current_partner_profile_and_recorded_client_control_the_guard() {
    for (tag, override_client) in [("pgcurrent", false), ("pgclient", true)] {
        let rig = Rig::new(tag);
        configure(&rig, &profiles(&rig, "claude", "codex"), "first", "");
        seat(&rig, SLOT, NAME, "source", "claude", false);
        peer(&rig, "claude");
        if override_client {
            let path = rig.scratch.join("config");
            let mut text = std::fs::read_to_string(&path).expect("config");
            writeln!(
                text,
                "[clients]\nother = \"{}\"",
                rig.scratch.join("tools/codex").display()
            )
            .expect("string");
            std::fs::write(path, text).expect("client override");
            row(&rig, "client.spawned.1", "other");
        } else {
            row(&rig, "profile.spawned.1", "first");
        }
        row(&rig, "agent_bin.spawned.1", "codex");
        let key = episode(&rig, SLOT, NAME, false, true);
        let before = rig.meta();
        leg(&rig, SLOT, key);
        pair_refusal(&rig, &before, NAME, "slice", false);
    }
}

fn barrier(rig: &Rig, key: &str) -> std::path::PathBuf {
    let dir = rig.scratch.join("read-gate");
    std::fs::create_dir_all(&dir).expect("private barrier");
    std::fs::write(dir.join(key), "").expect("arm existing read door");
    dir
}

fn wait_barrier(rig: &Rig, dir: &std::path::Path, key: &str, child: &mut super::cli::OwnedChild) {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        if std::fs::read_to_string(dir.join("trace"))
            .unwrap_or_default()
            .lines()
            .any(|line| line == format!("held {key}"))
        {
            return;
        }
        assert!(
            child.try_wait().expect("child status").is_none(),
            "required deterministic barrier was never reached: {}",
            rig.events()
        );
        assert!(
            Instant::now() < until,
            "barrier {key} not reached: {}",
            rig.events()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn partner_change_after_choice_is_refused_before_any_running_tool_is_stopped() {
    let rig = Rig::new("pgrace");
    // R7 lets this initially collapsed pair choose a diverse destination.
    // The peer follows onto that destination before the lifecycle proof.
    configure(&rig, &profiles(&rig, "claude", "codex"), "first", "");
    let pane = seat(&rig, SLOT, NAME, "source", "claude", true);
    peer(&rig, "claude");
    let key = episode(&rig, SLOT, NAME, false, true);
    let gate = barrier(&rig, "@auto-reseat-pair");
    let mut child = leg_command(&rig, SLOT, key)
        .env("AE_TEST_APP_READ_GATE", &gate)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("real leg child");
    wait_barrier(&rig, &gate, "@auto-reseat-pair", &mut child);
    let pid = rig.tool_pid(&pane, "claude").expect("running old source");
    row(&rig, "profile.spawned.1", "first");
    row(&rig, "agent_bin.spawned.1", "codex");
    let before = rig.meta();
    std::fs::remove_file(gate.join("@auto-reseat-pair")).expect("release choice barrier");
    child.wait_with_output().expect("leg ending");
    assert_eq!(rig.meta(), before);
    assert_eq!(rig.tool_pid(&pane, "claude"), Some(pid));
    assert!(of(&rig, "reseat").is_empty());
    assert!(of(&rig, DONE_ACTION).is_empty());
    assert!(
        of(&rig, HELD_ACTION).iter().any(|e| e
            .summary
            .as_deref()
            .is_some_and(|s| s.starts_with("pair-changed:"))),
        "{}",
        rig.events()
    );
}

#[test]
fn a_post_move_provider_collapse_is_named_once_on_the_human_channel() {
    let rig = Rig::new("pgpost");
    configure(&rig, &profiles(&rig, "codex", "claude"), "first", "");
    seat(&rig, SLOT, NAME, "source", "claude", false);
    peer(&rig, "codex");
    let key = episode(&rig, SLOT, NAME, false, true);
    let gate = barrier(&rig, "@auto-reseat-after");
    let mut child = leg_command(&rig, SLOT, key)
        .env("AE_TEST_APP_READ_GATE", &gate)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("real leg child");
    wait_barrier(&rig, &gate, "@auto-reseat-after", &mut child);
    assert_eq!(
        rig.meta_row("profile.spawned.0"),
        "first",
        "move must already be published"
    );
    row(&rig, "profile.spawned.1", "good");
    row(&rig, "agent_bin.spawned.1", "claude");
    std::fs::remove_file(gate.join("@auto-reseat-after")).expect("release post-move barrier");
    child.wait_with_output().expect("leg ending");
    let said: Vec<_> = chats(&rig)
        .into_iter()
        .filter(|s| s.starts_with("auto reseat: pair-provider collapse:"))
        .collect();
    assert_eq!(said.len(), 1, "{}", rig.events());
    assert!(
        said[0].contains(NAME) && said[0].contains("slice") && said[0].contains("Anthropic"),
        "{said:?}"
    );
    assert!(said[0].chars().count() <= ae::autoreseat::NOTICE_CHARS);
}

#[test]
fn trigger_observes_the_partners_in_flight_attempt_without_writing_another() {
    let rig = Rig::new("pgflight");
    configure(&rig, &profiles(&rig, "codex", "claude"), "first", "");
    let pane = seat(&rig, SLOT, NAME, "source", "claude", true);
    peer(&rig, "codex");
    rig.mark_limited(&pane);
    episode(&rig, SLOT, NAME, false, false);
    episode(&rig, "spawned.1", "slice", false, true);
    let before = rig.events();
    core(&rig)
        .args([&format!("@{}", rig.session), "send", NAME, "auto reseat"])
        .env("AE_SENDER_OVERRIDE", "watchdog")
        .env("_AE_EVENT_ACTION", ATTEMPT_ACTION)
        .output()
        .expect("real trigger");
    assert_eq!(
        rig.events(),
        before,
        "partner in flight declines before journaling"
    );
}

#[test]
fn unpaired_refusal_retains_exact_base_journal_and_notice_bytes() {
    let rig = Rig::new("pgbase");
    configure(
        &rig,
        &profiles(&rig, "codex", "claude"),
        "missing, first",
        "",
    );
    pressure(&rig, "home-first", 100.0);
    seat(&rig, SLOT, "solo", "source", "claude", false);
    let key = episode(&rig, SLOT, "solo", false, true);
    leg(&rig, SLOT, key);
    let result = of(&rig, REFUSED_ACTION);
    assert_eq!(result.len(), 1, "{}", rig.events());
    assert_eq!(
        result[0].reference.as_deref(),
        Some(key.to_string().as_str())
    );
    assert_eq!(
        result[0].summary.as_deref(),
        Some(
            "no-candidate: refused: no usable candidate: missing (not configured here), first (exhausted)"
        )
    );
    assert_eq!(
        chats(&rig),
        vec![format!(
            "auto reseat: no-candidate: solo not moved — refused: no usable candidate: missing (not configured here), first (exhausted). Next: move it by hand (ae reseat {} solo --using <profile>) or wait for the reset.",
            rig.session
        )]
    );
    // Base-captured wire bytes: normalize only nondeterministic time and session
    // identity. Keep field order, schema, routing, punctuation and notice bytes.
    let mut wire = String::new();
    for line in rig.events().lines() {
        let event = Event::parse_line(line).expect("base journal event");
        if event.action == REFUSED_ACTION || event.action == "chat" {
            let line = line
                .replace(&format!("\"ts\":\"{}\"", event.ts), "\"ts\":\"$TIME\"")
                .replace(&key.to_string(), "$KEY")
                .replace(&rig.session, "$SESSION");
            writeln!(wire, "{line}").expect("string");
        }
    }
    assert_eq!(wire, include_str!("pairguard-unpaired.golden"));
}

#[test]
fn unpaired_declared_tiers_ties_and_exclusions_keep_base_expected_picks() {
    for (tag, first, second, expected) in [
        ("pgbasefresh", Some(20.0), None, "first"),
        ("pgbasebetter", Some(95.0), Some(20.0), "good"),
        ("pgbasetie", Some(20.0), Some(20.0), "first"),
        ("pgbasespent", Some(100.0), Some(20.0), "good"),
    ] {
        let rig = Rig::new(tag);
        configure(
            &rig,
            &profiles(&rig, "codex", "claude"),
            "missing, first, good",
            "",
        );
        if let Some(used) = first {
            pressure(&rig, "home-first", used);
        }
        if let Some(used) = second {
            pressure(&rig, "home-good", used);
        }
        seat(&rig, SLOT, "solo", "source", "claude", false);
        let key = episode(&rig, SLOT, "solo", false, true);
        leg(&rig, SLOT, key);
        assert_eq!(
            rig.meta_row("profile.spawned.0"),
            expected,
            "{tag}: {}",
            rig.events()
        );
        let done = of(&rig, DONE_ACTION);
        assert_eq!(done.len(), 1, "{}", rig.events());
        assert_eq!(done[0].reference.as_deref(), Some("source"));
        assert_eq!(
            done[0].summary.as_deref(),
            Some(format!("to {expected}, seeded").as_str())
        );
    }
}

#[test]
fn an_unrelated_duplicate_name_does_not_turn_a_solo_seat_into_a_pair() {
    let rig = Rig::new("pgdupneutral");
    configure(&rig, &profiles(&rig, "codex", "claude"), "first", "");
    seat(&rig, SLOT, "solo", "source", "claude", false);
    seat(&rig, "spawned.1", "other", "peer", "codex", false);
    seat(&rig, "spawned.2", "other", "peer", "codex", false);
    let key = episode(&rig, SLOT, "solo", false, true);
    leg(&rig, SLOT, key);
    assert_eq!(
        rig.meta_row("profile.spawned.0"),
        "first",
        "{}",
        rig.events()
    );
}

#[test]
fn manual_reseat_remains_available_when_it_collapses_a_pair() {
    let rig = Rig::new("pgmanual");
    configure(&rig, &profiles(&rig, "claude", "claude"), "first", "");
    seat(&rig, SLOT, NAME, "source", "claude", false);
    peer(&rig, "claude");
    let out = core(&rig)
        .args(["reseat", &rig.session, NAME, "--using", "first"])
        .output()
        .expect("manual reseat");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(rig.meta_row("profile.spawned.0"), "first");
    assert!(!String::from_utf8_lossy(&out.stderr).contains("pair-provider"));
}

#[test]
fn paired_quota_only_failures_keep_the_existing_no_candidate_class() {
    let rig = Rig::new("pgquotaonly");
    configure(&rig, &profiles(&rig, "codex", "claude"), "first, good", "");
    pressure(&rig, "home-first", 100.0);
    pressure(&rig, "home-good", 100.0);
    seat(&rig, SLOT, NAME, "source", "claude", false);
    peer(&rig, "codex");
    let key = episode(&rig, SLOT, NAME, false, true);
    leg(&rig, SLOT, key);
    assert_eq!(
        of(&rig, REFUSED_ACTION)[0].summary.as_deref(),
        Some("no-candidate: refused: no usable candidate: first (exhausted), good (exhausted)")
    );
    assert!(of(&rig, DONE_ACTION).is_empty());
}

#[test]
fn diverse_candidates_still_obey_history_and_peer_account_latches() {
    for (tag, history) in [
        ("pghistory", "limit"),
        ("pgheadhistory", "headroom"),
        ("pglatched", "latched"),
    ] {
        let rig = Rig::new(tag);
        configure(&rig, &profiles(&rig, "codex", "claude"), "first, good", "");
        seat(&rig, SLOT, NAME, "source", "claude", false);
        peer(&rig, "codex");
        if history == "latched" {
            seat(&rig, "spawned.2", "blocked", "first", "claude", true);
            episode(&rig, "spawned.2", "blocked", false, false);
        } else {
            if history == "headroom" {
                let prior = Timestamp::from_epoch(Timestamp::now().epoch() - 150);
                append(
                    &rig,
                    SLOT,
                    NAME,
                    "auto-reseat-headroom",
                    prior,
                    "",
                    "headroom 95% weekly_all 7d",
                );
                append(
                    &rig,
                    SLOT,
                    NAME,
                    ATTEMPT_ACTION,
                    Timestamp::from_epoch(prior.epoch() + 1),
                    &prior.to_string(),
                    "",
                );
            }
            append(
                &rig,
                SLOT,
                NAME,
                DONE_ACTION,
                Timestamp::from_epoch(Timestamp::now().epoch() - 120),
                "first",
                "to source, seeded",
            );
        }
        let key = episode(&rig, SLOT, NAME, false, true);
        leg(&rig, SLOT, key);
        assert_eq!(
            rig.meta_row("profile.spawned.0"),
            "good",
            "{tag}: {}",
            rig.events()
        );
    }
}

#[test]
fn headroom_cannot_resurrect_a_diverse_candidate_without_headroom() {
    let rig = Rig::new("pgnoroom");
    configure(&rig, &profiles(&rig, "codex", "claude"), "first, good", "");
    pressure(&rig, "home-source", 95.0);
    pressure(&rig, "home-first", 96.0);
    seat(&rig, SLOT, NAME, "source", "claude", true);
    peer(&rig, "codex");
    let key = episode(&rig, SLOT, NAME, true, true);
    leg(&rig, SLOT, key);
    assert_eq!(
        rig.meta_row("profile.spawned.0"),
        "good",
        "{}",
        rig.events()
    );
}

#[test]
fn base_declared_priority_and_derived_provenance_remain_explicit_contracts() {
    for (tag, declared, expected, provenance) in [
        ("pgdeclared", "first", "first", ""),
        ("pgderived", "missing", "twin", ", derived twin"),
    ] {
        let rig = Rig::new(tag);
        let entries = vec![
            (
                "source",
                command(&rig, "claude", "home-source", "--pg family --model opus"),
            ),
            (
                "first",
                command(&rig, "claude", "home-first", "--pg declared --model opus"),
            ),
            (
                "twin",
                command(&rig, "claude", "home-twin", "--pg family --model opus"),
            ),
        ];
        configure(&rig, &entries, declared, "");
        pressure(&rig, "home-first", 95.0);
        pressure(&rig, "home-twin", 10.0);
        seat(&rig, SLOT, "solo", "source", "claude", true);
        row(&rig, "harness_session.spawned.0", "pending");
        let key = episode(&rig, SLOT, "solo", false, true);
        leg(&rig, SLOT, key);
        assert_eq!(
            rig.meta_row("profile.spawned.0"),
            expected,
            "{}",
            rig.events()
        );
        let done = of(&rig, DONE_ACTION);
        assert_eq!(done.len(), 1, "{}", rig.events());
        assert_eq!(
            done[0].summary.as_deref(),
            Some(format!("to {expected}{provenance}, seeded").as_str())
        );
    }
}

#[test]
fn a_diverse_derived_candidate_beats_a_collapsing_declared_candidate() {
    let rig = Rig::new("pgderivedpair");
    let entries = vec![
        (
            "source",
            command(&rig, "claude", "home-source", "--pg family --model opus"),
        ),
        ("first", command(&rig, "codex", "", "--pg declared")),
        (
            "twin",
            command(&rig, "claude", "home-twin", "--pg family --model opus"),
        ),
        ("peer", command(&rig, "codex", "", "--pg peer")),
    ];
    configure(&rig, &entries, "first", "");
    seat(&rig, SLOT, NAME, "source", "claude", true);
    row(&rig, "harness_session.spawned.0", "pending");
    peer(&rig, "codex");
    let key = episode(&rig, SLOT, NAME, false, true);
    moved(&rig, SLOT, key, "twin");
    assert_eq!(
        rig.meta_row("profile.spawned.0"),
        "twin",
        "{}",
        rig.events()
    );
    assert_eq!(
        of(&rig, DONE_ACTION)[0].summary.as_deref(),
        Some("to twin, derived twin, seeded")
    );
}

#[test]
fn long_pair_names_keep_the_class_both_identities_and_the_notice_bound() {
    let rig = Rig::new("pglong");
    configure(&rig, &profiles(&rig, "codex", "codex"), "first", "");
    let driver = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";
    let nav = format!("{driver}-nav");
    seat(&rig, SLOT, &nav, "source", "claude", false);
    seat(&rig, "spawned.1", driver, "peer", "codex", false);
    let key = episode(&rig, SLOT, &nav, false, true);
    leg(&rig, SLOT, key);
    let said = chats(&rig);
    assert_eq!(said.len(), 1, "{}", rig.events());
    assert!(
        said[0].starts_with("auto reseat: pair-provider:"),
        "{said:?}"
    );
    assert!(
        said[0].contains(driver) && said[0].contains(&nav),
        "{said:?}"
    );
    assert!(said[0].chars().count() <= ae::autoreseat::NOTICE_CHARS);
}

#[test]
fn an_already_collapsed_proven_pair_may_move_and_names_that_it_stays_collapsed() {
    let rig = Rig::new("pgstays");
    configure(&rig, &profiles(&rig, "claude", "claude"), "first", "");
    seat(&rig, SLOT, NAME, "source", "claude", false);
    peer(&rig, "claude");
    let key = episode(&rig, SLOT, NAME, false, true);
    moved(&rig, SLOT, key, "first");
    let done = of(&rig, DONE_ACTION);
    assert_eq!(done.len(), 1, "{}", rig.events());
    assert!(
        done[0]
            .summary
            .as_deref()
            .is_some_and(|why| why.contains("pair-provider: pair stays on Anthropic")),
        "{}",
        rig.events()
    );
    let said = chats(&rig);
    assert_eq!(said.len(), 1, "no extra newly-collapsed notice: {said:?}");
    assert!(
        said[0].contains("pair-provider: pair stays on Anthropic"),
        "{said:?}"
    );
    assert!(!said[0].contains("pair-provider collapse"), "{said:?}");
    assert!(said[0].chars().count() <= ae::autoreseat::NOTICE_CHARS);
}

#[test]
fn an_unproven_source_never_gets_the_already_collapsed_exemption() {
    let rig = Rig::new("pgunprovensource");
    let mut entries = profiles(&rig, "claude", "claude");
    entries[0].1 = command(&rig, "opencode", "", "--pg source");
    configure(&rig, &entries, "first", "");
    seat(&rig, SLOT, NAME, "source", "opencode", false);
    peer(&rig, "claude");
    let key = episode(&rig, SLOT, NAME, false, true);
    let before = rig.meta();
    leg(&rig, SLOT, key);
    pair_refusal(&rig, &before, NAME, "slice", true);
}
