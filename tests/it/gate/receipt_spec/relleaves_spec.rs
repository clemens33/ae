//! Independent #345 acceptance. No complete release runs in these tests.
//! The nested step is extracted from the real release recipe without rewriting
//! its command. Real just, NEST, lane claims and ps decide process ordering.
//! Shellcheck is the external dependency and waits without spawning a child.

use super::*;

fn release_cargo_ok(source: &str) -> bool {
    for (header, required) in [
        (
            "release:",
            &[
                "cargo check --locked",
                "cargo metadata ",
                "cargo test --doc",
            ][..],
        ),
        ("bundles:", &["cargo build --release --locked"][..]),
        ("rust-deny:", &["cargo deny --locked check"][..]),
    ] {
        let lines = recipe_text(source, header);
        let included = |name: &str| lines.iter().any(|line| line == name);
        if !included("{{ NEST }}") || !included("{{ DETACHED }}") {
            return false;
        }
        if !required
            .iter()
            .all(|command| lines.iter().any(|line| line.contains(command)))
        {
            return false;
        }
        if lines
            .iter()
            .any(|line| super::super::invokes_cargo(line) && !super::super::cargo_detached(line))
        {
            return false;
        }
    }
    true
}

#[test]
fn each_release_and_bundles_cargo_step_uses_the_shared_supervisor() {
    assert!(
        release_cargo_ok(&read(&root().join("justfile"))),
        "all release and bundles cargo steps use DETACHED, including doctests"
    );
}

#[test]
fn the_release_cargo_guard_rejects_each_unprotected_step() {
    let good = "release:\n    {{ NEST }}\n    {{ DETACHED }}\n    detached cargo check --locked\n    detached cargo metadata --manifest-path fuzz/Cargo.toml\n    detached env TMPDIR=x nice -n 10 cargo test --doc --locked\n\nbundles:\n    {{ NEST }}\n    {{ DETACHED }}\n    detached cargo build --release --locked\n\nrust-deny:\n    {{ NEST }}\n    {{ DETACHED }}\n    detached cargo deny --locked check\n";
    assert!(release_cargo_ok(good), "positive control");
    for from in [
        "detached cargo check",
        "detached cargo metadata",
        "detached env TMPDIR=x nice -n 10 cargo test",
        "detached cargo build",
        "detached cargo deny",
    ] {
        assert_eq!(good.matches(from).count(), 1, "control must apply");
        for to in [
            from.trim_start_matches("detached ").to_owned(),
            format!("nest {}", from.trim_start_matches("detached ")),
        ] {
            assert!(
                !release_cargo_ok(&good.replacen(from, &to, 1)),
                "must reject unsupervised {to}"
            );
        }
    }
    for from in ["    {{ NEST }}\n", "    {{ DETACHED }}\n"] {
        assert!(!release_cargo_ok(&good.replacen(from, "", 1)));
    }
    assert!(!release_cargo_ok(&good.replace(
        "    detached cargo check --locked\n",
        "    detached true; cargo check --locked\n"
    )));
    assert!(
        !release_cargo_ok(&format!("{good}    cargo check --locked\n")),
        "new unsupervised cargo commands cannot evade the inventory"
    );
}

fn unquoted_just_calls(line: &str) -> Vec<usize> {
    let mut quote = None;
    let mut escaped = false;
    let mut calls = Vec::new();
    for (at, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
        } else if line[at..].starts_with("just ")
            && line[..at]
                .chars()
                .next_back()
                .is_none_or(|before| " \t;&|(".contains(before))
        {
            calls.push(at);
        }
    }
    calls
}

