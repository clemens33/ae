//! Black-box tests: run the real binary, not the library.
//!
//! `CARGO_BIN_EXE_ae` is set by cargo for integration tests of a package with a
//! `[[bin]]` — it is the path to the binary this test run just built, so this
//! exercises argv handling, exit code mapping and stdout for real.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use super::parity::Invocation;
use super::parity::capture::ExitOutcome;
use super::parity::capture::raw;
use crate::phase2::run_tmux;

// ONE OF THREE DOORS — `clippy.toml` denies `std::process::Command` crate-wide
// and `parity_self_test::the_capability_boundary_holds_against_any_lint_relaxation`
// pins the complete inventory of exceptions by asking the compiler for it.
//
// This one is not a parity concern: these tests drive the PRODUCT binary and
// asserting on what it printed is their whole job, where the parity harness
// must never judge a lane. `ae` is private to this module, so nothing in the
// harness can reach a child process through it. The third door is the product's
// own, in `src/transport.rs`; a binary this file runs may therefore spawn tmux
// of its own accord, which is what makes the liveness assertions below real.
#[allow(
    clippy::disallowed_types,
    reason = "black-box tests must run the product binary; see clippy.toml"
)]
pub(crate) fn ae() -> std::process::Command {
    std::process::Command::new(env!("CARGO_BIN_EXE_ae"))
}

/// Run one GENERATED SESSION HELPER — a shim the launch wrote — as a black-box
/// process. The launch suite's door: the whole point of a shim is that a pane
/// can exec it by path, so proving it works means running the file rather than
/// the function behind it. Lives here beside `ae()` so the capability stays in
/// the files the boundary guard already names.
#[allow(
    clippy::disallowed_types,
    reason = "the black-box tests' door: a session helper must be RUN to be proven; see clippy.toml"
)]
pub(crate) fn helper(path: &std::path::Path) -> std::process::Command {
    std::process::Command::new(path)
}

/// Run a session helper reached BY NAME, through `PATH`.
///
/// The other black-box door for the same subject, and the only spelling that
/// produces it: a helper's identity is `argv[0]`, so "invoked by name" is a
/// process whose `argv[0]` is the bare name — which `Command::new(<path>)`
/// cannot make. It exists to prove the refusal, not to run a helper.
#[allow(
    clippy::disallowed_types,
    reason = "the black-box tests' second door: a helper reached by name is a process started AS that name; see clippy.toml"
)]
pub(crate) fn helper_by_name(name: &str) -> std::process::Command {
    std::process::Command::new(name)
}

// The FIFO fixture. Safe std can bind a socket and make a directory, but the
// one special file that BLOCKS an ungated open — the case a `-f` gate exists
// for — needs mkfifo(2), and the only route to it without libc is mkfifo(1).
// A fixture door, registered with the black-box door in the parity self-test's
// inventory; it never runs the product.
#[allow(
    clippy::disallowed_types,
    reason = "the FIFO fixture: safe std cannot make a FIFO, mkfifo(1) can; see clippy.toml"
)]
pub(crate) fn mkfifo(path: &std::path::Path) {
    let status = std::process::Command::new("mkfifo").arg(path).status();
    assert!(
        matches!(status, Ok(status) if status.success()),
        "a FIFO at {}",
        path.display()
    );
}

