//! Frozen phase A. Exercise the public reseat and real watchdog paths.

#![allow(
    clippy::expect_used,
    reason = "fixture setup failures name the missing fact"
)]

use super::{
    Cycles, DONE_ACTION, HELD_ACTION, KEY, REFUSED_ACTION, Rig, configure, daemon_err, leg,
    limited_on, limitstop_playback, rebind, records, remove_unused_profiles, watch_until,
};
use ae::time::Timestamp;
use std::fs;

const SINCE: i64 = 1_788_858_600;
const SINCE_ISO: &str = "2026-09-08T09:10:00Z";
const SCOPE: &str = "provider-github-copilot";
const REASON: &str = "Fixture vendor account suspended";
const FRAME: &str =
    include_str!("../../fixtures/vendorblock/opencode-copilot-quota-100x30.raw.txt");

/// Both profiles use the same client. Only the first names Copilot.
fn scoped_profiles(rig: &Rig) -> String {
    let opencode = rig.scratch.join("tools/opencode");
    let script = rig.scratch.join("opencode.pl");
    format!(
        "blocked-profile = \"ocfixture {} --agent build --model github-copilot/gpt-6.1-sol\"\n\
         fake-opencode = \"ocfixture {} --agent build --model opencode/big-pickle\"\n\
         [clients]\nocfixture = {}\n",
        script.display(),
        script.display(),
        opencode.display(),
    )
}

fn install_scoped_profiles(rig: &Rig) {
    let path = rig.scratch.join("config");
    let config = fs::read_to_string(&path).expect("fixture config");
    let config = config
        .lines()
        .filter(|line| !line.starts_with("fake-opencode ="))
        .collect::<Vec<_>>()
        .join("\n")
        .replace(
            "[workspace]",
            &format!("{}\n[workspace]", scoped_profiles(rig)),
        );
    fs::write(path, format!("{config}\n")).expect("two provider profiles");
}

fn block(rig: &Rig, provider: bool) {
    let dir = rig.scratch.join("vendorblock");
    fs::create_dir_all(&dir).expect("block directory");
    let (stem, row) = if provider {
        (SCOPE, "provider=github-copilot\n")
    } else {
        ("client-ocfixture", "")
    };
    fs::write(
        dir.join(format!("{stem}.rec")),
        format!(
            "version=1\ntool=opencode\nclient=ocfixture\n{row}since={SINCE}\n\
             kind=account\nseat=earlier\nreason={REASON}\n"
        ),
    )
    .expect("standing record");
}

#[test]
fn automatic_reseat_skips_a_blocked_provider_and_uses_another_provider_of_its_client() {
    let (rig, _) = limited_on("q2skip", "claude-b", "on", "blocked-profile, fake-opencode");
    remove_unused_profiles(&rig, &["fake-claude", "fake-claude-a"]);
    install_scoped_profiles(&rig);
    // Match spawn: a provider-qualified destination does not use the
    // slash-free client scope. The provider block still skips Copilot.
    block(&rig, false);
    block(&rig, true);

    let (code, out, err) = leg(&rig, "spawned.0", KEY);

    assert_eq!(code, Some(0), "{out}{err}\n{}", rig.events());
    assert_eq!(
        rig.meta_row("profile.spawned.0"),
        "fake-opencode",
        "the provider with no block is still available"
    );
}

#[test]
fn automatic_reseat_holds_when_the_only_destination_has_a_blocked_client() {
    let (rig, pane) = limited_on("q2hold", "claude-b", "on", "blocked-profile");
    remove_unused_profiles(&rig, &["fake-claude", "fake-claude-a"]);
    install_scoped_profiles(&rig);
    let path = rig.scratch.join("config");
    let config = fs::read_to_string(&path).expect("fixture config");
    fs::write(
        path,
        config.replace("github-copilot/gpt-6.1-sol", "big-pickle"),
    )
    .expect("the destination uses the client scope");
    block(&rig, false);
    let original = rig.tool_pid(&pane, "claude").expect("original tool");

    let (code, out, err) = leg(&rig, "spawned.0", KEY);

    assert_ne!(code, Some(0), "{out}{err}\n{}", rig.events());
    assert_eq!(rig.tool_pid(&pane, "claude"), Some(original));
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude-b");
    let events = records(&rig);
    assert!(!events.iter().any(|event| event.action == DONE_ACTION));
    assert!(
        events.iter().any(|event| {
            [HELD_ACTION, REFUSED_ACTION].contains(&event.action.as_str())
                && event.summary.as_ref().is_some_and(|summary| {
                    ["client-ocfixture", SINCE_ISO, REASON]
                        .iter()
                        .all(|part| summary.contains(part))
                })
        }),
        "the hold names the scope, time and reason: {}",
        rig.events()
    );
}

