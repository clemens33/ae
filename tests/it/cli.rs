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

    // Usage: 2, and nothing touched.
    for tail in [vec![], vec!["--clear", "extra"], vec!["\u{1b}"]] {
        let usage = run(&tail);
        assert_eq!(usage.0, Some(2), "{tail:?}: {usage:?}");
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
        vec!["read"],
        vec![],
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
    for tail in [vec!["Working"], vec!["blocked"], vec![]] {
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