fn release_nested_ok(source: &str) -> bool {
    let nest = super::super::justfile_variable(source, "NEST");
    if source.matches("nest_capture() {").count() != 1 || !nest.contains("nest_capture() {") {
        return false;
    }
    for (header, required) in [
        (
            "release:",
            &[
                "nest just check",
                "nest just release-proof ",
                "nest just rust-deny",
                "nest_capture VERSION just bump",
                "nest just rust-fmt-check",
                "nest just rust-lint",
                "nest just rust-fuzz-check",
                "nest just bundles",
                "nest just _cliff-run ",
                "nest_capture RELEASE_BODY just _cliff-run ",
            ][..],
        ),
        (
            "bundles:",
            &[
                "nest_capture version just version",
                "nest just bundle \"$version\" darwin-arm64 ",
                "nest just bundle \"$version\" linux-x86_64-musl ",
            ][..],
        ),
        (
            "_cliff-run +args:",
            &["nest_capture skip just _cliff-skip"][..],
        ),
    ] {
        let lines = recipe_text(source, header);
        if !lines.iter().any(|line| line == "{{ NEST }}")
            || !required.iter().all(|prefix| {
                lines.iter().any(|line| {
                    if prefix.ends_with(' ') {
                        line.starts_with(prefix)
                    } else {
                        line == prefix
                    }
                })
            })
        {
            return false;
        }
        for line in &lines {
            if line.contains("$(just ") || line.contains("$( just ") {
                return false;
            }
            for at in unquoted_just_calls(line) {
                let head = line[..at].rsplit([';', '&', '|']).next().unwrap_or("");
                let words: Vec<_> = head.split_whitespace().collect();
                if words != ["nest"] && !(words.len() == 2 && words[0] == "nest_capture") {
                    return false;
                }
            }
        }
    }
    let cliff = recipe_text(source, "_cliff-run +args:");
    let bundle = recipe_text(source, "bundle version platform binary:");
    let release = recipe_text(source, "release:");
    cliff
        .last()
        .is_some_and(|line| line == "exec git-cliff \"$@\"")
        && bundle.iter().any(|line| line == "{{ NEST }}")
        && bundle
            .iter()
            .any(|line| line.starts_with("nest env COPYFILE_DISABLE=1 tar "))
        && release
            .iter()
            .any(|line| line.starts_with("nest env ") && line.contains(" doctor "))
        && release.iter().any(|line| {
            line == "nest release_doctests" || line.starts_with("nest release_doctests || ")
        })
}

#[test]
fn every_nested_release_step_uses_nest_or_the_capture_helper() {
    assert!(
        release_nested_ok(&read(&root().join("justfile"))),
        "release nested steps, capture, doctests, tar and cliff retain their signal chain"
    );
}

#[test]
fn the_nested_release_guard_rejects_each_lost_signal_chain() {
    let good = "NEST := '''\n    nest_capture() { true; }\n'''\n\nrelease:\n    {{ NEST }}\n    nest just check\n    nest just release-proof expect=x\n    nest just rust-deny\n    nest_capture VERSION just bump\n    nest just rust-fmt-check\n    nest just rust-lint\n    nest just rust-fuzz-check\n    nest just bundles\n    nest just _cliff-run --tag x -o CHANGELOG.md\n    nest_capture RELEASE_BODY just _cliff-run --strip header\n    nest release_doctests\n    nest env TMPDIR=x timeout 10 binary doctor >out\n\nbundles:\n    {{ NEST }}\n    nest_capture version just version\n    nest just bundle \"$version\" darwin-arm64 \"$native_bin\"\n    nest just bundle \"$version\" linux-x86_64-musl \"$musl_bin\"\n\nbundle version platform binary:\n    {{ NEST }}\n    nest env COPYFILE_DISABLE=1 tar -czf archive root\n\n_cliff-run +args:\n    {{ NEST }}\n    nest_capture skip just _cliff-skip\n    exec git-cliff \"$@\"\n";
    assert!(release_nested_ok(good), "positive control");
    assert!(release_nested_ok(&good.replace(
        "    nest just check\n",
        "    nest just check\n    [ -f \"$asset\" ] || { echo \"Error: just bundles did not produce it\"; exit 1; }\n"
    )), "diagnostic text is not a nested invocation");
    for line in good.lines().filter(|line| {
        line.trim_start().starts_with("nest ")
            || line.trim_start().starts_with("nest_capture ")
            || line.trim_start().starts_with("exec git-cliff ")
    }) {
        assert_eq!(good.matches(line).count(), 1, "mutation must apply once");
        let broken = good.replacen(line, "    true", 1);
        assert!(!release_nested_ok(&broken), "must reject missing {line}");
    }
    let added = good.replace(
        "    nest just check\n",
        "    nest just check\n    just rust-lint\n",
    );
    assert!(
        !release_nested_ok(&added),
        "extra bare just cannot evade inventory"
    );
    assert!(!release_nested_ok(&good.replace(
        "    nest_capture VERSION just bump\n",
        "    VERSION=$(nest just bump)\n"
    )));
    assert!(
        !release_nested_ok(&good.replace(
            "    nest just release-proof expect=x\n",
            "    nest just release-proof expect=x; just rust-lint\n"
        )),
        "the second command on a protected line needs its own nest"
    );
    assert!(!release_nested_ok(
        &good.replace("nest_capture() {", "missing_capture() {")
    ));
}

