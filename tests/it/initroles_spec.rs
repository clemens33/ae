//! Independent acceptance: each init role selects harness, model and effort.

#![allow(
    clippy::expect_used,
    reason = "acceptance fixture helpers require valid config and owned scratch setup"
)]

use super::Rig;

fn command(text: &str, role: &str) -> Vec<String> {
    let config = ae::config::parse_identity(text).expect("ordinary INI identity config");
    let (_, profile) = config
        .roster
        .iter()
        .find(|(name, _)| name == role)
        .expect("role bound");
    let command = config
        .command(profile, None)
        .expect("resolvable profile")
        .expect("defined profile");
    ae::launch_cmd::lex_simple_command(command.as_str())
        .expect("one simple launch command")
        .words
}

fn assert_role(text: &str, role: &str, harness: &str, model: &str, effort: &str) {
    let words = command(text, role);
    assert_eq!(
        words.first().map(String::as_str),
        Some(harness),
        "{role}: {words:?}"
    );
    let model_flag = if harness == "claude" { "--model" } else { "-m" };
    assert!(
        words.windows(2).any(|pair| pair == [model_flag, model]),
        "{role}: {words:?}"
    );
    let wanted = if harness == "claude" {
        effort.to_owned()
    } else {
        format!("model_reasoning_effort={effort}")
    };
    let effort_flag = if harness == "claude" {
        "--effort"
    } else {
        "-c"
    };
    assert!(
        words
            .windows(2)
            .any(|pair| pair[0] == effort_flag && pair[1] == wanted),
        "{role}: {words:?}"
    );
}

pub(super) fn assert_defaults(text: &str) {
    assert_role(text, "lead", "claude", "claude-opus-5-5", "high");
    assert_role(text, "colead", "codex", "gpt-6.1-sol", "xhigh");
}

fn written(rig: &Rig, args: &[&str]) -> String {
    let (code, out, err) = rig.run(args);
    assert_eq!(code, Some(0), "{args:?}: {out}\n{err}");
    std::fs::read_to_string(&rig.config).expect("published config")
}

#[test]
fn discovered_pair_defaults_to_opus55_high_and_sol61_xhigh() {
    let rig = Rig::new("roles-default", &["claude", "codex"]);
    assert_defaults(&written(&rig, &["init", "--yes"]));
}

#[test]
fn both_roles_select_all_three_fields_independently_and_can_swap_harnesses() {
    let rig = Rig::new("roles-swapped", &["claude", "codex"]);
    let text = written(
        &rig,
        &[
            "init",
            "--yes",
            "--lead-harness",
            "codex",
            "--lead-model",
            "gpt-6.1-sol",
            "--lead-effort",
            "low",
            "--colead-harness",
            "claude",
            "--colead-model",
            "claude-opus-5-5",
            "--colead-effort",
            "max",
        ],
    );
    assert_role(&text, "lead", "codex", "gpt-6.1-sol", "low");
    assert_role(&text, "colead", "claude", "claude-opus-5-5", "max");
}

#[test]
fn same_harness_roles_keep_different_efforts_without_overwriting_each_other() {
    let rig = Rig::new("roles-same", &["claude", "codex"]);
    let text = written(
        &rig,
        &[
            "init",
            "--yes",
            "--lead-harness",
            "claude",
            "--lead-model",
            "claude-opus-5-5",
            "--lead-effort",
            "medium",
            "--colead-harness",
            "claude",
            "--colead-model",
            "claude-opus-5-5",
            "--colead-effort",
            "xhigh",
        ],
    );
    assert_role(&text, "lead", "claude", "claude-opus-5-5", "medium");
    assert_role(&text, "colead", "claude", "claude-opus-5-5", "xhigh");
}

#[test]
fn selecting_a_nondefault_model_changes_only_the_requested_role() {
    let rig = Rig::new("roles-model", &["claude", "codex"]);
    let text = written(
        &rig,
        &["init", "--yes", "--lead-model", "claude-sonnet-5-5"],
    );
    assert_role(&text, "lead", "claude", "claude-sonnet-5-5", "high");
    assert_role(&text, "colead", "codex", "gpt-6.1-sol", "xhigh");
}

#[test]
fn legacy_profile_flags_still_bind_the_requested_profiles() {
    let rig = Rig::new("roles-legacy", &["claude", "codex"]);
    let text = written(
        &rig,
        &["init", "--yes", "--lead", "fablex", "--colead", "solx"],
    );
    let config = ae::config::parse_identity(&text).expect("INI");
    assert!(
        config
            .roster
            .contains(&("lead".to_owned(), "fablex".to_owned()))
    );
    assert!(
        config
            .roster
            .contains(&("colead".to_owned(), "solx".to_owned()))
    );
    assert!(!command(&text, "lead").is_empty());
    assert!(!command(&text, "colead").is_empty());
}

