//! The helper LINKS and the pane's own `_run`, proven black-box.
//!
//! Both subjects only exist as processes. A helper is a symlink whose identity
//! is `argv[0]`, so proving it means EXECUTING the link rather than calling the
//! function behind it; and a launch command now exists nowhere but in the
//! `execve` the pane makes, so proving it means either running a tool that
//! reports its own argv, or asking `_run --print` for the plan it would exec.
//! No tmux is needed for either — which is the point: the plan is provable
//! without a pane.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::cli::{ae, helper, helper_by_name};

/// The record separator the fixture tool frames its argv with: the context is
/// kilobytes of prose containing every other candidate, newlines included.
const RS: char = '\u{1e}';

/// A tool that reports exactly what it was `exec`ed with, and what two
/// environment variables looked like when it got there.
const REPORTING_TOOL: &str = "#!/bin/sh\n\
     : > \"__OUT__\"\n\
     for a in \"$@\"; do printf '%s\\036' \"$a\" >> \"__OUT__\"; done\n\
     printf 'ENV\\036%s\\036%s\\036' \"${CLAUDECODE-<unset>}\" \
     \"${CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION-<unset>}\" >> \"__OUT__\"\n\
     printf 'CLAUDE_CONFIG_DIR=%s\\036CODEX_HOME=%s\\036' \
     \"${CLAUDE_CONFIG_DIR-<unset>}\" \"${CODEX_HOME-<unset>}\" >> \"__OUT__\"\n\
     printf 'HOME=%s\\036' \"${HOME-<unset>}\" >> \"__OUT__\"\n\
     if [ -n \"${AE_META_FILE-}\" ]; then \
       if grep -q '^config_home.main=' \"$AE_META_FILE\"; then \
         printf 'META_ROW=present\\036' >> \"__OUT__\"; \
       else printf 'META_ROW=missing\\036' >> \"__OUT__\"; fi; \
     fi\n";

