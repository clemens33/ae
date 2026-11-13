//! Driver regression for the parallel release proof (relpar), beside the frozen
//! `receipt_spec/parallel_spec.rs`. It runs the justfile's own `HP_LEG` functions in
//! bash, with real processes. The hp leg runs only on the hp, a Linux host.
#![cfg(target_os = "linux")]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use super::{justfile_variable, read, root, run_in_env};
use crate::cli::OwnedScratch;

fn hp_leg_text() -> String {
    let text = justfile_variable(&read(&root().join("justfile")), "HP_LEG");
    assert!(!text.is_empty(), "the justfile defines HP_LEG");
    text
}

fn write(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap_or_else(|why| panic!("{}: {why}", path.display()));
}

/// `just test` leaves an owned descendant in a session of its own, as cargo
/// runs under `detached()`, and then blocks; every other recipe is the real just.
const FAKE_JUST: &str = r#"#!/usr/bin/env bash
if [ "${1-}" = test ]; then
    setsid sleep 300 </dev/null >/dev/null 2>&1 &
    echo "$!" >"$AE_REG_DIR/descendant"
    echo "$$" >"$AE_REG_DIR/gate.tmp" && mv "$AE_REG_DIR/gate.tmp" "$AE_REG_DIR/gate"
    exec sleep 300
fi
exec "$AE_REG_REAL_JUST" "$@"
"#;

/// Starts the leg in the background, TERMs only the gate's own process and
/// waits for the leg to end.
const SIGNAL_THE_GATE: &str = r#"set -u
leg=$1 dir=$2; shift 2
bash -c "$(cat "$leg")"'; hp_leg "$@"' hp-leg "$@" </dev/null >"$dir/leg.log" 2>&1 &
leg_pid=$!
for _ in $(seq 1 400); do [ -s "$dir/gate" ] && break; sleep 0.05; done
[ -s "$dir/gate" ] || { echo "control: just test never started"; cat "$dir/leg.log"; exit 90; }
kill -TERM "$(cat "$dir/gate")"
wait "$leg_pid"
echo "leg exit $?"
"#;

fn executable(path: &Path, body: &str) {
    write(path, body);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|why| panic!("{}: {why}", path.display()));
}

fn tool(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("the regression needs {name} on PATH"))
}

/// A one-commit git repository at `hp` with the real justfile; returns its
/// head and tree.
fn fixture_repo(base: &Path, hp: &Path, env: &[(&str, &str)]) -> (String, String) {
    std::fs::copy(root().join("justfile"), hp.join("justfile"))
        .unwrap_or_else(|why| panic!("copy justfile: {why}"));
    write(&hp.join(".gitignore"), ".local/\n");
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.name", "Fixture"],
        vec!["config", "user.email", "fixture@example.invalid"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "fixture"],
    ] {
        let (code, out, err) = run_in_env(base, hp, "git", &args, env);
        assert_eq!(code, 0, "git {args:?}: {out}{err}");
    }
    let rev = |what: &str| {
        let (code, out, err) = run_in_env(base, hp, "git", &["rev-parse", what], env);
        assert_eq!(code, 0, "rev-parse {what}: {out}{err}");
        out.trim().to_owned()
    };
    (rev("HEAD"), rev("HEAD^{tree}"))
}

/// A signal that ends `just test` without reaching the leg (here a TERM to the
/// gate's own process only, exit 143) may leave detached descendants behind, so
/// the leg keeps the hp lane lock, writes its suspect note and records the
/// status with lock=held, exactly as on its own timeout.
#[test]
fn hp_leg_keeps_the_lock_when_a_signal_ends_just_test() {
    let base = OwnedScratch::root("gate", "relpar-legsignal");
    let bin = base.join("bin");
    let hp = base.join("hp");
    for dir in [base.join("io"), bin.clone(), hp.clone()] {
        std::fs::create_dir_all(&dir).unwrap_or_else(|why| panic!("{}: {why}", dir.display()));
    }
    let leg = base.join("hp_leg.sh");
    write(&leg, &hp_leg_text());
    executable(&bin.join("just"), FAKE_JUST);
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let real_just = tool("just");
    let env = [
        ("PATH", path.as_str()),
        ("AE_REG_DIR", base.to_str().unwrap_or_default()),
        ("AE_REG_REAL_JUST", real_just.to_str().unwrap_or_default()),
    ];
    let (head, tree) = fixture_repo(&base, &hp, &env);
    let lock = format!("release-{}", &head[..12]);
    std::fs::create_dir_all(hp.join(".local/release"))
        .unwrap_or_else(|why| panic!("release dir: {why}"));
    let (code, out, err) = run_in_env(
        &base,
        &base,
        "bash",
        &[
            "-c",
            SIGNAL_THE_GATE,
            "legsignal",
            leg.to_str().unwrap_or_default(),
            base.to_str().unwrap_or_default(),
            hp.to_str().unwrap_or_default(),
            &head,
            &tree,
            &lock,
            "regress-run",
        ],
        &env,
    );
    let descendant = read(&base.join("descendant"));
    let _ = run_in_env(&base, &base, "kill", &["-KILL", descendant.trim()], &[]);
    let log = read(&base.join("leg.log"));
    assert_eq!(code, 0, "{out}{err}\n{log}");
    let record = read(&hp.join(".local/release/regress-run.exit"));
    assert!(
        record.starts_with("gate=143 ") && record.contains(" lock=held"),
        "a signal-ended gate records its status with the lock held: {record}\n{log}"
    );
    let owner = read(&hp.join(".local/locks/gate/owner"));
    assert!(
        owner.starts_with(&format!("{lock} ")),
        "the hp lane lock stays held by the leg: {owner}\n{log}"
    );
    let suspect = read(&hp.join(".local/release/regress-run.suspect"));
    assert!(
        suspect.contains(&head) && suspect.contains("143"),
        "the suspect note names the run, the commit and the status: {suspect}"
    );
}
