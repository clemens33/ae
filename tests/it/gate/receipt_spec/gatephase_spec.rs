//! Independent #342 acceptance, authored before implementation.
//! Lead ruling ae-20261116T073047Z-c8e5b78c scopes long gate steps and
//! preserves direct, free lint/fuzz output, exit status, arguments and IO.
//! Real recipes, claims and census decide behavior. Only external build
//! tools are fixtures. The real ps supplies every process observation.

use super::*;

const PAYLOAD: &str = r#"#!/usr/bin/env bash
kind=$1
shift
dir=$AE_PHASE_DIR
identity() { LC_ALL=C ps -o lstart= -p "$1" 2>/dev/null | tr -cd A-Za-z0-9; }
record() {
    printf '%s %s\n' "$1" "$(identity "$1")" >"$dir/$2.tmp"
    mv "$dir/$2.tmp" "$dir/$2"
}
printf '%s\n' "$kind" >>"$dir/events"
if [[ "$kind" == taplo-fmt && "$AE_PHASE_TARGET" == fmt && "$AE_PHASE_MODE" == clean ]]; then
    printf 'taplo-out\n'
    printf 'taplo-err\n' >&2
    exit 0
fi
if [[ "$kind" != "$AE_PHASE_TARGET" ]]; then exit 0; fi
printf '%s\n' "$@" >"$dir/arguments"
if [[ "$AE_PHASE_MODE" == clean ]]; then
    if [[ "$kind" == fmt ]]; then
        printf 'formatter-note\n' >&2
        if [[ "$(cat src/lib.rs)" == 'fn phase_fmt() {}' ]]; then exit 0; fi
        printf 'Diff in %s/src/lib.rs:1:\n-fn phase_fmt(){}\n+fn phase_fmt() {}\n' "$PWD"
        exit 1
    fi
    IFS= read -r input || exit 84
    printf '%s\n' "$input" >"$dir/input-seen"
    printf 'cargo-out:%s:%s\n' "$kind" "$input"
    printf 'cargo-err:%s\n' "$kind" >&2
    exit "$AE_PHASE_EXIT"
fi
record "$$" payload
case "$kind" in
    fmt|lint|fuzz|selector|doctest)
        # Cargo exits on a signal while its compiler can survive it. Waiting
        # only for Cargo must not pass. Both waits use shell builtins.
        trap 'exit 130' INT
        trap 'exit 143' TERM
        trap 'exit 129' HUP
        exec 9<>"$dir/hold"
        bash -c '
            trap "" INT TERM HUP
            stamp=$(LC_ALL=C ps -o lstart= -p $$ | tr -cd A-Za-z0-9)
            printf "%s %s\n" "$$" "$stamp" >"$AE_PHASE_DIR/child.tmp"
            mv "$AE_PHASE_DIR/child.tmp" "$AE_PHASE_DIR/child"
            IFS= read -r finish <&9
        ' &
        child=$!
        while [[ ! -s "$dir/child" ]]; do sleep 0.01; done
        printf 'ready\n' >"$dir/ready"
        wait "$child"
        exit 0 ;;
    *)
        # taplo, shellcheck and shfmt obey TERM and have no compiler tree.
        printf 'ready\n' >"$dir/ready"
        exec sleep 300 ;;
esac
"#;

const CARGO: &str = r#"#!/usr/bin/env bash
kind=
case "$1:$2" in
    fmt:*) kind=fmt ;;
    clippy:*) kind=lint ;;
    check:*) kind=fuzz ;;
    run:*) kind=selector ;;
    test:--doc) kind=doctest ;;
    nextest:run) printf 'nextest\n' >>"$AE_PHASE_DIR/events" ;;
esac
if [[ "$kind" == selector && "$AE_PHASE_TARGET" != selector ]]; then
    printf 'selector\n' >>"$AE_PHASE_DIR/events"
    cat >"$AE_PHASE_DIR/selector-input"
    cat "$AE_PHASE_DIR/selector-output"
    exit 0
fi
if [[ -n "$kind" ]]; then exec "$AE_PHASE_PAYLOAD" "$kind" "$@"; fi
exec "$AE_PHASE_ORIGINAL_CARGO" "$@"
"#;

