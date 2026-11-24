//! Independent #340 acceptance. Real recipes, lock, signals and process census.
//! External cargo/SSH are fixtures. Only census-fault cases change ps output.
//! Frozen against plan-p0-r2.md 39017b7b094b1c152915478f490b5c7a91999958c07956f43bda966ca75aab5f.
//! Install beside `releaseproof_spec.rs` and add its path module in that file.

use super::*;

const CARGO: &str = r#"#!/usr/bin/env bash
if [[ "${1-}:${2-}" != nextest:run ]]; then
    exec "$AE_REAP_CARGO_ORIGINAL" "$@"
fi
record() {
    local identity
    identity=$(LC_ALL=C "$AE_REAP_REAL_PS" -o lstart= -p "$1" | tr -cd A-Za-z0-9)
    printf '%s %s\n' "$1" "$identity" >>"$AE_REAP_DIR/owned"
}
record "$$"
trap 'exit 143' TERM HUP
trap 'exit 130' INT
if [[ "$AE_REAP_MODE" == moved ]]; then set -m; fi
bash -c 'trap "" INT TERM HUP; echo ready >"$AE_REAP_DIR/child-ready"; exec sleep 300' &
child=$!
record "$child"
printf '%s\n' "$child" >"$AE_REAP_DIR/child"
while [[ ! -s "$AE_REAP_DIR/child-ready" ]]; do sleep 0.01; done
late() {
    trap '' INT TERM HUP
    bash -c 'trap "" INT TERM HUP; exec sleep 300' &
    local late_child=$!
    record "$late_child"
    printf '%s\n' "$late_child" >"$AE_REAP_DIR/late"
    wait "$late_child"
}
if [[ "$AE_REAP_MODE" == late || "$AE_REAP_MODE" == repeat ]]; then
    trap late INT TERM HUP
fi
printf '%s\n' "$$" >"$AE_REAP_DIR/cargo"
case "$AE_REAP_MODE" in
    normal0|normal7)
        while [[ ! -e "$AE_REAP_DIR/finish" ]]; do sleep 0.05; done
        [[ "$AE_REAP_MODE" == normal0 ]] && exit 0
        exit 7 ;;
esac
wait "$child"
"#;

const CENSUS: &str = r#"#!/usr/bin/env bash
case " $* " in
    *' -A '*) ;;
    *) exec "$AE_REAP_REAL_PS" "$@" ;;
esac
if [[ -e "$AE_REAP_DIR/fault" ]]; then
    case "$AE_REAP_MODE" in
        census-fail) echo called >>"$AE_REAP_DIR/fault-used"; exit 47 ;;
        census-bad) echo called >>"$AE_REAP_DIR/fault-used"; echo unreadable-census; exit 0 ;;
        census-sticky)
            echo called >>"$AE_REAP_DIR/fault-used"
            cat "$AE_REAP_DIR/census-saved"
            exit 0 ;;
    esac
fi
sample=$(LC_ALL=C "$AE_REAP_REAL_PS" "$@") || exit $?
if [[ -s "$AE_REAP_DIR/cargo" ]] &&
   awk -v p="$(cat "$AE_REAP_DIR/cargo")" '$1 == p { found=1 } END { exit !found }' <<<"$sample"; then
    printf '%s\n' "$sample" >"$AE_REAP_DIR/census-saved.tmp.$$"
    mv "$AE_REAP_DIR/census-saved.tmp.$$" "$AE_REAP_DIR/census-saved"
    if [[ -e "$AE_REAP_DIR/census-once" ]]; then touch "$AE_REAP_DIR/census-twice"; fi
    touch "$AE_REAP_DIR/census-once"
fi
printf '%s\n' "$sample"
"#;

