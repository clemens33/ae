//! O6 S1a black-box acceptance; install first for behavioral RED on base.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures own their temporary filesystem and inspect real _run output"
)]
use super::cli::{OwnedScratch, ae};
use ae::events::Event;
use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
const LAUNCH: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";
const ID: &str = "abcd1234-abcd-4123-8123-123456789abc";

struct RunRig {
    scratch: OwnedScratch,
    dir: PathBuf,
    project: PathBuf,
    home: PathBuf,
}

impl RunRig {
    fn new(tag: &str, id: Option<&str>, resume: bool) -> Self {
        let scratch = OwnedScratch::root("o6a", tag);
        let dir = scratch.join("state/sessions/fixture");
        let project = scratch.join("project");
        let home = scratch.join("home");
        let bin = scratch.join("bin");
        for p in [&dir, &project, &home, &bin, &home.join(".codex")] {
            std::fs::create_dir_all(p).expect("private fixture directory");
        }
        // Test-owned POSIX fixture reports argv through the real exec door.
        let tool = bin.join("codex");
        std::fs::write(&tool, "#!/bin/sh\nprintf '%s\n' \"$@\" > \"$AE_O6_ARGV\"\n")
            .expect("fake codex");
        std::fs::set_permissions(tool, std::fs::Permissions::from_mode(0o755))
            .expect("executable fixture");
        let config = scratch.join("config");
        std::fs::write(
            &config,
            format!(
                "[profiles]\ncodex = \"{}\"\n[roster]\nlead = codex\n[workspace]\nmain = lead\n",
                bin.join("codex").display()
            ),
        )
        .expect("config");
        let mut meta = format!(
            "schema=2\nmode=local\nlayout=vertical\nsession=fixture\nsession_id={OTHER}\norigin={}\nwork_dir={}\nconfig={}\nseat.main=lead\nprofile.main=codex\nagent_bin.main=codex\nlaunch_id.main={LAUNCH}\n",
            project.display(),
            project.display(),
            config.display()
        );
        if let Some(id) = id {
            writeln!(meta, "harness_session.main={id}").expect("write fixture row");
        }
        std::fs::write(dir.join("meta"), meta).expect("meta");
        if resume {
            std::fs::write(dir.join("launch.main.started"), "").expect("start marker");
        }
        Self {
            scratch,
            dir,
            project,
            home,
        }
    }

    fn run(&self) -> String {
        let out = ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env("HOME", &self.home)
            .env("CODEX_HOME", self.home.join(".codex"))
            .env("AE_O6_ARGV", self.scratch.join("argv"))
            .env("AE_HOME", self.scratch.join("state"))
            .env("TMUX_TMPDIR", self.scratch.path())
            .current_dir(&self.project)
            .arg(ae::cli::RUN)
            .arg(&self.dir)
            .arg("main")
            .output()
            .expect("real _run");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    fn argv(&self) -> Vec<String> {
        std::fs::read_to_string(self.scratch.join("argv"))
            .expect("tool really execed")
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn records(&self) -> Vec<Event> {
        std::fs::read_to_string(self.dir.join("events.jsonl"))
            .expect("durable journal")
            .lines()
            .map(|line| Event::parse_line(line).expect("valid event"))
            .collect()
    }
}

#[test]
fn actual_run_records_create_and_exact_codex_resume_without_inventing_loss() {
    for resume in [false, true] {
        let rig = RunRig::new(if resume { "exact" } else { "create" }, Some(ID), resume);
        let store = rig.home.join(".codex/sessions/2026/10/29");
        std::fs::create_dir_all(&store).expect("store");
        std::fs::write(store.join(format!("rollout-{ID}.jsonl")), "{}\n").expect("rollout");
        let said = rig.run();
        let records = rig.records();
        let decisions: Vec<_> = records
            .iter()
            .filter(|e| e.action.starts_with("conversation-"))
            .collect();
        assert_eq!(decisions.len(), 1, "one choice per tool start");
        let r = decisions[0];
        assert_eq!(r.actor, "watchdog");
        assert_eq!(r.target.as_deref(), Some("lead"));
        assert_eq!(
            r.action,
            if resume {
                "conversation-resumed"
            } else {
                "conversation-new"
            }
        );
        let fields: Vec<_> = r
            .reference
            .as_deref()
            .expect("typed ref")
            .split(' ')
            .collect();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[1], "-");

        assert!(!said.contains("starting a fresh"), "{said}");
        if resume {
            assert_eq!(fields[2], ID);
            assert!(rig.argv().iter().any(|a| a == "resume"));
            assert!(rig.argv().iter().any(|a| a == ID));
            assert!(said.contains(ae::run::RESUMING), "{said}");
        }
    }
}

#[test]
fn actual_codex_fallback_reports_absent_empty_pending_and_deleted_rollout() {
    for (tag, id, cause) in [
        ("absent", None, "no-id"),
        ("empty", Some(""), "no-id"),
        ("pending", Some("pending"), "no-id"),
        ("deleted", Some(ID), "not-found"),
        ("unreadable", Some(ID), "unreadable"),
    ] {
        let rig = RunRig::new(tag, id, true);
        if tag == "unreadable" {
            std::fs::write(rig.home.join(".codex/sessions"), "ENOTDIR").expect("unreadable store");
        }
        let said = rig.run();
        assert!(
            said.contains("starting a fresh codex conversation"),
            "{tag}: {said}"
        );
        assert!(
            !said.contains(ae::run::RESUMING),
            "false resume claim: {said}"
        );
        assert!(
            !rig.argv().iter().any(|a| a == "resume" || a == ID),
            "fallback must execute fresh argv"
        );
        match cause {
            "no-id" => assert!(said.contains("no conversation id was recorded"), "{said}"),
            "not-found" => assert!(
                said.contains("not found") || said.contains("not-found"),
                "{said}"
            ),
            "unreadable" => assert!(said.contains("unreadable"), "{said}"),
            _ => unreachable!("literal causes"),
        }
        let records = rig.records();
        let decisions: Vec<_> = records
            .iter()
            .filter(|e| e.action.starts_with("conversation-"))
            .collect();
        assert_eq!(decisions.len(), 1, "{tag}");
        let r = decisions[0];
        assert_eq!(r.action, format!("conversation-lost-{cause}"), "{tag}");
        let fields: Vec<_> = r
            .reference
            .as_deref()
            .expect("typed ref")
            .split(' ')
            .collect();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[1], if cause == "no-id" { "-" } else { ID });
        assert_eq!(fields[2], "pending", "codex fresh capture remains pending");
        assert!(fields[0].starts_with(&format!("main:{LAUNCH}:")));
    }
}

#[test]
fn append_failure_is_reported_but_does_not_prevent_tool_exec() {
    let rig = RunRig::new("append-failure", Some("pending"), true);
    std::fs::create_dir(rig.dir.join("events.jsonl")).expect("unappendable journal");
    let said = rig.run();
    assert!(
        said.contains("could not journal the conversation decision"),
        "{said}"
    );
    assert!(
        said.contains("starting a fresh codex conversation"),
        "{said}"
    );
    assert!(!said.contains(ae::run::RESUMING), "{said}");
}