#[test]
fn capture_preserves_data_status_and_cleans_its_temp_on_failure() {
    for code in [0, 7] {
        let fixture = ReceiptFixture::new(&format!("relcap{code}"), false);
        let source = read(&fixture.tree.join("justfile"));
        let script = r#"
relleaves-capture:
    #!/usr/bin/env bash
    set -euo pipefail
    {{ NEST }}
    before=$(ls -A "$TMPDIR")
    expected=$("$AE_CAPTURE_TOOL" 0)
    status=0
    nest_capture actual "$AE_CAPTURE_TOOL" "$AE_CAPTURE_EXIT" || status=$?
    [[ "$actual" == "$expected" ]] || exit 81
    [[ "$status" == "$AE_CAPTURE_EXIT" ]] || exit 82
    [[ "$(ls -A "$TMPDIR")" == "$before" ]] || exit 83
    printf '%s' "$actual" >"$AE_CAPTURE_RESULT"
"#;
        spec_put(
            &fixture.tree.join("justfile"),
            &format!("{source}\n{script}"),
        );
        let tool = fixture.base.join("capture-tool");
        spec_install(
            &tool,
            "#!/bin/sh\nprintf '%s\\n' 'literal %s %% \\n and \\backslash' 'second line' ''; exit \"$1\"\n",
        );
        let tmp = fixture.base.join("capture-tmp");
        std::fs::create_dir(&tmp).expect("capture-only TMPDIR");
        let result = fixture.base.join("capture-result");
        let exit = code.to_string();
        let env = [
            ("AE_CAPTURE_TOOL", tool.to_str().expect("tool path")),
            ("AE_CAPTURE_RESULT", result.to_str().expect("result path")),
            ("AE_CAPTURE_EXIT", exit.as_str()),
            ("TMPDIR", tmp.to_str().expect("temp path")),
        ];
        let (status, out, err) = fixture.just_in(&fixture.tree, "relleaves-capture", &[], &env);
        assert_eq!(status, 0, "capture {code}: {out}{err}");
        assert_eq!(
            read(&result),
            "literal %s %% \\n and \\backslash\nsecond line",
            "captured text is data, and command substitution strips trailing newlines"
        );
    }
}

const HELD_TOOL: &str = r#"#!/usr/bin/env bash
if [[ "${1-}" == --version ]]; then
    printf '%s\n' 'ShellCheck - shell script analysis tool' 'version: 0.11.0'
    exit 0
fi
dir=$AE_RELLEAVES_DIR
identity() { LC_ALL=C ps -o lstart= -p "$1" | tr -cd A-Za-z0-9; }
trap 'printf "ended\n" >>"$dir/events"; exit 143' TERM
trap 'printf "ended\n" >>"$dir/events"; exit 130' INT
exec 9<>"$dir/hold"
printf '%s %s\n' "$$" "$(identity "$$")" >"$dir/tool.tmp"
mv "$dir/tool.tmp" "$dir/tool"
printf 'started\n' >>"$dir/events"
printf 'ready\n' >"$dir/ready"
IFS= read -r finish <&9
printf 'ended\n' >>"$dir/events"
"#;

