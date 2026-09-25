//! The GATE's own hazards — the two files ae still builds itself with.
//!
//! Three guards over the `justfile` and the `install` script — text this crate
//! never runs but every release does. Each one is a bug that already shipped.
//!
//! Every guard is a PURE FUNCTION of file text, and each is exercised against
//! deliberately broken input as well as the real file. A rule that matches
//! nothing is indistinguishable from a clean tree, so the red cases are the
//! half that makes a green run mean something.

#![allow(
    clippy::disallowed_methods,
    reason = "these read repository text; the capability boundary is about what PRODUCT \
              code may reach"
)]

use std::path::{Path, PathBuf};

use super::parity::{Invocation, capture::ExitOutcome, capture::raw};

/// The repository root — this file's crate manifest directory.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|why| panic!("{} must be readable: {why}", path.display()))
}

/// The EXECUTABLE text of one justfile recipe: body lines only, full-line
/// comments and blanks dropped, backslash continuations folded so one command
/// is one line.
fn recipe_text(justfile: &str, header: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut inside = false;
    for line in justfile.lines() {
        if !inside {
            inside = line.starts_with(header);
            continue;
        }
        if !line.starts_with([' ', '\t']) && !line.is_empty() {
            break;
        }
        let mut text = if buf.is_empty() {
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            trimmed.to_owned()
        } else {
            format!(" {}", line.trim_start())
        };
        let folded = text.trim_end();
        if let Some(head) = folded.strip_suffix('\\') {
            text = head.to_owned();
            buf.push_str(&text);
            continue;
        }
        buf.push_str(&text);
        out.push(std::mem::take(&mut buf));
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

/// Whether a bare `shellcheck` word starts at `at` in `line`.
fn bare_word_at(line: &str, at: usize, word: &str) -> bool {
    let boundary = |ch: char| !ch.is_ascii_alphanumeric() && !"_.@/-".contains(ch);
    let before = line[..at].chars().next_back().is_none_or(boundary);
    let after = line[at + word.len()..].chars().next().is_none_or(boundary);
    before && after
}

fn bare_word_count(text: &str, word: &str) -> usize {
    let mut count = 0;
    let mut from = 0;
    while let Some(at) = text[from..].find(word).map(|index| index + from) {
        if bare_word_at(text, at, word) {
            count += 1;
        }
        from = at + word.len();
    }
    count
}

/// Whether the `lint` recipe PROTECTS shellcheck's stdin.
fn lint_redirect_ok(justfile: &str) -> bool {
    let lines = recipe_text(justfile, "lint:");
    let joined = lines.join("\n");
    if bare_word_count(&joined, "shellcheck") != 1 {
        return false;
    }
    lines.iter().filter(|line| bearing(line)).count() == 1
}

/// Whether one line IS the shellcheck command and carries the stdin redirect.
fn bearing(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("shellcheck") else {
        return false;
    };
    let Some(head) = rest.trim_end().strip_suffix("/dev/null") else {
        return false;
    };
    let Some(head) = head.trim_end_matches([' ', '\t']).strip_suffix('<') else {
        return false;
    };
    let head = head.strip_suffix('0').unwrap_or(head);
    head.ends_with([' ', '\t']) && !head.contains([';', '&', '|'])
}

/// Whether the pin recipe asks whether shellcheck EXISTS before probing it.
fn pin_availability_ok(justfile: &str) -> bool {
    let lines = recipe_text(justfile, "_shellcheck-pin:");
    let first = |needle: &str, tail: &str| {
        lines.iter().position(|line| {
            line.find(needle).is_some_and(|at| {
                line[at + needle.len()..]
                    .trim_start()
                    .starts_with(tail.trim_start())
            })
        })
    };
    match (
        first("command -v", " shellcheck"),
        first("shellcheck", " --version"),
    ) {
        (Some(available), Some(probe)) => available < probe,
        _ => false,
    }
}

/// Whether the Rust test lane contains every boundary that keeps product tmux
/// probes away from a developer's server.
fn rust_test_tmux_isolation_ok(justfile: &str) -> bool {
    let lines = recipe_text(justfile, "_tmux-isolated lane *args:");
    let position = |needle: &str| lines.iter().position(|line| line.contains(needle));
    let required = [
        "lane=\"$1\"",
        "shift",
        "owner_is_dead()",
        "reap_sockets()",
        "reap_stale_lanes()",
        "reap_stale_lanes",
        "base=\"${AE_TEST_TMPDIR:-/tmp}\"",
        "owned_root()",
        "reap_registry()",
        "reap_dead_scratch()",
        "keep_or_remove()",
        "kept $1: could not be deleted (a writer may still hold it)",
        "then keep_or_remove \"$dir\"; fi",
        "mktemp -d \"$base/ae-rust-test.$$.XXXXXX\"",
        "until reap_registry \"$test_tmux_tmp\"; do",
        "error: kept $test_tmux_tmp",
        "exit $((status ? status : 1))",
        "export TMPDIR=\"$test_tmux_tmp/tmp\"",
        "export NEXTEST_TEST_THREADS=$((cpus < 8 ? cpus : 8))",
        "TMUX_TMPDIR=\"$test_tmux_tmp\" env -u TMUX -u TMUX_PANE tmux -L ae kill-server",
        "keep_or_remove \"$test_tmux_tmp\"",
        "trap cleanup EXIT",
        "export TMUX_TMPDIR=\"$test_tmux_tmp\"",
        "unset TMUX TMUX_PANE",
        "tmux -f /dev/null -L ae new-session -d -s foreign-review-sentry -e AE_SESSION=foreign-review-sentry",
        "detached cargo nextest run --locked --all-features",
        "detached cargo test --doc --locked --all-features",
        "detached cargo llvm-cov nextest --locked --all-features",
        "detached cargo mutants --cargo-arg=--locked --jobs 1 \"$@\"",
    ];
    if required.iter().any(|needle| position(needle).is_none()) {
        return false;
    }
    let Some(unset) = position("unset TMUX TMUX_PANE") else {
        return false;
    };
    let Some(reap) = lines
        .iter()
        .position(|line| line.trim() == "reap_stale_lanes")
    else {
        return false;
    };
    let Some(mktemp) = position("mktemp -d") else {
        return false;
    };
    let Some(sentry) = position("tmux -f /dev/null -L ae new-session") else {
        return false;
    };
    let Some(nextest) = position("cargo nextest run") else {
        return false;
    };
    let Some(doctest) = position("cargo test --doc") else {
        return false;
    };
    // The lane reaps its own registry BEFORE it deletes it, and the temp dir
    // is the lane's before any test runs.
    let (Some(registry), Some(remove), Some(tmpdir)) = (
        position("reap_registry \"$test_tmux_tmp\""),
        position("keep_or_remove \"$test_tmux_tmp\""),
        position("export TMPDIR="),
    ) else {
        return false;
    };
    let callers = [
        ("rust-test:", "just _tmux-isolated test"),
        ("rust-cov:", "just _tmux-isolated cov"),
        (
            "rust-mutants *args:",
            "just _tmux-isolated mutants {{ args }}",
        ),
    ];
    reap < mktemp
        && registry < remove
        && mktemp < tmpdir
        && tmpdir < nextest
        && unset < sentry
        && sentry < nextest
        && nextest < doctest
        && callers
            .iter()
            .all(|(header, command)| recipe_text(justfile, header) == [*command])
}

/// Whether the `test` arm of `_tmux-isolated` forwards its extra arguments
/// to nextest and, on a filtered run, skips the doctests with one stderr
/// note — so a targeted receipt never needs a hand-replicated lane (#204).
fn test_arm_forwards_filter_args(justfile: &str) -> bool {
    let lines = recipe_text(justfile, "_tmux-isolated lane *args:");
    let Some(open) = lines.iter().position(|line| line == "test)") else {
        return false;
    };
    let Some(len) = lines[open..].iter().position(|line| line == ";;") else {
        return false;
    };
    let arm = &lines[open..open + len];
    let position = |needle: &str| arm.iter().position(|line| line.contains(needle));
    let (Some(guard), Some(filtered), Some(note), Some(doctest)) = (
        position("if (( $#"),
        position("cargo nextest run --locked --all-features \"$@\""),
        position("skipping doctests"),
        position("cargo test --doc --locked --all-features"),
    ) else {
        return false;
    };
    let Some(bare) = arm
        .iter()
        .position(|line| line == "detached cargo nextest run --locked --all-features")
    else {
        return false;
    };
    let order = [guard, filtered, note, bare, doctest];
    order.windows(2).all(|pair| pair[0] < pair[1]) && arm[note].contains(">&2")
}

#[test]
fn the_lint_recipe_protects_shellchecks_stdin() {
    assert!(
        lint_redirect_ok(&read(&root().join("justfile"))),
        "the real lint recipe must redirect shellcheck's stdin from /dev/null"
    );

    // RED — the comment-as-evidence lie: the only protected text is a comment.
    assert!(!lint_redirect_ok(
        "lint:\n    # shellcheck -x install < /dev/null\n    shellcheck -x install\n"
    ));
    // RED — the second-call lie: the redirect sits on a command doing no work.
    assert!(!lint_redirect_ok(
        "lint:\n    shellcheck install; shellcheck install < /dev/null\n"
    ));
    // RED — the plain regression this exists to catch.
    assert!(!lint_redirect_ok("lint:\n    shellcheck -x install\n"));
    // RED — the control-operator family.
    for tail in ["&& true", "|| true", "| cat", "& wait"] {
        assert!(
            !lint_redirect_ok(&format!(
                "lint:\n    shellcheck -x install {tail} < /dev/null\n"
            )),
            "'{tail}' takes the redirect"
        );
    }
    // RED — the numeric-fd family: the descriptor redirected is not stdin.
    for prefix in ["1", "2"] {
        assert!(
            !lint_redirect_ok(&format!(
                "lint:\n    shellcheck -x install {prefix}< /dev/null\n"
            )),
            "'{prefix}<' does not redirect stdin"
        );
    }
    // GREEN — the explicit spelling of the same thing IS stdin.
    assert!(lint_redirect_ok(
        "lint:\n    shellcheck -x install 0< /dev/null\n"
    ));
    // GREEN control — a folded multi-line invocation is still recognised, so
    // the guard is not passing by being blind to the shape the recipe uses.
    assert!(lint_redirect_ok(
        "lint:\n    shellcheck -x install \\\n        tests/x \\\n        y < /dev/null\n"
    ));
}

#[test]
fn the_pin_recipe_asks_whether_shellcheck_exists_before_probing_it() {
    assert!(
        pin_availability_ok(&read(&root().join("justfile"))),
        "the real pin recipe must test availability before the version probe"
    );
    // RED — the shape that shipped and was caught in review: probe first, no
    // availability test, so an absent binary kills the recipe at rc 127.
    assert!(!pin_availability_ok(
        "_shellcheck-pin:\n    want=\"0.11.0\"\n    have=\"$(shellcheck --version)\"\n"
    ));
}

#[test]
fn the_rust_test_recipe_isolates_every_real_tmux_probe() {
    assert!(
        rust_test_tmux_isolation_ok(&read(&root().join("justfile"))),
        "every Rust test tool must run through one fresh private -L ae server and clean it up"
    );

    // RED — merely unsetting the inherited client still reaches the ordinary
    // socket directory and therefore the developer's named ae server.
    assert!(!rust_test_tmux_isolation_ok(
        "rust-test:\n    unset TMUX TMUX_PANE\n    cargo nextest run --locked --all-features\n    cargo test --doc --locked --all-features\n"
    ));
    // RED — an isolated run that never kills its server or removes its socket
    // directory leaves state behind and can contaminate a later run.
    assert!(!rust_test_tmux_isolation_ok(
        "rust-test:\n    test_tmux_tmp=\"$(mktemp -d \"${TMPDIR:-/tmp}/ae-rust-test.$$.XXXXXX\")\"\n    export TMUX_TMPDIR=\"$test_tmux_tmp\"\n    unset TMUX TMUX_PANE\n    tmux -L ae new-session -d -s foreign-review-sentry -e AE_SESSION=foreign-review-sentry\n    cargo nextest run --locked --all-features\n    cargo test --doc --locked --all-features\n"
    ));
}

#[test]
fn the_test_arm_forwards_filter_args_to_nextest_and_skips_the_doctests() {
    assert!(
        test_arm_forwards_filter_args(&read(&root().join("justfile"))),
        "a filtered `just _tmux-isolated test` run must reach nextest, and skip the doctests"
    );

    // RED — today's arm takes no arguments: a worker needing a targeted
    // receipt must hand-replicate the lane, which killed the fleet server.
    assert!(!test_arm_forwards_filter_args(
        "_tmux-isolated lane *args:\n    case \"$lane\" in\n        test)\n            detached cargo nextest run --locked --all-features\n            detached cargo test --doc --locked --all-features\n            ;;\n    esac\n"
    ));
    // RED — forwarded but never skipped: a filtered run pays for the doctests.
    assert!(!test_arm_forwards_filter_args(
        "_tmux-isolated lane *args:\n    case \"$lane\" in\n        test)\n            detached cargo nextest run --locked --all-features \"$@\"\n            detached cargo test --doc --locked --all-features\n            ;;\n    esac\n"
    ));
}

struct StaleLane {
    root: PathBuf,
    socket: PathBuf,
}

impl Drop for StaleLane {
    fn drop(&mut self) {
        let _ = raw::run(
            &Invocation::new("tmux")
                .arg("-S")
                .arg(&self.socket)
                .arg("kill-server"),
            &self.root,
            &self.root.join("cleanup-out"),
            &self.root.join("cleanup-err"),
        );
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn stale_lane_owner_child() {
    if std::env::var_os("AE_GATE_STALE_LANE_OWNER_CHILD").is_none() {
        return;
    }
    let pid_file =
        std::env::var_os("AE_GATE_STALE_LANE_PID_FILE").expect("the stale-lane child pid file");
    std::fs::write(pid_file, std::process::id().to_string())
        .expect("the stale-lane child pid file");
    let scratch = std::env::temp_dir();
    let _ = raw::run(
        &Invocation::new("kill")
            .arg("-KILL")
            .arg(std::process::id().to_string()),
        &scratch,
        &scratch.join("stale-lane-kill-out"),
        &scratch.join("stale-lane-kill-err"),
    );
    panic!("SIGKILL must end the stale-lane owner child");
}

#[test]
fn stale_lane_sweep_reaps_a_nested_socket_owned_by_a_dead_lane() {
    let scratch_root = super::cli::OwnedScratch::root("gate", "stale-lane").keep();
    let child = std::env::current_exe().expect("the integration test binary");
    let status = raw::run(
        &Invocation::new(child)
            .arg("--exact")
            .arg("gate::stale_lane_owner_child")
            .env("AE_GATE_STALE_LANE_OWNER_CHILD", "1")
            .env(
                "AE_GATE_STALE_LANE_PID_FILE",
                scratch_root.join("owner-pid"),
            ),
        &scratch_root,
        &scratch_root.join("owner-out"),
        &scratch_root.join("owner-err"),
    )
    .expect("the stale-lane owner child starts");
    assert!(matches!(status.outcome(), ExitOutcome::Signalled));
    let owner = std::fs::read_to_string(scratch_root.join("owner-pid"))
        .expect("the stale-lane owner pid")
        .trim()
        .parse::<u32>()
        .expect("the stale-lane owner pid is numeric");
    let stale = scratch_root.join(format!("ae-rust-test.{owner}.stale"));
    let socket = stale.join("nested/socket");
    std::fs::create_dir_all(socket.parent().expect("a nested socket parent"))
        .expect("a nested socket parent");
    let fixture = StaleLane {
        root: scratch_root.clone(),
        socket: socket.clone(),
    };
    let created = raw::run(
        &Invocation::new("tmux")
            .arg("-S")
            .arg(&socket)
            .arg("new-session")
            .arg("-d")
            .arg("-s")
            .arg("stale-lane"),
        &stale,
        &stale.join("create-out"),
        &stale.join("create-err"),
    )
    .expect("the stale-lane server starts");
    assert!(matches!(created.outcome(), ExitOutcome::Code(0)));
    assert!(socket.exists(), "the stale lane owns a nested tmux socket");

    let swept = raw::run(
        &Invocation::new("just")
            .arg("_tmux-isolated")
            .arg("unknown")
            .env("TMPDIR", &scratch_root)
            .env("AE_TEST_TMPDIR", &scratch_root),
        &root(),
        &scratch_root.join("sweep-out"),
        &scratch_root.join("sweep-err"),
    )
    .expect("the stale-lane sweep runs");
    assert!(matches!(swept.outcome(), ExitOutcome::Code(2)));
    assert!(
        !stale.exists(),
        "the next lane must kill nested stale sockets before it creates its own lane"
    );
    drop(fixture);
}

/// Whether both lanes cap their test threads in CONFIG (#159): nextest at most
/// eight, every mutation run at most four.
fn thread_caps_ok(nextest: &str, mutants: &str) -> bool {
    let cap = |text: &str, key: &str, max: u32| {
        text.lines().any(|line| {
            line.trim()
                .strip_prefix(key)
                .and_then(|rest| rest.trim_end_matches(['"', ']']).parse::<u32>().ok())
                .is_some_and(|threads| (1..=max).contains(&threads))
        })
    };
    cap(nextest, "test-threads = ", 8)
        && cap(
            mutants,
            "additional_cargo_test_args = [\"--test-threads=",
            4,
        )
}

#[test]
fn the_lanes_cap_their_test_threads_in_config() {
    assert!(thread_caps_ok(
        &read(&root().join(".config/nextest.toml")),
        &read(&root().join(".cargo/mutants.toml"))
    ));
    let mutants = "additional_cargo_test_args = [\"--test-threads=4\"]\n";
    // RED — nextest's own default is every core; so is an explicit wide one.
    assert!(!thread_caps_ok("[profile.default]\n", mutants));
    assert!(!thread_caps_ok("test-threads = 18\n", mutants));
    // RED — a mutation run with no cap of its own.
    assert!(!thread_caps_ok(
        "test-threads = 8\n",
        "timeout_multiplier = 5.0\n"
    ));
    assert!(!thread_caps_ok(
        "test-threads = 8\n",
        "additional_cargo_test_args = [\"--test-threads=8\"]\n"
    ));
}

/// The child half of the killed-test pins: a scratch root under the base the
/// parent names, optionally holding a tmux server, then SIGKILL — no unwinding,
/// so no destructor runs.
#[test]
fn scratch_sigkill_child() {
    let Some(base) = std::env::var_os("AE_GATE_SCRATCH_KILL_BASE") else {
        return;
    };
    let pid = std::process::id();
    let root = PathBuf::from(base)
        .join(format!("ae-it-{pid}"))
        .join("killed");
    std::fs::create_dir_all(&root).expect("the killed child's root");
    let report = std::env::var_os("AE_GATE_SCRATCH_KILL_REPORT").expect("the report path");
    std::fs::write(report, pid.to_string()).expect("the report");
    if let Some(name) = std::env::var_os("AE_GATE_SCRATCH_KILL_SERVER") {
        let started = raw::run(
            &Invocation::new("tmux")
                .arg("-f")
                .arg("/dev/null")
                .arg("-L")
                .arg(name)
                .arg("new-session")
                .arg("-d"),
            &root,
            &root.join("start-out"),
            &root.join("start-err"),
        )
        .expect("the killed child's server starts");
        assert!(matches!(started.outcome(), ExitOutcome::Code(0)));
    }
    let _ = raw::run(
        &Invocation::new("kill").arg("-KILL").arg(pid.to_string()),
        &root,
        &root.join("kill-out"),
        &root.join("kill-err"),
    );
    panic!("SIGKILL must end the scratch child");
}

/// Run [`scratch_sigkill_child`] under `base`, its registry in `lane`; the
/// dead child's pid.
fn killed_scratch_child(base: &Path, lane: &Path, server: Option<&str>) -> String {
    let report = base.join("child-pid");
    let mut child = Invocation::new(
        std::env::current_exe().unwrap_or_else(|why| panic!("the integration test binary: {why}")),
    )
    .arg("--exact")
    .arg("gate::scratch_sigkill_child")
    .env("AE_GATE_SCRATCH_KILL_BASE", base)
    .env("AE_GATE_SCRATCH_KILL_REPORT", &report)
    .env("TMUX_TMPDIR", lane);
    if let Some(name) = server {
        child = child.env("AE_GATE_SCRATCH_KILL_SERVER", name);
    }
    let status = raw::run(
        &child,
        base,
        &base.join("child-out"),
        &base.join("child-err"),
    )
    .unwrap_or_else(|why| panic!("the killed child starts: {why}"));
    assert!(matches!(status.outcome(), ExitOutcome::Signalled));
    read(&report).trim().to_owned()
}

/// A test killed by `SIGKILL` while it owns a scratch root and a tmux server leaves
/// neither: a dead lane's is swept when the next lane starts, the running
/// lane's when it exits — waiting out an owner that outlives `cargo` — and an
/// unregistered root of a dead owner goes too.
#[test]
fn a_killed_tests_scratch_and_server_are_swept_by_the_lane() {
    let base = super::cli::OwnedScratch::root("gate", "sweep");
    let name = format!("aeitkill{}", std::process::id());
    let lane = base.join("lane");
    std::fs::create_dir_all(&lane).expect("the killed child's lane");
    let registered = killed_scratch_child(&base, &lane, Some(&name));
    killed_scratch_child(&base, &base.join("no-lane"), None);
    // The lane is named for a dead owner only now that one exists.
    let stale = base.join(format!("ae-rust-test.{registered}.killed"));
    std::fs::rename(&lane, &stale).expect("the dead lane");
    // The lane's own `cargo`: the killed child again, registered in THIS lane,
    // and a registered owner still alive when `cargo` returns.
    let cargo = base.join("bin").join("cargo");
    std::fs::create_dir_all(base.join("bin")).expect("the fake cargo's dir");
    std::fs::write(
        &cargo,
        "#!/bin/sh\n[ \"$1\" = nextest ] || exit 0\n\
         \"$AE_GATE_EXE\" --exact gate::scratch_sigkill_child >/dev/null 2>&1\n\
         sleep 3 & late=$!\nroot=\"$AE_TEST_TMPDIR/ae-it-$late/late\"\n\
         mkdir -p \"$root\" \"$TMUX_TMPDIR/.ae-parity-fixtures/$late\"\n\
         ln -s \"$root\" \"$TMUX_TMPDIR/.ae-parity-fixtures/$late/0\"\n",
    )
    .expect("the fake cargo");
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755))
            .expect("an executable fake cargo");
    }
    let path = std::env::var("PATH").unwrap_or_default();
    let exe = std::env::current_exe().expect("the integration test binary");

    let swept = raw::run(
        &Invocation::new("just")
            .arg("_tmux-isolated")
            .arg("test")
            .env("PATH", format!("{}:{path}", base.join("bin").display()))
            .env("AE_GATE_EXE", exe)
            .env("AE_GATE_SCRATCH_KILL_BASE", &*base)
            .env("AE_GATE_SCRATCH_KILL_REPORT", base.join("exit-pid"))
            .env("AE_GATE_SCRATCH_KILL_SERVER", format!("{name}-exit"))
            .env("TMPDIR", &*base)
            .env("AE_TEST_TMPDIR", &*base),
        &root(),
        &base.join("sweep-out"),
        &base.join("sweep-err"),
    )
    .expect("the lane sweep runs");
    let alive = raw::run(
        &Invocation::new("pgrep").arg("-f").arg(&name),
        &base,
        &base.join("pgrep-out"),
        &base.join("pgrep-err"),
    )
    .is_ok_and(|status| matches!(status.outcome(), ExitOutcome::Code(0)));
    if alive {
        let _ = raw::run(
            &Invocation::new("pkill").arg("-f").arg(&name),
            &base,
            &base.join("pkill-out"),
            &base.join("pkill-err"),
        );
    }
    assert!(matches!(swept.outcome(), ExitOutcome::Code(0)));
    assert!(
        !read(&base.join("exit-pid")).trim().is_empty(),
        "the lane ran the killed child"
    );
    assert!(
        !alive,
        "the killed test's tmux server -L {name} outlived the sweep"
    );
    let left: Vec<String> = std::fs::read_dir(&*base)
        .expect("the sweep base")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|entry| entry.starts_with("ae-it-") || entry.starts_with("ae-rust-test."))
        .collect();
    assert!(left.is_empty(), "scratch outlived the lanes: {left:?}");
}

