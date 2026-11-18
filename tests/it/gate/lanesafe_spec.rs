//! Independent acceptance for the lane safety brief. These tests drive the real
//! justfile in private checkouts and substitute external facts or effects.
#![allow(
    clippy::expect_used,
    reason = "fixture failures must name the failed prerequisite"
)]

use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use super::{killed_scratch_child, lane_lock_checkouts, lane_lock_env, read, run_in_env};
use crate::cli::{bounded, helper_by_name};

fn script(bin: &Path, name: &str, body: &str) {
    fs::create_dir_all(bin).expect("a private shim directory");
    let file = bin.join(name);
    fs::write(&file, format!("#!/bin/sh\nset -eu\n{body}\n")).expect("a private shim");
    fs::set_permissions(file, fs::Permissions::from_mode(0o755)).expect("an executable shim");
}

fn shim_path(bin: &Path) -> String {
    format!("{}:{}", bin.display(), std::env::var("PATH").expect("PATH"))
}

fn fake_disk(bin: &Path) {
    script(
        bin,
        "cargo",
        r#"
[ "$1" = metadata ] || exit 78
printf '{"target_directory":"%s","build_directory":"%s"}\n' "$CARGO_TARGET_DIR" "$CARGO_TARGET_DIR"
"#,
    );
    script(
        bin,
        "df",
        r#"
printf '%s\n' 'Filesystem 1024-blocks Used Available Capacity Mounted on'
for arg do
    case "$arg" in
        -*) continue ;;
    esac
    [ -e "$arg" ] || exit 78
    case "$arg" in
        "$AE_SPEC_LOW_PATH"|"$AE_SPEC_LOW_PATH"/*)
            printf '%s\n' '/dev/lanesafe-low 8388608 7340032 1048576 88% /fixture low volume' ;;
        *)
            printf '%s\n' '/dev/lanesafe-high 8388608 4194304 4194304 50% /fixture high volume' ;;
    esac
done
"#,
    );
}

fn assert_disk_refusal(code: i32, out: &str, err: &str) {
    let words = format!("{out}{err}");
    assert_ne!(code, 0, "a low volume requires refusal: {words}");
    assert!(
        words.contains("lanesafe-low") || words.contains("low volume"),
        "the refusal must identify the low volume: {words}"
    );
    assert!(
        (words.contains("1 GiB") || words.contains("1.0 GiB") || words.contains("1048576"))
            && (words.contains("2 GiB") || words.contains("2.0 GiB") || words.contains("2097152")),
        "the refusal must identify the measured space and threshold: {words}"
    );
}

#[test]
fn low_checkout_space_refuses_claim_before_creating_the_lock() {
    let (scratch, checkout, _) = lane_lock_checkouts("disk");
    let bin = scratch.join("bin");
    fake_disk(&bin);
    script(
        &bin,
        "mkdir",
        r#"
case "${*}" in
    *'/.local/locks'*) : > "$AE_SPEC_EFFECT"; exit 79 ;;
esac
PATH=${PATH#*:} exec mkdir "$@"
"#,
    );
    let path = shim_path(&bin);
    let target = scratch.join("healthy target");
    fs::create_dir(&target).expect("the target has its own healthy volume");
    let low = checkout
        .canonicalize()
        .expect("the checkout's physical path");
    let effect = scratch.join("mkdir-effect");
    let (code, out, err) = run_in_env(
        &scratch,
        &checkout,
        "just",
        &["lane-lock", "claim", "diskcheck"],
        &[
            ("PATH", &path),
            ("AE_LANE_MIN_FREE_GIB", "2"),
            ("AE_SPEC_LOW_PATH", low.to_str().expect("checkout path")),
            ("AE_SPEC_EFFECT", effect.to_str().expect("effect path")),
            ("CARGO_TARGET_DIR", target.to_str().expect("target path")),
        ],
    );
    assert!(
        !effect.exists(),
        "a low disk must refuse before lock mkdir: {out}{err}"
    );
    assert!(
        !checkout.join(".local/locks").exists(),
        "the lock directory must stay absent"
    );
    assert_disk_refusal(code, &out, &err);
}

#[test]
fn low_target_space_refuses_release_before_fetch() {
    let (scratch, checkout, _) = lane_lock_checkouts("release-disk");
    let bin = scratch.join("bin");
    fake_disk(&bin);
    script(
        &bin,
        "gh",
        r#"
case "$1:$2" in
    auth:status) exit 0 ;;
    repo:view) printf 'fixture/repo\n' ;;
    api:*) printf 'true\n' ;;
    *) exit 78 ;;
esac
"#,
    );
    script(&bin, "x86_64-unknown-linux-musl-gcc", "exit 78");
    script(
        &bin,
        "git",
        r#"
case "$1" in
    fetch|pull|push|tag) printf '%s\n' "$1" >>"$AE_SPEC_EFFECT"; exit 79 ;;
esac
PATH=${PATH#*:} exec git "$@"
"#,
    );
    let path = shim_path(&bin);
    let mount = scratch.join("alternate target volume");
    fs::create_dir(&mount).expect("an existing ancestor on the target volume");
    let mount = mount
        .canonicalize()
        .expect("the target's physical volume path");
    let target = mount.join("target not created yet");
    let effect = scratch.join("release-effect");
    let (code, out, err) = run_in_env(
        &scratch,
        &checkout,
        "just",
        &["release"],
        &[
            ("PATH", &path),
            ("AE_LANE_MIN_FREE_GIB", "2"),
            ("AE_SPEC_LOW_PATH", mount.to_str().expect("mount path")),
            ("AE_SPEC_EFFECT", effect.to_str().expect("effect path")),
            ("CARGO_TARGET_DIR", target.to_str().expect("target path")),
            ("AE_RELEASE_HP", "spec@hp.invalid"),
            ("AE_RELEASE_HP_ROOT", "/tmp/ae-lanesafe-unused"),
        ],
    );
    assert!(
        !effect.exists(),
        "release attempted publication setup: {out}{err}"
    );
    assert!(
        !target.exists(),
        "probing a future target must not create it"
    );
    assert_disk_refusal(code, &out, &err);
}

fn wait_for_waiting(log: &Path) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if fs::read_to_string(log).is_ok_and(|text| text.contains("lane-lock: waiting")) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

#[test]
fn cancel_wait_stops_a_recorded_waiter_and_preserves_holder_and_queue() {
    let (scratch, checkout, linked) = lane_lock_checkouts("cancel-wait");
    let locks = checkout.join(".local/locks");
    let owner = locks.join("gate/owner");
    let queue = locks.join("next");
    let lock =
        |args: &[&str]| lane_lock_env(&scratch, &checkout, args, &[("AE_LANE_MIN_FREE_GIB", "0")]);
    let (code, out, err) = lock(&["claim", "holder"]);
    assert_eq!(code, 0, "{out}{err}");
    let owner_line = read(&owner);
    fs::write(&queue, "waiter\n").expect("the lead's queue record");

    let log = scratch.join("waiting-err");
    let child = helper_by_name("just")
        .current_dir(&linked)
        .env("HOME", scratch.path())
        .env("GIT_CONFIG_GLOBAL", scratch.join("gitconfig"))
        .env("AE_LANE_MIN_FREE_GIB", "0")
        .args(["lane-lock", "claim", "waiter", "9"])
        .stdout(Stdio::null())
        .stderr(fs::File::create(&log).expect("waiter stderr"))
        .spawn()
        .expect("start the private waiting claim");
    let waiting = wait_for_waiting(&log);
    let holder_result = lock(&["cancel-wait", "holder"]);
    let missing_result = lock(&["cancel-wait", "unknown"]);
    let cancellation = lock(&["cancel-wait", "waiter"]);
    // Give an unfixed waiter its short natural timeout, so the RED arm leaves
    // no sleeping recipe or child behind. A successful cancel must end sooner.
    let bound = if cancellation.0 == 0 { 3 } else { 11 };
    let finished = bounded(child, Duration::from_secs(bound)).expect("the private waiter ended");

    assert!(
        waiting,
        "the real recipe did not reach WAITING: {}",
        read(&log)
    );
    assert_ne!(
        holder_result.0, 0,
        "a holder cannot be cancelled: {holder_result:?}"
    );
    assert_ne!(
        missing_result.0, 0,
        "an unrecorded name cannot be cancelled: {missing_result:?}"
    );
    assert_eq!(
        cancellation.0, 0,
        "the recorded waiter was not cancelled: {cancellation:?}"
    );
    assert!(
        !finished.status.success(),
        "cancellation cannot report a successful claim"
    );
    assert_eq!(
        read(&owner),
        owner_line,
        "the holder remains the same claim"
    );
    assert_eq!(
        read(&queue),
        "waiter\n",
        "cancellation leaves the lead's queue alone"
    );
    assert_ne!(lock(&["cancel-wait", "waiter"]).0, 0);
    assert_eq!(lock(&["release", "holder"]).0, 0);
}

#[test]
fn the_mac_filtered_lane_selects_its_guard_even_when_the_manifest_omits_it() {
    let (scratch, checkout, _) = lane_lock_checkouts("mac-guard");
    let bin = scratch.join("bin");
    script(
        &bin,
        "cargo",
        r#"
case "$1:${2:-}" in
    nextest:run) printf '%s\n' "$@" >> "$AE_SPEC_CARGO_ARGS" ;;
    *) exit 78 ;;
esac
"#,
    );
    fs::create_dir(checkout.join(".config")).expect("the subset config's directory");
    fs::write(checkout.join(".config/macos-subset"), "unit\n").expect("a valid sparse manifest");
    let temp = scratch.join("tmp");
    fs::create_dir(&temp).expect("a private lane temp base");
    let temp = temp.to_str().expect("temp path");
    let log = scratch.join("nextest-run");
    let (code, out, err) = run_in_env(
        &scratch,
        &checkout,
        "just",
        &["test-macos"],
        &[
            ("PATH", &shim_path(&bin)),
            ("AE_TEST_TMPDIR", temp),
            ("TMPDIR", temp),
            ("AE_SPEC_CARGO_ARGS", log.to_str().expect("log path")),
        ],
    );
    assert_eq!(
        code, 0,
        "the syntactically valid manifest can render: {out}{err}"
    );
    let args = read(&log);
    assert!(
        args.contains("gate::macos_subset") || args.contains("test(/^gate::/)"),
        "the selected integration tests must include the guard even if the manifest drops its module: {args}"
    );
}

#[test]
fn lane_start_reaps_own_dead_scratch_and_leaves_live_and_linked_paths_alone() {
    let (scratch, checkout, _) = lane_lock_checkouts("sweep");
    let base = scratch.join("tmp");
    fs::create_dir(&base).expect("a private lane temp base");
    let owner = killed_scratch_child(&base, &base.join("no-lane"), None);
    let stale = base.join(format!("ae-it-{owner}"));
    let stale_writer = stale.join("killed");
    fs::write(stale_writer.join("held"), "old writer's output").expect("the stale writer's byte");
    fs::set_permissions(&stale_writer, fs::Permissions::from_mode(0o555))
        .expect("a writer can die before restoring directory permissions");
    let live = base.join(format!("ae-it-{}", std::process::id()));
    fs::create_dir(&live).expect("this process has a live scratch root");
    fs::write(live.join("held"), "live writer's output").expect("the live writer's byte");
    let link = base.join(format!("ae-it-0{owner}"));
    symlink(&live, &link).expect("a matching name that is only a symlink");

    let bin = scratch.join("bin");
    let log = scratch.join("sweep-state");
    script(
        &bin,
        "cargo",
        r#"
case "$1:${2:-}" in
    nextest:run)
        for path in "$AE_SPEC_STALE" "$AE_SPEC_LIVE" "$AE_SPEC_LINK"; do
            if [ -e "$path" ] || [ -L "$path" ]; then echo present; else echo absent; fi
        done > "$AE_SPEC_SWEEP_STATE"
        ;;
esac
"#,
    );
    let (code, out, err) = run_in_env(
        &scratch,
        &checkout,
        "just",
        &["_tmux-isolated", "test", "-E", "all()"],
        &[
            ("PATH", &shim_path(&bin)),
            ("AE_TEST_TMPDIR", base.to_str().expect("temp path")),
            ("TMPDIR", base.to_str().expect("temp path")),
            ("AE_SPEC_STALE", stale.to_str().expect("stale path")),
            ("AE_SPEC_LIVE", live.to_str().expect("live path")),
            ("AE_SPEC_LINK", link.to_str().expect("symlink path")),
            ("AE_SPEC_SWEEP_STATE", log.to_str().expect("state path")),
        ],
    );
    // Keep the fixture removable on either side of the test.
    if stale_writer.is_dir() {
        fs::set_permissions(stale_writer, fs::Permissions::from_mode(0o755))
            .expect("restore the stale fixture for cleanup");
    }
    assert_eq!(code, 0, "the fake build must run: {out}{err}");
    assert_eq!(
        read(&log),
        "absent\npresent\npresent\n",
        "state when the first test would start"
    );
    assert_eq!(read(&live.join("held")), "live writer's output");
    assert!(
        link.is_symlink(),
        "a matching symlink is never a scratch directory"
    );
}
