//! Frozen relpar acceptance, authored by relpar-nav (OpenAI/gpt-6.1-sol).
//! Contract: plan-r3 bbde9f09b7d4a65aa6010aa9aa772b24d5da4a82, lead finish-alone and Mac foreground-parity rulings.
//! These tests execute the real recipes, receipt writer and lane lock.
//! Only external tools are substituted. The ssh substitute executes its command and stdin locally.
//! Linux executes the real detached hp topology. macOS signal propagation also needs the P3 probe.
#![cfg(target_os = "linux")]
#![allow(
    clippy::too_many_lines,
    reason = "independent acceptance fixtures keep the shell fault scenarios together"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{read, root, run_in_env};
use crate::cli::OwnedScratch;

fn put(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| panic!("{}: {e}", parent.display()));
    }
    std::fs::write(path, body).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

fn executable(path: &Path, body: &str) {
    // A tool path can be a symlink. Never write through it.
    if path.exists() || path.is_symlink() {
        std::fs::remove_file(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
    put(path, body);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

fn tool(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|p| p.join(name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| panic!("the fixture needs {name} on PATH"))
}

struct Fixture {
    base: OwnedScratch,
    mac: PathBuf,
    hp: PathBuf,
    env: Vec<(String, String)>,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let base = OwnedScratch::root("gate", &format!("relpar-{tag}"));
        let mac = base.join("mac");
        let hp = base.join("hp");
        for dir in [base.join("bin"), base.join("io"), mac.clone()] {
            std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("fixture directory: {e}"));
        }
        let mut fixture = Self {
            base,
            mac,
            hp,
            env: Vec::new(),
        };
        fixture.stock();
        fixture.checkout();
        fixture
    }

    fn stock(&mut self) {
        let bin = self.base.join("bin");
        let pin = read(&root().join("justfile"))
            .lines()
            .find_map(|l| {
                l.strip_prefix("NEXTEST_VERSION := \"")
                    .and_then(|s| s.strip_suffix('"'))
            })
            .unwrap_or_else(|| panic!("the exact nextest pin"))
            .to_owned();
        let mut pairs = vec![
            (
                "PATH".to_owned(),
                format!(
                    "{}:{}",
                    bin.display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            ),
            ("AE_RELEASE_HP".into(), "spec@hp.invalid".into()),
            ("AE_RELEASE_HP_ROOT".into(), self.hp.display().to_string()),
            ("AE_PAR_BASE".into(), self.base.display().to_string()),
            ("AE_PAR_HP".into(), self.hp.display().to_string()),
            ("AE_PAR_PIN".into(), pin),
            ("AE_PAR_SIDE".into(), "mac".into()),
            ("AE_PAR_MODE".into(), "normal".into()),
            ("AE_TEST_TMPDIR".into(), self.base.display().to_string()),
            ("TMPDIR".into(), self.base.display().to_string()),
            ("RUSTC_WRAPPER".into(), String::new()),
            ("CARGO_BUILD_RUSTC_WRAPPER".into(), String::new()),
        ];
        for name in ["git", "just", "sleep", "timeout"] {
            pairs.push((
                format!("AE_PAR_REAL_{}", name.to_uppercase()),
                tool(name).display().to_string(),
            ));
        }
        self.env = pairs;
        std::os::unix::fs::symlink(tool("tmux"), bin.join("tmux"))
            .unwrap_or_else(|e| panic!("link tmux: {e}"));
        executable(&bin.join("git"), GIT);
        executable(&bin.join("just"), JUST);
        executable(&bin.join("ssh"), SSH);
        executable(&bin.join("cargo"), CARGO);
        executable(&bin.join("timeout"), TIMEOUT);
        executable(
            &bin.join("sleep"),
            "#!/bin/sh\ncase \"$1\" in 30) exec \"$AE_PAR_REAL_SLEEP\" 0.02 ;; *) exec \"$AE_PAR_REAL_SLEEP\" \"$@\" ;; esac\n",
        );
        executable(&bin.join("gtimeout"), "#!/bin/sh\nexec timeout \"$@\"\n");
        executable(
            &bin.join("hostname"),
            "#!/bin/sh\nprintf 'spec-%s\\n' \"$AE_PAR_SIDE\"\n",
        );
        executable(
            &bin.join("rustc"),
            "#!/bin/sh\nprintf '%s\\n' 'rustc 1.97.1 (relpar-fixture)'\n",
        );
        for name in [
            "taplo",
            "shfmt",
            "shellcheck",
            "claude",
            "codex",
            "gemini",
            "agy",
            "grok",
            "muse",
            "opencode",
        ] {
            executable(&bin.join(name), "#!/bin/sh\nprintf '%s\\n' 'fixture 1'\n");
        }
    }

    fn call(
        &self,
        cwd: &Path,
        program: &str,
        args: &[&str],
        extra: &[(&str, &str)],
    ) -> (i32, String, String) {
        let env: Vec<_> = self
            .env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .chain(extra.iter().copied())
            .collect();
        run_in_env(&self.base, cwd, program, args, &env)
    }

    fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let (code, out, err) = self.call(cwd, "git", args, &[]);
        assert_eq!(code, 0, "git {args:?}: {out}{err}");
        out
    }

    fn checkout(&self) {
        for (name, body) in [
            (
                "Cargo.toml",
                "[package]\nname = \"ae\"\nversion = \"0.0.0\"\n",
            ),
            (
                "Cargo.lock",
                "version = 4\n\n[[package]]\nname = \"ae\"\nversion = \"0.0.0\"\n",
            ),
            ("fuzz/Cargo.toml", "[package]\nname = \"ae-fuzz\"\n"),
            (
                "fuzz/Cargo.lock",
                "version = 4\n\n[[package]]\nname = \"ae\"\nversion = \"0.0.0\"\n",
            ),
            ("src/lib.rs", ""),
            ("tests/it/main.rs", "mod boundary;\n"),
            (
                "tests/it/boundary.rs",
                "#[test] fn mac() { let _ = cfg!(target_os = \"macos\"); }\n",
            ),
            (".config/macos-subset", "unit\nmodule boundary\n"),
            (
                ".config/nextest.toml",
                "[profile.default]\ntest-threads = 1\n",
            ),
            (".gitignore", ".local/\ntarget/\n"),
        ] {
            put(&self.mac.join(name), body);
        }
        for name in ["justfile", "rust-toolchain.toml"] {
            std::fs::copy(root().join(name), self.mac.join(name))
                .unwrap_or_else(|e| panic!("copy {name}: {e}"));
        }
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.name", "Fixture"],
            vec!["config", "user.email", "fixture@example.invalid"],
            vec!["add", "-A"],
            vec!["commit", "-qm", "fixture base"],
        ] {
            self.git(&self.mac, &args);
        }
        put(&self.mac.join("version-marker"), "requested commit\n");
        self.git(&self.mac, &["add", "version-marker"]);
        self.git(&self.mac, &["commit", "-qm", "requested commit"]);
        self.git(
            &self.mac,
            &[
                "clone",
                "-q",
                "--no-local",
                self.mac.to_str().unwrap_or_default(),
                self.hp.to_str().unwrap_or_default(),
            ],
        );
        self.git(&self.hp, &["checkout", "-q", "--detach", "HEAD"]);
        let publication = self.base.join("publication.git");
        self.git(
            &self.mac,
            &[
                "init",
                "-q",
                "--bare",
                publication.to_str().unwrap_or_default(),
            ],
        );
        self.git(
            &self.mac,
            &[
                "remote",
                "add",
                "origin",
                publication.to_str().unwrap_or_default(),
            ],
        );
        self.git(&self.mac, &["push", "-q", "origin", "main"]);
        // Setup pushes are outside the observation window.
        put(&self.base.join("git-trace"), "");
    }

    fn proof(&self, extra: &[(&str, &str)]) -> (i32, String, String) {
        let head = self.git(&self.mac, &["rev-parse", "HEAD"]);
        let tree = self.git(&self.mac, &["rev-parse", "HEAD^{tree}"]);
        let expect = format!("expect={}:{}", head.trim(), tree.trim());
        self.call(
            &self.mac,
            "just",
            &["release-proof", &expect, "hp=run"],
            extra,
        )
    }

    fn head(&self) -> String {
        self.git(&self.mac, &["rev-parse", "HEAD"])
            .trim()
            .to_owned()
    }

    fn tree(&self) -> String {
        self.git(&self.mac, &["rev-parse", "HEAD^{tree}"])
            .trim()
            .to_owned()
    }

    fn receipt(&self) -> PathBuf {
        self.hp.join(".local/gate-receipts").join(self.tree())
    }

    fn owner(&self) -> PathBuf {
        self.hp.join(".local/locks/gate/owner")
    }

    fn trace(&self) -> String {
        std::fs::read_to_string(self.base.join("trace")).unwrap_or_default()
    }

    fn release_hp(&self) {
        put(&self.base.join("hp-release"), "");
        self.until(
            || self.trace().contains("hp:nextest:end"),
            "the independently running hp completes nextest",
        );
        self.until(
            || {
                self.exit_files()
                    .iter()
                    .any(|p| std::fs::read_to_string(p).is_ok_and(|s| s.contains("lock=released")))
            },
            "the hp publishes its final exit after releasing its lock",
        );
    }

    fn exit_files(&self) -> Vec<PathBuf> {
        std::fs::read_dir(self.hp.join(".local/release"))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "exit"))
            .collect()
    }

    fn until(&self, mut check: impl FnMut() -> bool, why: &str) {
        let start = Instant::now();
        while !check() && start.elapsed() < Duration::from_mins(1) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(check(), "{why}; trace={}", self.trace());
    }

    fn unchanged_publication(&self) {
        assert_eq!(
            self.git(&self.mac, &["status", "--porcelain"]),
            "",
            "Mac stays clean"
        );
        assert!(read(&self.mac.join("Cargo.toml")).contains("version = \"0.0.0\""));
        assert_eq!(self.git(&self.mac, &["tag", "--list"]), "");
        assert_eq!(
            self.git(&self.mac, &["ls-remote", "--heads", "origin"]),
            format!("{}\trefs/heads/main\n", self.head()),
            "publication branch stays at its original commit"
        );
        assert_eq!(self.git(&self.mac, &["ls-remote", "--tags", "origin"]), "");
        for line in read(&self.base.join("git-trace"))
            .lines()
            .filter(|l| l.contains("\tpush\t"))
        {
            assert!(!line.contains("\torigin\t"), "no publication push: {line}");
            assert!(
                !line.contains("--force") && !line.contains("\t-f\t"),
                "no force: {line}"
            );
            assert!(
                line.contains("refs/ae-release/"),
                "only staging refs: {line}"
            );
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Release fixture-owned fake tool waits even when an assertion fails.
        put(&self.base.join("hp-release"), "");
        put(&self.base.join("mac-release"), "");
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(20) {
            let live = std::fs::read_to_string(self.base.join("hp-cargo-pid"))
                .ok()
                .is_some_and(|s| Path::new("/proc").join(s.trim()).exists());
            let finished = !self.base.join("hp-start").exists() || !self.exit_files().is_empty();
            if !live && finished {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

const GIT: &str = r#"#!/usr/bin/env bash
printf 'git' >>"$AE_PAR_BASE/git-trace"
printf '\t%s' "$@" >>"$AE_PAR_BASE/git-trace"
printf '\n' >>"$AE_PAR_BASE/git-trace"
args=()
for arg; do
    if [[ "$arg" == "ssh://spec@hp.invalid$AE_PAR_HP" ]]; then arg="$AE_PAR_HP"; fi
    args+=("$arg")
done
exec "$AE_PAR_REAL_GIT" "${args[@]}"
"#;

const JUST: &str = r#"#!/usr/bin/env bash
args=("$@")
# Compress only the external lock-wait budget. The real lock recipe decides ownership and queue order.
if [[ "$AE_PAR_SIDE" == hp && "$*" == *"lane-lock claim "* && "${args[-1]}" == 1800 ]]; then
    args[-1]=1
fi
exec "$AE_PAR_REAL_JUST" "${args[@]}"
"#;

const TIMEOUT: &str = r#"#!/usr/bin/env bash
# A real timeout drives the hp's timeout branch. All other calls preserve their real bound.
if [[ "$AE_PAR_SIDE" == hp && "$AE_PAR_MODE" == hp-timeout && "$*" == *"3600 just test"* ]]; then
    # The clock starts when the fake gate has really started, so load cannot fire it first. A watcher
    # (bounded at 60 s) sends this process, the real timeout, SIGALRM, which takes its own timeout path (exit 124).
    # The 600 s bound is only a backstop for a dead watcher: the binding clock is that SIGALRM, 0.3 s after hp-start.
    parent=$$
    (
        for ((i=0; i<2400; i++)); do
            [[ ! -f "$AE_PAR_BASE/hp-start" ]] || break
            kill -0 "$parent" 2>/dev/null || exit 0
            "$AE_PAR_REAL_SLEEP" 0.025
        done
        [[ -f "$AE_PAR_BASE/hp-start" ]] || { kill -TERM "$parent" 2>/dev/null; exit 0; }
        "$AE_PAR_REAL_SLEEP" 0.3
        kill -ALRM "$parent" 2>/dev/null
    ) </dev/null >/dev/null 2>&1 &
    exec "$AE_PAR_REAL_TIMEOUT" -k 0.2 600 just test
fi
exec "$AE_PAR_REAL_TIMEOUT" "$@"
"#;

const SSH: &str = r#"#!/usr/bin/env bash
while (($#)); do
    case "$1" in
        -o) shift 2 ;;
        --) shift; break ;;
        -*) shift ;;
        *) break ;;
    esac
done
[[ "${1-}" == spec@hp.invalid ]] || exit 255
shift
input=$(mktemp "$AE_PAR_BASE/ssh-input.XXXXXX") || exit 1
cat >"$input"
output=$(mktemp "$AE_PAR_BASE/ssh-output.XXXXXX") || exit 1
trap 'rm -f "$input" "$output"' EXIT
printf 'ssh\n' >>"$AE_PAR_BASE/ssh-trace"
if [[ "$AE_PAR_MODE" == lost-polls && -f "$AE_PAR_BASE/hp-start" && -f "$AE_PAR_BASE/mac-start" ]]; then
    echo 'fixture: lost poll' >&2
    printf 'lost\n' >>"$AE_PAR_BASE/lost-polls"
    exit 255
fi
export AE_PAR_SIDE=hp
cd "$AE_PAR_HP" || exit 1
status=0
bash -c "$*" <"$input" >"$output" || status=$?
if [[ "$AE_PAR_MODE" == wrong-receipt && -f "$AE_PAR_BASE/hp-start" && "$*" == *gate-receipts* ]]; then
    sed 's/^head=.*/head=0000000000000000000000000000000000000000/' "$output"
elif [[ "$AE_PAR_MODE" == wrong-exit ]]; then
    sed 's/run=[^ ]*/run=another-run/' "$output"
else
    cat "$output"
fi
if [[ "$AE_PAR_MODE" == lost-start ]] && grep -q nohup "$input"; then
    echo 'fixture: start reply lost after execution' >&2
    exit 255
fi
exit "$status"
"#;

const CARGO: &str = r#"#!/usr/bin/env bash
case "${1-}:${2-}" in
    nextest:--version) echo "cargo-nextest $AE_PAR_PIN"; exit 0 ;;
    nextest:list) printf '%s\n' 'spec_one: test' 'spec_two: test'; exit 0 ;;
    nextest:run)
        side=$AE_PAR_SIDE
        printf '%s:nextest:start\n' "$side" >>"$AE_PAR_BASE/trace"
        echo "$$" >"$AE_PAR_BASE/$side-cargo-pid"
        : >"$AE_PAR_BASE/$side-start"
        trap 'printf "%s:terminated\n" "$side" >>"$AE_PAR_BASE/trace"; exit 143' INT TERM HUP
        # The two-way barrier is an independent overlap oracle, not an elapsed-time comparison.
        other=hp
        [[ "$side" != hp ]] || other=mac
        if [[ "$AE_PAR_MODE" == barrier || "$side" == mac && "$AE_PAR_MODE" == hold* ]]; then
            for ((i=0; i<2400; i++)); do [[ ! -f "$AE_PAR_BASE/$other-start" ]] || break; "$AE_PAR_REAL_SLEEP" 0.025; done
            [[ -f "$AE_PAR_BASE/$other-start" ]] || { echo 'barrier: peer never started' >&2; exit 91; }
        fi
        if [[ "$side" == hp ]]; then
            [[ -s "$AE_PAR_HP/.local/locks/gate/owner" ]] || { echo 'hp ran without the lane' >&2; exit 92; }
            if [[ "$AE_PAR_MODE" == hold* || "$AE_PAR_MODE" == lost-polls || "$AE_PAR_MODE" == hp-timeout || "$AE_PAR_MODE" == lost-start ]]; then
                for ((i=0; i<2400; i++)); do [[ ! -f "$AE_PAR_BASE/hp-release" ]] || break; "$AE_PAR_REAL_SLEEP" 0.025; done
                [[ -f "$AE_PAR_BASE/hp-release" ]] || exit 93
            fi
            if [[ "$AE_PAR_MODE" == hp-red-prior ]]; then
                mkdir -p "$AE_PAR_HP/.local/gate-receipts"
                cp "$AE_PAR_BASE/prior-receipt" "$AE_PAR_HP/.local/gate-receipts/$AE_PAR_TREE"
                printf 'hp:nextest:end\n' >>"$AE_PAR_BASE/trace"
                exit 19
            fi
        elif [[ "$AE_PAR_MODE" == hold-mac ]]; then
            for ((i=0; i<2400; i++)); do [[ ! -f "$AE_PAR_BASE/mac-release" ]] || break; "$AE_PAR_REAL_SLEEP" 0.025; done
        elif [[ "$AE_PAR_MODE" == hold-red ]]; then
            printf 'mac:nextest:end\n' >>"$AE_PAR_BASE/trace"
            exit 17
        fi
        printf '%s:nextest:end\n' "$side" >>"$AE_PAR_BASE/trace"
        exit 0 ;;
    test:--doc) printf '%s:doctest\n' "$AE_PAR_SIDE" >>"$AE_PAR_BASE/trace"; exit 0 ;;