#[test]
fn invalid_role_values_and_conflicting_flags_leave_no_config_or_proposal() {
    let cases: &[&[&str]] = &[
        &["--lead-harness", "missing"],
        &["--lead-harness", "grok"],
        &["--lead-model", "not-a-model"],
        &["--lead-harness", "claude", "--lead-model", "gpt-6.1-sol"],
        &["--lead-effort", "extreme"],
        &[
            "--colead-harness",
            "codex",
            "--colead-model",
            "claude-opus-5-5",
        ],
        &["--colead-model", "gpt-6-luna", "--colead-effort", "ultra"],
        &["--lead", "fablex", "--lead-effort", "low"],
        &["--lead-effort", "low", "--lead", "fablex"],
        &["--solo", "--colead-harness", "codex"],
        &["--colead-harness", "codex", "--solo"],
        &["--lead-effort", "low", "--lead-effort", "high"],
        &["--lead-model"],
        &["--lead-model", "bad\n[roster]"],
    ];
    for (index, flags) in cases.iter().enumerate() {
        let rig = Rig::new(&format!("roles-invalid-{index}"), &["claude", "codex"]);
        let mut args = vec!["init", "--yes"];
        args.extend_from_slice(flags);
        let (code, out, err) = rig.run(&args);
        assert_eq!(code, Some(2), "{args:?}: {out}\n{err}");
        assert!(err.contains("Usage: ae init"), "{err}");
        assert!(!rig.config.exists(), "{args:?}");
        assert!(!rig.config.with_file_name("config.proposed").exists());
    }
}

#[test]
fn partial_choices_infer_the_models_harness_and_keep_role_effort_defaults() {
    let rig = Rig::new("roles-infer", &["claude", "codex"]);
    let text = written(
        &rig,
        &[
            "init",
            "--yes",
            "--lead-model",
            "gpt-6.1-sol",
            "--colead-model",
            "claude-opus-5-5",
        ],
    );
    assert_role(&text, "lead", "codex", "gpt-6.1-sol", "high");
    assert_role(&text, "colead", "claude", "claude-opus-5-5", "xhigh");
}

#[test]
fn codex_max_is_a_supported_effort_and_is_not_silently_replaced() {
    let rig = Rig::new("roles-max", &["codex"]);
    let text = written(
        &rig,
        &[
            "init",
            "--yes",
            "--lead-model",
            "gpt-6.1-sol",
            "--lead-effort",
            "max",
        ],
    );
    assert_role(&text, "lead", "codex", "gpt-6.1-sol", "max");
}

#[test]
fn claude_only_defaults_to_opus55_for_both_roles_with_distinct_efforts() {
    let rig = Rig::new("roles-claude", &["claude"]);
    let text = written(&rig, &["init", "--yes"]);
    assert_role(&text, "lead", "claude", "claude-opus-5-5", "high");
    assert_role(&text, "colead", "claude", "claude-opus-5-5", "xhigh");
    assert!(!text.contains("\ncodex = codex\n"));
    assert!(!command(&text, "orchestrator").is_empty());
}

#[test]
fn codex_only_defaults_to_sol61_for_both_roles_with_distinct_efforts() {
    let rig = Rig::new("roles-codex", &["codex"]);
    let text = written(&rig, &["init", "--yes"]);
    assert_role(&text, "lead", "codex", "gpt-6.1-sol", "high");
    assert_role(&text, "colead", "codex", "gpt-6.1-sol", "xhigh");
    assert!(!text.contains("\nclaude = claude\n"));
    assert!(!command(&text, "orchestrator").is_empty());
}

fn tmux(rig: &Rig, tail: &[&str]) -> String {
    let mut args = vec![
        "-f".to_owned(),
        "/dev/null".to_owned(),
        "-S".to_owned(),
        rig.scratch.join("tmux.sock").display().to_string(),
    ];
    args.extend(tail.iter().map(|word| (*word).to_owned()));
    let (ok, out) = super::super::phase2::run_tmux(&args, &rig.scratch);
    assert!(ok, "{args:?}: {out}");
    out
}

fn terminal(rig: &Rig, extra: &[&str]) {
    let home = format!("HOME={}", rig.scratch.join("home").display());
    let state = format!("AE_HOME={}", rig.scratch.join("state").display());
    let config = format!("CONFIG_FILE={}", rig.config.display());
    let path = format!("PATH={}", rig.bin.display());
    let mut args = vec![
        "new-session",
        "-d",
        "-s",
        "ui",
        "-x",
        "180",
        "-y",
        "60",
        "/usr/bin/env",
        "-u",
        "TMUX",
        "-u",
        "TMUX_PANE",
        &home,
        &state,
        &config,
        &path,
        env!("CARGO_BIN_EXE_ae"),
        "init",
        "--no-orchestrator",
        "--palette",
        "a",
    ];
    args.extend_from_slice(extra);
    tmux(rig, &args);
    tmux(
        rig,
        &["set-option", "-w", "-t", "ui:0", "remain-on-exit", "on"],
    );
}