/// The cleanup-race pin's own unwritable directories, released even on a panic:
/// without the chmod a `remove_dir_all` fails, and every later lane would name
/// the same residue forever.
struct Unwritable(Vec<PathBuf>);

impl Drop for Unwritable {
    fn drop(&mut self) {
        for root in &self.0 {
            let _ = raw::run(
                &Invocation::new("chmod").arg("-R").arg("u+w").arg(root),
                root.parent().unwrap_or(root),
                &root.join("chmod-out"),
                &root.join("chmod-err"),
            );
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

/// #165: a cleanup race never fails an all-green lane. Two DEAD owners hold a
/// root `rm -rf` cannot empty — a stale lane dir (the start sweep) and an
/// unregistered scratch root (the exit reap) — and a fake `cargo` is the lane's
/// only verdict, green. Both roots are NAMED and kept, and the lane still ends
/// 0; the unfixed recipe died at the first failed rm.
#[test]
fn a_cleanup_race_never_fails_an_all_green_lane() {
    let base = super::cli::OwnedScratch::root("gate", "race");
    let owner = killed_scratch_child(&base, &base.join("no-lane"), None);
    let dead = base.join(format!("ae-it-{owner}"));
    let stale = base.join(format!("ae-rust-test.{owner}.stale"));
    // The child's own `killed` dir and the stale lane's `late`, each holding a
    // byte no `rm -rf` can unlink — a live writer's effect, every run.
    for held in [dead.join("killed"), stale.join("late")] {
        std::fs::create_dir_all(&held).expect("the held directory");
        std::fs::write(held.join("held"), "a writer's byte").expect("the held byte");
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o555))
                .expect("an unwritable directory");
        }
    }
    let _guard = Unwritable(vec![dead.clone(), stale.clone()]);
    let cargo = base.join("bin").join("cargo");
    std::fs::create_dir_all(base.join("bin")).expect("the fake cargo's dir");
    std::fs::write(&cargo, "#!/bin/sh\nexit 0\n").expect("the fake cargo");
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755))
            .expect("an executable fake cargo");
    }
    let path = std::env::var("PATH").unwrap_or_default();
    let swept = raw::run(
        &Invocation::new("just")
            .arg("_tmux-isolated")
            .arg("test")
            .env("PATH", format!("{}:{path}", base.join("bin").display()))
            .env("TMPDIR", &*base)
            .env("AE_TEST_TMPDIR", &*base),
        &root(),
        &base.join("race-out"),
        &base.join("race-err"),
    )
    .expect("the lane runs");
    let stderr = read(&base.join("race-err"));
    assert!(
        matches!(swept.outcome(), ExitOutcome::Code(0)),
        "a cleanup race must not fail an all-green lane: {stderr}"
    );
    for kept in [&dead, &stale] {
        assert!(
            stderr.contains(&format!("note: kept {}", kept.display())),
            "the kept root must be named: {stderr}"
        );
    }
}