esac
printf '%s:%s\n' "$AE_PAR_SIDE" "${1-}" >>"$AE_PAR_BASE/trace"
exit 0
"#;

#[test]
fn missing_receipt_runs_both_legs_with_a_two_way_start_barrier() {
    let f = Fixture::new("overlap");
    let (code, out, err) = f.proof(&[("AE_PAR_MODE", "barrier")]);
    assert_eq!(
        code,
        0,
        "both legs must overlap and finish: {out}{err}; {}",
        f.trace()
    );
    let trace = f.trace();
    for start in ["hp:nextest:start", "mac:nextest:start"] {
        let at = trace
            .find(start)
            .unwrap_or_else(|| panic!("{start}: {trace}"));
        for end in ["hp:nextest:end", "mac:nextest:end"] {
            assert!(
                at < trace.find(end).unwrap_or_else(|| panic!("{end}: {trace}")),
                "{trace}"
            );
        }
        assert_eq!(
            trace.matches(start).count(),
            1,
            "one invocation per leg: {trace}"
        );
    }
    assert!(
        trace.contains("hp:doctest"),
        "hp runs the whole lane: {trace}"
    );
    assert!(
        !trace.contains("mac:doctest"),
        "Mac runs the subset: {trace}"
    );
    assert!(
        f.receipt().is_file(),
        "the real whole-gate writer produced a receipt"
    );
    assert!(!f.owner().exists(), "the normal end released its hp lane");
    assert_eq!(
        f.git(
            &f.hp,
            &["for-each-ref", "--format=%(refname)", "refs/ae-release/"]
        ),
        ""
    );
    f.unchanged_publication();

    // A subsequent release reuses this hp receipt under the unchanged present-receipt semantics.
    put(&f.base.join("trace"), "");
    let (code, out, err) = f.proof(&[]);
    assert_eq!(code, 0, "receipt reuse: {out}{err}");
    assert!(!f.trace().contains("hp:nextest:start"));
    assert!(f.trace().contains("mac:nextest:start"));
}

