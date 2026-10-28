//! #252 requirements: same-account model/effort changes retain exact conversation.
#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect their private stores"
)]
use super::seat_relaunch::Rig;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const ID: &str = "22222222-2222-4222-8222-222222222222";

fn root(rig: &Rig) -> PathBuf {
    std::fs::canonicalize(&rig.scratch).unwrap_or_else(|why| panic!("canonical scratch: {why}"))
}
fn replace_row(rig: &Rig, key: &str, value: &str) {
    let mut text = rig
        .meta()
        .lines()
        .filter(|line| !line.starts_with(&format!("{key}=")))
        .fold(String::new(), |mut text, line| {
            let _ = writeln!(text, "{line}");
            text
        });
    let _ = writeln!(text, "{key}={value}");
    assert!(std::fs::write(rig.dir.join("meta"), text).is_ok());
}
fn project_key(work: &Path) -> String {
    work.display()
        .to_string()
        .encode_utf16()
        .map(|unit| match u8::try_from(unit) {
            Ok(byte) if byte.is_ascii_alphanumeric() => char::from(byte),
            _ => '-',
        })
        .collect()
}
fn transcript(rig: &Rig, work: &Path) -> PathBuf {
    root(rig)
        .join("home-keep/projects")
        .join(project_key(work))
        .join(format!("{ID}.jsonl"))
}
fn plant(path: &Path) {
    assert!(
        std::fs::create_dir_all(path.parent().unwrap_or_else(|| panic!("transcript parent")))
            .is_ok()
    );
    assert!(std::fs::write(path, b"{\"type\":\"user\"}\n").is_ok());
}
fn configured(rig: &Rig, old_flags: &str, new_flags: &str) -> String {
    let config = rig.scratch.join("config");
    let text = std::fs::read_to_string(&config).unwrap_or_else(|why| panic!("rig config: {why}"));
    let command = format!(
        "CLAUDE_CONFIG_DIR={} {} {}",
        root(rig).join("home-keep").display(),
        rig.scratch.join("tools/claude").display(),
        rig.scratch.join("claude.pl").display()
    );
    let text = text.replace("[profiles]\n", &format!(
        "[profiles]\nfake-keep-source = \"{command} {old_flags}\"\nfake-keep-target = \"{command} {new_flags}\"\n"));
    assert!(
        ae::config::parse_identity(&text).is_ok(),
        "real config parser accepts frozen fixture"
    );
    assert!(std::fs::write(config, text).is_ok());
    rig.seat_rows("spawned.0", "scout", "keep-source", "claude");
    let pane = rig.new_pane("spawned.0", "scout");
    rig.start(&pane, "spawned.0", "claude");
    rig.kill_tools(&pane);
    for (key, value) in [
        ("work_dir", root(rig).join("work").display().to_string()),
        ("harness_session.spawned.0", ID.to_owned()),
        ("capture_floor.spawned.0", "100".to_owned()),
        (
            "config_home.spawned.0",
            root(rig).join("home-keep").display().to_string(),
        ),
        ("observed_model.spawned.0", "Opus".to_owned()),
        ("observed_model_pin.spawned.0", "opus".to_owned()),
        (
            "effort.spawned.0",
            super::effort_resume_spec::VALID
                .replace("slot:main", "slot:spawned.0")
                .replace("agent:lead", "agent:scout")
                .replace("tool:codex", "tool:claude")
                .replace("profile:custom", "profile:fake-keep-source")
                .replace("model:gpt-6.1-sol", "model:opus")
                .replace("base:xhigh low:high", "base:high low:low"),
        ),
    ] {
        replace_row(rig, key, &value);
    }
    plant(&transcript(rig, &root(rig).join("work")));
    assert!(std::fs::write(rig.scratch.join("launched"), "").is_ok());
    assert!(std::fs::write(rig.scratch.join("received"), "").is_ok());
    pane
}
fn reseat(rig: &Rig) -> (Option<i32>, String, String) {
    rig.run_top(
        &rig.main_pane,
        &[
            "reseat",
            &rig.session,
            "scout",
            "--using",
            "fake-keep-target",
        ],
    )
}
fn assert_kept(rig: &Rig, pane: &str, expected_flags: &[&str]) {
    let (code, out, err) = reseat(rig);
    assert_eq!(code, Some(0), "out={out} err={err}");
    assert_eq!(
        rig.meta_row("harness_session.spawned.0"),
        ID,
        "same conversation"
    );
    assert_eq!(rig.meta_row("capture_floor.spawned.0"), "100", "same floor");
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-keep-target");
    assert_eq!(
        rig.meta_row("config_home.spawned.0"),
        root(rig).join("home-keep").display().to_string()
    );
    for row in [
        "harness_session_prior.spawned.0=",
        "observed_model.spawned.0=",
        "observed_model_pin.spawned.0=",
        "\neffort.spawned.0=",
    ] {
        assert!(
            !rig.meta().contains(row),
            "new profile owns facts; no predecessor: {row}\n{}",
            rig.meta()
        );
    }
    assert!(
        rig.dir.join("launch.spawned.0.started").exists(),
        "resume marker back"
    );
    assert!(!rig.dir.join("seed.scout.md").exists(), "no seed published");
    assert!(
        !rig.received().contains("ae:ctx"),
        "no seed pasted: {}",
        rig.received()
    );
    let argv = rig.launched();
    assert!(
        argv.contains(&format!("--resume {ID}")),
        "exact resume: {argv}"
    );
    assert!(
        !argv.contains("--session-id"),
        "never a fresh launch: {argv}"
    );
    for flag in expected_flags {
        assert!(argv.contains(flag), "new profile flags {flag}: {argv}");
    }
    assert!(
        rig.events().contains(&format!("conversation {ID}, kept]")),
        "honest record: {}",
        rig.events()
    );
    assert!(
        !rig.events().contains(&format!("prior {ID}")),
        "kept id never prior"
    );
    assert!(
        out.lines().any(|line| line.contains("kept conversation")
            && line.contains(ID)
            && line.contains("fake-keep-source")
            && line.contains("fake-keep-target")),
        "named kept change: {out}"
    );
    assert!(
        !out.contains(", carried"),
        "same account has no copy: {out}"
    );
    assert!(rig.tool_pid(pane, "claude").is_some());
}
#[test]
fn same_account_model_and_effort_change_exact_resumes_with_new_profile_flags() {
    let rig = Rig::new("kcflags");
    let pane = configured(
        &rig,
        "--model opus --effort high",
        "--model sonnet --effort low",
    );
    assert_kept(&rig, &pane, &["--model sonnet", "--effort low"]);
}
#[test]
fn effort_only_change_keeps_conversation_and_drops_old_quota_override() {
    let rig = Rig::new("kceffort");
    let pane = configured(
        &rig,
        "--model opus --effort high",
        "--model opus --effort medium",
    );
    assert_kept(&rig, &pane, &["--model opus", "--effort medium"]);
}
#[test]
fn missing_store_seeds_and_names_the_lost_conversation() {
    let rig = Rig::new("kcmissing");
    configured(
        &rig,
        "--model opus --effort high",
        "--model sonnet --effort low",
    );
    assert!(std::fs::remove_file(transcript(&rig, &root(&rig).join("work"))).is_ok());
    let (code, out, err) = reseat(&rig);
    assert_eq!(code, Some(0), "out={out} err={err}");
    assert_ne!(
        rig.meta_row("harness_session.spawned.0"),
        ID,
        "fresh fallback"
    );
    assert!(rig.dir.join("seed.scout.md").exists(), "pack published");
    assert!(
        out.contains("could not keep conversation") && out.contains(ID),
        "loss on stdout: {out}"
    );
    assert!(
        rig.events().contains(&format!("prior {ID}, seeded (")),
        "loss recorded: {}",
        rig.events()
    );
    assert!(!rig.events().contains(&format!("conversation {ID}, kept]")));
}
#[test]
fn third_option_change_stays_on_existing_seed_path() {
    let rig = Rig::new("kcthird");
    configured(
        &rig,
        "--model opus --effort high",
        "--model sonnet --effort low --verbose",
    );
    let (code, out, err) = reseat(&rig);
    assert_eq!(code, Some(0), "out={out} err={err}");
    assert_ne!(rig.meta_row("harness_session.spawned.0"), ID);
    assert!(rig.dir.join("seed.scout.md").exists());
    assert!(
        !out.contains("kept conversation") && !out.contains("could not keep conversation"),
        "old silent eligibility fallback: {out}"
    );
    assert!(
        rig.events().contains(&format!("prior {ID}]")),
        "old seed record unchanged: {}",
        rig.events()
    );
}
#[test]
fn post_start_probe_disagreement_seeds_loudly_instead_of_claiming_kept() {
    let rig = Rig::new("kclink");
    configured(
        &rig,
        "--model opus --effort high",
        "--model sonnet --effort low",
    );
    let physical = root(&rig).join("work");
    let linked = root(&rig).join("linked-work");
    assert!(std::os::unix::fs::symlink(&physical, &linked).is_ok());
    assert!(std::fs::remove_file(transcript(&rig, &physical)).is_ok());
    plant(&transcript(&rig, &linked));
    replace_row(&rig, "work_dir", &linked.display().to_string());
    let (code, out, err) = reseat(&rig);
    assert_eq!(
        code,
        Some(0),
        "fallback leaves seat usable: out={out} err={err}"
    );
    assert_ne!(
        rig.meta_row("harness_session.spawned.0"),
        ID,
        "in-pane physical cwd lost resume"
    );
    assert!(
        rig.dir.join("seed.scout.md").exists(),
        "recovered pack published"
    );
    assert!(!rig.received().is_empty(), "recovered pack pasted");
    assert!(
        out.contains("could not keep conversation") && out.contains("resume fell back"),
        "honest fallback: {out}"
    );
    assert!(
        rig.events()
            .contains(&format!("prior {ID}, seeded (resume fell back)]")),
        "fallback record: {}",
        rig.events()
    );
    assert!(
        !rig.events().contains(&format!("conversation {ID}, kept]")),
        "never false kept verdict"
    );
}

