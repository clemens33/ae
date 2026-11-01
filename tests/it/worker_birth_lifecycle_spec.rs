//! Frozen stats287 s1. Child of the existing spawn fixture; no copied rig.
//! Oracle: issue #287 counting table, approved plan tests 1-4.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "frozen acceptance fixtures inspect only their own real private-server state"
)]

use super::Rig;
use ae::json::Value;
use std::fmt::Write as _;

const SID: &str = "11111111-1111-4111-8111-111111111111";
const CANARY: &str = "PRIVATE_BRIEF_CANARY_wb287";

fn rig(tag: &str) -> Rig {
    let rig = Rig::new(tag);
    let mut meta = rig.meta();
    writeln!(meta, "session_id={SID}").expect("fixture UUID row");
    std::fs::write(rig.dir.join("meta"), meta).expect("fixture UUID");
    assert!(
        rig.tmux(&[
            "set-option",
            "-t",
            &format!("={}:", rig.session),
            "@ae_session_uuid",
            SID
        ])
        .0
    );
    rig
}

fn records(rig: &Rig, action: &str) -> Vec<Value> {
    rig.events()
        .lines()
        .map(|line| ae::json::parse(line).expect("the actual journal is JSON"))
        .filter(|event| text(event, "action") == Some(action))
        .collect()
}

fn text<'a>(record: &'a Value, key: &str) -> Option<&'a str> {
    match record.get(key) {
        Some(Value::Str(value)) => Some(value),
        _ => None,
    }
}