#[test]
fn a_mac_red_refuses_but_the_owned_hp_leg_finishes_alone_and_can_be_reused() {
    let f = Fixture::new("mac-red");
    let (code, out, err) = f.proof(&[("AE_PAR_MODE", "hold-red")]);
    assert_ne!(code, 0, "a red Mac refuses: {out}{err}");
    assert!(f.base.join("hp-start").is_file(), "the hp really started");
    assert!(!f.trace().contains("hp:nextest:end"), "hp is still running");
    let owner = read(&f.owner());
    assert!(
        owner.starts_with(&format!("release-{}", &f.head()[..12])),
        "{owner}"
    );
    assert!(
        !f.receipt().exists(),
        "hp has not yet completed its real writer"
    );
    f.unchanged_publication();

    let (code, out, err) = f.proof(&[]);
    assert_ne!(
        code, 0,
        "a second release detects the running leg: {out}{err}"
    );
    assert_eq!(
        read(&f.owner()),
        owner,
        "a second attempt cannot touch the live hp lock"
    );
    assert_eq!(f.trace().matches("hp:nextest:start").count(), 1);

    f.release_hp();
    assert!(
        f.receipt().is_file(),
        "the surviving hp leg wrote its normal receipt"
    );
    assert!(!f.owner().exists());
    let (code, out, err) = f.proof(&[]);
    assert_eq!(
        code, 0,
        "later release uses the completed hp receipt: {out}{err}"
    );
}

