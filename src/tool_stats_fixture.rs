//! #288 frozen synthetic fixture plumbing. No product decision or count oracle.
#![allow(dead_code, reason = "shared by independent acceptance modules")]
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned acceptance fixture IO"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) const S: &str = "11111111-1111-4111-8111-111111111111";
pub(crate) const T: &str = "22222222-2222-4222-8222-222222222222";
pub(crate) const C: &str = "01a08046-1974-7352-ade3-81a786200795";
pub(crate) const D: &str = "01a08046-1974-7352-ade3-81a786200796";
pub(crate) const PRIVATE: &str = "NEVER_RETAIN_PRIVATE_SENTINEL";
pub(crate) const CLASSES: &str = include_str!("../tests/fixtures/tool_stats/claude-classes.jsonl");
pub(crate) const CODEX: &str = include_str!("../tests/fixtures/tool_stats/codex-outcomes.jsonl");
pub(crate) const RESOURCES: &str =
    include_str!("../tests/fixtures/tool_stats/claude-resources.jsonl");

pub(crate) fn call(id: &str, message: &str, tool: &str, model: &str, input: &str) -> String {
    format!(
        "{{\"type\":\"assistant\",\"timestamp\":\"2026-11-01T00:00:01Z\",\"message\":{{\"id\":\"{message}\",\"model\":\"{model}\",\"content\":[{{\"type\":\"tool_use\",\"id\":\"{id}\",\"name\":\"{tool}\",\"input\":{input}}}]}}}}\n"
    )
}

pub(crate) fn result(id: &str, error: bool, load: Option<bool>) -> String {
    let load = load.map_or_else(String::new, |value| {
        format!(",\"toolUseResult\":{{\"success\":{value}}}")
    });
    format!(
        "{{\"type\":\"user\"{load},\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"{id}\",\"is_error\":{error},\"content\":\"{PRIVATE}\"}}]}}}}\n"
    )
}

pub(crate) fn base(id: &str, directory: &str) -> String {
    format!(
        "{{\"type\":\"user\",\"isMeta\":true,\"sourceToolUseID\":\"{id}\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"Base directory for this skill: {directory}\\n{PRIVATE}\"}}]}}}}\n"
    )
}

pub(crate) fn context(turn: &str, model: &str, effort: &str) -> String {
    format!(
        "{{\"type\":\"turn_context\",\"payload\":{{\"turn_id\":\"{turn}\",\"model\":\"{model}\",\"effort\":\"{effort}\"}}}}\n"
    )
}

pub(crate) fn codex_call(id: &str, turn: Option<&str>, name: &str) -> String {
    let turn = turn.map_or_else(String::new, |turn| {
        format!(",\"internal_chat_message_metadata_passthrough\":{{\"turn_id\":\"{turn}\"}}")
    });
    format!(
        "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call\",\"call_id\":\"{id}\",\"name\":\"{name}\",\"arguments\":\"{PRIVATE}\"{turn}}}}}\n"
    )
}

pub(crate) fn codex_result(id: &str) -> String {
    format!(
        "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call_output\",\"call_id\":\"{id}\",\"output\":\"{PRIVATE}\"}}}}\n"
    )
}

pub(crate) fn event(action: &str, summary: &str) -> String {
    format!(
        "{{\"ts\":\"2026-11-01T00:00:00Z\",\"actor\":\"lead\",\"action\":\"{action}\",\"target\":\"lead\",\"target_slot\":\"main\",\"summary\":\"{summary}\"}}\n"
    )
}

pub(crate) struct Fixture(pub(crate) PathBuf);

