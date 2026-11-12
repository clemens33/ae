//! Issue 282 diagnosis pin. The report suspected cached startup artifacts: a
//! resumed seat's injected text naming the pre-rename session and helper path.
//! Oracle: the requirement that a start composes its text from the CURRENT
//! session dir and meta. `_run` on a renamed session dir must name only the new
//! session in every tool's context channel; the stale text a resumed seat sees
//! lives in its retained conversation, which `session_notice::due` tells it about.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned scratch fixtures inspect real _run argv and context files"
)]
use super::cli::{OwnedScratch, ae};
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

const LAUNCH: &str = "11111111-1111-4111-8111-111111111111";
const ID: &str = "abcd1234-abcd-4123-8123-123456789abc";
const BEFORE: &str = "renres-before";
const AFTER: &str = "renres-after";

struct Rig {
    scratch: OwnedScratch,
    sessions: PathBuf,
    project: PathBuf,
    home: PathBuf,
    slot: String,
}

impl Rig {
    fn new(tag: &str, tool: &str, slot: &str) -> Self {
        let scratch = OwnedScratch::root("renres", tag);
        let sessions = scratch.join("state/sessions");
        let dir = sessions.join(BEFORE);
        let project = scratch.join("project");
        let home = scratch.join("home");
        let bin = scratch.join("bin");
        for path in [&dir, &project, &home, &bin, &home.join(".codex")] {
            std::fs::create_dir_all(path).expect("owned directory");
        }
        let fake = bin.join(tool);
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
        let meta = format!(
            "schema=2\nmode=local\nlayout=vertical\nsession={BEFORE}\nsession_id={LAUNCH}\norigin={}\nwork_dir={}\nconfig={}\nseat.{slot}=lead\nprofile.{slot}=test\nagent_bin.{slot}={tool}\nlaunch_id.{slot}={LAUNCH}\nharness_session.{slot}={ID}\n",
            project.display(),
            project.display(),
            config.display()
        );
        std::fs::write(dir.join("meta"), meta).expect("meta");
        Self {
            scratch,
            sessions,
            project,
            home,
            slot: slot.to_owned(),
        }
    }

    fn run(&self, dir: &std::path::Path) {
        let _ = std::fs::remove_file(self.scratch.join("argv"));
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
            .arg(dir)
            .arg(&self.slot)
            .output()
            .expect("real _run");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Everything the start handed the tool: its argv, and for the tool whose
    /// context rides a file, that file.
    fn handed(&self, dir: &std::path::Path) -> String {
        let bytes = std::fs::read(self.scratch.join("argv")).expect("argv");
        let mut text = String::from_utf8_lossy(&bytes).replace('\0', "\n");
        if let Ok(ctx) = std::fs::read_to_string(dir.join(format!("opencode.{}.md", self.slot))) {
            text.push_str(&ctx);
        }
        text
    }
}

#[test]
fn a_resume_after_a_live_rename_is_composed_from_the_new_session_dir_and_meta() {
    for tool in ["claude", "codex", "gemini", "muse", "opencode"] {
        let rig = Rig::new(tool, tool, "main");
        if tool == "codex" {
            let store = rig.home.join(".codex/sessions/2026/10/29");
            std::fs::create_dir_all(&store).expect("dated store");
            std::fs::write(store.join(format!("rollout-{ID}.jsonl")), "{}\n")
                .expect("found rollout");
        }
        let old_dir = rig.sessions.join(BEFORE);
        rig.run(&old_dir);
        assert!(
            rig.handed(&old_dir).contains(BEFORE),
            "{tool}: the control start names its session"
        );
        // The live rename: the state dir moves and the meta row follows.
        let new_dir = rig.sessions.join(AFTER);
        std::fs::rename(&old_dir, &new_dir).expect("move state dir");
        ae::meta::rewrite(&new_dir, "session", Some(AFTER)).expect("session row");
        rig.run(&new_dir);
        let resumed = rig.handed(&new_dir);
        assert!(
            !resumed.contains(BEFORE),
            "{tool}: the resumed start kept the old name:\n{resumed}"
        );
        assert!(
            tool == "muse" || resumed.contains(AFTER),
            "{tool}: the resumed start names the new session:\n{resumed}"
        );
    }
}