#[test]
fn three_lost_polls_refuse_without_stopping_or_unlocking_the_hp() {
    let f = Fixture::new("lost-polls");
    let (code, out, err) = f.proof(&[("AE_PAR_MODE", "lost-polls")]);
    assert_ne!(code, 0, "poll loss refuses: {out}{err}");
    assert!(f.base.join("hp-start").is_file());
    assert!(!f.trace().contains("hp:nextest:end"));
    assert!(f.owner().is_file(), "the running hp retains its own lock");
    assert_eq!(
        read(&f.base.join("lost-polls")).lines().count(),
        3,
        "the product tolerates two consecutive lost polls and refuses on the third"
    );
    f.release_hp();
    assert!(f.receipt().is_file());
    assert!(!f.owner().exists());
    f.unchanged_publication();
}

#[test]
fn hp_timeout_marks_the_run_suspect_and_keeps_its_lock() {
    let f = Fixture::new("hp-timeout");
    let (code, out, err) = f.proof(&[("AE_PAR_MODE", "hp-timeout")]);
    assert_ne!(code, 0, "the hp own timeout refuses: {out}{err}");
    assert!(
        f.base.join("hp-start").is_file(),
        "the real timeout reached a running gate"
    );
    assert!(
        f.owner().is_file(),
        "timeout must never auto-release a possibly live gate"
    );
    let notes: Vec<_> = std::fs::read_dir(f.hp.join(".local/release"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "suspect"))
        .collect();
    assert!(!notes.is_empty(), "a suspect note identifies this run");
    let note = read(&notes[0].path());
    assert!(
        note.contains(&f.head()),
        "the suspect note names the exact commit: {note}"
    );
    f.unchanged_publication();
}