// The observer is the external lock holder. It deliberately attempts the real
// release even after an interrupted gate, as the incident's callers do.
// It records liveness before its EXIT cleanup can hide any survivor.
const OBSERVER: &str = r#"set -u
set -m
recipe=$1 action=$2
dir=$AE_REAP_DIR
identity() { LC_ALL=C "$AE_REAP_REAL_PS" -o lstart= -p "$1" 2>/dev/null | tr -cd A-Za-z0-9; }
alive() {
    local table
    table=$(LC_ALL=C "$AE_REAP_REAL_PS" -A -o pid= -o stat= 2>/dev/null) || return 0
    awk -v pid="$1" '$1 == pid { state=$2 } END { exit !(state != "" && state !~ /^Z/) }' <<<"$table"
}
cleanup() {
    local pid stamp now
    if [[ -f "$dir/owned" ]]; then
        while read -r pid stamp; do
            [[ "$pid" =~ ^[1-9][0-9]*$ && -n "$stamp" ]] || continue
            now=$(identity "$pid")
            [[ "$now" != "$stamp" ]] || kill -KILL "$pid" 2>/dev/null || :
        done <"$dir/owned"
    fi
    wait 2>/dev/null || :
}
trap cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
just lane-lock claim reap-spec >"$dir/claim.log" 2>&1 || exit 80
just "$recipe" >"$dir/recipe.log" 2>&1 &
top=$!
printf '%s %s\n' "$top" "$(identity "$top")" >>"$dir/owned"
startup_started=$SECONDS
startup_deadline=$((startup_started + 75))
while ((SECONDS < startup_deadline)); do
    [[ -s "$dir/cargo" ]] && break
    sleep 0.05
done
printf 'startup waited: %s s\n' "$((SECONDS - startup_started))" >"$dir/startup.log"
if [[ ! -s "$dir/cargo" ]]; then
    # A process-table read never ends this wait early. Capture Bash's own job
    # state in the observer shell before its EXIT cleanup can kill the top.
    jobs -l >>"$dir/startup.log"
    jobs -rp >"$dir/startup-active"
    jobs -sp >>"$dir/startup-active"
    active=0
    while read -r pid; do [[ "$pid" != "$top" ]] || active=1; done <"$dir/startup-active"
    if ((active == 0)); then
        top_status=0
        wait "$top" || top_status=$?
        printf 'top wait status before cleanup: %s\n' "$top_status" >>"$dir/startup.log"
    fi
    LC_ALL=C "$AE_REAP_REAL_PS" -o pid= -o stat= -o lstart= -p "$top" >>"$dir/startup.log" 2>&1 || :
    cat "$dir/recipe.log"
    exit 81
fi
# Snapshot the real parent tree before signalling, including nested just/bash.
LC_ALL=C "$AE_REAP_REAL_PS" -A -o pid= -o ppid= >"$dir/tree"
awk -v top="$top" '
    { parent[$1]=$2 }
    END { for (p in parent) {
        q=p
        for (n=0; n<128 && q>1; n++) {
            if (q==top) {print p; break}
            q=parent[q]
        }
    }}' "$dir/tree" >"$dir/tree-pids"
while read -r pid; do
    stamp=$(identity "$pid")
    [[ -z "$stamp" ]] || printf '%s %s\n' "$pid" "$stamp" >>"$dir/owned"
done <"$dir/tree-pids"
if [[ "$AE_REAP_MODE" == moved ]]; then
    for ((tick=0; tick<200; tick++)); do
        [[ -e "$dir/census-twice" ]] && break
        alive "$top" || break
        sleep 0.05
    done
    cargo=$(cat "$dir/cargo"); child=$(cat "$dir/child")
    cg=$("$AE_REAP_REAL_PS" -o pgid= -p "$cargo" | tr -d ' ')
    dg=$("$AE_REAP_REAL_PS" -o pgid= -p "$child" | tr -d ' ')
    [[ -n "$cg" && -n "$dg" && "$cg" != "$dg" ]] || exit 83
fi
case "$AE_REAP_MODE" in
    census-*)
        LC_ALL=C "$AE_REAP_REAL_PS" -A -o pid= -o ppid= -o pgid= -o lstart= >"$dir/census-saved"
        touch "$dir/fault" ;;
esac
case "$action" in
    finish) touch "$dir/finish" ;;
    INT)
        group=$("$AE_REAP_REAL_PS" -o pgid= -p "$top" | tr -d ' ')
        [[ "$group" == "$top" ]] || exit 86
        kill -INT -- "-$top" || exit 85 ;;
    KILL-SUP)
        supervisor=$("$AE_REAP_REAL_PS" -o ppid= -p "$(cat "$dir/cargo")" | tr -d ' ')
        kill -KILL "$supervisor" || exit 85 ;;
    *)
        kill -"$action" "$top" || exit 85
        if [[ "$AE_REAP_MODE" == repeat ]]; then
            sleep 0.15
            kill -TERM "$top" 2>/dev/null || :
        fi ;;
