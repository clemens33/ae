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

#[allow(
    clippy::too_many_lines,
    reason = "the frozen #234 acceptance is one long matrix fixture and its bytes stay as the independent author froze them"
)]
mod mutantstime_spec;
#[allow(
    clippy::cast_lossless,
    clippy::cast_possible_wrap,
    clippy::cloned_ref_to_slice_refs,
    clippy::default_trait_access,
    clippy::expect_used,
    clippy::manual_range_contains,
    clippy::map_unwrap_or,
    clippy::nonminimal_bool,
    clippy::too_many_lines,
    clippy::unnecessary_cast,
    reason = "the frozen P2 acceptance is a fixture-heavy independent spec whose bytes stay as its author froze them; a lint finding is not a reason to edit them"
)]
// Its bytes are frozen as the independent author wrote them, so rustfmt leaves the file alone.
#[rustfmt::skip]
mod receipt_spec;
mod macos_subset;
#[allow(
    clippy::expect_used,
    reason = "independent acceptance fixtures must fail with a named precondition"
)]
mod macos_subset_spec;
mod modtree;
mod receipt_window;

/// The repository root — this file's crate manifest directory.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|why| panic!("{} must be readable: {why}", path.display()))
}

/// The text of one string variable (`NAME := '''` ... `'''`) of the justfile, or
/// an empty string where there is none.
fn justfile_variable(justfile: &str, name: &str) -> String {
    let opener = format!("{name} := '''");
    let mut lines = justfile
        .lines()
        .skip_while(|line| !line.starts_with(&opener));
    if lines.next().is_none() {
        return String::new();
    }
    let body: Vec<&str> = lines.take_while(|line| line.trim() != "'''").collect();
    body.join("\n")
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
        "{{ SIZING }}",
        "size_counts",
        "env -u TMUX -u TMUX_PANE tmux -S \"$test_tmux_tmp/tmux-$(id -u)/ae\" kill-server",
        "keep_or_remove \"$test_tmux_tmp\"",
        "trap cleanup EXIT",
        "export TMUX_TMPDIR=\"$test_tmux_tmp\"",
        "unset TMUX TMUX_PANE",
        "tmux -f /dev/null -L ae new-session -d -s foreign-review-sentry -e AE_SESSION=foreign-review-sentry",
        "detached nice -n 10 cargo nextest run --locked --all-features",
        "detached nice -n 10 cargo test --doc --locked --all-features",
        "detached nice -n 10 cargo llvm-cov nextest --locked --all-features",
        "detached nice -n 10 cargo mutants --cargo-arg=--locked --jobs 1 \"$@\"",
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
        // Positional words, never `{{ args }}`: that splices the arguments back into
        // a command line, where a space splits one and a `|` starts a pipeline (#221).
        ("rust-mutants *args:", "just _tmux-isolated mutants \"$@\""),
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
        .position(|line| line == "detached nice -n 10 cargo nextest run --locked --all-features")
    else {
        return false;
    };
    let order = [guard, filtered, note, bare, doctest];
    order.windows(2).all(|pair| pair[0] < pair[1]) && arm[note].contains(">&2")
}

/// Whether no destructive tmux verb in the justfile names its server with a
/// short `-L`: outside a set-and-existing `TMUX_TMPDIR` that name is the live
/// fleet server, so a lane cleanup through it kills developers' sessions.
/// Continuations fold before the scan; full-line comments are not commands.
/// The shell word a justfile token carries: past any assignment or command
/// substitution opener, stripped of surrounding punctuation.
fn word_core(word: &str) -> &str {
    word.rsplit(['=', '$', '(', '`', '"', '\''])
        .next()
        .unwrap_or(word)
        .trim_matches(|c: char| {
            !(c.is_ascii_alphanumeric() || c == '/' || c == '-' || c == '_' || c == '.')
        })
}

fn no_short_l_destructive_tmux(justfile: &str) -> bool {
    const VERBS: [&str; 4] = ["kill-server", "kill-session", "kill-pane", "respawn-pane"];
    justfile.replace("\\\n", " ").lines().all(|line| {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') {
            return true;
        }
        let words: Vec<&str> = line.split_whitespace().map(word_core).collect();
        let Some(start) = words
            .iter()
            .position(|word| *word == "tmux" || word.ends_with("/tmux"))
        else {
            return true;
        };
        let tail = &words[start + 1..];
        let short = tail
            .iter()
            .position(|word| *word == "-L" || word.starts_with("-L") && word.len() > 2);
        let verb = tail.iter().position(|word| VERBS.contains(word));
        !matches!((short, verb), (Some(l), Some(v)) if l < v)
    })
}

#[test]
fn the_justfile_names_no_destructive_tmux_server_with_a_short_l() {
    assert!(
        no_short_l_destructive_tmux(&read(&root().join("justfile"))),
        "a destructive tmux verb must name its server with -S, never a short -L"
    );

    // RED — every destructive verb through a short -L is refused.
    for verb in ["kill-server", "kill-session", "kill-pane", "respawn-pane"] {
        assert!(
            !no_short_l_destructive_tmux(&format!("cleanup:\n    tmux -L ae {verb}\n")),
            "{verb} through -L must be refused"
        );
    }
    // RED — the lane-cleanup shape: env and flags before tmux change nothing.
    assert!(!no_short_l_destructive_tmux(
        "cleanup:\n    TMUX_TMPDIR=\"$t\" env -u TMUX -u TMUX_PANE tmux -L ae kill-server >/dev/null 2>&1 || true\n"
    ));
    // RED — a folded spelling is still one command, and a command
    // substitution does not hide the verb either.
    assert!(!no_short_l_destructive_tmux(
        "cleanup:\n    tmux -L ae \\\n        kill-server\n"
    ));
    assert!(!no_short_l_destructive_tmux(
        "cleanup:\n    error=\"$(tmux -L ae kill-server 2>&1)\"\n"
    ));
    // GREEN — the explicit socket is the fix; the sentry's new-session is
    // not destructive; a comment is not a command and a file test is not
    // a flag.
    assert!(no_short_l_destructive_tmux(
        "cleanup:\n    tmux -S \"$t/tmux-$(id -u)/ae\" kill-server\n"
    ));
    assert!(no_short_l_destructive_tmux(
        "lane:\n    tmux -f /dev/null -L ae new-session -d -s foreign-review-sentry\n"
    ));
    assert!(no_short_l_destructive_tmux(
        "lane:\n    # tmux -L ae kill-server\n    [[ -d \"$d\" && ! -L \"$d\" ]] || continue\n"
    ));
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

    // RED — the public mutants recipe splicing its arguments back into a command
    // line (#221): a filter with a space or a `|` no longer reaches cargo-mutants
    // as one word. The real justfile with only that line put back.
    assert!(!rust_test_tmux_isolation_ok(
        &read(&root().join("justfile")).replace(
            "just _tmux-isolated mutants \"$@\"",
            "just _tmux-isolated mutants {{ args }}"
        )
    ));

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
            .env("HOME", &scratch_root)
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

/// The lines of one TOML table: after its exact `[header]` line, up to the next
/// table header, each trimmed.
fn toml_table<'a>(text: &'a str, header: &str) -> Vec<&'a str> {
    text.lines()
        .map(str::trim)
        .skip_while(|line| *line != header)
        .skip(1)
        .take_while(|line| !line.starts_with('['))
        .collect()
}

/// The number a `<key><N>` line of a table carries (the key spelled with its ` = `).
fn table_number(lines: &[&str], key: &str) -> Option<u32> {
    lines
        .iter()
        .find_map(|line| line.strip_prefix(key)?.trim().parse().ok())
}

/// The seconds one test lives under a table's
/// `slow-timeout = { period = "<N>s", terminate-after = <M>, grace-period = "<G>s" }`
/// before nextest KILLS it: N x M plus G. nextest sends SIGTERM at N x M and
/// SIGKILL only a grace period later, 10 s when none is named (nexte.st/docs/
/// features/slow-tests). `None` for no termination (a period alone only WARNS) or
/// a time not spelled in whole seconds.
fn kill_after_secs(lines: &[&str]) -> Option<u32> {
    let line = lines
        .iter()
        .find_map(|line| line.strip_prefix("slow-timeout = "))?;
    let seconds = |key: &str| {
        line.split_once(key)?
            .1
            .split_once("s\"")?
            .0
            .parse::<u32>()
            .ok()
    };
    let count = line.split_once("terminate-after = ")?.1;
    let count: String = count.chars().take_while(char::is_ascii_digit).collect();
    let grace = if line.contains("grace-period = ") {
        seconds("grace-period = \"")?
    } else {
        10
    };
    seconds(" period = \"")?
        .checked_mul(count.parse().ok()?)?
        .checked_add(grace)
}

/// Whether both lanes are bounded in CONFIG (#159): a bare nextest run (no lane,
/// no override) at most four tests at once, the isolated lane's own sizing
/// reaching its lane ceiling only through `NEXTEST_TEST_THREADS`; every mutation run (the `mutants` profile `.cargo/mutants.toml`
/// selects) at most two, each hard-killed within a minute. The thread count is
/// never a `--test-threads` flag in `mutants.toml`, because a flag beats
/// `NEXTEST_TEST_THREADS` and the operator could not tighten a run.
fn thread_caps_ok(nextest: &str, mutants: &str) -> bool {
    let default = toml_table(nextest, "[profile.default]");
    let bounded = toml_table(nextest, "[profile.mutants]");
    let config: Vec<&str> = mutants
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .collect();
    table_number(&default, "test-threads = ").is_some_and(|threads| (1..=4).contains(&threads))
        && table_number(&bounded, "test-threads = ")
            .is_some_and(|threads| (1..=2).contains(&threads))
        && kill_after_secs(&bounded).is_some_and(|secs| (1..=60).contains(&secs))
        && config.iter().any(|line| {
            line.starts_with("additional_cargo_test_args = [")
                && line.contains("\"--profile\", \"mutants\"")
        })
        && !config.iter().any(|line| line.contains("test-threads"))
}

#[test]
fn the_lanes_cap_their_test_threads_in_config() {
    assert!(thread_caps_ok(
        &read(&root().join(".config/nextest.toml")),
        &read(&root().join(".cargo/mutants.toml"))
    ));
    let nextest = |default: &str, bounded: &str| {
        format!("[profile.default]\n{default}\n\n[profile.mutants]\n{bounded}\n")
    };
    let default = "test-threads = 4";
    let bounded = "test-threads = 2\nslow-timeout = { period = \"30s\", terminate-after = 2, grace-period = \"0s\" }";
    let mutants = "additional_cargo_test_args = [\"--profile\", \"mutants\"]\n";
    assert!(thread_caps_ok(&nextest(default, bounded), mutants));
    // RED — nextest's own default is every core; so is an explicit wide one.
    assert!(!thread_caps_ok(&nextest("", bounded), mutants));
    assert!(!thread_caps_ok(
        &nextest("test-threads = 18", bounded),
        mutants
    ));
    // RED — the old bare-run default of eight: a run outside the lane would
    // start twice the small fixed count on a machine other sessions share.
    assert!(!thread_caps_ok(
        &nextest("test-threads = 8", bounded),
        mutants
    ));
    // RED — a mutation run with no bound of its own: no profile at all, none
    // selected, or a profile that sizes itself wider than two.
    assert!(!thread_caps_ok(
        "[profile.default]\ntest-threads = 4\n",
        mutants
    ));
    assert!(!thread_caps_ok(
        &nextest(default, bounded),
        "timeout_multiplier = 5.0\n"
    ));
    let wide = bounded.replace("= 2\n", "= 4\n");
    assert!(!thread_caps_ok(&nextest(default, &wide), mutants));
    assert!(!thread_caps_ok(
        &nextest(default, bounded.lines().nth(1).unwrap_or_default()),
        mutants
    ));
    // RED — a thread FLAG in mutants.toml beats NEXTEST_TEST_THREADS, so the
    // operator could never tighten a run, even with the profile selected.
    assert!(!thread_caps_ok(
        &nextest(default, bounded),
        "additional_cargo_test_args = [\"--test-threads=4\"]\n"
    ));
    assert!(!thread_caps_ok(
        &nextest(default, bounded),
        "additional_cargo_test_args = [\"--profile\", \"mutants\", \"--test-threads=1\"]\n"
    ));
    // RED — no hard kill under mutants: a warning-only period, a SIGTERM with no
    // grace-period (SIGKILL comes 10 s late), a stray test outliving a minute,
    // times not in whole seconds, no timeout at all.
    for slow in [
        "slow-timeout = { period = \"30s\" }",
        "slow-timeout = { period = \"30s\", terminate-after = 2 }",
        "slow-timeout = { period = \"30s\", terminate-after = 2, grace-period = \"5s\" }",
        "slow-timeout = { period = \"30s\", terminate-after = 2, grace-period = \"1m\" }",
        "slow-timeout = { period = \"30s\", terminate-after = 5, grace-period = \"0s\" }",
        "slow-timeout = { period = \"2m\", terminate-after = 1, grace-period = \"0s\" }",
        "slow-timeout = \"30s\"",
        "",
    ] {
        let bounded = format!("test-threads = 2\n{slow}");
        assert!(
            !thread_caps_ok(&nextest(default, &bounded), mutants),
            "{slow}"
        );
    }
}

// ---------------------------------------------------------------------------
// #234 — the finite `mutants` timeout exceptions, and the guard over the
// EFFECTIVE overrides. `thread_caps_ok` above reads only the two base tables,
// so a longer kill in an override table would pass it; `mutants_timeout_policy`
// reads every table the effective `mutants` run can reach and refuses what it
// cannot attribute. It is NOT a TOML parser: it accepts the committed file's own
// line shapes and fails closed on everything else.
// ---------------------------------------------------------------------------

/// The only three tests that may outlive the 60 s `mutants` default, each under
/// one named, finite exception.
const MUTANTS_APPROVED: [&str; 3] = [
    "autotrust_spec::measured_yes_and_no_use_only_the_arrow_then_enter_once_across_watchdog_restart",
    "reseat_headroom_spec::a_readable_unknown_frame_holds_headroom_instead_of_stopping_the_tool",
    "reseat_headroom_spec::busy_and_draft_holds_do_not_spend_attempts_and_move_after_idle",
];

/// The most an approved test may live: period x terminate-after plus grace.
const MUTANTS_EXCEPTION_CAP_SECS: u64 = 180;

/// The most any other test may live under the `mutants` profile.
const MUTANTS_BASE_CAP_SECS: u64 = 60;

/// How long the compiled-name listing may run before it is abandoned.
const INVENTORY_BOUND: std::time::Duration = std::time::Duration::from_secs(30);

const HEADER_DEFAULT: &str = "[profile.default]";
const HEADER_DEFAULT_OVERRIDE: &str = "[[profile.default.overrides]]";
const HEADER_LINUX: &str = "[profile.linux]";
const HEADER_LINUX_OVERRIDE: &str = "[[profile.linux.overrides]]";
const HEADER_MUTANTS: &str = "[profile.mutants]";
const HEADER_MUTANTS_OVERRIDE: &str = "[[profile.mutants.overrides]]";

/// The only table headers whose bytes the walker attributes.
const CANONICAL_HEADERS: [&str; 6] = [
    HEADER_DEFAULT,
    HEADER_DEFAULT_OVERRIDE,
    HEADER_LINUX,
    HEADER_LINUX_OVERRIDE,
    HEADER_MUTANTS,
    HEADER_MUTANTS_OVERRIDE,
];

/// One `slow-timeout = { ... }` value, in whole seconds.
#[derive(Clone, Copy, PartialEq, Eq)]
struct SlowTimeout {
    period: u64,
    terminate_after: u64,
    grace: Option<u64>,
}

impl SlowTimeout {
    /// Seconds before nextest kills the test: SIGTERM at period x terminate-after,
    /// SIGKILL a grace later (10 s when none is named). `None` on overflow.
    fn kill_secs(self) -> Option<u64> {
        self.period
            .checked_mul(self.terminate_after)?
            .checked_add(self.grace.unwrap_or(10))
    }
}

/// A timeout the committed config carries outside the `mutants` profile. The
/// `mutants` run inherits the default profile's overrides, so each one is part
/// of its effective bound and may not change or vanish unnoticed.
struct Baseline {
    header: &'static str,
    filter: Option<&'static str>,
    timeout: SlowTimeout,
}

const BASELINE: [Baseline; 3] = [
    Baseline {
        header: HEADER_DEFAULT_OVERRIDE,
        filter: Some(
            "test(=compact::a_writerless_fifo_swapped_in_after_lstat_is_refused_without_blocking)",
        ),
        timeout: SlowTimeout {
            period: 5,
            terminate_after: 2,
            grace: None,
        },
    },
    Baseline {
        header: HEADER_LINUX,
        filter: None,
        timeout: SlowTimeout {
            period: 60,
            terminate_after: 5,
            grace: None,
        },
    },
    Baseline {
        header: HEADER_LINUX_OVERRIDE,
        filter: Some("test(/^menu::/)"),
        timeout: SlowTimeout {
            period: 30,
            terminate_after: 2,
            grace: None,
        },
    },
];

/// One table of the nextest config: its header line (empty before the first
/// header) and its `key = value` lines, both sides trimmed.
struct ConfigBlock<'a> {
    header: &'a str,
    keys: Vec<(&'a str, &'a str)>,
}

impl<'a> ConfigBlock<'a> {
    /// Every value this table gives `key`, in file order.
    fn values(&self, key: &str) -> Vec<&'a str> {
        self.keys
            .iter()
            .filter(|(name, _)| *name == key)
            .map(|(_, value)| *value)
            .collect()
    }
}

fn is_canonical_header(text: &str) -> bool {
    CANONICAL_HEADERS.contains(&text)
}

/// A TOML bare key: ASCII letters, digits, `_` and `-`. A quoted, dotted or
/// escaped key (`"slow-timeout"`, `slow-timeout.period`) can name the same
/// setting as a bare one while sharing no text with it, so none is attributed.
fn is_bare_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// Group the config into blocks. A line that is not blank, a comment, a header
/// or `key = value`, a header spelled other than canonically, a key that is not
/// bare, and a value that opens an inline table, an array or a multi-line string
/// (where a second timeout could hide) each fail closed here.
fn mutants_policy_blocks<'a>(
    nextest: &'a str,
    violations: &mut Vec<String>,
) -> Vec<ConfigBlock<'a>> {
    let mut blocks = vec![ConfigBlock {
        header: "",
        keys: Vec::new(),
    }];
    for raw_line in nextest.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            let squeezed: String = line.chars().filter(|ch| !ch.is_whitespace()).collect();
            let unreadable = ["mutants", "overrides", "slow-timeout"]
                .iter()
                .any(|word| line.contains(word))
                || line.contains(['"', '\'', '\\']);
            if !is_canonical_header(line) && (is_canonical_header(&squeezed) || unreadable) {
                violations.push(format!(
                    "unsupported-form: header `{line}` is not spelled exactly as the policy reads it"
                ));
            }
            blocks.push(ConfigBlock {
                header: line,
                keys: Vec::new(),
            });
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            violations.push(format!(
                "unsupported-form: line `{line}` is not a header, a comment or `key = value`"
            ));
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if !is_bare_key(key) {
            violations.push(format!(
                "unsupported-form: key `{key}` is not a plain bare key, so it cannot be attributed"
            ));
        }
        if key != "slow-timeout"
            && ["{", "[", "\"\"\"", "'''"]
                .iter()
                .any(|opener| value.starts_with(opener))
        {
            violations.push(format!(
                "unsupported-form: the value of `{key}` opens an inline table, an array or a multi-line string the policy cannot attribute"
            ));
        }
        if let Some(block) = blocks.last_mut() {
            block.keys.push((key, value));
        }
    }
    blocks
}

/// Digits only: no sign, underscore or unit.
fn plain_number(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// `"<N>s"` in whole seconds.
fn quoted_seconds(text: &str) -> Option<u64> {
    let digits = text.strip_prefix('"')?.strip_suffix("s\"")?;
    plain_number(digits).map(u64::from)
}

/// Fill `slot` once. False when it was already filled or `parsed` is absent.
fn set_once(slot: &mut Option<u64>, parsed: Option<u64>) -> bool {
    if slot.is_some() {
        return false;
    }
    *slot = parsed;
    parsed.is_some()
}

/// The one-line inline table `{ period = "<N>s", terminate-after = <M>, grace-period = "<G>s" }`.
fn parse_slow_timeout(value: &str) -> Result<SlowTimeout, String> {
    let Some(inner) = value
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return Err(format!(
            "unsupported-form: `{value}` is not a one-line inline table"
        ));
    };
    let (mut period, mut terminate_after, mut grace) = (None, None, None);
    for part in inner.split(',') {
        let Some((key, field)) = part.split_once('=') else {
            return Err(format!("unsupported-form: `{part}` is not `key = value`"));
        };
        let (key, field) = (key.trim(), field.trim());
        let stored = match key {
            "period" => set_once(&mut period, quoted_seconds(field)),
            "terminate-after" => set_once(&mut terminate_after, plain_number(field).map(u64::from)),
            "grace-period" => set_once(&mut grace, quoted_seconds(field)),
            _ => false,
        };
        if !stored {
            return Err(format!(
                "unsupported-form: `{part}` in `{value}` is an unknown or repeated key, or not whole seconds or a plain number"
            ));
        }
    }
    let (Some(period), Some(terminate_after)) = (period, terminate_after) else {
        return Err(format!(
            "unbounded: `{value}` names no period and terminate-after, so it only warns"
        ));
    };
    Ok(SlowTimeout {
        period,
        terminate_after,
        grace,
    })
}

/// The text inside the one single-quoted `filter = '...'` of a block.
fn block_filter<'a>(block: &ConfigBlock<'a>) -> Option<&'a str> {
    let filters = block.values("filter");
    let [one] = filters.as_slice() else {
        return None;
    };
    one.strip_prefix('\'')?.strip_suffix('\'')
}

/// The NAME of a block's one `filter = 'test(=NAME)'`, or why it is not exactly that.
fn exact_selector<'a>(block: &ConfigBlock<'a>) -> Result<&'a str, String> {
    let Some(literal) = block_filter(block) else {
        return Err(
            "unattributable: a mutants timeout needs exactly one single-quoted filter".to_owned(),
        );
    };
    let Some(name) = literal
        .strip_prefix("test(=")
        .and_then(|rest| rest.strip_suffix(')'))
    else {
        return Err(format!(
            "inexact-selector: `{literal}` is not an exact `test(=NAME)` selector"
        ));
    };
    if name.is_empty() || name.contains(|ch: char| ch.is_whitespace() || "()|&!,".contains(ch)) {
        return Err(format!(
            "inexact-selector: `{literal}` is not one exact test name"
        ));
    }
    Ok(name)
}

/// The only keys the committed profile and override tables use. Anything else
/// in those tables (a retry count, a per-test thread rule, an unknown setting)
/// is not something the timeout policy has reviewed, so it is refused before a
/// harmless-looking or group-only block can be accepted.
const RECOGNIZED_KEYS: [&str; 5] = [
    "test-threads",
    "fail-fast",
    "filter",
    "test-group",
    "slow-timeout",
];

/// Refuse every key a canonical profile or override table has not been
/// reviewed for. Runs for every such block, whatever else the block holds.
fn check_recognized_keys(block: &ConfigBlock<'_>, violations: &mut Vec<String>) {
    for (key, _) in &block.keys {
        if !RECOGNIZED_KEYS.contains(key) {
            violations.push(format!(
                "unsupported-form: `{key}` under {} is not a key the #234 policy recognizes",
                block.header
            ));
        }
    }
}

/// A timeout under a header the walker cannot attribute to the baseline or to
/// a `mutants` exception.
fn check_unattributable(block: &ConfigBlock<'_>, violations: &mut Vec<String>) {
    if !block.values("slow-timeout").is_empty() {
        let header = if block.header.is_empty() {
            "no table header"
        } else {
            block.header
        };
        violations.push(format!(
            "unattributable: a slow-timeout under {header} cannot be attributed to the baseline or to an approved exception"
        ));
    }
}

/// `[profile.mutants]` keeps its own 60 s kill: exactly one explicit,
/// non-zero timeout with `grace-period = "0s"`.
fn check_mutants_base(block: &ConfigBlock<'_>, violations: &mut Vec<String>) {
    let timeouts = block.values("slow-timeout");
    let verdict = match timeouts.as_slice() {
        [one] => parse_slow_timeout(one),
        _ => Err(format!(
            "base-timeout: {HEADER_MUTANTS} must carry exactly one slow-timeout, found {}",
            timeouts.len()
        )),
    };
    match verdict {
        Ok(timeout)
            if timeout.grace == Some(0)
                && timeout.period > 0
                && timeout.terminate_after > 0
                && timeout
                    .kill_secs()
                    .is_some_and(|secs| secs <= MUTANTS_BASE_CAP_SECS) => {}
        Ok(_) => violations.push(format!(
            "base-timeout: the mutants default must be a non-zero kill within {MUTANTS_BASE_CAP_SECS} s with grace-period = \"0s\""
        )),
        Err(why) => violations.push(why),
    }
}

/// A timeout in a default or linux table must be one of the committed baseline
/// timeouts, unchanged: the `mutants` run inherits the default profile's
/// overrides, so a new or longer one would silently widen its bound.
fn check_baseline_block(block: &ConfigBlock<'_>, seen: &mut [usize], violations: &mut Vec<String>) {
    let timeouts = block.values("slow-timeout");
    if timeouts.is_empty() {
        return;
    }
    let filter = block_filter(block);
    let matched = BASELINE
        .iter()
        .position(|entry| entry.header == block.header && entry.filter == filter);
    match (timeouts.as_slice(), matched) {
        ([one], Some(index))
            if parse_slow_timeout(one).is_ok_and(|timeout| timeout == BASELINE[index].timeout) =>
        {
            seen[index] += 1;
        }
        _ => violations.push(format!(
            "non-mutants-timeout: {} carries a slow-timeout that is not the committed baseline; only the mutants profile may widen a kill, and the #234 policy in tests/it/gate.rs must be amended deliberately to change this",
            block.header
        )),
    }
}

/// An approved exception must be exactly: an exact-name filter, one finite
/// timeout of at most 180 s with grace zero.
fn check_exception_bound(name: &str, timeout: SlowTimeout, violations: &mut Vec<String>) {
    if timeout.period == 0 || timeout.terminate_after == 0 {
        violations.push(format!(
            "unbounded: the exception for `{name}` has a zero period or terminate-after"
        ));
    }
    if timeout.grace != Some(0) {
        violations.push(format!(
            "grace: the exception for `{name}` must say grace-period = \"0s\" so SIGKILL is not later than SIGTERM"
        ));
    }
    if timeout
        .kill_secs()
        .is_none_or(|secs| secs > MUTANTS_EXCEPTION_CAP_SECS)
    {
        violations.push(format!(
            "over-cap: the exception for `{name}` must kill within {MUTANTS_EXCEPTION_CAP_SECS} s"
        ));
    }
}

/// One `[[profile.mutants.overrides]]`: a group-only block relaxes no kill; one
/// that carries a timeout must be an approved, exact, finite exception.
fn check_mutants_override(
    block: &ConfigBlock<'_>,
    approved_seen: &mut [usize],
    violations: &mut Vec<String>,
) {
    let timeouts = block.values("slow-timeout");
    if timeouts.is_empty() {
        return;
    }
    for (key, _) in &block.keys {
        if !matches!(*key, "filter" | "slow-timeout") {
            violations.push(format!(
                "unsupported-form: `{key}` is not allowed beside a mutants slow-timeout"
            ));
        }
    }
    if timeouts.len() > 1 {
        violations.push(
            "duplicate-block: a mutants override carries more than one slow-timeout".to_owned(),
        );
    }
    let name = match exact_selector(block) {
        Ok(name) => name,
        Err(why) => {
            violations.push(why);
            return;
        }
    };
    let Some(index) = MUTANTS_APPROVED
        .iter()
        .position(|approved| *approved == name)
    else {
        violations.push(format!(
            "unknown-name: `{name}` is not one of the three approved mutants exceptions"
        ));
        return;
    };
    approved_seen[index] += 1;
    if let [one] = timeouts.as_slice() {
        match parse_slow_timeout(one) {
            Ok(timeout) => check_exception_bound(name, timeout, violations),
            Err(why) => violations.push(why),
        }
    }
}

/// Every reason the nextest config's EFFECTIVE `mutants` timeouts are not the
/// approved policy: 60 s base and inherited baseline unchanged, plus exactly the
/// three named exceptions, each once, each in the registered test inventory.
/// Empty means accepted.
fn mutants_timeout_policy(nextest: &str, registered: &[&str]) -> Vec<String> {
    let mut violations = Vec::new();
    let blocks = mutants_policy_blocks(nextest, &mut violations);
    let mut baseline_seen = [0_usize; BASELINE.len()];
    let mut approved_seen = [0_usize; MUTANTS_APPROVED.len()];
    let mut base_blocks = 0_usize;
    for block in &blocks {
        if is_canonical_header(block.header) {
            check_recognized_keys(block, &mut violations);
        }
        match block.header {
            HEADER_MUTANTS => {
                base_blocks += 1;
                check_mutants_base(block, &mut violations);
            }
            HEADER_MUTANTS_OVERRIDE => {
                check_mutants_override(block, &mut approved_seen, &mut violations);
            }
            HEADER_DEFAULT | HEADER_DEFAULT_OVERRIDE | HEADER_LINUX | HEADER_LINUX_OVERRIDE => {
                check_baseline_block(block, &mut baseline_seen, &mut violations);
            }
            _ => check_unattributable(block, &mut violations),
        }
    }
    if base_blocks != 1 {
        violations.push(format!(
            "base-timeout: {HEADER_MUTANTS} must appear exactly once, found {base_blocks}"
        ));
    }
    for (entry, seen) in BASELINE.iter().zip(baseline_seen) {
        if seen != 1 {
            violations.push(format!(
                "non-mutants-timeout: the committed baseline timeout of {} ({}) must appear exactly once, found {seen}; the #234 policy in tests/it/gate.rs must be amended deliberately to change it",
                entry.header,
                entry.filter.unwrap_or("no filter")
            ));
        }
    }
    for (name, seen) in MUTANTS_APPROVED.iter().zip(approved_seen) {
        match seen {
            0 => violations.push(format!(
                "missing-approved: no mutants exception for `{name}`"
            )),
            1 => {}
            _ => violations.push(format!(
                "duplicate-block: `{name}` has {seen} mutants exceptions"
            )),
        }
        match registered
            .iter()
            .filter(|candidate| **candidate == *name)
            .count()
        {
            1 => {}
            0 => violations.push(format!(
                "stale-selector: `{name}` matches no compiled test, so its exception selects nothing"
            )),
            count => violations.push(format!(
                "duplicate-registered: `{name}` is compiled {count} times"
            )),
        }
    }
    violations
}

/// The compiled test names a `--list --format terse` run reported: only exact
/// `name: test` records and empty lines, success with an empty stderr and at
/// least one record. Duplicates are kept: the policy counts them.
fn decode_mutants_inventory(
    outcome: ExitOutcome,
    stdout: &str,
    stderr: &str,
) -> Result<Vec<String>, String> {
    match outcome {
        ExitOutcome::Code(0) => {}
        ExitOutcome::Code(code) => return Err(format!("the listing exited with status {code}")),
        ExitOutcome::Signalled => return Err("the listing was killed by a signal".to_owned()),
    }
    if !stderr.is_empty() {
        return Err(format!("the listing wrote to stderr: {stderr:?}"));
    }
    let mut names = Vec::new();
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        let Some(name) = line.strip_suffix(": test") else {
            return Err(format!(
                "the listing has a line that is not `name: test`: {line:?}"
            ));
        };
        if name.is_empty() || name.trim() != name {
            return Err(format!("the listing has an unusable test name: {line:?}"));
        }
        names.push(name.to_owned());
    }
    if names.is_empty() {
        return Err("the listing named no test".to_owned());
    }
    Ok(names)
}

/// The test names compiled into THIS integration binary: it lists itself once
/// with `--list --format terse` (no test body runs, no cargo, no compile), its
/// output in owned scratch FILES because a pipe would fill before the child
/// exits, the child bounded and reaped by `cli::bounded`. Every failure to
/// acquire the list is an error; nothing substitutes for it.
fn compiled_it_test_names() -> Result<Vec<String>, String> {
    let exe = std::env::current_exe()
        .map_err(|why| format!("the integration test binary has no path: {why}"))?;
    let scratch = super::cli::OwnedScratch::root("gate", "inventory");
    let listing_path = scratch.path().join("listing-out");
    let problems_path = scratch.path().join("listing-err");
    let create = |path: &Path| {
        std::fs::File::create(path).map_err(|why| format!("{}: {why}", path.display()))
    };
    let mut runner = super::cli::helper(&exe);
    runner
        .arg("--list")
        .arg("--format")
        .arg("terse")
        .stdin(std::process::Stdio::null())
        .stdout(create(&listing_path)?)
        .stderr(create(&problems_path)?);
    let child = runner
        .spawn()
        .map_err(|why| format!("the listing child did not start: {why}"))?;
    let finished = super::cli::bounded(child, INVENTORY_BOUND)
        .ok_or_else(|| "the listing child timed out or could not be waited on".to_owned())?;
    let outcome = finished
        .status
        .code()
        .map_or(ExitOutcome::Signalled, ExitOutcome::Code);
    let text = |path: &Path| -> Result<String, String> {
        let bytes = std::fs::read(path).map_err(|why| format!("{}: {why}", path.display()))?;
        String::from_utf8(bytes).map_err(|why| format!("{}: {why}", path.display()))
    };
    decode_mutants_inventory(outcome, &text(&listing_path)?, &text(&problems_path)?)
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
            .env("HOME", &*base)
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
            .env("HOME", &*base)
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
            .env("HOME", &*base)
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

/// A lane run on a `PATH` of its own so no installed `jobs-budget` leaks in:
/// `<base>/bin` links `just` and `tmux`, holds the fake `cargo` with the shell
/// body `cargo`, and a `jobs-budget` printing `budget` when one is given. The
/// returned invocation has its environment and no arguments yet. `HOME` is the
/// scratch itself: the lane's sentry tmux server is started under it.
fn lane_over(base: &Path, budget: Option<&str>, cargo: &str) -> Invocation {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let bin = base.join("bin");
    let _ = std::fs::remove_dir_all(&bin);
    std::fs::create_dir_all(&bin).unwrap_or_else(|why| panic!("the fakes' dir: {why}"));
    let path = std::env::var_os("PATH").unwrap_or_default();
    for tool in ["just", "tmux"] {
        let Some(real) = std::env::split_paths(&path)
            .map(|dir| dir.join(tool))
            .find(|file| file.is_file())
        else {
            panic!("{tool} must be on PATH");
        };
        symlink(real, bin.join(tool)).unwrap_or_else(|why| panic!("the linked {tool}: {why}"));
    }
    // A quiet machine by default, so no lane test depends on the load the host has right now;
    // a test that needs another reading writes its own `uptime` over this one.
    let mut fakes = vec![
        ("cargo", cargo.to_owned()),
        (
            "uptime",
            "echo ' 15:25  up 1 day, 6 users, load averages: 0.50 0.50 0.50'\n".to_owned(),
        ),
    ];
    if let Some(answer) = budget {
        fakes.push(("jobs-budget", format!("echo '{answer}'\n")));
    }
    for (name, body) in fakes {
        let file = bin.join(name);
        std::fs::write(&file, format!("#!/bin/sh\n{body}"))
            .and_then(|()| std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)))
            .unwrap_or_else(|why| panic!("the fake {name}: {why}"));
    }
    Invocation::new("just")
        .env_cleared()
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
        )
        .env("HOME", base)
        .env("TMPDIR", base)
        .env("AE_TEST_TMPDIR", base)
}

/// One lane run for the sizing pins: the fake `cargo` records `<tool> <threads>
/// <jobs or unset> <niceness> <parent niceness>` for a nextest, llvm-cov or
/// mutants run (`detached` forks before `nice`, so the parent keeps the lane's
/// own). Returns the recorded words and the lane's stderr.
fn sized_lane(
    base: &Path,
    budget: Option<&str>,
    env: &[(&str, &str)],
    args: &[&str],
) -> (Vec<String>, String) {
    let cargo = "case \"$1\" in nextest|llvm-cov|mutants) ;; *) exit 0 ;; esac\n\
         echo $1 $NEXTEST_TEST_THREADS ${CARGO_BUILD_JOBS-unset} $(ps -o ni= -p $$) \
         $(ps -o ni= -p $PPID) >\"$AE_TEST_TMPDIR/seen\"\n";
    let mut lane = lane_over(base, budget, cargo).arg("_tmux-isolated");
    for arg in args {
        lane = lane.arg(arg);
    }
    for (key, value) in env {
        lane = lane.env(key, value);
    }
    let _ = std::fs::remove_file(base.join("seen"));
    let ran = raw::run(&lane, &root(), &base.join("out"), &base.join("err"))
        .unwrap_or_else(|why| panic!("the lane runs: {why}"));
    let stderr = read(&base.join("err"));
    assert!(
        matches!(ran.outcome(), ExitOutcome::Code(0)),
        "the lane must end green: {stderr}"
    );
    let seen = std::fs::read_to_string(base.join("seen")).unwrap_or_default();
    let words: Vec<String> = seen.split_whitespace().map(str::to_owned).collect();
    assert_eq!(words.len(), 5, "{args:?} ran no cargo: {stderr}");
    (words, stderr)
}

/// The lane starts a tmux server of its own, the sentry, and a pane inherits its
/// server's environment: so the `HOME` the lane runs under is the `HOME` that
/// server and every pane under it carries. A lane a test runs is confined to
/// the test's scratch, never the operator's home. The fake `cargo` asks the
/// sentry itself, from inside the lane, while that server is alive.
#[test]
fn the_lane_sentry_server_sees_the_fixture_home_not_the_operators() {
    let base = super::cli::OwnedScratch::root("gate", "sentryhome");
    let cargo = "case \"$1\" in nextest) tmux -L ae show-environment -g HOME \
                 >\"$AE_TEST_TMPDIR/sentry-home\" ;; *) exit 0 ;; esac\n";
    let lane = lane_over(&base, Some("6"), cargo)
        .arg("_tmux-isolated")
        .arg("test");
    let ran = raw::run(&lane, &root(), &base.join("out"), &base.join("err"))
        .unwrap_or_else(|why| panic!("the lane runs: {why}"));
    let stderr = read(&base.join("err"));
    assert!(
        matches!(ran.outcome(), ExitOutcome::Code(0)),
        "the lane must end green: {stderr}"
    );
    let seen = std::fs::read_to_string(base.join("sentry-home")).unwrap_or_default();
    let home = seen.trim().strip_prefix("HOME=").unwrap_or_default();
    assert!(
        !home.is_empty() && Path::new(home).starts_with(&*base),
        "the sentry must carry the fixture's HOME {}, not {seen:?}: {stderr}",
        base.display()
    );
}

/// A lane sizes each count as the LEAST of its ceilings and names every one it
/// applied on stderr. Threads: `jobs-budget` (the CPU count without it), the tmux
/// lane ceiling of sixteen, and an inherited `NEXTEST_TEST_THREADS`. Build jobs:
/// `jobs-budget` and an inherited `CARGO_BUILD_JOBS`. A shell profile that
/// exports a wide count (a fixed 16, or `jobs-budget --max`) may LOWER a lane
/// but never lift it past the live budget.
/// The deliberate per-run override is the lane's own `AE_TEST_THREADS` /
/// `AE_TEST_JOBS`, never the global names.
#[test]
fn a_lane_sizes_itself_as_the_least_of_its_ceilings_and_only_a_lane_override_lifts_them() {
    let base = super::cli::OwnedScratch::root("gate", "sizing");
    let both = |threads: &'static str, jobs: &'static str| {
        [
            ("NEXTEST_TEST_THREADS", threads),
            ("CARGO_BUILD_JOBS", jobs),
        ]
    };
    let none = &[][..];
    let explicit = both("3", "5");
    let profile = both("32", "32");
    let tie = both("6", "6");
    let junk = both("num-cpus", "default");
    // The relative forms a user config or profile may hold (cargo and nextest
    // read -2 as the CPU count minus two): no ceiling to compare, so the budget.
    let relative = both("-2", "-2");
    let empty = both("", "");
    let wide = both("1000", "1000");
    let lift = [
        ("NEXTEST_TEST_THREADS", "16"),
        ("CARGO_BUILD_JOBS", "16"),
        ("AE_TEST_THREADS", "12"),
        ("AE_TEST_JOBS", "20"),
    ];
    // (budget, env, (threads, source), (jobs, source))
    for (budget, env, (threads, from), (jobs, jobs_from)) in [
        ("6", none, ("6", "budget"), ("6", "budget")),
        ("999", none, ("16", "budget"), ("999", "budget")),
        // An inherited value below the budget lowers the lane.
        (
            "6",
            &explicit[..],
            ("3", "env; budget 6"),
            ("5", "env; budget 6"),
        ),
        ("6", &explicit[..1], ("3", "env; budget 6"), ("6", "budget")),
        ("6", &explicit[1..], ("6", "budget"), ("5", "env; budget 6")),
        // The wide profile export never lifts a lane past the live budget.
        (
            "6",
            &profile[..],
            ("6", "budget; env 32"),
            ("6", "budget; env 32"),
        ),
        // ... nor past the lane ceiling of sixteen threads; jobs may follow it.
        (
            "999",
            &profile[..],
            ("16", "budget; env 32"),
            ("32", "env; budget 999"),
        ),
        // A tie is the budget's, the equal value still named.
        (
            "6",
            &tie[..],
            ("6", "budget; env 6"),
            ("6", "budget; env 6"),
        ),
        // Nothing but a plain positive integer of at most three digits is a ceiling.
        (
            "6",
            &junk[..],
            ("6", "budget; env num-cpus ignored"),
            ("6", "budget; env default ignored"),
        ),
        (
            "6",
            &relative[..],
            ("6", "budget; env -2 ignored"),
            ("6", "budget; env -2 ignored"),
        ),
        (
            "999",
            &relative[..],
            ("16", "budget; env -2 ignored"),
            ("999", "budget; env -2 ignored"),
        ),
        // An empty export is no export.
        ("6", &empty[..], ("6", "budget"), ("6", "budget")),
        (
            "6",
            &wide[..],
            ("6", "budget; env 1000 ignored"),
            ("6", "budget; env 1000 ignored"),
        ),
        // The lane's own override replaces the rule, whatever the profile says.
        ("6", &lift[..], ("12", "override"), ("20", "override")),
        ("6", &lift[2..3], ("12", "override"), ("6", "budget")),
    ] {
        let (seen, stderr) = sized_lane(&base, Some(budget), env, &["test"]);
        assert_eq!(seen[1..3], [threads, jobs], "budget {budget} env {env:?}");
        let said = format!("threads {threads} ({from}), cargo jobs {jobs} ({jobs_from})");
        assert!(
            stderr.contains(&said),
            "budget {budget} env {env:?}: {stderr}"
        );
    }
}

/// Without a `jobs-budget` the CPU count and the cap of sixteen are the threads'
/// ceiling, an inherited wide value cannot lift them, and the jobs have no live
/// ceiling to compare with, so an inherited value stands untouched (cargo reads
/// a relative one itself). A relative threads value is no ceiling and never
/// enters the arithmetic.
#[test]
fn a_lane_without_jobs_budget_keeps_the_cpu_cap_and_leaves_an_inherited_jobs_value() {
    let base = super::cli::OwnedScratch::root("gate", "nobudget");
    let profile = [("NEXTEST_TEST_THREADS", "32"), ("CARGO_BUILD_JOBS", "16")];
    let relative = [("NEXTEST_TEST_THREADS", "-2"), ("CARGO_BUILD_JOBS", "-2")];
    let (seen, stderr) = sized_lane(&base, None, &profile[..], &["test"]);
    let threads: usize = seen[1].parse().expect("the recorded threads");
    assert!((1..=16).contains(&threads), "{seen:?}: {stderr}");
    assert_eq!(seen[2], "16", "{stderr}");
    let said = format!("threads {threads} (cpus; env 32), cargo jobs 16 (env)");
    assert!(stderr.contains(&said), "{stderr}");
    let (seen, stderr) = sized_lane(&base, None, &relative[..], &["test"]);
    let threads: usize = seen[1].parse().expect("the recorded threads");
    assert!((1..=16).contains(&threads), "{seen:?}: {stderr}");
    assert_eq!(seen[2], "-2", "{stderr}");
    let said = format!("threads {threads} (cpus; env -2 ignored), cargo jobs -2 (env)");
    assert!(stderr.contains(&said), "{stderr}");
}

/// A per-run override that is not one positive integer of at most three digits
/// is a typo, not a wish: the lane refuses (status 2) before it runs cargo
/// rather than size itself by a guess.
#[test]
fn an_unusable_lane_override_refuses_before_cargo_runs() {
    let base = super::cli::OwnedScratch::root("gate", "overridebad");
    let cargo = "case \"$1\" in nextest) echo ran >\"$AE_TEST_TMPDIR/seen\" ;; esac\n";
    for (var, value) in [
        ("AE_TEST_THREADS", "0"),
        ("AE_TEST_THREADS", "many"),
        ("AE_TEST_THREADS", "3 4"),
        ("AE_TEST_THREADS", "1000"),
        ("AE_TEST_JOBS", "-1"),
        ("AE_TEST_JOBS", "8x"),
    ] {
        let _ = std::fs::remove_file(base.join("seen"));
        let lane = lane_over(&base, Some("6"), cargo)
            .env(var, value)
            .arg("_tmux-isolated")
            .arg("test");
        let ran = raw::run(&lane, &root(), &base.join("out"), &base.join("err"))
            .unwrap_or_else(|why| panic!("the lane runs: {why}"));
        let stderr = read(&base.join("err"));
        assert!(
            matches!(ran.outcome(), ExitOutcome::Code(2)),
            "{var}={value:?} must refuse with status 2: {stderr}"
        );
        assert!(stderr.contains(var), "{var}={value:?}: {stderr}");
        assert!(
            !base.join("seen").exists(),
            "{var}={value:?}: cargo ran anyway"
        );
    }
}

/// A mutation run is never wider than two tests. An env value beats a profile,
/// so the lane itself must not hand nextest a wider one: it tightens to one on
/// request and caps anything else at two, said on stderr. The `test` lane keeps
/// its own ceilings.
#[test]
fn a_mutants_lane_never_runs_wider_than_two_tests() {
    let base = super::cli::OwnedScratch::root("gate", "mutcap");
    let with = |threads: &'static str| [("NEXTEST_TEST_THREADS", threads)];
    let over = |threads: &'static str| [("AE_TEST_THREADS", threads)];
    let none = &[][..];
    // (budget, env, threads, how the lane says it got there)
    for (budget, env, threads, said) in [
        ("6", none, "2", "threads 2 (budget, was 6, capped)"),
        ("1", none, "1", "threads 1 (budget)"),
        ("6", &with("1")[..], "1", "threads 1 (env; budget 6)"),
        ("6", &with("2")[..], "2", "threads 2 (env; budget 6)"),
        (
            "6",
            &with("3")[..],
            "2",
            "threads 2 (env; budget 6, was 3, capped)",
        ),
        (
            "6",
            &with("8")[..],
            "2",
            "threads 2 (budget; env 8, was 6, capped)",
        ),
        (
            "6",
            &with("num-cpus")[..],
            "2",
            "(budget; env num-cpus ignored, was 6, capped)",
        ),
        (
            "6",
            &with("-1")[..],
            "2",
            "(budget; env -1 ignored, was 6, capped)",
        ),
        ("6", &over("1")[..], "1", "threads 1 (override)"),
        ("6", &over("2")[..], "2", "threads 2 (override)"),
        (
            "6",
            &over("8")[..],
            "2",
            "threads 2 (override, was 8, capped)",
        ),
    ] {
        let (seen, stderr) = sized_lane(&base, Some(budget), env, &["mutants"]);
        assert_eq!(seen[1], threads, "budget {budget} env {env:?}: {stderr}");
        assert!(
            stderr.contains(said),
            "budget {budget} env {env:?}: {stderr}"
        );
    }
    // The `test` lane keeps its sixteen-wide ceiling: the cap is the mutants lane's,
    // and an inherited wider count no longer lifts it past the budget.
    let (seen, _) = sized_lane(&base, Some("6"), &with("8")[..], &["test"]);
    assert_eq!(seen[1], "6");
    let (seen, _) = sized_lane(&base, Some("999"), &with("32")[..], &["test"]);
    assert_eq!(seen[1], "16");
}

/// One `test` lane run of the REAL justfile with its named lane ceiling
/// (`lane_threads`) replaced by `ceiling`, so a raised ceiling is proven without
/// raising the shipped one, and a fake `uptime` on the lane's private `PATH`:
/// one that prints `uptime`, or, for `None`, one that fails. The fake `cargo` is
/// the recording one of [`sized_lane`]; the run must end green whatever the load.
/// Returns the recorded words and the lane's stderr.
fn loaded_lane(
    base: &Path,
    ceiling: u32,
    uptime: Option<&str>,
    budget: &str,
    env: &[(&str, &str)],
) -> (Vec<String>, String) {
    use std::os::unix::fs::PermissionsExt as _;
    let named = "\n    lane_threads=16\n";
    let justfile = read(&root().join("justfile"));
    assert_eq!(
        justfile.matches(named).count(),
        1,
        "the lane ceiling is ONE named line of the SIZING helper"
    );
    let fixture = base.join("justfile.fixture");
    std::fs::write(
        &fixture,
        justfile.replace(named, &format!("\n    lane_threads={ceiling}\n")),
    )
    .unwrap_or_else(|why| panic!("the fixture justfile: {why}"));
    let cargo = "case \"$1\" in nextest) ;; *) exit 0 ;; esac\n\
         echo $1 $NEXTEST_TEST_THREADS ${CARGO_BUILD_JOBS-unset} >\"$AE_TEST_TMPDIR/seen\"\n";
    let mut lane = lane_over(base, Some(budget), cargo);
    let fake = base.join("bin").join("uptime");
    let body = uptime.map_or_else(
        || "#!/bin/sh\nexit 1\n".to_owned(),
        |line| format!("#!/bin/sh\necho '{line}'\n"),
    );
    std::fs::write(&fake, body)
        .and_then(|()| std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)))
        .unwrap_or_else(|why| panic!("the fake uptime: {why}"));
    lane = lane
        .arg("--justfile")
        .arg(&fixture)
        .arg("--working-directory")
        .arg(root())
        .arg("_tmux-isolated")
        .arg("test");
    for (key, value) in env {
        lane = lane.env(key, value);
    }
    let _ = std::fs::remove_file(base.join("seen"));
    let ran = raw::run(&lane, &root(), &base.join("out"), &base.join("err"))
        .unwrap_or_else(|why| panic!("the lane runs: {why}"));
    let stderr = read(&base.join("err"));
    assert!(
        matches!(ran.outcome(), ExitOutcome::Code(0)),
        "load never aborts the lane: {stderr}"
    );
    let seen = std::fs::read_to_string(base.join("seen")).unwrap_or_default();
    let words: Vec<String> = seen.split_whitespace().map(str::to_owned).collect();
    assert_eq!(words.len(), 3, "{uptime:?} ran no cargo: {stderr}");
    (words, stderr)
}

/// The lane starts at its named ceiling on a quiet machine and falls to the
/// loaded cap when load1 is ABOVE 14 at the start, or cannot be read (that counts
/// as high), and says so on stderr. The guard only lowers and never refuses:
/// every case below ends green. An explicit `AE_TEST_THREADS` replaces the whole
/// rule and an inherited export still only lowers. The ceiling is set to 12
/// in a copy of the justfile, and the SHIPPED sixteen is shown once below: quiet reaches it,
/// a busy start falls to eight.
#[test]
fn a_busy_machine_lowers_the_lanes_threads_and_never_refuses_the_run() {
    let base = super::cli::OwnedScratch::root("gate", "loadguard");
    let mac = |load: &str| format!(" 15:25  up 1 day, 6 users, load averages: {load} 3.00 3.00");
    let linux =
        |load: &str| format!(" 15:25:01 up 1 day, 6 users,  load average: {load}, 9.00, 9.00");
    let none = &[][..];
    let seen = |ceiling, uptime: Option<String>, budget, env: &[(&str, &str)]| {
        loaded_lane(&base, ceiling, uptime.as_deref(), budget, env)
    };
    // A quiet machine, and load1 AT the ceiling, keep the lane's own ceiling.
    for load in [mac("0.50"), mac("14.00"), linux("14.00"), mac("9.99")] {
        let (words, stderr) = seen(12, Some(load.clone()), "999", none);
        assert_eq!(words[1], "12", "{load}: {stderr}");
        assert!(stderr.contains("nextest threads 12 (budget)"), "{stderr}");
        assert!(!stderr.contains("load1"), "{load}: said nothing: {stderr}");
    }
    // The shipped ceiling of sixteen: a quiet start reaches it, a busy one falls to eight.
    let (words, stderr) = seen(16, Some(mac("0.50")), "999", none);
    assert_eq!(words[1], "16", "{stderr}");
    let (words, stderr) = seen(16, Some(mac("14.01")), "999", none);
    assert_eq!(words[1], "8", "{stderr}");
    assert!(stderr.contains("was 16, capped"), "{stderr}");
    // ABOVE the ceiling: the loaded cap, the cause on stderr, no refusal.
    for load in [mac("14.01"), mac("24.02"), linux("15.00"), mac("120.50")] {
        let (words, stderr) = seen(12, Some(load.clone()), "999", none);
        assert_eq!(words[1], "8", "{load}: {stderr}");
        assert!(
            stderr.contains("is above 14: nextest threads at most 8"),
            "{load}: {stderr}"
        );
        assert!(stderr.contains("was 12, capped"), "{load}: {stderr}");
    }
    // Unreadable counts as high: a line with no figure, a failing uptime.
    for uptime in [
        Some(" 15:25  up 1 day, 6 users, no figures here".to_owned()),
        None,
    ] {
        let (words, stderr) = seen(12, uptime.clone(), "999", none);
        assert_eq!(words[1], "8", "{uptime:?}: {stderr}");
        assert!(
            stderr.contains("load1 cannot be read: nextest threads at most 8"),
            "{uptime:?}: {stderr}"
        );
    }
    // The cap only lowers: a small budget stays small, and a load at 8 or below
    // the lane count is still said.
    let (words, stderr) = seen(12, Some(mac("30.00")), "6", none);
    assert_eq!(words[1], "6", "{stderr}");
    assert!(
        stderr.contains("is above 14: nextest threads at most 8"),
        "{stderr}"
    );
    assert!(!stderr.contains("capped"), "{stderr}");
    // An inherited export still only lowers; the lane override replaces the rule
    // and is not guarded, whatever the load says.
    let (words, _) = seen(
        12,
        Some(mac("30.00")),
        "999",
        &[("NEXTEST_TEST_THREADS", "3")],
    );
    assert_eq!(words[1], "3");
    let (words, stderr) = seen(8, Some(mac("30.00")), "6", &[("AE_TEST_THREADS", "12")]);
    assert_eq!(words[1], "12", "{stderr}");
    assert!(
        !stderr.contains("load1"),
        "an override is not guarded: {stderr}"
    );
}

/// Whether the SIZING helper names the three values of the thread guard once each
/// (`lane_threads`, `loaded_threads`, `load_ceiling`), spends none of them as a
/// literal, and owns the one reading of the load: the lane recipe reads no
/// `uptime` and carries no literal fourteen of its own.
fn load_guard_named_ok(justfile: &str) -> bool {
    let helper = justfile_variable(justfile, "SIZING");
    let lane = recipe_text(justfile, "_tmux-isolated lane *args:").join("\n");
    let once = |name: &str| {
        helper
            .lines()
            .filter(|line| {
                line.trim().strip_prefix(name).is_some_and(|rest| {
                    rest.starts_with('=') && rest[1..].bytes().all(|b| b.is_ascii_digit())
                })
            })
            .count()
            == 1
    };
    once("lane_threads")
        && once("loaded_threads")
        && once("load_ceiling")
        && helper.contains("< lane_threads ? ceiling : lane_threads")
        && helper.contains("size_load()")
        && helper.matches("uptime").count() == 1
        && !lane.contains("load averages")
        && !lane.contains(" 14")
        && lane.contains("size_load \"lane $lane\"")
        && lane
            .find("size_counts")
            .zip(lane.find("size_load"))
            .is_some_and(|(counts, load)| counts < load)
}

#[test]
fn the_thread_ceiling_and_the_load_guard_are_named_values_with_one_load_reading() {
    let justfile = read(&root().join("justfile"));
    assert!(
        load_guard_named_ok(&justfile),
        "SIZING must name lane_threads, loaded_threads and load_ceiling once each, read the \
         load in one place and be guarded by size_load right after size_counts"
    );
    // The shipped values: the lane ceiling is the P0 thread matrix's sixteen, a busy
    // start falls back to the old default of eight, and the start ceiling is the
    // doctrine's fourteen.
    for pin in [
        "\n    lane_threads=16\n",
        "\n    loaded_threads=8\n",
        "\n    load_ceiling=14\n",
    ] {
        assert_eq!(justfile.matches(pin).count(), 1, "{pin:?}");
    }
    // RED — a literal eight back in the arithmetic, a second reading of the load
    // in the lane, a second definition, and a guard that runs before the sizing.
    assert!(!load_guard_named_ok(&justfile.replace(
        "< lane_threads ? ceiling : lane_threads",
        "< 8 ? ceiling : 8"
    )));
    assert!(!load_guard_named_ok(&justfile.replace(
        "    unset TMUX TMUX_PANE\n",
        "    unset TMUX TMUX_PANE\n    sed -n 's/.*load averages//p'\n"
    )));
    assert!(!load_guard_named_ok(&justfile.replace(
        "\n    load_ceiling=14\n",
        "\n    load_ceiling=14\n    load_ceiling=15\n"
    )));
    assert!(!load_guard_named_ok(
        &justfile.replace("\n    loaded_threads=8\n", "\n")
    ));
    assert!(!load_guard_named_ok(&justfile.replace(
        "    size_counts\n    size_load \"lane $lane\"\n",
        "    size_load \"lane $lane\"\n    size_counts\n"
    )));
}

/// One `<testsuite>` row of a fake nextest `JUnit` report for a stress run, shaped
/// as nextest 0.9.143 writes it: one row per test binary and iteration.
struct Suite {
    binary: &'static str,
    /// Zero-based, as nextest's `@stress-N` counts.
    iteration: u32,
    tests: u32,
    errors: u32,
    failures: u32,
    /// Tests that failed first and passed on a retry.
    flaky: u32,
}

impl Suite {
    fn pass(iteration: u32) -> Self {
        Self {
            binary: "ae::it",
            iteration,
            tests: 3,
            errors: 0,
            failures: 0,
            flaky: 0,
        }
    }
}

/// The text of a fake `JUnit` report for `suites`; `closed` false cuts it short
/// before the closing `</testsuites>`, as a killed writer would leave it.
fn stress_junit(suites: &[Suite], closed: bool) -> String {
    use std::fmt::Write as _;
    let sum = |field: fn(&Suite) -> u32| suites.iter().map(field).sum::<u32>();
    let mut text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <testsuites name=\"nextest-run\" tests=\"{}\" skipped=\"0\" failures=\"{}\" errors=\"{}\" \
         uuid=\"22c298ad-040b-4fb0-9b16-d5e3426060a8\" time=\"0.030\">\n",
        sum(|suite| suite.tests),
        sum(|suite| suite.failures),
        sum(|suite| suite.errors),
    );
    for suite in suites {
        let _ = writeln!(
            text,
            "    <testsuite name=\"{}@stress-{}\" tests=\"{}\" skipped=\"0\" errors=\"{}\" failures=\"{}\">",
            suite.binary, suite.iteration, suite.tests, suite.errors, suite.failures
        );
        for case in 0..suite.tests {
            let child = if case < suite.failures {
                "<failure message=\"panicked\" type=\"test failure with exit code 101\">boom</failure>"
            } else if case < suite.failures + suite.errors {
                "<error message=\"spawn\" type=\"execution failure\">boom</error>"
            } else if case < suite.failures + suite.errors + suite.flaky {
                "<flakyFailure message=\"panicked\" type=\"test failure with exit code 101\">boom</flakyFailure>"
            } else {
                ""
            };
            if child.is_empty() {
                let _ = writeln!(
                    text,
                    "        <testcase name=\"t{case}\" classname=\"{}\" time=\"0.010\"/>",
                    suite.binary
                );
            } else {
                let _ = write!(
                    text,
                    "        <testcase name=\"t{case}\" classname=\"{}\" time=\"0.010\">\n            {child}\n        </testcase>\n",
                    suite.binary
                );
            }
        }
        text.push_str("    </testsuite>\n");
    }
    if closed {
        text.push_str("</testsuites>\n");
    }
    text
}

/// One `just rust-stress` run whose fake nextest leaves a report saying every
/// iteration of the named repeat count passed. See [`stress_lane_with`].
fn stress_lane(
    base: &Path,
    load: &str,
    cargo: &str,
    env: &[(&str, &str)],
    args: &[&str],
) -> Option<(ExitOutcome, String)> {
    let count: u32 = args.get(1).and_then(|raw| raw.parse().ok()).unwrap_or(1);
    let passing: Vec<Suite> = (0..count).map(Suite::pass).collect();
    stress_lane_with(
        base,
        load,
        cargo,
        env,
        args,
        Some(&stress_junit(&passing, true)),
    )
}

/// One `just rust-stress` run on the same private `PATH` as the other lane
/// pins, with a fake `uptime` saying `load` and the host's real `timeout` (or
/// GNU `gtimeout`) linked as `timeout`. The fake `cargo` records, for a nextest
/// run, one argument per line, then the threads it was handed, its own niceness
/// and the lane's. A `report` is copied by the fake nextest to where the lane
/// reads its `JUnit` report (a private `CARGO_TARGET_DIR`, so no test touches the
/// checkout's own report); `None` leaves nothing there. `None` back, after
/// proving the recipe refuses with status 2 and names `timeout`, where the host
/// has neither tool.
fn stress_lane_with(
    base: &Path,
    load: &str,
    cargo: &str,
    env: &[(&str, &str)],
    args: &[&str],
    report: Option<&str>,
) -> Option<(ExitOutcome, String)> {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let report_file = base.join("report.xml");
    let _ = std::fs::remove_file(&report_file);
    if let Some(text) = report {
        std::fs::write(&report_file, text).unwrap_or_else(|why| panic!("the fake report: {why}"));
    }
    let cargo = format!(
        "if [ \"$1\" = nextest ] && [ -f \"$AE_TEST_TMPDIR/report.xml\" ]; then \
         mkdir -p \"$CARGO_TARGET_DIR/nextest/default\" && \
         cp \"$AE_TEST_TMPDIR/report.xml\" \"$CARGO_TARGET_DIR/nextest/default/junit.xml\"; fi\n{cargo}"
    );
    let mut lane =
        lane_over(base, Some("6"), &cargo).env("CARGO_TARGET_DIR", base.join("cargo-target"));
    let bin = base.join("bin");
    let uptime = bin.join("uptime");
    std::fs::write(&uptime, format!("#!/bin/sh\necho '{load}'\n"))
        .and_then(|()| std::fs::set_permissions(&uptime, std::fs::Permissions::from_mode(0o755)))
        .unwrap_or_else(|why| panic!("the fake uptime: {why}"));
    let path = std::env::var_os("PATH").unwrap_or_default();
    let host = ["timeout", "gtimeout"].into_iter().find_map(|tool| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(tool))
            .find(|file| file.is_file())
    });
    let linked = host.is_some();
    if let Some(real) = host {
        symlink(real, bin.join("timeout"))
            .unwrap_or_else(|why| panic!("the linked timeout: {why}"));
    }
    lane = lane.arg("rust-stress");
    for arg in args {
        lane = lane.arg(arg);
    }
    for (key, value) in env {
        lane = lane.env(key, value);
    }
    let _ = std::fs::remove_file(base.join("seen"));
    let ran = raw::run(&lane, &root(), &base.join("out"), &base.join("err"))
        .unwrap_or_else(|why| panic!("the stress lane runs: {why}"));
    let stderr = read(&base.join("err"));
    if !linked {
        assert!(
            matches!(ran.outcome(), ExitOutcome::Code(2)) && stderr.contains("timeout"),
            "without timeout or gtimeout the recipe must refuse with status 2: {stderr}"
        );
        return None;
    }
    Some((ran.outcome(), stderr))
}

/// `just rust-stress` repeats ONE named filter and refuses to start while the
/// machine is busy: load1 above 14 (the doctrine's start ceiling) refuses before
/// any lane state exists, an unreadable load refuses too, and 14.00 itself or a
/// quiet machine runs. The refusal never reaches cargo.
#[test]
fn a_stress_run_refuses_above_load_14_and_before_it_starts_anything() {
    let base = super::cli::OwnedScratch::root("gate", "stressload");
    let cargo = "case \"$1\" in nextest) echo ran >\"$AE_TEST_TMPDIR/seen\" ;; esac\n";
    let args = ["test(=menu::x)", "3", "30"];
    // (the uptime line, whether the run starts)
    for (load, starts) in [
        (
            " 15:25  up 1 day, 6 users, load averages: 24.02 23.94 26.50",
            false,
        ),
        (
            " 15:25  up 1 day, 6 users, load averages: 14.01 3.00 3.00",
            false,
        ),
        (
            " 15:25:01 up 1 day, 6 users,  load average: 15.00, 14.00, 13.00",
            false,
        ),
        (
            " 15:25  up 1 day, 6 users, load averages: 120.50 90.00 60.00",
            false,
        ),
        // Fail closed: a line with no load figure is no proof of a quiet machine.
        (" 15:25  up 1 day, 6 users, no figures here", false),
        (
            " 15:25  up 1 day, 6 users, load averages: 14.00 20.00 20.00",
            true,
        ),
        (
            " 15:25:01 up 1 day, 6 users,  load average: 0.52, 9.00, 9.00",
            true,
        ),
    ] {
        let Some((outcome, stderr)) = stress_lane(&base, load, cargo, &[], &args) else {
            return;
        };
        let ran = base.join("seen").exists();
        assert_eq!(ran, starts, "{load:?}: {stderr}");
        if starts {
            assert!(
                matches!(outcome, ExitOutcome::Code(0)),
                "{load:?}: {stderr}"
            );
        } else {
            assert!(
                matches!(outcome, ExitOutcome::Code(1)),
                "{load:?}: {stderr}"
            );
            assert!(
                stderr.contains("load") && stderr.contains("14"),
                "{load:?}: {stderr}"
            );
        }
    }
    // A refusal leaves no lane directory behind: it came before any state.
    let lanes: Vec<String> = std::fs::read_dir(&*base)
        .expect("the lane base")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|entry| entry.starts_with("ae-rust-test."))
        .collect();
    assert!(lanes.is_empty(), "lane directories left: {lanes:?}");
}

/// The recipe takes a filter, a repeat count and a time bound, nothing else:
/// an empty or whole-suite filter, a count or bound that is not one positive
/// integer (three digits, five digits) refuses with status 2 before any load
/// reading or lane state.
#[test]
fn a_stress_run_needs_a_named_filter_and_plain_bounds() {
    let base = super::cli::OwnedScratch::root("gate", "stressargs");
    let cargo = "case \"$1\" in nextest) echo ran >\"$AE_TEST_TMPDIR/seen\" ;; esac\n";
    let quiet = " 15:25  up 1 day, 6 users, load averages: 0.50 0.50 0.50";
    for args in [
        ["", "3", "30"],
        ["all()", "3", "30"],
        ["test(=menu::x)", "0", "30"],
        ["test(=menu::x)", "many", "30"],
        ["test(=menu::x)", "1000", "30"],
        ["test(=menu::x)", "3", "0"],
        ["test(=menu::x)", "3", "soon"],
        ["test(=menu::x)", "3", "100000"],
    ] {
        let Some((outcome, stderr)) = stress_lane(&base, quiet, cargo, &[], &args) else {
            return;
        };
        assert!(
            matches!(outcome, ExitOutcome::Code(2)),
            "{args:?}: {stderr}"
        );
        assert!(!base.join("seen").exists(), "{args:?}: cargo ran anyway");
    }
}

/// A stress run hands nextest exactly the named filter, the repeat count and
/// `--test-threads` from the lane's sizing (the least of the budget, the lane
/// ceiling and a wide inherited export), at `nice -n 10`; and when its time bound
/// ends it, the whole group goes: status 124 and no child of cargo survives, the
/// lane directory included.
#[test]
fn a_stress_run_is_sized_niced_bounded_and_leaves_no_child_behind() {
    let base = super::cli::OwnedScratch::root("gate", "stressrun");
    let quiet = " 15:25  up 1 day, 6 users, load averages: 0.50 0.50 0.50";
    let record = "case \"$1\" in nextest) ;; *) exit 0 ;; esac\n\
         { printf '%s\\n' \"$@\"; echo \"$NEXTEST_TEST_THREADS\"; ps -o ni= -p $$; } \
         >\"$AE_TEST_TMPDIR/seen\"\n";
    let profile = [("NEXTEST_TEST_THREADS", "16"), ("CARGO_BUILD_JOBS", "16")];
    let filter = "test(=menu::a) | test(=menu::b)";
    let Some((outcome, stderr)) = stress_lane(&base, quiet, record, &profile, &[filter, "7", "30"])
    else {
        return;
    };
    assert!(matches!(outcome, ExitOutcome::Code(0)), "{stderr}");
    let seen = read(&base.join("seen"));
    let lines: Vec<&str> = seen.lines().collect();
    let want = [
        "nextest",
        "run",
        "--locked",
        "--all-features",
        "--test-threads",
        "6",
        "--stress-count",
        "7",
        "-E",
        filter,
        "6",
    ];
    assert_eq!(lines[..want.len()], want, "{stderr}");
    let own: i32 = lines[want.len()].trim().parse().expect("cargo's niceness");
    // `nice` with no argument prints the niceness it inherited: this test's own,
    // which is the lane's (the fake `cargo` is two exec hops below it).
    raw::run(
        &Invocation::new("nice"),
        &base,
        &base.join("nice-out"),
        &base.join("nice-err"),
    )
    .unwrap_or_else(|why| panic!("nice runs: {why}"));
    let lane_nice: i32 = read(&base.join("nice-out")).trim().parse().unwrap_or(0);
    assert!(
        own > lane_nice || own >= 19,
        "cargo {own}, lane {lane_nice}"
    );

    // The bound: a cargo that leaves a background child and then waits.
    let hang = "case \"$1\" in nextest) ;; *) exit 0 ;; esac\n\
         sleep 300 &\n\
         echo $! >\"$AE_TEST_TMPDIR/child.tmp\" && mv \"$AE_TEST_TMPDIR/child.tmp\" \"$AE_TEST_TMPDIR/child\"\n\
         wait\n";
    let _ = std::fs::remove_file(base.join("child"));
    let Some((outcome, stderr)) = stress_lane(&base, quiet, hang, &[], &[filter, "2", "1"]) else {
        return;
    };
    assert!(
        matches!(outcome, ExitOutcome::Code(124)) && stderr.contains("bound ended the run"),
        "the bound ends it and the lane says so: {stderr}"
    );
    let child = wait_written(&base.join("child"));
    let pid = child.trim();
    let alive = |pid: &str| {
        matches!(
            raw::run(
                &Invocation::new("kill").arg("-0").arg(pid),
                &base,
                &base.join("kill-out"),
                &base.join("kill-err"),
            )
            .map(|ran| ran.outcome()),
            Ok(ExitOutcome::Code(0))
        )
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while alive(pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let survived = alive(pid);
    if survived {
        let _ = raw::run(
            &Invocation::new("kill").arg("-KILL").arg(pid),
            &base,
            &base.join("kill-out"),
            &base.join("kill-err"),
        );
    }
    assert!(!survived, "the bounded stress run left child {pid} running");
    let left: Vec<String> = std::fs::read_dir(&*base)
        .expect("the lane base")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|entry| entry.starts_with("ae-rust-test."))
        .collect();
    assert!(left.is_empty(), "the stress lane was not reaped: {left:?}");
}

/// nextest 0.9.143 exits 0 when only the LAST stress iteration passed (issue
/// 327), so `just rust-stress` judges the `JUnit` report itself: a fake nextest that
/// exits 0 and leaves a report with a failed, errored, retried-then-passed or
/// missing iteration must fail the lane, and say which iterations, counted from
/// 1 as nextest's log counts them. Status 100 is a failed iteration, status 1 a
/// run that cannot prove every iteration ran.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one flat case table: each row names the report it plants and what the lane must say"
)]
fn a_stress_run_fails_when_the_report_shows_a_failed_or_missing_iteration() {
    let base = super::cli::OwnedScratch::root("gate", "stressverdict");
    let quiet = " 15:25  up 1 day, 6 users, load averages: 0.50 0.50 0.50";
    let args = ["test(=menu::x)", "3", "30"];
    let exit0 = "case \"$1\" in nextest) ;; *) exit 0 ;; esac\n";
    let pass = Suite::pass;
    let broken = |iteration, errors, failures, flaky| Suite {
        errors,
        failures,
        flaky,
        ..Suite::pass(iteration)
    };
    let other = |iteration, failures| Suite {
        binary: "ae",
        failures,
        ..Suite::pass(iteration)
    };
    // (what the report says, its text, the lane's status, what stderr must say)
    let cases: Vec<(&str, String, i32, &str)> = vec![
        (
            "the first two failed and the last passed",
            stress_junit(&[broken(0, 0, 1, 0), broken(1, 0, 2, 0), pass(2)], true),
            100,
            "iteration(s) 1, 2 of 3 failed",
        ),
        (
            "only the last failed",
            stress_junit(&[pass(0), pass(1), broken(2, 0, 1, 0)], true),
            100,
            "iteration(s) 3 of 3 failed",
        ),
        (
            "an execution error without a failure",
            stress_junit(&[pass(0), broken(1, 1, 0, 0), pass(2)], true),
            100,
            "iteration(s) 2 of 3 failed",
        ),
        (
            "a test that passed only on a retry",
            stress_junit(&[broken(0, 0, 0, 1), pass(1), pass(2)], true),
            100,
            "iteration(s) 1 of 3 failed",
        ),
        (
            "a second binary failed beside a passing first one",
            stress_junit(
                &[
                    pass(0),
                    pass(1),
                    pass(2),
                    other(0, 0),
                    other(1, 1),
                    other(2, 0),
                ],
                true,
            ),
            100,
            "iteration(s) 2 of 3 failed",
        ),
        (
            "a failing binary listed before a passing one",
            stress_junit(
                &[
                    other(0, 0),
                    other(1, 1),
                    other(2, 0),
                    pass(0),
                    pass(1),
                    pass(2),
                ],
                true,
            ),
            100,
            "iteration(s) 2 of 3 failed",
        ),
        (
            "a suite row has lost its counts",
            stress_junit(&[pass(0), pass(1), pass(2)], true).replacen(
                " errors=\"0\" failures=\"0\">",
                ">",
                1,
            ),
            1,
            "cut short or unreadable",
        ),
        (
            "an iteration is missing",
            stress_junit(&[pass(0), pass(2)], true),
            1,
            "iteration(s) 2 of 3 did not run",
        ),
        (
            "an iteration ran no tests",
            stress_junit(
                &[
                    pass(0),
                    Suite {
                        tests: 0,
                        ..pass(1)
                    },
                    pass(2),
                ],
                true,
            ),
            1,
            "iteration(s) 2 of 3 did not run",
        ),
        (
            "one failed and one is missing",
            stress_junit(&[broken(0, 0, 1, 0), pass(1)], true),
            100,
            "iteration(s) 1 of 3 failed although nextest exited 0; iteration(s) 3 of 3 did not run",
        ),
        (
            "the report is cut short",
            stress_junit(&[pass(0), pass(1), pass(2)], false),
            1,
            "cut short or unreadable",
        ),
        (
            "the report has no stress rows at all",
            stress_junit(&[], true),
            1,
            "iteration(s) 1, 2, 3 of 3 did not run",
        ),
        (
            "every iteration passed",
            stress_junit(&[pass(0), pass(1), pass(2)], true),
            0,
            "3 of 3 iterations passed",
        ),
    ];
    for (what, report, status, said) in cases {
        let Some((outcome, stderr)) =
            stress_lane_with(&base, quiet, exit0, &[], &args, Some(&report))
        else {
            return;
        };
        assert!(
            matches!(outcome, ExitOutcome::Code(code) if code == status),
            "{what}: want status {status}: {stderr}"
        );
        assert!(stderr.contains(said), "{what}: want {said:?}: {stderr}");
    }
}

/// The verdict reads THIS run's report and nothing else: a report left by an
/// earlier run, a symlink, an old report that cannot be removed and a nextest
/// that leaves nothing all fail the lane; a non-zero status from nextest stands
/// as it is and is named.
#[test]
fn a_stress_run_judges_only_a_fresh_report_and_keeps_nextests_own_status() {
    let base = super::cli::OwnedScratch::root("gate", "stressfresh");
    let quiet = " 15:25  up 1 day, 6 users, load averages: 0.50 0.50 0.50";
    let args = ["test(=menu::x)", "2", "30"];
    let exit0 = "case \"$1\" in nextest) ;; *) exit 0 ;; esac\n";
    let passing = stress_junit(&[Suite::pass(0), Suite::pass(1)], true);
    let old = base.join("cargo-target/nextest/default/junit.xml");
    let seed = |text: &str| {
        let _ = std::fs::remove_dir_all(base.join("cargo-target"));
        std::fs::create_dir_all(old.parent().expect("the report's dir"))
            .unwrap_or_else(|why| panic!("the report's dir: {why}"));
        std::fs::write(&old, text).unwrap_or_else(|why| panic!("the old report: {why}"));
    };

    // A passing report from an earlier run does not stand in for this run's.
    seed(&passing);
    let Some((outcome, stderr)) = stress_lane_with(&base, quiet, exit0, &[], &args, None) else {
        return;
    };
    assert!(
        matches!(outcome, ExitOutcome::Code(1)) && stderr.contains("left no JUnit report"),
        "a stale report must not pass: {stderr}"
    );

    // A symlink is no report, even to a passing one.
    seed(&passing);
    let linked = "case \"$1\" in nextest) ;; *) exit 0 ;; esac\n\
         mkdir -p \"$CARGO_TARGET_DIR/nextest/default\" && \
         ln -s \"$AE_TEST_TMPDIR/linked.xml\" \"$CARGO_TARGET_DIR/nextest/default/junit.xml\"\n";
    std::fs::write(base.join("linked.xml"), &passing)
        .unwrap_or_else(|why| panic!("the linked report: {why}"));
    let Some((outcome, stderr)) = stress_lane_with(&base, quiet, linked, &[], &args, None) else {
        return;
    };
    assert!(
        matches!(outcome, ExitOutcome::Code(1)) && stderr.contains("left no JUnit report"),
        "a symlinked report must not pass: {stderr}"
    );

    // An old report that cannot be removed refuses before nextest starts.
    let _ = std::fs::remove_dir_all(base.join("cargo-target"));
    std::fs::create_dir_all(old.join("kept"))
        .unwrap_or_else(|why| panic!("the unremovable report: {why}"));
    let _ = std::fs::remove_file(base.join("seen"));
    let ran = "case \"$1\" in nextest) echo ran >\"$AE_TEST_TMPDIR/seen\" ;; esac\n";
    let Some((outcome, stderr)) = stress_lane_with(&base, quiet, ran, &[], &args, None) else {
        return;
    };
    assert!(
        matches!(outcome, ExitOutcome::Code(1)) && stderr.contains("cannot be removed"),
        "an old report that stays must refuse: {stderr}"
    );
    assert!(!base.join("seen").exists(), "nextest ran anyway");

    // nextest's own non-zero status is the lane's, whatever the report says.
    let _ = std::fs::remove_dir_all(base.join("cargo-target"));
    let fails = "case \"$1\" in nextest) ;; *) exit 0 ;; esac\nexit 17\n";
    let Some((outcome, stderr)) = stress_lane_with(&base, quiet, fails, &[], &args, Some(&passing))
    else {
        return;
    };
    assert!(
        matches!(outcome, ExitOutcome::Code(17)) && stderr.contains("exited with status 17"),
        "nextest's status stands: {stderr}"
    );
}

/// The niceness this test process runs at: `nice` with no argument prints the
/// niceness it inherited, which is also the lane's.
fn own_niceness(base: &Path) -> i32 {
    raw::run(
        &Invocation::new("nice"),
        base,
        &base.join("nice-out"),
        &base.join("nice-err"),
    )
    .unwrap_or_else(|why| panic!("nice runs: {why}"));
    read(&base.join("nice-out")).trim().parse().unwrap_or(0)
}

/// One run of a heavy recipe (`rust-lint`, `rust-fuzz-check`) on the lane fakes'
/// `PATH` plus a fake `taplo`. The fake `cargo` records one argument per line,
/// then `JOBS=<CARGO_BUILD_JOBS or unset>` and `NICE=<its niceness>`. Returns
/// those lines and the recipe's stderr.
fn sized_recipe(
    base: &Path,
    budget: Option<&str>,
    env: &[(&str, &str)],
    recipe: &str,
) -> (Vec<String>, String) {
    use std::os::unix::fs::PermissionsExt as _;
    let cargo = "{ printf '%s\\n' \"$@\"; echo \"JOBS=${CARGO_BUILD_JOBS-unset}\"; \
                 echo \"NICE=$(ps -o ni= -p $$ | tr -d ' ')\"; } >\"$AE_TEST_TMPDIR/seen\"\n";
    let mut lane = lane_over(base, budget, cargo).arg(recipe);
    let taplo = base.join("bin/taplo");
    std::fs::write(&taplo, "#!/bin/sh\nexit 0\n")
        .and_then(|()| std::fs::set_permissions(&taplo, std::fs::Permissions::from_mode(0o755)))
        .unwrap_or_else(|why| panic!("the fake taplo: {why}"));
    for (key, value) in env {
        lane = lane.env(key, value);
    }
    let _ = std::fs::remove_file(base.join("seen"));
    let ran = raw::run(&lane, &root(), &base.join("out"), &base.join("err"))
        .unwrap_or_else(|why| panic!("{recipe} runs: {why}"));
    let stderr = read(&base.join("err"));
    assert!(
        matches!(ran.outcome(), ExitOutcome::Code(0)),
        "{recipe} must end green: {stderr}"
    );
    let seen = read(&base.join("seen"));
    (seen.lines().map(str::to_owned).collect(), stderr)
}

/// `rust-lint` and `rust-fuzz-check` are heavy cargo runs of the gate, and they
/// size their jobs by the SAME rule as the test lane (the least of the live
/// `jobs-budget` and an inherited `CARGO_BUILD_JOBS`; a relative `-2` is no
/// ceiling; `AE_TEST_JOBS` the override) and run at nice 10. The fuzz check
/// keeps its own cap of four on top.
#[test]
fn lint_and_the_fuzz_check_are_sized_by_the_lane_rule_and_niced() {
    let base = super::cli::OwnedScratch::root("gate", "heavyrecipes");
    let both = |value: &'static str| [("NEXTEST_TEST_THREADS", value), ("CARGO_BUILD_JOBS", value)];
    let wider = [("CARGO_BUILD_JOBS", "16"), ("AE_TEST_JOBS", "9")];
    let none = &[][..];
    let (profile, relative, low) = (both("16"), both("-2"), both("3"));
    let lint = [
        "clippy",
        "--locked",
        "--all-targets",
        "--all-features",
        "--",
        "-D",
        "warnings",
    ];
    // (budget, env, CARGO_BUILD_JOBS the cargo saw, how the recipe says it got there)
    for (budget, env, jobs, said) in [
        (Some("6"), none, "6", "cargo jobs 6 (budget)"),
        (
            Some("6"),
            &profile[..],
            "6",
            "cargo jobs 6 (budget; env 16)",
        ),
        (
            Some("6"),
            &relative[..],
            "6",
            "cargo jobs 6 (budget; env -2 ignored)",
        ),
        (Some("6"), &low[..], "3", "cargo jobs 3 (env; budget 6)"),
        (Some("6"), &wider[..], "9", "cargo jobs 9 (override)"),
        (None, &profile[..], "16", "cargo jobs 16 (env)"),
        (None, &relative[..], "-2", "cargo jobs -2 (env)"),
    ] {
        let (seen, stderr) = sized_recipe(&base, budget, env, "rust-lint");
        assert_eq!(seen[..lint.len()], lint, "{budget:?} {env:?}: {stderr}");
        assert_eq!(
            seen[lint.len()],
            format!("JOBS={jobs}"),
            "{budget:?} {env:?}: {stderr}"
        );
        assert!(stderr.contains(said), "{budget:?} {env:?}: {stderr}");
        let nice: i32 = seen[lint.len() + 1]
            .trim_start_matches("NICE=")
            .parse()
            .expect("cargo's niceness");
        let own = own_niceness(&base);
        assert!(nice > own || nice >= 19, "cargo {nice}, recipe {own}");
    }
    let target = format!("{}/target", root().display());
    let fuzz = |jobs: &str| {
        [
            "check",
            "--locked",
            "--manifest-path",
            "fuzz/Cargo.toml",
            "--bins",
            "--target-dir",
            target.as_str(),
            "--jobs",
            jobs,
        ]
        .map(str::to_owned)
    };
    // (budget, env, the --jobs argument, CARGO_BUILD_JOBS the cargo saw)
    for (budget, env, arg, jobs) in [
        (Some("6"), &profile[..], "4", "6"),
        (Some("2"), none, "2", "2"),
        (Some("6"), &relative[..], "4", "6"),
        (None, &relative[..], "4", "-2"),
        (None, none, "4", "unset"),
    ] {
        let (seen, stderr) = sized_recipe(&base, budget, env, "rust-fuzz-check");
        assert_eq!(seen[..9], fuzz(arg), "{budget:?} {env:?}: {stderr}");
        assert_eq!(
            seen[9],
            format!("JOBS={jobs}"),
            "{budget:?} {env:?}: {stderr}"
        );
    }
}

/// The doctest step of the test lane gets its thread count from the same rule as
/// nextest (rustdoc's libtest otherwise starts one test per core): the least of
/// the budget, sixteen and an inherited export, `AE_TEST_THREADS` the override.
#[test]
fn the_doctest_step_takes_the_lanes_thread_count() {
    let base = super::cli::OwnedScratch::root("gate", "doctests");
    let cargo = "case \"$1\" in test) printf '%s\\n' \"$@\" >\"$AE_TEST_TMPDIR/seen\" ;; esac\n";
    let want = |threads: &str| {
        [
            "test",
            "--doc",
            "--locked",
            "--all-features",
            "--",
            "--test-threads",
            threads,
        ]
        .map(str::to_owned)
    };
    let profile = [("NEXTEST_TEST_THREADS", "32"), ("CARGO_BUILD_JOBS", "32")];
    let lift = [("NEXTEST_TEST_THREADS", "16"), ("AE_TEST_THREADS", "12")];
    for (budget, env, threads) in [
        ("6", &[][..], "6"),
        ("999", &[][..], "16"),
        ("6", &profile[..], "6"),
        ("999", &profile[..], "16"),
        ("6", &lift[..], "12"),
    ] {
        let _ = std::fs::remove_file(base.join("seen"));
        let mut lane = lane_over(&base, Some(budget), cargo)
            .arg("_tmux-isolated")
            .arg("test");
        for (key, value) in env {
            lane = lane.env(key, value);
        }
        let ran = raw::run(&lane, &root(), &base.join("out"), &base.join("err"))
            .unwrap_or_else(|why| panic!("the lane runs: {why}"));
        let stderr = read(&base.join("err"));
        assert!(matches!(ran.outcome(), ExitOutcome::Code(0)), "{stderr}");
        let seen: Vec<String> = read(&base.join("seen"))
            .lines()
            .map(str::to_owned)
            .collect();
        assert_eq!(seen, want(threads), "budget {budget} env {env:?}: {stderr}");
    }
}

/// A shell profile may export `RUSTC_WRAPPER=sccache` for every cargo call, though
/// the repo does not use sccache (`.cargo/config.toml`). Real cargo runs
/// `$RUSTC_WRAPPER rustc -vV` on every invocation that probes or compiles with rustc,
/// with nothing to compile too (measured; `cargo fmt` never does), and
/// the first sccache client starts the one per-user server, which keeps the TMPDIR
/// of that client. A lane run starts it with the lane's own TMPDIR, deleted when the
/// lane ends, and every later cache miss in every session fails with "Failed to
/// create temp dir". So no cargo call of a gate recipe may see an inherited wrapper.
///
/// The fake `cargo` does what real cargo does with each of the two variables that
/// name a wrapper, and the fake wrappers record that they ran. A control run first
/// proves the fakes can see a leak, so a lane that never reaches the wrapper is not
/// read as clean. The wrappers differ by name, so a failure says which variable
/// leaked.
#[test]
fn a_gate_recipe_never_runs_cargo_through_an_inherited_compiler_wrapper() {
    use std::os::unix::fs::PermissionsExt as _;
    let base = super::cli::OwnedScratch::root("gate", "wrapper");
    let cargo = "echo \"$1\" >>\"$AE_TEST_TMPDIR/cargo-ran\"\n\
         [ -z \"${RUSTC_WRAPPER:-}\" ] || \"$RUSTC_WRAPPER\" rustc -vV\n\
         [ -z \"${CARGO_BUILD_RUSTC_WRAPPER:-}\" ] || \"$CARGO_BUILD_RUSTC_WRAPPER\" rustc -vV\n";
    let fakes = |lane_bin: &Path| {
        for name in ["sccache-env", "sccache-build", "taplo"] {
            let body = if name == "taplo" {
                "exit 0\n".to_owned()
            } else {
                format!("echo {name} >>\"$AE_TEST_TMPDIR/wrapper-ran\"\n")
            };
            let file = lane_bin.join(name);
            std::fs::write(&file, format!("#!/bin/sh\n{body}"))
                .and_then(|()| {
                    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))
                })
                .unwrap_or_else(|why| panic!("the fake {name}: {why}"));
        }
    };
    let with_wrappers = |lane: Invocation, lane_bin: &Path| {
        lane.env(
            "RUSTC_WRAPPER",
            lane_bin.join("sccache-env").to_str().unwrap_or_default(),
        )
        .env(
            "CARGO_BUILD_RUSTC_WRAPPER",
            lane_bin.join("sccache-build").to_str().unwrap_or_default(),
        )
    };
    let wrappers_ran = || {
        let mut ran: Vec<String> = std::fs::read_to_string(base.join("wrapper-ran"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect();
        ran.sort();
        ran
    };
    let forget = || {
        let _ = std::fs::remove_file(base.join("wrapper-ran"));
        let _ = std::fs::remove_file(base.join("cargo-ran"));
    };
    // The control: the fake cargo, run bare under both variables, reaches both wrappers.
    let bin = base.join("bin");
    let _ = lane_over(&base, Some("6"), cargo);
    fakes(&bin);
    forget();
    let direct = with_wrappers(
        Invocation::new("sh")
            .env_cleared()
            .arg(bin.join("cargo").to_str().unwrap_or_default())
            .arg("check")
            .env("AE_TEST_TMPDIR", &base),
        &bin,
    );
    let ran = raw::run(&direct, &root(), &base.join("out"), &base.join("err"))
        .unwrap_or_else(|why| panic!("the control runs: {why}"));
    assert!(matches!(ran.outcome(), ExitOutcome::Code(0)));
    assert_eq!(
        wrappers_ran(),
        ["sccache-build", "sccache-env"],
        "the control: the fake cargo must reach both wrappers"
    );
    let mut leaks = Vec::new();
    for recipe in [
        &["rust-lint"][..],
        &["rust-fuzz-check"],
        &["_tmux-isolated", "test"],
        &["_tmux-isolated", "mutants"],
        &["_tmux-isolated", "cov"],
    ] {
        let lane_base = lane_over(&base, Some("6"), cargo);
        fakes(&bin);
        forget();
        let mut lane = with_wrappers(lane_base, &bin);
        for arg in recipe {
            lane = lane.arg(arg);
        }
        let ran = raw::run(&lane, &root(), &base.join("out"), &base.join("err"))
            .unwrap_or_else(|why| panic!("{recipe:?} runs: {why}"));
        let stderr = read(&base.join("err"));
        assert!(
            matches!(ran.outcome(), ExitOutcome::Code(0)),
            "{recipe:?} must end green: {stderr}"
        );
        assert!(
            !read(&base.join("cargo-ran")).is_empty(),
            "{recipe:?} ran no cargo, so the pin saw nothing: {stderr}"
        );
        let ran = wrappers_ran();
        if !ran.is_empty() {
            leaks.push(format!("{recipe:?} reached {ran:?}"));
        }
    }
    assert!(
        leaks.is_empty(),
        "a gate recipe ran cargo through a compiler wrapper from the environment: {leaks:?}"
    );
}

/// The statement that keeps cargo off a compiler wrapper. EMPTY, not unset: cargo
/// (measured, 1.97.1) reads an empty `RUSTC_WRAPPER` as "no wrapper" even over a
/// `build.rustc-wrapper` value in `~/.cargo/config.toml`, which `unset` leaves in force.
const EMPTY_WRAPPER: &str = "export RUSTC_WRAPPER= CARGO_BUILD_RUSTC_WRAPPER=";

/// Whether the statement exists and comes before the first line that is `before`.
fn empties_the_wrapper_before(lines: &[String], before: &str) -> bool {
    let clear = lines.iter().position(|line| line == EMPTY_WRAPPER);
    let first = lines.iter().position(|line| line == before);
    matches!((clear, first), (Some(clear), Some(first)) if clear < first)
}

/// Whether `rust-check` empties both wrapper variables before the receipt it writes
/// after the four dependencies (the receipt runs `cargo nextest`), and the `SIZING`
/// helper empties them as its first statement, so every recipe that includes it does
/// so before its first cargo call. The behavioural pin runs the lane, `rust-lint` and
/// `rust-fuzz-check` with the two variables set, and a fake cargo cannot tell empty
/// from unset on a config value, so the form is read here. The `rust-check` body cannot
/// run in a test (it writes into the checkout's `.local`), so its text is read too.
fn the_wrapper_is_emptied_where_cargo_starts(justfile: &str) -> bool {
    let helper: Vec<String> = justfile_variable(justfile, "SIZING")
        .lines()
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    helper.first().is_some_and(|line| line == EMPTY_WRAPPER)
        && empties_the_wrapper_before(&recipe_text(justfile, "rust-check:"), "{{ RECEIPT }}")
}

#[test]
fn the_gate_empties_the_compiler_wrapper_variables_where_its_cargo_calls_start() {
    assert!(
        the_wrapper_is_emptied_where_cargo_starts(&read(&root().join("justfile"))),
        "the SIZING helper (first statement) and rust-check (before {{{{ RECEIPT }}}}) must run \
         `{EMPTY_WRAPPER}`"
    );
    let good = format!(
        "SIZING := '''\n    {EMPTY_WRAPPER}\n    lane_threads=16\n'''\n\n\
         rust-check: rust-fmt-check\n    #!/usr/bin/env bash\n    set -euo pipefail\n    \
         {EMPTY_WRAPPER}\n    {{{{ RECEIPT }}}}\n"
    );
    assert!(the_wrapper_is_emptied_where_cargo_starts(&good));
    let red = |from: &str, to: &str| {
        !the_wrapper_is_emptied_where_cargo_starts(&good.replacen(from, to, 1))
    };
    // RED — the helper's statement is gone, or no longer first.
    assert!(red(
        &format!("    {EMPTY_WRAPPER}\n    lane_threads"),
        "    lane_threads"
    ));
    assert!(red(
        &format!("    {EMPTY_WRAPPER}\n    lane_threads=16\n"),
        &format!("    lane_threads=16\n    {EMPTY_WRAPPER}\n")
    ));
    // RED — rust-check's statement is gone, or only after the receipt's cargo calls.
    assert!(red(
        &format!("    {EMPTY_WRAPPER}\n    {{{{ RECEIPT }}}}"),
        "    {{ RECEIPT }}"
    ));
    assert!(red(
        &format!("    {EMPTY_WRAPPER}\n    {{{{ RECEIPT }}}}\n"),
        &format!("    {{{{ RECEIPT }}}}\n    {EMPTY_WRAPPER}\n")
    ));
    // RED — only one of the two variables, or unset in place of empty (a
    // `build.rustc-wrapper` config value then stays in force).
    for statement in [
        "export RUSTC_WRAPPER=",
        "export CARGO_BUILD_RUSTC_WRAPPER=",
        "unset RUSTC_WRAPPER CARGO_BUILD_RUSTC_WRAPPER",
    ] {
        assert!(!the_wrapper_is_emptied_where_cargo_starts(
            &good.replace(EMPTY_WRAPPER, statement)
        ));
        assert!(red(
            &format!("    {EMPTY_WRAPPER}\n    lane_threads"),
            &format!("    {statement}\n    lane_threads")
        ));
        assert!(red(
            &format!("    {EMPTY_WRAPPER}\n    {{{{ RECEIPT }}}}"),
            &format!("    {statement}\n    {{{{ RECEIPT }}}}")
        ));
    }
}

/// Whether the test lane, `rust-lint` and `rust-fuzz-check` size themselves from
/// the ONE helper (`SIZING`): each includes it and calls `size_counts`, none
/// carries a `jobs-budget` call of its own, the helper alone owns the call and
/// the two exports, and the two cargo recipes run cargo at nice 10.
fn sizing_shared_ok(justfile: &str) -> bool {
    let helper = justfile_variable(justfile, "SIZING");
    let owns = [
        "jobs-budget",
        "export NEXTEST_TEST_THREADS=\"$threads\"",
        "export CARGO_BUILD_JOBS=\"$jobs\"",
        "size_counts()",
        "size_say()",
    ];
    if owns.iter().any(|needle| !helper.contains(needle)) {
        return false;
    }
    [
        ("_tmux-isolated lane *args:", ""),
        ("rust-lint:", "nice -n 10 cargo clippy "),
        ("rust-fuzz-check:", "nice -n 10 cargo check "),
    ]
    .iter()
    .all(|(header, cargo)| {
        let lines = recipe_text(justfile, header);
        lines.iter().any(|line| line == "{{ SIZING }}")
            && lines.iter().any(|line| line.starts_with("size_counts"))
            && !lines.join("\n").contains("jobs-budget")
            && lines.join("\n").contains(cargo)
    })
}

#[test]
fn one_sizing_helper_serves_the_lane_lint_and_the_fuzz_check() {
    assert!(
        sizing_shared_ok(&read(&root().join("justfile"))),
        "the test lane, rust-lint and rust-fuzz-check must size themselves from the ONE \
         SIZING helper, with no jobs-budget call of their own"
    );
    let good = "SIZING := '''\n    size_counts() { jobs-budget; export NEXTEST_TEST_THREADS=\"$threads\"; export CARGO_BUILD_JOBS=\"$jobs\"; }\n    size_say() { :; }\n'''\n\n\
        rust-lint:\n    #!/usr/bin/env bash\n    {{ SIZING }}\n    size_counts\n    nice -n 10 cargo clippy --locked\n\n\
        rust-fuzz-check:\n    #!/usr/bin/env bash\n    {{ SIZING }}\n    size_counts\n    nice -n 10 cargo check --locked\n\n\
        _tmux-isolated lane *args:\n    #!/usr/bin/env bash\n    {{ SIZING }}\n    size_counts\n    detached nice -n 10 cargo nextest run\n";
    assert!(sizing_shared_ok(good));
    // RED — a recipe that sizes itself by hand instead of including the helper.
    let lint = "rust-lint:\n    #!/usr/bin/env bash\n    {{ SIZING }}\n    size_counts\n";
    assert!(!sizing_shared_ok(&good.replacen(
        lint,
        "rust-lint:\n    #!/usr/bin/env bash\n    nice -n 10 cargo clippy --locked\n",
        1
    )));
    // RED — a copy of the arithmetic: a second jobs-budget call in the fuzz check.
    assert!(!sizing_shared_ok(&good.replacen(
        "nice -n 10 cargo check",
        "jobs=\"$(jobs-budget)\"\n    nice -n 10 cargo check",
        1
    )));
    // RED — included but never called.
    assert!(!sizing_shared_ok(&good.replacen(
        "    size_counts\n    detached",
        "    detached",
        1
    )));
    // RED — a heavy cargo run left at the caller's priority.
    assert!(!sizing_shared_ok(
        &good.replace("nice -n 10 cargo clippy", "cargo clippy")
    ));
    // RED — the helper without its export, or without a variable at all.
    assert!(!sizing_shared_ok(
        &good.replace("export CARGO_BUILD_JOBS=\"$jobs\"", "")
    ));
    assert!(!sizing_shared_ok("rust-lint:\n    cargo clippy\n"));
}

/// Whether the mutants arm of the isolated lane starts the memory watcher
/// BEFORE cargo-mutants, the watcher's ceiling defaults to 2048 MB, it forgets
/// every pid it killed after one cycle (a recycled pid must stay bounded) and it
/// re-reads a process by pid before the signal.
fn mutants_arm_watches_memory(justfile: &str) -> bool {
    let lines = recipe_text(justfile, "_tmux-isolated lane *args:");
    let arm: Vec<&str> = lines
        .iter()
        .map(String::as_str)
        .skip_while(|line| *line != "mutants)")
        .skip(1)
        .take_while(|line| *line != ";;")
        .collect();
    let watch = arm.iter().position(|line| *line == "rss_watch &");
    let mutants = arm.iter().position(|line| line.contains("cargo mutants"));
    let has = |text: &str| lines.iter().any(|line| line.contains(text));
    has("|| ceiling=2048")
        && watch.is_some()
        && watch < mutants
        && lines.iter().any(|line| line == "killed=\"$now\"")
        && !has("killed=\"$killed")
        && has("ps -wwo args= -p \"$pid\"")
}

#[test]
fn the_mutants_arm_starts_the_memory_watcher_with_a_2048_mb_default() {
    assert!(mutants_arm_watches_memory(&read(&root().join("justfile"))));
    let recipe = |ceiling: &str, arm: &str| {
        format!(
            "_tmux-isolated lane *args:\n    #!/usr/bin/env bash\n    rss_watch() {{\n        \
             {ceiling}\n        [[ \"$(ps -wwo args= -p \"$pid\")\" == \"$args\" ]]\n        killed=\"$now\"\n    }}\n    case \"$lane\" in\n        mutants)\n{arm}            ;;\n    esac\n"
        )
    };
    let default = "[[ \"$ceiling\" =~ ^[1-9][0-9]{0,5}$ ]] || ceiling=2048";
    let arm = "            rss_watch &\n            detached nice -n 10 cargo mutants \"$@\"\n";
    assert!(mutants_arm_watches_memory(&recipe(default, arm)));
    // RED — no watcher, a watcher started after the run it must bound, a watcher
    // in another arm only, a different default.
    let bare = "            detached nice -n 10 cargo mutants \"$@\"\n";
    assert!(!mutants_arm_watches_memory(&recipe(default, bare)));
    let late = "            detached cargo mutants \"$@\"\n            rss_watch &\n";
    assert!(!mutants_arm_watches_memory(&recipe(default, late)));
    let other = format!("{bare}            ;;\n        cov)\n{arm}");
    assert!(!mutants_arm_watches_memory(&recipe(default, &other)));
    let wide = default.replace("2048", "8192");
    assert!(!mutants_arm_watches_memory(&recipe(&wide, arm)));
    // RED — a lifetime pid list (a recycled pid would be exempt for the whole
    // run), a pid signalled without being re-read first.
    let good = recipe(default, arm);
    let lifetime = good.replace("killed=\"$now\"", "killed=\"$killed$pid \"");
    assert!(!mutants_arm_watches_memory(&lifetime));
    let unread = good.replace("ps -wwo args= -p", "ps -o comm= -p");
    assert!(!mutants_arm_watches_memory(&unread));
}

/// The mutants lane's memory watcher kills exactly one kind of process: a test
/// binary (first argv word under `target/*/deps/`) below the lane's own
/// `cargo-mutants` that holds more than the ceiling, and it leaves a receipt.
/// A same-size process outside `deps/`, a small test binary and an over-size
/// test binary outside the cargo-mutants tree all live; an unusable ceiling
/// falls back to the default, under which a 60 MB process lives. The "tools"
/// are symlinks to bash, so `ps` shows them by the path they were started as.
#[test]
fn the_mutants_watcher_kills_only_an_over_ceiling_test_binary_below_its_cargo_mutants() {
    use std::os::unix::fs::symlink;
    let base = super::cli::OwnedScratch::root("gate", "rsskill");
    let (watch, work) = (base.join("watch"), base.join("work"));
    for dir in ["target/debug/deps", "other"] {
        std::fs::create_dir_all(watch.join(dir)).unwrap_or_else(|why| panic!("{dir}: {why}"));
    }
    std::fs::create_dir_all(&work).unwrap_or_else(|why| panic!("work: {why}"));
    for name in [
        "cargo-mutants",
        "target/debug/deps/ae-hog",
        "target/debug/deps/ae-small",
        "target/debug/deps/ae-orphan",
        "other/rustc-hog",
    ] {
        let _ = std::fs::remove_file(watch.join(name));
        symlink("/bin/bash", watch.join(name)).unwrap_or_else(|why| panic!("{name}: {why}"));
    }
    let script = |name: &str, body: &str| {
        std::fs::write(watch.join(name), body).unwrap_or_else(|why| panic!("{name}: {why}"));
    };
    // `kid.sh <pidfile> <bytes>`: publish the pid, hold that much memory in this
    // very process (no fork the watcher could mistake for the holder), say so in
    // `<pidfile>.ready`, then stay until the release file appears (or a minute
    // passes, so a harness that died leaves nothing behind).
    script(
        "kid.sh",
        "echo $$ >\"$1.tmp\" && mv \"$1.tmp\" \"$1\"\nprintf -v x '%*s' \"$2\" ''\n: >\"$1.ready\"\n\
         i=0\nwhile [ ! -e \"$AE_FAKE_W/release\" ] && [ \"$i\" -lt 600 ]; do sleep 0.1; i=$((i + 1)); done\n",
    );
    script(
        "root.sh",
        "\"$AE_FAKE_W/target/debug/deps/ae-hog\" \"$AE_FAKE_W/kid.sh\" \"$AE_FAKE_W/hog.pid\" 60000000 &\n\
         \"$AE_FAKE_W/target/debug/deps/ae-small\" \"$AE_FAKE_W/kid.sh\" \"$AE_FAKE_W/small.pid\" 1000 &\n\
         \"$AE_FAKE_W/other/rustc-hog\" \"$AE_FAKE_W/kid.sh\" \"$AE_FAKE_W/rustc.pid\" 60000000 &\n\
         wait\n",
    );
    // The fake cargo waits for the three processes that must SURVIVE to say they
    // hold their memory, then (when a kill is expected) for the receipt, then a
    // few watcher polls more, and only then reads who is alive. Every wait is
    // bounded and the EXIT trap releases and kills whatever is left.
    let cargo = "case \"$1\" in mutants) ;; *) exit 0 ;; esac\n\
         W=$AE_FAKE_W\n\
         trap 'touch \"$W/release\"; kill $(cat \"$W\"/*.pid 2>/dev/null) 2>/dev/null' EXIT\n\
         \"$W/target/debug/deps/ae-orphan\" \"$W/kid.sh\" \"$W/orphan.pid\" 60000000 &\n\
         \"$W/cargo-mutants\" \"$W/root.sh\" &\n\
         i=0\n\
         until [ -e \"$W/small.pid.ready\" ] && [ -e \"$W/rustc.pid.ready\" ] && [ -e \"$W/orphan.pid.ready\" ]; do\n\
           i=$((i + 1)); [ \"$i\" -lt 300 ] || exit 3; sleep 0.2\n\
         done\n\
         if [ \"$AE_FAKE_KILLED\" = 1 ]; then\n\
           i=0\n\
           while [ ! -s mutants.out/ae-rss-kills.log ]; do i=$((i + 1)); [ \"$i\" -lt 300 ] || exit 4; sleep 0.2; done\n\
         fi\n\
         sleep 3\n\
         for k in hog small rustc orphan; do\n\
           p=$(cat \"$W/$k.pid\" 2>/dev/null)\n\
           if [ -n \"$p\" ] && kill -0 \"$p\" 2>/dev/null; then echo \"$k alive\"; else echo \"$k dead\"; fi\n\
         done >\"$AE_TEST_TMPDIR/seen.tmp\"\n\
         mv \"$AE_TEST_TMPDIR/seen.tmp\" \"$AE_TEST_TMPDIR/seen\"\n\
         exit 0\n";
    // (the ceiling as typed, whether the hog is killed, who is alive after)
    for (ceiling, killed, seen) in [
        (
            "32",
            true,
            "hog dead\nsmall alive\nrustc alive\norphan alive\n",
        ),
        (
            "lots",
            false,
            "hog alive\nsmall alive\nrustc alive\norphan alive\n",
        ),
    ] {
        for stale in ["hog", "small", "rustc", "orphan"] {
            for ext in ["pid", "pid.ready"] {
                let _ = std::fs::remove_file(watch.join(format!("{stale}.{ext}")));
            }
        }
        let _ = std::fs::remove_file(watch.join("release"));
        let _ = std::fs::remove_dir_all(work.join("mutants.out"));
        let _ = std::fs::remove_file(base.join("seen"));
        let lane = lane_over(&base, Some("6"), cargo)
            .env("AE_FAKE_W", &watch)
            .env("AE_MUTANTS_RSS_MAX_MB", ceiling)
            .env("AE_FAKE_KILLED", if killed { "1" } else { "0" })
            .arg("--working-directory")
            .arg(&work)
            .arg("--justfile")
            .arg(root().join("justfile"))
            .arg("_tmux-isolated")
            .arg("mutants");
        let ran = raw::run(&lane, &work, &base.join("out"), &base.join("err"))
            .unwrap_or_else(|why| panic!("the lane runs: {why}"));
        let stderr = read(&base.join("err"));
        assert!(
            matches!(ran.outcome(), ExitOutcome::Code(0)),
            "{ceiling}: {stderr}"
        );
        assert_eq!(
            std::fs::read_to_string(base.join("seen")).unwrap_or_default(),
            seen,
            "{ceiling}: {stderr}"
        );
        let receipt = std::fs::read_to_string(work.join("mutants.out/ae-rss-kills.log"));
        if killed {
            let receipt = receipt.unwrap_or_default();
            // A fork of the hog (its `$(...)` subshell) is the same test binary,
            // so one or more lines, every one of them the hog.
            assert!(!receipt.is_empty(), "no receipt: {stderr}");
            for line in receipt.lines() {
                let fields: Vec<&str> = line.split_whitespace().collect();
                assert!(
                    fields.get(2).and_then(|mb| mb.parse::<u32>().ok()) >= Some(32),
                    "{line:?}"
                );
                assert!(
                    fields
                        .get(3)
                        .is_some_and(|binary| binary.ends_with("/target/debug/deps/ae-hog")),
                    "{line:?}"
                );
            }
            assert!(
                stderr.contains("lane mutants: killed over the 32 MB ceiling"),
                "{stderr}"
            );
        } else {
            assert!(receipt.is_err(), "{receipt:?}");
            assert!(!stderr.contains("killed over"), "{stderr}");
        }
    }
}

/// A `jobs-budget` that is missing or answers anything but one positive integer
/// of at most three digits is no answer: the CPU count sizes the threads, cargo
/// keeps its own jobs. A 20-digit answer used to wrap the lane's arithmetic to
/// zero threads (2^64) or a negative number.
#[test]
fn an_unusable_jobs_budget_answer_leaves_the_lane_sized_as_before() {
    let base = super::cli::OwnedScratch::root("gate", "unusable");
    let answers = ["0", "many", "", "6\n6", "1000", "18446744073709551616"];
    for answer in answers.map(Some).into_iter().chain([None]) {
        let (seen, stderr) = sized_lane(&base, answer, &[], &["test"]);
        let threads: usize = seen[1].parse().expect("the recorded threads");
        assert!((1..=16).contains(&threads), "{answer:?}: {seen:?}");
        assert_eq!(seen[2], "unset", "{answer:?}: {seen:?}");
        let said = format!("threads {threads} (cpus), cargo jobs unset (cargo default)");
        assert!(stderr.contains(&said), "{answer:?}: {stderr}");
    }
}

/// The full test, coverage and mutants runs start cargo at `nice -n 10`; a
/// filtered `just test <filter>` does not.
#[test]
fn the_heavy_lanes_run_cargo_niced_and_a_filtered_run_does_not() {
    let base = super::cli::OwnedScratch::root("gate", "nice");
    for args in [
        &["test"][..],
        &["cov"][..],
        &["mutants"][..],
        &["test", "filter"][..],
    ] {
        let (seen, _) = sized_lane(&base, Some("6"), &[], args);
        let (own, lane): (i32, i32) = (
            seen[3].parse().expect("niceness"),
            seen[4].parse().expect("niceness"),
        );
        // A lane already at the ceiling cannot be pushed further.
        let ok = if args.len() == 1 {
            own > lane || lane >= 19
        } else {
            own == lane
        };
        assert!(ok, "{args:?}: cargo {own}, lane {lane}");
    }
}

/// #222: the fake `cargo` for the Cargo-root pins. For a nextest, doctest,
/// llvm-cov or mutants call it resolves the final root (`CARGO_TARGET_DIR`,
/// else `CARGO_BUILD_TARGET_DIR`, else `$PWD/target`) and the intermediate one
/// (`CARGO_BUILD_BUILD_DIR`, else the final one), notes whether a marker of an
/// earlier call was already there, leaves a marker of its own in both, and then
/// records one block of `key=value` lines ending `end`: the tool, every
/// argument, each of the three selectors as it saw it (or `<name> unset`), the
/// lane's `TMUX_TMPDIR`, its cwd and the two roots. The model is only where the
/// markers go; no assertion leans on which selector Cargo ranks first.
const ROOTS_CARGO: &str = r#"case "$1" in nextest|test|llvm-cov|mutants) ;; *) exit 0 ;; esac
t="${CARGO_TARGET_DIR:-${CARGO_BUILD_TARGET_DIR:-$PWD/target}}"
b="${CARGO_BUILD_BUILD_DIR:-$t}"
stale=no
for f in "$t"/ae-mark.* "$b"/ae-mark.*; do [ -e "$f" ] && stale=yes; done
mkdir -p "$t" "$b" && : >"$t/ae-mark.$1.$$" && : >"$b/ae-mark.$1.$$" || exit 97
{
  echo "tool=$1"
  for a in "$@"; do echo "arg=$a"; done
  if [ -n "${CARGO_TARGET_DIR+x}" ]; then echo "CARGO_TARGET_DIR=$CARGO_TARGET_DIR"; else echo "CARGO_TARGET_DIR unset"; fi
  if [ -n "${CARGO_BUILD_TARGET_DIR+x}" ]; then echo "CARGO_BUILD_TARGET_DIR=$CARGO_BUILD_TARGET_DIR"; else echo "CARGO_BUILD_TARGET_DIR unset"; fi
  if [ -n "${CARGO_BUILD_BUILD_DIR+x}" ]; then echo "CARGO_BUILD_BUILD_DIR=$CARGO_BUILD_BUILD_DIR"; else echo "CARGO_BUILD_BUILD_DIR unset"; fi
  echo "TMUX_TMPDIR=$TMUX_TMPDIR"
  echo "pwd=$PWD"
  echo "target=$t"
  echo "build=$b"
  echo "stale=$stale"
  echo end
} >>"$AE_FAKE_REC"
exit "${AE_FAKE_EXIT:-0}"
"#;

/// The three Cargo output selectors a caller can export, in the order the
/// #222 pins name them.
const CARGO_ROOT_VARS: [&str; 3] = [
    "CARGO_TARGET_DIR",
    "CARGO_BUILD_TARGET_DIR",
    "CARGO_BUILD_BUILD_DIR",
];

/// One fake `cargo` call [`ROOTS_CARGO`] recorded: its lines, `end` dropped.
struct Call(Vec<String>);

impl Call {
    /// The value of `key=value`; `None` for `<key> unset` and for no such line.
    fn field(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
    }

    /// Every argument the fake was handed, in order.
    fn args(&self) -> Vec<&str> {
        self.0
            .iter()
            .filter_map(|line| line.strip_prefix("arg="))
            .collect()
    }
}

/// Every call `record` holds, oldest first; an absent record holds none.
fn calls(record: &Path) -> Vec<Call> {
    let text = std::fs::read_to_string(record).unwrap_or_default();
    let mut out = Vec::new();
    let mut lines = Vec::new();
    for line in text.lines() {
        if line == "end" {
            out.push(Call(std::mem::take(&mut lines)));
        } else {
            lines.push(line.to_owned());
        }
    }
    out
}

/// The caller's three output selectors for the #222 pins, each an existing
/// directory under `base` with a space in its name and one `caller-sentinel`
/// file in it: `(variable, directory)`.
fn caller_roots(base: &Path) -> [(&'static str, String); 3] {
    [
        ("CARGO_TARGET_DIR", "caller t"),
        ("CARGO_BUILD_TARGET_DIR", "caller bt"),
        ("CARGO_BUILD_BUILD_DIR", "caller b"),
    ]
    .map(|(var, name)| {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join("caller-sentinel"), ""))
            .unwrap_or_else(|why| panic!("the caller's {name}: {why}"));
        let text = dir
            .to_str()
            .unwrap_or_else(|| panic!("a UTF-8 scratch path: {}", dir.display()))
            .to_owned();
        (var, text)
    })
}

/// One `_tmux-isolated` run for the #222 pins: from `<base>/work`, over
/// `lane_base` as the lane's `AE_TEST_TMPDIR`, with `cargo` as the fake body,
/// the caller's `env` and the lane's `args`. The fake records into
/// `<base>/record`. Returns the outcome, the recorded calls and the stderr.
fn rooted_lane(
    base: &Path,
    lane_base: &Path,
    cargo: &str,
    env: &[(&str, String)],
    args: &[&str],
) -> (ExitOutcome, Vec<Call>, String) {
    let mut words = vec!["_tmux-isolated"];
    words.extend_from_slice(args);
    rooted_just(base, lane_base, cargo, env, &words)
}

/// `rooted_lane` for any recipe: `recipe_args` is the recipe name and then the
/// words `just` is given, so the PUBLIC `rust-mutants` runs end to end.
fn rooted_just(
    base: &Path,
    lane_base: &Path,
    cargo: &str,
    env: &[(&str, String)],
    recipe_args: &[&str],
) -> (ExitOutcome, Vec<Call>, String) {
    let work = base.join("work");
    for dir in [work.as_path(), lane_base] {
        std::fs::create_dir_all(dir).unwrap_or_else(|why| panic!("{}: {why}", dir.display()));
    }
    let record = base.join("record");
    let _ = std::fs::remove_file(&record);
    let mut lane = lane_over(base, Some("6"), cargo)
        .env("AE_TEST_TMPDIR", lane_base)
        .env("TMPDIR", lane_base)
        .env("AE_FAKE_REC", &record)
        .arg("--working-directory")
        .arg(&work)
        .arg("--justfile")
        .arg(root().join("justfile"));
    for arg in recipe_args {
        lane = lane.arg(arg);
    }
    for (key, value) in env {
        lane = lane.env(key, value);
    }
    let ran = raw::run(&lane, &work, &base.join("out"), &base.join("err"))
        .unwrap_or_else(|why| panic!("the lane runs: {why}"));
    (ran.outcome(), calls(&record), read(&base.join("err")))
}

/// The lane directory `call` ran in — the `TMUX_TMPDIR` the lane exports as
/// its own `$test_tmux_tmp` — proven one `ae-rust-test.*` directory directly
/// under `lane_base`.
fn lane_root<'c>(call: &'c Call, lane_base: &Path, context: &str) -> &'c str {
    let dir = call
        .field("TMUX_TMPDIR")
        .unwrap_or_else(|| panic!("{context}: no TMUX_TMPDIR recorded"));
    let path = Path::new(dir);
    assert!(
        path.parent() == Some(lane_base)
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("ae-rust-test.")),
        "{context}: {dir:?} is not a lane directory under {}",
        lane_base.display()
    );
    dir
}

/// `dir` is an absolute path strictly beneath `lane`, with no `..` in it.
fn assert_beneath(dir: &str, lane: &str, context: &str) {
    let path = Path::new(dir);
    assert!(
        path.is_absolute()
            && path.starts_with(lane)
            && path != Path::new(lane)
            && path
                .components()
                .all(|part| part != std::path::Component::ParentDir),
        "{context}: {dir:?} is not beneath the lane's own {lane:?}"
    );
}

/// Whether the mutants lane's memory watcher would take `path` for a test
/// binary: its awk pattern `/target/.*/deps/[^/]+$`.
fn watcher_sees(path: &str) -> bool {
    path.rsplit_once('/').is_some_and(|(head, name)| {
        !name.is_empty()
            && head
                .strip_suffix("/deps")
                .is_some_and(|above| above.contains("/target/"))
    })
}

/// The `ae-rust-test.*` lane directories left under `lane_base`.
fn lanes_left(lane_base: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(lane_base) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("ae-rust-test."))
        .collect()
}

/// Each caller directory still holds its `caller-sentinel` and nothing else.
fn assert_untouched(callers: &[(&str, String)], context: &str) {
    for (var, dir) in callers {
        let names: Vec<String> = std::fs::read_dir(dir)
            .unwrap_or_else(|why| panic!("{context}: the caller's {var}: {why}"))
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            ["caller-sentinel"],
            "{context}: the caller's {var} was written"
        );
    }
}

/// #222: a mutation run builds its final AND its intermediate artifacts in
/// Cargo roots the lane owns, whichever output selectors the caller exported or
/// left unset — an inherited file config is covered by the same exports,
/// because Cargo ranks the environment above configuration files. Both
/// target-dir spellings name ONE lane root, so no ranking between them is
/// assumed. Every root lies beneath the lane's own directory, which its cleanup
/// removes, keeps the `/target/…/deps/` shape the memory watcher reads, and no
/// caller directory is written.
#[test]
fn a_mutants_lane_builds_in_cargo_roots_it_owns_whatever_the_caller_exported() {
    let scratch = super::cli::OwnedScratch::root("gate", "mroot");
    let base = scratch.path();
    let lane_base = base.join("l b");
    let callers = caller_roots(base);
    let none: &[(&str, String)] = &[];
    for (case, env) in [
        ("all three exported", &callers[..]),
        ("the intermediate root alone", &callers[2..]),
        ("the alternate target spelling alone", &callers[1..2]),
        ("none exported", none),
    ] {
        let (outcome, calls, stderr) =
            rooted_lane(base, &lane_base, ROOTS_CARGO, env, &["mutants"]);
        assert!(matches!(outcome, ExitOutcome::Code(0)), "{case}: {stderr}");
        let [call] = &calls[..] else {
            panic!(
                "{case}: one cargo call expected, {} recorded: {stderr}",
                calls.len()
            );
        };
        let lane = lane_root(call, &lane_base, case);
        for var in CARGO_ROOT_VARS {
            let dir = call.field(var).unwrap_or_else(|| {
                panic!("{case}: cargo mutants ran with no {var} of the lane's: {stderr}")
            });
            assert_beneath(dir, lane, &format!("{case}: {var}"));
        }
        assert_eq!(
            call.field("CARGO_TARGET_DIR"),
            call.field("CARGO_BUILD_TARGET_DIR"),
            "{case}: the two target-dir spellings name different roots"
        );
        let build = call.field("build").unwrap_or_default();
        assert!(
            watcher_sees(&format!("{build}/debug/deps/ae-0123abcd")),
            "{case}: the memory watcher cannot see a test binary under {build:?}"
        );
        assert_eq!(
            call.field("stale"),
            Some("no"),
            "{case}: a marker was already there"
        );
        for key in ["target", "build"] {
            let dir = call.field(key).unwrap_or_default();
            assert!(
                !Path::new(dir).exists(),
                "{case}: the lane left its {key} root {dir:?}"
            );
        }
        assert!(lanes_left(&lane_base).is_empty(), "{case}: {stderr}");
        assert_untouched(&callers, case);
        assert!(
            !base.join("work/target").exists(),
            "{case}: the caller's own target/ was written"
        );
    }
}

/// #222 control: only the mutation run gets roots of its own. The full test
/// run, a filtered one and the coverage run hand Cargo exactly the output
/// selectors the caller exported — spaces kept — and invent none when the
/// caller exported none.
#[test]
fn the_test_and_cov_lanes_keep_the_callers_cargo_roots() {
    let scratch = super::cli::OwnedScratch::root("gate", "oroot");
    let base = scratch.path();
    let lane_base = base.join("l b");
    let callers = caller_roots(base);
    let none: &[(&str, String)] = &[];
    for (args, tools) in [
        (&["test"][..], &["nextest", "test"][..]),
        (&["test", "filter"][..], &["nextest"][..]),
        (&["cov"][..], &["llvm-cov"][..]),
    ] {
        for env in [&callers[..], none] {
            let (outcome, calls, stderr) = rooted_lane(base, &lane_base, ROOTS_CARGO, env, args);
            assert!(
                matches!(outcome, ExitOutcome::Code(0)),
                "{args:?}: {stderr}"
            );
            let ran: Vec<&str> = calls
                .iter()
                .map(|call| call.field("tool").unwrap_or_default())
                .collect();
            assert_eq!(ran, tools, "{args:?}: {stderr}");
            for call in &calls {
                for var in CARGO_ROOT_VARS {
                    let exported = env
                        .iter()
                        .find(|(name, _)| *name == var)
                        .map(|(_, dir)| dir.as_str());
                    assert_eq!(call.field(var), exported, "{args:?} {var}: {stderr}");
                }
            }
            assert!(lanes_left(&lane_base).is_empty(), "{args:?}: {stderr}");
        }
    }
}

/// #222: two mutation runs from one checkout never share a Cargo root, so the
/// second can neither build on nor run what the first left.
#[test]
fn two_mutants_lanes_never_share_or_reuse_a_cargo_root() {
    let scratch = super::cli::OwnedScratch::root("gate", "mtwice");
    let base = scratch.path();
    let lane_base = base.join("l b");
    let mut seen: Vec<(String, Vec<String>)> = Vec::new();
    for attempt in ["first", "second"] {
        let (outcome, calls, stderr) =
            rooted_lane(base, &lane_base, ROOTS_CARGO, &[], &["mutants"]);
        assert!(
            matches!(outcome, ExitOutcome::Code(0)),
            "{attempt}: {stderr}"
        );
        let [call] = &calls[..] else {
            panic!(
                "{attempt}: one cargo call expected, {} recorded: {stderr}",
                calls.len()
            );
        };
        let lane = lane_root(call, &lane_base, attempt).to_owned();
        let mut roots = Vec::new();
        for var in CARGO_ROOT_VARS {
            let dir = call
                .field(var)
                .unwrap_or_else(|| panic!("{attempt}: no {var} of the lane's: {stderr}"));
            assert_beneath(dir, &lane, &format!("{attempt}: {var}"));
            roots.push(dir.to_owned());
        }
        assert_eq!(
            call.field("stale"),
            Some("no"),
            "{attempt}: a marker was already there"
        );
        seen.push((lane, roots));
    }
    assert_ne!(seen[0].0, seen[1].0, "both runs used one lane directory");
    for dir in &seen[1].1 {
        assert!(!seen[0].1.contains(dir), "the second run reused {dir:?}");
    }
}

/// #222 control: the INNER `_tmux-isolated` mutants arm still hands
/// cargo-mutants its own pinned flags and then every argument it received —
/// spaces kept, nothing added or dropped — runs it from the caller's directory,
/// ends with its status and still removes the lane. This pins that inner argv
/// only; the public `rust-mutants` recipe that feeds it is pinned by
/// `rust_mutants_forwards_each_argument_as_one_word` (#221).
#[test]
fn a_mutants_lane_passes_its_arguments_through_and_ends_as_cargo_mutants_did() {
    let scratch = super::cli::OwnedScratch::root("gate", "margs");
    let base = scratch.path();
    let lane_base = base.join("l b");
    let mut env = caller_roots(base).to_vec();
    env.push(("AE_FAKE_EXIT", "7".to_owned()));
    let args = ["mutants", "--in-diff", "a b.diff", "--", "-E", "test(x)"];
    let (outcome, calls, stderr) = rooted_lane(base, &lane_base, ROOTS_CARGO, &env, &args);
    assert!(matches!(outcome, ExitOutcome::Code(7)), "{stderr}");
    let [call] = &calls[..] else {
        panic!(
            "one cargo call expected, {} recorded: {stderr}",
            calls.len()
        );
    };
    assert_eq!(
        call.args(),
        [
            "mutants",
            "--cargo-arg=--locked",
            "--jobs",
            "1",
            "--in-diff",
            "a b.diff",
            "--",
            "-E",
            "test(x)",
        ],
        "{stderr}"
    );
    let pwd = call.field("pwd").unwrap_or_default();
    assert_eq!(
        std::fs::canonicalize(pwd).ok(),
        std::fs::canonicalize(base.join("work")).ok(),
        "cargo mutants ran from {pwd:?}"
    );
    assert!(lanes_left(&lane_base).is_empty(), "{stderr}");
}

/// #221: the PUBLIC `rust-mutants` recipe hands the lane every argument it
/// received as ONE word. It used to splice them back into a command line with
/// `{{ args }}`, so a regex such as `fn a b|fn c` was cut at its space and its
/// `|` started a pipeline. The words below carry a space, a `|`, a `$`, both
/// kinds of quote, a glob and a leading dash: cargo-mutants must be handed each
/// exactly as `just` was, after the lane's own pinned flags.
#[test]
fn rust_mutants_forwards_each_argument_as_one_word() {
    let scratch = super::cli::OwnedScratch::root("gate", "pubargs");
    let base = scratch.path();
    let lane_base = base.join("l b");
    let env = caller_roots(base).to_vec();
    // The recipe calls `just` again, which finds its justfile from the working
    // directory as it does for a developer in the checkout: give `work` the
    // real one, by link, so nothing runs in the repository itself.
    let work = base.join("work");
    std::fs::create_dir_all(&work).unwrap_or_else(|why| panic!("{}: {why}", work.display()));
    std::os::unix::fs::symlink(root().join("justfile"), work.join("justfile"))
        .unwrap_or_else(|why| panic!("the linked justfile: {why}"));
    let words = [
        "--in-diff",
        "a b.diff",
        "--regex",
        "fn a b|fn c",
        "-E",
        "test(x) | test(y)",
        "$HOME 'q' \"d\" *",
        "-- dash",
    ];
    let mut recipe = vec!["rust-mutants"];
    recipe.extend_from_slice(&words);
    let (outcome, calls, stderr) = rooted_just(base, &lane_base, ROOTS_CARGO, &env, &recipe);
    let [call] = &calls[..] else {
        panic!(
            "one cargo call expected, {} recorded: {stderr}",
            calls.len()
        );
    };
    let mut want = vec!["mutants", "--cargo-arg=--locked", "--jobs", "1"];
    want.extend_from_slice(&words);
    assert_eq!(
        call.args(),
        want,
        "rust-mutants must pass each argument through as one word: {stderr}"
    );
    assert!(matches!(outcome, ExitOutcome::Code(0)), "{stderr}");
    assert!(lanes_left(&lane_base).is_empty(), "{stderr}");
}

/// `root.sh <binary>`: start `<binary>` (a link to bash) on `kid.sh`, so the
/// binary runs below the fake `cargo-mutants` that runs this, wait for it and
/// write its exit status to `hog.status` — `137` when the watcher killed it.
const WATCH_ROOT_SH: &str = r#""$1" "$AE_FAKE_W/kid.sh" "$AE_FAKE_W/hog.pid" 60000000 &
wait "$!"
echo "$?" >"$AE_FAKE_W/hog.status"
"#;

/// `kid.sh <pidfile> <bytes>`: publish the pid, hold that much memory in this
/// very process, then stay until the release file appears or a minute passes.
/// Publish `<pidfile>.ready` only on release: an eligible over-ceiling child
/// must be killed before it can publish readiness, without a scheduling race.
const WATCH_KID_SH: &str = r#"echo $$ >"$1.tmp" && mv "$1.tmp" "$1"
printf -v x '%*s' "$2" ''
i=0
while [ ! -e "$AE_FAKE_W/release" ] && [ "$i" -lt 600 ]; do sleep 0.1; i=$((i + 1)); done
if [ -e "$AE_FAKE_W/release" ]; then : >"$1.ready"; fi
"#;

/// The fake `cargo` for the #222 watcher pin: it links a test binary `ae-hog`
/// into `<intermediate root>/debug/deps/` (resolved as [`ROOTS_CARGO`] does),
/// runs it over the 32 MB ceiling below a fake `cargo-mutants`, and waits for
/// readiness or the exact child's saved status before waiting up to 15 s for
/// the lane's receipt. It records `dead <status>` when `root.sh` already saw
/// the binary end, else `alive` — read from that status, never from a pid.
/// Only then does it release the binary and wait for its own
/// `cargo-mutants` child, so no fixture process outlives it and nothing is ever
/// signalled by a pid that may have been reused.
const WATCHED_CARGO: &str = r#"case "$1" in mutants) ;; *) exit 0 ;; esac
W=$AE_FAKE_W
b="${CARGO_BUILD_BUILD_DIR:-${CARGO_TARGET_DIR:-${CARGO_BUILD_TARGET_DIR:-$PWD/target}}}"
mkdir -p "$b/debug/deps" && ln -sf /bin/bash "$b/debug/deps/ae-hog" || exit 97
"$W/cargo-mutants" "$W/root.sh" "$b/debug/deps/ae-hog" &
cm=$!
trap 'touch "$W/release"; wait "$cm" 2>/dev/null' EXIT
i=0
until [ -e "$W/hog.pid.ready" ] || [ -e "$W/hog.status" ]; do i=$((i + 1)); [ "$i" -lt 300 ] || exit 3; sleep 0.2; done
i=0
while [ ! -s mutants.out/ae-rss-kills.log ] && [ "$i" -lt 75 ]; do i=$((i + 1)); sleep 0.2; done
sleep 1
if [ -e "$W/hog.status" ]; then echo "dead $(cat "$W/hog.status")"; else echo alive; fi >"$AE_FAKE_REC"
exit 0
"#;

/// #222: the memory watcher still sees a test binary of the mutation run in
/// the lane's own intermediate root — even when the caller's intermediate
/// directory has no `target` component it could match — and the receipt still
/// lands in the caller's `mutants.out`. The lane base has no space: the
/// watcher reads the first argv word, so a spaced base blinds it, today and
/// after (a named residual, not this pin's subject).
/// Readiness is held until teardown release: a real kill must be observable
/// even when the child cannot publish its allocation-ready marker first.
#[test]
fn the_mutants_watcher_sees_a_test_binary_in_the_lanes_own_build_root() {
    use std::os::unix::fs::symlink;
    let scratch = super::cli::OwnedScratch::root("gate", "mwatch");
    let base = scratch.path();
    let (watch, lane_base, caller) = (base.join("watch"), base.join("lane"), base.join("callerb"));
    for dir in [&watch, &lane_base, &caller] {
        std::fs::create_dir_all(dir).unwrap_or_else(|why| panic!("{}: {why}", dir.display()));
    }
    std::fs::write(caller.join("caller-sentinel"), "")
        .unwrap_or_else(|why| panic!("the caller sentinel: {why}"));
    let _ = std::fs::remove_file(watch.join("cargo-mutants"));
    symlink("/bin/bash", watch.join("cargo-mutants"))
        .unwrap_or_else(|why| panic!("the fake cargo-mutants: {why}"));
    for (name, body) in [("root.sh", WATCH_ROOT_SH), ("kid.sh", WATCH_KID_SH)] {
        std::fs::write(watch.join(name), body).unwrap_or_else(|why| panic!("{name}: {why}"));
    }
    let text = |path: &Path| {
        path.to_str()
            .unwrap_or_else(|| panic!("a UTF-8 scratch path: {}", path.display()))
            .to_owned()
    };
    let env = [
        ("AE_FAKE_W", text(watch.as_path())),
        ("AE_MUTANTS_RSS_MAX_MB", "32".to_owned()),
        ("CARGO_BUILD_BUILD_DIR", text(caller.as_path())),
    ];
    let (outcome, _, stderr) = rooted_lane(base, &lane_base, WATCHED_CARGO, &env, &["mutants"]);
    let ready = watch.join("hog.pid.ready").exists();
    let status = std::fs::read_to_string(watch.join("hog.status")).unwrap_or_default();
    assert!(
        matches!(outcome, ExitOutcome::Code(0)),
        "ready={ready}, hog.status={status:?}: {stderr}"
    );
    assert!(
        !ready,
        "the child published readiness before its kill: {stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(base.join("record")).unwrap_or_default(),
        "dead 137\n",
        "the over-ceiling test binary outlived the watcher: {stderr}"
    );
    let receipt =
        std::fs::read_to_string(base.join("work/mutants.out/ae-rss-kills.log")).unwrap_or_default();
    assert!(
        !receipt.is_empty(),
        "no receipt in the caller's mutants.out: {stderr}"
    );
    let lanes = text(lane_base.as_path());
    let pid = read(&watch.join("hog.pid"));
    for line in receipt.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(fields.get(1).copied(), Some(pid.trim()), "{line:?}");
        // The receipt truncates KiB to whole MiB, so a kill just over 32 MiB
        // legitimately records 32, even though the watcher's comparison is >.
        assert!(
            fields
                .get(2)
                .and_then(|rss| rss.parse::<u32>().ok())
                .is_some_and(|rss| rss >= 32),
            "{line:?}"
        );
        let binary = fields.get(3).copied().unwrap_or_default();
        assert!(
            binary.starts_with(&format!("{lanes}/ae-rust-test."))
                && binary.ends_with("/debug/deps/ae-hog"),
            "{line:?}"
        );
    }
    assert_untouched(
        &[("CARGO_BUILD_BUILD_DIR", text(caller.as_path()))],
        "watcher",
    );
    assert!(lanes_left(&lane_base).is_empty(), "{stderr}");
}

/// #222: the repository's own cargo-mutants config adds no Cargo output
/// redirect of its own. Its extra argv is an explicit Cargo argument, which the
/// lane passes through and its isolation guarantee does not cover, so a
/// redirect there would sit outside that guarantee for every run.
#[test]
fn the_repo_mutants_config_adds_no_cargo_output_redirect() {
    let text = read(&root().join(".cargo/mutants.toml"));
    for needle in [
        "target-dir",
        "build-dir",
        "target_dir",
        "build_dir",
        "--config",
    ] {
        assert!(!text.contains(needle), ".cargo/mutants.toml names {needle}");
    }
}

/// #222: Cargo 1.97.1 expands `{…}` templates in a build-dir path (its
/// `build.build-dir` "supports path templating", and no escape is documented),
/// so a lane directory whose path holds `{` or `}` cannot be bound as the
/// mutation run's intermediate root without the resolved path possibly leaving
/// the lane's own tree. The mutants lane refuses such a path before it starts
/// cargo — nonzero, a diagnostic naming the path, no cargo call — and still
/// removes the lane. A known template and an unmatched brace of either kind are
/// refused alike; the test and coverage lanes over the same bases run as before.
///
/// Each base is proven usable FIRST: its deepest lane socket fits `sun_path`,
/// and the test and coverage lanes run there. Only then can the mutants lane's
/// failure be the refusal, not a base no lane could start on.
#[test]
fn a_mutants_lane_refuses_a_scratch_path_cargo_would_template() {
    use std::os::unix::fs::MetadataExt as _;
    let scratch = super::cli::OwnedScratch::root("gate", "mb");
    let base = scratch.path();
    let uid = std::fs::metadata(base)
        .unwrap_or_else(|why| panic!("{}: {why}", base.display()))
        .uid();
    for name in ["t{workspace-root}", "u{", "v}"] {
        let lane_base = base.join(name);
        let shown = lane_base
            .to_str()
            .unwrap_or_else(|| panic!("a UTF-8 scratch path: {}", lane_base.display()))
            .to_owned();
        // The deepest socket a lane binds here: its `ae-rust-test.<pid>.XXXXXX`
        // (seven pid digits, Linux's PID_MAX_LIMIT), then `tmux-<uid>/ae` for
        // its `tmux -L ae` sentry.
        let deepest = format!("{shown}/ae-rust-test.4194304.XXXXXX/tmux-{uid}/ae");
        assert!(
            deepest.len() <= super::scratch::SOCKET_PATH_MAX,
            "{name}: a lane under {shown:?} binds a {}-byte tmux socket, past {}: shorten AE_TEST_TMPDIR",
            deepest.len(),
            super::scratch::SOCKET_PATH_MAX
        );
        for (args, tools) in [
            (&["test"][..], &["nextest", "test"][..]),
            (&["cov"][..], &["llvm-cov"][..]),
        ] {
            let (outcome, calls, stderr) = rooted_lane(base, &lane_base, ROOTS_CARGO, &[], args);
            assert!(
                matches!(outcome, ExitOutcome::Code(0)),
                "{name} {args:?}: {stderr}"
            );
            let ran: Vec<&str> = calls
                .iter()
                .map(|call| call.field("tool").unwrap_or_default())
                .collect();
            assert_eq!(ran, tools, "{name} {args:?}: {stderr}");
            assert!(
                lanes_left(&lane_base).is_empty(),
                "{name} {args:?}: {stderr}"
            );
        }
        let (outcome, calls, stderr) =
            rooted_lane(base, &lane_base, ROOTS_CARGO, &[], &["mutants"]);
        assert!(
            matches!(outcome, ExitOutcome::Code(code) if code != 0),
            "{name}: the mutants lane did not refuse: {stderr}"
        );
        assert!(
            calls.is_empty(),
            "{name}: cargo ran {} time(s): {stderr}",
            calls.len()
        );
        assert!(
            stderr.contains(&shown),
            "{name}: the refusal does not name {shown:?}: {stderr}"
        );
        assert!(lanes_left(&lane_base).is_empty(), "{name}: {stderr}");
    }
}

/// Whether the `release` recipe refreshes the fuzz crate's lock inside the
/// version commit, with the product compiler, lets that refresh move nothing
/// but the ae version, and stops before the commit otherwise.
///
/// The fuzz crate is outside the workspace, so the bump's own rewrite never
/// reaches its lock — and that lock records the ae version. Refreshed too early
/// it records the OLD version; refreshed after the commit it needs a second
/// one. Since #240 the gate type-checks the fuzz crate with `--locked`, so a
/// lock the release leaves stale turns the release commit's own gate red, and
/// every gate after it. The refresh therefore uses the toolchain every gate
/// already has — never the fuzz nightly, never behind a probe that may skip
/// it, never `--locked` (which refuses instead of refreshing) — and a failure
/// stops the release rather than committing the stale lock. A release is not a
/// dependency update: the lock's delta is compared with the bumped version, and
/// any other change stops the release before the commit.
fn fuzz_lock_refresh_ok(justfile: &str) -> bool {
    let lines = recipe_text(justfile, "release:");
    let position = |needle: &str| lines.iter().position(|line| line.contains(needle));
    let (Some(bump), Some(refresh), Some(commit)) = (
        position("just bump"),
        position("--manifest-path fuzz/Cargo.toml"),
        position("chore(release):"),
    ) else {
        return false;
    };
    // The bump writes the version the refresh records, and the commit that
    // carries the bump carries the refreshed lock with it.
    if !(bump < refresh && refresh < commit) {
        return false;
    }
    let line = &lines[refresh];
    // The product compiler, as every gate runs it: no override, no probe.
    let foreign = [
        "rustup",
        "cargo +",
        "FUZZ_TOOLCHAIN",
        "nightly",
        "--locked",
        "--frozen",
    ];
    if bare_word_count(line, "cargo") == 0 || foreign.iter().any(|word| line.contains(word)) {
        return false;
    }
    // A failure stops the release ON ITS OWN STATUS: a plain command under
    // `set -e`, an `||` arm that exits, or an `if` guard whose own body exits
    // before its `fi`. A later check cannot stand in: a refresh that wrote the
    // allowed version line and then failed leaves a delta that check accepts.
    let errexit = lines.iter().any(|row| row.contains("set -euo pipefail"));
    let (stops, end) = if line.starts_with("if ") {
        let Some(end) = (refresh + 1..commit).find(|&at| {
            lines[at] == "fi" || lines[at].starts_with("fi ") || lines[at].starts_with("fi;")
        }) else {
            return false;
        };
        let exits = lines[refresh..end]
            .iter()
            .any(|row| bare_word_count(row, "exit") > 0);
        (exits, end)
    } else if let Some((_, arm)) = line.split_once("||") {
        (bare_word_count(arm, "exit") > 0, refresh)
    } else {
        (errexit, refresh)
    };
    // The refresh may move only the ae version: after it and before the
    // commit, the lock's diff is compared with the bumped `$VERSION`, and a
    // mismatch exits.
    let Some(delta) = (end + 1..commit)
        .find(|&at| lines[at].contains("git diff") && lines[at].contains("fuzz/Cargo.lock"))
    else {
        return false;
    };
    let compared = lines[delta..commit]
        .iter()
        .any(|row| row.contains("$VERSION") || row.contains("${VERSION}"));
    let refuses = lines[delta..commit]
        .iter()
        .any(|row| bare_word_count(row, "exit") > 0);
    // And nothing after it may put the stale lock back.
    stops
        && compared
        && refuses
        && lines[refresh..commit]
            .iter()
            .all(|row| !row.contains("fuzz/Cargo.lock") || !row.contains("checkout"))
}

#[test]
fn the_release_recipe_refreshes_the_fuzz_lock_with_the_product_compiler_or_stops() {
    assert!(
        fuzz_lock_refresh_ok(&read(&root().join("justfile"))),
        "the release must carry a fuzz lock refreshed by the product compiler in its \
         version commit, whose only change is the ae version, and stop before that \
         commit otherwise: the gate checks the fuzz crate with --locked (#240)"
    );

    let head = "release:\n    set -euo pipefail\n    VERSION=$(just bump)\n    cargo metadata --manifest-path fuzz/Cargo.toml --format-version 1 >/dev/null\n";
    let delta = r#"    delta="$(git diff -U0 -- fuzz/Cargo.lock | grep -E '^[-+][^-+]' || true)"
    if [ "$delta" != "$(printf -- '-version = "%s"\n+version = "%s"' "$PREVIOUS" "$VERSION")" ]; then
        echo "Error: fuzz/Cargo.lock changed beyond the ae version" >&2
        exit 1
    fi
"#;
    let tail = "    git add -u\n    git commit -m \"chore(release): $TAG\"\n";
    let good = format!("{head}{delta}{tail}");
    assert!(fuzz_lock_refresh_ok(&good));
    // GREEN — an explicit refusal of the refresh is a stop too.
    assert!(fuzz_lock_refresh_ok(&good.replace(
        "--format-version 1 >/dev/null\n",
        "--format-version 1 >/dev/null || { echo \"fuzz lock refresh failed\" >&2; exit 1; }\n"
    )));
    // GREEN — an `if` guard whose own failing body exits.
    assert!(fuzz_lock_refresh_ok(&good.replace(
        "    cargo metadata --manifest-path fuzz/Cargo.toml --format-version 1 >/dev/null\n",
        "    if ! cargo metadata --manifest-path fuzz/Cargo.toml --format-version 1 >/dev/null; then\n        echo \"Error: fuzz lock refresh failed\" >&2\n        exit 1\n    fi\n"
    )));
    // RED — a guard that only warns, leaning on the later delta check: a
    // refresh that wrote the allowed version line and then failed passes it.
    assert!(!fuzz_lock_refresh_ok(&good.replace(
        "    cargo metadata --manifest-path fuzz/Cargo.toml --format-version 1 >/dev/null\n",
        "    if ! cargo metadata --manifest-path fuzz/Cargo.toml --format-version 1 >/dev/null; then\n        echo \"Warning: fuzz lock not refreshed\" >&2\n    fi\n"
    )));
    // RED — the shape before #240: refreshed only when the fuzz nightly answers,
    // otherwise the stale lock is put back and committed with a warning.
    assert!(!fuzz_lock_refresh_ok(
        "release:\n    set -euo pipefail\n    VERSION=$(just bump)\n    if rustup run nightly rustc --version >/dev/null 2>&1 && cargo +nightly metadata --manifest-path fuzz/Cargo.toml --format-version 1 >/dev/null 2>&1; then\n        git add fuzz/Cargo.lock\n    else\n        git checkout -- fuzz/Cargo.lock\n        echo \"Warning: fuzz/Cargo.lock NOT refreshed\" >&2\n    fi\n    git commit -m \"chore(release): $TAG\"\n"
    ));
    // RED — no refresh at all.
    assert!(!fuzz_lock_refresh_ok(
        "release:\n    set -euo pipefail\n    VERSION=$(just bump)\n    git commit -m \"chore(release): $TAG\"\n"
    ));
    // RED — refreshed BEFORE the bump, so the lock records the replaced version.
    assert!(!fuzz_lock_refresh_ok(&format!(
        "release:\n    set -euo pipefail\n    cargo metadata --manifest-path fuzz/Cargo.toml --format-version 1 >/dev/null\n    VERSION=$(just bump)\n{delta}{tail}"
    )));
    // RED — `--locked` refuses the bumped lock instead of refreshing it.
    assert!(!fuzz_lock_refresh_ok(&good.replace(
        "cargo metadata --manifest",
        "cargo metadata --locked --manifest"
    )));
    // RED — the refresh's failure is swallowed.
    assert!(!fuzz_lock_refresh_ok(&good.replace(
        "--format-version 1 >/dev/null\n",
        "--format-version 1 >/dev/null || true\n"
    )));
    // RED — no errexit, so a plain failing refresh runs on into the commit.
    assert!(!fuzz_lock_refresh_ok(
        &good.replace("    set -euo pipefail\n", "")
    ));
    // RED — refreshed, then the old lock put back.
    assert!(!fuzz_lock_refresh_ok(&good.replace(
        "    git add -u\n",
        "    git checkout -- fuzz/Cargo.lock\n    git add -u\n"
    )));
    // RED — no delta check: a dependency drift rides into the release commit.
    assert!(!fuzz_lock_refresh_ok(&format!("{head}{tail}")));
    // RED — the delta is checked only after the commit already carries it.
    assert!(!fuzz_lock_refresh_ok(&format!("{head}{tail}{delta}")));
    // RED — a mismatch only warns.
    assert!(!fuzz_lock_refresh_ok(&good.replace("        exit 1\n", "")));
    // RED — the delta is not compared with the bumped version: any change passes.
    assert!(!fuzz_lock_refresh_ok(&good.replace(
        r#"    if [ "$delta" != "$(printf -- '-version = "%s"\n+version = "%s"' "$PREVIOUS" "$VERSION")" ]; then"#,
        r#"    if [ -z "$delta" ]; then"#
    )));
}

/// Whether the `release` recipe has the fuzz lock refreshed AND proved before it
/// first type-checks the fuzz crate on the bumped tree.
///
/// The bump moves the ae version the fuzz lock records, and the gate's
/// `rust-fuzz-check` runs `--locked`, so a lock still carrying the old version
/// makes that check refuse (`cannot update the lock file ... because --locked was
/// passed`). The release always runs the check on the bumped tree,
/// so the refresh and its "only the ae version moved" proof must stand between
/// the bump and the first check: a later refresh is too late, and a check that
/// runs before the proof would vouch for a lock the proof has not yet cleared.
/// At least one check must remain on the bumped tree. The hp proof covers the
/// pre-bump tree and cannot prove that the refreshed fuzz lock still compiles.
fn fuzz_lock_ready_before_bumped_fuzz_check_ok(justfile: &str) -> bool {
    let lines = recipe_text(justfile, "release:");
    let position = |needle: &str| lines.iter().position(|line| line.contains(needle));
    let (Some(bump), Some(refresh)) = (
        position("just bump"),
        position("--manifest-path fuzz/Cargo.toml"),
    ) else {
        return false;
    };
    if refresh < bump {
        return false;
    }
    // The proof is the first diff of the fuzz lock after the refresh, and it is
    // finished at the first refusal that follows it.
    let Some(proof) = (refresh + 1..lines.len())
        .find(|&at| lines[at].contains("git diff") && lines[at].contains("fuzz/Cargo.lock"))
    else {
        return false;
    };
    let Some(refusal) =
        (proof + 1..lines.len()).find(|&at| bare_word_count(&lines[at], "exit") > 0)
    else {
        return false;
    };
    // Every type-check of the fuzz crate after the bump: the recipe, or a locked
    // cargo call on the fuzz manifest (the refresh itself is not one).
    let checks: Vec<usize> = (bump + 1..lines.len())
        .filter(|&at| {
            lines[at].contains("rust-fuzz-check")
                || (at != refresh && lines[at].contains("--manifest-path fuzz/Cargo.toml"))
        })
        .collect();
    !checks.is_empty() && checks.iter().all(|&at| at > refusal)
}

#[test]
fn the_release_refreshes_and_proves_the_fuzz_lock_before_the_bumped_fuzz_check() {
    assert!(
        fuzz_lock_ready_before_bumped_fuzz_check_ok(&read(&root().join("justfile"))),
        "the release must refresh fuzz/Cargo.lock and prove that only the ae version moved \
         BEFORE its first `just rust-fuzz-check` on the bumped tree: that check runs --locked \
         and refuses a lock that still records the replaced version"
    );

    let refresh =
        "    cargo metadata --manifest-path fuzz/Cargo.toml --format-version 1 >/dev/null\n";
    let proof = "    fuzz_delta=\"$(git diff --no-ext-diff --numstat -- fuzz/Cargo.lock)\"\n    if [ \"$fuzz_delta\" != \"1\t1\tfuzz/Cargo.lock\" ]; then\n        echo \"Error: fuzz/Cargo.lock must change only the ae version to $VERSION\" >&2\n        exit 1\n    fi\n";
    let check = "    just rust-fmt-check\n    just rust-fuzz-check\n";
    let head = "release:\n    set -euo pipefail\n    VERSION=$(just bump)\n";
    let tail = "    git commit -m \"chore(release): $TAG\"\n";
    // GREEN — refreshed, proved, then checked: the shape this pin demands.
    assert!(fuzz_lock_ready_before_bumped_fuzz_check_ok(&format!(
        "{head}{refresh}{proof}{check}{tail}"
    )));
    // RED — the order before this fix: checked, then refreshed and proved. The
    // check refuses the stale lock under --locked and the release stops.
    assert!(!fuzz_lock_ready_before_bumped_fuzz_check_ok(&format!(
        "{head}{check}{refresh}{proof}{tail}"
    )));
    // RED — refreshed but checked before the proof has cleared the lock.
    assert!(!fuzz_lock_ready_before_bumped_fuzz_check_ok(&format!(
        "{head}{refresh}{check}{proof}{tail}"
    )));
    // RED — a good check later does not excuse one that still precedes the refresh.
    assert!(!fuzz_lock_ready_before_bumped_fuzz_check_ok(&format!(
        "{head}{check}{refresh}{proof}{check}{tail}"
    )));
    // RED — an inline locked cargo check of the fuzz crate ahead of the refresh.
    assert!(!fuzz_lock_ready_before_bumped_fuzz_check_ok(&format!(
        "{head}    cargo check --locked --manifest-path fuzz/Cargo.toml --bins\n{refresh}{proof}{tail}"
    )));
    // RED — the bumped-tree check is dropped, so the refreshed lock never compiles.
    assert!(!fuzz_lock_ready_before_bumped_fuzz_check_ok(&format!(
        "{head}{refresh}{proof}{tail}"
    )));
    // RED — no proof after the refresh: any drift of the lock passes.
    assert!(!fuzz_lock_ready_before_bumped_fuzz_check_ok(&format!(
        "{head}{refresh}{check}{tail}"
    )));
    // RED — the refresh stands before the bump, so it records the replaced version.
    assert!(!fuzz_lock_ready_before_bumped_fuzz_check_ok(&format!(
        "release:\n    set -euo pipefail\n{refresh}    VERSION=$(just bump)\n{proof}{check}{tail}"
    )));
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
///
/// The gate body lives ONCE, in `_linux-gate`, and two thin recipes hand it an
/// image and a bus: `rust-linux` (Ubuntu 24.04, no bus) and `rust-linux-26`
/// (Ubuntu 26.04), whose `bus` parameter defaults to `stall`: a user bus whose
/// systemd never answers, so tmux 3.6's `StartTransientUnit` waits out its 1 s
/// timeout before every pane child execs. It takes `off` for the validated-host
/// arm. The default is `stall` because that arm went green; moving it back is an
/// edit of this guard. A lane that cannot prove its
/// bus stalls would pass whatever the tests do, so on the stall arm the stall is
/// a PROVEN fact, set up and checked before anything is built; the bus kind is
/// validated on the host before any container starts, and a mismatch between the
/// host's source and the mount is fatal.
#[allow(
    clippy::too_many_lines,
    reason = "one ordered rule list over one recipe"
)]
fn linux_lanes_ok(justfile: &str) -> bool {
    let lines = recipe_text(justfile, "_linux-gate flavor image bus:");
    let contains = |needle: &str| lines.iter().any(|line| line.contains(needle));
    // The recipes take NO arch parameter: an arch flag would promise amd64
    // evidence the QEMU path cannot produce (the smoke covers the bundle
    // instead). The shared body names no amd64 either.
    if !recipe_text(justfile, "rust-linux arch=").is_empty()
        || lines.iter().any(|line| line.contains("amd64"))
    {
        return false;
    }
    // Two thin entry points, each handing the body its own pinned image and
    // bus: the 24.04 lane never grows a bus, and the 26.04 lane takes its bus
    // as a parameter that defaults to `stall` and is forwarded as given, so `off`
    // (the validated-host arm) stays reachable and nothing else decides it.
    let has_line = |wanted: &str| justfile.lines().any(|line| line == wanted);
    if !has_line("rust-linux: (_linux-gate \"linux\" LINUX_IMAGE \"off\")")
        || !has_line("rust-linux-26 bus=\"stall\": (_linux-gate \"linux26\" LINUX_IMAGE_26 bus)")
    {
        return false;
    }
    // Both base images are digest pins, and the build's FROM is the one the
    // caller handed in — never a tag a remote can move under us.
    if !justfile.contains("LINUX_IMAGE := \"ubuntu:24.04@sha256:")
        || !justfile.contains("LINUX_IMAGE_26 := \"ubuntu:26.04@sha256:")
        || !contains("FROM {{ image }}")
    {
        return false;
    }
    // The daemon probe and the volume ownership one-shot run that same image.
    if lines
        .iter()
        .filter(|line| line.contains("docker run") && line.contains("--label ae.lane=linux"))
        .filter(|line| !line.contains("--init"))
        .any(|line| !line.contains("{{ image }}"))
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
        .map(|line| line.matches("ae-{{ flavor }}-arm64-").count())
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
    // The bus kind is validated on the HOST, before the first docker call: only
    // `off` and `stall` pass, anything else exits 2 and starts nothing.
    let (Some(check), Some(first_docker)) = (
        joined.find("case \"{{ bus }}\" in off|stall) ;; *)"),
        joined.find("docker "),
    ) else {
        return false;
    };
    match joined[check..].find("exit 2 ;; esac") {
        Some(found) if check + found < first_docker => {}
        _ => return false,
    }
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
    // The lane's exit status is the REAL `just test`'s: the container ends on
    // it, the status is captured, and the recipe exits with it.
    if !contains("just test' || rc=$?") || !contains("exit \"$rc\"") {
        return false;
    }
    // The mounted checkout is the tree the host saw: the host records HEAD, tree
    // and the count of dirty paths, passes them in, and the container compares
    // them through a git config of its own scoped to those commands, before
    // anything runs. A dirty tree is identified by HEAD and that count only, so
    // exact-tree evidence needs a clean checkout.
    if !contains("-e AE_LANE_HEAD=")
        || !contains("-e AE_LANE_TREE=")
        || !contains("-e AE_LANE_DIRTY=")
    {
        return false;
    }
    // The bus: handed in as data, and on the stall arm set up and PROVEN to
    // stall before the first build step, fatal when the proof does not hold.
    // The proof is a real method call to the very name tmux asks, and it must
    // end in a reply timeout (`NoReply`) after at least two seconds: an
    // immediate ServiceUnknown (no stall at all), a connection error or any
    // other slow failure cannot pass. The off arm runs none of it.
    if !contains("-e AE_LANE_BUS={{ bus }}") {
        return false;
    }
    // Each needle must appear AFTER the one before it, so the order is the rule:
    // the fatal source check, then the stall arm's own condition, the service
    // file, the daemon, the address, a reset clock, the call and its timeout,
    // the error kind it must end in, the elapsed bound and the exit that makes
    // the proof fatal, and only then the first build step and the suite.
    let mut at = 0;
    for needle in [
        "GIT_CONFIG_GLOBAL=",
        "|| exit 3",
        "if [ \"$AE_LANE_BUS\" = stall ]; then",
        "Name=org.freedesktop.systemd1",
        "dbus-daemon --session --fork",
        "export DBUS_SESSION_BUS_ADDRESS=",
        "SECONDS=0",
        "dbus-send --session --print-reply --dest=org.freedesktop.systemd1",
        "--reply-timeout=2500",
        "*DBus.Error.NoReply*) ;; *) exit 4 ;; esac",
        "\"$SECONDS\" -lt 2 ]; then exit 4; fi",
        "/stage/rustup-init",
        "just test",
    ] {
        match container[at..].find(needle) {
            Some(found) => at += found + needle.len(),
            None => return false,
        }
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

/// The container body's source-identity check, bus prelude and gate steps are
/// separate pieces so a red below removes or moves exactly one of them.
const LINUX_IDENT: &str = concat!(
    "        gitcfg=\"$(mktemp)\"\n",
    "        GIT_CONFIG_GLOBAL=\"$gitcfg\" git rev-parse HEAD\n",
    "        [ \"$got\" = \"$AE_LANE_HEAD\" ] || exit 3\n",
);
const LINUX_BUS: &str = concat!(
    "        if [ \"$AE_LANE_BUS\" = stall ]; then\n",
    "        printf \"Name=org.freedesktop.systemd1\\n\" > \"$svc\"\n",
    "        dbus-daemon --session --fork --address=unix:path=/tmp/ae-lane-bus\n",
    "        export DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/ae-lane-bus\n",
    "        SECONDS=0\n",
    "        err=\"$(dbus-send --session --print-reply --dest=org.freedesktop.systemd1 --reply-timeout=2500 /org/freedesktop/systemd1 org.freedesktop.DBus.Peer.Ping 2>&1 || true)\"\n",
    "        case \"$err\" in *DBus.Error.NoReply*) ;; *) exit 4 ;; esac\n",
    "        if [ \"$SECONDS\" -lt 2 ]; then exit 4; fi\n",
    "        fi\n",
);
const LINUX_BUILD: &str = concat!(
    "        /stage/rustup-init -y\n",
    "        just rust-setup\n",
    "        cargo clean --locked -p ae\n",
    "        just test' || rc=$?\n",
);

/// The synthetic `justfile` the Linux lane guard is proved against: both pins,
/// both thin entry points, the shared body with its identity check, bus prelude
/// and gate steps, and the smoke.
fn linux_lanes_green() -> String {
    format!(
        "{}{}{}{}{}{}{}",
        concat!(
            "LINUX_IMAGE := \"ubuntu:24.04@sha256:008173\"\n",
            "LINUX_IMAGE_26 := \"ubuntu:26.04@sha256:6a0611\"\n",
            "RUSTUP_VERSION := \"1.29.1\"\n",
            "test: rust-check\n",
            "rust-check: rust-fmt-check rust-test\n",
            "release:\n",
            "    just test\n",
            "\n",
            "rust-linux: (_linux-gate \"linux\" LINUX_IMAGE \"off\")\n",
            "rust-linux-26 bus=\"stall\": (_linux-gate \"linux26\" LINUX_IMAGE_26 bus)\n",
            "_linux-gate flavor image bus:\n",
            "    case \"{{ bus }}\" in off|stall) ;; *) echo \"Error: bus must be off or stall\" >&2; exit 2 ;; esac\n",
            "    daemon_arch=\"$(docker run --rm --label ae.lane=linux {{ image }} uname -m)\"\n",
            "    for v in cargo rustup target; do docker volume create \"ae-{{ flavor }}-arm64-$v\" >/dev/null; done\n",
            "    curl -o \"$lane/rustup-init\" https://static.rust-lang.org/rustup/archive/{{ RUSTUP_VERSION }}/rustup-init\n",
            "    repo=\"$PWD\"\n",
            "    mounts=(-v \"$repo:$repo:ro\")\n",
            "    docker build --build-arg UID=\"$(id -u)\" -t ae-{{ flavor }}-arm64 - <<'DOCKERFILE'\n",
            "    FROM {{ image }}\n",
            "    USER \"$UID\"\n",
            "    DOCKERFILE\n",
            "    docker run --rm --init \"${mounts[@]}\" -v \"ae-{{ flavor }}-arm64-cargo:/c\" \\\n",
            "        -v \"ae-{{ flavor }}-arm64-rustup:/r\" -v \"ae-{{ flavor }}-arm64-target:/t\" -e NEXTEST_PROFILE=linux \\\n",
            "        -e AE_LANE_HEAD=\"$head\" -e AE_LANE_TREE=\"$tree\" -e AE_LANE_DIRTY=\"$dirty\" -e AE_LANE_BUS={{ bus }} \\\n",
            "        ae-{{ flavor }}-arm64 \\\n",
            "        bash -c 'set -euo pipefail\n",
        ),
        LINUX_IDENT,
        LINUX_BUS,
        LINUX_BUILD,
        "    echo \"==> ae-linux gate: rc=$rc\"\n    exit \"$rc\"\n\n",
        concat!(
            "rust-linux-smoke:\n",
            "    version=\"$(just version)\"\n",
            "    line=\"$(grep -F \" ae-$version-linux-x86_64-musl.tar.gz\" dist/SHA256SUMS)\"\n",
            "    [ -f \"dist/ae-$version-linux-x86_64-musl.tar.gz\" ] || { echo \"run: just bundles\" >&2; exit 1; }\n",
            "    out=\"$(docker run --rm --platform linux/amd64 ae-smoke /run/ae-core --version)\"\n",
            "    [ \"$out\" = \"ae $version\" ]\n",
        ),
        "",
    )
}

/// Whether the gate chain stays free of the on-demand container lanes: `just
/// test`, the checks under it, the advisory `pre` and the release must never
/// reach `rust-linux` or `rust-linux-26`, because a container lane is hours on
/// a cold cache and needs Docker and a lock.
fn linux_lanes_stay_on_demand(justfile: &str) -> bool {
    let header_clean = |prefix: &str| {
        justfile
            .lines()
            .filter(|line| line.starts_with(prefix))
            .all(|line| !line.contains("rust-linux"))
    };
    [
        "test:",
        "rust-check:",
        "rust-test:",
        "pre:",
        "check:",
        "release:",
    ]
    .iter()
    .all(|prefix| {
        header_clean(prefix)
            && recipe_text(justfile, prefix)
                .iter()
                .all(|line| !line.contains("rust-linux"))
    })
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one planted mutation per guard, listed flat so each line names its rule"
)]
fn the_linux_container_lanes_are_digest_pinned_read_only_and_non_root() {
    let green = linux_lanes_green();
    let green = green.as_str();
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
        "        just rust-setup\n        cargo clean --locked -p ae\n        just test' || rc=$?\n",
        "        cargo fmt --all --check && cargo nextest run' || rc=$?\n"
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
        &green.replace("ae-{{ flavor }}-arm64-", "ae-linux-")
    ));
    assert!(!linux_lanes_ok(&green.replace(
        "docker build --build-arg UID=\"$(id -u)\"",
        "docker build"
    )));
    // RED — an arch parameter is back: it would promise amd64 evidence the
    // dropped QEMU path cannot produce.
    assert!(!linux_lanes_ok(&green.replace(
        "rust-linux: (_linux-gate \"linux\" LINUX_IMAGE \"off\")",
        "rust-linux arch=\"arm64\": (_linux-gate \"linux\" LINUX_IMAGE \"off\")"
    )));
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
        "        cargo clean --locked -p ae\n        just test' || rc=$?\n",
        "        just test' || rc=$?\n        cargo clean --locked -p ae\n"
    )));
    // RED — the clean on the host side: it must run inside the container.
    assert!(!linux_lanes_ok(
        &green
            .replace("        cargo clean --locked -p ae\n", "")
            .replace(
                "        bash -c 'set -euo pipefail\n",
                "        cargo clean --locked -p ae\n        bash -c 'set -euo pipefail\n"
            )
    ));

    // The rules below are the 26.04 lane's. Each new guard is proved by one
    // planted mutation, named when it is rejected, so a run shows WHICH unsafe
    // edit each guard caught and none passes by being unable to fail.
    let rejected = |name: &str, mutated: String| {
        assert!(!linux_lanes_ok(&mutated), "guard failed to reject: {name}");
        println!("rejected: {name}");
    };
    let wrapper = "rust-linux-26 bus=\"stall\": (_linux-gate \"linux26\" LINUX_IMAGE_26 bus)";
    rejected(
        "26.04 base image pinned by tag",
        green.replace(
            "LINUX_IMAGE_26 := \"ubuntu:26.04@sha256:6a0611\"",
            "LINUX_IMAGE_26 := \"ubuntu:26.04\"",
        ),
    );
    rejected(
        "26.04 pin missing",
        green.replace("LINUX_IMAGE_26 := \"ubuntu:26.04@sha256:6a0611\"\n", ""),
    );
    rejected(
        "rust-linux-26 wired to the 24.04 image",
        green.replace(
            wrapper,
            "rust-linux-26 bus=\"stall\": (_linux-gate \"linux26\" LINUX_IMAGE bus)",
        ),
    );
    rejected(
        "rust-linux-26 loses its bus parameter, the stall arm is unreachable",
        green.replace(
            wrapper,
            "rust-linux-26: (_linux-gate \"linux26\" LINUX_IMAGE_26 \"off\")",
        ),
    );
    rejected(
        "rust-linux-26 is hard-wired to stall, the validated-host arm is gone",
        green.replace(
            wrapper,
            "rust-linux-26: (_linux-gate \"linux26\" LINUX_IMAGE_26 \"stall\")",
        ),
    );
    rejected(
        "rust-linux-26 defaults to off",
        green.replace(
            wrapper,
            "rust-linux-26 bus=\"off\": (_linux-gate \"linux26\" LINUX_IMAGE_26 bus)",
        ),
    );
    rejected(
        "rust-linux-26 ignores its bus parameter",
        green.replace(
            wrapper,
            "rust-linux-26 bus=\"stall\": (_linux-gate \"linux26\" LINUX_IMAGE_26 \"stall\")",
        ),
    );
    rejected(
        "rust-linux-26 has no default, a bare run is a usage error",
        green.replace(
            wrapper,
            "rust-linux-26 bus: (_linux-gate \"linux26\" LINUX_IMAGE_26 bus)",
        ),
    );
    rejected(
        "rust-linux gains the bus",
        green.replace(
            "rust-linux: (_linux-gate \"linux\" LINUX_IMAGE \"off\")",
            "rust-linux: (_linux-gate \"linux\" LINUX_IMAGE \"stall\")",
        ),
    );
    rejected(
        "26.04 lane shares the 24.04 caches",
        green.replace(
            wrapper,
            "rust-linux-26 bus=\"stall\": (_linux-gate \"linux\" LINUX_IMAGE_26 bus)",
        ),
    );
    rejected(
        "bus kind not validated on the host",
        green.replace(
            "    case \"{{ bus }}\" in off|stall) ;; *) echo \"Error: bus must be off or stall\" >&2; exit 2 ;; esac\n",
            "",
        ),
    );
    rejected(
        "bus kind validated only after docker has started",
        green
            .replace(
                "    case \"{{ bus }}\" in off|stall) ;; *) echo \"Error: bus must be off or stall\" >&2; exit 2 ;; esac\n",
                "",
            )
            .replace(
                "    for v in cargo",
                "    case \"{{ bus }}\" in off|stall) ;; *) echo \"Error: bus must be off or stall\" >&2; exit 2 ;; esac\n    for v in cargo",
            ),
    );
    rejected(
        "unknown bus kind is not fatal",
        green.replace("exit 2 ;; esac", "true ;; esac"),
    );
    rejected(
        "an unknown bus kind is accepted",
        green.replace("in off|stall) ;; *)", "in off|stall|*) ;; *)"),
    );
    rejected(
        "stall proof runs on the wrong bus kind",
        green.replace("\"$AE_LANE_BUS\" = stall ]", "\"$AE_LANE_BUS\" = off ]"),
    );
    rejected(
        "stall proof not conditioned on the bus kind",
        green.replace("        if [ \"$AE_LANE_BUS\" = stall ]; then\n", ""),
    );
    rejected(
        "build FROM a fixed image instead of the caller's",
        green.replace("    FROM {{ image }}\n", "    FROM ubuntu:26.04\n"),
    );
    rejected(
        "daemon probe runs a different image",
        green.replace("{{ image }} uname -m", "ubuntu:24.04 uname -m"),
    );
    rejected(
        "no source identity handed in",
        green.replace(" -e AE_LANE_TREE=\"$tree\"", ""),
    );
    rejected(
        "no source identity check in the container",
        green.replace(LINUX_IDENT, ""),
    );
    rejected(
        "source mismatch is not fatal",
        green.replace("|| exit 3", "|| true"),
    );
    rejected(
        "bus kind not handed in",
        green.replace(" -e AE_LANE_BUS={{ bus }}", ""),
    );
    rejected("no bus prelude at all", green.replace(LINUX_BUS, ""));
    rejected(
        "no stalling service file",
        green.replace(
            "        printf \"Name=org.freedesktop.systemd1\\n\" > \"$svc\"\n",
            "",
        ),
    );
    rejected(
        "no stall proof call",
        green.replace(
            "        err=\"$(dbus-send --session --print-reply --dest=org.freedesktop.systemd1 --reply-timeout=2500 /org/freedesktop/systemd1 org.freedesktop.DBus.Peer.Ping 2>&1 || true)\"\n",
            "",
        ),
    );
    rejected(
        "proof is a signal, not a method call",
        green.replace("dbus-send --session --print-reply ", "dbus-send --session "),
    );
    rejected(
        "proof accepts any failure, not the reply timeout",
        green.replace("DBus.Error.NoReply", "DBus.Error"),
    );
    rejected(
        "proof drops its timeout",
        green.replace(" --reply-timeout=2500", ""),
    );
    rejected(
        "proof drops its elapsed bound",
        green.replace("        if [ \"$SECONDS\" -lt 2 ]; then exit 4; fi\n", ""),
    );
    rejected(
        "proof does not reset its clock",
        green.replace("        SECONDS=0\n", ""),
    );
    rejected("proof outcome never fatal", green.replace("exit 4", "true"));
    rejected(
        "wrong error kind passes the proof",
        green.replace("*) exit 4 ;; esac", "*) true ;; esac"),
    );
    rejected(
        "too-fast failure passes the proof",
        green.replace("then exit 4; fi", "then true; fi"),
    );
    rejected(
        "bus set up after the suite",
        green.replace(LINUX_BUS, "").replace(
            "        just test' || rc=$?\n",
            &format!("        just test' || rc=$?\n{LINUX_BUS}"),
        ),
    );
    rejected(
        "bus set up after the toolchain bootstrap",
        green.replace(LINUX_BUS, "").replace(
            "        just rust-setup\n",
            &format!("        just rust-setup\n{LINUX_BUS}"),
        ),
    );
    rejected(
        "suite status swallowed",
        green.replace("just test' || rc=$?", "just test' || true"),
    );
    rejected(
        "recipe exits with something else than the suite status",
        green.replace("exit \"$rc\"", "exit 0"),
    );

    assert!(linux_lanes_ok(&read(&root().join("justfile"))));
}

#[test]
fn the_container_lanes_stay_out_of_the_gate_chain() {
    let green = linux_lanes_green();
    assert!(linux_lanes_stay_on_demand(&green));
    for (name, mutated) in [
        (
            "test depends on the 26.04 lane",
            green.replace("test: rust-check\n", "test: rust-check rust-linux-26\n"),
        ),
        (
            "rust-check depends on the lane",
            green.replace(
                "rust-check: rust-fmt-check rust-test\n",
                "rust-check: rust-fmt-check rust-test rust-linux\n",
            ),
        ),
        (
            "release runs the lane",
            green.replace(
                "release:\n    just test\n",
                "release:\n    just rust-linux-26\n",
            ),
        ),
    ] {
        assert!(
            !linux_lanes_stay_on_demand(&mutated),
            "guard failed to reject: {name}"
        );
        println!("rejected: {name}");
    }
    assert!(linux_lanes_stay_on_demand(&read(&root().join("justfile"))));
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

/// The string value of a `NAME := "value"` line of the justfile, or empty.
fn justfile_string(justfile: &str, name: &str) -> String {
    justfile
        .lines()
        .find_map(|line| line.trim_end().strip_prefix(&format!("{name} := ")))
        .map(|value| value.trim().trim_matches('"').to_owned())
        .unwrap_or_default()
}

/// Whether the release builds both halves in ONE cargo invocation (#P1b): the
/// `bundles` recipe runs exactly one `cargo build`, release and locked, naming
/// the native and the foreign target once each and no other, with the musl C
/// compiler scoped to the foreign target, and takes its two binaries from the
/// `target/<triple>/release/` directories that invocation fills. Two cargo
/// processes on one target directory would queue on its lock; one process
/// shares one jobserver and builds the two single-codegen-unit crates side by
/// side. `just install` and `just rust-build-release` build the host alone and
/// keep `target/release/ae`, which the Linux CI legs read.
fn bundles_build_both_targets_once_ok(justfile: &str, toolchain: &str) -> bool {
    let (native, cross) = (
        justfile_string(justfile, "RUST_NATIVE_TARGET"),
        justfile_string(justfile, "RUST_CROSS_TARGET"),
    );
    let targets = toolchain
        .lines()
        .find_map(|line| line.trim().strip_prefix("targets = "))
        .unwrap_or_default();
    let listed = |triple: &str| !triple.is_empty() && targets.contains(&format!("\"{triple}\""));
    let lines = recipe_text(justfile, "bundles:");
    let builds: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains("cargo build"))
        .collect();
    let [build] = builds.as_slice() else {
        return false;
    };
    let word = |name: &str| format!("--target {{{{ {name} }}}}");
    let host_alone = ["install:", "rust-build-release:"].iter().all(|header| {
        let own = recipe_text(justfile, header);
        own.iter()
            .filter(|line| line.contains("cargo build"))
            .all(|line| line.ends_with("cargo build --release --locked"))
            && own.iter().any(|line| line.contains("target/release/ae"))
    });
    listed(&native)
        && listed(&cross)
        && native != cross
        && build.starts_with("CC_x86_64_unknown_linux_musl={{ RUST_MUSL_CC }} ")
        && build.contains("cargo build --release --locked ")
        && build.matches("--target").count() == 2
        && build.matches(&word("RUST_NATIVE_TARGET")).count() == 1
        && build.matches(&word("RUST_CROSS_TARGET")).count() == 1
        && lines
            .iter()
            .any(|line| line == "native_bin=\"target/{{ RUST_NATIVE_TARGET }}/release/ae\"")
        && lines
            .iter()
            .any(|line| line == "musl_bin=\"target/{{ RUST_CROSS_TARGET }}/release/ae\"")
        && !lines.join("\n").contains("target/release/")
        && host_alone
}

#[test]
fn the_release_builds_both_halves_in_one_cargo_invocation_with_two_targets() {
    let justfile = read(&root().join("justfile"));
    let toolchain = read(&root().join("rust-toolchain.toml"));
    assert!(
        bundles_build_both_targets_once_ok(&justfile, &toolchain),
        "`just bundles` must run ONE `cargo build --release --locked` with a --target for each \
         of the native and the musl triple, and read both binaries from target/<triple>/release"
    );
    assert_eq!(
        justfile_string(&justfile, "RUST_NATIVE_TARGET"),
        "aarch64-apple-darwin"
    );
    // RED — the old shape: a native build, then a second process for musl.
    let one = "cargo build --release --locked --target {{ RUST_NATIVE_TARGET }} --target {{ RUST_CROSS_TARGET }}";
    assert!(justfile.contains(one));
    let two = justfile.replace(
        one,
        "cargo build --release --locked --target {{ RUST_CROSS_TARGET }}\n    cargo build --release --locked --target {{ RUST_NATIVE_TARGET }}",
    );
    assert!(!bundles_build_both_targets_once_ok(&two, &toolchain));
    // RED — one target only, a third target, a build without the lockfile or the
    // release profile, a musl compiler no longer scoped in front of the build.
    for broken in [
        one.replace(" --target {{ RUST_NATIVE_TARGET }}", ""),
        format!("{one} --target wasm32-unknown-unknown"),
        one.replace(" --locked", ""),
        one.replace(" --release", ""),
    ] {
        assert!(!bundles_build_both_targets_once_ok(
            &justfile.replace(one, &broken),
            &toolchain
        ));
    }
    assert!(!bundles_build_both_targets_once_ok(
        &justfile.replace("CC_x86_64_unknown_linux_musl={{ RUST_MUSL_CC }}", "CC=cc"),
        &toolchain
    ));
    // RED — the native binary still read from the host directory, a triple the
    // toolchain does not list, and a host-alone recipe that grew a target.
    assert!(!bundles_build_both_targets_once_ok(
        &justfile.replace(
            "native_bin=\"target/{{ RUST_NATIVE_TARGET }}/release/ae\"",
            "native_bin=\"target/release/ae\""
        ),
        &toolchain
    ));
    assert!(!bundles_build_both_targets_once_ok(
        &justfile.replace(
            "RUST_NATIVE_TARGET := \"aarch64-apple-darwin\"",
            "RUST_NATIVE_TARGET := \"aarch64-unknown-linux-gnu\""
        ),
        &toolchain
    ));
    assert!(!bundles_build_both_targets_once_ok(
        &justfile.replace(
            "rust-build-release:\n    cargo build --release --locked\n",
            "rust-build-release:\n    cargo build --release --locked --target {{ RUST_NATIVE_TARGET }}\n"
        ),
        &toolchain
    ));
}

/// `just bundles` really hands cargo ONE invocation with both targets, and the
/// foreign C compiler in its environment. On the Apple Silicon host the recipe
/// accepts, with a fake `cargo` recording its argv, a fake musl compiler and a
/// fake pinned `llvm-readobj`: the run ends at the static proof (no binary was
/// built), which is after the only cargo call. Elsewhere the recipe refuses
/// before it builds, and there is nothing to record.
#[test]
fn bundles_hands_cargo_one_build_with_both_targets_and_the_musl_compiler() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    if !(cfg!(target_os = "macos") && cfg!(target_arch = "aarch64")) {
        return;
    }
    let base = super::cli::OwnedScratch::root("gate", "bundles2t");
    let work = base.join("work");
    let bin = base.join("bin");
    let sysroot = base.join("sysroot");
    let readobj_dir = sysroot.join("lib/rustlib/aarch64-apple-darwin/bin");
    for dir in [&work, &bin, &readobj_dir] {
        std::fs::create_dir_all(dir).unwrap_or_else(|why| panic!("{}: {why}", dir.display()));
    }
    std::fs::write(work.join("justfile"), read(&root().join("justfile")))
        .and_then(|()| {
            std::fs::write(
                work.join("Cargo.toml"),
                "[package]\nname = \"ae\"\nversion = \"0.0.0\"\n",
            )
        })
        .unwrap_or_else(|why| panic!("the fixture checkout: {why}"));
    let script = |path: PathBuf, body: String| {
        std::fs::write(&path, format!("#!/bin/sh\n{body}"))
            .and_then(|()| std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)))
            .unwrap_or_else(|why| panic!("{}: {why}", path.display()));
    };
    script(
        bin.join("cargo"),
        format!(
            "echo \"$@\" >>{0}/cargo.argv\necho \"CC=$CC_x86_64_unknown_linux_musl\" >>{0}/cargo.env\n",
            base.display()
        ),
    );
    script(bin.join("x86_64-unknown-linux-musl-gcc"), String::new());
    script(
        bin.join("rustc"),
        format!(
            "case \"$1\" in --print) echo {} ;; -vV) echo 'host: aarch64-apple-darwin' ;; esac\n",
            sysroot.display()
        ),
    );
    script(readobj_dir.join("llvm-readobj"), String::new());
    let path = std::env::var_os("PATH").unwrap_or_default();
    let Some(just) = std::env::split_paths(&path)
        .map(|dir| dir.join("just"))
        .find(|file| file.is_file())
    else {
        panic!("just must be on PATH");
    };
    symlink(just, bin.join("just")).unwrap_or_else(|why| panic!("the linked just: {why}"));
    let run = Invocation::new("just")
        .env_cleared()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("HOME", &*base)
        .arg("bundles");
    let ran = raw::run(&run, &work, &base.join("out"), &base.join("err"))
        .unwrap_or_else(|why| panic!("the recipe runs: {why}"));
    let stderr = read(&base.join("err"));
    assert!(
        !matches!(ran.outcome(), ExitOutcome::Code(0)),
        "no binary was built, so the recipe cannot succeed: {stderr}"
    );
    assert!(
        stderr.contains("does not report a static binary"),
        "the recipe must reach the static proof, after its cargo call: {stderr}"
    );
    assert_eq!(
        read(&base.join("cargo.argv")).trim_end(),
        "build --release --locked --target aarch64-apple-darwin --target x86_64-unknown-linux-musl"
    );
    assert_eq!(
        read(&base.join("cargo.env")).trim_end(),
        "CC=x86_64-unknown-linux-musl-gcc"
    );
}

/// A canned nextest `JUnit` report in the shape nextest 0.9 writes: `tests` test
/// cases, the `i`-th taking `step * i` seconds, and a run `wall` seconds long.
fn junit_report(tests: u32, step: f64, wall: f64) -> String {
    use std::fmt::Write as _;
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"nextest-run\" tests=\"{tests}\" skipped=\"0\" failures=\"0\" errors=\"0\" uuid=\"u\" timestamp=\"2026-11-11T10:46:51.457+01:00\" time=\"{wall:.3}\">\n    <testsuite name=\"ae::it\" tests=\"{tests}\" skipped=\"0\" errors=\"0\" failures=\"0\">\n"
    );
    for i in 1..=tests {
        let _ = writeln!(
            out,
            "        <testcase name=\"mod::test_{i}\" classname=\"ae::it\" timestamp=\"2026-11-11T10:46:51.459+01:00\" time=\"{:.3}\"/>",
            step * f64::from(i)
        );
    }
    out + "    </testsuite>\n</testsuites>\n"
}

/// One `_tmux-isolated test` lane run of the real justfile with `cwd` as the
/// checkout it runs in. The fake `cargo` plays nextest (it drops `report` as the
/// `JUnit` file at `target/nextest/default/junit.xml`, and fails when `fail`) and
/// the doctests (it succeeds). `extra` is the filter a filtered run carries.
fn ledger_lane(
    base: &Path,
    cwd: &Path,
    report: Option<&str>,
    fail: bool,
    extra: &[&str],
) -> (ExitOutcome, String) {
    let cargo = "case \"$1\" in nextest)\n\
         mkdir -p target/nextest/default && rm -f target/nextest/default/junit.xml\n\
         [ ! -f \"$AE_TEST_TMPDIR/report.xml\" ] || cp \"$AE_TEST_TMPDIR/report.xml\" target/nextest/default/junit.xml\n\
         [ ! -f \"$AE_TEST_TMPDIR/fail\" ] || exit 1 ;; esac\n";
    let _ = std::fs::remove_file(base.join("report.xml"));
    let _ = std::fs::remove_file(base.join("fail"));
    if let Some(body) = report {
        std::fs::write(base.join("report.xml"), body)
            .unwrap_or_else(|why| panic!("the canned report: {why}"));
    }
    if fail {
        std::fs::write(base.join("fail"), "x").unwrap_or_else(|why| panic!("{why}"));
    }
    let mut lane = lane_over(base, Some("6"), cargo)
        .arg("--justfile")
        .arg(root().join("justfile"))
        .arg("--working-directory")
        .arg(cwd)
        .arg("_tmux-isolated")
        .arg("test");
    for arg in extra {
        lane = lane.arg(arg);
    }
    let ran = raw::run(&lane, cwd, &base.join("out"), &base.join("err"))
        .unwrap_or_else(|why| panic!("the lane runs: {why}"));
    (ran.outcome(), read(&base.join("err")))
}

/// A throwaway git checkout with one commit, so a linked worktree can hang off it.
fn ledger_checkout(base: &Path) -> PathBuf {
    let repo = base.join("repo");
    std::fs::create_dir_all(&repo).unwrap_or_else(|why| panic!("{why}"));
    for args in [
        &["init", "-q", "."][..],
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ][..],
    ] {
        let ran = Invocation::new("git")
            .env_cleared()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", base);
        let mut ran = ran;
        for arg in args {
            ran = ran.arg(arg);
        }
        let done = raw::run(&ran, &repo, &base.join("git.out"), &base.join("git.err"))
            .unwrap_or_else(|why| panic!("git runs: {why}"));
        assert!(
            matches!(done.outcome(), ExitOutcome::Code(0)),
            "git {args:?}: {}",
            read(&base.join("git.err"))
        );
    }
    repo
}

/// The data lines of a ledger file (everything but the `#` header).
fn ledger_entries(repo: &Path) -> Vec<String> {
    std::fs::read_to_string(repo.join(".local/gate-ledger.tsv"))
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// A whole green gate prints its test-seconds, its thread-seconds, the 30 slowest
/// tests and the delta against the previous entry, and appends ONE line to the
/// main checkout's `.local/gate-ledger.tsv`, from a linked worktree too.
#[test]
fn a_whole_green_gate_reports_its_test_seconds_and_appends_one_ledger_line() {
    let base = super::cli::OwnedScratch::root("gate", "ledger");
    let repo = ledger_checkout(&base);
    let green = |outcome: &ExitOutcome| matches!(outcome, ExitOutcome::Code(0));

    // First whole run: 40 tests of 0.5 s ... 20.0 s = 410.0 test-seconds, 120 s of
    // wall on 6 threads (the fake jobs-budget says 6) = 720.0 thread-seconds.
    let (outcome, stderr) = ledger_lane(
        &base,
        &repo,
        Some(&junit_report(40, 0.5, 120.0)),
        false,
        &[],
    );
    assert!(green(&outcome), "{stderr}");
    assert!(
        stderr.contains("gate ledger: 40 tests, wall 120.0 s on 6 threads"),
        "{stderr}"
    );
    assert!(
        stderr.contains("test-seconds 410.0, thread-seconds 720.0"),
        "{stderr}"
    );
    assert!(stderr.contains("first entry"), "{stderr}");
    let listed: Vec<&str> = stderr
        .lines()
        .filter(|line| line.contains("ae::it  mod::test_"))
        .collect();
    assert_eq!(listed.len(), 30, "the 30 slowest, no more: {stderr}");
    assert!(
        listed[0].contains("20.000") && listed[0].ends_with("mod::test_40"),
        "slowest first: {listed:?}"
    );
    assert!(
        listed[29].ends_with("mod::test_11"),
        "the 30th slowest: {listed:?}"
    );
    let first = ledger_entries(&repo);
    assert_eq!(first.len(), 1, "{first:?}");
    let fields: Vec<&str> = first[0].split('\t').collect();
    assert_eq!(fields.len(), 8, "{first:?}");
    assert_eq!(
        fields[3..],
        ["6", "40", "120.0", "410.0", "720.0"],
        "{first:?}"
    );

    // Second whole run, from a LINKED worktree: the delta against the first and the
    // entry in the MAIN checkout's ledger.
    let linked = base.join("linked");
    let add = Invocation::new("git")
        .env_cleared()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &*base)
        .arg("worktree")
        .arg("add")
        .arg("-q")
        .arg(&linked)
        .arg("-b")
        .arg("wt");
    let done = raw::run(&add, &repo, &base.join("git.out"), &base.join("git.err"))
        .unwrap_or_else(|why| panic!("git runs: {why}"));
    assert!(green(&done.outcome()), "{}", read(&base.join("git.err")));
    let (outcome, stderr) = ledger_lane(
        &base,
        &linked,
        Some(&junit_report(42, 0.5, 130.0)),
        false,
        &[],
    );
    assert!(green(&outcome), "{stderr}");
    assert!(
        stderr.contains("tests +2, wall +10.0 s (+8.3%), test-seconds +41.5 (+10.1%), thread-seconds +60.0 (+8.3%)"),
        "{stderr}"
    );
    assert_eq!(ledger_entries(&repo).len(), 2, "the main checkout's ledger");
    assert!(
        !linked.join(".local/gate-ledger.tsv").exists(),
        "never the worktree's own"
    );
}

/// A filtered run, a red run, a missing or unreadable report, a report the lane cannot
/// remove and an unusable `.local` write nothing to the ledger and never change the lane's
/// status: the ledger is a report, never a failure.
#[test]
fn a_filtered_or_red_gate_and_a_broken_report_never_write_the_ledger_or_fail_the_lane() {
    let base = super::cli::OwnedScratch::root("gate", "ledger-quiet");
    let repo = ledger_checkout(&base);
    let green = |outcome: &ExitOutcome| matches!(outcome, ExitOutcome::Code(0));
    let (outcome, stderr) = ledger_lane(
        &base,
        &repo,
        Some(&junit_report(40, 0.5, 120.0)),
        false,
        &[],
    );
    assert!(green(&outcome), "{stderr}");

    // Nothing is written by a filtered run, a red run, a run with no report, a
    // report that names no test, and the lane still ends as nextest did.
    let before = ledger_entries(&repo);
    let (outcome, stderr) = ledger_lane(
        &base,
        &repo,
        Some(&junit_report(40, 0.5, 120.0)),
        false,
        &["-E", "test(=x)"],
    );
    assert!(
        green(&outcome) && !stderr.contains("gate ledger"),
        "a filtered run is silent: {stderr}"
    );
    let (outcome, stderr) =
        ledger_lane(&base, &repo, Some(&junit_report(40, 0.5, 120.0)), true, &[]);
    assert!(
        matches!(outcome, ExitOutcome::Code(code) if code != 0) && !stderr.contains("gate ledger"),
        "{stderr}"
    );
    let (outcome, stderr) = ledger_lane(&base, &repo, None, false, &[]);
    assert!(
        green(&outcome) && stderr.contains("note: gate ledger skipped: no JUnit report"),
        "{stderr}"
    );
    for broken in [
        "not xml at all\n",
        &junit_report(0, 1.0, 5.0),
        "<testsuites time=\"x\">\n<testcase name=\"a\" time=\"1\"/>\n",
    ] {
        let (outcome, stderr) = ledger_lane(&base, &repo, Some(broken), false, &[]);
        assert!(
            green(&outcome) && stderr.contains("note: gate ledger skipped"),
            "{broken:?}: {stderr}"
        );
    }
    assert_eq!(ledger_entries(&repo), before, "none of those wrote a line");

    // An old report the lane cannot remove (a directory where the file goes) turns
    // the ledger off with a note and leaves the gate green.
    let blocked = repo.join("target/nextest/default/junit.xml");
    let _ = std::fs::remove_file(&blocked);
    std::fs::create_dir_all(&blocked).unwrap_or_else(|why| panic!("{why}"));
    let (outcome, stderr) = ledger_lane(
        &base,
        &repo,
        Some(&junit_report(40, 0.5, 120.0)),
        false,
        &[],
    );
    assert!(green(&outcome), "{stderr}");
    assert!(
        stderr.contains("note: gate ledger off: the old report"),
        "{stderr}"
    );
    assert!(!stderr.contains("test-seconds"), "{stderr}");
    assert_eq!(
        ledger_entries(&repo),
        before,
        "a blocked report wrote no line"
    );
    std::fs::remove_dir_all(&blocked).unwrap_or_else(|why| panic!("{why}"));

    // An unusable `.local` (a plain file where the directory goes) is a note, not
    // a failure.
    std::fs::remove_dir_all(repo.join(".local")).unwrap_or_else(|why| panic!("{why}"));
    std::fs::write(repo.join(".local"), "in the way").unwrap_or_else(|why| panic!("{why}"));
    let (outcome, stderr) = ledger_lane(
        &base,
        &repo,
        Some(&junit_report(40, 0.5, 120.0)),
        false,
        &[],
    );
    assert!(green(&outcome), "{stderr}");
    assert!(
        stderr.contains("note: gate ledger entry not written"),
        "{stderr}"
    );
    assert!(
        stderr.contains("test-seconds 410.0"),
        "the report is still printed: {stderr}"
    );
}

/// The ledger's two ends agree: nextest is told to write a `JUnit` report for the
/// default profile, and the lane reads `nextest/default/junit.xml`, removing the
/// previous file before the run so a crashed nextest cannot be read as a new one.
fn ledger_inputs_ok(justfile: &str, nextest: &str) -> bool {
    let lines = recipe_text(justfile, "_tmux-isolated lane *args:");
    let position = |needle: &str| lines.iter().position(|line| line.contains(needle));
    let table: Vec<&str> = toml_table(nextest, "[profile.default.junit]")
        .into_iter()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let (Some(remove), Some(run), Some(report)) = (
        position("rm -f \"$junit\""),
        position("detached nice -n 10 cargo nextest run --locked --all-features"),
        position("ledger_report \"$junit\" \"$NEXTEST_TEST_THREADS\" ||"),
    ) else {
        return false;
    };
    let Some(doctest) = position("cargo test --doc --locked --all-features") else {
        return false;
    };
    table == ["path = \"junit.xml\""]
        && position("junit=\"${CARGO_TARGET_DIR:-target}/nextest/default/junit.xml\"")
            .is_some_and(|set| set < remove)
        && remove < run
        && doctest < report
        && lines.iter().any(|line| line == "{{ LEDGER }}")
}

#[test]
fn the_ledger_reads_the_junit_report_nextest_is_told_to_write() {
    let justfile = read(&root().join("justfile"));
    let nextest = read(&root().join(".config/nextest.toml"));
    assert!(
        ledger_inputs_ok(&justfile, &nextest),
        "the lane must delete the old report, run nextest, then read target/nextest/default/junit.xml \
         after the doctests, and .config/nextest.toml must write it"
    );
    // RED — no report is configured, the wrong file, the report read before the
    // doctests, the old report never removed.
    assert!(!ledger_inputs_ok(
        &justfile,
        &nextest.replace("[profile.default.junit]", "[profile.default.report]")
    ));
    assert!(!ledger_inputs_ok(
        &justfile,
        &nextest.replace("path = \"junit.xml\"", "path = \"other.xml\"")
    ));
    assert!(!ledger_inputs_ok(
        &justfile.replace(
            "                rm -f \"$junit\" 2>/dev/null || { echo \"note: gate ledger off: the old report $junit cannot be removed\" >&2; junit=\"\"; }\n",
            ""
        ),
        &nextest
    ));
    let late = justfile
        .replace("                ledger_report \"$junit\" \"$NEXTEST_TEST_THREADS\" || echo \"note: gate ledger skipped\" >&2\n", "")
        .replace(
            "                detached nice -n 10 cargo nextest run --locked --all-features\n",
            "                ledger_report \"$junit\" \"$NEXTEST_TEST_THREADS\" || true\n                detached nice -n 10 cargo nextest run --locked --all-features\n",
        );
    assert!(!ledger_inputs_ok(&late, &nextest));
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
/// A dead test's registered root is removable even when SIGKILL left a
/// non-empty directory at 0555. Permission repair stays inside that root:
/// directory and file links preserve the outside targets' modes and bytes.
#[test]
fn registered_readonly_residue_is_reaped_without_following_links() {
    use std::os::unix::fs::PermissionsExt as _;

    let base = super::cli::OwnedScratch::root("gate", "ro-reap");
    let outside = base.join("outside");
    let outside_file = outside.join("kept");
    std::fs::create_dir_all(&outside).expect("the outside target directory");
    std::fs::write(&outside_file, "outside target bytes\n").expect("outside target bytes");
    std::fs::set_permissions(&outside_file, std::fs::Permissions::from_mode(0o444))
        .expect("the outside file mode");
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o555))
        .expect("the outside directory mode");
    let _outside_guard = Unwritable(vec![outside.clone()]);
    let outside_mode = std::fs::metadata(&outside)
        .expect("outside directory metadata")
        .permissions()
        .mode();
    let outside_file_mode = std::fs::metadata(&outside_file)
        .expect("outside file metadata")
        .permissions()
        .mode();

    let cargo = base.join("bin").join("cargo");
    std::fs::create_dir_all(base.join("bin")).expect("the fake cargo directory");
    std::fs::write(
        &cargo,
        "#!/bin/sh\n\
         [ \"$1\" = nextest ] || exit 0\n\
         unset AE_GATE_SCRATCH_KILL_SERVER\n\
         \"$AE_GATE_EXE\" --exact gate::scratch_sigkill_child >/dev/null 2>&1\n\
         owner=$(cat \"$AE_GATE_SCRATCH_KILL_REPORT\") || exit 1\n\
         held=\"$AE_TEST_TMPDIR/ae-it-$owner/killed\"\n\
         mkdir -p \"$TMUX_TMPDIR/.ae-parity-fixtures/$owner\" || exit 1\n\
         ln -s \"$held\" \"$TMUX_TMPDIR/.ae-parity-fixtures/$owner/0\" || exit 1\n\
         printf 'registered readonly residue\\n' >\"$held/held\" || exit 1\n\
         ln -s \"$AE_GATE_OUTSIDE_DIR\" \"$held/outside-dir\" || exit 1\n\
         ln -s \"$AE_GATE_OUTSIDE_FILE\" \"$held/outside-file\" || exit 1\n\
         chmod 0555 \"$held\" || exit 1\n\
         exit 0\n",
    )
    .expect("the fake cargo leaves registered readonly residue");
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755))
        .expect("an executable fake cargo");
    let path = std::env::var("PATH").unwrap_or_default();
    let report = base.join("dead-owner");
    let swept = raw::run(
        &Invocation::new("just")
            .arg("_tmux-isolated")
            .arg("test")
            .env("PATH", format!("{}:{path}", base.join("bin").display()))
            .env("HOME", &*base)
            .env(
                "AE_GATE_EXE",
                std::env::current_exe().expect("the test binary"),
            )
            .env("AE_GATE_SCRATCH_KILL_BASE", &*base)
            .env("AE_GATE_SCRATCH_KILL_REPORT", &report)
            .env("AE_GATE_OUTSIDE_DIR", &outside)
            .env("AE_GATE_OUTSIDE_FILE", &outside_file)
            .env("TMPDIR", &*base)
            .env("AE_TEST_TMPDIR", &*base),
        &root(),
        &base.join("readonly-out"),
        &base.join("readonly-err"),
    );
    let owner = read(&report).trim().to_owned();
    assert!(
        owner.parse::<u32>().is_ok(),
        "the killed child's numeric pid"
    );
    let dead_root = base.join(format!("ae-it-{owner}"));
    let _residue_guard = Unwritable(vec![dead_root.clone()]);
    let swept = swept.expect("the lane runs to teardown");
    let stderr = read(&base.join("readonly-err"));
    assert!(
        matches!(swept.outcome(), ExitOutcome::Code(0)),
        "registered 0555 residue must not fail an all-green lane: {stderr}"
    );
    assert!(
        !dead_root.exists(),
        "registered dead-owner root must be reaped"
    );
    assert_eq!(read(&outside_file), "outside target bytes\n");
    assert_eq!(
        std::fs::metadata(&outside)
            .expect("outside directory preserved")
            .permissions()
            .mode(),
        outside_mode
    );
    assert_eq!(
        std::fs::metadata(&outside_file)
            .expect("outside file preserved")
            .permissions()
            .mode(),
        outside_file_mode
    );
}

fn buildtune_profile<'a>(
    manifest: &'a str,
    table: &str,
) -> std::collections::BTreeMap<&'a str, &'a str> {
    let mut active = false;
    let mut values = std::collections::BTreeMap::new();
    for line in manifest.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') {
            active = line == table;
        } else if active && let Some((key, value)) = line.split_once('=') {
            values.insert(key.trim(), value.trim());
        }
    }
    values
}

/// The only dev-profile package overrides: the test-only parser crates of the
/// source view (docs/history.md section 11), `opt-level = 3` and no other key.
const BUILDTUNE_OPTIMISED_DEPS: [&str; 3] = ["proc-macro2", "syn", "unicode-ident"];

/// Every `[profile.dev.package.*]` table is one of [`BUILDTUNE_OPTIMISED_DEPS`],
/// each present once, holding `opt-level = 3` alone. A wildcard, another
/// package, a second key or another level is refused, and so is a package
/// override written as a `package` key inside `[profile.dev]` or
/// `[profile.debugging]`, which the table scan would not see.
fn buildtune_package_overrides_ok(manifest: &str) -> bool {
    let tables: Vec<&str> = manifest
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|line| line.starts_with("[profile.dev.package."))
        .collect();
    let optimised = std::collections::BTreeMap::from([("opt-level", "3")]);
    tables.len() == BUILDTUNE_OPTIMISED_DEPS.len()
        && BUILDTUNE_OPTIMISED_DEPS.iter().all(|name| {
            let table = format!("[profile.dev.package.{name}]");
            tables.contains(&table.as_str()) && buildtune_profile(manifest, &table) == optimised
        })
        && ["[profile.dev]", "[profile.debugging]"]
            .iter()
            .all(|table| {
                !buildtune_profile(manifest, table)
                    .keys()
                    .any(|key| key.starts_with("package"))
            })
}

fn buildtune_profiles_ok(manifest: &str) -> bool {
    let dev = buildtune_profile(manifest, "[profile.dev]");
    let debugging = buildtune_profile(manifest, "[profile.debugging]");
    let release = buildtune_profile(manifest, "[profile.release]");
    let release_contract = std::collections::BTreeMap::from([
        ("strip", "\"symbols\""),
        ("lto", "\"thin\""),
        ("codegen-units", "1"),
        ("panic", "\"abort\""),
    ]);
    dev.get("debug") == Some(&"\"line-tables-only\"")
        && debugging.get("inherits") == Some(&"\"dev\"")
        && debugging.get("debug") == Some(&"\"full\"")
        && buildtune_package_overrides_ok(manifest)
        && release == release_contract
}

#[test]
fn buildtune_profiles_keep_line_tables_full_debug_opt_in_and_release_contract() {
    let good = "[profile.dev]\ndebug = \"line-tables-only\"\n\
                [profile.dev.package.proc-macro2]\nopt-level = 3\n\
                [profile.dev.package.syn]\nopt-level = 3\n\
                [profile.dev.package.unicode-ident]\nopt-level = 3\n\
                [profile.debugging]\ninherits = \"dev\"\ndebug = \"full\"\n\
                [profile.release]\nstrip = \"symbols\"\nlto = \"thin\"\n\
                codegen-units = 1\npanic = \"abort\"\n";
    assert!(buildtune_profiles_ok(good));
    for bad in [
        good.replace("\"line-tables-only\"", "0"),
        good.replace("debug = \"full\"", "debug = \"line-tables-only\""),
        good.replace("inherits = \"dev\"", "inherits = \"release\""),
        good.replace("lto = \"thin\"", "lto = false"),
        format!("{good}\n[profile.dev.package.\"*\"]\ndebug = 0\n"),
    ] {
        assert!(
            !buildtune_profiles_ok(&bad),
            "the profile guard accepted broken input: {bad}"
        );
    }
    // The same breaks, made in the real manifest text: each is the shipped
    // Cargo.toml with ONE edit, and the edit must land, or the control is vacuous.
    let manifest = read(&root().join("Cargo.toml"));
    assert!(
        buildtune_profiles_ok(&manifest),
        "dev needs line tables, debugging needs full info including deps, release stays unchanged"
    );
    let syn = "[profile.dev.package.syn]\nopt-level = 3\n";
    for (what, edit) in [
        (
            "a wildcard override",
            manifest.replace(
                syn,
                &format!("{syn}\n[profile.dev.package.\"*\"]\nopt-level = 3\n"),
            ),
        ),
        (
            "another package",
            manifest.replace(
                syn,
                &format!("{syn}\n[profile.dev.package.ureq]\nopt-level = 3\n"),
            ),
        ),
        (
            "a second key",
            manifest.replace(syn, "[profile.dev.package.syn]\nopt-level = 3\ndebug = 0\n"),
        ),
        (
            "another level",
            manifest.replace(syn, "[profile.dev.package.syn]\nopt-level = 2\n"),
        ),
        ("a missing table", manifest.replace(syn, "")),
        (
            "a quoted package name",
            manifest.replace(syn, "[profile.dev.package.\"syn\"]\nopt-level = 3\n"),
        ),
        (
            "an override as a dotted key",
            manifest.replace(
                "[profile.dev]\ndebug = \"line-tables-only\"\n",
                "[profile.dev]\ndebug = \"line-tables-only\"\npackage.\"*\".debug = 0\n",
            ),
        ),
    ] {
        assert_ne!(edit, manifest, "the edit for {what} changed nothing");
        assert!(
            !buildtune_profiles_ok(&edit),
            "the profile guard accepted {what} in the real manifest"
        );
    }
}

/// Observe the real guard call sites, with only the external Cargo executable
/// replaced. It emits no diagnostics: the guard must reject it as non-vacuous,
/// while its recorded environment/argv prove the changed subprocess behavior.
#[test]
fn buildtune_force_warn_children_override_incremental_without_changing_scope() {
    use std::os::unix::fs::PermissionsExt as _;

    let scratch = super::cli::OwnedScratch::root("gate", "buildtune-guard-env");
    let fake_cargo = scratch.join("cargo");
    std::fs::write(
        &fake_cargo,
        "#!/bin/sh\nprintf '%s\\n' \"${CARGO_INCREMENTAL-unset}\" \"$@\" > \"$AE_BUILDTUNE_GUARD_CAPTURE\"\n",
    )
    .expect("the owned Cargo invocation observer");
    std::fs::set_permissions(&fake_cargo, std::fs::Permissions::from_mode(0o755))
        .expect("the invocation observer is executable");

    let cases = [
        (
            "doors::the_capability_boundary_holds_against_any_lint_relaxation",
            "force-warn-guard",
            ["--all-targets"].as_slice(),
            "clippy::disallowed_types",
            "the force-warn probe reported no `Command` anywhere",
        ),
        (
            "phase3::criterion_3_the_places_this_crate_can_read_the_world_are_the_inventoried_ones",
            "world-read-guard",
            ["--lib", "--bins"].as_slice(),
            "clippy::disallowed_methods",
            "clippy world-read probe failed: no compiler messages",
        ),
    ];
    for (test, target, scope, lint, rejection) in cases {
        let capture = scratch.join(format!("{target}-capture"));
        let out = scratch.join(format!("{target}-out"));
        let err = scratch.join(format!("{target}-err"));
        let invocation =
            Invocation::new(std::env::current_exe().expect("the integration test executable"))
                .arg("--exact")
                .arg(test)
                .arg("--nocapture")
                .env("CARGO", &fake_cargo)
                .env("CARGO_INCREMENTAL", "1")
                .env("AE_BUILDTUNE_GUARD_CAPTURE", &capture);
        let status =
            raw::run(&invocation, &scratch, &out, &err).expect("the real guard child starts");
        let transcript = format!("{}\n{}", read(&out), read(&err));
        assert!(
            matches!(status.outcome(), ExitOutcome::Code(101)) && transcript.contains(rejection),
            "the guard must reject the observer's empty diagnostics: {transcript}"
        );
        let recorded = read(&capture);
        let lines: Vec<&str> = recorded.lines().collect();
        assert_eq!(
            lines.first().copied(),
            Some("0"),
            "{test} must override an inherited CARGO_INCREMENTAL=1"
        );
        let isolated_target = root().join("target").join(target);
        // This pins the full guard argv; intentional scope changes update this contract.
        // The Command guard asks ALL targets; the world-read guard asks the
        // product `--lib --bins` only, so no `cfg(test)` item is compiled.
        let expected: Vec<&str> = ["clippy", "--quiet", "--locked"]
            .into_iter()
            .chain(scope.iter().copied())
            .chain([
                "--all-features",
                "--message-format=json",
                "--target-dir",
                isolated_target
                    .to_str()
                    .expect("the repository path is UTF-8"),
                "--",
                "--force-warn",
                lint,
            ])
            .collect();
        assert_eq!(
            &lines[1..],
            expected.as_slice(),
            "the isolated force-warn lint scope is unchanged"
        );
    }
}

/// A real ae library frame calls a real dependency frame before the panic.
/// The callback is dynamic: these frames are compiled in their own crates,
/// not monomorphized in this integration test with its debug settings.
#[test]
fn buildtune_backtrace_probe_child() {
    if std::env::var_os("AE_BUILDTUNE_BACKTRACE_CHILD").is_none() {
        return;
    }
    let mut splitter = ae::board::Splitter::new();
    splitter.feed_each(b"backtrace probe\n", &mut |_| {
        let buffer =
            ratatui_core::buffer::Buffer::empty(ratatui_core::layout::Rect::new(0, 0, 1, 1));
        let _ = buffer.index_of(2, 0);
    });
    panic!("the out-of-bounds dependency probe must panic before returning");
}

/// A symbol name or panic-location header is insufficient: require a resolved frame.
fn buildtune_has_source_line(trace: &str, source: &str) -> bool {
    trace.lines().any(|line| {
        let Some(frame) = line.trim_start().strip_prefix("at ") else {
            return false;
        };
        frame.split_once(source).is_some_and(|(_, suffix)| {
            suffix.strip_prefix(':').is_some_and(|location| {
                location
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_digit())
            })
        })
    })
}

#[test]
fn buildtune_panic_backtraces_resolve_ae_and_dependency_source_lines() {
    assert!(buildtune_has_source_line(
        "at /repo/src/board.rs:703:4",
        "src/board.rs"
    ));
    assert!(!buildtune_has_source_line(
        "ae::board::Splitter::feed_each",
        "src/board.rs"
    ));
    assert!(!buildtune_has_source_line(
        "at /repo/src/board.rs",
        "src/board.rs"
    ));
    assert!(!buildtune_has_source_line(
        "at /repo/src/board.rs:unknown",
        "src/board.rs"
    ));
    assert!(!buildtune_has_source_line(
        "thread 'probe' panicked at /repo/src/buffer/buffer.rs:251:13:",
        "src/buffer/buffer.rs"
    ));

    let scratch = super::cli::OwnedScratch::root("gate", "buildtune-backtrace");
    let out = scratch.join("out");
    let err = scratch.join("err");
    let invocation =
        Invocation::new(std::env::current_exe().expect("the integration test executable"))
            .arg("--exact")
            .arg("gate::buildtune_backtrace_probe_child")
            .arg("--nocapture")
            .env("AE_BUILDTUNE_BACKTRACE_CHILD", "1")
            .env("RUST_BACKTRACE", "full");
    let status =
        raw::run(&invocation, &scratch, &out, &err).expect("the dependency panic probe starts");
    let trace = read(&err);
    assert!(
        matches!(status.outcome(), ExitOutcome::Code(101)),
        "the child must report a failed test, not a missing executable: {trace}"
    );
    assert!(
        trace.contains("index outside of buffer"),
        "the real dependency panic: {trace}"
    );
    for source in ["src/board.rs", "src/buffer/buffer.rs", "tests/it/gate.rs"] {
        assert!(
            buildtune_has_source_line(&trace, source),
            "panic backtrace lost file:line for {source}: {trace}"
        );
    }
}

/// The six trees `just rust-prune` cleans, relative to a checkout. The fixture puts an
/// `ae-unit` (what a clean removes) and an `other/keep` (what it must not) in each.
const PRUNE_TREES: [&str; 6] = [
    "target/debug",
    "target/release",
    "target/aarch64-apple-darwin/release",
    "target/x86_64-unknown-linux-musl/release",
    "target/force-warn-guard/debug",
    "target/world-read-guard/debug",
];

/// The labels the report and the apply run print for them, in order.
const PRUNE_LABELS: [&str; 6] = [
    "debug",
    "release",
    "native-release",
    "musl-release",
    "force-warn-guard",
    "world-read-guard",
];

/// A `cargo` that logs every call as `<cwd>|<argv>` and plays the two verbs the recipe uses.
/// `metadata` finds `target-dir` / `build-dir` in the nearest `.cargo/config.toml` at or above ITS
/// working directory, as cargo does. `clean` lists the `ae-*` entries under the tree it is
/// pointed at, one path per line (a `--target` clean also lists the host `release` units, and
/// a `--target-dir` one wants its `CACHEDIR.TAG`, as real cargo 1.97 does); a dry run prints
/// them, plus the lines of the `lie` knob when its first line is part of the tree path, and a
/// real run removes them. Knob files in `$AE_FIX`: `metadata.fail`, `clean.fail`, `lie`, and for a
/// dry run's stderr `stderr.extra` (lines added before the Summary), `summary.text` (the Summary
/// lines, instead of its own) and `summary.none` (no Summary at all).
const FAKE_CARGO: &str = r#"#!/bin/sh
printf '%s|%s\n' "$(pwd -P)" "$*" >>"$AE_FIX/cargo.log"
walk() {
    d=$(pwd -P)
    while :; do
        if [ -f "$d/.cargo/config.toml" ]; then
            v=$(sed -n "s/^$1 *= *\"\\(.*\\)\"\$/\\1/p" "$d/.cargo/config.toml" | head -1)
            if [ -n "$v" ]; then printf '%s\n' "$v"; return; fi
        fi
        [ "$d" = / ] && break
        d=$(dirname "$d")
    done
    printf '%s/target\n' "$(pwd -P)"
}
cmd=$1
shift
case "$cmd" in
metadata)
    [ ! -f "$AE_FIX/metadata.fail" ] || exit 1
    printf '{"packages":[],"workspace_members":[],"target_directory":"%s","build_directory":"%s","version":1}\n' "$(walk target-dir)" "$(walk build-dir)"
    ;;
clean)
    dry=no; tdir=""; tgt=""; prof=dev
    while [ $# -gt 0 ]; do
        case "$1" in
            --dry-run) dry=yes ;;
            --target-dir) tdir=$2; shift ;;
            --target) tgt=$2; shift ;;
            --profile) prof=$2; shift ;;
            --manifest-path) shift ;;
        esac
        shift
    done
    if [ -n "$tdir" ] && [ ! -f "$tdir/CACHEDIR.TAG" ]; then
        echo "error: cannot clean $tdir: missing or invalid CACHEDIR.TAG file" >&2
        exit 101
    fi
    base=${tdir:-$(pwd -P)/target}
    if [ -n "$tgt" ]; then dirs="$base/release $base/$tgt/release"; elif [ "$prof" = release ]; then dirs=$base/release; else dirs=$base/debug; fi
    list=$(for dir in $dirs; do find -P "$dir" -name 'ae-*' -prune -print 2>/dev/null; done)
    n=$(printf '%s\n' "$list" | grep -c .)
    if [ "$dry" = yes ]; then
        [ ! -f "$AE_FIX/clean.fail" ] || exit 1
        [ -z "$list" ] || printf '%s\n' "$list"
        if [ -f "$AE_FIX/lie" ]; then
            case "$dirs" in *"$(head -1 "$AE_FIX/lie")"*) sed 1d "$AE_FIX/lie" ;; esac
        fi
        [ ! -f "$AE_FIX/stderr.extra" ] || cat "$AE_FIX/stderr.extra" >&2
        if [ -f "$AE_FIX/summary.text" ]; then
            cat "$AE_FIX/summary.text" >&2
        elif [ -f "$AE_FIX/summary.none" ]; then
            :
        elif [ "$n" = 0 ]; then
            echo "     Summary 0 files" >&2
        else
            echo "     Summary $n files, 1B total" >&2
        fi
        echo "warning: no files deleted due to --dry-run" >&2
    else
        printf '%s\n' "$list" | while IFS= read -r p; do [ -z "$p" ] || rm -rf "$p"; done
        echo "     Removed $n files, 1B total" >&2
    fi
    ;;
esac
"#;

/// The first `tool` in `dirs`, then on `PATH`.
fn tool_path(tool: &str, dirs: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    dirs.iter()
        .map(PathBuf::from)
        .chain(std::env::split_paths(&path))
        .map(|dir| dir.join(tool))
        .find(|file| file.is_file())
}

/// Write `text` to `path`, creating its parent directories.
fn put(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|why| panic!("{}: {why}", parent.display()));
    }
    std::fs::write(path, text).unwrap_or_else(|why| panic!("{}: {why}", path.display()));
}

/// One finished command: its exit code (`-1` when a signal ended it) and both streams.
struct Ran {
    code: i32,
    out: String,
    err: String,
}

/// A throwaway checkout for the `rust-prune` pins: a git repository `tree` whose six target
/// trees each hold an `ae-unit` and an `other/keep`, beside a `bin` that is the recipe's WHOLE
/// `PATH`. `bin` links the real `just`, `git`, `bash` and the few tools the recipe and the fake
/// cargo use, and holds the fake `cargo` and a `ps` that is the real one unless `ps.fail` or
/// `ps.extra` (rows appended to the table) say otherwise. There is no `lsof` until a test adds
/// one. Knobs are files in `base`, which the fakes read through `$AE_FIX`.
struct Prune {
    base: super::cli::OwnedScratch,
    tree: PathBuf,
}

impl Prune {
    fn new(tag: &str) -> Self {
        let base = super::cli::OwnedScratch::root("gate", tag);
        std::fs::create_dir_all(base.join("tree")).unwrap_or_else(|why| panic!("the tree: {why}"));
        let tree = std::fs::canonicalize(base.join("tree"))
            .unwrap_or_else(|why| panic!("the tree's real path: {why}"));
        let fixture = Self { base, tree };
        fixture.stock_bin();
        put(
            &fixture.tree.join("Cargo.toml"),
            "[package]\nname = \"ae\"\nversion = \"0.0.0\"\n",
        );
        for tree in PRUNE_TREES {
            put(&fixture.tree.join(tree).join("ae-unit/payload"), "x\n");
            put(&fixture.tree.join(tree).join("other/keep"), "x\n");
        }
        for guard in ["force-warn-guard", "world-read-guard"] {
            put(
                &fixture.tree.join("target").join(guard).join("CACHEDIR.TAG"),
                "Signature: 8a477f597d28d172789f06886806bc55\n",
            );
        }
        let init = fixture.exec(&fixture.tree, "git", &["init", "-q"], &[]);
        assert_eq!(init.code, 0, "git init: {}", init.err);
        fixture
    }

    fn bin(&self) -> PathBuf {
        self.base.join("bin")
    }

    fn stock_bin(&self) {
        std::fs::create_dir_all(self.bin()).unwrap_or_else(|why| panic!("the fakes' dir: {why}"));
        for (tool, system) in [
            ("just", false),
            ("git", false),
            // `just` runs the justfile's start-snapshot backtick (`RUN_START`) through `sh`
            // before any recipe, so even this sparse world needs one.
            ("sh", true),
            ("bash", true),
            ("sed", true),
            ("head", true),
            ("dirname", true),
            ("find", true),
            ("grep", true),
            ("rm", true),
            ("cat", true),
        ] {
            let dirs: &[&str] = if system { &["/bin", "/usr/bin"] } else { &[] };
            let real = tool_path(tool, dirs).unwrap_or_else(|| panic!("{tool} must be installed"));
            std::os::unix::fs::symlink(real, self.bin().join(tool))
                .unwrap_or_else(|why| panic!("the linked {tool}: {why}"));
        }
        let ps = tool_path("ps", &["/bin", "/usr/bin"])
            .unwrap_or_else(|| panic!("ps must be installed"));
        self.install(
            "ps",
            &format!(
                "#!/bin/sh\n[ ! -f \"$AE_FIX/ps.fail\" ] || exit 1\n{} \"$@\"\n\
                 [ ! -f \"$AE_FIX/ps.extra\" ] || cat \"$AE_FIX/ps.extra\"\nexit 0\n",
                ps.display()
            ),
        );
        self.install("cargo", FAKE_CARGO);
    }

    /// An executable `name` in `bin`, replacing whatever is there.
    fn install(&self, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt as _;
        let file = self.bin().join(name);
        let _ = std::fs::remove_file(&file);
        put(&file, body);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|why| panic!("the fake {name}: {why}"));
    }

    /// A fake `lsof` that prints the `lsof.out` knob, or fails when `lsof.fail` exists.
    fn install_fake_lsof(&self) {
        self.install(
            "lsof",
            "#!/bin/sh\n[ ! -f \"$AE_FIX/lsof.fail\" ] || exit 1\ncat \"$AE_FIX/lsof.out\"\n",
        );
    }

    fn knob(&self, name: &str, text: &str) {
        put(&self.base.join(name), text);
    }

    fn clear(&self, name: &str) {
        let _ = std::fs::remove_file(self.base.join(name));
    }

    /// The command's environment: nothing inherited, `bin` the only `PATH` entry.
    fn environment(&self, invocation: Invocation) -> Invocation {
        invocation
            .env_cleared()
            .env("PATH", self.bin())
            .env("HOME", &*self.base)
            .env("TMPDIR", &*self.base)
            .env("AE_FIX", &*self.base)
            .env("GIT_CONFIG_GLOBAL", self.base.join("gitconfig"))
    }

    fn finish(&self, invocation: &Invocation, cwd: &Path, name: &str) -> Ran {
        let out = self.base.join(format!("capture-{name}.out"));
        let err = self.base.join(format!("capture-{name}.err"));
        let status = raw::run(invocation, cwd, &out, &err)
            .unwrap_or_else(|why| panic!("{name} must be runnable: {why}"));
        Ran {
            code: match status.outcome() {
                ExitOutcome::Code(code) => code,
                ExitOutcome::Signalled => -1,
            },
            out: std::fs::read_to_string(&out).unwrap_or_default(),
            err: std::fs::read_to_string(&err).unwrap_or_default(),
        }
    }

    fn exec(&self, cwd: &Path, program: &str, args: &[&str], env: &[(&str, &str)]) -> Ran {
        let mut invocation = self.environment(Invocation::new(self.bin().join(program)));
        for arg in args {
            invocation = invocation.arg(arg);
        }
        for (key, value) in env {
            invocation = invocation.env(key, value);
        }
        self.finish(&invocation, cwd, program)
    }

    /// `just rust-prune <args>` for the checkout, started in it.
    fn prune(&self, args: &[&str], env: &[(&str, &str)]) -> Ran {
        self.prune_in(&self.tree, &self.tree, args, env)
    }

    /// `just rust-prune <args>` for the checkout at `working`, started from `at`.
    fn prune_in(&self, at: &Path, working: &Path, args: &[&str], env: &[(&str, &str)]) -> Ran {
        let mut invocation = self
            .environment(Invocation::new(self.bin().join("just")))
            .arg("--justfile")
            .arg(root().join("justfile"))
            .arg("--working-directory")
            .arg(working)
            .arg("rust-prune");
        for arg in args {
            invocation = invocation.arg(arg);
        }
        for (key, value) in env {
            invocation = invocation.env(key, value);
        }
        self.finish(&invocation, at, "just")
    }

    /// Every `<cwd>|<argv>` line the fake cargo logged.
    fn cargo_log(&self) -> Vec<String> {
        std::fs::read_to_string(self.base.join("cargo.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// How many logged cargo calls were real cleans (not `--dry-run`).
    fn real_cleans(&self) -> usize {
        self.cargo_log()
            .iter()
            .filter(|line| line.contains("|clean ") && !line.contains("--dry-run"))
            .count()
    }

    fn unit(&self, tree: &str) -> PathBuf {
        self.tree.join(tree).join("ae-unit/payload")
    }

    /// Whether every tree still holds its `ae-unit`.
    fn alive(&self) -> bool {
        PRUNE_TREES.iter().all(|tree| self.unit(tree).is_file())
    }

    /// Whether no tree holds its `ae-unit` any more.
    fn gone(&self) -> bool {
        PRUNE_TREES.iter().all(|tree| !self.unit(tree).exists())
    }

    /// Whether every tree still holds its `other/keep`.
    fn kept(&self) -> bool {
        PRUNE_TREES
            .iter()
            .all(|tree| self.tree.join(tree).join("other/keep").is_file())
    }

    /// Move the checkout's `rel` to `<base>/<to>` and leave a link to it in its place.
    fn relink(&self, rel: &str, to: &str) -> PathBuf {
        let moved = self.base.join(to);
        std::fs::rename(self.tree.join(rel), &moved)
            .and_then(|()| std::os::unix::fs::symlink(&moved, self.tree.join(rel)))
            .unwrap_or_else(|why| panic!("linking {rel}: {why}"));
        moved
    }

    /// Undo [`Prune::relink`].
    fn unlink(&self, rel: &str, moved: &Path) {
        std::fs::remove_file(self.tree.join(rel))
            .and_then(|()| std::fs::rename(moved, self.tree.join(rel)))
            .unwrap_or_else(|why| panic!("restoring {rel}: {why}"));
    }
}

/// The cargo calls a clean run makes in `tree`: metadata, the six dry runs and, when `apply`,
/// the six real cleans, in that order.
fn expected_cargo(tree: &Path, apply: bool) -> Vec<String> {
    let t = tree.display();
    let head = format!("{t}|clean --locked --manifest-path {t}/Cargo.toml -p ae");
    let flags = [
        "--profile dev".to_owned(),
        "--profile release".to_owned(),
        "--profile release --target aarch64-apple-darwin".to_owned(),
        "--profile release --target x86_64-unknown-linux-musl".to_owned(),
        format!("--profile dev --target-dir {t}/target/force-warn-guard"),
        format!("--profile dev --target-dir {t}/target/world-read-guard"),
    ];
    let mut log = vec![format!(
        "{t}|metadata --no-deps --locked --format-version 1 --manifest-path {t}/Cargo.toml"
    )];
    log.extend(
        flags
            .iter()
            .map(|flag| format!("{head} {flag} --dry-run --verbose")),
    );
    if apply {
        log.extend(flags.iter().map(|flag| format!("{head} {flag}")));
    }
    log
}

/// A refusal: exit 1, the recipe's own `rust-prune: refused:` line, and `why` in it.
fn assert_refused(ran: &Ran, why: &str) {
    assert_eq!(ran.code, 1, "a refusal exits 1: {}{}", ran.out, ran.err);
    assert!(
        ran.err.contains("rust-prune: refused: "),
        "the recipe's own refusal line: {}",
        ran.err
    );
    assert!(
        ran.err.contains(why),
        "the refusal names `{why}`: {}",
        ran.err
    );
}

/// A process that outlives its starter so the recipe can find it; killed when dropped.
struct Sleeper {
    pid: String,
    child: Option<String>,
    dir: PathBuf,
}

impl Sleeper {
    /// A process tree whose argv names `argv0`, started in `cwd`.
    fn with_argv(fixture: &Prune, cwd: &Path, argv0: &str) -> Self {
        let mut sleeper = Self::start(
            fixture,
            cwd,
            "sh -c 'sleep 30 & echo $! >\"$1\"; wait' \"$1\" \"$3\" </dev/null >/dev/null 2>&1 & echo $! >\"$2\"",
            argv0,
        );
        let child = wait_written(&fixture.base.join("sleeper-child.pid"))
            .trim()
            .to_owned();
        assert!(!child.is_empty(), "the sleeper wrote its child pid");
        sleeper.child = Some(child);
        sleeper
    }

    /// `sleep 30` with `cwd` as its working directory.
    fn in_dir(fixture: &Prune, cwd: &Path) -> Self {
        Self::start(
            fixture,
            cwd,
            "sleep 30 </dev/null >/dev/null 2>&1 & echo $! >\"$2\"",
            "-",
        )
    }

    fn start(fixture: &Prune, cwd: &Path, script: &str, first: &str) -> Self {
        let pidfile = fixture.base.join("sleeper.pid");
        let _ = std::fs::remove_file(&pidfile);
        let _ = std::fs::remove_file(fixture.base.join("sleeper-child.pid"));
        let invocation = Invocation::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .arg("starter")
            .arg(first)
            .arg(&pidfile)
            .arg(fixture.base.join("sleeper-child.pid"));
        let status = raw::run(
            &invocation,
            cwd,
            &fixture.base.join("start.out"),
            &fixture.base.join("start.err"),
        )
        .unwrap_or_else(|why| panic!("the sleeper starts: {why}"));
        assert!(matches!(status.outcome(), ExitOutcome::Code(0)));
        let pid = wait_written(&pidfile).trim().to_owned();
        assert!(!pid.is_empty(), "the sleeper wrote its pid");
        Self {
            pid,
            child: None,
            dir: fixture.base.to_path_buf(),
        }
    }
}

impl Drop for Sleeper {
    fn drop(&mut self) {
        let mut kill = Invocation::new("kill").arg(&self.pid);
        if let Some(child) = &self.child {
            kill = kill.arg(child);
        }
        let _ = raw::run(
            &kill,
            &self.dir,
            &self.dir.join("kill.out"),
            &self.dir.join("kill.err"),
        );
    }
}

fn ps(dir: &Path) -> String {
    let out = dir.join("sleeper-ps.out");
    let command = Invocation::new("ps")
        .arg("-A")
        .arg("-ww")
        .arg("-o")
        .arg("pid=,ppid=,stat=");
    assert!(raw::run(&command, dir, &out, Path::new("/dev/null")).is_ok());
    read(&out)
}
fn child_of(dir: &Path, parent: &str) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if let Some(pid) = ps(dir).lines().find_map(|line| {
            let f: Vec<_> = line.split_whitespace().collect();
            (f.get(1).copied() == Some(parent)).then(|| f[0].to_owned())
        }) {
            return pid;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("sleeper child of pid {parent} appears");
}
#[test]
fn a_dropped_sleeper_leaves_no_process_behind() {
    let fixture = Prune::new("sleeper-drop");
    let argv0 = format!("{}/target/debug/deps/ae-sleeper", fixture.tree.display());
    let sleeper = Sleeper::with_argv(&fixture, &fixture.base, &argv0);
    let child = child_of(&fixture.base, &sleeper.pid);
    drop(sleeper);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut alive = true;
    while alive && std::time::Instant::now() < deadline {
        alive = ps(&fixture.base).lines().any(|line| {
            let f: Vec<_> = line.split_whitespace().collect();
            f.first()
                .zip(f.get(2))
                .is_some_and(|(p, s)| *p == child.as_str() && !s.starts_with('Z'))
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if alive {
        let null = Path::new("/dev/null");
        let kill = Invocation::new("kill").arg(&child);
        let _ = raw::run(&kill, &fixture.base, null, null);
    }
    assert!(!alive, "pid {child} outlived the fixture (#261)");
}

/// `just rust-prune` reports by default: one `would clean` line per tree, cargo asked only for
/// the build directories and six dry runs, nothing removed.
#[test]
fn rust_prune_reports_by_default_and_removes_nothing() {
    let fixture = Prune::new("prune-report");
    let ran = fixture.prune(&[], &[]);
    assert_eq!(ran.code, 0, "{}{}", ran.out, ran.err);
    for label in PRUNE_LABELS {
        assert!(
            ran.out.contains(&format!("would clean {label}: ")),
            "a report line for {label}: {}",
            ran.out
        );
    }
    assert!(
        ran.out.contains("report only, nothing removed"),
        "{}",
        ran.out
    );
    assert_eq!(fixture.cargo_log(), expected_cargo(&fixture.tree, false));
    assert!(
        fixture.alive() && fixture.kept(),
        "a report removes nothing"
    );
}

/// `apply=yes` runs the six cleans, in order, each from the checkout, after all six dry
/// runs; the ae units go and every other file stays.
#[test]
fn rust_prune_apply_yes_runs_the_six_cleans_and_touches_nothing_else() {
    let fixture = Prune::new("prune-apply");
    put(&fixture.tree.join("src/main.rs"), "fn main() {}\n");
    let ran = fixture.prune(&["apply=yes"], &[]);
    assert_eq!(ran.code, 0, "{}{}", ran.out, ran.err);
    for label in PRUNE_LABELS {
        assert!(
            ran.out.contains(&format!("cleaned {label}: ")),
            "an apply line for {label}: {}",
            ran.out
        );
    }
    assert_eq!(fixture.cargo_log(), expected_cargo(&fixture.tree, true));
    assert!(fixture.gone(), "the ae units are cleaned");
    assert!(fixture.kept(), "every other file stays");
    assert!(fixture.tree.join("src/main.rs").is_file());
    assert!(fixture.tree.join("Cargo.toml").is_file());
}

/// Anything but nothing or exactly `apply=yes` is a usage error, before cargo is asked.
#[test]
fn rust_prune_takes_no_argument_or_exactly_apply_yes() {
    let fixture = Prune::new("prune-usage");
    for word in ["yes", "apply=no", "everything"] {
        let ran = fixture.prune(&[word], &[]);
        assert_eq!(
            ran.code, 2,
            "`{word}` is a usage error: {}{}",
            ran.out, ran.err
        );
        assert!(ran.err.contains("takes no argument"), "{}", ran.err);
        assert!(fixture.cargo_log().is_empty(), "`{word}` asked cargo");
    }
    assert!(fixture.alive());
}

/// A redirect variable hides the real build directory, so a SET one refuses, whatever it names.
#[test]
fn rust_prune_refuses_a_set_redirect_variable_before_asking_cargo_anything() {
    let fixture = Prune::new("prune-env");
    for var in [
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_TARGET_DIR",
        "CARGO_BUILD_BUILD_DIR",
        "CARGO_BUILD_TARGET",
    ] {
        let ran = fixture.prune(&["apply=yes"], &[(var, "/elsewhere")]);
        assert_refused(&ran, var);
        assert!(fixture.cargo_log().is_empty(), "{var} reached cargo");
        assert!(fixture.alive(), "{var} removed something");
    }
}

/// Cargo finds `.cargo/config.toml` from the working directory and its parents, never from
/// `--manifest-path`, so the recipe asks from the checkout and judges THAT config: a redirect
/// in the checkout's own, or a parent's, refuses, and one only the caller has does not.
#[test]
fn rust_prune_judges_the_checkouts_cargo_config_not_the_callers() {
    let fixture = Prune::new("prune-config");
    let redirect =
        |key: &str, dir: &str| format!("[build]\n{key} = \"{}/{dir}\"\n", fixture.base.display());

    // The caller sits in a directory with a redirect; the checkout has none.
    let caller = fixture.base.join("caller");
    put(
        &caller.join(".cargo/config.toml"),
        &redirect("target-dir", "caller-out"),
    );
    let ran = fixture.prune_in(&caller, &fixture.tree, &[], &[]);
    assert_eq!(
        ran.code, 0,
        "the caller's config is not the checkout's: {}{}",
        ran.out, ran.err
    );
    let prefix = format!("{}|", fixture.tree.display());
    assert!(
        fixture
            .cargo_log()
            .iter()
            .all(|line| line.starts_with(&prefix)),
        "cargo ran from the checkout: {:?}",
        fixture.cargo_log()
    );

    // The checkout's own config, the other way round, or a parent directory's.
    let clean_caller = fixture.base.join("clean-caller");
    std::fs::create_dir_all(&clean_caller).unwrap_or_else(|why| panic!("a caller dir: {why}"));
    let own = fixture.tree.join(".cargo/config.toml");
    let parent = fixture.base.join(".cargo/config.toml");
    for (file, key, guard) in [
        (&own, "target-dir", "target_directory"),
        (&own, "build-dir", "build_directory"),
        (&parent, "target-dir", "target_directory"),
    ] {
        fixture.clear("cargo.log");
        put(file, &redirect(key, "elsewhere"));
        let ran = fixture.prune_in(&clean_caller, &fixture.tree, &["apply=yes"], &[]);
        assert_refused(&ran, guard);
        assert!(
            ran.err.contains("a redirect in a cargo config"),
            "{}",
            ran.err
        );
        assert_eq!(fixture.real_cleans(), 0, "{guard} cleaned something");
        assert!(fixture.alive(), "{guard} removed something");
        std::fs::remove_file(file).unwrap_or_else(|why| panic!("{}: {why}", file.display()));
    }
}

/// It judges the one checkout it runs at the root of: a subdirectory refuses, a link to the
/// root works and cargo still runs from the real path.
#[test]
fn rust_prune_wants_the_checkout_root_and_resolves_a_linked_working_directory() {
    let fixture = Prune::new("prune-root");
    let sub = fixture.tree.join("src");
    std::fs::create_dir_all(&sub).unwrap_or_else(|why| panic!("a subdirectory: {why}"));
    let ran = fixture.prune_in(&sub, &sub, &[], &[]);
    assert_refused(&ran, "run it from the checkout root");
    assert!(fixture.cargo_log().is_empty());

    let link = fixture.base.join("tree-link");
    std::os::unix::fs::symlink(&fixture.tree, &link)
        .unwrap_or_else(|why| panic!("a link to the checkout: {why}"));
    let ran = fixture.prune_in(&link, &link, &[], &[]);
    assert_eq!(ran.code, 0, "{}{}", ran.out, ran.err);
    assert_eq!(fixture.cargo_log(), expected_cargo(&fixture.tree, false));
}

/// With no `target/` there is nothing to judge and cargo is never asked.
#[test]
fn rust_prune_with_no_target_directory_does_nothing() {
    let fixture = Prune::new("prune-none");
    std::fs::remove_dir_all(fixture.tree.join("target")).unwrap_or_else(|why| panic!("{why}"));
    let ran = fixture.prune(&["apply=yes"], &[]);
    assert_eq!(ran.code, 0, "{}{}", ran.out, ran.err);
    assert!(ran.out.contains("nothing to do"), "{}", ran.out);
    assert!(fixture.cargo_log().is_empty());
}

/// A cargo that lists a path outside the build directory (or its guard's own), through a dot
/// segment, or through a link is not trusted with the real clean: all six dry runs happen
/// first, so nothing is cleaned and the file outside survives.
#[test]
fn rust_prune_refuses_a_dry_run_path_that_escapes_and_cleans_nothing() {
    let fixture = Prune::new("prune-escape");
    let outside = fixture.base.join("outside");
    put(&outside.join("victim"), "x\n");
    std::os::unix::fs::symlink(&outside, fixture.tree.join("target/debug/linkdir"))
        .and_then(|()| {
            std::os::unix::fs::symlink(
                &outside,
                fixture
                    .tree
                    .join("target/world-read-guard/debug/incremental"),
            )
        })
        .unwrap_or_else(|why| panic!("the links: {why}"));
    let t = fixture.tree.display();
    for (filter, line, why) in [
        (
            "/target/debug",
            format!("{}/victim", outside.display()),
            "outside",
        ),
        (
            "/target/debug",
            format!("{t}/target/debug/../../evil"),
            "dot segment",
        ),
        (
            "/target/debug",
            format!("{t}/target/debug/linkdir/victim"),
            "through a link",
        ),
        (
            "world-read-guard",
            format!("{t}/target/world-read-guard/debug/incremental/victim"),
            "through a link",
        ),
        (
            "force-warn-guard",
            format!("{t}/target/debug/ae-unit/payload"),
            "outside",
        ),
    ] {
        fixture.clear("cargo.log");
        fixture.knob("lie", &format!("{filter}\n{line}\n"));
        let ran = fixture.prune(&["apply=yes"], &[]);
        assert_refused(&ran, "cargo would remove");
        assert!(ran.err.contains(why), "{why}: {}", ran.err);
        assert_eq!(
            fixture.real_cleans(),
            0,
            "{why}: {line} reached a real clean"
        );
        assert!(
            outside.join("victim").is_file(),
            "{why}: the outside file survived"
        );
        assert!(fixture.alive(), "{why}: nothing was removed");
    }
}

/// Only cargo's STDOUT is the inventory, and every non-empty line of it must be an absolute
/// path: a relative one (`../outside/victim`), a decorated one or prose is a path the recipe
/// cannot contain, so it is refused before any real clean, never skipped.
#[test]
fn rust_prune_refuses_a_dry_run_line_that_is_not_an_absolute_path() {
    let fixture = Prune::new("prune-grammar");
    let outside = fixture.base.join("outside");
    put(&outside.join("victim"), "x\n");
    for (filter, line) in [
        ("/target/debug", "../outside/victim"),
        ("/target/debug", "target/debug/ae-unit"),
        ("/target/debug", "./ae-unit"),
        ("/target/debug", "removing ae-unit"),
        ("/target/debug", " /target/debug/ae-unit"),
        ("world-read-guard", "../../outside/victim"),
    ] {
        fixture.clear("cargo.log");
        fixture.knob("lie", &format!("{filter}\n{line}\n"));
        let ran = fixture.prune(&["apply=yes"], &[]);
        assert_refused(&ran, "not an absolute path");
        assert!(ran.err.contains(line), "`{line}` is named: {}", ran.err);
        assert_eq!(fixture.real_cleans(), 0, "`{line}` reached a real clean");
        assert!(
            outside.join("victim").is_file(),
            "`{line}`: the outside file survived"
        );
        assert!(fixture.alive(), "`{line}`: nothing was removed");
    }
}

/// A dry run is also proven by its Summary: exactly one valid line on stderr
/// (`Summary <n> file[s]`, then optionally `, <size> total`), so a missing, malformed,
/// repeated or mislabelled one refuses before any real clean.
#[test]
fn rust_prune_refuses_a_dry_run_that_does_not_end_in_a_valid_summary() {
    let fixture = Prune::new("prune-summary");
    for (knob, text) in [
        ("summary.none", ""),
        ("summary.text", "     Summary\n"),
        ("summary.text", "     Summary lots of files, 1B total\n"),
        ("summary.text", "     Summary -3 files, 1B total\n"),
        ("summary.text", "     Summary 3 files and more\n"),
        ("summary.text", "     Summary 3 files, \n"),
        (
            "summary.text",
            "     Summary 3 files, 1B total\n     Summary 3 files, 1B total\n",
        ),
        ("summary.text", "     Removed 3 files, 1B total\n"),
    ] {
        fixture.clear("summary.none");
        fixture.clear("summary.text");
        fixture.clear("cargo.log");
        fixture.knob(knob, text);
        let ran = fixture.prune(&["apply=yes"], &[]);
        assert_refused(&ran, "without a valid Summary");
        assert_eq!(fixture.real_cleans(), 0, "`{text}` reached a real clean");
        assert!(fixture.alive(), "`{text}`: nothing was removed");
    }
}

/// A Summary that claims files while STDOUT lists none is not a proof of what a clean removes,
/// and a path that only STDERR carries is not inventory.
#[test]
fn rust_prune_refuses_a_summary_that_claims_files_stdout_does_not_list() {
    let fixture = Prune::new("prune-claim");
    std::fs::remove_dir_all(fixture.tree.join("target/debug/ae-unit"))
        .unwrap_or_else(|why| panic!("emptying the dev tree: {why}"));
    fixture.knob("summary.text", "     Summary 7 files, 1B total\n");
    let ran = fixture.prune(&["apply=yes"], &[]);
    assert_refused(&ran, "listed no path on stdout");
    assert_eq!(fixture.real_cleans(), 0);

    fixture.clear("cargo.log");
    fixture.knob(
        "stderr.extra",
        &format!("{}/target/debug/ae-unit\n", fixture.tree.display()),
    );
    let ran = fixture.prune(&["apply=yes"], &[]);
    assert_refused(&ran, "listed no path on stdout");
    assert_eq!(fixture.real_cleans(), 0);
}

/// Cargo's diagnostics are not paths: a stderr line that starts with `/` and names something
/// elsewhere neither counts as inventory nor refuses, and the run goes on to clean.
#[test]
fn rust_prune_reads_paths_from_stdout_only_and_never_from_diagnostics() {
    let fixture = Prune::new("prune-streams");
    fixture.knob(
        "stderr.extra",
        "warning: could not read /elsewhere/other-unit\n/elsewhere/not-ours: skipped\n",
    );
    let ran = fixture.prune(&["apply=yes"], &[]);
    assert_eq!(ran.code, 0, "{}{}", ran.out, ran.err);
    assert_eq!(fixture.cargo_log(), expected_cargo(&fixture.tree, true));
    assert!(fixture.gone(), "the ae units are cleaned");
    assert!(fixture.kept(), "every other file stays");
}

/// What a genuine run looks like still passes: the Summary spellings cargo uses, a blank line on
/// stdout, and a tree with nothing to remove (empty stdout, `Summary 0 files`).
#[test]
fn rust_prune_accepts_a_zero_removal_dry_run_and_the_summaries_cargo_prints() {
    let fixture = Prune::new("prune-genuine");
    fixture.knob("lie", "/target/debug\n\n");
    for text in [
        "     Summary 1 file, 1B total\n",
        "     Summary 12 files, 3.4MiB total\n",
        "     Summary 0 files\n",
        "     Summary 0 files, 0B total\n",
    ] {
        fixture.knob("summary.text", text);
        let ran = fixture.prune(&[], &[]);
        assert_eq!(ran.code, 0, "`{text}`: {}{}", ran.out, ran.err);
        assert!(ran.out.contains("would clean debug: "), "{}", ran.out);
    }
    fixture.clear("lie");
    fixture.clear("summary.text");
    for tree in PRUNE_TREES {
        std::fs::remove_dir_all(fixture.tree.join(tree).join("ae-unit"))
            .unwrap_or_else(|why| panic!("emptying {tree}: {why}"));
    }
    fixture.clear("cargo.log");
    let ran = fixture.prune(&["apply=yes"], &[]);
    assert_eq!(
        ran.code, 0,
        "nothing to remove is not a refusal: {}{}",
        ran.out, ran.err
    );
    assert_eq!(fixture.cargo_log(), expected_cargo(&fixture.tree, true));
    assert!(fixture.kept(), "every other file stays");
}

/// Without `lsof` the run cannot see a process that only has its working directory in the
/// checkout; it says so, once for the whole run, and stays quiet when `lsof` is there.
#[test]
fn rust_prune_says_once_that_a_missing_lsof_leaves_it_judging_by_argv_alone() {
    let fixture = Prune::new("prune-nolsof");
    let ran = fixture.prune(&["apply=yes"], &[]);
    assert_eq!(ran.code, 0, "{}{}", ran.out, ran.err);
    assert_eq!(
        ran.err.matches("lsof is not installed").count(),
        1,
        "named once although apply asks twice: {}",
        ran.err
    );
    assert!(
        ran.err.contains("working directory"),
        "the limitation is named: {}",
        ran.err
    );

    fixture.install_fake_lsof();
    fixture.knob("lsof.out", "");
    let ran = fixture.prune(&[], &[]);
    assert_eq!(ran.code, 0, "{}{}", ran.out, ran.err);
    assert!(
        !ran.err.contains("lsof"),
        "an installed lsof needs no note: {}",
        ran.err
    );
}

/// `target/`, or a tree the recipe cleans, that is a LINK is refused whole, so a clean never
/// walks out through it: the file the link points at survives.
#[test]
fn rust_prune_refuses_a_linked_tree_and_removes_nothing_through_it() {
    let fixture = Prune::new("prune-links");
    for (rel, to) in [
        ("target/force-warn-guard", "moved-guard"),
        ("target/world-read-guard", "moved-world"),
        ("target/debug", "moved-debug"),
        ("target/x86_64-unknown-linux-musl", "moved-musl"),
        ("target", "moved-target"),
    ] {
        let moved = fixture.relink(rel, to);
        let ran = fixture.prune(&["apply=yes"], &[]);
        assert_refused(&ran, "is not a plain directory");
        assert_eq!(fixture.real_cleans(), 0, "{rel} cleaned something");
        let kept = std::fs::read_dir(&moved).map_or(0, Iterator::count);
        assert!(kept > 0, "{rel}: the linked directory is intact");
        fixture.unlink(rel, &moved);
        assert!(fixture.alive(), "{rel}: restored and untouched");
    }
}

/// Cargo's own failure stops the run with its message, and a build directory it cannot name
/// is no build directory: nothing is cleaned in any of the three cases.
#[test]
fn rust_prune_stops_on_a_cargo_failure_and_shows_why() {
    let fixture = Prune::new("prune-cargo");
    fixture.knob("metadata.fail", "");
    assert_refused(&fixture.prune(&["apply=yes"], &[]), "cargo metadata failed");
    fixture.clear("metadata.fail");

    fixture.knob("clean.fail", "");
    assert_refused(
        &fixture.prune(&["apply=yes"], &[]),
        "cargo clean --dry-run failed for debug",
    );
    fixture.clear("clean.fail");

    let tag = fixture.tree.join("target/force-warn-guard/CACHEDIR.TAG");
    std::fs::remove_file(&tag).unwrap_or_else(|why| panic!("{why}"));
    let ran = fixture.prune(&["apply=yes"], &[]);
    assert_refused(&ran, "cargo clean --dry-run failed for force-warn-guard");
    assert!(
        ran.err.contains("CACHEDIR.TAG"),
        "cargo's message is shown: {}",
        ran.err
    );
    assert_eq!(fixture.real_cleans(), 0);
    assert!(fixture.alive());
}

/// A process outside the run's own ancestry that names the checkout in its argv, or has its
/// working directory in it, blocks the prune before cargo is asked; `ps` is required, `lsof`
/// is read when installed, and a sibling directory sharing the checkout's name prefix is not it.
#[test]
fn rust_prune_refuses_a_process_using_the_checkout() {
    let fixture = Prune::new("prune-use");
    let t = fixture.tree.display().to_string();

    fixture.knob(
        "ps.extra",
        &format!("424242 1 {t}/target/debug/deps/ae-123 --nocapture\n"),
    );
    assert_refused(&fixture.prune(&[], &[]), "pid 424242 is running under");
    fixture.clear("ps.extra");
    fixture.knob("ps.fail", "");
    assert_refused(&fixture.prune(&[], &[]), "ps failed");
    fixture.clear("ps.fail");
    assert!(
        fixture.cargo_log().is_empty(),
        "a busy checkout reached cargo"
    );

    fixture.install_fake_lsof();
    fixture.knob("lsof.out", &format!("p424242\nfcwd\nn{t}/src\n"));
    assert_refused(
        &fixture.prune(&[], &[]),
        "pid 424242 has its working directory under",
    );
    fixture.knob("lsof.out", &format!("p424242\nfcwd\nn{t}\n"));
    assert_refused(
        &fixture.prune(&[], &[]),
        "pid 424242 has its working directory under",
    );
    fixture.knob("lsof.fail", "");
    assert_refused(&fixture.prune(&[], &[]), "lsof failed");
    fixture.clear("lsof.fail");
    assert!(
        fixture.cargo_log().is_empty(),
        "a busy checkout reached cargo"
    );

    fixture.knob(
        "lsof.out",
        &format!("p424242\nfcwd\nn{t}-sibling\np424243\nfcwd\nn/\n"),
    );
    let ran = fixture.prune(&[], &[]);
    assert_eq!(
        ran.code, 0,
        "only a path INSIDE the checkout counts: {}{}",
        ran.out, ran.err
    );
}

/// With the real `ps`: a run whose own ancestor names the checkout is still its own
/// (the exemption is the proven ancestry), while an unrelated process with an argv under
/// `target/` refuses.
#[test]
fn rust_prune_exempts_its_own_ancestry_but_not_an_unrelated_argv() {
    let fixture = Prune::new("prune-argv");
    let t = fixture.tree.display().to_string();
    let wrapped = fixture
        .environment(Invocation::new("/bin/sh"))
        .arg("-c")
        .arg("\"$@\"; :")
        .arg(format!("{t}/target/debug/deps/ae-own-run"))
        .arg(fixture.bin().join("just"))
        .arg("--justfile")
        .arg(root().join("justfile"))
        .arg("--working-directory")
        .arg(&fixture.tree)
        .arg("rust-prune");
    let own = fixture.finish(&wrapped, &fixture.tree, "wrapped");
    assert_eq!(
        own.code, 0,
        "an ancestor's argv under the checkout is the run's own: {}{}",
        own.out, own.err
    );

    let _stray = Sleeper::with_argv(
        &fixture,
        &fixture.base,
        &format!("{t}/target/debug/deps/ae-stray"),
    );
    let ran = fixture.prune(&[], &[]);
    assert_refused(&ran, "is running under");
    assert!(
        ran.err.contains("ae-stray"),
        "the process is named: {}",
        ran.err
    );
    assert_eq!(fixture.real_cleans(), 0);
}

/// With the real `lsof`: the run's own shells have the checkout as their working directory and
/// are exempt; an unrelated process there refuses. Skipped, loudly, where `lsof` is not installed
/// (the recipe treats it as optional).
#[test]
fn rust_prune_exempts_its_own_working_directory_but_not_an_unrelated_one() {
    let Some(lsof) = tool_path("lsof", &["/usr/sbin", "/usr/bin", "/sbin", "/bin"]) else {
        eprintln!("note: lsof is not installed, so the working-directory pin is skipped");
        return;
    };
    let fixture = Prune::new("prune-cwd");
    std::os::unix::fs::symlink(lsof, fixture.bin().join("lsof"))
        .unwrap_or_else(|why| panic!("the linked lsof: {why}"));
    let own = fixture.prune(&[], &[]);
    assert_eq!(
        own.code, 0,
        "the run's own working directory is exempt: {}{}",
        own.out, own.err
    );

    let _stray = Sleeper::in_dir(&fixture, &fixture.tree);
    let ran = fixture.prune(&[], &[]);
    assert_refused(&ran, "has its working directory under");
    assert_eq!(fixture.real_cleans(), 0);
}

/// What is wrong with the `rust-prune` recipe body `lines`, empty when nothing is: no hand
/// deletion and no glob, nothing a lead-only wrapper owns, nothing GNU-only, the two guard
/// trees named, and cargo asked for exactly one `metadata` and two `clean`s, each locked, scoped
/// to `ae` and pointed at the manifest.
fn prune_recipe_defects(lines: &[String]) -> Vec<String> {
    let mut defects = Vec::new();
    let text = lines.join("\n");
    if lines.is_empty() {
        defects.push("no recipe body".to_owned());
    }
    for word in ["rm", "rmdir", "unlink", "shred"] {
        if bare_word_count(&text, word) > 0 {
            defects.push(format!("the recipe calls `{word}`"));
        }
    }
    for forbidden in [
        "-delete",
        "*-guard",
        "git clean",
        "git worktree",
        ".local",
        "ae-wt",
        "AE_GATE",
        "locks/gate",
        "readlink -f",
        "stat -c",
        "flock",
        "timeout ",
        "grep -P",
        "sed -i",
    ] {
        if text.contains(forbidden) {
            defects.push(format!("the recipe contains `{forbidden}`"));
        }
    }
    for tree in [
        "force-warn-guard",
        "world-read-guard",
        "x86_64-unknown-linux-musl",
    ] {
        if !text.contains(tree) {
            defects.push(format!("the recipe never names `{tree}`"));
        }
    }
    let calls: Vec<&str> = text
        .split("$(cargo ")
        .skip(1)
        .filter_map(|rest| rest.split_whitespace().next())
        .collect();
    if calls != ["metadata", "clean", "clean"] {
        defects.push(format!("cargo is called as {calls:?}"));
    }
    for line in lines.iter().filter(|line| line.contains("$(cargo ")) {
        let need: &[&str] = if line.contains("$(cargo metadata") {
            &["--locked", "--no-deps", "--manifest-path"]
        } else {
            &["--locked", " -p ae", "--manifest-path"]
        };
        for flag in need {
            if !line.contains(flag) {
                defects.push(format!("`{flag}` is missing from `{line}`"));
            }
        }
    }
    defects
}

/// The recipe's text obeys [`prune_recipe_defects`], and each deliberately broken body is caught.
#[test]
fn the_rust_prune_recipe_has_no_hand_deletion_no_private_machinery_and_scoped_cargo() {
    let justfile = read(&root().join("justfile"));
    assert!(
        justfile
            .lines()
            .any(|line| line == "set positional-arguments"),
        "the recipe reads its argument as $1, which needs positional arguments"
    );
    let good = recipe_text(&justfile, "rust-prune apply=\"\":");
    assert!(!good.is_empty(), "the recipe exists");
    assert_eq!(prune_recipe_defects(&good), Vec::<String>::new());

    let with = |extra: &str| {
        let mut lines = good.clone();
        lines.push(extra.to_owned());
        lines
    };
    let swapped = |from: &str, to: &str| -> Vec<String> {
        good.iter().map(|line| line.replace(from, to)).collect()
    };
    for (name, body) in [
        ("rm", with("rm -rf \"$target\"")),
        ("find -delete", with("find \"$target\" -delete")),
        (
            "a glob over the guards",
            with("for g in \"$target\"/*-guard; do :; done"),
        ),
        ("the fleet lock", with("mkdir .local/locks/gate")),
        ("a worktree path", with("cd ../ae-wt/x")),
        ("GNU readlink", with("readlink -f \"$target\"")),
        ("another cargo verb", with("x=\"$(cargo build)\"")),
        ("an unlocked cargo", swapped("--locked", "")),
        ("an unscoped clean", swapped(" -p ae", "")),
        ("a dropped guard tree", swapped("world-read-guard", "other")),
        ("an empty recipe", Vec::new()),
    ] {
        assert!(
            !prune_recipe_defects(&body).is_empty(),
            "{name} must be reported"
        );
    }
}

/// A real `cargo` in `cwd`, offline, with a clean environment: only what a toolchain needs, so
/// a redirect variable the lane or a developer exports cannot reach it. Captures go to `scratch`.
fn real_cargo(scratch: &Path, cwd: &Path, args: &[&str]) -> Ran {
    let mut invocation = Invocation::new("cargo").env_cleared();
    for key in [
        "PATH",
        "HOME",
        "RUSTUP_TOOLCHAIN",
        "RUSTUP_HOME",
        "CARGO_HOME",
    ] {
        if let Some(value) = std::env::var_os(key) {
            invocation = invocation.env(key, value);
        }
    }
    for arg in args {
        invocation = invocation.arg(arg);
    }
    let out = scratch.join("cargo.out");
    let err = scratch.join("cargo.err");
    let status = raw::run(&invocation, cwd, &out, &err)
        .unwrap_or_else(|why| panic!("cargo must be runnable: {why}"));
    Ran {
        code: match status.outcome() {
            ExitOutcome::Code(code) => code,
            ExitOutcome::Signalled => -1,
        },
        out: std::fs::read_to_string(&out).unwrap_or_default(),
        err: std::fs::read_to_string(&err).unwrap_or_default(),
    }
}

/// A one-package crate named `ae` with a lockfile, at `dir`.
fn scratch_crate(scratch: &Path, dir: &Path) {
    put(
        &dir.join("Cargo.toml"),
        "[package]\nname = \"ae\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    );
    put(&dir.join("src/lib.rs"), "");
    let made = real_cargo(scratch, dir, &["generate-lockfile", "--offline"]);
    assert_eq!(
        made.code, 0,
        "a lockfile for the scratch crate: {}",
        made.err
    );
}

/// CONTRACT PIN, not a regression test: it describes real cargo and passes before the recipe
/// exists. The recipe asks `cargo metadata` from the checkout because cargo reads
/// `.cargo/config.toml` from the working directory and its parents; `--manifest-path`
/// moves the manifest and leaves the config where the caller stands.
#[test]
fn cargo_metadata_follows_the_working_directorys_config_not_the_manifests() {
    let scratch = super::cli::OwnedScratch::root("gate", "prune-cfg");
    let base = std::fs::canonicalize(&*scratch).unwrap_or_else(|why| panic!("{why}"));
    let (with_config, without) = (base.join("with"), base.join("without"));
    scratch_crate(&base, &with_config);
    scratch_crate(&base, &without);
    put(
        &with_config.join(".cargo/config.toml"),
        &format!("[build]\ntarget-dir = \"{}/redirected\"\n", base.display()),
    );
    let ask = |cwd: &Path, manifest: &Path| {
        let manifest = format!("{}/Cargo.toml", manifest.display());
        let ran = real_cargo(
            &base,
            cwd,
            &[
                "metadata",
                "--no-deps",
                "--locked",
                "--offline",
                "--format-version",
                "1",
                "--manifest-path",
                manifest.as_str(),
            ],
        );
        assert_eq!(ran.code, 0, "cargo metadata: {}", ran.err);
        ran.out
    };
    let moved = ask(&with_config, &without);
    assert!(
        moved.contains(&format!(
            "\"target_directory\":\"{}/redirected\"",
            base.display()
        )),
        "the config of the directory cargo runs in applies to another manifest: {moved}"
    );
    let left = ask(&without, &with_config);
    assert!(
        left.contains(&format!(
            "\"target_directory\":\"{}/target\"",
            with_config.display()
        )),
        "a manifest's own directory config does not apply from elsewhere: {left}"
    );
}

/// The file count of the ONE `Summary` line in a dry run's stderr when it is in the grammar
/// `rust-prune` accepts (`Summary <n> file[s]`, then optionally `, <size> total`), else `None`.
/// The contract pin below holds real cargo's output to it.
fn dry_run_summary_files(stderr: &str) -> Option<u64> {
    let mut lines = stderr
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("Summary "));
    let text = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    let (count, rest) = text.split_once(' ')?;
    if count.is_empty() || !count.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let sized = |unit: &str| {
        rest.strip_prefix(unit)
            .and_then(|tail| tail.strip_prefix(", "))
            .is_some_and(|tail| tail.ends_with(" total"))
    };
    if rest == "file" || rest == "files" || sized("file") || sized("files") {
        count.parse().ok()
    } else {
        None
    }
}

/// CONTRACT PIN, not a regression test: the grammar `rust-prune` parses. `cargo clean -p ae
/// --dry-run --verbose` prints the absolute paths it would remove, and nothing else, on stdout
/// and exactly one `Summary` on stderr and deletes nothing; a tree with nothing to remove
/// prints no path and `Summary 0 files`; a `--target` clean also lists the host units under
/// `target/release` (why the recipe's root is `target/`); a `--target-dir` one without its
/// `CACHEDIR.TAG` is refused by cargo.
#[test]
fn cargo_clean_dry_run_lists_absolute_paths_and_removes_nothing() {
    let scratch = super::cli::OwnedScratch::root("gate", "prune-dry");
    let base = std::fs::canonicalize(&*scratch).unwrap_or_else(|why| panic!("{why}"));
    let krate = base.join("ae");
    scratch_crate(&base, &krate);
    let target = krate.join("target");
    let fake_files = [
        "debug/.fingerprint/ae-abc123/lib-ae",
        "debug/incremental/ae-xyz/s-1/dep-graph.bin",
        "debug/deps/libdep-keep.rlib",
        "release/.fingerprint/ae-abc123/lib-ae",
        "aarch64-apple-darwin/release/.fingerprint/ae-abc123/lib-ae",
        "x86_64-unknown-linux-musl/release/.fingerprint/ae-abc123/lib-ae",
        "force-warn-guard/debug/.fingerprint/ae-abc123/lib-ae",
    ];
    for rel in fake_files {
        put(&target.join(rel), "x\n");
    }
    let tag = target.join("force-warn-guard/CACHEDIR.TAG");
    put(&tag, "Signature: 8a477f597d28d172789f06886806bc55\n");
    let manifest = format!("{}/Cargo.toml", krate.display());
    let clean = |flags: &[&str]| {
        let mut args = vec![
            "clean",
            "--locked",
            "--manifest-path",
            manifest.as_str(),
            "-p",
            "ae",
        ];
        args.extend_from_slice(flags);
        args.extend(["--dry-run", "--verbose"]);
        real_cargo(&base, &krate, &args)
    };
    let listed = |ran: &Ran, prefixes: &[String]| {
        assert_eq!(ran.code, 0, "a dry run succeeds: {}", ran.err);
        assert!(
            dry_run_summary_files(&ran.err).is_some_and(|files| files > 0),
            "one valid Summary, claiming files, is on stderr: {}",
            ran.err
        );
        let lines: Vec<&str> = ran.out.lines().filter(|line| !line.is_empty()).collect();
        assert!(!lines.is_empty(), "a dry run lists paths: {}", ran.err);
        for line in &lines {
            assert!(
                prefixes
                    .iter()
                    .any(|prefix| line.starts_with(prefix.as_str())),
                "`{line}` is an absolute path under one of {prefixes:?}"
            );
        }
        ran.out.clone()
    };
    let t = target.display();
    let dev = listed(&clean(&["--profile", "dev"]), &[format!("{t}/debug/")]);
    assert!(
        dev.contains("ae-abc123") && dev.contains("dep-graph.bin"),
        "{dev}"
    );
    assert!(
        !dev.contains("libdep-keep"),
        "a dependency is not an ae unit: {dev}"
    );
    let musl = listed(
        &clean(&[
            "--profile",
            "release",
            "--target",
            "x86_64-unknown-linux-musl",
        ]),
        &[
            format!("{t}/release/"),
            format!("{t}/x86_64-unknown-linux-musl/"),
        ],
    );
    assert!(
        musl.contains(&format!("{t}/release/")) && musl.contains("x86_64-unknown-linux-musl/"),
        "a --target clean lists the host units too: {musl}"
    );
    let native = listed(
        &clean(&["--profile", "release", "--target", "aarch64-apple-darwin"]),
        &[
            format!("{t}/release/"),
            format!("{t}/aarch64-apple-darwin/"),
        ],
    );
    assert!(
        native.contains(&format!("{t}/release/")) && native.contains("aarch64-apple-darwin/"),
        "a --target clean of the host triple lists the host units too: {native}"
    );
    let guard_dir = format!("{t}/force-warn-guard");
    let guard = clean(&["--profile", "dev", "--target-dir", guard_dir.as_str()]);
    listed(&guard, &[format!("{guard_dir}/debug/")]);
    std::fs::remove_file(&tag).unwrap_or_else(|why| panic!("{why}"));
    let untagged = clean(&["--profile", "dev", "--target-dir", guard_dir.as_str()]);
    assert_ne!(untagged.code, 0, "cargo refuses an untagged target-dir");
    assert!(untagged.err.contains("CACHEDIR.TAG"), "{}", untagged.err);
    for rel in fake_files {
        assert!(target.join(rel).is_file(), "a dry run left {rel} alone");
    }
}

/// CONTRACT PIN, not a regression test: a genuine run that removes nothing. `cargo clean -p ae
/// --dry-run --verbose` over a tree that holds no ae unit exits 0, prints no path on stdout and
/// says `Summary 0 files` on stderr, which `rust-prune` must accept rather than read as a
/// missing proof.
#[test]
fn cargo_clean_dry_run_with_nothing_to_remove_lists_nothing_and_says_zero_files() {
    let scratch = super::cli::OwnedScratch::root("gate", "prune-zero");
    let base = std::fs::canonicalize(&*scratch).unwrap_or_else(|why| panic!("{why}"));
    let krate = base.join("ae");
    scratch_crate(&base, &krate);
    let guard = krate.join("target/world-read-guard");
    put(
        &guard.join("CACHEDIR.TAG"),
        "Signature: 8a477f597d28d172789f06886806bc55\n",
    );
    put(&guard.join("debug/deps/libdep-keep.rlib"), "x\n");
    let manifest = format!("{}/Cargo.toml", krate.display());
    let target_dir = guard.display().to_string();
    let ran = real_cargo(
        &base,
        &krate,
        &[
            "clean",
            "--locked",
            "--manifest-path",
            manifest.as_str(),
            "-p",
            "ae",
            "--profile",
            "dev",
            "--target-dir",
            target_dir.as_str(),
            "--dry-run",
            "--verbose",
        ],
    );
    assert_eq!(
        ran.code, 0,
        "a tree with nothing to remove is a success: {}",
        ran.err
    );
    assert!(ran.out.trim().is_empty(), "and lists no path: {}", ran.out);
    assert_eq!(
        dry_run_summary_files(&ran.err),
        Some(0),
        "and says `Summary 0 files`: {}",
        ran.err
    );
    assert!(guard.join("debug/deps/libdep-keep.rlib").is_file());
}

/// The recipes a justfile recipe header names as its dependencies.
fn recipe_deps(justfile: &str, name: &str) -> Vec<String> {
    let header = format!("{name}:");
    justfile
        .lines()
        .find_map(|line| line.strip_prefix(header.as_str()))
        .map(|rest| rest.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Whether THE GATE type-checks every declared fuzz target, on the product
/// compiler, in a target directory the gate may write (#240).
///
/// The fuzz targets are a crate outside the workspace, so no product lane built
/// them, and an enum variant added on main broke one for days unseen. The fix
/// is a stable `cargo check` of that crate inside `rust-check`, so a break
/// shows at the commit that causes it. It must select every `[[bin]]` (not a
/// default or a single target), honour the committed lock, use the compiler
/// `rust-toolchain.toml` pins rather than the fuzz nightly, run no fuzzer, and
/// write its output under the inherited `CARGO_TARGET_DIR` or the checkout's
/// own `target` — never under `fuzz/`, which the Linux gate mounts read-only.
fn fuzz_check_ok(justfile: &str) -> bool {
    if !recipe_deps(justfile, "rust-check")
        .iter()
        .any(|dep| dep == "rust-fuzz-check")
    {
        return false;
    }
    let lines = recipe_text(justfile, "rust-fuzz-check:");
    let text = lines.join("\n");
    let foreign = [
        "FUZZ_TOOLCHAIN",
        "nightly",
        "cargo +",
        "rustup",
        "cargo fuzz",
        "cargo-fuzz",
        "max_total_time",
        "-Z",
        "fuzz/target",
    ];
    if foreign.iter().any(|word| text.contains(word)) {
        return false;
    }
    let checks: Vec<&String> = lines
        .iter()
        .filter(|line| bare_word_count(line, "cargo") > 0 && bare_word_count(line, "check") > 0)
        .collect();
    let [check] = checks.as_slice() else {
        return false;
    };
    let every_bin = check.contains("--bins") || check.contains("--all-targets");
    let narrowed = check.contains("--bin ") || check.contains("--bin=") || check.contains("--lib");
    let redirect = text.contains("${CARGO_TARGET_DIR:-")
        && (check.contains("--target-dir")
            || check.contains("CARGO_TARGET_DIR=")
            || text.contains("export CARGO_TARGET_DIR="));
    check.contains("--locked")
        && check.contains("--manifest-path")
        && check.contains("fuzz/Cargo.toml")
        && every_bin
        && !narrowed
        && redirect
}

#[test]
fn the_gate_type_checks_every_fuzz_target_on_the_product_compiler() {
    assert!(
        fuzz_check_ok(&read(&root().join("justfile"))),
        "rust-check must depend on rust-fuzz-check, a locked stable `cargo check` of every \
         fuzz/Cargo.toml bin into the inherited or checkout target dir, with no nightly \
         and no fuzz run (#240)"
    );

    let good = "rust-check: rust-fmt-check rust-lint rust-fuzz-check rust-test\n\nrust-fuzz-check:\n    #!/usr/bin/env bash\n    set -euo pipefail\n    dir=\"${CARGO_TARGET_DIR:-$PWD/target}\"\n    nice -n 10 cargo check --locked --manifest-path fuzz/Cargo.toml --bins --target-dir \"$dir\"\n";
    assert!(fuzz_check_ok(good));
    // GREEN — the redirect handed on through the environment instead.
    assert!(fuzz_check_ok(&good.replace(
        "    dir=\"${CARGO_TARGET_DIR:-$PWD/target}\"\n    nice -n 10 cargo check --locked --manifest-path fuzz/Cargo.toml --bins --target-dir \"$dir\"\n",
        "    export CARGO_TARGET_DIR=\"${CARGO_TARGET_DIR:-$PWD/target}\"\n    cargo check --locked --manifest-path fuzz/Cargo.toml --all-targets\n"
    )));
    // RED — the recipe exists but the gate never runs it: #240 as it stands.
    assert!(!fuzz_check_ok(&good.replace(
        "rust-lint rust-fuzz-check rust-test",
        "rust-lint rust-test"
    )));
    // RED — the fuzz nightly in an ordinary lane: CI carries none.
    assert!(!fuzz_check_ok(&good.replace(
        "nice -n 10 cargo check",
        "cargo +nightly-2026-08-20 check"
    )));
    // RED — a second compiler pin beside rust-toolchain.toml.
    assert!(!fuzz_check_ok(
        &good.replace("nice -n 10 cargo check", "cargo +1.97.1 check")
    ));
    // RED — cargo-fuzz's own check: needs the nightly and its sanitizer flags.
    assert!(!fuzz_check_ok(&good.replace(
        "cargo check --locked --manifest-path fuzz/Cargo.toml --bins",
        "cargo fuzz check --locked"
    )));
    // RED — the gate runs a fuzzer.
    assert!(!fuzz_check_ok(&format!(
        "{good}    cargo +nightly-2026-08-20 fuzz run console_command -- -max_total_time=1\n"
    )));
    // RED — unlocked: cargo may rewrite the committed lock and report green.
    assert!(!fuzz_check_ok(&good.replace("--locked ", "")));
    // RED — one target checked, the other declared ones never.
    assert!(!fuzz_check_ok(
        &good.replace("--bins", "--bin console_command")
    ));
    // RED — no target selection named: the all-bins claim rests on a default.
    assert!(!fuzz_check_ok(&good.replace(" --bins", "")));
    // RED — default output dir fuzz/target, which the Linux gate mounts read-only.
    assert!(!fuzz_check_ok(&good.replace(" --target-dir \"$dir\"", "")));
    assert!(!fuzz_check_ok(&good.replace(
        "${CARGO_TARGET_DIR:-$PWD/target}",
        "$PWD/fuzz/target"
    )));
    // RED — an inherited redirect (a contained runner) overridden.
    assert!(!fuzz_check_ok(
        &good.replace("${CARGO_TARGET_DIR:-$PWD/target}", "$PWD/target")
    ));
}

/// The `[[bin]]` blocks of a cargo manifest as (name, path); `None` when a
/// block lacks either key.
fn manifest_bins(manifest: &str) -> Option<Vec<(String, String)>> {
    let mut blocks = Vec::new();
    let mut open: Option<(Option<String>, Option<String>)> = None;
    for raw in manifest.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            blocks.extend(open.take());
            if line == "[[bin]]" {
                open = Some((None, None));
            }
            continue;
        }
        if let (Some((name, path)), Some((key, value))) = (open.as_mut(), line.split_once('=')) {
            let value = value.trim().trim_matches('"').to_owned();
            match key.trim() {
                "name" => *name = Some(value),
                "path" => *path = Some(value),
                _ => {}
            }
        }
    }
    blocks.extend(open);
    blocks
        .into_iter()
        .map(|(name, path)| Some((name?, path?)))
        .collect()
}

/// Whether every fuzz target source is a declared `[[bin]]` that the gate's
/// all-bins check builds in full.
///
/// `cargo check --bins` builds exactly the manifest's `[[bin]]` blocks and
/// silently skips one whose `required-features` are off. A source no block
/// declares, or code behind a `cfg` the stable check does not set (cargo-fuzz
/// sets `fuzzing`, the gate does not), is type-checked by no lane at all.
fn fuzz_inventory_ok(manifest: &str, sources: &[(String, String)]) -> bool {
    let Some(bins) = manifest_bins(manifest) else {
        return false;
    };
    let mut names: Vec<&str> = bins.iter().map(|(name, _)| name.as_str()).collect();
    names.sort_unstable();
    let declared = names.len();
    names.dedup();
    let mut files: Vec<&str> = sources
        .iter()
        .filter_map(|(file, _)| file.strip_suffix(".rs"))
        .collect();
    files.sort_unstable();
    let hidden = |body: &str| {
        ["#[cfg", "#![cfg", "cfg_attr"]
            .iter()
            .any(|attr| body.contains(attr))
    };
    declared > 0
        && names.len() == declared
        && files.len() == sources.len()
        && names == files
        && bins.iter().all(|(name, path)| {
            path.strip_prefix("fuzz_targets/")
                .and_then(|rest| rest.strip_suffix(".rs"))
                == Some(name.as_str())
        })
        && !manifest.contains("required-features")
        && sources.iter().all(|(_, body)| !hidden(body))
}

#[test]
fn every_fuzz_target_source_is_a_declared_bin_the_gate_check_cannot_skip() {
    let dir = root().join("fuzz/fuzz_targets");
    let mut sources = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap_or_else(|why| panic!("{}: {why}", dir.display())) {
        let path = entry.expect("a fuzz_targets entry").path();
        let file = path
            .file_name()
            .and_then(|file| file.to_str())
            .expect("a UTF-8 file name")
            .to_owned();
        // A dot file is an editor's or the OS's, never a target.
        if !file.starts_with('.') {
            sources.push((file, read(&path)));
        }
    }
    assert!(
        fuzz_inventory_ok(&read(&root().join("fuzz/Cargo.toml")), &sources),
        "every fuzz/fuzz_targets/<name>.rs must be a [[bin]] named <name> with that path, \
         no bin may carry required-features and no target may hide code behind a cfg: \
         the gate's all-bins check must reach every target (#240)"
    );

    let manifest = "[package]\nname = \"ae-fuzz\"\n\n[dependencies]\nlibfuzzer-sys = \"=0.4.13\"\n\n[[bin]]\nname = \"a\"\npath = \"fuzz_targets/a.rs\"\ntest = false\n\n[[bin]]\nname = \"b\"\npath = \"fuzz_targets/b.rs\"\ntest = false\n";
    let body = "#![no_main]\nuse libfuzzer_sys::fuzz_target;\nfuzz_target!(|data: &[u8]| {\n    let _ = data;\n});\n";
    let two = [
        ("a.rs".to_owned(), body.to_owned()),
        ("b.rs".to_owned(), body.to_owned()),
    ];
    assert!(fuzz_inventory_ok(manifest, &two));
    // RED — a target file no block declares: built by no lane.
    let mut three = two.to_vec();
    three.push(("c.rs".to_owned(), body.to_owned()));
    assert!(!fuzz_inventory_ok(manifest, &three));
    // RED — a declared bin whose path disagrees with its name and file.
    assert!(!fuzz_inventory_ok(
        &manifest.replace("fuzz_targets/b.rs", "fuzz_targets/other.rs"),
        &two
    ));
    // RED — the same name declared twice.
    assert!(!fuzz_inventory_ok(
        &manifest.replace(
            "name = \"b\"\npath = \"fuzz_targets/b.rs\"",
            "name = \"a\"\npath = \"fuzz_targets/a.rs\""
        ),
        &two
    ));
    // RED — required-features: `--bins` skips the target without a word.
    assert!(!fuzz_inventory_ok(
        &manifest.replacen("test = false", "required-features = [\"x\"]", 1),
        &two
    ));
    // RED — a block with no path.
    assert!(!fuzz_inventory_ok(
        &manifest.replace("path = \"fuzz_targets/b.rs\"\n", ""),
        &two
    ));
    // RED — the target's body sits behind cargo-fuzz's cfg: the stable check sees nothing.
    let hidden = [
        (
            "a.rs".to_owned(),
            body.replace("fuzz_target!", "#[cfg(fuzzing)]\nfuzz_target!"),
        ),
        ("b.rs".to_owned(), body.to_owned()),
    ];
    assert!(!fuzz_inventory_ok(manifest, &hidden));
    // RED — nothing declared: a check of no target is no check.
    assert!(!fuzz_inventory_ok("[package]\nname = \"ae-fuzz\"\n", &[]));
}

/// Each way the justfile's `FUZZ_TARGETS` and the fuzz README's `## Targets`
/// table (first column) disagree with the fuzz manifest's `[[bin]]` names, one
/// line per entry naming its list. The lists are compared as sets; a name
/// twice in any one of them (the manifest included) is reported by name.
fn fuzz_list_problems(manifest: &str, justfile: &str, readme: &str) -> Vec<String> {
    use std::collections::BTreeSet;
    let Some(bins) = manifest_bins(manifest) else {
        return vec!["manifest: a [[bin]] block lacks its name or path".to_owned()];
    };
    let bins: Vec<String> = bins.into_iter().map(|(name, _)| name).collect();
    let just = justfile
        .lines()
        .find_map(|line| line.strip_prefix("FUZZ_TARGETS := \""))
        .and_then(|rest| rest.split_once('"'))
        .map_or("", |(names, _)| names);
    let just: Vec<String> = just.split_whitespace().map(str::to_owned).collect();
    let rows: Vec<String> = readme
        .lines()
        .skip_while(|line| *line != "## Targets")
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .filter_map(|line| line.strip_prefix("| `")?.split_once('`'))
        .map(|(name, _)| name.to_owned())
        .collect();
    let lists = [("manifest", &bins), ("justfile", &just), ("README", &rows)];
    let mut problems = Vec::new();
    for (label, names) in lists {
        let mut seen = BTreeSet::new();
        for name in names.iter().filter(|name| !seen.insert(*name)) {
            problems.push(format!("{name}: duplicated in {label}"));
        }
    }
    let wanted: BTreeSet<_> = bins.iter().collect();
    for (label, names) in &lists[1..] {
        let have: BTreeSet<_> = names.iter().collect();
        for name in wanted.difference(&have) {
            problems.push(format!("{name}: missing from {label}"));
        }
        for name in have.difference(&wanted) {
            problems.push(format!("{name}: unknown in {label}"));
        }
    }
    problems
}

#[test]
fn the_fuzz_target_lists_in_justfile_manifest_and_readme_name_the_same_targets() {
    let problems = fuzz_list_problems(
        &read(&root().join("fuzz/Cargo.toml")),
        &read(&root().join("justfile")),
        &read(&root().join("fuzz/README.md")),
    );
    assert!(
        problems.is_empty(),
        "fuzz/Cargo.toml [[bin]] names (manifest), justfile FUZZ_TARGETS and the fuzz/README.md \
         Targets table (README) must be one set: just rust-fuzz takes only FUZZ_TARGETS, so a \
         target missing there is refused or skipped without a word (#273): {problems:#?}"
    );

    let m = "[package]\nname = \"ae-fuzz\"\n\n[[bin]]\nname = \"a\"\npath = \"fuzz_targets/a.rs\"\n\n[[bin]]\nname = \"b\"\npath = \"fuzz_targets/b.rs\"\n";
    let r = "## Targets\n\n| Target | Parser |\n|---|---|\n| `a` | x |\n| `b` | y |\n\n## Next\n\n| `z` | no target |\n";
    let check = |m: &str, names: &str, r: &str| {
        fuzz_list_problems(m, &format!("FUZZ_TARGETS := \"{names}\"\n"), r)
    };
    assert!(check(m, "a b", r).is_empty());
    // RED — a target the justfile does not list: `rust-fuzz target=b` refuses.
    assert_eq!(check(m, "a", r), ["b: missing from justfile"]);
    // RED — a listed name no bin declares, and one listed twice.
    assert_eq!(check(m, "a b c", r), ["c: unknown in justfile"]);
    assert_eq!(check(m, "a b b", r), ["b: duplicated in justfile"]);
    // RED — the README table swaps one name, and lists another twice.
    let swapped = check(m, "a b", &r.replace("`b`", "`c`"));
    assert_eq!(swapped, ["b: missing from README", "c: unknown in README"]);
    let dup = check(m, "a b", &r.replace("`b`", "`a`"));
    assert_eq!(dup, ["a: duplicated in README", "b: missing from README"]);
    // RED — a bin declared twice, and a [[bin]] block with no path: named, not an empty list.
    let dup = check(&m.replace("\"b\"", "\"a\""), "a", r);
    assert_eq!(dup, ["a: duplicated in manifest", "b: unknown in README"]);
    let bad = check(&m.replace("path = \"fuzz_targets/b.rs\"\n", ""), "a b", r);
    assert_eq!(bad, ["manifest: a [[bin]] block lacks its name or path"]);
}

/// The `paths` filter one trigger of a workflow lists, in order.
fn trigger_paths(workflow: &str, trigger: &str) -> Vec<String> {
    let header = format!("  {trigger}:");
    let mut out = Vec::new();
    let mut listing = false;
    for line in workflow
        .lines()
        .skip_while(|line| *line != header.as_str())
        .skip(1)
    {
        if !line.starts_with("    ") && !line.trim().is_empty() {
            break;
        }
        if line == "    paths:" {
            listing = true;
        } else if let Some(item) = line.strip_prefix("      - ").filter(|_| listing) {
            out.push(item.trim().to_owned());
        } else if !line.trim_start().starts_with('#') {
            listing = false;
        }
    }
    out
}

/// Whether a change to the fuzz crate alone starts the workflow that now
/// type-checks it, on push and on pull request alike.
fn fuzz_ci_triggers_ok(workflow: &str) -> bool {
    let push = trigger_paths(workflow, "push");
    let pull = trigger_paths(workflow, "pull_request");
    let reaches = |file: &str| push.iter().any(|path| path == file || path == "fuzz/**");
    !push.is_empty()
        && push == pull
        && ["fuzz/Cargo.toml", "fuzz/Cargo.lock", "fuzz/fuzz_targets/**"]
            .iter()
            .all(|file| reaches(file))
        && workflow.contains("run: just rust-check")
}

#[test]
fn a_fuzz_only_change_starts_the_gate_and_no_workflow_carries_a_nightly() {
    let rust = read(&root().join(".github/workflows/rust.yml"));
    assert!(
        fuzz_ci_triggers_ok(&rust),
        "both rust.yml triggers must list fuzz/Cargo.toml, fuzz/Cargo.lock and \
         fuzz/fuzz_targets/** (or fuzz/**), identically, and the job must run just \
         rust-check: a target break shows at the commit that causes it (#240)"
    );
    // The fuzz runs stay human-run: no workflow line installs or runs a nightly.
    let dir = root().join(".github/workflows");
    for entry in std::fs::read_dir(&dir).unwrap_or_else(|why| panic!("{}: {why}", dir.display())) {
        let path = entry.expect("a workflows entry").path();
        let text = read(&path);
        for line in text
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
        {
            assert!(
                !line.contains("nightly")
                    && !line.contains("cargo fuzz")
                    && !line.contains("cargo-fuzz"),
                "{}: CI carries no nightly and runs no fuzzer: {line}",
                path.display()
            );
        }
    }

    let good = "on:\n  push:\n    branches: [main]\n    # comment\n    paths:\n      - src/**\n      - fuzz/Cargo.toml\n      - fuzz/Cargo.lock\n      - fuzz/fuzz_targets/**\n  pull_request:\n    branches: [main]\n    paths:\n      - src/**\n      - fuzz/Cargo.toml\n      - fuzz/Cargo.lock\n      - fuzz/fuzz_targets/**\n  # the manual escape hatch\n  workflow_dispatch:\njobs:\n  rust:\n    steps:\n      - name: just rust-check\n        run: just rust-check\n";
    assert!(fuzz_ci_triggers_ok(good));
    // GREEN — the whole crate directory as one entry.
    assert!(fuzz_ci_triggers_ok(&good.replace(
        "      - fuzz/Cargo.toml\n      - fuzz/Cargo.lock\n      - fuzz/fuzz_targets/**\n",
        "      - fuzz/**\n"
    )));
    // RED — the filter before #240: a target-only commit starts nothing.
    assert!(!fuzz_ci_triggers_ok(&good.replace(
        "      - fuzz/Cargo.toml\n      - fuzz/Cargo.lock\n      - fuzz/fuzz_targets/**\n",
        ""
    )));
    // RED — a lock-only commit (a dependency bump) starts nothing.
    assert!(!fuzz_ci_triggers_ok(
        &good.replace("      - fuzz/Cargo.lock\n", "")
    ));
    // RED — the two hand-synced lists drifted: pull requests miss target edits.
    assert!(!fuzz_ci_triggers_ok(&good.replace(
        "      - fuzz/fuzz_targets/**\n  # the manual",
        "  # the manual"
    )));
    // RED — triggered, but the job no longer runs the gate.
    assert!(!fuzz_ci_triggers_ok(
        &good.replace("run: just rust-check", "run: just rust-test")
    ));
}

/// Whether the `AGENTS.md` lanes row for THE GATE names every recipe
/// `rust-check` runs: the contract says what the gate is.
fn gate_row_ok(agents: &str, justfile: &str) -> bool {
    let deps = recipe_deps(justfile, "rust-check");
    let Some(row) = agents
        .lines()
        .find(|line| line.starts_with("| `just test` / `just rust-check` |"))
    else {
        return false;
    };
    !deps.is_empty() && deps.iter().all(|dep| row.contains(&format!("`{dep}`")))
}

#[test]
fn the_contract_names_every_lane_the_gate_runs() {
    assert!(
        gate_row_ok(
            &read(&root().join("AGENTS.md")),
            &read(&root().join("justfile"))
        ),
        "the AGENTS.md lanes row for just test / just rust-check must name every \
         rust-check dependency"
    );

    let justfile = "rust-check: rust-fmt-check rust-lint rust-fuzz-check rust-test\n";
    let row = "| `just test` / `just rust-check` | **the gate**: `rust-fmt-check` + `rust-lint` + `rust-fuzz-check` + `rust-test` |\n";
    assert!(gate_row_ok(row, justfile));
    // RED — the row before #240 against the gate after it.
    assert!(!gate_row_ok(
        &row.replace(" + `rust-fuzz-check`", ""),
        justfile
    ));
    // RED — no such row.
    assert!(!gate_row_ok("| `just test` | the gate |\n", justfile));
}

// ── `just pre` and the lane lock ─────────────────────────────────────────

/// `just pre` is a convenience in front of the gate, never part of it: `test`
/// and `rust-check` keep exactly the dependencies they had, `pre` runs only
/// the three checks it names through the lane's filtered arm, retries nothing,
/// and the contract says in plain words that it is advisory.
#[test]
fn the_advisory_pre_check_stays_out_of_the_gate_and_the_contract_says_so() {
    let justfile = read(&root().join("justfile"));
    assert_eq!(recipe_deps(&justfile, "test"), ["rust-check"]);
    assert_eq!(
        recipe_deps(&justfile, "rust-check"),
        [
            "rust-fmt-check",
            "rust-lint",
            "rust-fuzz-check",
            "rust-test"
        ],
        "the gate is exactly what it was; `pre` is not a member of it"
    );
    assert_eq!(
        recipe_deps(&justfile, "pre"),
        ["rust-fmt-check", "rust-lint", "_pre-test"]
    );
    let body = recipe_text(&justfile, "_pre-test:").join(" ");
    assert!(
        body.starts_with("just _tmux-isolated test -E '"),
        "the unit and guard tests run through the lane's filtered arm: {body}"
    );
    for module in ["doors", "floor", "gate"] {
        assert!(body.contains(module), "the guard module {module} is run");
    }
    assert!(
        !body.contains("retries") && !body.contains("retry"),
        "a pre-check never retries: {body}"
    );

    let agents = read(&root().join("AGENTS.md"));
    let row = agents
        .lines()
        .find(|line| line.starts_with("| `just pre` |"))
        .unwrap_or_default();
    assert!(
        row.contains("ADVISORY") && row.contains("NOT the gate"),
        "the AGENTS.md lanes table must call `just pre` advisory and not the gate: {row:?}"
    );
    let development = read(&root().join("docs/development.md"));
    assert!(
        development
            .lines()
            .any(|line| line.contains("`just pre`") && line.to_lowercase().contains("advisory")),
        "docs/development.md must call `just pre` advisory"
    );
}

/// The capture files of one child, named so concurrent callers never share one.
fn next_capture_tag() -> usize {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// One command in `cwd` with the fixture's own `HOME` and git config:
/// `(exit code, stdout, stderr)`.
fn run_in(scratch: &Path, cwd: &Path, program: &str, args: &[&str]) -> (i32, String, String) {
    run_in_env(scratch, cwd, program, args, &[])
}

/// [`run_in`] with extra environment variables.
fn run_in_env(
    scratch: &Path,
    cwd: &Path,
    program: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> (i32, String, String) {
    let mut invocation = Invocation::new(program)
        .env("HOME", scratch)
        .env("GIT_CONFIG_GLOBAL", scratch.join("gitconfig"));
    for (key, value) in env {
        invocation = invocation.env(key, value);
    }
    for arg in args {
        invocation = invocation.arg(arg);
    }
    let tag = next_capture_tag();
    let io = scratch.join("io");
    let (out, err) = (io.join(format!("{tag}.out")), io.join(format!("{tag}.err")));
    let status = raw::run(&invocation, cwd, &out, &err)
        .unwrap_or_else(|why| panic!("{program} must be runnable: {why}"));
    let code = match status.outcome() {
        ExitOutcome::Code(code) => code,
        ExitOutcome::Signalled => -1,
    };
    (
        code,
        std::fs::read_to_string(out).unwrap_or_default(),
        std::fs::read_to_string(err).unwrap_or_default(),
    )
}

/// A scratch main checkout holding the REAL justfile and a linked worktree of
/// it: `(scratch owner, main, linked)`. The lock root is the main checkout's.
/// `tag` keeps concurrent tests of one process on separate roots.
fn lane_lock_checkouts(tag: &str) -> (super::cli::OwnedScratch, PathBuf, PathBuf) {
    let scratch = super::cli::OwnedScratch::root("gate", &format!("lanelock.{tag}"));
    let (main, linked) = (scratch.join("main"), scratch.join("linked"));
    assert!(std::fs::create_dir_all(&main).is_ok(), "a main checkout");
    assert!(
        std::fs::create_dir_all(scratch.join("io")).is_ok(),
        "captures"
    );
    assert!(
        std::fs::copy(root().join("justfile"), main.join("justfile")).is_ok(),
        "the fixture carries the real justfile"
    );
    for words in [
        vec!["init", "-q", "-b", "main"],
        vec!["add", "justfile"],
        vec![
            "-c",
            "user.email=lock@example.invalid",
            "-c",
            "user.name=Lock",
            "commit",
            "-qm",
            "fixture",
        ],
        vec!["worktree", "add", "-q", "../linked", "-b", "linked"],
    ] {
        let (code, _, err) = run_in(&scratch, &main, "git", &words);
        assert_eq!(code, 0, "git {words:?} must succeed: {err}");
    }
    (scratch, main, linked)
}

/// `just lane-lock <args>` in `cwd`.
fn lane_lock(scratch: &Path, cwd: &Path, args: &[&str]) -> (i32, String, String) {
    lane_lock_env(scratch, cwd, args, &[])
}

/// [`lane_lock`] with extra environment variables.
fn lane_lock_env(
    scratch: &Path,
    cwd: &Path,
    args: &[&str],
    env: &[(&str, &str)],
) -> (i32, String, String) {
    let mut words = vec!["lane-lock"];
    words.extend_from_slice(args);
    run_in_env(scratch, cwd, "just", &words, env)
}

/// The claim is one exclusive create of the owner line, so a worktree meets the main checkout's
/// lock; only the recorded owner releases it, by exact name; nothing in the
/// recipe clears a lock it did not write.
#[test]
fn the_lane_lock_is_one_lock_for_every_checkout_and_only_its_owner_releases_it() {
    let (scratch, main, linked) = lane_lock_checkouts("owner");
    let owner = main.join(".local/locks/gate/owner");
    let (code, out, _) = lane_lock(&scratch, &main, &["status"]);
    assert_eq!((code, out.contains("gate: free")), (0, true), "{out}");

    let (code, out, err) = lane_lock(&scratch, &main, &["claim", "alpha"]);
    assert_eq!(code, 0, "{out}{err}");
    let recorded = read(&owner);
    assert!(
        recorded.starts_with("alpha 20") && recorded.trim_end().ends_with('Z'),
        "the owner line is `<name> <UTC timestamp>`: {recorded:?}"
    );

    // From the linked worktree: the same lock, refused, nothing written there.
    let (code, _, err) = lane_lock(&scratch, &linked, &["claim", "beta"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("alpha"), "the holder is named: {err}");
    let (code, _, err) = lane_lock(&scratch, &linked, &["claim", "alpha"]);
    assert_eq!(
        code, 1,
        "a second claim on the owner's own name is refused: {err}"
    );
    assert!(err.contains("already holds"), "{err}");
    assert!(
        !linked.join(".local").exists(),
        "the worktree has no lock root of its own"
    );

    // Release is the owner's, by exact name.
    for impostor in ["beta", "alph", "alpha2"] {
        let (code, _, err) = lane_lock(&scratch, &linked, &["release", impostor]);
        assert_eq!(code, 1, "{impostor} must not release alpha's lock: {err}");
        assert!(err.contains("lock left intact"), "{err}");
        assert_eq!(read(&owner), recorded, "the lock is untouched");
    }
    let (code, out, _) = lane_lock(&scratch, &linked, &["status"]);
    assert_eq!(code, 1);
    assert!(
        out.contains("held by alpha"),
        "status names the owner: {out}"
    );

    let (code, out, err) = lane_lock(&scratch, &linked, &["release", "alpha"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(!main.join(".local/locks/gate").exists(), "released");
    let (code, _, err) = lane_lock(&scratch, &main, &["release", "alpha"]);
    assert_eq!(code, 1, "nothing is held, so nothing is released: {err}");
}

/// The queue file is honoured, a lock taken by hand holds off the recipe and
/// the reverse, and a lock that looks abandoned is REPORTED and left standing.
#[test]
fn the_lane_lock_keeps_the_hand_protocol_and_reports_a_stale_lock_without_touching_it() {
    let (scratch, main, _linked) = lane_lock_checkouts("hand");
    let locks = main.join(".local/locks");
    let (gate, owner) = (locks.join("gate"), locks.join("gate/owner"));

    // `next` names the one slice that may claim.
    assert!(std::fs::create_dir_all(&locks).is_ok());
    assert!(std::fs::write(locks.join("next"), "gamma\n").is_ok());
    let (code, _, err) = lane_lock(&scratch, &main, &["claim", "alpha"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("next is gamma") && !gate.exists(), "{err}");
    let (code, out, err) = lane_lock(&scratch, &main, &["claim", "gamma"]);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(
        read(&locks.join("next")),
        "gamma\n",
        "the queue file is the lead's"
    );
    assert_eq!(lane_lock(&scratch, &main, &["release", "gamma"]).0, 0);
    assert!(std::fs::write(locks.join("next"), "").is_ok());

    // Recipe holder, then hand claimant: the second `mkdir` is refused.
    assert_eq!(lane_lock(&scratch, &main, &["claim", "alpha"]).0, 0);
    assert!(
        std::fs::create_dir(&gate).is_err(),
        "the hand protocol meets the recipe's lock"
    );
    assert_eq!(lane_lock(&scratch, &main, &["release", "alpha"]).0, 0);

    // Hand holder, long gone by its timestamp: reported, never cleared.
    assert!(std::fs::create_dir(&gate).is_ok());
    let hand = "hand 2000-01-01T00:00:00Z\n";
    assert!(std::fs::write(&owner, hand).is_ok());
    // A leading zero is not a whole number of seconds: bash would read `08` as octal.
    for wait in ["08", "010", "00", "1x"] {
        let (code, _, err) = lane_lock(&scratch, &main, &["claim", "alpha", wait]);
        assert_eq!(code, 2, "wait {wait:?}: {err}");
        assert!(err.contains("whole seconds"), "{err}");
    }
    let (code, _, err) = lane_lock(&scratch, &main, &["claim", "alpha", "1"]);
    assert_eq!(code, 1, "{err}");
    // Whether a "waiting" line precedes it depends on how long the first attempt took.
    assert!(err.contains("held by hand"), "the holder is named: {err}");
    let (code, out, _) = lane_lock(&scratch, &main, &["status"]);
    assert_eq!(code, 1);
    assert!(
        out.contains("held by hand") && out.contains("suspect"),
        "{out}"
    );
    let (code, _, err) = lane_lock(&scratch, &main, &["release", "alpha"]);
    assert_eq!(code, 1, "{err}");
    assert_eq!(read(&owner), hand, "a stale lock is never touched");

    // A gate with no owner line (a claim mid-write, or one that died).
    assert!(std::fs::remove_file(&owner).is_ok());
    for args in [["claim", "alpha"], ["release", "alpha"]] {
        let (code, _, err) = lane_lock(&scratch, &main, &args);
        assert_eq!(code, 1, "{args:?}: {err}");
        assert!(err.contains("no owner"), "{args:?} says why: {err}");
        assert!(
            gate.is_dir(),
            "{args:?} must leave the ownerless gate standing"
        );
    }
    let (_, out, _) = lane_lock(&scratch, &main, &["status"]);
    assert!(out.contains("suspect"), "{out}");

    // More than the owner line in the gate: release refuses rather than `rm -rf`.
    assert!(std::fs::write(&owner, "alpha 2000-01-01T00:00:00Z\n").is_ok());
    assert!(std::fs::write(gate.join("junk"), "x").is_ok());
    let (code, _, err) = lane_lock(&scratch, &main, &["release", "alpha"]);
    assert_eq!(code, 1, "{err}");
    assert!(gate.join("junk").exists() && owner.exists(), "{err}");
}

/// Two claimants at the same instant: one `mkdir` wins, every round.
#[test]
fn two_simultaneous_lane_lock_claims_have_exactly_one_winner() {
    let (scratch, main, linked) = lane_lock_checkouts("race");
    for round in 0..8 {
        let start = std::sync::Barrier::new(2);
        let (first, second) = std::thread::scope(|scope| {
            let race = |cwd: &Path, name: &'static str| {
                let (scratch, start) = (&scratch, &start);
                let cwd = cwd.to_path_buf();
                scope.spawn(move || {
                    start.wait();
                    (name, lane_lock(scratch, &cwd, &["claim", name]).0)
                })
            };
            let (a, b) = (race(&main, "left"), race(&linked, "right"));
            (a.join().unwrap_or(("", -1)), b.join().unwrap_or(("", -1)))
        });
        let winners: Vec<&str> = [first, second]
            .into_iter()
            .filter(|(_, code)| *code == 0)
            .map(|(name, _)| name)
            .collect();
        assert_eq!(winners.len(), 1, "round {round}: {first:?} {second:?}");
        let recorded = read(&main.join(".local/locks/gate/owner"));
        assert!(
            recorded.starts_with(winners[0]),
            "round {round}: {recorded:?}"
        );
        assert_eq!(lane_lock(&scratch, &main, &["release", winners[0]]).0, 0);
    }
}

/// A release that stalls after its owner check cannot touch the NEXT holder's
/// lock. Whoever releases alpha meanwhile (this recipe, or the hand-typed
/// protocol, which takes no part in anything the recipe does), and whoever
/// claims after that, the stalled release resumes against the directory it
/// entered, finds that directory gone, and removes nothing. An `rm` shim stalls
/// it at the removal of its owner line, deterministically.
fn stalled_release_meets_a_new_holder(tag: &str, released_by_hand: bool) {
    let (scratch, main, _linked) = lane_lock_checkouts(tag);
    let shim = scratch.join("shim");
    assert!(std::fs::create_dir_all(&shim).is_ok(), "a shim directory");
    let body = "#!/bin/sh\n\
        case \"$1\" in owner | */owner) : >\"$AE_PAUSED\"; n=0\n\
        while [ ! -e \"$AE_GO\" ] && [ \"$n\" -lt 2400 ]; do sleep 0.05; n=$((n + 1)); done ;; esac\n\
        exec /bin/rm \"$@\"\n";
    assert!(std::fs::write(shim.join("rm"), body).is_ok(), "the rm shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert!(
            std::fs::set_permissions(shim.join("rm"), std::fs::Permissions::from_mode(0o755))
                .is_ok(),
            "an executable rm shim"
        );
    }
    let path = format!(
        "{}:{}",
        shim.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let (paused, go) = (scratch.join("paused"), scratch.join("go"));
    let env = [
        ("PATH", path.as_str()),
        ("AE_PAUSED", paused.to_str().unwrap_or_default()),
        ("AE_GO", go.to_str().unwrap_or_default()),
    ];
    assert_eq!(lane_lock(&scratch, &main, &["claim", "alpha"]).0, 0);
    let (gate, owner) = (
        main.join(".local/locks/gate"),
        main.join(".local/locks/gate/owner"),
    );

    let (first, meanwhile) = std::thread::scope(|scope| {
        let stalled = scope.spawn(|| lane_lock_env(&scratch, &main, &["release", "alpha"], &env));
        for _ in 0..1200 {
            if paused.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        // The first release has passed its owner check and is stalled at `rm`.
        // Alpha's lock is released by someone else, and beta claims it.
        let released = if released_by_hand {
            // "confirm the owner line starts alpha, rm owner, rmdir gate"
            let line = std::fs::read_to_string(&owner).unwrap_or_default();
            line.starts_with("alpha")
                && std::fs::remove_file(&owner).is_ok()
                && std::fs::remove_dir(&gate).is_ok()
        } else {
            lane_lock(&scratch, &main, &["release", "alpha"]).0 == 0
        };
        let claimed = lane_lock(&scratch, &main, &["claim", "beta"]).0 == 0;
        let beta = std::fs::read_to_string(&owner).unwrap_or_default();
        assert!(
            std::fs::write(&go, "").is_ok(),
            "let the stalled release go"
        );
        (
            stalled.join().unwrap_or((-1, String::new(), String::new())),
            (released, claimed, beta),
        )
    });
    let (released, claimed, beta) = meanwhile;
    assert!(
        paused.exists(),
        "the first release reached its removal: {first:?}"
    );
    assert!(released, "alpha's lock was released meanwhile");
    assert!(
        claimed && beta.starts_with("beta "),
        "beta claimed: {beta:?}"
    );
    assert_ne!(first.0, 0, "the stalled release removed nothing: {first:?}");
    assert_eq!(read(&owner), beta, "beta's lock is untouched: {first:?}");
    assert_eq!(
        lane_lock(&scratch, &main, &["claim", "gamma"]).0,
        1,
        "gamma cannot claim while beta holds the lock"
    );

    // A late release by alpha, run to completion, still finds beta's lock.
    let (code, _, err) = lane_lock(&scratch, &main, &["release", "alpha"]);
    assert_eq!(code, 1, "{err}");
    assert_eq!(read(&owner), beta);
}

#[test]
fn a_stalled_lane_lock_release_cannot_remove_the_lock_the_recipe_gave_to_the_next_holder() {
    stalled_release_meets_a_new_holder("stallr", false);
}

#[test]
fn a_stalled_lane_lock_release_cannot_remove_the_lock_a_hand_release_gave_to_the_next_holder() {
    stalled_release_meets_a_new_holder("stallh", true);
}

/// The `PATH` that puts a `mkdir` shim first. Ubuntu 26.04 ships uutils 0.10.0, whose `mkdir`
/// is not atomic: 1686 of 3000 two-way races of `mkdir t` both exited 0. The shim runs `hook`
/// (shell, the target in `$last`) for every `mkdir` that is not `-p`, then falls through to the
/// real one, so a hook that ends in `exit 0` is that defect, deterministically.
fn mkdir_shim_path(scratch: &Path, hook: &str) -> String {
    let shim = scratch.join("mkdir-shim");
    assert!(std::fs::create_dir_all(&shim).is_ok(), "a shim directory");
    let body = format!(
        "#!/bin/sh\ncase \"$1\" in -p) exec /bin/mkdir \"$@\" ;; esac\n\
         for last; do :; done\n{hook}\nexec /bin/mkdir \"$@\"\n"
    );
    assert!(
        std::fs::write(shim.join("mkdir"), body).is_ok(),
        "the mkdir shim"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert!(
            std::fs::set_permissions(shim.join("mkdir"), std::fs::Permissions::from_mode(0o755))
                .is_ok(),
            "an executable mkdir shim"
        );
    }
    format!(
        "{}:{}",
        shim.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

/// The measured defect itself: `mkdir` of a directory that exists reports success.
const MKDIR_SUCCEEDS_ON_AN_EXISTING_DIRECTORY: &str = "[ -d \"$last\" ] && exit 0";

/// On a host whose `mkdir` is not atomic, a claim over a gate that is already held is refused,
/// and the holder's owner line is never touched: not by another name, not by the holder's own,
/// and not when the gate stands with no owner line.
#[test]
fn a_lane_lock_claim_over_a_standing_gate_is_refused_where_mkdir_is_not_atomic() {
    let (scratch, main, linked) = lane_lock_checkouts("naheld");
    let owner = main.join(".local/locks/gate/owner");
    let path = mkdir_shim_path(&scratch, MKDIR_SUCCEEDS_ON_AN_EXISTING_DIRECTORY);
    let env = [("PATH", path.as_str())];

    assert_eq!(lane_lock(&scratch, &main, &["claim", "alpha"]).0, 0);
    let held = read(&owner);
    assert!(held.starts_with("alpha "), "alpha holds: {held:?}");

    let (code, out, err) = lane_lock_env(&scratch, &linked, &["claim", "beta"], &env);
    assert_eq!(code, 1, "beta must not take alpha's gate: {out}{err}");
    assert!(err.contains("held by alpha"), "the holder is named: {err}");
    assert_eq!(read(&owner), held, "alpha's owner line is untouched");

    let (code, out, err) = lane_lock_env(&scratch, &main, &["claim", "alpha"], &env);
    assert_eq!(code, 1, "the holder's own name is refused too: {out}{err}");
    assert!(err.contains("already holds"), "{err}");
    assert_eq!(read(&owner), held, "alpha's owner line is untouched");

    // A gate with no owner line is held, not free: the recipe never claims it.
    assert!(std::fs::remove_file(&owner).is_ok());
    let (code, out, err) = lane_lock_env(&scratch, &linked, &["claim", "beta"], &env);
    assert_eq!(code, 1, "an ownerless gate is not claimed: {out}{err}");
    assert!(err.contains("no owner"), "{err}");
    assert!(
        main.join(".local/locks/gate").is_dir() && !owner.exists(),
        "the ownerless gate is left as it was"
    );
}

/// The race the defect opens: another claimant creates the gate and its owner line between
/// this claim's look and its `mkdir`, and `mkdir` still reports success. The owner line is
/// created exclusively, so the later claimant loses, exits 1 and leaves the winner's line alone.
#[test]
fn a_lane_lock_claim_that_loses_the_race_is_refused_where_mkdir_is_not_atomic() {
    let (scratch, main, _linked) = lane_lock_checkouts("narace");
    let owner = main.join(".local/locks/gate/owner");
    let rival = "alpha 2000-01-01T00:00:00Z\n";
    let hook = format!(
        "if [ ! -d \"$last\" ]; then /bin/mkdir \"$last\" && printf '%s' '{rival}' >\"$last/owner\"; fi\n{MKDIR_SUCCEEDS_ON_AN_EXISTING_DIRECTORY}"
    );
    let path = mkdir_shim_path(&scratch, &hook);

    let (code, out, err) = lane_lock_env(&scratch, &main, &["claim", "beta"], &[("PATH", &path)]);
    assert_eq!(code, 1, "the later claimant must lose: {out}{err}");
    assert!(!out.contains("claimed"), "{out}");
    assert!(err.contains("held by alpha"), "the winner is named: {err}");
    assert_eq!(read(&owner), rival, "the winner's owner line is untouched");
}

/// `mkdir` reports success, the holder releases before the claim can write its owner line, so the
/// gate is gone: that is a free lock, and the claim takes it. It is never `could not create`.
#[test]
fn a_lane_lock_claim_retries_when_the_gate_vanishes_under_it() {
    let (scratch, main, _linked) = lane_lock_checkouts("navanish");
    let once = scratch.join("once");
    let hook = format!(
        "if [ ! -e '{once}' ]; then : >'{once}'; exit 0; fi",
        once = once.display()
    );
    let path = mkdir_shim_path(&scratch, &hook);

    let (code, out, err) = lane_lock_env(&scratch, &main, &["claim", "beta"], &[("PATH", &path)]);
    assert_eq!(
        code, 0,
        "a vanished gate is a retry, not a refusal: {out}{err}"
    );
    assert!(once.exists(), "the shim reported its one false success");
    assert!(!err.contains("could not create"), "{err}");
    let recorded = read(&main.join(".local/locks/gate/owner"));
    assert!(recorded.starts_with("beta 20"), "beta holds: {recorded:?}");
}

/// Two claimants at the same instant under a `mkdir` that always reports success: still exactly
/// one winner per round, and the owner line is the winner's, whole.
#[test]
fn two_simultaneous_lane_lock_claims_have_exactly_one_winner_where_mkdir_is_not_atomic() {
    let (scratch, main, linked) = lane_lock_checkouts("narace2");
    let path = mkdir_shim_path(&scratch, "/bin/mkdir -p \"$last\"; exit 0");
    for round in 0..8 {
        let start = std::sync::Barrier::new(2);
        let (first, second) = std::thread::scope(|scope| {
            let race = |cwd: &Path, name: &'static str| {
                let (scratch, start, path) = (&scratch, &start, &path);
                let cwd = cwd.to_path_buf();
                scope.spawn(move || {
                    start.wait();
                    let env = [("PATH", path.as_str())];
                    (name, lane_lock_env(scratch, &cwd, &["claim", name], &env).0)
                })
            };
            let (a, b) = (race(&main, "left"), race(&linked, "right"));
            (a.join().unwrap_or(("", -1)), b.join().unwrap_or(("", -1)))
        });
        let winners: Vec<&str> = [first, second]
            .into_iter()
            .filter(|(_, code)| *code == 0)
            .map(|(name, _)| name)
            .collect();
        assert_eq!(winners.len(), 1, "round {round}: {first:?} {second:?}");
        let recorded = read(&main.join(".local/locks/gate/owner"));
        assert!(
            recorded.starts_with(&format!("{} 20", winners[0])) && recorded.ends_with("Z\n"),
            "round {round}: {recorded:?}"
        );
        assert_eq!(lane_lock(&scratch, &main, &["release", winners[0]]).0, 0);
    }
}

/// A `mkdir` that fails with no gate standing is still the refusal it always was, at once.
#[test]
fn a_lane_lock_claim_refuses_when_mkdir_fails_and_no_gate_stands() {
    let (scratch, main, _linked) = lane_lock_checkouts("nafail");
    let path = mkdir_shim_path(&scratch, "echo 'mkdir: simulated failure' >&2; exit 1");

    let (code, out, err) = lane_lock_env(&scratch, &main, &["claim", "beta"], &[("PATH", &path)]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(err.contains("could not create"), "{err}");
    assert!(
        err.contains("simulated failure"),
        "mkdir's own words: {err}"
    );
    assert!(!main.join(".local/locks/gate").exists(), "no gate was made");
}

// ── the on-demand live harness smoke ─────────────────────────────────────

/// The words that mean "run ignored tests" or "open the live smoke's gate".
const SMOKE_REACH: [&str; 3] = ["run-ignored", "--ignored", "AE_HARNESS_SMOKE"];

/// Every way the live harness smoke is reachable from somewhere it must not be,
/// or runs without what makes it safe, as plain sentences.
///
/// `just harness-smoke` spends real model turns on real logins, so the only way
/// into it is typing it: the recipe is nobody's dependency, nothing else in the
/// justfile or the workflows runs ignored tests or sets the gate variable, and
/// the recipe itself runs the locked, ignored-only, exact-name selection.
fn harness_smoke_leaks(justfile: &str, others: &[(&str, &str)]) -> Vec<String> {
    let mut leaks = Vec::new();
    let body = recipe_text(justfile, "harness-smoke ").join(" ");
    if body.is_empty() {
        leaks.push("the `harness-smoke` recipe is missing".to_owned());
    }
    for needed in [
        "AE_HARNESS_SMOKE=1",
        "TMUX_TMPDIR",
        "jobs-budget",
        "NEXTEST_TEST_THREADS=1",
        "nice -n 10 cargo nextest run",
        "--locked",
        "--run-ignored only",
        "--no-capture",
        "binary(it)",
        "test(=harness_smoke::live_harness_smoke)",
        "test(=harness_smoke::live_harness_smoke_hang_control)",
    ] {
        if !body.is_empty() && !body.contains(needed) {
            leaks.push(format!("the recipe lost `{needed}`"));
        }
    }
    // The justfile minus that recipe: header, indented body and blank lines.
    let mut rest = Vec::new();
    let mut inside = false;
    for line in justfile.lines() {
        if line.starts_with("harness-smoke ") {
            inside = true;
            continue;
        }
        if inside && (line.starts_with([' ', '\t']) || line.is_empty()) {
            continue;
        }
        inside = false;
        rest.push(line);
    }
    let live = |line: &str| !line.trim_start().starts_with('#');
    for line in rest.iter().copied().filter(|line| live(line)) {
        for word in SMOKE_REACH.iter().copied().chain(["harness-smoke"]) {
            if line.contains(word) {
                leaks.push(format!(
                    "the justfile reaches the smoke outside its recipe: {line:?}"
                ));
            }
        }
    }
    for (name, text) in others {
        for line in text.lines().filter(|line| live(line)) {
            for word in SMOKE_REACH.iter().copied().chain(["harness-smoke"]) {
                if line.contains(word) {
                    leaks.push(format!("{name} reaches the smoke: {line:?}"));
                }
            }
        }
    }
    leaks
}

/// What the live smoke's test file must keep: exactly two ignored tests, each
/// opening with the gate check, no process door of its own, and no tmux read
/// that files a pane's text under the scratch.
fn harness_smoke_source_leaks(text: &str) -> Vec<String> {
    let mut leaks = Vec::new();
    let ignored = text.matches("#[ignore = \"live:").count();
    if ignored != 2 {
        leaks.push(format!(
            "{ignored} live ignored tests, not exactly the two named"
        ));
    }
    for name in ["live_harness_smoke", "live_harness_smoke_hang_control"] {
        let body = text
            .split_once(&format!("fn {name}() {{"))
            .map_or("", |(_, rest)| rest);
        if !body.trim_start().starts_with("if !gated() {") {
            leaks.push(format!("`{name}` does not open with the gate check"));
        }
    }
    if !text.contains("value == Some(\"1\")") {
        leaks.push("the gate variable is no longer compared exactly to `1`".to_owned());
    }
    for door in [
        "std::process::Command",
        "run_tmux",
        "capture_tmux",
        "Invocation::new",
    ] {
        let code: String = text
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        if code.contains(door) {
            leaks.push(format!(
                "the smoke names `{door}`; it reuses the cli doors and reads panes in memory"
            ));
        }
    }
    leaks
}

#[test]
fn the_live_harness_smoke_is_reachable_only_by_typing_its_recipe() {
    let justfile = read(&root().join("justfile"));
    let rust = read(&root().join(".github/workflows/rust.yml"));
    let release = read(&root().join(".github/workflows/release.yml"));
    let mutants = read(&root().join(".cargo/mutants.toml"));
    let nextest = read(&root().join(".config/nextest.toml"));
    let leaks = harness_smoke_leaks(
        &justfile,
        &[
            ("rust.yml", &rust),
            ("release.yml", &release),
            ("mutants.toml", &mutants),
            ("nextest.toml", &nextest),
        ],
    );
    assert!(leaks.is_empty(), "{leaks:#?}");
    for gate in ["test", "rust-check", "pre", "rust-test"] {
        assert!(
            !recipe_deps(&justfile, gate)
                .iter()
                .any(|dep| dep.contains("harness")),
            "`{gate}` must not depend on the live smoke"
        );
    }
    let source = read(&root().join("tests/it/harness_smoke.rs"));
    let leaks = harness_smoke_source_leaks(&source);
    assert!(leaks.is_empty(), "{leaks:#?}");
}

#[test]
fn the_live_harness_smoke_guards_catch_every_way_of_reaching_it() {
    let good = "test: rust-check\n\
                harness-smoke mode=\"\":\n    \
                export TMUX_TMPDIR=lane\n    command -v jobs-budget\n    export NEXTEST_TEST_THREADS=1\n    \
                AE_HARNESS_SMOKE=1 nice -n 10 cargo nextest run --locked --run-ignored only --no-capture \
                -E \"binary(it) & test(=harness_smoke::live_harness_smoke) test(=harness_smoke::live_harness_smoke_hang_control)\"\n\
                rust-test:\n    just _tmux-isolated test\n";
    assert!(
        harness_smoke_leaks(good, &[]).is_empty(),
        "{:?}",
        harness_smoke_leaks(good, &[])
    );

    // A gate recipe that depends on it.
    let wired = good.replace("test: rust-check", "test: rust-check harness-smoke");
    assert!(!harness_smoke_leaks(&wired, &[]).is_empty());
    // Another recipe that runs ignored tests.
    let ignored = good.replace(
        "just _tmux-isolated test",
        "cargo nextest run --run-ignored all",
    );
    assert!(!harness_smoke_leaks(&ignored, &[]).is_empty());
    // A workflow that names the recipe or sets the gate variable.
    assert!(!harness_smoke_leaks(good, &[("ci", "      - run: just harness-smoke")]).is_empty());
    assert!(!harness_smoke_leaks(good, &[("ci", "  AE_HARNESS_SMOKE: 1")]).is_empty());
    // A comment naming it is not a reach.
    assert!(harness_smoke_leaks(good, &[("ci", "# just harness-smoke is on demand")]).is_empty());
    // A recipe that lost its safety words, or is gone.
    assert!(!harness_smoke_leaks(&good.replace("--locked ", ""), &[]).is_empty());
    // The lane sizing: no nice, no jobs-budget, no single test thread.
    assert!(!harness_smoke_leaks(&good.replace("nice -n 10 ", ""), &[]).is_empty());
    assert!(!harness_smoke_leaks(&good.replace("jobs-budget", "true"), &[]).is_empty());
    assert!(!harness_smoke_leaks(&good.replace("NEXTEST_TEST_THREADS=1", "X=1"), &[]).is_empty());
    assert!(
        !harness_smoke_leaks(
            &good.replace("--run-ignored only", "--run-ignored all"),
            &[]
        )
        .is_empty()
    );
    assert!(!harness_smoke_leaks("test: rust-check\n", &[]).is_empty());

    let source = "#[test]\n#[ignore = \"live: a\"]\nfn live_harness_smoke() {\n    if !gated() {\n        return;\n    }\n}\n\
                  #[test]\n#[ignore = \"live: b\"]\nfn live_harness_smoke_hang_control() {\n    if !gated() {\n        return;\n    }\n}\n\
                  fn opens(value: Option<&str>) -> bool { value == Some(\"1\") }\n";
    assert!(
        harness_smoke_source_leaks(source).is_empty(),
        "{:?}",
        harness_smoke_source_leaks(source)
    );
    // A third ignored test, an ungated body, a loosened gate and a process door.
    assert!(!harness_smoke_source_leaks(&format!("{source}#[ignore = \"live: c\"]\n")).is_empty());
    assert!(
        !harness_smoke_source_leaks(&source.replacen(
            "if !gated() {",
            "let x = 1; if !gated() {",
            1
        ))
        .is_empty()
    );
    assert!(
        !harness_smoke_source_leaks(&source.replace("value == Some(\"1\")", "value.is_some()"))
            .is_empty()
    );
    assert!(
        !harness_smoke_source_leaks(&format!(
            "{source}fn x() {{ std::process::Command::new(\"ls\"); }}\n"
        ))
        .is_empty()
    );
    assert!(
        !harness_smoke_source_leaks(&format!("{source}fn x() {{ run_tmux(&[], p); }}\n"))
            .is_empty()
    );
}
