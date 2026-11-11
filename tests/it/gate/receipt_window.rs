//! The receipt vouches only for a tree the whole run tested.
//!
//! `receipt_spec` pins that a clean whole green `just test` writes a receipt
//! and that a tree dirty at the END writes none. It cannot see a run whose
//! tree moved while the tests ran: a commit made mid-run leaves a clean tree
//! at the end that no test ever saw, and dirt that is cleaned mid-run leaves a
//! clean tree at the end that the tests did not run on. This module pins the
//! other half: the run records its HEAD, tree and cleanliness when it STARTS,
//! and a receipt is written only when the tree is still clean and both values
//! are unchanged when it ENDS; otherwise the gate says why and writes none.
//!
//! The fixture is the smallest world the real `rust-check` needs: the real
//! justfile in a throwaway git checkout, a fake `cargo` that answers in
//! constant time, and a fake `nextest run` that acts on the checkout the way
//! `AE_WINDOW_ACT` says. Nothing here edits the frozen spec.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use super::root;
use crate::parity::Invocation;
use crate::parity::capture::ExitOutcome;
use crate::parity::capture::raw;

fn put(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|why| panic!("{}: {why}", parent.display()));
    }
    std::fs::write(path, text).unwrap_or_else(|why| panic!("{}: {why}", path.display()));
}

fn install(path: &Path, body: &str) {
    let _ = std::fs::remove_file(path);
    put(path, body);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|why| panic!("{}: {why}", path.display()));
}