#[test]
fn an_active_red_hp_cannot_hide_behind_a_prior_valid_receipt() {
    let f = Fixture::new("red-prior");
    let (code, out, err) = f.proof(&[]);
    assert_eq!(code, 0, "establish a real valid receipt: {out}{err}");
    std::fs::copy(f.receipt(), f.base.join("prior-receipt"))
        .unwrap_or_else(|e| panic!("keep the independent prior receipt: {e}"));
    std::fs::remove_file(f.receipt())
        .unwrap_or_else(|e| panic!("force the missing-receipt entry: {e}"));
    let tree = f.tree();
    let (code, out, err) = f.proof(&[("AE_PAR_MODE", "hp-red-prior"), ("AE_PAR_TREE", &tree)]);
    assert_ne!(
        code, 0,
        "active red plus a matching prior receipt must refuse: {out}{err}"
    );
    assert_eq!(
        read(&f.receipt()),
        read(&f.base.join("prior-receipt")),
        "the opposed valid receipt was really present"
    );
    assert!(
        !f.owner().exists(),
        "ordinary red is a normal end, so it releases its own lane"
    );
    f.unchanged_publication();
}

#[test]
fn a_held_or_queued_hp_lane_prevents_any_warm_worktree_checkout() {
    for queued in [false, true] {
        let f = Fixture::new(if queued { "queued" } else { "held" });
        let warm = PathBuf::from(format!("{}-wt/release", f.hp.display()));
        f.git(
            &f.hp,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                warm.to_str().unwrap_or_default(),
                "HEAD^",
            ],
        );
        let before = f.git(&warm, &["rev-parse", "HEAD"]);
        let locks = f.hp.join(".local/locks");
        if queued {
            put(&locks.join("next"), "another-slice\n");
        } else {
            put(
                &locks.join("gate/owner"),
                "another-slice 2026-11-13T00:00:00Z\n",
            );
        }
        let control = read(&locks.join(if queued { "next" } else { "gate/owner" }));
        let (code, out, err) = f.proof(&[]);
        assert_ne!(code, 0, "a held or queued lane refuses: {out}{err}");
        assert_eq!(
            f.git(&warm, &["rev-parse", "HEAD"]),
            before,
            "no checkout before the hp lane"
        );
        assert_eq!(
            read(&locks.join(if queued { "next" } else { "gate/owner" })),
            control
        );
        assert!(!f.trace().contains("hp:nextest:start"));
        f.unchanged_publication();
        // Flip only the external lane precondition. The same attempted release must then work.
        if queued {
            std::fs::remove_file(locks.join("next"))
                .unwrap_or_else(|e| panic!("remove fixture queue: {e}"));
        } else {
            std::fs::remove_file(locks.join("gate/owner"))
                .unwrap_or_else(|e| panic!("remove fixture owner: {e}"));
            std::fs::remove_dir(locks.join("gate"))
                .unwrap_or_else(|e| panic!("remove fixture gate: {e}"));
        }
        let (code, out, err) = f.proof(&[]);
        assert_eq!(
            code, 0,
            "the same checkout runs after only its lane obstruction is removed: {out}{err}"
        );
        assert!(f.trace().contains("hp:nextest:start"));
        assert!(!f.owner().exists());
    }
}

