//! Frozen #296B session-resume acceptance after a REAL reseat.
#![allow(
    clippy::expect_used,
    reason = "private fixture errors must fail loudly"
)]

use super::{Rig, add_profile, ae, helper, split_binary_rig, stop};

fn observable_rig(tag: &str) -> (Rig, std::path::PathBuf) {
    let rig = split_binary_rig(tag);
    let script = rig.scratch.join("observable-claude.pl");
    let fake = super::FAKE_AGENT
        .replace("__LAUNCHED__", &rig.launched.display().to_string())
        .replace("__SID__", "")
        .replacen(
            "close($log);",
            "print $log \"side=\", ($^X =~ m{/b/claude\\z} ? \"b\" : \"a\"), \"\\n\";\nclose($log);",
            1,
        );
    std::fs::write(&script, fake).expect("signed external Claude fixture");
    for side in ["a", "b"] {
        // A script shebang leaves ps comm=perl. A named interpreter has the
        // same observable process identity as the tool reseat must prove.
        std::fs::copy("/usr/bin/perl", rig.bin.join(side).join("claude"))
            .expect("Claude-named interpreter");
    }
    (rig, script)
}

fn reseated_then_stopped(tag: &str, session: &str, spawned: bool) -> Rig {
    let (rig, script) = observable_rig(tag);
    add_profile(
        &rig,
        "destination",
        &format!("claude {} --model destination", script.display()),
    );
    let full = std::fs::read_to_string(&rig.config).expect("fixture config");
    std::fs::write(
        &rig.config,
        format!("{full}\n[client_pairs]\ndestination = cc-mic\n"),
    )
    .expect("explicit global approval");
    let (code, out, err) = rig.launch(&["--local", session, "--lead", "solx"]);
    assert_eq!(code, Some(0), "{out}{err}");
    let (name, slot) = if spawned {
        let out = helper(&rig.dir(session).join("spawn"))
            .args(["helper", "--using", "solx", "--", "a synthetic task"])
            .env("HOME", &rig.scratch)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .output()
            .expect("real bare spawn");
        assert!(out.status.success(), "{out:?}");
        ("helper", "spawned.0")
    } else {
        ("lead", "main")
    };
    let pane = rig
        .panes(session)
        .into_iter()
        .find(|(_, found, _)| found == slot)
        .expect("the source seat has a pane")
        .0;
    // Product-valid dead seat: the external harness exited; its shell and stamps remain.
    assert!(
        rig.tmux(&["respawn-pane", "-k", "-t", &pane, "exec /bin/sh"])
            .0
    );
    let out = ae()
        .env("HOME", &rig.scratch)
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", &rig.config)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("AE_SENDER_OVERRIDE")
        .args(["reseat", session, name, "--using", "destination@cc-mic"])
        .output()
        .expect("real public reseat");
    assert!(out.status.success(), "{out:?}");
    let moved = rig.meta(session);
    assert!(
        moved.contains(&format!("profile.{slot}=destination\n")),
        "{moved}"
    );
    assert!(
        moved.contains(&format!("client.{slot}=cc-mic\n")),
        "{moved}"
    );
    stop(&rig, session);
    std::fs::write(&rig.launched, "").expect("fresh resume exec receipt");
    // Admission policy is for NEW choices, not a recorded identity.
    std::fs::write(&rig.config, full).expect("pair approval retired after reseat");
    rig
}

fn assert_resumed(rig: &Rig, session: &str, slot: &str) {
    let log = rig.launch_argv();
    assert!(
        log.contains("--model destination"),
        "reseated preset resumed: {log}"
    );
    assert_eq!(
        log.matches("side=b").count(),
        1,
        "override executable resumed: {log}"
    );
    assert!(
        !log.contains("side=a"),
        "default executable must not replace recorded override: {log}"
    );
    let store = std::fs::canonicalize(rig.scratch.join(".claude-mic")).expect("override store");
    assert!(
        log.contains(&format!("CLAUDE_CONFIG_DIR={}", store.display())),
        "override account: {log}"
    );
    let rebuilt = rig.meta(session);
    assert!(
        rebuilt.contains(&format!("profile.{slot}=destination\n")),
        "{rebuilt}"
    );
    assert!(
        rebuilt.contains(&format!("client.{slot}=cc-mic\n")),
        "{rebuilt}"
    );
    assert!(
        rebuilt.contains(&format!("config_home.{slot}={}\n", store.display())),
        "{rebuilt}"
    );
}

#[test]
fn presetreseat_session_resume_of_roster_slot_keeps_client_account_and_binary() {
    let rig = reseated_then_stopped("prresume-main", "lnprmain", false);
    let (code, out, err) = rig.launch(&["--local", "lnprmain"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert_resumed(&rig, "lnprmain", "main");
}

#[test]
fn presetreseat_session_resume_of_spawned_slot_keeps_client_in_flagless_snapshot() {
    let rig = reseated_then_stopped("prresume-spawn", "lnprspawn", true);
    let (code, out, err) = rig.launch(&["--local", "lnprspawn"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert_resumed(&rig, "lnprspawn", "spawned.0");
}

#[test]
fn presetreseat_session_resume_of_spawned_slot_keeps_client_with_explicit_seat_flag() {
    let rig = reseated_then_stopped("prresume-seat", "lnprseat", true);
    let (code, out, err) = rig.launch(&["--local", "lnprseat", "--lead", "solx"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert_resumed(&rig, "lnprseat", "spawned.0");
}
