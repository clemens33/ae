//! #296 follow-up 2. The chosen command's account is the warning source;
//! only bare profiles supply possible exits. Both public verbs stay advisory.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "independent acceptance fixtures own all tools, stores and sockets"
)]

use super::clientfollow_resume_spec::{Rig, Shape, success};

const WARNING: &str = "has no auto-reseat exit";

#[derive(Clone, Copy)]
enum Verb {
    Spawn,
    Reseat,
}

fn configured(tag: &str, candidate_store: &str, override_store: &str, on: bool) -> Rig {
    let rig = Rig::new(tag, Shape::Flagless);
    // The reseat predecessor differs outside the model flag, so it cannot
    // accidentally become a derived exit for p on account A.
    let source_script = rig.scratch.join("source.pl");
    std::fs::copy(rig.scratch.join("harness.pl"), &source_script).expect("distinct source family");
    let identity = format!(
        "{}\n[clients]\nexitc = {} config_home={}\n[profiles]\nexit = \"exitc {} --model chosen\"\nsource = \"default {} --model source\"\n",
        rig.identity("b", override_store, "claude"),
        rig.binary("c", "claude").display(),
        rig.scratch.join(candidate_store).display(),
        rig.scratch.join("harness.pl").display(),
        source_script.display(),
    );
    rig.write_config(&identity, "[auto_reseat]\np = exit\n", on);
    rig.launch();
    rig
}

fn choose(rig: &Rig, verb: Verb, pair: bool, override_store: &str) -> std::process::Output {
    let using = if pair { "p@carry" } else { "p" };
    let out = match verb {
        Verb::Spawn => rig.spawn("helper", using),
        Verb::Reseat => {
            success(&rig.spawn("helper", "source"));
            rig.wait_exec("a", "store-a");
            rig.provider_history("store-a");
            let pane = rig.pane("spawned.0").expect("actual predecessor pane");
            assert!(
                rig.tmux(&["respawn-pane", "-k", "-t", &pane, "exec /bin/sh"])
                    .0
            );
            super::support::pane_command(
                |args| rig.tmux(args),
                &pane,
                std::time::Instant::now() + super::support::PANE_WAIT,
                super::support::is_shell,
            );
            std::fs::write(&rig.receipt, "").expect("destination-only receipt");
            rig.reseat(using)
        }
    };
    success(&out);
    assert_eq!(
        rig.row("profile.spawned.0").as_deref(),
        Some("p"),
        "choice actually happened"
    );
    assert_eq!(
        rig.row("client.spawned.0").as_deref(),
        pair.then_some("carry")
    );
    rig.wait_exec(
        if pair { "b" } else { "a" },
        if pair { override_store } else { "store-a" },
    );
    out
}

fn warning(out: &std::process::Output) -> Option<String> {
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .find(|line| line.contains(WARNING))
        .map(ToOwned::to_owned)
}

// Same config within each contrast. Candidate B covers default A but cannot
// get an override B off B. This catches an unconditional pair-warning skip.
fn override_loses_cover(verb: Verb) {
    let bare = configured("cover-loss-bare", "store-b", "store-b", true);
    assert_eq!(
        warning(&choose(&bare, verb, false, "store-b")),
        None,
        "A can exit to bare B"
    );
    let pair = configured("cover-loss-pair", "store-b", "store-b", true);
    let out = choose(&pair, verb, true, "store-b");
    assert!(
        warning(&out).is_some(),
        "chosen B has only B as an exit; advisory warning required: {out:?}"
    );
}

// Candidate A cannot get default A off A, but does cover override B. This
// catches wrappers that merely apply the default profile's old verdict.
fn override_gains_cover(verb: Verb) {
    let bare = configured("cover-gain-bare", "store-a", "store-b", true);
    assert!(
        warning(&choose(&bare, verb, false, "store-b")).is_some(),
        "A has only A as an exit"
    );
    let pair = configured("cover-gain-pair", "store-a", "store-b", true);
    assert_eq!(
        warning(&choose(&pair, verb, true, "store-b")),
        None,
        "chosen B can exit to bare A"
    );
}

#[test]
fn spawn_override_loses_cover_and_warns() {
    override_loses_cover(Verb::Spawn);
}
#[test]
fn human_reseat_override_loses_cover_and_warns() {
    override_loses_cover(Verb::Reseat);
}
#[test]
fn spawn_override_gains_cover_and_is_silent() {
    override_gains_cover(Verb::Spawn);
}
#[test]
fn human_reseat_override_gains_cover_and_is_silent() {
    override_gains_cover(Verb::Reseat);
}

fn neutral(verb: Verb) {
    // Calibrated by the changed-account tests above. Alternate executable B
    // on A is not itself an account exit: pair and bare warnings match bytes.
    let bare = configured("neutral-bare", "store-a", "store-a", true);
    let expected = warning(&choose(&bare, verb, false, "store-a")).expect("bare A is uncovered");
    assert_eq!(
        expected,
        "ae: warning: profile 'p' has no auto-reseat exit off its account — a usage limit would leave the seat stuck; no profile in [profiles] runs on another account, add one first",
        "bare warning bytes unchanged"
    );
    let pair = configured("neutral-pair", "store-a", "store-a", true);
    assert_eq!(
        warning(&choose(&pair, verb, true, "store-a")).as_deref(),
        Some(expected.as_str())
    );
}
#[test]
fn spawn_same_account_alternate_binary_is_neutral() {
    neutral(Verb::Spawn);
}
#[test]
fn human_reseat_same_account_alternate_binary_is_neutral() {
    neutral(Verb::Reseat);
}

