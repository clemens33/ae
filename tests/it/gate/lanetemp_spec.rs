//! Acceptance for the lane lock's thermal log and bounded cool-down (slice hpheat). These tests
//! drive the real justfile in private checkouts and read a fake sysfs tree named by
//! `AE_LANE_SYSFS_ROOT`, so no test depends on the machine's own temperature.
#![allow(
    clippy::expect_used,
    reason = "fixture failures must name the failed prerequisite"
)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use super::{lane_lock_checkouts, lane_lock_env, read};
use crate::cli::{OwnedChild, bounded, helper_by_name};

const CORE: &str = "sys/devices/platform/coretemp.0/hwmon/hwmon1";
const THROTTLE: &str = "sys/devices/system/cpu/cpu0/thermal_throttle";

/// A sysfs tree as a Linux host lays it out: `class/hwmon/hwmonN` are symlinks into `devices`, a
/// decoy hwmon named `acpitz` reads 99 C, and the real `coretemp` one reads `temp` millidegrees.
pub(super) fn fake_sysfs(scratch: &Path, temp: &str, count: &str) -> PathBuf {
    let root = scratch.join("sysroot");
    let decoy = root.join("sys/devices/platform/acpitz/hwmon/hwmon0");
    fs::create_dir_all(&decoy).expect("the decoy hwmon directory");
    fs::write(decoy.join("name"), "acpitz\n").expect("the decoy name");
    fs::write(decoy.join("temp1_input"), "99000\n").expect("the decoy temperature");
    fs::create_dir_all(root.join(CORE)).expect("the coretemp hwmon directory");
    fs::write(root.join(CORE).join("name"), "coretemp\n").expect("the coretemp name");
    fs::write(root.join(CORE).join("temp1_input"), format!("{temp}\n"))
        .expect("the package temperature");
    fs::create_dir_all(root.join("sys/class/hwmon")).expect("the hwmon class directory");
    symlink(
        "../../devices/platform/acpitz/hwmon/hwmon0",
        root.join("sys/class/hwmon/hwmon0"),
    )
    .expect("the decoy hwmon link");
    symlink(
        "../../devices/platform/coretemp.0/hwmon/hwmon1",
        root.join("sys/class/hwmon/hwmon1"),
    )
    .expect("the coretemp hwmon link");
    fs::create_dir_all(root.join(THROTTLE)).expect("the throttle directory");
    set_count(&root, count);
    root
}

fn set_temp(root: &Path, temp: &str) {
    fs::write(root.join(CORE).join("temp1_input"), format!("{temp}\n"))
        .expect("a new package temperature");
}

fn set_count(root: &Path, count: &str) {
    fs::write(
        root.join(THROTTLE).join("package_throttle_count"),
        format!("{count}\n"),
    )
    .expect("a new throttle count");
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a utf-8 path")
}

/// A claim that is still running: its child and the files its output goes to.
struct Running {
    child: OwnedChild,
    out: PathBuf,
    err: PathBuf,
}

fn spawn_claim(scratch: &Path, cwd: &Path, name: &str, tag: &str, env: &[(&str, &str)]) -> Running {
    let (out, err) = (
        scratch.join(format!("{tag}.out")),
        scratch.join(format!("{tag}.err")),
    );
    let mut command = helper_by_name("just");
    command
        .current_dir(cwd)
        .env("HOME", scratch)
        .env("GIT_CONFIG_GLOBAL", scratch.join("gitconfig"))
        .env("AE_LANE_MIN_FREE_GIB", "0")
        .env("AE_LANE_MAX_TEMP_C", "0");
    for (key, value) in env {
        command.env(key, value);
    }
    let child = command
        .args(["lane-lock", "claim", name])
        .stdout(fs::File::create(&out).expect("claim stdout"))
        .stderr(fs::File::create(&err).expect("claim stderr"))
        .stdin(Stdio::null())
        .spawn()
        .expect("start the private claim");
    Running { child, out, err }
}

