//! Quota command surface over a hermetic vendor-cache rig.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures build and inspect real directories; the boundary is about what product code may reach"
)]

use std::path::PathBuf;

const NOW: i64 = 1_788_858_600; // 2026-09-08T09:10:00Z
const FIRST_ID: &str = "11111111-2222-4333-8444-555555555555";
const SECOND_ID: &str = "66666666-7777-4888-8999-aaaaaaaaaaaa";

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
            "solx = CODEX_HOME=\"$HOME/.codex\" codex --model sol\n",
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
        text.contains("claude · ~/.claude-work") && text.contains("unknown"),
        "{text}"
    );
    assert!(stderr.is_empty());
    let _ = std::fs::remove_dir_all(root);
}