const TOOL: &str = r#"#!/usr/bin/env bash
tool=$(basename "$0")
case "$tool:$1" in
    taplo:fmt) kind=taplo-fmt ;;
    taplo:lint) kind=taplo ;;
    shellcheck:--version) exec "$AE_PHASE_ORIGINAL_SHELLCHECK" "$@" ;;
    shellcheck:*) kind=shellcheck ;;
    shfmt:*) kind=shfmt ;;
    *) exit 0 ;;
esac
exec "$AE_PHASE_PAYLOAD" "$kind" "$@"
"#;

// A refused live release writes a permanent suspect note. The live and TERM
// arms therefore use separate claims. The TERM arm observes survivors before
// release or cleanup. The live arm checks liveness on both sides of release.
const OBSERVER: &str = r#"set -u
set -m
recipe=$1 action=$2
dir=$AE_PHASE_DIR
identity() { LC_ALL=C ps -o lstart= -p "$1" 2>/dev/null | tr -cd A-Za-z0-9; }
matched() { [[ -n "$2" && "$(identity "$1")" == "$2" ]]; }
record() {
    local stamp
    stamp=$(identity "$1")
    [[ -z "$stamp" ]] || printf '%s %s\n' "$1" "$stamp" >>"$dir/owned"
}
cleanup() {
    local pid stamp file
    for file in payload child; do
        if [[ -s "$dir/$file" ]]; then cat "$dir/$file" >>"$dir/owned"; fi
    done
    if [[ -f "$dir/owned" ]]; then
        while read -r pid stamp; do
            if matched "$pid" "$stamp"; then kill -KILL "$pid" 2>/dev/null || :; fi
        done <"$dir/owned"
    fi
    wait 2>/dev/null || :
}
trap cleanup EXIT
trap 'exit 143' TERM HUP
trap 'exit 130' INT
mkfifo "$dir/hold" || exit 80
just lane-lock claim gatephase-spec 0 >"$dir/claim.out" 2>&1 || exit 81
cp .local/locks/gate/owner "$dir/initial-owner" || exit 81
just "$recipe" >"$dir/run.out" 2>"$dir/run.err" &
top=$!
record "$top"
for ((tick=0; tick<900; tick++)); do
    [[ -s "$dir/ready" ]] && break
    kill -0 "$top" 2>/dev/null || break
    sleep 0.05
done
if [[ ! -s "$dir/ready" || ! -s "$dir/payload" ]]; then
    cat "$dir/run.out" "$dir/run.err"
    echo 'the requested phase did not start' >&2
    exit 82
fi
cat "$dir/payload" >>"$dir/owned"
if [[ -s "$dir/child" ]]; then cat "$dir/child" >>"$dir/owned"; fi
# The parent tree is for teardown. Assertions concern the long target and its
# compiler, rather than the explicitly permitted short-command residual.
LC_ALL=C ps -A -o pid= -o ppid= >"$dir/tree" || exit 83
awk -v top="$top" '
    { parent[$1]=$2 }
    END { for (p in parent) {
        q=p
        for (n=0; n<128 && q>1; n++) {
            if (q==top) {print p; break}
            q=parent[q]
        }
    }}' "$dir/tree" >"$dir/tree-pids"
while read -r pid; do record "$pid"; done <"$dir/tree-pids"
snapshot() {
    local name=$1
    LC_ALL=C ps -A -o pid= -o stat= -o lstart= >"$dir/$name.census" || exit 85
    awk '
        NR == FNR { stamp[$1]=$2; next }
        {
            identity=""
            for (i=3; i<=NF; i++) identity=identity $i
            gsub(/[^A-Za-z0-9]/, "", identity)
            if ($1 in stamp && stamp[$1] == identity && $2 !~ /^Z/) print $1, identity
        }' "$dir/targets" "$dir/$name.census" >"$dir/$name"
}
cp "$dir/payload" "$dir/targets"
if [[ -s "$dir/child" ]]; then cat "$dir/child" >>"$dir/targets"; fi
if [[ "$action" == live || "$action" == KILL ]]; then
    if [[ "$action" == KILL ]]; then
        read -r pid stamp <"$dir/owned"
        matched "$pid" "$stamp" || exit 86
        kill -KILL "$pid" || exit 86
        wait "$top" 2>/dev/null || :
    fi
    snapshot before-release
    release_status=0
    just lane-lock release gatephase-spec >"$dir/release.out" 2>&1 || release_status=$?
    printf '%s\n' "$release_status" >"$dir/release-status"
    snapshot after-release
    cp .local/locks/gate/owner "$dir/owner-after" 2>/dev/null || :
    cp .local/locks/gate/suspect "$dir/suspect" 2>/dev/null || :
    printf 'finish\n' >"$dir/hold"
    if [[ "$action" == live ]]; then
        status=0
        wait "$top" || status=$?
        printf '%s\n' "$status" >"$dir/gate-status"
    fi