fn wait_for(log: &Path, needle: &str) -> bool {
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if fs::read_to_string(log).is_ok_and(|text| text.contains(needle)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

const HOT: [(&str, &str); 3] = [
    ("AE_LANE_MAX_TEMP_C", "75"),
    ("AE_LANE_COOL_STEP_SECS", "1"),
    ("AE_LANE_COOL_CAP_SECS", "3"),
];

fn with<'a>(root: &'a Path, base: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut env = vec![("AE_LANE_SYSFS_ROOT", text(root))];
    env.extend_from_slice(base);
    env
}

#[test]
fn claim_and_release_each_print_one_thermal_line_after_their_result_line() {
    let (scratch, checkout, _) = lane_lock_checkouts("thermal-log");
    let root = fake_sysfs(&scratch, "61200", "377596");
    let env = with(&root, &[]);
    let (code, out, err) = lane_lock_env(&scratch, &checkout, &["claim", "logger"], &env);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains(
            "lane-lock: claimed by logger\nlane-lock: thermal at claim: package 61.2 C, package_throttle_count 377596\n"
        ),
        "the claim names the coretemp package, not the decoy: {out}"
    );
    assert_eq!(
        read(&checkout.join(".local/locks/gate/owner"))
            .lines()
            .count(),
        1,
        "the owner record stays one line"
    );

    set_temp(&root, "74040");
    set_count(&root, "377611");
    let (code, out, err) = lane_lock_env(&scratch, &checkout, &["release", "logger"], &env);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains(
            "lane-lock: released by logger\nlane-lock: thermal at release: package 74.0 C, package_throttle_count 377611\n"
        ),
        "the release reads the box again: {out}"
    );
    assert!(!checkout.join(".local/locks/gate").exists());
}

#[test]
fn a_hot_claim_cools_while_holding_no_gate_then_claims_anyway_in_one_line() {
    let (scratch, checkout, _) = lane_lock_checkouts("hot-cap");
    let root = fake_sysfs(&scratch, "90000", "5");
    let started = Instant::now();
    let run = spawn_claim(&scratch, &checkout, "hotter", "hot", &with(&root, &HOT));
    let cooling = wait_for(&run.err, "lane-lock: cooling, ");
    let gate_while_cooling = checkout.join(".local/locks/gate").exists();
    let registered = checkout.join(".local/locks/waiting").is_dir();
    let finished = bounded(run.child, Duration::from_secs(20)).expect("the claim ended");
    let (out, err) = (read(&run.out), read(&run.err));

    assert!(cooling, "a hot reading must cool first: {err}");
    assert!(!gate_while_cooling, "a cooling claim holds no gate");
    assert!(
        registered,
        "a cooling claim registers so cancel-wait can find it"
    );
    assert!(finished.status.success(), "{out}{err}");
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "the claim waited out the cap: {:?}",
        started.elapsed()
    );
    assert_eq!(
        err.matches("claiming anyway").count(),
        1,
        "one line says the cap ended the wait: {err}"
    );
    assert!(
        err.contains("above 75 C (AE_LANE_MAX_TEMP_C)"),
        "the line names the threshold variable: {err}"
    );
    assert!(out.contains("lane-lock: claimed by hotter"), "{out}");
    assert!(read(&checkout.join(".local/locks/gate/owner")).starts_with("hotter "));
}

#[test]
fn a_reading_that_falls_ends_the_cool_down_before_the_cap() {
    let (scratch, checkout, _) = lane_lock_checkouts("cools");
    let root = fake_sysfs(&scratch, "90000", "5");
    let slow = [
        ("AE_LANE_MAX_TEMP_C", "75"),
        ("AE_LANE_COOL_STEP_SECS", "1"),
        ("AE_LANE_COOL_CAP_SECS", "60"),
    ];
    let started = Instant::now();
    let run = spawn_claim(&scratch, &checkout, "cooler", "cool", &with(&root, &slow));
    assert!(
        wait_for(&run.err, "lane-lock: cooling, "),
        "{}",
        read(&run.err)
    );
    set_temp(&root, "60000");
    let finished = bounded(run.child, Duration::from_secs(15)).expect("the claim ended");
    let (out, err) = (read(&run.out), read(&run.err));
    assert!(finished.status.success(), "{out}{err}");
    assert!(
        started.elapsed() < Duration::from_secs(40),
        "the claim must not wait out the 60 s cap: {:?}",
        started.elapsed()
    );
    assert!(
        !err.contains("claiming anyway"),
        "a cool box says nothing: {err}"
    );
    assert!(
        out.contains("thermal at claim: package 60.0 C"),
        "the claim line reads the cooled box: {out}"
    );
}

