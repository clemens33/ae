//! O6 S2 acceptance: oracle = lead ruling r1/r1a, not the notice renderer.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned scratch fixtures inspect real _run argv, context files and journal"
)]
use super::cli::{OwnedScratch, ae};
use ae::events::Event;
use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

const LAUNCH: &str = "11111111-1111-4111-8111-111111111111";
const ID: &str = "abcd1234-abcd-4123-8123-123456789abc";
const NO_ID: &str = "CONVERSATION: your previous conversation (no recorded id) was not resumed (no-id); this start opened a fresh one.";
const GENERIC: &str = " Recover state with: ae brief 'fixture' --seat 'lead'";
const FLEET: &str = " Re-read fleet state (ae list, ae brief --all) and tell the owner.";

struct Rig {
    scratch: OwnedScratch,
    dir: PathBuf,
    project: PathBuf,
    home: PathBuf,
    tool: String,
    slot: String,
}

impl Rig {
    fn new(tag: &str, tool: &str, slot: &str, id: Option<&str>, resume: bool) -> Self {
        let scratch = OwnedScratch::root("o6c", tag);
        let dir = scratch.join("state/sessions/fixture");
        let project = scratch.join("project");
        let home = scratch.join("home");
        let bin = scratch.join("bin");
        for path in [&dir, &project, &home, &bin, &home.join(".codex")] {
            std::fs::create_dir_all(path).expect("owned directory");
        }
        let fake = bin.join(tool);
        // NUL-delimited argv preserves the actual argument boundaries and newlines.
        std::fs::write(
            &fake,
            "#!/bin/sh\nprintf '%s\\0' \"$@\" > \"$AE_O6_ARGV\"\nprintf '%s' \"$$\" > \"$AE_O6_PID\"\n",
        )
        .expect("owned fake tool");
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
            .expect("executable fake tool");
        let config = scratch.join("config");
        std::fs::write(
            &config,
            format!("[profiles]\ntest = \"{}\"\n", fake.display()),
        )
        .expect("profile");
        let mut meta = format!(
            "schema=2\nmode=local\nlayout=vertical\nsession=fixture\nsession_id={LAUNCH}\norigin={}\nwork_dir={}\nconfig={}\nseat.{slot}=lead\nprofile.{slot}=test\nagent_bin.{slot}={tool}\nlaunch_id.{slot}={LAUNCH}\n",
            project.display(),
            project.display(),
            config.display()
        );
        if let Some(id) = id {
            writeln!(meta, "harness_session.{slot}={id}").expect("id row");
        }
        std::fs::write(dir.join("meta"), meta).expect("meta");
        if resume {
            std::fs::write(dir.join(format!("launch.{slot}.started")), "").expect("resume marker");
        }
        Self {
            scratch,
            dir,
            project,
            home,
            tool: tool.to_owned(),
            slot: slot.to_owned(),
        }
    }

    fn row(&self, key: &str, value: &str) {
        let text = std::fs::read_to_string(self.dir.join("meta")).expect("meta");
        let mut lines: Vec<_> = text
            .lines()
            .filter(|line| !line.starts_with(&format!("{key}=")))
            .map(str::to_owned)
            .collect();
        lines.push(format!("{key}={value}"));
        std::fs::write(self.dir.join("meta"), format!("{}\n", lines.join("\n")))
            .expect("owned row");
    }