else
    if [[ "$action" == INT ]]; then
        read -r pid stamp <"$dir/owned"
        matched "$pid" "$stamp" || exit 86
        group=$(ps -o pgid= -p "$top" | tr -d ' ')
        [[ "$group" == "$top" ]] || exit 86
        matched "$pid" "$stamp" || exit 86
        kill -INT -- "-$top" || exit 86
    else
        read -r pid stamp <"$dir/owned"
        matched "$pid" "$stamp" || exit 86
        kill -TERM "$pid" || exit 86
    fi
    status=0
    wait "$top" || status=$?
    printf '%s\n' "$status" >"$dir/gate-status"
    snapshot at-gate-exit
    release_status=0
    just lane-lock release gatephase-spec >"$dir/release.out" 2>&1 || release_status=$?
    printf '%s\n' "$release_status" >"$dir/release-status"
    if [[ -e .local/locks/gate ]]; then echo held; else echo free; fi >"$dir/lock-after"
fi
cat "$dir/run.out" "$dir/run.err" "$dir/release.out"
"#;

struct PhaseFixture {
    old: ReceiptFixture,
    dir: PathBuf,
}

impl PhaseFixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let old = ReceiptFixture::new(&format!("ph{id:x}"), false);
        let dir = old.base.join("phase");
        std::fs::create_dir(&dir).expect("phase evidence directory");
        for (name, saved) in [
            ("cargo", "cargo-original"),
            ("shellcheck", "shellcheck-original"),
        ] {
            std::fs::rename(old.bin().join(name), old.bin().join(saved))
                .expect("save external fixture");
        }
        spec_install(&old.bin().join("cargo"), CARGO);
        spec_install(&old.bin().join("payload"), PAYLOAD);
        for name in ["taplo", "shellcheck", "shfmt"] {
            spec_install(&old.bin().join(name), TOOL);
        }
        spec_put(
            &dir.join("selector-output"),
            "version 1\nverdict subset\nowner doors\nowner floor\nowner gate\n\
             doctests yes\ncheck yes\ncounter 0 3\n\
             filterset kind(lib) | kind(bin) | (binary_id(ae::it) & test(/^(doors|floor|gate)::/))\nend\n",
        );
        Self { old, dir }
    }

    fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("AE_PHASE_DIR", self.dir.to_string_lossy().into_owned()),
            (
                "AE_PHASE_PAYLOAD",
                self.old
                    .bin()
                    .join("payload")
                    .to_string_lossy()
                    .into_owned(),
            ),
            (
                "AE_PHASE_ORIGINAL_CARGO",
                self.old
                    .bin()
                    .join("cargo-original")
                    .to_string_lossy()
                    .into_owned(),
            ),
            (
                "AE_PHASE_ORIGINAL_SHELLCHECK",
                self.old
                    .bin()
                    .join("shellcheck-original")
                    .to_string_lossy()
                    .into_owned(),
            ),
            (
                "AE_SPEC_REC",
                self.old.base.join("rec").to_string_lossy().into_owned(),
            ),
            ("AE_TEST_JOBS", "2".into()),
            ("AE_TEST_THREADS", "2".into()),
            ("AE_GATE_REAP_TERM_SECS", "1".into()),
            ("AE_GATE_REAP_KILL_SECS", "1".into()),
        ]
    }

    fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.join(name)).unwrap_or_default()
    }
}

fn executable(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(name))
        .find(|file| file.is_file())
        .unwrap_or_else(|| panic!("acceptance needs {name}"))
}

