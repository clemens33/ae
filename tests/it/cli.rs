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
fn ae() -> std::process::Command {
    std::process::Command::new(env!("CARGO_BIN_EXE_ae"))
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
fn mkfifo(path: &std::path::Path) {
    let status = std::process::Command::new("mkfifo").arg(path).status();
    assert!(
        matches!(status, Ok(status) if status.success()),
        "a FIFO at {}",
        path.display()
    );
}

/// Wait at most `limit` for a spawned `child`: `Some(output)` if it exited,
/// `None` if it had to be killed. A test whose subject can hang must have a
/// red that ARRIVES, not one that stalls the lane.
fn bounded(
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
fn sc_022_an_unknown_option_exits_two_and_diagnoses_on_stderr() {
    let out = ae()
        .arg("--frobnicate")
        .output()
        .expect("the ae binary should run");

    assert_eq!(out.status.code(), Some(2), "exit status: {:?}", out.status);
    assert!(
        out.stdout.is_empty(),
        "stdout must stay empty for a machine caller, got {:?}",
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
    // And help names them, so a shipped surface is discoverable.
    let out = ae()
        .arg("--help")
        .output()
        .expect("the ae binary should run");
    let help = String::from_utf8(out.stdout).expect("stdout should be utf-8");
    for spelling in [ae::cli::REQUESTS, ae::cli::EVENTS_TAIL] {
        assert!(help.contains(spelling), "help omits {spelling}: {help}");
    }
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

/// A real isolated server with two stamped panes in one session, a session
/// directory whose `send` and `_send-deliver` are one STUB that records what
/// it was handed (and under which name) and answers like the frozen entry it
/// stands in for, and the binary run as a helper would run it. The stub is the point: the paste path is the frozen
/// bash body, exercised live in the smoke; what these tests pin is everything
/// the core does around it — composition, resolution, the env contract, the
/// event — and that a refused paste leaves no event behind.
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
        assert!(
            fixture
                .tmux(&["-f", "/dev/null", "new-session", "-d", "-s", &session])
                .0
        );
        assert!(fixture.tmux(&["split-window", "-d", "-t", &session]).0);
        let (_, panes) = fixture.tmux(&["list-panes", "-s", "-t", &session, "-F", "#{pane_id}"]);
        let ids: Vec<&str> = panes.lines().collect();
        assert_eq!(ids.len(), 2, "{panes}");
        let (main, worker) = (ids[0].to_owned(), ids[1].to_owned());
        for (pane, slot, agent) in [
            (&main, "main", "cl:lead"),
            (&worker, "worker.0", "cl:worker"),
        ] {
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
        assert!(
            std::fs::write(fixture.dir.join("meta"), format!("session={session}\n")).is_ok(),
            "a meta file"
        );
        // One script, installed under both names: `send` (the public helper,
        // which the frozen body makes record its own event) and
        // `_send-deliver` (the internal delivery-only entry, which prints the
        // recovery file's path). Which one ran is recorded, because that IS
        // the contract under test.
        let stub = r#"#!/bin/bash
here="$(cd "$(dirname "$0")" && pwd)"
me="$(basename "$0")"
printf '%s\n' "$me" >"$here/send.helper"
printf '%s\n' "$1" >"$here/send.target"
printf '%s' "$2" >"$here/send.message"
env | grep -E '^(_AE_DELIVER_ONLY|_AE_EVENT_ACTION|_AE_EVENT_REF|AE_SENDER_OVERRIDE)=' | LC_ALL=C sort >"$here/send.env"
rc="${STUB_SEND_RC:-0}"
if [[ "$rc" != 0 ]]; then echo "stub send: refused" >&2; exit "$rc"; fi
mkdir -p "$here/messages"
f="$here/messages/${_AE_EVENT_REF:-msg}.${_AE_EVENT_ACTION:-send}.stub.txt"
printf '%s' "$2" >"$f"
[[ "$me" == _send-deliver ]] && printf '%s\n' "$f"
exit 0
"#;
        for name in ["send", "_send-deliver"] {
            assert!(
                std::fs::write(fixture.dir.join(name), stub).is_ok(),
                "the stub {name}"
            );
            let executable = std::fs::set_permissions(
                fixture.dir.join(name),
                std::fs::Permissions::from_mode(0o755),
            );
            assert!(executable.is_ok(), "an executable stub {name}");
        }
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

    /// What the stub recorded: the target, the message, and the env lines.
    fn stub(&self) -> (String, String, String) {
        let read = |name: &str| std::fs::read_to_string(self.dir.join(name)).unwrap_or_default();
        (read("send.target"), read("send.message"), read("send.env"))
    }

    /// The name the stub was run under: `send` or `_send-deliver`, plus `\n`.
    fn stub_helper(&self) -> String {
        std::fs::read_to_string(self.dir.join("send.helper")).unwrap_or_default()
    }

    fn forget_stub(&self) {
        for name in ["send.target", "send.message", "send.env", "send.helper"] {
            let _ = std::fs::remove_file(self.dir.join(name));
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
fn stub_ref(env: &str) -> String {
    env.lines()
        .find_map(|line| line.strip_prefix("_AE_EVENT_REF="))
        .unwrap_or_else(|| panic!("the stub saw a ref: {env:?}"))
        .to_owned()
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
    let (target, message, env) = fx.stub();
    assert_eq!(
        target, "cl:worker\n",
        "the bare name resolved to the display ref"
    );
    let id = stub_ref(&env);
    assert!(is_request_id(&id, "ae"), "{id}");
    assert_eq!(
        fx.stub_helper(),
        "_send-deliver\n",
        "a tracked request goes through the internal delivery-only entry"
    );
    assert_eq!(
        env,
        format!("_AE_EVENT_ACTION=ask\n_AE_EVENT_REF={id}\n"),
        "the names the frozen body store reads, and NO ambient switch"
    );
    let reply_cmd = format!(
        "{}/reply --as \"cl:worker\" \"{id}\" \"<your reply>\"",
        fx.dir.display()
    );
    assert_eq!(
        message,
        ae::tracked::compose(
            ae::tracked::Kind::Ask,
            &id,
            "cl:lead",
            "the question",
            &reply_cmd
        )
    );
    assert!(message.starts_with(&format!(
        "REQUEST {id} from cl:lead: the question\n\nREQUIRED:"
    )));
    let events = fx.events();
    assert_eq!(events.len(), 1, "{events:?}");
    let body_file = fx.dir.join("messages").join(format!("{id}.ask.stub.txt"));
    assert!(
        events[0].ends_with(&format!(
            "\"actor\":\"cl:lead\",\"action\":\"ask\",\"target\":\"cl:worker\",\"ref\":\"{id}\",\"actor_slot\":\"main\",\"actor_session\":\"trask\",\"target_slot\":\"worker.0\",\"target_session\":\"trask\",\"summary\":\"the question\",\"body_file\":\"{}\"}}",
            body_file.display()
        )),
        "{}",
        events[0]
    );
    assert!(events[0].starts_with("{\"ts\":\""), "{}", events[0]);
    assert_eq!(
        std::fs::read_to_string(&body_file).unwrap(),
        message,
        "the event points at the stored delivered text"
    );
    // And the core's own requests surface reads it back as pending.
    let listed = fx.run(ae::cli::REQUESTS, Some(&fx.main), &["all"], &[]);
    assert_eq!(listed.0, Some(0), "{listed:?}");
    assert!(
        listed.1.contains("pending") && listed.1.contains(&id) && listed.1.contains("cl:worker"),
        "{listed:?}"
    );
}

#[test]
fn review_carries_its_instructions_and_every_target_spelling_resolves_as_the_helper_does() {
    let fx = Tracked::new("rev");
    let reviewed = fx.run(
        ae::cli::REVIEW,
        Some(&fx.worker),
        &["cl:lead", "look", "at", "x"],
        &[],
    );
    assert_eq!(reviewed, (Some(0), String::new(), String::new()));
    let (target, message, env) = fx.stub();
    assert_eq!(target, "cl:lead\n");
    assert_eq!(fx.stub_helper(), "_send-deliver\n");
    let id = stub_ref(&env);
    assert!(is_request_id(&id, "review"), "{id}");
    assert!(message.starts_with(&format!(
        "REVIEW REQUEST {id} from cl:worker: look at x\n\n{}\n\nREQUIRED",
        ae::tracked::REVIEW_INSTRUCTIONS
    )));
    assert!(message.ends_with(&format!("\n{}/reply --as \"cl:lead\" \"{id}\" \"<your review>\"\nDo not reply any other way. Do NOT use peek/peak as a reply mechanism.", fx.dir.display())));
    let events = fx.events();
    assert!(
        events[0].contains(&format!("\"actor\":\"cl:worker\",\"action\":\"review\",\"target\":\"cl:lead\",\"ref\":\"{id}\",\"actor_slot\":\"worker.0\",\"actor_session\":\"trrev\",\"target_slot\":\"main\",\"target_session\":\"trrev\",")),
        "{}",
        events[0]
    );

    // A pane id passes through and is read for its stamps; the explicit
    // own-session spelling resolves without the cross-session prefix; a unique
    // alias resolves too (each pane has a distinct one here).
    for (spelling, expected) in [
        (fx.main.as_str(), "cl:lead"),
        ("@trrev:lead", "cl:lead"),
        ("@trrev:cl:worker", "cl:worker"),
    ] {
        fx.forget_stub();
        let sent = fx.run(ae::cli::ASK, Some(&fx.worker), &[spelling, "q"], &[]);
        assert_eq!(sent.0, Some(0), "{spelling}: {sent:?}");
        assert_eq!(fx.stub().0, format!("{expected}\n"), "{spelling}");
    }
}

#[test]
fn a_request_that_does_not_resolve_or_is_refused_leaves_no_event_and_no_paste() {
    let fx = Tracked::new("ref");
    let cases: [Refusal<'_>; 8] = [
        (
            &["cl", "q"],
            &[],
            Some(1),
            "Error: ambiguous name 'cl' in session 'trref' — use alias:name format\n".to_owned(),
        ),
        (
            &["nobody", "q"],
            &[],
            Some(1),
            "Error: agent 'nobody' not found in session 'trref'\n".to_owned(),
        ),
        (
            &["@nosuch:cl:lead", "q"],
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
        (
            &["worker", "q"],
            &[("STUB_SEND_RC", "3")],
            Some(3),
            "stub send: refused\n".to_owned(),
        ),
    ];
    for (tail, envs, code, stderr) in cases {
        fx.forget_stub();
        let out = fx.run(ae::cli::ASK, Some(&fx.main), tail, envs);
        assert_eq!(out, (code, String::new(), stderr), "{tail:?}");
        assert!(
            fx.events().is_empty(),
            "{tail:?}: an event was written: {:?}",
            fx.events()
        );
        if envs.is_empty() {
            assert!(fx.stub().0.is_empty(), "{tail:?}: the stub was called");
        }
    }
    // The refused paste DID reach the stub, with the composed message — the
    // event is what was withheld.
    assert!(fx.stub().1.starts_with("REQUEST ae-"), "{:?}", fx.stub());
    // A session with a public `send` but no `_send-deliver` (helpers older
    // than this core) is said so, at 1, naming the entry that is missing —
    // and the public `send` is NOT used in its place, because that would
    // record a second event for the request.
    let bare = fx.scratch_dir.join("sessions").join("bare");
    std::fs::create_dir_all(&bare).expect("a session dir");
    std::fs::write(bare.join("meta"), "session=bare\n").expect("a meta file");
    std::fs::copy(fx.dir.join("send"), bare.join("send")).expect("the stub send");
    let out = ae()
        .env("TMUX", format!("{},0,0", fx.sock.display()))
        .env("TMUX_PANE", &fx.main)
        .arg(ae::cli::ASK)
        .arg(&bare)
        .args([&fx.main, "q"])
        .output()
        .expect("the ae binary should run");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("ask not delivered: could not run")
            && stderr.contains("_send-deliver")
            && stderr.contains("ae doctor --refresh"),
        "{out:?}"
    );
    assert!(
        !bare.join("send.helper").exists(),
        "the public send must not stand in for the missing entry"
    );
}

#[test]
fn no_identity_falls_back_to_a_plain_send_and_external_and_override_senders_are_event_only_or_slotless()
 {
    let fx = Tracked::new("idn");
    // No pane: the frozen warning, then a plain send of the raw body — the
    // helper writes its own event on that path, so the core writes none.
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
    assert_eq!(
        fx.stub_helper(),
        "send\n",
        "the fallback is the PUBLIC send, which records itself"
    );
    assert!(fx.events().is_empty());

    // An external sink: no paste, no body file, the event with the literal
    // target and the caller's slot.
    fx.forget_stub();
    let external = fx.run(ae::cli::ASK, Some(&fx.main), &["telegram:42", "hello"], &[]);
    assert_eq!(external, (Some(0), String::new(), String::new()));
    assert!(
        fx.stub().0.is_empty(),
        "the stub was called for an event-only sink"
    );
    let events = fx.events();
    assert_eq!(events.len(), 1);
    assert!(
        events[0].contains(
            "\"actor\":\"cl:lead\",\"action\":\"ask\",\"target\":\"telegram:42\",\"ref\":\"ae-"
        ) && events[0].ends_with(
            "\"actor_slot\":\"main\",\"actor_session\":\"tridn\",\"summary\":\"hello\"}"
        ),
        "{}",
        events[0]
    );

    // AE_SENDER_OVERRIDE: the actor is the override, slotless, and the env
    // reaches the helper for its provenance envelope.
    let bridged = fx.run(
        ae::cli::REVIEW,
        Some(&fx.main),
        &["worker", "from", "the", "bridge"],
        &[("AE_SENDER_OVERRIDE", "bridge")],
    );
    assert_eq!(bridged, (Some(0), String::new(), String::new()));
    let (_, message, env) = fx.stub();
    assert!(
        message.starts_with("REVIEW REQUEST review-")
            && message.contains(" from bridge: from the bridge\n"),
        "{message}"
    );
    assert!(env.contains("AE_SENDER_OVERRIDE=bridge\n"), "{env}");
    let events = fx.events();
    assert_eq!(events.len(), 2);
    assert!(
        events[1].contains(
            "\"actor\":\"bridge\",\"action\":\"review\",\"target\":\"cl:worker\",\"ref\":\"review-"
        ) && events[1].contains("\",\"actor_session\":\"tridn\",\"target_slot\":\"worker.0\",")
            && !events[1].contains("actor_slot"),
        "{}",
        events[1]
    );
}

// ---- tracked requests: reply ---------------------------------------------

/// An `ask` from the main pane to the worker, through the stub: its id.
fn ask_from_main(fx: &Tracked, body: &str) -> String {
    let asked = fx.run(ae::cli::ASK, Some(&fx.main), &["worker", body], &[]);
    assert_eq!(asked, (Some(0), String::new(), String::new()));
    let id = stub_ref(&fx.stub().2);
    assert!(is_request_id(&id, "ae"), "{id}");
    fx.forget_stub();
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
        fx.tmux(&[
            "set-option",
            "-p",
            "-t",
            &fx.main,
            "@ae_agent",
            "renamed:lead"
        ])
        .0
    );
    let replied = fx.run(
        ae::cli::REPLY,
        Some(&fx.worker),
        &[&id, "the", "answer"],
        &[],
    );
    assert_eq!(replied, (Some(0), String::new(), String::new()));
    let (target, message, env) = fx.stub();
    assert_eq!(fx.stub_helper(), "_send-deliver\n");
    assert_eq!(
        target, "renamed:lead\n",
        "routed by slot to the current name"
    );
    assert_eq!(message, format!("[{id}] the answer"));
    assert_eq!(
        env,
        format!("AE_SENDER_OVERRIDE=cl:worker\n_AE_EVENT_ACTION=reply\n_AE_EVENT_REF={id}\n"),
        "the verified sender rides to the entry as the envelope's identity"
    );
    let events = fx.events();
    assert_eq!(events.len(), 2, "{events:?}");
    let body_file = fx.dir.join("messages").join(format!("{id}.reply.stub.txt"));
    assert!(
        events[1].ends_with(&format!(
            "\"actor\":\"cl:worker\",\"action\":\"reply\",\"target\":\"renamed:lead\",\"ref\":\"{id}\",\"actor_slot\":\"worker.0\",\"actor_session\":\"{session}\",\"target_slot\":\"main\",\"target_session\":\"{session}\",\"summary\":\"the answer\",\"body_file\":\"{}\"}}",
            body_file.display()
        )),
        "{}",
        events[1]
    );
    assert_eq!(
        std::fs::read_to_string(&body_file).unwrap_or_default(),
        message,
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
    let as_reply = fx.run(
        ae::cli::REPLY,
        Some(&fx.worker),
        &["--as", "stale:old", &id2, "ok"],
        &[],
    );
    assert_eq!(
        as_reply,
        (
            Some(0),
            String::new(),
            "Warning: --as 'stale:old' != stored target name 'cl:worker' (name is advisory; slot verified)\n".to_owned()
        )
    );
    let events = fx.events();
    assert!(
        events[3]
            .contains("\"actor\":\"stale:old\",\"action\":\"reply\",\"target\":\"renamed:lead\""),
        "{}",
        events[3]
    );
    assert_eq!(
        fx.stub().2,
        format!("AE_SENDER_OVERRIDE=stale:old\n_AE_EVENT_ACTION=reply\n_AE_EVENT_REF={id2}\n"),
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
    let (target, message, env) = fx.stub();
    assert_eq!(target, "cl:lead\n");
    assert_eq!(message, format!("[{id}] clean"));
    assert_eq!(
        env,
        format!("AE_SENDER_OVERRIDE=cl:worker\n_AE_EVENT_ACTION=reply\n_AE_EVENT_REF={id}\n"),
        "the pane's verified identity, not the inherited one"
    );
    let last = fx.events().pop().unwrap_or_default();
    assert!(
        last.contains("\"actor\":\"cl:worker\",\"action\":\"reply\"") && !last.contains("spoof"),
        "{last}"
    );
    // A `--as` reply carries the --as identity, which is also what the event
    // names — the two never disagree.
    let id2 = ask_from_main(&fx, "again");
    let as_reply = fx.run(
        ae::cli::REPLY,
        Some(&fx.worker),
        &["--as", "cl:worker", &id2, "ok"],
        &[("AE_SENDER_OVERRIDE", "spoof")],
    );
    assert_eq!(as_reply, (Some(0), String::new(), String::new()));
    assert_eq!(
        fx.stub().2,
        format!("AE_SENDER_OVERRIDE=cl:worker\n_AE_EVENT_ACTION=reply\n_AE_EVENT_REF={id2}\n")
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
            vec!["--as", "cl:worker", &id, "x"],
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
            vec!["--as", "cl:worker", &id],
            Some(2),
            ae::reply::USAGE.to_owned(),
        ),
    ];
    for (pane, tail, code, stderr) in cases {
        fx.forget_stub();
        let out = fx.run(ae::cli::REPLY, pane, &tail, &[]);
        assert_eq!(out, (code, String::new(), stderr), "{tail:?}");
        assert!(fx.stub().0.is_empty(), "{tail:?}: something was pasted");
        assert_eq!(fx.events().len(), 1, "{tail:?}: an event was written");
    }
    // A paste the helper refused leaves the request OPEN: its status is the
    // helper's, verbatim, and no reply is recorded.
    fx.forget_stub();
    let refused = fx.run(
        ae::cli::REPLY,
        Some(&fx.worker),
        &[&id, "late"],
        &[("STUB_SEND_RC", "3")],
    );
    assert_eq!(refused.0, Some(3), "{refused:?}");
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
        r#"{"ts":"2026-01-01T00:00:00Z","actor":"cl:lead","action":"ask","target":"cl:worker","ref":"ae-old-1","summary":"old"}"#,
    );
    let old_cases: Vec<ReplyCase<'_>> = vec![
        (
            Some(&fx.worker),
            vec!["--as", "x:y", "ae-old-1", "x"],
            Some(1),
            "Error: override agent 'x:y' does not match assigned target 'cl:worker'\n".to_owned(),
        ),
        (
            Some(&fx.main),
            vec!["ae-old-1", "x"],
            Some(1),
            "Error: request 'ae-old-1' is assigned to 'cl:worker', current pane is 'cl:lead'\n"
                .to_owned(),
        ),
        (
            None,
            vec!["ae-old-1", "x"],
            Some(1),
            "Error: could not detect current agent identity; rerun with --as 'cl:worker' from the assigned agent context\n".to_owned(),
        ),
    ];
    for (pane, tail, code, stderr) in old_cases {
        fx.forget_stub();
        let out = fx.run(ae::cli::REPLY, pane, &tail, &[]);
        assert_eq!(out, (code, String::new(), stderr), "{tail:?}");
        assert!(fx.stub().0.is_empty(), "{tail:?}: something was pasted");
    }
    fx.forget_stub();
    let old = fx.run(ae::cli::REPLY, Some(&fx.worker), &["ae-old-1", "seen"], &[]);
    assert_eq!(old, (Some(0), String::new(), String::new()));
    assert_eq!(
        fx.stub().0,
        "cl:lead\n",
        "the stored name, no slot to route by"
    );
    let last = fx.events().pop().unwrap_or_default();
    assert!(
        last.contains(
            "\"actor\":\"cl:worker\",\"action\":\"reply\",\"target\":\"cl:lead\",\"ref\":\"ae-old-1\",\"actor_slot\":\"worker.0\",\"actor_session\":\"trro\",\"summary\":\"seen\""
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
            r#"{{"ts":"2026-08-27T10:00:00Z","actor":"cl:lead","action":"ask","target":"cl:worker","ref":"{id}","actor_slot":"main","actor_session":"trrb","target_slot":"worker.0","target_session":"trrb","summary":"bash asked","body_file":"/nonexistent"}}"#
        ),
    );
    let replied = fx.run(ae::cli::REPLY, Some(&fx.worker), &[id, "done"], &[]);
    assert_eq!(replied, (Some(0), String::new(), String::new()));
    assert_eq!(fx.stub().0, "cl:lead\n", "routed by the bash-stored slot");
    assert!(
        fx.events()[1].contains(&format!(
            "\"actor\":\"cl:worker\",\"action\":\"reply\",\"target\":\"cl:lead\",\"ref\":\"{id}\",\"actor_slot\":\"worker.0\",\"actor_session\":\"trrb\",\"target_slot\":\"main\",\"target_session\":\"trrb\",\"summary\":\"done\""
        )),
        "{}",
        fx.events()[1]
    );
    assert!(requests_all(&fx).contains("replied"));
    // A follow-up is delivered and recorded, and said so — never refused.
    fx.forget_stub();
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
    assert_eq!(fx.stub().1, format!("[{id}] and more"));
    assert_eq!(fx.events().len(), 3);
    // An asker with no pane — a bridge naming itself through
    // AE_SENDER_OVERRIDE — is answered as the frozen send answers a sink:
    // the event, nothing pasted, no body file.
    let bridged = "ae-20260827T100001Z-0badf00e";
    append_event(
        &fx,
        &format!(
            r#"{{"ts":"2026-08-27T10:00:01Z","actor":"telegram:42","action":"ask","target":"cl:worker","ref":"{bridged}","target_slot":"worker.0","target_session":"trrb","summary":"from the bridge"}}"#
        ),
    );
    fx.forget_stub();
    let to_bridge = fx.run(ae::cli::REPLY, Some(&fx.worker), &[bridged, "hello"], &[]);
    assert_eq!(to_bridge, (Some(0), String::new(), String::new()));
    assert!(fx.stub().0.is_empty(), "nothing is pasted to a sink");
    let last = fx.events().pop().unwrap_or_default();
    assert!(
        last.ends_with(&format!(
            "\"actor\":\"cl:worker\",\"action\":\"reply\",\"target\":\"telegram:42\",\"ref\":\"{bridged}\",\"actor_slot\":\"worker.0\",\"actor_session\":\"trrb\",\"summary\":\"hello\"}}"
        )),
        "{last}"
    );
    assert!(requests_all(&fx).contains("from the bridge") || requests_all(&fx).contains("hello"));
}