    fn run(&self) -> String {
        let out = ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env("HOME", &self.home)
            .env("CODEX_HOME", self.home.join(".codex"))
            .env("AE_HOME", self.scratch.join("state"))
            .env("TMUX_TMPDIR", self.scratch.path())
            .env("AE_O6_ARGV", self.scratch.join("argv"))
            .env("AE_O6_PID", self.scratch.join("pid"))
            .current_dir(&self.project)
            .arg(ae::cli::RUN)
            .arg(&self.dir)
            .arg(&self.slot)
            .output()
            .expect("real _run");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(self.scratch.join("pid").is_file(), "tool actually execed");
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    fn argv(&self) -> Vec<String> {
        std::fs::read(self.scratch.join("argv"))
            .expect("argv")
            .split(|byte| *byte == 0)
            .filter(|bytes| !bytes.is_empty())
            .map(|bytes| String::from_utf8(bytes.to_vec()).expect("UTF-8 fixture argv"))
            .collect()
    }

    fn payload(&self) -> String {
        let args = self.argv();
        match self.tool.as_str() {
            "codex" => {
                let literal = args
                    .iter()
                    .find_map(|arg| arg.strip_prefix("developer_instructions="))
                    .expect("developer context argument");
                // Fixture subset of TOML basic strings shares JSON string escapes.
                let ae::json::Value::Str(text) = ae::json::parse(literal).expect("encoded context")
                else {
                    panic!("string context")
                };
                text
            }
            "claude" => args
                .windows(2)
                .find(|pair| pair[0] == "--append-system-prompt")
                .expect("system context")[1]
                .clone(),
            "opencode" => {
                std::fs::read_to_string(self.dir.join(format!("opencode.{}.md", self.slot)))
                    .expect("actual instruction file")
            }
            _ => args
                .iter()
                .find(|arg| arg.starts_with("⟦ae:ctx⟧\n"))
                .expect("existing marked user turn")
                .clone(),
        }
    }

    fn decisions(&self) -> Vec<Event> {
        std::fs::read_to_string(self.dir.join("events.jsonl"))
            .expect("journal")
            .lines()
            .map(|line| Event::parse_line(line).expect("valid event"))
            .filter(|event| event.action.starts_with("conversation-"))
            .collect()
    }
}

fn notice(payload: &str, exact: &str) {
    assert_eq!(payload.matches("CONVERSATION:").count(), 1, "{payload}");
    let suffix = payload.split_once("CONVERSATION:").expect("notice").1;
    let after = suffix
        .strip_prefix(exact.strip_prefix("CONVERSATION:").expect("oracle prefix"))
        .unwrap_or_else(|| panic!("expected {exact:?} in {payload:?}"));
    assert!(
        after.is_empty()
            || after == "\n"
            || after.starts_with(" --- CRITICAL FIRST TASK:")
            || after.starts_with("\nAE_")
            || after.starts_with(" --- IMPORTANT:"),
        "unexpected notice tail {after:?}"
    );
    assert!(!payload.contains("you have no memory"));
}

#[test]
fn codex_loss_causes_and_prior_come_from_this_runs_decision() {
    for (tag, id, cause, prior) in [
        ("absent", None, "no-id", "(no recorded id)"),
        ("empty", Some(""), "no-id", "(no recorded id)"),
        ("pending", Some("pending"), "no-id", "(no recorded id)"),
        ("deleted", Some(ID), "not-found", "abcd1234"),
        ("unreadable", Some(ID), "unreadable", "abcd1234"),
    ] {
        let rig = Rig::new(tag, "codex", "main", id, true);
        if tag == "unreadable" {
            std::fs::write(rig.home.join(".codex/sessions"), "ENOTDIR").expect("unreadable store");
        }
        rig.run();
        let expected = format!(
            "CONVERSATION: your previous conversation {prior} was not resumed ({cause}); this start opened a fresh one.{GENERIC}"
        );
        notice(&rig.payload(), &expected);
        assert!(!rig.payload().contains("Re-read fleet state"));
        let decisions = rig.decisions();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].action, format!("conversation-lost-{cause}"));
        let pid = std::fs::read_to_string(rig.scratch.join("pid")).expect("exec pid");
        let expected_prior = if cause == "no-id" { "-" } else { ID };
        assert_eq!(
            decisions[0].reference.as_deref(),
            Some(format!("main:{LAUNCH}:{pid} {expected_prior} pending").as_str())
        );
    }
}