#[test]
fn global_off_suppresses_both_verbs_on_both_accounts() {
    for verb in [Verb::Spawn, Verb::Reseat] {
        for pair in [false, true] {
            let rig = configured("off", "store-a", "store-b", false);
            assert_eq!(warning(&choose(&rig, verb, pair, "store-b")), None);
        }
    }
}

#[test]
fn at_client_is_not_an_automatic_exit() {
    let rig = configured("no-at-exit", "store-a", "store-b", true);
    let text = std::fs::read_to_string(&rig.config).expect("config");
    // There is no bare B profile. An approved pair is never itself an exit.
    std::fs::write(&rig.config, text.replace("p = exit\n", "p = p@carry\n"))
        .expect("invalid automatic candidate");
    assert!(warning(&choose(&rig, Verb::Spawn, false, "store-b")).is_some());
}

fn watchdog_rig(tag: &str) -> Rig {
    let rig = configured(tag, "store-b", "store-b", true);
    let script = rig.scratch.join("exit.pl");
    std::fs::copy(rig.scratch.join("harness.pl"), &script)
        .expect("destination outside the source family");
    let config = std::fs::read_to_string(&rig.config).expect("global config");
    let config = config.replace(
        &format!(
            "exit = \"exitc {}",
            rig.scratch.join("harness.pl").display()
        ),
        &format!("exit = \"exitc {}", script.display()),
    );
    std::fs::write(
        &rig.config,
        format!("{config}\n[workspace]\nauto_reseat_at = 95\nauto_reseat_grace_secs = 0\n"),
    )
    .expect("headroom automatic move enabled");
    rig
}

#[test]
fn real_watchdog_reseat_is_silent_where_a_human_gets_the_warning() {
    // Calibrate the very same uncovered bare destination through human reseat.
    let human = watchdog_rig("watch-human");
    success(&human.spawn("helper", "source"));
    human.wait_exec("a", "store-a");
    human.provider_history("store-a");
    let pane = human.pane("spawned.0").expect("source pane");
    assert!(
        human
            .tmux(&["respawn-pane", "-k", "-t", &pane, "exec /bin/sh"])
            .0
    );
    super::support::pane_command(
        |args| human.tmux(args),
        &pane,
        std::time::Instant::now() + super::support::PANE_WAIT,
        super::support::is_shell,
    );
    let out = human.reseat("exit");
    assert!(
        out.status.success(),
        "human calibration actually moves: {out:?}"
    );
    assert!(
        warning(&out).is_some(),
        "uncovered human destination calibrates watchdog silence: {out:?}"
    );
    human.wait_exec("c", "store-b");

    let rig = watchdog_rig("watch-auto");
    success(&rig.spawn("helper", "p"));
    rig.wait_exec("a", "store-a");
    rig.provider_history("store-a");
    // A real tool exit leaves a valid dead seat. The automatic move still
    // traverses the actual leg, quota choice, lifecycle, reseat and warning
    // wrapper; this warning spec does not substitute a stop/frame oracle.
    let pane = rig.pane("spawned.0").expect("automatic predecessor pane");
    assert!(
        rig.tmux(&["respawn-pane", "-k", "-t", &pane, "exec /bin/sh"])
            .0
    );
    super::support::pane_command(
        |args| rig.tmux(args),
        &pane,
        std::time::Instant::now() + super::support::PANE_WAIT,
        super::support::is_shell,
    );
    let now = ae::time::Timestamp::now().epoch();
    for (store, percent) in [("store-a", 98), ("store-b", 5)] {
        std::fs::write(rig.scratch.join(store).join(".claude.json"), format!(
            "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"weekly_all\",\"group\":\"weekly_all\",\"percent\":{percent},\"resets_at\":\"{}\",\"scope\":null}}]}}}}}}\n",
            now * 1_000, ae::time::Timestamp::from_epoch(now + 86_400),
        )).expect("provider quota cache");
    }
    let key = ae::time::Timestamp::from_epoch(now - 20);
    let attempted = ae::time::Timestamp::from_epoch(now - 19);
    let events = rig.dir().join("events.jsonl");
    let prior = std::fs::read_to_string(&events).expect("real spawn journal");
    std::fs::write(events, format!(
        "{prior}{{\"ts\":\"{key}\",\"actor\":\"watchdog\",\"action\":\"auto-reseat-headroom\",\"target\":\"helper\",\"target_slot\":\"spawned.0\",\"target_session\":\"cfsession\",\"summary\":\"headroom 98% weekly_all 7d\"}}\n{{\"ts\":\"{attempted}\",\"actor\":\"watchdog\",\"action\":\"{}\",\"target\":\"helper\",\"target_slot\":\"spawned.0\",\"target_session\":\"cfsession\",\"ref\":\"{key}\"}}\n", ae::autoreseat::ATTEMPT_ACTION,
    )).expect("valid pending watchdog headroom attempt");
    let out = rig
        .command()
        .arg("_auto-reseat")
        .arg(rig.dir())
        .args(["spawned.0", &key.to_string()])
        .output()
        .expect("real watchdog automatic leg");
    assert!(
        out.status.success(),
        "automatic leg actually moves: {out:?}"
    );
    assert_eq!(
        rig.row("profile.spawned.0").as_deref(),
        Some("exit"),
        "watchdog actually moves: {out:?}"
    );
    rig.wait_exec("c", "store-b");
    assert_eq!(
        warning(&out),
        None,
        "watchdog remains silent despite uncovered destination"
    );
}