/// One command with the fixture's own `HOME`: `(code, out, err)`.
fn run(fixture: &Fixture, cwd: &Path, program: &str, args: &[&str]) -> (i32, String, String) {
    let mut invocation = Invocation::new(program)
        .env("HOME", &*fixture.base)
        .env("GIT_CONFIG_GLOBAL", fixture.base.join("gitconfig"));
    for arg in args {
        invocation = invocation.arg(arg);
    }
    let io = fixture.base.join("io");
    std::fs::create_dir_all(&io).unwrap_or_else(|why| panic!("the fixture's io dir: {why}"));
    let (out, err) = (io.join("git.out"), io.join("git.err"));
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

struct Fixture {
    base: crate::cli::OwnedScratch,
    tree: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let base = crate::cli::OwnedScratch::root("gate", &format!("window-{tag}"));
        let tree = base.join("tree");
        let fixture = Self { base, tree };
        fixture.stock();
        fixture.git_init();
        fixture
    }

    fn bin(&self) -> PathBuf {
        self.base.join("bin")
    }

    fn stock(&self) {
        std::fs::create_dir_all(self.bin()).unwrap_or_else(|why| panic!("the fakes' dir: {why}"));
        let path = std::env::var_os("PATH").unwrap_or_default();
        for tool in ["just", "tmux", "git"] {
            let real = std::env::split_paths(&path)
                .map(|dir| dir.join(tool))
                .find(|file| file.is_file())
                .unwrap_or_else(|| panic!("{tool} must be on PATH"));
            std::os::unix::fs::symlink(&real, self.bin().join(tool))
                .unwrap_or_else(|why| panic!("the linked {tool}: {why}"));
        }
        // The fake `nextest run` is the mid-run actor. It runs in the checkout.
        //   commit:  a new commit, so HEAD and the tree move and the tree stays clean
        //   restore: drops every tracked edit, so a tree dirty at the start is clean at the end
        install(
            &self.bin().join("cargo"),
            "#!/bin/sh\n\
             case \"$1\" in\n\
             fmt|clippy|check) exit 0 ;;\n\
             nextest) case \"$2\" in --version) echo 'cargo-nextest 0.9.143'; exit 0 ;;\n\
                 list) printf '%s\\n' 'window_one: test' 'window_two: test'; exit 0 ;;\n\
                 run) cd \"$AE_WINDOW_TREE\" || exit 1\n\
                     case \"$AE_WINDOW_ACT\" in\n\
                     commit) echo mid >mid.txt; git add mid.txt; git commit -qm mid ;;\n\
                     restore) git checkout -q -- . ;;\n\
                     esac\n\
                     exit 0 ;;\n\
                 esac ;;\n\
             esac\nexit 0\n",
        );
        install(&self.bin().join("taplo"), "#!/bin/sh\nexit 0\n");
        install(&self.bin().join("shfmt"), "#!/bin/sh\nexit 0\n");
        install(
            &self.bin().join("shellcheck"),
            "#!/bin/sh\ncase \"$1\" in --version) printf '%s\\n' 'ShellCheck - shell script analysis tool' 'version: 0.11.0' ;; esac\nexit 0\n",
        );
        install(
            &self.bin().join("hostname"),
            "#!/bin/sh\necho window-host\n",
        );
        install(
            &self.bin().join("rustc"),
            "#!/bin/sh\ncase \"$1\" in -vV) printf '%s\\n' 'rustc 1.97.1 (window)' 'host: aarch64-apple-darwin' ;;\n\
             --version) echo 'rustc 1.97.1 (window)' ;; esac\nexit 0\n",
        );
    }

    fn git_init(&self) {
        std::fs::create_dir_all(&self.tree).unwrap_or_else(|why| panic!("the tree: {why}"));
        put(
            &self.tree.join("Cargo.toml"),
            "[package]\nname = \"ae\"\nversion = \"0.0.0\"\n",
        );
        put(
            &self.tree.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"ae\"\nversion = \"0.0.0\"\n",
        );
        put(
            &self.tree.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.97.1\"\n",
        );
        put(
            &self.tree.join(".config/nextest.toml"),
            "[profile.default]\ntest-threads = 4\n",
        );
        put(&self.tree.join("src/lib.rs"), "");
        put(
            &self.tree.join("fuzz/Cargo.toml"),
            "[package]\nname = \"ae-fuzz\"\n",
        );
        put(&self.tree.join("fuzz/Cargo.lock"), "version = 4\n");
        put(&self.tree.join("tracked.txt"), "one\n");
        std::fs::copy(root().join("justfile"), self.tree.join("justfile"))
            .unwrap_or_else(|why| panic!("the fixture needs the justfile: {why}"));
        put(&self.tree.join(".gitignore"), ".local/\n/dist/\n/target/\n");
        for words in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "window@example.invalid"],
            vec!["config", "user.name", "Window"],
            vec!["add", "-A"],
            vec!["commit", "-qm", "fixture"],
        ] {
            let (code, out, err) = self.git(&words);
            assert_eq!(code, 0, "git {words:?}: {out}{err}");
        }
    }

    fn git(&self, words: &[&str]) -> (i32, String, String) {
        let mut full = vec!["-C", self.tree.to_str().unwrap_or_default()];
        full.extend_from_slice(words);
        run(self, &self.tree, "git", &full)
    }

    /// `just test` in the checkout with `act` as the mid-run action.
    fn gate(&self, act: &str) -> (i32, String, String) {
        let invocation = Invocation::new("just")
            .env_cleared()
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.bin().display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("HOME", &*self.base)
            .env("TMPDIR", &*self.base)
            .env("AE_TEST_TMPDIR", &*self.base)
            .env("GIT_CONFIG_GLOBAL", self.base.join("gitconfig"))
            .env("AE_WINDOW_TREE", &self.tree)
            .env("AE_WINDOW_ACT", act)
            .arg("--justfile")
            .arg(self.tree.join("justfile"))
            .arg("--working-directory")
            .arg(&self.tree)
            .arg("test");
        let (out, err) = (self.base.join("gate.out"), self.base.join("gate.err"));
        let status = raw::run(&invocation, &self.tree, &out, &err)
            .unwrap_or_else(|why| panic!("the gate runs: {why}"));
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

    /// Every file under the main checkout's receipts directory, sorted.
    fn receipts(&self) -> Vec<String> {
        let dir = self.tree.join(".local/gate-receipts");
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn clear_receipts(&self) {
        let _ = std::fs::remove_dir_all(self.tree.join(".local/gate-receipts"));
    }
}

/// The control: with nothing moving, the fixture writes one receipt. Without
/// it, a red result below could be a fixture that never writes at all.
#[test]
fn a_quiet_run_in_the_window_fixture_writes_its_receipt() {
    let fixture = Fixture::new("quiet");
    let (code, out, err) = fixture.gate("none");
    assert_eq!(code, 0, "the quiet gate is green: {out}{err}");
    assert_eq!(
        fixture.receipts().len(),
        1,
        "a clean unmoving run writes one receipt: {err}"
    );
}

/// A commit made while the tests ran leaves a clean tree at the end. The
/// tests ran on the tree before the commit, so no receipt may name either one.
#[test]
fn a_commit_during_the_run_writes_no_receipt() {
    let fixture = Fixture::new("commit");
    let (code, out, err) = fixture.gate("commit");
    assert_eq!(code, 0, "the gate itself is still green: {out}{err}");
    assert_eq!(
        fixture.receipts(),
        Vec::<String>::new(),
        "no receipt for a tree that moved during the run: {err}"
    );
    assert!(
        err.contains("gate-receipt: not written") && err.contains("HEAD"),
        "the gate must say that HEAD moved during the run: {err}"
    );
}

/// Dirt that is cleaned while the tests ran leaves a clean tree at the end,
/// but the tests ran on the dirty one: the start state decides, not the end.
#[test]
fn a_tree_dirty_at_the_start_and_clean_at_the_end_writes_no_receipt() {
    let fixture = Fixture::new("restore");
    put(&fixture.tree.join("tracked.txt"), "dirty before the run\n");
    let (code, out, err) = fixture.gate("restore");
    assert_eq!(code, 0, "the gate itself is still green: {out}{err}");
    assert_eq!(
        fixture.receipts(),
        Vec::<String>::new(),
        "no receipt when the run started dirty: {err}"
    );
    assert!(
        err.contains("gate-receipt: not written") && err.contains("start"),
        "the gate must say that the run started on a dirty tree: {err}"
    );
}

/// A later quiet run still writes: the refusal belongs to one run, not to the
/// checkout.
#[test]
fn a_quiet_run_after_a_moved_one_writes_its_receipt() {
    let fixture = Fixture::new("after");
    let (code, out, err) = fixture.gate("commit");
    assert_eq!(code, 0, "{out}{err}");
    fixture.clear_receipts();
    let (code, out, err) = fixture.gate("none");
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(
        fixture.receipts().len(),
        1,
        "the quiet run after it writes: {err}"
    );
}