#[test]
fn hostile_missing_receipt_preconditions_refuse_without_starting_a_leg() {
    for kind in [
        "receipt-symlink",
        "directory-symlink",
        "parent-file",
        "staging-ref",
    ] {
        let f = Fixture::new(kind);
        let receipt = f.receipt();
        match kind {
            "receipt-symlink" => {
                std::fs::create_dir_all(
                    receipt.parent().unwrap_or_else(|| panic!("receipt parent")),
                )
                .unwrap_or_else(|e| panic!("receipt directory: {e}"));
                std::os::unix::fs::symlink(f.base.join("does-not-exist"), &receipt)
                    .unwrap_or_else(|e| panic!("dangling receipt control: {e}"));
            }
            "directory-symlink" => {
                std::fs::create_dir_all(f.hp.join(".local"))
                    .unwrap_or_else(|e| panic!("local: {e}"));
                std::os::unix::fs::symlink(
                    f.base.join("does-not-exist"),
                    f.hp.join(".local/gate-receipts"),
                )
                .unwrap_or_else(|e| panic!("dangling parent control: {e}"));
            }
            "parent-file" => put(&f.hp.join(".local"), "not a directory\n"),
            "staging-ref" => {
                let reference = format!("refs/ae-release/{}", f.head());
                f.git(&f.hp, &["update-ref", &reference, &f.head()]);
            }
            _ => unreachable!(),
        }
        let (code, out, err) = f.proof(&[]);
        assert_ne!(code, 0, "{kind} refuses: {out}{err}");
        assert!(
            !f.trace().contains("hp:nextest:start"),
            "{kind}: no hp execution"
        );
        assert!(!f.owner().exists(), "{kind}: no lock was taken");
        if kind == "staging-ref" {
            assert!(
                !f.git(
                    &f.hp,
                    &["for-each-ref", "--format=%(refname)", "refs/ae-release/"]
                )
                .is_empty(),
                "a preexisting ref is reported, never deleted as this attempt's own"
            );
        }
        f.unchanged_publication();
        // Remove exactly the planted obstruction. An unrelated refusal cannot satisfy this flip.
        match kind {
            "receipt-symlink" => std::fs::remove_file(&receipt)
                .unwrap_or_else(|e| panic!("remove fixture receipt symlink: {e}")),
            "directory-symlink" => std::fs::remove_file(f.hp.join(".local/gate-receipts"))
                .unwrap_or_else(|e| panic!("remove fixture directory symlink: {e}")),
            "parent-file" => std::fs::remove_file(f.hp.join(".local"))
                .unwrap_or_else(|e| panic!("remove fixture parent file: {e}")),
            "staging-ref" => {
                let reference = format!("refs/ae-release/{}", f.head());
                f.git(&f.hp, &["update-ref", "-d", &reference, &f.head()]);
            }
            _ => unreachable!(),
        }
        let (code, out, err) = f.proof(&[]);
        assert_eq!(
            code, 0,
            "{kind}: removing only the hostile precondition allows the same release: {out}{err}"
        );
        assert!(f.trace().contains("hp:nextest:start"));
        assert!(!f.owner().exists());
    }
}