#[test]
fn unobserved_tool_after_resume_fallback_never_records_kept() {
    use std::os::unix::fs::PermissionsExt;
    let rig = Rig::new("kcunseen");
    configured(
        &rig,
        "--model opus --effort high",
        "--model sonnet --effort low",
    );
    let physical = root(&rig).join("work");
    let linked = root(&rig).join("linked-work");
    assert!(std::os::unix::fs::symlink(&physical, &linked).is_ok());
    assert!(std::fs::remove_file(transcript(&rig, &physical)).is_ok());
    plant(&transcript(&rig, &linked));
    replace_row(&rig, "work_dir", &linked.display().to_string());
    assert!(
        std::fs::set_permissions(
            rig.scratch.join("tools/claude"),
            std::fs::Permissions::from_mode(0o644),
        )
        .is_ok()
    );
    let (code, out, err) = reseat(&rig);
    assert_eq!(code, Some(1), "tool never started: out={out} err={err}");
    assert_ne!(
        rig.meta_row("harness_session.spawned.0"),
        ID,
        "_run fell back before its failed exec"
    );
    assert!(
        rig.events().contains("pasted, tool not seen"),
        "failed start recorded: {}",
        rig.events()
    );
    assert!(
        !rig.events().contains(&format!("conversation {ID}, kept]")),
        "unproven resume must never be claimed kept: {}",
        rig.events()
    );
    assert!(!out.contains("kept conversation"), "unproven resume: {out}");
}