fn wait_prompt(rig: &Rig, needle: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let out = tmux(rig, &["capture-pane", "-p", "-t", "ui:0.0", "-S", "-200"]);
        if out.contains(needle) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "waiting for {needle}: {out}"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

fn answer(rig: &Rig, prompt: &str, value: &str) {
    wait_prompt(rig, prompt);
    if !value.is_empty() {
        tmux(rig, &["send-keys", "-l", "-t", "ui:0.0", value]);
    }
    tmux(rig, &["send-keys", "-t", "ui:0.0", "Enter"]);
}

#[test]
fn terminal_chooses_each_roles_harness_model_and_effort_separately() {
    let rig = Rig::new("roles-terminal", &["claude", "codex"]);
    terminal(&rig, &[]);
    answer(&rig, "Lead harness", "codex");
    answer(&rig, "Lead model", "gpt-6.1-sol");
    answer(&rig, "Lead effort", "low");
    answer(&rig, "Colead harness", "claude");
    answer(&rig, "Colead model", "claude-opus-5-5");
    answer(&rig, "Colead effort", "max");
    wait_prompt(&rig, "Wrote config to");
    let text = std::fs::read_to_string(&rig.config).expect("interactive config");
    assert_role(&text, "lead", "codex", "gpt-6.1-sol", "low");
    assert_role(&text, "colead", "claude", "claude-opus-5-5", "max");
}

#[test]
fn terminal_accepts_recomputed_model_defaults_after_harness_changes() {
    let rig = Rig::new("roles-terminal-default", &["claude", "codex"]);
    terminal(&rig, &[]);
    answer(&rig, "Lead harness", "codex");
    answer(&rig, "Lead model", "");
    answer(&rig, "Lead effort", "");
    answer(&rig, "Colead harness", "claude");
    answer(&rig, "Colead model", "");
    answer(&rig, "Colead effort", "");
    wait_prompt(&rig, "Wrote config to");
    let text = std::fs::read_to_string(&rig.config).expect("interactive config");
    assert_role(&text, "lead", "codex", "gpt-6.1-sol", "high");
    assert_role(&text, "colead", "claude", "claude-opus-5-5", "xhigh");
}

#[test]
fn terminal_can_choose_solo_and_skip_the_colead_model_and_effort() {
    let rig = Rig::new("roles-terminal-solo", &["claude", "codex"]);
    terminal(&rig, &["--lead", "fablex"]);
    answer(&rig, "Colead harness", "solo");
    wait_prompt(&rig, "Wrote config to");
    let text = std::fs::read_to_string(&rig.config).expect("interactive config");
    let config = ae::config::parse_identity(&text).expect("INI");
    assert!(!config.roster.iter().any(|(role, _)| role == "colead"));
    assert!(text.contains("layout = lead-solo\n"));
}

#[test]
fn role_choices_use_existing_proposal_and_force_backup_rules() {
    let rig = Rig::new("roles-proposal", &["claude", "codex"]);
    std::fs::create_dir_all(rig.config.parent().expect("config parent")).expect("parent");
    std::fs::write(&rig.config, b"original\n").expect("existing config");
    let args = [
        "init",
        "--yes",
        "--lead-effort",
        "low",
        "--colead-effort",
        "high",
    ];
    let (code, out, err) = rig.run(&args);
    assert_eq!(code, Some(0), "{out}\n{err}");
    assert_eq!(std::fs::read(&rig.config).expect("original"), b"original\n");
    let proposed =
        std::fs::read_to_string(rig.config.with_file_name("config.proposed")).expect("proposal");
    assert_role(&proposed, "lead", "claude", "claude-opus-5-5", "low");
    assert_role(&proposed, "colead", "codex", "gpt-6.1-sol", "high");
    let mut forced = args.to_vec();
    forced.push("--force");
    let text = written(&rig, &forced);
    assert_role(&text, "lead", "claude", "claude-opus-5-5", "low");
    assert_role(&text, "colead", "codex", "gpt-6.1-sol", "high");
    let backups: Vec<_> = std::fs::read_dir(rig.config.parent().expect("parent"))
        .expect("files")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "bak"))
        .collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(std::fs::read(&backups[0]).expect("backup"), b"original\n");
}