#[test]
fn a_lost_start_reply_refuses_while_the_hp_finishes_and_keeps_a_reusable_receipt() {
    let f = Fixture::new("lost-start");
    let (code, out, err) = f.proof(&[("AE_PAR_MODE", "lost-start")]);
    assert_ne!(code, 0, "ambiguous launch refuses: {out}{err}");
    f.until(
        || f.base.join("hp-start").is_file(),
        "the lost-ACK control really launched the hp",
    );
    assert!(f.owner().is_file(), "the started hp keeps its lock");
    assert!(!f.trace().contains("hp:nextest:end"));
    f.release_hp();
    assert!(f.receipt().is_file());
    assert!(!f.owner().exists());
    f.unchanged_publication();
}

#[test]
fn a_term_to_the_proof_recipe_refuses_and_leaves_the_hp_running() {
    let f = Fixture::new("term");
    // Target the recipe bash, not an assumed just signal-forwarding behavior.
    // P3 must separately check the human-facing top-level just release pid.
    let runner = r#"just release-proof hp=run &
top=$!
for i in $(seq 1 2400); do
    [ ! -f "$AE_PAR_BASE/hp-start" ] || [ ! -f "$AE_PAR_BASE/mac-start" ] || break
    "$AE_PAR_REAL_SLEEP" 0.025
done
[ -f "$AE_PAR_BASE/hp-start" ] && [ -f "$AE_PAR_BASE/mac-start" ] || exit 94
recipe=$(ps -ax -o pid=,ppid=,args= | awk -v p="$top" '$2 == p && /release-proof/ {print $1; exit}')
[ -n "$recipe" ] || exit 95
kill -TERM "$recipe"
# The foreground command may finish before bash delivers its pending TERM, as in today's recipe.
: >"$AE_PAR_BASE/mac-release"
wait "$top"
"#;
    let (code, out, err) = f.call(
        &f.mac,
        "bash",
        &["-c", runner],
        &[("AE_PAR_MODE", "hold-mac")],
    );
    assert_ne!(code, 0, "TERM refuses: {out}{err}");
    assert!(
        f.base.join("hp-start").is_file() && f.base.join("mac-start").is_file(),
        "the signal control reached both active legs: {out}{err}"
    );
    assert!(
        f.owner().is_file(),
        "the Mac signal does not release the hp lock"
    );
    assert!(!f.trace().contains("hp:nextest:end"));
    // The lead scoped Mac interruption to foreground parity, not a new descendant-cleanup guarantee.
    f.release_hp();
    assert!(f.receipt().is_file());
    assert!(!f.owner().exists());
    f.unchanged_publication();
}

#[test]
fn a_green_active_exit_still_requires_the_strict_receipt_and_its_own_exit_identity() {
    for mode in ["wrong-receipt", "wrong-exit"] {
        let f = Fixture::new(mode);
        let (code, out, err) = f.proof(&[("AE_PAR_MODE", mode)]);
        assert_ne!(code, 0, "{mode} must refuse: {out}{err}");
        assert!(
            f.trace().contains("hp:nextest:end"),
            "the active leg completed: {}",
            f.trace()
        );
        assert!(
            f.receipt().is_file(),
            "the actual writer produced a receipt before the opposed reply"
        );
        assert!(!f.owner().exists());
        f.unchanged_publication();
    }
}