#[test]
fn an_unknown_reading_never_waits_and_prints_unknown() {
    let hot = [
        ("AE_LANE_MAX_TEMP_C", "75"),
        ("AE_LANE_COOL_STEP_SECS", "1"),
        ("AE_LANE_COOL_CAP_SECS", "60"),
    ];
    for case in ["no-sys", "no-root", "no-temp", "bad-temp", "no-coretemp"] {
        let (scratch, checkout, _) = lane_lock_checkouts(&format!("unknown-{case}"));
        let root = fake_sysfs(&scratch, "90000", "5");
        match case {
            "no-sys" => {
                fs::remove_dir_all(root.join("sys")).expect("a root without sys");
            }
            "no-root" => {
                fs::remove_dir_all(&root).expect("a root that does not exist");
            }
            "no-temp" => {
                fs::remove_file(root.join(CORE).join("temp1_input")).expect("no temp1_input");
            }
            "bad-temp" => set_temp(&root, "hot"),
            _ => fs::write(root.join(CORE).join("name"), "k10temp\n").expect("another driver"),
        }
        let started = Instant::now();
        let (code, out, err) =
            lane_lock_env(&scratch, &checkout, &["claim", "blind"], &with(&root, &hot));
        assert_eq!(code, 0, "{case}: {out}{err}");
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{case}: an unknown reading never waits"
        );
        assert!(!err.contains("cooling"), "{case}: {err}");
        assert!(
            out.contains("lane-lock: thermal at claim: package unknown, package_throttle_count "),
            "{case}: {out}"
        );
        if case == "no-sys" || case == "no-root" {
            assert!(
                out.contains("package unknown, package_throttle_count unknown"),
                "{case}: both fields read unknown: {out}"
            );
        }
    }
}

#[test]
fn a_missing_throttle_count_reads_unknown_beside_a_known_temperature() {
    let (scratch, checkout, _) = lane_lock_checkouts("no-count");
    let root = fake_sysfs(&scratch, "61200", "5");
    fs::remove_file(root.join(THROTTLE).join("package_throttle_count")).expect("no count file");
    let (code, out, err) = lane_lock_env(
        &scratch,
        &checkout,
        &["claim", "counter"],
        &with(&root, &[]),
    );
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains("thermal at claim: package 61.2 C, package_throttle_count unknown"),
        "{out}"
    );
}

#[test]
fn zero_turns_the_wait_off_and_only_a_reading_above_the_threshold_waits() {
    let slow = |max: &'static str| {
        [
            ("AE_LANE_MAX_TEMP_C", max),
            ("AE_LANE_COOL_STEP_SECS", "1"),
            ("AE_LANE_COOL_CAP_SECS", "2"),
        ]
    };
    for (tag, temp, max, waits) in [
        ("off", "99000", "0", false),
        ("equal", "75000", "75", false),
        ("above", "75100", "75", true),
    ] {
        let (scratch, checkout, _) = lane_lock_checkouts(&format!("limit-{tag}"));
        let root = fake_sysfs(&scratch, temp, "5");
        let (code, out, err) = lane_lock_env(
            &scratch,
            &checkout,
            &["claim", "limit"],
            &with(&root, &slow(max)),
        );
        assert_eq!(code, 0, "{tag}: {out}{err}");
        assert_eq!(err.contains("cooling"), waits, "{tag}: {err}");
        assert!(
            out.contains("lane-lock: thermal at claim: package "),
            "{tag}: the log line stays even when the wait is off: {out}"
        );
    }
}

#[test]
fn a_malformed_variable_refuses_with_exit_2_before_any_effect() {
    let (scratch, checkout, _) = lane_lock_checkouts("malformed");
    let root = fake_sysfs(&scratch, "61200", "5");
    for (name, value) in [
        ("AE_LANE_MAX_TEMP_C", "abc"),
        ("AE_LANE_MAX_TEMP_C", "-5"),
        ("AE_LANE_MAX_TEMP_C", "007"),
        ("AE_LANE_MAX_TEMP_C", "75.5"),
        ("AE_LANE_MAX_TEMP_C", "1000"),
        ("AE_LANE_COOL_STEP_SECS", "0"),
        ("AE_LANE_COOL_STEP_SECS", "x"),
        ("AE_LANE_COOL_CAP_SECS", "-1"),
        ("AE_LANE_COOL_CAP_SECS", "01"),
        ("AE_LANE_SYSFS_ROOT", "relative/sys"),
    ] {
        let (code, out, err) = lane_lock_env(
            &scratch,
            &checkout,
            &["claim", "refused"],
            &with(&root, &[(name, value)]),
        );
        assert_eq!(code, 2, "{name}={value}: {out}{err}");
        assert!(
            err.contains("lane-lock: refused: ") && err.contains(name) && err.contains(value),
            "{name}={value}: the refusal names the variable and the value: {err}"
        );
        assert!(
            !checkout.join(".local").exists(),
            "{name}={value}: a refused claim makes no directory"
        );
    }
}