/// A fake `cargo` in `<base>/bin` for the terminal pin; the `PATH` that finds it
/// first. It exits `$AE_GATE_EXIT` when set, else reports whether `/dev/tty`
/// opens and its pid, then sleeps.
fn fake_tty_cargo(base: &Path) -> String {
    use std::os::unix::fs::PermissionsExt as _;
    let cargo = base.join("bin").join("cargo");
    std::fs::create_dir_all(base.join("bin"))
        .and_then(|()| {
            std::fs::write(
                &cargo,
                "#!/bin/sh\n[ \"$1\" = nextest ] || exit 0\n[ -z \"$AE_GATE_EXIT\" ] || exit \"$AE_GATE_EXIT\"\n\
                 if (: </dev/tty) 2>/dev/null; then t=tty; else t=none; fi\n\
                 echo \"$t $$\" >\"$AE_TEST_TMPDIR/tty.tmp\" && mv \"$AE_TEST_TMPDIR/tty.tmp\" \"$AE_TEST_TMPDIR/tty\"\n\
                 exec sleep 300\n",
            )
        })
        .and_then(|()| std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)))
        .unwrap_or_else(|why| panic!("the fake cargo: {why}"));
    format!(
        "{}:{}",
        base.join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

/// The text of `file` once something wrote it, or empty after a minute.
fn wait_written(file: &Path) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_mins(1);
    loop {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        if !text.is_empty() || std::time::Instant::now() > deadline {
            return text;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// A lane started in a terminal hands cargo NO controlling terminal (#149), so
/// no test can take `just test`'s terminal for a pane it is in. Ctrl-C there
/// still ends cargo, through the lane, and the lane ends as cargo did.
#[test]
fn a_lane_run_in_a_terminal_detaches_cargo_and_still_stops_on_ctrl_c() {
    let base = super::cli::OwnedScratch::root("gate", "tty");
    let path = fake_tty_cargo(&base);
    let lane = |invocation: Invocation| {
        invocation
            .env("PATH", &path)
            .env("TMPDIR", &*base)
            .env("AE_TEST_TMPDIR", &*base)
    };
    let run = |invocation: Invocation, name: &str| {
        raw::run(
            &lane(invocation),
            &base,
            &base.join(format!("{name}-out")),
            &base.join(format!("{name}-err")),
        )
        .unwrap_or_else(|why| panic!("{name} runs: {why}"))
        .outcome()
    };
    let wait = |file: &str| wait_written(&base.join(file));

    // A failing cargo fails the lane with cargo's own status.
    let failed = run(
        Invocation::new("just")
            .arg("--working-directory")
            .arg(root())
            .arg("--justfile")
            .arg(root().join("justfile"))
            .arg("_tmux-isolated")
            .arg("test")
            .env("AE_GATE_EXIT", "7"),
        "failing",
    );
    assert!(matches!(failed, ExitOutcome::Code(7)));

    // A tmux pane is a real controlling terminal, and C-c there signals the
    // pane's foreground process group: what a developer's Ctrl-C does.
    let socket = base.join("s");
    let started = run(
        Invocation::new("tmux")
            .arg("-f")
            .arg("/dev/null")
            .arg("-S")
            .arg(&socket)
            .arg("new-session")
            .arg("-d")
            .arg("-s")
            .arg("lane")
            .arg("-c")
            .arg(root())
            .arg("--")
            .arg("sh")
            .arg("-c")
            .arg(
                "trap : INT; just _tmux-isolated test; echo $? >\"$AE_TEST_TMPDIR/rc.tmp\"; \
                 mv \"$AE_TEST_TMPDIR/rc.tmp\" \"$AE_TEST_TMPDIR/rc\"",
            ),
        "pane",
    );
    assert!(matches!(started, ExitOutcome::Code(0)));
    let seen = wait("tty");
    let (tty, pid) = seen.trim().split_once(' ').unwrap_or(("", ""));
    let _ = run(
        Invocation::new("tmux")
            .arg("-S")
            .arg(&socket)
            .arg("send-keys")
            .arg("-t")
            .arg("lane")
            .arg("C-c"),
        "ctrl-c",
    );
    let rc = wait("rc");
    let alive = matches!(
        run(Invocation::new("kill").arg("-0").arg(pid), "alive"),
        ExitOutcome::Code(0)
    );
    if alive {
        let _ = run(Invocation::new("kill").arg("-KILL").arg(pid), "reap");
    }
    assert_eq!(tty, "none", "cargo ran with the lane's terminal: {seen:?}");
    assert!(!alive, "Ctrl-C left the lane's cargo {pid} running");
    assert!(
        !matches!(rc.trim(), "" | "0"),
        "a Ctrl-C'd lane must end nonzero, got {rc:?}"
    );
    let lanes: Vec<String> = std::fs::read_dir(&*base)
        .expect("the lane base")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|entry| entry.starts_with("ae-rust-test."))
        .collect();
    assert!(
        lanes.is_empty(),
        "the Ctrl-C'd lane was not reaped: {lanes:?}"
    );
}

/// Whether the `release` recipe refreshes the fuzz crate's lock inside the
/// version commit, and cannot refuse the release when it does not.
///
/// The fuzz crate is outside the workspace, so the bump's own rewrite never
/// reaches its lock — and that lock records the ae version. Refreshed too early
/// it records the OLD version; refreshed after the commit it needs a second
/// one; refreshed without a probe it turns a missing DEV toolchain into a
/// failed release.
fn fuzz_lock_refresh_ok(justfile: &str) -> bool {
    let lines = recipe_text(justfile, "release:");
    let position = |needle: &str| lines.iter().position(|line| line.contains(needle));
    let (Some(bump), Some(refresh), Some(commit)) = (
        position("just bump"),
        position("metadata --manifest-path fuzz/Cargo.toml"),
        position("chore(release):"),
    ) else {
        return false;
    };
    // The bump writes the version the refresh records, and the commit that
    // carries the bump carries the refreshed lock with it.
    if !(bump < refresh && refresh < commit) {
        return false;
    }
    // ONE condition: the refresh runs only when the fuzz nightly answers.
    if !lines[refresh].contains("rustup run") {
        return false;
    }
    // And nothing from the refresh to the commit may refuse.
    lines[refresh..commit]
        .iter()
        .all(|line| bare_word_count(line, "exit") == 0)
}

#[test]
fn the_release_recipe_refreshes_the_fuzz_lock_without_being_able_to_refuse() {
    assert!(
        fuzz_lock_refresh_ok(&read(&root().join("justfile"))),
        "the release must carry the refreshed fuzz lock in its version commit, and \
         a missing fuzz nightly must warn rather than fail the release"
    );

    // RED — no refresh at all: the fuzz lane refuses on a stale lock after every
    // release until someone refreshes it by hand.
    assert!(!fuzz_lock_refresh_ok(
        "release:\n    VERSION=$(just bump)\n    git commit -m \"chore(release): $TAG\"\n"
    ));
    // RED — refreshed BEFORE the bump, so the lock records the version the
    // release is replacing.
    assert!(!fuzz_lock_refresh_ok(
        "release:\n    if rustup run nightly rustc --version && cargo +nightly metadata --manifest-path fuzz/Cargo.toml; then git add fuzz/Cargo.lock; fi\n    VERSION=$(just bump)\n    git commit -m \"chore(release): $TAG\"\n"
    ));
    // RED — unguarded: a laptop without the fuzz nightly cannot release.
    assert!(!fuzz_lock_refresh_ok(
        "release:\n    VERSION=$(just bump)\n    cargo +nightly metadata --manifest-path fuzz/Cargo.toml --format-version 1 >/dev/null\n    git commit -m \"chore(release): $TAG\"\n"
    ));
    // RED — guarded, in order, and still able to refuse.
    assert!(!fuzz_lock_refresh_ok(
        "release:\n    VERSION=$(just bump)\n    if rustup run nightly rustc --version && cargo +nightly metadata --manifest-path fuzz/Cargo.toml; then git add fuzz/Cargo.lock; else exit 1; fi\n    git commit -m \"chore(release): $TAG\"\n"
    ));
}

/// Whether the fuzz lane BOUNDS the duration it hands libFuzzer, on every route.
///
/// libFuzzer reads `-max_total_time` into an int and treats zero as NO LIMIT, so
/// a digits-only guard was never one: `00` is all digits and is not the literal
/// `0`, and a value past the int range wrapped. The test has to be ANCHORED and
/// bounded, it has to run before the value reaches cargo-fuzz, and the sweep
/// must reach the fuzzer through the same guarded recipe rather than around it.
fn fuzz_secs_guard_ok(justfile: &str) -> bool {
    let lines = recipe_text(justfile, "rust-fuzz target secs=");
    let position = |needle: &str| lines.iter().position(|line| line.contains(needle));
    let (Some(guard), Some(run)) = (
        position("=~ ^[1-9][0-9]{0,5}$"),
        position("-max_total_time="),
    ) else {
        return false;
    };
    // A digits-only test, anywhere in the recipe, accepts `00`.
    if lines.iter().any(|line| line.contains("*[!0-9]*")) {
        return false;
    }
    if guard >= run {
        return false;
    }
    let sweep = recipe_text(justfile, "rust-fuzz-all secs=");
    sweep.iter().any(|line| line.contains("just rust-fuzz "))
        && sweep.iter().all(|line| !line.contains("cargo +"))
}

#[test]
fn the_fuzz_lane_bounds_every_duration_it_hands_libfuzzer() {
    assert!(
        fuzz_secs_guard_ok(&read(&root().join("justfile"))),
        "an unbounded fuzz run is the one thing this lane must not be able to start"
    );

    // GREEN on a synthetic lane, so every red case below differs by ONE rule and
    // a green run cannot come from a check that matches nothing.
    assert!(fuzz_secs_guard_ok(
        "rust-fuzz target secs=\"secs=60\":\n    if ! [[ $secs =~ ^[1-9][0-9]{0,5}$ ]]; then exit 2; fi\n    cargo +nightly fuzz run \"$target\" -- -max_total_time=\"$secs\"\n\nrust-fuzz-all secs=\"secs=60\":\n    for target in $TARGETS; do just rust-fuzz \"target=$target\" \"$secs\"; done\n"
    ));

    // RED — the digits-only guard that shipped in the first round: `00` is all
    // digits and is not the literal `0`, so it passed and the run had no bound.
    assert!(!fuzz_secs_guard_ok(
        "rust-fuzz target secs=\"secs=60\":\n    case \"$secs\" in\n        '' | 0 | *[!0-9]*) exit 2 ;;\n    esac\n    cargo +nightly fuzz run \"$target\" -- -max_total_time=\"$secs\"\n\nrust-fuzz-all secs=\"secs=60\":\n    for target in $TARGETS; do just rust-fuzz \"target=$target\" \"$secs\"; done\n"
    ));
    // RED — the anchored test is there, but the loose one survives beside it.
    assert!(!fuzz_secs_guard_ok(
        "rust-fuzz target secs=\"secs=60\":\n    if ! [[ $secs =~ ^[1-9][0-9]{0,5}$ ]]; then exit 2; fi\n    case \"$secs\" in\n        *[!0-9]*) exit 2 ;;\n    esac\n    cargo +nightly fuzz run \"$target\" -- -max_total_time=\"$secs\"\n\nrust-fuzz-all secs=\"secs=60\":\n    for target in $TARGETS; do just rust-fuzz \"target=$target\" \"$secs\"; done\n"
    ));
    // RED — guarded, but only after the value has already reached cargo-fuzz.
    assert!(!fuzz_secs_guard_ok(
        "rust-fuzz target secs=\"secs=60\":\n    cargo +nightly fuzz run \"$target\" -- -max_total_time=\"$secs\"\n    if ! [[ $secs =~ ^[1-9][0-9]{0,5}$ ]]; then exit 2; fi\n\nrust-fuzz-all secs=\"secs=60\":\n    for target in $TARGETS; do just rust-fuzz \"target=$target\" \"$secs\"; done\n"
    ));
    // RED — the sweep runs the fuzzer itself, so ITS duration is never guarded.
    assert!(!fuzz_secs_guard_ok(
        "rust-fuzz target secs=\"secs=60\":\n    if ! [[ $secs =~ ^[1-9][0-9]{0,5}$ ]]; then exit 2; fi\n    cargo +nightly fuzz run \"$target\" -- -max_total_time=\"$secs\"\n\nrust-fuzz-all secs=\"secs=60\":\n    cargo +nightly fuzz run \"$target\" -- -max_total_time=\"$secs\"\n"
    ));
}

/// Whether the Linux container lanes keep their safety shape.
///
/// `rust-linux` runs THE GATE on real Linux (native arm64) inside a local
/// container, and its hazards are the ones a container gate can quietly grow: a
/// moving base image tag, a writable checkout mount, a tty that lets a test
/// take the lane (#149), a root container whose mode-bit tests lie, a rebuilt
/// image whose uid no longer matches the volumes it is handed, a rustup-init
/// fetched from the moving dist URL whose digest changes with every release,
/// and an arch parameter that would silently promise `x86_64` evidence the
/// dropped amd64 path could never deliver. The smoke recipe must keep proving
/// the RELEASED musl bundle without growing a per-release download pin.
fn linux_lanes_ok(justfile: &str) -> bool {
    let lines = recipe_text(justfile, "rust-linux:");
    let contains = |needle: &str| lines.iter().any(|line| line.contains(needle));
    // The recipe takes NO parameter: an arch flag would promise amd64 evidence
    // the QEMU path cannot produce (the smoke covers the bundle instead).
    if !recipe_text(justfile, "rust-linux arch=").is_empty()
        || lines.iter().any(|line| line.contains("amd64"))
    {
        return false;
    }
    // The base image is a digest pin, and the build's FROM is that pin — never
    // a tag a remote can move under us.
    if !justfile.contains("LINUX_IMAGE := \"ubuntu:24.04@sha256:")
        || !contains("FROM {{ LINUX_IMAGE }}")
    {
        return false;
    }
    // rustup-init comes from the versioned archive URL, whose digests hold.
    if !justfile.contains("RUSTUP_VERSION := \"")
        || !contains("rustup/archive/{{ RUSTUP_VERSION }}/")
    {
        return false;
    }
    // Caches are named volumes carrying the arch they serve, on every mount.
    // Counted as occurrences, because the recipe's folded continuations put
    // several mounts on one line.
    if lines
        .iter()
        .map(|line| line.matches("ae-linux-arm64-").count())
        .sum::<usize>()
        < 4
    {
        return false;
    }
    // The image is built with the HOST's uid: without the build arg it rebuilds
    // as the default 1000 (ubuntu 24.04's own user) and cannot write its volumes.
    if !contains("--build-arg UID=\"$(id -u)\"") {
        return false;
    }
    // --init: the gate's bash is PID 1 without it — the kernel drops SIGTERM to
    // a PID 1 with no handler, so docker stop waits 10 s and then SIGKILLs, and
    // orphaned test processes reparent to a bash that may not reap them, so a
    // zombie still answers ps and kill -0 to a liveness proof. (Measured: a
    // Ctrl-C on the docker CLI ends the run in NEITHER configuration.)
    if !contains("--init") {
        return false;
    }
    // The container lane runs the WHOLE suite on the bounded linux profile:
    // fail-fast off, so one run lists every Linux-only failure, and the hang
    // bound ends what would otherwise stall the lane forever.
    if !contains("-e NEXTEST_PROFILE=linux") {
        return false;
    }
    // No tty on a run (docker build's -t is its tag flag, not a terminal): a
    // test must not be able to take the lane's terminal (#149). The checkout is
    // bound READ-ONLY and the container does not run as root — mode-bit tests
    // would lie.
    let has_tty = lines.iter().any(|line| {
        line.contains("docker run")
            && line
                .split_whitespace()
                .any(|word| matches!(word, "-t" | "-it" | "--tty"))
    });
    if has_tty || !contains(":$repo:ro") || !contains("USER \"$UID\"") {
        return false;
    }
    // No second copy of the gate: the recipe calls the repo's own recipes.
    if !contains("just rust-setup") || !contains("just test") {
        return false;
    }
    // The target volume is shared by EVERY checkout (#190): cargo judges the
    // `ae` package fresh when a new tree's sources predate the last build, so
    // the container rebuilds that package from the mounted checkout before the
    // gate runs, while dependency artifacts stay shared and warm.
    let joined = lines.join("\n");
    let Some(container_at) = joined.find("bash -c") else {
        return false;
    };
    let container = &joined[container_at..];
    match (
        container.find("cargo clean --locked -p ae"),
        container.find("just test"),
    ) {
        (Some(clean), Some(test)) if clean < test => {}
        _ => return false,
    }
    // The smoke runs the RELEASED bundle from ./dist, verified against the
    // manifest `just bundles` wrote — it fetches nothing, so it never needs a
    // per-release digest pin, and a missing bundle names its builder.
    let smoke = recipe_text(justfile, "rust-linux-smoke:");
    let smoke_has = |needle: &str| smoke.iter().any(|line| line.contains(needle));
    smoke_has("dist/SHA256SUMS")
        && smoke_has("just bundles")
        && smoke_has("--platform linux/amd64")
        && smoke_has("just version")
        && !smoke
            .iter()
            .any(|line| line.contains("https://") || line.contains("curl"))
}

#[test]
fn the_linux_container_lanes_are_digest_pinned_read_only_and_non_root() {
    let green = concat!(
        "LINUX_IMAGE := \"ubuntu:24.04@sha256:008173\"\n",
        "RUSTUP_VERSION := \"1.29.1\"\n",
        "rust-linux:\n",
        "    for v in cargo rustup target; do docker volume create \"ae-linux-arm64-$v\" >/dev/null; done\n",
        "    curl -o \"$lane/rustup-init\" https://static.rust-lang.org/rustup/archive/{{ RUSTUP_VERSION }}/rustup-init\n",
        "    repo=\"$PWD\"\n",
        "    mounts=(-v \"$repo:$repo:ro\")\n",
        "    docker build --build-arg UID=\"$(id -u)\" -t ae-linux-arm64 - <<'DOCKERFILE'\n",
        "    FROM {{ LINUX_IMAGE }}\n",
        "    USER \"$UID\"\n",
        "    DOCKERFILE\n",
        "    docker run --rm --init \"${mounts[@]}\" -v \"ae-linux-arm64-cargo:/c\" \\\n",
        "        -v \"ae-linux-arm64-rustup:/r\" -v \"ae-linux-arm64-target:/t\" -e NEXTEST_PROFILE=linux \\\n",
        "        ae-linux-arm64 \\\n",
        "        bash -c 'just rust-setup\n",
        "        cargo clean --locked -p ae\n",
        "        just test'\n",
        "\n",
        "rust-linux-smoke:\n",
        "    version=\"$(just version)\"\n",
        "    line=\"$(grep -F \" ae-$version-linux-x86_64-musl.tar.gz\" dist/SHA256SUMS)\"\n",
        "    [ -f \"dist/ae-$version-linux-x86_64-musl.tar.gz\" ] || { echo \"run: just bundles\" >&2; exit 1; }\n",
        "    out=\"$(docker run --rm --platform linux/amd64 ae-smoke /run/ae-core --version)\"\n",
        "    [ \"$out\" = \"ae $version\" ]\n",
    );
    assert!(
        linux_lanes_ok(green),
        "the synthetic green lane must satisfy every rule, so each red below is one rule"
    );

    // RED — the base image pinned by tag: a remote can move it under the gate.
    assert!(!linux_lanes_ok(&green.replace(
        "LINUX_IMAGE := \"ubuntu:24.04@sha256:008173\"",
        "LINUX_IMAGE := \"ubuntu:24.04\""
    )));
    // RED — a tty on the gate run: a test could take the lane's terminal (#149).
    assert!(!linux_lanes_ok(&green.replace(
        "docker run --rm --init \"${mounts[@]}\"",
        "docker run --rm --init -t \"${mounts[@]}\""
    )));
    // RED — the checkout mount made writable: a run could touch the live tree.
    assert!(!linux_lanes_ok(&green.replace(
        "mounts=(-v \"$repo:$repo:ro\")",
        "mounts=(-v \"$repo:$repo\")"
    )));
    // RED — a root container: root ignores mode bits, so ae's 0555/0444 and
    // permission-refusal tests would lie.
    assert!(!linux_lanes_ok(&green.replace("    USER \"$UID\"\n", "")));
    // RED — the gate steps inlined into the recipe: a second copy of the gate
    // drifts from `just test` and the pin block stops being the one source.
    assert!(!linux_lanes_ok(&green.replace(
        "bash -c 'just rust-setup\n        cargo clean --locked -p ae\n        just test'",
        "bash -c 'cargo fmt --all --check && cargo nextest run'"
    )));
    // RED — no --init: the gate's bash is PID 1 and swallows signals and orphans.
    assert!(!linux_lanes_ok(&green.replace(
        "docker run --rm --init \"${mounts[@]}\"",
        "docker run --rm \"${mounts[@]}\""
    )));
    // RED — the unbounded default profile: fail-fast would again hide every
    // Linux-only failure behind the first, and a hang would stall the lane.
    assert!(!linux_lanes_ok(&green.replace(
        "-e NEXTEST_PROFILE=linux",
        "-e NEXTEST_PROFILE=default"
    )));
    // RED — volumes without the arch they serve, or a build without the host
    // uid: the run cannot write its own cache, or shares it with a foreign arch.
    assert!(!linux_lanes_ok(
        &green.replace("ae-linux-arm64-", "ae-linux-")
    ));
    assert!(!linux_lanes_ok(&green.replace(
        "docker build --build-arg UID=\"$(id -u)\"",
        "docker build"
    )));
    // RED — an arch parameter is back: it would promise amd64 evidence the
    // dropped QEMU path cannot produce.
    assert!(!linux_lanes_ok(
        &green.replace("rust-linux:\n", "rust-linux arch=\"arm64\":\n")
    ));
    // RED — a smoke that downloads a pinned release: the digest goes stale at
    // every release and the pin becomes a lie.
    assert!(!linux_lanes_ok(&green.replace(
        "line=\"$(grep -F \" ae-$version-linux-x86_64-musl.tar.gz\" dist/SHA256SUMS)\"",
        "curl -o /tmp/ae.tar.gz https://example.com/ae.tar.gz"
    )));
    // RED — no clean of the ae package: the shared target volume could serve
    // another checkout's build (#190).
    assert!(!linux_lanes_ok(
        &green.replace("        cargo clean --locked -p ae\n", "")
    ));
    // RED — the clean after the gate: the suite already ran on stale binaries.
    assert!(!linux_lanes_ok(&green.replace(
        "        cargo clean --locked -p ae\n        just test'",
        "        just test'\n        cargo clean --locked -p ae"
    )));
    // RED — the clean on the host side: it must run inside the container.
    assert!(!linux_lanes_ok(&green.replace(
        "        bash -c 'just rust-setup\n        cargo clean --locked -p ae",
        "        cargo clean --locked -p ae\n        bash -c 'just rust-setup"
    )));

    assert!(linux_lanes_ok(&read(&root().join("justfile"))));
}

/// The joined, comment-free text the portability rules read.
fn installer_lines(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for line in source.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        buf.push_str(line);
        let folded = buf.trim_end();
        if folded.ends_with("||") || folded.ends_with('\\') {
            continue;
        }
        out.push(std::mem::take(&mut buf));
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

/// What may legally follow the flag letter inside its option cluster.
#[derive(Clone, Copy)]
enum Tail {
    /// The flag takes an argument, which GNU allows ATTACHED — `stat -c%Y`,
    /// `date -dyesterday`.
    Arg,
    /// The flag takes NO argument, so cluster letters may legally FOLLOW it:
    /// `grep -Po` as well as `grep -oP`.
    NoArg,
    /// The argument must be separate — but `-i''` and `-i""` DO count, because
    /// the shell strips the quotes and what reaches sed is a bare `-i`, the
    /// GNU-only spelling that breaks BSD.
    WordFinal,
}

/// Whether `line` calls `cmd` with `flag` set, options in ANY order.
fn calls_with_flag(line: &str, cmd: &str, flag: char, tail: Tail) -> bool {
    let mut from = 0;
    while let Some(at) = line[from..].find(cmd).map(|index| index + from) {
        from = at + cmd.len();
        // A command token begins at start of line, after whitespace, or after a
        // shell operator, and may carry a PATH prefix — so `/usr/bin/date -d`
        // is caught while `iso-date -d` is not.
        let before = line[..at].chars().next_back();
        let starts = match before {
            None => true,
            Some('/') => {
                line[..at]
                    .trim_end_matches(|ch: char| ch != ' ' && ch != '\t')
                    .len()
                    <= at
            }
            Some(ch) => ch.is_whitespace() || "|&;(`".contains(ch),
        };
        if !starts {
            continue;
        }
        let mut rest = &line[at + cmd.len()..];
        while rest.starts_with([' ', '\t']) {
            let word = rest.trim_start_matches([' ', '\t']);
            let Some(cluster) = word.strip_prefix('-') else {
                break;
            };
            let end = cluster.find([' ', '\t']).unwrap_or(cluster.len());
            if cluster_hits(&cluster[..end], flag, tail) {
                return true;
            }
            rest = &cluster[end..];
        }
    }
    false
}

/// Whether an option cluster (the text after its leading `-`) sets `flag`.
fn cluster_hits(cluster: &str, flag: char, tail: Tail) -> bool {
    let Some(at) = cluster.find(flag) else {
        return false;
    };
    if !cluster[..at].chars().all(|ch| ch.is_ascii_alphabetic()) {
        return false;
    }
    let after = &cluster[at + flag.len_utf8()..];
    match tail {
        Tail::Arg => true,
        Tail::NoArg => after.chars().all(|ch| ch.is_ascii_alphabetic()),
        Tail::WordFinal => matches!(after, "" | "''" | "\"\""),
    }
}

/// Every GNU-only form `source` still carries, by label.
fn portability_flags(source: &str) -> Vec<&'static str> {
    let lines: Vec<String> = installer_lines(source)
        .into_iter()
        // The inline exemption marker is dropped HERE rather than in the reader,
        // so this rule is exercised by the mutation cases too: a filter only the
        // real run passes through is a rule no test can falsify.
        .filter(|line| !line.contains("port-ok:"))
        .collect();
    let mut bad = Vec::new();
    // An `unless` is a REVIEWED pair on the same line — an explicit `command
    // -v` test or the portable spelling beside the GNU one.
    let mut flag = |label: &'static str, hit: &dyn Fn(&str) -> bool, unless: &str| {
        if lines
            .iter()
            .any(|line| hit(line) && (unless.is_empty() || !line.contains(unless)))
        {
            bad.push(label);
        }
    };
    flag(
        "stat-c",
        &|line| calls_with_flag(line, "stat", 'c', Tail::Arg),
        "",
    );
    flag(
        "date-d",
        &|line| calls_with_flag(line, "date", 'd', Tail::Arg),
        "",
    );
    flag(
        "sed-i",
        &|line| calls_with_flag(line, "sed", 'i', Tail::WordFinal),
        "",
    );
    flag(
        "grep-oP",
        &|line| calls_with_flag(line, "grep", 'P', Tail::NoArg),
        "",
    );
    flag("tac", &|line| word_call(line, "tac"), "command -v tac");
    flag(
        "md5sum",
        &|line| word_call(line, "md5sum"),
        "command -v md5sum",
    );
    // `/proc/sys/kernel/random/uuid` is the one allowed read:
    // existence-guarded, with a uuidgen fallback.
    flag(
        "proc",
        &|line| line.contains("/proc/"),
        "/proc/sys/kernel/random/uuid",
    );
    flag(
        "readlink-f",
        &|line| line.contains("readlink -f"),
        "realpath",
    );
    flag(
        "find-printf",
        &|line| {
            line.find("find")
                .is_some_and(|at| line[at..].contains(" -printf"))
        },
        "",
    );
    // GNU-only sed BRE alternation, and the GNU-only regex/replacement escapes.
    flag("sed-BRE-alternation", &|line| line.contains(r"\(^\|"), "");
    flag(
        "sed-GNU-escape",
        &|line| {
            line.find("sed ").is_some_and(|at| {
                let rest = &line[at..];
                let head = rest.split('|').next().unwrap_or(rest);
                ['s', 'n', 'b', 'w', '+', '?']
                    .iter()
                    .any(|ch| head.contains(&format!("\\{ch}")))
            })
        },
        "",
    );
    bad
}

/// Whether `line` calls the bare command `word`.
fn word_call(line: &str, word: &str) -> bool {
    let mut from = 0;
    while let Some(at) = line[from..].find(word).map(|index| index + from) {
        from = at + word.len();
        let before = line[..at].chars().next_back();
        let after = line[at + word.len()..].chars().next();
        let opens = matches!(before, None | Some(' ' | '(' | '|'));
        let closes = matches!(after, None | Some(' ' | '"' | '|'));
        if opens && closes {
            return true;
        }
    }
    false
}

#[test]
fn the_installer_carries_no_gnu_only_coreutils_and_no_unreviewed_exemption() {
    let source = read(&root().join("install"));
    assert_eq!(
        portability_flags(&source),
        Vec::<&str>::new(),
        "install must run on BSD userland as written"
    );
    // Marker budget.
    assert_eq!(
        source.matches("port-ok:").count(),
        0,
        "a new inline exemption needs review, not a passing suite"
    );

    // The guard must FIRE, not merely pass.
    for (line, label) in [
        (r#"ts="$(date -d "$x" +%s)""#, "date-d"),
        (r#"ts="$(date -u -d "2 hours ago" +%FT%TZ)""#, "date-d"),
        (r#"ts="$(date -dyesterday)""#, "date-d"),
        (r#"sed -i "s/a/b/" "$f""#, "sed-i"),
        (r#"sed -E -i "s/(a)/b/" "$f""#, "sed-i"),
        (r#"sed -i'' "s/a/b/" "$f""#, "sed-i"),
        (r#"m="$(stat -c %Y "$f")""#, "stat-c"),
        (r#"m="$(stat -Lc%Y "$f")""#, "stat-c"),
        (r#"v="$(grep -oP '"k":\s*"\K[^"]+' "$f")""#, "grep-oP"),
        (r#"v="$(grep -Po '\d+' "$f")""#, "grep-oP"),
        (r#"newest="$(tac "$f" | head -1)""#, "tac"),
        (r#"sum="$(md5sum "$f")""#, "md5sum"),
        (r#"ppid="$(cut -d' ' -f4 /proc/$pid/stat)""#, "proc"),
        (r#"real="$(readlink -f "$f")""#, "readlink-f"),
        (r"find . -name '*.x' -printf '%p\n'", "find-printf"),
        (r#"sed -E 's/\(^\|,\)//' "$f""#, "sed-BRE-alternation"),
        (r#"sed -E 's/\s+//' "$f""#, "sed-GNU-escape"),
        (r#"/usr/bin/date -d "$x" +%s"#, "date-d"),
    ] {
        assert!(
            portability_flags(line).contains(&label),
            "the rule set must flag {label} in: {line}"
        );
    }
    // And it must not fire on the portable spellings, or on a command whose
    // NAME merely ends in one it knows.
    for line in [
        r#"m="$(stat -f %m "$f")""#,
        r#"ts="$(date -u -j -f "%FT%TZ" "$x" +%s)""#,
        r#"sed -E 's/(a|b)/c/' "$f" > "$t" && mv "$t" "$f""#,
        r#"newest="$(tail -r "$f" | head -1)""#,
        r#"iso-date -d "$x""#,
        r#"unused-sed -i "s/a/b/""#,
        r#"if command -v tac >/dev/null; then tac "$f"; fi"#,
        r#"uuid="$(cat /proc/sys/kernel/random/uuid 2>/dev/null || uuidgen)""#,
        r#"real="$(readlink -f "$f" 2>/dev/null || realpath "$f")""#,
    ] {
        assert_eq!(
            portability_flags(line),
            Vec::<&str>::new(),
            "the rule set must stay quiet on: {line}"
        );
    }
}

#[test]
fn the_installer_uses_canonical_latest_and_tag_download_routes() {
    let source = read(&root().join("install"));
    for route in [
        r#"url="$REPOSITORY/releases/latest/download""#,
        r#"url="$REPOSITORY/releases/download/$release""#,
    ] {
        assert!(source.contains(route), "install must carry `{route}`");
    }
    assert!(
        !source.contains(r#"url="$REPOSITORY/releases/$release/download""#),
        "install must not use the invalid tag route"
    );
}

#[test]
fn the_bundle_recipe_is_the_one_definition_of_a_bundle_and_both_release_legs_call_it() {
    let justfile = read(&root().join("justfile"));
    let recipe = recipe_text(&justfile, "bundle version platform binary:").join("\n");
    // The three members and their published modes are DEFINED here, so a second
    // open-coded tar elsewhere could drift silently.
    for pin in [
        r#"cp "$binary" "$root/ae-core""#,
        r#"cp install "$root/install""#,
        "sums ae-core install > SHA256SUMS",
        r#"chmod 0555 "$root/ae-core" "$root/install""#,
        r#"chmod 0444 "$root/SHA256SUMS""#,
        r#"chmod 0555 "$root""#,
        // -F IS LOAD-BEARING on the foreign-member proof: without it the dots
        // in a CalVer version are BRE wildcards, so `2026.9.1` matches the
        // bytes `2026x9y1` and a wrong core passes the only check this host
        // can make of a binary it cannot run.
        r#"LC_ALL=C grep -Fqa -- "$version" "$binary""#,
    ] {
        assert_eq!(
            recipe.matches(pin).count(),
            1,
            "the bundle recipe must carry exactly one `{pin}`"
        );
    }

    let release = read(&root().join(".github/workflows/release.yml"));
    assert_eq!(
        release.matches(r#"just bundle "$version""#).count(),
        2,
        "both release legs bundle through the one recipe"
    );
    assert_eq!(
        release.matches(r#"got="$($bin --version)""#).count(),
        2,
        "both legs check ae-core's version against the tag"
    );
    // No second binary has a version word or is copied into a bundle.
    for retired in ["_AE_ENTRY_VERSION", "AE_VERSION=", r#"$root/ae""#] {
        assert_eq!(
            release.matches(retired).count(),
            0,
            "the release workflow must not name the retired `{retired}`"
        );
    }
}

/// The musl compiler/linker name the justfile pins, from `RUST_MUSL_CC := "…"`.
fn justfile_musl_cc(justfile: &str) -> String {
    justfile
        .lines()
        .find_map(|line| line.trim_end().strip_prefix("RUST_MUSL_CC := "))
        .map(|value| value.trim().trim_matches('"').to_owned())
        .unwrap_or_default()
}

/// The linker `.cargo/config.toml` pins for the musl target: the first
/// `linker = "…"` under `[target.x86_64-unknown-linux-musl]` and no other.
fn cargo_musl_linker(config: &str) -> String {
    let mut inside = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[target.x86_64-unknown-linux-musl]";
            continue;
        }
        if inside && let Some(value) = line.strip_prefix("linker = ") {
            return value.trim().trim_matches('"').to_owned();
        }
    }
    String::new()
}

/// ONE NAME FOR THE CROSS COMPILER, in the two files that have to agree.
#[test]
fn the_musl_cross_compiler_has_one_spelling_in_the_justfile_and_the_cargo_config() {
    let justfile = read(&root().join("justfile"));
    let config = read(&root().join(".cargo/config.toml"));

    let pinned = justfile_musl_cc(&justfile);
    assert!(
        !pinned.is_empty(),
        "the justfile must pin RUST_MUSL_CC — it is the one name three readers share"
    );
    assert_eq!(
        cargo_musl_linker(&config),
        pinned,
        "`.cargo/config.toml`'s musl linker and the justfile's RUST_MUSL_CC must name one compiler"
    );

    // RED — each parser must read its OWN section, not any line that looks like
    // one.
    assert_eq!(
        cargo_musl_linker("[target.aarch64-apple-darwin]\nlinker = \"wrong\"\n"),
        "",
        "a linker pinned for another target is not the musl one"
    );
    assert_eq!(
        cargo_musl_linker("[target.x86_64-unknown-linux-musl]\nlinker = \"cc\"\n"),
        "cc"
    );
    assert_eq!(
        justfile_musl_cc("RUST_CROSS_TARGET := \"x86_64-unknown-linux-musl\"\n"),
        "",
        "the target triple is not the compiler name"
    );
    assert_eq!(
        justfile_musl_cc("RUST_MUSL_CC := \"probe-gcc\"\n"),
        "probe-gcc"
    );
}

/// A RELEASE IS BUILT AND PUBLISHED HERE, and it refuses before it can half
/// finish (human ruling, 2026-09-04: agents release locally, Actions optional).
#[test]
fn a_release_builds_both_bundles_locally_and_proves_its_rights_before_the_bump() {
    let justfile = read(&root().join("justfile"));

    // `just bundle` stays the ONE definition of a bundle: `bundles` calls it
    // once per platform, exactly as the two workflow legs do.
    let bundles = recipe_text(&justfile, "bundles:").join("\n");
    assert!(
        !bundles.is_empty(),
        "the justfile must carry a `bundles` recipe"
    );
    for pin in [
        r#"just bundle "$version" darwin-arm64"#,
        r#"just bundle "$version" linux-x86_64-musl"#,
        "sums -- ae-*.tar.gz > SHA256SUMS",
    ] {
        assert_eq!(
            bundles.matches(pin).count(),
            1,
            "the bundles recipe must carry exactly one `{pin}`"
        );
    }
    // The static proof is LOCAL and it is the pinned toolchain's, not the
    // machine's: macOS ships no readelf, and llvm-readobj arrives with the
    // llvm-tools component rust-toolchain.toml already pins.
    for pin in [
        "rustc --print sysroot",
        "llvm-readobj",
        "--program-headers",
        "PT_INTERP",
    ] {
        assert!(
            bundles.contains(pin),
            "the bundles recipe must prove the musl half static via `{pin}`"
        );
    }

    // THE ORDER OF THE RELEASE, read off the recipe itself.
    let release = recipe_text(&justfile, "release:");
    let step = |needle: &str| {
        release
            .iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("the release recipe must run `{needle}`"))
    };
    let rights = step(".permissions.push");
    let branch = step("releases must be from");
    let bump = step("just bump");
    let build = step("just bundles");
    let assets = step("just bundles did not produce it");
    let push = step("git push");
    let publish = step("gh release create");
    assert!(
        rights < bump && branch < bump,
        "push rights and the branch are proved before the bump writes a version file"
    );
    assert!(
        bump < build,
        "the bundles are built from the version the bump just wrote"
    );
    assert!(
        build < assets && assets < push,
        "the notes and every asset exist before anything is pushed"
    );
    assert!(
        push < publish,
        "the branch is pushed before the release names one of its commits"
    );
    assert!(
        release.iter().any(|line| line.contains("--notes-file")),
        "the release body reaches gh as a file, not as an argv-sized string"
    );

    // THE REMOTE TAG IS THE RELEASE'S, NOT A PUSH'S.
    assert!(
        !release
            .iter()
            .any(|line| line.contains("git push") && line.contains("\"$TAG\"")),
        "the tag must never be pushed ahead of the release that carries its assets"
    );
    assert!(
        release
            .iter()
            .any(|line| line.contains("gh release create") && line.contains("--target")),
        "`gh release create --target` is what creates the remote tag"
    );

    // The dispatch-only workflow is retained as a MANUAL Linux run-proof lane.
    let workflow = read(&root().join(".github/workflows/release.yml"));
    assert!(
        workflow.contains("on:\n  workflow_dispatch:"),
        "the release workflow is dispatch-only"
    );
    assert!(
        !workflow.contains("  push:\n    tags:"),
        "the release workflow must not be tag-triggered — `just release` publishes"
    );

    // Both workflow legs that LINK musl name their own linker, because the
    // pin in `.cargo/config.toml` is the macOS cross toolchain's name and
    // Ubuntu's musl-tools ships no triple-prefixed alias.
    for file in [
        ".github/workflows/release.yml",
        ".github/workflows/rust.yml",
    ] {
        assert_eq!(
            read(&root().join(file))
                .matches("CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER: musl-gcc")
                .count(),
            1,
            "{file} must name the musl linker its runner actually has"
        );
    }
}

#[test]
fn the_release_workflow_is_a_manual_proof_lane_with_artifacts_only() {
    let workflow = read(&root().join(".github/workflows/release.yml"));
    let header = workflow
        .split_once("on:\n")
        .map_or(workflow.as_str(), |(header, _)| header);
    assert!(
        !["gh release upload", "gh release create", "contents: write"]
            .iter()
            .any(|banned| header.contains(banned)),
        "the workflow header must not mention release mutation or write permission"
    );
    assert!(
        workflow.contains("on:\n  workflow_dispatch:"),
        "the release workflow is dispatch-only"
    );
    assert!(
        !workflow.contains("  push:\n    tags:"),
        "the release workflow must not be tag-triggered — `just release` publishes"
    );
    assert_eq!(
        workflow.matches("uses: actions/upload-artifact@").count(),
        2,
        "both proof legs must upload their bundles as workflow artifacts"
    );
    assert!(
        !workflow.contains("gh release upload"),
        "the proof-only workflow must not upload or overwrite release assets"
    );
    assert!(
        !workflow.contains("contents: write"),
        "the proof-only workflow must not request release write permission"
    );
    assert!(
        !workflow.contains("gh release create"),
        "the proof-only workflow must not create a GitHub release"
    );
}

/// One command in `dir`, with `PATH` led by that fixture's own `bin`.
fn in_fixture(dir: &Path, program: &str, args: &[&str], env: &[(&str, &str)]) -> (i32, String) {
    let mut invocation = super::parity::Invocation::new(program)
        .env("PATH", {
            let real = std::env::var("PATH").unwrap_or_default();
            format!("{}:{real}", dir.join("bin").display())
        })
        .env("HOME", dir)
        .env("GIT_CONFIG_GLOBAL", dir.join("gitconfig"));
    for arg in args {
        invocation = invocation.arg(arg);
    }
    for (key, value) in env {
        invocation = invocation.env(key, value);
    }
    let out = dir.join("out");
    let err = dir.join("err");
    let status = super::parity::capture::raw::run(&invocation, dir, &out, &err)
        .unwrap_or_else(|why| panic!("{program} must be runnable: {why}"));
    let code = match status.outcome() {
        super::parity::capture::ExitOutcome::Code(code) => code,
        super::parity::capture::ExitOutcome::Signalled => -1,
    };
    (code, std::fs::read_to_string(&out).unwrap_or_default())
}

/// `just bump` derives the next calendar-version sequence from the repository's OWN tags,
/// and moves both version-bearing files or neither.
#[test]
fn just_bump_derives_the_next_sequence_from_the_tags_and_refuses_a_stale_recovery() {
    let root = root();
    let dir = super::cli::OwnedScratch::root("gate", "bump").keep();
    assert!(
        std::fs::create_dir_all(dir.join("bin")).is_ok(),
        "a fixture"
    );
    for name in ["Cargo.toml", "Cargo.lock", "justfile"] {
        assert!(
            std::fs::copy(root.join(name), dir.join(name)).is_ok(),
            "the fixture needs {name}"
        );
    }
    let shim = dir.join("bin").join("date");
    assert!(
        std::fs::write(
            &shim,
            "#!/bin/sh\ncase \"$2\" in\n+%Y) printf '%s\\n' \"$AE_FIXTURE_YEAR\" ;;\n\
             +%m) printf '%s\\n' \"$AE_FIXTURE_MONTH\" ;;\n*) exit 1 ;;\nesac\n",
        )
        .is_ok(),
        "the date shim"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert!(
            std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).is_ok(),
            "an executable date shim"
        );
    }
    for words in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "calver@example.invalid"],
        vec!["config", "user.name", "CalVer"],
        vec!["add", "Cargo.toml", "Cargo.lock", "justfile"],
        vec!["commit", "-qm", "fixture"],
    ] {
        let (code, _) = in_fixture(&dir, "git", &words, &[]);
        assert_eq!(code, 0, "git {words:?} must succeed");
    }
    let bump = |year: &str, month: &str| {
        in_fixture(
            &dir,
            "just",
            &["bump"],
            &[("AE_FIXTURE_YEAR", year), ("AE_FIXTURE_MONTH", month)],
        )
    };
    let crate_version = || {
        std::fs::read_to_string(dir.join("Cargo.toml"))
            .unwrap_or_default()
            .lines()
            .find_map(|line| line.strip_prefix("version = \""))
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or_default()
            .to_owned()
    };
    let tag = |name: &str| {
        let (code, _) = in_fixture(&dir, "git", &["tag", name], &[]);
        assert_eq!(code, 0, "the fixture takes tag {name}");
    };

    // No matching tag: the month's first release is sequence 1, and the month
    // is written UNPADDED even though `date +%m` answers `09`.
    let (code, stdout) = bump("2026", "09");
    assert_eq!((code, stdout.trim()), (0, "2026.9.1"));
    assert_eq!(crate_version(), "2026.9.1", "Cargo.toml moved");
    assert!(
        std::fs::read_to_string(dir.join("Cargo.lock"))
            .unwrap_or_default()
            .contains("version = \"2026.9.1\""),
        "and Cargo.lock moved with it — both files or neither"
    );
    assert!(
        !dir.join(".ae-bump-recovery").exists(),
        "a completed bump leaves no recovery marker"
    );

    // Two tags of the same month: one past the HIGHEST, not one past the count.
    tag("v2026.9.1");
    tag("v2026.9.2");
    let (code, stdout) = bump("2026", "09");
    assert_eq!((code, stdout.trim()), (0, "2026.9.3"));

    // A STALE recovery marker refuses before any edit.
    tag("v2026.9.3");
    let before = crate_version();
    assert!(
        std::fs::create_dir(dir.join(".ae-bump-recovery")).is_ok(),
        "a stale marker"
    );
    let (code, stdout) = bump("2026", "09");
    assert_ne!(code, 0, "a stale recovery marker must fail the bump");
    assert_eq!(stdout, "", "and emit no version anyone could act on");
    assert_eq!(crate_version(), before, "and leave the live files alone");
    assert!(
        dir.join(".ae-bump-recovery").exists(),
        "and preserve the marker for the recovery it names"
    );
    assert!(
        std::fs::remove_dir_all(dir.join(".ae-bump-recovery")).is_ok(),
        "the marker clears"
    );

    // The month rolls over and the sequence RESETS, tags of the old month
    // notwithstanding.
    let (code, stdout) = bump("2026", "10");
    assert_eq!((code, stdout.trim()), (0, "2026.10.1"));

    let _ = std::fs::remove_dir_all(&dir);
}

/// The sha argv values a recorded git-cliff invocation carries after `--skip-commit`, or
/// the reason the crossing is malformed. A joined value (one argument holding several
/// shas) or a non-sha is exactly what the boundary must never hand over.
fn skip_values(record: &[String]) -> Result<Vec<String>, String> {
    let mut values = Vec::new();
    let mut collecting = false;
    let mut flagged = false;
    for arg in record {
        if arg == "--skip-commit" {
            collecting = true;
            flagged = true;
            continue;
        }
        if !collecting {
            continue;
        }
        if arg.starts_with('-') {
            collecting = false;
            continue;
        }
        if arg.contains(' ') {
            return Err(format!("joined skip value: {arg}"));
        }
        let hex = arg
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if arg.len() != 40 || !hex {
            return Err(format!("malformed skip value: {arg}"));
        }
        values.push(arg.clone());
    }
    if flagged && values.is_empty() {
        return Err("--skip-commit with no value".to_owned());
    }
    Ok(values)
}

/// The argv a stub recorded, one entry per line.
fn recorded_argv(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// A fixture repository holding the REAL justfile (or a deliberate mutation) and a
/// `git-cliff` stub that records its own argv, then refuses a `--skip-commit` value that
/// is not exactly one 40-character lowercase hex sha — the receipt check for the
/// boundary. `absorbed` commits are merged in from a side branch.
fn cliff_fixture(dir: &Path, name: &str, absorbed: usize, justfile: &str) -> PathBuf {
    let repo = dir.join(name);
    assert!(
        std::fs::create_dir_all(repo.join("bin")).is_ok(),
        "a fixture repo"
    );
    assert!(
        std::fs::write(repo.join("justfile"), justfile).is_ok(),
        "the fixture justfile"
    );
    let shim = repo.join("bin").join("git-cliff");
    assert!(
        std::fs::write(
            &shim,
            "#!/bin/sh\nrec=${AE_CLIFF_ARGV:?}\n: > \"$rec\"\n\
             for a in \"$@\"; do printf '%s\\n' \"$a\" >> \"$rec\"; done\n\
             prev=\"\"\nfor a in \"$@\"; do\n\
             if [ \"$prev\" = \"--skip-commit\" ]; then\n\
             case \"$a\" in *[!0-9a-f]*) echo \"stub: non-hex or joined skip value\" >&2; exit 7 ;; esac\n\
             [ \"${#a}\" -eq 40 ] || { echo \"stub: short skip value\" >&2; exit 7; }\n\
             fi\nprev=\"$a\"\ndone\nexit 0\n",
        )
        .is_ok(),
        "the git-cliff stub"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert!(
            std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).is_ok(),
            "an executable stub"
        );
    }
    let mut words = vec![
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "cliff@example.invalid"],
        vec!["config", "user.name", "Cliff"],
        vec!["commit", "-q", "--allow-empty", "-m", "Root"],
    ];
    if absorbed > 0 {
        words.extend([
            vec!["checkout", "-q", "-b", "slice"],
            vec!["commit", "-q", "--allow-empty", "-m", "Absorbed one"],
            vec!["commit", "-q", "--allow-empty", "-m", "Absorbed two"],
            vec!["checkout", "-q", "main"],
            vec![
                "merge",
                "-q",
                "--no-ff",
                "slice",
                "-m",
                "Merge slice: shipped change",
            ],
        ]);
    }
    for words in words {
        let (code, _) = in_fixture(&repo, "git", &words, &[]);
        assert_eq!(code, 0, "git {words:?} must succeed");
    }
    repo
}

/// `just <recipe> [args]` in a fixture; returns the exit code and the stub's argv record.
fn cliff_run(repo: &Path, recipe: &str, args: &[&str]) -> (i32, Vec<String>) {
    let argv_file = repo.join("argv");
    let mut full = vec![recipe];
    full.extend_from_slice(args);
    let (code, _) = in_fixture(
        repo,
        "just",
        &full,
        &[("AE_CLIFF_ARGV", argv_file.to_str().unwrap_or_default())],
    );
    (code, recorded_argv(&argv_file))
}

/// The shas a fixture's side branch absorbed, resolved from git and sorted.
fn absorbed_shas(repo: &Path) -> Vec<String> {
    let mut shas = Vec::new();
    for name in ["slice", "slice~1"] {
        let (code, stdout) = in_fixture(repo, "git", &["rev-parse", name], &[]);
        assert_eq!(code, 0, "the fixture resolves {name}");
        shas.push(stdout.trim().to_owned());
    }
    shas.sort();
    shas
}

/// The leading argv entries as string slices, for comparison against expected literals.
fn as_strs(argv: &[String]) -> Vec<&str> {
    argv.iter().map(String::as_str).collect()
}

/// `_cliff-run` owns the ONE place where the skip list stops being text and becomes argv.
/// `_cliff-skip` can only prove its text, so these run the REAL recipes in fixture
/// repositories with a git-cliff stub that records its OWN argv: an empty complement
/// emits no flag, a non-empty one emits one argv entry per sha, and the joined crossing a
/// quoted `$skip` would produce is refused rather than silently accepted.
#[test]
fn the_changelog_skip_list_crosses_to_git_cliff_as_separate_arguments() {
    let dir = super::cli::OwnedScratch::root("gate", "cliff").keep();
    let justfile = read(&root().join("justfile"));

    // An EMPTY complement: the linear fixture emits no flag at all.
    let empty = cliff_fixture(&dir, "empty", 0, &justfile);
    let (code, argv) = cliff_run(&empty, "changelog", &[]);
    assert_eq!(code, 0, "an empty skip list must still generate");
    assert!(
        !argv.iter().any(|arg| arg == "--skip-commit"),
        "an empty complement must emit no --skip-commit: {argv:?}"
    );

    // A NON-EMPTY complement: one argv entry per absorbed sha, at the real recipe.
    let merge = cliff_fixture(&dir, "merge", 2, &justfile);
    let expected = absorbed_shas(&merge);
    let (code, argv) = cliff_run(&merge, "changelog", &[]);
    assert_eq!(code, 0, "the merge fixture must generate");
    let mut got = skip_values(&argv).unwrap_or_else(|why| panic!("the boundary: {why}"));
    got.sort();
    assert_eq!(
        got, expected,
        "every absorbed sha, one argv entry: {argv:?}"
    );
    assert_eq!(
        as_strs(&argv[argv.len() - 2..]),
        vec!["-o", "CHANGELOG.md"],
        "trailing args preserved"
    );

    // The two RELEASE-shaped invocations run through the same owner and keep their own
    // trailing arguments.
    for trailing in [
        vec!["--tag", "v0.0.1", "-o", "CHANGELOG.md"],
        vec!["--tag", "v0.0.1", "--unreleased", "--strip", "header"],
    ] {
        let (code, argv) = cliff_run(&merge, "_cliff-run", &trailing);
        assert_eq!(code, 0, "_cliff-run {trailing:?} must succeed");
        let mut got = skip_values(&argv).unwrap_or_else(|why| panic!("the boundary: {why}"));
        got.sort();
        assert_eq!(
            got, expected,
            "the release shape carries the list: {argv:?}"
        );
        assert_eq!(
            as_strs(&argv[argv.len() - trailing.len()..]),
            trailing,
            "trailing args preserved"
        );
    }

    // A JOINED crossing: the quoted variant of the same owner is refused, not accepted.
    let quoted = justfile.replace(
        "set -- --skip-commit $skip \"$@\"",
        "set -- --skip-commit \"$skip\" \"$@\"",
    );
    assert_ne!(
        quoted, justfile,
        "the mutation must apply to the real owner"
    );
    let joined = cliff_fixture(&dir, "joined", 2, &quoted);
    let (code, argv) = cliff_run(&joined, "changelog", &[]);
    assert_ne!(code, 0, "a joined skip value must fail the run");
    assert!(
        skip_values(&argv).is_err(),
        "the joined value is detected as malformed, not accepted: {argv:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Every changelog invocation takes the skip list from the ONE owner, so the three call
/// sites cannot drift into carrying or dropping it by themselves.
#[test]
fn every_changelog_invocation_takes_the_skip_list_from_the_one_owner() {
    let justfile = read(&root().join("justfile"));
    let changelog = recipe_text(&justfile, "changelog:");
    let release = recipe_text(&justfile, "release:");
    let owner = recipe_text(&justfile, "_cliff-run +args:");

    let calls = |lines: &[String]| {
        lines
            .iter()
            .filter(|line| line.contains("just _cliff-run"))
            .count()
    };
    assert_eq!(
        calls(&changelog),
        1,
        "changelog runs through the owner: {changelog:?}"
    );
    assert_eq!(
        calls(&release),
        2,
        "release's two invocations run through the owner: {release:?}"
    );
    assert!(
        owner
            .iter()
            .any(|line| line.contains("set -- --skip-commit $skip \"$@\"")),
        "the owner passes the list unquoted, one argv per sha: {owner:?}"
    );
    assert!(
        owner.iter().any(|line| line.trim() == "git-cliff \"$@\""),
        "and emits no skip flag when the list is empty: {owner:?}"
    );
}

/// Every non-comment line that spells the `git-cliff` command word, with the recipe it
/// belongs to (or `<top level>`). Comments are skipped, so prose about the tool is not a
/// site; a variable DEFINITION that spells the command is one, because it is a way to
/// reach the tool from elsewhere.
fn git_cliff_sites(justfile: &str) -> Vec<(String, String)> {
    let mut recipe = String::from("<top level>");
    let mut sites = Vec::new();
    for line in justfile.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indented = line.starts_with([' ', '\t']);
        if !indented && trimmed.ends_with(':') {
            recipe.clear();
            recipe.push_str(trimmed);
            continue;
        }
        if !indented {
            // An unindented line that is not a recipe header is top level.
            recipe.clear();
            recipe.push_str("<top level>");
        }
        let is_site = if indented {
            // Command position only: the command word itself, or inside a command
            // substitution. The rust-setup probe (`ensure git-cliff … 'git-cliff
            // --version'`) installs and asks a version; it runs neither generator nor
            // changelog, so it is not an invocation.
            trimmed.starts_with("git-cliff")
                || trimmed.contains("$(git-cliff")
                || trimmed.contains("| git-cliff")
        } else {
            // A top-level definition that spells the command is an indirection.
            bare_word_count(line, "git-cliff") > 0
        };
        if is_site {
            sites.push((recipe.clone(), trimmed.to_owned()));
        }
    }
    sites
}

/// `git-cliff` is installed and probed by the justfile, but INVOKED in exactly one place:
/// the boundary owner. A future direct call in any other recipe — or a top-level alias
/// that spells the command — fails here by construction, with no list of call sites to
/// maintain. `_cliff-skip` and `_cliff-run` keep the single invocation on the measured
/// path; this keeps it the only path.
///
/// WHAT THIS SCAN DOES NOT COVER, so a silent gap is named rather than assumed: a command
/// reached under a prefix this scan does not read (`command git-cliff`, `env git-cliff`,
/// `sudo git-cliff`), a backtick substitution, an indirection that never spells the literal
/// word (a runtime-assembled shell variable `c${X}-cliff`, a renamed binary behind
/// `command -v`, a wrapper crate or binary), or a second justfile reached with `just -f`.
/// The justfile is the artifact this gate owns (see the module doc); text it cannot see is
/// text it cannot guard.
#[test]
fn git_cliff_is_invoked_only_from_the_boundary_owner() {
    let justfile = read(&root().join("justfile"));
    let sites = git_cliff_sites(&justfile);
    assert_eq!(
        sites.len(),
        1,
        "git-cliff must be invoked in exactly one place: {sites:?}"
    );
    assert_eq!(
        sites[0].0, "_cliff-run +args:",
        "and that place is the boundary owner: {sites:?}"
    );

    // The red cases: a fourth direct call in another recipe, and a top-level alias.
    let bypass = format!("{justfile}\nbypass:\n    git-cliff -o OTHER.md\n");
    let bypass_sites = git_cliff_sites(&bypass);
    assert_eq!(
        bypass_sites.len(),
        2,
        "a direct call elsewhere is a site: {bypass_sites:?}"
    );
    let alias = format!("{justfile}\ncliff := \"git-cliff\"\n");
    assert!(
        git_cliff_sites(&alias)
            .iter()
            .any(|(where_, _)| where_ == "<top level>"),
        "a top-level alias that spells the command is a site: {alias:?}"
    );
}