fn process_case(recipe: &str, phase: &str, action: &str) {
    let f = PhaseFixture::new();
    let timeout = executable(if cfg!(target_os = "macos") {
        "gtimeout"
    } else {
        "timeout"
    });
    let mut invocation = Invocation::new("nice")
        .arg("-n")
        .arg("10")
        .arg(timeout)
        .arg("-k")
        .arg("5")
        .arg("90")
        .arg("bash")
        .arg("-c")
        .arg(OBSERVER)
        .arg("phase-observer")
        .arg(recipe)
        .arg(action)
        .env_cleared()
        .env(
            "PATH",
            format!(
                "{}:{}",
                f.old.bin().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("HOME", &*f.old.base)
        .env("TMPDIR", &*f.old.base)
        .env("AE_TEST_TMPDIR", &*f.old.base)
        .env("AE_LANE_MIN_FREE_GIB", "0")
        .env("GIT_CONFIG_GLOBAL", f.old.base.join("gitconfig"))
        .env("AE_PHASE_TARGET", phase)
        .env("AE_PHASE_MODE", "hold");
    for (key, value) in f.env() {
        invocation = invocation.env(key, value);
    }
    let (out, err) = (f.dir.join("observer.out"), f.dir.join("observer.err"));
    let status = raw::run(&invocation, &f.old.tree, &out, &err).expect("bounded process observer");
    let evidence = format!(
        "{recipe}/{phase}/{action}:\n{}\n{}\n{}\n{}",
        read(&out),
        read(&err),
        f.file("run.out"),
        f.file("run.err")
    );
    assert!(
        matches!(status.outcome(), ExitOutcome::Code(0)),
        "setup or timeout: {evidence}"
    );
    assert!(
        f.file("events").lines().any(|line| line == phase),
        "the requested phase really ran: {evidence}"
    );
    if matches!(action, "live" | "KILL") {
        assert_eq!(
            f.file("release-status").trim(),
            "1",
            "release must refuse live owned work: {evidence}"
        );
        assert_eq!(
            f.file("before-release").lines().count(),
            2,
            "Cargo and its compiler are alive before release: {evidence}"
        );
        assert_eq!(
            f.file("after-release").lines().count(),
            2,
            "release must only observe them: {evidence}"
        );
        assert_eq!(
            f.file("owner-after"),
            f.file("initial-owner"),
            "the same claim stays held: {evidence}"
        );
        assert!(
            f.file("suspect").contains(": still running: "),
            "durable suspect note: {evidence}"
        );
        if action == "live" {
            assert_eq!(
                f.file("gate-status").trim(),
                "0",
                "cooperative completion: {evidence}"
            );
        }
    } else {
        assert_ne!(
            f.file("gate-status").trim(),
            "0",
            "interrupt must remain red: {evidence}"
        );
        assert_eq!(
            f.file("at-gate-exit"),
            "",
            "no target or compiler survives gate exit, before release/cleanup: {evidence}"
        );
        assert_eq!(
            f.file("release-status").trim(),
            "0",
            "completed killable work permits release: {evidence}"
        );
        assert_eq!(
            f.file("lock-after").trim(),
            "free",
            "claim removed only after completion: {evidence}"
        );
        let events = f.file("events");
        assert_eq!(
            events.lines().last(),
            Some(phase),
            "no later long step starts after interruption: {evidence}"
        );
    }
}

#[test]
fn test_fmt_term_reaps_cargo_and_formatter_before_gate_exit() {
    process_case("test", "fmt", "TERM");
}

#[test]
fn test_lint_term_reaps_cargo_and_compiler_before_gate_exit() {
    process_case("test", "lint", "TERM");
}

#[test]
fn rust_check_lint_term_reaps_cargo_and_compiler_before_gate_exit() {
    process_case("rust-check", "lint", "TERM");
}

#[test]
fn affected_lint_term_reaps_cargo_and_compiler_before_gate_exit() {
    process_case("test-affected", "lint", "TERM");
}

#[test]
fn pre_lint_term_reaps_cargo_and_compiler_before_gate_exit() {
    process_case("pre", "lint", "TERM");
}

#[test]
fn test_fuzz_term_reaps_cargo_and_compiler_before_gate_exit() {
    process_case("test", "fuzz", "TERM");
}

#[test]
fn rust_check_fuzz_term_reaps_cargo_and_compiler_before_gate_exit() {
    process_case("rust-check", "fuzz", "TERM");
}

#[test]
fn affected_fuzz_term_reaps_cargo_and_compiler_before_gate_exit() {
    process_case("test-affected", "fuzz", "TERM");
}

#[test]
fn affected_selector_term_reaps_cargo_and_compiler_before_gate_exit() {
    process_case("test-affected", "selector", "TERM");
}

#[test]
fn affected_doctest_term_reaps_cargo_and_compiler_before_gate_exit() {
    process_case("test-affected", "doctest", "TERM");
}

#[test]
fn affected_taplo_term_ends_the_tool_before_gate_exit() {
    process_case("test-affected", "taplo", "TERM");
}

#[test]
fn affected_shellcheck_term_ends_the_tool_before_gate_exit() {
    process_case("test-affected", "shellcheck", "TERM");
}

#[test]
fn affected_shfmt_term_ends_the_tool_before_gate_exit() {
    process_case("test-affected", "shfmt", "TERM");
}

#[test]
fn a_live_lint_run_keeps_its_claim_and_a_suspect_note_on_release() {
    process_case("rust-lint", "lint", "live");
}

#[test]
fn a_live_fuzz_run_keeps_its_claim_and_a_suspect_note_on_release() {
    process_case("rust-fuzz-check", "fuzz", "live");
}

#[test]
fn killing_the_top_cannot_free_its_live_lint_run() {
    process_case("rust-lint", "lint", "KILL");
}

#[test]
fn killing_the_top_cannot_free_its_live_fuzz_run() {
    process_case("rust-fuzz-check", "fuzz", "KILL");
}

#[test]
fn terminal_group_int_reaps_a_lint_compiler_outside_the_foreground_group() {
    process_case("test", "lint", "INT");
}

fn clean_case(recipe: &str, phase: &str, code: i32) {
    let f = PhaseFixture::new();
    spec_put(&f.dir.join("input"), "phase-stdin\n");
    let vars = f.env();
    let mut env: Vec<(&str, &str)> = vars
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    let exit = code.to_string();
    env.extend([
        ("AE_PHASE_TARGET", phase),
        ("AE_PHASE_MODE", "clean"),
        ("AE_PHASE_EXIT", exit.as_str()),
    ]);
    let mut invocation = Invocation::new("bash")
        .arg("-c")
        .arg("exec just --quiet \"$1\" <\"$AE_PHASE_DIR/input\"")
        .arg("clean-phase")
        .arg(recipe)
        .env_cleared()
        .env(
            "PATH",
            format!(
                "{}:{}",
                f.old.bin().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("HOME", &*f.old.base)
        .env("TMPDIR", &*f.old.base)
        .env("GIT_CONFIG_GLOBAL", f.old.base.join("gitconfig"));
    for (key, value) in env {
        invocation = invocation.env(key, value);
    }
    let (out, err) = (f.dir.join("clean.out"), f.dir.join("clean.err"));
    let status = raw::run(&invocation, &f.old.tree, &out, &err).expect("direct clean recipe");
    assert!(
        matches!(status.outcome(), ExitOutcome::Code(value) if value == code),
        "{recipe} status: {}",
        read(&err)
    );
    assert_eq!(
        read(&out),
        format!("cargo-out:{phase}:phase-stdin\n"),
        "stdout stays exact"
    );
    let sizing = format!("{recipe}: nextest threads 2 (override), cargo jobs 2 (override)\n");
    // Quiet mode excludes just's framing, as authorized by lead.
    assert_eq!(
        read(&err),
        format!("{sizing}cargo-err:{phase}\n"),
        "free direct stderr stays exact"
    );
    assert_eq!(
        f.file("input-seen"),
        "phase-stdin\n",
        "stdin survives supervision"
    );
    let expected = if phase == "lint" {
        vec![
            "clippy".to_owned(),
            "--locked".into(),
            "--all-targets".into(),
            "--all-features".into(),
            "--".into(),
            "-D".into(),
            "warnings".into(),
        ]
    } else {
        vec![
            "check".to_owned(),
            "--locked".into(),
            "--manifest-path".into(),
            "fuzz/Cargo.toml".into(),
            "--bins".into(),
            "--target-dir".into(),
            std::fs::canonicalize(&f.old.tree)
                .expect("physical fixture cwd")
                .join("target")
                .to_string_lossy()
                .into_owned(),
            "--jobs".into(),
            "2".into(),
        ]
    };
    assert_eq!(
        f.file("arguments").lines().collect::<Vec<_>>(),
        expected,
        "Cargo argv stays exact"
    );
    let events = f.file("events");
    assert_eq!(
        events.lines().collect::<Vec<_>>(),
        if phase == "lint" && code == 0 {
            vec!["lint", "taplo"]
        } else {
            vec![phase]
        },
        "a failed Cargo starts no later tool"
    );
}

#[test]
fn free_direct_lint_success_keeps_output_exit_arguments_and_stdin() {
    clean_case("rust-lint", "lint", 0);
}

#[test]
fn free_direct_lint_failure_keeps_output_exit_arguments_and_stdin() {
    clean_case("rust-lint", "lint", 7);
}

#[test]
fn free_direct_fuzz_success_keeps_output_exit_arguments_and_stdin() {
    clean_case("rust-fuzz-check", "fuzz", 0);
}

#[test]
fn free_direct_fuzz_failure_keeps_output_exit_arguments_and_stdin() {
    clean_case("rust-fuzz-check", "fuzz", 7);
}

fn unheld_case(recipe: &str, phase: &str) {
    let f = PhaseFixture::new();
    let vars = f.env();
    let mut env: Vec<(&str, &str)> = vars
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    env.extend([
        ("AE_PHASE_TARGET", phase),
        ("AE_PHASE_MODE", "clean"),
        ("AE_PHASE_EXIT", "0"),
        ("AE_LANE_CLAIM", "absent-claim"),
    ]);
    let (code, out, err) = f.old.just_in(&f.old.tree, recipe, &[], &env);
    assert_ne!(code, 0, "an explicit unheld claim must refuse: {out}{err}");
    assert!(
        err.contains("detached: refused:"),
        "the refusal names the supervisor: {out}{err}"
    );
    assert_eq!(f.file("events"), "", "no Cargo or later tool starts");
}

#[test]
fn an_explicit_unheld_claim_refuses_lint_before_cargo_starts() {
    unheld_case("rust-lint", "lint");
}

#[test]
fn an_explicit_unheld_claim_refuses_fuzz_before_cargo_starts() {
    unheld_case("rust-fuzz-check", "fuzz");
}

fn format_tool_stderr(stderr: &str) -> String {
    stderr
        .split_inclusive('\n')
        .filter(|line| {
            let text = line.trim_end_matches('\n');
            !matches!(
                text,
                "exec cargo fmt --all --check" | "exec taplo fmt --check"
            ) && !text.starts_with("error: recipe `rust-fmt-check` failed ")
                && !text.starts_with("error: Recipe `rust-fmt-check` failed ")
        })
        .collect()
}

fn format_case(misformatted: bool) {
    let f = PhaseFixture::new();
    let source = if misformatted {
        "fn phase_fmt(){}\n"
    } else {
        "fn phase_fmt() {}\n"
    };
    spec_put(&f.old.tree.join("src/lib.rs"), source);
    let vars = f.env();
    let mut env: Vec<(&str, &str)> = vars
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    env.extend([("AE_PHASE_TARGET", "fmt"), ("AE_PHASE_MODE", "clean")]);
    // just 1.57 silences linewise tool streams under --quiet. Run normally
    // and remove only the just framing whose change lead authorized.
    let (code, out, err) = f.old.just_in(&f.old.tree, "rust-fmt-check", &[], &env);
    assert_eq!(
        code,
        i32::from(misformatted),
        "the formatter exit stays exact: {out}{err}"
    );
    let expected_out = if misformatted {
        format!(
            "Diff in {}/src/lib.rs:1:\n-fn phase_fmt(){{}}\n+fn phase_fmt() {{}}\n",
            std::fs::canonicalize(&f.old.tree)
                .expect("physical formatter cwd")
                .display()
        )
    } else {
        "taplo-out\n".into()
    };
    assert_eq!(
        out, expected_out,
        "the formatter diff and taplo stdout stay exact"
    );
    assert_eq!(
        format_tool_stderr(&err),
        if misformatted {
            "formatter-note\n"
        } else {
            "formatter-note\ntaplo-err\n"
        },
        "both stderr streams stay exact: {err}"
    );
    assert_eq!(
        f.file("arguments"),
        "fmt\n--all\n--check\n",
        "Cargo fmt argv stays exact"
    );
    assert_eq!(
        f.file("events").lines().collect::<Vec<_>>(),
        if misformatted {
            vec!["fmt"]
        } else {
            vec!["fmt", "taplo-fmt"]
        },
        "taplo follows only a successful formatter"
    );
    assert_eq!(
        read(&f.old.tree.join("src/lib.rs")),
        source,
        "a format check never rewrites its input"
    );
}

#[test]
fn a_clean_format_check_keeps_tool_streams_and_exit_status() {
    format_case(false);
}

#[test]
fn a_misformatted_tree_keeps_the_diff_and_failure_status() {
    format_case(true);
}
