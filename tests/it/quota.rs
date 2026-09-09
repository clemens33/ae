//! Quota command surface over a hermetic vendor-cache rig.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures build and inspect real directories; the boundary is about what product code may reach"
)]

use std::path::PathBuf;

const NOW: i64 = 1_788_858_600; // 2026-09-08T09:10:00Z
const FIRST_ID: &str = "01a08046-1974-7352-ade3-81a786200795";
const SECOND_ID: &str = "01a08046-2000-7abc-8abc-aaaaaaaaaaaa";
const THIRD_ID: &str = "01a08046-2100-7abc-8abc-bbbbbbbbbbbb";
const FOURTH_ID: &str = "01a08046-2200-7abc-8abc-cccccccccccc";

fn rig(tag: &str) -> PathBuf {
    let root = PathBuf::from(format!("/tmp/ae-quota-it-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join(".codex/sessions/2026/09/08"))
        .expect("quota rig directories");
    std::fs::create_dir_all(root.join("sessions/session")).expect("session directory");
    std::fs::write(
        root.join("config"),
        concat!(
            "[clients]\n",
            "claude = claude\n",
            "mic = claude config_home=$HOME/.claude-mic\n",
            "codex = codex\n",
            "[profiles]\n",
            "fablex = claude --model fable\n",
            "fable5 = claude --model fable\n",
            "opusx = claude --model opus\n",
            "micx = mic --model fable\n",
            "astrax = codex --model astra\n",
            "solx = codex --model sol\n",
            "grok46 = grok --model grok-4.6\n",
            "agy = agy\n",
            "opencode = opencode\n",
            "gemini = gemini\n",
        ),
    )
    .expect("config");
    std::fs::write(
        root.join("sessions/session/meta"),
        format!(
            "schema=2\nmode=local\norigin={}\nwork_dir={}\nseat.main=lead\nprofile.main=astrax\nharness_session.main={FIRST_ID}\nagent_bin.main=codex\nseat.worker.0=reviewer\nprofile.worker.0=solx\nharness_session.worker.0={SECOND_ID}\nagent_bin.worker.0=codex\n",
            root.display(),
            root.display(),
        ),
    )
    .expect("meta");
    std::fs::write(
        root.join(".claude.json"),
        include_bytes!("../fixtures/quota/claude-cache.json"),
    )
    .expect("Claude cache");
    std::fs::create_dir_all(root.join(".claude-mic")).expect("custom Claude home");
    let custom_cache =
        String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"))
            .replace("\"percent\": 66", "\"percent\": 77");
    std::fs::write(
        root.join(".claude-mic/.claude.json"),
        custom_cache.as_bytes(),
    )
    .expect("custom Claude cache");
    for id in [FIRST_ID, SECOND_ID] {
        std::fs::write(
            root.join(format!(
                ".codex/sessions/2026/09/08/rollout-2026-09-08T09-00-00-{id}.jsonl"
            )),
            include_bytes!("../fixtures/quota/codex-rollout.jsonl"),
        )
        .expect("Codex rollout");
    }
    root
}

fn run_quota(root: &std::path::Path) -> String {
    run_quota_with_home(root, Some(root))
}

fn run_quota_with_home(root: &std::path::Path, home: Option<&std::path::Path>) -> String {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = ae::quota::run(
        &ae::quota::Inputs {
            home,
            global: Some(&root.join("config")),
            local: None,
            sessions: Some(&root.join("sessions")),
            now: NOW,
        },
        &mut stdout,
        &mut stderr,
    )
    .expect("quota runs");
    assert_eq!(code, 0);
    assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
    String::from_utf8(stdout).expect("UTF-8 table")
}

fn add_codex_session(root: &std::path::Path, name: &str, id: &str, rollout: &[u8]) {
    let session = root.join("sessions").join(name);
    std::fs::create_dir_all(&session).expect("session directory");
    std::fs::write(
        session.join("meta"),
        format!(
            "schema=2\nseat.main=lead\nprofile.main=astrax\nharness_session.main={id}\nagent_bin.main=codex\n"
        ),
    )
    .expect("session meta");
    let day = root.join(".codex/sessions/2026/09/08");
    std::fs::write(
        day.join(format!("rollout-2026-09-08T09-00-00-{id}.jsonl")),
        rollout,
    )
    .expect("Codex rollout");
}

#[test]
fn ae_quota_renders_fixture_caches_missing_scopes_and_unsupported_clients() {
    let root = rig("golden");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = ae::quota::run(
        &ae::quota::Inputs {
            home: Some(&root),
            global: Some(&root.join("config")),
            local: None,
            sessions: Some(&root.join("sessions")),
            now: NOW,
        },
        &mut stdout,
        &mut stderr,
    )
    .expect("quota runs");
    assert_eq!(code, 0);
    assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(
        String::from_utf8(stdout).expect("UTF-8 table"),
        include_str!("../fixtures/quota/expected.stdout")
    );
    let rendered = include_str!("../fixtures/quota/expected.stdout");
    assert!(rendered.lines().all(|line| line.chars().count() <= 160));
    assert_eq!(rendered.matches("unidentified").count(), 2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn absent_and_unexpected_sources_are_unknown_and_read_error() {
    let root = rig("source-errors");
    std::fs::remove_file(root.join(".claude.json")).expect("remove planted cache");
    std::fs::create_dir(root.join(".claude.json")).expect("unexpected directory source");
    std::fs::remove_file(root.join(".claude-mic/.claude.json")).expect("remove custom cache");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = ae::quota::run(
        &ae::quota::Inputs {
            home: Some(&root),
            global: Some(&root.join("config")),
            local: None,
            sessions: Some(&root.join("sessions")),
            now: NOW,
        },
        &mut stdout,
        &mut stderr,
    )
    .expect("quota runs");
    assert_eq!(code, 0);
    let text = String::from_utf8(stdout).expect("UTF-8 table");
    assert!(
        text.contains("claude · ~/.claude") && text.contains("read-error"),
        "{text}"
    );
    assert!(
        text.contains("claude · mic") && text.contains("unknown"),
        "{text}"
    );
    assert!(stderr.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn client_labels_sharing_one_home_merge_into_one_scope() {
    let root = rig("shared-client-home");
    std::fs::write(
        root.join("config"),
        concat!(
            "[clients]\n",
            "mic = claude config_home=$HOME/.claude-mic\n",
            "work = claude config_home=$HOME/.claude-mic\n",
            "[profiles]\n",
            "micx = mic\n",
            "workx = work\n",
        ),
    )
    .expect("profile config");
    let text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        text.contains("micx workx") && text.contains("claude · mic, work") && text.contains("77%"),
        "{text}"
    );
    assert_eq!(text.matches("claude · mic, work").count(), 1, "{text}");
}

#[test]
fn profile_home_assignment_selects_claude_cache_from_effective_home() {
    let root = rig("effective-home");
    let alternate = root.join("alternate");
    std::fs::create_dir_all(&alternate).expect("alternate home");
    let cache = String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"))
        .replace("\"percent\": 66", "\"percent\": 77");
    std::fs::write(alternate.join(".claude.json"), cache.as_bytes())
        .expect("alternate default cache");
    std::fs::write(
        root.join("config"),
        format!("[profiles]\nmoved = HOME={} claude\n", alternate.display()),
    )
    .expect("profile config");
    let text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        text.contains("moved")
            && text.contains("claude · ~/alternate/.claude")
            && text.contains("77%"),
        "{text}"
    );
    assert!(!text.contains("66%"), "{text}");
}

#[cfg(unix)]
#[test]
fn symlinked_client_home_and_its_target_are_one_scope() {
    use std::os::unix::fs::symlink;

    let root = rig("symlink-client-home");
    let link = root.join(".claude-mic");
    let target = root.join("claude-target");
    std::fs::remove_dir_all(&link).expect("remove planted client home");
    std::fs::create_dir_all(&target).expect("target client home");
    let cache = String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"))
        .replace("\"percent\": 66", "\"percent\": 77");
    std::fs::write(target.join(".claude.json"), cache.as_bytes()).expect("target cache");
    symlink(&target, &link).expect("client-home symlink");
    std::fs::write(
        root.join("config"),
        format!(
            "[clients]\nmic = claude config_home=$HOME/.claude-mic\ntarget = claude config_home={}\n[profiles]\nmicx = mic\ntargetx = target\n",
            target.display()
        ),
    )
    .expect("profile config");
    let text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        text.contains("micx targetx")
            && text.contains("claude · mic, target")
            && text.contains("77%"),
        "{text}"
    );
    assert_eq!(text.matches("claude · mic, target").count(), 1, "{text}");
}

#[test]
fn client_resolution_error_is_unknown_without_hiding_other_scopes() {
    let root = rig("client-resolution-error");
    let good = root.join("good-claude");
    std::fs::create_dir_all(&good).expect("good client home");
    let cache = String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"))
        .replace("\"percent\": 66", "\"percent\": 77");
    std::fs::write(good.join(".claude.json"), cache.as_bytes()).expect("good cache");
    std::fs::write(
        root.join("config"),
        format!(
            "[clients]\nbad = claude config_home=$HOME/.claude-bad\n[profiles]\nbadx = bad\ngoodx = CLAUDE_CONFIG_DIR={} claude\n",
            good.display()
        ),
    )
    .expect("profile config");
    let text = run_quota_with_home(&root, None);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        text.contains("badx")
            && text.contains("claude · bad")
            && text.contains("unknown")
            && text.contains("HOME")
            && text.contains("unavailable"),
        "{text}"
    );
    assert!(text.contains("goodx") && text.contains("77%"), "{text}");
}

#[test]
fn changed_config_home_never_relabels_a_retained_rollout() {
    let root = rig("retained-home");
    let changed = root.join(".codex-b/sessions/2026/09/08");
    std::fs::create_dir_all(&changed).expect("changed Codex home");
    let changed_rollout =
        String::from_utf8_lossy(include_bytes!("../fixtures/quota/codex-rollout.jsonl"))
            .replace("7.0", "91.0");
    std::fs::write(
        changed.join(format!("rollout-2026-09-08T09-00-00-{SECOND_ID}.jsonl")),
        changed_rollout,
    )
    .expect("changed-home rollout");
    std::fs::write(
        root.join("config"),
        "[profiles]\nchanged = CODEX_HOME=$HOME/.codex-b codex\n",
    )
    .expect("changed profile");
    std::fs::write(
        root.join("sessions/session/meta"),
        format!(
            "schema=2\nseat.main=lead\nprofile.main=changed\nharness_session.main={FIRST_ID}\nagent_bin.main=codex\n"
        ),
    )
    .expect("changed meta");
    let mut stdout = Vec::new();
    let code = ae::quota::run(
        &ae::quota::Inputs {
            home: Some(&root),
            global: Some(&root.join("config")),
            local: None,
            sessions: Some(&root.join("sessions")),
            now: NOW,
        },
        &mut stdout,
        &mut Vec::new(),
    )
    .expect("quota runs");
    let text = String::from_utf8(stdout).expect("UTF-8 table");
    assert_eq!(code, 0);
    assert!(
        text.contains("codex · ~/.codex-b · unidentified") && text.contains("(session:lead)"),
        "{text}"
    );
    assert!(!text.contains("91%") && !text.contains("7%"), "{text}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn quota_helper_is_read_only_and_starts_no_tmux_or_vendor_process() {
    let root = rig("helper-observational");
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("sentinel bin");
    let marker = root.join("invoked");
    for name in [
        "tmux", "claude", "codex", "grok", "agy", "opencode", "gemini",
    ] {
        let path = bin.join(name);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf called > '{}'\nexit 99\n",
                marker.display()
            ),
        )
        .expect("sentinel program");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("executable sentinel");
        }
    }
    let before = crate::cli::byte_tree(&root);
    let session = root.join("sessions/session");
    let out = crate::cli::ae()
        .env("HOME", &root)
        .env("AE_HOME", &root)
        .env("CONFIG_FILE", root.join("config"))
        .env("PATH", &bin)
        .current_dir(&root)
        .args(["_quota", session.to_str().expect("session path")])
        .output()
        .expect("quota helper should run");
    assert!(out.status.success(), "{:?}", out.status);
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("codex · ~/.codex · unidentified") && stdout.contains("(session:lead)"),
        "{stdout}"
    );
    assert_eq!(
        crate::cli::byte_tree(&root),
        before,
        "quota helper changed scratch HOME bytes"
    );
    assert!(!marker.exists(), "quota helper executed a sentinel program");
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn fifo_and_symlink_vendor_sources_are_read_errors_without_being_opened() {
    use std::os::unix::fs::symlink;

    let root = rig("hostile-nodes");
    std::fs::remove_file(root.join(".claude.json")).expect("remove regular Claude cache");
    crate::cli::mkfifo(&root.join(".claude.json"));
    let rollout = root.join(format!(
        ".codex/sessions/2026/09/08/rollout-2026-09-08T09-00-00-{FIRST_ID}.jsonl"
    ));
    std::fs::remove_file(&rollout).expect("remove regular Codex rollout");
    symlink(root.join("config"), &rollout).expect("rollout symlink");
    let mut stdout = Vec::new();
    ae::quota::run(
        &ae::quota::Inputs {
            home: Some(&root),
            global: Some(&root.join("config")),
            local: None,
            sessions: Some(&root.join("sessions")),
            now: NOW,
        },
        &mut stdout,
        &mut Vec::new(),
    )
    .expect("quota refuses hostile nodes without blocking");
    let text = String::from_utf8(stdout).expect("UTF-8 table");
    assert!(text.matches("read-error").count() >= 2, "{text}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn malformed_hidden_rollout_keeps_read_error_visible_after_display_cap() {
    let root = rig("malformed-hidden");
    let fixture = include_bytes!("../fixtures/quota/codex-rollout.jsonl");
    add_codex_session(&root, "session-2", THIRD_ID, fixture);
    add_codex_session(&root, "session-3", FOURTH_ID, b"{broken}\n");
    let text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);

    let control = rig("malformed-control");
    add_codex_session(&control, "session-2", THIRD_ID, b"{broken}\n");
    let control_text = run_quota(&control);
    let _ = std::fs::remove_dir_all(&control);
    assert!(control_text.contains("read-error"), "{control_text}");
    assert!(!control_text.contains("+1 rollout"), "{control_text}");
    assert!(
        text.lines().any(|line| {
            line.contains("+1 rollout not shown")
                && line.contains("1 unreadable")
                && line.contains("read-error")
        }),
        "{text}"
    );
}