fn committed(rig: &Rig, name: &str, profile: &str) -> Value {
    let before = records(rig, "worker-birth").len();
    let (code, stdout, stderr) = rig.run(ae::cli::SPAWN, &[name, "--using", profile, "--", CANARY]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let births = records(rig, "worker-birth");
    assert_eq!(
        births.len(),
        before + 1,
        "one committed new worker, one certificate"
    );
    let birth = births.last().expect("new certificate").clone();
    assert_eq!(text(&birth, "target"), Some(name));
    assert_eq!(text(&birth, "target_session_id"), Some(SID));
    assert!(matches!(birth.get("birth_v"), Some(Value::Num(1))));
    assert!(matches!(birth.get("schema_version"), Some(Value::Num(1))));
    let slot = text(&birth, "target_slot").expect("stable slot");
    assert!(slot.starts_with("spawned."));
    assert_eq!(text(&birth, "birth"), Some(rig.launch_id(slot).as_str()));
    let announcements = records(rig, "spawn");
    let announcement = announcements.last().expect("existing announcement remains");
    assert_eq!(text(announcement, "birth"), text(&birth, "birth"));
    assert_eq!(text(announcement, "summary"), Some(CANARY));
    assert!(
        announcement.get("target_slot").is_none(),
        "existing usage attribution unchanged"
    );
    // The certificate is a projection, never a second copy of a task payload.
    let raw = rig.events();
    for line in raw.lines().filter(|line| {
        let parsed = ae::json::parse(line).expect("JSON");
        text(&parsed, "action") == Some("worker-birth")
    }) {
        assert!(!line.contains(CANARY));
        for forbidden in [
            "summary",
            "body_file",
            "command",
            "argv",
            "config_home",
            "endpoint",
        ] {
            assert!(
                ae::json::parse(line)
                    .expect("certificate")
                    .get(forbidden)
                    .is_none(),
                "{forbidden}"
            );
        }
        assert!(
            !line.contains(&rig.scratch.display().to_string()),
            "no command/config path"
        );
    }
    assert!(
        records(rig, "worker-fact").is_empty(),
        "fact writers belong to later slices"
    );
    birth
}

#[test]
fn rejected_and_rolled_back_spawns_leave_no_birth_even_with_a_leftover_seat() {
    let rig = rig("wbrollback");
    let windows = rig.windows().len();
    let (code, _, _) = rig.run(ae::cli::SPAWN, &["badselection", "--using", "missing"]);
    assert_eq!(code, Some(1));
    assert!(records(&rig, "worker-birth").is_empty());
    assert_eq!(rig.windows().len(), windows);

    std::fs::write(rig.dir.join("messages"), "not a directory").expect("body-store refusal");
    let (code, _, stderr) = rig.run(
        ae::cli::SPAWN,
        &["unstored", "--using", "codexish", "--", CANARY],
    );
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("spawn rolled back"), "{stderr}");
    assert!(records(&rig, "worker-birth").is_empty());
    assert_eq!(rig.windows().len(), windows);
    std::fs::remove_file(rig.dir.join("messages")).expect("remove fixture blocker");

    assert!(
        rig.tmux(&[
            "set",
            "-s",
            "command-alias[100]",
            "set-option=nosuchcommand"
        ])
        .0
    );
    let (code, _, stderr) = rig.run(ae::cli::SPAWN, &["unstamped", "--using", "fake"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("could not be stamped"), "{stderr}");
    assert!(records(&rig, "worker-birth").is_empty());
    assert_eq!(rig.windows().len(), windows);

    assert!(
        rig.tmux(&["set", "-s", "command-alias[101]", "kill-pane=nosuchcommand"])
            .0
    );
    let (code, _, stderr) = rig.run(ae::cli::SPAWN, &["leftover", "--using", "fake"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        rig.meta().contains("=leftover"),
        "fixture really retained a reservation"
    );
    assert_eq!(rig.windows().len(), windows + 1);
    assert!(
        records(&rig, "worker-birth").is_empty(),
        "reservation is not commitment"
    );
}

#[test]
fn a_committed_worker_with_an_undelivered_brief_still_has_one_birth() {
    let rig = rig("wbbusy");
    rig.make_claude_input_busy();
    let (code, stdout, stderr) =
        rig.run(ae::cli::SPAWN, &["busy", "--using", "fake", "--", CANARY]);
    assert_eq!(code, Some(1), "{stdout}\n{stderr}");
    assert!(
        stderr.contains("SPAWN INCOMPLETE"),
        "reached committed brief failure: {stderr}"
    );
    assert!(
        rig.panes()
            .iter()
            .any(|(_, slot, name)| slot == "spawned.0" && name == "busy")
    );
    let births = records(&rig, "worker-birth");
    assert_eq!(births.len(), 1);
    assert_eq!(
        text(&births[0], "birth"),
        Some(rig.launch_id("spawned.0").as_str())
    );
    assert_eq!(records(&rig, "spawn").len(), 1);
    assert_eq!(records(&rig, "spawn-failed").len(), 1);
    assert_eq!(
        text(&records(&rig, "spawn")[0], "birth"),
        text(&births[0], "birth")
    );
    assert!(records(&rig, "worker-fact").is_empty());
    assert!(!format!("{:?}", births[0]).contains(CANARY));
}

#[test]
fn a_dispatched_spawn_whose_harness_exec_fails_still_has_one_birth() {
    let rig = rig("wbexecfail");
    std::fs::write(rig.scratch.join("claude"), "#!/missing-wb287-interpreter\n")
        .expect("executable fixture whose exec fails");
    let (code, stdout, stderr) = rig.run(
        ae::cli::SPAWN,
        &["failedstart", "--using", "fake", "--", CANARY],
    );
    assert_eq!(
        code,
        Some(1),
        "startup/brief is not successful: {stdout}\n{stderr}"
    );
    assert!(
        rig.panes()
            .iter()
            .any(|(_, slot, name)| slot == "spawned.0" && name == "failedstart")
    );
    assert_eq!(
        records(&rig, "worker-birth").len(),
        1,
        "birth measures commitment, not model execution"
    );
    assert_eq!(records(&rig, "spawn").len(), 1);
    assert!(
        records(&rig, "worker-fact").is_empty(),
        "outcome writers arrive later"
    );
}

#[test]
fn a_failed_certificate_append_is_reported_and_does_not_fail_the_committed_spawn() {
    let rig = rig("wblost");
    std::fs::create_dir(rig.dir.join("events.jsonl")).expect("unwritable journal fixture");
    let (code, stdout, stderr) = rig.run(
        ae::cli::SPAWN,
        &["retained", "--using", "fake", "--", CANARY],
    );
    assert_eq!(code, Some(0), "launch continues: {stdout}\n{stderr}");
    assert!(stdout.starts_with("Spawned retained in pane %"));
    assert!(
        stderr.contains("birth"),
        "the certificate append gap is said: {stderr}"
    );
    assert!(
        rig.panes()
            .iter()
            .any(|(_, slot, name)| slot == "spawned.0" && name == "retained")
    );
    assert!(
        rig.submitted().contains(CANARY),
        "the brief still reaches its agent"
    );
}

#[test]
fn resolved_clients_and_direct_commands_preserve_birth_selection_without_the_prompt() {
    let rig = rig("wbroutes");
    rig.enable_codex_profile();
    let config_path = rig.scratch.join("config");
    let mut config = std::fs::read_to_string(&config_path).expect("fixture config");
    write!(
        config,
        "\n[clients]\nroute-a = {} config_home={}\nroute-b = {} config_home={}\n\n[profiles]\nvia-a = \"route-a --model sonnet --effort high\"\nvia-b = \"route-b --model sonnet --effort high\"\nvia-a-alt = \"route-a --model opus --effort max\"\nvia-codex = \"codex-test --model gpt-6.1-sol -c model_reasoning_effort=xhigh\"\n",
        rig.scratch.join("claude").display(), rig.scratch.join("home-a").display(),
        rig.scratch.join("claude").display(), rig.scratch.join("home-b").display()
    )
    .expect("route configuration");
    std::fs::write(config_path, config).expect("two routes");
    let a = committed(&rig, "one", "via-a");
    let b = committed(&rig, "two", "via-b");
    let alt = committed(&rig, "three", "via-a-alt");
    let direct = committed(&rig, "four", "fake");
    let codex = committed(&rig, "five", "via-codex");
    for (record, client, profile) in [(&a, "route-a", "via-a"), (&b, "route-b", "via-b")] {
        assert_eq!(text(record, "birth_tool"), Some("claude"));
        assert_eq!(text(record, "birth_client"), Some(client));
        assert_eq!(text(record, "birth_model"), Some("sonnet"));
        assert_eq!(text(record, "birth_effort"), Some("high"));
        assert_eq!(text(record, "birth_profile"), Some(profile));
    }
    assert_ne!(text(&a, "birth"), text(&b, "birth"));
    assert_eq!(text(&alt, "birth_client"), Some("route-a"));
    assert_eq!(text(&alt, "birth_model"), Some("opus"));
    assert_eq!(text(&alt, "birth_effort"), Some("max"));
    assert_eq!(text(&direct, "birth_tool"), Some("claude"));
    assert_eq!(text(&codex, "birth_tool"), Some("codex"));
    assert_eq!(text(&codex, "birth_client"), Some("codex-test"));
    assert_eq!(text(&codex, "birth_model"), Some("gpt-6.1-sol"));
    assert_eq!(text(&codex, "birth_effort"), Some("xhigh"));
    for key in ["birth_client", "birth_model", "birth_effort"] {
        assert!(
            direct.get(key).is_none(),
            "no configured {key}, no invented selection"
        );
    }
}

#[test]
fn fixed_seats_restart_reseat_and_session_resume_do_not_create_new_births() {
    use std::os::unix::fs::PermissionsExt;

    let rig = rig("wblifetime");
    // A shebang fake runs as perl. Relaunch and reseat prove the adapter by
    // executable name, so this fixture needs a genuinely claude-named process.
    // This is the existing seat_relaunch fixture's named-interpreter pattern.
    let binary = rig.scratch.join("claude");
    let script = rig.scratch.join("claude.pl");
    std::fs::rename(&binary, &script).expect("keep the existing fake body");
    std::fs::copy("/usr/bin/perl", &binary).expect("named interpreter");
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
        .expect("executable named fake");
    let path = rig.scratch.join("config");
    let home = rig.scratch.join("home").display().to_string();
    std::fs::write(
        &path,
        format!(
            "[profiles]\nfake = \"CLAUDE_CONFIG_DIR={home} {} {}\"\n\n[roster]\nlead = fake\n\n[workspace]\nmain = lead\nlayout = vertical\n",
            binary.display(),
            script.display()
        ),
    )
    .expect("named fake profile for initial start and resume");
    assert!(
        records(&rig, "worker-birth").is_empty(),
        "the fixed lead is excluded"
    );
    let first = committed(&rig, "scout", "fake");
    let slot = text(&first, "target_slot").expect("slot").to_owned();
    let birth_id = text(&first, "birth").expect("birth").to_owned();
    let pane = rig
        .panes()
        .into_iter()
        .find(|(_, found, _)| found == &slot)
        .expect("worker pane")
        .0;
    assert!(rig.tmux(&["respawn-pane", "-k", "-t", &pane, "sh"]).0);
    super::wait_for_agent_gone(&rig, &slot, "claude");
    let (code, out, err) = rig.run("_relaunch", &["scout"]);
    assert_eq!(code, Some(0), "relaunch: {out}\n{err}");
    assert_eq!(records(&rig, "worker-birth").len(), 1);

    let mut config = std::fs::read_to_string(&path).expect("config");
    write!(
        config,
        "\n[profiles]\nother = \"CLAUDE_CONFIG_DIR={home} {} {} --model sonnet --effort high\"\n",
        binary.display(),
        script.display()
    )
    .expect("second profile configuration");
    std::fs::write(&path, config).expect("second resolved profile");
    let output = super::super::cli::ae()
        .env("AE_HOME", &rig.scratch)
        .env("CONFIG_FILE", &path)
        // The owned synthetic frame has no supported idle/busy marker.
        .args([
            "reseat",
            &rig.session,
            "scout",
            "--using",
            "other",
            "--stop-unknown",
        ])
        .output()
        .expect("real reseat");
    assert!(
        output.status.success(),
        "reseat: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_ne!(
        rig.launch_id(&slot),
        birth_id,
        "a real reseat rotates the start identity"
    );
    assert_eq!(records(&rig, "worker-birth").len(), 1);
    assert_eq!(
        text(&records(&rig, "worker-birth")[0], "birth"),
        Some(birth_id.as_str())
    );

    // Leave the private server alive: its successful listing proves the stopped
    // target absent. A missing socket would be Unknown, not a valid resume fixture.
    assert!(rig.tmux(&["new-session", "-d", "-s", "wbkeeper", "sh"]).0);
    assert!(
        rig.tmux(&["kill-session", "-t", &format!("={}", rig.session)])
            .0
    );
    let output = super::super::cli::ae()
        .env("AE_HOME", &rig.scratch)
        .env("CONFIG_FILE", &path)
        .env("AE_NO_AUTOSTART", "1")
        .env("AE_TMUX_SERVER", &rig.sock)
        .args([&rig.session, "--no-attach"])
        .output()
        .expect("real session resume");
    assert!(
        output.status.success(),
        "resume: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(records(&rig, "worker-birth").len(), 1);
}

#[test]
fn retiring_then_spawning_the_same_name_is_a_distinct_birth() {
    let rig = rig("wbreborn");
    let first = committed(&rig, "scout", "fake");
    let (code, out, err) = rig.run(ae::cli::RETIRE, &["scout"]);
    assert_eq!(code, Some(0), "{out}\n{err}");
    assert_eq!(
        records(&rig, "worker-birth").len(),
        1,
        "retirement preserves history"
    );
    let second = committed(&rig, "scout", "fake");
    assert_ne!(text(&first, "birth"), text(&second, "birth"));
    assert_eq!(
        text(&first, "target_session_id"),
        text(&second, "target_session_id")
    );
    assert_eq!(records(&rig, "worker-birth").len(), 2);
}