/// One hand-built session: a config with one profile per tool, and a meta whose
/// seat names one of them.
struct Rig {
    scratch: PathBuf,
    dir: PathBuf,
    project: PathBuf,
    config: PathBuf,
    out: PathBuf,
    bin: PathBuf,
    /// The rig's own `HOME`.
    home: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let scratch = super::cli::OwnedScratch::root("run", tag).keep();
        let dir = scratch.join("sessions").join(tag);
        let project = scratch.join("project");
        let bin = scratch.join("bin");
        let home = scratch.join("home");
        for path in [&dir, &project, &bin, &home] {
            assert!(std::fs::create_dir_all(path).is_ok(), "a fixture dir");
        }
        for path in [home.join(".claude"), home.join(".codex")] {
            assert!(std::fs::create_dir_all(path).is_ok(), "a config home");
        }
        let out = scratch.join("argv");
        let mut profiles = String::from("[profiles]\n");
        for tool in [
            "claude", "codex", "gemini", "grok", "opencode", "agy", "muse",
        ] {
            let path = bin.join(tool);
            let body = REPORTING_TOOL.replace("__OUT__", &out.display().to_string());
            assert!(std::fs::write(&path, body).is_ok(), "the fixture {tool}");
            assert!(
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).is_ok(),
                "an executable fixture {tool}"
            );
            let _ = writeln!(profiles, "{tool} = \"{} --flag\"", path.display());
        }
        let config = scratch.join("config");
        assert!(
            std::fs::write(
                &config,
                format!("{profiles}\n[roster]\nlead = claude\n\n[workspace]\nmain = lead\n"),
            )
            .is_ok(),
            "a fixture config"
        );
        Self {
            scratch,
            dir,
            project,
            config,
            out,
            bin,
            home,
        }
    }

    /// Publish a meta whose `main` seat runs `profile`, with `id` recorded as
    /// its harness session (empty for the capture tools, which have none yet).
    fn seat(&self, profile: &str, id: &str) {
        self.seat_with_launch_id(profile, id, Some("tok-1"));
    }

    /// The same fixture with an optional launch-id row. An explicitly empty
    /// row is distinct from no row: the observed-model CAS must refuse both.
    fn seat_with_launch_id(&self, profile: &str, id: &str, launch_id: Option<&str>) {
        let mut body = String::new();
        for (key, value) in [
            ("mode", "local"),
            ("schema", "2"),
            ("session", "fixture"),
            ("origin", &self.project.display().to_string()),
            ("work_dir", &self.project.display().to_string()),
            ("layout", "vertical"),
            ("config", &self.config.display().to_string()),
            ("seat.main", "lead"),
            ("profile.main", profile),
        ] {
            let _ = writeln!(body, "{key}={value}");
        }
        if let Some(launch_id) = launch_id {
            let _ = writeln!(body, "launch_id.main={launch_id}");
        }
        if !id.is_empty() {
            let _ = writeln!(body, "harness_session.main={id}");
        }
        assert!(
            std::fs::write(self.dir.join("meta"), body).is_ok(),
            "a fixture meta"
        );
    }

    /// Link one helper name into the session directory.
    fn link(&self, name: &str) -> PathBuf {
        let path = self.dir.join(name);
        assert!(
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), &path).is_ok(),
            "a {name} link"
        );
        path
    }

    /// Mark the seat as having run once, which is what makes the next `_run` a
    /// RESUME.
    fn started(&self) {
        assert!(
            std::fs::write(self.dir.join("launch.main.started"), "").is_ok(),
            "a start marker"
        );
    }

    /// Append one named row to the fixture meta.
    fn append_meta(&self, row: &str) {
        let path = self.dir.join("meta");
        let mut body = std::fs::read_to_string(&path)
            .unwrap_or_else(|why| panic!("fixture meta should read: {why}"));
        body.push_str(row);
        if !body.ends_with('\n') {
            body.push('\n');
        }
        assert!(std::fs::write(path, body).is_ok(), "updated fixture meta");
    }

    /// Plant the evidence a tool's own resume probe looks for.
    fn transcript(&self, tool: &str, id: &str) {
        match tool {
            "claude" => {
                // The PHYSICAL working directory, because the probe asks
                // `getcwd(2)` — which is what claude's own `process.cwd()`
                // asks, and on macOS `/tmp` is a symlink.
                let key: String = std::fs::canonicalize(&self.project)
                    .unwrap_or_else(|_| self.project.clone())
                    .display()
                    .to_string()
                    .chars()
                    .map(|ch| if ch == '/' { '-' } else { ch })
                    .collect();
                let dir = self.home.join(".claude/projects").join(key);
                assert!(std::fs::create_dir_all(&dir).is_ok(), "a transcript dir");
                assert!(
                    std::fs::write(dir.join(format!("{id}.jsonl")), "{}\n").is_ok(),
                    "a transcript"
                );
            }
            "codex" => {
                let dir = self.home.join(".codex/sessions/2026/09/04");
                assert!(std::fs::create_dir_all(&dir).is_ok(), "a session-log dir");
                assert!(
                    std::fs::write(dir.join(format!("rollout-{id}.jsonl")), "{}\n").is_ok(),
                    "a session log"
                );
            }
            // agy keeps ONE file per conversation, named for the id, in one
            // flat directory — so the evidence is that file and nothing else.
            "agy" => {
                let dir = self.home.join(".gemini/antigravity-cli/conversations");
                assert!(
                    std::fs::create_dir_all(&dir).is_ok(),
                    "a conversation store"
                );
                assert!(
                    std::fs::write(dir.join(format!("{id}.db")), "").is_ok(),
                    "a conversation"
                );
            }
            _ => {}
        }
    }

    /// `_run --print` for the `main` seat.
    fn plan(&self) -> String {
        let out = self.plan_raw();
        assert!(
            out.status.success(),
            "_run --print: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// `_run --print` without asserting success, for refusal proofs.
    fn plan_raw(&self) -> std::process::Output {
        ae().env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env("HOME", &self.home)
            .current_dir(&self.project)
            .args([ae::cli::RUN, "--print"])
            .arg(&self.dir)
            .arg("main")
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"))
    }

    /// The argv `--print` reports, decoded out of its JSON.
    fn planned_argv(&self) -> Vec<String> {
        let line = self.plan();
        let value =
            ae::json::parse(line.trim()).unwrap_or_else(|_| panic!("one JSON line: {line}"));
        let ae::json::Value::Obj(fields) = value else {
            panic!("an object: {line}")
        };
        let Some((_, ae::json::Value::Arr(argv))) =
            fields.into_iter().find(|(key, _)| key == "argv")
        else {
            panic!("an argv array: {line}")
        };
        argv.into_iter()
            .map(|word| match word {
                ae::json::Value::Str(text) => text,
                other => panic!("an argv word is a string, not {other:?}"),
            })
            .collect()
    }

    /// `_run` for real: it `exec`s the fixture tool, which reports its argv.
    fn exec(&self) -> (Vec<String>, String) {
        let _ = std::fs::remove_file(&self.out);
        let out = ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            // Set so the claude nesting guard has something to REMOVE.
            .env("CLAUDECODE", "1")
            .env("HOME", &self.home)
            .current_dir(&self.project)
            .arg(ae::cli::RUN)
            .arg(&self.dir)
            .arg("main")
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        assert!(
            out.status.success(),
            "_run: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let said = String::from_utf8_lossy(&out.stderr).into_owned();
        let dumped = std::fs::read_to_string(&self.out)
            .unwrap_or_else(|why| panic!("the tool should have reported its argv: {why}"));
        let argv = dumped
            .split(RS)
            .filter(|word| !word.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        (argv, said)
    }

    fn tool(&self, name: &str) -> String {
        self.bin.join(name).display().to_string()
    }

    /// Replace the config so the seat's `custom` profile runs `cmd` verbatim.
    fn only_profile(&self, cmd: &str) {
        self.profile("custom", cmd);
    }

    /// Replace the config with one named profile running `cmd` verbatim.
    fn profile(&self, profile: &str, cmd: &str) {
        assert!(
            std::fs::write(
                &self.config,
                format!(
                    "[profiles]\n{profile} = \"{cmd}\"\n\n[roster]\nlead = {profile}\n\n[workspace]\nmain = lead\n"
                ),
            )
            .is_ok(),
            "a fixture config"
        );
    }

    /// Replace one profile with a configured client for `tool` and its store.
    fn client_profile(&self, profile: &str, tool: &str, config_home: &Path) {
        assert!(
            std::fs::write(
                &self.config,
                format!(
                    "[clients]\nselected = {} config_home={}\n\n[profiles]\n{profile} = \"selected --flag\"\n\n[roster]\nlead = {profile}\n\n[workspace]\nmain = lead\n",
                    self.tool(tool),
                    config_home.display()
                ),
            )
            .is_ok(),
            "a client fixture config"
        );
    }

    /// `_run` for the `main` seat with `extra` in its environment, returning
    /// the raw result — a test about a REFUSAL cannot use a runner that
    /// asserts success.
    fn run_raw(&self, extra: &[(&str, &str)]) -> std::process::Output {
        let mut cmd = ae();
        cmd.env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env("HOME", &self.home)
            .current_dir(&self.project)
            .arg(ae::cli::RUN)
            .arg(&self.dir)
            .arg("main");
        for (name, value) in extra {
            cmd.env(name, value);
        }
        cmd.output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"))
    }

    /// Set the session directory's mode, so a test can make publishing fail.
    fn chmod(&self, mode: u32) {
        use std::os::unix::fs::PermissionsExt as _;
        assert!(
            std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(mode)).is_ok(),
            "a fixture chmod"
        );
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

// ---- the helper links -----------------------------------------------------

#[test]
fn a_link_invoked_by_path_reaches_the_core_with_its_own_session() {
    let rig = Rig::new("link");
    rig.seat("claude", "u-1");
    let memo = rig.link("memo");

    let added = helper(&memo)
        .env_remove("TMUX_PANE")
        .args(["add", "a durable finding"])
        .output()
        .unwrap_or_else(|why| panic!("the memo link should run: {why}"));
    assert!(
        added.status.success(),
        "memo add: {}",
        String::from_utf8_lossy(&added.stderr)
    );
    // THE SESSION CAME OUT OF argv[0]: nothing else on that command line names
    // a directory, and the memo landed in the one the link lives in.
    assert!(
        std::fs::read_to_string(rig.dir.join("memo.tsv"))
            .unwrap_or_default()
            .contains("a durable finding"),
        "the memo was written beside the link"
    );
    let read = helper(&memo)
        .env_remove("TMUX_PANE")
        .arg("read")
        .output()
        .unwrap_or_else(|why| panic!("the memo link should run: {why}"));
    assert!(
        String::from_utf8_lossy(&read.stdout).contains("a durable finding"),
        "and reads back through the same link"
    );
}

/// `_run` must apply the global-only identity rule too: a legacy seat profile
/// with the same name cannot shadow the command recorded in the global roster.
#[test]
fn an_orchestrator_run_ignores_a_shadowing_legacy_profile() {
    let mut rig = Rig::new("orch-shadow");
    rig.dir = rig.home.join("sessions").join("orchestrator");
    assert!(
        std::fs::create_dir_all(&rig.dir).is_ok(),
        "an orchestrator session dir"
    );
    let global = format!(
        "[profiles]\nshared = \"{} --global\"\n\n[roster]\norchestrator = shared\n\n[workspace]\nmain = orchestrator\n",
        rig.tool("claude")
    );
    assert!(
        std::fs::write(&rig.config, global).is_ok(),
        "a global config"
    );
    let seat = rig.home.join("orchestrator.config");
    let old_seat = format!(
        "[profiles]\nshared = \"{} --legacy\"\n\n[roster]\norchestrator = shared\n\n[workspace]\nmain = orchestrator\n",
        rig.tool("codex")
    );
    assert!(
        std::fs::write(&seat, old_seat).is_ok(),
        "an old seat config"
    );
    let meta = format!(
        "mode=local\nschema=2\nsession=orchestrator\norigin={}\nwork_dir={}\nlayout=vertical\nconfig={}\nlocal_config={}\nseat.main=orchestrator\nprofile.main=shared\nlaunch_id.main=tok-1\n",
        rig.project.display(),
        rig.project.display(),
        rig.config.display(),
        seat.display()
    );
    assert!(
        std::fs::write(rig.dir.join("meta"), meta).is_ok(),
        "a session meta"
    );

    let (argv, _) = rig.exec();
    assert_eq!(argv[0], "--global", "the global command ran: {argv:?}");
    assert!(
        !argv.contains(&"--legacy".to_owned()),
        "the legacy command was ignored: {argv:?}"
    );
}

#[test]
fn an_alias_link_prepends_its_own_fixed_word() {
    let rig = Rig::new("alias");
    rig.seat("claude", "u-1");
    let done = rig.link("mark-done");
    let state = rig.link("state");
    let reason = "the slice landed";

    // The SAME words through both links, and they part company on the word
    // `mark-done` inserts.
    let via_state = helper(&state)
        .env_remove("TMUX_PANE")
        .arg(reason)
        .output()
        .unwrap_or_else(|why| panic!("the state link should run: {why}"));
    assert_eq!(via_state.status.code(), Some(2), "an unknown state value");
    assert!(
        String::from_utf8_lossy(&via_state.stderr).contains("Usage: state"),
        "{}",
        String::from_utf8_lossy(&via_state.stderr)
    );

    let via_alias = helper(&done)
        .env_remove("TMUX_PANE")
        .arg(reason)
        .output()
        .unwrap_or_else(|why| panic!("the mark-done link should run: {why}"));
    assert_eq!(
        via_alias.status.code(),
        Some(1),
        "the value parsed; the pane did not"
    );
    assert!(
        String::from_utf8_lossy(&via_alias.stderr).contains("current agent identity"),
        "{}",
        String::from_utf8_lossy(&via_alias.stderr)
    );
}

#[test]
fn a_typo_alias_and_a_deprecated_one_reach_the_entries_they_alias() {
    let rig = Rig::new("aliases");
    rig.seat("claude", "u-1");
    let answer = |name: &str| {
        let out = helper(&rig.link(name))
            .env_remove("TMUX_PANE")
            .output()
            .unwrap_or_else(|why| panic!("the {name} link should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    assert_eq!(answer("peak"), answer("peek"), "peak IS peek");
    assert_eq!(
        answer("loop"),
        answer("watchdog"),
        "loop is the deprecated spelling of watchdog"
    );
}

#[test]
fn a_helper_reached_by_name_refuses_and_names_the_full_path_rule() {
    let rig = Rig::new("bare");
    rig.seat("claude", "u-1");
    rig.link("send");
    let path = format!(
        "{}:{}",
        rig.dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = helper_by_name("send")
        .env("PATH", path)
        .args(["lead", "hello"])
        .output()
        .unwrap_or_else(|why| panic!("the send link should be on PATH: {why}"));
    assert_eq!(out.status.code(), Some(2), "a usage error, not a failure");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("<session-dir>/send") && stderr.contains("session helper"),
        "the refusal states the rule: {stderr}"
    );
}

// ---- the pane's own command ------------------------------------------------

#[test]
fn each_tool_gets_the_argv_its_capability_row_promises() {
    // claude: an ae-generated id at launch, and the context on its own
    // append-style flag.
    let rig = Rig::new("claude");
    rig.seat("claude", "u-1");
    let argv = rig.planned_argv();
    assert_eq!(
        argv[..4],
        [
            rig.tool("claude"),
            "--flag".to_owned(),
            "--session-id".to_owned(),
            "u-1".to_owned()
        ]
    );
    assert_eq!(argv[4], "--append-system-prompt");
    assert!(
        argv[5].contains("You are agent lead (slot main)"),
        "{}",
        argv[5]
    );
    assert_eq!(argv.len(), 6, "{argv:?}");
    let plan = rig.plan();
    assert!(
        plan.contains(r#""env_unset":["CLAUDECODE","CLAUDE_CODE_SESSION"]"#),
        "{plan}"
    );
    assert!(
        plan.contains(r#""CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION":"0""#)
            && !plan.contains("CLAUDE_CONFIG_DIR"),
        "{plan}"
    );

    // codex: no launch-time id flag exists, so nothing is baked; the context
    // rides `developer_instructions` and a passive inline first user turn. The
    // turn itself is pinned in `codex_gets_a_marked_passive_inline_first_turn`.

    // gemini: `-i`, with the wait suffix that keeps a USER TURN from being
    // acted on.
    let rig = Rig::new("gemini");
    rig.seat("gemini", "");
    let argv = rig.planned_argv();
    assert_eq!(
        argv[..3],
        [rig.tool("gemini"), "--flag".to_owned(), "-i".to_owned()]
    );
    assert!(argv[3].contains("This is context only"), "{}", argv[3]);
    assert_eq!(argv.len(), 4, "{argv:?}");

    // agy: `-i`, gemini-shaped down to the wait suffix, because `agy --help`
    // (1.1.25, measured 2026-09-04) has no append-style system-prompt flag at
    // all — and no launch-time id flag either, so nothing is baked.
    let rig = Rig::new("agy");
    rig.seat("agy", "");
    let argv = rig.planned_argv();
    assert_eq!(
        argv[..3],
        [rig.tool("agy"), "--flag".to_owned(), "-i".to_owned()]
    );
    assert!(argv[3].contains("This is context only"), "{}", argv[3]);
    assert!(argv[3].contains("AE_AGY_LAUNCH_ID=tok-1"), "{}", argv[3]);
    assert!(
        !argv.iter().any(|word| word == "--session-id"),
        "agy takes no id at launch: {argv:?}"
    );
    assert_eq!(argv.len(), 4, "{argv:?}");

    // grok: an ae-generated id, and the context as the POSITIONAL prompt —
    // never `--system-prompt-override`, which would replace grok's own.
    let rig = Rig::new("grok");
    rig.seat("grok", "u-2");
    let argv = rig.planned_argv();
    assert_eq!(
        argv[..4],
        [
            rig.tool("grok"),
            "--flag".to_owned(),
            "--session-id".to_owned(),
            "u-2".to_owned()
        ]
    );
    assert!(argv[4].contains("This is context only"), "{}", argv[4]);
    assert!(
        !argv.iter().any(|word| word.starts_with("--system-prompt")),
        "{argv:?}"
    );
    assert_eq!(argv.len(), 5, "{argv:?}");

    // opencode: the context is a FILE named by an environment variable, so the
    // `env` prefix ae composed becomes a real environment delta and the argv
    // holds nothing but the tool.
    let rig = Rig::new("opencode");
    rig.seat("opencode", "");
    let argv = rig.planned_argv();
    assert_eq!(argv, [rig.tool("opencode"), "--flag".to_owned()]);
    let plan = rig.plan();
    assert!(plan.contains("OPENCODE_CONFIG"), "{plan}");
    assert!(
        std::fs::read_to_string(rig.dir.join("opencode.main.md"))
            .unwrap_or_default()
            .contains("You are agent lead"),
        "the instructions file the config points at is published"
    );
}

/// Does `argv` carry `wanted` as a contiguous run of words?
fn carries(argv: &[String], wanted: &[&str]) -> bool {
    wanted.is_empty() || argv.windows(wanted.len()).any(|run| run == wanted)
}

/// codex gets NO launch-time id flag, its context rides
/// `developer_instructions`, and its passive inline first user turn is ae's own
/// — so it opens with the ctx marker, never bare (bare reads as the human).
#[test]
fn codex_gets_a_marked_passive_inline_first_turn() {
    let rig = Rig::new("codex");
    rig.seat("codex", "");
    let argv = rig.planned_argv();
    assert_eq!(
        argv[..3],
        [rig.tool("codex"), "--flag".to_owned(), "-c".to_owned()]
    );
    assert!(
        argv[3].starts_with("developer_instructions=")
            && argv[3].contains("_register-sid main")
            && argv[3].contains("AE_CODEX_LAUNCH_ID=tok-1"),
        "{}",
        argv[3]
    );
    // The turn stays (codex-cli 0.153.2 writes no rollout without one) and it
    // is PASSIVE — the reason is pinned at `launch::initial_prompt_for`.
    assert!(argv[4].contains("/_register-sid main"), "{}", argv[4]);
    assert!(argv[4].contains("do not start any work"), "{}", argv[4]);
    let ctx = ae::provenance::ctx();
    assert_eq!(argv[4].lines().next(), Some(ctx.as_str()), "{}", argv[4]);
    assert_eq!(argv.len(), 5, "{argv:?}");
}

#[test]
fn a_recorded_first_message_folds_into_the_user_turn_as_one_positional() {
    for tool in ["muse", "grok", "agy", "gemini"] {
        let rig = Rig::new(&format!("fold{tool}"));
        rig.seat(tool, "");
        let header = ae::provenance::brief("lead");
        let framed = format!("{header}\ndo the fold");
        assert!(
            std::fs::write(ae::run::prompt_file(&rig.dir, "main"), &framed).is_ok(),
            "{tool}: the recorded first message"
        );
        let argv = rig.planned_argv();
        // Exactly one positional carries the whole turn: binary, the
        // fixture's --flag, the -i flag where the channel has one, the turn.
        let width = if tool == "agy" || tool == "gemini" {
            4
        } else {
            3
        };
        assert_eq!(argv.len(), width, "{tool}: {argv:?}");
        // The double-send guard: the brief occurs EXACTLY ONCE in the whole
        // composed line — folded, never also a second positional.
        let joined = argv.join("\n");
        assert_eq!(joined.matches(&header).count(), 1, "{tool}: {joined}");
        assert_eq!(joined.matches("do the fold").count(), 1, "{tool}: {joined}");
        let turn = argv.last().unwrap_or_else(|| panic!("{tool} has a turn"));
        let ctx = ae::provenance::ctx();
        assert_eq!(turn.lines().next(), Some(ctx.as_str()), "{tool}: {turn}");
        assert!(turn.contains("do the fold"), "{tool}: {turn}");
        assert!(!turn.contains("This is context only"), "{tool}: {turn}");
        assert!(turn.contains("START NOW"), "{tool}: {turn}");
    }
}

#[test]
fn a_recorded_first_message_stays_a_second_positional_for_codex() {
    let rig = Rig::new("foldcodex");
    rig.seat("codex", "");
    let header = ae::provenance::brief("lead");
    let turn = format!("{header}\nRun /tmp/x/_register-sid main once, then do the codex task");
    assert!(
        std::fs::write(ae::run::prompt_file(&rig.dir, "main"), &turn).is_ok(),
        "the recorded first message"
    );
    let argv = rig.planned_argv();
    assert_eq!(argv.len(), 5, "{argv:?}");
    assert!(
        argv[3].starts_with("developer_instructions="),
        "{}",
        argv[3]
    );
    assert!(
        !argv[3].contains("do the codex task"),
        "the brief stays out of the instructions: {}",
        argv[3]
    );
    assert_eq!(argv[4], turn, "the second positional is byte-identical");
}

#[test]
fn no_recorded_first_message_leaves_the_user_turn_byte_identical() {
    for (tag, plant) in [("foldnone", false), ("foldempty", true)] {
        let rig = Rig::new(tag);
        rig.seat("muse", "");
        if plant {
            assert!(
                std::fs::write(ae::run::prompt_file(&rig.dir, "main"), "").is_ok(),
                "an empty prompt file"
            );
        }
        let argv = rig.planned_argv();
        assert_eq!(argv.len(), 3, "{tag}: {argv:?}");
        let turn = argv.last().unwrap_or_else(|| panic!("{tag} has a turn"));
        assert!(turn.contains("This is context only"), "{tag}: {turn}");
        assert!(!turn.contains("START NOW"), "{tag}: {turn}");
    }
}

#[test]
fn a_resume_sends_no_recorded_first_message() {
    // A fallback resume starts a fresh conversation AND re-renders the
    // context turn — but the turn carries no brief.
    let rig = Rig::new("foldfallback");
    rig.seat("muse", "");
    rig.started();
    assert!(
        std::fs::write(
            ae::run::prompt_file(&rig.dir, "main"),
            format!("{}\ndo the fold", ae::provenance::brief("lead")),
        )
        .is_ok(),
        "the recorded first message"
    );
    let joined = rig.planned_argv().join("\n");
    assert!(!joined.contains("do the fold"), "{joined}");
    assert!(joined.contains("This is context only"), "{joined}");
    // An exact resume carries no turn at all.
    let rig = Rig::new("foldexact");
    rig.seat("muse", "01a09b51-c88a-7fc0-8f71-200ea396c8a7");
    rig.started();
    assert!(
        std::fs::write(
            ae::run::prompt_file(&rig.dir, "main"),
            format!("{}\ndo the fold", ae::provenance::brief("lead")),
        )
        .is_ok(),
        "the recorded first message"
    );
    let argv = rig.planned_argv();
    assert_eq!(
        argv,
        [
            rig.tool("muse"),
            "--flag".to_owned(),
            "resume".to_owned(),
            "01a09b51-c88a-7fc0-8f71-200ea396c8a7".to_owned()
        ],
        "{argv:?}"
    );
}

#[test]
fn a_nul_in_the_recorded_first_message_refuses_loud_before_any_marker() {
    let rig = Rig::new("foldnul");
    rig.seat("muse", "");
    let prompt = ae::run::prompt_file(&rig.dir, "main");
    assert!(
        std::fs::write(&prompt, "do the\0fold").is_ok(),
        "a hand-edited prompt file"
    );
    let out = rig.plan_raw();
    assert!(!out.status.success(), "the launch refuses");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("NUL"), "{stderr}");
    assert!(
        prompt.is_file(),
        "the prompt file is kept, the pane re-runnable"
    );
    assert!(
        !rig.dir.join("launch.main.started").exists(),
        "no start marker: a re-run retries the Create"
    );
}

#[test]
fn a_recorded_id_is_the_resume_target_for_every_tool() {
    // The resume form each tool's capability row promises, and the fallback it
    // offers when there is no id to resume BY. Every fallback is a
    // FRESH start — claude and grok name the new conversation with a minted
    // `--session-id`, the rest take the bare command. codex's, gemini's,
    // agy's, muse's and opencode's fallback is the plain command — there is
    // no word to look for, so its absence is the assertion.
    for (tool, exact, fallback) in [
        ("claude", &["--resume", "u-9"][..], &["--session-id"][..]),
        ("codex", &["resume", "u-9"][..], &[][..]),
        ("gemini", &["--resume", "u-9"][..], &[][..]),
        ("agy", &["--conversation", "u-9"][..], &[][..]),
        ("grok", &["--resume", "u-9"][..], &["--session-id"][..]),
        ("muse", &["resume", "u-9"][..], &[][..]),
        ("opencode", &["--session", "u-9"][..], &[][..]),
    ] {
        // WITH an id: the exact form, for every tool. grok, gemini and opencode
        // have no probe to pass, and that is not a reason to refuse their own
        // recorded conversation.
        let rig = Rig::new(&format!("res-{tool}"));
        rig.seat(tool, "u-9");
        rig.started();
        rig.transcript(tool, "u-9");
        let argv = rig.planned_argv();
        assert!(
            carries(&argv, exact),
            "{tool} resumes the id its meta records: {argv:?}"
        );
        assert!(
            !carries(&argv, fallback) || fallback.is_empty(),
            "{tool} does not also carry its fallback: {argv:?}"
        );

        // WITHOUT one: the tool's own fallback, and no half-written flag that
        // would have taken the next word as its value.
        let rig = Rig::new(&format!("nores-{tool}"));
        rig.seat(tool, "");
        rig.started();
        let argv = rig.planned_argv();
        assert!(
            carries(&argv, fallback),
            "{tool} falls back when there is no id: {argv:?}"
        );
        assert!(
            !argv.iter().any(|word| word == "u-9" || word.is_empty()),
            "{tool} never names an id it does not have: {argv:?}"
        );
        // No fallback ever resumes another seat's conversation.
        assert!(
            !argv.iter().any(|word| word == "--continue"),
            "{tool} never --continue: {argv:?}"
        );
        if tool == "gemini" {
            assert!(
                !carries(&argv, &["--resume", "latest"]),
                "gemini never --resume latest: {argv:?}"
            );
        }
        if tool == "codex" {
            assert!(
                !argv.iter().any(|word| word == "resume"),
                "codex with no id starts fresh: {argv:?}"
            );
        }
        if tool == "opencode" {
            assert!(
                !argv
                    .iter()
                    .any(|word| word == "--session" || word == "--continue"),
                "opencode with no id starts fresh: {argv:?}"
            );
        }
    }
}

/// #56 R5: a pending opencode seat resumes fresh, never `--continue`: the
/// newest session of the project is not provably its own.
#[test]
fn a_pending_opencode_seat_resumes_fresh_never_continue() {
    let rig = Rig::new("oc-fresh");
    rig.seat("opencode", "");
    rig.started();
    let argv = rig.planned_argv();
    assert!(
        !argv.iter().any(|word| word == "--continue"),
        "a pending opencode seat must never --continue: {argv:?}"
    );
    assert!(
        !argv.iter().any(|word| word == "--session"),
        "a pending opencode seat names no session: {argv:?}"
    );
    assert_eq!(argv[0], rig.tool("opencode"), "{argv:?}");
}

/// The `--session-id` value an argv carries, when it carries one.
fn session_id_of(argv: &[String]) -> Option<&str> {
    argv.iter()
        .position(|word| word == "--session-id")
        .and_then(|at| argv.get(at + 1).map(String::as_str))
}

/// A fallback's fresh start renders exactly like a Create of the same seat:
/// with no recorded prompt the two are byte-identical for the same id. Same
/// rig (the context embeds its paths); `recorded` is the harness row the
/// fixture holds — `None` for a pending seat — restored afterwards.
fn assert_fresh_is_create(rig: &Rig, recorded: Option<&str>) {
    let planned = rig.planned_argv();
    let shown = session_id_of(&planned).map(str::to_owned);
    if let Some(shown) = shown.as_deref() {
        ae::meta::rewrite(&rig.dir, "harness_session.main", Some(shown))
            .unwrap_or_else(|why| panic!("point the fixture at the shown id: {why:?}"));
    }
    std::fs::remove_file(rig.dir.join("launch.main.started"))
        .unwrap_or_else(|why| panic!("a Create renders: {why}"));
    let created = rig.planned_argv();
    if shown.is_some() {
        ae::meta::rewrite(&rig.dir, "harness_session.main", recorded)
            .unwrap_or_else(|why| panic!("restore the row: {why:?}"));
    }
    rig.started();
    assert_eq!(
        created, planned,
        "a fresh start is a Create with no first message"
    );
}

/// A seat whose recorded conversation is gone or never proven starts FRESH —
/// never another seat's newest — and names only its own new conversation.
/// The profile pins each tool's session grammar; the fallback strips it.
#[test]
fn a_fallback_starts_fresh_and_names_only_its_own_new_conversation() {
    for (tag, tool, tail, recorded, banned, mints) in [
        (
            "cl-fresh",
            "claude",
            "--continue --resume OLD --flag",
            Some("0199c0de-1111-4890-abcd-ef0123456789"),
            &["--continue", "--resume", "OLD"][..],
            true,
        ),
        (
            "grok-fresh",
            "grok",
            "-s OLD --resume OTHER -c --flag",
            None,
            &["--continue", "-c", "--resume", "-r", "OLD", "OTHER"][..],
            true,
        ),
        (
            "agy-fresh",
            "agy",
            "--conversation OLD -c --flag",
            Some("0199c0de-2222-4890-abcd-ef0123456789"),
            &["--continue", "-c", "--conversation", "--resume", "OLD"][..],
            false,
        ),
        (
            "gem-fresh",
            "gemini",
            "--resume latest --continue --flag",
            None,
            &["--continue", "--resume", "latest"][..],
            false,
        ),
    ] {
        let rig = Rig::new(tag);
        let cmd = format!("{} {tail}", rig.tool(tool));
        rig.profile("custom", &cmd);
        rig.seat("custom", recorded.unwrap_or(""));
        rig.started();
        assert_fresh_is_create(&rig, recorded);
        assert_fresh_row(&rig, tool, banned, recorded, mints);
    }
}

/// One fresh-start row, execed: the argv names nothing unproven, the notice
/// names the fresh start, and the meta rows match the argv.
fn assert_fresh_row(rig: &Rig, tool: &str, banned: &[&str], recorded: Option<&str>, mints: bool) {
    let (argv, said) = rig.exec();
    for word in banned {
        assert!(
            !argv.iter().any(|arg| arg == word),
            "{tool} never {word}: {argv:?}"
        );
    }
    if let Some(gone) = recorded {
        assert!(
            !argv.iter().any(|arg| arg == gone),
            "{tool} never the gone id: {argv:?}"
        );
    }
    let settled = std::fs::read_to_string(rig.dir.join("meta"))
        .unwrap_or_else(|why| panic!("the meta: {why}"));
    if mints {
        let minted =
            session_id_of(&argv).unwrap_or_else(|| panic!("{tool} mints a --session-id: {argv:?}"));
        assert_eq!(
            argv.iter()
                .filter(|arg| arg.as_str() == "--session-id")
                .count(),
            1,
            "{tool}: the mint is the only session id: {argv:?}"
        );
        match recorded {
            Some(gone) => assert_ne!(minted, gone, "{tool}: the mint is new: {argv:?}"),
            None => assert_eq!(minted.len(), 36, "{tool}: a uuid: {argv:?}"),
        }
        assert!(
            said.contains(&format!("no proven {tool} conversation"))
                && said.contains("starting a fresh one")
                && said.contains(minted)
                && !said.contains("instead of"),
            "{tool}: the notice names the fresh start and its id: {said}"
        );
        assert!(
            settled.contains(&format!("harness_session.main={minted}\n")),
            "{tool}: the argv id is the recorded id: {settled}"
        );
    } else {
        assert!(
            !argv.iter().any(|arg| arg == "--session-id"),
            "{tool} names no session: {argv:?}"
        );
        assert!(
            said.contains(&format!(
                "no proven {tool} conversation — starting a fresh one"
            )),
            "{tool}: the fresh start is named: {said}"
        );
        assert!(
            settled.contains("harness_session.main=pending\n"),
            "{tool}: no id to record: {settled}"
        );
    }
    match recorded {
        Some(gone) => assert!(
            settled.contains(&format!("harness_session_prior.main={gone}\n")),
            "{tool}: the gone conversation is the predecessor: {settled}"
        ),
        None => assert!(
            !settled.contains("harness_session_prior"),
            "{tool}: nothing abandoned: {settled}"
        ),
    }
}

/// `--print` renders an illustrative mint it never writes — the meta
/// is byte-identical, the real run mints its own, and the print says so.
#[test]
fn a_fallback_print_illustrates_a_mint_it_never_writes() {
    let rig = Rig::new("print-mint");
    rig.seat("claude", "");
    rig.started();
    let before = std::fs::read_to_string(rig.dir.join("meta")).expect("the meta");
    let plan = rig.plan();
    assert!(
        plan.contains("illustrative"),
        "the print says the mint is illustrative: {plan}"
    );
    let argv = rig.planned_argv();
    let shown =
        session_id_of(&argv).unwrap_or_else(|| panic!("an illustrative --session-id: {argv:?}"));
    assert_eq!(shown.len(), 36, "a uuid: {argv:?}");
    let after = std::fs::read_to_string(rig.dir.join("meta")).expect("the meta");
    assert_eq!(after, before, "--print writes nothing");
}

/// A fallback whose record fails still launches: a seat that minted goes
/// UNNAMED — never with an id no row records, and stderr names none — while
/// a seat with nothing to name starts fresh anyway, and still says so.
#[test]
fn a_fallback_with_a_failed_record_launches_without_the_unrecorded_id() {
    for (tag, tool, mints) in [
        ("unrec-claude", "claude", true),
        ("unrec-agy", "agy", false),
    ] {
        let rig = Rig::new(tag);
        rig.seat(tool, "0199c0de-1234-4890-abcd-ef0123456789");
        // A recorded but EMPTY home: the probe misses (no transcript), the
        // resume publishes nothing before the fallback record, and the chmod
        // breaks only that write.
        let home = rig.dir.join("home");
        std::fs::create_dir_all(&home).expect("a home dir");
        rig.append_meta(&format!("config_home.main={}\n", home.display()));
        rig.started();
        rig.chmod(0o555);
        let (argv, said) = rig.exec();
        rig.chmod(0o755);
        assert!(
            !argv.iter().any(|word| word == "--session-id"),
            "{tool} launches with no session id: {argv:?}"
        );
        assert!(
            said.contains("could not record the fresh conversation"),
            "{tool} reports the failed record: {said}"
        );
        if mints {
            assert!(
                said.contains("starting unnamed instead") && !said.contains("(--session-id"),
                "no id is named: {said}"
            );
        } else {
            assert!(
                said.contains("starting fresh anyway") && said.contains("starting a fresh one"),
                "still fresh, still said: {said}"
            );
        }
    }
}

/// #56 R5 (notice): the fresh resume names the unproven conversation on
/// stderr, before the exec.
#[test]
fn a_fresh_opencode_resume_names_the_unproven_conversation() {
    let rig = Rig::new("oc-fresh-say");
    rig.seat("opencode", "");
    rig.started();
    let (argv, stderr) = rig.exec();
    assert!(
        !argv.iter().any(|word| word == "--continue"),
        "a pending opencode seat must never --continue: {argv:?}"
    );
    assert!(
        stderr.contains("no proven opencode conversation"),
        "the fresh start is named: {stderr}"
    );
}

/// #56 B1: an opencode seat WITH a recorded id resumes exactly and stays
/// silent — the fresh-start notice fires only on the unproven path.
#[test]
fn a_recorded_opencode_resume_prints_no_fresh_start_notice() {
    let rig = Rig::new("oc-exact-say");
    rig.seat("opencode", "ses_mine");
    rig.started();
    let (argv, stderr) = rig.exec();
    assert!(argv.contains(&"--session".to_owned()), "{argv:?}");
    assert!(
        !stderr.contains("no proven"),
        "an exact resume stays silent: {stderr}"
    );
}

#[test]
fn a_recorded_id_with_no_start_marker_creates_for_every_tool() {
    // WHAT `ae reseat` LEAVES BEHIND, asked of every harness at once: the meta
    // names a conversation — a fresh uuid where the tool takes one at launch,
    // `pending` where it cannot — and the start marker is GONE, because the
    // slot's launch files were cleared. The MARKER is what decides
    // create-versus-resume, so every tool must CREATE here. A tool that
    // resumed on the mere presence of an id would hand the ARRIVING harness a
    // conversation it has never seen, in a store it cannot read.
    //
    // The transcript is planted deliberately: the store probe PASSES, and the
    // seat still creates. Without it the pin would only prove that a missing
    // conversation is not resumed, which is a different rule.
    for (tool, resume, fallback) in [
        ("claude", &["--resume"][..], &["--continue"][..]),
        ("codex", &["resume"][..], &[][..]),
        ("gemini", &["--resume"][..], &["latest"][..]),
        ("agy", &["--conversation"][..], &["--continue"][..]),
        ("grok", &["--resume"][..], &["--continue"][..]),
        ("muse", &["resume"][..], &[][..]),
        ("opencode", &["--session"][..], &["--continue"][..]),
    ] {
        let rig = Rig::new(&format!("nomark-{tool}"));
        rig.seat(tool, "u-9");
        rig.transcript(tool, "u-9");
        let argv = rig.planned_argv();
        assert!(
            !carries(&argv, resume),
            "{tool} must not resume without the marker: {argv:?}"
        );
        assert!(
            fallback.is_empty() || !carries(&argv, fallback),
            "{tool} must not take its resume fallback either: {argv:?}"
        );
    }
    // And the two classes that take a conversation AT LAUNCH carry the
    // recorded one on the create form, which is what makes the id ae minted
    // for the successor the id the successor actually runs under.
    for tool in ["claude", "grok"] {
        let rig = Rig::new(&format!("nomark-id-{tool}"));
        rig.seat(tool, "u-9");
        let argv = rig.planned_argv();
        assert!(
            carries(&argv, &["--session-id", "u-9"]),
            "{tool} creates ON the recorded conversation: {argv:?}"
        );
    }
}

/// Muse's exact resume is a SUBCOMMAND that parses no positional argument:
/// appending the ctx turn makes its parser fall back to TUI mode and refuse
/// the launch (`invalid TUI options: unknown argument 'resume'`). The retained
/// conversation already received the turn at creation, so the exact form
/// carries none — and only that form, because the fallback starts a FRESH
/// conversation. Grok's `--resume <uuid>` is a TUI invocation, not a
/// subcommand, and keeps its positional turn.
#[test]
fn a_subcommand_resume_carries_no_positional_context_turn() {
    let rig = Rig::new("muse-exact");
    rig.seat("muse", "u-9");
    rig.started();
    let argv = rig.planned_argv();
    assert_eq!(
        argv,
        [
            rig.tool("muse"),
            "--flag".to_owned(),
            "resume".to_owned(),
            "u-9".to_owned()
        ],
        "muse's exact resume is exactly the subcommand and the id: {argv:?}"
    );
    let ctx = ae::provenance::ctx();
    assert!(
        !argv.iter().any(|word| word.contains(ctx.as_str())),
        "muse's exact resume carries no context turn: {argv:?}"
    );

    // The FALLBACK — a fresh Muse conversation — still carries the marked,
    // passive turn, or the new conversation never learns its binding.
    let rig = Rig::new("muse-fallback");
    rig.seat("muse", "");
    rig.started();
    let argv = rig.planned_argv();
    let turn = argv.last().expect("a context turn");
    assert!(
        turn.contains(ctx.as_str()) && turn.contains("This is context only"),
        "the fallback keeps the context turn: {argv:?}"
    );
    assert!(
        !argv.iter().any(|word| word == "resume"),
        "the fallback starts fresh, with no subcommand: {argv:?}"
    );

    // Grok's exact resume is TUI mode with a positional prompt: unchanged.
    let rig = Rig::new("grok-exact");
    rig.seat("grok", "u-9");
    rig.started();
    let argv = rig.planned_argv();
    assert!(carries(&argv, &["--resume", "u-9"]), "{argv:?}");
    let turn = argv.last().expect("a context turn");
    assert!(
        turn.contains(ctx.as_str()),
        "grok's exact resume keeps its positional turn: {argv:?}"
    );
}

#[test]
fn a_probe_that_can_run_and_fails_still_falls_back() {
    // The other half of the rule: where a tool DOES leave evidence, a recorded
    // id whose conversation is gone is not a resume target either. (The
    // claude and agy gone rows live in the fresh-start table above; codex is
    // the remaining gone-probe pin.)
    let rig = Rig::new("gone-codex");
    rig.seat("codex", "u-9");
    rig.started();
    // No rollout planted: the id names a conversation that is not there.
    let argv = rig.planned_argv();
    assert!(
        !argv.iter().any(|word| word == "resume"),
        "codex starts fresh rather than resuming a missing log: {argv:?}"
    );
}

#[test]
fn a_resume_fallback_tags_the_abandoned_conversation_with_its_own_tool() {
    // The fixture above records no `agent_bin`, and that row is exactly what
    // decides the tag — so a real meta is built here. Without it the row a
    // fallback leaves says nothing about which store its conversation is in,
    // and after a seat has been moved to another CLI nobody can tell.
    let rig = Rig::new("fallback-tagged");
    let gone = "0199c0de-5678-4890-abcd-ef0123456789";
    rig.seat("claude", gone);
    let meta = rig.dir.join("meta");
    let text = std::fs::read_to_string(&meta).expect("the fixture meta");
    assert!(
        std::fs::write(&meta, format!("{text}agent_bin.main=claude\n")).is_ok(),
        "a meta that records its binary"
    );
    rig.started();
    let (argv, _) = rig.exec();
    let minted = session_id_of(&argv).unwrap_or_else(|| panic!("a minted --session-id: {argv:?}"));
    let settled = std::fs::read_to_string(&meta).expect("the meta");
    assert!(
        settled.contains(&format!("harness_session_prior.main=claude:{gone}\n")),
        "the abandoned conversation names the store it lives in: {settled}"
    );
    assert!(
        settled.contains(&format!("harness_session.main={minted}\n")),
        "the mint names the fresh conversation: {settled}"
    );
}

#[test]
fn a_resume_fallback_records_the_abandoned_id_once_and_mints_the_current_one() {
    let rig = Rig::new("fallback");
    let gone = "0199c0de-1234-4890-abcd-ef0123456789";
    rig.seat("claude", gone);
    rig.started();
    // No transcript for the id: claude's probe misses and the run falls back.
    let (argv, _) = rig.exec();
    let first = session_id_of(&argv).unwrap_or_else(|| panic!("a minted --session-id: {argv:?}"));
    let settled = std::fs::read_to_string(rig.dir.join("meta")).expect("the meta");
    assert!(
        settled.contains(&format!("harness_session_prior.main={gone}\n")),
        "the passed-over conversation is recorded: {settled}"
    );
    assert!(
        settled.contains(&format!("harness_session.main={first}\n")),
        "the mint names the fresh conversation: {settled}"
    );

    // The fake tool writes no transcript, so the minted conversation
    // is UNBORN and the next re-run abandons it and mints again — one minted
    // id per re-run into the prior list, `gone` recorded exactly once.
    let (argv, _) = rig.exec();
    let second = session_id_of(&argv).unwrap_or_else(|| panic!("a minted --session-id: {argv:?}"));
    assert_ne!(second, first, "each re-run mints again: {argv:?}");
    let again = std::fs::read_to_string(rig.dir.join("meta")).expect("the meta");
    assert!(
        again.contains(&format!("harness_session_prior.main={gone},{first}\n")),
        "the prior list gains the abandoned mint: {again}"
    );
    assert_eq!(
        again.matches(gone).count(),
        1,
        "gone recorded once: {again}"
    );
    assert!(
        again.contains(&format!("harness_session.main={second}\n")),
        "the newest mint is current: {again}"
    );

    // The churn is bounded: the prior list holds at most four, oldest first —
    // and the named mint-churn residual is that the real predecessor is evicted.
    for _ in 0..5 {
        rig.exec();
    }
    let churned = std::fs::read_to_string(rig.dir.join("meta")).expect("the meta");
    let priors = churned
        .lines()
        .find_map(|line| line.strip_prefix("harness_session_prior.main="))
        .expect("a prior row");
    assert!(
        priors.split(',').count() <= 4,
        "at most four predecessors: {churned}"
    );
    assert!(
        !priors.contains(gone),
        "the real predecessor is evicted by the churn: {churned}"
    );
}

#[test]
fn a_resume_fallback_on_a_capture_tool_publishes_a_fresh_capture_floor() {
    let rig = Rig::new("fallback-floor");
    let gone = "0199c0de-aaaa-4890-abcd-ef0123456789";
    rig.seat("codex", gone);
    let meta = rig.dir.join("meta");
    let text = std::fs::read_to_string(&meta).expect("the fixture meta");
    assert!(
        std::fs::write(
            &meta,
            format!("{text}agent_bin.main=codex\ncapture_floor.main=1700000000\n")
        )
        .is_ok(),
        "a meta that records its binary and an old floor"
    );
    rig.started();
    // No rollout planted: codex's probe misses and the run falls back.
    let (argv, _) = rig.exec();
    assert!(
        !argv.iter().any(|word| word == "resume"),
        "codex starts fresh rather than resuming a missing log: {argv:?}"
    );
    let settled = std::fs::read_to_string(&meta).expect("the meta");
    assert!(
        settled.contains(&format!("harness_session_prior.main=codex:{gone}\n")),
        "the passed-over conversation is recorded: {settled}"
    );
    assert!(
        settled.contains("harness_session.main=pending\n"),
        "the dead id is cleared to the honest unknown: {settled}"
    );
    let floor: i64 = settled
        .lines()
        .find_map(|line| {
            line.strip_prefix("capture_floor.main=")
                .and_then(|value| value.parse().ok())
        })
        .expect("a capture floor row");
    assert!(
        floor > 1_700_000_000,
        "the fallback republishes the floor: {settled}"
    );

    // A re-run with a pending id is a no-op, byte for byte — the floor
    // included.
    let (argv, _) = rig.exec();
    assert!(
        !argv.iter().any(|word| word == "resume"),
        "still a fresh start: {argv:?}"
    );
    let again = std::fs::read_to_string(&meta).expect("the meta");
    assert_eq!(again, settled, "a re-run with a pending id is a no-op");
}

#[test]
fn a_resume_fallback_on_a_tool_that_needs_no_capture_writes_no_floor() {
    let rig = Rig::new("fallback-nofloor");
    let gone = "0199c0de-bbbb-4890-abcd-ef0123456789";
    rig.seat("claude", gone);
    let meta = rig.dir.join("meta");
    let text = std::fs::read_to_string(&meta).expect("the fixture meta");
    assert!(
        std::fs::write(&meta, format!("{text}agent_bin.main=claude\n")).is_ok(),
        "a meta that records its binary"
    );
    rig.started();
    // No transcript for the id: claude's probe misses and the run falls back.
    let (argv, _) = rig.exec();
    let minted = session_id_of(&argv).unwrap_or_else(|| panic!("a minted --session-id: {argv:?}"));
    let settled = std::fs::read_to_string(&meta).expect("the meta");
    assert!(
        settled.contains(&format!("harness_session.main={minted}\n")),
        "the mint names the fresh conversation: {settled}"
    );
    assert!(
        !settled.contains("capture_floor"),
        "no floor row for a tool that needs no capture: {settled}"
    );
}

#[test]
fn a_first_run_creates_a_second_resumes_and_the_marker_is_the_difference() {
    let rig = Rig::new("twice");
    rig.seat("claude", "u-3");
    let marker = rig.dir.join("launch.main.started");
    assert!(!marker.exists(), "a fresh seat has never run");

    // FIRST RUN: the create form, and the environment deltas really applied —
    // `CLAUDECODE` was set for the ae process and is gone from the tool's.
    let (argv, said) = rig.exec();
    assert!(
        !said.contains("re-run"),
        "a first run announces no resume: {said}"
    );
    assert!(
        argv.contains(&"--session-id".to_owned()) && argv.contains(&"u-3".to_owned()),
        "{argv:?}"
    );
    let env = argv
        .iter()
        .position(|word| word == "ENV")
        .unwrap_or_else(|| panic!("the tool reports its environment: {argv:?}"));
    assert_eq!(
        argv[env + 1],
        "<unset>",
        "the nesting guard removed CLAUDECODE"
    );
    assert_eq!(argv[env + 2], "0", "and set the suggestion knob");
    assert!(
        marker.is_file(),
        "the run marked the seat before becoming the tool"
    );

    // SECOND RUN: the SAME line. The fake tool wrote no transcript, so the
    // recorded id is gone and the re-run falls back to a FRESH start — which
    // is why the marker exists: no second conversation is created beside a
    // live one, and the fresh one is named.
    let (argv, said) = rig.exec();
    assert!(said.contains(ae::run::RESUMING), "{said}");
    // Claude takes the fresh-start fallback, so its notice fires.
    assert!(said.contains("no proven claude conversation"), "{said}");
    let minted = session_id_of(&argv).unwrap_or_else(|| panic!("a minted --session-id: {argv:?}"));
    assert_ne!(
        minted, "u-3",
        "the mint is not the create-once id: {argv:?}"
    );
    assert!(
        !argv.iter().any(|word| word == "--continue"),
        "never --continue: {argv:?}"
    );
    assert!(rig.plan().contains(r#""mode":"resume""#));
}

#[test]
fn a_config_home_is_recorded_before_exec_then_survives_config_and_symlink_changes() {
    use std::os::unix::fs::symlink;

    let rig = Rig::new("config-home");
    let first = rig.scratch.join("account-a");
    let second = rig.scratch.join("account-b");
    let link = rig.home.join("client");
    std::fs::create_dir_all(&first).expect("first account");
    std::fs::create_dir_all(&second).expect("second account");
    symlink(&first, &link).expect("client link");
    let config = format!(
        "[clients]\ncc = {} config_home={}\n\n[profiles]\ncustom = \"AE_META_FILE={} cc --flag\"\n\n[roster]\nlead = custom\n\n[workspace]\nmain = lead\n",
        rig.tool("claude"),
        link.display(),
        rig.dir.join("meta").display()
    );
    std::fs::write(&rig.config, config).expect("client config");
    let id = "88888888-8888-4888-8888-888888888888";
    rig.seat("custom", id);
    let first = std::fs::canonicalize(&first).expect("canonical first account");
    let second = std::fs::canonicalize(&second).expect("canonical second account");

    let preview = rig.plan();
    assert!(preview.contains(&first.display().to_string()), "{preview}");
    assert!(
        !std::fs::read_to_string(rig.dir.join("meta"))
            .expect("meta")
            .contains("config_home.main="),
        "--print is read-only"
    );
    let (reported, said) = rig.exec();
    assert!(!said.contains("config now points"), "{said}");
    let recorded_meta = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
    assert!(
        recorded_meta.contains(&format!("config_home.main={}\n", first.display())),
        "the first start records the canonical target: {recorded_meta}"
    );
    assert!(
        reported.contains(&format!("CLAUDE_CONFIG_DIR={}", first.display())),
        "the canonical value reaches exec: {reported:?}"
    );
    assert!(
        reported.contains(&"META_ROW=present".to_owned()),
        "publication precedes exec: {reported:?}"
    );

    let key: String = std::fs::canonicalize(&rig.project)
        .expect("canonical project")
        .display()
        .to_string()
        .chars()
        .map(|ch| if ch == '/' { '-' } else { ch })
        .collect();
    let transcript = first.join("projects").join(key).join(format!("{id}.jsonl"));
    std::fs::create_dir_all(transcript.parent().expect("transcript directory"))
        .expect("transcript directory");
    std::fs::write(&transcript, "{}\n").expect("transcript");
    std::fs::remove_file(&link).expect("old link");
    symlink(&second, &link).expect("retargeted link");

    let planned = rig.planned_argv();
    assert!(carries(&planned, &["--resume", id]), "{planned:?}");
    let (reported, said) = rig.exec();
    assert!(
        said.contains(&format!("config now points claude at {}", second.display()))
            && said.contains(&format!(
                "retained conversation lives in {}",
                first.display()
            )),
        "{said}"
    );
    assert!(
        reported.contains(&format!("CLAUDE_CONFIG_DIR={}", first.display())),
        "recorded root wins after retargeting: {reported:?}"
    );
    assert!(
        !reported.contains(&format!("CLAUDE_CONFIG_DIR={}", second.display())),
        "new config root must not capture the retained conversation: {reported:?}"
    );
}

#[test]
fn a_default_claude_home_is_recorded_without_becoming_an_explicit_override() {
    let rig = Rig::new("default-config-home");
    rig.seat("claude", "");

    let plan = rig.plan();
    assert!(!plan.contains("CLAUDE_CONFIG_DIR"), "{plan}");
    let (reported, _) = rig.exec();
    assert!(
        reported.contains(&"CLAUDE_CONFIG_DIR=<unset>".to_owned()),
        "the default keeps Claude state at ~/.claude.json: {reported:?}"
    );
    let meta = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
    let expected = std::fs::canonicalize(rig.home.join(".claude")).expect("default store");
    let expected_base = std::fs::canonicalize(&rig.home).expect("default HOME");
    assert!(
        meta.contains(&format!(
            "config_home.main=implicit:{}\n",
            expected.display()
        )),
        "the probe and purge still receive the recorded store: {meta}"
    );
    assert!(
        meta.contains(&format!(
            "config_home_base.main={}\n",
            expected_base.display()
        )),
        "implicit mode records its original HOME: {meta}"
    );

    let (reported, _) = rig.exec();
    assert!(
        reported.contains(&"CLAUDE_CONFIG_DIR=<unset>".to_owned()),
        "default -> default never turns the default into an override: {reported:?}"
    );
    let custom = rig.scratch.join("later-client");
    std::fs::create_dir_all(&custom).expect("later client store");
    rig.client_profile("claude", "claude", &custom);
    let (reported, _) = rig.exec();
    assert!(
        reported.contains(&"CLAUDE_CONFIG_DIR=<unset>".to_owned()),
        "default -> client unsets the relocation variable: {reported:?}"
    );
}

#[test]
fn a_client_config_cannot_name_the_default_claude_store() {
    let rig = Rig::new("default-claude-to-same-client");
    let id = "66666666-6666-4666-8666-666666666666";
    rig.seat("claude", id);
    let (reported, _) = rig.exec();
    assert!(
        reported.contains(&"CLAUDE_CONFIG_DIR=<unset>".to_owned()),
        "first default run leaves Claude variable unset: {reported:?}"
    );
    let meta_before = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
    let expected = std::fs::canonicalize(rig.home.join(".claude")).expect("default store");
    assert!(
        meta_before.contains(&format!(
            "config_home.main=implicit:{}\n",
            expected.display()
        )),
        "first run records canonical default store: {meta_before}"
    );

    rig.client_profile("claude", "claude", &rig.home.join(".claude"));
    let _ = std::fs::remove_file(&rig.out);
    let result = rig.run_raw(&[]);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert_eq!(result.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("switches the Claude state file; use the default client"),
        "{stderr}"
    );
    assert!(!rig.out.exists(), "a refused config never execs");
    let meta_after = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
    assert_eq!(
        meta_after, meta_before,
        "resume keeps recorded metadata stable"
    );
}

#[test]
fn an_explicit_claude_client_at_default_store_stays_explicit_after_client_removal() {
    let rig = Rig::new("same-client-claude-to-default");
    let id = "77777777-7777-4777-8777-777777777777";
    let expected = std::fs::canonicalize(rig.home.join(".claude")).expect("default store");
    rig.profile(
        "claude",
        &format!(
            "CLAUDE_CONFIG_DIR=$HOME/.claude {} --flag",
            rig.tool("claude")
        ),
    );
    rig.seat("claude", id);

    let (reported, _) = rig.exec();
    assert!(
        reported.contains(&format!("CLAUDE_CONFIG_DIR={}", expected.display())),
        "explicit raw prefix reaches first exec at canonical default store: {reported:?}"
    );
    let meta_before = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
    assert!(
        meta_before.contains(&format!("config_home.main={}\n", expected.display())),
        "first run records explicit mode without an implicit prefix: {meta_before}"
    );

    rig.transcript("claude", id);
    rig.profile("claude", &format!("{} --flag", rig.tool("claude")));
    let (reported, _) = rig.exec();
    assert!(
        carries(&reported, &["--resume", id]),
        "second run resumes retained conversation: {reported:?}"
    );
    assert!(
        reported.contains(&format!("CLAUDE_CONFIG_DIR={}", expected.display())),
        "explicit -> default retains the recorded explicit variable: {reported:?}"
    );
}

#[test]
fn an_implicit_claude_home_stays_unset_after_a_raw_prefix_is_added() {
    let rig = Rig::new("default-claude-to-raw-prefix");
    let id = "88888888-7777-4777-8777-777777777777";
    rig.seat("claude", id);
    let _ = rig.exec();
    let expected = std::fs::canonicalize(rig.home.join(".claude")).expect("default store");
    let meta_before = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
    assert!(
        meta_before.contains(&format!(
            "config_home.main=implicit:{}\n",
            expected.display()
        )),
        "first run records implicit mode: {meta_before}"
    );

    rig.transcript("claude", id);
    rig.profile(
        "claude",
        &format!(
            "CLAUDE_CONFIG_DIR=$HOME/.claude {} --flag",
            rig.tool("claude")
        ),
    );
    let (reported, _) = rig.exec();
    assert!(carries(&reported, &["--resume", id]), "{reported:?}");
    assert!(
        reported.contains(&"CLAUDE_CONFIG_DIR=<unset>".to_owned()),
        "recorded implicit mode removes a later raw prefix: {reported:?}"
    );
}

#[test]
fn a_recorded_implicit_home_survives_changed_cleared_and_aliased_home() {
    use std::os::unix::fs::symlink;

    for next in ["changed", "cleared", "alias", "shared-store"] {
        let rig = Rig::new(&format!("implicit-home-{next}"));
        let id = "99999999-7777-4777-8777-777777777777";
        rig.seat("claude", id);
        let _ = rig.exec();
        rig.transcript("claude", id);

        let expected_home = std::fs::canonicalize(&rig.home).expect("canonical HOME");
        let reported_home = match next {
            "changed" => {
                let changed = rig.scratch.join("changed-home");
                std::fs::create_dir_all(changed.join(".claude")).expect("changed HOME");
                rig.profile(
                    "claude",
                    &format!("HOME={} {} --flag", changed.display(), rig.tool("claude")),
                );
                expected_home.clone()
            }
            "cleared" => {
                rig.profile("claude", &format!("env -i {} --flag", rig.tool("claude")));
                expected_home.clone()
            }
            "alias" => {
                let alias = rig.scratch.join("home-alias");
                symlink(&rig.home, &alias).expect("HOME alias");
                rig.profile(
                    "claude",
                    &format!("HOME={} {} --flag", alias.display(), rig.tool("claude")),
                );
                alias
            }
            "shared-store" => {
                let changed = rig.scratch.join("same-store-other-home");
                std::fs::create_dir_all(&changed).expect("other HOME");
                symlink(rig.home.join(".claude"), changed.join(".claude"))
                    .expect("shared default store");
                rig.profile(
                    "claude",
                    &format!("HOME={} {} --flag", changed.display(), rig.tool("claude")),
                );
                expected_home.clone()
            }
            _ => unreachable!(),
        };

        let (reported, _) = rig.exec();
        assert!(
            carries(&reported, &["--resume", id]),
            "{next}: {reported:?}"
        );
        assert!(
            reported.contains(&"CLAUDE_CONFIG_DIR=<unset>".to_owned()),
            "{next}: implicit mode keeps the variable unset: {reported:?}"
        );
        assert!(
            reported.contains(&format!("HOME={}", reported_home.display())),
            "{next}: execution selects the retained default store: {reported:?}"
        );
    }
}

#[allow(clippy::expect_used)]
fn assert_recorded_implicit_home_survives_symlinked_claude_store(label: &str, target: &str) {
    use std::os::unix::fs::symlink;

    let rig = Rig::new(&format!("implicit-home-symlinked-{label}"));
    let id = "88888888-8888-4888-8888-888888888888";
    let relocated = rig.scratch.join(target);
    std::fs::create_dir_all(&relocated).expect("relocated store");
    std::fs::remove_dir(rig.home.join(".claude")).expect("default store directory");
    symlink(&relocated, rig.home.join(".claude")).expect("symlinked default store");

    rig.seat("claude", id);
    let (first, _) = rig.exec();
    assert!(
        first.contains(&"CLAUDE_CONFIG_DIR=<unset>".to_owned()),
        "{label}: first implicit run leaves Claude variable unset: {first:?}"
    );
    let original_home = std::fs::canonicalize(&rig.home).expect("canonical original HOME");
    let recorded_store =
        std::fs::canonicalize(rig.home.join(".claude")).expect("canonical recorded store");
    let meta_before = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
    assert!(
        meta_before.contains(&format!(
            "config_home.main=implicit:{}\n",
            recorded_store.display()
        )),
        "{label}: first run records canonical implicit store: {meta_before}"
    );
    assert!(
        meta_before.contains(&format!(
            "config_home_base.main={}\n",
            original_home.display()
        )),
        "{label}: first run records canonical implicit HOME: {meta_before}"
    );
    let first_home = first
        .iter()
        .find_map(|word| word.strip_prefix("HOME="))
        .expect("first tool reports HOME");
    assert_eq!(
        std::fs::canonicalize(first_home).expect("canonical first HOME"),
        original_home,
        "{label}: first run uses original HOME"
    );
    rig.transcript("claude", id);

    let other_home = rig.scratch.join("other-home");
    std::fs::create_dir_all(other_home.join(".claude")).expect("other default store");
    rig.profile(
        "claude",
        &format!(
            "HOME={} {} --flag",
            other_home.display(),
            rig.tool("claude")
        ),
    );
    let _ = std::fs::remove_file(&rig.out);
    let second = rig.run_raw(&[]);
    let meta_after = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
    let output = std::fs::read_to_string(&rig.out).ok();
    if !second.status.success() {
        assert!(output.is_none(), "{label}: refusal must not exec tool");
        assert_eq!(
            meta_after, meta_before,
            "{label}: refusal leaves meta intact"
        );
        return;
    }
    let reported: Vec<String> = output
        .expect("successful resume executes tool")
        .split(RS)
        .filter(|word| !word.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    assert!(
        carries(&reported, &["--resume", id]),
        "{label}: second run resumes retained conversation: {reported:?}"
    );
    let reported_home = reported
        .iter()
        .find_map(|word| word.strip_prefix("HOME="))
        .expect("successful tool reports HOME");
    assert_eq!(
        std::fs::canonicalize(reported_home).expect("canonical reported HOME"),
        original_home,
        "{label}: implicit mode retains original HOME"
    );
    assert!(
        reported.contains(&"CLAUDE_CONFIG_DIR=<unset>".to_owned()),
        "{label}: implicit mode keeps the variable unset: {reported:?}"
    );
}

#[test]
fn a_recorded_implicit_home_survives_a_relocated_symlinked_claude_store() {
    assert_recorded_implicit_home_survives_symlinked_claude_store(
        "relocated-store",
        "relocated-store",
    );
}

#[test]
fn a_recorded_implicit_home_survives_a_storage_claude_symlink() {
    assert_recorded_implicit_home_survives_symlinked_claude_store(
        "storage-dot-claude",
        "storage/.claude",
    );
}

#[test]
#[allow(clippy::expect_used)]
fn a_recorded_implicit_home_refuses_or_keeps_store_after_symlink_retarget() {
    use std::os::unix::fs::symlink;

    let rig = Rig::new("implicit-home-symlink-retarget");
    let id = "88888888-9999-4999-8999-999999999999";
    let store_a = rig.scratch.join("store-a");
    let store_b = rig.scratch.join("store-b");
    std::fs::create_dir_all(&store_a).expect("store A");
    std::fs::create_dir_all(&store_b).expect("store B");
    std::fs::remove_dir(rig.home.join(".claude")).expect("default store directory");
    symlink(&store_a, rig.home.join(".claude")).expect("store A symlink");

    rig.seat("claude", id);
    let (first, _) = rig.exec();
    let original_home = std::fs::canonicalize(&rig.home).expect("canonical HOME");
    let canonical_a = std::fs::canonicalize(&store_a).expect("canonical store A");
    assert_eq!(
        std::fs::canonicalize(rig.home.join(".claude")).expect("first store"),
        canonical_a,
        "first store is A"
    );
    assert!(
        first.contains(&"CLAUDE_CONFIG_DIR=<unset>".to_owned()),
        "first env: {first:?}"
    );
    let meta_before = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
    assert!(
        meta_before.contains(&format!(
            "config_home.main=implicit:{}\n",
            canonical_a.display()
        )),
        "implicit store A recorded: {meta_before}"
    );
    assert!(
        meta_before.contains(&format!(
            "config_home_base.main={}\n",
            original_home.display()
        )),
        "base HOME recorded: {meta_before}"
    );
    rig.transcript("claude", id);

    let (unchanged, _) = rig.exec();
    assert!(
        carries(&unchanged, &["--resume", id]),
        "unchanged link resumes: {unchanged:?}"
    );
    assert!(
        unchanged.contains(&"CLAUDE_CONFIG_DIR=<unset>".to_owned()),
        "unchanged env: {unchanged:?}"
    );
    let meta_before_retarget = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");

    std::fs::remove_file(rig.home.join(".claude")).expect("remove store A symlink");
    symlink(&store_b, rig.home.join(".claude")).expect("store B symlink");
    let canonical_b = std::fs::canonicalize(rig.home.join(".claude")).expect("retargeted store");
    assert_ne!(canonical_b, canonical_a, "retargeted store differs from A");
    let refusal = format!(
        "ae: seat main: {}/.claude now resolves to {}; the retained conversation lives in {} — restore the link or end the session\n",
        original_home.display(),
        canonical_b.display(),
        canonical_a.display()
    );
    let _ = std::fs::remove_file(&rig.out);
    let preview = rig.plan_raw();
    assert!(!preview.status.success(), "retargeted preview must refuse");
    assert!(preview.stdout.is_empty(), "refused preview has no plan");
    assert_eq!(String::from_utf8_lossy(&preview.stderr), refusal);
    assert!(!rig.out.exists(), "preview never execs the tool");
    assert_eq!(
        std::fs::read_to_string(rig.dir.join("meta")).expect("meta after preview"),
        meta_before_retarget,
        "preview leaves meta byte-identical"
    );
    assert_eq!(
        std::fs::canonicalize(rig.home.join(".claude")).expect("link after preview"),
        canonical_b,
        "preview never repairs the operator's symlink"
    );

    let retargeted = rig.run_raw(&[]);
    let meta_after = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
    let output = std::fs::read_to_string(&rig.out).ok();
    assert!(!retargeted.status.success(), "retargeted link must refuse");
    assert!(output.is_none(), "refusal must not exec tool: {output:?}");
    assert_eq!(String::from_utf8_lossy(&retargeted.stderr), refusal);
    assert_eq!(
        meta_after, meta_before_retarget,
        "refusal leaves meta byte-identical"
    );
    assert_eq!(
        std::fs::canonicalize(rig.home.join(".claude")).expect("link remains retargeted"),
        canonical_b,
        "ae never repairs the operator's symlink"
    );
}

#[test]
fn a_recorded_custom_home_wins_after_the_client_is_removed_or_changed() {
    for next in ["default", "client-b"] {
        let rig = Rig::new(&format!("custom-to-{next}"));
        let first = rig.scratch.join("account-a");
        let second = rig.scratch.join("account-b");
        std::fs::create_dir_all(&first).expect("first account");
        std::fs::create_dir_all(&second).expect("second account");
        rig.client_profile("claude", "claude", &first);
        rig.seat("claude", "");
        let _ = rig.exec();
        let first = std::fs::canonicalize(first).expect("canonical first account");

        if next == "default" {
            rig.profile("claude", &format!("{} --flag", rig.tool("claude")));
        } else {
            rig.client_profile("claude", "claude", &second);
        }
        let (reported, _) = rig.exec();
        assert!(
            reported.contains(&format!("CLAUDE_CONFIG_DIR={}", first.display())),
            "client A -> {next} retains custom A: {reported:?}"
        );
    }
}

#[test]
fn a_recorded_default_codex_home_unsets_a_later_client_override() {
    let rig = Rig::new("codex-default-to-client");
    rig.seat("codex", "");
    let _ = rig.exec();
    let custom = rig.scratch.join("later-codex-client");
    std::fs::create_dir_all(&custom).expect("later codex client store");
    rig.client_profile("codex", "codex", &custom);

    let (reported, _) = rig.exec();
    assert!(
        reported.contains(&"CODEX_HOME=<unset>".to_owned()),
        "default -> client unsets CODEX_HOME too: {reported:?}"
    );
}

#[test]
fn a_recorded_home_survives_an_unresolvable_current_config() {
    use std::os::unix::fs::symlink;

    for bad_tail in ["parent", "dangling"] {
        let rig = Rig::new(&format!("recorded-vs-{bad_tail}"));
        let recorded = rig.scratch.join("recorded");
        std::fs::create_dir_all(&recorded).expect("recorded account");
        rig.client_profile("claude", "claude", &recorded);
        rig.seat("claude", "");
        let _ = rig.exec();
        let recorded = std::fs::canonicalize(recorded).expect("canonical recorded account");

        let bad = if bad_tail == "parent" {
            rig.scratch.join("missing/../account")
        } else {
            let dangling = rig.scratch.join("dangling");
            symlink(rig.scratch.join("absent-target"), &dangling).expect("dangling link");
            dangling.join("account")
        };
        rig.client_profile("claude", "claude", &bad);
        let (reported, said) = rig.exec();
        assert!(
            said.contains("current config unresolvable:"),
            "the degraded current config is visible: {said}"
        );
        assert!(
            reported.contains(&format!("CLAUDE_CONFIG_DIR={}", recorded.display())),
            "the recorded store still reaches exec: {reported:?}"
        );
    }
}

#[test]
fn a_not_yet_existing_config_home_is_recorded_from_its_canonical_ancestor() {
    let rig = Rig::new("future-config-home");
    let parent = rig.scratch.join("account-parent");
    std::fs::create_dir_all(&parent).expect("existing account parent");
    let future = parent.join("new/nested");
    rig.only_profile(&format!(
        "CLAUDE_CONFIG_DIR={} {} --flag",
        future.display(),
        rig.tool("claude")
    ));
    rig.seat("custom", "");

    let expected = std::fs::canonicalize(&parent)
        .expect("canonical parent")
        .join("new/nested");
    let (reported, _) = rig.exec();
    assert!(
        reported.contains(&format!("CLAUDE_CONFIG_DIR={}", expected.display())),
        "the reconstructed canonical path reaches exec: {reported:?}"
    );
    let meta = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
    assert!(
        meta.contains(&format!("config_home.main={}\n", expected.display())),
        "the future store is pinned as a path, not unknown: {meta}"
    );
}

#[test]
fn config_home_publication_failure_refuses_before_marker_or_exec() {
    let rig = Rig::new("config-home-write");
    rig.seat("claude", "u-1");
    rig.chmod(0o555);
    let result = rig.run_raw(&[]);
    rig.chmod(0o755);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert_eq!(result.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("could not record config_home.main before launch"),
        "{stderr}"
    );
    assert!(!rig.dir.join("launch.main.started").exists());
    assert!(!rig.out.exists(), "the tool did not run");
    let meta = std::fs::read_to_string(rig.dir.join("meta")).expect("meta survives");
    assert!(!meta.contains("config_home.main="), "{meta}");
    assert!(!meta.contains("config_home_base.main="), "{meta}");
}

#[test]
fn uncertain_recorded_config_homes_resume_exactly_and_hostile_rows_refuse() {
    let id = "99999999-9999-4999-8999-999999999999";
    for row in ["absent", "unknown"] {
        let rig = Rig::new(&format!("config-home-{row}"));
        rig.seat("claude", id);
        rig.started();
        rig.append_meta(&format!("config_home.main={row}\n"));
        let argv = rig.planned_argv();
        assert!(
            carries(&argv, &["--resume", id]),
            "{row} lets the tool answer the exact id: {argv:?}"
        );
        assert!(!argv.iter().any(|word| word == "--continue"), "{argv:?}");
    }

    for rows in [
        "config_home.main\n",
        "config_home.main=relative\n",
        "config_home.main=implicit:/store\n",
        "config_home.main=/one\nconfig_home.main=/two\n",
    ] {
        let rig = Rig::new("config-home-hostile");
        rig.seat("claude", id);
        rig.append_meta(rows);
        let result = rig.run_raw(&[]);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert_eq!(result.status.code(), Some(1), "{stderr}");
        assert!(
            stderr.contains("malformed or duplicate config_home metadata"),
            "{stderr}"
        );
        assert!(!rig.out.exists());
    }
}

#[test]
fn a_seat_that_cannot_be_launched_refuses_instead_of_execing() {
    let rig = Rig::new("refuse");
    rig.seat("claude", "u-1");
    for (slot, expected) in [
        ("worker.0", "no seat 'worker.0'"),
        ("main", "not configured on this machine"),
    ] {
        if slot == "main" {
            // The seat names a profile the config no longer defines.
            assert!(std::fs::write(&rig.config, "[profiles]\nother = \"x\"\n").is_ok());
        }
        let out = ae()
            .env_remove("TMUX_PANE")
            .arg(ae::cli::RUN)
            .arg(&rig.dir)
            .arg(slot)
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        assert_eq!(out.status.code(), Some(1), "a refusal, not a usage error");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(expected), "{stderr}");
    }
}

#[test]
fn pane_commands_quote_their_operands_and_an_optional_snapshot() {
    let line = ae::run::pane_command(
        Path::new("/opt/ae 1/ae-core"),
        Path::new("/s/tg1"),
        "spawned.0",
    );
    assert_eq!(line, "'/opt/ae 1/ae-core' _run '/s/tg1' 'spawned.0'");
    let snapshot = ae::run::pane_command_with_snapshot(
        Path::new("/opt/ae 1/ae-core"),
        Path::new("/s/tg1"),
        "spawned.0",
        "claude --model 'fable 5'",
    );
    assert_eq!(
        snapshot,
        "'/opt/ae 1/ae-core' _run --command-snapshot 'claude --model '\\''fable 5'\\''' '/s/tg1' 'spawned.0'"
    );
}

#[test]
fn a_pane_line_survives_the_upgrade_that_prunes_the_core_it_was_launched_from() {
    // Field #131: post-upgrade re-runs failed with `Unknown command:
    // ~/.ae/versions/<V>/ae-core`. Installed lines head the command link.
    let rig = Rig::new("paneline");
    rig.seat("claude", "u-9");
    rig.transcript("claude", "u-9");
    rig.started();
    // HOME doubles as install root; the session outside `HOME/.ae/sessions`
    // stands in for a MIGRATED session no longer naming vA — else no prune.
    let home = &rig.home;
    let ae_home = home.join(".ae");
    let v_a = ae_home.join("versions/2026.9.100");
    let core_a = v_a.join("ae-core");
    std::fs::create_dir_all(&v_a).expect("a vA version dir");
    std::fs::write(&core_a, "never executed").expect("a vA core");
    let link = home.join(".local/bin/ae");
    std::fs::create_dir_all(link.parent().expect("parent")).expect("a bin");
    std::os::unix::fs::symlink(&core_a, &link).expect("the link at vA");
    let shape_a = ae::shape::Shape::Installed {
        home: ae_home.clone(),
        version_dir: v_a.clone(),
        version: "2026.9.100".to_owned(),
    };
    let head = format!("'{}' _run ", link.display());
    let rerun = ae::run::pane_command_for(&shape_a, &core_a, &rig.dir, "main");
    assert!(rerun.starts_with(&head), "the link heads the line: {rerun}");
    let snapshot =
        ae::run::pane_command_with_snapshot_for(&shape_a, &core_a, &rig.dir, "main", "x");
    assert!(snapshot.starts_with(&head), "same head: {snapshot}");
    // vB the way the installer would: a COMPLETE version dir (the structural
    // gate refuses a re-run through anything less), then repoint, then prune.
    let v_b = ae_home.join("versions").join(ae::VERSION);
    let core_b = v_b.join("ae-core");
    std::fs::create_dir_all(&v_b).expect("a vB version dir");
    std::fs::copy(env!("CARGO_BIN_EXE_ae"), &core_b).expect("a runnable vB core");
    std::fs::write(v_b.join("install"), "not executed").expect("install");
    let manifest = format!("{s}  ae-core\n{s}  install\n", s = "0".repeat(64));
    std::fs::write(v_b.join("SHA256SUMS"), manifest).expect("a manifest");
    std::fs::remove_file(&link).expect("the old link");
    std::os::unix::fs::symlink(&core_b, &link).expect("the repoint");
    std::fs::create_dir_all(ae_home.join("sessions")).expect("a census");
    let notes = ae::migrate::prune_versions(&ae_home, &link, ae::VERSION);
    assert!(!v_a.exists(), "the prune took vA: {notes:?}");
    assert!(notes.join(" ").contains("removed"), "{notes:?}");
    // The captured line, re-run after the prune: exact resume through the link.
    let _ = std::fs::remove_file(&rig.out);
    let out = helper(Path::new("/bin/sh"))
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env("HOME", home)
        .current_dir(&rig.project)
        .arg("-c")
        .arg(&rerun)
        .output()
        .expect("the shell runs the line");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "the re-run lands: {err}");
    assert!(err.contains(ae::run::RESUMING), "a resume: {err}");
    let dumped = std::fs::read_to_string(&rig.out).expect("the tool argv");
    let reported: Vec<String> = dumped
        .split(RS)
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    assert!(carries(&reported, &["--resume", "u-9"]), "{reported:?}");
    // RED on today's spelling: the baked core is gone — the field failure, pinned.
    let old = ae::run::pane_command_for(&ae::shape::Shape::Checkout, &core_a, &rig.dir, "main");
    let red = helper(Path::new("/bin/sh"))
        .env("HOME", home)
        .current_dir(&rig.project)
        .arg("-c")
        .arg(&old)
        .output()
        .expect("the shell runs the old line");
    assert_eq!(red.status.code(), Some(127), "today's spelling, post-prune");
}

// ---- the environment prefix (colead Z2 BLOCKER-1) --------------------------

#[test]
fn a_bare_leading_assignment_is_an_environment_delta_and_never_the_binary() {
    // `A=1 codex --yolo` CLASSIFIES as codex — `split_binary` has always
    // skipped an assignment word — but `_run` peeled assignments only after a
    // literal `env`, so the exec ran a binary named `A=1`.
    let rig = Rig::new("assign");
    rig.only_profile(&format!("AE_Z2_MARK=set {} --flag", rig.tool("codex")));
    rig.seat("custom", "");
    let plan = rig.plan();
    assert!(plan.contains(r#""AE_Z2_MARK":"set""#), "{plan}");
    assert_eq!(
        rig.planned_argv()[0],
        rig.tool("codex"),
        "the assignment is not the binary"
    );
    // And it really execs: pre-fix this was ENOENT on a file named `AE_Z2_MARK=set`.
    let (argv, _) = rig.exec();
    assert_eq!(argv[0], "--flag", "the tool ran and reported its own argv");
}

#[test]
fn a_quoted_leading_assignment_is_the_binary_on_both_sides() {
    // The off-diagonal the peel itself could create: `words` decodes `'A=1'` to
    // `A=1`, so a peel that re-derives assignment-shape from the VALUE assigns
    // where `lex_simple_command` — and bash — run a binary of that name.
    let rig = Rig::new("quoted");
    rig.seat("custom", "");

    rig.only_profile(&format!("'AE_Z2_MARK=set' {} --flag", rig.tool("codex")));
    let plan = rig.plan();
    assert!(
        plan.contains(r#""env_set":{}"#),
        "a quoted word assigns nothing: {plan}"
    );
    assert_eq!(
        rig.planned_argv()[0],
        "AE_Z2_MARK=set",
        "the quoted word IS the binary"
    );
    assert_eq!(
        ae::launch_cmd::lex_simple_command(&format!(
            "'AE_Z2_MARK=set' {} --flag",
            rig.tool("codex")
        ))
        .map(|parsed| parsed.binary),
        Ok("AE_Z2_MARK=set".to_owned()),
        "…which is what the validator says too"
    );
    // So the run refuses the way an exec of a missing binary refuses, rather
    // than silently launching codex with an environment the operator did not
    // ask for.
    let out = rig.run_raw(&[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("could not start AE_Z2_MARK=set"),
        "{stderr}"
    );

    // The same line WITHOUT the quotes is an ordinary assignment.
    rig.only_profile(&format!("AE_Z2_MARK=set {} --flag", rig.tool("codex")));
    let plan = rig.plan();
    assert!(plan.contains(r#""AE_Z2_MARK":"set""#), "{plan}");
    assert_eq!(rig.planned_argv()[0], rig.tool("codex"), "{plan}");
}

#[test]
fn an_env_dash_i_starts_the_tool_from_an_empty_environment() {
    // `-i` was peeled and then dropped, so a profile that asked for a clean
    // environment inherited the pane's whole one.
    let rig = Rig::new("envi");
    rig.only_profile("env -i /usr/bin/env");
    rig.seat("custom", "");
    assert!(rig.plan().contains(r#""env_clear":true"#), "{}", rig.plan());
    let out = rig.run_raw(&[("FOO_AE_Z2_LEAK", "present")]);
    assert!(
        out.status.success(),
        "_run: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let printed = String::from_utf8_lossy(&out.stdout);
    assert!(
        !printed.contains("FOO_AE_Z2_LEAK"),
        "the pane's environment must not survive `env -i`: {printed}"
    );
    assert!(
        printed.trim().is_empty(),
        "and nothing else survives it either: {printed}"
    );
}

// ---- the create-once marker (colead Z2 BLOCKER-2) --------------------------

#[test]
fn a_start_marker_that_cannot_be_published_refuses_before_the_exec() {
    // The marker is the whole create-vs-resume discriminator.
    let rig = Rig::new("marker");
    rig.seat("claude", "u-1");
    rig.append_meta(&format!(
        "config_home.main={}\n",
        std::fs::canonicalize(rig.home.join(".claude"))
            .expect("canonical config home")
            .display()
    ));
    rig.chmod(0o555);
    for run in 1..=2 {
        let out = rig.run_raw(&[]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "run {run}: {stderr}");
        assert!(
            stderr.contains("launch.main.started") && stderr.contains("refusing to launch"),
            "run {run} names the marker and the reason: {stderr}"
        );
        assert!(
            !rig.out.exists(),
            "run {run}: the tool must not have been exec'ed"
        );
    }
    rig.chmod(0o755);
    // With the directory writable again the same seat launches, once.
    let (argv, _) = rig.exec();
    assert!(argv.contains(&"u-1".to_owned()), "{argv:?}");
    assert!(
        rig.dir.join("launch.main.started").exists(),
        "and the marker is there afterwards"
    );
}

// ---- one grammar for both lexers (colead Z2 BLOCKER-3) ---------------------

#[test]
fn the_profile_read_at_run_time_is_validated_by_the_plan_time_validator() {
    // A profile edited after its session started reached the exec unvalidated.
    let rig = Rig::new("offdiag1");
    rig.seat("custom", "");
    for (profile, named) in [
        (format!("{} {{a,b}}", rig.tool("claude")), "brace expansion"),
        (format!("{} # note", rig.tool("claude")), "comment"),
    ] {
        rig.only_profile(&profile);
        let out = rig.run_raw(&[]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{profile}: {stderr}");
        assert!(
            stderr.contains("is not one simple command") && stderr.contains(named),
            "{profile}: {stderr}"
        );
        assert!(!rig.out.exists(), "{profile}: nothing was exec'ed");
    }
}

#[test]
fn a_parameter_form_the_validator_accepts_is_one_the_run_can_expand() {
    // The off-diagonal pointed the other way: `lex_simple_command` accepted
    // `${X:-default}` and the runner refused it, so a seat planned green and
    // then exited 1 in its own pane.
    let rig = Rig::new("offdiag2");
    rig.only_profile("/bin/echo ${AE_Z2_UNSET:-fallback} ${AE_Z2_SET:-fallback}");
    rig.seat("custom", "");
    assert!(ae::launch_cmd::lex_simple_command("/bin/echo ${AE_Z2_UNSET:-fallback}").is_ok());
    let out = rig.run_raw(&[("AE_Z2_SET", "chosen")]);
    assert!(
        out.status.success(),
        "_run: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "fallback chosen"
    );
}

/// An observed-model retirement must keep its rows when the identity guard is
/// empty or absent. The explicit empty row separately defeats a guard that
/// only relies on the later compare-and-swap.
#[test]
fn a_resumed_model_retirement_refuses_empty_and_missing_launch_guards() {
    for (tag, launch_id) in [
        ("empty-model-guard", Some("")),
        ("missing-model-guard", None),
    ] {
        let rig = Rig::new(tag);
        rig.profile(
            "claudefix",
            &format!("{} --model sonnet", rig.tool("claude")),
        );
        rig.seat_with_launch_id("claudefix", "sid", launch_id);
        rig.append_meta(
            "agent_bin.main=claude\nobserved_model.main=Opus 5\nobserved_model_pin.main=fable\n",
        );
        rig.started();

        let argv = rig.planned_argv();
        assert!(
            carries(&argv, &["--model", "sonnet"]),
            "the newer profile pin keeps running: {argv:?}"
        );
        let meta = std::fs::read_to_string(rig.dir.join("meta"))
            .unwrap_or_else(|why| panic!("the fixture meta should read: {why}"));
        assert!(
            meta.contains("observed_model.main=Opus 5\n")
                && meta.contains("observed_model_pin.main=fable\n"),
            "the unguarded retirement must not erase either row: {meta}"
        );
    }
}

/// The migration trap: a display label an older ae ALREADY persisted must
/// never be replayed. The adapter declares Claude's scraped text a display
/// name, so the refusal sits on the injection path and the seat resumes on the
/// profile pin. The argv is the proof, not the notice.
#[test]
fn an_already_recorded_claude_display_label_never_reaches_the_resume_argv() {
    let rig = Rig::new("claude-label-replay");
    rig.profile(
        "claudefix",
        &format!("{} --model fable", rig.tool("claude")),
    );
    rig.seat_with_launch_id("claudefix", "sid", Some("guard-1"));
    rig.append_meta(
        "agent_bin.main=claude\nobserved_model.main=Opus 5 (1M context)\nobserved_model_pin.main=fable\n",
    );
    rig.started();

    let argv = rig.planned_argv();
    assert!(
        carries(&argv, &["--model", "fable"]),
        "the profile pin is what runs: {argv:?}"
    );
    assert!(
        !argv.iter().any(|word| word.contains("Opus 5")),
        "the display label is no argv word: {argv:?}"
    );
}

/// A launch id is meta-only for tools without a marker capability. Compare the
/// whole plan because any injected-context byte is part of the command passed
/// to the tool.
#[test]
fn claude_and_grok_contexts_are_byte_identical_with_a_meta_only_launch_id() {
    for tool in ["claude", "grok"] {
        let rig = Rig::new(&format!("meta-only-context-{tool}"));
        let id = "11111111-1111-4111-8111-111111111111";

        rig.seat_with_launch_id(tool, id, None);
        let created_without = rig.plan();
        rig.seat_with_launch_id(tool, id, Some("meta-only-token"));
        let created_with = rig.plan();
        assert_eq!(
            created_with, created_without,
            "a create {tool} context must ignore its meta-only launch id"
        );

        rig.started();
        rig.seat_with_launch_id(tool, id, None);
        let resumed_without = rig.plan();
        rig.seat_with_launch_id(tool, id, Some("meta-only-token"));
        let resumed_with = rig.plan();
        if tool == "claude" {
            // The rig plants no transcript, so the resumed arm takes the
            // fresh-start fallback — and each `--print` mints the id the real
            // run would record, an illustrative mint that differs per print.
            // What is pinned here is everything BUT that word: each plan's
            // own mint is asserted for shape, then normalized away.
            let without = illustrative_mint(&resumed_without, id);
            let with = illustrative_mint(&resumed_with, id);
            assert_eq!(
                resumed_with.replace(&with, MINT_PLACEHOLDER),
                resumed_without.replace(&without, MINT_PLACEHOLDER),
                "a resumed claude context must ignore its meta-only launch id"
            );
        } else {
            // Grok resumes exactly on its recorded id, which is byte-stable.
            assert_eq!(
                resumed_with, resumed_without,
                "a resumed {tool} context must ignore its meta-only launch id"
            );
        }
    }
}

/// The placeholder a normalized plan carries where its own illustrative
/// mint was, every occurrence.
const MINT_PLACEHOLDER: &str = "00000000-0000-4000-8000-000000000000";

/// The single illustrative mint a fallback plan carries: exactly one
/// `--session-id` pair, its value a lowercase uuid, and not the recorded id.
fn illustrative_mint(plan: &str, recorded: &str) -> String {
    assert_eq!(
        plan.matches("\"--session-id\"").count(),
        1,
        "one session-id pair: {plan}"
    );
    let at = plan
        .find("\"--session-id\",\"")
        .unwrap_or_else(|| panic!("a session-id value: {plan}"))
        + "\"--session-id\",\"".len();
    let mint = plan[at..]
        .split('"')
        .next()
        .unwrap_or_else(|| panic!("a terminated value: {plan}"));
    assert!(
        mint.len() == 36
            && mint.bytes().enumerate().all(|(at, byte)| {
                let dash = matches!(at, 8 | 13 | 18 | 23);
                (dash && byte == b'-')
                    || (!dash && byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            }),
        "a lowercase uuid: {mint}"
    );
    assert_ne!(mint, recorded, "the mint is not the recorded id");
    mint.to_owned()
}