#[test]
fn direct_reseat_refuses_a_blocked_destination_before_stopping_the_original() {
    let (rig, pane) = limited_on("q2direct", "claude-b", "on", "blocked-profile");
    install_scoped_profiles(&rig);
    block(&rig, true);
    let original = rig.tool_pid(&pane, "claude").expect("original tool");

    let (code, out, err) = rig.run_top(
        &rig.main_pane,
        &[
            "reseat",
            &rig.session,
            "scout",
            "--using",
            "blocked-profile",
        ],
    );

    assert_ne!(code, Some(0), "{out}{err}");
    for part in [SCOPE, SINCE_ISO, REASON] {
        assert!(err.contains(part), "missing {part:?} in {err}");
    }
    assert_eq!(rig.tool_pid(&pane, "claude"), Some(original));
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude-b");
}

fn notice_count(rig: &Rig, since: &str) -> usize {
    // Read the fake lead's actual input. The playback worker also records
    // its input here, but its limit latch prevents sends to it. Separate
    // recipient roots provide independent receipts.
    rig.received()
        .split(['\r', '\n'])
        .filter(|line| {
            [
                SCOPE,
                since,
                "You have exceeded your monthly quota",
                "ae quota clear provider-github-copilot",
            ]
            .iter()
            .all(|part| line.contains(part))
        })
        .count()
}

fn record_stamp(path: &std::path::Path) -> Option<String> {
    let record = fs::read_to_string(path).ok()?;
    let seconds = record
        .lines()
        .find_map(|line| line.strip_prefix("since="))?
        .parse()
        .ok()?;
    Some(Timestamp::from_epoch(seconds).to_string())
}

fn run_recipient(rig: &Rig, source: &Rig, stamp: &str, expected: usize) {
    let last = std::cell::Cell::new(super::beat(rig));
    let begun = std::cell::Cell::new(0);
    assert!(
        super::watch_until_in(rig, &source.scratch, None, || {
            let beat = super::beat(rig);
            if beat != last.get() && beat.is_some() {
                begun.set(begun.get() + 1);
                last.set(beat);
            }
            begun.get() >= 4
        }),
        "{}\n{}",
        rig.events(),
        daemon_err(rig)
    );
    assert_eq!(
        notice_count(rig, stamp),
        expected,
        "{} must receive {expected} notices: {:?}\n{}\n{}",
        rig.session,
        rig.received(),
        rig.events(),
        daemon_err(rig)
    );
}

fn recipient(source: &Rig, tag: &str, orchestrator: bool, eligible: bool) -> Rig {
    let mut rig = Rig::new(tag);
    // A second leader uses the same fleet state, with its own server,
    // journal and already-running lead. Each rig retains its own fake log.
    let old_dir = rig.dir.clone();
    if orchestrator {
        assert!(
            rig.tmux(&["rename-session", "-t", &rig.session, "orchestrator"])
                .0
        );
        "orchestrator".clone_into(&mut rig.session);
        assert!(
            rig.tmux(&[
                "set-option",
                "-p",
                "-t",
                &rig.main_pane,
                "@ae_agent",
                "orchestrator"
            ])
            .0
        );
    }
    let dir = source.scratch.join("sessions").join(&rig.session);
    fs::create_dir_all(&dir).expect("same fleet, another session");
    let meta = fs::read_to_string(old_dir.join("meta")).expect("recipient meta");
    let meta = if orchestrator {
        meta.replace("session=rlq2orch", "session=orchestrator")
            .replace("seat.main=lead", "seat.main=orchestrator")
            + "meta_agent=true\n"
    } else {
        meta
    };
    fs::write(dir.join("meta"), meta).expect("recipient meta in shared fleet");
    rig.dir = dir;
    install_scoped_profiles(&rig);
    if !eligible {
        let path = rig.scratch.join("config");
        let config = fs::read_to_string(&path).expect("recipient config");
        fs::write(
            path,
            config.replace("github-copilot/gpt-6.1-sol", "opencode/big-pickle"),
        )
        .expect("the same client has no Copilot launch in this session");
    }
    rig.start(&rig.main_pane, "main", "opencode");
    rig
}