/// Run `git` with `args` in `repo` for TEST FIXTURE SETUP — a repo the git it-
/// tests build to exercise the preview's git facts. Config is fully isolated
/// (`GIT_CONFIG_GLOBAL`/`SYSTEM=/dev/null`, identity via env) so the run does not
/// touch, or depend on, the developer's git config. This reuses this file's
/// existing `Command` door rather than opening a new one; asserting on the
/// preview's OWN git facts is what the tests do, so a setup helper is not the
/// subject. Returns stdout, trimmed.
#[allow(
    clippy::disallowed_types,
    clippy::expect_used,
    reason = "a door: builds real fixture repos for the preview's git-facts tests (cli.rs is inventoried); a fixture that cannot run git must panic loudly, like the #[test] callers it feeds"
)]
pub(crate) fn git_in(repo: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("git should run for fixture setup");
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        repo.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Wait at most `limit` for a spawned `child`: `Some(output)` if it exited,
/// `None` if it had to be killed. A test whose subject can hang must have a
/// red that ARRIVES, not one that stalls the lane.
pub(crate) fn bounded(
    mut child: std::process::Child,
    limit: std::time::Duration,
) -> Option<std::process::Output> {
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) if started.elapsed() <= limit => {}
            // Timed out, or the wait itself failed: either way the child is
            // still ours and possibly blocked, so it is killed and reaped
            // before the `None` — never dropped alive (review NIT).
            Ok(None) | Err(_) => break,
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

#[test]
fn version_prints_the_version_line_and_exits_zero() {
    let out = ae()
        .arg("--version")
        .output()
        .expect("the ae binary should run");

    assert!(out.status.success(), "exit status: {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).expect("stdout should be utf-8");
    assert_eq!(stdout, format!("ae {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn sc_022_an_unknown_option_is_diagnosed_on_stderr_with_stdout_empty() {
    // WHAT THIS MEASURES CHANGED IN SLICE Z3, and the test moved with it rather
    // than being weakened. Until Z3 the binary was reached by `ae-entry`, which
    // prepended a preamble; a top-level `--frobnicate` therefore fell through
    // the router into the LAUNCH grammar and was refused there — naming the
    // four flags a launch does define — at exit 1. Called without a preamble,
    // as this test used to call it, the same word reached `cli::Request::parse`
    // instead and exited 2. That second path was never a path a human could
    // take, and Z3 deletes it: the binary IS the public `ae` now, so what this
    // asserts is the surface the operator actually gets.
    //
    // RESIDUAL, recorded rather than papered over: SC-022 rules that an unknown
    // top-level OPTION exits 2, and the launch grammar's usage refusals exit 1.
    // The gap predates this slice — the shipped bash product answered 1 here
    // too — and closing it means re-coding every launch-plan refusal, which is
    // a ruling and not a test edit. The half of SC-022 the DISPATCHER owns (an
    // unknown `list`/`ls` tail) is unaffected and still exits 2; see
    // `an_unknown_list_flag_is_a_usage_error`.
    let out = ae()
        .arg("--frobnicate")
        .output()
        .expect("the ae binary should run");

    assert!(
        out.stdout.is_empty(),
        "stdout must stay empty for a machine caller, got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8(out.stderr).expect("stderr should be utf-8");
    assert!(stderr.contains("--frobnicate"), "stderr: {stderr}");
    assert_ne!(out.status.code(), Some(0), "exit status: {:?}", out.status);
}

/// The half of SC-022 the DISPATCHER owns, through the public binary.
///
/// An unknown `list` tail is a token the router hands to a parser that DOES
/// define its flag set, so there is no launch grammar underneath to soften it:
/// stderr, empty stdout, exit 2.
#[test]
fn an_unknown_list_flag_is_a_usage_error() {
    let out = ae()
        .arg("list")
        .arg("--frobnicate")
        .output()
        .expect("the ae binary should run");

    assert_eq!(out.status.code(), Some(2), "exit status: {:?}", out.status);
    assert!(
        out.stdout.is_empty(),
        "stdout must stay empty: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8(out.stderr).expect("stderr should be utf-8");
    assert!(stderr.contains("--frobnicate"), "stderr: {stderr}");
}

#[test]
fn sc_022_a_top_level_session_name_is_never_an_unknown_command() {
    let out = ae()
        .arg("my-feature")
        .output()
        .expect("the ae binary should run");

    assert_ne!(
        out.status.code(),
        Some(2),
        "a session name is not usage-wrong"
    );
    assert!(out.stdout.is_empty(), "stdout: {:?}", out.stdout);
    let stderr = String::from_utf8(out.stderr).expect("stderr should be utf-8");
    assert!(
        !stderr.contains("unknown"),
        "the row forbids this phrase for such a token: {stderr}"
    );
}

#[test]
fn criterion_1_the_real_list_and_ls_surfaces_answer_over_a_real_state_root() {
    // THE REFUSAL IS GONE. The shipped binary is invoked exactly as an operator
    // would invoke it, against a state root planted on disk, and it renders —
    // human and JSON, `list` and `ls`. The phase-2 baseline refused here with
    // `no session source is wired`; nothing in this build can print that,
    // because the constant no longer exists.
    let root = scratch("entry-point");
    plant_session(&root, "AlphaR");
    plant_session(&root, "ZetaR");

    for spelling in ["list", "ls"] {
        for json in [false, true] {
            let mut command = ae();
            command.env("AE_HOME", &root).arg(spelling);
            if json {
                command.arg("--json");
            }
            let out = command.output().expect("the ae binary should run");
            let stdout = String::from_utf8(out.stdout).expect("stdout should be utf-8");
            let stderr = String::from_utf8(out.stderr).expect("stderr should be utf-8");

            // Per-surface (gate blob 8cccbe44 / OC-P3-HUMAN-DIAGNOSTIC vs
            // OC-P3-JSON-WARNING): incomplete-human rc is open; JSON process
            // rc is retained. These planted sessions record a server this
            // invocation cannot query, so the snapshot is incomplete.
            if json {
                assert_eq!(
                    out.status.code(),
                    Some(0),
                    "{spelling}/json={json}: {stderr}"
                );
            }
            assert!(
                !stderr.contains("no session source"),
                "{spelling}/json={json}: the unwired refusal came back: {stderr}"
            );
            assert!(
                stdout.contains("AlphaR") && stdout.contains("ZetaR"),
                "{spelling}/json={json}: the planted sessions did not reach output: {stdout}"
            );
            // THE STATUS IS `unknown`, AND THAT IS THE WHOLE POINT. The
            // transport is real now and it really ran: these sessions record a
            // server that is not running, so the query FAILED — and SC-017l says
            // an unanswerable query is `unknown`, never `stopped`. If the
            // transport reported a SUCCESSFUL EMPTY query instead of a failure,
            // every one of these rows would say `stopped`: ae would be asserting
            // these sessions are gone on the strength of a question that got no
            // answer. That is #105 restated at the entry point.
            //
            // The opposed arm — a server that DOES answer, making the same route
            // say `running` and `stopped` — is in `transport.rs`. Without it this
            // assertion would also pass on a transport that can never succeed.
            assert!(
                stdout.contains("unknown"),
                "{spelling}/json={json}: an unverifiable session must be unknown: {stdout}"
            );
            assert!(
                !stdout.contains("stopped"),
                "{spelling}/json={json}: nothing was proven absent, so nothing may say \
                 stopped: {stdout}"
            );
            if json {
                assert!(
                    stdout.contains(r#""schema_version":2"#),
                    "{spelling}: the successor document is what reached stdout: {stdout}"
                );
                // INCOMPLETE, and correctly so: these sessions record a server,
                // ae is entitled to ask it, and this build cannot. SC-017o makes
                // an entitled server whose enumeration fails a loss, so the
                // snapshot says it could not look everywhere — rather than
                // reporting a complete picture it did not establish.
                assert!(
                    stdout.contains(r#""inventory_complete":false"#),
                    "{spelling}: a build that cannot query must not claim completeness: {stdout}"
                );
            } else {
                assert!(
                    !stderr.is_empty(),
                    "{spelling}: and the human surface says so on stderr"
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Run `git` with `args` and return its stdout, or `None` if it could not run.
///
/// Through the parity harness's pinned door rather than a second `Command` in
/// this file: `the_doors_to_a_child_process_are_the_inventoried_ones` counts
/// relaxations per file, and this needs no new one.
fn git(args: &[&str]) -> Option<String> {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = std::env::temp_dir().join(format!("ae-git-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&scratch);
    let out = scratch.join("out");
    let err = scratch.join("err");
    let mut invocation = Invocation::new("git");
    for arg in args {
        invocation = invocation.arg(arg);
    }
    let status = raw::run(&invocation, manifest, &out, &err).ok()?;
    let text = std::fs::read_to_string(&out).ok();
    let _ = std::fs::remove_dir_all(&scratch);
    matches!(status.outcome(), ExitOutcome::Code(0)).then_some(text?)
}

#[test]
fn criterion_1_the_opposed_control_is_that_the_phase_2_baseline_still_refused() {
    // A POSITIVE WITHOUT AN OPPOSED CONTROL PROVES LESS THAN IT LOOKS. The arm
    // above shows `list` and `ls` answer today; this one shows they did NOT
    // before, so the change is attributable to this work rather than to a
    // refusal that was never reachable in the first place.
    //
    // The baseline is DERIVED, not written down: the most recent commit that
    // changed the refusal constant in `src/lib.rs` is the one that removed it,
    // so its parent is the last tree that still had it. A hardcoded sha would
    // rot the first time history is rewritten, and would say nothing about WHY
    // that commit is the boundary.
    let Some(removal) = git(&[
        "log",
        "-S",
        "NO_SESSION_SOURCE",
        "--format=%H",
        "--",
        "src/lib.rs",
    ]) else {
        panic!("this control needs git and the repository history to be present");
    };
    let removal = removal
        .lines()
        .next()
        .unwrap_or_else(|| panic!("the refusal constant must appear in history"))
        .to_owned();
    let baseline = format!("{removal}^:src/lib.rs");
    let Some(before) = git(&["show", &baseline]) else {
        panic!("the baseline tree must be readable: {baseline}");
    };

    // THE OPPOSED CONTROL: the baseline refused, and had no callable render path
    // from the entry point — `run` handed `run_with` no source at all.
    assert!(
        before.contains("NO_SESSION_SOURCE"),
        "the baseline must still carry the refusal, or it is not the opposed control"
    );
    assert!(
        before.contains("no session source is wired"),
        "including the message a user would have seen"
    );
    assert!(
        before.contains("run_with(args, None, out, err)"),
        "and its entry point reached no world at all"
    );

    // TODAY: neither the constant nor its message exists anywhere in the crate.
    let Ok(now) = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
    ) else {
        panic!("src/lib.rs must be readable");
    };
    assert!(
        !now.contains("NO_SESSION_SOURCE") && !now.contains("no session source is wired"),
        "the refusal is gone from the product, not merely unreachable"
    );
}

#[test]
fn a_machine_that_cannot_say_where_its_state_lives_is_told_so() {
    // The one remaining refusal, and it is about THIS INVOCATION rather than
    // about the build: no AE_HOME, no HOME, nothing to enumerate from.
    let out = ae()
        .env_clear()
        .arg("list")
        .output()
        .expect("the ae binary should run");

    assert_eq!(out.status.code(), Some(1), "{:?}", out.status);
    assert!(out.stdout.is_empty(), "stdout must stay empty");
    let stderr = String::from_utf8(out.stderr).expect("stderr should be utf-8");
    assert!(stderr.contains(ae::NO_STATE_ROOT), "stderr: {stderr}");
}

/// A scratch state root, short-lived and per-test.
///
/// `/tmp` DIRECTLY rather than `std::env::temp_dir()`, because a socket path
/// lives under here now. `sun_path` is 104 bytes on macOS and `temp_dir()`
/// eats most of them, so `<root>/no-server.sock` can exceed the limit — and
/// then tmux fails for PATH LENGTH rather than for the absence this fixture
/// means to assert. That is the right answer for the wrong reason, which is
/// worse than the premise it was meant to replace: it would survive a transport
/// that had stopped being able to look at all. `phase2.rs` and `transport.rs`
/// use `/tmp` for the same reason.
fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(format!("/tmp/ae-cli-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        std::fs::create_dir_all(dir.join("sessions")).is_ok(),
        "a scratch state root"
    );
    dir
}

/// A durable session the product will discover for itself.
///
/// The meta carries a POSITIVE server selector, and that is load-bearing rather
/// than decorative: without one, SC-405l normalizes the selector to `missing`,
/// the classifier never asks anything, and the liveness query branch is never
/// reached. A fixture like that cannot tell a transport that FAILS from one that
/// answers successfully-empty — and those two differ by exactly the `unknown`
/// versus `stopped` this test exists to pin. The mutation lane found that hole;
/// the selector closes it.
///
/// THE SERVER MUST NOT EXIST, and that premise is now ASSERTED rather than
/// argued. It became contingent the moment the transport stopped being inert: a
/// fixture recording a server ae actually queries depends on nobody running one
/// by that address, or the query SUCCEEDS, legitimately reports these sessions
/// absent, and renders `stopped` against an assertion of not-stopped.
///
/// A named server could only NARROW that — a per-process name is unlikely to be
/// occupied, never proven unoccupied, and pids are reused. A socket path inside
/// this test's own scratch directory is STRUCTURAL: the directory was created
/// empty moments ago, the path is checked absent here, and no other process has
/// a reason to bind it. The residual on the old form was in the safe direction
/// (a collision can only fabricate an alarm, never mask a defect) but the red
/// would have been unexplainable — a developer cannot tell ae breaking from
/// someone's stray tmux server.
fn plant_session(root: &std::path::Path, name: &str) {
    let dir = root.join("sessions").join(name);
    let server = root.join("no-server.sock");
    assert!(
        !server.exists(),
        "this fixture's whole premise is that nothing answers at {}",
        server.display()
    );
    let written = std::fs::create_dir_all(&dir).and_then(|()| {
        std::fs::write(
            dir.join("meta"),
            format!(
                "mode=local\nagent.main=cl:lead\ntmux_server_kind=socket\ntmux_server={}\n",
                server.display()
            ),
        )
    });
    assert!(written.is_ok(), "a planted session");
}

#[test]
fn an_unknown_list_flag_exits_two_not_one() {
    // The usage error is decided by argv, before the missing source matters —
    // so `2` must not be swallowed by the unwired path's `1`.
    for tail in [["list", "--frobnicate"], ["ls", "my-feature"]] {
        let out = ae().args(tail).output().expect("the ae binary should run");

        assert_eq!(out.status.code(), Some(2), "{tail:?}: {:?}", out.status);
        assert!(out.stdout.is_empty(), "{tail:?}: stdout must stay empty");
        let stderr = String::from_utf8(out.stderr).expect("stderr should be utf-8");
        assert!(stderr.contains(tail[1]), "{tail:?}: {stderr}");
    }
}

// ── the internal helper surfaces (`_requests`, `_events-tail`, `_state`) ─────
//
// The LIBRARY behind them is compared against every frozen corpus row in
// `super::helper_corpus`. What is proved here is the other half — the argv, the
// exit-code mapping, which stream each answer lands on, and the fact that the
// follow surface actually follows. A parity run invokes the binary, not the
// library, so this is the half that makes the corpus comparison a claim about
// the product.

/// A session meta directory with the events container these tests need.
fn plant_events(root: &std::path::Path, session: &str, lines: &[&str]) -> std::path::PathBuf {
    let dir = root.join("sessions").join(session);
    let mut body = String::new();
    for line in lines {
        body.push_str(line);
        body.push('\n');
    }
    let written =
        std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(dir.join("events.jsonl"), body));
    assert!(written.is_ok(), "a planted event container");
    dir
}

const PLANTED_ASK: &str = r#"{"ts":"2026-08-20T16:12:55Z","actor":"cl:lead","action":"ask","target":"cl:worker","ref":"ae-1","actor_slot":"main","actor_session":"s","target_slot":"worker.0","target_session":"s","summary":"the planted question"}"#;

#[test]
fn requests_all_prints_the_table_on_stdout_and_says_nothing_else() {
    let root = scratch("requests-all");
    let dir = plant_events(&root, "tg1", &[PLANTED_ASK]);

    let out = ae()
        .arg(ae::cli::REQUESTS)
        .arg(&dir)
        .arg("all")
        .output()
        .expect("the ae binary should run");

    assert_eq!(out.status.code(), Some(0), "{:?}", out.status);
    assert!(out.stderr.is_empty(), "stderr: {:?}", out.stderr);
    // The bytes, not a substring: this surface's whole contract is its bytes.
    let mut expected = ae::requests::header();
    expected.extend_from_slice(
        b"pending  ask      ae-1                         cl:lead              cl:worker            the planted question\n",
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&expected)
    );
}

#[test]
fn requests_mine_and_inbox_answer_for_the_pane_tmux_pane_names() {
    // A REAL isolated server, created and stamped through the harness's pinned
    // process door (`-S` addressing). The product runs a BARE `tmux`, which
    // takes its socket from `$TMUX`, so pointing that at the same socket is
    // what makes the product's ambient server this one and no other. Short
    // path on purpose — `sun_path` is 104 bytes on macOS.
    let scratch_dir = std::path::PathBuf::from(format!("/tmp/aeid.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch_dir);
    std::fs::create_dir_all(&scratch_dir).expect("a scratch directory");
    let sock = scratch_dir.join("sock");
    let server = ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(sock.clone()));
    let tmux = |tail: &[&str]| {
        let mut args = ae::tmux::server_args(&server);
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &scratch_dir)
    };
    let (created, _) = tmux(&["-f", "/dev/null", "new-session", "-d", "-s", "idsess"]);
    assert!(created, "creating the session must succeed");
    let (_, pane) = tmux(&["display-message", "-p", "-t", "idsess", "#{pane_id}"]);
    let pane = pane.trim().to_owned();
    assert!(pane.starts_with('%'), "a pane id, got {pane:?}");
    assert!(tmux(&["set-option", "-p", "-t", &pane, "@ae_slot", "worker.0"]).0);
    assert!(tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", "cl:worker"]).0);

    // The planted row is routed main -> worker.0 in session `idsess`, so the
    // stamped pane is its addressee and not its asker.
    let root = scratch("requests-identity");
    let planted = PLANTED_ASK
        .replace(r#""actor_session":"s""#, r#""actor_session":"idsess""#)
        .replace(r#""target_session":"s""#, r#""target_session":"idsess""#);
    let dir = plant_events(&root, "idsess", &[&planted]);
    std::fs::write(dir.join("meta"), "session=idsess\n").expect("a meta file");

    let run = |mode: &str| {
        let out = ae()
            .env("TMUX", format!("{},0,0", sock.display()))
            .env("TMUX_PANE", &pane)
            .arg(ae::cli::REQUESTS)
            .arg(&dir)
            .arg(mode)
            .output()
            .expect("the ae binary should run");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let inbox = run("inbox");
    let mine = run("mine");
    let _ = tmux(&["kill-server"]);
    let _ = std::fs::remove_dir_all(&scratch_dir);

    assert_eq!(inbox.0, Some(0), "{inbox:?}");
    assert!(inbox.2.is_empty(), "{inbox:?}");
    assert!(
        inbox.1.contains("the planted question"),
        "the addressee sees the row: {inbox:?}"
    );
    assert_eq!(mine.0, Some(0), "{mine:?}");
    assert_eq!(
        mine.1,
        String::from_utf8_lossy(&ae::requests::header()),
        "not the asker, so only the header: {mine:?}"
    );
}

#[test]
fn goal_set_and_clear_rewrite_meta_and_announce_and_fail_loudly_without_a_meta() {
    let root = scratch("goal");
    let dir = root.join("sessions").join("g1");
    std::fs::create_dir_all(&dir).expect("a session dir");
    std::fs::write(dir.join("meta"), "mode=local\nsession=g1\n").expect("a meta file");
    let run = |tail: &[&str]| {
        let out = ae()
            .env_remove("TMUX_PANE")
            .arg(ae::cli::GOAL)
            .arg(&dir)
            .args(tail)
            .output()
            .expect("the ae binary should run");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let set = run(&["ship", "it\u{7}"]);
    assert_eq!(
        set,
        (Some(0), "Goal set: ship it\n".to_owned(), String::new())
    );
    let meta = std::fs::read_to_string(dir.join("meta")).unwrap();
    assert_eq!(
        meta, "mode=local\nsession=g1\ngoal=ship it\n",
        "appended, one line, control dropped"
    );
    let events = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
    assert!(
        events.contains("\"actor\":\"human\",\"action\":\"goal\",\"summary\":\"ship it\"}"),
        "{events}"
    );

    let cleared = run(&["--clear"]);
    assert_eq!(
        cleared,
        (Some(0), "Goal cleared.\n".to_owned(), String::new())
    );
    let meta = std::fs::read_to_string(dir.join("meta")).unwrap();
    assert_eq!(
        meta, "mode=local\nsession=g1\n",
        "the key is gone, the rest untouched"
    );
    let events = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
    assert_eq!(events.lines().count(), 2);
    assert!(
        events
            .lines()
            .nth(1)
            .unwrap()
            .contains("\"summary\":\"goal cleared\"}"),
        "{events}"
    );
    assert!(
        std::fs::metadata(dir.join("meta.lock")).is_ok(),
        "the lock bash takes"
    );

    // Usage: 2, and nothing touched. `--help` is the same exit and text, as
    // the frozen body answers it.
    for tail in [
        vec!["--clear", "extra"],
        vec!["\u{1b}"],
        vec!["--help"],
        vec!["-h", "ignored"],
    ] {
        let usage = run(&tail);
        assert_eq!(usage.0, Some(2), "{tail:?}: {usage:?}");
        assert!(usage.1.is_empty(), "{tail:?}: {usage:?}");
        assert!(usage.2.starts_with("Usage: goal"), "{tail:?}: {usage:?}");
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        2
    );

    // FAILURE CONTROL: no meta file. The set fails at 1, says so, and emits
    // no event — nothing was recorded, so nothing is announced.
    let bare = root.join("sessions").join("g2");
    std::fs::create_dir_all(&bare).expect("a session dir");
    let out = ae()
        .env_remove("TMUX_PANE")
        .arg(ae::cli::GOAL)
        .arg(&bare)
        .arg("no meta here")
        .output()
        .expect("the ae binary should run");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stdout.is_empty(), "no success line: {out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("goal not recorded: could not write session meta"),
        "{out:?}"
    );
    assert!(
        !bare.join("events.jsonl").exists(),
        "no event for a goal that was not written"
    );
    assert!(!bare.join("meta").exists(), "and no meta was conjured");
}

#[test]
fn goal_reads_the_first_record_or_no_goal_and_reports_an_unreadable_meta() {
    let root = scratch("goal-read");
    let run = |dir: &std::path::Path, tail: &[&str]| {
        let out = ae()
            .env_remove("TMUX_PANE")
            .arg(ae::cli::GOAL)
            .arg(dir)
            .args(tail)
            .output()
            .expect("the ae binary should run");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let none = (Some(0), "(no goal set)\n".to_owned(), String::new());
    let dir = root.join("sessions").join("r1");
    std::fs::create_dir_all(&dir).expect("a session dir");
    std::fs::write(dir.join("meta"), "mode=local\nsession=r1\n").expect("a meta file");
    assert_eq!(run(&dir, &[]), none, "no record is no goal");
    std::fs::write(
        dir.join("meta"),
        "mode=local\ngoal=first=kept\r\ngoal=second\nsession=r1\n",
    )
    .expect("a meta file");
    assert_eq!(
        run(&dir, &[]),
        (Some(0), "first=kept\r\n".to_owned(), String::new()),
        "head -1, cut -d= -f2-, bytes verbatim"
    );
    assert_eq!(run(&dir, &["ship", "it"]).0, Some(0));
    assert_eq!(
        run(&dir, &[]),
        (Some(0), "ship it\n".to_owned(), String::new()),
        "the record the binary writes reads back"
    );
    assert_eq!(run(&dir, &["--clear"]).0, Some(0));
    assert_eq!(run(&dir, &[]), none, "and so does the clear");

    // No meta at all is no goal — the frozen grep's `|| true`.
    let bare = root.join("sessions").join("r2");
    std::fs::create_dir_all(&bare).expect("a session dir");
    assert_eq!(run(&bare, &[]), none);

    // But a meta that exists and cannot be read is reported, where the frozen
    // body would have printed `(no goal set)` over the failure.
    let unreadable = root.join("sessions").join("r3");
    std::fs::create_dir_all(unreadable.join("meta")).expect("a directory where the meta goes");
    let failed = run(&unreadable, &[]);
    assert_eq!(failed.0, Some(1), "{failed:?}");
    assert!(failed.1.is_empty(), "{failed:?}");
    assert!(
        failed
            .2
            .contains("goal not read: could not read session meta"),
        "{failed:?}"
    );
}

#[test]
fn memo_add_appends_the_tsv_record_and_announces_and_fails_loudly_when_it_cannot() {
    let root = scratch("memo");
    let dir = root.join("sessions").join("m1");
    std::fs::create_dir_all(&dir).expect("a session dir");
    let run = |dir: &std::path::Path, tail: &[&str]| {
        let out = ae()
            .env_remove("TMUX_PANE")
            .arg(ae::cli::MEMO)
            .arg(dir)
            .args(tail)
            .output()
            .expect("the ae binary should run");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let added = run(&dir, &["add", "--topic", "p2", "one\tline"]);
    assert_eq!(
        added,
        (Some(0), String::new(), String::new()),
        "silent on success, as bash is"
    );
    let tsv = std::fs::read_to_string(dir.join("memo.tsv")).unwrap();
    let fields: Vec<&str> = tsv.trim_end_matches('\n').split('\t').collect();
    assert_eq!(fields.len(), 4, "{tsv:?}");
    assert!(
        fields[0].ends_with('Z') && fields[0].len() == 20,
        "ts {:?}",
        fields[0]
    );
    assert_eq!(&fields[1..], ["human", "p2", "one line"]);
    assert!(
        std::fs::metadata(dir.join("memo.tsv.lock")).is_ok(),
        "the lock bash takes"
    );
    let events = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
    assert!(
        events.contains(
            "\"actor\":\"human\",\"action\":\"memo\",\"ref\":\"p2\",\"summary\":\"one line\"}"
        ),
        "{events}"
    );

    // Usage: 2, nothing touched.
    for tail in [
        vec!["add"],
        vec!["add", "--topic", "t"],
        vec!["read", "--topic"],
        vec!["read", "x"],
        vec!["tail", "x"],
        vec!["tail", "1", "2"],
        vec!["show"],
    ] {
        let usage = run(&dir, &tail);
        assert_eq!(usage.0, Some(2), "{tail:?}: {usage:?}");
        assert!(
            usage.2.starts_with("Usage: memo add"),
            "{tail:?}: {usage:?}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("memo.tsv"))
            .unwrap()
            .lines()
            .count(),
        1
    );

    // FAILURE CONTROL: memo.tsv is a directory, so the append cannot open it.
    // 1, said on stderr, and NO event — the record did not land, so nothing
    // is announced.
    let blocked = root.join("sessions").join("m2");
    std::fs::create_dir_all(blocked.join("memo.tsv")).expect("a directory where the file goes");
    let failed = run(&blocked, &["add", "cannot land"]);
    assert_eq!(failed.0, Some(1), "{failed:?}");
    assert!(failed.1.is_empty());
    assert!(
        failed
            .2
            .contains("memo not recorded: could not append to memo.tsv"),
        "{failed:?}"
    );
    assert!(
        !blocked.join("events.jsonl").exists(),
        "no event for a memo that was not recorded"
    );
}

#[test]
fn memo_read_and_tail_render_the_frozen_shape_and_report_an_unreadable_file() {
    let root = scratch("memo-read");
    let run = |dir: &std::path::Path, tail: &[&str]| {
        let out = ae()
            .env_remove("TMUX_PANE")
            .arg(ae::cli::MEMO)
            .arg(dir)
            .args(tail)
            .output()
            .expect("the ae binary should run");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let empty = (Some(0), String::new(), String::new());
    // No memo file at all is empty output at 0, as in bash.
    let dir = root.join("sessions").join("r1");
    std::fs::create_dir_all(&dir).expect("a session dir");
    assert_eq!(run(&dir, &["read"]), empty);
    assert_eq!(run(&dir, &["tail", "3"]), empty);

    // One record the binary wrote, one a bash writer wrote: the frozen
    // renderer's exact shape over both; the bare call is `read`; a topic
    // filter that matches nothing is empty; `tail 1` is the last record.
    assert_eq!(run(&dir, &["add", "--topic", "p2", "one\tline"]), empty);
    let tsv = std::fs::read_to_string(dir.join("memo.tsv")).unwrap();
    let ts = tsv.split('\t').next().unwrap().to_owned();
    std::fs::write(
        dir.join("memo.tsv"),
        format!("{tsv}2026-01-01T00:00:00Z\tcl:lead\tgeneral\tbash writer\n"),
    )
    .expect("a second record");
    let first = format!("{ts} — human [p2]\none line\n\n");
    let second = "2026-01-01T00:00:00Z — cl:lead\nbash writer\n\n";
    let both = format!("{first}{second}");
    for (tail, expected) in [
        (vec![], both.as_str()),
        (vec!["read"], both.as_str()),
        (vec!["read", "--topic", "p2"], first.as_str()),
        (vec!["read", "--topic", "other"], ""),
        (vec!["tail"], both.as_str()),
        (vec!["tail", "1"], second),
        (vec!["tail", "0"], ""),
    ] {
        assert_eq!(
            run(&dir, &tail),
            (Some(0), expected.to_owned(), String::new()),
            "{tail:?}"
        );
    }

    // A directory in the file's place is what `[[ -f ]] || exit 0` rejects:
    // empty at 0, as in bash, and never opened.
    let blocked = root.join("sessions").join("r2");
    std::fs::create_dir_all(blocked.join("memo.tsv")).expect("a directory where the file goes");
    assert_eq!(run(&blocked, &["read"]), empty);
    assert_eq!(run(&blocked, &["tail", "1"]), empty);
}

#[test]
fn a_fifo_in_a_containers_place_is_the_frozen_empty_answer_and_not_a_hang() {
    // The frozen bodies gate every container read on `[[ -f ]]`; the core's
    // first cut opened first and asked later, and a FIFO — no writer, ever —
    // left it blocked with no stdout, no stderr and no exit (found in review).
    // Every read surface over a container is exercised, under a bound, so a
    // regression is a red that arrives rather than a lane that stalls.
    let root = scratch("fifo");
    let dir = root.join("sessions").join("f1");
    std::fs::create_dir_all(&dir).expect("a session dir");
    std::fs::write(dir.join("meta"), "session=f1\n").expect("a meta file");
    mkfifo(&dir.join("events.jsonl"));
    mkfifo(&dir.join("memo.tsv"));
    let limit = std::time::Duration::from_secs(10);
    let run = |sub: &str, tail: &[&str]| {
        let child = ae()
            .env_remove("TMUX_PANE")
            .arg(sub)
            .arg(&dir)
            .args(tail)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the ae binary should spawn");
        bounded(child, limit).map(|out| {
            (
                out.status.code(),
                String::from_utf8_lossy(&out.stdout).into_owned(),
                String::from_utf8_lossy(&out.stderr).into_owned(),
            )
        })
    };
    assert_eq!(
        run(ae::cli::STATE, &[]),
        Some((
            Some(0),
            "human state: (none declared)\n".to_owned(),
            String::new()
        )),
        "state read: a FIFO is not a regular file, so there is no declaration"
    );
    let empty = Some((Some(0), String::new(), String::new()));
    assert_eq!(run(ae::cli::MEMO, &["read"]), empty, "memo read");
    assert_eq!(run(ae::cli::MEMO, &["tail", "1"]), empty, "memo tail");
    let requests = run(ae::cli::REQUESTS, &["all"]).expect("requests all exited");
    assert_eq!(requests.0, Some(0), "requests all: {requests:?}");
    assert!(requests.2.is_empty(), "requests all: {requests:?}");
}

#[test]
fn state_refuses_without_a_pane_identity_and_writes_nothing() {
    let root = scratch("state-noid");
    let dir = root.join("sessions").join("s1");
    std::fs::create_dir_all(&dir).expect("a session dir");
    let out = ae()
        .env_remove("TMUX_PANE")
        .arg(ae::cli::STATE)
        .arg(&dir)
        .arg("working")
        .arg("nothing should land")
        .output()
        .expect("the ae binary should run");
    assert_eq!(out.status.code(), Some(1), "{:?}", out.status);
    assert!(out.stdout.is_empty(), "no success line: {:?}", out.stdout);
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!("{}\n", ae::state::NO_IDENTITY)
    );
    assert!(
        !dir.join("events.jsonl").exists(),
        "a refused declaration opens nothing"
    );
    // Usage errors are 2, decided before any identity question.
    for tail in [vec!["Working"], vec!["blocked"]] {
        let mut command = ae();
        command
            .env_remove("TMUX_PANE")
            .arg(ae::cli::STATE)
            .arg(&dir);
        command.args(&tail);
        let out = command.output().expect("the ae binary should run");
        assert_eq!(out.status.code(), Some(2), "{tail:?}: {:?}", out.status);
        assert!(
            String::from_utf8_lossy(&out.stderr)
                .contains("Usage: state <working|waiting-user|blocked|done> [reason]"),
            "{tail:?}"
        );
    }

    // The READ needs no identity: it asks about `human`, as the frozen body
    // does from any shell — and a reason-less declaration keeps its
    // timestamp, where the frozen body's `IFS=$'\t' read` slides it into the
    // reason (measured: `working — 2026-…Z  (since )`).
    let read = |dir: &std::path::Path| {
        let out = ae()
            .env_remove("TMUX_PANE")
            .arg(ae::cli::STATE)
            .arg(dir)
            .output()
            .expect("the ae binary should run");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    assert_eq!(
        read(&dir),
        (
            Some(0),
            "human state: (none declared)\n".to_owned(),
            String::new()
        ),
        "no container yet is no declaration"
    );
    let planted = plant_events(
        &root,
        "s2",
        &[
            r#"{"ts":"2026-08-27T08:00:00Z","actor":"human","action":"state","ref":"working"}"#,
            r#"{"ts":"2026-08-27T08:00:01Z","actor":"cl:lead","action":"state","ref":"done","summary":"not the human's"}"#,
        ],
    );
    assert_eq!(
        read(&planted),
        (
            Some(0),
            "human state: working  (since 2026-08-27T08:00:00Z)\n".to_owned(),
            String::new()
        )
    );
}

#[test]
fn state_declares_for_the_pane_and_a_held_lock_fails_it_at_the_bound() {
    // A real isolated server, as in the requests identity test.
    let scratch_dir = std::path::PathBuf::from(format!("/tmp/aest.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch_dir);
    std::fs::create_dir_all(&scratch_dir).expect("a scratch directory");
    let sock = scratch_dir.join("sock");
    let server = ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(sock.clone()));
    let tmux = |tail: &[&str]| {
        let mut args = ae::tmux::server_args(&server);
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &scratch_dir)
    };
    assert!(tmux(&["-f", "/dev/null", "new-session", "-d", "-s", "stsess"]).0);
    let (_, pane) = tmux(&["display-message", "-p", "-t", "stsess", "#{pane_id}"]);
    let pane = pane.trim().to_owned();
    assert!(tmux(&["set-option", "-p", "-t", &pane, "@ae_slot", "main"]).0);
    assert!(tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", "cl:lead"]).0);

    let root = scratch("state-pane");
    let dir = root.join("sessions").join("stsess");
    std::fs::create_dir_all(&dir).expect("a session dir");
    std::fs::write(dir.join("meta"), "session=stsess\n").expect("a meta file");
    let run = |tail: &[&str]| {
        let out = ae()
            .env("TMUX", format!("{},0,0", sock.display()))
            .env("TMUX_PANE", &pane)
            .arg(ae::cli::STATE)
            .arg(&dir)
            .args(tail)
            .output()
            .expect("the ae binary should run");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };

    let done = run(&["done", "all", "green"]);
    let container = std::fs::read_to_string(dir.join("events.jsonl")).unwrap_or_default();

    // Now hold the lock the way a bash writer does and try again.
    let lock = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join("events.jsonl.lock"))
        .expect("the lock file the declaration created");
    lock.lock().expect("an exclusive flock");
    let started = std::time::Instant::now();
    let held = run(&["working", "must not land"]);
    let waited = started.elapsed();
    drop(lock);
    let after_hold = std::fs::read_to_string(dir.join("events.jsonl")).unwrap_or_default();
    let released = run(&["working", "lands now"]);
    let shown = run(&[]);
    let _ = tmux(&["kill-server"]);
    let _ = std::fs::remove_dir_all(&scratch_dir);

    assert_eq!(done.0, Some(0), "{done:?}");
    assert_eq!(done.1, "Marked cl:lead done: all green\n");
    assert!(done.2.is_empty(), "{done:?}");
    let lines: Vec<&str> = container.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "state line plus the legacy done line: {container}"
    );
    assert!(
        lines[0].contains(
            "\"actor\":\"cl:lead\",\"action\":\"state\",\"ref\":\"done\",\"summary\":\"all green\"}"
        ),
        "{container}"
    );
    assert!(
        lines[1].contains("\"actor\":\"cl:lead\",\"action\":\"done\",\"summary\":\"all green\"}"),
        "{container}"
    );

    assert_eq!(held.0, Some(1), "held lock: {held:?}");
    assert!(
        held.1.is_empty(),
        "no success line under a held lock: {held:?}"
    );
    assert!(held.2.contains("could not lock"), "{held:?}");
    assert!(
        waited >= std::time::Duration::from_secs(5),
        "the 5s bound was honoured: {waited:?}"
    );
    assert!(
        waited < std::time::Duration::from_secs(20),
        "and it is a bound: {waited:?}"
    );
    assert_eq!(
        after_hold, container,
        "nothing was appended under the held lock"
    );

    assert_eq!(released.0, Some(0), "after release: {released:?}");
    let final_lines = std::fs::read_to_string(dir.join("events.jsonl")).unwrap_or_default();
    assert_eq!(final_lines.lines().count(), 3, "{final_lines}");

    // The READ, from the same pane: the newest of ITS declarations, in the
    // frozen printf, with the timestamp the write recorded (`{"ts":"` is the
    // emitter's first member, so the stamp is bytes 7..27 of the last line).
    let ts = &final_lines.lines().last().expect("a last line")[7..27];
    let line = format!("cl:lead state: working — lands now  (since {ts})\n");
    assert_eq!(shown, (Some(0), line, String::new()));
}

#[test]
fn requests_defaults_to_mine_and_refuses_at_one_with_no_identity() {
    // The default mode is `mine`, and outside a pane `mine` cannot be answered.
    // `1` here is PINNED by 24 corpus rows and must not drift to the usage `2`.
    let root = scratch("requests-default");
    let dir = plant_events(&root, "tg1", &[PLANTED_ASK]);

    for tail in [vec![], vec!["mine"], vec!["inbox"]] {
        let mut command = ae();
        // The test itself may run inside an ae pane; the product must not
        // inherit that pane's identity, or "no identity" is not what is tested.
        command.env_remove("TMUX_PANE");
        command.arg(ae::cli::REQUESTS).arg(&dir);
        command.args(&tail);
        let out = command.output().expect("the ae binary should run");

        assert_eq!(out.status.code(), Some(1), "{tail:?}: {:?}", out.status);
        assert!(
            out.stdout.is_empty(),
            "{tail:?}: the refusal precedes the header, so stdout stays empty"
        );
        let stderr = String::from_utf8(out.stderr).expect("stderr should be utf-8");
        assert_eq!(
            stderr,
            format!("{}\n", ae::requests::NO_IDENTITY),
            "{tail:?}"
        );
    }
}

#[test]
fn a_helper_surface_asked_wrong_exits_two_and_prints_nothing() {
    let root = scratch("requests-usage");
    let dir = plant_events(&root, "tg1", &[PLANTED_ASK]);
    let dir = dir.to_string_lossy().into_owned();

    let wrong: [Vec<&str>; 5] = [
        vec![ae::cli::REQUESTS],
        vec![ae::cli::REQUESTS, &dir, "bogus"],
        vec![ae::cli::REQUESTS, &dir, "all", "extra"],
        vec![ae::cli::EVENTS_TAIL],
        vec![ae::cli::EVENTS_TAIL, &dir, "extra"],
    ];
    for argv in wrong {
        let out = ae().args(&argv).output().expect("the ae binary should run");
        assert_eq!(out.status.code(), Some(2), "{argv:?}: {:?}", out.status);
        assert!(out.stdout.is_empty(), "{argv:?}: stdout must stay empty");
        assert!(
            !out.stderr.is_empty(),
            "{argv:?}: a usage error has to say something"
        );
    }
}

#[test]
fn the_underscore_spellings_are_commands_and_never_launch_candidates() {
    // SC-022's grammar is not narrowed by these two: a leading underscore is
    // not a legal session name, so nothing that WAS a launch candidate stopped
    // being one. Both directions are asserted — the spelling dispatches, and it
    // never falls through to the launcher's message.
    for spelling in [ae::cli::REQUESTS, ae::cli::EVENTS_TAIL] {
        let out = ae()
            .arg(spelling)
            .output()
            .expect("the ae binary should run");
        let stderr = String::from_utf8(out.stderr).expect("stderr should be utf-8");
        assert!(
            !stderr.contains("start is not implemented"),
            "{spelling} reached the launcher: {stderr}"
        );
        assert_eq!(out.status.code(), Some(2), "{spelling}");
    }
    // And the ARGV help names them, so a shipped surface is discoverable.
    //
    // `ae::help_text()` and not what `ae --help` prints: since slice Z3 the
    // binary IS the public `ae`, so `--help` answers with the human command
    // list ([`ae::entry::HELP`]) exactly as it did through the wrapper, and
    // that list has never named the underscore entries — they are reached
    // through a session helper, not typed. The argv help is where they belong
    // and where this row can still see them.
    let help = ae::help_text();
    for spelling in [ae::cli::REQUESTS, ae::cli::EVENTS_TAIL] {
        assert!(help.contains(spelling), "help omits {spelling}: {help}");
    }
    // The human list is the other half of the same fact: it is what `--help`
    // prints, and it names the SESSION HELPERS rather than the entries.
    let out = ae()
        .arg("--help")
        .output()
        .expect("the ae binary should run");
    assert_eq!(
        String::from_utf8(out.stdout).expect("stdout should be utf-8"),
        ae::entry::HELP
    );
}

#[test]
fn events_tail_prints_its_opening_then_follows_what_is_appended() {
    // THE ONLY TEST OF THE FOLLOW. Everything else about this surface is a pure
    // function over bytes; the loop that keeps reading is not, and a monitor
    // pane that shows the replay and then goes deaf would pass every other
    // test in the tree.
    //
    // Shaped like the corpus instrument: start it, let it produce, kill it.
    // Reads happen on a worker thread behind `recv_timeout` so a surface that
    // produces too little fails the test instead of hanging the suite.
    use std::io::Read as _;
    use std::sync::mpsc;
    use std::time::Duration;

    let root = scratch("events-tail-follow");
    let session = "tg1";
    let dir = plant_events(&root, session, &[PLANTED_ASK]);

    let mut opening = ae::events_tail::banner(session.as_bytes());
    opening.extend_from_slice(&ae::events_tail::replay(
        format!("{PLANTED_ASK}\n").as_bytes(),
    ));

    let appended = r#"{"ts":"2026-08-20T16:13:01Z","actor":"cl:worker","action":"reply","target":"cl:lead","ref":"ae-1","summary":"the appended answer"}"#;
    let followed = ae::events_tail::format_event(appended.as_bytes()).expect("an event line");

    let mut child = ae()
        .arg(ae::cli::EVENTS_TAIL)
        .arg(&dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the ae binary should start");
    let mut piped = child.stdout.take().expect("stdout was piped");

    // One reader thread for both reads, in order: the opening, then the follow
    // line. Appending happens here, between them, so the record genuinely
    // arrives after the process was already running.
    let (sender, receiver) = mpsc::channel();
    let want = (opening.len(), followed.len());
    std::thread::spawn(move || {
        let mut first = vec![0_u8; want.0];
        let opening_read = piped.read_exact(&mut first).map(|()| first);
        let _ = sender.send(opening_read);
        let mut second = vec![0_u8; want.1];
        let follow_read = piped.read_exact(&mut second).map(|()| second);
        let _ = sender.send(follow_read);
    });

    let got_opening = receiver.recv_timeout(Duration::from_secs(15));

    // THE APPEND IS TORN ON PURPOSE, and this is the part that matters most: a
    // writer is not atomic, so the follow WILL read a record mid-write. Half of
    // it lands, at least one poll goes by, then the rest. The single complete
    // line asserted below is only what arrives if the follow refuses to yield an
    // unterminated record — emit the fragment and these bytes are wrong.
    let (half, rest) = appended.split_at(appended.len() / 2);
    let open_for_append = || {
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("events.jsonl"))
    };
    let torn = open_for_append()
        .and_then(|mut file| std::io::Write::write_all(&mut file, half.as_bytes()));
    assert!(torn.is_ok(), "the first half should land");
    // Two poll intervals, so a follow that emits fragments has had every chance
    // to emit one before the record is finished.
    std::thread::sleep(Duration::from_millis(2_100));
    let completed = open_for_append()
        .and_then(|mut file| std::io::Write::write_all(&mut file, format!("{rest}\n").as_bytes()));
    assert!(completed.is_ok(), "the rest should land");

    // The poll interval is one second, so this waits for a real poll to notice.
    let got_follow = receiver.recv_timeout(Duration::from_secs(15));

    let killed = child.kill();
    let status = child.wait();
    let mut leftover = Vec::new();
    if let Some(mut errors) = child.stderr.take() {
        let _ = errors.read_to_end(&mut leftover);
    }

    let opening_bytes = got_opening
        .expect("the opening should arrive")
        .expect("and be complete");
    assert_eq!(
        String::from_utf8_lossy(&opening_bytes),
        String::from_utf8_lossy(&opening)
    );
    let follow_bytes = got_follow
        .expect("the appended record should be followed")
        .expect("and be complete");
    assert_eq!(
        String::from_utf8_lossy(&follow_bytes),
        String::from_utf8_lossy(&followed)
    );
    assert!(killed.is_ok(), "the follow was still running to be killed");
    assert!(status.is_ok());
    assert!(
        leftover.is_empty(),
        "this surface writes nothing to stderr — the frozen capture's \
         `Terminated: 15` was the SHELL's job notification, not ae's: {:?}",
        String::from_utf8_lossy(&leftover)
    );
}

// ---- tracked requests: ask / review -------------------------------------

/// A real isolated server with two stamped panes in one session, each pane
/// RECORDING what it receives, and the binary run as a helper would run it.
///
/// Since B move 1 the core pastes for itself, so there is no delivery helper
/// to stand in: what a target got is read off the target. Each pane runs
/// `cat` into a file, which makes it an UNMODELLED tool — no input sensor, no
/// deferral, no staged re-check — so these tests pin what the core does
/// AROUND the paste (composition, resolution, the envelope, the recovery
/// record, the event) while `deliver.rs`'s own suite pins the modelled TUI
/// mechanics against a fake one.
///
/// A `send` stub survives for exactly one path: the no-identity fallback,
/// which is a plain `send` and so still runs the public helper.
struct Tracked {
    scratch_dir: std::path::PathBuf,
    sock: std::path::PathBuf,
    dir: std::path::PathBuf,
    main: String,
    worker: String,
}

impl Tracked {
    fn new(tag: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let scratch_dir =
            std::path::PathBuf::from(format!("/tmp/aetr.{}.{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch_dir);
        assert!(
            std::fs::create_dir_all(&scratch_dir).is_ok(),
            "a scratch directory"
        );
        let sock = scratch_dir.join("sock");
        let session = format!("tr{tag}");
        let mut fixture = Self {
            scratch_dir: scratch_dir.clone(),
            sock,
            dir: scratch_dir.join("sessions").join(&session),
            main: String::new(),
            worker: String::new(),
        };
        // Each pane RECORDS what it is sent: `cat` appending to its own file.
        // `exec` makes `cat` the pane's own process, so the dead-pane guard
        // sees a live non-shell foreground, as it does for a real agent.
        let recorder = |file: &str| format!("exec cat >> {}", scratch_dir.join(file).display());
        assert!(
            fixture
                .tmux(&[
                    "-f",
                    "/dev/null",
                    "new-session",
                    "-d",
                    "-x",
                    "400",
                    "-y",
                    "40",
                    "-s",
                    &session,
                    &recorder("received.main"),
                ])
                .0
        );
        assert!(
            fixture
                .tmux(&[
                    "split-window",
                    "-d",
                    "-t",
                    &session,
                    &recorder("received.worker")
                ])
                .0
        );
        let (_, panes) = fixture.tmux(&["list-panes", "-s", "-t", &session, "-F", "#{pane_id}"]);
        let ids: Vec<&str> = panes.lines().collect();
        assert_eq!(ids.len(), 2, "{panes}");
        let (main, worker) = (ids[0].to_owned(), ids[1].to_owned());
        for (pane, slot, agent) in [(&main, "main", "lead"), (&worker, "worker.0", "worker")] {
            assert!(
                fixture
                    .tmux(&["set-option", "-p", "-t", pane, "@ae_slot", slot])
                    .0
            );
            assert!(
                fixture
                    .tmux(&["set-option", "-p", "-t", pane, "@ae_agent", agent])
                    .0
            );
        }
        assert!(
            std::fs::create_dir_all(&fixture.dir).is_ok(),
            "a session dir"
        );
        // The meta records the socket the fixture's tmux runs on, as a real
        // launch does: target resolution reads this recorded selector (not the
        // caller's ambient server) and FAILS CLOSED without it, so a serverless
        // meta would be an unrealistic fixture, not a passing one.
        assert!(
            std::fs::write(
                fixture.dir.join("meta"),
                format!(
                    "session={session}\ntmux_server_kind=socket\ntmux_server={}\n",
                    fixture.sock.display()
                ),
            )
            .is_ok(),
            "a meta file"
        );
        // The public `send` helper, stubbed. Only the no-identity fallback
        // runs it now, and what that path must show is that it ran AT ALL
        // (with the raw body and no event names), because a plain send
        // records its own event and the core must write none.
        let stub = r#"#!/bin/bash
here="$(cd "$(dirname "$0")" && pwd)"
printf '%s\n' "$1" >"$here/send.target"
printf '%s' "$2" >"$here/send.message"
env | grep -E '^(_AE_EVENT_ACTION|_AE_EVENT_REF|AE_SENDER_OVERRIDE)=' | LC_ALL=C sort >"$here/send.env"
exit 0
"#;
        assert!(
            std::fs::write(fixture.dir.join("send"), stub).is_ok(),
            "the stub send"
        );
        assert!(
            std::fs::set_permissions(
                fixture.dir.join("send"),
                std::fs::Permissions::from_mode(0o755),
            )
            .is_ok(),
            "an executable stub send"
        );
        fixture.main = main;
        fixture.worker = worker;
        fixture
    }

    fn tmux(&self, tail: &[&str]) -> (bool, String) {
        let server =
            ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(self.sock.clone()));
        let mut args = ae::tmux::server_args(&server);
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &self.scratch_dir)
    }

    /// Run `sub` as `pane` would (or with no pane at all), with `envs` added.
    fn run(
        &self,
        sub: &str,
        pane: Option<&str>,
        tail: &[&str],
        envs: &[(&str, &str)],
    ) -> (Option<i32>, String, String) {
        let mut command = ae();
        command
            .env("TMUX", format!("{},0,0", self.sock.display()))
            .env_remove("AE_SENDER_OVERRIDE")
            .env_remove("STUB_SEND_RC")
            .env_remove("STUB_RECORD")
            .arg(sub)
            .arg(&self.dir)
            .args(tail);
        match pane {
            Some(pane) => command.env("TMUX_PANE", pane),
            None => command.env_remove("TMUX_PANE"),
        };
        for (key, value) in envs {
            command.env(key, value);
        }
        let out = command
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// What the stubbed public `send` recorded: target, message, env lines.
    fn stub(&self) -> (String, String, String) {
        let read = |name: &str| std::fs::read_to_string(self.dir.join(name)).unwrap_or_default();
        (read("send.target"), read("send.message"), read("send.env"))
    }

    /// What a pane RECEIVED, waiting briefly for `cat` to flush it.
    ///
    /// The paste and the Enter are two tmux calls and the recorder is a
    /// separate process, so an immediate read can see a prefix. The delivery
    /// already returned by the time this is called, so this only waits out the
    /// pipe, never for the product.
    fn received(&self, which: &str) -> String {
        let path = self.scratch_dir.join(format!("received.{which}"));
        for _ in 0..80 {
            let seen = std::fs::read_to_string(&path).unwrap_or_default();
            if !seen.is_empty() {
                return seen;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        std::fs::read_to_string(&path).unwrap_or_default()
    }

    /// What a pane has received SO FAR, without waiting — for asserting that
    /// nothing arrived, where waiting would only be waiting.
    fn received_now(&self, which: &str) -> String {
        std::fs::read_to_string(self.scratch_dir.join(format!("received.{which}")))
            .unwrap_or_default()
    }

    /// Respawn a pane as a bare shell, leaving its stamps in place — a pane
    /// whose agent has DIED.
    fn kill_agent_in(&self, pane: &str) {
        assert!(self.tmux(&["respawn-pane", "-k", "-t", pane, "exec sh"]).0);
    }

    /// The recovery record the last event points at, and its content.
    fn record(&self) -> (String, String) {
        let last = self.events().pop().unwrap_or_default();
        let path = last
            .split("\"body_file\":\"")
            .nth(1)
            .and_then(|tail| tail.split('"').next())
            .unwrap_or_default()
            .to_owned();
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        (path, content)
    }

    /// A second session directory on this server whose only agent is a pane
    /// running a plain shell while the roster records `binary` for its seat —
    /// the shape the dead-pane guard exists for.
    fn dead_pane_session(&self, binary: &str) -> std::path::PathBuf {
        let session = "dead";
        assert!(
            self.tmux(&[
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                session,
                "exec sh"
            ])
            .0
        );
        let (_, panes) = self.tmux(&["list-panes", "-s", "-t", session, "-F", "#{pane_id}"]);
        let pane = panes.lines().next().unwrap_or_default().to_owned();
        assert!(
            self.tmux(&["set-option", "-p", "-t", &pane, "@ae_slot", "main"])
                .0
        );
        assert!(
            self.tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", "ghost"])
                .0
        );
        let dir = self.scratch_dir.join("sessions").join(session);
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
        assert!(
            std::fs::write(
                dir.join("meta"),
                format!(
                    "session={session}\ntmux_server_kind=socket\ntmux_server={}\nseat.main=ghost\nagent_bin.main={binary}\n",
                    self.sock.display()
                ),
            )
            .is_ok(),
            "a meta file"
        );
        dir
    }

    fn forget(&self) {
        for name in ["send.target", "send.message", "send.env"] {
            let _ = std::fs::remove_file(self.dir.join(name));
        }
        for which in ["main", "worker"] {
            // TRUNCATE, never unlink: the recorder holds the file open, so a
            // removed inode would take every later delivery with it and the
            // next assertion would read an empty file forever.
            let _ = std::fs::write(self.scratch_dir.join(format!("received.{which}")), "");
        }
    }

    fn events(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
        let _ = std::fs::remove_dir_all(&self.scratch_dir);
    }
}

/// The request id the stub was handed, out of its recorded env.
/// The `ref` of the fixture's LAST event — the request id the core minted.
fn event_ref(fx: &Tracked) -> String {
    let last = fx.events().pop().unwrap_or_default();
    last.split("\"ref\":\"")
        .nth(1)
        .and_then(|tail| tail.split('"').next())
        .unwrap_or_else(|| panic!("the event carries a ref: {last:?}"))
        .to_owned()
}

/// The framed text a pane received, with the trailing newline the Enter added
/// stripped — what the core pasted, byte for byte.
fn pasted(fx: &Tracked, which: &str) -> String {
    let seen = fx.received(which);
    seen.strip_suffix('\n').unwrap_or(&seen).to_owned()
}

/// One refusal case: argv after the directory, extra env, the exit code, the
/// exact stderr.
type Refusal<'a> = (&'a [&'a str], &'a [(&'a str, &'a str)], Option<i32>, String);

fn is_request_id(id: &str, prefix: &str) -> bool {
    let parts: Vec<&str> = id.split('-').collect();
    parts.len() == 3
        && parts[0] == prefix
        && parts[1].len() == 16
        && parts[1].ends_with('Z')
        && parts[2].len() == 8
        && parts[2]
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[test]
fn ask_composes_the_frozen_message_delivers_through_send_and_writes_the_slotted_event() {
    let fx = Tracked::new("ask");
    let asked = fx.run(
        ae::cli::ASK,
        Some(&fx.main),
        &["worker", "the", "question"],
        &[],
    );
    assert_eq!(asked, (Some(0), String::new(), String::new()));
    let events = fx.events();
    assert_eq!(events.len(), 1, "{events:?}");
    let id = event_ref(&fx);
    assert!(is_request_id(&id, "ae"), "{id}");
    let reply_cmd = format!(
        "{}/reply --as \"worker\" \"{id}\" \"<your reply>\"",
        fx.dir.display()
    );
    let message = ae::tracked::compose(
        ae::tracked::Kind::Ask,
        &id,
        "lead",
        "the question",
        &reply_cmd,
    );
    assert!(message.starts_with(&format!(
        "REQUEST {id} from lead: the question\n\nREQUIRED:"
    )));
    // THE TARGET GOT IT, framed by the core from the pane's own stamp — the
    // envelope leads, the composed request follows, byte for byte.
    assert_eq!(
        pasted(&fx, "worker"),
        format!("⟦ae:msg from lead⟧\n{message}"),
        "the worker pane received the framed request"
    );
    assert!(
        fx.received_now("main").is_empty(),
        "and the asker's own pane received nothing"
    );
    let (body_file, stored) = fx.record();
    assert!(
        body_file.starts_with(
            &fx.dir
                .join("messages")
                .join(format!("{id}.ask."))
                .display()
                .to_string()
        ) && std::path::Path::new(&body_file)
            .extension()
            .is_some_and(|ext| ext == "txt"),
        "the record is named for the request and the action: {body_file}"
    );
    assert_eq!(
        stored,
        format!("⟦ae:msg from lead⟧\n{message}"),
        "the record is the delivered text, not the message as typed"
    );
    assert!(
        events[0].ends_with(&format!(
            "\"actor\":\"lead\",\"action\":\"ask\",\"target\":\"worker\",\"ref\":\"{id}\",\"actor_slot\":\"main\",\"actor_session\":\"trask\",\"target_slot\":\"worker.0\",\"target_session\":\"trask\",\"summary\":\"the question\",\"body_file\":\"{body_file}\"}}"
        )),
        "{}",
        events[0]
    );
    assert!(events[0].starts_with("{\"ts\":\""), "{}", events[0]);
    // And the core's own requests surface reads it back as pending.
    let listed = fx.run(ae::cli::REQUESTS, Some(&fx.main), &["all"], &[]);
    assert_eq!(listed.0, Some(0), "{listed:?}");
    assert!(
        listed.1.contains("pending") && listed.1.contains(&id) && listed.1.contains("worker"),
        "{listed:?}"
    );
}

#[test]
fn review_carries_its_instructions_and_every_target_spelling_resolves_as_the_helper_does() {
    let fx = Tracked::new("rev");
    let reviewed = fx.run(
        ae::cli::REVIEW,
        Some(&fx.worker),
        &["lead", "look", "at", "x"],
        &[],
    );
    assert_eq!(reviewed, (Some(0), String::new(), String::new()));
    let id = event_ref(&fx);
    assert!(is_request_id(&id, "review"), "{id}");
    let message = pasted(&fx, "main");
    assert!(
        message.starts_with(&format!(
            "⟦ae:msg from worker⟧\nREVIEW REQUEST {id} from worker: look at x\n\n{}\n\nREQUIRED",
            ae::tracked::REVIEW_INSTRUCTIONS
        )),
        "{message}"
    );
    assert!(message.ends_with(&format!("\n{}/reply --as \"lead\" \"{id}\" \"<your review>\"\nDo not reply any other way. Do NOT use peek/peak as a reply mechanism.", fx.dir.display())), "{message}");
    let events = fx.events();
    assert!(
        events[0].contains(&format!("\"actor\":\"worker\",\"action\":\"review\",\"target\":\"lead\",\"ref\":\"{id}\",\"actor_slot\":\"worker.0\",\"actor_session\":\"trrev\",\"target_slot\":\"main\",\"target_session\":\"trrev\",")),
        "{}",
        events[0]
    );

    // THE THREE SPELLINGS OF ONE PANE, plus its id. Identity v2 widens what is
    // ACCEPTED and moves nothing that is printed: the bare name, the session
    // qualified without an `@`, and the `@` form all reach the same pane, and
    // all three answer with the same display ref.
    for (spelling, expected, pane) in [
        (fx.main.clone(), "lead", "main"),
        ("lead".to_owned(), "lead", "main"),
        ("trrev:lead".to_owned(), "lead", "main"),
        ("@trrev:lead".to_owned(), "lead", "main"),
        ("@trrev:worker".to_owned(), "worker", "worker"),
    ] {
        fx.forget();
        let sent = fx.run(ae::cli::ASK, Some(&fx.worker), &[&spelling, "q"], &[]);
        assert_eq!(sent.0, Some(0), "{spelling}: {sent:?}");
        let last = fx.events().pop().unwrap_or_default();
        assert!(
            last.contains(&format!("\"target\":\"{expected}\"")),
            "{spelling}: {last}"
        );
        assert!(
            !fx.received_now(pane).is_empty(),
            "{spelling}: the {pane} pane received nothing"
        );
    }
}

#[test]
fn a_request_that_does_not_resolve_or_is_refused_leaves_no_event_and_no_paste() {
    let fx = Tracked::new("ref");
    let cases: [Refusal<'_>; 8] = [
        // IDENTITY V2: the alias-only and bare-name arms of the resolver are
        // retired, so a legacy alias addresses nothing. It is NOT FOUND, not
        // ambiguous — the roster can no longer make a target ambiguous at all,
        // because a name is one seat.
        (
            &["cl", "q"],
            &[],
            Some(1),
            "Error: agent 'cl' not found in session 'trref'\n".to_owned(),
        ),
        // One colon and no `@` is now a CROSS-SESSION address, so an
        // alias-shaped target names a session that does not exist.
        (
            &["cl:lead", "q"],
            &[],
            Some(1),
            "Error: session 'cl' not found\n".to_owned(),
        ),
        (
            &["nobody", "q"],
            &[],
            Some(1),
            "Error: agent 'nobody' not found in session 'trref'\n".to_owned(),
        ),
        (
            &["@nosuch:lead", "q"],
            &[],
            Some(1),
            "Error: session 'nosuch' not found\n".to_owned(),
        ),
        (
            &["@nosuch", "q"],
            &[],
            Some(1),
            "Error: cross-session target must be @session:agent, got '@nosuch'\n".to_owned(),
        ),
        (
            &["worker", " ", "\t"],
            &[],
            Some(1),
            ae::tracked::refusal("ask"),
        ),
        (&["worker"], &[], Some(2), ae::tracked::ASK_USAGE.to_owned()),
        (&[], &[], Some(2), ae::tracked::ASK_USAGE.to_owned()),
    ];
    for (tail, envs, code, stderr) in cases {
        fx.forget();
        let out = fx.run(ae::cli::ASK, Some(&fx.main), tail, envs);
        assert_eq!(out, (code, String::new(), stderr), "{tail:?}");
        assert!(
            fx.events().is_empty(),
            "{tail:?}: an event was written: {:?}",
            fx.events()
        );
        assert!(
            fx.received_now("worker").is_empty() && fx.received_now("main").is_empty(),
            "{tail:?}: something was pasted"
        );
    }
    // A DEAD TARGET is refused before anything is stored or pasted: the pane's
    // foreground is a shell while the roster expects a real binary there. The
    // ask never reaches the pane, and no event is written.
    fx.forget();
    let dead = fx.dead_pane_session("claude");
    let out = ae()
        .env("TMUX", format!("{},0,0", fx.sock.display()))
        .env("TMUX_PANE", &fx.main)
        .arg(ae::cli::ASK)
        .arg(&dead)
        .args(["ghost", "q"])
        .output()
        .expect("the ae binary should run");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        stderr,
        "ae: send to ghost REFUSED — target pane is a shell, not a running agent (the agent process is gone). Nothing pasted; a stray Enter would EXECUTE the message as a shell command. Re-launch the agent, then re-send.\n",
        "the frozen refusal, verbatim"
    );
    assert!(
        !dead.join("events.jsonl").exists(),
        "a refused delivery records nothing"
    );
    assert!(
        !dead.join("messages").exists(),
        "and stores nothing: the guard is BEFORE the body store"
    );
}

#[test]
fn no_identity_falls_back_to_a_plain_send_and_external_and_override_senders_are_event_only_or_slotless()
 {
    let fx = Tracked::new("idn");
    // No pane: the frozen warning, then a plain send of the raw body through
    // the PUBLIC helper — that path records its own event, so the core writes
    // none, and the core never pastes for it.
    let plain = fx.run(ae::cli::ASK, None, &["worker", "raw", "body"], &[]);
    assert_eq!(
        plain,
        (
            Some(0),
            String::new(),
            ae::tracked::NO_IDENTITY_WARNING.to_owned()
        )
    );
    let (target, message, env) = fx.stub();
    assert_eq!(
        (target.as_str(), message.as_str()),
        ("worker\n", "raw body")
    );
    assert!(env.is_empty(), "no event names: {env:?}");
    assert!(fx.events().is_empty());
    assert!(
        fx.received_now("worker").is_empty(),
        "the core pasted nothing — the helper owns that delivery"
    );

    // An external sink: no paste, no body file, the event with the literal
    // target and the caller's slot.
    fx.forget();
    let external = fx.run(ae::cli::ASK, Some(&fx.main), &["telegram:42", "hello"], &[]);
    assert_eq!(external, (Some(0), String::new(), String::new()));
    assert!(
        fx.received_now("worker").is_empty() && fx.stub().0.is_empty(),
        "an event-only sink is delivered to nobody"
    );
    let events = fx.events();
    assert_eq!(events.len(), 1);
    assert!(
        events[0].contains(
            "\"actor\":\"lead\",\"action\":\"ask\",\"target\":\"telegram:42\",\"ref\":\"ae-"
        ) && events[0].ends_with(
            "\"actor_slot\":\"main\",\"actor_session\":\"tridn\",\"summary\":\"hello\"}"
        ),
        "{}",
        events[0]
    );

    // AE_SENDER_OVERRIDE: the actor is the override, slotless, and it is what
    // the ENVELOPE the core pastes names too.
    fx.forget();
    let bridged = fx.run(
        ae::cli::REVIEW,
        Some(&fx.main),
        &["worker", "from", "the", "bridge"],
        &[("AE_SENDER_OVERRIDE", "bridge")],
    );
    assert_eq!(bridged, (Some(0), String::new(), String::new()));
    let message = pasted(&fx, "worker");
    assert!(
        message.starts_with("⟦ae:msg from bridge⟧\nREVIEW REQUEST review-")
            && message.contains(" from bridge: from the bridge\n"),
        "{message}"
    );
    let events = fx.events();
    assert_eq!(events.len(), 2);
    assert!(
        events[1].contains(
            "\"actor\":\"bridge\",\"action\":\"review\",\"target\":\"worker\",\"ref\":\"review-"
        ) && events[1].contains("\",\"actor_session\":\"tridn\",\"target_slot\":\"worker.0\",")
            && !events[1].contains("actor_slot"),
        "{}",
        events[1]
    );
}

// ---- tracked requests: reply ---------------------------------------------

/// An `ask` from the main pane to the worker: its request id.
fn ask_from_main(fx: &Tracked, body: &str) -> String {
    let asked = fx.run(ae::cli::ASK, Some(&fx.main), &["worker", body], &[]);
    assert_eq!(asked, (Some(0), String::new(), String::new()));
    let id = event_ref(fx);
    assert!(is_request_id(&id, "ae"), "{id}");
    fx.forget();
    id
}

/// Append one hand-written event line — a request the core did not create.
fn append_event(fx: &Tracked, line: &str) {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(fx.dir.join("events.jsonl"))
        .unwrap_or_else(|why| panic!("the ledger should open: {why}"));
    writeln!(file, "{line}").unwrap_or_else(|why| panic!("the ledger should append: {why}"));
}

/// The core's own `requests all`, as the main pane sees it.
fn requests_all(fx: &Tracked) -> String {
    let (code, view, stderr) = fx.run(ae::cli::REQUESTS, Some(&fx.main), &["all"], &[]);
    assert_eq!(code, Some(0), "{stderr}");
    view
}

#[test]
fn reply_routes_to_the_asker_by_stored_slot_records_the_frozen_event_and_closes_the_request() {
    let fx = Tracked::new("rp");
    let session = "trrp";
    let id = ask_from_main(&fx, "the question");
    // The asker is renamed AFTER the ask: the reply must reach the pane
    // holding the stored slot under its CURRENT name, never the stale one.
    assert!(
        fx.tmux(&["set-option", "-p", "-t", &fx.main, "@ae_agent", "renamed"])
            .0
    );
    let replied = fx.run(
        ae::cli::REPLY,
        Some(&fx.worker),
        &[&id, "the", "answer"],
        &[],
    );
    assert_eq!(replied, (Some(0), String::new(), String::new()));
    let message = format!("[{id}] the answer");
    // Routed by SLOT to the pane's CURRENT name, and framed with the VERIFIED
    // sender — the slot check decided who this reply is from, so the envelope
    // takes its answer rather than the caller's environment.
    assert_eq!(
        pasted(&fx, "main"),
        format!("⟦ae:msg from worker⟧\n{message}")
    );
    let events = fx.events();
    assert_eq!(events.len(), 2, "{events:?}");
    let (body_file, stored) = fx.record();
    assert!(
        body_file.starts_with(
            &fx.dir
                .join("messages")
                .join(format!("{id}.reply."))
                .display()
                .to_string()
        ),
        "{body_file}"
    );
    assert!(
        events[1].ends_with(&format!(
            "\"actor\":\"worker\",\"action\":\"reply\",\"target\":\"renamed\",\"ref\":\"{id}\",\"actor_slot\":\"worker.0\",\"actor_session\":\"{session}\",\"target_slot\":\"main\",\"target_session\":\"{session}\",\"summary\":\"the answer\",\"body_file\":\"{body_file}\"}}"
        )),
        "{}",
        events[1]
    );
    assert_eq!(
        stored,
        format!("⟦ae:msg from worker⟧\n{message}"),
        "the reply keeps its own recovery file"
    );
    let view = requests_all(&fx);
    assert!(
        view.contains("replied") && view.contains(&id) && view.contains("the answer"),
        "the core's requests view closes it with the reply's text: {view}"
    );
    // --as is DISPLAY only: it names the actor, and a disagreement with the
    // stored name is a warning after the slot has been verified.
    let id2 = ask_from_main(&fx, "again");
    fx.forget();
    let as_reply = fx.run(
        ae::cli::REPLY,
        Some(&fx.worker),
        &["--as", "stale", &id2, "ok"],
        &[],
    );
    assert_eq!(
        as_reply,
        (
            Some(0),
            String::new(),
            "Warning: --as 'stale' != stored target name 'worker' (name is advisory; slot verified)\n".to_owned()
        )
    );
    let events = fx.events();
    assert!(
        events[3].contains("\"actor\":\"stale\",\"action\":\"reply\",\"target\":\"renamed\""),
        "{}",
        events[3]
    );
    assert_eq!(
        pasted(&fx, "main"),
        format!("⟦ae:msg from stale⟧\n[{id2}] ok"),
        "the envelope names the --as identity, as the event does"
    );
}

#[test]
fn the_verified_sender_overwrites_an_override_inherited_from_the_caller() {
    // The frozen helper `exec env AE_SENDER_OVERRIDE="$reply_sender"` AFTER the
    // slot check; a caller's own override must not survive into the entry,
    // where it would become the provenance envelope.
    let fx = Tracked::new("rv");
    let id = ask_from_main(&fx, "q");
    let poisoned = fx.run(
        ae::cli::REPLY,
        Some(&fx.worker),
        &[&id, "clean"],
        &[("AE_SENDER_OVERRIDE", "spoof")],
    );
    assert_eq!(poisoned, (Some(0), String::new(), String::new()));
    assert_eq!(
        pasted(&fx, "main"),
        format!("⟦ae:msg from worker⟧\n[{id}] clean"),
        "the pane's verified identity envelopes it, not the inherited one"
    );
    let last = fx.events().pop().unwrap_or_default();
    assert!(
        last.contains("\"actor\":\"worker\",\"action\":\"reply\"") && !last.contains("spoof"),
        "{last}"
    );
    // A `--as` reply carries the --as identity, which is also what the event
    // names — the two never disagree.
    let id2 = ask_from_main(&fx, "again");
    fx.forget();
    let as_reply = fx.run(
        ae::cli::REPLY,
        Some(&fx.worker),
        &["--as", "worker", &id2, "ok"],
        &[("AE_SENDER_OVERRIDE", "spoof")],
    );
    assert_eq!(as_reply, (Some(0), String::new(), String::new()));
    assert_eq!(
        pasted(&fx, "main"),
        format!("⟦ae:msg from worker⟧\n[{id2}] ok"),
        "an inherited override never reaches the envelope"
    );
}

type ReplyCase<'a> = (Option<&'a str>, Vec<&'a str>, Option<i32>, String);

#[test]
fn a_reply_is_refused_exactly_and_pastes_nothing_when_the_pane_the_id_or_the_body_is_wrong() {
    let fx = Tracked::new("rr");
    let session = "trrr";
    let id = ask_from_main(&fx, "q");
    let slot_error = format!(
        "Error: request '{id}' is assigned to slot 'worker.0'@'{session}', current pane is slot 'main'@'{session}'\n"
    );
    let cases: Vec<ReplyCase<'_>> = vec![
        (Some(&fx.main), vec![&id, "x"], Some(1), slot_error.clone()),
        (
            Some(&fx.main),
            vec!["--as", "worker", &id, "x"],
            Some(1),
            slot_error,
        ),
        (
            None,
            vec![&id, "x"],
            Some(1),
            format!(
                "Error: request '{id}' is assigned to slot 'worker.0'@'{session}', current pane is slot 'none'@''\n"
            ),
        ),
        (
            Some(&fx.worker),
            vec!["nope", "x"],
            Some(1),
            format!(
                "Error: request id 'nope' not found in {}\n",
                fx.dir.join("events.jsonl").display()
            ),
        ),
        (
            Some(&fx.worker),
            vec![&id, "  "],
            Some(1),
            ae::tracked::refusal("reply"),
        ),
        (
            Some(&fx.worker),
            vec![&id],
            Some(2),
            ae::reply::USAGE.to_owned(),
        ),
        (
            Some(&fx.worker),
            vec!["--as", "worker", &id],
            Some(2),
            ae::reply::USAGE.to_owned(),
        ),
    ];
    for (pane, tail, code, stderr) in cases {
        fx.forget();
        let out = fx.run(ae::cli::REPLY, pane, &tail, &[]);
        assert_eq!(out, (code, String::new(), stderr), "{tail:?}");
        assert!(
            fx.received_now("main").is_empty(),
            "{tail:?}: something was pasted"
        );
        assert_eq!(fx.events().len(), 1, "{tail:?}: an event was written");
    }
    // A REFUSED delivery leaves the request OPEN: the asker's agent has died
    // and its pane dropped to a shell, so the reply is refused before it is
    // stored and no reply is recorded.
    fx.forget();
    std::fs::write(
        fx.dir.join("meta"),
        format!(
            "session={session}\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=claude\n",
            fx.sock.display()
        ),
    )
    .expect("a meta naming the seat's binary");
    fx.kill_agent_in(&fx.main);
    let refused = fx.run(ae::cli::REPLY, Some(&fx.worker), &[&id, "late"], &[]);
    assert_eq!(refused.0, Some(1), "{refused:?}");
    assert!(
        refused.2.starts_with("ae: send to lead REFUSED —"),
        "{refused:?}"
    );
    assert_eq!(fx.events().len(), 1);
    assert!(requests_all(&fx).contains("pending"));
}

#[test]
fn a_pre_migration_row_name_matches_with_the_frozen_errors_and_is_answered_at_the_stored_name() {
    // A pre-migration row (no routing members) name-matches, with the frozen
    // errors, and is answered at the stored name because there is no slot to
    // route by.
    let fx = Tracked::new("ro");
    append_event(
        &fx,
        r#"{"ts":"2026-01-01T00:00:00Z","actor":"lead","action":"ask","target":"worker","ref":"ae-old-1","summary":"old"}"#,
    );
    let old_cases: Vec<ReplyCase<'_>> = vec![
        (
            Some(&fx.worker),
            vec!["--as", "x:y", "ae-old-1", "x"],
            Some(1),
            "Error: override agent 'x:y' does not match assigned target 'worker'\n".to_owned(),
        ),
        (
            Some(&fx.main),
            vec!["ae-old-1", "x"],
            Some(1),
            "Error: request 'ae-old-1' is assigned to 'worker', current pane is 'lead'\n"
                .to_owned(),
        ),
        (
            None,
            vec!["ae-old-1", "x"],
            Some(1),
            "Error: could not detect current agent identity; rerun with --as 'worker' from the assigned agent context\n".to_owned(),
        ),
    ];
    for (pane, tail, code, stderr) in old_cases {
        fx.forget();
        let out = fx.run(ae::cli::REPLY, pane, &tail, &[]);
        assert_eq!(out, (code, String::new(), stderr), "{tail:?}");
        assert!(
            fx.received_now("main").is_empty(),
            "{tail:?}: something was pasted"
        );
    }
    fx.forget();
    let old = fx.run(ae::cli::REPLY, Some(&fx.worker), &["ae-old-1", "seen"], &[]);
    assert_eq!(old, (Some(0), String::new(), String::new()));
    assert_eq!(
        pasted(&fx, "main"),
        "⟦ae:msg from worker⟧\n[ae-old-1] seen",
        "the stored name, no slot to route by"
    );
    let last = fx.events().pop().unwrap_or_default();
    assert!(
        last.contains(
            "\"actor\":\"worker\",\"action\":\"reply\",\"target\":\"lead\",\"ref\":\"ae-old-1\",\"actor_slot\":\"worker.0\",\"actor_session\":\"trro\",\"summary\":\"seen\""
        ) && !last.contains("target_slot"),
        "{last}"
    );
}

#[test]
fn a_bash_shaped_request_is_consumed_a_pane_less_asker_is_answered_event_only_and_a_follow_up_is_noted()
 {
    let fx = Tracked::new("rb");
    // The frozen ae_emit_event's member order, as bash `ask` writes it.
    let id = "ae-20260827T100000Z-0badf00d";
    append_event(
        &fx,
        &format!(
            r#"{{"ts":"2026-08-27T10:00:00Z","actor":"lead","action":"ask","target":"worker","ref":"{id}","actor_slot":"main","actor_session":"trrb","target_slot":"worker.0","target_session":"trrb","summary":"bash asked","body_file":"/nonexistent"}}"#
        ),
    );
    let replied = fx.run(ae::cli::REPLY, Some(&fx.worker), &[id, "done"], &[]);
    assert_eq!(replied, (Some(0), String::new(), String::new()));
    assert_eq!(
        pasted(&fx, "main"),
        format!("⟦ae:msg from worker⟧\n[{id}] done"),
        "routed by the bash-stored slot"
    );
    assert!(
        fx.events()[1].contains(&format!(
            "\"actor\":\"worker\",\"action\":\"reply\",\"target\":\"lead\",\"ref\":\"{id}\",\"actor_slot\":\"worker.0\",\"actor_session\":\"trrb\",\"target_slot\":\"main\",\"target_session\":\"trrb\",\"summary\":\"done\""
        )),
        "{}",
        fx.events()[1]
    );
    assert!(requests_all(&fx).contains("replied"));
    // A follow-up is delivered and recorded, and said so — never refused.
    fx.forget();
    let again = fx.run(ae::cli::REPLY, Some(&fx.worker), &[id, "and", "more"], &[]);
    assert_eq!(
        again,
        (
            Some(0),
            String::new(),
            format!(
                "Note: request '{id}' already has a reply on file; delivering this one as a follow-up\n"
            )
        )
    );
    assert_eq!(
        pasted(&fx, "main"),
        format!("⟦ae:msg from worker⟧\n[{id}] and more")
    );
    assert_eq!(fx.events().len(), 3);
    // An asker with no pane — a bridge naming itself through
    // AE_SENDER_OVERRIDE — is answered as the frozen send answers a sink:
    // the event, nothing pasted, no body file.
    let bridged = "ae-20260827T100001Z-0badf00e";
    append_event(
        &fx,
        &format!(
            r#"{{"ts":"2026-08-27T10:00:01Z","actor":"telegram:42","action":"ask","target":"worker","ref":"{bridged}","target_slot":"worker.0","target_session":"trrb","summary":"from the bridge"}}"#
        ),
    );
    fx.forget();
    let to_bridge = fx.run(ae::cli::REPLY, Some(&fx.worker), &[bridged, "hello"], &[]);
    assert_eq!(to_bridge, (Some(0), String::new(), String::new()));
    assert!(
        fx.received_now("main").is_empty() && fx.received_now("worker").is_empty(),
        "nothing is pasted to a sink"
    );
    let last = fx.events().pop().unwrap_or_default();
    assert!(
        last.ends_with(&format!(
            "\"actor\":\"worker\",\"action\":\"reply\",\"target\":\"telegram:42\",\"ref\":\"{bridged}\",\"actor_slot\":\"worker.0\",\"actor_session\":\"trrb\",\"summary\":\"hello\"}}"
        )),
        "{last}"
    );
    assert!(requests_all(&fx).contains("from the bridge") || requests_all(&fx).contains("hello"));
}

// ---- the public send ------------------------------------------------------

#[test]
fn send_pastes_through_the_entry_and_records_the_one_frozen_event() {
    let fx = Tracked::new("sp");
    let sent = fx.run(
        ae::cli::SEND,
        Some(&fx.main),
        &["worker", "hello", "there"],
        &[],
    );
    assert_eq!(sent, (Some(0), String::new(), String::new()));
    assert_eq!(
        pasted(&fx, "worker"),
        "⟦ae:msg from lead⟧\nhello there",
        "the framed text reached the pane"
    );
    let events = fx.events();
    assert_eq!(events.len(), 1, "{events:?}");
    let (body_file, stored) = fx.record();
    assert!(
        body_file.starts_with(&fx.dir.join("messages").join("msg-").display().to_string())
            && body_file.contains(".send.")
            && std::path::Path::new(&body_file)
                .extension()
                .is_some_and(|ext| ext == "txt"),
        "no ref: the record is stamped, not named for one — {body_file}"
    );
    assert_eq!(stored, "⟦ae:msg from lead⟧\nhello there");
    assert!(
        events[0].ends_with(&format!(
            "\"actor\":\"lead\",\"action\":\"send\",\"target\":\"worker\",\"summary\":\"⟦ae:msg from lead⟧ hello there\",\"body_file\":\"{body_file}\"}}"
        )),
        "a plain send carries no routing members, as the frozen emitter writes it: {}",
        events[0]
    );
    // A request id in the text is the event's ref and names the recovery file.
    fx.forget();
    let noted = fx.run(
        ae::cli::SEND,
        Some(&fx.main),
        &["worker", "[ae-1]", "noted"],
        &[],
    );
    assert_eq!(noted, (Some(0), String::new(), String::new()));
    let last = fx.events().pop().unwrap_or_default();
    assert!(
        last.contains("\"action\":\"send\",\"target\":\"worker\",\"ref\":\"ae-1\",\"summary\":\"⟦ae:msg from lead⟧ [ae-1] noted\",\"body_file\":")
            && last.contains("/ae-1.send."),
        "{last}"
    );
    // A pane id resolves to that pane's stamp, as the frozen ae_resolve reads
    // it back: the event names the agent, and the pane still gets the text.
    fx.forget();
    let by_id = fx.run(
        ae::cli::SEND,
        Some(&fx.main),
        &[&fx.worker, "by", "id"],
        &[],
    );
    assert_eq!(by_id, (Some(0), String::new(), String::new()));
    assert_eq!(pasted(&fx, "worker"), "⟦ae:msg from lead⟧\nby id");
    let last = fx.events().pop().unwrap_or_default();
    assert!(last.contains("\"target\":\"worker\""), "{last}");
    // The cross-session spelling of this session resolves to the same pane.
    fx.forget();
    let spelled = fx.run(ae::cli::SEND, Some(&fx.main), &["@trsp:worker", "hi"], &[]);
    assert_eq!(spelled, (Some(0), String::new(), String::new()));
    assert_eq!(pasted(&fx, "worker"), "⟦ae:msg from lead⟧\nhi");
    let last = fx.events().pop().unwrap_or_default();
    assert!(last.contains("\"target\":\"worker\""), "{last}");
}

#[test]
fn send_carries_the_frozen_event_fields_from_the_environment_and_the_override_to_the_envelope() {
    let fx = Tracked::new("ss");
    // The shape `ae cancel` execs the helper under.
    let cancel = fx.run(
        ae::cli::SEND,
        Some(&fx.main),
        &["worker", "withdrawn:", "--digest-only"],
        &[
            ("AE_SENDER_OVERRIDE", "lead"),
            ("_AE_EVENT_ACTION", "cancel"),
            ("_AE_EVENT_REF", "ae-9"),
            ("_AE_EVENT_SUMMARY", "withdrawn"),
            ("_AE_EVENT_ACTOR_SLOT", "main"),
            ("_AE_EVENT_ACTOR_SESSION", "trss"),
            ("_AE_EVENT_TARGET_SLOT", "worker.0"),
            ("_AE_EVENT_TARGET_SESSION", "trss"),
        ],
    );
    assert_eq!(cancel, (Some(0), String::new(), String::new()));
    assert_eq!(
        pasted(&fx, "worker"),
        "⟦ae:msg from lead⟧\nwithdrawn: --digest-only",
        "the explicit override is the envelope's name"
    );
    let events = fx.events();
    let (body_file, _) = fx.record();
    assert!(
        body_file.contains("/ae-9.cancel."),
        "the action and ref name the record: {body_file}"
    );
    assert!(
        events[0].ends_with(&format!(
            "\"actor\":\"lead\",\"action\":\"cancel\",\"target\":\"worker\",\"ref\":\"ae-9\",\"actor_slot\":\"main\",\"actor_session\":\"trss\",\"target_slot\":\"worker.0\",\"target_session\":\"trss\",\"summary\":\"withdrawn\",\"body_file\":\"{body_file}\"}}"
        )),
        "{}",
        events[0]
    );
    // A pane-less caller naming itself, as the telegram bridge does.
    fx.forget();
    let bridged = fx.run(
        ae::cli::SEND,
        None,
        &["worker", "from", "the", "bridge"],
        &[("AE_SENDER_OVERRIDE", "telegram:42")],
    );
    assert_eq!(bridged, (Some(0), String::new(), String::new()));
    assert_eq!(
        pasted(&fx, "worker"),
        "⟦ae:msg from telegram:42⟧\nfrom the bridge"
    );
    let last = fx.events().pop().unwrap_or_default();
    assert!(
        last.contains("\"actor\":\"telegram:42\",\"action\":\"send\",\"target\":\"worker\",\"summary\":\"⟦ae:msg from telegram:42⟧ from the bridge\""),
        "{last}"
    );
    // A pane-less caller with no name at all: the EVENT says `human`, the
    // ENVELOPE says `unverified`. Bare is the human's signature, and nothing
    // through a helper may wear it.
    fx.forget();
    let nobody = fx.run(ae::cli::SEND, None, &["worker", "anon"], &[]);
    assert_eq!(nobody, (Some(0), String::new(), String::new()));
    assert_eq!(pasted(&fx, "worker"), "⟦ae:msg from unverified⟧\nanon");
    let last = fx.events().pop().unwrap_or_default();
    assert!(
        last.contains("\"actor\":\"human\",\"action\":\"send\""),
        "{last}"
    );
}

#[test]
fn send_refuses_exactly_records_nothing_on_a_failed_delivery_and_names_the_gap_after_a_confirmed_one()
 {
    let fx = Tracked::new("sr");
    let cases: Vec<ReplyCase<'_>> = vec![
        (Some(&fx.main), vec![], Some(2), ae::send::USAGE.to_owned()),
        (
            Some(&fx.main),
            vec!["worker"],
            Some(2),
            ae::send::USAGE.to_owned(),
        ),
        (
            Some(&fx.main),
            vec!["worker", "  "],
            Some(1),
            ae::tracked::refusal("send"),
        ),
        (
            Some(&fx.main),
            vec!["nobody", "x"],
            Some(1),
            "Error: agent 'nobody' not found in session 'trsr'\n".to_owned(),
        ),
    ];
    for (pane, tail, code, stderr) in cases {
        fx.forget();
        let out = fx.run(ae::cli::SEND, pane, &tail, &[]);
        assert_eq!(out, (code, String::new(), stderr), "{tail:?}");
        assert!(
            fx.received_now("worker").is_empty(),
            "{tail:?}: something was pasted"
        );
        assert!(fx.events().is_empty(), "{tail:?}: an event was written");
    }
    // A DEAD TARGET: refused with the frozen line, nothing stored, nothing
    // recorded.
    fx.forget();
    let dead = fx.dead_pane_session("claude");
    let out = ae()
        .env("TMUX", format!("{},0,0", fx.sock.display()))
        .env("TMUX_PANE", &fx.main)
        .arg(ae::cli::SEND)
        .arg(&dead)
        .args(["ghost", "late"])
        .output()
        .expect("the ae binary should run");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).starts_with("ae: send to ghost REFUSED —"),
        "{out:?}"
    );
    assert!(!dead.join("events.jsonl").exists());
    // An external sink: the event, nothing pasted, no body file.
    fx.forget();
    let sink = fx.run(ae::cli::SEND, Some(&fx.main), &["telegram:42", "hi"], &[]);
    assert_eq!(sink, (Some(0), String::new(), String::new()));
    assert!(fx.received_now("worker").is_empty());
    let last = fx.events().pop().unwrap_or_default();
    assert!(
        last.ends_with(
            "\"actor\":\"lead\",\"action\":\"send\",\"target\":\"telegram:42\",\"summary\":\"hi\"}"
        ),
        "{last}"
    );
}

#[test]
fn a_confirmed_delivery_whose_event_cannot_be_written_is_reported_as_that_gap() {
    let fx = Tracked::new("sg");
    // A ledger that cannot be appended to: a directory in its place.
    std::fs::create_dir_all(fx.dir.join("events.jsonl"))
        .expect("a directory in the ledger's place");
    let gap = fx.run(ae::cli::SEND, Some(&fx.main), &["worker", "delivered"], &[]);
    assert_eq!(gap.0, Some(1), "{gap:?}");
    assert!(
        gap.2
            .starts_with("ae: send to worker was delivered but its event was not emitted: "),
        "{gap:?}"
    );
    assert_eq!(
        pasted(&fx, "worker"),
        "⟦ae:msg from lead⟧\ndelivered",
        "the paste happened, once, before the gap"
    );
}

/// `_AE_EVENT_ACTION=chat` takes the frozen emitter's chat arm on both paths:
/// the summary keeps its newlines and tabs and is capped at 3500 characters,
/// neither flattened nor cut at 200 — on the pane path the record's envelope
/// line stays a line of its own.
#[test]
fn a_chat_send_keeps_its_lines_and_its_own_cap_on_the_pane_and_at_the_sink() {
    let fx = Tracked::new("chat");
    let line = "x".repeat(120);
    let payload = format!("first {line}\nsecond\tline {line}\nthird");
    let pane = fx.run(
        ae::cli::SEND,
        Some(&fx.main),
        &["worker", payload.as_str()],
        &[("_AE_EVENT_ACTION", "chat")],
    );
    assert_eq!(pane, (Some(0), String::new(), String::new()));
    let events = fx.events();
    assert_eq!(events.len(), 1, "{events:?}");
    let expected = format!(
        "\"action\":\"chat\",\"target\":\"worker\",\"summary\":\"⟦ae:msg from lead⟧\\nfirst {line}\\nsecond\\tline {line}\\nthird\",\"body_file\":"
    );
    assert!(events[0].contains(&expected), "{}", events[0]);
    fx.forget();
    let over = format!("{payload}\n{}", "y".repeat(3600));
    let sink = fx.run(
        ae::cli::SEND,
        Some(&fx.main),
        &["telegram:42", over.as_str()],
        &[("_AE_EVENT_ACTION", "chat")],
    );
    assert_eq!(sink, (Some(0), String::new(), String::new()));
    assert!(
        fx.received_now("worker").is_empty(),
        "a sink pastes nothing"
    );
    let last = fx.events().pop().unwrap_or_default();
    let kept = 3500 - payload.chars().count() - 1;
    let expected = format!(
        "\"actor\":\"lead\",\"action\":\"chat\",\"target\":\"telegram:42\",\"summary\":\"first {line}\\nsecond\\tline {line}\\nthird\\n{}\"}}",
        "y".repeat(kept)
    );
    assert!(
        last.ends_with(&expected),
        "3500 characters, lines kept: {}…",
        &last[..last.len().min(160)]
    );
}

#[test]
fn the_telegram_daemon_entry_names_what_it_actually_needs_and_never_stutters() {
    // Two presentation defects a unit test cannot see, both found by running
    // the binary: the shared missing-operand line told a machine-global entry
    // to supply a SESSION meta directory, and the startup refusal printed
    // "telegram: telegram:" because the error type already names its subsystem.
    let missing = ae().arg("_telegram-run").output().expect("the binary runs");
    assert_eq!(missing.status.code(), Some(2));
    let said = String::from_utf8_lossy(&missing.stderr);
    assert_eq!(
        said.trim_end(),
        "ae: _telegram-run needs an ae home directory"
    );
    assert!(
        String::from_utf8_lossy(&missing.stdout).is_empty(),
        "a diagnostic must not reach stdout (SC-022)"
    );

    // And the per-session entries keep the phrase that is right for THEM.
    let watchdog = ae().arg("_watchdog-run").output().expect("the binary runs");
    assert_eq!(
        String::from_utf8_lossy(&watchdog.stderr).trim_end(),
        "ae: _watchdog-run needs a session meta directory"
    );

    // A startup refusal: one "telegram:", named once, with the path that is
    // wrong and nothing else. NO NETWORK — the refusal happens before any
    // client is built.
    let empty = std::env::temp_dir().join(format!("ae-tg-entry-{}", std::process::id()));
    std::fs::create_dir_all(&empty).expect("a temp ae home");
    let refused = ae()
        .arg("_telegram-run")
        .arg(&empty)
        .output()
        .expect("the binary runs");
    assert_eq!(refused.status.code(), Some(1));
    let refusal = String::from_utf8_lossy(&refused.stderr);
    assert!(
        refusal.starts_with("ae: telegram: unreadable config "),
        "{refusal}"
    );
    assert_eq!(refusal.matches("telegram:").count(), 1, "{refusal}");
    std::fs::remove_dir_all(&empty).ok();
}

/// The `ae next` fixture: a real isolated server holding two ae-marked
/// sessions, one wanting a human louder than the other.
///
/// Both halves are needed and neither is decoration. The tmux server makes the
/// sessions RUNNING — the status is the whole stopped-exclusion, and a planted
/// directory alone classifies as `unknown`, which `next` skips for a different
/// reason and would let this fixture pass while proving nothing. The `AE_SESSION`
/// marker is what makes them ae's rather than someone's.
///
/// `nx-hot` raises `dead` structurally: its roster names a seat with no pane.
/// `nx-mild` raises `blocked` from its own ledger and carries the ONLY activity
/// timestamp in the fixture, so severity has to beat recency for `nx-hot` to
/// win — a selection that merely sorted by recency would answer `nx-mild`.
struct NextFixture {
    root: std::path::PathBuf,
    scratch: std::path::PathBuf,
    socket: std::path::PathBuf,
    hot_pane: String,
}

impl NextFixture {
    /// Build it. Every step panics loudly on failure, like the `#[test]`
    /// callers it feeds: a fixture that quietly half-built itself would report
    /// as a product defect somewhere further down.
    #[allow(
        clippy::expect_used,
        reason = "a fixture that cannot build must panic where it broke, not later"
    )]
    fn plant(tag: &str) -> Self {
        let scratch = std::path::PathBuf::from(format!("/tmp/aenx.{tag}.{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("a scratch directory");
        // `<scratch>/tmux-<uid>/default` — the exact path a bare `tmux` derives
        // from `$TMUX_TMPDIR`, so pointing that at the scratch directory makes
        // THIS server the ambient one. The jump test needs that shape, because
        // it must run with `$TMUX` absent to be provably outside tmux.
        let uid = std::os::unix::fs::MetadataExt::uid(
            &std::fs::metadata(&scratch).expect("the scratch directory exists"),
        );
        let socket_dir = scratch.join(format!("tmux-{uid}"));
        std::fs::create_dir_all(&socket_dir).expect("a socket directory");
        // tmux refuses a socket directory anyone else can reach — "has unsafe
        // permissions" — so the fixture makes it exactly as private as the one
        // tmux would have made for itself.
        std::fs::set_permissions(
            &socket_dir,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .expect("a private socket directory");
        let socket = socket_dir.join("default");
        let server = ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(socket.clone()));
        let tmux = |tail: &[&str]| {
            let mut args = ae::tmux::server_args(&server);
            args.extend(tail.iter().map(|arg| (*arg).to_owned()));
            run_tmux(&args, &scratch)
        };

        let root = scratch.join("home");
        std::fs::create_dir_all(root.join("sessions")).expect("a scratch state root");
        let pane_of = |name: &str, roster: &str| {
            assert!(
                tmux(&["-f", "/dev/null", "new-session", "-d", "-s", name]).0,
                "creating {name} must succeed"
            );
            assert!(tmux(&["set-environment", "-t", name, "AE_SESSION", name]).0);
            let (_, pane) = tmux(&["display-message", "-p", "-t", name, "#{pane_id}"]);
            let pane = pane.trim().to_owned();
            assert!(pane.starts_with('%'), "a pane id, got {pane:?}");
            assert!(tmux(&["set-option", "-p", "-t", &pane, "@ae_slot", "main"]).0);
            assert!(tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", "lead"]).0);
            let dir = root.join("sessions").join(name);
            std::fs::create_dir_all(&dir).expect("a session directory");
            std::fs::write(
                dir.join("meta"),
                format!(
                    "session={name}\nmode=local\nseat.main=lead\nprofile.main=cl\n{roster}\
                     tmux_server_kind=socket\ntmux_server={}\n",
                    socket.display()
                ),
            )
            .expect("a meta file");
            pane
        };

        let hot_pane = pane_of("nx-hot", "seat.worker.0=gone\nprofile.worker.0=cl\n");
        pane_of("nx-mild", "");
        std::fs::write(
            root.join("sessions").join("nx-mild").join("events.jsonl"),
            "{\"ts\":\"2026-08-27T08:00:00Z\",\"actor\":\"lead\",\"action\":\"state\",\
             \"ref\":\"blocked\",\"actor_slot\":\"main\",\"actor_session\":\"nx-mild\"}\n",
        )
        .expect("a planted ledger");

        Self {
            root,
            scratch,
            socket,
            hot_pane,
        }
    }

    /// `ae <tail>` against this fixture's state root, with `env` applied.
    #[allow(
        clippy::expect_used,
        reason = "a lane that cannot run the product binary must say so where it failed"
    )]
    fn run(&self, tail: &[&str], env: &[(&str, &str)]) -> (Option<i32>, String, String) {
        let mut command = ae();
        // A developer running this suite from inside tmux must not lend the
        // product their own server: every tmux fact here is the fixture's.
        command
            .env("AE_HOME", &self.root)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .args(tail);
        for (key, value) in env {
            command.env(key, value);
        }
        let out = command.output().expect("the ae binary should run");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// `$TMUX` as tmux itself writes it, pointing at this fixture's server so
    /// the product's BARE `tmux` is addressing it and no other.
    fn tmux_env(&self) -> String {
        format!("{},1,0", self.socket.display())
    }

    fn tear_down(&self) {
        let server =
            ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(self.socket.clone()));
        let mut args = ae::tmux::server_args(&server);
        args.push("kill-server".to_owned());
        let _ = run_tmux(&args, &self.scratch);
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

#[test]
fn next_names_the_top_running_session_needing_attention_under_both_spellings() {
    let fixture = NextFixture::plant("pick");
    let next = fixture.run(&["next"], &[]);
    let jump = fixture.run(&["jump"], &[]);
    fixture.tear_down();

    assert_eq!(next.0, Some(0), "{next:?}");
    assert_eq!(
        next.1, "nx-hot  attn:dead  rank:6  gone\n",
        "severity beats the fresher blocked session, and the line names the \
         agent that raised the reason: {next:?}"
    );
    assert!(next.2.is_empty(), "{next:?}");
    assert_eq!(jump, next, "`jump` is the same command, not a near-alias");
}

#[test]
fn next_refuses_when_no_running_session_needs_a_human() {
    // The whole fixture minus what makes either session ask for anything: the
    // roster seat with no pane, and the blocked declaration.
    let fixture = NextFixture::plant("quiet");
    std::fs::write(
        fixture.root.join("sessions").join("nx-hot").join("meta"),
        format!(
            "session=nx-hot\nmode=local\nseat.main=lead\nprofile.main=cl\n\
             tmux_server_kind=socket\ntmux_server={}\n",
            fixture.socket.display()
        ),
    )
    .expect("a rewritten meta");
    std::fs::remove_file(
        fixture
            .root
            .join("sessions")
            .join("nx-mild")
            .join("events.jsonl"),
    )
    .expect("the planted ledger goes");
    let quiet = fixture.run(&["next"], &[]);
    fixture.tear_down();

    assert_eq!(quiet.0, Some(1), "non-zero so it composes: {quiet:?}");
    assert!(quiet.1.is_empty(), "the refusal is never stdout: {quiet:?}");
    assert_eq!(
        quiet.2, "ae next: no running session needs attention.\n",
        "{quiet:?}"
    );
}

#[test]
fn the_next_argv_is_answered_before_any_session_is_looked_at() {
    // A state root that holds NOTHING: `--help` and a refused word must still
    // answer, and must not degrade into the unavailable `1`.
    //
    // The root is named rather than absent, which is a Z3 change of instrument
    // and not of subject. `ae` derives every path from `AE_HOME` or `HOME`, and
    // with neither there is no ae to run at all — the wrapper died on an
    // unbound `$HOME` under `set -u` in exactly the same case, and the core now
    // says so with `NO_STATE_ROOT` instead. Pointing at an empty directory
    // tests what this row is about — that the argv is answered before anything
    // is enumerated — more directly than removing the root did.
    let help = ae()
        .env_clear()
        .env("AE_HOME", "/nonexistent/ae")
        .args(["next", "--help"])
        .output()
        .expect("the ae binary should run");
    assert_eq!(help.status.code(), Some(0), "{:?}", help.status);
    assert!(help.stdout.is_empty(), "frozen wrote the usage to stderr");
    let expected = [
        "Usage: ae next [--attach]   Name the top running session needing attention.",
        "       ae jump [--attach]   Alias for ae next.",
        "",
        "Read-only by default: prints \"<session>  attn:<reason>  rank:<n>  <agent>\" and",
        "exits 0, or a message on stderr and non-zero when nothing needs attention.",
        "--attach (alias --switch) jumps to that session: switch-client inside tmux,",
        "attach-session outside.",
        "",
    ]
    .join("\n");
    assert_eq!(String::from_utf8_lossy(&help.stderr), expected);

    let refused = ae()
        .env_clear()
        .env("AE_HOME", "/nonexistent/ae")
        .args(["next", "--frobnicate"])
        .output()
        .expect("the ae binary should run");
    assert_eq!(refused.status.code(), Some(2), "{:?}", refused.status);
    assert!(refused.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&refused.stderr),
        "ae next: unknown argument '--frobnicate' (see: ae next --help)\n"
    );
}

#[test]
fn attach_refuses_a_session_the_server_it_would_jump_on_does_not_have() {
    // The re-validation between the scan and the jump, and the reason it is an
    // EXACT list-sessions match rather than a prefix-matching `has-session`.
    // Here the ambient server is a second, empty one: the chosen session is as
    // absent from it as an ended session would be, and the jump refuses instead
    // of focusing whatever else is there.
    let fixture = NextFixture::plant("elsewhere");
    let elsewhere = fixture.scratch.join("e");
    let other = ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(elsewhere.clone()));
    let mut create = ae::tmux::server_args(&other);
    create
        .extend(["-f", "/dev/null", "new-session", "-d", "-s", "nx-hotel"].map(ToOwned::to_owned));
    assert!(
        run_tmux(&create, &fixture.scratch).0,
        "the second server must come up"
    );

    let refused = fixture.run(
        &["next", "--attach"],
        &[("TMUX", &format!("{},1,0", elsewhere.display()))],
    );
    let mut kill = ae::tmux::server_args(&other);
    kill.push("kill-server".to_owned());
    let _ = run_tmux(&kill, &fixture.scratch);
    fixture.tear_down();

    assert_eq!(refused.0, Some(1), "{refused:?}");
    assert!(refused.1.is_empty(), "{refused:?}");
    assert_eq!(
        refused.2, "ae next: 'nx-hot' disappeared before attach.\n",
        "a prefix sibling on the server is not the session: {refused:?}"
    );
}

#[test]
fn attach_jumps_on_the_ambient_server_and_tmux_own_status_is_the_command_s() {
    // The jump itself. `TMUX_TMPDIR` points a BARE `tmux` at this fixture's
    // server, and `$TMUX` is absent — so the caller is provably OUTSIDE tmux and
    // the verb is `attach-session`, with no dependence on whether the lane
    // running this test has a controlling terminal.
    //
    // Nothing is attached here and the test's stdin is not a terminal, so tmux
    // refuses — which is the assertion. Frozen's last statement is tmux, so
    // TMUX'S status is the command's: what must be shown is that the refusal
    // came from tmux (a jump attempted on the chosen session) rather than from
    // ae (a jump abandoned before it started).
    let fixture = NextFixture::plant("jump");
    let attached = fixture.run(
        &["next", "--attach"],
        &[("TMUX_TMPDIR", &fixture.scratch.display().to_string())],
    );
    fixture.tear_down();

    assert!(
        attached.1.is_empty(),
        "the jump prints nothing of its own: {attached:?}"
    );
    assert_ne!(
        attached.0,
        Some(0),
        "an unattached server cannot be jumped to: {attached:?}"
    );
    assert!(
        !attached.2.contains("disappeared before attach")
            && !attached.2.contains("no running session needs attention"),
        "the selection ran and the session was there; the refusal is tmux's: {attached:?}"
    );
}

#[test]
fn attach_reports_being_already_there_rather_than_jumping_to_it() {
    // Frozen's inside-ness rule has TWO halves, and this pins the second: the
    // tty comparison that separates a real pane from an inherited `$TMUX` is
    // allowed to be UNANSWERABLE, and then `$TMUX` is trusted "as ae always
    // has" — a probe that cannot speak must not become a verdict.
    //
    // Here `ps` cannot answer: `PATH` leads to one that refuses. So the caller
    // is taken to be inside, the client's session IS the chosen one, and the
    // answer is the already-there line on STDOUT at 0 — never a jump.
    let fixture = NextFixture::plant("already");
    let bin = fixture.scratch.join("bin");
    std::fs::create_dir_all(&bin).expect("a shim directory");
    std::fs::write(bin.join("ps"), "#!/bin/sh\nexit 1\n").expect("a ps that refuses");
    #[allow(
        clippy::permissions_set_readonly_false,
        reason = "a fixture executable"
    )]
    std::fs::set_permissions(
        bin.join("ps"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("the shim is executable");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let already = fixture.run(
        &["next", "--attach"],
        &[
            ("PATH", &path),
            ("TMUX", &fixture.tmux_env()),
            ("TMUX_PANE", &fixture.hot_pane.clone()),
        ],
    );
    fixture.tear_down();

    assert_eq!(already.0, Some(0), "{already:?}");
    assert_eq!(
        already.1, "ae next: already in 'nx-hot' (attn:dead).\n",
        "frozen prints this one on stdout, not stderr: {already:?}"
    );
    assert!(already.2.is_empty(), "{already:?}");
}