#[test]
fn a_reading_that_vanishes_mid_cool_down_ends_it_quietly_and_claims() {
    let (scratch, checkout, _) = lane_lock_checkouts("vanish");
    let root = fake_sysfs(&scratch, "90000", "5");
    let slow = [
        ("AE_LANE_MAX_TEMP_C", "75"),
        ("AE_LANE_COOL_STEP_SECS", "1"),
        ("AE_LANE_COOL_CAP_SECS", "60"),
    ];
    let run = spawn_claim(&scratch, &checkout, "blinded", "blind", &with(&root, &slow));
    assert!(
        wait_for(&run.err, "lane-lock: cooling, "),
        "{}",
        read(&run.err)
    );
    fs::remove_file(root.join(CORE).join("temp1_input")).expect("the sensor goes away");
    let finished = bounded(run.child, Duration::from_secs(15)).expect("the claim ended");
    let (out, err) = (read(&run.out), read(&run.err));
    assert!(finished.status.success(), "{out}{err}");
    assert!(
        !err.contains("claiming anyway"),
        "an unknown reading is not a cap: {err}"
    );
    assert!(
        out.contains("thermal at claim: package unknown"),
        "the claim line says unknown: {out}"
    );
}

#[test]
fn cancel_wait_stops_a_cooling_claim_before_it_takes_the_lock() {
    let (scratch, checkout, _) = lane_lock_checkouts("cancel-cool");
    let root = fake_sysfs(&scratch, "90000", "5");
    let slow = [
        ("AE_LANE_MAX_TEMP_C", "75"),
        ("AE_LANE_COOL_STEP_SECS", "1"),
        ("AE_LANE_COOL_CAP_SECS", "60"),
    ];
    let run = spawn_claim(&scratch, &checkout, "chilled", "chill", &with(&root, &slow));
    assert!(
        wait_for(&run.err, "lane-lock: cooling, "),
        "{}",
        read(&run.err)
    );
    let (code, out, err) = lane_lock_env(&scratch, &checkout, &["cancel-wait", "chilled"], &[]);
    assert_eq!(code, 0, "a cooling claim is cancellable: {out}{err}");
    let finished = bounded(run.child, Duration::from_secs(8)).expect("the cancelled claim ended");
    assert!(!finished.status.success(), "a cancelled claim cannot claim");
    assert!(read(&run.err).contains("cancelled"), "{}", read(&run.err));
    assert!(!read(&run.out).contains("claimed by"), "{}", read(&run.out));
    assert!(
        !checkout.join(".local/locks/gate").exists(),
        "no gate was ever taken"
    );
}

#[test]
fn a_cap_of_zero_claims_at_once_with_the_line_and_no_registration() {
    let (scratch, checkout, _) = lane_lock_checkouts("cap-zero");
    let root = fake_sysfs(&scratch, "90000", "5");
    let env = with(
        &root,
        &[("AE_LANE_MAX_TEMP_C", "75"), ("AE_LANE_COOL_CAP_SECS", "0")],
    );
    let (code, out, err) = lane_lock_env(&scratch, &checkout, &["claim", "quick"], &env);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(err.matches("claiming anyway").count(), 1, "{err}");
    assert!(!err.contains("lane-lock: cooling, "), "{err}");
    assert!(
        !checkout.join(".local/locks/waiting").exists(),
        "nothing slept, so nothing registered"
    );
}

#[test]
fn a_blocked_claim_does_not_cool_and_does_not_change_the_queue() {
    let (scratch, checkout, _) = lane_lock_checkouts("blocked");
    let root = fake_sysfs(&scratch, "90000", "5");
    let (code, out, err) = lane_lock_env(&scratch, &checkout, &["claim", "holder"], &[]);
    assert_eq!(code, 0, "{out}{err}");
    let env = with(&root, &HOT);
    let started = Instant::now();
    let (code, _, err) = lane_lock_env(&scratch, &checkout, &["claim", "waiter"], &env);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("not claimed: gate held by holder"), "{err}");
    assert!(
        !err.contains("cooling"),
        "a holder blocks before any cool-down: {err}"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(
        lane_lock_env(&scratch, &checkout, &["release", "holder"], &[]).0,
        0
    );

    let queue = checkout.join(".local/locks/next");
    fs::write(&queue, "other\n").expect("the lead's queue record");
    let (code, _, err) = lane_lock_env(&scratch, &checkout, &["claim", "waiter"], &env);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("queue: next is other, not waiter"), "{err}");
    assert!(
        !err.contains("cooling"),
        "the queue blocks before any cool-down: {err}"
    );
    assert_eq!(read(&queue), "other\n", "the queue is not touched");
}
