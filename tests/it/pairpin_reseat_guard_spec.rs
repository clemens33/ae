//! #296 relaunch residual, lead R1: reseat judges a labeled spawned source
//! before stopping it. A project overlay cannot repair a broken global pair.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "independent acceptance owns private configs, stores and processes"
)]

use super::clientfollow_resume_spec::{Rig, Shape, success};

fn source(tag: &str, labeled: bool) -> Rig {
    let rig = Rig::new(tag, Shape::Flagless);
    let config = std::fs::read_to_string(&rig.config).expect("global fixture");
    std::fs::write(
        &rig.config,
        format!(
            "{config}\n[profiles]\naway = \"default {} --model destination\"\n",
            rig.scratch.join("harness.pl").display()
        ),
    )
    .expect("bare destination");
    rig.launch();
    success(&rig.spawn("helper", if labeled { "p@carry" } else { "p" }));
    rig.wait_exec(
        if labeled { "b" } else { "a" },
        if labeled { "store-b" } else { "store-a" },
    );
    rig.provider_history(if labeled { "store-b" } else { "store-a" });
    rig
}

fn tool_pid(rig: &Rig) -> u32 {
    let pane = rig.pane("spawned.0").expect("live source pane");
    let (ok, root) = rig.tmux(&["display-message", "-p", "-t", &pane, "#{pane_pid}"]);
    assert!(ok, "pane identity reads");
    let root: u32 = root.trim().parse().expect("pane shell pid");
    let table = ae::procs::snapshot().expect("real process table");
    table
        .iter()
        .find(|proc| proc.ppid == root && proc.comm.rsplit('/').next() == Some("claude"))
        .expect("the actual source tool remains a child of its pane shell")
        .pid
}

fn reseat_live(rig: &Rig) -> std::process::Output {
    rig.command()
        .args([
            "reseat",
            "cfsession",
            "helper",
            "--using",
            "away",
            "--stop-unknown",
        ])
        .output()
        .expect("real human reseat of the live source")
}

fn refusal_before_stop(changed_store: bool) {
    let rig = source(
        if changed_store {
            "pin-source-store"
        } else {
            "pin-source-missing"
        },
        true,
    );
    let global = std::fs::read_to_string(&rig.config).expect("global identity");
    let carry = global
        .lines()
        .find(|line| line.starts_with("carry = "))
        .expect("the established global client");
    std::fs::write(
        rig.project.join(".ae/config"),
        format!("[clients]\n{carry}\n"),
    )
    .expect("project retains the original client and account");
    let replacement = if changed_store {
        carry.replace(
            &rig.scratch.join("store-b").display().to_string(),
            &rig.scratch.join("store-a").display().to_string(),
        )
    } else {
        String::new()
    };
    assert_ne!(carry, replacement, "the global authority actually changes");
    std::fs::write(&rig.config, global.replace(carry, &replacement))
        .expect("global pair no longer judges");
    let before_pid = tool_pid(&rig);
    let before_meta = rig.meta();
    let before_execs = rig.execs();
    let before_events =
        std::fs::read_to_string(rig.dir().join("events.jsonl")).expect("established journal");
    let out = reseat_live(&rig);
    assert!(
        !out.status.success(),
        "broken labeled source must refuse: {out:?}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("carry")
            && ["restore", "retire", "end the session"]
                .iter()
                .any(|word| stderr.contains(word)),
        "the refusal names the client and a remedy: {stderr}"
    );
    assert_eq!(tool_pid(&rig), before_pid, "refusal must precede any stop");
    assert_eq!(rig.meta(), before_meta, "source identity remains unchanged");
    assert_eq!(
        rig.execs(),
        before_execs,
        "no destination or restarted source executes"
    );
    let after_events =
        std::fs::read_to_string(rig.dir().join("events.jsonl")).expect("journal after refusal");
    assert!(
        after_events.starts_with(&before_events),
        "existing history is preserved"
    );
    assert!(
        !after_events[before_events.len()..]
            .lines()
            .filter_map(|line| ae::events::Event::parse_line(line).ok())
            .any(|event| event.action == "stop" || event.action == "reseat"),
        "no stop or move record may follow the refusal: {after_events}"
    );
}

#[test]
fn missing_global_client_refuses_reseat_before_the_source_stops() {
    refusal_before_stop(false);
}

#[test]
fn changed_global_store_refuses_reseat_before_the_source_stops() {
    refusal_before_stop(true);
}

#[test]
fn bare_profile_reseat_still_moves() {
    let rig = source("pin-source-bare", false);
    std::fs::write(&rig.receipt, "").expect("destination-only receipt");
    success(&reseat_live(&rig));
    assert_eq!(rig.row("profile.spawned.0").as_deref(), Some("away"));
    assert_eq!(rig.row("client.spawned.0"), None);
    rig.wait_exec("a", "store-a");
}