// The ancestry census is evidence, never permission to kill. Permission needs
// both the recorded start identity and the per-run marker from ps of that pid
// alone. The list is capped before any kill, and each pid is rechecked again
// immediately before its kill. Duplicate records make the cap stricter.
const SAFE_CLEANUP: &str = r#"
identity() { LC_ALL=C ps -o lstart= -p "$1" 2>/dev/null | tr -cd A-Za-z0-9; }
matched() { [[ -n "$2" && "$(identity "$1")" == "$2" ]]; }
marked() {
    local description
    description=$(LC_ALL=C ps eww -p "$1" -o command= 2>/dev/null) || return 1
    case " $description " in
        *" $2=$dir "*) return 0 ;;
        *) return 1 ;;
    esac
}
qualified() {
    local state
    [[ "$1" =~ ^[1-9][0-9]*$ ]] || return 1
    matched "$1" "$2" || return 1
    state=$(LC_ALL=C ps -p "$1" -o stat= 2>/dev/null) || {
        matched "$1" "$2" || return 1
        return 2
    }
    case "${state// /}" in Z*) return 1 ;; esac
    marked "$1" AE_RELLEAVES_MARKER || {
        matched "$1" "$2" || return 1
        return 2
    }
}
record() {
    local stamp
    stamp=$(identity "$1")
    [[ -z "$stamp" ]] || printf '%s %s\n' "$1" "$stamp" >>"$dir/owned"
}
cleanup_owned() {
    local pid stamp status count=0 refused=0
    : >"$dir/qualified"
    if [[ -f "$dir/owned" ]]; then
        while read -r pid stamp; do
            status=0
            qualified "$pid" "$stamp" || status=$?
            case "$status" in
                0)
                    printf '%s %s\n' "$pid" "$stamp" >>"$dir/qualified"
                    count=$((count + 1)) ;;
                1) ;;
                *)
                    echo "cleanup refused: live recorded pid lacks readable run marker: $pid $stamp" >&2
                    refused=1 ;;
            esac
        done <"$dir/owned"
    fi
    if ((count > 32)); then
        echo 'cleanup refused: more than 32 records qualify; killed nothing' >&2
        cat "$dir/qualified" >&2
        return 91
    fi
    ((refused == 0)) || return 92
    while read -r pid stamp; do
        status=0
        qualified "$pid" "$stamp" || status=$?
        case "$status" in
            0) kill -KILL "$pid" 2>/dev/null || : ;;
            1) ;;
            *)
                echo "cleanup refused: run marker became unreadable: $pid $stamp" >&2
                return 92 ;;
        esac
    done <"$dir/qualified"
}
"#;

const OBSERVER: &str = r#"set -u
set -m
dir=$AE_RELLEAVES_DIR
cleanup() {
    local status pid stamp pending tick
    if [[ -s "$dir/tool" ]]; then cat "$dir/tool" >>"$dir/owned"; fi
    # Release the held tool without signalling. Platform binaries can hide their
    # environment on macOS, so let their whole chain end before marker checks.
    # The deciding survivor snapshot above remains untouched by this cleanup.
    if [[ -p "$dir/hold" ]]; then
        exec 9<>"$dir/hold"
        printf 'finish\n' >&9
        exec 9>&-
    fi
    for ((tick=0; tick<40; tick++)); do
        pending=0
        if [[ -f "$dir/owned" ]]; then
            while read -r pid stamp; do
                status=0
                qualified "$pid" "$stamp" || status=$?
                if ((status != 1)); then pending=1; break; fi
            done <"$dir/owned"
        fi
        ((pending)) || break
        sleep 0.05
    done
    status=0
    cleanup_owned || status=$?
    if ((status)); then
        trap - EXIT
        exit "$status"
    fi
    wait 2>/dev/null || :
}
trap cleanup EXIT
trap 'exit 143' TERM HUP
trap 'exit 130' INT
mkfifo "$dir/hold" || exit 80
just lane-lock claim relleaves-spec 0 >"$dir/claim.out" 2>&1 || exit 81
cp .local/locks/gate/owner "$dir/initial-owner" || exit 81
just relleaves-step >"$dir/run.out" 2>"$dir/run.err" &
top=$!
record "$top"
for ((tick=0; tick<600; tick++)); do
    [[ -s "$dir/ready" ]] && break
    kill -0 "$top" 2>/dev/null || break
    sleep 0.05
