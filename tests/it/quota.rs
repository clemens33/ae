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

fn rig(tag: &str) -> PathBuf {
    let root = PathBuf::from(format!("/tmp/ae-quota-it-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join(".codex/sessions/2026/09/08"))
        .expect("quota rig directories");
    std::fs::create_dir_all(root.join("session")).expect("session directory");
    std::fs::write(
        root.join("config"),
        concat!(
            "[profiles]\n",
            "fablex = claude --model fable\n",
            "fable5 = claude --model fable\n",
            "opusx = claude --model opus\n",
            "fablework = CLAUDE_CONFIG_DIR=\"$HOME/.claude-work\" claude\n",
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
        root.join("session/meta"),
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

#[test]
fn ae_quota_renders_fixture_caches_missing_scopes_and_unsupported_clients() {
    let root = rig("golden");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let meta = ae::session::read_meta(&root.join("session")).expect("readable session meta");
    let code = ae::quota::run(
        &ae::quota::Inputs {
            home: Some(&root),
            cwd: &root,
            global: Some(&root.join("config")),
            local: None,
            meta: Some(&meta),
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
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = ae::quota::run(
        &ae::quota::Inputs {
            home: Some(&root),
            cwd: &root,
            global: Some(&root.join("config")),
            local: None,
            meta: None,
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
        text.contains("claude · unknown") && text.contains("unknown"),
        "{text}"
    );
    assert!(stderr.is_empty());
    let _ = std::fs::remove_dir_all(root);
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
        changed.join(format!("rollout-2026-09-08T09-00-00-{FIRST_ID}.jsonl")),
        changed_rollout,
    )
    .expect("changed-home rollout");
    std::fs::write(
        root.join("config"),
        "[profiles]\nchanged = CODEX_HOME=\"$HOME/.codex-b\" codex\n",
    )
    .expect("changed profile");
    std::fs::write(
        root.join("session/meta"),
        format!(
            "schema=2\nseat.main=lead\nprofile.main=changed\nharness_session.main={FIRST_ID}\nagent_bin.main=codex\n"
        ),
    )
    .expect("changed meta");
    let meta = ae::session::read_meta(&root.join("session")).expect("readable meta");
    let mut stdout = Vec::new();
    let code = ae::quota::run(
        &ae::quota::Inputs {
            home: Some(&root),
            cwd: &root,
            global: Some(&root.join("config")),
            local: None,
            meta: Some(&meta),
            now: NOW,
        },
        &mut stdout,
        &mut Vec::new(),
    )
    .expect("quota runs");
    let text = String::from_utf8(stdout).expect("UTF-8 table");
    assert_eq!(code, 0);
    assert!(text.contains("codex · unknown · unidentified"), "{text}");
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
    let session = root.join("session");
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
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("codex · ~/.codex · unidentified"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
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
    let meta = ae::session::read_meta(&root.join("session")).expect("readable meta");
    let mut stdout = Vec::new();
    ae::quota::run(
        &ae::quota::Inputs {
            home: Some(&root),
            cwd: &root,
            global: Some(&root.join("config")),
            local: None,
            meta: Some(&meta),
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