#[test]
fn a_dirty_warm_worktree_refuses_without_overwriting_the_foreign_bytes() {
    let f = Fixture::new("dirty-warm");
    let warm = PathBuf::from(format!("{}-wt/release", f.hp.display()));
    f.git(
        &f.hp,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            warm.to_str().unwrap_or_default(),
            "HEAD^",
        ],
    );
    put(&warm.join("foreign-untracked"), "keep this exact content\n");
    let before = f.git(&warm, &["rev-parse", "HEAD"]);
    let (code, out, err) = f.proof(&[]);
    assert_ne!(code, 0, "a dirty reused checkout refuses: {out}{err}");
    assert_eq!(f.git(&warm, &["rev-parse", "HEAD"]), before);
    assert_eq!(
        read(&warm.join("foreign-untracked")),
        "keep this exact content\n"
    );
    assert!(!f.trace().contains("hp:nextest:start"));
    assert!(
        !f.owner().exists(),
        "a worktree refusal releases the lock it actually claimed"
    );
    f.unchanged_publication();
    std::fs::remove_file(warm.join("foreign-untracked"))
        .unwrap_or_else(|e| panic!("remove only the fixture dirt: {e}"));
    let (code, out, err) = f.proof(&[]);
    assert_eq!(
        code, 0,
        "the same reused worktree succeeds after only its dirt is removed: {out}{err}"
    );
    assert!(f.trace().contains("hp:nextest:start"));
    assert!(!f.owner().exists());
}

#[test]
fn unreadable_receipt_and_unsearchable_parent_refuse_then_reuse_the_readable_receipt() {
    for kind in ["receipt", "parent"] {
        let f = Fixture::new(&format!("permissions-{kind}"));
        let (code, out, err) = f.proof(&[]);
        assert_eq!(code, 0, "establish the real whole-gate receipt: {out}{err}");
        let receipt = f.receipt();
        let original = read(&receipt);
        let target = if kind == "receipt" {
            receipt.clone()
        } else {
            receipt
                .parent()
                .unwrap_or_else(|| panic!("receipt parent"))
                .to_path_buf()
        };
        let restored_mode = if kind == "receipt" { 0o644 } else { 0o755 };
        put(&f.base.join("trace"), "");
        std::fs::remove_file(f.base.join("hp-start"))
            .unwrap_or_else(|e| panic!("remove only the completed hp start marker: {e}"));
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o000))
            .unwrap_or_else(|e| panic!("plant the permission denial: {e}"));
        let probe = if kind == "receipt" {
            "test -r \"$1\""
        } else {
            "test -r \"$1\" && test -x \"$1\""
        };
        let (can_access, _, _) = f.call(
            &f.hp,
            "bash",
            &[
                "-c",
                probe,
                "permission-probe",
                target.to_str().unwrap_or_default(),
            ],
            &[],
        );
        if can_access == 0 {
            // A privileged runner can bypass mode 000. The hp's ordinary user exercises both arms.
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(restored_mode))
                .unwrap_or_else(|e| panic!("restore a privileged runner's fixture: {e}"));
            eprintln!(
                "{kind}: permission-denial arm is inapplicable because this runner bypasses mode 000"
            );
            continue;
        }
        let (code, out, err) = f.proof(&[]);
        // Restore fixture access before assertions, so a failing test leaves no inaccessible tree.
        let observed_mode = std::fs::metadata(&target).map_or_else(
            |e| panic!("permission target remains present: {e}"),
            |m| m.permissions().mode() & 0o777,
        );
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(restored_mode))
            .unwrap_or_else(|e| panic!("restore fixture access: {e}"));
        assert_ne!(
            code, 0,
            "{kind}: unreadable persisted evidence must refuse: {out}{err}"
        );
        assert_eq!(
            observed_mode, 0,
            "{kind}: refusal must preserve the permission bits"
        );
        assert_eq!(
            read(&receipt),
            original,
            "{kind}: refusal must preserve all receipt bytes"
        );
        assert!(
            !f.base.join("hp-start").exists(),
            "{kind}: no replacement hp gate starts"
        );
        assert!(
            !f.trace().contains("hp:nextest:start"),
            "{kind}: no hp cargo invocation"
        );
        assert!(!f.owner().exists(), "{kind}: no hp lane is claimed");

        // Flip only accessibility. The already valid receipt must be used, with no new hp gate.
        let (code, out, err) = f.proof(&[]);
        assert_eq!(
            code, 0,
            "{kind}: readable prior receipt must succeed: {out}{err}"
        );
        assert_eq!(
            read(&receipt),
            original,
            "{kind}: reuse does not rewrite the prior receipt"
        );
        assert!(
            !f.trace().contains("hp:nextest:start"),
            "{kind}: the present-receipt path reuses the hp proof"
        );
        assert!(
            f.trace().contains("mac:nextest:start"),
            "{kind}: the Mac subset still runs"
        );
        assert!(!f.owner().exists());
        f.unchanged_publication();
    }
}

#[test]
fn the_release_entry_requests_the_parallel_missing_receipt_path() {
    let justfile = read(&root().join("justfile"));
    let release = super::recipe_text(&justfile, "release:");
    let proof = release
        .iter()
        .position(|s| s.contains("just release-proof"))
        .unwrap_or_else(|| panic!("release must prove the two hosts"));
    assert!(
        release[proof].contains("hp=run"),
        "one release command starts a missing hp gate"
    );
    let bump = release
        .iter()
        .position(|s| s.contains("just bump"))
        .unwrap_or_else(|| panic!("release retains its bump"));
    assert!(proof < bump, "both host proofs precede the bump");
}
