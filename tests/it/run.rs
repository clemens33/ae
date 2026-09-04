//! Slice Z2's two deletions, proven black-box: the helper LINKS and the pane's
//! own `_run`.
//!
//! Both subjects only exist as processes. A helper is a symlink whose identity
//! is `argv[0]`, so proving it means EXECUTING the link rather than calling the
//! function behind it; and a launch command now exists nowhere but in the
//! `execve` the pane makes, so proving it means either running a tool that
//! reports its own argv, or asking `_run --print` for the plan it would exec.
//! No tmux is needed for either — which is the point: the bash that used to
//! hold both of these is gone, and what replaced it is testable without a pane.

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
     \"${CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION-<unset>}\" >> \"__OUT__\"\n";

/// One hand-built session: a config with one profile per tool, and a meta whose
/// seat names one of them.
struct Rig {
    scratch: PathBuf,
    dir: PathBuf,
    project: PathBuf,
    config: PathBuf,
    out: PathBuf,
    bin: PathBuf,
    /// The rig's own `HOME`. The resume PROBE reads a tool's session store
    /// under it, so a test that plants one — or deliberately does not — has to
    /// own the directory it is planted in.
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
        let out = scratch.join("argv");
        let mut profiles = String::from("[profiles]\n");
        for tool in ["claude", "codex", "gemini", "grok", "opencode"] {
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
    /// RESUME. `_run` writes this itself; a test that wants only the resume
    /// half writes it directly rather than launching a tool to get it.
    fn started(&self) {
        assert!(
            std::fs::write(self.dir.join("launch.main.started"), "").is_ok(),
            "a start marker"
        );
    }

    /// Plant the evidence a tool's own resume probe looks for.
    ///
    /// Only claude and codex leave any, and where a tool leaves none there is
    /// nothing to plant — the recorded id is the whole answer.
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
            _ => {}
        }
    }

    /// `_run --print` for the `main` seat.
    fn plan(&self) -> String {
        let out = ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
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
    /// Returns that argv and whatever `_run` said before it became the tool.
    fn exec(&self) -> (Vec<String>, String) {
        let _ = std::fs::remove_file(&self.out);
        let out = ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
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
    ///
    /// The profile is what `_run` reads FRESH on every run, so this is also how
    /// a test changes one after its session has started.
    fn only_profile(&self, cmd: &str) {
        assert!(
            std::fs::write(
                &self.config,
                format!(
                    "[profiles]\ncustom = \"{cmd}\"\n\n[roster]\nlead = custom\n\n[workspace]\nmain = lead\n"
                ),
            )
            .is_ok(),
            "a fixture config"
        );
    }

    /// `_run` for the `main` seat with `extra` in its environment, returning
    /// the raw result — a test about a REFUSAL cannot use a runner that
    /// asserts success.
    fn run_raw(&self, extra: &[(&str, &str)]) -> std::process::Output {
        let mut cmd = ae();
        cmd.env_remove("TMUX")
            .env_remove("TMUX_PANE")
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

#[test]
fn an_alias_link_prepends_its_own_fixed_word() {
    let rig = Rig::new("alias");
    rig.seat("claude", "u-1");
    let done = rig.link("mark-done");
    let state = rig.link("state");
    let reason = "the slice landed";

    // The SAME words through both links, and they part company on the word
    // `mark-done` inserts. Through `state` the reason is read as the state
    // VALUE and refused as a usage error; through `mark-done` the value is
    // already `done`, the reason is a reason, and the only thing left to refuse
    // is the identity a test process outside a pane does not have.
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
    // append-style flag. The nesting guard is an ENVIRONMENT delta, not an
    // `env` word: there is no shell left to read one.
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
        plan.contains(r#""env_set":{"CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION":"0"}"#),
        "{plan}"
    );

    // codex: no launch-time id flag exists, so nothing is baked; the context
    // rides `developer_instructions` and the inline first user turn is `Go`.
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
    assert_eq!(
        argv[4], "Go",
        "codex needs a user turn to act on its instructions"
    );
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
    for (tool, gone) in [("claude", "--continue"), ("codex", "resume")] {
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
                "claude takes its cwd heuristic rather than a missing transcript: {argv:?}"
            );
        }
        assert!(
            !carries(&argv, &["--resume", "u-9"]),
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
    // conversation — which is the whole reason the marker exists. It says so
    // on the way, because a human who arrow-upped the pane's command is owed an
    // answer to "did that start a second conversation?".
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
fn the_pane_command_is_the_core_this_entry_and_the_two_operands() {
    let line = ae::run::pane_command(
        Path::new("/opt/ae 1/ae-core"),
        Path::new("/s/tg1"),
        "spawned.0",
    );
    assert_eq!(line, "'/opt/ae 1/ae-core' _run '/s/tg1' 'spawned.0'");
}

// ---- the environment prefix (colead Z2 BLOCKER-1) --------------------------

#[test]
fn a_bare_leading_assignment_is_an_environment_delta_and_never_the_binary() {
    // `A=1 codex --yolo` CLASSIFIES as codex — `split_binary` has always
    // skipped an assignment word — but `_run` peeled assignments only after a
    // literal `env`, so the exec ran a binary named `A=1`.
    // CODEX deliberately: ae prepends its own `env` prefix to a claude command,
    // and that prefix hid the defect — the assignment was peeled as one of
    // `env`'s own operands. Codex gets no prefix, so the profile's assignment
    // is the FIRST word of the composed line, which is where the peel failed.
    let rig = Rig::new("assign");
    rig.only_profile(&format!("AE_Z2_MARK=set {} --flag", rig.tool("codex")));
    rig.seat("custom", "");
    let plan = rig.plan();
    assert!(plan.contains(r#""env_set":{"AE_Z2_MARK":"set"}"#), "{plan}");
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
    // The marker is the whole create-vs-resume discriminator. Writing it best
    // effort meant a transient failure produced TWO runs that both took the
    // create branch — and for claude and grok the second one collides on a
    // create-once `--session-id`.
    let rig = Rig::new("marker");
    rig.seat("claude", "u-1");
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
    // `lex_simple_command` refuses brace expansion and a word-initial comment;
    // `_run` used to hand both to the tool as literal bytes.
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