#[test]
fn non_loss_starts_do_not_invent_a_notice() {
    for (tag, tool, resume, action) in [
        ("create", "codex", false, "conversation-new"),
        ("resumed", "codex", true, "conversation-resumed"),
        ("unproven", "gemini", true, "conversation-unproven"),
    ] {
        let rig = Rig::new(tag, tool, "main", Some(ID), resume);
        if tool == "codex" {
            let store = rig.home.join(".codex/sessions/2026/10/29");
            std::fs::create_dir_all(&store).expect("dated store");
            std::fs::write(store.join(format!("rollout-{ID}.jsonl")), "{}\n")
                .expect("found rollout");
        }
        rig.run();
        assert!(!rig.payload().contains("CONVERSATION:"));
        assert_eq!(rig.decisions().last().expect("decision").action, action);
    }
}

#[test]
fn fallback_uses_existing_system_and_user_turn_channels() {
    for tool in ["claude", "gemini", "muse"] {
        let rig = Rig::new(tool, tool, "main", Some("pending"), true);
        rig.run();
        let text = rig.payload();
        notice(&text, &format!("{NO_ID}{GENERIC}"));
        if tool != "claude" {
            assert_eq!(text.lines().next(), Some("⟦ae:ctx⟧"));
            assert!(text.contains("Wait for the user to give you a task."));
        }
    }
}

#[test]
fn opencode_rewrites_instructions_without_stale_loss_on_the_next_start() {
    let rig = Rig::new("oc", "opencode", "main", Some("pending"), true);
    rig.run();
    notice(&rig.payload(), &format!("{NO_ID}{GENERIC}"));
    assert_eq!(
        rig.decisions().last().expect("loss").action,
        "conversation-lost-no-id"
    );
    rig.row("harness_session.main", ID);
    rig.run();
    assert!(!rig.payload().contains("CONVERSATION:"));
    assert_eq!(
        rig.decisions().last().expect("unproven resume").action,
        "conversation-unproven"
    );
}

#[test]
fn fleet_recovery_is_only_for_a_proven_orchestrator_main_seat() {
    for (tag, slot, role, fleet) in [
        ("role", "main", "true", true),
        ("worker", "spawned.0", "true", false),
        ("damaged", "main", "false", false),
        ("absent-role", "main", "", false),
        ("duplicate", "main", "true\nmeta_agent=true", false),
    ] {
        let rig = Rig::new(tag, "codex", slot, Some("pending"), true);
        if !role.is_empty() {
            rig.row("meta_agent", role);
        }
        rig.run();
        notice(
            &rig.payload(),
            &format!("{NO_ID}{}", if fleet { FLEET } else { GENERIC }),
        );
        assert_eq!(rig.payload().contains("Re-read fleet state"), fleet);
    }
}

#[test]
fn recovery_pointer_quotes_operands_and_handles_unusable_names() {
    let cases = [
        (
            "quoted",
            "old team's session",
            "lead",
            " Recover state with: ae brief 'old team'\\''s session' --seat 'lead'",
        ),
        (
            "bad-agent",
            "fixture",
            "bad agent",
            " Recover state with: ae brief 'fixture'",
        ),
        (
            "agent-tab",
            "fixture",
            "bad\tagent",
            " Recover state with: ae brief 'fixture'",
        ),
        ("session-tab", "bad\tname", "lead", ""),
    ];
    for (tag, session, agent, tail) in cases {
        let rig = Rig::new(tag, "codex", "main", Some("pending"), true);
        rig.row("session", session);
        rig.row("seat.main", agent);
        rig.run();
        notice(&rig.payload(), &format!("{NO_ID}{tail}"));
    }
}

#[test]
fn failed_journal_append_still_tells_the_execed_seat() {
    let rig = Rig::new("append", "codex", "main", Some("pending"), true);
    std::fs::create_dir(rig.dir.join("events.jsonl")).expect("unappendable journal");
    let err = rig.run();
    assert!(
        err.contains("could not journal the conversation decision"),
        "{err}"
    );
    notice(&rig.payload(), &format!("{NO_ID}{GENERIC}"));
}