#[test]
fn claude_model_display_name_controls_never_reach_quota_stdout() {
    let root = rig("claude-controls");
    let escaped = String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"))
        .replace("Fable", "\\u001b[2J\\r\\n");
    std::fs::write(root.join(".claude.json"), escaped.as_bytes()).expect("escaped cache");
    let text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        text.chars().all(|ch| ch == '\n' || !ch.is_control()),
        "{text:?}"
    );
    assert_eq!(
        text.matches('\n').count(),
        include_str!("../fixtures/quota/expected.stdout")
            .matches('\n')
            .count(),
        "vendor newlines must not add renderer lines"
    );
    assert!(text.contains("weekly_scoped ???"), "{text:?}");
    assert!(!text.contains("[2J") && !text.contains('\r') && !text.contains('\u{1b}'));
}

#[test]
fn quoted_and_raw_env_assignments_read_the_same_custom_claude_home() {
    let root = rig("claude-profile-prefix");
    let old = String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"));
    let default_cache = old
        .replace("[REDACTED]", "old-account")
        .replace("\"percent\": 66", "\"percent\": 11");
    let custom_cache = old
        .replace("[REDACTED]", "new-account")
        .replace("\"percent\": 66", "\"percent\": 77");
    std::fs::write(root.join(".claude.json"), default_cache.as_bytes()).expect("default cache");
    std::fs::create_dir_all(root.join(".claude-work")).expect("work cache dir");
    std::fs::write(
        root.join(".claude-work/.claude.json"),
        custom_cache.as_bytes(),
    )
    .expect("custom cache");
    std::fs::write(
        root.join("config"),
        "[profiles]\nquoted = env \"CLAUDE_CONFIG_DIR=$HOME/.claude-work\" claude\nraw = env CLAUDE_CONFIG_DIR=$HOME/.claude-work claude\n",
    )
    .expect("profile config");
    let text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        text.contains("quoted raw")
            && text.contains("claude · ~/.claude-work")
            && text.contains("77%"),
        "{text}"
    );
    assert!(!text.contains("11%"), "{text}");
}