#[test]
fn watchdog_notices_reach_each_eligible_lead_once_per_record_across_restarts() {
    let source = Rig::new("q2write");
    configure(&source, "off", "fake-opencode");
    // Keep the emitting worker's Copilot scope separate from the healthy
    // lead's profile while using the same fake external tool.
    install_scoped_profiles(&source);
    let peer = recipient(&source, "q2peer", false, true);
    let orchestrator = recipient(&source, "q2orch", true, true);
    let unqualified = recipient(&source, "q2other", false, false);
    rebind(&source, "profile.main", "fake-claude");
    rebind(&source, "agent_bin.main", "claude");
    source.start(&source.main_pane, "main", "claude");
    let config_path = source.scratch.join("config");
    let config = fs::read_to_string(&config_path).expect("config");
    fs::write(
        &config_path,
        config.replace("opencode/big-pickle", "github-copilot/gpt-6.1-sol"),
    )
    .expect("emitter starts with the provider named in its profile");
    let worker = limitstop_playback(&source, "opencode", FRAME);
    let running = source
        .tool_pid(&worker, "opencode")
        .expect("emitting worker");
    let path = source
        .scratch
        .join("vendorblock")
        .join(format!("{SCOPE}.rec"));

    let cycles = Cycles::default();
    assert!(
        watch_until(&source, || path.exists() && cycles.finished(&source) >= 4),
        "{}\n{}\n{}",
        source.capture(&worker),
        source.events(),
        daemon_err(&source)
    );
    let first = record_stamp(&path).expect("watchdog wrote the provider record");
    assert_eq!(notice_count(&source, &first), 1, "{:?}", source.received());
    assert_eq!(source.tool_pid(&worker, "opencode"), Some(running));
    assert_eq!(
        notice_count(&peer, &first),
        0,
        "the other watchdog is stopped"
    );
    assert_eq!(notice_count(&orchestrator, &first), 0);
    run_recipient(&peer, &source, &first, 1);
    run_recipient(&orchestrator, &source, &first, 1);
    run_recipient(&unqualified, &source, &first, 0);
    run_recipient(&peer, &source, &first, 1);
    run_recipient(&source, &source, &first, 1);

    // The vendor frame leaves and comes back, which creates a new episode.
    let healthy = source.scratch.join("limitstop-clear");
    fs::write(&healthy, "").expect("clear the vendor frame");
    crate::support::observe(
        |args| source.tmux(args),
        &["capture-pane", "-p", "-t", &worker],
        std::time::Instant::now() + crate::support::SETUP_WAIT,
        |frame| frame.contains("Usage restored"),
    );
    assert!(source.tmux(&["clear-history", "-t", &worker]).0);
    let clear = crate::cli::ae()
        .env("AE_HOME", &source.scratch)
        .args(["quota", "clear", SCOPE, "-y"])
        .output()
        .expect("human clears the provider record");
    assert!(clear.status.success(), "{clear:?}");
    assert!(!path.exists(), "the original record was cleared");
    fs::remove_file(healthy).expect("draw a fresh vendor refusal");
    let cycles = Cycles::default();
    assert!(
        watch_until(&source, || path.exists() && cycles.finished(&source) >= 4),
        "{}\n{}",
        source.events(),
        daemon_err(&source)
    );
    let second = record_stamp(&path).expect("the persistent vendor condition records again");
    // Accepted residual: a replacement with the same scope, second, kind,
    // seat and reason can share a receipt. The standing block still refuses.
    assert_ne!(first, second, "a new record has a new identity");
    assert_eq!(notice_count(&source, &first), 1);
    assert_eq!(notice_count(&source, &second), 1);
    run_recipient(&peer, &source, &second, 1);
    run_recipient(&orchestrator, &source, &second, 1);
    assert_eq!(notice_count(&peer, &first), 1);
    assert_eq!(notice_count(&orchestrator, &first), 1);
    assert_eq!(source.tool_pid(&worker, "opencode"), Some(running));
}