impl Fixture {
    pub(crate) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-toolstats-{PRIVATE}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("exclusive fixture root");
        let path = fs::canonicalize(path).expect("canonical fixture root");
        for child in ["sessions", "archive", "home", "store-a", "store-b", "work"] {
            fs::create_dir(path.join(child)).expect("fixture namespace");
        }
        Self(path)
    }

    pub(crate) fn home(&self) -> PathBuf {
        self.0.join("home")
    }

    pub(crate) fn store(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "explicit independent fixture evidence"
    )]
    pub(crate) fn meta(
        &self,
        name: &str,
        session: &str,
        adapter: &str,
        conversation: &str,
        client: Option<&str>,
        store: &str,
    ) -> String {
        let client = client.map_or_else(String::new, |label| format!("client.main={label}\n"));
        format!(
            "meta_version=3\nsession={name}\nsession_id={session}\nseat.main=lead\nprofile.main=fixture-profile\nagent_bin.main={adapter}\nconfig_home.main={}\nharness_session.main={conversation}\nwork_dir.main={}\n{client}",
            self.store(store).display(),
            self.0.join("work").display(),
        )
    }

    pub(crate) fn live(&self, name: &str, meta: &str, journal: &str) -> PathBuf {
        let directory = self.0.join("sessions").join(name);
        fs::create_dir_all(&directory).expect("live fixture directory");
        fs::write(directory.join("meta"), meta).expect("fixture meta");
        fs::write(directory.join("events.jsonl"), journal).expect("fixture journal");
        directory
    }

    pub(crate) fn claude(&self, store: &str, conversation: &str, bytes: &[u8]) -> PathBuf {
        let directory = self.store(store).join("projects").join("fixture-project");
        fs::create_dir_all(&directory).expect("Claude project");
        let path = directory.join(format!("{conversation}.jsonl"));
        fs::write(&path, bytes).expect("synthetic Claude transcript");
        path
    }

    pub(crate) fn child(
        &self,
        store: &str,
        conversation: &str,
        name: &str,
        bytes: &[u8],
    ) -> PathBuf {
        let directory = self
            .store(store)
            .join("projects/fixture-project")
            .join(conversation)
            .join("subagents");
        fs::create_dir_all(&directory).expect("Claude subagent directory");
        let path = directory.join(format!("{name}.jsonl"));
        fs::write(&path, bytes).expect("synthetic Claude child");
        path
    }

    pub(crate) fn codex(&self, store: &str, conversation: &str, bytes: &[u8]) -> PathBuf {
        // C and D encode 2026-09-08 in UUIDv7. The literal day is an independent
        // fixture fact, not obtained from the product locator.
        let directory = self.store(store).join("sessions/2026/09/08");
        fs::create_dir_all(&directory).expect("Codex day directory");
        let path = directory.join(format!("rollout-fixture-{conversation}.jsonl"));
        fs::write(&path, bytes).expect("synthetic Codex transcript");
        path
    }

    pub(crate) fn archive(&self, session: &str, meta: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = self.0.join("archive").join(session);
        fs::create_dir_all(directory.join("messages")).expect("archive directory");
        for node in [
            &self.0.join("archive"),
            &directory,
            &directory.join("messages"),
        ] {
            fs::set_permissions(node, fs::Permissions::from_mode(0o700))
                .expect("private directory");
        }
        let archive_meta = format!(
            "archive_id={session}\narchive_version=1\nsource_session=fixture-old\nsource_session_id={session}\nhandover_count=0\nmemo_topic_count=0\npending_request_count=0\n"
        );
        let digest = format!(
            "- Archive ID: {session}\n## Handover (0)\n## Memo topics (0)\n## Unresolved requests (0)\n"
        );
        let (record, errors) = crate::archive::harness::from_meta(meta.as_bytes(), session);
        assert!(errors.is_empty(), "fixture archive evidence: {errors:?}");
        let harness = crate::archive::harness::encode(&record);
        for (name, bytes) in [
            ("meta", archive_meta.as_bytes()),
            ("digest.md", digest.as_bytes()),
            ("memo.tsv", b"".as_slice()),
            ("events.jsonl", b"".as_slice()),
            ("harness.tsv", harness.as_slice()),
        ] {
            let path = directory.join(name);
            fs::write(&path, bytes).expect("archive member");
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("private member");
        }
        directory
    }

    pub(crate) fn rewrite(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).expect("fixture mutation");
        assert_eq!(fs::read(path).expect("fixture mutation receipt"), bytes);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