#[test]
fn bare_quoted_assignment_is_an_unknown_command_not_a_home_prefix() {
    let root = rig("claude-bare-quoted");
    std::fs::write(
        root.join("config"),
        "[profiles]\ncontrol = \"CLAUDE_CONFIG_DIR=$HOME/.claude-mic\" claude\n",
    )
    .expect("profile config");
    let text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        text.contains("control")
            && text.contains("unknown · unknown")
            && text.contains("unsupported"),
        "{text}"
    );
    assert!(!text.contains("66%") && !text.contains("77%"), "{text}");
}

#[test]
fn claude_account_switch_hides_mismatched_cached_usage() {
    let root = rig("claude-account-switch");
    let fixture = String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"));
    let mismatch = fixture
        .replacen(
            "{\n",
            "{\n  \"oauthAccount\": {\"accountUuid\": \"new-account\"},\n",
            1,
        )
        .replace("[REDACTED]", "old-account")
        .replace("\"percent\": 66", "\"percent\": 11");
    let same = mismatch.replace("old-account", "new-account");
    std::fs::write(root.join(".claude.json"), same.as_bytes()).expect("same cache");
    let same_text = run_quota(&root);
    std::fs::write(root.join(".claude.json"), mismatch.as_bytes()).expect("mismatch cache");
    let mismatch_text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(same_text.contains("11%"), "{same_text}");
    assert!(!mismatch_text.contains("11%"), "{mismatch_text}");
    assert!(mismatch_text.contains("unknown"), "{mismatch_text}");
}