esac
gate_status=0
wait "$top" || gate_status=$?
printf '%s\n' "$gate_status" >"$dir/gate-status"
release_status=0
just lane-lock release reap-spec >"$dir/release.log" 2>&1 || release_status=$?
printf '%s\n' "$release_status" >"$dir/release-status"
LC_ALL=C "$AE_REAP_REAL_PS" -A -o pid= -o stat= -o lstart= >"$dir/survivor-census" 2>"$dir/survivor-census.err" || {
    echo 'observer: the survivor census failed' >&2
    cat "$dir/survivor-census.err" >&2
    exit 87
}
awk '
    NR == FNR { stamp[$1]=$2; next }
    {
        identity=""
        for (field=3; field<=NF; field++) identity=identity $field
        gsub(/[^A-Za-z0-9]/, "", identity)
        if ($1 in stamp && stamp[$1] == identity && $2 !~ /^Z/) print $1, identity
    }' "$dir/owned" "$dir/survivor-census" >"$dir/survivors"
just lane-lock status >"$dir/lock-status" 2>&1 || :
cat "$dir/recipe.log" "$dir/release.log" "$dir/lock-status"
"#;

fn executable_tool(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("acceptance needs {name}"))
}

fn process_case(recipe: &str, action: &str, mode: &str, held: bool) {
    static NEXT_CASE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    let case = NEXT_CASE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let fixture = ProofFixture::new(&format!("gp{case:02x}"));
    let old = &fixture.old;
    let dir = old.base.join("reap");
    std::fs::create_dir(&dir).expect("process evidence directory");
    let original = old.bin().join("cargo-before-reap");
    std::fs::rename(old.bin().join("cargo"), &original).expect("keep cargo dependency");
    spec_install(&old.bin().join("cargo"), CARGO);
    spec_install(&old.bin().join("ps"), CENSUS);
    let timeout = executable_tool(if cfg!(target_os = "macos") {
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
        .arg("120")
        .arg("bash")
        .arg("-c")
        .arg(OBSERVER)
        .arg("reap-observer")
        .arg(recipe)
        .arg(action)
        .env_cleared()
        .env(
            "PATH",
            format!(
                "{}:{}",
                old.bin().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("HOME", &*old.base)
        .env("TMPDIR", &*old.base)
        .env("AE_TEST_TMPDIR", &*old.base)
        .env("AE_LANE_MIN_FREE_GIB", "0")
        .env("AE_LANE_MAX_TEMP_C", "0")
        .env("GIT_CONFIG_GLOBAL", old.base.join("gitconfig"))
        .env("AE_SPEC_REC", old.base.join("rec"))
        .env("AE_SPEC_CLIFF", old.base.join("cliff-argv"))
        .env("AE_SPEC_SYSROOT", old.base.join("sysroot"))
        .env("AE_REAP_DIR", &dir)
        .env("AE_REAP_REAL_PS", executable_tool("ps"))
        .env("AE_REAP_CARGO_ORIGINAL", original)
        .env("AE_REAP_MODE", mode)
        .env("AE_GATE_REAP_TERM_SECS", "1")
        .env("AE_GATE_REAP_KILL_SECS", "1");
    for (key, value) in &fixture.env {
        invocation = invocation.env(key, value);
    }
    let out = dir.join("observer.out");
    let err = dir.join("observer.err");
    let status =
        raw::run(&invocation, &old.tree, &out, &err).expect("bounded real recipe observer");
    let outcome = match status.outcome() {
        ExitOutcome::Code(code) => format!("exit {code}"),
        ExitOutcome::Signalled => "signal".to_owned(),
    };
    let evidence = format!(
        "observer: {outcome}\nclaim: {}\nstartup: {}\n{}\n{}",
        std::fs::read_to_string(dir.join("claim.log")).unwrap_or_default(),
        std::fs::read_to_string(dir.join("startup.log")).unwrap_or_default(),
        read(&out),
        read(&err)
    );
    assert!(
        matches!(status.outcome(), ExitOutcome::Code(0)),
        "observer setup or timeout: {evidence}"
    );
    let gate = read(&dir.join("gate-status"));
    let release = read(&dir.join("release-status"));
    let lock = read(&dir.join("lock-status"));
    if mode == "normal0" {
        assert_eq!(gate.trim(), "0", "green cargo status: {evidence}");
    } else {
        assert_ne!(
            gate.trim(),
            "0",
            "failed/interrupted gate must stay red: {evidence}"
        );
    }
    if mode == "normal7" {
        assert_eq!(
            gate.trim(),
            "7",
            "cargo's failure status survives cleanup: {evidence}"
        );
    }
    if held {
        assert_ne!(
            release.trim(),
            "0",
            "uncertain cleanup must refuse release: {evidence}"
        );
        assert!(
            old.tree.join(".local/locks/gate/owner").is_file(),
            "holder remains: {evidence}"
        );
        let note = read(&old.tree.join(".local/locks/gate/suspect"));
        assert!(
            note.contains("suspect:"),
            "durable suspect note: {evidence}"
        );
        assert!(
            lock.contains("reap-spec") && lock.contains("suspect"),
            "visible suspect holder: {evidence}"
        );
        if mode.starts_with("census-") {
            assert!(
                dir.join("fault-used").is_file(),
                "the census fault must apply: {evidence}"
            );
        }
        if mode == "census-sticky" {
            assert!(
                read(&dir.join("survivors")).is_empty(),
                "the real KILL path must run before sticky-census refusal: {evidence}"
            );
        }
    } else {
        assert_eq!(
            release.trim(),
            "0",
            "killable gate releases its lock: {evidence}"
        );
        assert!(
            lock.contains("gate: free"),
            "real status shows free: {evidence}"
        );
        assert!(
            read(&dir.join("survivors")).is_empty(),
            "release left a real gate process alive: {evidence}"
        );
        if mode == "moved" {
            assert!(
                dir.join("census-twice").is_file(),
                "the moved child was observed before signalling: {evidence}"
            );
        }
    }
    if action != "finish" {
        assert!(
            !old.recorded().contains(&"doctest".into()),
            "no later doctest starts after stopping: {evidence}"
        );
    }
    if matches!(mode, "late" | "repeat") {
        assert!(
            dir.join("late").is_file(),
            "the late-spawn arm must run: {evidence}"
        );
    }
}

#[test]
fn term_to_each_public_gate_reaps_before_the_holder_releases() {
    for recipe in ["test", "test-affected", "test-macos", "release-proof"] {
        process_case(recipe, "TERM", "sleep", false);
    }
}

#[test]
fn ctrl_c_to_the_foreground_group_reaps_resistant_children_before_release() {
    process_case("test", "INT", "sleep", false);
}

#[test]
fn a_term_handler_can_start_a_child_and_wait_without_outliving_the_gate() {
    process_case("test-macos", "TERM", "late", false);
}

#[test]
fn repeated_term_during_reaping_does_not_release_early() {
    process_case("test-macos", "TERM", "repeat", false);
}

#[test]
fn an_observed_child_in_another_group_is_still_reaped() {
    process_case("test-macos", "TERM", "moved", false);
}

#[test]
fn ordinary_cargo_status_survives_cleanup_of_its_stray_child() {
    for mode in ["normal0", "normal7"] {
        process_case("test", "finish", mode, false);
    }
}

#[test]
fn a_failed_or_unreadable_census_keeps_the_lock_with_a_suspect_note() {
    for mode in ["census-fail", "census-bad"] {
        process_case("test-macos", "TERM", mode, true);
    }
}

#[test]
fn a_census_member_still_reported_after_kill_keeps_the_lock() {
    process_case("test-macos", "TERM", "census-sticky", true);
}

#[test]
fn killing_the_supervisor_keeps_its_registry_and_the_lock() {
    process_case("test-macos", "KILL-SUP", "killed", true);
}

fn registry_fixture(tag: &str) -> ReceiptFixture {
    let fixture = ReceiptFixture::new(&format!("gr-{tag}"), false);
    let (code, out, err) =
        fixture.just_in(&fixture.tree, "lane-lock", &["claim", "reap-spec"], &[]);
    assert_eq!(code, 0, "scratch claim: {out}{err}");
    std::fs::create_dir_all(fixture.tree.join(".local/locks/groups")).expect("registry");
    fixture
}

fn entry(fixture: &ReceiptFixture, body: &str) {
    let run = body
        .lines()
        .nth(1)
        .expect("run line")
        .strip_prefix("run ")
        .expect("run fields");
    let (pid, identity) = run.split_once(' ').expect("run identity");
    spec_put(
        &fixture
            .tree
            .join(format!(".local/locks/groups/{pid}.{identity}")),
        body,
    );
}

#[test]
fn a_pending_entry_with_a_dead_supervisor_cannot_release() {
    let fixture = registry_fixture("dead-pending");
    let owner = read(&fixture.tree.join(".local/locks/gate/owner"));
    // 2147483647 exceeds pid_max on both supported hosts and cannot be live.
    entry(
        &fixture,
        &format!(
            "ae-gate-group 1\nrun 2147483647 OldIdentity\nclaim {}\n",
            owner.trim()
        ),
    );
    let (code, out, err) =
        fixture.just_in(&fixture.tree, "lane-lock", &["release", "reap-spec"], &[]);
    assert_ne!(
        code, 0,
        "a dead pending supervisor is uncertainty: {out}{err}"
    );
    assert!(fixture.tree.join(".local/locks/gate/owner").is_file());
    assert!(read(&fixture.tree.join(".local/locks/gate/suspect")).contains("suspect:"));
}

#[test]
fn a_reused_group_number_with_a_different_start_identity_does_not_hold_the_lock() {
    let fixture = registry_fixture("reused-group");
    let owner = read(&fixture.tree.join(".local/locks/gate/owner"));
    // The observed pid/pgid exists, but its start identity differs from the
    // recorded identity. It belongs to a later incarnation, not this gate.
    spec_install(
        &fixture.bin().join("ps"),
        r#"#!/bin/sh
case " $* " in
    *' -A '*) printf '%s\n' '2147483647 1 2147483647 Sun Nov 15 08:00:00 2026' ;;
    *) exec "$AE_REAP_REAL_PS" "$@" ;;
esac
"#,
    );
    entry(
        &fixture,
        &format!(
            "ae-gate-group 1\nrun 2147483646 OldSupervisor\nclaim {}\nleader 2147483647 OldIdentity\nmember 2147483647 OldIdentity\n",
            owner.trim()
        ),
    );
    let real_ps = executable_tool("ps");
    let env = [("AE_REAP_REAL_PS", real_ps.to_str().expect("real ps path"))];
    let (code, out, err) =
        fixture.just_in(&fixture.tree, "lane-lock", &["release", "reap-spec"], &env);
    assert_eq!(code, 0, "a reused group is not this run: {out}{err}");
    assert!(!fixture.tree.join(".local/locks/gate").exists());
}

const FOREIGN_CLAIM: &str = r#"set -eu
set -m
identity() { LC_ALL=C "$AE_REAP_REAL_PS" -o lstart= -p "$1" 2>/dev/null | tr -cd A-Za-z0-9; }
cleanup() {
    local pid stamp
    if [[ -s "$AE_REAP_DIR/foreign-owned" ]]; then
        read -r pid stamp <"$AE_REAP_DIR/foreign-owned"
        if [[ -n "$stamp" && "$(identity "$pid")" == "$stamp" ]]; then
            kill -KILL "$pid" 2>/dev/null || :
        fi
        wait "$pid" 2>/dev/null || :
    fi
}
trap cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
sleep 300 &
member=$!
stamp=$(identity "$member")
[[ -n "$stamp" ]] || exit 80
printf '%s %s\n' "$member" "$stamp" >"$AE_REAP_DIR/foreign-owned"
supervisor_stamp=$(identity "$$")
entry=".local/locks/groups/$$.$supervisor_stamp"
printf 'ae-gate-group 1\nrun %s %s\nclaim %s\nleader %s %s\nmember %s %s\n' \
    "$$" "$supervisor_stamp" "$1" "$member" "$stamp" "$member" "$stamp" >"$entry"
cp "$entry" "$AE_REAP_DIR/foreign-before"
status=0
just lane-lock release reap-spec >"$AE_REAP_DIR/foreign-release" 2>&1 || status=$?
cat "$AE_REAP_DIR/foreign-release"
[[ "$status" == 0 ]] || exit 81
[[ ! -e .local/locks/gate ]] || exit 82
[[ -f "$entry" ]] || exit 83
cmp "$entry" "$AE_REAP_DIR/foreign-before" || exit 84
kill -0 "$member" || exit 85
[[ "$(identity "$member")" == "$stamp" ]] || exit 86
"#;

#[test]
fn live_entries_from_free_and_other_claims_do_not_block_or_change_on_release() {
    for claim in ["free", "another-owner 2026-11-15T08:00:00Z"] {
        let fixture = registry_fixture("foreign-claim");
        let timeout = executable_tool(if cfg!(target_os = "macos") {
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
            .arg("90")
            .arg("bash")
            .arg("-c")
            .arg(FOREIGN_CLAIM)
            .arg("foreign-claim-observer")
            .arg(claim)
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
            .env("AE_REAP_DIR", &*fixture.base)
            .env("AE_REAP_REAL_PS", executable_tool("ps"));
        let out = fixture.base.join("foreign.out");
        let err = fixture.base.join("foreign.err");
        let status = raw::run(&invocation, &fixture.tree, &out, &err)
            .expect("bounded foreign-claim observer");
        assert!(
            matches!(status.outcome(), ExitOutcome::Code(0)),
            "a live {claim:?} entry stays intact and alive while this claim releases: {}\n{}",
            read(&out),
            read(&err)
        );
    }
}

#[test]
fn malformed_current_claim_entries_refuse_release_instead_of_disappearing() {
    for suffix in [
        "member broken\n",
        "leader 99 Identity",
        "unknown 1 Identity\n",
    ] {
        let fixture = registry_fixture("malformed");
        let owner = read(&fixture.tree.join(".local/locks/gate/owner"));
        entry(
            &fixture,
            &format!(
                "ae-gate-group 1\nrun 2147483647 OldIdentity\nclaim {}\n{suffix}",
                owner.trim()
            ),
        );
        let (code, out, err) =
            fixture.just_in(&fixture.tree, "lane-lock", &["release", "reap-spec"], &[]);
        assert_ne!(code, 0, "malformed entries fail closed: {out}{err}");
        assert!(fixture.tree.join(".local/locks/gate/owner").is_file());
        assert!(fixture.tree.join(".local/locks/gate/suspect").is_file());
    }
}

#[test]
fn a_registration_failure_prevents_cargo_from_starting() {
    let fixture = ProofFixture::new("gr-regfail");
    let old = &fixture.old;
    let groups = old.tree.join(".local/locks/groups");
    if groups.is_dir() {
        std::fs::remove_dir_all(&groups).expect("empty scratch registry");
    }
    spec_put(&groups, "a file cannot be a process registry\n");
    let original = old.bin().join("cargo-before-reap");
    std::fs::rename(old.bin().join("cargo"), &original).expect("keep cargo dependency");
    spec_install(
        &old.bin().join("cargo"),
        r#"#!/bin/sh
if [ "${1-}:${2-}" = nextest:run ]; then echo started >"$AE_REAP_STARTED"; fi
exec "$AE_REAP_CARGO_ORIGINAL" "$@"
"#,
    );
    let started = old.base.join("cargo-started");
    let extra = [
        ("AE_REAP_STARTED", started.to_str().expect("scratch path")),
        (
            "AE_REAP_CARGO_ORIGINAL",
            original.to_str().expect("cargo path"),
        ),
    ];
    let (code, out, err) = fixture.run("_tmux-isolated", &["test"], &extra);
    assert_ne!(
        code, 0,
        "registration failure must fail the gate: {out}{err}"
    );
    assert!(
        !started.exists(),
        "cargo cannot precede durable registration: {out}{err}"
    );
}