done
if [[ ! -s "$dir/ready" || ! -s "$dir/tool" ]]; then
    cat "$dir/run.out" "$dir/run.err"
    echo 'the extracted release step did not reach shellcheck' >&2
    exit 82
fi
cat "$dir/tool" >>"$dir/owned"
LC_ALL=C ps -A -o pid= -o ppid= >"$dir/tree" || exit 83
awk -v top="$top" '
    { parent[$1]=$2 }
    END { for (p in parent) {
        q=p
        for (n=0; n<128 && q>1; n++) {
            if (q==top) { print p; break }
            q=parent[q]
        }
    }}' "$dir/tree" >"$dir/tree-pids"
while read -r pid; do record "$pid"; done <"$dir/tree-pids"
snapshot() {
    LC_ALL=C ps -A -o pid= -o stat= -o lstart= >"$dir/census" || exit 84
    awk '
        NR==FNR { stamp[$1]=$2; next }
        {
            identity=""
            for (i=3; i<=NF; i++) identity=identity $i
            gsub(/[^A-Za-z0-9]/, "", identity)
            if ($1 in stamp && stamp[$1]==identity && $2 !~ /^Z/) print $1, identity
        }' "$dir/owned" "$dir/census" >"$dir/survivors"
}
read -r pid stamp <"$dir/owned"
matched "$pid" "$stamp" || exit 85
marked "$pid" AE_RELLEAVES_MARKER || exit 85
kill -TERM "$pid" || exit 85
status=0
wait "$top" || status=$?
printf '%s\n' "$status" >"$dir/step-status"
printf 'step-exit\n' >>"$dir/events"
# Observe before release, then before any cleanup can remove the evidence.
snapshot
release_status=0
just lane-lock release relleaves-spec >"$dir/release.out" 2>&1 || release_status=$?
printf '%s\n' "$release_status" >"$dir/release-status"
if [[ ! -e .local/locks/gate ]]; then printf 'released\n' >>"$dir/events"; fi
cat "$dir/run.out" "$dir/run.err" "$dir/release.out"
"#;

const SAFETY_CONTROL: &str = r#"set -u
dir=$AE_RELLEAVES_DIR
mode=$AE_RELLEAVES_SAFETY_MODE
control_pid= control_stamp=
finish_control() {
    if [[ -n "$control_pid" && -n "$control_stamp" ]] &&
       matched "$control_pid" "$control_stamp" &&
       marked "$control_pid" AE_RELLEAVES_CONTROL; then
        kill -KILL "$control_pid" 2>/dev/null || :
    fi
    [[ -z "$control_pid" ]] || wait "$control_pid" 2>/dev/null || :
}
trap finish_control EXIT
trap 'exit 143' TERM HUP
trap 'exit 130' INT
control_marker=foreign
[[ "$mode" != cap ]] || control_marker=$dir
# A missing marker never triggers a fallback kill; the sentinel ends itself
# inside the outer 30-second bound even when direct teardown must refuse.
AE_RELLEAVES_MARKER="$control_marker" AE_RELLEAVES_CONTROL="$dir" "$AE_RELLEAVES_SENTINEL" 10 &
control_pid=$!
control_stamp=$(identity "$control_pid")
for ((tick=0; tick<200; tick++)); do
    marked "$control_pid" AE_RELLEAVES_CONTROL && break
    sleep 0.01
done
marked "$control_pid" AE_RELLEAVES_CONTROL || exit 81
[[ -n "$control_stamp" ]] || exit 81
matched "$control_pid" "$control_stamp" || exit 81
: >"$dir/owned"
count=1
[[ "$mode" != cap ]] || count=33
for ((n=0; n<count; n++)); do
    printf '%s %s\n' "$control_pid" "$control_stamp" >>"$dir/owned"
