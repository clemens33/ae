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
        let scratch = PathBuf::from(format!("/tmp/aerun.{}.{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
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
        for tool in ["claude", "codex", "gemini", "grok", "opencode", "agy"] {
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
            ("launch_id.main", "tok-1"),
        ] {
            let _ = writeln!(body, "{key}={value}");
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
        let out = ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env("HOME", &self.home)
            .current_dir(&self.project)
            .args([ae::cli::RUN, "--print"])
            .arg(&self.dir)
            .arg("main")
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        assert!(
            out.status.success(),
            "_run --print: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
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
    // rides `developer_instructions` and a passive inline first user turn.
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
    assert_eq!(argv.len(), 5, "{argv:?}");

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

#[test]
fn a_recorded_id_is_the_resume_target_for_every_tool() {
    // The resume form each tool's capability row promises, and the fallback it
    // offers when there is no id to resume BY. codex's fallback is its plain
    // command — there is no word to look for, so its absence is the assertion.
    for (tool, exact, fallback) in [
        ("claude", &["--resume", "u-9"][..], &["--continue"][..]),
        ("codex", &["resume", "u-9"][..], &[][..]),
        (
            "gemini",
            &["--resume", "u-9"][..],
            &["--resume", "latest"][..],
        ),
        ("agy", &["--conversation", "u-9"][..], &["--continue"][..]),
        ("grok", &["--resume", "u-9"][..], &["--continue"][..]),
        ("opencode", &["--session", "u-9"][..], &["--continue"][..]),
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
        if tool == "codex" {
            assert!(
                !argv.iter().any(|word| word == "resume"),
                "codex with no id starts fresh: {argv:?}"
            );
        }
    }
}

#[test]
fn a_probe_that_can_run_and_fails_still_falls_back() {
    // The other half of the rule: where a tool DOES leave evidence, a recorded
    // id whose conversation is gone is not a resume target either.
    for (tool, gone) in [
        ("claude", "--continue"),
        ("codex", "resume"),
        ("agy", "--continue"),
    ] {
        let rig = Rig::new(&format!("gone-{tool}"));
        rig.seat(tool, "u-9");
        rig.started();
        // No transcript planted: the id names a conversation that is not there.
        let argv = rig.planned_argv();
        if tool == "codex" {
            assert!(
                !argv.iter().any(|word| word == gone),
                "codex starts fresh rather than resuming a missing log: {argv:?}"
            );
        } else {
            assert!(
                argv.iter().any(|word| word == gone),
                "{tool} takes its fallback rather than a missing conversation: {argv:?}"
            );
        }
        assert!(
            !carries(&argv, &["--resume", "u-9"]) && !carries(&argv, &["--conversation", "u-9"]),
            "{tool} does not ask for what is not there: {argv:?}"
        );
    }
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

    // SECOND RUN: the SAME line, and it resumes rather than creating a second
    // conversation — which is the whole reason the marker exists.
    let (argv, said) = rig.exec();
    assert!(said.contains(ae::run::RESUMING), "{said}");
    assert!(
        !argv.contains(&"--session-id".to_owned()),
        "a re-run must not collide on a create-once id: {argv:?}"
    );
    assert!(argv.contains(&"--continue".to_owned()), "{argv:?}");
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
    assert!(
        meta.contains(&format!(
            "config_home.main=implicit:{}\n",
            expected.display()
        )),
        "the probe and purge still receive the recorded store: {meta}"
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

    for next in ["changed", "cleared", "alias"] {
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