done
status=0
cleanup_owned || status=$?
matched "$control_pid" "$control_stamp" || exit 82
marked "$control_pid" AE_RELLEAVES_CONTROL || exit 82
if [[ "$mode" == cap ]]; then
    [[ "$status" == 91 ]] || exit 83
    [[ "$(wc -l <"$dir/qualified" | tr -d ' ')" == 33 ]] || exit 83
else
    [[ "$status" == 92 && ! -s "$dir/qualified" ]] || exit 84
fi
printf 'control alive after cleanup\n'
"#;

fn safety_control_case(mode: &str) {
    let fixture = ReceiptFixture::new(&format!("rel-safety-{mode}"), false);
    let dir = fixture.base.join("safety");
    std::fs::create_dir(&dir).expect("cleanup safety evidence directory");
    let timeout = real_executable(if cfg!(target_os = "macos") {
        "gtimeout"
    } else {
        "timeout"
    });
    // macOS SIP platform binaries hide their environment from ps. Homebrew
    // coreutils gsleep is a non-platform sentinel with a readable marker.
    let sentinel = real_executable(if cfg!(target_os = "macos") {
        "gsleep"
    } else {
        "sleep"
    });
    let invocation = Invocation::new("nice")
        .arg("-n")
        .arg("10")
        .arg(timeout)
        .arg("-k")
        .arg("5")
        .arg("30")
        .arg("bash")
        .arg("-c")
        .arg(format!("{SAFE_CLEANUP}\n{SAFETY_CONTROL}"))
        .env_cleared()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("AE_RELLEAVES_DIR", &dir)
        .env("AE_RELLEAVES_MARKER", &dir)
        .env("AE_RELLEAVES_SENTINEL", sentinel)
        .env("AE_RELLEAVES_SAFETY_MODE", mode);
    let (out, err) = (dir.join("control.out"), dir.join("control.err"));
    let status =
        raw::run(&invocation, &fixture.tree, &out, &err).expect("bounded cleanup safety control");
    assert!(
        matches!(status.outcome(), ExitOutcome::Code(0)),
        "{mode}: {}\n{}\n{}",
        read(&out),
        read(&err),
        run_evidence(status.outcome(), &dir, &["owned", "qualified"])
    );
    assert_eq!(read(&out), "control alive after cleanup\n");
    if mode == "cap" {
        assert!(read(&err).contains("more than 32 records qualify; killed nothing"));
    } else {
        assert!(read(&err).contains("live recorded pid lacks readable run marker"));
    }
}

/// How a bounded bash run ended, and the named evidence files it left in
/// `dir`, so a failure that prints nothing still names its exit and its step.
/// Only regular files are read: the observer's hold FIFO would block a read.
fn run_evidence(outcome: ExitOutcome, dir: &Path, names: &[&str]) -> String {
    let mut text = match outcome {
        ExitOutcome::Code(code) => format!("ended: exit {code}\n"),
        ExitOutcome::Signalled => "ended: killed by a signal\n".to_owned(),
    };
    for name in names {
        let path = dir.join(name);
        let body = match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() => {
                std::fs::read_to_string(&path).unwrap_or_else(|why| format!("<unreadable: {why}>"))
            }
            Ok(_) => "<not a regular file>".to_owned(),
            Err(_) => "<absent>".to_owned(),
        };
        text.push_str("--- ");
        text.push_str(name);
        text.push('\n');
        text.push_str(&body);
        text.push('\n');
    }
    text
}

#[test]
fn cleanup_never_kills_an_unrelated_process_with_a_matching_recorded_start() {
    safety_control_case("unrelated");
}

#[test]
fn cleanup_cap_refuses_before_any_kill_even_when_all_records_match() {
    safety_control_case("cap");
}

fn real_executable(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("acceptance needs {name}"))
}

fn extracted_step(source: &str, kind: &str) -> String {
    if kind == "doctest" {
        let release = source.split_once("\nrelease:\n").expect("release recipe").1;
        let start = release
            .find("    doc_dir=\"$(mktemp -d)\"")
            .expect("doctest setup");
        let block = &release[start..];
        let end = block.find("    # Gate 4:").expect("end of doctest step");
        return block[..end].to_owned();
    }
    let steps = recipe_text(source, "release:");
    let candidates: Vec<_> = steps
        .iter()
        .filter(|line| {
            if kind == "check" {
                line.ends_with("just check")
            } else {
                line.contains("RELEASE_BODY") && line.contains("just _cliff-run ")
            }
        })
        .collect();
    assert_eq!(candidates.len(), 1, "exactly one requested release step");
    format!("    {}\n", candidates[0])
}

fn interrupted_step_case(kind: &str) {
    let fixture = ReceiptFixture::new(&format!("relleaves-{kind}"), false);
    let source = read(&fixture.tree.join("justfile"));
    let step = extracted_step(&source, kind);
    let justfile = format!(
        "{source}\nrelleaves-step:\n    #!/usr/bin/env bash\n    set -euo pipefail\n    {{{{ NEST }}}}\n    {{{{ DETACHED }}}}\n    TAG=vfixture\n{step}\n"
    );
    spec_put(&fixture.tree.join("justfile"), &justfile);
    let tool = match kind {
        "check" => "shellcheck",
        "capture" => "git-cliff",
        "doctest" => "cargo",
        _ => panic!("unknown extracted step"),
    };
    spec_install(&fixture.bin().join(tool), HELD_TOOL);
    let dir = fixture.base.join("relleaves");
    std::fs::create_dir(&dir).expect("release-step evidence directory");
    let timeout = real_executable(if cfg!(target_os = "macos") {
        "gtimeout"
    } else {
        "timeout"
    });
    let invocation = Invocation::new("nice")
        .arg("-n")
        .arg("10")
        .arg(timeout)
        .arg("-k")
        .arg("5")
        .arg("60")
        .arg("bash")
        .arg("-c")
        .arg(format!("{SAFE_CLEANUP}\n{OBSERVER}"))
        .env_cleared()
        .env(
            "PATH",
            format!(
                "{}:{}",
                fixture.bin().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("HOME", &*fixture.base)
        .env("TMPDIR", &*fixture.base)
        .env("GIT_CONFIG_GLOBAL", fixture.base.join("gitconfig"))
        .env("AE_TEST_JOBS", "2")
        .env("AE_TEST_THREADS", "2")
        .env("AE_GATE_REAP_TERM_SECS", "1")
        .env("AE_GATE_REAP_KILL_SECS", "1")
        .env("AE_RELLEAVES_MARKER", &dir)
        .env("AE_RELLEAVES_DIR", &dir);
    let (out, err) = (dir.join("observer.out"), dir.join("observer.err"));
    let status = raw::run(&invocation, &fixture.tree, &out, &err)
        .expect("bounded extracted-release observer");
    let evidence = format!("{}\n{}", read(&out), read(&err));
    assert!(
        matches!(status.outcome(), ExitOutcome::Code(0)),
        "observer setup or timeout: {evidence}\n{}",
        run_evidence(
            status.outcome(),
            &dir,
            &[
                "claim.out",
                "events",
                "ready",
                "tool",
                "owned",
                "tree-pids",
                "step-status",
                "survivors",
                "release.out",
                "release-status",
                "run.out",
                "run.err",
            ],
        )
    );
    assert_ne!(read(&dir.join("step-status")).trim(), "0", "{evidence}");
    assert_eq!(
        read(&dir.join("survivors")),
        "",
        "no observed process of the release step survives recipe exit: {evidence}"
    );
    assert_eq!(
        read(&dir.join("release-status")).trim(),
        "0",
        "ended work permits release: {evidence}"
    );
    assert_eq!(
        read(&dir.join("events")),
        "started\nended\nstep-exit\nreleased\n",
        "the tool ends before recipe exit and claim release: {evidence}"
    );
}

#[test]
fn term_to_an_extracted_nested_release_step_finishes_before_claim_release() {
    interrupted_step_case("check");
}

#[test]
fn term_during_captured_changelog_finishes_before_claim_release() {
    interrupted_step_case("capture");
}

#[test]
fn term_during_release_doctests_finishes_before_claim_release() {
    interrupted_step_case("doctest");
}
